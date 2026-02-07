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
    builder.appendff("v{}:{}", value.index(), type_to_string(value.type()));
}

static void dump_instruction(Instruction const& instruction, StringBuilder& builder, Bytecode::Executable const* executable)
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
    case Opcode::ContinuePendingUnwind:
    case Opcode::EnterUnwindContext: {
        auto const& jump = static_cast<TerminatorInstruction const&>(instruction);
        if (auto target = jump.true_target_index(); target.has_value())
            builder.appendff(" block{}", *target);
        break;
    }

    case Opcode::ScheduleJump: {
        auto const& term = static_cast<TerminatorInstruction const&>(instruction);
        if (auto target = term.true_target_index(); target.has_value())
            builder.appendff(" finalizer: block{}", *target);
        if (auto target = term.false_target_index(); target.has_value())
            builder.appendff(", target: block{}", *target);
        break;
    }

    case Opcode::Branch: {
        auto const& branch = static_cast<TerminatorInstruction const&>(instruction);
        for (size_t i = 0; i < instruction.operand_count(); ++i)
            append_operand(instruction.operand(i));
        if (auto target = branch.true_target_index(); target.has_value())
            builder.appendff(", block{}", *target);
        if (auto target = branch.false_target_index(); target.has_value())
            builder.appendff(", block{}", *target);
        break;
    }

    case Opcode::Phi: {
        auto const& phi = static_cast<PhiInstruction const&>(instruction);
        builder.append(" ["sv);
        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            if (i > 0)
                builder.append(", "sv);
            builder.appendff("block{}:", phi.incoming_block(i));
            if (phi.incoming_value(i))
                dump(*phi.incoming_value(i), builder);
            else
                builder.append("null"sv);
        }
        builder.append(']');
        break;
    }

    case Opcode::ParallelCopy: {
        auto const& pcopy = static_cast<ParallelCopyInstruction const&>(instruction);
        builder.append(" ["sv);
        for (size_t i = 0; i < pcopy.copies().size(); ++i) {
            if (i > 0)
                builder.append(", "sv);
            dump(*pcopy.copy_dst(i), builder);
            builder.append(" <- "sv);
            dump(*pcopy.copy_src(i), builder);
        }
        builder.append(']');
        break;
    }

    case Opcode::GetGlobal:
    case Opcode::SetGlobal:
    case Opcode::GetBinding:
    case Opcode::SetBinding:
    case Opcode::InitializeBinding:
        if (executable && instruction.identifier_index().is_valid()) {
            auto name = executable->identifier_table->get(instruction.identifier_index());
            builder.appendff(" {}", name);
        }
        for (size_t i = 0; i < instruction.operand_count(); ++i)
            append_operand(instruction.operand(i));
        if (instruction.opcode() == Opcode::InitializeBinding || instruction.opcode() == Opcode::SetBinding) {
            if (instruction.environment_mode() == Bytecode::Op::EnvironmentMode::Var)
                builder.append(" (var)"sv);
        }
        break;

    case Opcode::GetById:
    case Opcode::GetMethod:
    case Opcode::PutById:
    case Opcode::PutByIdWithThis:
    case Opcode::DeleteById:
    case Opcode::DeleteByIdWithThis:
        for (size_t i = 0; i < instruction.operand_count(); ++i)
            append_operand(instruction.operand(i));
        if (executable && instruction.property_key_index().is_valid()) {
            auto name = executable->property_key_table->get(instruction.property_key_index());
            builder.appendff(".{}", name);
        }
        break;

    case Opcode::ExtractValue:
        for (size_t i = 0; i < instruction.operand_count(); ++i)
            append_operand(instruction.operand(i));
        builder.appendff(", {}", instruction.extract_index());
        break;

    case Opcode::Yield:
    case Opcode::Await: {
        auto const& term = static_cast<TerminatorInstruction const&>(instruction);
        for (size_t i = 0; i < instruction.operand_count(); ++i)
            append_operand(instruction.operand(i));
        if (auto target = term.true_target_index(); target.has_value())
            builder.appendff(", continuation: block{}", *target);
        break;
    }

    case Opcode::CreateArguments:
        builder.appendff(" kind:{}, immutable:{}", instruction.arguments_kind() == Bytecode::Op::ArgumentsKind::Mapped ? "mapped"sv : "unmapped"sv, instruction.is_immutable());
        break;

    case Opcode::CreateRestParams:
        builder.appendff(" rest_index:{}", instruction.rest_index());
        break;

    case Opcode::ArrayAppend:
        for (size_t i = 0; i < instruction.operand_count(); ++i)
            append_operand(instruction.operand(i));
        builder.appendff(", is_spread:{}", instruction.is_spread());
        break;

    default:
        for (size_t i = 0; i < instruction.operand_count(); ++i)
            append_operand(instruction.operand(i));
        break;
    }
}

void dump(Instruction const& instruction, StringBuilder& builder)
{
    dump_instruction(instruction, builder, nullptr);
}

static void dump_block(BasicBlock const& block, StringBuilder& builder, Bytecode::Executable const* executable)
{
    if (block.name().is_empty())
        builder.appendff("block{}:", block.index());
    else
        builder.appendff("{}:", block.name());

    if (block.exception_handler_index().has_value())
        builder.appendff(" [handler: block{}]", *block.exception_handler_index());
    if (block.finalizer_index().has_value())
        builder.appendff(" [finalizer: block{}]", *block.finalizer_index());

    builder.append('\n');

    block.for_each_instruction([&](Instruction const& instruction) {
        builder.append("    "sv);
        dump_instruction(instruction, builder, executable);
        builder.append('\n');
    });
}

void dump(BasicBlock const& block, StringBuilder& builder)
{
    dump_block(block, builder, nullptr);
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

    auto* executable = function.source_executable().ptr();
    for (auto const& block : function.basic_blocks()) {
        dump_block(*block, builder, executable);
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
