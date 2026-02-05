/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/IR/Passes/PassManager.h>

namespace JS::IR {

// Base class for optimization passes
class JS_API Pass {
public:
    virtual ~Pass() = default;
    virtual PreservedAnalyses run(Function&, PassManager&) = 0;
    virtual char const* name() const = 0;
};

}
