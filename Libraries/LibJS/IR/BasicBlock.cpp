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
    m_instructions.append(move(instruction));
}

void BasicBlock::prepend(NonnullOwnPtr<Instruction> instruction)
{
    instruction->set_parent_block(this);
    m_instructions.prepend(move(instruction));
}

Instruction* BasicBlock::last_instruction() const
{
    if (m_instructions.is_empty())
        return nullptr;
    return m_instructions.last().ptr();
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
    m_instructions[index]->clear_operand_uses();
    m_instructions.remove(index);
}

void BasicBlock::remove_terminator()
{
    VERIFY(is_terminated());
    m_instructions.last()->clear_operand_uses();
    m_instructions.remove(m_instructions.size() - 1);
}

NonnullOwnPtr<Instruction> BasicBlock::take_instruction(size_t index)
{
    VERIFY(index < m_instructions.size());
    auto instruction = m_instructions.take(index);
    instruction->set_parent_block(nullptr);
    return instruction;
}

Vector<NonnullOwnPtr<Instruction>> BasicBlock::take_all_instructions()
{
    for (auto& instruction : m_instructions)
        instruction->set_parent_block(nullptr);
    return move(m_instructions);
}

void BasicBlock::insert_before_terminator(NonnullOwnPtr<Instruction> instruction)
{
    VERIFY(is_terminated());
    VERIFY(!instruction->is_terminator());
    instruction->set_parent_block(this);
    m_instructions.insert(m_instructions.size() - 1, move(instruction));
}

void BasicBlock::remove_instructions_if(AK::Function<bool(Instruction const&)> predicate)
{
    m_instructions.remove_all_matching([&](auto const& instruction) {
        if (predicate(*instruction)) {
            instruction->clear_operand_uses();
            return true;
        }
        return false;
    });
}

void BasicBlock::clear_instructions()
{
    for (auto& instruction : m_instructions)
        instruction->clear_operand_uses();
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
