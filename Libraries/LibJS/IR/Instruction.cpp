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

NonnullOwnPtr<Instruction> Instruction::create(Opcode opcode)
{
    // Terminators get their own class with CFG target support
    if (is_terminator_opcode(opcode))
        return TerminatorInstruction::create(opcode);
    return adopt_own(*new Instruction(opcode));
}

TerminatorInstruction::TerminatorInstruction(Opcode opcode)
    : Instruction(opcode)
{
    VERIFY(is_terminator_opcode(opcode));
}

NonnullOwnPtr<TerminatorInstruction> TerminatorInstruction::create(Opcode opcode)
{
    VERIFY(is_terminator_opcode(opcode));
    return adopt_own(*new TerminatorInstruction(opcode));
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

void Instruction::add_phi_operand(BasicBlock* predecessor, Value* value)
{
    VERIFY(m_opcode == Opcode::Phi);
    m_phi_predecessors.append(predecessor);
    add_operand(value);
}

char const* opcode_to_string(Opcode opcode)
{
    switch (opcode) {
    case Opcode::Jump:
        return "Jump";
    case Opcode::Branch:
        return "Branch";
    case Opcode::Return:
        return "Return";
    case Opcode::End:
        return "End";
    case Opcode::Throw:
        return "Throw";
    case Opcode::Phi:
        return "Phi";
    case Opcode::LoadConstant:
        return "LoadConstant";
    case Opcode::LoadUndefined:
        return "LoadUndefined";
    case Opcode::LoadNull:
        return "LoadNull";
    case Opcode::Add:
        return "Add";
    case Opcode::Sub:
        return "Sub";
    case Opcode::Mul:
        return "Mul";
    case Opcode::Div:
        return "Div";
    case Opcode::Mod:
        return "Mod";
    case Opcode::Exp:
        return "Exp";
    case Opcode::Negate:
        return "Negate";
    case Opcode::UnaryPlus:
        return "UnaryPlus";
    case Opcode::BitwiseAnd:
        return "BitwiseAnd";
    case Opcode::BitwiseOr:
        return "BitwiseOr";
    case Opcode::BitwiseXor:
        return "BitwiseXor";
    case Opcode::BitwiseNot:
        return "BitwiseNot";
    case Opcode::LeftShift:
        return "LeftShift";
    case Opcode::RightShift:
        return "RightShift";
    case Opcode::UnsignedRightShift:
        return "UnsignedRightShift";
    case Opcode::LessThan:
        return "LessThan";
    case Opcode::LessThanEquals:
        return "LessThanEquals";
    case Opcode::GreaterThan:
        return "GreaterThan";
    case Opcode::GreaterThanEquals:
        return "GreaterThanEquals";
    case Opcode::LooselyEquals:
        return "LooselyEquals";
    case Opcode::StrictlyEquals:
        return "StrictlyEquals";
    case Opcode::LooselyInequals:
        return "LooselyInequals";
    case Opcode::StrictlyInequals:
        return "StrictlyInequals";
    case Opcode::Typeof:
        return "Typeof";
    case Opcode::TypeofBinding:
        return "TypeofBinding";
    case Opcode::ToBoolean:
        return "ToBoolean";
    case Opcode::ToNumber:
        return "ToNumber";
    case Opcode::ToString:
        return "ToString";
    case Opcode::ToObject:
        return "ToObject";
    case Opcode::ToInt32:
        return "ToInt32";
    case Opcode::ToLength:
        return "ToLength";
    case Opcode::Not:
        return "Not";
    case Opcode::IsUndefined:
        return "IsUndefined";
    case Opcode::IsNullish:
        return "IsNullish";
    case Opcode::Increment:
        return "Increment";
    case Opcode::Decrement:
        return "Decrement";
    case Opcode::PostfixIncrement:
        return "PostfixIncrement";
    case Opcode::PostfixDecrement:
        return "PostfixDecrement";
    case Opcode::ConcatString:
        return "ConcatString";
    case Opcode::GetById:
        return "GetById";
    case Opcode::GetByIdWithThis:
        return "GetByIdWithThis";
    case Opcode::GetByValue:
        return "GetByValue";
    case Opcode::GetByValueWithThis:
        return "GetByValueWithThis";
    case Opcode::GetLength:
        return "GetLength";
    case Opcode::PutById:
        return "PutById";
    case Opcode::PutByValue:
        return "PutByValue";
    case Opcode::DeleteById:
        return "DeleteById";
    case Opcode::DeleteByValue:
        return "DeleteByValue";
    case Opcode::HasProperty:
        return "HasProperty";
    case Opcode::GetPrivateById:
        return "GetPrivateById";
    case Opcode::PutPrivateById:
        return "PutPrivateById";
    case Opcode::PutGetterById:
        return "PutGetterById";
    case Opcode::PutSetterById:
        return "PutSetterById";
    case Opcode::PutPrototypeById:
        return "PutPrototypeById";
    case Opcode::PutGetterByIdWithThis:
        return "PutGetterByIdWithThis";
    case Opcode::PutSetterByIdWithThis:
        return "PutSetterByIdWithThis";
    case Opcode::PutPrototypeByIdWithThis:
        return "PutPrototypeByIdWithThis";
    case Opcode::PutGetterByValue:
        return "PutGetterByValue";
    case Opcode::PutSetterByValue:
        return "PutSetterByValue";
    case Opcode::PutPrototypeByValue:
        return "PutPrototypeByValue";
    case Opcode::PutGetterByValueWithThis:
        return "PutGetterByValueWithThis";
    case Opcode::PutSetterByValueWithThis:
        return "PutSetterByValueWithThis";
    case Opcode::PutPrototypeByValueWithThis:
        return "PutPrototypeByValueWithThis";
    case Opcode::PutBySpread:
        return "PutBySpread";
    case Opcode::Call:
        return "Call";
    case Opcode::CallBuiltin:
        return "CallBuiltin";
    case Opcode::CallDirectEval:
        return "CallDirectEval";
    case Opcode::CallWithArgumentArray:
        return "CallWithArgumentArray";
    case Opcode::Construct:
        return "Construct";
    case Opcode::ConstructWithArgumentArray:
        return "ConstructWithArgumentArray";
    case Opcode::SuperCallWithArgumentArray:
        return "SuperCallWithArgumentArray";
    case Opcode::ImportCall:
        return "ImportCall";
    case Opcode::GetCalleeAndThisFromEnvironment:
        return "GetCalleeAndThisFromEnvironment";
    case Opcode::GetBinding:
        return "GetBinding";
    case Opcode::InitializeBinding:
        return "InitializeBinding";
    case Opcode::SetBinding:
        return "SetBinding";
    case Opcode::GetGlobal:
        return "GetGlobal";
    case Opcode::SetGlobal:
        return "SetGlobal";
    case Opcode::CreateVariable:
        return "CreateVariable";
    case Opcode::CreateLexicalEnvironment:
        return "CreateLexicalEnvironment";
    case Opcode::CreateMutableBinding:
        return "CreateMutableBinding";
    case Opcode::CreateImmutableBinding:
        return "CreateImmutableBinding";
    case Opcode::LeaveLexicalEnvironment:
        return "LeaveLexicalEnvironment";
    case Opcode::EnterObjectEnvironment:
        return "EnterObjectEnvironment";
    case Opcode::DeleteVariable:
        return "DeleteVariable";
    case Opcode::ResolveThisBinding:
        return "ResolveThisBinding";
    case Opcode::ResolveSuperBase:
        return "ResolveSuperBase";
    case Opcode::NewObject:
        return "NewObject";
    case Opcode::NewArray:
        return "NewArray";
    case Opcode::NewArrayWithLength:
        return "NewArrayWithLength";
    case Opcode::ArrayAppend:
        return "ArrayAppend";
    case Opcode::NewClass:
        return "NewClass";
    case Opcode::NewFunction:
        return "NewFunction";
    case Opcode::NewRegExp:
        return "NewRegExp";
    case Opcode::InitObjectLiteralProperty:
        return "InitObjectLiteralProperty";
    case Opcode::CacheObjectShape:
        return "CacheObjectShape";
    case Opcode::In:
        return "In";
    case Opcode::InstanceOf:
        return "InstanceOf";
    case Opcode::GetIterator:
        return "GetIterator";
    case Opcode::IteratorNext:
        return "IteratorNext";
    case Opcode::IteratorNextUnpack:
        return "IteratorNextUnpack";
    case Opcode::IteratorClose:
        return "IteratorClose";
    case Opcode::IteratorToArray:
        return "IteratorToArray";
    case Opcode::Move:
        return "Move";
    case Opcode::ExtractValue:
        return "ExtractValue";
    case Opcode::Yield:
        return "Yield";
    case Opcode::Await:
        return "Await";
    case Opcode::GetCompletionFields:
        return "GetCompletionFields";
    case Opcode::CreateArguments:
        return "CreateArguments";
    case Opcode::CreateRestParams:
        return "CreateRestParams";
    case Opcode::GetNewTarget:
        return "GetNewTarget";
    case Opcode::ThrowIfNotObject:
        return "ThrowIfNotObject";
    case Opcode::ThrowIfNullish:
        return "ThrowIfNullish";
    case Opcode::ThrowIfTDZ:
        return "ThrowIfTDZ";
    }
    VERIFY_NOT_REACHED();
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
