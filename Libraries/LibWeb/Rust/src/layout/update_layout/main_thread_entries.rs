/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The FFI entries of the parent module that mint the main thread capability. Only this module can
//! construct its marker, and the entries are private, so stage code in the parent module can neither
//! mint the capability nor call an entry that does.

use super::*;

pub(crate) struct MainThreadFfiEntry {
    _private: (),
}

const MAIN_THREAD_FFI_ENTRY: MainThreadFfiEntry = MainThreadFfiEntry { _private: () };

/// Ends the layout update and tells the document which web font faces its passes reached while
/// they wait on their load, so the document requests them.
///
/// # Safety
///
/// `arena` must be a live handle with a registered layout host, used on the document thread, with
/// a layout update running.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_end_update_layout(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe { crate::layout::HostTables::from_handle(arena) }.end_layout_update();
    // Only this document's stages' wants: another document's stage may be running beside it.
    let messages: Vec<_> = libgfx_rust::font::take_wanted_pending_faces(arena as u64)
        .into_iter()
        .map(
            |(pending_face, pending_face_has_been_retried)| crate::layout::commit::FfiCommitMessage {
                style_node: 0,
                other_style_node: 0,
                kind: crate::layout::commit::FfiCommitMessageKind::PendingFontFaceWanted,
                pending_face,
                pending_face_has_been_retried,
            },
        )
        .collect();
    if messages.is_empty() {
        return;
    }
    let host = crate::layout::formatting_context::LayoutHost::of(&main_thread);
    // SAFETY: The host keeps the document alive for this synchronous call.
    unsafe { host.deliver_commit_messages(&main_thread, &messages) };
}

/// Runs the document's layout update to a fixed point: style, then the layout tree build, then
/// either a partial relayout of the registered boundaries or a full pass, until nothing is
/// pending. The document-side steps run through the registered layout update host.
///
/// # Safety
///
/// `arena` must be a live handle with registered layout and layout update hosts, used on the
/// document thread between `layout_arena_begin_update_layout` and its end, and `inputs` must
/// remain valid for the call.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_update_layout(
    arena: *mut c_void,
    inputs: *const FfiLayoutUpdateInputs,
) -> FfiLayoutUpdateOutcome {
    assert!(!arena.is_null(), "layout node arena handle is null");
    assert!(!inputs.is_null());
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    abort_on_panic(|| {
        // SAFETY: Guaranteed by the entry point's contract.
        unsafe { update_layout(&main_thread, arena, &*inputs) }
    })
}

/// Whether the layout update the document thread is about to run, which may submit its full layout
/// pass if `may_submit_pass`, runs its first round's style in the flight it submits. If it does,
/// the style pass the document thread submits next is collected for the update, which it then
/// begins style for and submits ahead of the update, as the update's first round would have.
///
/// # Safety
///
/// `arena` must be a live handle with a registered layout host, used on the document thread
/// between `layout_arena_begin_update_layout` and the layout update.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_collect_style_pass_for_flight(arena: *mut c_void, may_submit_pass: bool) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let _main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    if !runs_style_in_flight(may_submit_pass) {
        return false;
    }
    crate::css::style::bridge::collect_next_style_pass_for_flight();
    true
}

/// Submits the flight the document's layout update readied (see
/// `FfiLayoutUpdateOutcome::FlightReady`), once the document has sealed what its recording reads.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread, whose `layout_arena_update_layout` just
/// answered with a flight ready.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_submit_prepared_flight(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    abort_on_panic(|| {
        // SAFETY: Guaranteed by the entry point's contract.
        unsafe { submit_prepared_flight(&main_thread, arena) }
    });
}

/// Ends the layout frame of the document `arena` names once the document thread has taken back the
/// frame in flight that ran its round, and ends the update.
/// The document thread runs it where it takes the frame back, ahead of anything else that reaches
/// the arena.
pub(super) fn finish_layout_frame_taken_back(arena: *mut c_void, frame: LayoutFrame) {
    // SAFETY: Only the take-back of the frame the update submitted calls this, on the document
    // thread, for the arena whose update is still running.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    abort_on_panic(|| {
        // SAFETY: As above.
        unsafe { finish_layout_frame(&main_thread, frame) }
    });
}

/// Hands the clock of the document a fresh layout frame for its ticks to lay out in on the
/// render side while the document thread idles (see `ClockLayoutFrame`), with the document as it
/// read itself into `round`.
///
/// # Safety
///
/// `arena` must be a live handle with a registered layout update host, used on the document thread
/// with no layout update running and no frame in flight. `round` must be valid, and so must the
/// selection it points to, if any.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_renew_clock_layout_frame(arena: *mut c_void, round: *const FfiLayoutRoundFacts) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    let round = unsafe { LayoutRoundFacts::from_ffi(&*round) };
    // SAFETY: As above.
    let frame = unsafe { make_clock_layout_frame(&main_thread, arena, round) };
    crate::clock_frames::set_clock_layout_frame(arena, frame);
}

/// Takes in the layout frame the document's clock ticks laid out in, if they did, and ends the layout
/// update the document began for it: pays what the rounds owe the document and applies their
/// messages, as a submitted pass's frame is taken in. Returns whether there was one; the document
/// then renews the clock's frame (`layout_arena_renew_clock_layout_frame`).
///
/// # Safety
///
/// `arena` must be a live handle with a registered layout update host, used on the document thread
/// after `layout_arena_begin_update_layout`, with the ticks over.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_take_in_clock_layout_frame(arena: *mut c_void) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(frame) = crate::clock_frames::take_laid_out_clock_layout_frame(arena) else {
        return false;
    };
    let shown_on_render_side = crate::clock_frames::presented_since_adoption(arena);
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    abort_on_panic(|| {
        // SAFETY: As above.
        unsafe { take_in_clock_layout_frame(&main_thread, frame, shown_on_render_side) }
    });
    true
}

/// Whether the document's clock ticks laid out in its layout frame since the document last took it in.
#[unsafe(no_mangle)]
extern "C" fn layout_arena_clock_layout_frame_laid_out(arena: *mut c_void) -> bool {
    crate::clock_frames::clock_layout_frame_laid_out(arena)
}

/// Ends the layout frame a flight ran and recorded the document after, as
/// [`finish_layout_frame_taken_back`] does. Answers whether the recording stands.
pub(super) fn finish_layout_frame_recorded_in_flight(arena: *mut c_void, frame: LayoutFrame) -> bool {
    // SAFETY: Only the take-back of the flight the update submitted calls this, on the document
    // thread, for the arena whose update is still running.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    abort_on_panic(|| {
        // SAFETY: As above.
        unsafe { super::finish_layout_frame_recorded_in_flight(&main_thread, frame) }
    })
}
