/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

Value::Value(Kind kind, ValueIndex index)
    : m_kind(kind)
    , m_index(index)
{
}

void Value::add_use(InstructionIndex instruction, u32 operand_slot)
{
    m_uses.append({ instruction, operand_slot });
}

void Value::remove_use(InstructionIndex instruction, u32 operand_slot)
{
    m_uses.remove_first_matching([&](Use const& use) {
        return use.instruction == instruction && use.operand_slot == operand_slot;
    });
}

void Value::replace_all_uses_with(Value* replacement)
{
    // Make a copy since we're modifying the use list as we iterate
    auto uses_copy = m_uses;
    for (auto const& use : uses_copy) {
        auto* instruction = m_function->instruction_by_index(use.instruction);
        instruction->set_operand(use.operand_slot, replacement);
    }
}

NonnullOwnPtr<Value> Value::create_for_instruction(ValueIndex index)
{
    return adopt_own(*new Value(Kind::Instruction, index));
}

NonnullOwnPtr<Value> Value::create_for_parameter(ValueIndex index, u32 parameter_index)
{
    auto value = adopt_own(*new Value(Kind::Parameter, index));
    value->m_parameter_index = parameter_index;
    return value;
}

NonnullOwnPtr<Value> Value::create_for_constant(ValueIndex index, JS::Value constant)
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

NonnullOwnPtr<Value> Value::create_for_this(ValueIndex index)
{
    return adopt_own(*new Value(Kind::This, index));
}

}
