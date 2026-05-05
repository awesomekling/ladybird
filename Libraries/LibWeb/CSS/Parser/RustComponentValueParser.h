/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Optional.h>
#include <AK/StringView.h>
#include <AK/Vector.h>
#include <LibWeb/CSS/Parser/ComponentValue.h>
#include <LibWeb/CSS/Parser/Types.h>
#include <LibWeb/Export.h>

namespace Web::CSS::Parser {

class WEB_API RustComponentValueParser {
public:
    static Vector<ComponentValue> parse_a_list_of_component_values(StringView input, StringView encoding);
    static Optional<Declaration> parse_a_declaration(StringView input, StringView encoding);
    static Optional<Rule> parse_a_rule(StringView input, StringView encoding);
};

}
