/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/Dump.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Optimize.h>
#include <LibJS/IR/Passes/ADCE.h>
#include <LibJS/IR/Passes/CopyCoalescing.h>
#include <LibJS/IR/Passes/CopyPropagation.h>
#include <LibJS/IR/Passes/DeadCodeElimination.h>
#include <LibJS/IR/Passes/GlobalValueNumbering.h>
#include <LibJS/IR/Passes/InstCombine.h>
#include <LibJS/IR/Passes/LoopInvariantCodeMotion.h>
#include <LibJS/IR/Passes/LoopSimplify.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Passes/PostSSACleanup.h>
#include <LibJS/IR/Passes/RedundantCheckElimination.h>
#include <LibJS/IR/Passes/SCCP.h>
#include <LibJS/IR/Passes/SSAConstructionPass.h>
#include <LibJS/IR/Passes/SimplifyCFG.h>
#include <LibJS/IR/Passes/SplitCriticalEdges.h>
#include <LibJS/IR/Passes/SsaDestructionPass.h>
#include <LibJS/IR/Passes/TypeRefinement.h>

namespace JS::IR {

bool g_dump_ir = false;
bool g_optimize_ir = false;
bool g_dump_ir_between_passes = false;
bool g_lower_ir = false;

// IR Pipeline Phases 2-3: SSA construction + optimization
//
// The full IR pipeline is:
//   Phase 1: Bytecode → IR (CFG construction)           — Lifter::lift()
//   Phase 2: SSA construction                            — SSAConstructionPass (always runs)
//   Phase 3: Optimization passes on SSA-form IR          — (gated by g_optimize_ir)
//   Phase 4: SSA destruction (phi coalescing) + lowering — Lowerer::lower() via PhiCoalescing
void optimize(Function& function)
{
    // Phase 2: SSA construction (always runs, required before lowering)
    PassManager pass_manager;

    SSAConstructionPass ssa_pass;
    auto preserved = ssa_pass.run(function, pass_manager);
    pass_manager.invalidate(preserved);

    if (g_dump_ir_between_passes)
        dbgln("=== After {} ===\n{}", ssa_pass.name(), dump(function));

    // SCCP: global constant propagation + unreachable code elimination
    SCCP sccp_pass;
    preserved = sccp_pass.run(function, pass_manager);
    pass_manager.invalidate(preserved);

    if (g_dump_ir_between_passes)
        dbgln("=== After {} ===\n{}", sccp_pass.name(), dump(function));

    // Phase 3: Optimization passes
    // Dead Code Removal
    pass_manager.add_pass(make<DeadCodeElimination>());
    pass_manager.add_pass(make<AggressiveDeadCodeElimination>());

    // Local Optimizations
    pass_manager.add_pass(make<CopyPropagation>());
    pass_manager.add_pass(make<TypeRefinement>());
    pass_manager.add_pass(make<RedundantCheckElimination>());
    pass_manager.add_pass(make<InstCombine>());

    // Global Optimizations
    pass_manager.add_pass(make<GlobalValueNumbering>());

    // CFG Cleanup
    pass_manager.add_pass(make<SimplifyCFG>());

    pass_manager.run(function);

    // Loop Optimizations (run once, after the fixed-point loop).
    // LoopSimplify inserts preheader and single-latch blocks that
    // SimplifyCFG would fold away, so these must not participate
    // in the fixed-point loop.
    auto run_once = [&](Pass& pass) {
        auto preserved = pass.run(function, pass_manager);
        if (!preserved.is_all()) {
            pass_manager.invalidate(preserved);
            if (g_dump_ir_between_passes)
                dbgln("=== After {} ===\n{}", pass.name(), dump(function));
        }
    };

    LoopSimplify loop_simplify_pass;
    run_once(loop_simplify_pass);

    LoopInvariantCodeMotion licm_pass;
    run_once(licm_pass);

    // Lowering Preparation (also runs once, after the fixed-point loop).
    // Split critical edges so phi moves have a clean block to land in
    // during SSA deconstruction. This must not participate in the
    // fixed-point loop because subsequent CFG cleanup would fold the
    // split blocks back, recreating the critical edges.
    SplitCriticalEdges split_pass;
    run_once(split_pass);

    // SSA Destruction: replace phi nodes with ParallelCopy instructions.
    // Must run after SplitCriticalEdges so copies have clean blocks.
    SsaDestructionPass ssa_destruction_pass;
    run_once(ssa_destruction_pass);

    // Copy Coalescing: merge non-interfering ParallelCopy src/dst pairs
    // so copies become no-ops, enabling PostSSACleanup to fold more blocks.
    CopyCoalescing copy_coalescing_pass;
    run_once(copy_coalescing_pass);

    // Post-SSA Cleanup: eliminate trivial [ParallelCopy...] + Jump blocks
    // left over from SSA destruction by moving copies into predecessors.
    PostSSACleanup post_ssa_cleanup_pass;
    run_once(post_ssa_cleanup_pass);

    function.set_stage(IRStage::PostSSA);
}

}
