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

use super::animations::{AnimationSlot, AnimationTimingRow};
use super::tree::StyleNodeID;
use std::cell::RefCell;
use std::collections::HashMap;
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
pub(crate) fn note_declined(input: &str) {
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

/// The rows the engine expects the host to publish for the CSS animations a plan it handed over
/// starts, by the engine, the element and the animation list, each with its place in the element's
/// `animation-name` list.
type ExpectedRows = HashMap<(usize, StyleNodeID, AnimationSlot), Vec<(u32, AnimationTimingRow)>>;

thread_local! {
    static EXPECTED_NEW_ANIMATION_ROWS: RefCell<ExpectedRows> = RefCell::new(HashMap::new());
}

/// Expect the host to publish these rows once it has created the animations a plan starts, which
/// the next whole-stack sample of the list checks.
pub(crate) fn expect_new_animation_rows(
    engine: usize,
    node: StyleNodeID,
    slot: AnimationSlot,
    rows: Vec<(u32, AnimationTimingRow)>,
) {
    EXPECTED_NEW_ANIMATION_ROWS.with_borrow_mut(|expected| expected.insert((engine, node, slot), rows));
}

/// Check the rows the host published for a list against the ones the engine expected it to
/// publish for the animations it started.
pub(crate) fn check_new_animation_rows(
    engine: usize,
    node: StyleNodeID,
    slot: AnimationSlot,
    published: &[AnimationTimingRow],
) {
    let Some(expected) = EXPECTED_NEW_ANIMATION_ROWS.with_borrow_mut(|expected| expected.remove(&(engine, node, slot)))
    else {
        return;
    };
    for (name_index, row) in expected {
        let found = published
            .iter()
            .find(|published| published.is_listed_css_animation(node, slot, name_index));
        match found {
            Some(published) if row.predicts(published) => note_agreed("new animation timing"),
            _ => note_difference("new animation timing", &|| {
                format!(
                    "node {} slot {slot} animation {name_index}: engine {row:?}, host {found:?}",
                    node.raw()
                )
            }),
        }
    }
}

/// What the engine's own sample of an element leaves for the overlay record: the table after the
/// animated box-type finalization, and the overlay. The host publishes its own right after it
/// samples, and the publication checks the two against each other.
pub(crate) struct EngineSampledStyle {
    pub(crate) table: *mut crate::css::computed_longhand_table::ComputedLonghandTable,
    pub(crate) overlay: *mut crate::css::animated_overlay::AnimatedOverlay,
}

impl Drop for EngineSampledStyle {
    fn drop(&mut self) {
        unsafe {
            crate::css::computed_longhand_table::rust_computed_longhand_table_release(self.table);
            crate::css::animated_overlay::rust_animated_overlay_free(self.overlay);
        }
    }
}

type ExpectedStyles = HashMap<(usize, StyleNodeID, u8), EngineSampledStyle>;

thread_local! {
    static EXPECTED_SAMPLED_STYLES: RefCell<ExpectedStyles> = RefCell::new(HashMap::new());
}

/// Keep what the engine's sample of `(node, pseudo_kind)` composed for the publication that
/// follows, or forget what an earlier sample left where the engine could not sample.
pub(crate) fn expect_sampled_style(
    engine: usize,
    node: StyleNodeID,
    pseudo_kind: u8,
    style: Option<EngineSampledStyle>,
) {
    EXPECTED_SAMPLED_STYLES.with_borrow_mut(|expected| match style {
        Some(style) => {
            expected.insert((engine, node, pseudo_kind), style);
        }
        None => {
            expected.remove(&(engine, node, pseudo_kind));
        }
    });
}

pub(crate) fn take_expected_sampled_style(
    engine: usize,
    node: StyleNodeID,
    pseudo_kind: u8,
) -> Option<EngineSampledStyle> {
    EXPECTED_SAMPLED_STYLES.with_borrow_mut(|expected| expected.remove(&(engine, node, pseudo_kind)))
}

/// What the engine's sample of each row its pass settled composed, kept for the host's sample of
/// the row once it installs it, by the engine and the element. The pass can run on another thread
/// than the host's sample.
struct SettledRowSamples(HashMap<(usize, StyleNodeID), crate::css::style_compute::SettledRowSample>);

// SAFETY: The samples own what they point to, and only the check reads them, under the lock.
unsafe impl Send for SettledRowSamples {}

static SETTLED_ROW_SAMPLES: std::sync::Mutex<Option<SettledRowSamples>> = std::sync::Mutex::new(None);

/// Keep what the engine's sample of a row its pass settled composed, or forget an earlier one where
/// the engine could not sample the row.
pub(crate) fn expect_settled_row_sample(
    engine: usize,
    node: StyleNodeID,
    sample: Option<crate::css::style_compute::SettledRowSample>,
) {
    let mut samples = SETTLED_ROW_SAMPLES.lock().expect("the check's lock is never poisoned");
    let samples = &mut samples.get_or_insert_with(|| SettledRowSamples(HashMap::new())).0;
    match sample {
        Some(sample) => {
            samples.insert((engine, node), sample);
        }
        None => {
            samples.remove(&(engine, node));
        }
    }
}

/// Look at what the engine's sample of the element's settled row composed; `keep` says whether it
/// stays for the check of the style the host finalizes.
pub(crate) fn with_settled_row_sample(
    engine: usize,
    node: StyleNodeID,
    check: impl FnOnce(&crate::css::style_compute::SettledRowSample) -> bool,
) {
    let mut samples = SETTLED_ROW_SAMPLES.lock().expect("the check's lock is never poisoned");
    let Some(samples) = samples.as_mut() else {
        return;
    };
    let Some(sample) = samples.0.get(&(engine, node)) else {
        return;
    };
    if !check(sample) {
        samples.0.remove(&(engine, node));
    }
}

pub(crate) fn take_settled_row_sample(
    engine: usize,
    node: StyleNodeID,
) -> Option<crate::css::style_compute::SettledRowSample> {
    SETTLED_ROW_SAMPLES
        .lock()
        .expect("the check's lock is never poisoned")
        .as_mut()?
        .0
        .remove(&(engine, node))
}
