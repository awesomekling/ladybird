/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibWeb/CSS/PropertyID.h>

namespace Web::CSS {

class MutableComputedValues;
class StyleValue;

// Apply a single resolved StyleValue to the corresponding typed field in ComputedValues.
// Returns true if the property was handled, false if the caller should fall back to
// a full round-trip through ComputedProperties + populate_computed_values().
bool apply_style_value_to_computed_values(MutableComputedValues&, PropertyID, StyleValue const&);

}
