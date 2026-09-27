/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Atomic.h>
#include <AK/ConditionVariable.h>
#include <AK/Function.h>
#include <AK/HashMap.h>
#include <AK/Mutex.h>
#include <AK/NonnullOwnPtr.h>
#include <AK/Optional.h>
#include <AK/RefPtr.h>
#include <LibCompositing/Types.h>
#include <LibCore/Forward.h>
#include <LibIPC/TransportHandle.h>
#include <LibThreading/Forward.h>
#include <LibWeb/Export.h>

namespace Web::Compositor {

class RenderClockChannel;

// The WebContent end of the render clock channel: a thread of its own that asks the Compositor for display ticks for
// the contexts that are armed, and hands each tick it is delivered to the Rendering side without passing through the
// main thread. There is one per process; a Compositor reconnect swaps its channel, not its thread.
//
// Main owns it and reaches its thread only through the calls below. The thread owns the channel, the armed contexts
// and their watchdogs, and is the only one that posts ticks, so once the destructor (which joins it) has returned,
// no tick is posted again.
class WEB_API RenderClock {
    AK_MAKE_NONCOPYABLE(RenderClock);
    AK_MAKE_NONMOVABLE(RenderClock);

public:
    AK_ALLOC_WITH_KMALLOC;

    // Runs on the clock thread, for each tick delivered to an armed context.
    using PostTick = Function<void(Compositing::CompositorContextId, i64 frame_time_nanoseconds, double frame_interval_milliseconds)>;

    static ErrorOr<NonnullOwnPtr<RenderClock>> create(PostTick);
    ~RenderClock();

    // Builds a new channel on the clock thread, in place of the previous one and with nothing armed, and returns the
    // end to offer the Compositor. Blocks until the channel exists.
    ErrorOr<IPC::TransportHandle> attach();

    // Asynchronous. A context is armed until it is disarmed or the channel is lost; a tick that arrives for a context
    // that is not armed is dropped.
    void arm(Compositing::CompositorContextId, double maximum_frames_per_second);
    void disarm(Compositing::CompositorContextId);

    struct Statistics {
        u64 armed_contexts { 0 };
        u64 ticks_posted { 0 };
        u64 ticks_dropped { 0 };
        u64 watchdog_requests { 0 };
        u64 channels_lost { 0 };
        u64 channels_destroyed { 0 };
    };
    Statistics statistics() const;

private:
    struct ArmedContext {
        double maximum_frames_per_second { 60.0 };
        double frame_interval_milliseconds { 1000.0 / 60.0 };
        RefPtr<Core::Timer> watchdog {};
    };

    friend class RenderClockChannel;

    explicit RenderClock(PostTick);
    intptr_t thread_main();
    [[nodiscard]] bool invoke_on_clock_thread(Function<void()>);

    // On the clock thread.
    ErrorOr<IPC::TransportHandle> replace_channel();
    void request_clock_tick(Compositing::CompositorContextId, ArmedContext const&);
    void restart_watchdog(Compositing::CompositorContextId, ArmedContext&);
    void did_receive_clock_tick(Compositing::CompositorContextId, i64 frame_time_nanoseconds, double frame_interval_milliseconds);
    void did_lose_channel();
    void did_destroy_channel() { m_channels_destroyed.fetch_add(1, AK::MemoryOrder::memory_order_relaxed); }
    void clear_armed_contexts();

    PostTick m_post_tick;
    NonnullRefPtr<Threading::Thread> m_thread;

    Mutex m_mutex;
    ConditionVariable m_condition { m_mutex };
    bool m_started { false };
    RefPtr<Core::WeakEventLoopReference> m_event_loop;

    // Owned by the clock thread.
    RefPtr<RenderClockChannel> m_channel;
    HashMap<Compositing::CompositorContextId, ArmedContext> m_armed_contexts;

    Atomic<u64> m_armed_context_count { 0 };
    Atomic<u64> m_ticks_posted { 0 };
    Atomic<u64> m_ticks_dropped { 0 };
    Atomic<u64> m_watchdog_requests { 0 };
    Atomic<u64> m_channels_lost { 0 };
    Atomic<u64> m_channels_destroyed { 0 };
};

}
