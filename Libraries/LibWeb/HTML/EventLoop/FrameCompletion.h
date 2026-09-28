/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/Mutex.h>
#include <AK/Noncopyable.h>
#include <AK/RefPtr.h>
#include <LibCore/Forward.h>
#include <LibWeb/Export.h>

namespace Web::HTML {

// The completion of a frame that ran beside the document thread. The thread that
// finishes the frame posts it; the thread that registered receives it through its own Core event loop, which runs
// the delivery steps no matter whether the page is visible, idle or waiting for a rendering opportunity.
//
// A completion is never lost: one posted before the event loop registers is delivered at registration, and one posted
// while a delivery is already queued is taken by that delivery. Posts between two takes coalesce into one.
class WEB_API FrameCompletion {
    AK_MAKE_NONCOPYABLE(FrameCompletion);
    AK_MAKE_NONMOVABLE(FrameCompletion);

public:
    // The document thread's completion, whose delivery schedules the main thread event loop's processing.
    static FrameCompletion& the();

    FrameCompletion() = default;
    ~FrameCompletion();

    // Registering thread only. Takes the current thread's Core event loop; a completion already posted is delivered.
    // Registering again only replaces the delivery steps.
    void register_event_loop(Function<void()> deliver);
    bool is_registered() const;

    // Any thread.
    void post();

    // Registering thread only.
    bool is_pending() const;
    bool take();

    // Any thread.
    u64 posted_count() const;
    u64 delivered_count() const;

private:
    void queue_delivery_if_needed();
    void deliver();

    mutable Mutex m_mutex;
    RefPtr<Core::WeakEventLoopReference> m_event_loop;
    Function<void()> m_deliver;
    bool m_pending { false };
    bool m_delivery_queued { false };
    u64 m_posted_count { 0 };
    u64 m_delivered_count { 0 };
};

}
