/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/DOM/Text.h>

namespace Web::DOM {

NodeIdentity NodeIdentity::of_style_node(CSS::StyleNodeID style_node)
{
    if (style_node == 0)
        return {};
    return { Kind::StyleNode, style_node };
}

NodeIdentity NodeIdentity::of(Node const& node)
{
    if (node.is_document())
        return of_document();
    if (auto const* element = as_if<Element>(node))
        return of_style_node(element->style_node_id());
    if (auto const* text = as_if<Text>(node))
        return of_style_node(text->style_node_id());
    if (auto const* shadow_root = as_if<ShadowRoot>(node)) {
        auto const* host = shadow_root->host();
        if (host && host->style_node_id() != 0)
            return { Kind::ShadowRootOfStyleNode, host->style_node_id() };
    }
    return {};
}

GC::Ptr<Node> NodeIdentity::resolve(Document& document) const
{
    switch (m_kind) {
    case Kind::None:
        return nullptr;
    case Kind::StyleNode:
        return document.style_computer().node_for_style_node(m_style_node);
    case Kind::ShadowRootOfStyleNode: {
        auto host = document.style_computer().element_for_style_node(m_style_node);
        return host ? host->shadow_root() : nullptr;
    }
    case Kind::Document:
        return document;
    }
    VERIFY_NOT_REACHED();
}

}
