/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The main-side access census: every time main-thread code turns an arena handle into the arena,
//! which is how the DOM side reads or writes render-owned state: layout rows, committed layout,
//! paintable rows, scroll state and hit-test lists. With a render thread overlapping the main
//! thread, each of those passages is either a join, where the main thread waits for the render
//! side to take in what the invalidation journal holds, or a read of the last committed state.
//! The census counts passages per call site, the passages made while the document's journal
//! held marks the render side had not taken yet (the would-be forced joins), and the passages
//! made while a stage was running, which are the pipeline's own. It also counts the document's
//! display list recordings, so a page's passages can be put per rendering update.
//!
//! `LIBWEB_CENSUS_MAIN_READS=1` turns it on, and `LIBWEB_CENSUS_MAIN_READS_LOG` names a file each
//! process appends its counts to; they go to stderr otherwise. Off, a passage costs one flag test.

use super::HostTables;
use crate::css::style::fast_hash::FastMap as HashMap;
use std::cell::Cell;
use std::ffi::c_void;
use std::panic::Location;
use std::sync::{Mutex, OnceLock};

#[derive(Default)]
struct Counts {
    passages: u64,
    with_journal_pending: u64,
    during_stage: u64,
}

#[derive(Default)]
struct Census {
    counts: HashMap<&'static Location<'static>, Counts>,
    rendering_updates: u64,
    rendering_updates_with_journal_pending: u64,
    unflushed: u64,
}

// Process-wide rather than per thread: a process can end with documents still alive, and the exit
// handler that writes their counts out runs after thread-locals are gone.
static CENSUS: Mutex<Option<Census>> = Mutex::new(None);

thread_local! {
    static COUNTED_PASSAGE_DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// How many passages the census counts before it writes its counts out on its own.
const FLUSH_INTERVAL: u64 = 1 << 20;

#[inline(always)]
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if let Some(enabled) = ENABLED.get() {
        return *enabled;
    }
    *ENABLED.get_or_init(|| {
        let enabled = std::env::var_os("LIBWEB_CENSUS_MAIN_READS").is_some();
        if enabled {
            // A process that ends by exiting may still have documents alive, and their counts
            // with them.
            // SAFETY: The handler only formats and writes the process's counts.
            unsafe { atexit(flush_at_exit) };
        }
        enabled
    })
}

unsafe extern "C" {
    fn atexit(handler: extern "C" fn()) -> std::ffi::c_int;
}

extern "C" fn flush_at_exit() {
    flush();
}

fn journal_pending(handle: *mut c_void) -> bool {
    // SAFETY: Every caller holds a live handle from `layout_arena_create`.
    unsafe { HostTables::from_handle(handle) }
        .invalidation_journal_pending
        .get()
}

fn a_stage_is_running(handle: *mut c_void) -> bool {
    // SAFETY: Every caller holds a live handle from `layout_arena_create`, and this shared borrow
    // ends before the caller takes its own.
    unsafe { &*handle.cast::<super::LayoutNodeArena>() }.a_stage_is_running()
}

/// Counts one passage from `call_site` through the arena `handle` names.
#[inline(always)]
pub(crate) fn note_arena_access(call_site: &'static Location<'static>, handle: *mut c_void) {
    if enabled() {
        count_arena_access(call_site, handle);
    }
}

#[inline(never)]
fn count_arena_access(call_site: &'static Location<'static>, handle: *mut c_void) {
    if COUNTED_PASSAGE_DEPTH.with(Cell::get) != 0 {
        return;
    }
    let pending = journal_pending(handle);
    let during_stage = a_stage_is_running(handle);
    let unflushed = {
        let mut census = CENSUS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let census = census.get_or_insert_with(Census::default);
        let entry = census.counts.entry(call_site).or_default();
        entry.passages += 1;
        entry.with_journal_pending += u64::from(pending);
        entry.during_stage += u64::from(during_stage);
        census.unflushed += 1;
        census.unflushed
    };
    if unflushed >= FLUSH_INTERVAL {
        flush();
    } else if unflushed % 1024 == 0 {
        flush_if_due();
    }
}

/// Writes the counts out if a while has passed since they last were. A renderer process is
/// usually killed rather than let exit, so nothing may wait for its end.
fn flush_if_due() {
    static LAST_FLUSH: Mutex<Option<std::time::Instant>> = Mutex::new(None);
    let now = std::time::Instant::now();
    {
        let mut last = LAST_FLUSH.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if last.is_some_and(|last| now.duration_since(last) < std::time::Duration::from_millis(100)) {
            return;
        }
        *last = Some(now);
    }
    flush();
}

/// Runs `operation`, part of a passage already counted, without counting the handle conversions
/// it makes.
pub(crate) fn within_counted_passage<R>(operation: impl FnOnce() -> R) -> R {
    if !enabled() {
        return operation();
    }
    COUNTED_PASSAGE_DEPTH.with(|depth| depth.set(depth.get() + 1));
    let result = operation();
    COUNTED_PASSAGE_DEPTH.with(|depth| depth.set(depth.get() - 1));
    result
}

/// Counts one display list recording of the document the arena `handle` names.
pub(crate) fn note_rendering_update(handle: *mut c_void) {
    if !enabled() {
        return;
    }
    let pending = journal_pending(handle);
    {
        let mut census = CENSUS.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let census = census.get_or_insert_with(Census::default);
        census.rendering_updates += 1;
        census.rendering_updates_with_journal_pending += u64::from(pending);
    }
    flush_if_due();
}

/// Writes out what was counted since the last time, one record per call site.
pub(crate) fn flush() {
    if !enabled() {
        return;
    }
    let Some(census) = CENSUS.lock().unwrap_or_else(|poisoned| poisoned.into_inner()).take() else {
        return;
    };
    let mut records = Vec::new();
    for (call_site, counts) in census.counts {
        records.push(format!(
            "MAIN SIDE ACCESS site={call_site} passages={} journal_pending={} during_stage={}\n",
            counts.passages, counts.with_journal_pending, counts.during_stage
        ));
    }
    let (updates, updates_with_journal_pending) =
        (census.rendering_updates, census.rendering_updates_with_journal_pending);
    if updates != 0 {
        records.push(format!(
            "MAIN SIDE ACCESS rendering_updates={updates} journal_pending={updates_with_journal_pending}\n"
        ));
    }
    let log = std::env::var_os("LIBWEB_CENSUS_MAIN_READS_LOG");
    for record in records {
        match &log {
            Some(path) => {
                use std::io::Write;
                if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                    let _ = file.write_all(record.as_bytes());
                }
            }
            None => eprint!("{record}"),
        }
    }
}

/// Whether the main-side access census is on, so the document only reports its journal's state
/// when someone counts.
#[unsafe(no_mangle)]
pub extern "C" fn layout_main_side_census_enabled() -> bool {
    enabled()
}

/// Records whether the invalidation journal of the document that owns the arena holds marks the
/// render side has not taken yet.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_note_invalidation_journal_pending(arena: *mut c_void, pending: bool) {
    // SAFETY: Guaranteed by the caller.
    unsafe { HostTables::from_handle(arena) }
        .invalidation_journal_pending
        .set(pending);
}
