/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the main thread writes of a document's layout, as the typed changes the render owner applies with the document's
//! arena. The main thread names a node by its slot and sends; it never reaches the arena itself. What it reads of the
//! layout tree, it reads from the rows the owner publishes (see [`super::row_reads`]).

use super::LayoutNodeArena;
use super::layout_node_arena::{HostPayment, LayoutUpdateMarksHandle};
use super::node_data::{CompositorAnimationFrameKind, NodeFlag, NodeSlotId};
use super::partial_relayout::FfiPossibleBoundaryUpdate;
use super::tree_builder::FfiRemovedBoxPlace;
use super::used_values::FfiCssPixelPoint;
use crate::css::style::tree::{NaturalSize, StyleNodeID};
use crate::render_owner::{Answer, ArenaChange, ChangeSeq, Query};
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
    /// The box may be a partial relayout boundary, which the owner decides as it takes the change in: a boundary lays
    /// out alone, as `update` says.
    SetNeedsLayoutUpdateOfPossibleBoundary {
        node: NodeSlotId,
        update: FfiPossibleBoundaryUpdate,
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
    /// The nodes `nodes` names leave the document: what is left of their boxes is detached while their identities still
    /// name them (see [`super::tree_builder::detach_remaining_layout_rows_for_removal`]), and what that owes the host goes
    /// with the next payment.
    DetachRemainingRowsForRemoval {
        nodes: Box<[StyleNodeID]>,
    },
    /// The box of the node `place` names leaves its parent's box in place, which the rows the document thread read
    /// allowed as `layout_node` and `parent`: see [`super::tree_builder::rust_detach_removed_box_in_place`].
    DetachRemovedBoxInPlace {
        place: FfiRemovedBoxPlace,
        layout_node: NodeSlotId,
        parent: NodeSlotId,
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
            Self::SetNeedsLayoutUpdateOfPossibleBoundary { node, update } => {
                if !arena.slot_is_live(node) {
                    return;
                }
                if arena.node_is_partial_relayout_boundary(node) {
                    if update == FfiPossibleBoundaryUpdate::StyleChange {
                        arena.set_node_flag(node, NodeFlag::NeedsOwnGeometryUpdate, true);
                    }
                    arena.set_needs_layout_update(node, false);
                } else if update == FfiPossibleBoundaryUpdate::ChildListInsertion {
                    arena.defer_child_list_insertion_layout_update(node);
                } else {
                    arena.set_needs_layout_update(node, true);
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
                    // The document reads from the rows it was published whether a text renders a slice of its data,
                    // and has the build slice it again instead of sending this.
                    debug_assert!(
                        !arena.text_has_source_range(node),
                        "a text sliced by its first letter is rebuilt, not invalidated in place"
                    );
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
            Self::DetachRemainingRowsForRemoval { nodes } => {
                let payment = arena.owed_for(|arena| {
                    for &node in &nodes {
                        super::tree_builder::detach_remaining_layout_rows_for_removal(arena, node);
                    }
                });
                arena.append_leftover_payment(payment);
            }
            Self::DetachRemovedBoxInPlace {
                place,
                layout_node,
                parent,
            } => {
                if super::tree_builder::removed_box_detachable_in_place(arena, &place) == Some((layout_node, parent)) {
                    arena.release_published_paintable_rows();
                    let payment = arena.owed_for(|arena| {
                        super::tree_builder::detach_removed_box_in_place(arena, layout_node, parent);
                    });
                    arena.append_leftover_payment(payment);
                } else {
                    // The rows changed under what the document thread read of them: the tree is built again, without
                    // the box.
                    arena.set_needs_full_layout_tree_update(true);
                }
            }
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

    /// Whether applying the change can alter what the rows the owner publishes answer the document thread: the rows
    /// themselves, their DOM facts, identities and styles. Marks for the next layout, the flags it reads, and what the
    /// arena keeps for the host (the document thread keeps its own copy of the compositor animation frames) alter
    /// none.
    pub(crate) fn alters_published_rows(&self) -> bool {
        match self {
            Self::SetNodeDomPaintFacts { .. }
            | Self::DetachRemainingRowsForRemoval { .. }
            | Self::DetachRemovedBoxInPlace { .. }
            | Self::StyleNodeChanged { .. }
            | Self::InstallRowStyle { .. }
            | Self::ReplaceRowStyleRecord { .. }
            | Self::AdoptDerivedNodeStyle { .. }
            | Self::InstallAnimationSample { .. } => true,
            Self::SetNodeFlag { .. }
            | Self::SetNeedsLayoutUpdateOfPossibleBoundary { .. }
            | Self::SetNodeNeedsCompositorAnimationFrame { .. }
            | Self::SetNeedsLayoutUpdate { .. }
            | Self::SetNeedsOwnGeometryUpdate { .. }
            | Self::SetNeedsFullLayoutTreeUpdate(_)
            | Self::ResetCachedIntrinsicSizesOfSelfAndAncestors { .. }
            | Self::DeferChildListInsertionLayoutUpdate { .. }
            | Self::InvalidateTextContent { .. }
            | Self::EnrollTextAfterLanguageChange { .. }
            | Self::RecordPartialRelayoutEscape
            | Self::SetOwnedImageNaturalSize { .. }
            | Self::SetElementScrollOffset { .. }
            | Self::SetPseudoElementScrollOffset { .. }
            | Self::SetIdentityInFocusedTextControl { .. }
            | Self::SetListOwnerHasStaleItemCounters { .. }
            | Self::SetRowImageObservers { .. }
            | Self::RowOwnsImageProvider { .. }
            | Self::SetStyleImageResourcesAttached { .. }
            | Self::PinBoundBoxStyleRecordForDetachment { .. }
            | Self::SetTableSpans { .. } => false,
        }
    }

    /// Whether the change can mark a node for layout, which is all a change can do to whether the layout is up to date.
    fn lays_out_again(&self) -> bool {
        matches!(
            self,
            Self::SetNeedsLayoutUpdate { .. }
                | Self::SetNeedsLayoutUpdateOfPossibleBoundary { .. }
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
        let arena = crate::render_owner::do_owner_work_here(|owner| unsafe {
            &mut *super::ArenaHandle::held_by_waiting_thread(owner, arena)
        })
        .arena_mut();
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
    /// tree, and the subtree is detached from its parent, if it has one. Where `clears_committed_boxes`, the committed
    /// boxes of its rows are cleared first, as for a box that leaves a tree that stays.
    DropSubtree {
        root: NodeSlotId,
        clears_committed_boxes: bool,
    },
    /// Detaches the layout placement of the top layer element `element` and clears every stale projected subtree of
    /// it, with the document's layout tree update marks lent to the write.
    DetachTopLayerElement(StyleNodeID),
}

impl LayoutWrite {
    /// Makes the write, and answers what it owes the host, after what the owner's changes owed it before.
    pub(crate) fn apply(self, arena: &mut LayoutNodeArena) -> HostPayment {
        let written = match self {
            Self::DropSubtree {
                root,
                clears_committed_boxes,
            } => {
                if !arena.slot_is_live(root) {
                    return arena.take_leftover_payment();
                }
                arena.release_published_paintable_rows();
                arena.owed_for(|arena| {
                    if clears_committed_boxes {
                        let mut rows = Vec::new();
                        arena.for_each_node_in_layout_subtree_in_pre_order(root, |row| rows.push(row));
                        for row in rows {
                            // SAFETY: Nothing borrows the arena across the clear.
                            unsafe { crate::painting::ffi::clear_paintable_row_of_node(arena, row) };
                        }
                    }
                    super::layout_node_arena::prepare_subtree_for_detach(arena, root);
                    super::layout_node_arena::detach_and_free_subtree(arena, root);
                })
            }
            Self::DetachTopLayerElement(element) => arena.owed_for(|arena| {
                super::tree_builder::detach_top_layer_element_layout_subtree(arena, element);
            }),
        };
        let mut payment = arena.take_leftover_payment();
        payment.append(written);
        payment
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
