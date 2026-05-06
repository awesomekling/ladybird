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
#include <LibWeb/CSS/MathFunctions.h>
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
#include <LibWeb/CSS/StyleValues/BorderRadiusRectStyleValue.h>
#include <LibWeb/CSS/StyleValues/BorderRadiusStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorFunctionStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorInterpolationMethodStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorMixStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorStyleValue.h>
#include <LibWeb/CSS/StyleValues/ConicGradientStyleValue.h>
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
#include <LibWeb/CSS/StyleValues/GridTrackPlacementStyleValue.h>
#include <LibWeb/CSS/StyleValues/GridTrackSizeListStyleValue.h>
#include <LibWeb/CSS/StyleValues/GuaranteedInvalidStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageSetStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/LightDarkStyleValue.h>
#include <LibWeb/CSS/StyleValues/LinearGradientStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/OpacityValueStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/PositionStyleValue.h>
#include <LibWeb/CSS/StyleValues/RadialGradientStyleValue.h>
#include <LibWeb/CSS/StyleValues/RadialSizeStyleValue.h>
#include <LibWeb/CSS/StyleValues/RandomValueSharingStyleValue.h>
#include <LibWeb/CSS/StyleValues/RatioStyleValue.h>
#include <LibWeb/CSS/StyleValues/RectStyleValue.h>
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
    auto remaining_tokens = tokens.remaining_tokens();
    // AD-HOC: Re-parsing substituted component values through Rust would lose
    // C++-side attr() taint metadata until that metadata is carried over FFI.
    if (!end_token_type.has_value() && !remaining_tokens.first_matching([](auto const& component_value) { return component_value.contains_attr_tainted_value(); }).has_value()) {
        auto serialized_input = Parser::serialize_component_values_for_reparsing(remaining_tokens);
        if (RustComponentValueParser::parse_optional_declaration_value_descriptor(serialized_input.bytes_as_string_view(), "utf-8"sv)) {
            while (tokens.has_next_token())
                tokens.discard_a_token();
            return RustComponentValueParser::parse_a_list_of_component_values(serialized_input.bytes_as_string_view(), "utf-8"sv);
        }
    }

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

// https://www.w3.org/TR/css-syntax-3/#urange-syntax
Optional<Gfx::UnicodeRange> Parser::parse_unicode_range(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto start = tokens.current_index();
    while (tokens.has_next_token())
        tokens.discard_a_token();

    auto serialized_unicode_range = serialize_component_values_for_reparsing(tokens.tokens_since(start));
    auto maybe_unicode_range = RustComponentValueParser::parse_a_unicode_range(serialized_unicode_range.bytes_as_string_view(), "utf-8"sv);
    if (!maybe_unicode_range.has_value())
        return {};

    transaction.commit();
    return maybe_unicode_range.release_value();
}

Vector<Gfx::UnicodeRange> Parser::parse_unicode_ranges(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto start = tokens.current_index();
    while (tokens.has_next_token())
        tokens.discard_a_token();

    auto serialized_unicode_ranges = serialize_component_values_for_reparsing(tokens.tokens_since(start));
    auto maybe_unicode_ranges = RustComponentValueParser::parse_a_unicode_range_list(serialized_unicode_ranges.bytes_as_string_view(), "utf-8"sv);
    if (!maybe_unicode_ranges.has_value())
        return {};

    transaction.commit();
    return maybe_unicode_ranges.release_value();
}

RefPtr<UnicodeRangeStyleValue const> Parser::parse_unicode_range_value(TokenStream<ComponentValue>& tokens)
{
    if (auto range = parse_unicode_range(tokens); range.has_value())
        return UnicodeRangeStyleValue::create(range.release_value());
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_integer_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    tokens.discard_whitespace();

    auto const& peek_token = tokens.next_token();
    if (peek_token.is(Token::Type::Number) && peek_token.token().is_integer() && accepted_range.contains(peek_token.token().to_integer())) {
        tokens.discard_a_token(); // integer
        return IntegerStyleValue::create(peek_token.token().to_integer());
    }

    if (auto calc = parse_calculated_value(peek_token, { .resolve_numbers_as_integers = true, .accepted_ranges_by_type = { { ValueType::Integer, accepted_range } } }); calc && calc->as_calculated().resolves_to_number()) {
        tokens.discard_a_token(); // calc
        return calc;
    }

    if (auto tree_counting_function = parse_tree_counting_function(tokens, TreeCountingFunctionStyleValue::ComputedType::Integer); tree_counting_function)
        return tree_counting_function;

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_number_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    tokens.discard_whitespace();

    auto const& peek_token = tokens.next_token();
    if (peek_token.is(Token::Type::Number) && accepted_range.contains(peek_token.token().number_value())) {
        tokens.discard_a_token(); // number
        return NumberStyleValue::create(peek_token.token().number_value());
    }

    if (auto calc = parse_calculated_value(peek_token, { .accepted_ranges_by_type = { { ValueType::Number, accepted_range } } }); calc && calc->as_calculated().resolves_to_number()) {
        tokens.discard_a_token(); // calc
        return calc;
    }

    if (auto tree_counting_function = parse_tree_counting_function(tokens, TreeCountingFunctionStyleValue::ComputedType::Number); tree_counting_function)
        return tree_counting_function;

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_number_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_number_range, NumericRange const& accepted_percentage_range)
{
    // Parses [<percentage> | <number>] (which is equivalent to [<alpha-value>])
    if (auto value = parse_number_value(tokens, accepted_number_range))
        return value;
    if (auto value = parse_percentage_value(tokens, accepted_percentage_range))
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

RefPtr<StyleValue const> Parser::parse_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    tokens.discard_whitespace();

    auto const& peek_token = tokens.next_token();
    if (peek_token.is(Token::Type::Percentage) && accepted_range.contains(peek_token.token().percentage())) {
        tokens.discard_a_token(); // percentage
        return PercentageStyleValue::create(Percentage(peek_token.token().percentage()));
    }

    if (auto calc = parse_calculated_value(peek_token, { .accepted_ranges_by_type = { { ValueType::Percentage, accepted_range } } }); calc && calc->as_calculated().resolves_to_percentage()) {
        tokens.discard_a_token(); // calc
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

static RefPtr<AngleStyleValue const> parse_literal_angle_value(TokenStream<ComponentValue>& tokens, bool is_parsing_svg_presentation_attribute, NumericRange const& accepted_range)
{
    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto& dimension_token = tokens.consume_a_token().token();
        if (auto angle_type = string_to_angle_unit(dimension_token.dimension_unit()); angle_type.has_value()) {
            Angle angle { dimension_token.dimension_value(), angle_type.release_value() };

            if (!accepted_range.contains(angle.to_degrees()))
                return nullptr;

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

        return AngleStyleValue::create(move(angle));
    }

    return nullptr;
}

static RefPtr<PercentageStyleValue const> parse_literal_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Percentage) && accepted_range.contains(tokens.next_token().token().percentage()))
        return PercentageStyleValue::create(Percentage { tokens.consume_a_token().token().percentage() });

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_angle_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    if (auto literal_angle = parse_literal_angle_value(tokens, is_parsing_svg_presentation_attribute(), accepted_range))
        return literal_angle;

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Angle, accepted_range } } }); calc && calc->as_calculated().resolves_to_angle()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_angle_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_angle_range, NumericRange const& accepted_percentage_range)
{
    if (auto literal_angle = parse_literal_angle_value(tokens, is_parsing_svg_presentation_attribute(), accepted_angle_range))
        return literal_angle;

    if (auto literal_percentage = parse_literal_percentage_value(tokens, accepted_percentage_range))
        return literal_percentage;

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .percentages_resolve_as = ValueType::Angle, .accepted_ranges_by_type = { { ValueType::Angle, { accepted_angle_range } } } }); calc && calc->as_calculated().resolves_to_angle_percentage()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_flex_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto& dimension_token = tokens.consume_a_token().token();
        if (auto flex_type = string_to_flex_unit(dimension_token.dimension_unit()); flex_type.has_value()) {
            Flex flex { (dimension_token.dimension_value()), flex_type.release_value() };

            if (!accepted_range.contains(flex.to_fr()))
                return nullptr;

            transaction.commit();
            return FlexStyleValue::create(move(flex));
        }
        return nullptr;
    }

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Flex, accepted_range } } }); calc && calc->as_calculated().resolves_to_flex()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

static RefPtr<FrequencyStyleValue const> parse_literal_frequency_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto& dimension_token = tokens.consume_a_token().token();
        if (auto frequency_type = string_to_frequency_unit(dimension_token.dimension_unit()); frequency_type.has_value()) {
            Frequency frequency { dimension_token.dimension_value(), frequency_type.release_value() };

            if (!accepted_range.contains(frequency.to_hertz()))
                return nullptr;

            transaction.commit();
            return FrequencyStyleValue::create(move(frequency));
        }
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_frequency_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    if (auto literal_frequency = parse_literal_frequency_value(tokens, accepted_range))
        return literal_frequency;

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Frequency, accepted_range } } }); calc && calc->as_calculated().resolves_to_frequency()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_frequency_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_frequency_range, NumericRange const& accepted_percentage_range)
{
    if (auto literal_frequency = parse_literal_frequency_value(tokens, accepted_frequency_range))
        return literal_frequency;

    if (auto literal_percentage = parse_literal_percentage_value(tokens, accepted_percentage_range))
        return literal_percentage;

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .percentages_resolve_as = ValueType::Frequency, .accepted_ranges_by_type = { { ValueType::Frequency, accepted_frequency_range } } }); calc && calc->as_calculated().resolves_to_frequency_percentage()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

static RefPtr<LengthStyleValue const> parse_literal_length_value(TokenStream<ComponentValue>& tokens, bool context_allows_quirky_length, bool is_parsing_svg_presentation_attribute, NumericRange const& accepted_range)
{
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

            transaction.commit();
            return LengthStyleValue::create(Length::make_px(0));
        }
        if (context_allows_quirky_length) {
            auto nearest_value = CSSPixels::nearest_value_for(numeric_value);

            if (!accepted_range.contains(nearest_value.to_double()))
                return nullptr;

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

            transaction.commit();
            return LengthStyleValue::create(Length::make_px(nearest_value));
        }
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_length_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    if (auto literal_length = parse_literal_length_value(tokens, context_allows_quirky_length(), is_parsing_svg_presentation_attribute(), accepted_range))
        return literal_length;

    if (tokens.next_token().is_function("anchor-size"sv))
        return parse_anchor_size(tokens);

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Length, accepted_range } } }); calc && calc->as_calculated().resolves_to_length()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_length_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_length_range, NumericRange const& accepted_percentage_range)
{
    if (auto literal_length = parse_literal_length_value(tokens, context_allows_quirky_length(), is_parsing_svg_presentation_attribute(), accepted_length_range))
        return literal_length;

    if (auto literal_percentage = parse_literal_percentage_value(tokens, accepted_percentage_range))
        return literal_percentage;

    if (tokens.next_token().is_function("anchor-size"sv))
        return parse_anchor_size(tokens);

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .percentages_resolve_as = ValueType::Length, .accepted_ranges_by_type = { { ValueType::Length, accepted_length_range } } }); calc && calc->as_calculated().resolves_to_length_percentage()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_resolution_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
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

            transaction.commit();
            return ResolutionStyleValue::create(move(resolution));
        }
        return nullptr;
    }

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Resolution, accepted_range } } }); calc && calc->as_calculated().resolves_to_resolution()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

static RefPtr<TimeStyleValue const> parse_literal_time_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Dimension)) {
        auto transaction = tokens.begin_transaction();
        auto const& dimension_token = tokens.consume_a_token().token();
        if (auto time_type = string_to_time_unit(dimension_token.dimension_unit()); time_type.has_value()) {
            Time time { dimension_token.dimension_value(), time_type.release_value() };

            if (!accepted_range.contains(time.to_seconds()))
                return nullptr;

            transaction.commit();
            return TimeStyleValue::create(move(time));
        }
    }

    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_time_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_range)
{
    if (auto literal_time = parse_literal_time_value(tokens, accepted_range))
        return literal_time;

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .accepted_ranges_by_type = { { ValueType::Time, accepted_range } } }); calc && calc->as_calculated().resolves_to_time()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_time_percentage_value(TokenStream<ComponentValue>& tokens, NumericRange const& accepted_time_range, NumericRange const& accepted_percentage_range)
{
    if (auto literal_time = parse_literal_time_value(tokens, accepted_time_range))
        return literal_time;

    if (auto literal_percentage = parse_literal_percentage_value(tokens, accepted_percentage_range))
        return literal_percentage;

    auto transaction = tokens.begin_transaction();
    if (auto calc = parse_calculated_value(tokens.consume_a_token(), { .percentages_resolve_as = ValueType::Time, .accepted_ranges_by_type = { { ValueType::Time, accepted_time_range } } }); calc && calc->as_calculated().resolves_to_time_percentage()) {
        transaction.commit();
        return calc;
    }
    return nullptr;
}

// https://drafts.csswg.org/scroll-animations-1/#view-timeline-inset
RefPtr<StyleValue const> Parser::parse_view_timeline_inset_value(TokenStream<ComponentValue>& tokens)
{
    // [ [ auto | <length-percentage> ]{1,2} ]
    auto transaction = tokens.begin_transaction();

    StyleValueVector inset_values;

    while (tokens.has_next_token() && inset_values.size() < 2) {
        tokens.discard_whitespace();

        if (tokens.next_token().is_ident("auto"sv)) {
            tokens.discard_a_token(); // auto
            inset_values.append(KeywordStyleValue::create(Keyword::Auto));
            continue;
        }

        if (auto length_percentage = parse_length_percentage_value(tokens, infinite_range, infinite_range)) {
            inset_values.append(length_percentage.release_nonnull());
            continue;
        }

        break;
    }

    if (inset_values.is_empty())
        return nullptr;

    transaction.commit();

    // If the second value is omitted, it is set to the first.
    if (inset_values.size() == 1)
        return StyleValueList::create({ inset_values[0], inset_values[0] }, StyleValueList::Separator::Space);

    return StyleValueList::create(move(inset_values), StyleValueList::Separator::Space);
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
RefPtr<FunctionStyleValue const> Parser::parse_scroll_function_value(TokenStream<ComponentValue>& tokens)
{
    // <scroll()> = scroll( [ <scroller> || <axis> ]? )
    auto transaction = tokens.begin_transaction();
    auto const& function_token = tokens.consume_a_token();
    if (!function_token.is_function("scroll"sv))
        return nullptr;

    auto serialized_scroll_function = function_token.original_source_text();
    if (serialized_scroll_function.is_empty())
        serialized_scroll_function = function_token.to_string();
    auto scroll_function = RustComponentValueParser::parse_scroll_function(serialized_scroll_function.bytes_as_string_view(), "utf-8"sv);
    if (scroll_function.kind == FFI::CssScrollFunctionValueKind::Invalid)
        return nullptr;

    StyleValueTuple tuple;
    tuple.resize_with_default_value(2, nullptr);

    switch (scroll_function.scroller) {
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

    switch (scroll_function.axis) {
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

    transaction.commit();
    return FunctionStyleValue::create("scroll"_fly_string, TupleStyleValue::create(move(tuple)));
}

// https://drafts.csswg.org/scroll-animations-1/#funcdef-view
RefPtr<FunctionStyleValue const> Parser::parse_view_function_value(TokenStream<ComponentValue>& tokens)
{
    // <view()> = view( [ <axis> || <'view-timeline-inset'> ]? )
    auto transaction = tokens.begin_transaction();
    auto const& function_token = tokens.consume_a_token();
    if (!function_token.is_function("view"sv))
        return nullptr;

    auto serialized_view_function = function_token.original_source_text();
    if (serialized_view_function.is_empty())
        serialized_view_function = function_token.to_string();
    auto view_function = RustComponentValueParser::parse_view_function(serialized_view_function.bytes_as_string_view(), "utf-8"sv);
    if (view_function.kind == FFI::CssViewFunctionValueKind::Invalid)
        return nullptr;

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "view"sv });

    StyleValueTuple tuple;
    tuple.resize_with_default_value(2, nullptr);

    switch (view_function.axis) {
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

    switch (view_function.inset) {
    case FFI::CssViewFunctionInsetKind::None:
    case FFI::CssViewFunctionInsetKind::Default:
        break;
    case FFI::CssViewFunctionInsetKind::NonDefault: {
        auto argument_tokens = TokenStream { function_token.function().value };
        if (view_function.inset_position == FFI::CssViewFunctionInsetPosition::AfterAxis) {
            argument_tokens.discard_whitespace();
            argument_tokens.discard_a_token();
        }

        auto inset_value = parse_view_timeline_inset_value(argument_tokens);
        if (!inset_value)
            return nullptr;

        tuple[TupleStyleValue::Indices::ViewFunction::Inset] = inset_value.release_nonnull();
        break;
    }
    }

    transaction.commit();
    return FunctionStyleValue::create("view"_fly_string, TupleStyleValue::create(move(tuple)));
}

// https://www.w3.org/TR/CSS2/visufx.html#value-def-shape
RefPtr<StyleValue const> Parser::parse_rect_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto const& function_token = tokens.consume_a_token();
    if (!function_token.is_function("rect"sv))
        return nullptr;

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { "rect"sv });

    StyleValueVector params;
    params.ensure_capacity(4);

    auto argument_tokens = TokenStream { function_token.function().value };

    enum class CommaRequirement {
        Unknown,
        RequiresCommas,
        RequiresNoCommas
    };

    enum class Side {
        Top = 0,
        Right = 1,
        Bottom = 2,
        Left = 3
    };

    auto comma_requirement = CommaRequirement::Unknown;

    // In CSS 2.1, the only valid <shape> value is: rect(<top>, <right>, <bottom>, <left>) where
    // <top> and <bottom> specify offsets from the top border edge of the box, and <right>, and
    //  <left> specify offsets from the left border edge of the box.
    for (size_t side = 0; side < 4; side++) {
        argument_tokens.discard_whitespace();

        // <top>, <right>, <bottom>, and <left> may either have a <length> value or 'auto'.
        // Negative lengths are permitted.
        if (argument_tokens.next_token().is_ident("auto"sv)) {
            (void)argument_tokens.consume_a_token(); // `auto`
            params.append(KeywordStyleValue::create(Keyword::Auto));
        } else {
            auto maybe_length = parse_length_value(argument_tokens, infinite_range);
            if (!maybe_length)
                return nullptr;

            params.append(maybe_length.release_nonnull());
        }
        argument_tokens.discard_whitespace();

        // The last side, should be no more tokens following it.
        if (static_cast<Side>(side) == Side::Left) {
            if (argument_tokens.has_next_token())
                return nullptr;
            break;
        }

        bool next_is_comma = argument_tokens.next_token().is(Token::Type::Comma);

        // Authors should separate offset values with commas. User agents must support separation
        // with commas, but may also support separation without commas (but not a combination),
        // because a previous revision of this specification was ambiguous in this respect.
        if (comma_requirement == CommaRequirement::Unknown)
            comma_requirement = next_is_comma ? CommaRequirement::RequiresCommas : CommaRequirement::RequiresNoCommas;

        if (comma_requirement == CommaRequirement::RequiresCommas) {
            if (next_is_comma)
                argument_tokens.discard_a_token();
            else
                return nullptr;
        } else if (comma_requirement == CommaRequirement::RequiresNoCommas) {
            if (next_is_comma)
                return nullptr;
        } else {
            VERIFY_NOT_REACHED();
        }
    }

    transaction.commit();
    return RectStyleValue::create(params[0], params[1], params[2], params[3]);
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
RefPtr<StyleValue const> Parser::parse_color_value(TokenStream<ComponentValue>& tokens)
{

    // Keywords: <system-color> | <deprecated-color> | currentColor
    {
        auto transaction = tokens.begin_transaction();
        if (auto keyword = parse_keyword_value(tokens); keyword && keyword->has_color()) {
            transaction.commit();
            return keyword;
        }
    }

    // Functions
    if (auto color = parse_color_function(tokens))
        return color;

    if (auto color = parse_color_mix_function(tokens))
        return color;

    if (auto rgb = parse_rgb_color_value(tokens))
        return rgb;
    if (auto hsl = parse_hsl_color_value(tokens))
        return hsl;
    if (auto hwb = parse_hwb_color_value(tokens))
        return hwb;
    if (auto lab = parse_lab_color_value(tokens))
        return lab;
    if (auto lch = parse_lch_color_value(tokens))
        return lch;
    if (auto oklab = parse_oklab_color_value(tokens))
        return oklab;
    if (auto oklch = parse_oklch_color_value(tokens))
        return oklch;
    if (auto light_dark = parse_light_dark_color_value(tokens))
        return light_dark;

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    auto const& component_value = tokens.consume_a_token();

    if (component_value.is(Token::Type::Ident)) {
        auto ident = component_value.token().ident();

        auto color = Color::from_string(ident);
        if (color.has_value()) {
            transaction.commit();
            return ColorStyleValue::create_from_color(color.release_value(), ColorSyntax::Legacy, ident);
        }
        // Otherwise, fall through to the hashless-hex-color case
    }

    if (component_value.is(Token::Type::Hash)) {
        auto color = Color::from_string(MUST(String::formatted("#{}", component_value.token().hash_value())));
        if (color.has_value()) {
            transaction.commit();
            return ColorStyleValue::create_from_color(color.release_value(), ColorSyntax::Legacy);
        }
        return {};
    }

    // https://drafts.csswg.org/css-color-4/#quirky-color
    if (in_quirks_mode()) {
        // "When CSS is being parsed in quirks mode, <quirky-color> is a type of <color> that is only valid in certain properties:"
        // (NOTE: List skipped for brevity; quirks data is assigned in Properties.json)
        // "It is not valid in properties that include or reference these properties, such as the background shorthand,
        // or inside functional notations such as color-mix()"

        bool quirky_color_allowed = false;
        if (!m_value_context.is_empty()) {
            quirky_color_allowed = m_value_context.first().visit(
                [](PropertyID const& property_id) { return property_has_quirk(property_id, Quirk::HashlessHexColor); },
                [](auto const&) { return false; });
        }
        for (auto i = 1u; i < m_value_context.size() && quirky_color_allowed; i++) {
            quirky_color_allowed = m_value_context[i].visit(
                [](PropertyID const& property_id) { return property_has_quirk(property_id, Quirk::HashlessHexColor); },
                [](auto const&) { return false; });
        }
        if (quirky_color_allowed) {
            // NOTE: This algorithm is no longer in the spec, since the concept got moved and renamed. However, it works,
            //       and so we might as well keep using it.

            // The value of a quirky color is obtained from the possible component values using the following algorithm,
            // aborting on the first step that returns a value:

            // 1. Let cv be the component value.
            auto const& cv = component_value;
            String serialization;
            // 2. If cv is a <number-token> or a <dimension-token>, follow these substeps:
            if (cv.is(Token::Type::Number) || cv.is(Token::Type::Dimension)) {
                // 1. If cv’s type flag is not "integer", return an error.
                //    This means that values that happen to use scientific notation, e.g., 5e5e5e, will fail to parse.
                if (!cv.token().is_integer())
                    return {};

                // 2. If cv’s value is less than zero, return an error.
                auto value = cv.is(Token::Type::Number) ? cv.token().to_integer() : cv.token().dimension_value_int();
                if (value < 0)
                    return {};

                // 3. Let serialization be the serialization of cv’s value, as a base-ten integer using digits 0-9 (U+0030 to U+0039) in the shortest form possible.
                StringBuilder serialization_builder;
                serialization_builder.appendff("{}", value);

                // 4. If cv is a <dimension-token>, append the unit to serialization.
                if (cv.is(Token::Type::Dimension))
                    serialization_builder.append(cv.token().dimension_unit());

                // 5. If serialization consists of fewer than six characters, prepend zeros (U+0030) so that it becomes six characters.
                serialization = MUST(serialization_builder.to_string());
                if (serialization_builder.length() < 6) {
                    StringBuilder builder;
                    for (size_t i = 0; i < (6 - serialization_builder.length()); i++)
                        builder.append('0');
                    builder.append(serialization_builder.string_view());
                    serialization = MUST(builder.to_string());
                }
            }
            // 3. Otherwise, cv is an <ident-token>; let serialization be cv’s value.
            else {
                if (!cv.is(Token::Type::Ident))
                    return {};
                serialization = cv.token().ident().to_string();
            }

            // 4. If serialization does not consist of three or six characters, return an error.
            if (serialization.bytes().size() != 3 && serialization.bytes().size() != 6)
                return {};

            // 5. If serialization contains any characters not in the range [0-9A-Fa-f] (U+0030 to U+0039, U+0041 to U+0046, U+0061 to U+0066), return an error.
            for (auto c : serialization.bytes_as_string_view()) {
                if (!((c >= '0' && c <= '9') || (c >= 'A' && c <= 'F') || (c >= 'a' && c <= 'f')))
                    return {};
            }

            // 6. Return the concatenation of "#" (U+0023) and serialization.
            auto color = Color::from_string(MUST(String::formatted("#{}", serialization)));
            if (color.has_value()) {
                transaction.commit();
                return ColorStyleValue::create_from_color(color.release_value(), ColorSyntax::Legacy);
            }
        }
    }

    return {};
}

// https://drafts.csswg.org/css-borders-4/#typedef-corner-shape-value
RefPtr<StyleValue const> Parser::parse_corner_shape_value(TokenStream<ComponentValue>& tokens)
{
    // <corner-shape-value> = round | scoop | bevel | notch | square | squircle | <superellipse()>
    auto transaction = tokens.begin_transaction();

    tokens.discard_whitespace();

    auto token = tokens.consume_a_token();

    if (token.is(Token::Type::Ident)) {
        auto keyword = keyword_from_string(token.token().ident());

        if (!keyword.has_value())
            return nullptr;

        if (!first_is_one_of(keyword, Keyword::Round, Keyword::Scoop, Keyword::Bevel, Keyword::Notch, Keyword::Square, Keyword::Squircle))
            return nullptr;

        transaction.commit();
        return KeywordStyleValue::create(keyword.value());
    }

    if (token.is_function("superellipse"sv)) {
        // superellipse() = superellipse(<number> | infinity | -infinity)
        auto const& function = token.function();

        auto context_guard = push_temporary_value_parsing_context(FunctionContext { function.name });

        TokenStream function_tokens { function.value };

        function_tokens.discard_whitespace();

        if (parse_all_as_single_keyword_value(function_tokens, Keyword::NegativeInfinity)) {
            transaction.commit();
            return SuperellipseStyleValue::create(NumberStyleValue::create(-AK::Infinity<double>));
        }

        if (parse_all_as_single_keyword_value(function_tokens, Keyword::Infinity)) {
            transaction.commit();
            return SuperellipseStyleValue::create(NumberStyleValue::create(AK::Infinity<double>));
        }

        if (auto number_value = parse_number_value(function_tokens, infinite_range); number_value) {
            function_tokens.discard_whitespace();

            if (function_tokens.has_next_token())
                return nullptr;

            transaction.commit();
            return SuperellipseStyleValue::create(number_value.release_nonnull());
        }
    }

    return nullptr;
}

// https://drafts.csswg.org/css-lists-3/#counter-functions
RefPtr<StyleValue const> Parser::parse_counter_value(TokenStream<ComponentValue>& tokens)
{
    auto parse_counter_name = [this](TokenStream<ComponentValue>& tokens) -> Optional<FlyString> {
        // https://drafts.csswg.org/css-lists-3/#typedef-counter-name
        // Counters are referred to in CSS syntax using the <counter-name> type, which represents
        // their name as a <custom-ident>. A <counter-name> name cannot match the keyword none;
        // such an identifier is invalid as a <counter-name>.
        auto transaction = tokens.begin_transaction();
        tokens.discard_whitespace();

        auto counter_name = parse_custom_ident_value(tokens, { { "none"sv } });
        if (!counter_name)
            return {};

        tokens.discard_whitespace();
        if (tokens.has_next_token())
            return {};

        transaction.commit();
        return counter_name->custom_ident();
    };

    auto parse_counter_style = [this](TokenStream<ComponentValue>& tokens) -> RefPtr<StyleValue const> {
        auto transaction = tokens.begin_transaction();
        tokens.discard_whitespace();

        auto counter_style = parse_counter_style_value(tokens);
        if (!counter_style)
            return {};

        tokens.discard_whitespace();
        if (tokens.has_next_token())
            return {};

        transaction.commit();
        return counter_style.release_nonnull();
    };

    auto transaction = tokens.begin_transaction();
    auto const& token = tokens.consume_a_token();
    if (token.is_function("counter"sv)) {
        // counter() = counter( <counter-name>, <counter-style>? )
        auto& function = token.function();
        auto context_guard = push_temporary_value_parsing_context(FunctionContext { function.name });

        TokenStream function_tokens { function.value };
        auto function_values = parse_a_comma_separated_list_of_component_values(function_tokens);
        if (function_values.is_empty() || function_values.size() > 2)
            return nullptr;

        TokenStream name_tokens { function_values[0] };
        auto counter_name = parse_counter_name(name_tokens);
        if (!counter_name.has_value())
            return nullptr;

        RefPtr<StyleValue const> counter_style;
        if (function_values.size() > 1) {
            TokenStream counter_style_tokens { function_values[1] };
            counter_style = parse_counter_style(counter_style_tokens);
            if (!counter_style)
                return nullptr;
        } else {
            // In both cases, if the <counter-style> argument is omitted it defaults to `decimal`.
            counter_style = CounterStyleStyleValue::create("decimal"_fly_string);
        }

        transaction.commit();
        return CounterStyleValue::create_counter(counter_name.release_value(), counter_style.release_nonnull());
    }

    if (token.is_function("counters"sv)) {
        // counters() = counters( <counter-name>, <string>, <counter-style>? )
        auto& function = token.function();
        auto context_guard = push_temporary_value_parsing_context(FunctionContext { function.name });

        TokenStream function_tokens { function.value };
        auto function_values = parse_a_comma_separated_list_of_component_values(function_tokens);
        if (function_values.size() < 2 || function_values.size() > 3)
            return nullptr;

        TokenStream name_tokens { function_values[0] };
        auto counter_name = parse_counter_name(name_tokens);
        if (!counter_name.has_value())
            return nullptr;

        TokenStream string_tokens { function_values[1] };
        string_tokens.discard_whitespace();
        auto join_string = parse_string_value(string_tokens);
        string_tokens.discard_whitespace();
        if (!join_string || string_tokens.has_next_token())
            return nullptr;

        RefPtr<StyleValue const> counter_style;
        if (function_values.size() > 2) {
            TokenStream counter_style_tokens { function_values[2] };
            counter_style = parse_counter_style(counter_style_tokens);
            if (!counter_style)
                return nullptr;
        } else {
            // In both cases, if the <counter-style> argument is omitted it defaults to `decimal`.
            counter_style = CounterStyleStyleValue::create("decimal"_fly_string);
        }

        transaction.commit();
        return CounterStyleValue::create_counters(counter_name.release_value(), join_string->string_value(), counter_style.release_nonnull());
    }

    return nullptr;
}

// https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style-name
Optional<FlyString> Parser::parse_counter_style_name(TokenStream<ComponentValue>& tokens)
{
    // <counter-style-name> is a <custom-ident> that is not an ASCII case-insensitive match for none.
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    auto const& component_value = tokens.next_token();
    auto original_source_text = component_value.original_source_text();
    auto source = original_source_text.is_empty() ? component_value.to_string() : original_source_text;

    auto counter_style_name = RustComponentValueParser::parse_a_counter_style_name(source.bytes_as_string_view(), "utf-8"sv);
    if (!counter_style_name.has_value())
        return {};
    tokens.discard_a_token();

    // https://drafts.csswg.org/css-counter-styles-3/#the-counter-style-rule
    // Counter style names are case-sensitive. However, the names defined in this specification are ASCII lowercased
    // on parse wherever they are used as counter styles, e.g. in the list-style set of properties, in the
    // @counter-style rule, and in the counter() functions.

    // NB: The "names defined in this specification" are defined in the `CounterStyleNameKeyword` enum
    auto const& keyword = keyword_from_string(counter_style_name.value());
    if (keyword.has_value() && keyword_to_counter_style_name_keyword(keyword.value()).has_value())
        counter_style_name = counter_style_name->to_ascii_lowercase();

    transaction.commit();
    return counter_style_name;
}

// https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style
RefPtr<StyleValue const> Parser::parse_counter_style_value(TokenStream<ComponentValue>& tokens)
{
    // <counter-style> = <counter-style-name> | <symbols()>
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return nullptr;

    auto start = tokens.current_index();
    tokens.discard_a_token();
    auto serialized_counter_style = serialize_component_values_for_reparsing(tokens.tokens_since(start));
    auto maybe_counter_style = RustComponentValueParser::parse_a_counter_style(serialized_counter_style.bytes_as_string_view(), "utf-8"sv);
    if (!maybe_counter_style.has_value())
        return nullptr;

    auto counter_style = maybe_counter_style.release_value();
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

        transaction.commit();
        return CounterStyleStyleValue::create(counter_style_name);
    }

    if (counter_style.kind == FFI::CssCounterStyleKind::SymbolsFunction) {
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
        transaction.commit();
        return CounterStyleStyleValue::create(CounterStyleStyleValue::SymbolsFunction { symbols_type, move(counter_style.symbols) });
    }

    VERIFY_NOT_REACHED();
}

// https://drafts.csswg.org/css-values-4/#ratios
RefPtr<StyleValue const> Parser::parse_ratio_value(TokenStream<ComponentValue>& tokens)
{
    // <ratio> = <number [0,∞]> [ / <number [0,∞]> ]?
    auto transaction = tokens.begin_transaction();

    tokens.discard_whitespace();

    auto numerator = parse_number_value(tokens, non_negative_range);

    if (!numerator)
        return nullptr;

    tokens.discard_whitespace();

    if (tokens.next_token().is(Token::Type::Delim) && tokens.next_token().token().delim() == '/') {
        tokens.discard_a_token();
        tokens.discard_whitespace();

        auto denominator = parse_number_value(tokens, non_negative_range);
        if (!denominator)
            return nullptr;

        transaction.commit();
        return RatioStyleValue::create(numerator.release_nonnull(), denominator.release_nonnull());
    }

    transaction.commit();
    // The second <number> is optional, defaulting to 1.
    return RatioStyleValue::create(numerator.release_nonnull(), NumberStyleValue::create(1));
}

RefPtr<StringStyleValue const> Parser::parse_string_value(TokenStream<ComponentValue>& tokens)
{
    tokens.discard_whitespace();
    auto const& peek = tokens.next_token();
    if (peek.is(Token::Type::String)) {
        tokens.discard_a_token();
        return StringStyleValue::create(peek.token().string());
    }

    return nullptr;
}

RefPtr<AbstractImageStyleValue const> Parser::parse_image_value(TokenStream<ComponentValue>& tokens)
{
    return parse_image_value(tokens, AllowImageSet::Yes);
}

RefPtr<ImageSetStyleValue const> Parser::parse_image_set_function(TokenStream<ComponentValue>& tokens)
{
    tokens.discard_whitespace();
    auto const& function_token = tokens.next_token();
    if (!function_token.is_function("image-set"sv) && !function_token.is_function("-webkit-image-set"sv))
        return nullptr;

    auto transaction = tokens.begin_transaction();
    auto const& function = tokens.consume_a_token().function();
    TokenStream function_tokens { function.value };
    auto image_set_options_tokens = parse_a_comma_separated_list_of_component_values(function_tokens);
    function_tokens.discard_whitespace();
    if (!function_tokens.is_empty())
        return nullptr;

    Vector<ImageSetStyleValue::Option> options;
    options.ensure_capacity(image_set_options_tokens.size());
    for (auto const& option_tokens_list : image_set_options_tokens) {
        if (option_tokens_list.first_matching([](auto const& component_value) { return component_value.contains_attr_tainted_value(); }).has_value())
            return nullptr;

        TokenStream option_tokens { option_tokens_list };
        option_tokens.discard_whitespace();

        RefPtr<AbstractImageStyleValue const> image;
        if (option_tokens.next_token().is(Token::Type::String)) {
            auto url = URL { option_tokens.consume_a_token().token().string().to_string() };
            image = ImageStyleValue::create(url);
        } else {
            image = parse_image_value(option_tokens, AllowImageSet::No);
        }
        if (!image)
            return nullptr;

        RefPtr<StyleValue const> resolution;
        Optional<String> type;
        while (true) {
            option_tokens.discard_whitespace();
            if (option_tokens.is_empty())
                break;

            if (!resolution) {
                if (auto parsed_resolution = parse_resolution_value(option_tokens, infinite_range)) {
                    resolution = parsed_resolution;
                    continue;
                }
            }

            if (!type.has_value() && option_tokens.next_token().is_function("type"sv)) {
                auto const& type_function = option_tokens.consume_a_token().function();
                TokenStream type_tokens { type_function.value };
                type_tokens.discard_whitespace();
                if (!type_tokens.next_token().is(Token::Type::String))
                    return nullptr;
                type = type_tokens.consume_a_token().token().string().to_string();
                type_tokens.discard_whitespace();
                if (!type_tokens.is_empty())
                    return nullptr;
                continue;
            }

            return nullptr;
        }

        if (!resolution)
            resolution = ResolutionStyleValue::create(Resolution { 1, ResolutionUnit::X });

        options.unchecked_append({
            .image = image.release_nonnull(),
            .resolution = resolution.release_nonnull(),
            .type = move(type),
        });
    }

    if (options.is_empty())
        return nullptr;

    transaction.commit();
    return ImageSetStyleValue::create(move(options));
}

RefPtr<AbstractImageStyleValue const> Parser::parse_image_value(TokenStream<ComponentValue>& tokens, AllowImageSet allow_image_set)
{
    tokens.mark();
    auto url = parse_url_function(tokens);
    if (url.has_value()) {
        // If the value is a 'url(..)' parse as image, but if it is just a reference 'url(#xx)', leave it alone,
        // so we can parse as URL further on. These URLs are used as references inside SVG documents for masks.
        // FIXME: Remove this special case once mask-image accepts `<image>`.
        if (!url->url().starts_with('#')) {
            tokens.discard_a_mark();
            return ImageStyleValue::create(url.release_value());
        }
        tokens.restore_a_mark();
        return nullptr;
    }
    tokens.discard_a_mark();

    if (allow_image_set == AllowImageSet::Yes) {
        if (auto image_set = parse_image_set_function(tokens))
            return image_set;
    }

    if (auto linear_gradient = parse_linear_gradient_function(tokens))
        return linear_gradient;

    if (auto conic_gradient = parse_conic_gradient_function(tokens))
        return conic_gradient;

    if (auto radial_gradient = parse_radial_gradient_function(tokens))
        return radial_gradient;

    return nullptr;
}

// https://svgwg.org/svg2-draft/painting.html#SpecifyingPaint
RefPtr<StyleValue const> Parser::parse_paint_value(TokenStream<ComponentValue>& tokens)
{
    // `<paint> = none | <color> | <url> [none | <color>]? | context-fill | context-stroke`

    auto parse_color_or_none = [&]() -> Optional<RefPtr<StyleValue const>> {
        if (auto color = parse_color_value(tokens))
            return color;

        // NOTE: <color> also accepts identifiers, so we do this identifier check last.
        if (tokens.next_token().is(Token::Type::Ident)) {
            auto maybe_keyword = keyword_from_string(tokens.next_token().token().ident());
            if (maybe_keyword.has_value()) {
                // FIXME: Accept `context-fill` and `context-stroke`
                switch (*maybe_keyword) {
                case Keyword::None:
                    tokens.discard_a_token();
                    return KeywordStyleValue::create(*maybe_keyword);
                default:
                    return nullptr;
                }
            }
        }

        return OptionalNone {};
    };

    // FIXME: Allow context-fill/context-stroke here
    if (auto color_or_none = parse_color_or_none(); color_or_none.has_value())
        return *color_or_none;

    if (auto url = parse_url_value(tokens)) {
        tokens.discard_whitespace();
        if (auto color_or_none = parse_color_or_none(); color_or_none == nullptr) {
            // Fail to parse if the fallback is invalid, but otherwise ignore it.
            return nullptr;
        } else if (color_or_none.has_value() && *color_or_none && (*color_or_none)->has_color()) {
            return URLStyleValue::create(url->as_url().url(), color_or_none->release_nonnull());
        }
        return url;
    }

    return nullptr;
}

// https://www.w3.org/TR/css-values-4/#position
RefPtr<PositionStyleValue const> Parser::parse_position_value(TokenStream<ComponentValue>& tokens, PositionParsingMode position_parsing_mode)
{
    auto parse_position_edge = [](TokenStream<ComponentValue>& tokens) -> Optional<PositionEdge> {
        auto transaction = tokens.begin_transaction();
        auto& token = tokens.consume_a_token();
        if (!token.is(Token::Type::Ident))
            return {};
        auto keyword = keyword_from_string(token.token().ident());
        if (!keyword.has_value())
            return {};
        transaction.commit();
        return keyword_to_position_edge(*keyword);
    };

    auto is_horizontal = [](PositionEdge edge, bool accept_center) -> bool {
        switch (edge) {
        case PositionEdge::Left:
        case PositionEdge::Right:
            return true;
        case PositionEdge::Center:
            return accept_center;
        default:
            return false;
        }
    };

    auto is_vertical = [](PositionEdge edge, bool accept_center) -> bool {
        switch (edge) {
        case PositionEdge::Top:
        case PositionEdge::Bottom:
            return true;
        case PositionEdge::Center:
            return accept_center;
        default:
            return false;
        }
    };

    // <position> = [
    //   [ left | center | right | top | bottom | <length-percentage> ]
    // |
    //   [ left | center | right ] && [ top | center | bottom ]
    // |
    //   [ left | center | right | <length-percentage> ]
    //   [ top | center | bottom | <length-percentage> ]
    // |
    //   [ [ left | right ] <length-percentage> ] &&
    //   [ [ top | bottom ] <length-percentage> ]
    // ]

    // [ left | center | right | top | bottom | <length-percentage> ]
    auto alternative_1 = [&]() -> RefPtr<PositionStyleValue const> {
        auto transaction = tokens.begin_transaction();

        tokens.discard_whitespace();

        // [ left | center | right | top | bottom ]
        if (auto maybe_edge = parse_position_edge(tokens); maybe_edge.has_value()) {
            auto edge = maybe_edge.release_value();
            transaction.commit();

            // [ left | right ]
            if (is_horizontal(edge, false))
                return PositionStyleValue::create(EdgeStyleValue::create(edge, {}), EdgeStyleValue::create(PositionEdge::Center, {}));

            // [ top | bottom ]
            if (is_vertical(edge, false))
                return PositionStyleValue::create(EdgeStyleValue::create(PositionEdge::Center, {}), EdgeStyleValue::create(edge, {}));

            // [ center ]
            VERIFY(edge == PositionEdge::Center);
            return PositionStyleValue::create(EdgeStyleValue::create(PositionEdge::Center, {}), EdgeStyleValue::create(PositionEdge::Center, {}));
        }

        // [ <length-percentage> ]
        if (auto maybe_percentage = parse_length_percentage_value(tokens, infinite_range, infinite_range)) {
            transaction.commit();
            return PositionStyleValue::create(EdgeStyleValue::create({}, maybe_percentage), EdgeStyleValue::create(PositionEdge::Center, {}));
        }

        return nullptr;
    };

    // [ left | center | right ] && [ top | center | bottom ]
    auto alternative_2 = [&]() -> RefPtr<PositionStyleValue const> {
        auto transaction = tokens.begin_transaction();

        tokens.discard_whitespace();

        // Parse out two position edges
        auto maybe_first_edge = parse_position_edge(tokens);
        if (!maybe_first_edge.has_value())
            return nullptr;

        auto first_edge = maybe_first_edge.release_value();
        tokens.discard_whitespace();

        auto maybe_second_edge = parse_position_edge(tokens);
        if (!maybe_second_edge.has_value())
            return nullptr;

        auto second_edge = maybe_second_edge.release_value();

        // If 'left' or 'right' is given, that position is X and the other is Y.
        // Conversely -
        // If 'top' or 'bottom' is given, that position is Y and the other is X.
        if (is_vertical(first_edge, false) || is_horizontal(second_edge, false))
            swap(first_edge, second_edge);

        // [ left | center | right ] [ top | bottom | center ]
        if (is_horizontal(first_edge, true) && is_vertical(second_edge, true)) {
            transaction.commit();
            return PositionStyleValue::create(EdgeStyleValue::create(first_edge, {}), EdgeStyleValue::create(second_edge, {}));
        }

        return nullptr;
    };

    // [ left | center | right | <length-percentage> ]
    // [ top | center | bottom | <length-percentage> ]
    auto alternative_3 = [&]() -> RefPtr<PositionStyleValue const> {
        auto transaction = tokens.begin_transaction();

        auto parse_position_or_length = [&](bool as_horizontal) -> RefPtr<EdgeStyleValue const> {
            tokens.discard_whitespace();

            if (auto maybe_position = parse_position_edge(tokens); maybe_position.has_value()) {
                auto position = maybe_position.release_value();
                bool valid = as_horizontal ? is_horizontal(position, true) : is_vertical(position, true);
                if (!valid)
                    return nullptr;
                return EdgeStyleValue::create(position, {});
            }

            auto maybe_length = parse_length_percentage_value(tokens, infinite_range, infinite_range);
            if (!maybe_length)
                return nullptr;

            return EdgeStyleValue::create({}, maybe_length);
        };

        // [ left | center | right | <length-percentage> ]
        auto horizontal_edge = parse_position_or_length(true);
        if (!horizontal_edge)
            return nullptr;

        // [ top | center | bottom | <length-percentage> ]
        auto vertical_edge = parse_position_or_length(false);
        if (!vertical_edge)
            return nullptr;

        transaction.commit();
        return PositionStyleValue::create(horizontal_edge.release_nonnull(), vertical_edge.release_nonnull());
    };

    // [ [ left | right ] <length-percentage> ] &&
    // [ [ top | bottom ] <length-percentage> ]
    auto alternative_4 = [&]() -> RefPtr<PositionStyleValue const> {
        struct PositionAndLength {
            PositionEdge position;
            NonnullRefPtr<StyleValue const> length;
        };

        auto parse_position_and_length = [&]() -> Optional<PositionAndLength> {
            tokens.discard_whitespace();

            auto maybe_position = parse_position_edge(tokens);
            if (!maybe_position.has_value())
                return {};

            tokens.discard_whitespace();

            auto maybe_length = parse_length_percentage_value(tokens, infinite_range, infinite_range);
            if (!maybe_length)
                return {};

            return PositionAndLength {
                .position = maybe_position.release_value(),
                .length = maybe_length.release_nonnull(),
            };
        };

        auto transaction = tokens.begin_transaction();

        auto maybe_group1 = parse_position_and_length();
        if (!maybe_group1.has_value())
            return nullptr;

        auto maybe_group2 = parse_position_and_length();
        if (!maybe_group2.has_value())
            return nullptr;

        auto group1 = maybe_group1.release_value();
        auto group2 = maybe_group2.release_value();

        // [ [ left | right ] <length-percentage> ] [ [ top | bottom ] <length-percentage> ]
        if (is_horizontal(group1.position, false) && is_vertical(group2.position, false)) {
            transaction.commit();
            return PositionStyleValue::create(EdgeStyleValue::create(group1.position, group1.length), EdgeStyleValue::create(group2.position, group2.length));
        }

        // [ [ top | bottom ] <length-percentage> ] [ [ left | right ] <length-percentage> ]
        if (is_vertical(group1.position, false) && is_horizontal(group2.position, false)) {
            transaction.commit();
            return PositionStyleValue::create(EdgeStyleValue::create(group2.position, group2.length), EdgeStyleValue::create(group1.position, group1.length));
        }

        return nullptr;
    };

    // The extra 3-value syntax that's allowed for background-position:
    // [ center | [ left | right ] <length-percentage>? ] &&
    // [ center | [ top | bottom ] <length-percentage>? ]
    auto alternative_5_for_background_position = [&]() -> RefPtr<PositionStyleValue const> {
        auto transaction = tokens.begin_transaction();

        struct PositionAndMaybeLength {
            PositionEdge position;
            RefPtr<StyleValue const> length;
        };

        // [ <position> <length-percentage>? ]
        auto parse_position_and_maybe_length = [&]() -> Optional<PositionAndMaybeLength> {
            auto inner_transaction = tokens.begin_transaction();
            tokens.discard_whitespace();

            auto maybe_position = parse_position_edge(tokens);
            if (!maybe_position.has_value())
                return {};

            tokens.discard_whitespace();

            auto maybe_length = parse_length_percentage_value(tokens, infinite_range, infinite_range);
            if (maybe_length) {
                // 'center' cannot be followed by a <length-percentage>
                if (maybe_position.value() == PositionEdge::Center && maybe_length)
                    return {};
            }

            inner_transaction.commit();
            return PositionAndMaybeLength {
                .position = maybe_position.release_value(),
                .length = maybe_length,
            };
        };

        auto maybe_group1 = parse_position_and_maybe_length();
        if (!maybe_group1.has_value())
            return nullptr;

        auto maybe_group2 = parse_position_and_maybe_length();
        if (!maybe_group2.has_value())
            return nullptr;

        auto group1 = maybe_group1.release_value();
        auto group2 = maybe_group2.release_value();

        // 2-value or 4-value if both <length-percentage>s are present or missing.
        if ((group1.length && group2.length) || (!group1.length && !group2.length))
            return nullptr;

        // If 'left' or 'right' is given, that position is X and the other is Y.
        // Conversely -
        // If 'top' or 'bottom' is given, that position is Y and the other is X.
        if (is_vertical(group1.position, false) || is_horizontal(group2.position, false))
            swap(group1, group2);

        // [ center | [ left | right ] ]
        if (!is_horizontal(group1.position, true))
            return nullptr;

        // [ center | [ top | bottom ] ]
        if (!is_vertical(group2.position, true))
            return nullptr;

        auto to_style_value = [&](PositionAndMaybeLength const& group) -> NonnullRefPtr<EdgeStyleValue const> {
            if (group.position == PositionEdge::Center)
                return EdgeStyleValue::create(PositionEdge::Center, {});

            return EdgeStyleValue::create(group.position, group.length);
        };

        transaction.commit();
        return PositionStyleValue::create(to_style_value(group1), to_style_value(group2));
    };

    // Note: The alternatives must be attempted in this order since shorter alternatives can match a prefix of longer ones.
    if (auto position = alternative_4())
        return position;
    if (position_parsing_mode == PositionParsingMode::BackgroundPosition) {
        if (auto position = alternative_5_for_background_position())
            return position;
    }
    if (auto position = alternative_3())
        return position;
    if (auto position = alternative_2())
        return position;
    if (auto position = alternative_1())
        return position;
    return nullptr;
}

RefPtr<StyleValue const> Parser::parse_easing_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();

    tokens.discard_whitespace();

    auto const& part = tokens.consume_a_token();

    if (part.is(Token::Type::Ident)) {
        auto name = part.token().ident();
        auto maybe_simple_easing = [&] -> RefPtr<EasingStyleValue const> {
            if (name.equals_ignoring_ascii_case("step-start"sv))
                return EasingStyleValue::create(EasingStyleValue::Steps { IntegerStyleValue::create(1), StepPosition::Start });
            if (name.equals_ignoring_ascii_case("step-end"sv))
                return EasingStyleValue::create(EasingStyleValue::Steps { IntegerStyleValue::create(1), StepPosition::End });
            return {};
        }();

        if (!maybe_simple_easing)
            return nullptr;

        transaction.commit();
        return maybe_simple_easing;
    }

    if (!part.is_function())
        return nullptr;

    TokenStream argument_tokens { part.function().value };
    auto comma_separated_arguments = parse_a_comma_separated_list_of_component_values(argument_tokens);

    // Remove whitespace
    for (auto& argument : comma_separated_arguments)
        argument.remove_all_matching([](auto& value) { return value.is(Token::Type::Whitespace); });

    auto name = part.function().name;
    auto context_guard = push_temporary_value_parsing_context(FunctionContext { name });

    if (name.equals_ignoring_ascii_case("linear"sv)) {
        // linear() = linear( [ <number> && <percentage>{0,2} ]# )
        Vector<EasingStyleValue::Linear::Stop> stops;
        for (auto const& argument : comma_separated_arguments) {
            TokenStream argument_tokens { argument };

            RefPtr<StyleValue const> output;
            RefPtr<StyleValue const> first_input;
            RefPtr<StyleValue const> second_input;

            if (auto maybe_output = parse_number_value(argument_tokens, infinite_range))
                output = maybe_output;

            if (auto maybe_first_input = parse_percentage_value(argument_tokens, infinite_range)) {
                first_input = maybe_first_input;
                if (auto maybe_second_input = parse_percentage_value(argument_tokens, infinite_range)) {
                    second_input = maybe_second_input;
                }
            }

            if (auto maybe_output = parse_number_value(argument_tokens, infinite_range)) {
                if (output)
                    return nullptr;
                output = maybe_output;
            }

            if (argument_tokens.has_next_token() || !output)
                return nullptr;

            stops.append({ *output, first_input });
            if (second_input)
                stops.append({ *output, second_input });
        }

        if (stops.is_empty())
            return nullptr;

        transaction.commit();
        return EasingStyleValue::create(EasingStyleValue::Linear { move(stops) });
    }

    if (name.equals_ignoring_ascii_case("cubic-bezier"sv)) {
        if (comma_separated_arguments.size() != 4)
            return nullptr;

        for (auto const& argument : comma_separated_arguments) {
            if (argument.size() != 1)
                return nullptr;
        }

        auto parse_argument = [this, &comma_separated_arguments](auto index, NumericRange accepted_range) {
            TokenStream<ComponentValue> argument_tokens { comma_separated_arguments[index] };
            return parse_number_value(argument_tokens, accepted_range);
        };

        auto x1 = parse_argument(0, { .min = 0, .max = 1 });
        auto x2 = parse_argument(2, { .min = 0, .max = 1 });
        auto y1 = parse_argument(1, infinite_range);
        auto y2 = parse_argument(3, infinite_range);
        if (!x1 || !y1 || !x2 || !y2)
            return nullptr;

        EasingStyleValue::CubicBezier bezier {
            x1.release_nonnull(),
            y1.release_nonnull(),
            x2.release_nonnull(),
            y2.release_nonnull(),
        };

        transaction.commit();
        return EasingStyleValue::create(bezier);
    }

    if (name.equals_ignoring_ascii_case("steps"sv)) {
        if (comma_separated_arguments.is_empty() || comma_separated_arguments.size() > 2)
            return nullptr;

        for (auto const& argument : comma_separated_arguments) {
            if (argument.size() != 1)
                return nullptr;
        }

        StepPosition position = StepPosition::End;

        if (comma_separated_arguments.size() == 2) {
            if (comma_separated_arguments[1].size() != 1)
                return nullptr;

            auto token = comma_separated_arguments[1][0];

            if (!token.is(Token::Type::Ident))
                return nullptr;

            auto keyword = keyword_from_string(token.token().ident());

            if (!keyword.has_value())
                return nullptr;

            auto step_position = keyword_to_step_position(keyword.value());

            if (!step_position.has_value())
                return nullptr;

            position = step_position.value();
        }

        auto const& intervals_argument = comma_separated_arguments[0][0];
        auto intervals_token = TokenStream<ComponentValue>::of_single_token(intervals_argument);

        // https://drafts.csswg.org/css-easing/#step-easing-functions
        // If the <step-position> is jump-none, the <integer> must be at least 2, or the function is invalid.
        // Otherwise, the <integer> must be at least 1, or the function is invalid.

        double min_internals = position == StepPosition::JumpNone ? 2 : 1;
        auto intervals = parse_integer_value(intervals_token, NumericRange { .min = min_internals, .max = AK::NumericLimits<i32>::max() });

        if (!intervals)
            return nullptr;

        transaction.commit();
        return EasingStyleValue::create(EasingStyleValue::Steps { intervals.release_nonnull(), position });
    }

    return nullptr;
}

// https://drafts.csswg.org/css-values-4/#url-value
Optional<URL> Parser::parse_url_function(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto const& component_value = tokens.next_token();
    auto serialized_url = serialize_component_values_for_reparsing({ &component_value, 1 });
    auto maybe_url = RustComponentValueParser::parse_a_url_function(serialized_url.bytes_as_string_view(), "utf-8"sv);
    if (!maybe_url.has_value())
        return {};

    tokens.discard_a_token();
    transaction.commit();
    return maybe_url.release_value();
}

RefPtr<URLStyleValue const> Parser::parse_url_value(TokenStream<ComponentValue>& tokens)
{
    auto url = parse_url_function(tokens);
    if (!url.has_value())
        return nullptr;
    return URLStyleValue::create(url.release_value());
}

RefPtr<BorderRadiusRectStyleValue const> Parser::parse_border_radius_rect_value(TokenStream<ComponentValue>& tokens)
{
    auto top_left = [&](StyleValueVector& radii) { return radii[0]; };
    auto top_right = [&](StyleValueVector& radii) {
        switch (radii.size()) {
        case 4:
        case 3:
        case 2:
            return radii[1];
        case 1:
            return radii[0];
        default:
            VERIFY_NOT_REACHED();
        }
    };
    auto bottom_right = [&](StyleValueVector& radii) {
        switch (radii.size()) {
        case 4:
        case 3:
            return radii[2];
        case 2:
        case 1:
            return radii[0];
        default:
            VERIFY_NOT_REACHED();
        }
    };
    auto bottom_left = [&](StyleValueVector& radii) {
        switch (radii.size()) {
        case 4:
            return radii[3];
        case 3:
        case 2:
            return radii[1];
        case 1:
            return radii[0];
        default:
            VERIFY_NOT_REACHED();
        }
    };

    StyleValueVector horizontal_radii;
    StyleValueVector vertical_radii;
    bool reading_vertical = false;
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    while (tokens.has_next_token()) {
        if (tokens.next_token().is_delim('/')) {
            if (reading_vertical || horizontal_radii.is_empty())
                return nullptr;

            reading_vertical = true;
            tokens.discard_a_token(); // `/`
            tokens.discard_whitespace();
            continue;
        }

        auto maybe_dimension = parse_length_percentage_value(tokens, non_negative_range, non_negative_range);
        if (!maybe_dimension)
            return nullptr;
        if (reading_vertical) {
            vertical_radii.append(maybe_dimension.release_nonnull());
        } else {
            horizontal_radii.append(maybe_dimension.release_nonnull());
        }
        tokens.discard_whitespace();
    }

    if (horizontal_radii.size() > 4 || vertical_radii.size() > 4
        || horizontal_radii.is_empty()
        || (reading_vertical && vertical_radii.is_empty()))
        return nullptr;

    auto top_left_radius = BorderRadiusStyleValue::create(top_left(horizontal_radii),
        vertical_radii.is_empty() ? top_left(horizontal_radii) : top_left(vertical_radii));
    auto top_right_radius = BorderRadiusStyleValue::create(top_right(horizontal_radii),
        vertical_radii.is_empty() ? top_right(horizontal_radii) : top_right(vertical_radii));
    auto bottom_right_radius = BorderRadiusStyleValue::create(bottom_right(horizontal_radii),
        vertical_radii.is_empty() ? bottom_right(horizontal_radii) : bottom_right(vertical_radii));
    auto bottom_left_radius = BorderRadiusStyleValue::create(bottom_left(horizontal_radii),
        vertical_radii.is_empty() ? bottom_left(horizontal_radii) : bottom_left(vertical_radii));

    transaction.commit();
    return BorderRadiusRectStyleValue::create(top_left_radius, top_right_radius, bottom_right_radius, bottom_left_radius);
}

// https://drafts.csswg.org/css-images-4/#radial-size
RefPtr<RadialSizeStyleValue const> Parser::parse_radial_size(TokenStream<ComponentValue>& tokens)
{
    // <radial-size> = <radial-extent>{1,2} | <length-percentage [0,∞]>{1,2}
    // <radial-extent> = closest-corner | closest-side | farthest-corner | farthest-side
    // AD-HOC: The grammar by the spec above is incorrect as it disallows mixing of <length-percentage> and
    //         <radial-extent> which breaks backwards compatibility with `<shape-radius>` which it is intended to
    //         replace (see https://github.com/w3c/csswg-drafts/issues/9729). To avoid this issue we instead use the
    //         following grammar:
    //         `<radial-size> = [ <radial-extent> | <length-percentage [0,∞]> ]{1,2}`
    auto parse_radial_extent = [&](TokenStream<ComponentValue>& tokens) -> Optional<RadialExtent> {
        auto radial_extent_transaction = tokens.begin_transaction();

        auto keyword_value = parse_keyword_value(tokens);
        if (!keyword_value)
            return {};

        auto radial_extent = keyword_to_radial_extent(keyword_value->to_keyword());
        if (!radial_extent.has_value())
            return {};

        radial_extent_transaction.commit();
        return radial_extent;
    };

    auto parse_nonnegative_length_percentage_value = [&](TokenStream<ComponentValue>& tokens) -> RefPtr<StyleValue const> {
        auto length_percentage_transaction = tokens.begin_transaction();

        auto length_percentage_value = parse_length_percentage_value(tokens, non_negative_range, non_negative_range);
        if (!length_percentage_value)
            return nullptr;

        length_percentage_transaction.commit();
        return length_percentage_value;
    };

    auto transaction = tokens.begin_transaction();
    Vector<RadialSizeStyleValue::Component> values;

    while (tokens.has_next_token() && values.size() < 2) {
        tokens.discard_whitespace();

        if (auto radial_extent = parse_radial_extent(tokens); radial_extent.has_value()) {
            values.append(*radial_extent);
            continue;
        }

        if (auto length_percentage = parse_nonnegative_length_percentage_value(tokens); length_percentage) {
            values.append(length_percentage.release_nonnull());
            continue;
        }

        break;
    }

    if (values.is_empty())
        return nullptr;

    transaction.commit();
    return RadialSizeStyleValue::create(values);
}

RefPtr<StyleValue const> Parser::parse_fit_content_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto& component_value = tokens.consume_a_token();

    if (component_value.is_ident("fit-content"sv)) {
        transaction.commit();
        return KeywordStyleValue::create(Keyword::FitContent);
    }

    if (!component_value.is_function())
        return nullptr;

    auto const& function = component_value.function();
    if (function.name != "fit-content"sv)
        return nullptr;
    TokenStream argument_tokens { function.value };
    argument_tokens.discard_whitespace();
    auto length_percentage_value = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range);
    if (!length_percentage_value)
        return nullptr;
    argument_tokens.discard_whitespace();
    if (argument_tokens.has_next_token())
        return nullptr;

    transaction.commit();
    return FunctionStyleValue::create("fit-content"_fly_string, length_percentage_value.release_nonnull());
}

static FontStyleKeyword font_style_keyword_from_rust(FFI::CssFontStyleKind font_style)
{
    switch (font_style) {
    case FFI::CssFontStyleKind::Normal:
        return FontStyleKeyword::Normal;
    case FFI::CssFontStyleKind::Italic:
        return FontStyleKeyword::Italic;
    case FFI::CssFontStyleKind::Left:
        return FontStyleKeyword::Left;
    case FFI::CssFontStyleKind::Right:
        return FontStyleKeyword::Right;
    case FFI::CssFontStyleKind::Oblique:
        return FontStyleKeyword::Oblique;
    }
    VERIFY_NOT_REACHED();
}

RefPtr<StyleValue const> Parser::parse_font_style_value(TokenStream<ComponentValue>& tokens)
{
    // https://drafts.csswg.org/css-fonts/#font-style-prop
    // normal | italic | left | right | oblique <angle [-90deg,90deg]>?
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    auto start = tokens.current_index();
    if (!tokens.has_next_token())
        return nullptr;

    tokens.discard_a_token();
    auto serialized_font_style = serialize_component_values_for_reparsing(tokens.tokens_since(start));
    auto font_style = RustComponentValueParser::parse_a_font_style(serialized_font_style.bytes_as_string_view(), "utf-8"sv);

    if (!font_style.has_value())
        return nullptr;

    if (font_style->kind == FFI::CssFontStyleKind::Oblique) {
        auto angle_transaction = tokens.begin_transaction();
        tokens.discard_whitespace();
        if (tokens.has_next_token()) {
            auto angle_start = tokens.current_index();
            tokens.discard_a_token();
            serialized_font_style = serialize_component_values_for_reparsing(tokens.tokens_since(start));
            auto maybe_font_style_with_angle = RustComponentValueParser::parse_a_font_style(serialized_font_style.bytes_as_string_view(), "utf-8"sv);
            if (maybe_font_style_with_angle.has_value() && maybe_font_style_with_angle->has_angle) {
                Vector<ComponentValue> angle_component_values;
                for (auto const& component_value : tokens.tokens_since(angle_start))
                    angle_component_values.append(component_value);
                TokenStream<ComponentValue> angle_tokens { angle_component_values };
                auto angle_value = parse_angle_value(angle_tokens, { .min = -90, .max = 90 });
                angle_tokens.discard_whitespace();
                if (angle_value && !angle_tokens.has_next_token()) {
                    angle_transaction.commit();
                    transaction.commit();
                    return FontStyleStyleValue::create(font_style_keyword_from_rust(font_style->kind), angle_value.release_nonnull());
                }
            }
        }
    }

    transaction.commit();
    return FontStyleStyleValue::create(font_style_keyword_from_rust(font_style->kind));
}

RefPtr<StyleValue const> Parser::parse_font_variant_alternates_value(TokenStream<ComponentValue>& tokens)
{
    // 6.8 https://drafts.csswg.org/css-fonts/#font-variant-alternates-prop
    // [ stylistic(<feature-value-name>) || historical-forms || styleset(<feature-value-name>#) || character-variant(<feature-value-name>#) || swash(<feature-value-name>) || ornaments(<feature-value-name>) || annotation(<feature-value-name>) ]
    // <feature-value-name> = <ident>
    auto transaction = tokens.begin_transaction();
    auto start = tokens.current_index();
    Optional<Vector<RustComponentValueParser::FontVariantAlternatesValue>> parsed_values;

    while (tokens.has_next_token()) {
        auto component_transaction = tokens.begin_transaction();
        tokens.discard_whitespace();
        if (!tokens.has_next_token())
            break;
        tokens.discard_a_token();

        auto serialized_font_variant_alternates = serialize_component_values_for_reparsing(tokens.tokens_since(start));
        auto maybe_values = RustComponentValueParser::parse_a_font_variant_alternates(serialized_font_variant_alternates.bytes_as_string_view(), "utf-8"sv);
        if (!maybe_values.has_value())
            break;

        component_transaction.commit();
        parsed_values = maybe_values.release_value();
    }

    if (!parsed_values.has_value())
        return nullptr;

    StyleValueVector values;
    for (auto const& value : *parsed_values) {
        if (value.kind == FFI::CssFontVariantAlternatesValueKind::HistoricalForms) {
            values.append(KeywordStyleValue::create(Keyword::HistoricalForms));
            continue;
        }

        StyleValueVector feature_value_names;
        feature_value_names.ensure_capacity(value.feature_value_names.size());
        for (auto const& feature_value_name : value.feature_value_names)
            feature_value_names.append(CustomIdentStyleValue::create(feature_value_name));

        FlyString function_name;
        switch (value.kind) {
        case FFI::CssFontVariantAlternatesValueKind::Stylistic:
            function_name = "stylistic"_fly_string;
            break;
        case FFI::CssFontVariantAlternatesValueKind::Styleset:
            function_name = "styleset"_fly_string;
            break;
        case FFI::CssFontVariantAlternatesValueKind::CharacterVariant:
            function_name = "character-variant"_fly_string;
            break;
        case FFI::CssFontVariantAlternatesValueKind::Swash:
            function_name = "swash"_fly_string;
            break;
        case FFI::CssFontVariantAlternatesValueKind::Ornaments:
            function_name = "ornaments"_fly_string;
            break;
        case FFI::CssFontVariantAlternatesValueKind::Annotation:
            function_name = "annotation"_fly_string;
            break;
        case FFI::CssFontVariantAlternatesValueKind::HistoricalForms:
            VERIFY_NOT_REACHED();
        }

        values.append(FunctionStyleValue::create(move(function_name), StyleValueList::create(move(feature_value_names), StyleValueList::Separator::Comma)));
    }

    transaction.commit();
    return StyleValueList::create(move(values), StyleValueList::Separator::Space);
}

RefPtr<StyleValue const> Parser::parse_font_variant_east_asian_value(TokenStream<ComponentValue>& tokens)
{
    // 6.10 https://drafts.csswg.org/css-fonts/#propdef-font-variant-east-asian
    // [ <east-asian-variant-values> || <east-asian-width-values> || ruby ]
    // <east-asian-variant-values> = [ jis78 | jis83 | jis90 | jis04 | simplified | traditional ]
    // <east-asian-width-values>   = [ full-width | proportional-width ]
    auto transaction = tokens.begin_transaction();
    auto start = tokens.current_index();
    Optional<Vector<RustComponentValueParser::FontVariantEastAsianValue>> parsed_values;

    while (tokens.has_next_token()) {
        auto component_transaction = tokens.begin_transaction();
        tokens.discard_whitespace();
        if (!tokens.has_next_token())
            break;
        tokens.discard_a_token();

        auto serialized_font_variant_east_asian = serialize_component_values_for_reparsing(tokens.tokens_since(start));
        auto maybe_values = RustComponentValueParser::parse_a_font_variant_east_asian(serialized_font_variant_east_asian.bytes_as_string_view(), "utf-8"sv);
        if (!maybe_values.has_value())
            break;

        component_transaction.commit();
        parsed_values = maybe_values.release_value();
    }

    if (!parsed_values.has_value())
        return nullptr;

    StyleValueTuple tuple;
    tuple.resize_with_default_value(3, nullptr);

    for (auto const& value : *parsed_values) {
        auto maybe_keyword = keyword_from_string(value.value);
        if (!maybe_keyword.has_value())
            return nullptr;
        auto style_value = KeywordStyleValue::create(*maybe_keyword);
        switch (value.kind) {
        case FFI::CssFontVariantEastAsianValueKind::Variant:
            tuple[TupleStyleValue::Indices::FontVariantEastAsian::Variant] = style_value;
            break;
        case FFI::CssFontVariantEastAsianValueKind::Width:
            tuple[TupleStyleValue::Indices::FontVariantEastAsian::Width] = style_value;
            break;
        case FFI::CssFontVariantEastAsianValueKind::Ruby:
            tuple[TupleStyleValue::Indices::FontVariantEastAsian::Ruby] = style_value;
            break;
        }
    }

    transaction.commit();
    return TupleStyleValue::create(tuple);
}

RefPtr<StyleValue const> Parser::parse_font_variant_numeric_value(TokenStream<ComponentValue>& tokens)
{
    // 6.7 https://drafts.csswg.org/css-fonts/#propdef-font-variant-numeric
    // [ <numeric-figure-values> || <numeric-spacing-values> || <numeric-fraction-values> || ordinal || slashed-zero]
    // <numeric-figure-values>       = [ lining-nums | oldstyle-nums ]
    // <numeric-spacing-values>      = [ proportional-nums | tabular-nums ]
    // <numeric-fraction-values>     = [ diagonal-fractions | stacked-fractions ]
    auto transaction = tokens.begin_transaction();
    auto start = tokens.current_index();
    Optional<Vector<RustComponentValueParser::FontVariantNumericValue>> parsed_values;

    while (tokens.has_next_token()) {
        auto component_transaction = tokens.begin_transaction();
        tokens.discard_whitespace();
        if (!tokens.has_next_token())
            break;
        tokens.discard_a_token();

        auto serialized_font_variant_numeric = serialize_component_values_for_reparsing(tokens.tokens_since(start));
        auto maybe_values = RustComponentValueParser::parse_a_font_variant_numeric(serialized_font_variant_numeric.bytes_as_string_view(), "utf-8"sv);
        if (!maybe_values.has_value())
            break;

        component_transaction.commit();
        parsed_values = maybe_values.release_value();
    }

    if (!parsed_values.has_value())
        return nullptr;

    StyleValueTuple tuple;
    tuple.resize_with_default_value(5, nullptr);

    for (auto const& value : *parsed_values) {
        auto maybe_keyword = keyword_from_string(value.value);
        if (!maybe_keyword.has_value())
            return nullptr;
        auto style_value = KeywordStyleValue::create(*maybe_keyword);
        switch (value.kind) {
        case FFI::CssFontVariantNumericValueKind::Figure:
            tuple[TupleStyleValue::Indices::FontVariantNumeric::Figure] = style_value;
            break;
        case FFI::CssFontVariantNumericValueKind::Spacing:
            tuple[TupleStyleValue::Indices::FontVariantNumeric::Spacing] = style_value;
            break;
        case FFI::CssFontVariantNumericValueKind::Fraction:
            tuple[TupleStyleValue::Indices::FontVariantNumeric::Fraction] = style_value;
            break;
        case FFI::CssFontVariantNumericValueKind::Ordinal:
            tuple[TupleStyleValue::Indices::FontVariantNumeric::Ordinal] = style_value;
            break;
        case FFI::CssFontVariantNumericValueKind::SlashedZero:
            tuple[TupleStyleValue::Indices::FontVariantNumeric::SlashedZero] = style_value;
            break;
        }
    }

    transaction.commit();
    return TupleStyleValue::create(tuple);
}

RefPtr<StyleValue const> Parser::parse_font_variant_ligatures_value(TokenStream<ComponentValue>& tokens)
{
    // 6.4 https://drafts.csswg.org/css-fonts/#propdef-font-variant-ligatures
    // [ <common-lig-values> || <discretionary-lig-values> || <historical-lig-values> || <contextual-alt-values> ]
    // <common-lig-values>       = [ common-ligatures | no-common-ligatures ]
    // <discretionary-lig-values> = [ discretionary-ligatures | no-discretionary-ligatures ]
    // <historical-lig-values>   = [ historical-ligatures | no-historical-ligatures ]
    // <contextual-alt-values>   = [ contextual | no-contextual ]
    auto transaction = tokens.begin_transaction();
    auto start = tokens.current_index();
    Optional<Vector<RustComponentValueParser::FontVariantLigaturesValue>> parsed_values;

    while (tokens.has_next_token()) {
        auto component_transaction = tokens.begin_transaction();
        tokens.discard_whitespace();
        if (!tokens.has_next_token())
            break;
        tokens.discard_a_token();

        auto serialized_font_variant_ligatures = serialize_component_values_for_reparsing(tokens.tokens_since(start));
        auto maybe_values = RustComponentValueParser::parse_a_font_variant_ligatures(serialized_font_variant_ligatures.bytes_as_string_view(), "utf-8"sv);
        if (!maybe_values.has_value())
            break;

        component_transaction.commit();
        parsed_values = maybe_values.release_value();
    }

    if (!parsed_values.has_value())
        return nullptr;

    StyleValueTuple tuple;
    tuple.resize_with_default_value(4, nullptr);

    for (auto const& value : *parsed_values) {
        auto maybe_keyword = keyword_from_string(value.value);
        if (!maybe_keyword.has_value())
            return nullptr;
        auto style_value = KeywordStyleValue::create(*maybe_keyword);
        switch (value.kind) {
        case FFI::CssFontVariantLigaturesValueKind::Common:
            tuple[TupleStyleValue::Indices::FontVariantLigatures::Common] = style_value;
            break;
        case FFI::CssFontVariantLigaturesValueKind::Discretionary:
            tuple[TupleStyleValue::Indices::FontVariantLigatures::Discretionary] = style_value;
            break;
        case FFI::CssFontVariantLigaturesValueKind::Historical:
            tuple[TupleStyleValue::Indices::FontVariantLigatures::Historical] = style_value;
            break;
        case FFI::CssFontVariantLigaturesValueKind::Contextual:
            tuple[TupleStyleValue::Indices::FontVariantLigatures::Contextual] = style_value;
            break;
        }
    }

    transaction.commit();
    return TupleStyleValue::create(tuple);
}

RefPtr<StyleValue const> Parser::parse_basic_shape_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    auto& component_value = tokens.consume_a_token();
    if (!component_value.is_function())
        return nullptr;

    auto function_name = component_value.function().name.bytes_as_string_view();
    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_name });

    auto parse_fill_rule_argument = [](Vector<ComponentValue> const& component_values) -> Optional<Gfx::WindingRule> {
        TokenStream tokens { component_values };

        tokens.discard_whitespace();
        auto& maybe_ident = tokens.consume_a_token();
        tokens.discard_whitespace();

        if (tokens.has_next_token())
            return {};

        if (maybe_ident.is_ident("nonzero"sv))
            return Gfx::WindingRule::Nonzero;

        if (maybe_ident.is_ident("evenodd"sv))
            return Gfx::WindingRule::EvenOdd;

        return {};
    };

    if (function_name.equals_ignoring_ascii_case("inset"sv)) {
        // inset() = inset( <length-percentage>{1,4} [ round <'border-radius'> ]? )
        auto arguments_tokens = TokenStream { component_value.function().value };

        // If less than four <length-percentage> values are provided,
        // the omitted values default in the same way as the margin shorthand:
        // an omitted second or third value defaults to the first, and an omitted fourth value defaults to the second.

        // The four <length-percentage>s define the position of the top, right, bottom, and left edges of a rectangle.

        arguments_tokens.discard_whitespace();
        auto top = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
        if (!top)
            return nullptr;

        arguments_tokens.discard_whitespace();
        auto right = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
        if (!right)
            right = top;

        arguments_tokens.discard_whitespace();
        auto bottom = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
        if (!bottom)
            bottom = top;

        arguments_tokens.discard_whitespace();
        auto left = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
        if (!left)
            left = right;

        arguments_tokens.discard_whitespace();

        NonnullRefPtr<StyleValue const> border_radius = BorderRadiusRectStyleValue::create_zero();
        if (arguments_tokens.next_token().is_ident("round"sv)) {
            arguments_tokens.discard_a_token(); // 'round'
            auto parsed_border_radius = parse_border_radius_rect_value(arguments_tokens);

            if (!parsed_border_radius)
                return nullptr;

            border_radius = parsed_border_radius.release_nonnull();

            arguments_tokens.discard_whitespace();
        }

        if (arguments_tokens.has_next_token())
            return nullptr;

        transaction.commit();
        return BasicShapeStyleValue::create(Inset { top.release_nonnull(), right.release_nonnull(), bottom.release_nonnull(), left.release_nonnull(), border_radius });
    }

    if (function_name.equals_ignoring_ascii_case("xywh"sv)) {
        // xywh() = xywh( <length-percentage>{2} <length-percentage [0,∞]>{2} [ round <'border-radius'> ]? )
        auto arguments_tokens = TokenStream { component_value.function().value };

        arguments_tokens.discard_whitespace();
        auto x = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
        if (!x)
            return nullptr;

        arguments_tokens.discard_whitespace();
        auto y = parse_length_percentage_value(arguments_tokens, infinite_range, infinite_range);
        if (!y)
            return nullptr;

        arguments_tokens.discard_whitespace();
        auto width = parse_length_percentage_value(arguments_tokens, non_negative_range, non_negative_range);
        if (!width)
            return nullptr;

        arguments_tokens.discard_whitespace();
        auto height = parse_length_percentage_value(arguments_tokens, non_negative_range, non_negative_range);
        if (!height)
            return nullptr;

        arguments_tokens.discard_whitespace();

        NonnullRefPtr<StyleValue const> border_radius = BorderRadiusRectStyleValue::create_zero();
        if (arguments_tokens.next_token().is_ident("round"sv)) {
            arguments_tokens.discard_a_token(); // 'round'
            auto parsed_border_radius = parse_border_radius_rect_value(arguments_tokens);

            if (!parsed_border_radius)
                return nullptr;

            border_radius = parsed_border_radius.release_nonnull();

            arguments_tokens.discard_whitespace();
        }

        if (arguments_tokens.has_next_token())
            return nullptr;

        transaction.commit();
        return BasicShapeStyleValue::create(Xywh { x.release_nonnull(), y.release_nonnull(), width.release_nonnull(), height.release_nonnull(), border_radius });
    }

    if (function_name.equals_ignoring_ascii_case("rect"sv)) {
        // rect() = rect( [ <length-percentage> | auto ]{4} [ round <'border-radius'> ]? )
        auto arguments_tokens = TokenStream { component_value.function().value };

        auto parse_length_percentage_or_auto = [this](TokenStream<ComponentValue>& tokens) -> RefPtr<StyleValue const> {
            tokens.discard_whitespace();
            if (auto value = parse_length_percentage_value(tokens, infinite_range, infinite_range); value)
                return value;
            if (tokens.consume_a_token().is_ident("auto"sv))
                return KeywordStyleValue::create(Keyword::Auto);
            return {};
        };

        auto top = parse_length_percentage_or_auto(arguments_tokens);
        auto right = parse_length_percentage_or_auto(arguments_tokens);
        auto bottom = parse_length_percentage_or_auto(arguments_tokens);
        auto left = parse_length_percentage_or_auto(arguments_tokens);

        if (!top || !right || !bottom || !left)
            return nullptr;

        arguments_tokens.discard_whitespace();

        NonnullRefPtr<StyleValue const> border_radius = BorderRadiusRectStyleValue::create_zero();
        if (arguments_tokens.next_token().is_ident("round"sv)) {
            arguments_tokens.discard_a_token(); // 'round'

            auto parsed_border_radius = parse_border_radius_rect_value(arguments_tokens);

            if (!parsed_border_radius)
                return nullptr;

            border_radius = parsed_border_radius.release_nonnull();

            arguments_tokens.discard_whitespace();
        }

        if (arguments_tokens.has_next_token())
            return nullptr;

        transaction.commit();
        return BasicShapeStyleValue::create(Rect { top.release_nonnull(), right.release_nonnull(), bottom.release_nonnull(), left.release_nonnull(), border_radius });
    }

    if (function_name.equals_ignoring_ascii_case("circle"sv)) {
        // circle() = circle( <radial-size>? [ at <position> ]? )
        auto arguments_tokens = TokenStream { component_value.function().value };

        auto radius = parse_radial_size(arguments_tokens);

        if (radius && radius->components().size() != 1)
            return nullptr;

        if (!radius)
            radius = RadialSizeStyleValue::create({ RadialExtent::ClosestSide });

        RefPtr<PositionStyleValue const> position;
        arguments_tokens.discard_whitespace();
        if (arguments_tokens.next_token().is_ident("at"sv)) {
            arguments_tokens.discard_a_token();
            arguments_tokens.discard_whitespace();
            auto maybe_position = parse_position_value(arguments_tokens);
            if (maybe_position.is_null())
                return nullptr;

            position = maybe_position;
        }

        arguments_tokens.discard_whitespace();
        if (arguments_tokens.has_next_token())
            return nullptr;

        transaction.commit();
        return BasicShapeStyleValue::create(Circle { radius.release_nonnull(), position });
    }

    if (function_name.equals_ignoring_ascii_case("ellipse"sv)) {
        // ellipse() = ellipse( <radial-size>? [ at <position> ]? )
        auto arguments_tokens = TokenStream { component_value.function().value };

        auto radius = parse_radial_size(arguments_tokens);

        // NB: The spec doesn't specify whether a single value radius is valid here but WPT expects it to not be.
        if (radius && radius->components().size() != 2)
            return nullptr;

        if (!radius) {
            // AD-HOC: The spec calls for this to default to `closest-side` but as outlined above it's not clear whether
            //         the spec intends for single value radii to be valid.
            radius = RadialSizeStyleValue::create({ RadialExtent::ClosestSide, RadialExtent::ClosestSide });
        }

        RefPtr<PositionStyleValue const> position;
        arguments_tokens.discard_whitespace();
        if (arguments_tokens.next_token().is_ident("at"sv)) {
            arguments_tokens.discard_a_token();
            arguments_tokens.discard_whitespace();
            auto maybe_position = parse_position_value(arguments_tokens);
            if (maybe_position.is_null())
                return nullptr;

            position = maybe_position;
        }

        arguments_tokens.discard_whitespace();
        if (arguments_tokens.has_next_token())
            return nullptr;

        transaction.commit();
        return BasicShapeStyleValue::create(Ellipse { radius.release_nonnull(), position });
    }

    if (function_name.equals_ignoring_ascii_case("polygon"sv)) {
        // polygon() = polygon( <'fill-rule'>? , [<length-percentage> <length-percentage>]# )
        auto arguments_tokens = TokenStream { component_value.function().value };
        auto arguments = parse_a_comma_separated_list_of_component_values(arguments_tokens);

        if (arguments.size() < 1)
            return nullptr;

        Optional<Gfx::WindingRule> fill_rule;
        fill_rule = parse_fill_rule_argument(arguments[0]);

        if (fill_rule.has_value()) {
            arguments.remove(0);
        } else {
            fill_rule = Gfx::WindingRule::Nonzero;
        }

        if (arguments.size() < 1)
            return nullptr;

        Vector<Polygon::Point> points;
        for (auto& argument : arguments) {
            TokenStream argument_tokens { argument };

            argument_tokens.discard_whitespace();
            auto x_pos = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range);
            if (!x_pos)
                return nullptr;

            argument_tokens.discard_whitespace();
            auto y_pos = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range);
            if (!y_pos)
                return nullptr;

            argument_tokens.discard_whitespace();
            if (argument_tokens.has_next_token())
                return nullptr;

            points.append(Polygon::Point { x_pos.release_nonnull(), y_pos.release_nonnull() });
        }

        transaction.commit();
        return BasicShapeStyleValue::create(Polygon { fill_rule.value(), move(points) });
    }

    if (function_name.equals_ignoring_ascii_case("path"sv)) {
        // <path()> = path( <'fill-rule'>?, <string> )
        auto arguments_tokens = TokenStream { component_value.function().value };
        auto arguments = parse_a_comma_separated_list_of_component_values(arguments_tokens);

        if (arguments.size() < 1 || arguments.size() > 2)
            return nullptr;

        // <'fill-rule'>?
        Gfx::WindingRule fill_rule { Gfx::WindingRule::Nonzero };
        if (arguments.size() == 2) {
            auto maybe_fill_rule = parse_fill_rule_argument(arguments[0]);
            if (!maybe_fill_rule.has_value())
                return nullptr;
            fill_rule = maybe_fill_rule.release_value();
        }

        // <string>, which is a path string
        TokenStream path_argument_tokens { arguments.last() };
        path_argument_tokens.discard_whitespace();
        auto& maybe_string = path_argument_tokens.consume_a_token();
        path_argument_tokens.discard_whitespace();

        if (!maybe_string.is(Token::Type::String) || path_argument_tokens.has_next_token())
            return nullptr;
        auto path_data = SVG::AttributeParser::parse_path_data(maybe_string.token().string().to_string());
        if (path_data.instructions().is_empty())
            return nullptr;

        transaction.commit();
        return BasicShapeStyleValue::create(Path { fill_rule, move(path_data) });
    }

    return nullptr;
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
Optional<FlyString> Parser::parse_custom_ident(TokenStream<ComponentValue>& tokens, ReadonlySpan<StringView> blacklist)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    auto const& component_value = tokens.next_token();
    auto original_source_text = component_value.original_source_text();
    auto source = original_source_text.is_empty() ? component_value.to_string() : original_source_text;

    auto custom_ident = RustComponentValueParser::parse_a_custom_ident(source.bytes_as_string_view(), "utf-8"sv);
    if (!custom_ident.has_value())
        return {};

    for (auto& value : blacklist) {
        if (custom_ident->equals_ignoring_ascii_case(value))
            return {};
    }

    tokens.discard_a_token();

    transaction.commit();
    return custom_ident;
}

RefPtr<CustomIdentStyleValue const> Parser::parse_custom_ident_value(TokenStream<ComponentValue>& tokens, ReadonlySpan<StringView> blacklist)
{
    if (auto custom_ident = parse_custom_ident(tokens, blacklist); custom_ident.has_value())
        return CustomIdentStyleValue::create(custom_ident.release_value());
    return nullptr;
}

// https://drafts.csswg.org/css-values-5/#typedef-random-value-sharing
RefPtr<RandomValueSharingStyleValue const> Parser::parse_random_value_sharing(TokenStream<ComponentValue>& tokens)
{
    // <random-value-sharing> = [ [ auto | <dashed-ident> ] || element-shared ] | fixed <number [0,1]>
    auto transaction = tokens.begin_transaction();

    tokens.discard_whitespace();

    if (!tokens.has_next_token())
        return nullptr;

    // fixed <number [0,1]>
    if (tokens.next_token().is_ident("fixed"sv)) {
        tokens.discard_a_token();
        tokens.discard_whitespace();

        // NB: Fixed values have to be less than one and numbers serialize with six digits of precision
        if (auto fixed_value = parse_number_value(tokens, { .min = 0, .max = 0.999999 })) {
            tokens.discard_whitespace();

            if (tokens.has_next_token())
                return nullptr;

            transaction.commit();
            return RandomValueSharingStyleValue::create_fixed(fixed_value.release_nonnull());
        }

        return nullptr;
    }

    // [ [ auto | <dashed-ident> ] || element-shared ]
    bool has_explicit_auto = false;
    Optional<FlyString> dashed_ident;
    bool element_shared = false;

    while (tokens.has_next_token()) {
        if (auto maybe_dashed_ident_value = parse_dashed_ident_value(tokens)) {
            if (has_explicit_auto || dashed_ident.has_value())
                return nullptr;

            dashed_ident = maybe_dashed_ident_value->custom_ident();

            tokens.discard_whitespace();
            continue;
        }

        auto maybe_keyword_value = parse_keyword_value(tokens);

        if (maybe_keyword_value && maybe_keyword_value->to_keyword() == Keyword::Auto) {
            if (has_explicit_auto || dashed_ident.has_value())
                return nullptr;

            has_explicit_auto = true;

            tokens.discard_whitespace();
            continue;
        }

        if (maybe_keyword_value && maybe_keyword_value->to_keyword() == Keyword::ElementShared) {
            if (element_shared)
                return nullptr;

            element_shared = true;

            tokens.discard_whitespace();
            continue;
        }

        return nullptr;
    }

    if (!dashed_ident.has_value())
        return RandomValueSharingStyleValue::create_auto(random_value_sharing_auto_name(), element_shared);

    return RandomValueSharingStyleValue::create_dashed_ident(dashed_ident.value(), element_shared);
}

// https://drafts.csswg.org/css-values-4/#typedef-dashed-ident
Optional<FlyString> Parser::parse_dashed_ident(TokenStream<ComponentValue>& tokens)
{
    // The <dashed-ident> production is a <custom-ident>, with all the case-sensitivity that implies, with the
    // additional restriction that it must start with two dashes (U+002D HYPHEN-MINUS).
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    auto const& component_value = tokens.next_token();
    auto original_source_text = component_value.original_source_text();
    auto source = original_source_text.is_empty() ? component_value.to_string() : original_source_text;

    auto dashed_ident = RustComponentValueParser::parse_a_dashed_ident(source.bytes_as_string_view(), "utf-8"sv);
    if (!dashed_ident.has_value())
        return {};
    tokens.discard_a_token();

    transaction.commit();
    return dashed_ident;
}

RefPtr<CustomIdentStyleValue const> Parser::parse_dashed_ident_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    if (auto dashed_ident = parse_dashed_ident(tokens); dashed_ident.has_value()) {
        transaction.commit();
        return CustomIdentStyleValue::create(*dashed_ident);
    }
    return nullptr;
}

// https://www.w3.org/TR/css-grid-2/#typedef-track-breadth
Optional<GridSize> Parser::parse_grid_track_breadth(TokenStream<ComponentValue>& tokens)
{
    // <track-breadth> = <length-percentage [0,∞]> | <flex [0,∞]> | min-content | max-content | auto

    if (auto inflexible_breadth = parse_grid_inflexible_breadth(tokens); inflexible_breadth.has_value())
        return inflexible_breadth;

    if (auto flex_value = parse_flex_value(tokens, non_negative_range))
        return GridSize(flex_value.release_nonnull());

    return {};
}

// https://www.w3.org/TR/css-grid-2/#typedef-inflexible-breadth
Optional<GridSize> Parser::parse_grid_inflexible_breadth(TokenStream<ComponentValue>& tokens)
{
    // <inflexible-breadth>  = <length-percentage [0,∞]> | min-content | max-content | auto

    if (auto fixed_breadth = parse_grid_fixed_breadth(tokens))
        return GridSize { fixed_breadth.release_nonnull() };

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return {};

    auto const& token = tokens.consume_a_token();
    if (token.is_ident("max-content"sv)) {
        transaction.commit();
        return GridSize(KeywordStyleValue::create(Keyword::MaxContent));
    }
    if (token.is_ident("min-content"sv)) {
        transaction.commit();
        return GridSize(KeywordStyleValue::create(Keyword::MinContent));
    }
    if (token.is_ident("auto"sv)) {
        transaction.commit();
        return GridSize::make_auto();
    }

    return {};
}

// https://www.w3.org/TR/css-grid-2/#typedef-fixed-breadth
RefPtr<StyleValue const> Parser::parse_grid_fixed_breadth(TokenStream<ComponentValue>& tokens)
{
    // <fixed-breadth> = <length-percentage [0,∞]>

    auto transaction = tokens.begin_transaction();
    auto length_percentage = parse_length_percentage_value(tokens, non_negative_range, non_negative_range);
    if (!length_percentage)
        return {};
    transaction.commit();
    return length_percentage;
}

// https://www.w3.org/TR/css-grid-2/#typedef-line-names
Optional<GridLineNames> Parser::parse_grid_line_names(TokenStream<ComponentValue>& tokens)
{
    // <line-names> = '[' <custom-ident>* ']'

    auto transactions = tokens.begin_transaction();
    GridLineNames line_names;
    tokens.discard_whitespace();
    auto const& token = tokens.consume_a_token();
    if (!token.is_block() || !token.block().is_square())
        return line_names;

    TokenStream block_tokens { token.block().value };
    block_tokens.discard_whitespace();
    while (block_tokens.has_next_token()) {
        auto maybe_ident = parse_custom_ident(block_tokens, { { "span"sv, "auto"sv } });
        if (!maybe_ident.has_value())
            return OptionalNone {};
        line_names.append(maybe_ident.release_value());
        block_tokens.discard_whitespace();
    }

    transactions.commit();
    return line_names;
}

size_t Parser::parse_track_list_impl(TokenStream<ComponentValue>& tokens, GridTrackSizeList& output, GridTrackParser const& track_parsing_callback, AllowTrailingLineNamesForEachTrack allow_trailing_line_names_for_each_track)
{
    size_t parsed_tracks_count = 0;
    tokens.discard_whitespace();
    while (tokens.has_next_token()) {
        auto transaction = tokens.begin_transaction();
        auto line_names = parse_grid_line_names(tokens);

        tokens.discard_whitespace();
        auto explicit_grid_track = track_parsing_callback(tokens);
        tokens.discard_whitespace();

        if (!explicit_grid_track.has_value())
            break;

        if (line_names.has_value() && !line_names->is_empty())
            output.append(line_names.release_value());

        output.append(explicit_grid_track.release_value());
        if (allow_trailing_line_names_for_each_track == AllowTrailingLineNamesForEachTrack::Yes) {
            auto trailing_line_names = parse_grid_line_names(tokens);
            if (trailing_line_names.has_value() && !trailing_line_names->is_empty()) {
                output.append(trailing_line_names.release_value());
            }
        }
        transaction.commit();
        parsed_tracks_count++;
        tokens.discard_whitespace();
    }

    if (allow_trailing_line_names_for_each_track == AllowTrailingLineNamesForEachTrack::No) {
        if (auto trailing_line_names = parse_grid_line_names(tokens); trailing_line_names.has_value() && !trailing_line_names->is_empty()) {
            output.append(trailing_line_names.release_value());
        }
    }

    return parsed_tracks_count;
}

Optional<GridRepeat> Parser::parse_grid_track_repeat_impl(TokenStream<ComponentValue>& tokens, GridRepeatTypeParser const& repeat_type_parser, GridTrackParser const& repeat_track_parser)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    if (!tokens.has_next_token())
        return {};

    auto const& token = tokens.consume_a_token();
    if (!token.is_function())
        return {};

    auto const& function_token = token.function();
    if (!function_token.name.equals_ignoring_ascii_case("repeat"sv))
        return {};
    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.name });

    auto function_tokens = TokenStream(function_token.value);
    auto comma_separated_list = parse_a_comma_separated_list_of_component_values(function_tokens);
    if (comma_separated_list.size() != 2)
        return {};

    TokenStream first_arg_tokens { comma_separated_list[0] };
    first_arg_tokens.discard_whitespace();
    if (!first_arg_tokens.has_next_token())
        return {};

    auto repeat_params = repeat_type_parser(first_arg_tokens);
    if (!repeat_params.has_value())
        return {};
    first_arg_tokens.discard_whitespace();
    if (first_arg_tokens.has_next_token())
        return {};

    TokenStream second_arg_tokens { comma_separated_list[1] };
    second_arg_tokens.discard_whitespace();
    GridTrackSizeList track_list;
    if (auto parsed_track_count = parse_track_list_impl(second_arg_tokens, track_list, repeat_track_parser); parsed_track_count == 0)
        return {};
    if (second_arg_tokens.has_next_token())
        return {};
    transaction.commit();
    return GridRepeat(GridTrackSizeList(move(track_list)), repeat_params.release_value());
}

Optional<ExplicitGridTrack> Parser::parse_grid_minmax(TokenStream<ComponentValue>& tokens, GridMinMaxParamParser const& min_parser, GridMinMaxParamParser const& max_parser)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    if (!tokens.has_next_token())
        return {};

    auto const& token = tokens.consume_a_token();
    if (!token.is_function())
        return {};

    auto const& function_token = token.function();
    if (!function_token.name.equals_ignoring_ascii_case("minmax"sv))
        return {};

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.name });
    auto function_tokens = TokenStream(function_token.value);
    auto comma_separated_list = parse_a_comma_separated_list_of_component_values(function_tokens);
    if (comma_separated_list.size() != 2)
        return {};

    TokenStream min_tokens { comma_separated_list[0] };
    min_tokens.discard_whitespace();
    auto min_value = min_parser(min_tokens);
    if (!min_value.has_value())
        return {};
    min_tokens.discard_whitespace();
    if (min_tokens.has_next_token())
        return {};

    TokenStream max_tokens { comma_separated_list[1] };
    max_tokens.discard_whitespace();
    auto max_value = max_parser(max_tokens);
    if (!max_value.has_value())
        return {};
    max_tokens.discard_whitespace();
    if (max_tokens.has_next_token())
        return {};

    transaction.commit();
    return ExplicitGridTrack(GridMinMax(min_value.release_value(), max_value.release_value()));
}

// https://www.w3.org/TR/css-grid-2/#typedef-track-repeat
Optional<GridRepeat> Parser::parse_grid_track_repeat(TokenStream<ComponentValue>& tokens)
{
    // <track-repeat> = repeat( [ <integer [1,∞]> ] , [ <line-names>? <track-size> ]+ <line-names>? )

    GridRepeatTypeParser parse_repeat_type = [this](TokenStream<ComponentValue>& tokens) -> Optional<GridRepeatParams> {
        auto maybe_integer = parse_integer_value(tokens, { .min = 1, .max = NumericLimits<i32>::max() });
        if (!maybe_integer)
            return {};

        return GridRepeatParams { GridRepeatType::Fixed, maybe_integer };
    };
    GridTrackParser parse_track = [this](TokenStream<ComponentValue>& tokens) {
        return parse_grid_track_size(tokens);
    };
    return parse_grid_track_repeat_impl(tokens, parse_repeat_type, parse_track);
}

// https://www.w3.org/TR/css-grid-2/#typedef-auto-repeat
Optional<GridRepeat> Parser::parse_grid_auto_repeat(TokenStream<ComponentValue>& tokens)
{
    // <auto-repeat> = repeat( [ auto-fill | auto-fit ] , [ <line-names>? <fixed-size> ]+ <line-names>? )

    GridRepeatTypeParser parse_repeat_type = [](TokenStream<ComponentValue>& tokens) -> Optional<GridRepeatParams> {
        tokens.discard_whitespace();
        auto const& first_token = tokens.consume_a_token();
        if (!first_token.is_token() || !first_token.token().is(Token::Type::Ident))
            return {};

        auto ident_value = first_token.token().ident();
        if (ident_value.equals_ignoring_ascii_case("auto-fill"sv))
            return GridRepeatParams { GridRepeatType::AutoFill };
        if (ident_value.equals_ignoring_ascii_case("auto-fit"sv))
            return GridRepeatParams { GridRepeatType::AutoFit };
        return {};
    };
    GridTrackParser parse_track = [this](TokenStream<ComponentValue>& tokens) {
        return parse_grid_fixed_size(tokens);
    };
    return parse_grid_track_repeat_impl(tokens, parse_repeat_type, parse_track);
}

// https://www.w3.org/TR/css-grid-2/#typedef-fixed-repeat
Optional<GridRepeat> Parser::parse_grid_fixed_repeat(TokenStream<ComponentValue>& tokens)
{
    // <fixed-repeat> = repeat( [ <integer [1,∞]> ] , [ <line-names>? <fixed-size> ]+ <line-names>? )

    GridRepeatTypeParser parse_repeat_type = [this](TokenStream<ComponentValue>& tokens) -> Optional<GridRepeatParams> {
        auto maybe_integer = parse_integer_value(tokens, { .min = 1, .max = NumericLimits<i32>::max() });
        if (!maybe_integer)
            return {};

        return GridRepeatParams { GridRepeatType::Fixed, maybe_integer };
    };
    GridTrackParser parse_track = [this](TokenStream<ComponentValue>& tokens) {
        return parse_grid_fixed_size(tokens);
    };
    return parse_grid_track_repeat_impl(tokens, parse_repeat_type, parse_track);
}

// https://www.w3.org/TR/css-grid-2/#typedef-track-size
Optional<ExplicitGridTrack> Parser::parse_grid_track_size(TokenStream<ComponentValue>& tokens)
{
    // <track-size> = <track-breadth> | minmax( <inflexible-breadth> , <track-breadth> ) | fit-content( <length-percentage [0,∞]> )
    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return {};

    if (tokens.next_token().is_function()) {
        auto const& token = tokens.next_token();
        auto const& function_token = token.function();

        if (function_token.name.equals_ignoring_ascii_case("minmax"sv)) {
            GridMinMaxParamParser parse_min = [this](auto& tokens) { return parse_grid_inflexible_breadth(tokens); };
            GridMinMaxParamParser parse_max = [this](auto& tokens) { return parse_grid_track_breadth(tokens); };
            return parse_grid_minmax(tokens, parse_min, parse_max);
        }

        auto transaction = tokens.begin_transaction();
        tokens.discard_a_token();
        auto context_guard = push_temporary_value_parsing_context(FunctionContext { function_token.name });

        if (function_token.name.equals_ignoring_ascii_case("fit-content"sv)) {
            auto function_tokens = TokenStream(function_token.value);
            function_tokens.discard_whitespace();
            auto maybe_length_percentage = parse_grid_fixed_breadth(function_tokens);
            if (!maybe_length_percentage)
                return {};
            if (function_tokens.has_next_token())
                return {};
            transaction.commit();
            return ExplicitGridTrack(GridSize(FunctionStyleValue::create("fit-content"_fly_string, maybe_length_percentage.release_nonnull())));
        }
    }

    if (auto track_breadth = parse_grid_track_breadth(tokens); track_breadth.has_value()) {
        return ExplicitGridTrack(track_breadth.value());
    }

    return {};
}

// https://www.w3.org/TR/css-grid-2/#typedef-fixed-size
Optional<ExplicitGridTrack> Parser::parse_grid_fixed_size(TokenStream<ComponentValue>& tokens)
{
    // <fixed-size> = <fixed-breadth> | minmax( <fixed-breadth> , <track-breadth> ) | minmax( <inflexible-breadth> , <fixed-breadth> )
    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return {};

    if (tokens.next_token().is_function()) {
        auto const& token = tokens.next_token();
        auto const& function_token = token.function();
        if (function_token.name.equals_ignoring_ascii_case("minmax"sv)) {
            {
                GridMinMaxParamParser parse_min = [this](auto& tokens) -> Optional<GridSize> {
                    if (auto result = parse_grid_fixed_breadth(tokens))
                        return GridSize(result.release_nonnull());
                    return {};
                };

                GridMinMaxParamParser parse_max = [this](auto& tokens) { return parse_grid_track_breadth(tokens); };

                if (auto result = parse_grid_minmax(tokens, parse_min, parse_max); result.has_value())
                    return result;
            }
            {
                GridMinMaxParamParser parse_min = [this](auto& tokens) { return parse_grid_inflexible_breadth(tokens); };

                GridMinMaxParamParser parse_max = [this](auto& tokens) -> Optional<GridSize> {
                    if (auto result = parse_grid_fixed_breadth(tokens))
                        return GridSize(result.release_nonnull());
                    return {};
                };

                if (auto result = parse_grid_minmax(tokens, parse_min, parse_max); result.has_value())
                    return result;
            }

            return {};
        }
    }

    if (auto fixed_breadth = parse_grid_fixed_breadth(tokens)) {
        return ExplicitGridTrack(GridSize { fixed_breadth.release_nonnull() });
    }

    return {};
}

// https://www.w3.org/TR/css-grid-2/#typedef-track-list
GridTrackSizeList Parser::parse_grid_track_list(TokenStream<ComponentValue>& tokens)
{
    // <track-list> = [ <line-names>? [ <track-size> | <track-repeat> ] ]+ <line-names>?

    auto transaction = tokens.begin_transaction();
    GridTrackSizeList track_list;
    auto parsed_track_count = parse_track_list_impl(tokens, track_list, [&](auto& tokens) -> Optional<ExplicitGridTrack> {
        if (auto track_repeat = parse_grid_track_repeat(tokens); track_repeat.has_value())
            return ExplicitGridTrack(track_repeat.value());
        if (auto track_size = parse_grid_track_size(tokens); track_size.has_value())
            return ExplicitGridTrack(track_size.value());
        return Optional<ExplicitGridTrack> {};
    });
    if (parsed_track_count == 0)
        return {};
    transaction.commit();
    return track_list;
}

// https://www.w3.org/TR/css-grid-2/#typedef-auto-track-list
GridTrackSizeList Parser::parse_grid_auto_track_list(TokenStream<ComponentValue>& tokens)
{
    // <auto-track-list> = [ <line-names>? [ <fixed-size> | <fixed-repeat> ] ]* <line-names>? <auto-repeat>
    //                     [ <line-names>? [ <fixed-size> | <fixed-repeat> ] ]* <line-names>?

    auto transaction = tokens.begin_transaction();
    GridTrackSizeList track_list;
    size_t parsed_track_count = 0;
    auto parse_zero_or_more_fixed_tracks = [&] {
        parsed_track_count += parse_track_list_impl(tokens, track_list, [&](auto& tokens) -> Optional<ExplicitGridTrack> {
            if (auto fixed_repeat = parse_grid_fixed_repeat(tokens); fixed_repeat.has_value())
                return ExplicitGridTrack(fixed_repeat.value());
            if (auto fixed_size = parse_grid_fixed_size(tokens); fixed_size.has_value())
                return ExplicitGridTrack(fixed_size.value());
            return Optional<ExplicitGridTrack> {};
        });
    };

    parse_zero_or_more_fixed_tracks();
    tokens.discard_whitespace();
    if (!tokens.has_next_token()) {
        if (parsed_track_count == 0)
            return {};
        transaction.commit();
        return track_list;
    }

    if (auto auto_repeat = parse_grid_auto_repeat(tokens); auto_repeat.has_value()) {
        track_list.append(ExplicitGridTrack(auto_repeat.release_value()));
    } else {
        return {};
    }

    parse_zero_or_more_fixed_tracks();
    transaction.commit();
    return track_list;
}

// https://www.w3.org/TR/css-grid-2/#typedef-explicit-track-list
GridTrackSizeList Parser::parse_explicit_track_list(TokenStream<ComponentValue>& tokens)
{
    // <explicit-track-list> = [ <line-names>? <track-size> ]+ <line-names>?

    auto transaction = tokens.begin_transaction();
    GridTrackSizeList track_list;
    auto parsed_track_count = parse_track_list_impl(tokens, track_list, [&](auto& tokens) -> Optional<ExplicitGridTrack> {
        return parse_grid_track_size(tokens);
    });
    if (parsed_track_count == 0)
        return {};
    transaction.commit();
    return track_list;
}

RefPtr<GridTrackPlacementStyleValue const> Parser::parse_grid_track_placement(TokenStream<ComponentValue>& tokens)
{
    // https://www.w3.org/TR/css-grid-2/#line-placement
    // Line-based Placement: the grid-row-start, grid-column-start, grid-row-end, and grid-column-end properties
    // <grid-line> =
    //     auto |
    //     <custom-ident> |
    //     [ [ <integer [-∞,-1]> | <integer [1,∞]> ] && <custom-ident>? ] |
    //     [ span && [ <integer [1,∞]> || <custom-ident> ] ]
    bool is_span = false;
    Optional<String> parsed_custom_ident;
    RefPtr<StyleValue const> parsed_integer;

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    if (auto auto_keyword = parse_all_as_single_keyword_value(tokens, Keyword::Auto)) {
        transaction.commit();
        return GridTrackPlacementStyleValue::create(GridTrackPlacement::make_auto());
    }

    while (tokens.has_next_token()) {
        if (tokens.next_token().is_ident("span"sv)) {
            if (is_span)
                return nullptr;

            tokens.discard_a_token(); // span

            // NOTE: "span" must not appear in between <custom-ident> and <integer>
            if (tokens.has_next_token() && (parsed_custom_ident.has_value() || parsed_integer))
                return nullptr;

            is_span = true;
            tokens.discard_whitespace();
            continue;
        }

        if (auto maybe_parsed_custom_ident = parse_custom_ident(tokens, { { "auto"sv } }); maybe_parsed_custom_ident.has_value()) {
            if (parsed_custom_ident.has_value())
                return nullptr;

            parsed_custom_ident = maybe_parsed_custom_ident->to_string();
            tokens.discard_whitespace();
            continue;
        }

        // FIXME: Use the correct value parsing context here to clamp calculated values (note the non-contiguous valid
        //        range for integers for non-span)
        if (auto maybe_parsed_integer = parse_integer_value(tokens, infinite_integer_range)) {
            if (parsed_integer)
                return nullptr;

            parsed_integer = maybe_parsed_integer;
            tokens.discard_whitespace();
            continue;
        }

        return nullptr;
    }

    transaction.commit();

    // <custom-ident>
    // [ [ <integer [-∞,-1]> | <integer [1,∞]> ] && <custom-ident>? ]
    if (!is_span && (parsed_integer || parsed_custom_ident.has_value()) && (!parsed_integer || !parsed_integer->is_integer() || parsed_integer->as_integer().integer() != 0))
        return GridTrackPlacementStyleValue::create(GridTrackPlacement::make_line(parsed_integer, parsed_custom_ident));

    // [ span && [ <integer [1,∞]> || <custom-ident> ] ]
    if (is_span && (parsed_integer || parsed_custom_ident.has_value()) && (!parsed_integer || !parsed_integer->is_integer() || parsed_integer->as_integer().integer() > 0))
        // If the <integer> is omitted, it defaults to 1.
        return GridTrackPlacementStyleValue::create(GridTrackPlacement::make_span(parsed_integer ? parsed_integer.release_nonnull() : IntegerStyleValue::create(1), parsed_custom_ident));

    return nullptr;
}

RefPtr<CalculatedStyleValue const> Parser::parse_calculated_value(ComponentValue const& component_value, CalculationContext&& context)
{
    if (!component_value.is_function())
        return nullptr;

    auto function_node = parse_a_calc_function_node(component_value.function(), context);
    if (!function_node)
        return nullptr;

    auto function_type = function_node->numeric_type();
    if (!function_type.has_value())
        return nullptr;

    return CalculatedStyleValue::create(function_node.release_nonnull(), function_type.release_value(), context);
}

RefPtr<CalculationNode const> Parser::parse_a_calc_function_node(Function const& function, CalculationContext const& context)
{
    auto context_guard = push_temporary_value_parsing_context(FunctionContext { function.name });

    if (function.name.equals_ignoring_ascii_case("calc"sv)) {
        TokenStream tokens { function.value };
        return parse_a_calculation(tokens, context);
    }

    if (auto maybe_function = parse_math_function(function, context)) {
        // NOTE: We have to simplify manually here, since parse_math_function() is a helper for calc() parsing
        //       that doesn't do it directly by itself.
        return simplify_a_calculation_tree(*maybe_function, context, CalculationResolutionContext {});
    }

    return nullptr;
}

RefPtr<CalculationNode const> Parser::convert_to_calculation_node(CalcParsing::Node const& node, CalculationContext const& context)
{
    return node.visit(
        [this, &context](NonnullOwnPtr<CalcParsing::ProductNode> const& product_node) -> RefPtr<CalculationNode const> {
            Vector<NonnullRefPtr<CalculationNode const>> children;
            children.ensure_capacity(product_node->children.size());

            for (auto const& child : product_node->children) {
                if (auto child_as_node = convert_to_calculation_node(child, context)) {
                    children.append(child_as_node.release_nonnull());
                } else {
                    return nullptr;
                }
            }

            return ProductCalculationNode::create(move(children));
        },
        [this, &context](NonnullOwnPtr<CalcParsing::SumNode> const& sum_node) -> RefPtr<CalculationNode const> {
            Vector<NonnullRefPtr<CalculationNode const>> children;
            children.ensure_capacity(sum_node->children.size());

            for (auto const& child : sum_node->children) {
                if (auto child_as_node = convert_to_calculation_node(child, context)) {
                    children.append(child_as_node.release_nonnull());
                } else {
                    return nullptr;
                }
            }

            return SumCalculationNode::create(move(children));
        },
        [this, &context](NonnullOwnPtr<CalcParsing::InvertNode> const& invert_node) -> RefPtr<CalculationNode const> {
            if (auto child_as_node = convert_to_calculation_node(invert_node->child, context))
                return InvertCalculationNode::create(child_as_node.release_nonnull());
            return nullptr;
        },
        [this, &context](NonnullOwnPtr<CalcParsing::NegateNode> const& negate_node) -> RefPtr<CalculationNode const> {
            if (auto child_as_node = convert_to_calculation_node(negate_node->child, context))
                return NegateCalculationNode::create(child_as_node.release_nonnull());
            return nullptr;
        },
        [this, &context](NonnullRawPtr<ComponentValue const> const& component_value) -> RefPtr<CalculationNode const> {
            // NOTE: This is the "process the leaf nodes" part of step 5 of https://drafts.csswg.org/css-values-4/#parse-a-calculation
            //       We divert a little from the spec: Rather than modify an existing tree of values, we construct a new one from that source tree.
            //       This lets us make CalculationNodes immutable.

            // 1. If leaf is a parenthesized simple block, replace leaf with the result of parsing a calculation from leaf’s contents.
            if (component_value->is_block() && component_value->block().is_paren()) {
                TokenStream tokens { component_value->block().value };
                auto leaf_calculation = parse_a_calculation(tokens, context);
                if (!leaf_calculation)
                    return nullptr;

                return leaf_calculation.release_nonnull();
            }

            // 2. If leaf is a math function, replace leaf with the internal representation of that math function.
            if (component_value->is_function() && math_function_from_string(component_value->function().name).has_value()) {
                auto const& function = component_value->function();
                auto leaf_calculation = parse_a_calc_function_node(function, context);
                if (!leaf_calculation)
                    return nullptr;

                return leaf_calculation.release_nonnull();
            }

            // AD-HOC: We also need to convert tokens into their numeric types.

            if (component_value->is(Token::Type::Ident)) {
                auto maybe_keyword = keyword_from_string(component_value->token().ident());
                if (!maybe_keyword.has_value())
                    return nullptr;
                return NumericCalculationNode::from_keyword(*maybe_keyword, context);
            }

            if (component_value->is(Token::Type::Number))
                return NumericCalculationNode::create(Number { Number::Type::Number, component_value->token().number_value() }, context);

            if (component_value->is(Token::Type::Dimension)) {
                auto numeric_value = component_value->token().dimension_value();
                auto unit_string = component_value->token().dimension_unit();

                if (auto length_type = string_to_length_unit(unit_string); length_type.has_value())
                    return NumericCalculationNode::create(Length { numeric_value, length_type.release_value() }, context);

                if (auto angle_type = string_to_angle_unit(unit_string); angle_type.has_value())
                    return NumericCalculationNode::create(Angle { numeric_value, angle_type.release_value() }, context);

                if (auto flex_type = string_to_flex_unit(unit_string); flex_type.has_value())
                    return NumericCalculationNode::create(Flex { numeric_value, flex_type.release_value() }, context);

                if (auto frequency_type = string_to_frequency_unit(unit_string); frequency_type.has_value())
                    return NumericCalculationNode::create(Frequency { numeric_value, frequency_type.release_value() }, context);

                if (auto resolution_type = string_to_resolution_unit(unit_string); resolution_type.has_value())
                    return NumericCalculationNode::create(Resolution { numeric_value, resolution_type.release_value() }, context);

                if (auto time_type = string_to_time_unit(unit_string); time_type.has_value())
                    return NumericCalculationNode::create(Time { numeric_value, time_type.release_value() }, context);

                ErrorReporter::the().report(InvalidValueError {
                    .value_type = "math-function"_fly_string,
                    .value_string = component_value->to_string(),
                    .description = "Unrecognized dimension type."_string,
                });
                return nullptr;
            }

            if (component_value->is(Token::Type::Percentage))
                return NumericCalculationNode::create(Percentage { component_value->token().percentage() }, context);

            auto tree_counting_function_tokens = TokenStream<ComponentValue>::of_single_token(component_value);
            if (auto tree_counting_function = parse_tree_counting_function(tree_counting_function_tokens, TreeCountingFunctionStyleValue::ComputedType::Number))
                return NonMathFunctionCalculationNode::create(tree_counting_function.release_nonnull(), NumericType {});

            // NOTE: If we get here, then we have a ComponentValue that didn't get replaced with something else,
            //       so the calc() is invalid.
            ErrorReporter::the().report(InvalidValueError {
                .value_type = "math-function"_fly_string,
                .value_string = component_value->to_string(),
                .description = "Left-over ComponentValue in calculation tree."_string,
            });
            return nullptr;
        },
        [](CalcParsing::Operator const& op) -> RefPtr<CalculationNode const> {
            ErrorReporter::the().report(InvalidValueError {
                .value_type = "math-function"_fly_string,
                .value_string = String::from_code_point(op.delim),
                .description = "Left-over Operator in calculation tree."_string,
            });
            return nullptr;
        });
}

// https://drafts.csswg.org/css-values-4/#parse-a-calculation
RefPtr<CalculationNode const> Parser::parse_a_calculation(TokenStream<ComponentValue>& tokens, CalculationContext const& context)
{
    auto transaction = tokens.begin_transaction();

    // 1. Discard any <whitespace-token>s from values.
    // 2. An item in values is an “operator” if it’s a <delim-token> with the value "+", "-", "*", or "/". Otherwise, it’s a “value”.

    Vector<CalcParsing::Node> values;
    while (tokens.has_next_token()) {
        auto const& value = tokens.consume_a_token();
        if (value.is(Token::Type::Whitespace))
            continue;
        if (value.is(Token::Type::Delim)) {
            if (first_is_one_of(value.token().delim(), static_cast<u32>('+'), static_cast<u32>('-'), static_cast<u32>('*'), static_cast<u32>('/'))) {
                // NOTE: Sequential operators are invalid syntax.
                if (!values.is_empty() && values.last().has<CalcParsing::Operator>())
                    return nullptr;

                values.append(CalcParsing::Operator { static_cast<char>(value.token().delim()) });
                continue;
            }
        }

        values.append(NonnullRawPtr { value });
    }

    // If we have no values, the syntax is invalid.
    if (values.is_empty())
        return nullptr;

    // NOTE: If the first or last value is an operator, the syntax is invalid.
    if (values.first().has<CalcParsing::Operator>() || values.last().has<CalcParsing::Operator>())
        return nullptr;

    // 3. Collect children into Product and Invert nodes.
    //    For every consecutive run of value items in values separated by "*" or "/" operators:
    while (true) {
        Optional<size_t> first_product_operator = values.find_first_index_if([](auto const& item) {
            return item.template has<CalcParsing::Operator>()
                && first_is_one_of(item.template get<CalcParsing::Operator>().delim, '*', '/');
        });

        if (!first_product_operator.has_value())
            break;

        auto start_of_run = first_product_operator.value() - 1;
        auto end_of_run = first_product_operator.value() + 1;
        for (auto i = start_of_run + 1; i < values.size(); i += 2) {
            auto& item = values[i];
            if (!item.has<CalcParsing::Operator>()) {
                end_of_run = i - 1;
                break;
            }

            auto delim = item.get<CalcParsing::Operator>().delim;
            if (!first_is_one_of(delim, '*', '/')) {
                end_of_run = i - 1;
                break;
            }
        }

        // 1. For each "/" operator in the run, replace its right-hand value item rhs with an Invert node containing rhs as its child.
        Vector<CalcParsing::Node> run_values;
        run_values.append(move(values[start_of_run]));
        for (auto i = start_of_run + 1; i <= end_of_run; i += 2) {
            auto& operator_ = values[i].get<CalcParsing::Operator>().delim;
            auto& rhs = values[i + 1];
            if (operator_ == '/') {
                run_values.append(make<CalcParsing::InvertNode>(move(rhs)));
                continue;
            }
            VERIFY(operator_ == '*');
            run_values.append(move(rhs));
        }
        // 2. Replace the entire run with a Product node containing the value items of the run as its children.
        values.remove(start_of_run, end_of_run - start_of_run + 1);
        values.insert(start_of_run, make<CalcParsing::ProductNode>(move(run_values)));
    }

    // 4. Collect children into Sum and Negate nodes.
    Optional<CalcParsing::Node> single_value;
    {
        // 1. For each "-" operator item in values, replace its right-hand value item rhs with a Negate node containing rhs as its child.
        for (auto i = 0u; i < values.size(); ++i) {
            auto& maybe_minus_operator = values[i];
            if (!maybe_minus_operator.has<CalcParsing::Operator>() || maybe_minus_operator.get<CalcParsing::Operator>().delim != '-')
                continue;

            auto rhs_index = ++i;
            auto negate_node = make<CalcParsing::NegateNode>(move(values[rhs_index]));
            values.remove(rhs_index);
            values.insert(rhs_index, move(negate_node));
        }

        // 2. If values has only one item, and it is a Product node or a parenthesized simple block, replace values with that item.
        if (values.size() == 1) {
            values.first().visit(
                [&](ComponentValue const& component_value) {
                    if (component_value.is_block() && component_value.block().is_paren())
                        single_value = NonnullRawPtr { component_value };
                },
                [&](NonnullOwnPtr<CalcParsing::ProductNode>& node) {
                    single_value = move(node);
                },
                [](auto&) {});
        }
        //    Otherwise, replace values with a Sum node containing the value items of values as its children.
        if (!single_value.has_value()) {
            auto operator_count = 0u;
            for (size_t i = 0; i < values.size();) {
                auto& value = values[i];
                if (value.has<CalcParsing::Operator>()) {
                    operator_count++;
                    values.remove(i);
                } else {
                    i++;
                }
            }
            if (values.size() == 0 || operator_count != values.size() - 1)
                return nullptr;

            single_value = make<CalcParsing::SumNode>(move(values));
        }
    }
    VERIFY(single_value.has_value());

    // 5. At this point values is a tree of Sum, Product, Negate, and Invert nodes, with other types of values at the leaf nodes. Process the leaf nodes.
    // NOTE: We process leaf nodes as part of this conversion.
    auto calculation_tree = convert_to_calculation_node(*single_value, context);
    if (!calculation_tree)
        return nullptr;

    // 6. Return the result of simplifying a calculation tree from values.
    transaction.commit();
    return simplify_a_calculation_tree(*calculation_tree, context, CalculationResolutionContext {});
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
        auto parsed_boolean_expression = RustComponentValueParser::parse_an_if_condition(serialized_if_condition.bytes_as_string_view(), "utf-8"sv, [&](Vector<ComponentValue>&& component_values) -> OwnPtr<BooleanExpression> {
            TokenStream<ComponentValue> test_tokens { component_values };
            auto const& maybe_function_token = test_tokens.consume_a_token();

            if (!maybe_function_token.is_function())
                return nullptr;

            auto const& function = maybe_function_token.function();
            TokenStream argument_tokens { function.value };

            // supports( [ <ident> : <declaration-value> ] | <supports-condition> )
            if (function.name.equals_ignoring_ascii_case("supports"sv)) {
                // [ <ident> : <declaration-value> ]
                m_rule_context.append(RuleContext::SupportsCondition);
                auto maybe_supports_declaration = parse_supports_declaration(argument_tokens);
                m_rule_context.take_last();

                if (maybe_supports_declaration)
                    return maybe_supports_declaration;

                // <supports-condition>
                if (auto maybe_supports_condition = materialize_rust_supports_condition(function.value))
                    return maybe_supports_condition;

                return nullptr;
            }

            // media( <media-feature> | <media-condition> )
            if (function.name.equals_ignoring_ascii_case("media"sv)) {
                auto serialized_media_test = serialize_component_values_for_reparsing(function.value);
                return RustComponentValueParser::parse_a_media_test(serialized_media_test.bytes_as_string_view(), "utf-8"sv, [this](RustComponentValueParser::MediaFeatureTest&& media_feature_test) -> OwnPtr<BooleanExpression> {
                    return materialize_rust_media_feature_test(move(media_feature_test));
                });
            }

            // FIXME: Support style()
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
    if (auto value = parse_number_percentage_value(tokens, infinite_range, infinite_range))
        return OpacityValueStyleValue::create(value.release_nonnull());

    return nullptr;
}

// https://drafts.csswg.org/css-fonts/#typedef-opentype-tag
RefPtr<StringStyleValue const> Parser::parse_opentype_tag_value(TokenStream<ComponentValue>& tokens)
{
    // <opentype-tag> = <string>
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    auto start = tokens.current_index();
    if (!tokens.has_next_token())
        return nullptr;
    tokens.discard_a_token();

    auto serialized_opentype_tag = serialize_component_values_for_reparsing(tokens.tokens_since(start));
    auto opentype_tag = RustComponentValueParser::parse_an_opentype_tag(serialized_opentype_tag.bytes_as_string_view(), "utf-8"sv);
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

// https://drafts.csswg.org/css-transforms-1/#typedef-transform-function
RefPtr<StyleValue const> Parser::parse_transform_function_value(TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    auto const& part = tokens.consume_a_token();
    if (!part.is_function())
        return nullptr;
    auto maybe_function = transform_function_from_string(part.function().name);
    if (!maybe_function.has_value())
        return nullptr;

    auto context_guard = push_temporary_value_parsing_context(FunctionContext { part.function().name });

    auto function = maybe_function.release_value();
    auto function_metadata = transform_function_metadata(function);

    auto function_tokens = TokenStream { part.function().value };
    auto arguments = parse_a_comma_separated_list_of_component_values(function_tokens);

    if (arguments.size() > function_metadata.parameters.size()) {
        ErrorReporter::the().report(InvalidValueError {
            .value_type = "<transform-function>"_fly_string,
            .value_string = part.function().original_source_text(),
            .description = MUST(String::formatted("Too many arguments to {}. max: {}", part.function().name, function_metadata.parameters.size())),
        });
        return nullptr;
    }

    if (arguments.size() < function_metadata.parameters.size() && function_metadata.parameters[arguments.size()].required) {
        ErrorReporter::the().report(InvalidValueError {
            .value_type = "<transform-function>"_fly_string,
            .value_string = part.function().original_source_text(),
            .description = MUST(String::formatted("Required parameter at position {} is missing", arguments.size())),
        });
        return nullptr;
    }

    StyleValueVector values;
    for (auto argument_index = 0u; argument_index < arguments.size(); ++argument_index) {
        TokenStream argument_tokens { arguments[argument_index] };
        argument_tokens.discard_whitespace();

        switch (function_metadata.parameters[argument_index].type) {
        case TransformFunctionParameterType::Angle: {
            // These are `<angle> | <zero>` in the spec, so we have to check for both kinds.
            if (auto angle_value = parse_angle_value(argument_tokens, infinite_range)) {
                values.append(angle_value.release_nonnull());
                break;
            }
            if (argument_tokens.next_token().is(Token::Type::Number) && argument_tokens.next_token().token().number_value() == 0) {
                argument_tokens.discard_a_token(); // 0
                values.append(AngleStyleValue::create(Angle::make_degrees(0)));
                break;
            }
            return nullptr;
        }
        case TransformFunctionParameterType::Length:
        case TransformFunctionParameterType::LengthNone: {
            if (auto length_value = parse_length_value(argument_tokens, infinite_range)) {
                values.append(length_value.release_nonnull());
                break;
            }
            if (function_metadata.parameters[argument_index].type == TransformFunctionParameterType::LengthNone
                && argument_tokens.next_token().is_ident("none"sv)) {

                argument_tokens.discard_a_token(); // none
                values.append(KeywordStyleValue::create(Keyword::None));
                break;
            }
            return nullptr;
        }
        case TransformFunctionParameterType::LengthPercentage: {
            if (auto length_percentage_value = parse_length_percentage_value(argument_tokens, infinite_range, infinite_range)) {
                values.append(length_percentage_value.release_nonnull());
                break;
            }
            return nullptr;
        }
        case TransformFunctionParameterType::Number: {
            if (auto number_value = parse_number_value(argument_tokens, infinite_range)) {
                values.append(number_value.release_nonnull());
                break;
            }
            return nullptr;
        }
        case TransformFunctionParameterType::NumberPercentage: {
            if (auto number_percentage_value = parse_number_percentage_value(argument_tokens, infinite_range, infinite_range)) {
                values.append(number_percentage_value.release_nonnull());
                break;
            }
            return nullptr;
        }
        }

        argument_tokens.discard_whitespace();
        if (argument_tokens.has_next_token())
            return nullptr;
    }

    transaction.commit();
    return TransformationStyleValue::create(PropertyID::Transform, function, move(values));
}

// https://drafts.csswg.org/css-transforms-1/#typedef-transform-list
RefPtr<StyleValue const> Parser::parse_transform_list_value(TokenStream<ComponentValue>& tokens)
{
    // <transform-list> = <transform-function>+
    // https://www.w3.org/TR/css-transforms-1/#transform-property
    StyleValueVector transformations;
    auto transaction = tokens.begin_transaction();
    while (tokens.has_next_token()) {
        if (auto maybe_function = parse_transform_function_value(tokens)) {
            transformations.append(maybe_function.release_nonnull());
            tokens.discard_whitespace();
            continue;
        }
        break;
    }
    if (transformations.is_empty())
        return {};
    transaction.commit();
    return StyleValueList::create(move(transformations), StyleValueList::Separator::Space);
}

RefPtr<StyleValue const> Parser::parse_value(ValueType value_type, TokenStream<ComponentValue>& tokens)
{
    switch (value_type) {
    case ValueType::Anchor:
        return parse_anchor(tokens);
    case ValueType::AnchorSize:
        return parse_anchor_size(tokens);
    case ValueType::Angle:
        return parse_angle_value(tokens, infinite_range);
    case ValueType::AnglePercentage:
        return parse_angle_percentage_value(tokens, infinite_range, infinite_range);
    case ValueType::BackgroundPosition:
        return parse_position_value(tokens, PositionParsingMode::BackgroundPosition);
    case ValueType::BasicShape:
        return parse_basic_shape_value(tokens);
    case ValueType::Color:
        return parse_color_value(tokens);
    case ValueType::CornerShape:
        return parse_corner_shape_value(tokens);
    case ValueType::Counter:
        return parse_counter_value(tokens);
    case ValueType::CounterStyle:
        return parse_counter_style_value(tokens);
    case ValueType::CustomIdent:
        // FIXME: Figure out how to pass the blacklist here
        return parse_custom_ident_value(tokens, {});
    case ValueType::DashedIdent:
        return parse_dashed_ident_value(tokens);
    case ValueType::EasingFunction:
        return parse_easing_value(tokens);
    case ValueType::FilterValueList:
        return parse_filter_value_list_value(tokens);
    case ValueType::FitContent:
        return parse_fit_content_value(tokens);
    case ValueType::Flex:
        return parse_flex_value(tokens, infinite_range);
    case ValueType::FontStyle:
        return parse_font_style_value(tokens);
    case ValueType::FontKerningValue:
        return parse_font_kerning_value_value(tokens);
    case ValueType::FontOpticalSizingValue:
        return parse_font_optical_sizing_value_value(tokens);
    case ValueType::FontWeightAbsolute:
        return parse_font_weight_absolute_value(tokens);
    case ValueType::FontWidthCss3:
        return parse_font_width_css3_value(tokens);
    case ValueType::FontVariantAlternates:
        return parse_font_variant_alternates_value(tokens);
    case ValueType::FontVariantCapsValue:
        return parse_font_variant_caps_value_value(tokens);
    case ValueType::FontVariantCss2:
        return parse_font_variant_css2_value(tokens);
    case ValueType::FontVariantEastAsian:
        return parse_font_variant_east_asian_value(tokens);
    case ValueType::FontVariantEmojiValue:
        return parse_font_variant_emoji_value_value(tokens);
    case ValueType::FontVariantLigatures:
        return parse_font_variant_ligatures_value(tokens);
    case ValueType::FontVariantNumeric:
        return parse_font_variant_numeric_value(tokens);
    case ValueType::FontVariantPositionValue:
        return parse_font_variant_position_value_value(tokens);
    case ValueType::Frequency:
        return parse_frequency_value(tokens, infinite_range);
    case ValueType::FrequencyPercentage:
        return parse_frequency_percentage_value(tokens, infinite_range, infinite_range);
    case ValueType::Image:
        return parse_image_value(tokens);
    case ValueType::Integer:
        return parse_integer_value(tokens, infinite_integer_range);
    case ValueType::Length:
        return parse_length_value(tokens, infinite_range);
    case ValueType::LengthPercentage:
        return parse_length_percentage_value(tokens, infinite_range, infinite_range);
    case ValueType::Number:
        return parse_number_value(tokens, infinite_range);
    case ValueType::OpacityValue:
        return parse_opacity_value_value(tokens);
    case ValueType::OpentypeTag:
        return parse_opentype_tag_value(tokens);
    case ValueType::Paint:
        return parse_paint_value(tokens);
    case ValueType::Percentage:
        return parse_percentage_value(tokens, infinite_range);
    case ValueType::Position:
        return parse_position_value(tokens);
    case ValueType::Ratio:
        return parse_ratio_value(tokens);
    case ValueType::Rect:
        return parse_rect_value(tokens);
    case ValueType::Resolution:
        return parse_resolution_value(tokens, infinite_range);
    case ValueType::ScrollFunction:
        return parse_scroll_function_value(tokens);
    case ValueType::String:
        return parse_string_value(tokens);
    case ValueType::Symbol:
        return parse_symbol_value(tokens);
    case ValueType::Time:
        return parse_time_value(tokens, infinite_range);
    case ValueType::TimePercentage:
        return parse_time_percentage_value(tokens, infinite_range, infinite_range);
    case ValueType::TransformFunction:
        return parse_transform_function_value(tokens);
    case ValueType::TransformList:
        return parse_transform_list_value(tokens);
    case ValueType::Url:
        return parse_url_value(tokens);
    case ValueType::ViewFunction:
        return parse_view_function_value(tokens);
    case ValueType::ViewTimelineInset:
        return parse_view_timeline_inset_value(tokens);
    }
    VERIFY_NOT_REACHED();
}

}
