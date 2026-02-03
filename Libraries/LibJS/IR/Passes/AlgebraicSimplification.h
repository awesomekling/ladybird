/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Algebraic Simplification: Simplify expressions using algebraic identities
// Examples: x + 0 → x, x * 1 → x, x * 0 → 0, x - x → 0
class AlgebraicSimplification final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "AlgebraicSimplification"; }
};

}
