/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// TypeRefinement: Flow-sensitive type narrowing pass.
// Propagates type facts from branch guards (typeof checks, nullish guards,
// equality with null/undefined) into dominated code, enabling typeof folding,
// guard elimination, and comparison folding.
class TypeRefinement final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "TypeRefinement"; }
};

}
