/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// LoopSimplify: Normalize loop structure by ensuring each loop has a
// dedicated preheader (single entry edge) and a single latch (single
// back-edge). This simplifies downstream loop passes like LICM.
class LoopSimplify final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "LoopSimplify"; }
};

}
