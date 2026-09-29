/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The rows of a document's layout as the document thread reads them: the row a node is bound to, the rows linked to
//! one, what each row is, and what it paints (see [`crate::painting::published_frame`]). The arena publishes them as
//! one [`RowSnapshot`] wherever the render owner ends a unit that may have changed them (a job of a layout frame, a
//! rendering update, a style transaction, a clock tick, a paint pass, the changes a unit or a question comes after),
//! before it answers, and wherever a main thread entry changed them. The document thread reads the latest one where
//! the arena keeps it, reaching neither the arena nor the owner, unless it sent a change that alters them which the
//! latest one does not reflect yet: then it asks the owner to publish them again first.

use super::layout_changes::LayoutChange;
use super::layout_node_arena::{BOUND_ROWS_PER_CHUNK, PSEUDO_ELEMENT_ROWS_PER_CHUNK, PseudoElementRows};
use super::node_data::{FfiNodeLink, GENERATED_FOR_FIRST_LETTER, NodeKind, NodeSlotId, PaintNode};
use super::node_facts;
use super::tree_shape::{PUBLISHED_ROWS_PER_CHUNK, PublishedStyle};
use super::{HostTables, LayoutNodeArena};
use crate::cow_column::ColumnSnapshot;
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::style::fast_hash::FastMap as HashMap;
use crate::css::style::published_record::PublishedStyleRecord;
use crate::css::style::tree::StyleNodeID;
use crate::painting::published_frame::{PaintStatus, PublishedPaintFacts, PublishedRows};
use crate::render_owner::{ChangeSeq, ScriptForcedRead};
use smallvec::SmallVec;
use std::cell::Cell;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, PoisonError};

/// The rows of a document's layout as the arena published them: the shape and style of every row, the row each node is
/// bound to, and the paintable rows with what they paint from. It is immutable and owns all of it, through the
/// copy-on-write generations the arena's columns publish, so it is read on the document thread while the arena goes on
/// changing.
#[derive(Clone, Default)]
pub(crate) struct RowSnapshot {
    pub(super) nodes: ColumnSnapshot<PaintNode, PUBLISHED_ROWS_PER_CHUNK>,
    /// Every row's style, owned: the snapshot holds a reference on every style record's payloads it reads, so what the
    /// document's style engine reclaims meanwhile does not reach it.
    pub(super) styles: ColumnSnapshot<PublishedStyle, PUBLISHED_ROWS_PER_CHUNK>,
    /// The row each element is bound to, by its element index, and each text node, by its text index.
    pub(super) element_rows: ColumnSnapshot<NodeSlotId, BOUND_ROWS_PER_CHUNK>,
    pub(super) text_rows: ColumnSnapshot<NodeSlotId, BOUND_ROWS_PER_CHUNK>,
    /// The rows of each element's pseudo-elements, by its element index.
    pub(super) pseudo_element_rows: ColumnSnapshot<PseudoElementRows, PSEUDO_ELEMENT_ROWS_PER_CHUNK>,
    /// The row the document is bound to: the viewport's.
    pub(super) viewport_row: NodeSlotId,
    /// The last change of the document thread's the rows include.
    pub(super) changes_taken_in: ChangeSeq,
    /// How many rows the arena published before these, which orders them against the rows the document thread adopted
    /// from its display ticks.
    pub(super) generation: u64,
    /// The paintable rows, and the columns read beside them.
    pub(crate) paintable: PublishedRows,
    /// What the rows paint from beside their styles.
    pub(crate) paint_facts: PublishedPaintFacts,
    /// The arena's absolute rect memo epoch: a rect computed from rows of the same epoch holds for these.
    pub(crate) geometry_epoch: u64,
    pub(crate) paint_status: PaintStatus,
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
    /// waits for the owner, lets it run a unit of the document, or sends it a change it may take in right away.
    #[track_caller]
    pub(crate) unsafe fn published<'a>(handle: *mut c_void) -> &'a Self {
        assert!(!handle.is_null(), "layout node arena handle is null");
        crate::stage_thread::join_frame_in_flight(handle);
        // SAFETY: Guaranteed by the caller.
        unsafe { Self::read_slot(handle).latest() }
    }

    /// The slot the document thread reads the rows of the arena `handle` names from: the arena's, or the one of the
    /// rows it adopted from display ticks, where those are later.
    ///
    /// # Safety
    ///
    /// As for [`Self::published`].
    unsafe fn read_slot<'a>(handle: *mut c_void) -> &'a RowSnapshotSlot {
        // SAFETY: Guaranteed by the caller. A handle is also a pointer to its arena, and the projection borrows nothing
        // of the arena beside the slot, which only the arena's owner writes, in a unit the document thread is not in.
        let published = unsafe { &*std::ptr::addr_of!((*handle.cast::<LayoutNodeArena>()).published_rows) };
        // SAFETY: As above; only the document thread writes the rows it adopted, as it adopts them.
        let adopted = unsafe { &HostTables::beside_frame(handle).adopted_rows };
        // SAFETY: As above.
        if unsafe { adopted.latest() }.generation > unsafe { published.latest() }.generation {
            adopted
        } else {
            published
        }
    }

    /// The rows the arena `handle` names published last, as of every change the document thread sent that alters them
    /// ([`crate::render_owner::ArenaChange`] says which do): where the owner has not published rows that reflect those
    /// yet, the document thread asks it to.
    ///
    /// # Safety
    ///
    /// As for [`Self::published`].
    #[track_caller]
    pub(crate) unsafe fn current<'a>(handle: *mut c_void, read: ScriptForcedRead) -> &'a Self {
        // SAFETY: Guaranteed by the caller.
        unsafe { Self::published_as_of_sent_changes(handle, Freshness::Current, read) }
    }

    /// Like [`Self::current`], with the rows as committed: once the scrollable overflow a commit or a writer left is
    /// measured.
    ///
    /// # Safety
    ///
    /// As for [`Self::published`].
    #[track_caller]
    pub(crate) unsafe fn committed<'a>(handle: *mut c_void, read: ScriptForcedRead) -> &'a Self {
        // SAFETY: Guaranteed by the caller.
        unsafe { Self::published_as_of_sent_changes(handle, Freshness::Committed, read) }
    }

    /// Like [`Self::current`], as of every change the document thread sent, for a test that reads what the owner counts
    /// after them.
    ///
    /// # Safety
    ///
    /// As for [`Self::published`].
    #[track_caller]
    pub(crate) unsafe fn settled<'a>(handle: *mut c_void, read: ScriptForcedRead) -> &'a Self {
        // SAFETY: Guaranteed by the caller.
        unsafe { Self::published_as_of_sent_changes(handle, Freshness::Settled, read) }
    }

    /// Like [`Self::current`], shared, so the rows outlive the next publication.
    ///
    /// # Safety
    ///
    /// `handle` must be a live handle on the document thread.
    pub(crate) unsafe fn current_shared(handle: *mut c_void, read: ScriptForcedRead) -> Arc<Self> {
        // SAFETY: Guaranteed by the caller.
        unsafe { Self::current(handle, read) };
        // SAFETY: As above; the rows were just read, and nothing published since.
        unsafe { Self::read_slot(handle) }.shared()
    }

    #[track_caller]
    unsafe fn published_as_of_sent_changes<'a>(
        handle: *mut c_void,
        freshness: Freshness,
        read: ScriptForcedRead,
    ) -> &'a Self {
        // SAFETY: Guaranteed by the caller.
        let rows = unsafe { Self::published(handle) };
        // SAFETY: As above.
        let document = unsafe { super::ArenaHandle::document_of(handle) };
        let changes_to_reflect = match freshness {
            Freshness::Settled => crate::render_owner::sent_through(document),
            Freshness::Current | Freshness::Committed => crate::render_owner::sent_row_changes_through(document),
        };
        // The owner holds no state of an arena of no document (a unit test's), which is written in place: the thread
        // that holds it publishes it.
        let reflects_sent_changes = document.is_valid() && rows.changes_taken_in >= changes_to_reflect;
        let committed = matches!(freshness, Freshness::Committed);
        if reflects_sent_changes && !(committed && rows.paint_status.scrollable_overflow_unmeasured) {
            return rows;
        }
        // SAFETY: As above. The owner publishes rows that reflect the changes, and nothing borrows the ones read.
        unsafe {
            crate::render_owner::ask_about(
                handle,
                crate::render_owner::Query::CommittedRows {
                    measured_overflow: committed,
                },
                read,
            )
        };
        // SAFETY: As above.
        unsafe { Self::published(handle) }
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
        let rows = self.pseudo_element_rows.get(generator.element_index()? as usize)?;
        rows.get(usize::from(generated_for).checked_sub(1)?)
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

    /// The box whose content box the row in `id` is laid out against.
    pub(crate) fn containing_block(&self, id: NodeSlotId) -> Option<NodeSlotId> {
        super::tree_builder::RemovedBoxRows::containing_block(self, id)
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

impl super::tree_builder::RemovedBoxRows for RowSnapshot {
    fn node(&self, id: NodeSlotId) -> Option<PaintNode> {
        RowSnapshot::node(self, id).copied()
    }

    fn style(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>> {
        RowSnapshot::style(self, id)
    }

    fn bound_row(&self, style_node: StyleNodeID) -> Option<NodeSlotId> {
        RowSnapshot::bound_row(self, style_node)
    }

    fn viewport_row(&self) -> Option<NodeSlotId> {
        RowSnapshot::viewport_row(self)
    }
}

/// How up to date the rows the document thread reads have to be.
#[derive(Clone, Copy)]
enum Freshness {
    /// As of every change the thread sent that alters them.
    Current,
    /// As [`Self::Current`], and as committed: with the scrollable overflow a commit or a writer left measured.
    Committed,
    /// As of every change the thread sent.
    Settled,
}

/// Where the arena keeps the latest [`RowSnapshot`] it published. The arena's owner replaces it wherever it runs; the
/// document thread takes a reference of its own on the rows it reads beside a frame in flight ([`FrameRows`]), and
/// borrows them only where no frame in flight owns the arena ([`RowSnapshot::published`]).
#[derive(Default)]
pub(crate) struct RowSnapshotSlot(Mutex<Arc<RowSnapshot>>);

impl RowSnapshotSlot {
    fn rows(&self) -> std::sync::MutexGuard<'_, Arc<RowSnapshot>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Replaces the rows the slot holds with `rows`, on the thread that writes the slot: the arena's owner, or the
    /// document thread for the rows it adopted.
    pub(crate) fn publish(&self, rows: Arc<RowSnapshot>) {
        // The rows it held go once the slot is let go of: a reader may hold the last reference to them.
        let previous = std::mem::replace(&mut *self.rows(), rows);
        drop(previous);
    }

    /// The rows the slot holds, with a reference of the caller's own.
    pub(crate) fn shared(&self) -> Arc<RowSnapshot> {
        self.rows().clone()
    }

    /// # Safety
    ///
    /// Nothing may publish rows while the borrow is live.
    unsafe fn latest<'a>(&self) -> &'a RowSnapshot {
        // SAFETY: Guaranteed by the caller: the slot keeps the rows until the next publication.
        unsafe { &*Arc::as_ptr(&self.rows()) }
    }
}

/// The rows the arena's owner published last, with a reference of the document thread's own: a frame's own snapshot.
/// Reading them waits for nothing (no frame in flight, no owner that publishes rows reflecting what the document thread
/// sent since), so internal code reads the rows through this and only a script's forced read asks for current ones.
pub(crate) struct FrameRows(Arc<RowSnapshot>);

impl std::ops::Deref for FrameRows {
    type Target = RowSnapshot;

    fn deref(&self) -> &RowSnapshot {
        &self.0
    }
}

impl FrameRows {
    /// The rows the arena `handle` names published last: the arena's, or those the document thread adopted from display
    /// ticks, where those are later.
    ///
    /// # Safety
    ///
    /// `handle` must be a live handle on the document thread.
    pub(crate) unsafe fn of(handle: *mut c_void) -> Self {
        assert!(!handle.is_null(), "layout node arena handle is null");
        // SAFETY: Guaranteed by the caller. A handle is also a pointer to its arena, and the projection borrows nothing
        // of the arena beside the slot, which guards what it holds.
        let published = unsafe { &*std::ptr::addr_of!((*handle.cast::<LayoutNodeArena>()).published_rows) }.shared();
        // SAFETY: As above; only the document thread writes the rows it adopted.
        let adopted = unsafe { &HostTables::beside_frame(handle).adopted_rows }.shared();
        Self(if adopted.generation > published.generation {
            adopted
        } else {
            published
        })
    }

    /// The rows, for a holder that keeps them beyond the read.
    pub(crate) fn into_shared(self) -> Arc<RowSnapshot> {
        self.0
    }

    /// Whether the rows include every change the document thread sent that alters them, which a read that answers
    /// conservatively where they do not asks before it trusts what they lack.
    ///
    /// # Safety
    ///
    /// `handle` must be the live handle the rows were read from, on the document thread.
    pub(crate) unsafe fn include_sent_changes(&self, handle: *mut c_void) -> bool {
        // SAFETY: Guaranteed by the caller.
        let document = unsafe { super::ArenaHandle::document_of(handle) };
        // An arena of no document (a unit test's) is written in place.
        !document.is_valid() || self.changes_taken_in >= crate::render_owner::sent_row_changes_through(document)
    }
}

/// What the document thread wrote to rows that the rows it last read do not include yet, each with the change that
/// wrote it: the styles it applied to rows (a record, or none where the arena derives the row's style from now on), the
/// boxes it took out of the tree and freed, subtree and all, with the links that closed over them, and the boxes whose
/// committed box it cleared. Its reads answer them over the snapshot, as the rows will read once the owner has
/// published them.
#[derive(Default)]
pub(crate) struct RowsSentAhead {
    styles: HashMap<NodeSlotId, (ChangeSeq, Option<Arc<PublishedStyleRecord>>)>,
    detached: HashMap<NodeSlotId, ChangeSeq>,
    cleared: HashMap<NodeSlotId, ChangeSeq>,
    links: HashMap<NodeSlotId, LinksSentAhead>,
    /// The scroll offsets it wrote to rows.
    scroll_offsets: HashMap<NodeSlotId, (ChangeSeq, crate::css::css_pixels::CssPixelPoint)>,
    /// The change that wrote the latest of them.
    latest: ChangeSeq,
    /// The last change the rows it forgot what they include of had taken in: every change noted since is later.
    forgotten_through: ChangeSeq,
}

/// The links of a row that the boxes the document thread took out of the tree changed, each with the change that did.
#[derive(Default)]
struct LinksSentAhead {
    first_child: Option<(ChangeSeq, NodeSlotId)>,
    previous_sibling: Option<(ChangeSeq, NodeSlotId)>,
    next_sibling: Option<(ChangeSeq, NodeSlotId)>,
}

impl RowsSentAhead {
    /// Forgets what the rows `rows` include. Only rows that took in more than it last forgot walk what it holds, so the
    /// reads of a drain the owner publishes nothing during do not walk it once per row.
    fn forget_taken_in(&mut self, rows: &RowSnapshot) {
        let through = rows.changes_taken_in;
        if through == self.forgotten_through {
            return;
        }
        if self.latest <= through {
            *self = Self {
                forgotten_through: through,
                ..Self::default()
            };
            return;
        }
        self.forgotten_through = through;
        let ahead = |seq: &ChangeSeq| *seq > through;
        self.styles.retain(|_, (seq, _)| ahead(seq));
        self.detached.retain(|_, seq| ahead(seq));
        self.cleared.retain(|_, seq| ahead(seq));
        self.scroll_offsets.retain(|_, (seq, _)| ahead(seq));
        self.links.retain(|_, links| {
            for link in [
                &mut links.first_child,
                &mut links.previous_sibling,
                &mut links.next_sibling,
            ] {
                if link.is_some_and(|(seq, _)| !ahead(&seq)) {
                    *link = None;
                }
            }
            links.first_child.is_some() || links.previous_sibling.is_some() || links.next_sibling.is_some()
        });
    }

    /// Notes that the change `sent` gave the row `row` the style `style`. A change the owner took in as it was sent is
    /// in the rows already.
    fn note_style(&mut self, sent: Option<ChangeSeq>, row: NodeSlotId, style: Option<Arc<PublishedStyleRecord>>) {
        if let Some(sent) = sent {
            self.styles.insert(row, (sent, style));
            self.latest = sent;
        }
    }

    /// The scroll offset the document thread wrote to the box `row`, which the rows do not include yet.
    pub(crate) fn scroll_offset(&self, row: NodeSlotId) -> Option<crate::css::css_pixels::CssPixelPoint> {
        self.scroll_offsets.get(&row).map(|(_, offset)| *offset)
    }

    /// Notes that the change `sent` scrolled the box `row` to `offset`.
    fn note_scroll_offset(
        &mut self,
        sent: Option<ChangeSeq>,
        row: NodeSlotId,
        offset: crate::css::css_pixels::CssPixelPoint,
    ) {
        if let Some(sent) = sent {
            self.scroll_offsets.insert(row, (sent, offset));
            self.latest = sent;
        }
    }

    /// Notes that the change `sent` freed the box `row`, subtree and all, which hung from no parent.
    pub(crate) fn note_freed(&mut self, sent: Option<ChangeSeq>, row: NodeSlotId) {
        if let Some(sent) = sent {
            self.detached.insert(row, sent);
            self.latest = sent;
        }
    }

    /// Notes that the change `sent` cleared the committed box of `row`, which stays in the tree.
    pub(crate) fn note_cleared(&mut self, sent: Option<ChangeSeq>, row: NodeSlotId) {
        if let Some(sent) = sent {
            self.cleared.insert(row, sent);
            self.latest = sent;
        }
    }

    /// Notes that the change `sent` took the box `row`, which read as `removed`, out of its parent `parent`, and freed
    /// it, subtree and all.
    pub(crate) fn note_detached(
        &mut self,
        sent: Option<ChangeSeq>,
        row: NodeSlotId,
        parent: NodeSlotId,
        removed: &PaintNode,
    ) {
        let Some(sent) = sent else {
            return;
        };
        self.detached.insert(row, sent);
        let (previous, next) = (removed.previous_sibling, removed.next_sibling);
        if previous.is_invalid() {
            self.links.entry(parent).or_default().first_child = Some((sent, next));
        } else {
            self.links.entry(previous).or_default().next_sibling = Some((sent, next));
        }
        if !next.is_invalid() {
            self.links.entry(next).or_default().previous_sibling = Some((sent, previous));
        }
        self.latest = sent;
    }
}

/// The rows as the document thread last wrote them: the snapshot, with what it sent the owner since that the owner has
/// not taken in.
pub(crate) struct RowsAsSent<'a> {
    rows: &'a RowSnapshot,
    sent: &'a RowsSentAhead,
    /// Whether a read reached a row whose style the owner has not taken in: what the arena derives of the style (the
    /// row's flags, those of its children) is the owner's to know.
    read_style_sent_ahead: Cell<bool>,
}

impl<'a> RowsAsSent<'a> {
    pub(crate) fn new(rows: &'a RowSnapshot, sent: &'a RowsSentAhead) -> Self {
        Self {
            rows,
            sent,
            read_style_sent_ahead: Cell::new(false),
        }
    }

    /// Whether a read reached a row whose style the owner has not taken in, which leaves what the arena derives of it
    /// unknown here.
    pub(crate) fn read_style_sent_ahead(&self) -> bool {
        self.read_style_sent_ahead.get()
    }

    /// The style record of the row in `id`. What the arena derives for a row is its to make: until it has, the row
    /// reads as published.
    pub(crate) fn style_record(&self, id: NodeSlotId) -> Option<&'a PublishedStyleRecord> {
        match self.sent.styles.get(&id) {
            Some((_, Some(record))) => Some(record),
            _ => self.rows.style_record(id),
        }
    }

    /// Whether `slot` has a committed box: a change the thread sent frees the subtrees it detached and clears the
    /// committed boxes it named, and populates none.
    pub(crate) fn has_committed_box(&self, slot: NodeSlotId) -> bool {
        if !self.rows.paintable.paintable_row_is_populated(slot) || self.sent.cleared.contains_key(&slot) {
            return false;
        }
        if self.sent.detached.is_empty() {
            return true;
        }
        let mut row = slot;
        loop {
            if self.sent.detached.contains_key(&row) {
                return false;
            }
            match self.rows.node(row) {
                Some(node) if !node.parent.is_invalid() => row = node.parent,
                _ => return true,
            }
        }
    }

    /// Whether the row's style is one the arena derives for it.
    fn holds_derived_style(&self, id: NodeSlotId) -> bool {
        match self.sent.styles.get(&id) {
            Some((_, style)) => style.is_none(),
            None => self.rows.node(id).is_none_or(|row| row.holds_derived_style),
        }
    }
}

impl super::tree_builder::RemovedBoxRows for RowsAsSent<'_> {
    fn node(&self, id: NodeSlotId) -> Option<PaintNode> {
        if self.sent.detached.contains_key(&id) {
            return None;
        }
        if self.sent.styles.contains_key(&id) {
            self.read_style_sent_ahead.set(true);
        }
        let mut node = *self.rows.node(id)?;
        let Some(links) = self.sent.links.get(&id) else {
            return Some(node);
        };
        if let Some((_, first_child)) = links.first_child {
            node.first_child = first_child;
            // A box whose last child the owner takes out has no inline children left.
            if first_child.is_invalid() {
                node.flags &= !(super::node_data::NodeFlag::ChildrenAreInline as u32);
            }
        }
        node.previous_sibling = links.previous_sibling.map_or(node.previous_sibling, |(_, row)| row);
        node.next_sibling = links.next_sibling.map_or(node.next_sibling, |(_, row)| row);
        Some(node)
    }

    fn style(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>> {
        if self.sent.styles.contains_key(&id) {
            self.read_style_sent_ahead.set(true);
        }
        let record = self.style_record(id)?;
        Some(ComputedValuesView::new(&record.payloads.as_ffi().groups))
    }

    fn bound_row(&self, style_node: StyleNodeID) -> Option<NodeSlotId> {
        self.rows
            .bound_row(style_node)
            .filter(|row| !self.sent.detached.contains_key(row))
    }

    fn viewport_row(&self) -> Option<NodeSlotId> {
        self.rows.viewport_row()
    }
}

/// Notes that the change `sent` scrolled the box `row` of the arena `arena` names to `offset`, for the document thread
/// to read its own write until the rows include it.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
pub(crate) unsafe fn note_scroll_offset_sent_ahead(
    arena: *mut c_void,
    sent: Option<ChangeSeq>,
    row: NodeSlotId,
    offset: crate::css::css_pixels::CssPixelPoint,
) {
    // SAFETY: Guaranteed by the caller.
    unsafe { HostTables::beside_frame(arena) }
        .rows_sent_ahead
        .borrow_mut()
        .note_scroll_offset(sent, row, offset);
}

/// The rows the owner published last, with what the document thread wrote to them since, which the rows do not
/// include: its own writes, read without waiting for anything.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
pub(crate) unsafe fn frame_rows_and_sent_ahead<'a>(
    arena: *mut c_void,
) -> (FrameRows, std::cell::RefMut<'a, RowsSentAhead>) {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { FrameRows::of(arena) };
    // SAFETY: As above.
    let mut sent = unsafe { HostTables::beside_frame(arena) }.rows_sent_ahead.borrow_mut();
    sent.forget_taken_in(&rows);
    (rows, sent)
}

/// The rows the arena `arena` names published last, and what the document thread wrote to them that they do not
/// include yet.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread, and the rows must be let go of as [`RowSnapshot::published`]
/// requires.
#[track_caller]
pub(crate) unsafe fn rows_and_sent_ahead<'a>(
    arena: *mut c_void,
) -> (&'a RowSnapshot, std::cell::RefMut<'a, RowsSentAhead>) {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { RowSnapshot::published(arena) };
    // SAFETY: As above.
    let mut sent = unsafe { HostTables::beside_frame(arena) }.rows_sent_ahead.borrow_mut();
    sent.forget_taken_in(rows);
    (rows, sent)
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
    let (rows, sent) = unsafe { rows_and_sent_ahead(arena) };
    RowsAsSent::new(rows, &sent)
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
    let (rows, sent) = unsafe { rows_and_sent_ahead(arena) };
    RowsAsSent::new(rows, &sent)
        .style_record(id)
        .map_or(0, |record| record.dependency_flags)
}

/// Applies the style of the published record `style_record` names to the row `node`, taking a style that holds no
/// images: see [`LayoutNodeArena::install_row_style`]. Answers the image observers the row let go of, which the host
/// deletes.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread, `node` must name a live row with style, and `style_record`
/// must be a live handle of a record the style engine holds.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_install_row_style(
    arena: *mut c_void,
    node: NodeSlotId,
    style_record: *const c_void,
) -> *mut c_void {
    // SAFETY: Guaranteed by the caller.
    let record = unsafe { crate::css::style::published_record::shared_from_handle(style_record) };
    // SAFETY: As above.
    let old_image_observers =
        unsafe { HostTables::from_handle(arena) }.replace_image_observers(node, std::ptr::null_mut());
    // SAFETY: As above.
    let (_, mut sent_ahead) = unsafe { rows_and_sent_ahead(arena) };
    let change = LayoutChange::InstallRowStyle {
        node,
        style_record: record.style_record,
    };
    // SAFETY: As above.
    let sent = unsafe { super::layout_changes::send(arena, change) };
    sent_ahead.note_style(sent, node, Some(record));
    old_image_observers
}

/// Moves the row `node` to its DOM target's record `style_record` names, without applying the style: see
/// [`LayoutNodeArena::replace_row_style_record`]. Answers whether the row takes the record, which a row holding one
/// the arena derived for it does not.
///
/// # Safety
///
/// As for [`layout_arena_install_row_style`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_replace_row_style_record(
    arena: *mut c_void,
    node: NodeSlotId,
    style_record: *const c_void,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    let (rows, mut sent_ahead) = unsafe { rows_and_sent_ahead(arena) };
    if RowsAsSent::new(rows, &sent_ahead).holds_derived_style(node) {
        return false;
    }
    // SAFETY: As above.
    let record = unsafe { crate::css::style::published_record::shared_from_handle(style_record) };
    let change = LayoutChange::ReplaceRowStyleRecord {
        node,
        style_record: record.style_record,
    };
    // SAFETY: As above.
    let sent = unsafe { super::layout_changes::send(arena, change) };
    sent_ahead.note_style(sent, node, Some(record));
    true
}

/// Gives the row `node` the record `record` the host derived for it from its DOM target's style.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread, and the style engine must hold `record`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_adopt_derived_node_style(arena: *mut c_void, node: NodeSlotId, record: u64) {
    // SAFETY: Guaranteed by the caller.
    let (_, mut sent_ahead) = unsafe { rows_and_sent_ahead(arena) };
    // SAFETY: As above.
    let sent = unsafe { super::layout_changes::send(arena, LayoutChange::AdoptDerivedNodeStyle { node, record }) };
    sent_ahead.note_style(sent, node, None);
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_is_atomic_inline(arena: *mut c_void, id: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { RowSnapshot::published(arena) };
    rows.node(id)
        .is_some_and(|node| node_facts::node_is_atomic_inline(node, rows.style(id)))
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_is_fragmented_inline(arena: *mut c_void, id: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { RowSnapshot::published(arena) };
    rows.node(id)
        .is_some_and(|node| node_facts::node_is_fragmented_inline(node, rows.style(id)))
}

/// Whether the text row `id` renders a slice of its text node's data: the remainder beside a `::first-letter` box,
/// whose first letter slice is a row built for the same text node inside that box. The layout tree build decides the
/// slices, so a change to the data rebuilds them.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_text_has_source_range(arena: *mut c_void, id: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { RowSnapshot::published(arena) };
    rows.rows_built_for_same_node(id).iter().skip(1).any(|&row| {
        rows.parent(row)
            .and_then(|parent| rows.node(parent))
            .is_some_and(|parent| parent.generated_for == GENERATED_FOR_FIRST_LETTER)
    })
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
    fn the_document_thread_reads_its_own_scroll_offset_until_the_rows_include_it() {
        use crate::css::css_pixels::{CssPixelPoint, CssPixels};
        use crate::render_owner::ChangeSeq;

        let mut arena = LayoutNodeArena::new();
        let scroller = row(&mut arena, NodeKind::BlockContainer, None);
        let offset = CssPixelPoint::new(CssPixels::from_integer(0), CssPixels::from_integer(40));
        let mut sent = super::RowsSentAhead::default();
        sent.note_scroll_offset(Some(ChangeSeq::nth(2)), scroller, offset);

        let mut rows = super::RowSnapshot {
            changes_taken_in: ChangeSeq::nth(1),
            ..Default::default()
        };
        sent.forget_taken_in(&rows);
        assert_eq!(sent.scroll_offset(scroller), Some(offset));

        rows.changes_taken_in = ChangeSeq::nth(2);
        sent.forget_taken_in(&rows);
        assert_eq!(sent.scroll_offset(scroller), None);
        arena.free_subtree(scroller).invoke_callbacks();
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
