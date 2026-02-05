/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Global Value Numbering: Eliminates redundant computations across basic blocks
// by walking the dominator tree with a scoped expression table.
class GlobalValueNumbering final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "GlobalValueNumbering"; }
};

}
