/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Retiring what a frame in flight holds before the document thread destroys it.
//!
//! A frame the rendering update submits runs beside the document thread until the frame scheduler
//! takes it in (consume-commit), and tasks run meanwhile. The frame holds its documents' arenas,
//! and the compositor contexts consume-commit hands its compositor frames to. A task can navigate,
//! remove an iframe, close the window or reconnect the compositor, which destroys what the frame
//! holds. Destruction is retirement: it waits for the frame that holds the thing and takes it in
//! first, and it moves the thing to a new generation, so whatever the frame produced for the old
//! one is discarded rather than published. A frame that is discarded this way is retired, which is
//! counted apart from a frame that was dropped.

use super::host_tables::HostTables;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;

/// Why a document's render state is retired.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FfiRenderStateRetirement {
    /// The document is destroyed: its iframe was removed, or its window closed.
    DocumentDestroyed,
    /// The document stopped being its navigable's active document: a navigation replaced it.
    DocumentBecameInactive,
    /// The document is finalized. Nothing holds it then, since a frame keeps its documents alive.
    DocumentFinalized,
}

/// How often render state was retired, and what that cost the frames that held it.
#[repr(C)]
#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct FfiFrameRetirementCounters {
    pub documents_destroyed: u64,
    pub documents_became_inactive: u64,
    pub documents_finalized: u64,
    pub compositor_contexts_retired: u64,
    /// Retirements that found a frame in flight holding what they retired, and waited for it.
    pub frames_waited_for: u64,
    /// Frames whose result was discarded because what they were made for was retired meanwhile.
    pub frames_retired: u64,
}

thread_local! {
    // On the document thread, the compositor contexts the submitted frame hands its compositor
    // frames to once it is taken in. A frame holds its arenas as the stages it submitted.
    static HOLDS: RefCell<Vec<u64>> = const { RefCell::new(Vec::new()) };
    // On the document thread, the retirements so far.
    static COUNTERS: Cell<FfiFrameRetirementCounters> = const {
        Cell::new(FfiFrameRetirementCounters {
            documents_destroyed: 0,
            documents_became_inactive: 0,
            documents_finalized: 0,
            compositor_contexts_retired: 0,
            frames_waited_for: 0,
            frames_retired: 0,
        })
    };
}

fn count(update: impl FnOnce(&mut FfiFrameRetirementCounters)) {
    COUNTERS.with(|counters| {
        let mut value = counters.get();
        update(&mut value);
        counters.set(value);
    });
}

/// Retiring what the frame in flight holds first waits for that frame and takes it in, which runs
/// its consume-commit.
fn wait_for_frame_in_flight(arena: *mut c_void) {
    crate::stage_thread::join_frame_in_flight(arena);
    count(|counters| counters.frames_waited_for += 1);
}

/// The generation of the arena's render state. A frame takes it as it is submitted, and what the
/// frame produced is only published while the generation has not moved.
///
/// # Safety
///
/// `arena_handle` must be a live handle from `layout_arena_create`, on the document thread.
pub(crate) unsafe fn frame_generation(arena_handle: *mut c_void) -> u64 {
    // SAFETY: Guaranteed by the caller.
    unsafe { HostTables::beside_frame(arena_handle) }.frame_generation.get()
}

/// Whether a frame submitted at `generation` was made for render state retired since. If so, it is
/// counted as retired, and its caller discards it.
///
/// # Safety
///
/// As for [`frame_generation`].
pub(crate) unsafe fn frame_was_retired(arena_handle: *mut c_void, generation: u64) -> bool {
    // SAFETY: Guaranteed by the caller.
    let retired = unsafe { frame_generation(arena_handle) } != generation;
    if retired {
        count(|counters| counters.frames_retired += 1);
    }
    retired
}

/// Retires the document's render state before the document tears it down: waits for a frame in
/// flight that holds the arena, and moves the arena to a new generation, so nothing a frame made
/// for the old one is published.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_retire_render_state(arena: *mut c_void, reason: FfiRenderStateRetirement) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let held = crate::stage_thread::frame_in_flight_owns(arena);
    // Waiting would run the frame's consume-commit inside a garbage collection.
    assert!(
        !(held && reason == FfiRenderStateRetirement::DocumentFinalized),
        "a document was finalized while a frame in flight held its arena"
    );
    // The generation moves first, so the consume-commit the wait runs does not publish what the
    // frame recorded for the render state being torn down.
    // SAFETY: Guaranteed by the caller.
    let tables = unsafe { HostTables::beside_frame(arena) };
    tables.frame_generation.set(tables.frame_generation.get() + 1);
    if held {
        wait_for_frame_in_flight(arena);
    }
    count(|counters| match reason {
        FfiRenderStateRetirement::DocumentDestroyed => counters.documents_destroyed += 1,
        FfiRenderStateRetirement::DocumentBecameInactive => counters.documents_became_inactive += 1,
        FfiRenderStateRetirement::DocumentFinalized => counters.documents_finalized += 1,
    });
}

/// The submitted frame hands a compositor frame to the context `context_id` once it is taken in, so
/// it holds the context until [`rust_frame_release_compositor_context`] is called with the same id.
#[unsafe(no_mangle)]
pub extern "C" fn rust_frame_hold_compositor_context(context_id: u64) {
    HOLDS.with(|holds| holds.borrow_mut().push(context_id));
}

/// Ends the hold [`rust_frame_hold_compositor_context`] began.
#[unsafe(no_mangle)]
pub extern "C" fn rust_frame_release_compositor_context(context_id: u64) {
    HOLDS.with(|holds| {
        let mut holds = holds.borrow_mut();
        let position = holds
            .iter()
            .rposition(|held| *held == context_id)
            .expect("a frame hold ends that never began");
        holds.remove(position);
    });
}

/// Retires the compositor context `context_id` before the navigable destroys or replaces it: waits
/// for the frame in flight that holds it and takes it in, so its compositor frame is handed to the
/// context while the context is still there.
#[unsafe(no_mangle)]
pub extern "C" fn rust_retire_compositor_context(context_id: u64) {
    if HOLDS.with(|holds| holds.borrow().contains(&context_id)) && crate::stage_thread::has_frame_in_flight() {
        wait_for_frame_in_flight(std::ptr::null_mut());
    }
    count(|counters| counters.compositor_contexts_retired += 1);
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_frame_retirement_counters() -> FfiFrameRetirementCounters {
    COUNTERS.with(Cell::get)
}

#[unsafe(no_mangle)]
pub extern "C" fn rust_reset_frame_retirement_counters() {
    COUNTERS.with(|counters| counters.set(FfiFrameRetirementCounters::default()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_compositor_context_hold_ends_when_released() {
        rust_frame_hold_compositor_context(7);
        assert!(HOLDS.with(|holds| holds.borrow().contains(&7)));
        rust_frame_release_compositor_context(7);
        assert!(HOLDS.with(|holds| holds.borrow().is_empty()));
    }

    #[test]
    fn retiring_what_no_frame_holds_waits_for_nothing() {
        rust_reset_frame_retirement_counters();
        rust_retire_compositor_context(3);
        let counters = rust_frame_retirement_counters();
        assert_eq!(counters.compositor_contexts_retired, 1);
        assert_eq!(counters.frames_waited_for, 0);
    }
}
