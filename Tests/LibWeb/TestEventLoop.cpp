/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/ScopeGuard.h>
#include <LibCore/EventLoop.h>
#include <LibTest/TestCase.h>
#include <LibThreading/Thread.h>
#include <LibWeb/Bindings/MainThreadVM.h>
#include <LibWeb/Bindings/PrincipalHostDefined.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameCompletion.h>
#include <LibWeb/HTML/Scripting/Environments.h>
#include <LibWeb/HTML/Scripting/TemporaryExecutionContext.h>
#include <LibWeb/HTML/Window.h>
#include <LibWeb/Platform/FontPlugin.h>
#include <unistd.h>

static void install_font_plugin()
{
    static Web::Platform::FontPlugin font_plugin { false };
    static bool s_installed = false;
    if (!exchange(s_installed, true))
        Web::Platform::FontPlugin::install(font_plugin);
}

TEST_CASE(nested_pause_handles_keep_the_event_loop_paused)
{
    install_font_plugin();

    auto realm = Web::Bindings::create_a_principal_javascript_realm();
    URL::URL url;
    auto* window = Web::HTML::window_from_global_object(realm->global_object());
    VERIFY(window);
    auto document = Web::DOM::Document::create(Web::Bindings::principal_host_defined_page(*realm), *window, url);
    document->set_window(*window);
    window->set_associated_document(document);
    auto& event_loop = Web::HTML::main_thread_event_loop();

    {
        auto outer_pause = event_loop.pause();
        EXPECT(event_loop.execution_paused());

        {
            auto inner_pause = event_loop.pause();
            EXPECT(event_loop.execution_paused());
        }

        EXPECT(event_loop.execution_paused());
    }

    EXPECT(!event_loop.execution_paused());
}

static Core::EventLoop& core_event_loop()
{
    static Core::EventLoop s_event_loop;
    // Installs the platform plugins the event loop spins through.
    (void)Web::Bindings::main_thread_vm();
    return s_event_loop;
}

static bool pump_until(Function<bool()> const& condition)
{
    auto& event_loop = core_event_loop();
    for (int i = 0; i < 5000; ++i) {
        if (condition())
            return true;
        event_loop.pump(Core::EventLoop::WaitMode::PollForEvents);
        usleep(1000);
    }
    return condition();
}

static void post_from_another_thread(Web::HTML::FrameCompletion& completion, int times = 1)
{
    auto thread = Threading::Thread::construct("FrameStage"sv, [&completion, times]() -> intptr_t {
        for (int i = 0; i < times; ++i)
            completion.post();
        return 0;
    });
    thread->start();
    (void)thread->join();
}

TEST_CASE(frame_completion_posted_before_registration_is_delivered_at_registration)
{
    core_event_loop();
    Web::HTML::FrameCompletion completion;
    post_from_another_thread(completion);
    EXPECT(completion.is_pending());

    int deliveries = 0;
    completion.register_event_loop([&] { ++deliveries; });
    EXPECT(pump_until([&] { return deliveries == 1; }));
    EXPECT(completion.take());
    EXPECT(!completion.is_pending());
}

TEST_CASE(frame_completion_posts_coalesce_into_one_delivery)
{
    core_event_loop();
    Web::HTML::FrameCompletion completion;
    int deliveries = 0;
    completion.register_event_loop([&] { ++deliveries; });
    EXPECT(!completion.is_pending());

    post_from_another_thread(completion, 3);
    EXPECT(pump_until([&] { return deliveries == 1; }));
    EXPECT_EQ(completion.posted_count(), 3u);
    EXPECT_EQ(completion.delivered_count(), 1u);
    EXPECT(completion.take());
    EXPECT(!completion.take());

    // A completion posted after the last take is delivered again.
    post_from_another_thread(completion);
    EXPECT(pump_until([&] { return deliveries == 2; }));
    EXPECT(completion.take());
}

TEST_CASE(frame_completion_taken_before_its_delivery_runs_is_not_delivered)
{
    core_event_loop();
    Web::HTML::FrameCompletion completion;
    int deliveries = 0;
    completion.register_event_loop([&] { ++deliveries; });
    post_from_another_thread(completion);
    // A forced join took the frame before the delivery ran.
    EXPECT(completion.take());
    core_event_loop().pump(Core::EventLoop::WaitMode::PollForEvents);
    EXPECT_EQ(deliveries, 0);
}

// Leaves the JavaScript execution context stack empty (creating a realm pushes onto it), as the processing model's
// outermost step 1 sees it.
static GC::Ref<Web::DOM::Document> create_test_document()
{
    ScopeGuard empty_the_stack = [] {
        auto& vm = Web::Bindings::main_thread_vm();
        while (!vm.execution_context_stack().is_empty())
            vm.pop_execution_context();
    };
    install_font_plugin();
    auto realm = Web::Bindings::create_a_principal_javascript_realm();
    URL::URL url;
    auto* window = Web::HTML::window_from_global_object(realm->global_object());
    VERIFY(window);
    auto document = Web::DOM::Document::create(Web::Bindings::principal_host_defined_page(*realm), *window, url);
    document->set_window(*window);
    window->set_associated_document(document);
    return document;
}

struct FrameConsumerLog {
    int calls { 0 };
    int commits { 0 };
    int tails { 0 };
    int tails_held_back { 0 };
    int commits_in_nested_loop { 0 };
};

// Consumes like the scheduler core does: commit whenever allowed, run the tail only where allowed, else ask again.
static void install_test_frame_consumer(FrameConsumerLog& log, bool& tail_pending)
{
    auto& event_loop = Web::HTML::main_thread_event_loop();
    event_loop.set_finished_frame_consumer(GC::create_function(GC::Heap::the(), [&log, &tail_pending, &event_loop] {
        ++log.calls;
        if (Web::HTML::FrameCompletion::the().is_pending() && event_loop.may_consume_commit(Web::HTML::EventLoop::FrameConsumeSite::StepOne)) {
            event_loop.consume_commit(Web::HTML::EventLoop::FrameConsumeSite::StepOne, [&] {
                VERIFY(Web::HTML::FrameCompletion::the().take());
                ++log.commits;
                if (event_loop.spin_depth() > 0)
                    ++log.commits_in_nested_loop;
                tail_pending = true;
            });
        }
        if (!tail_pending)
            return;
        if (!event_loop.may_run_consume_tail()) {
            ++log.tails_held_back;
            event_loop.call_finished_frame_consumer_again();
            return;
        }
        event_loop.run_consume_tail([&] {
            ++log.tails;
            tail_pending = false;
        });
    }));
}

TEST_CASE(idle_and_hidden_pages_consume_a_finished_frame)
{
    core_event_loop();
    auto document = create_test_document();
    auto& event_loop = Web::HTML::main_thread_event_loop();

    FrameConsumerLog log;
    bool tail_pending = false;
    install_test_frame_consumer(log, tail_pending);

    // Idle: no task, no rendering opportunity. The completion alone gets the frame consumed.
    post_from_another_thread(Web::HTML::FrameCompletion::the());
    EXPECT(pump_until([&] { return log.tails == 1; }));
    EXPECT_EQ(log.commits, 1);
    EXPECT_EQ(log.tails_held_back, 0);

    // Hidden: no rendering opportunity will ever come.
    document->update_the_visibility_state(Web::HTML::VisibilityState::Hidden);
    EXPECT(document->hidden());
    post_from_another_thread(Web::HTML::FrameCompletion::the());
    EXPECT(pump_until([&] { return log.tails == 2; }));
    EXPECT_EQ(log.commits, 2);
    EXPECT(!Web::HTML::FrameCompletion::the().is_pending());

    event_loop.set_finished_frame_consumer(nullptr);
}

TEST_CASE(a_nested_event_loop_commits_but_never_runs_a_consume_tail)
{
    core_event_loop();
    auto document = create_test_document();
    auto& event_loop = Web::HTML::main_thread_event_loop();

    FrameConsumerLog log;
    bool tail_pending = false;
    install_test_frame_consumer(log, tail_pending);

    // A task spins the event loop until the frame that finished meanwhile is committed; its tail waits for the spin
    // to end and runs at the next outermost step 1.
    bool task_done = false;
    int tails_when_task_finished = -1;
    bool nested_task_ran = false;
    Web::HTML::queue_a_task(Web::HTML::Task::Source::Unspecified, event_loop, nullptr, GC::create_function(GC::Heap::the(), [&] {
        // A task the nested loop runs first leaves no currently running task behind, so only the nesting itself tells
        // the nested step 1 that a caller is suspended below it.
        Web::HTML::queue_a_task(Web::HTML::Task::Source::Unspecified, event_loop, nullptr, GC::create_function(GC::Heap::the(), [&] {
            nested_task_ran = true;
        }));
        event_loop.spin_until(GC::create_function(GC::Heap::the(), [&] { return nested_task_ran; }));
        post_from_another_thread(Web::HTML::FrameCompletion::the());
        event_loop.spin_until(GC::create_function(GC::Heap::the(), [&] { return log.commits == 1; }));
        EXPECT(!event_loop.may_run_consume_tail());
        tails_when_task_finished = log.tails;
        task_done = true;
    }));
    event_loop.schedule();

    EXPECT(pump_until([&] { return task_done && log.tails == 1; }));
    EXPECT_EQ(tails_when_task_finished, 0);
    EXPECT_EQ(log.commits_in_nested_loop, 1);
    EXPECT(log.tails_held_back >= 1);

    // A paused event loop (dialogs, synchronous XHR) consumes nothing at step 1, and runs the tail once unpaused.
    {
        Web::HTML::TemporaryExecutionContext context { Web::HTML::relevant_realm(*document) };
        auto pause = event_loop.pause(Web::HTML::EventLoop::UpdateTheRendering::No);
        EXPECT(!event_loop.may_consume_commit(Web::HTML::EventLoop::FrameConsumeSite::StepOne));
        EXPECT(event_loop.may_consume_commit(Web::HTML::EventLoop::FrameConsumeSite::ForcedJoin));
        post_from_another_thread(Web::HTML::FrameCompletion::the());
        for (int i = 0; i < 20; ++i) {
            core_event_loop().pump(Core::EventLoop::WaitMode::PollForEvents);
            usleep(1000);
        }
        EXPECT_EQ(log.commits, 1);
    }
    EXPECT(pump_until([&] { return log.tails == 2; }));
    EXPECT_EQ(log.commits, 2);

    event_loop.set_finished_frame_consumer(nullptr);
}
