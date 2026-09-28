/*
 * Copyright (c) 2021, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Array.h>
#include <AK/Badge.h>
#include <AK/Function.h>
#include <AK/Noncopyable.h>
#include <AK/NonnullOwnPtr.h>
#include <AK/Queue.h>
#include <LibCore/Forward.h>
#include <LibGC/Ptr.h>
#include <LibGC/Weak.h>
#include <LibJS/Forward.h>
#include <LibWeb/DOM/LayoutOverlapBlocker.h>
#include <LibWeb/Export.h>
#include <LibWeb/HTML/EventLoop/TaskQueue.h>
#include <LibWeb/Painting/RecordingOrigin.h>
#include <LibWebCommon/HighResolutionTime/DOMHighResTimeStamp.h>

namespace Web::HTML {

class FrameScheduler;

class WEB_API EventLoop : public JS::Cell {
    GC_CELL(EventLoop, JS::Cell);
    GC_DECLARE_ALLOCATOR(EventLoop);

    struct WEB_API PauseHandle {
        PauseHandle(EventLoop&, JS::Object const& global, HighResolutionTime::DOMHighResTimeStamp);
        ~PauseHandle();

        AK_MAKE_NONCOPYABLE(PauseHandle);
        AK_MAKE_NONMOVABLE(PauseHandle);

        GC::Ref<EventLoop> event_loop;
        GC::Ref<JS::Object const> global;
        HighResolutionTime::DOMHighResTimeStamp const time_before_pause;
    };

public:
    // Why a frame ran while main waited for it rather than beside main.
    enum class FrameLockstepReason : u8 {
        // A stage of the frame still has joins, so main has to serve them.
        JoinsLeft,
        // A document with resize observations must deliver them within the rendering update.
        ResizeObserverDocument,
        ViewTransition,
        // A read of render state outside the rendering update, which cannot return before the frame does.
        SynchronousCaller,
        Count,
    };
    static StringView frame_lockstep_reason_name(FrameLockstepReason);

    // What an invalidation journal entry is about, for counting the ones made while a frame is in flight.
    enum class JournalEntryKind : u8 {
        LayoutUpdate,
        LayoutTreeUpdate,
        Repaint,
        PaintFacts,
        PaintCache,
        Editability,
        Selection,
        ScrollOffset,
        Scrollbar,
        TextData,
        SVGAttributes,
        TableSpans,
        VisualContext,
        Count,
    };
    static StringView journal_entry_kind_name(JournalEntryKind);

    struct RenderingSchedulerCounters {
        u64 update_requests { 0 };
        u64 coalesced_update_requests { 0 };
        u64 update_requests_while_rendering { 0 };
        u64 opportunities_received { 0 };
        u64 opportunities_that_queued_a_task { 0 };
        u64 watchdog_opportunities { 0 };
        u64 updates_run { 0 };
        u64 updates_skipped_as_unnecessary { 0 };
        u64 update_microseconds { 0 };
        u64 tasks_between_updates { 0 };
        u64 task_microseconds_between_updates { 0 };
        u64 posted_message_tasks_between_updates { 0 };
        u64 posted_message_task_microseconds_between_updates { 0 };
        u64 timer_tasks_between_updates { 0 };
        u64 timer_task_microseconds_between_updates { 0 };
        u64 networking_tasks_between_updates { 0 };
        u64 networking_task_microseconds_between_updates { 0 };
        u64 dom_manipulation_tasks_between_updates { 0 };
        u64 dom_manipulation_task_microseconds_between_updates { 0 };
        u64 paints { 0 };

        // What the frames of the rendering updates cost the main thread, and what ran beside them. A frame is
        // "submitted" when it leaves main to run on its own and "consumed" when main takes its result back; one that
        // runs while main waits for it is "lockstep". A rendering update whose frame the frame scheduler does not submit
        // is a lockstep frame.
        u64 frames_submitted { 0 };
        u64 frames_consumed { 0 };
        Array<u64, to_underlying(FrameLockstepReason::Count)> frames_lockstep {};
        // Frames whose result was thrown away instead of consumed. Must stay 0.
        u64 frames_dropped { 0 };
        u64 tasks_started_with_frame_in_flight { 0 };
        u64 frame_in_flight_nanoseconds { 0 };
        // Task and microtask time main spent while a frame was in flight: the work overlap exists for.
        u64 overlap_task_nanoseconds { 0 };
        // Main's own share of a rendering update: up to the submission, then the two halves of consuming the frame.
        // A lockstep frame's main half is the whole update.
        u64 main_half_nanoseconds { 0 };
        // The part of the main half that runs the script-observable steps before style and layout (7 to 14: autofocus,
        // resize, scroll, media queries, animations, fullscreen and animation frame callbacks).
        u64 observable_steps_nanoseconds { 0 };
        u64 consume_commit_nanoseconds { 0 };
        u64 consume_tail_nanoseconds { 0 };
        u64 submit_to_consume_nanoseconds { 0 };
        u64 max_submit_to_consume_nanoseconds { 0 };
        // What the DOM side journaled while a frame was in flight, which it could do without waiting for the frame.
        Array<u64, to_underlying(JournalEntryKind::Count)> journal_entries_during_flight {};
        u64 finished_frame_consumer_calls { 0 };
        // How long rendering tasks waited for the render side to finish the previous rendering update's frame before
        // they could start (finish_frame_now()), and how many held their rendering opportunity instead of waiting.
        u64 rendering_task_blocked_on_frame_nanoseconds { 0 };
        u64 rendering_tasks_held { 0 };
        // Rendering tasks that ran ahead of tasks queued before them.
        u64 rendering_tasks_ahead_of_queue { 0 };
        // Rendering updates whose layout the render side could lay out beside main, and the ones it could not, by the
        // first thing that kept them in place. Counted only where the render side submits layout.
        u64 layout_overlap_eligible_updates { 0 };
        Array<u64, DOM::layout_overlap_blocker_count> layout_overlap_blocked_updates {};
        // Rendering updates by how many frames they submitted (their hops to the render side): none, 1, 2, 3, and 4 or
        // more.
        Array<u64, 5> rendering_updates_by_frames_submitted {};
        // Display list recordings the main thread waited for rather than submitting them, by what asked for them.
        Array<u64, to_underlying(Painting::RecordingOrigin::Count)> recordings_waited_for {};
        Array<u64, to_underlying(Painting::RecordingOrigin::Count)> recording_wait_nanoseconds {};
        // Flights the main thread sealed paint for, and the ones it did not, by why.
        u64 flight_paint_seals { 0 };
        Array<u64, to_underlying(Painting::FlightPaintDecline::Count)> flight_paint_declines {};
    };

    enum class Type {
        // https://html.spec.whatwg.org/multipage/webappapis.html#window-event-loop
        Window,
        // https://html.spec.whatwg.org/multipage/webappapis.html#worker-event-loop
        Worker,
        // https://html.spec.whatwg.org/multipage/webappapis.html#worklet-event-loop
        Worklet,
    };

    virtual ~EventLoop() override;

    Type type() const { return m_type; }

    void run_upon_reaching_step_1(GC::Ref<GC::Function<void()>> task) { m_reached_step_1_tasks.append(task); }

    TaskQueue& task_queue() { return *m_task_queue; }
    TaskQueue const& task_queue() const { return *m_task_queue; }

    bool microtask_queue_empty() const { return m_microtask_queue.is_empty(); }
    void enqueue_microtask(GC::Ref<HTML::Task> task) { m_microtask_queue.enqueue(task); }
    GC::Ref<HTML::Task> dequeue_microtask() { return m_microtask_queue.dequeue(); }

    void spin_until(GC::Ref<GC::Function<bool()>> goal_condition);
    void process();
    void request_rendering_update();
    enum class RenderingOpportunitySource {
        Compositor,
        LocalTimer,
        Watchdog,
        Manual,
    };
    bool rendering_opportunity(HighResolutionTime::DOMHighResTimeStamp frame_time, RenderingOpportunitySource);
    bool rendering_task_queued_or_running() const { return m_rendering_task_queued || m_running_rendering_task; }

    // Whether the rendering task found the previous rendering update's frame still in flight and holds its rendering
    // opportunity instead of waiting for the frame. The held rendering update runs at the step 1 that takes that frame
    // in and runs its tail, ahead of the tasks queued meanwhile.
    bool rendering_task_held() const { return m_rendering_task_held; }
    // Whether the queued rendering task runs before the other tasks queued ahead of it. Asked by the task queue. A held
    // rendering task does once it is queued again, and so does one that has waited a frame interval behind other tasks,
    // but not twice without another task in between.
    bool rendering_task_runs_ahead_of_queue() const;
    void did_run_rendering_task_ahead_of_queue();
    bool running_synchronous_rendering_update() const { return m_running_synchronous_rendering_update; }

    // Whether the layout of the running rendering update may run beside the main thread, decided once all of its
    // animation frame callbacks and their microtasks have run (step 14).
    // What keeps the layout of a rendering update over docs in place, if anything. The whole rendering update decides
    // together: once one document's layout has been submitted, tasks run before every later document's step 16.
    [[nodiscard]] Optional<DOM::LayoutOverlapBlocker> layout_overlap_blocker_for_rendering_update(ReadonlySpan<GC::Root<DOM::Document>> docs) const;

    // https://html.spec.whatwg.org/multipage/browsing-the-web.html#termination-nesting-level
    size_t termination_nesting_level() const { return m_termination_nesting_level; }
    void increment_termination_nesting_level() { ++m_termination_nesting_level; }
    void decrement_termination_nesting_level() { --m_termination_nesting_level; }

    GC::Ptr<Task const> currently_running_task() const { return m_currently_running_task; }

    u64 task_generation() const { return m_task_generation; }

    void schedule();

    void perform_a_microtask_checkpoint();

    void register_document(Badge<DOM::Document>, DOM::Document&);
    void unregister_document(Badge<DOM::Document>, DOM::Document&);
    void document_navigable_did_change(Badge<DOM::Document>);

    [[nodiscard]] Vector<GC::Root<DOM::Document>> documents_in_this_event_loop_matching(Function<bool(DOM::Document&)> callback) const;

    Vector<GC::Root<HTML::Window>> same_loop_windows() const;

    void push_onto_backup_incumbent_realm_stack(GC::Ref<EnvironmentSettingsObject>);
    void pop_backup_incumbent_realm_stack();
    EnvironmentSettingsObject& top_of_backup_incumbent_realm_stack();
    bool is_backup_incumbent_realm_stack_empty() const { return m_backup_incumbent_realm_stack.is_empty(); }

    void register_environment_settings_object(Badge<EnvironmentSettingsObject>, EnvironmentSettingsObject&);
    void unregister_environment_settings_object(Badge<EnvironmentSettingsObject>, EnvironmentSettingsObject&);

    double compute_deadline() const;

    enum class UpdateTheRendering {
        No,
        Yes,
    };

    [[nodiscard]] PauseHandle pause(UpdateTheRendering = UpdateTheRendering::Yes);
    void unpause(Badge<PauseHandle>, JS::Object const& global, HighResolutionTime::DOMHighResTimeStamp);
    bool execution_paused() const { return m_execution_pause_depth > 0; }

    bool running_rendering_task() const { return m_running_rendering_task; }

    // A frame finishes beside the document thread and is consumed at step 1 of the processing model. The consumer is called there whenever FrameCompletion::the() has a completion pending, or it
    // asked to be called again, no matter whether any page is visible or has a rendering opportunity. Setting it
    // registers this event loop for completions, so one posted earlier is delivered now.
    void set_finished_frame_consumer(GC::Ptr<GC::Function<void()>>);
    // The consumer left work for a later step 1 (e.g. a tail a nested loop may not run). Processing is scheduled
    // again once the nested loop or pause that held it back ends.
    void call_finished_frame_consumer_again() { m_finished_frame_consumer_call_requested = true; }
    bool has_finished_frame_work() const;

    // How deeply the processing model is nested, and what a finished frame may do there. A spin of the event loop
    // empties the JavaScript execution context stack, so its tasks run above a suspended caller with an empty stack;
    // a pause (dialogs, synchronous XHR) runs no tasks at all. Consume-commit is script-free, so a forced join may run
    // it anywhere and step 1 anywhere but inside the rendering task that owns the frame. A consume-tail runs script
    // (resize observers, screenshots, intersection observer tasks), so only the outermost step 1 runs it.
    size_t processing_depth() const { return m_processing_depth; }
    size_t spin_depth() const { return m_spin_depth; }
    enum class FrameConsumeSite {
        StepOne,
        ForcedJoin,
    };
    bool may_consume_commit(FrameConsumeSite) const;
    bool may_run_consume_tail() const;
    bool consuming_frame() const { return m_consuming_frame_commit || m_running_consume_tail; }
    void consume_commit(FrameConsumeSite, Function<void()> const&);
    void run_consume_tail(Function<void()> const&);

    RenderingSchedulerCounters const& rendering_scheduler_counters() const { return m_rendering_scheduler_counters; }
    void reset_rendering_scheduler_counters();

    // The frame scheduler's hook points for the counters above. A frame is in flight from did_submit_frame() until
    // did_consume_frame_commit(); did_consume_frame_tail() follows once the rest of the rendering update has run.
    void did_submit_frame();
    void did_consume_frame_commit(u64 nanoseconds);
    void did_consume_frame_tail(u64 nanoseconds);
    void did_drop_frame() { ++m_rendering_scheduler_counters.frames_dropped; }
    void did_run_frame_in_lockstep(FrameLockstepReason reason) { ++m_rendering_scheduler_counters.frames_lockstep[to_underlying(reason)]; }
    void did_seal_flight_paint(Optional<Painting::FlightPaintDecline> decline)
    {
        if (decline.has_value())
            ++m_rendering_scheduler_counters.flight_paint_declines[to_underlying(*decline)];
        else
            ++m_rendering_scheduler_counters.flight_paint_seals;
    }
    void did_wait_for_recording(Painting::RecordingOrigin origin, i64 nanoseconds)
    {
        ++m_rendering_scheduler_counters.recordings_waited_for[to_underlying(origin)];
        m_rendering_scheduler_counters.recording_wait_nanoseconds[to_underlying(origin)] += nanoseconds;
    }
    // Main-thread only, and cheap enough to ask on every journal write.
    static bool a_frame_is_in_flight() { return s_a_frame_is_in_flight; }
    void note_journal_entry_during_flight(JournalEntryKind kind) { ++m_rendering_scheduler_counters.journal_entries_during_flight[to_underlying(kind)]; }

    FrameScheduler& frame_scheduler() { return *m_frame_scheduler; }
    void note_frame_painted(Badge<FrameScheduler>) { ++m_rendering_scheduler_counters.paints; }
    // The steps of a rendering update after its frame: the screenshots of the frame and the end of the update.
    void run_rendering_update_tail(Badge<FrameScheduler>, ReadonlySpan<GC::Ref<LocalNavigable>> painted_local_roots);
    // Goes on with the rendering update whose layout pass the frame scheduler has taken back, at step 16 for the document
    // the pass laid out.
    void resume_rendering_update_after_layout(Badge<FrameScheduler>, Vector<GC::Ref<DOM::Document>> const& docs, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp);
    // Goes on with the rendering update whose first style pass the frame scheduler has taken back and whose style update
    // it has finished, at step 16 for the document the pass styled. Its layout pass may still be submitted.
    void resume_rendering_update_after_style(Badge<FrameScheduler>, Vector<GC::Ref<DOM::Document>> const& docs, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp);

private:
    explicit EventLoop(Type);

    virtual void visit_edges(Visitor&) override;

    void process_input_events() const;
    void update_the_rendering();
    enum class LayoutSubmission : u8 {
        Wait,
        // The style update has run; the layout pass may still be submitted.
        MaySubmitLayout,
        // The first style pass or the layout pass may be submitted.
        MaySubmit,
    };
    void resume_rendering_update(Vector<GC::Ref<DOM::Document>> const& docs, size_t document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp, LayoutSubmission);
    bool run_rendering_update_from_step_16(Vector<GC::Ref<DOM::Document>> const& docs, size_t first_document_index, HighResolutionTime::DOMHighResTimeStamp frame_timestamp, LayoutSubmission);
    void finish_rendering_update_steps(ReadonlySpan<GC::Ref<DOM::Document>> docs);
    void end_rendering_update();
    void run_rendering_task();
    void queue_held_rendering_task_if_frame_finished();

    Type m_type { Type::Window };

    Vector<GC::Ref<GC::Function<void()>>> m_reached_step_1_tasks;

    GC::Ptr<TaskQueue> m_task_queue;
    Queue<GC::Ref<HTML::Task>> m_microtask_queue;

    // https://html.spec.whatwg.org/multipage/webappapis.html#currently-running-task
    GC::Ptr<Task> m_currently_running_task { nullptr };

    u64 m_task_generation { 0 };

    // https://html.spec.whatwg.org/multipage/webappapis.html#last-render-opportunity-time
    double m_last_render_opportunity_time { 0 };
    RenderingOpportunitySource m_last_render_opportunity_source { RenderingOpportunitySource::Compositor };
    // https://html.spec.whatwg.org/multipage/webappapis.html#last-idle-period-start-time
    double m_last_idle_period_start_time { 0 };

    GC::Ptr<Platform::Timer> m_system_event_loop_timer;
    GC::Ptr<Platform::Timer> m_idle_period_timer;

    // https://html.spec.whatwg.org/multipage/webappapis.html#performing-a-microtask-checkpoint
    bool m_performing_a_microtask_checkpoint { false };

    mutable Vector<GC::Weak<DOM::Document>> m_documents;
    mutable bool m_documents_sort_dirty { false };
    void ensure_documents_sorted() const;

    // Used to implement step 4 of "perform a microtask checkpoint".
    // NOTE: These are weak references! ESO registers and unregisters itself from the event loop manually.
    Vector<RawPtr<EnvironmentSettingsObject>> m_related_environment_settings_objects;

    // https://html.spec.whatwg.org/multipage/webappapis.html#backup-incumbent-settings-object-stack
    Vector<GC::Ref<EnvironmentSettingsObject>> m_backup_incumbent_realm_stack;

    // https://html.spec.whatwg.org/multipage/browsing-the-web.html#termination-nesting-level
    size_t m_termination_nesting_level { 0 };

    size_t m_execution_pause_depth { 0 };

    bool m_running_rendering_task { false };
    bool m_running_synchronous_rendering_update { false };
    bool m_rendering_update_may_overlap { false };
    // Whether the rendering update in progress granted the clock leases as its frame went in flight.
    bool m_clock_leases_granted_in_flight { false };
    bool m_rendering_task_queued { false };
    // The queued rendering task ran while the previous rendering update's frame was in flight and held its rendering
    // opportunity: it counts as queued, and is queued again once that frame has been taken in.
    bool m_rendering_task_held { false };
    bool m_rendering_task_runs_ahead { false };
    bool m_rendering_task_ran_ahead_since_last_task { false };
    u64 m_rendering_task_queued_at_nanoseconds { 0 };
    bool m_rendering_update_requested { false };

    RenderingSchedulerCounters m_rendering_scheduler_counters;
    RenderingSchedulerCounters m_rendering_scheduler_counters_at_last_update;
    double m_last_rendering_update_end_time { 0 };
    u64 m_frame_submitted_at_nanoseconds { 0 };
    // The frames the rendering update running now has submitted so far.
    u64 m_frames_submitted_by_rendering_update { 0 };
    static bool s_a_frame_is_in_flight;

    GC::Ptr<GC::Function<void()>> m_rendering_task_function;

    GC::Ptr<GC::Function<void()>> m_finished_frame_consumer;
    bool m_finished_frame_consumer_call_requested { false };
    size_t m_processing_depth { 0 };
    size_t m_spin_depth { 0 };
    bool m_calling_finished_frame_consumer { false };
    bool m_consuming_frame_commit { false };
    bool m_running_consume_tail { false };

    NonnullOwnPtr<FrameScheduler> m_frame_scheduler;
    double m_rendering_update_start_time { 0 };
};

WEB_API EventLoop& main_thread_event_loop();
WEB_API void run_when_event_loop_reaches_step_1(GC::Ref<GC::Function<void()>> steps);
WEB_API TaskID queue_a_task(HTML::Task::Source, GC::Ptr<EventLoop>, GC::Ptr<DOM::Document>, GC::Ref<GC::Function<void()>> steps, Task::Priority = Task::Priority::Normal);
WEB_API TaskID queue_global_task(HTML::Task::Source, JS::Object&, GC::Ref<GC::Function<void()>> steps, Task::Priority = Task::Priority::Normal);
WEB_API void queue_a_microtask(GC::Ptr<DOM::Document const>, GC::Ref<GC::Function<void()>> steps);
void perform_a_microtask_checkpoint();

}
