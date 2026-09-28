/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The FFI entries of the parent module that mint the main thread capability. Only this module can
//! construct its marker, and the entries are private, so stage code in the parent module can neither
//! mint the capability nor call an entry that does.

use super::*;
use crate::layout::layout_changes;

pub(crate) struct MainThreadFfiEntry {
    _private: (),
}

const MAIN_THREAD_FFI_ENTRY: MainThreadFfiEntry = MainThreadFfiEntry { _private: () };

/// Takes the layout subtree `root` heads out of the tree, and frees it: every row in it is prepared
/// for leaving the tree, the subtree is detached from its parent, if it has one, and what the rows
/// held is handed back to the host before this returns.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_drop_subtree(arena: *mut c_void, root: NodeSlotId) {
    // SAFETY: Guaranteed by the caller.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe { layout_changes::write(arena, layout_changes::LayoutWrite::DropSubtree(root)) }.pay(&main_thread);
}

/// What the render owner left of applying the batch of a style transaction to the layout nodes of
/// the rows' elements, which the document thread takes with the transaction, before the host
/// installs the batch: what applying it handed back, and what each row marked of its element's
/// layout nodes, with the record it installed, which the install reads to leave a covered row alone.
pub(crate) struct OwnerAppliedStyle {
    handed_back: Option<HostPayment>,
    damages: HashMap<StyleNodeID, (u32, u64)>,
}

impl OwnerAppliedStyle {
    /// Takes what applying a batch left in `arena`, on the render owner, which applied it just now.
    pub(crate) fn take_from(arena: &LayoutNodeArena) -> Self {
        // What the changes the owner applied owe the host comes first.
        let mut handed_back = arena.take_leftover_payment();
        let applied = arena.resolve_flight_style_handbacks();
        let handed_back = match applied {
            Some(applied) => {
                handed_back.append(applied);
                Some(handed_back)
            }
            None => (!handed_back.is_nothing()).then_some(handed_back),
        };
        Self {
            handed_back,
            damages: arena.take_flight_style_damages(),
        }
    }

    /// Pays what applying the batch handed back, and holds what the rows marked in the host tables
    /// of `arena`, beside what earlier transactions of the style update left there, for the host's
    /// install to read.
    ///
    /// # Safety
    ///
    /// On the document thread, from an FFI entry whose C++ contract requires it; `arena` must be
    /// the live arena the owner applied the batch to.
    pub(crate) unsafe fn hand_to_host(self, arena: *mut c_void) {
        assert!(!arena.is_null(), "layout node arena handle is null");
        // SAFETY: Guaranteed by the caller.
        let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
        if let Some(payment) = self.handed_back {
            payment.pay(&main_thread);
        }
        if let Some(host_tables) = main_thread.host_tables() {
            host_tables.hold_owner_style_damages(self.damages);
        }
    }
}

/// Ends the host half of the batches the render owner applied to the layout nodes as the host took a
/// style update's transactions, once the update has installed them: the owner puts back a row the
/// install did not adopt the record of with the record its element holds, before its next unit, and
/// hands what that owes the host with the next payment. Nothing waits for it.
///
/// # Safety
///
/// The arena must be live on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_finish_owner_style_host_half(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // What the rows the owner applied marked that the install did not take goes with the update.
    if let Some(host_tables) = main_thread.host_tables() {
        host_tables.hold_flight_style_damages(Default::default());
    }
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { crate::layout::ArenaHandle::document_of(arena) };
    if document.is_valid() {
        crate::render_owner::send_arena_change(document, crate::render_owner::ArenaChange::FinishOwnerStyleHostHalf);
    }
}
