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
//! The stage is the whole of [`super::tree_builder::rust_build_layout_tree`], the walk the layout
//! entry runs before the pass. `LIBWEB_SEAL_TREE_BUILD_STAGE` turns the gate on. Unset or `0`, it
//! costs only the mode check and nothing is recorded. `1` reports each distinct route once and
//! keeps a census; `abort` makes the first route out of the build fatal. Reports and census totals
//! go to standard error, or to the file `LIBWEB_SEAL_TREE_BUILD_STAGE_LOG` names, since a test
//! runner does not keep the render process's standard error.
//!
//! **The stage is not sealed yet.** The census a `1` run leaves is the to-do list: every line with
//! a non-zero `during_build` is a route that has to go before the stage can be sealed. The count
//! matters as much as the name, because a route a full suite takes twice is a different problem
//! from one it takes a hundred thousand times.
//!
//! What the whole suite takes, by `during_build` count, as of the commit that stamped the marker a
//! list-item pseudo-element nests. Two of these are not tree builder callback slots at all, which
//! is what the gate was for:
//!
//! | route | during build | note |
//! |---|---|---|
//! | `layout_node_shell_factory` | 4045613 | **not a slot**: the arena materialising a shell |
//! | `shell_style_changed` | 81661 | **not a slot**: a row's style reaching its shell |
//! | `attach_style_resources` | 11890 | slot: every box's style resources, wherever it was built |
//! | `pseudo.create_content_replacement_box` | 61 | slot: a box that owns the image it replaces its contents with |
//! | `pseudo.create_content_item` | 20 | slot: what is left is the generated image cases |
//!
//! `pseudo.create_layout_node` is gone: the build stamps every pseudo-element box but the content
//! replacement, which is what the slot is named for now. `pseudo.create_nested_list_marker` is gone
//! too, and with it the last style record the build asked to have computed while it ran.
//!
//! What the allow-list costs, by the same count, so that the debt is a number rather than a word:
//! `notify_box_presence` 11382371, `layout_node_shell_destroy` 2575221, `paintable_row_reset`
//! 567016, `image_observers_destroy` 2047, `deliver_commit_messages` 1028,
//! `owned_image_provider_notify_detach` 2, `owned_image_provider_destroy` 2.
//!
//! Two of these counts are not stable: `layout_node_shell_factory` and `notify_box_presence` swing
//! by about a tenth between runs of the same binary, because a handful of tests do a variable
//! amount of build work. The rest hold to under a percent, so read those two as an order of
//! magnitude rather than a number to compare against.
//!
//! # Where the shells come from
//!
//! A census keyed by the entry point each materialisation was reached through, over the same
//! suite: 3986003 of the 3986151 a build makes are reached from the render side itself - the
//! build's own `node_shell` assertions - and 148 from the host's `node_shell_if_live`. Every
//! materialisation the host reaches through `node_link_shell` (207877) and
//! `containing_block_shell_if_live` (147) happens outside a build. So the build materialises
//! almost four million shells for itself, and the main side asks for a fifth of a million.
//!
//! `build_replaced_content_facts` and `viewport_propagation_facts` are counted too and have never
//! been taken during a build: they belong to the layout entry that follows it.
//!
//! # The allow-list
//!
//! Three kinds of call are permitted and are marked `allowed` in the census.
//!
//! **Outputs.** The render side telling the document what it decided is not a read of the
//! document:
//!
//! - `deliver_commit_messages` - what a finished build found out, delivered once the walk is over.
//! - `notify_box_presence` - a row telling the document that it gained or lost a box.
//! - `paintable_row_reset` - the paint state that rides with a box going away.
//!
//! **Render-side objects the host owns the memory of.** The arena holds the shells, owned image
//! providers and image observer sets as opaque pointers, and the host frees them; a detaching
//! row's provider is told its row has gone the same way. These hand memory back or clear a
//! pointer into the arena, rather than asking the document anything, and they are the same upcalls
//! the sealed layout stage already permits outside a pass. An owned image provider is always the
//! one a box's generated content is built around, whose detach notification clears two pointers.
//!
//! **The shared resource services.** Fonts, text shaping and the Unicode services are the purity
//! exception a render thread keeps: thread-safe services, not reads of the document. They are not
//! recorded at all, the same as in [`super::seal`].
//!
//! Anything else the build asks the document is a route that has to be retired. Add a
//! `note_host_call` beside any new call out of the build rather than leaving it uncounted.

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

/// The routes a sealed tree build would still be allowed to take. Kept beside the census rather
/// than at the call sites, so that one list answers "what does the seal permit".
fn route_is_allowed(callback: &'static str) -> bool {
    matches!(
        callback,
        "deliver_commit_messages"
            | "notify_box_presence"
            | "paintable_row_reset"
            | "layout_node_shell_destroy"
            | "owned_image_provider_destroy"
            | "owned_image_provider_notify_detach"
            | "image_observers_destroy"
    )
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
    if !during_build || route_is_allowed(callback) {
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
            "TREE BUILD SEAL COUNT: callback={callback} calls={} during_build={} allowed={}\n",
            counts.calls,
            counts.during_build,
            route_is_allowed(callback),
        ));
    }
}
