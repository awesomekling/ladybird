/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/PhiCoalescing.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

PhiCoalescing::PhiCoalescing(Function const& function)
    : m_function(function)
{
    m_representative.resize(function.values().size());
    compute();
}

Value const* PhiCoalescing::find_representative(Value const* v) const
{
    Vector<Value const*> path;
    for (;;) {
        auto const* rep = m_representative[static_cast<u32>(v->index())];
        if (!rep)
            break;
        path.append(v);
        v = rep;
    }
    // Path compression
    for (auto const* p : path)
        m_representative[static_cast<u32>(p->index())] = v;
    return v;
}

Value const* PhiCoalescing::representative(Value const& value) const
{
    return find_representative(&value);
}

void PhiCoalescing::compute()
{
    auto coalesce = [&](Value const* a, Value const* b) {
        auto const* rep_a = find_representative(a);
        auto const* rep_b = find_representative(b);
        if (rep_a != rep_b)
            m_representative[static_cast<u32>(rep_a->index())] = rep_b;
    };

    for (auto const& block : m_function.basic_blocks()) {
        for (auto const& instruction : block->instructions()) {
            if (instruction->opcode() != Opcode::Phi)
                break; // Phis are always at the start

            auto* phi_result = instruction->result();
            if (!phi_result)
                continue;

            // Helper to check if coalescing this operand with the phi result would
            // create a conflict with another operand of the same phi.
            // Two operands of the same phi represent different values from different
            // control flow paths and must NOT share a register.
            auto would_conflict_with_other_operand = [&](Value const* operand_to_check) -> bool {
                auto const* rep_to_check = find_representative(operand_to_check);
                auto const* phi_rep = find_representative(phi_result);

                for (auto* other_operand : instruction->operands()) {
                    if (!other_operand || other_operand == operand_to_check)
                        continue;
                    if (other_operand->is_constant() || other_operand->is_parameter() || other_operand->is_this())
                        continue;

                    auto const* other_rep = find_representative(other_operand);

                    // If this operand is already in the same equivalence class as
                    // another operand, coalescing would make phi_result share a
                    // register with multiple different incoming values.
                    if (rep_to_check == other_rep)
                        return true;

                    // If another operand is already coalesced with phi_result,
                    // coalescing this one too would make both operands share a register.
                    if (other_rep == phi_rep)
                        return true;
                }
                return false;
            };

            auto const& phi = static_cast<PhiInstruction const&>(*instruction);

            for (size_t i = 0; i < phi.incoming_count(); ++i) {
                auto* operand = phi.incoming_value(i);
                if (!operand)
                    continue;

                // Can't coalesce constants, parameters, or this
                if (operand->is_constant() || operand->is_parameter() || operand->is_this())
                    continue;

                // Yield and Await results are fixed to the accumulator (reg0)
                // at runtime. Coalescing them with a phi result would assign
                // them a different register, losing the resume value.
                if (auto* def = operand->defining_instruction()) {
                    if (def->opcode() == Opcode::Yield || def->opcode() == Opcode::Await)
                        continue;
                }

                // Back-edge operands (from the same block as the phi) cannot be
                // coalesced with the phi result. The operand is defined later in
                // the block while the phi result is still live, so they have
                // overlapping lifetimes.
                if (phi.incoming_block(i) == block.ptr())
                    continue;

                // Check for conflicts before any coalescing
                if (would_conflict_with_other_operand(operand))
                    continue;

                // Check if the phi result is still live in the incoming block after
                // the point where the incoming value is defined. If so, they have
                // overlapping lifetimes and cannot share a register.
                auto* incoming_block = phi.incoming_block(i);
                if (auto* defining = operand->defining_instruction()) {
                    bool has_interference = false;
                    bool past_definition = false;
                    // We must check not only for direct uses of phi_result,
                    // but also for uses of any value already coalesced with
                    // phi_result. If a coalesced value is still live after
                    // the operand's definition, sharing a register would
                    // clobber that value.
                    auto const* phi_rep = find_representative(phi_result);
                    for (auto const& inst : incoming_block->instructions()) {
                        if (inst.ptr() == defining) {
                            past_definition = true;
                            continue;
                        }
                        if (past_definition) {
                            for (auto* use_operand : inst->operands()) {
                                if (use_operand && find_representative(use_operand) == phi_rep) {
                                    has_interference = true;
                                    break;
                                }
                            }
                            if (has_interference)
                                break;
                        }
                    }

                    // Also check if phi_result is used as a phi input from the
                    // same predecessor for a sibling phi in this block.
                    // If the operand's defining instruction is in the incoming
                    // block, it would overwrite phi_result's register before the
                    // phi moves read it for the sibling phi.
                    if (!has_interference && defining->parent_block() == incoming_block) {
                        for (auto const& sibling : block->instructions()) {
                            if (sibling->opcode() != Opcode::Phi)
                                break;
                            if (sibling.ptr() == instruction.ptr())
                                continue;
                            auto const& sibling_phi = static_cast<PhiInstruction const&>(*sibling);
                            for (size_t k = 0; k < sibling_phi.incoming_count(); ++k) {
                                if (sibling_phi.incoming_block(k) == incoming_block && sibling_phi.incoming_value(k) == phi_result) {
                                    has_interference = true;
                                    break;
                                }
                            }
                            if (has_interference)
                                break;
                        }
                    }

                    if (has_interference)
                        continue;
                }

                // Chain coalescing: if operand is a phi result in a DIFFERENT block,
                // coalesce the two phis. This makes chains like:
                // block1: v14 = Phi[v0,v2]
                // block2: v15 = Phi[v14,v4]
                // all share the same register.
                //
                // However, we must not coalesce two phis that form a swap cycle.
                // E.g. v1 = Phi[.., v2], v2 = Phi[.., v1] — if coalesced, both
                // map to the same register and the swap becomes a no-op.
                //
                // We also must not chain-coalesce phi results in the SAME block.
                // Two phi results in the same block both become live at the block
                // entry and may hold different values, so they need separate
                // registers unless the standard check proves otherwise.
                if (auto* def = operand->defining_instruction(); def && def->opcode() == Opcode::Phi && def->parent_block() != block.ptr()) {
                    auto const& other_phi = static_cast<PhiInstruction const&>(*def);
                    bool forms_cycle = false;
                    for (size_t j = 0; j < other_phi.incoming_count(); ++j) {
                        auto* other_input = other_phi.incoming_value(j);
                        if (!other_input)
                            continue;
                        if (find_representative(other_input) == find_representative(phi_result)) {
                            forms_cycle = true;
                            break;
                        }
                    }
                    if (!forms_cycle) {
                        // Verify the operand is dead after this phi edge.
                        // If it has uses outside its defining block (other than
                        // this phi), its live range extends through blocks where
                        // phi_result may hold a different value due to back-edge
                        // updates, making register sharing unsafe.
                        bool has_external_use = false;
                        for (auto* use : operand->uses()) {
                            if (use == instruction.ptr())
                                continue;
                            if (use->parent_block() != def->parent_block()) {
                                has_external_use = true;
                                break;
                            }
                        }
                        if (!has_external_use) {
                            coalesce(operand, phi_result);
                            continue;
                        }
                    }
                }

                // Standard coalescing: if all of operand's non-phi uses are terminators
                // (Branch, Return, etc.), then the operand is dead at the phi point.
                bool can_coalesce = true;
                for (auto* use : operand->uses()) {
                    if (use == instruction.ptr()) // This phi
                        continue;
                    if (!use->is_terminator()) {
                        can_coalesce = false;
                        break;
                    }
                }
                if (can_coalesce)
                    coalesce(operand, phi_result);
            }
        }
    }
}

}
