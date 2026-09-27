/*
 * Copyright (c) 2025, Callum Law <callumlaw1709@outlook.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include "RandomValueSharingStyleValue.h"
#include <LibWeb/CSS/Serialize.h>
#include <LibWeb/CSS/StyleComputeFFI.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/DOM/Document.h>

namespace Web::CSS {

ValueComparingNonnullRefPtr<StyleValue const> RandomValueSharingStyleValue::absolutized(ComputationContext const& computation_context) const
{
    auto dependencies = StyleValueFFI::rust_random_sharing_input_dependencies(rust_style_value_data());
    auto length = to_ffi_length_resolution_context_with_container_bases(computation_context.length_resolution_context, dependencies & 0x3f);
    auto* element = computation_context.abstract_element.has_value() ? &computation_context.abstract_element->element() : nullptr;
    if (element && (dependencies & (1 << 6)))
        const_cast<DOM::Element&>(*element).set_style_uses_tree_counting_function();
    // Settling an element-shared random base value writes it to the style engine.
    auto* engine = element ? const_cast<DOM::Document&>(element->document()).render_inputs_for_write().style_engine().rust_handle() : nullptr;
    return StyleValue::adopt_rust_style_value_data(StyleValueFFI::rust_random_sharing_absolutize(
        rust_style_value_data(), &length, engine, element ? element->style_node_id().value() : 0));
}

double RandomValueSharingStyleValue::random_base_value() const
{
    return number_from_style_value(*fixed_value(), {});
}

}
