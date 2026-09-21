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
/// The live arena and viewport are exclusively available on the document thread.
/// The query stays readable during the call. DOM callbacks may inspect eligibility
/// and collect ranges, but must not mutate the DOM or layout tree.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_find_matching_text(
    arena: *mut c_void,
    viewport: NodeSlotId,
    query: FfiUtf16View,
    case_sensitive: bool,
    is_searchable: unsafe extern "C" fn(*mut c_void) -> bool,
    context: *mut c_void,
    append: unsafe extern "C" fn(*mut c_void, FfiDomTextRange),
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: The host lends the query for this synchronous operation.
    let query = unsafe { query.to_utf16() }.expect("query carries no storage");
    if query.is_empty() {
        return;
    }
    // SAFETY: Source and DOM callbacks finish before native cache publication.
    unsafe { ensure_searchable_text(&main_thread, arena.cast(), viewport, is_searchable) };
    let matches = {
        // SAFETY: Cache preparation is complete; matching performs no callbacks.
        let arena = unsafe { LayoutNodeArena::from_handle(arena) };
        let mut matches = Vec::new();
        for block in arena.searchable_text.as_ref().expect("search cache was prepared") {
            let mut offset = 0;
            while let Some(index) = find_text(&block.text, &query, offset, case_sensitive) {
                if let Some(range) = block.dom_range(&main_thread, arena, index..index + query.len()) {
                    matches.push(range);
                }
                offset = index + query.len() + 1;
                if offset >= block.text.len() {
                    break;
                }
            }
        }
        matches
    };
    for range in matches {
        // SAFETY: Native matching is complete; the host resolves each row's DOM node itself.
        unsafe { append(context, range) };
    }
}
