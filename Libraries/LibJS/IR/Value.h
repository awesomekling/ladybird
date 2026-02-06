/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/Optional.h>
#include <AK/Vector.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/IR/Type.h>
#include <LibJS/Runtime/Value.h>

namespace JS::IR {

struct Use {
    InstructionIndex instruction;
    u32 operand_slot;
    bool operator==(Use const&) const = default;
};

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

    ReadonlySpan<Use> uses() const { return m_uses.span(); }
    void add_use(InstructionIndex instruction, u32 operand_slot);
    void remove_use(InstructionIndex instruction, u32 operand_slot);
    void replace_all_uses_with(Value* replacement);

    JS::Value constant_value() const
    {
        VERIFY(is_constant());
        return m_constant_value;
    }

    // Returns the truthiness of a constant value, or empty if not determinable.
    // Use this for constant folding of branch conditions.
    Optional<bool> constant_truthiness() const
    {
        if (!is_constant())
            return {};

        auto const& cv = m_constant_value;
        if (cv.is_boolean())
            return cv.as_bool();
        if (cv.is_int32())
            return cv.as_i32() != 0;
        if (cv.is_undefined() || cv.is_null())
            return false;
        if (cv.is_double())
            return !cv.is_nan() && cv.as_double() != 0;
        return {};
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
    friend class Function;
    friend class Instruction;

    void set_defining_instruction(Instruction* instruction) { m_defining_instruction = instruction; }
    void set_parent_function(Function* function) { m_function = function; }

    Value(Kind kind, ValueIndex index);

    Kind m_kind;
    Type m_type { Type::Unknown };
    ValueIndex m_index;
    u32 m_parameter_index { 0 };
    Function* m_function { nullptr };
    Instruction* m_defining_instruction { nullptr };
    Vector<Use> m_uses;
    JS::Value m_constant_value;
};

}
