/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;
use crate::layout::node_data::NodeFlag;
use crate::painting::hit_test::read::CaretRead;
use crate::painting::host::FfiCaretBoundaryKind;
use crate::painting::paintable_data::SELECTION_STATE_START_AND_END;
use crate::painting::published_frame::PaintRead;

/// What a hit names for an event to be dispatched to, and where in its node it landed: rows, which
/// the caller finds the layout nodes of.
#[derive(Default)]
pub(crate) struct ResolvedHit {
    pub(crate) dispatch: Option<NodeSlotId>,
    /// Whether a dispatch row that stands for no DOM node dispatches to the element its pseudo-element
    /// was generated for.
    pub(crate) allow_pseudo_fallback: bool,
    pub(crate) fallback_dispatch: Option<NodeSlotId>,
    pub(crate) has_index_in_node: bool,
    pub(crate) index_in_node: usize,
    pub(crate) is_text_fragment: bool,
}

/// The caret position a hit resolves to, in the row whose node it is a position in.
#[derive(Default)]
pub(crate) struct ResolvedCaret {
    pub(crate) has_position: bool,
    pub(crate) node: Option<NodeSlotId>,
    pub(crate) boundary: FfiCaretBoundaryKind,
    pub(crate) offset: usize,
    pub(crate) affinity_is_upstream: bool,
    pub(crate) has_debug_rect: bool,
    pub(crate) debug_rect: CssPixelRect,
}

pub(crate) fn empty_line_is_anchored_to_its_forced_break(arena: &impl PaintRead, item: &HitTestItem) -> bool {
    arena.node_kind_if_live(item.caret_node) == Some(crate::layout::node_data::NodeKind::BreakNode)
}

/// The DOM node a row stands for, named the way the host names one: by the style node, or by 0
/// for a row that stands for no node of its own. An anonymous row stands for none, and so does
/// the viewport row, whose node is the document and which the style mirror holds no identity for.
pub(crate) fn row_dom_style_node(arena: &impl CaretRead, slot: NodeSlotId) -> u32 {
    if !arena.node_is_dom_backed(slot) {
        return 0;
    }
    if arena.node_kind_if_live(slot) == Some(crate::layout::node_data::NodeKind::Viewport) {
        return 0;
    }
    arena.node_style_node(slot).map_or(0, |style_node| style_node.raw())
}

/// Whether the node a row stands for is `scope` or lies below it in the DOM tree. The scope is a
/// node the host named before the query, and the answer is read out of the style mirror, which
/// carries the DOM child sequence the test walks.
pub(crate) fn row_is_in_scope(arena: &impl CaretRead, scope: u32, document: u32, slot: NodeSlotId) -> bool {
    use crate::css::style::tree::StyleNodeID;
    let (Some(scope), Some(document), Some(node)) = (
        StyleNodeID::from_raw(scope),
        StyleNodeID::from_raw(document),
        StyleNodeID::from_raw(row_dom_style_node(arena, slot)),
    ) else {
        return false;
    };
    arena.node_is_in_dom_subtree_of(node, scope, document)
}

impl HitTestList {
    pub(crate) fn item_target_slot(&self, arena: &impl PaintRead, item_index: usize) -> Option<NodeSlotId> {
        let item = &self.items[item_index];
        match item.kind {
            HitTestItemKind::TextFragment => fragment_layout_node_slot(arena, item),
            HitTestItemKind::EmptyLine => Some(item.caret_node),
            _ => Some(item.paintable),
        }
    }

    /// The row an event at the item is dispatched to, and whether one that stands for no DOM node
    /// dispatches to the element its pseudo-element was generated for.
    pub(crate) fn item_dispatch_slot(&self, arena: &impl PaintRead, item_index: usize) -> (Option<NodeSlotId>, bool) {
        let item = &self.items[item_index];
        match item.kind {
            HitTestItemKind::TextFragment => (fragment_layout_node_slot(arena, item), true),
            HitTestItemKind::EmptyLine => (Some(item.caret_node), false),
            _ => (event_dispatch_slot_for_paintable(arena, item.paintable), false),
        }
    }

    pub(crate) fn resolve_hit(
        &self,
        arena: &impl PaintRead,
        item_index: usize,
        local_point: CssPixelPoint,
    ) -> ResolvedHit {
        let item = &self.items[item_index];
        let (dispatch, allow_pseudo_fallback) = self.item_dispatch_slot(arena, item_index);
        let mut result = ResolvedHit {
            dispatch,
            allow_pseudo_fallback,
            ..Default::default()
        };
        match item.kind {
            HitTestItemKind::TextFragment => {
                result.fallback_dispatch = event_dispatch_slot_for_paintable(arena, item.paintable);
                result.has_index_in_node = true;
                result.index_in_node = fragment_index_in_node_for_point(arena, item, local_point);
                result.is_text_fragment = true;
            }
            HitTestItemKind::EmptyEditable => {
                result.has_index_in_node = true;
            }
            _ => {}
        }
        result
    }

    pub(crate) fn resolve_caret(
        &self,
        arena: &impl CaretRead,
        item_index: usize,
        local_point: CssPixelPoint,
        position_type: crate::painting::hit_test::caret::CaretPositionType,
    ) -> ResolvedCaret {
        let item = &self.items[item_index];
        match item.kind {
            HitTestItemKind::TextFragment => with_item_fragment(arena, item, |fragment| {
                let offset = match position_type {
                    crate::painting::hit_test::caret::CaretPositionType::Before => fragment.dom_start_offset_in_node,
                    crate::painting::hit_test::caret::CaretPositionType::After => {
                        // INTEROP: Fully collapsed whitespace at the end of a text run is not a caret stop.
                        //          Keep the whitespace boundary at soft wraps for upstream affinity.
                        let end = fragment.start
                            + fragment.length_in_code_units
                            + fragment.trailing_whitespace_length_in_code_units;
                        if arena
                            .text_content_length(fragment.layout_node)
                            .is_some_and(|length| end < length)
                        {
                            fragment.dom_end_offset_with_trailing_whitespace
                        } else {
                            fragment.dom_end_offset_in_node
                        }
                    }
                    crate::painting::hit_test::caret::CaretPositionType::Closest => {
                        let paintable_rows = arena;
                        crate::painting::text_fragment::index_in_node_for_point(paintable_rows, fragment, local_point)
                    }
                };
                let affinity_is_upstream = offset >= fragment.dom_end_offset_in_node
                    && offset == fragment.dom_end_offset_with_trailing_whitespace;
                let debug_rect = fragment_caret_range_rect(arena, fragment, offset);
                ResolvedCaret {
                    has_position: true,
                    node: Some(fragment.layout_node),
                    boundary: FfiCaretBoundaryKind::Offset,
                    offset,
                    affinity_is_upstream,
                    has_debug_rect: true,
                    debug_rect,
                }
            })
            .unwrap_or_default(),
            HitTestItemKind::EmptyLine => ResolvedCaret {
                has_position: true,
                node: Some(item.caret_node),
                boundary: if empty_line_is_anchored_to_its_forced_break(arena, item) {
                    FfiCaretBoundaryKind::IndexOfNodeInParent
                } else {
                    FfiCaretBoundaryKind::Offset
                },
                offset: item.caret_offset,
                has_debug_rect: true,
                debug_rect: item.caret_rect,
                ..Default::default()
            },
            HitTestItemKind::EmptyEditable => ResolvedCaret {
                has_position: true,
                node: Some(item.paintable),
                boundary: FfiCaretBoundaryKind::Offset,
                has_debug_rect: true,
                debug_rect: item.caret_rect,
                ..Default::default()
            },
            HitTestItemKind::Box => {
                let is_before = match position_type {
                    crate::painting::hit_test::caret::CaretPositionType::Before => true,
                    crate::painting::hit_test::caret::CaretPositionType::After => false,
                    crate::painting::hit_test::caret::CaretPositionType::Closest => {
                        self.box_point_is_before(item_index, local_point)
                    }
                };
                ResolvedCaret {
                    has_position: true,
                    node: Some(item.paintable),
                    boundary: if is_before {
                        FfiCaretBoundaryKind::BeforeNode
                    } else {
                        FfiCaretBoundaryKind::AfterNode
                    },
                    has_debug_rect: true,
                    debug_rect: item.caret_rect,
                    ..Default::default()
                }
            }
            HitTestItemKind::SvgPath | HitTestItemKind::ChromeWidget => Default::default(),
        }
    }
}

pub(crate) fn with_item_fragment<R>(
    arena: &impl PaintRead,
    item: &HitTestItem,
    f: impl FnOnce(&crate::painting::paintable_data::FragmentRecord) -> R,
) -> Option<R> {
    let fragment_index = item.text_fragment_index? as usize;
    arena
        .committed_side_data(item.paintable)
        .fragments()
        .get(fragment_index)
        .map(f)
}

pub(crate) fn fragment_layout_node_slot(arena: &impl PaintRead, item: &HitTestItem) -> Option<NodeSlotId> {
    with_item_fragment(arena, item, |fragment| fragment.layout_node)
}

/// The row an event at a paintable is dispatched to: the paintable's, or its nearest paint ancestor's
/// that is not anonymous.
pub(crate) fn event_dispatch_slot_for_paintable(arena: &impl PaintRead, slot: NodeSlotId) -> Option<NodeSlotId> {
    let mut current = arena.paintable_row_is_populated(slot).then_some(slot);
    while let Some(paintable) = current {
        if arena.node_flags_if_live(paintable) & NodeFlag::Anonymous as u32 == 0 {
            return Some(paintable);
        }
        current = crate::painting::paint_order::paint_parent(arena, paintable);
    }
    None
}

pub(crate) fn fragment_index_in_node_for_point(
    arena: &impl PaintRead,
    item: &HitTestItem,
    local_point: CssPixelPoint,
) -> usize {
    with_item_fragment(arena, item, |fragment| {
        let paintable_rows = arena;
        crate::painting::text_fragment::index_in_node_for_point(paintable_rows, fragment, local_point)
    })
    .unwrap_or(0)
}

fn fragment_caret_range_rect(
    arena: &impl PaintRead,
    fragment: &crate::painting::paintable_data::FragmentRecord,
    offset: usize,
) -> CssPixelRect {
    let paintable_rows = arena;
    let Some(offsets) = crate::painting::text_fragment::compute_selection_offsets(
        paintable_rows,
        fragment,
        SELECTION_STATE_START_AND_END,
        offset,
        offset,
    ) else {
        return CssPixelRect::default();
    };
    crate::painting::text_fragment::rect_for_selection_offsets(paintable_rows, fragment, offsets, || {
        crate::painting::text_fragment::first_available_font(paintable_rows, fragment)
    })
}
