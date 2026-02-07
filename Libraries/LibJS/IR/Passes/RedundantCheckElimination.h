/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// RedundantCheckElimination: Removes redundant guard instructions
// (ThrowIfNotObject, ThrowIfNullish, ThrowIfTDZ) when they are
// dominated by an equivalent guard on the same SSA value, and
// eliminates identity conversions (ToNumber, ToInt32, etc.) when
// the operand already has the target type.
class RedundantCheckElimination final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "RedundantCheckElimination"; }
};

}
