/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/Bytecode/Instruction.h>
#include <LibJS/Bytecode/Op.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Lifter.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

Lifter::Lifter(Bytecode::Executable const& executable)
    : m_executable(executable)
    , m_function(Function::create(&executable))
{
}

NonnullOwnPtr<Function> Lifter::lift(Bytecode::Executable const& executable)
{
    Lifter lifter(executable);
    lifter.lift_basic_blocks();
    lifter.connect_control_flow();
    return move(lifter.m_function);
}

void Lifter::lift_basic_blocks()
{
    // First pass: create IR basic blocks for each bytecode basic block
    for (size_t i = 0; i < m_executable.basic_block_start_offsets.size(); ++i) {
        auto& block = m_function->create_block(String::formatted("block{}", i).release_value_but_fixme_should_propagate_errors());
        m_block_map.set(static_cast<u32>(i), &block);
        if (i == 0)
            m_function->set_entry_block(&block);
    }

    // Second pass: lift instructions from each basic block
    for (size_t block_index = 0; block_index < m_executable.basic_block_start_offsets.size(); ++block_index) {
        auto& ir_block = *m_block_map.get(static_cast<u32>(block_index)).value();

        size_t start_offset = m_executable.basic_block_start_offsets[block_index];
        size_t end_offset = (block_index + 1 < m_executable.basic_block_start_offsets.size())
            ? m_executable.basic_block_start_offsets[block_index + 1]
            : m_executable.bytecode.size();

        auto bytecode_span = ReadonlyBytes { m_executable.bytecode.data() + start_offset, end_offset - start_offset };
        Bytecode::InstructionStreamIterator it(bytecode_span, &m_executable);

        while (!it.at_end()) {
            lift_instruction(*it, ir_block);
            ++it;
        }
    }
}

Value& Lifter::get_or_create_value_for_operand(Bytecode::Operand operand)
{
    auto raw = operand.raw();

    // Check if we already have a value for this operand
    if (auto it = m_operand_to_value.find(raw); it != m_operand_to_value.end())
        return *it->value;

    // Decode the operand to get the real type (operands are stored in a flat space)
    auto decoded_operand = m_executable.original_operand_from_raw(raw);

    // Create a new value for this operand
    Value* value = nullptr;

    if (decoded_operand.is_constant()) {
        // Get the constant from the executable
        auto constant = m_executable.constants[decoded_operand.index()];
        value = &m_function->create_constant(constant);
    } else {
        // For registers/locals/arguments, create a register value
        // NB: This is a simplification - in full SSA we'd need phi nodes
        value = &m_function->create_register_value();
    }

    m_operand_to_value.set(raw, value);
    return *value;
}

Value& Lifter::create_value_for_destination(Bytecode::Operand operand)
{
    // For SSA, each write creates a new value
    // We update the mapping to point to the new value
    // NB: This is handled by the instruction creation, which returns the result value
    return get_or_create_value_for_operand(operand);
}

void Lifter::lift_instruction(Bytecode::Instruction const& instruction, BasicBlock& block)
{
    using enum Bytecode::Instruction::Type;

    switch (instruction.type()) {
    // Arithmetic binary ops
    case Add: {
        auto const& op = static_cast<Bytecode::Op::Add const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_add(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case Sub: {
        auto const& op = static_cast<Bytecode::Op::Sub const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_sub(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case Mul: {
        auto const& op = static_cast<Bytecode::Op::Mul const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_mul(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case Div: {
        auto const& op = static_cast<Bytecode::Op::Div const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_div(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case Mod: {
        auto const& op = static_cast<Bytecode::Op::Mod const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_mod(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case Exp: {
        auto const& op = static_cast<Bytecode::Op::Exp const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_exp(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Bitwise binary ops
    case BitwiseAnd: {
        auto const& op = static_cast<Bytecode::Op::BitwiseAnd const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_bitwise_and(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case BitwiseOr: {
        auto const& op = static_cast<Bytecode::Op::BitwiseOr const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_bitwise_or(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case BitwiseXor: {
        auto const& op = static_cast<Bytecode::Op::BitwiseXor const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_bitwise_xor(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case LeftShift: {
        auto const& op = static_cast<Bytecode::Op::LeftShift const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_left_shift(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case RightShift: {
        auto const& op = static_cast<Bytecode::Op::RightShift const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_right_shift(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case UnsignedRightShift: {
        auto const& op = static_cast<Bytecode::Op::UnsignedRightShift const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_unsigned_right_shift(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Comparison ops
    case LessThan: {
        auto const& op = static_cast<Bytecode::Op::LessThan const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_less_than(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case LessThanEquals: {
        auto const& op = static_cast<Bytecode::Op::LessThanEquals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_less_than_equals(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case GreaterThan: {
        auto const& op = static_cast<Bytecode::Op::GreaterThan const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_greater_than(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case GreaterThanEquals: {
        auto const& op = static_cast<Bytecode::Op::GreaterThanEquals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_greater_than_equals(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case LooselyEquals: {
        auto const& op = static_cast<Bytecode::Op::LooselyEquals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_loosely_equals(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case StrictlyEquals: {
        auto const& op = static_cast<Bytecode::Op::StrictlyEquals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_strictly_equals(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case LooselyInequals: {
        auto const& op = static_cast<Bytecode::Op::LooselyInequals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_loosely_inequals(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case StrictlyInequals: {
        auto const& op = static_cast<Bytecode::Op::StrictlyInequals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_strictly_inequals(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Unary ops
    case BitwiseNot: {
        auto const& op = static_cast<Bytecode::Op::BitwiseNot const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_bitwise_not(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case UnaryMinus: {
        auto const& op = static_cast<Bytecode::Op::UnaryMinus const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_negate(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case UnaryPlus: {
        auto const& op = static_cast<Bytecode::Op::UnaryPlus const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_unary_plus(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case Not: {
        auto const& op = static_cast<Bytecode::Op::Not const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_not(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case Typeof: {
        auto const& op = static_cast<Bytecode::Op::Typeof const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_typeof(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case ToBoolean: {
        auto const& op = static_cast<Bytecode::Op::ToBoolean const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value());
        auto& result = m_function->build_to_boolean(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case ToObject: {
        auto const& op = static_cast<Bytecode::Op::ToObject const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value());
        auto& result = m_function->build_to_object(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case ToString: {
        auto const& op = static_cast<Bytecode::Op::ToString const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value());
        auto& result = m_function->build_to_string(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case ToInt32: {
        auto const& op = static_cast<Bytecode::Op::ToInt32 const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value());
        auto& result = m_function->build_to_int32(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case ToLength: {
        auto const& op = static_cast<Bytecode::Op::ToLength const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value());
        auto& result = m_function->build_to_length(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case TypeofBinding: {
        auto const& op = static_cast<Bytecode::Op::TypeofBinding const&>(instruction);
        auto& result = m_function->build_typeof_binding(block, op.identifier());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Increment/Decrement
    case Increment: {
        auto const& op = static_cast<Bytecode::Op::Increment const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.dst());
        auto& result = m_function->build_increment(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case Decrement: {
        auto const& op = static_cast<Bytecode::Op::Decrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.dst());
        auto& result = m_function->build_decrement(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case PostfixIncrement: {
        auto const& op = static_cast<Bytecode::Op::PostfixIncrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_postfix_increment(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case PostfixDecrement: {
        auto const& op = static_cast<Bytecode::Op::PostfixDecrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_postfix_decrement(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // String ops
    case ConcatString: {
        auto const& op = static_cast<Bytecode::Op::ConcatString const&>(instruction);
        auto& dst = get_or_create_value_for_operand(op.dst());
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_concat_string(block, dst, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Move
    case Mov: {
        auto const& op = static_cast<Bytecode::Op::Mov const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src());
        auto& result = m_function->build_move(block, src);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Control flow - handled in connect_control_flow()
    case Jump:
    case JumpIf:
    case JumpTrue:
    case JumpFalse:
        // These are handled later when we connect control flow
        break;

    case Return: {
        auto const& op = static_cast<Bytecode::Op::Return const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.value());
        m_function->build_return(block, value);
        break;
    }

    case Throw: {
        auto const& op = static_cast<Bytecode::Op::Throw const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src());
        m_function->build_throw(block, value);
        break;
    }

    case End: {
        auto const& op = static_cast<Bytecode::Op::End const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.value());
        m_function->build_return(block, value);
        break;
    }

    // Property access
    case GetById: {
        auto const& op = static_cast<Bytecode::Op::GetById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base());
        auto& result = m_function->build_get_by_id(block, base, op.property());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case GetByValue: {
        auto const& op = static_cast<Bytecode::Op::GetByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base());
        auto& property = get_or_create_value_for_operand(op.property());
        auto& result = m_function->build_get_by_value(block, base, property);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case PutNormalById: {
        auto const& op = static_cast<Bytecode::Op::PutNormalById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base());
        auto& value = get_or_create_value_for_operand(op.src());
        m_function->build_put_by_id(block, base, op.property(), value);
        break;
    }
    case PutNormalByValue: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base());
        auto& property = get_or_create_value_for_operand(op.property());
        auto& value = get_or_create_value_for_operand(op.src());
        m_function->build_put_by_value(block, base, property, value);
        break;
    }
    case DeleteById: {
        auto const& op = static_cast<Bytecode::Op::DeleteById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base());
        auto& result = m_function->build_delete_by_id(block, base, op.property());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case DeleteByValue: {
        auto const& op = static_cast<Bytecode::Op::DeleteByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base());
        auto& property = get_or_create_value_for_operand(op.property());
        auto& result = m_function->build_delete_by_value(block, base, property);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // In/InstanceOf
    case In: {
        auto const& op = static_cast<Bytecode::Op::In const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_in(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case InstanceOf: {
        auto const& op = static_cast<Bytecode::Op::InstanceOf const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs());
        auto& rhs = get_or_create_value_for_operand(op.rhs());
        auto& result = m_function->build_instance_of(block, lhs, rhs);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Environment
    case GetBinding: {
        auto const& op = static_cast<Bytecode::Op::GetBinding const&>(instruction);
        auto& result = m_function->build_get_binding(block, op.identifier());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case GetInitializedBinding: {
        auto const& op = static_cast<Bytecode::Op::GetInitializedBinding const&>(instruction);
        auto& result = m_function->build_get_binding(block, op.identifier());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case SetLexicalBinding: {
        auto const& op = static_cast<Bytecode::Op::SetLexicalBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src());
        m_function->build_set_binding(block, op.identifier(), value);
        break;
    }
    case SetVariableBinding: {
        auto const& op = static_cast<Bytecode::Op::SetVariableBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src());
        m_function->build_set_binding(block, op.identifier(), value);
        break;
    }
    case InitializeLexicalBinding: {
        auto const& op = static_cast<Bytecode::Op::InitializeLexicalBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src());
        m_function->build_set_binding(block, op.identifier(), value);
        break;
    }
    case InitializeVariableBinding: {
        auto const& op = static_cast<Bytecode::Op::InitializeVariableBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src());
        m_function->build_set_binding(block, op.identifier(), value);
        break;
    }
    case DeleteVariable: {
        auto const& op = static_cast<Bytecode::Op::DeleteVariable const&>(instruction);
        auto& result = m_function->build_delete_variable(block, op.identifier());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case GetGlobal: {
        auto const& op = static_cast<Bytecode::Op::GetGlobal const&>(instruction);
        auto& result = m_function->build_get_global(block, op.identifier());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case SetGlobal: {
        auto const& op = static_cast<Bytecode::Op::SetGlobal const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src());
        m_function->build_set_global(block, op.identifier(), value);
        break;
    }

    // Object creation
    case NewObject: {
        auto const& op = static_cast<Bytecode::Op::NewObject const&>(instruction);
        auto& result = m_function->build_new_object(block);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case NewArray: {
        auto const& op = static_cast<Bytecode::Op::NewArray const&>(instruction);
        Vector<Value*> elements;
        for (auto operand : op.elements())
            elements.append(&get_or_create_value_for_operand(operand));
        auto& result = m_function->build_new_array(block, elements.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case NewPrimitiveArray: {
        auto const& op = static_cast<Bytecode::Op::NewPrimitiveArray const&>(instruction);
        Vector<Value*> elements;
        for (auto value : op.elements())
            elements.append(&m_function->create_constant(value));
        auto& result = m_function->build_new_array(block, elements.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Calls
    case Call: {
        auto const& op = static_cast<Bytecode::Op::Call const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee());
        auto& this_value = get_or_create_value_for_operand(op.this_value());
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand));
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case CallBuiltin: {
        auto const& op = static_cast<Bytecode::Op::CallBuiltin const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee());
        auto& this_value = get_or_create_value_for_operand(op.this_value());
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand));
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case CallConstruct: {
        auto const& op = static_cast<Bytecode::Op::CallConstruct const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee());
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand));
        auto& result = m_function->build_construct(block, callee, args.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case CallWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee());
        auto& this_value = get_or_create_value_for_operand(op.this_value());
        auto& args_array = get_or_create_value_for_operand(op.arguments());
        // NB: In full implementation, we'd need to handle spreading the array
        Vector<Value*> args { &args_array };
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case CallConstructWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallConstructWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee());
        auto& args_array = get_or_create_value_for_operand(op.arguments());
        Vector<Value*> args { &args_array };
        auto& result = m_function->build_construct(block, callee, args.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case CallDirectEval: {
        auto const& op = static_cast<Bytecode::Op::CallDirectEval const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee());
        auto& this_value = get_or_create_value_for_operand(op.this_value());
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand));
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case CallDirectEvalWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallDirectEvalWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee());
        auto& this_value = get_or_create_value_for_operand(op.this_value());
        auto& args_array = get_or_create_value_for_operand(op.arguments());
        Vector<Value*> args { &args_array };
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Iterators
    case GetIterator: {
        auto const& op = static_cast<Bytecode::Op::GetIterator const&>(instruction);
        auto& iterable = get_or_create_value_for_operand(op.iterable());
        auto& result = m_function->build_get_iterator(block, iterable);
        // NB: GetIterator writes to 3 destinations. We track the iterator object as the main result.
        m_operand_to_value.set(op.dst_iterator_object().raw(), &result);
        break;
    }
    case IteratorNext: {
        auto const& op = static_cast<Bytecode::Op::IteratorNext const&>(instruction);
        auto& iterator = get_or_create_value_for_operand(op.iterator_object());
        auto& result = m_function->build_iterator_next(block, iterator);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }
    case IteratorNextUnpack: {
        auto const& op = static_cast<Bytecode::Op::IteratorNextUnpack const&>(instruction);
        auto& iterator = get_or_create_value_for_operand(op.iterator_object());
        auto& result = m_function->build_iterator_next_unpack(block, iterator);
        m_operand_to_value.set(op.dst_value().raw(), &result);
        break;
    }
    case IteratorClose: {
        auto const& op = static_cast<Bytecode::Op::IteratorClose const&>(instruction);
        auto& iterator = get_or_create_value_for_operand(op.iterator_object());
        m_function->build_iterator_close(block, iterator);
        break;
    }
    case IteratorToArray: {
        auto const& op = static_cast<Bytecode::Op::IteratorToArray const&>(instruction);
        auto& iterator = get_or_create_value_for_operand(op.iterator_object());
        auto& result = m_function->build_iterator_to_array(block, iterator);
        m_operand_to_value.set(op.dst().raw(), &result);
        break;
    }

    // Environment creation ops (no IR needed, affects runtime)
    case CreateVariable:
    case CreateMutableBinding:
    case CreateImmutableBinding:
    case CreateLexicalEnvironment:
    case CreateVariableEnvironment:
    case CreatePrivateEnvironment:
    case LeaveLexicalEnvironment:
    case LeavePrivateEnvironment:
    case EnterObjectEnvironment:
        // These affect the environment but don't produce IR values
        break;

    // TODO: Handle more opcodes as needed
    default:
        // For unhandled opcodes, we skip them for now
        // In a complete implementation, we'd handle all opcodes
        break;
    }
}

u32 Lifter::address_to_block_index(size_t address) const
{
    // Find the basic block that contains this address
    for (size_t i = 0; i < m_executable.basic_block_start_offsets.size(); ++i) {
        if (m_executable.basic_block_start_offsets[i] == address)
            return static_cast<u32>(i);
    }
    // If we didn't find an exact match, find the block that contains this address
    for (size_t i = 0; i + 1 < m_executable.basic_block_start_offsets.size(); ++i) {
        if (address >= m_executable.basic_block_start_offsets[i] && address < m_executable.basic_block_start_offsets[i + 1])
            return static_cast<u32>(i);
    }
    // Default to the last block
    return static_cast<u32>(m_executable.basic_block_start_offsets.size() - 1);
}

void Lifter::connect_control_flow()
{
    // Second pass through instructions to connect control flow edges
    for (size_t block_index = 0; block_index < m_executable.basic_block_start_offsets.size(); ++block_index) {
        auto& ir_block = *m_block_map.get(static_cast<u32>(block_index)).value();

        // If block is already terminated, skip
        if (ir_block.is_terminated())
            continue;

        size_t start_offset = m_executable.basic_block_start_offsets[block_index];
        size_t end_offset = (block_index + 1 < m_executable.basic_block_start_offsets.size())
            ? m_executable.basic_block_start_offsets[block_index + 1]
            : m_executable.bytecode.size();

        auto bytecode_span = ReadonlyBytes { m_executable.bytecode.data() + start_offset, end_offset - start_offset };
        Bytecode::InstructionStreamIterator it(bytecode_span, &m_executable);

        // Find the last instruction (the terminator)
        Bytecode::Instruction const* last_instruction = nullptr;
        while (!it.at_end()) {
            last_instruction = &*it;
            ++it;
        }

        if (!last_instruction)
            continue;

        using enum Bytecode::Instruction::Type;
        switch (last_instruction->type()) {
        case Jump: {
            auto const& op = static_cast<Bytecode::Op::Jump const&>(*last_instruction);
            auto* target = m_block_map.get(address_to_block_index(op.target().address())).value();
            m_function->build_jump(ir_block, *target);
            break;
        }
        case JumpIf: {
            auto const& op = static_cast<Bytecode::Op::JumpIf const&>(*last_instruction);
            auto& condition = get_or_create_value_for_operand(op.condition());
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpTrue: {
            auto const& op = static_cast<Bytecode::Op::JumpTrue const&>(*last_instruction);
            auto& condition = get_or_create_value_for_operand(op.condition());
            // JumpTrue only has one target - we need to find the fallthrough
            auto* target = m_block_map.get(address_to_block_index(op.target().address())).value();
            // Fallthrough to next block
            if (block_index + 1 < m_executable.basic_block_start_offsets.size()) {
                auto* fallthrough = m_block_map.get(static_cast<u32>(block_index + 1)).value();
                m_function->build_branch(ir_block, condition, *target, *fallthrough);
            } else {
                // No fallthrough, just jump
                m_function->build_jump(ir_block, *target);
            }
            break;
        }
        case JumpFalse: {
            auto const& op = static_cast<Bytecode::Op::JumpFalse const&>(*last_instruction);
            auto& condition = get_or_create_value_for_operand(op.condition());
            auto* target = m_block_map.get(address_to_block_index(op.target().address())).value();
            // Fallthrough to next block
            if (block_index + 1 < m_executable.basic_block_start_offsets.size()) {
                auto* fallthrough = m_block_map.get(static_cast<u32>(block_index + 1)).value();
                m_function->build_branch(ir_block, condition, *fallthrough, *target);
            } else {
                // Negate and jump
                auto& negated = m_function->build_not(ir_block, condition);
                m_function->build_branch(ir_block, negated, *target, *target);
            }
            break;
        }
        default:
            // If not terminated by a known terminator, fall through to next block
            if (block_index + 1 < m_executable.basic_block_start_offsets.size()) {
                auto* next_block = m_block_map.get(static_cast<u32>(block_index + 1)).value();
                m_function->build_jump(ir_block, *next_block);
            }
            break;
        }
    }
}

void Lifter::insert_phi_nodes()
{
    // TODO: Implement proper SSA phi node insertion using dominance frontiers
    // For now, we're using a simplified approach that doesn't require full SSA
}

}
