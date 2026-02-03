/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

Instruction::Instruction(Opcode opcode)
    : m_opcode(opcode)
{
}

NonnullOwnPtr<Instruction> Instruction::create(Opcode opcode)
{
    return adopt_own(*new Instruction(opcode));
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
    case Opcode::GetByValue:
        return "GetByValue";
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
    case Opcode::DeleteVariable:
        return "DeleteVariable";
    case Opcode::NewObject:
        return "NewObject";
    case Opcode::NewArray:
        return "NewArray";
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
    }
    VERIFY_NOT_REACHED();
}

}
