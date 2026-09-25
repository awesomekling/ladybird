/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Types.h>
#include <AK/Utf16View.h>
#include <LibWeb/Export.h>

namespace Web::DOM {

// Why a rendering update lays out in place rather than beside the main thread. Each one is something that has to run
// after the layout pass of the rendering update with no task in between, or a rendering update that cannot yield.
#define ENUMERATE_LAYOUT_OVERLAP_BLOCKERS(X)   \
    X(SynchronousRenderingUpdate)              \
    X(ResizeObservation)                       \
    X(ViewTransition)                          \
    X(ScrollStateContainer)                    \
    X(ContentVisibilityAutoFirstDetermination) \
    X(ScrollTimeline)

enum class LayoutOverlapBlocker : u8 {
#define ENUMERATE_LAYOUT_OVERLAP_BLOCKER(e) e,
    ENUMERATE_LAYOUT_OVERLAP_BLOCKERS(ENUMERATE_LAYOUT_OVERLAP_BLOCKER)
#undef ENUMERATE_LAYOUT_OVERLAP_BLOCKER
};

static constexpr size_t layout_overlap_blocker_count = 0
#define ENUMERATE_LAYOUT_OVERLAP_BLOCKER(e) +1
    ENUMERATE_LAYOUT_OVERLAP_BLOCKERS(ENUMERATE_LAYOUT_OVERLAP_BLOCKER)
#undef ENUMERATE_LAYOUT_OVERLAP_BLOCKER
    ;

[[nodiscard]] WEB_API Utf16View to_string(LayoutOverlapBlocker);

}
