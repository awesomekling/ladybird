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
    unsafe { LayoutNodeArena::from_handle(arena) }.end_update_layout();
    let messages: Vec<_> = libgfx_rust::font::take_wanted_pending_faces()
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

/// Runs the rest of the layout frame of the document `arena` names once the document thread has
/// taken back the frame in flight that ran its full layout pass `laid_out`, and ends the update.
/// The document thread runs it where it takes the frame back, ahead of anything else that reaches
/// the arena.
pub(super) fn finish_layout_frame_taken_back(arena: *mut c_void, frame: LayoutFrame, laid_out: LaidOutPass) {
    // SAFETY: Only the take-back of the frame the update submitted calls this, on the document
    // thread, for the arena whose update is still running.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    abort_on_panic(|| {
        // SAFETY: As above.
        unsafe { finish_layout_frame(&main_thread, frame, laid_out) }
    });
}
