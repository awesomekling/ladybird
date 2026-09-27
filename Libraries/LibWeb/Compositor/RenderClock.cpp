/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Math.h>
#include <Compositor/RenderClockClientEndpoint.h>
#include <Compositor/RenderClockServerEndpoint.h>
#include <LibCore/EventLoop.h>
#include <LibCore/ThreadEventQueue.h>
#include <LibCore/Timer.h>
#include <LibIPC/ConnectionToServer.h>
#include <LibIPC/Transport.h>
#include <LibThreading/Thread.h>
#include <LibWeb/Compositor/RenderClock.h>

namespace Web::Compositor {

static thread_local bool s_on_render_clock_thread = false;

// LibIPC pins a connection to the thread that constructs it, so the clock thread creates and destroys every channel.
class RenderClockChannel final : public IPC::ConnectionToServer<RenderClockClientEndpoint, RenderClockServerEndpoint> {
public:
    RenderClockChannel(NonnullOwnPtr<IPC::Transport> transport, RenderClock& clock)
        : IPC::ConnectionToServer<RenderClockClientEndpoint, RenderClockServerEndpoint>(*this, move(transport))
        , m_clock(clock)
    {
    }

    virtual ~RenderClockChannel() override
    {
        VERIFY(s_on_render_clock_thread);
        m_clock.did_destroy_channel();
    }

    // A channel the clock let go of on purpose reports nothing more, not even its own death.
    void detach() { m_detached = true; }

private:
    // The Compositor went away. Main learns of it on its own connection.
    virtual void die() override
    {
        if (!m_detached)
            m_clock.did_lose_channel();
    }

    virtual void clock_tick(Compositing::CompositorContextId context_id, i64 frame_time_nanoseconds, double frame_interval_milliseconds) override
    {
        if (!m_detached)
            m_clock.did_receive_clock_tick(context_id, frame_time_nanoseconds, frame_interval_milliseconds);
    }

    RenderClock& m_clock;
    bool m_detached { false };
};

ErrorOr<NonnullOwnPtr<RenderClock>> RenderClock::create(PostTick post_tick)
{
    auto clock = adopt_own(*new RenderClock(move(post_tick)));
    MutexLocker locker(clock->m_mutex);
    clock->m_condition.wait_while([&] { return !clock->m_started; });
    if (!clock->m_event_loop)
        return Error::from_string_literal("RenderClock thread did not start");
    return clock;
}

RenderClock::RenderClock(PostTick post_tick)
    : m_post_tick(move(post_tick))
    , m_thread(Threading::Thread::construct("RenderClock"sv, [this] {
        return thread_main();
    }))
{
    m_thread->start();
}

RenderClock::~RenderClock()
{
    RefPtr<Core::WeakEventLoopReference> event_loop;
    {
        MutexLocker locker(m_mutex);
        event_loop = m_event_loop;
    }

    if (event_loop) {
        if (auto strong_event_loop = event_loop->take()) {
            strong_event_loop->quit(0);
            strong_event_loop->wake();
        }
    }

    if (m_thread->needs_to_be_joined())
        (void)m_thread->join();

    // The thread let go of its channel and of every armed context before it returned.
    VERIFY(!m_channel);
    VERIFY(m_armed_contexts.is_empty());
}

intptr_t RenderClock::thread_main()
{
    s_on_render_clock_thread = true;
    Core::EventLoop event_loop;
    {
        MutexLocker locker(m_mutex);
        m_event_loop = Core::EventLoop::current_weak();
        m_started = true;
        m_condition.broadcast();
    }

    auto result = event_loop.exec();

    // What is still queued on this thread runs before it returns: a connection's deferred invocations hold references
    // to it, and the last one to go would destroy the channel as the thread exits, after its thread-locals are gone.
    // Nothing it runs arms a context, or sets up a channel, for the thread to leave behind.
    do {
        clear_armed_contexts();
        if (auto channel = move(m_channel)) {
            channel->detach();
            channel->shutdown();
        }
    } while (Core::ThreadEventQueue::current().process() > 0);

    {
        MutexLocker locker(m_mutex);
        m_event_loop.clear();
    }
    return result;
}

bool RenderClock::invoke_on_clock_thread(Function<void()> function)
{
    RefPtr<Core::WeakEventLoopReference> event_loop;
    {
        MutexLocker locker(m_mutex);
        event_loop = m_event_loop;
    }
    if (!event_loop)
        return false;
    auto strong_event_loop = event_loop->take();
    if (!strong_event_loop)
        return false;
    strong_event_loop->deferred_invoke(move(function));
    return true;
}

ErrorOr<IPC::TransportHandle> RenderClock::attach()
{
    Mutex result_mutex;
    ConditionVariable result_condition { result_mutex };
    Optional<ErrorOr<IPC::TransportHandle>> result;

    auto invoked = invoke_on_clock_thread([&] {
        auto handle_or_error = replace_channel();
        MutexLocker locker(result_mutex);
        result = move(handle_or_error);
        result_condition.broadcast();
    });
    if (!invoked)
        return Error::from_string_literal("RenderClock thread is gone");

    // The clock thread only goes away when this RenderClock is destroyed, so it runs what was just handed to it.
    MutexLocker locker(result_mutex);
    result_condition.wait_while([&] { return !result.has_value(); });
    return result.release_value();
}

ErrorOr<IPC::TransportHandle> RenderClock::replace_channel()
{
    VERIFY(s_on_render_clock_thread);
    // A new channel starts with nothing armed: the contexts armed on the previous one were armed for a Compositor
    // that is gone.
    clear_armed_contexts();
    if (auto channel = move(m_channel)) {
        channel->detach();
        channel->shutdown();
    }

    auto paired = TRY(IPC::Transport::create_paired());
    m_channel = adopt_ref(*new RenderClockChannel(move(paired.local), *this));
    return move(paired.remote_handle);
}

void RenderClock::arm(Compositing::CompositorContextId context_id, double maximum_frames_per_second)
{
    VERIFY(isfinite(maximum_frames_per_second) && maximum_frames_per_second > 0);
    (void)invoke_on_clock_thread([this, context_id, maximum_frames_per_second] {
        auto& armed = m_armed_contexts.ensure(context_id, [&] {
            return ArmedContext { .maximum_frames_per_second = maximum_frames_per_second, .frame_interval_milliseconds = 1000.0 / maximum_frames_per_second };
        });
        armed.maximum_frames_per_second = maximum_frames_per_second;
        m_armed_context_count.store(m_armed_contexts.size(), AK::MemoryOrder::memory_order_relaxed);
        request_clock_tick(context_id, armed);
        restart_watchdog(context_id, armed);
    });
}

void RenderClock::disarm(Compositing::CompositorContextId context_id)
{
    (void)invoke_on_clock_thread([this, context_id] {
        auto armed = m_armed_contexts.take(context_id);
        if (!armed.has_value())
            return;
        if (armed->watchdog)
            armed->watchdog->stop();
        m_armed_context_count.store(m_armed_contexts.size(), AK::MemoryOrder::memory_order_relaxed);
    });
}

void RenderClock::request_clock_tick(Compositing::CompositorContextId context_id, ArmedContext const& armed)
{
    VERIFY(s_on_render_clock_thread);
    if (m_channel)
        m_channel->async_request_clock_tick(context_id, armed.maximum_frames_per_second);
}

void RenderClock::restart_watchdog(Compositing::CompositorContextId context_id, ArmedContext& armed)
{
    // A request can be lost: one that raced the context's registration with the Compositor is dropped there. After
    // four intervals with no tick, the watchdog asks again; the Compositor ignores a request it already has.
    auto timeout_milliseconds = max(1, static_cast<int>(AK::ceil(4 * armed.frame_interval_milliseconds)));
    if (!armed.watchdog) {
        armed.watchdog = Core::Timer::create_single_shot(timeout_milliseconds, [this, context_id] {
            auto it = m_armed_contexts.find(context_id);
            if (it == m_armed_contexts.end())
                return;
            m_watchdog_requests.fetch_add(1, AK::MemoryOrder::memory_order_relaxed);
            request_clock_tick(context_id, it->value);
            restart_watchdog(context_id, it->value);
        });
    }
    armed.watchdog->restart(timeout_milliseconds);
}

void RenderClock::did_receive_clock_tick(Compositing::CompositorContextId context_id, i64 frame_time_nanoseconds, double frame_interval_milliseconds)
{
    VERIFY(s_on_render_clock_thread);
    // A tick for a context disarmed after its request went out, or armed on an earlier channel.
    auto it = m_armed_contexts.find(context_id);
    if (it == m_armed_contexts.end()) {
        m_ticks_dropped.fetch_add(1, AK::MemoryOrder::memory_order_relaxed);
        return;
    }

    // The next request goes out before this tick is posted, so that the Compositor's pacing is the only limit on the
    // rate: however long the Rendering side takes with this tick, the next one is already on its way.
    auto& armed = it->value;
    if (isfinite(frame_interval_milliseconds) && frame_interval_milliseconds > 0)
        armed.frame_interval_milliseconds = frame_interval_milliseconds;
    request_clock_tick(context_id, armed);
    restart_watchdog(context_id, armed);

    m_post_tick(context_id, frame_time_nanoseconds, frame_interval_milliseconds);
    m_ticks_posted.fetch_add(1, AK::MemoryOrder::memory_order_relaxed);
}

void RenderClock::did_lose_channel()
{
    VERIFY(s_on_render_clock_thread);
    m_channels_lost.fetch_add(1, AK::MemoryOrder::memory_order_relaxed);
    clear_armed_contexts();
    // The channel is dying from inside its own handler, which keeps it alive until it returns.
    m_channel = nullptr;
}

void RenderClock::clear_armed_contexts()
{
    for (auto& armed : m_armed_contexts) {
        if (armed.value.watchdog)
            armed.value.watchdog->stop();
    }
    m_armed_contexts.clear();
    m_armed_context_count.store(0, AK::MemoryOrder::memory_order_relaxed);
}

RenderClock::Statistics RenderClock::statistics() const
{
    return {
        .armed_contexts = m_armed_context_count.load(AK::MemoryOrder::memory_order_relaxed),
        .ticks_posted = m_ticks_posted.load(AK::MemoryOrder::memory_order_relaxed),
        .ticks_dropped = m_ticks_dropped.load(AK::MemoryOrder::memory_order_relaxed),
        .watchdog_requests = m_watchdog_requests.load(AK::MemoryOrder::memory_order_relaxed),
        .channels_lost = m_channels_lost.load(AK::MemoryOrder::memory_order_relaxed),
        .channels_destroyed = m_channels_destroyed.load(AK::MemoryOrder::memory_order_relaxed),
    };
}

}
