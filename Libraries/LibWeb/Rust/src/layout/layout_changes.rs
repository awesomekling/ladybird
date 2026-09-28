/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the main thread writes of a document's layout, and what it asks of it, as the typed changes and questions the
//! render owner applies and answers with the document's arena. The main thread names a node by its slot and sends; it
//! never reaches the arena itself.

use super::LayoutNodeArena;
use super::layout_node_arena::LayoutUpdateMarksHandle;
use super::node_data::NodeSlotId;
use super::partial_relayout::FfiLayoutTreeUpdateClassification;
use super::tree_builder::FfiRemovedBoxPlace;
use crate::render_owner::{Answer, ArenaChange, DocumentId, Query};
use std::ffi::c_void;

/// One write of the main thread to a document's layout marks or layout facts, which the owner applies to the arena
/// before the next unit that reads it.
pub(crate) enum LayoutChange {
    SetNeedsLayoutUpdate {
        node: NodeSlotId,
        propagate_through_ancestors: bool,
    },
    ResetCachedIntrinsicSizesOfSelfAndAncestors {
        node: NodeSlotId,
    },
    DeferChildListInsertionLayoutUpdate {
        parent: NodeSlotId,
    },
    InvalidateTextContent {
        node: NodeSlotId,
    },
    /// The text under `root` renders with the language it now resolves: where any does, the root lays out again.
    EnrollTextAfterLanguageChange {
        root: NodeSlotId,
    },
    RecordPartialRelayoutEscape,
}

impl LayoutChange {
    /// Applies the change to `arena`, the arena of the document it was sent for. A node freed since it was sent has
    /// nothing left to mark.
    pub(crate) fn apply(self, arena: &mut LayoutNodeArena) {
        match self {
            Self::SetNeedsLayoutUpdate {
                node,
                propagate_through_ancestors,
            } => {
                if arena.slot_is_live(node) {
                    arena.set_needs_layout_update(node, propagate_through_ancestors);
                }
            }
            Self::ResetCachedIntrinsicSizesOfSelfAndAncestors { node } => {
                if arena.slot_is_live(node) {
                    arena.reset_cached_intrinsic_sizes_of_self_and_ancestors(node);
                }
            }
            Self::DeferChildListInsertionLayoutUpdate { parent } => {
                if arena.slot_is_live(parent) {
                    arena.defer_child_list_insertion_layout_update(parent);
                }
            }
            Self::InvalidateTextContent { node } => {
                if arena.slot_is_live(node) {
                    arena.invalidate_text_content(node);
                }
            }
            Self::EnrollTextAfterLanguageChange { root } => {
                if arena.slot_is_live(root) && super::rendered_text::enroll_text_after_language_change(arena, root) {
                    arena.set_needs_layout_update(root, true);
                }
            }
            Self::RecordPartialRelayoutEscape => arena.record_partial_relayout_escape(),
        }
    }

    /// Whether the change can mark a node for layout, which is all a change can do to whether the layout is up to date.
    fn lays_out_again(&self) -> bool {
        matches!(
            self,
            Self::SetNeedsLayoutUpdate { .. } | Self::EnrollTextAfterLanguageChange { .. }
        )
    }
}

/// Sends `change` to the owner of the document whose arena `arena` names. A host call of a unit the owner runs is the
/// owner's already: it applies the change to the arena the unit holds, in place.
///
/// # Safety
///
/// `arena` must be a live arena handle, on the document thread or inside a unit the owner runs for it.
pub(super) unsafe fn send(arena: *mut c_void, change: LayoutChange) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // A test's hold on the owner has the document thread read the arena in place, which the change goes to as well.
    if crate::stage_thread::running_inside_stage() || crate::stage_thread::owner_work_runs_here() {
        // SAFETY: The unit the owner runs holds the arena, and its document thread waits for it; or the document
        // thread does the owner's work.
        change.apply(unsafe { &mut *super::ArenaHandle::held_by_waiting_thread(arena) }.arena_mut());
        return;
    }
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { super::ArenaHandle::document_of(arena) };
    let lays_out_again = change.lays_out_again();
    let seq = crate::render_owner::send_arena_change(document, ArenaChange::Layout(change));
    if lays_out_again {
        // SAFETY: As above.
        unsafe { super::HostTables::beside_frame(arena) }
            .last_relayout_change_sent
            .set(Some(seq));
    }
}

/// Whether the document thread sent the owner a change that lays a node out again, which nothing it sent since reaches
/// the arena after: the arena does not have it yet, so the layout is not up to date, whatever the arena says.
///
/// # Safety
///
/// `arena` must be a live arena handle on the document thread.
pub(crate) unsafe fn changes_sent_not_taken_in(arena: *mut c_void) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    if crate::stage_thread::running_inside_stage() {
        return false;
    }
    // SAFETY: Guaranteed by the caller.
    let Some(seq) = unsafe { super::HostTables::beside_frame(arena) }
        .last_relayout_change_sent
        .get()
    else {
        return false;
    };
    // SAFETY: As above.
    let document = unsafe { super::ArenaHandle::document_of(arena) };
    !crate::render_owner::taken_in_before_next_arena_reach(document, seq)
}

/// Sends `change` through the layout update marks the document's render inputs hand out.
///
/// # Safety
///
/// As for [`send`].
pub(super) unsafe fn send_through_marks(marks: LayoutUpdateMarksHandle, change: LayoutChange) {
    // SAFETY: Guaranteed by the caller.
    unsafe { send(marks.arena, change) };
}

/// A question the main thread asks about a document's layout tree.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LayoutRead {
    ClassifyLayoutTreeUpdate {
        node: NodeSlotId,
        reason_is_structural_boundary_self_rebuild: bool,
    },
    RemovedBoxDetachableInPlace(FfiRemovedBoxPlace),
    TextHasSourceRange(NodeSlotId),
    NodeIsPartialRelayoutBoundary(NodeSlotId),
    NodeIsAtomicInline(NodeSlotId),
    NodeIsFragmentedInline(NodeSlotId),
}

/// The answer to a [`LayoutRead`], of the variant it asked for.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LayoutReadAnswer {
    Classification(FfiLayoutTreeUpdateClassification),
    Bool(bool),
}

impl LayoutRead {
    /// Answers the question from `arena`, as the units before it left it.
    pub(crate) fn answer(self, arena: &mut LayoutNodeArena) -> LayoutReadAnswer {
        match self {
            Self::ClassifyLayoutTreeUpdate {
                node,
                reason_is_structural_boundary_self_rebuild,
            } => LayoutReadAnswer::Classification(if arena.slot_is_live(node) {
                arena.classify_layout_tree_update(node, reason_is_structural_boundary_self_rebuild)
            } else {
                FfiLayoutTreeUpdateClassification::default()
            }),
            Self::RemovedBoxDetachableInPlace(place) => {
                LayoutReadAnswer::Bool(super::tree_builder::removed_box_detachable_in_place(arena, &place).is_some())
            }
            Self::TextHasSourceRange(node) => {
                LayoutReadAnswer::Bool(arena.slot_is_live(node) && arena.text_has_source_range(node))
            }
            Self::NodeIsPartialRelayoutBoundary(node) => {
                LayoutReadAnswer::Bool(arena.slot_is_live(node) && arena.node_is_partial_relayout_boundary(node))
            }
            Self::NodeIsAtomicInline(node) => LayoutReadAnswer::Bool(
                arena.slot_is_live(node) && {
                    let data = arena.data(node);
                    super::node_facts::node_is_atomic_inline(data, super::node_facts::node_style_view(data))
                },
            ),
            Self::NodeIsFragmentedInline(node) => LayoutReadAnswer::Bool(
                arena.slot_is_live(node) && {
                    let data = arena.data(node);
                    super::node_facts::node_is_fragmented_inline(data, super::node_facts::node_style_view(data))
                },
            ),
        }
    }

    /// The answer where the owner answered nothing.
    pub(crate) fn unanswered(self) -> LayoutReadAnswer {
        match self {
            Self::ClassifyLayoutTreeUpdate { .. } => {
                LayoutReadAnswer::Classification(FfiLayoutTreeUpdateClassification::default())
            }
            Self::RemovedBoxDetachableInPlace(_)
            | Self::TextHasSourceRange(_)
            | Self::NodeIsPartialRelayoutBoundary(_)
            | Self::NodeIsAtomicInline(_)
            | Self::NodeIsFragmentedInline(_) => LayoutReadAnswer::Bool(false),
        }
    }
}

/// Asks the owner of the document whose arena `arena` names `read`, and waits for the answer.
///
/// # Safety
///
/// `arena` must be a live arena handle on the document thread.
pub(super) unsafe fn ask(arena: *mut c_void, read: LayoutRead) -> LayoutReadAnswer {
    assert!(!arena.is_null(), "layout node arena handle is null");
    crate::stage_thread::join_frame_in_flight(arena);
    // SAFETY: Guaranteed by the caller.
    let document: DocumentId = unsafe { super::ArenaHandle::document_of(arena) };
    // SAFETY: As above.
    match unsafe { crate::render_owner::ask(document, arena, Query::Layout(read)) } {
        Answer::Layout(answer) => answer,
        _ => {
            debug_assert!(false, "a layout read is answered with a layout answer");
            read.unanswered()
        }
    }
}

/// # Safety
///
/// `arena` must be a live arena handle on the document thread.
pub(super) unsafe fn ask_bool(arena: *mut c_void, read: LayoutRead) -> bool {
    // SAFETY: Guaranteed by the caller.
    match unsafe { ask(arena, read) } {
        LayoutReadAnswer::Bool(value) => value,
        LayoutReadAnswer::Classification(_) => {
            debug_assert!(false, "a yes-or-no layout read is answered yes or no");
            false
        }
    }
}
