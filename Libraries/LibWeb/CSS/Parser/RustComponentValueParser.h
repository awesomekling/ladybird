/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/StringView.h>
#include <AK/Vector.h>
#include <LibWeb/CSS/Parser/ComponentValue.h>
#include <LibWeb/Export.h>

namespace Web::CSS::Parser {

class WEB_API RustComponentValueParser {
public:
    static Vector<ComponentValue> parse_a_list_of_component_values(StringView input, StringView encoding);
};

}
