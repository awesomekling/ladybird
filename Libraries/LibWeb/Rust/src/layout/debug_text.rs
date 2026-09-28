/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Debug text about a document's boxes, which the render owner writes and the document thread finishes: the owner
//! knows each box, and the document thread knows the DOM node it was built for.

use super::LayoutNodeArena;
use super::node_data::{NodeFlag, NodeKind, NodeSlotId};
use std::ffi::c_void;
use std::fmt::Write;

/// Appends `length` bytes at `bytes` to the sink it is handed.
pub(crate) type AppendBytes = unsafe extern "C" fn(sink: *mut c_void, bytes: *const u8, length: usize);

/// Appends what the host describes the DOM node `node` names as (a style node, or 0 for the document) to `sink`
/// through `append`: `<` its node name `>`, then its ID and classes, or `(anonymous)` where the node is gone.
pub(crate) type DescribeDomNode =
    unsafe extern "C" fn(context: *mut c_void, node: u32, sink: *mut c_void, append: AppendBytes);

/// Text the owner wrote about a document's boxes, with the DOM node of each box it names left for the document thread
/// to describe.
#[derive(Clone, Debug, Default)]
pub(crate) struct DebugText {
    text: String,
    /// Where in the text each DOM node's description goes, and the node: its style node, or 0 for the document.
    nodes: Vec<(usize, u32)>,
}

impl DebugText {
    /// The text written so far, to write more to.
    pub(crate) fn text(&mut self) -> &mut String {
        &mut self.text
    }

    /// Names `row` as the host's debug description of a box does: its kind, then the DOM node it was built for, or
    /// `(anonymous)`.
    pub(crate) fn push_box(&mut self, arena: &LayoutNodeArena, row: NodeSlotId) {
        let data = arena.data(row);
        let kind = data.kind.get();
        let _ = write!(self.text, "{kind:?}");
        let node = if data.flags.get() & NodeFlag::Anonymous as u32 != 0 {
            None
        } else if kind == NodeKind::Viewport {
            Some(0)
        } else {
            arena.node_style_node(row).map(|node| node.raw())
        };
        match node {
            Some(node) => self.push_dom_node(node),
            None => self.text.push_str("(anonymous)"),
        }
    }

    /// Leaves the description of the DOM node `node` names (a style node, or 0 for the document) to the host.
    pub(crate) fn push_dom_node(&mut self, node: u32) {
        self.nodes.push((self.text.len(), node));
    }

    /// How many DOM nodes the text names.
    pub(crate) fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// The DOM nodes the text names from the `start`th on, in order.
    pub(crate) fn nodes_from(&self, start: usize) -> impl Iterator<Item = u32> + '_ {
        self.nodes[start.min(self.nodes.len())..].iter().map(|&(_, node)| node)
    }

    /// The text, with each DOM node it names as `describe` describes it.
    pub(crate) fn finish(self, mut describe: impl FnMut(u32, &mut String)) -> String {
        let mut text = String::with_capacity(self.text.len());
        let mut from = 0;
        for (at, node) in self.nodes {
            text.push_str(&self.text[from..at]);
            describe(node, &mut text);
            from = at;
        }
        text.push_str(&self.text[from..]);
        text
    }

    /// The text, with each DOM node it names as the host's `describe` describes it.
    ///
    /// # Safety
    ///
    /// `describe` must be callable with `context` for the duration of this call.
    pub(crate) unsafe fn finish_with_host(self, context: *mut c_void, describe: DescribeDomNode) -> String {
        unsafe extern "C" fn append(sink: *mut c_void, bytes: *const u8, length: usize) {
            // SAFETY: The host hands the sink back with bytes valid for this call.
            let bytes = unsafe { libcompositing_rust::ffi::ffi_slice(bytes, length) };
            // SAFETY: The sink is the string `finish` describes into, alive for the call.
            unsafe { &mut *sink.cast::<String>() }.push_str(&String::from_utf8_lossy(bytes));
        }
        self.finish(|node, text| {
            // SAFETY: Guaranteed by the caller.
            unsafe { describe(context, node, std::ptr::from_mut(text).cast(), append) };
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_host_describes_each_node_where_it_was_named() {
        let mut text = DebugText::default();
        text.text().push_str("SC for Viewport");
        text.push_dom_node(0);
        text.text().push_str(" [0,0 800x600]\nSC for BlockContainer");
        text.push_dom_node(7);
        text.text().push('\n');
        let finished = text.finish(|node, text| {
            text.push_str(if node == 0 { "<#document>" } else { "<DIV>#a" });
        });
        assert_eq!(
            finished,
            "SC for Viewport<#document> [0,0 800x600]\nSC for BlockContainer<DIV>#a\n"
        );
    }
}
