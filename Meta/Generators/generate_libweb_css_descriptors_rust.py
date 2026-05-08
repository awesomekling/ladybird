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

VALUE_TYPE_NAMES = {
    "<family-name>": "FamilyName",
    "<font-src-list>": "FontSrcList",
    "<font-weight-absolute>{1,2}": "FontWeightAbsolutePair",
    "<declaration-value>?": "OptionalDeclarationValue",
    "<length>": "Length",
    "<page-size>": "PageSize",
    "<percentage [0,∞]>": "PositivePercentage",
    "<string>": "String",
    "<unicode-range-token>#": "UnicodeRangeTokens",
    "<counter-style-system>": "CounterStyleSystem",
    "<counter-style-negative>": "CounterStyleNegative",
    "<symbol>": "Symbol",
    "<symbol>+": "Symbols",
    "<counter-style-range>": "CounterStyleRange",
    "<counter-style-pad>": "CounterStylePad",
    "<counter-style-name>": "CounterStyleName",
    "<counter-style-additive-symbols>": "CounterStyleAdditiveSymbols",
}


def rust_string_literal(string: str) -> str:
    return json.dumps(string)


def is_legacy_alias(descriptor: dict) -> bool:
    return "legacy-alias-for" in descriptor


def collect_all_descriptors(at_rules_data: dict) -> list:
    names = set()
    for at_rule in at_rules_data.values():
        for descriptor_name, descriptor in at_rule.get("descriptors", {}).items():
            if is_legacy_alias(descriptor):
                continue
            names.add(descriptor_name)
    return sorted(names)


def write_at_rule_id(out: TextIO, at_rules_data: dict) -> None:
    out.write("""#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum AtRuleId {""")

    for index, at_rule_name in enumerate(at_rules_data.keys()):
        out.write(f"""
    {title_casify(at_rule_name)} = {index},""")

    out.write("""
}

pub(crate) fn at_rule_id_from_u8(at_rule_id: u8) -> Option<AtRuleId> {
    match at_rule_id {""")

    for index, at_rule_name in enumerate(at_rules_data.keys()):
        out.write(f"""
        {index} => Some(AtRuleId::{title_casify(at_rule_name)}),""")

    out.write("""
        _ => None,
    }
}

""")


def write_descriptor_id(out: TextIO, all_descriptors: list) -> None:
    out.write("""#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum DescriptorId {""")

    for index, descriptor_name in enumerate(all_descriptors):
        out.write(f"""
    {title_casify(descriptor_name)} = {index},""")

    out.write(f"""
    Custom = {len(all_descriptors)},
}}

pub(crate) fn descriptor_id_from_u8(descriptor_id: u8) -> Option<DescriptorId> {{
    match descriptor_id {{""")

    for index, descriptor_name in enumerate(all_descriptors):
        out.write(f"""
        {index} => Some(DescriptorId::{title_casify(descriptor_name)}),""")

    out.write(f"""
        {len(all_descriptors)} => Some(DescriptorId::Custom),
        _ => None,
    }}
}}

""")


def write_descriptor_value_type(out: TextIO) -> None:
    out.write("""pub(crate) enum DescriptorSyntax {
    Keyword(&'static str),
    Property(PropertyId),
    ValueType(CssDescriptorValueType),
}

""")


def syntax_entry_expression(entry: str) -> str:
    if entry.startswith("<'"):
        return f"DescriptorSyntax::Property(PropertyId::{title_casify(entry[2:-2])})"
    if entry.startswith("<"):
        if entry not in VALUE_TYPE_NAMES:
            print(f"Unrecognized value type: `{entry}`", file=sys.stderr)
            sys.exit(1)
        return f"DescriptorSyntax::ValueType(CssDescriptorValueType::{VALUE_TYPE_NAMES[entry]})"
    if entry == "crop || cross":
        return "DescriptorSyntax::ValueType(CssDescriptorValueType::CropOrCross)"
    return f"DescriptorSyntax::Keyword({rust_string_literal(entry)})"


def write_supported_descriptors(out: TextIO, at_rules_data: dict) -> None:
    out.write("""pub(crate) fn at_rule_supports_descriptor(at_rule_id: AtRuleId, descriptor_id: DescriptorId) -> bool {
    match at_rule_id {""")

    for at_rule_name, at_rule in at_rules_data.items():
        out.write(f"""
        AtRuleId::{title_casify(at_rule_name)} => matches!(descriptor_id""")

        first = True
        for descriptor_name, descriptor in at_rule["descriptors"].items():
            if is_legacy_alias(descriptor):
                continue
            separator = "," if first else " |"
            out.write(f"{separator}\n            DescriptorId::{title_casify(descriptor_name)}")
            first = False

        if "custom-descriptors" in at_rule:
            separator = "," if first else " |"
            out.write(f"{separator}\n            DescriptorId::Custom")

        out.write("""
        ),""")

    out.write("""
    }
}

""")


def write_descriptor_metadata(out: TextIO, at_rules_data: dict) -> None:
    out.write("""pub(crate) fn descriptor_allows_arbitrary_substitution_functions(
    at_rule_id: AtRuleId,
    descriptor_id: DescriptorId,
) -> bool {
    match at_rule_id {""")

    for at_rule_name, at_rule in at_rules_data.items():
        descriptors_allowing_arbitrary = []

        for descriptor_name, descriptor in at_rule["descriptors"].items():
            if is_legacy_alias(descriptor):
                continue
            if descriptor.get("allow-arbitrary-substitution-functions", False):
                descriptors_allowing_arbitrary.append(f"DescriptorId::{title_casify(descriptor_name)}")

        if "custom-descriptors" in at_rule:
            if at_rule["custom-descriptors"].get("allow-arbitrary-substitution-functions", False):
                descriptors_allowing_arbitrary.append("DescriptorId::Custom")

        if descriptors_allowing_arbitrary:
            out.write(f"""
        AtRuleId::{title_casify(at_rule_name)} => matches!(descriptor_id""")
            first = True
            for descriptor in descriptors_allowing_arbitrary:
                separator = "," if first else " |"
                out.write(f"{separator}\n            {descriptor}")
                first = False
            out.write("""
        ),""")
        else:
            out.write(f"""
        AtRuleId::{title_casify(at_rule_name)} => false,""")

    out.write("""
    }
}

pub(crate) fn for_each_descriptor_syntax(
    at_rule_id: AtRuleId,
    descriptor_id: DescriptorId,
    mut callback: impl FnMut(DescriptorSyntax),
) -> bool {
    match at_rule_id {""")

    for at_rule_name, at_rule in at_rules_data.items():
        out.write(f"""
        AtRuleId::{title_casify(at_rule_name)} => match descriptor_id {{""")

        for descriptor_name, descriptor in at_rule["descriptors"].items():
            if is_legacy_alias(descriptor):
                continue
            out.write(f"""
            DescriptorId::{title_casify(descriptor_name)} => {{""")
            for entry in descriptor["syntax"]:
                out.write(f"""
                callback({syntax_entry_expression(entry)});""")
            out.write("""
                true
            },""")

        if "custom-descriptors" in at_rule:
            custom_descriptors = at_rule["custom-descriptors"]
            out.write("""
            DescriptorId::Custom => {""")
            for entry in custom_descriptors["syntax"]:
                out.write(f"""
                callback({syntax_entry_expression(entry)});""")
            out.write("""
                true
            },""")

        out.write("""
            _ => false,
        },""")

    out.write("""
    }
}
""")


def write_rust_file(out: TextIO, at_rules_data: dict, all_descriptors: list) -> None:
    out.write("""/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// Automatically generated by Meta/Generators/generate_libweb_css_descriptors_rust.py.

use crate::css_parser::CssDescriptorValueType;
use crate::generated_properties::PropertyId;

""")
    write_at_rule_id(out, at_rules_data)
    write_descriptor_id(out, all_descriptors)
    write_descriptor_value_type(out)
    write_supported_descriptors(out, at_rules_data)
    write_descriptor_metadata(out, at_rules_data)


def main():
    parser = argparse.ArgumentParser(description="Generate Rust CSS descriptor metadata", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("--json", required=True, help="Path to the JSON file to read from")
    parser.add_argument("--output", required=True, help="Path to the generated Rust file")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as input_file:
        at_rules_data = json.load(input_file)

    with open(args.output, "w", encoding="utf-8") as output_file:
        write_rust_file(output_file, at_rules_data, collect_all_descriptors(at_rules_data))


if __name__ == "__main__":
    main()
