/*
 * Copyright (c) 2018-2020, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2021-2023, Sam Atkins <atkinssj@serenityos.org>
 * Copyright (c) 2022-2023, MacDue <macdue@dueutil.tech>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include "BackgroundSizeStyleValue.h"
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/PropertyNameAndID.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>

namespace Web::CSS {

BackgroundSizeStyleValue::BackgroundSizeStyleValue(ValueComparingNonnullRefPtr<StyleValue const> size_x, ValueComparingNonnullRefPtr<StyleValue const> size_y)
    : StyleValueWithDefaultOperators(Type::BackgroundSize)
    , m_properties { .size_x = move(size_x), .size_y = move(size_y) }
{
}

BackgroundSizeStyleValue::~BackgroundSizeStyleValue() = default;

void BackgroundSizeStyleValue::serialize(StringBuilder& builder, SerializationMode mode) const
{
    if (m_properties.size_x->has_auto() && m_properties.size_y->has_auto()) {
        builder.append("auto"sv);
        return;
    }
    m_properties.size_x->serialize(builder, mode);
    builder.append(' ');
    m_properties.size_y->serialize(builder, mode);
}

ValueComparingNonnullRefPtr<StyleValue const> BackgroundSizeStyleValue::absolutized(ComputationContext const& computation_context) const
{
    auto absolutize_or_resolve = [&](NonnullRefPtr<StyleValue const> const& value) -> ValueComparingNonnullRefPtr<StyleValue const> {
        if (!value->is_unresolved() || !computation_context.abstract_element.has_value())
            return value->absolutized(computation_context);

        auto resolved = Parser::Parser::resolve_unresolved_style_value(
            Parser::ParsingParams { computation_context.abstract_element->document() },
            *computation_context.abstract_element,
            PropertyNameAndID::from_id(PropertyID::Width),
            value->as_unresolved());
        return resolved->absolutized(computation_context);
    };

    auto absolutized_size_x = absolutize_or_resolve(m_properties.size_x);
    auto absolutized_size_y = absolutize_or_resolve(m_properties.size_y);

    if (absolutized_size_x == m_properties.size_x && absolutized_size_y == m_properties.size_y)
        return *this;

    return BackgroundSizeStyleValue::create(absolutized_size_x, absolutized_size_y);
}

}
