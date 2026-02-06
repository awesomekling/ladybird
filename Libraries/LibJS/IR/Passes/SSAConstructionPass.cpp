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

    auto* data = function.ssa_construction_data();
    VERIFY(data);

    SSAConstruction ssa(function, dominator_tree, *function.source_executable(),
        data->written_operands, data->block_actual_definitions,
        data->block_definitions, data->value_to_operand_raw);
    ssa.run();

    function.clear_ssa_construction_data();
    function.set_stage(IRStage::SSA);

    return PreservedAnalyses::none();
}

}
