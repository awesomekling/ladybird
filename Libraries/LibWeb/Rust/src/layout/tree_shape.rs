/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The layout tree's shape as the paint side reads it, published copy-on-write.
//!
//! A node's links, kind, paint facts and style live in its [`NodeData`], which tree builds, style
//! installs and invalidation write in place. A recording that read them there would race the next
//! layout. The arena therefore keeps a [`CowColumn`] of [`PaintNode`]s beside its node chunks, and
//! brings the rows of every node written since it last published up to date when it publishes
//! again, so publishing costs the nodes written since, not the whole tree.
//!
//! The fields a [`PaintNode`] copies are [`ShapeCell`]s. They read like a `Cell`, but only a
//! [`ShapeWriter`] writes one, and only a [`Chunk`] hands out a writer, through which a write that
//! changes a field marks its node in the chunk. A write the next publication would miss does not
//! compile.

use super::layout_node_arena::SLOTS_PER_CHUNK;
use super::node_data::{NodeData, NodeKind, NodeSlotId, PaintNode, StylePayloadsRef};
use crate::cow_column::{ColumnSnapshot, CowColumn};
use std::cell::Cell;
use std::ops::Deref;

/// A field of a node that a [`PaintNode`] copies. It reads like a `Cell`, and is written through a
/// [`ShapeWriter`] or through `&mut`, both of which mark the node in its chunk.
#[repr(transparent)]
pub(crate) struct ShapeCell<T>(Cell<T>);

impl<T: Copy> ShapeCell<T> {
    pub(crate) const fn new(value: T) -> Self {
        Self(Cell::new(value))
    }

    #[inline]
    pub(crate) fn get(&self) -> T {
        self.0.get()
    }

    #[inline]
    pub(crate) fn get_mut(&mut self) -> &mut T {
        self.0.get_mut()
    }

    #[inline]
    fn set(&self, value: T) {
        self.0.set(value);
    }
}

/// Writes the shape of one node, marking it in its chunk for the next publication when a write
/// changes it. It reads as the node's [`NodeData`], so the fields a publication does not copy are written
/// through it as before.
pub(crate) struct ShapeWriter<'a> {
    data: &'a NodeData,
    written_rows: &'a Cell<u64>,
    row_bit: u64,
}

impl Deref for ShapeWriter<'_> {
    type Target = NodeData;

    fn deref(&self) -> &NodeData {
        self.data
    }
}

impl ShapeWriter<'_> {
    /// Writes a field, marking the chunk only for a value the field does not already hold.
    #[inline]
    fn write<T: Copy + PartialEq>(&self, field: &ShapeCell<T>, value: T) {
        if field.get() != value {
            self.written_rows.set(self.written_rows.get() | self.row_bit);
            field.set(value);
        }
    }

    pub(crate) fn set_parent(&self, parent: NodeSlotId) {
        self.write(&self.data.parent, parent);
    }

    pub(crate) fn set_first_child(&self, child: NodeSlotId) {
        self.write(&self.data.first_child, child);
    }

    pub(crate) fn set_next_sibling(&self, sibling: NodeSlotId) {
        self.write(&self.data.next_sibling, sibling);
    }

    pub(crate) fn set_kind(&self, kind: NodeKind) {
        self.write(&self.data.kind, kind);
    }

    pub(crate) fn set_generated_for(&self, generated_for: u8) {
        self.write(&self.data.generated_for, generated_for);
    }

    pub(crate) fn set_flags(&self, flags: u32) {
        self.write(&self.data.flags, flags);
    }

    pub(crate) fn set_dom_paint_facts(&self, facts: u8) {
        self.write(&self.data.dom_paint_facts, facts);
    }

    pub(crate) fn set_compositor_animation_frame_kinds(&self, kinds: u8) {
        self.write(&self.data.compositor_animation_frame_kinds, kinds);
    }

    pub(crate) fn set_style(&self, style: StylePayloadsRef) {
        self.write(&self.data.style, style);
    }
}

/// A run of the arena's nodes. It is the only way to reach a node's data, so it knows which of its
/// nodes may have changed since the last publication.
// NodeData is sized to one cache line; the aligned chunk keeps every densely-strided slot
// line-aligned, and per-slot bookkeeping lives in a parallel array so it stays that way.
#[repr(align(64))]
pub(crate) struct Chunk {
    slots: [NodeData; SLOTS_PER_CHUNK],
    /// One bit per node whose shape may have been written since the column was last brought up to
    /// date with this chunk.
    written_rows: [Cell<u64>; SLOTS_PER_CHUNK / 64],
}

impl Chunk {
    pub(crate) fn new() -> Box<Self> {
        // SAFETY: Every slot is written with NodeData::default() before the chunk is exposed. The
        // chunk is built in place on the heap because it is far too large for the stack.
        unsafe {
            let mut chunk = Box::<Self>::new_uninit();
            let slots = &raw mut (*chunk.as_mut_ptr()).slots;
            for offset in 0..SLOTS_PER_CHUNK {
                (&raw mut (*slots)[offset]).write(NodeData::default());
            }
            (&raw mut (*chunk.as_mut_ptr()).written_rows).write(std::array::from_fn(|_| Cell::new(u64::MAX)));
            chunk.assume_init()
        }
    }

    /// Where the chunk's first node is, which never changes.
    pub(crate) fn slots_address(&self) -> usize {
        (&raw const self.slots) as usize
    }

    #[inline]
    pub(crate) fn slot(&self, offset: usize) -> &NodeData {
        &self.slots[offset]
    }

    #[inline]
    pub(crate) fn slot_mut(&mut self, offset: usize) -> &mut NodeData {
        *self.written_rows[offset / 64].get_mut() |= 1 << (offset % 64);
        &mut self.slots[offset]
    }

    #[inline]
    pub(crate) fn write_shape(&self, offset: usize) -> ShapeWriter<'_> {
        ShapeWriter {
            data: &self.slots[offset],
            written_rows: &self.written_rows[offset / 64],
            row_bit: 1 << (offset % 64),
        }
    }
}

/// The arena's column of what the paint side reads of every node, which it publishes from.
#[derive(Default)]
pub(crate) struct TreeShape {
    nodes: CowColumn<PaintNode, SLOTS_PER_CHUNK>,
}

impl TreeShape {
    /// Brings the rows of every node written since the last publication up to date and publishes
    /// the column.
    pub(crate) fn publish(&mut self, chunks: &[Box<Chunk>]) -> ColumnSnapshot<PaintNode, SLOTS_PER_CHUNK> {
        self.update(chunks);
        self.nodes.publish()
    }

    /// Brings the rows of every node written since the last call up to date. A row whose node did
    /// not change is not written, so a chunk an earlier publication shares is copied only for a
    /// change.
    fn update(&mut self, chunks: &[Box<Chunk>]) {
        self.nodes.grow_to(chunks.len() * SLOTS_PER_CHUNK);
        for (chunk_index, chunk) in chunks.iter().enumerate() {
            for (word_index, word) in chunk.written_rows.iter().enumerate() {
                let mut written = word.replace(0);
                while written != 0 {
                    let offset = word_index * 64 + written.trailing_zeros() as usize;
                    written &= written - 1;
                    let index = chunk_index * SLOTS_PER_CHUNK + offset;
                    let node = PaintNode::of(&chunk.slots[offset]);
                    if self.nodes.get(index) != Some(&node) {
                        *self.nodes.get_mut(index).expect("the column holds every chunk") = node;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
impl TreeShape {
    /// Whether the column changed since it was last published, once it is brought up to date.
    pub(crate) fn changed_since_publish(&mut self, chunks: &[Box<Chunk>]) -> bool {
        self.update(chunks);
        self.nodes.written_since_publish()
    }
}

#[cfg(test)]
mod tests {
    use crate::cow_column::ColumnSnapshot;
    use crate::layout::LayoutNodeArena;
    use crate::layout::SLOTS_PER_CHUNK;
    use crate::layout::node_data::{NodeFlag, NodeKind, NodeSlotId, PaintNode};

    fn tree(arena: &mut LayoutNodeArena) -> (NodeSlotId, NodeSlotId, NodeSlotId) {
        let root = arena.allocate_for_test().slot;
        let first = arena.allocate_for_test().slot;
        let second = arena.allocate_for_test().slot;
        arena.write_shape(root).set_kind(NodeKind::BlockContainer);
        arena.write_shape(first).set_kind(NodeKind::InlineNode);
        arena.write_shape(second).set_kind(NodeKind::TextNode);
        arena.insert_child(root, first, NodeSlotId::INVALID);
        arena.insert_child(root, second, NodeSlotId::INVALID);
        (root, first, second)
    }

    fn node(nodes: &ColumnSnapshot<PaintNode, SLOTS_PER_CHUNK>, id: NodeSlotId) -> Option<&PaintNode> {
        nodes
            .get(id.slot_index() as usize)
            .filter(|node| node.generation != 0 && node.generation == id.generation())
    }

    #[test]
    fn a_published_column_holds_what_the_arena_does() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, second) = tree(&mut arena);
        arena.set_node_flag(first, NodeFlag::Anonymous, true);
        let nodes = arena.publish_paint_tree();
        for id in [root, first, second] {
            assert!(node(&nodes, id) == Some(&PaintNode::of(arena.data(id))));
        }
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn a_published_column_does_not_see_later_writes() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, second) = tree(&mut arena);
        let nodes = arena.publish_paint_tree();

        arena.remove_child(root, first);
        arena.set_node_flag(second, NodeFlag::IsFlexItem, true);
        arena.free_subtree(first).destroy_shells_and_invoke_callbacks();
        let added = arena.allocate_for_test().slot;
        arena.write_shape(added).set_kind(NodeKind::Box);
        arena.insert_child(root, added, NodeSlotId::INVALID);

        assert_eq!(node(&nodes, root).unwrap().first_child, first);
        assert_eq!(node(&nodes, first).unwrap().next_sibling, second);
        assert_eq!(node(&nodes, first).unwrap().parent, root);
        assert_eq!(node(&nodes, first).unwrap().kind, NodeKind::InlineNode);
        assert_eq!(node(&nodes, second).unwrap().flags & NodeFlag::IsFlexItem as u32, 0);
        assert!(node(&nodes, second).unwrap().next_sibling.is_invalid());

        let later = arena.publish_paint_tree();
        assert!(node(&later, first).is_none());
        assert_eq!(node(&later, root).unwrap().first_child, second);
        assert_eq!(node(&later, second).unwrap().next_sibling, added);
        assert_eq!(node(&later, added).unwrap().kind, NodeKind::Box);
        assert_ne!(node(&later, second).unwrap().flags & NodeFlag::IsFlexItem as u32, 0);
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn a_write_that_leaves_the_nodes_as_published_writes_no_row() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, _) = tree(&mut arena);
        let _nodes = arena.publish_paint_tree();
        assert!(!arena.paint_tree_changed_since_publish());
        arena.set_node_flag(first, NodeFlag::NeedsLayoutUpdate, true);
        arena.set_node_flag(first, NodeFlag::NeedsLayoutUpdate, false);
        assert!(!arena.paint_tree_changed_since_publish());
        arena.set_node_flag(first, NodeFlag::IsGridItem, true);
        assert!(arena.paint_tree_changed_since_publish());
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }
}
