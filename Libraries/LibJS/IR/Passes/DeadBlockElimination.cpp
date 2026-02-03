/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <AK/Queue.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/DeadBlockElimination.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

bool DeadBlockElimination::run(Function& function)
{
    // Find all reachable blocks using BFS from entry
    HashTable<BasicBlock*> reachable;
    Queue<BasicBlock*> worklist;

    if (auto* entry = function.entry_block()) {
        worklist.enqueue(entry);
        reachable.set(entry);
    }

    while (!worklist.is_empty()) {
        auto* block = worklist.dequeue();

        for (auto const& instruction : block->instructions()) {
            if (auto* target = instruction->true_target()) {
                if (!reachable.contains(target)) {
                    reachable.set(target);
                    worklist.enqueue(target);
                }
            }
            if (auto* target = instruction->false_target()) {
                if (!reachable.contains(target)) {
                    reachable.set(target);
                    worklist.enqueue(target);
                }
            }
        }
    }

    // Collect dead blocks
    Vector<BasicBlock*> dead_blocks;
    for (auto& block : function.basic_blocks()) {
        if (!reachable.contains(block.ptr()))
            dead_blocks.append(block.ptr());
    }

    if (dead_blocks.is_empty())
        return false;

    // Build a map of phi results from dead blocks to their replacement values
    // If a phi in a dead block has all identical operands, we can replace uses of
    // the phi result with that operand value
    HashMap<Value*, Value*> dead_phi_replacements;
    for (auto* dead_block : dead_blocks) {
        for (auto& instruction : dead_block->instructions()) {
            if (instruction->opcode() != Opcode::Phi)
                continue;

            auto const& operands = instruction->operands();
            if (operands.is_empty())
                continue;

            // Check if all operands are the same
            Value* replacement = operands[0];
            bool all_same = true;
            for (size_t i = 1; i < operands.size(); ++i) {
                if (operands[i] != replacement) {
                    all_same = false;
                    break;
                }
            }

            if (all_same && replacement && instruction->result())
                dead_phi_replacements.set(instruction->result(), replacement);
        }
    }

    // Replace uses of dead phi results in live blocks
    for (auto& block : function.basic_blocks()) {
        if (!reachable.contains(block.ptr()))
            continue;

        for (auto& instruction : block->instructions()) {
            for (size_t i = 0; i < instruction->operands().size(); ++i) {
                auto* operand = instruction->operands()[i];
                // Follow the replacement chain
                auto replacement = dead_phi_replacements.get(operand);
                while (replacement.has_value()) {
                    operand = *replacement;
                    replacement = dead_phi_replacements.get(operand);
                }
                if (operand != instruction->operands()[i])
                    instruction->set_operand(i, operand);
            }
        }
    }

    // For each phi in live blocks that references a dead block, remove that entry
    for (auto& block : function.basic_blocks()) {
        if (!reachable.contains(block.ptr()))
            continue;

        for (auto& instruction : block->instructions()) {
            if (instruction->opcode() != Opcode::Phi)
                continue;

            // Check each phi predecessor
            for (size_t i = instruction->phi_predecessors().size(); i > 0; --i) {
                auto* pred = instruction->phi_predecessors()[i - 1];
                if (!reachable.contains(pred))
                    instruction->remove_phi_operand(i - 1);
            }
        }
    }

    // Remove unreachable blocks
    function.basic_blocks().remove_all_matching([&](auto const& block) {
        return !reachable.contains(block.ptr());
    });

    return true;
}

}
