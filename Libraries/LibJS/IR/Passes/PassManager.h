/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/OwnPtr.h>
#include <AK/Vector.h>
#include <LibJS/Export.h>
#include <LibJS/IR/DominatorTree.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/IR/LoopTree.h>

namespace JS::IR {

class Pass;

// Describes which cached analyses a pass preserves after running.
// Passes that don't modify the CFG can preserve dominator-based analyses.
// Passes that mutate the CFG should return none().
class PreservedAnalyses {
public:
    // Nothing changed — all analyses remain valid.
    static PreservedAnalyses all() { return PreservedAnalyses(true, true); }

    // CFG was mutated — all analyses are invalidated.
    static PreservedAnalyses none() { return PreservedAnalyses(false, false); }

    // Operands/instructions changed but CFG structure is intact.
    static PreservedAnalyses all_cfg_analyses()
    {
        return PreservedAnalyses(false, true);
    }

    bool is_all() const { return m_nothing_changed; }
    bool are_dominators_preserved() const { return m_dominators; }

private:
    PreservedAnalyses(bool nothing_changed, bool dominators)
        : m_nothing_changed(nothing_changed)
        , m_dominators(dominators)
    {
    }

    bool m_nothing_changed { false };
    bool m_dominators { false };
};

// Manages optimization pass execution and caches analyses across passes.
// Passes that need analyses (e.g., DominatorTree) request them through the
// PassManager, which lazily computes and caches them. When a pass mutates
// the IR, it returns PreservedAnalyses to indicate which cached results
// remain valid.
class JS_API PassManager {
public:
    PassManager();
    ~PassManager();

    void add_pass(NonnullOwnPtr<Pass>);

    // Run all passes in a fixed-point loop until convergence.
    void run(Function&);

    // Lazily compute and cache the DominatorTree analysis.
    DominatorTree const& dominator_tree(Function const&);

    // Lazily compute and cache the LoopTree analysis.
    LoopTree const& loop_tree(Function const&);

    // Invalidate cached analyses based on what a pass preserved.
    void invalidate(PreservedAnalyses const&);

private:
    Vector<NonnullOwnPtr<Pass>> m_passes;
    OwnPtr<DominatorTree> m_dominator_tree;
    OwnPtr<LoopTree> m_loop_tree;
};

}
