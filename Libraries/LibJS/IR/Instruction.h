/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/Vector.h>
#include <LibJS/Bytecode/IdentifierTable.h>
#include <LibJS/Bytecode/PropertyKeyTable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

enum class Opcode : u8 {
    // Control flow
    Jump,
    Branch,
    Return,
    Throw,

    // SSA
    Phi,

    // Constants
    LoadConstant,
    LoadUndefined,
    LoadNull,

    // Arithmetic
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Exp,
    Negate,
    UnaryPlus,

    // Bitwise
    BitwiseAnd,
    BitwiseOr,
    BitwiseXor,
    BitwiseNot,
    LeftShift,
    RightShift,
    UnsignedRightShift,

    // Comparison
    LessThan,
    LessThanEquals,
    GreaterThan,
    GreaterThanEquals,
    LooselyEquals,
    StrictlyEquals,
    LooselyInequals,
    StrictlyInequals,

    // Type ops
    Typeof,
    TypeofBinding,
    ToBoolean,
    ToNumber,
    ToString,
    ToObject,
    ToInt32,
    ToLength,
    Not,

    // Increment/Decrement
    Increment,
    Decrement,
    PostfixIncrement,
    PostfixDecrement,

    // String ops
    ConcatString,

    // Property access
    GetById,
    GetByValue,
    GetLength,
    PutById,
    PutByValue,
    DeleteById,
    DeleteByValue,
    HasProperty,

    // Calls
    Call,
    Construct,

    // Environment
    GetBinding,
    SetBinding,
    GetGlobal,
    SetGlobal,
    DeleteVariable,

    // Object creation
    NewObject,
    NewArray,
    NewFunction,

    // Special
    In,
    InstanceOf,

    // Iterators
    GetIterator,
    IteratorNext,
    IteratorNextUnpack,
    IteratorClose,
    IteratorToArray,

    // Copy
    Move,
};

constexpr bool is_terminator_opcode(Opcode opcode)
{
    switch (opcode) {
    case Opcode::Jump:
    case Opcode::Branch:
    case Opcode::Return:
    case Opcode::Throw:
        return true;
    default:
        return false;
    }
}

constexpr bool may_throw_opcode(Opcode opcode)
{
    switch (opcode) {
    case Opcode::Jump:
    case Opcode::Branch:
    case Opcode::Return:
    case Opcode::Phi:
    case Opcode::LoadConstant:
    case Opcode::LoadUndefined:
    case Opcode::LoadNull:
    case Opcode::ToBoolean:
    case Opcode::Not:
    case Opcode::Typeof:
    case Opcode::Move:
        return false;
    default:
        return true;
    }
}

JS_API char const* opcode_to_string(Opcode opcode);

class JS_API Instruction {
    AK_MAKE_NONCOPYABLE(Instruction);
    AK_MAKE_NONMOVABLE(Instruction);

public:
    static NonnullOwnPtr<Instruction> create(Opcode opcode);

    Opcode opcode() const { return m_opcode; }

    BasicBlock* parent_block() const { return m_parent_block; }
    void set_parent_block(BasicBlock* block) { m_parent_block = block; }

    Value* result() const { return m_result; }
    void set_result(Value* value) { m_result = value; }

    Vector<Value*> const& operands() const { return m_operands; }
    void add_operand(Value* value);
    void set_operand(size_t index, Value* value);

    // For Branch/Jump
    BasicBlock* true_target() const { return m_true_target; }
    BasicBlock* false_target() const { return m_false_target; }
    void set_true_target(BasicBlock* block) { m_true_target = block; }
    void set_false_target(BasicBlock* block) { m_false_target = block; }

    // For Phi nodes
    Vector<BasicBlock*> const& phi_predecessors() const { return m_phi_predecessors; }
    void add_phi_operand(BasicBlock* predecessor, Value* value);

    // Instruction-specific indices (reuse bytecode tables)
    Bytecode::PropertyKeyTableIndex property_key_index() const { return m_property_key_index; }
    void set_property_key_index(Bytecode::PropertyKeyTableIndex index) { m_property_key_index = index; }

    Bytecode::IdentifierTableIndex identifier_index() const { return m_identifier_index; }
    void set_identifier_index(Bytecode::IdentifierTableIndex index) { m_identifier_index = index; }

    bool is_terminator() const { return is_terminator_opcode(m_opcode); }
    bool may_throw() const { return may_throw_opcode(m_opcode); }

private:
    explicit Instruction(Opcode opcode);

    Opcode m_opcode;
    BasicBlock* m_parent_block { nullptr };
    Value* m_result { nullptr };
    Vector<Value*> m_operands;

    // For Branch/Jump
    BasicBlock* m_true_target { nullptr };
    BasicBlock* m_false_target { nullptr };

    // For Phi
    Vector<BasicBlock*> m_phi_predecessors;

    // Instruction-specific indices
    Bytecode::PropertyKeyTableIndex m_property_key_index;
    Bytecode::IdentifierTableIndex m_identifier_index;
};

}
