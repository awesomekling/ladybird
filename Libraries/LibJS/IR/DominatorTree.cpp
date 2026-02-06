/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/DominatorTree.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>

namespace JS::IR {

static inline size_t to_index(BlockIndex b) { return static_cast<u32>(b); }

static size_t block_index_capacity(Function const& function)
{
    size_t max_index = 0;
    for (auto const& block : function.basic_blocks())
        max_index = max(max_index, to_index(block->index()) + 1);
    return max_index;
}

static Vector<BasicBlock*> build_block_index_table(Function const& function, size_t capacity)
{
    Vector<BasicBlock*> table;
    table.resize_with_default_value(capacity, nullptr);
    for (auto const& block : function.basic_blocks())
        table[to_index(block->index())] = block.ptr();
    return table;
}

DominatorTree::DominatorTree(Function const& function)
    : m_function(function)
    , m_block_index_capacity(block_index_capacity(function))
    , m_block_table(build_block_index_table(function, m_block_index_capacity))
{
    m_immediate_dominator.resize(m_block_index_capacity);
    compute_reverse_postorder();
    compute_dominators();
}

void DominatorTree::compute_reverse_postorder()
{
    // DFS to compute reverse postorder using explicit stack with frames.

    if (!m_function.entry_block())
        return;

    struct Frame {
        BlockIndex block;
        bool children_pushed { false };
    };

    auto visited = MUST(AK::Bitmap::create(m_block_index_capacity, false));
    Vector<BlockIndex> postorder;
    Vector<Frame> stack;

    auto entry_index = m_function.entry_block()->index();
    visited.set(to_index(entry_index), true);
    stack.append({ entry_index });

    while (!stack.is_empty()) {
        auto& frame = stack.last();

        if (frame.children_pushed) {
            postorder.append(frame.block);
            stack.take_last();
            continue;
        }

        frame.children_pushed = true;

        // NB: Save the block index before pushing children, since appending
        // to the stack may reallocate and invalidate the frame reference.
        auto block_idx = frame.block;
        auto* block = m_block_table[to_index(block_idx)];

        // Push successors (will be processed before we return to this block)
        // Include both control flow successors and exception edges
        CFG::for_each_successor(*block, [&](BasicBlock& target) {
            auto target_idx = to_index(target.index());
            if (!visited.get(target_idx)) {
                visited.set(target_idx, true);
                stack.append({ target.index() });
            }
        });
    }

    // Reverse to get reverse postorder
    m_reverse_postorder.ensure_capacity(postorder.size());
    m_reverse_postorder_blocks.ensure_capacity(postorder.size());
    for (size_t i = postorder.size(); i > 0; --i) {
        m_reverse_postorder.append(postorder[i - 1]);
        m_reverse_postorder_blocks.append(m_block_table[to_index(postorder[i - 1])]);
    }
}

void DominatorTree::compute_dominators()
{
    // Simple iterative algorithm for computing dominators
    // Based on "A Simple, Fast Dominance Algorithm" by Cooper, Harvey, Kennedy

    if (m_reverse_postorder.is_empty())
        return;

    auto entry_idx = m_function.entry_block()->index();

    // Map blocks to their reverse postorder index
    Vector<Optional<size_t>> rpo_index;
    rpo_index.resize(m_block_index_capacity);
    for (size_t i = 0; i < m_reverse_postorder.size(); ++i)
        rpo_index[to_index(m_reverse_postorder[i])] = i;

    // Initialize: entry dominates itself, others undefined
    m_immediate_dominator[to_index(entry_idx)] = entry_idx;

    // Intersect helper: find common dominator of two blocks
    auto intersect = [&](BlockIndex b1, BlockIndex b2) -> Optional<BlockIndex> {
        auto finger1 = b1;
        auto finger2 = b2;

        while (finger1 != finger2) {
            while (rpo_index[to_index(finger1)].value_or(SIZE_MAX) > rpo_index[to_index(finger2)].value_or(SIZE_MAX)) {
                auto const& idom = m_immediate_dominator[to_index(finger1)];
                if (!idom.has_value())
                    return {};
                finger1 = *idom;
            }
            while (rpo_index[to_index(finger2)].value_or(SIZE_MAX) > rpo_index[to_index(finger1)].value_or(SIZE_MAX)) {
                auto const& idom = m_immediate_dominator[to_index(finger2)];
                if (!idom.has_value())
                    return {};
                finger2 = *idom;
            }
        }

        return finger1;
    };

    // Iterate until fixed point
    bool changed = true;
    while (changed) {
        changed = false;

        // Process all blocks except entry in reverse postorder
        for (size_t i = 1; i < m_reverse_postorder.size(); ++i) {
            auto block_idx = m_reverse_postorder[i];
            auto* block = m_block_table[to_index(block_idx)];

            // Find first processed predecessor
            Optional<BlockIndex> new_idom;
            for (auto* pred : block->predecessors()) {
                if (m_immediate_dominator[to_index(pred->index())].has_value()) {
                    if (!new_idom.has_value()) {
                        new_idom = pred->index();
                    } else {
                        new_idom = intersect(*new_idom, pred->index());
                        if (!new_idom.has_value())
                            break;
                    }
                }
            }

            if (new_idom.has_value()) {
                auto const& existing = m_immediate_dominator[to_index(block_idx)];
                if (!existing.has_value() || *existing != *new_idom) {
                    m_immediate_dominator[to_index(block_idx)] = *new_idom;
                    changed = true;
                }
            }
        }
    }
}

void DominatorTree::ensure_dominance_frontiers() const
{
    if (m_dominance_frontiers_computed)
        return;
    m_dominance_frontiers_computed = true;

    // Dominance frontier algorithm from Cytron et al.
    // DF(n) = { y | exists pred p of y such that n dominates p but n does not strictly dominate y }

    m_dominance_frontier.resize(m_block_index_capacity);
    for (auto& bitmap : m_dominance_frontier)
        bitmap = MUST(AK::Bitmap::create(m_block_index_capacity, false));

    for (auto block_idx : m_reverse_postorder) {
        auto* block = m_block_table[to_index(block_idx)];
        auto const& preds = block->predecessors();
        if (preds.size() < 2)
            continue; // Only join points have non-empty dominance frontiers contributed here

        for (auto* pred : preds) {
            auto runner_idx = pred->index();
            auto const& block_idom = m_immediate_dominator[to_index(block_idx)];
            while (m_block_table[to_index(runner_idx)] && (!block_idom.has_value() || runner_idx != *block_idom)) {
                auto runner_i = to_index(runner_idx);
                if (runner_i >= m_block_index_capacity)
                    break; // Not reachable from entry
                m_dominance_frontier[runner_i].set(to_index(block_idx), true);
                auto const& idom = m_immediate_dominator[runner_i];
                if (!idom.has_value())
                    break;
                runner_idx = *idom;
            }
        }
    }
}

BasicBlock* DominatorTree::immediate_dominator(BasicBlock const* block) const
{
    auto idx = to_index(block->index());
    if (idx >= m_immediate_dominator.size())
        return nullptr;
    auto const& idom = m_immediate_dominator[idx];
    if (!idom.has_value())
        return nullptr;
    // Entry block's idom is itself, but we return nullptr for external interface
    if (*idom == block->index())
        return nullptr;
    return m_block_table[to_index(*idom)];
}

bool DominatorTree::strictly_dominates(BasicBlock const* a, BasicBlock const* b) const
{
    if (a == b)
        return false;
    return dominates(a, b);
}

bool DominatorTree::dominates(BasicBlock const* a, BasicBlock const* b) const
{
    if (a == b)
        return true;

    // Walk up the dominator tree from b looking for a
    auto a_idx = a->index();
    auto runner_idx_opt = m_immediate_dominator[to_index(b->index())];
    while (runner_idx_opt.has_value()) {
        auto runner_idx = *runner_idx_opt;
        if (runner_idx == a_idx)
            return true;
        auto const& next = m_immediate_dominator[to_index(runner_idx)];
        if (!next.has_value() || *next == runner_idx)
            break; // Entry block
        runner_idx_opt = next;
    }

    return false;
}

void DominatorTree::ensure_dominator_children() const
{
    if (m_dominator_children_computed)
        return;
    m_dominator_children_computed = true;

    // Build the dominator tree children by grouping blocks by their immediate dominator
    m_dominator_children.resize(m_block_index_capacity);

    for (auto block_idx : m_reverse_postorder) {
        auto* block = m_block_table[to_index(block_idx)];
        auto* idom = immediate_dominator(block);
        if (idom)
            m_dominator_children[to_index(idom->index())].append(block_idx);
    }
}

}
