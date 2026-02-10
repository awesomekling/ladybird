/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Bitmap.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/ADCE.h>
#include <LibJS/IR/PostDominatorTree.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

static inline size_t to_index(InstructionIndex i) { return static_cast<u32>(i); }

PreservedAnalyses AggressiveDeadCodeElimination::run(Function& function, PassManager& pass_manager)
{
    auto const& post_dominator_tree = pass_manager.post_dominator_tree(function);

    // Step 1: Mark initial live instructions.
    // Live roots are:
    //   - Instructions with side effects
    //   - Return/End/Throw terminators (observable exits)
    //   - NOT Branch/Jump terminators (these are control-only)

    size_t max_instruction_index = 0;
    for (auto const& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const& instruction) {
            auto index = to_index(instruction.index());
            if (index >= max_instruction_index)
                max_instruction_index = index + 1;
        });
    }

    auto live_instructions = MUST(AK::Bitmap::create(max_instruction_index, false));
    Vector<Instruction*> worklist;

    for (auto const& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const& instruction) {
            bool is_live = false;
            if (instruction.has_side_effects()) {
                is_live = true;
            } else if (instruction.is_terminator()) {
                // Exit terminators are live; Branch/Jump are not initially live.
                switch (instruction.opcode()) {
                case Opcode::Return:
                case Opcode::End:
                case Opcode::Throw:
                    is_live = true;
                    break;
                default:
                    break;
                }
            }

            if (is_live) {
                live_instructions.set(to_index(instruction.index()), true);
                worklist.append(const_cast<Instruction*>(&instruction));
            }
        });
    }

    // Helper: mark a branch in the post-dominance frontier of a block as live.
    auto mark_controlling_branches_live = [&](BasicBlock* block) {
        if (!block)
            return;
        post_dominator_tree.for_each_post_dominance_frontier_block(block, [&](BasicBlock& frontier_block) {
            auto* terminator = frontier_block.terminator();
            if (!terminator)
                return;
            if (terminator->opcode() != Opcode::Branch)
                return;
            auto terminator_index = to_index(terminator->index());
            if (terminator_index < max_instruction_index && !live_instructions.get(terminator_index)) {
                live_instructions.set(terminator_index, true);
                worklist.append(terminator);
            }
        });
    };

    // Step 2: Backward liveness propagation.
    // For each newly-live instruction:
    //   a) Mark all its operands' defining instructions live
    //   b) Mark the controlling branch live (via post-dominance frontier)
    //   c) For live Phi nodes: mark the branches that control which
    //      predecessor path reaches the phi as live
    while (!worklist.is_empty()) {
        auto* instruction = worklist.take_last();

        // (a) Mark operands live
        for (size_t i = 0; i < instruction->operand_count(); ++i) {
            auto* operand = instruction->operand(i);
            if (!operand)
                continue;
            auto* defining_instruction = operand->defining_instruction();
            if (!defining_instruction)
                continue;
            auto defining_index = to_index(defining_instruction->index());
            if (defining_index < max_instruction_index && !live_instructions.get(defining_index)) {
                live_instructions.set(defining_index, true);
                worklist.append(defining_instruction);
            }
        }

        // (b) Mark controlling branches live via post-dominance frontier.
        mark_controlling_branches_live(instruction->parent_block());

        // (c) For Phi nodes: the phi selects a value based on which
        // predecessor was taken. For each predecessor that provides a
        // value to this phi, mark the branch in PostDF(predecessor) live.
        // This ensures the branch deciding which path reaches the phi
        // is not eliminated.
        if (instruction->opcode() == Opcode::Phi) {
            auto const& phi = static_cast<PhiInstruction const&>(*instruction);
            for (size_t i = 0; i < phi.incoming_count(); ++i) {
                auto* predecessor = function.block_by_index(phi.incoming_block(i));
                mark_controlling_branches_live(predecessor);
            }
        }
    }

    // Step 3: Safety net — if a dead branch has no post-dominator (and
    // thus can't be converted to a jump), mark it live so its operands
    // survive the instruction removal phase.
    for (auto const& block : function.basic_blocks()) {
        auto* terminator = block->terminator();
        if (!terminator || terminator->opcode() != Opcode::Branch)
            continue;
        if (live_instructions.get(to_index(terminator->index())))
            continue;
        auto* post_dominator = post_dominator_tree.immediate_post_dominator(block.ptr());
        if (post_dominator)
            continue;
        // This branch is dead but can't be converted. Mark it and its
        // operands live so the branch condition is not removed.
        live_instructions.set(to_index(terminator->index()), true);
        for (size_t i = 0; i < terminator->operand_count(); ++i) {
            auto* operand = terminator->operand(i);
            if (!operand)
                continue;
            auto* defining_instruction = operand->defining_instruction();
            if (!defining_instruction)
                continue;
            auto defining_index = to_index(defining_instruction->index());
            if (defining_index < max_instruction_index)
                live_instructions.set(defining_index, true);
        }
    }

    // Step 4: Remove dead code.
    bool removed_instructions = false;
    bool converted_branches = false;

    for (auto& block : function.basic_blocks()) {
        // Remove dead non-terminator instructions.
        for (size_t i = block->instructions().size(); i > 0; --i) {
            auto& instruction = *function.instruction_by_index(block->instructions()[i - 1]);

            // Don't remove terminators in this loop (handled separately below).
            if (instruction.is_terminator())
                continue;

            // Don't remove instructions without results that have side effects.
            if (instruction.has_side_effects())
                continue;

            // Skip instructions that are live.
            if (live_instructions.get(to_index(instruction.index())))
                continue;

            if (!instruction.result())
                continue;

            block->remove_instruction(i - 1);
            removed_instructions = true;
        }

        // Convert dead Branch terminators to unconditional jumps.
        auto* terminator = block->terminator();
        if (!terminator)
            continue;
        if (terminator->opcode() != Opcode::Branch)
            continue;
        if (live_instructions.get(to_index(terminator->index())))
            continue;

        // The branch is dead — no live instruction depends on which
        // path is taken. Replace it with an unconditional jump to one
        // of the targets. We prefer the immediate post-dominator when
        // it is a direct target; otherwise we jump to the true target
        // and let SimplifyCFG clean up the resulting chain.
        auto* post_dominator = post_dominator_tree.immediate_post_dominator(block.ptr());
        if (!post_dominator)
            continue;

        auto* true_target = terminator->true_target();
        auto* false_target = terminator->false_target();

        if (true_target == false_target) {
            // Both targets are the same block — just replace with a jump.
            // Don't remove any predecessor since the single edge is kept.
            block->remove_terminator();
            block->append(JumpInstruction::create(*true_target));
        } else if (post_dominator == false_target) {
            CFG::replace_branch_with_jump(*block, *false_target, true_target);
        } else {
            CFG::replace_branch_with_jump(*block, *true_target, false_target);
        }
        converted_branches = true;
    }

    if (converted_branches)
        return PreservedAnalyses::none();
    if (removed_instructions)
        return PreservedAnalyses::all_cfg_analyses();
    return PreservedAnalyses::all();
}

}
