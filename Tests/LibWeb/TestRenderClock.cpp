/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Atomic.h>
#include <AK/HashTable.h>
#include <AK/Random.h>
#include <AK/Time.h>
#include <Compositor/RenderClockClientEndpoint.h>
#include <Compositor/RenderClockServerEndpoint.h>
#include <LibCore/EventLoop.h>
#include <LibCore/Timer.h>
#include <LibIPC/ConnectionFromClient.h>
#include <LibTest/TestCase.h>
#include <LibWeb/Compositor/RenderClock.h>
#include <pthread.h>
#include <unistd.h>

namespace {

// The Compositor end of the channel, on the test's main thread. It answers each request on its next "display tick",
// or at once.
class FakeCompositor final : public IPC::ConnectionFromClient<RenderClockClientEndpoint, RenderClockServerEndpoint> {
public:
    enum class Answer {
        OnNextTick,
        AtOnce,
    };

    static NonnullRefPtr<FakeCompositor> create(IPC::TransportHandle handle, Answer answer = Answer::OnNextTick)
    {
        return adopt_ref(*new FakeCompositor(MUST(handle.create_transport()), answer));
    }

    virtual ~FakeCompositor() override
    {
        m_display_tick->stop();
    }

    size_t requests_received() const { return m_requests_received; }
    bool is_dead() const { return m_is_dead; }

    // Drops the next requests, as the real Compositor does with one for a context it does not have (yet).
    void drop_next_requests(size_t count) { m_requests_to_drop = count; }

private:
    FakeCompositor(NonnullOwnPtr<IPC::Transport> transport, Answer answer)
        : IPC::ConnectionFromClient<RenderClockClientEndpoint, RenderClockServerEndpoint>(*this, move(transport), 1)
        , m_answer(answer)
        , m_display_tick(Core::Timer::create_repeating(4, [this] { tick(); }))
    {
        if (m_answer == Answer::OnNextTick)
            m_display_tick->start();
    }

    virtual void die() override
    {
        m_is_dead = true;
        m_display_tick->stop();
    }

    virtual void request_clock_tick(Compositing::CompositorContextId context_id, double maximum_frames_per_second) override
    {
        ++m_requests_received;
        if (m_requests_to_drop > 0) {
            --m_requests_to_drop;
            return;
        }
        if (m_answer == Answer::AtOnce) {
            async_clock_tick(context_id, MonotonicTime::now().nanoseconds(), 1000.0 / maximum_frames_per_second, {}, {});
            return;
        }
        m_pending_requests.set(context_id);
    }

    void tick()
    {
        auto frame_time = MonotonicTime::now().nanoseconds();
        for (auto context_id : m_pending_requests)
            async_clock_tick(context_id, frame_time, 4.0, {}, {});
        m_pending_requests.clear();
    }

    Answer m_answer;
    NonnullRefPtr<Core::Timer> m_display_tick;
    HashTable<Compositing::CompositorContextId> m_pending_requests;
    size_t m_requests_received { 0 };
    size_t m_requests_to_drop { 0 };
    bool m_is_dead { false };
};

struct PostedTicks {
    Atomic<u64> count { 0 };
    Atomic<u64> threads_other_than_the_first { 0 };
    Atomic<pthread_t> first_thread {};
    Atomic<bool> has_first_thread { false };

    void post()
    {
        auto thread = pthread_self();
        bool expected = false;
        if (has_first_thread.compare_exchange_strong(expected, true))
            first_thread.store(thread);
        else if (!pthread_equal(first_thread.load(), thread))
            threads_other_than_the_first.fetch_add(1);
        count.fetch_add(1);
    }
};

bool spin_event_loop_until(Core::EventLoop& event_loop, int timeout_in_milliseconds, Function<bool()> condition)
{
    bool timed_out = false;
    auto timeout_timer = Core::Timer::create_single_shot(timeout_in_milliseconds, [&] { timed_out = true; });
    timeout_timer->start();
    // What the clock thread does wakes nothing on this loop, so the condition is polled.
    auto poll_timer = Core::Timer::create_repeating(1, [] { });
    poll_timer->start();
    event_loop.spin_until([&] { return timed_out || condition(); });
    return !timed_out;
}

constexpr Compositing::CompositorContextId context_id { 1 };

}

// A Compositor crash takes the channel with it. The clock forgets what was armed, posts nothing more, and resumes on
// the channel it is attached to next, on the same thread.
TEST_CASE(a_render_clock_resumes_on_a_new_channel_after_losing_one)
{
    Core::EventLoop event_loop;
    PostedTicks posted;
    OwnPtr<Web::Compositor::RenderClock> clock = MUST(Web::Compositor::RenderClock::create([&](auto, i64, double, auto, auto) { posted.post(); }));

    auto compositor_a = FakeCompositor::create(MUST(clock->attach()));
    clock->arm(context_id, 60);
    EXPECT(spin_event_loop_until(event_loop, 2000, [&] { return posted.count.load() >= 5; }));

    // The Compositor closes its end, as its death would.
    compositor_a->shutdown();
    EXPECT(spin_event_loop_until(event_loop, 2000, [&] {
        auto statistics = clock->statistics();
        return statistics.channels_lost == 1 && statistics.channels_destroyed == 1;
    }));
    EXPECT_EQ(clock->statistics().armed_contexts, 0u);
    auto posted_before_the_new_channel = posted.count.load();
    EXPECT(!spin_event_loop_until(event_loop, 100, [&] { return posted.count.load() > posted_before_the_new_channel; }));

    // A new channel has nothing armed: the Compositor gets no request until the context is armed again.
    auto compositor_b = FakeCompositor::create(MUST(clock->attach()));
    EXPECT(!spin_event_loop_until(event_loop, 50, [&] { return compositor_b->requests_received() > 0; }));
    clock->arm(context_id, 60);
    EXPECT(spin_event_loop_until(event_loop, 2000, [&] { return posted.count.load() >= posted_before_the_new_channel + 5; }));

    // Every tick was posted from the one clock thread, which is not this one.
    EXPECT_EQ(posted.threads_other_than_the_first.load(), 0u);
    EXPECT(!pthread_equal(posted.first_thread.load(), pthread_self()));

    // A disarmed context gets nothing more; the tick answering its last request is dropped.
    clock->disarm(context_id);
    EXPECT(spin_event_loop_until(event_loop, 2000, [&] { return clock->statistics().ticks_dropped == 1; }));
    auto posted_after_disarm = posted.count.load();
    EXPECT(!spin_event_loop_until(event_loop, 100, [&] { return posted.count.load() > posted_after_disarm || clock->statistics().ticks_dropped > 1; }));

    clock = nullptr;
    EXPECT(spin_event_loop_until(event_loop, 2000, [&] { return compositor_b->is_dead(); }));
}

// On a Compositor reconnect, the UI process registers the contexts (a synchronous call to the Compositor) before it
// tells WebContent that it reconnected, so the first request after a reconnect finds its context. A request can still
// be dropped when it races a context's first registration; the watchdog asks again.
TEST_CASE(a_render_clock_asks_again_when_a_request_goes_unanswered)
{
    Core::EventLoop event_loop;
    PostedTicks posted;
    OwnPtr<Web::Compositor::RenderClock> clock = MUST(Web::Compositor::RenderClock::create([&](auto, i64, double, auto, auto) { posted.post(); }));

    auto compositor = FakeCompositor::create(MUST(clock->attach()));
    compositor->drop_next_requests(1);
    clock->arm(context_id, 60);
    EXPECT(spin_event_loop_until(event_loop, 2000, [&] { return posted.count.load() >= 3; }));
    EXPECT(clock->statistics().watchdog_requests >= 1);
    EXPECT(compositor->requests_received() >= 4);

    clock->disarm(context_id);
}

// The owner destroys the clock while ticks are arriving as fast as it can post them. Once the destructor has
// returned, no tick is posted and none is being posted.
TEST_CASE(a_render_clock_posts_nothing_once_destroyed_under_load)
{
    Core::EventLoop event_loop;

    struct Sentinel {
        Atomic<bool> destroyed { false };
        Atomic<u64> posts_in_progress { 0 };
        Atomic<u64> posts_after_destruction { 0 };
        Atomic<u64> posts { 0 };
    };
    Sentinel sentinel;

    for (size_t iteration = 0; iteration < 200; ++iteration) {
        sentinel.destroyed.store(false);
        OwnPtr<Web::Compositor::RenderClock> clock = MUST(Web::Compositor::RenderClock::create([&](auto, i64, double, auto, auto) {
            sentinel.posts_in_progress.fetch_add(1);
            if (sentinel.destroyed.load())
                sentinel.posts_after_destruction.fetch_add(1);
            usleep(1000);
            if (sentinel.destroyed.load())
                sentinel.posts_after_destruction.fetch_add(1);
            sentinel.posts.fetch_add(1);
            sentinel.posts_in_progress.fetch_sub(1);
        }));
        auto compositor = FakeCompositor::create(MUST(clock->attach()), FakeCompositor::Answer::AtOnce);
        clock->arm(context_id, 60);
        clock->arm(Compositing::CompositorContextId { 2 }, 60);

        auto lifetime_in_milliseconds = static_cast<int>(get_random_uniform(21));
        if (lifetime_in_milliseconds > 0)
            (void)spin_event_loop_until(event_loop, lifetime_in_milliseconds, [] { return false; });

        clock = nullptr;
        sentinel.destroyed.store(true);
        EXPECT_EQ(sentinel.posts_in_progress.load(), 0u);

        // Let the Compositor end see the channel close, and give a stray post the time to show up.
        (void)spin_event_loop_until(event_loop, 2000, [&] { return compositor->is_dead(); });
        EXPECT(compositor->is_dead());
    }

    EXPECT_EQ(sentinel.posts_after_destruction.load(), 0u);
    EXPECT(sentinel.posts.load() > 0u);
}
