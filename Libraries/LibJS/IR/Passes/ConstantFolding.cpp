/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <AK/NumericLimits.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/ConstantFolding.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/Value.h>

namespace JS::IR {

bool ConstantFolding::run(Function& function)
{
    bool changed = false;

    // Map from values to their constant replacements
    HashMap<Value*, Value*> replacements;

    for (auto& block : function.basic_blocks()) {
        for (auto& instruction : block->instructions()) {
            // Check if all operands are constants
            bool all_constants = true;
            for (auto* operand : instruction->operands()) {
                if (!operand || !operand->is_constant()) {
                    all_constants = false;
                    break;
                }
            }

            if (!all_constants || !instruction->result())
                continue;

            // Try to fold the instruction
            JS::Value result_value;
            bool can_fold = false;

            auto const& operands = instruction->operands();

            // Helper to create a JS value from an i64 result, converting to double if overflow
            auto make_int_or_double = [](i64 result) -> JS::Value {
                if (result >= NumericLimits<i32>::min() && result <= NumericLimits<i32>::max())
                    return JS::Value(static_cast<i32>(result));
                return JS::Value(static_cast<double>(result));
            };

            switch (instruction->opcode()) {
            case Opcode::Add:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    i64 lhs = operands[0]->constant_value().as_i32();
                    i64 rhs = operands[1]->constant_value().as_i32();
                    result_value = make_int_or_double(lhs + rhs);
                    can_fold = true;
                }
                break;

            case Opcode::Sub:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    i64 lhs = operands[0]->constant_value().as_i32();
                    i64 rhs = operands[1]->constant_value().as_i32();
                    result_value = make_int_or_double(lhs - rhs);
                    can_fold = true;
                }
                break;

            case Opcode::Mul:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    i64 lhs = operands[0]->constant_value().as_i32();
                    i64 rhs = operands[1]->constant_value().as_i32();
                    result_value = make_int_or_double(lhs * rhs);
                    can_fold = true;
                }
                break;

            case Opcode::LessThan:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    result_value = JS::Value(operands[0]->constant_value().as_i32() < operands[1]->constant_value().as_i32());
                    can_fold = true;
                }
                break;

            case Opcode::LessThanEquals:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    result_value = JS::Value(operands[0]->constant_value().as_i32() <= operands[1]->constant_value().as_i32());
                    can_fold = true;
                }
                break;

            case Opcode::GreaterThan:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    result_value = JS::Value(operands[0]->constant_value().as_i32() > operands[1]->constant_value().as_i32());
                    can_fold = true;
                }
                break;

            case Opcode::GreaterThanEquals:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    result_value = JS::Value(operands[0]->constant_value().as_i32() >= operands[1]->constant_value().as_i32());
                    can_fold = true;
                }
                break;

            case Opcode::StrictlyEquals:
                if (operands.size() == 2) {
                    auto const& lhs = operands[0]->constant_value();
                    auto const& rhs = operands[1]->constant_value();
                    if (lhs.is_int32() && rhs.is_int32()) {
                        result_value = JS::Value(lhs.as_i32() == rhs.as_i32());
                        can_fold = true;
                    } else if (lhs.is_boolean() && rhs.is_boolean()) {
                        result_value = JS::Value(lhs.as_bool() == rhs.as_bool());
                        can_fold = true;
                    }
                }
                break;

            case Opcode::Not:
                if (operands.size() == 1 && operands[0]->constant_value().is_boolean()) {
                    result_value = JS::Value(!operands[0]->constant_value().as_bool());
                    can_fold = true;
                }
                break;

            case Opcode::Move:
                // Move of a constant is just the constant
                if (operands.size() == 1) {
                    replacements.set(instruction->result(), operands[0]);
                    changed = true;
                }
                continue;

            default:
                break;
            }

            if (can_fold) {
                // Create a constant value for the result
                auto& constant = function.create_constant(result_value);
                replacements.set(instruction->result(), &constant);
                changed = true;
            }
        }
    }

    // Replace uses of folded values
    if (!replacements.is_empty()) {
        for (auto& block : function.basic_blocks()) {
            for (auto& instruction : block->instructions()) {
                for (size_t i = 0; i < instruction->operands().size(); ++i) {
                    auto replacement = replacements.get(instruction->operands()[i]);
                    if (replacement.has_value()) {
                        instruction->set_operand(i, *replacement);
                    }
                }
            }
        }
    }

    return changed;
}

}
