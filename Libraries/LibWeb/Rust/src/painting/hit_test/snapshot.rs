/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the main thread hit tests: a document's hit-test list, with the rows it was recorded over.
//!
//! A [`HitTestSnapshot`] is a [`PublishedFrame`] of the document's committed rows, with the list of
//! the last recording the document took in and the structures a query derives from it built first:
//! the frame's rows hold the list and the visual context tree its items convert points through,
//! and every read a hit test makes of the rows, their layout nodes and their styles is one the frame
//! answers (see [`super::read`]). It is immutable and owns all of it, so the main thread hit tests
//! it while the arena goes on changing, and it holds no arena and names no layout node: a hit names
//! rows, which the caller finds what they stand for.
//!
//! A list outlives the rows it was recorded over: a scroll, a clip or a transform moves what a point
//! hits through the visual context tree and the committed rows, not through the list. So the
//! document publishes a snapshot for each hit test it makes, and what did not change since the
//! last one is shared with it.

use crate::css::css_pixels::CssPixelPoint;
use crate::layout::FfiCssPixelPoint;
use crate::layout::LayoutNodeArena;
use crate::layout::node_data::NodeSlotId;
use crate::painting::display_list::commands::ContextRef;
use crate::painting::hit_test::HitTestList;
use crate::painting::published_frame::{PaintSource, PublishedFrame};
use crate::painting::visual_context::VisualContextTree;
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::Arc;

pub(crate) struct HitTestSnapshot {
    frame: PublishedFrame,
}

// A snapshot is hit tested on the main thread while the arena is written wherever its owner runs:
// it holds no cell, no borrow and no handle of the arena.
const _: () = {
    const fn assert_published<T: Send + Sync + 'static>() {}
    assert_published::<HitTestSnapshot>();
};

impl LayoutNodeArena {
    /// Publishes the document's committed rows, with its hit-test list, as a snapshot.
    pub(crate) fn publish_hit_test_snapshot(&mut self) -> HitTestSnapshot {
        // The frame's rows pin the list as it is once its structures are built.
        self.prepare_hit_test_list_for_query(true, true);
        HitTestSnapshot {
            frame: self.freeze_frame_without_damage(),
        }
    }
}

impl HitTestSnapshot {
    fn list(&self) -> Option<&HitTestList> {
        self.frame.rows.hit_test_list.as_deref()
    }

    /// The generation of the list, as the document's paint state numbered it; zero for none.
    fn generation(&self) -> u64 {
        self.list().map_or(0, |list| list.generation)
    }

    /// Runs a query over the list, the visual context tree it converts points through and the rows it
    /// was recorded over, or answers `default` where the snapshot holds no list to query.
    fn query<R>(&self, default: R, query: impl FnOnce(&HitTestList, &VisualContextTree, &PaintSource<'_>) -> R) -> R {
        let (Some(list), Some(tree)) = (self.list(), self.frame.rows.visual_context_tree.as_deref()) else {
            return default;
        };
        if !list.spatial_indexes_built {
            return default;
        }
        let absolute_rects = RefCell::default();
        let _pass = crate::painting::seal::enter(crate::painting::seal::Pass::HitTest);
        query(list, tree, &PaintSource::new(&self.frame, &absolute_rects))
    }

    /// Runs a query over the list and the rows it was recorded over.
    fn read<R>(&self, default: R, read: impl FnOnce(&HitTestList, &PaintSource<'_>) -> R) -> R {
        let Some(list) = self.list() else {
            return default;
        };
        let absolute_rects = RefCell::default();
        read(list, &PaintSource::new(&self.frame, &absolute_rects))
    }
}

/// An item of a snapshot's list, as the host reads it: rows, which the host finds what they stand for.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiHitTestSnapshotItem {
    pub can_produce_caret_position: bool,
    pub paintable: NodeSlotId,
    pub hit_node: NodeSlotId,
    pub chrome_widget_kind: u8,
    pub caret_node: NodeSlotId,
    pub caret_rect: crate::layout::used_values::FfiCssPixelRect,
    pub context: ContextRef,
}

/// What a hit on a snapshot's item resolves to: the rows an event is dispatched to, and where in its
/// node it landed.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiHitTestSnapshotHit {
    pub dispatch: NodeSlotId,
    pub allow_pseudo_fallback: bool,
    pub fallback_dispatch: NodeSlotId,
    pub has_index_in_node: bool,
    pub index_in_node: usize,
    pub is_text_fragment: bool,
}

impl Default for FfiHitTestSnapshotHit {
    fn default() -> Self {
        Self {
            dispatch: NodeSlotId::INVALID,
            allow_pseudo_fallback: false,
            fallback_dispatch: NodeSlotId::INVALID,
            has_index_in_node: false,
            index_in_node: 0,
            is_text_fragment: false,
        }
    }
}

/// SAFETY: `snapshot` must be a live handle from `layout_arena_publish_hit_test_snapshot`.
unsafe fn snapshot_from_handle<'a>(snapshot: *const c_void) -> &'a HitTestSnapshot {
    assert!(!snapshot.is_null(), "hit-test snapshot handle is null");
    unsafe { &*snapshot.cast::<HitTestSnapshot>() }
}

/// Publishes the document's committed rows, with its hit-test list, as a snapshot the caller
/// releases with `hit_test_snapshot_release`.
///
/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_hit_test_snapshot(arena: *mut c_void) -> *const c_void {
    let arena = unsafe { crate::painting::ffi::arena_from_handle_mut(arena) };
    Arc::into_raw(Arc::new(arena.publish_hit_test_snapshot())).cast()
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_hit_test_snapshot`, released once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_release(snapshot: *const c_void) {
    assert!(!snapshot.is_null(), "hit-test snapshot handle is null");
    drop(unsafe { Arc::from_raw(snapshot.cast::<HitTestSnapshot>()) });
}

/// The generation of the snapshot's list, as the document's paint state numbered it; zero for none.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_generation(snapshot: *const c_void) -> u64 {
    unsafe { snapshot_from_handle(snapshot) }.generation()
}

/// Visits the chrome widgets the snapshot's list holds items of.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_hit_test_snapshot`, and `visit` must
/// accept `sink` for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_visit_chrome_widgets(
    snapshot: *const c_void,
    sink: *mut c_void,
    visit: unsafe extern "C" fn(*mut c_void, NodeSlotId, u8),
) {
    let Some(list) = unsafe { snapshot_from_handle(snapshot) }.list() else {
        return;
    };
    for item in list.items.iter() {
        if item.chrome_widget_kind != crate::painting::hit_test::CHROME_WIDGET_NONE {
            // SAFETY: The C++ host consumes the visit synchronously.
            unsafe { visit(sink, item.paintable, item.chrome_widget_kind) };
        }
    }
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_hit_test_snapshot`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_find_topmost_item(
    snapshot: *const c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
) -> crate::painting::host::FfiTopmostItem {
    unsafe { snapshot_from_handle(snapshot) }.query(Default::default(), |list, tree, rows| {
        crate::painting::ffi::ffi_topmost(list.find_topmost_item(rows, tree, &callbacks, point.into()))
    })
}

/// Pushes the index of every item the point hits, topmost first.
///
/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_hit_test_snapshot`, and `push` must
/// accept `push_context` for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_all(
    snapshot: *const c_void,
    callbacks: crate::painting::host::FfiHitTestQueryCallbacks,
    point: FfiCssPixelPoint,
    push_context: *mut c_void,
    push: unsafe extern "C" fn(*mut c_void, usize),
) {
    let indices = unsafe { snapshot_from_handle(snapshot) }.query(Vec::new(), |list, tree, rows| {
        list.hit_test_all(rows, tree, &callbacks, point.into())
    });
    for index in indices {
        // SAFETY: The C++ sink consumes the index synchronously.
        unsafe { push(push_context, index) };
    }
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_hit_test_snapshot`, and `index` an
/// item of its list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_item(snapshot: *const c_void, index: usize) -> FfiHitTestSnapshotItem {
    let snapshot = unsafe { snapshot_from_handle(snapshot) };
    let list = snapshot.list().expect("an item of a snapshot with no list");
    let item = &list.items[index];
    FfiHitTestSnapshotItem {
        can_produce_caret_position: item.can_produce_caret_position,
        paintable: item.paintable,
        hit_node: item.hit_node,
        chrome_widget_kind: item.chrome_widget_kind,
        caret_node: item.caret_node,
        caret_rect: item.caret_rect.into(),
        context: item.context,
    }
}

/// # Safety
///
/// `snapshot` must be a live handle from `layout_arena_publish_hit_test_snapshot`, and `index` an
/// item of its list.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn hit_test_snapshot_resolve_hit(
    snapshot: *const c_void,
    index: usize,
    local_point: FfiCssPixelPoint,
) -> FfiHitTestSnapshotHit {
    unsafe { snapshot_from_handle(snapshot) }.read(Default::default(), |list, rows| {
        let local_point: CssPixelPoint = local_point.into();
        let resolved = list.resolve_hit(rows, index, local_point);
        FfiHitTestSnapshotHit {
            dispatch: resolved.dispatch.unwrap_or(NodeSlotId::INVALID),
            allow_pseudo_fallback: resolved.allow_pseudo_fallback,
            fallback_dispatch: resolved.fallback_dispatch.unwrap_or(NodeSlotId::INVALID),
            has_index_in_node: resolved.has_index_in_node,
            index_in_node: resolved.index_in_node,
            is_text_fragment: resolved.is_text_fragment,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::painting::visual_context::{TransformData, TransformDataRole};

    fn tree() -> Option<Arc<VisualContextTree>> {
        Some(Arc::new(VisualContextTree::create(TransformData {
            matrix: libgfx_rust::FloatMatrix4x4::identity(),
            origin: Default::default(),
            sorting_context_root_index: None,
            flattens_inherited_transform: false,
            role: TransformDataRole::CssTransform,
            synthetic_plane: false,
            establishes_sorting_context: false,
        })))
    }

    /// A snapshot answers from the list and the visual context tree it was published with, its
    /// structures built, whatever the arena publishes after it.
    #[test]
    fn a_snapshot_answers_from_what_it_was_published_with() {
        let mut arena = LayoutNodeArena::new();
        *arena.hit_test_list.get_mut() = Some(Arc::new(HitTestList {
            generation: 1,
            ..Default::default()
        }));
        let published_tree = tree();
        arena.paint_state().borrow_mut().visual_context.tree = published_tree.clone();
        let snapshot = arena.publish_hit_test_snapshot();

        *arena.hit_test_list.get_mut() = Some(Arc::new(HitTestList {
            generation: 2,
            ..Default::default()
        }));
        arena.paint_state().borrow_mut().visual_context.tree = None;
        arena.publish_paintable_rows();

        assert_eq!(snapshot.generation(), 1);
        let list = snapshot.list().expect("the snapshot holds its list");
        assert!(list.spatial_indexes_built && list.caret_lines_built);
        assert!(snapshot.query(false, |_, tree, _| std::ptr::eq(
            tree,
            published_tree.as_deref().unwrap()
        )));
        assert_eq!(arena.publish_hit_test_snapshot().generation(), 2);
    }
}
