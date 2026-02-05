/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/DeadCodeElimination.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

PreservedAnalyses DeadCodeElimination::run(Function& function, PassManager&)
{
    bool changed = false;

    // Step 1: Find all "live roots" - values that must be kept because they:
    // - Are used by instructions with side effects
    // - Are used by terminators
    // - Are results of instructions with side effects
    HashTable<Value*> live_values;
    Vector<Value*> worklist;

    for (auto const& block : function.basic_blocks()) {
        for (auto const& instruction : block->instructions()) {
            bool is_live_root = instruction->is_terminator() || instruction->has_side_effects();

            if (is_live_root) {
                // All operands of live instructions are live
                for (auto* operand : instruction->operands()) {
                    if (operand && !live_values.contains(operand)) {
                        live_values.set(operand);
                        worklist.append(operand);
                    }
                }
            }
        }
    }

    // Step 2: Propagate liveness backwards through operand chains
    // If a value is live, all values it depends on are also live
    while (!worklist.is_empty()) {
        auto* value = worklist.take_last();

        // If this value is defined by an instruction, its operands are also live
        if (auto* defining_instr = value->defining_instruction()) {
            for (auto* operand : defining_instr->operands()) {
                if (operand && !live_values.contains(operand)) {
                    live_values.set(operand);
                    worklist.append(operand);
                }
            }
        }
    }

    // Step 3: Remove dead instructions (those whose results are not live)
    for (auto& block : function.basic_blocks()) {
        for (size_t i = block->instructions().size(); i > 0; --i) {
            auto& instruction = block->instructions()[i - 1];

            // Skip instructions without results (terminators, etc.)
            if (!instruction->result())
                continue;

            // Skip instructions with side effects
            if (instruction->has_side_effects())
                continue;

            // If the result is not live, remove the instruction
            if (!live_values.contains(instruction->result())) {
                block->remove_instruction(i - 1);
                changed = true;
            }
        }
    }

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
