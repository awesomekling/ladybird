/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use std::collections::HashMap;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::{Arc, Mutex, OnceLock, Weak};

unsafe extern "C" {
    fn ladybird_gfx_decoded_image_frame_retain(
        frame: *const c_void,
        out_snapshot: *mut FfiImageFrameSnapshot,
    ) -> *mut c_void;
    fn ladybird_gfx_decoded_image_frame_release(frame: *mut c_void);

    // The id-to-frame registry is LibGfx's: this crate is compiled into two libraries, and a
    // frame registered through one copy has to be findable through the other. See
    // `LibGfx/RustProcessState.cpp`.
    fn ladybird_gfx_process_register_image_frame(id: u64, frame: *const c_void);
    fn ladybird_gfx_process_forget_image_frame(id: u64, frame: *const c_void);
    fn ladybird_gfx_process_image_frame_for_id(id: u64, out_snapshot: *mut FfiImageFrameSnapshot) -> *mut c_void;
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FfiImageFrameSnapshot {
    pub id: u64,
    pub width: i32,
    pub height: i32,
}

struct ImageFrameEntry {
    snapshot: FfiImageFrameSnapshot,
    raw: NonNull<c_void>,
}

// SAFETY: The entry owns an immutable Gfx::DecodedImageFrame copy. Its bitmap is
// atomically reference counted, and neither the frame nor its color space is
// mutated while the entry is live.
unsafe impl Send for ImageFrameEntry {}
// SAFETY: See the Send implementation above.
unsafe impl Sync for ImageFrameEntry {}

impl Drop for ImageFrameEntry {
    fn drop(&mut self) {
        image_frame_storage().lock().unwrap().remove(&self.snapshot.id);
        // SAFETY: Forgetting the frame before releasing it is what keeps a concurrent lookup from
        // retaining a copy that is on its way out.
        unsafe {
            ladybird_gfx_process_forget_image_frame(self.snapshot.id, self.raw.as_ptr());
            // SAFETY: ImageFrameHandle::retain took the copy this releases.
            ladybird_gfx_decoded_image_frame_release(self.raw.as_ptr());
        }
    }
}

#[derive(Clone)]
pub struct ImageFrameHandle(Arc<ImageFrameEntry>);

/// This copy's handles, so that two handles for one frame are one `Arc`. Only the dedup is per
/// copy, and duplicating it costs a redundant retain and nothing else; the id-to-frame map that
/// `resolve` needs is LibGfx's, because a frame registered through the other copy of this crate
/// has to be findable here.
fn image_frame_storage() -> &'static Mutex<HashMap<u64, Weak<ImageFrameEntry>>> {
    static STORAGE: OnceLock<Mutex<HashMap<u64, Weak<ImageFrameEntry>>>> = OnceLock::new();
    STORAGE.get_or_init(|| Mutex::new(HashMap::new()))
}

impl ImageFrameHandle {
    /// # Safety
    ///
    /// `frame` must point to a live `Gfx::DecodedImageFrame`.
    pub unsafe fn retain(frame: *const c_void) -> Self {
        let mut snapshot = FfiImageFrameSnapshot::default();
        // SAFETY: The caller guarantees the frame is live, and the out-pointer
        // addresses a local. The returned copy is owned by the entry.
        let raw = unsafe { ladybird_gfx_decoded_image_frame_retain(frame, &raw mut snapshot) };
        let raw = NonNull::new(raw).expect("Gfx::DecodedImageFrame copy must not be null");
        let mut storage = image_frame_storage().lock().unwrap();
        if let Some(entry) = storage.get(&snapshot.id).and_then(Weak::upgrade) {
            // The existing immutable record is the same decoded frame. Drop the
            // redundant copy made before the registry lookup.
            unsafe { ladybird_gfx_decoded_image_frame_release(raw.as_ptr()) };
            return Self(entry);
        }
        let entry = Arc::new(ImageFrameEntry { snapshot, raw });
        storage.insert(snapshot.id, Arc::downgrade(&entry));
        // SAFETY: The entry owns the copy this registers, and forgets it before releasing it.
        unsafe { ladybird_gfx_process_register_image_frame(snapshot.id, raw.as_ptr()) };
        Self(entry)
    }

    pub fn resolve(id: u64) -> Option<Self> {
        if id == 0 {
            return None;
        }
        if let Some(entry) = image_frame_storage().lock().unwrap().get(&id).and_then(Weak::upgrade) {
            return Some(Self(entry));
        }
        // The frame was registered through the other copy of this crate, which has an entry of its
        // own. Take a copy of the frame and make one here too.
        let mut snapshot = FfiImageFrameSnapshot::default();
        // SAFETY: The registry retains the frame under its own lock, so the copy is live.
        let raw = unsafe { ladybird_gfx_process_image_frame_for_id(id, &raw mut snapshot) };
        let raw = NonNull::new(raw)?;
        let mut storage = image_frame_storage().lock().unwrap();
        if let Some(entry) = storage.get(&id).and_then(Weak::upgrade) {
            // SAFETY: Another thread made the entry first; this copy is redundant.
            unsafe { ladybird_gfx_decoded_image_frame_release(raw.as_ptr()) };
            return Some(Self(entry));
        }
        let entry = Arc::new(ImageFrameEntry { snapshot, raw });
        storage.insert(snapshot.id, Arc::downgrade(&entry));
        // SAFETY: The entry owns the frame this registers, and forgets it before releasing it.
        unsafe { ladybird_gfx_process_register_image_frame(snapshot.id, raw.as_ptr()) };
        Some(Self(entry))
    }

    #[inline]
    pub fn id(&self) -> u64 {
        self.0.snapshot.id
    }

    #[inline]
    pub fn width(&self) -> i32 {
        self.0.snapshot.width
    }

    #[inline]
    pub fn height(&self) -> i32 {
        self.0.snapshot.height
    }

    #[inline]
    pub fn as_raw(&self) -> *const c_void {
        self.0.raw.as_ptr()
    }
}

/// # Safety
///
/// `frame` must point to a live `Gfx::DecodedImageFrame`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ladybird_gfx_image_frame_handle_create(frame: *const c_void) -> *const c_void {
    Arc::into_raw(unsafe { ImageFrameHandle::retain(frame) }.0).cast()
}

/// # Safety
///
/// `handle` must be null or an Arc pointer returned by this module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ladybird_gfx_image_frame_handle_ref(handle: *const c_void) -> *const c_void {
    if handle.is_null() {
        return handle;
    }
    unsafe { Arc::increment_strong_count(handle.cast::<ImageFrameEntry>()) };
    handle
}

/// # Safety
///
/// `handle` must be null or an owned Arc pointer returned by this module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ladybird_gfx_image_frame_handle_unref(handle: *const c_void) {
    if !handle.is_null() {
        drop(unsafe { Arc::from_raw(handle.cast::<ImageFrameEntry>()) });
    }
}

/// # Safety
///
/// `handle` must be null or a live Arc pointer returned by this module.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ladybird_gfx_image_frame_handle_id(handle: *const c_void) -> u64 {
    if handle.is_null() {
        return 0;
    }
    unsafe { &*handle.cast::<ImageFrameEntry>() }.snapshot.id
}

impl PartialEq for ImageFrameHandle {
    fn eq(&self, other: &Self) -> bool {
        self.id() == other.id()
    }
}

impl Eq for ImageFrameHandle {}

impl std::fmt::Debug for ImageFrameHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ImageFrameHandle")
            .field("id", &self.id())
            .field("width", &self.width())
            .field("height", &self.height())
            .finish()
    }
}
