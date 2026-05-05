/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringBuilder.h>
#include <LibTest/TestCase.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>

namespace {

static String dump_component_values(Vector<Web::CSS::Parser::ComponentValue> const& component_values)
{
    StringBuilder builder;
    for (auto const& component_value : component_values)
        builder.appendff("{}\n", component_value.to_debug_string());
    return builder.to_string_without_validation();
}

static void expect_rust_component_values_match_cpp(StringView input)
{
    auto cpp_values = Web::CSS::Parser::Parser::create(Web::CSS::Parser::ParsingParams {}, input).parse_as_list_of_component_values();
    auto rust_values = Web::CSS::Parser::RustComponentValueParser::parse_a_list_of_component_values(input, "utf-8"sv);

    EXPECT_EQ(dump_component_values(rust_values), dump_component_values(cpp_values));
}

static String dump_declaration(Optional<Web::CSS::Parser::Declaration> const& declaration)
{
    if (!declaration.has_value())
        return "<invalid>"_string;

    StringBuilder builder;
    builder.appendff("{}\n", declaration->name);
    builder.appendff("{}\n", declaration->important == Web::CSS::Important::Yes ? "important"sv : "normal"sv);
    builder.append(dump_component_values(declaration->value));
    return builder.to_string_without_validation();
}

static void expect_rust_declaration_matches_cpp(StringView input)
{
    auto cpp_declaration = Web::CSS::Parser::Parser::create(Web::CSS::Parser::ParsingParams {}, input).parse_as_declaration();
    auto rust_declaration = Web::CSS::Parser::RustComponentValueParser::parse_a_declaration(input, "utf-8"sv);

    EXPECT_EQ(dump_declaration(rust_declaration), dump_declaration(cpp_declaration));
}

static void dump_rule_or_list_of_declarations(StringBuilder&, Web::CSS::Parser::RuleOrListOfDeclarations const&);

static void dump_rule(StringBuilder& builder, Web::CSS::Parser::Rule const& rule)
{
    rule.visit(
        [&](Web::CSS::Parser::AtRule const& at_rule) {
            builder.appendff("@{}{}\n", at_rule.name, at_rule.is_block_rule ? " block"sv : ""sv);
            builder.append("prelude\n"sv);
            builder.append(dump_component_values(at_rule.prelude));
            builder.append("children\n"sv);
            for (auto const& child : at_rule.child_rules_and_lists_of_declarations)
                dump_rule_or_list_of_declarations(builder, child);
        },
        [&](Web::CSS::Parser::QualifiedRule const& qualified_rule) {
            builder.append("qualified\n"sv);
            builder.append("prelude\n"sv);
            builder.append(dump_component_values(qualified_rule.prelude));
            builder.append("declarations\n"sv);
            for (auto const& declaration : qualified_rule.declarations)
                builder.append(dump_declaration(declaration));
            builder.append("children\n"sv);
            for (auto const& child : qualified_rule.child_rules)
                dump_rule_or_list_of_declarations(builder, child);
        });
}

static void dump_rule_or_list_of_declarations(StringBuilder& builder, Web::CSS::Parser::RuleOrListOfDeclarations const& rule_or_list_of_declarations)
{
    rule_or_list_of_declarations.visit(
        [&](Web::CSS::Parser::Rule const& rule) {
            dump_rule(builder, rule);
        },
        [&](Vector<Web::CSS::Parser::Declaration, 0> const& declarations) {
            builder.append("list-of-declarations\n"sv);
            for (auto const& declaration : declarations)
                builder.append(dump_declaration(declaration));
        });
}

static String dump_rule(Optional<Web::CSS::Parser::Rule> const& rule)
{
    if (!rule.has_value())
        return "<invalid>"_string;

    StringBuilder builder;
    dump_rule(builder, *rule);
    return builder.to_string_without_validation();
}

static void expect_rust_rule_matches_cpp(StringView input)
{
    auto cpp_rule = Web::CSS::Parser::Parser::create(Web::CSS::Parser::ParsingParams {}, input).parse_as_rule();
    auto rust_rule = Web::CSS::Parser::RustComponentValueParser::parse_a_rule(input, "utf-8"sv);

    EXPECT_EQ(dump_rule(rust_rule), dump_rule(cpp_rule));
}

}

TEST_CASE(basic_values)
{
    expect_rust_component_values_match_cpp("a, b 1px #id"sv);
}

TEST_CASE(simple_blocks)
{
    expect_rust_component_values_match_cpp("{ color: rgb(1 2 3); } [foo=\"bar\"] (1 + 2)"sv);
}

TEST_CASE(nested_functions)
{
    expect_rust_component_values_match_cpp("calc(1px + var(--gap, max(2em, 3rem)))"sv);
}

TEST_CASE(mismatched_blocks)
{
    expect_rust_component_values_match_cpp("{ [ ( foo }"sv);
}

TEST_CASE(eof_terminated_blocks_and_functions)
{
    expect_rust_component_values_match_cpp("{ color: red"sv);
    expect_rust_component_values_match_cpp("calc(1px + 2px"sv);
}

TEST_CASE(declarations)
{
    expect_rust_declaration_matches_cpp("color: red"sv);
    expect_rust_declaration_matches_cpp("margin: calc(1px + var(--gap)) ! important"sv);
    expect_rust_declaration_matches_cpp("--foo: { red } blue"sv);
    expect_rust_declaration_matches_cpp("color: { red } blue"sv);
    expect_rust_declaration_matches_cpp("@media screen"sv);
}

TEST_CASE(rules)
{
    expect_rust_rule_matches_cpp("a { color: red }"sv);
    expect_rust_rule_matches_cpp("@media screen { a { color: red } }"sv);
    expect_rust_rule_matches_cpp("@layer foo;"sv);
    expect_rust_rule_matches_cpp("a { color: red; @media screen { color: green } & { color: blue } }"sv);
    expect_rust_rule_matches_cpp("a { --foo: { red } blue }"sv);
}
