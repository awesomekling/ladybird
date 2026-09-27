/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Layout outputs retained for the next style stage.
//!
//! Layout gathers a commit's rows privately, in its arena, and applies them to the published
//! generation at once, under its write lock, when the commit completes, so no style evaluation
//! observes a partly committed table. The published generation changes in place; it is copied only
//! where a reader still holds it.

use crate::css::style::tree::StyleNodeID;
use crate::layout::used_values::FfiCssPixelSize;
use crate::layout::{LayoutNodeArena, node_data::NodeSlotId};
use std::ffi::c_void;
use std::sync::{Arc, RwLock};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct LayoutStyleSnapshotRow {
    pub(crate) content_width_raw: i32,
    pub(crate) content_height_raw: i32,
    pub(crate) stuck: u8,
    pub(crate) snapped: u8,
    pub(crate) scrollable: u8,
    pub(crate) scrolled: u8,
    pub(crate) layout_commit_generation: u64,
    pub(crate) style_record: u64,
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

/// The rows of a generation, by element index: only elements are containers, which are all that
/// style asks layout about, and nearly every element has a box.
#[derive(Clone, Default)]
struct SnapshotGeneration {
    layout_commit_generation: u64,
    rows: Vec<Option<LayoutStyleSnapshotRow>>,
}

impl SnapshotGeneration {
    fn row(&self, node: StyleNodeID) -> Option<&LayoutStyleSnapshotRow> {
        self.rows.get(node.element_index()? as usize)?.as_ref()
    }

    fn row_mut(&mut self, node: StyleNodeID) -> Option<&mut LayoutStyleSnapshotRow> {
        let index = node.element_index()? as usize;
        if self.rows.len() <= index {
            self.rows.resize(index + 1, None);
        }
        Some(self.rows[index].get_or_insert_default())
    }
}

#[derive(Clone, Copy)]
pub(crate) struct CommittedGeometry {
    node: StyleNodeID,
    content_width_raw: i32,
    content_height_raw: i32,
    style_record: u64,
    has_committed_box: bool,
    writing_mode: u8,
}

/// The rows of the layout commit in progress, which the arena that commits gathers for itself.
#[derive(Default)]
pub(crate) struct LayoutStyleSnapshotCommit {
    layout_commit_generation: Option<u64>,
    rows: Vec<CommittedGeometry>,
}

impl LayoutStyleSnapshotCommit {
    pub(crate) fn begin(&mut self, generation: u64) {
        debug_assert!(
            self.layout_commit_generation.is_none(),
            "layout snapshot commit began inside another"
        );
        self.layout_commit_generation = Some(generation);
        self.rows.clear();
    }

    pub(crate) fn push(
        &mut self,
        node: StyleNodeID,
        size: FfiCssPixelSize,
        has_committed_box: bool,
        writing_mode: u8,
        style_record: u64,
    ) {
        debug_assert!(
            self.layout_commit_generation.is_some(),
            "layout snapshot geometry published outside a commit"
        );
        debug_assert!(
            node.element_index().is_some(),
            "layout snapshot geometry published for a text node"
        );
        self.rows.push(CommittedGeometry {
            node,
            content_width_raw: size.width.raw_value(),
            content_height_raw: size.height.raw_value(),
            style_record,
            has_committed_box,
            writing_mode,
        });
    }
}

#[derive(Default)]
pub(crate) struct LayoutStyleSnapshotStore {
    published: RwLock<Arc<SnapshotGeneration>>,
}

impl LayoutStyleSnapshotStore {
    /// Applies a commit's rows to the published generation, leaving the commit empty for the next.
    pub(crate) fn finish_layout_commit(&self, commit: &mut LayoutStyleSnapshotCommit) {
        let Some(generation) = commit.layout_commit_generation.take() else {
            debug_assert!(false, "layout snapshot commit finished without beginning");
            commit.rows.clear();
            return;
        };
        let mut published = self.published.write().unwrap();
        let next = Arc::make_mut(&mut published);
        next.layout_commit_generation = generation;
        for geometry in commit.rows.drain(..) {
            let Some(row) = next.row_mut(geometry.node) else {
                continue;
            };
            row.content_width_raw = geometry.content_width_raw;
            row.content_height_raw = geometry.content_height_raw;
            row.layout_commit_generation = generation;
            row.style_record = geometry.style_record;
            row.has_committed_box = geometry.has_committed_box;
            row.writing_mode = geometry.writing_mode;
        }
    }

    pub(crate) fn publish_scroll_states(&self, states: &[FfiLayoutStyleScrollState]) {
        if states.is_empty() {
            return;
        }
        let mut published = self.published.write().unwrap();
        let next = Arc::make_mut(&mut published);
        for state in states {
            let Some(node) = StyleNodeID::from_raw(state.style_node) else {
                continue;
            };
            let generation = next.layout_commit_generation;
            let Some(row) = next.row_mut(node) else {
                continue;
            };
            row.stuck = state.stuck;
            row.snapped = state.snapped;
            row.scrollable = state.scrollable;
            row.scrolled = state.scrolled;
            row.layout_commit_generation = generation;
        }
    }

    pub(crate) fn retire(&self, nodes: &[StyleNodeID]) {
        if nodes.is_empty() {
            return;
        }
        // A script that replaces the text of thousands of elements retires their text nodes one at a time, so the
        // generation is changed in place, and copied only where a reader still holds it.
        let mut published = self.published.write().unwrap();
        if !nodes.iter().any(|node| published.row(*node).is_some()) {
            return;
        }
        let next = Arc::make_mut(&mut published);
        for node in nodes {
            if let Some(index) = node.element_index()
                && let Some(row) = next.rows.get_mut(index as usize)
            {
                *row = None;
            }
        }
    }

    pub(crate) fn row(&self, node: StyleNodeID) -> Option<LayoutStyleSnapshotRow> {
        self.published.read().unwrap().row(node).copied()
    }
}

impl LayoutNodeArena {
    pub(crate) fn begin_layout_style_snapshot_commit(&self) {
        self.layout_style_snapshot_commit
            .borrow_mut()
            .begin(self.layout_commit_generation());
    }

    /// Gathers the node's row for the style snapshot. `laid_out_content_size` is the content size of
    /// the fragment this commit just linked the node's populated row to, which is what the row reads.
    pub(crate) fn publish_layout_style_snapshot_geometry(
        &self,
        node: NodeSlotId,
        laid_out_content_size: Option<FfiCssPixelSize>,
    ) {
        // Style asks layout only about elements.
        let Some(style_node) = self
            .node_style_node(node)
            .filter(|style_node| style_node.element_index().is_some())
        else {
            return;
        };
        if self.bound_row(style_node) != node {
            return;
        }
        let writing_mode = crate::layout::node_facts::node_style_view(self.data(node))
            .map_or(crate::css::css_enums::writing_mode::HORIZONTAL_TB, |style| {
                style.writing_mode()
            });
        let rows = self.paintable_rows();
        let has_committed_box = rows.paintable_row_is_populated(node);
        let size = if has_committed_box {
            laid_out_content_size
                .unwrap_or_else(|| crate::painting::paintable_geometry::committed_content_size(&rows, node))
        } else {
            FfiCssPixelSize::default()
        };
        debug_assert!(
            laid_out_content_size.is_none_or(|laid_out| !has_committed_box
                || laid_out == crate::painting::paintable_geometry::committed_content_size(&rows, node)),
            "a committed row reads another content size than the fragment linked to it"
        );
        self.layout_style_snapshot_commit.borrow_mut().push(
            style_node,
            size,
            has_committed_box,
            writing_mode,
            self.node_style_record(node),
        );
    }

    pub(crate) fn finish_layout_style_snapshot_commit(&self) {
        self.layout_style_snapshots
            .finish_layout_commit(&mut self.layout_style_snapshot_commit.borrow_mut());
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
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    arena.join_frame_for_main_side_write("style snapshot scroll states");
    arena.layout_style_snapshots.publish_scroll_states(states);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::css_pixels::CssPixels;

    #[test]
    fn a_layout_generation_becomes_visible_only_when_finished() {
        let store = LayoutStyleSnapshotStore::default();
        let node = StyleNodeID::element(1);
        let mut commit = LayoutStyleSnapshotCommit::default();
        commit.begin(7);
        commit.push(
            node,
            FfiCssPixelSize {
                width: CssPixels::from_raw(11),
                height: CssPixels::from_raw(13),
            },
            true,
            crate::css::css_enums::writing_mode::HORIZONTAL_TB,
            17,
        );
        assert!(store.row(node).is_none());
        store.finish_layout_commit(&mut commit);
        assert_eq!(
            store.row(node),
            Some(LayoutStyleSnapshotRow {
                content_width_raw: 11,
                content_height_raw: 13,
                layout_commit_generation: 7,
                style_record: 17,
                has_committed_box: true,
                ..Default::default()
            })
        );
    }

    #[test]
    fn scroll_state_updates_preserve_geometry_and_retirement_clears_the_row() {
        let store = LayoutStyleSnapshotStore::default();
        let node = StyleNodeID::element(1);
        let mut commit = LayoutStyleSnapshotCommit::default();
        commit.begin(3);
        commit.push(
            node,
            FfiCssPixelSize::default(),
            true,
            crate::css::css_enums::writing_mode::HORIZONTAL_TB,
            19,
        );
        store.finish_layout_commit(&mut commit);
        store.publish_scroll_states(&[FfiLayoutStyleScrollState {
            style_node: node.raw(),
            stuck: 1,
            snapped: 2,
            scrollable: 4,
            scrolled: 8,
        }]);
        let row = store.row(node).unwrap();
        assert!(row.has_committed_box);
        assert_eq!(row.style_record, 19);
        assert_eq!((row.stuck, row.snapped, row.scrollable, row.scrolled), (1, 2, 4, 8));
        store.retire(&[node]);
        assert!(store.row(node).is_none());
    }
}
