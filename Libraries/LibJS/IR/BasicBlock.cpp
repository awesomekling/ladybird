/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>

namespace JS::IR {

BasicBlock::BasicBlock(BlockIndex index, String name)
    : m_index(index)
    , m_name(move(name))
{
}

NonnullOwnPtr<BasicBlock> BasicBlock::create(BlockIndex index, String name)
{
    return adopt_own(*new BasicBlock(index, move(name)));
}

void BasicBlock::append(NonnullOwnPtr<Instruction> instruction)
{
    VERIFY(!is_terminated());
    instruction->set_parent_block(this);
    instruction->set_parent_function(m_parent_function);
    auto instruction_index = m_parent_function->register_instruction(move(instruction));
    m_instructions.append(instruction_index);
}

void BasicBlock::prepend(NonnullOwnPtr<Instruction> instruction)
{
    instruction->set_parent_block(this);
    instruction->set_parent_function(m_parent_function);
    auto instruction_index = m_parent_function->register_instruction(move(instruction));
    m_instructions.prepend(instruction_index);
}

Instruction* BasicBlock::last_instruction() const
{
    if (m_instructions.is_empty())
        return nullptr;
    return m_parent_function->instruction_by_index(m_instructions.last());
}

TerminatorInstruction* BasicBlock::terminator() const
{
    auto* last = last_instruction();
    if (!last || !last->is_terminator())
        return nullptr;
    return static_cast<TerminatorInstruction*>(last);
}

void BasicBlock::remove_instruction(size_t index)
{
    VERIFY(index < m_instructions.size());
    m_parent_function->instruction_by_index(m_instructions[index])->clear_operand_uses();
    m_instructions.remove(index);
}

void BasicBlock::remove_terminator()
{
    VERIFY(is_terminated());
    m_parent_function->instruction_by_index(m_instructions.last())->clear_operand_uses();
    m_instructions.remove(m_instructions.size() - 1);
}

InstructionIndex BasicBlock::take_instruction(size_t index)
{
    VERIFY(index < m_instructions.size());
    auto instruction_index = m_instructions[index];
    m_instructions.remove(index);
    auto* instruction = m_parent_function->instruction_by_index(instruction_index);
    instruction->set_parent_block(nullptr);
    return instruction_index;
}

Vector<InstructionIndex> BasicBlock::take_all_instructions()
{
    for (auto instruction_index : m_instructions) {
        auto* instruction = m_parent_function->instruction_by_index(instruction_index);
        instruction->set_parent_block(nullptr);
    }
    return move(m_instructions);
}

void BasicBlock::insert_before_terminator(NonnullOwnPtr<Instruction> instruction)
{
    VERIFY(is_terminated());
    VERIFY(!instruction->is_terminator());
    instruction->set_parent_block(this);
    instruction->set_parent_function(m_parent_function);
    auto instruction_index = m_parent_function->register_instruction(move(instruction));
    m_instructions.insert(m_instructions.size() - 1, instruction_index);
}

void BasicBlock::insert_before_terminator(InstructionIndex index)
{
    VERIFY(is_terminated());
    auto* instruction = m_parent_function->instruction_by_index(index);
    VERIFY(!instruction->is_terminator());
    instruction->set_parent_block(this);
    m_instructions.insert(m_instructions.size() - 1, index);
}

void BasicBlock::append_instruction(InstructionIndex index)
{
    auto* instruction = m_parent_function->instruction_by_index(index);
    instruction->set_parent_block(this);
    m_instructions.append(index);
}

void BasicBlock::remove_instructions_if(AK::Function<bool(Instruction const&)> predicate)
{
    m_instructions.remove_all_matching([&](auto instruction_index) {
        auto* instruction = m_parent_function->instruction_by_index(instruction_index);
        if (predicate(*instruction)) {
            instruction->clear_operand_uses();
            return true;
        }
        return false;
    });
}

void BasicBlock::clear_instructions()
{
    for (auto instruction_index : m_instructions)
        m_parent_function->instruction_by_index(instruction_index)->clear_operand_uses();
    m_instructions.clear();
}

BasicBlock* BasicBlock::exception_handler() const
{
    if (!m_exception_handler.has_value())
        return nullptr;
    return m_parent_function->block_by_index(*m_exception_handler);
}

BasicBlock* BasicBlock::finalizer() const
{
    if (!m_finalizer.has_value())
        return nullptr;
    return m_parent_function->block_by_index(*m_finalizer);
}

void BasicBlock::add_predecessor(BlockIndex block)
{
    if (!m_predecessors.contains_slow(block))
        m_predecessors.append(block);
}

void BasicBlock::remove_predecessor(BlockIndex block)
{
    m_predecessors.remove_first_matching([block](auto b) { return b == block; });
}

bool BasicBlock::is_terminated() const
{
    auto* last = last_instruction();
    return last && last->is_terminator();
}

void BasicBlock::remove_phi_operands_for_predecessor(BlockIndex predecessor)
{
    for_each_phi([&](PhiInstruction& phi) {
        phi.remove_incoming_from(predecessor);
    });
}

void BasicBlock::replace_phi_predecessor(BlockIndex old_pred, BlockIndex new_pred)
{
    for_each_phi([&](PhiInstruction& phi) {
        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            if (phi.incoming_block(i) == old_pred) {
                phi.set_incoming_block(i, new_pred);
                break;
            }
        }
    });
}

}
