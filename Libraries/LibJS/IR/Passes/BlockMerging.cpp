/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/BlockMerging.h>

namespace JS::IR {

bool BlockMerging::run(Function& function)
{
    bool changed = false;

    // Keep merging until no more opportunities
    bool merged_any;
    do {
        merged_any = false;

        for (auto& block_a : function.basic_blocks()) {
            auto* terminator = block_a->last_instruction();
            if (!terminator)
                continue;

            // Only merge if A ends with an unconditional jump
            // (either Jump opcode, or Branch with false_target = nullptr from constant folding)
            bool is_unconditional = terminator->opcode() == Opcode::Jump
                || (terminator->opcode() == Opcode::Branch && terminator->false_target() == nullptr);
            if (!is_unconditional)
                continue;

            auto* block_b = terminator->true_target();
            if (!block_b)
                continue;

            // Don't merge self-loops
            if (block_b == block_a.ptr())
                continue;

            // B must have exactly one predecessor (A)
            if (block_b->predecessors().size() != 1)
                continue;

            // B shouldn't be the entry block
            if (block_b == function.entry_block())
                continue;

            // Don't merge into blocks with phi nodes (preserves loop preheaders)
            if (!block_b->instructions().is_empty() && block_b->instructions().first()->opcode() == Opcode::Phi)
                continue;

            // Merge B into A:
            // 1. Remove the Jump from A
            block_a->instructions().remove(block_a->instructions().size() - 1);

            // 2. Move all instructions from B to A
            for (auto& instruction : block_b->instructions()) {
                instruction->set_parent_block(block_a.ptr());
                block_a->instructions().append(move(instruction));
            }
            block_b->instructions().clear();

            // 3. Update all references to B to point to A
            for (auto& other_block : function.basic_blocks()) {
                for (auto& instruction : other_block->instructions()) {
                    if (instruction->true_target() == block_b)
                        instruction->set_true_target(block_a.ptr());
                    if (instruction->false_target() == block_b)
                        instruction->set_false_target(block_a.ptr());

                    // Update phi predecessor references
                    for (size_t i = 0; i < instruction->phi_predecessors().size(); ++i) {
                        if (instruction->phi_predecessors()[i] == block_b)
                            instruction->set_phi_predecessor(i, block_a.ptr());
                    }
                }

                // Update predecessor lists
                other_block->remove_predecessor(block_b);
            }

            // 4. Copy B's exception handler/finalizer to A if A doesn't have them
            if (!block_a->exception_handler() && block_b->exception_handler())
                block_a->set_exception_handler(block_b->exception_handler());
            if (!block_a->finalizer() && block_b->finalizer())
                block_a->set_finalizer(block_b->finalizer());

            merged_any = true;
            changed = true;
            break; // Restart iteration since we modified the list
        }
    } while (merged_any);

    // Remove empty blocks that were merged
    function.basic_blocks().remove_all_matching([](auto const& block) {
        return block->instructions().is_empty();
    });

    return changed;
}

}
