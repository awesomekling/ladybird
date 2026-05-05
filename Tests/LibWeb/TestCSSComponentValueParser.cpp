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
