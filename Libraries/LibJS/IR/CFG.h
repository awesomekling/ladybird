/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
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

namespace CFG {

// Remove a predecessor from a block and update all phi nodes accordingly.
// This is the core operation that must be done atomically to maintain consistency.
JS_API void remove_predecessor(BasicBlock& block, BasicBlock& predecessor);

// Add a predecessor to a block and add corresponding phi operands.
// The `value_for_phi` callback is called for each phi node to determine
// what value should be used for the new predecessor's operand.
// Pass nullptr callback to add nullptr phi operands.
JS_API void add_predecessor(
    BasicBlock& block,
    BasicBlock& predecessor,
    AK::Function<Value*(Instruction&)> value_for_phi);

// Replace all occurrences of `old_pred` with `new_pred` in block's
// predecessor list and all phi nodes.
JS_API void replace_predecessor(BasicBlock& block, BasicBlock& old_pred, BasicBlock& new_pred);

// Redirect all edges from `from_block` that target `old_target` to instead
// target `new_target`. Updates:
// - Terminator targets in from_block
// - Predecessor lists (removes from_block from old_target, adds to new_target)
// - Phi operands (removes from old_target, adds to new_target with mapped values)
//
// The `value_mapper` callback transforms phi values from old_target to new_target.
// If null, phi operands in new_target will be nullptr.
JS_API void redirect_edge(
    BasicBlock& from_block,
    BasicBlock& old_target,
    BasicBlock& new_target,
    AK::Function<Value*(Value*)> value_mapper = nullptr);

// Remove all references to `block_to_remove` from `live_block`:
// - Removes block_to_remove from predecessor list
// - Clears terminator targets pointing to block_to_remove
// - Removes block_to_remove from all phi predecessors
// - Clears EH edges pointing to block_to_remove
JS_API void remove_block_reference(BasicBlock& live_block, BasicBlock& block_to_remove);

} // namespace CFG

} // namespace JS::IR
