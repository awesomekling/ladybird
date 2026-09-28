/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::css::css_pixels::{CssPixelPoint, CssPixelRect, CssPixels};
use crate::css::ffi_support::FfiUtf16View;
use crate::layout::LayoutNodeArena;
use crate::layout::node_data::NodeSlotId;
use crate::layout::row_reads::RowSnapshot;
use crate::layout::svg_formatting_context;
use crate::layout::used_values::FfiCssPixelPoint;
use crate::layout::used_values::FfiCssPixelRect;
use crate::layout::used_values::FfiCssPixelSize;
use crate::painting::display_list::commands::{ContextRef, SpatialNodeIndex};
use crate::painting::filter_bytes::{FfiFilterFunction, filter_functions_graph};
use crate::painting::force_dark::ForceDarkRole;
use crate::painting::geometry_read::GeometryRead;
use crate::painting::host::visual_context::FfiSvgFilterPrimitive;
use crate::painting::paintable_data::*;
use crate::painting::paintable_rows::{PaintableRowsRead, with_inline_pieces};
use crate::painting::published_frame::{PaintRead, PaintSource};
use crate::painting::rect_to_viewport_transform::RectToViewportTransform;
use crate::painting::scroll_chain::ViewportWheelOverflow;
use crate::painting::svg_filter::SvgFilterPrimitive;
use libcompositing_rust::ffi::{ffi_slice, tree_from_handle};
use libgfx_rust::filter::Filter;
use std::cell::RefCell;
use std::ffi::c_void;

mod main_thread_entries;

pub(crate) use main_thread_entries::MainThreadFfiEntry;

/// Reads the paint state of the document whose arena `arena` names through the rows its owner published, as of every
/// change the document thread sent ([`RowSnapshot::current`]).
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and `read` may not let the
/// owner publish again (it calls nothing of the host).
pub(crate) unsafe fn read_current<R>(arena: *mut c_void, read: impl FnOnce(&PaintSource<'_>) -> R) -> R {
    let absolute_rects = RefCell::default();
    // SAFETY: Guaranteed by the caller.
    read(&PaintSource::of_rows(
        unsafe { RowSnapshot::current(arena) },
        &absolute_rects,
    ))
}

/// Like [`read_current`], of the rows as committed ([`RowSnapshot::committed`]): with their scrollable overflow
/// measured.
///
/// # Safety
///
/// As for [`read_current`].
pub(crate) unsafe fn read_committed<R>(arena: *mut c_void, read: impl FnOnce(&PaintSource<'_>) -> R) -> R {
    let absolute_rects = RefCell::default();
    // SAFETY: Guaranteed by the caller.
    read(&PaintSource::of_rows(
        unsafe { RowSnapshot::committed(arena) },
        &absolute_rects,
    ))
}

/// The arena `arena` names, for a host call that a unit the owner runs for its document makes on the owner, which
/// holds the arena: a clock tick's, a flight's present stage.
///
/// # Safety
///
/// `arena` must be the live arena handle of the unit the owner runs on the calling thread, with no other borrow of it
/// live while the returned one is used.
unsafe fn arena_of_owner_unit<'a>(arena: *mut c_void) -> &'a mut LayoutNodeArena {
    debug_assert!(
        crate::stage_thread::on_owner_thread(),
        "only a unit the owner runs holds the arena"
    );
    // SAFETY: Guaranteed by the caller.
    crate::render_owner::do_owner_work_here(|owner| unsafe {
        &mut *crate::layout::ArenaHandle::held_by_waiting_thread(owner, arena)
    })
    .arena_mut()
}

/// Sends the owner of the document whose arena `arena` names the paint state write `change`.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
unsafe fn send(arena: *mut c_void, change: crate::painting::paint_changes::PaintChange) {
    // SAFETY: Guaranteed by the caller.
    unsafe { crate::painting::paint_changes::send(arena, change) };
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ScrollbarEnlarged {
                slot,
                direction,
                enlarged,
            },
        );
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_physical_resize_axes(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiPhysicalResizeAxes {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            let axes = crate::painting::chrome_geometry::physical_resize_axes(rows, slot);
            FfiPhysicalResizeAxes {
                horizontal: axes.horizontal,
                vertical: axes.vertical,
            }
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_is_chrome_mirrored(arena: *mut c_void, slot: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            crate::painting::chrome_geometry::is_chrome_mirrored(rows, slot)
        })
    }
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
    let scroll_state = has_device_scroll_offset.then_some(crate::painting::chrome_geometry::ScrollbarScrollState {
        device_scroll_offset,
        device_pixels_per_css_pixel,
    });
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let data = crate::painting::chrome_geometry::ChromeGeometry {
                arena: paintable_rows,
                metrics,
                viewport_wheel_overflow_x: viewport_overflow_x,
                viewport_wheel_overflow_y: viewport_overflow_y,
            }
            .compute_scrollbar_data(slot, direction, enlarged, scroll_state);
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
        })
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            crate::painting::chrome_geometry::minimum_scroll_offset(paintable_rows, slot).into()
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_maximum_scroll_offset(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelPoint {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            crate::painting::chrome_geometry::maximum_scroll_offset(paintable_rows, slot).into()
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let axes = crate::painting::chrome_geometry::wheel_scrollable_axes(
                paintable_rows,
                slot,
                viewport_overflow_x,
                viewport_overflow_y,
            );
            FfiPhysicalResizeAxes {
                horizontal: axes.horizontal,
                vertical: axes.vertical,
            }
        })
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ChromeStateListens(true),
        );
    };
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_clear_chrome_state_callback(arena: *mut c_void) {
    unsafe { crate::layout::HostTables::from_handle(arena) }
        .chrome_state_callback
        .set(None);
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ChromeStateListens(false),
        );
    };
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_establishes_an_absolute_positioning_containing_block(
    arena: *mut c_void,
    node: NodeSlotId,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            crate::painting::style_queries::establishes_positioning_containing_blocks(rows, node).0
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_establishes_a_fixed_positioning_containing_block(
    arena: *mut c_void,
    node: NodeSlotId,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            crate::painting::style_queries::establishes_positioning_containing_blocks(rows, node).1
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_any_ancestor_establishes_a_fixed_position_containing_block(
    arena: *mut c_void,
    node: NodeSlotId,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            crate::painting::style_queries::any_ancestor_establishes_a_fixed_position_containing_block(rows, node)
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_has_css_transform(arena: *mut c_void, node: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            rows.node_style_if_live(node)
                .is_some_and(|style| crate::painting::style_queries::has_css_transform(rows, node, style))
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `node` must name a live node in this arena.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_invalidate_nearest_self_painting_inline_paint_cache(
    arena: *mut c_void,
    node: NodeSlotId,
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::NearestSelfPaintingInlineRepaint(node),
        );
    }
}

/// The fields of a committed paintable row that C++ reads, copied out of the published rows.
/// `is_populated` is false, and the rest default, when the slot has no committed box.
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
/// its row. It reads the rows' tree alone: a style or paint change the document thread sent, as
/// its style drain does per element, leaves the rows it reads current.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_has_committed_box(arena: *mut c_void, slot: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::current_tree(arena) }
        .paintable
        .paintable_row_is_populated(slot)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_committed_row(arena: *mut c_void, slot: NodeSlotId) -> FfiCommittedRow {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
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
        })
    }
}

/// Clears the paint state of `layout_node`'s row, whose box is going away, and hands back its
/// reset.
///
/// # Safety
///
/// `arena` must be live, on the thread that owns it, with no borrow of it held across the call.
pub(crate) unsafe fn clear_paintable_row_of_node(arena: *mut LayoutNodeArena, layout_node: NodeSlotId) {
    let reset = {
        // SAFETY: Guaranteed by the caller.
        let arena = unsafe { &*arena };
        arena.clear_committed_fragment_link(layout_node);
        arena.prepare_paintable_row_cleared_reset(layout_node)
    };
    if let Some(reset) = reset {
        // SAFETY: As above.
        unsafe { &*arena }.hand_back_paintable_row_reset(reset);
        // SAFETY: As above; the shared borrow has ended.
        unsafe { &mut *arena }.paintable_row_cleared(reset);
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_has_child_paintables(arena: *mut c_void, slot: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            crate::painting::paint_order::first_paint_child(rows, slot).is_some()
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    let snapshot = unsafe { crate::painting::selection::SelectionSnapshot::from_ffi(&*snapshot) };
    // SAFETY: Guaranteed by the caller.
    unsafe { send(arena, crate::painting::paint_changes::PaintChange::Selection(snapshot)) };
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_selection_clear(arena: *mut c_void, viewport: NodeSlotId) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::SelectionCleared { viewport },
        );
    };
}

#[derive(Default)]
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let Some(rect) = crate::painting::paintable_geometry::scrollable_overflow_rect(paintable_rows, slot) else {
                return FfiOptionalOverflowData::default();
            };
            let mut value = paintable_rows
                .committed_side_data(slot)
                .overflow_relative_to_padding_box;
            value.rect = rect.into();
            FfiOptionalOverflowData { has_value: true, value }
        })
    }
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
        arena
            .paint_state()
            .borrow_mut()
            .update_root_background_source(arena, root_background_source)
    };
    let clamped = crate::painting::scrollable_overflow::measure_and_find_scroll_offsets_to_clamp(arena);
    (background_source_changed, clamped)
}

/// [`prepare_root_background_and_overflow`] with the root background source the arena's tree derives.
pub(crate) fn root_background_and_overflow(arena: &mut LayoutNodeArena) -> (bool, Vec<(NodeSlotId, CssPixelPoint)>) {
    let root_background_source = crate::layout::root_background_source(arena);
    prepare_root_background_and_overflow(arena, root_background_source)
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
    // SAFETY: Guaranteed by the caller.
    let count = unsafe { RowSnapshot::settled(arena) }
        .paint_status
        .scrollable_overflow_recalculations;
    if reset {
        // SAFETY: As above.
        unsafe {
            send(
                arena,
                crate::painting::paint_changes::PaintChange::ScrollableOverflowRecalculationCountReset,
            );
        }
    }
    count
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

impl FfiBoxModelMetrics {
    /// The box model of the committed box in `slot`, which `rows` has populated.
    pub(crate) fn of(rows: &impl crate::painting::geometry_read::GeometryRead, slot: NodeSlotId) -> Self {
        Self {
            margin: crate::painting::paintable_geometry::committed_margin(rows, slot),
            padding: crate::painting::paintable_geometry::committed_padding(rows, slot),
            border: crate::painting::paintable_geometry::committed_border(rows, slot),
            inset: crate::painting::paintable_geometry::committed_inset(rows, slot),
        }
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_content_size(arena: *mut c_void, slot: NodeSlotId) -> FfiCssPixelSize {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(slot) {
                return FfiCssPixelSize::default();
            }
            crate::painting::paintable_geometry::committed_content_size(paintable_rows, slot)
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_svg_viewport_size(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelSize {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(slot) {
                return FfiCssPixelSize::default();
            }
            crate::painting::paintable_geometry::committed_svg_viewport_size(paintable_rows, slot)
        })
    }
}

#[derive(Default)]
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let transform = if paintable_rows.paintable_row_is_populated(slot) {
                crate::painting::paintable_geometry::committed_svg_viewport_transform(paintable_rows, slot)
            } else {
                None
            };
            FfiOptionalAffineTransform {
                has_value: transform.is_some(),
                transform: transform.unwrap_or_default(),
            }
        })
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(slot) {
                return FfiCssPixelRect::default();
            }
            let Some(style) = paintable_rows.node_style_if_live(slot) else {
                return FfiCssPixelRect::default();
            };
            crate::painting::visual_context::node_values::transform_reference_box(style, paintable_rows, slot).into()
        })
    }
}

/// # Safety
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_box_model(arena: *mut c_void, slot: NodeSlotId) -> FfiBoxModelMetrics {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(slot) {
                return FfiBoxModelMetrics::default();
            }
            FfiBoxModelMetrics::of(paintable_rows, slot)
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_is_positioned(arena: *mut c_void, slot: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            paintable_rows.paintable_row_is_populated(slot)
                && crate::painting::style_queries::is_positioned(paintable_rows, slot)
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_absolute_rect(arena: *mut c_void, slot: NodeSlotId) -> FfiCssPixelRect {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            crate::painting::paintable_geometry::absolute_rect_or_default(paintable_rows, slot).into()
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_absolute_padding_box_rect(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelRect {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(slot) {
                return FfiCssPixelRect::default();
            }
            crate::painting::paintable_geometry::absolute_padding_box_rect(paintable_rows, slot).into()
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_absolute_border_box_rect(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> FfiCssPixelRect {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(slot) {
                return FfiCssPixelRect::default();
            }
            crate::painting::paintable_geometry::absolute_border_box_rect(paintable_rows, slot).into()
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_physical_overflow_directions(
    arena: *mut c_void,
    paintable: NodeSlotId,
) -> FfiPhysicalOverflowDirections {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let directions = if paintable_rows.paintable_row_is_populated(paintable) {
                crate::painting::scrollable_overflow::physical_overflow_directions(paintable_rows, paintable)
            } else {
                crate::painting::scrollable_overflow::PhysicalOverflowDirections::default()
            };
            FfiPhysicalOverflowDirections {
                horizontal_axis_is_positive: directions.horizontal_axis_is_positive,
                vertical_axis_is_positive: directions.vertical_axis_is_positive,
            }
        })
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
    let kind = match kind {
        FfiVisualContextBoxDirtyKind::StyleValueChange => VisualContextBoxDirtyKind::StyleValueChange,
        FfiVisualContextBoxDirtyKind::StyleStructuralChange => VisualContextBoxDirtyKind::StyleStructuralChange,
        FfiVisualContextBoxDirtyKind::ScrollableOverflowFlipped => VisualContextBoxDirtyKind::ScrollableOverflowFlipped,
    };
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::VisualContextBoxDirty { slot, kind },
        );
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::VisualContextFullRebuild(reason),
        );
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_context_pending_dirty_box_count(arena: *mut c_void) -> usize {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::settled(arena).paint_status.visual_context_dirty_boxes }
}

/// # Safety
///
/// `arena` must be a live layout arena handle used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_background_color_can_be_compositor_animated(
    arena: *mut c_void,
    slot: NodeSlotId,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            // The source the document's last preparation for rendering found, which it prepared the committed rows
            // with.
            crate::painting::record::paint::background_resolution::background_color_can_be_compositor_animated(
                rows,
                slot,
                rows.rows().paint_status.root_background_source,
            )
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe { visual_context_node_indices(arena, slot, list) }.len()
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
    // SAFETY: Guaranteed by the caller.
    let indices = unsafe { visual_context_node_indices(arena, slot, list) };
    assert!(indices.len() <= capacity);
    // SAFETY: the caller warrants `capacity` writable indices behind `out`.
    unsafe { std::ptr::copy_nonoverlapping(indices.as_ptr(), out, indices.len()) };
}

/// The indices of the visual context nodes of the `list` kind the box `slot` names owns.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
unsafe fn visual_context_node_indices(
    arena: *mut c_void,
    slot: NodeSlotId,
    list: crate::painting::host::FfiVisualContextBoxNodeList,
) -> Vec<u32> {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            use crate::painting::host::FfiVisualContextBoxNodeList;
            rows.with_paintable_visual_context_node_handles(slot, |handles| match list {
                FfiVisualContextBoxNodeList::SpatialNodes => handles.spatial.iter().map(|index| index.0).collect(),
                FfiVisualContextBoxNodeList::ClipNodes => handles.clip_handles().map(|index| index.0).collect(),
                FfiVisualContextBoxNodeList::EffectNodes => handles.effects.iter().map(|index| index.0).collect(),
            })
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        crate::painting::owner_pass::run_paint_pass_of(
            arena,
            crate::painting::owner_pass::PaintPass::AccumulatedVisualContexts,
            |arena, viewport| {
                if !arena.paintable_row_is_populated(viewport) {
                    return crate::painting::host::FfiVisualContextUpdateOutcome::default();
                }
                update_accumulated_visual_contexts_stage(arena, viewport)
            },
            viewport,
        )
    }
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
    // SAFETY: Guaranteed by the caller.
    let geometry = unsafe {
        read_committed(arena, |paintable_rows| {
            crate::painting::scroll_snap::snap_container_geometry(paintable_rows, snap_container)
        })
    };
    let Some(geometry) = geometry else {
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            crate::painting::scroll_snap::scroll_snapport_rect(rows, snap_container, scrollport.into()).into()
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_update_visual_viewport_transform(arena: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        crate::painting::owner_pass::run_paint_pass_of(
            arena,
            crate::painting::owner_pass::PaintPass::VisualViewportTransform,
            |arena, ()| update_visual_viewport_transform_stage(arena),
            (),
        )
    }
}

/// Updates the visual viewport transform of the arena's visual context tree, and answers whether it has a tree.
fn update_visual_viewport_transform_stage(arena: &mut LayoutNodeArena) -> bool {
    let mut paint_state = arena.paint_state().borrow_mut();
    let Some(tree) = &mut paint_state.visual_context.tree else {
        return false;
    };
    let inputs = arena.visual_context_tree_inputs();
    std::sync::Arc::make_mut(tree).set_visual_viewport_transform(
        crate::painting::visual_context::node_values::visual_viewport_transform_data(&inputs),
    );
    true
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_invalidate_scroll_state(arena: *mut c_void) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ScrollStateInvalidated,
        );
    };
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            use crate::painting::visual_context::SpatialData;
            let Some(tree) = rows.rows().paintable.visual_context_tree.as_deref() else {
                return u32::MAX;
            };
            rows.with_paintable_visual_context_node_handles(paintable, |handles| {
                handles
                    .spatial
                    .iter()
                    .find(|index| {
                        tree.spatial_nodes.get(index.0 as usize).is_some_and(|node| {
                            matches!(&node.data, SpatialData::Sticky(sticky) if sticky.owner_paintable == paintable)
                        })
                    })
                    .map_or(u32::MAX, |index| index.0)
            })
        })
    }
}

/// Re-reads the scroll containers' offsets when something invalidated them since the last
/// refresh, resolves the sticky nodes' offsets on top of them, and hands the dense device-pixel
/// snapshot to `publish`. Otherwise the caller keeps its copy.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `publish` is called synchronously with `sink` and a view of the snapshot that is valid only
/// for the duration of that call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_refresh_scroll_state(
    arena: *mut c_void,
    sink: *mut c_void,
    publish: unsafe extern "C" fn(*mut c_void, *const libgfx_rust::FloatPoint, usize),
) {
    // SAFETY: Guaranteed by the caller.
    let refresh = unsafe {
        crate::painting::owner_pass::run_paint_pass_of(
            arena,
            crate::painting::owner_pass::PaintPass::ScrollState,
            refresh_scroll_state_stage,
            (),
        )
    };
    let Some(snapshot) = refresh else {
        return;
    };
    // SAFETY: The C++ sink copies the offsets synchronously.
    unsafe { publish(sink, snapshot.as_ptr(), snapshot.len()) };
}

/// Refreshes the arena's scroll state where something invalidated it, and answers with its dense device-pixel
/// snapshot, the sticky nodes' offsets resolved.
fn refresh_scroll_state_stage(arena: &mut LayoutNodeArena, (): ()) -> Option<Vec<libgfx_rust::FloatPoint>> {
    let paintable_rows = arena.paintable_rows();
    let mut paint_state = arena.paint_state().borrow_mut();
    let state = &mut paint_state.visual_context;
    if !state.needs_to_refresh_scroll_state {
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
        absolute_rects,
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
    crate::stage_thread::hold_here(crate::stage_thread::FfiStageHoldPoint::MidRecording);
    let recording = crate::painting::record::traversal::record_display_list(
        &frame,
        absolute_rects,
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
                absolute_rects,
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
/// recording runs in the submitted frame and this returns before it has, leaving the ticket it answers
/// on in `submitted_ticket`, retained for the caller, which learns from it whether the recording is still
/// in flight; otherwise it runs now, on the stage thread, while the caller waits for it, and
/// `submitted_ticket` is left null. The document thread never records itself.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
/// Input arrays and byte buffers must remain valid and immutable throughout this call;
/// fonts for enabled overlays must be live `Gfx::Font`s. A submitted recording owns the arena
/// until the host takes the frame back, and the host keeps the arena alive until then.
/// `submitted_ticket` must point to writable storage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_record_display_list(
    arena_handle: *mut c_void,
    viewport: NodeSlotId,
    inputs: crate::painting::host::FfiRecordingInputs,
    run: FfiRecordingRun,
    submitted_ticket: *mut *const c_void,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe { *submitted_ticket = std::ptr::null() };
    // A recording of the document the frame in flight made (one a main-thread record, such as a display list dump,
    // reaches beside the frame) is the frame's to publish, which the frame's presentation may not, and which a read of
    // the arena may have taken in unpublished already: taking the frame in runs its consume, which publishes it.
    crate::stage_thread::join_recording_in_flight_of(arena_handle);
    // The recording is made for the render state as it stands now. It is not published if the
    // document retires that render state before the host takes the recording in.
    // SAFETY: Guaranteed by the caller.
    let frame_generation = unsafe { crate::layout::frame_retirement::frame_generation(arena_handle) };
    let arguments = (viewport, inputs, frame_generation);
    if run == FfiRecordingRun::InSubmittedFrame && crate::stage_thread::submits() {
        // SAFETY: Guaranteed by the caller; this thread waits for the pass, which reads the inputs it lends.
        let prepared = unsafe {
            crate::painting::owner_pass::run_held_pass(
                arena_handle,
                arguments,
                |arena, (viewport, inputs, frame_generation)| {
                    // SAFETY: The document thread waits for the pass, keeping the inputs live.
                    let inputs = prepare_recording(arena, viewport, &inputs)?;
                    let should_paint_overlay = inputs.should_paint_overlay;
                    let input = recording_stage_input(arena, viewport, inputs.into_owned());
                    let (job, ticket) = RecordingJob::new(input, should_paint_overlay, frame_generation);
                    arena.recording().await_recording(ticket.clone());
                    Some((job, ticket))
                },
            )
        };
        let Some((job, ticket)) = prepared else {
            return false;
        };
        // SAFETY: Guaranteed by the caller.
        unsafe { *submitted_ticket = std::sync::Arc::into_raw(ticket).cast() };
        crate::stage_thread::submit_recording(arena_handle, move || job.run());
        return true;
    }
    // SAFETY: Guaranteed by the caller; this thread waits for the pass, which reads the inputs it lends.
    unsafe {
        crate::painting::owner_pass::run_held_pass(
            arena_handle,
            arguments,
            |arena, (viewport, inputs, frame_generation)| {
                // SAFETY: The document thread waits for the pass, keeping the inputs live.
                let Some(inputs) = prepare_recording(arena, viewport, &inputs) else {
                    return false;
                };
                let publishes_recording = inputs.publishes_recording;
                let should_paint_overlay = inputs.should_paint_overlay;
                let output = record_display_list_stage(recording_stage_input(arena, viewport, inputs));
                leave_pending_recording(
                    arena,
                    viewport,
                    should_paint_overlay,
                    publishes_recording,
                    frame_generation,
                    output,
                );
                true
            },
        )
    }
}

/// Readies the arena to record the display list below `viewport` with the host's `inputs`, and answers with what the
/// recording reads, or `None` where there is nothing to record: the viewport has no committed box or stacking contexts.
///
/// # Safety
///
/// The input arrays and byte buffers `inputs` names must stay live and unwritten for as long as the answer is.
unsafe fn prepare_recording<'a>(
    arena: &LayoutNodeArena,
    viewport: NodeSlotId,
    inputs: &'a crate::painting::host::FfiRecordingInputs,
) -> Option<crate::painting::record::RecordingInputs<'a>> {
    {
        let mut recording = arena.recording();
        // A recording left unpublished is a frame dropped, which neither the frame scheduler nor the join before the
        // pass leaves; should one be, it is dropped unpublished.
        debug_assert!(
            recording.pending_recording().is_none(),
            "a frame was dropped: its recording was not published before the next one started"
        );
        recording.discard_pending_recording();
    }
    let recording_inputs = {
        let paint_state = arena.paint_state().borrow();
        if !arena.paintable_row_is_populated(viewport) || arena.stacking_context_entries(viewport).is_none() {
            return None;
        }
        let visual_context = &paint_state.visual_context;
        // SAFETY: Guaranteed by the caller. Only owned output and retained resources escape into the pending
        // recording.
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
    // A clock lease's ticks record again with what this recording records with.
    if recording_inputs.publishes_recording {
        arena.paint_state().borrow_mut().clock_recording = Some(crate::painting::paint_state::ClockRecording {
            viewport,
            inputs: recording_inputs.clone().into_owned(),
        });
    }
    Some(recording_inputs)
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
pub(crate) unsafe fn record_for_clock_tick(state: *mut crate::layout::ArenaHandle) -> bool {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { &mut *state }.arena_mut();
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
    let frame_generation = unsafe { crate::layout::frame_retirement::frame_generation(state.cast()) };
    let output = record_display_list_stage(recording_stage_input(arena, viewport, inputs));
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
pub(crate) unsafe fn settle_visual_contexts_for_clock_tick(state: *mut crate::layout::ArenaHandle) -> bool {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { &mut *state }.arena_mut();
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
    let outcome = update_accumulated_visual_contexts_stage(arena, viewport);
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
    // SAFETY: Guaranteed by the caller: the tick that calls this runs on the owner, and holds the arena.
    let arena = unsafe { arena_of_owner_unit(arena_handle) };
    let snapshot = {
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
pub(crate) fn discard_sealed_flight_paint() {
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
/// `state` must be the live render state of the document the frame in flight is of, which the
/// frame owns, with no borrow of it held.
pub(crate) unsafe fn paint_in_flight(
    state: *mut crate::layout::ArenaHandle,
    seal: FlightPaintSeal,
) -> Result<FlightPaintProducts, FlightPaintStop> {
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { &mut *state }.arena_mut();
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
        debug_assert!(
            recording.pending_recording().is_none(),
            "a frame was dropped: its recording was not published before the next one started"
        );
        recording.discard_pending_recording();
    }
    let should_paint_overlay = inputs.should_paint_overlay;
    let publishes_recording = inputs.publishes_recording;
    // SAFETY: The frame in flight owns the arena, and no borrow of it is held here.
    let output = record_display_list_stage(recording_stage_input(
        unsafe { &mut *state }.arena_mut(),
        viewport,
        inputs,
    ));
    // SAFETY: The stage has returned its borrow.
    let arena = unsafe { &*state }.arena();
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
/// `state` must be the live render state the frame in flight owns, whose recording the flight left
/// pending, with no borrow of it held.
pub(crate) unsafe fn present_in_flight(state: *mut crate::layout::ArenaHandle, products: &FlightPaintProducts) {
    let presentation = products
        .presentation
        .expect("a flight presents through the presentation sealed with its paint");
    // SAFETY: Guaranteed by the caller.
    let arena = unsafe { &*state }.arena();
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

/// Publishes the arena's pending recording from a flight's present stage, as `layout_arena_publish_recording` does on
/// the main thread. Returns
/// the generation of the hit-test list the recording made, or 0 if there was nothing to publish.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, owned by the flight whose present stage
/// calls this; the callbacks in `publish` are called synchronously with their context, which the
/// host lent that stage.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_recording_in_frame(
    arena: *mut c_void,
    publish: crate::painting::host::FfiRecordingPublishCallbacks,
    out: *mut FfiPresentedRecording,
) {
    // SAFETY: Guaranteed by the caller: a flight's present stage runs on the owner, and holds the arena.
    let arena = unsafe { arena_of_owner_unit(arena) };
    let pending = arena.recording().pending_recording().take();
    if let Some(pending) = pending {
        let publish = crate::painting::host::RecordingPublishHost::from(publish);
        // SAFETY: Guaranteed by the caller: this is a flight's present stage.
        let presentation = unsafe { crate::painting::host::FramePresentation::new() };
        crate::painting::record::publish::publish_recording(arena, pending, &presentation, &publish);
    }
    let presented = FfiPresentedRecording::of_last_recording(arena);
    arena.publish_rows();
    // SAFETY: Guaranteed by the caller.
    unsafe { out.write(presented) };
}

/// Whether the recording whose ticket `layout_arena_record_display_list` handed the document thread is still in
/// flight: the document has not taken it in yet. The ticket is what the recording answers on, so this reaches no arena.
///
/// # Safety
///
/// `ticket` must be a live ticket from `layout_arena_record_display_list`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_recording_ticket_is_in_flight(ticket: *const c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    !unsafe { &*ticket.cast::<crate::painting::recording_slot::RecordingTicket>() }.is_taken_in()
}

/// Retains the ticket `ticket` for the frame's presentation, which publishes the recording's answer from it: the
/// document takes the answer in once the presentation has published it.
///
/// # Safety
///
/// `ticket` must be a live ticket from `layout_arena_record_display_list`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_recording_ticket_retain_for_presentation(ticket: *const c_void) -> *const c_void {
    // SAFETY: Guaranteed by the caller.
    let retained = unsafe { layout_recording_ticket_retain(ticket) };
    // SAFETY: As above.
    unsafe { &*retained.cast::<crate::painting::recording_slot::RecordingTicket>() }.will_be_presented();
    retained
}

/// Retains `ticket` once more, for another holder to release.
///
/// # Safety
///
/// `ticket` must be a live ticket from `layout_arena_record_display_list`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_recording_ticket_retain(ticket: *const c_void) -> *const c_void {
    let ticket = ticket.cast::<crate::painting::recording_slot::RecordingTicket>();
    // SAFETY: Guaranteed by the caller.
    unsafe { std::sync::Arc::increment_strong_count(ticket) };
    ticket.cast()
}

/// # Safety
///
/// `ticket` must be null or a retained ticket from `layout_arena_record_display_list`, released
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
/// `ticket` must be a live ticket from `layout_recording_ticket_retain_for_presentation`.
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

impl Default for FfiPresentedRecording {
    fn default() -> Self {
        Self {
            is_identical_to_published_frame: false,
            has_blocking_wheel_event_listeners: false,
            display_list: std::ptr::null(),
        }
    }
}

impl FfiPresentedRecording {
    /// What the host reads of the last recording the document took in, which a publication in its arena presents.
    fn of_last_recording(arena: &LayoutNodeArena) -> Self {
        let paint_state = arena.paint_state().borrow();
        let recording = paint_state.last_recording.as_deref();
        Self {
            is_identical_to_published_frame: recording
                .is_some_and(|recording| recording.is_identical_to_published_frame),
            has_blocking_wheel_event_listeners: recording
                .is_some_and(|recording| recording.has_blocking_wheel_event_listeners),
            display_list: recording.map_or(std::ptr::null(), |recording| {
                std::sync::Arc::into_raw(recording.display_list.clone()).cast()
            }),
        }
    }
}

/// Publishes the answer of the recording whose ticket the frame's presentation holds: hands its
/// resources to the host and leaves its output for the document to take in, without reaching the
/// document. Returns false if the recording has nothing to publish.
///
/// # Safety
///
/// `ticket` must be a live ticket from `layout_recording_ticket_retain_for_presentation`, and this
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
    // SAFETY: Guaranteed by the caller.
    unsafe { crate::painting::owner_pass::run_held_pass(arena, (), |arena, ()| drop(arena.recording())) };
}

/// Runs `handoff(context)`, which hands a navigable's finished frame to its compositor frame sink,
/// as a render stage, on the stage thread.
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
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ReplacedPaintFacts {
                slot,
                facts: crate::painting::replaced_paint_facts::ReplacedPaintFacts::FormControl(facts),
                damage_when_changed: crate::painting::record::damage::PaintDamage::NONE,
            },
        );
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_canvas_paint_facts(
    arena: *mut c_void,
    slot: NodeSlotId,
    facts: crate::painting::host::FfiCanvasPaintFacts,
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ReplacedPaintFacts {
                slot,
                facts: crate::painting::replaced_paint_facts::ReplacedPaintFacts::Canvas(facts),
                damage_when_changed: crate::painting::record::damage::PaintDamage::ALL_DRAW
                    | crate::painting::record::damage::PaintDamage::ALL_HIT,
            },
        );
    }
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
) {
    // SAFETY: Guaranteed by the caller.
    let entries = unsafe { ffi_slice(entries, count) }
        .iter()
        .map(
            |entry| crate::painting::layer_image_paint_facts::LayerImagePaintFactsEntry {
                list: entry.list,
                computed_index: entry.computed_index,
                facts: crate::painting::layer_image_paint_facts::LayerImagePaintFacts::from_ffi(&entry.facts),
            },
        )
        .collect();
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::LayerImagePaintFacts { slot, entries },
        );
    }
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
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ReplacedPaintFacts {
                slot,
                facts: crate::painting::replaced_paint_facts::ReplacedPaintFacts::Image(
                    crate::painting::replaced_paint_facts::ImagePaintFacts::from_ffi(&facts),
                ),
                damage_when_changed: crate::painting::record::damage::PaintDamage::NONE,
            },
        );
    }
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
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ReplacedPaintFacts {
                slot,
                facts: crate::painting::replaced_paint_facts::ReplacedPaintFacts::Video(
                    crate::painting::replaced_paint_facts::VideoPaintFacts::from_ffi(&facts),
                ),
                damage_when_changed: crate::painting::record::damage::PaintDamage::NONE,
            },
        );
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_navigable_container_paint_facts(
    arena: *mut c_void,
    slot: NodeSlotId,
    facts: crate::painting::host::FfiNavigableContainerPaintFacts,
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ReplacedPaintFacts {
                slot,
                facts: crate::painting::replaced_paint_facts::ReplacedPaintFacts::NavigableContainer(facts),
                damage_when_changed: crate::painting::record::damage::PaintDamage::ALL_DRAW
                    | crate::painting::record::damage::PaintDamage::ALL_HIT,
            },
        );
    }
}

/// Whether the row's DOM node published itself as editable or as an editing host. A row that no
/// DOM node ever bound answers no, the way a node that is neither does.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_is_editable_or_editing_host(arena: *mut c_void, slot: NodeSlotId) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            rows.slot_is_live(slot)
                && rows.node_has_dom_paint_fact(slot, crate::layout::node_data::DomPaintFact::EditableOrEditingHost)
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_current(arena, |rows| {
            rows.replaced_paint_facts(slot)
                .and_then(|facts| facts.navigable_container())
                .map(|facts| facts.local_content_navigable)
                .unwrap_or_default()
        })
    }
}

/// Tells the render owner that the element with `element_style_node` published a `::selection` style, which the rows
/// that paint text under it paint selected text with. Nothing waits for it.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_sync_selection_pseudo_style(arena: *mut c_void, element_style_node: u32) {
    let Some(element) = crate::css::style::tree::StyleNodeID::from_raw(element_style_node) else {
        return;
    };
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { crate::layout::ArenaHandle::document_of(arena) };
    if document.is_valid() {
        crate::render_owner::send_arena_change(
            document,
            crate::render_owner::ArenaChange::SelectionPseudoStylePublished(element),
        );
    }
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
pub unsafe extern "C" fn layout_arena_paintable_invalidate_paint_cache(
    arena: *mut c_void,
    paintable: NodeSlotId,
    propagated_text_decorations: bool,
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::PaintCacheInvalidated {
                slot: paintable,
                propagated_text_decorations,
            },
        );
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_row_reset_version(arena: *mut c_void, paintable: NodeSlotId) -> u64 {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::current(arena).paintable.row_reset_version(paintable) }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_invalidate_for_repaint(
    arena: *mut c_void,
    paintable: NodeSlotId,
    include_hit_test_items: bool,
) {
    use crate::painting::record::damage::PaintDamage;
    let damage = if include_hit_test_items {
        PaintDamage::ALL_PRODUCERS
    } else {
        PaintDamage::ALL_DRAW
    };
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::Repaint {
                slot: paintable,
                damage,
            },
        );
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_invalidate_subtree_for_repaint(
    arena: *mut c_void,
    paintable: NodeSlotId,
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::SubtreeRepaint(paintable),
        );
    };
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_invalidate_all_paint_caches(arena: *mut c_void) {
    // SAFETY: Guaranteed by the caller.
    unsafe { send(arena, crate::painting::paint_changes::PaintChange::FullRepaint) };
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_paintable_computed_svg_path(
    arena: *mut c_void,
    paintable: NodeSlotId,
) -> *const c_void {
    // SAFETY: Guaranteed by the caller.
    let path = unsafe {
        read_current(arena, |rows| {
            if !rows.paintable_row_is_populated(paintable) {
                return None;
            }
            crate::painting::paintable_geometry::committed_svg_path(rows, paintable)
        })
    };
    // The box's fragment keeps the path live beside this reference.
    path.map_or(std::ptr::null(), |path| path.as_raw())
}

#[derive(Default)]
#[repr(C)]
pub struct FfiCaretRectResult {
    pub found: bool,
    pub rect: FfiCssPixelRect,
    /// The row whose style the caret is painted with.
    pub style_source: NodeSlotId,
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let fragments = paintable_rows.text_fragments(primary);
            match crate::painting::caret::caret_rect_in_dom_range(paintable_rows, fragments.as_slice(), offset) {
                Some(rect) => FfiOptionalCssPixelRect {
                    has_value: true,
                    rect: rect.into(),
                },
                None => FfiOptionalCssPixelRect::default(),
            }
        })
    }
}

#[derive(Default)]
#[repr(C)]
pub struct FfiEmptyLineCaretRect {
    pub has_value: bool,
    pub rect: FfiCssPixelRect,
    pub style_source: NodeSlotId,
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

/// Publishes the document's committed geometry as a query snapshot. The caller releases the snapshot with
/// `query_snapshot_release`.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread, and
/// `viewport.device_scroll_offsets` must address `viewport.device_scroll_offsets_len` points.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_query_snapshot(
    arena: *mut c_void,
    viewport: crate::painting::query_snapshot::FfiQuerySnapshotViewport,
) -> *const c_void {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { RowSnapshot::current_shared(arena) };
    crate::painting::query_snapshot::into_handle(crate::painting::query_snapshot::QuerySnapshot::new(rows, &viewport))
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
    // SAFETY: Guaranteed by the caller.
    let rects = unsafe {
        read_committed(arena, |paintable_rows| {
            // SAFETY: Guaranteed by the caller.
            let transform = rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform);
            let mut rects = Vec::new();
            crate::painting::client_rects::for_each_client_rect(
                paintable_rows,
                layout_node,
                transform.as_ref(),
                |rect| {
                    rects.push(FfiCssPixelRect::from(rect));
                },
            );
            rects
        })
    };
    for rect in rects {
        // SAFETY: The consumer copies the plain-data rect synchronously.
        unsafe { push_rect(context, rect) };
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            // SAFETY: Guaranteed by the caller.
            let transform = rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform);
            crate::painting::client_rects::bounding_client_rect(paintable_rows, layout_node, transform.as_ref()).into()
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            // SAFETY: Guaranteed by the caller.
            let transform = rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform);
            crate::painting::intersection_observer::transform_subtree_is_clipped_outside(
                paintable_rows,
                target,
                root_bounds.into(),
                transform.as_ref(),
            )
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            // SAFETY: Guaranteed by the caller.
            let transform = rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform);
            crate::painting::intersection_observer::intersection_rect(
                paintable_rows,
                target,
                target_rect.into(),
                intersection_root,
                root_bounds.into(),
                transform.as_ref(),
                |clip_rect: CssPixelRect| -> CssPixelRect {
                    // SAFETY: Guaranteed by the caller: the callback only computes the inflated rect from the scroll
                    // margin it is lent.
                    inflate_scroll_container_clip_rect_by_scroll_margin(context, clip_rect.into()).into()
                },
            )
            .into()
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            crate::painting::client_rects::can_compute_client_rects_without_visual_context_update(
                paintable_rows,
                layout_node,
                viewport_scroll_offset_is_zero,
            )
        })
    }
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_inline_paintable_has_content_pieces(
    arena: *mut c_void,
    inline_paintable: NodeSlotId,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let mut has_content = false;
            with_inline_pieces(paintable_rows, inline_paintable, |piece, _| {
                if !piece.is_geometry_only_placeholder {
                    has_content = true;
                    return false;
                }
                true
            });
            has_content
        })
    }
}

#[derive(Default)]
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let position = inline_first_piece_position(paintable_rows, inline_paintable);
            FfiOptionalCssPixelPoint {
                has_value: position.is_some(),
                x: position.map_or(CssPixels::from_raw(0), |position| position.x),
                y: position.map_or(CssPixels::from_raw(0), |position| position.y),
            }
        })
    }
}

/// The absolute position of the padding box of an inline box's first piece, or none for an inline
/// box with no pieces.
pub(crate) fn inline_first_piece_position(
    rows: &impl GeometryRead,
    inline_paintable: NodeSlotId,
) -> Option<CssPixelPoint> {
    let root = rows.inline_pieces_root(inline_paintable)?;
    let root_position = crate::painting::paintable_geometry::absolute_position(rows, root);
    let border_widths = crate::painting::paintable_geometry::committed_border(rows, inline_paintable);
    let padding_widths = crate::painting::paintable_geometry::committed_padding(rows, inline_paintable);
    let mut result = None;
    with_inline_pieces(rows, inline_paintable, |piece, _data| {
        let border_rect = CssPixelRect::from(piece.border_box_rect);
        let rect = if piece.is_geometry_only_placeholder {
            border_rect
        } else {
            let padding_rect = piece.shrunken_by_present_edges(border_rect, border_widths);
            piece.shrunken_by_present_edges(padding_rect, padding_widths)
        };
        result = Some(CssPixelPoint {
            x: rect.x + root_position.x,
            y: rect.y + root_position.y,
        });
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
    // SAFETY: Guaranteed by the caller.
    let lines = unsafe {
        read_committed(arena, |paintable_rows| {
            let fragments = paintable_rows.text_fragments(primary);
            crate::painting::visual_lines::collect_visual_lines(paintable_rows, fragments.as_slice())
                .into_iter()
                .map(|line| FfiVisualLine {
                    start_offset: line.start_offset,
                    end_offset: line.end_offset,
                    end_offset_with_trailing_whitespace: line.end_offset_with_trailing_whitespace,
                    has_fragments: line.has_fragments,
                    owner_paintable: line.owner.index,
                    line_index: line.line_index,
                })
                .collect::<Vec<_>>()
        })
    };
    for line in lines {
        // SAFETY: The consumer copies the POD line synchronously.
        unsafe { push(context, line) };
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let fragments = paintable_rows.text_fragments(primary);
            has_rendered_text_matching(paintable_rows, fragments.as_slice(), |fragment| {
                fragment.dom_start_offset_in_node < offset
            })
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let fragments = paintable_rows.text_fragments(primary);
            has_rendered_text_matching(paintable_rows, fragments.as_slice(), |fragment| {
                fragment.dom_end_offset_in_node > offset
            })
        })
    }
}

#[derive(Default)]
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            let fragments = paintable_rows.text_fragments(primary);
            let coordinate = crate::painting::visual_lines::caret_inline_coordinate(
                paintable_rows,
                owner_paintable,
                line_index,
                fragments.as_slice(),
                offset,
            );
            FfiOptionalCssPixels {
                has_value: coordinate.is_some(),
                value: coordinate.unwrap_or(CssPixels::from_raw(0)),
            }
        })
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
    // SAFETY: Guaranteed by the caller.
    let offset = unsafe {
        read_committed(arena, |paintable_rows| {
            let fragments = paintable_rows.text_fragments(primary);
            crate::painting::visual_lines::offset_closest_to_inline_coordinate(
                paintable_rows,
                owner_paintable,
                line_index,
                fragments.as_slice(),
                inline_coordinate,
            )
        })
    };
    offset.unwrap_or(fallback_offset)
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
    // SAFETY: Guaranteed by the caller.
    let rects = unsafe {
        read_committed(arena, |paintable_rows| {
            // SAFETY: Guaranteed by the caller.
            let transform = rect_to_viewport_transform_from_ffi(&rect_to_viewport_transform);
            let fragments = paintable_rows.text_fragments(primary);
            let mut rects = Vec::new();
            crate::painting::text_fragment::for_each_fragment_of_nodes(
                paintable_rows,
                fragments.as_slice(),
                |block, _, fragment| {
                    if fragment.dom_end_offset_in_node <= filter_dom_start
                        || fragment.dom_start_offset_in_node >= filter_dom_end
                    {
                        return true;
                    }
                    let rect = crate::painting::text_fragment::range_rect(
                        paintable_rows,
                        fragment,
                        selection_state,
                        range_start_offset,
                        range_end_offset,
                    );
                    let rect_in_viewport_space = if paintable_rows.slot_is_live(block) {
                        crate::painting::rect_to_viewport_transform::transform_rect_to_viewport_or_identity(
                            transform.as_ref(),
                            paintable_rows,
                            block,
                            rect,
                        )
                    } else {
                        rect
                    };
                    rects.push(FfiCssPixelRect::from(rect_in_viewport_space));
                    true
                },
            );
            rects
        })
    };
    for rect in rects {
        // SAFETY: The consumer copies the plain-data rect synchronously.
        unsafe { push_rect(context, rect) };
    }
}

#[derive(Default)]
#[repr(C)]
pub struct FfiOptionalCssPixelRect {
    pub has_value: bool,
    pub rect: FfiCssPixelRect,
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
    // SAFETY: Guaranteed by the caller.
    let data = unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(paintable) {
                return None;
            }
            crate::painting::paintable_geometry::committed_grid_layout_data(paintable_rows, paintable)
        })
    };
    if let Some(data) = data {
        let json = crate::painting::devtools_layout::serialize_grid_layout(&data, container_node_id);
        // SAFETY: The consumer copies the byte span synchronously.
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
    // SAFETY: Guaranteed by the caller.
    let data = unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(paintable) {
                return None;
            }
            crate::painting::paintable_geometry::committed_flex_layout_data(paintable_rows, paintable)
        })
    };
    if let Some(data) = data {
        let json = crate::painting::devtools_layout::serialize_flex_layout(&data, container_node_id, |style_node| {
            // SAFETY: The host answers synchronously from the document the paintable belongs to.
            let node_id = unsafe { resolve_node_id(document_context, style_node) };
            (node_id >= 0).then_some(node_id)
        });
        // SAFETY: The consumer copies the byte span synchronously.
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
    // SAFETY: Guaranteed by the caller.
    let tracks = unsafe {
        read_committed(arena, |paintable_rows| {
            if !paintable_rows.paintable_row_is_populated(paintable) {
                return None;
            }
            crate::painting::paintable_geometry::committed_used_grid_tracks(paintable_rows, paintable)
        })
    };
    let Some(tracks) = tracks else {
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
    // SAFETY: Guaranteed by the caller.
    let tree = unsafe { RowSnapshot::current(arena) }
        .paintable
        .visual_context_tree
        .clone();
    tree.map_or(std::ptr::null(), |tree| std::sync::Arc::into_raw(tree).cast())
}

/// Like `layout_arena_main_visual_context_tree_retain`, for a clock tick's presentation, which holds the arena the tick
/// moved the visual contexts of.
///
/// # Safety
///
/// `arena` must be the live arena of the clock tick the owner runs on the calling thread. The caller owns one reference
/// to the returned tree and releases it with `visual_context_tree_release`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_clock_tick_visual_context_tree_retain(arena: *mut c_void) -> *const c_void {
    // SAFETY: Guaranteed by the caller.
    let tree = unsafe { arena_of_owner_unit(arena) }
        .paint_state()
        .borrow()
        .visual_context
        .tree
        .clone();
    tree.map_or(std::ptr::null(), |tree| std::sync::Arc::into_raw(tree).cast())
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_has_visual_context_tree(arena: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::current(arena) }
        .paintable
        .visual_context_tree
        .is_some()
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ScrollOffset {
                slot,
                offset: offset.into(),
                dom_target_stores_offset,
            },
        );
    }
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
    // SAFETY: Guaranteed by the caller.
    let (areas, coords) = unsafe { (ffi_slice(areas, area_count), ffi_slice(coords, coords_count)) };
    let areas = areas
        .iter()
        .map(|area| {
            let start = area.coords_offset as usize;
            let end = start + area.coords_count as usize;
            PublishedImageMapArea {
                style_node: area.style_node,
                shape: AreaShape::from_raw(area.shape),
                editable: area.editable != 0,
                coords: coords[start..end].into(),
            }
        })
        .collect();
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::ImageMapAreas { slot, areas },
        );
    };
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        read_committed(arena, |paintable_rows| {
            paintable_rows.with_image_map_areas(|areas| areas.area_editability(slot, style_node))
        })
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::VisualContextTreeInputs(inputs),
        );
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe { read_committed(arena, |paintable_rows| paintable_rows.scroll_offset(slot).into()) }
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
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::current(arena) }
        .paint_status
        .layout_commit_generation
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_context_tree_structural_epoch(arena: *mut c_void) -> u64 {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::current(arena) }
        .paintable
        .visual_context_tree
        .as_ref()
        .map_or(0, |tree| tree.structural_epoch)
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_visual_context_tree_has_visual_animations(arena: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    unsafe { RowSnapshot::current(arena) }
        .paintable
        .visual_context_tree
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
    use crate::painting::host::FfiVisualAnimationTargetKind;
    use crate::painting::visual_animation_builder::{Host, Request, effect_state_from_handle};
    let state = unsafe { effect_state_from_handle(state) };
    let request = unsafe { Request::new(&*request) };
    let host = Host::new(unsafe { &*host });
    const KINDS: [FfiVisualAnimationTargetKind; 4] = [
        FfiVisualAnimationTargetKind::Opacity,
        FfiVisualAnimationTargetKind::BackgroundColor,
        FfiVisualAnimationTargetKind::Filter,
        FfiVisualAnimationTargetKind::Transform,
    ];
    // The builder asks the host as it goes, which may let the owner publish again, so the target nodes of every kind
    // are read first.
    // SAFETY: Guaranteed by the caller.
    let targets = unsafe {
        read_current(arena, |rows| {
            let tree = rows.rows().paintable.visual_context_tree.as_deref();
            KINDS.map(|kind| visual_animation_target_indices(rows, request.layout_node(), tree, kind))
        })
    };
    state.build(&request, &host, |kind| {
        targets[KINDS
            .iter()
            .position(|candidate| *candidate == kind)
            .unwrap_or_default()]
        .clone()
    })
}

/// The nodes of the box that an animation of the kind drives: the effect nodes of an opacity, background color or
/// filter animation, or the spatial nodes of a transform animation, as far as the tree holds nodes of the right kind
/// for them.
fn visual_animation_target_indices(
    rows: &impl PaintRead,
    id: NodeSlotId,
    tree: Option<&crate::painting::visual_context::VisualContextTree>,
    target_kind: crate::painting::host::FfiVisualAnimationTargetKind,
) -> Vec<u32> {
    use crate::painting::host::FfiVisualAnimationTargetKind;
    let Some(tree) = tree else {
        return Vec::new();
    };
    rows.with_paintable_visual_context_node_handles(id, |handles| {
        let indices: &mut dyn Iterator<Item = u32> = match target_kind {
            FfiVisualAnimationTargetKind::Opacity
            | FfiVisualAnimationTargetKind::BackgroundColor
            | FfiVisualAnimationTargetKind::Filter => &mut handles.effects.iter().map(|index| index.0),
            FfiVisualAnimationTargetKind::Transform => &mut handles.spatial.iter().map(|index| index.0),
        };
        indices
            .filter(|&index| tree.visual_animation_target_is_valid(target_kind, index))
            .collect()
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::CompositorAnimations(animations),
        );
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::CompositorAnimationUpdateBegun,
        );
    }
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
    // SAFETY: Guaranteed by the caller.
    unsafe {
        crate::painting::owner_pass::run_held_pass(arena, publish_pending, |arena, publish_pending| {
            let mut paint_state = arena.paint_state().borrow_mut();
            crate::painting::visual_context::publish_compositor_animations(
                &mut paint_state.visual_context,
                publish_pending,
            )
        })
    }
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
    // Only enrolled resources are synchronized, which a visual context update then reads.
    // SAFETY: Guaranteed by the caller.
    let has_enrolled = !unsafe { RowSnapshot::current(arena) }
        .paint_facts
        .svg_paint_resources
        .is_empty();
    if has_enrolled {
        // SAFETY: As above.
        unsafe {
            send(
                arena,
                crate::painting::paint_changes::PaintChange::SvgPaintResourcesChanged,
            );
        };
    }
    has_enrolled
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_has_enrolled_svg_paint_resources(arena: *mut c_void) -> bool {
    // SAFETY: Guaranteed by the caller.
    !unsafe { RowSnapshot::current(arena) }
        .paint_facts
        .svg_paint_resources
        .is_empty()
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

/// The generation of the document's hit-test list as the rows published it, or zero for none.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hit_test_list_generation(arena: *mut c_void) -> u64 {
    // SAFETY: Guaranteed by the caller.
    let rows = unsafe { crate::layout::row_reads::RowSnapshot::published(arena) };
    rows.paintable.hit_test_list.as_ref().map_or(0, |list| list.generation)
}

pub(crate) fn ffi_topmost(
    item: Option<crate::painting::hit_test::query::TopmostItem>,
) -> crate::painting::host::FfiTopmostItem {
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
/// `arena` is live and used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_recording_trace_enabled(arena: *mut c_void, enabled: bool) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        send(
            arena,
            crate::painting::paint_changes::PaintChange::RecordingTraceEnabled(enabled),
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::css_pixels::CssPixelRect;
    use crate::layout::LayoutNodeArena;
    use crate::layout::node_data::NodeKind;
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
    fn committed_rows_measure_viewport_overflow_before_reading_it() {
        let mut arena = LayoutNodeArena::new();
        let viewport = arena.allocate_for_test().slot;
        arena.write_shape(viewport).set_kind(NodeKind::Viewport);
        arena.populate_paintable_row(viewport);
        arena.scrollable_overflow.viewport.set(Some(viewport));
        let root = arena.allocate_for_test().slot;
        arena.populate_paintable_row(root);
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
        // Leave stale overflow for the committed rows to measure before they are read. Losing
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

        let rect = arena
            .with_committed_rows(|rows| crate::painting::paintable_geometry::scrollable_overflow_rect(rows, viewport));
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
