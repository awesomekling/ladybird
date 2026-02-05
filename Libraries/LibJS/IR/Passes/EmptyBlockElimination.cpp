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
                    for (auto& instr : target->instructions()) {
                        if (instr->opcode() != Opcode::Phi)
                            continue;

                        auto& phi = static_cast<PhiInstruction&>(*instr);
                        Value* value_from_empty = nullptr;
                        Value* value_from_direct = nullptr;

                        for (size_t i = 0; i < phi.incoming_count(); ++i) {
                            if (phi.incoming_block(i) == block.ptr())
                                value_from_empty = phi.incoming_value(i);
                            if (phi.incoming_block(i) == pred)
                                value_from_direct = phi.incoming_value(i);
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
            // and no other predecessors (otherwise it would become an entry block
            // with predecessors, violating the SSA invariant).
            if (is_entry) {
                bool can_eliminate = true;
                for (auto& instr : target->instructions()) {
                    if (instr->opcode() == Opcode::Phi) {
                        can_eliminate = false;
                        break;
                    }
                }
                // If the target has predecessors other than us, eliminating would
                // create an entry block with predecessors.
                for (auto* pred : target->predecessors()) {
                    if (pred != block.ptr()) {
                        can_eliminate = false;
                        break;
                    }
                }
                if (!can_eliminate)
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
                CFG::add_predecessor(*target, *pred, [&](Instruction& instr) -> Value* {
                    auto& target_phi = static_cast<PhiInstruction&>(instr);

                    // Find the value this phi expects from the empty block
                    Value* value_from_empty = nullptr;
                    for (size_t i = 0; i < target_phi.incoming_count(); ++i) {
                        if (target_phi.incoming_block(i) == block.ptr()) {
                            value_from_empty = target_phi.incoming_value(i);
                            break;
                        }
                    }

                    if (!value_from_empty)
                        return nullptr;

                    // If value_from_empty is a phi, trace to find what this pred would contribute
                    if (auto* def = value_from_empty->defining_instruction();
                        def && def->opcode() == Opcode::Phi) {
                        auto& def_phi = static_cast<PhiInstruction&>(*def);
                        for (size_t j = 0; j < def_phi.incoming_count(); ++j) {
                            if (def_phi.incoming_block(j) == pred)
                                return def_phi.incoming_value(j);
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
