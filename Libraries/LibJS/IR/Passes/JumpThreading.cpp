/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/JumpThreading.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

PreservedAnalyses JumpThreading::run(Function& function, PassManager&)
{
    bool changed = false;

    // Look for blocks where a Branch condition is a Phi node
    // and some phi inputs are constants
    for (auto& block : function.basic_blocks()) {
        auto* terminator = block->terminator();
        if (!terminator || terminator->opcode() != Opcode::Branch)
            continue;

        if (terminator->operands().is_empty())
            continue;

        auto* condition = terminator->operands()[0];
        if (!condition->defining_instruction())
            continue;

        auto* phi_instr = condition->defining_instruction();
        if (phi_instr->opcode() != Opcode::Phi)
            continue;

        auto& phi = static_cast<PhiInstruction&>(*phi_instr);

        // The phi must be in this same block
        if (phi.parent_block() != block.ptr())
            continue;

        auto* true_target = terminator->true_target();
        auto* false_target = terminator->false_target();

        if (!true_target || !false_target)
            continue;

        // For each phi predecessor with a constant value, we can thread
        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            auto* pred_block = phi.incoming_block(i);
            auto* pred_value = phi.incoming_value(i);

            if (!pred_value)
                continue;

            auto truthiness = pred_value->constant_truthiness();
            if (!truthiness.has_value())
                continue;

            bool take_true = *truthiness;

            auto* thread_target = take_true ? true_target : false_target;

            // Check if the bypassed block has side effects that must execute.
            // We can't thread if the block has observable side effects.
            bool bypassed_block_has_side_effects = false;
            for (auto& instr : block->instructions()) {
                // Skip phi nodes and terminators
                if (instr->opcode() == Opcode::Phi)
                    continue;
                if (instr->is_terminator())
                    continue;

                if (instr->has_side_effects()) {
                    bypassed_block_has_side_effects = true;
                    break;
                }
            }

            if (bypassed_block_has_side_effects)
                continue; // Can't thread - bypassed block has side effects

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

            // Check if the phi result is used outside the bypassed block.
            // If the phi has uses beyond just the branch in this block, we can't safely
            // remove the phi operand because other code depends on the merged value.
            bool phi_used_outside_block = false;
            if (auto* phi_result = phi.result()) {
                for (auto* use : phi_result->uses()) {
                    if (use != terminator) {
                        phi_used_outside_block = true;
                        break;
                    }
                }
            }

            if (phi_used_outside_block)
                continue; // Can't thread - phi result is used elsewhere

            // Redirect edge from pred_block: bypass the current block, go to thread_target
            if (!pred_block->terminator())
                continue;

            // Check if pred actually targets this block
            auto* pred_terminator = pred_block->terminator();
            if (pred_terminator->true_target() != block.ptr() && pred_terminator->false_target() != block.ptr())
                continue;

            // Precompute traced phi values before redirect_edge removes pred_block
            // from the bypassed block's phis (which would break tracing).
            HashMap<Instruction*, Value*> traced_values;
            for (auto& instr : thread_target->instructions()) {
                if (instr->opcode() != Opcode::Phi)
                    break;

                auto& target_phi = static_cast<PhiInstruction&>(*instr);

                Value* value_from_bypassed = nullptr;
                for (size_t j = 0; j < target_phi.incoming_count(); ++j) {
                    if (target_phi.incoming_block(j) == block.ptr()) {
                        value_from_bypassed = target_phi.incoming_value(j);
                        break;
                    }
                }

                if (!value_from_bypassed)
                    continue;

                // If value_from_bypassed is a phi in the bypassed block, trace to find
                // what value pred_block would contribute
                if (auto* def = value_from_bypassed->defining_instruction();
                    def && def->opcode() == Opcode::Phi && def->parent_block() == block.ptr()) {
                    auto& def_phi = static_cast<PhiInstruction&>(*def);
                    for (size_t k = 0; k < def_phi.incoming_count(); ++k) {
                        if (def_phi.incoming_block(k) == pred_block) {
                            traced_values.set(instr.ptr(), def_phi.incoming_value(k));
                            break;
                        }
                    }
                } else {
                    traced_values.set(instr.ptr(), value_from_bypassed);
                }
            }

            CFG::redirect_edge(*pred_block, *block, *thread_target, [&](Instruction& instr, Value*) -> Value* {
                if (auto it = traced_values.find(&instr); it != traced_values.end())
                    return it->value;
                return nullptr;
            });

            changed = true;
        }
    }

    return changed ? PreservedAnalyses::none() : PreservedAnalyses::all();
}

}
