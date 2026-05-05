#!/usr/bin/env python3

# Copyright (c) 2026, Callum Law <callumlaw1709@outlook.com>
#
# SPDX-License-Identifier: BSD-2-Clause

import argparse
import json
import sys

from pathlib import Path
from typing import Any
from typing import TextIO

sys.path.append(str(Path(__file__).resolve().parent.parent))

from Utils.CSSGrammar.generator import generate_css_parser_expression_for_grammar
from Utils.utils import snake_casify
from Utils.utils import title_casify


def validate_branch_syntax(syntax: Any, value_type_name: str, branch_name: str, json_path: str) -> bool:
    if not isinstance(syntax, dict):
        print(
            f"{json_path}: Value type `{value_type_name}` branch `{branch_name}` has syntax that is not an object",
            file=sys.stderr,
        )
        return False

    if "type" not in syntax:
        print(
            f"{json_path}: Value type `{value_type_name}` branch `{branch_name}` is missing a syntax type",
            file=sys.stderr,
        )
        return False

    syntax_type = syntax["type"]
    if not isinstance(syntax_type, str):
        print(
            f"{json_path}: Value type `{value_type_name}` branch `{branch_name}` has a syntax type that is not a string",
            file=sys.stderr,
        )
        return False

    required_fields_by_type = {
        "custom-ident": (),
        "ident": ("value",),
        "number": ("min", "max"),
        "string": (),
    }
    if syntax_type not in required_fields_by_type:
        print(
            f"{json_path}: Value type `{value_type_name}` branch `{branch_name}` has unsupported syntax type `{syntax_type}`",
            file=sys.stderr,
        )
        return False

    is_valid = True
    allowed_fields = ("type", *required_fields_by_type[syntax_type])
    for field_name in syntax:
        if field_name in allowed_fields:
            continue

        print(
            f"{json_path}: Value type `{value_type_name}` branch `{branch_name}` has unexpected syntax field `{field_name}`",
            file=sys.stderr,
        )
        is_valid = False

    for field_name in required_fields_by_type[syntax_type]:
        if field_name in syntax:
            continue

        print(
            f"{json_path}: Value type `{value_type_name}` branch `{branch_name}` is missing syntax field `{field_name}`",
            file=sys.stderr,
        )
        is_valid = False

    return is_valid


def validate_branches(branches: Any, value_type_name: str, json_path: str) -> bool:
    if not isinstance(branches, list):
        print(f"{json_path}: Value type `{value_type_name}` has branches that are not a list", file=sys.stderr)
        return False

    is_valid = True
    for branch in branches:
        branch_name = "<unknown>"
        if isinstance(branch, dict) and isinstance(branch.get("name"), str):
            branch_name = branch["name"]

        if not isinstance(branch, dict):
            print(f"{json_path}: Value type `{value_type_name}` branch is not an object", file=sys.stderr)
            is_valid = False
            continue

        for field_name in branch:
            if field_name in ("name", "syntax"):
                continue

            print(
                f"{json_path}: Value type `{value_type_name}` branch `{branch_name}` has unexpected field `{field_name}`",
                file=sys.stderr,
            )
            is_valid = False

        if "name" not in branch:
            print(f"{json_path}: Value type `{value_type_name}` branch is missing a name", file=sys.stderr)
            is_valid = False
        elif not isinstance(branch["name"], str):
            print(
                f"{json_path}: Value type `{value_type_name}` branch has a name that is not a string", file=sys.stderr
            )
            is_valid = False

        if "syntax" not in branch:
            print(
                f"{json_path}: Value type `{value_type_name}` branch `{branch_name}` is missing syntax",
                file=sys.stderr,
            )
            is_valid = False
        elif not validate_branch_syntax(branch["syntax"], value_type_name, branch_name, json_path):
            is_valid = False

    return is_valid


def json_is_valid(value_type_data: dict[str, Any], json_path: str) -> bool:
    is_valid = True
    most_recent_value_type_name = ""

    for value_type_name, value_type_definition in value_type_data.items():
        if value_type_name.lower() < most_recent_value_type_name.lower():
            print(
                f"{json_path}: Value type `{value_type_name}` is in the wrong position. Please keep this list alphabetical!",
                file=sys.stderr,
            )
            is_valid = False

        most_recent_value_type_name = value_type_name

        if not isinstance(value_type_definition, dict):
            print(f"{json_path}: Value type `{value_type_name}` is not an object", file=sys.stderr)
            is_valid = False
            continue

        if "spec" not in value_type_definition:
            print(f"{json_path}: Value type `{value_type_name}` is missing a spec link", file=sys.stderr)
            is_valid = False
        elif not isinstance(value_type_definition["spec"], str):
            print(f"{json_path}: Value type `{value_type_name}` has a spec field that is not a string", file=sys.stderr)
            is_valid = False

        if "grammar" not in value_type_definition:
            print(f"{json_path}: Value type `{value_type_name}` is missing a grammar", file=sys.stderr)
            is_valid = False
        elif not isinstance(value_type_definition["grammar"], str):
            print(
                f"{json_path}: Value type `{value_type_name}` has a grammar field that is not a string",
                file=sys.stderr,
            )
            is_valid = False

        if "branches" in value_type_definition and not validate_branches(
            value_type_definition["branches"], value_type_name, json_path
        ):
            is_valid = False

        for field_name in value_type_definition:
            if field_name in ("spec", "grammar", "branches", "__comment"):
                continue

            print(
                f"{json_path}: Value type `{value_type_name}` has an unexpected field `{field_name}`",
                file=sys.stderr,
            )
            is_valid = False

    return is_valid


def value_type_name_to_snake_case(value_type_name: str) -> str:
    return snake_casify(value_type_name[1:-1])


def value_type_name_to_title_case(value_type_name: str) -> str:
    return title_casify(value_type_name[1:-1])


def generate_css_materialization_expression(out: TextIO, branch: dict[str, Any]) -> None:
    syntax = branch["syntax"]
    syntax_type = syntax["type"]

    if syntax_type == "ident":
        keyword_name = title_casify(syntax["value"])
        out.write(f"        return parse_specific_keyword_value(tokens, Keyword::{keyword_name});\n")
        return

    if syntax_type == "number":
        minimum = syntax["min"]
        maximum = syntax["max"]
        out.write(f"        return parse_number_value(tokens, {{ {minimum}, {maximum} }});\n")
        return

    if syntax_type == "string":
        out.write("        return parse_string_value(tokens);\n")
        return

    if syntax_type == "custom-ident":
        out.write("        return parse_custom_ident_value(tokens);\n")
        return

    raise RuntimeError(f"unsupported value type branch syntax type: {syntax_type}")


def generate_rust_parser_expression(
    out: TextIO, value_type_name: str, value_type_definition: dict[str, Any], value_type_index: int
) -> bool:
    if "branches" not in value_type_definition:
        return False

    out.write(f"""    auto syntax_kind = RustComponentValueParser::parse_a_value_type({value_type_index}, tokens);
    switch (syntax_kind) {{
""")

    for branch in value_type_definition["branches"]:
        branch_name = branch["name"]
        out.write(
            f"    case FFI::CssValueTypeSyntaxKind::{value_type_name_to_title_case(value_type_name)}{branch_name}:\n"
        )
        generate_css_materialization_expression(out, branch)

    out.write("""    default:
        return nullptr;
    }
""")
    return True


def generate_header_file(out: TextIO, value_type_data: dict[str, Any]) -> None:
    out.write("""// This file is generated by generate_libweb_css_value_types_parsing.py

#pragma once

namespace Web::CSS {

#define ENUMERATE_GENERATED_CSS_VALUE_TYPES \\
""")

    for value_type_name in value_type_data:
        out.write(f"    __ENUMERATE_GENERATED_CSS_VALUE_TYPE({value_type_name_to_snake_case(value_type_name)}) \\\n")

    out.write("\n")
    out.write("}")


def generate_implementation_file(out: TextIO, value_type_data: dict[str, Any]) -> None:
    out.write("""// This file is generated by generate_libweb_css_value_types_parsing.py

#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>

#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>

namespace Web::CSS::Parser {

""")

    for value_type_index, (value_type_name, value_type_definition) in enumerate(value_type_data.items()):
        spec_link = value_type_definition["spec"]
        grammar = value_type_definition["grammar"]
        name_snake_case = value_type_name_to_snake_case(value_type_name)

        out.write(f"""
// {spec_link}
RefPtr<StyleValue const> Parser::parse_{name_snake_case}_value(TokenStream<ComponentValue>& tokens)
{{
    // {value_type_name} = {grammar}
""")
        if generate_rust_parser_expression(out, value_type_name, value_type_definition, value_type_index):
            out.write("""}
""")
        else:
            generate_css_parser_expression_for_grammar(out, name_snake_case, grammar)
            out.write(f"""    return {name_snake_case};
}}
""")

    out.write("}\n")


def main() -> int:
    parser = argparse.ArgumentParser(description="Generate CSS value types parsing methods", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument(
        "-h",
        "--header",
        required=True,
        help="Path to the GeneratedValueTypesParsing header file to generate",
    )
    parser.add_argument(
        "-c",
        "--implementation",
        required=True,
        help="Path to the GeneratedValueTypesParsing implementation file to generate",
    )
    parser.add_argument("-j", "--json", required=True, help="Path to the JSON file to read from")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as json_file:
        value_type_data = json.load(json_file)

    if not isinstance(value_type_data, dict):
        raise RuntimeError(f"{args.json}: expected a JSON object")

    if not json_is_valid(value_type_data, args.json):
        sys.exit(1)

    with (
        open(args.header, "w", encoding="utf-8") as header_file,
        open(args.implementation, "w", encoding="utf-8") as implementation_file,
    ):
        generate_header_file(header_file, value_type_data)
        generate_implementation_file(implementation_file, value_type_data)

    return 0


if __name__ == "__main__":
    sys.exit(main())
