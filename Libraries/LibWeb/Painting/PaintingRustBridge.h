/*
 * Copyright (c) 2026, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Types.h>
#include <LibCompositing/DisplayList/AccumulatedVisualContext.h>
#include <LibCompositing/DisplayList/DisplayListResourceStorage.h>
#include <LibGfx/Filter.h>
#include <LibWeb/CSS/StyleValues/AbstractImageStyleValue.h>
#include <LibWeb/Compositor/NavigablePresenter.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/HTML/PaintConfig.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Painting/BoxSlot.h>
#include <LibWeb/Painting/FlexboxInspectorOverlay.h>
#include <LibWeb/Painting/GridInspectorOverlay.h>
#include <LibWeb/Painting/PaintableTypes.h>
#include <LibWeb/Painting/PendingDisplayListRecording.h>

namespace Web::Painting {

struct ImagePaint;
struct ImagePaintRequest;

WEB_API void dump_stacking_context_tree(StringBuilder&, DOM::Document const&);
WEB_API void dump_layout_tree(StringBuilder&, BoxSlot const&, bool interactive);

WEB_API Layout::RustFFI::FfiVisualContextUpdateOutcome rust_update_accumulated_visual_contexts(DOM::Document&);
WEB_API Vector<u32> rust_owned_visual_context_node_indices(DOM::Document const&, DOM::NodeIdentity, Layout::RustFFI::FfiVisualContextBoxNodeList);
WEB_API bool rust_background_color_can_be_compositor_animated(BoxSlot const&);
WEB_API bool rust_background_color_can_be_compositor_animated(DOM::Document const&, DOM::NodeIdentity);
WEB_API void const* retain_rust_main_visual_context_tree(DOM::Document const&);
WEB_API Layout::RustFFI::FfiPhysicalOverflowDirections rust_physical_overflow_directions(BoxSlot const&);
WEB_API Layout::RustFFI::FfiPhysicalOverflowDirections rust_physical_overflow_directions(DOM::Document const&, DOM::NodeIdentity);
WEB_API void register_geometry_host(DOM::Document&);
WEB_API Layout::RustFFI::FfiRenderingPreparationOutcome rust_prepare_for_rendering(DOM::Document&, bool visual_context_update_pending);
WEB_API void rust_update_visual_viewport_transform(DOM::Document&);
// Refreshes the snapshot from the Rust scroll state, unless nothing has invalidated it.
WEB_API void rust_refresh_scroll_state(DOM::Document&, Compositing::ScrollStateSnapshot&);
WEB_API void rust_invalidate_scroll_state(DOM::Document&);
struct InspectorOverlayInputs {
    BoxSlot highlighted_box;
    Color tooltip_color;
    Color tooltip_text_color;
    Color tooltip_border_color;
    struct GridHighlight {
        BoxSlot box;
        GridInspectorOverlayOptions options;
    };
    struct FlexHighlight {
        BoxSlot box;
        FlexboxInspectorOverlayOptions options;
    };
    Vector<GridHighlight> grid_highlights;
    Vector<FlexHighlight> flex_highlights;
    Optional<CSSPixelRect> caret_debug_rect;
};

// Resolves what the recording reads on the main thread and has the render side record it. Returns nothing if there is
// nothing to record; otherwise finish_rust_display_list_recording() finishes it once the render side has recorded it.
WEB_API Optional<PendingDisplayListRecording> begin_rust_display_list_recording(DOM::Document&, Compositing::DisplayList const& placeholder_display_list, Compositing::DisplayListResourceStorage&, PaintCommandCacheMode, HTML::PaintConfig const&, InspectorOverlayInputs const&, RecordingRun);
// What the main thread seals of a recording that a flight makes after its layout (LIBWEB_STAGE_OVERLAP naming flight),
// beyond what it hands the flight.
struct FlightRecordingSeal {
    DevicePixelRect device_viewport_rect;
    BlockingWheelEventRegionState wheel_event_region_state;
    // The canvas the recording paints, read ahead of the layout.
    Color canvas_color;
    Color background_color;
};
// Resolves what the recording reads on the main thread, as begin_rust_display_list_recording() does, and seals it for
// the next flight of the document, which records once it has laid the document out.
// With `present`, the flight presents what it records through it, called on the render side with `present_context`,
// the visual context tree the recording was made against (a reference it owns) and the scroll state snapshot the flight
// refreshed, if it did.
using FlightPresent = void (*)(void* context, void const* visual_context_tree, Gfx::FloatPoint const* scroll_offsets, size_t scroll_offset_count, bool scroll_state_refreshed);
WEB_API FlightRecordingSeal seal_rust_display_list_recording_for_flight(DOM::Document&, Compositing::DisplayListResourceStorage&, HTML::PaintConfig const&, InspectorOverlayInputs const&, FlightPresent present = nullptr, void* present_context = nullptr);
// Publishes the recording in its arena and returns its display list, which is the paint command cache source if the
// recording is identical to it. Reaches the document only through `source`.
WEB_API Compositor::PublishedDisplayList publish_rust_display_list_recording(PendingDisplayListRecording&, Compositing::DisplayList* paint_command_cache_source, Compositing::DisplayListResourceSet const& paint_command_cache_source_resources, Compositor::PresentationSource&);

// Like publish_rust_display_list_recording(), from the presentation stage of the frame in flight: from the recording's
// ticket, without reaching the arena, if the recording was submitted with one, or else from the arena the frame owns.
WEB_API Optional<Compositor::PublishedDisplayList> publish_rust_display_list_recording_in_frame(PendingDisplayListRecording&, void const* recording_ticket, Compositing::DisplayList* paint_command_cache_source, Compositing::DisplayListResourceSet const& paint_command_cache_source_resources, Compositor::PresentationSource&);
// Takes the trace of the document's last recording, if one was asked for.
WEB_API void take_recording_trace_if_pending(DOM::Document&);

// A presentation source that reads the document as it stands, on the main thread.
class WEB_API DocumentPresentationSource final : public Compositor::PresentationSource {
public:
    DocumentPresentationSource(DOM::Document&, u64 adopted_async_scroll_sequence);

    virtual Compositing::AccumulatedVisualContextTree published_display_list_visual_context_tree() override;
    virtual Optional<Compositor::AsyncScrollingStamp> async_scrolling_stamp() override;
    virtual void did_publish_recording() override;
    virtual Compositing::AccumulatedVisualContextTree visual_context_tree(Compositing::DisplayListResourceStorage&) override;
    virtual bool visual_context_tree_needs_compositor_update() override;
    virtual void did_update_visual_context_tree_in_compositor() override;
    virtual Compositing::ScrollStateSnapshot scroll_state_snapshot() override;

private:
    GC::Ref<DOM::Document> m_document;
    u64 m_adopted_async_scroll_sequence { 0 };
};
// Discards the recording if its document retired the render state it was made for since the recording began, and
// returns whether it did. The recording is not finished then.
WEB_API bool discard_retired_rust_display_list_recording(PendingDisplayListRecording&);
// A visual context tree read after the recording was published can reference SVG filter images it never saw.
WEB_API void add_published_svg_filter_image_frames(DOM::Document const&, Compositing::DisplayListResourceStorage&);
WEB_API bool last_recording_missed_vector_images(DOM::Document const&);
WEB_API Utf16String serialize_painting_dump(DOM::Document const&, Compositing::AccumulatedVisualContextTree const&, Compositing::DisplayList const&, Compositing::DisplayListResourceStorage const&);

WEB_API CSS::ColorResolutionContext gradient_stop_color_resolution_context(BoxSlot const&);
WEB_API CSS::ColorResolutionContext gradient_stop_color_resolution_context(DOM::Element const&);
// The graph applying a list of filter functions in order, or nothing for an empty list.
WEB_API Optional<Gfx::Filter> filter_from_functions(ReadonlySpan<Compositing::RustFFI::FfiFilterFunction>);

WEB_API Compositing::DisplayListResource record_image_paint_display_list(ImagePaint const&, ImagePaintRequest const&, double device_pixels_per_css_pixel);

}
