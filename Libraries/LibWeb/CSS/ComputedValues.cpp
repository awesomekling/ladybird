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
    size_t index = to_underlying(property_id) - to_underlying(first_longhand_property_id);
    m_property_values[index] = move(value);
}

RefPtr<StyleValue const> ComputedValues::property_value(PropertyID property_id) const
{
    size_t index = to_underlying(property_id) - to_underlying(first_longhand_property_id);
    return m_property_values[index];
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

void ComputedValues::copy_all_from(ComputedValues const& other)
{
    m_inherited = other.m_inherited;
    m_noninherited = other.m_noninherited;
    m_property_values = other.m_property_values;
    m_property_important = other.m_property_important;
    m_property_inherited = other.m_property_inherited;
}

NonnullOwnPtr<ComputedValues> ComputedValues::clone_inherited_values() const
{
    auto clone = make<ComputedValues>();
    clone->m_inherited = m_inherited;
    return clone;
}

}
