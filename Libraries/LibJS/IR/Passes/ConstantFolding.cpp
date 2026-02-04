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
#include <LibJS/Runtime/AbstractOperations.h>
#include <LibJS/Runtime/Value.h>
#include <math.h>

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

            // Helper to get a double from a numeric constant (Int32 or double)
            auto numeric_to_double = [](JS::Value v) -> Optional<double> {
                if (v.is_int32())
                    return static_cast<double>(v.as_i32());
                if (v.is_double())
                    return v.as_double();
                return {};
            };

            // Helper to check if two operands are both numeric constants
            auto both_numeric = [&]() -> bool {
                return operands.size() == 2
                    && (operands[0]->constant_value().is_int32() || operands[0]->constant_value().is_double())
                    && (operands[1]->constant_value().is_int32() || operands[1]->constant_value().is_double());
            };

            switch (instruction->opcode()) {
            case Opcode::Add:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    i64 lhs = operands[0]->constant_value().as_i32();
                    i64 rhs = operands[1]->constant_value().as_i32();
                    result_value = make_int_or_double(lhs + rhs);
                    can_fold = true;
                } else if (both_numeric()) {
                    result_value = JS::Value(*numeric_to_double(operands[0]->constant_value()) + *numeric_to_double(operands[1]->constant_value()));
                    can_fold = true;
                }
                break;

            case Opcode::Sub:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    i64 lhs = operands[0]->constant_value().as_i32();
                    i64 rhs = operands[1]->constant_value().as_i32();
                    result_value = make_int_or_double(lhs - rhs);
                    can_fold = true;
                } else if (both_numeric()) {
                    result_value = JS::Value(*numeric_to_double(operands[0]->constant_value()) - *numeric_to_double(operands[1]->constant_value()));
                    can_fold = true;
                }
                break;

            case Opcode::Mul:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    i64 lhs = operands[0]->constant_value().as_i32();
                    i64 rhs = operands[1]->constant_value().as_i32();
                    result_value = make_int_or_double(lhs * rhs);
                    can_fold = true;
                } else if (both_numeric()) {
                    result_value = JS::Value(*numeric_to_double(operands[0]->constant_value()) * *numeric_to_double(operands[1]->constant_value()));
                    can_fold = true;
                }
                break;

            case Opcode::Div:
                if (both_numeric()) {
                    result_value = JS::Value(*numeric_to_double(operands[0]->constant_value()) / *numeric_to_double(operands[1]->constant_value()));
                    can_fold = true;
                }
                break;

            case Opcode::Mod:
                if (both_numeric()) {
                    result_value = JS::Value(fmod(*numeric_to_double(operands[0]->constant_value()), *numeric_to_double(operands[1]->constant_value())));
                    can_fold = true;
                }
                break;

            case Opcode::Exp:
                if (both_numeric()) {
                    result_value = JS::Value(pow(*numeric_to_double(operands[0]->constant_value()), *numeric_to_double(operands[1]->constant_value())));
                    can_fold = true;
                }
                break;

            case Opcode::LessThan:
                if (both_numeric()) {
                    result_value = JS::Value(*numeric_to_double(operands[0]->constant_value()) < *numeric_to_double(operands[1]->constant_value()));
                    can_fold = true;
                }
                break;

            case Opcode::LessThanEquals:
                if (both_numeric()) {
                    result_value = JS::Value(*numeric_to_double(operands[0]->constant_value()) <= *numeric_to_double(operands[1]->constant_value()));
                    can_fold = true;
                }
                break;

            case Opcode::GreaterThan:
                if (both_numeric()) {
                    result_value = JS::Value(*numeric_to_double(operands[0]->constant_value()) > *numeric_to_double(operands[1]->constant_value()));
                    can_fold = true;
                }
                break;

            case Opcode::GreaterThanEquals:
                if (both_numeric()) {
                    result_value = JS::Value(*numeric_to_double(operands[0]->constant_value()) >= *numeric_to_double(operands[1]->constant_value()));
                    can_fold = true;
                }
                break;

            case Opcode::StrictlyEquals:
            case Opcode::StrictlyInequals:
                if (operands.size() == 2) {
                    auto const& lhs = operands[0]->constant_value();
                    auto const& rhs = operands[1]->constant_value();
                    bool equals = false;
                    bool can_determine = false;

                    if (both_numeric()) {
                        // Int32 and Double are both Number type — compare as doubles.
                        double l = *numeric_to_double(lhs);
                        double r = *numeric_to_double(rhs);
                        // NaN !== NaN, and +0 === -0 are handled correctly by C++ ==.
                        equals = l == r;
                        can_determine = true;
                    } else if (lhs.is_boolean() && rhs.is_boolean()) {
                        equals = lhs.as_bool() == rhs.as_bool();
                        can_determine = true;
                    } else if ((lhs.is_null() && rhs.is_null()) || (lhs.is_undefined() && rhs.is_undefined())) {
                        equals = true;
                        can_determine = true;
                    } else if ((lhs.is_null() || lhs.is_undefined() || lhs.is_boolean() || lhs.is_number())
                        && (rhs.is_null() || rhs.is_undefined() || rhs.is_boolean() || rhs.is_number())) {
                        // Different primitive types are never strictly equal.
                        equals = false;
                        can_determine = true;
                    }

                    if (can_determine) {
                        bool result = (instruction->opcode() == Opcode::StrictlyEquals) ? equals : !equals;
                        result_value = JS::Value(result);
                        can_fold = true;
                    }
                }
                break;

            case Opcode::LooselyEquals:
            case Opcode::LooselyInequals:
                if (operands.size() == 2) {
                    auto const& lhs = operands[0]->constant_value();
                    auto const& rhs = operands[1]->constant_value();
                    bool equals = false;
                    bool can_determine = false;

                    auto is_nullish = [](JS::Value v) { return v.is_null() || v.is_undefined(); };

                    if (is_nullish(lhs) && is_nullish(rhs)) {
                        // null == undefined (and vice versa) is true.
                        equals = true;
                        can_determine = true;
                    } else if (is_nullish(lhs) || is_nullish(rhs)) {
                        // null/undefined == non-nullish is always false.
                        equals = false;
                        can_determine = true;
                    } else if (both_numeric()) {
                        equals = *numeric_to_double(lhs) == *numeric_to_double(rhs);
                        can_determine = true;
                    } else if (lhs.is_boolean() && rhs.is_boolean()) {
                        equals = lhs.as_bool() == rhs.as_bool();
                        can_determine = true;
                    } else if (lhs.is_boolean() && (rhs.is_int32() || rhs.is_double())) {
                        // ToNumber(bool) == number
                        equals = static_cast<double>(lhs.as_bool() ? 1 : 0) == *numeric_to_double(rhs);
                        can_determine = true;
                    } else if ((lhs.is_int32() || lhs.is_double()) && rhs.is_boolean()) {
                        equals = *numeric_to_double(lhs) == static_cast<double>(rhs.as_bool() ? 1 : 0);
                        can_determine = true;
                    }

                    if (can_determine) {
                        bool result = (instruction->opcode() == Opcode::LooselyEquals) ? equals : !equals;
                        result_value = JS::Value(result);
                        can_fold = true;
                    }
                }
                break;

            case Opcode::BitwiseAnd:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    result_value = JS::Value(operands[0]->constant_value().as_i32() & operands[1]->constant_value().as_i32());
                    can_fold = true;
                }
                break;

            case Opcode::BitwiseOr:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    result_value = JS::Value(operands[0]->constant_value().as_i32() | operands[1]->constant_value().as_i32());
                    can_fold = true;
                }
                break;

            case Opcode::BitwiseXor:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    result_value = JS::Value(operands[0]->constant_value().as_i32() ^ operands[1]->constant_value().as_i32());
                    can_fold = true;
                }
                break;

            case Opcode::BitwiseNot:
                if (operands.size() == 1 && operands[0]->constant_value().is_int32()) {
                    result_value = JS::Value(~operands[0]->constant_value().as_i32());
                    can_fold = true;
                }
                break;

            case Opcode::LeftShift:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    i32 lhs = operands[0]->constant_value().as_i32();
                    u32 rhs = static_cast<u32>(operands[1]->constant_value().as_i32()) & 0x1f;
                    result_value = JS::Value(lhs << rhs);
                    can_fold = true;
                }
                break;

            case Opcode::RightShift:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    i32 lhs = operands[0]->constant_value().as_i32();
                    u32 rhs = static_cast<u32>(operands[1]->constant_value().as_i32()) & 0x1f;
                    result_value = JS::Value(lhs >> rhs);
                    can_fold = true;
                }
                break;

            case Opcode::UnsignedRightShift:
                if (operands.size() == 2 && operands[0]->constant_value().is_int32() && operands[1]->constant_value().is_int32()) {
                    u32 lhs = static_cast<u32>(operands[0]->constant_value().as_i32());
                    u32 rhs = static_cast<u32>(operands[1]->constant_value().as_i32()) & 0x1f;
                    u32 result = lhs >> rhs;
                    if (result <= static_cast<u32>(NumericLimits<i32>::max()))
                        result_value = JS::Value(static_cast<i32>(result));
                    else
                        result_value = JS::Value(static_cast<double>(result));
                    can_fold = true;
                }
                break;

            case Opcode::Negate:
                if (operands.size() == 1) {
                    auto const& v = operands[0]->constant_value();
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
                if (operands.size() == 1) {
                    auto const& v = operands[0]->constant_value();
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
                if (operands.size() == 1) {
                    auto const& v = operands[0]->constant_value();
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
                if (operands.size() == 1) {
                    auto const& v = operands[0]->constant_value();
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
                if (operands.size() == 1) {
                    auto const& v = operands[0]->constant_value();
                    if (v.is_int32()) {
                        result_value = v;
                        can_fold = true;
                    } else if (v.is_double()) {
                        double d = v.as_double();
                        if (!isfinite(d) || d == 0.0) {
                            result_value = JS::Value(0);
                        } else {
                            auto int_val = floor(fabs(d));
                            if (signbit(d))
                                int_val = -int_val;
                            auto int32bit = modulo(int_val, 4294967296.0);
                            if (int32bit >= 2147483648.0)
                                int32bit -= 4294967296.0;
                            result_value = JS::Value(static_cast<i32>(int32bit));
                        }
                        can_fold = true;
                    }
                }
                break;

            case Opcode::ToNumber:
                if (operands.size() == 1) {
                    auto const& v = operands[0]->constant_value();
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

            case Opcode::ToBoolean:
                if (operands.size() == 1) {
                    if (auto truthiness = operands[0]->constant_truthiness(); truthiness.has_value()) {
                        result_value = JS::Value(*truthiness);
                        can_fold = true;
                    }
                }
                break;

            case Opcode::Not:
                if (operands.size() == 1) {
                    if (auto truthiness = operands[0]->constant_truthiness(); truthiness.has_value()) {
                        result_value = JS::Value(!*truthiness);
                        can_fold = true;
                    }
                }
                break;

            case Opcode::IsUndefined:
                if (operands.size() == 1) {
                    result_value = JS::Value(operands[0]->constant_value().is_undefined());
                    can_fold = true;
                }
                break;

            case Opcode::IsNullish:
                if (operands.size() == 1) {
                    auto const& v = operands[0]->constant_value();
                    result_value = JS::Value(v.is_null() || v.is_undefined());
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
