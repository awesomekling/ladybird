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
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
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
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on
    // the document thread.
    unsafe {
        (*arena.cast::<LayoutNodeArena>()).release_published_paintable_rows_for_main_side_write();
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
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on
    // the document thread.
    unsafe {
        (*arena.cast::<LayoutNodeArena>()).release_published_paintable_rows_for_main_side_write();
        paying_host_handbacks(&main_thread, arena, || detach_and_free_subtree(arena.cast(), node))
    }
}

/// # Safety
///
/// The arena must remain valid for the duration of the call.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_pre_order_label_violation_count(arena: *mut c_void, root: NodeSlotId) -> u64 {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
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
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle_beside_recording(arena) }.shell_if_live(&main_thread, id)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_node_link_shell(
    arena: *mut c_void,
    id: NodeSlotId,
    link: FfiNodeLink,
) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle_beside_recording(arena) }.node_link_shell(&main_thread, id, link)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_node_containing_block_shell_if_live(
    arena: *mut c_void,
    id: NodeSlotId,
) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call.
    unsafe { LayoutNodeArena::from_handle_beside_recording(arena) }
        .node_containing_block_shell_if_live(&main_thread, id)
}

/// The shell of the row the element or text node with `style_node` is bound to, materialised if
/// nothing has asked for it yet, or null if the node has no row.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bound_shell(arena: *mut c_void, style_node: u32) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return std::ptr::null_mut();
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread. The shell and the row it is bound to are
    // nothing a recording writes.
    let arena = unsafe { LayoutNodeArena::from_handle_beside_recording(arena) };
    let row = arena.bound_row(style_node);
    if row.is_invalid() {
        return std::ptr::null_mut();
    }
    arena.node_shell(&main_thread, row)
}

/// The element the tree build last saw as the shadow-including parent of the element with
/// `style_node`, which is the host when the element's parent is a shadow root, or 0 for none.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_shadow_including_parent_element(arena: *mut c_void, style_node: u32) -> u32 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let _main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return 0;
    };
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call. The ancestry is
    // nothing a recording writes.
    unsafe { LayoutNodeArena::from_handle_beside_recording(arena) }
        .shadow_including_parent(style_node)
        .element
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
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return std::ptr::null_mut();
    };
    // SAFETY: The C++ wrapper keeps the arena alive for this call and
    // serializes all access on the document thread. The shell and the row it is bound to are
    // nothing a recording writes.
    let arena = unsafe { LayoutNodeArena::from_handle_beside_recording(arena) };
    let row = arena.bound_pseudo_element_row(style_node, generated_for);
    if row.is_invalid() {
        return std::ptr::null_mut();
    }
    arena.node_shell(&main_thread, row)
}

/// A row the host names by its slot, with the shell the host made for it, if it made one.
#[repr(C)]
pub struct FfiBoundRow {
    /// The row, or an invalid slot if there is none.
    pub slot: NodeSlotId,
    /// The row's shell, or null if nothing has asked for one yet.
    pub shell: *mut c_void,
    pub kind: NodeKind,
}

impl FfiBoundRow {
    const NONE: Self = Self {
        slot: NodeSlotId::INVALID,
        shell: std::ptr::null_mut(),
        kind: NodeKind::Unset,
    };

    fn of(arena: &LayoutNodeArena, main_thread: &crate::stage::MainThread, slot: NodeSlotId) -> Self {
        if slot.is_invalid() {
            return Self::NONE;
        }
        let data = arena.data(slot);
        Self {
            slot,
            shell: data
                .shell
                .get()
                .map_or(std::ptr::null_mut(), |shell| shell.host_object(main_thread)),
            kind: data.kind.get(),
        }
    }
}

/// The row the element or text node with `style_node` is bound to, or, for a nonzero
/// `generated_for`, the row of its pseudo-element of that kind. Unlike
/// [`layout_arena_bound_shell`], this makes no shell.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bound_row_of(arena: *mut c_void, style_node: u32, generated_for: u8) -> FfiBoundRow {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return FfiBoundRow::NONE;
    };
    // SAFETY: As for `layout_arena_bound_shell`: the binding, the row's kind and its shell are
    // nothing a recording writes.
    let arena = unsafe { LayoutNodeArena::from_handle_beside_recording(arena) };
    let slot = if generated_for == 0 {
        arena.bound_row(style_node)
    } else {
        arena.bound_pseudo_element_row(style_node, generated_for)
    };
    FfiBoundRow::of(arena, &main_thread, slot)
}

/// The viewport row the document is bound to. Unlike [`layout_arena_bound_viewport_shell`], this
/// makes no shell.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bound_viewport_row(arena: *mut c_void) -> FfiBoundRow {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle_beside_recording(arena) };
    FfiBoundRow::of(arena, &main_thread, arena.bound_viewport_row())
}

/// The row `slot` links to by `link`, with its shell if one was made.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_linked_row(arena: *mut c_void, slot: NodeSlotId, link: FfiNodeLink) -> FfiBoundRow {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call. The links are
    // nothing a recording writes.
    let arena = unsafe { LayoutNodeArena::from_handle_beside_recording(arena) };
    FfiBoundRow::of(arena, &main_thread, arena.node_link_slot(slot, link))
}

/// The row `slot` names, with its shell if one was made, or none if the row is no longer live,
/// which a slot noted earlier may not be.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_row_if_live(arena: *mut c_void, slot: NodeSlotId) -> FfiBoundRow {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: The C++ caller keeps the arena alive for this synchronous call. Which slots are live
    // is nothing a recording writes.
    let arena = unsafe { LayoutNodeArena::from_handle_beside_recording(arena) };
    if !arena.slot_is_live(slot) {
        return FfiBoundRow::NONE;
    }
    FfiBoundRow::of(arena, &main_thread, slot)
}

/// The shell of the viewport row the document is bound to, materialised if nothing has asked for
/// it yet, or null.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bound_viewport_shell(arena: *mut c_void) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry_beside_recording(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle_beside_recording(arena) };
    let row = arena.bound_viewport_row();
    if row.is_invalid() {
        return std::ptr::null_mut();
    }
    arena.node_shell(&main_thread, row)
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_bind_row(arena: *mut c_void, id: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (LayoutNodeArena::from_handle(arena)).bind_row(id);
        });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_unbind_row(arena: *mut c_void, id: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (LayoutNodeArena::from_handle(arena)).unbind_row(id);
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
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The C++ wrapper keeps the arena alive for this call and serializes all access on
    // the document thread.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
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
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (LayoutNodeArena::from_handle(arena))
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
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            (LayoutNodeArena::from_handle(arena))
                .set_style_node_of_generated_subtree(root, StyleNodeID::from_raw(style_node));
        });
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_prepare_node_for_detach(arena: *mut c_void, row: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { LayoutNodeArena::from_handle(arena) }.assert_owner_thread();
    // SAFETY: As above.
    unsafe { paying_host_handbacks(&main_thread, arena, || prepare_row_for_detach(arena, row)) }
}

/// Prepares every row in the layout subtree `root` heads for leaving the tree.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_prepare_subtree_for_detach(arena: *mut c_void, root: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: The handle came from layout_arena_create and outlives this call.
    unsafe { paying_host_handbacks(&main_thread, arena, || prepare_subtree_for_detach(arena, root)) }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_forget_style_node(arena: *mut c_void, style_node: u32) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let Some(style_node) = StyleNodeID::from_raw(style_node) else {
        return;
    };
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe {
        paying_host_handbacks(&main_thread, arena, || {
            LayoutNodeArena::from_handle_mut(arena).forget_style_node(style_node);
        });
        // A retired identity leaves its layout tree update marks behind too, which the document
        // thread holds.
        super::super::tree_update_marks::with_document_marks(arena, |marks| marks.clear(style_node));
    }
}

/// The arena and record must be live on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_adopt_derived_node_style(arena: *mut c_void, node: NodeSlotId, record: u64) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let derived = arena.with_style_engine(|engine| {
        engine.pin_layout_style_record(record);
        DerivedStyleRecord {
            record,
            payloads: crate::layout::node_data::StylePayloadsRef::new(
                engine.style_record_payloads(record).unwrap().as_ptr().cast(),
            ),
        }
    });
    arena.apply_reinherited_style_record(node, derived, ShellStyleChangeNotice::Now(&main_thread));
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_set_layout_display(arena: *mut c_void, node: NodeSlotId, display: u32) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    unsafe { LayoutNodeArena::from_handle(arena) }.update_layout_style(
        node,
        ShellStyleChangeNotice::Now(&main_thread),
        |style| {
            style.set_display(crate::css::display::FfiDisplay::from_raw(display));
        },
    );
}

/// What the host's `NodeWithStyle::apply_style()` does to the arena for a row without a shell,
/// taking a style that holds no images, in one call: the host's pin follows the record, an
/// adoption left by a sample installed ahead is taken, and otherwise the record, its flags and
/// the anonymous descendants' inherited style are written. Returns the image observers the row
/// let go of, which the host deletes.
///
/// # Safety
///
/// The arena must be live on the document thread, `node` must name a live row with style, and
/// `payloads` must be the payloads the style engine holds for `style_record`.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_install_row_style(
    arena: *mut c_void,
    node: NodeSlotId,
    style_record: u64,
    payloads: *const c_void,
) -> *mut c_void {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    let host_tables = unsafe { crate::layout::HostTables::from_handle(arena) };
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    if style_record != arena.node_style_record(node) {
        arena.release_node_style_record_pin_for_host(node);
    }
    // Taking the adoption hands the host a pin of its own, so this comes after the old pin went.
    let installed_ahead = arena.take_animation_adoption(node, style_record);
    if !installed_ahead {
        if arena.set_node_style(node, style_record, payloads) {
            arena.refresh_style_flags(node);
        }
        arena.enroll_node_for_svg_paint_resources_sync(node);
        arena.set_node_flag(node, NodeFlag::HasAnimatedOpacityOrTransform, false);
        arena.reinherit_anonymous_descendants(node, ShellStyleChangeNotice::Now(&main_thread));
    }
    let old_image_observers = arena.replace_image_observers(host_tables, node, std::ptr::null_mut());
    arena.note_style_image_resources_attached(node, false);
    // A pseudo-element's row can outlive its DOM pseudo-element's record until the tree is rebuilt.
    if arena.node_generated_for(node) != 0 {
        arena.pin_node_style_record_for_host(node, style_record);
    }
    old_image_observers
}

/// The record a row holds, and whether it holds one the arena derived for it rather than one the
/// host installs.
#[repr(C)]
pub struct FfiRowStyleRecord {
    pub derived: bool,
    pub record: u64,
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_row_style_record(arena: *mut c_void, node: NodeSlotId) -> FfiRowStyleRecord {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above; the row's record and its pins are nothing a recording writes.
    let arena = unsafe { LayoutNodeArena::from_handle_beside_recording(arena) };
    FfiRowStyleRecord {
        derived: arena.node_style_record_is_pinned_by_arena(node),
        record: arena.node_style_record(node),
    }
}

/// What the host's `NodeWithStyle::set_style_record_identity()` does to the arena for a row without
/// a shell, in one call: the host's pin follows the record, an adoption left by a sample installed
/// ahead is taken, and otherwise the record is written, with the caches of the row and its
/// ancestors reset if `changes_layout_affecting_style` and the record is another than the row's.
///
/// # Safety
///
/// The arena must be live on the document thread, `node` must name a live row with style, and
/// `payloads` must be the payloads the style engine holds for `style_record`.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_replace_row_style_record(
    arena: *mut c_void,
    node: NodeSlotId,
    style_record: u64,
    payloads: *const c_void,
    changes_layout_affecting_style: bool,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let keeps_record = arena.node_style_record(node) == style_record;
    let pinned_by_host = arena.node_style_record_pinned_by_host(node);
    // A record installed ahead of the host is the row's already; the host's pin on the old record
    // still goes first.
    if !keeps_record || (pinned_by_host != 0 && pinned_by_host != style_record) {
        arena.release_node_style_record_pin_for_host(node);
    }
    // Taking the adoption hands the host a pin of its own, so this comes after the old pin went.
    let installed_ahead = arena.take_animation_adoption(node, style_record);
    if !installed_ahead {
        if arena.set_node_style(node, style_record, payloads) {
            arena.refresh_style_flags(node);
        }
        arena.enroll_node_for_svg_paint_resources_sync(node);
    }
    if pinned_by_host != 0 {
        arena.pin_node_style_record_for_host(node, style_record);
    }
    if !keeps_record && changes_layout_affecting_style && !installed_ahead {
        arena.bump_fragment_cache_epoch_of_self_and_ancestors(node);
        arena.reset_cached_intrinsic_sizes_of_self_and_ancestors(node);
    }
}

#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_reinherit_anonymous_descendants(arena: *mut c_void, node: NodeSlotId) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }
        .reinherit_anonymous_descendants(node, ShellStyleChangeNotice::Now(&main_thread));
}
