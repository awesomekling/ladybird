/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/String.h>
#include <AK/Types.h>
#include <AK/Utf16View.h>
#include <LibWeb/Forward.h>

// Where the main thread's time goes inside a benchmark step: the runner's performance marks (Speedometer's
// "<suite>.<test>-start", "-sync-end" and "-async-end") open and close a window, and the main thread's time in it is
// charged, exclusively, to the innermost phase it is in. Test only: read through internals.
namespace Web::HTML::MainThreadPhases {

#define ENUMERATE_MAIN_THREAD_PHASES(X)                           \
    X(Outside, "outside")                                         \
    X(TaskTimer, "task.timer")                                    \
    X(TaskDOMManipulation, "task.domManipulation")                \
    X(TaskNetworking, "task.networking")                          \
    X(TaskPostedMessage, "task.postedMessage")                    \
    X(TaskUserInteraction, "task.userInteraction")                \
    X(TaskOther, "task.other")                                    \
    X(TaskRendering, "task.rendering")                            \
    X(Microtasks, "microtasks")                                   \
    X(FrameConsumer, "frameConsumer")                             \
    X(StepOne, "stepOne")                                         \
    X(RenderingInput, "ru.input")                                 \
    X(RenderingCommitMessages, "ru.commitMessages")               \
    X(RenderingResizeScrollMedia, "ru.resizeScrollMedia")         \
    X(RenderingAnimations, "ru.animations")                       \
    X(RenderingAnimationFrameCallbacks, "ru.rafCallbacks")        \
    X(RenderingStep16, "ru.step16")                               \
    X(RenderingResizeObservers, "ru.resizeObservers")             \
    X(RenderingIntersectionObservers, "ru.intersectionObservers") \
    X(RenderingPaint, "ru.paint")                                 \
    X(RenderingFinish, "ru.finish")                               \
    X(RenderingSubmit, "ru.submit")                               \
    X(StyleInUpdate, "style.ru.top")                              \
    X(StyleInUpdateChild, "style.ru.child")                       \
    X(LayoutInUpdate, "layout.ru.top")                            \
    X(LayoutInUpdateChild, "layout.ru.child")                     \
    X(StyleForced, "style.forced.top")                            \
    X(StyleForcedChild, "style.forced.child")                     \
    X(LayoutForced, "layout.forced.top")                          \
    X(LayoutForcedChild, "layout.forced.child")                   \
    X(StyleBeginTop, "style.begin.top")                           \
    X(StyleBeginChild, "style.begin.child")                       \
    X(StyleUserAgentSheets, "style.uaSheets")                     \
    X(StyleScopeCaches, "style.scopeCaches")                      \
    X(FlightJoin, "flightJoin")                                   \
    X(GarbageCollection, "gc")                                    \
    X(GarbageSweep, "gc.sweepSlice")

enum class Phase : u8 {
#define __ENUMERATE_MAIN_THREAD_PHASE(name, label) name,
    ENUMERATE_MAIN_THREAD_PHASES(__ENUMERATE_MAIN_THREAD_PHASE)
#undef __ENUMERATE_MAIN_THREAD_PHASE
        Count,
};

class Scope {
public:
    explicit Scope(Phase);
    ~Scope();

    Scope(Scope const&) = delete;
    Scope& operator=(Scope const&) = delete;
};

// The style or layout phase of an update of `document`: run by the rendering update's own steps, or forced by a read
// (as script run inside the rendering update forces it too), of a top-level document or of a child document.
Phase style_phase(DOM::Document const&);
Phase layout_phase(DOM::Document const&);
Phase task_phase(Task const&);

// Off until a test enables it: a page's marks alone never open a window.
void set_enabled(bool);
void did_write_benchmark_mark(Utf16View name);
void reset();
String report_as_json();

}
