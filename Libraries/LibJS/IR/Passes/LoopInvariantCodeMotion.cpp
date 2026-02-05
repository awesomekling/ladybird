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
#include <LibJS/IR/Passes/LoopInvariantCodeMotion.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

// Collect all blocks in a natural loop given the header and back-edge source.
// A natural loop consists of the header plus all blocks that can reach the
// back-edge source without going through the header.
static HashTable<BasicBlock*> collect_loop_blocks(BasicBlock* header, BasicBlock* back_edge_source)
{
    HashTable<BasicBlock*> loop_blocks;
    loop_blocks.set(header);

    if (back_edge_source == header)
        return loop_blocks; // Single-block loop

    // Work backwards from back_edge_source to find all blocks in the loop
    Queue<BasicBlock*> worklist;
    worklist.enqueue(back_edge_source);
    loop_blocks.set(back_edge_source);

    while (!worklist.is_empty()) {
        auto* block = worklist.dequeue();
        for (auto* pred : block->predecessors()) {
            if (!loop_blocks.contains(pred)) {
                loop_blocks.set(pred);
                worklist.enqueue(pred);
            }
        }
    }

    return loop_blocks;
}

PreservedAnalyses LoopInvariantCodeMotion::run(Function& function, PassManager& pass_manager)
{
    bool changed = false;

    // Compute dominators for proper back-edge detection
    auto const& dominators = pass_manager.dominators(function);

    // Find natural loops by looking for back-edges
    // A back-edge is an edge B -> H where H dominates B
    for (auto& header : function.basic_blocks()) {
        BasicBlock* back_edge_source = nullptr;
        BasicBlock* preheader = nullptr;

        for (auto* pred : header->predecessors()) {
            if (dominators.dominates(header.ptr(), pred)) {
                // This is a back-edge: pred -> header where header dominates pred
                back_edge_source = pred;
            } else {
                // This could be the preheader (entry edge into the loop)
                preheader = pred;
            }
        }

        // Not a loop header if no back-edge found
        if (!back_edge_source)
            continue;

        // Need exactly one non-back-edge predecessor as the preheader
        // (multiple entry points make hoisting unsafe without more analysis)
        if (!preheader)
            continue;

        // Verify the preheader has exactly one successor (the header)
        // This ensures hoisted code will execute exactly when entering the loop
        auto* preheader_term = preheader->last_instruction();
        if (!preheader_term || preheader_term->opcode() != Opcode::Jump)
            continue;

        // Collect all blocks in the natural loop
        auto loop_blocks = collect_loop_blocks(header.ptr(), back_edge_source);

        // Find loop-invariant instructions in loop body blocks (not the header)
        for (auto* loop_block : loop_blocks) {
            if (loop_block == header.ptr())
                continue; // Don't hoist from header (has phi nodes)

            Vector<Instruction*> to_hoist;

            for (auto& instruction : loop_block->instructions()) {
                if (!instruction->result())
                    continue;

                if (!instruction->is_hoistable())
                    continue;

                // Check if all operands are defined outside the loop
                bool all_operands_invariant = true;
                for (auto* operand : instruction->operands()) {
                    if (!operand)
                        continue;

                    // Constants are always invariant
                    if (operand->is_constant())
                        continue;

                    // Check if the operand is defined in a loop block
                    auto* def_instr = operand->defining_instruction();
                    if (def_instr && loop_blocks.contains(def_instr->parent_block())) {
                        all_operands_invariant = false;
                        break;
                    }
                }

                if (all_operands_invariant)
                    to_hoist.append(instruction.ptr());
            }

            // Move instructions to preheader (insert before the jump)
            for (auto* instruction : to_hoist) {
                // Find and take ownership from current block
                NonnullOwnPtr<Instruction> owned_instr = [&]() -> NonnullOwnPtr<Instruction> {
                    for (size_t i = 0; i < loop_block->instructions().size(); ++i) {
                        if (loop_block->instructions()[i].ptr() == instruction) {
                            return loop_block->take_instruction(i);
                        }
                    }
                    VERIFY_NOT_REACHED();
                }();

                // Insert at end of preheader, before the terminator
                preheader->insert_before_terminator(move(owned_instr));

                changed = true;
            }
        }
    }

    return changed ? PreservedAnalyses::none() : PreservedAnalyses::all();
}

}
