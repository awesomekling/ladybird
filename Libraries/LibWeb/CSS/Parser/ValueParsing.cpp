/*
 * Copyright (c) 2018-2024, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2020-2021, the SerenityOS developers.
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2022, MacDue <macdue@dueutil.tech>
 * Copyright (c) 2024, Shannon Booth <shannon@serenityos.org>
 * Copyright (c) 2024, Tommy van der Vorst <tommy@pixelspark.nl>
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>
 * Copyright (c) 2024, Glenn Skrzypczak <glenn.skrzypczak@gmail.com>
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 * Copyright (c) 2025, Jelle Raaijmakers <jelle@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringConversions.h>
#include <AK/TemporaryChange.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/FontFeatureData.h>
#include <LibWeb/CSS/Parser/ArbitrarySubstitutionFunctions.h>
#include <LibWeb/CSS/Parser/ErrorReporter.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/PropertyNameAndID.h>
#include <LibWeb/CSS/StyleValues/AnchorSizeStyleValue.h>
#include <LibWeb/CSS/StyleValues/AnchorStyleValue.h>
#include <LibWeb/CSS/StyleValues/AngleStyleValue.h>
#include <LibWeb/CSS/StyleValues/BackgroundSizeStyleValue.h>
#include <LibWeb/CSS/StyleValues/BasicShapeStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorFunctionStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorInterpolationMethodStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorMixStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterDefinitionsStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleValue.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/EasingStyleValue.h>
#include <LibWeb/CSS/StyleValues/EdgeStyleValue.h>
#include <LibWeb/CSS/StyleValues/FlexStyleValue.h>
#include <LibWeb/CSS/StyleValues/FontStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/FrequencyStyleValue.h>
#include <LibWeb/CSS/StyleValues/FunctionStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTrackSizeListStyleValue.h>
#include <LibWeb/CSS/StyleValues/GuaranteedInvalidStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/LightDarkStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/OpacityValueStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/PositionStyleValue.h>
#include <LibWeb/CSS/StyleValues/RandomValueSharingStyleValue.h>
#include <LibWeb/CSS/StyleValues/RepeatStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/ResolutionStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/SuperellipseStyleValue.h>
#include <LibWeb/CSS/StyleValues/TimeStyleValue.h>
#include <LibWeb/CSS/StyleValues/TransformationStyleValue.h>
#include <LibWeb/CSS/StyleValues/TupleStyleValue.h>
#include <LibWeb/CSS/StyleValues/URLStyleValue.h>
#include <LibWeb/CSS/StyleValues/UnicodeRangeStyleValue.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/Dump.h>
#include <LibWeb/Infra/CharacterTypes.h>

namespace Web::CSS::Parser {

static bool rust_primitive_value_prefix_matches(TokenStream<ComponentValue>& tokens, FFI::CssPrimitiveValueType value_type, FFI::CssPrimitiveValueOptions options = {})
{
    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return false;

    auto const& component_value = tokens.next_token();
    auto serialized_input = Parser::serialize_component_values_for_reparsing({ &component_value, 1 });
    return RustComponentValueParser::parse_primitive_value_prefix(serialized_input.bytes_as_string_view(), "utf-8"sv, value_type, options) != FFI::CssPrimitiveValueKind::Invalid;
}

static bool rust_primitive_value_matches(TokenStream<ComponentValue>& tokens, size_t start, FFI::CssPrimitiveValueType value_type, FFI::CssPrimitiveValueOptions options = {}, Optional<StringView> original_source_text = {})
{
    if (original_source_text.has_value() && !original_source_text->is_empty())
        return RustComponentValueParser::parse_primitive_value(*original_source_text, "utf-8"sv, value_type, options) != FFI::CssPrimitiveValueKind::Invalid;

    auto serialized_input = Parser::serialize_component_values_for_reparsing(tokens.tokens_since(start));
    return RustComponentValueParser::parse_primitive_value(serialized_input.bytes_as_string_view(), "utf-8"sv, value_type, options) != FFI::CssPrimitiveValueKind::Invalid;
}

static void discard_remaining_tokens_if_using_original_source(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    if (!original_source_text.has_value())
        return;

    while (tokens.has_next_token())
        tokens.discard_a_token();
}

static FFI::CssPrimitiveValueOptions primitive_value_options(bool allow_quirky_length = false, bool allow_svg_unitless_length = false, bool allow_svg_unitless_angle = false)
{
    return { allow_quirky_length, false, allow_svg_unitless_length, allow_svg_unitless_angle };
}

RefPtr<StyleValueList const> Parser::parse_comma_separated_value_list(TokenStream<ComponentValue>& tokens, ParseFunction parse_one_value)
{
    tokens.discard_whitespace();
    auto first = parse_one_value(tokens);
    tokens.discard_whitespace();
    if (!first)
        return nullptr;

    StyleValueVector values;
    values.append(first.release_nonnull());

    while (tokens.has_next_token()) {
        if (!tokens.consume_a_token().is(Token::Type::Comma))
            return nullptr;

        tokens.discard_whitespace();

        if (auto maybe_value = parse_one_value(tokens)) {
            values.append(maybe_value.release_nonnull());
            tokens.discard_whitespace();
            continue;
        }
        return nullptr;
    }

    return StyleValueList::create(move(values), StyleValueList::Separator::Comma);
}

// https://drafts.csswg.org/css-syntax/#typedef-declaration-value
Optional<Vector<ComponentValue>> Parser::parse_declaration_value(TokenStream<ComponentValue>& tokens, Optional<Token::Type> end_token_type)
{
    // The <declaration-value> production matches any sequence of one or more tokens, so long as the sequence does not
    // contain <bad-string-token>, <bad-url-token>, unmatched <)-token>, <]-token>, or <}-token>, or top-level
    // <semicolon-token> tokens or <delim-token> tokens with a value of "!". It represents the entirety of what a valid
    // declaration can have as its value.
    Vector<ComponentValue> top_level_declaration_value;

    AK::Function<void(TokenStream<ComponentValue>&, Nested)> const parse_declaration_value_impl = [&](TokenStream<ComponentValue>& current_tokens, Nested nested) {
        auto consume_a_token = [&]() {
            if (nested == Nested::No)
                top_level_declaration_value.append(current_tokens.consume_a_token());
            else
                current_tokens.discard_a_token();
        };

        auto transaction = current_tokens.begin_transaction();
        while (current_tokens.has_next_token()) {
            auto const& peek = current_tokens.next_token();

            if (peek.is_block()) {
                TokenStream block_stream { peek.block().value };
                parse_declaration_value_impl(block_stream, Nested::Yes);
                if (block_stream.is_empty()) {
                    consume_a_token();
                    continue;
                }

                break;
            }

            if (peek.is_function()) {
                TokenStream function_stream { peek.function().value };
                parse_declaration_value_impl(function_stream, Nested::Yes);
                if (function_stream.is_empty()) {
                    consume_a_token();
                    continue;
                }

                break;
            }

            if (!peek.is_token()) {
                consume_a_token();
                continue;
            }

            bool valid = true;
            switch (peek.token().type()) {
            case Token::Type::Invalid:
            case Token::Type::EndOfFile:
            case Token::Type::BadString:
            case Token::Type::BadUrl:
                // NB: We're dealing with ComponentValues, so all valid function and block-related tokens will already be
                //     converted to Function or SimpleBlock ComponentValues. Any remaining ones are invalid.
            case Token::Type::Function:
            case Token::Type::OpenCurly:
            case Token::Type::OpenParen:
            case Token::Type::OpenSquare:
            case Token::Type::CloseCurly:
            case Token::Type::CloseParen:
            case Token::Type::CloseSquare:
                valid = false;
                break;
            case Token::Type::Semicolon:
                valid = nested == Nested::Yes;
                break;
            case Token::Type::Delim:
                valid = nested == Nested::Yes || peek.token().delim() != '!';
                break;
            default:
                valid = nested == Nested::Yes || !end_token_type.has_value() || !peek.is(end_token_type.value());
                break;
            }

            if (!valid)
                break;

            consume_a_token();
        }

        transaction.commit();
    };

    parse_declaration_value_impl(tokens, Nested::No);

    if (top_level_declaration_value.is_empty())
        return OptionalNone {};

    return top_level_declaration_value;
}

RefPtr<StyleValue const> Parser::parse_integer_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Integer))
        return nullptr;

    tokens.discard_whitespace();

    auto const& peek_token = tokens.next_token();
    if (peek_token.is(Token::Type::Number) && peek_token.token().is_integer() && accepted_range.contains(peek_token.token().to_integer())) {
        tokens.discard_a_token(); // integer
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Integer, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return IntegerStyleValue::create(peek_token.token().to_integer());
    }

    if (auto calc = parse_calculated_value(peek_token, { .resolve_numbers_as_integers = true, .accepted_ranges_by_type = { { ValueType::Integer, accepted_range } } }); calc && calc->as_calculated().resolves_to_number()) {
        tokens.discard_a_token(); // calc
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Integer, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return calc;
    }

    if (auto tree_counting_function = parse_tree_counting_function(tokens, TreeCountingFunctionStyleValue::ComputedType::Integer); tree_counting_function) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Integer, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return tree_counting_function;
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_number_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Number))
        return nullptr;

    tokens.discard_whitespace();

    auto const& peek_token = tokens.next_token();
    if (peek_token.is(Token::Type::Number) && accepted_range.contains(peek_token.token().number_value())) {
        tokens.discard_a_token(); // number
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Number, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return NumberStyleValue::create(peek_token.token().number_value());
    }

    if (auto calc = parse_calculated_value(peek_token, { .accepted_ranges_by_type = { { ValueType::Number, accepted_range } } }); calc && calc->as_calculated().resolves_to_number()) {
        tokens.discard_a_token(); // calc
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Number, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return calc;
    }

    if (auto tree_counting_function = parse_tree_counting_function(tokens, TreeCountingFunctionStyleValue::ComputedType::Number); tree_counting_function) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Number, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return tree_counting_function;
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_number_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_number_range, NumericRange const& accepted_percentage_range, Optional<StringView> original_source_text)
{
    // Parses [<percentage> | <number>] (which is equivalent to [<alpha-value>])
    if (auto value = parse_number_value(tokens, accepted_number_range, original_source_text))
        return value;
    if (auto value = parse_percentage_value(tokens, accepted_percentage_range, original_source_text))
        return value;
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_number_percentage_none_value(TokenStream<ComponentValue>& tokens)
{
    // Parses [<percentage> | <number> | none] (which is equivalent to [<alpha-value> | none])
    if (auto value = parse_number_value(tokens, infinite_range))
        return value;
    if (auto value = parse_percentage_value(tokens, infinite_range))
        return value;

    if (tokens.next_token().is_ident("none"sv)) {
        tokens.discard_a_token(); // keyword none
        return KeywordStyleValue::create(Keyword::None);
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Percentage))
        return nullptr;

    tokens.discard_whitespace();

    auto const& peek_token = tokens.next_token();
    if (peek_token.is(Token::Type::Percentage) && accepted_range.contains(peek_token.token().percentage())) {
        tokens.discard_a_token(); // percentage
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Percentage, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return PercentageStyleValue::create(Percentage(peek_token.token().percentage()));
    }

    if (auto calc = parse_calculated_value(peek_token, { .accepted_ranges_by_type = { { ValueType::Percentage, accepted_range } } }); calc && calc->as_calculated().resolves_to_percentage()) {
        tokens.discard_a_token(); // calc
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Percentage, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return calc;
    }

    return nullptr;
}

// https://drafts.csswg.org/css-anchor-position-1/#funcdef-anchor
RefPtr<StyleValue const> Parser::parse_anchor(TokenStream<ComponentValue>& tokens)
{
    // <anchor()> = anchor( <anchor-name>? && <anchor-side>, <length-percentage>? )

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    auto const& function_token = tokens.consume_a_token();
    if (!function_token.is_function("anchor"sv))
        return {};

    auto argument_tokens = TokenStream { function_token.function().value };
    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.function().name });
    Optional<FlyString> anchor_name;
    RefPtr<StyleValue const> anchor_side_value;
    RefPtr<StyleValue const> fallback_value;
    for (auto i = 0; i < 2; ++i) {
        argument_tokens.discard_whitespace();

        // <anchor-name> = <dashed-ident>
        if (auto dashed_ident = parse_dashed_ident(argument_tokens); dashed_ident.has_value()) {
            if (anchor_name.has_value())
                return {};

            anchor_name = dashed_ident.value();
            continue;
        }

        if (anchor_side_value)
            break;

        // <anchor-side> = inside | outside
        //               | top | left | right | bottom
        //               | start | end | self-start | self-end
        //               | <percentage> | center
        anchor_side_value = parse_keyword_value(argument_tokens);
        if (!anchor_side_value) {
            // FIXME: Only percentages are allowed here, but we parse a length-percentage so that calc values are handled.
            anchor_side_value = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range);
            if (!anchor_side_value)
                return {};

            if (anchor_side_value->is_length())
                return {};

        } else if (auto anchor_side_keyword = keyword_to_anchor_side(anchor_side_value->to_keyword()); !anchor_side_keyword.has_value()) {
            return {};
        }
    }
    if (argument_tokens.next_token().is(Token::Type::Comma)) {
        argument_tokens.discard_a_token();
        argument_tokens.discard_whitespace();
        fallback_value = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range);
        if (!fallback_value)
            fallback_value = parse_anchor(argument_tokens);
        if (!fallback_value)
            return {};
    }

    argument_tokens.discard_whitespace();
    if (argument_tokens.has_next_token())
        return {};

    if (!anchor_side_value)
        return {};

    transaction.commit();
    return AnchorStyleValue::create(anchor_name, anchor_side_value.release_nonnull(), fallback_value);
}

// https://drafts.csswg.org/css-anchor-position-1/#sizing
RefPtr<StyleValue const> Parser::parse_anchor_size(TokenStream<ComponentValue>& tokens)
{
    // anchor-size() = anchor-size( [ <anchor-name> || <anchor-size> ]? , <length-percentage>? )

    auto transaction = tokens.begin_transaction();
    auto const& function_token = tokens.consume_a_token();
    if (!function_token.is_function("anchor-size"sv))
        return {};

    // It is only allowed in the accepted @position-try properties (and is otherwise invalid).
    static Array allowed_property_ids = {
        // inset properties
        PropertyID::Inset,
        PropertyID::Top,
        PropertyID::Right,
        PropertyID::Bottom,
        PropertyID::Left,
        PropertyID::InsetBlock,
        PropertyID::InsetBlockStart,
        PropertyID::InsetBlockEnd,
        PropertyID::InsetInline,
        PropertyID::InsetInlineStart,
        PropertyID::InsetInlineEnd,
        // margin properties
        PropertyID::Margin,
        PropertyID::MarginTop,
        PropertyID::MarginRight,
        PropertyID::MarginBottom,
        PropertyID::MarginLeft,
        PropertyID::MarginBlock,
        PropertyID::MarginBlockStart,
        PropertyID::MarginBlockEnd,
        PropertyID::MarginInline,
        PropertyID::MarginInlineStart,
        PropertyID::MarginInlineEnd,
        // sizing properties
        PropertyID::Width,
        PropertyID::MinWidth,
        PropertyID::MaxWidth,
        PropertyID::Height,
        PropertyID::MinHeight,
        PropertyID::MaxHeight,
        PropertyID::BlockSize,
        PropertyID::MinBlockSize,
        PropertyID::MaxBlockSize,
        PropertyID::InlineSize,
        PropertyID::MinInlineSize,
        PropertyID::MaxInlineSize,
        // self-alignment properties
        PropertyID::AlignSelf,
        PropertyID::JustifySelf,
        PropertyID::PlaceSelf,
        // FIXME: position-anchor
        // FIXME: position-area
    };
    bool valid_property_context = false;
    for (auto& value_context : m_value_context) {
        if (!value_context.has<PropertyID>())
            continue;
        if (!allowed_property_ids.contains_slow(value_context.get<PropertyID>())) {
            valid_property_context = false;
            break;
        }
        valid_property_context = true;
    }
    if (!valid_property_context)
        return {};

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.function().name });
    auto argument_tokens = TokenStream { function_token.function().value };

    Optional<FlyString> anchor_name;
    Optional<AnchorSize> anchor_size;
    ValueComparingRefPtr<StyleValue const> fallback_value;

    // Parse optional anchor name and anchor size in arbitrary order.
    for (auto i = 0; i < 2; ++i) {
        argument_tokens.discard_whitespace();
        auto const& peek_token = argument_tokens.next_token();
        if (!peek_token.is(Token::Type::Ident))
            break;

        // <anchor-name> = <dashed-ident>
        if (auto dashed_ident = parse_dashed_ident(argument_tokens); dashed_ident.has_value()) {
            if (anchor_name.has_value())
                return {};
            anchor_name = dashed_ident.value();
            continue;
        }

        // <anchor-size> = width | height | block | inline | self-block | self-inline
        auto keyword = keyword_from_string(peek_token.token().ident());
        if (!keyword.has_value())
            return {};
        auto maybe_anchor_size = keyword_to_anchor_size(keyword.value());
        if (!maybe_anchor_size.has_value() || anchor_size.has_value())
            return {};
        argument_tokens.discard_a_token();
        anchor_size = maybe_anchor_size.release_value();
    }

    argument_tokens.discard_whitespace();
    auto has_name_or_size = anchor_name.has_value() || anchor_size.has_value();
    auto comma_present = false;
    if (argument_tokens.next_token().is(Token::Type::Comma)) {
        if (!has_name_or_size)
            return {};
        comma_present = true;
        argument_tokens.discard_a_token();
        argument_tokens.discard_whitespace();
    }

    // FIXME: Nested anchor sizes should actually be handled by parse_length_percentage()
    if (auto nested_anchor_size = parse_anchor_size(argument_tokens))
        fallback_value = nested_anchor_size.release_nonnull();
    else if (auto length_percentage = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range))
        fallback_value = length_percentage.release_nonnull();

    if (!fallback_value && comma_present)
        return {};
    if (fallback_value && !comma_present && has_name_or_size)
        return {};
    if (argument_tokens.has_next_token())
        return {};

    transaction.commit();
    return AnchorSizeStyleValue::create(anchor_name, anchor_size, fallback_value);
}

static RefPtr<AngleStyleValue const> parse_literal_angle_value(TokenStream<ComponentValue>& tokens, bool is_parsing_svg_presentation_attribute, NumericRange const& accepted_range, Optional<StringView> original_source_text = {})
{
    auto options = primitive_value_options(false, false, is_parsing_svg_presentation_attribute);
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Angle, options))
        return nullptr;

    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto& dimension_token = tokens.consume_a_token().token();
        if (auto angle_type = string_to_angle_unit(dimension_token.dimension_unit()); angle_type.has_value()) {
            Angle angle { dimension_token.dimension_value(), angle_type.release_value() };

            if (!accepted_range.contains(angle.to_degrees()))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Angle, options, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return AngleStyleValue::create(move(angle));
        }
        return nullptr;
    }

    // https://svgwg.org/svg2-draft/types.html#presentation-attribute-css-value
    // When parsing an SVG attribute, an angle is allowed without a unit.
    // FIXME: How should these numbers be interpreted? https://github.com/w3c/svgwg/issues/792
    //        For now: Convert to an angle in degrees.
    if (tokens.next_token().is(Token::Type::Number) && is_parsing_svg_presentation_attribute) {
        auto angle = Angle::make_degrees(tokens.consume_a_token().token().number_value());

        if (!accepted_range.contains(angle.to_degrees()))
            return nullptr;

        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Angle, options, original_source_text))
            return nullptr;

        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return AngleStyleValue::create(move(angle));
    }

    return nullptr;
}

static RefPtr<PercentageStyleValue const> parse_literal_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text = {})
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Percentage))
        return nullptr;

    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Percentage) && accepted_range.contains(tokens.next_token().token().percentage())) {
        auto value = Percentage { tokens.consume_a_token().token().percentage() };
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Percentage, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return PercentageStyleValue::create(value);
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_angle_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    if (auto literal_angle = parse_literal_angle_value(tokens, is_parsing_svg_presentation_attribute(), accepted_range, original_source_text))
        return literal_angle;

    auto start = tokens.current_index();
    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Angle, accepted_range } } }); calc && calc->as_calculated().resolves_to_angle()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Angle, primitive_value_options(false, false, is_parsing_svg_presentation_attribute()), original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_angle_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_angle_range, NumericRange const& accepted_percentage_range, Optional<StringView> original_source_text)
{
    if (auto literal_angle = parse_literal_angle_value(tokens, is_parsing_svg_presentation_attribute(), accepted_angle_range, original_source_text))
        return literal_angle;

    if (auto literal_percentage = parse_literal_percentage_value(tokens, accepted_percentage_range, original_source_text))
        return literal_percentage;

    auto start = tokens.current_index();
    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .percentages_resolve_as = ValueType::Angle, .accepted_ranges_by_type = { { ValueType::Angle, { accepted_angle_range } } } }); calc && calc->as_calculated().resolves_to_angle_percentage()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Angle, primitive_value_options(false, false, is_parsing_svg_presentation_attribute()), original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_flex_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Flex))
        return nullptr;

    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto& dimension_token = tokens.consume_a_token().token();
        if (auto flex_type = string_to_flex_unit(dimension_token.dimension_unit()); flex_type.has_value()) {
            Flex flex { (dimension_token.dimension_value()), flex_type.release_value() };

            if (!accepted_range.contains(flex.to_fr()))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Flex, {}, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return FlexStyleValue::create(move(flex));
        }
        return nullptr;
    }

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Flex, accepted_range } } }); calc && calc->as_calculated().resolves_to_flex()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Flex, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

static RefPtr<FrequencyStyleValue const> parse_literal_frequency_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text = {})
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Frequency))
        return nullptr;

    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto& dimension_token = tokens.consume_a_token().token();
        if (auto frequency_type = string_to_frequency_unit(dimension_token.dimension_unit()); frequency_type.has_value()) {
            Frequency frequency { dimension_token.dimension_value(), frequency_type.release_value() };

            if (!accepted_range.contains(frequency.to_hertz()))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Frequency, {}, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return FrequencyStyleValue::create(move(frequency));
        }
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_frequency_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    if (auto literal_frequency = parse_literal_frequency_value(tokens, accepted_range, original_source_text))
        return literal_frequency;

    auto start = tokens.current_index();
    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Frequency, accepted_range } } }); calc && calc->as_calculated().resolves_to_frequency()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Frequency, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_frequency_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_frequency_range, NumericRange const& accepted_percentage_range, Optional<StringView> original_source_text)
{
    if (auto literal_frequency = parse_literal_frequency_value(tokens, accepted_frequency_range, original_source_text))
        return literal_frequency;

    if (auto literal_percentage = parse_literal_percentage_value(tokens, accepted_percentage_range, original_source_text))
        return literal_percentage;

    auto start = tokens.current_index();
    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .percentages_resolve_as = ValueType::Frequency, .accepted_ranges_by_type = { { ValueType::Frequency, accepted_frequency_range } } }); calc && calc->as_calculated().resolves_to_frequency_percentage()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Frequency, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

static RefPtr<LengthStyleValue const> parse_literal_length_value(TokenStream<ComponentValue>& tokens, bool context_allows_quirky_length, bool is_parsing_svg_presentation_attribute, NumericRange const& accepted_range, Optional<StringView> original_source_text = {})
{
    auto options = primitive_value_options(context_allows_quirky_length, is_parsing_svg_presentation_attribute);
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Length, options))
        return nullptr;

    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto const& dimension_token = tokens.consume_a_token().token();
        if (auto length_type = string_to_length_unit(dimension_token.dimension_unit()); length_type.has_value()) {
            Length length { dimension_token.dimension_value(), length_type.release_value() };

            // NB: Since we can't convert font/viewport relative lengths to their canonical units at parse time it
            //     doesn't make sense to have non-zero/non-infinite bounds for lengths
            VERIFY(accepted_range.min == AK::NumericLimits<float>::lowest() || accepted_range.min == 0);
            VERIFY(accepted_range.max == AK::NumericLimits<float>::max() || accepted_range.max == 0);

            if (!accepted_range.contains(length.raw_value()))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Length, options, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return LengthStyleValue::create(length);
        }
        return nullptr;
    }

    if (tokens.next_token().is(Token::Type::Number)) {
        auto transaction = tokens.begin_transaction();
        auto numeric_value = tokens.consume_a_token().token().number_value();
        if (numeric_value == 0) {
            if (!accepted_range.contains(0))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Length, options, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return LengthStyleValue::create(Length::make_px(0));
        }
        if (context_allows_quirky_length) {
            auto nearest_value = CSSPixels::nearest_value_for(numeric_value);

            if (!accepted_range.contains(nearest_value.to_double()))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Length, options, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return LengthStyleValue::create(Length::make_px(nearest_value));
        }

        // https://svgwg.org/svg2-draft/types.html#presentation-attribute-css-value
        // When parsing an SVG attribute, a length is allowed without a unit.
        // FIXME: How should these numbers be interpreted? https://github.com/w3c/svgwg/issues/792
        //        For now: Convert to a length in pixels.
        if (is_parsing_svg_presentation_attribute) {
            auto nearest_value = CSSPixels::nearest_value_for(numeric_value);

            if (!accepted_range.contains(nearest_value.to_double()))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Length, options, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return LengthStyleValue::create(Length::make_px(nearest_value));
        }
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_length_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    if (auto literal_length = parse_literal_length_value(tokens, context_allows_quirky_length(), is_parsing_svg_presentation_attribute(), accepted_range, original_source_text))
        return literal_length;

    auto options = primitive_value_options(context_allows_quirky_length(), is_parsing_svg_presentation_attribute());
    auto start = tokens.current_index();
    if (tokens.next_token().is_function("anchor-size"sv)) {
        if (auto anchor_size = parse_anchor_size(tokens); anchor_size && rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Length, options, original_source_text)) {
            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            return anchor_size;
        }
        return nullptr;
    }

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Length, accepted_range } } }); calc && calc->as_calculated().resolves_to_length()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Length, options, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_length_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_length_range, NumericRange const& accepted_percentage_range, Optional<StringView> original_source_text)
{
    if (auto literal_length = parse_literal_length_value(tokens, context_allows_quirky_length(), is_parsing_svg_presentation_attribute(), accepted_length_range, original_source_text))
        return literal_length;

    if (auto literal_percentage = parse_literal_percentage_value(tokens, accepted_percentage_range, original_source_text))
        return literal_percentage;

    auto options = primitive_value_options(context_allows_quirky_length(), is_parsing_svg_presentation_attribute());
    auto start = tokens.current_index();
    if (tokens.next_token().is_function("anchor-size"sv)) {
        if (auto anchor_size = parse_anchor_size(tokens); anchor_size && rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Length, options, original_source_text)) {
            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            return anchor_size;
        }
        return nullptr;
    }

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .percentages_resolve_as = ValueType::Length, .accepted_ranges_by_type = { { ValueType::Length, accepted_length_range } } }); calc && calc->as_calculated().resolves_to_length_percentage()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Length, options, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_resolution_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Resolution))
        return nullptr;

    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto& dimension_token = tokens.consume_a_token().token();
        if (auto resolution_type = string_to_resolution_unit(dimension_token.dimension_unit()); resolution_type.has_value()) {
            Resolution resolution { dimension_token.dimension_value(), resolution_type.release_value() };

            // The allowed range of <resolution> values always excludes negative values, in addition to any explicit
            // ranges that might be specified.
            // https://drafts.csswg.org/css-values-4/#resolution
            if (dimension_token.dimension_value() < 0 || !accepted_range.contains(resolution.to_dots_per_pixel()))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Resolution, {}, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return ResolutionStyleValue::create(move(resolution));
        }
        return nullptr;
    }

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Resolution, accepted_range } } }); calc && calc->as_calculated().resolves_to_resolution()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Resolution, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

static RefPtr<TimeStyleValue const> parse_literal_time_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text = {})
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Time))
        return nullptr;

    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto const& dimension_token = tokens.consume_a_token().token();
        if (auto time_type = string_to_time_unit(dimension_token.dimension_unit()); time_type.has_value()) {
            Time time { dimension_token.dimension_value(), time_type.release_value() };

            if (!accepted_range.contains(time.to_seconds()))
                return nullptr;

            if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Time, {}, original_source_text))
                return nullptr;

            discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
            transaction.commit();
            return TimeStyleValue::create(move(time));
        }
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_time_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range, Optional<StringView> original_source_text)
{
    if (auto literal_time = parse_literal_time_value(tokens, accepted_range, original_source_text))
        return literal_time;

    auto start = tokens.current_index();
    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Time, accepted_range } } }); calc && calc->as_calculated().resolves_to_time()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Time, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_time_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_time_range, NumericRange const& accepted_percentage_range, Optional<StringView> original_source_text)
{
    if (auto literal_time = parse_literal_time_value(tokens, accepted_time_range, original_source_text))
        return literal_time;

    if (auto literal_percentage = parse_literal_percentage_value(tokens, accepted_percentage_range, original_source_text))
        return literal_percentage;

    auto start = tokens.current_index();
    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .percentages_resolve_as = ValueType::Time, .accepted_ranges_by_type = { { ValueType::Time, accepted_time_range } } }); calc && calc->as_calculated().resolves_to_time_percentage()) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Time, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_keyword_value(TokenStream<ComponentValue>& tokens)
{
    tokens.discard_whitespace();
    auto const& peek_token = tokens.next_token();
    if (peek_token.is(Token::Type::Ident)) {
        auto keyword = keyword_from_string(peek_token.token().ident());
        if (keyword.has_value()) {
            tokens.discard_a_token(); // ident
            return KeywordStyleValue::create(keyword.value());
        }
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_specific_keyword_value(TokenStream<ComponentValue>& tokens, Keyword keyword)
{
    auto transaction = tokens.begin_transaction();

    if (auto keyword_value = parse_keyword_value(tokens); keyword_value && keyword_value->to_keyword() == keyword) {
        transaction.commit();
        return keyword_value;
    }

    return nullptr;
}

// https://drafts.csswg.org/scroll-animations-1/#funcdef-scroll
RefPtr<FunctionStyleValue const> Parser::parse_scroll_function_value(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    // <scroll()> = scroll( [ <scroller> || <axis> ]? )
    auto transaction = tokens.begin_transaction();
    auto const& function_token = tokens.consume_a_token();
    if (!function_token.is_function("scroll"sv))
        return nullptr;

    Optional<String> serialized_scroll_function;
    auto scroll_function_source = original_source_text.value_or_lazy_evaluated([&] {
        serialized_scroll_function = function_token.original_source_text();
        if (serialized_scroll_function->is_empty())
            serialized_scroll_function = function_token.to_string();
        return serialized_scroll_function->bytes_as_string_view();
    });
    auto scroll_function = RustComponentValueParser::parse_style_value_for_value_type(PropertyID::AnimationTimeline, ValueType::ScrollFunction, scroll_function_source);
    if (!scroll_function.has_value() || scroll_function->kind != FFI::CssStyleValueKind::ScrollFunction)
        return nullptr;

    StyleValueTuple tuple;
    tuple.resize_with_default_value(2, nullptr);

    switch (scroll_function->scroll_function_scroller) {
    case FFI::CssScrollFunctionScrollerKind::None:
    case FFI::CssScrollFunctionScrollerKind::Nearest:
        break;
    case FFI::CssScrollFunctionScrollerKind::Root:
        tuple[TupleStyleValue::Indices::ScrollFunction::Scroller] = KeywordStyleValue::create(Keyword::Root);
        break;
    case FFI::CssScrollFunctionScrollerKind::Self_:
        tuple[TupleStyleValue::Indices::ScrollFunction::Scroller] = KeywordStyleValue::create(Keyword::Self);
        break;
    }

    switch (scroll_function->scroll_function_axis) {
    case FFI::CssScrollFunctionAxisKind::None:
    case FFI::CssScrollFunctionAxisKind::Block:
        break;
    case FFI::CssScrollFunctionAxisKind::Inline:
        tuple[TupleStyleValue::Indices::ScrollFunction::Axis] = KeywordStyleValue::create(Keyword::Inline);
        break;
    case FFI::CssScrollFunctionAxisKind::X:
        tuple[TupleStyleValue::Indices::ScrollFunction::Axis] = KeywordStyleValue::create(Keyword::X);
        break;
    case FFI::CssScrollFunctionAxisKind::Y:
        tuple[TupleStyleValue::Indices::ScrollFunction::Axis] = KeywordStyleValue::create(Keyword::Y);
        break;
    }

    if (original_source_text.has_value()) {
        while (tokens.has_next_token())
            tokens.discard_a_token();
    }

    transaction.commit();
    return FunctionStyleValue::create("scroll"_fly_string, TupleStyleValue::create(move(tuple)));
}

// https://drafts.csswg.org/scroll-animations-1/#funcdef-view
RefPtr<FunctionStyleValue const> Parser::parse_view_function_value(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    // <view()> = view( [ <axis> || <'view-timeline-inset'> ]? )
    auto transaction = tokens.begin_transaction();
    auto const& function_token = tokens.consume_a_token();
    if (!function_token.is_function("view"sv))
        return nullptr;

    Optional<String> serialized_view_function;
    auto view_function_source = original_source_text.value_or_lazy_evaluated([&] {
        serialized_view_function = function_token.original_source_text();
        if (serialized_view_function->is_empty())
            serialized_view_function = function_token.to_string();
        return serialized_view_function->bytes_as_string_view();
    });
    auto view_function = RustComponentValueParser::parse_style_value_for_value_type(PropertyID::AnimationTimeline, ValueType::ViewFunction, view_function_source);
    if (!view_function.has_value() || view_function->kind != FFI::CssStyleValueKind::ViewFunction)
        return nullptr;

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "view"sv });

    StyleValueTuple tuple;
    tuple.resize_with_default_value(2, nullptr);

    switch (view_function->scroll_function_axis) {
    case FFI::CssScrollFunctionAxisKind::None:
    case FFI::CssScrollFunctionAxisKind::Block:
        break;
    case FFI::CssScrollFunctionAxisKind::Inline:
        tuple[TupleStyleValue::Indices::ViewFunction::Axis] = KeywordStyleValue::create(Keyword::Inline);
        break;
    case FFI::CssScrollFunctionAxisKind::X:
        tuple[TupleStyleValue::Indices::ViewFunction::Axis] = KeywordStyleValue::create(Keyword::X);
        break;
    case FFI::CssScrollFunctionAxisKind::Y:
        tuple[TupleStyleValue::Indices::ViewFunction::Axis] = KeywordStyleValue::create(Keyword::Y);
        break;
    }

    switch (view_function->view_function_inset) {
    case FFI::CssViewFunctionInsetKind::None:
    case FFI::CssViewFunctionInsetKind::Default:
        break;
    case FFI::CssViewFunctionInsetKind::NonDefault: {
        auto argument_tokens = TokenStream { function_token.function().value };
        if (view_function->view_function_inset_position == FFI::CssViewFunctionInsetPosition::AfterAxis) {
            argument_tokens.discard_whitespace();
            argument_tokens.discard_a_token();
        }

        auto inset_value = parse_rust_owned_property_value_prefix(PropertyID::ViewTimelineInset, argument_tokens);
        if (!inset_value)
            return nullptr;

        tuple[TupleStyleValue::Indices::ViewFunction::Inset] = inset_value.release_nonnull();
        break;
    }
    }

    if (original_source_text.has_value()) {
        while (tokens.has_next_token())
            tokens.discard_a_token();
    }

    transaction.commit();
    return FunctionStyleValue::create("view"_fly_string, TupleStyleValue::create(move(tuple)));
}

// https://www.w3.org/TR/css-color-4/#typedef-hue
RefPtr<StyleValue const> Parser::parse_hue_none_value(TokenStream<ComponentValue>& tokens)
{
    // Parses [<hue> | none]
    //   <hue> = <number> | <angle>

    if (auto angle = parse_angle_value(tokens, infinite_range))
        return angle;
    if (auto number = parse_number_value(tokens, infinite_range))
        return number;
    if (tokens.next_token().is_ident("none"sv)) {
        tokens.discard_a_token(); // keyword none
        return KeywordStyleValue::create(Keyword::None);
    }

    return nullptr;
}

// https://www.w3.org/TR/css-color-4/#typedef-color-alpha-value
RefPtr<StyleValue const> Parser::parse_solidus_and_alpha_value(TokenStream<ComponentValue>& tokens)
{
    // [ / [<alpha-value> | none] ]?
    // <alpha-value> = <number> | <percentage>
    // Common to the modern-syntax color functions.

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    if (!tokens.consume_a_token().is_delim('/'))
        return {};
    tokens.discard_whitespace();
    auto alpha = parse_number_percentage_none_value(tokens);
    if (!alpha)
        return {};
    tokens.discard_whitespace();

    transaction.commit();
    return alpha;
}

// https://www.w3.org/TR/css-color-4/#funcdef-rgb
RefPtr<StyleValue const> Parser::parse_rgb_color_value(TokenStream<ComponentValue>& outer_tokens)
{
    // rgb() = [ <legacy-rgb-syntax> | <modern-rgb-syntax> ]
    // rgba() = [ <legacy-rgba-syntax> | <modern-rgba-syntax> ]
    // <legacy-rgb-syntax> = rgb( <percentage>#{3} , <alpha-value>? ) |
    //                       rgb( <number>#{3} , <alpha-value>? )
    // <legacy-rgba-syntax> = rgba( <percentage>#{3} , <alpha-value>? ) |
    //                        rgba( <number>#{3} , <alpha-value>? )
    // <modern-rgb-syntax> = rgb(
    //     [ <number> | <percentage> | none]{3}
    //     [ / [<alpha-value> | none] ]?  )
    // <modern-rgba-syntax> = rgba(
    //     [ <number> | <percentage> | none]{3}
    //     [ / [<alpha-value> | none] ]?  )

    auto transaction = outer_tokens.begin_transaction();
    outer_tokens.discard_whitespace();

    auto& function_token = outer_tokens.consume_a_token();
    if (!function_token.is_function("rgb"sv) && !function_token.is_function("rgba"sv))
        return {};

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.function().name });

    RefPtr<StyleValue const> red;
    RefPtr<StyleValue const> green;
    RefPtr<StyleValue const> blue;
    RefPtr<StyleValue const> alpha;

    auto inner_tokens = TokenStream { function_token.function().value };
    inner_tokens.discard_whitespace();

    red = parse_number_percentage_none_value(inner_tokens);
    if (!red)
        return {};

    inner_tokens.discard_whitespace();
    bool legacy_syntax = inner_tokens.next_token().is(Token::Type::Comma);
    if (legacy_syntax) {
        // Legacy syntax
        //   <percentage>#{3} , <alpha-value>?
        //   | <number>#{3} , <alpha-value>?
        // So, r/g/b can be numbers or percentages, as long as they're all the same type.

        // We accepted the 'none' keyword when parsing the red value, but it's not allowed in the legacy syntax.
        if (red->is_keyword())
            return {};

        inner_tokens.discard_a_token(); // comma
        inner_tokens.discard_whitespace();

        green = parse_number_percentage_value(inner_tokens, infinite_range, infinite_range);
        if (!green)
            return {};
        inner_tokens.discard_whitespace();

        if (!inner_tokens.consume_a_token().is(Token::Type::Comma))
            return {};
        inner_tokens.discard_whitespace();

        blue = parse_number_percentage_value(inner_tokens, infinite_range, infinite_range);
        if (!blue)
            return {};
        inner_tokens.discard_whitespace();

        if (inner_tokens.has_next_token()) {
            // Try and read comma and alpha
            if (!inner_tokens.consume_a_token().is(Token::Type::Comma))
                return {};
            inner_tokens.discard_whitespace();

            alpha = parse_number_percentage_value(inner_tokens, infinite_range, infinite_range);

            if (!alpha)
                return {};

            inner_tokens.discard_whitespace();

            if (inner_tokens.has_next_token())
                return {};
        }

        // Verify we're all percentages or all numbers
        auto is_percentage = [](StyleValue const& style_value) {
            return style_value.is_percentage()
                || (style_value.is_calculated() && style_value.as_calculated().resolves_to_percentage());
        };
        bool red_is_percentage = is_percentage(*red);
        bool green_is_percentage = is_percentage(*green);
        bool blue_is_percentage = is_percentage(*blue);
        if (red_is_percentage != green_is_percentage || red_is_percentage != blue_is_percentage)
            return {};

    } else {
        // Modern syntax
        //   [ <number> | <percentage> | none]{3}  [ / [<alpha-value> | none] ]?

        green = parse_number_percentage_none_value(inner_tokens);
        if (!green)
            return {};
        inner_tokens.discard_whitespace();

        blue = parse_number_percentage_none_value(inner_tokens);
        if (!blue)
            return {};
        inner_tokens.discard_whitespace();

        if (inner_tokens.has_next_token()) {
            alpha = parse_solidus_and_alpha_value(inner_tokens);
            if (!alpha || inner_tokens.has_next_token())
                return {};
        }
    }

    if (!alpha)
        alpha = NumberStyleValue::create(1);

    transaction.commit();
    return ColorFunctionStyleValue::create(ColorStyleValue::ColorType::RGB, red.release_nonnull(), green.release_nonnull(), blue.release_nonnull(), alpha.release_nonnull(), legacy_syntax ? ColorSyntax::Legacy : ColorSyntax::Modern);
}

// https://www.w3.org/TR/css-color-4/#funcdef-hsl
RefPtr<StyleValue const> Parser::parse_hsl_color_value(TokenStream<ComponentValue>& outer_tokens)
{
    // hsl() = [ <legacy-hsl-syntax> | <modern-hsl-syntax> ]
    // hsla() = [ <legacy-hsla-syntax> | <modern-hsla-syntax> ]
    // <modern-hsl-syntax> = hsl(
    //     [<hue> | none]
    //     [<percentage> | <number> | none]
    //     [<percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )
    // <modern-hsla-syntax> = hsla(
    //     [<hue> | none]
    //     [<percentage> | <number> | none]
    //     [<percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )
    // <legacy-hsl-syntax> = hsl( <hue>, <percentage>, <percentage>, <alpha-value>? )
    // <legacy-hsla-syntax> = hsla( <hue>, <percentage>, <percentage>, <alpha-value>? )

    auto transaction = outer_tokens.begin_transaction();
    outer_tokens.discard_whitespace();

    auto& function_token = outer_tokens.consume_a_token();
    if (!function_token.is_function("hsl"sv) && !function_token.is_function("hsla"sv))
        return {};

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.function().name });

    RefPtr<StyleValue const> h;
    RefPtr<StyleValue const> s;
    RefPtr<StyleValue const> l;
    RefPtr<StyleValue const> alpha;

    auto inner_tokens = TokenStream { function_token.function().value };
    inner_tokens.discard_whitespace();

    h = parse_hue_none_value(inner_tokens);
    if (!h)
        return {};

    inner_tokens.discard_whitespace();
    bool legacy_syntax = inner_tokens.next_token().is(Token::Type::Comma);
    if (legacy_syntax) {
        // Legacy syntax
        //   <hue>, <percentage>, <percentage>, <alpha-value>?

        // We accepted the 'none' keyword when parsing the h value, but it's not allowed in the legacy syntax.
        if (h->is_keyword())
            return {};

        (void)inner_tokens.consume_a_token(); // comma
        inner_tokens.discard_whitespace();

        s = parse_percentage_value(inner_tokens, infinite_range);
        if (!s)
            return {};
        inner_tokens.discard_whitespace();

        if (!inner_tokens.consume_a_token().is(Token::Type::Comma))
            return {};
        inner_tokens.discard_whitespace();

        l = parse_percentage_value(inner_tokens, infinite_range);
        if (!l)
            return {};
        inner_tokens.discard_whitespace();

        if (inner_tokens.has_next_token()) {
            // Try and read comma and alpha
            if (!inner_tokens.consume_a_token().is(Token::Type::Comma))
                return {};
            inner_tokens.discard_whitespace();

            alpha = parse_number_percentage_value(inner_tokens, infinite_range, infinite_range);
            // The parser has consumed a comma, so the alpha value is now required
            if (!alpha)
                return {};
            inner_tokens.discard_whitespace();

            if (inner_tokens.has_next_token())
                return {};
        }
    } else {
        // Modern syntax
        //   [<hue> | none]
        //   [<percentage> | <number> | none]
        //   [<percentage> | <number> | none]
        //   [ / [<alpha-value> | none] ]?

        s = parse_number_percentage_none_value(inner_tokens);
        if (!s)
            return {};
        inner_tokens.discard_whitespace();

        l = parse_number_percentage_none_value(inner_tokens);
        if (!l)
            return {};
        inner_tokens.discard_whitespace();

        if (inner_tokens.has_next_token()) {
            alpha = parse_solidus_and_alpha_value(inner_tokens);
            if (!alpha || inner_tokens.has_next_token())
                return {};
        }
    }

    if (!alpha)
        alpha = NumberStyleValue::create(1);

    transaction.commit();
    return ColorFunctionStyleValue::create(ColorStyleValue::ColorType::HSL, h.release_nonnull(), s.release_nonnull(), l.release_nonnull(), alpha.release_nonnull(), legacy_syntax ? ColorSyntax::Legacy : ColorSyntax::Modern);
}

// https://www.w3.org/TR/css-color-4/#funcdef-hwb
RefPtr<StyleValue const> Parser::parse_hwb_color_value(TokenStream<ComponentValue>& outer_tokens)
{
    // hwb() = hwb(
    //     [<hue> | none]
    //     [<percentage> | <number> | none]
    //     [<percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )

    auto transaction = outer_tokens.begin_transaction();
    outer_tokens.discard_whitespace();

    auto& function_token = outer_tokens.consume_a_token();
    if (!function_token.is_function("hwb"sv))
        return {};

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.function().name });

    RefPtr<StyleValue const> h;
    RefPtr<StyleValue const> w;
    RefPtr<StyleValue const> b;
    RefPtr<StyleValue const> alpha;

    auto inner_tokens = TokenStream { function_token.function().value };
    inner_tokens.discard_whitespace();

    h = parse_hue_none_value(inner_tokens);
    if (!h)
        return {};
    inner_tokens.discard_whitespace();

    w = parse_number_percentage_none_value(inner_tokens);
    if (!w)
        return {};
    inner_tokens.discard_whitespace();

    b = parse_number_percentage_none_value(inner_tokens);
    if (!b)
        return {};
    inner_tokens.discard_whitespace();

    if (inner_tokens.has_next_token()) {
        alpha = parse_solidus_and_alpha_value(inner_tokens);
        if (!alpha || inner_tokens.has_next_token())
            return {};
    }

    if (!alpha)
        alpha = NumberStyleValue::create(1);

    transaction.commit();
    return ColorFunctionStyleValue::create(ColorStyleValue::ColorType::HWB, h.release_nonnull(), w.release_nonnull(), b.release_nonnull(), alpha.release_nonnull());
}

Optional<Array<RefPtr<StyleValue const>, 4>> Parser::parse_lab_like_color_value(TokenStream<ComponentValue>& outer_tokens, StringView function_name)
{
    // This helper is designed to be compatible with lab and oklab and parses a function with a form like:
    // f() = f( [ <percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )

    auto transaction = outer_tokens.begin_transaction();
    outer_tokens.discard_whitespace();

    auto& function_token = outer_tokens.consume_a_token();
    if (!function_token.is_function(function_name))
        return OptionalNone {};

    RefPtr<StyleValue const> l;
    RefPtr<StyleValue const> a;
    RefPtr<StyleValue const> b;
    RefPtr<StyleValue const> alpha;

    auto inner_tokens = TokenStream { function_token.function().value };
    inner_tokens.discard_whitespace();

    l = parse_number_percentage_none_value(inner_tokens);
    if (!l)
        return OptionalNone {};
    inner_tokens.discard_whitespace();

    a = parse_number_percentage_none_value(inner_tokens);
    if (!a)
        return OptionalNone {};
    inner_tokens.discard_whitespace();

    b = parse_number_percentage_none_value(inner_tokens);
    if (!b)
        return OptionalNone {};
    inner_tokens.discard_whitespace();

    if (inner_tokens.has_next_token()) {
        alpha = parse_solidus_and_alpha_value(inner_tokens);
        if (!alpha || inner_tokens.has_next_token())
            return OptionalNone {};
    }

    if (!alpha)
        alpha = NumberStyleValue::create(1);

    transaction.commit();

    return Array { move(l), move(a), move(b), move(alpha) };
}

// https://www.w3.org/TR/css-color-4/#funcdef-lab
RefPtr<StyleValue const> Parser::parse_lab_color_value(TokenStream<ComponentValue>& outer_tokens)
{
    // lab() = lab( [<percentage> | <number> | none]
    //      [ <percentage> | <number> | none]
    //      [ <percentage> | <number> | none]
    //      [ / [<alpha-value> | none] ]? )

    auto maybe_color_values = parse_lab_like_color_value(outer_tokens, "lab"sv);
    if (!maybe_color_values.has_value())
        return {};

    auto& color_values = *maybe_color_values;

    return ColorFunctionStyleValue::create(ColorStyleValue::ColorType::Lab,
        color_values[0].release_nonnull(),
        color_values[1].release_nonnull(),
        color_values[2].release_nonnull(),
        color_values[3].release_nonnull());
}

// https://www.w3.org/TR/css-color-4/#funcdef-oklab
RefPtr<StyleValue const> Parser::parse_oklab_color_value(TokenStream<ComponentValue>& outer_tokens)
{
    // oklab() = oklab( [ <percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )

    auto maybe_color_values = parse_lab_like_color_value(outer_tokens, "oklab"sv);
    if (!maybe_color_values.has_value())
        return {};

    auto& color_values = *maybe_color_values;

    return ColorFunctionStyleValue::create(ColorStyleValue::ColorType::OKLab,
        color_values[0].release_nonnull(),
        color_values[1].release_nonnull(),
        color_values[2].release_nonnull(),
        color_values[3].release_nonnull());
}

Optional<Array<RefPtr<StyleValue const>, 4>> Parser::parse_lch_like_color_value(TokenStream<ComponentValue>& outer_tokens, StringView function_name)
{
    // This helper is designed to be compatible with lch and oklch and parses a function with a form like:
    // f() = f( [<percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ <hue> | none]
    //     [ / [<alpha-value> | none] ]? )

    auto transaction = outer_tokens.begin_transaction();
    outer_tokens.discard_whitespace();

    auto const& function_token = outer_tokens.consume_a_token();
    if (!function_token.is_function(function_name))
        return OptionalNone {};

    auto inner_tokens = TokenStream { function_token.function().value };
    inner_tokens.discard_whitespace();

    auto l = parse_number_percentage_none_value(inner_tokens);
    if (!l)
        return OptionalNone {};
    inner_tokens.discard_whitespace();

    auto c = parse_number_percentage_none_value(inner_tokens);
    if (!c)
        return OptionalNone {};
    inner_tokens.discard_whitespace();

    auto h = parse_hue_none_value(inner_tokens);
    if (!h)
        return OptionalNone {};
    inner_tokens.discard_whitespace();

    RefPtr<StyleValue const> alpha;
    if (inner_tokens.has_next_token()) {
        alpha = parse_solidus_and_alpha_value(inner_tokens);
        if (!alpha || inner_tokens.has_next_token())
            return OptionalNone {};
    }

    if (!alpha)
        alpha = NumberStyleValue::create(1);

    transaction.commit();

    return Array { move(l), move(c), move(h), move(alpha) };
}

// https://www.w3.org/TR/css-color-4/#funcdef-lch
RefPtr<StyleValue const> Parser::parse_lch_color_value(TokenStream<ComponentValue>& outer_tokens)
{
    // lch() = lch( [<percentage> | <number> | none]
    //      [ <percentage> | <number> | none]
    //      [ <hue> | none]
    //      [ / [<alpha-value> | none] ]? )

    auto maybe_color_values = parse_lch_like_color_value(outer_tokens, "lch"sv);
    if (!maybe_color_values.has_value())
        return {};

    auto& color_values = *maybe_color_values;

    return ColorFunctionStyleValue::create(ColorStyleValue::ColorType::LCH, color_values[0].release_nonnull(),
        color_values[1].release_nonnull(),
        color_values[2].release_nonnull(),
        color_values[3].release_nonnull());
}

// https://www.w3.org/TR/css-color-4/#funcdef-oklch
RefPtr<StyleValue const> Parser::parse_oklch_color_value(TokenStream<ComponentValue>& outer_tokens)
{
    // oklch() = oklch( [ <percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ <hue> | none]
    //     [ / [<alpha-value> | none] ]? )

    auto maybe_color_values = parse_lch_like_color_value(outer_tokens, "oklch"sv);
    if (!maybe_color_values.has_value())
        return {};

    auto& color_values = *maybe_color_values;

    return ColorFunctionStyleValue::create(ColorStyleValue::ColorType::OKLCH, color_values[0].release_nonnull(),
        color_values[1].release_nonnull(),
        color_values[2].release_nonnull(),
        color_values[3].release_nonnull());
}

// https://www.w3.org/TR/css-color-4/#funcdef-color
RefPtr<StyleValue const> Parser::parse_color_function(TokenStream<ComponentValue>& outer_tokens)
{
    // color() = color( <colorspace-params> [ / [ <alpha-value> | none ] ]? )
    //     <colorspace-params> = [ <predefined-rgb-params> | <xyz-params>]
    //     <predefined-rgb-params> = <predefined-rgb> [ <number> | <percentage> | none ]{3}
    //     <predefined-rgb> = srgb | srgb-linear | display-p3 | a98-rgb | prophoto-rgb | rec2020
    //     <xyz-params> = <xyz-space> [ <number> | <percentage> | none ]{3}
    //     <xyz-space> = xyz | xyz-d50 | xyz-d65

    auto transaction = outer_tokens.begin_transaction();
    outer_tokens.discard_whitespace();

    auto const& function_token = outer_tokens.consume_a_token();
    if (!function_token.is_function("color"sv))
        return {};

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.function().name });

    auto inner_tokens = TokenStream { function_token.function().value };
    inner_tokens.discard_whitespace();

    auto const& maybe_color_space = inner_tokens.consume_a_token();
    inner_tokens.discard_whitespace();
    if (!maybe_color_space.is(Token::Type::Ident))
        return {};

    auto color_space = maybe_color_space.token().ident().to_ascii_lowercase();
    if (!color_type_from_color_function_name(color_space).has_value())
        return {};

    auto c1 = parse_number_percentage_none_value(inner_tokens);
    if (!c1)
        return {};
    inner_tokens.discard_whitespace();

    auto c2 = parse_number_percentage_none_value(inner_tokens);
    if (!c2)
        return {};
    inner_tokens.discard_whitespace();

    auto c3 = parse_number_percentage_none_value(inner_tokens);
    if (!c3)
        return {};
    inner_tokens.discard_whitespace();

    RefPtr<StyleValue const> alpha;
    if (inner_tokens.has_next_token()) {
        alpha = parse_solidus_and_alpha_value(inner_tokens);
        if (!alpha || inner_tokens.has_next_token())
            return {};
    }

    if (!alpha)
        alpha = NumberStyleValue::create(1);

    transaction.commit();
    auto color_type = color_type_from_color_function_name(color_space);
    VERIFY(color_type.has_value());
    return ColorFunctionStyleValue::create(*color_type,
        c1.release_nonnull(),
        c2.release_nonnull(),
        c3.release_nonnull(),
        alpha.release_nonnull());
}

// https://drafts.csswg.org/css-color-5/#color-interpolation-method
RefPtr<ColorInterpolationMethodStyleValue const> Parser::parse_color_interpolation_method_value(TokenStream<ComponentValue>& tokens)
{
    // <rectangular-color-space> = srgb | srgb-linear | display-p3 | display-p3-linear | a98-rgb | prophoto-rgb | rec2020 | lab | oklab | <xyz-space>
    // <polar-color-space> = hsl | hwb | lch | oklch
    // <custom-color-space> = <dashed-ident>
    // <hue-interpolation-method> = [ shorter | longer | increasing | decreasing ] hue
    // <color-interpolation-method> = in [ <rectangular-color-space> | <polar-color-space> <hue-interpolation-method>? | <custom-color-space> ]
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    if (!tokens.consume_a_token().is_ident("in"sv))
        return {};

    tokens.discard_whitespace();

    if (auto maybe_keyword_value = parse_keyword_value(tokens)) {
        auto keyword = maybe_keyword_value->to_keyword();

        // <rectangular-color-space>
        if (auto rectangular_color_space = keyword_to_rectangular_color_space(keyword); rectangular_color_space.has_value()) {
            if (rectangular_color_space == RectangularColorSpace::Xyz)
                rectangular_color_space = RectangularColorSpace::XyzD65;
            transaction.commit();
            return ColorInterpolationMethodStyleValue::create(rectangular_color_space.release_value());
        }

        // <polar-color-space> <hue-interpolation-method>?
        if (auto polar_color_space = keyword_to_polar_color_space(keyword); polar_color_space.has_value()) {
            auto hue_interpolation_method = HueInterpolationMethod::Shorter;
            tokens.discard_whitespace();
            if (auto hue_interpolation_method_keyword = parse_keyword_value(tokens)) {
                auto maybe_hue_interpolation_method = keyword_to_hue_interpolation_method(hue_interpolation_method_keyword->to_keyword());
                if (!maybe_hue_interpolation_method.has_value())
                    return {};

                hue_interpolation_method = maybe_hue_interpolation_method.release_value();
                tokens.discard_whitespace();
                if (!tokens.consume_a_token().is_ident("hue"sv))
                    return {};
            }

            transaction.commit();
            return ColorInterpolationMethodStyleValue::create(ColorInterpolationMethodStyleValue::PolarColorInterpolationMethod { polar_color_space.release_value(), hue_interpolation_method });
        }
    }

    // TODO: Support <custom-color-space> once we support @color-profile rules

    return nullptr;
}

// https://drafts.csswg.org/css-color-5/#color-mix
RefPtr<StyleValue const> Parser::parse_color_mix_function(TokenStream<ComponentValue>& tokens)
{
    auto parse_component = [this](TokenStream<ComponentValue>& function_tokens) -> Optional<ColorMixStyleValue::ColorMixComponent> {
        function_tokens.discard_whitespace();
        auto percentage_style_value = parse_percentage_value(function_tokens, { .min = 0, .max = 100 });
        function_tokens.discard_whitespace();
        auto color_style_value = parse_color_value(function_tokens);
        if (!color_style_value)
            return {};
        function_tokens.discard_whitespace();
        if (!percentage_style_value) {
            percentage_style_value = parse_percentage_value(function_tokens, { .min = 0, .max = 100 });
            function_tokens.discard_whitespace();
        }
        return ColorMixStyleValue::ColorMixComponent {
            .color = color_style_value.release_nonnull(),
            .percentage = move(percentage_style_value),
        };
    };

    // color-mix() = color-mix( <color-interpolation-method>? , [ <color> && <percentage [0,100]>? ]#)
    // FIXME: Update color-mix to accept 1+ colors instead of exactly 2.
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    auto const& function_token = tokens.consume_a_token();
    if (!function_token.is_function("color-mix"sv))
        return {};

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.function().name });
    auto function_tokens = TokenStream { function_token.function().value };
    auto color_interpolation_method = parse_color_interpolation_method_value(function_tokens);
    if (color_interpolation_method) {
        function_tokens.discard_whitespace();
        if (!function_tokens.consume_a_token().is(Token::Type::Comma))
            return {};
    }

    auto first_component = parse_component(function_tokens);
    if (!first_component.has_value())
        return {};
    tokens.discard_whitespace();
    if (!function_tokens.consume_a_token().is(Token::Type::Comma))
        return {};

    auto second_component = parse_component(function_tokens);
    if (!second_component.has_value())
        return {};

    if (first_component->percentage && second_component->percentage
        && !first_component->percentage->is_calculated() && !second_component->percentage->is_calculated()
        && first_component->percentage->as_percentage().percentage().value() == 0 && second_component->percentage->as_percentage().percentage().value() == 0) {
        return {};
    }

    tokens.discard_whitespace();
    if (function_tokens.has_next_token())
        return {};

    transaction.commit();
    return ColorMixStyleValue::create(move(color_interpolation_method), move(*first_component), move(*second_component));
}

// https://drafts.csswg.org/css-color-5/#funcdef-light-dark
RefPtr<StyleValue const> Parser::parse_light_dark_color_value(TokenStream<ComponentValue>& outer_tokens)
{
    auto transaction = outer_tokens.begin_transaction();

    outer_tokens.discard_whitespace();
    auto const& function_token = outer_tokens.consume_a_token();
    if (!function_token.is_function("light-dark"sv))
        return {};

    auto inner_tokens = TokenStream { function_token.function().value };

    inner_tokens.discard_whitespace();
    auto light = parse_color_value(inner_tokens);
    if (!light)
        return {};

    inner_tokens.discard_whitespace();
    if (!inner_tokens.consume_a_token().is(Token::Type::Comma))
        return {};

    inner_tokens.discard_whitespace();
    auto dark = parse_color_value(inner_tokens);
    if (!dark)
        return {};

    inner_tokens.discard_whitespace();
    if (inner_tokens.has_next_token())
        return {};

    transaction.commit();
    return LightDarkStyleValue::create(light.release_nonnull(), dark.release_nonnull());
}

// https://www.w3.org/TR/css-color-4/#color-syntax
RefPtr<StyleValue const> Parser::parse_color_value(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    {
        auto transaction = tokens.begin_transaction();
        auto start = tokens.current_index();
        tokens.discard_whitespace();
        if (tokens.has_next_token()) {
            tokens.discard_a_token();
            Optional<String> serialized_color;
            auto color_source = original_source_text.value_or_lazy_evaluated([&] {
                serialized_color = serialize_component_values_for_reparsing(tokens.tokens_since(start));
                return serialized_color->bytes_as_string_view();
            });
            auto rust_color = RustComponentValueParser::parse_simple_color(color_source, "utf-8"sv, context_allows_quirky_color());
            if (rust_color.has_value()) {
                switch (rust_color.value().kind) {
                case FFI::CssParsedColorKind::Invalid:
                    break;
                case FFI::CssParsedColorKind::Rgba: {
                    transaction.commit();
                    return ColorStyleValue::create_from_color({ rust_color->red, rust_color->green, rust_color->blue, rust_color->alpha }, ColorSyntax::Legacy, rust_color->name);
                }
                case FFI::CssParsedColorKind::Keyword: {
                    if (!rust_color->name.has_value())
                        break;
                    auto keyword = keyword_from_string(*rust_color->name);
                    if (!keyword.has_value())
                        break;
                    transaction.commit();
                    return KeywordStyleValue::create(*keyword);
                }
                }
            }
        }
    }

    auto start = tokens.current_index();
    auto validate_parsed_color = [&](RefPtr<StyleValue const> value, bool allow_quirky_color = false) -> RefPtr<StyleValue const> {
        if (!value)
            return nullptr;

        Optional<String> serialized_color;
        auto color_source = original_source_text.value_or_lazy_evaluated([&] {
            serialized_color = serialize_component_values_for_reparsing(tokens.tokens_since(start));
            return serialized_color->bytes_as_string_view();
        });
        if (RustComponentValueParser::parse_color(color_source, "utf-8"sv, allow_quirky_color) == FFI::CssColorValueKind::Invalid)
            return nullptr;

        return value;
    };

    // Keywords: <system-color> | <deprecated-color> | currentColor
    {
        auto transaction = tokens.begin_transaction();
        if (auto keyword = parse_keyword_value(tokens); keyword && keyword->has_color()) {
            if (auto color = validate_parsed_color(keyword)) {
                transaction.commit();
                return color;
            }
        }
    }

    // Functions
    if (auto color = validate_parsed_color(parse_color_function(tokens)))
        return color;

    if (auto color = validate_parsed_color(parse_color_mix_function(tokens)))
        return color;

    if (auto rgb = validate_parsed_color(parse_rgb_color_value(tokens)))
        return rgb;
    if (auto hsl = validate_parsed_color(parse_hsl_color_value(tokens)))
        return hsl;
    if (auto hwb = validate_parsed_color(parse_hwb_color_value(tokens)))
        return hwb;
    if (auto lab = validate_parsed_color(parse_lab_color_value(tokens)))
        return lab;
    if (auto lch = validate_parsed_color(parse_lch_color_value(tokens)))
        return lch;
    if (auto oklab = validate_parsed_color(parse_oklab_color_value(tokens)))
        return oklab;
    if (auto oklch = validate_parsed_color(parse_oklch_color_value(tokens)))
        return oklch;
    if (auto light_dark = validate_parsed_color(parse_light_dark_color_value(tokens)))
        return light_dark;

    return {};
}

NonnullRefPtr<StyleValue const> Parser::materialize_rust_counter_style(Optional<RustComponentValueParser::CounterStyle> const& maybe_counter_style)
{
    if (!maybe_counter_style.has_value())
        return CounterStyleStyleValue::create("decimal"_fly_string);

    auto counter_style = *maybe_counter_style;
    if (counter_style.kind == FFI::CssCounterStyleKind::Name) {
        auto counter_style_name = counter_style.name;

        // https://drafts.csswg.org/css-counter-styles-3/#the-counter-style-rule
        // Counter style names are case-sensitive. However, the names defined in this specification are ASCII lowercased
        // on parse wherever they are used as counter styles, e.g. in the list-style set of properties, in the
        // @counter-style rule, and in the counter() functions.

        // NB: The "names defined in this specification" are defined in the `CounterStyleNameKeyword` enum
        auto const& keyword = keyword_from_string(counter_style_name);
        if (keyword.has_value() && keyword_to_counter_style_name_keyword(keyword.value()).has_value())
            counter_style_name = counter_style_name.to_ascii_lowercase();

        return CounterStyleStyleValue::create(counter_style_name);
    }

    VERIFY(counter_style.kind == FFI::CssCounterStyleKind::SymbolsFunction);
    auto symbols_type = [&] {
        switch (counter_style.symbols_type) {
        case FFI::CssCounterStyleSymbolsType::Cyclic:
            return SymbolsType::Cyclic;
        case FFI::CssCounterStyleSymbolsType::Numeric:
            return SymbolsType::Numeric;
        case FFI::CssCounterStyleSymbolsType::Alphabetic:
            return SymbolsType::Alphabetic;
        case FFI::CssCounterStyleSymbolsType::Symbolic:
            return SymbolsType::Symbolic;
        case FFI::CssCounterStyleSymbolsType::Fixed:
            return SymbolsType::Fixed;
        }
        VERIFY_NOT_REACHED();
    }();
    return CounterStyleStyleValue::create(CounterStyleStyleValue::SymbolsFunction { symbols_type, move(counter_style.symbols) });
}

// https://drafts.csswg.org/css-lists-3/#counter-functions
RefPtr<StyleValue const> Parser::parse_counter_value(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    // counter() = counter( <counter-name>, <counter-style>? )
    // counters() = counters( <counter-name>, <string>, <counter-style>? )
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    Optional<String> serialized_counter;
    auto counter_source = original_source_text.has_value() && !original_source_text->is_empty()
        ? original_source_text.value()
        : [&] {
              serialized_counter = serialize_component_values_for_reparsing(tokens.remaining_tokens());
              return serialized_counter->bytes_as_string_view();
          }();
    auto counter = RustComponentValueParser::parse_a_counter(counter_source, "utf-8"sv);
    if (!counter.has_value())
        return nullptr;

    auto counter_style = materialize_rust_counter_style(counter->counter_style);
    while (tokens.has_next_token())
        tokens.discard_a_token();

    transaction.commit();
    switch (counter->function) {
    case RustComponentValueParser::RustCounterFunctionKind::Counter:
        return CounterStyleValue::create_counter(counter->name, counter_style);
    case RustComponentValueParser::RustCounterFunctionKind::Counters:
        return CounterStyleValue::create_counters(counter->name, counter->join_string, counter_style);
    }
    VERIFY_NOT_REACHED();
}

RefPtr<StringStyleValue const> Parser::parse_string_value(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::String))
        return nullptr;

    tokens.discard_whitespace();
    auto const& peek = tokens.next_token();
    if (peek.is(Token::Type::String)) {
        tokens.discard_a_token();
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::String, {}, original_source_text))
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        return StringStyleValue::create(peek.token().string());
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_easing_value(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return nullptr;

    auto const& component_value = tokens.consume_a_token();
    Optional<String> serialized_easing;
    auto easing_source = original_source_text.has_value() && !original_source_text->is_empty()
        ? original_source_text.value()
        : [&] {
              serialized_easing = serialize_component_values_for_reparsing({ &component_value, 1 });
              return serialized_easing->bytes_as_string_view();
          }();
    auto rust_style_value = RustComponentValueParser::parse_style_value_for_value_type(PropertyID::AnimationTimingFunction, ValueType::EasingFunction, easing_source);
    if (!rust_style_value.has_value() || rust_style_value->kind != FFI::CssStyleValueKind::EasingFunction)
        return nullptr;
    auto easing = &*rust_style_value;

    auto parse_nested_number = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value, NumericRange const& range) -> RefPtr<StyleValue const> {
        if (!value.source_component_values.is_empty()) {
            TokenStream value_tokens { value.source_component_values };
            auto parsed = parse_number_value(value_tokens, range);
            value_tokens.discard_whitespace();
            if (!parsed || value_tokens.has_next_token())
                return nullptr;
            return parsed;
        }

        if (value.numeric_value.has_value()) {
            if (value.primitive_kind != FFI::CssPrimitiveValueKind::Number || !range.contains(*value.numeric_value))
                return nullptr;
            return NumberStyleValue::create(*value.numeric_value);
        }

        return nullptr;
    };

    auto parse_nested_percentage = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value) -> RefPtr<StyleValue const> {
        if (!value.source_component_values.is_empty()) {
            TokenStream value_tokens { value.source_component_values };
            auto parsed = parse_percentage_value(value_tokens, infinite_range);
            value_tokens.discard_whitespace();
            if (!parsed || value_tokens.has_next_token())
                return nullptr;
            return parsed;
        }

        if (value.numeric_value.has_value()) {
            if (value.primitive_kind != FFI::CssPrimitiveValueKind::Percentage)
                return nullptr;
            return PercentageStyleValue::create(Percentage { *value.numeric_value });
        }

        return nullptr;
    };

    auto parse_nested_integer = [&](RustComponentValueParser::RustNestedPrimitiveValue const& value, NumericRange const& range) -> RefPtr<StyleValue const> {
        if (!value.source_component_values.is_empty()) {
            TokenStream value_tokens { value.source_component_values };
            auto parsed = parse_integer_value(value_tokens, range);
            value_tokens.discard_whitespace();
            if (!parsed || value_tokens.has_next_token())
                return nullptr;
            return parsed;
        }

        if (value.numeric_value.has_value()) {
            if (value.primitive_kind != FFI::CssPrimitiveValueKind::Integer || !range.contains(*value.numeric_value))
                return nullptr;
            return IntegerStyleValue::create(static_cast<i32>(*value.numeric_value));
        }

        return nullptr;
    };

    auto materialize_easing = [&]() -> RefPtr<StyleValue const> {
        enum : u8 {
            Keyword,
            Linear,
            CubicBezier,
            Steps,
        };

        switch (easing->easing_function_kind) {
        case Keyword:
            return EasingStyleValue::create(EasingStyleValue::Steps { IntegerStyleValue::create(1), easing->easing_function_step_position });
        case Linear: {
            auto context_guard = push_temporary_value_parsing_context(FunctionContext { "linear"sv });
            Vector<EasingStyleValue::Linear::Stop> stops;
            for (auto const& stop : easing->linear_easing_stops) {
                auto output = parse_nested_number(stop.output, infinite_range);
                if (!output)
                    return nullptr;

                RefPtr<StyleValue const> first_input;
                if (stop.first_stop_length.has_value()) {
                    first_input = parse_nested_percentage(*stop.first_stop_length);
                    if (!first_input)
                        return nullptr;
                }

                auto output_value = output.release_nonnull();
                stops.append({ output_value, first_input });
                if (stop.second_stop_length.has_value()) {
                    auto second_input = parse_nested_percentage(*stop.second_stop_length);
                    if (!second_input)
                        return nullptr;
                    stops.append({ output_value, second_input.release_nonnull() });
                }
            }
            if (stops.is_empty())
                return nullptr;
            return EasingStyleValue::create(EasingStyleValue::Linear { move(stops) });
        }
        case CubicBezier: {
            auto context_guard = push_temporary_value_parsing_context(FunctionContext { "cubic-bezier"sv });
            if (easing->easing_function_values.size() != 4)
                return nullptr;
            auto x1 = parse_nested_number(easing->easing_function_values[0], { .min = 0, .max = 1 });
            auto y1 = parse_nested_number(easing->easing_function_values[1], infinite_range);
            auto x2 = parse_nested_number(easing->easing_function_values[2], { .min = 0, .max = 1 });
            auto y2 = parse_nested_number(easing->easing_function_values[3], infinite_range);
            if (!x1 || !y1 || !x2 || !y2)
                return nullptr;
            return EasingStyleValue::create(EasingStyleValue::CubicBezier {
                x1.release_nonnull(),
                y1.release_nonnull(),
                x2.release_nonnull(),
                y2.release_nonnull(),
            });
        }
        case Steps: {
            auto context_guard = push_temporary_value_parsing_context(FunctionContext { "steps"sv });
            if (easing->easing_function_values.size() != 1)
                return nullptr;
            auto position = easing->easing_function_step_position;

            // https://drafts.csswg.org/css-easing/#step-easing-functions
            // If the <step-position> is jump-none, the <integer> must be at least 2, or the function is invalid.
            // Otherwise, the <integer> must be at least 1, or the function is invalid.
            double min_intervals = position == StepPosition::JumpNone ? 2 : 1;
            auto intervals = parse_nested_integer(easing->easing_function_values[0], NumericRange { .min = min_intervals, .max = AK::NumericLimits<i32>::max() });
            if (!intervals)
                return nullptr;
            return EasingStyleValue::create(EasingStyleValue::Steps { intervals.release_nonnull(), position });
        }
        default:
            return nullptr;
        }
    };

    auto value = materialize_easing();
    if (!value)
        return nullptr;

    transaction.commit();
    return value;
}

// https://drafts.csswg.org/css-values-4/#url-value
Optional<URL> Parser::parse_url_function(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    auto transaction = tokens.begin_transaction();
    auto const& component_value = tokens.next_token();
    Optional<String> serialized_url;
    auto url_source = original_source_text.has_value() && !original_source_text->is_empty()
        ? original_source_text.value()
        : [&] {
              serialized_url = serialize_component_values_for_reparsing({ &component_value, 1 });
              return serialized_url->bytes_as_string_view();
          }();
    auto maybe_url = RustComponentValueParser::parse_a_url_function(url_source, "utf-8"sv);
    if (!maybe_url.has_value())
        return {};

    tokens.discard_a_token();
    transaction.commit();
    return maybe_url.release_value();
}

RefPtr<StyleValue const> Parser::parse_builtin_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto& component_value = tokens.consume_a_token();
    if (component_value.is(Token::Type::Ident)) {
        auto ident = component_value.token().ident();
        if (ident.equals_ignoring_ascii_case("inherit"sv)) {
            transaction.commit();
            return KeywordStyleValue::create(Keyword::Inherit);
        }
        if (ident.equals_ignoring_ascii_case("initial"sv)) {
            transaction.commit();
            return KeywordStyleValue::create(Keyword::Initial);
        }
        if (ident.equals_ignoring_ascii_case("unset"sv)) {
            transaction.commit();
            return KeywordStyleValue::create(Keyword::Unset);
        }
        if (ident.equals_ignoring_ascii_case("revert"sv)) {
            transaction.commit();
            return KeywordStyleValue::create(Keyword::Revert);
        }
        if (ident.equals_ignoring_ascii_case("revert-layer"sv)) {
            transaction.commit();
            return KeywordStyleValue::create(Keyword::RevertLayer);
        }
    }

    return nullptr;
}

// https://www.w3.org/TR/css-values-4/#custom-idents
Optional<FlyString> Parser::parse_custom_ident(TokenStream<ComponentValue>& tokens, ReadonlySpan<StringView> blacklist, Optional<StringView> original_source_text)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    auto const& component_value = tokens.next_token();
    Optional<String> generated_source;
    StringView source;
    if (original_source_text.has_value()) {
        source = *original_source_text;
    } else {
        auto token_original_source_text = component_value.original_source_text();
        if (token_original_source_text.is_empty()) {
            generated_source = component_value.to_string();
            source = generated_source->bytes_as_string_view();
        } else {
            source = token_original_source_text;
        }
    }

    auto custom_ident = RustComponentValueParser::parse_a_custom_ident(source, "utf-8"sv);
    if (!custom_ident.has_value())
        return {};

    for (auto& value : blacklist) {
        if (custom_ident->equals_ignoring_ascii_case(value))
            return {};
    }

    tokens.discard_a_token();
    discard_remaining_tokens_if_using_original_source(tokens, original_source_text);

    transaction.commit();
    return custom_ident;
}

// https://drafts.csswg.org/css-values-4/#typedef-dashed-ident
Optional<FlyString> Parser::parse_dashed_ident(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    // The <dashed-ident> production is a <custom-ident>, with all the case-sensitivity that implies, with the
    // additional restriction that it must start with two dashes (U+002D HYPHEN-MINUS).
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    auto const& component_value = tokens.next_token();
    Optional<String> generated_source;
    StringView source;
    if (original_source_text.has_value()) {
        source = *original_source_text;
    } else {
        auto token_original_source_text = component_value.original_source_text();
        if (token_original_source_text.is_empty()) {
            generated_source = component_value.to_string();
            source = generated_source->bytes_as_string_view();
        } else {
            source = token_original_source_text;
        }
    }

    auto dashed_ident = RustComponentValueParser::parse_a_dashed_ident(source, "utf-8"sv);
    if (!dashed_ident.has_value())
        return {};
    tokens.discard_a_token();
    discard_remaining_tokens_if_using_original_source(tokens, original_source_text);

    transaction.commit();
    return dashed_ident;
}

RefPtr<CalculationNode const> Parser::materialize_rust_calculation_node_events(ReadonlySpan<RustComponentValueParser::RustCalculationNodeEvent const> calculation_node_events, CalculationContext const& context)
{
    if (calculation_node_events.is_empty())
        return nullptr;

    Vector<NonnullRefPtr<CalculationNode const>> stack;
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
            return NumericCalculationNode::create(Number { Number::Type::Number, *event.numeric_value }, context);
        case FFI::CssPrimitiveValueKind::Keyword: {
            auto maybe_keyword = keyword_from_string(event.metadata);
            if (!maybe_keyword.has_value())
                return nullptr;
            return NumericCalculationNode::from_keyword(*maybe_keyword, context);
        }
        case FFI::CssPrimitiveValueKind::Percentage:
            return NumericCalculationNode::create(Percentage { *event.numeric_value }, context);
        case FFI::CssPrimitiveValueKind::Angle: {
            auto unit = string_to_angle_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Angle { *event.numeric_value, unit.release_value() }, context);
        }
        case FFI::CssPrimitiveValueKind::Flex: {
            auto unit = string_to_flex_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Flex { *event.numeric_value, unit.release_value() }, context);
        }
        case FFI::CssPrimitiveValueKind::Frequency: {
            auto unit = string_to_frequency_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Frequency { *event.numeric_value, unit.release_value() }, context);
        }
        case FFI::CssPrimitiveValueKind::Length: {
            auto unit = string_to_length_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Length { *event.numeric_value, unit.release_value() }, context);
        }
        case FFI::CssPrimitiveValueKind::Resolution: {
            auto unit = string_to_resolution_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Resolution { *event.numeric_value, unit.release_value() }, context);
        }
        case FFI::CssPrimitiveValueKind::Time: {
            auto unit = string_to_time_unit(event.metadata);
            if (!unit.has_value())
                return nullptr;
            return NumericCalculationNode::create(Time { *event.numeric_value, unit.release_value() }, context);
        }
        default:
            return nullptr;
        }
    };
    auto matches_number = [&](CalculationNode const& node) {
        auto const& numeric_type = node.numeric_type();
        return numeric_type.has_value() && numeric_type->matches_number(context.percentages_resolve_as);
    };
    auto matches_sign_argument = [&](CalculationNode const& node) {
        auto const& numeric_type = node.numeric_type();
        return numeric_type.has_value()
            && (numeric_type->matches_number(context.percentages_resolve_as)
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
    auto append_random_node = [&](Vector<NonnullRefPtr<CalculationNode const>> const& children, StringView metadata) -> bool {
        if (!context_allows_random_functions())
            return false;

        auto metadata_parts = metadata.split_view('\0', SplitBehavior::KeepEmpty);
        if (metadata_parts.size() != 4 || metadata_parts[0] != "random"sv)
            return false;

        auto has_fixed_value_sharing = metadata_parts[1] == "fixed"sv;
        auto minimum_index = has_fixed_value_sharing ? 1uz : 0uz;
        if (children.size() != minimum_index + 2 && children.size() != minimum_index + 3)
            return false;

        auto const& minimum = children.at(minimum_index);
        auto const& maximum = children.at(minimum_index + 1);
        auto step_index = minimum_index + 2;
        if (!matches_sign_argument(*minimum) || !matches_sign_argument(*maximum))
            return false;
        if (!have_consistent_types(*minimum, *maximum))
            return false;
        if (children.size() == step_index + 1) {
            if (!matches_sign_argument(*children.at(step_index)))
                return false;
            if (!have_consistent_types(*minimum, *children.at(step_index)))
                return false;
        }

        m_random_function_index++;

        auto element_shared = metadata_parts[2] == "1"sv;
        RefPtr<RandomValueSharingStyleValue const> value_sharing;
        if (metadata_parts[1] == "auto"sv) {
            if (has_fixed_value_sharing || !metadata_parts[3].is_empty())
                return false;
            value_sharing = RandomValueSharingStyleValue::create_auto(random_value_sharing_auto_name(), element_shared);
        } else if (metadata_parts[1] == "dashed-ident"sv) {
            if (has_fixed_value_sharing || metadata_parts[3].is_empty())
                return false;
            value_sharing = RandomValueSharingStyleValue::create_dashed_ident(MUST(FlyString::from_utf8(metadata_parts[3])), element_shared);
        } else if (metadata_parts[1] == "fixed"sv) {
            if (element_shared)
                return false;
            auto preserve_fixed_calculation = metadata_parts[3] == "calc"sv;
            if (!metadata_parts[3].is_empty() && !preserve_fixed_calculation)
                return false;
            CalculationContext fixed_value_sharing_context {
                .accepted_ranges_by_type = { { ValueType::Number, NumericRange { .min = 0, .max = 0.999999 } } },
            };
            CalculationContext fixed_value_sharing_validation_context;
            auto fixed_calculation_tree = simplify_a_calculation_tree(*children.at(0), fixed_value_sharing_validation_context, CalculationResolutionContext {});
            auto fixed_calculation_type = fixed_calculation_tree->numeric_type();
            if (!fixed_calculation_type.has_value() || !fixed_calculation_type->matches_number(fixed_value_sharing_validation_context.percentages_resolve_as))
                return false;
            auto fixed_value = CalculatedStyleValue::create(fixed_calculation_tree, fixed_calculation_type.release_value(), fixed_value_sharing_context);
            if (!fixed_value->resolves_to_number())
                return false;
            if (is<NumericCalculationNode>(*fixed_calculation_tree)) {
                auto const* fixed_number = as<NumericCalculationNode>(*fixed_calculation_tree).value().get_pointer<Number>();
                if (!fixed_number || fixed_number->value() < 0 || fixed_number->value() > 0.999999)
                    return false;
            }
            if (is<NumericCalculationNode>(*children.at(0)) && !preserve_fixed_calculation) {
                auto const* fixed_number = as<NumericCalculationNode>(*children.at(0)).value().get_pointer<Number>();
                if (!fixed_number)
                    return false;
                value_sharing = RandomValueSharingStyleValue::create_fixed(NumberStyleValue::create(fixed_number->value()));
            } else {
                value_sharing = RandomValueSharingStyleValue::create_fixed(fixed_value);
            }
        } else {
            return false;
        }

        RefPtr<CalculationNode const> step;
        if (children.size() == step_index + 1)
            step = children.at(step_index);
        stack.append(RandomCalculationNode::create(value_sharing.release_nonnull(), minimum, maximum, move(step)));
        return true;
    };

    for (auto const& event : calculation_node_events) {
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
            if (event.metadata.equals_ignoring_ascii_case("sqrt"sv) && children->size() == 1) {
                stack.append(SqrtCalculationNode::create(children->first()));
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
                    : NumericCalculationNode::from_keyword(Keyword::E, context).release_nonnull();
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
            if (event.metadata.bytes_as_string_view().starts_with("random"sv)) {
                if (!append_random_node(*children, event.metadata))
                    return nullptr;
                break;
            }
            return nullptr;
        }
        case FFI::CssCalculationNodeKind::TreeCountingFunction: {
            if (!context_allows_tree_counting_functions())
                return nullptr;

            TreeCountingFunctionStyleValue::TreeCountingFunction function;
            if (event.metadata.equals_ignoring_ascii_case("sibling-count"sv)) {
                function = TreeCountingFunctionStyleValue::TreeCountingFunction::SiblingCount;
            } else if (event.metadata.equals_ignoring_ascii_case("sibling-index"sv)) {
                function = TreeCountingFunctionStyleValue::TreeCountingFunction::SiblingIndex;
            } else {
                return nullptr;
            }

            auto tree_counting_function = TreeCountingFunctionStyleValue::create(function, TreeCountingFunctionStyleValue::ComputedType::Number);
            stack.append(NonMathFunctionCalculationNode::create(*tree_counting_function, NumericType {}));
            break;
        }
        }
    }

    if (stack.size() != 1)
        return nullptr;
    return stack.first();
}

RefPtr<CalculatedStyleValue const> Parser::parse_calculated_value(ComponentValue const& component_value, CalculationContext&& context)
{
    if (!component_value.is_function())
        return nullptr;

    auto source = component_value.to_string();

    auto calculation_node_events = RustComponentValueParser::parse_calculation(source, "utf-8"sv);
    if (!calculation_node_events.has_value())
        return nullptr;

    auto function_node = materialize_rust_calculation_node_events(*calculation_node_events, context);
    if (!function_node)
        return nullptr;

    function_node = simplify_a_calculation_tree(*function_node, context, CalculationResolutionContext {});

    auto function_type = function_node->numeric_type();
    if (!function_type.has_value())
        return nullptr;

    return CalculatedStyleValue::create(function_node.release_nonnull(), function_type.release_value(), context);
}

// https://drafts.csswg.org/css-values-5/#tree-counting
RefPtr<TreeCountingFunctionStyleValue const> Parser::parse_tree_counting_function(TokenStream<ComponentValue>& tokens, TreeCountingFunctionStyleValue::ComputedType computed_type)
{
    if (!context_allows_tree_counting_functions())
        return nullptr;

    auto has_no_arguments = [](Vector<ComponentValue> const& component_values) {
        return !any_of(component_values, [](ComponentValue const& value) { return !value.is(Token::Type::Whitespace); });
    };

    auto transaction = tokens.begin_transaction();

    auto token = tokens.consume_a_token();

    if (token.is_function("sibling-count"sv) && has_no_arguments(token.function().value)) {
        transaction.commit();
        return TreeCountingFunctionStyleValue::create(TreeCountingFunctionStyleValue::TreeCountingFunction::SiblingCount, computed_type);
    }

    if (token.is_function("sibling-index"sv) && has_no_arguments(token.function().value)) {
        transaction.commit();
        return TreeCountingFunctionStyleValue::create(TreeCountingFunctionStyleValue::TreeCountingFunction::SiblingIndex, computed_type);
    }

    return nullptr;
}

// https://drafts.csswg.org/css-values-5/#typedef-if-condition
OwnPtr<BooleanExpression> Parser::parse_if_condition(TokenStream<ComponentValue>& tokens)
{
    // <if-condition> = <boolean-expr[ <if-test> ]> | else
    // <if-test> =
    //   supports( [ <ident> : <declaration-value> ] | <supports-condition> ) |
    //   media( <media-feature> | <media-condition> ) |
    //   style( <style-query> )

    // <boolean-expr[ <if-test> ]>
    {
        auto transaction = tokens.begin_transaction();
        Vector<ComponentValue> if_condition;
        while (tokens.has_next_token())
            if_condition.append(tokens.consume_a_token());

        auto serialized_if_condition = serialize_component_values_for_reparsing(if_condition);
        auto parsed_boolean_expression = RustComponentValueParser::parse_an_if_condition(serialized_if_condition.bytes_as_string_view(), "utf-8"sv, [&](Optional<RustComponentValueParser::MediaFeatureTest>&& media_feature, Optional<RustComponentValueParser::SupportsFeature>&& supports_feature, Vector<ComponentValue>&& component_values) -> OwnPtr<BooleanExpression> {
            if (supports_feature.has_value()) {
                m_rule_context.append(RuleContext::SupportsCondition);
                auto expression = materialize_rust_supports_feature(move(supports_feature), move(component_values));
                m_rule_context.take_last();
                return expression;
            }

            if (media_feature.has_value())
                return materialize_rust_media_feature_test(media_feature.release_value());

            TokenStream<ComponentValue> test_tokens { component_values };
            auto const& maybe_function_token = test_tokens.consume_a_token();

            if (!maybe_function_token.is_function())
                return nullptr;

            auto const& function = maybe_function_token.function();

            // FIXME: Support style()
            if (function.name.equals_ignoring_ascii_case("style"sv))
                return nullptr;

            return nullptr;
        });

        if (parsed_boolean_expression) {
            transaction.commit();
            return parsed_boolean_expression;
        }
    }

    // else
    auto transaction = tokens.begin_transaction();
    if (parse_all_as_single_keyword_value(tokens, Keyword::Else)) {
        transaction.commit();
        // The else keyword represents a condition that is always true.
        return ConstantBooleanExpression::create(MatchResult::True);
    }

    return nullptr;
}

// https://drafts.csswg.org/css-color-4/#typedef-opacity-opacity-value
RefPtr<StyleValue const> Parser::parse_opacity_value_value(TokenStream<ComponentValue>& tokens)
{
    // <opacity-value> = <number> | <percentage>
    auto start = tokens.current_index();
    if (!rust_primitive_value_prefix_matches(tokens, FFI::CssPrimitiveValueType::Opacity))
        return nullptr;

    if (auto value = parse_number_percentage_value(tokens, infinite_range, infinite_range)) {
        if (!rust_primitive_value_matches(tokens, start, FFI::CssPrimitiveValueType::Opacity))
            return nullptr;
        return OpacityValueStyleValue::create(value.release_nonnull());
    }

    return nullptr;
}

// https://drafts.csswg.org/css-fonts/#typedef-opentype-tag
RefPtr<StringStyleValue const> Parser::parse_opentype_tag_value(TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    // <opentype-tag> = <string>
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    auto start = tokens.current_index();
    if (!tokens.has_next_token())
        return nullptr;
    tokens.discard_a_token();

    Optional<String> serialized_opentype_tag;
    auto opentype_tag_source = original_source_text.has_value() && !original_source_text->is_empty()
        ? original_source_text.value()
        : [&] {
              serialized_opentype_tag = serialize_component_values_for_reparsing(tokens.tokens_since(start));
              return serialized_opentype_tag->bytes_as_string_view();
          }();
    auto opentype_tag = RustComponentValueParser::parse_an_opentype_tag(opentype_tag_source, "utf-8"sv);
    if (!opentype_tag.has_value())
        return nullptr;

    transaction.commit();
    return StringStyleValue::create(opentype_tag.release_value());
}

NonnullRefPtr<StyleValue const> Parser::resolve_unresolved_style_value(ParsingParams const& context, DOM::AbstractElement abstract_element, PropertyNameAndID const& property, UnresolvedStyleValue const& unresolved, Optional<GuardedSubstitutionContexts&> existing_guarded_contexts)
{
    auto parser = Parser::create(context, ""sv);
    if (existing_guarded_contexts.has_value())
        return parser.resolve_unresolved_style_value(abstract_element, existing_guarded_contexts.value(), property, unresolved);
    GuardedSubstitutionContexts guarded_contexts;
    return parser.resolve_unresolved_style_value(abstract_element, guarded_contexts, property, unresolved);
}

// https://drafts.csswg.org/css-values-5/#property-replacement
NonnullRefPtr<StyleValue const> Parser::resolve_unresolved_style_value(DOM::AbstractElement element, GuardedSubstitutionContexts& guarded_contexts, PropertyNameAndID const& property, UnresolvedStyleValue const& unresolved)
{
    // AD-HOC: Report that we might rely on custom properties.
    if (unresolved.includes_attr_function())
        element.element().set_style_uses_attr_css_function();
    if (unresolved.includes_if_function())
        element.element().set_style_uses_if_css_function();
    if (unresolved.includes_inherit_function())
        element.element().set_style_uses_inherit_css_function();
    if (unresolved.includes_var_function())
        element.element().set_style_uses_var_css_function();

    // To replace substitution functions in a property prop:

    // 1. Substitute arbitrary substitution functions in prop’s value, given «"property", prop’s name» as the
    //    substitution context. Let result be the returned component value sequence.
    auto result = substitute_arbitrary_substitution_functions(element, guarded_contexts, unresolved.values(), SubstitutionContext { SubstitutionContext::DependencyType::Property, property.name().to_string() });

    // 2. If result contains the guaranteed-invalid value, prop is invalid at computed-value time; return.
    if (contains_guaranteed_invalid_value(result))
        return GuaranteedInvalidStyleValue::create();

    // 3. Parse result according to prop’s grammar. If this returns failure, prop is invalid at computed-value time; return.
    // NB: Custom properties have no grammar as such, so we skip this step for them.
    // FIXME: Parse according to @property syntax once we support that.
    if (property.is_custom_property())
        return UnresolvedStyleValue::create(move(result), {});

    auto expanded_value_tokens = TokenStream { result };
    auto parsed_value = parse_css_value(property.id(), expanded_value_tokens);
    if (parsed_value.is_error())
        return GuaranteedInvalidStyleValue::create();

    // 4. Otherwise, replace prop’s value with the parsed result.
    return parsed_value.release_value();
}

// https://drafts.csswg.org/css-counter-styles-3/#typedef-symbol
RefPtr<StyleValue const> Parser::parse_symbol_value(TokenStream<ComponentValue>& tokens)
{
    // <symbol> = <string> | <custom-ident>
    // AD-HOC: The spec actually defines this as '<string> | <image> | <custom-ident>' but the image portion is
    // considered at-risk and no other browser supports it.
    auto transaction = tokens.begin_transaction();
    if (auto string = parse_string_value(tokens)) {
        transaction.commit();
        return string.release_nonnull();
    }

    if (auto custom_ident = parse_custom_ident(tokens, {}); custom_ident.has_value()) {
        transaction.commit();
        return CustomIdentStyleValue::create(custom_ident.release_value());
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_rust_owned_property_value_prefix(PropertyID property_id, TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    if (original_source_text.has_value())
        return parse_css_value_for_property(property_id, tokens, original_source_text);

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    auto start = tokens.current_index();
    RefPtr<StyleValue const> parsed_value;

    while (tokens.has_next_token()) {
        auto component_transaction = tokens.begin_transaction();
        tokens.discard_whitespace();
        if (!tokens.has_next_token())
            break;
        tokens.discard_a_token();

        auto serialized_value = Parser::serialize_component_values_for_reparsing(tokens.tokens_since(start));
        Vector<ComponentValue> value_tokens;
        for (auto const& token : tokens.tokens_since(start))
            value_tokens.append(token);
        TokenStream<ComponentValue> value_token_stream { value_tokens };
        auto maybe_value = parse_css_value_for_property(property_id, value_token_stream, serialized_value.bytes_as_string_view());
        value_token_stream.discard_whitespace();
        if (!maybe_value || value_token_stream.has_next_token())
            break;

        component_transaction.commit();
        parsed_value = maybe_value.release_nonnull();
    }

    if (!parsed_value)
        return nullptr;

    transaction.commit();
    return parsed_value;
}

RefPtr<StyleValue const> Parser::parse_value(ValueType value_type, TokenStream<ComponentValue>& tokens, Optional<StringView> original_source_text)
{
    auto parse_rust_owned_property_value = [&](PropertyID property_id, auto accepts_value) -> RefPtr<StyleValue const> {
        auto transaction = tokens.begin_transaction();
        auto value = parse_css_value_for_property(property_id, tokens, original_source_text);
        if (!value || !accepts_value(*value))
            return nullptr;

        transaction.commit();
        return value;
    };

    switch (value_type) {
    case ValueType::Anchor:
        return parse_anchor(tokens);
    case ValueType::AnchorSize:
        return parse_anchor_size(tokens);
    case ValueType::Angle:
        return parse_angle_value(tokens, infinite_range, original_source_text);
    case ValueType::AnglePercentage:
        return parse_angle_percentage_value(tokens, infinite_range, infinite_range, original_source_text);
    case ValueType::BackgroundPosition:
        return parse_rust_owned_property_value(PropertyID::BackgroundPosition, [](StyleValue const& value) { return value.is_position(); });
    case ValueType::BasicShape:
        return parse_rust_owned_property_value(PropertyID::ShapeOutside, [](StyleValue const& value) { return value.is_basic_shape(); });
    case ValueType::Color:
        return parse_color_value(tokens, original_source_text);
    case ValueType::CornerShape:
        return parse_rust_owned_property_value(PropertyID::CornerTopLeftShape, [](StyleValue const& value) { return value.is_keyword() || value.is_superellipse(); });
    case ValueType::Counter:
        return parse_counter_value(tokens, original_source_text);
    case ValueType::CounterStyle:
        return parse_rust_owned_property_value_prefix(PropertyID::ListStyleType, tokens, original_source_text);
    case ValueType::CustomIdent: {
        // FIXME: Figure out how to pass the blacklist here
        auto custom_ident = parse_custom_ident(tokens, {}, original_source_text);
        if (!custom_ident.has_value())
            return nullptr;
        return CustomIdentStyleValue::create(custom_ident.release_value());
    }
    case ValueType::DashedIdent: {
        auto dashed_ident = parse_dashed_ident(tokens, original_source_text);
        if (!dashed_ident.has_value())
            return nullptr;
        return CustomIdentStyleValue::create(dashed_ident.release_value());
    }
    case ValueType::EasingFunction:
        return parse_easing_value(tokens, original_source_text);
    case ValueType::FilterValueList:
        return parse_css_value_for_property(PropertyID::Filter, tokens, original_source_text);
    case ValueType::FitContent:
        return parse_rust_owned_property_value(PropertyID::Width, [](StyleValue const& value) { return (value.is_keyword() && value.to_keyword() == Keyword::FitContent) || (value.is_function() && value.as_function().name() == "fit-content"_fly_string); });
    case ValueType::Flex:
        return parse_flex_value(tokens, infinite_range, original_source_text);
    case ValueType::FontStyle:
        return parse_rust_owned_property_value_prefix(PropertyID::FontStyle, tokens, original_source_text);
    case ValueType::FontKerningValue:
        return parse_rust_owned_property_value(PropertyID::FontKerning, [](StyleValue const& value) { return value.is_keyword(); });
    case ValueType::FontOpticalSizingValue:
        return parse_rust_owned_property_value(PropertyID::FontOpticalSizing, [](StyleValue const& value) { return value.is_keyword(); });
    case ValueType::FontWeightAbsolute:
        return parse_rust_owned_property_value(PropertyID::FontWeight, [](StyleValue const& value) {
            return value.is_number() || (value.is_keyword() && first_is_one_of(value.to_keyword(), Keyword::Normal, Keyword::Bold));
        });
    case ValueType::FontWidthCss3:
        return parse_rust_owned_property_value(PropertyID::FontWidth, [](StyleValue const& value) { return value.is_keyword(); });
    case ValueType::FontVariantAlternates:
        return parse_rust_owned_property_value_prefix(PropertyID::FontVariantAlternates, tokens, original_source_text);
    case ValueType::FontVariantCapsValue:
        return parse_rust_owned_property_value(PropertyID::FontVariantCaps, [](StyleValue const& value) { return value.is_keyword(); });
    case ValueType::FontVariantCss2:
        return parse_rust_owned_property_value(PropertyID::FontVariantCaps, [](StyleValue const& value) {
            return value.is_keyword() && first_is_one_of(value.to_keyword(), Keyword::Normal, Keyword::SmallCaps);
        });
    case ValueType::FontVariantEastAsian:
        return parse_rust_owned_property_value_prefix(PropertyID::FontVariantEastAsian, tokens, original_source_text);
    case ValueType::FontVariantEmojiValue:
        return parse_rust_owned_property_value(PropertyID::FontVariantEmoji, [](StyleValue const& value) { return value.is_keyword(); });
    case ValueType::FontVariantLigatures:
        return parse_rust_owned_property_value_prefix(PropertyID::FontVariantLigatures, tokens, original_source_text);
    case ValueType::FontVariantNumeric:
        return parse_rust_owned_property_value_prefix(PropertyID::FontVariantNumeric, tokens, original_source_text);
    case ValueType::FontVariantPositionValue:
        return parse_rust_owned_property_value(PropertyID::FontVariantPosition, [](StyleValue const& value) { return value.is_keyword(); });
    case ValueType::Frequency:
        return parse_frequency_value(tokens, infinite_range, original_source_text);
    case ValueType::FrequencyPercentage:
        return parse_frequency_percentage_value(tokens, infinite_range, infinite_range, original_source_text);
    case ValueType::Image: {
        auto transaction = tokens.begin_transaction();
        tokens.discard_whitespace();
        if (!tokens.has_next_token())
            return nullptr;

        auto const& component_value = tokens.consume_a_token();
        if (component_value.contains_attr_tainted_value())
            return nullptr;

        auto component_value_tokens = TokenStream<ComponentValue>::of_single_token(component_value);
        auto value = parse_css_value_for_property(PropertyID::BorderImageSource, component_value_tokens, original_source_text);
        component_value_tokens.discard_whitespace();
        if (!value || (!original_source_text.has_value() && component_value_tokens.has_next_token()) || !value->is_abstract_image())
            return nullptr;

        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return value;
    }
    case ValueType::Integer:
        return parse_integer_value(tokens, infinite_integer_range, original_source_text);
    case ValueType::Length:
        return parse_length_value(tokens, infinite_range, original_source_text);
    case ValueType::LengthPercentage:
        return parse_length_percentage_value(tokens, infinite_range, infinite_range, original_source_text);
    case ValueType::Number:
        return parse_number_value(tokens, infinite_range, original_source_text);
    case ValueType::OpacityValue:
        return parse_rust_owned_property_value(PropertyID::Opacity, [](StyleValue const& value) { return value.is_opacity_value(); });
    case ValueType::OpentypeTag:
        return parse_opentype_tag_value(tokens, original_source_text);
    case ValueType::Paint:
        return parse_css_value_for_property(PropertyID::Fill, tokens, original_source_text);
    case ValueType::Percentage:
        return parse_percentage_value(tokens, infinite_range, original_source_text);
    case ValueType::Position:
        return parse_rust_owned_property_value(PropertyID::ObjectPosition, [](StyleValue const& value) { return value.is_position(); });
    case ValueType::Ratio:
        return parse_rust_owned_property_value(PropertyID::AspectRatio, [](StyleValue const& value) { return value.is_ratio(); });
    case ValueType::Rect:
        return parse_rust_owned_property_value(PropertyID::Clip, [](StyleValue const& value) { return value.is_rect(); });
    case ValueType::Resolution:
        return parse_resolution_value(tokens, infinite_range, original_source_text);
    case ValueType::ScrollFunction:
        return parse_scroll_function_value(tokens, original_source_text);
    case ValueType::String:
        return parse_string_value(tokens, original_source_text);
    case ValueType::Symbol:
        return parse_symbol_value(tokens);
    case ValueType::Time:
        return parse_time_value(tokens, infinite_range, original_source_text);
    case ValueType::TimePercentage:
        return parse_time_percentage_value(tokens, infinite_range, infinite_range, original_source_text);
    case ValueType::TransformFunction: {
        auto transaction = tokens.begin_transaction();
        tokens.discard_whitespace();
        if (!tokens.has_next_token())
            return nullptr;

        auto const& part = tokens.consume_a_token();
        auto component_value_tokens = TokenStream<ComponentValue>::of_single_token(part);
        auto value = parse_css_value_for_property(PropertyID::Transform, component_value_tokens, original_source_text);
        component_value_tokens.discard_whitespace();
        if (!value || (!original_source_text.has_value() && component_value_tokens.has_next_token()) || !value->is_value_list())
            return nullptr;

        auto const& transformations = value->as_value_list();
        if (transformations.size() != 1)
            return nullptr;
        discard_remaining_tokens_if_using_original_source(tokens, original_source_text);
        transaction.commit();
        return transformations.value_at(0, false);
    }
    case ValueType::TransformList: {
        auto value = parse_rust_owned_property_value_prefix(PropertyID::Transform, tokens, original_source_text);
        if (!value || !value->is_value_list())
            return nullptr;
        return value;
    }
    case ValueType::Url: {
        auto url = parse_url_function(tokens, original_source_text);
        if (!url.has_value())
            return nullptr;
        return URLStyleValue::create(url.release_value());
    }
    case ValueType::ViewFunction:
        return parse_view_function_value(tokens, original_source_text);
    case ValueType::ViewTimelineInset:
        return parse_rust_owned_property_value_prefix(PropertyID::ViewTimelineInset, tokens, original_source_text);
    }
    VERIFY_NOT_REACHED();
}

}
