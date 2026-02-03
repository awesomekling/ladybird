/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/NumericLimits.h>
#include <AK/StdLibExtras.h>
#include <LibJS/Bytecode/Instruction.h>
#include <LibJS/Bytecode/Op.h>
#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Lowerer.h>
#include <LibJS/IR/Value.h>
#include <LibJS/Runtime/Completion.h>
#include <LibJS/Runtime/EnvironmentCoordinate.h>
#include <LibJS/Runtime/VM.h>

namespace JS::IR {

Lowerer::Lowerer(VM& vm, Function const& function)
    : m_vm(vm)
    , m_function(function)
{
}

Bytecode::Operand Lowerer::allocate_register()
{
    return Bytecode::Operand(Bytecode::Register(m_next_register++));
}

u32 Lowerer::get_or_add_constant(JS::Value constant_value)
{
    // Check if we already have this constant
    for (size_t i = 0; i < m_constants.size(); ++i) {
        if (m_constants[i].is_int32() && constant_value.is_int32() && m_constants[i].as_i32() == constant_value.as_i32())
            return static_cast<u32>(i);
        if (m_constants[i].is_double() && constant_value.is_double() && m_constants[i].as_double() == constant_value.as_double())
            return static_cast<u32>(i);
        if (m_constants[i].is_undefined() && constant_value.is_undefined())
            return static_cast<u32>(i);
        if (m_constants[i].is_null() && constant_value.is_null())
            return static_cast<u32>(i);
        if (m_constants[i].is_boolean() && constant_value.is_boolean() && m_constants[i].as_bool() == constant_value.as_bool())
            return static_cast<u32>(i);
    }

    // Add new constant
    auto index = static_cast<u32>(m_constants.size());
    m_constants.append(constant_value);
    return index;
}

Bytecode::Operand Lowerer::operand_for_value(Value const& value)
{
    // Check if we already have an operand for this value
    if (auto it = m_value_to_operand.find(&value); it != m_value_to_operand.end())
        return it->value;

    Bytecode::Operand operand = [&]() {
        if (value.is_constant()) {
            // For constants, create a constant operand
            auto constant_value = value.constant_value();
            auto index = get_or_add_constant(constant_value);
            return Bytecode::Operand(Bytecode::Operand::Type::Constant, index);
        }
        if (value.is_parameter()) {
            // For parameters, create an argument operand
            return Bytecode::Operand(Bytecode::Operand::Type::Argument, value.parameter_index());
        }
        return allocate_register();
    }();

    m_value_to_operand.set(&value, operand);
    return operand;
}

Bytecode::Operand Lowerer::allocate_tuple_registers(Value const& tuple, u32 count)
{
    // Allocate consecutive registers for tuple elements
    auto base = Bytecode::Operand(Bytecode::Register(m_next_register));
    m_next_register += count;
    m_tuple_base_operand.set(&tuple, base);
    return base;
}

Bytecode::Operand Lowerer::operand_for_tuple_element(Value const& tuple, u32 index)
{
    auto it = m_tuple_base_operand.find(&tuple);
    VERIFY(it != m_tuple_base_operand.end());
    auto base_index = it->value.index();
    return Bytecode::Operand(Bytecode::Register(base_index + index));
}

template<typename OpType, typename... Args>
void Lowerer::emit(Args&&... args)
{
    VERIFY(m_current_block);
    size_t slot_offset = m_current_block->size();
    m_current_block->grow(sizeof(OpType));
    void* slot = m_current_block->data() + slot_offset;
    new (slot) OpType(forward<Args>(args)...);
}

template<typename OpType, typename... Args>
void Lowerer::emit_with_extra_operand_slots(size_t extra_operand_slots, Args&&... args)
{
    VERIFY(m_current_block);
    size_t size_to_allocate = round_up_to_power_of_two(sizeof(OpType) + extra_operand_slots * sizeof(Bytecode::Operand), alignof(void*));
    size_t slot_offset = m_current_block->size();
    m_current_block->grow(size_to_allocate);
    void* slot = m_current_block->data() + slot_offset;
    new (slot) OpType(forward<Args>(args)...);
}

void Lowerer::emit_phi_moves_for_successor(BasicBlock const& from, BasicBlock const& to)
{
    // For each phi in the successor block, emit a Mov to set up the phi value
    for (auto const& instruction : to.instructions()) {
        if (instruction->opcode() != Opcode::Phi)
            break; // Phis are always at the start

        auto const& preds = instruction->phi_predecessors();
        auto const& operands = instruction->operands();

        for (size_t i = 0; i < preds.size(); ++i) {
            if (preds[i] == &from) {
                // This predecessor provides operands[i] for the phi
                if (!operands[i] || !instruction->result())
                    continue;
                auto src = operand_for_value(*operands[i]);
                auto dst = operand_for_value(*instruction->result());
                if (src != dst)
                    emit<Bytecode::Op::Mov>(dst, src);
                break;
            }
        }
    }
}

void Lowerer::lower_instruction(Instruction const& instruction)
{
    auto dst = [&]() -> Bytecode::Operand {
        if (instruction.result())
            return operand_for_value(*instruction.result());
        return Bytecode::Operand(Bytecode::Register(0)); // Dummy
    };

    auto operand = [&](size_t index) -> Bytecode::Operand {
        if (index < instruction.operands().size() && instruction.operands()[index])
            return operand_for_value(*instruction.operands()[index]);
        return Bytecode::Operand(Bytecode::Register(0)); // Dummy
    };

    switch (instruction.opcode()) {
    case Opcode::Phi:
        // Phi nodes are handled by emitting moves in predecessors
        break;

    case Opcode::Move: {
        auto d = dst();
        auto s = operand(0);
        if (d != s)
            emit<Bytecode::Op::Mov>(d, s);
        break;
    }

    // Arithmetic
    case Opcode::Add:
        emit<Bytecode::Op::Add>(dst(), operand(0), operand(1));
        break;
    case Opcode::Sub:
        emit<Bytecode::Op::Sub>(dst(), operand(0), operand(1));
        break;
    case Opcode::Mul:
        emit<Bytecode::Op::Mul>(dst(), operand(0), operand(1));
        break;
    case Opcode::Div:
        emit<Bytecode::Op::Div>(dst(), operand(0), operand(1));
        break;
    case Opcode::Mod:
        emit<Bytecode::Op::Mod>(dst(), operand(0), operand(1));
        break;
    case Opcode::Exp:
        emit<Bytecode::Op::Exp>(dst(), operand(0), operand(1));
        break;

    // Bitwise
    case Opcode::BitwiseAnd:
        emit<Bytecode::Op::BitwiseAnd>(dst(), operand(0), operand(1));
        break;
    case Opcode::BitwiseOr:
        emit<Bytecode::Op::BitwiseOr>(dst(), operand(0), operand(1));
        break;
    case Opcode::BitwiseXor:
        emit<Bytecode::Op::BitwiseXor>(dst(), operand(0), operand(1));
        break;
    case Opcode::BitwiseNot:
        emit<Bytecode::Op::BitwiseNot>(dst(), operand(0));
        break;
    case Opcode::LeftShift:
        emit<Bytecode::Op::LeftShift>(dst(), operand(0), operand(1));
        break;
    case Opcode::RightShift:
        emit<Bytecode::Op::RightShift>(dst(), operand(0), operand(1));
        break;
    case Opcode::UnsignedRightShift:
        emit<Bytecode::Op::UnsignedRightShift>(dst(), operand(0), operand(1));
        break;

    // Comparison
    case Opcode::LessThan:
        emit<Bytecode::Op::LessThan>(dst(), operand(0), operand(1));
        break;
    case Opcode::LessThanEquals:
        emit<Bytecode::Op::LessThanEquals>(dst(), operand(0), operand(1));
        break;
    case Opcode::GreaterThan:
        emit<Bytecode::Op::GreaterThan>(dst(), operand(0), operand(1));
        break;
    case Opcode::GreaterThanEquals:
        emit<Bytecode::Op::GreaterThanEquals>(dst(), operand(0), operand(1));
        break;
    case Opcode::LooselyEquals:
        emit<Bytecode::Op::LooselyEquals>(dst(), operand(0), operand(1));
        break;
    case Opcode::StrictlyEquals:
        emit<Bytecode::Op::StrictlyEquals>(dst(), operand(0), operand(1));
        break;
    case Opcode::LooselyInequals:
        emit<Bytecode::Op::LooselyInequals>(dst(), operand(0), operand(1));
        break;
    case Opcode::StrictlyInequals:
        emit<Bytecode::Op::StrictlyInequals>(dst(), operand(0), operand(1));
        break;

    // Unary
    case Opcode::Negate:
        emit<Bytecode::Op::UnaryMinus>(dst(), operand(0));
        break;
    case Opcode::UnaryPlus:
        emit<Bytecode::Op::UnaryPlus>(dst(), operand(0));
        break;
    case Opcode::Not:
        emit<Bytecode::Op::Not>(dst(), operand(0));
        break;
    case Opcode::Typeof:
        emit<Bytecode::Op::Typeof>(dst(), operand(0));
        break;

    // Type conversions
    case Opcode::ToBoolean:
        emit<Bytecode::Op::ToBoolean>(dst(), operand(0));
        break;
    case Opcode::ToNumber:
        emit<Bytecode::Op::UnaryPlus>(dst(), operand(0)); // ToNumber is essentially unary plus
        break;
    case Opcode::ToString:
        emit<Bytecode::Op::ToString>(dst(), operand(0));
        break;
    case Opcode::ToObject:
        emit<Bytecode::Op::ToObject>(dst(), operand(0));
        break;
    case Opcode::ToInt32:
        emit<Bytecode::Op::ToInt32>(dst(), operand(0));
        break;
    case Opcode::ToLength:
        emit<Bytecode::Op::ToLength>(dst(), operand(0));
        break;

    // Increment/Decrement
    case Opcode::Increment:
        if (dst() != operand(0))
            emit<Bytecode::Op::Mov>(dst(), operand(0));
        emit<Bytecode::Op::Increment>(dst());
        break;
    case Opcode::Decrement:
        if (dst() != operand(0))
            emit<Bytecode::Op::Mov>(dst(), operand(0));
        emit<Bytecode::Op::Decrement>(dst());
        break;

    // String
    case Opcode::ConcatString:
        emit<Bytecode::Op::ConcatString>(dst(), operand(0));
        break;
    case Opcode::GetLength:
        emit<Bytecode::Op::GetLength>(dst(), operand(0), OptionalNone {}, instruction.cache_index());
        break;

    // Property access
    case Opcode::GetById:
        emit<Bytecode::Op::GetById>(dst(), operand(0), instruction.property_key_index(), OptionalNone {}, instruction.cache_index());
        break;
    case Opcode::GetByValue:
        emit<Bytecode::Op::GetByValue>(dst(), operand(0), operand(1), OptionalNone {});
        break;
    case Opcode::PutById:
        emit<Bytecode::Op::PutNormalById>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index(), OptionalNone {});
        break;
    case Opcode::PutByValue:
        emit<Bytecode::Op::PutNormalByValue>(operand(0), operand(1), operand(2), OptionalNone {});
        break;
    case Opcode::DeleteById:
        emit<Bytecode::Op::DeleteById>(dst(), operand(0), instruction.property_key_index());
        break;
    case Opcode::DeleteByValue:
        emit<Bytecode::Op::DeleteByValue>(dst(), operand(0), operand(1));
        break;
    case Opcode::HasProperty:
        emit<Bytecode::Op::In>(dst(), operand(1), operand(0));
        break;

    // Environment
    case Opcode::GetBinding:
        emit<Bytecode::Op::GetBinding>(dst(), instruction.identifier_index());
        break;
    case Opcode::SetBinding:
        // NB: Using InitializeLexicalBinding since we don't track the init vs set distinction in IR
        emit<Bytecode::Op::InitializeLexicalBinding>(instruction.identifier_index(), operand(0));
        break;
    case Opcode::GetGlobal:
        emit<Bytecode::Op::GetGlobal>(dst(), instruction.identifier_index(), instruction.cache_index());
        break;
    case Opcode::SetGlobal:
        emit<Bytecode::Op::SetGlobal>(instruction.identifier_index(), operand(0), instruction.cache_index());
        break;
    case Opcode::DeleteVariable:
        emit<Bytecode::Op::DeleteVariable>(dst(), instruction.identifier_index());
        break;
    case Opcode::TypeofBinding:
        emit<Bytecode::Op::TypeofBinding>(dst(), instruction.identifier_index());
        break;

    // In/InstanceOf
    case Opcode::In:
        emit<Bytecode::Op::In>(dst(), operand(0), operand(1));
        break;
    case Opcode::InstanceOf:
        emit<Bytecode::Op::InstanceOf>(dst(), operand(0), operand(1));
        break;

    // Object creation
    case Opcode::NewObject:
        emit<Bytecode::Op::NewObject>(dst(), instruction.cache_index());
        break;

    // Postfix increment/decrement (dst gets old value, src gets mutated)
    case Opcode::PostfixIncrement:
        emit<Bytecode::Op::PostfixIncrement>(dst(), operand(0));
        break;
    case Opcode::PostfixDecrement:
        emit<Bytecode::Op::PostfixDecrement>(dst(), operand(0));
        break;

    // Control flow - handled separately
    case Opcode::Jump:
    case Opcode::Branch:
    case Opcode::Return:
    case Opcode::Throw:
        // These are terminators, handled in lower_blocks
        break;

    // Opcodes that don't produce bytecode (side-effect only in IR)
    case Opcode::LoadConstant:
    case Opcode::LoadUndefined:
    case Opcode::LoadNull:
        // Constants are handled by operand_for_value - no explicit lowering needed
        break;

    // Call (variable-length arguments)
    case Opcode::Call: {
        // IR Call operands: [callee, this_value, arg0, arg1, ...]
        auto callee = operand(0);
        auto this_value = operand(1);
        size_t arg_count = instruction.operands().size() - 2;
        Vector<Bytecode::Operand> args;
        for (size_t i = 0; i < arg_count; ++i)
            args.append(operand(i + 2));
        emit_with_extra_operand_slots<Bytecode::Op::Call>(arg_count, dst(), callee, this_value, Optional<Bytecode::StringTableIndex> {}, ReadonlySpan<Bytecode::Operand> { args });
        break;
    }

    // NewArray (variable-length elements)
    case Opcode::NewArray: {
        Vector<Bytecode::Operand> elements;
        for (size_t i = 0; i < instruction.operands().size(); ++i)
            elements.append(operand(i));
        emit_with_extra_operand_slots<Bytecode::Op::NewArray>(elements.size(), dst(), ReadonlySpan<Bytecode::Operand> { elements });
        break;
    }

    // NewFunction
    case Opcode::NewFunction:
        emit<Bytecode::Op::NewFunction>(dst(), *instruction.function_node(), instruction.lhs_name(), OptionalNone {});
        break;

    // Construct (variable-length arguments)
    case Opcode::Construct: {
        // IR Construct operands: [callee, arg0, arg1, ...]
        auto callee = operand(0);
        size_t arg_count = instruction.operands().size() - 1;
        Vector<Bytecode::Operand> args;
        for (size_t i = 0; i < arg_count; ++i)
            args.append(operand(i + 1));
        emit_with_extra_operand_slots<Bytecode::Op::CallConstruct>(arg_count, dst(), callee, Optional<Bytecode::StringTableIndex> {}, ReadonlySpan<Bytecode::Operand> { args });
        break;
    }

    // Iterators
    case Opcode::GetIterator: {
        // GetIterator produces a tuple of (iterator_object, iterator_next, iterator_done)
        // Allocate 3 consecutive registers for the tuple result
        auto tuple_base = allocate_tuple_registers(*instruction.result(), 3);
        auto iterable = operand(0);
        emit<Bytecode::Op::GetIterator>(
            Bytecode::Operand(Bytecode::Register(tuple_base.index())),
            Bytecode::Operand(Bytecode::Register(tuple_base.index() + 1)),
            Bytecode::Operand(Bytecode::Register(tuple_base.index() + 2)),
            iterable,
            instruction.iterator_hint());
        break;
    }
    case Opcode::IteratorNext: {
        // Operands: [iterator_object, iterator_next, iterator_done]
        emit<Bytecode::Op::IteratorNext>(dst(), operand(0), operand(1), operand(2));
        break;
    }
    case Opcode::IteratorNextUnpack: {
        // IteratorNextUnpack produces a tuple of (value, done)
        // Allocate 2 consecutive registers for the result
        auto tuple_base = allocate_tuple_registers(*instruction.result(), 2);
        // Operands: [iterator_object, iterator_next, iterator_done]
        emit<Bytecode::Op::IteratorNextUnpack>(
            Bytecode::Operand(Bytecode::Register(tuple_base.index())),
            Bytecode::Operand(Bytecode::Register(tuple_base.index() + 1)),
            operand(0), operand(1), operand(2));
        break;
    }
    case Opcode::IteratorClose: {
        // Operands: [iterator_object, iterator_next, iterator_done]
        // NB: Using Normal completion type since we don't track completion in IR
        emit<Bytecode::Op::IteratorClose>(operand(0), operand(1), operand(2), Completion::Type::Normal, OptionalNone {});
        break;
    }
    case Opcode::IteratorToArray: {
        // Operands: [iterator_object, iterator_next, iterator_done]
        emit<Bytecode::Op::IteratorToArray>(dst(), operand(0), operand(1), operand(2));
        break;
    }

    // Tuple extraction
    case Opcode::ExtractValue: {
        // Get the tuple operand and extract the element at the given index
        auto* tuple_value = instruction.operands()[0];
        auto element_reg = operand_for_tuple_element(*tuple_value, instruction.extract_index());
        // Map the result to this register
        if (instruction.result())
            m_value_to_operand.set(instruction.result(), element_reg);
        break;
    }
    }
}

void Lowerer::lower_blocks()
{
    // First pass: create bytecode blocks and map IR blocks to indices
    for (size_t i = 0; i < m_function.basic_blocks().size(); ++i) {
        auto const& ir_block = m_function.basic_blocks()[i];
        m_ir_block_to_bytecode_index.set(ir_block.ptr(), i);
        auto bc_block = Bytecode::BasicBlock::create(static_cast<u32>(i), ir_block->name());
        m_bytecode_blocks.append(move(bc_block));
    }

    // Second pass: lower each block
    for (size_t i = 0; i < m_function.basic_blocks().size(); ++i) {
        auto const& ir_block = m_function.basic_blocks()[i];
        m_current_block = m_bytecode_blocks[i].ptr();

        // Lower non-terminator instructions
        for (auto const& instruction : ir_block->instructions()) {
            if (instruction->is_terminator())
                continue;
            lower_instruction(*instruction);
        }

        // Handle terminator
        auto* terminator = ir_block->last_instruction();
        if (!terminator)
            continue;

        switch (terminator->opcode()) {
        case Opcode::Jump: {
            auto* target = terminator->true_target();
            if (target) {
                emit_phi_moves_for_successor(*ir_block, *target);
                auto target_index = m_ir_block_to_bytecode_index.get(target).value();
                emit<Bytecode::Op::Jump>(Bytecode::Label { static_cast<u32>(target_index) });
            }
            break;
        }
        case Opcode::Branch: {
            auto condition = operand_for_value(*terminator->operands()[0]);
            auto* true_target = terminator->true_target();
            auto* false_target = terminator->false_target();

            // Emit phi moves before the branch
            // NB: This is a simplification - in reality we'd need to emit moves
            // only along the taken edge, which requires critical edge splitting
            if (true_target)
                emit_phi_moves_for_successor(*ir_block, *true_target);
            if (false_target && false_target != true_target)
                emit_phi_moves_for_successor(*ir_block, *false_target);

            if (true_target && false_target) {
                auto true_index = m_ir_block_to_bytecode_index.get(true_target).value();
                auto false_index = m_ir_block_to_bytecode_index.get(false_target).value();
                emit<Bytecode::Op::JumpIf>(condition,
                    Bytecode::Label { static_cast<u32>(true_index) },
                    Bytecode::Label { static_cast<u32>(false_index) });
            } else if (true_target) {
                auto target_index = m_ir_block_to_bytecode_index.get(true_target).value();
                emit<Bytecode::Op::Jump>(Bytecode::Label { static_cast<u32>(target_index) });
            }
            break;
        }
        case Opcode::Return: {
            auto value = operand_for_value(*terminator->operands()[0]);
            emit<Bytecode::Op::Return>(value);
            break;
        }
        case Opcode::Throw: {
            auto value = operand_for_value(*terminator->operands()[0]);
            emit<Bytecode::Op::Throw>(value);
            break;
        }
        default:
            break;
        }
    }
}

GC::Ref<Bytecode::Executable> Lowerer::lower(VM& vm, Function const& function)
{
    Lowerer lowerer(vm, function);
    lowerer.lower_blocks();

    auto source_executable = function.source_executable();
    auto const number_of_registers = lowerer.m_next_register;
    auto const number_of_constants = lowerer.m_constants.size();

    // Combine all bytecode blocks into one flat buffer, tracking block offsets and label positions
    Vector<u8> bytecode;
    Vector<size_t> basic_block_start_offsets;
    Vector<size_t> label_offsets;
    HashMap<Bytecode::BasicBlock const*, size_t> block_offsets;

    for (auto const& block : lowerer.m_bytecode_blocks) {
        basic_block_start_offsets.append(bytecode.size());
        block_offsets.set(block.ptr(), bytecode.size());

        Bytecode::InstructionStreamIterator it(block->instruction_stream());
        while (!it.at_end()) {
            auto& instruction = const_cast<Bytecode::Instruction&>(*it);

            // Offset constant and argument operands to flat indices
            // NB: Layout is [registers | constants | arguments] (no locals in lowered code)
            instruction.visit_operands([number_of_registers, number_of_constants](Bytecode::Operand& operand) {
                if (operand.type() == Bytecode::Operand::Type::Constant) {
                    operand.offset_index_by(number_of_registers);
                } else if (operand.type() == Bytecode::Operand::Type::Argument) {
                    operand.offset_index_by(number_of_registers + number_of_constants);
                }
            });

            // Track label positions for patching
            instruction.visit_labels([&](Bytecode::Label& label) {
                size_t label_offset = bytecode.size() + (bit_cast<FlatPtr>(&label) - bit_cast<FlatPtr>(&instruction));
                label_offsets.append(label_offset);
            });

            bytecode.append(reinterpret_cast<u8 const*>(&instruction), instruction.length());
            ++it;
        }
    }

    // Patch labels with actual instruction offsets
    for (auto label_offset : label_offsets) {
        auto& label = *reinterpret_cast<Bytecode::Label*>(bytecode.data() + label_offset);
        auto* block = lowerer.m_bytecode_blocks[label.basic_block_index()].ptr();
        label.set_address(block_offsets.get(block).value());
    }

    // Copy tables from source executable
    auto identifier_table = make<Bytecode::IdentifierTable>();
    for (auto const& identifier : source_executable->identifier_table->identifiers())
        identifier_table->insert(identifier);

    auto property_key_table = make<Bytecode::PropertyKeyTable>();
    for (auto const& key : source_executable->property_key_table->property_keys())
        property_key_table->insert(key);

    // NB: String and regex tables are not exposed for iteration, create empty ones for now
    auto string_table = make<Bytecode::StringTable>();
    auto regex_table = make<Bytecode::RegexTable>();

    Vector<JS::Value> constants = move(lowerer.m_constants);

    auto executable = vm.heap().allocate<Bytecode::Executable>(
        move(bytecode),
        move(identifier_table),
        move(property_key_table),
        move(string_table),
        move(regex_table),
        move(constants),
        source_executable->source_code,
        source_executable->property_lookup_caches.size(),
        source_executable->global_variable_caches.size(),
        source_executable->template_object_caches.size(),
        source_executable->object_shape_caches.size(),
        number_of_registers,
        source_executable->is_strict_mode ? Strict::Yes : Strict::No);

    executable->basic_block_start_offsets = move(basic_block_start_offsets);
    executable->name = source_executable->name;

    // Copy cache contents from source executable to preserve cached information
    for (size_t i = 0; i < source_executable->property_lookup_caches.size(); ++i)
        executable->property_lookup_caches[i] = source_executable->property_lookup_caches[i];
    for (size_t i = 0; i < source_executable->global_variable_caches.size(); ++i)
        executable->global_variable_caches[i] = source_executable->global_variable_caches[i];
    for (size_t i = 0; i < source_executable->template_object_caches.size(); ++i)
        executable->template_object_caches[i] = source_executable->template_object_caches[i];
    for (size_t i = 0; i < source_executable->object_shape_caches.size(); ++i)
        executable->object_shape_caches[i] = source_executable->object_shape_caches[i];

    // Set up register/constant/argument counts
    // NB: No locals in lowered code, so registers_and_locals_count == number_of_registers
    executable->registers_and_locals_count = number_of_registers;
    executable->registers_and_locals_and_constants_count = number_of_registers + number_of_constants;
    executable->argument_index_base = number_of_registers + number_of_constants;

    return executable;
}

}
