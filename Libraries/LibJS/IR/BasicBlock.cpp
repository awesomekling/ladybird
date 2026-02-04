/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
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

void BasicBlock::add_predecessor(BasicBlock* block)
{
    if (!m_predecessors.contains_slow(block))
        m_predecessors.append(block);
}

void BasicBlock::remove_predecessor(BasicBlock* block)
{
    m_predecessors.remove_first_matching([block](auto* b) { return b == block; });
}

bool BasicBlock::is_terminated() const
{
    auto* last = last_instruction();
    return last && last->is_terminator();
}

void BasicBlock::remove_phi_operands_for_predecessor(BasicBlock* predecessor)
{
    for (auto& instr : m_instructions) {
        if (instr->opcode() != Opcode::Phi)
            break;
        auto& phi = static_cast<PhiInstruction&>(*instr);
        phi.remove_incoming_from(predecessor);
    }
}

void BasicBlock::replace_phi_predecessor(BasicBlock* old_pred, BasicBlock* new_pred)
{
    for (auto& instr : m_instructions) {
        if (instr->opcode() != Opcode::Phi)
            break;
        auto& phi = static_cast<PhiInstruction&>(*instr);
        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            if (phi.incoming_block(i) == old_pred) {
                phi.set_incoming_block(i, new_pred);
                break;
            }
        }
    }
}

}
