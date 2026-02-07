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

    // Phase 2: Build interference graph.
    //
    // Walk all blocks backward, recording pairwise interference between
    // values that are simultaneously live. For ParallelCopy instructions,
    // we apply the phi exception: dst does not interfere with src.

    auto bitmap_words = (value_count + 7) / 8;
    Vector<Bitmap> interferes_with;
    interferes_with.ensure_capacity(value_count);
    for (size_t i = 0; i < value_count; ++i)
        interferes_with.unchecked_append(MUST(Bitmap::create(value_count, false)));

    for (size_t block_index = 0; block_index < block_count; ++block_index) {
        auto& block = *blocks[block_index];
        auto const& instructions = block.instructions();
        if (instructions.is_empty())
            continue;

        auto live = MUST(Bitmap::create(value_count, false));
        __builtin_memcpy(live.data(), live_out[block_index].data(), live.size_in_bytes());

        for (size_t i = instructions.size(); i-- > 0;) {
            auto* instruction = function.instruction_by_index(instructions[i]);

            if (instruction->opcode() == Opcode::ParallelCopy) {
                auto const& pcopy = static_cast<ParallelCopyInstruction const&>(*instruction);

                // Collect all dsts in this ParallelCopy so we can restrict
                // the phi exception: dst must not lose interference with a
                // src that is itself a dst in the same ParallelCopy (swap).
                auto scratch_dsts = MUST(Bitmap::create(value_count, false));
                for (size_t j = 0; j < pcopy.copies().size(); ++j)
                    scratch_dsts.set(static_cast<u32>(pcopy.copies()[j].dst), true);

                // For each copy dst <- src: dst interferes with everything
                // live, EXCEPT src (phi exception). The phi exception is
                // suppressed when src is also a dst in the same copy, since
                // swapped values must interfere.
                for (size_t j = 0; j < pcopy.copies().size(); ++j) {
                    auto dst = static_cast<u32>(pcopy.copies()[j].dst);
                    auto src = static_cast<u32>(pcopy.copies()[j].src);

                    bool phi_exception = !scratch_dsts.get(src);

                    // dst interferes with all live values.
                    for (size_t w = 0; w < bitmap_words; ++w)
                        interferes_with[dst].data()[w] |= live.data()[w];

                    // Remove self-interference and (conditionally) phi exception.
                    interferes_with[dst].set(dst, false);
                    if (phi_exception)
                        interferes_with[dst].set(src, false);

                    // Symmetric: mark live values as interfering with dst.
                    for (size_t bit = 0; bit < value_count; ++bit) {
                        if (live.get(bit) && (bit != src || !phi_exception))
                            interferes_with[bit].set(dst, true);
                    }
                }

                // Update live set: remove all dsts, then add all srcs.
                for (size_t j = 0; j < pcopy.copies().size(); ++j)
                    live.set(static_cast<u32>(pcopy.copies()[j].dst), false);
                for (size_t j = 0; j < pcopy.copies().size(); ++j)
                    live.set(static_cast<u32>(pcopy.copies()[j].src), true);
            } else {
                if (auto result_index = instruction->result_index(); result_index.has_value()) {
                    auto d = static_cast<u32>(*result_index);

                    // d interferes with all live values.
                    for (size_t w = 0; w < bitmap_words; ++w)
                        interferes_with[d].data()[w] |= live.data()[w];
                    interferes_with[d].set(d, false);

                    // Symmetric.
                    for (size_t bit = 0; bit < value_count; ++bit) {
                        if (live.get(bit))
                            interferes_with[bit].set(d, true);
                    }

                    live.set(d, false);
                }

                for (size_t j = 0; j < instruction->operand_count(); ++j) {
                    if (auto operand_index = instruction->operand_index(j); operand_index.has_value())
                        live.set(static_cast<u32>(*operand_index), true);
                }
            }
        }
    }

    // Phase 3: Coalesce using interference graph with union-find.

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

    // Track members of each equivalence class for cross-class checks.
    Vector<Vector<u32>> class_members;
    class_members.resize(value_count);
    for (u32 i = 0; i < value_count; ++i)
        class_members[i].append(i);

    auto is_non_coalesceable_value = [&](u32 index) -> bool {
        auto const& value = *function.values()[index];
        if (value.is_constant() || value.is_parameter() || value.is_this())
            return true;
        if (auto* defining = value.defining_instruction()) {
            if (defining->opcode() == Opcode::Yield || defining->opcode() == Opcode::Await)
                return true;
            // ExtractValue results are bound to specific tuple element
            // registers in the Lowerer and cannot be reassigned.
            if (defining->opcode() == Opcode::ExtractValue)
                return true;
            // Comparison results used as Branch conditions may be fused with
            // the Branch in the Lowerer (JumpStrictlyEquals, etc). Fusion
            // skips writing the result to a register, so coalescing another
            // value into the same register would leave that register
            // uninitialized on the fused path. Guard against this by marking
            // comparison results that feed a Branch as non-coalesceable.
            switch (defining->opcode()) {
            case Opcode::LessThan:
            case Opcode::LessThanEquals:
            case Opcode::GreaterThan:
            case Opcode::GreaterThanEquals:
            case Opcode::LooselyEquals:
            case Opcode::StrictlyEquals:
            case Opcode::LooselyInequals:
            case Opcode::StrictlyInequals: {
                for (auto const& use : value.uses()) {
                    auto* using_instruction = function.instruction_by_index(use.instruction);
                    if (using_instruction->opcode() == Opcode::Branch
                        && using_instruction->operand(0) == &value)
                        return true;
                }
                break;
            }
            default:
                break;
            }
        }
        return false;
    };

    bool any_coalesced = false;

    for (auto& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() != Opcode::ParallelCopy)
                return;
            auto const& pcopy = static_cast<ParallelCopyInstruction const&>(instruction);

            for (size_t j = 0; j < pcopy.copies().size(); ++j) {
                auto dst = static_cast<u32>(pcopy.copies()[j].dst);
                auto src = static_cast<u32>(pcopy.copies()[j].src);

                if (is_non_coalesceable_value(dst) || is_non_coalesceable_value(src))
                    continue;

                auto dst_rep = find(dst);
                auto src_rep = find(src);
                if (dst_rep == src_rep)
                    continue;

                // Cross-class interference check.
                bool has_interference = false;
                for (auto a : class_members[dst_rep]) {
                    for (auto b : class_members[src_rep]) {
                        if (interferes_with[a].get(b)) {
                            has_interference = true;
                            break;
                        }
                    }
                    if (has_interference)
                        break;
                }
                if (has_interference)
                    continue;

                // Merge dst class into src class.
                auto& dst_members = class_members[dst_rep];
                auto& src_members = class_members[src_rep];
                src_members.extend(move(dst_members));
                parent[dst_rep] = src_rep;
                any_coalesced = true;
            }
        });
    }

    if (!any_coalesced)
        return PreservedAnalyses::all();

    // Phase 4: Store coalescing map and prune copies.

    Vector<ValueIndex> coalescing_map;
    coalescing_map.resize(value_count);
    for (u32 i = 0; i < value_count; ++i)
        coalescing_map[i] = ValueIndex(find(i));
    function.set_coalescing_map(move(coalescing_map));

    // Remove coalesced copies from ParallelCopy instructions.
    // Keep the original Value pointers — do NOT call replace_all_uses_with.
    for (auto& block : function.basic_blocks()) {
        bool has_parallel_copies = false;
        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() == Opcode::ParallelCopy)
                has_parallel_copies = true;
        });
        if (!has_parallel_copies)
            continue;

        struct PositionedCopy {
            Value* dst;
            Value* src;
        };

        Vector<PositionedCopy> start_copies;
        Vector<PositionedCopy> end_copies;

        bool seen_non_pcopy = false;
        for (auto instruction_index : block->instructions()) {
            auto* instruction = function.instruction_by_index(instruction_index);
            if (instruction->opcode() != Opcode::ParallelCopy) {
                if (!instruction->is_terminator())
                    seen_non_pcopy = true;
                continue;
            }

            auto const& pcopy = static_cast<ParallelCopyInstruction const&>(*instruction);
            auto& target = seen_non_pcopy ? end_copies : start_copies;

            for (size_t j = 0; j < pcopy.copies().size(); ++j) {
                auto dst = static_cast<u32>(pcopy.copies()[j].dst);
                auto src = static_cast<u32>(pcopy.copies()[j].src);
                if (find(dst) != find(src)) {
                    target.append({
                        function.values()[dst].ptr(),
                        function.values()[src].ptr(),
                    });
                }
            }
        }

        block->remove_instructions_if([](Instruction const& instruction) {
            return instruction.opcode() == Opcode::ParallelCopy;
        });

        if (!start_copies.is_empty()) {
            auto new_pcopy = ParallelCopyInstruction::create();
            for (auto& copy : start_copies)
                new_pcopy->add_copy(copy.dst, copy.src);
            block->prepend(move(new_pcopy));
        }

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
