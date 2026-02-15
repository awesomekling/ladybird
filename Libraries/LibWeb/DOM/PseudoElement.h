/*
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/OwnPtr.h>
#include <LibGC/CellAllocator.h>
#include <LibJS/Heap/Cell.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/CustomPropertyData.h>
#include <LibWeb/CSS/PseudoClassBitmap.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/PixelUnits.h>
#include <LibWeb/TreeNode.h>

namespace Web::DOM {

class WEB_API PseudoElement : public JS::Cell {
    GC_CELL(PseudoElement, JS::Cell);
    GC_DECLARE_ALLOCATOR(PseudoElement);

    GC::Ptr<Layout::NodeWithStyle> layout_node() const { return m_layout_node; }
    void set_layout_node(GC::Ptr<Layout::NodeWithStyle> value) { m_layout_node = value; }

    CSS::ComputedValues const* computed_values() const;
    CSS::ComputedValues& ensure_computed_values();

    CSS::AnimatedPropertyData* animated_property_data() { return m_animated_property_data.ptr(); }
    CSS::AnimatedPropertyData const* animated_property_data() const { return m_animated_property_data.ptr(); }
    void set_animated_property_data(OwnPtr<CSS::AnimatedPropertyData>);

    RefPtr<CSS::CustomPropertyData const> custom_property_data() const { return m_custom_property_data; }
    void set_custom_property_data(RefPtr<CSS::CustomPropertyData const> value) { m_custom_property_data = move(value); }

    bool has_non_empty_counters_set() const { return m_counters_set; }
    Optional<CSS::CountersSet const&> counters_set() const;
    CSS::CountersSet& ensure_counters_set();
    void set_counters_set(OwnPtr<CSS::CountersSet>&&);

    CSSPixelPoint scroll_offset() const { return m_scroll_offset; }
    void set_scroll_offset(CSSPixelPoint value) { m_scroll_offset = value; }

    bool has_attempted_match_against_pseudo_class(CSS::PseudoClass pseudo_class) const { return m_attempted_pseudo_class_matches.get(pseudo_class); }
    void set_attempted_pseudo_class_matches(CSS::PseudoClassBitmap const& bitmap) { m_attempted_pseudo_class_matches = bitmap; }

    virtual void visit_edges(JS::Cell::Visitor&) override;

private:
    GC::Ptr<Layout::NodeWithStyle> m_layout_node;
    OwnPtr<CSS::ComputedValues> m_computed_values;
    OwnPtr<CSS::AnimatedPropertyData> m_animated_property_data;
    RefPtr<CSS::CustomPropertyData const> m_custom_property_data;
    OwnPtr<CSS::CountersSet> m_counters_set;
    CSSPixelPoint m_scroll_offset {};
    CSS::PseudoClassBitmap m_attempted_pseudo_class_matches;
};

// https://drafts.csswg.org/css-view-transitions/#pseudo-element-tree
class PseudoElementTreeNode
    : public PseudoElement
    , public TreeNode<PseudoElementTreeNode> {
    GC_CELL(PseudoElementTreeNode, PseudoElement);
    GC_DECLARE_ALLOCATOR(PseudoElementTreeNode);

protected:
    virtual void visit_edges(JS::Cell::Visitor& visitor) override
    {
        Base::visit_edges(visitor);
        TreeNode::visit_edges(visitor);
    }
};

}
