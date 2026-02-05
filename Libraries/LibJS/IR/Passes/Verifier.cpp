/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Dominators.h>
#include <LibJS/IR/Dump.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Passes/Verifier.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

bool Verifier::run(Function& function)
{
    verify(function, VerifierMode::InterPass, true);
    return false; // Verifier never modifies the IR
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

    // Build set of all blocks for quick lookup
    HashTable<BasicBlock*> all_blocks;
    for (auto const& block : function.basic_blocks())
        all_blocks.set(block.ptr());

    // Check: Entry block has no predecessors
    if (function.entry_block() && !function.entry_block()->predecessors().is_empty()) {
        report_error("Entry block has predecessors"sv);
    }

    // Compute reachable blocks via BFS (including EH/finalizer edges)
    HashTable<BasicBlock*> reachable;
    if (auto* entry = function.entry_block()) {
        Vector<BasicBlock*> worklist;
        worklist.append(entry);
        reachable.set(entry);
        while (!worklist.is_empty()) {
            auto* current = worklist.take_last();
            auto enqueue = [&](BasicBlock* target) {
                if (target && !reachable.contains(target)) {
                    reachable.set(target);
                    worklist.append(target);
                }
            };
            if (auto* term = current->terminator()) {
                enqueue(term->true_target());
                enqueue(term->false_target());
            }
            enqueue(current->exception_handler());
            enqueue(current->finalizer());
        }
    }

    // Full mode: no unreachable blocks allowed
    if (full_mode) {
        for (auto const& block : function.basic_blocks()) {
            if (!reachable.contains(block.ptr())) {
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
    HashTable<Value const*> defined_values;
    for (auto const& block : function.basic_blocks()) {
        for (auto const& instr : block->instructions()) {
            if (instr->result()) {
                // Check: No two instructions share the same result Value*
                if (defined_values.contains(instr->result())) {
                    report_error(ByteString::formatted(
                        "Value v{} is defined by multiple instructions",
                        instr->result()->index()));
                }
                defined_values.set(instr->result());
            }
        }
    }

    // Add parameter, constant, and this values as defined
    for (auto const& value : function.values()) {
        if (value->is_parameter() || value->is_constant() || value->is_this())
            defined_values.set(value.ptr());
    }

    for (auto const& block : function.basic_blocks()) {
        bool block_is_reachable = reachable.contains(block.ptr());

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
            // NB: Phi and Call have variable arity, so we only check fixed-arity opcodes.
            // Metadata stored via set_identifier_index/set_property_key_index/etc. does NOT
            // count as an operand — only values passed via add_operand() count.
            {
                auto expected_arity = [](Opcode opcode) -> Optional<size_t> {
                    switch (opcode) {
                    // 0 operands: no value operands (all data is metadata or implicit)
                    case Opcode::Jump:
                    case Opcode::LoadUndefined:
                    case Opcode::LoadNull:
                    case Opcode::NewObject:
                    case Opcode::NewRegExp:
                    case Opcode::CreateLexicalEnvironment:
                    case Opcode::LeaveLexicalEnvironment:
                    case Opcode::CreatePrivateEnvironment:
                    case Opcode::LeavePrivateEnvironment:
                    case Opcode::CreateVariableEnvironment:
                    case Opcode::GetNewTarget:
                    case Opcode::ResolveThisBinding:
                    case Opcode::ResolveSuperBase:
                    case Opcode::GetGlobal:
                    case Opcode::GetBinding:
                    case Opcode::TypeofBinding:
                    case Opcode::DeleteVariable:
                    case Opcode::GetCalleeAndThisFromEnvironment:
                    case Opcode::AddPrivateName:
                    case Opcode::CreateArguments:
                    case Opcode::CreateRestParams:
                        return 0;
                    // 1 operand
                    case Opcode::LoadConstant:
                    case Opcode::Return:
                    case Opcode::End:
                    case Opcode::Throw:
                    case Opcode::Branch:
                    case Opcode::Negate:
                    case Opcode::UnaryPlus:
                    case Opcode::BitwiseNot:
                    case Opcode::Typeof:
                    case Opcode::ToBoolean:
                    case Opcode::ToNumber:
                    case Opcode::ToString:
                    case Opcode::ToObject:
                    case Opcode::ToInt32:
                    case Opcode::ToLength:
                    case Opcode::Not:
                    case Opcode::IsUndefined:
                    case Opcode::IsNullish:
                    case Opcode::Increment:
                    case Opcode::Decrement:
                    case Opcode::PostfixIncrement:
                    case Opcode::PostfixDecrement:
                    case Opcode::GetById:
                    case Opcode::GetLength:
                    case Opcode::GetPrivateById:
                    case Opcode::DeleteById:
                    case Opcode::GetIterator:
                    case Opcode::GetObjectPropertyIterator:
                    case Opcode::Yield:
                    case Opcode::Await:
                    case Opcode::Move:
                    case Opcode::ThrowIfNotObject:
                    case Opcode::ThrowIfNullish:
                    case Opcode::ThrowIfTDZ:
                    case Opcode::EnterObjectEnvironment:
                    case Opcode::ExtractValue:
                    case Opcode::CacheObjectShape:
                    case Opcode::NewArrayWithLength:
                    case Opcode::SetGlobal:
                    case Opcode::SetBinding:
                    case Opcode::InitializeBinding:
                    case Opcode::GetCompletionFields:
                    case Opcode::CreateMutableBinding:
                    case Opcode::CreateImmutableBinding:
                        return 1;
                    // 2 operands
                    case Opcode::Add:
                    case Opcode::Sub:
                    case Opcode::Mul:
                    case Opcode::Div:
                    case Opcode::Mod:
                    case Opcode::Exp:
                    case Opcode::BitwiseAnd:
                    case Opcode::BitwiseOr:
                    case Opcode::BitwiseXor:
                    case Opcode::LeftShift:
                    case Opcode::RightShift:
                    case Opcode::UnsignedRightShift:
                    case Opcode::LessThan:
                    case Opcode::LessThanEquals:
                    case Opcode::GreaterThan:
                    case Opcode::GreaterThanEquals:
                    case Opcode::LooselyEquals:
                    case Opcode::StrictlyEquals:
                    case Opcode::LooselyInequals:
                    case Opcode::StrictlyInequals:
                    case Opcode::In:
                    case Opcode::InstanceOf:
                    case Opcode::GetByValue:
                    case Opcode::GetByIdWithThis:
                    case Opcode::DeleteByValue:
                    case Opcode::HasProperty:
                    case Opcode::PutById:
                    case Opcode::PutPrivateById:
                    case Opcode::PutGetterById:
                    case Opcode::PutSetterById:
                    case Opcode::PutPrototypeById:
                    case Opcode::PutBySpread:
                    case Opcode::ConcatString:
                    case Opcode::ArrayAppend:
                    case Opcode::ImportCall:
                    case Opcode::InitObjectLiteralProperty:
                        return 2;
                    // 3 operands
                    case Opcode::IteratorNext:
                    case Opcode::IteratorNextUnpack:
                    case Opcode::IteratorClose:
                    case Opcode::IteratorToArray:
                    case Opcode::PutByValue:
                    case Opcode::GetByValueWithThis:
                    case Opcode::PutGetterByValue:
                    case Opcode::PutSetterByValue:
                    case Opcode::PutPrototypeByValue:
                    case Opcode::PutGetterByIdWithThis:
                    case Opcode::PutSetterByIdWithThis:
                    case Opcode::PutPrototypeByIdWithThis:
                        return 3;
                    // 4 operands
                    case Opcode::PutGetterByValueWithThis:
                    case Opcode::PutSetterByValueWithThis:
                    case Opcode::PutPrototypeByValueWithThis:
                        return 4;
                    // Variable arity
                    case Opcode::Phi:
                    case Opcode::Call:
                    case Opcode::CallBuiltin:
                    case Opcode::CallDirectEval:
                    case Opcode::CallWithArgumentArray:
                    case Opcode::Construct:
                    case Opcode::ConstructWithArgumentArray:
                    case Opcode::SuperCallWithArgumentArray:
                    case Opcode::NewArray:
                    case Opcode::NewClass:
                    case Opcode::NewFunction:
                    case Opcode::CreateVariable:
                        return {};
                    case Opcode::__Count:
                        VERIFY_NOT_REACHED();
                    }
                    VERIFY_NOT_REACHED();
                };
                auto arity = expected_arity(instr->opcode());
                if (arity.has_value() && instr->operands().size() != *arity) {
                    report_error(ByteString::formatted(
                        "{} in block{} has {} operands (expected {})",
                        opcode_to_string(instr->opcode()), block->index(),
                        instr->operands().size(), *arity));
                }
            }

            // Check: Result type sanity
            // If a result type is set (not Unknown), verify it matches the opcode's
            // expected output type. This catches type corruption from optimization passes.
            if (instr->result() && instr->result()->type() != Type::Unknown) {
                auto actual_type = instr->result()->type();
                auto expected_type = [](Opcode opcode) -> Optional<Type> {
                    switch (opcode) {
                    // Always Boolean
                    case Opcode::ToBoolean:
                    case Opcode::Not:
                    case Opcode::IsUndefined:
                    case Opcode::IsNullish:
                    case Opcode::LessThan:
                    case Opcode::LessThanEquals:
                    case Opcode::GreaterThan:
                    case Opcode::GreaterThanEquals:
                    case Opcode::LooselyEquals:
                    case Opcode::StrictlyEquals:
                    case Opcode::LooselyInequals:
                    case Opcode::StrictlyInequals:
                    case Opcode::In:
                    case Opcode::InstanceOf:
                    case Opcode::HasProperty:
                    case Opcode::DeleteById:
                    case Opcode::DeleteByValue:
                    case Opcode::DeleteVariable:
                        return Type::Boolean;
                    // Always Int32
                    case Opcode::BitwiseAnd:
                    case Opcode::BitwiseOr:
                    case Opcode::BitwiseXor:
                    case Opcode::LeftShift:
                    case Opcode::RightShift:
                    case Opcode::BitwiseNot:
                    case Opcode::ToInt32:
                        return Type::Int32;
                    // Always String
                    case Opcode::Typeof:
                    case Opcode::TypeofBinding:
                    case Opcode::ToString:
                    case Opcode::ConcatString:
                        return Type::String;
                    // Always Number
                    case Opcode::UnsignedRightShift:
                    case Opcode::ToNumber:
                    case Opcode::UnaryPlus:
                    case Opcode::Negate:
                    case Opcode::Increment:
                    case Opcode::Decrement:
                    case Opcode::PostfixIncrement:
                    case Opcode::PostfixDecrement:
                        return Type::Number;
                    // Always Undefined
                    case Opcode::LoadUndefined:
                        return Type::Undefined;
                    // Always Null
                    case Opcode::LoadNull:
                        return Type::Null;
                    // Always Object
                    case Opcode::NewObject:
                    case Opcode::NewRegExp:
                    case Opcode::ToObject:
                        return Type::Object;
                    // Always Array
                    case Opcode::NewArray:
                    case Opcode::NewArrayWithLength:
                    case Opcode::IteratorToArray:
                        return Type::Array;
                    // Always Function
                    case Opcode::NewClass:
                    case Opcode::NewFunction:
                        return Type::Function;
                    default:
                        return {};
                    }
                }(instr->opcode());
                if (expected_type.has_value() && actual_type != *expected_type) {
                    report_error(ByteString::formatted(
                        "{} in block{} has result type {} (expected {})",
                        opcode_to_string(instr->opcode()), block->index(),
                        type_to_string(actual_type), type_to_string(*expected_type)));
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
            if (instr->opcode() == Opcode::ExtractValue) {
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
        }

        // Check: Exception handler/finalizer targets exist
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

            // Check: Terminator operand arity
            Optional<size_t> expected_operands;
            switch (term->opcode()) {
            case Opcode::Jump:
                expected_operands = 0;
                break;
            case Opcode::Branch:
            case Opcode::Return:
            case Opcode::Throw:
            case Opcode::End:
            case Opcode::Yield:
            case Opcode::Await:
                expected_operands = 1;
                break;
            default:
                break;
            }
            if (expected_operands.has_value() && term->operands().size() != *expected_operands) {
                report_error(ByteString::formatted(
                    "{} in block{} has {} operands, expected {}",
                    opcode_to_string(term->opcode()), block->index(),
                    term->operands().size(), *expected_operands));
            }
        }

        // Successor/predecessor edge checks only for reachable blocks.
        if (block_is_reachable) {
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
    }

    // Check: Recompute CFG predecessors from successor edges of reachable blocks
    // This catches desync between terminator targets and predecessor lists
    {
        HashMap<BasicBlock*, HashTable<BasicBlock*>> computed_preds;
        for (auto const& block : function.basic_blocks())
            computed_preds.set(block.ptr(), {});

        for (auto const& block : function.basic_blocks()) {
            if (!reachable.contains(block.ptr()))
                continue;
            if (auto* term = block->terminator()) {
                if (auto* target = term->true_target())
                    computed_preds.get(target)->set(block.ptr());
                if (auto* target = term->false_target())
                    computed_preds.get(target)->set(block.ptr());
            }
        }

        for (auto const& block : function.basic_blocks()) {
            if (!reachable.contains(block.ptr()))
                continue;
            auto const& stored_preds = block->predecessors();
            auto& expected_preds = *computed_preds.get(block.ptr());

            // Check: Every stored predecessor should be a computed predecessor.
            // In InterPass mode, skip unreachable predecessors since they may
            // have stale edges that DeadBlockElimination will clean up.
            for (auto* pred : stored_preds) {
                if (!full_mode && !reachable.contains(pred))
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

    // Precompute instruction position within each block for O(1) same-block ordering checks
    HashMap<Instruction const*, size_t> instruction_position;
    for (auto const& block : function.basic_blocks()) {
        size_t position = 0;
        for (auto const& instr : block->instructions())
            instruction_position.set(instr.ptr(), position++);
    }

    for (auto const& block : function.basic_blocks()) {
        if (!reachable.contains(block.ptr()))
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
                    if (!full_mode && !reachable.contains(pred))
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
