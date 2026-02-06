/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <AK/Vector.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/GlobalValueNumbering.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

// Key for identifying equivalent expressions
struct ExpressionKey {
    Opcode opcode;
    Value* operand1 { nullptr };
    Value* operand2 { nullptr };
    u32 extra { 0 }; // For ExtractValue: extract_index

    bool operator==(ExpressionKey const&) const = default;
};

}

template<>
struct AK::Traits<JS::IR::ExpressionKey> : public DefaultTraits<JS::IR::ExpressionKey> {
    static unsigned hash(JS::IR::ExpressionKey const& key)
    {
        return pair_int_hash(
            pair_int_hash(static_cast<u8>(key.opcode), key.extra),
            pair_int_hash(ptr_hash(key.operand1), ptr_hash(key.operand2)));
    }
};

namespace JS::IR {

PreservedAnalyses GlobalValueNumbering::run(Function& function, PassManager& pass_manager)
{
    if (!function.entry_block())
        return PreservedAnalyses::all();

    bool changed = false;
    auto const& dominators = pass_manager.dominator_tree(function);
    HashMap<ExpressionKey, Value*> expressions;

    // Walk the dominator tree in pre-order with a scoped expression table.
    // When entering a block, expressions from all dominating blocks are
    // already in the table. When returning, we remove entries added by
    // that block to restore the table for sibling subtrees.
    auto process_block = [&](auto& self, BasicBlock* block) -> void {
        Vector<ExpressionKey> added_keys;

        for (auto const& instruction : block->instructions()) {
            if (!instruction->result())
                continue;

            // Never value-number Phi nodes. A Phi's result depends on which
            // predecessor edge was taken, not just its operand set.
            if (instruction->opcode() == Opcode::Phi)
                continue;

            if (!instruction->is_pure())
                continue;

            auto const& operands = instruction->operands();
            if (operands.is_empty())
                continue;

            ExpressionKey key;
            key.opcode = instruction->opcode();
            key.operand1 = operands.size() > 0 ? operands[0] : nullptr;
            key.operand2 = operands.size() > 1 ? operands[1] : nullptr;

            if (key.opcode == Opcode::ExtractValue)
                key.extra = instruction->extract_index();

            // Normalize operand order for commutative operations
            if (is_commutative_opcode(key.opcode) && key.operand1 && key.operand2) {
                if (key.operand1->index() > key.operand2->index())
                    swap(key.operand1, key.operand2);
            }

            auto existing = expressions.get(key);
            if (existing.has_value()) {
                instruction->result()->replace_all_uses_with(*existing);
                changed = true;
            } else {
                expressions.set(key, instruction->result());
                added_keys.append(key);
            }
        }

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
