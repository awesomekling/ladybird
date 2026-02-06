/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Bitmap.h>
#include <AK/Vector.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

// Computes dominator tree and dominance frontiers for a control flow graph.
// Uses a simple iterative algorithm for dominator computation.
class DominatorTree {
public:
    explicit DominatorTree(Function const& function);

    // Returns the immediate dominator of a block, or nullptr for the entry block
    BasicBlock* immediate_dominator(BasicBlock const* block) const;

    // Returns true if a strictly dominates b (a != b and a dominates b)
    bool strictly_dominates(BasicBlock const* a, BasicBlock const* b) const;

    // Returns true if a dominates b (a == b or a strictly dominates b)
    bool dominates(BasicBlock const* a, BasicBlock const* b) const;

    // Iterate over each block in the dominance frontier of the given block.
    template<typename Callback>
    void for_each_frontier_block(BasicBlock const* block, Callback callback) const;

    // Iterate over each child in the dominator tree of the given block.
    template<typename Callback>
    void for_each_dominator_child(BasicBlock const* block, Callback callback) const;

    // Returns all blocks in the function in reverse postorder (useful for dataflow)
    Vector<BasicBlock*> const& reverse_postorder() const { return m_reverse_postorder_blocks; }

private:
    void compute_reverse_postorder();
    void compute_dominators();
    void ensure_dominance_frontiers() const;
    void ensure_dominator_children() const;

    Function const& m_function;
    size_t m_block_index_capacity { 0 };
    Vector<BasicBlock*> m_block_table;
    Vector<Optional<BlockIndex>> m_immediate_dominator;
    mutable Vector<AK::Bitmap> m_dominance_frontier;
    mutable Vector<Vector<BlockIndex>> m_dominator_children;
    Vector<BlockIndex> m_reverse_postorder;
    Vector<BasicBlock*> m_reverse_postorder_blocks;
    mutable bool m_dominance_frontiers_computed { false };
    mutable bool m_dominator_children_computed { false };
};

}

// Template implementations (requires full BasicBlock definition).
#include <LibJS/IR/BasicBlock.h>

namespace JS::IR {

template<typename Callback>
void DominatorTree::for_each_frontier_block(BasicBlock const* block, Callback callback) const
{
    ensure_dominance_frontiers();
    auto idx = static_cast<u32>(block->index());
    if (idx >= m_dominance_frontier.size())
        return;
    auto const& bitmap = m_dominance_frontier[idx];
    for (size_t i = 0; i < bitmap.size(); ++i) {
        if (bitmap.get(i))
            callback(*m_block_table[i]);
    }
}

template<typename Callback>
void DominatorTree::for_each_dominator_child(BasicBlock const* block, Callback callback) const
{
    ensure_dominator_children();
    auto idx = static_cast<u32>(block->index());
    if (idx >= m_dominator_children.size())
        return;
    for (auto child_index : m_dominator_children[idx])
        callback(*m_block_table[static_cast<u32>(child_index)]);
}

}
