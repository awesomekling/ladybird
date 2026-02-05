/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/BinarySearch.h>
#include <AK/QuickSort.h>
#include <LibJS/Bytecode/Instruction.h>
#include <LibJS/Bytecode/Op.h>
#include <LibJS/Bytecode/Register.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/CFG.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Lifter.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

Lifter::Lifter(Bytecode::Executable const& executable)
    : m_executable(executable)
    , m_function(Function::create(&executable))
{
}

NonnullOwnPtr<Function> Lifter::lift(Bytecode::Executable const& executable)
{
    Lifter lifter(executable);
    lifter.lift_basic_blocks();
    lifter.connect_control_flow();
    lifter.compute_block_predecessors();

    // If the entry block has predecessors (e.g., from labelled breaks that jump
    // to the start of the function), insert a new empty entry block.
    // SSA requires the entry block to have no predecessors.
    if (auto* entry = lifter.m_function->entry_block(); entry && !entry->predecessors().is_empty()) {
        auto& new_entry = lifter.m_function->create_block("entry"_string);
        lifter.m_function->build_jump(new_entry, *entry);
        lifter.m_function->set_entry_block(&new_entry);
        lifter.compute_block_predecessors();
    }

    lifter.compute_dominators();
    lifter.eliminate_unreachable_blocks();

    // SSA construction using dominance-based approach:
    // Phase 1: Place phis at dominance frontiers of defining blocks
    // Phase 2: Fill in phi operands by finding reaching definitions
    lifter.place_phi_nodes();
    lifter.fill_phi_operands();

    // Store the source block map for exception handler remapping in the lowerer
    lifter.m_function->set_source_block_map(move(lifter.m_block_map));

    return move(lifter.m_function);
}

void Lifter::lift_basic_blocks()
{
    // First pass: create IR basic blocks for each bytecode basic block
    for (size_t i = 0; i < m_executable.basic_block_start_offsets.size(); ++i) {
        auto& block = m_function->create_block(String::formatted("block{}", i).release_value_but_fixme_should_propagate_errors());
        m_block_map.set(static_cast<u32>(i), &block);
        if (i == 0)
            m_function->set_entry_block(&block);
    }

    // Pre-pass: identify Yield/Await continuation blocks and create resume values
    // These blocks receive an implicit resume value in the accumulator (reg0)
    for (size_t block_index = 0; block_index < m_executable.basic_block_start_offsets.size(); ++block_index) {
        size_t start_offset = m_executable.basic_block_start_offsets[block_index];
        size_t end_offset = (block_index + 1 < m_executable.basic_block_start_offsets.size())
            ? m_executable.basic_block_start_offsets[block_index + 1]
            : m_executable.bytecode.size();

        auto bytecode_span = ReadonlyBytes { m_executable.bytecode.data() + start_offset, end_offset - start_offset };
        Bytecode::InstructionStreamIterator it(bytecode_span, &m_executable);

        // Find the last instruction (terminator)
        Bytecode::Instruction const* last_instruction = nullptr;
        while (!it.at_end()) {
            last_instruction = &*it;
            ++it;
        }

        if (!last_instruction)
            continue;

        using enum Bytecode::Instruction::Type;
        if (last_instruction->type() == Yield) {
            auto const& op = static_cast<Bytecode::Op::Yield const&>(*last_instruction);
            if (op.continuation_label().has_value()) {
                auto cont_block_index = address_to_block_index(op.continuation_label()->address());
                // Create a placeholder value for the resume value
                auto& resume_value = m_function->create_register_value();
                m_continuation_resume_values.set(cont_block_index, &resume_value);
            }
        } else if (last_instruction->type() == Await) {
            auto const& op = static_cast<Bytecode::Op::Await const&>(*last_instruction);
            auto cont_block_index = address_to_block_index(op.continuation_label().address());
            auto& resume_value = m_function->create_register_value();
            m_continuation_resume_values.set(cont_block_index, &resume_value);
        }
    }

    // Second pass: lift instructions from each basic block
    //
    // EH Splitting Invariant:
    // We split IR blocks at may-throw instructions when there are more bytecode
    // instructions to follow. This ensures that:
    // 1. Each block with an exception handler has at most one may-throw instruction
    //    (plus any non-throwing cleanup like ExtractValue) before the terminator.
    // 2. The exception handler sees the correct reaching definitions at the throw
    //    point, not values that would have been defined after the throw.
    //
    // The IR verifier's dominance checks implicitly verify this: if values defined
    // after a throw point were incorrectly visible to the handler, they wouldn't
    // dominate the handler's uses and the verifier would report an SSA violation.
    u32 split_counter = 0;
    for (size_t block_index = 0; block_index < m_executable.basic_block_start_offsets.size(); ++block_index) {
        auto* current_block = m_block_map.get(static_cast<u32>(block_index)).value();

        size_t start_offset = m_executable.basic_block_start_offsets[block_index];
        size_t end_offset = (block_index + 1 < m_executable.basic_block_start_offsets.size())
            ? m_executable.basic_block_start_offsets[block_index + 1]
            : m_executable.bytecode.size();

        // Get exception handlers for this bytecode block
        BasicBlock* exception_handler = nullptr;
        BasicBlock* finalizer = nullptr;
        if (auto handlers = m_executable.exception_handlers_for_offset(start_offset); handlers.has_value()) {
            if (handlers->handler_offset.has_value()) {
                auto handler_block_index = address_to_block_index(handlers->handler_offset.value());
                exception_handler = m_block_map.get(handler_block_index).value();
            }
            if (handlers->finalizer_offset.has_value()) {
                auto finalizer_block_index = address_to_block_index(handlers->finalizer_offset.value());
                finalizer = m_block_map.get(finalizer_block_index).value();
            }
        }
        current_block->set_exception_handler(exception_handler);
        current_block->set_finalizer(finalizer);

        // If this block is a Yield/Await continuation, set up the resume value
        // for the accumulator (reg0) so instructions in this block can use it
        if (auto resume_value = m_continuation_resume_values.get(static_cast<u32>(block_index)); resume_value.has_value()) {
            auto acc_raw = Bytecode::Operand(Bytecode::Register::accumulator()).raw();
            m_current_definitions.set(acc_raw, *resume_value);
            m_value_to_operand_raw.set(*resume_value, acc_raw);
        }

        auto bytecode_span = ReadonlyBytes { m_executable.bytecode.data() + start_offset, end_offset - start_offset };
        Bytecode::InstructionStreamIterator it(bytecode_span, &m_executable);

        while (!it.at_end()) {
            size_t instr_count_before = current_block->instructions().size();

            // Look up source record for this bytecode instruction
            auto absolute_offset = start_offset + it.offset();
            auto* source_entry = binary_search(m_executable.source_map, absolute_offset, nullptr, [](size_t needle, Bytecode::SourceMapEntry const& entry) -> int {
                if (needle < entry.bytecode_offset)
                    return -1;
                if (needle > entry.bytecode_offset)
                    return 1;
                return 0;
            });

            lift_instruction(*it, *current_block);
            ++it;

            // Attach source record to all IR instructions created from this bytecode instruction
            if (source_entry) {
                for (size_t i = instr_count_before; i < current_block->instructions().size(); ++i)
                    current_block->instructions()[i]->set_source_record(source_entry->source_record);
            }

            // Check if we added any may-throw instructions
            bool added_may_throw = false;
            for (size_t i = instr_count_before; i < current_block->instructions().size(); ++i) {
                if (may_throw_opcode(current_block->instructions()[i]->opcode())) {
                    added_may_throw = true;
                    break;
                }
            }

            // If we added a may-throw instruction and there are more bytecode instructions,
            // split the block to ensure the exception edge has correct reaching definitions.
            // This way, values defined after the throw point won't incorrectly flow to handlers.
            if (added_may_throw && !it.at_end()) {
                // Save current block's definitions (this is the state at the throw point)
                m_block_definitions.set(current_block, m_current_definitions);

                // Create continuation block for remaining instructions
                auto& continuation = m_function->create_block(
                    String::formatted("block{}_split{}", block_index, split_counter++).release_value_but_fixme_should_propagate_errors());

                // Continuation inherits exception handlers
                continuation.set_exception_handler(exception_handler);
                continuation.set_finalizer(finalizer);

                // Emit fallthrough jump from current block to continuation
                m_function->build_jump(*current_block, continuation);

                // Continue lifting into continuation block
                current_block = &continuation;
            }
        }

        // Save final block's definitions (snapshot at end of block)
        m_block_definitions.set(current_block, m_current_definitions);

        // Record the final IR block for this bytecode block (after any EH splits)
        // This is the block that should receive the bytecode block's terminator
        m_final_ir_block.set(static_cast<u32>(block_index), current_block);
    }
}

Value& Lifter::get_or_create_value_for_operand(Bytecode::Operand operand, BasicBlock& block)
{
    auto raw = operand.raw();

    // Check if we already have a value for this operand in the current block
    if (auto it = m_current_definitions.find(raw); it != m_current_definitions.end())
        return *it->value;

    // Decode the operand to get the real type (operands are stored in a flat space)
    auto decoded_operand = m_executable.original_operand_from_raw(raw);

    // Create a new value for this operand
    Value* value = nullptr;

    if (decoded_operand.is_constant()) {
        // Get the constant from the executable
        auto constant = m_executable.constants[decoded_operand.index()];
        value = &m_function->create_constant(constant);
    } else if (decoded_operand.type() == Bytecode::Operand::Type::Argument) {
        // For arguments, create a parameter value to preserve the argument index
        value = &m_function->create_parameter(decoded_operand.index());
    } else if (decoded_operand.is_register() && decoded_operand.index() == Bytecode::Register::this_value().index()) {
        // For the this register, create a special this value
        value = &m_function->create_this();
    } else {
        // For registers/locals, create a register value
        // NB: In full SSA, phi nodes would be inserted at merge points
        value = &m_function->create_register_value();
        // NB: Register values need the operand mapping so SSA renaming can replace
        // them with the proper reaching definition.
        m_value_to_operand_raw.set(value, raw);
    }

    m_current_definitions.set(raw, value);
    return *value;

    (void)block; // Will be used for phi node resolution
}

void Lifter::define_operand(Bytecode::Operand operand, Value& value, BasicBlock& block)
{
    auto raw = operand.raw();
    m_current_definitions.set(raw, &value);
    m_written_operands.set(raw);
    m_block_actual_definitions.ensure(&block).set(raw);
    m_value_to_operand_raw.set(&value, raw);
}

template<typename BytecodeOp>
void Lifter::lift_binary_op(Bytecode::Instruction const& instruction, BasicBlock& block, Value& (Function::*build_fn)(BasicBlock&, Value&, Value&))
{
    auto const& op = static_cast<BytecodeOp const&>(instruction);
    auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
    auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
    auto& result = (m_function.ptr()->*build_fn)(block, lhs, rhs);
    define_operand(op.dst(), result, block);
}

template<typename BytecodeOp>
void Lifter::lift_unary_op_src(Bytecode::Instruction const& instruction, BasicBlock& block, Value& (Function::*build_fn)(BasicBlock&, Value&))
{
    auto const& op = static_cast<BytecodeOp const&>(instruction);
    auto& src = get_or_create_value_for_operand(op.src(), block);
    auto& result = (m_function.ptr()->*build_fn)(block, src);
    define_operand(op.dst(), result, block);
}

template<typename BytecodeOp>
void Lifter::lift_unary_op_value(Bytecode::Instruction const& instruction, BasicBlock& block, Value& (Function::*build_fn)(BasicBlock&, Value&))
{
    auto const& op = static_cast<BytecodeOp const&>(instruction);
    auto& src = get_or_create_value_for_operand(op.value(), block);
    auto& result = (m_function.ptr()->*build_fn)(block, src);
    define_operand(op.dst(), result, block);
}

void Lifter::lift_instruction(Bytecode::Instruction const& instruction, BasicBlock& block)
{
    using enum Bytecode::Instruction::Type;

    switch (instruction.type()) {
    // Arithmetic binary ops
    case Add:
        lift_binary_op<Bytecode::Op::Add>(instruction, block, &Function::build_add);
        break;
    case Sub:
        lift_binary_op<Bytecode::Op::Sub>(instruction, block, &Function::build_sub);
        break;
    case Mul:
        lift_binary_op<Bytecode::Op::Mul>(instruction, block, &Function::build_mul);
        break;
    case Div:
        lift_binary_op<Bytecode::Op::Div>(instruction, block, &Function::build_div);
        break;
    case Mod:
        lift_binary_op<Bytecode::Op::Mod>(instruction, block, &Function::build_mod);
        break;
    case Exp:
        lift_binary_op<Bytecode::Op::Exp>(instruction, block, &Function::build_exp);
        break;

    // Bitwise binary ops
    case BitwiseAnd:
        lift_binary_op<Bytecode::Op::BitwiseAnd>(instruction, block, &Function::build_bitwise_and);
        break;
    case BitwiseOr:
        lift_binary_op<Bytecode::Op::BitwiseOr>(instruction, block, &Function::build_bitwise_or);
        break;
    case BitwiseXor:
        lift_binary_op<Bytecode::Op::BitwiseXor>(instruction, block, &Function::build_bitwise_xor);
        break;
    case LeftShift:
        lift_binary_op<Bytecode::Op::LeftShift>(instruction, block, &Function::build_left_shift);
        break;
    case RightShift:
        lift_binary_op<Bytecode::Op::RightShift>(instruction, block, &Function::build_right_shift);
        break;
    case UnsignedRightShift:
        lift_binary_op<Bytecode::Op::UnsignedRightShift>(instruction, block, &Function::build_unsigned_right_shift);
        break;

    // Comparison ops
    case LessThan:
        lift_binary_op<Bytecode::Op::LessThan>(instruction, block, &Function::build_less_than);
        break;
    case LessThanEquals:
        lift_binary_op<Bytecode::Op::LessThanEquals>(instruction, block, &Function::build_less_than_equals);
        break;
    case GreaterThan:
        lift_binary_op<Bytecode::Op::GreaterThan>(instruction, block, &Function::build_greater_than);
        break;
    case GreaterThanEquals:
        lift_binary_op<Bytecode::Op::GreaterThanEquals>(instruction, block, &Function::build_greater_than_equals);
        break;
    case LooselyEquals:
        lift_binary_op<Bytecode::Op::LooselyEquals>(instruction, block, &Function::build_loosely_equals);
        break;
    case StrictlyEquals:
        lift_binary_op<Bytecode::Op::StrictlyEquals>(instruction, block, &Function::build_strictly_equals);
        break;
    case LooselyInequals:
        lift_binary_op<Bytecode::Op::LooselyInequals>(instruction, block, &Function::build_loosely_inequals);
        break;
    case StrictlyInequals:
        lift_binary_op<Bytecode::Op::StrictlyInequals>(instruction, block, &Function::build_strictly_inequals);
        break;

    // Unary ops with src()
    case BitwiseNot:
        lift_unary_op_src<Bytecode::Op::BitwiseNot>(instruction, block, &Function::build_bitwise_not);
        break;
    case UnaryMinus:
        lift_unary_op_src<Bytecode::Op::UnaryMinus>(instruction, block, &Function::build_negate);
        break;
    case UnaryPlus:
        lift_unary_op_src<Bytecode::Op::UnaryPlus>(instruction, block, &Function::build_unary_plus);
        break;
    case Not:
        lift_unary_op_src<Bytecode::Op::Not>(instruction, block, &Function::build_not);
        break;
    case Typeof:
        lift_unary_op_src<Bytecode::Op::Typeof>(instruction, block, &Function::build_typeof);
        break;

    // Unary ops with value()
    case ToBoolean:
        lift_unary_op_value<Bytecode::Op::ToBoolean>(instruction, block, &Function::build_to_boolean);
        break;
    case ToObject:
        lift_unary_op_value<Bytecode::Op::ToObject>(instruction, block, &Function::build_to_object);
        break;
    case ToString:
        lift_unary_op_value<Bytecode::Op::ToString>(instruction, block, &Function::build_to_string);
        break;
    case ToInt32:
        lift_unary_op_value<Bytecode::Op::ToInt32>(instruction, block, &Function::build_to_int32);
        break;
    case ToLength:
        lift_unary_op_value<Bytecode::Op::ToLength>(instruction, block, &Function::build_to_length);
        break;
    case ToNumeric:
        lift_unary_op_value<Bytecode::Op::ToNumeric>(instruction, block, &Function::build_to_numeric);
        break;
    case TypeofBinding: {
        auto const& op = static_cast<Bytecode::Op::TypeofBinding const&>(instruction);
        auto& result = m_function->build_typeof_binding(block, op.identifier());
        define_operand(op.dst(), result, block);
        break;
    }

    // Increment/Decrement
    case Increment: {
        auto const& op = static_cast<Bytecode::Op::Increment const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.dst(), block);
        auto& result = m_function->build_increment(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case Decrement: {
        auto const& op = static_cast<Bytecode::Op::Decrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.dst(), block);
        auto& result = m_function->build_decrement(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case PostfixIncrement: {
        auto const& op = static_cast<Bytecode::Op::PostfixIncrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        // dst gets the OLD NUMERIC value (PostfixIncrement returns ToNumeric(src))
        auto& old_value = m_function->build_to_numeric(block, src);
        define_operand(op.dst(), old_value, block);
        // src gets MUTATED to src + 1 - create a new SSA value for it
        auto& incremented = m_function->build_increment(block, old_value);
        define_operand(op.src(), incremented, block);
        break;
    }
    case PostfixDecrement: {
        auto const& op = static_cast<Bytecode::Op::PostfixDecrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        // dst gets the OLD NUMERIC value (PostfixDecrement returns ToNumeric(src))
        auto& old_value = m_function->build_to_numeric(block, src);
        define_operand(op.dst(), old_value, block);
        // src gets MUTATED to src - 1 - create a new SSA value for it
        auto& decremented = m_function->build_decrement(block, old_value);
        define_operand(op.src(), decremented, block);
        break;
    }

    // String ops
    case ConcatString: {
        auto const& op = static_cast<Bytecode::Op::ConcatString const&>(instruction);
        auto& dst = get_or_create_value_for_operand(op.dst(), block);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_function->build_concat_string(block, dst, src);
        define_operand(op.dst(), result, block);
        break;
    }

    // Move
    case Mov: {
        auto const& op = static_cast<Bytecode::Op::Mov const&>(instruction);
        // Detect writes to special registers used by the unwind mechanism:
        // - Mov to saved_return_value (register 1): emit SetSavedReturnValue
        // - Mov to exception (register 2) with empty value: emit ClearException
        if (op.dst().is_register() && op.dst().index() == Bytecode::Register::saved_return_value_index) {
            auto& src = get_or_create_value_for_operand(op.src(), block);
            m_function->build_set_saved_return_value(block, src);
            break;
        }
        if (op.dst().is_register() && op.dst().index() == Bytecode::Register::exception_index) {
            // Mov to exception register: use SetException to write physical reg2.
            auto& src = get_or_create_value_for_operand(op.src(), block);
            m_function->build_set_exception(block, src);
            break;
        }
        if (op.src().is_register() && op.src().index() == Bytecode::Register::exception_index) {
            // Mov from exception register: use GetException to read physical reg2.
            auto& result = m_function->build_get_exception(block);
            define_operand(op.dst(), result, block);
            break;
        }
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_function->build_move(block, src);
        define_operand(op.dst(), result, block);
        break;
    }

    // Control flow - handled in connect_control_flow()
    case Jump:
    case JumpIf:
    case JumpTrue:
    case JumpFalse:
    case JumpGreaterThan:
    case JumpGreaterThanEquals:
    case JumpLessThan:
    case JumpLessThanEquals:
    case JumpLooselyEquals:
    case JumpLooselyInequals:
    case JumpStrictlyEquals:
    case JumpStrictlyInequals:
    case JumpNullish:
    case JumpUndefined:
        // These are handled later when we connect control flow
        break;

    case Return: {
        auto const& op = static_cast<Bytecode::Op::Return const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.value(), block);
        m_function->build_return(block, value);
        break;
    }

    case Throw: {
        auto const& op = static_cast<Bytecode::Op::Throw const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_throw(block, value);
        break;
    }

    case End: {
        auto const& op = static_cast<Bytecode::Op::End const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.value(), block);
        m_function->build_end(block, value);
        break;
    }

    // Property access
    case GetById: {
        auto const& op = static_cast<Bytecode::Op::GetById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_get_by_id(block, base, op.property(), op.base_identifier());
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case GetByValue: {
        auto const& op = static_cast<Bytecode::Op::GetByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_function->build_get_by_value(block, base, property, op.base_identifier());
        define_operand(op.dst(), result, block);
        break;
    }
    case PutNormalById: {
        auto const& op = static_cast<Bytecode::Op::PutNormalById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_id(block, base, op.property(), value, op.base_identifier());
        block.instructions().last()->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutNormalByValue: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_value(block, base, property, value, op.base_identifier());
        break;
    }

    // WithThis property access variants
    case GetByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::GetByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& result = m_function->build_get_by_id_with_this(block, base, this_value, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::GetByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_function->build_get_by_value_with_this(block, base, this_value, property);
        define_operand(op.dst(), result, block);
        break;
    }
    case PutNormalByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_id_with_this(block, base, this_value, op.property(), value);
        break;
    }
    case PutNormalByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_value_with_this(block, base, this_value, property, value);
        break;
    }
    case DeleteByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::DeleteByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& result = m_function->build_delete_by_id_with_this(block, base, this_value, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case DeleteByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::DeleteByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_function->build_delete_by_value_with_this(block, base, this_value, property);
        define_operand(op.dst(), result, block);
        break;
    }

    case DeleteById: {
        auto const& op = static_cast<Bytecode::Op::DeleteById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_delete_by_id(block, base, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case DeleteByValue: {
        auto const& op = static_cast<Bytecode::Op::DeleteByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_function->build_delete_by_value(block, base, property);
        define_operand(op.dst(), result, block);
        break;
    }

    // Other Put variants (for object literals, classes, etc.)
    case PutOwnById: {
        auto const& op = static_cast<Bytecode::Op::PutOwnById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_id(block, base, op.property(), value);
        break;
    }
    case PutOwnByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutOwnByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_id(block, base, op.property(), value);
        break;
    }
    case PutOwnByValue: {
        auto const& op = static_cast<Bytecode::Op::PutOwnByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_value(block, base, property, value);
        break;
    }
    case PutOwnByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutOwnByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_value(block, base, property, value);
        break;
    }
    case InitObjectLiteralProperty: {
        auto const& op = static_cast<Bytecode::Op::InitObjectLiteralProperty const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_init_object_literal_property(block, object, op.property(), value, CacheIndex { op.shape_cache_index() }, PropertySlot { op.property_slot() });
        break;
    }
    case CreateDataPropertyOrThrow: {
        auto const& op = static_cast<Bytecode::Op::CreateDataPropertyOrThrow const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.object(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.value(), block);
        m_function->build_put_by_value(block, base, property, value);
        break;
    }

    // Getters/setters/prototypes
    case PutGetterById: {
        auto const& op = static_cast<Bytecode::Op::PutGetterById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& getter = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_getter_by_id(block, base, op.property(), getter, op.base_identifier());
        break;
    }
    case PutSetterById: {
        auto const& op = static_cast<Bytecode::Op::PutSetterById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& setter = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_setter_by_id(block, base, op.property(), setter, op.base_identifier());
        break;
    }
    case PutPrototypeById: {
        auto const& op = static_cast<Bytecode::Op::PutPrototypeById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& prototype = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_prototype_by_id(block, base, op.property(), prototype, op.base_identifier());
        break;
    }
    case PutGetterByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutGetterByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& getter = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_getter_by_id_with_this(block, base, this_value, op.property(), getter);
        break;
    }
    case PutSetterByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutSetterByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& setter = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_setter_by_id_with_this(block, base, this_value, op.property(), setter);
        break;
    }
    case PutPrototypeByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutPrototypeByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& prototype = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_prototype_by_id_with_this(block, base, this_value, op.property(), prototype);
        break;
    }
    case PutGetterByValue: {
        auto const& op = static_cast<Bytecode::Op::PutGetterByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& getter = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_getter_by_value(block, base, property, getter, op.base_identifier());
        break;
    }
    case PutSetterByValue: {
        auto const& op = static_cast<Bytecode::Op::PutSetterByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& setter = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_setter_by_value(block, base, property, setter, op.base_identifier());
        break;
    }
    case PutPrototypeByValue: {
        auto const& op = static_cast<Bytecode::Op::PutPrototypeByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& prototype = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_prototype_by_value(block, base, property, prototype, op.base_identifier());
        break;
    }
    case PutGetterByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutGetterByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& getter = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_getter_by_value_with_this(block, base, property, this_value, getter);
        break;
    }
    case PutSetterByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutSetterByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& setter = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_setter_by_value_with_this(block, base, property, this_value, setter);
        break;
    }
    case PutPrototypeByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutPrototypeByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& prototype = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_prototype_by_value_with_this(block, base, property, this_value, prototype);
        break;
    }
    case PutBySpread: {
        auto const& op = static_cast<Bytecode::Op::PutBySpread const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& source = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_spread(block, base, source);
        break;
    }

    // In/InstanceOf
    case In:
        lift_binary_op<Bytecode::Op::In>(instruction, block, &Function::build_in);
        break;
    case InstanceOf:
        lift_binary_op<Bytecode::Op::InstanceOf>(instruction, block, &Function::build_instance_of);
        break;

    // Environment
    case GetBinding: {
        auto const& op = static_cast<Bytecode::Op::GetBinding const&>(instruction);
        auto& result = m_function->build_get_binding(block, op.identifier());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetInitializedBinding: {
        auto const& op = static_cast<Bytecode::Op::GetInitializedBinding const&>(instruction);
        auto& result = m_function->build_get_binding(block, op.identifier());
        define_operand(op.dst(), result, block);
        break;
    }
    case SetLexicalBinding: {
        auto const& op = static_cast<Bytecode::Op::SetLexicalBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_set_binding(block, op.identifier(), value, Bytecode::Op::EnvironmentMode::Lexical);
        break;
    }
    case SetVariableBinding: {
        auto const& op = static_cast<Bytecode::Op::SetVariableBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_set_binding(block, op.identifier(), value, Bytecode::Op::EnvironmentMode::Var);
        break;
    }
    case InitializeLexicalBinding: {
        auto const& op = static_cast<Bytecode::Op::InitializeLexicalBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_initialize_binding(block, op.identifier(), value, Bytecode::Op::EnvironmentMode::Lexical);
        break;
    }
    case InitializeVariableBinding: {
        auto const& op = static_cast<Bytecode::Op::InitializeVariableBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_initialize_binding(block, op.identifier(), value, Bytecode::Op::EnvironmentMode::Var);
        break;
    }
    case DeleteVariable: {
        auto const& op = static_cast<Bytecode::Op::DeleteVariable const&>(instruction);
        auto& result = m_function->build_delete_variable(block, op.identifier());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetGlobal: {
        auto const& op = static_cast<Bytecode::Op::GetGlobal const&>(instruction);
        auto& result = m_function->build_get_global(block, op.identifier());
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case SetGlobal: {
        auto const& op = static_cast<Bytecode::Op::SetGlobal const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_set_global(block, op.identifier(), value);
        block.instructions().last()->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }

    // Object creation
    case NewObject: {
        auto const& op = static_cast<Bytecode::Op::NewObject const&>(instruction);
        auto& result = m_function->build_new_object(block);
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case NewArray: {
        auto const& op = static_cast<Bytecode::Op::NewArray const&>(instruction);
        Vector<Value*> elements;
        for (auto operand : op.elements())
            elements.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_function->build_new_array(block, elements.span());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewPrimitiveArray: {
        auto const& op = static_cast<Bytecode::Op::NewPrimitiveArray const&>(instruction);
        Vector<Value*> elements;
        for (auto value : op.elements())
            elements.append(&m_function->create_constant(value));
        auto& result = m_function->build_new_array(block, elements.span());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewArrayWithLength: {
        auto const& op = static_cast<Bytecode::Op::NewArrayWithLength const&>(instruction);
        auto& length = get_or_create_value_for_operand(op.array_length(), block);
        auto& result = m_function->build_new_array_with_length(block, length);
        define_operand(op.dst(), result, block);
        break;
    }
    case NewObjectWithNoPrototype: {
        auto const& op = static_cast<Bytecode::Op::NewObjectWithNoPrototype const&>(instruction);
        auto& result = m_function->build_new_object(block);
        define_operand(op.dst(), result, block);
        break;
    }
    case NewFunction: {
        auto const& op = static_cast<Bytecode::Op::NewFunction const&>(instruction);
        Value* home_object = nullptr;
        if (op.home_object().has_value())
            home_object = &get_or_create_value_for_operand(op.home_object().value(), block);
        auto& result = m_function->build_new_function(block, home_object);
        result.defining_instruction()->set_function_node(&op.function_node());
        result.defining_instruction()->set_lhs_name(op.lhs_name());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewRegExp: {
        auto const& op = static_cast<Bytecode::Op::NewRegExp const&>(instruction);
        auto& result = m_function->build_new_regexp(block, op.source_index(), op.flags_index(), op.regex_index());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewTypeError: {
        auto const& op = static_cast<Bytecode::Op::NewTypeError const&>(instruction);
        auto& result = m_function->build_new_type_error(block, op.error_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewClass: {
        auto const& op = static_cast<Bytecode::Op::NewClass const&>(instruction);
        Value* super_class = nullptr;
        if (op.super_class().has_value())
            super_class = &get_or_create_value_for_operand(op.super_class().value(), block);
        Vector<Value*> element_keys;
        for (size_t i = 0; i < op.element_keys_count(); ++i) {
            if (op.element_keys()[i].has_value())
                element_keys.append(&get_or_create_value_for_operand(op.element_keys()[i].value(), block));
            else
                element_keys.append(nullptr);
        }
        auto& result = m_function->build_new_class(block, super_class, element_keys.span());
        result.defining_instruction()->set_class_expression(&op.class_expression());
        result.defining_instruction()->set_lhs_name(op.lhs_name());
        define_operand(op.dst(), result, block);
        break;
    }

    // Calls
    case Call: {
        auto const& op = static_cast<Bytecode::Op::Call const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_function->build_call(block, callee, this_value, args.span(), op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallBuiltin: {
        auto const& op = static_cast<Bytecode::Op::CallBuiltin const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_function->build_call_builtin(block, callee, this_value, args.span(), op.builtin(), op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallConstruct: {
        auto const& op = static_cast<Bytecode::Op::CallConstruct const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_function->build_construct(block, callee, args.span(), op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        auto& result = m_function->build_call_with_argument_array(block, callee, this_value, args_array, op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallConstructWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallConstructWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        auto& result = m_function->build_construct_with_argument_array(block, callee, this_value, args_array, op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallDirectEval: {
        auto const& op = static_cast<Bytecode::Op::CallDirectEval const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_function->build_call_direct_eval(block, callee, this_value, args.span(), op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallDirectEvalWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallDirectEvalWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        Vector<Value*> args { &args_array };
        auto& result = m_function->build_call_direct_eval(block, callee, this_value, args.span(), op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }

    // Additional property access
    case GetLength: {
        auto const& op = static_cast<Bytecode::Op::GetLength const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_get_length(block, base, op.base_identifier());
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case GetLengthWithThis: {
        auto const& op = static_cast<Bytecode::Op::GetLengthWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_get_length(block, base);
        define_operand(op.dst(), result, block);
        break;
    }
    case GetMethod: {
        auto const& op = static_cast<Bytecode::Op::GetMethod const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        auto& result = m_function->build_get_by_id(block, object, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetNewTarget: {
        auto const& op = static_cast<Bytecode::Op::GetNewTarget const&>(instruction);
        auto& result = m_function->build_get_new_target(block);
        define_operand(op.dst(), result, block);
        break;
    }
    case GetCalleeAndThisFromEnvironment: {
        auto const& op = static_cast<Bytecode::Op::GetCalleeAndThisFromEnvironment const&>(instruction);
        // This instruction produces a tuple of (callee, this_value)
        auto& tuple = m_function->build_get_callee_and_this_from_environment(block, op.identifier());
        auto& callee = m_function->build_extract_value(block, tuple, 0);
        auto& this_value = m_function->build_extract_value(block, tuple, 1);
        define_operand(op.callee(), callee, block);
        define_operand(op.this_value(), this_value, block);
        break;
    }
    case ResolveThisBinding: {
        m_function->build_resolve_this_binding(block);
        break;
    }
    case GetObjectPropertyIterator: {
        auto const& op = static_cast<Bytecode::Op::GetObjectPropertyIterator const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        auto& tuple = m_function->build_get_object_property_iterator(block, object);
        auto& iterator_object = m_function->build_extract_value(block, tuple, 0);
        auto& iterator_next = m_function->build_extract_value(block, tuple, 1);
        auto& iterator_done = m_function->build_extract_value(block, tuple, 2);
        define_operand(op.dst_iterator_object(), iterator_object, block);
        define_operand(op.dst_iterator_next(), iterator_next, block);
        define_operand(op.dst_iterator_done(), iterator_done, block);
        break;
    }

    // Iterators
    // NB: Iterator ops use tuple extraction. GetIterator produces a 3-element tuple
    // (iterator_object, iterator_next, iterator_done) and we use ExtractValue to
    // extract each component for the corresponding bytecode destinations.
    case GetIterator: {
        auto const& op = static_cast<Bytecode::Op::GetIterator const&>(instruction);
        auto& iterable = get_or_create_value_for_operand(op.iterable(), block);
        auto& tuple = m_function->build_get_iterator(block, iterable);
        tuple.defining_instruction()->set_iterator_hint(op.hint());
        auto& iterator_object = m_function->build_extract_value(block, tuple, 0);
        auto& iterator_next = m_function->build_extract_value(block, tuple, 1);
        auto& iterator_done = m_function->build_extract_value(block, tuple, 2);
        define_operand(op.dst_iterator_object(), iterator_object, block);
        define_operand(op.dst_iterator_next(), iterator_next, block);
        define_operand(op.dst_iterator_done(), iterator_done, block);
        break;
    }
    case IteratorNext: {
        auto const& op = static_cast<Bytecode::Op::IteratorNext const&>(instruction);
        auto& iterator_object = get_or_create_value_for_operand(op.iterator_object(), block);
        auto& iterator_next = get_or_create_value_for_operand(op.iterator_next(), block);
        auto& iterator_done = get_or_create_value_for_operand(op.iterator_done(), block);
        auto& result = m_function->build_iterator_next(block, iterator_object);
        result.defining_instruction()->add_operand(&iterator_next);
        result.defining_instruction()->add_operand(&iterator_done);
        define_operand(op.dst(), result, block);
        break;
    }
    case IteratorNextUnpack: {
        auto const& op = static_cast<Bytecode::Op::IteratorNextUnpack const&>(instruction);
        auto& iterator_object = get_or_create_value_for_operand(op.iterator_object(), block);
        auto& iterator_next = get_or_create_value_for_operand(op.iterator_next(), block);
        auto& iterator_done = get_or_create_value_for_operand(op.iterator_done(), block);
        auto& tuple = m_function->build_iterator_next_unpack(block, iterator_object);
        tuple.defining_instruction()->add_operand(&iterator_next);
        tuple.defining_instruction()->add_operand(&iterator_done);
        auto& value = m_function->build_extract_value(block, tuple, 0);
        auto& done = m_function->build_extract_value(block, tuple, 1);
        define_operand(op.dst_value(), value, block);
        define_operand(op.dst_done(), done, block);
        break;
    }
    case IteratorClose: {
        auto const& op = static_cast<Bytecode::Op::IteratorClose const&>(instruction);
        auto& iterator_object = get_or_create_value_for_operand(op.iterator_object(), block);
        auto& iterator_next = get_or_create_value_for_operand(op.iterator_next(), block);
        auto& iterator_done = get_or_create_value_for_operand(op.iterator_done(), block);
        m_function->build_iterator_close(block, iterator_object);
        // NB: Add iterator_next and iterator_done as operands even though they're not used by IR.
        // This preserves data flow for lowering back to bytecode.
        auto* instr = block.instructions().last().ptr();
        instr->add_operand(&iterator_next);
        instr->add_operand(&iterator_done);
        break;
    }
    case IteratorToArray: {
        auto const& op = static_cast<Bytecode::Op::IteratorToArray const&>(instruction);
        auto& iterator_object = get_or_create_value_for_operand(op.iterator_object(), block);
        auto& iterator_next = get_or_create_value_for_operand(op.iterator_next_method(), block);
        auto& iterator_done = get_or_create_value_for_operand(op.iterator_done_property(), block);
        auto& result = m_function->build_iterator_to_array(block, iterator_object);
        result.defining_instruction()->add_operand(&iterator_next);
        result.defining_instruction()->add_operand(&iterator_done);
        define_operand(op.dst(), result, block);
        break;
    }

    // Environment creation ops
    case CreateVariable: {
        auto const& op = static_cast<Bytecode::Op::CreateVariable const&>(instruction);
        m_function->build_create_variable(block, op.identifier(), op.mode(), op.is_immutable(), op.is_global(), op.is_strict());
        break;
    }
    case CreateLexicalEnvironment: {
        auto const& op = static_cast<Bytecode::Op::CreateLexicalEnvironment const&>(instruction);
        auto& result = m_function->build_create_lexical_environment(block, op.capacity());
        if (op.dst().has_value())
            define_operand(*op.dst(), result, block);
        break;
    }
    case CreateMutableBinding: {
        auto const& op = static_cast<Bytecode::Op::CreateMutableBinding const&>(instruction);
        auto& env = get_or_create_value_for_operand(op.environment(), block);
        m_function->build_create_mutable_binding(block, env, op.identifier(), op.can_be_deleted());
        break;
    }
    case CreateImmutableBinding: {
        auto const& op = static_cast<Bytecode::Op::CreateImmutableBinding const&>(instruction);
        auto& env = get_or_create_value_for_operand(op.environment(), block);
        m_function->build_create_immutable_binding(block, env, op.identifier(), op.strict_binding());
        break;
    }
    case LeaveLexicalEnvironment:
        m_function->build_leave_lexical_environment(block);
        break;
    case EnterObjectEnvironment: {
        auto const& op = static_cast<Bytecode::Op::EnterObjectEnvironment const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        m_function->build_enter_object_environment(block, object);
        break;
    }
    case CreateVariableEnvironment: {
        auto const& op = static_cast<Bytecode::Op::CreateVariableEnvironment const&>(instruction);
        m_function->build_create_variable_environment(block, op.capacity());
        break;
    }
    case CreatePrivateEnvironment:
        m_function->build_create_private_environment(block);
        break;
    case LeavePrivateEnvironment:
        m_function->build_leave_private_environment(block);
        break;

    // Exception handling
    case Catch: {
        auto const& op = static_cast<Bytecode::Op::Catch const&>(instruction);
        auto& result = m_function->build_catch(block);
        define_operand(op.dst(), result, block);
        break;
    }
    case LeaveUnwindContext:
        m_function->build_leave_unwind_context(block);
        break;
    case LeaveFinally:
        m_function->build_leave_finally(block);
        break;
    case RestoreScheduledJump:
        m_function->build_restore_scheduled_jump(block);
        break;

    // Terminators handled in connect_control_flow()
    case EnterUnwindContext:
    case ContinuePendingUnwind:
    case ScheduleJump:
        break;

    // Throw guard ops
    case ThrowIfNotObject: {
        auto const& op = static_cast<Bytecode::Op::ThrowIfNotObject const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_throw_if_not_object(block, value);
        break;
    }
    case ThrowIfNullish: {
        auto const& op = static_cast<Bytecode::Op::ThrowIfNullish const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_throw_if_nullish(block, value);
        break;
    }
    case ThrowIfTDZ: {
        auto const& op = static_cast<Bytecode::Op::ThrowIfTDZ const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_throw_if_tdz(block, value);
        break;
    }

    // Array operations
    case ArrayAppend: {
        auto const& op = static_cast<Bytecode::Op::ArrayAppend const&>(instruction);
        auto& array = get_or_create_value_for_operand(op.dst(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_array_append(block, array, value, op.is_spread());
        break;
    }

    // Object spread/rest
    case CopyObjectExcludingProperties: {
        auto const& op = static_cast<Bytecode::Op::CopyObjectExcludingProperties const&>(instruction);
        auto& from = get_or_create_value_for_operand(op.from_object(), block);
        Vector<IR::Value*> excluded_names;
        for (auto const& excluded_name : op.excluded_names())
            excluded_names.append(&get_or_create_value_for_operand(excluded_name, block));
        auto& result = m_function->build_copy_object_excluding_properties(block, from, excluded_names.span());
        define_operand(op.dst(), result, block);
        break;
    }

    // Arguments and rest params
    case CreateArguments: {
        auto const& op = static_cast<Bytecode::Op::CreateArguments const&>(instruction);
        auto& result = m_function->build_create_arguments(block, op.kind(), op.is_immutable());
        if (op.dst().has_value())
            define_operand(op.dst().value(), result, block);
        break;
    }
    case CreateRestParams: {
        auto const& op = static_cast<Bytecode::Op::CreateRestParams const&>(instruction);
        auto& result = m_function->build_create_rest_params(block, op.rest_index());
        define_operand(op.dst(), result, block);
        break;
    }

    // Module/Import related
    case GetImportMeta: {
        auto const& op = static_cast<Bytecode::Op::GetImportMeta const&>(instruction);
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }
    case ImportCall: {
        auto const& op = static_cast<Bytecode::Op::ImportCall const&>(instruction);
        auto& specifier = get_or_create_value_for_operand(op.specifier(), block);
        auto& options = get_or_create_value_for_operand(op.options(), block);
        auto& result = m_function->build_import_call(block, specifier, options);
        define_operand(op.dst(), result, block);
        break;
    }
    case GetTemplateObject: {
        auto const& op = static_cast<Bytecode::Op::GetTemplateObject const&>(instruction);
        Vector<Value*> strings;
        for (auto operand : op.strings())
            strings.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_function->build_get_template_object(block, strings.span(), op.cache_index());
        define_operand(op.dst(), result, block);
        break;
    }

    // Private fields
    case GetPrivateById: {
        auto const& op = static_cast<Bytecode::Op::GetPrivateById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_get_private_by_id(block, base, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case PutPrivateById: {
        auto const& op = static_cast<Bytecode::Op::PutPrivateById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_private_by_id(block, base, op.property(), value);
        break;
    }
    case HasPrivateId: {
        auto const& op = static_cast<Bytecode::Op::HasPrivateId const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_has_private_id(block, base, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case AddPrivateName: {
        auto const& op = static_cast<Bytecode::Op::AddPrivateName const&>(instruction);
        m_function->build_add_private_name(block, op.name());
        break;
    }

    // Super
    case ResolveSuperBase: {
        auto const& op = static_cast<Bytecode::Op::ResolveSuperBase const&>(instruction);
        auto& result = m_function->build_resolve_super_base(block);
        define_operand(op.dst(), result, block);
        break;
    }
    case SuperCallWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::SuperCallWithArgumentArray const&>(instruction);
        auto& arguments = get_or_create_value_for_operand(op.arguments(), block);
        auto& result = m_function->build_super_call_with_argument_array(block, arguments, op.is_synthetic());
        define_operand(op.dst(), result, block);
        break;
    }

    // Async/Await/Yield (terminators with results - handled in connect_control_flow)
    case Await:
    case Yield:
        // These are terminators that also define a result (the resume value)
        // They're handled in connect_control_flow() where we have block context
        break;
    case PrepareYield: {
        auto const& op = static_cast<Bytecode::Op::PrepareYield const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_function->build_move(block, value);
        define_operand(op.dest(), result, block);
        break;
    }
    case CreateAsyncFromSyncIterator: {
        auto const& op = static_cast<Bytecode::Op::CreateAsyncFromSyncIterator const&>(instruction);
        auto& iterator = get_or_create_value_for_operand(op.iterator(), block);
        // NB: CreateAsyncFromSyncIterator wraps a sync iterator. We use a move as a placeholder
        // since the actual transformation happens at runtime.
        auto& result = m_function->build_move(block, iterator);
        define_operand(op.dst(), result, block);
        break;
    }
    case AsyncIteratorClose: {
        auto const& op = static_cast<Bytecode::Op::AsyncIteratorClose const&>(instruction);
        auto& iterator_object = get_or_create_value_for_operand(op.iterator_object(), block);
        auto& iterator_next = get_or_create_value_for_operand(op.iterator_next(), block);
        auto& iterator_done = get_or_create_value_for_operand(op.iterator_done(), block);
        m_function->build_async_iterator_close(block, iterator_object);
        // NB: Add iterator_next and iterator_done as operands even though they're not used by IR.
        // This preserves data flow for lowering back to bytecode.
        auto* instr = block.instructions().last().ptr();
        instr->add_operand(&iterator_next);
        instr->add_operand(&iterator_done);
        break;
    }

    // Type checks
    case IsCallable: {
        auto const& op = static_cast<Bytecode::Op::IsCallable const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_function->build_move(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case IsConstructor: {
        auto const& op = static_cast<Bytecode::Op::IsConstructor const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_function->build_move(block, src);
        define_operand(op.dst(), result, block);
        break;
    }

    // Completion tracking (for generators and finally blocks)
    case GetCompletionFields: {
        auto const& op = static_cast<Bytecode::Op::GetCompletionFields const&>(instruction);
        auto& completion = get_or_create_value_for_operand(op.completion(), block);
        auto& tuple = m_function->build_get_completion_fields(block, completion);
        auto& type_value = m_function->build_extract_value(block, tuple, 0);
        auto& value_value = m_function->build_extract_value(block, tuple, 1);
        define_operand(op.type_dst(), type_value, block);
        define_operand(op.value_dst(), value_value, block);
        break;
    }
    case SetCompletionType: {
        auto const& op = static_cast<Bytecode::Op::SetCompletionType const&>(instruction);
        auto& completion = get_or_create_value_for_operand(op.completion(), block);
        m_function->build_set_completion_type(block, completion, op.completion_type());
        break;
    }
    case CacheObjectShape: {
        auto const& op = static_cast<Bytecode::Op::CacheObjectShape const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        m_function->build_cache_object_shape(block, object, CacheIndex { op.cache_index() });
        break;
    }
    }
    // NB: No default case - all bytecode opcodes must be explicitly handled above.
    // This ensures new opcodes cause a compile error rather than being silently skipped.
}

u32 Lifter::address_to_block_index(size_t address) const
{
    // Binary search for the block containing this address.
    // basic_block_start_offsets is sorted, so we find the last offset <= address.
    auto const& offsets = m_executable.basic_block_start_offsets;
    VERIFY(!offsets.is_empty());

    size_t nearby_index = 0;
    if (binary_search(offsets, address, &nearby_index)) {
        // Exact match
        return static_cast<u32>(nearby_index);
    }

    // No exact match - nearby_index points to the closest element.
    // If the element at nearby_index is > address, step back to previous block.
    if (nearby_index > 0 && offsets[nearby_index] > address)
        --nearby_index;

    return static_cast<u32>(nearby_index);
}

void Lifter::connect_control_flow()
{
    // Second pass through instructions to connect control flow edges
    // NB: We use m_final_ir_block which points to the last IR block for each bytecode
    // block (after any EH splits). This ensures terminators are added to the correct
    // block even when the original block was split at may-throw instructions.
    for (size_t block_index = 0; block_index < m_executable.basic_block_start_offsets.size(); ++block_index) {
        auto& ir_block = *m_final_ir_block.get(static_cast<u32>(block_index)).value();

        // If block is already terminated, skip
        if (ir_block.is_terminated())
            continue;

        // Restore this block's definitions so get_or_create_value_for_operand works correctly
        auto block_defs = m_block_definitions.get(&ir_block);
        if (block_defs.has_value())
            m_current_definitions = *block_defs;

        size_t start_offset = m_executable.basic_block_start_offsets[block_index];
        size_t end_offset = (block_index + 1 < m_executable.basic_block_start_offsets.size())
            ? m_executable.basic_block_start_offsets[block_index + 1]
            : m_executable.bytecode.size();

        auto bytecode_span = ReadonlyBytes { m_executable.bytecode.data() + start_offset, end_offset - start_offset };
        Bytecode::InstructionStreamIterator it(bytecode_span, &m_executable);

        // Find the last instruction (the terminator)
        Bytecode::Instruction const* last_instruction = nullptr;
        while (!it.at_end()) {
            last_instruction = &*it;
            ++it;
        }

        if (!last_instruction)
            continue;

        using enum Bytecode::Instruction::Type;
        switch (last_instruction->type()) {
        case Jump: {
            auto const& op = static_cast<Bytecode::Op::Jump const&>(*last_instruction);
            auto* target = m_block_map.get(address_to_block_index(op.target().address())).value();
            m_function->build_jump(ir_block, *target);
            break;
        }
        case JumpIf: {
            auto const& op = static_cast<Bytecode::Op::JumpIf const&>(*last_instruction);
            auto& condition = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpTrue: {
            auto const& op = static_cast<Bytecode::Op::JumpTrue const&>(*last_instruction);
            auto& condition = get_or_create_value_for_operand(op.condition(), ir_block);
            // JumpTrue only has one target - we need to find the fallthrough
            auto* target = m_block_map.get(address_to_block_index(op.target().address())).value();
            // Fallthrough to next block
            if (block_index + 1 < m_executable.basic_block_start_offsets.size()) {
                auto* fallthrough = m_block_map.get(static_cast<u32>(block_index + 1)).value();
                m_function->build_branch(ir_block, condition, *target, *fallthrough);
            } else {
                // No fallthrough, just jump
                m_function->build_jump(ir_block, *target);
            }
            break;
        }
        case JumpFalse: {
            auto const& op = static_cast<Bytecode::Op::JumpFalse const&>(*last_instruction);
            auto& condition = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* target = m_block_map.get(address_to_block_index(op.target().address())).value();
            // Fallthrough to next block
            if (block_index + 1 < m_executable.basic_block_start_offsets.size()) {
                auto* fallthrough = m_block_map.get(static_cast<u32>(block_index + 1)).value();
                m_function->build_branch(ir_block, condition, *fallthrough, *target);
            } else {
                // Negate and jump
                auto& negated = m_function->build_not(ir_block, condition);
                m_function->build_branch(ir_block, negated, *target, *target);
            }
            break;
        }

        // Optimized comparison jumps
        case JumpGreaterThan: {
            auto const& op = static_cast<Bytecode::Op::JumpGreaterThan const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = m_function->build_greater_than(ir_block, lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpGreaterThanEquals: {
            auto const& op = static_cast<Bytecode::Op::JumpGreaterThanEquals const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = m_function->build_greater_than_equals(ir_block, lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpLessThan: {
            auto const& op = static_cast<Bytecode::Op::JumpLessThan const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = m_function->build_less_than(ir_block, lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpLessThanEquals: {
            auto const& op = static_cast<Bytecode::Op::JumpLessThanEquals const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = m_function->build_less_than_equals(ir_block, lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpLooselyEquals: {
            auto const& op = static_cast<Bytecode::Op::JumpLooselyEquals const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = m_function->build_loosely_equals(ir_block, lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpLooselyInequals: {
            auto const& op = static_cast<Bytecode::Op::JumpLooselyInequals const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = m_function->build_loosely_inequals(ir_block, lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpStrictlyEquals: {
            auto const& op = static_cast<Bytecode::Op::JumpStrictlyEquals const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = m_function->build_strictly_equals(ir_block, lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpStrictlyInequals: {
            auto const& op = static_cast<Bytecode::Op::JumpStrictlyInequals const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = m_function->build_strictly_inequals(ir_block, lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpNullish: {
            auto const& op = static_cast<Bytecode::Op::JumpNullish const&>(*last_instruction);
            auto& value = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            // JumpNullish jumps to true_target if value is null or undefined
            auto& is_nullish = m_function->build_is_nullish(ir_block, value);
            m_function->build_branch(ir_block, is_nullish, *true_target, *false_target);
            break;
        }
        case JumpUndefined: {
            auto const& op = static_cast<Bytecode::Op::JumpUndefined const&>(*last_instruction);
            auto& value = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            // JumpUndefined jumps to true_target if value is undefined
            auto& is_undef = m_function->build_is_undefined(ir_block, value);
            m_function->build_branch(ir_block, is_undef, *true_target, *false_target);
            break;
        }

        // Generators/Async - terminators with result (the resume value)
        case Yield: {
            auto const& op = static_cast<Bytecode::Op::Yield const&>(*last_instruction);
            auto& value = get_or_create_value_for_operand(op.value(), ir_block);
            if (op.continuation_label().has_value()) {
                auto cont_block_index = address_to_block_index(op.continuation_label()->address());
                auto* continuation = m_block_map.get(cont_block_index).value();
                // Build the Yield instruction (creates a new result value internally)
                auto& auto_resume_value = m_function->build_yield(ir_block, value, continuation);

                // Replace the auto-created result with our pre-created resume value
                // The pre-created one is what the continuation block's instructions are using
                if (auto pre_created = m_continuation_resume_values.get(cont_block_index); pre_created.has_value()) {
                    auto* yield_instr = auto_resume_value.defining_instruction();
                    yield_instr->set_result(*pre_created);
                }
            } else {
                // Final yield (return from generator) - emit Yield without continuation
                // Result is intentionally unused since there's no continuation
                (void)m_function->build_yield(ir_block, value, nullptr);
            }
            break;
        }
        case Await: {
            auto const& op = static_cast<Bytecode::Op::Await const&>(*last_instruction);
            auto& argument = get_or_create_value_for_operand(op.argument(), ir_block);
            auto cont_block_index = address_to_block_index(op.continuation_label().address());
            auto* continuation = m_block_map.get(cont_block_index).value();
            // Build the Await instruction (creates a new result value internally)
            auto& auto_resume_value = m_function->build_await(ir_block, argument, *continuation);

            // Replace the auto-created result with our pre-created resume value
            if (auto pre_created = m_continuation_resume_values.get(cont_block_index); pre_created.has_value()) {
                auto* await_instr = auto_resume_value.defining_instruction();
                await_instr->set_result(*pre_created);
            }
            break;
        }

        case EnterUnwindContext: {
            auto const& op = static_cast<Bytecode::Op::EnterUnwindContext const&>(*last_instruction);
            auto* target = m_block_map.get(address_to_block_index(op.entry_point().address())).value();
            m_function->build_enter_unwind_context(ir_block, *target);
            break;
        }
        case ScheduleJump: {
            auto const& op = static_cast<Bytecode::Op::ScheduleJump const&>(*last_instruction);
            auto* deferred_target = m_block_map.get(address_to_block_index(op.target().address())).value();
            auto handlers = m_executable.exception_handlers_for_offset(start_offset);
            VERIFY(handlers.has_value() && handlers->finalizer_offset.has_value());
            auto* finalizer = m_block_map.get(address_to_block_index(handlers->finalizer_offset.value())).value();
            m_function->build_schedule_jump(ir_block, *finalizer, *deferred_target);
            break;
        }
        case ContinuePendingUnwind: {
            auto const& op = static_cast<Bytecode::Op::ContinuePendingUnwind const&>(*last_instruction);
            auto* target = m_block_map.get(address_to_block_index(op.resume_target().address())).value();
            m_function->build_continue_pending_unwind(ir_block, *target);
            break;
        }

        default:
            // If not terminated by a known terminator, fall through to next block
            if (block_index + 1 < m_executable.basic_block_start_offsets.size()) {
                auto* next_block = m_block_map.get(static_cast<u32>(block_index + 1)).value();
                m_function->build_jump(ir_block, *next_block);
            }
            break;
        }

        // Save updated definitions (terminators may read operands like parameters)
        m_block_definitions.set(&ir_block, m_current_definitions);
    }
}

void Lifter::compute_block_predecessors()
{
    // Clear existing predecessor data (this may be called more than once,
    // e.g. after inserting a new entry block).
    m_predecessors.clear();
    for (auto& block : m_function->basic_blocks())
        block->clear_predecessors();

    // Build predecessor lists by examining each block's terminator
    for (auto& block : m_function->basic_blocks()) {
        auto* term = block->terminator();
        if (!term)
            continue;
        if (term->true_target()) {
            m_predecessors.ensure(term->true_target()).append(block.ptr());
            CFG::add_predecessor(*term->true_target(), *block);
        }
        if (term->false_target() && term->false_target() != term->true_target()) {
            m_predecessors.ensure(term->false_target()).append(block.ptr());
            CFG::add_predecessor(*term->false_target(), *block);
        }
    }

    // Add exception edges: if a block has throwing instructions and an exception
    // handler/finalizer, add edges so phi placement accounts for exception flow.
    // Blocks without throwing instructions have their exception handlers stripped
    // since they can never actually reach the handler.
    for (auto& block : m_function->basic_blocks()) {
        bool has_throwing_instr = false;
        for (auto const& instr : block->instructions()) {
            if (may_throw_opcode(instr->opcode())) {
                has_throwing_instr = true;
                break;
            }
        }

        // NB: ScheduleJump needs the finalizer annotation preserved because
        //     the runtime handler finds the finalizer via exception_handlers_for_offset.
        bool needs_eh_annotations = has_throwing_instr;
        if (auto* term = block->terminator(); term && term->opcode() == Opcode::ScheduleJump)
            needs_eh_annotations = true;

        if (needs_eh_annotations) {
            if (auto* handler = block->exception_handler()) {
                if (!m_predecessors.ensure(handler).contains_slow(block.ptr())) {
                    m_predecessors.ensure(handler).append(block.ptr());
                    CFG::add_predecessor(*handler, *block);
                }
            }
            if (auto* finalizer = block->finalizer()) {
                if (!m_predecessors.ensure(finalizer).contains_slow(block.ptr())) {
                    m_predecessors.ensure(finalizer).append(block.ptr());
                    CFG::add_predecessor(*finalizer, *block);
                }
            }
        } else {
            block->set_exception_handler(nullptr);
            block->set_finalizer(nullptr);
        }
    }
}

void Lifter::compute_dominators()
{
    m_dominators = make<Dominators>(*m_function);
}

void Lifter::eliminate_unreachable_blocks()
{
    // Remove blocks that are not reachable from the entry block.
    // A block is reachable if it has an immediate dominator or is the entry.
    auto* entry = m_function->entry_block();

    // Clear operand use chains before removing blocks, so values defined
    // in reachable blocks don't retain stale use entries.
    for (auto& block : m_function->basic_blocks()) {
        if (block.ptr() == entry)
            continue;
        if (m_dominators->immediate_dominator(block.ptr()))
            continue;
        for (auto& instruction : block->instructions())
            instruction->clear_operand_uses();
    }

    // Collect unreachable blocks so we can clear stale pointers.
    HashTable<BasicBlock*> unreachable;
    for (auto& block : m_function->basic_blocks()) {
        if (block.ptr() == entry)
            continue;
        if (m_dominators->immediate_dominator(block.ptr()))
            continue;
        unreachable.set(block.ptr());
    }

    if (unreachable.is_empty())
        return;

    // Clear exception_handler/finalizer pointers that reference removed blocks.
    for (auto& block : m_function->basic_blocks()) {
        if (unreachable.contains(block.ptr()))
            continue;
        if (block->exception_handler() && unreachable.contains(block->exception_handler()))
            block->set_exception_handler(nullptr);
        if (block->finalizer() && unreachable.contains(block->finalizer()))
            block->set_finalizer(nullptr);
    }

    m_function->basic_blocks().remove_all_matching([&](auto const& block) {
        return unreachable.contains(block.ptr());
    });

    compute_block_predecessors();
}

// Phase 1: Place phis at dominance frontiers of defining blocks
// This implements the standard SSA phi placement algorithm from Cytron et al.
void Lifter::place_phi_nodes()
{
    // NB: Sort written operands for deterministic phi ordering across runs.
    //     HashTable iteration order depends on capacity, which varies between
    //     allocators (e.g. system malloc vs ASAN), causing different phi
    //     numbering in release vs sanitizer builds.
    Vector<u32> sorted_operands;
    sorted_operands.ensure_capacity(m_written_operands.size());
    for (auto raw : m_written_operands)
        sorted_operands.append(raw);
    quick_sort(sorted_operands);

    // For each written operand, compute where phis are needed
    for (auto raw : sorted_operands) {
        // Find all blocks that actually define this operand
        HashTable<BasicBlock*> def_blocks;
        for (auto& [block, defs] : m_block_actual_definitions) {
            if (defs.contains(raw))
                def_blocks.set(block);
        }

        // Compute iterated dominance frontier (where phis are needed)
        HashTable<BasicBlock*> phi_blocks;
        Vector<BasicBlock*> worklist;
        for (auto* block : def_blocks)
            worklist.append(block);

        while (!worklist.is_empty()) {
            auto* block = worklist.take_last();
            for (auto* frontier_block : m_dominators->dominance_frontier(block)) {
                if (!phi_blocks.contains(frontier_block)) {
                    phi_blocks.set(frontier_block);
                    // If this block doesn't already define the variable, add to worklist
                    // (the phi itself is a definition that extends the frontier)
                    if (!def_blocks.contains(frontier_block))
                        worklist.append(frontier_block);
                }
            }
        }

        // Place phis at the computed locations.
        // NB: Sort by block index for deterministic phi ordering across runs,
        //     since phi_blocks is a HashTable with pointer-based ordering.
        Vector<BasicBlock*> sorted_phi_blocks;
        sorted_phi_blocks.ensure_capacity(phi_blocks.size());
        for (auto* block : phi_blocks)
            sorted_phi_blocks.append(block);
        quick_sort(sorted_phi_blocks, [](auto* a, auto* b) { return a->index() < b->index(); });

        for (auto* block : sorted_phi_blocks) {
            auto preds = m_predecessors.get(block);
            if (!preds.has_value() || preds->is_empty())
                continue;

            // Create an empty phi (we'll fill operands in phase 2)
            Vector<Value*> empty_values;
            Vector<BasicBlock*> empty_blocks;
            for (size_t i = 0; i < preds->size(); ++i) {
                empty_values.append(nullptr);
                empty_blocks.append((*preds)[i]);
            }

            auto& phi = m_function->build_phi(*block, empty_values, empty_blocks);
            m_value_to_operand_raw.set(&phi, raw);

            // Update m_block_definitions to include the phi value, UNLESS the block
            // has an actual definition that would override it. This ensures successors
            // inherit the correct value.
            auto actual_defs = m_block_actual_definitions.get(block);
            if (!actual_defs.has_value() || !actual_defs->contains(raw)) {
                m_block_definitions.ensure(block).set(raw, &phi);
            }
        }
    }
}

// Phase 2: Fill in phi operands and rename uses using dominator tree walk
// This implements standard SSA renaming from Cytron et al.
void Lifter::fill_phi_operands()
{
    // NB: We intentionally start with empty stacks. The standard SSA renaming
    // algorithm (Cytron et al.) builds up stacks during the dominator tree walk
    // by pushing definitions as they are encountered. Seeding stacks with
    // end-of-block definitions would cause early uses to be incorrectly rewritten
    // to later definitions within the same block.
    HashMap<u32, Vector<Value*>> operand_stacks;

    // Seed stacks with parameter values for arguments that may be reassigned.
    // Function parameters are implicit definitions at the entry block, but since
    // they aren't instruction results, the SSA renaming won't encounter them as
    // definitions. We must seed them so that reaching definitions through paths
    // that don't reassign the argument see the original parameter value instead
    // of undefined.
    for (auto* param : m_function->parameters()) {
        u32 raw = m_executable.argument_index_base + param->parameter_index();
        if (m_written_operands.contains(raw))
            operand_stacks.ensure(raw).append(param);
    }

    // Walk dominator tree starting from entry block
    if (m_function->entry_block())
        rename_ssa(*m_function->entry_block(), operand_stacks);

    // Compute phi types by joining incoming value types.
    for (auto& block : m_function->basic_blocks()) {
        for (auto& instruction : block->instructions()) {
            if (instruction->opcode() != Opcode::Phi)
                break;

            auto const& operands = instruction->operands();
            if (operands.is_empty())
                continue;

            Type phi_type = Type::Unknown;
            bool first = true;

            for (auto* operand : operands) {
                if (!operand)
                    continue;

                if (first) {
                    phi_type = operand->type();
                    first = false;
                } else {
                    phi_type = join_types(phi_type, operand->type());
                }
            }

            if (phi_type != Type::Unknown)
                instruction->result()->set_type(phi_type);

            // Re-derive result types for users whose types depend on operand
            // types, since the Phi's type may have widened after these
            // instructions were created (e.g. int32 → number).
            for (auto* user : instruction->result()->uses())
                user->recompute_result_type();
        }
    }
}

// Iterative SSA renaming using dominator tree walk
void Lifter::rename_ssa(BasicBlock& start_block, HashMap<u32, Vector<Value*>>& stacks)
{
    struct WorkItem {
        BasicBlock* block;
        bool is_restore { false };
        HashMap<u32, size_t> entry_sizes;
    };

    Vector<WorkItem> work_stack;
    work_stack.empend(&start_block, false, HashMap<u32, size_t> {});

    while (!work_stack.is_empty()) {
        auto item = move(work_stack.last());
        work_stack.take_last();

        if (item.is_restore) {
            // Restore stack sizes (pop what we pushed in this block)
            for (auto& [op_raw, target_size] : item.entry_sizes) {
                auto& stack = stacks.ensure(op_raw);
                while (stack.size() > target_size)
                    stack.take_last();
            }
            continue;
        }

        auto& block = *item.block;

        // Record stack sizes at entry so we can restore them on exit
        HashMap<u32, size_t> entry_sizes;
        for (auto& [op_raw, stack] : stacks) {
            entry_sizes.set(op_raw, stack.size());
        }

        // Process phis first - they define values at block entry
        for (auto& instruction : block.instructions()) {
            if (instruction->opcode() != Opcode::Phi)
                break;

            auto raw_opt = m_value_to_operand_raw.get(instruction->result());
            if (raw_opt.has_value()) {
                stacks.ensure(*raw_opt).append(instruction->result());
                if (!entry_sizes.contains(*raw_opt))
                    entry_sizes.set(*raw_opt, 0);
            }
        }

        // Rewrite operand uses in non-phi instructions and push new definitions
        for (auto& instruction : block.instructions()) {
            if (instruction->opcode() == Opcode::Phi)
                continue;

            // Rewrite operand uses to current stack top
            for (size_t i = 0; i < instruction->operands().size(); ++i) {
                auto* operand_value = instruction->operands()[i];
                if (!operand_value)
                    continue;

                auto raw_opt = m_value_to_operand_raw.get(operand_value);
                if (!raw_opt.has_value())
                    continue;

                auto stack_opt = stacks.get(*raw_opt);
                if (stack_opt.has_value() && !stack_opt->is_empty()) {
                    auto* current = stack_opt->last();
                    if (current != operand_value)
                        instruction->set_operand(i, current);
                } else {
                    // No reaching definition: register was never written on this path.
                    // Replace with undefined (JavaScript default for uninitialized registers).
                    auto& undef = m_function->create_constant(js_undefined());
                    instruction->set_operand(i, &undef);
                }
            }

            // If instruction defines a value, push it onto the stack
            if (instruction->result()) {
                auto raw_opt = m_value_to_operand_raw.get(instruction->result());
                if (raw_opt.has_value()) {
                    stacks.ensure(*raw_opt).append(instruction->result());
                    if (!entry_sizes.contains(*raw_opt))
                        entry_sizes.set(*raw_opt, 0);
                }
            }
        }

        // Fill phi operands in CFG successors
        auto fill_phi_for_successor = [&](BasicBlock* succ) {
            if (!succ)
                return;

            // Find our index in the successor's predecessor list
            size_t pred_index = SIZE_MAX;
            auto const& phi_preds = succ->predecessors();
            for (size_t i = 0; i < phi_preds.size(); ++i) {
                if (phi_preds[i] == &block) {
                    pred_index = i;
                    break;
                }
            }
            if (pred_index == SIZE_MAX)
                return;

            // Fill phi operands for this predecessor
            for (auto& instruction : succ->instructions()) {
                if (instruction->opcode() != Opcode::Phi)
                    break;

                auto& phi = static_cast<PhiInstruction&>(*instruction);
                auto raw_opt = m_value_to_operand_raw.get(phi.result());
                if (!raw_opt.has_value())
                    continue;

                // Get current value from stack
                auto stack_opt = stacks.get(*raw_opt);
                Value* reaching = nullptr;
                if (stack_opt.has_value() && !stack_opt->is_empty()) {
                    reaching = stack_opt->last();
                } else {
                    // No definition reaches here - use undefined
                    reaching = &m_function->create_constant(JS::js_undefined());
                }

                // Find the correct phi operand index and set the value
                for (size_t i = 0; i < phi.incoming_count(); ++i) {
                    if (phi.incoming_block(i) == &block) {
                        phi.set_incoming_value(i, reaching);
                        break;
                    }
                }
            }
        };

        // Fill phis for all CFG successors
        if (auto* term = block.terminator()) {
            fill_phi_for_successor(term->true_target());
            if (term->false_target() && term->false_target() != term->true_target())
                fill_phi_for_successor(term->false_target());
        }
        // Also fill phis for exception edges
        fill_phi_for_successor(block.exception_handler());
        if (block.finalizer() != block.exception_handler())
            fill_phi_for_successor(block.finalizer());

        // Push restore item (will be processed after all children)
        work_stack.empend(&block, true, move(entry_sizes));

        // Push dominated children in reverse order so first child is processed first
        auto const& children = m_dominators->dominator_children(&block);
        for (int i = static_cast<int>(children.size()) - 1; i >= 0; --i) {
            work_stack.empend(children[i], false, HashMap<u32, size_t> {});
        }
    }
}

}
