/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Dominators.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/Verifier.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

bool Verifier::run(Function& function)
{
    verify(function, true);
    return false; // Verifier never modifies the IR
}

bool Verifier::verify(Function& function, bool crash_on_error)
{
    bool valid = true;

    auto report_error = [&](StringView message) {
        if (crash_on_error) {
            warnln("IR Verifier: {}", message);
            VERIFY_NOT_REACHED();
        }
        valid = false;
    };

    // Build set of all blocks for quick lookup
    HashTable<BasicBlock*> all_blocks;
    for (auto const& block : function.basic_blocks())
        all_blocks.set(block.ptr());

    // Check 1: Entry block has no predecessors
    if (function.entry_block() && !function.entry_block()->predecessors().is_empty()) {
        report_error("Entry block has predecessors"sv);
    }

    // Build set of all defined values
    HashTable<Value const*> defined_values;
    for (auto const& block : function.basic_blocks()) {
        for (auto const& instr : block->instructions()) {
            if (instr->result())
                defined_values.set(instr->result());
        }
    }

    // Add parameter, constant, and this values as defined
    for (auto const& value : function.values()) {
        if (value->is_parameter() || value->is_constant() || value->is_this())
            defined_values.set(value.ptr());
    }

    for (auto const& block : function.basic_blocks()) {
        HashTable<BasicBlock*> block_predecessor_set;
        for (auto* pred : block->predecessors())
            block_predecessor_set.set(pred);

        // Check: Block structure invariants
        if (!block->instructions().is_empty()) {
            // Check: "Phis first" - all phi nodes must come before non-phi instructions
            bool seen_non_phi = false;
            for (auto const& instr : block->instructions()) {
                if (instr->opcode() == Opcode::Phi) {
                    if (seen_non_phi) {
                        report_error(ByteString::formatted(
                            "Block{} has Phi after non-Phi instruction",
                            block->index()));
                        break;
                    }
                } else {
                    seen_non_phi = true;
                }
            }

            // Check: "Terminator last" - terminator must be last instruction
            for (size_t i = 0; i < block->instructions().size() - 1; ++i) {
                if (block->instructions()[i]->is_terminator()) {
                    report_error(ByteString::formatted(
                        "Block{} has terminator before last instruction",
                        block->index()));
                    break;
                }
            }
        }

        for (auto const& instr : block->instructions()) {
            // Check: Phi operand count == phi predecessor count
            if (instr->opcode() == Opcode::Phi) {
                if (instr->operands().size() != instr->phi_predecessors().size()) {
                    report_error(ByteString::formatted(
                        "Phi in block{} has {} operands but {} predecessors",
                        block->index(), instr->operands().size(), instr->phi_predecessors().size()));
                }

                // Check 3: Phi predecessors ⊆ block predecessors
                for (auto* phi_pred : instr->phi_predecessors()) {
                    if (!block_predecessor_set.contains(phi_pred)) {
                        report_error(ByteString::formatted(
                            "Phi in block{} has predecessor block{} not in block's predecessor list",
                            block->index(), phi_pred->index()));
                    }
                }
            }

            // Check: Only terminators may have CFG targets
            // This invariant is relied upon by dominator computation which computes
            // successors from the last instruction's targets and EH edges only.
            if (!instr->is_terminator() && (instr->true_target() || instr->false_target())) {
                report_error(ByteString::formatted(
                    "Non-terminator instruction in block{} has CFG targets",
                    block->index()));
            }

            // Check: All terminator targets exist in function
            if (instr->true_target() && !all_blocks.contains(instr->true_target())) {
                report_error(ByteString::formatted(
                    "Instruction in block{} has true_target not in function",
                    block->index()));
            }
            if (instr->false_target() && !all_blocks.contains(instr->false_target())) {
                report_error(ByteString::formatted(
                    "Instruction in block{} has false_target not in function",
                    block->index()));
            }

            // Check: All operands are non-null and reference defined values
            for (size_t i = 0; i < instr->operands().size(); ++i) {
                auto* operand = instr->operands()[i];
                if (!operand) {
                    report_error(ByteString::formatted(
                        "Instruction in block{} has null operand at index {}",
                        block->index(), i));
                    continue;
                }
                if (!defined_values.contains(operand)) {
                    report_error(ByteString::formatted(
                        "Instruction in block{} uses undefined value v{}",
                        block->index(), operand->index()));
                }
                // Check: Instruction-kind values must have a defining instruction
                // This catches placeholder register values that weren't properly renamed
                if (operand->is_instruction() && !operand->defining_instruction()) {
                    report_error(ByteString::formatted(
                        "Instruction in block{} uses v{} which has no defining instruction (likely SSA renaming failure)",
                        block->index(), operand->index()));
                }
            }
        }

        // Check 6: Exception handler/finalizer targets exist
        if (block->exception_handler() && !all_blocks.contains(block->exception_handler())) {
            report_error(ByteString::formatted(
                "Block{} has exception_handler not in function",
                block->index()));
        }
        if (block->finalizer() && !all_blocks.contains(block->finalizer())) {
            report_error(ByteString::formatted(
                "Block{} has finalizer not in function",
                block->index()));
        }
    }

    // SSA dominance verification
    // This check implicitly verifies EH correctness: if exception handler blocks
    // reference values that were defined after a throw point (due to incorrect EH
    // splitting), those values won't dominate the handler and we'll report an error.
    //
    // Build map from Value* to its defining block
    HashMap<Value const*, BasicBlock*> value_to_block;
    for (auto const& block : function.basic_blocks()) {
        for (auto const& instr : block->instructions()) {
            if (instr->result())
                value_to_block.set(instr->result(), block.ptr());
        }
    }

    // Compute dominators for dominance checking
    Dominators dominators(function);

    for (auto const& block : function.basic_blocks()) {
        for (auto const& instr : block->instructions()) {
            if (instr->opcode() == Opcode::Phi) {
                // For phi instructions, each operand must be reachable from its corresponding predecessor
                // The defining block must dominate the predecessor (not the current block)
                auto const& operands = instr->operands();
                auto const& phi_preds = instr->phi_predecessors();
                for (size_t i = 0; i < operands.size() && i < phi_preds.size(); ++i) {
                    auto* operand = operands[i];
                    auto* pred = phi_preds[i];
                    if (!operand)
                        continue;

                    // Skip constants, parameters, and this values - they dominate everything
                    if (operand->is_constant() || operand->is_parameter() || operand->is_this())
                        continue;

                    auto def_block = value_to_block.get(operand);
                    if (!def_block.has_value()) {
                        report_error(ByteString::formatted(
                            "Phi operand v{} in block{} has no defining block",
                            operand->index(), block->index()));
                        continue;
                    }

                    // The definition must dominate the predecessor block
                    // (the value flows from pred -> current block via the phi)
                    if (!dominators.dominates(*def_block, pred)) {
                        report_error(ByteString::formatted(
                            "SSA violation: phi operand v{} (defined in block{}) does not dominate predecessor block{} for phi in block{}",
                            operand->index(), (*def_block)->index(), pred->index(), block->index()));
                    }
                }
            } else {
                // For non-phi instructions, each operand's definition must dominate this block
                for (size_t i = 0; i < instr->operands().size(); ++i) {
                    auto* operand = instr->operands()[i];
                    if (!operand)
                        continue;

                    // Skip constants, parameters, and this values - they dominate everything
                    if (operand->is_constant() || operand->is_parameter() || operand->is_this())
                        continue;

                    auto def_block = value_to_block.get(operand);
                    if (!def_block.has_value()) {
                        report_error(ByteString::formatted(
                            "Operand v{} in block{} has no defining block",
                            operand->index(), block->index()));
                        continue;
                    }

                    // Check dominance: def_block must dominate use_block
                    // If in same block, check instruction order
                    if (*def_block == block.ptr()) {
                        // Same block: definition must come before use
                        bool found_def = false;
                        for (auto const& check_instr : block->instructions()) {
                            if (check_instr->result() == operand) {
                                found_def = true;
                            }
                            if (check_instr.ptr() == instr.ptr()) {
                                if (!found_def) {
                                    report_error(ByteString::formatted(
                                        "SSA violation: operand v{} used before definition in block{}",
                                        operand->index(), block->index()));
                                }
                                break;
                            }
                        }
                    } else if (!dominators.dominates(*def_block, block.ptr())) {
                        report_error(ByteString::formatted(
                            "SSA violation: operand v{} (defined in block{}) does not dominate use in block{}",
                            operand->index(), (*def_block)->index(), block->index()));
                    }
                }
            }
        }
    }

    return valid;
}

}
