/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/CopyPropagation.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

static inline size_t to_index(ValueIndex v) { return static_cast<u32>(v); }

PreservedAnalyses CopyPropagation::run(Function& function, PassManager&)
{
    bool changed = false;

    auto value_capacity = function.values().size();

    // Build a map of copy relationships: if v1 = Move v0, then copies[v1] = v0
    // NB: We don't propagate through phi nodes since they represent merge points
    Vector<Optional<ValueIndex>> copies;
    copies.resize(value_capacity);
    bool has_copies = false;

    for (auto const& block : function.basic_blocks()) {
        for (auto const& instruction : block->instructions()) {
            if (instruction->opcode() == Opcode::Move && instruction->result()) {
                auto* src = instruction->operands()[0];
                copies[to_index(instruction->result()->index())] = src->index();
                has_copies = true;
            }
        }
    }

    // Helper to follow the copy chain to find the ultimate source
    auto resolve = [&](ValueIndex v) -> ValueIndex {
        for (;;) {
            auto vi = to_index(v);
            if (vi >= copies.size() || !copies[vi].has_value())
                break;
            v = *copies[vi];
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
            Optional<ValueIndex> common_idx;
            bool all_same = true;
            for (auto* operand : operands) {
                if (!operand)
                    continue;
                auto resolved = resolve(operand->index());
                if (!common_idx.has_value()) {
                    common_idx = resolved;
                } else if (resolved != *common_idx) {
                    all_same = false;
                    break;
                }
            }

            if (all_same && common_idx.has_value() && phi.result()) {
                auto* common_value = function.values()[to_index(*common_idx)].ptr();
                phi.result()->replace_all_uses_with(common_value);
                changed = true;
            }
        });
    }

    if (!has_copies)
        return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();

    // Replace uses of copied values with their sources
    for (auto& block : function.basic_blocks()) {
        for (auto& instruction : block->instructions()) {
            for (size_t i = 0; i < instruction->operands().size(); ++i) {
                auto* operand = instruction->operands()[i];
                if (!operand)
                    continue;

                // Follow the copy chain to find the ultimate source
                auto resolved = resolve(operand->index());

                if (resolved != operand->index()) {
                    auto* replacement = function.values()[to_index(resolved)].ptr();
                    instruction->set_operand(i, replacement);
                    changed = true;
                }
            }
        }
    }

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
