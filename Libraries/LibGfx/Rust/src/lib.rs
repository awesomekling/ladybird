/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[cfg(feature = "allocator")]
/// cbindgen:ignore
#[path = "../../../RustAllocator.rs"]
mod rust_allocator;

#[path = "../../../RustPanic.rs"]
mod rust_panic;

pub mod bsp_tree;
pub mod color;
pub mod corner_radii;
pub mod filter;
pub mod font;
pub mod font_catalog;
pub mod geometry;
pub mod image_frame;
pub mod matrix;
pub mod paint_enums;
pub mod path;
pub mod text_layout;
pub mod yuv;

pub use color::*;
pub use corner_radii::*;
pub use geometry::*;
pub use matrix::*;
pub use paint_enums::*;

/// Reports this copy of the crate to LibGfx.
///
/// The crate is compiled into more than one library, so there is more than one of everything in
/// it. That is only a problem for state, which is why none of it lives here (see
/// `LibGfx/RustProcessState.cpp`), but a process should be able to say how many copies it is
/// running, and a test should be able to prove that the shared state is not one of them. Each
/// copy has a marker of its own, and registering its address is how LibGfx counts them.
///
/// `MARKER` is the one `static` this crate keeps, and it is immutable and per-copy on purpose.
#[unsafe(no_mangle)]
pub extern "C" fn ladybird_gfx_register_rust_crate_copy() {
    static MARKER: u8 = 0;
    // SAFETY: The address of a `static` outlives the process.
    unsafe { ladybird_gfx_process_note_crate_copy((&raw const MARKER).cast()) };
}

unsafe extern "C" {
    fn ladybird_gfx_process_note_crate_copy(marker: *const std::ffi::c_void);
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
    extern "C" fn ladybird_gfx_process_set_wanted_face_owner(_owner: u64) -> u64 {
        0
    }
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_process_take_wanted_pending_faces(
        _owner: u64,
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
