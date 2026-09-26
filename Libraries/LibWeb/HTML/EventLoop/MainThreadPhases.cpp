/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Array.h>
#include <AK/HashMap.h>
#include <AK/NeverDestroyed.h>
#include <AK/StringBuilder.h>
#include <AK/Time.h>
#include <LibGC/Heap.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/EventLoop/MainThreadPhases.h>
#include <LibWeb/HTML/EventLoop/Task.h>
#include <time.h>

namespace Web::HTML::MainThreadPhases {

static constexpr size_t phase_count = to_underlying(Phase::Count);

static constexpr Array<StringView, phase_count> phase_labels {
#define __ENUMERATE_MAIN_THREAD_PHASE(name, label) label##sv,
    ENUMERATE_MAIN_THREAD_PHASES(__ENUMERATE_MAIN_THREAD_PHASE)
#undef __ENUMERATE_MAIN_THREAD_PHASE
};

struct Row {
    u64 windows { 0 };
    u64 wall_nanoseconds { 0 };
    u64 cpu_nanoseconds { 0 };
    Array<u64, phase_count> phase_nanoseconds {};
    Array<u64, phase_count> phase_entries {};
};

// Rows by "<window>:<suite>". Only the thread that writes the runner's marks (the main thread) touches them.
static HashMap<String, Row>& rows()
{
    static NeverDestroyed<HashMap<String, Row>> rows;
    return *rows;
}

static bool s_enabled { false };

static constexpr size_t max_depth = 256;
static thread_local Array<Phase, max_depth> s_stack;
static thread_local size_t s_depth { 0 };
static thread_local Row* s_row { nullptr };
static thread_local u64 s_last_nanoseconds { 0 };
static thread_local u64 s_window_start_nanoseconds { 0 };
static thread_local u64 s_window_start_cpu_nanoseconds { 0 };

static u64 now_nanoseconds()
{
    return MonotonicTime::now().nanoseconds();
}

static u64 thread_cpu_nanoseconds()
{
    timespec ts {};
    clock_gettime(CLOCK_THREAD_CPUTIME_ID, &ts);
    return static_cast<u64>(ts.tv_sec) * 1'000'000'000 + static_cast<u64>(ts.tv_nsec);
}

static Phase top()
{
    if (s_depth == 0)
        return Phase::Outside;
    return s_stack[min(s_depth, max_depth) - 1];
}

static bool is_rendering_phase(Phase phase)
{
    return phase >= Phase::RenderingInput && phase <= Phase::RenderingSubmit;
}

static void charge_top()
{
    if (!s_row)
        return;
    auto now = now_nanoseconds();
    s_row->phase_nanoseconds[to_underlying(top())] += now - s_last_nanoseconds;
    s_last_nanoseconds = now;
}

Scope::Scope(Phase phase)
{
    charge_top();
    if (s_row)
        ++s_row->phase_entries[to_underlying(phase)];
    if (s_depth < max_depth)
        s_stack[s_depth] = phase;
    ++s_depth;
}

Scope::~Scope()
{
    charge_top();
    --s_depth;
}

// Whether a phase runs script, whose style and layout reads are forced ones even inside a rendering update.
static bool runs_script(Phase phase)
{
    switch (phase) {
    case Phase::TaskTimer:
    case Phase::TaskDOMManipulation:
    case Phase::TaskNetworking:
    case Phase::TaskPostedMessage:
    case Phase::TaskUserInteraction:
    case Phase::TaskOther:
    case Phase::Microtasks:
    case Phase::RenderingInput:
    case Phase::RenderingResizeScrollMedia:
    case Phase::RenderingAnimations:
    case Phase::RenderingAnimationFrameCallbacks:
    case Phase::RenderingResizeObservers:
    case Phase::RenderingIntersectionObservers:
        return true;
    default:
        return false;
    }
}

// Whether the innermost phase that decides it is a step of the rendering update that updates style and layout itself,
// rather than one that runs script, or a task.
static bool in_rendering_update_steps()
{
    for (size_t index = min(s_depth, max_depth); index > 0; --index) {
        auto phase = s_stack[index - 1];
        if (runs_script(phase))
            return false;
        if (is_rendering_phase(phase))
            return true;
    }
    return false;
}

static bool is_child_document(DOM::Document const& document)
{
    return document.container_document() != nullptr;
}

Phase style_phase(DOM::Document const& document)
{
    bool child = is_child_document(document);
    if (in_rendering_update_steps())
        return child ? Phase::StyleInUpdateChild : Phase::StyleInUpdate;
    return child ? Phase::StyleForcedChild : Phase::StyleForced;
}

Phase layout_phase(DOM::Document const& document)
{
    bool child = is_child_document(document);
    if (in_rendering_update_steps())
        return child ? Phase::LayoutInUpdateChild : Phase::LayoutInUpdate;
    return child ? Phase::LayoutForcedChild : Phase::LayoutForced;
}

Phase task_phase(Task const& task)
{
    switch (task.source()) {
    case Task::Source::TimerTask:
        return Phase::TaskTimer;
    case Task::Source::DOMManipulation:
        return Phase::TaskDOMManipulation;
    case Task::Source::Networking:
        return Phase::TaskNetworking;
    case Task::Source::PostedMessage:
        return Phase::TaskPostedMessage;
    case Task::Source::UserInteraction:
        return Phase::TaskUserInteraction;
    case Task::Source::Rendering:
        return Phase::TaskRendering;
    default:
        return Phase::TaskOther;
    }
}

static void observe_collection(GC::Heap::CollectionWork work, bool begins)
{
    // A heap on another thread is not the main thread's time.
    if (!s_row && s_depth == 0 && begins)
        return;
    if (begins) {
        charge_top();
        auto phase = work == GC::Heap::CollectionWork::Collection ? Phase::GarbageCollection : Phase::GarbageSweep;
        if (s_row)
            ++s_row->phase_entries[to_underlying(phase)];
        if (s_depth < max_depth)
            s_stack[s_depth] = phase;
        ++s_depth;
        return;
    }
    auto phase = top();
    if (phase != Phase::GarbageCollection && phase != Phase::GarbageSweep)
        return;
    charge_top();
    --s_depth;
}

static void close_window()
{
    if (!s_row)
        return;
    charge_top();
    ++s_row->windows;
    s_row->wall_nanoseconds += s_last_nanoseconds - s_window_start_nanoseconds;
    s_row->cpu_nanoseconds += thread_cpu_nanoseconds() - s_window_start_cpu_nanoseconds;
    s_row = nullptr;
}

static void open_window(StringView window, Utf16View suite)
{
    close_window();
    GC::Heap::set_collection_observer(observe_collection);
    auto key = MUST(String::formatted("{}:{}", window, suite));
    s_row = &rows().ensure(key);
    s_window_start_nanoseconds = s_last_nanoseconds = now_nanoseconds();
    s_window_start_cpu_nanoseconds = thread_cpu_nanoseconds();
}

void set_enabled(bool enabled)
{
    s_enabled = enabled;
    if (!enabled)
        reset();
}

void did_write_benchmark_mark(Utf16View name)
{
    if (!s_enabled)
        return;
    auto dot = name.find_code_unit_offset(u'.');
    if (!dot.has_value())
        return;
    auto suite = name.substring_view(0, *dot);
    if (name.ends_with(u"-async-start"sv))
        return;
    if (name.ends_with(u"-async-end"sv))
        close_window();
    else if (name.ends_with(u"-sync-end"sv))
        open_window("async"sv, suite);
    else if (name.ends_with(u"-start"sv))
        open_window("sync"sv, suite);
}

void reset()
{
    s_row = nullptr;
    rows().clear();
}

String report_as_json()
{
    StringBuilder builder;
    builder.append('{');
    bool first_row = true;
    for (auto const& [key, row] : rows()) {
        if (!first_row)
            builder.append(',');
        first_row = false;
        builder.appendff("\"{}\":{{\"windows\":{},\"wallNs\":{},\"cpuNs\":{},\"phases\":{{", key, row.windows, row.wall_nanoseconds, row.cpu_nanoseconds);
        bool first_phase = true;
        for (size_t i = 0; i < phase_count; ++i) {
            if (!row.phase_nanoseconds[i] && !row.phase_entries[i])
                continue;
            if (!first_phase)
                builder.append(',');
            first_phase = false;
            builder.appendff("\"{}\":[{},{}]", phase_labels[i], row.phase_nanoseconds[i], row.phase_entries[i]);
        }
        builder.append("}}"sv);
    }
    builder.append('}');
    return MUST(builder.to_string());
}

}
