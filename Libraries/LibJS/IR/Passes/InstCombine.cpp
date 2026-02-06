/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <AK/NumericLimits.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/InstCombine.h>
#include <LibJS/IR/Type.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/AbstractOperations.h>
#include <LibJS/Runtime/PrimitiveString.h>
#include <LibJS/Runtime/VM.h>
#include <LibJS/Runtime/Value.h>
#include <math.h>

namespace JS::IR {

static GC::Ptr<PrimitiveString> typeof_result_for_type(Type type)
{
    auto& vm = VM::the();
    switch (type) {
    case Type::Int32:
    case Type::Number:
        return vm.cached_strings.number;
    case Type::Boolean:
        return vm.cached_strings.boolean;
    case Type::Undefined:
        return vm.cached_strings.undefined;
    case Type::Null:
        return vm.cached_strings.object;
    case Type::String:
        return vm.cached_strings.string;
    case Type::Symbol:
        return vm.cached_strings.symbol;
    case Type::BigInt:
        return vm.cached_strings.bigint;
    case Type::Function:
        return vm.cached_strings.function;
    case Type::Array:
        return vm.cached_strings.object;
    default:
        return nullptr;
    }
}

// Check if a type is guaranteed to never be NaN when used in numeric comparisons.
static bool type_cannot_be_nan(Type type)
{
    return type == Type::Int32 || type == Type::Boolean;
}

// Returns the inverted comparison opcode if safe to invert.
static Optional<Opcode> inverted_comparison_if_safe(Instruction const& cmp_instr)
{
    auto inverted = inverted_comparison_opcode(cmp_instr.opcode());
    if (!inverted.has_value())
        return {};

    bool is_relational = cmp_instr.opcode() == Opcode::LessThan
        || cmp_instr.opcode() == Opcode::LessThanEquals
        || cmp_instr.opcode() == Opcode::GreaterThan
        || cmp_instr.opcode() == Opcode::GreaterThanEquals;

    if (is_relational) {
        if (cmp_instr.operand_count() < 2)
            return {};
        bool lhs_safe = cmp_instr.operand(0) && type_cannot_be_nan(cmp_instr.operand(0)->type());
        bool rhs_safe = cmp_instr.operand(1) && type_cannot_be_nan(cmp_instr.operand(1)->type());
        if (!lhs_safe || !rhs_safe)
            return {};
    }

    return inverted;
}

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

static JS::Value make_int_or_double(i64 result)
{
    if (result >= NumericLimits<i32>::min() && result <= NumericLimits<i32>::max())
        return JS::Value(static_cast<i32>(result));
    return JS::Value(static_cast<double>(result));
}

static Optional<double> numeric_to_double(JS::Value v)
{
    if (v.is_int32())
        return static_cast<double>(v.as_i32());
    if (v.is_double())
        return v.as_double();
    return {};
}

static bool both_numeric(Value* a, Value* b)
{
    return a && b
        && (a->constant_value().is_int32() || a->constant_value().is_double())
        && (b->constant_value().is_int32() || b->constant_value().is_double());
}

static Optional<i32> constant_to_i32(JS::Value v)
{
    if (v.is_int32())
        return v.as_i32();
    if (v.is_double()) {
        double d = v.as_double();
        if (!isfinite(d) || d == 0.0)
            return static_cast<i32>(0);
        auto int_val = floor(fabs(d));
        if (signbit(d))
            int_val = -int_val;
        auto int32bit = modulo(int_val, 4294967296.0);
        if (int32bit >= 2147483648.0)
            int32bit -= 4294967296.0;
        return static_cast<i32>(int32bit);
    }
    if (v.is_boolean())
        return v.as_bool() ? 1 : 0;
    if (v.is_null() || v.is_undefined())
        return static_cast<i32>(0);
    return {};
}

// Try to constant-fold an instruction where all operands are constants.
// Returns the folded JS::Value and sets can_fold to true on success.
static JS::Value try_constant_fold(Instruction& instruction, Function& function, bool& can_fold)
{
    // Check if all operands are constants
    for (size_t i = 0; i < instruction.operand_count(); ++i) {
        if (!instruction.operand(i) || !instruction.operand(i)->is_constant())
            return {};
    }

    if (!instruction.result())
        return {};

    // Skip if the result has no uses
    if (instruction.result()->uses().is_empty())
        return {};

    // Local helpers for concise constant-fold patterns.
    auto op = [&](size_t i) -> Value* { return instruction.operand(i); };
    auto nops = instruction.operand_count();

    JS::Value result_value;

    switch (instruction.opcode()) {
    case Opcode::Add:
        if (nops == 2 && op(0)->constant_value().is_int32() && op(1)->constant_value().is_int32()) {
            i64 lhs = op(0)->constant_value().as_i32();
            i64 rhs = op(1)->constant_value().as_i32();
            result_value = make_int_or_double(lhs + rhs);
            can_fold = true;
        } else if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(*numeric_to_double(op(0)->constant_value()) + *numeric_to_double(op(1)->constant_value()));
            can_fold = true;
        }
        break;

    case Opcode::Sub:
        if (nops == 2 && op(0)->constant_value().is_int32() && op(1)->constant_value().is_int32()) {
            i64 lhs = op(0)->constant_value().as_i32();
            i64 rhs = op(1)->constant_value().as_i32();
            result_value = make_int_or_double(lhs - rhs);
            can_fold = true;
        } else if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(*numeric_to_double(op(0)->constant_value()) - *numeric_to_double(op(1)->constant_value()));
            can_fold = true;
        }
        break;

    case Opcode::Mul:
        if (nops == 2 && op(0)->constant_value().is_int32() && op(1)->constant_value().is_int32()) {
            i64 lhs = op(0)->constant_value().as_i32();
            i64 rhs = op(1)->constant_value().as_i32();
            result_value = make_int_or_double(lhs * rhs);
            can_fold = true;
        } else if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(*numeric_to_double(op(0)->constant_value()) * *numeric_to_double(op(1)->constant_value()));
            can_fold = true;
        }
        break;

    case Opcode::Div:
        if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(*numeric_to_double(op(0)->constant_value()) / *numeric_to_double(op(1)->constant_value()));
            can_fold = true;
        }
        break;

    case Opcode::Mod:
        if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(fmod(*numeric_to_double(op(0)->constant_value()), *numeric_to_double(op(1)->constant_value())));
            can_fold = true;
        }
        break;

    case Opcode::Exp:
        if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(pow(*numeric_to_double(op(0)->constant_value()), *numeric_to_double(op(1)->constant_value())));
            can_fold = true;
        }
        break;

    case Opcode::LessThan:
        if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(*numeric_to_double(op(0)->constant_value()) < *numeric_to_double(op(1)->constant_value()));
            can_fold = true;
        }
        break;

    case Opcode::LessThanEquals:
        if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(*numeric_to_double(op(0)->constant_value()) <= *numeric_to_double(op(1)->constant_value()));
            can_fold = true;
        }
        break;

    case Opcode::GreaterThan:
        if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(*numeric_to_double(op(0)->constant_value()) > *numeric_to_double(op(1)->constant_value()));
            can_fold = true;
        }
        break;

    case Opcode::GreaterThanEquals:
        if (nops == 2 && both_numeric(op(0), op(1))) {
            result_value = JS::Value(*numeric_to_double(op(0)->constant_value()) >= *numeric_to_double(op(1)->constant_value()));
            can_fold = true;
        }
        break;

    case Opcode::StrictlyEquals:
    case Opcode::StrictlyInequals:
        if (nops == 2) {
            auto const& lhs = op(0)->constant_value();
            auto const& rhs = op(1)->constant_value();
            bool equals = false;
            bool can_determine = false;

            if (both_numeric(op(0), op(1))) {
                double l = *numeric_to_double(lhs);
                double r = *numeric_to_double(rhs);
                equals = l == r;
                can_determine = true;
            } else if (lhs.is_boolean() && rhs.is_boolean()) {
                equals = lhs.as_bool() == rhs.as_bool();
                can_determine = true;
            } else if ((lhs.is_null() && rhs.is_null()) || (lhs.is_undefined() && rhs.is_undefined())) {
                equals = true;
                can_determine = true;
            } else if (lhs.is_string() && rhs.is_string()) {
                equals = lhs.as_string().utf8_string_view() == rhs.as_string().utf8_string_view();
                can_determine = true;
            } else if ((lhs.is_null() || lhs.is_undefined() || lhs.is_boolean() || lhs.is_number())
                && (rhs.is_null() || rhs.is_undefined() || rhs.is_boolean() || rhs.is_number())) {
                equals = false;
                can_determine = true;
            }

            if (can_determine) {
                bool result = (instruction.opcode() == Opcode::StrictlyEquals) ? equals : !equals;
                result_value = JS::Value(result);
                can_fold = true;
            }
        }
        break;

    case Opcode::LooselyEquals:
    case Opcode::LooselyInequals:
        if (nops == 2) {
            auto const& lhs = op(0)->constant_value();
            auto const& rhs = op(1)->constant_value();
            bool equals = false;
            bool can_determine = false;

            auto is_nullish = [](JS::Value v) { return v.is_null() || v.is_undefined(); };

            if (is_nullish(lhs) && is_nullish(rhs)) {
                equals = true;
                can_determine = true;
            } else if (is_nullish(lhs) || is_nullish(rhs)) {
                equals = false;
                can_determine = true;
            } else if (both_numeric(op(0), op(1))) {
                equals = *numeric_to_double(lhs) == *numeric_to_double(rhs);
                can_determine = true;
            } else if (lhs.is_boolean() && rhs.is_boolean()) {
                equals = lhs.as_bool() == rhs.as_bool();
                can_determine = true;
            } else if (lhs.is_string() && rhs.is_string()) {
                equals = lhs.as_string().utf8_string_view() == rhs.as_string().utf8_string_view();
                can_determine = true;
            } else if (lhs.is_boolean() && (rhs.is_int32() || rhs.is_double())) {
                equals = static_cast<double>(lhs.as_bool() ? 1 : 0) == *numeric_to_double(rhs);
                can_determine = true;
            } else if ((lhs.is_int32() || lhs.is_double()) && rhs.is_boolean()) {
                equals = *numeric_to_double(lhs) == static_cast<double>(rhs.as_bool() ? 1 : 0);
                can_determine = true;
            }

            if (can_determine) {
                bool result = (instruction.opcode() == Opcode::LooselyEquals) ? equals : !equals;
                result_value = JS::Value(result);
                can_fold = true;
            }
        }
        break;

    case Opcode::BitwiseAnd:
        if (nops == 2) {
            auto lhs = constant_to_i32(op(0)->constant_value());
            auto rhs = constant_to_i32(op(1)->constant_value());
            if (lhs.has_value() && rhs.has_value()) {
                result_value = JS::Value(*lhs & *rhs);
                can_fold = true;
            }
        }
        break;

    case Opcode::BitwiseOr:
        if (nops == 2) {
            auto lhs = constant_to_i32(op(0)->constant_value());
            auto rhs = constant_to_i32(op(1)->constant_value());
            if (lhs.has_value() && rhs.has_value()) {
                result_value = JS::Value(*lhs | *rhs);
                can_fold = true;
            }
        }
        break;

    case Opcode::BitwiseXor:
        if (nops == 2) {
            auto lhs = constant_to_i32(op(0)->constant_value());
            auto rhs = constant_to_i32(op(1)->constant_value());
            if (lhs.has_value() && rhs.has_value()) {
                result_value = JS::Value(*lhs ^ *rhs);
                can_fold = true;
            }
        }
        break;

    case Opcode::BitwiseNot:
        if (nops == 1) {
            auto val = constant_to_i32(op(0)->constant_value());
            if (val.has_value()) {
                result_value = JS::Value(~*val);
                can_fold = true;
            }
        }
        break;

    case Opcode::LeftShift:
        if (nops == 2) {
            auto lhs = constant_to_i32(op(0)->constant_value());
            auto rhs = constant_to_i32(op(1)->constant_value());
            if (lhs.has_value() && rhs.has_value()) {
                u32 shift_count = static_cast<u32>(*rhs) & 0x1f;
                result_value = JS::Value(*lhs << shift_count);
                can_fold = true;
            }
        }
        break;

    case Opcode::RightShift:
        if (nops == 2) {
            auto lhs = constant_to_i32(op(0)->constant_value());
            auto rhs = constant_to_i32(op(1)->constant_value());
            if (lhs.has_value() && rhs.has_value()) {
                u32 shift_count = static_cast<u32>(*rhs) & 0x1f;
                result_value = JS::Value(*lhs >> shift_count);
                can_fold = true;
            }
        }
        break;

    case Opcode::UnsignedRightShift:
        if (nops == 2) {
            auto lhs = constant_to_i32(op(0)->constant_value());
            auto rhs = constant_to_i32(op(1)->constant_value());
            if (lhs.has_value() && rhs.has_value()) {
                u32 unsigned_lhs = static_cast<u32>(*lhs);
                u32 shift_count = static_cast<u32>(*rhs) & 0x1f;
                u32 result = unsigned_lhs >> shift_count;
                if (result <= static_cast<u32>(NumericLimits<i32>::max()))
                    result_value = JS::Value(static_cast<i32>(result));
                else
                    result_value = JS::Value(static_cast<double>(result));
                can_fold = true;
            }
        }
        break;

    case Opcode::Negate:
        if (nops == 1) {
            auto const& v = op(0)->constant_value();
            if (v.is_int32()) {
                i32 val = v.as_i32();
                if (val == 0)
                    result_value = JS::Value(-0.0);
                else
                    result_value = make_int_or_double(-static_cast<i64>(val));
                can_fold = true;
            } else if (v.is_double()) {
                result_value = JS::Value(-v.as_double());
                can_fold = true;
            } else if (v.is_boolean()) {
                result_value = v.as_bool() ? JS::Value(-1) : JS::Value(-0.0);
                can_fold = true;
            } else if (v.is_null()) {
                result_value = JS::Value(-0.0);
                can_fold = true;
            } else if (v.is_undefined()) {
                result_value = JS::Value(js_nan());
                can_fold = true;
            }
        }
        break;

    case Opcode::UnaryPlus:
        if (nops == 1) {
            auto const& v = op(0)->constant_value();
            if (v.is_int32() || v.is_double()) {
                result_value = v;
                can_fold = true;
            } else if (v.is_boolean()) {
                result_value = JS::Value(v.as_bool() ? 1 : 0);
                can_fold = true;
            } else if (v.is_null()) {
                result_value = JS::Value(0);
                can_fold = true;
            } else if (v.is_undefined()) {
                result_value = JS::Value(js_nan());
                can_fold = true;
            }
        }
        break;

    case Opcode::Increment:
        if (nops == 1) {
            auto const& v = op(0)->constant_value();
            if (v.is_int32()) {
                result_value = make_int_or_double(static_cast<i64>(v.as_i32()) + 1);
                can_fold = true;
            } else if (v.is_double()) {
                result_value = JS::Value(v.as_double() + 1);
                can_fold = true;
            }
        }
        break;

    case Opcode::Decrement:
        if (nops == 1) {
            auto const& v = op(0)->constant_value();
            if (v.is_int32()) {
                result_value = make_int_or_double(static_cast<i64>(v.as_i32()) - 1);
                can_fold = true;
            } else if (v.is_double()) {
                result_value = JS::Value(v.as_double() - 1);
                can_fold = true;
            }
        }
        break;

    case Opcode::ToInt32:
        if (nops == 1) {
            auto val = constant_to_i32(op(0)->constant_value());
            if (val.has_value()) {
                result_value = JS::Value(*val);
                can_fold = true;
            }
        }
        break;

    case Opcode::ToNumber:
        if (nops == 1) {
            auto const& v = op(0)->constant_value();
            if (v.is_int32() || v.is_double()) {
                result_value = v;
                can_fold = true;
            } else if (v.is_boolean()) {
                result_value = JS::Value(v.as_bool() ? 1 : 0);
                can_fold = true;
            } else if (v.is_undefined()) {
                result_value = JS::Value(js_nan());
                can_fold = true;
            } else if (v.is_null()) {
                result_value = JS::Value(0);
                can_fold = true;
            }
        }
        break;

    case Opcode::ToNumeric:
        if (nops == 1) {
            auto const& v = op(0)->constant_value();
            if (v.is_int32() || v.is_double() || v.is_bigint()) {
                result_value = v;
                can_fold = true;
            } else if (v.is_boolean()) {
                result_value = JS::Value(v.as_bool() ? 1 : 0);
                can_fold = true;
            } else if (v.is_undefined()) {
                result_value = JS::Value(js_nan());
                can_fold = true;
            } else if (v.is_null()) {
                result_value = JS::Value(0);
                can_fold = true;
            }
        }
        break;

    case Opcode::ToBoolean:
        if (nops == 1) {
            if (auto truthiness = op(0)->constant_truthiness(); truthiness.has_value()) {
                result_value = JS::Value(*truthiness);
                can_fold = true;
            }
        }
        break;

    case Opcode::Not:
        if (nops == 1) {
            if (auto truthiness = op(0)->constant_truthiness(); truthiness.has_value()) {
                result_value = JS::Value(!*truthiness);
                can_fold = true;
            }
        }
        break;

    case Opcode::IsUndefined:
        if (nops == 1) {
            result_value = JS::Value(op(0)->constant_value().is_undefined());
            can_fold = true;
        }
        break;

    case Opcode::IsNullish:
        if (nops == 1) {
            auto const& v = op(0)->constant_value();
            result_value = JS::Value(v.is_null() || v.is_undefined());
            can_fold = true;
        }
        break;

    case Opcode::Move:
        // Move of a constant is just the constant
        if (nops == 1) {
            instruction.result()->replace_all_uses_with(op(0));
            can_fold = true;
        }
        return {};

    default:
        break;
    }

    if (can_fold) {
        auto& constant = function.create_constant(result_value);
        instruction.result()->replace_all_uses_with(&constant);
    }

    return result_value;
}

// Try algebraic simplification (partial-constant identities).
// Returns the replacement value, or nullptr if no simplification applies.
static Value* try_algebraic_simplify(Instruction& instruction, Function& function)
{
    if (!instruction.result())
        return nullptr;

    auto op = [&](size_t i) -> Value* { return instruction.operand(i); };
    auto nops = instruction.operand_count();

    switch (instruction.opcode()) {
    case Opcode::Add:
        // x + 0 -> x, 0 + x -> x (only for numeric types, since string + 0 = "0")
        if (nops == 2) {
            if (is_constant_zero(op(1)) && is_numeric_type(op(0)->type()))
                return op(0);
            if (is_constant_zero(op(0)) && is_numeric_type(op(1)->type()))
                return op(1);
        }
        break;

    case Opcode::Sub:
        // x - 0 -> x (only for numeric types, since "5" - 0 = 5)
        if (nops == 2) {
            if (is_constant_zero(op(1)) && is_numeric_type(op(0)->type()))
                return op(0);
            // x - x -> 0 (only safe for Int32, since NaN - NaN = NaN)
            if (op(0) == op(1) && op(0)->type() == Type::Int32)
                return &function.create_constant(JS::Value(0));
        }
        break;

    case Opcode::Mul:
        // x * 0 -> 0, 0 * x -> 0 (only safe for Int32, since NaN * 0 = NaN)
        if (nops == 2) {
            if ((is_constant_zero(op(0)) && op(1)->type() == Type::Int32)
                || (is_constant_zero(op(1)) && op(0)->type() == Type::Int32))
                return &function.create_constant(JS::Value(0));
            // x * 1 -> x, 1 * x -> x (only for numeric types, since "5" * 1 = 5)
            if (is_constant_one(op(1)) && is_numeric_type(op(0)->type()))
                return op(0);
            if (is_constant_one(op(0)) && is_numeric_type(op(1)->type()))
                return op(1);
        }
        break;

    case Opcode::Div:
        // x / 1 -> x (only for numeric types, since "5" / 1 = 5)
        if (nops == 2) {
            if (is_constant_one(op(1)) && is_numeric_type(op(0)->type()))
                return op(0);
        }
        break;

    case Opcode::BitwiseAnd:
        // x & 0 -> 0 (always safe, result is always 0)
        if (nops == 2) {
            if (is_constant_zero(op(0)) || is_constant_zero(op(1)))
                return &function.create_constant(JS::Value(0));
            // x & x -> x (only if x is already Int32, since bitwise ops coerce to Int32)
            if (op(0) == op(1) && op(0)->type() == Type::Int32)
                return op(0);
        }
        break;

    case Opcode::BitwiseOr:
        // x | 0 -> x, 0 | x -> x (only if x is already Int32)
        if (nops == 2) {
            if (is_constant_zero(op(1)) && op(0)->type() == Type::Int32)
                return op(0);
            if (is_constant_zero(op(0)) && op(1)->type() == Type::Int32)
                return op(1);
            // x | x -> x (only if x is already Int32)
            if (op(0) == op(1) && op(0)->type() == Type::Int32)
                return op(0);
        }
        break;

    case Opcode::BitwiseXor:
        // x ^ 0 -> x, 0 ^ x -> x (only if x is already Int32)
        if (nops == 2) {
            if (is_constant_zero(op(1)) && op(0)->type() == Type::Int32)
                return op(0);
            if (is_constant_zero(op(0)) && op(1)->type() == Type::Int32)
                return op(1);
            // x ^ x -> 0 (always safe, result is always 0)
            if (op(0) == op(1))
                return &function.create_constant(JS::Value(0));
        }
        break;

    case Opcode::LeftShift:
    case Opcode::RightShift:
        // x << 0 -> x, x >> 0 -> x (only if x is already Int32)
        if (nops == 2) {
            if (is_constant_zero(op(1)) && op(0)->type() == Type::Int32)
                return op(0);
        }
        break;

    case Opcode::UnsignedRightShift:
        // NB: x >>> 0 cannot be simplified to x, even for Int32.
        //     (-1) >>> 0 = 4294967295, which doesn't fit in Int32.
        break;

    // +x -> x when x is already numeric
    case Opcode::UnaryPlus:
        if (nops == 1 && is_numeric_type(op(0)->type()))
            return op(0);
        break;

    // ToNumber(x) -> x when x is already numeric
    case Opcode::ToNumber:
        if (nops == 1 && is_numeric_type(op(0)->type()))
            return op(0);
        break;

    // ToNumeric(x) -> x when x is already numeric
    case Opcode::ToNumeric:
        if (nops == 1 && is_numeric_type(op(0)->type()))
            return op(0);
        break;

    // ToInt32(x) -> x when x is already Int32
    case Opcode::ToInt32:
        if (nops == 1 && op(0)->type() == Type::Int32)
            return op(0);
        break;

    // ToBoolean(x) -> x when x is already Boolean
    case Opcode::ToBoolean:
        if (nops == 1 && op(0)->type() == Type::Boolean)
            return op(0);
        break;

    default:
        break;
    }

    return nullptr;
}

// Try instruction combining (pattern matching on instruction chains).
// Returns true if a simplification was applied.
static bool try_instruction_combine(Instruction& instruction, Function& function, BasicBlock& block)
{
    auto* result = instruction.result();

    switch (instruction.opcode()) {

    // Not (Not x) -> x
    case Opcode::Not: {
        if (instruction.operand_count() == 0 || !instruction.operand(0))
            break;

        auto* inner = instruction.operand(0)->defining_instruction();
        if (!inner)
            break;

        if (inner->opcode() == Opcode::Not) {
            if (auto* inner_operand = inner->operand(0)) {
                result->replace_all_uses_with(inner_operand);
                return true;
            }
        }
        break;
    }

    // BitwiseNot (BitwiseNot x) -> x (only if x is already Int32)
    case Opcode::BitwiseNot: {
        if (instruction.operand_count() == 0 || !instruction.operand(0))
            break;

        auto* inner = instruction.operand(0)->defining_instruction();
        if (inner && inner->opcode() == Opcode::BitwiseNot) {
            if (auto* inner_operand = inner->operand(0);
                inner_operand && inner_operand->type() == Type::Int32) {
                result->replace_all_uses_with(inner_operand);
                return true;
            }
        }
        break;
    }

    // Negate (Negate x) -> x (only if x is Int32)
    case Opcode::Negate: {
        if (instruction.operand_count() == 0 || !instruction.operand(0))
            break;

        auto* inner = instruction.operand(0)->defining_instruction();
        if (inner && inner->opcode() == Opcode::Negate) {
            if (auto* inner_operand = inner->operand(0);
                inner_operand && inner_operand->type() == Type::Int32) {
                result->replace_all_uses_with(inner_operand);
                return true;
            }
        }
        break;
    }

    // ToBoolean (ToBoolean x) -> ToBoolean x
    // ToBoolean (Not x) -> Not x (Not already returns boolean)
    case Opcode::ToBoolean: {
        if (instruction.operand_count() == 0 || !instruction.operand(0))
            break;

        auto* inner = instruction.operand(0)->defining_instruction();
        if (!inner)
            break;

        if (inner->opcode() == Opcode::ToBoolean || inner->opcode() == Opcode::Not) {
            result->replace_all_uses_with(instruction.operand(0));
            return true;
        }
        break;
    }

    // ToNumber (ToNumber x) -> ToNumber x
    case Opcode::ToNumber: {
        if (instruction.operand_count() == 0 || !instruction.operand(0))
            break;

        auto* inner = instruction.operand(0)->defining_instruction();
        if (inner && inner->opcode() == Opcode::ToNumber) {
            result->replace_all_uses_with(instruction.operand(0));
            return true;
        }
        break;
    }

    // ToNumeric (ToNumeric x) -> ToNumeric x
    case Opcode::ToNumeric: {
        if (instruction.operand_count() == 0 || !instruction.operand(0))
            break;

        auto* inner = instruction.operand(0)->defining_instruction();
        if (inner && inner->opcode() == Opcode::ToNumeric) {
            result->replace_all_uses_with(instruction.operand(0));
            return true;
        }
        break;
    }

    // Typeof x -> constant string when x has a known type
    case Opcode::Typeof: {
        if (instruction.operand_count() == 0 || !instruction.operand(0))
            break;

        auto typeof_string = typeof_result_for_type(instruction.operand(0)->type());
        if (!typeof_string)
            break;

        auto& constant = function.create_constant(JS::Value(typeof_string));
        result->replace_all_uses_with(&constant);
        return true;
    }

    // Branch (Not x), T, F -> Branch x, F, T
    case Opcode::Branch: {
        if (instruction.operand_count() == 0 || !instruction.operand(0))
            break;

        auto* condition = instruction.operand(0);
        auto* not_instr = condition->defining_instruction();
        if (!not_instr || not_instr->opcode() != Opcode::Not)
            break;

        if (condition->uses().size() != 1)
            break;

        auto* not_input = not_instr->operand(0);
        if (!not_input)
            break;

        auto* branch = static_cast<TerminatorInstruction*>(&instruction);

        auto* cmp_instr = not_input->defining_instruction();
        if (cmp_instr && not_input->uses().size() == 1) {
            if (inverted_comparison_if_safe(*cmp_instr).has_value()) {
                VERIFY(cmp_instr->try_invert_comparison());
                branch->set_operand(0, not_input);
                return true;
            }
        }

        branch->set_operand(0, not_input);
        CFG::swap_branch_targets(block);
        return true;
    }

    default:
        break;
    }

    return false;
}

PreservedAnalyses InstCombine::run(Function& function, PassManager&)
{
    bool changed = false;
    HashTable<Instruction*> dead_instructions;

    for (auto& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction& instruction) {
            // 1. Try constant folding (all operands are constants)
            bool folded = false;
            try_constant_fold(instruction, function, folded);
            if (folded) {
                changed = true;
                return;
            }

            // 2. Try algebraic simplification (partial-constant identities)
            if (auto* replacement = try_algebraic_simplify(instruction, function)) {
                if (!instruction.result()->uses().is_empty()) {
                    instruction.result()->replace_all_uses_with(replacement);
                    dead_instructions.set(&instruction);
                    changed = true;
                    return;
                }
            }

            // 3. Try instruction combining (pattern matching)
            if (try_instruction_combine(instruction, function, *block))
                changed = true;
        });
    }

    // Remove instructions that were simplified away.
    // This is needed because some (e.g. ToNumber) have has_side_effects=true
    // and would otherwise survive DCE even when they have no uses.
    if (!dead_instructions.is_empty()) {
        for (auto& block : function.basic_blocks()) {
            block->remove_instructions_if([&](Instruction const& instruction) {
                return dead_instructions.contains(const_cast<Instruction*>(&instruction));
            });
        }
    }

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
