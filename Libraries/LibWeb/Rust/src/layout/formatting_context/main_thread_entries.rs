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

/// # Safety
///
/// `arena` must be a live handle with a registered layout host, used on the document thread,
/// and `viewport` must be its live viewport box.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_run_root_layout(
    arena: *mut c_void,
    viewport: NodeSlotId,
    viewport_inline_size_raw: i32,
    viewport_block_size_raw: i32,
    document_in_quirks_mode: bool,
    should_collect_devtools_layout_data: bool,
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: Guaranteed by the entry point's contract.
    unsafe {
        run_root_layout(
            &main_thread,
            arena,
            viewport,
            viewport_inline_size_raw,
            viewport_block_size_raw,
            document_in_quirks_mode,
            should_collect_devtools_layout_data,
        );
    }
}

/// # Safety
///
/// `arena` must be a live handle with a registered layout host, used on the document thread;
/// `root` must be a live partial relayout boundary and `viewport` the arena's live viewport box.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_compute_subtree_layout(
    arena: *mut c_void,
    root: NodeSlotId,
    viewport: NodeSlotId,
    viewport_inline_size_raw: i32,
    viewport_block_size_raw: i32,
    document_in_quirks_mode: bool,
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: Guaranteed by the entry point's contract.
    unsafe {
        compute_subtree_layout(
            &main_thread,
            arena,
            root,
            viewport,
            viewport_inline_size_raw,
            viewport_block_size_raw,
            document_in_quirks_mode,
        );
    }
}
