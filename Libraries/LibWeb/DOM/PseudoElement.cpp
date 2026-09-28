/*
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/Animations/KeyframeEffect.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/DOM/AbstractElement.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/PseudoElement.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxSlot.h>

namespace Web::DOM {

GC_DEFINE_ALLOCATOR(PseudoElement);
GC_DEFINE_ALLOCATOR(SyntheticPseudoElement);
GC_DEFINE_ALLOCATOR(SyntheticPseudoElementTreeNode);
GC_DEFINE_ALLOCATOR(ElementReferencePseudoElement);

SyntheticPseudoElement::SyntheticPseudoElement(CSS::PseudoElement type)
    : m_type(type)
{
}
SyntheticPseudoElement::SyntheticPseudoElement(CSS::PseudoElement type, GC::Ref<Element> originating_element)
    : m_type(type)
    , m_originating_element(originating_element)
{
}
SyntheticPseudoElement::~SyntheticPseudoElement() = default;

void SyntheticPseudoElement::visit_edges(JS::Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);

    visitor.visit(m_originating_element);
}

bool SyntheticPseudoElement::has_box() const
{
    return m_originating_element && Painting::BoxSlot::of_pseudo_element(*m_originating_element, m_type);
}

CSSPixelPoint SyntheticPseudoElement::scroll_offset() const
{
    if (!m_originating_element)
        return {};
    // The render side stores the offset, so a write still in the journal lands before the read.
    m_originating_element->document().drain_invalidation_journal();
    auto render_document = m_originating_element->document().render_document_id();
    if (render_document == 0)
        return {};
    return Layout::RustFFI::render_owner_pseudo_element_scroll_offset(render_document,
        m_originating_element->style_node_id().value(), encode_generated_for(m_type));
}

void SyntheticPseudoElement::set_scroll_offset(CSSPixelPoint value)
{
    VERIFY(m_originating_element);
    auto* arena = m_originating_element->document().layout_arena_handle();
    // Nothing has scrolled anything before a layout tree exists, so there is no offset to forget.
    if (!arena) {
        if (value.is_zero())
            return;
        arena = Layout::document_layout_arena(m_originating_element->document());
    }
    Layout::RustFFI::layout_arena_set_pseudo_element_scroll_offset(arena,
        m_originating_element->style_node_id().value(), encode_generated_for(m_type), value);
}

Node& SyntheticPseudoElement::root() const
{
    VERIFY(m_originating_element);
    return m_originating_element->root();
}

void SyntheticPseudoElement::update_animated_properties(Badge<Web::Animations::KeyframeEffect> const&, DOM::AbstractElement abstract_element, Web::Animations::KeyframeEffect& effect, Web::Animations::AnimationUpdateContext& context)
{
    if (!m_style_record)
        return;
    effect.update_computed_properties_for_style(context, abstract_element);
}

void SyntheticPseudoElement::replace_style_record(RefPtr<CSS::PublishedStyleRecord const> style_record)
{
    VERIFY(m_originating_element);
    if (style_record_identity() == (style_record ? style_record->identity() : CSS::StyleRecordID {}))
        return;
    m_style_record = move(style_record);
    if (auto box = Painting::BoxSlot::of_pseudo_element(*m_originating_element, m_type))
        Layout::set_style_record_of_box(box, m_style_record);
}

void SyntheticPseudoElement::set_computed_style(RefPtr<CSS::PublishedStyleRecord const> style_record)
{
    if (!style_record) {
        clear_computed_style();
        return;
    }
    replace_style_record(move(style_record));
}

void SyntheticPseudoElement::clear_computed_style(RefPtr<CSS::ComputedValues const> style_to_preserve_for_detachment)
{
    if (auto box = m_originating_element ? Painting::BoxSlot::of_pseudo_element(*m_originating_element, m_type) : Painting::BoxSlot {}) {
        if (style_to_preserve_for_detachment) {
            auto style_record = box.document().style_computer().intern_computed_style_inputs({ *m_originating_element, m_type }, *style_to_preserve_for_detachment);
            Layout::RustFFI::layout_arena_adopt_derived_node_style(box.arena(), box.slot(), style_record.value());
        } else {
            Layout::RustFFI::layout_arena_pin_bound_box_style_record_for_detachment(box.arena(), m_originating_element->style_node_id().value(), encode_generated_for(m_type));
        }
    }
    m_style_record = nullptr;
}

void SyntheticPseudoElement::refresh_computed_style(NonnullRefPtr<CSS::PublishedStyleRecord const> style_record)
{
    replace_style_record(move(style_record));
}

SyntheticPseudoElementTreeNode::SyntheticPseudoElementTreeNode(CSS::PseudoElement type)
    : SyntheticPseudoElement(type)
{
}
SyntheticPseudoElementTreeNode::SyntheticPseudoElementTreeNode(CSS::PseudoElement type, GC::Ref<Element> originating_element)
    : SyntheticPseudoElement(type, originating_element)
{
}
SyntheticPseudoElementTreeNode::~SyntheticPseudoElementTreeNode() = default;

void SyntheticPseudoElementTreeNode::visit_edges(JS::Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);
    TreeNode::visit_edges(visitor);
}

bool ElementReferencePseudoElement::has_box() const
{
    return !!Painting::BoxSlot::bound_to(*m_referenced_element);
}

Node& ElementReferencePseudoElement::root() const
{
    return m_referenced_element->root();
}

CSS::StyleRecordID ElementReferencePseudoElement::style_record_identity() const
{
    return m_referenced_element->style_record_identity({});
}

CSS::PublishedStyleRecord const* ElementReferencePseudoElement::published_style_record() const
{
    return m_referenced_element->published_style_record({});
}

void ElementReferencePseudoElement::update_animated_properties(Badge<Web::Animations::KeyframeEffect> const& badge, DOM::AbstractElement abstract_element, Web::Animations::KeyframeEffect& effect, Web::Animations::AnimationUpdateContext& context)
{
    m_referenced_element->update_animated_properties_for_abstract_element(badge, abstract_element, effect, context);
}

void ElementReferencePseudoElement::visit_edges(JS::Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_referenced_element);
}

}
