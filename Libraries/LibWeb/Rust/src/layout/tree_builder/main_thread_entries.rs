/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The FFI entries of the parent module that mint the main thread capability. Only this module can
//! construct its marker, and the entries are private, so stage code in the parent module can neither
//! mint the capability nor call an entry that does.

use super::*;
use crate::layout::layout_changes::{self, LayoutWrite};

pub(crate) struct MainThreadFfiEntry {
    _private: (),
}

const MAIN_THREAD_FFI_ENTRY: MainThreadFfiEntry = MainThreadFfiEntry { _private: () };

/// Detaches a top-layer element's layout placement and clears every stale projected subtree.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn rust_detach_top_layer_element_layout_subtree(arena: *mut c_void, style_node: u32) {
    // A top-layer member the style engine no longer tracks has left the DOM. Nothing of it is in the mirror, and
    // nothing of it is bound to a row, so there is nothing to detach or clear.
    let Some(element) = StyleNodeID::from_raw(style_node) else {
        return;
    };
    // SAFETY: Guaranteed by the caller.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // The clear retires the layout tree update marks of the boxes it gives up, which the document thread holds.
    // SAFETY: As above.
    let payment = unsafe {
        super::super::tree_update_marks::lend_to_stale_box_clear(arena, || {
            layout_changes::write(arena, LayoutWrite::DetachTopLayerElement(element))
        })
    };
    payment.pay(&main_thread);
}
