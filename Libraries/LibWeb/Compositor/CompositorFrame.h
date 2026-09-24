/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/AtomicRefCounted.h>
#include <AK/NonnullRefPtr.h>
#include <AK/Optional.h>
#include <LibGfx/Rect.h>
#include <LibWeb/Compositor/Types.h>
#include <LibWeb/Export.h>
#include <LibWeb/Painting/AccumulatedVisualContext.h>
#include <LibWeb/Painting/DisplayList.h>
#include <LibWeb/Painting/DisplayListResourceStorage.h>
#include <LibWeb/Painting/ScrollState.h>

namespace Web::Compositor {

// What a navigable hands its compositor context for one frame. The frame owns everything its messages carry, so it
// can be handed to the compositor from any thread.
struct CompositorFrame {
    // A newly recorded display list, with the visual context tree, resources and scroll state it paints with.
    struct DisplayListUpdate {
        NonnullRefPtr<Painting::DisplayList> display_list;
        Painting::AccumulatedVisualContextTree visual_context_tree;
        Painting::DisplayListResourceTransaction resource_transaction;
        Painting::ScrollStateSnapshot scroll_state_snapshot;
    };

    // A new visual context tree for the display list the compositor already has.
    struct VisualContextTreeUpdate {
        Painting::AccumulatedVisualContextTree visual_context_tree;
        Painting::DisplayListResourceTransaction resource_transaction;
    };

    // The scroll state of the display list the compositor already has.
    struct ScrollStateUpdate {
        Painting::ScrollStateSnapshot scroll_state_snapshot;
        KeyboardScrollState keyboard_scroll_state;
    };

    CompositorContextId context_id;
    Optional<DisplayListUpdate> display_list_update;
    Optional<VisualContextTreeUpdate> visual_context_tree_update;
    Optional<ScrollStateUpdate> scroll_state_update;
    // Set when the frame is presented once the compositor has applied it.
    Optional<Gfx::IntRect> present_viewport_rect;
};

// Hands finished frames to the compositor. Unlike the rest of a compositor connection, which belongs to the thread that
// created it, a frame sink may be used from any thread. The messages of one frame reach the compositor together, and
// frames reach it in the order they were submitted.
class WEB_API CompositorFrameSink : public AtomicRefCounted<CompositorFrameSink> {
public:
    virtual ~CompositorFrameSink() = default;

    virtual void submit(CompositorFrame&&) = 0;
};

}
