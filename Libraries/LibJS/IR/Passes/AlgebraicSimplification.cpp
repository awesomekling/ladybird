/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/AlgebraicSimplification.h>
#include <LibJS/IR/Type.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

static bool is_constant_zero(Value* value)
{
    if (!value || !value->is_constant())
        return false;
    auto const& v = value->constant_value();
    if (v.is_int32())
        return v.as_i32() == 0;
    if (v.is_double())
        return v.as_double() == 0.0;
    return false;
}

static bool is_constant_one(Value* value)
{
    if (!value || !value->is_constant())
        return false;
    auto const& v = value->constant_value();
    if (v.is_int32())
        return v.as_i32() == 1;
    if (v.is_double())
        return v.as_double() == 1.0;
    return false;
}

// Is this type known to be numeric (not String, Object, etc)?
// Used to guard simplifications that would change semantics for non-numeric types.
static bool is_numeric_type(Type type)
{
    switch (type) {
    case Type::Int32:
    case Type::Number:
        return true;
    default:
        return false;
    }
}

bool AlgebraicSimplification::run(Function& function)
{
    bool changed = false;

    for (auto& block : function.basic_blocks()) {
        for (auto& instruction : block->instructions()) {
            if (!instruction->result())
                continue;

            auto const& operands = instruction->operands();
            Value* replacement = nullptr;

            switch (instruction->opcode()) {
            case Opcode::Add:
                // x + 0 → x, 0 + x → x (only for numeric types, since string + 0 = "0")
                if (operands.size() == 2) {
                    if (is_constant_zero(operands[1]) && is_numeric_type(operands[0]->type()))
                        replacement = operands[0];
                    else if (is_constant_zero(operands[0]) && is_numeric_type(operands[1]->type()))
                        replacement = operands[1];
                }
                break;

            case Opcode::Sub:
                // x - 0 → x
                if (operands.size() == 2) {
                    if (is_constant_zero(operands[1]))
                        replacement = operands[0];
                    // x - x → 0 (only safe for Int32, since NaN - NaN = NaN)
                    else if (operands[0] == operands[1] && operands[0]->type() == Type::Int32)
                        replacement = &function.create_constant(JS::Value(0));
                }
                break;

            case Opcode::Mul:
                // x * 0 → 0, 0 * x → 0 (only safe for Int32, since NaN * 0 = NaN)
                if (operands.size() == 2) {
                    if ((is_constant_zero(operands[0]) && operands[1]->type() == Type::Int32)
                        || (is_constant_zero(operands[1]) && operands[0]->type() == Type::Int32))
                        replacement = &function.create_constant(JS::Value(0));
                    // x * 1 → x, 1 * x → x (only for numeric types, since "5" * 1 = 5)
                    else if (is_constant_one(operands[1]) && is_numeric_type(operands[0]->type()))
                        replacement = operands[0];
                    else if (is_constant_one(operands[0]) && is_numeric_type(operands[1]->type()))
                        replacement = operands[1];
                }
                break;

            case Opcode::Div:
                // x / 1 → x (only for numeric types, since "5" / 1 = 5)
                if (operands.size() == 2) {
                    if (is_constant_one(operands[1]) && is_numeric_type(operands[0]->type()))
                        replacement = operands[0];
                }
                break;

            case Opcode::BitwiseAnd:
                // x & 0 → 0 (always safe, result is always 0)
                if (operands.size() == 2) {
                    if (is_constant_zero(operands[0]) || is_constant_zero(operands[1]))
                        replacement = &function.create_constant(JS::Value(0));
                    // x & x → x (only if x is already Int32, since bitwise ops coerce to Int32)
                    else if (operands[0] == operands[1] && operands[0]->type() == Type::Int32)
                        replacement = operands[0];
                }
                break;

            case Opcode::BitwiseOr:
                // x | 0 → x, 0 | x → x (only if x is already Int32)
                if (operands.size() == 2) {
                    if (is_constant_zero(operands[1]) && operands[0]->type() == Type::Int32)
                        replacement = operands[0];
                    else if (is_constant_zero(operands[0]) && operands[1]->type() == Type::Int32)
                        replacement = operands[1];
                    // x | x → x (only if x is already Int32)
                    else if (operands[0] == operands[1] && operands[0]->type() == Type::Int32)
                        replacement = operands[0];
                }
                break;

            case Opcode::BitwiseXor:
                // x ^ 0 → x, 0 ^ x → x (only if x is already Int32)
                if (operands.size() == 2) {
                    if (is_constant_zero(operands[1]) && operands[0]->type() == Type::Int32)
                        replacement = operands[0];
                    else if (is_constant_zero(operands[0]) && operands[1]->type() == Type::Int32)
                        replacement = operands[1];
                    // x ^ x → 0 (always safe, result is always 0)
                    else if (operands[0] == operands[1])
                        replacement = &function.create_constant(JS::Value(0));
                }
                break;

            case Opcode::LeftShift:
            case Opcode::RightShift:
            case Opcode::UnsignedRightShift:
                // x << 0 → x, x >> 0 → x, x >>> 0 → x (only if x is already Int32)
                if (operands.size() == 2) {
                    if (is_constant_zero(operands[1]) && operands[0]->type() == Type::Int32)
                        replacement = operands[0];
                }
                break;

            default:
                break;
            }

            if (replacement) {
                // Replace all uses of the result with the simplified value
                instruction->result()->replace_all_uses_with(replacement);
                changed = true;
            }
        }
    }

    return changed;
}

}
