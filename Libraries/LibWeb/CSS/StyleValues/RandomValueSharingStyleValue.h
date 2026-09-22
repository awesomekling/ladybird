/*
 * Copyright (c) 2025, Callum Law <callumlaw1709@outlook.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibWeb/CSS/StyleValues/StyleValue.h>

namespace Web::CSS {

class RandomValueSharingStyleValue : public StyleValueWithDefaultOperators<RandomValueSharingStyleValue> {
public:
    virtual ~RandomValueSharingStyleValue() override = default;

    ValueComparingNonnullRefPtr<StyleValue const> absolutized(ComputationContext const&) const;

    double random_base_value() const;

private:
    friend class StyleValue;

    explicit RandomValueSharingStyleValue(StyleValueFFI::StyleValueData const* data)
        : StyleValueWithDefaultOperators(Type::RandomValueSharing, data)
    {
    }

    ValueComparingRefPtr<StyleValue const> fixed_value() const { return wrap_rust_child_or_null(m_value->random_value_sharing.fixed_value); }

};

}
