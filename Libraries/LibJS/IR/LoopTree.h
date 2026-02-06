/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/NonnullOwnPtr.h>
#include <AK/Vector.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class DominatorTree;

class Loop {
public:
    BasicBlock* header() const { return m_header; }
    Vector<BasicBlock*> const& latches() const { return m_latches; }
    HashTable<BasicBlock*> const& blocks() const { return m_blocks; }
    bool contains(BasicBlock const* block) const { return m_blocks.contains(const_cast<BasicBlock*>(block)); }

private:
    friend class LoopTree;

    BasicBlock* m_header { nullptr };
    Vector<BasicBlock*> m_latches;
    HashTable<BasicBlock*> m_blocks;
};

class LoopTree {
public:
    explicit LoopTree(Function const&, DominatorTree const&);

    Vector<NonnullOwnPtr<Loop>> const& loops() const { return m_loops; }
    Loop const* loop_for_block(BasicBlock const* block) const;

private:
    Vector<NonnullOwnPtr<Loop>> m_loops;
    HashMap<BasicBlock const*, Loop*> m_block_to_loop;
};

}
