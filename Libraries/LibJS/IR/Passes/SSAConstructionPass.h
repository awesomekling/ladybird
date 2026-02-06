/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Converts raw CFG IR into SSA form by placing phi nodes and renaming
// operands using the dominator tree. Consumes and clears the
// SsaConstructionData that the lifter attached to the function.
class SSAConstructionPass final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "SSAConstruction"; }
};

}
