/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/ComputedProperties.h>
#include <LibWeb/CSS/ComputedValues.h>

namespace Web::CSS {

ComputedValues::~ComputedValues() = default;

NonnullOwnPtr<ComputedValues> ComputedValues::clone_inherited_values() const
{
    auto clone = make<ComputedValues>();
    clone->m_inherited = m_inherited;
    return clone;
}

void MutableComputedValues::set_source_computed_properties(ComputedProperties const& properties)
{
    m_source_computed_properties = properties;
}

}
