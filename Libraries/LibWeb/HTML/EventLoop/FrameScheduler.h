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
#include <LibCompositing/Types.h>
#include <LibGC/Function.h>
#include <LibGC/Ptr.h>
#include <LibJS/Heap/Cell.h>
#include <LibWeb/CSS/StyleEngineIdentifiers.h>
#include <LibWeb/Forward.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/LayoutRustFFI.h>

namespace Web::Layout::RustFFI {

struct ClockSender;

}

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
            // A clock lease's tick (LIBWEB_RENDER_CLOCK_FRAMES), which goes on at step 16 for every document.
            Clock,
            // A flight (LIBWEB_STAGE_OVERLAP naming flight): the style pass and the stages after it, up to the first
            // one that needs the main thread. The rendering update goes on where the flight ended.
            Flight,
        };
        Kind kind { Kind::Layout };
        Vector<GC::Ref<DOM::Document>> documents;
        size_t document_index { 0 };
        HighResolutionTime::DOMHighResTimeStamp frame_timestamp { 0 };
        // Where the flight ended, once consume-commit has taken it in.
        Optional<Layout::RustFFI::FfiFlightOutcome> flight_outcome {};
    };
    Optional<SubmittedPass> submitted_pass;

    // Whether the rest of the rendering update records in place instead of submitting its recordings: after a flight
    // whose recording did not stand, the rendering update records again at once rather than leave another frame in
    // flight a task later.
    bool records_in_place { false };
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
    // Whether the pass in flight is a flight that goes on to record its document: the recording the rendering update
    // goes on to make is in the frame in flight already.
    bool pass_in_flight_records() const;
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
    // LIBWEB_RENDER_CLOCK_FRAMES: at the end of a rendering update, grants a clock lease to every document whose next
    // rendering update would change nothing but what the running animations of its document timeline show, and ends
    // the lease of every other one.
    void grant_clock_leases();
    // Before a rendering update moves the documents' timelines to frame_timestamp: ends the leases the rendering update
    // cannot tick, so that the update samples their effects itself.
    void prepare_clock_ticks(ReadonlySpan<GC::Root<DOM::Document>> docs, HighResolutionTime::DOMHighResTimeStamp frame_timestamp);
    // The time the timeline of `document` reads in the rendering update running now, where a render clock that keeps up
    // ticks its lease: the time of its last tick, rather than the rendering update's.
    Optional<double> clock_lease_timeline_time(DOM::Document const&) const;
    // Ends the main half with a frame that ticks the lease of the first leased document from docs[first_document_index]
    // on, if there is one and `may_submit` says so. Once the frame is taken back and the document has adopted the tick,
    // the next leased document ticks, and then the rendering update goes on at step 16. Where no lease is left to tick,
    // ends every lease the rendering update did not tick, and returns false.
    bool tick_clock_leases(Vector<GC::Ref<DOM::Document>> const& docs, size_t first_document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp, bool may_submit);
    // Whether a render clock ticks the lease of `document`, without the main thread.
    bool render_clock_ticks(DOM::Document const&) const;
    // Ends every clock lease: the compositor went away, or the process is going.
    void revoke_all_clock_leases();
    // The main thread's outermost event loop is about to block: the render clock may tick the leases until it wakes.
    void main_thread_will_idle();
    // The main thread woke: the documents adopt what the render clock's ticks installed while it idled, and the render
    // clock goes on ticking the leases while the main thread runs its tasks.
    void main_thread_did_wake();
    // The main thread's outermost event loop went round without blocking: its tasks keep it busy. The documents adopt
    // what the render clock's ticks installed beside the tasks it ran, as they would had it idled, and the ticks go on
    // beside the next ones.
    void main_thread_did_not_block();
    // Something the main thread reached took back the arena of a leased document mid-task: the document's rows hold its
    // own records again, laid out at its own time.
    void clock_lend_taken_back(void* arena);
    // A read of render state ended: the render clock may go on ticking the leases if the task changed nothing since.
    void relend_clock_leases_after_read();
    // The main thread presented a frame of its own mid-task, which shows its documents at the task's time: the render
    // clock's ticks, which show them at theirs, present no more beside the task.
    void did_present_beside_clock_lend();
    // A render clock tick ended a lease; the rendering update takes over.
    void render_clock_needs_main();
    // For tests: whether leases are left to the main thread's rendering updates, with no render clock armed.
    void set_render_clock_suspended(bool);
    // For tests: hands the leases a render clock ticks a display tick at `frame_time` (unsafe shared current time, ms)
    // once the main thread goes idle, and calls `on_end` once it has run, with whether a lease took it.
    void inject_render_clock_tick(double frame_time, Function<void(bool)> on_end);

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

    // Has the style engine of `document` reclaim no style record until the frame in flight has been taken in. A
    // recording in flight reads the records its rows name, and an engine entry beside it can release the record a row
    // names before the row takes its new one at the arena's doors.
    void hold_style_records_for_frame(DOM::Document&);

    EventLoop& event_loop() { return m_event_loop; }

    void visit_edges(JS::Cell::Visitor&);

private:
    void submit_pass(FrameTicket::SubmittedPass::Kind, Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp);
    void commit();
    void run_tail();
    void resume_rendering_update_after_flight(FrameTicket::SubmittedPass const&);
    // Takes in the frame in flight, waiting for it if it has not finished, and runs its tail. Returns how long it
    // waited for the render side.
    u64 finish_one_frame();
    void apply_deferred_arena_changes();

    struct ClockLeaseHold {
        GC::Ref<DOM::Document> document;
        // The effects the lease ticks, which the main thread does not sample while it holds.
        Vector<GC::Ref<Animations::KeyframeEffect>> effects;
        // Whether the rendering update running now ticked the lease.
        bool ticked { false };
        // Whether the rendering update running now renders the lease's document: one that renders only other documents
        // (another top-level traversable's, at its own rendering opportunity) leaves the lease as it is.
        bool renders_in_update { true };
        // The compositor context at whose display ticks a render clock ticks the lease, if one does.
        Optional<Compositing::CompositorContextId> render_clock_context {};
        // The time the document timeline reads in the rendering update running now: that of the render clock's last
        // tick, where the rendering update leaves the lease to the render clock.
        Optional<double> timeline_time_for_update {};
        // What the render clock's ticks present the document's frames with.
        OwnPtr<LocalNavigable::RenderClockFrameKit> render_clock_kit {};
        // The document's layout commit generation and published style transaction when the ticks last had its arena,
        // or when the main thread last took it back from them: a task that moves either changed what a tick would show.
        u64 lend_layout_commit_generation { 0 };
        u64 lend_style_transaction { 0 };
        // Whether the lease ends once the document has adopted the tick in flight: it was revoked beside it.
        bool revoke_at_adoption { false };
    };
    void lend_clock_leases_to_busy_main(bool relend);
    void take_back_clock_lend_for_adoption();
    void adopt_render_clock_ticks();
    enum class ClockLendSuspension : u8 {
        // The task changed what a tick would show.
        Write,
        // The task's restores took too much of its time, or only a layout update of the main thread's could lay them out.
        Budget,
    };
    void suspend_clock_lend(ClockLendSuspension);
    void replace_render_clock_kit(ClockLeaseHold&, OwnPtr<LocalNavigable::RenderClockFrameKit>);
    void update_render_clock(ClockLeaseHold&, Optional<Compositing::CompositorContextId>);
    bool publish_clock_lease_targets(ClockLeaseHold const&);
    bool submit_clock_tick(Vector<GC::Ref<DOM::Document>> const& docs, size_t first_document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp);
    // Ends the lease at `index`, or, where its tick is in flight, has it end once the document adopted the tick.
    void revoke_clock_lease(size_t index);
    bool clock_tick_in_flight_for(DOM::Document const&) const;
    void adopt_clock_tick(DOM::Document&);

    EventLoop& m_event_loop;
    State m_state { State::Idle };
    bool m_synchronous_update { false };
    OwnPtr<FrameTicket> m_ticket;

    Vector<ClockLeaseHold> m_clock_leases;
    bool m_render_clock_suspended { false };
    // Whether the main thread lent the leased arenas to the render clock since it last took in what the ticks sampled.
    bool m_clock_lent_this_wake { false };
    // Whether something the main thread reached took the leased arenas back from the render clock mid-task.
    bool m_clock_lend_taken_back { false };
    // Whether the main thread lends the arenas no more until it next idles.
    bool m_clock_lend_suspended { false };
    // When the main thread last woke, and how much of its time restores took since.
    u64 m_clock_lend_woke_at_nanoseconds { 0 };
    u64 m_clock_lend_restore_nanoseconds { 0 };

    // The display ticks tests injected, waiting for the main thread to go idle, and those the render side runs now.
    struct InjectedClockTick {
        double frame_time { 0 };
        Function<void(bool)> on_end;
    };
    Vector<InjectedClockTick> m_injected_clock_ticks;
    Vector<Function<void(bool)>> m_injected_clock_ticks_in_flight;
    Layout::RustFFI::ClockSender* m_injected_clock_tick_sender { nullptr };

    // In the order the changes were made, which is the order the arena takes them in.
    Vector<GC::Ref<GC::Function<void()>>> m_deferred_arena_changes;
    Vector<GC::Ref<DOM::Document>> m_documents_holding_style_records;
};

// LIBWEB_RENDER_CLOCK_FRAMES: A read of render state that script makes. Whatever it reaches may take the arenas of leased
// documents back from the render clock's ticks mid-task; once the outermost such read is over, the ticks may have them
// again (see FrameScheduler::relend_clock_leases_after_read()).
class ClockLendReadScope {
    AK_MAKE_NONCOPYABLE(ClockLendReadScope);
    AK_MAKE_NONMOVABLE(ClockLendReadScope);

public:
    ClockLendReadScope();
    ~ClockLendReadScope();

    static bool is_active();
};

}
