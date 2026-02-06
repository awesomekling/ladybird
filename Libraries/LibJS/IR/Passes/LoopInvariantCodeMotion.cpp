/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/LoopTree.h>
#include <LibJS/IR/Passes/LoopInvariantCodeMotion.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

PreservedAnalyses LoopInvariantCodeMotion::run(Function& function, PassManager& pass_manager)
{
    bool changed = false;

    auto const& lt = pass_manager.loop_tree(function);
    auto const& dom_tree = pass_manager.dominator_tree(function);

    for (auto const& loop : lt.loops()) {
        auto* header = loop->header();

        // Find the preheader: the single non-back-edge predecessor ending
        // with an unconditional jump. LoopSimplify guarantees at most one
        // entry-edge predecessor, but we still validate.
        BasicBlock* preheader = nullptr;
        for (auto* pred : header->predecessors()) {
            if (dom_tree.dominates(header, pred))
                continue; // back-edge
            auto* term = pred->last_instruction();
            if (!term || term->opcode() != Opcode::Jump)
                continue;
            preheader = pred;
            break;
        }

        if (!preheader)
            continue;

        // Find loop-invariant instructions in loop body blocks (not the header).
        for (auto* loop_block : loop->blocks()) {
            if (loop_block == header)
                continue; // Don't hoist from header (has phi nodes)

            Vector<Instruction*> to_hoist;

            for (auto const& instruction : loop_block->instructions()) {
                if (!instruction->result())
                    continue;

                // Never hoist Phi nodes — they are tied to their block's predecessors.
                if (instruction->opcode() == Opcode::Phi)
                    continue;

                if (!instruction->is_hoistable())
                    continue;

                // Check if all operands are defined outside the loop
                bool all_operands_invariant = true;
                for (auto* operand : instruction->operands()) {
                    if (!operand)
                        continue;

                    if (operand->is_constant())
                        continue;

                    auto* def_instr = operand->defining_instruction();
                    if (def_instr && loop->contains(def_instr->parent_block())) {
                        all_operands_invariant = false;
                        break;
                    }
                }

                if (all_operands_invariant)
                    to_hoist.append(instruction.ptr());
            }

            // Move instructions to preheader (insert before the jump)
            for (auto* instruction : to_hoist) {
                NonnullOwnPtr<Instruction> owned_instr = [&]() -> NonnullOwnPtr<Instruction> {
                    for (size_t i = 0; i < loop_block->instructions().size(); ++i) {
                        if (loop_block->instructions()[i].ptr() == instruction) {
                            return loop_block->take_instruction(i);
                        }
                    }
                    VERIFY_NOT_REACHED();
                }();

                preheader->insert_before_terminator(move(owned_instr));

                changed = true;
            }
        }
    }

    return changed ? PreservedAnalyses::none() : PreservedAnalyses::all();
}

}
