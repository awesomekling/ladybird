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

PARAMETER_TYPES = {
    "<an+b>": "AnPlusB",
    "<an+b-of>": "AnPlusBOf",
    "<compound-selector>": "CompoundSelector",
    "<forgiving-selector-list>": "ForgivingSelectorList",
    "<forgiving-relative-selector-list>": "ForgivingRelativeSelectorList",
    "<ident>": "Ident",
    "<language-ranges>": "LanguageRanges",
    "<level>#": "LevelList",
    "<relative-selector-list>": "RelativeSelectorList",
    "<selector-list>": "SelectorList",
}


def rust_string_literal(string: str) -> str:
    return json.dumps(string)


def write_rust_file(out: TextIO, pseudo_classes_data: dict) -> None:
    out.write("""/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PseudoClassId {""")

    pseudo_class_names = [name for name, value in pseudo_classes_data.items() if "legacy-alias-for" not in value]

    for index, name in enumerate(pseudo_class_names):
        out.write(f"""
    {title_casify(name)} = {index},""")

    out.write("""
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PseudoClassParameterType {
    None,
    AnPlusB,
    AnPlusBOf,
    CompoundSelector,
    ForgivingSelectorList,
    ForgivingRelativeSelectorList,
    Ident,
    LanguageRanges,
    LevelList,
    RelativeSelectorList,
    SelectorList,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PseudoClassMetadata {
    pub(crate) parameter_type: PseudoClassParameterType,
    pub(crate) is_valid_as_function: bool,
    pub(crate) is_valid_as_identifier: bool,
}

pub(crate) fn pseudo_class_id_from_string(string: &str) -> Option<PseudoClassId> {""")

    for name, value in pseudo_classes_data.items():
        alias_for = value.get("legacy-alias-for")
        target_name = alias_for if alias_for is not None else name
        out.write(f"""
    if string.eq_ignore_ascii_case({rust_string_literal(name)}) {{
        return Some(PseudoClassId::{title_casify(target_name)});
    }}""")

    out.write("""
    None
}

#[allow(dead_code)]
pub(crate) fn pseudo_class_name(pseudo_class_id: PseudoClassId) -> &'static str {
    match pseudo_class_id {""")

    for name in pseudo_class_names:
        out.write(f"""
        PseudoClassId::{title_casify(name)} => {rust_string_literal(name)},""")

    out.write("""
    }
}

pub(crate) fn pseudo_class_metadata(pseudo_class_id: PseudoClassId) -> PseudoClassMetadata {
    match pseudo_class_id {""")

    for name, value in pseudo_classes_data.items():
        if "legacy-alias-for" in value:
            continue

        argument_string = value["argument"]
        is_valid_as_identifier = argument_string == ""
        is_valid_as_function = argument_string != ""

        if argument_string.endswith("?"):
            is_valid_as_identifier = True
            argument_string = argument_string[:-1]

        parameter_type = "None"
        if is_valid_as_function:
            if argument_string not in PARAMETER_TYPES:
                print(f"Unrecognized pseudo-class argument type: `{argument_string}`", file=sys.stderr)
                sys.exit(1)
            parameter_type = PARAMETER_TYPES[argument_string]

        out.write(f"""
        PseudoClassId::{title_casify(name)} => PseudoClassMetadata {{
            parameter_type: PseudoClassParameterType::{parameter_type},
            is_valid_as_function: {str(is_valid_as_function).lower()},
            is_valid_as_identifier: {str(is_valid_as_identifier).lower()},
        }},""")

    out.write("""
    }
}
""")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Rust CSS pseudo-class metadata", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("-j", "--json", required=True, help="Path to the JSON file to read from")
    parser.add_argument("-o", "--output", required=True, help="Path to the Rust file to generate")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as input_file:
        pseudo_classes_data = json.load(input_file)

    if not isinstance(pseudo_classes_data, dict):
        raise RuntimeError(f"{args.json}: expected a JSON object")

    with open(args.output, "w", encoding="utf-8") as output_file:
        write_rust_file(output_file, pseudo_classes_data)


if __name__ == "__main__":
    main()
