/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Dump.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/Value.h>
#include <LibTest/TestCase.h>

TEST_CASE(create_empty_function_with_entry_block)
{
    auto function = JS::IR::Function::create();
    auto& entry = function->create_block("entry"_string);
    function->set_entry_block(&entry);

    EXPECT_EQ(function->basic_blocks().size(), 1u);
    EXPECT_EQ(function->entry_block(), &entry);
    EXPECT_EQ(entry.name(), "entry"_string);
    EXPECT_EQ(entry.parent_function(), function.ptr());
}

TEST_CASE(build_simple_return_a_plus_b)
{
    // Build IR for: function(a, b) { return a + b; }
    auto function = JS::IR::Function::create();

    auto& param_a = function->create_parameter(0);
    auto& param_b = function->create_parameter(1);

    auto& entry = function->create_block("entry"_string);
    function->set_entry_block(&entry);

    auto& sum = function->build_add(entry, param_a, param_b);
    function->build_return(entry, sum);

    EXPECT_EQ(function->parameters().size(), 2u);
    EXPECT_EQ(function->basic_blocks().size(), 1u);
    EXPECT_EQ(entry.instructions().size(), 2u);
    EXPECT(entry.is_terminated());

    // Verify instruction types
    EXPECT_EQ(entry.instructions()[0]->opcode(), JS::IR::Opcode::Add);
    EXPECT_EQ(entry.instructions()[1]->opcode(), JS::IR::Opcode::Return);

    // Verify def-use chains
    EXPECT_EQ(param_a.uses().size(), 1u);
    EXPECT_EQ(param_b.uses().size(), 1u);
    EXPECT_EQ(sum.uses().size(), 1u);
}

TEST_CASE(build_branch_with_then_else)
{
    // Build IR for: function(cond) { if (cond) { return 1; } else { return 0; } }
    auto function = JS::IR::Function::create();

    auto& param_cond = function->create_parameter(0);

    auto& entry = function->create_block("entry"_string);
    auto& then_block = function->create_block("then"_string);
    auto& else_block = function->create_block("else"_string);
    function->set_entry_block(&entry);

    function->build_branch(entry, param_cond, then_block, else_block);

    auto& one = function->build_load_constant(then_block, JS::Value(1));
    function->build_return(then_block, one);

    auto& zero = function->build_load_constant(else_block, JS::Value(0));
    function->build_return(else_block, zero);

    EXPECT_EQ(function->basic_blocks().size(), 3u);
    EXPECT(entry.is_terminated());
    EXPECT(then_block.is_terminated());
    EXPECT(else_block.is_terminated());

    // Verify predecessors
    EXPECT_EQ(then_block.predecessors().size(), 1u);
    EXPECT_EQ(then_block.predecessors()[0], &entry);
    EXPECT_EQ(else_block.predecessors().size(), 1u);
    EXPECT_EQ(else_block.predecessors()[0], &entry);
}

TEST_CASE(create_phi_node_at_merge_point)
{
    // Build IR for: function(cond) { var x; if (cond) { x = 1; } else { x = 2; } return x; }
    auto function = JS::IR::Function::create();

    auto& param_cond = function->create_parameter(0);

    auto& entry = function->create_block("entry"_string);
    auto& then_block = function->create_block("then"_string);
    auto& else_block = function->create_block("else"_string);
    auto& merge_block = function->create_block("merge"_string);
    function->set_entry_block(&entry);

    function->build_branch(entry, param_cond, then_block, else_block);

    auto& one = function->build_load_constant(then_block, JS::Value(1));
    function->build_jump(then_block, merge_block);

    auto& two = function->build_load_constant(else_block, JS::Value(2));
    function->build_jump(else_block, merge_block);

    Vector<JS::IR::Value*> phi_values { &one, &two };
    Vector<JS::IR::BasicBlock*> phi_preds { &then_block, &else_block };
    auto& phi = function->build_phi(merge_block, move(phi_values), move(phi_preds));
    function->build_return(merge_block, phi);

    EXPECT_EQ(function->basic_blocks().size(), 4u);
    EXPECT_EQ(merge_block.predecessors().size(), 2u);

    // Verify phi node
    auto& phi_instruction = *merge_block.instructions()[0];
    EXPECT_EQ(phi_instruction.opcode(), JS::IR::Opcode::Phi);
    EXPECT_EQ(phi_instruction.operands().size(), 2u);
    EXPECT_EQ(phi_instruction.phi_predecessors().size(), 2u);
}

TEST_CASE(setup_exception_handler_edge)
{
    auto function = JS::IR::Function::create();

    auto& try_block = function->create_block("try"_string);
    auto& catch_block = function->create_block("catch"_string);
    function->set_entry_block(&try_block);

    try_block.set_exception_handler(&catch_block);

    EXPECT_EQ(try_block.exception_handler(), &catch_block);
    EXPECT_EQ(try_block.finalizer(), nullptr);
}

TEST_CASE(setup_finalizer_edge)
{
    auto function = JS::IR::Function::create();

    auto& try_block = function->create_block("try"_string);
    auto& finally_block = function->create_block("finally"_string);
    function->set_entry_block(&try_block);

    try_block.set_finalizer(&finally_block);

    EXPECT_EQ(try_block.finalizer(), &finally_block);
    EXPECT_EQ(try_block.exception_handler(), nullptr);
}

TEST_CASE(dump_output_format)
{
    auto function = JS::IR::Function::create();

    auto& param_a = function->create_parameter(0);
    auto& param_b = function->create_parameter(1);

    auto& entry = function->create_block("entry"_string);
    function->set_entry_block(&entry);

    auto& sum = function->build_add(entry, param_a, param_b);
    function->build_return(entry, sum);

    auto output = JS::IR::dump(*function);

    // Verify the output contains expected elements
    EXPECT(output.contains("function("sv));
    EXPECT(output.contains("entry:"sv));
    EXPECT(output.contains("Add"sv));
    EXPECT(output.contains("Return"sv));
}

TEST_CASE(value_type_inference_for_constants)
{
    auto function = JS::IR::Function::create();
    auto& entry = function->create_block("entry"_string);

    auto& undefined_val = function->build_load_undefined(entry);
    EXPECT_EQ(undefined_val.type(), JS::IR::Type::Undefined);

    auto& null_val = function->build_load_null(entry);
    EXPECT_EQ(null_val.type(), JS::IR::Type::Null);
}

TEST_CASE(value_kinds)
{
    auto function = JS::IR::Function::create();

    auto& param = function->create_parameter(0);
    EXPECT(param.is_parameter());
    EXPECT(!param.is_constant());
    EXPECT(!param.is_instruction());

    auto& constant = function->create_constant(JS::Value(42));
    EXPECT(constant.is_constant());
    EXPECT(!constant.is_parameter());
    EXPECT(!constant.is_instruction());
    EXPECT_EQ(constant.constant_value().as_i32(), 42);

    auto& entry = function->create_block("entry"_string);
    auto& result = function->build_load_undefined(entry);
    EXPECT(result.is_instruction());
    EXPECT(!result.is_constant());
    EXPECT(!result.is_parameter());
    EXPECT(result.defining_instruction() != nullptr);
}

TEST_CASE(instruction_terminators)
{
    EXPECT(JS::IR::is_terminator_opcode(JS::IR::Opcode::Jump));
    EXPECT(JS::IR::is_terminator_opcode(JS::IR::Opcode::Branch));
    EXPECT(JS::IR::is_terminator_opcode(JS::IR::Opcode::Return));
    EXPECT(JS::IR::is_terminator_opcode(JS::IR::Opcode::Throw));
    EXPECT(!JS::IR::is_terminator_opcode(JS::IR::Opcode::Add));
    EXPECT(!JS::IR::is_terminator_opcode(JS::IR::Opcode::Call));
}

TEST_CASE(instruction_may_throw)
{
    EXPECT(!JS::IR::may_throw_opcode(JS::IR::Opcode::Jump));
    EXPECT(!JS::IR::may_throw_opcode(JS::IR::Opcode::Return));
    EXPECT(!JS::IR::may_throw_opcode(JS::IR::Opcode::LoadUndefined));
    EXPECT(!JS::IR::may_throw_opcode(JS::IR::Opcode::ToBoolean));
    EXPECT(!JS::IR::may_throw_opcode(JS::IR::Opcode::Not));

    EXPECT(JS::IR::may_throw_opcode(JS::IR::Opcode::Add));
    EXPECT(JS::IR::may_throw_opcode(JS::IR::Opcode::Call));
    EXPECT(JS::IR::may_throw_opcode(JS::IR::Opcode::GetById));
    EXPECT(JS::IR::may_throw_opcode(JS::IR::Opcode::Throw));
}

TEST_CASE(def_use_chains)
{
    auto function = JS::IR::Function::create();

    auto& param = function->create_parameter(0);
    auto& entry = function->create_block("entry"_string);

    EXPECT_EQ(param.uses().size(), 0u);

    // First use
    auto& negated = function->build_negate(entry, param);
    EXPECT_EQ(param.uses().size(), 1u);

    // Second use
    function->build_add(entry, param, negated);
    EXPECT_EQ(param.uses().size(), 2u);
    EXPECT_EQ(negated.uses().size(), 1u);
}
