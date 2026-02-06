/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Queue.h>
#include <AK/QuickSort.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/DominatorTree.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/LoopTree.h>

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

LoopTree::LoopTree(Function const& function, DominatorTree const& dom_tree)
    : m_block_index_capacity(block_index_capacity(function))
    , m_block_table(build_block_index_table(function, m_block_index_capacity))
{
    // Step 1: Find back-edges and group latches by header.
    // A back-edge is B -> S where S dominates B.
    Vector<Vector<BlockIndex>> latches_by_header;
    latches_by_header.resize(m_block_index_capacity);

    for (auto const& block : function.basic_blocks()) {
        // NB: Only consider normal control flow edges (terminator targets),
        // not exception handler or finalizer edges.
        auto* term = block->terminator();
        if (!term)
            continue;
        auto check_successor = [&](BasicBlock* successor) {
            if (successor && dom_tree.dominates(successor, block.ptr()))
                latches_by_header[to_index(successor->index())].append(block->index());
        };
        check_successor(term->true_target());
        check_successor(term->false_target());
    }

    // Step 2: For each loop header, collect the natural loop body via
    // backwards BFS from each latch.
    for (size_t header_idx = 0; header_idx < latches_by_header.size(); ++header_idx) {
        auto& latches = latches_by_header[header_idx];
        if (latches.is_empty())
            continue;

        auto loop = make<Loop>();
        loop->m_header = m_block_table[header_idx];
        loop->m_latches = move(latches);
        loop->m_blocks = MUST(AK::Bitmap::create(m_block_index_capacity, false));
        loop->m_blocks.set(header_idx, true);
        loop->m_block_count = 1;
        loop->m_block_table = &m_block_table;

        for (auto latch_idx : loop->m_latches) {
            auto li = to_index(latch_idx);
            if (loop->m_blocks.get(li))
                continue;
            loop->m_blocks.set(li, true);
            ++loop->m_block_count;

            Queue<BlockIndex> worklist;
            worklist.enqueue(latch_idx);

            while (!worklist.is_empty()) {
                auto block_idx = worklist.dequeue();
                auto* block = m_block_table[to_index(block_idx)];
                for (auto* pred : block->predecessors()) {
                    auto pi = to_index(pred->index());
                    if (!loop->m_blocks.get(pi)) {
                        loop->m_blocks.set(pi, true);
                        ++loop->m_block_count;
                        worklist.enqueue(pred->index());
                    }
                }
            }
        }

        m_loops.append(move(loop));
    }

    // Step 3: Build block-to-loop map. Process larger loops first so that
    // last-writer-wins gives us the innermost loop for each block.
    quick_sort(m_loops, [](auto const& a, auto const& b) {
        return a->m_block_count > b->m_block_count;
    });

    m_block_to_loop.resize_with_default_value(m_block_index_capacity, nullptr);

    for (auto& loop : m_loops) {
        for (size_t i = 0; i < loop->m_blocks.size(); ++i) {
            if (loop->m_blocks.get(i))
                m_block_to_loop[i] = loop.ptr();
        }
    }
}

Loop const* LoopTree::loop_for_block(BasicBlock const* block) const
{
    auto idx = to_index(block->index());
    if (idx >= m_block_to_loop.size())
        return nullptr;
    return m_block_to_loop[idx];
}

}
