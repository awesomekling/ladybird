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

    // Remove unreachable blocks
    bool changed = false;
    function.basic_blocks().remove_all_matching([&](auto const& block) {
        if (!reachable.contains(block.ptr())) {
            changed = true;
            return true;
        }
        return false;
    });

    return changed;
}

}
