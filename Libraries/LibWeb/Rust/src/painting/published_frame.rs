/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What a document publishes for the paint side to read, and the one trait the paint side reads it
//! through.
//!
//! A [`PublishedFrame`] is immutable: every column in it is a [`ColumnSnapshot`] or an `Arc` the
//! document shares with it, so the document writes its live columns (copying a chunk a frame still
//! shares) while a frame is read. [`PaintRead`] names every read the display list recording makes
//! of a document, so the recording is written against the trait rather than against the live
//! [`LayoutNodeArena`], and the compiler lists what a frame still has to answer.

use crate::cow_column::ColumnSnapshot;
use crate::css::computed_value_views::ComputedValuesView;
use crate::layout::LayoutNodeArena;
use crate::layout::fragment_tree::FragmentLink;
use crate::layout::node_data::{NodeKind, NodeSlotId};
use crate::painting::hit_test::HitTestList;
use crate::painting::image_map_areas::ImageMapAreas;
use crate::painting::paintable_data::{CommittedSideData, PaintableData};
use crate::painting::paintable_rows::{CommittedFragmentLinkSlot, CommittedSideDataRef, PAINTABLE_SLOTS_PER_CHUNK};
use crate::painting::visual_context::VisualContextTree;
use crate::painting::visual_context::scroll_state::ScrollOffsets;
use std::sync::Arc;

/// One published generation of a document's paintable rows and of the columns read beside them.
pub(crate) struct PublishedFrame {
    pub(super) rows: ColumnSnapshot<PaintableData, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) fragment_links: ColumnSnapshot<CommittedFragmentLinkSlot, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) side_data: ColumnSnapshot<CommittedSideData, PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) unique_node_ids: ColumnSnapshot<(NodeSlotId, i64), PAINTABLE_SLOTS_PER_CHUNK>,
    pub(super) scroll_offsets: Arc<ScrollOffsets>,
    pub(super) image_map_areas: Arc<ImageMapAreas>,
    pub(super) hit_test_list: Option<Arc<HitTestList>>,
    pub(super) visual_context_tree: Option<Arc<VisualContextTree>>,
}

// A frame is read on whichever thread paints it while the document writes its live columns: it
// holds no cell, no raw pointer and no borrow of the document.
const _: () = {
    const fn assert_published<T: Send + Sync + 'static>() {}
    assert_published::<PublishedFrame>();
};

/// The reads the display list recording makes of a document. The live arena answers them from the
/// columns layout writes; a [`PublishedFrame`] answers them from what the document published.
pub(crate) trait PaintRead {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData;
    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool;
    /// Reads the fragment link a populated row committed, from the same generation as its row.
    fn with_committed_fragment_link<R>(&self, id: NodeSlotId, read: impl FnOnce(Option<&FragmentLink>) -> R) -> R;
    /// The side data a populated row committed, from the same generation as its row.
    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_>;

    fn slot_is_live(&self, id: NodeSlotId) -> bool;
    fn node_kind_if_live(&self, id: NodeSlotId) -> Option<NodeKind>;
    fn node_flags_if_live(&self, id: NodeSlotId) -> u32;
    fn node_parent_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_first_child_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_next_sibling_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_containing_block_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_generated_for(&self, id: NodeSlotId) -> u8;
    fn node_is_generated_for_pseudo_element(&self, id: NodeSlotId) -> bool;
    fn node_is_dom_backed(&self, id: NodeSlotId) -> bool;
    fn node_is_element_backed(&self, id: NodeSlotId) -> bool;
    fn node_is_out_of_flow_if_live(&self, id: NodeSlotId) -> bool;
    fn node_style_if_live(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>>;
}

/// Answers [`PaintRead`]'s layout tree and style reads from the live arena the implementing type
/// dereferences to.
macro_rules! read_layout_tree_from_live_arena {
    () => {
        fn slot_is_live(&self, id: NodeSlotId) -> bool {
            LayoutNodeArena::slot_is_live(self, id)
        }

        fn node_kind_if_live(&self, id: NodeSlotId) -> Option<NodeKind> {
            LayoutNodeArena::node_kind_if_live(self, id)
        }

        fn node_flags_if_live(&self, id: NodeSlotId) -> u32 {
            LayoutNodeArena::node_flags_if_live(self, id)
        }

        fn node_parent_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
            LayoutNodeArena::node_parent_if_live(self, id)
        }

        fn node_first_child_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
            LayoutNodeArena::node_first_child_if_live(self, id)
        }

        fn node_next_sibling_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
            LayoutNodeArena::node_next_sibling_if_live(self, id)
        }

        fn node_containing_block_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId> {
            LayoutNodeArena::node_containing_block_if_live(self, id)
        }

        fn node_generated_for(&self, id: NodeSlotId) -> u8 {
            LayoutNodeArena::node_generated_for(self, id)
        }

        fn node_is_generated_for_pseudo_element(&self, id: NodeSlotId) -> bool {
            LayoutNodeArena::node_is_generated_for_pseudo_element(self, id)
        }

        fn node_is_dom_backed(&self, id: NodeSlotId) -> bool {
            LayoutNodeArena::node_is_dom_backed(self, id)
        }

        fn node_is_element_backed(&self, id: NodeSlotId) -> bool {
            LayoutNodeArena::node_is_element_backed(self, id)
        }

        fn node_is_out_of_flow_if_live(&self, id: NodeSlotId) -> bool {
            LayoutNodeArena::node_is_out_of_flow_if_live(self, id)
        }

        fn node_style_if_live(&self, id: NodeSlotId) -> Option<ComputedValuesView<'_>> {
            LayoutNodeArena::node_style_if_live(self, id)
        }
    };
}

pub(crate) use read_layout_tree_from_live_arena;

impl PaintRead for LayoutNodeArena {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData {
        self.live_paintable_data(id)
    }

    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool {
        LayoutNodeArena::paintable_row_is_populated(self, id)
    }

    fn with_committed_fragment_link<R>(&self, id: NodeSlotId, read: impl FnOnce(Option<&FragmentLink>) -> R) -> R {
        LayoutNodeArena::with_committed_fragment_link(self, id, read)
    }

    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_> {
        CommittedSideDataRef::Live(self.live_committed_side_data(id))
    }

    read_layout_tree_from_live_arena!();
}
