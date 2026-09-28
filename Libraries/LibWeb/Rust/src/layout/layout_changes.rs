/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the main thread writes of a document's layout, and what it asks of it, as the typed changes and questions the
//! render owner applies and answers with the document's arena. The main thread names a node by its slot and sends; it
//! never reaches the arena itself.

use super::LayoutNodeArena;
use super::layout_node_arena::{HostPayment, LayoutUpdateMarksHandle};
use super::node_data::{CompositorAnimationFrameKind, NodeFlag, NodeSlotId};
use super::partial_relayout::FfiLayoutTreeUpdateClassification;
use super::tree_builder::FfiRemovedBoxPlace;
use super::used_values::FfiCssPixelPoint;
use crate::css::style::tree::{NaturalSize, StyleNodeID};
use crate::render_owner::{Answer, ArenaChange, ChangeSeq, DocumentId, Query};
use std::ffi::c_void;

/// One write of the main thread to a document's layout marks or layout facts, which the owner applies to the arena
/// before the next unit that reads it.
pub(crate) enum LayoutChange {
    SetNeedsLayoutUpdate {
        node: NodeSlotId,
        propagate_through_ancestors: bool,
    },
    SetNeedsOwnGeometryUpdate {
        node: NodeSlotId,
    },
    SetNeedsFullLayoutTreeUpdate(bool),
    /// What the node's content is sized from changed: its fragment caches and intrinsic sizes, and those of its
    /// ancestors, are stale.
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
    /// The owned provider of the image box `node` shows an image of this natural size now.
    SetOwnedImageNaturalSize {
        node: NodeSlotId,
        natural_size: NaturalSize,
    },
    /// A fact of the node's DOM node the host stamps into its row.
    SetNodeFlag {
        node: NodeSlotId,
        flag: NodeFlag,
        value: bool,
    },
    /// What the rows built for the DOM node of `node` are painted and hit-tested with.
    SetNodeDomPaintFacts {
        node: NodeSlotId,
        facts: u8,
    },
    /// Whether the compositor animates the box on its own frames of `kind`, as the rendering update chose.
    SetNodeNeedsCompositorAnimationFrame {
        node: NodeSlotId,
        kind: CompositorAnimationFrameKind,
        value: bool,
    },
    /// What the element has scrolled to.
    SetElementScrollOffset {
        element: StyleNodeID,
        offset: FfiCssPixelPoint,
    },
    /// What the pseudo-element `pseudo_kind` of `generator` has scrolled to.
    SetPseudoElementScrollOffset {
        generator: StyleNodeID,
        pseudo_kind: u8,
        offset: FfiCssPixelPoint,
    },
    /// The DOM node with `old` took `new`, or none: see [`LayoutNodeArena::change_style_node`].
    StyleNodeChanged {
        old: StyleNodeID,
        new: Option<StyleNodeID>,
    },
    /// Whether the node sits in the user agent shadow tree of the focused text control.
    SetIdentityInFocusedTextControl {
        node: StyleNodeID,
        value: bool,
    },
    /// Whether the list owner's items were renumbered without its layout tree being rebuilt.
    SetListOwnerHasStaleItemCounters {
        list_owner: StyleNodeID,
        value: bool,
    },
    /// The host gave the row the image observer set that `set` names, or none (`set` is 0).
    SetRowImageObservers {
        node: NodeSlotId,
        set: usize,
    },
    /// The host gave the image box the provider it owns.
    RowOwnsImageProvider {
        node: NodeSlotId,
    },
    /// Whether attaching the row's style resources loaded any image.
    SetStyleImageResourcesAttached {
        node: NodeSlotId,
        attached: bool,
    },
    /// The node left the document, and the box it is bound to, or that of its pseudo-element of kind `generated_for`,
    /// keeps its style readable until it is freed.
    PinBoundBoxStyleRecordForDetachment {
        style_node: StyleNodeID,
        generated_for: u8,
    },
    /// The host applied the style of `style_record` to the row: see [`LayoutNodeArena::install_row_style`].
    InstallRowStyle {
        node: NodeSlotId,
        style_record: u64,
    },
    /// The row's DOM target took `style_record`: see [`LayoutNodeArena::replace_row_style_record`].
    ReplaceRowStyleRecord {
        node: NodeSlotId,
        style_record: u64,
    },
    /// The host derived `record` for the row from its DOM target's style.
    AdoptDerivedNodeStyle {
        node: NodeSlotId,
        record: u64,
    },
    /// An animation sample published this record for the element, which adopted it: the element's row takes it over.
    InstallAnimationSample {
        style_node: StyleNodeID,
        style_record: u64,
        needs_relayout: bool,
    },
    /// The table spans the element the row was built for asks for.
    SetTableSpans {
        node: NodeSlotId,
        column_span: u16,
        row_span: u16,
        raw_column_span: u32,
    },
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
            Self::SetNeedsOwnGeometryUpdate { node } => {
                if arena.slot_is_live(node) {
                    arena.set_node_flag(node, NodeFlag::NeedsOwnGeometryUpdate, true);
                }
            }
            Self::SetNeedsFullLayoutTreeUpdate(value) => arena.set_needs_full_layout_tree_update(value),
            Self::ResetCachedIntrinsicSizesOfSelfAndAncestors { node } => {
                if arena.slot_is_live(node) {
                    arena.bump_fragment_cache_epoch_of_self_and_ancestors(node);
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
            Self::SetOwnedImageNaturalSize { node, natural_size } => {
                if arena.slot_is_live(node) {
                    arena.set_owned_image_natural_size(node, natural_size);
                }
            }
            Self::SetNodeFlag { node, flag, value } => {
                if arena.slot_is_live(node) {
                    arena.set_node_flag(node, flag, value);
                }
            }
            Self::SetNodeDomPaintFacts { node, facts } => {
                if arena.slot_is_live(node) {
                    arena.set_node_dom_paint_facts(node, facts);
                }
            }
            Self::SetNodeNeedsCompositorAnimationFrame { node, kind, value } => {
                if arena.slot_is_live(node) {
                    arena.set_node_needs_compositor_animation_frame(node, kind, value);
                }
            }
            Self::SetElementScrollOffset { element, offset } => arena.set_element_scroll_offset(element, offset),
            Self::SetPseudoElementScrollOffset {
                generator,
                pseudo_kind,
                offset,
            } => arena.set_pseudo_element_scroll_offset(generator, pseudo_kind, offset),
            Self::StyleNodeChanged { old, new } => arena.change_style_node(old, new),
            Self::InstallRowStyle { node, style_record } => {
                if arena.slot_is_live(node) {
                    arena.install_row_style(node, style_record);
                }
            }
            Self::ReplaceRowStyleRecord { node, style_record } => {
                if arena.slot_is_live(node) {
                    arena.replace_row_style_record(node, style_record);
                }
            }
            Self::AdoptDerivedNodeStyle { node, record } => {
                if arena.slot_is_live(node) {
                    arena.adopt_derived_node_style(node, record);
                }
            }
            Self::SetIdentityInFocusedTextControl { node, value } => {
                arena.set_identity_in_focused_text_control(node, value);
            }
            Self::SetListOwnerHasStaleItemCounters { list_owner, value } => {
                arena.set_list_owner_has_stale_item_counters(list_owner, value);
            }
            Self::SetRowImageObservers { node, set } => {
                if arena.slot_is_live(node) {
                    arena.set_row_image_observers(node, set);
                }
            }
            Self::RowOwnsImageProvider { node } => {
                if arena.slot_is_live(node) {
                    arena.note_row_owns_image_provider(node);
                }
            }
            Self::SetStyleImageResourcesAttached { node, attached } => {
                if arena.slot_is_live(node) {
                    arena.note_style_image_resources_attached(node, attached);
                }
            }
            Self::PinBoundBoxStyleRecordForDetachment {
                style_node,
                generated_for,
            } => arena.pin_bound_box_style_record_for_detachment(style_node, generated_for),
            Self::InstallAnimationSample {
                style_node,
                style_record,
                needs_relayout,
            } => {
                let row = arena.bound_row(style_node);
                if arena.install_animation_sample(style_node, style_record, needs_relayout) {
                    // The element adopted the record as it published it, so the adoption leaves the log at once.
                    let adopted = arena.take_animation_adoption(row, style_record);
                    debug_assert!(adopted, "an installed sample is adopted");
                } else if needs_relayout && !row.is_invalid() {
                    // The host installs the record over the row itself, and the row lays out again as the host's
                    // install would have marked it.
                    arena.set_needs_layout_update(row, true);
                }
            }
            Self::SetTableSpans {
                node,
                column_span,
                row_span,
                raw_column_span,
            } => {
                if arena.slot_is_live(node) {
                    arena.set_table_spans(node, column_span, row_span, raw_column_span);
                }
            }
        }
    }

    /// Whether the change can mark a node for layout, which is all a change can do to whether the layout is up to date.
    fn lays_out_again(&self) -> bool {
        matches!(
            self,
            Self::SetNeedsLayoutUpdate { .. }
                | Self::EnrollTextAfterLanguageChange { .. }
                | Self::SetNeedsFullLayoutTreeUpdate(true)
                | Self::SetTableSpans { .. }
                | Self::InstallAnimationSample {
                    needs_relayout: true,
                    ..
                }
        )
    }
}

/// Sends `change` to the owner of the document whose arena `arena` names, and answers its number. A host call of a
/// unit the owner runs is the owner's already: it applies the change to the arena the unit holds, in place, and
/// answers none.
///
/// # Safety
///
/// `arena` must be a live arena handle, on the document thread or inside a unit the owner runs for it.
pub(super) unsafe fn send(arena: *mut c_void, change: LayoutChange) -> Option<ChangeSeq> {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // A test's hold on the owner has the document thread read the arena in place, which the change goes to as well.
    if crate::stage_thread::running_inside_stage() || crate::stage_thread::owner_work_runs_here() {
        // SAFETY: The unit the owner runs holds the arena, and its document thread waits for it; or the document
        // thread does the owner's work.
        let arena = unsafe { &mut *super::ArenaHandle::held_by_waiting_thread(arena) }.arena_mut();
        change.apply(arena);
        // The unit publishes the rows as it ends; the document thread reads them next.
        if !crate::stage_thread::running_inside_stage() {
            arena.publish_rows();
        }
        return None;
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
    Some(seq)
}

/// Whether the document thread sent the owner a change that lays a node out again, which nothing it sent since reaches
/// the arena after: the arena does not have it yet, so the layout is not up to date, whatever the arena says.
///
/// # Safety
///
/// `arena` must be a live arena handle on the document thread.
pub(crate) unsafe fn changes_sent_not_taken_in(arena: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    let sent = unsafe { super::HostTables::beside_frame(arena) }
        .last_relayout_change_sent
        .get();
    // SAFETY: Guaranteed by the caller.
    sent.is_some_and(|seq| unsafe { not_taken_in(arena, seq) })
}

/// Whether the change `seq` the document thread sent the owner of the arena `arena` names is one nothing it sent since
/// reaches the arena after: the arena does not have it yet.
///
/// # Safety
///
/// `arena` must be a live arena handle on the document thread.
pub(super) unsafe fn not_taken_in(arena: *mut c_void, seq: ChangeSeq) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // Without a Rendering thread, a change goes to the arena as it is sent.
    if crate::stage_thread::running_inside_stage() || !crate::stage_thread::has_owner_thread() {
        return false;
    }
    // SAFETY: Guaranteed by the caller.
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

/// A write the main thread waits for the owner to make, as it pays what the write owes the host at once.
#[derive(Clone, Copy, Debug)]
pub(crate) enum LayoutWrite {
    /// Takes the layout subtree `root` heads out of the tree, and frees it: every row in it is prepared for leaving the
    /// tree, and the subtree is detached from its parent, if it has one.
    DropSubtree(NodeSlotId),
}

impl LayoutWrite {
    /// Makes the write, and answers what it owes the host.
    pub(crate) fn apply(self, arena: &mut LayoutNodeArena) -> HostPayment {
        match self {
            Self::DropSubtree(root) => {
                if !arena.slot_is_live(root) {
                    return HostPayment::nothing();
                }
                arena.release_published_paintable_rows();
                arena.owed_for(|arena| {
                    super::layout_node_arena::prepare_subtree_for_detach(arena, root);
                    super::layout_node_arena::detach_and_free_subtree(arena, root);
                })
            }
        }
    }
}

/// Has the owner of the document whose arena `arena` names make `write`, once the frame in flight that owns the arena,
/// if any, has been taken back, and answers what the write owes the host.
///
/// # Safety
///
/// `arena` must be a live arena handle on the document thread.
pub(crate) unsafe fn write(arena: *mut c_void, write: LayoutWrite) -> HostPayment {
    // SAFETY: Guaranteed by the caller.
    match unsafe { crate::render_owner::ask_about(arena, Query::Write(write)) } {
        Answer::Payment(payment) => payment,
        _ => HostPayment::nothing(),
    }
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
    pub(crate) fn answer(self, arena: &LayoutNodeArena) -> LayoutReadAnswer {
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
