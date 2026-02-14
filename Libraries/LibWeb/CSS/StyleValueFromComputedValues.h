/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullRefPtr.h>
#include <LibWeb/CSS/PropertyID.h>

namespace Web::CSS {

class ComputedValues;
class StyleValue;

RefPtr<StyleValue const> style_value_for_property(PropertyID, ComputedValues const&);

}
