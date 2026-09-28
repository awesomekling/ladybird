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
#include <LibWeb/Painting/BoxModelMetrics.h>
#include <LibWeb/Painting/BoxSlot.h>
#include <LibWeb/Painting/PaintableTypes.h>

namespace Web::Painting {

WEB_API void set_paint_viewport_scrollbars(bool enabled);
bool should_paint_viewport_scrollbars();

// One url() reference of a filter list, resolved against the SVG <filter> element it names.
WEB_API GC::Ptr<SVG::SVGFilterElement> resolve_svg_filter_reference(CSS::ComputedValuesFFI::ComputedStyleValueHandle const& url_value, DOM::Document const&);

Compositing::RustFFI::NodeSlotId viewport_row_slot(DOM::Document const&);
// The row the identity's node is bound to in its document, or an invalid slot.
WEB_API Compositing::RustFFI::NodeSlotId committed_row_slot(DOM::Document const&, DOM::NodeIdentity);
// The kind of the row the identity's node is bound to in its document, if it is bound to one.
WEB_API Optional<Layout::RustFFI::NodeKind> bound_row_kind(DOM::Document const&, DOM::NodeIdentity);

WEB_API u64 committed_row_reset_version(DOM::Document const&, Compositing::RustFFI::NodeSlotId);
// The DOM node a committed slot (e.g. a hit's box) was built for; nothing for an anonymous box or a slot no longer live.
WEB_API DOM::NodeIdentity dom_node_identity_of_committed_slot(DOM::Document const&, Compositing::RustFFI::NodeSlotId);

// The box a committed slot names, if layout committed it and its row is still live.
WEB_API BoxSlot committed_box(DOM::Document const&, Compositing::RustFFI::NodeSlotId);

WEB_API bool has_committed_box(BoxSlot const&);
WEB_API CSSPixelRect absolute_rect(BoxSlot const&);
WEB_API CSSPixelRect absolute_padding_box_rect(BoxSlot const&);
WEB_API CSSPixelRect absolute_border_box_rect(BoxSlot const&);
WEB_API CSSPixelPoint absolute_position(BoxSlot const&);
WEB_API CSSPixelSize content_size(BoxSlot const&);
WEB_API CSSPixels content_width(BoxSlot const&);
WEB_API CSSPixels content_height(BoxSlot const&);
WEB_API CSSPixels border_box_width(BoxSlot const&);
WEB_API CSSPixels border_box_height(BoxSlot const&);
WEB_API BoxModelMetrics box_model(BoxSlot const&);
WEB_API CSSPixelRect transform_reference_box(BoxSlot const&);
WEB_API Optional<CSSPixelRect> scrollable_overflow_rect(BoxSlot const&);
WEB_API bool has_scrollable_overflow(BoxSlot const&);

WEB_API bool is_visible(BoxSlot const&);
WEB_API bool visible_for_hit_testing(BoxSlot const&);
WEB_API bool has_stacking_context(BoxSlot const&);
WEB_API CSS::Display display(BoxSlot const&);
WEB_API bool is_positioned(BoxSlot const&);
WEB_API bool is_navigable_container_viewport_paintable(BoxSlot const&);
WEB_API bool is_viewport_paintable(BoxSlot const&);
WEB_API bool is_paintable_with_lines(BoxSlot const&);
WEB_API bool is_inline_paintable(BoxSlot const&);
WEB_API bool is_svg_svg_paintable(BoxSlot const&);

WEB_API CSSPixelRect transform_rect_to_viewport(BoxSlot const&, CSSPixelRect const&, Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform = Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform::Yes);
WEB_API Optional<CSSPixelPoint> transform_point_to_local(BoxSlot const&, CSSPixelPoint);
WEB_API CSSPixelPoint inverse_transform_point(BoxSlot const&, CSSPixelPoint);
WEB_API CSSPixelPoint transform_to_local_coordinates(BoxSlot const&, CSSPixelPoint);

WEB_API bool has_accumulated_visual_context(BoxSlot const&);
WEB_API Compositing::ContextRef accumulated_visual_context(BoxSlot const&);
WEB_API Compositing::ContextRef accumulated_visual_context_for_descendants(BoxSlot const&);
WEB_API Compositing::SpatialNodeIndex enclosing_scroll_node_index(BoxSlot const&);
WEB_API Compositing::SpatialNodeIndex own_scroll_node_index(BoxSlot const&);

WEB_API Gfx::Path const* committed_svg_path(BoxSlot const&);
WEB_API CSSPixelSize svg_viewport_size(BoxSlot const&);
WEB_API Optional<Gfx::AffineTransform> svg_viewport_transform(BoxSlot const&);
WEB_API CSS::RustStyleValueHandle used_value_for_grid_template(BoxSlot const&, CSS::PropertyID);
WEB_API Optional<String> grid_layout_json(BoxSlot const&, UniqueNodeID);
WEB_API Optional<String> flex_layout_json(BoxSlot const&, UniqueNodeID);

WEB_API CSSPixelPoint box_type_agnostic_position(BoxSlot const&);
WEB_API CSSPixelRect caret_rect_for_child_offset(BoxSlot const&, size_t offset);
// The text a text box renders, its whitespace collapsed or as it is in the text; nothing for any other box.
WEB_API Utf16String rendered_text(BoxSlot const& text_box, bool collapse_whitespace);
WEB_API Vector<CSSPixelRect> client_rects(BoxSlot const&, Layout::RustFFI::FfiRectToViewportTransform const&);
WEB_API CSSPixelRect bounding_client_rect(BoxSlot const&, Layout::RustFFI::FfiRectToViewportTransform const&);
WEB_API CSSPixelPoint cumulative_scroll_compensation(BoxSlot const&);

WEB_API Optional<CSS::BorderData> outline_data(DOM::Element const&, CSS::ComputedValues const&);

// Per-document paint facts the recording inputs carry, resolved once per recording.
WEB_API Layout::RustFFI::FfiCaretPaint resolve_document_caret_paint(DOM::Document&);
WEB_API Layout::RustFFI::FfiFocusedTextControlSelection resolve_focused_text_control_selection(DOM::Document const&);
WEB_API Layout::RustFFI::FfiFocusedAreaOutline resolve_focused_area_outline(DOM::Document const&, Vector<u8>& path_bytes);
WEB_API void push_selection_pseudo_style(DOM::Element const&);

WEB_API void set_needs_repaint(BoxSlot const&, InvalidateDisplayList = InvalidateDisplayList::PaintCommandsAndHitTestList);
WEB_API void set_needs_repaint_in_subtree(BoxSlot const&);
WEB_API void set_needs_repaint(DOM::Document&, DOM::NodeIdentity, InvalidateDisplayList = InvalidateDisplayList::PaintCommandsAndHitTestList);
WEB_API void set_needs_repaint_in_subtree(DOM::Document&, DOM::NodeIdentity);

enum class PaintCacheInvalidation : u8 {
    PaintAndHitTest,
    PropagatedTextDecorations,
};

// Journals a paint cache invalidation for the box of the node the identity names.
WEB_API void invalidate_paint_cache(DOM::Document const&, DOM::NodeIdentity);
WEB_API void invalidate_propagated_text_decoration_caches(BoxSlot const&);
WEB_API void apply_paint_cache_invalidation(BoxSlot const&, PaintCacheInvalidation);
WEB_API void apply_repaint_damage(BoxSlot const&, InvalidateDisplayList);
// A text row paints through its containing block and its nearest self-painting inline.
WEB_API void apply_text_repaint_damage(BoxSlot const&, InvalidateDisplayList);
WEB_API void apply_subtree_repaint_damage(BoxSlot const&);
WEB_API void repaint_after_style_change(BoxSlot const&, CSS::RequiredInvalidationAfterStyleChange const&);
// The render owner repainted the layout nodes of rows it applied a style batch to: the document's navigable paints again.
WEB_API void repaint_document_after_owner_style_change(DOM::Document&, InvalidateDisplayList);

WEB_API Layout::RustFFI::FfiRectToViewportTransform identity_rect_to_viewport_transform();
WEB_API Layout::RustFFI::FfiRectToViewportTransform rect_to_viewport_transform(DOM::Document const&, Compositing::AccumulatedVisualContextTree const&);

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
