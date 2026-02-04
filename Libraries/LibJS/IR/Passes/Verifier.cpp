/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
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

    // Add parameter values as defined
    for (auto const& value : function.values()) {
        if (value->is_parameter() || value->is_constant())
            defined_values.set(value.ptr());
    }

    for (auto const& block : function.basic_blocks()) {
        HashTable<BasicBlock*> block_predecessor_set;
        for (auto* pred : block->predecessors())
            block_predecessor_set.set(pred);

        for (auto const& instr : block->instructions()) {
            // Check 2: Phi operand count == phi predecessor count
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

            // Check 4: All terminator targets exist in function
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

            // Check 5: All operands reference defined values
            for (auto* operand : instr->operands()) {
                if (!defined_values.contains(operand)) {
                    report_error(ByteString::formatted(
                        "Instruction in block{} uses undefined value v{}",
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

    return valid;
}

}
