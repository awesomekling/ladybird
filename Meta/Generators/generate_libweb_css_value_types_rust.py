#!/usr/bin/env python3

# Copyright (c) 2026-present, the Ladybird developers.
#
# SPDX-License-Identifier: BSD-2-Clause

import argparse
import json
import sys

from pathlib import Path
from typing import TextIO

sys.path.append(str(Path(__file__).resolve().parent.parent))

from Utils.utils import title_casify


def value_type_name_without_brackets(value_type_name: str) -> str:
    return value_type_name[1:-1]


def rust_string_literal(string: str) -> str:
    return json.dumps(string)


def write_generated_parser(out: TextIO, value_type_names: list[str]) -> None:
    out.write("""
pub(crate) fn component_values_parse_as_generated_value_type(
    value_type_id: ValueTypeId,
    component_values: &[crate::css_parser::ComponentValue],
) -> crate::css_parser::CssValueTypeSyntaxKind {
    match value_type_id {""")

    for name in value_type_names:
        value_type_title = title_casify(value_type_name_without_brackets(name))
        value_type_snake = value_type_name_without_brackets(name).replace("-", "_")
        out.write(f"""
        ValueTypeId::{value_type_title} => component_values_parse_as_{value_type_snake}(component_values),""")

    out.write("""
    }
}

""")

    for name in value_type_names:
        value_type_snake = value_type_name_without_brackets(name).replace("-", "_")
        if name == "<font-weight-absolute>":
            out.write(f"""fn component_values_parse_as_{value_type_snake}(
    component_values: &[crate::css_parser::ComponentValue],
) -> crate::css_parser::CssValueTypeSyntaxKind {{
    // https://drafts.csswg.org/css-fonts-4/#typedef-font-weight-absolute
    // <font-weight-absolute> = normal | bold | <number [1,1000]>
    let component_values = crate::css_parser::strip_whitespace(component_values);

    if crate::css_parser::component_values_parse_as_ident(component_values, "normal") {{
        return crate::css_parser::CssValueTypeSyntaxKind::FontWeightAbsoluteNormal;
    }}

    if crate::css_parser::component_values_parse_as_ident(component_values, "bold") {{
        return crate::css_parser::CssValueTypeSyntaxKind::FontWeightAbsoluteBold;
    }}

    if crate::css_parser::component_values_parse_as_number(component_values, 1.0, 1000.0) {{
        return crate::css_parser::CssValueTypeSyntaxKind::FontWeightAbsoluteNumber;
    }}

    crate::css_parser::CssValueTypeSyntaxKind::Invalid
}}

""")
            continue

        if name == "<symbol>":
            out.write(f"""fn component_values_parse_as_{value_type_snake}(
    component_values: &[crate::css_parser::ComponentValue],
) -> crate::css_parser::CssValueTypeSyntaxKind {{
    // https://drafts.csswg.org/css-counter-styles-3/#typedef-symbol
    // <symbol> = <string> | <custom-ident>
    let component_values = crate::css_parser::strip_whitespace(component_values);

    if crate::css_parser::component_values_parse_as_string(component_values) {{
        return crate::css_parser::CssValueTypeSyntaxKind::SymbolString;
    }}

    if crate::css_parser::component_values_parse_as_custom_ident(component_values) {{
        return crate::css_parser::CssValueTypeSyntaxKind::SymbolCustomIdent;
    }}

    crate::css_parser::CssValueTypeSyntaxKind::Invalid
}}

""")
            continue

        raise RuntimeError(f"{name}: value type grammar is not supported by the Rust generator yet")


def write_rust_file(out: TextIO, value_type_data: dict) -> None:
    out.write("""/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum ValueTypeId {""")

    value_type_names = list(value_type_data.keys())

    for index, name in enumerate(value_type_names):
        out.write(f"""
    {title_casify(value_type_name_without_brackets(name))} = {index},""")

    out.write("""
}

#[allow(dead_code)]
pub(crate) fn value_type_id_from_u8(value_type_id: u8) -> Option<ValueTypeId> {
    match value_type_id {""")

    for index, name in enumerate(value_type_names):
        out.write(f"""
        {index} => Some(ValueTypeId::{title_casify(value_type_name_without_brackets(name))}),""")

    out.write("""
        _ => None,
    }
}

""")
    write_generated_parser(out, value_type_names)

    out.write("""
#[allow(dead_code)]
pub(crate) fn value_type_spec_link(value_type_id: ValueTypeId) -> &'static str {
    match value_type_id {""")

    for name, value_type in value_type_data.items():
        out.write(f"""
        ValueTypeId::{title_casify(value_type_name_without_brackets(name))} => {rust_string_literal(value_type["spec"])},""")

    out.write("""
    }
}

#[allow(dead_code)]
pub(crate) fn value_type_grammar(value_type_id: ValueTypeId) -> &'static str {
    match value_type_id {""")

    for name, value_type in value_type_data.items():
        out.write(f"""
        ValueTypeId::{title_casify(value_type_name_without_brackets(name))} => {rust_string_literal(value_type["grammar"])},""")

    out.write("""
    }
}
""")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Rust CSS value type metadata", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("-j", "--json", required=True, help="Path to the JSON file to read from")
    parser.add_argument("-o", "--output", required=True, help="Path to the Rust file to generate")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as input_file:
        value_type_data = json.load(input_file)

    with open(args.output, "w", encoding="utf-8") as output_file:
        write_rust_file(output_file, value_type_data)


if __name__ == "__main__":
    main()
