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
//! A node's style is not in its [`PaintNode`]: the row owns its record's payloads through a
//! [`StyleCell`], and the arena publishes those owners in a column of their own, so a publication
//! holds every style it names and a row copies a reference count only when its style changes.
//!
//! The fields a [`PaintNode`] copies are [`ShapeCell`]s. They read like a `Cell`, but only a
//! [`ShapeWriter`] writes one, and only a [`Chunk`] hands out a writer, through which a write that
//! changes a field marks its node in the chunk. A write the next publication would miss does not
//! compile.
//!
//! A slot freed while a publication that may name it is alive is retired rather than reused: it
//! joins the [`RetireEpoch`] of the latest publication, and the epoch hands it back to the arena
//! when it is dropped. An earlier publication keeps every later epoch alive, since a slot freed
//! after a later publication was live when the earlier one was made, so a slot comes back once no
//! publication that could name it is left. A slot freed while no publication is alive is reused at
//! once, as before.

use super::layout_node_arena::SLOTS_PER_CHUNK;
use super::node_data::{NodeData, NodeKind, NodeSlotId, PaintNode, StylePayloadsRef};
use crate::cow_column::{ColumnSnapshot, CowColumn};
use crate::css::style::record_payloads::StyleRecordPayloads;
use std::cell::Cell;
use std::ops::Deref;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError, Weak};

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

/// A node's style: the payloads of its record, which the row owns. It reads like a [`ShapeCell`] of
/// the payload pointer, and only a [`ShapeWriter`] writes it.
pub(crate) struct StyleCell(Cell<Option<Arc<StyleRecordPayloads>>>);

impl StyleCell {
    pub(crate) const fn new() -> Self {
        Self(Cell::new(None))
    }

    /// The row's payload pointer array, or null for a row without style.
    #[inline]
    pub(crate) fn get(&self) -> StylePayloadsRef {
        self.with_owner(|owner| {
            owner.map_or(StylePayloadsRef::null(), |payloads| {
                StylePayloadsRef::new(payloads.as_ptr())
            })
        })
    }

    /// A reference of the row's own on its payloads, for a publication to hold.
    pub(crate) fn owner(&self) -> Option<Arc<StyleRecordPayloads>> {
        self.with_owner(|owner| owner.cloned())
    }

    fn owner_address(&self) -> usize {
        self.with_owner(|owner| owner.map_or(0, |payloads| Arc::as_ptr(payloads).addr()))
    }

    #[inline]
    fn with_owner<R>(&self, read: impl FnOnce(Option<&Arc<StyleRecordPayloads>>) -> R) -> R {
        // SAFETY: The cell is not `Sync`, and `read` cannot reach the cell: every caller above hands
        // it a closure that only reads the owner it is given, so no write replaces the value while
        // the reference lives.
        read(unsafe { &*self.0.as_ptr() }.as_ref())
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

    pub(crate) fn set_style(&self, style: Option<Arc<StyleRecordPayloads>>) {
        let address = style.as_ref().map_or(0, |payloads| Arc::as_ptr(payloads).addr());
        if self.data.style.owner_address() != address {
            self.written_rows.set(self.written_rows.get() | self.row_bit);
            drop(self.data.style.0.replace(style));
        }
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

/// A node's style owner as a published row. A row is the same as another when it names the same
/// owner: an owner's payloads never change once published.
#[derive(Clone, Default)]
pub(crate) struct PublishedStyle(pub(crate) Option<Arc<StyleRecordPayloads>>);

impl PublishedStyle {
    fn address(&self) -> Option<usize> {
        self.0.as_ref().map(|payloads| Arc::as_ptr(payloads).addr())
    }
}

impl PartialEq for PublishedStyle {
    fn eq(&self, other: &Self) -> bool {
        crate::cow_column::same_payload(self.0.as_ref(), other.0.as_ref(), |_, _| false)
    }
}

/// The arena's column of what the paint side reads of every node, which it publishes from, and the
/// slots its publications retire.
pub(crate) struct TreeShape {
    nodes: CowColumn<PaintNode, SLOTS_PER_CHUNK>,
    /// Every node's style owner, beside its row in `nodes`.
    styles: CowColumn<PublishedStyle, SLOTS_PER_CHUNK>,
    /// The epoch of the latest publication, while one that holds it is alive.
    latest_epoch: Weak<RetireEpoch>,
    /// Where dropped epochs send the slots they retired.
    returned_slots: Receiver<Vec<u32>>,
    returns: Sender<Vec<u32>>,
}

impl Default for TreeShape {
    fn default() -> Self {
        let (returns, returned_slots) = channel();
        Self {
            nodes: CowColumn::default(),
            styles: CowColumn::default(),
            latest_epoch: Weak::new(),
            returned_slots,
            returns,
        }
    }
}

impl TreeShape {
    /// Brings the rows of every node written since the last publication up to date and publishes
    /// the column. Slots freed from now on are retired until the returned [`RetiredSlots`] is
    /// dropped.
    pub(crate) fn publish(&mut self, chunks: &[Box<Chunk>]) -> PublishedShape {
        self.update(chunks);
        let epoch = Arc::new(RetireEpoch {
            slots: Mutex::default(),
            later: OnceLock::new(),
            returns: self.returns.clone(),
        });
        if let Some(previous) = self.latest_epoch.upgrade() {
            let chained = previous.later.set(epoch.clone());
            debug_assert!(chained.is_ok(), "only the latest epoch is chained to");
        }
        self.latest_epoch = Arc::downgrade(&epoch);
        PublishedShape {
            nodes: self.nodes.publish(),
            styles: self.styles.publish(),
            retired_slots: RetiredSlots { _epoch: epoch },
        }
    }

    /// Retires a freed slot while a publication that may name it is alive. A slot that is not
    /// retired is free to reuse at once.
    pub(crate) fn retire(&self, index: u32) -> bool {
        let Some(epoch) = self.latest_epoch.upgrade() else {
            return false;
        };
        epoch.slots.lock().unwrap_or_else(PoisonError::into_inner).push(index);
        true
    }

    /// Moves the slots whose publications are all gone to `free_list`.
    pub(crate) fn reclaim_retired_slots(&self, free_list: &mut Vec<u32>) {
        while let Ok(slots) = self.returned_slots.try_recv() {
            free_list.extend(slots);
        }
    }

    /// Brings the rows of every node written since the last call up to date. A row whose node did
    /// not change is not written, so a chunk an earlier publication shares is copied only for a
    /// change.
    fn update(&mut self, chunks: &[Box<Chunk>]) {
        self.nodes.grow_to(chunks.len() * SLOTS_PER_CHUNK);
        self.styles.grow_to(chunks.len() * SLOTS_PER_CHUNK);
        for (chunk_index, chunk) in chunks.iter().enumerate() {
            for (word_index, word) in chunk.written_rows.iter().enumerate() {
                let mut written = word.replace(0);
                while written != 0 {
                    let offset = word_index * 64 + written.trailing_zeros() as usize;
                    written &= written - 1;
                    let index = chunk_index * SLOTS_PER_CHUNK + offset;
                    let data = &chunk.slots[offset];
                    self.nodes
                        .set(index, PaintNode::of(data))
                        .expect("the column holds every chunk");
                    // Clone the owner only for a row whose owner changed.
                    let published_style = self.styles.get(index).and_then(PublishedStyle::address);
                    if published_style.unwrap_or(0) != data.style.owner_address() {
                        self.styles
                            .set(index, PublishedStyle(data.style.owner()))
                            .expect("the column holds every chunk");
                    }
                }
            }
        }
    }
}

/// What one publication of the tree's shape holds: every node's row, the style each row names, and
/// the slots freed since, which are not reused while it is alive.
pub(crate) struct PublishedShape {
    pub(crate) nodes: ColumnSnapshot<PaintNode, SLOTS_PER_CHUNK>,
    pub(crate) styles: ColumnSnapshot<PublishedStyle, SLOTS_PER_CHUNK>,
    pub(crate) retired_slots: RetiredSlots,
}

/// Keeps the slots freed after a publication from being reused while it is alive.
pub(crate) struct RetiredSlots {
    _epoch: Arc<RetireEpoch>,
}

/// The slots freed after one publication and before the next. The publication holds it, and so does
/// the epoch of the publication before, so it is dropped once no publication that could name one of
/// its slots is alive. Dropping it hands its slots back to the arena.
struct RetireEpoch {
    slots: Mutex<Vec<u32>>,
    later: OnceLock<Arc<RetireEpoch>>,
    returns: Sender<Vec<u32>>,
}

impl RetireEpoch {
    fn hand_back(&mut self) {
        let slots = std::mem::take(self.slots.get_mut().unwrap_or_else(PoisonError::into_inner));
        if !slots.is_empty() {
            // An arena that is gone has no use for its slots.
            let _ = self.returns.send(slots);
        }
    }
}

impl Drop for RetireEpoch {
    fn drop(&mut self) {
        self.hand_back();
        // Drop the chain of later epochs this one alone held in a loop rather than recursively.
        let mut later = self.later.take();
        while let Some(epoch) = later {
            let Some(mut epoch) = Arc::into_inner(epoch) else {
                break;
            };
            epoch.hand_back();
            later = epoch.later.take();
        }
    }
}

#[cfg(test)]
impl TreeShape {
    /// Whether the column changed since it was last published, once it is brought up to date.
    pub(crate) fn changed_since_publish(&mut self, chunks: &[Box<Chunk>]) -> bool {
        self.update(chunks);
        self.nodes.written_since_publish() || self.styles.written_since_publish()
    }
}

#[cfg(test)]
mod tests {
    use crate::cow_column::ColumnSnapshot;
    use crate::layout::LayoutNodeArena;
    use crate::layout::SLOTS_PER_CHUNK;
    use crate::layout::node_data::{NodeFlag, NodeKind, NodeSlotId, PaintNode};
    use crate::layout::tree_shape::PublishedShape;

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
        let PublishedShape {
            nodes,
            retired_slots: _retired,
            ..
        } = arena.publish_paint_tree();
        for id in [root, first, second] {
            assert!(node(&nodes, id) == Some(&PaintNode::of(arena.data(id))));
        }
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn a_published_column_does_not_see_later_writes() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, second) = tree(&mut arena);
        let PublishedShape {
            nodes,
            retired_slots: _retired,
            ..
        } = arena.publish_paint_tree();

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

        let PublishedShape {
            nodes: later,
            retired_slots: _retired_later,
            ..
        } = arena.publish_paint_tree();
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
        let _published = arena.publish_paint_tree();
        assert!(!arena.paint_tree_changed_since_publish());
        arena.set_node_flag(first, NodeFlag::NeedsLayoutUpdate, true);
        arena.set_node_flag(first, NodeFlag::NeedsLayoutUpdate, false);
        assert!(!arena.paint_tree_changed_since_publish());
        arena.set_node_flag(first, NodeFlag::IsGridItem, true);
        assert!(arena.paint_tree_changed_since_publish());
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn a_slot_a_live_publication_names_is_not_reused_until_it_is_dropped() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, _) = tree(&mut arena);
        let PublishedShape {
            nodes,
            retired_slots: retired,
            ..
        } = arena.publish_paint_tree();
        arena.remove_child(root, first);
        arena.free_subtree(first).destroy_shells_and_invoke_callbacks();

        let while_published = arena.allocate_for_test().slot;
        assert_ne!(while_published.slot_index(), first.slot_index());
        assert_eq!(node(&nodes, first).unwrap().kind, NodeKind::InlineNode);

        drop((nodes, retired));
        let once_dropped = arena.allocate_for_test().slot;
        assert_eq!(once_dropped.slot_index(), first.slot_index());
        assert_ne!(once_dropped.generation(), first.generation());
        arena
            .free_subtree(while_published)
            .destroy_shells_and_invoke_callbacks();
        arena.free_subtree(once_dropped).destroy_shells_and_invoke_callbacks();
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn an_earlier_publication_keeps_the_slots_freed_after_a_later_one() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, _) = tree(&mut arena);
        let PublishedShape {
            nodes: earlier,
            retired_slots: earlier_retired,
            ..
        } = arena.publish_paint_tree();
        let later = arena.publish_paint_tree();
        arena.remove_child(root, first);
        arena.free_subtree(first).destroy_shells_and_invoke_callbacks();

        drop(later);
        let while_earlier_is_alive = arena.allocate_for_test().slot;
        assert_ne!(while_earlier_is_alive.slot_index(), first.slot_index());
        assert_eq!(node(&earlier, first).unwrap().parent, root);

        drop((earlier, earlier_retired));
        let once_both_are_dropped = arena.allocate_for_test().slot;
        assert_eq!(once_both_are_dropped.slot_index(), first.slot_index());
        arena
            .free_subtree(while_earlier_is_alive)
            .destroy_shells_and_invoke_callbacks();
        arena
            .free_subtree(once_both_are_dropped)
            .destroy_shells_and_invoke_callbacks();
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn a_slot_freed_with_no_publication_alive_is_reused_at_once() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, _) = tree(&mut arena);
        drop(arena.publish_paint_tree());
        arena.remove_child(root, first);
        arena.free_subtree(first).destroy_shells_and_invoke_callbacks();
        let reused = arena.allocate_for_test().slot;
        assert_eq!(reused.slot_index(), first.slot_index());
        arena.free_subtree(reused).destroy_shells_and_invoke_callbacks();
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn a_publication_dropped_on_another_thread_hands_its_slots_back() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, _) = tree(&mut arena);
        let published = arena.publish_paint_tree();
        arena.remove_child(root, first);
        arena.free_subtree(first).destroy_shells_and_invoke_callbacks();
        std::thread::spawn(move || {
            let PublishedShape {
                nodes,
                retired_slots: _retired,
                ..
            } = published;
            assert_eq!(node(&nodes, first).unwrap().parent, root);
        })
        .join()
        .unwrap();
        let reused = arena.allocate_for_test().slot;
        assert_eq!(reused.slot_index(), first.slot_index());
        arena.free_subtree(reused).destroy_shells_and_invoke_callbacks();
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }

    #[test]
    fn a_slot_a_live_published_frame_names_is_not_reused_until_it_is_dropped() {
        let mut arena = LayoutNodeArena::new();
        let (root, first, _) = tree(&mut arena);
        let frame = arena.freeze_paint_frame();
        arena.remove_child(root, first);
        arena.free_subtree(first).destroy_shells_and_invoke_callbacks();

        let while_published = arena.allocate_for_test().slot;
        assert_ne!(while_published.slot_index(), first.slot_index());

        std::thread::spawn(move || drop(frame)).join().unwrap();
        let once_dropped = arena.allocate_for_test().slot;
        assert_eq!(once_dropped.slot_index(), first.slot_index());
        arena
            .free_subtree(while_published)
            .destroy_shells_and_invoke_callbacks();
        arena.free_subtree(once_dropped).destroy_shells_and_invoke_callbacks();
        arena.free_subtree(root).destroy_shells_and_invoke_callbacks();
    }
}
