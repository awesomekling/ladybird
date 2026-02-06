/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Bitmap.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/DeadCodeElimination.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

static inline size_t to_index(ValueIndex v) { return static_cast<u32>(v); }

PreservedAnalyses DeadCodeElimination::run(Function& function, PassManager&)
{
    bool changed = false;

    auto value_capacity = function.values().size();
    auto live_values = MUST(AK::Bitmap::create(value_capacity, false));
    Vector<ValueIndex> worklist;

    // Step 1: Find all "live roots" - values that must be kept because they:
    // - Are used by instructions with side effects
    // - Are used by terminators
    // - Are results of instructions with side effects
    for (auto const& block : function.basic_blocks()) {
        for (auto instruction_index : block->instructions()) {
            auto& instruction = *function.instruction_by_index(instruction_index);
            bool is_live_root = instruction.is_terminator() || instruction.has_side_effects();

            if (is_live_root) {
                // All operands of live instructions are live
                for (size_t j = 0; j < instruction.operand_count(); ++j) {
                    auto* op = instruction.operand(j);
                    if (op && !live_values.get(to_index(op->index()))) {
                        live_values.set(to_index(op->index()), true);
                        worklist.append(op->index());
                    }
                }
            }
        }
    }

    // Step 2: Propagate liveness backwards through operand chains
    // If a value is live, all values it depends on are also live
    while (!worklist.is_empty()) {
        auto value_idx = worklist.take_last();
        auto* value = function.values()[to_index(value_idx)].ptr();

        // If this value is defined by an instruction, its operands are also live
        if (auto* defining_instr = value->defining_instruction()) {
            for (size_t j = 0; j < defining_instr->operand_count(); ++j) {
                auto* op = defining_instr->operand(j);
                if (op && !live_values.get(to_index(op->index()))) {
                    live_values.set(to_index(op->index()), true);
                    worklist.append(op->index());
                }
            }
        }
    }

    // Step 3: Remove dead instructions (those whose results are not live)
    for (auto& block : function.basic_blocks()) {
        for (size_t i = block->instructions().size(); i > 0; --i) {
            auto& instruction = *function.instruction_by_index(block->instructions()[i - 1]);

            // Skip instructions without results (terminators, etc.)
            if (!instruction.result())
                continue;

            // Skip instructions with side effects
            if (instruction.has_side_effects())
                continue;

            // If the result is not live, remove the instruction
            if (!live_values.get(to_index(instruction.result()->index()))) {
                block->remove_instruction(i - 1);
                changed = true;
            }
        }
    }

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
