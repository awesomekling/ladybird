/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/BinarySearch.h>
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

static inline size_t to_index(BlockIndex b) { return static_cast<u32>(b); }
static inline size_t to_index(ValueIndex v) { return static_cast<u32>(v); }

template<typename T>
static inline void ensure_index(Vector<T>& vec, size_t index)
{
    if (index >= vec.size()) {
        vec.ensure_capacity(max(index + 1, vec.size() * 2));
        vec.resize(index + 1);
    }
}

Lifter::Lifter(Bytecode::Executable const& executable)
    : m_executable(executable)
    , m_function(Function::create(&executable))
    , m_builder(*m_function)
{
}

LiftResult Lifter::lift(Bytecode::Executable const& executable)
{
    // IR Pipeline Phase 1: Bytecode → IR (CFG construction)
    Lifter lifter(executable);
    lifter.lift_basic_blocks();
    lifter.connect_control_flow();
    lifter.compute_block_predecessors();

    // SSA requires the entry block to have no predecessors.
    if (auto* entry = lifter.m_function->entry_block(); entry && !entry->predecessor_indices().is_empty()) {
        auto& new_entry = lifter.m_function->create_block("entry"_string);
        lifter.m_builder.set_insertion_block(&new_entry);
        lifter.m_builder.build_jump(*entry);
        lifter.m_function->set_entry_block(&new_entry);
        lifter.compute_block_predecessors();
    }

    lifter.compute_dominators();
    lifter.eliminate_unreachable_blocks();

    // Package up SSA side tables for the SSAConstructionPass to consume later.
    SsaConstructionData ssa_data;
    ssa_data.written_operands = move(lifter.m_written_operands);
    ssa_data.block_actual_definitions = move(lifter.m_block_actual_definitions);
    ssa_data.block_definitions = move(lifter.m_block_definitions);
    ssa_data.value_to_operand_raw = move(lifter.m_value_to_operand_raw);

    // Store the source block map for exception handler remapping in the lowerer
    lifter.m_function->set_source_block_map(move(lifter.m_block_map));

    return { move(lifter.m_function), move(ssa_data) };
}

void Lifter::lift_basic_blocks()
{
    // Pre-create Parameter values for all formal parameters.
    // This ensures SSA construction always seeds them on the operand stack,
    // even if the bytecode writes to a parameter slot before reading it
    // in linear block order. Without this, SSA renaming would resolve
    // unseen parameters to undefined on paths where the write doesn't dominate.
    for (u32 i = 0; i < m_executable.formal_parameter_count; ++i) {
        auto& parameter = m_function->create_parameter(i);
        auto vi = to_index(parameter.index());
        ensure_index(m_value_to_operand_raw, vi);
        m_value_to_operand_raw[vi] = m_executable.argument_index_base + i;
    }

    // First pass: create IR basic blocks for each bytecode basic block
    for (size_t i = 0; i < m_executable.basic_block_start_offsets.size(); ++i) {
        auto& block = m_function->create_block(String::formatted("block{}", i).release_value_but_fixme_should_propagate_errors());
        m_block_map.set(static_cast<u32>(i), &block);
        if (i == 0)
            m_function->set_entry_block(&block);
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
        current_block->set_exception_handler(exception_handler ? Optional<BlockIndex>(exception_handler->index()) : Optional<BlockIndex>());
        current_block->set_finalizer(finalizer ? Optional<BlockIndex>(finalizer->index()) : Optional<BlockIndex>());

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

            m_builder.set_insertion_block(current_block);
            lift_instruction(*it, *current_block);
            ++it;

            // Attach source record to all IR instructions created from this bytecode instruction
            if (source_entry) {
                for (size_t i = instr_count_before; i < current_block->instructions().size(); ++i)
                    m_function->instruction_by_index(current_block->instructions()[i])->set_source_record(source_entry->source_record);
            }

            // Check if we added any may-throw instructions
            bool added_may_throw = false;
            for (size_t i = instr_count_before; i < current_block->instructions().size(); ++i) {
                if (may_throw_opcode(m_function->instruction_by_index(current_block->instructions()[i])->opcode())) {
                    added_may_throw = true;
                    break;
                }
            }

            // If we added a may-throw instruction and there are more bytecode instructions,
            // split the block to ensure the exception edge has correct reaching definitions.
            // This way, values defined after the throw point won't incorrectly flow to handlers.
            if (added_may_throw && !it.at_end()) {
                // Save current block's definitions (this is the state at the throw point)
                auto bi = to_index(current_block->index());
                ensure_index(m_block_definitions, bi);
                m_block_definitions[bi] = m_current_definitions;

                // Create continuation block for remaining instructions
                auto& continuation = m_function->create_block(
                    String::formatted("block{}_split{}", block_index, split_counter++).release_value_but_fixme_should_propagate_errors());

                // Continuation inherits exception handlers
                continuation.set_exception_handler(exception_handler ? Optional<BlockIndex>(exception_handler->index()) : Optional<BlockIndex>());
                continuation.set_finalizer(finalizer ? Optional<BlockIndex>(finalizer->index()) : Optional<BlockIndex>());

                // Emit fallthrough jump from current block to continuation
                m_builder.set_insertion_block(current_block);
                m_builder.build_jump(continuation);

                // Continue lifting into continuation block
                current_block = &continuation;
            }
        }

        // Save final block's definitions (snapshot at end of block)
        {
            auto bi = to_index(current_block->index());
            ensure_index(m_block_definitions, bi);
            m_block_definitions[bi] = m_current_definitions;
        }

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
        // For arguments, create a parameter value to preserve the argument index.
        // NB: Also register in m_value_to_operand_raw so that SSA renaming can
        // replace uses with the reaching definition when the argument is reassigned.
        value = &m_function->create_parameter(decoded_operand.index());
        auto vi = to_index(value->index());
        ensure_index(m_value_to_operand_raw, vi);
        m_value_to_operand_raw[vi] = raw;
    } else if (decoded_operand.is_register() && decoded_operand.index() == Bytecode::Register::this_value().index()) {
        // For the this register, create a special this value
        value = &m_function->create_this();
    } else {
        // For registers/locals, create a register value
        // NB: In full SSA, phi nodes would be inserted at merge points
        value = &m_function->create_register_value();
        // NB: Register values need the operand mapping so SSA renaming can replace
        // them with the proper reaching definition.
        auto vi = to_index(value->index());
        ensure_index(m_value_to_operand_raw, vi);
        m_value_to_operand_raw[vi] = raw;
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
    auto bi = to_index(block.index());
    ensure_index(m_block_actual_definitions, bi);
    m_block_actual_definitions[bi].set(raw);
    auto vi = to_index(value.index());
    ensure_index(m_value_to_operand_raw, vi);
    m_value_to_operand_raw[vi] = raw;
}

template<typename BytecodeOp>
void Lifter::lift_binary_op(Bytecode::Instruction const& instruction, BasicBlock& block, Value& (Builder::*build_fn)(Value&, Value&))
{
    auto const& op = static_cast<BytecodeOp const&>(instruction);
    auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
    auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
    auto& result = (m_builder.*build_fn)(lhs, rhs);
    define_operand(op.dst(), result, block);
}

template<typename BytecodeOp>
void Lifter::lift_unary_op_src(Bytecode::Instruction const& instruction, BasicBlock& block, Value& (Builder::*build_fn)(Value&))
{
    auto const& op = static_cast<BytecodeOp const&>(instruction);
    auto& src = get_or_create_value_for_operand(op.src(), block);
    auto& result = (m_builder.*build_fn)(src);
    define_operand(op.dst(), result, block);
}

template<typename BytecodeOp>
void Lifter::lift_unary_op_value(Bytecode::Instruction const& instruction, BasicBlock& block, Value& (Builder::*build_fn)(Value&))
{
    auto const& op = static_cast<BytecodeOp const&>(instruction);
    auto& src = get_or_create_value_for_operand(op.value(), block);
    auto& result = (m_builder.*build_fn)(src);
    define_operand(op.dst(), result, block);
}

void Lifter::lift_instruction(Bytecode::Instruction const& instruction, BasicBlock& block)
{
    using enum Bytecode::Instruction::Type;

    switch (instruction.type()) {
        // Binary ops: bytecode (dst, lhs, rhs) -> IR binary instruction
#define LIFT_BINARY(BcName, BuildFn)                                                 \
    case BcName:                                                                     \
        lift_binary_op<Bytecode::Op::BcName>(instruction, block, &Builder::BuildFn); \
        break;
        LIFT_BINARY(Add, build_add)
        LIFT_BINARY(Sub, build_sub)
        LIFT_BINARY(Mul, build_mul)
        LIFT_BINARY(Div, build_div)
        LIFT_BINARY(Mod, build_mod)
        LIFT_BINARY(Exp, build_exp)
        LIFT_BINARY(BitwiseAnd, build_bitwise_and)
        LIFT_BINARY(BitwiseOr, build_bitwise_or)
        LIFT_BINARY(BitwiseXor, build_bitwise_xor)
        LIFT_BINARY(LeftShift, build_left_shift)
        LIFT_BINARY(RightShift, build_right_shift)
        LIFT_BINARY(UnsignedRightShift, build_unsigned_right_shift)
        LIFT_BINARY(LessThan, build_less_than)
        LIFT_BINARY(LessThanEquals, build_less_than_equals)
        LIFT_BINARY(GreaterThan, build_greater_than)
        LIFT_BINARY(GreaterThanEquals, build_greater_than_equals)
        LIFT_BINARY(LooselyEquals, build_loosely_equals)
        LIFT_BINARY(StrictlyEquals, build_strictly_equals)
        LIFT_BINARY(LooselyInequals, build_loosely_inequals)
        LIFT_BINARY(StrictlyInequals, build_strictly_inequals)
#undef LIFT_BINARY

        // Unary ops with src() accessor
#define LIFT_UNARY_SRC(BcName, BuildFn)                                                 \
    case BcName:                                                                        \
        lift_unary_op_src<Bytecode::Op::BcName>(instruction, block, &Builder::BuildFn); \
        break;
        LIFT_UNARY_SRC(BitwiseNot, build_bitwise_not)
        LIFT_UNARY_SRC(UnaryMinus, build_negate)
        LIFT_UNARY_SRC(UnaryPlus, build_unary_plus)
        LIFT_UNARY_SRC(Not, build_not)
        LIFT_UNARY_SRC(Typeof, build_typeof)
#undef LIFT_UNARY_SRC

        // Unary ops with value() accessor
#define LIFT_UNARY_VALUE(BcName, BuildFn)                                                 \
    case BcName:                                                                          \
        lift_unary_op_value<Bytecode::Op::BcName>(instruction, block, &Builder::BuildFn); \
        break;
        LIFT_UNARY_VALUE(ToBoolean, build_to_boolean)
        LIFT_UNARY_VALUE(ToObject, build_to_object)
        LIFT_UNARY_VALUE(ToString, build_to_string)
        LIFT_UNARY_VALUE(ToInt32, build_to_int32)
        LIFT_UNARY_VALUE(ToLength, build_to_length)
        LIFT_UNARY_VALUE(ToNumeric, build_to_numeric)
#undef LIFT_UNARY_VALUE
    case TypeofBinding: {
        auto const& op = static_cast<Bytecode::Op::TypeofBinding const&>(instruction);
        auto& result = m_builder.build_typeof_binding(op.identifier());
        define_operand(op.dst(), result, block);
        break;
    }

    // Increment/Decrement
    case Increment: {
        auto const& op = static_cast<Bytecode::Op::Increment const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.dst(), block);
        auto& result = m_builder.build_increment(src);
        define_operand(op.dst(), result, block);
        break;
    }
    case Decrement: {
        auto const& op = static_cast<Bytecode::Op::Decrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.dst(), block);
        auto& result = m_builder.build_decrement(src);
        define_operand(op.dst(), result, block);
        break;
    }
    case PostfixIncrement: {
        auto const& op = static_cast<Bytecode::Op::PostfixIncrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        // dst gets the OLD NUMERIC value (PostfixIncrement returns ToNumeric(src))
        auto& old_value = m_builder.build_to_numeric(src);
        define_operand(op.dst(), old_value, block);
        // src gets MUTATED to src + 1 - create a new SSA value for it
        auto& incremented = m_builder.build_increment(old_value);
        define_operand(op.src(), incremented, block);
        break;
    }
    case PostfixDecrement: {
        auto const& op = static_cast<Bytecode::Op::PostfixDecrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        // dst gets the OLD NUMERIC value (PostfixDecrement returns ToNumeric(src))
        auto& old_value = m_builder.build_to_numeric(src);
        define_operand(op.dst(), old_value, block);
        // src gets MUTATED to src - 1 - create a new SSA value for it
        auto& decremented = m_builder.build_decrement(old_value);
        define_operand(op.src(), decremented, block);
        break;
    }

    // String ops
    case ConcatString: {
        auto const& op = static_cast<Bytecode::Op::ConcatString const&>(instruction);
        auto& dst = get_or_create_value_for_operand(op.dst(), block);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_builder.build_concat_string(dst, src);
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
            m_builder.build_set_saved_return_value(src);
            break;
        }
        if (op.dst().is_register() && op.dst().index() == Bytecode::Register::exception_index) {
            // Mov to exception register: use SetException to write physical reg2.
            auto& src = get_or_create_value_for_operand(op.src(), block);
            m_builder.build_set_exception(src);
            break;
        }
        if (op.src().is_register() && op.src().index() == Bytecode::Register::exception_index) {
            // Mov from exception register: use GetException to read physical reg2.
            auto& result = m_builder.build_get_exception();
            define_operand(op.dst(), result, block);
            break;
        }
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_builder.build_move(src);
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
        m_builder.build_return(value);
        break;
    }

    case Throw: {
        auto const& op = static_cast<Bytecode::Op::Throw const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_throw(value);
        break;
    }

    case End: {
        auto const& op = static_cast<Bytecode::Op::End const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.value(), block);
        m_builder.build_end(value);
        break;
    }

    // Property access
    case GetById: {
        auto const& op = static_cast<Bytecode::Op::GetById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_builder.build_get_by_id(base, op.property(), op.base_identifier());
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case GetByValue: {
        auto const& op = static_cast<Bytecode::Op::GetByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_builder.build_get_by_value(base, property, op.base_identifier());
        define_operand(op.dst(), result, block);
        break;
    }
    case PutNormalById: {
        auto const& op = static_cast<Bytecode::Op::PutNormalById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_id(base, op.property(), value, op.base_identifier());
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutNormalByValue: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_value(base, property, value, op.base_identifier());
        break;
    }

    // WithThis property access variants
    case GetByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::GetByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& result = m_builder.build_get_by_id_with_this(base, this_value, op.property());
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case GetByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::GetByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_builder.build_get_by_value_with_this(base, this_value, property);
        define_operand(op.dst(), result, block);
        break;
    }
    case PutNormalByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_id_with_this(base, this_value, op.property(), value);
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutNormalByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_value_with_this(base, this_value, property, value);
        break;
    }
    case DeleteByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::DeleteByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& result = m_builder.build_delete_by_id_with_this(base, this_value, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case DeleteByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::DeleteByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_builder.build_delete_by_value_with_this(base, this_value, property);
        define_operand(op.dst(), result, block);
        break;
    }

    case DeleteById: {
        auto const& op = static_cast<Bytecode::Op::DeleteById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_builder.build_delete_by_id(base, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case DeleteByValue: {
        auto const& op = static_cast<Bytecode::Op::DeleteByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_builder.build_delete_by_value(base, property);
        define_operand(op.dst(), result, block);
        break;
    }

    // Other Put variants (for object literals, classes, etc.)
    case PutOwnById: {
        auto const& op = static_cast<Bytecode::Op::PutOwnById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_id(base, op.property(), value, op.base_identifier(), Bytecode::PutKind::Own);
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutOwnByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutOwnByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_id_with_this(base, this_value, op.property(), value, Bytecode::PutKind::Own);
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutOwnByValue: {
        auto const& op = static_cast<Bytecode::Op::PutOwnByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_value(base, property, value, op.base_identifier(), Bytecode::PutKind::Own);
        break;
    }
    case PutOwnByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutOwnByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_value_with_this(base, this_value, property, value, Bytecode::PutKind::Own);
        break;
    }
    case InitObjectLiteralProperty: {
        auto const& op = static_cast<Bytecode::Op::InitObjectLiteralProperty const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_init_object_literal_property(object, op.property(), value, CacheIndex { op.shape_cache_index() }, PropertySlot { op.property_slot() });
        break;
    }
    case CreateDataPropertyOrThrow: {
        auto const& op = static_cast<Bytecode::Op::CreateDataPropertyOrThrow const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.object(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.value(), block);
        m_builder.build_put_by_value(base, property, value, {}, Bytecode::PutKind::Own);
        break;
    }

    // Getters/setters/prototypes
    case PutGetterById: {
        auto const& op = static_cast<Bytecode::Op::PutGetterById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& getter = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_getter_by_id(base, op.property(), getter, op.base_identifier());
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutSetterById: {
        auto const& op = static_cast<Bytecode::Op::PutSetterById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& setter = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_setter_by_id(base, op.property(), setter, op.base_identifier());
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutPrototypeById: {
        auto const& op = static_cast<Bytecode::Op::PutPrototypeById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& prototype = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_prototype_by_id(base, op.property(), prototype, op.base_identifier());
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutGetterByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutGetterByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& getter = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_getter_by_id_with_this(base, this_value, op.property(), getter);
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutSetterByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutSetterByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& setter = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_setter_by_id_with_this(base, this_value, op.property(), setter);
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutPrototypeByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutPrototypeByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& prototype = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_prototype_by_id_with_this(base, this_value, op.property(), prototype);
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }
    case PutGetterByValue: {
        auto const& op = static_cast<Bytecode::Op::PutGetterByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& getter = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_getter_by_value(base, property, getter, op.base_identifier());
        break;
    }
    case PutSetterByValue: {
        auto const& op = static_cast<Bytecode::Op::PutSetterByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& setter = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_setter_by_value(base, property, setter, op.base_identifier());
        break;
    }
    case PutPrototypeByValue: {
        auto const& op = static_cast<Bytecode::Op::PutPrototypeByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& prototype = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_prototype_by_value(base, property, prototype, op.base_identifier());
        break;
    }
    case PutGetterByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutGetterByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& getter = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_getter_by_value_with_this(base, property, this_value, getter);
        break;
    }
    case PutSetterByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutSetterByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& setter = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_setter_by_value_with_this(base, property, this_value, setter);
        break;
    }
    case PutPrototypeByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutPrototypeByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& prototype = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_prototype_by_value_with_this(base, property, this_value, prototype);
        break;
    }
    case PutBySpread: {
        auto const& op = static_cast<Bytecode::Op::PutBySpread const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& source = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_by_spread(base, source);
        break;
    }

    // In/InstanceOf
    case In:
        lift_binary_op<Bytecode::Op::In>(instruction, block, &Builder::build_in);
        break;
    case InstanceOf:
        lift_binary_op<Bytecode::Op::InstanceOf>(instruction, block, &Builder::build_instance_of);
        break;

    // Environment
    case GetBinding: {
        auto const& op = static_cast<Bytecode::Op::GetBinding const&>(instruction);
        auto& result = m_builder.build_get_binding(op.identifier());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetInitializedBinding: {
        auto const& op = static_cast<Bytecode::Op::GetInitializedBinding const&>(instruction);
        auto& result = m_builder.build_get_binding(op.identifier());
        define_operand(op.dst(), result, block);
        break;
    }
    case SetLexicalBinding: {
        auto const& op = static_cast<Bytecode::Op::SetLexicalBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_set_binding(op.identifier(), value, Bytecode::Op::EnvironmentMode::Lexical);
        break;
    }
    case SetVariableBinding: {
        auto const& op = static_cast<Bytecode::Op::SetVariableBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_set_binding(op.identifier(), value, Bytecode::Op::EnvironmentMode::Var);
        break;
    }
    case InitializeLexicalBinding: {
        auto const& op = static_cast<Bytecode::Op::InitializeLexicalBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_initialize_binding(op.identifier(), value, Bytecode::Op::EnvironmentMode::Lexical);
        break;
    }
    case InitializeVariableBinding: {
        auto const& op = static_cast<Bytecode::Op::InitializeVariableBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_initialize_binding(op.identifier(), value, Bytecode::Op::EnvironmentMode::Var);
        break;
    }
    case DeleteVariable: {
        auto const& op = static_cast<Bytecode::Op::DeleteVariable const&>(instruction);
        auto& result = m_builder.build_delete_variable(op.identifier());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetGlobal: {
        auto const& op = static_cast<Bytecode::Op::GetGlobal const&>(instruction);
        auto& result = m_builder.build_get_global(op.identifier());
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case SetGlobal: {
        auto const& op = static_cast<Bytecode::Op::SetGlobal const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_set_global(op.identifier(), value);
        m_function->instruction_by_index(block.instructions().last())->set_cache_index(CacheIndex { op.cache_index() });
        break;
    }

    // Object creation
    case NewObject: {
        auto const& op = static_cast<Bytecode::Op::NewObject const&>(instruction);
        auto& result = m_builder.build_new_object();
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case NewArray: {
        auto const& op = static_cast<Bytecode::Op::NewArray const&>(instruction);
        Vector<Value*> elements;
        for (auto operand : op.elements())
            elements.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_builder.build_new_array(elements.span());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewPrimitiveArray: {
        auto const& op = static_cast<Bytecode::Op::NewPrimitiveArray const&>(instruction);
        Vector<Value*> elements;
        for (auto value : op.elements())
            elements.append(&m_function->create_constant(value));
        auto& result = m_builder.build_new_array(elements.span());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewArrayWithLength: {
        auto const& op = static_cast<Bytecode::Op::NewArrayWithLength const&>(instruction);
        auto& length = get_or_create_value_for_operand(op.array_length(), block);
        auto& result = m_builder.build_new_array_with_length(length);
        define_operand(op.dst(), result, block);
        break;
    }
    case NewObjectWithNoPrototype: {
        auto const& op = static_cast<Bytecode::Op::NewObjectWithNoPrototype const&>(instruction);
        auto& result = m_builder.build_new_object_with_no_prototype();
        define_operand(op.dst(), result, block);
        break;
    }
    case NewFunction: {
        auto const& op = static_cast<Bytecode::Op::NewFunction const&>(instruction);
        Value* home_object = nullptr;
        if (op.home_object().has_value())
            home_object = &get_or_create_value_for_operand(op.home_object().value(), block);
        auto& result = m_builder.build_new_function(home_object);
        result.defining_instruction()->set_function_node(&op.function_node());
        result.defining_instruction()->set_lhs_name(op.lhs_name());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewRegExp: {
        auto const& op = static_cast<Bytecode::Op::NewRegExp const&>(instruction);
        auto& result = m_builder.build_new_regexp(op.source_index(), op.flags_index(), op.regex_index());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewTypeError: {
        auto const& op = static_cast<Bytecode::Op::NewTypeError const&>(instruction);
        auto& result = m_builder.build_new_type_error(op.error_string());
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
        auto& result = m_builder.build_new_class(super_class, element_keys.span());
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
        auto& result = m_builder.build_call(callee, this_value, args.span(), op.expression_string());
        result.defining_instruction()->set_cache_index(CacheIndex { op.call_target_profile_index() });
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
        auto& result = m_builder.build_call_builtin(callee, this_value, args.span(), op.builtin(), op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallConstruct: {
        auto const& op = static_cast<Bytecode::Op::CallConstruct const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_builder.build_construct(callee, args.span(), op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        auto& result = m_builder.build_call_with_argument_array(callee, this_value, args_array, op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallConstructWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallConstructWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        auto& result = m_builder.build_construct_with_argument_array(callee, this_value, args_array, op.expression_string());
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
        auto& result = m_builder.build_call_direct_eval(callee, this_value, args.span(), op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallDirectEvalWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallDirectEvalWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        auto& result = m_builder.build_call_direct_eval_with_argument_array(callee, this_value, args_array, op.expression_string());
        define_operand(op.dst(), result, block);
        break;
    }

    // Additional property access
    case GetLength: {
        auto const& op = static_cast<Bytecode::Op::GetLength const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_builder.build_get_length(base, op.base_identifier());
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case GetLengthWithThis: {
        auto const& op = static_cast<Bytecode::Op::GetLengthWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& result = m_builder.build_get_length_with_this(base, this_value);
        result.defining_instruction()->set_cache_index(CacheIndex { op.cache_index() });
        define_operand(op.dst(), result, block);
        break;
    }
    case GetMethod: {
        auto const& op = static_cast<Bytecode::Op::GetMethod const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        auto& result = m_builder.build_get_method(object, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetNewTarget: {
        auto const& op = static_cast<Bytecode::Op::GetNewTarget const&>(instruction);
        auto& result = m_builder.build_get_new_target();
        define_operand(op.dst(), result, block);
        break;
    }
    case GetCalleeAndThisFromEnvironment: {
        auto const& op = static_cast<Bytecode::Op::GetCalleeAndThisFromEnvironment const&>(instruction);
        // This instruction produces a tuple of (callee, this_value)
        auto& tuple = m_builder.build_get_callee_and_this_from_environment(op.identifier());
        auto& callee = m_builder.build_extract_value(tuple, 0);
        auto& this_value = m_builder.build_extract_value(tuple, 1);
        define_operand(op.callee(), callee, block);
        define_operand(op.this_value(), this_value, block);
        break;
    }
    case ResolveThisBinding: {
        m_builder.build_resolve_this_binding();
        break;
    }
    case GetObjectPropertyIterator: {
        auto const& op = static_cast<Bytecode::Op::GetObjectPropertyIterator const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        auto& tuple = m_builder.build_get_object_property_iterator(object);
        auto& iterator_object = m_builder.build_extract_value(tuple, 0);
        auto& iterator_next = m_builder.build_extract_value(tuple, 1);
        auto& iterator_done = m_builder.build_extract_value(tuple, 2);
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
        auto& tuple = m_builder.build_get_iterator(iterable);
        tuple.defining_instruction()->set_iterator_hint(op.hint());
        auto& iterator_object = m_builder.build_extract_value(tuple, 0);
        auto& iterator_next = m_builder.build_extract_value(tuple, 1);
        auto& iterator_done = m_builder.build_extract_value(tuple, 2);
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
        auto& result = m_builder.build_iterator_next(iterator_object);
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
        auto& tuple = m_builder.build_iterator_next_unpack(iterator_object);
        tuple.defining_instruction()->add_operand(&iterator_next);
        tuple.defining_instruction()->add_operand(&iterator_done);
        auto& value = m_builder.build_extract_value(tuple, 0);
        auto& done = m_builder.build_extract_value(tuple, 1);
        define_operand(op.dst_value(), value, block);
        define_operand(op.dst_done(), done, block);
        break;
    }
    case IteratorClose: {
        auto const& op = static_cast<Bytecode::Op::IteratorClose const&>(instruction);
        auto& iterator_object = get_or_create_value_for_operand(op.iterator_object(), block);
        auto& iterator_next = get_or_create_value_for_operand(op.iterator_next(), block);
        auto& iterator_done = get_or_create_value_for_operand(op.iterator_done(), block);
        m_builder.build_iterator_close(iterator_object);
        // NB: Add iterator_next and iterator_done as operands even though they're not used by IR.
        // This preserves data flow for lowering back to bytecode.
        auto* last_instruction = m_function->instruction_by_index(block.instructions().last());
        last_instruction->add_operand(&iterator_next);
        last_instruction->add_operand(&iterator_done);
        break;
    }
    case IteratorToArray: {
        auto const& op = static_cast<Bytecode::Op::IteratorToArray const&>(instruction);
        auto& iterator_object = get_or_create_value_for_operand(op.iterator_object(), block);
        auto& iterator_next = get_or_create_value_for_operand(op.iterator_next_method(), block);
        auto& iterator_done = get_or_create_value_for_operand(op.iterator_done_property(), block);
        auto& result = m_builder.build_iterator_to_array(iterator_object);
        result.defining_instruction()->add_operand(&iterator_next);
        result.defining_instruction()->add_operand(&iterator_done);
        define_operand(op.dst(), result, block);
        break;
    }

    // Environment creation ops
    case CreateVariable: {
        auto const& op = static_cast<Bytecode::Op::CreateVariable const&>(instruction);
        m_builder.build_create_variable(op.identifier(), op.mode(), op.is_immutable(), op.is_global(), op.is_strict());
        break;
    }
    case CreateLexicalEnvironment: {
        auto const& op = static_cast<Bytecode::Op::CreateLexicalEnvironment const&>(instruction);
        auto& result = m_builder.build_create_lexical_environment(op.capacity());
        if (op.dst().has_value())
            define_operand(*op.dst(), result, block);
        break;
    }
    case CreateMutableBinding: {
        auto const& op = static_cast<Bytecode::Op::CreateMutableBinding const&>(instruction);
        auto& env = get_or_create_value_for_operand(op.environment(), block);
        m_builder.build_create_mutable_binding(env, op.identifier(), op.can_be_deleted());
        break;
    }
    case CreateImmutableBinding: {
        auto const& op = static_cast<Bytecode::Op::CreateImmutableBinding const&>(instruction);
        auto& env = get_or_create_value_for_operand(op.environment(), block);
        m_builder.build_create_immutable_binding(env, op.identifier(), op.strict_binding());
        break;
    }
    case LeaveLexicalEnvironment:
        m_builder.build_leave_lexical_environment();
        break;
    case EnterObjectEnvironment: {
        auto const& op = static_cast<Bytecode::Op::EnterObjectEnvironment const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        m_builder.build_enter_object_environment(object);
        break;
    }
    case CreateVariableEnvironment: {
        auto const& op = static_cast<Bytecode::Op::CreateVariableEnvironment const&>(instruction);
        m_builder.build_create_variable_environment(op.capacity());
        break;
    }
    case CreatePrivateEnvironment:
        m_builder.build_create_private_environment();
        break;
    case LeavePrivateEnvironment:
        m_builder.build_leave_private_environment();
        break;

    // Exception handling
    case Catch: {
        auto const& op = static_cast<Bytecode::Op::Catch const&>(instruction);
        auto& result = m_builder.build_catch();
        define_operand(op.dst(), result, block);
        break;
    }
    case LeaveUnwindContext:
        m_builder.build_leave_unwind_context();
        break;
    case LeaveFinally:
        m_builder.build_leave_finally();
        break;
    case RestoreScheduledJump:
        m_builder.build_restore_scheduled_jump();
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
        m_builder.build_throw_if_not_object(value);
        break;
    }
    case ThrowIfNullish: {
        auto const& op = static_cast<Bytecode::Op::ThrowIfNullish const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_throw_if_nullish(value);
        break;
    }
    case ThrowIfTDZ: {
        auto const& op = static_cast<Bytecode::Op::ThrowIfTDZ const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_throw_if_tdz(value);
        break;
    }

    // Array operations
    case ArrayAppend: {
        auto const& op = static_cast<Bytecode::Op::ArrayAppend const&>(instruction);
        auto& array = get_or_create_value_for_operand(op.dst(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_array_append(array, value, op.is_spread());
        break;
    }

    // Object spread/rest
    case CopyObjectExcludingProperties: {
        auto const& op = static_cast<Bytecode::Op::CopyObjectExcludingProperties const&>(instruction);
        auto& from = get_or_create_value_for_operand(op.from_object(), block);
        Vector<IR::Value*> excluded_names;
        for (auto const& excluded_name : op.excluded_names())
            excluded_names.append(&get_or_create_value_for_operand(excluded_name, block));
        auto& result = m_builder.build_copy_object_excluding_properties(from, excluded_names.span());
        define_operand(op.dst(), result, block);
        break;
    }

    // Arguments and rest params
    case CreateArguments: {
        auto const& op = static_cast<Bytecode::Op::CreateArguments const&>(instruction);
        auto& result = m_builder.build_create_arguments(op.kind(), op.is_immutable(), op.dst().has_value());
        if (op.dst().has_value())
            define_operand(op.dst().value(), result, block);
        break;
    }
    case CreateRestParams: {
        auto const& op = static_cast<Bytecode::Op::CreateRestParams const&>(instruction);
        auto& result = m_builder.build_create_rest_params(op.rest_index());
        define_operand(op.dst(), result, block);
        break;
    }

    // Module/Import related
    case GetImportMeta: {
        auto const& op = static_cast<Bytecode::Op::GetImportMeta const&>(instruction);
        auto& result = m_builder.build_get_import_meta();
        define_operand(op.dst(), result, block);
        break;
    }
    case ImportCall: {
        auto const& op = static_cast<Bytecode::Op::ImportCall const&>(instruction);
        auto& specifier = get_or_create_value_for_operand(op.specifier(), block);
        auto& options = get_or_create_value_for_operand(op.options(), block);
        auto& result = m_builder.build_import_call(specifier, options);
        define_operand(op.dst(), result, block);
        break;
    }
    case GetTemplateObject: {
        auto const& op = static_cast<Bytecode::Op::GetTemplateObject const&>(instruction);
        Vector<Value*> strings;
        for (auto operand : op.strings())
            strings.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_builder.build_get_template_object(strings.span(), op.cache_index());
        define_operand(op.dst(), result, block);
        break;
    }

    // Private fields
    case GetPrivateById: {
        auto const& op = static_cast<Bytecode::Op::GetPrivateById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_builder.build_get_private_by_id(base, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case PutPrivateById: {
        auto const& op = static_cast<Bytecode::Op::PutPrivateById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_builder.build_put_private_by_id(base, op.property(), value);
        break;
    }
    case HasPrivateId: {
        auto const& op = static_cast<Bytecode::Op::HasPrivateId const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_builder.build_has_private_id(base, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case AddPrivateName: {
        auto const& op = static_cast<Bytecode::Op::AddPrivateName const&>(instruction);
        m_builder.build_add_private_name(op.name());
        break;
    }

    // Super
    case ResolveSuperBase: {
        auto const& op = static_cast<Bytecode::Op::ResolveSuperBase const&>(instruction);
        auto& result = m_builder.build_resolve_super_base();
        define_operand(op.dst(), result, block);
        break;
    }
    case SuperCallWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::SuperCallWithArgumentArray const&>(instruction);
        auto& arguments = get_or_create_value_for_operand(op.arguments(), block);
        auto& result = m_builder.build_super_call_with_argument_array(arguments, op.is_synthetic());
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
        m_builder.build_prepare_yield(value);
        break;
    }
    case CreateAsyncFromSyncIterator: {
        auto const& op = static_cast<Bytecode::Op::CreateAsyncFromSyncIterator const&>(instruction);
        auto& iterator = get_or_create_value_for_operand(op.iterator(), block);
        auto& next_method = get_or_create_value_for_operand(op.next_method(), block);
        auto& done = get_or_create_value_for_operand(op.done(), block);
        auto& result = m_builder.build_create_async_from_sync_iterator(iterator, next_method, done);
        define_operand(op.dst(), result, block);
        break;
    }
    case AsyncIteratorClose: {
        auto const& op = static_cast<Bytecode::Op::AsyncIteratorClose const&>(instruction);
        auto& iterator_object = get_or_create_value_for_operand(op.iterator_object(), block);
        auto& iterator_next = get_or_create_value_for_operand(op.iterator_next(), block);
        auto& iterator_done = get_or_create_value_for_operand(op.iterator_done(), block);
        m_builder.build_async_iterator_close(iterator_object);
        // NB: Add iterator_next and iterator_done as operands even though they're not used by IR.
        // This preserves data flow for lowering back to bytecode.
        auto* last_instruction = m_function->instruction_by_index(block.instructions().last());
        last_instruction->add_operand(&iterator_next);
        last_instruction->add_operand(&iterator_done);
        break;
    }

    // Type checks
    case IsCallable: {
        auto const& op = static_cast<Bytecode::Op::IsCallable const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_builder.build_is_callable(src);
        define_operand(op.dst(), result, block);
        break;
    }
    case IsConstructor: {
        auto const& op = static_cast<Bytecode::Op::IsConstructor const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_builder.build_is_constructor(src);
        define_operand(op.dst(), result, block);
        break;
    }
    case IsNullish: {
        auto const& op = static_cast<Bytecode::Op::IsNullish const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_builder.build_is_nullish(src);
        define_operand(op.dst(), result, block);
        break;
    }

    // Completion tracking (for generators and finally blocks)
    case GetCompletionFields: {
        auto const& op = static_cast<Bytecode::Op::GetCompletionFields const&>(instruction);
        auto& completion = get_or_create_value_for_operand(op.completion(), block);
        auto& tuple = m_builder.build_get_completion_fields(completion);
        auto& type_value = m_builder.build_extract_value(tuple, 0);
        auto& value_value = m_builder.build_extract_value(tuple, 1);
        define_operand(op.type_dst(), type_value, block);
        define_operand(op.value_dst(), value_value, block);
        break;
    }
    case SetCompletionType: {
        auto const& op = static_cast<Bytecode::Op::SetCompletionType const&>(instruction);
        auto& completion = get_or_create_value_for_operand(op.completion(), block);
        m_builder.build_set_completion_type(completion, op.completion_type());
        break;
    }
    case CacheObjectShape: {
        auto const& op = static_cast<Bytecode::Op::CacheObjectShape const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        m_builder.build_cache_object_shape(object, CacheIndex { op.cache_index() });
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

        // Set the builder's insertion block for this iteration
        m_builder.set_insertion_block(&ir_block);

        // Restore this block's definitions so get_or_create_value_for_operand works correctly
        auto bi = to_index(ir_block.index());
        if (bi < m_block_definitions.size())
            m_current_definitions = m_block_definitions[bi];

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
            m_builder.build_jump(*target);
            break;
        }
        case JumpIf: {
            auto const& op = static_cast<Bytecode::Op::JumpIf const&>(*last_instruction);
            auto& condition = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_builder.build_branch(condition, *true_target, *false_target);
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
                m_builder.build_branch(condition, *target, *fallthrough);
            } else {
                // No fallthrough, just jump
                m_builder.build_jump(*target);
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
                m_builder.build_branch(condition, *fallthrough, *target);
            } else {
                // Negate and jump
                auto& negated = m_builder.build_not(condition);
                m_builder.build_branch(negated, *target, *target);
            }
            break;
        }

        // Optimized comparison jumps — all share the same (lhs, rhs, true_target, false_target) layout.
        case JumpGreaterThan:
        case JumpGreaterThanEquals:
        case JumpLessThan:
        case JumpLessThanEquals:
        case JumpLooselyEquals:
        case JumpLooselyInequals:
        case JumpStrictlyEquals:
        case JumpStrictlyInequals: {
            using BuildFn = Value& (Builder::*)(Value&, Value&);
            BuildFn build_fn = nullptr;
            switch (last_instruction->type()) {
            case JumpGreaterThan:
                build_fn = &Builder::build_greater_than;
                break;
            case JumpGreaterThanEquals:
                build_fn = &Builder::build_greater_than_equals;
                break;
            case JumpLessThan:
                build_fn = &Builder::build_less_than;
                break;
            case JumpLessThanEquals:
                build_fn = &Builder::build_less_than_equals;
                break;
            case JumpLooselyEquals:
                build_fn = &Builder::build_loosely_equals;
                break;
            case JumpLooselyInequals:
                build_fn = &Builder::build_loosely_inequals;
                break;
            case JumpStrictlyEquals:
                build_fn = &Builder::build_strictly_equals;
                break;
            case JumpStrictlyInequals:
                build_fn = &Builder::build_strictly_inequals;
                break;
            default:
                VERIFY_NOT_REACHED();
            }
            // All comparison jump ops share the same memory layout, so we can
            // use any of them to access lhs/rhs/true_target/false_target.
            auto const& op = static_cast<Bytecode::Op::JumpGreaterThan const&>(*last_instruction);
            auto& lhs = get_or_create_value_for_operand(op.lhs(), ir_block);
            auto& rhs = get_or_create_value_for_operand(op.rhs(), ir_block);
            auto& condition = (m_builder.*build_fn)(lhs, rhs);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            m_builder.build_branch(condition, *true_target, *false_target);
            break;
        }
        case JumpNullish: {
            auto const& op = static_cast<Bytecode::Op::JumpNullish const&>(*last_instruction);
            auto& value = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            // JumpNullish jumps to true_target if value is null or undefined
            auto& is_nullish = m_builder.build_is_nullish(value);
            m_builder.build_branch(is_nullish, *true_target, *false_target);
            break;
        }
        case JumpUndefined: {
            auto const& op = static_cast<Bytecode::Op::JumpUndefined const&>(*last_instruction);
            auto& value = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            // JumpUndefined jumps to true_target if value is undefined
            auto& is_undef = m_builder.build_is_undefined(value);
            m_builder.build_branch(is_undef, *true_target, *false_target);
            break;
        }

        // Generators/Async - terminators with result (the resume value)
        case Yield: {
            auto const& op = static_cast<Bytecode::Op::Yield const&>(*last_instruction);
            auto& value = get_or_create_value_for_operand(op.value(), ir_block);
            if (op.continuation_label().has_value()) {
                auto cont_block_index = address_to_block_index(op.continuation_label()->address());
                auto* continuation = m_block_map.get(cont_block_index).value();
                auto& resume_value = m_builder.build_yield(value, continuation);
                // The resume value (passed via .next()/.throw()/.return()) arrives in the
                // accumulator (reg0). Register it so SSA construction places Phi nodes
                // when multiple Yields share a continuation block.
                define_operand(Bytecode::Operand(Bytecode::Register::accumulator()), resume_value, ir_block);
            } else {
                (void)m_builder.build_yield(value, nullptr);
            }
            break;
        }
        case Await: {
            auto const& op = static_cast<Bytecode::Op::Await const&>(*last_instruction);
            auto& argument = get_or_create_value_for_operand(op.argument(), ir_block);
            auto cont_block_index = address_to_block_index(op.continuation_label().address());
            auto* continuation = m_block_map.get(cont_block_index).value();
            auto& resume_value = m_builder.build_await(argument, *continuation);
            define_operand(Bytecode::Operand(Bytecode::Register::accumulator()), resume_value, ir_block);
            break;
        }

        case EnterUnwindContext: {
            auto const& op = static_cast<Bytecode::Op::EnterUnwindContext const&>(*last_instruction);
            auto* target = m_block_map.get(address_to_block_index(op.entry_point().address())).value();
            m_builder.build_enter_unwind_context(*target);
            break;
        }
        case ScheduleJump: {
            auto const& op = static_cast<Bytecode::Op::ScheduleJump const&>(*last_instruction);
            auto* deferred_target = m_block_map.get(address_to_block_index(op.target().address())).value();
            auto handlers = m_executable.exception_handlers_for_offset(start_offset);
            VERIFY(handlers.has_value() && handlers->finalizer_offset.has_value());
            auto* finalizer = m_block_map.get(address_to_block_index(handlers->finalizer_offset.value())).value();
            m_builder.build_schedule_jump(*finalizer, *deferred_target);
            break;
        }
        case ContinuePendingUnwind: {
            auto const& op = static_cast<Bytecode::Op::ContinuePendingUnwind const&>(*last_instruction);
            auto* target = m_block_map.get(address_to_block_index(op.resume_target().address())).value();
            m_builder.build_continue_pending_unwind(*target);
            break;
        }

        default:
            // If not terminated by a known terminator, fall through to next block
            if (block_index + 1 < m_executable.basic_block_start_offsets.size()) {
                auto* next_block = m_block_map.get(static_cast<u32>(block_index + 1)).value();
                m_builder.build_jump(*next_block);
            }
            break;
        }

        // Save updated definitions (terminators may read operands like parameters)
        {
            auto bi2 = to_index(ir_block.index());
            ensure_index(m_block_definitions, bi2);
            m_block_definitions[bi2] = m_current_definitions;
        }
    }
}

void Lifter::compute_block_predecessors()
{
    // Clear existing predecessor data (this may be called more than once,
    // e.g. after inserting a new entry block).
    for (auto& block : m_function->basic_blocks())
        block->clear_predecessors();

    // Build predecessor lists by examining each block's terminator
    for (auto& block : m_function->basic_blocks()) {
        auto* term = block->terminator();
        if (!term)
            continue;
        if (term->true_target())
            CFG::add_predecessor(*term->true_target(), *block);
        if (term->false_target() && term->false_target() != term->true_target())
            CFG::add_predecessor(*term->false_target(), *block);
    }

    // Add exception edges: if a block has throwing instructions and an exception
    // handler/finalizer, add edges so phi placement accounts for exception flow.
    // Blocks without throwing instructions have their exception handlers stripped
    // since they can never actually reach the handler.
    for (auto& block : m_function->basic_blocks()) {
        bool has_throwing_instr = false;
        for (auto instruction_index : block->instructions()) {
            if (may_throw_opcode(m_function->instruction_by_index(instruction_index)->opcode())) {
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
            if (auto* handler = block->exception_handler())
                CFG::add_predecessor(*handler, *block);
            if (auto* finalizer = block->finalizer())
                CFG::add_predecessor(*finalizer, *block);
        } else {
            block->set_exception_handler({});
            block->set_finalizer({});
        }
    }
}

void Lifter::compute_dominators()
{
    m_dominators = make<DominatorTree>(*m_function);
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
        block->for_each_instruction([](Instruction& instruction) {
            instruction.clear_operand_uses();
        });
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
            block->set_exception_handler({});
        if (block->finalizer() && unreachable.contains(block->finalizer()))
            block->set_finalizer({});
    }

    m_function->basic_blocks().remove_all_matching([&](auto const& block) {
        return unreachable.contains(block.ptr());
    });

    compute_block_predecessors();
}

}
