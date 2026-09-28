/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The caret queries the main thread makes of a [`HitTestSnapshot`]: where a point or a DOM position
//! puts the caret, over the caret lines the snapshot's list was published with.
//!
//! They read the snapshot alone. A caret line search confined to a DOM node is told which lines are
//! in scope by the host, which decides it from the DOM nodes the snapshot names for each line
//! ([`hit_test_snapshot_visit_caret_line_nodes`]): a snapshot knows the rows, the host knows the DOM.

use super::HitTestList;
use super::caret::{CaretLineDirection, CaretPositionMode, CaretPositionType};
use super::snapshot::{FfiHitNodeIdentity, HitTestSnapshot, dispatch_identity, snapshot_from_handle};
use crate::css::css_pixels::CssPixels;
use crate::layout::FfiCssPixelPoint;
use crate::painting::host::{
    FfiAdjacentLine, FfiCaretItemForLine, FfiCaretLineExport, FfiCaretLineForPosition, FfiCaretPositionQuery,
    FfiClosestLine, FfiHitTestQueryCallbacks, FfiResolvedCaret, FfiTopmostItemsForCaret,
};
use crate::painting::published_frame::PaintSource;
use std::ffi::c_void;

impl HitTestSnapshot<'_> {
    /// Runs a query of the list's caret lines, or answers `default` where the snapshot holds no list
    /// with its caret lines built.
    fn read_caret_lines<R>(&self, default: R, read: impl FnOnce(&HitTestList, &PaintSource<'_>) -> R) -> R {
        if !self.list().is_some_and(|list| list.caret_lines_built) {
            return default;
        }
        self.read(default, read)
    }

    /// Whether `line_index` names one of the list's caret lines. The host names lines a query of the
    /// same snapshot answered with, so a line out of range is a bug the query answers nothing for.
    fn has_caret_line(list: &HitTestList, line_index: usize) -> bool {
        let has_line = line_index < list.caret_lines.len();
        debug_assert!(has_line, "a caret line the snapshot does not hold");
        has_line
    }

    fn has_item(list: &HitTestList, item_index: usize) -> bool {
        let has_item = item_index < list.items.len();
        debug_assert!(has_item, "an item the snapshot does not hold");
        has_item
    }
}

/// The topmost item that can produce a caret position at `point`, and the topmost item there.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_find_topmost_items_for_caret(
    snapshot: *const c_void,
    callbacks: FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
) -> FfiTopmostItemsForCaret {
    unsafe { snapshot_from_handle(snapshot) }.query(Default::default(), |list, tree, rows| {
        let (caret_item, hit_item) = list.find_topmost_items_for_caret(rows, tree, &callbacks, point.into());
        FfiTopmostItemsForCaret {
            caret_item: crate::painting::ffi::ffi_topmost(caret_item),
            hit_item: crate::painting::ffi::ffi_topmost(hit_item),
        }
    })
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_caret_line(
    snapshot: *const c_void,
    line_index: usize,
) -> FfiCaretLineExport {
    unsafe { snapshot_from_handle(snapshot) }.read_caret_lines(Default::default(), |list, _| {
        if !HitTestSnapshot::has_caret_line(list, line_index) {
            return Default::default();
        }
        let line = &list.caret_lines[line_index];
        FfiCaretLineExport {
            rect: line.rect.into(),
            context: line.context,
            first_caret_item_index: line.first_caret_item_index,
            last_caret_item_index: line.last_caret_item_index,
        }
    })
}

/// The caret line the DOM position `query` names lies on.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, and the query's
/// boundary descent must be readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_caret_line_for_position(
    snapshot: *const c_void,
    query: FfiCaretPositionQuery,
    offset: usize,
    affinity_is_downstream: bool,
) -> FfiCaretLineForPosition {
    unsafe { snapshot_from_handle(snapshot) }.read_caret_lines(Default::default(), |list, rows| {
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
        match list.caret_line_for_position(rows, &query, offset, affinity_is_downstream) {
            Some(line_index) => FfiCaretLineForPosition {
                has_line: true,
                line_index,
            },
            None => Default::default(),
        }
    })
}

/// The first or last item of a caret line, as `position_type` asks; `usize::MAX` for none.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_item_at_line_edge(
    snapshot: *const c_void,
    line_index: usize,
    position_type: u8,
) -> usize {
    let position_type = CaretPositionType::from_u8(position_type);
    unsafe { snapshot_from_handle(snapshot) }.read_caret_lines(usize::MAX, |list, _| {
        if !HitTestSnapshot::has_caret_line(list, line_index) {
            return usize::MAX;
        }
        list.item_at_line_edge(line_index, position_type)
    })
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_caret_item_for_line(
    snapshot: *const c_void,
    line_index: usize,
    point: FfiCssPixelPoint,
    mode: u8,
) -> FfiCaretItemForLine {
    unsafe { snapshot_from_handle(snapshot) }.read_caret_lines(Default::default(), |list, rows| {
        if !HitTestSnapshot::has_caret_line(list, line_index) {
            return Default::default();
        }
        match list.caret_item_for_line(rows, line_index, point.into(), CaretPositionMode::from_u8(mode)) {
            Some((item_index, position_type)) => FfiCaretItemForLine {
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
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_line_block_coordinate(snapshot: *const c_void, line_index: usize) -> i32 {
    unsafe { snapshot_from_handle(snapshot) }.read_caret_lines(0, |list, _| {
        if !HitTestSnapshot::has_caret_line(list, line_index) {
            return 0;
        }
        list.line_block_coordinate(line_index).raw_value()
    })
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_item_is_inline_adjacent_to_line(
    snapshot: *const c_void,
    item_index: usize,
    line_index: usize,
) -> bool {
    unsafe { snapshot_from_handle(snapshot) }.read_caret_lines(false, |list, _| {
        HitTestSnapshot::has_item(list, item_index)
            && HitTestSnapshot::has_caret_line(list, line_index)
            && list.item_is_inline_adjacent_to_line(item_index, line_index)
    })
}

/// The caret line closest to `point`; for a `scoped` search, among the lines `callbacks` says are in
/// scope.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, and the scroll
/// offsets and the scope mask `callbacks` points at must be readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_find_closest_line(
    snapshot: *const c_void,
    callbacks: FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
    mode: u8,
    scoped: bool,
    respect_clip: bool,
) -> FfiClosestLine {
    unsafe { snapshot_from_handle(snapshot) }.query(Default::default(), |list, tree, rows| {
        if !list.caret_lines_built {
            return Default::default();
        }
        let closest = list.find_closest_line(
            rows,
            tree,
            &callbacks,
            point.into(),
            CaretPositionMode::from_u8(mode),
            scoped,
            respect_clip,
        );
        FfiClosestLine {
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

/// The caret line in scope next to `current_line_index` in `direction` (1 for the next line, 0 for
/// the previous one), and the point on it closest to the inline coordinate.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, and the scope mask
/// `callbacks` points at must be readable for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_adjacent_line(
    snapshot: *const c_void,
    callbacks: FfiHitTestQueryCallbacks,
    current_line_index: usize,
    direction: u8,
    inline_coordinate_raw: i32,
) -> FfiAdjacentLine {
    let direction = if direction == 1 {
        CaretLineDirection::Next
    } else {
        CaretLineDirection::Previous
    };
    unsafe { snapshot_from_handle(snapshot) }.read_caret_lines(Default::default(), |list, _| {
        if !HitTestSnapshot::has_caret_line(list, current_line_index) {
            return Default::default();
        }
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
        match list.adjacent_line(
            &callbacks,
            current_line_index,
            direction,
            CssPixels::from_raw(inline_coordinate_raw),
        ) {
            Some((line_index, point)) => FfiAdjacentLine {
                has_line: true,
                line_index,
                point_x: point.x.raw_value(),
                point_y: point.y.raw_value(),
            },
            None => Default::default(),
        }
    })
}

/// Visits, for every caret line, the DOM nodes its caret items stand for, as style nodes, which the
/// host decides a scoped search's lines from.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`, and `visit` must
/// accept `sink` for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_visit_caret_line_nodes(
    snapshot: *const c_void,
    sink: *mut c_void,
    visit: unsafe extern "C" fn(*mut c_void, usize, u32),
) {
    unsafe { snapshot_from_handle(snapshot) }.read_caret_lines((), |list, rows| {
        for line_index in 0..list.caret_lines.len() {
            list.visit_caret_line_dom_style_nodes(rows, line_index, |style_node| {
                // SAFETY: The C++ host consumes the visit synchronously.
                unsafe { visit(sink, line_index, style_node) };
            });
        }
    });
}

/// The DOM node the item stands for as a caret target.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_item_target_node(
    snapshot: *const c_void,
    item_index: usize,
) -> FfiHitNodeIdentity {
    unsafe { snapshot_from_handle(snapshot) }.read(Default::default(), |list, rows| {
        if !HitTestSnapshot::has_item(list, item_index) {
            return Default::default();
        }
        dispatch_identity(rows, list.item_target_slot(rows, item_index), false)
    })
}

/// The DOM node an event at the item is dispatched to.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_item_dispatch_node(
    snapshot: *const c_void,
    item_index: usize,
) -> FfiHitNodeIdentity {
    unsafe { snapshot_from_handle(snapshot) }.read(Default::default(), |list, rows| {
        if !HitTestSnapshot::has_item(list, item_index) {
            return Default::default();
        }
        let (dispatch, allow_pseudo_fallback) = list.item_dispatch_slot(rows, item_index);
        dispatch_identity(rows, dispatch, allow_pseudo_fallback)
    })
}

/// The caret position a hit on the item at `local_point` resolves to.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_resolve_caret(
    snapshot: *const c_void,
    item_index: usize,
    local_point: FfiCssPixelPoint,
    position_type: u8,
) -> FfiResolvedCaret {
    unsafe { snapshot_from_handle(snapshot) }.read(Default::default(), |list, rows| {
        if !HitTestSnapshot::has_item(list, item_index) {
            return Default::default();
        }
        let resolved = list.resolve_caret(
            rows,
            item_index,
            local_point.into(),
            CaretPositionType::from_u8(position_type),
        );
        FfiResolvedCaret {
            has_position: resolved.has_position,
            node: dispatch_identity(rows, resolved.node, false),
            boundary: resolved.boundary,
            offset: resolved.offset,
            affinity_is_upstream: resolved.affinity_is_upstream,
            has_debug_rect: resolved.has_debug_rect,
            debug_rect: resolved.debug_rect.into(),
        }
    })
}
