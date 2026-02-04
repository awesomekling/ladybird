/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/Vector.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

// Computes dominator tree and dominance frontiers for a control flow graph.
// Uses a simple iterative algorithm for dominator computation.
class Dominators {
public:
    explicit Dominators(Function const& function);

    // Returns the immediate dominator of a block, or nullptr for the entry block
    BasicBlock* immediate_dominator(BasicBlock const* block) const;

    // Returns true if a strictly dominates b (a != b and a dominates b)
    bool strictly_dominates(BasicBlock const* a, BasicBlock const* b) const;

    // Returns true if a dominates b (a == b or a strictly dominates b)
    bool dominates(BasicBlock const* a, BasicBlock const* b) const;

    // Returns the dominance frontier of a block
    HashTable<BasicBlock*> const& dominance_frontier(BasicBlock const* block) const;

    // Returns all blocks in the function in reverse postorder (useful for dataflow)
    Vector<BasicBlock*> const& reverse_postorder() const { return m_reverse_postorder; }

    // Returns the children of a block in the dominator tree
    Vector<BasicBlock*> const& dominator_children(BasicBlock const* block) const;

private:
    void compute_reverse_postorder();
    void compute_dominators();
    void compute_dominance_frontiers();

    void compute_dominator_children();

    Function const& m_function;
    HashMap<BasicBlock const*, BasicBlock*> m_immediate_dominator;
    HashMap<BasicBlock const*, HashTable<BasicBlock*>> m_dominance_frontier;
    HashMap<BasicBlock const*, Vector<BasicBlock*>> m_dominator_children;
    Vector<BasicBlock*> m_reverse_postorder;
};

}
