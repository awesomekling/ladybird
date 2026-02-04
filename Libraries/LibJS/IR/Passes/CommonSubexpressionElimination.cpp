/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/CommonSubexpressionElimination.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

// Key for identifying equivalent expressions
struct ExpressionKey {
    Opcode opcode;
    Value* operand1 { nullptr };
    Value* operand2 { nullptr };

    bool operator==(ExpressionKey const&) const = default;
};

}

template<>
struct AK::Traits<JS::IR::ExpressionKey> : public DefaultTraits<JS::IR::ExpressionKey> {
    static unsigned hash(JS::IR::ExpressionKey const& key)
    {
        return pair_int_hash(
            static_cast<u8>(key.opcode),
            pair_int_hash(ptr_hash(key.operand1), ptr_hash(key.operand2)));
    }
};

namespace JS::IR {

static bool is_commutative(Opcode opcode)
{
    switch (opcode) {
    case Opcode::Add:
    case Opcode::Mul:
    case Opcode::BitwiseAnd:
    case Opcode::BitwiseOr:
    case Opcode::BitwiseXor:
    case Opcode::LooselyEquals:
    case Opcode::StrictlyEquals:
    case Opcode::LooselyInequals:
    case Opcode::StrictlyInequals:
        return true;
    default:
        return false;
    }
}

bool CommonSubexpressionElimination::run(Function& function)
{
    bool changed = false;

    // For now, do local CSE within each basic block
    // Global CSE would require dominance analysis
    for (auto& block : function.basic_blocks()) {
        HashMap<ExpressionKey, Value*> expressions;

        for (auto& instruction : block->instructions()) {
            if (!instruction->result())
                continue;

            if (!instruction->is_pure())
                continue;

            auto const& operands = instruction->operands();
            if (operands.is_empty())
                continue;

            // Build the expression key
            ExpressionKey key;
            key.opcode = instruction->opcode();
            key.operand1 = operands.size() > 0 ? operands[0] : nullptr;
            key.operand2 = operands.size() > 1 ? operands[1] : nullptr;

            // Normalize operand order for commutative operations
            if (is_commutative(key.opcode) && key.operand1 && key.operand2) {
                if (key.operand1->index() > key.operand2->index())
                    swap(key.operand1, key.operand2);
            }

            // Check if we've seen this expression before
            auto existing = expressions.get(key);
            if (existing.has_value()) {
                // Replace all uses of this result with the previous result
                instruction->result()->replace_all_uses_with(*existing);
                changed = true;
            } else {
                // Record this expression
                expressions.set(key, instruction->result());
            }
        }
    }

    return changed;
}

}
