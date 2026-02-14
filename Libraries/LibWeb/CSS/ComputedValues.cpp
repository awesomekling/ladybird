/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/ComputedValues.h>

namespace Web::CSS {

ComputedValues::~ComputedValues() = default;

NonnullOwnPtr<ComputedValues> ComputedValues::clone_inherited_values() const
{
    auto clone = make<ComputedValues>();
    clone->m_inherited = m_inherited;
    return clone;
}

}
