/*
 * Copyright (c) 2025, Jonathan Gamble <gamblej@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/CSSStyleProperties.h>
#include <LibWeb/CSS/PropertyID.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Page/ElementResizeAction.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/ChromeMetrics.h>

// https://drafts.csswg.org/css-ui#resize

namespace Web {

static Optional<CSSPixelSize> containing_block_padding_box_size(Painting::BoxSlot const& box)
{
    auto parent_box = box.containing_block();
    if (!parent_box)
        return {};
    if (Painting::has_committed_box(parent_box))
        return Painting::absolute_padding_box_rect(parent_box).size();
    return {};
}

ElementResizeAction::ElementResizeAction(GC::Ref<DOM::Element> element, CSSPixelPoint pointer_down_origin)
    : m_element(element)
    , m_pointer_down_origin(pointer_down_origin)
{
    auto identity = DOM::NodeIdentity::of(*element);
    if (Painting::has_committed_box(element->document(), identity))
        m_initial_border_box_size = Painting::absolute_border_box_rect(element->document(), identity).size();
}

void ElementResizeAction::handle_pointer_move(CSSPixelPoint pointer_position)
{
    auto element = m_element.ptr();
    if (!element || !element->is_connected())
        return;

    auto box = Painting::BoxSlot::bound_to(*element);
    if (!box || !Painting::has_committed_box(box))
        return;
    auto const* box_values = box.style_group<CSS::ComputedValues::BoxValues>();
    auto const* inherited_box_values = box.style_group<CSS::ComputedValues::InheritedBoxValues>();
    auto const* sizing_values = box.style_group<CSS::ComputedValues::SizingValues>();
    auto const* border_values = box.style_group<CSS::ComputedValues::BorderValues>();
    if (!box_values || !inherited_box_values || !sizing_values || !border_values)
        return;
    auto resize = static_cast<CSS::Resize>(box_values->resize);
    if (resize == CSS::Resize::None)
        return;

    auto writing_mode = static_cast<CSS::WritingMode>(inherited_box_values->writing_mode);
    auto direction = static_cast<CSS::Direction>(inherited_box_values->direction);
    bool horizontal_writing_mode = writing_mode == CSS::WritingMode::HorizontalTb;
    bool resize_x = resize == CSS::Resize::Both
        || resize == CSS::Resize::Horizontal
        || (resize == CSS::Resize::Inline && horizontal_writing_mode)
        || (resize == CSS::Resize::Block && !horizontal_writing_mode);

    bool resize_y = resize == CSS::Resize::Both
        || resize == CSS::Resize::Vertical
        || (resize == CSS::Resize::Inline && !horizontal_writing_mode)
        || (resize == CSS::Resize::Block && horizontal_writing_mode);

    CSSPixels dx = resize_x ? pointer_position.x() - m_pointer_down_origin.x() : 0;
    CSSPixels dy = resize_y ? pointer_position.y() - m_pointer_down_origin.y() : 0;
    if ((writing_mode == CSS::WritingMode::HorizontalTb && direction == CSS::Direction::Rtl)
        || writing_mode == CSS::WritingMode::VerticalRl
        || writing_mode == CSS::WritingMode::SidewaysRl) {
        dx = -dx;
    }
    CSSPixels css_width = max(ChromeMetrics::ZOOM_INVARIANT_RESIZE_GRIPPER_SIZE, m_initial_border_box_size.width() + dx);
    CSSPixels css_height = max(ChromeMetrics::ZOOM_INVARIANT_RESIZE_GRIPPER_SIZE, m_initial_border_box_size.height() + dy);

    auto reference_basis = containing_block_padding_box_size(box);

    if (reference_basis.has_value()) {
        if (auto const& min_width = CSS::Size::view(sizing_values->min_width); !min_width.is_auto()) {
            css_width = max(css_width, min_width.to_px(reference_basis->width()));
        }
        if (auto const& max_width = CSS::Size::view(sizing_values->max_width); !max_width.is_none()) {
            css_width = min(css_width, max_width.to_px(reference_basis->width()));
        }
        if (auto const& min_height = CSS::Size::view(sizing_values->min_height); !min_height.is_auto()) {
            css_height = max(css_height, min_height.to_px(reference_basis->height()));
        }
        if (auto const& max_height = CSS::Size::view(sizing_values->max_height); !max_height.is_none()) {
            css_height = min(css_height, max_height.to_px(reference_basis->height()));
        }
    }
    if (static_cast<CSS::BoxSizing>(box_values->box_sizing) == CSS::BoxSizing::ContentBox) {
        auto const metrics = Painting::box_model(box);
        css_width -= metrics.padding.left + metrics.padding.right + border_values->border_left_value().width + border_values->border_right_value().width;
        css_height -= metrics.padding.top + metrics.padding.bottom + border_values->border_top_value().width + border_values->border_bottom_value().width;
    }

    auto style = element->style();
    auto width_str = Utf16String::formatted("{:.2f}px", max(0.0, css_width.to_double()));
    auto height_str = Utf16String::formatted("{:.2f}px", max(0.0, css_height.to_double()));

    MUST(style->set_property(CSS::PropertyID::Width, width_str.utf16_view()));
    MUST(style->set_property(CSS::PropertyID::Height, height_str.utf16_view()));
}

}
