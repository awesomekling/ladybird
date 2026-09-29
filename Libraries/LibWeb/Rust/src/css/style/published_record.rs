/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! A style record as the engine published it, as one immutable value that owns everything a read
//! of the record reads.
//!
//! The document thread reads computed style (getComputedStyle, the layout nodes it keeps, the
//! drain's own comparisons) through [`PublishedStyleRecord`]s. Each owns its record's group
//! payloads, the payloads of its base record, the base record's frozen longhand table, its
//! animated overlay and the metadata a read looks at, so a reader holding one reads it wherever
//! the engine is, and whatever the engine reclaims meanwhile. It holds no engine, no arena and no
//! identity it has to look up: a read made through one cannot reach render-side state, and sends
//! nothing to it.
//!
//! The engine makes one where it hands a record to the host: the drain installing what a style
//! pass published, and the answer of a style read demand.

use super::bridge::FfiStyleRecordView;
use super::record_payloads::StyleRecordPayloads;
use crate::css::animated_overlay::AnimatedOverlay;
use crate::css::computed_longhand_table::ComputedLonghandTable;
use crate::css::host_shared::SharedPayload;
use std::ffi::c_void;
use std::ptr::NonNull;
use std::sync::Arc;

/// One strong reference to a frozen longhand table.
pub(crate) struct SharedLonghandTable(NonNull<ComputedLonghandTable>);

// SAFETY: A table is shared only once it is frozen and its storage is dense (`into_raw_shared`),
// after which nothing writes it and a read of it touches no reference count. Its values are
// `StyleValueData`, whose counts are an `Arc`'s, so its last reference may go anywhere.
unsafe impl Send for SharedLonghandTable {}
unsafe impl Sync for SharedLonghandTable {}

impl SharedLonghandTable {
    /// Takes a reference of its own on `table`.
    ///
    /// # Safety
    /// `table` must be a live, frozen table.
    pub(crate) unsafe fn retain(table: NonNull<ComputedLonghandTable>) -> Self {
        unsafe { crate::css::computed_longhand_table::rust_computed_longhand_table_retain(table.as_ptr()) };
        Self(table)
    }

    fn as_ptr(&self) -> *const ComputedLonghandTable {
        self.0.as_ptr()
    }
}

impl Drop for SharedLonghandTable {
    fn drop(&mut self) {
        // SAFETY: The value owns one strong reference, taken in `retain`.
        unsafe { crate::css::computed_longhand_table::rust_computed_longhand_table_release(self.0.as_ptr()) };
    }
}

/// A published style record: see the module documentation.
pub(crate) struct PublishedStyleRecord {
    pub(crate) style_record: u64,
    pub(crate) payloads: Arc<StyleRecordPayloads>,
    /// The payloads of the record an animation overlay was layered on; the record's own for a base
    /// record.
    pub(crate) base_payloads: Arc<StyleRecordPayloads>,
    /// The base record's table: an animation overlay stores no table entries.
    pub(crate) longhand_table: Option<SharedLonghandTable>,
    pub(crate) animated_overlay: Option<Arc<AnimatedOverlay>>,
    pub(crate) pseudo_element_styles: u64,
    pub(crate) counter_style_environment_identity: u64,
    pub(crate) animation_overlay_identity: u64,
    pub(crate) custom_property_environment: u64,
    pub(crate) dependency_flags: u8,
}

// The value is made wherever the engine runs and read and dropped on the document thread, while the
// engine goes on: it holds only what it owns, and nothing of the engine.
const _: () = {
    const fn assert_published<T: Send + Sync + 'static>() {}
    assert_published::<PublishedStyleRecord>();
};

impl PublishedStyleRecord {
    /// A base record of `payloads` alone, for a test that drives rows without an engine.
    #[cfg(test)]
    pub(crate) fn of_payloads(style_record: u64, payloads: Arc<StyleRecordPayloads>) -> Arc<Self> {
        Arc::new(Self {
            style_record,
            base_payloads: payloads.clone(),
            payloads,
            longhand_table: None,
            animated_overlay: None,
            pseudo_element_styles: 0,
            counter_style_environment_identity: 0,
            animation_overlay_identity: 0,
            custom_property_environment: 0,
            dependency_flags: 0,
        })
    }

    /// The record as the engine's comparisons read one, or None for a record without a longhand table, which no
    /// element holds.
    fn comparison_view(&self) -> Option<super::computed::StyleRecordView<'_>> {
        let table = self.longhand_table.as_ref()?;
        // SAFETY: The record holds a reference to the frozen table.
        let longhand_values = SharedPayload::from_pointer_slice(unsafe { &*table.as_ptr() }.value_pointers());
        Some(super::computed::StyleRecordView {
            payloads: self.payloads.as_slice(),
            base_payloads: self.base_payloads.as_slice(),
            longhand_table: crate::css::host_shared::HostShared::new(table.as_ptr()),
            longhand_values,
            animated_overlay: crate::css::host_shared::HostShared::new(
                self.animated_overlay
                    .as_deref()
                    .map_or(std::ptr::null(), std::ptr::from_ref),
            ),
            pseudo_element_styles: self.pseudo_element_styles,
            counter_style_environment_identity: self.counter_style_environment_identity,
            animation_overlay_identity: self.animation_overlay_identity,
            dependency_flags: self.dependency_flags,
        })
    }

    fn view(&self) -> FfiStyleRecordView {
        FfiStyleRecordView {
            payloads: SharedPayload::as_pointer_slice(self.payloads.as_slice()).as_ptr(),
            base_payloads: SharedPayload::as_pointer_slice(self.base_payloads.as_slice()).as_ptr(),
            longhand_table: self
                .longhand_table
                .as_ref()
                .map_or(std::ptr::null(), |table| table.as_ptr().cast()),
            animated_overlay: self
                .animated_overlay
                .as_deref()
                .map_or(std::ptr::null(), |overlay| std::ptr::from_ref(overlay).cast()),
            payload_count: self.payloads.as_slice().len(),
            pseudo_element_styles: self.pseudo_element_styles,
            counter_style_environment_identity: self.counter_style_environment_identity,
            animation_overlay_identity: self.animation_overlay_identity,
            dependency_flags: self.dependency_flags,
            present: true,
        }
    }
}

/// Hands a published record to C++, which reads it through `published_style_record_read` and
/// releases it with `published_style_record_release`.
pub(crate) fn into_handle(record: Arc<PublishedStyleRecord>) -> *const c_void {
    Arc::into_raw(record).cast()
}

/// A reference of its own to the record C++ names by `handle`.
///
/// # Safety
///
/// `handle` must be a live handle from `into_handle`.
pub(crate) unsafe fn shared_from_handle(handle: *const c_void) -> Arc<PublishedStyleRecord> {
    let record = handle.cast::<PublishedStyleRecord>();
    // SAFETY: Guaranteed by the caller: the handle holds a reference, to which this adds one.
    unsafe {
        Arc::increment_strong_count(record);
        Arc::from_raw(record)
    }
}

/// What C++ reads of a published record, all of it borrowed from the record for as long as the
/// handle lives.
#[repr(C)]
pub struct FfiPublishedStyleRecordRead {
    pub view: FfiStyleRecordView,
    pub style_record: u64,
    pub custom_property_environment: u64,
}

/// SAFETY: `record` must be a live handle from `into_handle`.
unsafe fn record_from_handle<'a>(record: *const c_void) -> &'a PublishedStyleRecord {
    assert!(!record.is_null(), "published style record handle is null");
    unsafe { &*record.cast::<PublishedStyleRecord>() }
}

/// # Safety
///
/// `record` must be a live handle the engine handed out. Everything the result points at lives as
/// long as the handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn published_style_record_read(record: *const c_void) -> FfiPublishedStyleRecordRead {
    let record = unsafe { record_from_handle(record) };
    FfiPublishedStyleRecordRead {
        view: record.view(),
        style_record: record.style_record,
        custom_property_environment: record.custom_property_environment,
    }
}

/// Whether the record styles a counter or a quote, as
/// [`ComputedValuesView::affects_generated_content_state`](crate::css::computed_value_views::ComputedValuesView::affects_generated_content_state)
/// says. A read of the record alone, which asks the engine nothing.
///
/// # Safety
///
/// `record` must be a live handle the engine handed out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn published_style_record_affects_generated_content_state(record: *const c_void) -> bool {
    let record = unsafe { record_from_handle(record) };
    crate::css::computed_value_views::ComputedValuesView::new(SharedPayload::as_pointer_slice(
        record.payloads.as_slice(),
    ))
    .affects_generated_content_state()
}

/// What an element's place in the tree adds to what moving it from one record to another damages: whether it is an
/// SVG graphics element, whether it folds its transform into its SVG container's layout (its parent is an SVG element
/// other than a foreignObject), and whether the viewport takes its overflow (it is the root, or the html root's first
/// body child).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiElementDamageFacts {
    pub is_svg_graphics_element: bool,
    pub folds_transform_into_svg_container_layout: bool,
    pub propagates_overflow_to_viewport: bool,
}

/// What moving an element from the record `old` to the record `new` damages, with what `facts` say of its place in the
/// tree, as the engine answers it: a read of the two records alone, which asks the engine nothing.
///
/// # Safety
///
/// `old` must be null or, as `new` must be, a live handle the engine handed out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn published_style_record_damage(
    old: *const c_void,
    new: *const c_void,
    facts: FfiElementDamageFacts,
) -> u32 {
    // SAFETY: Guaranteed by the caller.
    let old = unsafe { old.cast::<PublishedStyleRecord>().as_ref() };
    let new = unsafe { record_from_handle(new) };
    match (
        old.and_then(PublishedStyleRecord::comparison_view),
        new.comparison_view(),
    ) {
        (Some(old), Some(new)) => super::style_invalidation::element_record_move_damage(&old, &new, facts),
        _ => super::style_invalidation::unreadable_record_damage("PublishedStyleRecordDamageWithoutTable"),
    }
}

/// # Safety
///
/// `record` must be a live handle the engine handed out, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn published_style_record_release(record: *const c_void) {
    assert!(!record.is_null(), "published style record handle is null");
    drop(unsafe { Arc::from_raw(record.cast::<PublishedStyleRecord>()) });
}
