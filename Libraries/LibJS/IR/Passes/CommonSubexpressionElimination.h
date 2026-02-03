/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Common Subexpression Elimination: If we compute the same expression twice,
// reuse the first result instead of recomputing
class CommonSubexpressionElimination final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "CommonSubexpressionElimination"; }
};

}
