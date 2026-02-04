/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
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

            auto* jump = block->instructions()[0].ptr();
            if (jump->opcode() != Opcode::Jump)
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
                for (auto& instr : pred->instructions()) {
                    if (instr->true_target() == target || instr->false_target() == target) {
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
                        if (would_conflict)
                            break;
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
                for (auto& instr : pred->instructions()) {
                    if (instr->true_target() == block.ptr())
                        instr->set_true_target(target);
                    if (instr->false_target() == block.ptr())
                        instr->set_false_target(target);
                }
            }

            // If eliminating the entry block, make target the new entry
            if (is_entry)
                function.set_entry_block(target);

            // Update phi nodes in the target block
            for (auto& instr : target->instructions()) {
                if (instr->opcode() != Opcode::Phi)
                    continue;

                // Find the value associated with the empty block
                Value* value_from_empty = nullptr;
                size_t empty_index = SIZE_MAX;

                for (size_t i = 0; i < instr->phi_predecessors().size(); ++i) {
                    if (instr->phi_predecessors()[i] == block.ptr()) {
                        value_from_empty = instr->operands()[i];
                        empty_index = i;
                        break;
                    }
                }

                if (empty_index == SIZE_MAX)
                    continue;

                // For each predecessor of the empty block, add a phi entry
                // But only if that predecessor doesn't already have an entry
                for (auto* pred : predecessors) {
                    bool already_has_entry = false;
                    for (auto* existing_pred : instr->phi_predecessors()) {
                        if (existing_pred == pred) {
                            already_has_entry = true;
                            break;
                        }
                    }
                    if (already_has_entry)
                        continue;

                    // Find the correct value for this predecessor
                    // If value_from_empty is defined by a phi in a predecessor-only block,
                    // we need to trace what value that phi would contribute for this specific predecessor
                    Value* value_for_pred = value_from_empty;
                    if (value_from_empty && value_from_empty->defining_instruction()) {
                        auto* def_instr = value_from_empty->defining_instruction();
                        if (def_instr->opcode() == Opcode::Phi) {
                            // The value is a phi - check if we can trace through to find
                            // what value this specific predecessor would contribute
                            for (size_t j = 0; j < def_instr->phi_predecessors().size(); ++j) {
                                if (def_instr->phi_predecessors()[j] == pred) {
                                    value_for_pred = def_instr->operands()[j];
                                    break;
                                }
                            }
                        }
                    }

                    instr->add_phi_operand(pred, value_for_pred);
                }

                // Remove the entry for the empty block
                instr->remove_phi_operand(empty_index);
            }

            // Update predecessor lists
            for (auto* pred : predecessors) {
                target->add_predecessor(pred);
            }
            target->remove_predecessor(block.ptr());

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

        // Clean up exception_handler and finalizer references
        if (block->exception_handler() && blocks_to_remove.contains(block->exception_handler()))
            block->set_exception_handler(nullptr);
        if (block->finalizer() && blocks_to_remove.contains(block->finalizer()))
            block->set_finalizer(nullptr);

        // Clean up predecessor lists
        for (auto* removed : blocks_to_remove)
            block->remove_predecessor(removed);

        // Clean up phi predecessors and terminator targets
        for (auto& instr : block->instructions()) {
            // Clean up terminator targets
            if (instr->true_target() && blocks_to_remove.contains(instr->true_target()))
                instr->set_true_target(nullptr);
            if (instr->false_target() && blocks_to_remove.contains(instr->false_target()))
                instr->set_false_target(nullptr);

            // Clean up phi predecessors (iterate backwards for safe removal)
            for (size_t i = instr->phi_predecessors().size(); i > 0; --i) {
                if (blocks_to_remove.contains(instr->phi_predecessors()[i - 1]))
                    instr->remove_phi_operand(i - 1);
            }
        }
    }

    // Remove empty blocks
    function.basic_blocks().remove_all_matching([](auto const& block) {
        return block->instructions().is_empty();
    });

    return changed;
}

}
