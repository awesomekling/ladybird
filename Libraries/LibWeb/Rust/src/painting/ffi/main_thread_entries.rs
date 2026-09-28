/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The FFI entries of the parent module that mint the main thread capability. Only this module can
//! construct its marker, and the entries are private, so stage code in the parent module can neither
//! mint the capability nor call an entry that does.

use super::*;
use crate::render_owner::ScriptForcedRead;

pub(crate) struct MainThreadFfiEntry {
    _private: (),
}

const MAIN_THREAD_FFI_ENTRY: MainThreadFfiEntry = MainThreadFfiEntry { _private: () };

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_scrolling_box_for_scroll_step(
    arena: *mut c_void,
    target: NodeSlotId,
    viewport: NodeSlotId,
    delta: FfiCssPixelPoint,
    viewport_wheel_overflow_x: u8,
    viewport_wheel_overflow_y: u8,
) -> NodeSlotId {
    let overflow = ViewportWheelOverflow {
        x: viewport_wheel_overflow_x,
        y: viewport_wheel_overflow_y,
    };
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            let scrolling_box = crate::painting::scroll_chain::scrolling_box_for_scroll_step(
                paintable_rows,
                target,
                viewport,
                delta.into(),
                overflow,
                &scroll_offset_reader(paintable_rows),
            );
            live_slot(paintable_rows, scrolling_box)
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread. The
/// host callback receives the slot of each box.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_for_each_wheel_scrollable_box_in_containing_block_chain(
    arena: *mut c_void,
    start: NodeSlotId,
    wheel_delta_x: f64,
    wheel_delta_y: f64,
    viewport_wheel_overflow_x: u8,
    viewport_wheel_overflow_y: u8,
    context: *mut c_void,
    push_scrollable_box: unsafe extern "C" fn(*mut c_void, NodeSlotId, f64, f64),
) {
    let overflow = ViewportWheelOverflow {
        x: viewport_wheel_overflow_x,
        y: viewport_wheel_overflow_y,
    };
    // SAFETY: Guaranteed by the caller.
    let boxes = unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            let mut boxes = Vec::new();
            crate::painting::scroll_chain::for_each_wheel_scrollable_box_in_containing_block_chain(
                paintable_rows,
                start,
                wheel_delta_x,
                wheel_delta_y,
                overflow,
                &scroll_offset_reader(paintable_rows),
                |node, accepted_delta_x, accepted_delta_y| boxes.push((node, accepted_delta_x, accepted_delta_y)),
            );
            boxes
        })
    };
    for (node, accepted_delta_x, accepted_delta_y) in boxes {
        // SAFETY: The C++ callback appends the slot and deltas to a caller-owned collection.
        unsafe { push_scrollable_box(context, node, accepted_delta_x, accepted_delta_y) };
    }
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
) -> NodeSlotId {
    let overflow = ViewportWheelOverflow {
        x: viewport_wheel_overflow_x,
        y: viewport_wheel_overflow_y,
    };
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            let scrollable_box = crate::painting::scroll_chain::first_wheel_scrollable_box_in_containing_block_chain(
                paintable_rows,
                start,
                overflow,
            );
            live_slot(paintable_rows, scrollable_box)
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_paintable_event_dispatch_slot(arena: *mut c_void, slot: NodeSlotId) -> NodeSlotId {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            crate::painting::hit_test::resolve::event_dispatch_slot_for_paintable(paintable_rows, slot)
                .map_or(NodeSlotId::INVALID, |slot| live_slot(paintable_rows, slot))
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_paintable_committed_slot(arena: *mut c_void, slot: NodeSlotId) -> NodeSlotId {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, ScriptForcedRead::at_script_entry(), |rows| {
            if rows.paintable_row_is_populated(slot) {
                live_slot(rows, slot)
            } else {
                NodeSlotId::INVALID
            }
        })
    }
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
    use crate::painting::owner_pass::{PaintPass, run_paint_pass_of};
    // SAFETY: Guaranteed by the caller.
    let (background_source_changed, clamped) = unsafe {
        run_paint_pass_of(
            arena,
            PaintPass::RootBackgroundAndOverflow,
            |arena, ()| crate::painting::ffi::root_background_and_overflow(arena),
            (),
        )
    };
    crate::painting::scrollable_overflow::hand_over_clamped_scroll_offsets(&main_thread, clamped);
    // SAFETY: As above.
    unsafe {
        run_paint_pass_of(
            arena,
            PaintPass::FinishRenderingPreparation,
            |arena, (background_source_changed, visual_context_update_pending)| {
                finish_rendering_preparation(arena, background_source_changed, visual_context_update_pending)
            },
            (background_source_changed, visual_context_update_pending),
        )
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread. The
/// host callback receives each snap area's geometry, valid for the duration of the call, and the
/// area's slot.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_for_each_snap_area(
    arena: *mut c_void,
    snap_container: NodeSlotId,
    context: *mut c_void,
    push_snap_area: unsafe extern "C" fn(*mut c_void, *const crate::painting::host::FfiSnapAreaGeometry, NodeSlotId),
) {
    // SAFETY: Guaranteed by the caller.
    let areas = unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            let mut areas = Vec::new();
            crate::painting::scroll_snap::for_each_snap_area(paintable_rows, snap_container, |slot, area| {
                areas.push((slot, area));
            });
            areas
        })
    };
    for (slot, area) in areas {
        // SAFETY: The C++ callback copies the geometry into a caller-owned collection.
        unsafe { push_snap_area(context, &raw const area, slot) };
    }
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
    // The prediction reads the paint damage the next recording records afresh, which only the owner holds.
    // SAFETY: Guaranteed by the caller.
    let crate::render_owner::ArenaAnswer::VectorImages(painted) = (unsafe {
        crate::render_owner::ask_arena_of(
            arena,
            crate::render_owner::ArenaQuery::PaintedVectorImages {
                css_viewport_rect: inputs.css_viewport_rect,
                document_declares_light_or_dark_color_scheme: inputs.document_declares_light_or_dark_color_scheme,
                image_color_scheme_fallback: inputs.image_color_scheme_fallback,
            },
            crate::render_owner::LockstepProof::recording_on_main(),
        )
    }) else {
        return;
    };
    // Each render lays out and records another document, which the main thread does.
    let mut resolved = crate::painting::record::vector_images::VectorImageDisplayLists::default();
    for request in painted {
        let display_list = vector_images.resolve_vector_image_display_list(&main_thread, &request.to_ffi());
        resolved.insert(
            request,
            crate::painting::display_list::commands::DisplayListResourceId(display_list),
        );
    }
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::VectorImageDisplayLists(std::sync::Arc::new(resolved)),
        );
    }
}

/// Whether the last recording painted an SVG-as-image render as an empty image because the main
/// thread had not resolved it, so the host has to schedule the frame that paints it.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_last_recording_missed_vector_images(arena: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::current(arena, ScriptForcedRead::at_script_entry()) }
        .paint_status
        .last_recording_missed_vector_images
}

/// Discards the pending recording if the document retired the render state it was made for since,
/// and tore down what was recorded. Returns whether it did; the recording is not published then.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_discard_retired_recording(arena: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    let generation = unsafe { crate::layout::frame_retirement::frame_generation(arena) };
    // SAFETY: As above; this thread waits for the pass.
    let discarded = unsafe {
        crate::painting::owner_pass::run_held_pass(arena, generation, |arena, generation| {
            let mut recording = arena.recording();
            let retired = recording
                .pending_recording()
                .as_ref()
                .is_some_and(|pending| pending.frame_generation != generation);
            if retired {
                recording.discard_pending_recording();
            }
            retired
        })
    };
    if discarded {
        crate::layout::frame_retirement::note_frame_retired();
    }
    discarded
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`; the callbacks in `publish` are
/// called synchronously with their context while the recording's resources are live.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_publish_recording(
    arena: *mut c_void,
    publish: crate::painting::host::FfiRecordingPublishCallbacks,
    out: *mut FfiPresentedRecording,
) {
    // SAFETY: Guaranteed by the caller; this thread waits for the pass, which reaches the resource storage it lends.
    let presented = unsafe {
        crate::painting::owner_pass::run_held_pass(arena, publish, |arena, publish| {
            let pending = arena.recording().pending_recording().take();
            if let Some(pending) = pending {
                let publish = crate::painting::host::RecordingPublishHost::from(publish);
                // SAFETY: This is a paint pass the document thread waits for.
                let publication = crate::painting::host::WaitedPublication::new();
                crate::painting::record::publish::publish_recording(arena, pending, &publication, &publish);
            }
            FfiPresentedRecording::of_last_recording(arena)
        })
    };
    // SAFETY: Guaranteed by the caller.
    unsafe { out.write(presented) };
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
    // SAFETY: Guaranteed by the caller.
    let frames = crate::painting::svg_paint_resources::published_filter_image_frames_in(
        &unsafe { RowSnapshot::current(arena, ScriptForcedRead::at_script_entry()) }
            .paint_facts
            .svg_paint_resources,
    );
    let publish = crate::painting::host::RecordingPublishHost::from(publish);
    for frame in frames {
        publish.add_image_frame(&main_thread, &frame);
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`; `describe_node` and `append_text`
/// are called synchronously with `context`, and `describe_node` is handed the slots of the last
/// recording's rows.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_take_recording_trace(
    arena: *mut c_void,
    context: *mut c_void,
    describe_node: crate::layout::debug_text::DescribeDomNode,
    append_text: unsafe extern "C" fn(*mut c_void, *const u8, usize),
) -> bool {
    // SAFETY: Guaranteed by the caller.
    let trace = unsafe {
        crate::painting::owner_pass::run_held_pass(arena, (), |arena, ()| {
            let pending = arena.recording().pending_recording_trace().take()?;
            let recording = arena.paint_state().borrow().last_recording.clone()?;
            let log = recording.capture_log_for_verification.as_ref()?;
            let mut text = crate::layout::debug_text::DebugText::default();
            *text.text() = format!("recording (overlay={})\n", pending.should_paint_overlay);
            let log = log.format(|text, slot| {
                if slot == pending.viewport {
                    text.text().push_str("@viewport");
                } else if arena.slot_is_live(slot) {
                    text.push_box(arena, slot);
                } else {
                    text.text().push_str("(gone)");
                }
            });
            text.append(log);
            Some(text)
        })
    };
    let Some(trace) = trace else {
        return false;
    };
    // SAFETY: Guaranteed by the caller.
    let text = unsafe { trace.finish_with_host(context, describe_node) };
    // SAFETY: The host copies the text synchronously.
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            let fragments = paintable_rows.text_fragments(primary);
            caret_rect_result(
                paintable_rows,
                crate::painting::caret::caret_rect_for_position(
                    paintable_rows,
                    fragments.as_slice(),
                    offset,
                    affinity_is_downstream,
                ),
            )
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            caret_rect_result(
                paintable_rows,
                crate::painting::caret::caret_rect_for_atomic_inline(paintable_rows, primary, after),
            )
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(block) {
                return FfiEmptyLineCaretRect::default();
            }
            let fragments = paintable_rows.text_fragments(primary);
            let side = paintable_rows.committed_side_data(block);
            let Some(first_fragment) = side.fragments().first() else {
                return FfiEmptyLineCaretRect::default();
            };
            if !fragments.as_slice().contains(&first_fragment.layout_node) {
                return FfiEmptyLineCaretRect::default();
            }
            crate::painting::visual_lines::empty_line_caret_targets(paintable_rows, block)
                .into_iter()
                .find(|target| target.offset == offset)
                .map_or_else(FfiEmptyLineCaretRect::default, |target| FfiEmptyLineCaretRect {
                    has_value: true,
                    rect: target.rect.into(),
                    style_source: live_slot(
                        paintable_rows,
                        crate::painting::text_fragment::style_source(paintable_rows, first_fragment),
                    ),
                })
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_for_each_subtree_fragment_rect(
    arena: *mut c_void,
    root: NodeSlotId,
    context: *mut c_void,
    consume: unsafe extern "C" fn(*mut c_void, NodeSlotId, FfiCssPixelRect),
) {
    // SAFETY: Guaranteed by the caller.
    let rects = unsafe {
        read_committed(arena, ScriptForcedRead::at_script_entry(), |paintable_rows| {
            let mut rects = Vec::new();
            if !paintable_rows.paintable_row_is_populated(root) {
                return rects;
            }
            crate::painting::paint_order::for_each_in_paint_subtree(paintable_rows, root, |current| {
                for fragment in paintable_rows.committed_side_data(current).fragments() {
                    rects.push((
                        live_slot(paintable_rows, fragment.layout_node),
                        FfiCssPixelRect::from(crate::painting::text_fragment::absolute_rect(paintable_rows, fragment)),
                    ));
                }
            });
            rects
        })
    };
    for (slot, rect) in rects {
        // SAFETY: The consumer copies its plain-data arguments synchronously.
        unsafe { consume(context, slot, rect) };
    }
}

/// What a caret rect read answers from `answer`.
fn caret_rect_result(
    paintable_rows: &PaintSource<'_>,
    answer: Option<crate::painting::caret::CaretRectResult>,
) -> FfiCaretRectResult {
    let Some(answer) = answer else {
        return FfiCaretRectResult::default();
    };
    FfiCaretRectResult {
        found: true,
        rect: answer.rect.into(),
        style_source: live_slot(paintable_rows, answer.style_source),
        owner_paintable: answer.owner,
        nearest_self_painting_inline: crate::painting::fragment_ownership::nearest_self_painting_inline_box(
            paintable_rows,
            answer.node,
        )
        .unwrap_or(NodeSlotId::INVALID),
    }
}

/// An SVG paint resource of an enrolled row the main thread resolves from the DOM.
enum SvgPaintResourceRequest {
    PaintServer {
        slot: NodeSlotId,
        kind: crate::painting::svg_paint_resources::SvgPaintResourceKind,
    },
    /// A filter list, with the `url()` of each filter that names an SVG filter.
    Filter {
        slot: NodeSlotId,
        kind: crate::painting::svg_paint_resources::SvgPaintResourceKind,
        urls: Vec<crate::css::style_value::RetainedStyleValueData>,
    },
}

/// What the main thread resolved of an [`SvgPaintResourceRequest`].
enum ResolvedSvgPaintResource {
    PaintServer {
        slot: NodeSlotId,
        kind: crate::painting::svg_paint_resources::SvgPaintResourceKind,
        published: crate::painting::svg_paint_resources::PublishedSvgPaintServer,
    },
    Filter {
        slot: NodeSlotId,
        kind: crate::painting::svg_paint_resources::SvgPaintResourceKind,
        published: crate::painting::svg_paint_resources::PublishedSvgFilter,
    },
}

/// On the owner: what the main thread resolves of the enrolled SVG paint resources, if they changed since they were
/// resolved last. A row that went away, or whose filters name no SVG filter any more, leaves them.
fn svg_paint_resource_requests(arena: &mut crate::layout::LayoutNodeArena) -> Option<Vec<SvgPaintResourceRequest>> {
    use crate::painting::svg_paint_resources::SvgPaintResourceKind;
    let resources = arena.svg_paint_resources();
    if !resources.take_needs_sync() {
        return None;
    }
    let mut requests = Vec::new();
    for (slot, kind) in resources.enrolled_entries() {
        let Some(style) = arena.node_style_if_live(slot) else {
            resources.forget_slot(slot);
            continue;
        };
        if matches!(kind, SvgPaintResourceKind::Fill | SvgPaintResourceKind::Stroke) {
            requests.push(SvgPaintResourceRequest::PaintServer { slot, kind });
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
        let urls = filter_list
            .operations
            .as_slice()
            .iter()
            .filter(|operation| operation.kind == crate::painting::css_filter::FILTER_KIND_URL)
            .map(|operation| {
                // SAFETY: The row's style holds the value live; the request holds a reference of its own.
                unsafe {
                    crate::css::style_value::RetainedStyleValueData::from_retained_optional_pointer(
                        crate::css::style_value::retain_style_value(operation.url_value.pointer.cast()),
                    )
                }
            })
            .collect();
        requests.push(SvgPaintResourceRequest::Filter { slot, kind, urls });
    }
    Some(requests)
}

/// On the owner: publishes what the main thread resolved of the enrolled SVG paint resources, and answers whether any
/// changed.
fn publish_resolved_svg_paint_resources(
    arena: &mut crate::layout::LayoutNodeArena,
    resolved: Vec<ResolvedSvgPaintResource>,
) -> bool {
    use crate::painting::record::damage::PaintDamage;
    let resources = arena.svg_paint_resources();
    let mut any_changed = false;
    for resolved in resolved {
        match resolved {
            ResolvedSvgPaintResource::PaintServer { slot, kind, published } => {
                if arena.slot_is_live(slot) && resources.publish_paint_server(slot, kind, published) {
                    any_changed = true;
                    arena.push_paint_damage(slot, PaintDamage::SVG | PaintDamage::SCOPE_PREAMBLE);
                }
            }
            ResolvedSvgPaintResource::Filter { slot, kind, published } => {
                if !arena.slot_is_live(slot) || !resources.publish_filter(slot, kind, published) {
                    continue;
                }
                any_changed = true;
                if arena.paintable_row_is_populated(slot) {
                    arena.note_visual_context_box_dirty(
                        slot,
                        crate::painting::visual_context::dirty::VisualContextBoxDirtyKind::StyleValueChange,
                    );
                    arena.push_paint_damage(slot, PaintDamage::SVG | PaintDamage::SCOPE_PREAMBLE);
                }
            }
        }
    }
    any_changed
}

/// `slot` if its row is live, or an invalid slot.
fn live_slot(arena: &impl PaintRead, slot: NodeSlotId) -> NodeSlotId {
    if arena.slot_is_live(slot) {
        slot
    } else {
        NodeSlotId::INVALID
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// both resolvers must answer synchronously with `context` for the row the slot names and only push
/// into the sink whose pointer they receive.
#[unsafe(no_mangle)]
unsafe extern "C" fn layout_arena_sync_svg_paint_resources(
    arena: *mut c_void,
    context: *mut c_void,
    resolve_filter: unsafe extern "C" fn(*mut c_void, NodeSlotId, *const c_void, *mut c_void) -> bool,
    resolve_paint_server: unsafe extern "C" fn(*mut c_void, NodeSlotId, bool, *mut c_void),
) -> bool {
    let _main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY, arena) };
    use crate::painting::svg_paint_resources::{PublishedSvgFilter, PublishedSvgPaintServer, SvgPaintResourceKind};
    // What to resolve of the enrolled resources, where they changed: each reaches the DOM, which the main thread does.
    // SAFETY: Guaranteed by the caller.
    let Some(requests) = (unsafe {
        crate::painting::owner_pass::run_held_pass(arena, (), |arena, ()| svg_paint_resource_requests(arena))
    }) else {
        return false;
    };
    let mut resolved = Vec::with_capacity(requests.len());
    for request in requests {
        match request {
            SvgPaintResourceRequest::PaintServer { slot, kind } => {
                let mut published = PublishedSvgPaintServer::None;
                // SAFETY: The host resolves synchronously for the row, and only pushes into the sink it is handed.
                unsafe {
                    resolve_paint_server(
                        context,
                        slot,
                        kind == SvgPaintResourceKind::Stroke,
                        (&raw mut published).cast(),
                    );
                }
                resolved.push(ResolvedSvgPaintResource::PaintServer { slot, kind, published });
            }
            SvgPaintResourceRequest::Filter { slot, kind, urls } => {
                let mut published = PublishedSvgFilter::default();
                for url in &urls {
                    let mut primitives: Vec<SvgFilterPrimitive> = Vec::new();
                    // SAFETY: The host resolves synchronously for the row, and only pushes into the primitive list
                    // it is handed as its sink.
                    let found =
                        unsafe { resolve_filter(context, slot, url.pointer().cast(), (&raw mut primitives).cast()) };
                    published = PublishedSvgFilter {
                        failed: !found,
                        primitives: if found { primitives } else { Vec::new() },
                    };
                    if published.failed {
                        break;
                    }
                }
                resolved.push(ResolvedSvgPaintResource::Filter { slot, kind, published });
            }
        }
    }
    // SAFETY: As above.
    unsafe {
        crate::painting::owner_pass::run_held_pass(arena, resolved, |arena, resolved| {
            publish_resolved_svg_paint_resources(arena, resolved)
        })
    }
}
