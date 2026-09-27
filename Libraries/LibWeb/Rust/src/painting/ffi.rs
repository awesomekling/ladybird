/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::css::css_pixels::{CssPixelPoint, CssPixelRect, CssPixels};
use crate::css::ffi_support::FfiUtf16View;
use crate::layout::LayoutNodeArena;
use crate::layout::node_data::NodeSlotId;
use crate::layout::svg_formatting_context;
use crate::layout::used_values::FfiCssPixelPoint;
use crate::layout::used_values::FfiCssPixelRect;
use crate::layout::used_values::FfiCssPixelSize;
use crate::painting::display_list::commands::{ContextRef, SpatialNodeIndex};
use crate::painting::filter_bytes::{FfiFilterFunction, filter_functions_graph};
use crate::painting::force_dark::ForceDarkRole;
use crate::painting::host::visual_context::FfiSvgFilterPrimitive;
use crate::painting::paintable_data::*;
use crate::painting::paintable_rows::{MainSidePaintableRows, PaintableRowsRead, with_inline_pieces};
use crate::painting::published_frame::PaintRead;
use crate::painting::rect_to_viewport_transform::RectToViewportTransform;
use crate::painting::scroll_chain::ViewportWheelOverflow;
use crate::painting::svg_filter::SvgFilterPrimitive;
use libcompositing_rust::ffi::{ffi_slice, tree_from_handle};
use libgfx_rust::filter::Filter;
use std::ffi::c_void;

mod main_thread_entries;

pub(crate) use main_thread_entries::MainThreadFfiEntry;

/// SAFETY: `arena` must be a live handle from `layout_arena_create`, borrowed for this call on
/// the document thread.
#[track_caller]
pub(crate) unsafe fn arena_from_handle<'a>(arena: *mut c_void) -> &'a LayoutNodeArena {
    crate::painting::seal::note_main_side_read(std::panic::Location::caller());
    unsafe { LayoutNodeArena::from_handle(arena) }
}

/// Like [`arena_from_handle`], for a read of the layout tree's styles and links that goes on beside a recording of the
/// arena in flight, which writes none of them (see [`LayoutNodeArena::from_handle_beside_recording`]).
///
/// SAFETY: As for [`arena_from_handle`], and the caller reads nothing a recording writes.
#[track_caller]
unsafe fn arena_from_handle_beside_recording<'a>(arena: *mut c_void) -> &'a LayoutNodeArena {
    crate::painting::seal::note_main_side_read(std::panic::Location::caller());
    unsafe { LayoutNodeArena::from_handle_beside_recording(arena) }
}

/// SAFETY: `arena` must be a live handle from `layout_arena_create`, exclusively borrowed for
/// this call on the document thread. No C++ callback may re-enter the arena during the borrow.
#[track_caller]
unsafe fn arena_from_handle_mut<'a>(arena: *mut c_void) -> &'a mut LayoutNodeArena {
    unsafe { LayoutNodeArena::from_handle_mut(arena) }
}

/// The main side's one door to the paintable rows. Between stages it reads the rows as last
/// committed; a host call made while a stage runs belongs to that stage and reads the rows the
/// stage is writing.
///
/// SAFETY: Same as [`arena_from_handle`]. Between stages, no other borrow of the arena may be live
/// while the view is.
#[track_caller]
unsafe fn main_side_paintable_rows<'a>(arena: *mut c_void) -> MainSidePaintableRows<'a> {
    if crate::stage_thread::reads_beside_recording_of(arena) {
        crate::painting::seal::note_main_side_read(std::panic::Location::caller());
        // SAFETY: The recording in flight owns the arena, but reads the rows its layout published and writes none of
        // them; the view reads nothing else the recording writes.
        return MainSidePaintableRows::Committed(unsafe { &*arena.cast::<LayoutNodeArena>() }.rows_beside_recording());
    }
    let shared = unsafe { arena_from_handle(arena) };
    if shared.a_stage_is_running() {
        return MainSidePaintableRows::DuringStage(shared.paintable_rows());
    }
    MainSidePaintableRows::Committed(unsafe { arena_from_handle_mut(arena) }.committed_paintable_rows())
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum ScrollDirection {
    #[default]
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct FfiChromeMetrics {
    pub scroll_thumb_min_length: CssPixels,
    pub scroll_thumb_padding_thin: CssPixels,
    pub scroll_thumb_thickness_thin: CssPixels,
    pub scroll_thumb_thickness: CssPixels,
    pub scroll_gutter_thickness: CssPixels,
    pub resize_gripper_size: CssPixels,
    pub resize_gripper_padding: CssPixels,
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FfiPhysicalResizeAxes {
    pub horizontal: bool,
    pub vertical: bool,
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FfiScrollbarData {
    pub gutter_rect: FfiCssPixelRect,
    pub thumb_rect: FfiCssPixelRect,
    pub track_rect: FfiCssPixelRect,
    pub thumb_travel_to_scroll_ratio: f64,
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FfiOptionalScrollbarData {
    pub has_value: bool,
    pub value: FfiScrollbarData,
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_set_scrollbar_enlarged(
    arena: *mut c_void,
    slot: NodeSlotId,
    direction: ScrollDirection,
    enlarged: bool,
) {
    let arena = unsafe { arena_from_handle_mut(arena) };
    let _write = arena.join_frame_for_main_side_write("scrollbar interaction");
    let mut rows = arena.paintable_rows_mut();
    if !rows.paintable_row_is_populated(slot) {
        return;
    }
    let flag = match direction {
        ScrollDirection::Horizontal => PaintableFlag::HorizontalScrollbarEnlarged,
        ScrollDirection::Vertical => PaintableFlag::VerticalScrollbarEnlarged,
    };
    if rows.paintable_data(slot).has_flag(flag) == enlarged {
        return;
    }
    rows.paintable_data_mut(slot).set_flag(flag, enlarged);
    use crate::painting::record::damage::PaintDamage;
    rows.push_paint_damage(slot, PaintDamage::DRAW_OVERLAY | PaintDamage::HIT_OVERLAY);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_physical_resize_axes(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiPhysicalResizeAxes {
    let arena = unsafe { arena_from_handle(arena) };
    let axes = crate::painting::chrome_geometry::physical_resize_axes(&arena.paintable_rows(), slot);
    FfiPhysicalResizeAxes {
        horizontal: axes.horizontal,
        vertical: axes.vertical,
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_is_chrome_mirrored(arena: *mut c_void, slot: NodeSlotId) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::chrome_geometry::is_chrome_mirrored(&arena.paintable_rows(), slot)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_compute_scrollbar_data(
    arena: *mut c_void,
    slot: NodeSlotId,
    direction: ScrollDirection,
    metrics: FfiChromeMetrics,
    viewport_overflow_x: u8,
    viewport_overflow_y: u8,
    enlarged: bool,
    has_device_scroll_offset: bool,
    device_scroll_offset: f32,
    device_pixels_per_css_pixel: f64,
) -> FfiOptionalScrollbarData {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let data = crate::painting::chrome_geometry::ChromeGeometry {
        arena: &paintable_rows,
        metrics,
        viewport_wheel_overflow_x: viewport_overflow_x,
        viewport_wheel_overflow_y: viewport_overflow_y,
    }
    .compute_scrollbar_data(
        slot,
        direction,
        enlarged,
        has_device_scroll_offset.then_some(crate::painting::chrome_geometry::ScrollbarScrollState {
            device_scroll_offset,
            device_pixels_per_css_pixel,
        }),
    );
    let Some(data) = data else {
        return FfiOptionalScrollbarData::default();
    };
    FfiOptionalScrollbarData {
        has_value: true,
        value: FfiScrollbarData {
            gutter_rect: data.gutter_rect.into(),
            thumb_rect: data.thumb_rect.into(),
            track_rect: data.track_rect.into(),
            thumb_travel_to_scroll_ratio: data.thumb_travel_to_scroll_ratio.to_double(),
        },
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_minimum_scroll_offset(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelPoint {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    crate::painting::chrome_geometry::minimum_scroll_offset(&paintable_rows, slot).into()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_maximum_scroll_offset(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelPoint {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    crate::painting::chrome_geometry::maximum_scroll_offset(&paintable_rows, slot).into()
}

fn scroll_offset_reader(paintable_rows: &impl PaintableRowsRead) -> impl Fn(NodeSlotId) -> CssPixelPoint {
    move |node| {
        if !paintable_rows.paintable_row_is_populated(node) {
            return CssPixelPoint::default();
        }
        paintable_rows.scroll_offset(node)
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_wheel_scrollable_axes(
    arena: *mut c_void,
    slot: NodeSlotId,
    viewport_overflow_x: u8,
    viewport_overflow_y: u8,
) -> FfiPhysicalResizeAxes {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let axes = crate::painting::chrome_geometry::wheel_scrollable_axes(
        &paintable_rows,
        slot,
        viewport_overflow_x,
        viewport_overflow_y,
    );
    FfiPhysicalResizeAxes {
        horizontal: axes.horizontal,
        vertical: axes.vertical,
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_chrome_state_callback(
    arena: *mut c_void,
    context: *mut c_void,
    callback: unsafe extern "C" fn(*mut c_void, NodeSlotId, PaintableRowResetKind, bool),
) {
    unsafe { crate::layout::HostTables::from_handle(arena) }
        .chrome_state_callback
        .set(Some((context, callback)));
    unsafe { arena_from_handle(arena) }.set_chrome_state_listens(true);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_clear_chrome_state_callback(arena: *mut c_void) {
    unsafe { crate::layout::HostTables::from_handle(arena) }
        .chrome_state_callback
        .set(None);
    unsafe { arena_from_handle(arena) }.set_chrome_state_listens(false);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_establishes_an_absolute_positioning_containing_block(
    arena: *mut c_void,
    node: NodeSlotId,
) -> bool {
    let arena = unsafe { arena_from_handle_beside_recording(arena) };
    crate::painting::style_queries::establishes_positioning_containing_blocks(arena, node).0
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_establishes_a_fixed_positioning_containing_block(
    arena: *mut c_void,
    node: NodeSlotId,
) -> bool {
    let arena = unsafe { arena_from_handle_beside_recording(arena) };
    crate::painting::style_queries::establishes_positioning_containing_blocks(arena, node).1
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_any_ancestor_establishes_a_fixed_position_containing_block(
    arena: *mut c_void,
    node: NodeSlotId,
) -> bool {
    let arena = unsafe { arena_from_handle_beside_recording(arena) };
    crate::painting::style_queries::any_ancestor_establishes_a_fixed_position_containing_block(arena, node)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_has_css_transform(arena: *mut c_void, node: NodeSlotId) -> bool {
    let arena = unsafe { arena_from_handle_beside_recording(arena) };
    let Some(style) = arena.node_style_if_live(node) else {
        return false;
    };
    crate::painting::style_queries::has_css_transform(arena, node, style)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `node` must name a live node in this arena.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_invalidate_nearest_self_painting_inline_paint_cache(
    arena: *mut c_void,
    node: NodeSlotId,
    stage: u8,
) {
    let arena = unsafe { arena_from_handle(arena) };
    let _writer = crate::painting::published_immutable::enter_writer(match stage {
        0 => "journal drain",
        1 => "anonymous row invalidation",
        _ => "unknown repaint damage stage",
    });
    if let Some(ancestor) =
        crate::painting::fragment_ownership::nearest_self_painting_inline_box(&arena.paintable_rows(), node)
    {
        use crate::painting::record::damage::PaintDamage;
        arena.push_paint_damage(ancestor, PaintDamage::ALL_DRAW | PaintDamage::ALL_HIT);
    }
}

/// The committed row of the document element, as the last rendering preparation published it, or
/// an invalid slot when the document has no root element or the root holds no row.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_published_root_element_row(arena: *mut c_void) -> NodeSlotId {
    let arena = unsafe { arena_from_handle(arena) };
    let slot = arena
        .paint_state()
        .borrow()
        .root_background_source
        .map_or(NodeSlotId::INVALID, |source| source.root_layout_node);
    if !arena.paintable_rows().paintable_row_is_populated(slot) {
        return NodeSlotId::INVALID;
    }
    slot
}

/// The fields of a committed paintable row that C++ reads, copied out through
/// [`main_side_paintable_rows`]. `is_populated` is false, and the rest default, when the slot has
/// no committed box.
#[derive(Default)]
#[repr(C)]
pub struct FfiCommittedRow {
    pub is_populated: bool,
    pub establishes_stacking_context: bool,
    pub has_accumulated_visual_context: bool,
    pub accumulated_visual_context: ContextRef,
    pub accumulated_visual_context_for_descendants: ContextRef,
    pub enclosing_scroll_node_index: SpatialNodeIndex,
    pub own_scroll_node_index: SpatialNodeIndex,
}

/// Whether `slot` has a committed box, which is all most main-side callers ask before they touch
/// its row. Publishing the rows, and measuring the overflow they are published with, changes no
/// row's population, so the live rows answer this without the publication
/// [`layout_arena_committed_row`] makes.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_has_committed_box(arena: *mut c_void, slot: NodeSlotId) -> bool {
    if crate::stage_thread::reads_beside_recording_of(arena) {
        crate::painting::seal::note_main_side_read(std::panic::Location::caller());
        // SAFETY: As in `main_side_paintable_rows`, the recording in flight writes none of the rows it published.
        return unsafe { &*arena.cast::<LayoutNodeArena>() }
            .rows_beside_recording()
            .paintable_row_is_populated(slot);
    }
    unsafe { arena_from_handle(arena) }
        .paintable_rows()
        .paintable_row_is_populated(slot)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_committed_row(arena: *mut c_void, slot: NodeSlotId) -> FfiCommittedRow {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(slot) {
        return FfiCommittedRow::default();
    }
    let row = paintable_rows.paintable_data(slot);
    FfiCommittedRow {
        is_populated: true,
        establishes_stacking_context: row.establishes_stacking_context,
        has_accumulated_visual_context: row.has_accumulated_visual_context,
        accumulated_visual_context: row.accumulated_visual_context,
        accumulated_visual_context_for_descendants: row.accumulated_visual_context_for_descendants,
        enclosing_scroll_node_index: row.enclosing_scroll_node_index,
        own_scroll_node_index: row.own_scroll_node_index,
    }
}

/// Clears the paint state of `layout_node`'s row, whose box is going away, and hands back its
/// reset.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
pub(crate) unsafe fn clear_paintable_row_of_node(arena: *mut c_void, layout_node: NodeSlotId) {
    let reset = {
        let arena = unsafe { arena_from_handle(arena) };
        crate::painting::published_immutable::note_row_mutation(
            arena,
            layout_node,
            "M1 layout_arena_paintable_cleared_from_node",
        );
        arena.clear_committed_fragment_link(layout_node);
        arena.prepare_paintable_row_cleared_reset(layout_node)
    };
    if let Some(reset) = reset {
        unsafe { arena_from_handle(arena) }.hand_back_paintable_row_reset(reset);
        let arena = unsafe { arena_from_handle_mut(arena) };
        arena.paintable_row_cleared(reset);
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_has_child_paintables(arena: *mut c_void, slot: NodeSlotId) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::paint_order::first_paint_child(&arena.paintable_rows(), slot).is_some()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `snapshot` must address a snapshot whose nodes are valid for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_selection_apply_snapshot(
    arena: *mut c_void,
    snapshot: *const FfiSelectionSnapshot,
) {
    let arena = unsafe { arena_from_handle_mut(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::SELECTION_WRITER);
    // SAFETY: Guaranteed by the caller.
    let snapshot = unsafe { crate::painting::selection::SelectionSnapshot::from_ffi(&*snapshot) };
    snapshot.apply(arena);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_selection_clear(arena: *mut c_void, viewport: NodeSlotId) {
    let arena = unsafe { arena_from_handle_mut(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::SELECTION_WRITER);
    if !arena.paintable_row_is_populated(viewport) {
        return;
    }
    crate::painting::selection::clear(&mut arena.paintable_rows_mut(), viewport);
}

#[repr(C)]
pub struct FfiPhysicalOverflowDirections {
    pub horizontal_axis_is_positive: bool,
    pub vertical_axis_is_positive: bool,
}

/// # Safety
///
/// `arena` must be a live arena on the document thread. The callbacks must remain
/// valid until it is destroyed and must not mutate layout geometry.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_geometry_host(
    arena: *mut c_void,
    host: crate::painting::host::FfiGeometryHostCallbacks,
) {
    unsafe { crate::layout::HostTables::from_handle(arena) }
        .geometry_host
        .set(Some(host.into()));
}

/// # Safety
///
/// `arena` must be a live arena used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_scrollable_overflow(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiOptionalOverflowData {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let Some(rect) = crate::painting::paintable_geometry::scrollable_overflow_rect(&paintable_rows, slot) else {
        return FfiOptionalOverflowData::default();
    };
    let mut value = paintable_rows
        .committed_side_data(slot)
        .overflow_relative_to_padding_box;
    value.rect = rect.into();
    FfiOptionalOverflowData { has_value: true, value }
}

#[derive(Default)]
#[repr(C)]
pub struct FfiRenderingPreparationOutcome {
    pub requires_display_list_recording: bool,
    pub requires_visual_context_update: bool,
    pub visual_context_values_changed: bool,
}

/// The render half of preparing for rendering ahead of the scroll offset handover: the root
/// background source is updated, and the overflow left unmeasured is measured, which answers with
/// whether the source changed and the scroll offsets the document is to store in place of the ones
/// the measured boxes now store out of range.
///
/// The overflow recalculation is a pass of its own, and the document stores the scroll offsets it
/// settled only once that pass is over. It measures all overflow left unmeasured, including the
/// root's: the root background covers the viewport united with it, so a flip in its scrollability
/// is seen here rather than while recording holds the paint state.
pub(crate) fn prepare_root_background_and_overflow(
    arena: &LayoutNodeArena,
    root_background_source: crate::painting::host::FfiRootBackgroundSource,
) -> (bool, Vec<(NodeSlotId, CssPixelPoint)>) {
    let background_source_changed = {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::VisualContextUpdate);
        arena
            .paint_state()
            .borrow_mut()
            .update_root_background_source(arena, root_background_source)
    };
    let clamped = crate::painting::scrollable_overflow::measure_and_find_scroll_offsets_to_clamp(arena);
    (background_source_changed, clamped)
}

/// The render half of preparing for rendering after the scroll offset handover: takes what the
/// overflow measurement changed, and refreshes the sticky constraints the changed geometry moves
/// unless a visual context update is pending anyway. It is a visual context update of its own, which
/// starts after the handover so the handover is not inside a pass.
pub(crate) fn finish_rendering_preparation(
    arena: &LayoutNodeArena,
    background_source_changed: bool,
    visual_context_update_pending: bool,
) -> FfiRenderingPreparationOutcome {
    let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::VisualContextUpdate);
    let changed = arena.scrollable_overflow.geometry_changed.replace(false);
    let flipped = arena.scrollable_overflow.scrollability_changed.replace(false);
    let mut visual_context_values_changed = false;
    if changed && !flipped && !visual_context_update_pending {
        let rows = arena.paintable_rows();
        let mut state = arena.paint_state().borrow_mut();
        let state = &mut state.visual_context;
        if let Some(tree) = state.tree.as_mut() {
            visual_context_values_changed = crate::painting::visual_context::refresh::refresh_sticky_constraints(
                &rows,
                &state.scroll_state,
                tree,
                &arena.visual_context_tree_inputs(),
            );
        }
        state.needs_to_refresh_scroll_state = true;
    }
    FfiRenderingPreparationOutcome {
        requires_display_list_recording: changed || background_source_changed,
        requires_visual_context_update: flipped,
        visual_context_values_changed,
    }
}

/// # Safety
///
/// `arena` must be a live arena used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_scrollable_overflow_recalculation_count(arena: *mut c_void, reset: bool) -> u64 {
    let state = &unsafe { arena_from_handle(arena) }.scrollable_overflow;
    if reset {
        state.recalculations.replace(0)
    } else {
        state.recalculations.get()
    }
}

#[derive(Default)]
#[repr(C)]
pub struct FfiOptionalOverflowData {
    pub has_value: bool,
    pub value: crate::painting::paintable_data::FfiOverflowData,
}

#[repr(C)]
#[derive(Default)]
pub struct FfiBoxModelMetrics {
    pub margin: crate::painting::paintable_data::FfiPixelBox,
    pub padding: crate::painting::paintable_data::FfiPixelBox,
    pub border: crate::painting::paintable_data::FfiPixelBox,
    pub inset: crate::painting::paintable_data::FfiPixelBox,
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_content_size(arena: *mut c_void, slot: NodeSlotId) -> FfiCssPixelSize {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(slot) {
        return FfiCssPixelSize::default();
    }
    crate::painting::paintable_geometry::committed_content_size(&paintable_rows, slot)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_svg_viewport_size(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelSize {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(slot) {
        return FfiCssPixelSize::default();
    }
    crate::painting::paintable_geometry::committed_svg_viewport_size(&paintable_rows, slot)
}

#[repr(C)]
pub struct FfiOptionalAffineTransform {
    pub has_value: bool,
    pub transform: svg_formatting_context::FfiAffineTransform,
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_svg_viewport_transform(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiOptionalAffineTransform {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let transform = if paintable_rows.paintable_row_is_populated(slot) {
        crate::painting::paintable_geometry::committed_svg_viewport_transform(&paintable_rows, slot)
    } else {
        None
    };
    FfiOptionalAffineTransform {
        has_value: transform.is_some(),
        transform: transform.unwrap_or_default(),
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_transform_reference_box(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelRect {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(slot) {
        return FfiCssPixelRect::default();
    }
    let Some(style) = paintable_rows.node_style_if_live(slot) else {
        return FfiCssPixelRect::default();
    };
    crate::painting::visual_context::node_values::transform_reference_box(style, &paintable_rows, slot).into()
}

/// # Safety
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_box_model(arena: *mut c_void, slot: NodeSlotId) -> FfiBoxModelMetrics {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(slot) {
        return FfiBoxModelMetrics::default();
    }
    FfiBoxModelMetrics {
        margin: crate::painting::paintable_geometry::committed_margin(&paintable_rows, slot),
        padding: crate::painting::paintable_geometry::committed_padding(&paintable_rows, slot),
        border: crate::painting::paintable_geometry::committed_border(&paintable_rows, slot),
        inset: crate::painting::paintable_geometry::committed_inset(&paintable_rows, slot),
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_is_positioned(arena: *mut c_void, slot: NodeSlotId) -> bool {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(slot) {
        return false;
    }
    crate::painting::style_queries::is_positioned(&paintable_rows, slot)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_absolute_rect(arena: *mut c_void, slot: NodeSlotId) -> FfiCssPixelRect {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    crate::painting::paintable_geometry::absolute_rect_or_default(&paintable_rows, slot).into()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_absolute_padding_box_rect(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelRect {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(slot) {
        return FfiCssPixelRect::default();
    }
    crate::painting::paintable_geometry::absolute_padding_box_rect(&paintable_rows, slot).into()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_absolute_border_box_rect(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelRect {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(slot) {
        return FfiCssPixelRect::default();
    }
    crate::painting::paintable_geometry::absolute_border_box_rect(&paintable_rows, slot).into()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_physical_overflow_directions(
    arena: *mut c_void,
    paintable: NodeSlotId,
) -> FfiPhysicalOverflowDirections {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let directions = if paintable_rows.paintable_row_is_populated(paintable) {
        crate::painting::scrollable_overflow::physical_overflow_directions(&paintable_rows, paintable)
    } else {
        crate::painting::scrollable_overflow::PhysicalOverflowDirections::default()
    };
    FfiPhysicalOverflowDirections {
        horizontal_axis_is_positive: directions.horizontal_axis_is_positive,
        vertical_axis_is_positive: directions.vertical_axis_is_positive,
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_context_note_box_dirty(
    arena: *mut c_void,
    slot: NodeSlotId,
    kind: crate::painting::host::FfiVisualContextBoxDirtyKind,
) {
    use crate::painting::host::FfiVisualContextBoxDirtyKind;
    use crate::painting::visual_context::dirty::VisualContextBoxDirtyKind;
    let arena = unsafe { arena_from_handle(arena) };
    if !arena.paintable_row_is_populated(slot) {
        return;
    }
    let kind = match kind {
        FfiVisualContextBoxDirtyKind::StyleValueChange => VisualContextBoxDirtyKind::StyleValueChange,
        FfiVisualContextBoxDirtyKind::StyleStructuralChange => VisualContextBoxDirtyKind::StyleStructuralChange,
        FfiVisualContextBoxDirtyKind::ScrollableOverflowFlipped => VisualContextBoxDirtyKind::ScrollableOverflowFlipped,
    };
    arena.note_visual_context_box_dirty(slot, kind);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_context_request_full_rebuild(
    arena: *mut c_void,
    reason: crate::painting::host::FfiVisualContextGlobalRebuildReason,
) {
    use crate::painting::host::FfiVisualContextGlobalRebuildReason;
    use crate::painting::visual_context::dirty::VisualContextGlobalRebuildReason;
    let arena = unsafe { arena_from_handle(arena) };
    let reason = match reason {
        FfiVisualContextGlobalRebuildReason::FirstBuild => VisualContextGlobalRebuildReason::FirstBuild,
        FfiVisualContextGlobalRebuildReason::DocumentWideStructuralChange => {
            VisualContextGlobalRebuildReason::DocumentWideStructuralChange
        }
        FfiVisualContextGlobalRebuildReason::FilterResourcesChanged => {
            VisualContextGlobalRebuildReason::FilterResourcesChanged
        }
        FfiVisualContextGlobalRebuildReason::ForcedForTesting => VisualContextGlobalRebuildReason::ForcedForTesting,
        FfiVisualContextGlobalRebuildReason::CanonicalDumpRequested => {
            VisualContextGlobalRebuildReason::CanonicalDumpRequested
        }
    };
    arena.request_full_visual_context_rebuild(reason);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_context_pending_dirty_box_count(arena: *mut c_void) -> usize {
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state.visual_context.dirty_boxes.boxes.len()
}

/// # Safety
///
/// `arena` must be a live layout arena handle used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_background_color_can_be_compositor_animated(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::record::paint::background_resolution::background_color_can_be_compositor_animated(
        &arena.paintable_rows(),
        slot,
        crate::layout::root_background_source(arena),
    )
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_visual_context_node_count(
    arena: *mut c_void,
    slot: NodeSlotId,
    list: crate::painting::host::FfiVisualContextBoxNodeList,
) -> usize {
    use crate::painting::host::FfiVisualContextBoxNodeList;
    let arena = unsafe { arena_from_handle(arena) };
    arena.with_paintable_visual_context_node_handles(slot, |handles| match list {
        FfiVisualContextBoxNodeList::SpatialNodes => handles.spatial.len(),
        FfiVisualContextBoxNodeList::ClipNodes => handles.clip_handles().count(),
        FfiVisualContextBoxNodeList::EffectNodes => handles.effects.len(),
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread; `out`
/// must have room for `capacity` indices.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_visual_context_copy_node_indices(
    arena: *mut c_void,
    slot: NodeSlotId,
    list: crate::painting::host::FfiVisualContextBoxNodeList,
    out: *mut u32,
    capacity: usize,
) {
    use crate::painting::host::FfiVisualContextBoxNodeList;
    let arena = unsafe { arena_from_handle(arena) };
    arena.with_paintable_visual_context_node_handles(slot, |handles| {
        let indices: Vec<u32> = match list {
            FfiVisualContextBoxNodeList::SpatialNodes => handles.spatial.iter().map(|index| index.0).collect(),
            FfiVisualContextBoxNodeList::ClipNodes => handles.clip_handles().map(|index| index.0).collect(),
            FfiVisualContextBoxNodeList::EffectNodes => handles.effects.iter().map(|index| index.0).collect(),
        };
        assert!(indices.len() <= capacity);
        // SAFETY: the caller warrants `capacity` writable indices behind `out`.
        unsafe { std::ptr::copy_nonoverlapping(indices.as_ptr(), out, indices.len()) };
    });
}

fn apply_walk_assignments(
    arena: &mut crate::layout::LayoutNodeArena,
    viewport: NodeSlotId,
    outcome: &mut crate::painting::visual_context::incremental::IncrementalUpdateOutcome,
    state: &mut crate::painting::visual_context::VisualContextState,
) {
    {
        let mut paintable_rows = arena.paintable_rows_mut();
        for assignment in std::mem::take(&mut outcome.assignments) {
            assignment.apply(&mut paintable_rows);
        }
    }
    if outcome.mask_node_owners_changed {
        state.paintables_with_mask_nodes = paintables_with_mask_nodes_in_paint_order(arena, viewport);
    }
}

fn fresh_visual_context_tree_build(
    arena: &mut crate::layout::LayoutNodeArena,
    viewport: NodeSlotId,
    inputs: crate::painting::host::FfiVisualContextTreeInputs,
    state: &mut crate::painting::visual_context::VisualContextState,
) -> crate::painting::host::FfiVisualContextUpdateOutcome {
    use crate::painting::visual_context::dirty::VisualContextUpdateScope;
    use crate::painting::visual_context::incremental::{
        IncrementalUpdateResult, debug_assert_every_live_node_is_owned, update_visual_context_tree,
    };
    let fresh_tree = {
        let arena = &*arena;
        let paintable_rows = arena.paintable_rows();
        let mut fresh_tree = crate::painting::visual_context::build::create_fresh_tree_with_viewport_nodes(
            &paintable_rows,
            viewport,
            &inputs,
        );
        fresh_tree.viewport_assignment.node_identity = paintable_rows.unique_node_id(viewport);
        fresh_tree
    };
    {
        let arena = &mut *arena;
        let mut paintable_rows = arena.paintable_rows_mut();
        paintable_rows.drop_all_visual_context_records();
        fresh_tree.viewport_assignment.apply(&mut paintable_rows);
    }
    state.tree = Some(std::sync::Arc::new(fresh_tree.tree));
    state.dirty_boxes.clear();
    state.build_count += 1;
    let mut outcome = {
        let arena = &*arena;
        let paintable_rows = arena.paintable_rows();
        match update_visual_context_tree(
            &paintable_rows,
            viewport,
            inputs,
            VisualContextUpdateScope::FreshTree,
            state,
        ) {
            IncrementalUpdateResult::Applied(outcome) => *outcome,
            IncrementalUpdateResult::NeedsFullBuild(_) => {
                unreachable!("a fresh tree walk has a tree and a viewport record")
            }
        }
    };
    let arena = &mut *arena;
    outcome.mask_node_owners_changed = true;
    // Everything records again; pushing that first keeps the per-row pushes below free.
    arena.push_all_paint_damage();
    apply_walk_assignments(arena, viewport, &mut outcome, state);
    arena.rebuild_all_stacking_context_entries_from_records(viewport);
    arena.take_line_roots_needing_fragment_ownership();
    crate::painting::fragment_ownership::assign_fragment_ownership(&arena.paintable_rows(), viewport);
    state.quarantined_slots_are_releasable = false;
    debug_assert_every_live_node_is_owned(
        &arena.paintable_rows(),
        state.tree.as_deref().expect("a fresh tree walk keeps the tree"),
        viewport,
    );
    crate::painting::host::FfiVisualContextUpdateOutcome {
        performed_full_build: true,
        structural_epoch_changed: true,
        requires_display_list_recording: true,
        structural_epoch: state.structural_epoch(),
        tree_changed: true,
    }
}

fn paintables_with_mask_nodes_in_paint_order(
    arena: &crate::layout::LayoutNodeArena,
    viewport: NodeSlotId,
) -> Vec<NodeSlotId> {
    let paintable_rows = arena.paintable_rows();
    let mut owners = Vec::new();
    crate::painting::paint_order::for_each_in_paint_subtree(&paintable_rows, viewport, |slot| {
        if arena
            .paintable_visual_context_record(slot)
            .is_some_and(|record| record.has_mask_nodes)
        {
            owners.push(slot);
        }
    });
    owners
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_update_accumulated_visual_contexts(
    arena: *mut c_void,
    viewport: NodeSlotId,
) -> crate::painting::host::FfiVisualContextUpdateOutcome {
    let arena = unsafe { arena_from_handle_mut(arena) };
    if !arena.paintable_row_is_populated(viewport) {
        return crate::painting::host::FfiVisualContextUpdateOutcome::default();
    }
    arena.run_stage(|arena| update_accumulated_visual_contexts_stage(arena, viewport))
}

/// The visual context update, run on the render stage right before the rows it leaves are
/// published.
fn update_accumulated_visual_contexts_stage(
    arena: &mut crate::layout::LayoutNodeArena,
    viewport: NodeSlotId,
) -> crate::painting::host::FfiVisualContextUpdateOutcome {
    use crate::painting::visual_context::dirty::{VisualContextGlobalRebuildReason, VisualContextUpdateScope};
    use crate::painting::visual_context::incremental::{
        IncrementalUpdateResult, debug_assert_every_live_node_is_owned, update_visual_context_tree,
    };
    arena.release_published_paintable_rows();
    let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::VisualContextUpdate);
    let inputs = arena.visual_context_tree_inputs();
    let mut state = std::mem::take(&mut arena.paint_state().borrow_mut().visual_context);
    state.release_quarantined_slots_while_no_handle_is_retained();

    let mut reason = state.dirty_boxes.global_reason;
    if state.tree.is_none() {
        reason = reason.max(VisualContextGlobalRebuildReason::FirstBuild);
    }
    if state.last_tree_inputs.is_some_and(|last| {
        last.device_pixels_per_css_pixel != inputs.device_pixels_per_css_pixel
            || last.viewport_wheel_overflow_x != inputs.viewport_wheel_overflow_x
            || last.viewport_wheel_overflow_y != inputs.viewport_wheel_overflow_y
    }) {
        reason = reason.max(VisualContextGlobalRebuildReason::TreeInputsChanged);
    }
    if state.tree.as_deref().is_some_and(|tree| tree.should_compact()) {
        reason = reason.max(VisualContextGlobalRebuildReason::Compaction);
    }

    loop {
        let scope = VisualContextUpdateScope::for_reason(reason);
        if scope == VisualContextUpdateScope::FreshTree {
            break;
        }
        let result = {
            let paintable_rows = arena.paintable_rows();
            update_visual_context_tree(&paintable_rows, viewport, inputs, scope, &mut state)
        };
        match result {
            IncrementalUpdateResult::Applied(mut outcome) => {
                apply_walk_assignments(arena, viewport, &mut outcome, &mut state);
                arena.resort_stacking_context_entries_flagged_for_resort();
                crate::painting::fragment_ownership::assign_fragment_ownership_for_pending_line_roots(arena);
                let performed_full_build = scope == VisualContextUpdateScope::EveryBox;
                if performed_full_build {
                    state.build_count += 1;
                    state.last_full_build_reason = reason;
                    debug_assert_every_live_node_is_owned(
                        &arena.paintable_rows(),
                        state.tree.as_deref().expect("an applied walk keeps the tree"),
                        viewport,
                    );
                } else {
                    state.incremental_update_count += 1;
                }
                let structural_epoch_changed = outcome.delta.structural_epoch_changed;
                let requires_display_list_recording = outcome.delta.requires_display_list_recording;
                let tree_changed = outcome.delta.payload_changed
                    || structural_epoch_changed
                    || outcome.delta.tombstoned_any_node
                    || requires_display_list_recording;
                state.dirty_boxes.clear();
                state.last_tree_inputs = Some(inputs);
                let structural_epoch = state.structural_epoch();
                arena.paint_state().borrow_mut().visual_context = state;
                arena.publish_paintable_rows();
                return crate::painting::host::FfiVisualContextUpdateOutcome {
                    performed_full_build,
                    structural_epoch_changed,
                    requires_display_list_recording,
                    structural_epoch,
                    tree_changed,
                };
            }
            IncrementalUpdateResult::NeedsFullBuild(fallback_reason) => {
                assert!(
                    VisualContextUpdateScope::for_reason(fallback_reason) > scope,
                    "a fallback widens the update scope"
                );
                reason = fallback_reason;
            }
        }
    }

    state.last_full_build_reason = reason;
    let outcome = fresh_visual_context_tree_build(arena, viewport, inputs, &mut state);
    state.last_tree_inputs = Some(inputs);
    arena.paint_state().borrow_mut().visual_context = state;
    arena.publish_paintable_rows();
    outcome
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
/// `out_geometry` must point to writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_snap_container_geometry(
    arena: *mut c_void,
    snap_container: NodeSlotId,
    out_geometry: *mut crate::painting::host::FfiSnapContainerGeometry,
) -> bool {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let Some(geometry) = crate::painting::scroll_snap::snap_container_geometry(&paintable_rows, snap_container) else {
        return false;
    };
    // SAFETY: The caller provides writable storage for the geometry.
    unsafe { *out_geometry = geometry };
    true
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_scroll_snapport_rect(
    arena: *mut c_void,
    snap_container: NodeSlotId,
    scrollport: FfiCssPixelRect,
) -> FfiCssPixelRect {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::scroll_snap::scroll_snapport_rect(arena, snap_container, scrollport.into()).into()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_update_visual_viewport_transform(arena: *mut c_void) -> bool {
    let arena = unsafe { arena_from_handle_mut(arena) };
    arena.run_stage(|arena| {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::VisualContextUpdate);
        let mut paint_state = arena.paint_state().borrow_mut();
        let Some(tree) = &mut paint_state.visual_context.tree else {
            return false;
        };
        let inputs = arena.visual_context_tree_inputs();
        std::sync::Arc::make_mut(tree).set_visual_viewport_transform(
            crate::painting::visual_context::node_values::visual_viewport_transform_data(&inputs),
        );
        true
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_invalidate_scroll_state(arena: *mut c_void) {
    let arena = unsafe { arena_from_handle(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::SCROLL_OFFSETS_WRITER);
    arena
        .paint_state()
        .borrow_mut()
        .visual_context
        .needs_to_refresh_scroll_state = true;
}

/// The index of the sticky node the accumulated visual context tree holds for `paintable`, which
/// is where the scroll state snapshot keeps its resolved sticky offset, or `u32::MAX` when the tree
/// holds none.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_sticky_spatial_node_index(arena: *mut c_void, paintable: NodeSlotId) -> u32 {
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state
        .visual_context
        .scroll_state
        .states
        .iter()
        .find(|state| state.is_sticky && state.paintable == paintable)
        .map_or(u32::MAX, |state| state.node_index.0)
}

/// Re-reads the scroll containers' offsets when something invalidated them since the last
/// refresh, resolves the sticky nodes' offsets on top of them, and hands the dense device-pixel
/// snapshot to `publish`. Returns whether that happened, so the caller keeps its copy otherwise;
/// `force` re-derives the snapshot even when nothing invalidated it, for verification.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `publish` is called synchronously with `sink` and a view of the snapshot that is valid only
/// for the duration of that call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_refresh_scroll_state(
    arena: *mut c_void,
    force: bool,
    sink: *mut c_void,
    publish: unsafe extern "C" fn(*mut c_void, *const libgfx_rust::FloatPoint, usize),
) -> bool {
    let arena = unsafe { arena_from_handle_mut(arena) };
    let refresh = arena.run_stage(|arena| {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::ScrollStateRefresh);
        let paintable_rows = arena.paintable_rows();
        let mut paint_state = arena.paint_state().borrow_mut();
        let state = &mut paint_state.visual_context;
        if !force && !state.needs_to_refresh_scroll_state {
            return None;
        }
        state.needs_to_refresh_scroll_state = false;
        crate::painting::visual_context::refresh::refresh_scroll_state(&paintable_rows, &mut state.scroll_state);
        let mut snapshot = state
            .scroll_state
            .snapshot(arena.visual_context_tree_inputs().device_pixels_per_css_pixel);
        // https://drafts.csswg.org/css-position/#sticky-pos
        if let Some(tree) = state.tree.as_deref() {
            tree.resolve_sticky_offsets_in_place(&mut snapshot);
        }
        Some(snapshot)
    });
    let Some(snapshot) = refresh else {
        return false;
    };
    // SAFETY: The C++ sink copies the offsets synchronously.
    unsafe { publish(sink, snapshot.as_ptr(), snapshot.len()) };
    true
}

/// What a recording stage runs on: the frame it records, frozen before the stage is handed the
/// input and moved into it, and the recorder state it records with, which the document hands it
/// for the run. It holds no arena, so the stage reaches nothing the document goes on writing.
struct RecordingStageInput<'a> {
    frame: std::sync::Arc<crate::painting::published_frame::PublishedFrame>,
    recorder: crate::painting::record::recorder_state::RecorderState,
    viewport: NodeSlotId,
    inputs: crate::painting::record::RecordingInputs<'a>,
    trace_recordings: bool,
}

struct RecordingStageOutput {
    recording: crate::painting::record::RecordingResult,
    recording_from_scratch: Option<crate::painting::record::RecordingResult>,
    recorder: crate::painting::record::recorder_state::RecorderState,
    svg_paint_resources: std::sync::Arc<crate::painting::svg_paint_resources::SvgPaintResourceRows>,
}

const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<RecordingStageInput<'static>>();
    assert_send::<RecordingStageOutput>();
};

/// Prepares a recording on the thread that owns the arena: freezes the frame it records and takes
/// the recorder state it records with.
fn recording_stage_input<'a>(
    arena: &mut LayoutNodeArena,
    viewport: NodeSlotId,
    inputs: crate::painting::record::RecordingInputs<'a>,
) -> RecordingStageInput<'a> {
    let frame = freeze_recording_frame(arena, &inputs);
    RecordingStageInput {
        frame,
        recorder: arena.recording().take_recorder(),
        viewport,
        inputs,
        trace_recordings: arena.paint_state().borrow().trace_recordings,
    }
}

/// Freezes the frame a recording with `inputs` reads, on the thread that owns the arena, before the
/// recording is handed to the stage that runs it.
fn freeze_recording_frame(
    arena: &mut LayoutNodeArena,
    inputs: &crate::painting::record::RecordingInputs<'_>,
) -> std::sync::Arc<crate::painting::published_frame::PublishedFrame> {
    {
        let mut recording = arena.recording();
        // The root background paints the union of the viewport and the root's overflow, so it
        // is the one output a viewport move can change. Drop its caches before recording
        // starts instead of treating the viewport position as a frame-wide input.
        if let Some(source) = &recording.recorder().published_recording {
            let root = inputs.uncaptured.root_background_source.root_layout_node;
            let rows = arena.paintable_rows();
            let canvas_rect = crate::painting::record::paint::background_resolution::root_background_canvas_rect(
                &rows,
                root,
                inputs.css_viewport_rect,
            );
            if canvas_rect != source.root_background_canvas_rect {
                let _writer = crate::painting::published_immutable::enter_writer_if_unattributed("recording preflight");
                arena.push_paint_damage(root, crate::painting::record::damage::PaintDamage::DRAW_BACKGROUND);
            }
        }
    }
    // Damage pushed from here on is for the next recording to consume.
    if inputs.publishes_recording {
        arena.note_publishing_paint_recording_started();
    }
    std::sync::Arc::new(arena.freeze_paint_frame())
}

/// The host-free display-list recording stage. Host callbacks require a `MainThread` capability,
/// which this function neither receives nor stores in its input, and it names no arena.
fn record_display_list_stage(stage: RecordingStageInput<'_>) -> RecordingStageOutput {
    let RecordingStageInput {
        frame,
        mut recorder,
        viewport,
        mut inputs,
        trace_recordings,
    } = stage;
    let crate::painting::record::recorder_state::RecorderState {
        published_recording,
        published_hit_test_items,
        paint_order_tree,
        scratch,
    } = &mut recorder;
    // The retained tree describes the published tape and is written in place while a frame
    // is assembled, so only a recording that publishes may copy from that frame or touch
    // the tree; any other recording records from scratch into a tree of its own.
    let mut throwaway_tree = crate::painting::record::order_tree::PaintOrderTree::default();
    let (tree, source_frame, source_items) = if inputs.publishes_recording {
        (
            paint_order_tree,
            published_recording.clone(),
            published_hit_test_items.clone(),
        )
    } else {
        (&mut throwaway_tree, None, None)
    };
    let copies_from_published_frame = source_frame.is_some();
    let pass = crate::painting::seal::enter(crate::painting::seal::Pass::Recording);
    crate::stage_thread::hold_here(crate::stage_thread::FfiStageHoldPoint::MidRecording);
    let recording = crate::painting::record::traversal::record_display_list(
        &frame,
        scratch,
        tree,
        viewport,
        &inputs,
        source_frame,
        source_items,
        true,
        trace_recordings || crate::painting::record::verify::enabled_by_environment(),
    );
    // The oracle records the same frame from scratch into a throwaway tree whenever the
    // published frame could have been copied from.
    let recording_from_scratch =
        (crate::painting::record::verify::enabled_by_environment() && copies_from_published_frame).then(|| {
            inputs.publishes_recording = false;
            let mut tree_for_recording_from_scratch = crate::painting::record::order_tree::PaintOrderTree::default();
            crate::painting::record::traversal::record_display_list(
                &frame,
                scratch,
                &mut tree_for_recording_from_scratch,
                viewport,
                &inputs,
                None,
                None,
                false,
                false,
            )
        });
    drop(pass);
    RecordingStageOutput {
        recording,
        recording_from_scratch,
        recorder,
        svg_paint_resources: frame.svg_paint_resources().clone(),
    }
}

/// When the render side runs a display list recording the host has prepared.
#[repr(u32)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FfiRecordingRun {
    /// Right away, while the host waits for it.
    Now,
    /// In the frame the rendering update submits, while the host runs its event loop. The frame
    /// owns the arena until the host takes it back, and the recording is pending once it has.
    InSubmittedFrame,
}

/// What a recording answers when it ran to its end.
fn recorded_answer(
    output: RecordingStageOutput,
    viewport: NodeSlotId,
    should_paint_overlay: bool,
    publishes_recording: bool,
    frame_generation: u64,
    trace_recordings: bool,
) -> crate::painting::recording_slot::RecordingAnswer {
    let RecordingStageOutput {
        recording,
        recording_from_scratch,
        recorder,
        svg_paint_resources,
    } = output;
    let trace = (trace_recordings && recording.output.capture_log_for_verification.is_some()).then_some(
        crate::painting::paint_state::PendingRecordingTrace {
            viewport,
            should_paint_overlay,
        },
    );
    crate::painting::recording_slot::RecordingAnswer {
        recorder,
        pending: crate::painting::paint_state::PendingRecording {
            recording,
            recording_from_scratch,
            publishes_recording,
            frame_generation,
            svg_paint_resources,
        },
        trace,
    }
}

/// Leaves a finished recording in the arena's paint state, for the host to publish.
fn leave_pending_recording(
    arena: &LayoutNodeArena,
    viewport: NodeSlotId,
    should_paint_overlay: bool,
    publishes_recording: bool,
    frame_generation: u64,
    output: RecordingStageOutput,
) {
    let answer = recorded_answer(
        output,
        viewport,
        should_paint_overlay,
        publishes_recording,
        frame_generation,
        arena.paint_state().borrow().trace_recordings,
    );
    arena.recording().accept_recording_answer(answer);
}

/// A recording the main thread has prepared to run in the frame in flight. It owns what it
/// records from and with, and answers on the ticket [`RecordingJob::new`] returns.
pub(crate) struct RecordingJob {
    input: RecordingStageInput<'static>,
    should_paint_overlay: bool,
    publishes_recording: bool,
    frame_generation: u64,
    answerer: crate::painting::recording_slot::RecordingAnswerer,
}

const _: () = {
    const fn assert_send<T: Send + 'static>() {}
    assert_send::<RecordingJob>();
};

impl RecordingJob {
    fn new(
        input: RecordingStageInput<'static>,
        should_paint_overlay: bool,
        frame_generation: u64,
    ) -> (Self, std::sync::Arc<crate::painting::recording_slot::RecordingTicket>) {
        let (ticket, answerer) = crate::painting::recording_slot::RecordingTicket::new();
        let job = Self {
            should_paint_overlay,
            publishes_recording: input.inputs.publishes_recording,
            frame_generation,
            input,
            answerer,
        };
        (job, ticket)
    }

    /// Records, and answers. A recording that panics answers that it was abandoned as it unwinds,
    /// and its panic continues where the main thread takes its stage back.
    pub(crate) fn run(self) {
        let viewport = self.input.viewport;
        let trace_recordings = self.input.trace_recordings;
        let output = record_display_list_stage(self.input);
        self.answerer.answer(recorded_answer(
            output,
            viewport,
            self.should_paint_overlay,
            self.publishes_recording,
            self.frame_generation,
            trace_recordings,
        ));
    }
}

/// Records the document's display list and leaves the recording pending in the arena. With
/// `run` [`FfiRecordingRun::InSubmittedFrame`], and a frame scheduler that submits recordings, the
/// recording runs in the submitted frame and this returns before it has; otherwise it runs now, on
/// the stage thread, while the caller waits for it. The document thread never records itself.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
/// Input arrays and byte buffers must remain valid and immutable throughout this call;
/// fonts for enabled overlays must be live `Gfx::Font`s. A submitted recording owns the arena
/// until the host takes the frame back, and the host keeps the arena alive until then.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_record_display_list(
    arena_handle: *mut c_void,
    viewport: NodeSlotId,
    inputs: crate::painting::host::FfiRecordingInputs,
    run: FfiRecordingRun,
) -> bool {
    crate::layout::main_side_census::note_rendering_update(arena_handle);
    let arena = unsafe { arena_from_handle(arena_handle) };
    {
        let mut recording = arena.recording();
        // With the frame scheduler, a recording left unpublished is a frame the scheduler dropped, which it never
        // does, so that is checked in every build.
        if crate::stage_thread::submits("recording") {
            assert!(
                recording.pending_recording().is_none(),
                "a frame was dropped: its recording was not published before the next one started"
            );
        }
        debug_assert!(
            recording.pending_recording().is_none(),
            "a recording must be published before the next one starts"
        );
        *recording.pending_recording_trace() = None;
        *recording.pending_recording() = None;
    }
    let recording_inputs = {
        let paint_state = arena.paint_state().borrow();
        if !arena.paintable_row_is_populated(viewport) || arena.stacking_context_entries(viewport).is_none() {
            return false;
        }
        let visual_context = &paint_state.visual_context;
        // SAFETY: The host lends the input arrays and buffers for this call. Only owned
        // output and retained resources escape into the pending recording below.
        unsafe {
            inputs.borrow_recording_inputs(
                visual_context
                    .last_tree_inputs
                    .expect("a recording follows a visual context update"),
                paint_state
                    .root_background_source
                    .expect("a recording follows paint preparation"),
                paint_state.vector_image_display_lists.clone(),
            )
        }
    };
    let publishes_recording = recording_inputs.publishes_recording;
    let should_paint_overlay = recording_inputs.should_paint_overlay;
    // A clock lease's ticks record again with what this recording records with.
    if crate::clock_frames::enabled() && publishes_recording {
        arena.paint_state().borrow_mut().clock_recording = Some(crate::painting::paint_state::ClockRecording {
            viewport,
            inputs: recording_inputs.clone().into_owned(),
        });
    }
    // The recording is made for the render state as it stands now. It is not published if the
    // document retires that render state before the host takes the recording in.
    // SAFETY: Guaranteed by the caller.
    let frame_generation = unsafe { crate::layout::frame_retirement::frame_generation(arena_handle) };
    if run == FfiRecordingRun::InSubmittedFrame && crate::stage_thread::submits("recording") {
        if crate::stage_thread::recordings_lend_published_rows() {
            // A read of committed geometry beside the recording reads the rows as published now.
            // SAFETY: No borrow of the arena is live here.
            let _ = unsafe { arena_from_handle_mut(arena_handle) }.committed_paintable_rows();
        }
        // SAFETY: No borrow of the arena is live here.
        let arena = unsafe { arena_from_handle_mut(arena_handle) };
        let input = recording_stage_input(arena, viewport, recording_inputs.into_owned());
        let (job, ticket) = RecordingJob::new(input, should_paint_overlay, frame_generation);
        arena.recording().await_recording(ticket);
        crate::stage_thread::submit_recording(arena_handle, move || job.run());
        return true;
    }
    let output = {
        // SAFETY: No borrow of the arena is live here.
        let input = recording_stage_input(
            unsafe { arena_from_handle_mut(arena_handle) },
            viewport,
            recording_inputs,
        );
        crate::stage_thread::run_stage_on_stage_thread(|| record_display_list_stage(input))
    };
    // SAFETY: The stage has returned the arena.
    let arena = unsafe { arena_from_handle(arena_handle) };
    leave_pending_recording(
        arena,
        viewport,
        should_paint_overlay,
        publishes_recording,
        frame_generation,
        output,
    );
    true
}

/// Records the document's display list again for a clock lease's tick, with the inputs of the last
/// recording the main thread published, and leaves it pending in the arena for the tick to present.
/// Returns false, having left nothing pending, where there is no such recording to go by, a
/// recording is pending already, or the recording painted an SVG-as-image render the main thread
/// has not resolved.
///
/// # Safety
///
/// `arena_handle` must be a live arena that the calling tick owns, with the main thread idle.
pub(crate) unsafe fn record_for_clock_tick(arena_handle: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena_handle) };
    if arena.recording().pending_recording().is_some() {
        return false;
    }
    let (viewport, inputs) = {
        let paint_state = arena.paint_state().borrow();
        let Some(clock) = paint_state.clock_recording.clone() else {
            return false;
        };
        let mut inputs = clock.inputs;
        // What the tick's layout prepared, the recording reads as the main thread's would have.
        if let Some(root_background_source) = paint_state.root_background_source {
            inputs.uncaptured.root_background_source = root_background_source;
        }
        inputs.vector_image_display_lists = paint_state.vector_image_display_lists.clone();
        (clock.viewport, inputs)
    };
    if !arena.paintable_row_is_populated(viewport) || arena.stacking_context_entries(viewport).is_none() {
        return false;
    }
    let should_paint_overlay = inputs.should_paint_overlay;
    // SAFETY: Guaranteed by the caller.
    let frame_generation = unsafe { crate::layout::frame_retirement::frame_generation(arena_handle) };
    // SAFETY: Guaranteed by the caller; no borrow of the arena is live here.
    let output = record_display_list_stage(recording_stage_input(
        unsafe { arena_from_handle_mut(arena_handle) },
        viewport,
        inputs,
    ));
    // SAFETY: The recording has returned its borrow.
    let arena = unsafe { arena_from_handle(arena_handle) };
    // An SVG-as-image the tick paints at a size the main thread has not rendered it at would show as
    // an empty image: the main thread renders the image, and the frame, itself.
    if !output.recording.resources.missed_vector_images.is_empty() {
        let mut recording = arena.recording();
        recording.give_back_recorder(output.recorder);
        recording.forget_published_frame();
        return false;
    }
    leave_pending_recording(arena, viewport, should_paint_overlay, true, frame_generation, output);
    true
}

/// Updates the visual contexts a clock tick's layout left behind, on the render side, as the main
/// thread's rendering update does before it records. Returns whether the tick can show its frame
/// without the main thread: the update changed the tree incrementally, and the tick's frame takes
/// it to the compositor, whose copy is in step with the frames it was shown (see
/// `layout_arena_clock_tick_scroll_state_snapshot`). Where it returns false, the update built the
/// tree anew or left a compositor animation without its node, and the main thread's next frame
/// takes the tree to the compositor.
///
/// # Safety
///
/// `arena_handle` must be a live arena that the calling tick owns, with the main thread idle.
pub(crate) unsafe fn settle_visual_contexts_for_clock_tick(arena_handle: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena_handle) };
    // What resolves SVG paint servers and filters reaches the DOM.
    if arena.svg_paint_resources().needs_sync() {
        return false;
    }
    let viewport = {
        let paint_state = arena.paint_state().borrow();
        let Some(clock) = paint_state.clock_recording.as_ref() else {
            return false;
        };
        let state = &paint_state.visual_context;
        if state.tree.is_none()
            || state.dirty_boxes.global_reason
                != crate::painting::visual_context::dirty::VisualContextGlobalRebuildReason::None
        {
            return false;
        }
        if state.dirty_boxes.boxes.is_empty() && state.dirty_boxes.removed.is_empty() {
            return true;
        }
        clock.viewport
    };
    if !arena.paintable_row_is_populated(viewport) {
        return false;
    }
    let compositor_animations = arena
        .paint_state()
        .borrow()
        .visual_context
        .tree
        .as_ref()
        .map(|tree| tree.shared_visual_animations());
    // SAFETY: Guaranteed by the caller; no borrow of the arena is live here.
    let outcome = update_accumulated_visual_contexts_stage(unsafe { arena_from_handle_mut(arena_handle) }, viewport);
    if outcome.performed_full_build {
        return false;
    }
    // A new structure of the tree drops the compositor's animations, which the main thread publishes
    // again in its rendering update. The tick's frame takes the tree to the compositor with them,
    // or else the compositor would show what the main thread last laid down for the nodes they
    // drive. An animation whose node went away needs the main thread to publish it anew.
    if outcome.structural_epoch_changed
        && let Some(animations) = compositor_animations.filter(|animations| !animations.is_empty())
    {
        // SAFETY: Guaranteed by the caller; no borrow of the arena is live here.
        let arena = unsafe { arena_from_handle_mut(arena_handle) };
        let carried_every_animation = {
            let mut paint_state = arena.paint_state().borrow_mut();
            let Some(tree) = paint_state.visual_context.tree.as_mut() else {
                return true;
            };
            if tree.has_visual_animations() {
                return true;
            }
            std::sync::Arc::make_mut(tree).carry_visual_animations_over(animations)
        };
        // The rows the update published hold the tree it left.
        arena.publish_paintable_rows();
        return carried_every_animation;
    }
    true
}

/// Hands `publish` the scroll offsets of the nodes of the visual context tree a clock tick's layout
/// left, for the frame that takes a tree of another structure to the compositor. The main thread
/// refreshes its own copy of the scroll state still.
///
/// # Safety
///
/// `arena_handle` must be a live arena that the calling tick owns, with the main thread idle;
/// `publish` is called synchronously with `sink` and a view of the snapshot that is valid only for
/// the duration of that call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_clock_tick_scroll_state_snapshot(
    arena_handle: *mut c_void,
    sink: *mut c_void,
    publish: unsafe extern "C" fn(*mut c_void, *const libgfx_rust::FloatPoint, usize),
) {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena_handle) };
    let snapshot = {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::ScrollStateRefresh);
        let paintable_rows = arena.paintable_rows();
        let mut paint_state = arena.paint_state().borrow_mut();
        let state = &mut paint_state.visual_context;
        if state.needs_to_refresh_scroll_state {
            crate::painting::visual_context::refresh::refresh_scroll_state(&paintable_rows, &mut state.scroll_state);
        }
        let mut snapshot = state
            .scroll_state
            .snapshot(arena.visual_context_tree_inputs().device_pixels_per_css_pixel);
        // https://drafts.csswg.org/css-position/#sticky-pos
        if let Some(tree) = state.tree.as_deref() {
            tree.resolve_sticky_offsets_in_place(&mut snapshot);
        }
        snapshot
    };
    // SAFETY: The C++ sink copies the offsets synchronously.
    unsafe { publish(sink, snapshot.as_ptr(), snapshot.len()) };
}

/// What a flight's recording reads of the host, which the main thread sealed where it submitted the
/// flight, ahead of the layout the flight runs first. What the recording reads of the paint state
/// (the visual context tree inputs, the root background source and the SVG-as-image renders) the
/// flight takes from the arena once it has prepared it.
pub(crate) struct FlightPaintSeal {
    inputs: crate::painting::record::RecordingInputs<'static>,
    frame_generation: u64,
    presentation: Option<FlightPresentation>,
}

/// How a flight presents what it recorded, which the host sealed with its paint: `present` is
/// called on the stage with `context`, the visual context tree the recording was made against (a
/// reference the callee owns), and the scroll state snapshot the flight refreshed, if it did.
pub(crate) type FfiFlightPresent =
    unsafe extern "C" fn(*mut c_void, *const c_void, *const libgfx_rust::FloatPoint, usize, bool);

#[derive(Clone, Copy)]
pub(crate) struct FlightPresentation {
    context: usize,
    present: FfiFlightPresent,
}

/// What preparing and recording a flight's paint left for the main thread to take in.
#[derive(Default)]
pub(crate) struct FlightPaintProducts {
    pub(crate) visual_context_update: crate::painting::host::FfiVisualContextUpdateOutcome,
    pub(crate) scroll_state_snapshot: Option<Vec<libgfx_rust::FloatPoint>>,
    /// How the flight presents its recording, if the host sealed a presentation with its paint.
    pub(crate) presentation: Option<FlightPresentation>,
    /// Why the flight did not record after preparing the paint state, if it did not.
    pub(crate) stopped: Option<FlightPaintStop>,
}

/// Why a flight did not record.
#[derive(Clone, Copy, Debug)]
pub(crate) enum FlightPaintStop {
    /// The document's SVG paint resources wait for the main thread to synchronize them.
    SvgPaintResources,
    /// The recording would paint an SVG-as-image render the main thread has not made.
    VectorImages,
    /// The document has no viewport box to record.
    NoViewport,
}

thread_local! {
    // On the main thread, the paint sealed for the flight about to be submitted, and its arena.
    static SEALED_FLIGHT_PAINT: std::cell::RefCell<Option<(usize, FlightPaintSeal)>> =
        const { std::cell::RefCell::new(None) };
}

/// Seals what the next flight of the arena records with, from the host inputs of a recording as
/// `layout_arena_record_display_list` takes them. The SVG-as-image renders the recording paints are
/// resolved into the arena before this. With `present`, the flight may present what it records
/// through it, with `present_context`.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, on the document thread, with no frame in
/// flight; the input arrays and buffers must stay valid for this call. `present` must be callable on
/// the stage thread with `present_context` until the flight is taken back.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_seal_flight_paint(
    arena_handle: *mut c_void,
    inputs: crate::painting::host::FfiRecordingInputs,
    present: Option<unsafe extern "C" fn(*mut c_void, *const c_void, *const libgfx_rust::FloatPoint, usize, bool)>,
    present_context: *mut c_void,
) {
    // The paint state parts are the flight's to fill in once it has prepared them.
    // SAFETY: Guaranteed by the caller; the owned copy outlives the borrow.
    let inputs = unsafe {
        inputs.borrow_recording_inputs(
            crate::painting::host::FfiVisualContextTreeInputs::default(),
            crate::painting::host::FfiRootBackgroundSource::default(),
            std::sync::Arc::default(),
        )
    }
    .into_owned();
    // SAFETY: Guaranteed by the caller.
    let frame_generation = unsafe { crate::layout::frame_retirement::frame_generation(arena_handle) };
    SEALED_FLIGHT_PAINT.with_borrow_mut(|sealed| {
        *sealed = Some((
            arena_handle as usize,
            FlightPaintSeal {
                inputs,
                frame_generation,
                presentation: present.map(|present| FlightPresentation {
                    context: present_context as usize,
                    present,
                }),
            },
        ));
    });
}

/// Drops the paint sealed for a flight that was not submitted.
#[unsafe(no_mangle)]
pub extern "C" fn layout_arena_discard_sealed_flight_paint() {
    SEALED_FLIGHT_PAINT.with_borrow_mut(Option::take);
}

/// Takes the paint sealed for the flight of the arena `arena_handle`, if one was.
pub(crate) fn take_sealed_flight_paint(arena_handle: *mut c_void) -> Option<FlightPaintSeal> {
    SEALED_FLIGHT_PAINT.with_borrow_mut(|sealed| match sealed.take() {
        Some((arena, seal)) if arena == arena_handle as usize => Some(seal),
        _ => None,
    })
}

/// Prepares the arena's paint state for the recording a flight makes after its layout, and records:
/// the visual context update, the scroll state refresh and the recording, as the main thread would
/// run them before a recording it submits. The recording is left pending in the arena, as a
/// submitted recording leaves it.
///
/// # Safety
///
/// `arena_handle` must be a live arena the frame in flight owns, with no borrow of it held.
pub(crate) unsafe fn paint_in_flight(
    arena_handle: *mut c_void,
    seal: FlightPaintSeal,
) -> Result<FlightPaintProducts, FlightPaintStop> {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { &mut *arena_handle.cast::<LayoutNodeArena>() };
    let viewport = arena.layout_root();
    if arena.svg_paint_resources().needs_sync() {
        return Err(FlightPaintStop::SvgPaintResources);
    }
    if !arena.paintable_row_is_populated(viewport) {
        return Err(FlightPaintStop::NoViewport);
    }
    let FlightPaintSeal {
        mut inputs,
        frame_generation,
        presentation,
    } = seal;
    // The renders the main thread resolved as it sealed the flight are the ones the layout before it
    // predicted: the layout the flight ran may paint others. This is checked before anything is
    // prepared, so a flight that does not record leaves the paint state to the main thread.
    let resolved = arena.paint_state().borrow().vector_image_display_lists.clone();
    let prediction_inputs = crate::painting::record::vector_images::FirstPaintPredictionInputs {
        device_pixels_per_css_pixel: arena.visual_context_tree_inputs().device_pixels_per_css_pixel,
        root_background_source: arena.paint_state().borrow().root_background_source,
        css_viewport_rect: inputs.css_viewport_rect,
        document_declares_light_or_dark_color_scheme: inputs.document_declares_light_or_dark_color_scheme,
        image_color_scheme_fallback: inputs.image_color_scheme_fallback,
    };
    let predicted = crate::painting::record::vector_images::predict_first_paint_renders(arena, &prediction_inputs);
    if predicted.iter().any(|request| resolved.get(request).is_none()) {
        return Err(FlightPaintStop::VectorImages);
    }
    let visual_context_update = update_accumulated_visual_contexts_stage(arena, viewport);
    let scroll_state_snapshot = {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::ScrollStateRefresh);
        let paintable_rows = arena.paintable_rows();
        let mut paint_state = arena.paint_state().borrow_mut();
        let state = &mut paint_state.visual_context;
        state.needs_to_refresh_scroll_state.then(|| {
            state.needs_to_refresh_scroll_state = false;
            crate::painting::visual_context::refresh::refresh_scroll_state(&paintable_rows, &mut state.scroll_state);
            let mut snapshot = state
                .scroll_state
                .snapshot(arena.visual_context_tree_inputs().device_pixels_per_css_pixel);
            // https://drafts.csswg.org/css-position/#sticky-pos
            if let Some(tree) = state.tree.as_deref() {
                tree.resolve_sticky_offsets_in_place(&mut snapshot);
            }
            snapshot
        })
    };
    // The visual context update settles the viewport's stacking contexts, which the recording starts from.
    if arena.stacking_context_entries(viewport).is_none() {
        return Ok(FlightPaintProducts {
            visual_context_update,
            scroll_state_snapshot,
            presentation: None,
            stopped: Some(FlightPaintStop::NoViewport),
        });
    }
    {
        let paint_state = arena.paint_state().borrow();
        let tree_inputs = paint_state
            .visual_context
            .last_tree_inputs
            .expect("a recording follows a visual context update");
        inputs.device_pixels_per_css_pixel = tree_inputs.device_pixels_per_css_pixel;
        inputs.uncaptured.viewport_wheel_overflow_x = tree_inputs.viewport_wheel_overflow_x;
        inputs.uncaptured.viewport_wheel_overflow_y = tree_inputs.viewport_wheel_overflow_y;
        inputs.uncaptured.root_background_source = paint_state
            .root_background_source
            .expect("a recording follows paint preparation");
        inputs.vector_image_display_lists = resolved;
    }
    {
        let mut recording = arena.recording();
        assert!(
            recording.pending_recording().is_none(),
            "a frame was dropped: its recording was not published before the next one started"
        );
        *recording.pending_recording_trace() = None;
    }
    let should_paint_overlay = inputs.should_paint_overlay;
    let publishes_recording = inputs.publishes_recording;
    // SAFETY: The frame in flight owns the arena, and no borrow of it is held here.
    let output = record_display_list_stage(recording_stage_input(
        unsafe { &mut *arena_handle.cast::<LayoutNodeArena>() },
        viewport,
        inputs,
    ));
    // SAFETY: The stage has returned its borrow.
    let arena = unsafe { &*arena_handle.cast::<LayoutNodeArena>() };
    leave_pending_recording(
        arena,
        viewport,
        should_paint_overlay,
        publishes_recording,
        frame_generation,
        output,
    );
    Ok(FlightPaintProducts {
        visual_context_update,
        scroll_state_snapshot,
        presentation,
        stopped: None,
    })
}

/// Presents what a flight recorded through the presentation the host sealed with its paint.
///
/// # Safety
///
/// `arena_handle` must be the live arena the frame in flight owns, whose recording the flight left
/// pending, with no borrow of it held.
pub(crate) unsafe fn present_in_flight(arena_handle: *mut c_void, products: &FlightPaintProducts) {
    let presentation = products
        .presentation
        .expect("a flight presents through the presentation sealed with its paint");
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { arena_from_handle(arena_handle) };
    let tree = arena
        .paint_state()
        .borrow()
        .visual_context
        .tree
        .as_ref()
        .map_or(std::ptr::null(), |tree| {
            std::sync::Arc::into_raw(std::sync::Arc::clone(tree)).cast()
        });
    let snapshot = products.scroll_state_snapshot.as_deref();
    // SAFETY: Guaranteed by the host that sealed the presentation.
    unsafe {
        (presentation.present)(
            presentation.context as *mut c_void,
            tree,
            snapshot.map_or(std::ptr::null(), <[libgfx_rust::FloatPoint]>::as_ptr),
            snapshot.map_or(0, <[libgfx_rust::FloatPoint]>::len),
            snapshot.is_some(),
        );
    }
}

/// Publishes the arena's pending recording from the presentation stage of the frame in flight
/// (unless LIBWEB_RENDER_PRESENTS=0), as `layout_arena_publish_recording` does on the main thread. Returns
/// the generation of the hit-test list the recording made, or 0 if there was nothing to publish.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, owned by the frame in flight whose
/// presentation stage calls this; the callbacks in `publish` are called synchronously with their
/// context, which the host lent that stage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_recording_in_frame(
    arena: *mut c_void,
    publish: crate::painting::host::FfiRecordingPublishCallbacks,
) -> u64 {
    let arena = unsafe { arena_from_handle(arena) };
    let Some(pending) = arena.recording().pending_recording().take() else {
        return 0;
    };
    let publish = crate::painting::host::RecordingPublishHost::from(publish);
    // SAFETY: Guaranteed by the caller: this is the frame's presentation stage.
    let presentation = unsafe { crate::painting::host::FramePresentation::new() };
    // NB: The published-rows verifier keeps its baseline per thread, on the main thread; it sees this
    //     publication as the rows the next main-side publication finds.
    crate::painting::record::publish::publish_recording(arena, pending, &presentation, &publish)
}

/// Whether a recording of the arena's document is in flight: submitted, and not taken in yet.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_has_recording_in_flight(arena: *mut c_void) -> bool {
    // SAFETY: The caller passes a live handle. This reads only the recording slot, which the recording in flight does
    // not reach, so it goes through no door that would take the recording in.
    unsafe { &*arena.cast::<LayoutNodeArena>() }.has_recording_in_flight()
}

/// The ticket of the recording of the arena's document in flight, retained for the frame's
/// presentation to publish the recording's answer from, or null if none is in flight. The document
/// takes the answer in once the presentation has published it.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_recording_ticket_for_presentation(arena: *mut c_void) -> *const c_void {
    // SAFETY: The caller passes a live handle. This reads only the recording slot, which the recording in flight does
    // not reach (its job holds no arena), so it goes through no door that would take the recording in.
    let arena = unsafe { &*arena.cast::<LayoutNodeArena>() };
    arena
        .recording_ticket_for_presentation()
        .map_or(std::ptr::null(), |ticket| std::sync::Arc::into_raw(ticket).cast())
}

/// # Safety
///
/// `ticket` must be null or come from `layout_arena_recording_ticket_for_presentation`, released
/// once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_recording_ticket_release(ticket: *const c_void) {
    if !ticket.is_null() {
        // SAFETY: Guaranteed by the caller.
        drop(unsafe { std::sync::Arc::from_raw(ticket.cast::<crate::painting::recording_slot::RecordingTicket>()) });
    }
}

/// Whether the recording whose ticket the frame's presentation holds unwound, which left nothing to
/// publish: the frame shows nothing, and the recording's panic continues where the main thread takes
/// the frame back. Waits for the recording to answer.
///
/// # Safety
///
/// `ticket` must be a live ticket from `layout_arena_recording_ticket_for_presentation`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_recording_ticket_was_abandoned(ticket: *const c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe { &*ticket.cast::<crate::painting::recording_slot::RecordingTicket>() }.was_abandoned()
}

/// What the frame's presentation publishes of a recording, for the host to build its frame from.
#[repr(C)]
pub struct FfiPresentedRecording {
    pub is_identical_to_published_frame: bool,
    pub has_blocking_wheel_event_listeners: bool,
    /// The recorded display list, retained for the host to adopt.
    pub display_list: *const c_void,
}

/// Publishes the answer of the recording whose ticket the frame's presentation holds: hands its
/// resources to the host and leaves its output for the document to take in, without reaching the
/// document. Returns false if the recording has nothing to publish.
///
/// # Safety
///
/// `ticket` must be a live ticket from `layout_arena_recording_ticket_for_presentation`, and this
/// must run on the presentation stage of the frame in flight; the callbacks in `publish` are called
/// synchronously with their context, which the host lent that stage. `out` must point to writable
/// storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_recording_ticket_publish_in_frame(
    ticket: *const c_void,
    publish: crate::painting::host::FfiRecordingPublishCallbacks,
    out: *mut FfiPresentedRecording,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    let ticket = unsafe { &*ticket.cast::<crate::painting::recording_slot::RecordingTicket>() };
    let publish = crate::painting::host::RecordingPublishHost::from(publish);
    // SAFETY: Guaranteed by the caller: this is the frame's presentation stage.
    let presentation = unsafe { crate::painting::host::FramePresentation::new() };
    let presented = ticket.present(|pending, recorder| {
        let output = crate::painting::record::publish::publish_to_host(pending, recorder, &presentation, &publish);
        let presented = FfiPresentedRecording {
            is_identical_to_published_frame: output.is_identical_to_published_frame,
            has_blocking_wheel_event_listeners: output.has_blocking_wheel_event_listeners,
            display_list: std::sync::Arc::into_raw(output.display_list.clone()).cast(),
        };
        (output, presented)
    });
    let Some(presented) = presented else {
        return false;
    };
    // SAFETY: Guaranteed by the caller.
    unsafe { out.write(presented) };
    true
}

/// Takes in the recording of the arena's document the frame in flight made and presented, waiting
/// for it if it has not answered yet.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_take_in_recording(arena: *mut c_void) {
    let arena = unsafe { arena_from_handle(arena) };
    drop(arena.recording());
}

/// Runs `handoff(context)`, which hands a navigable's finished frame to its compositor frame sink,
/// as a render stage: on the stage thread under `LIBWEB_STAGE_THREAD=lockstep` or `overlap`, here
/// otherwise.
///
/// # Safety
///
/// `handoff` must be safe to call with `context` from any thread, and `context` must stay valid
/// until this returns. The frame it hands over may only be reachable through `context`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_run_compositor_frame_handoff_stage(
    handoff: unsafe extern "C" fn(*mut c_void),
    context: *mut c_void,
) {
    // SAFETY: Guaranteed by the caller: `handoff` may be called with `context` from any thread, and
    // the frame is reachable only through `context`.
    let context = unsafe { crate::stage_thread::CallerWaits::new(context) };
    // SAFETY: As above.
    crate::stage_thread::run_stage(move || unsafe { handoff(context.into_inner()) });
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_form_control_paint_facts(
    arena: *mut c_void,
    slot: NodeSlotId,
    facts: crate::painting::host::FfiFormControlPaintFacts,
) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::published_immutable::note_row_mutation(
        arena,
        slot,
        "M6 layout_arena_set_form_control_paint_facts",
    );
    arena.set_replaced_paint_facts(
        slot,
        crate::painting::replaced_paint_facts::ReplacedPaintFacts::FormControl(facts),
    )
}

/// # Safety
///
/// `arena` must be a live handle used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_before_invalidation_journal_drain(arena: *mut c_void) {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::published_immutable::before_journal_publication(arena);
}

/// # Safety
///
/// `arena` must be a live handle used on the document thread and must have had a matching
/// `layout_arena_before_invalidation_journal_drain` call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_after_invalidation_journal_drain(arena: *mut c_void) {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::published_immutable::after_journal_publication(arena);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_canvas_paint_facts(
    arena: *mut c_void,
    slot: NodeSlotId,
    facts: crate::painting::host::FfiCanvasPaintFacts,
) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::published_immutable::note_row_mutation(arena, slot, "M6 layout_arena_set_canvas_paint_facts");
    arena.set_replaced_paint_facts(
        slot,
        crate::painting::replaced_paint_facts::ReplacedPaintFacts::Canvas(facts),
    )
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `entries` must point at `count` readable entries whose frame ids name live shared resources.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_layer_image_paint_facts(
    arena: *mut c_void,
    slot: NodeSlotId,
    entries: *const crate::painting::host::FfiLayerImagePaintFactsEntry,
    count: usize,
) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::published_immutable::note_row_mutation(arena, slot, "M6 layout_arena_set_layer_image_paint_facts");
    let entries = if count == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(entries, count) }
            .iter()
            .map(
                |entry| crate::painting::layer_image_paint_facts::LayerImagePaintFactsEntry {
                    list: entry.list,
                    computed_index: entry.computed_index,
                    facts: crate::painting::layer_image_paint_facts::LayerImagePaintFacts::from_ffi(&entry.facts),
                },
            )
            .collect()
    };
    arena.set_layer_image_paint_facts(slot, entries)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// Any nonzero frame id in `facts` must name a live shared resource.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_replaced_image_paint_facts(
    arena: *mut c_void,
    slot: NodeSlotId,
    facts: crate::painting::host::FfiReplacedImagePaintFacts,
) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::published_immutable::note_row_mutation(
        arena,
        slot,
        "M6 layout_arena_set_replaced_image_paint_facts",
    );
    let facts = crate::painting::replaced_paint_facts::ImagePaintFacts::from_ffi(&facts);
    arena.set_replaced_paint_facts(
        slot,
        crate::painting::replaced_paint_facts::ReplacedPaintFacts::Image(facts),
    )
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// Any nonzero poster frame id in `facts` must name a live shared resource.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_video_paint_facts(
    arena: *mut c_void,
    slot: NodeSlotId,
    facts: crate::painting::host::FfiVideoPaintFacts,
) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::published_immutable::note_row_mutation(arena, slot, "M6 layout_arena_set_video_paint_facts");
    let facts = crate::painting::replaced_paint_facts::VideoPaintFacts::from_ffi(&facts);
    arena.set_replaced_paint_facts(
        slot,
        crate::painting::replaced_paint_facts::ReplacedPaintFacts::Video(facts),
    )
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_navigable_container_paint_facts(
    arena: *mut c_void,
    slot: NodeSlotId,
    facts: crate::painting::host::FfiNavigableContainerPaintFacts,
) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::published_immutable::note_row_mutation(
        arena,
        slot,
        "M7 layout_arena_set_navigable_container_paint_facts",
    );
    arena.set_replaced_paint_facts(
        slot,
        crate::painting::replaced_paint_facts::ReplacedPaintFacts::NavigableContainer(facts),
    )
}

/// Whether the row's DOM node published itself as editable or as an editing host. A row that no
/// DOM node ever bound answers no, the way a node that is neither does.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_is_editable_or_editing_host(arena: *mut c_void, slot: NodeSlotId) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    arena.node_has_dom_paint_fact(slot, crate::layout::node_data::DomPaintFact::EditableOrEditingHost)
}

/// The locally hosted content navigable a navigable container's row last published. A zero id names
/// none, which is also what a row carrying no navigable container facts answers.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_navigable_container_local_content_navigable(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> crate::painting::host::FfiCrossProcessId {
    let arena = unsafe { arena_from_handle(arena) };
    arena
        .replaced_paint_facts(slot)
        .and_then(|facts| facts.navigable_container())
        .map(|facts| facts.local_content_navigable)
        .unwrap_or_default()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_scroll_snap_axes(
    arena: *mut c_void,
    snap_container: NodeSlotId,
) -> crate::painting::host::FfiSnapAxes {
    let arena = unsafe { arena_from_handle(arena) };
    crate::painting::scroll_snap::snap_axes_of_scroll_container(arena, snap_container)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_sync_selection_pseudo_style(arena: *mut c_void, element_style_node: u32) {
    let arena = unsafe { arena_from_handle(arena) };
    let Some(element) = crate::css::style::tree::StyleNodeID::from_raw(element_style_node) else {
        return;
    };
    crate::painting::selection::sync_selection_pseudo_style(arena, element);
}

/// # Safety
///
/// `sink` must be the pointer handed to the callback, used synchronously.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paint_push_color_stop(
    sink: *mut c_void,
    color: libgfx_rust::Color,
    position: f32,
) {
    let sink = unsafe { &mut *sink.cast::<crate::painting::svg_paint_resources::PublishedSvgPaintServer>() };
    if let crate::painting::svg_paint_resources::PublishedSvgPaintServer::Gradient(gradient) = sink {
        gradient
            .stops
            .push(crate::painting::svg_paint_resources::PublishedSvgGradientStop { color, position });
    }
}

/// # Safety
///
/// `sink` must be the pointer handed to the callback, used synchronously, and `description`
/// must be readable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_svg_paint_resources_push_gradient(
    sink: *mut c_void,
    description: *const crate::painting::host::FfiSvgGradientDescription,
) {
    let sink = unsafe { &mut *sink.cast::<crate::painting::svg_paint_resources::PublishedSvgPaintServer>() };
    *sink = crate::painting::svg_paint_resources::PublishedSvgPaintServer::Gradient(
        crate::painting::svg_paint_resources::PublishedSvgGradient {
            description: unsafe { *description },
            stops: Vec::new(),
        },
    );
}

/// # Safety
///
/// `sink` must be the pointer handed to the callback, used synchronously, `description` must be
/// readable, and `css_transform_entries` must point at `css_transform_count` readable
/// `ComputedResolvedTransform` values of a live computed style.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_svg_paint_resources_push_pattern(
    sink: *mut c_void,
    description: *const crate::painting::host::FfiSvgPatternDescription,
    css_transform_entries: *const c_void,
    css_transform_count: usize,
) {
    let sink = unsafe { &mut *sink.cast::<crate::painting::svg_paint_resources::PublishedSvgPaintServer>() };
    *sink = crate::painting::svg_paint_resources::PublishedSvgPaintServer::Pattern(
        crate::painting::svg_paint_resources::PublishedSvgPattern {
            description: unsafe { *description },
            css_transform: unsafe {
                ffi_slice(
                    css_transform_entries.cast::<crate::css::computed_value_types::ComputedResolvedTransform>(),
                    css_transform_count,
                )
            }
            .to_vec(),
        },
    );
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiImagePaintRecordKind {
    DecodedFrame,
    NestedDisplayList,
    Gradient,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiImagePaintRecordInputs {
    pub kind: FfiImagePaintRecordKind,
    pub device_pixels_per_css_pixel: f64,
    pub dest_rect: libgfx_rust::FloatRect,
    pub frame_id: u64,
    pub scaling_mode: libgfx_rust::ScalingMode,
    pub nested_display_list_id: u64,
    pub nested_display_list_size: libgfx_rust::IntSize,
    pub gradient_style_value: *const c_void,
    pub gradient_tile_size: FfiCssPixelSize,
    /// The `FfiColorResolutionInput` the gradient's color stops resolve against.
    pub gradient_stop_color_resolution_input: *const c_void,
}

/// # Safety
///
/// `inputs`, and the gradient style value and color resolution input it points at, must be
/// live for the call. `consume` is called synchronously and takes ownership of the command
/// storage and visual context tree handles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn ladybird_web_record_image_paint_display_list(
    inputs: *const FfiImagePaintRecordInputs,
    context: *mut c_void,
    consume: unsafe extern "C" fn(*mut c_void, *const c_void, *const c_void),
) {
    use crate::css::color_resolution::{
        FfiColorResolutionInput, relative_color_context_from_ffi, resolution_input_from_ffi,
    };
    use crate::painting::display_list::commands::{DisplayListResourceId, ImageFrameResourceId};
    use crate::painting::display_list::device_pixels::DevicePixelConverter;
    use crate::painting::display_list::recorder::DisplayListRecorder;
    use crate::painting::record::paint::gradient_resolution::{
        record_resolved_gradient_fill, resolve_gradient_paint_with_input,
    };
    use crate::painting::visual_context::{TransformData, TransformDataRole, VisualContextTree};
    use libgfx_rust::{CompositingAndBlendingOperator, FloatMatrix4x4, FloatPoint};
    let inputs = unsafe { &*inputs };
    let tree = VisualContextTree::create(TransformData {
        matrix: FloatMatrix4x4::identity(),
        origin: FloatPoint::default(),
        sorting_context_root_index: None,
        flattens_inherited_transform: false,
        role: TransformDataRole::CssTransform,
        synthetic_plane: false,
        establishes_sorting_context: false,
    });
    // Force-dark never reaches here: an image is darkened by classifying the image itself — not by inverting the
    // fills that raster it.
    let mut recorder = DisplayListRecorder::new(None);
    let dest_rect = inputs.dest_rect;
    match inputs.kind {
        FfiImagePaintRecordKind::DecodedFrame => recorder.draw_scaled_decoded_image_frame(
            dest_rect,
            None,
            ImageFrameResourceId(inputs.frame_id),
            inputs.scaling_mode,
            CompositingAndBlendingOperator::Normal,
            None,
            ForceDarkRole::None,
        ),
        FfiImagePaintRecordKind::NestedDisplayList => recorder.paint_nested_display_list(
            DisplayListResourceId(inputs.nested_display_list_id),
            dest_rect,
            inputs.nested_display_list_size,
        ),
        FfiImagePaintRecordKind::Gradient => {
            // SAFETY: the host keeps the gradient style value and its color resolution input
            // live for the duration of the call.
            let (gradient_style_value, color_resolution_input) = unsafe {
                (
                    &*inputs
                        .gradient_style_value
                        .cast::<crate::css::style_value::StyleValueData>(),
                    &*inputs
                        .gradient_stop_color_resolution_input
                        .cast::<FfiColorResolutionInput>(),
                )
            };
            let relative_color_channels = relative_color_context_from_ffi(color_resolution_input);
            // SAFETY: the borrowed input outlives the resolution below.
            let color_input = unsafe { resolution_input_from_ffi(color_resolution_input, &relative_color_channels) };
            let resolved =
                resolve_gradient_paint_with_input(gradient_style_value, inputs.gradient_tile_size.into(), &color_input);
            record_resolved_gradient_fill(
                &mut recorder,
                DevicePixelConverter::new(inputs.device_pixels_per_css_pixel),
                &resolved,
                dest_rect,
                CompositingAndBlendingOperator::Normal,
                ForceDarkRole::None,
            );
        }
    }
    let recorded = recorder.into_builder().finish();
    unsafe {
        consume(
            context,
            std::sync::Arc::into_raw(std::sync::Arc::new(recorded)).cast(),
            std::sync::Arc::into_raw(std::sync::Arc::new(tree)).cast(),
        );
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_last_recording_is_identical_to_published_frame(arena: *mut c_void) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state
        .last_recording
        .as_ref()
        .is_some_and(|recording| recording.is_identical_to_published_frame)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_last_recording_has_blocking_wheel_event_listeners(arena: *mut c_void) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state
        .last_recording
        .as_ref()
        .is_some_and(|recording| recording.has_blocking_wheel_event_listeners)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_invalidate_paint_cache(
    arena: *mut c_void,
    paintable: NodeSlotId,
    propagated_text_decorations: bool,
    stage: u8,
) {
    use crate::painting::record::damage::PaintDamage;
    let arena = unsafe { arena_from_handle(arena) };
    let writer = match stage {
        0 => "journal drain",
        1 => "anonymous row invalidation",
        2 => "layout detach cleanup",
        3 => "paint fact reconciliation",
        _ => "unknown paint-cache invalidation stage",
    };
    let _writer = crate::painting::published_immutable::enter_writer(writer);
    crate::painting::published_immutable::note_row_mutation_with_writer(
        arena,
        paintable,
        "M12 layout_arena_paintable_invalidate_paint_cache",
        writer,
    );
    if propagated_text_decorations {
        arena.push_propagated_text_decoration_damage(paintable);
    } else {
        arena.push_paint_damage(paintable, PaintDamage::ALL_DRAW | PaintDamage::ALL_HIT);
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_row_reset_version(arena: *mut c_void, paintable: NodeSlotId) -> u64 {
    let arena = unsafe { arena_from_handle(arena) };
    arena.paintable_rows().paintable_row_reset_version(paintable)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_invalidate_for_repaint(
    arena: *mut c_void,
    paintable: NodeSlotId,
    include_hit_test_items: bool,
    stage: u8,
) {
    use crate::painting::record::damage::PaintDamage;
    let arena = unsafe { arena_from_handle(arena) };
    let _writer = crate::painting::published_immutable::enter_writer(match stage {
        0 => "journal drain",
        1 => "anonymous row invalidation",
        _ => "unknown repaint damage stage",
    });
    let damage = if include_hit_test_items {
        PaintDamage::ALL_PRODUCERS
    } else {
        PaintDamage::ALL_DRAW
    };
    arena.push_paint_damage_for_repaint(paintable, damage);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_invalidate_subtree_for_repaint(
    arena: *mut c_void,
    paintable: NodeSlotId,
    stage: u8,
) {
    let arena = unsafe { arena_from_handle(arena) };
    let _writer = crate::painting::published_immutable::enter_writer(match stage {
        0 => "journal drain",
        1 => "anonymous row invalidation",
        _ => "unknown repaint damage stage",
    });
    arena.push_paint_damage_to_paint_subtree(paintable, crate::painting::record::damage::PaintDamage::ALL_PRODUCERS);
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_invalidate_all_paint_caches(arena: *mut c_void) {
    let arena = unsafe { arena_from_handle(arena) };
    arena.push_all_paint_damage();
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_computed_svg_path(
    arena: *mut c_void,
    paintable: NodeSlotId,
) -> *const c_void {
    let arena = unsafe { arena_from_handle(arena) };
    let paintable_rows = arena.paintable_rows();
    if !paintable_rows.paintable_row_is_populated(paintable) {
        return std::ptr::null();
    }
    crate::painting::paintable_geometry::committed_svg_path(&paintable_rows, paintable)
        .map_or(std::ptr::null(), |path| path.as_raw())
}

#[repr(C)]
pub struct FfiCaretRectResult {
    pub found: bool,
    pub rect: FfiCssPixelRect,
    pub style_source: *mut c_void,
    pub owner_paintable: NodeSlotId,
    pub nearest_self_painting_inline: NodeSlotId,
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_text_caret_rect_in_dom_range(
    arena: *mut c_void,
    primary: NodeSlotId,
    offset: usize,
) -> FfiOptionalCssPixelRect {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    match crate::painting::caret::caret_rect_in_dom_range(&paintable_rows, node_slots, offset) {
        Some(rect) => FfiOptionalCssPixelRect {
            has_value: true,
            rect: rect.into(),
        },
        None => FfiOptionalCssPixelRect {
            has_value: false,
            rect: FfiCssPixelRect::default(),
        },
    }
}

#[repr(C)]
pub struct FfiEmptyLineCaretRect {
    pub has_value: bool,
    pub rect: FfiCssPixelRect,
    pub style_source: *mut c_void,
}

#[repr(C)]
pub struct FfiRectToViewportTransform {
    pub visual_context_tree: *const c_void,
    pub scroll_offsets: *const libgfx_rust::FloatPoint,
    pub scroll_offsets_len: usize,
    pub device_pixels_per_css_pixel: f32,
}

/// SAFETY: A non-null `visual_context_tree` must be a live retained tree handle and `scroll_offsets`
/// must address `scroll_offsets_len` points for as long as the returned borrow lives.
unsafe fn rect_to_viewport_transform_from_ffi(
    transform: &FfiRectToViewportTransform,
) -> Option<RectToViewportTransform<'_>> {
    if transform.visual_context_tree.is_null() {
        return None;
    }
    Some(RectToViewportTransform {
        visual_context_tree: unsafe { tree_from_handle(transform.visual_context_tree) },
        scroll_offsets: unsafe { ffi_slice(transform.scroll_offsets, transform.scroll_offsets_len) },
        device_pixels_per_css_pixel: transform.device_pixels_per_css_pixel,
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `rect_to_viewport_transform` must satisfy `rect_to_viewport_transform_from_ffi`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_client_rects(
    arena: *mut c_void,
    layout_node: NodeSlotId,
    rect_to_viewport_transform: FfiRectToViewportTransform,
    context: *mut c_void,
    push_rect: unsafe extern "C" fn(*mut c_void, FfiCssPixelRect),
) {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let rect_to_viewport_transform = unsafe { rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform) };
    crate::painting::client_rects::for_each_client_rect(
        &paintable_rows,
        layout_node,
        rect_to_viewport_transform.as_ref(),
        |rect| {
            // SAFETY: The consumer copies the plain-data rect synchronously.
            unsafe { push_rect(context, rect.into()) };
        },
    );
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `rect_to_viewport_transform` must satisfy `rect_to_viewport_transform_from_ffi`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_bounding_client_rect(
    arena: *mut c_void,
    layout_node: NodeSlotId,
    rect_to_viewport_transform: FfiRectToViewportTransform,
) -> FfiCssPixelRect {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let rect_to_viewport_transform = unsafe { rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform) };
    crate::painting::client_rects::bounding_client_rect(
        &paintable_rows,
        layout_node,
        rect_to_viewport_transform.as_ref(),
    )
    .into()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `rect_to_viewport_transform` must satisfy `rect_to_viewport_transform_from_ffi`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_transform_subtree_is_clipped_outside(
    arena: *mut c_void,
    target: NodeSlotId,
    root_bounds: FfiCssPixelRect,
    rect_to_viewport_transform: FfiRectToViewportTransform,
) -> bool {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let rect_to_viewport_transform = unsafe { rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform) };
    crate::painting::intersection_observer::transform_subtree_is_clipped_outside(
        &paintable_rows,
        target,
        root_bounds.into(),
        rect_to_viewport_transform.as_ref(),
    )
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `rect_to_viewport_transform` must satisfy `rect_to_viewport_transform_from_ffi`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_intersection_observer_intersection_rect(
    arena: *mut c_void,
    target: NodeSlotId,
    target_rect: FfiCssPixelRect,
    intersection_root: NodeSlotId,
    root_bounds: FfiCssPixelRect,
    rect_to_viewport_transform: FfiRectToViewportTransform,
    context: *mut c_void,
    inflate_scroll_container_clip_rect_by_scroll_margin: unsafe extern "C" fn(
        *mut c_void,
        FfiCssPixelRect,
    ) -> FfiCssPixelRect,
) -> FfiCssPixelRect {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let rect_to_viewport_transform = unsafe { rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform) };
    crate::painting::intersection_observer::intersection_rect(
        &paintable_rows,
        target,
        target_rect.into(),
        intersection_root,
        root_bounds.into(),
        rect_to_viewport_transform.as_ref(),
        |clip_rect: CssPixelRect| -> CssPixelRect {
            // SAFETY: The callback copies the plain-data rect synchronously.
            unsafe { inflate_scroll_container_clip_rect_by_scroll_margin(context, clip_rect.into()) }.into()
        },
    )
    .into()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_can_compute_client_rects_without_visual_context_update(
    arena: *mut c_void,
    layout_node: NodeSlotId,
    viewport_scroll_offset_is_zero: bool,
) -> bool {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    crate::painting::client_rects::can_compute_client_rects_without_visual_context_update(
        &paintable_rows,
        layout_node,
        viewport_scroll_offset_is_zero,
    )
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_inline_paintable_has_content_pieces(
    arena: *mut c_void,
    inline_paintable: NodeSlotId,
) -> bool {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let mut has_content = false;
    with_inline_pieces(&paintable_rows, inline_paintable, |piece, _| {
        if !piece.is_geometry_only_placeholder {
            has_content = true;
            return false;
        }
        true
    });
    has_content
}

#[repr(C)]
pub struct FfiOptionalCssPixelPoint {
    pub has_value: bool,
    pub x: CssPixels,
    pub y: CssPixels,
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_inline_paintable_first_piece_position(
    arena: *mut c_void,
    inline_paintable: NodeSlotId,
) -> FfiOptionalCssPixelPoint {
    let mut result = FfiOptionalCssPixelPoint {
        has_value: false,
        x: CssPixels::from_raw(0),
        y: CssPixels::from_raw(0),
    };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let Some(root) = paintable_rows.inline_pieces_root(inline_paintable) else {
        return result;
    };
    let root_position = crate::painting::paintable_geometry::absolute_position(&paintable_rows, root);
    let border_widths = crate::painting::paintable_geometry::committed_border(&paintable_rows, inline_paintable);
    let padding_widths = crate::painting::paintable_geometry::committed_padding(&paintable_rows, inline_paintable);
    with_inline_pieces(&paintable_rows, inline_paintable, |piece, _data| {
        let border_rect = CssPixelRect::from(piece.border_box_rect);
        let rect = if piece.is_geometry_only_placeholder {
            border_rect
        } else {
            let padding_rect = piece.shrunken_by_present_edges(border_rect, border_widths);
            piece.shrunken_by_present_edges(padding_rect, padding_widths)
        };
        result.has_value = true;
        result.x = rect.x + root_position.x;
        result.y = rect.y + root_position.y;
        false
    });
    result
}

#[repr(C)]
pub struct FfiVisualLine {
    pub start_offset: usize,
    pub end_offset: usize,
    pub end_offset_with_trailing_whitespace: usize,
    pub has_fragments: bool,
    pub owner_paintable: u32,
    pub line_index: u32,
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_text_visual_lines(
    arena: *mut c_void,
    primary: NodeSlotId,
    context: *mut c_void,
    push: unsafe extern "C" fn(*mut c_void, FfiVisualLine),
) {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    for line in crate::painting::visual_lines::collect_visual_lines(&paintable_rows, node_slots) {
        // SAFETY: The consumer copies the POD line synchronously.
        unsafe {
            push(
                context,
                FfiVisualLine {
                    start_offset: line.start_offset,
                    end_offset: line.end_offset,
                    end_offset_with_trailing_whitespace: line.end_offset_with_trailing_whitespace,
                    has_fragments: line.has_fragments,
                    owner_paintable: line.owner.index,
                    line_index: line.line_index,
                },
            );
        }
    }
}

fn has_rendered_text_matching(
    arena: &impl PaintableRowsRead,
    node_slots: &[NodeSlotId],
    matches: impl Fn(&FragmentRecord) -> bool,
) -> bool {
    let mut found = false;
    crate::painting::text_fragment::for_each_fragment_of_nodes(arena, node_slots, |_, _, fragment| {
        if fragment.length_in_code_units > 0 && matches(fragment) {
            found = true;
            return false;
        }
        true
    });
    found
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_text_has_rendered_text_before(
    arena: *mut c_void,
    primary: NodeSlotId,
    offset: usize,
) -> bool {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    has_rendered_text_matching(&paintable_rows, node_slots, |fragment| {
        fragment.dom_start_offset_in_node < offset
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_text_has_rendered_text_after(
    arena: *mut c_void,
    primary: NodeSlotId,
    offset: usize,
) -> bool {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    has_rendered_text_matching(&paintable_rows, node_slots, |fragment| {
        fragment.dom_end_offset_in_node > offset
    })
}

#[repr(C)]
pub struct FfiOptionalCssPixels {
    pub has_value: bool,
    pub value: CssPixels,
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_line_caret_inline_coordinate(
    arena: *mut c_void,
    owner_paintable: u32,
    line_index: u32,
    primary: NodeSlotId,
    offset: usize,
) -> FfiOptionalCssPixels {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    let coordinate = crate::painting::visual_lines::caret_inline_coordinate(
        &paintable_rows,
        owner_paintable,
        line_index,
        node_slots,
        offset,
    );
    match coordinate {
        Some(value) => FfiOptionalCssPixels { has_value: true, value },
        None => FfiOptionalCssPixels {
            has_value: false,
            value: CssPixels::from_raw(0),
        },
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_line_offset_closest_to_inline_coordinate(
    arena: *mut c_void,
    owner_paintable: u32,
    line_index: u32,
    primary: NodeSlotId,
    inline_coordinate: CssPixels,
    fallback_offset: usize,
) -> usize {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    crate::painting::visual_lines::offset_closest_to_inline_coordinate(
        &paintable_rows,
        owner_paintable,
        line_index,
        node_slots,
        inline_coordinate,
    )
    .unwrap_or(fallback_offset)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document
/// thread, and
/// `rect_to_viewport_transform` must satisfy `rect_to_viewport_transform_from_ffi`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_text_range_rects(
    arena: *mut c_void,
    primary: NodeSlotId,
    selection_state: u8,
    range_start_offset: usize,
    range_end_offset: usize,
    filter_dom_start: usize,
    filter_dom_end: usize,
    rect_to_viewport_transform: FfiRectToViewportTransform,
    context: *mut c_void,
    push_rect: unsafe extern "C" fn(*mut c_void, FfiCssPixelRect),
) {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    let rect_to_viewport_transform = unsafe { rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform) };

    let fragments = paintable_rows.text_fragments(primary);
    let node_slots = fragments.as_slice();
    crate::painting::text_fragment::for_each_fragment_of_nodes(&paintable_rows, node_slots, |block, _, fragment| {
        let fragment_dom_start = fragment.dom_start_offset_in_node;
        let fragment_dom_end = fragment.dom_end_offset_in_node;
        if fragment_dom_end <= filter_dom_start || fragment_dom_start >= filter_dom_end {
            return true;
        }

        let rect = crate::painting::text_fragment::range_rect(
            &paintable_rows,
            fragment,
            selection_state,
            range_start_offset,
            range_end_offset,
        );

        let rect_in_viewport_space = if paintable_rows.slot_is_live(block) {
            crate::painting::rect_to_viewport_transform::transform_rect_to_viewport_or_identity(
                rect_to_viewport_transform.as_ref(),
                &paintable_rows,
                block,
                rect,
            )
        } else {
            rect
        };

        // SAFETY: The consumer copies the plain-data rect synchronously.
        unsafe { push_rect(context, rect_in_viewport_space.into()) };
        true
    });
}

#[repr(C)]
pub struct FfiOptionalCssPixelRect {
    pub has_value: bool,
    pub rect: FfiCssPixelRect,
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_first_fragment_rect_for_node(
    arena: *mut c_void,
    block: NodeSlotId,
    node: NodeSlotId,
) -> FfiOptionalCssPixelRect {
    let mut result = FfiOptionalCssPixelRect {
        has_value: false,
        rect: FfiCssPixelRect::default(),
    };
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(block) {
        return result;
    }
    for fragment in paintable_rows.committed_side_data(block).fragments() {
        if fragment.layout_node != node {
            continue;
        }
        result.has_value = true;
        result.rect = crate::painting::text_fragment::absolute_rect(&paintable_rows, fragment).into();
        break;
    }
    result
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `consume` copies the byte span synchronously.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_stacking_context_structure_verification_report(
    arena: *mut c_void,
    viewport: NodeSlotId,
    context: *mut c_void,
    consume: unsafe extern "C" fn(*mut c_void, *const u8, usize),
) {
    let arena = unsafe { arena_from_handle(arena) };
    let report = crate::painting::stacking_context::verify::verification_report(arena, viewport);
    if !report.is_empty() {
        // SAFETY: The consumer copies the byte span synchronously.
        unsafe { consume(context, report.as_ptr(), report.len()) };
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_grid_layout_json(
    arena: *mut c_void,
    paintable: NodeSlotId,
    container_node_id: i64,
    context: *mut c_void,
    consume: unsafe extern "C" fn(*mut c_void, *const u8, usize),
) {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(paintable) {
        return;
    }
    if let Some(data) = crate::painting::paintable_geometry::committed_grid_layout_data(&paintable_rows, paintable) {
        let json = crate::painting::devtools_layout::serialize_grid_layout(&data, container_node_id);
        unsafe { consume(context, json.as_ptr(), json.len()) };
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_flex_layout_json(
    arena: *mut c_void,
    paintable: NodeSlotId,
    container_node_id: i64,
    context: *mut c_void,
    consume: unsafe extern "C" fn(*mut c_void, *const u8, usize),
    document_context: *mut c_void,
    resolve_node_id: unsafe extern "C" fn(*mut c_void, u32) -> i64,
) {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(paintable) {
        return;
    }
    if let Some(data) = crate::painting::paintable_geometry::committed_flex_layout_data(&paintable_rows, paintable) {
        // SAFETY: The host answers synchronously from the document the paintable belongs to.
        let json = crate::painting::devtools_layout::serialize_flex_layout(&data, container_node_id, |style_node| {
            let node_id = unsafe { resolve_node_id(document_context, style_node) };
            (node_id >= 0).then_some(node_id)
        });
        unsafe { consume(context, json.as_ptr(), json.len()) };
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_used_grid_tracks(
    arena: *mut c_void,
    paintable: NodeSlotId,
    columns: bool,
) -> *const c_void {
    let paintable_rows = unsafe { main_side_paintable_rows(arena) };
    if !paintable_rows.paintable_row_is_populated(paintable) {
        return std::ptr::null();
    }
    let Some(tracks) = crate::painting::paintable_geometry::committed_used_grid_tracks(&paintable_rows, paintable)
    else {
        return std::ptr::null();
    };
    let list = if columns { &tracks.columns } else { &tracks.rows };
    std::sync::Arc::into_raw(std::sync::Arc::new(list.style_value())).cast()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread. The
/// returned tree is retained; the caller owns one reference and releases it with
/// `visual_context_tree_release`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_main_visual_context_tree_retain(arena: *mut c_void) -> *const c_void {
    if crate::stage_thread::reads_beside_recording_of(arena) {
        // Beside the recording in flight, the tree as published with the rows it reads.
        return unsafe { main_side_paintable_rows(arena) }
            .visual_context_tree()
            .map_or(std::ptr::null(), |tree| std::sync::Arc::into_raw(tree).cast());
    }
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state
        .visual_context
        .tree
        .as_ref()
        .map_or(std::ptr::null(), |tree| {
            std::sync::Arc::into_raw(std::sync::Arc::clone(tree)).cast()
        })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_has_visual_context_tree(arena: *mut c_void) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state.visual_context.tree.is_some()
}

/// Publishes the scroll offset a box holds, as the DOM stores it. Called wherever that stored
/// offset can change: a box becoming the box of something that holds one, a write of the stored
/// offset, and a move of the viewport's offset.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_scroll_offset(
    arena: *mut c_void,
    slot: NodeSlotId,
    offset: FfiCssPixelPoint,
    dom_target_stores_offset: bool,
) {
    let arena = unsafe { arena_from_handle(arena) };
    let _write = arena.join_frame_for_main_side_write(LayoutNodeArena::SCROLL_OFFSETS_WRITER);
    arena.set_node_flag(
        slot,
        crate::layout::node_data::NodeFlag::HasScrollOffset,
        dom_target_stores_offset,
    );
    arena.scroll_offsets().publish(slot, offset.into());
}

/// One `<area>` of an image map, as the document hands it over: the style-tree identity to name as
/// the hit target, the state of its `shape` attribute, and where its parsed `coords` sit in the
/// flat array published beside it.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiImageMapArea {
    pub style_node: u32,
    pub shape: u8,
    pub editable: u8,
    pub coords_offset: u32,
    pub coords_count: u32,
}

/// Publishes the `<area>` elements of the image map an image is associated with, in tree order.
/// The document publishes them when the image takes a box and whenever the association or the
/// areas themselves can have changed, so a hit test reads the row instead of the DOM. Publishing
/// no area is how an image with no image map is named.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread. `areas`
/// must point at `area_count` areas and `coords` at `coords_count` values, and every area's
/// coordinate range must lie within them.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_image_map_areas(
    arena: *mut c_void,
    slot: NodeSlotId,
    areas: *const FfiImageMapArea,
    area_count: usize,
    coords: *const f64,
    coords_count: usize,
) {
    use crate::painting::image_map_areas::{AreaShape, PublishedImageMapArea};
    let arena = unsafe { arena_from_handle(arena) };
    let areas = if area_count == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(areas, area_count) }
    };
    let coords = if coords_count == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(coords, coords_count) }
    };
    let published = areas
        .iter()
        .map(|area| {
            let start = area.coords_offset as usize;
            let end = start + area.coords_count as usize;
            PublishedImageMapArea {
                style_node: area.style_node,
                shape: AreaShape::from_raw(area.shape),
                editable: area.editable != 0,
                coords: coords[start..end].to_vec().into_boxed_slice(),
            }
        })
        .collect::<Vec<_>>()
        .into_boxed_slice();
    let _write = arena.join_frame_for_main_side_write("image map areas");
    arena.image_map_areas().publish(slot, published);
}

/// The style-tree identity of the first `<area>` of the image's map, in tree order, whose shape
/// covers the point. Zero when the image has no map, or when no shape covers the point.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_image_map_area_for_point(
    arena: *mut c_void,
    slot: NodeSlotId,
    x: f32,
    y: f32,
    image_width: f32,
    image_height: f32,
) -> u32 {
    unsafe { main_side_paintable_rows(arena) }
        .with_image_map_areas(|areas| areas.area_for_point(slot, x, y, image_width, image_height))
}

/// Whether the `<area>` of this image named by `style_node` is editable or an editing host: 1 or
/// 0, and -1 when the identity names no area of this image. An area is never rendered, so this is
/// where the fact every other hit target publishes onto its own row lives.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_image_map_area_editability(
    arena: *mut c_void,
    slot: NodeSlotId,
    style_node: u32,
) -> i8 {
    unsafe { main_side_paintable_rows(arena) }.with_image_map_areas(|areas| areas.area_editability(slot, style_node))
}

/// Publishes what the render side needs to know about the viewport it draws into. The document
/// publishes it before each pass that reads it, so no pass asks for it.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_visual_context_tree_inputs(
    arena: *mut c_void,
    inputs: crate::painting::host::FfiVisualContextTreeInputs,
) {
    let arena = unsafe { arena_from_handle(arena) };
    let _write = arena.join_frame_for_main_side_write("visual context tree inputs");
    arena.publish_visual_context_tree_inputs(inputs);
}

/// Publishes the unique node id of what a box is the box of, as the document names it. Called
/// wherever that answer can change: a box being built for a DOM node, a box becoming the box of a
/// pseudo-element, and the viewport's box, which is the document's.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_unique_node_id(arena: *mut c_void, slot: NodeSlotId, id: i64) {
    unsafe { arena_from_handle(arena) }.unique_node_ids().publish(slot, id);
}

/// The scroll offset last published for a box, or zero for one that holds none.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_published_scroll_offset(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelPoint {
    unsafe { main_side_paintable_rows(arena) }.scroll_offset(slot).into()
}

/// The number of layout commits this arena has published. It does not say whether layout is up
/// to date - a reader asks the arena that - but it does say whether the committed geometry a
/// reader saw earlier is still the one published. A recording commits no layout, so this is asked
/// beside one.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_layout_commit_generation(arena: *mut c_void) -> u64 {
    unsafe { arena_from_handle_beside_recording(arena) }.layout_commit_generation()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_context_tree_structural_epoch(arena: *mut c_void) -> u64 {
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state.visual_context.structural_epoch()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_context_tree_has_visual_animations(arena: *mut c_void) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state
        .visual_context
        .tree
        .as_deref()
        .is_some_and(|tree| tree.has_visual_animations())
}

/// # Safety
///
/// The returned handle is owned by the caller until `compositor_animation_effect_state_destroy`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_state_create() -> *mut c_void {
    Box::into_raw(Box::new(
        crate::painting::visual_animation_builder::CompositorAnimationEffectState::default(),
    ))
    .cast()
}

/// # Safety
///
/// `state` must be a handle from `compositor_animation_effect_state_create` that is given up here.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_state_destroy(state: *mut c_void) {
    if state.is_null() {
        return;
    }
    // SAFETY: The caller gives up the handle it created.
    drop(unsafe {
        Box::from_raw(state.cast::<crate::painting::visual_animation_builder::CompositorAnimationEffectState>())
    });
}

/// Builds the animation of the request's target kind for the effect and keeps it pending with the
/// effect. The target's nodes come from the arena's main tree.
///
/// # Safety
///
/// `state` must be a live effect state handle, `arena` a live handle from `layout_arena_create`
/// used on the document thread, and the request and host, with every range they address, live
/// for the call. The host's callbacks run synchronously and may not touch the arena.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_build(
    state: *mut c_void,
    arena: *mut c_void,
    request: *const crate::painting::host::FfiCompositorAnimationRequest,
    host: *const crate::painting::host::FfiCompositorAnimationHost,
) -> crate::painting::host::FfiCompositorAnimationBuildOutcome {
    use crate::painting::visual_animation_builder::{Host, Request, effect_state_from_handle};
    let state = unsafe { effect_state_from_handle(state) };
    let arena = unsafe { arena_from_handle(arena) };
    let request = unsafe { Request::new(&*request) };
    let host = Host::new(unsafe { &*host });
    let tree = arena.paint_state().borrow().visual_context.tree.clone();
    state.build(&request, &host, |kind| {
        arena.paintable_visual_animation_target_indices(request.layout_node(), tree.as_deref(), kind)
    })
}

/// # Safety
///
/// `state` must be a live effect state handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_discard_pending(
    state: *mut c_void,
    kind: crate::painting::host::FfiVisualAnimationTargetKind,
) {
    unsafe { crate::painting::visual_animation_builder::effect_state_from_handle(state) }.discard_pending(kind);
}

/// # Safety
///
/// `state` must be a live effect state handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_has_pending(state: *mut c_void) -> bool {
    unsafe { crate::painting::visual_animation_builder::effect_state_from_handle(state) }.has_pending()
}

/// # Safety
///
/// `state` must be a live effect state handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_clear_pending(state: *mut c_void) {
    unsafe { crate::painting::visual_animation_builder::effect_state_from_handle(state) }.clear_pending();
}

/// Publishes the effect's pending animations: they become the ones it retains, and copies join the
/// document's list for the current update pass.
///
/// # Safety
///
/// `state` must be a live effect state handle and `arena` a live handle from `layout_arena_create`
/// used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_publish_pending(
    state: *mut c_void,
    arena: *mut c_void,
    reuse_retained_timing_anchors: bool,
) {
    let animations = unsafe { crate::painting::visual_animation_builder::effect_state_from_handle(state) }
        .publish_pending(reuse_retained_timing_anchors);
    let arena = unsafe { arena_from_handle(arena) };
    arena
        .paint_state()
        .borrow_mut()
        .visual_context
        .pending_compositor_animations
        .extend(animations);
}

/// # Safety
///
/// `state` must be a live effect state handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_has_retained(state: *mut c_void) -> bool {
    unsafe { crate::painting::visual_animation_builder::effect_state_from_handle(state) }.has_retained()
}

/// # Safety
///
/// `state` must be a live effect state handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_clear_retained(state: *mut c_void) {
    unsafe { crate::painting::visual_animation_builder::effect_state_from_handle(state) }.clear_retained();
}

/// # Safety
///
/// `state` must be a live effect state handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_reset(state: *mut c_void) {
    unsafe { crate::painting::visual_animation_builder::effect_state_from_handle(state) }.reset();
}

/// # Safety
///
/// The request and host, with every range they address, must be live for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_only_translates_horizontally(
    request: *const crate::painting::host::FfiCompositorAnimationRequest,
    host: *const crate::painting::host::FfiCompositorAnimationHost,
) -> bool {
    use crate::painting::visual_animation_builder::{Host, Request, effect_only_translates_horizontally};
    let request = unsafe { Request::new(&*request) };
    effect_only_translates_horizontally(&request, &Host::new(unsafe { &*host }))
}

/// # Safety
///
/// The request and host, with every range they address, must be live for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn compositor_animation_effect_transform_preserves_axes(
    request: *const crate::painting::host::FfiCompositorAnimationRequest,
    host: *const crate::painting::host::FfiCompositorAnimationHost,
) -> bool {
    use crate::painting::visual_animation_builder::{Host, Request, effect_transform_preserves_axes};
    let request = unsafe { Request::new(&*request) };
    effect_transform_preserves_axes(&request, &Host::new(unsafe { &*host }))
}

/// Starts an update pass: the effects publish into an empty document list.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_begin_compositor_animation_update(arena: *mut c_void) {
    let arena = unsafe { arena_from_handle(arena) };
    arena
        .paint_state()
        .borrow_mut()
        .visual_context
        .pending_compositor_animations
        .clear();
}

/// Gives the arena's main tree the animations the update pass published, or none.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_compositor_animations(
    arena: *mut c_void,
    publish_pending: bool,
) -> crate::painting::host::FfiCompositorAnimationPublishOutcome {
    let arena = unsafe { arena_from_handle(arena) };
    let mut paint_state = arena.paint_state().borrow_mut();
    crate::painting::visual_context::publish_compositor_animations(&mut paint_state.visual_context, publish_pending)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread; the
/// sink pointer must stay valid for this synchronous call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_visit_chrome_widgets(
    arena: *mut c_void,
    sink: *mut c_void,
    visit: unsafe extern "C" fn(*mut c_void, NodeSlotId, u8),
) {
    with_hit_test_list_items_only(arena, (), |list, _arena| {
        for item in list.items.iter() {
            if item.chrome_widget_kind == crate::painting::hit_test::CHROME_WIDGET_NONE {
                continue;
            }
            // SAFETY: The C++ host consumes the visit synchronously.
            unsafe { visit(sink, item.paintable, item.chrome_widget_kind) };
        }
    });
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// the callback context and function pointers must remain valid for this synchronous call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_caret_line_for_position(
    arena: *mut c_void,
    query: crate::painting::host::FfiCaretPositionQuery,
    offset: usize,
    affinity_is_downstream: bool,
) -> crate::painting::host::FfiCaretLineForPosition {
    with_hit_test_list_and_caret_lines(arena, Default::default(), |list, arena| {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
        match list.caret_line_for_position(arena, &query, offset, affinity_is_downstream) {
            Some(line_index) => crate::painting::host::FfiCaretLineForPosition {
                has_line: true,
                line_index,
            },
            None => Default::default(),
        }
    })
}

/// # Safety
///
/// `sink` must be the pointer handed to the callback, used synchronously; `bytes` must point at
/// `length` readable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paint_push_bytes(sink: *mut c_void, bytes: *const u8, length: usize) {
    // SAFETY: `sink` is the Vec pointer handed out by the host callback wrapper; the caller
    // guarantees the byte range.
    let vec = unsafe { &mut *sink.cast::<Vec<u8>>() };
    if length > 0 {
        vec.extend_from_slice(unsafe { std::slice::from_raw_parts(bytes, length) });
    }
}

/// # Safety
///
/// Every pointer in `primitive` must be readable for the length that accompanies it, each UTF-16
/// view must satisfy [`FfiUtf16View::units`], and each non-null component transfer table must
/// hold 256 bytes.
unsafe fn svg_filter_primitive_from_ffi(primitive: &FfiSvgFilterPrimitive) -> SvgFilterPrimitive {
    let name = |view: FfiUtf16View| unsafe { view.to_utf16() }.unwrap_or_default();
    SvgFilterPrimitive {
        values: primitive.values,
        in1: name(primitive.in1),
        in2: name(primitive.in2),
        result: name(primitive.result),
        merge_inputs: unsafe { ffi_slice(primitive.merge_inputs, primitive.merge_input_count) }
            .iter()
            .map(|view| name(*view))
            .collect(),
        color_matrix_values: unsafe { ffi_slice(primitive.color_matrix_values, primitive.color_matrix_value_count) }
            .to_vec(),
        component_transfer_tables: primitive.component_transfer_tables.map(|table| {
            (!table.is_null()).then(|| {
                let table = unsafe { ffi_slice(table, 256) };
                Box::new(<[u8; 256]>::try_from(table).expect("a component transfer table holds 256 entries"))
            })
        }),
        image_frame: (!primitive.image_frame.is_null())
            .then(|| unsafe { libgfx_rust::image_frame::ImageFrameHandle::retain(primitive.image_frame) }),
    }
}

/// # Safety
///
/// `sink` must be the pointer handed to the callback, used synchronously; `primitive` must be
/// readable, with every pointer in it readable for the length that accompanies it, each UTF-16
/// view satisfying [`FfiUtf16View::units`], each non-null component transfer table holding
/// 256 bytes, and `image_frame` null or pointing to a live `Gfx::DecodedImageFrame`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paint_push_svg_filter_primitive(
    sink: *mut c_void,
    primitive: *const FfiSvgFilterPrimitive,
) {
    let primitives = unsafe { &mut *sink.cast::<Vec<SvgFilterPrimitive>>() };
    primitives.push(unsafe { svg_filter_primitive_from_ffi(&*primitive) });
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_note_svg_paint_resources_changed(arena: *mut c_void) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    arena.svg_paint_resources().note_changed()
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_has_enrolled_svg_paint_resources(arena: *mut c_void) -> bool {
    let arena = unsafe { arena_from_handle(arena) };
    arena.svg_paint_resources().has_enrolled_entries()
}

/// Serializes the graph a list of filter functions describes and hands the bytes to `append`.
/// Returns false for an empty list, which appends nothing.
///
/// # Safety
///
/// `functions` must point to `count` readable functions, and `append` must accept `context` and
/// the byte range it is handed, synchronously.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_filter_functions_serialize(
    functions: *const FfiFilterFunction,
    count: usize,
    append: unsafe extern "C" fn(*mut c_void, *const u8, usize),
    context: *mut c_void,
) -> bool {
    let functions = unsafe { ffi_slice(functions, count) };
    let Some(graph) = filter_functions_graph(functions.iter().copied().map(Filter::from)) else {
        return false;
    };
    let bytes = graph.serialize();
    unsafe { append(context, bytes.as_ptr(), bytes.len()) };
    true
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread. The
/// returned pointers borrow the last recording and stay valid until the next one replaces it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_retain_recorded_display_list(arena: *mut c_void) -> *const c_void {
    let arena = unsafe { arena_from_handle(arena) };
    let paint_state = arena.paint_state().borrow();
    paint_state
        .last_recording
        .as_ref()
        .map_or(std::ptr::null(), |recording| {
            std::sync::Arc::into_raw(recording.display_list.clone()).cast()
        })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`; `line_index` in range.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_caret_line(
    arena: *mut c_void,
    line_index: usize,
) -> crate::painting::host::FfiCaretLineExport {
    with_hit_test_list_and_caret_lines(arena, Default::default(), |list, _| {
        let line = &list.caret_lines[line_index];
        crate::painting::host::FfiCaretLineExport {
            rect: line.rect.into(),
            context: line.context,
            first_caret_item_index: line.first_caret_item_index,
            last_caret_item_index: line.last_caret_item_index,
        }
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_list_generation(arena: *mut c_void) -> u64 {
    let arena = unsafe { arena_from_handle(arena) };
    arena.try_take_in_recording();
    arena.hit_test_list.borrow().as_ref().map_or(0, |list| list.generation)
}

/// The rows a hit-test query reads, with the list and visual context tree published beside them.
/// The structures the query derives from the list are built before the rows are published.
///
/// SAFETY: Same as [`main_side_paintable_rows`].
unsafe fn hit_test_paintable_rows<'a>(
    arena: *mut c_void,
    needs_spatial_indexes: bool,
    needs_caret_lines: bool,
) -> MainSidePaintableRows<'a> {
    if !unsafe { arena_from_handle(arena) }.a_stage_is_running() {
        unsafe { arena_from_handle_mut(arena) }
            .prepare_hit_test_list_for_query(needs_spatial_indexes, needs_caret_lines);
    }
    unsafe { main_side_paintable_rows(arena) }
}

fn with_hit_test_list_items_only<R>(
    arena: *mut c_void,
    default: R,
    query: impl FnOnce(&crate::painting::hit_test::HitTestList, &MainSidePaintableRows<'_>) -> R,
) -> R {
    // SAFETY: The caller passes a live arena handle (documented on every entry point below).
    let rows = unsafe { hit_test_paintable_rows(arena, false, false) };
    rows.with_hit_test_list(|list| match list {
        Some(list) => query(list, &rows),
        None => default,
    })
}

fn with_hit_test_list_and_caret_lines<R>(
    arena: *mut c_void,
    default: R,
    query: impl FnOnce(&crate::painting::hit_test::HitTestList, &MainSidePaintableRows<'_>) -> R,
) -> R {
    // SAFETY: The caller passes a live arena handle (documented on every entry point below).
    let rows = unsafe { hit_test_paintable_rows(arena, false, true) };
    rows.with_hit_test_list(|list| match list {
        Some(list) if list.caret_lines_built => query(list, &rows),
        _ => default,
    })
}

fn with_hit_test_list_spatial_indexes_and_visual_context_tree<R>(
    arena: *mut c_void,
    needs_caret_lines: bool,
    default: R,
    query: impl FnOnce(
        &crate::painting::hit_test::HitTestList,
        &crate::painting::visual_context::VisualContextTree,
        &MainSidePaintableRows<'_>,
    ) -> R,
) -> R {
    // SAFETY: The caller passes a live arena handle (documented on every entry point below).
    let rows = unsafe { hit_test_paintable_rows(arena, true, needs_caret_lines) };
    let Some(tree) = rows.visual_context_tree() else {
        return default;
    };
    rows.with_hit_test_list(|list| match list {
        Some(list) if list.spatial_indexes_built && (!needs_caret_lines || list.caret_lines_built) => {
            query(list, &tree, &rows)
        }
        _ => default,
    })
}

fn ffi_topmost(item: Option<crate::painting::hit_test::query::TopmostItem>) -> crate::painting::host::FfiTopmostItem {
    match item {
        Some(item) => crate::painting::host::FfiTopmostItem {
            has_item: true,
            index: item.index,
            local: item.local_point.into(),
        },
        None => crate::painting::host::FfiTopmostItem::default(),
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_find_topmost_item(
    arena: *mut c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
) -> crate::painting::host::FfiTopmostItem {
    with_hit_test_list_spatial_indexes_and_visual_context_tree(arena, false, Default::default(), |list, tree, arena| {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
        ffi_topmost(list.find_topmost_item(arena, tree, &callbacks, point.into()))
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_find_topmost_items_for_caret(
    arena: *mut c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
) -> crate::painting::host::FfiTopmostItemsForCaret {
    with_hit_test_list_spatial_indexes_and_visual_context_tree(arena, false, Default::default(), |list, tree, arena| {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
        let (caret_item, hit_item) = list.find_topmost_items_for_caret(arena, tree, &callbacks, point.into());
        crate::painting::host::FfiTopmostItemsForCaret {
            caret_item: ffi_topmost(caret_item),
            hit_item: ffi_topmost(hit_item),
        }
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_all(
    arena: *mut c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
    push_context: *mut c_void,
    push: unsafe extern "C" fn(*mut c_void, usize),
) {
    let indices =
        with_hit_test_list_spatial_indexes_and_visual_context_tree(arena, false, Vec::new(), |list, tree, arena| {
            let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
            list.hit_test_all(arena, tree, &callbacks, point.into())
        });
    for index in indices {
        // SAFETY: The C++ sink consumes the index synchronously.
        unsafe { push(push_context, index) };
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_item_at_line_edge(
    arena: *mut c_void,
    line_index: usize,
    position_type: u8,
) -> usize {
    let position_type = crate::painting::hit_test::caret::CaretPositionType::from_u8(position_type);
    with_hit_test_list_and_caret_lines(arena, usize::MAX, |list, _| {
        list.item_at_line_edge(line_index, position_type)
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_caret_item_for_line(
    arena: *mut c_void,
    line_index: usize,
    point: FfiCssPixelPoint,
    mode: u8,
) -> crate::painting::host::FfiCaretItemForLine {
    with_hit_test_list_and_caret_lines(arena, Default::default(), |list, arena| {
        match list.caret_item_for_line(
            arena,
            line_index,
            point.into(),
            crate::painting::hit_test::caret::CaretPositionMode::from_u8(mode),
        ) {
            Some((item_index, position_type)) => crate::painting::host::FfiCaretItemForLine {
                has_item: true,
                item_index,
                position_type: position_type as u8,
            },
            None => Default::default(),
        }
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_line_block_coordinate(arena: *mut c_void, line_index: usize) -> i32 {
    with_hit_test_list_and_caret_lines(arena, 0, |list, _| list.line_block_coordinate(line_index).raw_value())
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_item_is_inline_adjacent_to_line(
    arena: *mut c_void,
    item_index: usize,
    line_index: usize,
) -> bool {
    with_hit_test_list_and_caret_lines(arena, false, |list, _| {
        list.item_is_inline_adjacent_to_line(item_index, line_index)
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_find_closest_line(
    arena: *mut c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
    mode: u8,
    scoped: bool,
    respect_clip: bool,
) -> crate::painting::host::FfiClosestLine {
    with_hit_test_list_spatial_indexes_and_visual_context_tree(arena, true, Default::default(), |list, tree, arena| {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
        let closest = list.find_closest_line(
            arena,
            tree,
            &callbacks,
            point.into(),
            crate::painting::hit_test::caret::CaretPositionMode::from_u8(mode),
            scoped,
            respect_clip,
        );
        crate::painting::host::FfiClosestLine {
            has_index: closest.index.is_some(),
            index: closest.index.unwrap_or(0),
            local_x: closest.local_point.x.raw_value(),
            local_y: closest.local_point.y.raw_value(),
            block_distance: closest.block_distance.raw_value(),
            block_start_distance: closest.block_start_distance.raw_value(),
            inline_distance: closest.inline_distance.raw_value(),
            is_before_point: closest.is_before_point,
            contains_point_in_block_axis: closest.contains_point_in_block_axis,
        }
    })
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_adjacent_line(
    arena: *mut c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    current_line_index: usize,
    direction: u8,
    inline_coordinate_raw: i32,
) -> crate::painting::host::FfiAdjacentLine {
    let direction = if direction == 1 {
        crate::painting::hit_test::caret::CaretLineDirection::Next
    } else {
        crate::painting::hit_test::caret::CaretLineDirection::Previous
    };
    with_hit_test_list_and_caret_lines(arena, Default::default(), |list, arena| {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
        match list.adjacent_line(
            arena,
            &callbacks,
            current_line_index,
            direction,
            CssPixels::from_raw(inline_coordinate_raw),
        ) {
            Some((line_index, point)) => crate::painting::host::FfiAdjacentLine {
                has_line: true,
                line_index,
                point_x: point.x.raw_value(),
                point_y: point.y.raw_value(),
            },
            None => Default::default(),
        }
    })
}

/// # Safety
/// `arena` is live and used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_recording_trace_enabled(arena: *mut c_void, enabled: bool) {
    unsafe { arena_from_handle(arena) }
        .paint_state()
        .borrow_mut()
        .trace_recordings = enabled;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::css_pixels::CssPixelRect;
    use crate::layout::LayoutNodeArena;
    use crate::layout::node_data::NodeKind;
    use crate::painting::hit_test::HitTestList;
    use crate::painting::host::FfiRootBackgroundSource;
    use crate::painting::paintable_data::FfiOverflowData;
    use crate::painting::record::damage::PaintDamage;
    use crate::painting::visual_context::dirty::VisualContextBoxDirtyKind;
    use crate::painting::visual_context::{TransformData, TransformDataRole, VisualContextTree};

    unsafe extern "C" {
        fn layout_arena_prepare_for_rendering(
            arena: *mut c_void,
            root_background_source: crate::painting::host::FfiRootBackgroundSource,
            visual_context_update_pending: bool,
        ) -> FfiRenderingPreparationOutcome;
    }

    #[test]
    fn hit_test_queries_measure_viewport_overflow_before_reading_it() {
        for (spatial_indexes, caret_lines) in [(true, false), (true, true), (false, true), (false, false)] {
            let mut arena = LayoutNodeArena::new();
            let viewport = arena.allocate_for_test().slot;
            arena.write_shape(viewport).set_kind(NodeKind::Viewport);
            arena.populate_paintable_row(viewport);
            arena.scrollable_overflow.viewport.set(Some(viewport));
            let root = arena.allocate_for_test().slot;
            arena.populate_paintable_row(root);
            *arena.hit_test_list.borrow_mut() = Some(std::sync::Arc::new(HitTestList::default()));
            {
                let mut state = arena.paint_state().borrow_mut();
                state.root_background_source = Some(FfiRootBackgroundSource {
                    root_layout_node: root,
                    ..Default::default()
                });
                state.visual_context.tree = Some(std::sync::Arc::new(VisualContextTree::create(TransformData {
                    matrix: libgfx_rust::FloatMatrix4x4::identity(),
                    origin: Default::default(),
                    sorting_context_root_index: None,
                    flattens_inherited_transform: false,
                    role: TransformDataRole::CssTransform,
                    synthetic_plane: false,
                    establishes_sorting_context: false,
                })));
                state.visual_context.dirty_boxes.clear();
            }
            // Leave stale overflow for the hit-test query to measure before it reads. Losing
            // scrollability must invalidate both the root background and visual context.
            arena.committed_side_data_mut(viewport).overflow_relative_to_padding_box = FfiOverflowData {
                rect: CssPixelRect::new(
                    CssPixels::from_integer(0),
                    CssPixels::from_integer(0),
                    CssPixels::from_integer(100),
                    CssPixels::from_integer(2000),
                )
                .into(),
                has_scrollable_overflow: true,
            };
            arena
                .paintable_side_data(viewport)
                .overflow_measured_this_commit
                .set(true);
            arena.note_publishing_paint_recording_started();
            arena.clear_paint_damage_consumed_by_published_recording();

            let handle = std::ptr::from_mut(&mut arena).cast();
            let query = |_: &HitTestList, rows: &MainSidePaintableRows<'_>| {
                crate::painting::paintable_geometry::scrollable_overflow_rect(rows, viewport)
            };
            let rect = if spatial_indexes {
                with_hit_test_list_spatial_indexes_and_visual_context_tree(
                    handle,
                    caret_lines,
                    None,
                    |list, _, arena| query(list, arena),
                )
            } else if caret_lines {
                with_hit_test_list_and_caret_lines(handle, None, query)
            } else {
                with_hit_test_list_items_only(handle, None, query)
            };
            assert_eq!(rect, Some(CssPixelRect::default()));
            assert!(arena.scrollable_overflow.geometry_changed.get());
            assert!(arena.scrollable_overflow.scrollability_changed.get());
            assert!(arena.paint_damage_of_row(root).contains(PaintDamage::DRAW_BACKGROUND));
            assert!(
                arena
                    .paint_damage_of_row(viewport)
                    .contains(PaintDamage::SCROLL_METADATA)
            );
            assert!(
                arena.paint_state().borrow().visual_context.dirty_boxes.boxes[&viewport]
                    .contains(VisualContextBoxDirtyKind::ScrollableOverflowFlipped)
            );
        }
    }

    #[test]
    fn preparing_for_rendering_measures_root_overflow_before_recording_reads_it() {
        // The entry mints the main thread capability from the handle, so the arena must be in one.
        let mut arena_handle = Box::new(crate::layout::ArenaHandle::new());
        let handle: *mut c_void = std::ptr::from_mut(&mut *arena_handle).cast();
        // SAFETY: The handle's arena is its first field, and nothing else borrows the handle.
        let arena = unsafe { &mut *handle.cast::<LayoutNodeArena>() };
        let viewport = arena.allocate_for_test().slot;
        arena.write_shape(viewport).set_kind(NodeKind::Viewport);
        arena.populate_paintable_row(viewport);
        arena.scrollable_overflow.viewport.set(Some(viewport));
        let root = arena.allocate_for_test().slot;
        arena.write_shape(root).set_kind(NodeKind::BlockContainer);
        arena.populate_paintable_row(root);
        // A structural change invalidated the root's overflow, measured earlier in this commit,
        // without queueing a recalculation, so nothing but a query measures it again. Measuring
        // it drops its scrollable overflow.
        arena.committed_side_data_mut(root).overflow_relative_to_padding_box = FfiOverflowData {
            rect: CssPixelRect::new(
                CssPixels::from_integer(0),
                CssPixels::from_integer(0),
                CssPixels::from_integer(100),
                CssPixels::from_integer(2000),
            )
            .into(),
            has_scrollable_overflow: true,
        };
        arena.paintable_side_data(root).overflow_measured_this_commit.set(true);
        arena.paint_state().borrow_mut().visual_context.dirty_boxes.clear();

        let outcome = unsafe {
            layout_arena_prepare_for_rendering(
                handle,
                FfiRootBackgroundSource {
                    root_layout_node: root,
                    ..Default::default()
                },
                false,
            )
        };
        assert!(outcome.requires_visual_context_update);
        assert!(
            arena.paint_state().borrow().visual_context.dirty_boxes.boxes[&root]
                .contains(VisualContextBoxDirtyKind::ScrollableOverflowFlipped)
        );

        // Recording reads the root's overflow while it holds the paint state.
        let _paint_state = arena.paint_state().borrow();
        let canvas_rect = crate::painting::record::paint::background_resolution::root_background_canvas_rect(
            &arena.paintable_rows(),
            root,
            CssPixelRect::default(),
        );
        assert_eq!(canvas_rect, CssPixelRect::default());
    }
}
