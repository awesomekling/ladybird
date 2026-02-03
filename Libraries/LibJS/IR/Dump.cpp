/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringBuilder.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Dump.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/Value.h>

namespace JS::IR {

void dump(Value const& value, StringBuilder& builder)
{
    builder.appendff("v{}", value.index());
}

void dump(Instruction const& instruction, StringBuilder& builder)
{
    if (instruction.result()) {
        dump(*instruction.result(), builder);
        builder.append(" = "sv);
    }

    builder.appendff("{}", opcode_to_string(instruction.opcode()));

    bool first_operand = true;
    auto append_operand = [&](Value const* value) {
        if (first_operand)
            builder.append(' ');
        else
            builder.append(", "sv);
        first_operand = false;

        if (value) {
            if (value->is_constant()) {
                auto constant = value->constant_value();
                if (constant.is_undefined())
                    builder.append("undefined"sv);
                else if (constant.is_null())
                    builder.append("null"sv);
                else if (constant.is_boolean())
                    builder.append(constant.as_bool() ? "true"sv : "false"sv);
                else if (constant.is_int32())
                    builder.appendff("{}", constant.as_i32());
                else if (constant.is_double())
                    builder.appendff("{}", constant.as_double());
                else
                    dump(*value, builder);
            } else {
                dump(*value, builder);
            }
        } else {
            builder.append("null"sv);
        }
    };

    // Handle special opcodes
    switch (instruction.opcode()) {
    case Opcode::Jump:
        if (instruction.true_target())
            builder.appendff(" block{}", instruction.true_target()->index());
        break;

    case Opcode::Branch:
        for (auto* operand : instruction.operands())
            append_operand(operand);
        if (instruction.true_target())
            builder.appendff(", block{}", instruction.true_target()->index());
        if (instruction.false_target())
            builder.appendff(", block{}", instruction.false_target()->index());
        break;

    case Opcode::Phi:
        builder.append(" ["sv);
        for (size_t i = 0; i < instruction.operands().size(); ++i) {
            if (i > 0)
                builder.append(", "sv);
            builder.appendff("block{}:", instruction.phi_predecessors()[i]->index());
            if (instruction.operands()[i])
                dump(*instruction.operands()[i], builder);
            else
                builder.append("null"sv);
        }
        builder.append(']');
        break;

    default:
        for (auto* operand : instruction.operands())
            append_operand(operand);
        break;
    }
}

void dump(BasicBlock const& block, StringBuilder& builder)
{
    if (block.name().is_empty())
        builder.appendff("block{}:", block.index());
    else
        builder.appendff("{}:", block.name());

    if (block.exception_handler())
        builder.appendff(" [handler: block{}]", block.exception_handler()->index());
    if (block.finalizer())
        builder.appendff(" [finalizer: block{}]", block.finalizer()->index());

    builder.append('\n');

    for (auto const& instruction : block.instructions()) {
        builder.append("    "sv);
        dump(*instruction, builder);
        builder.append('\n');
    }
}

void dump(Function const& function, StringBuilder& builder)
{
    builder.append("function("sv);

    for (size_t i = 0; i < function.parameters().size(); ++i) {
        if (i > 0)
            builder.append(", "sv);
        dump(*function.parameters()[i], builder);
    }

    builder.append("):\n"sv);

    for (auto const& block : function.basic_blocks()) {
        dump(*block, builder);
    }
}

String dump(Value const& value)
{
    StringBuilder builder;
    dump(value, builder);
    return MUST(builder.to_string());
}

String dump(Instruction const& instruction)
{
    StringBuilder builder;
    dump(instruction, builder);
    return MUST(builder.to_string());
}

String dump(BasicBlock const& block)
{
    StringBuilder builder;
    dump(block, builder);
    return MUST(builder.to_string());
}

String dump(Function const& function)
{
    StringBuilder builder;
    dump(function, builder);
    return MUST(builder.to_string());
}

}
