/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/Bytecode/Instruction.h>
#include <LibJS/Bytecode/Op.h>
#include <LibJS/Bytecode/Register.h>
#include <LibJS/IR/BasicBlock.h>
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

    // SSA construction using a two-phase approach:
    // Phase 1: Place placeholder phis at all join points for all written operands
    // Phase 2: Fill in phi operands now that all phis exist
    lifter.place_phi_nodes();
    lifter.fill_phi_operands();
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

    // Second pass: lift instructions from each basic block
    // NB: We don't clear definitions between blocks - this is a simplification
    // that lets values flow through. Proper SSA would require phi node insertion
    // at merge points with dominator-based definition resolution.
    for (size_t block_index = 0; block_index < m_executable.basic_block_start_offsets.size(); ++block_index) {
        auto& ir_block = *m_block_map.get(static_cast<u32>(block_index)).value();

        size_t start_offset = m_executable.basic_block_start_offsets[block_index];
        size_t end_offset = (block_index + 1 < m_executable.basic_block_start_offsets.size())
            ? m_executable.basic_block_start_offsets[block_index + 1]
            : m_executable.bytecode.size();

        // Set exception handler based on bytecode exception handler table
        if (auto handlers = m_executable.exception_handlers_for_offset(start_offset); handlers.has_value()) {
            if (handlers->handler_offset.has_value()) {
                auto handler_block_index = address_to_block_index(handlers->handler_offset.value());
                ir_block.set_exception_handler(m_block_map.get(handler_block_index).value());
            }
            if (handlers->finalizer_offset.has_value()) {
                auto finalizer_block_index = address_to_block_index(handlers->finalizer_offset.value());
                ir_block.set_finalizer(m_block_map.get(finalizer_block_index).value());
            }
        }

        auto bytecode_span = ReadonlyBytes { m_executable.bytecode.data() + start_offset, end_offset - start_offset };
        Bytecode::InstructionStreamIterator it(bytecode_span, &m_executable);

        while (!it.at_end()) {
            lift_instruction(*it, ir_block);
            ++it;
        }

        // Save this block's definitions (snapshot at end of block)
        m_block_definitions.set(&ir_block, m_current_definitions);
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

void Lifter::lift_instruction(Bytecode::Instruction const& instruction, BasicBlock& block)
{
    using enum Bytecode::Instruction::Type;

    switch (instruction.type()) {
    // Arithmetic binary ops
    case Add: {
        auto const& op = static_cast<Bytecode::Op::Add const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_add(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case Sub: {
        auto const& op = static_cast<Bytecode::Op::Sub const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_sub(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case Mul: {
        auto const& op = static_cast<Bytecode::Op::Mul const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_mul(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case Div: {
        auto const& op = static_cast<Bytecode::Op::Div const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_div(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case Mod: {
        auto const& op = static_cast<Bytecode::Op::Mod const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_mod(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case Exp: {
        auto const& op = static_cast<Bytecode::Op::Exp const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_exp(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }

    // Bitwise binary ops
    case BitwiseAnd: {
        auto const& op = static_cast<Bytecode::Op::BitwiseAnd const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_bitwise_and(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case BitwiseOr: {
        auto const& op = static_cast<Bytecode::Op::BitwiseOr const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_bitwise_or(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case BitwiseXor: {
        auto const& op = static_cast<Bytecode::Op::BitwiseXor const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_bitwise_xor(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case LeftShift: {
        auto const& op = static_cast<Bytecode::Op::LeftShift const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_left_shift(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case RightShift: {
        auto const& op = static_cast<Bytecode::Op::RightShift const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_right_shift(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case UnsignedRightShift: {
        auto const& op = static_cast<Bytecode::Op::UnsignedRightShift const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_unsigned_right_shift(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }

    // Comparison ops
    case LessThan: {
        auto const& op = static_cast<Bytecode::Op::LessThan const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_less_than(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case LessThanEquals: {
        auto const& op = static_cast<Bytecode::Op::LessThanEquals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_less_than_equals(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case GreaterThan: {
        auto const& op = static_cast<Bytecode::Op::GreaterThan const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_greater_than(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case GreaterThanEquals: {
        auto const& op = static_cast<Bytecode::Op::GreaterThanEquals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_greater_than_equals(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case LooselyEquals: {
        auto const& op = static_cast<Bytecode::Op::LooselyEquals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_loosely_equals(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case StrictlyEquals: {
        auto const& op = static_cast<Bytecode::Op::StrictlyEquals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_strictly_equals(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case LooselyInequals: {
        auto const& op = static_cast<Bytecode::Op::LooselyInequals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_loosely_inequals(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case StrictlyInequals: {
        auto const& op = static_cast<Bytecode::Op::StrictlyInequals const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_strictly_inequals(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }

    // Unary ops
    case BitwiseNot: {
        auto const& op = static_cast<Bytecode::Op::BitwiseNot const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_function->build_bitwise_not(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case UnaryMinus: {
        auto const& op = static_cast<Bytecode::Op::UnaryMinus const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_function->build_negate(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case UnaryPlus: {
        auto const& op = static_cast<Bytecode::Op::UnaryPlus const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_function->build_unary_plus(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case Not: {
        auto const& op = static_cast<Bytecode::Op::Not const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_function->build_not(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case Typeof: {
        auto const& op = static_cast<Bytecode::Op::Typeof const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        auto& result = m_function->build_typeof(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case ToBoolean: {
        auto const& op = static_cast<Bytecode::Op::ToBoolean const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_function->build_to_boolean(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case ToObject: {
        auto const& op = static_cast<Bytecode::Op::ToObject const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_function->build_to_object(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case ToString: {
        auto const& op = static_cast<Bytecode::Op::ToString const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_function->build_to_string(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case ToInt32: {
        auto const& op = static_cast<Bytecode::Op::ToInt32 const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_function->build_to_int32(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
    case ToLength: {
        auto const& op = static_cast<Bytecode::Op::ToLength const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.value(), block);
        auto& result = m_function->build_to_length(block, src);
        define_operand(op.dst(), result, block);
        break;
    }
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
        // dst gets the OLD value (that's what postfix returns)
        auto& old_value = m_function->build_move(block, src);
        define_operand(op.dst(), old_value, block);
        // src gets MUTATED to src + 1 - create a new SSA value for it
        auto& incremented = m_function->build_increment(block, src);
        define_operand(op.src(), incremented, block);
        break;
    }
    case PostfixDecrement: {
        auto const& op = static_cast<Bytecode::Op::PostfixDecrement const&>(instruction);
        auto& src = get_or_create_value_for_operand(op.src(), block);
        // dst gets the OLD value (that's what postfix returns)
        auto& old_value = m_function->build_move(block, src);
        define_operand(op.dst(), old_value, block);
        // src gets MUTATED to src - 1 - create a new SSA value for it
        auto& decremented = m_function->build_decrement(block, src);
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
        m_function->build_return(block, value);
        break;
    }

    // Property access
    case GetById: {
        auto const& op = static_cast<Bytecode::Op::GetById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_get_by_id(block, base, op.property());
        result.defining_instruction()->set_cache_index(op.cache_index());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetByValue: {
        auto const& op = static_cast<Bytecode::Op::GetByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_function->build_get_by_value(block, base, property);
        define_operand(op.dst(), result, block);
        break;
    }
    case PutNormalById: {
        auto const& op = static_cast<Bytecode::Op::PutNormalById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_id(block, base, op.property(), value);
        block.instructions().last()->set_cache_index(op.cache_index());
        break;
    }
    case PutNormalByValue: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByValue const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_value(block, base, property, value);
        break;
    }

    // WithThis property access variants
    case GetByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::GetByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_get_by_id(block, base, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case GetByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::GetByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_function->build_get_by_value(block, base, property);
        define_operand(op.dst(), result, block);
        break;
    }
    case PutNormalByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_id(block, base, op.property(), value);
        break;
    }
    case PutNormalByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::PutNormalByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_put_by_value(block, base, property, value);
        break;
    }
    case DeleteByIdWithThis: {
        auto const& op = static_cast<Bytecode::Op::DeleteByIdWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_delete_by_id(block, base, op.property());
        define_operand(op.dst(), result, block);
        break;
    }
    case DeleteByValueWithThis: {
        auto const& op = static_cast<Bytecode::Op::DeleteByValueWithThis const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& property = get_or_create_value_for_operand(op.property(), block);
        auto& result = m_function->build_delete_by_value(block, base, property);
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
        m_function->build_init_object_literal_property(block, object, op.property(), value, op.shape_cache_index(), op.property_slot());
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

    // Getters/setters/prototypes (just track as puts for now)
    case PutGetterById:
    case PutGetterByIdWithThis:
    case PutSetterById:
    case PutSetterByIdWithThis:
    case PutPrototypeById:
    case PutPrototypeByIdWithThis:
        // These define getters/setters/prototype - no result, just side effect
        break;
    case PutGetterByValue:
    case PutGetterByValueWithThis:
    case PutSetterByValue:
    case PutSetterByValueWithThis:
    case PutPrototypeByValue:
    case PutPrototypeByValueWithThis:
    case PutBySpread:
        // These define properties with spread - no result, just side effect
        break;

    // In/InstanceOf
    case In: {
        auto const& op = static_cast<Bytecode::Op::In const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_in(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }
    case InstanceOf: {
        auto const& op = static_cast<Bytecode::Op::InstanceOf const&>(instruction);
        auto& lhs = get_or_create_value_for_operand(op.lhs(), block);
        auto& rhs = get_or_create_value_for_operand(op.rhs(), block);
        auto& result = m_function->build_instance_of(block, lhs, rhs);
        define_operand(op.dst(), result, block);
        break;
    }

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
        m_function->build_set_binding(block, op.identifier(), value);
        break;
    }
    case SetVariableBinding: {
        auto const& op = static_cast<Bytecode::Op::SetVariableBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_set_binding(block, op.identifier(), value);
        break;
    }
    case InitializeLexicalBinding: {
        auto const& op = static_cast<Bytecode::Op::InitializeLexicalBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_initialize_binding(block, op.identifier(), value);
        break;
    }
    case InitializeVariableBinding: {
        auto const& op = static_cast<Bytecode::Op::InitializeVariableBinding const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_initialize_binding(block, op.identifier(), value);
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
        result.defining_instruction()->set_cache_index(op.cache_index());
        define_operand(op.dst(), result, block);
        break;
    }
    case SetGlobal: {
        auto const& op = static_cast<Bytecode::Op::SetGlobal const&>(instruction);
        auto& value = get_or_create_value_for_operand(op.src(), block);
        m_function->build_set_global(block, op.identifier(), value);
        block.instructions().last()->set_cache_index(op.cache_index());
        break;
    }

    // Object creation
    case NewObject: {
        auto const& op = static_cast<Bytecode::Op::NewObject const&>(instruction);
        auto& result = m_function->build_new_object(block);
        result.defining_instruction()->set_cache_index(op.cache_index());
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
        // Use build_new_array with no elements for now; length is tracked separately
        Vector<Value*> elements;
        auto& result = m_function->build_new_array(block, elements.span());
        (void)length; // TODO: Properly handle array length
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
        auto& result = m_function->build_new_function(block);
        result.defining_instruction()->set_function_node(&op.function_node());
        result.defining_instruction()->set_lhs_name(op.lhs_name());
        define_operand(op.dst(), result, block);
        break;
    }
    case NewRegExp: {
        auto const& op = static_cast<Bytecode::Op::NewRegExp const&>(instruction);
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }
    case NewTypeError: {
        auto const& op = static_cast<Bytecode::Op::NewTypeError const&>(instruction);
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }
    case NewClass: {
        auto const& op = static_cast<Bytecode::Op::NewClass const&>(instruction);
        auto& result = m_function->create_register_value();
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
        auto& result = m_function->build_call(block, callee, this_value, args.span());
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
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallConstruct: {
        auto const& op = static_cast<Bytecode::Op::CallConstruct const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        Vector<Value*> args;
        for (auto operand : op.arguments())
            args.append(&get_or_create_value_for_operand(operand, block));
        auto& result = m_function->build_construct(block, callee, args.span());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        // NB: In full implementation, we'd need to handle spreading the array
        Vector<Value*> args { &args_array };
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallConstructWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallConstructWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        Vector<Value*> args { &args_array };
        auto& result = m_function->build_construct(block, callee, args.span());
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
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        define_operand(op.dst(), result, block);
        break;
    }
    case CallDirectEvalWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::CallDirectEvalWithArgumentArray const&>(instruction);
        auto& callee = get_or_create_value_for_operand(op.callee(), block);
        auto& this_value = get_or_create_value_for_operand(op.this_value(), block);
        auto& args_array = get_or_create_value_for_operand(op.arguments(), block);
        Vector<Value*> args { &args_array };
        auto& result = m_function->build_call(block, callee, this_value, args.span());
        define_operand(op.dst(), result, block);
        break;
    }

    // Additional property access
    case GetLength: {
        auto const& op = static_cast<Bytecode::Op::GetLength const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_get_length(block, base);
        result.defining_instruction()->set_cache_index(op.cache_index());
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
        // GetNewTarget returns a special value from the execution context
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }
    case GetCalleeAndThisFromEnvironment: {
        auto const& op = static_cast<Bytecode::Op::GetCalleeAndThisFromEnvironment const&>(instruction);
        // This instruction writes to both callee and this_value operands
        auto& callee_result = m_function->build_get_binding(block, op.identifier());
        define_operand(op.callee(), callee_result, block);
        // this_value is typically undefined for function calls
        auto& this_result = m_function->create_register_value();
        define_operand(op.this_value(), this_result, block);
        break;
    }
    case ResolveThisBinding: {
        // ResolveThisBinding doesn't produce a value itself, it's a side-effect op
        break;
    }
    case GetObjectPropertyIterator: {
        auto const& op = static_cast<Bytecode::Op::GetObjectPropertyIterator const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        auto& result = m_function->build_get_iterator(block, object);
        define_operand(op.dst_iterator_object(), result, block);
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
        // NB: Add iterator_next and iterator_done as operands even though they're not used by IR
        // This preserves data flow for lowering back to bytecode.
        (void)iterator_next;
        (void)iterator_done;
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
    case CreateVariableEnvironment:
    case CreatePrivateEnvironment:
    case LeavePrivateEnvironment:
    case EnterObjectEnvironment:
        // These affect the environment but don't produce IR values
        break;

    // Exception handling
    case Catch: {
        auto const& op = static_cast<Bytecode::Op::Catch const&>(instruction);
        // Catch puts the caught exception into the destination
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }
    case EnterUnwindContext:
    case LeaveUnwindContext:
    case ContinuePendingUnwind:
    case LeaveFinally:
    case ScheduleJump:
    case RestoreScheduledJump:
        // Exception handling control flow - no IR values produced
        break;

    // Throw guard ops
    case ThrowIfNotObject:
    case ThrowIfNullish:
    case ThrowIfTDZ:
        // These are guard instructions that may throw but produce no value
        break;

    // Array operations
    case ArrayAppend: {
        auto const& op = static_cast<Bytecode::Op::ArrayAppend const&>(instruction);
        // ArrayAppend mutates dst array in place, adding src
        // Track both operands for data flow
        (void)get_or_create_value_for_operand(op.dst(), block);
        (void)get_or_create_value_for_operand(op.src(), block);
        break;
    }

    // Object spread/rest
    case CopyObjectExcludingProperties: {
        auto const& op = static_cast<Bytecode::Op::CopyObjectExcludingProperties const&>(instruction);
        auto& from = get_or_create_value_for_operand(op.from_object(), block);
        auto& result = m_function->build_move(block, from);
        define_operand(op.dst(), result, block);
        break;
    }

    // Arguments and rest params
    case CreateArguments: {
        auto const& op = static_cast<Bytecode::Op::CreateArguments const&>(instruction);
        if (op.dst().has_value()) {
            auto& result = m_function->create_register_value();
            define_operand(op.dst().value(), result, block);
        }
        break;
    }
    case CreateRestParams: {
        auto const& op = static_cast<Bytecode::Op::CreateRestParams const&>(instruction);
        auto& result = m_function->create_register_value();
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
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }
    case GetTemplateObject: {
        auto const& op = static_cast<Bytecode::Op::GetTemplateObject const&>(instruction);
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }

    // Private fields
    case GetPrivateById: {
        auto const& op = static_cast<Bytecode::Op::GetPrivateById const&>(instruction);
        auto& base = get_or_create_value_for_operand(op.base(), block);
        auto& result = m_function->build_move(block, base);
        define_operand(op.dst(), result, block);
        break;
    }
    case PutPrivateById:
    case HasPrivateId:
    case AddPrivateName:
        // Private field operations - no result value for most
        break;

    // Super
    case ResolveSuperBase: {
        auto const& op = static_cast<Bytecode::Op::ResolveSuperBase const&>(instruction);
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }
    case SuperCallWithArgumentArray: {
        auto const& op = static_cast<Bytecode::Op::SuperCallWithArgumentArray const&>(instruction);
        auto& result = m_function->create_register_value();
        define_operand(op.dst(), result, block);
        break;
    }

    // Async/Await/Yield (terminators, handled in control flow)
    case Await:
    case Yield:
    case PrepareYield:
    case CreateAsyncFromSyncIterator:
    case AsyncIteratorClose:
        // Async/generator control flow - no result value
        break;

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

    // Completion tracking (for finally blocks)
    case GetCompletionFields:
    case SetCompletionType:
        // Runtime bookkeeping - no IR values
        break;
    case CacheObjectShape: {
        auto const& op = static_cast<Bytecode::Op::CacheObjectShape const&>(instruction);
        auto& object = get_or_create_value_for_operand(op.object(), block);
        m_function->build_cache_object_shape(block, object, op.cache_index());
        break;
    }

    // TODO: Handle more opcodes as needed
    default:
        // For unhandled opcodes, we skip them for now
        // In a complete implementation, we'd handle all opcodes
        break;
    }
}

u32 Lifter::address_to_block_index(size_t address) const
{
    // Find the basic block that contains this address
    for (size_t i = 0; i < m_executable.basic_block_start_offsets.size(); ++i) {
        if (m_executable.basic_block_start_offsets[i] == address)
            return static_cast<u32>(i);
    }
    // If we didn't find an exact match, find the block that contains this address
    for (size_t i = 0; i + 1 < m_executable.basic_block_start_offsets.size(); ++i) {
        if (address >= m_executable.basic_block_start_offsets[i] && address < m_executable.basic_block_start_offsets[i + 1])
            return static_cast<u32>(i);
    }
    // Default to the last block
    return static_cast<u32>(m_executable.basic_block_start_offsets.size() - 1);
}

void Lifter::connect_control_flow()
{
    // Second pass through instructions to connect control flow edges
    for (size_t block_index = 0; block_index < m_executable.basic_block_start_offsets.size(); ++block_index) {
        auto& ir_block = *m_block_map.get(static_cast<u32>(block_index)).value();

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
            auto& condition = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            // NB: JumpNullish jumps if null or undefined - we'd need a proper IsNullish check
            // For now, treat as a branch on the condition
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
            break;
        }
        case JumpUndefined: {
            auto const& op = static_cast<Bytecode::Op::JumpUndefined const&>(*last_instruction);
            auto& condition = get_or_create_value_for_operand(op.condition(), ir_block);
            auto* true_target = m_block_map.get(address_to_block_index(op.true_target().address())).value();
            auto* false_target = m_block_map.get(address_to_block_index(op.false_target().address())).value();
            // NB: JumpUndefined jumps if undefined - we'd need a proper IsUndefined check
            // For now, treat as a branch on the condition
            m_function->build_branch(ir_block, condition, *true_target, *false_target);
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
    }
}

void Lifter::compute_block_predecessors()
{
    // Build predecessor lists by examining each block's successors
    for (auto& block : m_function->basic_blocks()) {
        for (auto& instruction : block->instructions()) {
            if (instruction->true_target())
                m_predecessors.ensure(instruction->true_target()).append(block.ptr());
            if (instruction->false_target() && instruction->false_target() != instruction->true_target())
                m_predecessors.ensure(instruction->false_target()).append(block.ptr());
        }
    }
}

// Phase 1: Place placeholder phis at all join points for all written operands
void Lifter::place_phi_nodes()
{
    for (auto& block : m_function->basic_blocks()) {
        auto preds = m_predecessors.get(block.ptr());
        if (!preds.has_value() || preds->size() <= 1)
            continue;

        // This is a join point - place a phi for each written operand
        for (auto raw : m_written_operands) {
            // Create an empty phi (we'll fill operands in phase 2)
            Vector<Value*> empty_values;
            Vector<BasicBlock*> empty_blocks;
            for (size_t i = 0; i < preds->size(); ++i) {
                empty_values.append(nullptr);
                empty_blocks.append((*preds)[i]);
            }

            auto& phi = m_function->build_phi(*block, empty_values, empty_blocks);
            m_phi_map.set(make_phi_key(block.ptr(), raw), &phi);
            m_value_to_operand_raw.set(&phi, raw);

            // Update m_block_definitions to include the phi value, UNLESS the block
            // has an actual definition that would override it. This ensures successors
            // inherit the correct value.
            auto actual_defs = m_block_actual_definitions.get(block.ptr());
            if (!actual_defs.has_value() || !actual_defs->contains(raw)) {
                m_block_definitions.ensure(block.ptr()).set(raw, &phi);
            }
        }
    }
}

// Phase 2: Fill in phi operands by finding reaching definitions
void Lifter::fill_phi_operands()
{
    for (auto& block : m_function->basic_blocks()) {
        auto preds = m_predecessors.get(block.ptr());
        if (!preds.has_value() || preds->size() <= 1)
            continue;

        // For each phi in this block, fill in its operands
        for (auto& instruction : block->instructions()) {
            if (instruction->opcode() != Opcode::Phi)
                break; // Phis are at the start

            auto operand_raw_opt = m_value_to_operand_raw.get(instruction->result());
            if (!operand_raw_opt.has_value())
                continue;

            auto operand_raw = *operand_raw_opt;

            // Find reaching definition from each predecessor
            for (size_t i = 0; i < preds->size(); ++i) {
                auto* pred = (*preds)[i];
                HashTable<BasicBlock*> visited;
                auto* reaching = find_reaching_def_for_phi(*pred, operand_raw, visited);

                if (!reaching) {
                    // No definition found - use undefined
                    reaching = &m_function->create_constant(JS::js_undefined());
                }

                instruction->set_operand(i, reaching);
            }
        }
    }

    // Now fix up uses in non-phi instructions to use the correct reaching definitions
    // We need to process blocks in a way that predecessors are processed before successors.
    // Since we may not have computed dominance order, we do multiple passes until stable.
    bool changed = true;
    while (changed) {
        changed = false;

        for (auto& block : m_function->basic_blocks()) {
            // Build a map of current definitions at this point
            HashMap<u32, Value*> current_defs;

            // Start with reaching defs from predecessors (for single-predecessor blocks)
            // or the phi results (for join points)
            auto preds = m_predecessors.get(block.ptr());
            if (preds.has_value() && preds->size() == 1) {
                // Single predecessor - inherit its definitions
                auto pred_defs = m_block_definitions.get((*preds)[0]);
                if (pred_defs.has_value())
                    current_defs = *pred_defs;
            }

            // Process phis first - they define values for this block
            for (auto& instruction : block->instructions()) {
                if (instruction->opcode() != Opcode::Phi)
                    break;

                auto raw_opt = m_value_to_operand_raw.get(instruction->result());
                if (raw_opt.has_value())
                    current_defs.set(*raw_opt, instruction->result());
            }

            // Now fix operand references in non-phi instructions
            for (auto& instruction : block->instructions()) {
                if (instruction->opcode() == Opcode::Phi)
                    continue;

                for (size_t i = 0; i < instruction->operands().size(); ++i) {
                    auto* operand_value = instruction->operands()[i];
                    if (!operand_value)
                        continue;

                    auto raw_opt = m_value_to_operand_raw.get(operand_value);
                    if (!raw_opt.has_value())
                        continue;

                    auto current = current_defs.get(*raw_opt);
                    if (current.has_value() && *current != operand_value) {
                        instruction->set_operand(i, *current);
                        changed = true;
                    }
                }

                // Update current_defs if this instruction defines a value
                if (instruction->result()) {
                    auto raw_opt = m_value_to_operand_raw.get(instruction->result());
                    if (raw_opt.has_value())
                        current_defs.set(*raw_opt, instruction->result());
                }
            }

            // Update m_block_definitions so successor blocks can inherit correctly
            m_block_definitions.set(block.ptr(), current_defs);
        }
    }
}

// Find reaching definition for filling phi operands
// This is simpler than the old version because all phis are already placed
Value* Lifter::find_reaching_def_for_phi(BasicBlock& from_block, u32 operand_raw, HashTable<BasicBlock*>& visited)
{
    // If we've already visited this block, we have a cycle - return nullptr
    if (visited.contains(&from_block))
        return nullptr;
    visited.set(&from_block);

    // First check if this block actually defines the operand (non-phi definition)
    // This takes priority because it's the value at the END of the block
    auto actual_defs = m_block_actual_definitions.get(&from_block);
    if (actual_defs.has_value() && actual_defs->contains(operand_raw)) {
        auto block_defs = m_block_definitions.get(&from_block);
        if (block_defs.has_value()) {
            auto value = block_defs->get(operand_raw);
            if (value.has_value())
                return *value;
        }
    }

    // If not defined in this block, check if there's a phi at entry
    // The phi merges incoming values from predecessors
    auto phi = m_phi_map.get(make_phi_key(&from_block, operand_raw));
    if (phi.has_value())
        return *phi;

    // No actual definition here - trace back through predecessors
    auto preds = m_predecessors.get(&from_block);
    if (!preds.has_value() || preds->is_empty()) {
        // Entry block with no definition - check for initial values (constants, parameters)
        auto block_defs = m_block_definitions.get(&from_block);
        if (block_defs.has_value()) {
            auto value = block_defs->get(operand_raw);
            if (value.has_value())
                return *value;
        }
        return nullptr;
    }

    // For single predecessor, just trace back
    if (preds->size() == 1)
        return find_reaching_def_for_phi(*(*preds)[0], operand_raw, visited);

    // Multiple predecessors - this should have a phi (placed in phase 1)
    // If we get here, something is wrong - return nullptr
    return nullptr;
}

}
