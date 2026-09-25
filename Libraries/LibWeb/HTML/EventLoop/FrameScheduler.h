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
#include <LibGC/Function.h>
#include <LibGC/Ptr.h>
#include <LibJS/Heap/Cell.h>
#include <LibWeb/CSS/StyleEngineIdentifiers.h>
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

    // A frame that runs a document's full layout pass or first style pass instead, and where the rendering update goes
    // on once the frame is taken back: step 16 for documents[document_index], which the pass laid out or styled.
    struct SubmittedPass {
        enum class Kind : u8 {
            Style,
            Layout,
        };
        Kind kind { Kind::Layout };
        Vector<GC::Ref<DOM::Document>> documents;
        size_t document_index { 0 };
        HighResolutionTime::DOMHighResTimeStamp frame_timestamp { 0 };
    };
    Optional<SubmittedPass> submitted_pass;
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

    void begin_main_half(bool synchronous);
    // The recording mode of the main half's recordings.
    Painting::RecordingRun recording_run() const;
    // Whether the ticket is taking the main half's frames: a frame begun after one the render side records is finished
    // after it too, so frames reach their compositor contexts in paint order.
    bool ticket_takes_frames() const { return m_ticket && !m_ticket->navigables.is_empty(); }
    // Whether the rendering update waits for its style or layout pass to be taken back, and goes on with the rest (its
    // recordings) once it is.
    bool awaits_pass() const { return m_ticket && m_ticket->submitted_pass.has_value(); }
    void add_to_ticket(LocalNavigable&, LocalNavigable::PendingCompositorFrame&&);
    // Ends the main half. Returns true if a frame is in flight, in which case the tail runs once it has been taken in.
    bool submit();
    // Ends the main half with a frame that runs the layout pass of documents[document_index], which the document has
    // submitted. Once the frame is taken back, its tail goes on with the rendering update at step 16 for that document,
    // as a main half of its own that may submit the recording.
    void submit_layout(Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp);
    // Ends the main half with a frame that runs the first style pass of documents[document_index], which the document
    // has submitted. Consume-commit finishes the document's style update; the tail then goes on with the rendering
    // update at step 16 for that document, as a main half of its own that may submit the layout pass and the recording.
    void submit_style(Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp);
    // Whether the frame in flight runs the style or layout pass of `document`. The ticket keeps its documents alive.
    bool pass_in_flight_holds(DOM::Document const&) const;

    // The event loop's finished frame consumer, called at step 1 once the render side has posted a frame completion:
    // takes in a finished frame, and runs the tail of a frame that is taken in, where the event loop lets it.
    void consume_finished_frame();
    // Waits for the frame in flight, takes it in and runs its tail. For a rendering update that has to start now.
    // Returns how long it waited for the render side.
    u64 finish_frame_now();
    // Takes in the frames in flight that the render side has finished and runs their tails, without waiting for one.
    // Returns whether none is left, i.e. whether a rendering update can start now without waiting.
    bool finish_finished_frames();
    // Whether a frame is in flight that the render side has not finished yet.
    bool has_unfinished_frame() const;

    // Takes in the frame the render side has handed back: publishes its recordings and hands off their compositor
    // frames. Runs no script: what the render side told the documents (their commit messages, which can dispatch
    // events) waits for the next rendering update or layout update to apply it.
    void consume_commit(EventLoop::FrameConsumeSite);

    // Whether a main-side change to the arena of `document` waits for the frame in flight instead of joining it: only
    // recordings, which read nothing of the style engine, and a layout pass, beside which what the document publishes
    // to its style engine waits for the pass, own the arena, so what the document goes on to do beside them reaches
    // the arena through the changes deferred here alone.
    static bool arena_changes_wait_for_frame(DOM::Document const&);

    // A change to an arena that waits for the frame in flight (see arena_changes_wait_for_frame()), which the recording
    // owns until it is taken in. The arena takes the change in once the frame has been taken in, right after its
    // consume-commit, where waiting for the frame at the change would have put it.
    void defer_arena_change(GC::Ref<GC::Function<void()>>);

    // Runs `change` on the arena of `document`, if it has one: now, or once the frame in flight has been taken in, if
    // changes to that arena wait for it. A deferred change keeps only the document alive, so `change` holds no GC
    // pointer of its own.
    static void change_arena(DOM::Document&, Function<void(Layout::NodeArena&)>);

    EventLoop& event_loop() { return m_event_loop; }

    void visit_edges(JS::Cell::Visitor&);

private:
    void submit_pass(FrameTicket::SubmittedPass::Kind, Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp);
    void commit();
    void run_tail();
    // Takes in the frame in flight, waiting for it if it has not finished, and runs its tail. Returns how long it
    // waited for the render side.
    u64 finish_one_frame();
    void apply_deferred_arena_changes();

    EventLoop& m_event_loop;
    State m_state { State::Idle };
    bool m_synchronous_update { false };
    OwnPtr<FrameTicket> m_ticket;

    // In the order the changes were made, which is the order the arena takes them in.
    Vector<GC::Ref<GC::Function<void()>>> m_deferred_arena_changes;
};

}
