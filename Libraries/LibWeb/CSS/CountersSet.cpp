/*
 * Copyright (c) 2024-2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/NeverDestroyed.h>
#include <LibWeb/CSS/CountersSet.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>

namespace Web::CSS {

bool innermost_list_item_counter_is_own_forward_counter(DOM::Element const& element)
{
    auto* arena = const_cast<DOM::Document&>(element.document()).layout_node_arena_if_created();
    if (!arena)
        return false;
    return Layout::RustFFI::render_owner_innermost_list_item_counter_is_own_forward_counter(arena->render_document(), element.style_node_id().value());
}

Utf16FlyString const& list_item_counter_name()
{
    static NeverDestroyed<Utf16FlyString> name = "list-item"_utf16_fly_string;
    return *name;
}

}
