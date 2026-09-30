/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Optional.h>
#include <AK/String.h>
#include <LibCompositing/RustFFI.h>
#include <LibGC/Ptr.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/PseudoElement.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Layout/TreeBuilderRustFFI.h>

namespace Web::Painting {

// A box of a document's layout, named by the slot of its row rather than pointed at: what C++ holds of a box. Every
// read goes to the document's layout rows by the slot, and a slot whose row is no longer live reads as no box.
class WEB_API BoxSlot {
public:
    BoxSlot() = default;

    // The row `slot` names in the document's layout, if it is live.
    static BoxSlot of(DOM::Document const&, Compositing::RustFFI::NodeSlotId);
    // The row the identity's node, or its pseudo-element, is bound to.
    static BoxSlot bound_to(DOM::Document const&, DOM::NodeIdentity, Optional<CSS::PseudoElement> = {});
    static BoxSlot bound_to(DOM::Node const&);
    // The box of the element's pseudo-element: its own row, or, for one that stands for another element, that
    // element's.
    static BoxSlot of_pseudo_element(DOM::Element const&, CSS::PseudoElement);
    static BoxSlot viewport_of(DOM::Document const&);

    explicit operator bool() const { return m_document && m_slot.index != Compositing::RustFFI::INVALID_NODE_SLOT_INDEX; }
    bool operator==(BoxSlot const& other) const { return m_document == other.m_document && m_slot.index == other.m_slot.index; }

    DOM::Document& document() const { return *m_document; }
    Compositing::RustFFI::NodeSlotId slot() const { return m_slot; }
    // The document's layout rows, as the layout entries take them.
    void* arena() const;

    bool is_live() const;
    bool has_committed_box() const;
    Layout::RustFFI::NodeKind kind() const { return m_kind; }
    StringView kind_name() const;
    bool is_viewport() const { return m_kind == Layout::RustFFI::NodeKind::Viewport; }
    bool is_box() const { return Layout::RustFFI::layout_node_kind_is_box(m_kind); }
    bool is_text() const { return m_kind == Layout::RustFFI::NodeKind::TextNode || m_kind == Layout::RustFFI::NodeKind::GeneratedTextNode; }
    bool is_svg_box() const { return Layout::RustFFI::layout_node_kind_is_svg_box(m_kind); }
    bool is_atomic_inline() const;
    // The viewport, and a box whose overflow is neither visible nor clip.
    bool is_scroll_container() const;

    u32 flags() const;
    bool has_flag(Layout::RustFFI::NodeFlag flag) const { return (flags() & static_cast<u32>(flag)) != 0; }
    bool is_anonymous() const { return has_flag(Layout::RustFFI::NodeFlag::Anonymous); }
    bool has_style() const { return has_flag(Layout::RustFFI::NodeFlag::HasStyle); }

    BoxSlot linked(Layout::RustFFI::FfiNodeLink) const;
    BoxSlot parent() const { return linked(Layout::RustFFI::FfiNodeLink::Parent); }
    BoxSlot first_child() const { return linked(Layout::RustFFI::FfiNodeLink::FirstChild); }
    BoxSlot next_sibling() const { return linked(Layout::RustFFI::FfiNodeLink::NextSibling); }
    // The box after this one in a pre-order walk of the subtree `root` names, or no box past its end.
    BoxSlot next_in_pre_order(BoxSlot const& root) const;
    // The arena finds the containing block by walking up the layout tree.
    BoxSlot containing_block() const;

    // The node the row was built for: nothing for an anonymous row, and the document for the viewport's.
    DOM::NodeIdentity dom_node_identity() const;
    GC::Ptr<DOM::Node> dom_node() const;
    CSS::StyleNodeID style_node() const;
    Optional<CSS::PseudoElement> generated_for_pseudo_element() const;
    // The element a pseudo-element's row was generated for.
    DOM::NodeIdentity pseudo_element_generator_identity() const;
    GC::Ptr<DOM::Element> pseudo_element_generator() const;

    // The payloads of the style record the row holds, or null for a row that holds none (a text row).
    void const* style_payloads() const;
    template<typename StyleGroup>
    StyleGroup const* style_group() const
    {
        auto const* payloads = static_cast<void const* const*>(style_payloads());
        if (!payloads)
            return nullptr;
        return static_cast<StyleGroup const*>(payloads[StyleGroup::style_group_index]);
    }
    // The row whose style a text row reads: its parent.
    BoxSlot style_source() const { return is_text() ? parent() : *this; }

    String debug_description() const;

private:
    BoxSlot(DOM::Document const&, Layout::RustFFI::FfiBoundRow const&);

    GC::Ptr<DOM::Document> m_document;
    Compositing::RustFFI::NodeSlotId m_slot { Compositing::RustFFI::INVALID_NODE_SLOT_INDEX };
    Layout::RustFFI::NodeKind m_kind { Layout::RustFFI::NodeKind::Unset };
};

WEB_API StringView layout_node_kind_name(Layout::RustFFI::NodeKind);

// What a box's debug description says of the DOM node it was built for: `<` its node name `>`, then its ID and
// classes, or `(anonymous)` for none.
WEB_API void append_dom_node_debug_description(StringBuilder&, GC::Ptr<DOM::Node const>);

// Describes the DOM node a debug text the render owner wrote names (a style node, or 0 for the document) to `sink`
// through `append`, as a box's debug description does.
WEB_API void describe_dom_node_for_debug(DOM::Document&, u32 node, void* sink, void (*append)(void*, u8 const*, size_t));
WEB_API bool overflow_value_makes_box_a_scroll_container(CSS::Overflow);

}
