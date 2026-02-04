/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>

namespace JS::IR::CFG {

void remove_predecessor(BasicBlock& block, BasicBlock& predecessor)
{
    // Remove phi operands for this predecessor first (uses existing helper)
    block.remove_phi_operands_for_predecessor(&predecessor);

    // Remove from predecessor list
    block.remove_predecessor(&predecessor);
}

void add_predecessor(BasicBlock& block, BasicBlock& predecessor, AK::Function<Value*(Instruction&)> value_for_phi)
{
    // Add to predecessor list
    block.add_predecessor(&predecessor);

    // Add corresponding phi operands
    for (auto& instruction : block.instructions()) {
        if (instruction->opcode() != Opcode::Phi)
            break; // Phis are always first

        Value* value = value_for_phi ? value_for_phi(*instruction) : nullptr;
        instruction->add_phi_operand(&predecessor, value);
    }
}

void replace_predecessor(BasicBlock& block, BasicBlock& old_pred, BasicBlock& new_pred)
{
    // Use existing helper which updates both predecessor list and phi predecessors
    block.replace_phi_predecessor(&old_pred, &new_pred);

    // Also update the predecessor list
    block.remove_predecessor(&old_pred);
    block.add_predecessor(&new_pred);
}

void redirect_edge(BasicBlock& from_block, BasicBlock& old_target, BasicBlock& new_target, AK::Function<Value*(Value*)> value_mapper)
{
    if (&old_target == &new_target)
        return;

    // Update terminator in from_block
    auto* terminator = from_block.terminator();
    if (terminator) {
        if (terminator->true_target() == &old_target)
            terminator->set_true_target(&new_target);
        if (terminator->false_target() == &old_target)
            terminator->set_false_target(&new_target);
    }

    // Collect phi values from old_target for this predecessor before removing
    HashMap<Instruction*, Value*> old_phi_values;
    for (auto& instruction : old_target.instructions()) {
        if (instruction->opcode() != Opcode::Phi)
            break;

        for (size_t i = 0; i < instruction->phi_predecessors().size(); ++i) {
            if (instruction->phi_predecessors()[i] == &from_block) {
                old_phi_values.set(instruction.ptr(), instruction->operands()[i]);
                break;
            }
        }
    }

    // Remove from_block from old_target
    remove_predecessor(old_target, from_block);

    // Add from_block to new_target with mapped phi values
    add_predecessor(new_target, from_block, [&](Instruction&) -> Value* {
        if (value_mapper) {
            // Use first available old phi value (caller provides mapping logic)
            for (auto& [old_phi, old_value] : old_phi_values) {
                if (old_value)
                    return value_mapper(old_value);
            }
            return value_mapper(nullptr);
        }
        return nullptr;
    });
}

void remove_block_reference(BasicBlock& live_block, BasicBlock& block_to_remove)
{
    // Remove from predecessor list (with phi updates)
    remove_predecessor(live_block, block_to_remove);

    // Clear terminator targets
    auto* terminator = live_block.terminator();
    if (terminator) {
        if (terminator->true_target() == &block_to_remove)
            terminator->set_true_target(nullptr);
        if (terminator->false_target() == &block_to_remove)
            terminator->set_false_target(nullptr);
    }

    // Clear EH edges
    if (live_block.exception_handler() == &block_to_remove)
        live_block.set_exception_handler(nullptr);
    if (live_block.finalizer() == &block_to_remove)
        live_block.set_finalizer(nullptr);
}

} // namespace JS::IR::CFG
