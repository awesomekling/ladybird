/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/AnyOf.h>
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

static void install_render_clock_host()
{
    if (!Layout::RustFFI::rust_clock_frames_enabled())
        return;
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

FrameScheduler::~FrameScheduler() = default;

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
    // Under LIBWEB_RENDER_PRESENTS=1, the frame in flight presents the frame once it has recorded it. Frames reach their
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
    submit_pass(FrameTicket::SubmittedPass::Kind::Layout, move(documents), document_index, frame_timestamp);
}

void FrameScheduler::submit_style(Vector<GC::Ref<DOM::Document>> documents, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    submit_pass(FrameTicket::SubmittedPass::Kind::Style, move(documents), document_index, frame_timestamp);
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
    // A clock tick's document adopts what the tick installed before anything reads it.
    if (m_ticket->submitted_pass.has_value() && m_ticket->submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Clock)
        adopt_clock_tick(m_ticket->submitted_pass->documents[m_ticket->submitted_pass->document_index]);
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
        // The rest of the rendering update is a main half of its own, with a new ticket for its recordings.
        m_ticket = make<FrameTicket>();
        m_state = State::MainHalf;
        if (submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Clock) {
            // The next leased document ticks, and then the rendering update goes on at step 16 for every document.
            if (tick_clock_leases(submitted_pass->documents, submitted_pass->document_index + 1, submitted_pass->frame_timestamp, true))
                return;
            m_event_loop.resume_rendering_update_after_style({}, submitted_pass->documents, 0, submitted_pass->frame_timestamp);
        } else if (submitted_pass->kind == FrameTicket::SubmittedPass::Kind::Style)
            m_event_loop.resume_rendering_update_after_style({}, submitted_pass->documents, submitted_pass->document_index, submitted_pass->frame_timestamp);
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

void FrameScheduler::revoke_clock_lease(size_t index)
{
    auto hold = m_clock_leases.take(index);
    // What the render clock's ticks laid out goes in before the lease that holds it ends.
    take_in_clock_layout_frame(*hold.document);
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
        // The render clock's ticks lay out what their samples leave in a frame of their own.
        if (hold.render_clock_context.has_value())
            Layout::RustFFI::layout_arena_renew_clock_layout_frame(document->layout_node_arena_if_created()->handle());
    }
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
    while (!m_clock_leases.is_empty())
        revoke_clock_lease(m_clock_leases.size() - 1);
}

void FrameScheduler::main_thread_will_idle()
{
    // Nothing ticks beside a frame in flight: the main thread takes it back first.
    if (m_clock_leases.is_empty() || m_state != State::Idle || Layout::RustFFI::rust_stage_thread_has_frame_in_flight())
        return;
    // A tick samples each element over the record it holds, which the main thread may have moved since the grant: a
    // lease its document no longer plans the same way waits for the rendering update that ends it.
    bool any_ticks = false;
    for (auto& hold : m_clock_leases) {
        auto* arena = hold.document->layout_node_arena_if_created();
        if (!arena || !hold.render_clock_context.has_value())
            continue;
        auto plan = clock_lease_plan(*hold.document);
        bool ticks = plan.has_value() && plan->effects == hold.effects && publish_clock_lease_targets(hold);
        Layout::RustFFI::rust_clock_lease_set_paused(arena->handle(), !ticks);
        any_ticks |= ticks;
    }
    if (any_ticks)
        Layout::RustFFI::rust_render_clock_main_will_idle();
}

void FrameScheduler::main_thread_did_wake()
{
    if (!Layout::RustFFI::rust_render_clock_main_did_wake())
        return;
    // What the render clock's ticks installed ahead of the main thread, each document adopts before anything else
    // reaches it, and its timeline shows the time of the last tick.
    for (size_t index = m_clock_leases.size(); index-- > 0;) {
        auto document = m_clock_leases[index].document;
        auto* arena = document->layout_node_arena_if_created();
        if (!arena || !m_clock_leases[index].render_clock_context.has_value())
            continue;
        auto time = Layout::RustFFI::rust_clock_lease_time(arena->handle());
        adopt_clock_tick(*document);
        // The style the ticks installed is the document's now, and so is what they laid out with it.
        take_in_clock_layout_frame(*document);
        if (!isnan(time)) {
            if (auto current = document->timeline()->current_time(); current.has_value() && current->type == Animations::TimeValue::Type::Milliseconds && current->value < time)
                document->timeline()->update_current_time(time);
        }
    }
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
        if (!(time < plan->deadline))
            revoke_clock_lease(index);
    }
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
            Animations::adopt_clock_tick_sample(scope, DOM::AbstractElement { *element }, CSS::StyleRecordID { style_record_before }, sample, installed_in_arena);
            installed_any = true;
        }
    });
    // What no element adopted leaves the arena's log, with its pins.
    Layout::RustFFI::rust_clock_lease_drop_unadopted(arena->handle());
    if (installed_any)
        Layout::RustFFI::rust_clock_ticks_note_presented();
    // A tick that could not sample every effect, or reached the deadline, ends the lease: the rendering update samples
    // the effects itself.
    if (Layout::RustFFI::rust_clock_lease_tick_outcome(arena->handle()) != Layout::RustFFI::FfiClockTickOutcome::Presented) {
        if (auto held = m_clock_leases.find_first_index_if([&](auto const& hold) { return hold.document.ptr() == &document; }); held.has_value())
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
