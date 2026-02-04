/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <AK/Queue.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
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

        // Check terminator targets
        if (auto* term = block->terminator()) {
            if (auto* target = term->true_target()) {
                if (!reachable.contains(target)) {
                    reachable.set(target);
                    worklist.enqueue(target);
                }
            }
            if (auto* target = term->false_target()) {
                if (!reachable.contains(target)) {
                    reachable.set(target);
                    worklist.enqueue(target);
                }
            }
        }

        // Also check exception handlers
        if (auto* handler = block->exception_handler()) {
            if (!reachable.contains(handler)) {
                reachable.set(handler);
                worklist.enqueue(handler);
            }
        }
        if (auto* finalizer = block->finalizer()) {
            if (!reachable.contains(finalizer)) {
                reachable.set(finalizer);
                worklist.enqueue(finalizer);
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

    // Clean up all references to dead blocks in live blocks
    for (auto& block : function.basic_blocks()) {
        if (!reachable.contains(block.ptr()))
            continue;

        for (auto* dead_block : dead_blocks)
            CFG::remove_block_reference(*block, *dead_block);
    }

    // Clear operand uses for instructions in dead blocks before removing them,
    // so use lists don't contain stale references
    for (auto* dead_block : dead_blocks) {
        for (auto& instruction : dead_block->instructions())
            instruction->clear_operand_uses();
    }

    // Remove unreachable blocks
    function.basic_blocks().remove_all_matching([&](auto const& block) {
        return !reachable.contains(block.ptr());
    });

    return true;
}

}
