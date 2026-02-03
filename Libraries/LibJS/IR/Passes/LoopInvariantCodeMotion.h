/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Loop Invariant Code Motion: Move computations that don't change inside a
// loop to the preheader block, reducing redundant computation
class LoopInvariantCodeMotion final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "LoopInvariantCodeMotion"; }
};

}
