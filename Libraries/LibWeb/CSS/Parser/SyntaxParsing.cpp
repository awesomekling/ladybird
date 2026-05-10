/*
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/Parser/ArbitrarySubstitutionFunctions.h>
#include <LibWeb/CSS/Parser/ErrorReporter.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/Syntax.h>
#include <LibWeb/CSS/Parser/SyntaxParsing.h>
#include <LibWeb/CSS/Parser/TokenStream.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/GuaranteedInvalidStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>
#include <LibWeb/CSS/ValueType.h>

namespace Web::CSS::Parser {

NonnullRefPtr<StyleValue const> parse_with_a_syntax(ParsingParams const& parsing_params, Vector<ComponentValue> const& input, SyntaxNode const& syntax)
{
    return Parser::create(parsing_params, ""sv).parse_with_a_syntax(input, syntax);
}

RefPtr<StyleValue const> Parser::parse_according_to_syntax_node(TokenStream<ComponentValue>& tokens, SyntaxNode const& syntax_node)
{
    auto transaction = tokens.begin_transaction();

    switch (syntax_node.type()) {
    case SyntaxNode::NodeType::Universal: {
        Vector<ComponentValue> remaining_tokens { tokens.remaining_tokens() };
        TokenStream declaration_value_tokens { remaining_tokens };
        auto declaration_value = parse_declaration_value(declaration_value_tokens);
        if (!declaration_value.has_value())
            return nullptr;
        declaration_value_tokens.discard_whitespace();
        if (!declaration_value_tokens.is_empty())
            return nullptr;

        SubstitutionFunctionsPresence substitution_functions_presence;
        if (collect_arbitrary_substitution_function_presence(declaration_value.value(), substitution_functions_presence).is_error())
            return nullptr;

        while (tokens.has_next_token())
            tokens.discard_a_token();

        transaction.commit();
        return UnresolvedStyleValue::create(declaration_value.release_value(), substitution_functions_presence);
    }
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
