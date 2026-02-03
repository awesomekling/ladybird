/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Constant Folding: Evaluate constant expressions at compile time
class ConstantFolding final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "ConstantFolding"; }
};

}
