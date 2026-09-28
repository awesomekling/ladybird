/*
 * Copyright (c) 2021, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2022, the SerenityOS developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/AnyOf.h>
#include <AK/Debug.h>
#include <AK/ScopeGuard.h>
#include <AK/TemporaryChange.h>
#include <AK/Time.h>
#include <LibCore/EventLoop.h>
#include <LibGC/Heap.h>
#include <LibJS/Runtime/VM.h>
#include <LibWeb/Animations/DocumentTimeline.h>
#include <LibWeb/Animations/ScrollTimeline.h>
#include <LibWeb/Bindings/MainThreadVM.h>
#include <LibWeb/CSS/FontComputer.h>
#include <LibWeb/CSS/FontFaceSet.h>
#include <LibWeb/Compositor/NavigablePresenter.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/InvalidationJournal.h>
#include <LibWeb/HTML/BrowsingContext.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameCompletion.h>
#include <LibWeb/HTML/EventLoop/FrameInFlightReferences.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/HTML/EventLoop/MainThreadPhases.h>
#include <LibWeb/HTML/HTMLMediaElement.h>
#include <LibWeb/HTML/LocalTraversableNavigable.h>
#include <LibWeb/HTML/NavigableContainer.h>
#include <LibWeb/HTML/Scripting/Agent.h>
#include <LibWeb/HTML/Scripting/Environments.h>
#include <LibWeb/HTML/Scripting/TemporaryExecutionContext.h>
#include <LibWeb/HTML/Window.h>
#include <LibWeb/HTML/WorkletGlobalScope.h>
#include <LibWeb/HighResolutionTime/Performance.h>
#include <LibWeb/HighResolutionTime/TimeOrigin.h>
#include <LibWeb/IndexedDB/Internal/Algorithms.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/PendingDisplayListRecording.h>
#include <LibWeb/Platform/EventLoopPlugin.h>
#include <LibWeb/Platform/Timer.h>
#include <LibWebCommon/Page/QueuedInputEvent.h>

namespace Web::HTML {

GC_DEFINE_ALLOCATOR(EventLoop);

EventLoop::EventLoop(Type type)
    : m_type(type)
    , m_frame_scheduler(make<FrameScheduler>(*this))
{
    if (m_type == Type::Window)
        forbid_heap_access_on_the_stage_thread();
    m_task_queue = GC::Heap::the().allocate<TaskQueue>(*this);

    m_rendering_task_function = GC::create_function(GC::Heap::the(), [this] {
        run_rendering_task();
    });
}

EventLoop::~EventLoop() = default;

bool EventLoop::s_a_frame_is_in_flight { false };

// One frame interval at 60 Hz: how long a rendering task waits behind other tasks before it runs ahead of them, and
// how long after its rendering opportunity the display has offered another.
static constexpr u64 rendering_task_queue_wait_limit_nanoseconds = 1'000'000'000 / 60;

void EventLoop::run_rendering_task()
{
    VERIFY(m_rendering_task_queued);
    VERIFY(!m_rendering_task_held);
    m_rendering_task_runs_ahead = false;
    // The previous rendering update's frame and tail come first. The tail hands its pages the rendering opportunity
    // this task was queued for again, and while the task still counts as queued, that queues no second one.
    // A frame the render side is still working on is not waited for: the task holds its opportunity instead, and the
    // step 1 that takes the frame in and runs its tail queues it again.
    if (!m_frame_scheduler->finish_finished_frames()) {
        m_rendering_task_held = true;
        ++m_rendering_scheduler_counters.rendering_tasks_held;
        return;
    }
    m_rendering_scheduler_counters.rendering_task_blocked_on_frame_nanoseconds += m_frame_scheduler->finish_frame_now();
    m_rendering_task_queued = false;
    // AD-HOC: The in-parallel steps set the last render opportunity time at every rendering opportunity, and the display
    //         went on offering them while this task waited behind other tasks. A task that waited a display frame or
    //         more takes the latest of them, so a document those tasks created is not given a frame from before it
    //         existed. An injected opportunity keeps the time it was given.
    if (m_last_render_opportunity_source != RenderingOpportunitySource::Manual
        && MonotonicTime::now().nanoseconds() - m_rendering_task_queued_at_nanoseconds >= rendering_task_queue_wait_limit_nanoseconds)
        m_last_render_opportunity_time = max(m_last_render_opportunity_time, HighResolutionTime::unsafe_shared_current_time());
    update_the_rendering();
}

void EventLoop::queue_held_rendering_task_if_frame_finished()
{
    if (!m_rendering_task_held || m_frame_scheduler->has_unfinished_frame())
        return;
    // The held rendering update goes right after the frame it waited for (normally taken in, with its tail run, by
    // this step 1's finished frame consumer), ahead of the tasks queued while it was held.
    VERIFY(m_rendering_task_queued);
    m_rendering_task_held = false;
    m_rendering_task_runs_ahead = true;
    queue_a_task(Task::Source::Rendering, this, nullptr, *m_rendering_task_function);
}

bool EventLoop::rendering_task_runs_ahead_of_queue() const
{
    if (!m_rendering_task_queued || m_rendering_task_held)
        return false;
    if (m_rendering_task_runs_ahead)
        return true;
    if (m_rendering_task_ran_ahead_since_last_task)
        return false;
    return MonotonicTime::now().nanoseconds() - m_rendering_task_queued_at_nanoseconds >= rendering_task_queue_wait_limit_nanoseconds;
}

void EventLoop::did_run_rendering_task_ahead_of_queue()
{
    ++m_rendering_scheduler_counters.rendering_tasks_ahead_of_queue;
    // The tasks it ran ahead of get to run before a rendering task runs ahead of them again.
    m_rendering_task_ran_ahead_since_last_task = true;
}

StringView EventLoop::frame_lockstep_reason_name(FrameLockstepReason reason)
{
    switch (reason) {
    case FrameLockstepReason::JoinsLeft:
        return "joinsLeft"sv;
    case FrameLockstepReason::ResizeObserverDocument:
        return "resizeObserverDocument"sv;
    case FrameLockstepReason::ViewTransition:
        return "viewTransition"sv;
    case FrameLockstepReason::SynchronousCaller:
        return "synchronousCaller"sv;
    case FrameLockstepReason::Count:
        break;
    }
    VERIFY_NOT_REACHED();
}

StringView EventLoop::journal_entry_kind_name(JournalEntryKind kind)
{
    switch (kind) {
    case JournalEntryKind::LayoutUpdate:
        return "layoutUpdate"sv;
    case JournalEntryKind::LayoutTreeUpdate:
        return "layoutTreeUpdate"sv;
    case JournalEntryKind::Repaint:
        return "repaint"sv;
    case JournalEntryKind::PaintFacts:
        return "paintFacts"sv;
    case JournalEntryKind::PaintCache:
        return "paintCache"sv;
    case JournalEntryKind::Editability:
        return "editability"sv;
    case JournalEntryKind::Selection:
        return "selection"sv;
    case JournalEntryKind::ScrollOffset:
        return "scrollOffset"sv;
    case JournalEntryKind::Scrollbar:
        return "scrollbar"sv;
    case JournalEntryKind::TextData:
        return "textData"sv;
    case JournalEntryKind::SVGAttributes:
        return "svgAttributes"sv;
    case JournalEntryKind::TableSpans:
        return "tableSpans"sv;
    case JournalEntryKind::VisualContext:
        return "visualContext"sv;
    case JournalEntryKind::Count:
        break;
    }
    VERIFY_NOT_REACHED();
}

void EventLoop::reset_rendering_scheduler_counters()
{
    m_rendering_scheduler_counters = {};
    m_rendering_scheduler_counters_at_last_update = {};
    m_last_rendering_update_end_time = 0;
    Layout::RustFFI::layout_arena_reset_door_counters();
}

void EventLoop::did_submit_frame()
{
    VERIFY(!s_a_frame_is_in_flight);
    s_a_frame_is_in_flight = true;
    m_frame_submitted_at_nanoseconds = MonotonicTime::now().nanoseconds();
    ++m_rendering_scheduler_counters.frames_submitted;
    ++m_frames_submitted_by_rendering_update;
}

void EventLoop::did_consume_frame_commit(u64 nanoseconds)
{
    VERIFY(s_a_frame_is_in_flight);
    s_a_frame_is_in_flight = false;
    auto& counters = m_rendering_scheduler_counters;
    auto in_flight = MonotonicTime::now().nanoseconds() - m_frame_submitted_at_nanoseconds;
    ++counters.frames_consumed;
    counters.frame_in_flight_nanoseconds += in_flight;
    counters.consume_commit_nanoseconds += nanoseconds;
    auto submit_to_consume = in_flight + nanoseconds;
    counters.submit_to_consume_nanoseconds += submit_to_consume;
    counters.max_submit_to_consume_nanoseconds = max(counters.max_submit_to_consume_nanoseconds, submit_to_consume);
}

void EventLoop::did_consume_frame_tail(u64 nanoseconds)
{
    m_rendering_scheduler_counters.consume_tail_nanoseconds += nanoseconds;
}

void EventLoop::visit_edges(Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_reached_step_1_tasks);
    visitor.visit(m_task_queue);
    m_microtask_queue.for_each([&](auto& task) { visitor.visit(task); });
    visitor.visit(m_currently_running_task);
    visitor.visit(m_backup_incumbent_realm_stack);
    visitor.visit(m_rendering_task_function);
    visitor.visit(m_system_event_loop_timer);
    visitor.visit(m_idle_period_timer);
    visitor.visit(m_finished_frame_consumer);
    m_frame_scheduler->visit_edges(visitor);
}

void EventLoop::schedule()
{
    if (!m_system_event_loop_timer) {
        m_system_event_loop_timer = Platform::Timer::create_single_shot(GC::Heap::the(), 0, GC::create_function(GC::Heap::the(), [this] {
            process();
        }));
    }

    if (!m_system_event_loop_timer->is_active())
        m_system_event_loop_timer->restart();
}

EventLoop& main_thread_event_loop()
{
    return *static_cast<HTML::Agent*>(Bindings::main_thread_vm().agent())->event_loop;
}

// https://html.spec.whatwg.org/multipage/webappapis.html#spin-the-event-loop
void EventLoop::spin_until(GC::Ref<GC::Function<bool()>> goal_condition)
{
    // FIXME: The spec wants us to do the rest of the enclosing algorithm (i.e. the caller)
    //    in the context of the currently running task on entry. That's not possible with this implementation.
    // 1. Let task be the event loop's currently running task.
    // 2. Let task source be task's source.

    // 3. Let old stack be a copy of the JavaScript execution context stack.
    // 4. Empty the JavaScript execution context stack.
    auto& vm = this->vm();
    vm.save_execution_context_stack();
    vm.clear_execution_context_stack();
    ++m_spin_depth;

    // 5. Perform a microtask checkpoint.
    perform_a_microtask_checkpoint();

    // 6. In parallel:
    //    1. Wait until the condition goal is met.
    //    2. Queue a task on task source to:
    //       1. Replace the JavaScript execution context stack with old stack.
    //       2. Perform any steps that appear after this spin the event loop instance in the original algorithm.
    //       NOTE: This is achieved by returning from the function.

    Platform::EventLoopPlugin::the().spin_until(GC::create_function(GC::Heap::the(), [this, goal_condition] {
        if (goal_condition->function()())
            return true;
        if (m_task_queue->has_runnable_tasks()) {
            schedule();
            // FIXME: Remove the platform event loop plugin so that this doesn't look out of place
            Core::EventLoop::current().wake();
        }
        return goal_condition->function()();
    }));

    vm.restore_execution_context_stack();
    --m_spin_depth;

    // A finished frame's tail the spin held back runs at the next outermost step 1.
    if (m_spin_depth == 0 && has_finished_frame_work())
        schedule();

    // 7. Stop task, allowing whatever algorithm that invoked it to resume.
    // NOTE: This is achieved by returning from the function.
}

// https://html.spec.whatwg.org/multipage/webappapis.html#event-loop-processing-model
void EventLoop::process()
{
    if (execution_paused())
        return;

    ++m_processing_depth;
    ScopeGuard leave_processing = [this] { --m_processing_depth; };

    // 1. Let oldestTask and taskStartTime be null.
    GC::Ptr<Task> oldest_task;
    [[maybe_unused]] double task_start_time = 0;
    bool task_started_with_frame_in_flight = false;

    m_task_generation++;

    // AD-HOC: A frame that finished beside this event loop is consumed here, before anything else sees the document.
    if (m_finished_frame_consumer && has_finished_frame_work()) {
        m_finished_frame_consumer_call_requested = false;
        ++m_rendering_scheduler_counters.finished_frame_consumer_calls;
        TemporaryChange at_step_one { m_calling_finished_frame_consumer, true };
        MainThreadPhases::Scope phase { MainThreadPhases::Phase::FrameConsumer };
        m_finished_frame_consumer->function()();
    }

    // AD-HOC: What the render clock's ticks installed beside this event loop is adopted here, before anything else sees
    //         the documents.
    if (m_type == Type::Window)
        m_frame_scheduler->adopt_render_clock_ticks_if_any();

    // AD-HOC: A rendering task that held its rendering opportunity while a frame was in flight is queued again once
    //         the render side has finished that frame.
    queue_held_rendering_task_if_frame_finished();

    // Some algorithms request that steps or states only occur once the event loop has reached step 1.
    // Invoke a set of tasks that these algorithms request us to in order to achieve this.
    auto reached_step_1_tasks = move(m_reached_step_1_tasks);
    for (auto& reached_step_1_task : reached_step_1_tasks) {
        MainThreadPhases::Scope phase { MainThreadPhases::Phase::StepOne };
        reached_step_1_task->function()();
    }

    // 2. If the event loop has a task queue with at least one runnable task, then:
    if (m_task_queue->has_runnable_tasks()) {
        // 1. Let taskQueue be one such task queue, chosen in an implementation-defined manner.
        auto task_queue = m_task_queue;

        // 2. Set taskStartTime to the unsafe shared current time.
        task_start_time = HighResolutionTime::unsafe_shared_current_time();
        if (s_a_frame_is_in_flight) {
            ++m_rendering_scheduler_counters.tasks_started_with_frame_in_flight;
            task_started_with_frame_in_flight = true;
        }

        // 3. Set oldestTask to the first runnable task in taskQueue, and remove it from taskQueue.
        oldest_task = task_queue->take_first_runnable();

        // FIXME: 4. If oldestTask's document is not null, then record task start time given taskStartTime and oldestTask's document.

        // 5. Set the event loop's currently running task to oldestTask.
        m_currently_running_task = oldest_task.ptr();

        // 6. Perform oldestTask's steps.
        MainThreadPhases::Scope phase { MainThreadPhases::task_phase(*oldest_task) };
        oldest_task->execute();

        // 7. Set the event loop's currently running task back to null.
        m_currently_running_task = nullptr;

        // 8. Perform a microtask checkpoint.
        perform_a_microtask_checkpoint();
    }

    // 3. Let taskEndTime be the unsafe shared current time. [HRT]
    [[maybe_unused]] auto task_end_time = HighResolutionTime::unsafe_shared_current_time();

    if (oldest_task && oldest_task->source() != Task::Source::Rendering) {
        m_rendering_task_ran_ahead_since_last_task = false;
        auto task_duration = task_end_time - task_start_time;
        auto task_duration_microseconds = static_cast<u64>(task_duration * 1000.0);
        ++m_rendering_scheduler_counters.tasks_between_updates;
        m_rendering_scheduler_counters.task_microseconds_between_updates += task_duration_microseconds;
        if (task_started_with_frame_in_flight)
            m_rendering_scheduler_counters.overlap_task_nanoseconds += static_cast<u64>(task_duration * 1'000'000.0);
        switch (oldest_task->source()) {
        case Task::Source::PostedMessage:
            ++m_rendering_scheduler_counters.posted_message_tasks_between_updates;
            m_rendering_scheduler_counters.posted_message_task_microseconds_between_updates += task_duration_microseconds;
            break;
        case Task::Source::TimerTask:
            ++m_rendering_scheduler_counters.timer_tasks_between_updates;
            m_rendering_scheduler_counters.timer_task_microseconds_between_updates += task_duration_microseconds;
            break;
        case Task::Source::Networking:
            ++m_rendering_scheduler_counters.networking_tasks_between_updates;
            m_rendering_scheduler_counters.networking_task_microseconds_between_updates += task_duration_microseconds;
            break;
        case Task::Source::DOMManipulation:
            ++m_rendering_scheduler_counters.dom_manipulation_tasks_between_updates;
            m_rendering_scheduler_counters.dom_manipulation_task_microseconds_between_updates += task_duration_microseconds;
            break;
        default:
            break;
        }
    }

    // 4. If oldestTask is not null, then:
    if (oldest_task) {
        // FIXME: 1. Let top-level browsing contexts be an empty set.
        // FIXME: 2. For each environment settings object settings of oldestTask's script evaluation environment settings object set:
        // FIXME: 2.1. Let global be settings's global object.
        // FIXME: 2.2. If global is not a Window object, then continue.
        // FIXME: 2.3. If global's browsing context is null, then continue.
        // FIXME: 2.4. Let tlbc be global's browsing context's top-level browsing context.
        // FIXME: 2.5. If tlbc is not null, then append it to top-level browsing contexts.
        // FIXME: 3. Report long tasks, passing in taskStartTime, taskEndTime, top-level browsing contexts, and oldestTask.
        // FIXME: 4. If oldestTask's document is not null, then record task end time given taskEndTime and oldestTask's document.
    }

    // 5. If this is a window event loop that has no runnable task in this event loop's task queues, then:
    if (m_type == Type::Window && !m_task_queue->has_runnable_tasks() && (!m_idle_period_timer || !m_idle_period_timer->is_active())) {
        auto windows = same_loop_windows();
        bool has_idle_callbacks = false;
        for (auto& window : windows) {
            if (!window->associated_document().hidden() && window->has_idle_callbacks()) {
                has_idle_callbacks = true;
                break;
            }
        }
        if (has_idle_callbacks) {
            // NB: Delay the next idle period until this one's 50 ms budget has elapsed. Callbacks registered
            //     during an idle period belong to the next one; immediately starting it lets self-scheduling
            //     callbacks spin continuously even when they have no work to do.
            if (!m_idle_period_timer) {
                m_idle_period_timer = Platform::Timer::create_single_shot(GC::Heap::the(), 50, GC::create_function(GC::Heap::the(), [this] {
                    schedule();
                }));
            }
            m_idle_period_timer->restart();

            // 1. Set this event loop's last idle period start time to the unsafe shared current time.
            m_last_idle_period_start_time = HighResolutionTime::unsafe_shared_current_time();

            // 2. Let computeDeadline be the following steps:
            // Implemented in EventLoop::compute_deadline()

            // 3. For each win of the same-loop windows for this event loop, perform the start an idle period algorithm for win with the following step: return the result of calling computeDeadline, coarsened given win's relevant settings object's cross-origin isolated capability. [REQUESTIDLECALLBACK]
            for (auto& window : windows)
                window->start_an_idle_period();
        }
    }

    // If there are eligible tasks in the queue, schedule a new round of processing. :^)
    if (m_task_queue->has_runnable_tasks() || (!m_microtask_queue.is_empty() && !m_performing_a_microtask_checkpoint)) {
        schedule();
    }
}

void EventLoop::set_finished_frame_consumer(GC::Ptr<GC::Function<void()>> consumer)
{
    VERIFY(this == &main_thread_event_loop());
    m_finished_frame_consumer = consumer;
    if (!consumer)
        return;
    // The delivery only schedules processing; the consumer runs at step 1, where it is safe to.
    FrameCompletion::the().register_event_loop([] {
        main_thread_event_loop().schedule();
    });
}

bool EventLoop::has_finished_frame_work() const
{
    return m_finished_frame_consumer_call_requested || FrameCompletion::the().is_pending();
}

bool EventLoop::may_consume_commit(FrameConsumeSite site) const
{
    if (consuming_frame())
        return false;
    switch (site) {
    case FrameConsumeSite::ForcedJoin:
        // The caller needs the frame's result now, wherever it is, and the commit runs no script.
        return true;
    case FrameConsumeSite::StepOne:
        // A rendering update consumes the frame it owns itself; a nested loop inside it must not.
        return !execution_paused() && !m_running_rendering_task;
    }
    VERIFY_NOT_REACHED();
}

bool EventLoop::may_run_consume_tail() const
{
    // Only the outermost step 1, with nothing suspended below it: an empty JavaScript execution context stack is not
    // enough, since spinning the event loop empties it above a suspended caller, and neither is the lack of a currently
    // running task, since a nested loop that ran a task leaves none behind for the task it is nested in.
    return m_calling_finished_frame_consumer
        && m_processing_depth == 1
        && m_spin_depth == 0
        && !execution_paused()
        && !consuming_frame()
        && !m_running_rendering_task
        && !m_performing_a_microtask_checkpoint
        && !m_currently_running_task
        && vm().execution_context_stack().is_empty();
}

void EventLoop::consume_commit(FrameConsumeSite site, Function<void()> const& commit)
{
    VERIFY(may_consume_commit(site));
    TemporaryChange consuming { m_consuming_frame_commit, true };
    commit();
}

void EventLoop::run_consume_tail(Function<void()> const& tail)
{
    VERIFY(may_run_consume_tail());
    {
        TemporaryChange running_tail { m_running_consume_tail, true };
        tail();
    }
    // The tail ends like a task: the microtasks its callbacks queued run before anything else.
    perform_a_microtask_checkpoint();
}

void EventLoop::request_rendering_update()
{
    ++m_rendering_scheduler_counters.update_requests;
    if (m_running_rendering_task)
        ++m_rendering_scheduler_counters.update_requests_while_rendering;

    if (m_rendering_update_requested || m_rendering_task_queued) {
        ++m_rendering_scheduler_counters.coalesced_update_requests;
        return;
    }

    m_rendering_update_requested = true;
}

// https://html.spec.whatwg.org/multipage/webappapis.html#event-loop-processing-model
bool EventLoop::rendering_opportunity(HighResolutionTime::DOMHighResTimeStamp frame_time, RenderingOpportunitySource source)
{
    ++m_rendering_scheduler_counters.opportunities_received;
    if (source == RenderingOpportunitySource::Watchdog)
        ++m_rendering_scheduler_counters.watchdog_opportunities;

    // FIXME: 1. Wait until at least one navigable whose active document's relevant agent's event loop is eventLoop might have a rendering opportunity.

    // 2. Set eventLoop's last render opportunity time to the unsafe shared current time.
    // INTEROP: Compositor-provided display timestamps match the shared monotonic clock. Clamp delayed or out-of-order
    //          IPC delivery so rendering timestamps remain monotonic and never describe a future frame.
    auto now = HighResolutionTime::unsafe_shared_current_time();
    m_last_render_opportunity_time = max(m_last_render_opportunity_time, min(frame_time, now));
    m_last_render_opportunity_source = source;

    // AD-HOC: A nested event loop can deliver a timer while a rendering update is running. Keep the request pending
    //         so the PageClient can schedule it for the next opportunity instead of queueing a second rendering task.
    if (m_running_rendering_task)
        return false;

    // NB: A rendering update whose frame has not been taken in, or whose tail has not run, does not hold the opportunity
    //     back: the rendering task queued for it finishes that frame first, as a rendering update in lockstep would
    //     have, or, where rendering opportunities are held, holds the opportunity itself until that frame has
    //     finished. Held back here, the opportunity would reach the rendering task queue only behind the tasks queued
    //     after it, and one granted by hand would be lost.
    m_rendering_update_requested = false;

    if (m_rendering_task_queued)
        return true;

    // 3. For each navigable that has a rendering opportunity, queue a global task on the rendering task source given navigable's active window to update the rendering:
    bool has_eligible_navigable = false;
    for (auto& navigable : all_local_navigables()) {
        if (!navigable->is_local_root())
            continue;
        if (!navigable->has_a_rendering_opportunity())
            continue;

        auto document = navigable->active_document();
        if (!document)
            continue;
        if (document->is_decoded_svg())
            continue;

        has_eligible_navigable = true;
        break;
    }

    if (!has_eligible_navigable)
        return false;

    // AD-HOC: One rendering update services every document in this event loop, so queue one event-loop task for the
    //         opportunity instead of one global task per page local root.
    VERIFY(!m_rendering_task_queued);
    m_rendering_task_queued = true;
    m_rendering_task_queued_at_nanoseconds = MonotonicTime::now().nanoseconds();
    queue_a_task(Task::Source::Rendering, this, nullptr, *m_rendering_task_function);
    ++m_rendering_scheduler_counters.opportunities_that_queued_a_task;
    return true;
}

void EventLoop::process_input_events() const
{
    auto process_input_events_queue = [&](Page& page) {
        auto& page_client = page.client();
        auto& input_events_queue = page_client.input_event_queue();

        // Process events only for this page, collecting others to re-enqueue
        Queue<Web::QueuedInputEvent> events_for_other_pages;

        while (!input_events_queue.is_empty()) {
            auto event = input_events_queue.dequeue();

            // Skip events that are not intended for this page
            if (event.page_id != page_client.id()) {
                events_for_other_pages.enqueue(move(event));
                continue;
            }

            // A key event goes to the tab's focused navigable. Other events go to the local root given by the event's
            // navigable ID, or the page's traversable if it has none.
            GC::Ptr<LocalNavigable> root;
            if (event.event.has<Compositing::KeyEvent>())
                root = page.hosted_focused_navigable();
            else if (event.navigable_id.has_value())
                root = as_if<LocalNavigable>(page.navigable_with_id(*event.navigable_id).ptr());
            else if (page.has_local_traversable())
                root = page.local_traversable();

            if (!root) {
                for (auto coalesced_event_id : event.coalesced_event_ids)
                    page_client.report_finished_handling_input_event(event.page_id, coalesced_event_id, EventResult::Dropped);
                page_client.report_finished_handling_input_event(event.page_id, input_event_id(event.event), EventResult::Dropped);
                continue;
            }

            Optional<RemoteInputEventTarget> remote_target;
            auto result = event.event.visit(
                [&](Compositing::KeyEvent const& key_event) {
                    switch (key_event.type) {
                    case Compositing::KeyEvent::Type::KeyDown:
                        return page.handle_keydown(key_event.key, key_event.modifiers, key_event.code_point, key_event.repeat, key_event.should_insert_text, key_event.async_scroll_performed_default_action);
                    case Compositing::KeyEvent::Type::KeyUp:
                        return page.handle_keyup(key_event.key, key_event.modifiers, key_event.code_point, key_event.repeat);
                    }
                    VERIFY_NOT_REACHED();
                },
                [&](Compositing::MouseEvent const& mouse_event) {
                    switch (mouse_event.type) {
                    case Compositing::MouseEvent::Type::MouseDown:
                        return page.handle_mousedown(*root, mouse_event.position, mouse_event.screen_position, mouse_event.button, mouse_event.buttons, mouse_event.modifiers, mouse_event.click_count, mouse_event.scrollbar_dragged_by_compositor, &remote_target);
                    case Compositing::MouseEvent::Type::MouseUp:
                        return page.handle_mouseup(*root, mouse_event.position, mouse_event.screen_position, mouse_event.button, mouse_event.buttons, mouse_event.modifiers, &remote_target);
                    case Compositing::MouseEvent::Type::MouseMove:
                        return page.handle_mousemove(*root, mouse_event.position, mouse_event.screen_position, mouse_event.buttons, mouse_event.modifiers, &remote_target);
                    case Compositing::MouseEvent::Type::MouseLeave:
                        return page.handle_mouseleave(*root);
                    case Compositing::MouseEvent::Type::MouseWheel:
                        if (mouse_event.async_scroll_performed_default_action) {
                            dbgln_if(COMPOSITOR_DEBUG, "[Compositor] Main thread handling DOM wheel after async default action");
                            return page.handle_mousewheel(*root, mouse_event.position, mouse_event.screen_position, mouse_event.button, mouse_event.buttons, mouse_event.modifiers, mouse_event.wheel_delta_x, mouse_event.wheel_delta_y, mouse_event.wheel_delta_precision, mouse_event.scroll_gesture_phase, true, nullptr, &remote_target);
                        }
                        return page.handle_mousewheel(*root, mouse_event.position, mouse_event.screen_position, mouse_event.button, mouse_event.buttons, mouse_event.modifiers, mouse_event.wheel_delta_x, mouse_event.wheel_delta_y, mouse_event.wheel_delta_precision, mouse_event.scroll_gesture_phase, false, nullptr, &remote_target);
                    }
                    VERIFY_NOT_REACHED();
                },
                [&](Web::DragEvent& drag_event) {
                    return page.handle_drag_and_drop_event(*root, drag_event.type, drag_event.position, drag_event.screen_position, drag_event.button, drag_event.buttons, drag_event.modifiers, move(drag_event.files));
                },
                [&](Compositing::PinchEvent& pinch_event) {
                    return page.handle_pinch_event(*root, pinch_event.position, pinch_event.modifiers, pinch_event.scale_delta);
                });

            for (auto coalesced_event_id : event.coalesced_event_ids)
                page_client.report_finished_handling_input_event(event.page_id, coalesced_event_id, EventResult::Dropped);

            // The pointer is over content another process hosts: that process handles the event, at the position in
            // the viewport of the navigable it hosts, and finishes it.
            if (remote_target.has_value()) {
                auto mouse_event = event.event.get<Compositing::MouseEvent>().clone_without_browser_data();
                mouse_event.position = page.css_to_device_point(remote_target->position);
                page_client.forward_mouse_event_to_remote_navigable(event.page_id, remote_target->navigable_id, move(mouse_event));
                continue;
            }

            page_client.did_handle_input_event(event.page_id, event.event);
            page_client.report_finished_handling_input_event(event.page_id, input_event_id(event.event), result);
        }

        // Re-enqueue events for other pages
        while (!events_for_other_pages.is_empty()) {
            input_events_queue.enqueue(events_for_other_pages.dequeue());
        }

        page.handle_sdl_input_events();
    };

    // Every page hosting a document takes the input events queued for it, once.
    Vector<GC::Ref<Page>> pages;
    for (auto& navigable : all_local_navigables()) {
        if (!navigable->is_local_root() || navigable->has_been_destroyed())
            continue;
        auto document = navigable->active_document();
        if (!document || document->is_decoded_svg())
            continue;
        if (!pages.contains_slow(GC::Ref { navigable->page() }))
            pages.append(navigable->page());
    }

    for (auto const& page : pages)
        process_input_events_queue(*page);
}

static GC::RootVector<GC::Ref<Page>> pages_of_local_roots()
{
    GC::RootVector<GC::Ref<Page>> pages;
    for (auto& navigable : all_local_navigables()) {
        if (!navigable->is_local_root())
            continue;
        if (!pages.contains_slow(GC::Ref { navigable->page() }))
            pages.append(navigable->page());
    }
    return pages;
}

// INTEROP: A compositor timestamp may describe a display tick immediately before a newly created document's
//          time origin. Keep document-relative rendering timestamps within the DOMHighResTimeStamp domain.
static HighResolutionTime::DOMHighResTimeStamp relative_frame_timestamp_for(HighResolutionTime::DOMHighResTimeStamp frame_timestamp, DOM::Document const& document)
{
    return max(0.0, HighResolutionTime::relative_high_resolution_time(frame_timestamp, relevant_global_object(document)));
}

// https://html.spec.whatwg.org/multipage/webappapis.html#update-the-rendering
void EventLoop::update_the_rendering()
{
    VERIFY(!m_running_rendering_task);
    // The previous rendering update's frame and tail come first.
    m_frame_scheduler->begin_main_half(m_running_synchronous_rendering_update);
    m_running_rendering_task = true;
    for (auto const& page : pages_of_local_roots())
        page->client().will_begin_rendering_update();
    m_rendering_update_start_time = HighResolutionTime::unsafe_shared_current_time();
    auto update_start_nanoseconds = MonotonicTime::now().nanoseconds();
    auto frames_submitted_before_update = m_rendering_scheduler_counters.frames_submitted;
    ++m_rendering_scheduler_counters.updates_run;
    bool frame_in_flight = false;
    m_rendering_update_may_overlap = false;
    ScopeGuard const guard = [this, &frame_in_flight, update_start_nanoseconds, frames_submitted_before_update] {
        // The main half ends here, with the submission of the frame if there is one.
        m_rendering_scheduler_counters.main_half_nanoseconds += MonotonicTime::now().nanoseconds() - update_start_nanoseconds;
        if (m_rendering_scheduler_counters.frames_submitted == frames_submitted_before_update)
            did_run_frame_in_lockstep(FrameLockstepReason::JoinsLeft);
        m_running_rendering_task = false;
        // A rendering update whose frame is in flight ends once its tail has run.
        if (!frame_in_flight)
            end_rendering_update();
    };

    {
        MainThreadPhases::Scope phase { MainThreadPhases::Phase::RenderingInput };
        process_input_events();
    }

    // 1. Let frameTimestamp be eventLoop's last render opportunity time.
    auto frame_timestamp = m_last_render_opportunity_time;

    // 2. Let docs be all fully active Document objects whose relevant agent's event loop is
    //    eventLoop, sorted arbitrarily except that the following conditions must be met:
    //    - Any Document B whose container document is A must be listed after A in the list.
    //    - If there are two documents A and B that both have the same non-null container document
    //      C, then the order of A and B in the list must match the shadow-including tree order
    //      of their respective navigable containers in C's node tree.
    // 3. Filter non-renderable documents: Remove from docs any Document object doc for which any of the following are true:
    auto docs = documents_in_this_event_loop_matching([&](auto const& document) {
        if (!document.is_fully_active())
            return false;

        // doc is render-blocked;
        if (document.is_render_blocked()) {
            return false;
        }

        // doc's visibility state is "hidden";
        if (document.hidden())
            return false;

        // doc's rendering is suppressed for view transitions; or
        if (document.rendering_suppression_for_view_transitions())
            return false;

        auto navigable = document.navigable();
        if (!navigable)
            return false;

        // doc's node navigable doesn't currently have a rendering opportunity.
        if (!navigable->has_a_rendering_opportunity())
            return false;

        return true;
    });

    // Everything the render side told each document goes through before the rendering opportunity
    // that reports it can observe it. Animation events, observations and input are ordered by the
    // list, so they keep their place relative to the steps below.
    {
        MainThreadPhases::Scope phase { MainThreadPhases::Phase::RenderingCommitMessages };
        for (auto& document : docs)
            document->apply_commit_messages();
    }

    // FIXME: 4. Unnecessary rendering: Remove from docs any Document object doc for which all of the following are true:

    // FIXME: 5. Remove from docs all Document objects for which the user agent believes that it's preferable to skip updating the rendering for other reasons.

    // FIXME: 6. For each doc of docs, reveal doc.

    auto observable_steps_start_nanoseconds = MonotonicTime::now().nanoseconds();
    Optional<MainThreadPhases::Scope> observable_phase;
    observable_phase.emplace(MainThreadPhases::Phase::RenderingResizeScrollMedia);

    // 7. For each doc of docs, flush autofocus candidates for doc if its node navigable is a top-level traversable.
    for (auto& document : docs) {
        auto navigable = document->navigable();
        if (navigable && navigable->is_top_level_traversable())
            document->flush_autofocus_candidates();
    }

    // 8. For each doc of docs, run the resize steps for doc. [CSSOMVIEW]
    for (auto& document : docs) {
        document->run_the_resize_steps();
    }

    // 9. For each doc of docs, run the scroll steps for doc. [CSSOMVIEW]
    for (auto& document : docs) {
        if (auto navigable = document->navigable()) {
            navigable->process_main_thread_smooth_scrolls();
            navigable->adopt_pending_async_scroll_offsets();
        }
        document->run_the_scroll_steps();
    }

    // 10. For each doc of docs, evaluate media queries and report changes for doc. [CSSOMVIEW]
    for (auto& document : docs) {
        document->evaluate_media_queries_and_report_changes();
    }

    // A clock lease this rendering update cannot tick ends before step 11, which then
    // samples the lease's effects itself.
    m_frame_scheduler->prepare_clock_ticks(docs, frame_timestamp);

    observable_phase.clear();
    observable_phase.emplace(MainThreadPhases::Phase::RenderingAnimations);

    // 11. For each doc of docs, update animations and send events for doc, passing in relative high resolution time given frameTimestamp and doc's relevant global object as the timestamp [WEBANIMATIONS]
    Vector<HighResolutionTime::DOMHighResTimeStamp> animation_timestamps;
    animation_timestamps.ensure_capacity(docs.size());
    for (auto& document : docs) {
        auto timestamp = relative_frame_timestamp_for(frame_timestamp, *document);
        // A document whose animations a render clock ticks shows its last tick.
        if (auto clock_time = m_frame_scheduler->clock_lease_timeline_time(*document); clock_time.has_value())
            timestamp = *clock_time;
        // The ticks a render clock presented beside a long task moved the document
        // timeline past a display frame that went by meanwhile. The timeline does not go
        // back, so the document takes its time, and its animation frame callbacks at step
        // 14 get that time too.
        else if (auto current = document->timeline()->current_time(); current.has_value() && current->type == Animations::TimeValue::Type::Milliseconds && current->value > timestamp)
            timestamp = current->value;
        animation_timestamps.unchecked_append(timestamp);
        document->update_animations_and_send_events(timestamp);
    };

    // 12. For each doc of docs, run the fullscreen steps for doc. [FULLSCREEN]
    for (auto& document : docs) {
        document->run_fullscreen_steps();
    }

    // FIXME: 13. For each doc of docs, if the user agent detects that the backing storage associated with a CanvasRenderingContext2D or an OffscreenCanvasRenderingContext2D, context, has been lost, then it must run the context lost steps for each such context:

    // 14. For each doc of docs, run the animation frame callbacks for doc, passing in the relative high resolution time given frameTimestamp and doc's relevant global object as the timestamp.
    observable_phase.clear();
    observable_phase.emplace(MainThreadPhases::Phase::RenderingAnimationFrameCallbacks);
    for (size_t index = 0; index < docs.size(); ++index)
        run_animation_frame_callbacks(*docs[index], animation_timestamps[index]);
    observable_phase.clear();
    m_rendering_scheduler_counters.observable_steps_nanoseconds += MonotonicTime::now().nanoseconds() - observable_steps_start_nanoseconds;

    // Every animation frame callback of the rendering update has run, and its microtasks with it, so nothing script
    // does before the layout pass can change the decision anymore.
    // NB: The style pass is submitted under the same conditions as the layout pass: a task that runs beside it runs
    //     before the step 16 it belongs to.
    if (Layout::RustFFI::rust_stage_thread_submits()) {
        auto blocker = layout_overlap_blocker_for_rendering_update(docs);
        m_rendering_update_may_overlap = !blocker.has_value();
        if (blocker.has_value())
            ++m_rendering_scheduler_counters.layout_overlap_blocked_updates[to_underlying(*blocker)];
        else
            ++m_rendering_scheduler_counters.layout_overlap_eligible_updates;
    }

    // FIXME: 15. Let unsafeStyleAndLayoutStartTime be the unsafe shared current time.

    Vector<GC::Ref<DOM::Document>> documents;
    documents.ensure_capacity(docs.size());
    for (auto& document : docs)
        documents.unchecked_append(*document);

    // Each leased document's tick samples the effects step 11 left to its lease, beside the
    // main thread, and the rendering update goes on at step 16 once every document has adopted its tick.
    if (m_frame_scheduler->tick_clock_leases(documents, 0, frame_timestamp, m_rendering_update_may_overlap)) {
        frame_in_flight = true;
        return;
    }

    frame_in_flight = run_rendering_update_from_step_16(documents, 0, frame_timestamp, LayoutSubmission::MaySubmit);
}

void EventLoop::resume_rendering_update_after_layout(Badge<FrameScheduler>, Vector<GC::Ref<DOM::Document>> const& docs, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    resume_rendering_update(docs, document_index, frame_timestamp, LayoutSubmission::Wait);
}

void EventLoop::resume_rendering_update_after_style(Badge<FrameScheduler>, Vector<GC::Ref<DOM::Document>> const& docs, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp)
{
    resume_rendering_update(docs, document_index, frame_timestamp, LayoutSubmission::MaySubmitLayout);
}

void EventLoop::resume_rendering_update(Vector<GC::Ref<DOM::Document>> const& docs, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp, LayoutSubmission layout_submission)
{
    VERIFY(!m_running_rendering_task);
    m_running_rendering_task = true;
    bool frame_in_flight = false;
    ScopeGuard const guard = [this, &frame_in_flight] {
        m_running_rendering_task = false;
        // A rendering update whose frame is in flight ends once its tail has run.
        if (!frame_in_flight)
            end_rendering_update();
    };
    frame_in_flight = run_rendering_update_from_step_16(docs, document_index, frame_timestamp, layout_submission);
}

// The steps of a rendering update from step 16 on, from the document at first_document_index. Returns true if a frame is
// in flight, in which case the frame scheduler goes on with the rendering update once it has taken the frame back.
bool EventLoop::run_rendering_update_from_step_16(Vector<GC::Ref<DOM::Document>> const& docs, size_t first_document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp, LayoutSubmission layout_submission)
{
    // 16. For each doc of docs:
    for (size_t document_index = first_document_index; document_index < docs.size(); ++document_index) {
        auto document = docs[document_index];
        MainThreadPhases::Scope step_16_phase { MainThreadPhases::Phase::RenderingStep16 };

        // A rendering update that may overlap its layout lets a document whose layout update runs a full layout pass
        // run the pass beside the main thread, and goes on at this step once the frame scheduler has taken it back.
        // The first style update of such a document runs its first pass beside the main thread the same way, and the
        // rendering update goes on at this step, style finished, once the frame scheduler has taken it back.
        // What was taken back belongs to the document the rendering update went on at: every document after it
        // submits its own passes, as the first did (a child document, such as an app's iframe, included).
        auto document_submission = document_index == first_document_index ? layout_submission : LayoutSubmission::MaySubmit;
        if (document_submission == LayoutSubmission::MaySubmit && m_rendering_update_may_overlap && document->submit_style_for_rendering_update()) {
            m_frame_scheduler->submit_document_pass(docs, document_index, frame_timestamp);
            return true;
        }
        if (document_submission != LayoutSubmission::Wait && m_rendering_update_may_overlap && document->submit_layout_for_rendering_update()) {
            m_frame_scheduler->submit_document_pass(docs, document_index, frame_timestamp);
            return true;
        }

        // 1. Let resizeObserverDepth be 0.
        size_t resize_observer_depth = 0;

        // https://github.com/whatwg/html/pull/11613
        // AD-HOC: Let didRunSnapshotPostLayoutStateSteps be false.
        bool did_run_snapshot_post_layout_state_steps = false;

        // 2. While true:
        while (true) {
            // 1. Recalculate styles and update layout for doc.
            // NOTE: Recalculation of styles is handled by update_layout()
            document->update_layout(DOM::UpdateLayoutReason::HTMLEventLoopRenderingUpdate);

            // AD-HOC: Script that ran earlier in this rendering update may have spun the event loop (e.g. with a
            //         synchronous XHR) and run tasks that stopped document from being actively rendered, for example
            //         by detaching it from its navigable after its iframe was removed. update_layout() is a no-op for
            //         such documents, which can leave them without a paint tree, so skip the rest of this step.
            if (!document->navigable() || document->navigable()->active_document().ptr() != document.ptr())
                break;

            // Clamp viewport scroll offset to valid range after layout, in case the
            // scrollable overflow area has shrunk (e.g. after a viewport size change).
            if (auto navigable = document->navigable()) {
                navigable->clamp_viewport_scroll_offset();

                // AD-HOC: A user scroll gesture that ended while layout was out of date could not select the snap
                //         position it ends at, so it does now.
                navigable->snap_user_scroll_gestures_that_awaited_layout();
            }

            // https://github.com/whatwg/html/pull/11613
            // AD-HOC: If didRunSnapshotPostLayoutStateSteps is false:
            if (!did_run_snapshot_post_layout_state_steps) {
                // 1. Run snapshot post-layout state steps for doc.
                // NB: The state these steps snapshot is what scroll-state() container queries read. The content-visibility
                //     steps below are part of them in that change, but they stay where they are until it lands.
                bool snapshotted_state_changed = document->scroll_state_query_containers().snapshot_post_layout_state(*document, CSS::ScrollStateQueryContainers::Snapshot::AllContainers);

                // 2. Set didRunSnapshotPostLayoutStateSteps to true.
                did_run_snapshot_post_layout_state_steps = true;

                // 3. If any snapshotted state changed, then continue.
                if (snapshotted_state_changed)
                    continue;
            }

            // AD-HOC: Set didRunSnapshotPostLayoutStateSteps to false.
            did_run_snapshot_post_layout_state_steps = false;

            // 2. Let hadInitialVisibleContentVisibilityDetermination be false.
            bool had_initial_visible_content_visibility_determination = false;

            // 3. For each element element with 'auto' used value of 'content-visibility':
            auto* document_element = document->document_element();
            if (document_element) {
                for (auto box_slot : document->paint_state().boxes_with_auto_content_visibility()) {
                    auto box = Painting::committed_box(*document, box_slot);
                    if (!box)
                        continue;
                    auto* element = as_if<DOM::Element>(box.dom_node().ptr());
                    if (!element)
                        continue;

                    // 1. Let checkForInitialDetermination be true if element's proximity to the viewport is not determined and it is not relevant to the user. Otherwise, let checkForInitialDetermination be false.
                    bool check_for_initial_determination = element->proximity_to_the_viewport() == Web::DOM::ProximityToTheViewport::NotDetermined && !element->is_relevant_to_the_user();

                    // 2. Determine proximity to the viewport for element.
                    element->determine_proximity_to_the_viewport();

                    // 3. If checkForInitialDetermination is true and element is now relevant to the user, then set hadInitialVisibleContentVisibilityDetermination to true.
                    if (check_for_initial_determination && element->is_relevant_to_the_user()) {
                        had_initial_visible_content_visibility_determination = true;
                    }
                }
            }

            // 4. If hadInitialVisibleContentVisibilityDetermination is true, then continue.
            if (had_initial_visible_content_visibility_determination)
                continue;

            // 5. Gather active resize observations at depth resizeObserverDepth for doc.
            document->gather_active_observations_at_depth(resize_observer_depth);

            // 6. If doc has active resize observations:
            if (document->has_active_resize_observations()) {
                // 1. Set resizeObserverDepth to the result of broadcasting active resize observations given doc.
                MainThreadPhases::Scope phase { MainThreadPhases::Phase::RenderingResizeObservers };
                resize_observer_depth = document->broadcast_active_resize_observations();

                // 2. Continue.
                continue;
            }

            // 7. Otherwise, break.
            break;
        }

        // 3. If doc has skipped resize observations, then deliver resize loop error given doc.
        if (document->has_skipped_resize_observations()) {
            // FIXME: Deliver resize loop error.
        }

        // https://drafts.csswg.org/scroll-animations-1/#event-loop
        // During step 7.14.1 of the HTML Processing Model, any created scroll progress timelines or view progress
        // timelines are collected into a stale timelines set. After step 7.14 if any timelines' named timeline ranges
        // have changed, these timelines are added to the stale timelines set. If there are any stale timelines they now
        // update their current time and associated ranges, the set of stale timelines is cleared and we run an additional
        // step to recalculate styles and update layout.

        // AD-HOC: This was step 7.14.1 at the time the CSS web-animations spec was written, but it has since been
        //         moved, see https://github.com/w3c/csswg-drafts/issues/12120

        bool requires_style_and_layout_update = false;

        TemporaryExecutionContext context { document->relevant_settings_object() };

        for (auto const& timeline : document->associated_animation_timelines()) {
            auto* scroll_timeline = as_if<Animations::ScrollTimeline>(*timeline);

            if (!scroll_timeline)
                continue;

            if (!scroll_timeline->is_stale())
                continue;

            // NB: The passed timestamp is ignored for ScrollTimelines so we can just use 0.
            timeline->update_current_time(0);
            requires_style_and_layout_update = true;
        }

        if (requires_style_and_layout_update)
            document->update_layout(DOM::UpdateLayoutReason::HTMLEventLoopRenderingUpdate);
    }

    // FIXME: 17. For each doc of docs, if the focused area of doc is not a focusable area, then run the focusing steps for doc's viewport, and set doc's relevant global object's navigation API's focus changed during ongoing navigation to false.

    Optional<MainThreadPhases::Scope> after_step_16_phase;
    after_step_16_phase.emplace(MainThreadPhases::Phase::RenderingIntersectionObservers);

    // 18. For each doc of docs, perform pending transition operations for doc. [CSSVIEWTRANSITIONS]
    for (auto& document : docs) {
        document->perform_pending_transition_operations();
    }

    // 19. For each doc of docs, run the update intersection observations steps for doc, passing in the relative high resolution time given now and doc's relevant global object as the timestamp. [INTERSECTIONOBSERVER]
    for (auto& document : docs) {
        // AD-HOC: Script that ran earlier in this rendering update may have detached document from its navigable, as
        //         in step 16. Its layout and paint state stay behind, but nothing is rendered for it anymore.
        if (!document->navigable() || document->navigable()->active_document().ptr() != document.ptr())
            continue;

        // NB: Layout may have been invalidated by previous steps (e.g. view transitions at step 18).
        //     Re-run layout here since intersection observations need up-to-date geometry.
        document->update_layout(DOM::UpdateLayoutReason::HTMLEventLoopRenderingUpdate);

        auto now = relative_frame_timestamp_for(frame_timestamp, *document);
        document->run_the_update_intersection_observations_steps(now);
    }

    // AD-HOC: Whether a video sink is ticked depends on whether the element would be painted, which is only known
    //         once layout is settled, so it is decided here rather than at the points that invalidate it. A page's
    //         media elements live in any of its documents, so this waits until step 19 has laid out every one of them:
    //         a task that ran while a layout pass was in flight may have left an earlier document's layout stale.
    Vector<GC::Ref<Page>, 1> synced_pages;
    for (auto& document : docs) {
        if (!document->navigable() || document->navigable()->active_document().ptr() != document.ptr())
            continue;
        if (synced_pages.contains_slow(GC::Ref { document->page() }))
            continue;
        synced_pages.append(document->page());
        document->page().sync_media_element_video_sink_ticking();
    }

    // FIXME: 20. For each doc of docs, record rendering time for doc given unsafeStyleAndLayoutStartTime.

    // FIXME: 21. For each doc of docs, mark paint timing for doc.

    // AD-HOC: Flush dirty canvas contexts in documents' pages after callbacks
    // have had a chance to update them, and before painting snapshots the frame.
    for (auto& document : docs)
        document->page().prepare_canvas_contexts_for_compositing();

    after_step_16_phase.clear();
    after_step_16_phase.emplace(MainThreadPhases::Phase::RenderingPaint);

    // 22. For each doc of docs, update the rendering or user interface of doc and its node navigable to reflect the current state.
    for (auto& doc : docs.in_reverse()) {
        // Steps after the layout update above can still mark something for repaint, and the gate
        // below is the first thing to read that.
        doc->drain_invalidation_journal();

        auto navigable = doc->navigable();
        // AD-HOC: Script that ran earlier in this rendering update may have spun the event loop and run tasks that
        //         detached doc from its navigable (e.g. after its iframe was removed).
        if (!navigable || !navigable->needs_repaint())
            continue;
        // OPTIMIZATION: Don't paint navigables hidden by an ancestor iframe with visibility: hidden.
        //               needs_repaint() stays true — so, once the navigable becomes visible, it's painted.
        if (navigable->has_inclusive_ancestor_with_visibility_hidden())
            continue;
        if (navigable->is_svg_page())
            continue;
        if (auto document = navigable->active_document()) {
            document->update_layout(DOM::UpdateLayoutReason::HTMLEventLoopRenderingUpdate);
            if (document->font_computer().should_defer_initial_paint())
                continue;
            // NB: A layout frame that runs beside this thread lets tasks run while the update above does. One of them
            //     may have replaced the navigable's active document, dropped the layout tree the frame built, or
            //     changed what it laid out. The navigable still needs a repaint, and the next rendering update
            //     paints what the task left.
            if (navigable->active_document() != document || !document->has_committed_viewport_box() || !document->layout_is_up_to_date()) {
                navigable->page().client().request_frame();
                continue;
            }
        }
        // A frame the render side records beside this thread is finished by the frame scheduler once it has
        // been, and so is every frame after it, so that frames reach their compositor contexts in paint order.
        TemporaryChange origin { Painting::current_recording_origin(), m_frame_scheduler->recording_origin() };
        auto pending_frame = navigable->begin_painting_next_frame(m_frame_scheduler->recording_run());
        if (!pending_frame.has_value())
            continue;
        if (m_frame_scheduler->ticket_takes_frames() || (pending_frame->recording && pending_frame->recording->run == Painting::RecordingRun::InSubmittedFrame)) {
            m_frame_scheduler->add_to_ticket(*navigable, pending_frame.release_value());
            continue;
        }
        navigable->finish_painting_next_frame(*pending_frame);
        ++m_rendering_scheduler_counters.paints;
        if (navigable->is_local_root())
            navigable->page().process_screenshot_requests();
    }

    // NB: The steps after painting run here, before the frame is submitted, even when the render side records it:
    //     once the main half ends, tasks run beside the frame, and the steps must not see what they change.
    after_step_16_phase.clear();
    after_step_16_phase.emplace(MainThreadPhases::Phase::RenderingFinish);
    finish_rendering_update_steps(docs);

    after_step_16_phase.clear();
    after_step_16_phase.emplace(MainThreadPhases::Phase::RenderingSubmit);
    return m_frame_scheduler->submit();
}

Optional<DOM::LayoutOverlapBlocker> EventLoop::layout_overlap_blocker_for_rendering_update(ReadonlySpan<GC::Root<DOM::Document>> docs) const
{
    // A rendering update run on top of script (EventLoop::pause()) cannot yield to the event loop.
    if (m_running_synchronous_rendering_update)
        return DOM::LayoutOverlapBlocker::SynchronousRenderingUpdate;
    for (auto const& document : docs) {
        if (auto blocker = document->layout_overlap_blocker(); blocker.has_value())
            return blocker;
    }
    return {};
}

void EventLoop::run_rendering_update_tail(Badge<FrameScheduler>, ReadonlySpan<GC::Ref<LocalNavigable>> painted_local_roots)
{
    // 22. (continued) The screenshots of the tab as this frame shows it.
    for (auto navigable : painted_local_roots) {
        if (!navigable->has_been_destroyed())
            navigable->page().process_screenshot_requests();
    }
    end_rendering_update();
}

// Whether the layout of a painted document is up to date, without waiting for its display list recording if that is
// in flight: the recording went in flight only once the layout was, and what was marked beside it since is held.
static bool layout_is_up_to_date_after_painting(DOM::Document& document)
{
    if (!frame_in_flight_holds(document))
        return document.layout_is_up_to_date();
    return document.invalidation_journal().is_empty() && !document.needs_layout_tree_update() && !document.child_needs_layout_tree_update();
}

void EventLoop::finish_rendering_update_steps(ReadonlySpan<GC::Ref<DOM::Document>> docs)
{
    // AD-HOC: Any scroll or layout of a document between a navigable container and its local root moves the rect the
    //         UI process routes input over the container's content navigable by. Report the rects of the containers
    //         in docs, whose layout is up to date along with that of every document above them.
    for (auto* container : NavigableContainer::all_instances()) {
        if (any_of(docs, [&](auto const& document) { return document.ptr() == &container->document(); }))
            container->report_content_navigable_viewport_rect();
    }

    // 23. For each doc of docs, process top layer removals given doc.
    for (auto& document : docs) {
        document->process_top_layer_removals();
    }

    for (auto& document : docs) {
        // https://drafts.csswg.org/css-font-loading/#fontfaceset-pending-on-the-environment
        // A FontFaceSet is pending on the environment if any of the following are true:
        // - the document is still loading
        // - the document has pending stylesheet requests
        // - the document has pending layout operations which might cause the user agent to request a font, or which depend on recently-loaded fonts
        TemporaryExecutionContext context(document->relevant_settings_object(), TemporaryExecutionContext::CallbacksEnabled::Yes);
        document->fonts()->set_is_pending_on_the_environment(document->readiness() == DocumentReadyState::Loading
            || document->has_pending_style_sheet_requests()
            || !layout_is_up_to_date_after_painting(*document));
    }
}

void EventLoop::end_rendering_update()
{
    auto update_start_time = m_rendering_update_start_time;
    auto update_end_time = HighResolutionTime::unsafe_shared_current_time();
    m_rendering_scheduler_counters.update_microseconds += static_cast<u64>((update_end_time - update_start_time) * 1000.0);
    auto& by_frames_submitted = m_rendering_scheduler_counters.rendering_updates_by_frames_submitted;
    ++by_frames_submitted[min(m_frames_submitted_by_rendering_update, by_frames_submitted.size() - 1)];
    m_frames_submitted_by_rendering_update = 0;

    for (auto const& page : pages_of_local_roots())
        page->client().did_finish_rendering_update();

    // The next rendering update of a document that has nothing but its animations to show
    // ticks them under a clock lease.
    m_frame_scheduler->grant_clock_leases();

    auto const& current = m_rendering_scheduler_counters;
    auto const& previous = m_rendering_scheduler_counters_at_last_update;
    [[maybe_unused]] auto lockstep_frames = [](RenderingSchedulerCounters const& counters) {
        u64 frames = 0;
        for (auto count : counters.frames_lockstep)
            frames += count;
        return frames;
    };
    dbgln_if(RENDERING_SCHEDULER_DEBUG,
        "[RenderSched] update #{} duration={:.1f}ms gap={:.1f}ms paints={} tasks={} ({:.1f}ms) "
        "[postmsg {} ({:.1f}ms), timer {} ({:.1f}ms), net {} ({:.1f}ms), dom {} ({:.1f}ms)] "
        "requests={} coalesced={} during_update={} "
        "frames submitted={} consumed={} lockstep={} dropped={} in_flight={:.1f}ms overlap_tasks={:.1f}ms main_half={:.1f}ms",
        current.updates_run, update_end_time - update_start_time,
        m_last_rendering_update_end_time > 0 ? update_start_time - m_last_rendering_update_end_time : 0.0,
        current.paints - previous.paints,
        current.tasks_between_updates - previous.tasks_between_updates,
        static_cast<double>(current.task_microseconds_between_updates - previous.task_microseconds_between_updates) / 1000.0,
        current.posted_message_tasks_between_updates - previous.posted_message_tasks_between_updates,
        static_cast<double>(current.posted_message_task_microseconds_between_updates - previous.posted_message_task_microseconds_between_updates) / 1000.0,
        current.timer_tasks_between_updates - previous.timer_tasks_between_updates,
        static_cast<double>(current.timer_task_microseconds_between_updates - previous.timer_task_microseconds_between_updates) / 1000.0,
        current.networking_tasks_between_updates - previous.networking_tasks_between_updates,
        static_cast<double>(current.networking_task_microseconds_between_updates - previous.networking_task_microseconds_between_updates) / 1000.0,
        current.dom_manipulation_tasks_between_updates - previous.dom_manipulation_tasks_between_updates,
        static_cast<double>(current.dom_manipulation_task_microseconds_between_updates - previous.dom_manipulation_task_microseconds_between_updates) / 1000.0,
        current.update_requests - previous.update_requests,
        current.coalesced_update_requests - previous.coalesced_update_requests,
        current.update_requests_while_rendering - previous.update_requests_while_rendering,
        current.frames_submitted - previous.frames_submitted,
        current.frames_consumed - previous.frames_consumed,
        lockstep_frames(current) - lockstep_frames(previous),
        current.frames_dropped - previous.frames_dropped,
        static_cast<double>(current.frame_in_flight_nanoseconds - previous.frame_in_flight_nanoseconds) / 1'000'000.0,
        static_cast<double>(current.overlap_task_nanoseconds - previous.overlap_task_nanoseconds) / 1'000'000.0,
        static_cast<double>(current.main_half_nanoseconds - previous.main_half_nanoseconds) / 1'000'000.0);
    m_rendering_scheduler_counters_at_last_update = current;
    m_last_rendering_update_end_time = update_end_time;
}

void run_when_event_loop_reaches_step_1(GC::Ref<GC::Function<void()>> steps)
{
    auto& event_loop = main_thread_event_loop();
    event_loop.run_upon_reaching_step_1(steps);
}

// https://html.spec.whatwg.org/multipage/webappapis.html#queue-a-task
TaskID queue_a_task(HTML::Task::Source source, GC::Ptr<EventLoop> event_loop, GC::Ptr<DOM::Document> document, GC::Ref<GC::Function<void()>> steps, Task::Priority priority)
{
    // 1. If event loop was not given, set event loop to the implied event loop.
    if (!event_loop)
        event_loop = main_thread_event_loop();

    // FIXME: 2. If document was not given, set document to the implied document.

    // 3. Let task be a new task.
    // 4. Set task's steps to steps.
    // 5. Set task's source to source.
    // 6. Set task's document to the document.
    // 7. Set task's script evaluation environment settings object set to an empty set.
    auto task = HTML::Task::create(source, document, steps, priority);

    // 8. Let queue be the task queue to which source is associated on event loop.
    // 9. Append task to queue.
    if (source == HTML::Task::Source::Microtask)
        event_loop->enqueue_microtask(task);
    else
        event_loop->task_queue().add(task);

    return task->id();
}

// https://html.spec.whatwg.org/multipage/webappapis.html#queue-a-global-task
TaskID queue_global_task(HTML::Task::Source source, JS::Object& global_object, GC::Ref<GC::Function<void()>> steps, Task::Priority priority)
{
    // 1. Let event loop be global's relevant agent's event loop.
    auto& event_loop = relevant_agent(global_object).event_loop;

    // 2. Let document be global's associated Document, if global is a Window object; otherwise null.
    DOM::Document* document { nullptr };
    if (auto* window_object = window_from_global_object(global_object))
        document = &window_object->associated_document();

    // 3. Queue a task given source, event loop, document, and steps.
    return queue_a_task(source, *event_loop, document, steps, priority);
}

// https://html.spec.whatwg.org/multipage/webappapis.html#queue-a-microtask
void queue_a_microtask(GC::Ptr<DOM::Document const> document, GC::Ref<GC::Function<void()>> steps)
{
    // 1. If event loop was not given, set event loop to the implied event loop.
    auto& event_loop = HTML::main_thread_event_loop();

    // FIXME: 2. If document was not given, set document to the implied document.

    // 3. Let microtask be a new task.
    // 4. Set microtask's steps to steps.
    // 5. Set microtask's source to the microtask task source.
    // 6. Set microtask's document to document.
    auto microtask = HTML::Task::create(HTML::Task::Source::Microtask, document, steps);

    // FIXME: 7. Set microtask's script evaluation environment settings object set to an empty set.

    // 8. Enqueue microtask on event loop's microtask queue.
    event_loop.enqueue_microtask(microtask);
}

void perform_a_microtask_checkpoint()
{
    main_thread_event_loop().perform_a_microtask_checkpoint();
}

// https://html.spec.whatwg.org/multipage/webappapis.html#perform-a-microtask-checkpoint
void EventLoop::perform_a_microtask_checkpoint()
{
    if (execution_paused())
        return;

    // 1. If the event loop's performing a microtask checkpoint is true, then return.
    if (m_performing_a_microtask_checkpoint)
        return;

    // NOTE: This assertion is per requirement 9.5 of the ECMA-262 spec, see: https://tc39.es/ecma262/#sec-jobs
    // > At some future point in time, when there is no running context in the agent for which the job is scheduled and that agent's execution context stack is empty...
    VERIFY(vm().execution_context_stack().is_empty());
    VERIFY(!vm().has_running_execution_context());
    MainThreadPhases::Scope phase { MainThreadPhases::Phase::Microtasks };

    // 2. Set the event loop's performing a microtask checkpoint to true.
    m_performing_a_microtask_checkpoint = true;

    // 3. While the event loop's microtask queue is not empty:
    while (!m_microtask_queue.is_empty()) {
        // 1. Let oldestMicrotask be the result of dequeuing from the event loop's microtask queue.
        auto oldest_microtask = m_microtask_queue.dequeue();

        // 2. Set the event loop's currently running task to oldestMicrotask.
        m_currently_running_task = oldest_microtask;

        // 3. Run oldestMicrotask.
        oldest_microtask->execute();

        // 4. Set the event loop's currently running task back to null.
        m_currently_running_task = nullptr;
    }

    // 4. For each environment settings object settingsObject whose responsible event loop is this event loop, notify about rejected promises given settingsObject's global object.
    auto environments = GC::RootVector { m_related_environment_settings_objects };
    for (auto& environment_settings_object : environments) {
        auto& global_object = environment_settings_object->global_object();
        if (auto* worklet_global_scope = Bindings::impl_from<WorkletGlobalScope>(&global_object)) {
            worklet_global_scope->notify_about_rejected_promises({});
            continue;
        }
        relevant_window_or_worker_global_scope(global_object).notify_about_rejected_promises({});
    }

    // 5. Cleanup Indexed Database transactions.
    IndexedDB::cleanup_indexed_database_transactions(*this);

    // 6. Perform ClearKeptObjects().
    vm().finish_execution_generation();

    // 7. Set the event loop's performing a microtask checkpoint to false.
    m_performing_a_microtask_checkpoint = false;

    // FIXME: 8. Record timing info for microtask checkpoint.
}

Vector<GC::Root<DOM::Document>> EventLoop::documents_in_this_event_loop_matching(Function<bool(DOM::Document&)> callback) const
{
    ensure_documents_sorted();
    Vector<GC::Root<DOM::Document>> documents;
    for (auto& document : m_documents) {
        VERIFY(document);
        if (document->is_decoded_svg())
            continue;
        if (!callback(*document))
            continue;
        documents.append(GC::make_root(*document));
    }
    return documents;
}

void EventLoop::register_document(Badge<DOM::Document>, DOM::Document& document)
{
    m_documents.append(&document);
    m_documents_sort_dirty = true;
}

void EventLoop::unregister_document(Badge<DOM::Document>, DOM::Document& document)
{
    bool did_remove = m_documents.remove_first_matching([&](auto& entry) { return entry.ptr().ptr() == &document; });
    VERIFY(did_remove);
}

void EventLoop::document_navigable_did_change(Badge<DOM::Document>)
{
    m_documents_sort_dirty = true;
}

void EventLoop::ensure_documents_sorted() const
{
    // https://html.spec.whatwg.org/multipage/webappapis.html#update-the-rendering step 3.2:
    // - Any Document B whose container document is A must be listed after A in the list.
    // - If there are two documents A and B that both have the same non-null container document
    //   C, then the order of A and B in the list must match the shadow-including tree order
    //   of their respective navigable containers in C's node tree.

    if (!m_documents_sort_dirty)
        return;
    m_documents_sort_dirty = false;

    HashMap<DOM::Document*, size_t> doc_to_index;
    doc_to_index.ensure_capacity(m_documents.size());
    for (size_t i = 0; i < m_documents.size(); ++i)
        doc_to_index.set(m_documents[i].ptr().ptr(), i);

    Vector<bool> visited;
    visited.resize(m_documents.size());
    Vector<GC::Weak<DOM::Document>> sorted;
    sorted.ensure_capacity(m_documents.size());

    auto visit = [&](auto& self, size_t idx) -> void {
        if (visited[idx])
            return;
        visited[idx] = true;
        if (auto navigable = m_documents[idx]->navigable()) {
            if (auto container_doc = navigable->container_document()) {
                if (auto container_idx = doc_to_index.get(container_doc.ptr()); container_idx.has_value())
                    self(self, *container_idx);
            }
        }
        sorted.append(m_documents[idx]);
    };
    for (size_t i = 0; i < m_documents.size(); ++i)
        visit(visit, i);

    m_documents = move(sorted);
}

void EventLoop::push_onto_backup_incumbent_realm_stack(GC::Ref<EnvironmentSettingsObject> environment_settings_object)
{
    m_backup_incumbent_realm_stack.append(environment_settings_object);
}

void EventLoop::pop_backup_incumbent_realm_stack()
{
    m_backup_incumbent_realm_stack.take_last();
}

EnvironmentSettingsObject& EventLoop::top_of_backup_incumbent_realm_stack()
{
    return m_backup_incumbent_realm_stack.last();
}

void EventLoop::register_environment_settings_object(Badge<EnvironmentSettingsObject>, EnvironmentSettingsObject& environment_settings_object)
{
    m_related_environment_settings_objects.append(&environment_settings_object);
}

void EventLoop::unregister_environment_settings_object(Badge<EnvironmentSettingsObject>, EnvironmentSettingsObject& environment_settings_object)
{
    bool did_remove = m_related_environment_settings_objects.remove_first_matching([&](auto& entry) { return entry == &environment_settings_object; });
    VERIFY(did_remove);
}

// https://html.spec.whatwg.org/multipage/webappapis.html#same-loop-windows
Vector<GC::Root<HTML::Window>> EventLoop::same_loop_windows() const
{
    Vector<GC::Root<HTML::Window>> windows;
    for (auto& document : documents_in_this_event_loop_matching([](auto& document) { return document.is_fully_active(); })) {
        windows.append(GC::make_root(document->window()));
    }
    return windows;
}

// https://html.spec.whatwg.org/multipage/webappapis.html#event-loop-processing-model:last-idle-period-start-time
double EventLoop::compute_deadline() const
{
    // 1. Let deadline be this event loop's last idle period start time plus 50.
    auto deadline = m_last_idle_period_start_time + 50;
    // 2. Let hasPendingRenders be false.
    auto has_pending_renders = false;
    // 3. For each windowInSameLoop of the same-loop windows for this event loop:
    for (auto& window : same_loop_windows()) {
        // 1. If windowInSameLoop's map of animation frame callbacks is not empty,
        //    or if the user agent believes that the windowInSameLoop might have pending rendering updates,
        //    set hasPendingRenders to true.
        if (window->has_animation_frame_callbacks())
            has_pending_renders = true;
        // FIXME: 2. Let timerCallbackEstimates be the result of getting the values of windowInSameLoop's map of active timers.
        // FIXME: 3. For each timeoutDeadline of timerCallbackEstimates, if timeoutDeadline is less than deadline, set deadline to timeoutDeadline.
    }
    // 4. If hasPendingRenders is true, then:
    if (has_pending_renders) {
        // 1. Let nextRenderDeadline be this event loop's last render opportunity time plus (1000 divided by the current refresh rate).
        // FIXME: Hardcoded to 60Hz
        auto next_render_deadline = m_last_render_opportunity_time + (1000.0 / 60.0);
        // 2. If nextRenderDeadline is less than deadline, then return nextRenderDeadline.
        if (next_render_deadline < deadline)
            return next_render_deadline;
    }
    // 5. Return deadline.
    return deadline;
}

EventLoop::PauseHandle::PauseHandle(EventLoop& event_loop, JS::Object const& global, HighResolutionTime::DOMHighResTimeStamp time_before_pause)
    : event_loop(event_loop)
    , global(global)
    , time_before_pause(time_before_pause)
{
}

EventLoop::PauseHandle::~PauseHandle()
{
    event_loop->unpause({}, *global, time_before_pause);
}

// https://html.spec.whatwg.org/multipage/webappapis.html#pause
EventLoop::PauseHandle EventLoop::pause(UpdateTheRendering should_update_the_rendering)
{
    ++m_execution_pause_depth;

    // 1. Let global be the current global object.
    auto& global = current_global_object();

    // 2. Let timeBeforePause be the current high resolution time given global.
    auto time_before_pause = HighResolutionTime::current_high_resolution_time(global);

    // 3. If necessary, update the rendering or user interface of any Document or navigable to reflect the current state.
    // NB: UpdateTheRendering::No skips this step, for a caller that must not run author callbacks (e.g., rAF callbacks)
    //     while it's blocked — a sync XHR send(), which may itself have been invoked from within a microtask.
    if (should_update_the_rendering == UpdateTheRendering::Yes && !m_running_rendering_task) {
        // The previous rendering update's tail first, so the rendering task it can queue is removed with the others.
        m_frame_scheduler->finish_frame_now();
        if (m_rendering_task_queued) {
            m_task_queue->remove_tasks_matching([](auto const& task) {
                return task.source() == Task::Source::Rendering;
            });
            m_rendering_task_queued = false;
            m_rendering_task_held = false;
            m_rendering_task_runs_ahead = false;
        }
        m_last_render_opportunity_time = max(m_last_render_opportunity_time, HighResolutionTime::unsafe_shared_current_time());
        TemporaryChange synchronous_rendering_update { m_running_synchronous_rendering_update, true };
        update_the_rendering();
    }

    // 4. Wait until the condition goal is met. While a user agent has a paused task, the corresponding event loop must
    //    not run further tasks, and any script in the currently running task must block. User agents should remain
    //    responsive to user input while paused, however, albeit in a reduced capacity since the event loop will not be
    //    doing anything.

    return PauseHandle { *this, global, time_before_pause };
}

void EventLoop::unpause(Badge<PauseHandle>, JS::Object const& global, HighResolutionTime::DOMHighResTimeStamp time_before_pause)
{
    VERIFY(m_execution_pause_depth > 0);
    --m_execution_pause_depth;
    // Also picks up a frame that finished while paused, and a tail the pause held back.
    if (m_execution_pause_depth == 0)
        schedule();

    // FIXME: 5. Record pause duration given the duration from timeBeforePause to the current high resolution time given global.
    [[maybe_unused]] auto pause_duration = HighResolutionTime::current_high_resolution_time(global) - time_before_pause;
}

}
