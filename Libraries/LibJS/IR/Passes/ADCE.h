/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Aggressive Dead Code Elimination: Removes dead instructions AND dead
// control flow using post-dominance frontiers to identify control dependencies.
//
// Unlike simple DCE which only removes unused instructions, ADCE can
// convert dead conditional branches into unconditional jumps when the
// branch condition only feeds dead computations.
class AggressiveDeadCodeElimination final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "AggressiveDeadCodeElimination"; }
};

}
