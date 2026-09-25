/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Optional.h>
#include <AK/kmalloc.h>
#include <LibCompositing/DisplayList/AccumulatedVisualContext.h>
#include <LibCore/ElapsedTimer.h>
#include <LibGC/Ptr.h>
#include <LibGfx/Color.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/PaintableTypes.h>
#include <LibWeb/PixelUnits.h>

namespace Web::Painting {

// When the render side runs a display list recording the main thread has prepared.
enum class RecordingRun : u8 {
    // Right away, while the main thread waits for it.
    Now,
    // In the frame the rendering update submits, while the main thread runs its event loop.
    InSubmittedFrame,
};

// A display list recording the main thread has prepared and the render side runs. The main thread finishes it once
// the recording is done: it publishes the recording, adopts its display list and updates the hit-test list.
// NB: Whoever holds a pending recording keeps its document alive.
struct PendingDisplayListRecording {
    AK_ALLOC_WITH_KMALLOC;

    GC::Ref<DOM::Document> document;
    // The document's layout node arena, as its handle. The document keeps it alive.
    void* arena { nullptr };
    Compositing::DisplayListResourceStorage& resource_storage;
    Compositing::AccumulatedVisualContextTree visual_context_tree;
    PaintCommandCacheMode cache_mode;
    // How the render side runs this recording: InSubmittedFrame only if it went to the frame in flight.
    RecordingRun run { RecordingRun::Now };
    Optional<Color> surface_clear_color;
    DevicePixelRect device_viewport_rect;
    BlockingWheelEventRegionState wheel_event_region_state;
    // How often the document's hit-test list had been invalidated when the recording was prepared. A recording made
    // before a later invalidation does not make a current hit-test list.
    u64 hit_test_display_list_invalidations { 0 };
    Core::ElapsedTimer timer;
};

}
