/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// InstCombine: Canonical instruction combining pass
// Combines constant folding, algebraic simplification, and instruction
// combining into a single forward scan over each block.
class InstCombine final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "InstCombine"; }
};

}
