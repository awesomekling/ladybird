/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/Function.h>
#include <LibJS/IR/Passes/SSAConstructionPass.h>
#include <LibJS/IR/SSAConstruction.h>

namespace JS::IR {

PreservedAnalyses SSAConstructionPass::run(Function& function, PassManager& pass_manager)
{
    VERIFY(function.stage() == IRStage::RawCFG);

    auto const& dominator_tree = pass_manager.dominator_tree(function);

    SSAConstruction ssa(function, dominator_tree, *function.source_executable(),
        m_ssa_construction_data.written_operands, m_ssa_construction_data.block_actual_definitions,
        m_ssa_construction_data.block_definitions, m_ssa_construction_data.value_to_operand_raw);
    ssa.run();

    function.set_stage(IRStage::SSA);

    return PreservedAnalyses::none();
}

}
