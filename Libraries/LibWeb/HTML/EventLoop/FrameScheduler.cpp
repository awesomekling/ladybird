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
#include <LibWeb/Animations/ScrollTimeline.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEffectDrain.h>
#include <LibWeb/Compositor/CompositorHost.h>
#include <LibWeb/Compositor/NavigablePresenter.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameCompletion.h>
#include <LibWeb/HTML/EventLoop/FrameInFlightReferences.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/HTML/EventLoop/MainThreadPhases.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/Scripting/Environments.h>
#include <LibWeb/HTML/Scripting/TemporaryExecutionContext.h>
#include <LibWeb/HTML/Window.h>
#include <LibWeb/HighResolutionTime/TimeOrigin.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Namespace.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxSlot.h>
#include <LibWeb/Painting/PendingDisplayListRecording.h>
#include <LibWeb/Painting/QueryView.h>
#include <LibWeb/StyleEngineRustFFI.h>

namespace Web::HTML {

static FrameScheduler* s_frame_scheduler_with_host = nullptr;

// LIBWEB_RENDER_CLOCK_FRAMES: A render clock ticks the leases beside the main thread, and tells it where a tick ended
// one, or left it something to adopt. The stage thread reaches the main thread through this.
struct RenderClockNeedsMain {
    Mutex mutex;
    RefPtr<Core::WeakEventLoopReference> event_loop;
    bool queued { false };
    bool adoption_queued { false };
    size_t injected_ticks_run { 0 };
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

// The kits the render clock's ticks present with, by the layout arena of their document. A tick presents from a kit, and
// the main thread changes or adopts one, with this held.
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
    Layout::RustFFI::rust_render_clock_set_present([](void* arena) {
        MutexLocker locker(render_clock_kits_mutex());
        auto* kit = render_clock_kits().get(arena).value_or(nullptr);
        if (!kit)
            return Layout::RustFFI::FfiClockPresent::Declined;
        // The main thread took the presenter back for frames of its own, which show what the tick laid out.
        if (!kit->presentation->presenter->present_for_render_clock([&] { LocalNavigable::present_render_clock_frame(*kit); }))
            return Layout::RustFFI::FfiClockPresent::MainPresents;
        return Layout::RustFFI::FfiClockPresent::Presented;
    });
    {
        auto& needs_main = render_clock_needs_main();
        MutexLocker locker(needs_main.mutex);
        needs_main.event_loop = Core::EventLoop::current_weak();
    }
    Layout::RustFFI::rust_render_clock_set_adopt_on_main([](bool injected_tick_ran) {
        auto& needs_main = render_clock_needs_main();
        MutexLocker locker(needs_main.mutex);
        if (injected_tick_ran)
            ++needs_main.injected_ticks_run;
        if (needs_main.adoption_queued || !needs_main.event_loop)
            return;
        auto event_loop = needs_main.event_loop->take();
        if (!event_loop.is_alive())
            return;
        needs_main.adoption_queued = true;
        event_loop->deferred_invoke([] {
            size_t injected_ticks_run = 0;
            {
                auto& needs_main = render_clock_needs_main();
                MutexLocker locker(needs_main.mutex);
                needs_main.adoption_queued = false;
                injected_ticks_run = exchange(needs_main.injected_ticks_run, 0);
            }
            if (s_frame_scheduler_with_host)
                s_frame_scheduler_with_host->render_clock_asks_to_adopt(injected_ticks_run);
        });
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
    // A rendering update that has to start now (a synchronous one, or one a rendering opportunity started while the
    // tail was still pending) finishes the previous frame first.
    if (m_state != State::Idle)
        finish_frame_now();
    VERIFY(m_state == State::Idle);
    VERIFY(!m_ticket);
    m_synchronous_update = synchronous;
    m_rendering_update_waits_for_recordings = false;
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
    if (m_state == State::MainHalf && m_ticket && !m_rendering_update_waits_for_recordings)
        return Painting::RecordingRun::InSubmittedFrame;
    return Painting::RecordingRun::Now;
}

Painting::RecordingOrigin FrameScheduler::recording_origin() const
{
    if (recording_run() == Painting::RecordingRun::InSubmittedFrame)
        return Painting::RecordingOrigin::RenderingUpdate;
    if (m_synchronous_update)
        return Painting::RecordingOrigin::SynchronousRenderingUpdate;
    if (m_ticket && m_rendering_update_waits_for_recordings)
        return Painting::RecordingOrigin::WaitsForRecordings;
    return Painting::RecordingOrigin::NoFrameScheduler;
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
    auto const* recording = frame.recording.ptr();
    if (m_takes_in_next_recording_before_its_presentation && recording && recording->run == Painting::RecordingRun::InSubmittedFrame && recording->submitted_ticket.is_in_flight()) {
        m_takes_in_next_recording_before_its_presentation = false;
        ++m_recordings_taken_in_before_their_presentation;
        Layout::RustFFI::layout_arena_take_in_recording(recording->arena);
    }
    m_ticket->navigables.append({ navigable, move(frame), held_compositor_context });
    if (!earlier_frame_is_presented_by_commit)
        navigable.submit_presentation(m_ticket->navigables.last().frame);
}

bool FrameScheduler::submit()
{
    VERIFY(m_state == State::MainHalf);
    if (!m_ticket || m_ticket->navigables.is_empty()) {
        // Nothing went to the render side, or a forced join during the main half took all of it in already. Then the
        // rendering update goes on as one that waited for its paint.
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

void FrameScheduler::submit_document_pass(Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    // The ticket runs the pass the frame in flight holds, whichever the document chose to submit.
    auto kind = FrameTicket::SubmittedPass::Kind::Layout;
    switch (Layout::RustFFI::rust_stage_thread_submitted_document_pass()) {
    case Layout::RustFFI::FfiSubmittedDocumentPass::Style:
        kind = FrameTicket::SubmittedPass::Kind::Style;
        break;
    case Layout::RustFFI::FfiSubmittedDocumentPass::Layout:
        kind = FrameTicket::SubmittedPass::Kind::Layout;
        break;
    case Layout::RustFFI::FfiSubmittedDocumentPass::Flight:
        kind = FrameTicket::SubmittedPass::Kind::Flight;
        break;
    case Layout::RustFFI::FfiSubmittedDocumentPass::None:
        // The document submitted a pass, which the frame in flight holds.
        ASSERT(false);
        break;
    }
    submit_pass(kind, move(documents), document_index, frame_timestamp);
}

void FrameScheduler::submit_pass(FrameTicket::SubmittedPass::Kind kind, Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    VERIFY(m_state == State::MainHalf);
    // Only a main half with a ticket submits, and the pass comes before any recording.
    VERIFY(m_ticket);
    VERIFY(m_ticket->navigables.is_empty());
    // The document sealed what its flight records as it submitted the flight.
    GC::Ptr<LocalNavigable> sealed_flight_paint;
    if (auto navigable = documents[document_index]->navigable(); kind == FrameTicket::SubmittedPass::Kind::Flight && navigable && navigable->has_sealed_flight_paint())
        sealed_flight_paint = navigable;
    m_ticket->submitted_pass = FrameTicket::SubmittedPass { kind, move(documents), document_index, frame_timestamp, {}, sealed_flight_paint };
    m_state = State::InFlight;
    m_event_loop.did_submit_frame();
}

bool FrameScheduler::pass_in_flight_records() const
{
    if (!awaits_pass() || m_ticket->submitted_pass->kind != FrameTicket::SubmittedPass::Kind::Flight)
        return false;
    if (!m_ticket->submitted_pass->sealed_flight_paint)
        return false;
    // A flight that runs the style of its layout records only if it applies that style itself; otherwise the rendering
    // update lays out after it, and records then.
    return !CSS::style_update_submitted_in_layout_flight() || Layout::RustFFI::rust_flight_applies_its_style();
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
        // A flight that ran its layout's style had the style installed as its layout frame was taken back.
        if (outcome.began == Layout::RustFFI::FfiFlightStage::Style && document->has_submitted_style_update())
            document->finish_submitted_style_update();
        // A flight that went on from the layout pass to record the document hands its frame off here, as the frame of a
        // recording in flight is.
        if (auto navigable = exchange(m_ticket->submitted_pass->sealed_flight_paint, nullptr)) {
            using FlightPaintEnd = LocalNavigable::FlightPaintEnd;
            auto end = FlightPaintEnd::NotRecorded;
            if (outcome.reached == Layout::RustFFI::FfiFlightStage::Present)
                end = outcome.end == Layout::RustFFI::FfiFlightEndReason::HostLeftWork ? FlightPaintEnd::PresentedAheadOfMoreWork : FlightPaintEnd::Presented;
            else if (outcome.reached >= Layout::RustFFI::FfiFlightStage::Record)
                end = FlightPaintEnd::Recorded;
            else if (outcome.end == Layout::RustFFI::FfiFlightEndReason::HostLeftWork)
                end = FlightPaintEnd::RecordedAheadOfMoreWork;
            auto const finished = navigable->finish_flight_paint(*document, end);
            if (finished.handed_off) {
                m_event_loop.note_frame_painted({});
                m_ticket->painted_local_roots.append(*navigable);
            }
            if (finished.paints_again_after_recording)
                m_rendering_update_waits_for_recordings = true;
        }
    }
    // A clock tick's document adopts what the tick installed before anything reads it, and then takes in what was
    // marked beside the tick, as the end of a layout pass's frame does: the next drain writes it.
    if (m_ticket->submitted_pass.has_value() && m_ticket->submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Clock) {
        auto& document = m_ticket->submitted_pass->documents[m_ticket->submitted_pass->document_index];
        (void)adopt_clock_tick(document);
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
        MainThreadPhases::Scope phase { MainThreadPhases::Phase::FlightJoin };
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
    // NB: A flight that recorded this rendering update's frame, and whose recording did not stand or whose presented
    //     frame the document paints again, had consume-commit have the rest of the rendering update wait for its
    //     recordings. Otherwise consume-commit handed off the frame the flight recorded, and the rest of the rendering
    //     update runs as after the layout pass, and paints again only what was marked beside the flight.
    case FfiFlightStage::Rounds:
    case FfiFlightStage::PaintPrep:
    case FfiFlightStage::Record:
    case FfiFlightStage::Present:
        m_event_loop.resume_rendering_update_after_layout({}, flight.documents, flight.document_index, flight.frame_timestamp);
        return;
    }
    VERIFY_NOT_REACHED();
}

bool FrameScheduler::arena_changes_wait_for_frame(DOM::Document const& document)
{
    auto* arena = document.layout_arena_handle();
    return arena && Layout::RustFFI::rust_stage_thread_arena_changes_wait_for_frame(arena);
}

void FrameScheduler::defer_arena_change(GC::Ref<GC::Function<void()>> change)
{
    m_deferred_arena_changes.append(change);
}

void FrameScheduler::change_arena(DOM::Document& document, Function<void(void* arena)> change)
{
    auto* arena = document.layout_arena_handle();
    if (!arena)
        return;
    if (!arena_changes_wait_for_frame(document)) {
        change(arena);
        return;
    }
    main_thread_event_loop().frame_scheduler().defer_arena_change(GC::create_function(document.heap(), [document = GC::Ref { document }, change = move(change)] {
        if (auto* arena = document->layout_arena_handle())
            change(arena);
    }));
}

void FrameScheduler::apply_deferred_arena_changes()
{
    // A change applied here can defer no other: nothing is in flight anymore.
    auto changes = move(m_deferred_arena_changes);
    for (auto const& change : changes)
        change->function()();
}

// Takes in what a clock lease's ticks laid out on the render side beside the main thread, as a layout update of its
// own. Returns whether they had laid anything out.
static bool take_in_clock_layout_frame(DOM::Document& document)
{
    auto* arena = document.layout_arena_handle();
    if (!arena || !Layout::RustFFI::layout_arena_clock_layout_frame_laid_out(arena))
        return false;
    // The layout update begins once the document's frame in flight is back: unloading may take the frame in beside
    // one.
    document.join_frame_in_flight();
    // What the ticks laid out moved boxes that no input of the main thread's did.
    (void)document.render_inputs_for_write();
    // The ticks' rounds ran as the rest of a layout update, which begins and ends here, around their frame: its end is
    // taken in as a submitted pass's frame is taken back (Document::take_in_layout_frame_effects).
    Layout::RustFFI::layout_arena_begin_update_layout(arena);
    document.begin_style_stabilization_epoch();
    document.style_computer().begin_style_record_view_epoch();
    if (!Layout::RustFFI::layout_arena_take_in_clock_layout_frame(arena))
        return false;
    document.renew_clock_layout_frame();
    return true;
}

// A scroll progress timeline whose effects a clock lease ticks: a tick samples it at the scroll offset the compositor
// scrolled its scroller to.
struct ClockLeaseScrollTimeline {
    GC::Ref<Animations::ScrollTimeline> timeline;
    // Its progress (percent) at the scroll offset the main thread holds now.
    double progress { 0 };
    Layout::RustFFI::FfiClockScrollTimeline lease {};
};

// What a clock lease of a document ticks, and until when.
struct ClockLeasePlan {
    Vector<GC::Ref<Animations::KeyframeEffect>> effects;
    Vector<ClockLeaseScrollTimeline> scroll_timelines;
    // The timeline time at which the main thread has events of an effect to send (a phase change, or a new iteration
    // someone listens for): no tick samples at or past it. The events of a new iteration nobody hears wait for the
    // next rendering update, which sends them as ever.
    double deadline { AK::Infinity<double> };
};

// The local times from which, and up to which, the phase and the current iteration of `effect` stay what they are at
// `local_time`, on a progress-based timeline.
struct EffectInterval {
    double start;
    double end;
};

static Optional<EffectInterval> phase_and_iteration_interval_in_local_time(Animations::KeyframeEffect const& effect, Animations::TimeValue local_time)
{
    if (effect.start_delay().type != local_time.type || effect.iteration_duration().type != local_time.type || effect.active_duration().type != local_time.type)
        return {};
    auto start_delay = effect.start_delay().value;
    auto iteration_duration = effect.iteration_duration().value;
    auto active_end = start_delay + effect.active_duration().value;
    if (local_time.value < start_delay)
        return EffectInterval { -AK::Infinity<double>, start_delay };
    if (!(iteration_duration > 0) || local_time.value >= active_end)
        return {};
    auto iteration_start = start_delay + floor((local_time.value - start_delay) / iteration_duration) * iteration_duration;
    return EffectInterval { iteration_start, min(iteration_start + iteration_duration, active_end) };
}

// The scroll progress timeline `timeline` as a lease ticks it, where the compositor scrolls its scroller: the document
// viewport or an element's box.
static Optional<ClockLeaseScrollTimeline> clock_lease_scroll_timeline(DOM::Document& document, Animations::ScrollTimeline& timeline)
{
    auto inputs = timeline.scroll_progress_inputs();
    if (!inputs.has_value())
        return {};
    ClockLeaseScrollTimeline scroll_timeline { timeline };
    scroll_timeline.progress = inputs->scroll_offset / inputs->max_scroll_offset * 100;
    scroll_timeline.lease.identity = timeline.style_engine_identity();
    if (inputs->scroller) {
        if (&inputs->scroller->document() != &document || !Painting::BoxSlot::bound_to(*inputs->scroller))
            return {};
        scroll_timeline.lease.node_id = inputs->scroller->unique_id().value();
        scroll_timeline.lease.kind = to_underlying(Compositing::AsyncScrollNodeKind::Element);
    } else {
        scroll_timeline.lease.node_id = document.unique_id().value();
        scroll_timeline.lease.kind = to_underlying(Compositing::AsyncScrollNodeKind::Viewport);
    }
    scroll_timeline.lease.pseudo_element_type = 0;
    scroll_timeline.lease.vertical = inputs->is_vertical;
    scroll_timeline.lease.max_scroll_offset = inputs->max_scroll_offset;
    scroll_timeline.lease.progress_start = -AK::Infinity<double>;
    scroll_timeline.lease.progress_end = AK::Infinity<double>;
    scroll_timeline.lease.progress = scroll_timeline.progress;
    return scroll_timeline;
}

// The next time, in the local time of `effect`, at which the main thread has events of it to send: where its phase
// changes, or where its next iteration starts if a listener hears its iteration events.
static Optional<double> next_event_in_local_time(Animations::KeyframeEffect const& effect, double local_time)
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
    if (!effect.css_animation_iteration_events_are_heard())
        return active_end;
    auto next_iteration_start = start_delay + (floor((local_time - start_delay) / iteration_duration) + 1) * iteration_duration;
    return min(next_iteration_start, active_end);
}

// Whether a clock lease can tick the running animations of `document`, and which of their effects it ticks. The lease
// ticks only what the main thread would do nothing else for until the deadline: the running, non-pending animations
// of the document timeline that animate no property that can change the layout tree's shape or what the main thread
// observes of it, on elements that have a box.
// Whether the effect moves or resizes boxes, which an intersection observer may see.
static bool moves_boxes(Animations::KeyframeEffect const& effect)
{
    auto const* key_frame_set = effect.key_frame_set();
    if (!key_frame_set)
        return true;
    for (auto const& keyframe : key_frame_set->keyframes_by_key) {
        for (auto const& [property, value] : keyframe.properties) {
            if (CSS::property_affects_layout(property.id()) || first_is_one_of(property.id(), CSS::PropertyID::Translate, CSS::PropertyID::Rotate, CSS::PropertyID::Scale, CSS::PropertyID::Transform))
                return true;
        }
    }
    return false;
}

static Optional<ClockLeasePlan> clock_lease_plan(DOM::Document& document)
{
    if (!Layout::RustFFI::rust_stage_thread_submits_clock())
        return {};
    if (!document.is_fully_active() || document.hidden() || document.is_decoded_svg())
        return {};
    auto navigable = document.navigable();
    if (!navigable || navigable->active_document().ptr() != &document || !document.layout_arena_handle())
        return {};
    // NB: A tick lays out on the render side, as a layout pass beside the main thread does. What keeps the layout of a
    //     scroll-driven animation's document in place is that a layout that changes a scroller's scroll range makes
    //     its timelines stale, which the main thread's rendering update takes care of after its layout. A tick keeps
    //     the range the main thread last laid out. What a tick laid out is up to date as well, while it waits for the
    //     main thread to take it in.
    auto* arena = document.layout_arena_handle();
    if (document.needs_animated_style_update() || !(document.layout_is_up_to_date() || Layout::RustFFI::layout_arena_clock_layout_frame_laid_out(arena)))
        return {};
    if (auto blocker = document.layout_overlap_blocker(); blocker.has_value() && *blocker != DOM::LayoutOverlapBlocker::ScrollTimeline)
        return {};
    // NB: A document with animation frame callbacks keeps its lease: while the main thread renders it at every display
    //     frame, its frames show what the render clock ticks, and the render clock presents the frames it misses.
    if (!document.window())
        return {};
    auto timeline = document.timeline();
    auto timeline_time = timeline->current_time();
    if (!timeline_time.has_value() || timeline_time->type != Animations::TimeValue::Type::Milliseconds)
        return {};

    // The render clock's ticks run no rendering update, which is where intersection observers see what moved.
    bool const intersections_are_observed = document.has_intersection_observations();
    ClockLeasePlan plan;
    for (auto const& associated_timeline : document.associated_animation_timelines()) {
        auto* scroll_timeline = as_if<Animations::ScrollTimeline>(*associated_timeline);
        Optional<size_t> scroll_timeline_index;
        for (auto& animation : associated_timeline->associated_animations()) {
            if (animation.play_state() != Bindings::AnimationPlayState::Running)
                continue;
            if (animation.pending() || !(animation.playback_rate() > 0))
                return {};
            if (associated_timeline.ptr() != timeline.ptr()) {
                if (!scroll_timeline)
                    return {};
                if (!scroll_timeline_index.has_value()) {
                    auto leased = clock_lease_scroll_timeline(document, *scroll_timeline);
                    if (!leased.has_value())
                        return {};
                    scroll_timeline_index = plan.scroll_timelines.size();
                    plan.scroll_timelines.append(leased.release_value());
                }
            }
            auto effect = animation.effect();
            if (!effect || !is<Animations::KeyframeEffect>(*effect))
                return {};
            auto& keyframe_effect = static_cast<Animations::KeyframeEffect&>(*effect);
            auto target = keyframe_effect.target();
            if (!target || &target->document() != &document || !target->is_connected())
                return {};
            // What the compositor or the offscreen throttle runs, the main thread does not sample per frame either:
            // the lease only has to stop at its events.
            bool const ticks_effect = !keyframe_effect.is_compositor_driven() && !keyframe_effect.is_compositor_replaced() && !keyframe_effect.can_skip_per_frame_style_update();
            if (ticks_effect && (keyframe_effect.pseudo_element_type().has_value() || target->namespace_uri() != Namespace::HTML || !Painting::BoxSlot::bound_to(*target)))
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
            if (scroll_timeline_index.has_value()) {
                // A tick past the progress at which the effect changes its phase or its iteration needs the main thread,
                // which has events to send.
                auto timeline_progress = scroll_timeline->current_time();
                if (!local_time.has_value() || local_time->type != Animations::TimeValue::Type::Percentage || !timeline_progress.has_value() || timeline_progress->type != Animations::TimeValue::Type::Percentage)
                    return {};
                auto interval = phase_and_iteration_interval_in_local_time(keyframe_effect, *local_time);
                if (!interval.has_value())
                    return {};
                auto& lease = plan.scroll_timelines[*scroll_timeline_index].lease;
                lease.progress_start = max(lease.progress_start, timeline_progress->value - (local_time->value - interval->start) / animation.playback_rate());
                lease.progress_end = min(lease.progress_end, timeline_progress->value + (interval->end - local_time->value) / animation.playback_rate());
                if (!ticks_effect)
                    continue;
                if (intersections_are_observed && moves_boxes(keyframe_effect))
                    return {};
                plan.effects.append(keyframe_effect);
                continue;
            }
            if (!local_time.has_value() || local_time->type != Animations::TimeValue::Type::Milliseconds)
                return {};
            auto next_event = next_event_in_local_time(keyframe_effect, local_time->value);
            if (!next_event.has_value())
                return {};
            plan.deadline = min(plan.deadline, timeline_time->value + (*next_event - local_time->value) / animation.playback_rate());
            if (!ticks_effect)
                continue;
            if (intersections_are_observed && moves_boxes(keyframe_effect))
                return {};
            plan.effects.append(keyframe_effect);
        }
    }
    if (plan.effects.is_empty() || !(plan.deadline > timeline_time->value))
        return {};
    // Animation frame callbacks read scroll-driven effects where a script that scrolls in them left them, which the
    // render clock only learns of at the end of the rendering update: a document with both samples them itself.
    if (!plan.scroll_timelines.is_empty() && document.window()->has_animation_frame_callbacks())
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
        if (auto* arena = hold.document->layout_arena_handle())
            Layout::RustFFI::rust_document_clock_set_paused(arena, true);
        return;
    }
    auto hold = m_clock_leases.take(index);
    // What the render clock's ticks laid out and presented goes in before the lease that holds it ends.
    take_in_clock_layout_frame(*hold.document);
    replace_render_clock_kit(hold, {});
    if (auto* arena = hold.document->layout_arena_handle())
        Layout::RustFFI::rust_document_clock_stop(arena);
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
    // What the render clock's ticks left since the rendering update began goes in before a lease starts its clock anew.
    adopt_render_clock_ticks_if_any();
    auto documents = m_synchronous_update ? Vector<GC::Root<DOM::Document>> {} : m_event_loop.documents_in_this_event_loop_matching([](auto&) { return true; });
    for (auto& hold : m_clock_leases)
        hold.timeline_time_for_update.clear();
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        if (!documents.first_matching([&](auto const& document) { return document.ptr() == m_clock_leases[index].document.ptr(); }).has_value())
            revoke_clock_lease(index);
    }
    for (auto& document : documents) {
        auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == document.ptr(); });
        // A lease whose document this rendering update did not render goes on as it was granted.
        if (held.has_value() && !exchange(m_clock_leases[*held].renders_in_update, true))
            continue;
        auto plan = clock_lease_plan(*document);
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
            // The render clock samples the lease's animations and lays the document out beside the main thread from
            // here on, so its geometry moves with no write of the main thread's to show for it.
            (void)document->render_inputs_for_write();
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
        Layout::RustFFI::rust_document_clock_start(document->layout_arena_handle(), context_id, timeline->style_engine_identity(), timeline_zero, timeline_time, plan->deadline);
        Vector<Layout::RustFFI::FfiClockScrollTimeline> scroll_timelines;
        for (auto const& scroll_timeline : plan->scroll_timelines)
            scroll_timelines.append(scroll_timeline.lease);
        Layout::RustFFI::rust_document_clock_set_scroll_timelines(document->layout_arena_handle(), scroll_timelines.data(), scroll_timelines.size());
        auto& hold = *m_clock_leases.find_if([&](auto const& hold) { return hold.document.ptr() == document.ptr(); });
        hold.timeline_time_of_last_update = timeline_time;
        publish_clock_lease_targets(hold);
        update_render_clock(hold, context_id ? Optional<Compositing::CompositorContextId> { context_id } : OptionalNone {});
        // The render clock's ticks lay out what their samples leave in a frame of their own, and present it as the frame
        // the navigable just painted was presented.
        OwnPtr<LocalNavigable::RenderClockFrameKit> kit;
        if (hold.render_clock_context.has_value()) {
            document->renew_clock_layout_frame();
            if (auto sealed = document->navigable()->seal_render_clock_frame_kit(); sealed.has_value())
                kit = make<LocalNavigable::RenderClockFrameKit>(sealed.release_value());
        }
        replace_render_clock_kit(hold, move(kit));
    }
    // Until a document adopts what a tick moved, its reads answer from what the rendering update laid out, and no tick
    // that runs meanwhile shows in them.
    for (auto& hold : m_clock_leases)
        hold.document->publish_query_snapshot_after_read(Painting::QueryVisualContexts::UpToDate);
}

// Takes in what the ticks presented from the lease's kit, and has them present from `kit` from now on.
void FrameScheduler::replace_render_clock_kit(ClockLeaseHold& hold, OwnPtr<LocalNavigable::RenderClockFrameKit> kit)
{
    auto* arena = hold.document->layout_arena_handle();
    MutexLocker locker(render_clock_kits_mutex());
    if (hold.render_clock_kit) {
        if (auto navigable = hold.document->navigable())
            navigable->adopt_render_clock_frame_kit(*hold.render_clock_kit);
    }
    if (arena) {
        if (kit)
            render_clock_kits().set(arena, kit.ptr());
        else
            render_clock_kits().remove(arena);
    }
    // The ticks present from the navigable's presenter until the main thread next needs it.
    if (kit)
        kit->presentation->presenter->lend_to_render_clock();
    else if (hold.render_clock_kit)
        (void)hold.render_clock_kit->presentation->presenter->take_back_from_render_clock();
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

// Tells the lease which elements it ticks: a tick samples each element over the record its box holds then.
void FrameScheduler::publish_clock_lease_targets(ClockLeaseHold const& hold)
{
    auto* arena = hold.document->layout_arena_handle();
    if (!arena)
        return;
    Vector<u32> style_nodes;
    Vector<bool> pseudo_element_styles_outside_box;
    for (auto effect : hold.effects) {
        auto target = effect->target();
        if (!target || style_nodes.contains_slow(target->style_node_id().value()))
            continue;
        style_nodes.append(target->style_node_id().value());
        // A tick derives no style of these from the element's, as it does for the text in its box.
        pseudo_element_styles_outside_box.append(target->has_style(CSS::PseudoElement::Backdrop) || target->has_style(CSS::PseudoElement::Selection));
    }
    Layout::RustFFI::rust_document_clock_set_targets(arena, style_nodes.data(), pseudo_element_styles_outside_box.data(), style_nodes.size());
}

bool FrameScheduler::render_clock_ticks(DOM::Document const& document) const
{
    auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; });
    return held.has_value() && m_clock_leases[*held].render_clock_context.has_value();
}

bool FrameScheduler::holds_clock_lease(DOM::Document const& document) const
{
    return m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; }).has_value();
}

void FrameScheduler::revoke_all_clock_leases()
{
    // A lease whose tick is in flight stays until the tick is in, and ending one can end others with it.
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        if (index < m_clock_leases.size())
            revoke_clock_lease(index);
    }
}

void FrameScheduler::revoke_clock_lease_of(DOM::Document const& document)
{
    if (auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; }); held.has_value())
        revoke_clock_lease(*held);
}

void FrameScheduler::inject_render_clock_tick(double frame_time, Function<void(bool)> on_end)
{
    // Without a render clock host, no tick reaches the main thread, and without a lease a render clock would tick, no
    // lease takes it.
    bool const render_clock_ticks_a_lease = any_of(m_clock_leases, [](auto const& hold) { return hold.render_clock_context.has_value(); });
    if (!Layout::RustFFI::rust_stage_thread_submits_clock() || s_frame_scheduler_with_host != this || !render_clock_ticks_a_lease) {
        Core::deferred_invoke([on_end = move(on_end)] { on_end(false); });
        return;
    }
    if (!m_injected_clock_tick_sender)
        m_injected_clock_tick_sender = Layout::RustFFI::rust_render_clock_sender_create();
    bool injected = false;
    auto frame_time_nanoseconds = static_cast<i64>(frame_time * 1'000'000.0);
    for (auto& hold : m_clock_leases) {
        if (m_injected_clock_tick_sender && hold.render_clock_context.has_value())
            injected |= Layout::RustFFI::rust_render_clock_inject_tick(m_injected_clock_tick_sender, hold.render_clock_context->value(), frame_time_nanoseconds);
    }
    if (!injected) {
        Core::deferred_invoke([on_end = move(on_end)] { on_end(false); });
        return;
    }
    m_injected_clock_ticks_in_flight.append(move(on_end));
}

void FrameScheduler::adopt_render_clock_ticks_if_any()
{
    if (Layout::RustFFI::rust_render_clock_take_ticks_to_adopt())
        adopt_render_clock_ticks();
}

void FrameScheduler::render_clock_asks_to_adopt(size_t injected_ticks_run)
{
    adopt_render_clock_ticks_if_any();
    // What waits for the ticks a test injected goes on once the documents adopted them.
    for (size_t i = 0; i < injected_ticks_run && !m_injected_clock_ticks_in_flight.is_empty(); ++i)
        m_injected_clock_ticks_in_flight.take_first()(true);
}

void FrameScheduler::adopt_render_clock_ticks()
{
    // What the render clock's ticks installed ahead of the main thread, each document adopts before anything else
    // reaches it, and one whose lease a render clock ticks shows the time of the last tick. A tick that ran before the
    // owner learned that a lease ended left samples too, which its document adopts all the same.
    auto documents = m_event_loop.documents_in_this_event_loop_matching([](auto& document) {
        auto* arena = document.layout_arena_handle();
        return arena && Layout::RustFFI::rust_document_clock_left_adoption(arena);
    });
    auto lease_of = [&](DOM::Document const& document) -> ClockLeaseHold* {
        auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; });
        return held.has_value() && m_clock_leases[*held].render_clock_context.has_value() ? &m_clock_leases[*held] : nullptr;
    };
    Vector<GC::Root<DOM::Document>> documents_to_move;
    Vector<double> times_to_move_to;
    for (auto& document : documents) {
        bool const render_clock_ticks = lease_of(*document);
        auto adopted = adopt_clock_tick(*document);
        if (!render_clock_ticks)
            continue;
        // The style the ticks installed is the document's now, and so is what they laid out with it, and what they
        // presented.
        take_in_clock_layout_frame(*document);
        if (auto* hold = lease_of(*document); hold && hold->render_clock_kit) {
            MutexLocker locker(render_clock_kits_mutex());
            if (auto navigable = document->navigable())
                navigable->adopt_render_clock_frame_kit(*hold->render_clock_kit);
        }
        // Reads answer from what the tick laid out, as the document shows it now.
        if (adopted.query_snapshot)
            document->adopt_render_clock_query_snapshot(adopted.query_snapshot.release_nonnull());
        if (!isnan(adopted.time)) {
            if (auto current = document->timeline()->current_time(); current.has_value() && current->type == Animations::TimeValue::Type::Milliseconds && current->value < adopted.time) {
                documents_to_move.append(document);
                times_to_move_to.append(adopted.time);
            }
        }
    }
    // Moving a timeline runs the pending tasks of its animations, which resolve their promises, as a rendering update
    // does in an execution context of the document's. The microtasks that follow run once every document adopted.
    for (size_t index = 0; index < documents_to_move.size(); ++index) {
        HTML::TemporaryExecutionContext execution_context { documents_to_move[index]->relevant_settings_object() };
        documents_to_move[index]->timeline()->update_current_time(times_to_move_to[index]);
    }
}

void FrameScheduler::adopt_render_clock_frames_of(LocalNavigable& navigable)
{
    auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document->navigable().ptr() == &navigable; });
    if (!held.has_value() || !m_clock_leases[*held].render_clock_kit)
        return;
    MutexLocker locker(render_clock_kits_mutex());
    navigable.adopt_render_clock_frame_kit(*m_clock_leases[*held].render_clock_kit);
}

void FrameScheduler::set_render_clock_suspended(bool suspended)
{
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
    adopt_render_clock_ticks_if_any();
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        m_clock_leases[index].ticked = false;
        // Only a rendering update that submits its frame ticks a lease.
        if (m_synchronous_update || !m_ticket) {
            revoke_clock_lease(index);
            continue;
        }
        auto document = m_clock_leases[index].document;
        bool renders = docs.first_matching([&](auto const& doc) { return doc.ptr() == document.ptr(); }).has_value();
        // A document the rendering update leaves out only because its navigable has no rendering opportunity now
        // renders at its own, as another top-level traversable's does: its lease goes on until then.
        auto navigable = document->navigable();
        m_clock_leases[index].renders_in_update = renders || !document->is_fully_active() || document->hidden() || document->is_render_blocked() || !navigable || navigable->has_a_rendering_opportunity();
        if (!m_clock_leases[index].renders_in_update)
            continue;
        auto* arena = document->layout_arena_handle();
        // Anything the main thread did since the grant that its own rendering update has to see ends the lease: the
        // plan finds it, or finds other effects to tick.
        auto plan = renders && arena && Layout::RustFFI::rust_document_clock_is_running(arena) ? clock_lease_plan(*document) : Optional<ClockLeasePlan> {};
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
        auto window = document->window();
        bool const has_frame_callbacks = window && window->has_animation_frame_callbacks();
        // The rendering update ticks a lease whose last tick sampled a scroll progress timeline elsewhere than where the
        // main thread has scrolled to since: the render clock follows the compositor's scrolling only beside a task.
        bool const ticks_followed_scrolling = all_of(plan->scroll_timelines, [&](auto const& scroll_timeline) {
            auto progress = Layout::RustFFI::rust_document_clock_scroll_progress(arena, scroll_timeline.lease.identity);
            // NB: Scroll offsets are in 1/64 CSS pixels.
            return fabs(progress - scroll_timeline.progress) * scroll_timeline.lease.max_scroll_offset / 100 < 1.0 / 64;
        });
        if (hold.render_clock_context.has_value() && ticks_followed_scrolling) {
            auto lease_time = Layout::RustFFI::rust_document_clock_time(arena);
            auto current = document->timeline()->current_time();
            // Frame callbacks read the timeline at every frame, which a render clock that did not tick since the last
            // rendering update does not keep up with.
            bool const ticked_since_last_update = !has_frame_callbacks || lease_time > hold.timeline_time_of_last_update;
            if (!isnan(lease_time) && current.has_value() && current->type == Animations::TimeValue::Type::Milliseconds
                && lease_time >= current->value && time - lease_time <= render_clock_lag_tolerance_milliseconds && ticked_since_last_update)
                hold.timeline_time_for_update = lease_time;
        }
        hold.timeline_time_of_last_update = hold.timeline_time_for_update.value_or(time);
        // Animation frame callbacks read the effects as the rendering update samples them, before the update would
        // tick the lease: where the render clock has not kept up, the update samples them itself.
        if (!hold.timeline_time_for_update.has_value() && has_frame_callbacks)
            revoke_clock_lease(index);
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
    // The render clock may have ticked since the rendering update began (it goes on beside the tasks that run between
    // the update's passes): a lease ticks here over the records its elements hold once they adopted that.
    adopt_render_clock_ticks_if_any();
    // A lease the render clock keeps up with, it ticks at the display's ticks, not the rendering update.
    for (auto& hold : m_clock_leases) {
        if (hold.timeline_time_for_update.has_value())
            hold.ticked = true;
    }
    if (may_submit && submit_clock_tick(docs, first_document_index, frame_timestamp))
        return true;
    // A lease this rendering update did not tick ends: the update samples its effects itself.
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        if (!exchange(m_clock_leases[index].ticked, false) && m_clock_leases[index].renders_in_update)
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
        auto* arena = document->layout_arena_handle();
        if (!held.has_value() || !arena)
            continue;
        if (m_clock_leases[*held].timeline_time_for_update.has_value())
            continue;
        auto time = document->timeline()->current_time();
        if (!time.has_value() || time->type != Animations::TimeValue::Type::Milliseconds) {
            revoke_clock_lease(*held);
            continue;
        }
        // Beside a tick in flight, the main thread takes its style inputs for applied, as it does beside a layout pass,
        // which is only submitted once the frame's style rounds applied every transaction. A tick submitted with one
        // pending would have a later task's change merge into it as one style change.
        auto& style_engine = document->style_computer().style_engine();
        if (style_engine.has_pending_transaction() || style_engine.has_deferred_geometry_transaction()) {
            revoke_clock_lease(*held);
            continue;
        }
        if (!Layout::RustFFI::rust_document_clock_submit_tick(document->render_inputs_for_write().style_engine().rust_handle(), arena, time->value)) {
            revoke_clock_lease(*held);
            continue;
        }
        m_clock_leases[*held].ticked = true;
        submit_pass(FrameTicket::SubmittedPass::Kind::Clock, docs, document_index, frame_timestamp);
        return true;
    }
    return false;
}

FrameScheduler::AdoptedClockTicks FrameScheduler::adopt_clock_tick(DOM::Document& document)
{
    auto* arena = document.layout_arena_handle();
    if (!arena)
        return { AK::NaN<double>, {} };
    // What the ticks left, the document adopts as the last of them left it. Their snapshot converts rects to viewport
    // space as one the document published now would: the ticks move no visual context.
    auto visual_contexts = document.accumulated_visual_contexts_are_up_to_date() ? Painting::QueryVisualContexts::UpToDate : Painting::QueryVisualContexts::Stale;
    auto adoption = Layout::RustFFI::rust_document_clock_take_adoption(arena, Painting::QuerySnapshot::viewport_of(document, visual_contexts));
    AdoptedClockTicks adopted { adoption.time, Painting::QuerySnapshot::adopt(adoption.query_snapshot, visual_contexts) };
    if (!adoption.has_samples)
        return adopted;
    // What the tick sampled is the document's style now, which no input of the main thread's installed.
    (void)document.render_inputs_for_write();
    bool installed_any = false;
    // What the render side presented already, adopting repaints nothing of.
    bool const presented_on_render_side = Layout::RustFFI::rust_document_clock_presented_since_adoption(arena);
    CSS::StyleEffectDrain::install(document, [&](CSS::StyleDrainScope const& scope) {
        u32 style_node = 0;
        u64 style_record_before = 0;
        bool installed_in_arena = false;
        CSS::StyleEngineFFI::FfiRowSampledInPass sample {};
        while (CSS::StyleEngineFFI::style_engine_clock_tick_take_entry(&style_node, &style_record_before, &installed_in_arena, &sample)) {
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
    Layout::RustFFI::rust_document_clock_drop_unadopted(arena);
    if (installed_any)
        Layout::RustFFI::rust_clock_ticks_note_presented();
    // A tick that could not sample every effect, or reached the deadline, ends the lease: the rendering update samples
    // the effects itself. So does a lease that was ended while its tick was in flight.
    if (auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; }); held.has_value()) {
        if (m_clock_leases[*held].revoke_at_adoption || Layout::RustFFI::rust_document_clock_tick_outcome(arena) != Layout::RustFFI::FfiClockTickOutcome::Presented)
            revoke_clock_lease(*held);
    }
    return adopted;
}

void FrameScheduler::visit_edges(JS::Cell::Visitor& visitor)
{
    visitor.visit(m_deferred_arena_changes);
    for (auto& hold : m_clock_leases) {
        visitor.visit(hold.document);
        visitor.visit(hold.effects);
    }
    if (!m_ticket)
        return;
    for (auto& submitted : m_ticket->navigables) {
        visitor.visit(submitted.navigable);
        visitor.visit(submitted.frame.document);
        if (submitted.frame.recording)
            visitor.visit(submitted.frame.recording->document);
    }
    visitor.visit(m_ticket->painted_local_roots);
    if (m_ticket->submitted_pass.has_value()) {
        visitor.visit(m_ticket->submitted_pass->documents);
        visitor.visit(m_ticket->submitted_pass->sealed_flight_paint);
    }
}

}
