/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/AnyOf.h>
#include <AK/HashMap.h>
#include <AK/Mutex.h>
#include <AK/NeverDestroyed.h>
#include <AK/Time.h>
#include <LibCore/EventLoop.h>
#include <LibWeb/Animations/Animation.h>
#include <LibWeb/Animations/AnimationEffect.h>
#include <LibWeb/Animations/DocumentTimeline.h>
#include <LibWeb/Animations/KeyframeEffect.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEffectDrain.h>
#include <LibWeb/Compositor/CompositorHost.h>
#include <LibWeb/Compositor/NavigablePresenter.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameCompletion.h>
#include <LibWeb/HTML/EventLoop/FrameInFlightReferences.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/Scripting/Environments.h>
#include <LibWeb/HTML/Window.h>
#include <LibWeb/HighResolutionTime/TimeOrigin.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Namespace.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/PendingDisplayListRecording.h>
#include <LibWeb/StyleEngineRustFFI.h>

namespace Web::HTML {

static FrameScheduler* s_frame_scheduler_with_host = nullptr;

// How long the main thread may spend putting the ticks' samples back under its tasks before the render clock leaves its
// arenas alone until the main thread next idles: each costs a layout of the animated rows. A task that reads once per
// frame spends a small share of its time on them; one whose restores cost it more than this share stops them.
static constexpr u64 clock_lend_restore_nanoseconds_allowed_anyway = 4'000'000;
static constexpr u64 clock_lend_restore_share_of_wake_divisor = 10;

// LIBWEB_RENDER_CLOCK_FRAMES: A render clock ticks the leases while the main thread idles, and tells it where a tick
// ended one. The stage thread reaches the main thread through this.
struct RenderClockNeedsMain {
    Mutex mutex;
    RefPtr<Core::WeakEventLoopReference> event_loop;
    bool queued { false };
};

static RenderClockNeedsMain& render_clock_needs_main()
{
    // The stage thread may post after the main thread has begun exiting, so this is never destroyed.
    static NeverDestroyed<RenderClockNeedsMain> s_needs_main;
    return *s_needs_main;
}

// How far behind a rendering update's time the last tick of a render clock may be for the rendering update to show that
// tick's time: a few display frames.
static constexpr double render_clock_lag_tolerance_milliseconds = 50;

// The kits the render clock's ticks present with, by the layout arena of their document. The main thread changes them
// only while no tick runs, and a tick reads them only while the main thread idles.
static Mutex& render_clock_kits_mutex()
{
    static NeverDestroyed<Mutex> s_mutex;
    return *s_mutex;
}

static HashMap<void*, LocalNavigable::RenderClockFrameKit*>& render_clock_kits()
{
    static NeverDestroyed<HashMap<void*, LocalNavigable::RenderClockFrameKit*>> s_kits;
    return *s_kits;
}

static void install_render_clock_host()
{
    if (!Layout::RustFFI::rust_clock_frames_enabled())
        return;
    Layout::RustFFI::rust_render_clock_set_present([](void* arena) -> bool {
        LocalNavigable::RenderClockFrameKit* kit = nullptr;
        {
            MutexLocker locker(render_clock_kits_mutex());
            kit = render_clock_kits().get(arena).value_or(nullptr);
        }
        if (!kit)
            return false;
        LocalNavigable::present_render_clock_frame(*kit);
        return true;
    });
    Layout::RustFFI::rust_render_clock_set_lend_taken_back([](void* arena) {
        if (s_frame_scheduler_with_host)
            s_frame_scheduler_with_host->clock_lend_taken_back(arena);
    });
    static Core::EventLoopIdleObserver const s_idle_observer {
        .will_block = [] {
            if (s_frame_scheduler_with_host)
                s_frame_scheduler_with_host->main_thread_will_idle(); },
        .did_wake = [] {
            if (s_frame_scheduler_with_host)
                s_frame_scheduler_with_host->main_thread_did_wake(); },
    };
    Core::set_idle_observer_for_current_thread(&s_idle_observer);
    {
        auto& needs_main = render_clock_needs_main();
        MutexLocker locker(needs_main.mutex);
        needs_main.event_loop = Core::EventLoop::current_weak();
    }
    Layout::RustFFI::rust_render_clock_set_wake_main([] {
        auto& needs_main = render_clock_needs_main();
        MutexLocker locker(needs_main.mutex);
        if (!needs_main.event_loop)
            return;
        if (auto event_loop = needs_main.event_loop->take(); event_loop.is_alive())
            event_loop->wake();
    });
    Layout::RustFFI::rust_render_clock_set_needs_main([](u64) {
        auto& needs_main = render_clock_needs_main();
        MutexLocker locker(needs_main.mutex);
        if (needs_main.queued || !needs_main.event_loop)
            return;
        auto event_loop = needs_main.event_loop->take();
        if (!event_loop.is_alive())
            return;
        needs_main.queued = true;
        event_loop->deferred_invoke([] {
            {
                auto& needs_main = render_clock_needs_main();
                MutexLocker locker(needs_main.mutex);
                needs_main.queued = false;
            }
            if (s_frame_scheduler_with_host)
                s_frame_scheduler_with_host->render_clock_needs_main();
        });
        event_loop->wake();
    });
}

// The render side reaches the main thread's scheduler through these. Installed once, by the first scheduler that
// submits a frame.
static void install_frame_scheduler_host(FrameScheduler& scheduler)
{
    if (!Layout::RustFFI::rust_stage_thread_wants_frame_scheduler_host())
        return;
    s_frame_scheduler_with_host = &scheduler;
    Layout::RustFFI::rust_stage_thread_set_frame_scheduler_host({
        .frame_completion_notify = [] { FrameCompletion::the().post(); },
        .consume_commit = [] { s_frame_scheduler_with_host->consume_commit(EventLoop::FrameConsumeSite::ForcedJoin); },
        .tearing_down_cells = [] { return GC::Heap::the().is_tearing_down_cells(); },
    });
    scheduler.event_loop().set_finished_frame_consumer(GC::create_function(GC::Heap::the(), [&scheduler] {
        scheduler.consume_finished_frame();
    }));
    install_render_clock_host();
}

FrameScheduler::FrameScheduler(EventLoop& event_loop)
    : m_event_loop(event_loop)
{
}

FrameScheduler::~FrameScheduler()
{
    Layout::RustFFI::rust_render_clock_sender_destroy(m_injected_clock_tick_sender);
}

bool FrameScheduler::submits_frames()
{
    return Layout::RustFFI::rust_stage_thread_wants_frame_scheduler_host() || s_frame_scheduler_with_host;
}

void FrameScheduler::begin_main_half(bool synchronous)
{
    // The rendering update moves the timelines on from where the render clock's ticks left them.
    take_back_clock_lend_for_adoption();
    // A rendering update that has to start now (a synchronous one, or one a rendering opportunity started while the
    // tail was still pending) finishes the previous frame first.
    if (m_state != State::Idle)
        finish_frame_now();
    VERIFY(m_state == State::Idle);
    VERIFY(!m_ticket);
    m_synchronous_update = synchronous;
    if (!synchronous && submits_frames()) {
        install_frame_scheduler_host(*this);
        // Only the main thread's own scheduler submits frames.
        if (s_frame_scheduler_with_host == this)
            m_ticket = make<FrameTicket>();
    }
    m_state = State::MainHalf;
}

Painting::RecordingRun FrameScheduler::recording_run() const
{
    if (m_state == State::MainHalf && m_ticket)
        return Painting::RecordingRun::InSubmittedFrame;
    return Painting::RecordingRun::Now;
}

void FrameScheduler::add_to_ticket(LocalNavigable& navigable, LocalNavigable::PendingCompositorFrame&& frame)
{
    VERIFY(m_state == State::MainHalf);
    VERIFY(m_ticket);
    // A second frame for the same navigable in one ticket would drop the first one.
    VERIFY(!m_ticket->navigables.first_matching([&](auto const& entry) { return entry.navigable.ptr() == &navigable; }).has_value());
    // The render side may already be recording this frame.
    hold_for_frame_in_flight(*frame.document);
    // Retiring the navigable's compositor context before consume-commit waits for the frame and takes it in.
    Optional<u64> held_compositor_context;
    if (navigable.has_compositor_context()) {
        held_compositor_context = navigable.compositor_context().id().value();
        Layout::RustFFI::rust_frame_hold_compositor_context(*held_compositor_context);
    }
    // Unless LIBWEB_RENDER_PRESENTS=0, the frame in flight presents the frame once it has recorded it. Frames reach their
    // compositor contexts in paint order, so once one frame of the ticket is presented by consume-commit instead, so
    // are the frames after it.
    bool const earlier_frame_is_presented_by_commit = any_of(m_ticket->navigables, [](auto const& entry) {
        return !entry.frame.presentation || !entry.frame.presentation->is_presented_by_frame_in_flight;
    });
    m_ticket->navigables.append({ navigable, move(frame), held_compositor_context });
    if (!earlier_frame_is_presented_by_commit)
        navigable.submit_presentation(m_ticket->navigables.last().frame);
}

bool FrameScheduler::submit()
{
    VERIFY(m_state == State::MainHalf);
    if (!m_ticket || m_ticket->navigables.is_empty()) {
        // Nothing went to the render side, or a forced join during the main half took all of it in already. Then the
        // rendering update goes on as one that painted in place.
        auto painted_local_roots = m_ticket ? move(m_ticket->painted_local_roots) : Vector<GC::Ref<LocalNavigable>> {};
        m_ticket = nullptr;
        m_state = State::Idle;
        for (auto navigable : painted_local_roots)
            navigable->page().process_screenshot_requests();
        return false;
    }
    m_state = State::InFlight;
    m_event_loop.did_submit_frame();
    // A forced join during the main half can take in a recording that was submitted before its navigable went into
    // the ticket. The frame has finished then, and its completion was taken with that join, so post one for step 1.
    if (!Layout::RustFFI::rust_stage_thread_has_frame_in_flight())
        FrameCompletion::the().post();
    return true;
}

void FrameScheduler::submit_layout(Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    // NB: The document submitted its layout pass as a flight under the same condition.
    auto kind = Layout::RustFFI::rust_stage_thread_submits_flight() ? FrameTicket::SubmittedPass::Kind::Flight : FrameTicket::SubmittedPass::Kind::Layout;
    submit_pass(kind, move(documents), document_index, frame_timestamp);
}

void FrameScheduler::submit_style(Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    // NB: The document submitted its style pass as a flight under the same condition.
    auto kind = Layout::RustFFI::rust_stage_thread_submits_flight() ? FrameTicket::SubmittedPass::Kind::Flight : FrameTicket::SubmittedPass::Kind::Style;
    submit_pass(kind, move(documents), document_index, frame_timestamp);
}

void FrameScheduler::submit_pass(FrameTicket::SubmittedPass::Kind kind, Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    VERIFY(m_state == State::MainHalf);
    // Only a main half with a ticket submits, and the pass comes before any recording.
    VERIFY(m_ticket);
    VERIFY(m_ticket->navigables.is_empty());
    m_ticket->submitted_pass = FrameTicket::SubmittedPass { kind, move(documents), document_index, frame_timestamp };
    m_state = State::InFlight;
    m_event_loop.did_submit_frame();
}

bool FrameScheduler::pass_in_flight_records() const
{
    if (!awaits_pass() || m_ticket->submitted_pass->kind != FrameTicket::SubmittedPass::Kind::Flight)
        return false;
    auto const& pass = *m_ticket->submitted_pass;
    auto navigable = pass.documents[pass.document_index]->navigable();
    return navigable && navigable->has_sealed_flight_paint();
}

bool FrameScheduler::pass_in_flight_holds(DOM::Document const& document) const
{
    if (m_state != State::InFlight || !m_ticket || !m_ticket->submitted_pass.has_value())
        return false;
    auto const& pass = *m_ticket->submitted_pass;
    return pass.documents[pass.document_index].ptr() == &document;
}

void FrameScheduler::consume_commit(EventLoop::FrameConsumeSite site)
{
    // A completion posted for the frame taken in here has nothing left to deliver.
    FrameCompletion::the().take();
    m_event_loop.consume_commit(site, [this] { commit(); });
    // A forced join leaves the tail of a submitted frame for the next step 1, and nothing else would call the
    // consumer again: no completion is pending anymore.
    if (site == EventLoop::FrameConsumeSite::ForcedJoin && m_state == State::CommittedTailPending) {
        m_event_loop.call_finished_frame_consumer_again();
        m_event_loop.schedule();
    }
}

void FrameScheduler::commit()
{
    VERIFY(m_ticket);
    VERIFY(!Layout::RustFFI::rust_stage_thread_has_frame_in_flight());
    // A forced join during the main half takes in the frames submitted so far; the main half goes on.
    auto const state_after = m_state == State::MainHalf ? State::MainHalf : State::CommittedTailPending;
    VERIFY(m_state == State::MainHalf || m_state == State::InFlight);
    // A frame is counted as submitted once the main half has ended.
    bool const counts_as_a_frame = m_state == State::InFlight;
    auto start_nanoseconds = MonotonicTime::now().nanoseconds();
    m_state = State::Consuming;
    // NB: The layout pass's frame has nothing to publish: its take-back ended the document's layout update already. The
    //     style pass's frame ends the document's style update here: its drain installs what the pass computed.
    if (m_ticket->submitted_pass.has_value() && m_ticket->submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Style)
        m_ticket->submitted_pass->documents[m_ticket->submitted_pass->document_index]->finish_submitted_style_update();
    // A flight that began with the style pass ends the document's style update here as a style pass's frame does. One
    // that began with the layout pass took its layout frame back already.
    if (m_ticket->submitted_pass.has_value() && m_ticket->submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Flight) {
        auto outcome = Layout::RustFFI::rust_flight_take_outcome();
        m_ticket->submitted_pass->flight_outcome = outcome;
        auto document = m_ticket->submitted_pass->documents[m_ticket->submitted_pass->document_index];
        if (outcome.began == Layout::RustFFI::FfiFlightStage::Style)
            document->finish_submitted_style_update();
        // A flight that went on from the layout pass to record the document hands its frame off here, as the frame of a
        // recording in flight is.
        if (auto navigable = document->navigable(); navigable && navigable->has_sealed_flight_paint()) {
            using FlightPaintEnd = LocalNavigable::FlightPaintEnd;
            auto end = FlightPaintEnd::NotRecorded;
            if (outcome.reached >= Layout::RustFFI::FfiFlightStage::Record)
                end = FlightPaintEnd::Recorded;
            else if (outcome.end == Layout::RustFFI::FfiFlightEndReason::HostLeftWork)
                end = FlightPaintEnd::RecordedAheadOfMoreWork;
            if (navigable->finish_flight_paint(*document, end)) {
                m_event_loop.note_frame_painted({});
                m_ticket->painted_local_roots.append(*navigable);
            }
        }
    }
    // A clock tick's document adopts what the tick installed before anything reads it, and then takes in what was
    // marked beside the tick, as the end of a layout pass's frame does: the next drain writes it.
    if (m_ticket->submitted_pass.has_value() && m_ticket->submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Clock) {
        auto& document = m_ticket->submitted_pass->documents[m_ticket->submitted_pass->document_index];
        adopt_clock_tick(document);
        document->release_held_invalidation_marks();
    }
    // NB: Each navigable's recording is published and its resources are added to its resource storage before its
    //     compositor frame is built and handed off, so a compositor frame never reaches its sink ahead of the
    //     resources it references. The canvases it shows were flushed before the recording was prepared, and the next
    //     flush waits for the next rendering update.
    auto navigables = move(m_ticket->navigables);
    for (auto& [navigable, frame, held_compositor_context] : navigables) {
        // FIXME: A navigable destroyed while its frame was in flight retires the frame instead of finishing it.
        navigable->finish_painting_next_frame(frame);
        if (held_compositor_context.has_value())
            Layout::RustFFI::rust_frame_release_compositor_context(*held_compositor_context);
        m_event_loop.note_frame_painted({});
        if (navigable->is_local_root())
            m_ticket->painted_local_roots.append(navigable);
    }
    release_holds_for_frame_in_flight();
    apply_deferred_arena_changes();
    m_state = state_after;
    if (counts_as_a_frame)
        m_event_loop.did_consume_frame_commit(MonotonicTime::now().nanoseconds() - start_nanoseconds);
}

void FrameScheduler::consume_finished_frame()
{
    FrameCompletion::the().take();
    if (m_state == State::InFlight && Layout::RustFFI::rust_stage_thread_frame_in_flight_has_finished()) {
        if (!m_event_loop.may_consume_commit(EventLoop::FrameConsumeSite::StepOne)) {
            m_event_loop.call_finished_frame_consumer_again();
            return;
        }
        Layout::RustFFI::rust_stage_thread_take_frame_in_flight();
        consume_commit(EventLoop::FrameConsumeSite::StepOne);
    }
    if (m_state != State::CommittedTailPending)
        return;
    if (!m_event_loop.may_run_consume_tail()) {
        m_event_loop.call_finished_frame_consumer_again();
        return;
    }
    if (m_ticket->submitted_pass.has_value()) {
        // The tail of a style or layout pass's frame is the rest of its rendering update, a main half whose forced joins take in
        // the recordings it submits, so it does not run as a consume-tail. It ends like the rendering task it goes on
        // with.
        run_tail();
        m_event_loop.perform_a_microtask_checkpoint();
        return;
    }
    m_event_loop.run_consume_tail([this] { run_tail(); });
}

u64 FrameScheduler::finish_frame_now()
{
    u64 waited_nanoseconds = 0;
    // NB: The tail of a style or layout pass's frame goes on with its rendering update, which can submit the recording.
    while (m_state != State::Idle)
        waited_nanoseconds += finish_one_frame();
    return waited_nanoseconds;
}

bool FrameScheduler::finish_finished_frames()
{
    // NB: A finished style or layout pass's tail can submit the next stage, which the loop then finds unfinished.
    while (m_state != State::Idle) {
        if (has_unfinished_frame())
            return false;
        finish_one_frame();
    }
    return true;
}

u64 FrameScheduler::finish_one_frame()
{
    u64 waited_nanoseconds = 0;
    if (m_state == State::InFlight) {
        auto wait_start_nanoseconds = MonotonicTime::now().nanoseconds();
        Layout::RustFFI::rust_stage_thread_take_frame_in_flight();
        waited_nanoseconds = MonotonicTime::now().nanoseconds() - wait_start_nanoseconds;
        consume_commit(EventLoop::FrameConsumeSite::ForcedJoin);
    }
    // NB: The tail is the rest of the previous rendering update, which the rendering update starting now has to
    //     follow. It runs here, where a lockstep frame would have run it too.
    VERIFY(m_state == State::CommittedTailPending);
    run_tail();
    return waited_nanoseconds;
}

bool FrameScheduler::has_unfinished_frame() const
{
    return m_state == State::InFlight && !Layout::RustFFI::rust_stage_thread_frame_in_flight_has_finished();
}

void FrameScheduler::run_tail()
{
    VERIFY(m_state == State::CommittedTailPending);
    // NB: The stack is a conservative root, so what the ticket held stays alive in these locals.
    if (auto submitted_pass = move(m_ticket->submitted_pass); submitted_pass.has_value()) {
        // The rest of the rendering update is a main half of its own, with a new ticket for its recordings. A flight that
        // recorded painted its local root already, whose screenshots wait for the end of the rendering update.
        auto painted_local_roots = move(m_ticket->painted_local_roots);
        m_ticket = make<FrameTicket>();
        m_ticket->painted_local_roots = move(painted_local_roots);
        m_state = State::MainHalf;
        if (submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Clock) {
            // The next leased document ticks, and then the rendering update goes on at step 16 for every document.
            if (tick_clock_leases(submitted_pass->documents, submitted_pass->document_index + 1, submitted_pass->frame_timestamp, true))
                return;
            m_event_loop.resume_rendering_update_after_style({}, submitted_pass->documents, 0, submitted_pass->frame_timestamp);
        } else if (submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Style)
            m_event_loop.resume_rendering_update_after_style({}, submitted_pass->documents, submitted_pass->document_index, submitted_pass->frame_timestamp);
        else if (submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Flight)
            resume_rendering_update_after_flight(*submitted_pass);
        else
            m_event_loop.resume_rendering_update_after_layout({}, submitted_pass->documents, submitted_pass->document_index, submitted_pass->frame_timestamp);
        return;
    }
    auto painted_local_roots = move(m_ticket->painted_local_roots);
    m_ticket = nullptr;
    m_state = State::Idle;
    auto start_nanoseconds = MonotonicTime::now().nanoseconds();
    m_event_loop.run_rendering_update_tail({}, painted_local_roots);
    m_event_loop.did_consume_frame_tail(MonotonicTime::now().nanoseconds() - start_nanoseconds);
}

// A flight that ended goes on as the rendering update would have gone on had it submitted the last stage the flight ran
// on its own.
void FrameScheduler::resume_rendering_update_after_flight(FrameTicket::SubmittedPass const& flight)
{
    using Layout::RustFFI::FfiFlightStage;
    auto reached = flight.flight_outcome->reached;
    switch (reached) {
    case FfiFlightStage::Style:
    case FfiFlightStage::StyleRenderHalf:
        m_event_loop.resume_rendering_update_after_style({}, flight.documents, flight.document_index, flight.frame_timestamp);
        return;
    case FfiFlightStage::Rounds:
    case FfiFlightStage::PaintPrep:
        m_event_loop.resume_rendering_update_after_layout({}, flight.documents, flight.document_index, flight.frame_timestamp);
        return;
    // NB: Consume-commit handed off the frame the flight recorded. The rest of the rendering update runs as after the
    //     layout pass, and paints again only what was marked beside the flight.
    case FfiFlightStage::Record:
    case FfiFlightStage::Present:
        m_event_loop.resume_rendering_update_after_layout({}, flight.documents, flight.document_index, flight.frame_timestamp);
        return;
    }
    VERIFY_NOT_REACHED();
}

bool FrameScheduler::arena_changes_wait_for_frame(DOM::Document const& document)
{
    auto const* arena = document.layout_node_arena_if_created();
    return arena && Layout::RustFFI::rust_stage_thread_arena_changes_wait_for_frame(arena->handle());
}

void FrameScheduler::defer_arena_change(GC::Ref<GC::Function<void()>> change)
{
    m_deferred_arena_changes.append(change);
}

void FrameScheduler::change_arena(DOM::Document& document, Function<void(Layout::NodeArena&)> change)
{
    auto* arena = document.layout_node_arena_if_created();
    if (!arena)
        return;
    if (!arena_changes_wait_for_frame(document)) {
        change(*arena);
        return;
    }
    main_thread_event_loop().frame_scheduler().defer_arena_change(GC::create_function(document.heap(), [document = GC::Ref { document }, change = move(change)] {
        if (auto* arena = document->layout_node_arena_if_created())
            change(*arena);
    }));
}

void FrameScheduler::hold_style_records_for_frame(DOM::Document& document)
{
    if (m_documents_holding_style_records.contains_slow(GC::Ref { document }))
        return;
    m_documents_holding_style_records.append(document);
    document.style_computer().style_engine().begin_pin_waiting_for_frame();
}

void FrameScheduler::apply_deferred_arena_changes()
{
    // A change applied here can defer no other: nothing is in flight anymore.
    auto changes = move(m_deferred_arena_changes);
    for (auto const& change : changes)
        change->function()();
    for (auto const& document : exchange(m_documents_holding_style_records, {}))
        document->style_computer().style_engine().end_pin_waiting_for_frame();
}

// Takes in what a clock lease's ticks laid out on the render side while the main thread idled, as a layout update of
// its own. Returns whether they had laid anything out.
static bool take_in_clock_layout_frame(DOM::Document& document)
{
    auto* arena = document.layout_node_arena_if_created();
    if (!arena || !Layout::RustFFI::layout_arena_clock_layout_frame_laid_out(arena->handle()))
        return false;
    // The ticks' rounds ran as the rest of a layout update, which begins and ends here, around their frame: its end is
    // taken in as a submitted pass's frame is taken back (finish_update_layout in the host callbacks).
    Layout::RustFFI::layout_arena_begin_update_layout(arena->handle());
    document.begin_style_stabilization_epoch();
    document.style_computer().begin_style_record_view_epoch();
    return Layout::RustFFI::layout_arena_take_in_clock_layout_frame(arena->handle());
}

// What a clock lease of a document ticks, and until when.
struct ClockLeasePlan {
    Vector<GC::Ref<Animations::KeyframeEffect>> effects;
    // The timeline time at which the document has something observable to do: an event, a phase change, the end of an
    // effect. No tick samples at or past it.
    double deadline { AK::Infinity<double> };
};

// The next time, in the local time of `effect`, at which its phase or current iteration changes.
static Optional<double> next_boundary_in_local_time(Animations::KeyframeEffect const& effect, double local_time)
{
    if (effect.start_delay().type != Animations::TimeValue::Type::Milliseconds
        || effect.iteration_duration().type != Animations::TimeValue::Type::Milliseconds
        || effect.active_duration().type != Animations::TimeValue::Type::Milliseconds)
        return {};
    auto start_delay = effect.start_delay().value;
    auto iteration_duration = effect.iteration_duration().value;
    auto active_end = start_delay + effect.active_duration().value;
    if (local_time < start_delay)
        return start_delay;
    if (!(iteration_duration > 0) || local_time >= active_end)
        return {};
    auto next_iteration_start = start_delay + (floor((local_time - start_delay) / iteration_duration) + 1) * iteration_duration;
    return min(next_iteration_start, active_end);
}

// Whether a clock lease can tick the running animations of `document`, and which of their effects it ticks. The lease
// ticks only what the main thread would do nothing else for until the deadline: the running, non-pending animations
// of the document timeline that animate no property that can change the layout tree's shape or what the main thread
// observes of it, on elements that have a box.
static Optional<ClockLeasePlan> clock_lease_plan(DOM::Document& document)
{
    if (!Layout::RustFFI::rust_stage_thread_submits_clock())
        return {};
    if (!document.is_fully_active() || document.hidden() || document.is_decoded_svg())
        return {};
    auto navigable = document.navigable();
    if (!navigable || navigable->active_document().ptr() != &document || !document.layout_node_arena_if_created())
        return {};
    if (document.needs_animated_style_update() || !document.layout_is_up_to_date() || document.layout_overlap_blocker().has_value())
        return {};
    if (auto window = document.window(); !window || window->has_animation_frame_callbacks())
        return {};
    auto timeline = document.timeline();
    auto timeline_time = timeline->current_time();
    if (!timeline_time.has_value() || timeline_time->type != Animations::TimeValue::Type::Milliseconds)
        return {};

    ClockLeasePlan plan;
    for (auto const& associated_timeline : document.associated_animation_timelines()) {
        for (auto& animation : associated_timeline->associated_animations()) {
            if (animation.play_state() != Bindings::AnimationPlayState::Running)
                continue;
            if (animation.pending() || associated_timeline.ptr() != timeline.ptr() || !(animation.playback_rate() > 0))
                return {};
            auto effect = animation.effect();
            if (!effect || !is<Animations::KeyframeEffect>(*effect))
                return {};
            auto& keyframe_effect = static_cast<Animations::KeyframeEffect&>(*effect);
            auto target = keyframe_effect.target();
            if (!target || &target->document() != &document || !target->is_connected() || keyframe_effect.pseudo_element_type().has_value())
                return {};
            if (target->namespace_uri() != Namespace::HTML || !target->unsafe_layout_node())
                return {};
            if (auto const* key_frame_set = keyframe_effect.key_frame_set()) {
                for (auto const& keyframe : key_frame_set->keyframes_by_key) {
                    for (auto const& [property, value] : keyframe.properties) {
                        switch (property.id()) {
                        case CSS::PropertyID::Custom:
                        case CSS::PropertyID::Display:
                        case CSS::PropertyID::Visibility:
                        case CSS::PropertyID::ContentVisibility:
                            return {};
                        default:
                            break;
                        }
                    }
                }
            }
            auto local_time = keyframe_effect.local_time();
            if (!local_time.has_value() || local_time->type != Animations::TimeValue::Type::Milliseconds)
                return {};
            auto boundary = next_boundary_in_local_time(keyframe_effect, local_time->value);
            if (!boundary.has_value())
                return {};
            plan.deadline = min(plan.deadline, timeline_time->value + (*boundary - local_time->value) / animation.playback_rate());
            // What the compositor or the offscreen throttle runs, the main thread does not sample per frame either.
            if (keyframe_effect.is_compositor_driven() || keyframe_effect.is_compositor_replaced() || keyframe_effect.can_skip_per_frame_style_update())
                continue;
            plan.effects.append(keyframe_effect);
        }
    }
    if (plan.effects.is_empty() || !(plan.deadline > timeline_time->value))
        return {};
    return plan;
}

bool FrameScheduler::clock_tick_in_flight_for(DOM::Document const& document) const
{
    if (m_state != State::InFlight || !m_ticket || !m_ticket->submitted_pass.has_value())
        return false;
    auto const& pass = *m_ticket->submitted_pass;
    return pass.kind == FrameTicket::SubmittedPass::Kind::Clock && pass.documents[pass.document_index].ptr() == &document;
}

void FrameScheduler::revoke_clock_lease(size_t index)
{
    // A tick in flight installs its samples in the arena ahead of the document, which adopts them when it takes the
    // frame in: the lease ends there, and nothing ticks it meanwhile.
    if (clock_tick_in_flight_for(*m_clock_leases[index].document)) {
        auto& hold = m_clock_leases[index];
        hold.revoke_at_adoption = true;
        if (auto armed = exchange(hold.render_clock_context, {}); armed.has_value())
            hold.document->page().client().disarm_render_clock(*armed);
        if (auto* arena = hold.document->layout_node_arena_if_created())
            Layout::RustFFI::rust_clock_lease_set_paused(arena->handle(), true);
        return;
    }
    // No tick may run beside what ending the lease reaches, and the documents take in what the ticks sampled first.
    auto document = m_clock_leases[index].document;
    take_back_clock_lend_for_adoption();
    auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == document.ptr(); });
    if (!held.has_value())
        return;
    auto hold = m_clock_leases.take(*held);
    // What the render clock's ticks laid out and presented goes in before the lease that holds it ends.
    take_in_clock_layout_frame(*hold.document);
    replace_render_clock_kit(hold, {});
    if (auto* arena = hold.document->layout_node_arena_if_created())
        Layout::RustFFI::rust_clock_lease_revoke(arena->handle());
    if (hold.render_clock_context.has_value())
        hold.document->page().client().disarm_render_clock(*hold.render_clock_context);
    // The main thread samples the effects again, at the time it moves their timeline to.
    for (auto effect : hold.effects) {
        effect->set_is_clock_driven(false);
        if (effect->target() && effect->associated_animation())
            effect->target()->document().set_needs_animated_style_update(*effect);
    }
}

void FrameScheduler::grant_clock_leases()
{
    if (!Layout::RustFFI::rust_clock_frames_enabled())
        return;
    auto documents = m_synchronous_update ? Vector<GC::Root<DOM::Document>> {} : m_event_loop.documents_in_this_event_loop_matching([](auto&) { return true; });
    for (auto& hold : m_clock_leases)
        hold.timeline_time_for_update.clear();
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        if (!documents.first_matching([&](auto const& document) { return document.ptr() == m_clock_leases[index].document.ptr(); }).has_value())
            revoke_clock_lease(index);
    }
    for (auto& document : documents) {
        auto plan = clock_lease_plan(*document);
        auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == document.ptr(); });
        if (!plan.has_value()) {
            if (held.has_value())
                revoke_clock_lease(*held);
            continue;
        }
        if (held.has_value()) {
            for (auto effect : m_clock_leases[*held].effects)
                effect->set_is_clock_driven(false);
            m_clock_leases[*held].effects = plan->effects;
        } else {
            m_clock_leases.append({ *document, plan->effects });
        }
        for (auto effect : plan->effects)
            effect->set_is_clock_driven(true);
        auto timeline = document->timeline();
        auto timeline_time = timeline->current_time()->value;
        // NB: The default document timeline's origin time is zero: its time is the document's relative time.
        auto now = HighResolutionTime::unsafe_shared_current_time();
        auto timeline_zero = now - HighResolutionTime::relative_high_resolution_time(now, relevant_global_object(*document));
        // A render clock ticks the lease at the display ticks of the document's compositor context.
        u64 context_id = 0;
        if (auto navigable = document->navigable(); navigable && navigable->has_compositor_context())
            context_id = navigable->compositor_context().id().value();
        Layout::RustFFI::rust_clock_lease_grant(document->layout_node_arena_if_created()->handle(), context_id, timeline->style_engine_identity(), timeline_zero, timeline_time, plan->deadline);
        auto& hold = *m_clock_leases.find_if([&](auto const& hold) { return hold.document.ptr() == document.ptr(); });
        publish_clock_lease_targets(hold);
        update_render_clock(hold, context_id ? Optional<Compositing::CompositorContextId> { context_id } : OptionalNone {});
        // The render clock's ticks lay out what their samples leave in a frame of their own, and present it as the frame
        // the navigable just painted was presented.
        OwnPtr<LocalNavigable::RenderClockFrameKit> kit;
        if (hold.render_clock_context.has_value()) {
            Layout::RustFFI::layout_arena_renew_clock_layout_frame(document->layout_node_arena_if_created()->handle());
            if (auto sealed = document->navigable()->seal_render_clock_frame_kit(); sealed.has_value())
                kit = make<LocalNavigable::RenderClockFrameKit>(sealed.release_value());
        }
        replace_render_clock_kit(hold, move(kit));
    }
}

// Takes in what the ticks presented from the lease's kit, and has them present from `kit` from now on.
void FrameScheduler::replace_render_clock_kit(ClockLeaseHold& hold, OwnPtr<LocalNavigable::RenderClockFrameKit> kit)
{
    auto* arena = hold.document->layout_node_arena_if_created();
    if (hold.render_clock_kit) {
        if (auto navigable = hold.document->navigable())
            navigable->adopt_render_clock_frame_kit(*hold.render_clock_kit);
    }
    {
        MutexLocker locker(render_clock_kits_mutex());
        if (arena) {
            if (kit)
                render_clock_kits().set(arena->handle(), kit.ptr());
            else
                render_clock_kits().remove(arena->handle());
        }
    }
    hold.render_clock_kit = move(kit);
}

// Arms the render clock for the compositor context `context` of the lease's document, or disarms it without one.
void FrameScheduler::update_render_clock(ClockLeaseHold& hold, Optional<Compositing::CompositorContextId> context)
{
    if (m_render_clock_suspended)
        context = {};
    if (hold.render_clock_context == context)
        return;
    auto& client = hold.document->page().client();
    if (auto armed = exchange(hold.render_clock_context, {}); armed.has_value())
        client.disarm_render_clock(*armed);
    if (context.has_value() && client.arm_render_clock(*context))
        hold.render_clock_context = context;
}

// Tells the lease which elements it ticks, with the records they hold now: a tick samples each element over it.
bool FrameScheduler::publish_clock_lease_targets(ClockLeaseHold const& hold)
{
    auto* arena = hold.document->layout_node_arena_if_created();
    if (!arena)
        return false;
    Vector<u32> style_nodes;
    Vector<u64> style_records;
    for (auto effect : hold.effects) {
        auto target = effect->target();
        if (!target || style_nodes.contains_slow(target->style_node_id().value()))
            continue;
        // The element's layout node is built while that record is live.
        (void)target->unsafe_layout_node();
        style_nodes.append(target->style_node_id().value());
        style_records.append(DOM::AbstractElement { *target }.style_record_identity().value());
    }
    Layout::RustFFI::rust_clock_lease_set_targets(arena->handle(), style_nodes.data(), style_records.data(), style_nodes.size());
    return true;
}

bool FrameScheduler::render_clock_ticks(DOM::Document const& document) const
{
    auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; });
    return held.has_value() && m_clock_leases[*held].render_clock_context.has_value();
}

void FrameScheduler::revoke_all_clock_leases()
{
    // A lease whose tick is in flight stays until the tick is in, and ending one can end others with it.
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        if (index < m_clock_leases.size())
            revoke_clock_lease(index);
    }
}

void FrameScheduler::main_thread_will_idle()
{
    // The task is over: what the ticks installed beside it is the documents' now, and nothing lends the arenas again
    // until the main thread wakes.
    m_clock_lend_suspended = true;
    take_back_clock_lend_for_adoption();
    // A tick a test injected finds no lease to take it.
    if (m_clock_leases.is_empty() && !m_injected_clock_ticks.is_empty()) {
        auto ticks = move(m_injected_clock_ticks);
        Core::deferred_invoke([ticks = move(ticks)] mutable {
            for (auto& tick : ticks)
                tick.on_end(false);
        });
    }
    // Nothing ticks beside a frame in flight: the main thread takes it back first.
    if (m_clock_leases.is_empty() || m_state != State::Idle || Layout::RustFFI::rust_stage_thread_has_frame_in_flight())
        return;
    // A tick samples each element over the record it holds, which the main thread may have moved since the grant: a
    // lease its document no longer plans the same way waits for the rendering update that ends it.
    bool any_ticks = false;
    for (auto& hold : m_clock_leases) {
        auto* arena = hold.document->layout_node_arena_if_created();
        if (!arena)
            continue;
        // A lease no render clock is armed for takes no tick, whichever context it was granted for.
        if (!hold.render_clock_context.has_value()) {
            Layout::RustFFI::rust_clock_lease_set_paused(arena->handle(), true);
            continue;
        }
        auto plan = clock_lease_plan(*hold.document);
        bool ticks = plan.has_value() && plan->effects == hold.effects && publish_clock_lease_targets(hold);
        Layout::RustFFI::rust_clock_lease_set_paused(arena->handle(), !ticks);
        // The ticks lay out with the document as it stands now: a resize or a selection change since the last frame
        // was laid out by a read, but nothing painted it yet.
        if (ticks)
            Layout::RustFFI::layout_arena_renew_clock_layout_frame(arena->handle());
        any_ticks |= ticks;
    }
    if (any_ticks)
        Layout::RustFFI::rust_render_clock_main_will_idle();

    // The ticks a test injected run now, as the render clock's would, and the main thread waits for them when it wakes.
    if (m_injected_clock_ticks.is_empty())
        return;
    auto ticks = move(m_injected_clock_ticks);
    if (any_ticks && !m_injected_clock_tick_sender)
        m_injected_clock_tick_sender = Layout::RustFFI::rust_render_clock_sender_create();
    for (auto& tick : ticks) {
        bool injected = false;
        for (auto& hold : m_clock_leases) {
            if (!any_ticks || !m_injected_clock_tick_sender || !hold.render_clock_context.has_value())
                continue;
            auto frame_time_nanoseconds = static_cast<i64>(tick.frame_time * 1'000'000.0);
            injected |= Layout::RustFFI::rust_render_clock_inject_tick(m_injected_clock_tick_sender, hold.render_clock_context->value(), frame_time_nanoseconds);
        }
        if (injected) {
            m_injected_clock_ticks_in_flight.append(move(tick.on_end));
            continue;
        }
        Core::deferred_invoke([on_end = move(tick.on_end)] { on_end(false); });
    }
}

void FrameScheduler::inject_render_clock_tick(double frame_time, Function<void(bool)> on_end)
{
    // Without a render clock host, the main thread never lets a tick in, and without a lease a render clock would tick,
    // no lease takes it.
    bool const render_clock_ticks_a_lease = any_of(m_clock_leases, [](auto const& hold) { return hold.render_clock_context.has_value(); });
    if (!Layout::RustFFI::rust_stage_thread_submits_clock() || s_frame_scheduler_with_host != this || !render_clock_ticks_a_lease) {
        Core::deferred_invoke([on_end = move(on_end)] { on_end(false); });
        return;
    }
    m_injected_clock_ticks.append({ frame_time, move(on_end) });
}

void FrameScheduler::main_thread_did_wake()
{
    m_clock_lend_taken_back = false;
    m_clock_lend_suspended = false;
    m_clock_lend_woke_at_nanoseconds = MonotonicTime::now().nanoseconds();
    m_clock_lend_restore_nanoseconds = 0;
    if (Layout::RustFFI::rust_render_clock_main_did_wake())
        adopt_render_clock_ticks();
    // The ticks a test injected have run: what waits for them goes on once the documents adopted them.
    if (!m_injected_clock_ticks_in_flight.is_empty()) {
        auto ends = move(m_injected_clock_ticks_in_flight);
        Core::deferred_invoke([ends = move(ends)] mutable {
            for (auto& end : ends)
                end(true);
        });
    }
    // The render clock goes on ticking the leases while the main thread runs its tasks.
    lend_clock_leases_to_busy_main(false);
}

// Takes back the arenas lent to the render clock's ticks while the main thread ran a task, and has the documents adopt
// what the ticks installed, as they do when the main thread wakes.
void FrameScheduler::take_back_clock_lend_for_adoption()
{
    if (!exchange(m_clock_lent_this_wake, false))
        return;
    // What a read put back under the task, the documents adopt too.
    if (Layout::RustFFI::rust_clock_lend_end_for_adoption())
        adopt_render_clock_ticks();
    Layout::RustFFI::rust_clock_lend_release_host_pins();
}

void FrameScheduler::adopt_render_clock_ticks()
{
    // What the render clock's ticks installed ahead of the main thread, each document adopts before anything else
    // reaches it, and its timeline shows the time of the last tick.
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        auto document = m_clock_leases[index].document;
        auto* arena = document->layout_node_arena_if_created();
        if (!arena || !m_clock_leases[index].render_clock_context.has_value())
            continue;
        auto time = Layout::RustFFI::rust_clock_lease_time(arena->handle());
        adopt_clock_tick(*document);
        // The style the ticks installed is the document's now, and so is what they laid out with it, and what they
        // presented.
        take_in_clock_layout_frame(*document);
        if (index < m_clock_leases.size() && m_clock_leases[index].document.ptr() == document.ptr() && m_clock_leases[index].render_clock_kit) {
            if (auto navigable = document->navigable())
                navigable->adopt_render_clock_frame_kit(*m_clock_leases[index].render_clock_kit);
        }
        if (!isnan(time)) {
            if (auto current = document->timeline()->current_time(); current.has_value() && current->type == Animations::TimeValue::Type::Milliseconds && current->value < time)
                document->timeline()->update_current_time(time);
        }
    }
}

// The render clock goes on ticking the leases while the main thread runs a task. A lent arena stands in the frame in
// flight, so whatever the task reaches of it or of its style engine takes it back first, and the document's rows hold
// its own records again, laid out at its own time (see clock_lend_taken_back()).
void FrameScheduler::lend_clock_leases_to_busy_main(bool relend)
{
    if (m_clock_leases.is_empty() || m_clock_lend_suspended || m_render_clock_suspended || m_state != State::Idle)
        return;
    if (!Layout::RustFFI::rust_clock_frames_enabled() || Layout::RustFFI::rust_clock_lend_is_active() || Layout::RustFFI::rust_stage_thread_has_frame_in_flight())
        return;
    // A tick would show what the task changed since the ticks last had the arenas, before the task is over.
    if (relend) {
        for (auto const& hold : m_clock_leases) {
            if (hold.document->layout_commit_generation() != hold.lend_layout_commit_generation
                || hold.document->style_computer().style_engine().published_transaction_version().transaction != hold.lend_style_transaction) {
                suspend_clock_lend(ClockLendSuspension::Write);
                return;
            }
        }
    }
    // As when the main thread idles, a lease its document no longer plans the same way waits for the rendering update.
    Vector<void*> arenas;
    for (auto& hold : m_clock_leases) {
        auto* arena = hold.document->layout_node_arena_if_created();
        if (!arena)
            continue;
        auto plan = hold.render_clock_context.has_value() && hold.render_clock_kit ? clock_lease_plan(*hold.document) : Optional<ClockLeasePlan> {};
        // A lend again goes on ticking from the samples the documents have not adopted yet.
        bool ticks = plan.has_value() && plan->effects == hold.effects && (relend || publish_clock_lease_targets(hold));
        Layout::RustFFI::rust_clock_lease_set_paused(arena->handle(), !ticks);
        if (!ticks)
            continue;
        hold.lend_layout_commit_generation = hold.document->layout_commit_generation();
        hold.lend_style_transaction = hold.document->style_computer().style_engine().published_transaction_version().transaction;
        arenas.append(arena->handle());
    }
    for (auto* arena : arenas)
        m_clock_lent_this_wake |= Layout::RustFFI::rust_clock_lend_to_busy_main(arena, relend);
    m_clock_lend_taken_back = false;
}

void FrameScheduler::clock_lend_taken_back(void* arena)
{
    m_clock_lend_taken_back = true;
    auto held = m_clock_leases.find_first_index_if([&](auto const& hold) {
        auto const* document_arena = hold.document->layout_node_arena_if_created();
        return document_arena && document_arena->handle() == arena;
    });
    if (!held.has_value())
        return;
    auto document = m_clock_leases[*held].document;
    // The task reads its document at its own time, not at the ticks'.
    auto start_nanoseconds = MonotonicTime::now().nanoseconds();
    auto restore = Layout::RustFFI::rust_clock_lease_restore_host_records(arena);
    if (restore != Layout::RustFFI::FfiClockRestore::Nothing) {
        take_in_clock_layout_frame(*document);
        auto now_nanoseconds = MonotonicTime::now().nanoseconds();
        m_clock_lend_restore_nanoseconds += now_nanoseconds - start_nanoseconds;
        auto allowed_nanoseconds = max(clock_lend_restore_nanoseconds_allowed_anyway, (now_nanoseconds - m_clock_lend_woke_at_nanoseconds) / clock_lend_restore_share_of_wake_divisor);
        if (restore == Layout::RustFFI::FfiClockRestore::NeedsMain || m_clock_lend_restore_nanoseconds > allowed_nanoseconds)
            suspend_clock_lend(ClockLendSuspension::Budget);
    }
    held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == document.ptr(); });
    if (!held.has_value())
        return;
    auto& hold = m_clock_leases[*held];
    hold.lend_layout_commit_generation = document->layout_commit_generation();
    hold.lend_style_transaction = document->style_computer().style_engine().published_transaction_version().transaction;
}

static u32 s_clock_lend_read_depth = 0;

ClockLendReadScope::ClockLendReadScope()
{
    ++s_clock_lend_read_depth;
}

ClockLendReadScope::~ClockLendReadScope()
{
    if (--s_clock_lend_read_depth == 0)
        main_thread_event_loop().frame_scheduler().relend_clock_leases_after_read();
}

bool ClockLendReadScope::is_active()
{
    return s_clock_lend_read_depth > 0;
}

void FrameScheduler::relend_clock_leases_after_read()
{
    // The outermost read lends them again once it is over.
    if (ClockLendReadScope::is_active())
        return;
    if (!m_clock_lend_taken_back || m_clock_lend_suspended || m_event_loop.running_rendering_task())
        return;
    lend_clock_leases_to_busy_main(true);
}

void FrameScheduler::suspend_clock_lend(ClockLendSuspension reason)
{
    if (exchange(m_clock_lend_suspended, true))
        return;
    Layout::RustFFI::rust_clock_lend_note_suspended(reason == ClockLendSuspension::Write ? Layout::RustFFI::FfiClockLendSuspension::Write : Layout::RustFFI::FfiClockLendSuspension::Budget);
}

void FrameScheduler::set_render_clock_suspended(bool suspended)
{
    take_back_clock_lend_for_adoption();
    m_render_clock_suspended = suspended;
    if (suspended) {
        for (auto& hold : m_clock_leases)
            update_render_clock(hold, {});
    }
}

void FrameScheduler::render_clock_needs_main()
{
    // A render clock tick ended a lease: past its deadline, where the main thread has events to send, or with a
    // sample only the main thread takes. The rendering update takes over.
    for (auto& hold : m_clock_leases) {
        if (hold.render_clock_context.has_value())
            hold.document->page().client().request_frame();
    }
}

void FrameScheduler::prepare_clock_ticks(ReadonlySpan<GC::Root<DOM::Document>> docs, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        m_clock_leases[index].ticked = false;
        // Only a rendering update that submits its frame ticks a lease.
        if (m_synchronous_update || !m_ticket) {
            revoke_clock_lease(index);
            continue;
        }
        auto document = m_clock_leases[index].document;
        bool renders = docs.first_matching([&](auto const& doc) { return doc.ptr() == document.ptr(); }).has_value();
        auto* arena = document->layout_node_arena_if_created();
        // Anything the main thread did since the grant that its own rendering update has to see ends the lease: the
        // plan finds it, or finds other effects to tick.
        auto plan = renders && arena && Layout::RustFFI::rust_clock_lease_is_live(arena->handle()) ? clock_lease_plan(*document) : Optional<ClockLeasePlan> {};
        if (!plan.has_value() || plan->effects != m_clock_leases[index].effects) {
            revoke_clock_lease(index);
            continue;
        }
        // NB: The default document timeline's origin time is zero: its time is the document's relative time.
        auto time = max(0.0, HighResolutionTime::relative_high_resolution_time(frame_timestamp, relevant_global_object(*document)));
        if (!(time < plan->deadline)) {
            revoke_clock_lease(index);
            continue;
        }
        // Where a render clock keeps up with the display, the document shows what its last tick presented, and its
        // timeline reads the time of that tick: the rendering update, which runs for something else, moves neither.
        // Ticking the lease here instead would leave the next rendering update something to tick again, and keep the
        // main thread rendering at every display frame. A render clock that falls behind, the rendering update ticks.
        auto& hold = m_clock_leases[index];
        hold.timeline_time_for_update.clear();
        if (hold.render_clock_context.has_value()) {
            auto lease_time = Layout::RustFFI::rust_clock_lease_time(arena->handle());
            auto current = document->timeline()->current_time();
            if (!isnan(lease_time) && current.has_value() && current->type == Animations::TimeValue::Type::Milliseconds
                && lease_time >= current->value && time - lease_time <= render_clock_lag_tolerance_milliseconds)
                hold.timeline_time_for_update = lease_time;
        }
    }
}

Optional<double> FrameScheduler::clock_lease_timeline_time(DOM::Document const& document) const
{
    auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; });
    if (!held.has_value())
        return {};
    return m_clock_leases[*held].timeline_time_for_update;
}

bool FrameScheduler::tick_clock_leases(Vector<GC::Ref<DOM::Document>> const& docs, size_t first_document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp, bool may_submit)
{
    if (may_submit && submit_clock_tick(docs, first_document_index, frame_timestamp))
        return true;
    // A lease this rendering update did not tick ends: the update samples its effects itself.
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        if (!exchange(m_clock_leases[index].ticked, false))
            revoke_clock_lease(index);
    }
    return false;
}

bool FrameScheduler::submit_clock_tick(Vector<GC::Ref<DOM::Document>> const& docs, size_t first_document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    if (m_state != State::MainHalf || !m_ticket || !m_ticket->navigables.is_empty())
        return false;
    for (size_t document_index = first_document_index; document_index < docs.size(); ++document_index) {
        auto document = docs[document_index];
        auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == document.ptr(); });
        auto* arena = document->layout_node_arena_if_created();
        if (!held.has_value() || !arena)
            continue;
        // A render clock that keeps up ticks the lease at the display's ticks, not the rendering update.
        if (m_clock_leases[*held].timeline_time_for_update.has_value()) {
            m_clock_leases[*held].ticked = true;
            continue;
        }
        auto time = document->timeline()->current_time();
        if (!time.has_value() || time->type != Animations::TimeValue::Type::Milliseconds) {
            revoke_clock_lease(*held);
            continue;
        }
        publish_clock_lease_targets(m_clock_leases[*held]);
        if (!Layout::RustFFI::rust_clock_lease_submit_tick(arena->handle(), time->value)) {
            revoke_clock_lease(*held);
            continue;
        }
        m_clock_leases[*held].ticked = true;
        submit_pass(FrameTicket::SubmittedPass::Kind::Clock, docs, document_index, frame_timestamp);
        return true;
    }
    return false;
}

void FrameScheduler::adopt_clock_tick(DOM::Document& document)
{
    auto* arena = document.layout_node_arena_if_created();
    if (!arena)
        return;
    bool installed_any = false;
    // What the render side presented already, adopting repaints nothing of.
    bool const presented_on_render_side = Layout::RustFFI::rust_clock_lease_presented_since_adoption(arena->handle());
    CSS::StyleEffectDrain::install(document, [&](CSS::StyleDrainScope const& scope) {
        u32 style_node = 0;
        u64 style_record_before = 0;
        bool installed_in_arena = false;
        CSS::StyleEngineFFI::FfiRowSampledInPass sample {};
        while (CSS::StyleEngineFFI::style_engine_clock_tick_take_entry(arena->handle(), &style_node, &style_record_before, &installed_in_arena, &sample)) {
            auto element = document.style_computer().element_for_style_node(CSS::StyleNodeID { style_node });
            // An element that left the document, or that the main thread restyled beside the tick, takes nothing.
            if (!element || !element->is_connected() || DOM::AbstractElement { *element }.style_record_identity().value() != style_record_before)
                continue;
            if (!sample.present)
                continue;
            Animations::adopt_clock_tick_sample(scope, DOM::AbstractElement { *element }, CSS::StyleRecordID { style_record_before }, sample, installed_in_arena, presented_on_render_side);
            installed_any = true;
        }
    });
    // What no element adopted leaves the arena's log, with its pins.
    Layout::RustFFI::rust_clock_lease_drop_unadopted(arena->handle());
    if (installed_any)
        Layout::RustFFI::rust_clock_ticks_note_presented();
    // A tick that could not sample every effect, or reached the deadline, ends the lease: the rendering update samples
    // the effects itself. So does a lease that was ended while its tick was in flight.
    if (auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; }); held.has_value()) {
        if (m_clock_leases[*held].revoke_at_adoption || Layout::RustFFI::rust_clock_lease_tick_outcome(arena->handle()) != Layout::RustFFI::FfiClockTickOutcome::Presented)
            revoke_clock_lease(*held);
    }
}

void FrameScheduler::visit_edges(JS::Cell::Visitor& visitor)
{
    visitor.visit(m_deferred_arena_changes);
    for (auto& hold : m_clock_leases) {
        visitor.visit(hold.document);
        visitor.visit(hold.effects);
    }
    visitor.visit(m_documents_holding_style_records);
    if (!m_ticket)
        return;
    for (auto& submitted : m_ticket->navigables) {
        visitor.visit(submitted.navigable);
        visitor.visit(submitted.frame.document);
        if (submitted.frame.recording)
            visitor.visit(submitted.frame.recording->document);
    }
    visitor.visit(m_ticket->painted_local_roots);
    if (m_ticket->submitted_pass.has_value())
        visitor.visit(m_ticket->submitted_pass->documents);
}

}
