/*
 * Copyright (c) 2024, Kostya Farber <kostya.farber@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/Layout/LegendBox.h>

namespace Web::Layout {

GC_DEFINE_ALLOCATOR(LegendBox);

LegendBox::LegendBox(DOM::Document& document, DOM::Element& element)
    : BlockContainer(document, element)
{
}

LegendBox::~LegendBox() = default;

}
