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

from Generators.generate_libweb_css_media_feature_id import json_is_valid
from Utils.utils import title_casify

VALUE_TYPE_NAMES = {
    "<mq-boolean>": "Boolean",
    "<integer>": "Integer",
    "<length>": "Length",
    "<ratio>": "Ratio",
    "<resolution>": "Resolution",
}


def rust_string_literal(string: str) -> str:
    return json.dumps(string)


def write_rust_file(out: TextIO, media_feature_data: dict) -> None:
    out.write("""/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaFeatureValueType {
    Boolean,
    Integer,
    Length,
    Ratio,
    Resolution,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum MediaFeatureId {""")

    for index, name in enumerate(media_feature_data):
        out.write(f"""
    {title_casify(name)} = {index},""")

    out.write("""
}

pub(crate) fn media_feature_id_from_string(string: &str) -> Option<MediaFeatureId> {
    if string.eq_ignore_ascii_case("") {
        return None;
    }""")

    for name in media_feature_data:
        out.write(f"""
    if string.eq_ignore_ascii_case("{name}") {{
        return Some(MediaFeatureId::{title_casify(name)});
    }}""")

    out.write("""
    None
}

pub(crate) fn media_feature_type_is_range(media_feature_id: MediaFeatureId) -> bool {
    match media_feature_id {""")

    for name, feature in media_feature_data.items():
        is_range = "true" if feature["type"] == "range" else "false"
        out.write(f"""
        MediaFeatureId::{title_casify(name)} => {is_range},""")

    out.write("""
    }
}

#[allow(dead_code)]
pub(crate) fn media_feature_accepts_type(
    media_feature_id: MediaFeatureId,
    value_type: MediaFeatureValueType,
) -> bool {
    match media_feature_id {""")

    for name, feature in media_feature_data.items():
        value_types = []

        if "values" in feature:
            for type_name in feature["values"]:
                if not type_name.startswith("<"):
                    continue
                value_types.append(VALUE_TYPE_NAMES[type_name])

        if not value_types:
            out.write(f"""
        MediaFeatureId::{title_casify(name)} => false,""")
            continue

        out.write(f"""
        MediaFeatureId::{title_casify(name)} => matches!(""")
        out.write("value_type")
        for index, value_type in enumerate(value_types):
            if index == 0:
                out.write(", ")
            else:
                out.write(" | ")
            out.write(f"MediaFeatureValueType::{value_type}")
        out.write("),")

    out.write("""
    }
}

#[allow(dead_code)]
pub(crate) fn media_feature_accepts_identifier(media_feature_id: MediaFeatureId, identifier: &str) -> bool {
    match media_feature_id {""")

    for name, feature in media_feature_data.items():
        keywords = [value for value in feature.get("values", []) if not value.startswith("<")]
        if not keywords:
            out.write(f"""
        MediaFeatureId::{title_casify(name)} => false,""")
            continue

        out.write(f"""
        MediaFeatureId::{title_casify(name)} => """)
        for index, keyword_name in enumerate(keywords):
            if index > 0:
                out.write(" || ")
            out.write(f"identifier.eq_ignore_ascii_case({rust_string_literal(keyword_name)})")
        out.write(",")

    out.write("""
    }
}
""")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Rust CSS media feature metadata", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("-j", "--json", required=True, help="Path to the JSON file to read from")
    parser.add_argument("-o", "--output", required=True, help="Path to the Rust file to generate")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as input_file:
        media_feature_data = json.load(input_file)

    if not isinstance(media_feature_data, dict):
        raise RuntimeError(f"{args.json}: expected a JSON object")

    if not json_is_valid(media_feature_data, args.json):
        sys.exit(1)

    with open(args.output, "w", encoding="utf-8") as output_file:
        write_rust_file(output_file, media_feature_data)


if __name__ == "__main__":
    main()
