/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/GeneratedContent.h>
#include <LibWeb/CSS/PublishedStyleRecord.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Node.h>

namespace Web::CSS {

// Whether the node or any of its descendants styles a counter or a quote, as the records the drain installed say: the
// ones the node's boxes were built with. Moving such a subtree renumbers what follows it, so the layout tree update has
// to rebuild rather than splice, and so does a removal.
bool subtree_affects_generated_content_state(DOM::Node const& node)
{
    auto affects = [](PublishedStyleRecord const* style_record) {
        return style_record && style_record->affects_generated_content_state();
    };
    return node.for_each_in_inclusive_subtree_of_type<DOM::Element>([&](DOM::Element const& element) {
        if (affects(element.published_style_record())
            || affects(element.published_style_record(PseudoElement::Before))
            || affects(element.published_style_record(PseudoElement::After))
            || affects(element.published_style_record(PseudoElement::Marker)))
            return TraversalDecision::Break;
        return TraversalDecision::Continue;
    }) == TraversalDecision::Break;
}

}
