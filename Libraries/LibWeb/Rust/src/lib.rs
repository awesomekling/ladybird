/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// The browser transfers HTML buffers to C++, so both sides must use the same allocator.
// The standalone replay program has no C++ runtime or cross-language buffer transfers.
#[cfg(not(feature = "style-replay"))]
/// cbindgen:ignore
#[path = "../../../RustAllocator.rs"]
mod rust_allocator;

#[path = "../../../RustPanic.rs"]
mod rust_panic;

mod encoding_detection;
mod font_seal;
pub use libcompositing_rust::fast_hash;

pub(crate) mod cow_column;
pub mod css;
pub mod layout;
pub mod painting;
pub(crate) mod stage;
pub(crate) mod stage_thread;
pub mod svg;

pub use libweb_html_tokenizer as html_tokenizer;

use crate::rust_panic::abort_on_panic;

unsafe fn bytes_from_raw<'a>(bytes: *const u8, len: usize) -> Option<&'a [u8]> {
    unsafe {
        if len == 0 {
            return Some(&[]);
        }
        if bytes.is_null() {
            eprintln!("bytes_from_raw: null pointer with non-zero length {len}");
            return None;
        }
        Some(std::slice::from_raw_parts(bytes, len))
    }
}

// The standalone cargo test binaries have no C++ side, so the process-wide state this crate is not
// allowed to hold is stubbed out here. See `LibGfx/RustProcessState.cpp`.
#[cfg(test)]
mod process_state_test_stubs {
    use std::ffi::c_void;

    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_set_host_reaching_call_hook(_hook: extern "C" fn(*const u8, usize)) {}
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_note_host_reaching_call(_name: *const u8, _length: usize) {}
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_note_wanted_pending_face(face_id: u64) {
        WANTED.lock().unwrap().push((face_id, false));
    }
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_requeue_wanted_pending_face(face_id: u64) {
        WANTED.lock().unwrap().push((face_id, true));
    }
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_take_wanted_pending_faces(
        context: *mut c_void,
        visit: extern "C" fn(*mut c_void, u64, bool),
    ) {
        let wanted = std::mem::take(&mut *WANTED.lock().unwrap());
        for (face_id, has_been_retried) in wanted {
            visit(context, face_id, has_been_retried);
        }
    }
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_next_path_identity() -> u64 {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    }
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_register_image_frame(_id: u64, _frame: *const c_void) {}
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_forget_image_frame(_id: u64, _frame: *const c_void) {}
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_image_frame_for_id(_id: u64, _out: *mut c_void) -> *mut c_void {
        std::ptr::null_mut()
    }
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_note_crate_copy(_marker: *const c_void) {}

    static WANTED: std::sync::Mutex<Vec<(u64, bool)>> = std::sync::Mutex::new(Vec::new());
}
