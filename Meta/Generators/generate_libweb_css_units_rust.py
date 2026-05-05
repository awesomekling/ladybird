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

from Generators.generate_libweb_css_units import json_is_valid
from Utils.utils import title_casify


def rust_string_literal(string: str) -> str:
    return json.dumps(string)


def write_rust_file(out: TextIO, dimensions_data: dict) -> None:
    out.write("""/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DimensionType {""")

    for dimension_name in dimensions_data:
        out.write(f"""
    {title_casify(dimension_name)},""")

    out.write("""
}

#[allow(dead_code)]
pub(crate) fn dimension_for_unit(unit_name: &str) -> Option<DimensionType> {""")

    for dimension_name, units in dimensions_data.items():
        out.write("""
    if """)
        for index, unit_name in enumerate(units):
            if index > 0:
                out.write(" || ")
            out.write(f"unit_name.eq_ignore_ascii_case({rust_string_literal(unit_name)})")
        out.write(f""" {{
        return Some(DimensionType::{title_casify(dimension_name)});
    }}""")

    out.write("""

    None
}
""")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Rust CSS unit metadata", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("-j", "--json", required=True, help="Path to the JSON file to read from")
    parser.add_argument("-o", "--output", required=True, help="Path to the Rust file to generate")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as input_file:
        dimensions_data = json.load(input_file)

    if not isinstance(dimensions_data, dict):
        raise RuntimeError(f"{args.json}: expected a JSON object")

    if not json_is_valid(dimensions_data, args.json):
        sys.exit(1)

    with open(args.output, "w", encoding="utf-8") as output_file:
        write_rust_file(output_file, dimensions_data)


if __name__ == "__main__":
    main()
