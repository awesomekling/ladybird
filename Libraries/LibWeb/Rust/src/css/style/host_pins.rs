/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The style records the document thread pins for its own readers: a record view outside a view
//! epoch, a removed subtree's records until its layout detach has read them, an SVG path length.
//!
//! The document thread takes and releases these pins at any time, a style pass in flight
//! included, and a pin must never wait for that pass. So the document thread owns them, in a table
//! the engine never writes. The engine reads it wherever it would reclaim a record: every time the
//! document thread waits on it, the table is lent to it for the call. A pass running beside the
//! document thread cannot read it, since a pin taken beside the pass may land at any moment: such a
//! pass reclaims no record, and retires what it would have reclaimed until its frame is taken back.

use super::fast_hash::FastMap;
use std::cell::RefCell;
use std::ptr::NonNull;

/// The document thread's style-record pins, counted per record as the host names it.
#[derive(Default)]
pub struct HostStyleRecordPins {
    counts: RefCell<FastMap<u64, u32>>,
}

impl HostStyleRecordPins {
    pub(crate) fn pin(&self, record: u64) {
        assert!(record != 0, "the host pinned a null style record");
        let mut counts = self.counts.borrow_mut();
        let count = counts.entry(record).or_default();
        *count = count.checked_add(1).expect("host style-record pin count overflow");
    }

    pub(crate) fn unpin(&self, record: u64) {
        let mut counts = self.counts.borrow_mut();
        let count = counts
            .get_mut(&record)
            .expect("the host unpinned a style record it had not pinned");
        *count -= 1;
        if *count == 0 {
            counts.remove(&record);
        }
    }

    pub(crate) fn is_pinned(&self, record: u64) -> bool {
        self.counts.borrow().contains_key(&record)
    }

    pub(crate) fn for_each_pinned(&self, mut visit: impl FnMut(u64)) {
        for &record in self.counts.borrow().keys() {
            visit(record);
        }
    }
}

/// What the engine may know of the document thread's pins.
#[derive(Clone, Copy, Default)]
pub(crate) enum HostPinsLend {
    /// No host pins records: an engine a test or a replay drives.
    #[default]
    NoHost,
    /// The document thread waits on the engine, which may read its table.
    Lent(HostPinsHandle),
    /// A pass runs beside the document thread, which may pin any record at any moment.
    BesideFlight(HostPinsHandle),
}

impl HostPinsLend {
    /// Whether the host holds a pin on `record`, or may take one before the engine can know.
    pub(crate) fn may_pin(self, record: u64) -> bool {
        match self {
            Self::NoHost => false,
            // SAFETY: The document thread waits on the engine while the table is lent.
            Self::Lent(handle) => unsafe { handle.0.as_ref() }.is_pinned(record),
            Self::BesideFlight(_) => true,
        }
    }

    /// The table, when the engine may read it.
    pub(crate) fn table(&self) -> Option<&HostStyleRecordPins> {
        match self {
            // SAFETY: As for `may_pin`.
            Self::Lent(handle) => Some(unsafe { handle.0.as_ref() }),
            Self::NoHost | Self::BesideFlight(_) => None,
        }
    }

    /// The table itself, whatever the engine may read of it now.
    pub(crate) fn handle(self) -> Option<HostPinsHandle> {
        match self {
            Self::NoHost => None,
            Self::Lent(handle) | Self::BesideFlight(handle) => Some(handle),
        }
    }

    pub(crate) fn is_beside_flight(self) -> bool {
        matches!(self, Self::BesideFlight(_))
    }

    /// Stops lending the table to a pass that runs beside the document thread.
    pub(crate) fn beside_flight(self) -> Self {
        match self {
            Self::Lent(handle) => Self::BesideFlight(handle),
            other => other,
        }
    }

    /// Lends the table again once the document thread has taken the frame back.
    pub(crate) fn taken_back(self) -> Self {
        match self {
            Self::BesideFlight(handle) => Self::Lent(handle),
            other => other,
        }
    }
}

/// The document thread's table, as the engine holds it.
#[derive(Clone, Copy)]
pub(crate) struct HostPinsHandle(NonNull<HostStyleRecordPins>);

// SAFETY: The engine dereferences the handle only while the document thread waits on it, on
// whichever thread runs the engine then.
unsafe impl Send for HostPinsHandle {}
// SAFETY: As above.
unsafe impl Sync for HostPinsHandle {}

impl HostPinsHandle {
    /// # Safety
    /// `pins` must come from [`super::bridge::style_record_host_pins_create`] and outlive the
    /// engine that holds the handle.
    pub(crate) unsafe fn new(pins: *mut HostStyleRecordPins) -> Self {
        Self(NonNull::new(pins).expect("a host pin table handle is not null"))
    }

    /// The table, for the document thread's own pins.
    ///
    /// # Safety
    /// The caller must be the document thread, or run while the document thread waits on it.
    pub(crate) unsafe fn pins(&self) -> &HostStyleRecordPins {
        // SAFETY: Guaranteed by the caller; the table outlives every holder of the handle.
        unsafe { self.0.as_ref() }
    }
}
