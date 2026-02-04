/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/Vector.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/IR/Type.h>
#include <LibJS/Runtime/Value.h>

namespace JS::IR {

class JS_API Value {
    AK_MAKE_NONCOPYABLE(Value);
    AK_MAKE_NONMOVABLE(Value);

public:
    enum class Kind : u8 {
        Instruction,
        Parameter,
        Constant,
        This,
    };

    Kind kind() const { return m_kind; }
    Type type() const { return m_type; }
    ValueIndex index() const { return m_index; }

    bool is_instruction() const { return m_kind == Kind::Instruction; }
    bool is_parameter() const { return m_kind == Kind::Parameter; }
    bool is_constant() const { return m_kind == Kind::Constant; }
    bool is_this() const { return m_kind == Kind::This; }

    Instruction* defining_instruction() const { return m_defining_instruction; }

    Vector<Instruction*> const& uses() const { return m_uses; }
    void add_use(Instruction* instruction);
    void remove_use(Instruction* instruction);
    void replace_all_uses_with(Value* replacement);

    JS::Value constant_value() const
    {
        VERIFY(is_constant());
        return m_constant_value;
    }

    u32 parameter_index() const
    {
        VERIFY(is_parameter());
        return m_parameter_index;
    }

    void set_type(Type type) { m_type = type; }

    static NonnullOwnPtr<Value> create_for_instruction(ValueIndex index);
    static NonnullOwnPtr<Value> create_for_parameter(ValueIndex index, u32 parameter_index);
    static NonnullOwnPtr<Value> create_for_constant(ValueIndex index, JS::Value constant);
    static NonnullOwnPtr<Value> create_for_this(ValueIndex index);

private:
    friend class Instruction;

    void set_defining_instruction(Instruction* instruction) { m_defining_instruction = instruction; }

    Value(Kind kind, ValueIndex index);

    Kind m_kind;
    Type m_type { Type::Unknown };
    ValueIndex m_index;
    u32 m_parameter_index { 0 };
    Instruction* m_defining_instruction { nullptr };
    Vector<Instruction*> m_uses;
    JS::Value m_constant_value;
};

}
