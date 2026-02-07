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

PreservedAnalyses InlineCalls::run(Function& function, PassManager&)
{
    auto source_executable = function.source_executable();
    if (!source_executable)
        return PreservedAnalyses::all();

    Vector<InlineCandidate> candidates;

    for (auto& block : function.basic_blocks()) {
        if (block->exception_handler_index().has_value() || block->finalizer_index().has_value())
            continue;

        block->for_each_instruction([&](Instruction const& instruction) {
            if (instruction.opcode() != Opcode::Call)
                return;

            auto& call = static_cast<CallInstruction const&>(instruction);
            auto profile_index = static_cast<u32>(call.cache_index());
            if (profile_index >= source_executable->call_target_profiles.size())
                return;

            auto const& profile = source_executable->call_target_profiles[profile_index];

            // Profile gates
            if (profile.total_count < 64)
                return;
            if (profile.hit_count * 100 < profile.total_count * 90)
                return;
            auto callee_ptr = profile.callee.ptr();
            if (!callee_ptr)
                return;

            // Callee must be an ECMAScript function
            auto* ecma_callee = dynamic_cast<ECMAScriptFunctionObject*>(static_cast<FunctionObject*>(callee_ptr));
            if (!ecma_callee)
                return;

            // Callee must have a bytecode executable
            auto callee_executable = ecma_callee->bytecode_executable();
            if (!callee_executable)
                return;

            // No recursive inlining
            if (callee_executable == source_executable)
                return;

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
            if (count_instructions(*callee_function) > 40)
                return;
            if (has_unsupported_opcode(*callee_function))
                return;
            if (count_return_terminators(*callee_function) != 1)
                return;

            if (g_log_tier_ups)
                dbgln("  inline candidate: call[{}] -> {}", profile_index, ecma_callee->name_for_call_stack());

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

    // Transform will be implemented in the next commit.
    return PreservedAnalyses::all();
}

}
