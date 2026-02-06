/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/Export.h>
#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// SSA Destruction: Replace phi nodes with ParallelCopy instructions.
// For each phi in a block, the pass records (phi.result, incoming_value)
// copies on the appropriate CFG edge, then inserts ParallelCopy
// instructions and removes all phi nodes.
class JS_API SsaDestructionPass final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "SsaDestructionPass"; }
};

}
