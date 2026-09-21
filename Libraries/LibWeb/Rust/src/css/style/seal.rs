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
//! A font cache miss is not the shared font resource service: the installed resolver can
//! synchronously enter the font loader and resolve a pending web face, including its GC-visible
//! callbacks. Likewise, callbacks that prepare C++ longhand state or report computed results are
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
    static BETWEEN_PASS_SERVICES: RefCell<HashMap<&'static str, (u64, u64)>> = RefCell::new(HashMap::new());
    static HOST_DRIVEN_ROWS: Cell<u64> = const { Cell::new(0) };
    static HOST_SAMPLED_ANIMATION_ROWS: Cell<u64> = const { Cell::new(0) };
    static HOST_DRIVEN_ROW_KINDS: RefCell<HashMap<&'static str, u64>> = RefCell::new(HashMap::new());
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

/// Run a main-thread resource service between sealed evaluation passes.
///
/// Unlike an allow-listed callback, this remains a visible dependency of the style update. Abort
/// mode therefore rejects it unless the service-specific escape hatch is set. The escape hatch is
/// useful for proving that every other crossing is gone while the resource service remains.
pub(crate) fn between_pass_font_service<T>(name: &'static str, requests: u64, service: impl FnOnce() -> T) -> T {
    let mode = mode();
    if mode == Mode::Off {
        return service();
    }
    if UPDATE_DEPTH.with(|depth| depth.get() == 0) {
        note_host_call(name);
        return service();
    }
    let suspended_depth = UPDATE_DEPTH.with(|depth| depth.replace(0));
    debug_assert_ne!(suspended_depth, 0);
    BETWEEN_PASS_SERVICES.with(|services| {
        let mut services = services.borrow_mut();
        let counts = services.entry(name).or_default();
        counts.0 = counts.0.wrapping_add(requests);
        counts.1 = counts.1.wrapping_add(1);
    });
    let allowed = std::env::var("LIBWEB_SEAL_STYLE_STAGE_ALLOW_FONT_SERVICE").as_deref() == Ok("1");
    assert!(
        mode != Mode::Abort || allowed,
        "style stage is sealed, but requires the between-pass {name} service"
    );
    let result = service();
    UPDATE_DEPTH.with(|depth| {
        assert_eq!(
            depth.get(),
            0,
            "unbalanced style update scope in a between-pass service"
        );
        depth.set(suspended_depth);
    });
    result
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
    let mut services = BETWEEN_PASS_SERVICES.with(|services| {
        std::mem::take(&mut *services.borrow_mut())
            .into_iter()
            .collect::<Vec<_>>()
    });
    services.sort_unstable_by_key(|(service, _)| *service);
    for (service, (requests, rounds)) in services {
        write_report(&format!(
            "STYLE SEAL COUNT: between_pass_service {service}: {requests} requests in {rounds} rounds\n"
        ));
    }
}
