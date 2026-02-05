/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/HashTable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

// Centralized CFG mutation helpers that ensure consistency between:
// - Predecessor lists
// - Phi node operands
// - Terminator targets
// - Exception handler edges
//
// Using these helpers prevents bugs where one data structure is updated
// but another is left stale.
//
// BasicBlock's CFG mutation methods are private and only accessible through
// this class, enforcing that all CFG changes go through these helpers.

class JS_API CFG {
public:
    // Remove a predecessor from a block and update all phi nodes accordingly.
    // This is the core operation that must be done atomically to maintain consistency.
    static void remove_predecessor(BasicBlock& block, BasicBlock& predecessor);

    // Add a predecessor to a block and add corresponding phi operands.
    // The `value_for_phi` callback is called for each phi node to determine
    // what value should be used for the new predecessor's operand.
    // Pass nullptr callback to add nullptr phi operands.
    static void add_predecessor(
        BasicBlock& block,
        BasicBlock& predecessor,
        AK::Function<Value*(Instruction&)> value_for_phi = nullptr);

    // Replace all occurrences of `old_pred` with `new_pred` in block's
    // predecessor list and all phi nodes.
    static void replace_predecessor(BasicBlock& block, BasicBlock& old_pred, BasicBlock& new_pred);

    // Redirect all edges from `from_block` that target `old_target` to instead
    // target `new_target`. Updates:
    // - Terminator targets in from_block
    // - Predecessor lists (removes from_block from old_target, adds to new_target)
    // - Phi operands (removes from old_target, adds to new_target with mapped values)
    //
    // The `value_mapper` callback is called for each phi in new_target with:
    // - The phi instruction that needs a value
    // - The old value from old_target's corresponding phi (if any, otherwise nullptr)
    // The mapper should return the value to use for this phi's new predecessor operand.
    // If value_mapper is null, phi operands in new_target will be nullptr.
    static void redirect_edge(
        BasicBlock& from_block,
        BasicBlock& old_target,
        BasicBlock& new_target,
        AK::Function<Value*(Instruction& phi, Value* old_value)> value_mapper = nullptr);

    // Remove all references to `block_to_remove` from `live_block`:
    // - Removes block_to_remove from predecessor list
    // - Clears terminator targets pointing to block_to_remove
    // - Removes block_to_remove from all phi predecessors
    // - Clears EH edges pointing to block_to_remove
    static void remove_block_reference(BasicBlock& live_block, BasicBlock& block_to_remove);

    // Set the exception handler for a block.
    static void set_exception_handler(BasicBlock& block, BasicBlock* handler);

    // Set the finalizer for a block.
    static void set_finalizer(BasicBlock& block, BasicBlock* finalizer);

    // Replace a Branch terminator with an unconditional Jump to `target`.
    // Removes the branch's operand uses, replaces the instruction, and removes
    // `block` from the not-taken target's predecessor list.
    static void replace_branch_with_jump(BasicBlock& block, BasicBlock& target, BasicBlock* not_taken);

    // Swap the true and false targets of a Branch terminator.
    // No predecessor changes needed (both targets remain connected).
    static void swap_branch_targets(BasicBlock& block);

    // Retarget all edges in `function` that point to `old_target` to instead
    // point to `new_target`. Updates terminator targets and predecessor lists
    // in all blocks.
    static void retarget_all_edges(Function& function, BasicBlock& old_target, BasicBlock& new_target);

    // Remove blocks from the function, cleaning up all references.
    // Handles: clearing operand uses, removing all references from surviving
    // blocks (predecessors, phi operands, terminator targets, EH edges),
    // and removing the blocks from function.basic_blocks().
    // Precondition: the caller has already redirected edges as needed.
    static void remove_blocks(Function& function, HashTable<BasicBlock*> const& blocks_to_remove);

    // Visit each successor of a block (terminator targets + exception handler + finalizer).
    // The callback is called once per non-null edge. A block may be visited
    // multiple times if multiple edges point to it.
    static void for_each_successor(BasicBlock const& block, AK::Function<void(BasicBlock&)> callback);
};

} // namespace JS::IR
