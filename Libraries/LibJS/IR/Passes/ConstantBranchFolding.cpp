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

// Helper to replace the terminator with an unconditional jump
static void replace_branch_with_jump(BasicBlock& block, BasicBlock& target, BasicBlock* not_taken)
{
    // Remove the old branch instruction
    block.instructions().remove(block.instructions().size() - 1);

    // Add a new jump instruction
    auto jump = JumpInstruction::create(target);
    jump->set_parent_block(&block);
    block.instructions().append(move(jump));

    // Remove this block from the not-taken block's predecessors
    if (not_taken)
        CFG::remove_predecessor(*not_taken, block);
}

bool ConstantBranchFolding::run(Function& function)
{
    bool changed = false;

    for (auto& block : function.basic_blocks()) {
        auto* term = block->terminator();
        if (!term || term->opcode() != Opcode::Branch)
            continue;

        // If both targets are the same, convert to unconditional jump
        if (term->true_target() == term->false_target() && term->true_target() != nullptr) {
            replace_branch_with_jump(*block, *term->true_target(), nullptr);
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
        replace_branch_with_jump(*block, *target, not_taken);
        changed = true;
    }

    return changed;
}

}
