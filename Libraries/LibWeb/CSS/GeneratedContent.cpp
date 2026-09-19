/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/GeneratedContent.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEngineBridge.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/Text.h>

namespace Web::CSS {

bool subtree_affects_generated_content_state(DOM::Node const& node)
{
    StyleNodeID identity;
    if (auto const* element = as_if<DOM::Element>(node))
        identity = element->style_node_id();
    else if (auto const* text = as_if<DOM::Text>(node))
        identity = text->style_node_id();
    if (identity.value() == 0)
        return false;
    return node.document().style_computer().style_engine().subtree_affects_generated_content_state(identity);
}

}
