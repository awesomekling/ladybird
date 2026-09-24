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
//! `engine_call` counts the engine entry points the host calls while an update runs, apart from
//! taking the style transaction itself. Nothing computes styles on the host any more, but the
//! host still walks the transaction's answers and asks the engine about each row it applies;
//! each such call is a round trip a single sealed pass would have to absorb. The count is only
//! reported: none of these calls is a violation.
//!
//! A font cache miss is no longer a host service. The installed resolver answers from the
//! document's published `@font-face` table and the process-wide font services, reads no document
//! and holds no pointer to one, so `between_pass_batch resolve_font` is a stage-local computation
//! the stage's own thread performs. It is still counted, because it is a round *between* passes
//! that a single sealed pass would have to absorb.
//!
//! UTF-16 fly-string releases are deferred by the complete-update scope and drained after it.
//! CSSOM rule-mutation notifications and rule-compilation visitors run outside a style update.
//! They are counted when instrumented, but are not seal violations. Debug/dump callbacks would be
//! allowed only outside an update; none exists in the audited callback set.

use std::cell::RefCell;
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

/// The seal's view of the style updates a document thread runs: whether one is open, and the
/// census it keeps. It belongs to the document thread, and a stage run carries it to the stage
/// thread and back (see [`take_state`]), so the part of an update that runs there is sealed and
/// counted as the rest is.
#[derive(Default)]
pub(crate) struct SealState {
    update_depth: u32,
    reported: HashSet<&'static str>,
    reported_refusals: HashSet<(&'static str, bool)>,
    counts: HashMap<&'static str, Counts>,
    between_pass_batches: HashMap<&'static str, (u64, u64)>,
    host_entry_causes: HashMap<HostEntryKey, u64>,
    engine_calls: HashMap<&'static str, u64>,
}

thread_local! {
    static STATE: RefCell<SealState> = RefCell::new(SealState::default());
}

fn update_is_running() -> bool {
    STATE.with_borrow(|state| state.update_depth != 0)
}

/// Take this thread's seal state, for a stage run to carry to the stage thread.
pub(crate) fn take_state() -> SealState {
    STATE.with_borrow_mut(std::mem::take)
}

/// Install seal state a stage run carried here.
pub(crate) fn install_state(state: SealState) {
    STATE.with_borrow_mut(|current| *current = state);
}

/// What one host entry is: the reason the engine sent this element to the host, which way in it
/// took, and whether it had a record already.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct HostEntryKey {
    pub(crate) cause: &'static str,
    pub(crate) kind: HostEntryKind,
    pub(crate) cold: bool,
}

/// The ways one element is entered from the host during an update.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) enum HostEntryKind {
    Row,
    Sampled,
    /// A row the engine refused and nothing computed: the element keeps the record it has.
    Refused,
}

impl HostEntryKind {
    fn name(self) -> &'static str {
        match self {
            Self::Row => "row",
            Self::Sampled => "sampled",
            Self::Refused => "refused",
        }
    }
}

/// Record one host entry under the reason the engine declined the element, so the census ranks
/// what reaches the host rather than what the engine attempted. An attempt that declines for a
/// class the host then skips costs nothing; only an entry does.
///
/// A refused row is a violation wherever it happens: the element is left with a record the engine
/// did not answer for, so `abort` makes it fatal and `1` reports each cause once.
pub(crate) fn note_host_entry(cause: &'static str, kind: HostEntryKind, cold: bool) {
    let mode = mode();
    if mode == Mode::Off {
        return;
    }
    if kind == HostEntryKind::Refused {
        assert!(
            mode != Mode::Abort,
            "style stage is sealed, but the engine refused a row ({cause}, cold: {cold})"
        );
        if STATE.with_borrow_mut(|state| state.reported_refusals.insert((cause, cold))) {
            write_report(&format!("STYLE SEAL: refused_row {cause} cold={cold}\n"));
        }
    }
    if !update_is_running() {
        return;
    }
    let key = HostEntryKey { cause, kind, cold };
    STATE.with_borrow_mut(|state| *state.host_entry_causes.entry(key).or_default() += 1);
}

/// Record one engine entry point the host called. Only calls made while an update runs are
/// counted, and none is fatal: the census ranks the round trips left between host and engine.
pub(crate) fn note_engine_call(entry: &'static str) {
    if mode() == Mode::Off || !update_is_running() {
        return;
    }
    STATE.with_borrow_mut(|state| *state.engine_calls.entry(entry).or_default() += 1);
}

/// Report how often the engine declined to compute a record itself, by the reason it recorded.
/// A host-driven row inside a published batch is a row one of these declined.
///
/// These numbers belong to one engine and are reported when that engine is destroyed, so they
/// are not a run total: an engine that outlives reporting never prints its own.
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
    STATE.with_borrow_mut(|state| {
        state.update_depth = state
            .update_depth
            .checked_add(1)
            .expect("style update depth overflowed");
    });
}

pub(crate) fn end_update() {
    if mode() == Mode::Off {
        return;
    }
    let finished = STATE.with_borrow_mut(|state| {
        state.update_depth = state
            .update_depth
            .checked_sub(1)
            .expect("unbalanced style update scope");
        state.update_depth == 0
    });
    if finished && mode() == Mode::Report {
        flush_census();
    }
}

/// Run a between-pass input batch.
///
/// This used to be a main-thread resource service: the batch entered the document's font computer,
/// which could resolve a pending web face and run its GC-visible callbacks. It is now a stage-local
/// computation over the published `@font-face` table and the process-wide font services, so it is
/// counted but not a crossing. The count stays because the batch is still a round *between* passes
/// rather than part of one, and that is what a single sealed pass would have to absorb.
pub(crate) fn between_pass_input_batch<T>(name: &'static str, requests: u64, batch: impl FnOnce() -> T) -> T {
    if mode() == Mode::Off || !update_is_running() {
        return batch();
    }
    STATE.with_borrow_mut(|state| {
        let counts = state.between_pass_batches.entry(name).or_default();
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
    let during_style = update_is_running();
    STATE.with_borrow_mut(|state| {
        let counts = state.counts.entry(callback).or_default();
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
    if STATE.with_borrow_mut(|state| state.reported.insert(callback)) {
        write_report(&format!("STYLE SEAL: a running update called {callback}()\n"));
    }
}

/// Flush this thread's counts at an engine lifetime boundary. Taking the map makes repeated engine
/// destruction produce deltas rather than cumulative totals; the suite log can be summed directly.
pub(crate) fn flush_census() {
    if mode() == Mode::Off {
        return;
    }
    let (counts, causes, engine_calls, batches) = STATE.with_borrow_mut(|state| {
        (
            std::mem::take(&mut state.counts),
            std::mem::take(&mut state.host_entry_causes),
            std::mem::take(&mut state.engine_calls),
            std::mem::take(&mut state.between_pass_batches),
        )
    });
    let mut counts = counts.into_iter().collect::<Vec<_>>();
    counts.sort_unstable_by_key(|(callback, _)| *callback);
    for (callback, counts) in counts {
        write_report(&format!(
            "STYLE SEAL COUNT: callback={callback} calls={} during_style={}\n",
            counts.calls, counts.during_style
        ));
    }
    let mut causes = causes.into_iter().collect::<Vec<_>>();
    causes.sort_unstable_by(|(first, left), (second, right)| right.cmp(left).then_with(|| first.cmp(second)));
    for (key, entries) in &causes {
        write_report(&format!(
            "STYLE SEAL COUNT: host_entries cause={} kind={} cold={}: {entries}\n",
            key.cause,
            key.kind.name(),
            u8::from(key.cold),
        ));
    }
    let mut engine_calls = engine_calls.into_iter().collect::<Vec<_>>();
    engine_calls.sort_unstable_by(|(first, left), (second, right)| right.cmp(left).then_with(|| first.cmp(second)));
    let total_engine_calls: u64 = engine_calls.iter().map(|(_, calls)| calls).sum();
    for (entry, calls) in engine_calls {
        write_report(&format!("STYLE SEAL COUNT: engine_call={entry} during_style={calls}\n"));
    }
    if total_engine_calls != 0 {
        write_report(&format!("STYLE SEAL COUNT: engine_calls: {total_engine_calls}\n"));
    }
    let mut batches = batches.into_iter().collect::<Vec<_>>();
    batches.sort_unstable_by_key(|(batch, _)| *batch);
    for (batch, (requests, rounds)) in batches {
        write_report(&format!(
            "STYLE SEAL COUNT: between_pass_batch {batch}: {requests} requests in {rounds} rounds\n"
        ));
    }
}

// The host's style-update boundary is reached even when a test exits before its document's
// engine is destroyed. Flush the per-thread census there so focused tests are measurable.
#[unsafe(no_mangle)]
extern "C" fn rust_style_seal_flush_census_for_update() {
    flush_census();
}
