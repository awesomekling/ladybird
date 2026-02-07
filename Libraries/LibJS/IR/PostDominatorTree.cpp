/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/PostDominatorTree.h>

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

// Returns true if this block is an exit block (Return, End, or Throw terminator).
static bool is_exit_block(BasicBlock const& block)
{
    auto* terminator = block.terminator();
    if (!terminator)
        return false;
    switch (terminator->opcode()) {
    case Opcode::Return:
    case Opcode::End:
    case Opcode::Throw:
        return true;
    default:
        return false;
    }
}

PostDominatorTree::PostDominatorTree(Function const& function)
    : m_function(function)
    , m_block_index_capacity(block_index_capacity(function))
    , m_block_table(build_block_index_table(function, m_block_index_capacity))
    , m_virtual_exit_index(m_block_index_capacity) // Virtual exit is one past last real block
{
    // Size arrays to include the virtual exit node.
    m_immediate_post_dominator.resize(m_block_index_capacity + 1);
    compute_reverse_postorder();
    compute_post_dominators();
}

void PostDominatorTree::compute_reverse_postorder()
{
    // DFS on the reverse CFG starting from the virtual exit node.
    // In the reverse CFG:
    //   - The virtual exit node's successors are all real exit blocks
    //   - A real block's successors are its predecessors in the forward CFG

    struct Frame {
        size_t block_index;
        bool children_pushed { false };
    };

    auto total_size = m_block_index_capacity + 1; // +1 for virtual exit
    auto visited = MUST(AK::Bitmap::create(total_size, false));
    Vector<size_t> postorder;
    Vector<Frame> stack;

    // Start from the virtual exit node
    visited.set(m_virtual_exit_index, true);
    stack.append({ m_virtual_exit_index });

    while (!stack.is_empty()) {
        auto& frame = stack.last();

        if (frame.children_pushed) {
            postorder.append(frame.block_index);
            stack.take_last();
            continue;
        }

        frame.children_pushed = true;
        auto block_index = frame.block_index;

        if (block_index == m_virtual_exit_index) {
            // Virtual exit's reverse-CFG successors are the real exit blocks
            for (auto const& block : m_function.basic_blocks()) {
                if (is_exit_block(*block)) {
                    auto target_index = to_index(block->index());
                    if (!visited.get(target_index)) {
                        visited.set(target_index, true);
                        stack.append({ target_index });
                    }
                }
            }
        } else {
            // Real block's reverse-CFG successors are its forward predecessors
            auto* block = m_block_table[block_index];
            if (block) {
                for (auto predecessor : block->predecessor_indices()) {
                    auto target_index = to_index(predecessor);
                    if (!visited.get(target_index)) {
                        visited.set(target_index, true);
                        stack.append({ target_index });
                    }
                }
            }
        }
    }

    // Reverse to get reverse postorder
    m_reverse_postorder.ensure_capacity(postorder.size());
    for (size_t i = postorder.size(); i > 0; --i)
        m_reverse_postorder.append(postorder[i - 1]);
}

void PostDominatorTree::compute_post_dominators()
{
    // Cooper-Harvey-Kennedy iterative algorithm on the reverse CFG.
    // The "entry" is the virtual exit node.

    if (m_reverse_postorder.is_empty())
        return;

    // Map blocks to their reverse postorder index
    auto total_size = m_block_index_capacity + 1;
    Vector<Optional<size_t>> rpo_index;
    rpo_index.resize(total_size);
    for (size_t i = 0; i < m_reverse_postorder.size(); ++i)
        rpo_index[m_reverse_postorder[i]] = i;

    // Initialize: virtual exit post-dominates itself
    m_immediate_post_dominator[m_virtual_exit_index] = m_virtual_exit_index;

    // Intersect helper: find common post-dominator of two blocks
    auto intersect = [&](size_t b1, size_t b2) -> Optional<size_t> {
        auto finger1 = b1;
        auto finger2 = b2;

        while (finger1 != finger2) {
            while (rpo_index[finger1].value_or(SIZE_MAX) > rpo_index[finger2].value_or(SIZE_MAX)) {
                auto const& ipdom = m_immediate_post_dominator[finger1];
                if (!ipdom.has_value())
                    return {};
                finger1 = *ipdom;
            }
            while (rpo_index[finger2].value_or(SIZE_MAX) > rpo_index[finger1].value_or(SIZE_MAX)) {
                auto const& ipdom = m_immediate_post_dominator[finger2];
                if (!ipdom.has_value())
                    return {};
                finger2 = *ipdom;
            }
        }

        return finger1;
    };

    // Helper: iterate over reverse-CFG predecessors of a node.
    // Reverse-CFG predecessors of a real block are its forward-CFG successors.
    // Reverse-CFG predecessors of the virtual exit are the exit blocks (but
    // virtual exit is the "entry" so we skip it in the loop below).
    auto for_each_reverse_cfg_predecessor = [&](size_t block_index, auto callback) {
        if (block_index == m_virtual_exit_index)
            return; // Virtual exit has no predecessors in the reverse CFG
        auto* block = m_block_table[block_index];
        if (!block)
            return;

        // Forward-CFG successors are reverse-CFG predecessors
        CFG::for_each_successor(*block, [&](BasicBlock& successor) {
            callback(to_index(successor.index()));
        });

        // If this is an exit block, the virtual exit is also a predecessor
        if (is_exit_block(*block))
            callback(m_virtual_exit_index);
    };

    // Iterate until fixed point
    bool changed = true;
    while (changed) {
        changed = false;

        // Process all blocks except entry (virtual exit) in reverse postorder
        for (size_t i = 1; i < m_reverse_postorder.size(); ++i) {
            auto block_index = m_reverse_postorder[i];

            // Find first processed predecessor (in reverse CFG)
            Optional<size_t> new_ipdom;
            for_each_reverse_cfg_predecessor(block_index, [&](size_t predecessor) {
                if (m_immediate_post_dominator[predecessor].has_value()) {
                    if (!new_ipdom.has_value()) {
                        new_ipdom = predecessor;
                    } else {
                        new_ipdom = intersect(*new_ipdom, predecessor);
                    }
                }
            });

            if (new_ipdom.has_value()) {
                auto const& existing = m_immediate_post_dominator[block_index];
                if (!existing.has_value() || *existing != *new_ipdom) {
                    m_immediate_post_dominator[block_index] = *new_ipdom;
                    changed = true;
                }
            }
        }
    }
}

void PostDominatorTree::ensure_post_dominance_frontiers() const
{
    if (m_post_dominance_frontiers_computed)
        return;
    m_post_dominance_frontiers_computed = true;

    // Post-dominance frontier algorithm, analogous to dominance frontier
    // but on the reverse CFG.
    // PDF(n) = { y | exists reverse-pred p of y such that n post-dominates p
    //            but n does not strictly post-dominate y }
    //
    // In forward-CFG terms, reverse-preds of y are y's forward successors.
    // So: PDF(n) = { y | exists forward-successor s of y such that n
    //               post-dominates s but n does not strictly post-dominate y }

    m_post_dominance_frontier.resize(m_block_index_capacity);
    for (auto& bitmap : m_post_dominance_frontier)
        bitmap = MUST(AK::Bitmap::create(m_block_index_capacity, false));

    for (auto block_index : m_reverse_postorder) {
        if (block_index == m_virtual_exit_index)
            continue;

        // Count reverse-CFG predecessors (= forward successors + possibly virtual exit)
        auto* block = m_block_table[block_index];
        if (!block)
            continue;

        size_t reverse_pred_count = 0;
        CFG::for_each_successor(*block, [&](BasicBlock&) { ++reverse_pred_count; });
        if (is_exit_block(*block))
            ++reverse_pred_count;

        if (reverse_pred_count < 2)
            continue;

        // For each reverse-CFG predecessor (forward successor + virtual exit)
        auto process_predecessor = [&](size_t predecessor) {
            auto runner = predecessor;
            auto const& block_ipdom = m_immediate_post_dominator[block_index];
            while (runner != m_virtual_exit_index && (!block_ipdom.has_value() || runner != *block_ipdom)) {
                if (runner >= m_block_index_capacity)
                    break;
                m_post_dominance_frontier[runner].set(block_index, true);
                auto const& ipdom = m_immediate_post_dominator[runner];
                if (!ipdom.has_value())
                    break;
                runner = *ipdom;
            }
        };

        CFG::for_each_successor(*block, [&](BasicBlock& successor) {
            process_predecessor(to_index(successor.index()));
        });
        if (is_exit_block(*block))
            process_predecessor(m_virtual_exit_index);
    }
}

BasicBlock* PostDominatorTree::immediate_post_dominator(BasicBlock const* block) const
{
    auto index = to_index(block->index());
    if (index >= m_immediate_post_dominator.size())
        return nullptr;
    auto const& ipdom = m_immediate_post_dominator[index];
    if (!ipdom.has_value())
        return nullptr;
    // If ipdom is the virtual exit or self, return nullptr
    if (*ipdom == m_virtual_exit_index || *ipdom == index)
        return nullptr;
    if (*ipdom >= m_block_table.size())
        return nullptr;
    return m_block_table[*ipdom];
}

bool PostDominatorTree::post_dominates(BasicBlock const* a, BasicBlock const* b) const
{
    if (a == b)
        return true;

    // Walk up the post-dominator tree from b looking for a
    auto a_index = to_index(a->index());
    auto runner_opt = m_immediate_post_dominator[to_index(b->index())];
    while (runner_opt.has_value()) {
        auto runner = *runner_opt;
        if (runner == a_index)
            return true;
        if (runner == m_virtual_exit_index)
            break;
        auto const& next = m_immediate_post_dominator[runner];
        if (!next.has_value() || *next == runner)
            break;
        runner_opt = next;
    }

    return false;
}

bool PostDominatorTree::strictly_post_dominates(BasicBlock const* a, BasicBlock const* b) const
{
    if (a == b)
        return false;
    return post_dominates(a, b);
}

}
