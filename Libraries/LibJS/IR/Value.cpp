/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/Value.h>

namespace JS::IR {

Value::Value(Kind kind, u32 index)
    : m_kind(kind)
    , m_index(index)
{
}

void Value::add_use(Instruction* instruction)
{
    m_uses.append(instruction);
}

void Value::remove_use(Instruction* instruction)
{
    m_uses.remove_first_matching([instruction](auto* i) { return i == instruction; });
}

NonnullOwnPtr<Value> Value::create_for_instruction(u32 index)
{
    return adopt_own(*new Value(Kind::Instruction, index));
}

NonnullOwnPtr<Value> Value::create_for_parameter(u32 index, u32 parameter_index)
{
    auto value = adopt_own(*new Value(Kind::Parameter, index));
    // NB: For parameters, we don't set anything special - they are just values
    // without a defining instruction.
    (void)parameter_index;
    return value;
}

NonnullOwnPtr<Value> Value::create_for_constant(u32 index, JS::Value constant)
{
    auto value = adopt_own(*new Value(Kind::Constant, index));
    value->m_constant_value = constant;

    // Set type based on constant value.
    if (constant.is_undefined())
        value->m_type = Type::Undefined;
    else if (constant.is_null())
        value->m_type = Type::Null;
    else if (constant.is_boolean())
        value->m_type = Type::Boolean;
    else if (constant.is_int32())
        value->m_type = Type::Int32;
    else if (constant.is_number())
        value->m_type = Type::Number;
    else if (constant.is_string())
        value->m_type = Type::String;
    else if (constant.is_symbol())
        value->m_type = Type::Symbol;
    else if (constant.is_bigint())
        value->m_type = Type::BigInt;
    else if (constant.is_object())
        value->m_type = Type::Object;

    return value;
}

}
