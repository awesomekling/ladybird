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

// Computes post-dominator tree and post-dominance frontiers for a control flow graph.
// Uses the same iterative Cooper-Harvey-Kennedy algorithm as DominatorTree,
// but operates on the reverse CFG (predecessors become successors, exit blocks
// become entries).
//
// A block X post-dominates block Y if every path from Y to an exit block
// passes through X.
class PostDominatorTree {
public:
    explicit PostDominatorTree(Function const& function);

    // Returns the immediate post-dominator of a block, or nullptr for exit blocks.
    BasicBlock* immediate_post_dominator(BasicBlock const* block) const;

    // Returns true if a post-dominates b (a == b or a strictly post-dominates b).
    bool post_dominates(BasicBlock const* a, BasicBlock const* b) const;

    // Returns true if a strictly post-dominates b (a != b and a post-dominates b).
    bool strictly_post_dominates(BasicBlock const* a, BasicBlock const* b) const;

    // Iterate over each block in the post-dominance frontier of the given block.
    template<typename Callback>
    void for_each_post_dominance_frontier_block(BasicBlock const* block, Callback callback) const;

private:
    void compute_reverse_postorder();
    void compute_post_dominators();
    void ensure_post_dominance_frontiers() const;

    Function const& m_function;
    size_t m_block_index_capacity { 0 };
    Vector<BasicBlock*> m_block_table;

    // Virtual exit node index (one past the last real block index).
    size_t m_virtual_exit_index { 0 };

    // Immediate post-dominator for each block (indexed by block index).
    // Uses size_t to accommodate the virtual exit node.
    Vector<Optional<size_t>> m_immediate_post_dominator;

    // Reverse postorder of the reverse CFG (starting from virtual exit).
    Vector<size_t> m_reverse_postorder;

    mutable Vector<AK::Bitmap> m_post_dominance_frontier;
    mutable bool m_post_dominance_frontiers_computed { false };
};

}

// Template implementations (requires full BasicBlock definition).
#include <LibJS/IR/BasicBlock.h>

namespace JS::IR {

template<typename Callback>
void PostDominatorTree::for_each_post_dominance_frontier_block(BasicBlock const* block, Callback callback) const
{
    ensure_post_dominance_frontiers();
    auto index = static_cast<u32>(block->index());
    if (index >= m_post_dominance_frontier.size())
        return;
    auto const& bitmap = m_post_dominance_frontier[index];
    for (size_t i = 0; i < bitmap.size(); ++i) {
        if (bitmap.get(i) && i < m_block_table.size() && m_block_table[i])
            callback(*m_block_table[i]);
    }
}

}
