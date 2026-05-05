/*
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/GenericShorthands.h>
#include <LibWeb/CSS/Parser/ArbitrarySubstitutionFunctions.h>
#include <LibWeb/CSS/Parser/ErrorReporter.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/Parser/Syntax.h>
#include <LibWeb/CSS/Parser/SyntaxParsing.h>
#include <LibWeb/CSS/Parser/TokenStream.h>
#include <LibWeb/CSS/Serialize.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/GuaranteedInvalidStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>
#include <LibWeb/CSS/ValueType.h>

namespace Web::CSS::Parser {

static bool serialize_component_value_for_reparsing(StringBuilder& builder, ComponentValue const& component_value)
{
    if (component_value.is_token()) {
        if (component_value.token().is(Token::Type::EndOfFile) || component_value.token().is(Token::Type::Invalid))
            return false;

        auto original_source_text = component_value.original_source_text();
        builder.append(original_source_text.is_empty() ? component_value.to_string() : original_source_text);
        return true;
    }

    if (component_value.is_block()) {
        auto const& block = component_value.block();
        builder.append(block.token.bracket_string());
        for (auto const& child : block.value) {
            if (!serialize_component_value_for_reparsing(builder, child))
                return false;
        }
        builder.append(block.token.bracket_mirror_string());
        return true;
    }

    if (component_value.is_function()) {
        auto const& function = component_value.function();
        serialize_an_identifier(builder, function.name);
        builder.append('(');
        for (auto const& child : function.value) {
            if (!serialize_component_value_for_reparsing(builder, child))
                return false;
        }
        builder.append(')');
        return true;
    }

    builder.append(component_value.to_string());
    return true;
}

static Optional<String> serialize_component_values_for_reparsing(Vector<ComponentValue> const& component_values)
{
    StringBuilder builder;
    for (auto const& component_value : component_values) {
        if (!serialize_component_value_for_reparsing(builder, component_value))
            return {};
    }
    return builder.to_string_without_validation();
}

static OwnPtr<SyntaxNode> parse_syntax_single_component(TokenStream<ComponentValue>& tokens, LimitSingleComponentIdentToCustomIdent limit_single_component_ident_to_custom_ident)
{
    // <syntax-single-component> = '<' <syntax-type-name> '>' | <ident>
    // <syntax-type-name> = angle | color | custom-ident | image | integer
    //                    | length | length-percentage | number
    //                    | percentage | resolution | string | time
    //                    | url | transform-function

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    // <ident>
    if (tokens.next_token().is(Token::Type::Ident)) {
        auto ident = tokens.consume_a_token().token().ident();

        // AD-HOC: Some users (i.e. the @property syntax descriptor) only allow custom idents here,
        //         https://github.com/w3c/csswg-drafts/issues/13614
        if (limit_single_component_ident_to_custom_ident == LimitSingleComponentIdentToCustomIdent::Yes && !is_valid_custom_ident(ident, {}))
            return {};

        transaction.commit();
        return IdentSyntaxNode::create(ident, limit_single_component_ident_to_custom_ident == LimitSingleComponentIdentToCustomIdent::Yes ? CaseSensitivity::CaseSensitive : CaseSensitivity::CaseInsensitive);
    }

    // '<' <syntax-type-name> '>'
    if (tokens.next_token().is_delim('<')) {
        tokens.discard_a_token(); // '<'
        auto const& type_name = tokens.consume_a_token();
        auto const& end_token = tokens.consume_a_token();

        if (end_token.is_delim('>')
            && type_name.is(Token::Type::Ident)
            && first_is_one_of(type_name.token().ident(), "angle"sv,
                "color"sv,
                "custom-ident"sv,
                "image"sv,
                "integer"sv,
                "length"sv,
                "length-percentage"sv,
                "number"sv,
                "percentage"sv,
                "resolution"sv,
                "string"sv,
                "time"sv,
                "url"sv,
                "transform-function"sv)) {
            transaction.commit();
            return TypeSyntaxNode::create(type_name.token().ident());
        }
    }

    return nullptr;
}

static Optional<char> parse_syntax_multiplier(TokenStream<ComponentValue>& tokens)
{
    // <syntax-multiplier> = [ '#' | '+' ]
    auto transaction = tokens.begin_transaction();

    auto delim = tokens.consume_a_token();
    if (delim.is_delim('#') || delim.is_delim('+')) {
        transaction.commit();
        return delim.token().delim();
    }

    return {};
}

OwnPtr<SyntaxNode> parse_syntax_component(TokenStream<ComponentValue>& tokens, LimitSingleComponentIdentToCustomIdent limit_single_component_ident_to_custom_ident)
{
    // <syntax-component> = <syntax-single-component> <syntax-multiplier>?
    //                    | '<' transform-list '>'

    auto transaction = tokens.begin_transaction();

    tokens.discard_whitespace();

    // '<' transform-list '>'
    if (tokens.next_token().is_delim('<')) {
        auto transform_list_transaction = transaction.create_child();
        tokens.discard_a_token(); // '<'
        auto& ident_token = tokens.consume_a_token();
        auto& end_token = tokens.consume_a_token();

        if (ident_token.is_ident("transform-list"sv) && end_token.is_delim('>')) {
            transform_list_transaction.commit();
            return TypeSyntaxNode::create("transform-list"_fly_string);
        }
    }

    // <syntax-single-component> <syntax-multiplier>?
    auto syntax_single_component = parse_syntax_single_component(tokens, limit_single_component_ident_to_custom_ident);
    if (!syntax_single_component)
        return nullptr;

    auto multiplier = parse_syntax_multiplier(tokens);
    if (!multiplier.has_value()) {
        transaction.commit();
        return syntax_single_component.release_nonnull();
    }

    switch (multiplier.value()) {
    case '#':
        transaction.commit();
        return CommaSeparatedMultiplierSyntaxNode::create(syntax_single_component.release_nonnull());
    case '+':
        transaction.commit();
        return MultiplierSyntaxNode::create(syntax_single_component.release_nonnull());
    default:
        return nullptr;
    }
}

// https://drafts.csswg.org/css-values-5/#typedef-syntax
OwnPtr<SyntaxNode> parse_as_syntax(Vector<ComponentValue> const& component_values, LimitSingleComponentIdentToCustomIdent limit_single_component_ident_to_custom_ident)
{
    // <syntax> = '*' | <syntax-component> [ <syntax-combinator> <syntax-component> ]* | <syntax-string>
    // <syntax-component> = <syntax-single-component> <syntax-multiplier>?
    //                    | '<' transform-list '>'
    // <syntax-single-component> = '<' <syntax-type-name> '>' | <ident>
    // <syntax-type-name> = angle | color | custom-ident | image | integer
    //                    | length | length-percentage | number
    //                    | percentage | resolution | string | time
    //                    | url | transform-function
    // <syntax-combinator> = '|'
    // <syntax-multiplier> = [ '#' | '+' ]
    //
    // <syntax-string> = <string>
    // FIXME: Eventually, extend this to also parse *any* CSS grammar, not just for the <syntax> type.
    auto serialized_syntax = serialize_component_values_for_reparsing(component_values);
    if (!serialized_syntax.has_value())
        return {};
    return RustComponentValueParser::parse_as_syntax(serialized_syntax->bytes_as_string_view(), "utf-8"sv, limit_single_component_ident_to_custom_ident);
}

NonnullRefPtr<StyleValue const> parse_with_a_syntax(ParsingParams const& parsing_params, Vector<ComponentValue> const& input, SyntaxNode const& syntax)
{
    return Parser::create(parsing_params, ""sv).parse_with_a_syntax(input, syntax);
}

RefPtr<StyleValue const> Parser::parse_according_to_syntax_node(TokenStream<ComponentValue>& tokens, SyntaxNode const& syntax_node)
{
    auto transaction = tokens.begin_transaction();

    switch (syntax_node.type()) {
    case SyntaxNode::NodeType::Universal:
        if (auto declaration_value = parse_declaration_value(tokens); declaration_value.has_value()) {
            SubstitutionFunctionsPresence substitution_functions_presence;
            if (collect_arbitrary_substitution_function_presence(declaration_value.value(), substitution_functions_presence).is_error())
                return nullptr;

            transaction.commit();
            return UnresolvedStyleValue::create(declaration_value.release_value(), substitution_functions_presence);
        }
        return nullptr;
    case SyntaxNode::NodeType::Ident: {
        auto const& ident_node = as<IdentSyntaxNode>(syntax_node);
        tokens.discard_whitespace();
        auto token = tokens.consume_a_token();

        if (!token.is(Token::Type::Ident))
            return nullptr;

        auto ident = token.token().ident();

        if (ident_node.case_sensitivity() == CaseSensitivity::CaseSensitive ? ident == ident_node.ident() : ident.equals_ignoring_ascii_case(ident_node.ident())) {
            transaction.commit();
            if (auto keyword = keyword_from_string(ident_node.ident()); keyword.has_value())
                return KeywordStyleValue::create(keyword.release_value());
            return CustomIdentStyleValue::create(ident_node.ident());
        }
        return nullptr;
    }
    case SyntaxNode::NodeType::Type: {
        auto const& type_node = as<TypeSyntaxNode>(syntax_node);
        auto const& type_name = type_node.type_name();
        if (auto value_type = value_type_from_string(type_name); value_type.has_value()) {
            if (auto result = parse_value(*value_type, tokens)) {
                transaction.commit();
                return result.release_nonnull();
            }
            return nullptr;
        }

        ErrorReporter::the().report(InvalidValueError {
            .value_type = MUST(String::formatted("<{}>", type_name)),
            .value_string = tokens.dump_string(),
            .description = "Unknown type in <syntax>."_string,
        });
        return nullptr;
    }
    case SyntaxNode::NodeType::Multiplier: {
        auto const& multiplier_node = as<MultiplierSyntaxNode>(syntax_node);
        StyleValueVector values;
        tokens.discard_whitespace();
        while (tokens.has_next_token()) {
            auto parsed_child = parse_according_to_syntax_node(tokens, multiplier_node.child());
            if (!parsed_child)
                break;
            values.append(parsed_child.release_nonnull());
            tokens.discard_whitespace();
        }
        if (values.is_empty())
            return nullptr;
        transaction.commit();
        return StyleValueList::create(move(values), StyleValueList::Separator::Space);
    }
    case SyntaxNode::NodeType::CommaSeparatedMultiplier: {
        auto const& multiplier_node = as<CommaSeparatedMultiplierSyntaxNode>(syntax_node);
        auto result = parse_comma_separated_value_list(tokens, [&](auto& tokens) {
            return parse_according_to_syntax_node(tokens, multiplier_node.child());
        });
        if (!result)
            return nullptr;
        transaction.commit();
        return result.release_nonnull();
    }
    case SyntaxNode::NodeType::Alternatives: {
        auto const& alternatives_node = as<AlternativesSyntaxNode>(syntax_node);
        for (auto const& child : alternatives_node.children()) {
            auto alternative_transaction = transaction.create_child();
            auto result = parse_according_to_syntax_node(tokens, *child);
            tokens.discard_whitespace();

            if (result && tokens.is_empty()) {
                alternative_transaction.commit();
                return result.release_nonnull();
            }
        }
        return nullptr;
    }
    }

    VERIFY_NOT_REACHED();
}

// https://drafts.csswg.org/css-values-5/#parse-with-a-syntax
NonnullRefPtr<StyleValue const> Parser::parse_with_a_syntax(Vector<ComponentValue> const& input, SyntaxNode const& syntax)
{
    // 1. Parse a list of component values from values, and let raw parse be the result.
    // NB: Already done before this point.

    // FIXME: 2. If el was given, substitute arbitrary substitution functions in raw parse, and set raw parse to that result.
    // NB: This is currently a no-op because our only caller already substitutes ASFs in the input before calling us.
    // FIXME: Move substitute_arbitrary_substitution_functions() into the Parser, and keep the guarded contexts there,
    //        so we don't have this awkward situation of needing to pass that to random other functions.

    // 3. parse values according to syntax, with a * value treated as <declaration-value>?, and let parsed result be
    //    the result.
    //    If syntax used a | combinator, let parsed result be the parse result from the first matching clause.
    TokenStream tokens { input };
    auto parsed_result = parse_according_to_syntax_node(tokens, syntax);
    tokens.discard_whitespace();

    // 4. If parsed result is failure, return the guaranteed-invalid value.
    if (!parsed_result || tokens.has_next_token())
        return GuaranteedInvalidStyleValue::create();

    // 5. Assert: parsed result is now a well-defined list of one or more CSS values, since each branch of a <syntax>
    //    defines an unambiguous parse result (or the * syntax is unambiguous on its own).
    // NB: Nothing to do.

    // 6. Return parsed result.
    return parsed_result.release_nonnull();
}

}
