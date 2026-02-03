/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Empty Block Elimination: Remove blocks that only contain a Jump instruction
// by redirecting predecessors to jump directly to the target
class EmptyBlockElimination final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "EmptyBlockElimination"; }
};

}
