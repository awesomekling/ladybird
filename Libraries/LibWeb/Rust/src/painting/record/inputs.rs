/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::css::css_pixels::{CssPixelPoint, CssPixelRect, CssPixels};
use crate::layout::node_data::NodeSlotId;
use crate::painting::display_list::commands::UniqueNodeId;
use crate::painting::ffi::FfiChromeMetrics;
use crate::painting::force_dark::ForceDarkSettings;
use crate::painting::host::{FfiFlexOverlayInput, FfiGridOverlayInput, FfiRootBackgroundSource};
use libgfx_rust::font::FontHandle;
use libgfx_rust::{Color, IntRect, IntSize};
use std::borrow::Cow;

/// The inputs read by content that is recorded outside per-box captures: scroll metadata,
/// viewport scrollbars, the wheel-target facts of hit-test items. Reusing a subtree capture from
/// the previous tape requires this whole bundle to be unchanged, so every reader takes these
/// values from here and a new one is part of that check automatically.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct UncapturedContentInputs {
    pub viewport_wheel_overflow_x: u8,
    pub viewport_wheel_overflow_y: u8,
    pub root_background_source: FfiRootBackgroundSource,
    // Scroll commands use a scrollport at the origin. Its position is compositor state.
    pub device_viewport_size: IntSize,
    pub is_recording_async_scrolling_metadata: bool,
    pub document_id: UniqueNodeId,
    pub has_blocking_wheel_event_region_covering_viewport: bool,
    pub chrome_metrics: FfiChromeMetrics,
    pub paint_viewport_scrollbars: bool,
    pub middle_button_scroll_origin: Option<CssPixelPoint>,
    pub canvas_color: Color,
    pub background_color: Color,
}

/// Inputs for one recording. A synchronous recording call borrows the host's arrays and byte
/// buffers; a recording that outlives the call owns copies of them ([`Self::into_owned`]).
/// Recording results retain their own resources and never borrow these inputs.
#[derive(Clone)]
pub(crate) struct RecordingInputs<'a> {
    pub device_pixels_per_css_pixel: f64,
    pub uncaptured: UncapturedContentInputs,
    // Carried on the recording so the compositor can tell which listener state it saw; no
    // recorded content reads it.
    pub wheel_event_listener_state_generation: u64,
    pub css_viewport_rect: CssPixelRect,
    pub should_show_line_box_borders: bool,
    pub force_dark_enabled: bool,
    pub force_dark_settings: ForceDarkSettings,
    pub should_paint_overlay: bool,
    pub canvas_fill_rect: Option<IntRect>,
    pub opaque_canvas: bool,
    pub bitmap_rect: IntRect,
    pub publishes_recording: bool,
    pub window_is_focused: bool,
    pub outline_auto_color: Color,
    pub selection_background_from_palette: Color,
    pub selection_background_light: Color,
    pub selection_background_dark: Color,
    pub palette_is_dark: bool,
    pub document_has_supported_color_schemes: bool,
    // An SVG used as an image answers `prefers-color-scheme` with the referencing element's used
    // `color-scheme`, but only when that element or the document opted into a scheme it can
    // answer with. These are the document's half of that decision.
    pub document_declares_light_or_dark_color_scheme: bool,
    pub image_color_scheme_fallback: u8,
    // The SVG-as-image renders the main thread resolved before the recording started.
    pub vector_image_display_lists: std::sync::Arc<crate::painting::record::vector_images::VectorImageDisplayLists>,
    pub inspector_highlight: Option<InspectorHighlight<'a>>,
    pub tooltip_color: Color,
    pub tooltip_text_color: Color,
    pub tooltip_border_color: Color,
    pub grid_overlays: Option<GridOverlays<'a>>,
    // These array elements are already plain values without pointers or optional-value tags.
    pub flex_overlays: Cow<'a, [FfiFlexOverlayInput]>,
    pub caret_debug_rect: Option<CssPixelRect>,
    pub caret: Option<CaretPaint>,
    pub focused_text_control: Option<FocusedTextControlSelection>,
    pub focused_area_outline: Option<FocusedAreaOutline<'a>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CaretTarget {
    InBlock {
        block: NodeSlotId,
        owner: Option<NodeSlotId>,
    },
    EmptyInline(NodeSlotId),
}

#[derive(Clone, Copy)]
pub(crate) struct CaretPaint {
    pub target: CaretTarget,
    pub rect: CssPixelRect,
    pub color: Color,
    pub blink_cycle_start_time_ns: i64,
    pub should_blink: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct FocusedTextControlSelection {
    pub text_node: NodeSlotId,
    pub start: usize,
    pub end: usize,
}

#[derive(Clone)]
pub(crate) struct FocusedAreaOutline<'a> {
    pub image: NodeSlotId,
    pub path_bytes: Cow<'a, [u8]>,
    pub color: Color,
    pub width: CssPixels,
}

#[derive(Clone)]
pub(crate) struct OverlayLabelFonts {
    pub css_font: FontHandle,
    pub device_font: FontHandle,
}

#[derive(Clone)]
pub(crate) struct InspectorHighlight<'a> {
    pub paintable: NodeSlotId,
    pub label: Cow<'a, str>,
    pub fonts: OverlayLabelFonts,
}

#[derive(Clone)]
pub(crate) struct GridOverlays<'a> {
    pub inputs: Cow<'a, [FfiGridOverlayInput]>,
    pub fonts: OverlayLabelFonts,
}

impl RecordingInputs<'_> {
    /// These inputs with copies of everything they borrow from the host.
    pub(crate) fn into_owned(self) -> RecordingInputs<'static> {
        RecordingInputs {
            inspector_highlight: self.inspector_highlight.map(|highlight| InspectorHighlight {
                paintable: highlight.paintable,
                label: Cow::Owned(highlight.label.into_owned()),
                fonts: highlight.fonts,
            }),
            grid_overlays: self.grid_overlays.map(|grid| GridOverlays {
                inputs: Cow::Owned(grid.inputs.into_owned()),
                fonts: grid.fonts,
            }),
            flex_overlays: Cow::Owned(self.flex_overlays.into_owned()),
            focused_area_outline: self.focused_area_outline.map(|outline| FocusedAreaOutline {
                image: outline.image,
                path_bytes: Cow::Owned(outline.path_bytes.into_owned()),
                color: outline.color,
                width: outline.width,
            }),
            ..self
        }
    }
}
