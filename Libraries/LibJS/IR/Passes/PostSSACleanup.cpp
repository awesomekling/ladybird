/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/PostSSACleanup.h>

namespace JS::IR {

PreservedAnalyses PostSSACleanup::run(Function& function, PassManager&)
{
    // After SSA destruction, split blocks often contain only
    // [ParallelCopy...] + Jump. We eliminate these by moving the copies
    // into each predecessor block (before their terminators) and
    // redirecting edges to skip the split block entirely.
    //
    // It is only safe to move copies into a predecessor that ends with
    // Jump (unconditional). If the predecessor ends with Branch, moving
    // copies before the Branch would make them execute on both paths,
    // which corrupts values when both branch targets have conflicting
    // copies for the same destination.
    //
    // Blocks with only Jump (no ParallelCopy) can always be bypassed
    // regardless of predecessor terminator type, since there are no
    // copies to move.

    HashTable<BasicBlock*> blocks_to_remove;

    for (auto& block : function.basic_blocks()) {
        // Skip the entry block.
        if (block.ptr() == function.entry_block())
            continue;

        // Check that this block contains only [ParallelCopy*] + Jump.
        auto const& instructions = block->instructions();
        if (instructions.is_empty())
            continue;

        auto* terminator = block->terminator();
        if (!terminator || terminator->opcode() != Opcode::Jump)
            continue;

        bool has_copies = false;
        bool all_parallel_copies = true;
        for (size_t i = 0; i + 1 < instructions.size(); ++i) {
            auto* instruction = function.instruction_by_index(instructions[i]);
            if (instruction->opcode() != Opcode::ParallelCopy) {
                all_parallel_copies = false;
                break;
            }
            has_copies = true;
        }
        if (!all_parallel_copies)
            continue;

        // If the block has copies, only eliminate it if all predecessors
        // end with Jump. This ensures copies are only moved into blocks
        // that unconditionally flow into this block.
        if (has_copies) {
            bool all_predecessors_jump = true;
            for (auto predecessor_index : block->predecessor_indices()) {
                auto* predecessor = function.block_by_index(predecessor_index);
                if (!predecessor || !predecessor->terminator()
                    || predecessor->terminator()->opcode() != Opcode::Jump) {
                    all_predecessors_jump = false;
                    break;
                }
            }
            if (!all_predecessors_jump)
                continue;
        }

        auto& target = static_cast<JumpInstruction*>(terminator)->target();

        // Move ParallelCopy instructions into each predecessor before
        // their terminator, then redirect the edge.
        auto predecessor_indices = block->predecessor_indices();
        for (auto predecessor_index : predecessor_indices) {
            auto* predecessor = function.block_by_index(predecessor_index);
            if (!predecessor)
                continue;

            // Duplicate ParallelCopy contents into the predecessor.
            for (size_t i = 0; i + 1 < instructions.size(); ++i) {
                auto* instruction = function.instruction_by_index(instructions[i]);
                auto const& source_copy = static_cast<ParallelCopyInstruction const&>(*instruction);
                auto new_copy = ParallelCopyInstruction::create();
                for (size_t j = 0; j < source_copy.copies().size(); ++j)
                    new_copy->add_copy(source_copy.copy_dst(j), source_copy.copy_src(j));
                predecessor->insert_before_terminator(move(new_copy));
            }

            CFG::redirect_edge(*predecessor, *block, target);
        }

        blocks_to_remove.set(block.ptr());
    }

    if (blocks_to_remove.is_empty())
        return PreservedAnalyses::all();

    CFG::remove_blocks(function, blocks_to_remove);
    return PreservedAnalyses::none();
}

}
