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

    for index, name in enumerate(value_type_data):
        out.write(f"""
    {title_casify(value_type_name_without_brackets(name))} = {index},""")

    out.write("""
}

#[allow(dead_code)]
pub(crate) fn value_type_id_from_u8(value_type_id: u8) -> Option<ValueTypeId> {
    match value_type_id {""")

    for index, name in enumerate(value_type_data):
        out.write(f"""
        {index} => Some(ValueTypeId::{title_casify(value_type_name_without_brackets(name))}),""")

    out.write("""
        _ => None,
    }
}

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
