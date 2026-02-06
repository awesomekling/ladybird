/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/Dump.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Optimize.h>
#include <LibJS/IR/Passes/AlgebraicSimplification.h>
#include <LibJS/IR/Passes/BlockMerging.h>
#include <LibJS/IR/Passes/ConstantBranchFolding.h>
#include <LibJS/IR/Passes/ConstantFolding.h>
#include <LibJS/IR/Passes/CopyPropagation.h>
#include <LibJS/IR/Passes/DeadBlockElimination.h>
#include <LibJS/IR/Passes/DeadCodeElimination.h>
#include <LibJS/IR/Passes/EmptyBlockElimination.h>
#include <LibJS/IR/Passes/GlobalValueNumbering.h>
#include <LibJS/IR/Passes/InstructionCombining.h>
#include <LibJS/IR/Passes/JumpThreading.h>
#include <LibJS/IR/Passes/LoopInvariantCodeMotion.h>
#include <LibJS/IR/Passes/LoopSimplify.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Passes/SimplifyCFG.h>
#include <LibJS/IR/Passes/SplitCriticalEdges.h>

namespace JS::IR {

bool g_dump_ir = false;
bool g_optimize_ir = false;
bool g_dump_ir_between_passes = false;
bool g_lower_ir = false;

// IR Pipeline Phase 3: Optimization (operates on SSA-form IR)
//
// The full IR pipeline is:
//   Phase 1: Bytecode → IR (CFG construction)           — Lifter::lift()
//   Phase 2: SSA construction                            — Lifter::lift() via SSAConstruction
//   Phase 3: Optimization passes on SSA-form IR          — optimize() (this function)
//   Phase 4: SSA destruction (phi coalescing) + lowering — Lowerer::lower() via PhiCoalescing
void optimize(Function& function)
{
    PassManager pass_manager;

    // CFG Simplification
    pass_manager.add_pass(make<ConstantBranchFolding>());
    pass_manager.add_pass(make<JumpThreading>());

    // Dead Code Removal
    pass_manager.add_pass(make<DeadCodeElimination>());
    pass_manager.add_pass(make<DeadBlockElimination>());

    // Local Optimizations
    pass_manager.add_pass(make<CopyPropagation>());
    pass_manager.add_pass(make<ConstantFolding>());
    pass_manager.add_pass(make<AlgebraicSimplification>());
    pass_manager.add_pass(make<InstructionCombining>());

    // Global Optimizations
    pass_manager.add_pass(make<GlobalValueNumbering>());

    // CFG Cleanup
    pass_manager.add_pass(make<SimplifyCFG>());

    pass_manager.run(function);

    // Loop Optimizations (run once, after the fixed-point loop).
    // LoopSimplify inserts preheader and single-latch blocks that
    // EmptyBlockElimination / BlockMerging would fold away, so these
    // must not participate in the fixed-point loop.
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
}

}
