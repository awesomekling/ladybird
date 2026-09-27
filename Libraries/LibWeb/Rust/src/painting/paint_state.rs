/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::layout::node_data::NodeSlotId;
use std::sync::Arc;

pub(crate) struct PendingRecording {
    pub(crate) recording: crate::painting::record::RecordingResult,
    pub(crate) recording_from_scratch: Option<crate::painting::record::RecordingResult>,
    pub(crate) publishes_recording: bool,
    /// The generation of the arena's render state the recording was made for.
    pub(crate) frame_generation: u64,
    /// The SVG paint resources of the frame recorded, whose filter images the publication hands the
    /// host.
    pub(crate) svg_paint_resources: Arc<crate::painting::svg_paint_resources::SvgPaintResourceRows>,
}

pub(crate) struct PendingRecordingTrace {
    pub(crate) viewport: NodeSlotId,
    pub(crate) should_paint_overlay: bool,
}

/// What a clock lease's tick records its document's display list with.
#[derive(Clone)]
pub(crate) struct ClockRecording {
    pub(crate) viewport: NodeSlotId,
    pub(crate) inputs: crate::painting::record::RecordingInputs<'static>,
}

#[derive(Default)]
pub struct PaintState {
    pub(crate) trace_recordings: bool,
    // The SVG-as-image renders the next recording looks up, resolved by the main thread.
    pub(crate) vector_image_display_lists:
        std::sync::Arc<crate::painting::record::vector_images::VectorImageDisplayLists>,
    pub(crate) visual_context: crate::painting::visual_context::VisualContextState,
    pub(crate) root_background_source: Option<crate::painting::host::FfiRootBackgroundSource>,
    pub(crate) hit_test_list_generation: u64,
    pub(crate) last_recording: Option<Arc<crate::painting::record::RecordingOutput>>,
    pub(crate) selection: Option<Arc<crate::painting::selection::SelectionRange>>,
    // LIBWEB_RENDER_CLOCK_FRAMES: the inputs of the last recording the main thread published, which
    // a clock lease's ticks record again with while the main thread idles.
    pub(crate) clock_recording: Option<ClockRecording>,
    // Shared with the frames published since it last changed.
    pub(crate) selection_pseudo_styles: Arc<SelectionPseudoStyles>,
}

pub(crate) type SelectionPseudoStyles =
    std::collections::HashMap<NodeSlotId, Arc<crate::painting::record::paint::text::SelectionStyleAnswer>>;

impl PaintState {
    pub(crate) fn update_root_background_source(
        &mut self,
        arena: &crate::layout::LayoutNodeArena,
        source: crate::painting::host::FfiRootBackgroundSource,
    ) -> bool {
        let Some(previous) = self.root_background_source.replace(source) else {
            return false;
        };
        if previous == source {
            return false;
        }
        use crate::painting::record::damage::PaintDamage;
        // Propagation changes which box paints the body's background. Push both the old
        // and new owners, including inline pieces painted by their containing block.
        for source in [previous, source] {
            for slot in [source.root_layout_node, source.body_layout_node] {
                arena.push_paint_damage_for_repaint(slot, PaintDamage::DRAW_BACKGROUND);
            }
            // The viewport's scrollbars take their colors from the propagated background.
            if let Some(viewport) = arena.node_parent_if_live(source.root_layout_node) {
                arena.push_paint_damage(viewport, PaintDamage::DRAW_OVERLAY | PaintDamage::SCROLL_METADATA);
            }
        }
        true
    }

    pub(crate) fn reset_visual_context_state(&mut self) {
        self.visual_context = crate::painting::visual_context::VisualContextState {
            needs_to_refresh_scroll_state: true,
            ..Default::default()
        };
        self.visual_context
            .dirty_boxes
            .request_full_rebuild(crate::painting::visual_context::dirty::VisualContextGlobalRebuildReason::FirstBuild);
    }
}
