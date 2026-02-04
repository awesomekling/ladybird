/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/LoopInvariantCodeMotion.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

bool LoopInvariantCodeMotion::run(Function& function)
{
    bool changed = false;

    // Find loop headers and their preheaders
    // A loop header is a block that has a back edge (a predecessor that comes after it)
    for (size_t header_idx = 0; header_idx < function.basic_blocks().size(); ++header_idx) {
        auto& header = function.basic_blocks()[header_idx];

        // Check if this block has a back edge
        BasicBlock* back_edge_source = nullptr;
        BasicBlock* preheader = nullptr;

        for (auto* pred : header->predecessors()) {
            // Find the predecessor's index
            size_t pred_idx = SIZE_MAX;
            for (size_t i = 0; i < function.basic_blocks().size(); ++i) {
                if (function.basic_blocks()[i].ptr() == pred) {
                    pred_idx = i;
                    break;
                }
            }

            if (pred_idx > header_idx) {
                // This is a back edge
                back_edge_source = pred;
            } else {
                // This could be the preheader
                preheader = pred;
            }
        }

        // Not a loop header if no back edge
        if (!back_edge_source || !preheader)
            continue;

        // The preheader should only jump to the header (not a branch)
        auto* preheader_term = preheader->last_instruction();
        if (!preheader_term || preheader_term->opcode() != Opcode::Jump)
            continue;

        // Collect all blocks in the loop (reachable from header via back edge path)
        HashTable<BasicBlock*> loop_blocks;
        loop_blocks.set(header.ptr());

        // Simple heuristic: blocks between header and back edge source are in the loop
        for (size_t i = header_idx; i < function.basic_blocks().size(); ++i) {
            auto* block = function.basic_blocks()[i].ptr();
            loop_blocks.set(block);
            if (block == back_edge_source)
                break;
        }

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
                            return loop_block->instructions().take(i);
                        }
                    }
                    VERIFY_NOT_REACHED();
                }();

                // Insert at end of preheader, before the terminator
                size_t insert_pos = preheader->instructions().size() - 1;
                owned_instr->set_parent_block(preheader);
                preheader->instructions().insert(insert_pos, move(owned_instr));

                changed = true;
            }
        }
    }

    return changed;
}

}
