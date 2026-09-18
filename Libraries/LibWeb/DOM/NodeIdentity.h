/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibGC/Ptr.h>
#include <LibWeb/CSS/StyleEngineIdentifiers.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>

namespace Web::DOM {

// Names a DOM node without pointing at it, so that render state can name a node without keeping it
// alive or reading it. An element or text node is named by the StyleNodeID the style tree gave it.
// The document and a shadow root have no StyleNodeID of their own, so they are named by what they
// belong to: the document by itself, a shadow root by its host. An identity whose node has left the
// tree resolves to nothing, where a pointer would have handed back a node the document no longer
// contains.
class WEB_API NodeIdentity {
public:
    NodeIdentity() = default;

    static NodeIdentity of(Node const&);
    static NodeIdentity of(Node const* node) { return node ? of(*node) : NodeIdentity {}; }
    static NodeIdentity of_style_node(CSS::StyleNodeID);
    static NodeIdentity of_document() { return { Kind::Document, {} }; }

    bool is_none() const { return m_kind == Kind::None; }
    explicit operator bool() const { return !is_none(); }
    bool operator==(NodeIdentity const&) const = default;

    [[nodiscard]] GC::Ptr<Node> resolve(Document&) const;

private:
    enum class Kind : u8 {
        None,
        StyleNode,
        ShadowRootOfStyleNode,
        Document,
    };

    NodeIdentity(Kind kind, CSS::StyleNodeID style_node)
        : m_style_node(style_node)
        , m_kind(kind)
    {
    }

    CSS::StyleNodeID m_style_node {};
    Kind m_kind { Kind::None };
};

}
