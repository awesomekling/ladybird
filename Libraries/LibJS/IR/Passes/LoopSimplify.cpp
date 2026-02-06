/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/LoopTree.h>
#include <LibJS/IR/Passes/LoopSimplify.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

// Insert a new block between a set of predecessors and a target block.
// The new block gets an unconditional jump to the target. If multiple
// predecessors exist, phi nodes are created in the new block to merge
// their values, and the target's phis reference the new block's phi results.
static void insert_dedicated_block(
    Function& function,
    BasicBlock& target,
    Vector<BasicBlock*> const& predecessors_to_redirect,
    StringView block_name_prefix)
{
    auto& new_block = function.create_block(
        MUST(String::formatted("{}_{}", block_name_prefix, target.name())));

    // Step 1: Snapshot phi incoming values for all predecessors being redirected.
    // We must do this before any mutation since redirect_edge removes phi entries.
    struct PhiSnapshot {
        Vector<Value*> values; // One per predecessor in predecessors_to_redirect
    };
    Vector<PhiSnapshot> snapshots;

    for (auto const& instruction : target.instructions()) {
        if (instruction->opcode() != Opcode::Phi)
            break;
        auto& phi = static_cast<PhiInstruction&>(*instruction);
        PhiSnapshot snapshot;
        for (auto* pred : predecessors_to_redirect)
            snapshot.values.append(phi.incoming_value_for(*pred));
        snapshots.append(move(snapshot));
    }

    // Step 2: Redirect each predecessor from target to new_block.
    // NB: new_block has no phis yet, so redirect_edge won't create any
    // spurious phi entries in it. It does remove each pred from target's
    // predecessor list and phi entries.
    for (auto* pred : predecessors_to_redirect)
        CFG::redirect_edge(*pred, target, new_block);

    // Step 3: Create phi nodes in new_block (if multiple predecessors) and
    // compute the values to forward to target's phis.
    Vector<Value*> forwarded_values;

    if (predecessors_to_redirect.size() == 1) {
        for (auto& snapshot : snapshots)
            forwarded_values.append(snapshot.values[0]);
    } else {
        for (auto& snapshot : snapshots) {
            auto new_phi = PhiInstruction::create();
            auto& result = function.create_value_for_instruction();
            new_phi->set_result(&result);
            for (size_t i = 0; i < predecessors_to_redirect.size(); ++i)
                new_phi->add_incoming(predecessors_to_redirect[i], snapshot.values[i]);
            new_block.append(move(new_phi));
            forwarded_values.append(&result);
        }
    }

    // Step 4: Add new_block as predecessor of target, forwarding the right values.
    size_t phi_index = 0;
    CFG::add_predecessor(target, new_block, [&](Instruction&) -> Value* {
        return forwarded_values[phi_index++];
    });

    // Step 5: Terminate the new block with a jump to target.
    new_block.append(JumpInstruction::create(target));
}

PreservedAnalyses LoopSimplify::run(Function& function, PassManager& pass_manager)
{
    bool changed = false;

    auto const& lt = pass_manager.loop_tree(function);

    for (auto const& loop : lt.loops()) {
        auto* header = loop->header();
        auto const& dom_tree = pass_manager.dominator_tree(function);

        // Classify predecessors as entry edges vs back edges.
        // NB: Only consider predecessors that have a terminator edge to
        // the header. Some predecessors may be connected only through
        // exception handler edges, which are not normal control flow.
        Vector<BasicBlock*> entry_preds;
        Vector<BasicBlock*> back_preds;

        for (auto* pred : header->predecessors()) {
            auto* term = pred->terminator();
            if (!term)
                continue;
            if (term->true_target() != header && term->false_target() != header)
                continue;
            if (dom_tree.dominates(header, pred))
                back_preds.append(pred);
            else
                entry_preds.append(pred);
        }

        // Insert preheader if the header has multiple entry-edge predecessors.
        if (entry_preds.size() > 1) {
            insert_dedicated_block(function, *header, entry_preds, "preheader"sv);
            changed = true;
        }

        // Merge multiple latches into a single latch block.
        if (back_preds.size() > 1) {
            insert_dedicated_block(function, *header, back_preds, "latch"sv);
            changed = true;
        }
    }

    return changed ? PreservedAnalyses::none() : PreservedAnalyses::all();
}

}
