/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
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

}
