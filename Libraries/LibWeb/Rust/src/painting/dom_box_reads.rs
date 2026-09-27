/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What DOM code reads of a document's boxes, named by the node a box was built for and the slot the arena bound it
//! to, so that no layout node is made or asked for the read.

use crate::css::style::tree::StyleNodeID;
use crate::layout::node_data::{NodeFlag, NodeKind, NodeSlotId};
use crate::layout::node_facts;
use crate::painting::ffi::arena_from_handle;
use std::ffi::c_void;

/// The slot of the principal box of the element with `style_node`: the table wrapper box of a table, which contains
/// its caption boxes, and otherwise the box the element is bound to. An invalid slot for an element with no box.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_principal_box_of(arena: *mut c_void, style_node: u32) -> NodeSlotId {
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return NodeSlotId::INVALID;
    };
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena) };
    let row = arena.bound_row(style_node);
    if row.is_invalid() {
        return row;
    }
    if arena
        .node_style_if_live(row)
        .is_some_and(|style| style.display().is_table_inside())
        && let Some(parent) = arena.node_parent_if_live(row)
        && arena.node_kind_if_live(parent) == Some(NodeKind::TableWrapper)
    {
        return parent;
    }
    row
}

/// Whether the box in `slot` is a scroll container, as the overflow its style has once the viewport took what the
/// root and the body propagate to it says. The viewport always is one.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_box_is_scroll_container(arena: *mut c_void, slot: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena) };
    let Some(kind) = arena.node_kind_if_live(slot) else {
        return false;
    };
    node_facts::kind_and_style_make_scroll_container(kind, arena.node_style_if_live(slot))
}

/// The box has a parent in the layout tree.
pub const BOX_PLACEMENT_HAS_PARENT: u8 = 1 << 0;
/// The box's parent is an anonymous box.
pub const BOX_PLACEMENT_PARENT_IS_ANONYMOUS: u8 = 1 << 1;
/// The box is a child of the viewport, directly or through anonymous boxes only, which is where the top layer places the
/// box of an element rendered in it.
pub const BOX_PLACEMENT_IN_TOP_LAYER: u8 = 1 << 2;
/// The box's children are inline-level.
pub const BOX_PLACEMENT_CHILDREN_ARE_INLINE: u8 = 1 << 3;
/// The box has children in the layout tree.
pub const BOX_PLACEMENT_HAS_CHILDREN: u8 = 1 << 4;

/// Where the box in `slot` sits in the layout tree, as the `BOX_PLACEMENT_*` bits say it. No bit for a slot that names
/// no live box.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_box_placement(arena: *mut c_void, slot: NodeSlotId) -> u8 {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena) };
    if arena.node_kind_if_live(slot).is_none() {
        return 0;
    }
    let is_anonymous = |node: NodeSlotId| arena.node_flags_if_live(node) & NodeFlag::Anonymous as u32 != 0;
    let mut placement = 0;
    if arena.node_flags_if_live(slot) & NodeFlag::ChildrenAreInline as u32 != 0 {
        placement |= BOX_PLACEMENT_CHILDREN_ARE_INLINE;
    }
    if arena.node_first_child_if_live(slot).is_some() {
        placement |= BOX_PLACEMENT_HAS_CHILDREN;
    }
    let Some(parent) = arena.node_parent_if_live(slot) else {
        return placement;
    };
    placement |= BOX_PLACEMENT_HAS_PARENT;
    if is_anonymous(parent) {
        placement |= BOX_PLACEMENT_PARENT_IS_ANONYMOUS;
    }
    let mut topmost = slot;
    while let Some(parent) = arena.node_parent_if_live(topmost)
        && is_anonymous(parent)
    {
        topmost = parent;
    }
    if let Some(parent) = arena.node_parent_if_live(topmost)
        && arena.node_kind_if_live(parent) == Some(NodeKind::Viewport)
    {
        placement |= BOX_PLACEMENT_IN_TOP_LAYER;
    }
    placement
}

/// Whether the layout subtree the box in `slot` heads holds a box of `kind`, that box included.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_subtree_contains_box_of_kind(
    arena: *mut c_void,
    slot: NodeSlotId,
    kind: NodeKind,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena) };
    if arena.node_kind_if_live(slot).is_none() {
        return false;
    }
    let mut contains = false;
    arena.for_each_node_in_layout_subtree_in_pre_order_with_pruning(slot, |node| {
        contains |= arena.node_kind_if_live(node) == Some(kind);
        !contains
    });
    contains
}

/// Whether the box in `ancestor` is the box in `descendant` or one of its ancestors in the layout tree.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_box_is_inclusive_ancestor_of(
    arena: *mut c_void,
    ancestor: NodeSlotId,
    descendant: NodeSlotId,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena) };
    if ancestor.is_invalid() || arena.node_kind_if_live(descendant).is_none() {
        return false;
    }
    let mut current = Some(descendant);
    while let Some(node) = current {
        if node == ancestor {
            return true;
        }
        current = arena.node_parent_if_live(node);
    }
    false
}
