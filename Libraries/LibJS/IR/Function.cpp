/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

Function::Function(GC::Ptr<Bytecode::Executable const> source_executable)
    : m_source_executable(source_executable)
{
}

NonnullOwnPtr<Function> Function::create(GC::Ptr<Bytecode::Executable const> source_executable)
{
    return adopt_own(*new Function(source_executable));
}

BasicBlock& Function::create_block(String name)
{
    auto block = BasicBlock::create(m_next_block_index++, move(name));
    block->set_parent_function(this);
    auto& ref = *block;
    m_basic_blocks.append(move(block));
    return ref;
}

Value& Function::create_parameter(u32 parameter_index)
{
    auto value = Value::create_for_parameter(m_next_value_index++, parameter_index);
    auto& ref = *value;
    m_values.append(move(value));
    m_parameters.append(&ref);
    return ref;
}

Value& Function::create_register_value()
{
    // Creates a value for a bytecode register/local that isn't a real parameter
    auto value = Value::create_for_instruction(m_next_value_index++);
    auto& ref = *value;
    m_values.append(move(value));
    return ref;
}

Value& Function::create_constant(JS::Value constant)
{
    auto value = Value::create_for_constant(m_next_value_index++, constant);
    auto& ref = *value;
    m_values.append(move(value));
    return ref;
}

Value& Function::create_value_for_instruction()
{
    auto value = Value::create_for_instruction(m_next_value_index++);
    auto& ref = *value;
    m_values.append(move(value));
    return ref;
}

Value& Function::build_binary_op(BasicBlock& block, Opcode opcode, Value& lhs, Value& rhs)
{
    auto instruction = Instruction::create(opcode);
    instruction->add_operand(&lhs);
    instruction->add_operand(&rhs);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_unary_op(BasicBlock& block, Opcode opcode, Value& operand)
{
    auto instruction = Instruction::create(opcode);
    instruction->add_operand(&operand);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Arithmetic
Value& Function::build_add(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::Add, lhs, rhs); }
Value& Function::build_sub(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::Sub, lhs, rhs); }
Value& Function::build_mul(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::Mul, lhs, rhs); }
Value& Function::build_div(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::Div, lhs, rhs); }
Value& Function::build_mod(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::Mod, lhs, rhs); }
Value& Function::build_exp(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::Exp, lhs, rhs); }
Value& Function::build_negate(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::Negate, operand); }
Value& Function::build_unary_plus(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::UnaryPlus, operand); }

// Bitwise
Value& Function::build_bitwise_and(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::BitwiseAnd, lhs, rhs); }
Value& Function::build_bitwise_or(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::BitwiseOr, lhs, rhs); }
Value& Function::build_bitwise_xor(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::BitwiseXor, lhs, rhs); }
Value& Function::build_bitwise_not(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::BitwiseNot, operand); }
Value& Function::build_left_shift(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::LeftShift, lhs, rhs); }
Value& Function::build_right_shift(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::RightShift, lhs, rhs); }
Value& Function::build_unsigned_right_shift(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::UnsignedRightShift, lhs, rhs); }

// Comparison
Value& Function::build_less_than(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::LessThan, lhs, rhs); }
Value& Function::build_less_than_equals(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::LessThanEquals, lhs, rhs); }
Value& Function::build_greater_than(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::GreaterThan, lhs, rhs); }
Value& Function::build_greater_than_equals(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::GreaterThanEquals, lhs, rhs); }
Value& Function::build_loosely_equals(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::LooselyEquals, lhs, rhs); }
Value& Function::build_strictly_equals(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::StrictlyEquals, lhs, rhs); }
Value& Function::build_loosely_inequals(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::LooselyInequals, lhs, rhs); }
Value& Function::build_strictly_inequals(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::StrictlyInequals, lhs, rhs); }

// Type ops
Value& Function::build_typeof(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::Typeof, operand); }

Value& Function::build_typeof_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create(Opcode::TypeofBinding);
    instruction->set_identifier_index(identifier);

    auto& result = create_value_for_instruction();
    result.set_type(Type::String);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_to_boolean(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToBoolean, operand); }
Value& Function::build_to_number(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToNumber, operand); }
Value& Function::build_to_string(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToString, operand); }
Value& Function::build_to_object(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToObject, operand); }
Value& Function::build_to_int32(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToInt32, operand); }
Value& Function::build_to_length(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToLength, operand); }
Value& Function::build_not(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::Not, operand); }

// Increment/Decrement
Value& Function::build_increment(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::Increment, operand); }
Value& Function::build_decrement(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::Decrement, operand); }
Value& Function::build_postfix_increment(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::PostfixIncrement, operand); }
Value& Function::build_postfix_decrement(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::PostfixDecrement, operand); }

// String ops
Value& Function::build_concat_string(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::ConcatString, lhs, rhs); }

// Constants
Value& Function::build_load_constant(BasicBlock& block, JS::Value constant)
{
    auto instruction = Instruction::create(Opcode::LoadConstant);

    auto& constant_value = create_constant(constant);
    instruction->add_operand(&constant_value);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_load_undefined(BasicBlock& block)
{
    auto instruction = Instruction::create(Opcode::LoadUndefined);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Undefined);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_load_null(BasicBlock& block)
{
    auto instruction = Instruction::create(Opcode::LoadNull);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Null);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Property access
Value& Function::build_get_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property)
{
    auto instruction = Instruction::create(Opcode::GetById);
    instruction->add_operand(&base);
    instruction->set_property_key_index(property);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_get_by_value(BasicBlock& block, Value& base, Value& property)
{
    auto instruction = Instruction::create(Opcode::GetByValue);
    instruction->add_operand(&base);
    instruction->add_operand(&property);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_get_length(BasicBlock& block, Value& base)
{
    return build_unary_op(block, Opcode::GetLength, base);
}

void Function::build_put_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& value)
{
    auto instruction = Instruction::create(Opcode::PutById);
    instruction->add_operand(&base);
    instruction->add_operand(&value);
    instruction->set_property_key_index(property);

    block.append(move(instruction));
}

void Function::build_put_by_value(BasicBlock& block, Value& base, Value& property, Value& value)
{
    auto instruction = Instruction::create(Opcode::PutByValue);
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&value);

    block.append(move(instruction));
}

Value& Function::build_delete_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property)
{
    auto instruction = Instruction::create(Opcode::DeleteById);
    instruction->add_operand(&base);
    instruction->set_property_key_index(property);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Boolean);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_delete_by_value(BasicBlock& block, Value& base, Value& property)
{
    auto instruction = Instruction::create(Opcode::DeleteByValue);
    instruction->add_operand(&base);
    instruction->add_operand(&property);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Boolean);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_has_property(BasicBlock& block, Value& object, Value& property)
{
    auto instruction = Instruction::create(Opcode::HasProperty);
    instruction->add_operand(&object);
    instruction->add_operand(&property);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Boolean);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Calls
Value& Function::build_call(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments)
{
    auto instruction = Instruction::create(Opcode::Call);
    instruction->add_operand(&callee);
    instruction->add_operand(&this_value);
    for (auto* arg : arguments)
        instruction->add_operand(arg);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_construct(BasicBlock& block, Value& callee, Span<Value*> arguments)
{
    auto instruction = Instruction::create(Opcode::Construct);
    instruction->add_operand(&callee);
    for (auto* arg : arguments)
        instruction->add_operand(arg);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Object);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Environment
Value& Function::build_get_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create(Opcode::GetBinding);
    instruction->set_identifier_index(identifier);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

void Function::build_set_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value)
{
    auto instruction = Instruction::create(Opcode::SetBinding);
    instruction->set_identifier_index(identifier);
    instruction->add_operand(&value);

    block.append(move(instruction));
}

Value& Function::build_get_global(BasicBlock& block, Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create(Opcode::GetGlobal);
    instruction->set_identifier_index(identifier);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

void Function::build_set_global(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value)
{
    auto instruction = Instruction::create(Opcode::SetGlobal);
    instruction->set_identifier_index(identifier);
    instruction->add_operand(&value);

    block.append(move(instruction));
}

Value& Function::build_delete_variable(BasicBlock& block, Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create(Opcode::DeleteVariable);
    instruction->set_identifier_index(identifier);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Boolean);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Object creation
Value& Function::build_new_object(BasicBlock& block)
{
    auto instruction = Instruction::create(Opcode::NewObject);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Object);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_new_array(BasicBlock& block, Span<Value*> elements)
{
    auto instruction = Instruction::create(Opcode::NewArray);
    for (auto* element : elements)
        instruction->add_operand(element);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Array);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_new_function(BasicBlock& block)
{
    auto instruction = Instruction::create(Opcode::NewFunction);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Function);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Special
Value& Function::build_in(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::In, lhs, rhs); }
Value& Function::build_instance_of(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::InstanceOf, lhs, rhs); }

// Iterators
Value& Function::build_get_iterator(BasicBlock& block, Value& iterable)
{
    return build_unary_op(block, Opcode::GetIterator, iterable);
}

Value& Function::build_iterator_next(BasicBlock& block, Value& iterator)
{
    return build_unary_op(block, Opcode::IteratorNext, iterator);
}

Value& Function::build_iterator_next_unpack(BasicBlock& block, Value& iterator)
{
    return build_unary_op(block, Opcode::IteratorNextUnpack, iterator);
}

void Function::build_iterator_close(BasicBlock& block, Value& iterator)
{
    auto instruction = Instruction::create(Opcode::IteratorClose);
    instruction->add_operand(&iterator);
    block.append(move(instruction));
}

Value& Function::build_iterator_to_array(BasicBlock& block, Value& iterator)
{
    auto instruction = Instruction::create(Opcode::IteratorToArray);
    instruction->add_operand(&iterator);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Array);
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Copy
Value& Function::build_move(BasicBlock& block, Value& source)
{
    return build_unary_op(block, Opcode::Move, source);
}

// Control flow
void Function::build_jump(BasicBlock& from, BasicBlock& to)
{
    auto instruction = Instruction::create(Opcode::Jump);
    instruction->set_true_target(&to);
    to.add_predecessor(&from);
    from.append(move(instruction));
}

void Function::build_branch(BasicBlock& from, Value& condition, BasicBlock& if_true, BasicBlock& if_false)
{
    auto instruction = Instruction::create(Opcode::Branch);
    instruction->add_operand(&condition);
    instruction->set_true_target(&if_true);
    instruction->set_false_target(&if_false);
    if_true.add_predecessor(&from);
    if_false.add_predecessor(&from);
    from.append(move(instruction));
}

void Function::build_return(BasicBlock& block, Value& value)
{
    auto instruction = Instruction::create(Opcode::Return);
    instruction->add_operand(&value);
    block.append(move(instruction));
}

void Function::build_throw(BasicBlock& block, Value& value)
{
    auto instruction = Instruction::create(Opcode::Throw);
    instruction->add_operand(&value);
    block.append(move(instruction));
}

// SSA
Value& Function::build_phi(BasicBlock& block, Vector<Value*> values, Vector<BasicBlock*> predecessors)
{
    VERIFY(values.size() == predecessors.size());

    auto instruction = Instruction::create(Opcode::Phi);

    for (size_t i = 0; i < values.size(); ++i)
        instruction->add_phi_operand(predecessors[i], values[i]);

    auto& result = create_value_for_instruction();
    result.set_defining_instruction(instruction.ptr());
    instruction->set_result(&result);

    // Phi nodes go at the start of the block (before other instructions)
    block.prepend(move(instruction));
    return result;
}

}
