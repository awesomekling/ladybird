/*
 * Copyright (c) 2018-2022, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2020-2021, the SerenityOS developers.
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2022, MacDue <macdue@dueutil.tech>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/StyleValues/AngleStyleValue.h>

namespace Web::CSS::Parser {

Optional<Vector<ColorStopListElement>> Parser::parse_color_stop_list(TokenStream<ComponentValue>& tokens, auto parse_position)
{
    enum class ElementType {
        Garbage,
        ColorStop,
        ColorHint
    };

    auto parse_color_stop_list_element = [&](auto& element) -> ElementType {
        tokens.discard_whitespace();
        if (!tokens.has_next_token())
            return ElementType::Garbage;

        RefPtr<StyleValue const> color;
        RefPtr<StyleValue const> position;
        RefPtr<StyleValue const> second_position;
        if (position = parse_position(tokens); position) {
            // [<T-percentage> <color>] or [<T-percentage>]
            tokens.discard_whitespace();
            // <T-percentage>
            if (!tokens.has_next_token() || tokens.next_token().is(Token::Type::Comma)) {
                element.transition_hint = position;
                return ElementType::ColorHint;
            }
            // <T-percentage> <color>
            auto maybe_color = parse_color_value(tokens);
            if (!maybe_color)
                return ElementType::Garbage;
            color = maybe_color.release_nonnull();
        } else {
            // [<color> <T-percentage>?]
            auto maybe_color = parse_color_value(tokens);
            if (!maybe_color)
                return ElementType::Garbage;
            color = maybe_color.release_nonnull();
            tokens.discard_whitespace();
            // Allow up to [<color> <T-percentage> <T-percentage>] (double-position color stops)
            // Note: Double-position color stops only appear to be valid in this order.
            for (auto stop_position : Array { &position, &second_position }) {
                if (tokens.has_next_token() && !tokens.next_token().is(Token::Type::Comma)) {
                    *stop_position = parse_position(tokens);
                    if (!stop_position)
                        return ElementType::Garbage;
                    tokens.discard_whitespace();
                }
            }
        }

        element.color_stop = ColorStopListElement::ColorStop { color, position, second_position };
        return ElementType::ColorStop;
    };

    ColorStopListElement first_element {};
    if (parse_color_stop_list_element(first_element) != ElementType::ColorStop)
        return {};

    Vector<ColorStopListElement> color_stops { first_element };
    while (tokens.has_next_token()) {
        ColorStopListElement list_element {};
        tokens.discard_whitespace();
        if (!tokens.consume_a_token().is(Token::Type::Comma))
            return {};
        auto element_type = parse_color_stop_list_element(list_element);
        if (element_type == ElementType::ColorHint) {
            // <color-hint>, <color-stop>
            tokens.discard_whitespace();
            if (!tokens.consume_a_token().is(Token::Type::Comma))
                return {};
            // Note: This fills in the color stop on the same list_element as the color hint (it does not overwrite it).
            if (parse_color_stop_list_element(list_element) != ElementType::ColorStop)
                return {};
        } else if (element_type == ElementType::ColorStop) {
            // <color-stop>
        } else {
            return {};
        }
        color_stops.append(list_element);
    }

    return color_stops;
}

Optional<Vector<ColorStopListElement>> Parser::parse_linear_color_stop_list(TokenStream<ComponentValue>& tokens)
{
    // <color-stop-list> =
    //   <linear-color-stop> , [ <linear-color-hint>? , <linear-color-stop> ]#
    return parse_color_stop_list(
        tokens,
        [&](auto& it) { return parse_length_percentage_value(it, infinite_range, infinite_range); });
}

Optional<Vector<ColorStopListElement>> Parser::parse_angular_color_stop_list(TokenStream<ComponentValue>& tokens)
{
    // <angular-color-stop-list> =
    //   <angular-color-stop> , [ <angular-color-hint>? , <angular-color-stop> ]#
    return parse_color_stop_list(
        tokens,
        [&](TokenStream<ComponentValue>& it) -> RefPtr<StyleValue const> {
            if (tokens.next_token().is(Token::Type::Number)) {
                auto transaction = tokens.begin_transaction();
                auto numeric_value = tokens.consume_a_token().token().number_value();
                if (numeric_value == 0) {
                    transaction.commit();
                    return AngleStyleValue::create(Angle::make_degrees(0));
                }
            }

            return parse_angle_percentage_value(it, infinite_range, infinite_range);
        });
}

}
