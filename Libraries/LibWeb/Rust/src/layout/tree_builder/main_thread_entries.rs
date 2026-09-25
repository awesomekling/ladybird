/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The FFI entries of the parent module that mint the main thread capability. Only this module can
//! construct its marker, and the entries are private, so stage code in the parent module can neither
//! mint the capability nor call an entry that does.

use super::*;

pub(crate) struct MainThreadFfiEntry {
    _private: (),
}

const MAIN_THREAD_FFI_ENTRY: MainThreadFfiEntry = MainThreadFfiEntry { _private: () };

/// Detaches a top-layer element's layout placement and clears every stale projected subtree.
///
/// # Safety
///
/// The callback table, arena, and element must remain valid for the duration of the call.
#[unsafe(no_mangle)]
unsafe extern "C" fn rust_detach_top_layer_element_layout_subtree(arena: *mut c_void, style_node: u32) {
    assert!(!arena.is_null());
    // SAFETY: The entry point's contract puts this call on the document thread.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: Guaranteed by the entry point's contract.
    unsafe {
        super::layout_node_arena::paying_host_handbacks(&main_thread, arena, || {
            detach_top_layer_element_layout_subtree(arena.cast(), style_node);
        });
    }
}

/// Detaches what is left of a node's boxes as the node leaves the document, while its identity
/// still names them: its synthetic pseudo-elements' boxes, subtree and all, the paint state of its
/// own box, and its box's top layer placement, which is a viewport child rather than part of the
/// parent's box subtree, so the parent's rebuild would never detach it. The rows are found by
/// identity, so no shell is made for any of this.
///
/// # Safety
///
/// The arena must remain valid for the duration of the call, which must be made on the document
/// thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn rust_detach_remaining_layout_rows_for_removal(arena: *mut c_void, style_node: u32) {
    assert!(!arena.is_null());
    // SAFETY: The entry point's contract puts this call on the document thread.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: Guaranteed by the entry point's contract.
    unsafe {
        super::layout_node_arena::paying_host_handbacks(&main_thread, arena, || {
            detach_remaining_layout_rows_for_removal(arena.cast(), style_node);
        });
    }
}

/// Makes the shells a finished layout tree build walk owes the host, which `walk` holds and this
/// takes. The rest of its host half is the frame's to pay, and the image resources its rows are
/// owed wait for the frame to be over. Answers with the build's outcome.
///
/// # Safety
///
/// The arena must remain valid for the duration of the call, which must be made on the document
/// thread, and `walk` must point to an `Option<LayoutTreeBuildWalk>` holding the walk of this arena.
#[unsafe(no_mangle)]
unsafe extern "C" fn rust_pay_layout_tree_build(arena: *mut c_void, walk: *mut c_void) -> FfiLayoutTreeBuildOutcome {
    assert!(!walk.is_null());
    // SAFETY: The entry point's contract puts this call on the document thread.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let host = dom_tree_builder_host(arena);
    // SAFETY: Guaranteed by the entry point's contract.
    let LayoutTreeBuildWalk(outcome) = unsafe { &mut *walk.cast::<Option<LayoutTreeBuildWalk>>() }
        .take()
        .expect("a layout tree build walk is paid once");

    let layout_host = host.layout();
    let arena = layout_host.arena();
    for row in arena.take_shells_owed_to_host() {
        arena.node_shell(&main_thread, row);
    }
    outcome
}
