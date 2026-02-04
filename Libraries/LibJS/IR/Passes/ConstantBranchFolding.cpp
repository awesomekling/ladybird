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

bool ConstantBranchFolding::run(Function& function)
{
    bool changed = false;

    for (auto& block : function.basic_blocks()) {
        auto* term = block->terminator();
        if (!term || term->opcode() != Opcode::Branch)
            continue;

        // If both targets are the same, convert to unconditional jump
        if (term->true_target() == term->false_target() && term->true_target() != nullptr) {
            term->set_false_target(nullptr);
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

        // Convert Branch to Jump
        auto* target = take_true_branch ? term->true_target() : term->false_target();
        auto* not_taken = take_true_branch ? term->false_target() : term->true_target();

        // Update the instruction to be a Jump
        // NB: We can't easily change the opcode, so we update the targets instead
        // and let dead block elimination clean up the unreachable block
        term->set_true_target(target);
        term->set_false_target(nullptr);

        // Remove this block from the not-taken block's predecessors
        if (not_taken)
            CFG::remove_predecessor(*not_taken, *block);

        changed = true;
    }

    return changed;
}

}
