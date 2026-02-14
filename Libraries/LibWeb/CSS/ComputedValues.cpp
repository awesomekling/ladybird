/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/ComputedValues.h>

namespace Web::CSS {

ComputedValues::~ComputedValues() = default;

void ComputedValues::set_property_value(PropertyID property_id, NonnullRefPtr<StyleValue const> value)
{
    m_property_values.set(property_id, move(value));
}

RefPtr<StyleValue const> ComputedValues::property_value(PropertyID property_id) const
{
    auto it = m_property_values.find(property_id);
    if (it != m_property_values.end())
        return it->value;
    return nullptr;
}

NonnullOwnPtr<ComputedValues> ComputedValues::clone_inherited_values() const
{
    auto clone = make<ComputedValues>();
    clone->m_inherited = m_inherited;
    return clone;
}

}
