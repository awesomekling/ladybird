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
    block.for_each_phi([&](PhiInstruction& phi) {
        Value* value = value_for_phi ? value_for_phi(phi) : nullptr;
        phi.add_incoming(&predecessor, value);
    });
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

void CFG::redirect_edge(BasicBlock& from_block, BasicBlock& old_target, BasicBlock& new_target, AK::Function<Value*(Instruction&)> value_mapper)
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

    // Remove from_block from old_target
    remove_predecessor(old_target, from_block);

    // Add from_block to new_target with mapped phi values.
    add_predecessor(new_target, from_block, [&](Instruction& phi) -> Value* {
        if (value_mapper)
            return value_mapper(phi);
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
    // Remove the old branch and add a new jump instruction
    block.remove_terminator();
    block.append(JumpInstruction::create(target));

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

void CFG::remove_blocks(Function& function, HashTable<BasicBlock*> const& blocks_to_remove)
{
    if (blocks_to_remove.is_empty())
        return;

    // Clear operand uses in blocks being removed so use lists don't retain stale references.
    for (auto* block : blocks_to_remove)
        block->clear_instructions();

    // Remove all references to dead blocks from surviving blocks.
    for (auto& block : function.basic_blocks()) {
        if (blocks_to_remove.contains(block.ptr()))
            continue;
        for (auto* dead_block : blocks_to_remove)
            remove_block_reference(*block, *dead_block);
    }

    // Remove dead blocks from the function (and index map).
    function.basic_blocks().remove_all_matching([&](auto const& block) {
        if (!blocks_to_remove.contains(block.ptr()))
            return false;
        function.remove_block_index(block->index());
        return true;
    });
}

void CFG::for_each_successor(BasicBlock const& block, AK::Function<void(BasicBlock&)> callback)
{
    if (auto* term = block.terminator()) {
        if (auto* target = term->true_target())
            callback(*target);
        if (auto* target = term->false_target())
            callback(*target);
    }
    if (auto* handler = block.exception_handler())
        callback(*handler);
    if (auto* finalizer = block.finalizer())
        callback(*finalizer);
}

} // namespace JS::IR
