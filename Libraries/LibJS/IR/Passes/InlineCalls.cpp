/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Builder.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Lifter.h>
#include <LibJS/IR/Optimize.h>
#include <LibJS/IR/Passes/InlineCalls.h>
#include <LibJS/IR/Passes/PassManager.h>
#include <LibJS/IR/Passes/SCCP.h>
#include <LibJS/IR/Passes/SSAConstructionPass.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/ECMAScriptFunctionObject.h>

namespace JS::IR {

struct InlineCandidate {
    CallInstruction* call;
    BasicBlock* call_block;
    GC::Ref<ECMAScriptFunctionObject> callee;
    NonnullOwnPtr<Function> callee_function;
};

static bool has_unsupported_opcode(Function const& function)
{
    for (auto& block : function.basic_blocks()) {
        bool found = false;
        block->for_each_instruction([&](Instruction const& instruction) {
            switch (instruction.opcode()) {
            // Environment ops
            case Opcode::CreateLexicalEnvironment:
            case Opcode::LeaveLexicalEnvironment:
            case Opcode::CreateVariable:
            case Opcode::GetBinding:
            case Opcode::SetBinding:
            case Opcode::InitializeBinding:
            case Opcode::EnterObjectEnvironment:
            case Opcode::CreatePrivateEnvironment:
            case Opcode::LeavePrivateEnvironment:
            case Opcode::CreateVariableEnvironment:
            // Exception handling ops
            case Opcode::EnterUnwindContext:
            case Opcode::LeaveUnwindContext:
            case Opcode::Catch:
            case Opcode::ScheduleJump:
            case Opcode::LeaveFinally:
            case Opcode::RestoreScheduledJump:
            case Opcode::GetException:
            case Opcode::SetException:
            // Generator/async ops
            case Opcode::Yield:
            case Opcode::Await:
            case Opcode::PrepareYield:
            case Opcode::GetCompletionFields:
            case Opcode::SetCompletionType:
            // Eval
            case Opcode::CallDirectEval:
            case Opcode::CallDirectEvalWithArgumentArray:
            // Argument introspection
            case Opcode::CreateArguments:
            case Opcode::CreateRestParams:
            case Opcode::GetNewTarget:
            // Regex (table remapping not yet supported)
            case Opcode::NewRegExp:
            // Terminators we can't clone
            case Opcode::End:
            case Opcode::ContinuePendingUnwind:
                found = true;
                break;
            default:
                break;
            }
        });
        if (found)
            return true;
    }
    return false;
}

static size_t count_instructions(Function const& function)
{
    size_t count = 0;
    for (auto& block : function.basic_blocks()) {
        block->for_each_instruction([&](Instruction const&) {
            ++count;
        });
    }
    return count;
}

static size_t count_return_terminators(Function const& function)
{
    size_t count = 0;
    for (auto& block : function.basic_blocks()) {
        auto* terminator = block->terminator();
        if (terminator && terminator->opcode() == Opcode::Return)
            ++count;
    }
    return count;
}

static Value* map_callee_value(Value* callee_value, HashMap<ValueIndex, Value*> const& value_map)
{
    if (!callee_value)
        return nullptr;
    auto it = value_map.find(callee_value->index());
    VERIFY(it != value_map.end());
    return it->value;
}

static void clone_instruction(
    Instruction const& source,
    Function& caller,
    Builder& builder,
    HashMap<ValueIndex, Value*>& value_map,
    Bytecode::Executable const& callee_executable,
    Bytecode::Executable& caller_executable)
{
    auto opcode = source.opcode();

    auto map = [&](Value* value) -> Value* {
        return map_callee_value(value, value_map);
    };

    // Remap table indices from callee's executable tables to caller's.
    auto remap_property_key = [&](Bytecode::PropertyKeyTableIndex index) -> Bytecode::PropertyKeyTableIndex {
        if (index.value < callee_executable.property_key_table->property_keys().size())
            return caller_executable.property_key_table->insert(callee_executable.property_key_table->get(index));
        return index;
    };

    auto remap_identifier = [&](Bytecode::IdentifierTableIndex index) -> Bytecode::IdentifierTableIndex {
        if (index.value < callee_executable.identifier_table->identifiers().size())
            return caller_executable.identifier_table->insert(callee_executable.identifier_table->get(index));
        return index;
    };

    auto remap_string = [&](Bytecode::StringTableIndex index) -> Bytecode::StringTableIndex {
        if (index.value < callee_executable.string_table->strings().size())
            return caller_executable.string_table->insert(callee_executable.string_table->get(index));
        return index;
    };

    auto remap_optional_identifier = [&](Optional<Bytecode::IdentifierTableIndex> index) -> Optional<Bytecode::IdentifierTableIndex> {
        if (!index.has_value())
            return index;
        return remap_identifier(index.value());
    };

    auto remap_optional_string = [&](Optional<Bytecode::StringTableIndex> index) -> Optional<Bytecode::StringTableIndex> {
        if (!index.has_value())
            return index;
        return remap_string(index.value());
    };

    auto remap_cache_index = [&]() -> CacheIndex {
        if (source.cache_index().value() == NumericLimits<u32>::max())
            return source.cache_index();

        // Allocate a fresh cache slot in the caller's executable.
        auto uses_property_lookup_cache = opcode == Opcode::GetById
            || opcode == Opcode::GetByIdWithThis
            || opcode == Opcode::GetLength
            || opcode == Opcode::GetLengthWithThis
            || opcode == Opcode::PutById
            || opcode == Opcode::PutByIdWithThis
            || opcode == Opcode::PutGetterById
            || opcode == Opcode::PutSetterById
            || opcode == Opcode::PutPrototypeById
            || opcode == Opcode::PutGetterByIdWithThis
            || opcode == Opcode::PutSetterByIdWithThis
            || opcode == Opcode::PutPrototypeByIdWithThis;
        if (uses_property_lookup_cache) {
            auto new_index = caller_executable.property_lookup_caches.size();
            caller_executable.property_lookup_caches.empend();
            return CacheIndex(new_index);
        }

        if (opcode == Opcode::GetGlobal || opcode == Opcode::SetGlobal) {
            auto new_index = caller_executable.global_variable_caches.size();
            caller_executable.global_variable_caches.empend();
            return CacheIndex(new_index);
        }

        if (opcode == Opcode::GetTemplateObject) {
            auto new_index = caller_executable.template_object_caches.size();
            caller_executable.template_object_caches.empend();
            return CacheIndex(new_index);
        }

        auto uses_object_shape_cache = opcode == Opcode::NewObject
            || opcode == Opcode::CacheObjectShape
            || opcode == Opcode::InitObjectLiteralProperty;
        if (uses_object_shape_cache) {
            auto new_index = caller_executable.object_shape_caches.size();
            caller_executable.object_shape_caches.empend();
            return CacheIndex(new_index);
        }

        if (opcode == Opcode::Call) {
            auto new_index = caller_executable.call_target_profiles.size();
            caller_executable.call_target_profiles.empend();
            return CacheIndex(new_index);
        }

        // Unknown cache type; invalidate.
        return CacheIndex(NumericLimits<u32>::max());
    };

    NonnullOwnPtr<Instruction> new_instruction = [&]() -> NonnullOwnPtr<Instruction> {
        if (is_call_opcode(opcode)) {
            auto instruction = CallInstruction::create(opcode, map(source.operand(0)), map(source.operand(1)));
            for (size_t i = 2; i < source.operand_count(); ++i)
                instruction->add_operand(map(source.operand(i)));
            return instruction;
        }

        if (opcode == Opcode::GetById) {
            auto const& get_by_id = static_cast<GetByIdInstruction const&>(source);
            return GetByIdInstruction::create(map(get_by_id.base()), remap_property_key(get_by_id.property()));
        }

        // Binary ops use specialized instruction class
        if (source.operand_count() == 2 && opcode_operand_arity(opcode) == 2 && opcode_has_result(opcode)) {
            return BinaryOpInstruction::create(opcode, map(source.operand(0)), map(source.operand(1)));
        }

        // Unary ops use specialized instruction class
        if (source.operand_count() == 1 && opcode_operand_arity(opcode) == 1 && opcode_has_result(opcode)) {
            return UnaryOpInstruction::create(opcode, map(source.operand(0)));
        }

        // Generic instruction
        auto instruction = Instruction::create(opcode);
        for (size_t i = 0; i < source.operand_count(); ++i)
            instruction->add_operand(map(source.operand(i)));
        return instruction;
    }();

    // Copy metadata fields, remapping table indices from callee to caller.
    new_instruction->set_property_key_index(remap_property_key(source.property_key_index()));
    new_instruction->set_identifier_index(remap_identifier(source.identifier_index()));
    new_instruction->set_cache_index(remap_cache_index());
    new_instruction->set_property_slot(source.property_slot());
    new_instruction->set_extract_index(source.extract_index());
    new_instruction->set_iterator_hint(source.iterator_hint());
    new_instruction->set_function_node(source.function_node());
    new_instruction->set_class_expression(source.class_expression());
    new_instruction->set_lhs_name(remap_optional_identifier(source.lhs_name()));
    new_instruction->set_base_identifier(remap_optional_identifier(source.base_identifier()));
    new_instruction->set_environment_mode(source.environment_mode());
    new_instruction->set_is_immutable(source.is_immutable());
    new_instruction->set_is_global(source.is_global());
    new_instruction->set_is_strict(source.is_strict());
    new_instruction->set_capacity(source.capacity());
    new_instruction->set_regex_source_index(remap_string(source.regex_source_index()));
    new_instruction->set_regex_flags_index(remap_string(source.regex_flags_index()));
    new_instruction->set_regex_index(source.regex_index());
    new_instruction->set_expression_string(remap_optional_string(source.expression_string()));
    new_instruction->set_builtin(source.builtin());
    new_instruction->set_arguments_kind(source.arguments_kind());
    new_instruction->set_create_arguments_needs_dst(source.create_arguments_needs_dst());
    new_instruction->set_rest_index(source.rest_index());
    new_instruction->set_is_synthetic(source.is_synthetic());
    new_instruction->set_put_kind(source.put_kind());
    new_instruction->set_is_spread(source.is_spread());
    new_instruction->set_completion_type(source.completion_type());
    new_instruction->set_string_table_index(remap_string(source.string_table_index()));
    if (source.source_record().has_value())
        new_instruction->set_source_record(source.source_record().value());

    if (opcode_has_result(opcode) && source.result()) {
        // Use pre-created value from the value_map if available,
        // otherwise create a new one (e.g. for non-inlined clones).
        Value* result;
        auto it = value_map.find(source.result()->index());
        if (it != value_map.end()) {
            result = it->value;
        } else {
            result = &caller.create_value_for_instruction();
            value_map.set(source.result()->index(), result);
        }
        new_instruction->set_result(result);
        builder.insertion_block()->append(move(new_instruction));
        result->defining_instruction()->recompute_result_type();
    } else {
        builder.insertion_block()->append(move(new_instruction));
    }
}

static void inline_candidate(InlineCandidate& candidate, Function& caller, Bytecode::Executable& caller_executable)
{
    auto& call = *candidate.call;
    auto& call_block = *candidate.call_block;
    auto& callee_function = *candidate.callee_function;
    auto& callee_executable = *callee_function.source_executable();

    // The call block ends with: [...instructions...] Call Jump(continuation)
    // After EH splitting, the Call is always the last non-terminator.
    auto* jump_terminator = call_block.terminator();
    VERIFY(jump_terminator);
    VERIFY(jump_terminator->opcode() == Opcode::Jump);
    auto& continuation_block = static_cast<JumpInstruction*>(jump_terminator)->target();

    Builder builder(caller);

    // Save the call's operands before removing it.
    auto* callee_operand = call.callee();
    auto* this_value = call.this_value();
    Vector<Value*> arguments;
    for (size_t i = 0; i < call.argument_count(); ++i)
        arguments.append(call.argument(i));
    auto expression_string = call.expression_string();
    auto profile_cache_index = call.cache_index();
    auto* call_result_value = call.result();

    // Remove the Call and Jump from call_block.
    call_block.remove_terminator();
    call_block.remove_instructions_if([&](Instruction const& instruction) {
        return &instruction == &call;
    });

    // NB: Don't remove call_block as predecessor of continuation_block yet.
    // We use replace_predecessor at the end to atomically swap it with
    // merge_block, preserving phi incoming values in continuation_block.

    // Build value map: callee values -> caller values
    HashMap<ValueIndex, Value*> value_map;

    // Map callee parameters to call arguments
    for (auto* param : callee_function.parameters()) {
        auto param_index = param->parameter_index();
        Value* mapped = param_index < arguments.size()
            ? arguments[param_index]
            : &caller.create_constant(JS::js_undefined());
        value_map.set(param->index(), mapped);
    }

    // Map callee 'this' to call's this_value
    for (auto& value : callee_function.values()) {
        if (value->is_this()) {
            value_map.set(value->index(), this_value);
            break;
        }
    }

    // Map callee constants
    for (auto& value : callee_function.values()) {
        if (value->is_constant())
            value_map.set(value->index(), &caller.create_constant(value->constant_value()));
    }

    // Create blocks in caller for each callee block
    HashMap<BlockIndex, BasicBlock*> block_map;
    for (auto& callee_block : callee_function.basic_blocks()) {
        auto& new_block = caller.create_block(
            String::formatted("inline_{}", callee_block->name()).release_value_but_fixme_should_propagate_errors());
        block_map.set(callee_block->index(), &new_block);
    }

    auto* fast_entry_block = block_map.get(callee_function.entry_block()->index()).value();

    // Create slow_block and merge_block
    auto& slow_block = caller.create_block("inline_slow"_string);
    auto& merge_block = caller.create_block("inline_merge"_string);

    // Emit guard in call_block: guard = StrictlyEquals(callee, expected_callee)
    builder.set_insertion_block(&call_block);
    auto& expected_callee = caller.create_constant(JS::Value(candidate.callee.ptr()));
    auto& guard_value = builder.build_strictly_equals(*callee_operand, expected_callee);
    builder.build_branch(guard_value, *fast_entry_block, slow_block);

    // Build slow path: re-emit the original call
    builder.set_insertion_block(&slow_block);
    auto& slow_call_result = builder.build_call(*callee_operand, *this_value, arguments.span(), expression_string);
    slow_call_result.defining_instruction()->set_cache_index(profile_cache_index);
    builder.build_jump(merge_block);

    // Pre-create result values for all callee instructions so that
    // phi incoming values from back-edges (loops) can be resolved.
    for (auto& callee_block : callee_function.basic_blocks()) {
        callee_block->for_each_phi([&](PhiInstruction const& phi) {
            if (phi.result()) {
                auto& result = caller.create_value_for_instruction();
                value_map.set(phi.result()->index(), &result);
            }
        });
        callee_block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() == Opcode::Phi)
                return;
            if (opcode_has_result(instruction.opcode()) && instruction.result()) {
                auto& result = caller.create_value_for_instruction();
                value_map.set(instruction.result()->index(), &result);
            }
        });
    }

    // Track the return value from the fast path
    Value* fast_return_value = nullptr;
    BasicBlock* fast_return_block = nullptr;

    // Clone callee instructions into the mapped blocks
    for (auto& callee_block : callee_function.basic_blocks()) {
        auto* target_block = block_map.get(callee_block->index()).value();
        builder.set_insertion_block(target_block);

        // Clone phi instructions, using pre-created result values.
        callee_block->for_each_phi([&](PhiInstruction const& phi) {
            Vector<Value*> mapped_values;
            Vector<BlockIndex> mapped_predecessors;
            for (size_t i = 0; i < phi.incoming_count(); ++i) {
                auto* mapped_block = block_map.get(phi.incoming_block(i)).value();
                mapped_predecessors.append(mapped_block->index());
                auto* incoming = phi.incoming_value(i);
                mapped_values.append(map_callee_value(incoming, value_map));
            }
            auto& phi_result = builder.build_phi(move(mapped_values), move(mapped_predecessors));
            // Replace the pre-created placeholder with the actual phi result.
            if (phi.result()) {
                auto* pre_created = value_map.get(phi.result()->index()).value();
                pre_created->replace_all_uses_with(&phi_result);
                value_map.set(phi.result()->index(), &phi_result);
            }
        });

        // Clone non-phi, non-terminator instructions
        callee_block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() == Opcode::Phi)
                return;
            if (instruction.is_terminator())
                return;
            clone_instruction(instruction, caller, builder, value_map, callee_executable, caller_executable);
        });

        // Handle terminators
        auto* terminator = callee_block->terminator();
        if (!terminator)
            continue;

        // NB: Use manual instruction creation instead of builder.build_jump/
        // build_branch to avoid CFG::add_predecessor adding duplicate null
        // entries to phis that we already cloned above. We add predecessor
        // entries directly to the target blocks without touching their phis.
        switch (terminator->opcode()) {
        case Opcode::Jump: {
            auto const& jump = static_cast<JumpInstruction const&>(*terminator);
            auto* mapped_target = block_map.get(jump.target().index()).value();
            target_block->append(JumpInstruction::create(*mapped_target));
            CFG::add_block_predecessor(*mapped_target, *target_block);
            break;
        }
        case Opcode::Branch: {
            auto const& branch = static_cast<BranchInstruction const&>(*terminator);
            auto* condition = map_callee_value(branch.condition(), value_map);
            auto* true_target = block_map.get(branch.true_branch().index()).value();
            auto* false_target = block_map.get(branch.false_branch().index()).value();
            target_block->append(BranchInstruction::create(condition, *true_target, *false_target));
            CFG::add_block_predecessor(*true_target, *target_block);
            CFG::add_block_predecessor(*false_target, *target_block);
            break;
        }
        case Opcode::Return: {
            fast_return_value = map_callee_value(terminator->operand(0), value_map);
            fast_return_block = target_block;
            target_block->append(JumpInstruction::create(merge_block));
            CFG::add_block_predecessor(merge_block, *target_block);
            break;
        }
        default:
            VERIFY_NOT_REACHED();
        }
    }

    VERIFY(fast_return_value);
    VERIFY(fast_return_block);

    // Build merge block with phi
    builder.set_insertion_block(&merge_block);
    auto& merged_result = builder.build_phi(
        { &slow_call_result, fast_return_value },
        { slow_block.index(), fast_return_block->index() });

    // Replace all uses of the original call result with the phi result
    if (call_result_value)
        call_result_value->replace_all_uses_with(&merged_result);

    // Jump from merge_block to continuation.
    // Use a manual Jump + replace_predecessor instead of build_jump to
    // preserve phi incoming values that continuation_block had for call_block.
    auto jump_instruction = JumpInstruction::create(continuation_block);
    merge_block.append(move(jump_instruction));
    CFG::replace_predecessor(continuation_block, call_block, merge_block);
}

PreservedAnalyses InlineCalls::run(Function& function, PassManager&)
{
    auto source_executable = function.source_executable();
    if (!source_executable)
        return PreservedAnalyses::all();

    Vector<InlineCandidate> candidates;

    for (auto& block : function.basic_blocks()) {
        bool block_has_eh = block->exception_handler_index().has_value() || block->finalizer_index().has_value();

        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() != Opcode::Call)
                return;

            auto const& call = static_cast<CallInstruction const&>(instruction);
            auto profile_index = static_cast<u32>(call.cache_index());
            if (profile_index >= source_executable->call_target_profiles.size())
                return;

            auto const& profile = source_executable->call_target_profiles[profile_index];
            if (profile.total_count == 0)
                return;

            auto callee_ptr = profile.callee.ptr();
            auto hit_rate = profile.hit_count * 100 / profile.total_count;

            auto log_skip = [&](char const* reason) {
                if (g_log_tier_ups) {
                    if (callee_ptr)
                        dbgln("  call[{}]: {} ({}% of {} calls) -- not inlined: {}", profile_index, static_cast<FunctionObject*>(callee_ptr)->name_for_call_stack(), hit_rate, profile.total_count, reason);
                    else
                        dbgln("  call[{}]: ?? ({}% of {} calls) -- not inlined: {}", profile_index, hit_rate, profile.total_count, reason);
                }
            };

            // Profile gates
            if (profile.total_count < 64) {
                log_skip("too few calls");
                return;
            }
            if (hit_rate < 90) {
                log_skip("polymorphic");
                return;
            }
            if (!callee_ptr) {
                log_skip("callee GC'd");
                return;
            }

            // Callee must be an ECMAScript function
            auto* ecma_callee = dynamic_cast<ECMAScriptFunctionObject*>(static_cast<FunctionObject*>(callee_ptr));
            if (!ecma_callee) {
                log_skip("not an ECMAScript function");
                return;
            }

            // Callee must have a bytecode executable
            auto callee_executable = ecma_callee->bytecode_executable();
            if (!callee_executable) {
                log_skip("no bytecode");
                return;
            }

            // No recursive inlining
            if (callee_executable == source_executable) {
                log_skip("recursive");
                return;
            }

            // Call site must not have exception handler
            if (block_has_eh) {
                log_skip("call site has exception handler");
                return;
            }

            // Lift callee bytecode to IR and optimize it
            auto [callee_function, ssa_data] = Lifter::lift(*callee_executable);

            PassManager callee_pass_manager;

            SSAConstructionPass ssa_pass(move(ssa_data));
            auto preserved = ssa_pass.run(*callee_function, callee_pass_manager);
            callee_pass_manager.invalidate(preserved);

            SCCP sccp_pass;
            preserved = sccp_pass.run(*callee_function, callee_pass_manager);
            callee_pass_manager.invalidate(preserved);

            // Callee body gates
            auto instruction_count = count_instructions(*callee_function);
            if (instruction_count > 40) {
                log_skip("callee too large");
                return;
            }
            if (has_unsupported_opcode(*callee_function)) {
                log_skip("unsupported opcode in callee");
                return;
            }
            if (count_return_terminators(*callee_function) != 1) {
                log_skip("multiple returns");
                return;
            }

            candidates.append({
                .call = const_cast<CallInstruction*>(&call),
                .call_block = block.ptr(),
                .callee = *ecma_callee,
                .callee_function = move(callee_function),
            });
        });
    }

    if (candidates.is_empty())
        return PreservedAnalyses::all();

    auto& caller_executable = *const_cast<Bytecode::Executable*>(source_executable.ptr());

    for (auto& candidate : candidates) {
        if (g_log_tier_ups) {
            auto profile_index = static_cast<u32>(candidate.call->cache_index());
            auto const& profile = source_executable->call_target_profiles[profile_index];
            auto hit_rate = profile.hit_count * 100 / profile.total_count;
            dbgln("  call[{}]: {} ({}% of {} calls) \033[32;1m[inlined]\033[0m", profile_index, candidate.callee->name_for_call_stack(), hit_rate, profile.total_count);
        }
        inline_candidate(candidate, function, caller_executable);
    }

    return PreservedAnalyses::none();
}

}
