/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibJS/IR/Function.h>
#include <LibJS/IR/Passes/Pass.h>

namespace JS::IR {

// Converts raw CFG IR into SSA form by placing phi nodes and renaming
// operands using the dominator tree. Consumes the SsaConstructionData
// that the lifter produced.
class SSAConstructionPass final : public Pass {
public:
    explicit SSAConstructionPass(SsaConstructionData data)
        : m_ssa_construction_data(move(data))
    {
    }

    virtual PreservedAnalyses run(Function&, PassManager&) override;
    virtual char const* name() const override { return "SSAConstruction"; }

private:
    SsaConstructionData m_ssa_construction_data;
};

}
