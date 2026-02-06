/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/CopyPropagation.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

PreservedAnalyses CopyPropagation::run(Function& function, PassManager&)
{
    bool changed = false;

    // Build a map of copy relationships: if v1 = Move v0, then copies[v1] = v0
    // NB: We don't propagate through phi nodes since they represent merge points
    HashMap<Value*, Value*> copies;

    for (auto const& block : function.basic_blocks()) {
        for (auto const& instruction : block->instructions()) {
            if (instruction->opcode() == Opcode::Move && instruction->result()) {
                auto* src = instruction->operands()[0];
                copies.set(instruction->result(), src);
            }
        }
    }

    // Helper to follow the copy chain to find the ultimate source
    auto resolve = [&](Value* v) -> Value* {
        for (;;) {
            auto source = copies.get(v);
            if (!source.has_value())
                break;
            v = *source;
        }
        return v;
    };

    // Trivial phi elimination: if all operands of a phi resolve to the same value,
    // replace the phi result with that value.
    // Example: v1 = Move v0; v2 = Move v0; v = Phi [v1, v2] → v = v0
    for (auto& block : function.basic_blocks()) {
        block->for_each_phi([&](PhiInstruction& phi) {
            auto const& operands = phi.operands();
            if (operands.is_empty())
                return;

            // Check if all operands resolve to the same ultimate source
            Value* common_value = nullptr;
            bool all_same = true;
            for (auto* operand : operands) {
                if (!operand)
                    continue;
                auto* resolved = resolve(operand);
                if (!common_value) {
                    common_value = resolved;
                } else if (resolved != common_value) {
                    all_same = false;
                    break;
                }
            }

            if (all_same && common_value && phi.result()) {
                phi.result()->replace_all_uses_with(common_value);
                changed = true;
            }
        });
    }

    if (copies.is_empty())
        return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();

    // Replace uses of copied values with their sources
    for (auto& block : function.basic_blocks()) {
        for (auto& instruction : block->instructions()) {
            for (size_t i = 0; i < instruction->operands().size(); ++i) {
                auto* operand = instruction->operands()[i];

                // Follow the copy chain to find the ultimate source
                auto* replacement = operand;
                for (;;) {
                    auto source = copies.get(replacement);
                    if (!source.has_value())
                        break;
                    replacement = *source;
                }

                if (replacement != operand) {
                    instruction->set_operand(i, replacement);
                    changed = true;
                }
            }
        }
    }

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
