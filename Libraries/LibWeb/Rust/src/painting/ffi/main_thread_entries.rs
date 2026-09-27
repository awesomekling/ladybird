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

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread. The
/// host callback receives live layout node shells.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_scrolling_box_for_scroll_step(
    arena: *mut c_void,
    target: NodeSlotId,
    viewport: NodeSlotId,
    delta: FfiCssPixelPoint,
    viewport_wheel_overflow_x: u8,
    viewport_wheel_overflow_y: u8,
) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let scrolling_box = crate::painting::scroll_chain::scrolling_box_for_scroll_step(
        &paintable_rows,
        target,
        viewport,
        delta.into(),
        ViewportWheelOverflow {
            x: viewport_wheel_overflow_x,
            y: viewport_wheel_overflow_y,
        },
        &scroll_offset_reader(&paintable_rows),
    );
    paintable_rows.shell_if_live(&main_thread, scrolling_box)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread. The
/// host callback receives live layout node shells.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_for_each_wheel_scrollable_box_in_containing_block_chain(
    arena: *mut c_void,
    start: NodeSlotId,
    wheel_delta_x: f64,
    wheel_delta_y: f64,
    viewport_wheel_overflow_x: u8,
    viewport_wheel_overflow_y: u8,
    context: *mut c_void,
    push_scrollable_box: unsafe extern "C" fn(*mut c_void, *mut c_void, f64, f64),
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    crate::painting::scroll_chain::for_each_wheel_scrollable_box_in_containing_block_chain(
        &paintable_rows,
        start,
        wheel_delta_x,
        wheel_delta_y,
        ViewportWheelOverflow {
            x: viewport_wheel_overflow_x,
            y: viewport_wheel_overflow_y,
        },
        &scroll_offset_reader(&paintable_rows),
        |node, accepted_delta_x, accepted_delta_y| {
            // SAFETY: The C++ callback appends the shell and deltas to a caller-owned collection.
            unsafe {
                push_scrollable_box(
                    context,
                    paintable_rows.node_shell(&main_thread, node),
                    accepted_delta_x,
                    accepted_delta_y,
                );
            }
        },
    );
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_first_wheel_scrollable_box_in_containing_block_chain(
    arena: *mut c_void,
    start: NodeSlotId,
    viewport_wheel_overflow_x: u8,
    viewport_wheel_overflow_y: u8,
) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let scrollable_box = crate::painting::scroll_chain::first_wheel_scrollable_box_in_containing_block_chain(
        &paintable_rows,
        start,
        ViewportWheelOverflow {
            x: viewport_wheel_overflow_x,
            y: viewport_wheel_overflow_y,
        },
    );
    paintable_rows.shell_if_live(&main_thread, scrollable_box)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_paintable_cleared_from_node(arena: *mut c_void, layout_node: NodeSlotId) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    // SAFETY: Guaranteed by the entry point's contract.
    unsafe {
        arena_from_handle_mut(arena).release_published_paintable_rows();
        crate::layout::paying_host_handbacks(&main_thread, arena, || {
            clear_paintable_row_of_node(arena, layout_node);
        });
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_paintable_event_dispatch_node_shell(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    crate::painting::hit_test::resolve::event_dispatch_shell_for_paintable(&main_thread, &paintable_rows, slot)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_paintable_layout_node_shell(arena: *mut c_void, slot: NodeSlotId) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let arena = unsafe { arena_from_handle(arena) };
    if !arena.paintable_row_is_populated(slot) {
        return std::ptr::null_mut();
    }
    arena.shell_if_live(&main_thread, slot)
}

/// # Safety
///
/// `arena` must be a live arena used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_prepare_for_rendering(
    arena: *mut c_void,
    visual_context_update_pending: bool,
) -> FfiRenderingPreparationOutcome {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let arena = unsafe { arena_from_handle_mut(arena) };
    let (background_source_changed, clamped) = arena.run_stage(|arena| {
        let root_background_source = crate::layout::root_background_source(arena);
        prepare_root_background_and_overflow(arena, root_background_source)
    });
    crate::painting::scrollable_overflow::hand_over_clamped_scroll_offsets(arena, &main_thread, clamped);
    arena.run_stage(|arena| {
        finish_rendering_preparation(arena, background_source_changed, visual_context_update_pending)
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread. The
/// host callback receives each snap area's geometry, valid for the duration of the call, and the
/// area's live layout node shell.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_for_each_snap_area(
    arena: *mut c_void,
    snap_container: NodeSlotId,
    context: *mut c_void,
    push_snap_area: unsafe extern "C" fn(*mut c_void, *const crate::painting::host::FfiSnapAreaGeometry, *mut c_void),
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    crate::painting::scroll_snap::for_each_snap_area(&paintable_rows, snap_container, |slot, area| {
        // SAFETY: The C++ callback copies the geometry into a caller-owned collection.
        unsafe { push_snap_area(context, &raw const area, paintable_rows.node_shell(&main_thread, slot)) };
    });
}

/// Resolves the SVG-as-image renders the next recording is predicted to paint into the map it
/// looks them up in: the ones the last recording painted or missed, and the first paints of image
/// elements and layers.
/// Rendering an image lays out and records another document, so the main thread does it here,
/// before the recording stage runs, rather than the stage asking for it.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`; `inputs` are the next recording's; the
/// callbacks in `vector_images` are called synchronously with their context.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_resolve_painted_vector_images(
    arena: *mut c_void,
    inputs: &crate::painting::host::FfiRecordingInputs,
    vector_images: crate::painting::host::FfiVectorImageCallbacks,
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let arena = unsafe { arena_from_handle(arena) };
    arena.join_frame_for_main_side_write("painted vector images");
    let mut last_painted = std::collections::HashSet::new();
    let prediction_inputs = {
        let paint_state = arena.paint_state().borrow();
        // The renders the last recording painted, whether it recorded them or copied them, and the
        // ones it missed, whose producers record again.
        if let Some(recording) = paint_state.last_recording.as_ref() {
            last_painted.extend(recording.vector_images.values().copied());
            last_painted.extend(recording.missed_vector_images.iter().copied());
        }
        paint_state.visual_context.last_tree_inputs.map(|tree_inputs| {
            crate::painting::record::vector_images::FirstPaintPredictionInputs {
                device_pixels_per_css_pixel: tree_inputs.device_pixels_per_css_pixel,
                root_background_source: paint_state.root_background_source,
                css_viewport_rect: inputs.css_viewport_rect.into(),
                document_declares_light_or_dark_color_scheme: inputs.document_declares_light_or_dark_color_scheme,
                image_color_scheme_fallback: inputs.image_color_scheme_fallback,
            }
        })
    };
    let predicted = prediction_inputs
        .map(|prediction_inputs| {
            crate::painting::record::vector_images::predict_first_paint_renders(arena, &prediction_inputs)
        })
        .unwrap_or_default();
    // The predicted renders go last and in their order: a document images share keeps the layout of
    // its last render.
    let predicted_set: std::collections::HashSet<_> = predicted.iter().copied().collect();
    let mut painted: Vec<_> = last_painted
        .into_iter()
        .filter(|request| !predicted_set.contains(request))
        .collect();
    painted.extend(predicted);
    let mut resolved = crate::painting::record::vector_images::VectorImageDisplayLists::default();
    // Each render records another document, which must not find this arena's paint state borrowed.
    for request in painted {
        let display_list = vector_images.resolve_vector_image_display_list(&main_thread, &request.to_ffi());
        resolved.insert(
            request,
            crate::painting::display_list::commands::DisplayListResourceId(display_list),
        );
    }
    arena.paint_state().borrow_mut().vector_image_display_lists = std::sync::Arc::new(resolved);
}

/// Whether the last recording painted an SVG-as-image render as an empty image because the main
/// thread had not resolved it, so the host has to schedule the frame that paints it.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_last_recording_missed_vector_images(arena: *mut c_void) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    arena
        .paint_state()
        .borrow()
        .last_recording
        .as_ref()
        .is_some_and(|recording| !recording.missed_vector_images.is_empty())
}

/// Discards the pending recording if the document retired the render state it was made for since,
/// and tore down what was recorded. Returns whether it did; the recording is not published then.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_discard_retired_recording(arena: *mut c_void) -> bool {
    let arena_handle = arena;
    let arena = unsafe { arena_from_handle(arena) };
    arena.join_frame_for_main_side_write("retired recording discard");
    let mut recording = arena.recording();
    let Some(generation) = recording
        .pending_recording()
        .as_ref()
        .map(|pending| pending.frame_generation)
    else {
        return false;
    };
    // SAFETY: The caller passes a live handle.
    if !unsafe { crate::layout::frame_retirement::frame_was_retired(arena_handle, generation) } {
        return false;
    }
    recording.discard_pending_recording();
    true
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`; the callbacks in `publish` are
/// called synchronously with their context while the recording's resources are live.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_publish_recording(
    arena: *mut c_void,
    publish: crate::painting::host::FfiRecordingPublishCallbacks,
) -> u64 {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let arena = unsafe { arena_from_handle(arena) };
    arena.join_frame_for_main_side_write("recording publication");
    let Some(pending) = arena.recording().pending_recording().take() else {
        return 0;
    };
    let publish = crate::painting::host::RecordingPublishHost::from(publish);
    crate::painting::record::publish::publish_recording(arena, pending, &main_thread, &publish)
}

/// Hands the image frames of the published SVG filters to the host's resource storage. A visual
/// context tree read after a recording was published can reference a filter image the recording
/// never saw (the read synchronizes SVG paint resources first), and the frame it references has to
/// be in the storage before the tree is sent.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`; the callbacks in `publish` are
/// called synchronously with their context.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_publish_svg_filter_image_frames(
    arena: *mut c_void,
    publish: crate::painting::host::FfiRecordingPublishCallbacks,
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let arena = unsafe { arena_from_handle(arena) };
    let publish = crate::painting::host::RecordingPublishHost::from(publish);
    for frame in arena.svg_paint_resources().published_filter_image_frames() {
        publish.add_image_frame(&main_thread, &frame);
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`; `describe_node` and `append_text`
/// are called synchronously with `context`, and the shells handed to `describe_node` are the
/// last recording's live paintable shells.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_take_recording_trace(
    arena: *mut c_void,
    context: *mut c_void,
    describe_node: unsafe extern "C" fn(*mut c_void, *mut c_void, *mut c_void),
    append_text: unsafe extern "C" fn(*mut c_void, *const u8, usize),
) -> bool {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let arena = unsafe { arena_from_handle(arena) };
    let (pending, recording) = {
        let Some(pending) = arena.recording().pending_recording_trace().take() else {
            return false;
        };
        let Some(recording) = arena.paint_state().borrow().last_recording.clone() else {
            return false;
        };
        (pending, recording)
    };
    let Some(log) = recording.capture_log_for_verification.as_ref() else {
        return false;
    };
    let mut name = |slot| {
        if slot == pending.viewport {
            return "@viewport".into();
        }
        let mut name = Vec::<u8>::new();
        // SAFETY: the last recording's paintable shells are still live, and the host copies the
        // description synchronously into the sink.
        unsafe { describe_node(context, arena.shell_if_live(&main_thread, slot), (&raw mut name).cast()) };
        String::from_utf8(name).expect("trace label must be UTF-8")
    };
    let text = format!(
        "recording (overlay={})\n{}",
        pending.should_paint_overlay,
        log.format(&mut name)
    );
    // SAFETY: the host copies the text synchronously.
    unsafe { append_text(context, text.as_ptr(), text.len()) };
    true
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_text_caret_rect_for_position(
    arena: *mut c_void,
    primary: NodeSlotId,
    offset: usize,
    affinity_is_downstream: bool,
) -> FfiCaretRectResult {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let mut result = FfiCaretRectResult {
        found: false,
        rect: FfiCssPixelRect::default(),
        style_source: std::ptr::null_mut(),
        owner_paintable: NodeSlotId::INVALID,
        nearest_self_painting_inline: NodeSlotId::INVALID,
    };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    let Some(answer) =
        crate::painting::caret::caret_rect_for_position(&paintable_rows, node_slots, offset, affinity_is_downstream)
    else {
        return result;
    };
    result.found = true;
    result.rect = answer.rect.into();
    result.style_source = paintable_rows.shell_if_live(&main_thread, answer.style_source);
    result.owner_paintable = answer.owner;
    result.nearest_self_painting_inline =
        crate::painting::fragment_ownership::nearest_self_painting_inline_box(&paintable_rows, answer.node)
            .unwrap_or(NodeSlotId::INVALID);
    result
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_atomic_inline_caret_rect_for_position(
    arena: *mut c_void,
    primary: NodeSlotId,
    after: bool,
) -> FfiCaretRectResult {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let mut result = FfiCaretRectResult {
        found: false,
        rect: FfiCssPixelRect::default(),
        style_source: std::ptr::null_mut(),
        owner_paintable: NodeSlotId::INVALID,
        nearest_self_painting_inline: NodeSlotId::INVALID,
    };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let Some(answer) = crate::painting::caret::caret_rect_for_atomic_inline(&paintable_rows, primary, after) else {
        return result;
    };
    result.found = true;
    result.rect = answer.rect.into();
    result.style_source = paintable_rows.shell_if_live(&main_thread, answer.style_source);
    result.owner_paintable = answer.owner;
    result.nearest_self_painting_inline =
        crate::painting::fragment_ownership::nearest_self_painting_inline_box(&paintable_rows, answer.node)
            .unwrap_or(NodeSlotId::INVALID);
    result
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_paintable_empty_line_caret_rect(
    arena: *mut c_void,
    block: NodeSlotId,
    primary: NodeSlotId,
    offset: usize,
) -> FfiEmptyLineCaretRect {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let mut result = FfiEmptyLineCaretRect {
        has_value: false,
        rect: FfiCssPixelRect::default(),
        style_source: std::ptr::null_mut(),
    };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(block) {
        return result;
    }
    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    let side = paintable_rows.committed_side_data(block);
    let Some(first_fragment) = side.fragments().first() else {
        return result;
    };
    if !node_slots.contains(&first_fragment.layout_node) {
        return result;
    }
    for target in crate::painting::visual_lines::empty_line_caret_targets(&paintable_rows, block) {
        if target.offset == offset {
            result.has_value = true;
            result.rect = target.rect.into();
            result.style_source = paintable_rows.shell_if_live(
                &main_thread,
                crate::painting::text_fragment::style_source(&paintable_rows, first_fragment),
            );
            break;
        }
    }
    result
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_for_each_subtree_fragment_rect(
    arena: *mut c_void,
    root: NodeSlotId,
    context: *mut c_void,
    consume: unsafe extern "C" fn(*mut c_void, *mut c_void, FfiCssPixelRect),
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(root) {
        return;
    }
    crate::painting::paint_order::for_each_in_paint_subtree(&paintable_rows, root, |current| {
        for fragment in paintable_rows.committed_side_data(current).fragments() {
            let shell = paintable_rows.shell_if_live(&main_thread, fragment.layout_node);
            let rect = crate::painting::text_fragment::absolute_rect(&paintable_rows, fragment).into();
            crate::painting::seal::note_host_call("for_each_subtree_fragment_rect_consume");
            // SAFETY: The consumer copies its plain-data arguments synchronously.
            unsafe { consume(context, shell, rect) };
        }
    });
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `index` in range.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_hit_test_item_facts(
    arena: *mut c_void,
    index: usize,
) -> crate::painting::host::FfiHitTestItemExport {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    with_hit_test_list_items_only(arena, None, |list, arena| {
        let item = &list.items[index];
        assert!(
            arena.paintable_row_is_populated(item.paintable),
            "exporting a hit-test item for a non-live paintable"
        );
        assert!(
            arena.paintable_row_is_populated(item.hit_node),
            "exporting a hit-test item that names a non-live paintable"
        );
        Some(crate::painting::host::FfiHitTestItemExport {
            can_produce_caret_position: item.can_produce_caret_position,
            paintable: item.paintable,
            hit_node: item.hit_node,
            chrome_widget_kind: item.chrome_widget_kind,
            caret_node_shell: arena.shell_if_live(&main_thread, item.caret_node),
            caret_rect: item.caret_rect.into(),
            context: item.context,
        })
    })
    .expect("no hit-test list")
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `item_index` must be in range for the current hit-test list.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_hit_test_item_target_shell(arena: *mut c_void, item_index: usize) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    with_hit_test_list_items_only(arena, std::ptr::null_mut(), |list, arena| {
        list.item_target_shell(&main_thread, arena, item_index)
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `item_index` must be in range for the current hit-test list and `out_allow_pseudo_fallback`
/// must be writable.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_hit_test_item_dispatch_shell(
    arena: *mut c_void,
    item_index: usize,
    out_allow_pseudo_fallback: *mut bool,
) -> *mut c_void {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    with_hit_test_list_items_only(arena, std::ptr::null_mut(), |list, arena| {
        let (shell, allow_pseudo_fallback) = list.item_dispatch_shell(&main_thread, arena, item_index);
        // SAFETY: The caller provides writable storage for the synchronous result.
        unsafe { *out_allow_pseudo_fallback = allow_pseudo_fallback };
        shell
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `item_index` must be in range for the current hit-test list.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_hit_test_resolve_hit(
    arena: *mut c_void,
    item_index: usize,
    local_point: FfiCssPixelPoint,
) -> crate::painting::host::FfiResolvedHit {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    with_hit_test_list_items_only(arena, Default::default(), |list, arena| {
        list.resolve_hit(&main_thread, arena, item_index, local_point.into())
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `item_index` must be in range for the current hit-test list.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_hit_test_resolve_caret(
    arena: *mut c_void,
    item_index: usize,
    local_point: FfiCssPixelPoint,
    position_type: u8,
) -> crate::painting::host::FfiResolvedCaret {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    with_hit_test_list_items_only(arena, Default::default(), |list, arena| {
        list.resolve_caret(
            &main_thread,
            arena,
            item_index,
            local_point.into(),
            crate::painting::hit_test::caret::CaretPositionType::from_u8(position_type),
        )
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// both resolvers must answer synchronously from a live layout node shell and only push into
/// the sink whose pointer they receive.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_sync_svg_paint_resources(
    arena: *mut c_void,
    resolve_filter: unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> bool,
    resolve_paint_server: unsafe extern "C" fn(*mut c_void, bool, *mut c_void),
) -> bool {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    use crate::painting::svg_paint_resources::{PublishedSvgFilter, PublishedSvgPaintServer, SvgPaintResourceKind};
    let arena = unsafe { arena_from_handle(arena) };
    let resources = arena.svg_paint_resources();
    if !resources.take_needs_sync() {
        return false;
    }
    let mut any_changed = false;
    for (slot, kind) in resources.enrolled_entries() {
        let Some(style) = arena.node_style_if_live(slot) else {
            resources.forget_slot(slot);
            continue;
        };
        if matches!(kind, SvgPaintResourceKind::Fill | SvgPaintResourceKind::Stroke) {
            let is_stroke = kind == SvgPaintResourceKind::Stroke;
            let mut published = PublishedSvgPaintServer::None;
            crate::painting::seal::note_host_call("resolve_paint_server");
            // SAFETY: The host resolves synchronously from the live shell and only pushes into
            // the sink it is handed.
            unsafe {
                resolve_paint_server(
                    arena.shell_if_live(&main_thread, slot),
                    is_stroke,
                    (&raw mut published).cast(),
                );
            }
            if resources.publish_paint_server(slot, kind, published) {
                any_changed = true;
                use crate::painting::record::damage::PaintDamage;
                arena.push_paint_damage(slot, PaintDamage::SVG | PaintDamage::SCOPE_PREAMBLE);
            }
            continue;
        }
        let effects = style.effects();
        let filter_list = match kind {
            SvgPaintResourceKind::Filter => &effects.filter,
            SvgPaintResourceKind::BackdropFilter => &effects.backdrop_filter,
            SvgPaintResourceKind::Fill | SvgPaintResourceKind::Stroke => unreachable!(),
        };
        if !crate::painting::css_filter::contains_url(filter_list) {
            resources.withdraw(slot, kind);
            continue;
        }
        let shell = arena.shell_if_live(&main_thread, slot);
        let mut published = PublishedSvgFilter::default();
        for operation in filter_list.operations.as_slice() {
            if operation.kind != crate::painting::css_filter::FILTER_KIND_URL {
                continue;
            }
            let mut primitives: Vec<SvgFilterPrimitive> = Vec::new();
            crate::painting::seal::note_host_call("resolve_filter");
            // SAFETY: The host resolves synchronously from the live shell and only pushes into the
            // primitive list it is handed as its sink.
            let resolved = unsafe { resolve_filter(shell, operation.url_value.pointer, (&raw mut primitives).cast()) };
            published = PublishedSvgFilter {
                failed: !resolved,
                primitives: if resolved { primitives } else { Vec::new() },
            };
            if published.failed {
                break;
            }
        }
        if resources.publish_filter(slot, kind, published) {
            any_changed = true;
            if arena.paintable_row_is_populated(slot) {
                arena.note_visual_context_box_dirty(
                    slot,
                    crate::painting::visual_context::dirty::VisualContextBoxDirtyKind::StyleValueChange,
                );
                use crate::painting::record::damage::PaintDamage;
                arena.push_paint_damage(slot, PaintDamage::SVG | PaintDamage::SCOPE_PREAMBLE);
            }
        }
    }
    any_changed
}
