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
    auto block_capacity = block_index_capacity(function);
    auto value_capacity = function.values().size();

    // Build set of all blocks for quick lookup
    auto all_blocks = MUST(AK::Bitmap::create(block_capacity, false));
    for (auto const& block : function.basic_blocks())
        all_blocks.set(to_index(block->index()), true);

    // Check: Entry block has no predecessors
    if (function.entry_block() && !function.entry_block()->predecessors().is_empty()) {
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

    // Build set of all defined values, checking for unique definitions
    auto defined_values = MUST(AK::Bitmap::create(value_capacity, false));
    for (auto const& block : function.basic_blocks()) {
        for (auto const& instr : block->instructions()) {
            if (instr->result()) {
                auto vi = to_index(instr->result()->index());
                // Check: No two instructions share the same result Value*
                if (defined_values.get(vi)) {
                    report_error(ByteString::formatted(
                        "Value v{} is defined by multiple instructions",
                        instr->result()->index()));
                }
                defined_values.set(vi, true);
            }
        }
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
        } else if (full_mode) {
            // Full mode: no empty blocks
            report_error(ByteString::formatted(
                "Block{} has no instructions",
                block->index()));
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
                auto const& phi = static_cast<PhiInstruction const&>(*instr);
                if (phi.operands().size() != phi.incoming_count()) {
                    report_error(ByteString::formatted(
                        "Phi in block{} has {} operands but {} predecessors",
                        block->index(), phi.operands().size(), phi.incoming_count()));
                }

                // Check: Phi predecessors ⊆ block predecessors
                for (size_t i = 0; i < phi.incoming_count(); ++i) {
                    auto* phi_pred = phi.incoming_block(i);
                    if (!block_predecessor_set.contains(phi_pred)) {
                        report_error(ByteString::formatted(
                            "Phi in block{} has predecessor block{} not in block's predecessor list",
                            block->index(), phi_pred->index()));
                    }
                }

                // Check: Block predecessors ⊆ phi predecessors (phi covers all preds)
                for (auto* pred : block->predecessors()) {
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
                            block->index(), pred->index()));
                    }
                }

                // Check: Phi predecessor uniqueness (no duplicate incoming blocks)
                {
                    HashTable<BasicBlock*> seen_phi_preds;
                    for (size_t i = 0; i < phi.incoming_count(); ++i) {
                        auto* phi_pred = phi.incoming_block(i);
                        if (seen_phi_preds.contains(phi_pred)) {
                            report_error(ByteString::formatted(
                                "Phi in block{} has duplicate predecessor block{}",
                                block->index(), phi_pred->index()));
                        }
                        seen_phi_preds.set(phi_pred);
                    }
                }
            }

            // Check: Opcode operand-arity
            {
                auto arity = opcode_operand_arity(instr->opcode());
                if (arity != VariableArity && instr->operands().size() != arity) {
                    report_error(ByteString::formatted(
                        "{} in block{} has {} operands (expected {})",
                        opcode_to_string(instr->opcode()), block->index(),
                        instr->operands().size(), arity));
                }
            }

            // Check: Result type sanity
            // If a result type is set (not Unknown), verify it matches the opcode's
            // expected output type. This catches type corruption from optimization passes.
            if (instr->result() && instr->result()->type() != Type::Unknown) {
                auto actual_type = instr->result()->type();
                auto expected = opcode_guaranteed_result_type(instr->opcode());
                if (expected != Type::Unknown && actual_type != expected) {
                    report_error(ByteString::formatted(
                        "{} in block{} has result type {} (expected {})",
                        opcode_to_string(instr->opcode()), block->index(),
                        type_to_string(actual_type), type_to_string(expected)));
                }
            }

            // Check: Result presence vs. opcode traits
            {
                bool trait_has_result = opcode_has_result(instr->opcode());
                if (trait_has_result && !instr->result()) {
                    report_error(ByteString::formatted(
                        "{} in block{} should have a result but doesn't",
                        opcode_to_string(instr->opcode()), block->index()));
                }
                if (!trait_has_result && instr->result()) {
                    report_error(ByteString::formatted(
                        "{} in block{} should not have a result but does",
                        opcode_to_string(instr->opcode()), block->index()));
                }
            }

            // Check: ExtractValue source must be a tuple-producing instruction
            // and the index must be within bounds
            if (instr->opcode() == Opcode::ExtractValue && block_is_reachable) {
                auto* source = instr->operands().is_empty() ? nullptr : instr->operands()[0];
                if (source && source->is_instruction() && source->defining_instruction()) {
                    auto source_opcode = source->defining_instruction()->opcode();
                    Optional<u32> tuple_size;
                    switch (source_opcode) {
                    case Opcode::GetCalleeAndThisFromEnvironment:
                    case Opcode::GetCompletionFields:
                    case Opcode::IteratorNextUnpack:
                        tuple_size = 2;
                        break;
                    case Opcode::GetIterator:
                    case Opcode::GetObjectPropertyIterator:
                        tuple_size = 3;
                        break;
                    default:
                        report_error(ByteString::formatted(
                            "ExtractValue in block{} extracts from non-tuple {} (v{})",
                            block->index(), opcode_to_string(source_opcode), source->index()));
                        break;
                    }
                    if (tuple_size.has_value() && instr->extract_index() >= *tuple_size) {
                        report_error(ByteString::formatted(
                            "ExtractValue in block{} index {} out of bounds (tuple size {})",
                            block->index(), instr->extract_index(), *tuple_size));
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
                bool allows_null_operands = instr->opcode() == Opcode::NewClass;
                for (size_t i = 0; i < instr->operands().size(); ++i) {
                    auto* operand = instr->operands()[i];
                    if (!operand) {
                        if (allows_null_operands)
                            continue;
                        report_error(ByteString::formatted(
                            "Instruction in block{} has null operand at index {}",
                            block->index(), i));
                        continue;
                    }
                    if (!defined_values.get(to_index(operand->index()))) {
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
        }

        // Check: Exception handler/finalizer targets exist
        if (block->exception_handler() && !all_blocks.get(to_index(block->exception_handler()->index()))) {
            report_error(ByteString::formatted(
                "Block{} has exception_handler not in function",
                block->index()));
        }
        if (block->finalizer() && !all_blocks.get(to_index(block->finalizer()->index()))) {
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
                if (!term->true_target()) {
                    report_error(ByteString::formatted(
                        "{} in block{} has no target",
                        opcode_to_string(term->opcode()), block->index()));
                }
                if (term->false_target()) {
                    report_error(ByteString::formatted(
                        "{} in block{} has false_target (should be null)",
                        opcode_to_string(term->opcode()), block->index()));
                }
                break;
            case Opcode::ScheduleJump:
                if (!term->true_target() || !term->false_target()) {
                    report_error(ByteString::formatted(
                        "ScheduleJump in block{} missing true_target or false_target",
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
                if (term->false_target()) {
                    report_error(ByteString::formatted(
                        "Yield in block{} has false_target (should be null)",
                        block->index()));
                }
                break;
            case Opcode::Await:
                if (!term->true_target()) {
                    report_error(ByteString::formatted(
                        "Await in block{} has no continuation target",
                        block->index()));
                }
                if (term->false_target()) {
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
                if (term->true_target() && !all_blocks.get(to_index(term->true_target()->index()))) {
                    report_error(ByteString::formatted(
                        "Terminator in block{} has true_target not in function",
                        block->index()));
                }
                if (term->false_target() && !all_blocks.get(to_index(term->false_target()->index()))) {
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
            auto const& stored_preds = block->predecessors();
            auto& expected_preds = computed_preds[to_index(block->index())];

            // Check: Every stored predecessor should be a computed predecessor.
            // In InterPass mode, skip unreachable predecessors since they may
            // have stale edges that DeadBlockElimination will clean up.
            for (auto* pred : stored_preds) {
                if (!full_mode && !reachable.get(to_index(pred->index())))
                    continue;
                if (!expected_preds.contains(pred)) {
                    report_error(ByteString::formatted(
                        "Block{} has predecessor block{} in stored list but no edge exists",
                        block->index(), pred->index()));
                }
            }

            // Check: Every computed predecessor should be a stored predecessor
            for (auto* pred : expected_preds) {
                bool found = stored_preds.contains_slow(pred);
                if (!found) {
                    report_error(ByteString::formatted(
                        "Block{} is missing predecessor block{} (has edge but not in list)",
                        block->index(), pred->index()));
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

        for (auto const* use : value->uses()) {
            if (!all_instructions.contains(use)) {
                report_error(ByteString::formatted(
                    "Value v{} has stale use pointing to removed instruction",
                    value->index()));
                continue;
            }
            // Check: Reverse validation - the using instruction must actually
            // have this value in its operand list
            bool found_in_operands = false;
            for (auto* operand : use->operands()) {
                if (operand == value.ptr()) {
                    found_in_operands = true;
                    break;
                }
            }
            if (!found_in_operands) {
                report_error(ByteString::formatted(
                    "Value v{} has use in instruction but is not in its operand list",
                    value->index()));
            }
        }

        // NB: Use-list entries are NOT unique per instruction. An instruction that
        // uses the same value in multiple operand positions (e.g., Add v0, v0) will
        // appear multiple times in the use list — once per operand reference.
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
        for (auto const& instr : block->instructions()) {
            if (instr->result())
                value_to_block[to_index(instr->result()->index())] = block.ptr();
        }
    }

    // Compute dominators for dominance checking
    DominatorTree dominators(function);

    // Precompute instruction position within each block for O(1) same-block ordering checks
    HashMap<Instruction const*, size_t> instruction_position;
    for (auto const& block : function.basic_blocks()) {
        size_t position = 0;
        for (auto const& instr : block->instructions())
            instruction_position.set(instr.ptr(), position++);
    }

    for (auto const& block : function.basic_blocks()) {
        if (!reachable.get(to_index(block->index())))
            continue;

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

                    // In InterPass mode, skip unreachable predecessors — they may have
                    // stale phi entries that DeadBlockElimination will clean up.
                    if (!full_mode && !reachable.get(to_index(pred->index())))
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
                for (size_t i = 0; i < instr->operands().size(); ++i) {
                    auto* operand = instr->operands()[i];
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
                        auto* def_instr = operand->defining_instruction();
                        if (def_instr) {
                            auto def_pos = instruction_position.get(def_instr);
                            auto use_pos = instruction_position.get(instr.ptr());
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
        }
    }

    return valid;
}

}
