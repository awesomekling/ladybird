/*
 * Copyright (c) 2018-2020, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/HTML/HTMLBodyElement.h>
#include <LibWeb/HTML/HTMLHtmlElement.h>
#include <LibWeb/Painting/BoxSlot.h>

namespace Web::HTML {

GC_DEFINE_ALLOCATOR(HTMLHtmlElement);

HTMLHtmlElement::HTMLHtmlElement(DOM::Document& document, DOM::QualifiedName qualified_name)
    : HTMLElement(document, move(qualified_name))
{
}

HTMLHtmlElement::~HTMLHtmlElement() = default;

bool HTMLHtmlElement::should_use_body_background_properties() const
{
    // https://drafts.csswg.org/css-contain-2/#contain-property
    // Additionally, when any containments are active on either the HTML <html> or <body> elements, propagation of
    // properties from the <body> element to the initial containing block, the viewport, or the canvas background, is
    // disabled. Notably, this affects:
    // - 'background' and its longhands (see CSS Backgrounds 3 § 2.11.2 The Canvas Background and the HTML <body> Element)
    // NB: Called during rendering, reading style off the boxes.
    auto has_containment = [](Painting::BoxSlot const& box) {
        auto const& values = *box.style_group<CSS::ComputedValues::BoxValues>();
        return !CSS::Containment { values.size_containment, values.inline_size_containment, values.layout_containment, values.style_containment, values.paint_containment }.is_empty();
    };

    auto box = Painting::BoxSlot::bound_to(*this);
    if (!box || has_containment(box))
        return false;

    auto const* body_element = first_child_of_type<HTML::HTMLBodyElement>();
    if (!body_element)
        return false;
    auto body_box = Painting::BoxSlot::bound_to(*body_element);
    if (!body_box || has_containment(body_box))
        return false;

    auto const& background = *box.style_group<CSS::ComputedValues::BackgroundValues>();
    if (background.background_color_value() != Color::Transparent)
        return false;
    return !any_of(background.background_layers_value(), [](auto const& layer) { return layer.background_image != nullptr; });
}

}
