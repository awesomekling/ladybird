/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/SvgPaintResources.h>
#include <LibWeb/SVG/SVGFilterElement.h>
#include <LibWeb/SVG/SVGGradientElement.h>
#include <LibWeb/SVG/SVGGraphicsElement.h>
#include <LibWeb/SVG/SVGPatternElement.h>

namespace Web::Painting {

static bool push_svg_filter_reference(void const* url_value, BoxSlot const& box, void* sink)
{
    if (!box)
        return false;
    auto filter_element = resolve_svg_filter_reference({ .pointer = url_value }, box.document());
    if (!filter_element)
        return false;
    filter_element->push_primitives(sink);
    return true;
}

// The pattern's content is laid out as a pattern box among the children of the box it paints.
static void push_svg_pattern_description(SVG::SVGPatternElement const& pattern, BoxSlot const& target, void* sink)
{
    auto content_element = pattern.pattern_content_element();
    if (!content_element)
        return;

    BoxSlot pattern_box;
    for (auto candidate = target.first_child(); candidate; candidate = candidate.next_sibling()) {
        if (candidate.kind() == Layout::RustFFI::NodeKind::SVGPatternBox && candidate.dom_node().ptr() == content_element.ptr()) {
            pattern_box = candidate;
            break;
        }
    }
    if (!pattern_box)
        return;

    Layout::RustFFI::FfiSvgPatternDescription description {};
    description.pattern_box = pattern_box.slot();
    description.units_are_object_bounding_box = pattern.pattern_units() == SVG::SVGUnits::ObjectBoundingBox;
    description.content_units_are_object_bounding_box = pattern.pattern_content_units() == SVG::SVGUnits::ObjectBoundingBox;
    description.has_view_box = pattern.view_box().has_value();
    description.x = Layout::to_ffi_number_percentage(pattern.pattern_x());
    description.y = Layout::to_ffi_number_percentage(pattern.pattern_y());
    description.width = Layout::to_ffi_number_percentage(pattern.pattern_width());
    description.height = Layout::to_ffi_number_percentage(pattern.pattern_height());
    description.pattern_transform_attribute = pattern.pattern_transform();
    auto const* transform_values = pattern.style_group<CSS::ComputedValues::TransformValues>();
    auto const* css_transform_entries = transform_values ? transform_values->resolved_transforms.pointer : nullptr;
    auto css_transform_count = transform_values ? transform_values->resolved_transforms.length : 0;
    Layout::RustFFI::layout_arena_svg_paint_resources_push_pattern(sink, &description, css_transform_entries, css_transform_count);
}

static void push_svg_paint_server_description(BoxSlot const& box, bool is_stroke, void* sink)
{
    auto const* graphics_element = as_if<SVG::SVGGraphicsElement>(box.dom_node().ptr());
    if (!graphics_element)
        return;
    auto const* svg_values = box.style_group<CSS::ComputedValues::InheritedSVGValues>();
    if (!svg_values)
        return;
    auto paint_server_element = graphics_element->paint_server_element(is_stroke ? svg_values->stroke_value() : svg_values->fill_value());
    if (auto const* gradient = as_if<SVG::SVGGradientElement>(paint_server_element.ptr()))
        gradient->push_paint_server_description(sink);
    else if (auto const* pattern = as_if<SVG::SVGPatternElement>(paint_server_element.ptr()))
        push_svg_pattern_description(*pattern, box, sink);
}

bool sync_svg_paint_resources(DOM::Document& document)
{
    return Layout::RustFFI::layout_arena_sync_svg_paint_resources(
        Layout::document_layout_arena(document),
        &document,
        [](void* context, Compositing::RustFFI::NodeSlotId slot, void const* url_value, void* sink) -> bool {
            return push_svg_filter_reference(url_value, BoxSlot::of(*static_cast<DOM::Document const*>(context), slot), sink);
        },
        [](void* context, Compositing::RustFFI::NodeSlotId slot, bool is_stroke, void* sink) {
            push_svg_paint_server_description(BoxSlot::of(*static_cast<DOM::Document const*>(context), slot), is_stroke, sink);
        });
}

}
