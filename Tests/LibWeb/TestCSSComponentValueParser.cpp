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
