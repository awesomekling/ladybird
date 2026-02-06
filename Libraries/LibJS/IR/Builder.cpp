/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Builder.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

BasicBlock& Builder::current_block()
{
    VERIFY(m_insertion_block);
    return *m_insertion_block;
}

Value& Builder::emit_with_result(NonnullOwnPtr<Instruction> instruction)
{
    auto& result = m_function.create_value_for_instruction();
    instruction->set_result(&result);
    current_block().append(move(instruction));
    // NB: recompute_result_type() needs m_function to resolve operand types,
    // so we call it after append() which sets the function pointer.
    result.defining_instruction()->recompute_result_type();
    return result;
}

Value& Builder::build_binary_op(Opcode opcode, Value& lhs, Value& rhs)
{
    return emit_with_result(BinaryOpInstruction::create(opcode, &lhs, &rhs));
}

Value& Builder::build_unary_op(Opcode opcode, Value& operand)
{
    return emit_with_result(UnaryOpInstruction::create(opcode, &operand));
}

// Arithmetic
Value& Builder::build_add(Value& lhs, Value& rhs) { return build_binary_op(Opcode::Add, lhs, rhs); }
Value& Builder::build_sub(Value& lhs, Value& rhs) { return build_binary_op(Opcode::Sub, lhs, rhs); }
Value& Builder::build_mul(Value& lhs, Value& rhs) { return build_binary_op(Opcode::Mul, lhs, rhs); }
Value& Builder::build_div(Value& lhs, Value& rhs) { return build_binary_op(Opcode::Div, lhs, rhs); }
Value& Builder::build_mod(Value& lhs, Value& rhs) { return build_binary_op(Opcode::Mod, lhs, rhs); }
Value& Builder::build_exp(Value& lhs, Value& rhs) { return build_binary_op(Opcode::Exp, lhs, rhs); }
Value& Builder::build_negate(Value& operand) { return build_unary_op(Opcode::Negate, operand); }
Value& Builder::build_unary_plus(Value& operand) { return build_unary_op(Opcode::UnaryPlus, operand); }

// Bitwise
Value& Builder::build_bitwise_and(Value& lhs, Value& rhs) { return build_binary_op(Opcode::BitwiseAnd, lhs, rhs); }
Value& Builder::build_bitwise_or(Value& lhs, Value& rhs) { return build_binary_op(Opcode::BitwiseOr, lhs, rhs); }
Value& Builder::build_bitwise_xor(Value& lhs, Value& rhs) { return build_binary_op(Opcode::BitwiseXor, lhs, rhs); }
Value& Builder::build_bitwise_not(Value& operand) { return build_unary_op(Opcode::BitwiseNot, operand); }
Value& Builder::build_left_shift(Value& lhs, Value& rhs) { return build_binary_op(Opcode::LeftShift, lhs, rhs); }
Value& Builder::build_right_shift(Value& lhs, Value& rhs) { return build_binary_op(Opcode::RightShift, lhs, rhs); }
Value& Builder::build_unsigned_right_shift(Value& lhs, Value& rhs) { return build_binary_op(Opcode::UnsignedRightShift, lhs, rhs); }

// Comparison
Value& Builder::build_less_than(Value& lhs, Value& rhs) { return build_binary_op(Opcode::LessThan, lhs, rhs); }
Value& Builder::build_less_than_equals(Value& lhs, Value& rhs) { return build_binary_op(Opcode::LessThanEquals, lhs, rhs); }
Value& Builder::build_greater_than(Value& lhs, Value& rhs) { return build_binary_op(Opcode::GreaterThan, lhs, rhs); }
Value& Builder::build_greater_than_equals(Value& lhs, Value& rhs) { return build_binary_op(Opcode::GreaterThanEquals, lhs, rhs); }
Value& Builder::build_loosely_equals(Value& lhs, Value& rhs) { return build_binary_op(Opcode::LooselyEquals, lhs, rhs); }
Value& Builder::build_strictly_equals(Value& lhs, Value& rhs) { return build_binary_op(Opcode::StrictlyEquals, lhs, rhs); }
Value& Builder::build_loosely_inequals(Value& lhs, Value& rhs) { return build_binary_op(Opcode::LooselyInequals, lhs, rhs); }
Value& Builder::build_strictly_inequals(Value& lhs, Value& rhs) { return build_binary_op(Opcode::StrictlyInequals, lhs, rhs); }

// Type ops
Value& Builder::build_typeof(Value& operand) { return build_unary_op(Opcode::Typeof, operand); }

Value& Builder::build_typeof_binding(Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::TypeofBinding>();
    instruction->set_identifier_index(identifier);
    return emit_with_result(move(instruction));
}

Value& Builder::build_to_boolean(Value& operand) { return build_unary_op(Opcode::ToBoolean, operand); }
Value& Builder::build_to_number(Value& operand) { return build_unary_op(Opcode::ToNumber, operand); }
Value& Builder::build_to_numeric(Value& operand) { return build_unary_op(Opcode::ToNumeric, operand); }
Value& Builder::build_to_string(Value& operand) { return build_unary_op(Opcode::ToString, operand); }
Value& Builder::build_to_object(Value& operand) { return build_unary_op(Opcode::ToObject, operand); }
Value& Builder::build_to_int32(Value& operand) { return build_unary_op(Opcode::ToInt32, operand); }
Value& Builder::build_to_length(Value& operand) { return build_unary_op(Opcode::ToLength, operand); }
Value& Builder::build_not(Value& operand) { return build_unary_op(Opcode::Not, operand); }
Value& Builder::build_is_undefined(Value& operand) { return build_unary_op(Opcode::IsUndefined, operand); }
Value& Builder::build_is_nullish(Value& operand) { return build_unary_op(Opcode::IsNullish, operand); }

// Increment/Decrement
Value& Builder::build_increment(Value& operand) { return build_unary_op(Opcode::Increment, operand); }
Value& Builder::build_decrement(Value& operand) { return build_unary_op(Opcode::Decrement, operand); }
Value& Builder::build_postfix_increment(Value& operand) { return build_unary_op(Opcode::PostfixIncrement, operand); }
Value& Builder::build_postfix_decrement(Value& operand) { return build_unary_op(Opcode::PostfixDecrement, operand); }

// String ops
Value& Builder::build_concat_string(Value& lhs, Value& rhs) { return build_binary_op(Opcode::ConcatString, lhs, rhs); }

// Constants
Value& Builder::build_load_constant(JS::Value constant)
{
    auto instruction = Instruction::create<Opcode::LoadConstant>();
    auto& constant_value = m_function.create_constant(constant);
    instruction->add_operand(&constant_value);
    return emit_with_result(move(instruction));
}

Value& Builder::build_load_undefined()
{
    return emit_with_result(Instruction::create<Opcode::LoadUndefined>());
}

Value& Builder::build_load_null()
{
    return emit_with_result(Instruction::create<Opcode::LoadNull>());
}

// Property access
Value& Builder::build_get_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = GetByIdInstruction::create(&base, property);
    instruction->set_base_identifier(base_identifier);
    return emit_with_result(move(instruction));
}

Value& Builder::build_get_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property)
{
    auto instruction = Instruction::create<Opcode::GetByIdWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->set_property_key_index(property);
    return emit_with_result(move(instruction));
}

Value& Builder::build_get_by_value(Value& base, Value& property, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::GetByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->set_base_identifier(base_identifier);
    return emit_with_result(move(instruction));
}

Value& Builder::build_get_by_value_with_this(Value& base, Value& this_value, Value& property)
{
    auto instruction = Instruction::create<Opcode::GetByValueWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&property);
    return emit_with_result(move(instruction));
}

Value& Builder::build_get_length(Value& base, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto& result = build_unary_op(Opcode::GetLength, base);
    result.defining_instruction()->set_base_identifier(base_identifier);
    return result;
}

void Builder::build_put_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Value& value, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::PutById>();
    instruction->add_operand(&base);
    instruction->add_operand(&value);
    instruction->set_property_key_index(property);
    instruction->set_base_identifier(base_identifier);

    current_block().append(move(instruction));
}

void Builder::build_put_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& value)
{
    auto instruction = Instruction::create<Opcode::PutByIdWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&value);
    instruction->set_property_key_index(property);

    current_block().append(move(instruction));
}

void Builder::build_put_by_value(Value& base, Value& property, Value& value, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Opcode::PutByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&value);
    instruction->set_base_identifier(base_identifier);

    current_block().append(move(instruction));
}

void Builder::build_put_by_value_with_this(Value& base, Value& this_value, Value& property, Value& value)
{
    auto instruction = Instruction::create<Opcode::PutByValueWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&property);
    instruction->add_operand(&value);

    current_block().append(move(instruction));
}

Value& Builder::build_delete_by_id(Value& base, Bytecode::PropertyKeyTableIndex property)
{
    auto instruction = Instruction::create<Opcode::DeleteById>();
    instruction->add_operand(&base);
    instruction->set_property_key_index(property);
    return emit_with_result(move(instruction));
}

Value& Builder::build_delete_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property)
{
    auto instruction = Instruction::create<Opcode::DeleteByIdWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->set_property_key_index(property);
    return emit_with_result(move(instruction));
}

Value& Builder::build_delete_by_value(Value& base, Value& property)
{
    auto instruction = Instruction::create<Opcode::DeleteByValue>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    return emit_with_result(move(instruction));
}

Value& Builder::build_delete_by_value_with_this(Value& base, Value& this_value, Value& property)
{
    auto instruction = Instruction::create<Opcode::DeleteByValueWithThis>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&property);
    return emit_with_result(move(instruction));
}

Value& Builder::build_has_property(Value& object, Value& property)
{
    auto instruction = Instruction::create<Opcode::HasProperty>();
    instruction->add_operand(&object);
    instruction->add_operand(&property);
    return emit_with_result(move(instruction));
}

Value& Builder::build_has_private_id(Value& base, Bytecode::IdentifierTableIndex property)
{
    auto instruction = Instruction::create<Opcode::HasPrivateId>();
    instruction->add_operand(&base);
    instruction->set_identifier_index(property);
    return emit_with_result(move(instruction));
}

Value& Builder::build_get_private_by_id(Value& base, Bytecode::IdentifierTableIndex property)
{
    auto instruction = Instruction::create<Opcode::GetPrivateById>();
    instruction->add_operand(&base);
    instruction->set_identifier_index(property);
    return emit_with_result(move(instruction));
}

void Builder::build_put_private_by_id(Value& base, Bytecode::IdentifierTableIndex property, Value& value)
{
    auto instruction = Instruction::create<Opcode::PutPrivateById>();
    instruction->add_operand(&base);
    instruction->add_operand(&value);
    instruction->set_identifier_index(property);
    current_block().append(move(instruction));
}

// NB: The put-accessor families (getter/setter/prototype) share identical
// operand layouts. Template helpers avoid repeating the same body 3 times.

template<Opcode Op>
static void build_put_accessor_by_id(BasicBlock& block,
    Value& base, Value& accessor, Bytecode::PropertyKeyTableIndex property,
    Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Op>();
    instruction->add_operand(&base);
    instruction->add_operand(&accessor);
    instruction->set_property_key_index(property);
    instruction->set_base_identifier(base_identifier);
    block.append(move(instruction));
}

template<Opcode Op>
static void build_put_accessor_by_id_with_this(BasicBlock& block,
    Value& base, Value& this_value, Value& accessor, Bytecode::PropertyKeyTableIndex property)
{
    auto instruction = Instruction::create<Op>();
    instruction->add_operand(&base);
    instruction->add_operand(&this_value);
    instruction->add_operand(&accessor);
    instruction->set_property_key_index(property);
    block.append(move(instruction));
}

template<Opcode Op>
static void build_put_accessor_by_value(BasicBlock& block,
    Value& base, Value& property, Value& accessor,
    Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    auto instruction = Instruction::create<Op>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&accessor);
    instruction->set_base_identifier(base_identifier);
    block.append(move(instruction));
}

template<Opcode Op>
static void build_put_accessor_by_value_with_this(BasicBlock& block,
    Value& base, Value& property, Value& this_value, Value& accessor)
{
    auto instruction = Instruction::create<Op>();
    instruction->add_operand(&base);
    instruction->add_operand(&property);
    instruction->add_operand(&this_value);
    instruction->add_operand(&accessor);
    block.append(move(instruction));
}

void Builder::build_put_getter_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Value& getter, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    build_put_accessor_by_id<Opcode::PutGetterById>(current_block(), base, getter, property, base_identifier);
}

void Builder::build_put_setter_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Value& setter, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    build_put_accessor_by_id<Opcode::PutSetterById>(current_block(), base, setter, property, base_identifier);
}

void Builder::build_put_prototype_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Value& prototype, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    build_put_accessor_by_id<Opcode::PutPrototypeById>(current_block(), base, prototype, property, base_identifier);
}

void Builder::build_put_getter_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& getter)
{
    build_put_accessor_by_id_with_this<Opcode::PutGetterByIdWithThis>(current_block(), base, this_value, getter, property);
}

void Builder::build_put_setter_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& setter)
{
    build_put_accessor_by_id_with_this<Opcode::PutSetterByIdWithThis>(current_block(), base, this_value, setter, property);
}

void Builder::build_put_prototype_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& prototype)
{
    build_put_accessor_by_id_with_this<Opcode::PutPrototypeByIdWithThis>(current_block(), base, this_value, prototype, property);
}

void Builder::build_put_getter_by_value(Value& base, Value& property, Value& getter, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    build_put_accessor_by_value<Opcode::PutGetterByValue>(current_block(), base, property, getter, base_identifier);
}

void Builder::build_put_setter_by_value(Value& base, Value& property, Value& setter, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    build_put_accessor_by_value<Opcode::PutSetterByValue>(current_block(), base, property, setter, base_identifier);
}

void Builder::build_put_prototype_by_value(Value& base, Value& property, Value& prototype, Optional<Bytecode::IdentifierTableIndex> base_identifier)
{
    build_put_accessor_by_value<Opcode::PutPrototypeByValue>(current_block(), base, property, prototype, base_identifier);
}

void Builder::build_put_getter_by_value_with_this(Value& base, Value& property, Value& this_value, Value& getter)
{
    build_put_accessor_by_value_with_this<Opcode::PutGetterByValueWithThis>(current_block(), base, property, this_value, getter);
}

void Builder::build_put_setter_by_value_with_this(Value& base, Value& property, Value& this_value, Value& setter)
{
    build_put_accessor_by_value_with_this<Opcode::PutSetterByValueWithThis>(current_block(), base, property, this_value, setter);
}

void Builder::build_put_prototype_by_value_with_this(Value& base, Value& property, Value& this_value, Value& prototype)
{
    build_put_accessor_by_value_with_this<Opcode::PutPrototypeByValueWithThis>(current_block(), base, property, this_value, prototype);
}

void Builder::build_put_by_spread(Value& base, Value& source)
{
    auto instruction = Instruction::create<Opcode::PutBySpread>();
    instruction->add_operand(&base);
    instruction->add_operand(&source);
    current_block().append(move(instruction));
}

// Calls
Value& Builder::build_call(Value& callee, Value& this_value, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = CallInstruction::create(Opcode::Call, &callee, &this_value);
    for (auto* arg : arguments)
        instruction->add_operand(arg);
    instruction->set_expression_string(expression_string);
    return emit_with_result(move(instruction));
}

Value& Builder::build_call_builtin(Value& callee, Value& this_value, Span<Value*> arguments, Bytecode::Builtin builtin, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = CallInstruction::create(Opcode::CallBuiltin, &callee, &this_value);
    for (auto* arg : arguments)
        instruction->add_operand(arg);
    instruction->set_builtin(builtin);
    instruction->set_expression_string(expression_string);
    return emit_with_result(move(instruction));
}

Value& Builder::build_call_direct_eval(Value& callee, Value& this_value, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = CallInstruction::create(Opcode::CallDirectEval, &callee, &this_value);
    for (auto* arg : arguments)
        instruction->add_operand(arg);
    instruction->set_expression_string(expression_string);
    return emit_with_result(move(instruction));
}

Value& Builder::build_call_with_argument_array(Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = CallInstruction::create(Opcode::CallWithArgumentArray, &callee, &this_value);
    instruction->add_operand(&arguments);
    instruction->set_expression_string(expression_string);
    return emit_with_result(move(instruction));
}

Value& Builder::build_construct(Value& callee, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = Instruction::create<Opcode::Construct>();
    instruction->add_operand(&callee);
    for (auto* arg : arguments)
        instruction->add_operand(arg);
    instruction->set_expression_string(expression_string);
    return emit_with_result(move(instruction));
}

Value& Builder::build_construct_with_argument_array(Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string)
{
    auto instruction = Instruction::create<Opcode::ConstructWithArgumentArray>();
    instruction->add_operand(&callee);
    instruction->add_operand(&this_value);
    instruction->add_operand(&arguments);
    instruction->set_expression_string(expression_string);
    return emit_with_result(move(instruction));
}

Value& Builder::build_super_call_with_argument_array(Value& arguments, bool is_synthetic)
{
    auto instruction = Instruction::create<Opcode::SuperCallWithArgumentArray>();
    instruction->add_operand(&arguments);
    instruction->set_is_synthetic(is_synthetic);
    return emit_with_result(move(instruction));
}

Value& Builder::build_import_call(Value& specifier, Value& options)
{
    auto instruction = Instruction::create<Opcode::ImportCall>();
    instruction->add_operand(&specifier);
    instruction->add_operand(&options);
    return emit_with_result(move(instruction));
}

// Environment
Value& Builder::build_get_callee_and_this_from_environment(Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::GetCalleeAndThisFromEnvironment>();
    instruction->set_identifier_index(identifier);
    return emit_with_result(move(instruction));
}

void Builder::build_create_variable(Bytecode::IdentifierTableIndex identifier, Bytecode::Op::EnvironmentMode mode, bool is_immutable, bool is_global, bool is_strict)
{
    auto instruction = Instruction::create<Opcode::CreateVariable>();
    instruction->set_identifier_index(identifier);
    instruction->set_environment_mode(mode);
    instruction->set_is_immutable(is_immutable);
    instruction->set_is_global(is_global);
    instruction->set_is_strict(is_strict);

    current_block().append(move(instruction));
}

Value& Builder::build_create_lexical_environment(u32 capacity)
{
    auto instruction = Instruction::create<Opcode::CreateLexicalEnvironment>();
    instruction->set_capacity(capacity);
    return emit_with_result(move(instruction));
}

void Builder::build_create_mutable_binding(Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict)
{
    auto instruction = Instruction::create<Opcode::CreateMutableBinding>();
    instruction->add_operand(&environment);
    instruction->set_identifier_index(identifier);
    instruction->set_is_strict(is_strict);
    current_block().append(move(instruction));
}

void Builder::build_create_immutable_binding(Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict)
{
    auto instruction = Instruction::create<Opcode::CreateImmutableBinding>();
    instruction->add_operand(&environment);
    instruction->set_identifier_index(identifier);
    instruction->set_is_strict(is_strict);
    current_block().append(move(instruction));
}

void Builder::build_leave_lexical_environment()
{
    auto instruction = Instruction::create<Opcode::LeaveLexicalEnvironment>();
    current_block().append(move(instruction));
}

void Builder::build_create_private_environment()
{
    auto instruction = Instruction::create<Opcode::CreatePrivateEnvironment>();
    current_block().append(move(instruction));
}

void Builder::build_leave_private_environment()
{
    auto instruction = Instruction::create<Opcode::LeavePrivateEnvironment>();
    current_block().append(move(instruction));
}

void Builder::build_add_private_name(Bytecode::IdentifierTableIndex name)
{
    auto instruction = Instruction::create<Opcode::AddPrivateName>();
    instruction->set_identifier_index(name);
    current_block().append(move(instruction));
}

void Builder::build_create_variable_environment(u32 capacity)
{
    auto instruction = Instruction::create<Opcode::CreateVariableEnvironment>();
    instruction->set_capacity(capacity);
    current_block().append(move(instruction));
}

void Builder::build_enter_object_environment(Value& object)
{
    auto instruction = Instruction::create<Opcode::EnterObjectEnvironment>();
    instruction->add_operand(&object);
    current_block().append(move(instruction));
}

void Builder::build_resolve_this_binding()
{
    auto instruction = Instruction::create<Opcode::ResolveThisBinding>();
    current_block().append(move(instruction));
}

Value& Builder::build_resolve_super_base()
{
    return emit_with_result(Instruction::create<Opcode::ResolveSuperBase>());
}

Value& Builder::build_get_binding(Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::GetBinding>();
    instruction->set_identifier_index(identifier);
    return emit_with_result(move(instruction));
}

void Builder::build_initialize_binding(Bytecode::IdentifierTableIndex identifier, Value& value, Bytecode::Op::EnvironmentMode mode)
{
    auto instruction = Instruction::create<Opcode::InitializeBinding>();
    instruction->set_identifier_index(identifier);
    instruction->set_environment_mode(mode);
    instruction->add_operand(&value);

    current_block().append(move(instruction));
}

void Builder::build_set_binding(Bytecode::IdentifierTableIndex identifier, Value& value, Bytecode::Op::EnvironmentMode mode)
{
    auto instruction = Instruction::create<Opcode::SetBinding>();
    instruction->set_identifier_index(identifier);
    instruction->set_environment_mode(mode);
    instruction->add_operand(&value);

    current_block().append(move(instruction));
}

Value& Builder::build_get_global(Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::GetGlobal>();
    instruction->set_identifier_index(identifier);
    return emit_with_result(move(instruction));
}

void Builder::build_set_global(Bytecode::IdentifierTableIndex identifier, Value& value)
{
    auto instruction = Instruction::create<Opcode::SetGlobal>();
    instruction->set_identifier_index(identifier);
    instruction->add_operand(&value);

    current_block().append(move(instruction));
}

Value& Builder::build_delete_variable(Bytecode::IdentifierTableIndex identifier)
{
    auto instruction = Instruction::create<Opcode::DeleteVariable>();
    instruction->set_identifier_index(identifier);
    return emit_with_result(move(instruction));
}

// Object creation
Value& Builder::build_new_object()
{
    return emit_with_result(Instruction::create<Opcode::NewObject>());
}

Value& Builder::build_new_array(Span<Value*> elements)
{
    auto instruction = Instruction::create<Opcode::NewArray>();
    for (auto* element : elements)
        instruction->add_operand(element);
    return emit_with_result(move(instruction));
}

Value& Builder::build_new_array_with_length(Value& length)
{
    auto instruction = Instruction::create<Opcode::NewArrayWithLength>();
    instruction->add_operand(&length);
    return emit_with_result(move(instruction));
}

void Builder::build_array_append(Value& array, Value& value, bool is_spread)
{
    auto instruction = Instruction::create<Opcode::ArrayAppend>();
    instruction->add_operand(&array);
    instruction->add_operand(&value);
    instruction->set_is_spread(is_spread);
    current_block().append(move(instruction));
}

Value& Builder::build_new_class(Value* super_class, Span<Value*> element_keys)
{
    auto instruction = Instruction::create<Opcode::NewClass>();
    instruction->add_operand(super_class);
    for (auto* key : element_keys)
        instruction->add_operand(key);
    return emit_with_result(move(instruction));
}

Value& Builder::build_new_function(Value* home_object)
{
    auto instruction = Instruction::create<Opcode::NewFunction>();
    if (home_object)
        instruction->add_operand(home_object);
    return emit_with_result(move(instruction));
}

Value& Builder::build_new_regexp(Bytecode::StringTableIndex source, Bytecode::StringTableIndex flags, Bytecode::RegexTableIndex regex)
{
    auto instruction = Instruction::create<Opcode::NewRegExp>();
    instruction->set_regex_source_index(source);
    instruction->set_regex_flags_index(flags);
    instruction->set_regex_index(regex);
    return emit_with_result(move(instruction));
}

Value& Builder::build_get_template_object(Span<Value*> strings, u32 cache_index)
{
    auto instruction = Instruction::create<Opcode::GetTemplateObject>();
    for (auto* string : strings)
        instruction->add_operand(string);
    instruction->set_cache_index(CacheIndex { cache_index });
    return emit_with_result(move(instruction));
}

void Builder::build_init_object_literal_property(Value& object, Bytecode::PropertyKeyTableIndex property, Value& value, CacheIndex shape_cache_index, PropertySlot property_slot)
{
    auto instruction = Instruction::create<Opcode::InitObjectLiteralProperty>();
    instruction->add_operand(&object);
    instruction->add_operand(&value);
    instruction->set_property_key_index(property);
    instruction->set_cache_index(shape_cache_index);
    instruction->set_property_slot(property_slot);

    current_block().append(move(instruction));
}

void Builder::build_cache_object_shape(Value& object, CacheIndex cache_index)
{
    auto instruction = Instruction::create<Opcode::CacheObjectShape>();
    instruction->add_operand(&object);
    instruction->set_cache_index(cache_index);

    current_block().append(move(instruction));
}

Value& Builder::build_copy_object_excluding_properties(Value& from_object, Span<Value*> excluded_names)
{
    auto instruction = Instruction::create<Opcode::CopyObjectExcludingProperties>();
    instruction->add_operand(&from_object);
    for (auto* name : excluded_names)
        instruction->add_operand(name);
    return emit_with_result(move(instruction));
}

// Special
Value& Builder::build_in(Value& lhs, Value& rhs) { return build_binary_op(Opcode::In, lhs, rhs); }
Value& Builder::build_instance_of(Value& lhs, Value& rhs) { return build_binary_op(Opcode::InstanceOf, lhs, rhs); }

// Arguments
Value& Builder::build_create_arguments(Bytecode::Op::ArgumentsKind kind, bool is_immutable, bool needs_dst)
{
    auto instruction = Instruction::create<Opcode::CreateArguments>();
    instruction->set_arguments_kind(kind);
    instruction->set_is_immutable(is_immutable);
    instruction->set_create_arguments_needs_dst(needs_dst);
    return emit_with_result(move(instruction));
}

Value& Builder::build_create_rest_params(u32 rest_index)
{
    auto instruction = Instruction::create<Opcode::CreateRestParams>();
    instruction->set_rest_index(rest_index);
    return emit_with_result(move(instruction));
}

Value& Builder::build_get_new_target()
{
    return emit_with_result(Instruction::create<Opcode::GetNewTarget>());
}

// Exception handling
Value& Builder::build_catch()
{
    return emit_with_result(Instruction::create<Opcode::Catch>());
}

void Builder::build_enter_unwind_context(BasicBlock& entry_point)
{
    auto& block = current_block();
    auto instruction = JumpInstruction::create_enter_unwind_context(entry_point);
    CFG::add_predecessor(entry_point, block);
    block.append(move(instruction));
}

void Builder::build_leave_unwind_context()
{
    auto instruction = Instruction::create<Opcode::LeaveUnwindContext>();
    current_block().append(move(instruction));
}

void Builder::build_schedule_jump(BasicBlock& finalizer, BasicBlock& deferred_target)
{
    auto& block = current_block();
    auto instruction = TerminatorInstruction::create<Opcode::ScheduleJump>();
    instruction->set_true_target(&finalizer);
    instruction->set_false_target(&deferred_target);
    CFG::add_predecessor(finalizer, block);
    block.append(move(instruction));
}

void Builder::build_leave_finally()
{
    auto instruction = Instruction::create<Opcode::LeaveFinally>();
    current_block().append(move(instruction));
}

void Builder::build_restore_scheduled_jump()
{
    auto instruction = Instruction::create<Opcode::RestoreScheduledJump>();
    current_block().append(move(instruction));
}

void Builder::build_set_saved_return_value(Value& value)
{
    auto instruction = Instruction::create<Opcode::SetSavedReturnValue>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

void Builder::build_prepare_yield(Value& value)
{
    auto instruction = Instruction::create<Opcode::PrepareYield>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

Value& Builder::build_get_exception()
{
    return emit_with_result(Instruction::create<Opcode::GetException>());
}

void Builder::build_set_exception(Value& value)
{
    auto instruction = Instruction::create<Opcode::SetException>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

// Guard operations (may throw but produce no value)
void Builder::build_throw_if_not_object(Value& value)
{
    auto instruction = Instruction::create<Opcode::ThrowIfNotObject>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

void Builder::build_throw_if_nullish(Value& value)
{
    auto instruction = Instruction::create<Opcode::ThrowIfNullish>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

void Builder::build_throw_if_tdz(Value& value)
{
    auto instruction = Instruction::create<Opcode::ThrowIfTDZ>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

// Iterators
Value& Builder::build_get_iterator(Value& iterable)
{
    return build_unary_op(Opcode::GetIterator, iterable);
}

Value& Builder::build_get_object_property_iterator(Value& object)
{
    return build_unary_op(Opcode::GetObjectPropertyIterator, object);
}

Value& Builder::build_iterator_next(Value& iterator)
{
    return build_unary_op(Opcode::IteratorNext, iterator);
}

Value& Builder::build_iterator_next_unpack(Value& iterator)
{
    return build_unary_op(Opcode::IteratorNextUnpack, iterator);
}

void Builder::build_iterator_close(Value& iterator)
{
    auto instruction = Instruction::create<Opcode::IteratorClose>();
    instruction->add_operand(&iterator);
    current_block().append(move(instruction));
}

void Builder::build_async_iterator_close(Value& iterator)
{
    auto instruction = Instruction::create<Opcode::AsyncIteratorClose>();
    instruction->add_operand(&iterator);
    current_block().append(move(instruction));
}

Value& Builder::build_iterator_to_array(Value& iterator)
{
    auto instruction = Instruction::create<Opcode::IteratorToArray>();
    instruction->add_operand(&iterator);
    return emit_with_result(move(instruction));
}

// Copy
Value& Builder::build_move(Value& source)
{
    return build_unary_op(Opcode::Move, source);
}

// Tuple extraction
Value& Builder::build_extract_value(Value& tuple, u32 index)
{
    auto instruction = Instruction::create<Opcode::ExtractValue>();
    instruction->add_operand(&tuple);
    instruction->set_extract_index(index);
    return emit_with_result(move(instruction));
}

// Control flow
void Builder::build_jump(BasicBlock& to)
{
    auto& block = current_block();
    auto instruction = JumpInstruction::create(to);
    CFG::add_predecessor(to, block);
    block.append(move(instruction));
}

void Builder::build_continue_pending_unwind(BasicBlock& resume_target)
{
    auto& block = current_block();
    auto instruction = JumpInstruction::create_continue_pending_unwind(resume_target);
    CFG::add_predecessor(resume_target, block);
    block.append(move(instruction));
}

void Builder::build_branch(Value& condition, BasicBlock& if_true, BasicBlock& if_false)
{
    auto& block = current_block();
    auto instruction = BranchInstruction::create(&condition, if_true, if_false);
    CFG::add_predecessor(if_true, block);
    CFG::add_predecessor(if_false, block);
    block.append(move(instruction));
}

void Builder::build_return(Value& value)
{
    auto instruction = TerminatorInstruction::create<Opcode::Return>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

void Builder::build_end(Value& value)
{
    auto instruction = TerminatorInstruction::create<Opcode::End>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

void Builder::build_throw(Value& value)
{
    auto instruction = TerminatorInstruction::create<Opcode::Throw>();
    instruction->add_operand(&value);
    current_block().append(move(instruction));
}

// Generators/Async - terminators with result (the resume value)
Value& Builder::build_yield(Value& value, BasicBlock* continuation)
{
    auto& block = current_block();
    auto instruction = TerminatorInstruction::create<Opcode::Yield>();
    instruction->add_operand(&value);
    if (continuation) {
        instruction->set_true_target(continuation);
        CFG::add_predecessor(*continuation, block);
    }

    // The result is the value passed to .next() when the generator resumes
    // For final yields (no continuation), the result is unused but we still create it
    auto& result = m_function.create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Builder::build_await(Value& argument, BasicBlock& continuation)
{
    auto& block = current_block();
    auto instruction = TerminatorInstruction::create<Opcode::Await>();
    instruction->add_operand(&argument);
    instruction->set_true_target(&continuation);
    CFG::add_predecessor(continuation, block);

    // The result is the resolved value of the awaited promise
    auto& result = m_function.create_value_for_instruction();
    instruction->set_result(&result);

    block.append(move(instruction));
    return result;
}

Value& Builder::build_get_completion_fields(Value& completion)
{
    auto instruction = Instruction::create<Opcode::GetCompletionFields>();
    instruction->add_operand(&completion);
    return emit_with_result(move(instruction));
}

void Builder::build_set_completion_type(Value& completion, Completion::Type type)
{
    auto instruction = Instruction::create<Opcode::SetCompletionType>();
    instruction->add_operand(&completion);
    instruction->set_completion_type(type);
    current_block().append(move(instruction));
}

Value& Builder::build_new_type_error(Bytecode::StringTableIndex error_string)
{
    auto instruction = Instruction::create<Opcode::NewTypeError>();
    instruction->set_string_table_index(error_string);
    return emit_with_result(move(instruction));
}

// SSA
Value& Builder::build_phi(Vector<Value*> values, Vector<BlockIndex> predecessors)
{
    VERIFY(values.size() == predecessors.size());

    auto instruction = PhiInstruction::create();

    for (size_t i = 0; i < values.size(); ++i)
        instruction->add_phi_operand(predecessors[i], values[i]);

    auto& result = m_function.create_value_for_instruction();
    instruction->set_result(&result);

    // Phi nodes go at the start of the block (before other instructions)
    current_block().prepend(move(instruction));
    return result;
}

}
