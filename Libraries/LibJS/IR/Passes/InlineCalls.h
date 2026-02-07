/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Guarded monomorphic inlining: for call sites with monomorphic profile data,
// emit a guard comparing the callee to the expected target, branch to an
// inlined fast path or a slow-path fallback call, and merge results with a phi.
class InlineCalls final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "InlineCalls"; }
};

}
