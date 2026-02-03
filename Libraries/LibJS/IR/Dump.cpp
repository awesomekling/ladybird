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
#include <LibJS/IR/Passes/CommonSubexpressionElimination.h>
#include <LibJS/IR/Passes/ConstantBranchFolding.h>
#include <LibJS/IR/Passes/ConstantFolding.h>
#include <LibJS/IR/Passes/CopyPropagation.h>
#include <LibJS/IR/Passes/DeadBlockElimination.h>
#include <LibJS/IR/Passes/DeadCodeElimination.h>
#include <LibJS/IR/Passes/EmptyBlockElimination.h>
#include <LibJS/IR/Passes/JumpThreading.h>
#include <LibJS/IR/Passes/LoopInvariantCodeMotion.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/Value.h>

namespace JS::IR {

bool g_dump_ir = false;
bool g_optimize_ir = false;
bool g_dump_ir_between_passes = false;

void dump(Value const& value, StringBuilder& builder)
{
    builder.appendff("v{}", value.index());
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

    case Opcode::GetGlobal:
    case Opcode::SetGlobal:
    case Opcode::GetBinding:
    case Opcode::SetBinding:
        if (executable && instruction.identifier_index().is_valid()) {
            auto name = executable->identifier_table->get(instruction.identifier_index());
            builder.appendff(" {}", name);
        }
        for (auto* operand : instruction.operands())
            append_operand(operand);
        break;

    case Opcode::GetById:
    case Opcode::PutById:
    case Opcode::DeleteById:
        for (auto* operand : instruction.operands())
            append_operand(operand);
        if (executable && instruction.property_key_index().is_valid()) {
            auto name = executable->property_key_table->get(instruction.property_key_index());
            builder.appendff(".{}", name);
        }
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

static void run_pass(Pass& pass, Function& function, bool& changed)
{
    bool pass_changed = pass.run(function);
    if (pass_changed) {
        changed = true;
        if (g_dump_ir_between_passes)
            outln("=== After {} ===\n{}", pass.name(), dump(function));
    }
}

void optimize(Function& function)
{
    if (g_dump_ir_between_passes)
        outln("=== Before optimization ===\n{}", dump(function));

    // Run optimization passes until no more changes (with safety limit)
    constexpr size_t max_iterations = 10;
    for (size_t iteration = 0; iteration < max_iterations; ++iteration) {
        bool changed = false;

        // Copy propagation first - enables other optimizations
        CopyPropagation copy_prop;
        run_pass(copy_prop, function, changed);

        // Loop invariant code motion - hoist invariant computations out of loops
        LoopInvariantCodeMotion licm;
        run_pass(licm, function, changed);

        // Constant folding - evaluate constant expressions
        ConstantFolding const_fold;
        run_pass(const_fold, function, changed);

        // Algebraic simplification - x + 0 → x, x * 1 → x, etc.
        AlgebraicSimplification alg_simp;
        run_pass(alg_simp, function, changed);

        // Common subexpression elimination - reuse computed values
        CommonSubexpressionElimination cse;
        run_pass(cse, function, changed);

        // Constant branch folding - simplify branches on constants
        ConstantBranchFolding branch_fold;
        run_pass(branch_fold, function, changed);

        // Jump threading - thread jumps through blocks with constant phi inputs
        JumpThreading jump_thread;
        run_pass(jump_thread, function, changed);

        // Dead code elimination - remove unused instructions
        DeadCodeElimination dce;
        run_pass(dce, function, changed);

        // Dead block elimination - remove unreachable blocks
        DeadBlockElimination dbe;
        run_pass(dbe, function, changed);

        // Empty block elimination - remove blocks that only jump
        EmptyBlockElimination ebe;
        run_pass(ebe, function, changed);

        // Block merging - merge linear chains of blocks
        BlockMerging block_merge;
        run_pass(block_merge, function, changed);

        if (!changed)
            break;
    }
}

}
