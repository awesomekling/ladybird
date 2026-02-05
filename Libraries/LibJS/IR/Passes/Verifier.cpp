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
        // Check: Block parent pointer
        if (block->parent_function() != &function) {
            report_error(ByteString::formatted(
                "Block{} has wrong parent_function",
                block->index()));
        }

        // Check: Predecessor list has no duplicates
        {
            HashTable<BasicBlock*> seen_preds;
            for (auto* pred : block->predecessors()) {
                if (seen_preds.contains(pred)) {
                    report_error(ByteString::formatted(
                        "Block{} has duplicate predecessor block{}",
                        block->index(), pred->index()));
                }
                seen_preds.set(pred);
            }
        }

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

            // Check: Block must end with a terminator
            auto* last_instr = block->instructions().last().ptr();
            if (!last_instr->is_terminator()) {
                report_error(ByteString::formatted(
                    "Block{} does not end with a terminator",
                    block->index()));
            }
        }

        for (auto const& instr : block->instructions()) {
            // Check: Instruction parent pointer
            if (instr->parent_block() != block.ptr()) {
                report_error(ByteString::formatted(
                    "Instruction in block{} has wrong parent_block (points to block{})",
                    block->index(), instr->parent_block() ? instr->parent_block()->index() : -1));
            }

            // Check: Phi operand count == phi predecessor count
            if (instr->opcode() == Opcode::Phi) {
                auto& phi = static_cast<PhiInstruction const&>(*instr);
                if (phi.operands().size() != phi.incoming_count()) {
                    report_error(ByteString::formatted(
                        "Phi in block{} has {} operands but {} predecessors",
                        block->index(), phi.operands().size(), phi.incoming_count()));
                }

                // Check 3: Phi predecessors ⊆ block predecessors
                for (size_t i = 0; i < phi.incoming_count(); ++i) {
                    auto* phi_pred = phi.incoming_block(i);
                    if (!block_predecessor_set.contains(phi_pred)) {
                        report_error(ByteString::formatted(
                            "Phi in block{} has predecessor block{} not in block's predecessor list",
                            block->index(), phi_pred->index()));
                    }
                }
            }

            // NB: The invariant "only terminators may have CFG targets" is now enforced
            // at compile time via TerminatorInstruction - only that class has target methods.

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

        // Check: Terminator target shape matches opcode
        if (auto* term = block->terminator()) {
            switch (term->opcode()) {
            case Opcode::Jump:
                if (!term->true_target()) {
                    report_error(ByteString::formatted(
                        "Jump in block{} has no target",
                        block->index()));
                }
                if (term->false_target()) {
                    report_error(ByteString::formatted(
                        "Jump in block{} has false_target (should be null)",
                        block->index()));
                }
                break;
            case Opcode::Branch:
                if (!term->true_target() || !term->false_target()) {
                    report_error(ByteString::formatted(
                        "Branch in block{} missing true_target or false_target",
                        block->index()));
                }
                break;
            case Opcode::Return:
            case Opcode::Throw:
            case Opcode::End:
                if (term->true_target() || term->false_target()) {
                    report_error(ByteString::formatted(
                        "{} in block{} has targets (should have none)",
                        opcode_to_string(term->opcode()), block->index()));
                }
                break;
            case Opcode::Yield:
            case Opcode::Await:
                if (term->false_target()) {
                    report_error(ByteString::formatted(
                        "{} in block{} has false_target (should be null)",
                        opcode_to_string(term->opcode()), block->index()));
                }
                break;
            default:
                break;
            }
        }

        // Check: Successor edges must be reflected in predecessor lists
        // (ensures CFG is consistent after transformations)
        if (auto* term = block->terminator()) {
            // Check: All terminator targets exist in function
            if (term->true_target() && !all_blocks.contains(term->true_target())) {
                report_error(ByteString::formatted(
                    "Terminator in block{} has true_target not in function",
                    block->index()));
            }
            if (term->false_target() && !all_blocks.contains(term->false_target())) {
                report_error(ByteString::formatted(
                    "Terminator in block{} has false_target not in function",
                    block->index()));
            }

            if (auto* true_target = term->true_target()) {
                bool found = false;
                for (auto* pred : true_target->predecessors()) {
                    if (pred == block.ptr()) {
                        found = true;
                        break;
                    }
                }
                if (!found) {
                    report_error(ByteString::formatted(
                        "Block{} has successor block{} but is not in its predecessor list",
                        block->index(), true_target->index()));
                }
            }
            if (auto* false_target = term->false_target()) {
                bool found = false;
                for (auto* pred : false_target->predecessors()) {
                    if (pred == block.ptr()) {
                        found = true;
                        break;
                    }
                }
                if (!found) {
                    report_error(ByteString::formatted(
                        "Block{} has successor block{} but is not in its predecessor list",
                        block->index(), false_target->index()));
                }
            }
        }
    }

    // Check: Use lists should only contain instructions still present in the function
    // (catches stale references from dead block elimination or other passes)
    HashTable<Instruction const*> all_instructions;
    for (auto const& block : function.basic_blocks()) {
        for (auto const& instr : block->instructions())
            all_instructions.set(instr.ptr());
    }

    for (auto const& value : function.values()) {
        for (auto const* use : value->uses()) {
            if (!all_instructions.contains(use)) {
                report_error(ByteString::formatted(
                    "Value v{} has stale use pointing to removed instruction",
                    value->index()));
            }
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
                auto const& phi = static_cast<PhiInstruction const&>(*instr);
                for (size_t i = 0; i < phi.incoming_count(); ++i) {
                    auto* operand = phi.incoming_value(i);
                    auto* pred = phi.incoming_block(i);
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
