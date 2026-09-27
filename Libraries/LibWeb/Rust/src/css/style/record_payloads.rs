/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! A style record's group payload pointers, as one immutable value that owns what it names.
//!
//! The engine keeps each record's payloads in an [`Arc`], and whatever reads them after the engine
//! may have let go of the record takes a clone: a layout row, and through the row every frame the
//! arena publishes. The value holds one reference on each group payload it names, so a reader never
//! depends on the engine keeping a record around.
//!
//! A group payload is not `Send`: dropping its last reference runs destructors that only the
//! document thread may run (a font cascade list's reference count, for one). The last clone of a
//! record's payloads may be dropped anywhere, a frame on the thread that painted it included, so the
//! value releases its references only on its document's thread, the one a stage that made it acted
//! for. Dropped anywhere else, a stage run for that thread included, it sends them back to its
//! engine's [`StylePayloadsHome`], which releases them the next time the engine may reclaim records.

use crate::css::computed_values::{release_group_payload, retain_group_payload};
use crate::css::host_shared::SharedPayload;
use crate::layout::node_data::{FfiStylePayloads, STYLE_GROUP_COUNT};
use std::sync::{Arc, Mutex, PoisonError, Weak};
use std::thread::ThreadId;

/// References on a record's group payloads. Nothing releases them but [`Self::release`]: a value
/// dropped without it leaks its references rather than releasing them on the wrong thread.
#[repr(C)]
struct GroupReferences {
    groups: [SharedPayload; STYLE_GROUP_COUNT],
    len: usize,
}

impl GroupReferences {
    const NONE: Self = Self {
        groups: [SharedPayload::null(); STYLE_GROUP_COUNT],
        len: 0,
    };

    fn as_slice(&self) -> &[SharedPayload] {
        &self.groups[..self.len]
    }

    /// Releases the references, on the thread whose engine made them or while it waits.
    fn release(self) {
        for (index, &payload) in self.as_slice().iter().enumerate() {
            if !payload.is_null() {
                release_group_payload(index, payload.as_ptr());
            }
        }
    }
}

/// Where the payloads an engine made come back to when their last clone is dropped on another
/// thread than the engine's.
#[derive(Default)]
pub(crate) struct StylePayloadsHome {
    returned: Mutex<Vec<GroupReferences>>,
}

impl StylePayloadsHome {
    /// Releases the references that came back, which the caller may do only where the engine may
    /// reclaim records: on the engine's thread, or while that thread waits on the engine.
    pub(crate) fn release_returned(&self) {
        let returned = std::mem::take(&mut *self.returned.lock().unwrap_or_else(PoisonError::into_inner));
        for references in returned {
            references.release();
        }
    }
}

/// The group payloads of one style record, each retained for as long as the value lives.
///
/// The pointers are stored inline, so a reader holding the `Arc` reaches a group payload with the
/// same number of loads as through the bare pointer array.
#[repr(C)]
pub(crate) struct StyleRecordPayloads {
    references: GroupReferences,
    home_thread: ThreadId,
    home: Weak<StylePayloadsHome>,
}

// The value is read from any thread: its payloads are immutable once published, and a read touches
// no reference count (see `HostShared`). It is dropped on any thread too, since its drop releases
// nothing but on its home thread and otherwise hands its references back to the engine that made
// them. The group payloads themselves are not `Send`, and the value never lets one be dropped
// elsewhere.
const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<StyleRecordPayloads>();
    assert_send_sync::<StylePayloadsHome>();
};

impl StyleRecordPayloads {
    /// Takes a reference on each of `payloads`, which the value releases when it is dropped, on the
    /// document thread the caller acts for or through `home`.
    pub(crate) fn retain(payloads: &[SharedPayload], home: &Arc<StylePayloadsHome>) -> Self {
        assert!(
            payloads.len() <= STYLE_GROUP_COUNT,
            "a style record has more groups than a row reads"
        );
        let mut references = GroupReferences::NONE;
        for (index, &payload) in payloads.iter().enumerate() {
            if !payload.is_null() {
                retain_group_payload(index, payload.as_ptr());
            }
            references.groups[index] = payload;
        }
        references.len = payloads.len();
        Self {
            references,
            home_thread: crate::stage_thread::acting_thread(),
            home: Arc::downgrade(home),
        }
    }

    pub(crate) fn as_slice(&self) -> &[SharedPayload] {
        self.references.as_slice()
    }

    /// The address of the group pointer array, as a row's untyped style pointer names it.
    pub(crate) fn as_ptr(&self) -> *const std::ffi::c_void {
        (&raw const self.references.groups).cast()
    }

    /// The payloads as the group pointer array a row's style reads.
    pub(crate) fn as_ffi(&self) -> &FfiStylePayloads {
        const _: () = assert!(size_of::<FfiStylePayloads>() == size_of::<[SharedPayload; STYLE_GROUP_COUNT]>());
        // SAFETY: `SharedPayload` is `repr(transparent)` over `*const c_void`, and `FfiStylePayloads`
        // is a `repr(C)` struct of that one array, so the two have the same layout. Groups past
        // `len` are null, as a record with fewer groups reads them.
        unsafe { &*(&raw const self.references.groups).cast::<FfiStylePayloads>() }
    }
}

impl std::ops::Deref for StyleRecordPayloads {
    type Target = [SharedPayload];

    fn deref(&self) -> &[SharedPayload] {
        self.as_slice()
    }
}

impl Drop for StyleRecordPayloads {
    fn drop(&mut self) {
        let references = std::mem::replace(&mut self.references, GroupReferences::NONE);
        if std::thread::current().id() == self.home_thread {
            references.release();
            return;
        }
        if let Some(home) = self.home.upgrade() {
            home.returned
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(references);
            return;
        }
        // The engine is gone, and so is the thread its payloads may be released on: the references
        // leak as they go out of scope rather than run a group's destructor here.
        debug_assert!(false, "style payloads outlived their engine off its thread");
    }
}
