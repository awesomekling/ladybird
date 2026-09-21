/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

/// The arena inputs borrowed while computing a layout result. The pass ends before commit takes a
/// mutable arena borrow, so its text and style views cannot survive commit.
///
/// It holds no host table: a running pass asks the document nothing. See `super::seal`.
#[derive(Clone, Copy)]
pub(crate) struct LayoutPass<'arena> {
    arena: &'arena LayoutNodeArena,
    scratch: &'arena LayoutScratch,
    pub(crate) initial_containing_block_inline_size: CssPixels,
    pub(crate) initial_containing_block_block_size: CssPixels,
    pub(crate) document_in_quirks_mode: bool,
}

impl<'arena> LayoutPass<'arena> {
    pub(crate) fn new(
        arena: &'arena LayoutNodeArena,
        scratch: &'arena LayoutScratch,
        initial_containing_block_inline_size: CssPixels,
        initial_containing_block_block_size: CssPixels,
        document_in_quirks_mode: bool,
    ) -> Self {
        Self {
            arena,
            scratch,
            initial_containing_block_inline_size,
            initial_containing_block_block_size,
            document_in_quirks_mode,
        }
    }

    pub(crate) fn arena(&self) -> &'arena LayoutNodeArena {
        self.arena
    }

    /// The scratch the pass's runs lend their records through.
    pub(crate) fn layout_scratch(&self) -> &'arena LayoutScratch {
        self.scratch
    }

    pub(crate) fn intrinsic_block_size_cache_get(
        &self,
        data: &NodeData,
        kind: IntrinsicSizeCacheKind,
        key: IntrinsicSizeCacheKey,
    ) -> Option<IntrinsicBlockSizeMeasurement> {
        self.scratch
            .intrinsic_size_caches
            .intrinsic_block_size_cache_get(self.arena, data, kind, key)
    }

    pub(crate) fn intrinsic_block_size_cache_put(
        &self,
        data: &NodeData,
        kind: IntrinsicSizeCacheKind,
        key: IntrinsicSizeCacheKey,
        value: IntrinsicBlockSizeMeasurement,
    ) {
        self.scratch
            .intrinsic_size_caches
            .intrinsic_block_size_cache_put(self.arena, data, kind, key, value);
    }

    pub(crate) fn intrinsic_inline_size_measurement_cache_get(
        &self,
        data: &NodeData,
        kind: IntrinsicSizeCacheKind,
        key: IntrinsicSizeCacheKey,
    ) -> Option<IntrinsicInlineSizeMeasurement> {
        self.scratch
            .intrinsic_size_caches
            .intrinsic_inline_size_measurement_cache_get(self.arena, data, kind, key)
    }

    pub(crate) fn intrinsic_inline_size_measurement_cache_put(
        &self,
        data: &NodeData,
        kind: IntrinsicSizeCacheKind,
        key: IntrinsicSizeCacheKey,
        value: IntrinsicInlineSizeMeasurement,
    ) {
        self.scratch
            .intrinsic_size_caches
            .intrinsic_inline_size_measurement_cache_put(self.arena, data, kind, key, value);
    }

    pub(crate) fn intrinsic_inline_size_depends_on_block_size(
        &self,
        data: &NodeData,
        compute: impl FnOnce() -> bool,
    ) -> bool {
        self.scratch
            .intrinsic_size_caches
            .intrinsic_inline_size_depends_on_block_size(self.arena, data, compute)
    }

    pub(crate) fn table_cell_measurement_cache_get(
        &self,
        data: &NodeData,
        key: TableCellMeasurementKey,
    ) -> Option<TableCellMeasurement> {
        self.scratch
            .intrinsic_size_caches
            .table_cell_measurement_cache_get(self.arena, data, key)
    }

    pub(crate) fn table_cell_measurement_cache_put(
        &self,
        data: &NodeData,
        key: TableCellMeasurementKey,
        value: TableCellMeasurement,
    ) {
        self.scratch
            .intrinsic_size_caches
            .table_cell_measurement_cache_put(self.arena, data, key, value);
    }

    pub(crate) fn node_data(&self, node: Node) -> &'arena NodeData {
        self.arena.assert_layout_read_is_in_scope(node);
        self.arena().data(node)
    }

    pub(crate) fn text_content(&self, node: Node) -> &'arena super::rendered_text::TextContent {
        self.arena.assert_layout_read_is_in_scope(node);
        self.arena
            .text_content(node)
            .expect("text node content must be synced to the arena before layout")
    }

    pub(crate) fn style_payloads(&self, node: Node) -> &'arena FfiStylePayloads {
        self.arena.assert_layout_read_is_in_scope(node);
        self.arena
            .style_payloads(node)
            .expect("styled node must publish its style container before layout")
    }

    pub(crate) fn replaced_content_facts(&self, node: Node) -> Option<FfiReplacedContentFacts> {
        self.arena.assert_layout_read_is_in_scope(node);
        self.arena.replaced_content_facts(node)
    }

    pub(crate) fn computed_values_view_if_styled(&self, node: Node) -> Option<ComputedValuesView<'arena>> {
        self.arena.assert_layout_read_is_in_scope(node);
        self.arena
            .style_payloads(node)
            .map(|payloads| ComputedValuesView::new(&payloads.groups))
    }

    pub(crate) fn can_skip_is_anonymous_text_run(&self, node: Node) -> bool {
        let data = self.node_data(node);
        if !node_facts::has_flag(data, NodeFlag::Anonymous) || data.generated_for.get() != 0 {
            return false;
        }

        let mut child = data.first_child.get();
        while !child.is_invalid() {
            let data = self.node_data(child);
            if !node_facts::kind_is_text(data.kind.get())
                || !self.text_content(child).untransformed_text_is_ascii_whitespace
            {
                return false;
            }
            child = data.next_sibling.get();
        }
        true
    }

    pub(crate) fn is_before(&self, node: Node, other: Node) -> bool {
        self.arena().is_before(self.node_data(node), self.node_data(other))
    }

    pub(crate) fn saved_abspos_layout_inputs(&self, node: Node) -> Option<abspos_inputs::AbsposLayoutInputs> {
        let data = self.node_data(node);
        assert!(node_facts::kind_is_box(data.kind.get()));
        self.arena().saved_abspos_layout_inputs(data)
    }

    pub(crate) fn committed_fragment_link(&self, node: Node) -> Option<FragmentLink> {
        self.arena().committed_fragment_link(self.node_data(node))
    }

    #[inline]
    pub(crate) fn has_committed_fragment_link(&self, node: Node) -> bool {
        self.node_data(node).flags.get() & NodeFlag::HasCommittedFragmentLink as u32 != 0
    }

    #[inline]
    pub(crate) fn parent(&self, node: Node) -> Node {
        self.node_data(node).parent.get()
    }

    #[inline]
    pub(crate) fn first_child(&self, node: Node) -> Node {
        self.node_data(node).first_child.get()
    }

    #[inline]
    pub(crate) fn last_child(&self, node: Node) -> Node {
        self.node_data(node).last_child.get()
    }

    #[inline]
    pub(crate) fn next_sibling(&self, node: Node) -> Node {
        self.node_data(node).next_sibling.get()
    }

    #[inline]
    pub(crate) fn previous_sibling(&self, node: Node) -> Node {
        self.node_data(node).previous_sibling.get()
    }

    pub(crate) fn in_flow_containing_block(&self, node: Node) -> Node {
        let (innermost_root, innermost_root_containing_block) = self.arena.innermost_run.get();
        if node == innermost_root {
            return innermost_root_containing_block;
        }
        let mut ancestor = self.parent(node);
        while !ancestor.is_invalid() {
            let data = self.node_data(ancestor);
            if node_facts::node_forms_containing_block_for_children(data, node_facts::node_style_view(data)) {
                return ancestor;
            }
            if ancestor == innermost_root {
                return innermost_root_containing_block;
            }
            ancestor = data.parent.get();
        }
        NodeSlotId::INVALID
    }

    pub(crate) fn containing_block_for_child_run(
        &self,
        child: Node,
        participation: &ParticipationInParentFormattingContext,
    ) -> Node {
        match participation {
            ParticipationInParentFormattingContext::AbsolutelyPositioned(inputs) => inputs.containing_block,
            _ => self.in_flow_containing_block(child),
        }
    }

    /// Whether `ancestor` is `node` or one of its ancestors. The walk stops at `root`, so `node` must be
    /// in `root`'s subtree and `ancestor` must not be above `root`.
    pub(crate) fn is_ancestor(&self, ancestor: Node, mut node: Node, root: Node) -> bool {
        while !node.is_invalid() {
            if node == ancestor {
                return true;
            }
            if node == root {
                return false;
            }
            node = self.node_data(node).parent.get();
        }
        false
    }
}
