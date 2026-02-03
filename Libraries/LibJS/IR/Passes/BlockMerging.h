/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Block Merging: Merge blocks where A jumps unconditionally to B and B has only A as predecessor
class BlockMerging final : public Pass {
public:
    virtual bool run(Function&) override;
    virtual char const* name() const override { return "BlockMerging"; }
};

}
