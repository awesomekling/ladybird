/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Partial Redundancy Elimination: Extends GVN by finding expressions that are
// redundant on some (but not all) paths through a merge point, inserting the
// missing computation on the other paths, and replacing the original with a phi.
class PartialRedundancyElimination final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "PartialRedundancyElimination"; }
};

}
