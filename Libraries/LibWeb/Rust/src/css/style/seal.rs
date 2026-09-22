/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Sealed mode for the style stage.
//!
//! The style stage spans a complete style update, after its document inputs are published and
//! before its outputs are committed.
//! `LIBWEB_SEAL_STYLE_STAGE` turns this gate on. Unset or `0`, it costs only the mode check. `1`
//! reports each callback site once; `abort` makes the first callback fatal. Reports and census
//! totals go to stderr or to the file named by `LIBWEB_SEAL_STYLE_STAGE_LOG`.
//!
//! `longhand_input_freeze` says that preparing one row's longhand transaction read live host
//! state, and its reasons say which. A row that reads none is frozen from the engine's own
//! retained state; the working set the row fills is created from the Rust longhand table and
//! dropped inside the row, so building it is not a read of anything the host already held.
//!
//! `longhand_result_apply` says that applying one row's results reached the host: a GC object, a
//! DOM node, or state the document exposes. A row that writes only the computation's own working
//! set - created and dropped inside the stage - is not counted, because moving that row's
//! application after the batch would change nothing. The working set itself is still a crossing,
//! and `longhand_input_freeze` still counts it for every row.
//!
//! A font cache miss is no longer a host service. The installed resolver answers from the
//! document's published `@font-face` table and the process-wide font services, reads no document
//! and holds no pointer to one, so `between_pass_batch resolve_font` is a stage-local computation
//! the stage's own thread performs. It is still counted, because it is a round *between* passes
//! that a single sealed pass would have to absorb. Callbacks that prepare C++ longhand state or
//! report computed results are
//! crossings of the future thread boundary until they become published inputs or commit messages.
//!
//! UTF-16 fly-string releases are deferred by the complete-update scope and drained after it.
//! CSSOM rule-mutation notifications and rule-compilation visitors run outside a style update.
//! They are counted when instrumented, but are not seal violations. Debug/dump callbacks would be
//! allowed only outside an update; none exists in the audited callback set.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Off,
    Report,
    Abort,
}

#[derive(Clone, Copy, Default)]
struct Counts {
    calls: u64,
    during_style: u64,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("LIBWEB_SEAL_STYLE_STAGE").as_deref() {
        Err(_) | Ok("0") => Mode::Off,
        Ok("abort") => Mode::Abort,
        Ok(_) => Mode::Report,
    })
}

/// Whether the seal keeps a census at all. The host asks once, so a row does not pay for
/// instrumentation that nobody reads.
pub(crate) fn is_reporting() -> bool {
    mode() != Mode::Off
}

thread_local! {
    static UPDATE_DEPTH: Cell<u32> = const { Cell::new(0) };
    static REPORTED: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
    static COUNTS: RefCell<HashMap<&'static str, Counts>> = RefCell::new(HashMap::new());
    static STAGE_INTERLEAVES: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
    static LONGHAND_INPUT_FREEZE_REASONS: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
    static BETWEEN_PASS_BATCHES: RefCell<HashMap<&'static str, (u64, u64)>> = RefCell::new(HashMap::new());
    static HOST_DRIVEN_ROWS: Cell<u64> = const { Cell::new(0) };
    static HOST_RETRY_ENTRIES: Cell<u64> = const { Cell::new(0) };
    static HOST_SAMPLED_ANIMATION_ROWS: Cell<u64> = const { Cell::new(0) };
    static HOST_DRIVEN_ROW_KINDS: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
    static HOST_ENTRY_CAUSES: RefCell<HashMap<HostEntryKey, HostEntryCounts>> = RefCell::new(HashMap::new());
    static CURRENT_HOST_ENTRY: RefCell<Option<HostEntryKey>> = const { RefCell::new(None) };
}

/// What one host entry is: the reason the engine sent this element to the host, which of the
/// three ways in it took, and whether it had a record already.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct HostEntryKey {
    pub(crate) cause: &'static str,
    pub(crate) kind: HostEntryKind,
    pub(crate) cold: bool,
}

/// The three ways one element is entered from the host during an update.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum HostEntryKind {
    Row,
    Retry,
    Sampled,
}

impl HostEntryKind {
    fn name(self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Retry => "retry",
            Self::Sampled => "sampled",
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) struct HostEntryCounts {
    entries: u64,
    applied: u64,
}

/// Record one host entry under the reason the engine declined the element, so the census ranks
/// what reaches the host rather than what the engine attempted. An attempt that declines for a
/// class the host then skips costs nothing; only an entry does.
pub(crate) fn note_host_entry(cause: &'static str, kind: HostEntryKind, cold: bool) {
    if mode() == Mode::Off || UPDATE_DEPTH.with(|depth| depth.get() == 0) {
        return;
    }
    let key = HostEntryKey { cause, kind, cold };
    HOST_ENTRY_CAUSES.with(|causes| {
        causes.borrow_mut().entry(key).or_default().entries += 1;
    });
    // Counted here rather than from the engine's own counter: the counter dies with its engine while this
    // census belongs to the thread, and the two must add up. `flush_engine_decline_census` leaves
    // `retryAfterAncestorCalls` out of its report for that reason.
    if kind == HostEntryKind::Retry {
        HOST_RETRY_ENTRIES.with(|entries| entries.set(entries.get().wrapping_add(1)));
    }
    if kind == HostEntryKind::Row {
        CURRENT_HOST_ENTRY.with(|current| *current.borrow_mut() = Some(key));
    }
}

/// Record that the row the host is driving now applied its results to the main side.
fn note_host_entry_applied() {
    let Some(key) = CURRENT_HOST_ENTRY.with(|current| current.borrow_mut().take()) else {
        return;
    };
    HOST_ENTRY_CAUSES.with(|causes| {
        causes.borrow_mut().entry(key).or_default().applied += 1;
    });
}

/// Record that one row's computation was entered from the host's per-element driver.
///
/// This is not a seal violation on its own: the row may neither read live host state nor write
/// anything the host can see. It is counted because the end state is one sealed pass over the
/// whole update, and a host loop that enters the engine once per element is not that - between
/// two rows control is on the host side, holding host state. Driving this to zero is what makes
/// the stage a single function rather than a sequence of calls.
pub(crate) fn note_host_driven_row(kinds: u8) {
    if mode() == Mode::Off || UPDATE_DEPTH.with(|depth| depth.get() == 0) {
        return;
    }
    HOST_DRIVEN_ROWS.with(|rows| rows.set(rows.get().wrapping_add(1)));
    let names = [
        "in_frozen_batch",
        "pseudo_element",
        "no_previous_record",
        "highlight_parent",
        "longhand_drive_only",
    ];
    HOST_DRIVEN_ROW_KINDS.with(|counts| {
        let mut counts = counts.borrow_mut();
        for (index, name) in names.into_iter().enumerate() {
            if kinds & (1 << index) != 0 {
                let count = counts.entry(name).or_default();
                *count = count.wrapping_add(1);
            }
        }
    });
}

/// Report how often the engine declined to compute a record itself, by the reason it recorded.
/// A host-driven row inside a published batch is a row one of these declined.
///
/// These numbers belong to one engine and are reported when that engine is destroyed, so they are not a run
/// total: an engine that outlives reporting never prints its own. That is why the retries are not reported from
/// `retryAfterAncestorCalls` here as well. The counter and the host-entry census count the same call, one per
/// engine and one per thread, so summing both over a run reads two totals for one population - a full suite run
/// reported 77,170 retry entries beside 75,461 calls. The retry count now has one home, the census, which is
/// drained as the thread runs and so covers the engines that are never destroyed.
pub(crate) fn flush_engine_decline_census<'a>(counters: impl Iterator<Item = (&'a str, u64)>) {
    if mode() == Mode::Off {
        return;
    }
    let mut rows = counters
        .filter(|(name, value)| *value != 0 && name.starts_with("engineComputedRecord"))
        .collect::<Vec<_>>();
    rows.sort_unstable_by_key(|(name, _)| *name);
    for (name, value) in rows {
        write_report(&format!("STYLE SEAL COUNT: engine_record {name}: {value}\n"));
    }
}

/// Record that one row's animations were sampled by the host after the stage returned.
///
/// The stage could not sample the element for itself, so it hands the rest of its
/// finalization back and the host samples between two sealed calls. That is no longer a
/// callback out of sealed computation, but it is still main-side work inside the update, and
/// it stays counted for the same reason `host_driven_rows` is: the end state is one sealed
/// pass, and a row the host has to finish is not that.
pub(crate) fn note_host_sampled_animation_row() {
    if mode() == Mode::Off || UPDATE_DEPTH.with(|depth| depth.get() == 0) {
        return;
    }
    HOST_SAMPLED_ANIMATION_ROWS.with(|rows| rows.set(rows.get().wrapping_add(1)));
    note_host_entry("animation_sampling", HostEntryKind::Sampled, false);
}

pub(crate) fn note_longhand_input_freeze(reasons: u8) {
    note_stage_interleave("longhand_input_freeze");
    if mode() == Mode::Off || UPDATE_DEPTH.with(|depth| depth.get() == 0) {
        return;
    }
    let names = [
        "element_adjustment_facts",
        "monospace_recascade",
        "tree_counting_inputs",
        "custom_property_inheritance_walk",
        "custom_property_adapter",
        "font_length_resolution_context",
        "box_type_parent_display",
        "unused_bit_7",
    ];
    LONGHAND_INPUT_FREEZE_REASONS.with(|counts| {
        let mut counts = counts.borrow_mut();
        for (index, name) in names.into_iter().enumerate() {
            if reasons & (1 << index) != 0 {
                let count = counts.entry(name).or_default();
                *count = count.wrapping_add(1);
            }
        }
    });
}

fn write_report(report: &str) {
    match std::env::var_os("LIBWEB_SEAL_STYLE_STAGE_LOG") {
        Some(path) => {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(report.as_bytes());
            }
        }
        None => eprint!("{report}"),
    }
}

pub(crate) fn begin_update() {
    if mode() == Mode::Off {
        return;
    }
    UPDATE_DEPTH.with(|depth| depth.set(depth.get().checked_add(1).expect("style update depth overflowed")));
}

pub(crate) fn end_update() {
    if mode() == Mode::Off {
        return;
    }
    UPDATE_DEPTH.with(|depth| depth.set(depth.get().checked_sub(1).expect("unbalanced style update scope")));
}

/// Record a return to main-thread work before the complete style stage has finished.
pub(crate) fn note_stage_interleave(name: &'static str) {
    let mode = mode();
    if mode == Mode::Off || UPDATE_DEPTH.with(|depth| depth.get() == 0) {
        return;
    }
    STAGE_INTERLEAVES.with(|interleaves| {
        let mut interleaves = interleaves.borrow_mut();
        let count = interleaves.entry(name).or_default();
        *count = count.wrapping_add(1);
    });
    if name == "longhand_result_apply" {
        note_host_entry_applied();
    }
    let allowed = match name {
        "longhand_input_freeze" => {
            std::env::var("LIBWEB_SEAL_STYLE_STAGE_ALLOW_LONGHAND_INPUT_FREEZE").as_deref() == Ok("1")
        }
        "longhand_result_apply" => {
            std::env::var("LIBWEB_SEAL_STYLE_STAGE_ALLOW_LONGHAND_RESULT_APPLY").as_deref() == Ok("1")
        }
        _ => false,
    };
    assert!(
        mode != Mode::Abort || allowed,
        "style stage is sealed, but interleaves main-thread work for {name}"
    );
    if REPORTED.with(|reported| reported.borrow_mut().insert(name)) {
        write_report(&format!("STYLE SEAL: stage_interleave {name}\n"));
    }
}

/// Run the between-pass font batch.
///
/// This used to be a main-thread resource service: the batch entered the document's font computer,
/// which could resolve a pending web face and run its GC-visible callbacks. It is now a stage-local
/// computation over the published `@font-face` table and the process-wide font services, so it is
/// counted but not a crossing. The count stays because the batch is still a round *between* passes
/// rather than part of one, and that is what a single sealed pass would have to absorb.
pub(crate) fn between_pass_font_batch<T>(name: &'static str, requests: u64, batch: impl FnOnce() -> T) -> T {
    if mode() == Mode::Off || UPDATE_DEPTH.with(|depth| depth.get() == 0) {
        return batch();
    }
    BETWEEN_PASS_BATCHES.with(|batches| {
        let mut batches = batches.borrow_mut();
        let counts = batches.entry(name).or_default();
        counts.0 = counts.0.wrapping_add(requests);
        counts.1 = counts.1.wrapping_add(1);
    });
    batch()
}

/// Record that the style stage's font batch had to ask a font question on the document thread's
/// own IPC connection, because no render-side broker was installed to ask it on. The batch is
/// otherwise stage-local; this is the one crossing left in it, and the seal treats it like any
/// other host call - reported once, fatal in `abort`.
#[unsafe(no_mangle)]
pub extern "C" fn rust_style_seal_note_font_match_reached_document_thread() {
    note_host_call("font_match_document_connection");
}

/// Record one Rust-to-C++ call. Calls outside sealed computation are input preparation, output
/// commit, resource publication, or CSSOM mutation, but remain in the census.
pub(crate) fn note_host_call(callback: &'static str) {
    let mode = mode();
    if mode == Mode::Off {
        return;
    }
    let during_style = UPDATE_DEPTH.with(|depth| depth.get() != 0);
    COUNTS.with(|counts| {
        let mut counts = counts.borrow_mut();
        let counts = counts.entry(callback).or_default();
        counts.calls = counts.calls.wrapping_add(1);
        counts.during_style = counts.during_style.wrapping_add(u64::from(during_style));
    });
    if !during_style {
        return;
    }
    assert!(
        mode != Mode::Abort,
        "style stage is sealed, but a running update called {callback}()"
    );
    if REPORTED.with(|reported| reported.borrow_mut().insert(callback)) {
        write_report(&format!("STYLE SEAL: a running update called {callback}()\n"));
    }
}

/// Flush this thread's counts at an engine lifetime boundary. Taking the map makes repeated engine
/// destruction produce deltas rather than cumulative totals; the suite log can be summed directly.
pub(crate) fn flush_census() {
    if mode() == Mode::Off {
        return;
    }
    let mut counts = COUNTS.with(|counts| {
        std::mem::take(&mut *counts.borrow_mut())
            .into_iter()
            .collect::<Vec<_>>()
    });
    counts.sort_unstable_by_key(|(callback, _)| *callback);
    for (callback, counts) in counts {
        write_report(&format!(
            "STYLE SEAL COUNT: callback={callback} calls={} during_style={}\n",
            counts.calls, counts.during_style
        ));
    }
    let host_driven_rows = HOST_DRIVEN_ROWS.with(|rows| rows.replace(0));
    if host_driven_rows != 0 {
        write_report(&format!("STYLE SEAL COUNT: host_driven_rows: {host_driven_rows}\n"));
    }
    let sampled = HOST_SAMPLED_ANIMATION_ROWS.with(|rows| rows.replace(0));
    // Every way one element is entered from the host during an update, in one number. A row the
    // host computed and a retry the host asked for after applying an ancestor cost the same
    // thing: control on the host side between two elements. Relaxing a gate usually moves rows
    // from the first to the second, so only the total says whether the stage got closer to being
    // one function.
    let host_retries = HOST_RETRY_ENTRIES.with(|entries| entries.replace(0));
    let host_entries = host_driven_rows + host_retries + sampled;
    CURRENT_HOST_ENTRY.with(|current| *current.borrow_mut() = None);
    let mut causes = HOST_ENTRY_CAUSES.with(|causes| {
        std::mem::take(&mut *causes.borrow_mut())
            .into_iter()
            .collect::<Vec<_>>()
    });
    causes.sort_unstable_by(|(first, left), (second, right)| {
        right.entries.cmp(&left.entries).then_with(|| first.cmp(second))
    });
    let attributed: u64 = causes.iter().map(|(_, counts)| counts.entries).sum();
    for (key, counts) in &causes {
        write_report(&format!(
            "STYLE SEAL COUNT: host_entries cause={} kind={} cold={}: {} (applied {})\n",
            key.cause,
            key.kind.name(),
            u8::from(key.cold),
            counts.entries,
            counts.applied
        ));
    }
    if attributed != host_entries {
        // Log-only: every host entry is meant to pass through one of the three notes, so a
        // difference means a way in that the census does not know about.
        write_report(&format!(
            "STYLE SEAL COUNT: host_entries unattributed: {}\n",
            host_entries as i64 - attributed as i64
        ));
    }
    if host_entries != 0 {
        write_report(&format!(
            "STYLE SEAL COUNT: host_entries: {host_entries} (rows {host_driven_rows} + retries {host_retries} + sampled {sampled})\n"
        ));
    }
    if sampled != 0 {
        write_report(&format!("STYLE SEAL COUNT: host_sampled_animation_rows: {sampled}\n"));
    }
    let mut row_kinds = HOST_DRIVEN_ROW_KINDS.with(|counts| {
        std::mem::take(&mut *counts.borrow_mut())
            .into_iter()
            .collect::<Vec<_>>()
    });
    row_kinds.sort_unstable_by_key(|(kind, _)| *kind);
    for (kind, count) in row_kinds {
        write_report(&format!("STYLE SEAL COUNT: host_driven_rows kind={kind}: {count}\n"));
    }
    let mut interleaves = STAGE_INTERLEAVES.with(|interleaves| {
        std::mem::take(&mut *interleaves.borrow_mut())
            .into_iter()
            .collect::<Vec<_>>()
    });
    interleaves.sort_unstable_by_key(|(name, _)| *name);
    for (name, count) in interleaves {
        write_report(&format!("STYLE SEAL COUNT: stage_interleave {name}: {count}\n"));
    }
    let mut freeze_reasons = LONGHAND_INPUT_FREEZE_REASONS.with(|counts| {
        std::mem::take(&mut *counts.borrow_mut())
            .into_iter()
            .collect::<Vec<_>>()
    });
    freeze_reasons.sort_unstable_by_key(|(reason, _)| *reason);
    for (reason, count) in freeze_reasons {
        write_report(&format!(
            "STYLE SEAL COUNT: longhand_input_freeze reason={reason}: {count}\n"
        ));
    }
    let mut batches = BETWEEN_PASS_BATCHES.with(|batches| {
        std::mem::take(&mut *batches.borrow_mut())
            .into_iter()
            .collect::<Vec<_>>()
    });
    batches.sort_unstable_by_key(|(batch, _)| *batch);
    for (batch, (requests, rounds)) in batches {
        write_report(&format!(
            "STYLE SEAL COUNT: between_pass_batch {batch}: {requests} requests in {rounds} rounds\n"
        ));
    }
}
