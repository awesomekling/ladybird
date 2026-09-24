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
#include <LibWeb/Layout/Node.h>

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

// A pseudo-element's box is the row bound to its generator's identity and its type. The generated
// content inside the box carries the same pair, so only this binding tells the box from its content.
// Node::layout_node() verifies that a row it hands out describes up-to-date layout. The
// pseudo-element path forwarded to the unchecked accessor instead, so nothing ever checked it.
// Report a stale read rather than assert on one, until we know whether any caller performs one.
Layout::NodeWithStyle* SyntheticPseudoElement::layout_node() const
{
    auto* layout_node = unsafe_layout_node();
    if (layout_node && !m_originating_element->document().layout_is_up_to_date())
        dbgln("FIXME: SyntheticPseudoElement::layout_node() read a layout row while layout was stale");
    return layout_node;
}

Layout::NodeWithStyle* SyntheticPseudoElement::unsafe_layout_node() const
{
    if (!m_originating_element)
        return nullptr;
    auto* arena = m_originating_element->document().layout_node_arena_if_created();
    if (!arena)
        return nullptr;
    return static_cast<Layout::NodeWithStyle*>(Layout::RustFFI::layout_arena_bound_pseudo_element_shell(arena->handle(), m_originating_element->style_node_id().value(), Layout::Node::encode_generated_for(m_type)));
}

CSSPixelPoint SyntheticPseudoElement::scroll_offset() const
{
    if (!m_originating_element)
        return {};
    // The render side stores the offset, so a write still in the journal lands before the read.
    m_originating_element->document().drain_invalidation_journal();
    auto* arena = m_originating_element->document().layout_node_arena_if_created();
    if (!arena)
        return {};
    return Layout::RustFFI::layout_arena_pseudo_element_scroll_offset(arena->handle(),
        m_originating_element->style_node_id().value(), Layout::Node::encode_generated_for(m_type));
}

void SyntheticPseudoElement::set_scroll_offset(CSSPixelPoint value)
{
    VERIFY(m_originating_element);
    auto* arena = m_originating_element->document().layout_node_arena_if_created();
    // Nothing has scrolled anything before a layout tree exists, so there is no offset to forget.
    if (!arena) {
        if (value.is_zero())
            return;
        arena = &m_originating_element->document().layout_node_arena();
    }
    Layout::RustFFI::layout_arena_set_pseudo_element_scroll_offset(arena->handle(),
        m_originating_element->style_node_id().value(), Layout::Node::encode_generated_for(m_type), value);
}

void SyntheticPseudoElement::set_layout_node(Layout::NodeWithStyle* value)
{
    auto* bound_row = unsafe_layout_node();
    if (bound_row && bound_row != value) {
        bound_row->pin_style_record_for_detachment();
        Layout::RustFFI::layout_arena_set_node_flag(bound_row->arena_handle(), Layout::Node::slot_id(bound_row), Layout::RustFFI::NodeFlag::IsPseudoElementPrincipalBox, false);
        Layout::RustFFI::layout_arena_unbind_row(bound_row->arena_handle(), Layout::Node::slot_id(bound_row));
    }
    // The box becomes the pseudo-element's box here, which is when it starts holding its scroll offset.
    if (value) {
        Layout::RustFFI::layout_arena_set_node_flag(value->arena_handle(), Layout::Node::slot_id(value), Layout::RustFFI::NodeFlag::IsPseudoElementPrincipalBox, true);
        Layout::RustFFI::layout_arena_bind_row(value->arena_handle(), Layout::Node::slot_id(value));
        value->publish_scroll_offset();
    }
}

Node& SyntheticPseudoElement::root() const
{
    VERIFY(m_originating_element);
    return m_originating_element->root();
}

void SyntheticPseudoElement::update_animated_properties(Badge<Web::Animations::KeyframeEffect> const&, DOM::AbstractElement abstract_element, Web::Animations::KeyframeEffect& effect, Web::Animations::AnimationUpdateContext& context)
{
    if (!m_style_record_identity)
        return;
    effect.update_computed_properties_for_style(context, abstract_element);
}

void SyntheticPseudoElement::replace_style_record(CSS::StyleRecordID style_record_identity)
{
    VERIFY(m_originating_element);
    auto old_style_record_identity = m_style_record_identity;
    if (old_style_record_identity == style_record_identity)
        return;
    m_style_record_identity = style_record_identity;
    if (auto* layout_node = unsafe_layout_node())
        layout_node->set_style_record_identity(style_record_identity);
}

void SyntheticPseudoElement::set_computed_style(CSS::StyleRecordID style_record_identity)
{
    if (!style_record_identity) {
        clear_computed_style();
        return;
    }
    replace_style_record(style_record_identity);
}

void SyntheticPseudoElement::clear_computed_style(RefPtr<CSS::ComputedValues const> style_to_preserve_for_detachment)
{
    if (auto* layout_node = unsafe_layout_node()) {
        if (style_to_preserve_for_detachment)
            layout_node->set_computed_values(style_to_preserve_for_detachment.release_nonnull());
        else
            layout_node->pin_style_record_for_detachment();
    }
    m_style_record_identity = 0;
}

void SyntheticPseudoElement::refresh_computed_style(CSS::StyleRecordID style_record_identity)
{
    replace_style_record(style_record_identity);
    VERIFY(m_style_record_identity);
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

Layout::NodeWithStyle* ElementReferencePseudoElement::layout_node() const
{
    return m_referenced_element->layout_node();
}

Layout::NodeWithStyle* ElementReferencePseudoElement::unsafe_layout_node() const
{
    return m_referenced_element->unsafe_layout_node();
}

Node& ElementReferencePseudoElement::root() const
{
    return m_referenced_element->root();
}

CSS::StyleRecordID ElementReferencePseudoElement::style_record_identity() const
{
    return m_referenced_element->style_record_identity({});
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
