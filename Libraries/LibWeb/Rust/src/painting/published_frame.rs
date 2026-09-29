/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What a document publishes for the paint side to read, and the one trait the paint side reads it
//! through.
//!
//! The paint side reads the rows the document published last ([`RowSnapshot`]), which carry its paintable rows and
//! paint facts beside the layout rows. A [`PublishedFrame`] is such rows with what one recording reads besides. Both
//! are immutable: every column in them is a [`ColumnSnapshot`] or an `Arc` the document shares with them, so the
//! document writes its live columns (copying a chunk they still share) while they are read. [`PaintRead`] names every
//! read the display list recording makes of a document, so the recording is written against the trait rather than
//! against the live [`LayoutNodeArena`], and the compiler lists what the published rows still have to answer.

use crate::cow_column::ColumnSnapshot;
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::css_pixels::{CssPixelPoint, CssPixelRect};
use crate::layout::LayoutNodeArena;
use crate::layout::PublishedTextSlot;
use crate::layout::SLOTS_PER_CHUNK;
use crate::layout::fragment_tree::FragmentLink;
use crate::layout::node_data::{CompositorAnimationFrameKind, DomPaintFact, NodeFlag, NodeKind, NodeSlotId, PaintNode};
use crate::layout::node_facts;
use crate::layout::row_reads::RowSnapshot;
use crate::layout::text_chunker::GraphemeSegmenter;
use crate::layout::tree_shape::RetiredSlots;
use crate::layout::used_values::FfiCssPixelRect;
use crate::layout::{RenderedText, RenderedTextBoundary, TextFragments};
use crate::lent::Lent;
use crate::painting::fragment_ownership::FragmentOwnershipFilter;
use crate::painting::geometry_read::{GeometryRead, read_live_geometry};
use crate::painting::hit_test::HitTestList;
use crate::painting::host::FfiLayerImageList;
use crate::painting::image_map_areas::ImageMapAreas;
use crate::painting::layer_image_paint_facts::{LayerImagePaintFacts, LayerImagePaintFactsTable};
use crate::painting::paint_order_plan::PaintOrderInputs;
use crate::painting::paint_state::{PaintState, SelectionPseudoStyles};
use crate::painting::paintable_data::{CommittedSideData, PaintableData};
use crate::painting::paintable_rows::{
    CommittedFragmentLinkSlot, CommittedSideDataRef, PAINTABLE_SLOTS_PER_CHUNK, PaintableRowsRead,
};
use crate::painting::record::damage::{FrameDamage, PaintDamage};
use crate::painting::record::recorder_state::AbsoluteRectMemo;
use crate::painting::replaced_paint_facts::{ReplacedPaintFacts, ReplacedPaintFactsTable};
use crate::painting::selection::SelectionRange;
use crate::painting::stacking_context::entries::StackingContextEntries;
use crate::painting::svg_paint_resources::{
    PublishedSvgFilter, PublishedSvgPaintServer, SvgPaintResourceKind, SvgPaintResourceRows, published_filter_in,
    published_paint_server_in,
};
use crate::painting::visual_context::scroll_state::ScrollOffsets;
use crate::painting::visual_context::{BoxVisualContextNodeHandles, VisualContextTree};
use std::cell::RefCell;
use std::ops::Deref;
use std::sync::Arc;

/// One published generation of a document's paintable rows and of the columns read beside them.
#[derive(Clone, Default)]
pub(crate) struct PublishedRows {
    pub(super) rows: ColumnSnapshot<PaintableData, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) fragment_links: ColumnSnapshot<CommittedFragmentLinkSlot, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) side_data: ColumnSnapshot<CommittedSideData, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) unique_node_ids: ColumnSnapshot<(NodeSlotId, i64), PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) stacking_context_entries: ColumnSnapshot<Option<Arc<StackingContextEntries>>, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) visual_context_node_handles:
        ColumnSnapshot<Option<Arc<BoxVisualContextNodeHandles>>, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) scroll_offsets: Arc<ScrollOffsets>,
    pub(super) image_map_areas: Arc<ImageMapAreas>,
    pub(super) visual_context_tree: Option<Arc<VisualContextTree>>,
    /// The hit-test list of the last recording the document took in, with what a query derives from it built: the
    /// rows are what the list is hit tested over, as a scroll, a clip or a transform moves what a point hits.
    pub(crate) hit_test_list: Option<Arc<HitTestList>>,
}

/// What a document published for one recording to read: its rows, its paint damage and the paint state the recording
/// reads.
#[derive(Default)]
pub(crate) struct PublishedFrame {
    /// The rows, which the frame shares with every reader of the same publication.
    pub(super) rows: Lent<RowSnapshot>,
    /// Keeps the arena from reusing a slot this frame may name until the frame is dropped.
    _retired_slots: RetiredSlots,
    damage: FrameDamage,
    paint_state: PublishedPaintState,
}

/// The document's text rows and its replaced, layer image and SVG paint resource tables, as they
/// were when the rows were published.
#[derive(Clone, Default)]
pub(crate) struct PublishedPaintFacts {
    pub(crate) text: ColumnSnapshot<PublishedTextSlot, SLOTS_PER_CHUNK>,
    pub(crate) replaced: Arc<ReplacedPaintFactsTable>,
    pub(crate) layer_images: Arc<LayerImagePaintFactsTable>,
    pub(crate) svg_paint_resources: Arc<SvgPaintResourceRows>,
}

/// What a recording reads of the document's paint state, as it was when the frame was published.
#[derive(Default)]
pub(crate) struct PublishedPaintState {
    pub(crate) visual_context_tree: Option<Arc<VisualContextTree>>,
    /// Each scroll state slot's own scroll offset.
    scroll_own_offsets: Vec<CssPixelPoint>,
    pub(crate) has_non_viewport_wheel_scroll_target_candidate: bool,
    pub(crate) selection: Option<Arc<SelectionRange>>,
    pub(crate) selection_pseudo_styles: Arc<SelectionPseudoStyles>,
    pub(crate) hit_test_list_generation: u64,
    /// How many items the document's hit-test list held, which the recording's list reserves.
    pub(crate) hit_test_item_capacity_hint: usize,
}

impl PublishedPaintState {
    pub(crate) fn new(paint_state: &PaintState, hit_test_item_capacity_hint: usize) -> Self {
        let visual_context = &paint_state.visual_context;
        Self {
            visual_context_tree: visual_context.tree.clone(),
            scroll_own_offsets: visual_context
                .scroll_state
                .states
                .iter()
                .map(|state| state.own_offset)
                .collect(),
            has_non_viewport_wheel_scroll_target_candidate: visual_context
                .scroll_state
                .has_non_viewport_wheel_scroll_target_candidate,
            selection: paint_state.selection.clone(),
            selection_pseudo_styles: paint_state.selection_pseudo_styles.clone(),
            hit_test_list_generation: paint_state.hit_test_list_generation,
            hit_test_item_capacity_hint,
        }
    }

    pub(crate) fn structural_epoch(&self) -> u64 {
        self.visual_context_tree
            .as_ref()
            .map_or(0, |tree| tree.structural_epoch)
    }

    pub(crate) fn scroll_own_offset(&self, slot: usize) -> CssPixelPoint {
        let offset = self.scroll_own_offsets.get(slot).copied();
        debug_assert!(offset.is_some(), "the tree's scroll node has a scroll state");
        offset.unwrap_or_default()
    }
}

/// What the document thread reads of a document's paint state beside its rows, as the rows were published.
#[derive(Clone, Copy, Default)]
pub(crate) struct PaintStatus {
    /// Whether scrollable overflow a commit or a writer left is still to be measured: the committed rows are the
    /// rows once it is.
    pub(crate) scrollable_overflow_unmeasured: bool,
    /// How many scrollable overflow recalculations the document ran, for tests.
    pub(crate) scrollable_overflow_recalculations: u64,
    /// How many boxes a visual context update has yet to update, for tests.
    pub(crate) visual_context_dirty_boxes: usize,
    pub(crate) layout_commit_generation: u64,
    /// Whether the last recording painted an SVG-as-image render the main thread had not resolved as an empty image.
    pub(crate) last_recording_missed_vector_images: bool,
    /// The boxes the canvas background is painted from, as the last preparation for rendering found them.
    pub(crate) root_background_source: crate::painting::host::FfiRootBackgroundSource,
}

// A frame is read on whichever thread paints it while the document writes its live columns: it
// holds no cell, no raw pointer and no borrow of the document, and owns everything it reads.
const _: () = {
    const fn assert_published<T: Send + Sync + 'static>() {}
    assert_published::<PublishedFrame>();
};

impl PublishedFrame {
    pub(crate) fn new(
        rows: Lent<RowSnapshot>,
        retired_slots: RetiredSlots,
        damage: FrameDamage,
        paint_state: PublishedPaintState,
    ) -> Self {
        Self {
            rows,
            _retired_slots: retired_slots,
            damage,
            paint_state,
        }
    }

    /// The SVG paint resources the frame was published with.
    pub(crate) fn svg_paint_resources(&self) -> &Arc<SvgPaintResourceRows> {
        &self.rows.paint_facts.svg_paint_resources
    }

    /// How many paintable rows the frame has room for.
    pub(crate) fn paintable_row_capacity(&self) -> usize {
        self.rows.paintable.row_capacity()
    }

    /// What the frame's recording reads of the document's paint state.
    pub(crate) fn paint_state(&self) -> &PublishedPaintState {
        &self.paint_state
    }

    /// The paint damage the frame was published with.
    pub(crate) fn damage(&self) -> &FrameDamage {
        &self.damage
    }
}

impl RowSnapshot {
    /// What the rows published of a live text row.
    fn text(&self, id: NodeSlotId) -> Option<&PublishedTextSlot> {
        self.node(id)?;
        self.paint_facts
            .text
            .get(id.slot_index() as usize)
            .filter(|text| text.generation == id.generation())
    }

    fn node_and_style(&self, id: NodeSlotId) -> Option<(&PaintNode, Option<ComputedValuesView<'_>>)> {
        let node = self.node(id)?;
        Some((node, self.style(id)))
    }

    /// The node in a slot a read requires to be live.
    #[track_caller]
    fn live_node(&self, id: NodeSlotId) -> &PaintNode {
        assert!(!id.is_invalid(), "invalid layout node arena slot ID");
        self.node(id).expect("layout node arena read a stale or unused slot")
    }
}

fn link(node: NodeSlotId) -> Option<NodeSlotId> {
    (!node.is_invalid()).then_some(node)
}

impl PublishedRows {
    /// How many rows the generation has room for.
    pub(crate) fn row_capacity(&self) -> usize {
        self.rows.slot_capacity()
    }

    /// How many times the row in slot `id` was reset.
    pub(crate) fn row_reset_version(&self, id: NodeSlotId) -> u64 {
        self.rows
            .get(id.slot_index() as usize)
            .map_or(0, |row| row.row_reset_version)
    }

    pub(crate) fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        assert!(!id.is_invalid(), "invalid paintable arena slot ID");
        let data = self
            .rows
            .get(id.slot_index() as usize)
            .expect("invalid paintable arena slot ID");
        assert_eq!(
            data.slot_generation,
            id.generation(),
            "paintable arena read a stale or unused slot"
        );
        data
    }

    pub(crate) fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        if id.is_invalid() {
            return false;
        }
        let Some(data) = self.rows.get(id.slot_index() as usize) else {
            return false;
        };
        data.slot_generation != 0 && data.slot_generation == id.generation()
    }

    pub(crate) fn with_committed_fragment_link<R>(
        &self,
        id: NodeSlotId,
        read: impl FnOnce(Option<&FragmentLink>) -> R,
    ) -> R {
        debug_assert!(self.paintable_row_is_populated(id));
        read(
            self.fragment_links
                .get(id.slot_index() as usize)
                .and_then(CommittedFragmentLinkSlot::link),
        )
    }

    pub(crate) fn visual_context_node_handles(&self, id: NodeSlotId) -> &BoxVisualContextNodeHandles {
        self.paintable_row_is_populated(id)
            .then(|| self.visual_context_node_handles.get(id.slot_index() as usize))
            .flatten()
            .and_then(|handles| handles.as_deref())
            .unwrap_or(&crate::painting::visual_context::EMPTY_BOX_VISUAL_CONTEXT_NODE_HANDLES)
    }

    pub(crate) fn stacking_context_entries(&self, root: NodeSlotId) -> Option<&StackingContextEntries> {
        if !self.paintable_row_is_populated(root) {
            return None;
        }
        self.stacking_context_entries
            .get(root.slot_index() as usize)
            .and_then(|table| table.as_deref())
    }

    pub(crate) fn committed_side_data(&self, id: NodeSlotId) -> &CommittedSideData {
        debug_assert!(self.paintable_row_is_populated(id));
        self.side_data
            .get(id.slot_index() as usize)
            .expect("a populated row has published side data")
    }
}

/// The reads the display list recording makes of a document. The live arena answers them from the
/// columns layout writes; a [`PublishedFrame`] answers them from what the document published.
pub(crate) trait PaintRead: GeometryRead {
    fn with_paintable_visual_context_node_handles<R>(
        &self,
        id: NodeSlotId,
        read: impl FnOnce(&BoxVisualContextNodeHandles) -> R,
    ) -> R;

    fn slot_is_live(&self, id: NodeSlotId) -> bool;
    fn node_first_child_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_next_sibling_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_containing_block_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_generated_for(&self, id: NodeSlotId) -> u8;
    fn node_is_generated_for_pseudo_element(&self, id: NodeSlotId) -> bool;
    fn node_is_dom_backed(&self, id: NodeSlotId) -> bool;
    /// The style node of what a node was built for, which is how the host names that DOM node.
    fn node_style_node(&self, id: NodeSlotId) -> Option<crate::css::style::tree::StyleNodeID>;
    fn node_is_element_backed(&self, id: NodeSlotId) -> bool;
    fn node_is_out_of_flow_if_live(&self, id: NodeSlotId) -> bool;
    fn node_is_atomic_inline(&self, id: NodeSlotId) -> bool;
    fn node_is_positioned(&self, id: NodeSlotId) -> bool;
    fn node_is_floating(&self, id: NodeSlotId) -> bool;
    fn node_has_dom_paint_fact(&self, id: NodeSlotId, fact: DomPaintFact) -> bool;
    fn node_has_compositor_animation_frame(&self, id: NodeSlotId, kind: CompositorAnimationFrameKind) -> bool;
    fn node_style_if_live(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>>;
    /// The rendered text of a text row.
    fn rendered_text(&self, id: NodeSlotId) -> Option<&RenderedText>;
    /// Reads a text row's grapheme boundaries, for a text row with rendered text.
    fn with_grapheme_segmenter<R>(&self, id: NodeSlotId, read: impl FnOnce(&GraphemeSegmenter) -> R) -> Option<R>;
    /// The rows a text node is painted in: its first-letter row, if any, then its own.
    fn text_fragments(&self, primary: NodeSlotId) -> TextFragments;
    fn published_svg_filter(&self, slot: NodeSlotId, kind: SvgPaintResourceKind) -> Option<Arc<PublishedSvgFilter>>;
    fn published_svg_paint_server(
        &self,
        slot: NodeSlotId,
        kind: SvgPaintResourceKind,
    ) -> Option<Arc<PublishedSvgPaintServer>>;
    fn replaced_paint_facts(&self, id: NodeSlotId) -> Option<ReplacedPaintFacts>;
    fn layer_image_paint_facts(
        &self,
        id: NodeSlotId,
        list: FfiLayerImageList,
        computed_index: u32,
    ) -> Option<LayerImagePaintFacts>;
    /// The bounds of the SVG filter a box references, as the visual context update resolved them.
    fn svg_filter_bounds(&self, id: NodeSlotId) -> Option<FfiCssPixelRect>;
    /// The fragments of its line root a box paints, when the line root assigned them.
    fn fragment_ownership_filter(&self, id: NodeSlotId) -> Option<FragmentOwnershipFilter>;
    fn paint_damage_of_row(&self, row: NodeSlotId) -> PaintDamage;
    /// The paint-order inputs paint preparation gathered for the row, if it gathered them.
    fn prepared_paint_order_inputs(&self, row: NodeSlotId) -> Option<PaintOrderInputs> {
        self.committed_side_data(row).prepared_order_inputs()
    }
    fn stacking_context_entries(&self, root: NodeSlotId) -> Option<impl Deref<Target = StackingContextEntries> + '_>;
    fn damaged_paint_rows(&self) -> Vec<NodeSlotId>;

    fn dom_offset_for_rendered_text_offset(
        &self,
        id: NodeSlotId,
        offset: usize,
        boundary: RenderedTextBoundary,
    ) -> usize {
        if !self.node_kind_if_live(id).is_some_and(node_facts::kind_is_text) {
            return offset;
        }
        self.rendered_text(id)
            .expect("text must be published before mapping rendered offsets")
            .dom_offset_for_rendered_text_offset(offset, boundary)
    }

    fn rendered_text_offset_for_dom_offset(
        &self,
        id: NodeSlotId,
        offset: usize,
        boundary: RenderedTextBoundary,
    ) -> usize {
        if !self.node_kind_if_live(id).is_some_and(node_facts::kind_is_text) {
            return offset;
        }
        self.rendered_text(id)
            .expect("text must be published before mapping DOM offsets")
            .rendered_text_offset_for_dom_offset(offset, boundary)
    }

    /// Visits the layout subtree under `root` in pre-order, descending into a node's children only
    /// when the visit asks to.
    fn for_each_node_in_layout_subtree_in_pre_order_with_pruning(
        &self,
        root: NodeSlotId,
        mut visit_node_and_report_whether_to_descend: impl FnMut(NodeSlotId) -> bool,
    ) {
        let mut current = root;
        loop {
            let descend_into_children = visit_node_and_report_whether_to_descend(current);
            if descend_into_children && let Some(first_child) = self.node_first_child_if_live(current) {
                current = first_child;
                continue;
            }
            if current == root {
                break;
            }
            if let Some(next_sibling) = self.node_next_sibling_if_live(current) {
                current = next_sibling;
                continue;
            }
            let Some(parent) = self.node_parent_if_live(current) else {
                debug_assert!(false, "a node under the walked root has no parent");
                return;
            };
            current = parent;
            while current != root {
                if let Some(next_sibling) = self.node_next_sibling_if_live(current) {
                    current = next_sibling;
                    break;
                }
                let Some(parent) = self.node_parent_if_live(current) else {
                    debug_assert!(false, "a node under the walked root has no parent");
                    return;
                };
                current = parent;
            }
            if current == root {
                break;
            }
        }
    }
}

/// Answers [`PaintRead`]'s reads of the layout tree, its style and what paint preparation keeps
/// beside the rows from the live arena that `$arena` maps the implementing type to: the reads a
/// [`PublishedFrame`] answers from what it published.
macro_rules! read_live_layout_tree {
    ($arena:path) => {
        fn fragment_ownership_filter(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::painting::fragment_ownership::FragmentOwnershipFilter> {
            crate::layout::LayoutNodeArena::fragment_ownership_filter($arena(self), id)
        }

        fn svg_filter_bounds(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::used_values::FfiCssPixelRect> {
            crate::layout::LayoutNodeArena::svg_filter_bounds($arena(self), id)
        }

        fn node_containing_block_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::node_data::NodeSlotId> {
            crate::layout::LayoutNodeArena::node_containing_block_if_live($arena(self), id)
        }

        fn slot_is_live(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::slot_is_live($arena(self), id)
        }

        fn node_style_node(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::css::style::tree::StyleNodeID> {
            crate::layout::LayoutNodeArena::node_style_node($arena(self), id)
        }

        fn node_first_child_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::node_data::NodeSlotId> {
            crate::layout::LayoutNodeArena::node_first_child_if_live($arena(self), id)
        }

        fn node_next_sibling_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::node_data::NodeSlotId> {
            crate::layout::LayoutNodeArena::node_next_sibling_if_live($arena(self), id)
        }

        fn node_generated_for(&self, id: crate::layout::node_data::NodeSlotId) -> u8 {
            crate::layout::LayoutNodeArena::node_generated_for($arena(self), id)
        }

        fn node_is_generated_for_pseudo_element(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_generated_for_pseudo_element($arena(self), id)
        }

        fn node_is_dom_backed(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_dom_backed($arena(self), id)
        }

        fn node_is_element_backed(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_element_backed($arena(self), id)
        }

        fn node_is_out_of_flow_if_live(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_out_of_flow_if_live($arena(self), id)
        }

        fn node_is_atomic_inline(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_atomic_inline($arena(self), id)
        }

        fn node_is_positioned(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_positioned($arena(self), id)
        }

        fn node_is_floating(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_floating($arena(self), id)
        }

        fn node_has_dom_paint_fact(
            &self,
            id: crate::layout::node_data::NodeSlotId,
            fact: crate::layout::node_data::DomPaintFact,
        ) -> bool {
            crate::layout::LayoutNodeArena::node_has_dom_paint_fact($arena(self), id, fact)
        }

        fn node_has_compositor_animation_frame(
            &self,
            id: crate::layout::node_data::NodeSlotId,
            kind: crate::layout::node_data::CompositorAnimationFrameKind,
        ) -> bool {
            crate::layout::LayoutNodeArena::node_has_compositor_animation_frame($arena(self), id, kind)
        }

        fn node_style_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::css::computed_value_views::ComputedValuesView<'_>> {
            crate::layout::LayoutNodeArena::node_style_if_live($arena(self), id)
        }
    };
}

pub(crate) use read_live_layout_tree;

/// Answers [`PaintRead`]'s reads of paint facts, side tables and damage from the live arena that
/// `$arena` maps the implementing type to.
macro_rules! read_live_paint_facts {
    ($arena:path) => {
        fn rendered_text(&self, id: crate::layout::node_data::NodeSlotId) -> Option<&crate::layout::RenderedText> {
            crate::layout::LayoutNodeArena::text_content($arena(self), id).map(|content| &**content)
        }

        fn with_grapheme_segmenter<R>(
            &self,
            id: crate::layout::node_data::NodeSlotId,
            read: impl FnOnce(&crate::layout::text_chunker::GraphemeSegmenter) -> R,
        ) -> Option<R> {
            crate::layout::LayoutNodeArena::text_content($arena(self), id)
                .map(|content| read(content.grapheme_segmenter()))
        }

        fn text_fragments(&self, primary: crate::layout::node_data::NodeSlotId) -> crate::layout::TextFragments {
            crate::layout::LayoutNodeArena::text_fragments($arena(self), primary)
        }
    };
}

pub(crate) use read_live_paint_facts;

/// Answers [`PaintRead`]'s reads of the replaced, layer image and SVG paint resource tables from
/// the live arena that `$arena` maps the implementing type to.
macro_rules! read_live_paint_fact_tables {
    ($arena:path) => {
        fn published_svg_filter(
            &self,
            slot: crate::layout::node_data::NodeSlotId,
            kind: crate::painting::svg_paint_resources::SvgPaintResourceKind,
        ) -> Option<std::sync::Arc<crate::painting::svg_paint_resources::PublishedSvgFilter>> {
            crate::layout::LayoutNodeArena::svg_paint_resources($arena(self)).published_filter(slot, kind)
        }

        fn published_svg_paint_server(
            &self,
            slot: crate::layout::node_data::NodeSlotId,
            kind: crate::painting::svg_paint_resources::SvgPaintResourceKind,
        ) -> Option<std::sync::Arc<crate::painting::svg_paint_resources::PublishedSvgPaintServer>> {
            crate::layout::LayoutNodeArena::svg_paint_resources($arena(self)).published_paint_server(slot, kind)
        }

        fn replaced_paint_facts(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::painting::replaced_paint_facts::ReplacedPaintFacts> {
            crate::layout::LayoutNodeArena::replaced_paint_facts($arena(self), id)
        }

        fn layer_image_paint_facts(
            &self,
            id: crate::layout::node_data::NodeSlotId,
            list: crate::painting::host::FfiLayerImageList,
            computed_index: u32,
        ) -> Option<crate::painting::layer_image_paint_facts::LayerImagePaintFacts> {
            crate::layout::LayoutNodeArena::layer_image_paint_facts($arena(self), id, list, computed_index)
        }
    };
}

pub(crate) use read_live_paint_fact_tables;

/// Answers [`PaintRead`]'s reads of paint damage from the live arena that `$arena` maps the
/// implementing type to.
macro_rules! read_live_paint_damage {
    ($arena:path) => {
        fn paint_damage_of_row(
            &self,
            row: crate::layout::node_data::NodeSlotId,
        ) -> crate::painting::record::damage::PaintDamage {
            crate::layout::LayoutNodeArena::paint_damage_of_row($arena(self), row)
        }

        fn damaged_paint_rows(&self) -> Vec<crate::layout::node_data::NodeSlotId> {
            crate::layout::LayoutNodeArena::damaged_paint_rows($arena(self))
        }
    };
}

pub(crate) use read_live_paint_damage;

/// Answers [`PaintRead`]'s reads of the stacking context entry tables and visual context node
/// handles from the live arena that `$arena` maps the implementing type to.
macro_rules! read_live_stacking_context_entries {
    ($arena:path) => {
        fn with_paintable_visual_context_node_handles<R>(
            &self,
            id: crate::layout::node_data::NodeSlotId,
            read: impl FnOnce(&crate::painting::visual_context::BoxVisualContextNodeHandles) -> R,
        ) -> R {
            crate::layout::LayoutNodeArena::with_paintable_visual_context_node_handles($arena(self), id, read)
        }

        fn stacking_context_entries(
            &self,
            root: crate::layout::node_data::NodeSlotId,
        ) -> Option<
            impl std::ops::Deref<Target = crate::painting::stacking_context::entries::StackingContextEntries> + '_,
        > {
            crate::layout::LayoutNodeArena::stacking_context_entries($arena(self), root)
        }
    };
}

pub(crate) use read_live_stacking_context_entries;

/// Answers every [`PaintRead`] read but the rows' from the live arena that `$arena` maps the
/// implementing type to.
macro_rules! read_live_arena {
    ($arena:path) => {
        $crate::painting::published_frame::read_live_layout_tree!($arena);
        $crate::painting::published_frame::read_live_paint_facts!($arena);
        $crate::painting::published_frame::read_live_paint_fact_tables!($arena);
        $crate::painting::published_frame::read_live_paint_damage!($arena);
        $crate::painting::published_frame::read_live_stacking_context_entries!($arena);
    };
}

pub(crate) use read_live_arena;

impl GeometryRead for LayoutNodeArena {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        self.live_paintable_data(id)
    }

    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        LayoutNodeArena::paintable_row_is_populated(self, id)
    }

    fn with_committed_fragment_link<R>(&self, id: NodeSlotId, read: impl FnOnce(Option<&FragmentLink>) -> R) -> R {
        LayoutNodeArena::with_committed_fragment_link(self, id, read)
    }

    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        CommittedSideDataRef::Live(self.live_committed_side_data(id))
    }

    fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<CssPixelRect> {
        LayoutNodeArena::memoized_absolute_rect(self, id)
    }

    fn memoize_absolute_rect(&self, id: NodeSlotId, rect: CssPixelRect) {
        LayoutNodeArena::memoize_absolute_rect(self, id, rect);
    }

    read_live_geometry!(std::convert::identity);
}

impl PaintRead for LayoutNodeArena {
    read_live_arena!(std::convert::identity);
}

static NO_DAMAGE: FrameDamage = FrameDamage::NONE;

/// What the paint side reads a document through: [`PaintRead`] over the rows the document published, and nothing
/// else. It holds no arena, so a read names nothing the rows do not answer.
pub(crate) struct PaintSource<'a> {
    rows: &'a RowSnapshot,
    /// The damage of the frame a recording records; a read outside a frame reads none.
    damage: &'a FrameDamage,
    // The reader's absolute rects, which hold for these rows where they carry their geometry epoch.
    absolute_rects: &'a RefCell<AbsoluteRectMemo>,
}

impl<'a> PaintSource<'a> {
    /// The frame a recording records.
    pub(crate) fn new(frame: &'a PublishedFrame, absolute_rects: &'a RefCell<AbsoluteRectMemo>) -> Self {
        Self {
            rows: &frame.rows,
            damage: &frame.damage,
            absolute_rects,
        }
    }

    /// The rows the document published, outside any frame.
    pub(crate) fn of_rows(rows: &'a RowSnapshot, absolute_rects: &'a RefCell<AbsoluteRectMemo>) -> Self {
        Self {
            rows,
            damage: &NO_DAMAGE,
            absolute_rects,
        }
    }

    /// The rows read.
    pub(crate) fn rows(&self) -> &'a RowSnapshot {
        self.rows
    }
}

impl GeometryRead for PaintSource<'_> {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        self.rows.paintable.paintable_data(id)
    }

    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        self.rows.paintable.paintable_row_is_populated(id)
    }

    fn with_committed_fragment_link<R>(&self, id: NodeSlotId, read: impl FnOnce(Option<&FragmentLink>) -> R) -> R {
        self.rows.paintable.with_committed_fragment_link(id, read)
    }

    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        CommittedSideDataRef::Published(self.rows.paintable.committed_side_data(id))
    }

    fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<CssPixelRect> {
        self.absolute_rects.borrow().get(id, self.rows.geometry_epoch)
    }

    fn memoize_absolute_rect(&self, id: NodeSlotId, rect: CssPixelRect) {
        self.absolute_rects.borrow_mut().set(id, self.rows.geometry_epoch, rect);
    }

    fn node_kind_if_live(&self, id: NodeSlotId) -> Option<NodeKind> {
        self.rows.node(id).map(|node| node.kind)
    }

    fn node_flags_if_live(&self, id: NodeSlotId) -> u32 {
        self.rows.node(id).map_or(0, |node| node.flags)
    }

    fn node_parent_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        self.rows.parent(id)
    }

    fn node_is_fragmented_inline(&self, id: NodeSlotId) -> bool {
        self.rows
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_fragmented_inline(node, style))
    }
}

impl PaintRead for PaintSource<'_> {
    fn slot_is_live(&self, id: NodeSlotId) -> bool {
        self.rows.node(id).is_some()
    }

    fn node_first_child_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        link(self.rows.node(id)?.first_child)
    }

    fn node_next_sibling_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        link(self.rows.node(id)?.next_sibling)
    }

    fn node_generated_for(&self, id: NodeSlotId) -> u8 {
        self.rows.live_node(id).generated_for
    }

    fn node_style_node(&self, id: NodeSlotId) -> Option<crate::css::style::tree::StyleNodeID> {
        self.rows.node(id)?.style_node
    }

    fn node_is_generated_for_pseudo_element(&self, id: NodeSlotId) -> bool {
        self.rows.node(id).is_some_and(|node| node.generated_for != 0)
    }

    fn node_is_dom_backed(&self, id: NodeSlotId) -> bool {
        // A slot that has not been given a shell yet stands for nothing at all, and its flags do
        // not say so.
        self.rows
            .node(id)
            .is_some_and(|node| node.kind != NodeKind::Unset && !node_facts::has_flag(node, NodeFlag::Anonymous))
    }

    fn node_is_element_backed(&self, id: NodeSlotId) -> bool {
        self.node_is_dom_backed(id)
            && self
                .rows
                .node(id)
                .is_some_and(|node| node.kind != NodeKind::Viewport && !node_facts::kind_is_text(node.kind))
    }

    fn node_is_out_of_flow_if_live(&self, id: NodeSlotId) -> bool {
        self.rows
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_out_of_flow(node, style))
    }

    fn node_is_atomic_inline(&self, id: NodeSlotId) -> bool {
        self.rows
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_atomic_inline(node, style))
    }

    fn node_is_positioned(&self, id: NodeSlotId) -> bool {
        self.rows
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_positioned(node, style))
    }

    fn node_is_floating(&self, id: NodeSlotId) -> bool {
        self.rows
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_floating(node, style))
    }

    fn node_has_dom_paint_fact(&self, id: NodeSlotId, fact: DomPaintFact) -> bool {
        self.rows.live_node(id).dom_paint_facts & fact as u8 != 0
    }

    fn node_has_compositor_animation_frame(&self, id: NodeSlotId, kind: CompositorAnimationFrameKind) -> bool {
        self.rows.live_node(id).compositor_animation_frame_kinds & kind as u8 != 0
    }

    fn node_style_if_live(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>> {
        self.rows.style(id)
    }

    fn node_containing_block_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        self.rows.containing_block(id)
    }

    fn svg_filter_bounds(&self, id: NodeSlotId) -> Option<FfiCssPixelRect> {
        self.rows.paintable.committed_side_data(id).svg_filter_bounds
    }

    fn fragment_ownership_filter(&self, id: NodeSlotId) -> Option<FragmentOwnershipFilter> {
        self.rows
            .paintable
            .committed_side_data(id)
            .fragment_ownership
            .as_deref()
            .cloned()
    }

    fn paint_damage_of_row(&self, row: NodeSlotId) -> PaintDamage {
        self.damage.of_row(row)
    }

    fn damaged_paint_rows(&self) -> Vec<NodeSlotId> {
        self.damage.rows()
    }

    fn stacking_context_entries(&self, root: NodeSlotId) -> Option<impl Deref<Target = StackingContextEntries> + '_> {
        self.rows.paintable.stacking_context_entries(root)
    }

    fn with_paintable_visual_context_node_handles<R>(
        &self,
        id: NodeSlotId,
        read: impl FnOnce(&BoxVisualContextNodeHandles) -> R,
    ) -> R {
        read(self.rows.paintable.visual_context_node_handles(id))
    }

    fn published_svg_filter(&self, slot: NodeSlotId, kind: SvgPaintResourceKind) -> Option<Arc<PublishedSvgFilter>> {
        published_filter_in(&self.rows.paint_facts.svg_paint_resources, slot, kind)
    }

    fn published_svg_paint_server(
        &self,
        slot: NodeSlotId,
        kind: SvgPaintResourceKind,
    ) -> Option<Arc<PublishedSvgPaintServer>> {
        published_paint_server_in(&self.rows.paint_facts.svg_paint_resources, slot, kind)
    }

    fn replaced_paint_facts(&self, id: NodeSlotId) -> Option<ReplacedPaintFacts> {
        self.rows.paint_facts.replaced.get(&id).cloned()
    }

    fn layer_image_paint_facts(
        &self,
        id: NodeSlotId,
        list: FfiLayerImageList,
        computed_index: u32,
    ) -> Option<LayerImagePaintFacts> {
        self.rows
            .paint_facts
            .layer_images
            .get(&id)?
            .iter()
            .find(|entry| entry.list == list && entry.computed_index == computed_index)
            .map(|entry| entry.facts.clone())
    }

    fn rendered_text(&self, id: NodeSlotId) -> Option<&RenderedText> {
        self.rows.text(id)?.rendered.as_deref()
    }

    fn with_grapheme_segmenter<R>(&self, id: NodeSlotId, read: impl FnOnce(&GraphemeSegmenter) -> R) -> Option<R> {
        self.rendered_text(id)
            .map(|rendered| read(&GraphemeSegmenter::new(&rendered.text)))
    }

    fn text_fragments(&self, primary: NodeSlotId) -> TextFragments {
        let mut fragments = TextFragments {
            nodes: [NodeSlotId::INVALID; 2],
            length: 0,
        };
        if !self.node_kind_if_live(primary).is_some_and(node_facts::kind_is_text) {
            return fragments;
        }
        if let Some(text) = self.rows.text(primary)
            && self.slot_is_live(text.first_letter)
        {
            fragments.nodes[0] = text.first_letter;
            fragments.length = 1;
        }
        fragments.nodes[fragments.length] = primary;
        fragments.length += 1;
        fragments
    }
}

impl PaintableRowsRead for PaintSource<'_> {
    fn scroll_offset(&self, id: NodeSlotId) -> CssPixelPoint {
        self.rows.paintable.scroll_offsets.offset(id)
    }

    fn unique_node_id(&self, id: NodeSlotId) -> i64 {
        self.rows.paintable.unique_node_id(id)
    }

    fn with_image_map_areas<R>(&self, read: impl FnOnce(&ImageMapAreas) -> R) -> R {
        read(&self.rows.paintable.image_map_areas)
    }
}
