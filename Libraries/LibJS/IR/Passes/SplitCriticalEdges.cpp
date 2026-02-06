/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/SplitCriticalEdges.h>

namespace JS::IR {

PreservedAnalyses SplitCriticalEdges::run(Function& function, PassManager&)
{
    // Phase 1: Collect critical edges.
    // A critical edge is A->B where A has multiple successors and B has
    // multiple predecessors.
    struct CriticalEdge {
        BasicBlock* from;
        BasicBlock* to;
    };
    Vector<CriticalEdge> edges;

    for (auto& block : function.basic_blocks()) {
        auto* term = block->terminator();
        if (!term || term->opcode() != Opcode::Branch)
            continue;

        auto* true_target = term->true_target();
        auto* false_target = term->false_target();

        // Same target means effectively one successor -- not critical.
        if (true_target == false_target)
            continue;

        if (true_target && true_target->predecessor_indices().size() > 1)
            edges.append({ block.ptr(), true_target });
        if (false_target && false_target->predecessor_indices().size() > 1)
            edges.append({ block.ptr(), false_target });
    }

    if (edges.is_empty())
        return PreservedAnalyses::all();

    // Phase 2: Split each critical edge by inserting a new block.
    for (auto& edge : edges) {
        auto& split = function.create_block(
            MUST(String::formatted("split_{}_{}", edge.from->name(), edge.to->name())));

        // Add split as predecessor of target with same phi values as from.
        // NB: This must happen before redirect_edge, which removes from as
        // a predecessor of to (and its phi operands).
        CFG::add_predecessor(*edge.to, split, [&](Instruction& phi) -> Value* {
            return static_cast<PhiInstruction&>(phi).incoming_value_for(*edge.from);
        });

        // Redirect from->to to from->split (removes from as predecessor of to).
        CFG::redirect_edge(*edge.from, *edge.to, split);

        // Add jump from split to target.
        split.append(JumpInstruction::create(*edge.to));
    }

    return PreservedAnalyses::none();
}

}
