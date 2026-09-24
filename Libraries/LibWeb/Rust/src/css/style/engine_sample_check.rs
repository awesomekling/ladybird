/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! A report of where the engine samples an element's animations itself, and where it declines to
//! and the host's inputs sample the element instead.
//!
//! `LIBWEB_ENGINE_SAMPLE_CHECK` turns it on: unset or `0`, it costs only the mode check; anything
//! else reports every sample the engine took over and every place it declined. Reports go to
//! stderr, or are appended to the file named by `LIBWEB_ENGINE_SAMPLE_CHECK_LOG`.

use std::io::Write;
use std::sync::OnceLock;

fn is_reporting() -> bool {
    static MODE: OnceLock<bool> = OnceLock::new();
    *MODE.get_or_init(|| !matches!(std::env::var("LIBWEB_ENGINE_SAMPLE_CHECK").as_deref(), Err(_) | Ok("0")))
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

/// The engine could not answer for this input, and the host's inputs sample the element.
pub(crate) fn note_declined(input: &str) {
    if !is_reporting() {
        return;
    }
    report(&format!("engine-sample-check declined {input}"));
}

/// The engine did something the host used to, which the report counts.
pub(crate) fn note_taken(what: &'static str) {
    if !is_reporting() {
        return;
    }
    report(&format!("engine-sample-check took {what}"));
}
