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
    "<compound-selector>": "CompoundSelector",
    "<ident>+": "IdentList",
    "<pt-name-selector>": "PTNameSelector",
}


def is_alias(pseudo_element: dict) -> bool:
    return "alias-for" in pseudo_element


def rust_string_literal(string: str) -> str:
    return json.dumps(string)


def write_rust_file(out: TextIO, pseudo_elements_data: dict) -> None:
    out.write("""/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum PseudoElementId {""")

    pseudo_element_names = [name for name, value in pseudo_elements_data.items() if not is_alias(value)]

    for index, name in enumerate(pseudo_element_names):
        out.write(f"""
    {title_casify(name)} = {index},""")

    out.write(f"""
    KnownPseudoElementCount = {len(pseudo_element_names)},
    UnknownWebKit = {len(pseudo_element_names) + 1},
}}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PseudoElementParameterType {{
    None,
    CompoundSelector,
    IdentList,
    PTNameSelector,
}}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct PseudoElementMetadata {{
    pub(crate) parameter_type: PseudoElementParameterType,
    pub(crate) is_valid_as_function: bool,
    pub(crate) is_valid_as_identifier: bool,
}}

pub(crate) fn pseudo_element_id_from_string(string: &str) -> Option<PseudoElementId> {{""")

    for name, value in pseudo_elements_data.items():
        if is_alias(value):
            continue
        out.write(f"""
    if string.eq_ignore_ascii_case({rust_string_literal(name)}) {{
        return Some(PseudoElementId::{title_casify(name)});
    }}""")

    out.write("""
    None
}

pub(crate) fn aliased_pseudo_element_id_from_string(string: &str) -> Option<PseudoElementId> {""")

    for name, value in pseudo_elements_data.items():
        alias_for = value.get("alias-for")
        if alias_for is None:
            continue
        out.write(f"""
    if string.eq_ignore_ascii_case({rust_string_literal(name)}) {{
        return Some(PseudoElementId::{title_casify(alias_for)});
    }}""")

    out.write("""
    None
}

#[allow(dead_code)]
pub(crate) fn pseudo_element_name(pseudo_element_id: PseudoElementId) -> &'static str {
    match pseudo_element_id {""")

    for name in pseudo_element_names:
        out.write(f"""
        PseudoElementId::{title_casify(name)} => {rust_string_literal(name)},""")

    out.write("""
        PseudoElementId::KnownPseudoElementCount | PseudoElementId::UnknownWebKit => {
            unreachable!()
        }
    }
}
""")

    has_allowed_pseudo_elements = [
        name
        for name, value in pseudo_elements_data.items()
        if not is_alias(value) and value.get("is-allowed-in-has", False)
    ]

    if not has_allowed_pseudo_elements:
        out.write("""
pub(crate) fn is_has_allowed_pseudo_element(_pseudo_element_id: PseudoElementId) -> bool {
    false
}

""")
    else:
        out.write("""
pub(crate) fn is_has_allowed_pseudo_element(pseudo_element_id: PseudoElementId) -> bool {
    matches!(pseudo_element_id""")
        for index, name in enumerate(has_allowed_pseudo_elements):
            separator = "," if index == 0 else " |"
            out.write(f"{separator}\n        PseudoElementId::{title_casify(name)}")
        out.write("""
    )
}

""")

    out.write("""
pub(crate) fn pseudo_element_metadata(pseudo_element_id: PseudoElementId) -> PseudoElementMetadata {
    match pseudo_element_id {""")

    for name, value in pseudo_elements_data.items():
        if is_alias(value):
            continue

        pseudo_element_type = value.get("type")
        if pseudo_element_type == "function":
            is_valid_as_function = True
            is_valid_as_identifier = False
        elif pseudo_element_type == "both":
            is_valid_as_function = True
            is_valid_as_identifier = True
        else:
            is_valid_as_function = False
            is_valid_as_identifier = True

        parameter_type = "None"
        if is_valid_as_function:
            function_syntax = value["function-syntax"]
            if function_syntax not in PARAMETER_TYPES:
                print(f"Unrecognized pseudo-element parameter type: `{function_syntax}`", file=sys.stderr)
                sys.exit(1)
            parameter_type = PARAMETER_TYPES[function_syntax]
        elif "function-syntax" in value:
            print(f"Pseudo-element `::{name}` has `function-syntax` but is not a function type.", file=sys.stderr)
            sys.exit(1)

        out.write(f"""
        PseudoElementId::{title_casify(name)} => PseudoElementMetadata {{
            parameter_type: PseudoElementParameterType::{parameter_type},
            is_valid_as_function: {str(is_valid_as_function).lower()},
            is_valid_as_identifier: {str(is_valid_as_identifier).lower()},
        }},""")

    out.write("""
        PseudoElementId::UnknownWebKit => PseudoElementMetadata {
            parameter_type: PseudoElementParameterType::None,
            is_valid_as_function: false,
            is_valid_as_identifier: true,
        },
        PseudoElementId::KnownPseudoElementCount => unreachable!(),
    }
}
""")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Rust CSS pseudo-element metadata", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("-j", "--json", required=True, help="Path to the JSON file to read from")
    parser.add_argument("-o", "--output", required=True, help="Path to the Rust file to generate")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as input_file:
        pseudo_elements_data = json.load(input_file)

    if not isinstance(pseudo_elements_data, dict):
        raise RuntimeError(f"{args.json}: expected a JSON object")

    with open(args.output, "w", encoding="utf-8") as output_file:
        write_rust_file(output_file, pseudo_elements_data)


if __name__ == "__main__":
    main()
