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
#include <LibJS/IR/Passes/AlgebraicSimplification.h>
#include <LibJS/IR/Passes/BlockMerging.h>
#include <LibJS/IR/Passes/ConstantBranchFolding.h>
#include <LibJS/IR/Passes/ConstantFolding.h>
#include <LibJS/IR/Passes/CopyPropagation.h>
#include <LibJS/IR/Passes/DeadBlockElimination.h>
#include <LibJS/IR/Passes/DeadCodeElimination.h>
#include <LibJS/IR/Passes/EmptyBlockElimination.h>
#include <LibJS/IR/Passes/GlobalValueNumbering.h>
#include <LibJS/IR/Passes/InstructionCombining.h>
#include <LibJS/IR/Passes/JumpThreading.h>
#include <LibJS/IR/Passes/LoopInvariantCodeMotion.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/Value.h>

namespace JS::IR {

bool g_dump_ir = false;
bool g_optimize_ir = false;
bool g_dump_ir_between_passes = false;
bool g_lower_ir = false;

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
    case Opcode::Jump: {
        auto const& jump = static_cast<TerminatorInstruction const&>(instruction);
        if (jump.true_target())
            builder.appendff(" block{}", jump.true_target()->index());
        break;
    }

    case Opcode::Branch: {
        auto const& branch = static_cast<TerminatorInstruction const&>(instruction);
        for (auto* operand : instruction.operands())
            append_operand(operand);
        if (branch.true_target())
            builder.appendff(", block{}", branch.true_target()->index());
        if (branch.false_target())
            builder.appendff(", block{}", branch.false_target()->index());
        break;
    }

    case Opcode::Phi: {
        auto const& phi = static_cast<PhiInstruction const&>(instruction);
        builder.append(" ["sv);
        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            if (i > 0)
                builder.append(", "sv);
            builder.appendff("block{}:", phi.incoming_block(i)->index());
            if (phi.incoming_value(i))
                dump(*phi.incoming_value(i), builder);
            else
                builder.append("null"sv);
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
        for (auto* operand : instruction.operands())
            append_operand(operand);
        if (instruction.opcode() == Opcode::InitializeBinding || instruction.opcode() == Opcode::SetBinding) {
            if (instruction.environment_mode() == Bytecode::Op::EnvironmentMode::Var)
                builder.append(" (var)"sv);
        }
        break;

    case Opcode::GetById:
    case Opcode::PutById:
    case Opcode::PutByIdWithThis:
    case Opcode::DeleteById:
    case Opcode::DeleteByIdWithThis:
        for (auto* operand : instruction.operands())
            append_operand(operand);
        if (executable && instruction.property_key_index().is_valid()) {
            auto name = executable->property_key_table->get(instruction.property_key_index());
            builder.appendff(".{}", name);
        }
        break;

    case Opcode::ExtractValue:
        for (auto* operand : instruction.operands())
            append_operand(operand);
        builder.appendff(", {}", instruction.extract_index());
        break;

    case Opcode::Yield:
    case Opcode::Await: {
        auto const& term = static_cast<TerminatorInstruction const&>(instruction);
        for (auto* operand : instruction.operands())
            append_operand(operand);
        if (term.true_target())
            builder.appendff(", continuation: block{}", term.true_target()->index());
        break;
    }

    case Opcode::CreateArguments:
        builder.appendff(" kind:{}, immutable:{}", instruction.arguments_kind() == Bytecode::Op::ArgumentsKind::Mapped ? "mapped"sv : "unmapped"sv, instruction.is_immutable());
        break;

    case Opcode::CreateRestParams:
        builder.appendff(" rest_index:{}", instruction.rest_index());
        break;

    case Opcode::ArrayAppend:
        for (auto* operand : instruction.operands())
            append_operand(operand);
        builder.appendff(", is_spread:{}", instruction.is_spread());
        break;

    default:
        for (auto* operand : instruction.operands())
            append_operand(operand);
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

    if (block.exception_handler())
        builder.appendff(" [handler: block{}]", block.exception_handler()->index());
    if (block.finalizer())
        builder.appendff(" [finalizer: block{}]", block.finalizer()->index());

    builder.append('\n');

    for (auto const& instruction : block.instructions()) {
        builder.append("    "sv);
        dump_instruction(*instruction, builder, executable);
        builder.append('\n');
    }
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

void optimize(Function& function)
{
    PassManager pass_manager;

    // Phase 1 — CFG Simplification
    pass_manager.add_pass(make<ConstantBranchFolding>());
    pass_manager.add_pass(make<JumpThreading>());

    // Phase 2 — Dead Code Removal
    pass_manager.add_pass(make<DeadCodeElimination>());
    pass_manager.add_pass(make<DeadBlockElimination>());

    // Phase 3 — Local Optimizations
    pass_manager.add_pass(make<CopyPropagation>());
    pass_manager.add_pass(make<ConstantFolding>());
    pass_manager.add_pass(make<AlgebraicSimplification>());
    pass_manager.add_pass(make<InstructionCombining>());

    // Phase 4 — Global Optimizations
    pass_manager.add_pass(make<GlobalValueNumbering>());

    // Phase 5 — Loop Optimizations
    pass_manager.add_pass(make<LoopInvariantCodeMotion>());

    // Phase 6 — Final CFG Cleanup
    pass_manager.add_pass(make<EmptyBlockElimination>());
    pass_manager.add_pass(make<BlockMerging>());

    pass_manager.run(function);
}

}
