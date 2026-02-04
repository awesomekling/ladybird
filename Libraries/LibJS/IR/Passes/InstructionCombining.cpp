/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/InstructionCombining.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

// Returns the inverted comparison opcode, or nullopt if not a comparison
static Optional<Opcode> inverted_comparison(Opcode opcode)
{
    switch (opcode) {
    case Opcode::LessThan:
        return Opcode::GreaterThanEquals;
    case Opcode::LessThanEquals:
        return Opcode::GreaterThan;
    case Opcode::GreaterThan:
        return Opcode::LessThanEquals;
    case Opcode::GreaterThanEquals:
        return Opcode::LessThan;
    case Opcode::StrictlyEquals:
        return Opcode::StrictlyInequals;
    case Opcode::StrictlyInequals:
        return Opcode::StrictlyEquals;
    case Opcode::LooselyEquals:
        return Opcode::LooselyInequals;
    case Opcode::LooselyInequals:
        return Opcode::LooselyEquals;
    default:
        return {};
    }
}

bool InstructionCombining::run(Function& function)
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

            // BitwiseNot (BitwiseNot x) → x
            case Opcode::BitwiseNot: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* inner = operands[0]->defining_instruction();
                if (inner && inner->opcode() == Opcode::BitwiseNot) {
                    if (auto* inner_operand = inner->operands()[0]) {
                        result->replace_all_uses_with(inner_operand);
                        changed = true;
                    }
                }
                break;
            }

            // Negate (Negate x) → x
            case Opcode::Negate: {
                if (operands.is_empty() || !operands[0])
                    break;

                auto* inner = operands[0]->defining_instruction();
                if (inner && inner->opcode() == Opcode::Negate) {
                    if (auto* inner_operand = inner->operands()[0]) {
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

                // Check if the Not's input is a comparison we can invert
                auto* cmp_instr = not_input->defining_instruction();
                if (cmp_instr && not_input->uses().size() == 1) {
                    if (auto inverted = inverted_comparison(cmp_instr->opcode()); inverted.has_value()) {
                        // Change the comparison opcode directly and use it as the branch condition
                        // This is safe because the comparison result is only used by the Not,
                        // which is only used by this Branch
                        cmp_instr->set_opcode(inverted.value());
                        instruction->set_operand(0, not_input);
                        changed = true;
                        break;
                    }
                }

                // Fall back to swapping targets if we can't invert the comparison
                auto* true_target = instruction->true_target();
                auto* false_target = instruction->false_target();
                instruction->set_operand(0, not_input);
                instruction->set_true_target(false_target);
                instruction->set_false_target(true_target);
                changed = true;
                break;
            }

            default:
                break;
            }
        }
    }

    return changed;
}

}
