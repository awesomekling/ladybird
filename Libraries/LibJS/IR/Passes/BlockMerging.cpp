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
            auto* terminator = block_a->terminator();
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

            // Don't merge if exception handling context differs - this would change
            // which handler catches exceptions from B's instructions
            if (block_a->exception_handler() != block_b->exception_handler())
                continue;
            if (block_a->finalizer() != block_b->finalizer())
                continue;

            // Merge B into A:
            // 1. Capture B's successors before modifying anything
            auto* b_terminator = block_b->terminator();
            BasicBlock* b_true_target = b_terminator ? b_terminator->true_target() : nullptr;
            BasicBlock* b_false_target = b_terminator ? b_terminator->false_target() : nullptr;

            // 2. Remove the Jump from A
            block_a->instructions().remove(block_a->instructions().size() - 1);

            // 3. Move all instructions from B to A
            for (auto& instruction : block_b->instructions()) {
                instruction->set_parent_block(block_a.ptr());
                block_a->instructions().append(move(instruction));
            }
            block_b->instructions().clear();

            // 4. Update all references to B to point to A
            for (auto& other_block : function.basic_blocks()) {
                // Update terminator targets
                if (auto* other_term = other_block->terminator()) {
                    if (other_term->true_target() == block_b)
                        other_term->set_true_target(block_a.ptr());
                    if (other_term->false_target() == block_b)
                        other_term->set_false_target(block_a.ptr());
                }

                // Update phi predecessor references and predecessor lists
                other_block->replace_phi_predecessor(block_b, block_a.ptr());
                other_block->remove_predecessor(block_b);
            }

            // 5. Add A to the predecessor lists of B's former successors
            if (b_true_target && b_true_target != block_b)
                b_true_target->add_predecessor(block_a.ptr());
            if (b_false_target && b_false_target != block_b && b_false_target != b_true_target)
                b_false_target->add_predecessor(block_a.ptr());

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
