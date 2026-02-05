/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Type.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

Instruction::Instruction(Opcode opcode)
    : m_opcode(opcode)
{
}

TerminatorInstruction::TerminatorInstruction(Opcode opcode)
    : Instruction(opcode)
{
    VERIFY(is_terminator_opcode(opcode));
}

JumpInstruction::JumpInstruction(Opcode opcode, BasicBlock& target)
    : TerminatorInstruction(opcode)
{
    VERIFY(opcode == Opcode::Jump || opcode == Opcode::ContinuePendingUnwind || opcode == Opcode::EnterUnwindContext);
    set_true_target(&target);
}

NonnullOwnPtr<JumpInstruction> JumpInstruction::create(BasicBlock& target)
{
    return adopt_own(*new JumpInstruction(Opcode::Jump, target));
}

NonnullOwnPtr<JumpInstruction> JumpInstruction::create_continue_pending_unwind(BasicBlock& resume_target)
{
    return adopt_own(*new JumpInstruction(Opcode::ContinuePendingUnwind, resume_target));
}

NonnullOwnPtr<JumpInstruction> JumpInstruction::create_enter_unwind_context(BasicBlock& entry_point)
{
    return adopt_own(*new JumpInstruction(Opcode::EnterUnwindContext, entry_point));
}

BranchInstruction::BranchInstruction(Value* condition, BasicBlock& true_target, BasicBlock& false_target)
    : TerminatorInstruction(Opcode::Branch)
{
    add_operand(condition);
    set_true_target(&true_target);
    set_false_target(&false_target);
}

NonnullOwnPtr<BranchInstruction> BranchInstruction::create(Value* condition, BasicBlock& true_target, BasicBlock& false_target)
{
    return adopt_own(*new BranchInstruction(condition, true_target, false_target));
}

PhiInstruction::PhiInstruction()
    : Instruction(Opcode::Phi)
{
}

NonnullOwnPtr<PhiInstruction> PhiInstruction::create()
{
    return adopt_own(*new PhiInstruction());
}

void PhiInstruction::add_incoming(BasicBlock* predecessor, Value* value)
{
    add_phi_operand(predecessor, value);
}

void PhiInstruction::remove_incoming(size_t index)
{
    remove_phi_operand(index);
}

void PhiInstruction::remove_incoming_from(BasicBlock* predecessor)
{
    for (size_t i = phi_predecessors().size(); i > 0; --i) {
        if (phi_predecessors()[i - 1] == predecessor) {
            remove_phi_operand(i - 1);
            return;
        }
    }
}

Value* PhiInstruction::incoming_value_for(BasicBlock const& predecessor) const
{
    for (size_t i = 0; i < incoming_count(); ++i) {
        if (incoming_block(i) == &predecessor)
            return incoming_value(i);
    }
    return nullptr;
}

void PhiInstruction::set_incoming_value_for(BasicBlock const& predecessor, Value* value)
{
    for (size_t i = 0; i < incoming_count(); ++i) {
        if (incoming_block(i) == &predecessor) {
            set_incoming_value(i, value);
            return;
        }
    }
    VERIFY_NOT_REACHED();
}

void PhiInstruction::set_incoming_block(size_t index, BasicBlock* block)
{
    set_phi_predecessor(index, block);
}

void PhiInstruction::set_incoming_value(size_t index, Value* value)
{
    set_operand(index, value);
}

CallInstruction::CallInstruction(Opcode opcode, Value* callee, Value* this_value)
    : Instruction(opcode)
{
    VERIFY(is_call_opcode(opcode));
    add_operand(callee);
    add_operand(this_value);
}

NonnullOwnPtr<CallInstruction> CallInstruction::create(Opcode opcode, Value* callee, Value* this_value)
{
    VERIFY(is_call_opcode(opcode));
    return adopt_own(*new CallInstruction(opcode, callee, this_value));
}

GetByIdInstruction::GetByIdInstruction(Value* base, Bytecode::PropertyKeyTableIndex property)
    : Instruction(Opcode::GetById)
{
    add_operand(base);
    set_property_key_index(property);
}

NonnullOwnPtr<GetByIdInstruction> GetByIdInstruction::create(Value* base, Bytecode::PropertyKeyTableIndex property)
{
    return adopt_own(*new GetByIdInstruction(base, property));
}

BinaryOpInstruction::BinaryOpInstruction(Opcode opcode, Value* lhs, Value* rhs)
    : Instruction(opcode)
{
    add_operand(lhs);
    add_operand(rhs);
}

NonnullOwnPtr<BinaryOpInstruction> BinaryOpInstruction::create(Opcode opcode, Value* lhs, Value* rhs)
{
    return adopt_own(*new BinaryOpInstruction(opcode, lhs, rhs));
}

UnaryOpInstruction::UnaryOpInstruction(Opcode opcode, Value* operand)
    : Instruction(opcode)
{
    add_operand(operand);
}

NonnullOwnPtr<UnaryOpInstruction> UnaryOpInstruction::create(Opcode opcode, Value* operand)
{
    return adopt_own(*new UnaryOpInstruction(opcode, operand));
}

void Instruction::set_result(Value* value)
{
    // Only opcodes that produce results should have a result set.
    // Setting nullptr is always allowed (to clear a result).
    if (value)
        VERIFY(opcode_has_result(m_opcode));
    m_result = value;
    if (value)
        value->set_defining_instruction(this);
}

void Instruction::add_operand(Value* value)
{
    m_operands.append(value);
    if (value)
        value->add_use(this);
}

void Instruction::set_operand(size_t index, Value* value)
{
    if (index < m_operands.size() && m_operands[index])
        m_operands[index]->remove_use(this);

    if (index >= m_operands.size())
        m_operands.resize(index + 1);

    m_operands[index] = value;
    if (value)
        value->add_use(this);
}

void Instruction::clear_operand_uses()
{
    for (auto* operand : m_operands) {
        if (operand)
            operand->remove_use(this);
    }
}

void Instruction::remove_phi_operand(size_t index)
{
    if (m_operands[index])
        m_operands[index]->remove_use(this);
    m_phi_predecessors.remove(index);
    m_operands.remove(index);
}

void Instruction::add_phi_operand(BasicBlock* predecessor, Value* value)
{
    VERIFY(m_opcode == Opcode::Phi);
    m_phi_predecessors.append(predecessor);
    add_operand(value);
}

void Instruction::recompute_result_type()
{
    if (!m_result)
        return;

    auto operand_type = [&](size_t i) -> Type {
        if (i < m_operands.size() && m_operands[i])
            return m_operands[i]->type();
        return Type::Unknown;
    };

    switch (m_opcode) {
    // Unary: inherit operand type
    case Opcode::Move:
        m_result->set_type(operand_type(0));
        break;

    // Always Int32
    case Opcode::BitwiseNot:
    case Opcode::ToInt32:
        m_result->set_type(Type::Int32);
        break;

    // Always Number
    case Opcode::ToNumber:
    case Opcode::UnaryPlus:
    case Opcode::Negate:
    case Opcode::Increment:
    case Opcode::Decrement:
    case Opcode::PostfixIncrement:
    case Opcode::PostfixDecrement:
    case Opcode::UnsignedRightShift:
        m_result->set_type(Type::Number);
        break;

    // Preserve numeric type
    case Opcode::ToNumeric: {
        auto t = operand_type(0);
        if (t == Type::Int32)
            m_result->set_type(Type::Int32);
        else if (t == Type::Number)
            m_result->set_type(Type::Number);
        else if (t == Type::BigInt)
            m_result->set_type(Type::BigInt);
        else
            m_result->set_type(Type::Unknown);
        break;
    }

    // Always Boolean
    case Opcode::ToBoolean:
    case Opcode::Not:
    case Opcode::IsUndefined:
    case Opcode::IsNullish:
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
        m_result->set_type(Type::Boolean);
        break;

    // Always String
    case Opcode::Typeof:
    case Opcode::ToString:
    case Opcode::ConcatString:
        m_result->set_type(Type::String);
        break;

    // Bitwise binary ops always Int32
    case Opcode::BitwiseAnd:
    case Opcode::BitwiseOr:
    case Opcode::BitwiseXor:
    case Opcode::LeftShift:
    case Opcode::RightShift:
        m_result->set_type(Type::Int32);
        break;

    // Arithmetic binary ops produce Number when both operands are safe numeric
    case Opcode::Add:
    case Opcode::Sub:
    case Opcode::Mul:
    case Opcode::Div:
    case Opcode::Mod:
    case Opcode::Exp:
        if (is_safe_numeric_type(operand_type(0)) && is_safe_numeric_type(operand_type(1)))
            m_result->set_type(Type::Number);
        else
            m_result->set_type(Type::Unknown);
        break;

    default:
        break;
    }
}

bool Instruction::try_invert_comparison()
{
    switch (m_opcode) {
    case Opcode::StrictlyEquals:
        m_opcode = Opcode::StrictlyInequals;
        return true;
    case Opcode::StrictlyInequals:
        m_opcode = Opcode::StrictlyEquals;
        return true;
    case Opcode::LooselyEquals:
        m_opcode = Opcode::LooselyInequals;
        return true;
    case Opcode::LooselyInequals:
        m_opcode = Opcode::LooselyEquals;
        return true;
    case Opcode::LessThan:
        m_opcode = Opcode::GreaterThanEquals;
        return true;
    case Opcode::LessThanEquals:
        m_opcode = Opcode::GreaterThan;
        return true;
    case Opcode::GreaterThan:
        m_opcode = Opcode::LessThanEquals;
        return true;
    case Opcode::GreaterThanEquals:
        m_opcode = Opcode::LessThan;
        return true;
    default:
        return false;
    }
}

// Check if all operands are safe primitive types (no ToPrimitive calls possible)
static bool all_operands_are_safe_primitives(Instruction const& instruction)
{
    for (auto* operand : instruction.operands()) {
        if (!operand)
            continue;
        if (!is_safe_primitive_type(operand->type()))
            return false;
    }
    return true;
}

// Check if all operands are safe numeric types (no throws, no user code)
static bool all_operands_are_safe_numerics(Instruction const& instruction)
{
    for (auto* operand : instruction.operands()) {
        if (!operand)
            continue;
        if (!is_safe_numeric_type(operand->type()))
            return false;
    }
    return true;
}

// Opcodes that may call user code via ToPrimitive when operands are objects
static bool opcode_may_call_user_code_on_objects(Opcode opcode)
{
    switch (opcode) {
    // Arithmetic operations call ToNumber -> ToPrimitive on objects
    case Opcode::Add:
    case Opcode::Sub:
    case Opcode::Mul:
    case Opcode::Div:
    case Opcode::Mod:
    case Opcode::Exp:
    case Opcode::Negate:
    case Opcode::UnaryPlus:
    case Opcode::ToNumeric:
    case Opcode::Increment:
    case Opcode::Decrement:
    case Opcode::PostfixIncrement:
    case Opcode::PostfixDecrement:
    // Bitwise operations call ToInt32 -> ToNumber -> ToPrimitive on objects
    case Opcode::BitwiseAnd:
    case Opcode::BitwiseOr:
    case Opcode::BitwiseXor:
    case Opcode::BitwiseNot:
    case Opcode::LeftShift:
    case Opcode::RightShift:
    case Opcode::UnsignedRightShift:
    // Relational comparisons call ToPrimitive on objects
    case Opcode::LessThan:
    case Opcode::LessThanEquals:
    case Opcode::GreaterThan:
    case Opcode::GreaterThanEquals:
    // Loose equality can call ToPrimitive
    case Opcode::LooselyEquals:
    case Opcode::LooselyInequals:
        return true;
    default:
        return false;
    }
}

bool Instruction::has_side_effects() const
{
    // First check opcode-level side effects
    if (!has_side_effects_opcode(m_opcode))
        return false;

    // For ops that may call user code on objects, check if operands are safe primitives
    if (opcode_may_call_user_code_on_objects(m_opcode)) {
        if (all_operands_are_safe_primitives(*this))
            return false;
    }

    return true;
}

bool Instruction::is_pure() const
{
    // First check opcode-level purity
    if (is_pure_opcode(m_opcode))
        return true;

    // For ops that may call user code on objects, check if operands are safe primitives
    if (opcode_may_call_user_code_on_objects(m_opcode)) {
        if (all_operands_are_safe_primitives(*this))
            return true;
    }

    return false;
}

bool Instruction::is_hoistable() const
{
    // First check opcode-level hoistability
    if (is_hoistable_opcode(m_opcode))
        return true;

    // For numeric ops, check if operands are safe numeric types (excludes String, BigInt, Symbol)
    // These operations are safe to hoist when we know no user code or throws can occur.
    switch (m_opcode) {
    case Opcode::Add:
    case Opcode::Sub:
    case Opcode::Mul:
    case Opcode::Div:
    case Opcode::Mod:
    case Opcode::Exp:
    case Opcode::Negate:
    case Opcode::UnaryPlus:
    case Opcode::BitwiseAnd:
    case Opcode::BitwiseOr:
    case Opcode::BitwiseXor:
    case Opcode::BitwiseNot:
    case Opcode::LeftShift:
    case Opcode::RightShift:
    case Opcode::UnsignedRightShift:
        if (all_operands_are_safe_numerics(*this))
            return true;
        break;
    default:
        break;
    }

    return false;
}

}
