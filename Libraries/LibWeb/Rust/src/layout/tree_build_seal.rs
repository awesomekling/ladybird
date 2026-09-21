/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Sealed mode for the layout tree build stage.
//!
//! A stage is finished when it reads only the output of the stage before it, so that it could run
//! on a thread of its own. The tree build is finished when building a document's boxes asks the
//! document nothing. This gate names each route out of the build into C++, so what is left is
//! countable rather than a matter of review.
//!
//! The stage is [`super::tree_builder::rust_build_layout_tree`]'s walk, the stage function the
//! layout entry runs before the pass. `LIBWEB_SEAL_TREE_BUILD_STAGE` turns the gate on. Unset or
//! `0`, it costs only the mode check and nothing is recorded. `1` reports each distinct route once
//! and keeps a census; `abort` makes the first route out of the build fatal. Reports and census
//! totals go to standard error, or to the file `LIBWEB_SEAL_TREE_BUILD_STAGE_LOG` names, since a
//! test runner does not keep the render process's standard error.
//!
//! **The stage is sealed, and the compiler holds it to that**: the whole suite passes with the
//! gate in `abort`, and nothing the walk can reach is able to call the host. The walk is not handed
//! the main-thread capability, and every host call the arena and the tree builder can make takes
//! it: the tree builder's callbacks, the shell factory, and paying what the arena owes the host.
//! What the build owes the host - the shells whose construction tells the host something, a box's
//! style resources, a generated image's provider, what the build found out, and what it let go of -
//! it queues, and its entry pays once the walk has returned.
//!
//! What the build lets go of is the arena's handbacks: the boxes nodes gained or lost, the shells,
//! owned image providers and image observer sets of the rows it freed, the resets of rows whose
//! paint state went with them, and a kept box's new style. The operations the walk shares with the
//! DOM mutation entries only queue these; a main-thread entry pays the queue as its change returns,
//! and the build returns its queue as part of its output.
//!
//! What the whole suite takes, as of the commit that emptied the allow-list. None of it is taken
//! while a build runs; the handbacks are counted where they are paid, which for a build is after
//! the walk:
//!
//! | route | calls | during build |
//! |---|---|---|
//! | `notify_box_presence` | 7918548 | 0 |
//! | `paintable_row_reset` | 3828173 | 0 |
//! | `layout_node_shell_factory` | 2807113 | 0 |
//! | `layout_node_shell_destroy` | 2803597 | 0 |
//! | `viewport_propagation_facts` | 180865 | 0 |
//! | `shell_style_changed` | 68029 | 0 |
//! | `build_replaced_content_facts` | 36973 | 0 |
//! | `attach_style_resources` | 13029 | 0 |
//! | `deliver_commit_messages` | 4028 | 0 |
//! | `image_observers_destroy` | 3030 | 0 |
//! | `owned_image_provider_destroy` | 96 | 0 |
//! | `owned_image_provider_notify_detach` | 96 | 0 |
//! | `attach_generated_image` | 20 | 0 |
//!
//! A build owes a shell only to a row whose shell's construction tells the host something about
//! it; every other row gets one when something first asks for its box. Of the 4.6 million rows a
//! suite run stamps, 2.8 million ever get a shell, most of them asked for by the layout tree dumps
//! the tests print; of the rows `treebuild.html` stamps, fewer than three in ten do.
//!
//! # No allow-list
//!
//! Every call out of a running build is a route to retire, and there are none left. The shared
//! resource services, the Unicode segmenters and category lookups the build uses for text, are the
//! purity exception a render thread keeps: thread-safe services, not reads of the document. They
//! are not recorded at all, the same as in [`super::seal`].
//!
//! Add a `note_host_call` beside any new call out of the build rather than leaving it uncounted.

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
    during_build: u64,
}

fn mode() -> Mode {
    static MODE: OnceLock<Mode> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var("LIBWEB_SEAL_TREE_BUILD_STAGE").as_deref() {
        Err(_) | Ok("0") => Mode::Off,
        Ok("abort") => Mode::Abort,
        Ok(_) => Mode::Report,
    })
}

thread_local! {
    static BUILD_DEPTH: Cell<u32> = const { Cell::new(0) };
    static REPORTED: RefCell<HashSet<&'static str>> = RefCell::new(HashSet::new());
    static COUNTS: RefCell<HashMap<&'static str, Counts>> = RefCell::new(HashMap::new());
}

fn write_report(report: &str) {
    match std::env::var_os("LIBWEB_SEAL_TREE_BUILD_STAGE_LOG") {
        Some(path) => {
            use std::io::Write;
            if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
                let _ = file.write_all(report.as_bytes());
            }
        }
        None => eprint!("{report}"),
    }
}

/// Opens the tree build scope. A build can nest inside another document's build, since building a
/// navigable container's box can build the content document's tree, so the scope counts.
pub(crate) fn begin_build() {
    if mode() == Mode::Off {
        return;
    }
    BUILD_DEPTH.with(|depth| depth.set(depth.get().checked_add(1).expect("tree build depth overflowed")));
}

pub(crate) fn end_build() {
    if mode() == Mode::Off {
        return;
    }
    BUILD_DEPTH.with(|depth| depth.set(depth.get().checked_sub(1).expect("unbalanced tree build scope")));
}

/// Records one route out of the layout tree build into C++, named by `callback`. Calls made
/// outside a build are the layout stage's or a DOM mutation's and stay in the census as context.
pub(crate) fn note_host_call(callback: &'static str) {
    let mode = mode();
    if mode == Mode::Off {
        return;
    }
    let during_build = BUILD_DEPTH.with(|depth| depth.get() != 0);
    COUNTS.with(|counts| {
        let counts = &mut *counts.borrow_mut();
        let counts = counts.entry(callback).or_default();
        counts.calls = counts.calls.wrapping_add(1);
        counts.during_build = counts.during_build.wrapping_add(u64::from(during_build));
    });
    if !during_build {
        return;
    }
    assert!(
        mode != Mode::Abort,
        "tree build stage is sealed, but a running build called {callback}()"
    );
    if REPORTED.with(|reported| reported.borrow_mut().insert(callback)) {
        write_report(&format!("TREE BUILD SEAL: a running build called {callback}()\n"));
    }
}

/// Flushes this thread's counts at an arena lifetime boundary. Taking the map makes a process that
/// outlives many documents produce deltas rather than cumulative totals, so a suite's log can be
/// summed line by line.
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
            "TREE BUILD SEAL COUNT: callback={callback} calls={} during_build={}\n",
            counts.calls, counts.during_build,
        ));
    }
}
