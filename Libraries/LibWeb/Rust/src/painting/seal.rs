/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Sealed mode for the paint stage.
//!
//! A stage is finished when it reads only the output of the stage before it, so that it could run
//! on a thread of its own. The paint stage is finished when a running pass asks the document
//! nothing. This gate names each host call a running pass still makes, so what is left is
//! countable rather than a matter of review. It is the same gate the layout stage has in
//! [`crate::layout::seal`], and it is read the same way.
//!
//! `LIBWEB_SEAL_PAINT_STAGE` turns it on. Unset, nothing is checked and nothing is paid for.
//! Set, each distinct call site reports itself once per pass kind. Set to `abort`, the first one
//! is fatal. Reports go to standard error, or to the file `LIBWEB_SEAL_PAINT_STAGE_LOG` names,
//! since a test runner does not keep the render process's standard error.
//!
//! **The paint stage is sealed.** As of the commit that scoped a caret line search by identity,
//! the whole test suite runs under `LIBWEB_SEAL_PAINT_STAGE=abort` without a single report. What
//! follows is what the seal still permits, and why each of them is not a read of the document
//! made by a running pass.
//!
//! # The passes
//!
//! [`Pass`] names the render-side passes the seal watches. A host call made while one of them is
//! running is a violation; the same call made between passes is an input the stage was handed,
//! and the seal permits it. Passes nest - a pass restores its predecessor when it ends, and a
//! report names the innermost one - so a pass that means to call out has to end first, and has
//! to check what it leaves running when it does.
//!
//! # The allow-list
//!
//! What a sealed paint stage may still do, none of which this gate sees as a violation:
//!
//! **Outputs, after the pass.** The resource service the recording's publish hands fonts, image
//! frames and video sinks to - `FfiRecordingPublishCallbacks::add_font`, `add_image_frame` and
//! `add_video_sink`. Publish is a pass of its own so that what it resolves is still counted, but
//! handing the resource service a font is not a read of the document.
//!
//! **A nested vector image, recorded beside the publish that asked for it.**
//! `resolve_vector_image_display_list` lays out and records an SVG-as-image document, which is a
//! paint stage of that document rather than of this one, so
//! [`crate::painting::record::publish::publish_recording`] ends the publish around it. It is the
//! one entry on this list that still reads this document - to find the image the placeholder
//! names - and pre-recording the nested list when the image's decoded data changes is what would
//! end that.
//!
//! **Result sinks of C++ to Rust queries.** A query that answers through a callback appending to
//! a caller-owned collection runs with no pass in progress at all, so the predicate excludes
//! them without any of them having to say so.
//!
//! **Replay.** `FfiDisplayListReplayCallbacks`, the display list player: a stage of its own.
//!
//! **Debug output.** Dumps, traces and verification reports, the position the layout seal already
//! takes for its own dump path.
//!
//! **Inputs synced before a pass, never during one.** The recording inputs, the SVG paint
//! resource sync (`resolve_filter`, `resolve_paint_server`) and the paint fact pushes. Each one
//! still gets a [`note_host_call`], so a call that moved inside a pass would be reported; none
//! fires inside one today.
//!
//! Anything else a running pass asks the document is a regression. Add a [`note_host_call`]
//! beside any new host call rather than leaving it uncounted.

use std::cell::Cell;
use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::OnceLock;

/// A render-side pass, as the seal names it in a report.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Pass {
    /// No pass is running: a host call is an input, not a violation.
    None,
    /// Display list recording, `layout_arena_record_display_list`.
    Recording,
    /// The publish that hands a finished recording's resources to the host.
    RecordingPublish,
    /// The accumulated visual context build, and the rendering preparation around it.
    VisualContextUpdate,
    /// The scroll state refresh that re-derives the published snapshot.
    ScrollStateRefresh,
    /// The scrollable overflow recalculation that runs after a commit.
    ScrollableOverflow,
    /// A hit test or caret query against the published hit test list.
    HitTest,
}

impl Pass {
    fn name(self) -> &'static str {
        match self {
            Pass::None => "no pass",
            Pass::Recording => "recording",
            Pass::RecordingPublish => "the recording publish",
            Pass::VisualContextUpdate => "the visual context update",
            Pass::ScrollStateRefresh => "the scroll state refresh",
            Pass::ScrollableOverflow => "the scrollable overflow recalculation",
            Pass::HitTest => "a hit test",
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Off,
    Report,
    Abort,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("LIBWEB_SEAL_PAINT_STAGE").as_deref() {
        Err(_) | Ok("0") => Mode::Off,
        Ok("abort") => Mode::Abort,
        Ok(_) => Mode::Report,
    })
}

thread_local! {
    static CURRENT_PASS: Cell<Pass> = const { Cell::new(Pass::None) };
    static REPORTED: RefCell<HashSet<(&'static str, &'static str)>> = RefCell::new(HashSet::new());
}

/// Restores the pass that was running when this one began.
pub(crate) struct PassScope(Pass);

impl Drop for PassScope {
    fn drop(&mut self) {
        CURRENT_PASS.with(|current| current.set(self.0));
    }
}

/// Marks `pass` as running until the returned scope is dropped. Off, this is a thread-local
/// store and nothing else, so the callers need not be gated.
#[must_use]
pub(crate) fn enter(pass: Pass) -> PassScope {
    PassScope(CURRENT_PASS.with(|current| current.replace(pass)))
}

/// Records that the paint stage asked the document something named `callback`. A call made
/// while no pass is running is what the seal permits.
pub(crate) fn note_host_call(callback: &'static str) {
    let mode = mode();
    if mode == Mode::Off {
        return;
    }
    let pass = CURRENT_PASS.with(|current| current.get());
    if pass == Pass::None {
        return;
    }
    let pass = pass.name();
    assert!(
        mode != Mode::Abort,
        "paint stage is sealed, but {pass} called {callback}()"
    );
    let first_time = REPORTED.with(|reported| reported.borrow_mut().insert((callback, pass)));
    if !first_time {
        return;
    }
    let report = format!("PAINT SEAL: {pass} called {callback}()\n");
    match std::env::var_os("LIBWEB_SEAL_PAINT_STAGE_LOG") {
        Some(path) => {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(report.as_bytes());
            }
        }
        None => eprint!("{report}"),
    }
}
