/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Sealed mode for the layout stage.
//!
//! A stage is finished when it reads only the output of the stage before it, so that it could run
//! on a thread of its own. Layout is finished when a running pass asks the document nothing. This
//! gate names each host call a running pass still makes, so what is left is countable rather than
//! a matter of review.
//!
//! `LIBWEB_SEAL_LAYOUT_STAGE` turns it on. Unset, nothing is checked and nothing is paid for.
//! Set, each distinct call site reports itself once. Set to `abort`, the first one is fatal.
//! Reports go to standard error, or to the file `LIBWEB_SEAL_LAYOUT_STAGE_LOG` names, since a
//! test runner does not keep the render process's standard error.
//!
//! The seal covers the host tables the arena holds, except what the render side tells the
//! document: the commit messages a finished pass delivers and the box presence a bound or
//! unbound row reports. Those are outputs, not reads of the document, and a sealed stage still
//! sends them.

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Off,
    Report,
    Abort,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("LIBWEB_SEAL_LAYOUT_STAGE").as_deref() {
        Err(_) | Ok("0") => Mode::Off,
        Ok("abort") => Mode::Abort,
        Ok(_) => Mode::Report,
    })
}

thread_local! {
    static REPORTED: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
}

/// Records that the layout stage asked the document something named `callback`. Callers pass
/// whether a layout pass is running; a call from outside one is what the seal permits.
pub(crate) fn note_host_call(a_layout_pass_is_running: bool, callback: &'static str) {
    let mode = mode();
    if mode == Mode::Off || !a_layout_pass_is_running {
        return;
    }
    assert!(
        mode != Mode::Abort,
        "layout stage is sealed, but a running pass called {callback}()"
    );
    let first_time = REPORTED.with(|reported| reported.borrow_mut().insert(callback));
    if !first_time {
        return;
    }
    let report = format!("LAYOUT SEAL: a running pass called {callback}()\n");
    match std::env::var_os("LIBWEB_SEAL_LAYOUT_STAGE_LOG") {
        Some(path) => {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(report.as_bytes());
            }
        }
        None => eprint!("{report}"),
    }
}
