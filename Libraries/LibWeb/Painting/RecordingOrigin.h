/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/StringView.h>
#include <LibWeb/Export.h>

namespace Web::Painting {

// What asked for a recording, for counting the recordings the render side runs while the main thread waits for them.
enum class RecordingOrigin : u8 {
    // The rendering update's paint step, recording in the frame it submits unless one of the reasons below holds.
    RenderingUpdate,
    // A rendering update run on top of script, which submits no frame.
    SynchronousRenderingUpdate,
    // A rendering update whose flight recorded a frame that did not stand, which records again at once.
    WaitsForRecordings,
    // A rendering update of a frame scheduler that submits no frames.
    NoFrameScheduler,
    // A hit test that found its hit-test list stale.
    HitTest,
    Screenshot,
    // Painting a navigable outside the rendering update (a screenshot's descendants, a navigation's first frame).
    PaintIfNeeded,
    // A document's display list asked for as a value: SVG-as-image renders, dumps and tests.
    DocumentRecord,
    Count,
};

WEB_API StringView recording_origin_name(RecordingOrigin);

// Why the main thread sealed no paint for a flight, which then ends after its layout.
enum class FlightPaintDecline : u8 {
    // The navigable does not show the document, or has nowhere to present it.
    Inactive,
    // Hidden, an SVG page, or painted with a debug overlay or force-dark.
    NotPaintedThatWay,
    // A frame in flight presents to the navigable already.
    PresenterLent,
    // No paint state or viewport box yet, or the first paint waits for fonts.
    NothingToPaintYet,
    InspectorOverlay,
    Caret,
    FocusedTextControl,
    MiddleButtonScroll,
    ResizeObserver,
    Animations,
    ForcedCompositorLayer,
    ViewTransitionOrScrollState,
    // A navigable the document hosts may paint after it.
    HostedNavigable,
    Count,
};

WEB_API StringView flight_paint_decline_name(FlightPaintDecline);
// The origin of the recordings the main thread asks for now. Callers set it with a TemporaryChange.
WEB_API RecordingOrigin& current_recording_origin();

}
