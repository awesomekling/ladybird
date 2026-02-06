/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Bitmap.h>
#include <AK/NonnullOwnPtr.h>
#include <AK/Vector.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class DominatorTree;

class Loop {
public:
    BasicBlock* header() const { return m_header; }
    bool contains(BasicBlock const* block) const;

    template<typename Callback>
    void for_each_block(Callback callback) const;

private:
    friend class LoopTree;

    BasicBlock* m_header { nullptr };
    Vector<BlockIndex> m_latches;
    AK::Bitmap m_blocks;
    size_t m_block_count { 0 };
    Vector<BasicBlock*> const* m_block_table { nullptr };
};

class LoopTree {
public:
    explicit LoopTree(Function const&, DominatorTree const&);

    Vector<NonnullOwnPtr<Loop>> const& loops() const { return m_loops; }
    Loop const* loop_for_block(BasicBlock const* block) const;

private:
    Vector<NonnullOwnPtr<Loop>> m_loops;
    Vector<Loop*> m_block_to_loop;
    size_t m_block_index_capacity { 0 };
    Vector<BasicBlock*> m_block_table;
};

}

// Template implementations (requires full BasicBlock definition).
#include <LibJS/IR/BasicBlock.h>

namespace JS::IR {

inline bool Loop::contains(BasicBlock const* block) const
{
    auto idx = static_cast<u32>(block->index());
    if (idx >= m_blocks.size())
        return false;
    return m_blocks.get(idx);
}

template<typename Callback>
void Loop::for_each_block(Callback callback) const
{
    for (size_t i = 0; i < m_blocks.size(); ++i) {
        if (m_blocks.get(i))
            callback(*(*m_block_table)[i]);
    }
}

}
