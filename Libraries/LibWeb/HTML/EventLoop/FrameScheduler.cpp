/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Time.h>
#include <LibCore/EventLoop.h>
#include <LibWeb/Bindings/MainThreadVM.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameInFlightReferences.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/PendingDisplayListRecording.h>
#include <LibWeb/Platform/Timer.h>

namespace Web::HTML {

static FrameScheduler* s_frame_scheduler_with_host = nullptr;

// The render side reaches the main thread's scheduler through these. Installed once, by the first scheduler that
// submits a frame.
static void install_frame_scheduler_host(FrameScheduler& scheduler)
{
    if (!Layout::RustFFI::rust_stage_thread_wants_frame_scheduler_host())
        return;
    s_frame_scheduler_with_host = &scheduler;
    static Core::WeakEventLoopReference* s_main_event_loop = nullptr;
    s_main_event_loop = &Core::EventLoop::current_weak().leak_ref();
    Layout::RustFFI::rust_stage_thread_set_frame_scheduler_host({
        .frame_completion_notify = [] {
            if (auto event_loop = s_main_event_loop->take(); event_loop.is_alive())
                event_loop->wake(); },
        .consume_commit = [] { s_frame_scheduler_with_host->consume_commit(); },
    });
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
    m_ticket->navigables.append({ navigable, move(frame) });
}

void FrameScheduler::retire_frames_for(DOM::Document& document)
{
    // FIXME: Retire the document's frame (discard its presentation work) instead of taking it in.
    if (m_ticket && m_ticket->navigables.first_matching([&](auto const& entry) { return entry.frame.document.ptr() == &document; }).has_value()) {
        auto location = SourceLocation::current();
        Layout::RustFFI::rust_stage_thread_join_frame_in_flight(reinterpret_cast<u8 const*>(location.filename().characters_without_null_termination()), location.filename().length(), location.line_number());
    }
}

bool FrameScheduler::submit(Vector<GC::Ref<DOM::Document>> documents)
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
    m_ticket->documents = move(documents);
    m_state = State::InFlight;
    m_event_loop.did_submit_frame();
    if (!m_completion_poll_timer) {
        m_completion_poll_timer = Platform::Timer::create_repeating(GC::Heap::the(), 1, GC::create_function(GC::Heap::the(), [this] {
            frame_completion_notify_poll();
        }));
    }
    m_completion_poll_timer->start();
    return true;
}

void FrameScheduler::frame_completion_notify_poll()
{
    if (m_state != State::InFlight && m_state != State::CommittedTailPending) {
        m_completion_poll_timer->stop();
        return;
    }
    if (m_state == State::CommittedTailPending || Layout::RustFFI::rust_stage_thread_frame_in_flight_has_finished())
        m_event_loop.schedule();
}

void FrameScheduler::consume_commit()
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
    // NB: Each navigable's recording is published and its resources are added to its resource storage before its
    //     compositor frame is built and handed off, so a compositor frame never reaches its sink ahead of the
    //     resources it references. The canvases it shows were flushed before the recording was prepared, and the next
    //     flush waits for the next rendering update.
    auto navigables = move(m_ticket->navigables);
    for (auto& [navigable, frame] : navigables) {
        // FIXME: A navigable destroyed while its frame was in flight retires the frame instead of finishing it.
        navigable->finish_painting_next_frame(frame);
        m_event_loop.note_frame_painted({});
        if (navigable->is_local_root())
            m_ticket->painted_local_roots.append(navigable);
    }
    release_holds_for_frame_in_flight();
    m_state = state_after;
    if (counts_as_a_frame)
        m_event_loop.did_consume_frame_commit(MonotonicTime::now().nanoseconds() - start_nanoseconds);
}

bool FrameScheduler::may_run_tail() const
{
    // The tail runs script, so it waits for an empty JavaScript execution context stack and an event loop that is not
    // paused.
    // FIXME: Decide which nested event loops (spin_until) may run a tail.
    return !m_event_loop.execution_paused() && Bindings::main_thread_vm().execution_context_stack().is_empty();
}

void FrameScheduler::run_at_step_1()
{
    if (m_state == State::InFlight && Layout::RustFFI::rust_stage_thread_frame_in_flight_has_finished()) {
        Layout::RustFFI::rust_stage_thread_take_frame_in_flight();
        consume_commit();
    }
    if (m_state == State::CommittedTailPending && may_run_tail())
        run_tail();
}

void FrameScheduler::finish_frame_now()
{
    if (m_state == State::InFlight) {
        Layout::RustFFI::rust_stage_thread_take_frame_in_flight();
        consume_commit();
    }
    if (m_state == State::CommittedTailPending)
        run_tail();
    VERIFY(m_state == State::Idle);
}

void FrameScheduler::run_tail()
{
    VERIFY(m_state == State::CommittedTailPending);
    // NB: The stack is a conservative root, so what the ticket held stays alive in these locals.
    auto documents = move(m_ticket->documents);
    auto painted_local_roots = move(m_ticket->painted_local_roots);
    m_ticket = nullptr;
    m_state = State::Idle;
    auto start_nanoseconds = MonotonicTime::now().nanoseconds();
    m_event_loop.run_rendering_update_tail({}, painted_local_roots, documents);
    m_event_loop.did_consume_frame_tail(MonotonicTime::now().nanoseconds() - start_nanoseconds);
}

void FrameScheduler::visit_edges(JS::Cell::Visitor& visitor)
{
    visitor.visit(m_completion_poll_timer);
    if (!m_ticket)
        return;
    for (auto& [navigable, frame] : m_ticket->navigables) {
        visitor.visit(navigable);
        visitor.visit(frame.document);
        if (frame.recording)
            visitor.visit(frame.recording->document);
    }
    visitor.visit(m_ticket->documents);
    visitor.visit(m_ticket->painted_local_roots);
}

}
