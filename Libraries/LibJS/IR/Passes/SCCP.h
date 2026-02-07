/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Sparse Conditional Constant Propagation: Combines global constant propagation
// with unreachable code elimination in a single lattice-based solve.
class SCCP final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "SCCP"; }
};

}
