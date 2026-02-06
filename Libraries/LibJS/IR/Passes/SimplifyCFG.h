/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// SimplifyCFG: Canonical CFG simplification pass
// Combines empty block elimination, block merging, constant branch folding,
// dead block elimination, and jump threading into a single fixpoint pass.
class SimplifyCFG final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "SimplifyCFG"; }
};

}
