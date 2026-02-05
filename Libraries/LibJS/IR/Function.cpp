/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
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

Value& Function::create_this()
{
    if (m_this_value)
        return *m_this_value;
    auto value = Value::create_for_this(m_next_value_index++);
    auto& ref = *value;
    m_values.append(move(value));
    m_this_value = &ref;
    return ref;
}

Value& Function::create_register_value()
{
    // Creates a placeholder value for a bytecode register/local.
    // NB: The caller MUST either:
    //   1. Set the operand mapping via Lifter::m_value_to_operand_raw so SSA
    //      renaming can replace this placeholder with the reaching definition, OR
    //   2. Call define_operand() which sets both m_current_definitions and the mapping
    // Failure to do so will cause SSA renaming to skip this value, leaving it
    // as a dangling placeholder that the verifier will catch.
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
    auto instruction = BinaryOpInstruction::create(opcode, &lhs, &rhs);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    // Set known result types
    switch (opcode) {
    // Arithmetic operations always produce Number
    case Opcode::Add:
    case Opcode::Sub:
    case Opcode::Mul:
    case Opcode::Div:
    case Opcode::Mod:
    case Opcode::Exp:
        if (is_safe_numeric_type(lhs.type()) && is_safe_numeric_type(rhs.type()))
            result.set_type(Type::Number);
        break;
    // Bitwise operations always produce Int32 (due to ToInt32 conversion in JS)
    case Opcode::BitwiseAnd:
    case Opcode::BitwiseOr:
    case Opcode::BitwiseXor:
    case Opcode::LeftShift:
    case Opcode::RightShift:
        result.set_type(Type::Int32);
        break;
    // Unsigned right shift produces Uint32 which may not fit in Int32
    case Opcode::UnsignedRightShift:
        result.set_type(Type::Number);
        break;
    // Comparison and membership operations always produce Boolean
    case Opcode::LessThan:
    case Opcode::LessThanEquals:
    case Opcode::GreaterThan:
    case Opcode::GreaterThanEquals:
    case Opcode::LooselyEquals:
    case Opcode::StrictlyEquals:
    case Opcode::LooselyInequals:
    case Opcode::StrictlyInequals:
    case Opcode::In:
    case Opcode::InstanceOf:
        result.set_type(Type::Boolean);
        break;
    // String concatenation always produces String
    case Opcode::ConcatString:
        result.set_type(Type::String);
        break;
    default:
        break;
    }

    block.append(move(instruction));
    return result;
}

Value& Function::build_unary_op(BasicBlock& block, Opcode opcode, Value& operand)
{
    auto instruction = UnaryOpInstruction::create(opcode, &operand);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    // Set known result types for type conversion and check operations
    switch (opcode) {
    case Opcode::Move:
        result.set_type(operand.type());
        break;
    case Opcode::BitwiseNot:
    case Opcode::ToInt32:
        result.set_type(Type::Int32);
        break;
    case Opcode::ToNumber:
    case Opcode::UnaryPlus:
    case Opcode::Negate:
    case Opcode::Increment:
    case Opcode::Decrement:
    case Opcode::PostfixIncrement:
    case Opcode::PostfixDecrement:
        result.set_type(Type::Number);
        break;
    case Opcode::ToNumeric:
        if (operand.type() == Type::Int32)
            result.set_type(Type::Int32);
        else if (operand.type() == Type::Number)
            result.set_type(Type::Number);
        else if (operand.type() == Type::BigInt)
            result.set_type(Type::BigInt);
        break;
    case Opcode::ToBoolean:
    case Opcode::Not:
    case Opcode::IsUndefined:
    case Opcode::IsNullish:
        result.set_type(Type::Boolean);
        break;
    case Opcode::Typeof:
    case Opcode::ToString:
        result.set_type(Type::String);
        break;
    default:
        break;
    }

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
    auto instruction = Instruction::create<Opcode::TypeofBinding>();
    instruction->set_identifier_index(identifier);

    auto& result = create_value_for_instruction();
    result.set_type(Type::String);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_to_boolean(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToBoolean, operand); }
Value& Function::build_to_number(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToNumber, operand); }
Value& Function::build_to_numeric(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToNumeric, operand); }
Value& Function::build_to_string(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToString, operand); }
Value& Function::build_to_object(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToObject, operand); }
Value& Function::build_to_int32(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToInt32, operand); }
Value& Function::build_to_length(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::ToLength, operand); }
Value& Function::build_not(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::Not, operand); }
Value& Function::build_is_undefined(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::IsUndefined, operand); }
Value& Function::build_is_nullish(BasicBlock& block, Value& operand) { return build_unary_op(block, Opcode::IsNullish, operand); }

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
    auto instruction = Instruction::create<Opcode::LoadConstant>();

    auto& constant_value = create_constant(constant);
    instruction->add_operand(&constant_value);

    auto& result = create_value_for_instruction();
    result.set_type(constant_value.type());
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_load_undefined(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::LoadUndefined>();

    auto& result = create_value_for_instruction();
    result.set_type(Type::Undefined);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_load_null(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::LoadNull>();

    auto& result = create_value_for_instruction();
    result.set_type(Type::Null);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Property access
Value& Function::build_get_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = GetByIdInstruction::create(&base, property);
    instruction->set_base_identifier(base_identifier);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_get_by_id_with_this(BasicBlock& block, Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property)
{
    auto instruction = Instruction::create<Opcode::GetByIdWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->set_property_key_index(property);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_get_by_value(BasicBlock& block, Value& base, Value& property, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::GetByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->set_base_identifier(base_identifier);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_get_by_value_with_this(BasicBlock& block, Value& base, Value& this_value, Value& property)
{
    auto instruction = Instruction::create<Opcode::GetByValueWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&property);

    auto& result = create_value_for_instruction();
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
    auto instruction = Instruction::create<Opcode::PutById>();
    instruction->add_operand(&base);
    instruction->add_operand(&value);
    instruction->set_property_key_index(property);

    block.append(move(instruction));
}

void Function::build_put_by_value(BasicBlock& block, Value& base, Value& property, Value& value)
{
    auto instruction = Instruction::create<Opcode::PutByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&value);

    block.append(move(instruction));
}

Value& Function::build_delete_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property)
{
    auto instruction = Instruction::create<Opcode::DeleteById>();
    instruction->add_operand(&base);
    instruction->set_property_key_index(property);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Boolean);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_delete_by_value(BasicBlock& block, Value& base, Value& property)
{
    auto instruction = Instruction::create<Opcode::DeleteByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Boolean);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_has_property(BasicBlock& block, Value& object, Value& property)
{
    auto instruction = Instruction::create<Opcode::HasProperty>();
    instruction->add_operand(&object);
    instruction->add_operand(&property);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Boolean);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_get_private_by_id(BasicBlock& block, Value& base, Bytecode::IdentifierTableIndex property)
{
    auto instruction = Instruction::create<Opcode::GetPrivateById>();
    instruction->add_operand(&base);
    instruction->set_identifier_index(property);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

void Function::build_put_private_by_id(BasicBlock& block, Value& base, Bytecode::IdentifierTableIndex property, Value& value)
{
    auto instruction = Instruction::create<Opcode::PutPrivateById>();
    instruction->add_operand(&base);
    instruction->add_operand(&value);
    instruction->set_identifier_index(property);
    block.append(move(instruction));
}

void Function::build_put_getter_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& getter, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::PutGetterById>();
    instruction->add_operand(&base);
    instruction->add_operand(&getter);
    instruction->set_property_key_index(property);
    instruction->set_base_identifier(base_identifier);
    block.append(move(instruction));
}

void Function::build_put_setter_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& setter, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::PutSetterById>();
    instruction->add_operand(&base);
    instruction->add_operand(&setter);
    instruction->set_property_key_index(property);
    instruction->set_base_identifier(base_identifier);
    block.append(move(instruction));
}

void Function::build_put_prototype_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& prototype, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::PutPrototypeById>();
    instruction->add_operand(&base);
    instruction->add_operand(&prototype);
    instruction->set_property_key_index(property);
    instruction->set_base_identifier(base_identifier);
    block.append(move(instruction));
}

void Function::build_put_getter_by_id_with_this(BasicBlock& block, Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& getter)
{
    auto instruction = Instruction::create<Opcode::PutGetterByIdWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&getter);
    instruction->set_property_key_index(property);
    block.append(move(instruction));
}

void Function::build_put_setter_by_id_with_this(BasicBlock& block, Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& setter)
{
    auto instruction = Instruction::create<Opcode::PutSetterByIdWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&setter);
    instruction->set_property_key_index(property);
    block.append(move(instruction));
}

void Function::build_put_prototype_by_id_with_this(BasicBlock& block, Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& prototype)
{
    auto instruction = Instruction::create<Opcode::PutPrototypeByIdWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&prototype);
    instruction->set_property_key_index(property);
    block.append(move(instruction));
}

void Function::build_put_getter_by_value(BasicBlock& block, Value& base, Value& property, Value& getter, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::PutGetterByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&getter);
    instruction->set_base_identifier(base_identifier);
    block.append(move(instruction));
}

void Function::build_put_setter_by_value(BasicBlock& block, Value& base, Value& property, Value& setter, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::PutSetterByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&setter);
    instruction->set_base_identifier(base_identifier);
    block.append(move(instruction));
}

void Function::build_put_prototype_by_value(BasicBlock& block, Value& base, Value& property, Value& prototype, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::PutPrototypeByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&prototype);
    instruction->set_base_identifier(base_identifier);
    block.append(move(instruction));
}

void Function::build_put_getter_by_value_with_this(BasicBlock& block, Value& base, Value& property, Value& this_value, Value& getter)
{
    auto instruction = Instruction::create<Opcode::PutGetterByValueWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&this_value);
    instruction->add_operand(&getter);
    block.append(move(instruction));
}

void Function::build_put_setter_by_value_with_this(BasicBlock& block, Value& base, Value& property, Value& this_value, Value& setter)
{
    auto instruction = Instruction::create<Opcode::PutSetterByValueWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&this_value);
    instruction->add_operand(&setter);
    block.append(move(instruction));
}

void Function::build_put_prototype_by_value_with_this(BasicBlock& block, Value& base, Value& property, Value& this_value, Value& prototype)
{
    auto instruction = Instruction::create<Opcode::PutPrototypeByValueWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&this_value);
    instruction->add_operand(&prototype);
    block.append(move(instruction));
}

void Function::build_put_by_spread(BasicBlock& block, Value& base, Value& source)
{
    auto instruction = Instruction::create<Opcode::PutBySpread>();
    instruction->add_operand(&base);
    instruction->add_operand(&source);
    block.append(move(instruction));
}

// Calls
Value& Function::build_call(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments)
{
    auto instruction = CallInstruction::create(Opcode::Call, &callee, &this_value);
    for (auto* arg : arguments)
        instruction->add_operand(arg);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_call_builtin(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments, Bytecode::Builtin builtin, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = CallInstruction::create(Opcode::CallBuiltin, &callee, &this_value);
    for (auto* arg : arguments)
        instruction->add_operand(arg);
    instruction->set_builtin(builtin);
    instruction->set_expression_string(expression_string);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_call_direct_eval(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = CallInstruction::create(Opcode::CallDirectEval, &callee, &this_value);
    for (auto* arg : arguments)
        instruction->add_operand(arg);
    instruction->set_expression_string(expression_string);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_call_with_argument_array(BasicBlock& block, Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = CallInstruction::create(Opcode::CallWithArgumentArray, &callee, &this_value);
    instruction->add_operand(&arguments);
    instruction->set_expression_string(expression_string);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_construct(BasicBlock& block, Value& callee, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = Instruction::create<Opcode::Construct>();
    instruction->add_operand(&callee);
    for (auto* arg : arguments)
        instruction->add_operand(arg);
    instruction->set_expression_string(expression_string);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Object);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_construct_with_argument_array(BasicBlock& block, Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = Instruction::create<Opcode::ConstructWithArgumentArray>();
    instruction->add_operand(&callee);
    instruction->add_operand(&this_value);
    instruction->add_operand(&arguments);
    instruction->set_expression_string(expression_string);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Object);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_super_call_with_argument_array(BasicBlock& block, Value& arguments, bool is_synthetic)
{
    auto instruction = Instruction::create<Opcode::SuperCallWithArgumentArray>();
    instruction->add_operand(&arguments);
    instruction->set_is_synthetic(is_synthetic);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Object);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_import_call(BasicBlock& block, Value& specifier, Value& options)
{
    auto instruction = Instruction::create<Opcode::ImportCall>();
    instruction->add_operand(&specifier);
    instruction->add_operand(&options);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Environment
Value& Function::build_get_callee_and_this_from_environment(BasicBlock& block, Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::GetCalleeAndThisFromEnvironment>();
    instruction->set_identifier_index(identifier);

    // This produces a tuple of (callee, this_value)
    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

void Function::build_create_variable(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Bytecode::Op::EnvironmentMode mode, bool is_immutable, bool is_global, bool is_strict)
{
    auto instruction = Instruction::create<Opcode::CreateVariable>();
    instruction->set_identifier_index(identifier);
    instruction->set_environment_mode(mode);
    instruction->set_is_immutable(is_immutable);
    instruction->set_is_global(is_global);
    instruction->set_is_strict(is_strict);

    block.append(move(instruction));
}

Value& Function::build_create_lexical_environment(BasicBlock& block, u32 capacity)
{
    auto instruction = Instruction::create<Opcode::CreateLexicalEnvironment>();
    instruction->set_capacity(capacity);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

void Function::build_create_mutable_binding(BasicBlock& block, Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict)
{
    auto instruction = Instruction::create<Opcode::CreateMutableBinding>();
    instruction->add_operand(&environment);
    instruction->set_identifier_index(identifier);
    instruction->set_is_strict(is_strict);
    block.append(move(instruction));
}

void Function::build_create_immutable_binding(BasicBlock& block, Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict)
{
    auto instruction = Instruction::create<Opcode::CreateImmutableBinding>();
    instruction->add_operand(&environment);
    instruction->set_identifier_index(identifier);
    instruction->set_is_strict(is_strict);
    block.append(move(instruction));
}

void Function::build_leave_lexical_environment(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::LeaveLexicalEnvironment>();
    block.append(move(instruction));
}

void Function::build_create_private_environment(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::CreatePrivateEnvironment>();
    block.append(move(instruction));
}

void Function::build_leave_private_environment(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::LeavePrivateEnvironment>();
    block.append(move(instruction));
}

void Function::build_add_private_name(BasicBlock& block, Bytecode::IdentifierTableIndex name)
{
    auto instruction = Instruction::create<Opcode::AddPrivateName>();
    instruction->set_identifier_index(name);
    block.append(move(instruction));
}

void Function::build_create_variable_environment(BasicBlock& block, u32 capacity)
{
    auto instruction = Instruction::create<Opcode::CreateVariableEnvironment>();
    instruction->set_capacity(capacity);
    block.append(move(instruction));
}

void Function::build_enter_object_environment(BasicBlock& block, Value& object)
{
    auto instruction = Instruction::create<Opcode::EnterObjectEnvironment>();
    instruction->add_operand(&object);
    block.append(move(instruction));
}

void Function::build_resolve_this_binding(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::ResolveThisBinding>();
    block.append(move(instruction));
}

Value& Function::build_resolve_super_base(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::ResolveSuperBase>();
    auto& result = create_value_for_instruction();
    instruction->set_result(&result);
    block.append(move(instruction));
    return result;
}

Value& Function::build_get_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::GetBinding>();
    instruction->set_identifier_index(identifier);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

void Function::build_initialize_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value)
{
    auto instruction = Instruction::create<Opcode::InitializeBinding>();
    instruction->set_identifier_index(identifier);
    instruction->add_operand(&value);

    block.append(move(instruction));
}

void Function::build_set_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value)
{
    auto instruction = Instruction::create<Opcode::SetBinding>();
    instruction->set_identifier_index(identifier);
    instruction->add_operand(&value);

    block.append(move(instruction));
}

Value& Function::build_get_global(BasicBlock& block, Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::GetGlobal>();
    instruction->set_identifier_index(identifier);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

void Function::build_set_global(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value)
{
    auto instruction = Instruction::create<Opcode::SetGlobal>();
    instruction->set_identifier_index(identifier);
    instruction->add_operand(&value);

    block.append(move(instruction));
}

Value& Function::build_delete_variable(BasicBlock& block, Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::DeleteVariable>();
    instruction->set_identifier_index(identifier);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Boolean);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Object creation
Value& Function::build_new_object(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::NewObject>();

    auto& result = create_value_for_instruction();
    result.set_type(Type::Object);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_new_array(BasicBlock& block, Span<Value*> elements)
{
    auto instruction = Instruction::create<Opcode::NewArray>();
    for (auto* element : elements)
        instruction->add_operand(element);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Array);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_new_array_with_length(BasicBlock& block, Value& length)
{
    auto instruction = Instruction::create<Opcode::NewArrayWithLength>();
    instruction->add_operand(&length);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Array);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

void Function::build_array_append(BasicBlock& block, Value& array, Value& value, bool is_spread)
{
    auto instruction = Instruction::create<Opcode::ArrayAppend>();
    instruction->add_operand(&array);
    instruction->add_operand(&value);
    instruction->set_is_spread(is_spread);
    block.append(move(instruction));
}

Value& Function::build_new_class(BasicBlock& block, Value* super_class, Span<Value*> element_keys)
{
    auto instruction = Instruction::create<Opcode::NewClass>();

    // super_class is the first operand (may be null)
    instruction->add_operand(super_class);

    // element_keys follow
    for (auto* key : element_keys)
        instruction->add_operand(key);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Function); // Classes are functions
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_new_function(BasicBlock& block, Value* home_object)
{
    auto instruction = Instruction::create<Opcode::NewFunction>();

    if (home_object)
        instruction->add_operand(home_object);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Function);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_new_regexp(BasicBlock& block, Bytecode::StringTableIndex source, Bytecode::StringTableIndex flags, Bytecode::RegexTableIndex regex)
{
    auto instruction = Instruction::create<Opcode::NewRegExp>();

    auto& result = create_value_for_instruction();
    result.set_type(Type::Object);
    instruction->set_result(&result);
    instruction->set_regex_source_index(source);
    instruction->set_regex_flags_index(flags);
    instruction->set_regex_index(regex);

    block.append(move(instruction));
    return result;
}

void Function::build_init_object_literal_property(BasicBlock& block, Value& object, Bytecode::PropertyKeyTableIndex property, Value& value, CacheIndex shape_cache_index, PropertySlot property_slot)
{
    auto instruction = Instruction::create<Opcode::InitObjectLiteralProperty>();
    instruction->add_operand(&object);
    instruction->add_operand(&value);
    instruction->set_property_key_index(property);
    instruction->set_cache_index(shape_cache_index);
    instruction->set_property_slot(property_slot);

    block.append(move(instruction));
}

void Function::build_cache_object_shape(BasicBlock& block, Value& object, CacheIndex cache_index)
{
    auto instruction = Instruction::create<Opcode::CacheObjectShape>();
    instruction->add_operand(&object);
    instruction->set_cache_index(cache_index);

    block.append(move(instruction));
}

// Special
Value& Function::build_in(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::In, lhs, rhs); }
Value& Function::build_instance_of(BasicBlock& block, Value& lhs, Value& rhs) { return build_binary_op(block, Opcode::InstanceOf, lhs, rhs); }

// Arguments
Value& Function::build_create_arguments(BasicBlock& block, Bytecode::Op::ArgumentsKind kind, bool is_immutable)
{
    auto instruction = Instruction::create<Opcode::CreateArguments>();
    instruction->set_arguments_kind(kind);
    instruction->set_is_immutable(is_immutable);
    auto& result = create_value_for_instruction();
    instruction->set_result(&result);
    block.append(move(instruction));
    return result;
}

Value& Function::build_create_rest_params(BasicBlock& block, u32 rest_index)
{
    auto instruction = Instruction::create<Opcode::CreateRestParams>();
    instruction->set_rest_index(rest_index);
    auto& result = create_value_for_instruction();
    instruction->set_result(&result);
    block.append(move(instruction));
    return result;
}

Value& Function::build_get_new_target(BasicBlock& block)
{
    auto instruction = Instruction::create<Opcode::GetNewTarget>();
    auto& result = create_value_for_instruction();
    instruction->set_result(&result);
    block.append(move(instruction));
    return result;
}

// Guard operations (may throw but produce no value)
void Function::build_throw_if_not_object(BasicBlock& block, Value& value)
{
    auto instruction = Instruction::create<Opcode::ThrowIfNotObject>();
    instruction->add_operand(&value);
    block.append(move(instruction));
}

void Function::build_throw_if_nullish(BasicBlock& block, Value& value)
{
    auto instruction = Instruction::create<Opcode::ThrowIfNullish>();
    instruction->add_operand(&value);
    block.append(move(instruction));
}

void Function::build_throw_if_tdz(BasicBlock& block, Value& value)
{
    auto instruction = Instruction::create<Opcode::ThrowIfTDZ>();
    instruction->add_operand(&value);
    block.append(move(instruction));
}

// Iterators
Value& Function::build_get_iterator(BasicBlock& block, Value& iterable)
{
    return build_unary_op(block, Opcode::GetIterator, iterable);
}

Value& Function::build_get_object_property_iterator(BasicBlock& block, Value& object)
{
    return build_unary_op(block, Opcode::GetObjectPropertyIterator, object);
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
    auto instruction = Instruction::create<Opcode::IteratorClose>();
    instruction->add_operand(&iterator);
    block.append(move(instruction));
}

Value& Function::build_iterator_to_array(BasicBlock& block, Value& iterator)
{
    auto instruction = Instruction::create<Opcode::IteratorToArray>();
    instruction->add_operand(&iterator);

    auto& result = create_value_for_instruction();
    result.set_type(Type::Array);
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Copy
Value& Function::build_move(BasicBlock& block, Value& source)
{
    return build_unary_op(block, Opcode::Move, source);
}

// Tuple extraction
Value& Function::build_extract_value(BasicBlock& block, Value& tuple, u32 index)
{
    auto instruction = Instruction::create<Opcode::ExtractValue>();
    instruction->add_operand(&tuple);
    instruction->set_extract_index(index);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// Control flow
void Function::build_jump(BasicBlock& from, BasicBlock& to)
{
    auto instruction = JumpInstruction::create(to);
    CFG::add_predecessor(to, from);
    from.append(move(instruction));
}

void Function::build_branch(BasicBlock& from, Value& condition, BasicBlock& if_true, BasicBlock& if_false)
{
    auto instruction = BranchInstruction::create(&condition, if_true, if_false);
    CFG::add_predecessor(if_true, from);
    CFG::add_predecessor(if_false, from);
    from.append(move(instruction));
}

void Function::build_return(BasicBlock& block, Value& value)
{
    auto instruction = TerminatorInstruction::create<Opcode::Return>();
    instruction->add_operand(&value);
    block.append(move(instruction));
}

void Function::build_end(BasicBlock& block, Value& value)
{
    auto instruction = TerminatorInstruction::create<Opcode::End>();
    instruction->add_operand(&value);
    block.append(move(instruction));
}

void Function::build_throw(BasicBlock& block, Value& value)
{
    auto instruction = TerminatorInstruction::create<Opcode::Throw>();
    instruction->add_operand(&value);
    block.append(move(instruction));
}

// Generators/Async - terminators with result (the resume value)
Value& Function::build_yield(BasicBlock& block, Value& value, BasicBlock* continuation)
{
    auto instruction = TerminatorInstruction::create<Opcode::Yield>();
    instruction->add_operand(&value);
    if (continuation) {
        instruction->set_true_target(continuation);
        CFG::add_predecessor(*continuation, block);
    }

    // The result is the value passed to .next() when the generator resumes
    // For final yields (no continuation), the result is unused but we still create it
    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_await(BasicBlock& block, Value& argument, BasicBlock& continuation)
{
    auto instruction = TerminatorInstruction::create<Opcode::Await>();
    instruction->add_operand(&argument);
    instruction->set_true_target(&continuation);
    CFG::add_predecessor(continuation, block);

    // The result is the resolved value of the awaited promise
    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Function::build_get_completion_fields(BasicBlock& block, Value& completion)
{
    // GetCompletionFields produces a tuple of (type, value)
    // Use ExtractValue to get individual components
    auto instruction = Instruction::create<Opcode::GetCompletionFields>();
    instruction->add_operand(&completion);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

// SSA
Value& Function::build_phi(BasicBlock& block, Vector<Value*> values, Vector<BasicBlock*> predecessors)
{
    VERIFY(values.size() == predecessors.size());

    auto instruction = PhiInstruction::create();

    for (size_t i = 0; i < values.size(); ++i)
        instruction->add_phi_operand(predecessors[i], values[i]);

    auto& result = create_value_for_instruction();
    instruction->set_result(&result);

    // Phi nodes go at the start of the block (before other instructions)
    block.prepend(move(instruction));
    return result;
}

}
