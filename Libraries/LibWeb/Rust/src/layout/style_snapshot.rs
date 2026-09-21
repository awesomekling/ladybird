/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Layout outputs retained for the next style stage.
//!
//! Layout builds a private generation and publishes it atomically when the commit completes. The
//! style engine holds the same store and clones the published `Arc` before reading a row, so no
//! style evaluation observes a partly committed table.

use crate::css::style::fast_hash::FastMap as HashMap;
use crate::css::style::tree::StyleNodeID;
use crate::layout::used_values::FfiCssPixelSize;
use crate::layout::{LayoutNodeArena, node_data::NodeSlotId};
use std::ffi::c_void;
use std::sync::{Arc, Mutex, RwLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LayoutStyleSnapshotRow {
    pub(crate) content_width_raw: i32,
    pub(crate) content_height_raw: i32,
    pub(crate) stuck: u8,
    pub(crate) snapped: u8,
    pub(crate) scrollable: u8,
    pub(crate) scrolled: u8,
    pub(crate) layout_commit_generation: u64,
    pub(crate) has_committed_box: bool,
    pub(crate) writing_mode: u8,
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FfiLayoutStyleScrollState {
    pub style_node: u32,
    pub stuck: u8,
    pub snapped: u8,
    pub scrollable: u8,
    pub scrolled: u8,
}

#[derive(Clone, Default)]
struct SnapshotGeneration {
    layout_commit_generation: u64,
    rows: HashMap<StyleNodeID, LayoutStyleSnapshotRow>,
}

#[derive(Default)]
pub(crate) struct LayoutStyleSnapshotStore {
    published: RwLock<Arc<SnapshotGeneration>>,
    building: Mutex<Option<SnapshotGeneration>>,
}

impl LayoutStyleSnapshotStore {
    pub(crate) fn begin_layout_commit(&self, generation: u64) {
        let published = self.published.read().unwrap().clone();
        let mut next = (*published).clone();
        next.layout_commit_generation = generation;
        *self.building.lock().unwrap() = Some(next);
    }

    pub(crate) fn publish_geometry(
        &self,
        node: StyleNodeID,
        size: FfiCssPixelSize,
        has_committed_box: bool,
        writing_mode: u8,
    ) {
        let mut building = self.building.lock().unwrap();
        let building = building
            .as_mut()
            .expect("layout snapshot geometry published outside a commit");
        let row = building.rows.entry(node).or_default();
        row.content_width_raw = size.width.raw_value();
        row.content_height_raw = size.height.raw_value();
        row.layout_commit_generation = building.layout_commit_generation;
        row.has_committed_box = has_committed_box;
        row.writing_mode = writing_mode;
    }

    pub(crate) fn finish_layout_commit(&self) {
        let generation = self
            .building
            .lock()
            .unwrap()
            .take()
            .expect("layout snapshot commit finished without beginning");
        *self.published.write().unwrap() = Arc::new(generation);
    }

    pub(crate) fn publish_scroll_states(&self, states: &[FfiLayoutStyleScrollState]) {
        if states.is_empty() {
            return;
        }
        let published = self.published.read().unwrap().clone();
        let mut next = (*published).clone();
        for state in states {
            let Some(node) = StyleNodeID::from_raw(state.style_node) else {
                continue;
            };
            let generation = next.layout_commit_generation;
            let row = next.rows.entry(node).or_default();
            row.stuck = state.stuck;
            row.snapped = state.snapped;
            row.scrollable = state.scrollable;
            row.scrolled = state.scrolled;
            row.layout_commit_generation = generation;
        }
        *self.published.write().unwrap() = Arc::new(next);
    }

    pub(crate) fn retire(&self, nodes: &[StyleNodeID]) {
        if nodes.is_empty() {
            return;
        }
        let published = self.published.read().unwrap().clone();
        let mut next = (*published).clone();
        for node in nodes {
            next.rows.remove(node);
        }
        *self.published.write().unwrap() = Arc::new(next);
    }

    pub(crate) fn row(&self, node: StyleNodeID) -> Option<LayoutStyleSnapshotRow> {
        self.published.read().unwrap().rows.get(&node).copied()
    }
}

impl LayoutNodeArena {
    pub(crate) fn begin_layout_style_snapshot_commit(&self) {
        self.layout_style_snapshots
            .begin_layout_commit(self.layout_commit_generation());
    }

    pub(crate) fn publish_layout_style_snapshot_geometry(&self, node: NodeSlotId, writing_mode: u8) {
        let Some(style_node) = self.node_style_node(node) else {
            return;
        };
        if self.bound_row(style_node) != node {
            return;
        }
        let rows = self.paintable_rows();
        let has_committed_box = rows.paintable_row_is_populated(node);
        let size = if has_committed_box {
            crate::painting::paintable_geometry::committed_content_size(&rows, node)
        } else {
            FfiCssPixelSize::default()
        };
        self.layout_style_snapshots
            .publish_geometry(style_node, size, has_committed_box, writing_mode);
    }

    pub(crate) fn finish_layout_style_snapshot_commit(&self) {
        self.layout_style_snapshots.finish_layout_commit();
    }
}

/// Publish the document's snapshotted scroll-state query inputs as one immutable generation.
///
/// # Safety
///
/// `arena` must be a live layout arena handle on the document thread. `states` must name `count`
/// initialized rows for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_publish_style_snapshot_scroll_states(
    arena: *mut c_void,
    states: *const FfiLayoutStyleScrollState,
    count: usize,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let states = if count == 0 {
        &[]
    } else {
        assert!(!states.is_null(), "layout style scroll states are null");
        unsafe { std::slice::from_raw_parts(states, count) }
    };
    unsafe { LayoutNodeArena::from_handle(arena) }
        .layout_style_snapshots
        .publish_scroll_states(states);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::css_pixels::CssPixels;

    #[test]
    fn a_layout_generation_becomes_visible_only_when_finished() {
        let store = LayoutStyleSnapshotStore::default();
        let node = StyleNodeID::element(1);
        store.begin_layout_commit(7);
        store.publish_geometry(
            node,
            FfiCssPixelSize {
                width: CssPixels::from_raw(11),
                height: CssPixels::from_raw(13),
            },
            true,
            crate::css::css_enums::writing_mode::HORIZONTAL_TB,
        );
        assert!(store.row(node).is_none());
        store.finish_layout_commit();
        assert_eq!(
            store.row(node),
            Some(LayoutStyleSnapshotRow {
                content_width_raw: 11,
                content_height_raw: 13,
                layout_commit_generation: 7,
                has_committed_box: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn scroll_state_updates_preserve_geometry_and_retirement_clears_the_row() {
        let store = LayoutStyleSnapshotStore::default();
        let node = StyleNodeID::element(1);
        store.begin_layout_commit(3);
        store.publish_geometry(
            node,
            FfiCssPixelSize::default(),
            true,
            crate::css::css_enums::writing_mode::HORIZONTAL_TB,
        );
        store.finish_layout_commit();
        store.publish_scroll_states(&[FfiLayoutStyleScrollState {
            style_node: node.raw(),
            stuck: 1,
            snapped: 2,
            scrollable: 4,
            scrolled: 8,
        }]);
        let row = store.row(node).unwrap();
        assert!(row.has_committed_box);
        assert_eq!((row.stuck, row.snapped, row.scrollable, row.scrolled), (1, 2, 4, 8));
        store.retire(&[node]);
        assert!(store.row(node).is_none());
    }
}
