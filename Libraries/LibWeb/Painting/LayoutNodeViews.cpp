/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Layout/TextNode.h>
#include <LibWeb/Painting/LayoutNodeViews.h>

namespace Web::Painting {

static BoxSlot box_slot(Layout::Node const& node)
{
    return BoxSlot::of(node.document(), Layout::Node::slot_id(&node));
}

static BoxSlot box_slot(Layout::Row const& row)
{
    if (!row)
        return {};
    return BoxSlot::of(row.document(), row.slot());
}

Compositing::RustFFI::NodeSlotId committed_row_slot(Layout::Node const& node)
{
    return Layout::Node::slot_id(&node);
}

CSSPixelRect absolute_rect(Layout::Node const& node)
{
    return absolute_rect(box_slot(node));
}

CSSPixelRect absolute_padding_box_rect(Layout::Node const& node)
{
    return absolute_padding_box_rect(box_slot(node));
}

CSSPixelRect absolute_border_box_rect(Layout::Node const& node)
{
    return absolute_border_box_rect(box_slot(node));
}

CSSPixelPoint absolute_position(Layout::Node const& node)
{
    return absolute_position(box_slot(node));
}

CSSPixelSize content_size(Layout::Node const& node)
{
    return content_size(box_slot(node));
}

CSSPixels content_width(Layout::Node const& node)
{
    return content_width(box_slot(node));
}

CSSPixels content_height(Layout::Node const& node)
{
    return content_height(box_slot(node));
}

BoxModelMetrics box_model(Layout::Node const& node)
{
    return box_model(box_slot(node));
}

Optional<CSSPixelRect> scrollable_overflow_rect(Layout::Node const& node)
{
    return scrollable_overflow_rect(box_slot(node));
}

bool is_positioned(Layout::Node const& node)
{
    return is_positioned(box_slot(node));
}

CSSPixelRect transform_reference_box(Layout::Node const& node)
{
    return transform_reference_box(box_slot(node));
}

Compositing::SpatialNodeIndex own_scroll_node_index(Layout::Node const& node)
{
    return own_scroll_node_index(box_slot(node));
}

CSSPixelRect transform_rect_to_viewport(Layout::Node const& node, CSSPixelRect const& rect, Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform include_visual_viewport_transform)
{
    return transform_rect_to_viewport(box_slot(node), rect, include_visual_viewport_transform);
}

CSSPixelPoint inverse_transform_point(Layout::Node const& node, CSSPixelPoint position)
{
    return inverse_transform_point(box_slot(node), position);
}

CSSPixelPoint cumulative_scroll_compensation(Layout::Node const& node)
{
    return cumulative_scroll_compensation(box_slot(node));
}

CSS::RustStyleValueHandle used_value_for_grid_template(Layout::Node const& node, CSS::PropertyID property)
{
    return used_value_for_grid_template(box_slot(node), property);
}

bool is_navigable_container_viewport_paintable(Layout::Node const& node)
{
    return is_navigable_container_viewport_paintable(box_slot(node));
}

CSSPixelPoint box_type_agnostic_position(Layout::Node const& node)
{
    return box_type_agnostic_position(box_slot(node));
}

CSSPixelPoint transform_to_local_coordinates(Layout::Node const& node, CSSPixelPoint position)
{
    return transform_to_local_coordinates(box_slot(node), position);
}

bool is_visible(Layout::Node const& node)
{
    return is_visible(box_slot(node));
}

CSS::Display display(Layout::Node const& node)
{
    return display(box_slot(node));
}

CSSPixelRect caret_rect_for_child_offset(Layout::Node const& block, size_t offset)
{
    return caret_rect_for_child_offset(box_slot(block), offset);
}

bool has_committed_box(Layout::Row const& row)
{
    return has_committed_box(box_slot(row));
}

u64 committed_row_reset_version(Layout::NodeArena& arena, Compositing::RustFFI::NodeSlotId slot)
{
    return Layout::RustFFI::layout_arena_paintable_row_reset_version(arena.handle(), slot);
}

void set_needs_repaint(Layout::Row const& row, InvalidateDisplayList should_invalidate_display_list)
{
    set_needs_repaint(box_slot(row), should_invalidate_display_list);
}

void set_needs_repaint_in_subtree(Layout::Row const& row)
{
    set_needs_repaint_in_subtree(box_slot(row));
}

void invalidate_propagated_text_decoration_caches(Layout::Row const& row)
{
    invalidate_propagated_text_decoration_caches(box_slot(row));
}

void apply_paint_cache_invalidation(Layout::Row const& row, PaintCacheInvalidation invalidation)
{
    apply_paint_cache_invalidation(box_slot(row), invalidation);
}

void apply_repaint_damage(Layout::Row const& row, InvalidateDisplayList should_invalidate_display_list)
{
    apply_repaint_damage(box_slot(row), should_invalidate_display_list);
}

void apply_repaint_damage(Layout::TextNode const& node, InvalidateDisplayList should_invalidate_display_list)
{
    apply_text_repaint_damage(box_slot(node), should_invalidate_display_list);
}

void apply_subtree_repaint_damage(Layout::Row const& row)
{
    apply_subtree_repaint_damage(box_slot(row));
}

void repaint_after_style_change(Layout::Row const& row, CSS::RequiredInvalidationAfterStyleChange const& invalidation)
{
    repaint_after_style_change(box_slot(row), invalidation);
}

CSSPixelPoint scroll_offset(Layout::Node const& node)
{
    return scroll_offset(box_slot(node));
}

CSSPixelPoint minimum_scroll_offset(Layout::Node const& node)
{
    return minimum_scroll_offset(box_slot(node));
}

CSSPixelPoint maximum_scroll_offset(Layout::Node const& node)
{
    return maximum_scroll_offset(box_slot(node));
}

CSSPixelPoint clamp_scroll_offset(Layout::Node const& node, CSSPixelPoint offset)
{
    return clamp_scroll_offset(box_slot(node), offset);
}

CSSPixelRect scroll_snapport_rect(Layout::Node const& node)
{
    return scroll_snapport_rect(box_slot(node));
}

CSSPixelRect scroll_snapport_rect(Layout::Node const& node, CSSPixelRect scrollport)
{
    return scroll_snapport_rect(box_slot(node), scrollport);
}

WheelScrollableAxes wheel_scrollable_axes(Layout::Node const& node)
{
    return wheel_scrollable_axes(box_slot(node));
}

Optional<Compositing::AsyncScrollNodeStableID> async_scroll_node_stable_id(Layout::Node const& node)
{
    return async_scroll_node_stable_id(box_slot(node));
}

ScrollHandled set_scroll_offset(Layout::Node& node, CSSPixelPoint offset)
{
    return set_scroll_offset(box_slot(node), offset);
}

ScrollHandled set_scroll_offset_from_user_input(Layout::Node& node, CSSPixelPoint offset, ScrollKind scroll_kind)
{
    return set_scroll_offset_from_user_input(box_slot(node), offset, scroll_kind);
}

}
