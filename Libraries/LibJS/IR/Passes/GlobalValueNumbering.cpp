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
#include <LibJS/IR/Passes/GlobalValueNumbering.h>
#include <LibJS/IR/Passes/PassManager.h>

namespace JS::IR {

PreservedAnalyses GlobalValueNumbering::run(Function& function, PassManager& pass_manager)
{
    if (!function.entry_block())
        return PreservedAnalyses::all();

    bool changed = false;
    auto const& dominators = pass_manager.dominator_tree(function);
    HashMap<ExpressionKey, ValueIndex> expressions;

    // Walk the dominator tree in pre-order with a scoped expression table.
    // When entering a block, expressions from all dominating blocks are
    // already in the table. When returning, we remove entries added by
    // that block to restore the table for sibling subtrees.
    auto process_block = [&](auto& self, BasicBlock* block) -> void {
        Vector<ExpressionKey> added_keys;

        block->for_each_instruction([&](Instruction const& instruction) {
            auto key = make_expression_key(instruction);
            if (!key.has_value())
                return;

            auto existing = expressions.get(*key);
            if (existing.has_value()) {
                auto* existing_value = function.values()[static_cast<u32>(*existing)].ptr();
                instruction.result()->replace_all_uses_with(existing_value);
                changed = true;
            } else {
                expressions.set(*key, instruction.result()->index());
                added_keys.append(*key);
            }
        });

        // Recurse into dominator tree children
        dominators.for_each_dominator_child(block, [&](BasicBlock& child) {
            self(self, &child);
        });

        // Remove expressions added by this block
        for (auto const& key : added_keys)
            expressions.remove(key);
    };

    process_block(process_block, function.entry_block());

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
