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

DominatorTree::DominatorTree(Function const& function)
    : m_function(function)
{
    compute_reverse_postorder();
    compute_dominators();
}

void DominatorTree::compute_reverse_postorder()
{
    // DFS to compute reverse postorder using explicit stack with frames.

    if (!m_function.entry_block())
        return;

    struct Frame {
        BasicBlock* block;
        bool children_pushed { false };
    };

    HashTable<BasicBlock*> visited;
    Vector<BasicBlock*> postorder;
    Vector<Frame> stack;

    visited.set(m_function.entry_block());
    stack.append({ m_function.entry_block() });

    while (!stack.is_empty()) {
        auto& frame = stack.last();

        if (frame.children_pushed) {
            postorder.append(frame.block);
            stack.take_last();
            continue;
        }

        frame.children_pushed = true;

        // NB: Save the block pointer before pushing children, since appending
        // to the stack may reallocate and invalidate the frame reference.
        auto* block = frame.block;

        // Push successors (will be processed before we return to this block)
        // Include both control flow successors and exception edges
        CFG::for_each_successor(*block, [&](BasicBlock& target) {
            if (!visited.contains(&target)) {
                visited.set(&target);
                stack.append({ &target });
            }
        });
    }

    // Reverse to get reverse postorder
    m_reverse_postorder.ensure_capacity(postorder.size());
    for (size_t i = postorder.size(); i > 0; --i)
        m_reverse_postorder.append(postorder[i - 1]);
}

void DominatorTree::compute_dominators()
{
    // Simple iterative algorithm for computing dominators
    // Based on "A Simple, Fast Dominance Algorithm" by Cooper, Harvey, Kennedy

    if (m_reverse_postorder.is_empty())
        return;

    auto* entry = m_function.entry_block();

    // Map blocks to their reverse postorder index
    HashMap<BasicBlock*, size_t> rpo_index;
    for (size_t i = 0; i < m_reverse_postorder.size(); ++i)
        rpo_index.set(m_reverse_postorder[i], i);

    // Initialize: entry dominates itself, others undefined
    m_immediate_dominator.set(entry, entry);

    // Intersect helper: find common dominator of two blocks
    auto intersect = [&](BasicBlock* b1, BasicBlock* b2) -> BasicBlock* {
        auto* finger1 = b1;
        auto* finger2 = b2;

        while (finger1 != finger2) {
            while (rpo_index.get(finger1).value_or(SIZE_MAX) > rpo_index.get(finger2).value_or(SIZE_MAX)) {
                auto idom = m_immediate_dominator.get(finger1);
                if (!idom.has_value())
                    return nullptr;
                finger1 = *idom;
            }
            while (rpo_index.get(finger2).value_or(SIZE_MAX) > rpo_index.get(finger1).value_or(SIZE_MAX)) {
                auto idom = m_immediate_dominator.get(finger2);
                if (!idom.has_value())
                    return nullptr;
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
            auto* block = m_reverse_postorder[i];

            // Find first processed predecessor
            BasicBlock* new_idom = nullptr;
            for (auto* pred : block->predecessors()) {
                if (m_immediate_dominator.contains(pred)) {
                    if (!new_idom) {
                        new_idom = pred;
                    } else {
                        new_idom = intersect(new_idom, pred);
                        if (!new_idom)
                            break;
                    }
                }
            }

            if (new_idom) {
                auto existing = m_immediate_dominator.get(block);
                if (!existing.has_value() || *existing != new_idom) {
                    m_immediate_dominator.set(block, new_idom);
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

    for (auto* block : m_reverse_postorder) {
        m_dominance_frontier.set(block, {});
    }

    for (auto* block : m_reverse_postorder) {
        auto const& preds = block->predecessors();
        if (preds.size() < 2)
            continue; // Only join points have non-empty dominance frontiers contributed here

        for (auto* pred : preds) {
            auto* runner = pred;
            while (runner && runner != m_immediate_dominator.get(block).value_or(nullptr)) {
                auto it = m_dominance_frontier.find(runner);
                if (it == m_dominance_frontier.end())
                    break; // Predecessor not reachable from entry
                it->value.set(block);
                runner = m_immediate_dominator.get(runner).value_or(nullptr);
            }
        }
    }
}

BasicBlock* DominatorTree::immediate_dominator(BasicBlock const* block) const
{
    auto it = m_immediate_dominator.find(block);
    if (it == m_immediate_dominator.end())
        return nullptr;
    // Entry block's idom is itself, but we return nullptr for external interface
    if (it->value == block)
        return nullptr;
    return it->value;
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
    auto* runner = m_immediate_dominator.get(b).value_or(nullptr);
    while (runner && runner != b) { // runner != b guards against entry (idom = itself)
        if (runner == a)
            return true;
        auto* next = m_immediate_dominator.get(runner).value_or(nullptr);
        if (next == runner)
            break; // Entry block
        runner = next;
    }

    return false;
}

HashTable<BasicBlock*> const& DominatorTree::dominance_frontier(BasicBlock const* block) const
{
    ensure_dominance_frontiers();
    static HashTable<BasicBlock*> empty;
    auto it = m_dominance_frontier.find(block);
    if (it == m_dominance_frontier.end())
        return empty;
    return it->value;
}

void DominatorTree::ensure_dominator_children() const
{
    if (m_dominator_children_computed)
        return;
    m_dominator_children_computed = true;

    // Build the dominator tree children by grouping blocks by their immediate dominator
    for (auto* block : m_reverse_postorder) {
        m_dominator_children.set(block, {});
    }

    for (auto* block : m_reverse_postorder) {
        auto* idom = immediate_dominator(block);
        if (idom)
            m_dominator_children.find(idom)->value.append(block);
    }
}

Vector<BasicBlock*> const& DominatorTree::dominator_children(BasicBlock const* block) const
{
    ensure_dominator_children();
    static Vector<BasicBlock*> empty;
    auto it = m_dominator_children.find(block);
    if (it == m_dominator_children.end())
        return empty;
    return it->value;
}

}
