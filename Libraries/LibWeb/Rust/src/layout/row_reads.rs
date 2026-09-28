/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The rows of a document's layout as the document thread reads them: the row a node is bound to, the rows linked to
//! one, and what each row is. The arena publishes them as one [`RowSnapshot`] wherever the render owner ends a unit
//! that may have changed them (a job of a layout frame, a rendering update, a style transaction, a clock tick, the
//! changes a unit or a question comes after), before it answers, and wherever a main thread entry changed them. The
//! document thread reads the latest one where the arena keeps it, reaching neither the arena nor the owner.

use super::LayoutNodeArena;
use super::layout_node_arena::{BOUND_ROWS_PER_CHUNK, PseudoElementRows, SLOTS_PER_CHUNK};
use super::node_data::{FfiNodeLink, NodeKind, NodeSlotId, PaintNode};
use super::node_facts;
use super::tree_shape::PublishedStyle;
use crate::cow_column::ColumnSnapshot;
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::css_enums::positioning;
use crate::css::style::published_record::PublishedStyleRecord;
use crate::css::style::tree::StyleNodeID;
use smallvec::SmallVec;
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicPtr, Ordering};

/// The rows of a document's layout as the arena published them: the shape and style of every row, and the row each
/// node is bound to. It is immutable and owns all of it, through the copy-on-write generations the arena's columns
/// publish, so it is read on the document thread while the arena goes on changing.
#[derive(Default)]
pub(crate) struct RowSnapshot {
    pub(super) nodes: ColumnSnapshot<PaintNode, SLOTS_PER_CHUNK>,
    /// Every row's style, which the snapshot keeps alive for the reads of it.
    pub(super) styles: ColumnSnapshot<PublishedStyle, SLOTS_PER_CHUNK>,
    /// The row each element is bound to, by its element index, and each text node, by its text index.
    pub(super) element_rows: ColumnSnapshot<NodeSlotId, BOUND_ROWS_PER_CHUNK>,
    pub(super) text_rows: ColumnSnapshot<NodeSlotId, BOUND_ROWS_PER_CHUNK>,
    pub(super) pseudo_element_rows: Arc<PseudoElementRows>,
    /// The row the document is bound to: the viewport's.
    pub(super) viewport_row: NodeSlotId,
}

// A snapshot is read on the document thread while the arena is written wherever its owner runs: it holds no cell, no
// borrow and no handle of the arena.
const _: () = {
    const fn assert_published<T: Send + Sync + 'static>() {}
    assert_published::<RowSnapshot>();
};

impl RowSnapshot {
    /// The rows the arena `handle` names published last, once the frame in flight, which publishes them again, is
    /// taken back.
    ///
    /// # Safety
    ///
    /// `handle` must be a live handle on the document thread, and the borrow must end before the document thread
    /// waits for the owner or lets it run a unit of the document.
    #[track_caller]
    pub(crate) unsafe fn published<'a>(handle: *mut c_void) -> &'a Self {
        assert!(!handle.is_null(), "layout node arena handle is null");
        crate::stage_thread::join_frame_in_flight(handle);
        // SAFETY: Guaranteed by the caller. A handle is also a pointer to its arena, and the projection borrows nothing
        // of the arena beside the slot, which only the arena's owner writes, in a unit the document thread is not in.
        unsafe { (*std::ptr::addr_of!((*handle.cast::<LayoutNodeArena>()).published_rows)).latest() }
    }

    pub(crate) fn node(&self, id: NodeSlotId) -> Option<&PaintNode> {
        if id.is_invalid() {
            return None;
        }
        self.nodes
            .get(id.slot_index() as usize)
            .filter(|node| node.generation != 0 && node.generation == id.generation())
    }

    /// The style record of the row in the live slot `id`.
    pub(crate) fn style_record(&self, id: NodeSlotId) -> Option<&PublishedStyleRecord> {
        self.node(id)?;
        self.styles.get(id.slot_index() as usize)?.0.as_deref()
    }

    pub(crate) fn style(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>> {
        let record = self.style_record(id)?;
        Some(ComputedValuesView::new(&record.payloads.as_ffi().groups))
    }

    /// The row the element or text node with `style_node` is bound to.
    pub(crate) fn bound_row(&self, style_node: StyleNodeID) -> Option<NodeSlotId> {
        let row = match style_node.element_index() {
            Some(index) => self.element_rows.get(index as usize),
            None => self.text_rows.get(style_node.text_index()? as usize),
        };
        row.copied().filter(|row| self.node(*row).is_some())
    }

    /// The row the pseudo-element of kind `generated_for` on the element with `generator` is bound to.
    pub(crate) fn bound_pseudo_element_row(&self, generator: StyleNodeID, generated_for: u8) -> Option<NodeSlotId> {
        self.pseudo_element_rows
            .get(&(generator, generated_for))
            .copied()
            .filter(|row| self.node(*row).is_some())
    }

    /// The row the document is bound to: the viewport.
    pub(crate) fn viewport_row(&self) -> Option<NodeSlotId> {
        self.node(self.viewport_row).is_some().then_some(self.viewport_row)
    }

    pub(crate) fn parent(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        let parent = self.node(id)?.parent;
        (!parent.is_invalid()).then_some(parent)
    }

    /// The box whose content box the row in `id` is laid out against, found by walking its ancestors as the arena's
    /// own walk does.
    pub(crate) fn containing_block(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        let node = self.node(id)?;
        let position = if node_facts::kind_is_text(node.kind) {
            positioning::STATIC
        } else {
            node_facts::node_position(self.style(id))
        };
        if position != positioning::ABSOLUTE && position != positioning::FIXED {
            let mut ancestor = self.parent(id);
            while let Some(candidate) = ancestor {
                if node_facts::node_forms_containing_block_for_children(self.node(candidate)?, self.style(candidate)) {
                    return Some(candidate);
                }
                ancestor = self.parent(candidate);
            }
            return None;
        }
        let is_fixed_position = position == positioning::FIXED;
        let establishes_containing_block = node_facts::containing_block_establishment_flag(is_fixed_position);
        let mut current = id;
        while let Some(ancestor) = self.parent(current) {
            current = ancestor;
            if self.node(current).is_some_and(|node| {
                node_facts::kind_is_box(node.kind) && node_facts::has_flag(node, establishes_containing_block)
            }) {
                return Some(current);
            }
        }
        // A fixed-position box with no ancestor establishing its containing block is laid out against the root.
        is_fixed_position.then_some(current)
    }

    /// The rows built for the same DOM node as the row in `id`, that row first.
    fn rows_built_for_same_node(&self, id: NodeSlotId) -> SmallVec<[NodeSlotId; 4]> {
        let mut rows = SmallVec::new();
        let Some(node) = self.node(id) else {
            return rows;
        };
        rows.push(id);
        let mut row = node.next_row_built_for_same_node;
        while !row.is_invalid() && row != id {
            rows.push(row);
            row = self
                .node(row)
                .map_or(NodeSlotId::INVALID, |node| node.next_row_built_for_same_node);
        }
        rows
    }

    fn row(&self, id: Option<NodeSlotId>) -> FfiBoundRow {
        id.and_then(|id| Some((id, self.node(id)?.kind)))
            .map_or(FfiBoundRow::NONE, |(slot, kind)| FfiBoundRow { slot, kind })
    }
}

/// Where the arena keeps the latest [`RowSnapshot`] it published. The arena's owner replaces it only where the
/// document thread cannot be reading it: in a unit the document thread waits for, takes back before it reads, or idles
/// through.
pub(crate) struct RowSnapshotSlot(AtomicPtr<RowSnapshot>);

impl Default for RowSnapshotSlot {
    fn default() -> Self {
        Self(AtomicPtr::new(Arc::into_raw(Arc::<RowSnapshot>::default()).cast_mut()))
    }
}

impl Drop for RowSnapshotSlot {
    fn drop(&mut self) {
        // SAFETY: The slot holds the reference it took in `publish`, or in `default`.
        drop(unsafe { Arc::from_raw(*self.0.get_mut()) });
    }
}

impl RowSnapshotSlot {
    /// Replaces the rows the slot holds with `rows`, on the arena's owner.
    pub(super) fn publish(&self, rows: Arc<RowSnapshot>) {
        let previous = self.0.swap(Arc::into_raw(rows).cast_mut(), Ordering::AcqRel);
        // SAFETY: The slot held the reference `previous` came from, and nothing borrows it: see the type.
        drop(unsafe { Arc::from_raw(previous) });
    }

    /// The rows the slot holds, for the arena's owner to share.
    pub(super) fn shared(&self) -> Arc<RowSnapshot> {
        let rows = self.0.load(Ordering::Acquire);
        // SAFETY: The slot holds a reference to `rows`, which only its owner, the calling thread, lets go of.
        unsafe {
            Arc::increment_strong_count(rows);
            Arc::from_raw(rows)
        }
    }

    /// # Safety
    ///
    /// Nothing may publish rows while the borrow is live.
    unsafe fn latest<'a>(&self) -> &'a RowSnapshot {
        // SAFETY: Guaranteed by the caller.
        unsafe { &*self.0.load(Ordering::Acquire) }
    }
}

/// A row the host names by its slot.
#[repr(C)]
pub struct FfiBoundRow {
    /// The row, or an invalid slot if there is none.
    pub slot: NodeSlotId,
    pub kind: NodeKind,
}

impl FfiBoundRow {
    const NONE: Self = Self {
        slot: NodeSlotId::INVALID,
        kind: NodeKind::Unset,
    };
}

/// The row the element or text node with `style_node` is bound to, or, for a nonzero `generated_for`, the row of its
/// pseudo-element of that kind.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_bound_row_of(
    arena: *mut c_void,
    style_node: u32,
    generated_for: u8,
) -> FfiBoundRow {
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return FfiBoundRow::NONE;
    };
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { RowSnapshot::published(arena) };
    rows.row(if generated_for == 0 {
        rows.bound_row(style_node)
    } else {
        rows.bound_pseudo_element_row(style_node, generated_for)
    })
}

/// The viewport row the document is bound to.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_bound_viewport_row(arena: *mut c_void) -> FfiBoundRow {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { RowSnapshot::published(arena) };
    rows.row(rows.viewport_row())
}

/// The row `slot` links to by `link`.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_linked_row(
    arena: *mut c_void,
    slot: NodeSlotId,
    link: FfiNodeLink,
) -> FfiBoundRow {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { RowSnapshot::published(arena) };
    let linked = rows.node(slot).map(|node| match link {
        FfiNodeLink::Parent => node.parent,
        FfiNodeLink::FirstChild => node.first_child,
        FfiNodeLink::NextSibling => node.next_sibling,
    });
    rows.row(linked)
}

/// The row `slot` names, or none if the row is no longer live, which a slot noted earlier may not be.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_row_if_live(arena: *mut c_void, slot: NodeSlotId) -> FfiBoundRow {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::published(arena) }.row(Some(slot))
}

/// The containing block of the row `slot` names, or an invalid slot if it has none or it is no longer live.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_containing_block_slot_if_live(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> NodeSlotId {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::published(arena) }
        .containing_block(slot)
        .unwrap_or(NodeSlotId::INVALID)
}

/// Visits every row built for the same DOM node as `slot`, that row included.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread; `visit` is called synchronously with `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_for_each_row_built_for_same_node(
    arena: *mut c_void,
    slot: NodeSlotId,
    context: *mut c_void,
    visit: unsafe extern "C" fn(*mut c_void, NodeSlotId),
) {
    // The rows are read before the host is called, which may make the owner publish them again.
    // SAFETY: Guaranteed by the caller.
    for row in unsafe { RowSnapshot::published(arena) }.rows_built_for_same_node(slot) {
        // SAFETY: The host answers synchronously.
        unsafe { visit(context, row) };
    }
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_flags(arena: *mut c_void, id: NodeSlotId) -> u32 {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::published(arena) }
        .node(id)
        .map_or(0, |node| node.flags)
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_generated_for(arena: *mut c_void, id: NodeSlotId) -> u8 {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::published(arena) }
        .node(id)
        .map_or(0, |node| node.generated_for)
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_style_node(arena: *mut c_void, id: NodeSlotId) -> u32 {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::published(arena) }
        .node(id)
        .and_then(|node| node.style_node)
        .map_or(0, StyleNodeID::raw)
}

/// The group payloads of the row's style record, or null for a row without style. They live until the owner publishes
/// the rows again without the record.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_style_payloads(arena: *mut c_void, id: NodeSlotId) -> *const c_void {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::published(arena) }
        .style_record(id)
        .map_or(std::ptr::null(), |record| record.payloads.as_ptr())
}

/// The dependency flags of the row's style record, or zero for a row without style.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_style_dependency_flags(arena: *mut c_void, id: NodeSlotId) -> u8 {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::published(arena) }
        .style_record(id)
        .map_or(0, |record| record.dependency_flags)
}

#[cfg(test)]
mod tests {
    use crate::css::style::tree::StyleNodeID;
    use crate::layout::LayoutNodeArena;
    use crate::layout::node_data::{NodeConstructionFacts, NodeKind, NodeSlotId};

    fn row(arena: &mut LayoutNodeArena, kind: NodeKind, style_node: Option<StyleNodeID>) -> NodeSlotId {
        arena.allocate(NodeConstructionFacts {
            kind,
            is_anonymous: false,
            dom_paint_facts: 0,
            style_node: style_node.map_or(0, StyleNodeID::raw),
        })
    }

    #[test]
    fn a_row_snapshot_reads_the_rows_as_the_arena_published_them() {
        let mut arena = LayoutNodeArena::new();
        let element = StyleNodeID::element(3);
        let text = StyleNodeID::text(2);
        let viewport = row(&mut arena, NodeKind::Viewport, None);
        let block = row(&mut arena, NodeKind::BlockContainer, Some(element));
        let referencer = row(&mut arena, NodeKind::BlockContainer, Some(element));
        let text_row = row(&mut arena, NodeKind::TextNode, Some(text));
        arena.insert_child(viewport, block, NodeSlotId::INVALID);
        arena.insert_child(viewport, referencer, NodeSlotId::INVALID);
        arena.insert_child(block, text_row, NodeSlotId::INVALID);
        for bound in [viewport, block, text_row] {
            arena.bind_row(bound);
        }
        arena.note_rows_share_dom_node(block, referencer);
        arena.publish_rows();
        let rows = arena.published_rows();

        // What the arena does after it published shows in the next snapshot, not in this one.
        arena.unbind_row(text_row);
        arena.remove_child(block, text_row);
        arena.free_subtree(text_row).invoke_callbacks();

        assert_eq!(rows.viewport_row(), Some(viewport));
        assert_eq!(rows.bound_row(element), Some(block));
        assert_eq!(rows.bound_row(text), Some(text_row));
        assert_eq!(rows.parent(text_row), Some(block));
        assert_eq!(rows.node(block).unwrap().first_child, text_row);
        assert_eq!(rows.containing_block(text_row), Some(block));
        assert_eq!(
            rows.rows_built_for_same_node(referencer).as_slice(),
            [referencer, block]
        );
        assert_eq!(rows.rows_built_for_same_node(text_row).as_slice(), [text_row]);

        arena.publish_rows();
        let later = arena.published_rows();
        assert_eq!(later.bound_row(text), None);
        assert!(later.node(text_row).is_none());
        assert_eq!(later.node(block).unwrap().first_child, NodeSlotId::INVALID);
        assert_eq!(rows.bound_row(text), Some(text_row));

        arena.remove_child(viewport, block);
        arena.remove_child(viewport, referencer);
        for freed in [block, referencer, viewport] {
            arena.free_subtree(freed).invoke_callbacks();
        }
    }
}
