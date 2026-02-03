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

bool CopyPropagation::run(Function& function)
{
    bool changed = false;

    // Build a map of copy relationships: if v1 = Move v0, then copies[v1] = v0
    // NB: We don't propagate through phi nodes since they represent merge points
    HashMap<Value*, Value*> copies;

    for (auto const& block : function.basic_blocks()) {
        for (auto const& instruction : block->instructions()) {
            if (instruction->opcode() == Opcode::Move && instruction->result()) {
                // v_result = Move v_src
                auto* src = instruction->operands()[0];
                // Don't propagate if the source is a phi result
                if (src->defining_instruction() && src->defining_instruction()->opcode() == Opcode::Phi)
                    continue;
                copies.set(instruction->result(), src);
            }
        }
    }

    if (copies.is_empty())
        return false;

    // Replace uses of copied values with their sources
    // NB: Skip phi nodes since their operands represent values from specific predecessors,
    // and propagating through them would break SSA semantics
    for (auto& block : function.basic_blocks()) {
        for (auto& instruction : block->instructions()) {
            if (instruction->opcode() == Opcode::Phi)
                continue;
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

    return changed;
}

}
