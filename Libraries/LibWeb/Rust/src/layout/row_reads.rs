/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The rows of a document's layout that C++ names by slot: the row a node is bound to, and the rows linked to one.
//! Each is a read of the arena by slot, which reaches no host state.

use super::LayoutNodeArena;
use super::node_data::{FfiNodeLink, NodeKind, NodeSlotId};
use crate::css::style::tree::StyleNodeID;
use std::ffi::c_void;

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

    fn of(arena: &LayoutNodeArena, slot: NodeSlotId) -> Self {
        if slot.is_invalid() {
            return Self::NONE;
        }
        Self {
            slot,
            kind: arena.data(slot).kind.get(),
        }
    }
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
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let slot = if generated_for == 0 {
        arena.bound_row(style_node)
    } else {
        arena.bound_pseudo_element_row(style_node, generated_for)
    };
    FfiBoundRow::of(arena, slot)
}

/// The viewport row the document is bound to.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_bound_viewport_row(arena: *mut c_void) -> FfiBoundRow {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    FfiBoundRow::of(arena, arena.bound_viewport_row())
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
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    FfiBoundRow::of(arena, arena.node_link_slot(slot, link))
}

/// The row `slot` names, or none if the row is no longer live, which a slot noted earlier may not be.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_row_if_live(arena: *mut c_void, slot: NodeSlotId) -> FfiBoundRow {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    if !arena.slot_is_live(slot) {
        return FfiBoundRow::NONE;
    }
    FfiBoundRow::of(arena, slot)
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
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    arena
        .node_containing_block_if_live(slot)
        .filter(|containing_block| arena.slot_is_live(*containing_block))
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
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    // The ring is a column of links rather than a borrow, so the host may re-enter the arena from `visit`. What it
    // must not do is change which rows are built for the node.
    // SAFETY: The host answers synchronously.
    arena.for_each_row_built_for_same_node(slot, |row| unsafe { visit(context, row) });
}
