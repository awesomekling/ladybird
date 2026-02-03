/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/DeadCodeElimination.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

static bool has_side_effects(Opcode opcode)
{
    switch (opcode) {
    // Pure arithmetic/logic - no side effects
    case Opcode::Add:
    case Opcode::Sub:
    case Opcode::Mul:
    case Opcode::Div:
    case Opcode::Mod:
    case Opcode::Exp:
    case Opcode::BitwiseAnd:
    case Opcode::BitwiseOr:
    case Opcode::BitwiseXor:
    case Opcode::BitwiseNot:
    case Opcode::LeftShift:
    case Opcode::RightShift:
    case Opcode::UnsignedRightShift:
    case Opcode::LessThan:
    case Opcode::LessThanEquals:
    case Opcode::GreaterThan:
    case Opcode::GreaterThanEquals:
    case Opcode::LooselyEquals:
    case Opcode::StrictlyEquals:
    case Opcode::LooselyInequals:
    case Opcode::StrictlyInequals:
    case Opcode::Not:
    case Opcode::Negate:
    case Opcode::UnaryPlus:
    case Opcode::Move:
    case Opcode::Phi:
    case Opcode::Increment:
    case Opcode::Decrement:
    case Opcode::PostfixIncrement:
    case Opcode::PostfixDecrement:
        return false;

    // Everything else potentially has side effects
    default:
        return true;
    }
}

bool DeadCodeElimination::run(Function& function)
{
    bool changed = false;

    // Collect all used values
    HashTable<Value*> used_values;

    for (auto const& block : function.basic_blocks()) {
        for (auto const& instruction : block->instructions()) {
            // All operands are used
            for (auto* operand : instruction->operands())
                used_values.set(operand);
        }
    }

    // Remove dead instructions
    for (auto& block : function.basic_blocks()) {
        auto& instructions = block->instructions();

        for (size_t i = instructions.size(); i > 0; --i) {
            auto& instruction = instructions[i - 1];

            // Skip instructions without results (terminators, etc.)
            if (!instruction->result())
                continue;

            // Skip instructions with side effects
            if (has_side_effects(instruction->opcode()))
                continue;

            // If the result is not used, remove the instruction
            if (!used_values.contains(instruction->result())) {
                instructions.remove(i - 1);
                changed = true;
            }
        }
    }

    return changed;
}

}
