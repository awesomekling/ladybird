/*
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/CascadedProperties.h>
#include <LibWeb/CSS/ComputedProperties.h>
#include <LibWeb/DOM/PseudoElement.h>
#include <LibWeb/Layout/Node.h>

namespace Web::DOM {

GC_DEFINE_ALLOCATOR(PseudoElement);
GC_DEFINE_ALLOCATOR(PseudoElementTreeNode);

RefPtr<CSS::ComputedProperties> PseudoElement::computed_properties() const { return m_computed_properties; }
void PseudoElement::set_computed_properties(RefPtr<CSS::ComputedProperties> value)
{
    m_computed_properties = move(value);
    if (m_computed_properties && !m_computed_properties->animated_property_data().values.is_empty())
        m_animated_property_data = make<CSS::AnimatedPropertyData>(m_computed_properties->animated_property_data());
    else
        m_animated_property_data = nullptr;
}

void PseudoElement::visit_edges(JS::Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);

    visitor.visit(m_layout_node);
    if (m_counters_set)
        m_counters_set->visit_edges(visitor);
}

Optional<CSS::CountersSet const&> PseudoElement::counters_set() const
{
    if (!m_counters_set)
        return {};
    return *m_counters_set;
}

CSS::CountersSet& PseudoElement::ensure_counters_set()
{
    if (!m_counters_set)
        m_counters_set = make<CSS::CountersSet>();
    return *m_counters_set;
}

void PseudoElement::set_counters_set(OwnPtr<CSS::CountersSet>&& counters_set)
{
    m_counters_set = move(counters_set);
}

}
