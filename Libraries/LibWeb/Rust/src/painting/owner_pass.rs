/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The paint preparation passes a document thread waits for outside a rendering update: the render owner runs each
//! with the arena of the document's render state, and answers the waiting thread with what the pass leaves for it.

use std::ffi::c_void;

use crate::css::css_pixels::CssPixelPoint;
use crate::layout::node_data::NodeSlotId;
use crate::layout::{ArenaHandle, LayoutNodeArena};
use crate::stage_thread::{CallerWaits, OwnerReplyTo};

/// A paint preparation pass over a document's render state.
pub(crate) enum PaintPass {
    /// Prepares the root background and the scrollable overflow, and answers whether the root background source
    /// changed, with the scroll offsets the new overflow clamped.
    RootBackgroundAndOverflow(Pass<(), (bool, Vec<(NodeSlotId, CssPixelPoint)>)>),
    /// Finishes preparing the document for rendering, given whether the root background source changed and whether a
    /// visual context update is pending.
    FinishRenderingPreparation(Pass<(bool, bool), crate::painting::ffi::FfiRenderingPreparationOutcome>),
    /// Updates the accumulated visual contexts of the tree under a viewport.
    AccumulatedVisualContexts(Pass<NodeSlotId, crate::painting::host::FfiVisualContextUpdateOutcome>),
    /// Updates the visual viewport transform of the visual context tree, and answers whether there is a tree.
    VisualViewportTransform(Pass<(), bool>),
    /// Refreshes the scroll state, where something invalidated it or the pass is forced, and answers with its
    /// snapshot.
    ScrollState(Pass<bool, Option<Vec<libgfx_rust::FloatPoint>>>),
    /// Measures the scrollable overflow a commit or a writer left unmeasured.
    ScrollableOverflow(Pass<(), ()>),
    /// Builds what a hit-test query derives from the hit-test list: its spatial indexes, its caret lines, or both.
    HitTestList(Pass<(bool, bool), ()>),
    /// Records the display list of the frame a recording slot on the waiting thread's stack holds, and leaves the
    /// recording in the slot.
    Recording(Pass<CallerWaits<*mut c_void>, ()>),
}

/// A pass taking `A` and answering `R`: its arguments, what it runs, and where the owner answers it.
pub(crate) struct Pass<A, R> {
    arguments: A,
    /// What the pass runs. The owner reaches each pass's work only through the passes it is sent, so what sends one
    /// pass (a unit test's arena) does not link the work of every other.
    body: fn(&mut LayoutNodeArena, A) -> R,
    reply: OwnerReplyTo<R>,
}

impl<A, R> Pass<A, R> {
    /// On the owner: runs the pass with the arena `arena` finds, with the faces it wants filed under the arena's
    /// document, and answers the waiting document thread. The arena is found inside the answer, so that a panic there
    /// answers the thread too.
    fn run(self, state: impl FnOnce() -> *mut ArenaHandle) {
        let Self { arguments, body, reply } = self;
        reply.answer(|| {
            let state = state();
            let _wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(state as u64);
            // SAFETY: The document thread waits for the pass, and reaches nothing of the state meanwhile.
            body(unsafe { &mut *state }.arena_mut(), arguments)
        });
    }
}

impl PaintPass {
    fn run(self, state: impl FnOnce() -> *mut ArenaHandle) {
        match self {
            Self::RootBackgroundAndOverflow(pass) => pass.run(state),
            Self::FinishRenderingPreparation(pass) => pass.run(state),
            Self::AccumulatedVisualContexts(pass) => pass.run(state),
            Self::VisualViewportTransform(pass) => pass.run(state),
            Self::ScrollState(pass) => pass.run(state),
            Self::ScrollableOverflow(pass) => pass.run(state),
            Self::HitTestList(pass) => pass.run(state),
            Self::Recording(pass) => pass.run(state),
        }
    }
}

/// A paint pass the owner runs for a document thread, which waits for it with the arena of the document it sent it
/// for.
pub(crate) struct OwnerPaintPass {
    arena: CallerWaits<*mut c_void>,
    pass: PaintPass,
}

impl OwnerPaintPass {
    /// Runs the pass on the owner, with the arena of the render state `found` finds for the pass's document. Where the
    /// owner holds none (a bug of the sender's), the pass runs with the arena the document thread sent.
    pub(crate) fn run(self, found: impl FnOnce() -> Option<*mut ArenaHandle>) {
        let Self { arena: sent, pass } = self;
        let sent = sent.into_inner();
        pass.run(|| {
            let found = found();
            debug_assert_eq!(
                found.map(<*mut ArenaHandle>::cast::<c_void>),
                Some(sent),
                "a paint pass runs with its document's arena"
            );
            // SAFETY: The document thread waits for the pass, and the arena it sent is its document's.
            found.unwrap_or_else(|| unsafe { ArenaHandle::held_by_waiting_thread(sent) })
        });
    }
}

/// Runs the paint pass `body` with `arguments` over the render state of `arena`'s document on the render owner, sent as
/// the kind of pass `kind` makes, and waits for its answer. Where the owner does not run it (no Rendering thread, the
/// calling thread is the owner, an arena of no document's render state, or a test holds the run it would queue
/// behind), it runs right here.
pub(crate) fn run_paint_pass<A, R>(
    arena: &mut LayoutNodeArena,
    kind: fn(Pass<A, R>) -> PaintPass,
    body: fn(&mut LayoutNodeArena, A) -> R,
    arguments: A,
) -> R {
    let document = arena.document();
    if !document.is_valid() {
        return body(arena, arguments);
    }
    let handle = std::ptr::from_mut(arena).cast::<c_void>();
    let arguments = std::cell::Cell::new(Some(arguments));
    let outcome = crate::stage_thread::wait_for_owner(
        |reply| crate::render_owner::ToOwner::Paint {
            document,
            pass: Box::new(OwnerPaintPass {
                // SAFETY: This thread waits for the pass, and reaches the arena only once it has the answer.
                arena: unsafe { CallerWaits::new(handle) },
                pass: kind(Pass {
                    arguments: arguments.take().expect("a pass is sent once"),
                    body,
                    reply,
                }),
            }),
        },
        // The arena is the caller's, which the owner does not reach: it runs no pass of it.
        || body(arena, arguments.take().expect("a pass runs once")),
    );
    // A pass that panicked on the owner panics here, as a stage the document thread waits for does.
    outcome.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
}

#[cfg(test)]
impl OwnerPaintPass {
    /// The pass `body` of the arena `arena` names, as a document thread sends it, answering at `reply`.
    pub(crate) fn new_for_test(
        arena: *mut c_void,
        body: fn(&mut LayoutNodeArena, ()) -> (),
        reply: OwnerReplyTo<()>,
    ) -> Self {
        Self {
            // SAFETY: The test waits for the pass.
            arena: unsafe { CallerWaits::new(arena) },
            pass: PaintPass::ScrollableOverflow(Pass {
                arguments: (),
                body,
                reply,
            }),
        }
    }
}
