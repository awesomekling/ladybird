/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Sealed mode for the style stage.
//!
//! The style stage is the complete C++-orchestrated update bracketed by
//! `rust_style_ffi_complete_style_update_begin` and `rust_style_ffi_complete_style_update_end`.
//! `LIBWEB_SEAL_STYLE_STAGE` turns this gate on. Unset or `0`, it costs only the mode check. `1`
//! reports each callback site once; `abort` makes the first callback fatal. Reports and census
//! totals go to stderr or to the file named by `LIBWEB_SEAL_STYLE_STAGE_LOG`.
//!
//! # The allow-list
//!
//! `computed_properties.did_mutate_post_compute` is allowed: its complete C++ implementation only
//! invalidates cached property data in the transaction-private `ComputedStyleWorkingSet` after
//! Rust has mutated that working set's longhand table. It does not read or write DOM, CSSOM, GC,
//! document, or shared cache state, and the working set has exclusive ownership during the call.
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

thread_local! {
    static UPDATE_DEPTH: Cell<u32> = const { Cell::new(0) };
    static REPORTED: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
    static COUNTS: RefCell<HashMap<&'static str, Counts>> = RefCell::new(HashMap::new());
    static BETWEEN_PASS_SERVICES: RefCell<HashMap<&'static str, (u64, u64)>> = RefCell::new(HashMap::new());
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

/// Run a main-thread resource service between sealed evaluation passes.
///
/// Unlike an allow-listed callback, this remains a visible dependency of the style update. Abort
/// mode therefore rejects it unless the service-specific escape hatch is set. The escape hatch is
/// useful for proving that every other crossing is gone while the resource service remains.
pub(crate) fn between_pass_font_service<T>(requests: u64, service: impl FnOnce() -> T) -> T {
    let mode = mode();
    if mode == Mode::Off {
        return service();
    }
    if UPDATE_DEPTH.with(|depth| depth.get() == 0) {
        note_host_call("resolve_font");
        return service();
    }
    let suspended_depth = UPDATE_DEPTH.with(|depth| depth.replace(0));
    debug_assert_ne!(suspended_depth, 0);
    BETWEEN_PASS_SERVICES.with(|services| {
        let mut services = services.borrow_mut();
        let counts = services.entry("resolve_font").or_default();
        counts.0 = counts.0.wrapping_add(requests);
        counts.1 = counts.1.wrapping_add(1);
    });
    let allowed = std::env::var("LIBWEB_SEAL_STYLE_STAGE_ALLOW_FONT_SERVICE").as_deref() == Ok("1");
    assert!(
        mode != Mode::Abort || allowed,
        "style stage is sealed, but requires the between-pass resolve_font service"
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

/// Record one Rust-to-C++ call. Calls outside the complete style update are part of input
/// publication or CSSOM mutation, not the style stage, but remain in the census.
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
    if callback == "computed_properties.did_mutate_post_compute" {
        // ComputedStyleWorkingSet::did_apply_style_finalization_from_rust() only updates the
        // exclusively owned working set's derived-property cache after Rust mutates its table.
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
