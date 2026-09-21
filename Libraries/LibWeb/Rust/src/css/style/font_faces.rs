/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The document's `@font-face` table, published.
//!
//! `FontComputer::m_font_faces` is a live `HashMap<FontFaceKey, Vector<FontFaceState>>` whose
//! entries are GC-visible objects with statuses, timers and fetches hanging off them. Resolving a
//! font from it therefore means being on the document thread. This module holds the same table as
//! an immutable per-generation snapshot: the facts matching needs, the facts a cascade entry
//! needs, and nothing else.
//!
//! The snapshot is `Send + Sync` with no `unsafe impl` anywhere in this module. Every field is a
//! `Box<[T]>` of `#[repr(C)]` plain data, a `Box<[u16]>` of UTF-16 family text, or a
//! [`RetainedTypeface`], which holds its `Gfx::Typeface` as an address. The compile-time assertion
//! below is what keeps that true.
//!
//! The resolver itself is C++ (`Web::CSS::resolve_font_cascade`): everything it calls -
//! `Gfx::Typeface::font`, `Gfx::FontCascadeList`, `Gfx::FontDatabase`, `Platform::FontPlugin` - is
//! C++, so one implementation parameterized by this table is smaller and safer than a second copy
//! of the CSS font-matching ladder. C++ reads the table back through [`FfiFontFaceSnapshotView`],
//! which is pointer-and-length over these boxes: the storage is already the C layout.

use libgfx_rust::font::RetainedTypeface;
use std::ffi::c_void;
use std::sync::Arc;

/// One entry of the table: a `FontFaceKey` and the run of faces registered under it, in order.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FfiFontFaceKey {
    pub family_offset: u32,
    pub family_length: u32,
    pub weight_min: i32,
    pub weight_max: i32,
    pub slope: i32,
    pub width: i32,
    pub first_record: u32,
    pub record_count: u32,
}

/// One `@font-face`, carrying exactly what `FrozenFontList` names per entry - the typeface it
/// resolved to, its `unicode-range`, and whether it is ready or still pending, with the number the
/// document knows it by - plus nothing else, because nothing else is read.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FfiFontFaceRecord {
    /// `FontFaceState::id()`, so a resolver that wants this face loaded can name it.
    pub face_id: u64,
    /// The address of the loaded `Gfx::Typeface`, or zero while the face is pending.
    pub typeface: u64,
    pub range_offset: u32,
    pub range_count: u32,
    /// `Gfx::PendingFontState`, peeked when the snapshot was built.
    pub pending_state: u8,
    /// Bit 0: the face has `src` urls, so a style selecting it can want it loaded. Bit 1: its
    /// `font-display` period failed or its load errored, so it contributes nothing. Bit 2: its
    /// `unicode-range` is narrower than everything, so it is fetched on demand. Bit 3: its status
    /// is still `unloaded`. NB: only the C++ resolver reads these; see `FontFaceSnapshotFlags`.
    pub flags: u8,
    pub padding: [u8; 6],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FfiFontFaceRange {
    pub first_code_point: u32,
    pub last_code_point: u32,
}

/// A borrowed, flat view of a snapshot. The host fills one of these to publish a table and reads
/// one back to resolve from it; it never owns anything.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct FfiFontFaceSnapshotView {
    pub keys: *const FfiFontFaceKey,
    pub key_count: usize,
    pub records: *const FfiFontFaceRecord,
    pub record_count: usize,
    pub family_text: *const u16,
    pub family_text_length: usize,
    pub ranges: *const FfiFontFaceRange,
    pub range_count: usize,
    pub generation: u64,
}

impl Default for FfiFontFaceSnapshotView {
    fn default() -> Self {
        Self {
            keys: std::ptr::null(),
            key_count: 0,
            records: std::ptr::null(),
            record_count: 0,
            family_text: std::ptr::null(),
            family_text_length: 0,
            ranges: std::ptr::null(),
            range_count: 0,
            generation: 0,
        }
    }
}

/// The `@font-face` table at one font-environment generation. Immutable once built.
pub struct FontFaceSnapshot {
    generation: u64,
    keys: Box<[FfiFontFaceKey]>,
    records: Box<[FfiFontFaceRecord]>,
    family_text: Box<[u16]>,
    ranges: Box<[FfiFontFaceRange]>,
    /// Parallel to `records`, holding the reference that keeps each loaded typeface alive.
    _typefaces: Box<[RetainedTypeface]>,
}

const _: () = {
    const fn assert_send_and_sync<T: Send + Sync>() {}
    assert_send_and_sync::<FontFaceSnapshot>();
    assert_send_and_sync::<Arc<FontFaceSnapshot>>();

    // C++ mirrors these three in `CSS/FontResolution.h`, and a layout difference would be silent.
    assert!(size_of::<FfiFontFaceKey>() == 32 && align_of::<FfiFontFaceKey>() == 4);
    assert!(size_of::<FfiFontFaceRecord>() == 32 && align_of::<FfiFontFaceRecord>() == 8);
    assert!(size_of::<FfiFontFaceRange>() == 8 && align_of::<FfiFontFaceRange>() == 4);
    assert!(size_of::<FfiFontFaceSnapshotView>() == 72 && align_of::<FfiFontFaceSnapshotView>() == 8);
};

impl FontFaceSnapshot {
    fn view(&self) -> FfiFontFaceSnapshotView {
        FfiFontFaceSnapshotView {
            keys: self.keys.as_ptr(),
            key_count: self.keys.len(),
            records: self.records.as_ptr(),
            record_count: self.records.len(),
            family_text: self.family_text.as_ptr(),
            family_text_length: self.family_text.len(),
            ranges: self.ranges.as_ptr(),
            range_count: self.ranges.len(),
            generation: self.generation,
        }
    }

    /// # Safety
    ///
    /// Every pointer in `view` must address at least the number of elements it is paired with,
    /// and every non-zero `typeface` address must name a live `Gfx::Typeface`.
    unsafe fn from_view(view: &FfiFontFaceSnapshotView) -> Self {
        // SAFETY: The caller guarantees each pointer addresses its stated length.
        let (keys, records, family_text, ranges) = unsafe {
            (
                slice_or_empty(view.keys, view.key_count).to_vec(),
                slice_or_empty(view.records, view.record_count).to_vec(),
                slice_or_empty(view.family_text, view.family_text_length).to_vec(),
                slice_or_empty(view.ranges, view.range_count).to_vec(),
            )
        };
        let typefaces = records
            .iter()
            // SAFETY: The caller guarantees a non-zero address names a live typeface.
            .map(|record| unsafe { RetainedTypeface::retain(record.typeface as usize) })
            .collect::<Vec<_>>();
        Self {
            generation: view.generation,
            keys: keys.into_boxed_slice(),
            records: records.into_boxed_slice(),
            family_text: family_text.into_boxed_slice(),
            ranges: ranges.into_boxed_slice(),
            _typefaces: typefaces.into_boxed_slice(),
        }
    }
}

/// # Safety
///
/// `data` must address `length` elements, or `length` must be zero.
unsafe fn slice_or_empty<'a, T>(data: *const T, length: usize) -> &'a [T] {
    if length == 0 {
        return &[];
    }
    // SAFETY: The caller guarantees the pointer addresses `length` elements.
    unsafe { std::slice::from_raw_parts(data, length) }
}

/// Publishes one generation of the table. The returned pointer is an `Arc<FontFaceSnapshot>` the
/// caller owns and must hand back to [`rust_font_face_snapshot_release`].
///
/// # Safety
///
/// `view` must point to a filled [`FfiFontFaceSnapshotView`] whose arrays are live for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_font_face_snapshot_build(view: *const FfiFontFaceSnapshotView) -> *const c_void {
    assert!(!view.is_null(), "a font face snapshot view must not be null");
    // SAFETY: The caller guarantees the view is live and filled.
    let snapshot = unsafe { FontFaceSnapshot::from_view(&*view) };
    Arc::into_raw(Arc::new(snapshot)).cast()
}

/// # Safety
///
/// `snapshot` must be a pointer [`rust_font_face_snapshot_build`] returned and nobody released.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_font_face_snapshot_release(snapshot: *const c_void) {
    if snapshot.is_null() {
        return;
    }
    // SAFETY: The caller guarantees this is a live pointer from `..._build`.
    unsafe { drop(Arc::from_raw(snapshot.cast::<FontFaceSnapshot>())) };
}

/// Fills `out_view` with a borrowed view of the table. The view is valid for as long as the caller
/// holds its reference to `snapshot`.
///
/// # Safety
///
/// `snapshot` must be a live pointer from [`rust_font_face_snapshot_build`], and `out_view` must
/// address a writable [`FfiFontFaceSnapshotView`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_font_face_snapshot_view(snapshot: *const c_void, out_view: *mut FfiFontFaceSnapshotView) {
    assert!(!out_view.is_null(), "a font face snapshot view must not be null");
    if snapshot.is_null() {
        // SAFETY: The caller guarantees the out-pointer is writable.
        unsafe { out_view.write(FfiFontFaceSnapshotView::default()) };
        return;
    }
    // SAFETY: The caller guarantees this is a live pointer from `..._build`.
    let snapshot = unsafe { &*snapshot.cast::<FontFaceSnapshot>() };
    // SAFETY: The caller guarantees the out-pointer is writable.
    unsafe { out_view.write(snapshot.view()) };
}

// The standalone cargo test binary has no C++ side, so the typeface a published record names is
// an address nobody dereferences and its reference counting is stubbed out here.
unsafe extern "C" {
    fn ladybird_libweb_font_cascade_memo_ref(memo: *const c_void);
    fn ladybird_libweb_font_cascade_memo_unref(memo: *const c_void);
}

/// One reference to the host's `Web::CSS::FontCascadeMemo`, held as an address so that the handle
/// is `Send + Sync` by construction. The memo is the resolver's own memo, not document state: it
/// remembers answers to a pure function of a published table and a request, and it is guarded by
/// its own lock so that the stage can fill it from wherever it runs.
pub(crate) struct RetainedFontCascadeMemo(usize);

impl RetainedFontCascadeMemo {
    /// # Safety
    ///
    /// `address` must be zero, or the address of a live `Web::CSS::FontCascadeMemo`.
    pub unsafe fn retain(address: usize) -> Option<Self> {
        if address == 0 {
            return None;
        }
        // SAFETY: The caller guarantees the memo is live for this call.
        unsafe { ladybird_libweb_font_cascade_memo_ref(address as *const c_void) };
        Some(Self(address))
    }

    pub fn address(&self) -> usize {
        self.0
    }
}

impl Drop for RetainedFontCascadeMemo {
    fn drop(&mut self) {
        // SAFETY: `retain` took the reference this releases.
        unsafe { ladybird_libweb_font_cascade_memo_unref(self.0 as *const c_void) };
    }
}

/// Takes one more reference to a published table, for a holder that outlives the publisher's own.
///
/// # Safety
///
/// `snapshot` must be a live pointer from [`rust_font_face_snapshot_build`].
pub(crate) unsafe fn retained(snapshot: *const c_void) -> Option<Arc<FontFaceSnapshot>> {
    if snapshot.is_null() {
        return None;
    }
    // SAFETY: The caller guarantees this is a live pointer; the clone below is the caller's own.
    let borrowed = std::mem::ManuallyDrop::new(unsafe { Arc::from_raw(snapshot.cast::<FontFaceSnapshot>()) });
    Some(Arc::clone(&borrowed))
}

/// The address a held table is named by, so the resolver can ask it for its view.
pub(crate) fn as_pointer(snapshot: &Arc<FontFaceSnapshot>) -> *const c_void {
    Arc::as_ptr(snapshot).cast()
}

#[cfg(test)]
mod ffi_test_stubs {
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_typeface_ref(_typeface: *const std::ffi::c_void) {}
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_gfx_typeface_unref(_typeface: *const std::ffi::c_void) {}
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_libweb_font_cascade_memo_ref(_memo: *const std::ffi::c_void) {}
    #[unsafe(no_mangle)]
    extern "C" fn ladybird_libweb_font_cascade_memo_unref(_memo: *const std::ffi::c_void) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_published_table_owns_its_own_copy_of_every_array() {
        let mut keys = vec![FfiFontFaceKey {
            family_offset: 0,
            family_length: 3,
            weight_min: 400,
            weight_max: 700,
            slope: 0,
            width: 100,
            first_record: 0,
            record_count: 1,
        }];
        let mut records = vec![FfiFontFaceRecord {
            face_id: 7,
            typeface: 0,
            range_offset: 0,
            range_count: 1,
            pending_state: 1,
            flags: 0b1001,
            padding: [0; 6],
        }];
        let mut family_text = vec![b'a' as u16, b'b' as u16, b'c' as u16];
        let mut ranges = vec![FfiFontFaceRange {
            first_code_point: 0x41,
            last_code_point: 0x5a,
        }];
        let view = FfiFontFaceSnapshotView {
            keys: keys.as_ptr(),
            key_count: keys.len(),
            records: records.as_ptr(),
            record_count: records.len(),
            family_text: family_text.as_ptr(),
            family_text_length: family_text.len(),
            ranges: ranges.as_ptr(),
            range_count: ranges.len(),
            generation: 5,
        };
        let published = unsafe { rust_font_face_snapshot_build(&raw const view) };

        // The publisher's arrays go away; the table must not follow them.
        keys.clear();
        records.clear();
        family_text.clear();
        ranges.clear();

        let mut read_back = FfiFontFaceSnapshotView::default();
        unsafe { rust_font_face_snapshot_view(published, &raw mut read_back) };
        assert_eq!(read_back.generation, 5);
        assert_eq!(read_back.key_count, 1);
        assert_eq!(read_back.record_count, 1);
        let key = unsafe { *read_back.keys };
        assert_eq!(key.weight_max, 700);
        let record = unsafe { *read_back.records };
        assert_eq!(record.face_id, 7);
        assert_eq!(record.flags, 0b1001);
        let text = unsafe { std::slice::from_raw_parts(read_back.family_text, read_back.family_text_length) };
        assert_eq!(text, [b'a' as u16, b'b' as u16, b'c' as u16]);
        let range = unsafe { *read_back.ranges };
        assert_eq!(range.last_code_point, 0x5a);

        let held = unsafe { retained(published) }.unwrap();
        assert_eq!(as_pointer(&held), published);
        unsafe { rust_font_face_snapshot_release(published) };
        // The publisher let go; the holder's own reference still answers.
        let mut held_view = FfiFontFaceSnapshotView::default();
        unsafe { rust_font_face_snapshot_view(as_pointer(&held), &raw mut held_view) };
        assert_eq!(held_view.generation, 5);
    }

    #[test]
    fn a_null_table_views_as_an_empty_one() {
        let mut view = FfiFontFaceSnapshotView {
            generation: 9,
            ..Default::default()
        };
        unsafe { rust_font_face_snapshot_view(std::ptr::null(), &raw mut view) };
        assert_eq!(view.generation, 0);
        assert_eq!(view.record_count, 0);
        assert!(unsafe { retained(std::ptr::null()) }.is_none());
        unsafe { rust_font_face_snapshot_release(std::ptr::null()) };
    }
}
