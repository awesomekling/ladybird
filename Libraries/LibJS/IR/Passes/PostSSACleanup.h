/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// PostSSACleanup: Eliminates trivial blocks left by SSA destruction.
// After SsaDestructionPass, split blocks often contain only
// [ParallelCopy...] + Jump. This pass moves copies into predecessor
// blocks (before their terminators) and redirects edges to skip the
// now-empty split blocks.
class PostSSACleanup final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "PostSSACleanup"; }
};

}
