/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! A cross-check of what the engine would sample against what the host samples.
//!
//! The engine is taking over sampling an element's animations from the host, one input at a time.
//! Before a step switches an input over, the engine derives it beside the host and this check
//! compares the two wherever the host samples. `LIBWEB_ENGINE_SAMPLE_CHECK` turns it on: unset or
//! `0`, it costs only the mode check; `1` reports every difference and every place the engine
//! declines to answer; `abort` makes a difference fatal. Reports go to stderr, or are appended to
//! the file named by `LIBWEB_ENGINE_SAMPLE_CHECK_LOG`.

use std::io::Write;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Off,
    Report,
    Abort,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("LIBWEB_ENGINE_SAMPLE_CHECK").as_deref() {
        Err(_) | Ok("0") => Mode::Off,
        Ok("abort") => Mode::Abort,
        Ok(_) => Mode::Report,
    })
}

/// Whether the host's samples are checked at all, which a caller asks before deriving anything.
pub(crate) fn is_checking() -> bool {
    mode() != Mode::Off
}

fn report(line: &str) {
    static LOG: OnceLock<Option<std::path::PathBuf>> = OnceLock::new();
    let log = LOG.get_or_init(|| std::env::var_os("LIBWEB_ENGINE_SAMPLE_CHECK_LOG").map(Into::into));
    match log {
        Some(path) => {
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(format!("{line}\n").as_bytes());
            }
        }
        None => eprintln!("{line}"),
    }
}

/// The engine could not answer for this input, so there is nothing to compare.
pub(crate) fn note_declined(input: &'static str) {
    if mode() == Mode::Off {
        return;
    }
    report(&format!("engine-sample-check declined {input}"));
}

/// The engine answered for this input exactly as the host did.
pub(crate) fn note_agreed(input: &'static str) {
    if mode() == Mode::Off {
        return;
    }
    report(&format!("engine-sample-check agreed {input}"));
}

/// The engine answered for this input differently from the host.
pub(crate) fn note_difference(input: &'static str, detail: &dyn Fn() -> String) {
    match mode() {
        Mode::Off => {}
        Mode::Report => report(&format!("engine-sample-check DIFFERS {input}: {}", detail())),
        Mode::Abort => {
            let detail = detail();
            report(&format!("engine-sample-check DIFFERS {input}: {detail}"));
            panic!("the engine's sample differs from the host's ({input}): {detail}");
        }
    }
}
