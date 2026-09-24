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

/// Detaches a top-layer element's layout placement and clears every stale projected subtree.
///
/// # Safety
///
/// The callback table, arena, and element must remain valid for the duration of the call.
#[unsafe(no_mangle)]
unsafe extern "C" fn rust_detach_top_layer_element_layout_subtree(arena: *mut c_void, style_node: u32) {
    assert!(!arena.is_null());
    // SAFETY: The entry point's contract puts this call on the document thread.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: Guaranteed by the entry point's contract.
    unsafe {
        super::layout_node_arena::paying_host_handbacks(&main_thread, arena, || {
            detach_top_layer_element_layout_subtree(arena.cast(), style_node);
        });
    }
}

/// Detaches what is left of a node's boxes as the node leaves the document, while its identity
/// still names them: its synthetic pseudo-elements' boxes, subtree and all, the paint state of its
/// own box, and its box's top layer placement, which is a viewport child rather than part of the
/// parent's box subtree, so the parent's rebuild would never detach it. The rows are found by
/// identity, so no shell is made for any of this.
///
/// # Safety
///
/// The arena must remain valid for the duration of the call, which must be made on the document
/// thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn rust_detach_remaining_layout_rows_for_removal(arena: *mut c_void, style_node: u32) {
    assert!(!arena.is_null());
    // SAFETY: The entry point's contract puts this call on the document thread.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: Guaranteed by the entry point's contract.
    unsafe {
        super::layout_node_arena::paying_host_handbacks(&main_thread, arena, || {
            detach_remaining_layout_rows_for_removal(arena.cast(), style_node);
        });
    }
}

/// Pays the host half of a finished layout tree build walk, which `walk` holds and this takes:
/// what the walk let go of, what it found out, and the shells and style resources its new rows are
/// owed. Answers with the build's outcome.
///
/// # Safety
///
/// The callback table and arena must remain valid for the duration of the call, which must be made
/// on the document thread, and `walk` must point to an `Option<LayoutTreeBuildWalk>` holding the
/// walk of this arena.
#[unsafe(no_mangle)]
unsafe extern "C" fn rust_pay_layout_tree_build(
    callbacks: *const FfiDomTreeBuilderCallbacks,
    arena: *mut c_void,
    walk: *mut c_void,
) -> FfiLayoutTreeBuildOutcome {
    assert!(!callbacks.is_null());
    assert!(!walk.is_null());
    // SAFETY: The entry point's contract puts this call on the document thread.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: Guaranteed by the entry point's contract.
    let callbacks = host_callbacks::TreeBuilderHostCallbacks::new(unsafe { &*callbacks });
    let host = dom_tree_builder_host(arena);
    // SAFETY: Guaranteed by the entry point's contract.
    let LayoutTreeBuildWalk(TreeBuildStageOutput {
        outcome,
        reports,
        handbacks,
    }) = unsafe { &mut *walk.cast::<Option<LayoutTreeBuildWalk>>() }
        .take()
        .expect("a layout tree build walk is paid once");

    let layout_host = host.layout();
    let arena = layout_host.arena();
    // What the walk let go of goes back to the host first, as it would have while the walk ran:
    // the boxes nodes gained or lost, and the host-owned objects of the rows it freed.
    arena.pay_tree_build_handbacks(&main_thread, handbacks);
    // What the build found out goes to the document now that the walk is complete and nothing can
    // clear a DOM update flag again, in the order the build found it out.
    if !reports.is_empty() {
        super::tree_build_seal::note_host_call("deliver_commit_messages");
        // SAFETY: The document outlives the build, and no arena borrow is held here.
        unsafe {
            crate::layout::LayoutHost::of(&main_thread).deliver_commit_messages(&main_thread, &reports);
        };
    }
    for (row, owed) in arena.take_rows_owed_to_host() {
        match owed {
            OwedToHost::Shell => {
                arena.node_shell(&main_thread, row);
            }
            OwedToHost::StyleResources {
                owns_content_replacement_image,
            } => {
                // SAFETY: The row is live, and every row a build owes style resources for is a
                // NodeWithStyle.
                unsafe { callbacks.attach_style_resources(&main_thread, row, owns_content_replacement_image) };
            }
            OwedToHost::GeneratedImage {
                generator,
                pseudo_element,
                item,
                pseudo_element_box,
            } => {
                // SAFETY: The row is a live image box, and the pseudo-element box it was built in
                // outlives it.
                unsafe {
                    callbacks.attach_generated_image(
                        &main_thread,
                        row,
                        generator.raw(),
                        pseudo_element,
                        item,
                        pseudo_element_box,
                    );
                };
            }
        }
    }
    outcome
}
