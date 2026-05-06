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
#include <LibWeb/CSS/StyleValues/CounterStyleSystemStyleValue.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/FontSourceStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/UnicodeRangeStyleValue.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>

namespace Web::CSS::Parser {

RefPtr<StyleValue const> Parser::materialize_nonnegative_integer_symbol_pair(ReadonlySpan<ComponentValue const> component_values, FFI::CssNonnegativeIntegerSymbolPairOrder order)
{
    auto pair_component_values = Vector<ComponentValue> { component_values };
    TokenStream<ComponentValue> pair_tokens { pair_component_values };

    RefPtr<StyleValue const> integer;
    RefPtr<StyleValue const> symbol;

    pair_tokens.discard_whitespace();
    switch (order) {
    case FFI::CssNonnegativeIntegerSymbolPairOrder::IntegerFirst:
        integer = parse_integer_value(pair_tokens, non_negative_integer_range);
        pair_tokens.discard_whitespace();
        symbol = parse_symbol_value(pair_tokens);
        break;
    case FFI::CssNonnegativeIntegerSymbolPairOrder::SymbolFirst:
        symbol = parse_symbol_value(pair_tokens);
        pair_tokens.discard_whitespace();
        integer = parse_integer_value(pair_tokens, non_negative_integer_range);
        break;
    }

    if (!integer || !symbol)
        return nullptr;

    pair_tokens.discard_whitespace();
    if (pair_tokens.has_next_token())
        return nullptr;

    return StyleValueList::create({ integer.release_nonnull(), symbol.release_nonnull() }, StyleValueList::Separator::Space);
}

Parser::ParseErrorOr<NonnullRefPtr<StyleValue const>> Parser::parse_descriptor_value(AtRuleID at_rule_id, DescriptorNameAndID const& descriptor_name_and_id, TokenStream<ComponentValue>& tokens)
{
    if (!at_rule_supports_descriptor(at_rule_id, descriptor_name_and_id.id())) {
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

    auto metadata = get_descriptor_metadata(at_rule_id, descriptor_name_and_id.id());

    if (substitution_functions_presence.has_any()) {
        // https://drafts.csswg.org/css-values-5/#resolve-property
        // Unless otherwise specified, arbitrary substitution functions can be used in place of any part of any
        // property’s value (including within other functional notations); and are not valid in any other context.

        // NB: Since we are not in a property value context we only allow ASFs if they are explicitly allowed in
        //     Descriptors.json
        if (!metadata.allow_arbitrary_substitution_functions) {
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

    for (auto const& option : metadata.syntax) {
        auto syntax_transaction = transaction.create_child();
        auto parsed_style_value = option.visit(
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
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_additive_symbols = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto additive_tuple_count = RustComponentValueParser::parse_counter_style_additive_symbols(serialized_additive_symbols.bytes_as_string_view(), "utf-8"sv);
                    if (!additive_tuple_count.has_value())
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> additive_symbol_tokens { component_values };

                    StyleValueVector additive_tuples;
                    for (size_t i = 0; i < *additive_tuple_count; ++i) {
                        additive_symbol_tokens.discard_whitespace();
                        auto additive_tuple_start = additive_symbol_tokens.current_index();
                        while (additive_symbol_tokens.has_next_token() && !additive_symbol_tokens.next_token().is(Token::Type::Comma))
                            additive_symbol_tokens.discard_a_token();

                        auto serialized_tuple = serialize_component_values_for_reparsing(additive_symbol_tokens.tokens_since(additive_tuple_start));
                        auto order = RustComponentValueParser::parse_a_nonnegative_integer_symbol_pair(serialized_tuple.bytes_as_string_view(), "utf-8"sv);
                        if (!order.has_value())
                            return nullptr;

                        auto additive_tuple = materialize_nonnegative_integer_symbol_pair(additive_symbol_tokens.tokens_since(additive_tuple_start), *order);
                        if (!additive_tuple)
                            return nullptr;

                        additive_tuples.append(additive_tuple.release_nonnull());

                        additive_symbol_tokens.discard_whitespace();
                        if (i + 1 < *additive_tuple_count) {
                            if (!additive_symbol_tokens.has_next_token() || !additive_symbol_tokens.consume_a_token().is(Token::Type::Comma))
                                return nullptr;
                        }
                    }

                    additive_symbol_tokens.discard_whitespace();
                    if (additive_symbol_tokens.has_next_token())
                        return nullptr;

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
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_system = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto system = RustComponentValueParser::parse_counter_style_system(serialized_system.bytes_as_string_view(), "utf-8"sv);
                    if (!system.has_value())
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> system_tokens { component_values };

                    auto keyword_value = parse_keyword_value(system_tokens);
                    if (!keyword_value)
                        return nullptr;

                    switch (*system) {
                    case FFI::CssCounterStyleSystemKind::Cyclic:
                    case FFI::CssCounterStyleSystemKind::Numeric:
                    case FFI::CssCounterStyleSystemKind::Alphabetic:
                    case FFI::CssCounterStyleSystemKind::Symbolic:
                    case FFI::CssCounterStyleSystemKind::Additive: {
                        auto counter_style_system = keyword_to_counter_style_system(keyword_value->to_keyword());
                        if (!counter_style_system.has_value())
                            return nullptr;
                        return CounterStyleSystemStyleValue::create(counter_style_system.release_value());
                    }
                    case FFI::CssCounterStyleSystemKind::Fixed:
                        if (keyword_value->to_keyword() != Keyword::Fixed)
                            return nullptr;
                        return CounterStyleSystemStyleValue::create_fixed(nullptr);
                    case FFI::CssCounterStyleSystemKind::FixedWithInteger: {
                        if (keyword_value->to_keyword() != Keyword::Fixed)
                            return nullptr;
                        system_tokens.discard_whitespace();
                        auto integer_value = parse_integer_value(system_tokens, infinite_integer_range);
                        if (!integer_value)
                            return nullptr;
                        system_tokens.discard_whitespace();
                        if (system_tokens.has_next_token())
                            return nullptr;
                        return CounterStyleSystemStyleValue::create_fixed(integer_value);
                    }
                    case FFI::CssCounterStyleSystemKind::Extends: {
                        if (keyword_value->to_keyword() != Keyword::Extends)
                            return nullptr;
                        system_tokens.discard_whitespace();
                        auto counter_style_name = parse_counter_style_name(system_tokens);
                        if (!counter_style_name.has_value())
                            return nullptr;
                        system_tokens.discard_whitespace();
                        if (system_tokens.has_next_token())
                            return nullptr;
                        return CounterStyleSystemStyleValue::create_extends(counter_style_name.release_value());
                    }
                    }
                    VERIFY_NOT_REACHED();
                }
                case DescriptorMetadata::ValueType::CounterStyleName: {
                    // https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style-name
                    // <counter-style-name> is a <custom-ident> that is not an ASCII case-insensitive match for none.
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_counter_style_name = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto counter_style_name = RustComponentValueParser::parse_a_counter_style_name(serialized_counter_style_name.bytes_as_string_view(), "utf-8"sv);

                    if (!counter_style_name.has_value())
                        return nullptr;

                    // https://drafts.csswg.org/css-counter-styles-3/#the-counter-style-rule
                    // Counter style names are case-sensitive. However, the names defined in this specification are
                    // ASCII lowercased on parse wherever they are used as counter styles, e.g. in the list-style set
                    // of properties, in the @counter-style rule, and in the counter() functions.
                    //
                    // NB: The "names defined in this specification" are defined in the `CounterStyleNameKeyword` enum
                    auto const& keyword = keyword_from_string(counter_style_name.value());
                    if (keyword.has_value() && keyword_to_counter_style_name_keyword(keyword.value()).has_value())
                        counter_style_name = counter_style_name->to_ascii_lowercase();

                    return CustomIdentStyleValue::create(counter_style_name.release_value());
                }
                case DescriptorMetadata::ValueType::CounterStyleNegative: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-negative
                    // <symbol> <symbol>?
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_negative = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto count = RustComponentValueParser::parse_counter_style_negative(serialized_negative.bytes_as_string_view(), "utf-8"sv);
                    if (!count.has_value())
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> negative_tokens { component_values };

                    auto first_symbol = parse_symbol_value(negative_tokens);

                    if (!first_symbol)
                        return nullptr;

                    if (*count == FFI::CssCounterStyleNegativeSymbolCount::One)
                        return StyleValueList::create({ first_symbol.release_nonnull() }, StyleValueList::Separator::Space);

                    negative_tokens.discard_whitespace();
                    auto second_symbol = parse_symbol_value(negative_tokens);
                    if (!second_symbol)
                        return nullptr;

                    negative_tokens.discard_whitespace();
                    if (negative_tokens.has_next_token())
                        return nullptr;

                    return StyleValueList::create({ first_symbol.release_nonnull(), second_symbol.release_nonnull() }, StyleValueList::Separator::Space, StyleValueList::Collapsible::No);
                }
                case DescriptorMetadata::ValueType::CounterStylePad: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-pad
                    // <integer [0,∞]> && <symbol>
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_pair = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto order = RustComponentValueParser::parse_a_nonnegative_integer_symbol_pair(serialized_pair.bytes_as_string_view(), "utf-8"sv);
                    if (!order.has_value())
                        return nullptr;

                    return materialize_nonnegative_integer_symbol_pair(tokens.tokens_since(start), *order);
                }
                case DescriptorMetadata::ValueType::CounterStyleRange: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-range
                    // [ [ <integer> | infinite ]{2} ]# | auto
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_range = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto range = RustComponentValueParser::parse_counter_style_range(serialized_range.bytes_as_string_view(), "utf-8"sv);
                    if (!range.has_value())
                        return nullptr;

                    if (range->kind == FFI::CssCounterStyleRangeKind::Auto)
                        return KeywordStyleValue::create(Keyword::Auto);

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> range_tokens { component_values };

                    auto const parse_value = [&]() -> RefPtr<StyleValue const> {
                        if (auto keyword_value = parse_specific_keyword_value(range_tokens, Keyword::Infinite))
                            return keyword_value;

                        if (auto integer_value = parse_integer_value(range_tokens, infinite_integer_range); integer_value)
                            return integer_value;

                        return nullptr;
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
                    for (size_t i = 0; i < range->count; ++i) {
                        range_tokens.discard_whitespace();
                        auto first_value = parse_value();
                        range_tokens.discard_whitespace();
                        auto second_value = parse_value();

                        if (!first_value || !second_value)
                            return nullptr;

                        // If the lower bound of any range is higher than the upper bound, the entire descriptor is
                        // invalid and must be ignored.
                        auto first_int = resolve_value(*first_value, NumericLimits<i32>::min());
                        auto second_int = resolve_value(*second_value, NumericLimits<i32>::max());

                        if (!first_int.has_value() || !second_int.has_value() || first_int.value() > second_int.value())
                            return nullptr;

                        range_entries.append(StyleValueList::create({ first_value.release_nonnull(), second_value.release_nonnull() }, StyleValueList::Separator::Space, StyleValueList::Collapsible::No));

                        range_tokens.discard_whitespace();
                        if (i + 1 < range->count) {
                            if (!range_tokens.has_next_token() || !range_tokens.consume_a_token().is(Token::Type::Comma))
                                return nullptr;
                        }
                    }

                    range_tokens.discard_whitespace();
                    if (range_tokens.has_next_token())
                        return nullptr;

                    return StyleValueList::create(move(range_entries), StyleValueList::Separator::Comma, StyleValueList::Collapsible::No);
                }
                case DescriptorMetadata::ValueType::CropOrCross: {
                    // https://drafts.csswg.org/css-page-3/#marks
                    // crop || cross
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_crop_or_cross = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto crop_or_cross = RustComponentValueParser::parse_crop_or_cross(serialized_crop_or_cross.bytes_as_string_view(), "utf-8"sv);
                    if (!crop_or_cross.has_value())
                        return nullptr;

                    switch (*crop_or_cross) {
                    case FFI::CssCropOrCrossKind::Crop:
                        return KeywordStyleValue::create(Keyword::Crop);
                    case FFI::CssCropOrCrossKind::Cross:
                        return KeywordStyleValue::create(Keyword::Cross);
                    case FFI::CssCropOrCrossKind::CropAndCross:
                        return StyleValueList::create(StyleValueVector {
                                                          KeywordStyleValue::create(Keyword::Crop),
                                                          KeywordStyleValue::create(Keyword::Cross) },
                            StyleValueList::Separator::Space);
                    }
                    VERIFY_NOT_REACHED();
                }
                case DescriptorMetadata::ValueType::FamilyName: {
                    // https://drafts.csswg.org/css-fonts-4/#family-name-syntax
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_family_name = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto family_name = RustComponentValueParser::parse_a_family_name(serialized_family_name.bytes_as_string_view(), "utf-8"sv);
                    if (!family_name.has_value())
                        return nullptr;

                    if (family_name->is_string)
                        return StringStyleValue::create(family_name->name);
                    return CustomIdentStyleValue::create(family_name->name);
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
                    auto source_lists = parse_a_comma_separated_list_of_component_values(tokens);
                    StyleValueVector valid_sources;
                    for (auto const& source_list : source_lists) {
                        // https://drafts.csswg.org/css-fonts/#font-face-src-parsing
                        // <font-src> = <url> [ format(<font-format>)]? [ tech( <font-tech>#)]? | local(<family-name>)
                        auto serialized_font_source = serialize_component_values_for_reparsing(source_list);
                        auto font_source = RustComponentValueParser::parse_a_font_source(serialized_font_source.bytes_as_string_view(), "utf-8"sv);
                        if (!font_source.has_value())
                            continue;

                        if (font_source->format.has_value() && !font_format_is_supported(*font_source->format)) {
                            ErrorReporter::the().report(InvalidValueError {
                                .value_type = "<font-src>"_fly_string,
                                .value_string = serialized_font_source,
                                .description = MUST(String::formatted("format({}) is not supported.", *font_source->format)),
                            });
                            continue;
                        }

                        bool supports_all_tech = true;
                        for (auto font_tech : font_source->tech) {
                            if (font_tech_is_supported(font_tech))
                                continue;

                            ErrorReporter::the().report(InvalidValueError {
                                .value_type = "<font-src>"_fly_string,
                                .value_string = serialized_font_source,
                                .description = MUST(String::formatted("tech({}) is not supported.", to_string(font_tech))),
                            });
                            supports_all_tech = false;
                            break;
                        }
                        if (!supports_all_tech)
                            continue;

                        auto source = font_source->source.visit(
                            [](RustComponentValueParser::FamilyName const& family_name) -> FontSourceStyleValue::Source {
                                if (family_name.is_string)
                                    return FontSourceStyleValue::Local { StringStyleValue::create(family_name.name) };
                                return FontSourceStyleValue::Local { CustomIdentStyleValue::create(family_name.name) };
                            },
                            [](URL const& url) -> FontSourceStyleValue::Source {
                                return url;
                            });
                        valid_sources.append(FontSourceStyleValue::create(move(source), move(font_source->format), move(font_source->tech)));
                    }
                    if (valid_sources.is_empty())
                        return nullptr;
                    return StyleValueList::create(move(valid_sources), StyleValueList::Separator::Comma);
                }
                case DescriptorMetadata::ValueType::FontWeightAbsolutePair: {
                    // https://drafts.csswg.org/css-fonts-4/#font-prop-desc
                    // <font-weight-absolute>{1,2}
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_font_weight_absolute_pair = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto count = RustComponentValueParser::parse_font_weight_absolute_pair(serialized_font_weight_absolute_pair.bytes_as_string_view(), "utf-8"sv);
                    if (!count.has_value())
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> font_weight_absolute_tokens { component_values };

                    auto first = parse_font_weight_absolute_value(font_weight_absolute_tokens);
                    if (!first)
                        return nullptr;

                    if (*count == 1)
                        return StyleValueList::create({ first.release_nonnull() }, StyleValueList::Separator::Space);

                    font_weight_absolute_tokens.discard_whitespace();
                    auto second = parse_font_weight_absolute_value(font_weight_absolute_tokens);
                    if (!second)
                        return nullptr;

                    font_weight_absolute_tokens.discard_whitespace();
                    if (font_weight_absolute_tokens.has_next_token())
                        return nullptr;

                    return StyleValueList::create({ first.release_nonnull(), second.release_nonnull() }, StyleValueList::Separator::Space);
                }
                case DescriptorMetadata::ValueType::Length: {
                    // https://drafts.csswg.org/css-values-4/#lengths
                    // <length>
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_length = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    if (!RustComponentValueParser::parse_length_descriptor(serialized_length.bytes_as_string_view(), "utf-8"sv))
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> length_tokens { component_values };

                    auto length = parse_length_value(length_tokens, infinite_range);
                    if (!length)
                        return nullptr;

                    length_tokens.discard_whitespace();
                    if (length_tokens.has_next_token())
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

                    auto serialized_declaration_value = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    if (!RustComponentValueParser::parse_optional_declaration_value_descriptor(serialized_declaration_value.bytes_as_string_view(), "utf-8"sv))
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> declaration_value_tokens { component_values };

                    declaration_value_tokens.discard_whitespace();
                    if (declaration_value_tokens.is_empty())
                        return UnresolvedStyleValue::create({}, {});

                    if (auto parsed_declaration_value = parse_declaration_value(declaration_value_tokens); parsed_declaration_value.has_value() && declaration_value_tokens.is_empty()) {
                        // NB: We know this contains no substitution functions otherwise we would have returned earlier
                        return UnresolvedStyleValue::create(parsed_declaration_value.release_value(), {});
                    }

                    return nullptr;
                }
                case DescriptorMetadata::ValueType::PageSize: {
                    // https://drafts.csswg.org/css-page-3/#page-size-prop
                    // <length [0,∞]>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_page_size = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    if (!RustComponentValueParser::parse_page_size_descriptor(serialized_page_size.bytes_as_string_view(), "utf-8"sv))
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> page_size_tokens { component_values };

                    // auto
                    if (auto value = parse_all_as_single_keyword_value(page_size_tokens, Keyword::Auto))
                        return value.release_nonnull();

                    // <length [0,∞]>{1,2}
                    if (auto first_length = parse_length_value(page_size_tokens, non_negative_range)) {
                        page_size_tokens.discard_whitespace();

                        if (auto second_length = parse_length_value(page_size_tokens, non_negative_range))
                            return StyleValueList::create(StyleValueVector { first_length.release_nonnull(), second_length.release_nonnull() }, StyleValueList::Separator::Space);

                        return first_length.release_nonnull();
                    }

                    // [ <page-size> || [ portrait | landscape ] ]
                    RefPtr<StyleValue const> page_size;
                    RefPtr<StyleValue const> orientation;
                    if (auto first_keyword = parse_keyword_value(page_size_tokens)) {
                        if (first_is_one_of(first_keyword->to_keyword(), Keyword::Landscape, Keyword::Portrait)) {
                            orientation = first_keyword.release_nonnull();
                        } else if (keyword_to_page_size(first_keyword->to_keyword()).has_value()) {
                            page_size = first_keyword.release_nonnull();
                        } else {
                            return nullptr;
                        }
                    } else {
                        return nullptr;
                    }

                    page_size_tokens.discard_whitespace();

                    if (auto second_keyword = parse_keyword_value(page_size_tokens)) {
                        if (orientation.is_null() && first_is_one_of(second_keyword->to_keyword(), Keyword::Landscape, Keyword::Portrait)) {
                            orientation = second_keyword.release_nonnull();
                        } else if (page_size.is_null() && keyword_to_page_size(second_keyword->to_keyword()).has_value()) {
                            page_size = second_keyword.release_nonnull();
                        } else {
                            return nullptr;
                        }

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
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_percentage = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    if (!RustComponentValueParser::parse_positive_percentage_descriptor(serialized_percentage.bytes_as_string_view(), "utf-8"sv))
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> percentage_tokens { component_values };

                    if (auto percentage_value = parse_percentage_value(percentage_tokens, non_negative_range)) {
                        percentage_tokens.discard_whitespace();
                        if (percentage_tokens.has_next_token())
                            return nullptr;

                        if (percentage_value->is_percentage())
                            return percentage_value.release_nonnull();

                        // FIXME: Support relative lengths within calcs here (i.e. by absolutizing and clamping rather
                        //        than rejecting anything that doesn't resolve at parse time)
                        if (percentage_value->is_calculated()) {
                            auto percentage = percentage_value->as_calculated().resolve_percentage({});
                            if (percentage.has_value() && percentage->value() >= 0)
                                return PercentageStyleValue::create(percentage.release_value());
                            return nullptr;
                        }
                    }
                    return nullptr;
                }
                case DescriptorMetadata::ValueType::String: {
                    // https://drafts.csswg.org/css-values-4/#strings
                    // <string>
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_string = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    if (!RustComponentValueParser::parse_string_descriptor(serialized_string.bytes_as_string_view(), "utf-8"sv))
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> string_tokens { component_values };

                    auto string = parse_string_value(string_tokens);
                    if (!string)
                        return nullptr;

                    string_tokens.discard_whitespace();
                    if (string_tokens.has_next_token())
                        return nullptr;

                    return string.release_nonnull();
                }
                case DescriptorMetadata::ValueType::Symbol: {
                    // https://drafts.csswg.org/css-counter-styles-3/#typedef-symbol
                    // <symbol> = <string> | <image> | <custom-ident>
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_symbol = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    if (!RustComponentValueParser::parse_counter_style_symbol(serialized_symbol.bytes_as_string_view(), "utf-8"sv))
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> symbol_tokens { component_values };

                    auto symbol = parse_symbol_value(symbol_tokens);
                    if (!symbol)
                        return nullptr;

                    symbol_tokens.discard_whitespace();
                    if (symbol_tokens.has_next_token())
                        return nullptr;

                    return symbol.release_nonnull();
                }
                case DescriptorMetadata::ValueType::Symbols: {
                    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-symbols
                    // <symbol>+
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_symbols = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    auto symbol_count = RustComponentValueParser::parse_counter_style_symbols(serialized_symbols.bytes_as_string_view(), "utf-8"sv);
                    if (!symbol_count.has_value())
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> symbol_tokens { component_values };

                    StyleValueVector symbols;
                    for (size_t i = 0; i < *symbol_count; ++i) {
                        symbol_tokens.discard_whitespace();
                        auto symbol = parse_symbol_value(symbol_tokens);
                        if (!symbol)
                            return nullptr;
                        symbols.append(symbol.release_nonnull());
                    }

                    symbol_tokens.discard_whitespace();
                    if (symbol_tokens.has_next_token())
                        return nullptr;

                    return StyleValueList::create(move(symbols), StyleValueList::Separator::Space, StyleValueList::Collapsible::No);
                }
                case DescriptorMetadata::ValueType::UnicodeRangeTokens: {
                    // https://drafts.csswg.org/css-syntax-3/#urange-syntax
                    // <urange>#
                    auto start = tokens.current_index();
                    while (tokens.has_next_token())
                        tokens.discard_a_token();

                    auto serialized_unicode_ranges = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                    if (!RustComponentValueParser::parse_a_unicode_range_list(serialized_unicode_ranges.bytes_as_string_view(), "utf-8"sv).has_value())
                        return nullptr;

                    auto component_values = Vector<ComponentValue> { tokens.tokens_since(start) };
                    TokenStream<ComponentValue> unicode_range_tokens { component_values };

                    auto unicode_ranges = parse_comma_separated_value_list(unicode_range_tokens, [this](auto& tokens) -> RefPtr<StyleValue const> {
                        return parse_unicode_range_value(tokens);
                    });
                    if (!unicode_ranges)
                        return nullptr;

                    unicode_range_tokens.discard_whitespace();
                    if (unicode_range_tokens.has_next_token())
                        return nullptr;

                    return unicode_ranges.release_nonnull();
                }
                }
                return nullptr;
            });
        if (!parsed_style_value || tokens.has_next_token())
            continue;
        syntax_transaction.commit();
        return parsed_style_value.release_nonnull();
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
    auto value = parse_descriptor_value(at_rule_id, descriptor_name_and_id.value(), value_token_stream);
    if (value.is_error())
        return {};

    return Descriptor { descriptor_name_and_id.value(), value.release_value() };
}

}
