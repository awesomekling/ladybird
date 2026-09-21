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
/// The arena must be live, and append_text must synchronously copy the supplied bytes.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_take_layout_trace(arena: *mut c_void, context: *mut c_void, append_text: AppendText) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let text = arena.layout_trace.take(&main_thread, arena);
    unsafe { append_text(context, text.as_ptr(), text.len()) };
}
