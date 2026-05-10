/*
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/FontFace.h>
#include <LibWeb/CSS/Parser/ErrorReporter.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/PropertyID.h>
#include <LibWeb/CSS/StyleValues/CalculatedStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleSystemStyleValue.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/FontSourceStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/UnicodeRangeStyleValue.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>

namespace Web::CSS::Parser {

static RefPtr<CalculatedStyleValue const> materialize_descriptor_calculation_tree(RustComponentValueParser::DescriptorResultItem const& item, ValueType value_type, NumericRange range)
{
    if (item.calculation_node_events.is_empty())
        return nullptr;

    CalculationContext calculation_context {
        .resolve_numbers_as_integers = value_type == ValueType::Integer,
    };

    switch (value_type) {
    case ValueType::Integer:
        calculation_context.accepted_ranges_by_type.set(ValueType::Integer, range);
        break;
    case ValueType::Number:
        calculation_context.accepted_ranges_by_type.set(ValueType::Number, range);
        break;
    case ValueType::Length:
        calculation_context.accepted_ranges_by_type.set(ValueType::Length, range);
        break;
    case ValueType::Percentage:
        calculation_context.accepted_ranges_by_type.set(ValueType::Percentage, range);
        break;
    default:
        return nullptr;
    }

    Vector<NonnullRefPtr<CalculationNode const>> stack;
    bool saw_percentage_leaf = false;

    auto pop_children = [&](u32 child_count) -> Optional<Vector<NonnullRefPtr<CalculationNode const>>> {
        if (child_count > stack.size())
            return {};

        Vector<NonnullRefPtr<CalculationNode const>> reversed_children;
        reversed_children.ensure_capacity(child_count);
        for (u32 i = 0; i < child_count; ++i)
            reversed_children.append(stack.take_last());

        Vector<NonnullRefPtr<CalculationNode const>> children;
        children.ensure_capacity(child_count);
        for (auto i = reversed_children.size(); i > 0; --i)
            children.append(reversed_children[i - 1]);
        return children;
    };

    auto numeric_node_from_event = [&](RustComponentValueParser::RustCalculationNodeEvent const& event) -> RefPtr<CalculationNode const> {
        if (!event.numeric_value.has_value())
            return nullptr;

        switch (event.primitive_kind) {
        case FFI::CssPrimitiveValueKind::Number:
            return NumericCalculationNode::create(Number { Number::Type::Number, *event.numeric_value }, calculation_context);
        case FFI::CssPrimitiveValueKind::Keyword: {
            auto maybe_keyword = keyword_from_string(event.metadata);
            if (!maybe_keyword.has_value())
                return nullptr;
            return NumericCalculationNode::from_keyword(*maybe_keyword, calculation_context);
        }
        case FFI::CssPrimitiveValueKind::Percentage:
            saw_percentage_leaf = true;
            return NumericCalculationNode::create(Percentage { *event.numeric_value }, calculation_context);
        case FFI::CssPrimitiveValueKind::Angle: {
            auto unit = string_to_angle_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Angle { *event.numeric_value, unit.release_value() }, calculation_context);
        }
        case FFI::CssPrimitiveValueKind::Flex: {
            auto unit = string_to_flex_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Flex { *event.numeric_value, unit.release_value() }, calculation_context);
        }
        case FFI::CssPrimitiveValueKind::Frequency: {
            auto unit = string_to_frequency_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Frequency { *event.numeric_value, unit.release_value() }, calculation_context);
        }
        case FFI::CssPrimitiveValueKind::Length: {
            auto unit = string_to_length_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Length { *event.numeric_value, unit.release_value() }, calculation_context);
        }
        case FFI::CssPrimitiveValueKind::Resolution: {
            auto unit = string_to_resolution_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Resolution { *event.numeric_value, unit.release_value() }, calculation_context);
        }
        case FFI::CssPrimitiveValueKind::Time: {
            auto unit = string_to_time_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Time { *event.numeric_value, unit.release_value() }, calculation_context);
        }
        default:
            return nullptr;
        }
    };

    auto matches_number = [&](CalculationNode const& node) {
        auto const& numeric_type = node.numeric_type();
        return numeric_type.has_value() && numeric_type->matches_number(calculation_context.percentages_resolve_as);
    };
    auto matches_sign_argument = [&](CalculationNode const& node) {
        auto const& numeric_type = node.numeric_type();
        return numeric_type.has_value()
            && (numeric_type->matches_number(calculation_context.percentages_resolve_as)
                || numeric_type->matches_dimension()
                || numeric_type->matches_percentage());
    };
    auto have_consistent_types = [](CalculationNode const& left, CalculationNode const& right) {
        auto const& left_numeric_type = left.numeric_type();
        auto const& right_numeric_type = right.numeric_type();
        return left_numeric_type.has_value()
            && right_numeric_type.has_value()
            && left_numeric_type->consistent_type(*right_numeric_type).has_value();
    };
    auto append_round_node = [&](Vector<NonnullRefPtr<CalculationNode const>> const& children, RoundingStrategy strategy) -> bool {
        if (children.size() != 2)
            return false;
        if (!matches_sign_argument(*children.at(0)) || !matches_sign_argument(*children.at(1)))
            return false;
        if (!have_consistent_types(*children.at(0), *children.at(1)))
            return false;

        stack.append(RoundCalculationNode::create(strategy, children.at(0), children.at(1)));
        return true;
    };

    for (auto const& event : item.calculation_node_events) {
        switch (event.kind) {
        case FFI::CssCalculationNodeKind::Numeric: {
            auto numeric_node = numeric_node_from_event(event);
            if (!numeric_node)
                return nullptr;
            stack.append(numeric_node.release_nonnull());
            break;
        }
        case FFI::CssCalculationNodeKind::Sum: {
            auto children = pop_children(event.child_count);
            if (!children.has_value())
                return nullptr;
            stack.append(SumCalculationNode::create(children.release_value()));
            break;
        }
        case FFI::CssCalculationNodeKind::Product: {
            auto children = pop_children(event.child_count);
            if (!children.has_value())
                return nullptr;
            stack.append(ProductCalculationNode::create(children.release_value()));
            break;
        }
        case FFI::CssCalculationNodeKind::Negate: {
            auto children = pop_children(1);
            if (!children.has_value())
                return nullptr;
            stack.append(NegateCalculationNode::create(children->first()));
            break;
        }
        case FFI::CssCalculationNodeKind::Invert: {
            auto children = pop_children(1);
            if (!children.has_value())
                return nullptr;
            stack.append(InvertCalculationNode::create(children->first()));
            break;
        }
        case FFI::CssCalculationNodeKind::Function: {
            auto children = pop_children(event.child_count);
            if (!children.has_value())
                return nullptr;

            if (event.metadata.equals_ignoring_ascii_case("min"sv)) {
                stack.append(MinCalculationNode::create(children.release_value()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("max"sv)) {
                stack.append(MaxCalculationNode::create(children.release_value()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("hypot"sv)) {
                stack.append(HypotCalculationNode::create(children.release_value()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("clamp"sv) && children->size() == 3) {
                stack.append(ClampCalculationNode::create(children->at(0), children->at(1), children->at(2)));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("abs"sv) && children->size() == 1) {
                stack.append(AbsCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("sign"sv) && children->size() == 1) {
                if (!matches_sign_argument(*children->first()))
                    return nullptr;
                stack.append(SignCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("sqrt"sv) && children->size() == 1) {
                stack.append(SqrtCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("sin"sv) && children->size() == 1) {
                stack.append(SinCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("cos"sv) && children->size() == 1) {
                stack.append(CosCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("tan"sv) && children->size() == 1) {
                stack.append(TanCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("asin"sv) && children->size() == 1) {
                stack.append(AsinCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("acos"sv) && children->size() == 1) {
                stack.append(AcosCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("atan"sv) && children->size() == 1) {
                stack.append(AtanCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("atan2"sv) && children->size() == 2) {
                stack.append(Atan2CalculationNode::create(children->at(0), children->at(1)));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("pow"sv) && children->size() == 2) {
                if (!matches_number(*children->at(0)) || !matches_number(*children->at(1)) || !have_consistent_types(*children->at(0), *children->at(1)))
                    return nullptr;
                stack.append(PowCalculationNode::create(children->at(0), children->at(1)));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("log"sv) && (children->size() == 1 || children->size() == 2)) {
                auto value = children->at(0);
                auto base = children->size() == 2
                    ? children->at(1)
                    : NumericCalculationNode::from_keyword(Keyword::E, calculation_context).release_nonnull();
                if (!matches_number(*value) || !matches_number(*base) || !have_consistent_types(*value, *base))
                    return nullptr;
                stack.append(LogCalculationNode::create(value, base));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("exp"sv) && children->size() == 1) {
                if (!matches_number(*children->first()))
                    return nullptr;
                stack.append(ExpCalculationNode::create(children->first()));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("round nearest"sv)) {
                if (!append_round_node(*children, RoundingStrategy::Nearest))
                    return nullptr;
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("round up"sv)) {
                if (!append_round_node(*children, RoundingStrategy::Up))
                    return nullptr;
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("round down"sv)) {
                if (!append_round_node(*children, RoundingStrategy::Down))
                    return nullptr;
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("round to-zero"sv)) {
                if (!append_round_node(*children, RoundingStrategy::ToZero))
                    return nullptr;
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("mod"sv) && children->size() == 2) {
                stack.append(ModCalculationNode::create(children->at(0), children->at(1)));
                break;
            }
            if (event.metadata.equals_ignoring_ascii_case("rem"sv) && children->size() == 2) {
                stack.append(RemCalculationNode::create(children->at(0), children->at(1)));
                break;
            }
            return nullptr;
        }
        case FFI::CssCalculationNodeKind::TreeCountingFunction:
            return nullptr;
        }
    }

    if (stack.size() != 1)
        return nullptr;

    auto calculation_tree = simplify_a_calculation_tree(*stack.first(), calculation_context, CalculationResolutionContext {});
    auto calculation_type = calculation_tree->numeric_type();
    if (!calculation_type.has_value())
        return nullptr;

    auto calculated_value = CalculatedStyleValue::create(calculation_tree, calculation_type.release_value(), calculation_context);
    switch (value_type) {
    case ValueType::Integer:
    case ValueType::Number:
        if (saw_percentage_leaf || !calculated_value->resolves_to_number())
            return nullptr;
        break;
    case ValueType::Length:
        if (!calculated_value->resolves_to_length())
            return nullptr;
        break;
    case ValueType::Percentage:
        if (!calculated_value->resolves_to_percentage())
            return nullptr;
        break;
    default:
        return nullptr;
    }

    return calculated_value;
}

static RefPtr<StyleValue const> materialize_descriptor_symbol(RustComponentValueParser::DescriptorResultItem const& item)
{
    switch (item.primitive_kind) {
    case FFI::CssPrimitiveValueKind::String:
        return StringStyleValue::create(FlyString::from_utf8_without_validation(item.source.bytes()));
    case FFI::CssPrimitiveValueKind::CustomIdent:
        return CustomIdentStyleValue::create(FlyString::from_utf8_without_validation(item.source.bytes()));
    default:
        return nullptr;
    }
}

static RefPtr<StyleValue const> materialize_descriptor_integer_symbol_pair(RustComponentValueParser::DescriptorResultItem const& item)
{
    if (item.primitive_kind != FFI::CssPrimitiveValueKind::String && item.primitive_kind != FFI::CssPrimitiveValueKind::CustomIdent)
        return nullptr;

    RefPtr<StyleValue const> integer;
    if (item.has_numeric_value) {
        if (item.numeric_value < 0 || item.numeric_value > NumericLimits<i32>::max())
            return nullptr;
        integer = IntegerStyleValue::create(static_cast<i32>(item.numeric_value));
    } else {
        integer = materialize_descriptor_calculation_tree(item, ValueType::Integer, non_negative_integer_range);
    }
    if (!integer)
        return nullptr;

    auto symbol = materialize_descriptor_symbol(item);
    if (!symbol)
        return nullptr;

    return StyleValueList::create({ integer.release_nonnull(), symbol.release_nonnull() }, StyleValueList::Separator::Space);
}

static RefPtr<StyleValue const> materialize_descriptor_font_weight_absolute(RustComponentValueParser::DescriptorResultItem const& item)
{
    switch (item.primitive_kind) {
    case FFI::CssPrimitiveValueKind::Keyword: {
        auto keyword = keyword_from_string(item.source);
        if (!keyword.has_value())
            return nullptr;
        return KeywordStyleValue::create(keyword.value());
    }
    case FFI::CssPrimitiveValueKind::Number:
        if (!item.has_numeric_value)
            return nullptr;
        return NumberStyleValue::create(item.numeric_value);
    case FFI::CssPrimitiveValueKind::Invalid:
        return materialize_descriptor_calculation_tree(item, ValueType::Number, infinite_range);
    default:
        return nullptr;
    }
}

static RefPtr<StyleValue const> materialize_descriptor_counter_style_range_bound(RustComponentValueParser::DescriptorResultItem const& item)
{
    switch (item.primitive_kind) {
    case FFI::CssPrimitiveValueKind::Keyword: {
        auto keyword = keyword_from_string(item.source);
        if (!keyword.has_value() || keyword.value() != Keyword::Infinite)
            return nullptr;
        return KeywordStyleValue::create(Keyword::Infinite);
    }
    case FFI::CssPrimitiveValueKind::Integer:
        if (!item.has_numeric_value)
            return nullptr;
        if (item.numeric_value < NumericLimits<i32>::min() || item.numeric_value > NumericLimits<i32>::max())
            return nullptr;
        return IntegerStyleValue::create(static_cast<i32>(item.numeric_value));
    case FFI::CssPrimitiveValueKind::Invalid:
        return materialize_descriptor_calculation_tree(item, ValueType::Integer, infinite_integer_range);
    default:
        return nullptr;
    }
}

Parser::ParseErrorOr<NonnullRefPtr<StyleValue const>> Parser::parse_descriptor_value(AtRuleID at_rule_id, DescriptorNameAndID const& descriptor_name_and_id, TokenStream<ComponentValue>& tokens, Optional<String> original_source_text)
{
    if (!RustComponentValueParser::at_rule_supports_descriptor(at_rule_id, descriptor_name_and_id.id())) {
        ErrorReporter::the().report(UnknownPropertyError {
            .rule_name = to_string(at_rule_id),
            .property_name = descriptor_name_and_id.name(),
        });
        return ParseError::SyntaxError;
    }

    auto context_guard = push_temporary_value_parsing_context(DescriptorContext { at_rule_id, descriptor_name_and_id.id() });

    auto transaction = tokens.begin_transaction();

    auto descriptor_value_start_index = tokens.current_index();
    SubstitutionFunctionsPresence substitution_functions_presence {};

    tokens.mark();
    while (tokens.has_next_token()) {
        auto const& token = tokens.consume_a_token();

        if (token.is(Token::Type::Semicolon))
            return ParseError::SyntaxError;

        if (collect_arbitrary_substitution_function_presence(token, substitution_functions_presence).is_error())
            return ParseError::SyntaxError;
    }

    if (substitution_functions_presence.has_any()) {
        // https://drafts.csswg.org/css-values-5/#resolve-property
        // Unless otherwise specified, arbitrary substitution functions can be used in place of any part of any
        // property’s value (including within other functional notations); and are not valid in any other context.

        // NB: Since we are not in a property value context we only allow ASFs if they are explicitly allowed in
        //     Descriptors.json
        if (!RustComponentValueParser::descriptor_allows_arbitrary_substitution_functions(at_rule_id, descriptor_name_and_id.id())) {
            ErrorReporter::the().report(InvalidValueError {
                .value_type = MUST(String::formatted("{}/{}", to_string(at_rule_id), descriptor_name_and_id.name())),
                .value_string = tokens.dump_string(),
                .description = "ASFs are not supported in this descriptor"_string,
            });
            return ParseError::SyntaxError;
        }

        return UnresolvedStyleValue::create(Vector<ComponentValue> { tokens.tokens_since(descriptor_value_start_index) }, substitution_functions_presence);
    }

    tokens.restore_a_mark();

    Optional<ComputationContext> computation_context = m_document
        ? ComputationContext { .length_resolution_context = Length::ResolutionContext::for_document(*m_document) }
        : Optional<ComputationContext> {};

    auto descriptor_source = original_source_text.value_or_lazy_evaluated([&] {
        return serialize_component_values_for_reparsing(tokens.remaining_tokens());
    });
    auto descriptor_value = RustComponentValueParser::parse_descriptor(at_rule_id, descriptor_name_and_id.id(), descriptor_source.bytes_as_string_view(), "utf-8"sv);
    if (descriptor_value.has_value()) {
        auto parsed_style_value = descriptor_value->syntax.visit(
            [&](Keyword keyword) {
                return parse_all_as_single_keyword_value(tokens, keyword);
            },
            [&](PropertyID property_id) -> RefPtr<StyleValue const> {
                auto value_or_error = parse_css_value(property_id, tokens);
                if (value_or_error.is_error())
                    return nullptr;
                auto value_for_property = value_or_error.release_value();
                // Descriptors don't accept the CSS-wide keywords
                if (value_for_property->is_css_wide_keyword())
                    return nullptr;
                return value_for_property;
            },
            [&](DescriptorMetadata::ValueType value_type) -> RefPtr<StyleValue const> {
                switch (value_type) {
                case DescriptorMetadata::ValueType::CounterStyleAdditiveSymbols: {
                    // [ <integer [0,∞]> && <symbol> ]#
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& additive_symbols = descriptor_value->result;
                    if (!additive_symbols.has_value() || additive_symbols->kind != FFI::CssDescriptorResultKind::CounterStyleAdditiveSymbols)
                        return nullptr;

                    StyleValueVector additive_tuples;
                    for (auto const& tuple : additive_symbols->items) {
                        auto additive_tuple = materialize_descriptor_integer_symbol_pair(tuple);
                        if (!additive_tuple)
                            return nullptr;

                        additive_tuples.append(additive_tuple.release_nonnull());
                    }

                    auto additive_tuple_list = StyleValueList::create(move(additive_tuples), StyleValueList::Separator::Comma, StyleValueList::Collapsible::No);

                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-symbols
                    // Each entry in the additive-symbols descriptor’s value defines an additive tuple, which consists
                    // of a counter symbol and an integer weight. Each weight must be a non-negative integer, and the
                    // additive tuples must be specified in order of strictly descending weight; otherwise, the
                    // declaration is invalid and must be ignored.
                    i32 previous_weight = NumericLimits<i32>::max();

                    for (auto const& tuple_style_value : additive_tuple_list->as_value_list().values()) {
                        auto const& weight = tuple_style_value->as_value_list().value_at(0, false);

                        i32 resolved_weight;

                        if (weight->is_integer()) {
                            resolved_weight = weight->as_integer().integer();
                        } else {
                            // FIXME: How should we actually handle calc() when we have no document to absolutize against
                            if (!computation_context.has_value())
                                return nullptr;

                            resolved_weight = weight->absolutized(computation_context.value())->as_calculated().resolve_integer({}).value();
                        }

                        if (resolved_weight >= previous_weight)
                            return nullptr;

                        previous_weight = resolved_weight;
                    }

                    return additive_tuple_list;
                }
                case DescriptorMetadata::ValueType::CounterStyleSystem: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-system
                    // cyclic | numeric | alphabetic | symbolic | additive | [fixed <integer>?] | [ extends <counter-style-name> ]
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& system = descriptor_value->result;
                    if (!system.has_value())
                        return nullptr;

                    switch (system->kind) {
                    case FFI::CssDescriptorResultKind::CounterStyleSystemCyclic:
                        return CounterStyleSystemStyleValue::create(CounterStyleSystem::Cyclic);
                    case FFI::CssDescriptorResultKind::CounterStyleSystemNumeric:
                        return CounterStyleSystemStyleValue::create(CounterStyleSystem::Numeric);
                    case FFI::CssDescriptorResultKind::CounterStyleSystemAlphabetic:
                        return CounterStyleSystemStyleValue::create(CounterStyleSystem::Alphabetic);
                    case FFI::CssDescriptorResultKind::CounterStyleSystemSymbolic:
                        return CounterStyleSystemStyleValue::create(CounterStyleSystem::Symbolic);
                    case FFI::CssDescriptorResultKind::CounterStyleSystemAdditive:
                        return CounterStyleSystemStyleValue::create(CounterStyleSystem::Additive);
                    case FFI::CssDescriptorResultKind::CounterStyleSystemFixed:
                        return CounterStyleSystemStyleValue::create_fixed(nullptr);
                    case FFI::CssDescriptorResultKind::CounterStyleSystemFixedWithInteger: {
                        if (system->items.size() != 1)
                            return nullptr;
                        auto const& system_item = system->items.first();
                        if (system_item.primitive_kind == FFI::CssPrimitiveValueKind::Integer && system_item.has_numeric_value)
                            return CounterStyleSystemStyleValue::create_fixed(IntegerStyleValue::create(static_cast<i32>(system_item.numeric_value)));

                        auto integer_value = materialize_descriptor_calculation_tree(system_item, ValueType::Integer, infinite_integer_range);
                        if (!integer_value)
                            return nullptr;
                        return CounterStyleSystemStyleValue::create_fixed(integer_value);
                    }
                    case FFI::CssDescriptorResultKind::CounterStyleSystemExtends:
                        if (system->items.size() != 1)
                            return nullptr;
                        return CounterStyleSystemStyleValue::create_extends(FlyString::from_utf8_without_validation(system->items.first().source.bytes()));
                    default:
                        break;
                    }
                    return nullptr;
                }
                case DescriptorMetadata::ValueType::CounterStyleName: {
                    // https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style-name
                    // <counter-style-name> is a <custom-ident> that is not an ASCII case-insensitive match for none.
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& counter_style_name_descriptor = descriptor_value->result;
                    if (!counter_style_name_descriptor.has_value() || counter_style_name_descriptor->kind != FFI::CssDescriptorResultKind::CounterStyleName || counter_style_name_descriptor->items.size() != 1)
                        return nullptr;

                    auto counter_style_name = FlyString::from_utf8_without_validation(counter_style_name_descriptor->items.first().source.bytes());

                    // https://drafts.csswg.org/css-counter-styles-3/#the-counter-style-rule
                    // Counter style names are case-sensitive. However, the names defined in this specification are
                    // ASCII lowercased on parse wherever they are used as counter styles, e.g. in the list-style set
                    // of properties, in the @counter-style rule, and in the counter() functions.
                    //
                    // NB: The "names defined in this specification" are defined in the `CounterStyleNameKeyword` enum
                    auto const& keyword = keyword_from_string(counter_style_name);
                    if (keyword.has_value() && keyword_to_counter_style_name_keyword(keyword.value()).has_value())
                        counter_style_name = counter_style_name.to_ascii_lowercase();

                    return CustomIdentStyleValue::create(counter_style_name);
                }
                case DescriptorMetadata::ValueType::CounterStyleNegative: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-negative
                    // <symbol> <symbol>?
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& negative = descriptor_value->result;
                    if (!negative.has_value() || negative->kind != FFI::CssDescriptorResultKind::CounterStyleNegative)
                        return nullptr;

                    StyleValueVector symbols;
                    for (auto const& item : negative->items) {
                        auto symbol = materialize_descriptor_symbol(item);
                        if (!symbol)
                            return nullptr;
                        symbols.append(symbol.release_nonnull());
                    }

                    if (symbols.size() == 1)
                        return StyleValueList::create(move(symbols), StyleValueList::Separator::Space);
                    return StyleValueList::create(move(symbols), StyleValueList::Separator::Space, StyleValueList::Collapsible::No);
                }
                case DescriptorMetadata::ValueType::CounterStylePad: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-pad
                    // <integer [0,∞]> && <symbol>
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& pad = descriptor_value->result;
                    if (!pad.has_value() || pad->kind != FFI::CssDescriptorResultKind::CounterStylePad || pad->items.size() != 1)
                        return nullptr;

                    auto pair = materialize_descriptor_integer_symbol_pair(pad->items.first());
                    if (!pair)
                        return nullptr;
                    return pair.release_nonnull();
                }
                case DescriptorMetadata::ValueType::CounterStyleRange: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-range
                    // [ [ <integer> | infinite ]{2} ]# | auto
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& range = descriptor_value->result;
                    if (!range.has_value())
                        return nullptr;

                    if (range->kind == FFI::CssDescriptorResultKind::CounterStyleRangeAuto)
                        return KeywordStyleValue::create(Keyword::Auto);
                    if (range->kind != FFI::CssDescriptorResultKind::CounterStyleRangeList)
                        return nullptr;
                    if (range->items.size() % 2 != 0)
                        return nullptr;

                    auto const parse_value = [&](RustComponentValueParser::DescriptorResultItem const& item) -> RefPtr<StyleValue const> {
                        return materialize_descriptor_counter_style_range_bound(item);
                    };

                    auto const resolve_value = [&](StyleValue const& value, i32 infinite_value) -> Optional<i32> {
                        if (value.is_integer())
                            return value.as_integer().integer();

                        if (value.is_keyword() && value.as_keyword().to_keyword() == Keyword::Infinite)
                            return infinite_value;

                        // FIXME: How should we actually handle calc() when we have no document to absolutize against
                        if (!computation_context.has_value())
                            return {};

                        return value.absolutized(computation_context.value())->as_calculated().resolve_integer({}).value();
                    };

                    StyleValueVector range_entries;
                    for (size_t i = 0; i < range->items.size(); i += 2) {
                        auto first_value = parse_value(range->items[i]);
                        auto second_value = parse_value(range->items[i + 1]);

                        if (!first_value || !second_value)
                            return nullptr;

                        // If the lower bound of any range is higher than the upper bound, the entire descriptor is
                        // invalid and must be ignored.
                        auto first_int = resolve_value(*first_value, NumericLimits<i32>::min());
                        auto second_int = resolve_value(*second_value, NumericLimits<i32>::max());

                        if (!first_int.has_value() || !second_int.has_value() || first_int.value() > second_int.value())
                            return nullptr;

                        range_entries.append(StyleValueList::create({ first_value.release_nonnull(), second_value.release_nonnull() }, StyleValueList::Separator::Space, StyleValueList::Collapsible::No));
                    }

                    return StyleValueList::create(move(range_entries), StyleValueList::Separator::Comma, StyleValueList::Collapsible::No);
                }
                case DescriptorMetadata::ValueType::CropOrCross: {
                    // https://drafts.csswg.org/css-page-3/#marks
                    // crop || cross
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& crop_or_cross = descriptor_value->result;
                    if (!crop_or_cross.has_value())
                        return nullptr;

                    switch (crop_or_cross->kind) {
                    case FFI::CssDescriptorResultKind::Crop:
                        return KeywordStyleValue::create(Keyword::Crop);
                    case FFI::CssDescriptorResultKind::Cross:
                        return KeywordStyleValue::create(Keyword::Cross);
                    case FFI::CssDescriptorResultKind::CropAndCross:
                        return StyleValueList::create(StyleValueVector {
                                                          KeywordStyleValue::create(Keyword::Crop),
                                                          KeywordStyleValue::create(Keyword::Cross) },
                            StyleValueList::Separator::Space);
                    default:
                        break;
                    }
                    return nullptr;
                }
                case DescriptorMetadata::ValueType::FamilyName: {
                    // https://drafts.csswg.org/css-fonts-4/#family-name-syntax
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& family_name = descriptor_value->result;
                    if (!family_name.has_value() || family_name->kind != FFI::CssDescriptorResultKind::FamilyName || family_name->items.size() != 1)
                        return nullptr;

                    auto const& parsed_family_name = family_name->items.first();
                    if (parsed_family_name.is_string)
                        return StringStyleValue::create(FlyString::from_utf8_without_validation(parsed_family_name.source.bytes()));
                    return CustomIdentStyleValue::create(FlyString::from_utf8_without_validation(parsed_family_name.source.bytes()));
                }
                case DescriptorMetadata::ValueType::FontSrcList: {
                    // "If a component value is parsed correctly and is of a font format or font tech that the UA
                    // supports, add it to the list of supported sources. If parsing a component value results in a
                    // parsing error or its format or tech are unsupported, do not add it to the list of supported
                    // sources.
                    // If there are no supported entries at the end of this process, the value for the src descriptor
                    // is a parse error.
                    // These parsing rules allow for graceful fallback of fonts for user agents which don’t support a
                    // particular font tech or font format."
                    // https://drafts.csswg.org/css-fonts-4/#font-face-src-parsing
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& source_lists = descriptor_value->result;
                    if (!source_lists.has_value() || source_lists->kind != FFI::CssDescriptorResultKind::FontSrcList)
                        return nullptr;

                    StyleValueVector valid_sources;
                    for (auto const& item : source_lists->items) {
                        // https://drafts.csswg.org/css-fonts/#font-face-src-parsing
                        // <font-src> = <url> [ format(<font-format>)]? [ tech( <font-tech>#)]? | local(<family-name>)
                        if (!item.font_source_kind.has_value())
                            continue;

                        if (item.font_source_format.has_value() && !font_format_is_supported(*item.font_source_format)) {
                            ErrorReporter::the().report(InvalidValueError {
                                .value_type = "<font-src>"_fly_string,
                                .value_string = item.source,
                                .description = MUST(String::formatted("format({}) is not supported.", *item.font_source_format)),
                            });
                            continue;
                        }

                        bool supports_all_tech = true;
                        for (auto font_tech : item.font_source_tech) {
                            if (font_tech_is_supported(font_tech))
                                continue;

                            ErrorReporter::the().report(InvalidValueError {
                                .value_type = "<font-src>"_fly_string,
                                .value_string = item.source,
                                .description = MUST(String::formatted("tech({}) is not supported.", to_string(font_tech))),
                            });
                            supports_all_tech = false;
                            break;
                        }
                        if (!supports_all_tech)
                            continue;

                        Optional<FontSourceStyleValue::Source> source;
                        switch (*item.font_source_kind) {
                        case FFI::CssFontSourceKind::Local: {
                            if (!item.font_source_family_name.has_value())
                                continue;
                            source = item.font_source_family_name->is_string
                                ? FontSourceStyleValue::Source { FontSourceStyleValue::Local { StringStyleValue::create(item.font_source_family_name->name) } }
                                : FontSourceStyleValue::Source { FontSourceStyleValue::Local { CustomIdentStyleValue::create(item.font_source_family_name->name) } };
                            break;
                        }
                        case FFI::CssFontSourceKind::Url:
                            if (!item.url_function_type.has_value() || !item.url.has_value())
                                continue;
                            source = URL { *item.url, *item.url_function_type, item.request_url_modifiers };
                            break;
                        }
                        valid_sources.append(FontSourceStyleValue::create(source.release_value(), item.font_source_format, item.font_source_tech));
                    }
                    if (valid_sources.is_empty())
                        return nullptr;
                    return StyleValueList::create(move(valid_sources), StyleValueList::Separator::Comma);
                }
                case DescriptorMetadata::ValueType::FontWeightAbsolutePair: {
                    // https://drafts.csswg.org/css-fonts-4/#font-prop-desc
                    // <font-weight-absolute>{1,2}
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& weight_sources = descriptor_value->result;
                    if (!weight_sources.has_value() || weight_sources->kind != FFI::CssDescriptorResultKind::FontWeightAbsolutePair)
                        return nullptr;

                    StyleValueVector weights;
                    for (auto const& item : weight_sources->items) {
                        auto weight = materialize_descriptor_font_weight_absolute(item);
                        if (!weight)
                            return nullptr;
                        weights.append(weight.release_nonnull());
                    }

                    return StyleValueList::create(move(weights), StyleValueList::Separator::Space);
                }
                case DescriptorMetadata::ValueType::Length: {
                    // https://drafts.csswg.org/css-values-4/#lengths
                    // <length>
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& length_sources = descriptor_value->result;
                    if (!length_sources.has_value() || length_sources->kind != FFI::CssDescriptorResultKind::Length || length_sources->items.size() != 1)
                        return nullptr;

                    auto const& length_source = length_sources->items.first();
                    if (length_source.primitive_kind == FFI::CssPrimitiveValueKind::Length && length_source.has_numeric_value) {
                        auto length_unit = string_to_length_unit(length_source.source);
                        if (!length_unit.has_value())
                            return nullptr;
                        return LengthStyleValue::create(Length(length_source.numeric_value, *length_unit));
                    }

                    auto length = materialize_descriptor_calculation_tree(length_source, ValueType::Length, infinite_range);
                    if (!length)
                        return nullptr;
                    return length.release_nonnull();
                }
                case DescriptorMetadata::ValueType::OptionalDeclarationValue: {
                    // https://drafts.csswg.org/css-syntax/#typedef-declaration-value
                    // The <declaration-value> production matches any sequence of one or more tokens, so long as the
                    // sequence does not contain <bad-string-token>, <bad-url-token>, unmatched <)-token>, <]-token>,
                    // or <}-token>, or top-level <semicolon-token> tokens or <delim-token> tokens with a value of
                    // "!". It represents the entirety of what a valid declaration can have as its value.
                    //
                    // https://drafts.css-houdini.org/css-properties-values-api/#the-initial-value-descriptor
                    // <declaration-value>?
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& optional_declaration_value = descriptor_value->result;
                    if (!optional_declaration_value.has_value() || optional_declaration_value->kind != FFI::CssDescriptorResultKind::OptionalDeclarationValue)
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> declaration_value_tokens { component_values };
                    declaration_value_tokens.discard_whitespace();
                    if (declaration_value_tokens.is_empty())
                        return UnresolvedStyleValue::create({}, {});

                    // NB: We know this contains no substitution functions otherwise we would have returned earlier.
                    return UnresolvedStyleValue::create(move(component_values), {});
                }
                case DescriptorMetadata::ValueType::PageSize: {
                    // https://drafts.csswg.org/css-page-3/#page-size-prop
                    // <length [0,∞]>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& page_size_descriptor = descriptor_value->result;
                    if (!page_size_descriptor.has_value())
                        return nullptr;

                    // auto
                    if (page_size_descriptor->kind == FFI::CssDescriptorResultKind::PageSizeAuto)
                        return KeywordStyleValue::create(Keyword::Auto);

                    // <length [0,∞]>{1,2}
                    if (page_size_descriptor->kind == FFI::CssDescriptorResultKind::PageSizeLengths) {
                        VERIFY(page_size_descriptor->items.size() == 1 || page_size_descriptor->items.size() == 2);

                        auto const& first_source = page_size_descriptor->items[0];
                        auto first_length = [&]() -> RefPtr<StyleValue const> {
                            if (first_source.primitive_kind == FFI::CssPrimitiveValueKind::Length && first_source.has_numeric_value) {
                                auto length_unit = string_to_length_unit(first_source.source);
                                if (!length_unit.has_value())
                                    return nullptr;
                                return LengthStyleValue::create(Length(first_source.numeric_value, *length_unit));
                            }

                            return materialize_descriptor_calculation_tree(first_source, ValueType::Length, non_negative_range);
                        }();
                        if (!first_length)
                            return nullptr;

                        if (page_size_descriptor->items.size() == 2) {
                            auto const& second_source = page_size_descriptor->items[1];
                            auto second_length = [&]() -> RefPtr<StyleValue const> {
                                if (second_source.primitive_kind == FFI::CssPrimitiveValueKind::Length && second_source.has_numeric_value) {
                                    auto length_unit = string_to_length_unit(second_source.source);
                                    if (!length_unit.has_value())
                                        return nullptr;
                                    return LengthStyleValue::create(Length(second_source.numeric_value, *length_unit));
                                }

                                return materialize_descriptor_calculation_tree(second_source, ValueType::Length, non_negative_range);
                            }();
                            if (!second_length)
                                return nullptr;
                            return StyleValueList::create(StyleValueVector { first_length.release_nonnull(), second_length.release_nonnull() }, StyleValueList::Separator::Space);
                        }

                        return first_length.release_nonnull();
                    }
                    if (page_size_descriptor->kind != FFI::CssDescriptorResultKind::PageSizeAndOrientation)
                        return nullptr;

                    auto page_size_keyword_to_style_value = [](u8 keyword) -> RefPtr<StyleValue const> {
                        switch (keyword) {
                        case 1:
                            return KeywordStyleValue::create(Keyword::A5);
                        case 2:
                            return KeywordStyleValue::create(Keyword::A4);
                        case 3:
                            return KeywordStyleValue::create(Keyword::A3);
                        case 4:
                            return KeywordStyleValue::create(Keyword::B5);
                        case 5:
                            return KeywordStyleValue::create(Keyword::B4);
                        case 6:
                            return KeywordStyleValue::create(Keyword::JisB5);
                        case 7:
                            return KeywordStyleValue::create(Keyword::JisB4);
                        case 8:
                            return KeywordStyleValue::create(Keyword::Letter);
                        case 9:
                            return KeywordStyleValue::create(Keyword::Legal);
                        case 10:
                            return KeywordStyleValue::create(Keyword::Ledger);
                        }
                        return nullptr;
                    };
                    auto orientation_to_style_value = [](u8 orientation) -> RefPtr<StyleValue const> {
                        switch (orientation) {
                        case 1:
                            return KeywordStyleValue::create(Keyword::Portrait);
                        case 2:
                            return KeywordStyleValue::create(Keyword::Landscape);
                        }
                        return nullptr;
                    };

                    // [ <page-size> || [ portrait | landscape ] ]
                    RefPtr<StyleValue const> page_size;
                    RefPtr<StyleValue const> orientation;
                    for (auto const& item : page_size_descriptor->items) {
                        if (item.page_size_keyword != 0) {
                            if (page_size)
                                return nullptr;
                            page_size = page_size_keyword_to_style_value(item.page_size_keyword);
                            if (!page_size)
                                return nullptr;
                            continue;
                        }
                        if (item.page_size_orientation != 0) {
                            if (orientation)
                                return nullptr;
                            orientation = orientation_to_style_value(item.page_size_orientation);
                            if (!orientation)
                                return nullptr;
                            continue;
                        }
                        return nullptr;
                    }

                    if (page_size && orientation) {
                        // Portrait is considered the default orientation, so don't include it.
                        if (orientation->to_keyword() == Keyword::Portrait)
                            return page_size.release_nonnull();

                        return StyleValueList::create(StyleValueVector { page_size.release_nonnull(), orientation.release_nonnull() }, StyleValueList::Separator::Space);
                    }

                    return page_size ? page_size.release_nonnull() : orientation.release_nonnull();
                }
                case DescriptorMetadata::ValueType::PositivePercentage: {
                    // https://drafts.csswg.org/css-values-4/#percentages
                    // <percentage [0,∞]>
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& percentage_sources = descriptor_value->result;
                    if (!percentage_sources.has_value() || percentage_sources->kind != FFI::CssDescriptorResultKind::PositivePercentage || percentage_sources->items.size() != 1)
                        return nullptr;

                    auto const& percentage_source = percentage_sources->items.first();
                    if (percentage_source.primitive_kind == FFI::CssPrimitiveValueKind::Percentage && percentage_source.has_numeric_value)
                        return PercentageStyleValue::create(Percentage(percentage_source.numeric_value));

                    return materialize_descriptor_calculation_tree(percentage_source, ValueType::Percentage, non_negative_range);
                }
                case DescriptorMetadata::ValueType::String: {
                    // https://drafts.csswg.org/css-values-4/#strings
                    // <string>
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& string_sources = descriptor_value->result;
                    if (!string_sources.has_value() || string_sources->kind != FFI::CssDescriptorResultKind::String || string_sources->items.size() != 1)
                        return nullptr;

                    auto const& string_value = string_sources->items.first();
                    return StringStyleValue::create(FlyString::from_utf8_without_validation(string_value.source.bytes()));
                }
                case DescriptorMetadata::ValueType::Symbol: {
                    // https://drafts.csswg.org/css-counter-styles-3/#typedef-symbol
                    // <symbol> = <string> | <image> | <custom-ident>
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& symbol_sources = descriptor_value->result;
                    if (!symbol_sources.has_value() || symbol_sources->kind != FFI::CssDescriptorResultKind::Symbol || symbol_sources->items.size() != 1)
                        return nullptr;

                    auto symbol = materialize_descriptor_symbol(symbol_sources->items.first());
                    return symbol ? symbol.release_nonnull() : nullptr;
                }
                case DescriptorMetadata::ValueType::Symbols: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-symbols
                    // <symbol>+
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& symbol_sources = descriptor_value->result;
                    if (!symbol_sources.has_value() || symbol_sources->kind != FFI::CssDescriptorResultKind::Symbols)
                        return nullptr;

                    StyleValueVector symbols;
                    for (auto const& item : symbol_sources->items) {
                        auto symbol = materialize_descriptor_symbol(item);
                        if (!symbol)
                            return nullptr;
                        symbols.append(symbol.release_nonnull());
                    }

                    return StyleValueList::create(move(symbols), StyleValueList::Separator::Space, StyleValueList::Collapsible::No);
                }
                case DescriptorMetadata::ValueType::UnicodeRangeTokens: {
                    // https://drafts.csswg.org/css-syntax-3/#urange-syntax
                    // <urange>#
                    while (tokens.has_next_token())
                        tokens.discard_a_token();
                    auto const& unicode_range_descriptor = descriptor_value->result;
                    if (!unicode_range_descriptor.has_value() || unicode_range_descriptor->kind != FFI::CssDescriptorResultKind::UnicodeRangeTokens)
                        return nullptr;

                    StyleValueVector unicode_range_values;
                    for (auto const& item : unicode_range_descriptor->items) {
                        if (!item.unicode_range.has_value())
                            return nullptr;
                        unicode_range_values.append(UnicodeRangeStyleValue::create(item.unicode_range.value()));
                    }

                    return StyleValueList::create(move(unicode_range_values), StyleValueList::Separator::Comma);
                }
                }
                return nullptr;
            });
        if (parsed_style_value && !tokens.has_next_token()) {
            transaction.commit();
            return parsed_style_value.release_nonnull();
        }
    }

    ErrorReporter::the().report(InvalidPropertyError {
        .rule_name = to_string(at_rule_id),
        .property_name = descriptor_name_and_id.name(),
        .value_string = tokens.dump_string(),
        .description = "Failed to parse."_string,
    });

    return ParseError::SyntaxError;
}

Optional<Descriptor> Parser::convert_to_descriptor(AtRuleID at_rule_id, Declaration const& declaration)
{
    auto descriptor_name_and_id = DescriptorNameAndID::from_name(at_rule_id, declaration.name);
    if (!descriptor_name_and_id.has_value())
        return {};

    auto value_token_stream = TokenStream(declaration.value);
    auto value = parse_descriptor_value(at_rule_id, descriptor_name_and_id.value(), value_token_stream, declaration.original_value_text);
    if (value.is_error())
        return {};

    return Descriptor { descriptor_name_and_id.value(), value.release_value() };
}

}
