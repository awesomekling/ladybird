/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The reads a box's geometry is computed from: its committed rows and the few facts of its layout
//! node that say how to read them.
//!
//! [`GeometryRead`] is what [`crate::painting::paintable_geometry`], the CSSOM client rects and the
//! rect-to-viewport transform are written against. Every [`PaintRead`] answers it, since painting
//! reads geometry too; a [`crate::painting::query_snapshot::QuerySnapshot`] answers it from what a
//! document published, and nothing else, so a geometry read made through a snapshot names no read
//! a snapshot does not answer.
//!
//! [`PaintRead`]: crate::painting::published_frame::PaintRead

use crate::css::css_pixels::CssPixelRect;
use crate::layout::fragment_tree::FragmentLink;
use crate::layout::node_data::{NodeKind, NodeSlotId};
use crate::painting::paintable_data::PaintableData;
use crate::painting::paintable_rows::CommittedSideDataRef;

pub(crate) trait GeometryRead: Sized {
    fn paintable_data(&self, id: NodeSlotId) -> &PaintableData;
    fn paintable_row_is_populated(&self, id: NodeSlotId) -> bool;
    /// Reads the fragment link a populated row committed, from the same generation as its row.
    fn with_committed_fragment_link<R>(&self, id: NodeSlotId, read: impl FnOnce(Option<&FragmentLink>) -> R) -> R;
    /// The side data a populated row committed, from the same generation as its row.
    fn committed_side_data(&self, id: NodeSlotId) -> CommittedSideDataRef<'_>;

    fn node_kind_if_live(&self, id: NodeSlotId) -> Option<NodeKind>;
    fn node_flags_if_live(&self, id: NodeSlotId) -> u32;
    fn node_parent_if_live(&self, id: NodeSlotId) -> Option<NodeSlotId>;
    fn node_is_fragmented_inline(&self, id: NodeSlotId) -> bool;

    /// The absolute rect memoized for a box, if any.
    fn memoized_absolute_rect(&self, id: NodeSlotId) -> Option<CssPixelRect>;
    fn memoize_absolute_rect(&self, id: NodeSlotId, rect: CssPixelRect);

    /// The line root whose committed side data holds an inline box's pieces, read from the same
    /// generation as the rows.
    fn inline_pieces_root(&self, inline_paintable: NodeSlotId) -> Option<NodeSlotId> {
        if !self.paintable_row_is_populated(inline_paintable) {
            return None;
        }
        let root = self.paintable_data(inline_paintable).containing_block;
        (self.paintable_row_is_populated(root) && crate::painting::node_painting::has_lines(self, root)).then_some(root)
    }
}

/// Answers [`GeometryRead`]'s reads of the layout tree from the live arena that `$arena` maps the
/// implementing type to.
macro_rules! read_live_geometry {
    ($arena:path) => {
        fn node_kind_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::node_data::NodeKind> {
            crate::layout::LayoutNodeArena::node_kind_if_live($arena(self), id)
        }

        fn node_flags_if_live(&self, id: crate::layout::node_data::NodeSlotId) -> u32 {
            crate::layout::LayoutNodeArena::node_flags_if_live($arena(self), id)
        }

        fn node_parent_if_live(
            &self,
            id: crate::layout::node_data::NodeSlotId,
        ) -> Option<crate::layout::node_data::NodeSlotId> {
            crate::layout::LayoutNodeArena::node_parent_if_live($arena(self), id)
        }

        fn node_is_fragmented_inline(&self, id: crate::layout::node_data::NodeSlotId) -> bool {
            crate::layout::LayoutNodeArena::node_is_fragmented_inline($arena(self), id)
        }
    };
}

pub(crate) use read_live_geometry;
