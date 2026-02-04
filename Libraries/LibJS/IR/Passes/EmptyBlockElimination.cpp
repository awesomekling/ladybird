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
#include <LibJS/IR/Passes/EmptyBlockElimination.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

bool EmptyBlockElimination::run(Function& function)
{
    bool changed = false;
    bool eliminated_any;

    do {
        eliminated_any = false;

        for (auto& block : function.basic_blocks()) {
            bool is_entry = block.ptr() == function.entry_block();

            // Check if block is empty (only a Jump instruction)
            if (block->instructions().size() != 1)
                continue;

            auto* jump = block->terminator();
            if (!jump || jump->opcode() != Opcode::Jump)
                continue;

            auto* target = jump->true_target();
            if (!target)
                continue;

            // Don't eliminate if jumping to self
            if (target == block.ptr())
                continue;

            // Get predecessors of the empty block
            auto predecessors = block->predecessors();

            // Don't eliminate if any predecessor would end up reaching the target
            // via two different paths with different phi values
            bool would_conflict = false;
            for (auto* pred : predecessors) {
                // Check if this predecessor already reaches the target directly
                auto* pred_term = pred->terminator();
                if (pred_term && (pred_term->true_target() == target || pred_term->false_target() == target)) {
                    // This predecessor can reach target both directly and via empty block
                    // Check if any phi in target would have different values for these paths
                    for (auto& phi_instr : target->instructions()) {
                        if (phi_instr->opcode() != Opcode::Phi)
                            continue;

                        Value* value_from_empty = nullptr;
                        Value* value_from_direct = nullptr;

                        for (size_t i = 0; i < phi_instr->phi_predecessors().size(); ++i) {
                            if (phi_instr->phi_predecessors()[i] == block.ptr())
                                value_from_empty = phi_instr->operands()[i];
                            if (phi_instr->phi_predecessors()[i] == pred)
                                value_from_direct = phi_instr->operands()[i];
                        }

                        if (value_from_empty && value_from_direct && value_from_empty != value_from_direct) {
                            would_conflict = true;
                            break;
                        }
                    }
                }
                if (would_conflict)
                    break;
            }

            if (would_conflict)
                continue;

            // Entry block has no predecessors - only eliminate if target has no phis
            // (otherwise the phi would have a dangling predecessor reference)
            if (is_entry) {
                bool target_has_phi = false;
                for (auto& instr : target->instructions()) {
                    if (instr->opcode() == Opcode::Phi) {
                        target_has_phi = true;
                        break;
                    }
                }
                if (target_has_phi)
                    continue;
            } else if (predecessors.is_empty()) {
                continue;
            }

            // Update all predecessors to jump to target instead
            for (auto* pred : predecessors) {
                if (auto* pred_term = pred->terminator()) {
                    if (pred_term->true_target() == block.ptr())
                        pred_term->set_true_target(target);
                    if (pred_term->false_target() == block.ptr())
                        pred_term->set_false_target(target);
                }
            }

            // If eliminating the entry block, make target the new entry
            if (is_entry)
                function.set_entry_block(target);

            // Add each predecessor of the empty block to target with traced phi values
            for (auto* pred : predecessors) {
                CFG::add_predecessor(*target, *pred, [&](Instruction& phi_instr) -> Value* {
                    // Find the value this phi expects from the empty block
                    Value* value_from_empty = nullptr;
                    for (size_t i = 0; i < phi_instr.phi_predecessors().size(); ++i) {
                        if (phi_instr.phi_predecessors()[i] == block.ptr()) {
                            value_from_empty = phi_instr.operands()[i];
                            break;
                        }
                    }

                    if (!value_from_empty)
                        return nullptr;

                    // If value_from_empty is a phi, trace to find what this pred would contribute
                    if (auto* def = value_from_empty->defining_instruction();
                        def && def->opcode() == Opcode::Phi) {
                        for (size_t j = 0; j < def->phi_predecessors().size(); ++j) {
                            if (def->phi_predecessors()[j] == pred)
                                return def->operands()[j];
                        }
                    }

                    return value_from_empty;
                });
            }

            // Remove the empty block from target's predecessors (and phi operands)
            CFG::remove_predecessor(*target, *block);

            // Clear the block's instructions (will be removed later)
            block->instructions().clear();

            eliminated_any = true;
            changed = true;
            break; // Restart since we modified the CFG
        }
    } while (eliminated_any);

    // Collect blocks to remove
    HashTable<BasicBlock*> blocks_to_remove;
    for (auto& block : function.basic_blocks()) {
        if (block->instructions().is_empty())
            blocks_to_remove.set(block.ptr());
    }

    // Clean up ALL references to blocks being removed
    for (auto& block : function.basic_blocks()) {
        if (blocks_to_remove.contains(block.ptr()))
            continue;

        for (auto* removed : blocks_to_remove)
            CFG::remove_block_reference(*block, *removed);
    }

    // Remove empty blocks
    function.basic_blocks().remove_all_matching([](auto const& block) {
        return block->instructions().is_empty();
    });

    return changed;
}

}
