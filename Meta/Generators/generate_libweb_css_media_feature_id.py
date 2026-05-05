#!/usr/bin/env python3

# Copyright (c) 2022-2026, Sam Atkins <sam@ladybird.org>
# Copyright (c) 2026-present, the Ladybird developers.
#
# SPDX-License-Identifier: BSD-2-Clause

import argparse
import json
import sys

from pathlib import Path
from typing import Any
from typing import TextIO

sys.path.append(str(Path(__file__).resolve().parent.parent))

from Utils.utils import title_casify
from Utils.utils import underlying_type_for_enum

VALUE_TYPE_NAMES = {
    "<mq-boolean>": "Boolean",
    "<integer>": "Integer",
    "<length>": "Length",
    "<ratio>": "Ratio",
    "<resolution>": "Resolution",
}


def json_is_valid(media_feature_data: dict[str, Any], json_path: str) -> bool:
    is_valid = True
    most_recent_media_feature_name = ""

    for media_feature_name, media_feature_definition in media_feature_data.items():
        if media_feature_name.lower() < most_recent_media_feature_name.lower():
            print(
                f"{json_path}: Media feature `{media_feature_name}` is in the wrong position. Please keep this list alphabetical!",
                file=sys.stderr,
            )
            is_valid = False

        most_recent_media_feature_name = media_feature_name

        if not isinstance(media_feature_definition, dict):
            print(f"{json_path}: Media feature `{media_feature_name}` is not an object", file=sys.stderr)
            is_valid = False
            continue

        if "type" not in media_feature_definition:
            print(f"{json_path}: Media feature `{media_feature_name}` is missing a type", file=sys.stderr)
            is_valid = False
        elif media_feature_definition["type"] not in ("discrete", "range"):
            print(
                f"{json_path}: Media feature `{media_feature_name}` has an unsupported type `{media_feature_definition['type']}`",
                file=sys.stderr,
            )
            is_valid = False

        if "values" not in media_feature_definition:
            print(f"{json_path}: Media feature `{media_feature_name}` is missing values", file=sys.stderr)
            is_valid = False
        elif not isinstance(media_feature_definition["values"], list):
            print(f"{json_path}: Media feature `{media_feature_name}` has values that are not a list", file=sys.stderr)
            is_valid = False
        else:
            for value in media_feature_definition["values"]:
                if not isinstance(value, str):
                    print(f"{json_path}: Media feature `{media_feature_name}` has a non-string value", file=sys.stderr)
                    is_valid = False
                    continue

                if value.startswith("<") and value not in VALUE_TYPE_NAMES:
                    print(
                        f"{json_path}: Media feature `{media_feature_name}` has an unsupported value type `{value}`",
                        file=sys.stderr,
                    )
                    is_valid = False

        if "false-keywords" in media_feature_definition:
            false_keywords = media_feature_definition["false-keywords"]
            if not isinstance(false_keywords, list):
                print(
                    f"{json_path}: Media feature `{media_feature_name}` has false-keywords that are not a list",
                    file=sys.stderr,
                )
                is_valid = False
            else:
                for false_keyword in false_keywords:
                    if not isinstance(false_keyword, str):
                        print(
                            f"{json_path}: Media feature `{media_feature_name}` has a non-string false-keyword",
                            file=sys.stderr,
                        )
                        is_valid = False

        for field_name in media_feature_definition:
            if field_name in ("type", "values", "false-keywords", "FIXME"):
                continue

            print(
                f"{json_path}: Media feature `{media_feature_name}` has an unexpected field `{field_name}`",
                file=sys.stderr,
            )
            is_valid = False

    return is_valid


def write_header_file(out: TextIO, media_feature_data: dict) -> None:
    underlying_type = underlying_type_for_enum(len(media_feature_data))
    out.write(f"""#pragma once

#include <AK/StringView.h>
#include <AK/Traits.h>
#include <AK/Types.h>
#include <LibWeb/CSS/Keyword.h>

namespace Web::CSS {{

enum class MediaFeatureValueType {{
    Boolean,
    Integer,
    Length,
    Ratio,
    Resolution,
}};

enum class MediaFeatureID : {underlying_type} {{""")

    for name in media_feature_data:
        out.write(f"""
    {title_casify(name)},""")

    out.write("""
};

Optional<MediaFeatureID> media_feature_id_from_string(StringView);
Optional<MediaFeatureID> media_feature_id_from_u8(u8);
StringView string_from_media_feature_id(MediaFeatureID);

bool media_feature_type_is_range(MediaFeatureID);
bool media_feature_accepts_type(MediaFeatureID, MediaFeatureValueType);
bool media_feature_accepts_keyword(MediaFeatureID, Keyword);

bool media_feature_keyword_is_falsey(MediaFeatureID, Keyword);

}
""")


def write_implementation_file(out: TextIO, media_feature_data: dict) -> None:
    out.write("""
#include <LibWeb/CSS/MediaFeatureID.h>
#include <LibWeb/Infra/Strings.h>

namespace Web::CSS {

Optional<MediaFeatureID> media_feature_id_from_string(StringView string)
{""")

    for name in media_feature_data:
        out.write(f"""
    if (string.equals_ignoring_ascii_case("{name}"sv))
        return MediaFeatureID::{title_casify(name)};
""")

    out.write("""
    return {};
}

Optional<MediaFeatureID> media_feature_id_from_u8(u8 value)
{
    switch (value) {""")

    for index, name in enumerate(media_feature_data):
        out.write(f"""
    case {index}:
        return MediaFeatureID::{title_casify(name)};""")

    out.write("""
    default:
        return {};
    }
}

StringView string_from_media_feature_id(MediaFeatureID media_feature_id)
{
    switch (media_feature_id) {""")

    for name in media_feature_data:
        out.write(f"""
    case MediaFeatureID::{title_casify(name)}:
        return "{name}"sv;""")

    out.write("""
    }
    VERIFY_NOT_REACHED();
}

bool media_feature_type_is_range(MediaFeatureID media_feature_id)
{
    switch (media_feature_id) {""")

    for name, feature in media_feature_data.items():
        is_range = "true" if feature["type"] == "range" else "false"
        out.write(f"""
    case MediaFeatureID::{title_casify(name)}:
        return {is_range};""")

    out.write("""
    }
    VERIFY_NOT_REACHED();
}

bool media_feature_accepts_type(MediaFeatureID media_feature_id, MediaFeatureValueType value_type)
{
    switch (media_feature_id) {""")

    for name, feature in media_feature_data.items():
        out.write(f"""
    case MediaFeatureID::{title_casify(name)}:""")

        have_output_value_type_switch = False
        if "values" in feature:
            for type_name in feature["values"]:
                # Skip keywords.
                if not type_name.startswith("<"):
                    continue
                if type_name not in VALUE_TYPE_NAMES:
                    print(f"Unrecognized media-feature value type: `{type_name}`", file=sys.stderr)
                    sys.exit(1)
                if not have_output_value_type_switch:
                    out.write("""
        switch (value_type) {""")
                    have_output_value_type_switch = True
                value_type = VALUE_TYPE_NAMES[type_name]
                out.write(f"""
        case MediaFeatureValueType::{value_type}:
            return true;""")

        if have_output_value_type_switch:
            out.write("""
        default:
            return false;
        }""")
        else:
            out.write("""
        return false;""")

    out.write("""
    }
    VERIFY_NOT_REACHED();
}

bool media_feature_accepts_keyword(MediaFeatureID media_feature_id, Keyword keyword)
{
    switch (media_feature_id) {""")

    for name, feature in media_feature_data.items():
        out.write(f"""
    case MediaFeatureID::{title_casify(name)}:""")

        have_output_keyword_switch = False
        if "values" in feature:
            for keyword_name in feature["values"]:
                # Skip types.
                if keyword_name.startswith("<"):
                    continue
                if not have_output_keyword_switch:
                    out.write("""
        switch (keyword) {""")
                    have_output_keyword_switch = True
                out.write(f"""
        case Keyword::{title_casify(keyword_name)}:
            return true;""")

        if have_output_keyword_switch:
            out.write("""
        default:
            return false;
        }""")
        else:
            out.write("""
        return false;""")

    out.write("""
    }
    VERIFY_NOT_REACHED();
}

bool media_feature_keyword_is_falsey(MediaFeatureID media_feature_id, Keyword keyword)
{
    switch (media_feature_id) {""")

    for name, feature in media_feature_data.items():
        false_keywords = feature.get("false-keywords")
        if not false_keywords:
            continue
        out.write(f"""
    case MediaFeatureID::{title_casify(name)}:
        switch (keyword) {{""")
        for false_keyword in false_keywords:
            out.write(f"""
        case Keyword::{title_casify(false_keyword)}:""")
        out.write("""
            return true;
        default:
            return false;
        }""")

    out.write("""
    default:
        return false;
    }
}

}
""")


def main():
    parser = argparse.ArgumentParser(description="Generate CSS MediaFeatureID", add_help=False)
    parser.add_argument("--help", action="help", help="Show this help message and exit")
    parser.add_argument("-h", "--header", required=True, help="Path to the MediaFeatureID header file to generate")
    parser.add_argument(
        "-c", "--implementation", required=True, help="Path to the MediaFeatureID implementation file to generate"
    )
    parser.add_argument("-j", "--json", required=True, help="Path to the JSON file to read from")
    args = parser.parse_args()

    with open(args.json, "r", encoding="utf-8") as input_file:
        media_feature_data = json.load(input_file)

    if not isinstance(media_feature_data, dict):
        raise RuntimeError(f"{args.json}: expected a JSON object")

    if not json_is_valid(media_feature_data, args.json):
        sys.exit(1)

    with open(args.header, "w", encoding="utf-8") as output_file:
        write_header_file(output_file, media_feature_data)

    with open(args.implementation, "w", encoding="utf-8") as output_file:
        write_implementation_file(output_file, media_feature_data)


if __name__ == "__main__":
    main()
