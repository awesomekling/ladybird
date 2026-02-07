/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/Dump.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Optimize.h>
#include <LibJS/IR/Passes/Pass.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Passes/Verifier.h>

namespace JS::IR {

PassManager::PassManager() = default;
PassManager::~PassManager() = default;

void PassManager::add_pass(NonnullOwnPtr<Pass> pass)
{
    m_passes.append(move(pass));
}

void PassManager::run(Function& function)
{
    if (g_dump_ir_between_passes)
        dbgln("=== Before optimization ===\n{}", dump(function));

    // Validate the IR from the builder before any optimization passes.
    if (!Verifier::verify(function, VerifierMode::Full, false)) {
        warnln("IR Verifier failed before optimization");
        Verifier::verify(function, VerifierMode::Full, true);
    }

    constexpr size_t max_iterations = 10;
    for (size_t iteration = 0; iteration < max_iterations; ++iteration) {
        bool changed = false;

        for (auto& pass : m_passes) {
            auto preserved = pass->run(function, *this);

            if (!preserved.is_all()) {
                changed = true;
                invalidate(preserved);

                if (g_dump_ir_between_passes)
                    dbgln("=== After {} ===\n{}", pass->name(), dump(function));

                if (!Verifier::verify(function, VerifierMode::InterPass, false)) {
                    warnln("IR Verifier failed after pass: {}", pass->name());
                    Verifier::verify(function, VerifierMode::InterPass, true);
                }
            }
        }

        if (!changed)
            break;
    }

    if (!Verifier::verify(function, VerifierMode::Full, false)) {
        warnln("IR Verifier (full) failed after optimization");
        Verifier::verify(function, VerifierMode::Full, true);
    }
}

DominatorTree const& PassManager::dominator_tree(Function const& function)
{
    if (!m_dominator_tree)
        m_dominator_tree = make<DominatorTree>(function);
    return *m_dominator_tree;
}

LoopTree const& PassManager::loop_tree(Function const& function)
{
    if (!m_loop_tree)
        m_loop_tree = make<LoopTree>(function, dominator_tree(function));
    return *m_loop_tree;
}

PostDominatorTree const& PassManager::post_dominator_tree(Function const& function)
{
    if (!m_post_dominator_tree)
        m_post_dominator_tree = make<PostDominatorTree>(function);
    return *m_post_dominator_tree;
}

void PassManager::invalidate(PreservedAnalyses const& preserved)
{
    if (!preserved.are_dominators_preserved()) {
        m_dominator_tree = nullptr;
        m_loop_tree = nullptr;
        m_post_dominator_tree = nullptr;
    }
}

}
