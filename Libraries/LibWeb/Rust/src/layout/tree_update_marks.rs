/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The layout tree update marks: what the DOM asks the layout tree build to rebuild. A mark is
//! written where the DOM changes and read by the next build, which retires it, keyed by the style
//! node identity the build walks by.
//!
//! The DOM writes and reads the marks at any time, a frame in flight included, and a mark must never
//! wait for a frame that does not read them. So the document thread owns them, beside the arena in
//! its host tables, and lends them to the arena for the one stage that reads them: the layout frame
//! lends them as its style round readies a tree build, and takes them back, with what the build
//! retired, the next time the document thread pays what the frame owes it. The stale box clear a
//! top layer member's detach runs outside a build borrows them the same way. A frame in flight that
//! holds them is joined by whatever reads them beside it, and its take-back hands them back. A
//! write beside it (a retirement, a fold, a child bit) waits for the take-back instead.

use super::LayoutNodeArena;
use super::host_tables::HostTables;
use crate::css::style::tree::StyleNodeID;
use std::ffi::c_void;

/// Runs `access` on the layout tree update marks of the document whose arena `handle` names: its
/// own, or the ones its tree build holds if the access is host work the build joined the document
/// thread for.
///
/// # Safety
///
/// `handle` must be a live handle from `layout_arena_create`, on the document thread.
pub(crate) unsafe fn with_document_marks<R>(
    handle: *mut c_void,
    access: impl FnOnce(&mut LayoutTreeUpdateMarks) -> R,
) -> R {
    // SAFETY: Guaranteed by the caller.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    // SAFETY: Guaranteed by the caller.
    unsafe { join_frame_in_flight_holding_marks(handle) };
    if host_tables.layout_tree_update_marks_are_lent.get() {
        // SAFETY: Guaranteed by the caller. The frame that holds the marks waits for this access,
        // or has run all of its stages.
        let arena = unsafe { LayoutNodeArena::from_handle(handle) };
        return access(&mut arena.layout_tree_update_marks_held_by_the_build());
    }
    access(&mut host_tables.layout_tree_update_marks.borrow_mut())
}

/// A write to the marks the document thread made beside the frame they are lent to.
pub(crate) enum MarkWriteWaitingForFrame {
    Clear(StyleNodeID),
    Merge {
        node: StyleNodeID,
        value: bool,
        reuse_reason: u8,
    },
    SetChildNeeds {
        node: StyleNodeID,
        value: bool,
    },
}

/// Whether a write to the marks of the document whose arena `handle` names waits for the frame in
/// flight: the frame holds them, and the calling thread is not running work it joined it for.
/// Such a write runs where joining the frame would have run it, once what the frame owed the
/// document thread is paid (see [`write_marks_waiting_for_frame`]). Anything that reads a mark
/// beside the frame joins it, which runs the waiting writes first.
fn mark_writes_wait_for_frame(host_tables: &HostTables, handle: *mut c_void) -> bool {
    host_tables.layout_tree_update_marks_are_lent.get()
        && crate::stage_thread::frame_in_flight_owns(handle)
        && !crate::stage_thread::running_join_work()
}

/// Retires the marks the node `style_node` names holds in the document whose arena `handle` names,
/// or has the retirement wait for the frame that holds them.
///
/// # Safety
///
/// As for [`with_document_marks`].
pub(crate) unsafe fn clear_document_marks(handle: *mut c_void, style_node: StyleNodeID) {
    // SAFETY: Guaranteed by the caller.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    if mark_writes_wait_for_frame(host_tables, handle) {
        host_tables
            .layout_tree_update_mark_writes_waiting_for_frame
            .borrow_mut()
            .push(MarkWriteWaitingForFrame::Clear(style_node));
        return;
    }
    // SAFETY: Guaranteed by the caller.
    unsafe { with_document_marks(handle, |marks| marks.clear(style_node)) }
}

/// Folds a mark into the one the node `style_node` names holds in the document whose arena `handle`
/// names, answering whether its own bit changed. Beside the frame that holds the marks the fold
/// waits for it, and the answer is that it changed: the build in flight retires the marks it
/// answers, so the fold is a transition once the frame hands them back, and what the mark site
/// widens from a transition it widens again at worst.
///
/// # Safety
///
/// As for [`with_document_marks`].
pub(crate) unsafe fn merge_document_mark(
    handle: *mut c_void,
    style_node: StyleNodeID,
    value: bool,
    reuse_reason: u8,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    if mark_writes_wait_for_frame(host_tables, handle) {
        host_tables
            .layout_tree_update_mark_writes_waiting_for_frame
            .borrow_mut()
            .push(MarkWriteWaitingForFrame::Merge {
                node: style_node,
                value,
                reuse_reason,
            });
        return true;
    }
    // SAFETY: Guaranteed by the caller.
    unsafe { with_document_marks(handle, |marks| marks.merge(style_node, value, reuse_reason)) }
}

/// Records whether a flat-tree descendant of the node `style_node` names holds a mark, answering
/// what was recorded before. Beside the frame that holds the marks the write waits for it, and the
/// answer is that nothing was, so an ancestor walk that stops where the bit was set goes on.
///
/// # Safety
///
/// As for [`with_document_marks`].
pub(crate) unsafe fn set_document_child_needs(handle: *mut c_void, style_node: StyleNodeID, value: bool) -> bool {
    // SAFETY: Guaranteed by the caller.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    if mark_writes_wait_for_frame(host_tables, handle) {
        host_tables
            .layout_tree_update_mark_writes_waiting_for_frame
            .borrow_mut()
            .push(MarkWriteWaitingForFrame::SetChildNeeds {
                node: style_node,
                value,
            });
        return false;
    }
    // SAFETY: Guaranteed by the caller.
    unsafe { with_document_marks(handle, |marks| marks.set_child_needs(style_node, value)) }
}

/// Whether the node `style_node` names or a flat-tree descendant of it may hold a mark. Beside the
/// frame that holds the marks the answer is that they may, rather than waiting for the frame to
/// hand them back.
///
/// # Safety
///
/// As for [`with_document_marks`].
pub(crate) unsafe fn subtree_may_hold_document_marks(handle: *mut c_void, style_node: StyleNodeID) -> bool {
    // SAFETY: Guaranteed by the caller.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    if mark_writes_wait_for_frame(host_tables, handle) {
        return true;
    }
    // SAFETY: Guaranteed by the caller.
    unsafe { with_document_marks(handle, |marks| marks.needs(style_node) || marks.child_needs(style_node)) }
}

/// Makes the writes to the marks that waited for the frame that held them, in order.
///
/// # Safety
///
/// `handle` must be a live handle from `layout_arena_create`, on the document thread, with the
/// marks handed back.
pub(crate) unsafe fn write_marks_waiting_for_frame(handle: *mut c_void) {
    // SAFETY: Guaranteed by the caller.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    let writes = host_tables.layout_tree_update_mark_writes_waiting_for_frame.take();
    if writes.is_empty() {
        return;
    }
    debug_assert!(!host_tables.layout_tree_update_marks_are_lent.get());
    let mut marks = host_tables.layout_tree_update_marks.borrow_mut();
    for write in writes {
        match write {
            MarkWriteWaitingForFrame::Clear(node) => marks.clear(node),
            MarkWriteWaitingForFrame::Merge {
                node,
                value,
                reuse_reason,
            } => {
                marks.merge(node, value, reuse_reason);
            }
            MarkWriteWaitingForFrame::SetChildNeeds { node, value } => {
                marks.set_child_needs(node, value);
            }
        }
    }
}

/// Joins the frame in flight if it holds the document's marks, so that its take-back hands them
/// back before they are reached.
///
/// # Safety
///
/// As for [`with_document_marks`].
unsafe fn join_frame_in_flight_holding_marks(handle: *mut c_void) {
    // SAFETY: Guaranteed by the caller.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    if host_tables.layout_tree_update_marks_are_lent.get() && crate::stage_thread::frame_in_flight_owns(handle) {
        crate::stage_thread::join_frame_in_flight(handle);
    }
}

/// Lends the document's layout tree update marks to its arena for the tree build a layout frame
/// has readied, which reads and retires them there.
///
/// # Safety
///
/// `handle` must be a live handle from `layout_arena_create`, on the document thread, with no
/// frame in flight and no stage running for it.
pub(crate) unsafe fn lend_to_frame(handle: *mut c_void) {
    // SAFETY: Guaranteed by the caller. The host tables sit beside the arena, not in it.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    assert!(
        !host_tables.layout_tree_update_marks_are_lent.get(),
        "a tree build walks alone"
    );
    // SAFETY: Guaranteed by the caller: nothing else reaches the arena.
    let arena = unsafe { &*handle.cast::<LayoutNodeArena>() };
    *arena.layout_tree_update_marks_held_by_the_build() = host_tables.layout_tree_update_marks.take();
    host_tables.layout_tree_update_marks_are_lent.set(true);
}

/// Takes back the marks [`lend_to_frame`] lent, with what the build retired, if they are lent.
///
/// # Safety
///
/// As for [`lend_to_frame`].
pub(crate) unsafe fn take_back_from_frame(handle: *mut c_void) {
    // SAFETY: Guaranteed by the caller. The host tables sit beside the arena, not in it.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    if !host_tables.layout_tree_update_marks_are_lent.get() {
        return;
    }
    // SAFETY: Guaranteed by the caller: nothing else reaches the arena.
    let arena = unsafe { &*handle.cast::<LayoutNodeArena>() };
    host_tables
        .layout_tree_update_marks
        .replace(std::mem::take(&mut *arena.layout_tree_update_marks_held_by_the_build()));
    host_tables.layout_tree_update_marks_are_lent.set(false);
}

/// Lends the document's layout tree update marks to the arena for `clear`, the stale box clear a
/// top layer member's detach runs at DOM mutation time, outside any tree build: it retires the marks
/// of the nodes whose boxes it clears, which the member's next mark must find retired to be the
/// transition that queues it for the build.
///
/// # Safety
///
/// `handle` must be a live handle from `layout_arena_create`, on the document thread.
pub(crate) unsafe fn lend_to_stale_box_clear<R>(handle: *mut c_void, clear: impl FnOnce() -> R) -> R {
    // SAFETY: Guaranteed by the caller. The host tables sit beside the arena, not in it.
    let host_tables = unsafe { HostTables::beside_frame(handle) };
    // SAFETY: Guaranteed by the caller.
    unsafe { join_frame_in_flight_holding_marks(handle) };
    // Host work a layout frame joined the document thread for finds the marks lent already.
    if host_tables.layout_tree_update_marks_are_lent.get() {
        return clear();
    }
    // SAFETY: Guaranteed by the caller. No frame is in flight once the arena is borrowed.
    let arena = unsafe { LayoutNodeArena::from_handle(handle) };
    *arena.layout_tree_update_marks_held_by_the_build() = host_tables.layout_tree_update_marks.take();
    host_tables.layout_tree_update_marks_are_lent.set(true);
    let result = clear();
    host_tables
        .layout_tree_update_marks
        .replace(std::mem::take(&mut *arena.layout_tree_update_marks_held_by_the_build()));
    host_tables.layout_tree_update_marks_are_lent.set(false);
    result
}

/// Which narrower rebuild the marks a node has collected so far still permit, as
/// `Node::LayoutTreeUpdateReuseReason` spells them. Nothing set means only a full rebuild will do.
pub(crate) mod layout_tree_update_reuse_reason {
    pub(crate) const CHILD_LIST_INSERTION: u8 = 1;
    pub(crate) const PSEUDO_ELEMENT_CHANGE: u8 = 2;
    pub(super) const ALL: u8 = CHILD_LIST_INSERTION | PSEUDO_ELEMENT_CHANGE;
}

/// The build has to rebuild what the node produces.
const NEEDS: u8 = 1 << 2;
/// A flat-tree descendant holds a mark: the chain the build climbs down to reach a node it has to
/// rebuild. Only an element, a shadow root and the document are ever on it.
const CHILD_NEEDS: u8 = 1 << 3;

/// One byte of marks per identity, in the element and the text index spaces apart. The low bits
/// are the reuse reasons, so no reason ever needs translating.
#[derive(Default)]
pub(crate) struct LayoutTreeUpdateMarks {
    elements: Vec<u8>,
    text: Vec<u8>,
}

impl LayoutTreeUpdateMarks {
    fn get(&self, node: StyleNodeID) -> u8 {
        let (column, index) = match node.element_index() {
            Some(index) => (&self.elements, index),
            None => (
                &self.text,
                node.text_index().expect("a style node is an element or a text node"),
            ),
        };
        column.get(index as usize).copied().unwrap_or(0)
    }

    fn update(&mut self, node: StyleNodeID, update: impl FnOnce(u8) -> u8) {
        let (column, index) = match node.element_index() {
            Some(index) => (&mut self.elements, index),
            None => (
                &mut self.text,
                node.text_index().expect("a style node is an element or a text node"),
            ),
        };
        let index = index as usize;
        let current = column.get(index).copied().unwrap_or(0);
        let updated = update(current);
        if updated == current {
            return;
        }
        if index >= column.len() {
            column.resize(index + 1, 0);
        }
        column[index] = updated;
    }

    /// Whether the layout tree build has to rebuild what this node produces.
    pub(crate) fn needs(&self, node: StyleNodeID) -> bool {
        self.get(node) & NEEDS != 0
    }

    /// Which narrower rebuilds the marks collected on this node still permit. See
    /// [`layout_tree_update_reuse_reason`].
    pub(crate) fn reuse_reasons(&self, node: StyleNodeID) -> u8 {
        self.get(node) & layout_tree_update_reuse_reason::ALL
    }

    /// Whether a flat-tree descendant holds a mark. A text node is never on the chain the mark
    /// climbs, so it answers no.
    pub(crate) fn child_needs(&self, node: StyleNodeID) -> bool {
        node.element_index().is_some() && self.get(node) & CHILD_NEEDS != 0
    }

    /// Fold one mark into the node's, answering whether its own bit changed. That answer is what
    /// tells the mark site it has a transition to widen from. Once a reason that forbids reuse
    /// arrives, a later one cannot narrow it back.
    pub(crate) fn merge(&mut self, node: StyleNodeID, value: bool, reuse_reason: u8) -> bool {
        let reuse_reason = reuse_reason & layout_tree_update_reuse_reason::ALL;
        let mut changed = false;
        self.update(node, |marks| {
            let reasons = marks & layout_tree_update_reuse_reason::ALL;
            let rest = marks & !(NEEDS | layout_tree_update_reuse_reason::ALL);
            if (marks & NEEDS != 0) == value {
                let merged = if reuse_reason == 0 || reasons == 0 {
                    0
                } else {
                    reasons | reuse_reason
                };
                return rest | (marks & NEEDS) | merged;
            }
            changed = true;
            rest | if value { NEEDS } else { 0 } | reuse_reason
        });
        changed
    }

    /// Record whether a flat-tree descendant holds a mark, answering what was recorded before. The
    /// mark's ancestor walk stops where the answer is already yes.
    pub(crate) fn set_child_needs(&mut self, node: StyleNodeID, value: bool) -> bool {
        if node.element_index().is_none() {
            return false;
        }
        let before = self.child_needs(node);
        self.update(node, |marks| {
            if value {
                marks | CHILD_NEEDS
            } else {
                marks & !CHILD_NEEDS
            }
        });
        before
    }

    /// Retire the marks the node holds, own and child alike: the build has just answered them, or
    /// the identity is retired and may name another node next.
    pub(crate) fn clear(&mut self, node: StyleNodeID) {
        self.update(node, |_| 0);
    }
}

#[cfg(test)]
mod tests {
    use super::layout_tree_update_reuse_reason::{CHILD_LIST_INSERTION, PSEUDO_ELEMENT_CHANGE};
    use super::*;

    #[test]
    fn a_reason_that_forbids_reuse_cannot_be_narrowed_again() {
        let mut marks = LayoutTreeUpdateMarks::default();
        let element = StyleNodeID::element(5);
        assert!(marks.merge(element, true, CHILD_LIST_INSERTION));
        assert!(!marks.merge(element, true, PSEUDO_ELEMENT_CHANGE));
        assert_eq!(
            marks.reuse_reasons(element),
            CHILD_LIST_INSERTION | PSEUDO_ELEMENT_CHANGE
        );
        assert!(!marks.merge(element, true, 0));
        assert_eq!(marks.reuse_reasons(element), 0);
        assert!(!marks.merge(element, true, CHILD_LIST_INSERTION));
        assert_eq!(marks.reuse_reasons(element), 0);
        assert!(marks.needs(element));
    }

    #[test]
    fn the_child_mark_answers_what_it_was_and_only_elements_hold_one() {
        let mut marks = LayoutTreeUpdateMarks::default();
        let element = StyleNodeID::element(3);
        let text = StyleNodeID::text(3);
        assert!(!marks.set_child_needs(element, true));
        assert!(marks.set_child_needs(element, true));
        assert!(marks.child_needs(element));
        assert!(!marks.set_child_needs(text, true));
        assert!(!marks.child_needs(text));
        assert!(marks.merge(text, true, 0));
        assert!(marks.needs(text) && !marks.needs(element));
        marks.clear(element);
        assert!(!marks.child_needs(element));
        assert!(marks.needs(text));
    }
}
