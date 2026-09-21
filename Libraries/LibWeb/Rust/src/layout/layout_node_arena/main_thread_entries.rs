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

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_allocate(
    arena: *mut c_void,
    construction_facts: FfiNodeConstructionFacts,
) -> NodeSlotId {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (&mut *arena.cast::<LayoutNodeArena>()).allocate(construction_facts)
        })
    }
}

/// # Safety
///
/// The arena must remain valid for the duration of the call, and `root` must name a live node
/// in this arena that has no parent. Every C++-side detach preparation that walks the subtree
/// must already have run. Every shell in the subtree is destroyed before this returns.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_free_subtree(arena: *mut c_void, root: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on
    // the document thread.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            crate::layout::tree_mutation::free_subtree_and_hand_back(arena.cast::<LayoutNodeArena>(), root);
        });
    }
}

/// # Safety
///
/// The arena must remain valid for the duration of the call, and `node` must name a live node
/// in this arena. Every C++-side detach preparation that walks the subtree must already have
/// run. The node and every shell in its subtree are destroyed before this returns.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_detach_and_free_subtree(arena: *mut c_void, node: NodeSlotId) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on
    // the document thread.
    unsafe { paying_host_handbacks(&main_thread, arena, || detach_and_free_subtree(arena.cast(), node)) }
}

/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_pre_order_label_violation_count(arena: *mut c_void, root: NodeSlotId) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    let arena = unsafe { &*arena.cast::<LayoutNodeArena>() };
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    if arena.shell_if_live(&main_thread, root).is_null() {
        return 0;
    }
    let mut violation_count = 0u64;
    let mut previous_label: Option<u64> = None;
    arena.for_each_node_in_layout_subtree_in_pre_order(root, |node| {
        let label = arena.node_pre_order_label(node);
        if previous_label.is_some_and(|previous| label <= previous) {
            violation_count += 1;
        }
        previous_label = Some(label);
    });
    violation_count
}

/// # Safety
///
/// The arena must remain valid for the duration of the call. `id` may be
/// invalid or stale; null is returned in that case.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_node_shell_if_live(arena: *mut c_void, id: NodeSlotId) -> *mut c_void {
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.shell_if_live(&main_thread, id)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_node_link_shell(
    arena: *mut c_void,
    id: NodeSlotId,
    link: FfiNodeLink,
) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle(arena) }.node_link_shell(&main_thread, id, link)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_node_containing_block_shell_if_live(
    arena: *mut c_void,
    id: NodeSlotId,
) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle(arena) }.node_containing_block_shell_if_live(&main_thread, id)
}

/// The shell of the row the element or text node with `style_node` is bound to, materialised if
/// nothing has asked for it yet, or null if the node has no row.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bound_shell(arena: *mut c_void, style_node: u32) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return std::ptr::null_mut();
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    let arena = unsafe { &*arena.cast::<LayoutNodeArena>() };
    let row = arena.bound_row(style_node);
    if row.is_invalid() {
        return std::ptr::null_mut();
    }
    arena.node_shell(&main_thread, row)
}

/// The shell of the row the pseudo-element of kind `generated_for` on the element with
/// `style_node` is bound to, materialised if nothing has asked for it yet, or null.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bound_pseudo_element_shell(
    arena: *mut c_void,
    style_node: u32,
    generated_for: u8,
) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return std::ptr::null_mut();
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    let arena = unsafe { &*arena.cast::<LayoutNodeArena>() };
    let row = arena.bound_pseudo_element_row(style_node, generated_for);
    if row.is_invalid() {
        return std::ptr::null_mut();
    }
    arena.node_shell(&main_thread, row)
}

/// The shell of the viewport row the document is bound to, materialised if nothing has asked for
/// it yet, or null.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bound_viewport_shell(arena: *mut c_void) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above.
    let arena = unsafe { &*arena.cast::<LayoutNodeArena>() };
    let row = arena.bound_viewport_row();
    if row.is_invalid() {
        return std::ptr::null_mut();
    }
    arena.node_shell(&main_thread, row)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bind_row(arena: *mut c_void, id: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (&*arena.cast::<LayoutNodeArena>()).bind_row(id);
        });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_unbind_row(arena: *mut c_void, id: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (&*arena.cast::<LayoutNodeArena>()).unbind_row(id);
        });
    }
}

/// Visits the live shell of every row built for the same DOM node as `id`, that row included.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread; `visit`
/// is called synchronously with `context`.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_for_each_row_built_for_same_node(
    arena: *mut c_void,
    id: NodeSlotId,
    context: *mut c_void,
    visit: unsafe extern "C" fn(*mut c_void, *mut c_void),
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on
    // the document thread.
    let arena = unsafe { &*arena.cast::<LayoutNodeArena>() };
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // The ring is a column of links rather than a borrow, so the host may re-enter the arena
    // from `visit`. What it must not do is change which rows are built for the node.
    arena.for_each_row_built_for_same_node(id, |row| {
        let shell = arena.shell_if_live(&main_thread, row);
        if !shell.is_null() {
            // SAFETY: The host answers synchronously and does not free the shell.
            unsafe { visit(context, shell) };
        }
    });
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_set_style_node_of_rows_sharing_dom_node_with(
    arena: *mut c_void,
    id: NodeSlotId,
    style_node: u32,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (&*arena.cast::<LayoutNodeArena>())
                .set_style_node_of_rows_sharing_dom_node_with(id, StyleNodeID::from_raw(style_node));
        });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_set_style_node_of_generated_subtree(
    arena: *mut c_void,
    root: NodeSlotId,
    style_node: u32,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (&*arena.cast::<LayoutNodeArena>())
                .set_style_node_of_generated_subtree(root, StyleNodeID::from_raw(style_node));
        });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_prepare_node_for_detach(arena: *mut c_void, row: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { &*arena.cast::<LayoutNodeArena>() }.assert_owner_thread();
    // SAFETY: As above.
    unsafe { paying_host_handbacks(&main_thread, arena, || prepare_row_for_detach(arena, row)) }
}

/// Prepares every row in the layout subtree `root` heads for leaving the tree.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_prepare_subtree_for_detach(arena: *mut c_void, root: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { paying_host_handbacks(&main_thread, arena, || prepare_subtree_for_detach(arena, root)) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_forget_style_node(arena: *mut c_void, style_node: u32) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return;
    };
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (&*arena.cast::<LayoutNodeArena>()).forget_style_node(style_node);
        });
    }
}

/// The arena and record must be live on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_adopt_derived_node_style(arena: *mut c_void, node: NodeSlotId, record: u64) {
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let derived = arena.with_style_engine(|engine| {
        engine.pin_layout_style_record(record);
        DerivedStyleRecord {
            record,
            payloads: engine.style_record_payloads(record).unwrap().as_ptr().cast(),
        }
    });
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    arena.apply_reinherited_style_record(node, derived, ShellStyleChangeNotice::Now(&main_thread));
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_set_layout_display(arena: *mut c_void, node: NodeSlotId, display: u32) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    unsafe { LayoutNodeArena::from_handle(arena) }.update_layout_style(
        node,
        ShellStyleChangeNotice::Now(&main_thread),
        |style| {
            style.set_display(crate::css::display::FfiDisplay::from_raw(display));
        },
    );
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_reinherit_anonymous_descendants(arena: *mut c_void, node: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above.
    unsafe { &*arena.cast::<LayoutNodeArena>() }
        .reinherit_anonymous_descendants(node, ShellStyleChangeNotice::Now(&main_thread));
}

/// Visits every subtree root the last layout tree build rebuilt and left live, as the row's layout
/// node. Anonymous roots stand for no DOM node and are skipped; the host resolves the rest.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread, and `visit` must return synchronously
/// without entering the arena.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_for_each_pending_rebuilt_subtree_root(
    arena: *mut c_void,
    context: *mut c_void,
    visit: unsafe extern "C" fn(*mut c_void, *mut c_void),
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: As above; the roots are copied out so no borrow spans the callback.
    let roots = unsafe { &*arena.cast::<LayoutNodeArena>() }
        .pending_rebuilt_subtree_roots
        .borrow()
        .clone();
    for root in roots {
        // SAFETY: As above.
        let arena = unsafe { &*arena.cast::<LayoutNodeArena>() };
        if !arena.node_is_dom_backed(root) {
            continue;
        }
        // SAFETY: The callback receives a layout node the arena keeps alive.
        unsafe { visit(context, arena.node_shell(&main_thread, root)) };
    }
}

/// # Safety
///
/// `arena` must be a live handle with a registered layout host, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_sync_enrolled_content_for_layout(arena: *mut c_void) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    // SAFETY: Guaranteed by the entry point's contract.
    unsafe { sync_enrolled_content_for_layout(&main_thread, arena) }
}
