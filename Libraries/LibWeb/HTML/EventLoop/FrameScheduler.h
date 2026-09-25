/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Noncopyable.h>
#include <AK/OwnPtr.h>
#include <AK/Vector.h>
#include <AK/kmalloc.h>
#include <LibGC/Ptr.h>
#include <LibJS/Heap/Cell.h>
#include <LibWeb/Forward.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/LocalNavigable.h>

namespace Web::HTML {

// A rendering update's frame, submitted to the render side. The main thread goes back to its event loop while the
// render side runs it, and takes it in later: at the top of the event loop once it has finished, or in a forced join
// when main-thread code reaches what the frame owns first.
// NB: The ticket lives on the heap and the frame scheduler traces what it holds, so no main-thread stack frame has to
//     outlive the frame.
struct FrameTicket {
    AK_ALLOC_WITH_KMALLOC;
    AK_MAKE_NONCOPYABLE(FrameTicket);
    AK_MAKE_NONMOVABLE(FrameTicket);

public:
    FrameTicket() = default;

    struct SubmittedNavigable {
        GC::Ref<LocalNavigable> navigable;
        LocalNavigable::PendingCompositorFrame frame;
        // The compositor context consume-commit hands the frame to, which the ticket holds until then.
        Optional<u64> held_compositor_context;
    };

    // The navigables whose frames the render side records, in the order the rendering update paints them. Each one's
    // compositor frame is built and handed off by consume-commit.
    Vector<SubmittedNavigable> navigables;

    // The local roots whose frames consume-commit has handed off, for the tail's screenshots.
    Vector<GC::Ref<LocalNavigable>> painted_local_roots;
};

// Runs the rendering update's frame beside the main thread under LIBWEB_STAGE_THREAD=overlap. One rendering update is
// a main half (every step of it but the recording, on the main thread), one frame (the recording, on the render side),
// a consume-commit that takes the frame in (publishes each recording, then builds and hands off its compositor frame,
// and runs no script) and a tail (the screenshots of the frame and the end of the rendering update). At most one frame
// is in flight, and the next rendering update starts only once the tail of the previous one has run.
class WEB_API FrameScheduler {
    AK_ALLOC_WITH_KMALLOC;
    AK_MAKE_NONCOPYABLE(FrameScheduler);
    AK_MAKE_NONMOVABLE(FrameScheduler);

public:
    enum class State : u8 {
        // No rendering update is running, and none waits for its frame.
        Idle,
        // The rendering update runs its main half, and may be adding frames to the ticket.
        MainHalf,
        // The frame is in flight beside the main thread, which runs its event loop.
        InFlight,
        // Consume-commit runs.
        Consuming,
        // The frame is taken in; the tail waits for the top of the event loop.
        CommittedTailPending,
    };

    explicit FrameScheduler(EventLoop&);
    ~FrameScheduler();

    State state() const { return m_state; }

    // Whether a rendering update may submit its frame, rather than waiting for the render side.
    static bool submits_frames();

    // Whether the rendering update holds on to its rendering opportunity: until its tail has run, the next rendering
    // update does not start.
    bool holds_rendering_update() const { return m_state != State::Idle && m_state != State::MainHalf; }

    void begin_main_half(bool synchronous);
    // The recording mode of the main half's recordings.
    Painting::RecordingRun recording_run() const;
    // Whether the ticket is taking the main half's frames: a frame begun after one the render side records is finished
    // after it too, so frames reach their compositor contexts in paint order.
    bool ticket_takes_frames() const { return m_ticket && !m_ticket->navigables.is_empty(); }
    void add_to_ticket(LocalNavigable&, LocalNavigable::PendingCompositorFrame&&);
    // Ends the main half. Returns true if a frame is in flight, in which case the tail runs once it has been taken in.
    bool submit();

    // The event loop's finished frame consumer, called at step 1 once the render side has posted a frame completion:
    // takes in a finished frame, and runs the tail of a frame that is taken in, where the event loop lets it.
    void consume_finished_frame();
    // Waits for the frame in flight, takes it in and runs its tail. For a rendering update that has to start now.
    void finish_frame_now();

    // Takes in the frame the render side has handed back: publishes its recordings and hands off their compositor
    // frames. Runs no script: what the render side told the documents (their commit messages, which can dispatch
    // events) waits for the next rendering update or layout update to apply it.
    void consume_commit(EventLoop::FrameConsumeSite);

    EventLoop& event_loop() { return m_event_loop; }

    void visit_edges(JS::Cell::Visitor&);

private:
    void commit();
    void run_tail();

    EventLoop& m_event_loop;
    State m_state { State::Idle };
    bool m_synchronous_update { false };
    OwnPtr<FrameTicket> m_ticket;
};

}
