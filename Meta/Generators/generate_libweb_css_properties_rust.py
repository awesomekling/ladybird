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

from Generators.generate_libweb_css_property_id import populate_all_property_longhands
from Generators.generate_libweb_css_property_id import replace_logical_aliases
from Generators.generate_libweb_css_property_id import verify_alphabetical
from Utils.utils import title_casify


def rust_string_literal(string: str) -> str:
    return json.dumps(string)


def property_title(name: str) -> str:
    return title_casify(name)


def value_type_title(name: str) -> str:
    return title_casify(name)


def split_valid_type(valid_type: str) -> tuple[str, str | None]:
    parts = valid_type.split(" ", 1)
    if len(parts) == 1:
        return parts[0], None
    return parts[0], parts[1]


def keyword_name(keyword: str) -> str:
    return keyword.split("=", 1)[0]


def numeric_range_from_parameters(parameters: str | None) -> tuple[str, str] | None:
    if parameters is None:
        return None
    if not (parameters.startswith("[") and parameters.endswith("]")):
        return None
    limits = parameters[1:-1].split(",")
    if len(limits) != 2:
        raise ValueError(f"Bad numeric range: {parameters}")

    def limit_to_rust(limit: str) -> str:
        if limit in ("-∞", "∞"):
            return "None"
        return f"Some({limit}.0)"

    return limit_to_rust(limits[0]), limit_to_rust(limits[1])


def valid_types_for_properties(properties: dict, enum_names: set[str]) -> list[str]:
    value_types = set()
    for property_data in properties.values():
        if "legacy-alias-for" in property_data:
            continue
        for valid_type in property_data.get("valid-types", []):
            type_name, _ = split_valid_type(valid_type)
            if type_name in enum_names:
                continue
            value_types.add(type_name)
    return sorted(value_types)


def ordered_property_names(properties: dict) -> list[str]:
    shorthand_property_ids = []
    inherited_longhand_property_ids = []
    noninherited_longhand_property_ids = []

    for name, value in properties.items():
        if "legacy-alias-for" in value:
            continue
        inherited = value.get("inherited")
        if "longhands" in value:
            shorthand_property_ids.append(name)
        elif inherited:
            inherited_longhand_property_ids.append(name)
        else:
            noninherited_longhand_property_ids.append(name)

    return shorthand_property_ids + inherited_longhand_property_ids + noninherited_longhand_property_ids


def write_property_id(out: TextIO, properties: dict, property_names: list[str]) -> None:
    out.write("""#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub(crate) enum PropertyId {
    Custom = 0,""")

    for index, name in enumerate(property_names, start=1):
        out.write(f"""
    {property_title(name)} = {index},""")

    out.write("""
}

#[allow(dead_code)]
pub(crate) fn property_id_from_u16(property_id: u16) -> Option<PropertyId> {
    match property_id {
        0 => Some(PropertyId::Custom),""")

    for index, name in enumerate(property_names, start=1):
        out.write(f"""
        {index} => Some(PropertyId::{property_title(name)}),""")

    out.write("""
        _ => None,
    }
}

#[allow(dead_code)]
pub(crate) fn property_id_from_string(string: &str) -> Option<PropertyId> {
    if string.starts_with("--") {
        return Some(PropertyId::Custom);
    }""")

    for name, value in properties.items():
        target_name = value.get("legacy-alias-for", name)
        out.write(f"""
    if string.eq_ignore_ascii_case({rust_string_literal(name)}) {{
        return Some(PropertyId::{property_title(target_name)});
    }}""")

    out.write("""
    None
}

#[allow(dead_code)]
pub(crate) fn property_name(property_id: PropertyId) -> &'static str {
    match property_id {
        PropertyId::Custom => "--*",""")

    for name in property_names:
        out.write(f"""
        PropertyId::{property_title(name)} => {rust_string_literal(name)},""")

    out.write("""
    }
}

""")


def write_property_value_type(out: TextIO, value_types: list[str]) -> None:
    out.write("""#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PropertyValueType {""")

    for index, name in enumerate(value_types):
        out.write(f"""
    {value_type_title(name)} = {index},""")

    out.write("""
}

#[allow(dead_code)]
pub(crate) fn property_value_type_from_u8(value_type: u8) -> Option<PropertyValueType> {
    match value_type {""")

    for index, name in enumerate(value_types):
        out.write(f"""
        {index} => Some(PropertyValueType::{value_type_title(name)}),""")

    out.write("""
        _ => None,
    }
}

#[allow(dead_code)]
pub(crate) fn property_value_type_from_css_value_type_name(value_type: &str) -> Option<PropertyValueType> {""")

    for name in value_types:
        title_name = value_type_title(name)
        out.write(f"""
    if value_type == {rust_string_literal(title_name)} {{
        return Some(PropertyValueType::{title_name});
    }}""")
        if name == "opentype-tag":
            out.write(f"""
    if value_type == "OpenTypeTag" {{
        return Some(PropertyValueType::{title_name});
    }}""")

    out.write("""
    None
}

""")


def write_is_shorthand(out: TextIO, properties: dict) -> None:
    out.write("""#[allow(dead_code)]
pub(crate) fn property_is_shorthand(property_id: PropertyId) -> bool {
    matches!(property_id""")
    first = True
    for name, value in properties.items():
        if "legacy-alias-for" in value or "longhands" not in value:
            continue
        separator = "," if first else " |"
        out.write(f"{separator}\n        PropertyId::{property_title(name)}")
        first = False
    out.write("""
    )
}

#[allow(dead_code)]
pub(crate) fn longhands_for_shorthand(property_id: PropertyId) -> &'static [PropertyId] {
    match property_id {""")

    for name, value in properties.items():
        if "legacy-alias-for" in value or "longhands" not in value:
            continue
        out.write(f"""
        PropertyId::{property_title(name)} => &[""")
        for longhand_name in value["longhands"]:
            out.write(f"""
            PropertyId::{property_title(longhand_name)},""")
        out.write("""
        ],""")

    out.write("""
        _ => &[],
    }
}

""")


def write_accepts_value_type(out: TextIO, properties: dict, enum_names: set[str]) -> None:
    out.write("""#[allow(dead_code)]
pub(crate) fn property_accepts_value_type(property_id: PropertyId, value_type: PropertyValueType) -> bool {
    match property_id {""")

    for name, value in properties.items():
        if "legacy-alias-for" in value:
            continue
        accepted_types = []
        for valid_type in value.get("valid-types", []):
            type_name, _ = split_valid_type(valid_type)
            if type_name in enum_names:
                continue
            accepted_types.append(type_name)
        if not accepted_types:
            continue
        out.write(f"""
        PropertyId::{property_title(name)} => matches!(value_type""")
        for index, type_name in enumerate(accepted_types):
            separator = "," if index == 0 else " |"
            out.write(f"{separator} PropertyValueType::{value_type_title(type_name)}")
        out.write("),")

    out.write("""
        _ => false,
    }
}

""")


def write_accepts_keyword(out: TextIO, properties: dict, enums: dict) -> None:
    out.write("""#[allow(dead_code)]
pub(crate) fn property_accepts_keyword(property_id: PropertyId, keyword: &str) -> bool {
    match property_id {""")

    for name, value in properties.items():
        if "legacy-alias-for" in value:
            continue
        keywords = []
        for keyword in value.get("valid-identifiers", []):
            keywords.append(keyword.split(">", 1)[0])
        for valid_type in value.get("valid-types", []):
            type_name, _ = split_valid_type(valid_type)
            if type_name not in enums:
                continue
            keywords.extend(keyword_name(keyword) for keyword in enums[type_name])
        if not keywords:
            continue
        out.write(f"""
        PropertyId::{property_title(name)} => """)
        for index, keyword in enumerate(dict.fromkeys(keywords)):
            if index > 0:
                out.write(" || ")
            out.write(f"keyword.eq_ignore_ascii_case({rust_string_literal(keyword)})")
        out.write(",")

    out.write("""
        _ => false,
    }
}

#[allow(dead_code)]
pub(crate) fn resolve_legacy_value_alias(property_id: PropertyId, keyword: &str) -> Option<&'static str> {
    match property_id {""")

    for name, value in properties.items():
        if "legacy-alias-for" in value:
            continue
        aliases = [keyword.split(">", 1) for keyword in value.get("valid-identifiers", []) if ">" in keyword]
        if not aliases:
            continue
        out.write(f"""
        PropertyId::{property_title(name)} => {{""")
        for alias, target in aliases:
            out.write(f"""
            if keyword.eq_ignore_ascii_case({rust_string_literal(alias)}) {{
                return Some({rust_string_literal(target)});
            }}""")
        out.write("""
            None
        },""")

    out.write("""
        _ => None,
    }
}

""")


def write_ranges(out: TextIO, properties: dict, enum_names: set[str]) -> None:
    out.write("""#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PropertyNumericRange {
    pub(crate) minimum: Option<f64>,
    pub(crate) maximum: Option<f64>,
}

#[allow(dead_code)]
pub(crate) fn property_accepted_range_by_value_type(
    property_id: PropertyId,
    value_type: PropertyValueType,
) -> Option<PropertyNumericRange> {
    match property_id {""")

    for name, value in properties.items():
        if "legacy-alias-for" in value:
            continue
        ranges = []
        for valid_type in value.get("valid-types", []):
            type_name, parameters = split_valid_type(valid_type)
            if type_name in enum_names:
                continue
            numeric_range = numeric_range_from_parameters(parameters)
            if numeric_range is None:
                continue
            ranges.append((type_name, numeric_range))
        if not ranges:
            continue
        out.write(f"""
        PropertyId::{property_title(name)} => match value_type {{""")
        for type_name, (minimum, maximum) in ranges:
            out.write(f"""
            PropertyValueType::{value_type_title(type_name)} => Some(PropertyNumericRange {{ minimum: {minimum}, maximum: {maximum} }}),""")
        out.write("""
            _ => None,
        },""")

    out.write("""
        _ => None,
    }
}

""")


def write_percentages_and_custom_ident(out: TextIO, properties: dict) -> None:
    out.write("""#[allow(dead_code)]
pub(crate) fn property_resolves_percentages_relative_to(property_id: PropertyId) -> Option<PropertyValueType> {
    match property_id {""")

    for name, value in properties.items():
        resolved_type = value.get("percentages-resolve-to")
        if "legacy-alias-for" in value or resolved_type is None:
            continue
        out.write(f"""
        PropertyId::{property_title(name)} => Some(PropertyValueType::{value_type_title(resolved_type)}),""")

    out.write("""
        _ => None,
    }
}

#[allow(dead_code)]
pub(crate) fn property_custom_ident_blacklist(property_id: PropertyId) -> &'static [&'static str] {
    match property_id {""")

    for name, value in properties.items():
        if "legacy-alias-for" in value:
            continue
        blacklisted_keywords = []
        for valid_type in value.get("valid-types", []):
            type_name, parameters = split_valid_type(valid_type)
            if type_name != "custom-ident" or parameters is None:
                continue
            if not (parameters.startswith("![") and parameters.endswith("]")):
                raise ValueError(f"Bad custom-ident parameters: {parameters}")
            blacklisted_keywords.extend(parameters[2:-1].split(","))
        if not blacklisted_keywords:
            continue
        out.write(f"""
        PropertyId::{property_title(name)} => &[""")
        for keyword in blacklisted_keywords:
            out.write(f"{rust_string_literal(keyword)}, ")
        out.write("],")

    out.write("""
        _ => &[],
    }
}
""")


def write_rust_file(out: TextIO, properties: dict, enums: dict) -> None:
    enum_names = set(enums.keys())
    property_names = ordered_property_names(properties)
    value_types = valid_types_for_properties(properties, enum_names)

    out.write("""/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

""")
    write_property_id(out, properties, property_names)
    write_property_value_type(out, value_types)
    write_is_shorthand(out, properties)
    write_accepts_value_type(out, properties, enum_names)
    write_accepts_keyword(out, properties, enums)
    write_ranges(out, properties, enum_names)
    write_percentages_and_custom_ident(out, properties)


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Rust CSS property metadata", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("-j", "--properties-json", required=True, help="Path to the properties JSON file to read from")
    parser.add_argument("-e", "--enums-json", required=True, help="Path to the enums JSON file to read from")
    parser.add_argument(
        "-g", "--groups-json", required=True, help="Path to the logical property groups JSON file to read from"
    )
    parser.add_argument("-o", "--output", required=True, help="Path to the Rust file to generate")
    args = parser.parse_args()

    with open(args.properties_json, "r", encoding="utf-8") as input_file:
        properties = json.load(input_file)
    with open(args.enums_json, "r", encoding="utf-8") as input_file:
        enums = json.load(input_file)
    with open(args.groups_json, "r", encoding="utf-8") as input_file:
        logical_property_groups = json.load(input_file)

    verify_alphabetical(properties, args.properties_json)
    verify_alphabetical(enums, args.enums_json)
    verify_alphabetical(logical_property_groups, args.groups_json)

    replace_logical_aliases(properties, logical_property_groups)
    populate_all_property_longhands(properties)

    with open(args.output, "w", encoding="utf-8") as output_file:
        write_rust_file(output_file, properties, enums)


if __name__ == "__main__":
    main()
