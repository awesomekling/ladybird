/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What a hit test reads besides its list.
//!
//! A hit test, its caret lines and its resolution to the rows a hit names read the rows the list was
//! recorded over as [`PaintRead`] answers them, and nothing else: whatever answers [`PaintRead`], a
//! published frame included, answers a hit test. A hit names rows; which layout node or DOM node a
//! row is, is the caller's to look up.
//!
//! A caret position reads a few facts of the document beside the rows: [`CaretRead`]. Neither
//! trait dereferences to the arena, so a query names no read its trait does not answer.

use crate::css::style::tree::StyleNodeID;
use crate::layout::node_data::NodeSlotId;
use crate::painting::published_frame::PaintRead;

pub(crate) trait CaretRead: PaintRead {
    /// Whether `node` is `scope` or lies below it in the DOM tree of `document`.
    fn node_is_in_dom_subtree_of(&self, node: StyleNodeID, scope: StyleNodeID, document: StyleNodeID) -> bool;
    /// The length, in code units, of the DOM text a text row was built for.
    fn text_content_length(&self, id: NodeSlotId) -> Option<usize>;
}

impl<T: crate::painting::paintable_rows::PaintableRowsRead> CaretRead for T {
    fn node_is_in_dom_subtree_of(&self, node: StyleNodeID, scope: StyleNodeID, document: StyleNodeID) -> bool {
        self.with_style_store(|engine| engine.tree().is_in_dom_subtree_of(node, scope, document))
    }

    fn text_content_length(&self, id: NodeSlotId) -> Option<usize> {
        self.text_content(id).map(|content| content.text.len())
    }
}
