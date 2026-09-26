/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What a document publishes for the paint side to read, and the one trait the paint side reads it
//! through.
//!
//! A [`PublishedFrame`] is immutable: every column in it is a [`ColumnSnapshot`] or an `Arc` the
//! document shares with it, so the document writes its live columns (copying a chunk a frame still
//! shares) while a frame is read. [`PaintRead`] names every read the display list recording makes
//! of a document, so the recording is written against the trait rather than against the live
//! [`LayoutNodeArena`], and the compiler lists what a frame still has to answer.

use crate::cow_column::ColumnSnapshot;
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::css_pixels::CssPixelRect;
use crate::layout::LayoutNodeArena;
use crate::layout::SLOTS_PER_CHUNK;
use crate::layout::fragment_tree::FragmentLink;
use crate::layout::node_data::{
    CompositorAnimationFrameKind, DomPaintFact, FfiStylePayloads, NodeFlag, NodeKind, NodeSlotId, PaintNode,
};
use crate::layout::node_facts;
use crate::layout::tree_shape::RetiredSlots;
use crate::layout::used_values::FfiCssPixelRect;
use crate::layout::{RenderedTextBoundary, TextContent, TextFragments};
use crate::painting::fragment_ownership::FragmentOwnershipFilter;
use crate::painting::hit_test::HitTestList;
use crate::painting::host::FfiLayerImageList;
use crate::painting::image_map_areas::ImageMapAreas;
use crate::painting::layer_image_paint_facts::LayerImagePaintFacts;
use crate::painting::paint_order_plan::PaintOrderInputs;
use crate::painting::paintable_data::{CommittedSideData, PaintableData};
use crate::painting::paintable_rows::{CommittedFragmentLinkSlot, CommittedSideDataRef, PAINTABLE_SLOTS_PER_CHUNK};
use crate::painting::record::damage::PaintDamage;
use crate::painting::replaced_paint_facts::ReplacedPaintFacts;
use crate::painting::stacking_context::entries::StackingContextEntries;
use crate::painting::svg_paint_resources::SvgPaintResources;
use crate::painting::visual_context::scroll_state::ScrollOffsets;
use crate::painting::visual_context::{BoxVisualContextNodeHandles, VisualContextTree};
use std::ops::Deref;
use std::sync::Arc;

/// One published generation of a document's paintable rows and of the columns read beside them.
#[derive(Clone)]
pub(crate) struct PublishedRows {
    pub(super) rows: ColumnSnapshot<PaintableData, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) fragment_links: ColumnSnapshot<CommittedFragmentLinkSlot, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) side_data: ColumnSnapshot<CommittedSideData, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) unique_node_ids: ColumnSnapshot<(NodeSlotId, i64), PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) scroll_offsets: Arc<ScrollOffsets>,
    pub(super) image_map_areas: Arc<ImageMapAreas>,
    pub(super) hit_test_list: Option<Arc<HitTestList>>,
    pub(super) visual_context_tree: Option<Arc<VisualContextTree>>,
}

/// What a document published for one recording to read: its rows, and what the paint side reads of
/// its layout nodes.
pub(crate) struct PublishedFrame {
    pub(super) rows: PublishedRows,
    nodes: ColumnSnapshot<PaintNode, SLOTS_PER_CHUNK>,
    /// Keeps the arena from reusing a slot this frame may name until the frame is dropped.
    _retired_slots: RetiredSlots,
}

// A frame is read on whichever thread paints it while the document writes its live columns: it
// holds no cell, no raw pointer and no borrow of the document.
const _: () = {
    const fn assert_published<T: Send + Sync + 'static>() {}
    assert_published::<PublishedFrame>();
};

impl PublishedFrame {
    pub(crate) fn new(
        rows: PublishedRows,
        nodes: ColumnSnapshot<PaintNode, SLOTS_PER_CHUNK>,
        retired_slots: RetiredSlots,
    ) -> Self {
        Self {
            rows,
            nodes,
            _retired_slots: retired_slots,
        }
    }

    /// The node in a live slot, as the frame published it.
    #[inline]
    fn node(&self, id: NodeSlotId) -> Option<&PaintNode> {
        if id.is_invalid() {
            return None;
        }
        self.nodes
            .get(id.slot_index() as usize)
            .filter(|node| node.generation != 0 && node.generation == id.generation())
    }

    fn node_and_style(&self, id: NodeSlotId) -> Option<(&PaintNode, Option<ComputedValuesView<'_>>)> {
        let node = self.node(id)?;
        Some((node, Self::style_of(node)))
    }

    fn style_of(node: &PaintNode) -> Option<ComputedValuesView<'_>> {
        if node.style.is_null() {
            return None;
        }
        // SAFETY: A non-null style pointer addresses a style container's group pointer array,
        // which FfiStylePayloads mirrors exactly. The document reclaims no style record a frame
        // it published may name until it has taken that frame back (hold_style_records_for_frame).
        let payloads = unsafe { &*node.style.as_ptr().cast::<FfiStylePayloads>() };
        Some(ComputedValuesView::new(&payloads.groups))
    }

    /// The node in a slot a read requires to be live.
    #[track_caller]
    fn live_node(&self, id: NodeSlotId) -> &PaintNode {
        assert!(!id.is_invalid(), "invalid layout node arena slot ID");
        self.node(id).expect("layout node arena read a stale or unused slot")
    }

    fn link(node: NodeSlotId) -> Option<NodeSlotId> {
        (!node.is_invalid()).then_some(node)
    }
}

impl PublishedRows {
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

    pub(crate) fn committed_side_data(&self, id: NodeSlotId) -> &CommittedSideData {
        debug_assert!(self.paintable_row_is_populated(id));
        self.side_data
            .get(id.slot_index() as usize)
            .expect("a populated row has published side data")
    }
}

/// The box whose content box a node is laid out against, found by walking its ancestors as the
/// arena's own walk does (`containing_block_by_walking_ancestors`), from what a [`PaintRead`] reads.
fn containing_block_by_walking_ancestors(read: &impl PaintRead, node: NodeSlotId) -> Option<NodeSlotId> {
    use crate::css::css_enums::positioning;
    let kind = read.node_kind_if_live(node)?;
    let position = if node_facts::kind_is_text(kind) {
        positioning::STATIC
    } else {
        crate::painting::style_queries::position(read, node)
    };
    if position != positioning::ABSOLUTE && position != positioning::FIXED {
        let mut ancestor = read.node_parent_if_live(node);
        while let Some(candidate) = ancestor {
            let shape = (read.node_kind_if_live(candidate)?, read.node_flags_if_live(candidate));
            if node_facts::node_forms_containing_block_for_children(&shape, read.node_style_if_live(candidate)) {
                return Some(candidate);
            }
            ancestor = read.node_parent_if_live(candidate);
        }
        return None;
    }
    let is_fixed_position = position == positioning::FIXED;
    let establishes_containing_block = node_facts::containing_block_establishment_flag(is_fixed_position) as u32;
    let mut current = node;
    while let Some(ancestor) = read.node_parent_if_live(current) {
        current = ancestor;
        if read.node_kind_if_live(current).is_some_and(node_facts::kind_is_box)
            && read.node_flags_if_live(current) & establishes_containing_block != 0
        {
            return Some(current);
        }
    }
    // A fixed-position box with no ancestor establishing its containing block is laid out against
    // the root.
    is_fixed_position.then_some(current)
}

/// The reads the display list recording makes of a document. The live arena answers them from the
/// columns layout writes; a [`PublishedFrame`] answers them from what the document published.
pub(crate) trait PaintRead: Sized {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData;
    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool;
    /// Reads the fragment link a populated row committed, from the same generation as its row.
    fn with_committed_fragment_link<R>(&self, id: NodeSlotId, read: impl FnOnce(Option<&FragmentLink>) -> R) -> R;
    /// The side data a populated row committed, from the same generation as its row.
    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_>;
    fn with_paintable_visual_context_node_handles<R>(
        &self,
        id: NodeSlotId,
        read: impl FnOnce(&BoxVisualContextNodeHandles) -> R,
    ) -> R;

    fn slot_is_live(&self, id: NodeSlotId) -> bool;
    fn node_kind_if_live(&self, id: NodeSlotId) -> Option<NodeKind>;
    fn node_flags_if_live(&self, id: NodeSlotId) -> u32;
    fn node_parent_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_first_child_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_next_sibling_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_containing_block_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_generated_for(&self, id: NodeSlotId) -> u8;
    fn node_is_generated_for_pseudo_element(&self, id: NodeSlotId) -> bool;
    fn node_is_dom_backed(&self, id: NodeSlotId) -> bool;
    fn node_is_element_backed(&self, id: NodeSlotId) -> bool;
    fn node_is_out_of_flow_if_live(&self, id: NodeSlotId) -> bool;
    fn node_is_fragmented_inline(&self, id: NodeSlotId) -> bool;
    fn node_is_atomic_inline(&self, id: NodeSlotId) -> bool;
    fn node_is_positioned(&self, id: NodeSlotId) -> bool;
    fn node_is_floating(&self, id: NodeSlotId) -> bool;
    fn node_has_dom_paint_fact(&self, id: NodeSlotId, fact: DomPaintFact) -> bool;
    fn node_has_compositor_animation_frame(&self, id: NodeSlotId, kind: CompositorAnimationFrameKind) -> bool;
    fn node_style_if_live(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>>;
    /// The rendered text of a text row.
    fn text_content(&self, id: NodeSlotId) -> Option<&TextContent>;
    /// The rows a text node is painted in: its first-letter row, if any, then its own.
    fn text_fragments(&self, primary: NodeSlotId) -> TextFragments;
    fn svg_paint_resources(&self) -> &SvgPaintResources;
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
    /// The paint-order inputs the row's paint state prepared, if it prepared them.
    fn prepared_paint_order_inputs(&self, row: NodeSlotId) -> Option<PaintOrderInputs>;
    fn stacking_context_entries(&self, root: NodeSlotId) -> Option<impl Deref<Target = StackingContextEntries> + '_>;
    fn damaged_paint_rows(&self) -> Vec<NodeSlotId>;

    /// The absolute rect memoized for a box, if any.
    fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<CssPixelRect>;
    fn memoize_absolute_rect(&self, id: NodeSlotId, rect: CssPixelRect);

    fn dom_offset_for_rendered_text_offset(
        &self,
        id: NodeSlotId,
        offset: usize,
        boundary: RenderedTextBoundary,
    ) -> usize {
        if !self.node_kind_if_live(id).is_some_and(node_facts::kind_is_text) {
            return offset;
        }
        self.text_content(id)
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
        self.text_content(id)
            .expect("text must be published before mapping DOM offsets")
            .rendered_text_offset_for_dom_offset(offset, boundary)
    }

    /// The line root whose committed side data holds an inline box's pieces, read from the same
    /// generation as the rows.
    fn inline_pieces_root(&self, inline_paintable: NodeSlotId) -> Option<NodeSlotId> {
        if !self.paintable_row_is_populated(inline_paintable) {
            return None;
        }
        let root = self.paintable_data(inline_paintable).containing_block;
        (self.paintable_row_is_populated(root) && crate::painting::node_painting::has_lines(self, root)).then_some(root)
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
            current = self.node_parent_if_live(current).unwrap_or(NodeSlotId::INVALID);
            while current != root {
                if let Some(next_sibling) = self.node_next_sibling_if_live(current) {
                    current = next_sibling;
                    break;
                }
                current = self.node_parent_if_live(current).unwrap_or(NodeSlotId::INVALID);
            }
            if current == root {
                break;
            }
        }
    }
}

/// Answers [`PaintRead`]'s reads of the layout tree and its style from the live arena that `$arena`
/// maps the implementing type to.
macro_rules! read_live_layout_tree {
    ($arena:path) => {
        fn node_containing_block_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::node_data::NodeSlotId> {
            crate::layout::LayoutNodeArena::node_containing_block_if_live($arena(self), id)
        }

        fn slot_is_live(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::slot_is_live($arena(self), id)
        }

        fn node_kind_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::node_data::NodeKind> {
            crate::layout::LayoutNodeArena::node_kind_if_live($arena(self), id)
        }

        fn node_flags_if_live(&self, id: crate::layout::node_data::NodeSlotId) -> u32 {
            crate::layout::LayoutNodeArena::node_flags_if_live($arena(self), id)
        }

        fn node_parent_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::node_data::NodeSlotId> {
            crate::layout::LayoutNodeArena::node_parent_if_live($arena(self), id)
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

        fn node_is_fragmented_inline(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_fragmented_inline($arena(self), id)
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
        fn with_paintable_visual_context_node_handles<R>(
            &self,
            id: crate::layout::node_data::NodeSlotId,
            read: impl FnOnce(&crate::painting::visual_context::BoxVisualContextNodeHandles) -> R,
        ) -> R {
            crate::layout::LayoutNodeArena::with_paintable_visual_context_node_handles($arena(self), id, read)
        }

        fn text_content(&self, id: crate::layout::node_data::NodeSlotId) -> Option<&crate::layout::TextContent> {
            crate::layout::LayoutNodeArena::text_content($arena(self), id)
        }

        fn text_fragments(&self, primary: crate::layout::node_data::NodeSlotId) -> crate::layout::TextFragments {
            crate::layout::LayoutNodeArena::text_fragments($arena(self), primary)
        }

        fn svg_paint_resources(&self) -> &crate::painting::svg_paint_resources::SvgPaintResources {
            crate::layout::LayoutNodeArena::svg_paint_resources($arena(self))
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

        fn svg_filter_bounds(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::used_values::FfiCssPixelRect> {
            crate::layout::LayoutNodeArena::svg_filter_bounds($arena(self), id)
        }

        fn fragment_ownership_filter(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::painting::fragment_ownership::FragmentOwnershipFilter> {
            crate::layout::LayoutNodeArena::fragment_ownership_filter($arena(self), id)
        }

        fn paint_damage_of_row(
            &self,
            row: crate::layout::node_data::NodeSlotId,
        ) -> crate::painting::record::damage::PaintDamage {
            crate::layout::LayoutNodeArena::paint_damage_of_row($arena(self), row)
        }

        fn damaged_paint_rows(&self) -> Vec<crate::layout::node_data::NodeSlotId> {
            crate::layout::LayoutNodeArena::damaged_paint_rows($arena(self))
        }

        fn prepared_paint_order_inputs(
            &self,
            row: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::painting::paint_order_plan::PaintOrderInputs> {
            crate::layout::LayoutNodeArena::row_paint_state($arena(self), row).order_inputs()
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

pub(crate) use read_live_paint_facts;

/// Answers every [`PaintRead`] read but the rows' from the live arena that `$arena` maps the
/// implementing type to.
macro_rules! read_live_arena {
    ($arena:path) => {
        $crate::painting::published_frame::read_live_layout_tree!($arena);
        $crate::painting::published_frame::read_live_paint_facts!($arena);
    };
}

pub(crate) use read_live_arena;

impl PaintRead for LayoutNodeArena {
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

    read_live_arena!(std::convert::identity);
}

/// What a display list recording reads its document through: [`PaintRead`] and nothing else. It
/// does not dereference to the arena, so the recording names no read the trait does not.
pub(crate) struct PaintSource<'a> {
    arena: &'a LayoutNodeArena,
    frame: &'a PublishedFrame,
}

impl<'a> PaintSource<'a> {
    pub(crate) fn new(arena: &'a LayoutNodeArena, frame: &'a PublishedFrame) -> Self {
        Self { arena, frame }
    }

    fn arena(&self) -> &'a LayoutNodeArena {
        self.arena
    }
}

impl PaintRead for PaintSource<'_> {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        self.frame.rows.paintable_data(id)
    }

    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        self.frame.rows.paintable_row_is_populated(id)
    }

    fn with_committed_fragment_link<R>(&self, id: NodeSlotId, read: impl FnOnce(Option<&FragmentLink>) -> R) -> R {
        self.frame.rows.with_committed_fragment_link(id, read)
    }

    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        CommittedSideDataRef::Published(self.frame.rows.committed_side_data(id))
    }

    fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<CssPixelRect> {
        self.arena.memoized_absolute_rect(id)
    }

    fn memoize_absolute_rect(&self, id: NodeSlotId, rect: CssPixelRect) {
        self.arena.memoize_absolute_rect(id, rect);
    }

    fn slot_is_live(&self, id: NodeSlotId) -> bool {
        self.frame.node(id).is_some()
    }

    fn node_kind_if_live(&self, id: NodeSlotId) -> Option<NodeKind> {
        self.frame.node(id).map(|node| node.kind)
    }

    fn node_flags_if_live(&self, id: NodeSlotId) -> u32 {
        self.frame.node(id).map_or(0, |node| node.flags)
    }

    fn node_parent_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        PublishedFrame::link(self.frame.node(id)?.parent)
    }

    fn node_first_child_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        PublishedFrame::link(self.frame.node(id)?.first_child)
    }

    fn node_next_sibling_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        PublishedFrame::link(self.frame.node(id)?.next_sibling)
    }

    fn node_generated_for(&self, id: NodeSlotId) -> u8 {
        self.frame.live_node(id).generated_for
    }

    fn node_is_generated_for_pseudo_element(&self, id: NodeSlotId) -> bool {
        self.frame.node(id).is_some_and(|node| node.generated_for != 0)
    }

    fn node_is_dom_backed(&self, id: NodeSlotId) -> bool {
        // A slot that has not been given a shell yet stands for nothing at all, and its flags do
        // not say so.
        self.frame
            .node(id)
            .is_some_and(|node| node.kind != NodeKind::Unset && !node_facts::has_flag(node, NodeFlag::Anonymous))
    }

    fn node_is_element_backed(&self, id: NodeSlotId) -> bool {
        self.node_is_dom_backed(id)
            && self
                .frame
                .node(id)
                .is_some_and(|node| node.kind != NodeKind::Viewport && !node_facts::kind_is_text(node.kind))
    }

    fn node_is_out_of_flow_if_live(&self, id: NodeSlotId) -> bool {
        self.frame
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_out_of_flow(node, style))
    }

    fn node_is_fragmented_inline(&self, id: NodeSlotId) -> bool {
        self.frame
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_fragmented_inline(node, style))
    }

    fn node_is_atomic_inline(&self, id: NodeSlotId) -> bool {
        self.frame
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_atomic_inline(node, style))
    }

    fn node_is_positioned(&self, id: NodeSlotId) -> bool {
        self.frame
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_positioned(node, style))
    }

    fn node_is_floating(&self, id: NodeSlotId) -> bool {
        self.frame
            .node_and_style(id)
            .is_some_and(|(node, style)| node_facts::node_is_floating(node, style))
    }

    fn node_has_dom_paint_fact(&self, id: NodeSlotId, fact: DomPaintFact) -> bool {
        self.frame.live_node(id).dom_paint_facts & fact as u8 != 0
    }

    fn node_has_compositor_animation_frame(&self, id: NodeSlotId, kind: CompositorAnimationFrameKind) -> bool {
        self.frame.live_node(id).compositor_animation_frame_kinds & kind as u8 != 0
    }

    fn node_style_if_live(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>> {
        PublishedFrame::style_of(self.frame.node(id)?)
    }

    fn node_containing_block_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        containing_block_by_walking_ancestors(self, id)
    }

    read_live_paint_facts!(PaintSource::arena);
}
