/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
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

        auto& phi = static_cast<PhiInstruction&>(*instruction);
        Value* value = value_for_phi ? value_for_phi(*instruction) : nullptr;
        phi.add_incoming(&predecessor, value);
    }
}

void CFG::replace_predecessor(BasicBlock& block, BasicBlock& old_pred, BasicBlock& new_pred)
{
    if (&old_pred == &new_pred)
        return;

    // Only replace if old_pred is actually a predecessor of this block
    if (!block.predecessors().contains_slow(&old_pred))
        return;

    // If new_pred is already a predecessor, just remove old_pred entirely.
    // Replacing would create duplicate phi entries for new_pred.
    if (block.predecessors().contains_slow(&new_pred)) {
        remove_predecessor(block, old_pred);
        return;
    }

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

        auto& phi = static_cast<PhiInstruction&>(*instruction);
        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            if (phi.incoming_block(i) == &from_block) {
                old_phi_values.set(instruction.ptr(), phi.incoming_value(i));
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

void CFG::replace_branch_with_jump(BasicBlock& block, BasicBlock& target, BasicBlock* not_taken)
{
    // Clean up use lists and remove the old branch instruction
    block.instructions().last()->clear_operand_uses();
    block.instructions().remove(block.instructions().size() - 1);

    // Add a new jump instruction
    auto jump = JumpInstruction::create(target);
    jump->set_parent_block(&block);
    block.instructions().append(move(jump));

    // Remove this block from the not-taken block's predecessors
    if (not_taken)
        CFG::remove_predecessor(*not_taken, block);
}

void CFG::swap_branch_targets(BasicBlock& block)
{
    auto* terminator = block.terminator();
    VERIFY(terminator);
    VERIFY(terminator->opcode() == Opcode::Branch);

    auto* true_target = terminator->true_target();
    auto* false_target = terminator->false_target();
    terminator->set_true_target(false_target);
    terminator->set_false_target(true_target);
}

void CFG::retarget_all_edges(Function& function, BasicBlock& old_target, BasicBlock& new_target)
{
    if (&old_target == &new_target)
        return;

    for (auto& block : function.basic_blocks()) {
        auto* terminator = block->terminator();
        if (!terminator)
            continue;

        if (terminator->true_target() == &old_target)
            terminator->set_true_target(&new_target);
        if (terminator->false_target() == &old_target)
            terminator->set_false_target(&new_target);
    }

    // Update predecessor lists and phi references
    for (auto& block : function.basic_blocks())
        CFG::replace_predecessor(*block, old_target, new_target);
}

} // namespace JS::IR
