/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/SsaDestructionPass.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

PreservedAnalyses SsaDestructionPass::run(Function& function, PassManager&)
{
    // Phase 1: Collect copies for each CFG edge from phi nodes.
    //
    // For normal edges (Jump/ContinuePendingUnwind), copies go before the
    // terminator in the predecessor block.
    //
    // For Yield/Await edges, phi moves must happen AFTER the generator
    // resumes (the resume value is the Yield/Await result which may be a
    // phi operand). We create an intermediate block between the Yield/Await
    // and its continuation.
    //
    // For exception handler edges, copies go at the START of the block
    // that has the handler, since any instruction can throw and the
    // handler phi values must already be set.

    struct Copy {
        Value* dst;
        Value* src;
    };

    // Key: (predecessor, successor) edge
    // Value: list of copies for that edge
    HashMap<u64, Vector<Copy>> normal_edge_copies;
    HashMap<u64, Vector<Copy>> handler_edge_copies;

    auto edge_key = [](BasicBlock const& from, BasicBlock const& to) -> u64 {
        return (static_cast<u64>(static_cast<u32>(from.index())) << 32) | static_cast<u64>(static_cast<u32>(to.index()));
    };

    bool has_any_phis = false;

    for (auto& block : function.basic_blocks()) {
        block->for_each_phi([&](PhiInstruction const& phi) {
            has_any_phis = true;
            auto* phi_result = phi.result();
            if (!phi_result)
                return;

            for (size_t i = 0; i < phi.incoming_count(); ++i) {
                auto* pred = phi.incoming_block(i);
                auto* value = phi.incoming_value(i);
                if (!pred || !value)
                    continue;

                // Skip self-copies
                if (value == phi_result)
                    continue;

                // Determine edge type based on predecessor's exception handler
                auto* term = pred->terminator();
                bool is_handler_edge = pred->exception_handler() == block.ptr();

                // NB: A Yield/Await's continuation edge is a normal edge
                // in the CFG, but the phi operand might be the resume
                // value which only becomes available after resume. These
                // are handled specially below.
                bool is_yield_await_edge = term && (term->opcode() == Opcode::Yield || term->opcode() == Opcode::Await) && term->true_target() == block.ptr();

                if (is_handler_edge && !is_yield_await_edge) {
                    auto key = edge_key(*pred, *block);
                    handler_edge_copies.ensure(key).append({ phi_result, value });
                } else {
                    auto key = edge_key(*pred, *block);
                    normal_edge_copies.ensure(key).append({ phi_result, value });
                }
            }
        });
    }

    if (!has_any_phis)
        return PreservedAnalyses::all();

    // Phase 2: Insert ParallelCopy instructions.

    // Normal edges: insert before the terminator in the predecessor block.
    // For Yield/Await edges: create an intermediate block.
    for (auto& [key, copies] : normal_edge_copies) {
        auto from_index = BlockIndex(static_cast<u32>(key >> 32));
        auto to_index = BlockIndex(static_cast<u32>(key & 0xFFFFFFFF));

        auto* from = function.block_by_index(from_index);
        auto* to = function.block_by_index(to_index);
        VERIFY(from && to);

        auto* term = from->terminator();
        VERIFY(term);

        bool is_yield_await_edge = (term->opcode() == Opcode::Yield || term->opcode() == Opcode::Await) && term->true_target() == to;

        if (is_yield_await_edge) {
            // Create intermediate block: Yield/Await -> intermediate -> continuation
            auto& intermediate = function.create_block(
                MUST(String::formatted("pcopy_{}_{}", from->name(), to->name())));

            // Insert ParallelCopy + Jump in intermediate block
            auto pcopy = ParallelCopyInstruction::create();
            for (auto& copy : copies)
                pcopy->add_copy(copy.dst, copy.src);
            intermediate.append(move(pcopy));
            intermediate.append(JumpInstruction::create(*to));

            // Redirect Yield/Await continuation to intermediate block
            CFG::redirect_edge(*from, *to, intermediate);
        } else {
            // Insert ParallelCopy before terminator
            auto pcopy = ParallelCopyInstruction::create();
            for (auto& copy : copies)
                pcopy->add_copy(copy.dst, copy.src);
            from->insert_before_terminator(move(pcopy));
        }
    }

    // Exception handler edges: insert at the START of the block that has the handler.
    for (auto& [key, copies] : handler_edge_copies) {
        auto from_index = BlockIndex(static_cast<u32>(key >> 32));

        auto* from = function.block_by_index(from_index);
        VERIFY(from);

        auto pcopy = ParallelCopyInstruction::create();
        for (auto& copy : copies)
            pcopy->add_copy(copy.dst, copy.src);
        from->prepend(move(pcopy));
    }

    // Phase 3: Remove all phi nodes from every block.
    for (auto& block : function.basic_blocks()) {
        block->remove_instructions_if([](Instruction const& inst) {
            return inst.opcode() == Opcode::Phi;
        });
    }

    return PreservedAnalyses::none();
}

}
