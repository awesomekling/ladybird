/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The paint preparation passes a document thread waits for outside a rendering update: the render owner runs each
//! with the arena of the document's render state, and answers the waiting thread with what the pass leaves for it.

use std::ffi::c_void;

use crate::layout::node_data::NodeSlotId;
use crate::layout::{ArenaHandle, LayoutNodeArena};
use crate::stage_thread::{CallerWaits, OwnerReplyTo};

/// A paint preparation pass over a document's render state.
pub(crate) enum PaintPass {
    /// Prepares the root background and the scrollable overflow, and finishes preparing the document for rendering
    /// unless the new overflow clamped scroll offsets, which the document thread stores first; given whether a visual
    /// context update is pending.
    PrepareForRendering(Pass<bool, crate::painting::ffi::RenderingPreparation>),
    /// Finishes preparing the document for rendering, given whether the root background source changed and whether a
    /// visual context update is pending.
    FinishRenderingPreparation(Pass<(bool, bool), crate::painting::ffi::FfiRenderingPreparationOutcome>),
    /// Updates the accumulated visual contexts of the tree under a viewport.
    AccumulatedVisualContexts(Pass<NodeSlotId, crate::painting::host::FfiVisualContextUpdateOutcome>),
    /// Updates the visual viewport transform of the visual context tree, and answers whether there is a tree.
    VisualViewportTransform(Pass<(), bool>),
    /// Refreshes the scroll state, where something invalidated it or the pass is forced, and answers with its
    /// snapshot.
    ScrollState(Pass<(), Option<Vec<libgfx_rust::FloatPoint>>>),
    /// Measures the scrollable overflow a commit or a writer left unmeasured.
    ScrollableOverflow(Pass<(), ()>),
    /// Runs the function a pass on the waiting thread's stack holds ([`run_held_pass`]), and leaves its answer there.
    Held(Pass<CallerWaits<*mut dyn RunHeldPass>, ()>),
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
            run_and_publish(unsafe { &mut *state }.arena_mut(), body, arguments)
        });
    }
}

impl PaintPass {
    fn run(self, state: impl FnOnce() -> *mut ArenaHandle) {
        match self {
            Self::PrepareForRendering(pass) => pass.run(state),
            Self::FinishRenderingPreparation(pass) => pass.run(state),
            Self::AccumulatedVisualContexts(pass) => pass.run(state),
            Self::VisualViewportTransform(pass) => pass.run(state),
            Self::ScrollState(pass) => pass.run(state),
            Self::ScrollableOverflow(pass) => pass.run(state),
            Self::Held(pass) => pass.run(state),
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
    pub(crate) fn run(self, owner: &crate::render_owner::Owner, found: impl FnOnce() -> Option<*mut ArenaHandle>) {
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
            found.unwrap_or_else(|| unsafe { ArenaHandle::held_by_waiting_thread(owner, sent) })
        });
    }
}

/// Runs the pass `body` with `arguments` over `arena`, then publishes the rows it may have changed, which the document
/// thread reads once it has the answer.
fn run_and_publish<A, R>(arena: &mut LayoutNodeArena, body: fn(&mut LayoutNodeArena, A) -> R, arguments: A) -> R {
    let answer = body(arena, arguments);
    arena.publish_rows();
    answer
}

/// Runs the paint pass `body` with `arguments` over the render state of `arena`'s document on the render owner, sent as
/// the kind of pass `kind` makes, and waits for its answer. Where the owner does not run it (the calling thread is the
/// owner, an arena of no document's render state, or a test holds the run it would queue behind), it runs right here.
pub(crate) fn run_paint_pass<A, R>(
    arena: &mut LayoutNodeArena,
    kind: fn(Pass<A, R>) -> PaintPass,
    body: fn(&mut LayoutNodeArena, A) -> R,
    arguments: A,
) -> R {
    if !arena.document().is_valid() {
        return run_and_publish(arena, body, arguments);
    }
    // SAFETY: The arena is the first field of its handle, and this thread holds it for the pass.
    unsafe { run_paint_pass_of(std::ptr::from_mut(arena).cast(), kind, body, arguments) }
}

/// Runs the paint pass `body` with `arguments` over the render state of the document whose arena the calling document
/// thread names as `arena`, as [`run_paint_pass`] does.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, on the document thread.
pub(crate) unsafe fn run_paint_pass_of<A, R>(
    arena: *mut c_void,
    kind: fn(Pass<A, R>) -> PaintPass,
    body: fn(&mut LayoutNodeArena, A) -> R,
    arguments: A,
) -> R {
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { ArenaHandle::document_of(arena) };
    let handle = arena;
    if !document.is_valid() {
        // SAFETY: Guaranteed by the caller; the owner holds no state of an arena of no document.
        let state = crate::render_owner::do_owner_work_here(|owner| unsafe {
            &mut *ArenaHandle::held_by_waiting_thread(owner, handle)
        });
        return run_and_publish(state.arena_mut(), body, arguments);
    }
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
        // The owner does not run the pass: the calling thread does its work.
        |owner| {
            // SAFETY: Guaranteed by the caller; this thread does the owner's work, and nothing else reaches the state.
            let state = unsafe { &mut *ArenaHandle::held_by_waiting_thread(owner, handle) };
            run_and_publish(state.arena_mut(), body, arguments.take().expect("a pass runs once"))
        },
    );
    // A pass that panicked on the owner panics here, as a stage the document thread waits for does.
    outcome.unwrap_or_else(|payload| std::panic::resume_unwind(payload))
}

/// A pass by a function over the arena, which the document thread holds on its stack while it waits: the function, its
/// arguments, and once the owner has run it, its answer.
struct HeldPass<A, R> {
    arguments: Option<A>,
    body: fn(&mut LayoutNodeArena, A) -> R,
    answer: Option<R>,
}

/// A [`HeldPass`] of any arguments and answer, as the owner runs it.
pub(crate) trait RunHeldPass {
    fn run(&mut self, arena: &mut LayoutNodeArena);
}

impl<A, R> RunHeldPass for HeldPass<A, R> {
    fn run(&mut self, arena: &mut LayoutNodeArena) {
        if let Some(arguments) = self.arguments.take() {
            self.answer = Some((self.body)(arena, arguments));
        }
    }
}

/// Runs `body` with `arguments` over the render state of the document whose arena the calling document thread names
/// as `arena`, on the render owner, and waits for its answer, as [`run_paint_pass_of`] does. The arguments and the
/// answer stay on the calling thread's stack, which lends them to the owner for the pass: they may hold what the
/// calling thread owns, which the owner reaches only while the thread waits.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, on the document thread.
pub(crate) unsafe fn run_held_pass<A: 'static, R: Default + 'static>(
    arena: *mut c_void,
    arguments: A,
    body: fn(&mut LayoutNodeArena, A) -> R,
) -> R {
    let mut held = HeldPass {
        arguments: Some(arguments),
        body,
        answer: None,
    };
    let reference: *mut dyn RunHeldPass = &mut held;
    // SAFETY: Guaranteed by the caller. The pass stays on this thread's stack, which waits for it.
    unsafe {
        run_paint_pass_of(
            arena,
            PaintPass::Held,
            // SAFETY: The document thread waits for the pass, with the pass it holds live.
            |arena, held| (*held.into_inner()).run(arena),
            CallerWaits::new(reference),
        );
    }
    // A pass that panicked panicked here too, so the owner ran this one.
    debug_assert!(held.answer.is_some(), "the owner ran the pass");
    held.answer.unwrap_or_default()
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
