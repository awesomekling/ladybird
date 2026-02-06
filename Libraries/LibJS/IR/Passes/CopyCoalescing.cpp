/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Bitmap.h>
#include <AK/HashMap.h>
#include <AK/Vector.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/CopyCoalescing.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

PreservedAnalyses CopyCoalescing::run(Function& function, PassManager&)
{
    auto value_count = function.values().size();
    if (value_count == 0)
        return PreservedAnalyses::all();

    // Map sparse BlockIndex to dense array positions.
    HashMap<BlockIndex, size_t> block_to_dense;
    auto const& blocks = function.basic_blocks();
    for (size_t i = 0; i < blocks.size(); ++i)
        block_to_dense.set(blocks[i]->index(), i);

    auto block_count = blocks.size();

    // Phase 1: Block-level liveness analysis.
    //
    // Compute gen/kill sets, then iterate to fixed point for live_in/live_out.

    // Allocate bitmaps for gen, kill, live_in, live_out per block.
    Vector<Bitmap> gen;
    Vector<Bitmap> kill;
    Vector<Bitmap> live_in;
    Vector<Bitmap> live_out;
    gen.ensure_capacity(block_count);
    kill.ensure_capacity(block_count);
    live_in.ensure_capacity(block_count);
    live_out.ensure_capacity(block_count);
    for (size_t i = 0; i < block_count; ++i) {
        gen.unchecked_append(MUST(Bitmap::create(value_count, false)));
        kill.unchecked_append(MUST(Bitmap::create(value_count, false)));
        live_in.unchecked_append(MUST(Bitmap::create(value_count, false)));
        live_out.unchecked_append(MUST(Bitmap::create(value_count, false)));
    }

    // Build gen/kill sets for each block.
    for (size_t block_index = 0; block_index < block_count; ++block_index) {
        auto& block = *blocks[block_index];
        auto& block_gen = gen[block_index];
        auto& block_kill = kill[block_index];

        block.for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() == Opcode::ParallelCopy) {
                auto const& pcopy = static_cast<ParallelCopyInstruction const&>(instruction);
                // Sources are uses (parallel read-before-write).
                for (size_t i = 0; i < pcopy.copies().size(); ++i) {
                    auto src_index = static_cast<u32>(pcopy.copies()[i].src);
                    if (!block_kill.get(src_index))
                        block_gen.set(src_index, true);
                }
                // Destinations are defs.
                for (size_t i = 0; i < pcopy.copies().size(); ++i) {
                    auto dst_index = static_cast<u32>(pcopy.copies()[i].dst);
                    block_kill.set(dst_index, true);
                }
            } else {
                // Operands are uses.
                for (size_t i = 0; i < instruction.operand_count(); ++i) {
                    if (auto operand_index = instruction.operand_index(i); operand_index.has_value()) {
                        auto index = static_cast<u32>(*operand_index);
                        if (!block_kill.get(index))
                            block_gen.set(index, true);
                    }
                }
                // Result is a def.
                if (auto result_index = instruction.result_index(); result_index.has_value()) {
                    block_kill.set(static_cast<u32>(*result_index), true);
                }
            }
        });
    }

    // Fixed-point iteration: recompute live_out and live_in each round.
    // Process blocks in reverse for faster convergence.
    auto scratch_out = MUST(Bitmap::create(value_count, false));
    auto scratch_in = MUST(Bitmap::create(value_count, false));
    bool changed = true;
    while (changed) {
        changed = false;
        for (size_t i = block_count; i-- > 0;) {
            auto& block = *blocks[i];

            // live_out[B] = union of live_in[S] for each successor S.
            __builtin_memset(scratch_out.data(), 0, scratch_out.size_in_bytes());
            CFG::for_each_successor(block, [&](BasicBlock& successor) {
                auto it = block_to_dense.find(successor.index());
                if (it == block_to_dense.end())
                    return;
                auto successor_dense = it->value;
                for (size_t bit = 0; bit < value_count; ++bit) {
                    if (live_in[successor_dense].get(bit))
                        scratch_out.set(bit, true);
                }
            });

            // live_in[B] = gen[B] | (live_out[B] - kill[B])
            __builtin_memset(scratch_in.data(), 0, scratch_in.size_in_bytes());
            for (size_t bit = 0; bit < value_count; ++bit) {
                if (gen[i].get(bit) || (scratch_out.get(bit) && !kill[i].get(bit)))
                    scratch_in.set(bit, true);
            }

            if (__builtin_memcmp(scratch_out.data(), live_out[i].data(), scratch_out.size_in_bytes()) != 0) {
                __builtin_memcpy(live_out[i].data(), scratch_out.data(), scratch_out.size_in_bytes());
                changed = true;
            }
            if (__builtin_memcmp(scratch_in.data(), live_in[i].data(), scratch_in.size_in_bytes()) != 0) {
                __builtin_memcpy(live_in[i].data(), scratch_in.data(), scratch_in.size_in_bytes());
                changed = true;
            }
        }
    }

    // Phase 2: Coalesce non-interfering copies using union-find.
    //
    // A copy dst <- src can be coalesced only if ALL copies that define
    // dst use the SAME src. If dst has different sources on different
    // paths, coalescing with one source would make the other paths
    // write to the wrong value.

    // For each dst value, track its unique source (or mark as non-coalesceable).
    // non_coalesceable[v] = true means v has copies from multiple different sources.
    HashMap<u32, u32> dst_to_unique_src;
    Bitmap non_coalesceable = MUST(Bitmap::create(value_count, false));

    for (auto& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() != Opcode::ParallelCopy)
                return;
            auto const& pcopy = static_cast<ParallelCopyInstruction const&>(instruction);
            for (size_t j = 0; j < pcopy.copies().size(); ++j) {
                auto dst = static_cast<u32>(pcopy.copies()[j].dst);
                auto src = static_cast<u32>(pcopy.copies()[j].src);

                if (non_coalesceable.get(dst))
                    continue;

                auto it = dst_to_unique_src.find(dst);
                if (it == dst_to_unique_src.end()) {
                    dst_to_unique_src.set(dst, src);
                } else if (it->value != src) {
                    non_coalesceable.set(dst, true);
                }
            }
        });
    }

    // Track values with non-ParallelCopy definitions. Once a
    // representative has such a def, merging it into another value
    // would leave the instruction writing to a dead value.
    Bitmap has_non_pcopy_def = MUST(Bitmap::create(value_count, false));
    for (auto& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() == Opcode::ParallelCopy)
                return;
            if (auto result_index = instruction.result_index(); result_index.has_value())
                has_non_pcopy_def.set(static_cast<u32>(*result_index), true);
        });
    }

    // Union-find data structure.
    Vector<u32> parent;
    parent.resize(value_count);
    for (u32 i = 0; i < value_count; ++i)
        parent[i] = i;

    auto find = [&](u32 x) -> u32 {
        while (parent[x] != x) {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        return x;
    };

    // Merge dst into src: src becomes the representative.
    auto merge_dst_into_src = [&](u32 dst, u32 src) {
        dst = find(dst);
        src = find(src);
        if (dst == src)
            return;
        parent[dst] = src;
    };

    bool any_coalesced = false;

    for (size_t block_index = 0; block_index < block_count; ++block_index) {
        auto& block = *blocks[block_index];
        auto const& instructions = block.instructions();
        if (instructions.is_empty())
            continue;

        // Create a working live set, initialized from live_out[B].
        auto live = MUST(Bitmap::create(value_count, false));
        __builtin_memcpy(live.data(), live_out[block_index].data(), live.size_in_bytes());

        // Walk instructions backward.
        for (size_t i = instructions.size(); i-- > 0;) {
            auto* instruction = function.instruction_by_index(instructions[i]);

            if (instruction->opcode() == Opcode::ParallelCopy) {
                auto const& pcopy = static_cast<ParallelCopyInstruction const&>(*instruction);

                // Check each copy for coalescing opportunity.
                for (size_t j = 0; j < pcopy.copies().size(); ++j) {
                    auto dst = static_cast<u32>(pcopy.copies()[j].dst);
                    auto src = static_cast<u32>(pcopy.copies()[j].src);

                    // Skip if dst has different sources on different paths.
                    if (non_coalesceable.get(dst))
                        continue;

                    auto dst_rep = find(dst);
                    auto src_rep = find(src);

                    // Already coalesced.
                    if (dst_rep == src_rep)
                        continue;

                    // If dst's representative has a non-ParallelCopy
                    // definition, merging it into src would leave that
                    // instruction writing to a dead value.
                    if (has_non_pcopy_def.get(dst_rep))
                        continue;

                    // Safe to coalesce if src is not live after the copy
                    // (no interference between dst and src).
                    if (!live.get(src)) {
                        merge_dst_into_src(dst, src);
                        any_coalesced = true;
                    }
                }

                // Update live set: remove all dsts, then add all srcs.
                for (size_t j = 0; j < pcopy.copies().size(); ++j)
                    live.set(static_cast<u32>(pcopy.copies()[j].dst), false);
                for (size_t j = 0; j < pcopy.copies().size(); ++j)
                    live.set(static_cast<u32>(pcopy.copies()[j].src), true);
            } else {
                // Remove result from live.
                if (auto result_index = instruction->result_index(); result_index.has_value())
                    live.set(static_cast<u32>(*result_index), false);

                // Add operands to live.
                for (size_t j = 0; j < instruction->operand_count(); ++j) {
                    if (auto operand_index = instruction->operand_index(j); operand_index.has_value())
                        live.set(static_cast<u32>(*operand_index), true);
                }
            }
        }
    }

    if (!any_coalesced)
        return PreservedAnalyses::all();

    // Phase 3: Rewrite IR.

    // Replace all uses of coalesced values with their representative.
    for (u32 i = 0; i < value_count; ++i) {
        auto representative = find(i);
        if (representative != i) {
            auto* value = function.values()[i].ptr();
            auto* representative_value = function.values()[representative].ptr();
            value->replace_all_uses_with(representative_value);
        }
    }

    // Rebuild ParallelCopy instructions, removing no-op copies.
    //
    // After SSA destruction, ParallelCopy instructions appear in two
    // positions: prepended at the start (handler edge copies) or just
    // before the terminator (normal edge copies). We detect position
    // by checking if any non-ParallelCopy, non-terminator instruction
    // precedes it.
    for (auto& block : function.basic_blocks()) {
        struct ReplacementCopy {
            Value* dst;
            Value* src;
        };

        Vector<ReplacementCopy> start_copies;
        Vector<ReplacementCopy> end_copies;
        bool has_parallel_copies = false;

        // Walk instructions to find ParallelCopy instructions and
        // classify them by position.
        bool seen_non_pcopy = false;
        for (auto instruction_index : block->instructions()) {
            auto* instruction = function.instruction_by_index(instruction_index);
            if (instruction->opcode() != Opcode::ParallelCopy) {
                if (!instruction->is_terminator())
                    seen_non_pcopy = true;
                continue;
            }

            has_parallel_copies = true;
            auto const& pcopy = static_cast<ParallelCopyInstruction const&>(*instruction);

            // If we haven't seen a non-ParallelCopy instruction yet,
            // this is a start (handler) copy. Otherwise it's an end copy.
            auto& target = seen_non_pcopy ? end_copies : start_copies;

            for (size_t j = 0; j < pcopy.copies().size(); ++j) {
                auto dst = find(static_cast<u32>(pcopy.copies()[j].dst));
                auto src = find(static_cast<u32>(pcopy.copies()[j].src));
                if (dst != src) {
                    target.append({
                        function.values()[dst].ptr(),
                        function.values()[src].ptr(),
                    });
                }
            }
        }

        if (!has_parallel_copies)
            continue;

        // Remove all old ParallelCopy instructions.
        block->remove_instructions_if([](Instruction const& instruction) {
            return instruction.opcode() == Opcode::ParallelCopy;
        });

        // Re-insert at start if needed.
        if (!start_copies.is_empty()) {
            auto new_pcopy = ParallelCopyInstruction::create();
            for (auto& copy : start_copies)
                new_pcopy->add_copy(copy.dst, copy.src);
            block->prepend(move(new_pcopy));
        }

        // Re-insert before terminator if needed.
        if (!end_copies.is_empty()) {
            auto new_pcopy = ParallelCopyInstruction::create();
            for (auto& copy : end_copies)
                new_pcopy->add_copy(copy.dst, copy.src);
            block->insert_before_terminator(move(new_pcopy));
        }
    }

    return PreservedAnalyses::all_cfg_analyses();
}

}
