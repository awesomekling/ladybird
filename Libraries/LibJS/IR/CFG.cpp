/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

void CFG::remove_predecessor(BasicBlock& block, BasicBlock& predecessor)
{
    // Remove phi operands for this predecessor first (uses existing helper)
    block.remove_phi_operands_for_predecessor(&predecessor);

    // Remove from predecessor list
    block.remove_predecessor(&predecessor);
}

void CFG::add_predecessor(BasicBlock& block, BasicBlock& predecessor, AK::Function<Value*(Instruction&)> value_for_phi)
{
    // Don't add duplicates
    if (block.predecessors().contains_slow(&predecessor))
        return;

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

void CFG::replace_predecessor(BasicBlock& block, BasicBlock& old_pred, BasicBlock& new_pred)
{
    // Use existing helper which updates both predecessor list and phi predecessors
    block.replace_phi_predecessor(&old_pred, &new_pred);

    // Also update the predecessor list
    block.remove_predecessor(&old_pred);
    block.add_predecessor(&new_pred);
}

void CFG::redirect_edge(BasicBlock& from_block, BasicBlock& old_target, BasicBlock& new_target, AK::Function<Value*(Instruction&, Value*)> value_mapper)
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

    // Collect phi values from old_target for this predecessor before removing.
    // Maps old phi instruction -> value that from_block contributed.
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

    // Add from_block to new_target with mapped phi values.
    // The mapper receives each phi in new_target and can determine the appropriate value.
    // NB: There's no automatic correspondence between old_target's phis and new_target's phis,
    // so we pass nullptr as the old_value. The mapper should use phi context to determine
    // the correct value, possibly by examining the old_phi_values captured above if needed.
    add_predecessor(new_target, from_block, [&](Instruction& phi) -> Value* {
        if (value_mapper) {
            // For now, pass nullptr as old_value since phi correspondence is context-dependent.
            // A more sophisticated implementation could try to match phis by result variable
            // or other criteria, but that requires domain-specific knowledge.
            return value_mapper(phi, nullptr);
        }
        return nullptr;
    });
}

void CFG::remove_block_reference(BasicBlock& live_block, BasicBlock& block_to_remove)
{
    // Remove from predecessor list (with phi updates)
    CFG::remove_predecessor(live_block, block_to_remove);

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

void CFG::set_exception_handler(BasicBlock& block, BasicBlock* handler)
{
    block.set_exception_handler(handler);
}

void CFG::set_finalizer(BasicBlock& block, BasicBlock* finalizer)
{
    block.set_finalizer(finalizer);
}

} // namespace JS::IR
