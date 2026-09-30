/*
 * Copyright (c) 2024-2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/NeverDestroyed.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/CountersSet.h>
#include <LibWeb/DOM/Element.h>

namespace Web::CSS {

// NB: An element whose counter-reset instantiates a list-item counter creates the innermost one of its counters set,
//     which counts forward unless it is reversed. One that instantiates none has no list-item counter of its own but
//     one counter-set or counter-increment instantiates, where it has no other, which this leaves out.
bool innermost_list_item_counter_is_own_forward_counter(DOM::Element const& element)
{
    auto style = element.computed_style();
    if (!style)
        return false;
    // https://drafts.csswg.org/css-lists-3/#counter-reset
    // "If multiple instances of the same <counter-name> occur in the property value, only the last one is honored."
    auto counter_reset = style->counter_reset();
    auto list_item_reset = counter_reset.last_matching([](auto const& definition) { return definition.name == list_item_counter_name(); });
    return list_item_reset.has_value() && !list_item_reset->is_reversed;
}

Utf16FlyString const& list_item_counter_name()
{
    static NeverDestroyed<Utf16FlyString> name = "list-item"_utf16_fly_string;
    return *name;
}

}
