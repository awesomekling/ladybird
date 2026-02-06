/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/Export.h>
#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Split Critical Edges: Insert new blocks on edges A->B where A has multiple
// successors and B has multiple predecessors. This gives phi moves a clean
// place to land during SSA deconstruction.
class JS_API SplitCriticalEdges final : public Pass {
public:
    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "SplitCriticalEdges"; }
};

}
