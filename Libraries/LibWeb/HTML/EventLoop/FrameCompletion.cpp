/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/NeverDestroyed.h>
#include <LibCore/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameCompletion.h>

namespace Web::HTML {

FrameCompletion& FrameCompletion::the()
{
    // The frame's thread may post after the main thread has begun exiting, so this is never destroyed.
    static NeverDestroyed<FrameCompletion> s_the;
    return *s_the;
}

FrameCompletion::~FrameCompletion() = default;

void FrameCompletion::register_event_loop(Function<void()> deliver)
{
    MutexLocker locker(m_mutex);
    m_deliver = move(deliver);
    m_event_loop = Core::EventLoop::current_weak();
    queue_delivery_if_needed();
}

bool FrameCompletion::is_registered() const
{
    MutexLocker locker(m_mutex);
    return m_event_loop;
}

void FrameCompletion::post()
{
    MutexLocker locker(m_mutex);
    ++m_posted_count;
    m_pending = true;
    queue_delivery_if_needed();
}

bool FrameCompletion::is_pending() const
{
    MutexLocker locker(m_mutex);
    return m_pending;
}

bool FrameCompletion::take()
{
    MutexLocker locker(m_mutex);
    return exchange(m_pending, false);
}

u64 FrameCompletion::posted_count() const
{
    MutexLocker locker(m_mutex);
    return m_posted_count;
}

u64 FrameCompletion::delivered_count() const
{
    MutexLocker locker(m_mutex);
    return m_delivered_count;
}

// With the mutex held. A post before registration stays pending until registration queues its delivery; a post while
// a delivery is queued is seen by that delivery, which reads the pending flag only once it runs.
void FrameCompletion::queue_delivery_if_needed()
{
    if (!m_pending || m_delivery_queued || !m_event_loop)
        return;
    auto event_loop = m_event_loop->take();
    if (!event_loop.is_alive())
        return;
    m_delivery_queued = true;
    event_loop->deferred_invoke([this] { deliver(); });
    // A post from another thread does not wake the loop by itself.
    event_loop->wake();
}

void FrameCompletion::deliver()
{
    {
        MutexLocker locker(m_mutex);
        m_delivery_queued = false;
        if (!m_pending)
            return;
        ++m_delivered_count;
    }
    // Outside the lock: the steps may take the completion, or post another one on this thread.
    if (m_deliver)
        m_deliver();
}

}
