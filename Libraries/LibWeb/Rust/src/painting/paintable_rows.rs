/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::cow_column::CowColumn;
use crate::css::css_pixels::{CssPixelPoint, CssPixelRect};
use crate::layout::LayoutNodeArena;
use crate::layout::node_data::{NodeFlag, NodeSlotId};
use crate::layout::{fragment_tree, used_values};
use crate::painting::hit_test::HitTestList;
use crate::painting::image_map_areas::{ImageMapAreaColumn, ImageMapAreas};
use crate::painting::node_painting;
use crate::painting::paintable_data::*;
use crate::painting::published_frame::{PaintRead, PublishedFrame, PublishedRows, read_live_arena};
use crate::painting::record::damage::{DamageSet, PaintDamage, RowPaintState};
use crate::painting::visual_context::dirty::{
    RemovedBoxBlocks, VisualContextBoxDirtyKind, VisualContextGlobalRebuildReason,
};
use crate::painting::visual_context::scroll_state::ScrollOffsetColumn;
use crate::painting::visual_context::{
    BoxVisualContextNodeHandles, EMPTY_BOX_VISUAL_CONTEXT_NODE_HANDLES, PaintableVisualContextRecord, VisualContextTree,
};
use smallvec::{SmallVec, smallvec};
use std::cell::{Cell, Ref, RefCell, RefMut};
use std::ffi::c_void;
use std::ops::{Deref, DerefMut};

pub(crate) const PAINTABLE_SLOTS_PER_CHUNK: usize = 64;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg(target_pointer_width = "64")]
    fn committed_geometry_validity_fits_in_the_link_slots_existing_padding() {
        assert_eq!(std::mem::size_of::<CommittedFragmentLinkSlot>(), 16);
    }

    #[test]
    fn committed_rows_keep_what_was_published_until_the_main_side_reads_them() {
        use crate::css::css_pixels::CssPixels;

        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.populate_paintable_row(node);
        arena.paintable_rows_mut().paintable_data_mut(node).offset.x = CssPixels::from_integer(10);
        arena.publish_paintable_rows();
        arena.paintable_rows_mut().paintable_data_mut(node).offset.x = CssPixels::from_integer(20);

        let published = arena.paintable_rows.published.as_ref().unwrap().rows.clone();
        let published_offset = |rows: &crate::cow_column::ColumnSnapshot<PaintableData, PAINTABLE_SLOTS_PER_CHUNK>| {
            rows.get(node.slot_index() as usize).unwrap().offset.x
        };
        assert_eq!(published_offset(&published), CssPixels::from_integer(10).into());
        assert_eq!(
            arena.committed_paintable_rows().paintable_data(node).offset.x,
            CssPixels::from_integer(20).into()
        );
        assert_eq!(published_offset(&published), CssPixels::from_integer(10).into());
    }

    #[test]
    fn columns_read_beside_the_rows_are_published_with_them() {
        use crate::css::css_pixels::{CssPixelPoint, CssPixels};
        use crate::painting::image_map_areas::{AreaShape, PublishedImageMapArea};

        let area = |style_node| {
            Box::new([PublishedImageMapArea {
                style_node,
                shape: AreaShape::Default,
                editable: false,
                coords: Box::new([]),
            }]) as Box<[_]>
        };
        let offset = |x| CssPixelPoint::new(CssPixels::from_integer(x), CssPixels::from_integer(0));
        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.populate_paintable_row(node);
        arena.scroll_offsets().publish(node, offset(10));
        arena.unique_node_ids().publish(node, 1);
        arena.image_map_areas().publish(node, area(1));
        arena.publish_paintable_rows();
        arena.scroll_offsets().publish(node, offset(20));
        arena.unique_node_ids().publish(node, 2);
        arena.image_map_areas().publish(node, area(2));

        let published = arena.paintable_rows.published.as_ref().unwrap();
        let (scroll_offsets, image_map_areas) = (published.scroll_offsets.clone(), published.image_map_areas.clone());
        let unique_node_ids = published.unique_node_ids.clone();
        let published_state = || {
            (
                scroll_offsets.offset(node),
                unique_node_id_of(unique_node_ids.get(node.slot_index() as usize), node),
                image_map_areas.area_editability(node, 1),
            )
        };
        assert_eq!(published_state(), (offset(10), 1, 0));
        let committed = arena.committed_paintable_rows();
        assert_eq!(
            (
                committed.scroll_offset(node),
                committed.unique_node_id(node),
                committed.with_image_map_areas(|areas| areas.area_editability(node, 1)),
            ),
            (offset(20), 2, -1)
        );
        assert_eq!(published_state(), (offset(10), 1, 0));
    }

    #[test]
    fn side_data_is_published_with_the_rows() {
        use crate::layout::inline_content::InlineContent;

        let content = |line_count| {
            std::sync::Arc::new(InlineContent {
                lines: vec![Default::default(); line_count],
                ..Default::default()
            })
        };
        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.populate_paintable_row(node);
        arena.committed_side_data_mut(node).inline_content = Some(content(1));
        arena.committed_side_data_mut(node).piece_indices = Some([0].into());
        arena.committed_side_data_mut(node).overflow_valid_across_recommits = true;
        arena.publish_paintable_rows();
        arena.paintable_rows_mut().begin_paintable_row_recommit(node);
        arena.committed_side_data_mut(node).inline_content = Some(content(2));
        arena.paintable_rows().clear_cached_overflow_data(node);

        let side_data = arena.paintable_rows.published.as_ref().unwrap().side_data.clone();
        let published_state = || {
            let side_data = side_data.get(node.slot_index() as usize).unwrap();
            (
                side_data.lines().len(),
                side_data.piece_indices().len(),
                side_data.overflow_valid_across_recommits,
            )
        };
        assert_eq!(published_state(), (1, 1, true));
        let live = arena.paintable_rows();
        let live = live.committed_side_data(node);
        assert_eq!(
            (
                live.lines().len(),
                live.piece_indices().len(),
                live.overflow_valid_across_recommits
            ),
            (2, 0, false)
        );
        drop(live);
        let committed = arena.committed_paintable_rows();
        assert_eq!(committed.committed_side_data(node).lines().len(), 2);
        assert_eq!(published_state(), (1, 1, true));
    }

    #[test]
    fn hit_test_list_and_visual_context_tree_are_published_with_the_rows() {
        use crate::painting::hit_test::HitTestList;
        use crate::painting::visual_context::{TransformData, TransformDataRole};
        use std::sync::Arc;

        let list = |generation| {
            Some(Arc::new(HitTestList {
                generation,
                ..Default::default()
            }))
        };
        let tree = || {
            Some(Arc::new(VisualContextTree::create(TransformData {
                matrix: libgfx_rust::FloatMatrix4x4::identity(),
                origin: Default::default(),
                sorting_context_root_index: None,
                flattens_inherited_transform: false,
                role: TransformDataRole::CssTransform,
                synthetic_plane: false,
                establishes_sorting_context: false,
            })))
        };
        let mut arena = LayoutNodeArena::new();
        *arena.hit_test_list.get_mut() = list(1);
        let published_tree = tree();
        arena.paint_state().borrow_mut().visual_context.tree = published_tree.clone();
        arena.publish_paintable_rows();
        *arena.hit_test_list.get_mut() = list(2);
        arena.paint_state().borrow_mut().visual_context.tree = tree();

        let published = arena.paintable_rows.published.as_ref().unwrap();
        assert_eq!(published.hit_test_list.as_ref().map(|list| list.generation), Some(1));
        assert!(Arc::ptr_eq(
            published.visual_context_tree.as_ref().unwrap(),
            published_tree.as_ref().unwrap()
        ));
        let committed = arena.committed_paintable_rows();
        assert_eq!(
            committed.with_hit_test_list(|list| list.map(|list| list.generation)),
            Some(2)
        );
        assert!(!Arc::ptr_eq(
            &committed.visual_context_tree().unwrap(),
            published_tree.as_ref().unwrap()
        ));
    }

    #[test]
    fn overflow_queries_do_not_measure_ordinary_inline_fragments() {
        use crate::css::css_pixels::{CssPixelRect, CssPixels};
        use crate::layout::node_data::NodeKind;
        use crate::painting::paintable_geometry;

        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.write_shape(node).set_kind(NodeKind::InlineNode);
        arena.populate_paintable_row(node);
        arena.scrollable_overflow.viewport.set(Some(node));
        let rect = CssPixelRect::new(
            CssPixels::from_integer(0),
            CssPixels::from_integer(0),
            CssPixels::from_integer(100),
            CssPixels::from_integer(80),
        );
        arena
            .paintable_rows_mut()
            .paintable_data_mut(node)
            .local_padding_box_union = rect.into();

        let rows = arena.paintable_rows();
        assert_eq!(paintable_geometry::scrollable_overflow_rect(&rows, node), None);
        assert!(!paintable_geometry::has_scrollable_overflow(&rows, node));
        assert!(!arena.paintable_side_data(node).overflow_measured_this_commit.get());
    }

    #[test]
    fn inline_that_starts_storing_a_scroll_offset_is_measured_before_publication() {
        use crate::css::css_pixels::{CssPixelRect, CssPixels};
        use crate::layout::node_data::NodeKind;
        use crate::painting::paintable_geometry;

        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.write_shape(node).set_kind(NodeKind::InlineNode);
        arena.populate_paintable_row(node);
        arena.scrollable_overflow.viewport.set(Some(node));
        let rect = CssPixelRect::new(
            CssPixels::from_integer(0),
            CssPixels::from_integer(0),
            CssPixels::from_integer(100),
            CssPixels::from_integer(80),
        );
        arena
            .paintable_rows_mut()
            .paintable_data_mut(node)
            .local_padding_box_union = rect.into();
        arena.measure_scrollable_overflow_before_publication();
        assert_eq!(
            paintable_geometry::scrollable_overflow_rect(&arena.paintable_rows(), node),
            None
        );

        arena.set_node_flag(node, NodeFlag::HasScrollOffset, true);
        arena.measure_scrollable_overflow_before_publication();
        assert_eq!(
            paintable_geometry::scrollable_overflow_rect(&arena.paintable_rows(), node),
            Some(rect)
        );
    }

    #[test]
    fn overflow_is_measured_before_publication_while_geometry_is_borrowed() {
        use crate::css::css_pixels::{CssPixelRect, CssPixels};
        use crate::layout::node_data::NodeKind;
        use crate::painting::paintable_geometry;

        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.write_shape(node).set_kind(NodeKind::InlineNode);
        arena.set_node_flag(node, NodeFlag::HasScrollOffset, true);
        arena.populate_paintable_row(node);
        arena.scrollable_overflow.viewport.set(Some(node));
        let rect = CssPixelRect::new(
            CssPixels::from_integer(0),
            CssPixels::from_integer(0),
            CssPixels::from_integer(100),
            CssPixels::from_integer(80),
        );
        arena
            .paintable_rows_mut()
            .paintable_data_mut(node)
            .local_padding_box_union = rect.into();
        let mut cache = arena.committed_side_data_mut(node);
        cache.overflow_relative_to_padding_box = FfiOverflowData {
            rect: CssPixelRect::new(rect.x, rect.y, CssPixels::from_integer(500), rect.height).into(),
            has_scrollable_overflow: true,
        };
        cache.overflow_valid_across_recommits = true;
        drop(cache);
        arena.paintable_side_data(node).overflow_measured_this_commit.set(true);

        let rows = arena.paintable_rows();
        let geometry = rows.paintable_data(node);
        let previous_geometry = *geometry;
        rows.clear_cached_overflow_data(node);
        // Reading overflow never measures it.
        assert_eq!(paintable_geometry::scrollable_overflow_rect(&rows, node), None);
        assert!(!arena.scrollable_overflow.geometry_changed.get());
        arena.measure_scrollable_overflow_before_publication();
        assert_eq!(paintable_geometry::scrollable_overflow_rect(&rows, node), Some(rect));
        assert!(!paintable_geometry::has_scrollable_overflow(&rows, node));
        assert_eq!(*geometry, previous_geometry);
        assert!(arena.scrollable_overflow.geometry_changed.get());
        assert!(arena.scrollable_overflow.scrollability_changed.get());
    }

    #[test]
    fn overflow_cache_invalidation_is_independent_of_borrowed_geometry() {
        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.populate_paintable_row(node);
        arena.committed_side_data_mut(node).overflow_valid_across_recommits = true;
        arena.paintable_side_data(node).overflow_measured_this_commit.set(true);
        arena.paintable_rows_mut().begin_paintable_row_recommit(node);
        assert!(arena.live_committed_side_data(node).overflow_valid_across_recommits);

        let rows = arena.paintable_rows();
        let geometry = rows.paintable_data(node);
        let previous_geometry = *geometry;
        rows.clear_cached_overflow_data(node);
        assert_eq!(*geometry, previous_geometry);
        assert!(!arena.live_committed_side_data(node).overflow_valid_across_recommits);
        assert!(!arena.paintable_side_data(node).overflow_measured_this_commit.get());
    }

    #[test]
    fn row_reset_version_changes_for_each_kind_of_reset() {
        let mut arena = LayoutNodeArena::new();
        let node = arena.allocate_for_test().slot;
        arena.populate_paintable_row(node);
        let initial_version = arena.paintable_rows().paintable_row_reset_version(node);

        arena.paintable_rows_mut().begin_paintable_row_recommit(node);
        let recommitted_version = arena.paintable_rows().paintable_row_reset_version(node);
        assert_eq!(recommitted_version, initial_version + 1);

        let reset = arena.prepare_paintable_row_cleared_reset(node).unwrap();
        arena.paintable_row_cleared(reset);
        let cleared_version = arena.paintable_rows().paintable_row_reset_version(node);
        assert_eq!(cleared_version, recommitted_version + 1);

        arena.populate_paintable_row(node);
        let reset = arena.prepare_paintable_row_freed_reset(node.slot_index()).unwrap();
        arena.paintable_row_freed(reset);
        assert_eq!(
            arena.paintable_rows().paintable_row_reset_version(node),
            cleared_version + 1
        );
    }
}

/// The document's callback for a reset row: the row, how it was reset, and whether it is the row the
/// document's viewport is bound to.
pub(crate) type ChromeStateCallback = (
    *mut c_void,
    unsafe extern "C" fn(*mut c_void, NodeSlotId, PaintableRowResetKind, bool),
);

#[derive(Clone, Copy)]
pub(crate) struct PaintableRowReset {
    slot: NodeSlotId,
    kind: PaintableRowResetKind,
    notifies_chrome_state: bool,
}

impl PaintableRowReset {
    /// Tells the document about the reset, where `viewport_row` is the row its viewport is bound to
    /// now, which the arena knows without the document asking it once for every reset row.
    pub(crate) fn invoke_callback_on_main_thread(
        self,
        main_thread: &crate::stage::MainThread,
        viewport_row: NodeSlotId,
    ) {
        if !self.notifies_chrome_state {
            return;
        }
        if let Some((context, callback)) = main_thread
            .host_tables()
            .and_then(|host_tables| host_tables.chrome_state_callback.get())
        {
            // SAFETY: Registration and unregistration keep the callback context live.
            unsafe { callback(context, self.slot, self.kind, self.slot == viewport_row) };
        }
    }
}

// The unique node id of what each box is the box of, as the document names it: an element, the
// element a pseudo-element was generated for, or the document itself for the viewport. It is the
// name the compositor scrolls and snaps by, and it is not the style node id - it outlives a style
// tree - so the document publishes it as a box is bound to what it is the box of.
//
// Dense by slot, because nearly every element box has one. Each entry names the row that published
// it, so a slot that has been recycled since answers for the new row and not the old one.
#[derive(Default)]
pub(crate) struct UniqueNodeIdColumn {
    ids: RefCell<CowColumn<(NodeSlotId, i64), PAINTABLE_SLOTS_PER_CHUNK>>,
}

fn unique_node_id_of(entry: Option<&(NodeSlotId, i64)>, slot: NodeSlotId) -> i64 {
    match entry {
        Some(&(published_for, id)) if !slot.is_invalid() && published_for == slot => id,
        _ => 0,
    }
}

impl UniqueNodeIdColumn {
    pub(crate) fn id(&self, slot: NodeSlotId) -> i64 {
        unique_node_id_of(self.ids.borrow().get(slot.slot_index() as usize), slot)
    }

    pub(crate) fn publish(&self, slot: NodeSlotId, id: i64) {
        if slot.is_invalid() || self.id(slot) == id {
            return;
        }
        let index = slot.slot_index() as usize;
        let mut ids = self.ids.borrow_mut();
        ids.grow_to(index + 1);
        *ids.get_mut(index).expect("the column grew to hold the slot") = (slot, id);
    }

    pub(crate) fn forget(&self, slot: NodeSlotId) {
        if slot.is_invalid() || self.id(slot) == 0 {
            return;
        }
        if let Some(entry) = self.ids.borrow_mut().get_mut(slot.slot_index() as usize) {
            *entry = (NodeSlotId::INVALID, 0);
        }
    }
}

/// The fragment link a layout slot committed. A link is shared, so a chunk a published generation
/// still holds is copied one reference count per slot.
#[derive(Clone, Default)]
pub(crate) struct CommittedFragmentLinkSlot {
    layout_slot_generation: u8,
    geometry_epoch: u32,
    geometry_is_current: bool,
    link: Option<std::sync::Arc<fragment_tree::FragmentLink>>,
}

impl CommittedFragmentLinkSlot {
    pub(crate) fn link(&self) -> Option<&fragment_tree::FragmentLink> {
        self.link.as_deref()
    }

    fn link_for(&self, layout_slot_generation: u8) -> Option<&fragment_tree::FragmentLink> {
        (self.layout_slot_generation == layout_slot_generation)
            .then_some(self.link.as_deref())
            .flatten()
    }
}

#[derive(Default)]
pub(crate) struct PaintableRowStore {
    rows: CowColumn<PaintableData, PAINTABLE_SLOTS_PER_CHUNK>,
    /// The rows as last published, for the main side to read, sharing unchanged chunks with
    /// `rows`. The layout commit and the visual context update release it when they start, since
    /// nothing reads it while they run and releasing it lets them write chunks in place, and
    /// publish again when they are done. A row a main-side writer changes is published when the
    /// main side next reads the rows.
    published: Option<PublishedRows>,
    side_data: RefCell<Vec<PaintableSideData>>,
    committed_side_data: RefCell<CowColumn<CommittedSideData, PAINTABLE_SLOTS_PER_CHUNK>>,
    row_reset_versions: Vec<u64>,
    pub(crate) row_paint_states: RefCell<Vec<RowPaintState>>,
    pub(crate) damage: DamageSet,
    visual_context_records: RefCell<Vec<Option<PaintableVisualContextRecord>>>,
    /// The node handles of each row's visual context record, published with the rows for the
    /// recording to read.
    visual_context_node_handles: RefCell<VisualContextNodeHandleColumn>,
    pub(crate) stacking_context_entries:
        RefCell<crate::painting::stacking_context::entries::StackingContextEntryColumn>,
    pub(crate) stacking_context_roots_flagged_for_resort: RefCell<Vec<NodeSlotId>>,
    line_roots_needing_fragment_ownership: RefCell<Vec<NodeSlotId>>,
    absolute_rect_memo: RefCell<Vec<Option<(NodeSlotId, u64, crate::css::css_pixels::CssPixelRect)>>>,
    absolute_rect_memo_epoch: Cell<u64>,
    committed_fragment_links: RefCell<CowColumn<CommittedFragmentLinkSlot, PAINTABLE_SLOTS_PER_CHUNK>>,
    /// Whether the chrome listens for paintable row resets. The callback itself is in the host
    /// tables, which only the main thread reaches.
    chrome_state_listens: Cell<bool>,
    paint_recording_in_progress: Cell<bool>,
    layout_commit_generation: Cell<u64>,
    scroll_offsets: ScrollOffsetColumn,
    image_map_areas: ImageMapAreaColumn,
    unique_node_ids: UniqueNodeIdColumn,
    visual_context_tree_inputs: Cell<crate::painting::host::FfiVisualContextTreeInputs>,
}

pub(crate) type VisualContextNodeHandleColumn =
    CowColumn<Option<std::sync::Arc<BoxVisualContextNodeHandles>>, PAINTABLE_SLOTS_PER_CHUNK>;

/// Sets the node handles published for a row, copying its chunk only when they change.
fn publish_visual_context_node_handles(
    column: &mut VisualContextNodeHandleColumn,
    index: usize,
    handles: Option<&BoxVisualContextNodeHandles>,
) {
    if column
        .get(index)
        .is_none_or(|published| published.as_deref() == handles)
    {
        return;
    }
    *column.get_mut(index).expect("the row is in the column") = handles.cloned().map(std::sync::Arc::new);
}

pub(crate) struct PaintableRows<Arena> {
    arena: Arena,
}

pub(crate) type PaintableRowsRef<'a> = PaintableRows<&'a LayoutNodeArena>;
pub(crate) type PaintableRowsMut<'a> = PaintableRows<&'a mut LayoutNodeArena>;

impl Clone for PaintableRowsRef<'_> {
    fn clone(&self) -> Self {
        Self { arena: self.arena }
    }
}

pub(crate) trait PaintableRowsRead: PaintRead + Deref<Target = LayoutNodeArena> {
    /// The scroll offset the document published for a box, or zero.
    fn scroll_offset(&self, id: NodeSlotId) -> CssPixelPoint;
    /// The unique node id the document published for what a box is the box of, or zero.
    fn unique_node_id(&self, id: NodeSlotId) -> i64;
    /// Reads the image map areas the document published.
    fn with_image_map_areas<R>(&self, read: impl FnOnce(&ImageMapAreas) -> R) -> R;
    /// Reads the hit-test list of the last recording, as it was when the rows were published.
    fn with_hit_test_list<R>(&self, read: impl FnOnce(Option<&HitTestList>) -> R) -> R;
    /// The visual context tree, as it was when the rows were published.
    fn visual_context_tree(&self) -> Option<std::sync::Arc<VisualContextTree>>;
}

/// A row's committed side data, as a published generation or the live column holds it.
pub(crate) enum CommittedSideDataRef<'a> {
    Published(&'a CommittedSideData),
    Live(Ref<'a, CommittedSideData>),
}

impl Deref for CommittedSideDataRef<'_> {
    type Target = CommittedSideData;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Published(side_data) => side_data,
            Self::Live(side_data) => side_data,
        }
    }
}

pub(crate) trait PaintableRowsWrite: PaintableRowsRead {
    fn paintable_data_mut(&mut self, id: NodeSlotId) -> &mut PaintableData;
}

impl<Arena> Deref for PaintableRows<Arena>
where
    Arena: Deref<Target = LayoutNodeArena>,
{
    type Target = LayoutNodeArena;

    fn deref(&self) -> &Self::Target {
        self.arena.deref()
    }
}

impl<Arena> DerefMut for PaintableRows<Arena>
where
    Arena: DerefMut<Target = LayoutNodeArena>,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.arena.deref_mut()
    }
}

impl<Arena> PaintableRows<Arena>
where
    Arena: Deref<Target = LayoutNodeArena>,
{
    pub(crate) fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        self.arena.live_paintable_data(id)
    }

    pub(crate) fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        if id.is_invalid() {
            return false;
        }
        let Some(data) = self.arena.paintable_rows.rows.get(id.slot_index() as usize) else {
            return false;
        };
        data.slot_generation != 0 && data.slot_generation == id.generation()
    }

    pub(crate) fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        CommittedSideDataRef::Live(self.arena.live_committed_side_data(id))
    }

    /// Identifies the version of the physical row slot. Unlike `NodeSlotId::generation()`, this
    /// changes when the same node's row is recommitted or cleared as well as when it is freed.
    pub(crate) fn paintable_row_reset_version(&self, id: NodeSlotId) -> u64 {
        self.arena.paintable_rows.row_reset_versions[id.slot_index() as usize]
    }

    pub(crate) fn clear_cached_overflow_data(&self, id: NodeSlotId) {
        if !self.paintable_row_is_populated(id) {
            return;
        }
        if self.arena.live_committed_side_data(id).overflow_valid_across_recommits {
            self.arena.committed_side_data_mut(id).overflow_valid_across_recommits = false;
            self.arena.note_row_overflow_unmeasured(id);
        }
    }

    /// A style repaint of a row also repaints the anonymous boxes it generated and, for an
    /// inline, the ancestors up to the line root that paints its pieces.
    pub(crate) fn for_each_row_repainted_with(&self, id: NodeSlotId, mut repaint: impl FnMut(NodeSlotId)) {
        if !self.paintable_row_is_populated(id) {
            return;
        }
        repaint(id);
        let mut stack: SmallVec<[NodeSlotId; 16]> = smallvec![id];
        while let Some(current) = stack.pop() {
            let mut child = crate::painting::paint_order::first_paint_child(self, current);
            while let Some(child_slot) = child {
                let child_flags = self.arena.node_flags_if_live(child_slot);
                if child_flags & NodeFlag::Anonymous as u32 != 0 {
                    repaint(child_slot);
                    stack.push(child_slot);
                }
                child = crate::painting::paint_order::next_paint_sibling(self, child_slot);
            }
        }
        if node_painting::is_inline(self, id) {
            let mut ancestor = crate::painting::paint_order::paint_parent(self, id);
            while let Some(current) = ancestor {
                repaint(current);
                if node_painting::has_lines(self, current) {
                    break;
                }
                ancestor = crate::painting::paint_order::paint_parent(self, current);
            }
        }
    }

    /// A text-decoration change on a box repaints the text of every line root and inline
    /// below it that inherits the decoration.
    pub(crate) fn push_propagated_text_decoration_damage(&self, root: NodeSlotId) {
        if !self.paintable_row_is_populated(root) {
            return;
        }

        let mut stack = Vec::new();
        if let Some(first_child) = crate::painting::paint_order::first_paint_child(self, root) {
            stack.push(first_child);
        }
        while let Some(current) = stack.pop() {
            if let Some(next_sibling) = crate::painting::paint_order::next_paint_sibling(self, current) {
                stack.push(next_sibling);
            }

            if crate::painting::style_queries::is_text_decoration_propagation_boundary(self.arena.deref(), current) {
                continue;
            }
            if node_painting::has_lines(self, current) || node_painting::is_inline(self, current) {
                self.arena.push_paint_damage(current, PaintDamage::DRAW_FOREGROUND);
            }
            if let Some(first_child) = crate::painting::paint_order::first_paint_child(self, current) {
                stack.push(first_child);
            }
        }
    }

    pub(crate) fn prepare_paintable_row_recommit_notification(&self, id: NodeSlotId) -> PaintableRowReset {
        assert!(self.paintable_row_is_populated(id));
        self.arena
            .prepare_paintable_row_reset(id, PaintableRowResetKind::Recommitted)
    }
}

impl<Arena> PaintableRows<Arena>
where
    Arena: DerefMut<Target = LayoutNodeArena>,
{
    pub(crate) fn paintable_data_mut(&mut self, id: NodeSlotId) -> &mut PaintableData {
        assert!(!id.is_invalid(), "invalid paintable arena slot ID");
        let data = self
            .arena
            .paintable_rows
            .rows
            .get_mut(id.slot_index() as usize)
            .expect("invalid paintable arena slot ID");
        assert_eq!(
            data.slot_generation,
            id.generation(),
            "paintable arena read a stale or unused slot"
        );
        data
    }

    pub(crate) fn begin_paintable_row_recommit(&mut self, id: NodeSlotId) {
        self.bump_paintable_row_reset_version(id);
        {
            let data = self.paintable_data_mut(id);
            data.offset = used_values::FfiCssPixelPoint::default();
            data.content_size = used_values::FfiCssPixelSize::default();
            data.local_padding_box_union = used_values::FfiCssPixelRect::default();
            data.local_border_box_union = used_values::FfiCssPixelRect::default();
        }
        self.arena
            .paintable_side_data(id)
            .overflow_measured_this_commit
            .set(false);
        // The row's damage is deliberately kept; the commit diff pushes what actually changed.
        let committed = self.arena.live_committed_side_data(id);
        let clears = committed.has_committed_records() || committed.fragment_ownership.is_some();
        drop(committed);
        if clears {
            let mut committed = self.arena.committed_side_data_mut(id);
            committed.clear_committed_records();
            if let Some(filter) = committed.fragment_ownership.take() {
                drop(committed);
                self.arena
                    .paintable_side_data_mut(id)
                    .fragment_ownership_before_recommit = Some(filter);
            }
        }
    }
}

/// The paintable rows as last published, for the main side. What is not a row is read from the
/// arena.
pub(crate) struct CommittedPaintableRows<'a> {
    arena: &'a LayoutNodeArena,
    /// Whether a recording of the arena is in flight, which writes the absolute rect memo: the view
    /// reads around it.
    beside_recording: bool,
}

impl Deref for CommittedPaintableRows<'_> {
    type Target = LayoutNodeArena;

    fn deref(&self) -> &Self::Target {
        self.arena
    }
}

impl CommittedPaintableRows<'_> {
    fn published(&self) -> &PublishedRows {
        self.arena
            .paintable_rows
            .published
            .as_ref()
            .expect("committed rows are published before they are read")
    }
}

impl PaintRead for CommittedPaintableRows<'_> {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        self.published().paintable_data(id)
    }

    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        self.published().paintable_row_is_populated(id)
    }

    fn with_committed_fragment_link<R>(
        &self,
        id: NodeSlotId,
        read: impl FnOnce(Option<&fragment_tree::FragmentLink>) -> R,
    ) -> R {
        self.published().with_committed_fragment_link(id, read)
    }

    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        CommittedSideDataRef::Published(self.published().committed_side_data(id))
    }

    read_live_arena!(std::ops::Deref::deref);

    fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<crate::css::css_pixels::CssPixelRect> {
        if self.beside_recording {
            return None;
        }
        self.arena.memoized_absolute_rect(id)
    }

    fn memoize_absolute_rect(&self, id: NodeSlotId, rect: crate::css::css_pixels::CssPixelRect) {
        if !self.beside_recording {
            self.arena.memoize_absolute_rect(id, rect);
        }
    }
}

impl PaintableRowsRead for CommittedPaintableRows<'_> {
    fn scroll_offset(&self, id: NodeSlotId) -> CssPixelPoint {
        self.published().scroll_offsets.offset(id)
    }

    fn unique_node_id(&self, id: NodeSlotId) -> i64 {
        unique_node_id_of(self.published().unique_node_ids.get(id.slot_index() as usize), id)
    }

    fn with_image_map_areas<R>(&self, read: impl FnOnce(&ImageMapAreas) -> R) -> R {
        read(&self.published().image_map_areas)
    }

    fn with_hit_test_list<R>(&self, read: impl FnOnce(Option<&HitTestList>) -> R) -> R {
        read(self.published().hit_test_list.as_deref())
    }

    fn visual_context_tree(&self) -> Option<std::sync::Arc<VisualContextTree>> {
        self.published().visual_context_tree.clone()
    }
}

/// The paintable rows as a main-side read sees them, from [`crate::painting::ffi`]'s one door
/// to them. The view holds the arena for as long as it lives, so every read through it agrees.
pub(crate) enum MainSidePaintableRows<'a> {
    /// A read between stages: the rows as last committed.
    Committed(CommittedPaintableRows<'a>),
    /// A host call made while a stage runs: the rows as that stage is writing them.
    DuringStage(PaintableRowsRef<'a>),
}

impl Deref for MainSidePaintableRows<'_> {
    type Target = LayoutNodeArena;

    fn deref(&self) -> &Self::Target {
        match self {
            Self::Committed(rows) => rows,
            Self::DuringStage(rows) => rows,
        }
    }
}

impl PaintRead for MainSidePaintableRows<'_> {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        match self {
            Self::Committed(rows) => rows.paintable_data(id),
            Self::DuringStage(rows) => rows.paintable_data(id),
        }
    }

    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        match self {
            Self::Committed(rows) => rows.paintable_row_is_populated(id),
            Self::DuringStage(rows) => rows.paintable_row_is_populated(id),
        }
    }

    fn with_committed_fragment_link<R>(
        &self,
        id: NodeSlotId,
        read: impl FnOnce(Option<&fragment_tree::FragmentLink>) -> R,
    ) -> R {
        match self {
            Self::Committed(rows) => rows.with_committed_fragment_link(id, read),
            Self::DuringStage(rows) => rows.with_committed_fragment_link(id, read),
        }
    }

    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        match self {
            Self::Committed(rows) => rows.committed_side_data(id),
            Self::DuringStage(rows) => rows.committed_side_data(id),
        }
    }

    read_live_arena!(std::ops::Deref::deref);

    fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<crate::css::css_pixels::CssPixelRect> {
        match self {
            Self::Committed(rows) => PaintRead::memoized_absolute_rect(rows, id),
            Self::DuringStage(rows) => PaintRead::memoized_absolute_rect(rows, id),
        }
    }

    fn memoize_absolute_rect(&self, id: NodeSlotId, rect: crate::css::css_pixels::CssPixelRect) {
        match self {
            Self::Committed(rows) => PaintRead::memoize_absolute_rect(rows, id, rect),
            Self::DuringStage(rows) => PaintRead::memoize_absolute_rect(rows, id, rect),
        }
    }
}

impl PaintableRowsRead for MainSidePaintableRows<'_> {
    fn scroll_offset(&self, id: NodeSlotId) -> CssPixelPoint {
        match self {
            Self::Committed(rows) => rows.scroll_offset(id),
            Self::DuringStage(rows) => rows.scroll_offset(id),
        }
    }

    fn unique_node_id(&self, id: NodeSlotId) -> i64 {
        match self {
            Self::Committed(rows) => rows.unique_node_id(id),
            Self::DuringStage(rows) => rows.unique_node_id(id),
        }
    }

    fn with_image_map_areas<R>(&self, read: impl FnOnce(&ImageMapAreas) -> R) -> R {
        match self {
            Self::Committed(rows) => rows.with_image_map_areas(read),
            Self::DuringStage(rows) => rows.with_image_map_areas(read),
        }
    }

    fn with_hit_test_list<R>(&self, read: impl FnOnce(Option<&HitTestList>) -> R) -> R {
        match self {
            Self::Committed(rows) => rows.with_hit_test_list(read),
            Self::DuringStage(rows) => rows.with_hit_test_list(read),
        }
    }

    fn visual_context_tree(&self) -> Option<std::sync::Arc<VisualContextTree>> {
        match self {
            Self::Committed(rows) => rows.visual_context_tree(),
            Self::DuringStage(rows) => rows.visual_context_tree(),
        }
    }
}

impl<Arena> PaintRead for PaintableRows<Arena>
where
    Arena: Deref<Target = LayoutNodeArena>,
{
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        PaintableRows::paintable_data(self, id)
    }

    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        PaintableRows::paintable_row_is_populated(self, id)
    }

    fn with_committed_fragment_link<R>(
        &self,
        id: NodeSlotId,
        read: impl FnOnce(Option<&fragment_tree::FragmentLink>) -> R,
    ) -> R {
        self.arena.with_committed_fragment_link(id, read)
    }

    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        PaintableRows::committed_side_data(self, id)
    }

    read_live_arena!(std::ops::Deref::deref);

    fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<CssPixelRect> {
        LayoutNodeArena::memoized_absolute_rect(self, id)
    }

    fn memoize_absolute_rect(&self, id: NodeSlotId, rect: CssPixelRect) {
        LayoutNodeArena::memoize_absolute_rect(self, id, rect);
    }
}

impl<Arena> PaintableRowsRead for PaintableRows<Arena>
where
    Arena: Deref<Target = LayoutNodeArena>,
{
    fn scroll_offset(&self, id: NodeSlotId) -> CssPixelPoint {
        self.arena.paintable_rows.scroll_offsets.offset(id)
    }

    fn unique_node_id(&self, id: NodeSlotId) -> i64 {
        self.arena.paintable_rows.unique_node_ids.id(id)
    }

    fn with_image_map_areas<R>(&self, read: impl FnOnce(&ImageMapAreas) -> R) -> R {
        self.arena.paintable_rows.image_map_areas.with_areas(read)
    }

    fn with_hit_test_list<R>(&self, read: impl FnOnce(Option<&HitTestList>) -> R) -> R {
        read(self.arena.hit_test_list.borrow().as_deref())
    }

    fn visual_context_tree(&self) -> Option<std::sync::Arc<VisualContextTree>> {
        self.arena.paint_state().borrow().visual_context.tree.clone()
    }
}

impl<Arena> PaintableRowsWrite for PaintableRows<Arena>
where
    Arena: DerefMut<Target = LayoutNodeArena>,
{
    fn paintable_data_mut(&mut self, id: NodeSlotId) -> &mut PaintableData {
        PaintableRows::paintable_data_mut(self, id)
    }
}

impl PaintableRowStore {
    pub(crate) fn with_current_committed_fragment<R>(
        &self,
        layout_slot_index: u32,
        layout_slot_generation: u8,
        geometry_epoch: u32,
        read: impl FnOnce(&fragment_tree::Fragment) -> R,
    ) -> Option<R> {
        let slots = self.committed_fragment_links.borrow();
        let slot = slots.get(layout_slot_index as usize)?;
        if slot.layout_slot_generation != layout_slot_generation
            || !slot.geometry_is_current
            || slot.geometry_epoch != geometry_epoch
        {
            return None;
        }
        Some(read(&slot.link.as_deref()?.fragment))
    }

    pub(crate) fn invalidate_committed_geometry(&self, layout_slot_index: u32) {
        if let Some(slot) = self
            .committed_fragment_links
            .borrow_mut()
            .get_mut(layout_slot_index as usize)
        {
            slot.geometry_is_current = false;
        }
    }

    pub(crate) fn committed_fragment_link_cloned(
        &self,
        layout_slot_index: u32,
        layout_slot_generation: u8,
    ) -> Option<fragment_tree::FragmentLink> {
        self.committed_fragment_links
            .borrow()
            .get(layout_slot_index as usize)
            .and_then(|slot| slot.link_for(layout_slot_generation).cloned())
    }

    pub(crate) fn with_committed_fragment_link<R>(
        &self,
        layout_slot_index: u32,
        layout_slot_generation: u8,
        read: impl FnOnce(Option<&fragment_tree::FragmentLink>) -> R,
    ) -> R {
        let slots = self.committed_fragment_links.borrow();
        read(
            slots
                .get(layout_slot_index as usize)
                .and_then(|slot| slot.link_for(layout_slot_generation)),
        )
    }

    pub(crate) fn set_committed_fragment_link(
        &self,
        layout_slot_index: u32,
        layout_slot_generation: u8,
        geometry_epoch: Option<u32>,
        link: fragment_tree::FragmentLink,
    ) {
        let mut slots = self.committed_fragment_links.borrow_mut();
        slots.grow_to(layout_slot_index as usize + 1);
        let slot = slots
            .get_mut(layout_slot_index as usize)
            .expect("the column grew to hold the slot");
        let geometry_is_current = geometry_epoch.is_some();
        let geometry_epoch = geometry_epoch.unwrap_or_default();
        // A link no published generation shares is overwritten in place.
        if slot.layout_slot_generation == layout_slot_generation
            && let Some(retained_link) = slot.link.as_mut().and_then(std::sync::Arc::get_mut)
        {
            *retained_link = link;
            slot.geometry_epoch = geometry_epoch;
            slot.geometry_is_current = geometry_is_current;
            return;
        }
        *slot = CommittedFragmentLinkSlot {
            layout_slot_generation,
            geometry_epoch,
            geometry_is_current,
            link: Some(std::sync::Arc::new(link)),
        };
    }

    pub(crate) fn take_committed_fragment_link(
        &self,
        layout_slot_index: u32,
        layout_slot_generation: u8,
    ) -> Option<fragment_tree::FragmentLink> {
        self.committed_fragment_links
            .borrow_mut()
            .get_mut(layout_slot_index as usize)
            .filter(|slot| slot.layout_slot_generation == layout_slot_generation)
            .and_then(|slot| slot.link.take())
            .map(std::sync::Arc::unwrap_or_clone)
    }

    pub(crate) fn reset_committed_fragment_link_slot(&mut self, layout_slot_index: u32) {
        if let Some(slot) = self
            .committed_fragment_links
            .get_mut()
            .get_mut(layout_slot_index as usize)
        {
            *slot = CommittedFragmentLinkSlot::default();
        }
    }
}

impl LayoutNodeArena {
    pub(crate) fn refresh_paint_order_inputs(&self, row: NodeSlotId) {
        if !self.paintable_row_is_populated(row) {
            return;
        }
        let inputs = crate::painting::paint_order_plan::PaintOrderInputs::gather(&self.paintable_rows(), row);
        if self.update_paint_order_inputs(row, inputs) {
            self.note_paint_order_changed(row);
        }
    }

    /// Records the paint-order decisions gathered for a row, and returns whether they changed.
    pub(crate) fn update_paint_order_inputs(
        &self,
        row: NodeSlotId,
        inputs: crate::painting::paint_order_plan::PaintOrderInputs,
    ) -> bool {
        if self.live_committed_side_data(row).order_inputs == inputs {
            return false;
        }
        self.committed_side_data_mut(row).order_inputs = inputs;
        true
    }

    // A row whose ordering decisions changed is placed differently by its ancestors' plans and
    // may plan its own descendants differently.
    pub(crate) fn note_paint_order_changed(&self, row: NodeSlotId) {
        let _writer = crate::painting::published_immutable::enter_writer_if_unattributed("paint-order maintenance");
        self.push_paint_damage(row, PaintDamage::ORDER);
        self.push_enclosing_paint_order_damage(row);
    }

    // The entry tables decide how a stacking context composes its hoisted content, so a table
    // change reorders the context's own painting even when no row changed its own decisions.
    pub(crate) fn note_stacking_context_composition_changed(&self, context_root: NodeSlotId) {
        let _writer = crate::painting::published_immutable::enter_writer_if_unattributed("paint-order maintenance");
        self.push_paint_damage(context_root, PaintDamage::CONTEXT_ORDER);
    }

    pub(crate) fn paintable_rows(&self) -> PaintableRowsRef<'_> {
        PaintableRows { arena: self }
    }

    pub(crate) fn schedule_scrollable_overflow_recalculation(&self, node: NodeSlotId) {
        // SAFETY: The caller supplies a live slot; data() generation-checks every slot the
        // containing-block walk visits.
        let node_kind = self.data(node).kind.get();
        if crate::layout::node_facts::kind_is_box(node_kind) {
            let paintable_rows = self.paintable_rows();
            let mut containing_box = node;
            loop {
                if paintable_rows.paintable_row_is_populated(containing_box) {
                    paintable_rows.clear_cached_overflow_data(containing_box);
                }
                // SAFETY: As above.
                let Some(next) = self.node_containing_block_if_live(containing_box) else {
                    break;
                };
                containing_box = next;
            }
        }

        if self.needs_full_scrollable_overflow_recalculation.get() {
            return;
        }

        // NB: Cap the queue in case it's never consumed (e.g. forced style updates in a document
        //     that never updates layout).
        const MAX_PENDING_SCROLLABLE_OVERFLOW_RECALCULATIONS: usize = 1024;
        let mut boxes = self.boxes_needing_scrollable_overflow_recalculation.borrow_mut();
        if boxes.len() >= MAX_PENDING_SCROLLABLE_OVERFLOW_RECALCULATIONS {
            boxes.clear();
            self.needs_full_scrollable_overflow_recalculation.set(true);
            return;
        }

        if self.paintable_rows().paintable_row_is_populated(node) {
            boxes.push(node);
        }
    }

    pub(crate) fn set_needs_full_scrollable_overflow_recalculation(&self) {
        self.needs_full_scrollable_overflow_recalculation.set(true);
    }

    pub(crate) fn has_scheduled_scrollable_overflow_recalculation(&self) -> bool {
        self.needs_full_scrollable_overflow_recalculation.get()
            || !self.boxes_needing_scrollable_overflow_recalculation.borrow().is_empty()
    }

    pub(crate) fn take_scrollable_overflow_recalculation_state(&self) -> (Vec<NodeSlotId>, bool) {
        (
            std::mem::take(&mut *self.boxes_needing_scrollable_overflow_recalculation.borrow_mut()),
            self.needs_full_scrollable_overflow_recalculation.replace(false),
        )
    }

    pub(crate) fn paintable_rows_mut(&mut self) -> PaintableRowsMut<'_> {
        PaintableRows { arena: self }
    }

    pub(crate) fn set_chrome_state_listens(&self, listens: bool) {
        self.paintable_rows.chrome_state_listens.set(listens);
    }

    pub(crate) fn with_committed_fragment_link<R>(
        &self,
        node: NodeSlotId,
        read: impl FnOnce(Option<&fragment_tree::FragmentLink>) -> R,
    ) -> R {
        debug_assert!(self.paintable_row_is_populated(node));
        let slots = self.paintable_rows.committed_fragment_links.borrow();
        read(
            slots
                .get(node.slot_index() as usize)
                .and_then(|entry| entry.link.as_deref()),
        )
    }

    pub(crate) fn with_committed_fragment_link_during_layout<R>(
        &self,
        node: NodeSlotId,
        read: impl FnOnce(Option<&fragment_tree::FragmentLink>) -> R,
    ) -> R {
        self.paintable_rows
            .with_committed_fragment_link(node.slot_index(), node.generation(), read)
    }

    fn prepare_paintable_row_reset(&self, slot: NodeSlotId, kind: PaintableRowResetKind) -> PaintableRowReset {
        PaintableRowReset {
            slot,
            kind,
            notifies_chrome_state: self.paintable_rows.chrome_state_listens.get(),
        }
    }

    pub(crate) fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<crate::css::css_pixels::CssPixelRect> {
        let store = &self.paintable_rows;
        let (memoized_id, memoized_epoch, rect) = store
            .absolute_rect_memo
            .borrow()
            .get(id.slot_index() as usize)
            .copied()
            .flatten()?;
        (memoized_id == id && memoized_epoch == store.absolute_rect_memo_epoch.get()).then_some(rect)
    }

    pub(crate) fn memoize_absolute_rect(&self, id: NodeSlotId, rect: crate::css::css_pixels::CssPixelRect) {
        let store = &self.paintable_rows;
        store.absolute_rect_memo.borrow_mut()[id.slot_index() as usize] =
            Some((id, store.absolute_rect_memo_epoch.get(), rect));
    }

    pub(crate) fn clear_absolute_rect_memo(&self) {
        let epoch = &self.paintable_rows.absolute_rect_memo_epoch;
        epoch.set(epoch.get().checked_add(1).expect("absolute rect memo epoch overflowed"));
    }

    pub(crate) fn paintable_row_count(&self) -> usize {
        self.paintable_rows.side_data.borrow().len()
    }

    pub(crate) fn published_paintable_rows(&self) -> Vec<NodeSlotId> {
        (0..self.paintable_row_count() as u32)
            .filter_map(|index| {
                let generation = self.paintable_data_by_index(index).slot_generation;
                (generation != 0).then(|| NodeSlotId::new(index, generation))
            })
            .collect()
    }

    /// The scroll offset each box holds, as published by the one place the DOM stores it.
    pub(crate) fn scroll_offsets(&self) -> &ScrollOffsetColumn {
        &self.paintable_rows.scroll_offsets
    }

    /// The areas of the image map each image is associated with, as the document published them.
    pub(crate) fn image_map_areas(&self) -> &ImageMapAreaColumn {
        &self.paintable_rows.image_map_areas
    }

    /// The unique node id each box is the box of something with, as the document published it.
    pub(crate) fn unique_node_ids(&self) -> &UniqueNodeIdColumn {
        &self.paintable_rows.unique_node_ids
    }

    /// What the document last published about the viewport the render side draws into.
    pub(crate) fn visual_context_tree_inputs(&self) -> crate::painting::host::FfiVisualContextTreeInputs {
        self.paintable_rows.visual_context_tree_inputs.get()
    }

    pub(crate) fn publish_visual_context_tree_inputs(&self, inputs: crate::painting::host::FfiVisualContextTreeInputs) {
        self.paintable_rows.visual_context_tree_inputs.set(inputs);
    }

    /// Counts the layout commits the arena has published. A main-side reader that remembers the
    /// generation it read at can tell whether the committed geometry it saw is still the one
    /// published, without asking what was dirty at the time.
    pub(crate) fn layout_commit_generation(&self) -> u64 {
        self.paintable_rows.layout_commit_generation.get()
    }

    pub(crate) fn note_layout_commit(&self) {
        let generation = &self.paintable_rows.layout_commit_generation;
        generation.set(generation.get().wrapping_add(1));
    }

    pub(crate) fn set_paint_recording_in_progress(&self, in_progress: bool) {
        self.paintable_rows.paint_recording_in_progress.set(in_progress);
    }

    pub(crate) fn debug_assert_not_recording(&self) {
        debug_assert!(
            !self.paintable_rows.paint_recording_in_progress.get(),
            "paint damage pushed during display list recording would be missed by it"
        );
    }

    pub(crate) fn inline_pieces_root(&self, inline_paintable: NodeSlotId) -> Option<NodeSlotId> {
        self.paintable_rows().inline_pieces_root(inline_paintable)
    }

    pub(crate) fn populate_paintable_row(&mut self, layout_node: NodeSlotId) {
        self.note_committed_box_changed(layout_node);
        self.note_overflow_contained_box_added(layout_node);
        let overflow_style = self
            .node_style_if_live(layout_node)
            .map(crate::painting::scrollable_overflow::OverflowStyle::new);
        {
            let store = &mut self.paintable_rows;
            let index = layout_node.slot_index() as usize;
            let mut side_data = store.side_data.borrow_mut();
            let mut row_paint_states = store.row_paint_states.borrow_mut();
            let mut absolute_rect_memo = store.absolute_rect_memo.borrow_mut();
            let mut visual_context_records = store.visual_context_records.borrow_mut();
            let mut stacking_context_entries = store.stacking_context_entries.borrow_mut();
            while side_data.len() <= index {
                side_data.push(PaintableSideData::default());
                store.row_reset_versions.push(0);
                row_paint_states.push(RowPaintState::default());
                absolute_rect_memo.push(None);
                visual_context_records.push(None);
            }
            stacking_context_entries.grow_to(side_data.len());
            let visual_context_node_handles = store.visual_context_node_handles.get_mut();
            visual_context_node_handles.grow_to(side_data.len());
            publish_visual_context_node_handles(visual_context_node_handles, index, None);

            store.rows.grow_to(side_data.len());
            let committed_side_data = store.committed_side_data.get_mut();
            committed_side_data.grow_to(side_data.len());
            *committed_side_data.get_mut(index).expect("the row was just grown") = CommittedSideData::default();
            *store.rows.get_mut(index).expect("the row was just grown") = PaintableData {
                slot_generation: layout_node.generation(),
                ..PaintableData::default()
            };
            side_data[index] = PaintableSideData {
                overflow_style,
                ..Default::default()
            };
            row_paint_states[index].clear();
            absolute_rect_memo[index] = None;
            self.scrollable_overflow.rows_to_measure.get_mut().push(layout_node);
            visual_context_records[index] = None;
            crate::painting::stacking_context::entries::drop_table(&mut stacking_context_entries, index);
        }
        self.flush_committed_box_changes();
    }

    fn reset_paintable_row(&mut self, row_is_still_linked: bool, reset: PaintableRowReset) {
        let id = reset.slot;
        self.bump_paintable_row_reset_version(id);
        if reset.kind == crate::painting::paintable_data::PaintableRowResetKind::Freed {
            self.paintable_rows.scroll_offsets.forget(id);
            self.paintable_rows.unique_node_ids.forget(id);
            self.paintable_rows.image_map_areas.forget(id);
        }
        self.note_committed_box_changed(id);
        if row_is_still_linked {
            // A cleared row is still linked, so the ancestor whose plans listed it is known now.
            self.push_enclosing_paint_order_damage(id);
        }
        if let Some(record) = self.take_paintable_visual_context_record(id) {
            self.withdraw_stacking_context_state_of_reset_row(id, Some(record.stacking_context));
            let former_paint_parent =
                crate::painting::paint_order::paint_parent(&self.paintable_rows(), id).unwrap_or(NodeSlotId::INVALID);
            self.paint_state()
                .borrow_mut()
                .visual_context
                .dirty_boxes
                .note_removed(RemovedBoxBlocks {
                    slot: id,
                    node_handles: record.node_handles,
                    former_paint_parent,
                });
        } else {
            self.withdraw_stacking_context_state_of_reset_row(id, None);
            self.paint_state()
                .borrow_mut()
                .visual_context
                .dirty_boxes
                .forget_box(id);
        }
        self.clear_absolute_rect_memo();
        let store = &mut self.paintable_rows;
        let index = id.slot_index() as usize;
        *store.rows.get_mut(index).expect("invalid paintable arena slot ID") = PaintableData::default();
        store.side_data.borrow_mut()[index] = PaintableSideData::default();
        *store
            .committed_side_data
            .get_mut()
            .get_mut(index)
            .expect("invalid paintable arena slot ID") = CommittedSideData::default();
        store.row_paint_states.borrow()[index].clear();
        store.visual_context_records.borrow_mut()[index] = None;
        publish_visual_context_node_handles(store.visual_context_node_handles.get_mut(), index, None);
        crate::painting::stacking_context::entries::drop_table(store.stacking_context_entries.get_mut(), index);
        self.flush_committed_box_changes();
    }

    fn bump_paintable_row_reset_version(&mut self, id: NodeSlotId) {
        let version = &mut self.paintable_rows.row_reset_versions[id.slot_index() as usize];
        *version = version.checked_add(1).expect("paintable row reset version overflowed");
    }

    pub(crate) fn paintable_visual_context_record(
        &self,
        id: NodeSlotId,
    ) -> Option<Ref<'_, PaintableVisualContextRecord>> {
        if !self.paintable_row_is_populated(id) {
            return None;
        }
        Ref::filter_map(self.paintable_rows.visual_context_records.borrow(), |records| {
            records.get(id.slot_index() as usize).and_then(Option::as_ref)
        })
        .ok()
    }

    pub(crate) fn take_paintable_visual_context_record(&self, id: NodeSlotId) -> Option<PaintableVisualContextRecord> {
        if !self.paintable_row_is_populated(id) {
            return None;
        }
        self.paintable_rows
            .visual_context_records
            .borrow_mut()
            .get_mut(id.slot_index() as usize)
            .and_then(Option::take)
    }

    pub(crate) fn set_paintable_visual_context_record(&self, id: NodeSlotId, record: PaintableVisualContextRecord) {
        debug_assert!(self.paintable_row_is_populated(id));
        let prepared = self.live_committed_side_data(id).prepared_order_inputs();
        let inputs = prepared.map(|inputs| {
            inputs.with_visual_context(
                &record.stacking_context,
                crate::painting::style_queries::z_index(self, id),
            )
        });
        publish_visual_context_node_handles(
            &mut self.paintable_rows.visual_context_node_handles.borrow_mut(),
            id.slot_index() as usize,
            Some(&record.node_handles),
        );
        self.paintable_rows.visual_context_records.borrow_mut()[id.slot_index() as usize] = Some(record);
        if let Some(inputs) = inputs {
            if self.update_paint_order_inputs(id, inputs) {
                self.note_paint_order_changed(id);
            }
        } else {
            // Some resource paintables are first prepared outside a layout commit.
            self.refresh_paint_order_inputs(id);
        }
    }

    pub(crate) fn drop_all_visual_context_records(&self) {
        self.paintable_rows.visual_context_records.borrow_mut().fill(None);
        let mut handles = self.paintable_rows.visual_context_node_handles.borrow_mut();
        for index in 0..self.paintable_row_count() {
            publish_visual_context_node_handles(&mut handles, index, None);
        }
    }

    pub(crate) fn with_paintable_visual_context_node_handles<R>(
        &self,
        id: NodeSlotId,
        read: impl FnOnce(&BoxVisualContextNodeHandles) -> R,
    ) -> R {
        match self.paintable_visual_context_record(id) {
            Some(record) => read(&record.node_handles),
            None => read(&EMPTY_BOX_VISUAL_CONTEXT_NODE_HANDLES),
        }
    }

    /// The nodes of the box that an animation of the kind drives: the effect nodes of an opacity,
    /// background color or filter animation, or the spatial nodes of a transform animation, as far
    /// as the tree holds nodes of the right kind for them.
    pub(crate) fn paintable_visual_animation_target_indices(
        &self,
        id: NodeSlotId,
        tree: Option<&crate::painting::visual_context::VisualContextTree>,
        target_kind: crate::painting::host::FfiVisualAnimationTargetKind,
    ) -> Vec<u32> {
        use crate::painting::host::FfiVisualAnimationTargetKind;
        let Some(tree) = tree else {
            return Vec::new();
        };
        self.with_paintable_visual_context_node_handles(id, |handles| {
            let valid_targets = |indices: &mut dyn Iterator<Item = u32>| -> Vec<u32> {
                indices
                    .filter(|&index| tree.visual_animation_target_is_valid(target_kind, index))
                    .collect()
            };
            match target_kind {
                FfiVisualAnimationTargetKind::Opacity
                | FfiVisualAnimationTargetKind::BackgroundColor
                | FfiVisualAnimationTargetKind::Filter => {
                    valid_targets(&mut handles.effects.iter().map(|index| index.0))
                }
                FfiVisualAnimationTargetKind::Transform => {
                    valid_targets(&mut handles.spatial.iter().map(|index| index.0))
                }
            }
        })
    }

    pub(crate) fn mark_paintable_subtree_may_own_geometry_dependent_nodes(&self, id: NodeSlotId) -> Option<bool> {
        if !self.paintable_row_is_populated(id) {
            return None;
        }
        let mut records = self.paintable_rows.visual_context_records.borrow_mut();
        let record = records.get_mut(id.slot_index() as usize)?.as_mut()?;
        let was_flagged = record.subtree_may_own_geometry_dependent_nodes;
        record.subtree_may_own_geometry_dependent_nodes = true;
        Some(was_flagged)
    }

    pub(crate) fn set_paintable_record_stacking_context_contribution_registered(&self, id: NodeSlotId) {
        debug_assert!(self.paintable_row_is_populated(id));
        let mut records = self.paintable_rows.visual_context_records.borrow_mut();
        if let Some(record) = records.get_mut(id.slot_index() as usize).and_then(Option::as_mut) {
            record.stacking_context.contribution_is_registered = true;
        }
    }

    pub(crate) fn note_line_root_needs_fragment_ownership(&self, line_root: NodeSlotId) {
        self.paintable_rows
            .line_roots_needing_fragment_ownership
            .borrow_mut()
            .push(line_root);
    }

    pub(crate) fn take_line_roots_needing_fragment_ownership(&self) -> Vec<NodeSlotId> {
        std::mem::take(&mut *self.paintable_rows.line_roots_needing_fragment_ownership.borrow_mut())
    }

    pub(crate) fn note_visual_context_box_dirty(&self, id: NodeSlotId, kind: VisualContextBoxDirtyKind) {
        let pending_box_limit = self
            .paintable_row_count()
            .max(crate::painting::visual_context::dirty::MINIMUM_PENDING_DIRTY_BOX_LIMIT);
        self.paint_state()
            .borrow_mut()
            .visual_context
            .dirty_boxes
            .note_box(id, kind, pending_box_limit);
    }

    pub(crate) fn request_full_visual_context_rebuild(&self, reason: VisualContextGlobalRebuildReason) {
        self.paint_state()
            .borrow_mut()
            .visual_context
            .dirty_boxes
            .request_full_rebuild(reason);
    }

    pub(crate) fn prepare_paintable_row_freed_reset(&self, layout_slot_index: u32) -> Option<PaintableRowReset> {
        if layout_slot_index as usize >= self.paintable_row_count() {
            return None;
        }
        let generation = self.paintable_data_by_index(layout_slot_index).slot_generation;
        if generation == 0 {
            return None;
        }
        Some(self.prepare_paintable_row_reset(
            NodeSlotId::new(layout_slot_index, generation),
            PaintableRowResetKind::Freed,
        ))
    }

    pub(crate) fn paintable_row_freed(&mut self, reset: PaintableRowReset) {
        crate::painting::published_immutable::note_row_mutation(self, reset.slot, "M1b paintable_row_freed");
        self.reset_paintable_row(false, reset);
    }

    pub(crate) fn prepare_paintable_row_cleared_reset(&self, layout_node: NodeSlotId) -> Option<PaintableRowReset> {
        self.paintable_row_is_populated(layout_node)
            .then(|| self.prepare_paintable_row_reset(layout_node, PaintableRowResetKind::Cleared))
    }

    pub(crate) fn paintable_row_cleared(&mut self, reset: PaintableRowReset) {
        if !self.scrollable_overflow.non_child_boxes.borrow().is_empty() {
            self.scrollable_overflow.contained_boxes_dirty.set(true);
        }
        self.reset_paintable_row(true, reset);
    }

    pub(crate) fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        self.paintable_rows().paintable_row_is_populated(id)
    }

    /// Lets a writer that runs while nothing reads the published rows write their chunks in place.
    pub(crate) fn release_published_paintable_rows(&mut self) {
        self.paintable_rows.published = None;
    }

    /// Lets a main-side writer write the rows' chunks in place. It runs after the frame holding
    /// the arena is taken in, so nothing reads the rows as last published until the main side
    /// reads them again, which publishes them anew.
    pub(crate) fn release_published_paintable_rows_for_main_side_write(&mut self) {
        let reads_beside_recording = crate::stage_thread::reads_beside_recording_of(std::ptr::from_ref(self).cast());
        debug_assert!(
            !reads_beside_recording,
            "a main-side write of the rows runs beside a recording that reads them"
        );
        if !reads_beside_recording {
            self.release_published_paintable_rows();
        }
    }

    /// Hands the main side the rows as they are now, if a writer changed them since they were last
    /// handed over.
    pub(crate) fn publish_paintable_rows(&mut self) {
        let hit_test_list = self.hit_test_list.get_mut().clone();
        let visual_context_tree = self.paint_state().borrow().visual_context.tree.clone();
        let store = &mut self.paintable_rows;
        let fragment_links = store.committed_fragment_links.get_mut();
        let side_data = store.committed_side_data.get_mut();
        let unique_node_ids = store.unique_node_ids.ids.get_mut();
        let stacking_context_entries = store.stacking_context_entries.get_mut();
        let visual_context_node_handles = store.visual_context_node_handles.get_mut();
        let Some(published) = &mut store.published else {
            store.published = Some(PublishedRows {
                rows: store.rows.publish(),
                fragment_links: fragment_links.publish(),
                side_data: side_data.publish(),
                unique_node_ids: unique_node_ids.publish(),
                stacking_context_entries: stacking_context_entries.publish(),
                visual_context_node_handles: visual_context_node_handles.publish(),
                scroll_offsets: store.scroll_offsets.snapshot(),
                image_map_areas: store.image_map_areas.snapshot(),
                hit_test_list,
                visual_context_tree,
            });
            return;
        };
        if store.rows.written_since_publish() {
            published.rows = store.rows.publish();
        }
        if fragment_links.written_since_publish() {
            published.fragment_links = fragment_links.publish();
        }
        if side_data.written_since_publish() {
            published.side_data = side_data.publish();
        }
        if unique_node_ids.written_since_publish() {
            published.unique_node_ids = unique_node_ids.publish();
        }
        if stacking_context_entries.written_since_publish() {
            published.stacking_context_entries = stacking_context_entries.publish();
        }
        if visual_context_node_handles.written_since_publish() {
            published.visual_context_node_handles = visual_context_node_handles.publish();
        }
        published.scroll_offsets = store.scroll_offsets.snapshot();
        published.image_map_areas = store.image_map_areas.snapshot();
        published.hit_test_list = hit_test_list;
        published.visual_context_tree = visual_context_tree;
    }

    /// Publishes the rows as they are now, for a recording to read while the arena goes on
    /// changing.
    pub(crate) fn freeze_paint_frame(&mut self) -> PublishedFrame {
        self.publish_paintable_rows();
        let rows = self
            .paintable_rows
            .published
            .clone()
            .expect("the rows were just published");
        let hit_test_item_capacity_hint = self.hit_test_list.get_mut().as_ref().map_or(0, |list| list.items.len());
        let paint_state = crate::painting::published_frame::PublishedPaintState::new(
            &self.paint_state().borrow(),
            hit_test_item_capacity_hint,
        );
        let (replaced, layer_images) = self.publish_paint_fact_tables();
        let facts = crate::painting::published_frame::PublishedPaintFacts {
            text: self.publish_text(),
            replaced,
            layer_images,
            svg_paint_resources: self.svg_paint_resources().publish(),
        };
        let (nodes, retired_slots) = self.publish_paint_tree();
        PublishedFrame::new(
            rows,
            nodes,
            retired_slots,
            self.paint_damage_for_frame(),
            paint_state,
            facts,
        )
    }

    /// Builds the structures a hit-test query derives from the list before the rows are
    /// published, so that the query only reads. A published generation that still pins the list
    /// lets go of it first, so building does not copy it.
    pub(crate) fn prepare_hit_test_list_for_query(&mut self, needs_spatial_indexes: bool, needs_caret_lines: bool) {
        if let Some(published) = &mut self.paintable_rows.published {
            published.hit_test_list = None;
        }
        let mut list = std::mem::take(self.hit_test_list.get_mut());
        if let Some(list) = list.as_mut()
            && ((needs_spatial_indexes && !list.spatial_indexes_built)
                || (needs_caret_lines && !list.caret_lines_built))
        {
            self.run_stage(|arena| {
                let list = std::sync::Arc::make_mut(list);
                if needs_spatial_indexes {
                    list.build_spatial_indexes_if_needed();
                }
                if needs_caret_lines {
                    list.build_caret_lines_if_needed(&arena.paintable_rows());
                }
            });
        }
        *self.hit_test_list.get_mut() = list;
    }

    #[cfg(test)]
    pub(crate) fn published_fragment_link_for_test(&self, id: NodeSlotId) -> Option<fragment_tree::FragmentLink> {
        self.paintable_rows
            .published
            .as_ref()?
            .fragment_links
            .get(id.slot_index() as usize)?
            .link_for(id.generation())
            .cloned()
    }

    /// The paintable rows as last published. Rows a main-side writer changed since are published
    /// first: that writer has finished, since the main side reads between writes. Overflow a
    /// commit or a writer left unmeasured is measured before they are.
    pub(crate) fn committed_paintable_rows(&mut self) -> CommittedPaintableRows<'_> {
        self.measure_scrollable_overflow_on_stage_before_publication();
        self.publish_paintable_rows();
        CommittedPaintableRows {
            arena: self,
            beside_recording: false,
        }
    }

    /// The paintable rows as last published, read beside a recording of the arena in flight. The
    /// recording published them before it was submitted, and changes none of them.
    pub(crate) fn rows_beside_recording(&self) -> CommittedPaintableRows<'_> {
        assert!(
            self.paintable_rows.published.is_some(),
            "a recording publishes the rows before it is submitted"
        );
        CommittedPaintableRows {
            arena: self,
            beside_recording: true,
        }
    }

    pub(crate) fn live_paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        assert!(!id.is_invalid(), "invalid paintable arena slot ID");
        let data = self
            .paintable_rows
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

    fn paintable_data_by_index(&self, index: u32) -> &PaintableData {
        self.paintable_rows
            .rows
            .get(index as usize)
            .expect("invalid paintable arena slot index")
    }

    pub(crate) fn transfer_fragments_to_replacement_node(
        &self,
        containing_block: NodeSlotId,
        old_node: NodeSlotId,
        new_node: NodeSlotId,
    ) {
        let _writer = crate::painting::published_immutable::enter_writer_if_unattributed("layout node replacement");
        if !self.paintable_row_is_populated(containing_block) {
            return;
        }
        self.push_paint_damage(containing_block, PaintDamage::ALL_PRODUCERS);
        if self.live_committed_side_data(containing_block).inline_content.is_none() {
            return;
        }
        let mut side = self.committed_side_data_mut(containing_block);
        let Some(content) = side.inline_content.as_mut() else {
            return;
        };
        // Replacement preserves current paint geometry until the next layout commit. Give that
        // version new node identities without modifying retained run outputs or copying glyphs.
        let content = std::sync::Arc::make_mut(content);
        for fragment in &mut content.fragments {
            if fragment.layout_node == old_node {
                fragment.layout_node = new_node;
            }
        }
        for piece in &mut content.inline_box_pieces {
            if piece.node == old_node {
                piece.node = new_node;
            }
        }
    }

    pub(crate) fn push_propagated_text_decoration_damage(&self, root: NodeSlotId) {
        self.paintable_rows().push_propagated_text_decoration_damage(root);
    }

    pub(crate) fn paintable_side_data(&self, id: NodeSlotId) -> Ref<'_, PaintableSideData> {
        debug_assert!(self.paintable_row_is_populated(id));
        Ref::map(self.paintable_rows.side_data.borrow(), |side_data| {
            &side_data[id.slot_index() as usize]
        })
    }

    pub(crate) fn svg_filter_bounds(&self, id: NodeSlotId) -> Option<used_values::FfiCssPixelRect> {
        self.live_committed_side_data(id).svg_filter_bounds
    }

    pub(crate) fn fragment_ownership_filter(
        &self,
        id: NodeSlotId,
    ) -> Option<crate::painting::fragment_ownership::FragmentOwnershipFilter> {
        self.live_committed_side_data(id).fragment_ownership.as_deref().cloned()
    }

    pub(crate) fn paintable_side_data_mut(&self, id: NodeSlotId) -> RefMut<'_, PaintableSideData> {
        debug_assert!(self.paintable_row_is_populated(id));
        RefMut::map(self.paintable_rows.side_data.borrow_mut(), |side_data| {
            &mut side_data[id.slot_index() as usize]
        })
    }

    /// The side data a row is committing, as the render side reads it. The main side reads it
    /// through [`PaintableRowsRead::committed_side_data`], which sees the published generation.
    pub(crate) fn live_committed_side_data(&self, id: NodeSlotId) -> Ref<'_, CommittedSideData> {
        debug_assert!(self.paintable_row_is_populated(id));
        Ref::map(self.paintable_rows.committed_side_data.borrow(), |side_data| {
            side_data
                .get(id.slot_index() as usize)
                .expect("invalid paintable arena slot ID")
        })
    }

    /// Writing copies the row's chunk if a published generation shares it, so a writer checks
    /// first that it changes something.
    pub(crate) fn committed_side_data_mut(&self, id: NodeSlotId) -> RefMut<'_, CommittedSideData> {
        debug_assert!(self.paintable_row_is_populated(id));
        RefMut::map(self.paintable_rows.committed_side_data.borrow_mut(), |side_data| {
            side_data
                .get_mut(id.slot_index() as usize)
                .expect("invalid paintable arena slot ID")
        })
    }
}

pub(crate) fn with_inline_pieces(
    arena: &impl PaintableRowsRead,
    inline_paintable: NodeSlotId,
    mut callback: impl FnMut(&InlineBoxPieceRecord, &PaintableData) -> bool,
) {
    let Some(root) = arena.inline_pieces_root(inline_paintable) else {
        return;
    };
    let data = arena.paintable_data(inline_paintable);
    let root_side = arena.committed_side_data(root);
    for piece_index in arena.committed_side_data(inline_paintable).piece_indices() {
        let piece = &root_side.inline_box_pieces()[*piece_index as usize];
        if !callback(piece, data) {
            return;
        }
    }
}
