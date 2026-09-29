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

/// How the engine holds the document thread's table: while the document thread waits on it, the engine may read it;
/// beside a pass in flight, the document thread may pin any record at any moment.
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub(crate) enum HostPinsLend {
    #[default]
    Lent,
    BesideFlight,
}

/// The document thread's pins as the engine holds them: the table, which no host pins in when a test or a replay
/// drives the engine, and whether it is lent now.
#[derive(Default)]
pub(crate) struct HostPins {
    table: Option<HostPinTable>,
    lend: HostPinsLend,
}

impl HostPins {
    pub(crate) fn new(table: HostPinTable) -> Self {
        Self {
            table: Some(table),
            lend: HostPinsLend::Lent,
        }
    }

    /// The table, as long as the engine lives: whoever holds it pins in it without entering the engine.
    pub(crate) fn table(&self) -> Option<&HostPinTable> {
        self.table.as_ref()
    }

    /// The table, when the engine may read it.
    pub(crate) fn lent_table(&self) -> Option<&HostStyleRecordPins> {
        match (&self.table, self.lend) {
            // SAFETY: The document thread waits on the engine while the table is lent.
            (Some(table), HostPinsLend::Lent) => Some(unsafe { table.pins() }),
            _ => None,
        }
    }

    /// Whether the host holds a pin on `record`, or may take one before the engine can know.
    pub(crate) fn may_pin(&self, record: u64) -> bool {
        match self.lent_table() {
            Some(table) => table.has_pins_waiting_for_frame() || table.is_pinned(record),
            None => self.table.is_some(),
        }
    }

    /// Whether the engine has to leave every record it would reclaim for later: beside a pass in
    /// flight, or while a pin the host promised waits for the frame in flight.
    pub(crate) fn defers_reclamation(&self) -> bool {
        match self.lent_table() {
            Some(table) => table.has_pins_waiting_for_frame(),
            None => self.table.is_some(),
        }
    }

    /// Stops lending the table while the engine runs beside the document thread, and returns how it was lent, which
    /// [`Self::restore`] lends it as again once it no longer does.
    pub(crate) fn lend_beside(&mut self) -> HostPinsLend {
        std::mem::replace(&mut self.lend, HostPinsLend::BesideFlight)
    }

    pub(crate) fn restore(&mut self, lend: HostPinsLend) {
        self.lend = lend;
    }
}

/// The document thread's table, owned jointly by the document thread and by everything on the render owner that pins
/// in it or reads it: the engine and the arena that links it, which may both outlive the document thread's own hold.
#[derive(Clone, Default)]
pub(crate) struct HostPinTable(std::sync::Arc<HostStyleRecordPins>);

// SAFETY: Only the document thread reads or writes the table itself, or a thread it waits on; the render owner beside it
// reaches only the channel's sender, which is `Sync`. Once the document thread lets go of it, the engine and its arena
// are left to reach it, which the owner does not do at once.
unsafe impl Send for HostPinTable {}
// SAFETY: As above.
unsafe impl Sync for HostPinTable {}

impl HostPinTable {
    /// Hands the document thread its own hold, for [`Self::from_host`] to reach and [`Self::release_host`] to drop.
    pub(crate) fn into_host(self) -> *const HostStyleRecordPins {
        std::sync::Arc::into_raw(self.0)
    }

    /// Another hold of the table the document thread holds at `pins`.
    ///
    /// # Safety
    /// `pins` must come from [`Self::into_host`] and not have been released.
    pub(crate) unsafe fn from_host(pins: *const HostStyleRecordPins) -> Self {
        // SAFETY: Guaranteed by the caller.
        unsafe { std::sync::Arc::increment_strong_count(pins) };
        // SAFETY: As above; the count just taken is this hold's.
        Self(unsafe { std::sync::Arc::from_raw(pins) })
    }

    /// Drops the document thread's own hold.
    ///
    /// # Safety
    /// `pins` must come from [`Self::into_host`], and the document thread must not reach it again.
    pub(crate) unsafe fn release_host(pins: *const HostStyleRecordPins) {
        // SAFETY: Guaranteed by the caller.
        drop(unsafe { std::sync::Arc::from_raw(pins) });
    }

    /// The table, for the document thread's own pins.
    ///
    /// # Safety
    /// The caller must be the document thread, or run while the document thread waits on it, or hold the table once
    /// the document thread has let go of it.
    pub(crate) unsafe fn pins(&self) -> &HostStyleRecordPins {
        &self.0
    }

    /// The table, for a test that stands in for the document thread.
    #[cfg(test)]
    pub(crate) fn host(&self) -> &HostStyleRecordPins {
        &self.0
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
        // SAFETY: The hold keeps the table alive, and the projection reaches only the sender, which nothing but the
        // render owner uses; the document thread goes on with the rest of the table beside it.
        let sender = unsafe { &*std::ptr::addr_of!((*std::sync::Arc::as_ptr(&self.0)).owner_writes_sender) };
        // The table keeps the receiver as long as the sender.
        let _ = sender.send(write);
    }
}

#[cfg(test)]
mod tests {
    use super::HostPinTable;

    #[test]
    fn the_table_takes_in_the_owners_pins_before_it_is_read_or_written() {
        let host = HostPinTable::default();
        // SAFETY: The test is the document thread.
        let table = unsafe { host.pins() };
        let handle = host.clone();
        let owner = std::thread::spawn(move || {
            handle.pin_from_owner(7);
            handle.pin_from_owner(9);
            handle.unpin_from_owner(9);
        });
        owner.join().unwrap();
        assert!(table.is_pinned(7));
        assert!(!table.is_pinned(9));

        // A write of the document thread's own comes after the pin the owner sent before it.
        host.pin_from_owner(11);
        table.unpin(11);
        table.unpin(7);
        let mut pinned = Vec::new();
        table.for_each_pinned(|record| pinned.push(record));
        assert!(pinned.is_empty());
    }

    /// The engine and its arena hold the table past the document thread's own hold, and pin and read in it as they
    /// leave: the table goes with the last hold.
    #[test]
    fn the_owners_holds_outlive_the_document_threads() {
        let host = HostPinTable::default().into_host();
        // SAFETY: `host` is the document thread's hold.
        let engine = unsafe { HostPinTable::from_host(host) };
        let arena = engine.clone();
        // SAFETY: As above; the test is the document thread.
        unsafe { &*host }.pin(3);
        // SAFETY: As above, and the document thread does not reach it again.
        unsafe { HostPinTable::release_host(host) };
        std::thread::spawn(move || {
            arena.pin_from_owner(5);
            arena.unpin_from_owner(3);
        })
        .join()
        .unwrap();
        // SAFETY: The document thread has let go of the table; the engine is its one reader.
        let table = unsafe { engine.pins() };
        assert!(table.is_pinned(5));
        assert!(!table.is_pinned(3));
    }
}
