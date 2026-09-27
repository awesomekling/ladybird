/*
 * Copyright (c) 2026, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibCompositing/DisplayList/AccumulatedVisualContext.h>
#include <LibGC/Ptr.h>
#include <LibGfx/AffineTransform.h>
#include <LibGfx/Forward.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/InvalidateDisplayList.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Painting/BoxModelMetrics.h>
#include <LibWeb/Painting/PaintableTypes.h>

namespace Web::Painting {

WEB_API void set_paint_viewport_scrollbars(bool enabled);
bool should_paint_viewport_scrollbars();

// One url() reference of a filter list, resolved against the SVG <filter> element it names.
WEB_API GC::Ptr<SVG::SVGFilterElement> resolve_svg_filter_reference(CSS::ComputedValuesFFI::ComputedStyleValueHandle const& url_value, DOM::Document const&);

Compositing::RustFFI::NodeSlotId committed_row_slot(Layout::Node const&);
Compositing::RustFFI::NodeSlotId viewport_row_slot(DOM::Document const&);
// The row the identity's node is bound to in its document, or an invalid slot.
WEB_API Compositing::RustFFI::NodeSlotId committed_row_slot(DOM::Document const&, DOM::NodeIdentity);
// The kind of the row the identity's node is bound to in its document, if it is bound to one.
WEB_API Optional<Layout::RustFFI::NodeKind> bound_row_kind(DOM::Document const&, DOM::NodeIdentity);

WEB_API bool has_committed_box(Layout::Row const&);
WEB_API Layout::Node* layout_node_for_committed_slot(Layout::NodeArena&, Compositing::RustFFI::NodeSlotId);
WEB_API u64 committed_row_reset_version(Layout::NodeArena&, Compositing::RustFFI::NodeSlotId);
WEB_API u64 committed_row_reset_version(DOM::Document const&, Compositing::RustFFI::NodeSlotId);
// The DOM node a committed slot (e.g. a hit's box) was built for; nothing for an anonymous box or a slot no longer live.
WEB_API DOM::NodeIdentity dom_node_identity_of_committed_slot(DOM::Document const&, Compositing::RustFFI::NodeSlotId);

WEB_API CSSPixelRect absolute_rect(Layout::Node const&);
WEB_API CSSPixelRect absolute_padding_box_rect(Layout::Node const&);
WEB_API CSSPixelRect absolute_border_box_rect(Layout::Node const&);
WEB_API CSSPixelPoint absolute_position(Layout::Node const&);
WEB_API CSSPixelSize content_size(Layout::Node const&);
WEB_API CSSPixels content_width(Layout::Node const&);
WEB_API CSSPixels content_height(Layout::Node const&);
WEB_API CSSPixels border_box_width(Layout::Node const&);
WEB_API CSSPixels border_box_height(Layout::Node const&);
WEB_API BoxModelMetrics box_model(Layout::Node const&);
WEB_API Optional<CSS::BorderData> outline_data(DOM::Element const&, CSS::ComputedValues const&);
WEB_API CSSPixelRect transform_reference_box(Layout::Node const&);
WEB_API Optional<CSSPixelRect> scrollable_overflow_rect(Layout::Node const&);
WEB_API bool has_scrollable_overflow(Layout::Node const&);

WEB_API bool is_visible(Layout::Node const&);
WEB_API bool visible_for_hit_testing(Layout::Node const&);
WEB_API bool has_stacking_context(Layout::Node const&);
WEB_API CSS::Display display(Layout::Node const&);
WEB_API bool is_positioned(Layout::Node const&);
WEB_API CSS::StyleRecordID style_record_identity(Layout::Node const&);
WEB_API bool is_navigable_container_viewport_paintable(Layout::Node const&);
WEB_API bool is_viewport_paintable(Layout::Node const&);
WEB_API bool is_paintable_with_lines(Layout::Node const&);
WEB_API bool is_svg_svg_paintable(Layout::Node const&);

WEB_API CSSPixelRect transform_rect_to_viewport(Layout::Node const&, CSSPixelRect const&, Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform = Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform::Yes);
WEB_API CSSPixelPoint inverse_transform_point(Layout::Node const&, CSSPixelPoint);
WEB_API CSSPixelPoint transform_to_local_coordinates(Layout::Node const&, CSSPixelPoint);

WEB_API bool has_accumulated_visual_context(Layout::Node const&);
WEB_API Compositing::ContextRef accumulated_visual_context(Layout::Node const&);
WEB_API Compositing::ContextRef accumulated_visual_context_for_descendants(Layout::Node const&);
WEB_API Compositing::SpatialNodeIndex own_scroll_node_index(Layout::Node const&);

WEB_API Gfx::Path const* committed_svg_path(Layout::Node const&);
WEB_API CSSPixelSize svg_viewport_size(Layout::Node const&);
WEB_API Optional<Gfx::AffineTransform> svg_viewport_transform(Layout::Node const&);
WEB_API CSS::RustStyleValueHandle used_value_for_grid_template(Layout::Node const&, CSS::PropertyID);
WEB_API Optional<String> grid_layout_json(Layout::Node const&, UniqueNodeID);
WEB_API Optional<String> flex_layout_json(Layout::Node const&, UniqueNodeID);

WEB_API CSSPixelPoint box_type_agnostic_position(Layout::Node const&);
WEB_API CSSPixelRect caret_rect_for_child_offset(Layout::Node const&, size_t offset);

// Per-document paint facts the recording inputs carry, resolved once per recording.
WEB_API Layout::RustFFI::FfiCaretPaint resolve_document_caret_paint(DOM::Document&);
WEB_API Layout::RustFFI::FfiFocusedTextControlSelection resolve_focused_text_control_selection(DOM::Document const&);
WEB_API Layout::RustFFI::FfiFocusedAreaOutline resolve_focused_area_outline(DOM::Document const&, Vector<u8>& path_bytes);
WEB_API void push_selection_pseudo_style(DOM::Element const&);

WEB_API void set_needs_repaint(Layout::Row const&, InvalidateDisplayList = InvalidateDisplayList::PaintCommandsAndHitTestList);
WEB_API void set_needs_repaint_in_subtree(Layout::Row const&);
WEB_API void set_needs_repaint(DOM::Document&, DOM::NodeIdentity, InvalidateDisplayList = InvalidateDisplayList::PaintCommandsAndHitTestList);
WEB_API void set_needs_repaint_in_subtree(DOM::Document&, DOM::NodeIdentity);

enum class PaintCacheInvalidation : u8 {
    PaintAndHitTest,
    PropagatedTextDecorations,
};

// Journals a paint cache invalidation for the box of the node the identity names.
WEB_API void invalidate_paint_cache(DOM::Document const&, DOM::NodeIdentity);
WEB_API void invalidate_propagated_text_decoration_caches(Layout::Row const&);
WEB_API void apply_paint_cache_invalidation(Layout::Row const&, PaintCacheInvalidation);
WEB_API void apply_repaint_damage(Layout::Row const&, InvalidateDisplayList);
WEB_API void apply_repaint_damage(Layout::TextNode const&, InvalidateDisplayList);
WEB_API void apply_subtree_repaint_damage(Layout::Row const&);
WEB_API void repaint_after_style_change(Layout::Row const&, CSS::RequiredInvalidationAfterStyleChange const&);
// The render owner repainted the layout nodes of rows it applied a style batch to: the document's navigable paints again.
WEB_API void repaint_document_after_owner_style_change(DOM::Document&, InvalidateDisplayList);

WEB_API Layout::RustFFI::FfiRectToViewportTransform identity_rect_to_viewport_transform();
WEB_API Layout::RustFFI::FfiRectToViewportTransform rect_to_viewport_transform(DOM::Document const&, Compositing::AccumulatedVisualContextTree const&);
WEB_API Vector<CSSPixelRect> client_rects(Layout::Node const&, Layout::RustFFI::FfiRectToViewportTransform const&);
WEB_API CSSPixelRect bounding_client_rect(Layout::Node const&, Layout::RustFFI::FfiRectToViewportTransform const&);

WEB_API CSSPixelPoint cumulative_scroll_compensation(Layout::Node const&);

// The same views, keyed on the DOM node the box belongs to rather than on a layout node.
WEB_API bool has_committed_box(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelRect absolute_rect(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelRect absolute_padding_box_rect(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelRect absolute_border_box_rect(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelPoint absolute_position(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelSize content_size(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixels content_width(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixels content_height(DOM::Document const&, DOM::NodeIdentity);
WEB_API BoxModelMetrics box_model(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixels border_box_width(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixels border_box_height(DOM::Document const&, DOM::NodeIdentity);
WEB_API bool has_scrollable_overflow(DOM::Document const&, DOM::NodeIdentity);
WEB_API Optional<CSSPixelRect> scrollable_overflow_rect(DOM::Document const&, DOM::NodeIdentity);
WEB_API bool is_positioned(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelSize svg_viewport_size(DOM::Document const&, DOM::NodeIdentity);
WEB_API Optional<Gfx::AffineTransform> svg_viewport_transform(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelRect transform_reference_box(DOM::Document const&, DOM::NodeIdentity);
WEB_API Vector<CSSPixelRect> client_rects(DOM::Document const&, DOM::NodeIdentity, Layout::RustFFI::FfiRectToViewportTransform const&);
WEB_API CSSPixelRect bounding_client_rect(DOM::Document const&, DOM::NodeIdentity, Layout::RustFFI::FfiRectToViewportTransform const&);
WEB_API bool has_stacking_context(DOM::Document const&, DOM::NodeIdentity);
WEB_API bool has_accumulated_visual_context(DOM::Document const&, DOM::NodeIdentity);
WEB_API Compositing::ContextRef accumulated_visual_context(DOM::Document const&, DOM::NodeIdentity);
WEB_API Compositing::ContextRef accumulated_visual_context_for_descendants(DOM::Document const&, DOM::NodeIdentity);
WEB_API Compositing::SpatialNodeIndex enclosing_scroll_node_index(DOM::Document const&, DOM::NodeIdentity);
WEB_API Compositing::SpatialNodeIndex own_scroll_node_index(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelRect transform_rect_to_viewport(DOM::Document const&, DOM::NodeIdentity, CSSPixelRect const&, Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform = Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform::Yes);
WEB_API Optional<CSSPixelPoint> transform_point_to_local(DOM::Document const&, DOM::NodeIdentity, CSSPixelPoint);
WEB_API CSSPixelPoint inverse_transform_point(DOM::Document const&, DOM::NodeIdentity, CSSPixelPoint);
WEB_API CSSPixelPoint cumulative_scroll_compensation(DOM::Document const&, DOM::NodeIdentity);
WEB_API Gfx::Path const* committed_svg_path(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSS::RustStyleValueHandle used_value_for_grid_template(DOM::Document const&, DOM::NodeIdentity, CSS::PropertyID);
WEB_API bool is_navigable_container_viewport_paintable(DOM::Document const&, DOM::NodeIdentity);
WEB_API bool is_viewport_paintable(DOM::Document const&, DOM::NodeIdentity);
WEB_API bool is_svg_svg_paintable(DOM::Document const&, DOM::NodeIdentity);
// Read from the element's published style, as its box holds the same record.
WEB_API bool is_visible(DOM::Element const&);
WEB_API bool visible_for_hit_testing(DOM::Element const&);
WEB_API CSS::Display display(DOM::Element const&);
WEB_API CSSPixelPoint box_type_agnostic_position(DOM::Document const&, DOM::NodeIdentity);
WEB_API CSSPixelPoint transform_to_local_coordinates(DOM::Document const&, DOM::NodeIdentity, CSSPixelPoint);
WEB_API Optional<String> grid_layout_json(DOM::Document const&, DOM::NodeIdentity, UniqueNodeID);
WEB_API Optional<String> flex_layout_json(DOM::Document const&, DOM::NodeIdentity, UniqueNodeID);
WEB_API bool is_paintable_with_lines(DOM::Document const&, DOM::NodeIdentity);
WEB_API bool is_inline_paintable(DOM::Document const&, DOM::NodeIdentity);

}
