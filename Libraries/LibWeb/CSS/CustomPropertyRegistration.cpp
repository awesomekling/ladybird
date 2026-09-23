/*
 * Copyright (c) 2026, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/ScopeGuard.h>
#include <LibWeb/CSS/CalculationResolutionContext.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/CustomPropertyRegistration.h>
#include <LibWeb/CSS/HypotheticalElement.h>
#include <LibWeb/CSS/Length.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>
#include <LibWeb/ComputedValuesRustFFI.h>
#include <LibWeb/DOM/AbstractElement.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/StyleEngineRustFFI.h>

namespace Web::CSS {

RefPtr<StyleValue const> answer_hypothetical_parent_custom_property(HypotheticalElement const&, Utf16FlyString const&);
NonnullRefPtr<StyleValue const> answer_custom_property_from_engine(AbstractOrHypotheticalElement const&, Utf16FlyString const&);

static ComputedValuesFFI::FfiUtf16View ffi_utf16_view(Utf16View view)
{
    return {
        .ascii = view.has_ascii_storage() ? reinterpret_cast<u8 const*>(view.ascii_span().data()) : nullptr,
        .utf16 = view.has_ascii_storage() ? nullptr : reinterpret_cast<u16 const*>(view.utf16_span().data()),
        .length = view.length_in_code_units(),
    };
}

RefPtr<StyleValue const> answer_hypothetical_parent_custom_property(HypotheticalElement const& parent, Utf16FlyString const& name)
{
    auto* registry = ComputedValuesFFI::rust_custom_property_registry_create();
    ScopeGuard destroy_registry = [&] { ComputedValuesFFI::rust_custom_property_registry_destroy(registry); };
    Vector<Utf16String> names;
    Vector<Optional<Utf16String>> initial_values;
    Vector<NonnullRefPtr<StyleValue const>> computed_initial_values;
    Vector<ComputedValuesFFI::FfiCustomPropertyRegistration> registrations;
    names.ensure_capacity(parent.custom_property_registry.size());
    initial_values.ensure_capacity(parent.custom_property_registry.size());
    computed_initial_values.ensure_capacity(parent.custom_property_registry.size());
    registrations.ensure_capacity(parent.custom_property_registry.size());
    for (auto const& [property_name, registration] : parent.custom_property_registry) {
        names.unchecked_append(property_name.to_utf16_string());
        initial_values.unchecked_append(registration.initial_value
                ? Optional<Utf16String> { registration.initial_value->to_utf16_string(SerializationMode::ResolvedValueForReparse) }
                : Optional<Utf16String> {});
        computed_initial_values.unchecked_append(compute_registered_custom_property_initial_value(parent.root_element.document(), registration));
        auto const& initial_value = initial_values.last();
        registrations.unchecked_append({
            .name = ffi_utf16_view(names.last()),
            .syntax = registration.syntax.data(),
            .inherits = registration.inherit,
            .has_initial_value = initial_value.has_value(),
            .initial_value = initial_value.has_value() ? ffi_utf16_view(*initial_value) : ComputedValuesFFI::FfiUtf16View {},
            .computed_initial_value = computed_initial_values.last()->rust_style_value_data(),
        });
    }
    auto const& document = parent.root_element.document();
    auto const& document_url = document.serialized_url();
    auto const& document_base_url = document.serialized_base_url();
    ComputedValuesFFI::FfiCustomPropertyRegistryContext context {
        .document_url = document_url.bytes().data(),
        .document_url_length = document_url.bytes().size(),
        .document_base_url = document_base_url.bytes().data(),
        .document_base_url_length = document_base_url.bytes().size(),
    };
    ComputedValuesFFI::rust_custom_property_registry_update(registry, &context, registrations.data(), registrations.size());
    Vector<u16> property_name;
    auto property_name_view = name.view();
    property_name.ensure_capacity(property_name_view.length_in_code_units());
    for (size_t index = 0; index < property_name_view.length_in_code_units(); ++index)
        property_name.unchecked_append(property_name_view.code_unit_at(index));
    auto& style_engine = const_cast<DOM::Document&>(document).style_computer().style_engine();
    auto* value = StyleEngineFFI::style_engine_answer_hypothetical_parent_custom_property(
        style_engine.rust_handle(), parent.root_element.element().style_node_id().value(), parent.custom_property_data->rust_store(), registry, property_name.data(), property_name.size());
    if (!value)
        return {};
    return StyleValue::adopt_rust_style_value_data(static_cast<StyleValueFFI::StyleValueData const*>(value));
}

NonnullRefPtr<StyleValue const> answer_custom_property_from_engine(AbstractOrHypotheticalElement const& source, Utf16FlyString const& name)
{
    if (source.has<HypotheticalElement*>()) {
        if (auto value = answer_hypothetical_parent_custom_property(*source.get<HypotheticalElement*>(), name))
            return value.release_nonnull();
    } else {
        auto const& abstract_element = source.get<DOM::AbstractElement>();
        auto& document = const_cast<DOM::Document&>(source.document());
        auto& style_computer = document.style_computer();
        auto& style_engine = style_computer.style_engine();
        if (!abstract_element.pseudo_element().has_value()) {
            auto answer = style_engine.answer_record_demand(abstract_element.element().style_node_id(), {}, false, false, true);
            if (answer.record.style_record) {
                bool installable = false;
                auto environment = abstract_element.element().custom_property_environment_of_engine_record(StyleRecordID { answer.record.style_record }, installable);
                if (installable) {
                    if (environment) {
                        if (auto const* property = environment->get(name))
                            return property->value;
                    }
                    return initial_custom_property_value(source.get_registered_custom_property(name), source.document());
                }
            }
        }
        if (auto data = abstract_element.custom_property_data()) {
            Vector<u16> property_name;
            auto name_view = name.view();
            property_name.ensure_capacity(name_view.length_in_code_units());
            for (size_t index = 0; index < name_view.length_in_code_units(); ++index)
                property_name.unchecked_append(name_view.code_unit_at(index));
            auto* value = StyleEngineFFI::style_engine_answer_hypothetical_parent_custom_property(
                style_engine.rust_handle(), abstract_element.element().style_node_id().value(), data->rust_store(), document.rust_custom_property_registry(), property_name.data(), property_name.size());
            if (value)
                return StyleValue::adopt_rust_style_value_data(static_cast<StyleValueFFI::StyleValueData const*>(value));
        }
    }
    if (auto value = source.get_custom_property(name))
        return value.release_nonnull();
    return initial_custom_property_value(source.get_registered_custom_property(name), source.document());
}

// https://drafts.css-houdini.org/css-properties-values-api/#calculation-of-computed-values
NonnullRefPtr<StyleValue const> compute_registered_custom_property_value(CustomPropertyRegistration const& registration, NonnullRefPtr<StyleValue const> value, ComputationContext const& computation_context)
{
    // If the registration’s syntax is the universal syntax definition, the computed value is the same as for
    // unregistered custom properties (either the specified value with variables substituted, or the guaranteed-invalid
    // value).
    if (registration.syntax.is_universal())
        return value;

    // Otherwise...
    // NB: Our regular computed-value computation already behaves how this wants.
    return value->absolutized(computation_context);
}

NonnullRefPtr<StyleValue const> compute_registered_custom_property_initial_value(DOM::Document const& document, CustomPropertyRegistration const& registration)
{
    if (registration.computed_initial_value)
        return *registration.computed_initial_value;

    NonnullRefPtr<StyleValue const> computed_initial_value = StyleValue::create_guaranteed_invalid();
    if (registration.initial_value) {
        ComputationContext computation_context {
            .length_resolution_context = Length::ResolutionContext::for_document(document),
        };
        computation_context.reset_viewport_metric_dependency_tracking();
        computed_initial_value = compute_registered_custom_property_value(registration, *registration.initial_value, computation_context);
        const_cast<CustomPropertyRegistration&>(registration).computed_initial_value_depends_on_viewport_metrics = computation_context.depends_on_viewport_metrics();
    }

    const_cast<CustomPropertyRegistration&>(registration).computed_initial_value = computed_initial_value;
    return computed_initial_value;
}

NonnullRefPtr<StyleValue const> initial_custom_property_value(Optional<CustomPropertyRegistration const&> registration, DOM::Document const& document)
{
    if (registration.has_value())
        return compute_registered_custom_property_initial_value(document, registration.value());

    // For non-registered properties, the initial value is the guaranteed-invalid value.
    // See: https://drafts.csswg.org/css-variables/#propdef-
    return StyleValue::create_guaranteed_invalid();
}

NonnullRefPtr<StyleValue const> inherited_custom_property_value(Optional<CustomPropertyRegistration const&> registration, AbstractOrHypotheticalElement const& element, Utf16FlyString const& name, ComputedStyleWorkingSet const*)
{
    if (auto element_to_inherit_style_from = element.element_to_inherit_style_from(); element_to_inherit_style_from.has_value()) {
        if (auto parent_property = element_to_inherit_style_from->get_custom_property(name)) {
            // NB: With normal style computation we know that ancestors' custom properties are already in their
            //     computed form (since style computation happens in tree order).
            if (element.has<DOM::AbstractElement>())
                return parent_property.release_nonnull();

            VERIFY(element.has<HypotheticalElement*>());

            // NB: Unlike with normal style computation - we don't know that parent's values are in their computed forms
            //     when evaluating a custom function - a property may rely on resolving a custom function which in turn
            //     contains a value which inherits a different, not yet computed, custom property's value.

            // FIXME: We probably need to compute this against the declaring element rather than the parent element.
            auto computed_parent_value = answer_custom_property_from_engine(element_to_inherit_style_from.value(), name);

            // https://drafts.csswg.org/css-mixins/#resolve-function-styles
            // inherit
            //   Resolves like an inherit() function with the custom property name as its one and only argument.
            // Note: This ensures that a function parameter defaulted to inherit is reinterpreted using the local parameter type.
            if (computed_parent_value->is_guaranteed_invalid())
                return StyleValue::create_guaranteed_invalid();

            return UnresolvedStyleValue::create(computed_parent_value->is_unresolved()
                    ? computed_parent_value->as_unresolved().token_source()
                    : computed_parent_value->to_utf16_string(SerializationMode::ResolvedValueForReparse),
                {});
        }
    }

    return initial_custom_property_value(registration, element.document());
}

}
