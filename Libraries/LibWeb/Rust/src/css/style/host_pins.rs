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
//!
//! A pin can also wait for a frame: a removed box's pin is taken where the box's row is, in an arena
//! a frame in flight owns until it is taken in. The document thread counts such pins as they are
//! promised, and the engine reclaims no record while one has yet to land, since the removal has
//! let go of every other pin its boxes' records had.
//!
//! The render owner pins and unpins records for the rows of the arena, the host's readers, as it
//! applies what the document thread sent and runs the units of a frame, often beside the document
//! thread. It does not write the table: it sends each write, in order, through a channel the table
//! takes in before anything reads or writes it next.

use super::fast_hash::FastMap;
use std::cell::{Cell, RefCell};
use std::ptr::NonNull;
use std::sync::mpsc::{Receiver, Sender, channel};

/// A write the render owner made to the document thread's table, which the table takes in before
/// it is read or written next.
enum OwnerPinWrite {
    Pin(u64),
    Unpin(u64),
}

/// The document thread's style-record pins, counted per record as the host names it.
pub struct HostStyleRecordPins {
    counts: RefCell<FastMap<u64, u32>>,
    /// Pins promised for records the table cannot name yet: they wait for the frame in flight.
    pins_waiting_for_frame: Cell<u32>,
    /// What the render owner pinned and unpinned, which the table takes in.
    owner_writes: Receiver<OwnerPinWrite>,
    /// Where the render owner sends them: the one field it reaches.
    owner_writes_sender: Sender<OwnerPinWrite>,
}

impl Default for HostStyleRecordPins {
    fn default() -> Self {
        let (owner_writes_sender, owner_writes) = channel();
        Self {
            counts: RefCell::default(),
            pins_waiting_for_frame: Cell::new(0),
            owner_writes,
            owner_writes_sender,
        }
    }
}

impl HostStyleRecordPins {
    /// Takes in what the render owner wrote since, in order.
    fn take_in_owner_writes(&self) {
        while let Ok(write) = self.owner_writes.try_recv() {
            match write {
                OwnerPinWrite::Pin(record) => self.count_pin(record),
                OwnerPinWrite::Unpin(record) => self.count_unpin(record),
            }
        }
    }

    pub(crate) fn pin(&self, record: u64) {
        self.take_in_owner_writes();
        self.count_pin(record);
    }

    pub(crate) fn unpin(&self, record: u64) {
        self.take_in_owner_writes();
        self.count_unpin(record);
    }

    fn count_pin(&self, record: u64) {
        assert!(record != 0, "the host pinned a null style record");
        let mut counts = self.counts.borrow_mut();
        let count = counts.entry(record).or_default();
        *count = count.checked_add(1).expect("host style-record pin count overflow");
    }

    fn count_unpin(&self, record: u64) {
        let mut counts = self.counts.borrow_mut();
        let count = counts
            .get_mut(&record)
            .expect("the host unpinned a style record it had not pinned");
        *count -= 1;
        if *count == 0 {
            counts.remove(&record);
        }
    }

    /// A pin that waits for the frame in flight is promised: until it lands, the host may pin any
    /// record.
    pub(crate) fn begin_pin_waiting_for_frame(&self) {
        let count = self.pins_waiting_for_frame.get();
        self.pins_waiting_for_frame.set(
            count
                .checked_add(1)
                .expect("waiting host style-record pin count overflow"),
        );
    }

    /// A promised pin has landed, or is no longer needed.
    pub(crate) fn end_pin_waiting_for_frame(&self) {
        let count = self.pins_waiting_for_frame.get();
        assert!(count > 0, "the host ended a pin wait it had not begun");
        self.pins_waiting_for_frame.set(count - 1);
    }

    pub(crate) fn has_pins_waiting_for_frame(&self) -> bool {
        self.pins_waiting_for_frame.get() != 0
    }

    pub(crate) fn is_pinned(&self, record: u64) -> bool {
        self.take_in_owner_writes();
        self.counts.borrow().contains_key(&record)
    }

    pub(crate) fn for_each_pinned(&self, mut visit: impl FnMut(u64)) {
        self.take_in_owner_writes();
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
            Self::Lent(handle) => {
                // SAFETY: The document thread waits on the engine while the table is lent.
                let table = unsafe { handle.0.as_ref() };
                table.has_pins_waiting_for_frame() || table.is_pinned(record)
            }
            Self::BesideFlight(_) => true,
        }
    }

    /// Whether the engine has to leave every record it would reclaim for later: beside a pass in
    /// flight, or while a pin the host promised waits for the frame in flight.
    pub(crate) fn defers_reclamation(self) -> bool {
        match self {
            Self::NoHost => false,
            // SAFETY: As for `may_pin`.
            Self::Lent(handle) => unsafe { handle.0.as_ref() }.has_pins_waiting_for_frame(),
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

    /// Pins `record` for the render owner, which the table takes in before it is next read or written.
    pub(crate) fn pin_from_owner(&self, record: u64) {
        self.send_from_owner(OwnerPinWrite::Pin(record));
    }

    /// Releases a pin [`Self::pin_from_owner`] took, as that does.
    pub(crate) fn unpin_from_owner(&self, record: u64) {
        self.send_from_owner(OwnerPinWrite::Unpin(record));
    }

    fn send_from_owner(&self, write: OwnerPinWrite) {
        // SAFETY: The table outlives every holder of the handle, and the projection reaches only the sender, which
        // nothing but the render owner uses; the document thread goes on with the rest of the table beside it.
        let sender = unsafe { &*std::ptr::addr_of!((*self.0.as_ptr()).owner_writes_sender) };
        // The document thread keeps the receiver as long as the table.
        let _ = sender.send(write);
    }
}

#[cfg(test)]
mod tests {
    use super::{HostPinsHandle, HostStyleRecordPins};

    #[test]
    fn the_table_takes_in_the_owners_pins_before_it_is_read_or_written() {
        let mut table = HostStyleRecordPins::default();
        // SAFETY: The table outlives the handle.
        let handle = unsafe { HostPinsHandle::new(&raw mut table) };
        let owner = std::thread::spawn(move || {
            handle.pin_from_owner(7);
            handle.pin_from_owner(9);
            handle.unpin_from_owner(9);
        });
        owner.join().unwrap();
        assert!(table.is_pinned(7));
        assert!(!table.is_pinned(9));

        // A write of the document thread's own comes after the pin the owner sent before it.
        handle.pin_from_owner(11);
        table.unpin(11);
        table.unpin(7);
        let mut pinned = Vec::new();
        table.for_each_pinned(|record| pinned.push(record));
        assert!(pinned.is_empty());
    }
}
