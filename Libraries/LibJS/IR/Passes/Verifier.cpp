/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Bitmap.h>
#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/DominatorTree.h>
#include <LibJS/IR/Dump.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/Verifier.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

static inline size_t to_index(BlockIndex b) { return static_cast<u32>(b); }
static inline size_t to_index(ValueIndex v) { return static_cast<u32>(v); }

static size_t block_index_capacity(Function const& function)
{
    size_t max_index = 0;
    for (auto const& block : function.basic_blocks())
        max_index = max(max_index, to_index(block->index()) + 1);
    return max_index;
}

PreservedAnalyses Verifier::run(Function& function, PassManager&)
{
    verify(function, VerifierMode::InterPass, true);
    return PreservedAnalyses::all(); // Verifier never modifies the IR
}

bool Verifier::verify(Function& function, VerifierMode mode, bool crash_on_error)
{
    bool valid = true;

    auto report_error = [&](StringView message) {
        if (crash_on_error) {
            warnln("IR Verifier: {}", message);
            warnln("{}", dump(function));
            VERIFY_NOT_REACHED();
        }
        valid = false;
    };

    bool const full_mode = mode == VerifierMode::Full;
    bool const post_ssa = function.stage() >= IRStage::PostSSA;
    auto block_capacity = block_index_capacity(function);
    auto value_capacity = function.values().size();

    // Build set of all blocks for quick lookup
    auto all_blocks = MUST(AK::Bitmap::create(block_capacity, false));
    for (auto const& block : function.basic_blocks())
        all_blocks.set(to_index(block->index()), true);

    // Check: Block index map consistency
    for (auto const& block : function.basic_blocks()) {
        auto* looked_up = function.block_by_index(block->index());
        if (looked_up != block.ptr()) {
            report_error(ByteString::formatted(
                "block_by_index(block{}) returns wrong block",
                block->index()));
        }
    }

    // Check: Entry block presence and membership
    if (!function.basic_blocks().is_empty()) {
        if (!function.entry_block()) {
            report_error("Function has blocks but no entry block"sv);
        } else if (!all_blocks.get(to_index(function.entry_block()->index()))) {
            report_error("Entry block is not in the block list"sv);
        }
    } else if (function.entry_block()) {
        report_error("Function has no blocks but entry block is set"sv);
    }

    // Check: Entry block has no predecessors
    if (function.entry_block() && !function.entry_block()->predecessor_indices().is_empty()) {
        report_error("Entry block has predecessors"sv);
    }

    // Compute reachable blocks via BFS (including EH/finalizer edges)
    auto reachable = MUST(AK::Bitmap::create(block_capacity, false));
    if (auto* entry = function.entry_block()) {
        Vector<BasicBlock*> worklist;
        worklist.append(entry);
        reachable.set(to_index(entry->index()), true);
        while (!worklist.is_empty()) {
            auto* current = worklist.take_last();
            CFG::for_each_successor(*current, [&](BasicBlock& target) {
                auto ti = to_index(target.index());
                if (!reachable.get(ti)) {
                    reachable.set(ti, true);
                    worklist.append(&target);
                }
            });
        }
    }

    // Full mode: no unreachable blocks allowed
    if (full_mode) {
        for (auto const& block : function.basic_blocks()) {
            if (!reachable.get(to_index(block->index()))) {
                report_error(ByteString::formatted(
                    "Block{} is unreachable from entry block",
                    block->index()));
            }
        }
    }

    // Full mode: block index uniqueness
    if (full_mode) {
        HashTable<BlockIndex> seen_indices;
        for (auto const& block : function.basic_blocks()) {
            if (seen_indices.contains(block->index())) {
                report_error(ByteString::formatted(
                    "Duplicate block index {}",
                    block->index()));
            }
            seen_indices.set(block->index());
        }
    }

    // Check: Instruction index uniqueness across blocks
    // Each instruction index must appear in exactly one block's instruction list.
    {
        HashTable<InstructionIndex> seen_instructions;
        for (auto const& block : function.basic_blocks()) {
            HashTable<InstructionIndex> seen_in_block;
            for (auto instruction_index : block->instructions()) {
                // Check: No duplicate instruction indices within a block
                if (seen_in_block.contains(instruction_index)) {
                    report_error(ByteString::formatted(
                        "Block{} has duplicate instruction index {}",
                        block->index(), instruction_index));
                }
                seen_in_block.set(instruction_index);

                // Check: Instruction index not in any other block
                if (seen_instructions.contains(instruction_index)) {
                    report_error(ByteString::formatted(
                        "Instruction index {} appears in multiple blocks (found in block{})",
                        instruction_index, block->index()));
                }
                seen_instructions.set(instruction_index);
            }
        }
    }

    // Build set of all defined values, checking for unique definitions
    auto defined_values = MUST(AK::Bitmap::create(value_capacity, false));
    for (auto const& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.result()) {
                auto* result = instruction.result();
                auto vi = to_index(result->index());
                // Check: No two instructions share the same result Value*
                if (defined_values.get(vi)) {
                    report_error(ByteString::formatted(
                        "Value v{} is defined by multiple instructions",
                        result->index()));
                }
                defined_values.set(vi, true);

                // Check: Result/defining-instruction round-trip
                if (!result->is_instruction()) {
                    report_error(ByteString::formatted(
                        "Result v{} of {} is not instruction-kind",
                        result->index(), opcode_to_string(instruction.opcode())));
                } else if (result->defining_instruction() != &instruction) {
                    report_error(ByteString::formatted(
                        "Result v{} of {} has wrong defining_instruction",
                        result->index(), opcode_to_string(instruction.opcode())));
                }
            }
        });
    }

    // Add parameter, constant, and this values as defined
    for (auto const& value : function.values()) {
        if (value->is_parameter() || value->is_constant() || value->is_this())
            defined_values.set(to_index(value->index()), true);
    }

    for (auto const& block : function.basic_blocks()) {
        bool block_is_reachable = reachable.get(to_index(block->index()));

        // Check: Block parent pointer
        if (block->parent_function() != &function) {
            report_error(ByteString::formatted(
                "Block{} has wrong parent_function",
                block->index()));
        }

        // Check: Predecessor list has no duplicates
        {
            HashTable<BlockIndex> seen_preds;
            for (auto pred : block->predecessor_indices()) {
                if (seen_preds.contains(pred)) {
                    report_error(ByteString::formatted(
                        "Block{} has duplicate predecessor block{}",
                        block->index(), pred));
                }
                seen_preds.set(pred);
            }
        }

        HashTable<BlockIndex> block_predecessor_set;
        for (auto pred : block->predecessor_indices())
            block_predecessor_set.set(pred);

        // Check: No phi nodes in blocks with zero predecessors
        if (block->predecessor_indices().is_empty()) {
            block->for_each_phi([&](PhiInstruction const&) {
                report_error(ByteString::formatted(
                    "Block{} has no predecessors but contains a Phi",
                    block->index()));
            });
        }

        // Check: Block structure invariants
        if (!block->instructions().is_empty()) {
            // Check: "Phis first" - all phi nodes must come before non-phi instructions
            bool seen_non_phi = false;
            block->for_each_instruction([&](Instruction const& instruction) {
                if (instruction.opcode() == Opcode::Phi) {
                    if (seen_non_phi) {
                        report_error(ByteString::formatted(
                            "Block{} has Phi after non-Phi instruction",
                            block->index()));
                    }
                } else {
                    seen_non_phi = true;
                }
            });

            // Check: "Terminator last" - terminator must be last instruction
            for (size_t i = 0; i < block->instructions().size() - 1; ++i) {
                if (function.instruction_by_index(block->instructions()[i])->is_terminator()) {
                    report_error(ByteString::formatted(
                        "Block{} has terminator before last instruction",
                        block->index()));
                    break;
                }
            }

            // Check: Block must end with a terminator
            auto* last_instruction = function.instruction_by_index(block->instructions().last());
            if (!last_instruction->is_terminator()) {
                report_error(ByteString::formatted(
                    "Block{} does not end with a terminator",
                    block->index()));
            }
        } else if (full_mode) {
            // Full mode: no empty blocks
            report_error(ByteString::formatted(
                "Block{} has no instructions",
                block->index()));
        }

        block->for_each_instruction([&](Instruction const& instruction) {
            // Check: Instruction parent pointer
            if (instruction.parent_block() != block.ptr()) {
                report_error(ByteString::formatted(
                    "Instruction in block{} has wrong parent_block (points to block{})",
                    block->index(), instruction.parent_block() ? instruction.parent_block()->index() : -1));
            }

            // Check: Phi operand count == phi predecessor count
            if (instruction.opcode() == Opcode::Phi) {
                auto const& phi = static_cast<PhiInstruction const&>(instruction);
                if (phi.operand_count() != phi.incoming_count()) {
                    report_error(ByteString::formatted(
                        "Phi in block{} has {} operands but {} predecessors",
                        block->index(), phi.operand_count(), phi.incoming_count()));
                }

                // Check: Phi predecessors ⊆ block predecessors
                for (size_t i = 0; i < phi.incoming_count(); ++i) {
                    auto phi_pred = phi.incoming_block(i);
                    if (!block_predecessor_set.contains(phi_pred)) {
                        report_error(ByteString::formatted(
                            "Phi in block{} has predecessor block{} not in block's predecessor list",
                            block->index(), phi_pred));
                    }
                }

                // Check: Block predecessors ⊆ phi predecessors (phi covers all preds)
                for (auto pred : block->predecessor_indices()) {
                    bool found = false;
                    for (size_t i = 0; i < phi.incoming_count(); ++i) {
                        if (phi.incoming_block(i) == pred) {
                            found = true;
                            break;
                        }
                    }
                    if (!found) {
                        report_error(ByteString::formatted(
                            "Phi in block{} missing incoming entry for predecessor block{}",
                            block->index(), pred));
                    }
                }

                // Check: Phi predecessor uniqueness (no duplicate incoming blocks)
                {
                    HashTable<BlockIndex> seen_phi_preds;
                    for (size_t i = 0; i < phi.incoming_count(); ++i) {
                        auto phi_pred = phi.incoming_block(i);
                        if (seen_phi_preds.contains(phi_pred)) {
                            report_error(ByteString::formatted(
                                "Phi in block{} has duplicate predecessor block{}",
                                block->index(), phi_pred));
                        }
                        seen_phi_preds.set(phi_pred);
                    }
                }
            }

            // Check: Opcode operand-arity
            {
                auto arity = opcode_operand_arity(instruction.opcode());
                if (arity != VariableArity && instruction.operand_count() != arity) {
                    report_error(ByteString::formatted(
                        "{} in block{} has {} operands (expected {})",
                        opcode_to_string(instruction.opcode()), block->index(),
                        instruction.operand_count(), arity));
                }
            }

            // Check: Variable-arity operand minimums
            {
                Optional<size_t> minimum_operands;
                switch (instruction.opcode()) {
                case Opcode::Call:
                case Opcode::CallBuiltin:
                case Opcode::CallDirectEval:
                    minimum_operands = 2; // callee + this
                    break;
                case Opcode::CallWithArgumentArray:
                case Opcode::CallDirectEvalWithArgumentArray:
                case Opcode::ConstructWithArgumentArray:
                    minimum_operands = 3; // callee + this + arguments array
                    break;
                case Opcode::Construct:
                case Opcode::SuperCallWithArgumentArray:
                case Opcode::NewClass:
                case Opcode::GetTemplateObject:
                case Opcode::CopyObjectExcludingProperties:
                    minimum_operands = 1;
                    break;
                default:
                    break;
                }
                if (minimum_operands.has_value() && instruction.operand_count() < *minimum_operands) {
                    report_error(ByteString::formatted(
                        "{} in block{} has {} operands (minimum {})",
                        opcode_to_string(instruction.opcode()), block->index(),
                        instruction.operand_count(), *minimum_operands));
                }
            }

            // Check: Result type sanity
            // If a result type is set (not Unknown), verify it matches the opcode's
            // expected output type. This catches type corruption from optimization passes.
            if (instruction.result() && instruction.result()->type() != Type::Unknown) {
                auto actual_type = instruction.result()->type();
                auto expected = opcode_guaranteed_result_type(instruction.opcode());
                if (expected != Type::Unknown && actual_type != expected) {
                    report_error(ByteString::formatted(
                        "{} in block{} has result type {} (expected {})",
                        opcode_to_string(instruction.opcode()), block->index(),
                        type_to_string(actual_type), type_to_string(expected)));
                }
            }

            // Check: Result presence vs. opcode traits
            {
                bool trait_has_result = opcode_has_result(instruction.opcode());
                if (trait_has_result && !instruction.result()) {
                    report_error(ByteString::formatted(
                        "{} in block{} should have a result but doesn't",
                        opcode_to_string(instruction.opcode()), block->index()));
                }
                if (!trait_has_result && instruction.result()) {
                    report_error(ByteString::formatted(
                        "{} in block{} should not have a result but does",
                        opcode_to_string(instruction.opcode()), block->index()));
                }
            }

            // Check: PostSSA - no phi nodes allowed
            if (post_ssa && instruction.opcode() == Opcode::Phi) {
                report_error(ByteString::formatted(
                    "Phi in block{} exists after SSA destruction (stage >= PostSSA)",
                    block->index()));
            }

            // Check: ParallelCopy internal consistency (PostSSA only)
            if (post_ssa && instruction.opcode() == Opcode::ParallelCopy) {
                auto const& pcopy = static_cast<ParallelCopyInstruction const&>(instruction);
                if (pcopy.copies().size() != pcopy.operand_count()) {
                    report_error(ByteString::formatted(
                        "ParallelCopy in block{} has {} copies but {} operands",
                        block->index(), pcopy.copies().size(), pcopy.operand_count()));
                }
                for (size_t i = 0; i < pcopy.copies().size() && i < pcopy.operand_count(); ++i) {
                    if (pcopy.copy_src(i) != pcopy.operand(i)) {
                        report_error(ByteString::formatted(
                            "ParallelCopy in block{} copy_src({}) does not match operand({})",
                            block->index(), i, i));
                    }
                    // Copy destinations must be instruction-kind values
                    auto* dst = pcopy.copy_dst(i);
                    if (dst && !dst->is_instruction()) {
                        report_error(ByteString::formatted(
                            "ParallelCopy in block{} copy_dst({}) v{} is not instruction-kind",
                            block->index(), i, dst->index()));
                    }
                }
            }

            // Check: Builtin enum validity
            if (instruction.opcode() == Opcode::CallBuiltin) {
                if (instruction.builtin() == Bytecode::Builtin::__Count) {
                    report_error(ByteString::formatted(
                        "CallBuiltin in block{} has invalid builtin (__Count)",
                        block->index()));
                }
            }

            // Check: LoadConstant operand must be a constant value
            if (instruction.opcode() == Opcode::LoadConstant && instruction.operand_count() > 0) {
                auto* operand = instruction.operand(0);
                if (operand && !operand->is_constant()) {
                    report_error(ByteString::formatted(
                        "LoadConstant in block{} has non-constant operand v{}",
                        block->index(), operand->index()));
                }
            }

            // Check: AST pointer presence
            if (instruction.opcode() == Opcode::NewFunction && !instruction.function_node()) {
                report_error(ByteString::formatted(
                    "NewFunction in block{} has no function_node",
                    block->index()));
            }
            if (instruction.opcode() == Opcode::NewClass && !instruction.class_expression()) {
                report_error(ByteString::formatted(
                    "NewClass in block{} has no class_expression",
                    block->index()));
            }

            // Check: ExtractValue source must be a tuple-producing instruction
            // and the index must be within bounds
            if (instruction.opcode() == Opcode::ExtractValue && block_is_reachable) {
                auto* source = instruction.operand_count() == 0 ? nullptr : instruction.operand(0);
                if (source && source->is_instruction() && source->defining_instruction()) {
                    auto source_opcode = source->defining_instruction()->opcode();
                    auto tuple_size = opcode_tuple_arity(source_opcode);
                    if (tuple_size == 0) {
                        report_error(ByteString::formatted(
                            "ExtractValue in block{} extracts from non-tuple {} (v{})",
                            block->index(), opcode_to_string(source_opcode), source->index()));
                    } else if (instruction.extract_index() >= tuple_size) {
                        report_error(ByteString::formatted(
                            "ExtractValue in block{} index {} out of bounds (tuple size {})",
                            block->index(), instruction.extract_index(), tuple_size));
                    }
                } else if (source && !source->is_instruction()) {
                    report_error(ByteString::formatted(
                        "ExtractValue in block{} source v{} is not an instruction result",
                        block->index(), source->index()));
                }
            }

            // Operand validity and dominance checks only for reachable blocks.
            // Unreachable blocks may reference values from other unreachable code
            // or have stale references that DeadBlockElimination will clean up.
            if (block_is_reachable) {
                // Check: All operands are non-null and reference defined values
                // NB: NewClass allows null operands (no superclass, optional element keys).
                bool allows_null_operands = instruction.opcode() == Opcode::NewClass;
                for (size_t i = 0; i < instruction.operand_count(); ++i) {
                    auto* op = instruction.operand(i);
                    if (!op) {
                        if (allows_null_operands)
                            continue;
                        report_error(ByteString::formatted(
                            "Instruction in block{} has null operand at index {}",
                            block->index(), i));
                        continue;
                    }
                    if (!defined_values.get(to_index(op->index()))) {
                        report_error(ByteString::formatted(
                            "Instruction in block{} uses undefined value v{}",
                            block->index(), op->index()));
                    }
                    // Check: Instruction-kind values must have a defining instruction
                    // This catches placeholder register values that weren't properly renamed
                    if (op->is_instruction() && !op->defining_instruction()) {
                        report_error(ByteString::formatted(
                            "Instruction in block{} uses v{} which has no defining instruction (likely SSA renaming failure)",
                            block->index(), op->index()));
                    }

                    // Check: Operand-to-use list completeness
                    // The operand's use list must contain this (instruction, slot) pair.
                    Use expected_use { instruction.index(), static_cast<u32>(i) };
                    bool found_in_use_list = false;
                    for (auto const& use : op->uses()) {
                        if (use == expected_use) {
                            found_in_use_list = true;
                            break;
                        }
                    }
                    if (!found_in_use_list) {
                        report_error(ByteString::formatted(
                            "Value v{} use list missing entry for {} in block{} at operand slot {}",
                            op->index(), opcode_to_string(instruction.opcode()),
                            block->index(), i));
                    }
                }
            }
        });

        // Check: Exception handler/finalizer targets exist
        if (block->exception_handler_index().has_value() && !all_blocks.get(to_index(*block->exception_handler_index()))) {
            report_error(ByteString::formatted(
                "Block{} has exception_handler not in function",
                block->index()));
        }
        if (block->finalizer_index().has_value() && !all_blocks.get(to_index(*block->finalizer_index()))) {
            report_error(ByteString::formatted(
                "Block{} has finalizer not in function",
                block->index()));
        }

        // Check: Terminator target shape matches opcode
        if (auto* term = block->terminator()) {
            switch (term->opcode()) {
            case Opcode::Jump:
            case Opcode::ContinuePendingUnwind:
            case Opcode::EnterUnwindContext:
                if (!term->true_target_index().has_value()) {
                    report_error(ByteString::formatted(
                        "{} in block{} has no target",
                        opcode_to_string(term->opcode()), block->index()));
                }
                if (term->false_target_index().has_value()) {
                    report_error(ByteString::formatted(
                        "{} in block{} has false_target (should be null)",
                        opcode_to_string(term->opcode()), block->index()));
                }
                break;
            case Opcode::ScheduleJump:
                if (!term->true_target_index().has_value() || !term->false_target_index().has_value()) {
                    report_error(ByteString::formatted(
                        "ScheduleJump in block{} missing true_target or false_target",
                        block->index()));
                }
                // ScheduleJump's true_target must be the block's finalizer
                if (term->true_target_index().has_value() && block->finalizer_index().has_value()
                    && *term->true_target_index() != *block->finalizer_index()) {
                    report_error(ByteString::formatted(
                        "ScheduleJump in block{} true_target (block{}) does not match finalizer (block{})",
                        block->index(), *term->true_target_index(), *block->finalizer_index()));
                }
                break;
            case Opcode::Branch:
                if (!term->true_target_index().has_value() || !term->false_target_index().has_value()) {
                    report_error(ByteString::formatted(
                        "Branch in block{} missing true_target or false_target",
                        block->index()));
                }
                break;
            case Opcode::Return:
            case Opcode::Throw:
            case Opcode::End:
                if (term->true_target_index().has_value() || term->false_target_index().has_value()) {
                    report_error(ByteString::formatted(
                        "{} in block{} has targets (should have none)",
                        opcode_to_string(term->opcode()), block->index()));
                }
                break;
            case Opcode::Yield:
                if (term->false_target_index().has_value()) {
                    report_error(ByteString::formatted(
                        "Yield in block{} has false_target (should be null)",
                        block->index()));
                }
                break;
            case Opcode::Await:
                if (!term->true_target_index().has_value()) {
                    report_error(ByteString::formatted(
                        "Await in block{} has no continuation target",
                        block->index()));
                }
                if (term->false_target_index().has_value()) {
                    report_error(ByteString::formatted(
                        "Await in block{} has false_target (should be null)",
                        block->index()));
                }
                break;
            default:
                break;
            }
        }

        // Successor/predecessor edge checks only for reachable blocks.
        if (block_is_reachable) {
            if (auto* term = block->terminator()) {
                // Check: All terminator targets exist in function
                if (term->true_target_index().has_value() && !all_blocks.get(to_index(*term->true_target_index()))) {
                    report_error(ByteString::formatted(
                        "Terminator in block{} has true_target not in function",
                        block->index()));
                }
                if (term->false_target_index().has_value() && !all_blocks.get(to_index(*term->false_target_index()))) {
                    report_error(ByteString::formatted(
                        "Terminator in block{} has false_target not in function",
                        block->index()));
                }

                if (auto* true_target = term->true_target()) {
                    if (!true_target->predecessor_indices().contains_slow(block->index())) {
                        report_error(ByteString::formatted(
                            "Block{} has successor block{} but is not in its predecessor list",
                            block->index(), true_target->index()));
                    }
                }
                if (auto* false_target = term->false_target()) {
                    if (!false_target->predecessor_indices().contains_slow(block->index())) {
                        report_error(ByteString::formatted(
                            "Block{} has successor block{} but is not in its predecessor list",
                            block->index(), false_target->index()));
                    }
                }

                // Check: When a block has both an explicit terminator edge and an
                // implicit EH edge to the same target, the target must not have phi
                // nodes. A phi can only hold one entry per predecessor, but the two
                // paths (normal vs exception) may need different values. Intermediate
                // split blocks should keep these paths separate.
                // ScheduleJump is excluded since it intentionally targets the finalizer.
                if (term->opcode() != Opcode::ScheduleJump) {
                    auto check_eh_overlap = [&](BasicBlock* target) {
                        if (!target)
                            return;
                        bool overlaps_eh = (target == block->exception_handler() || target == block->finalizer());
                        if (!overlaps_eh)
                            return;
                        bool target_has_phis = !target->instructions().is_empty()
                            && function.instruction_by_index(target->instructions().first())->opcode() == Opcode::Phi;
                        if (!target_has_phis)
                            return;
                        if (target == block->exception_handler()) {
                            report_error(ByteString::formatted(
                                "Block{} terminator targets block{} which is also its exception handler and has phi nodes",
                                block->index(), target->index()));
                        }
                        if (target == block->finalizer()) {
                            report_error(ByteString::formatted(
                                "Block{} terminator targets block{} which is also its finalizer and has phi nodes",
                                block->index(), target->index()));
                        }
                    };
                    check_eh_overlap(term->true_target());
                    check_eh_overlap(term->false_target());
                }

                // Check: No critical edges in PostSSA
                // A critical edge goes from a multi-successor block to a multi-predecessor block.
                if (post_ssa && term->true_target_index().has_value() && term->false_target_index().has_value()) {
                    auto check_critical = [&](BasicBlock* target) {
                        if (target && target->predecessor_indices().size() > 1) {
                            report_error(ByteString::formatted(
                                "Critical edge from block{} to block{} exists after SSA destruction",
                                block->index(), target->index()));
                        }
                    };
                    check_critical(term->true_target());
                    check_critical(term->false_target());
                }
            }
        }
    }

    // Check: Recompute CFG predecessors from successor edges of reachable blocks
    // This catches desync between terminator targets and predecessor lists
    {
        Vector<HashTable<BasicBlock*>> computed_preds;
        computed_preds.resize(block_capacity);

        for (auto const& block : function.basic_blocks()) {
            if (!reachable.get(to_index(block->index())))
                continue;
            CFG::for_each_successor(*block, [&](BasicBlock& target) {
                computed_preds[to_index(target.index())].set(block.ptr());
            });
        }

        for (auto const& block : function.basic_blocks()) {
            if (!reachable.get(to_index(block->index())))
                continue;
            auto const& stored_preds = block->predecessor_indices();
            auto& expected_preds = computed_preds[to_index(block->index())];

            // Check: Every stored predecessor should be a computed predecessor.
            // In InterPass mode, skip unreachable predecessors since they may
            // have stale edges that DeadBlockElimination will clean up.
            for (auto pred : stored_preds) {
                if (!full_mode && !reachable.get(to_index(pred)))
                    continue;
                if (!expected_preds.contains(function.block_by_index(pred))) {
                    report_error(ByteString::formatted(
                        "Block{} has predecessor block{} in stored list but no edge exists",
                        block->index(), pred));
                }
            }

            // Check: Every computed predecessor should be a stored predecessor
            for (auto* pred : expected_preds) {
                if (!stored_preds.contains_slow(pred->index())) {
                    report_error(ByteString::formatted(
                        "Block{} is missing predecessor block{} (has edge but not in list)",
                        block->index(), pred->index()));
                }
            }
        }
    }

    // Check: Value index sanity — values()[i]->index() == i
    for (size_t i = 0; i < function.values().size(); ++i) {
        if (to_index(function.values()[i]->index()) != i) {
            report_error(ByteString::formatted(
                "values()[{}] has index v{} (expected v{})",
                i, function.values()[i]->index(), i));
        }
    }

    // Check: Use lists should only contain instructions still present in the function
    // (catches stale references from dead block elimination or other passes)
    HashTable<Instruction const*> all_instructions;
    for (auto const& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const& instruction) {
            all_instructions.set(&instruction);
        });
    }

    for (auto const& value : function.values()) {
        // Check: Constant type sanity
        // Constants must not have Type::Unknown, and their IR::Type must match the JS::Value.
        if (value->is_constant()) {
            auto ir_type = value->type();
            auto const& cv = value->constant_value();
            if (ir_type == Type::Unknown && !cv.is_special_empty_value()) {
                report_error(ByteString::formatted(
                    "Constant v{} has Type::Unknown",
                    value->index()));
            } else {
                auto expected_type = [&]() -> Optional<Type> {
                    if (cv.is_undefined())
                        return Type::Undefined;
                    if (cv.is_null())
                        return Type::Null;
                    if (cv.is_boolean())
                        return Type::Boolean;
                    if (cv.is_int32())
                        return Type::Int32;
                    if (cv.is_number())
                        return Type::Number;
                    if (cv.is_string())
                        return Type::String;
                    if (cv.is_symbol())
                        return Type::Symbol;
                    if (cv.is_bigint())
                        return Type::BigInt;
                    if (cv.is_object())
                        return Type::Object;
                    return {};
                }();
                if (expected_type.has_value() && ir_type != *expected_type) {
                    report_error(ByteString::formatted(
                        "Constant v{} has type {} but JS::Value implies {}",
                        value->index(), type_to_string(ir_type), type_to_string(*expected_type)));
                }
            }
        }

        // Check: Defining-instruction/result round-trip
        if (value->is_instruction() && value->defining_instruction()) {
            if (value->defining_instruction()->result() != value.ptr()) {
                report_error(ByteString::formatted(
                    "Value v{} has defining_instruction but that instruction's result is not this value",
                    value->index()));
            }
        }

        // Check: Value kind consistency
        // Constants, parameters, and this values must NOT have a defining instruction
        if (!value->is_instruction() && value->defining_instruction()) {
            char const* kind_name = "this";
            if (value->is_constant())
                kind_name = "constant";
            else if (value->is_parameter())
                kind_name = "parameter";
            report_error(ByteString::formatted(
                "Value v{} (kind={}) has a defining instruction but shouldn't",
                value->index(), kind_name));
        }

        for (auto const& use : value->uses()) {
            auto* using_instruction = function.instruction_by_index(use.instruction);
            if (!all_instructions.contains(using_instruction)) {
                report_error(ByteString::formatted(
                    "Value v{} has stale use pointing to removed instruction",
                    value->index()));
                continue;
            }
            // Check: Use-list slot bounds
            if (use.operand_slot >= using_instruction->operand_count()) {
                report_error(ByteString::formatted(
                    "Value v{} has use with slot {} but instruction only has {} operands",
                    value->index(), use.operand_slot, using_instruction->operand_count()));
                continue;
            }
            // Check: Reverse validation - the using instruction must actually
            // have this value in its operand list at the specified slot
            if (using_instruction->operand(use.operand_slot) != value.ptr()) {
                report_error(ByteString::formatted(
                    "Value v{} has use at slot {} but instruction does not reference it there",
                    value->index(), use.operand_slot));
            }
        }

        // Check: Use-list uniqueness
        // Each (instruction, slot) pair must appear at most once. An instruction
        // that uses the same value in multiple slots (e.g., Add v0, v0) will have
        // separate entries with different operand_slot values.
        for (size_t i = 0; i < value->uses().size(); ++i) {
            for (size_t j = i + 1; j < value->uses().size(); ++j) {
                if (value->uses()[i] == value->uses()[j]) {
                    report_error(ByteString::formatted(
                        "Value v{} has duplicate use-list entry (instruction {}, slot {})",
                        value->index(), value->uses()[i].instruction, value->uses()[i].operand_slot));
                    break;
                }
            }
        }

        // Check: Tuple values must only be used by ExtractValue
        if (value->is_instruction() && value->defining_instruction()) {
            auto tuple_arity = opcode_tuple_arity(value->defining_instruction()->opcode());
            if (tuple_arity > 0) {
                for (auto const& use : value->uses()) {
                    auto* using_instruction = function.instruction_by_index(use.instruction);
                    if (!all_instructions.contains(using_instruction))
                        continue;
                    if (using_instruction->opcode() != Opcode::ExtractValue) {
                        report_error(ByteString::formatted(
                            "Tuple value v{} (from {}) used by non-ExtractValue instruction {}",
                            value->index(),
                            opcode_to_string(value->defining_instruction()->opcode()),
                            opcode_to_string(using_instruction->opcode())));
                    }
                }
            }
        }
    }

    // SSA dominance verification (only for reachable blocks)
    // This check implicitly verifies EH correctness: if exception handler blocks
    // reference values that were defined after a throw point (due to incorrect EH
    // splitting), those values won't dominate the handler and we'll report an error.
    //
    // Build map from Value index to its defining block
    Vector<BasicBlock*> value_to_block;
    value_to_block.resize_with_default_value(value_capacity, nullptr);
    for (auto const& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.result())
                value_to_block[to_index(instruction.result()->index())] = block.ptr();
        });
    }

    // Compute dominators for dominance checking
    DominatorTree dominators(function);

    // Precompute instruction position within each block for O(1) same-block ordering checks
    HashMap<Instruction const*, size_t> instruction_position;
    for (auto const& block : function.basic_blocks()) {
        size_t position = 0;
        block->for_each_instruction([&](Instruction const& instruction) {
            instruction_position.set(&instruction, position++);
        });
    }

    for (auto const& block : function.basic_blocks()) {
        if (!reachable.get(to_index(block->index())))
            continue;

        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() == Opcode::Phi) {
                // For phi instructions, each operand must be reachable from its corresponding predecessor
                // The defining block must dominate the predecessor (not the current block)
                auto const& phi = static_cast<PhiInstruction const&>(instruction);
                for (size_t i = 0; i < phi.incoming_count(); ++i) {
                    auto* operand = phi.incoming_value(i);
                    auto pred_block_index = phi.incoming_block(i);
                    auto* pred = function.block_by_index(pred_block_index);
                    if (!operand)
                        continue;

                    // In InterPass mode, skip unreachable predecessors — they may have
                    // stale phi entries that DeadBlockElimination will clean up.
                    if (!full_mode && !reachable.get(to_index(pred_block_index)))
                        continue;

                    // Skip constants, parameters, and this values - they dominate everything
                    if (operand->is_constant() || operand->is_parameter() || operand->is_this())
                        continue;

                    auto* def_block = value_to_block[to_index(operand->index())];
                    if (!def_block) {
                        report_error(ByteString::formatted(
                            "Phi operand v{} in block{} has no defining block",
                            operand->index(), block->index()));
                        continue;
                    }

                    // The definition must dominate the predecessor block
                    // (the value flows from pred -> current block via the phi)
                    if (!dominators.dominates(def_block, pred)) {
                        report_error(ByteString::formatted(
                            "SSA violation: phi operand v{} (defined in block{}) does not dominate predecessor block{} for phi in block{}",
                            operand->index(), def_block->index(), pred->index(), block->index()));
                    }
                }
            } else {
                // For non-phi instructions, each operand's definition must dominate this block
                for (size_t i = 0; i < instruction.operand_count(); ++i) {
                    auto* operand = instruction.operand(i);
                    if (!operand)
                        continue;

                    // Skip constants, parameters, and this values - they dominate everything
                    if (operand->is_constant() || operand->is_parameter() || operand->is_this())
                        continue;

                    auto* def_block = value_to_block[to_index(operand->index())];
                    if (!def_block) {
                        report_error(ByteString::formatted(
                            "Operand v{} in block{} has no defining block",
                            operand->index(), block->index()));
                        continue;
                    }

                    // Check dominance: def_block must dominate use_block
                    // If in same block, check instruction order
                    if (def_block == block.ptr()) {
                        // Same block: definition must come before use
                        auto* defining_instruction = operand->defining_instruction();
                        if (defining_instruction) {
                            auto def_pos = instruction_position.get(defining_instruction);
                            auto use_pos = instruction_position.get(&instruction);
                            if (def_pos.has_value() && use_pos.has_value() && *def_pos >= *use_pos) {
                                report_error(ByteString::formatted(
                                    "SSA violation: operand v{} used before definition in block{}",
                                    operand->index(), block->index()));
                            }
                        }
                    } else if (!dominators.dominates(def_block, block.ptr())) {
                        report_error(ByteString::formatted(
                            "SSA violation: operand v{} (defined in block{}) does not dominate use in block{}",
                            operand->index(), def_block->index(), block->index()));
                    }
                }
            }
        });
    }

    return valid;
}

}
