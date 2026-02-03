/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/ConstantBranchFolding.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

bool ConstantBranchFolding::run(Function& function)
{
    bool changed = false;

    for (auto& block : function.basic_blocks()) {
        auto* last = block->last_instruction();
        if (!last || last->opcode() != Opcode::Branch)
            continue;

        // Branch has condition as first operand
        if (last->operands().is_empty())
            continue;

        auto* condition = last->operands()[0];
        if (!condition->is_constant())
            continue;

        auto const& const_value = condition->constant_value();

        // Determine which branch to take
        bool take_true_branch = false;
        if (const_value.is_boolean()) {
            take_true_branch = const_value.as_bool();
        } else if (const_value.is_int32()) {
            take_true_branch = const_value.as_i32() != 0;
        } else if (const_value.is_undefined() || const_value.is_null()) {
            take_true_branch = false;
        } else {
            // Can't fold this branch
            continue;
        }

        // Convert Branch to Jump
        auto* target = take_true_branch ? last->true_target() : last->false_target();
        auto* not_taken = take_true_branch ? last->false_target() : last->true_target();

        // Update the instruction to be a Jump
        // NB: We can't easily change the opcode, so we update the targets instead
        // and let dead block elimination clean up the unreachable block
        last->set_true_target(target);
        last->set_false_target(nullptr);

        // Remove predecessor from the not-taken block
        if (not_taken)
            not_taken->remove_predecessor(block.ptr());

        changed = true;
    }

    return changed;
}

}
