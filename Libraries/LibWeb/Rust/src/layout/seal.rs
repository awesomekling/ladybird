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
//! **The layout stage is sealed.** As of the commit that retired `compute_svg_path`, the whole
//! test suite runs under `LIBWEB_SEAL_LAYOUT_STAGE=abort` without a single report, and
//! [`super::layout_pass::LayoutPass`] no longer borrows the host table at all. What follows is
//! what the seal still permits, and why each of them is not a read of the document.
//!
//! Full and partial layout now also have compiler-enforced boundaries: their shared input is
//! `Sync`, their output is `Send`, and their runners receive neither [`crate::stage::MainThread`]
//! nor a host callback table. Host calls therefore cannot be added to a runner through the
//! supported interfaces. This runtime seal remains as redundant diagnostics until all pipeline
//! stages use the same static boundary and the coordinator removes the seals together.
//!
//! # The allow-list
//!
//! **Outputs.** The render side tells the document what it decided. These run from commit, after
//! the pass has ended and the arena's mutable borrow has been released:
//!
//! - `deliver_commit_messages` - the messages a finished commit leaves, in the order it made
//!   them. `FfiLayoutHostCallbacks::deliver_commit_messages`, sent from
//!   `commit::CommitNotifications::notify_host`.
//! - box presence - a row telling the document that it gained or lost a box, and the paintable
//!   row resets that ride with it. Commit queues them, and `commit_entry_pass` pays them with the
//!   main thread capability through `LayoutNodeArena::finish_paying_host_handbacks` once commit
//!   has returned.
//!
//! **Inputs synced before a pass, never during one.** These are host calls, and they are the
//! two `note_host_call` sites that remain; each passes `layout_pass_is_running()`, so a call
//! from inside a pass would still be reported. Neither has ever fired inside one:
//!
//! - `build_replaced_content_facts` - the intrinsic size of an enrolled replaced box, synced into
//!   the arena ahead of the pass.
//! - `viewport_propagation_facts` - the document element and body facts the viewport propagation
//!   decides from, read once at the layout entry before the pass begins.
//!
//! **The shared resource service.** Fonts and text shaping (`libgfx_rust::text_layout`, the
//! thread-local shaping cache) are the one purity exception a render thread is meant to keep.
//! They are not host calls and the seal does not see them.
//!
//! A pass picks fonts out of `libgfx_rust::font::FrozenFontList`, a snapshot the document built
//! before the pass began: no caches to fill, no faces to resolve, and `Send + Sync` without an
//! `unsafe impl`. A code point no listed family covers goes to `Gfx::system_fallback_font`, a
//! process-wide memo whose answer depends on the installed font set and nothing else. A face
//! still on its `font-display` timeline is not resolved here at all: the frozen entry already
//! carries which period it is in, and the pass leaves the face's number behind for
//! `Gfx::request_wanted_pending_faces()` to request once the pass has ended.
//!
//! DEBT: The document's own `Gfx::FontCascadeList` is still reachable from Rust, because canvas
//! and the font-relative length code use it, and a call to it would pass this seal unseen: it is
//! a LibGfx call, not a host callback. Nothing in a stage makes one today.
//!
//! Anything else a running pass asks the document is a regression. Add a `note_host_call` beside
//! any new host call rather than leaving it uncounted.

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
