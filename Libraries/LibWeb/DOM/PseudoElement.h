/*
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Badge.h>
#include <AK/OwnPtr.h>
#include <LibGC/CellAllocator.h>
#include <LibJS/Heap/Cell.h>
#include <LibWeb/CSS/PseudoElement.h>
#include <LibWeb/CSS/PublishedStyleRecord.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/TreeNode.h>
#include <LibWebCommon/PixelUnits.h>

namespace Web::Animations {

struct AnimationUpdateContext;
class KeyframeEffect;

}

namespace Web::DOM {

// How the layout rows generated for a pseudo-element name it beside its generator's identity: 0 names no pseudo-element.
constexpr u8 encode_generated_for(CSS::PseudoElement pseudo_element)
{
    static_assert(static_cast<u8>(CSS::PseudoElement::UnknownWebKit) < 0xff);
    return static_cast<u8>(pseudo_element) + 1;
}

class WEB_API PseudoElement : public JS::Cell {
    GC_CELL(PseudoElement, JS::Cell);
    GC_DECLARE_ALLOCATOR(PseudoElement);

public:
    virtual Layout::NodeWithStyle* unsafe_layout_node() const = 0;
    // Whether a layout tree build gave the pseudo-element a box, which the arena keeps bound to it.
    virtual bool has_box() const = 0;

    virtual Node& root() const = 0;

    virtual CSS::StyleRecordID style_record_identity() const = 0;
    virtual CSS::PublishedStyleRecord const* published_style_record() const = 0;
    virtual void update_animated_properties(Badge<Web::Animations::KeyframeEffect> const&, DOM::AbstractElement, Web::Animations::KeyframeEffect&, Web::Animations::AnimationUpdateContext&) = 0;
};

class WEB_API SyntheticPseudoElement : public PseudoElement {
    GC_CELL(SyntheticPseudoElement, PseudoElement);
    GC_DECLARE_ALLOCATOR(SyntheticPseudoElement);

public:
    explicit SyntheticPseudoElement(CSS::PseudoElement type);
    SyntheticPseudoElement(CSS::PseudoElement type, GC::Ref<Element> originating_element);
    virtual ~SyntheticPseudoElement() override;

    CSS::PseudoElement type() const { return m_type; }

    Layout::NodeWithStyle* unsafe_layout_node() const override;
    bool has_box() const override;
    // The pseudo-element's box stops being bound to it, and holds its style record until it is detached.
    void unbind_box();

    virtual Node& root() const override;

    virtual CSS::StyleRecordID style_record_identity() const override { return m_style_record ? m_style_record->identity() : CSS::StyleRecordID {}; }
    virtual CSS::PublishedStyleRecord const* published_style_record() const override { return m_style_record; }
    void update_animated_properties(Badge<Web::Animations::KeyframeEffect> const&, DOM::AbstractElement, Web::Animations::KeyframeEffect&, Web::Animations::AnimationUpdateContext&) override;
    void set_computed_style(RefPtr<CSS::PublishedStyleRecord const>);
    void clear_computed_style(RefPtr<CSS::ComputedValues const> style_to_preserve_for_detachment = nullptr);
    void refresh_computed_style(NonnullRefPtr<CSS::PublishedStyleRecord const>);

    // The offset lives in the layout node arena, keyed by the generator's identity and this
    // pseudo-element's kind, so a box bound to the pseudo-element reads it without asking here.
    CSSPixelPoint scroll_offset() const;
    void set_scroll_offset(CSSPixelPoint value);

    virtual void visit_edges(JS::Cell::Visitor&) override;

private:
    void replace_style_record(RefPtr<CSS::PublishedStyleRecord const>);

    // A pseudo-element has no identity of its own: its box is the row bound to its generator's
    // identity and its type in the layout node arena.
    CSS::PseudoElement m_type;
    GC::Ptr<Element> m_originating_element;
    // The authoritative StyleEngine record. C++ compatibility consumers borrow the record-owned
    // computed-values view rather than retaining one complete style per pseudo-element.
    RefPtr<CSS::PublishedStyleRecord const> m_style_record;
};

// https://drafts.csswg.org/css-view-transitions/#pseudo-element-tree
class SyntheticPseudoElementTreeNode
    : public SyntheticPseudoElement
    , public TreeNode<SyntheticPseudoElementTreeNode> {
    GC_CELL(SyntheticPseudoElementTreeNode, SyntheticPseudoElement);
    GC_DECLARE_ALLOCATOR(SyntheticPseudoElementTreeNode);

public:
    explicit SyntheticPseudoElementTreeNode(CSS::PseudoElement type);
    SyntheticPseudoElementTreeNode(CSS::PseudoElement type, GC::Ref<Element> originating_element);
    virtual ~SyntheticPseudoElementTreeNode() override;

protected:
    virtual void visit_edges(JS::Cell::Visitor& visitor) override;
};

class WEB_API ElementReferencePseudoElement : public PseudoElement {
    GC_CELL(ElementReferencePseudoElement, PseudoElement);
    GC_DECLARE_ALLOCATOR(ElementReferencePseudoElement);

    ElementReferencePseudoElement(GC::Ref<Element> referenced_element)
        : m_referenced_element(referenced_element)
    {
    }

    Layout::NodeWithStyle* unsafe_layout_node() const override;
    bool has_box() const override;

    virtual Node& root() const override;

    virtual CSS::StyleRecordID style_record_identity() const override;
    virtual CSS::PublishedStyleRecord const* published_style_record() const override;
    void update_animated_properties(Badge<Web::Animations::KeyframeEffect> const&, DOM::AbstractElement, Web::Animations::KeyframeEffect&, Web::Animations::AnimationUpdateContext&) override;

    GC::Ref<Element> const& referenced_element() const { return m_referenced_element; }

protected:
    virtual void visit_edges(JS::Cell::Visitor& visitor) override;

private:
    GC::Ref<Element> m_referenced_element;
};

}
