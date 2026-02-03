/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Instruction.h>

namespace JS::IR {

BasicBlock::BasicBlock(u32 index, String name)
    : m_index(index)
    , m_name(move(name))
{
}

NonnullOwnPtr<BasicBlock> BasicBlock::create(u32 index, String name)
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

}
