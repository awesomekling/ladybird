/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <AK/Vector.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Passes/ExpressionKey.h>
#include <LibJS/IR/Passes/PartialRedundancyElimination.h>
#include <LibJS/IR/Passes/PassManager.h>

namespace JS::IR {

// Returns true if the value is available at the exit of the given block.
// A value is available if it's a constant/parameter/this, or if its defining
// block dominates the given block.
static bool value_available_at_block(Value const* value, BasicBlock const* block, DominatorTree const& dominator_tree)
{
    if (!value)
        return false;
    if (value->is_constant() || value->is_parameter() || value->is_this())
        return true;
    auto* defining_block = value->defining_instruction()->parent_block();
    return dominator_tree.dominates(defining_block, block);
}

// Clone a pure 1- or 2-operand instruction with the same opcode and operands.
static NonnullOwnPtr<Instruction> clone_pure_instruction(Instruction const& original)
{
    if (original.operand_count() == 2)
        return BinaryOpInstruction::create(original.opcode(), original.operand(0), original.operand(1));
    if (original.operand_count() == 1)
        return UnaryOpInstruction::create(original.opcode(), original.operand(0));
    VERIFY_NOT_REACHED();
}

PreservedAnalyses PartialRedundancyElimination::run(Function& function, PassManager& pass_manager)
{
    if (!function.entry_block())
        return PreservedAnalyses::all();

    auto const& dominator_tree = pass_manager.dominator_tree(function);

    // Phase 1: Build per-block availability by walking the dominator tree.
    // avail_out[block] maps ExpressionKey -> ValueIndex of the available result.
    HashMap<BasicBlock const*, HashMap<ExpressionKey, ValueIndex>> avail_out;

    {
        HashMap<ExpressionKey, ValueIndex> scoped_expressions;

        auto build_availability = [&](auto& self, BasicBlock* block) -> void {
            Vector<ExpressionKey> added_keys;

            block->for_each_instruction([&](Instruction const& instruction) {
                auto key = make_expression_key(instruction);
                if (!key.has_value())
                    return;

                if (!scoped_expressions.contains(*key)) {
                    scoped_expressions.set(*key, instruction.result()->index());
                    added_keys.append(*key);
                }
            });

            // Snapshot current availability at this block's exit.
            avail_out.set(block, scoped_expressions);

            // Recurse into dominator tree children.
            dominator_tree.for_each_dominator_child(block, [&](BasicBlock& child) {
                self(self, &child);
            });

            // Remove expressions added by this block.
            for (auto const& key : added_keys)
                scoped_expressions.remove(key);
        };

        build_availability(build_availability, function.entry_block());
    }

    // Phase 2: Exploit partial redundancies at merge points.
    bool changed = false;

    for (auto* block : dominator_tree.reverse_postorder()) {
        auto const& predecessors = block->predecessor_indices();
        if (predecessors.size() < 2)
            continue;

        // Collect pure expressions in this block that we might eliminate.
        // Skip instructions with no uses (already dead from GVN).
        Vector<Instruction*> candidates;
        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.result() && instruction.result()->uses().is_empty())
                return;
            auto key = make_expression_key(instruction);
            if (key.has_value())
                candidates.append(const_cast<Instruction*>(&instruction));
        });

        for (auto* instruction : candidates) {
            // Re-derive the key. Earlier replacements in this block may have
            // changed operand types, making this instruction no longer pure.
            auto key = make_expression_key(*instruction);
            if (!key.has_value())
                continue;

            // For each predecessor, check if this expression is available.
            struct PredecessorInfo {
                BasicBlock* block;
                Value* available_value; // non-null if expression is available
            };
            Vector<PredecessorInfo> predecessor_info;
            BasicBlock* missing_predecessor = nullptr;
            size_t missing_count = 0;

            for (auto predecessor_index : predecessors) {
                auto* predecessor = function.block_by_index(predecessor_index);
                auto it = avail_out.find(predecessor);
                Value* available_value = nullptr;
                if (it != avail_out.end()) {
                    auto value_it = it->value.get(*key);
                    if (value_it.has_value())
                        available_value = function.values()[static_cast<u32>(*value_it)].ptr();
                }
                if (!available_value) {
                    missing_count++;
                    missing_predecessor = predecessor;
                }
                predecessor_info.append({ predecessor, available_value });
            }

            if (missing_count == 0) {
                // Fully redundant at merge: all predecessors have the expression.
                auto phi = PhiInstruction::create();
                auto& result = function.create_value_for_instruction();
                phi->set_result(&result);
                for (auto& info : predecessor_info)
                    phi->add_incoming(info.block->index(), info.available_value);
                block->prepend(move(phi));
                result.defining_instruction()->recompute_result_type();
                instruction->result()->replace_all_uses_with(&result);
                changed = true;
            } else if (missing_count == 1) {
                // Partially redundant: one predecessor is missing the expression.
                // Check that the missing predecessor has a single successor (no critical edge)
                // and that all operands are available there.
                auto* missing_terminator = missing_predecessor->terminator();
                if (!missing_terminator)
                    continue;

                // Skip if missing predecessor has multiple successors (critical edge).
                if (missing_terminator->false_target())
                    continue;

                // Check operand availability at the missing predecessor.
                bool operands_available = true;
                for (size_t i = 0; i < instruction->operand_count(); ++i) {
                    if (!value_available_at_block(instruction->operand(i), missing_predecessor, dominator_tree)) {
                        operands_available = false;
                        break;
                    }
                }
                if (!operands_available)
                    continue;

                // Insert the cloned instruction before the missing predecessor's terminator.
                auto cloned = clone_pure_instruction(*instruction);
                auto& cloned_result = function.create_value_for_instruction();
                cloned->set_result(&cloned_result);
                missing_predecessor->insert_before_terminator(move(cloned));
                cloned_result.defining_instruction()->recompute_result_type();

                // Build a phi merging the available values and the newly inserted one.
                auto phi = PhiInstruction::create();
                auto& phi_result = function.create_value_for_instruction();
                phi->set_result(&phi_result);
                for (auto& info : predecessor_info) {
                    if (info.block == missing_predecessor)
                        phi->add_incoming(info.block->index(), &cloned_result);
                    else
                        phi->add_incoming(info.block->index(), info.available_value);
                }
                block->prepend(move(phi));
                phi_result.defining_instruction()->recompute_result_type();
                instruction->result()->replace_all_uses_with(&phi_result);

                // Update avail_out for the missing predecessor.
                avail_out.ensure(missing_predecessor).set(*key, cloned_result.index());
                changed = true;
            }
            // Otherwise (missing_count > 1): too many insertions needed, skip.
        }
    }

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
