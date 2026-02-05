/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/InstructionCombining.h>
#include <LibJS/IR/Type.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/PrimitiveString.h>
#include <LibJS/Runtime/VM.h>

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
// Only types that are already numeric and cannot represent NaN qualify.
// Other types (Number, String, Object, Undefined, etc.) could be or coerce to NaN.
static bool type_cannot_be_nan(Type type)
{
    return type == Type::Int32 || type == Type::Boolean;
}

// Returns the inverted comparison opcode if safe to invert.
// For relational comparisons (<, >, <=, >=), inversion is only safe when
// operands cannot be NaN, because !(NaN < x) != (NaN >= x).
// For equality comparisons, inversion is always safe.
static Optional<Opcode> inverted_comparison_if_safe(Instruction const& cmp_instr)
{
    auto opcode = cmp_instr.opcode();

    switch (opcode) {
    // Equality comparisons are always safe to invert
    case Opcode::StrictlyEquals:
        return Opcode::StrictlyInequals;
    case Opcode::StrictlyInequals:
        return Opcode::StrictlyEquals;
    case Opcode::LooselyEquals:
        return Opcode::LooselyInequals;
    case Opcode::LooselyInequals:
        return Opcode::LooselyEquals;

    // Relational comparisons: only safe if operands cannot be NaN
    case Opcode::LessThan:
    case Opcode::LessThanEquals:
    case Opcode::GreaterThan:
    case Opcode::GreaterThanEquals: {
        auto const& operands = cmp_instr.operands();
        if (operands.size() < 2)
            return {};

        // Both operands must be non-NaN types
        bool lhs_safe = operands[0] && type_cannot_be_nan(operands[0]->type());
        bool rhs_safe = operands[1] && type_cannot_be_nan(operands[1]->type());

        if (!lhs_safe || !rhs_safe)
            return {};

        switch (opcode) {
        case Opcode::LessThan:
            return Opcode::GreaterThanEquals;
        case Opcode::LessThanEquals:
            return Opcode::GreaterThan;
        case Opcode::GreaterThan:
            return Opcode::LessThanEquals;
        case Opcode::GreaterThanEquals:
            return Opcode::LessThan;
        default:
            VERIFY_NOT_REACHED();
        }
    }

    default:
        return {};
    }
}

PreservedAnalyses InstructionCombining::run(Function& function, PassManager&)
{
    bool changed = false;

    for (auto& block : function.basic_blocks()) {
        for (auto& instruction : block->instructions()) {
            auto* result = instruction->result();
            auto const& operands = instruction->operands();

            switch (instruction->opcode()) {

            // Not (Not x) → x
            case Opcode::Not: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* inner = operands[0]->defining_instruction();
                if (!inner)
                    break;

                // Not (Not x) → x
                if (inner->opcode() == Opcode::Not) {
                    if (auto* inner_operand = inner->operands()[0]) {
                        result->replace_all_uses_with(inner_operand);
                        changed = true;
                    }
                }
                break;
            }

            // BitwiseNot (BitwiseNot x) → x (only if x is already Int32)
            // NB: ~~x is ToInt32(x) in general, so this is only valid when x
            //     is already Int32. For example, ~~3.7 = 3, not 3.7.
            case Opcode::BitwiseNot: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* inner = operands[0]->defining_instruction();
                if (inner && inner->opcode() == Opcode::BitwiseNot) {
                    if (auto* inner_operand = inner->operands()[0];
                        inner_operand && inner_operand->type() == Type::Int32) {
                        result->replace_all_uses_with(inner_operand);
                        changed = true;
                    }
                }
                break;
            }

            // Negate (Negate x) → x (only if x is Int32)
            // NB: -(-0.0) = 0.0 ≠ -0.0, so this is only safe for Int32 which
            //     cannot represent negative zero.
            case Opcode::Negate: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* inner = operands[0]->defining_instruction();
                if (inner && inner->opcode() == Opcode::Negate) {
                    if (auto* inner_operand = inner->operands()[0];
                        inner_operand && inner_operand->type() == Type::Int32) {
                        result->replace_all_uses_with(inner_operand);
                        changed = true;
                    }
                }
                break;
            }

            // ToBoolean (ToBoolean x) → ToBoolean x
            // ToBoolean (Not x) → Not x (Not already returns boolean)
            case Opcode::ToBoolean: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* inner = operands[0]->defining_instruction();
                if (!inner)
                    break;

                if (inner->opcode() == Opcode::ToBoolean || inner->opcode() == Opcode::Not) {
                    result->replace_all_uses_with(operands[0]);
                    changed = true;
                }
                break;
            }

            // ToNumber (ToNumber x) → ToNumber x
            case Opcode::ToNumber: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* inner = operands[0]->defining_instruction();
                if (inner && inner->opcode() == Opcode::ToNumber) {
                    result->replace_all_uses_with(operands[0]);
                    changed = true;
                }
                break;
            }

            // ToNumeric (ToNumeric x) → ToNumeric x
            case Opcode::ToNumeric: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* inner = operands[0]->defining_instruction();
                if (inner && inner->opcode() == Opcode::ToNumeric) {
                    result->replace_all_uses_with(operands[0]);
                    changed = true;
                }
                break;
            }

            // Typeof x → constant string when x has a known type
            case Opcode::Typeof: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto typeof_string = typeof_result_for_type(operands[0]->type());
                if (!typeof_string)
                    break;

                auto& constant = function.create_constant(JS::Value(typeof_string));
                result->replace_all_uses_with(&constant);
                changed = true;
                break;
            }

            // Branch (Not x), T, F → Branch x, F, T
            // Branch (Not (comparison)), T, F → Branch (inverted comparison), T, F
            case Opcode::Branch: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* condition = operands[0];
                auto* not_instr = condition->defining_instruction();
                if (!not_instr || not_instr->opcode() != Opcode::Not)
                    break;

                // Only fold if the Not result is used solely by this branch
                if (condition->uses().size() != 1)
                    break;

                auto* not_input = not_instr->operands()[0];
                if (!not_input)
                    break;

                // Branch is a terminator, so we can safely cast
                auto* branch = static_cast<TerminatorInstruction*>(instruction.ptr());

                // Check if the Not's input is a comparison we can safely invert
                auto* cmp_instr = not_input->defining_instruction();
                if (cmp_instr && not_input->uses().size() == 1) {
                    if (inverted_comparison_if_safe(*cmp_instr).has_value()) {
                        // Invert the comparison opcode and use it as the branch condition
                        // This is safe because the comparison result is only used by the Not,
                        // which is only used by this Branch
                        VERIFY(cmp_instr->try_invert_comparison());
                        branch->set_operand(0, not_input);
                        changed = true;
                        break;
                    }
                }

                // Fall back to swapping targets if we can't invert the comparison
                auto* true_target = branch->true_target();
                auto* false_target = branch->false_target();
                branch->set_operand(0, not_input);
                branch->set_true_target(false_target);
                branch->set_false_target(true_target);
                changed = true;
                break;
            }

            default:
                break;
            }
        }
    }

    return changed ? PreservedAnalyses::all_cfg_analyses() : PreservedAnalyses::all();
}

}
