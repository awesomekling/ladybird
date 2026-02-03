/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Forward.h>

namespace JS::IR {

// Base class for optimization passes
class Pass {
public:
    virtual ~Pass() = default;
    virtual bool run(Function&) = 0;
    virtual char const* name() const = 0;
};

}
