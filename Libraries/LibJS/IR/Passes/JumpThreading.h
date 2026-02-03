/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Jump Threading: When a block branches on a phi node and one of the phi's
// incoming values is a constant, thread the predecessor's edge directly to
// the appropriate target
class JumpThreading final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "JumpThreading"; }
};

}
