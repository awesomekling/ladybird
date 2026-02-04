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

            // Check if thread_target uses any values defined in the bypassed block
            // (except through phi nodes in thread_target that we'll update)
            // If so, we can't safely thread because those values won't be available
            // when coming directly from pred_block.
            bool target_uses_bypassed_values = false;
            for (auto& instr : thread_target->instructions()) {
                // Skip phi nodes - we handle those separately
                if (instr->opcode() == Opcode::Phi)
                    continue;
                for (auto* operand : instr->operands()) {
                    if (operand->defining_instruction() && operand->defining_instruction()->parent_block() == block.ptr()) {
                        target_uses_bypassed_values = true;
                        break;
                    }
                }
                if (target_uses_bypassed_values)
                    break;
            }

            if (target_uses_bypassed_values)
                continue; // Can't thread this case safely

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

                // Remove phi operands in the bypassed block for the threaded predecessor
                for (auto& instr : block->instructions()) {
                    if (instr->opcode() != Opcode::Phi)
                        break;
                    for (size_t j = instr->phi_predecessors().size(); j > 0; --j) {
                        if (instr->phi_predecessors()[j - 1] == pred_block) {
                            instr->remove_phi_operand(j - 1);
                            break;
                        }
                    }
                }

                // Update phi nodes in the target block to include the threaded predecessor
                for (auto& instr : thread_target->instructions()) {
                    if (instr->opcode() != Opcode::Phi)
                        continue;

                    // Find the value this phi expects from the bypassed block
                    Value* value_from_bypassed = nullptr;
                    for (size_t j = 0; j < instr->phi_predecessors().size(); ++j) {
                        if (instr->phi_predecessors()[j] == block.ptr()) {
                            value_from_bypassed = instr->operands()[j];
                            break;
                        }
                    }

                    if (!value_from_bypassed)
                        continue;

                    // If value_from_bypassed is a phi in the bypassed block, we need to
                    // find what value that phi receives from pred_block
                    Value* value_for_pred = value_from_bypassed;
                    if (value_from_bypassed->defining_instruction() && value_from_bypassed->defining_instruction()->opcode() == Opcode::Phi && value_from_bypassed->defining_instruction()->parent_block() == block.ptr()) {
                        // This is a phi in the bypassed block - find the value from pred_block
                        auto* bypassed_phi = value_from_bypassed->defining_instruction();
                        for (size_t k = 0; k < bypassed_phi->phi_predecessors().size(); ++k) {
                            if (bypassed_phi->phi_predecessors()[k] == pred_block) {
                                value_for_pred = bypassed_phi->operands()[k];
                                break;
                            }
                        }
                    }

                    instr->add_phi_operand(pred_block, value_for_pred);
                }

                changed = true;
            }
        }
    }

    return changed;
}

}
