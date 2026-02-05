/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/ConstantBranchFolding.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

PreservedAnalyses ConstantBranchFolding::run(Function& function, PassManager&)
{
    bool changed = false;

    for (auto& block : function.basic_blocks()) {
        auto* term = block->terminator();
        if (!term || term->opcode() != Opcode::Branch)
            continue;

        // If both targets are the same, convert to unconditional jump
        if (term->true_target() == term->false_target() && term->true_target() != nullptr) {
            CFG::replace_branch_with_jump(*block, *term->true_target(), nullptr);
            changed = true;
            continue;
        }

        // Branch has condition as first operand
        if (term->operands().is_empty())
            continue;

        auto* condition = term->operands()[0];
        auto truthiness = condition->constant_truthiness();
        if (!truthiness.has_value())
            continue;

        bool take_true_branch = *truthiness;

        auto* target = take_true_branch ? term->true_target() : term->false_target();
        auto* not_taken = take_true_branch ? term->false_target() : term->true_target();

        // Replace Branch with Jump
        CFG::replace_branch_with_jump(*block, *target, not_taken);
        changed = true;
    }

    return changed ? PreservedAnalyses::none() : PreservedAnalyses::all();
}

}
