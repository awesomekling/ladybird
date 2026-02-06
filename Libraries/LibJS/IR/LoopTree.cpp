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

LoopTree::LoopTree(Function const& function, DominatorTree const& dom_tree)
{
    // Step 1: Find back-edges and group latches by header.
    // A back-edge is B -> S where S dominates B.
    HashMap<BasicBlock*, Vector<BasicBlock*>> latches_by_header;

    for (auto const& block : function.basic_blocks()) {
        // NB: Only consider normal control flow edges (terminator targets),
        // not exception handler or finalizer edges.
        auto* term = block->terminator();
        if (!term)
            continue;
        auto check_successor = [&](BasicBlock* successor) {
            if (successor && dom_tree.dominates(successor, block.ptr()))
                latches_by_header.ensure(successor).append(block.ptr());
        };
        check_successor(term->true_target());
        check_successor(term->false_target());
    }

    // Step 2: For each loop header, collect the natural loop body via
    // backwards BFS from each latch.
    for (auto& [header, latches] : latches_by_header) {
        auto loop = make<Loop>();
        loop->m_header = header;
        loop->m_latches = move(latches);
        loop->m_blocks.set(header);

        for (auto* latch : loop->m_latches) {
            if (loop->m_blocks.contains(latch))
                continue;
            loop->m_blocks.set(latch);

            Queue<BasicBlock*> worklist;
            worklist.enqueue(latch);

            while (!worklist.is_empty()) {
                auto* block = worklist.dequeue();
                for (auto* pred : block->predecessors()) {
                    if (!loop->m_blocks.contains(pred)) {
                        loop->m_blocks.set(pred);
                        worklist.enqueue(pred);
                    }
                }
            }
        }

        m_loops.append(move(loop));
    }

    // Step 3: Build block-to-loop map. Process larger loops first so that
    // last-writer-wins gives us the innermost loop for each block.
    quick_sort(m_loops, [](auto const& a, auto const& b) {
        return a->m_blocks.size() > b->m_blocks.size();
    });

    for (auto& loop : m_loops) {
        for (auto* block : loop->m_blocks)
            m_block_to_loop.set(block, loop.ptr());
    }
}

Loop const* LoopTree::loop_for_block(BasicBlock const* block) const
{
    auto it = m_block_to_loop.find(block);
    if (it == m_block_to_loop.end())
        return nullptr;
    return it->value;
}

}
