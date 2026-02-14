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

bool ComputedValues::is_property_important(PropertyID property_id) const
{
    size_t n = to_underlying(property_id) - to_underlying(first_longhand_property_id);
    return m_property_important[n / 8] & (1 << (n % 8));
}

void ComputedValues::set_property_important(PropertyID property_id, bool important)
{
    size_t n = to_underlying(property_id) - to_underlying(first_longhand_property_id);
    if (important)
        m_property_important[n / 8] |= (1 << (n % 8));
    else
        m_property_important[n / 8] &= ~(1 << (n % 8));
}

bool ComputedValues::is_property_inherited(PropertyID property_id) const
{
    size_t n = to_underlying(property_id) - to_underlying(first_longhand_property_id);
    return m_property_inherited[n / 8] & (1 << (n % 8));
}

void ComputedValues::set_property_inherited(PropertyID property_id, bool inherited)
{
    size_t n = to_underlying(property_id) - to_underlying(first_longhand_property_id);
    if (inherited)
        m_property_inherited[n / 8] |= (1 << (n % 8));
    else
        m_property_inherited[n / 8] &= ~(1 << (n % 8));
}

NonnullOwnPtr<ComputedValues> ComputedValues::clone_inherited_values() const
{
    auto clone = make<ComputedValues>();
    clone->m_inherited = m_inherited;
    return clone;
}

}
