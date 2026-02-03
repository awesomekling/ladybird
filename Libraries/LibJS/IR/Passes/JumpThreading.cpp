/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/JumpThreading.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

bool JumpThreading::run(Function& function)
{
    bool changed = false;

    // Look for blocks where a Branch condition is a Phi node
    // and some phi inputs are constants
    for (auto& block : function.basic_blocks()) {
        auto* terminator = block->last_instruction();
        if (!terminator || terminator->opcode() != Opcode::Branch)
            continue;

        if (terminator->operands().is_empty())
            continue;

        auto* condition = terminator->operands()[0];
        if (!condition->defining_instruction())
            continue;

        auto* phi = condition->defining_instruction();
        if (phi->opcode() != Opcode::Phi)
            continue;

        // The phi must be in this same block
        if (phi->parent_block() != block.ptr())
            continue;

        auto* true_target = terminator->true_target();
        auto* false_target = terminator->false_target();

        if (!true_target || !false_target)
            continue;

        // For each phi predecessor with a constant value, we can thread
        for (size_t i = 0; i < phi->phi_predecessors().size(); ++i) {
            auto* pred_block = phi->phi_predecessors()[i];
            auto* pred_value = phi->operands()[i];

            if (!pred_value || !pred_value->is_constant())
                continue;

            auto const& const_value = pred_value->constant_value();

            // Determine which branch to take
            bool take_true = false;
            if (const_value.is_boolean()) {
                take_true = const_value.as_bool();
            } else if (const_value.is_int32()) {
                take_true = const_value.as_i32() != 0;
            } else if (const_value.is_undefined() || const_value.is_null()) {
                take_true = false;
            } else {
                // Can't determine truthiness
                continue;
            }

            auto* thread_target = take_true ? true_target : false_target;

            // Update the predecessor to jump directly to the target
            auto* pred_terminator = pred_block->last_instruction();
            if (!pred_terminator)
                continue;

            bool updated = false;
            if (pred_terminator->true_target() == block.ptr()) {
                pred_terminator->set_true_target(thread_target);
                updated = true;
            }
            if (pred_terminator->false_target() == block.ptr()) {
                pred_terminator->set_false_target(thread_target);
                updated = true;
            }

            if (updated) {
                // Update predecessor lists
                thread_target->add_predecessor(pred_block);
                block->remove_predecessor(pred_block);
                changed = true;
            }
        }
    }

    return changed;
}

}
