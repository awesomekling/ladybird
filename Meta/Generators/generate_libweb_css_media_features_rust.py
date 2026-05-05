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


def write_rust_file(out: TextIO, media_feature_data: dict) -> None:
    out.write("""/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum MediaFeatureId {""")

    for name in media_feature_data:
        out.write(f"""
    {title_casify(name)},""")

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
""")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Rust CSS media feature metadata", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("-j", "--json", required=True, help="Path to the JSON file to read from")
    parser.add_argument("-o", "--output", required=True, help="Path to the Rust file to generate")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as input_file:
        media_feature_data = json.load(input_file)

    with open(args.output, "w", encoding="utf-8") as output_file:
        write_rust_file(output_file, media_feature_data)


if __name__ == "__main__":
    main()
