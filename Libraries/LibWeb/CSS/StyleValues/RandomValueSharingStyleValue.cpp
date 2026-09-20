/*
 * Copyright (c) 2025, Callum Law <callumlaw1709@outlook.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include "RandomValueSharingStyleValue.h"
#include <LibWeb/CSS/Serialize.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleValues/CalculatedStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/DOM/Document.h>

namespace Web::CSS {

ValueComparingNonnullRefPtr<StyleValue const> RandomValueSharingStyleValue::absolutized(ComputationContext const& computation_context) const
{
    // https://drafts.csswg.org/css-values-5/#random-caching
    // Each instance of a random function in styles has an associated random base value.
    // If the random function’s <random-value-sharing> is fixed <number>, the random base value is that number.
    if (fixed_value()) {
        auto const& absolutized_fixed_value = fixed_value()->absolutized(computation_context);

        if (fixed_value() == absolutized_fixed_value)
            return *this;

        return RandomValueSharingStyleValue::create_fixed(absolutized_fixed_value);
    }

    // Otherwise, the random base value is a pseudo-random real number in the range `[0, 1)` (greater than or equal to 0
    // and less than 1), generated from a uniform distribution, and influenced by the function’s random caching key.

    auto name = this->name().value();
    auto& element = computation_context.abstract_element->element();
    auto& style_engine = const_cast<StyleEngine&>(element.document().style_computer().style_engine());
    auto random_base_value = style_engine.ensure_random_base_value(element.style_node_id(), name.view(), element_shared());

    return RandomValueSharingStyleValue::create_fixed(NumberStyleValue::create(random_base_value));
}

double RandomValueSharingStyleValue::random_base_value() const
{
    return number_from_style_value(*fixed_value(), {});
}

}
