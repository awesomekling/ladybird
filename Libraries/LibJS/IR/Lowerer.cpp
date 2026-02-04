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

// Check if a comparison instruction should be fused with a Branch
// Returns true if the comparison has exactly one use and that use is a Branch terminator
static bool should_fuse_comparison_with_branch(Instruction const& instruction)
{
    switch (instruction.opcode()) {
    case Opcode::LessThan:
    case Opcode::LessThanEquals:
    case Opcode::GreaterThan:
    case Opcode::GreaterThanEquals:
    case Opcode::LooselyEquals:
    case Opcode::StrictlyEquals:
    case Opcode::LooselyInequals:
    case Opcode::StrictlyInequals:
        break;
    default:
        return false;
    }

    auto* result = instruction.result();
    if (!result)
        return false;

    auto const& uses = result->uses();
    if (uses.size() != 1)
        return false;

    auto* use = uses[0];
    if (use->opcode() != Opcode::Branch)
        return false;

    // The Branch must use this comparison as its condition (operand 0)
    if (use->operands().is_empty() || use->operands()[0] != result)
        return false;

    return true;
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
    // Check for coalescing: if this value coalesces with another, use the same operand
    // Follow the chain to find the ultimate representative
    Value const* lookup_value = &value;
    for (;;) {
        auto it = m_coalesce_representative.find(lookup_value);
        if (it == m_coalesce_representative.end())
            break;
        lookup_value = it->value;
    }

    // Check if we already have an operand for this value (or its coalescing representative)
    if (auto it = m_value_to_operand.find(lookup_value); it != m_value_to_operand.end())
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
        if (value.is_this()) {
            // For this, use the this register
            return Bytecode::Operand(Bytecode::Register::this_value());
        }
        // Yield and Await results are resume values that appear in the accumulator (reg0)
        if (auto* defining = value.defining_instruction()) {
            if (defining->opcode() == Opcode::Yield || defining->opcode() == Opcode::Await)
                return Bytecode::Operand(Bytecode::Register::accumulator());
        }
        return allocate_register();
    }();

    m_value_to_operand.set(lookup_value, operand);
    return operand;
}

void Lowerer::compute_phi_coalescing()
{
    // Phi coalescing: assign the same register to values that can share one.
    // This eliminates Mov instructions when lowering phi nodes.

    // Helper to find the representative of a coalescing class (with path compression)
    auto find_representative = [&](Value const* v) -> Value const* {
        Vector<Value const*> path;
        for (;;) {
            auto it = m_coalesce_representative.find(v);
            if (it == m_coalesce_representative.end())
                break;
            path.append(v);
            v = it->value;
        }
        // Path compression
        for (auto* p : path)
            m_coalesce_representative.set(p, v);
        return v;
    };

    auto coalesce = [&](Value const* a, Value const* b) {
        auto* rep_a = find_representative(a);
        auto* rep_b = find_representative(b);
        if (rep_a != rep_b)
            m_coalesce_representative.set(rep_a, rep_b);
    };

    for (auto const& block : m_function.basic_blocks()) {
        for (auto const& instruction : block->instructions()) {
            if (instruction->opcode() != Opcode::Phi)
                break; // Phis are always at the start

            auto* phi_result = instruction->result();
            if (!phi_result)
                continue;

            for (auto* operand : instruction->operands()) {
                if (!operand)
                    continue;

                // Can't coalesce constants, parameters, or this
                if (operand->is_constant() || operand->is_parameter() || operand->is_this())
                    continue;

                // Chain coalescing: if operand is a phi result, coalesce the two phis.
                // This makes chains like: v14 = Phi[v0,v2], v15 = Phi[v14,v4], ...
                // all share the same register.
                if (auto* def = operand->defining_instruction(); def && def->opcode() == Opcode::Phi) {
                    coalesce(operand, phi_result);
                    continue;
                }

                // Standard coalescing: if all of operand's non-phi uses are terminators
                // (Branch, Return, etc.), then the operand is dead at the phi point.
                bool can_coalesce = true;
                for (auto* use : operand->uses()) {
                    if (use == instruction.ptr()) // This phi
                        continue;
                    if (!use->is_terminator()) {
                        can_coalesce = false;
                        break;
                    }
                }
                if (can_coalesce)
                    coalesce(operand, phi_result);
            }
        }
    }
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

bool Lowerer::target_has_phis(BasicBlock const& target) const
{
    if (target.instructions().is_empty())
        return false;
    return target.instructions().first()->opcode() == Opcode::Phi;
}

bool Lowerer::needs_phi_moves_for_edge(BasicBlock const& from, BasicBlock const& to)
{
    // Check if any phi in the target block would need a move for this edge
    for (auto const& instruction : to.instructions()) {
        if (instruction->opcode() != Opcode::Phi)
            break;

        auto const& preds = instruction->phi_predecessors();
        auto const& operands = instruction->operands();

        for (size_t i = 0; i < preds.size(); ++i) {
            if (preds[i] == &from) {
                if (!operands[i] || !instruction->result())
                    continue;
                auto src = operand_for_value(*operands[i]);
                auto dst = operand_for_value(*instruction->result());
                if (src != dst)
                    return true;
                break;
            }
        }
    }
    return false;
}

size_t Lowerer::get_or_create_trampoline(BasicBlock const& from, BasicBlock const& to)
{
    // Create a unique key for this edge
    auto from_idx = m_ir_block_to_bytecode_index.get(&from).value();
    auto to_idx = m_ir_block_to_bytecode_index.get(&to).value();
    u64 edge_key = (static_cast<u64>(from_idx) << 32) | static_cast<u64>(to_idx);

    // Check if we already have a trampoline for this edge
    if (auto it = m_edge_to_trampoline.find(edge_key); it != m_edge_to_trampoline.end())
        return it->value;

    // Create a new trampoline block
    auto trampoline_idx = m_bytecode_blocks.size();
    auto trampoline = Bytecode::BasicBlock::create(static_cast<u32>(trampoline_idx),
        String::formatted("trampoline_{}_{}", from.name(), to.name()).release_value_but_fixme_should_propagate_errors());
    m_bytecode_blocks.append(move(trampoline));
    m_edge_to_trampoline.set(edge_key, trampoline_idx);

    // Emit phi moves and jump in the trampoline block
    auto* saved_block = m_current_block;
    m_current_block = m_bytecode_blocks[trampoline_idx].ptr();
    emit_phi_moves_for_successor(from, to);
    emit<Bytecode::Op::Jump>(Bytecode::Label { static_cast<u32>(to_idx) });
    m_current_block = saved_block;

    return trampoline_idx;
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
    // NOTE: These may be skipped if they will be fused with a Branch instruction
    case Opcode::LessThan:
        if (!should_fuse_comparison_with_branch(instruction))
            emit<Bytecode::Op::LessThan>(dst(), operand(0), operand(1));
        break;
    case Opcode::LessThanEquals:
        if (!should_fuse_comparison_with_branch(instruction))
            emit<Bytecode::Op::LessThanEquals>(dst(), operand(0), operand(1));
        break;
    case Opcode::GreaterThan:
        if (!should_fuse_comparison_with_branch(instruction))
            emit<Bytecode::Op::GreaterThan>(dst(), operand(0), operand(1));
        break;
    case Opcode::GreaterThanEquals:
        if (!should_fuse_comparison_with_branch(instruction))
            emit<Bytecode::Op::GreaterThanEquals>(dst(), operand(0), operand(1));
        break;
    case Opcode::LooselyEquals:
        if (!should_fuse_comparison_with_branch(instruction))
            emit<Bytecode::Op::LooselyEquals>(dst(), operand(0), operand(1));
        break;
    case Opcode::StrictlyEquals:
        if (!should_fuse_comparison_with_branch(instruction))
            emit<Bytecode::Op::StrictlyEquals>(dst(), operand(0), operand(1));
        break;
    case Opcode::LooselyInequals:
        if (!should_fuse_comparison_with_branch(instruction))
            emit<Bytecode::Op::LooselyInequals>(dst(), operand(0), operand(1));
        break;
    case Opcode::StrictlyInequals:
        if (!should_fuse_comparison_with_branch(instruction))
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
    case Opcode::IsUndefined: {
        auto undef_index = get_or_add_constant(js_undefined());
        auto undef_operand = Bytecode::Operand(Bytecode::Operand::Type::Constant, undef_index);
        emit<Bytecode::Op::StrictlyEquals>(dst(), operand(0), undef_operand);
        break;
    }
    case Opcode::IsNullish: {
        // Nullish means undefined or null
        // We emit: (value === undefined) || (value === null)
        // For simplicity, emit a series: temp = (value === undefined), result = temp || (value === null)
        // Actually, let's just emit a LooselyEquals with null, which is true for both null and undefined
        auto null_index = get_or_add_constant(js_null());
        auto null_operand = Bytecode::Operand(Bytecode::Operand::Type::Constant, null_index);
        emit<Bytecode::Op::LooselyEquals>(dst(), operand(0), null_operand);
        break;
    }
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
    case Opcode::ConcatString: {
        auto d = dst();
        auto base = operand(0);
        auto to_append = operand(1);
        // Bytecode ConcatString expects dst to already contain the base string
        if (d != base)
            emit<Bytecode::Op::Mov>(d, base);
        emit<Bytecode::Op::ConcatString>(d, to_append);
        break;
    }
    case Opcode::GetLength:
        emit<Bytecode::Op::GetLength>(dst(), operand(0), OptionalNone {}, instruction.cache_index());
        break;

    // Property access
    case Opcode::GetById:
        emit<Bytecode::Op::GetById>(dst(), operand(0), instruction.property_key_index(), instruction.base_identifier(), instruction.cache_index());
        break;
    case Opcode::GetByIdWithThis:
        emit<Bytecode::Op::GetByIdWithThis>(dst(), operand(0), instruction.property_key_index(), operand(1), instruction.cache_index());
        break;
    case Opcode::GetByValue:
        emit<Bytecode::Op::GetByValue>(dst(), operand(0), operand(1), instruction.base_identifier());
        break;
    case Opcode::GetByValueWithThis:
        // operands: base (0), this_value (1), property (2)
        // constructor: dst, base, property, this_value
        emit<Bytecode::Op::GetByValueWithThis>(dst(), operand(0), operand(2), operand(1));
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
    case Opcode::GetPrivateById:
        emit<Bytecode::Op::GetPrivateById>(dst(), operand(0), instruction.identifier_index());
        break;
    case Opcode::PutPrivateById:
        emit<Bytecode::Op::PutPrivateById>(operand(0), instruction.identifier_index(), operand(1));
        break;
    case Opcode::PutGetterById:
        emit<Bytecode::Op::PutGetterById>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index(), instruction.base_identifier());
        break;
    case Opcode::PutSetterById:
        emit<Bytecode::Op::PutSetterById>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index(), instruction.base_identifier());
        break;
    case Opcode::PutPrototypeById:
        emit<Bytecode::Op::PutPrototypeById>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index(), instruction.base_identifier());
        break;
    case Opcode::PutGetterByValue:
        emit<Bytecode::Op::PutGetterByValue>(operand(0), operand(1), operand(2), instruction.base_identifier());
        break;
    case Opcode::PutSetterByValue:
        emit<Bytecode::Op::PutSetterByValue>(operand(0), operand(1), operand(2), instruction.base_identifier());
        break;
    case Opcode::PutPrototypeByValue:
        emit<Bytecode::Op::PutPrototypeByValue>(operand(0), operand(1), operand(2), instruction.base_identifier());
        break;
    case Opcode::PutBySpread:
        emit<Bytecode::Op::PutBySpread>(operand(0), operand(1));
        break;

    // Environment
    case Opcode::GetCalleeAndThisFromEnvironment: {
        // This produces a tuple of (callee, this_value)
        // Allocate 2 consecutive registers for the tuple result
        auto tuple_base = allocate_tuple_registers(*instruction.result(), 2);
        emit<Bytecode::Op::GetCalleeAndThisFromEnvironment>(
            Bytecode::Operand(Bytecode::Register(tuple_base.index())),
            Bytecode::Operand(Bytecode::Register(tuple_base.index() + 1)),
            instruction.identifier_index());
        break;
    }
    case Opcode::CreateVariable:
        emit<Bytecode::Op::CreateVariable>(instruction.identifier_index(), instruction.environment_mode(), instruction.is_immutable(), instruction.is_global(), instruction.is_strict());
        break;
    case Opcode::CreateLexicalEnvironment:
        emit<Bytecode::Op::CreateLexicalEnvironment>(dst(), instruction.capacity());
        break;
    case Opcode::CreateMutableBinding:
        emit<Bytecode::Op::CreateMutableBinding>(operand(0), instruction.identifier_index(), instruction.is_strict());
        break;
    case Opcode::CreateImmutableBinding:
        emit<Bytecode::Op::CreateImmutableBinding>(operand(0), instruction.identifier_index(), instruction.is_strict());
        break;
    case Opcode::LeaveLexicalEnvironment:
        emit<Bytecode::Op::LeaveLexicalEnvironment>();
        break;
    case Opcode::EnterObjectEnvironment:
        emit<Bytecode::Op::EnterObjectEnvironment>(operand(0));
        break;
    case Opcode::ResolveThisBinding:
        emit<Bytecode::Op::ResolveThisBinding>();
        break;
    case Opcode::ResolveSuperBase:
        emit<Bytecode::Op::ResolveSuperBase>(dst());
        break;
    case Opcode::GetBinding:
        emit<Bytecode::Op::GetBinding>(dst(), instruction.identifier_index());
        break;
    case Opcode::InitializeBinding:
        emit<Bytecode::Op::InitializeLexicalBinding>(instruction.identifier_index(), operand(0));
        break;
    case Opcode::SetBinding:
        emit<Bytecode::Op::SetLexicalBinding>(instruction.identifier_index(), operand(0));
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
    case Opcode::End:
    case Opcode::Throw:
    case Opcode::Yield:
    case Opcode::Await:
        // These are terminators, handled in lower_blocks
        break;

    case Opcode::GetCompletionFields: {
        // GetCompletionFields produces a tuple of (type, value)
        // Allocate 2 consecutive registers for the result
        auto tuple_base = allocate_tuple_registers(*instruction.result(), 2);
        emit<Bytecode::Op::GetCompletionFields>(
            Bytecode::Operand(Bytecode::Register(tuple_base.index())),
            Bytecode::Operand(Bytecode::Register(tuple_base.index() + 1)),
            operand(0));
        break;
    }

    // Opcodes that don't produce bytecode (side-effect only in IR)
    case Opcode::LoadConstant:
    case Opcode::LoadUndefined:
    case Opcode::LoadNull:
        // Constants are handled by operand_for_value - no explicit lowering needed
        break;

    // Arguments
    case Opcode::CreateArguments:
        emit<Bytecode::Op::CreateArguments>(dst(), instruction.arguments_kind(), instruction.is_immutable());
        break;
    case Opcode::CreateRestParams:
        emit<Bytecode::Op::CreateRestParams>(dst(), instruction.rest_index());
        break;
    case Opcode::GetNewTarget:
        emit<Bytecode::Op::GetNewTarget>(dst());
        break;

    case Opcode::SuperCallWithArgumentArray:
        emit<Bytecode::Op::SuperCallWithArgumentArray>(dst(), operand(0), instruction.is_synthetic());
        break;
    case Opcode::ImportCall:
        emit<Bytecode::Op::ImportCall>(dst(), operand(0), operand(1));
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

    // CallBuiltin (variable-length arguments)
    case Opcode::CallBuiltin: {
        // IR CallBuiltin operands: [callee, this_value, arg0, arg1, ...]
        auto callee = operand(0);
        auto this_value = operand(1);
        size_t arg_count = instruction.operands().size() - 2;
        Vector<Bytecode::Operand> args;
        for (size_t i = 0; i < arg_count; ++i)
            args.append(operand(i + 2));
        emit_with_extra_operand_slots<Bytecode::Op::CallBuiltin>(arg_count, dst(), callee, this_value, instruction.builtin(), instruction.expression_string(), ReadonlySpan<Bytecode::Operand> { args });
        break;
    }

    // CallDirectEval (variable-length arguments)
    case Opcode::CallDirectEval: {
        // IR CallDirectEval operands: [callee, this_value, arg0, arg1, ...]
        auto callee = operand(0);
        auto this_value = operand(1);
        size_t arg_count = instruction.operands().size() - 2;
        Vector<Bytecode::Operand> args;
        for (size_t i = 0; i < arg_count; ++i)
            args.append(operand(i + 2));
        emit_with_extra_operand_slots<Bytecode::Op::CallDirectEval>(arg_count, dst(), callee, this_value, instruction.expression_string(), ReadonlySpan<Bytecode::Operand> { args });
        break;
    }

    // CallWithArgumentArray
    case Opcode::CallWithArgumentArray: {
        // IR CallWithArgumentArray operands: [callee, this_value, arguments_array]
        auto callee = operand(0);
        auto this_value = operand(1);
        auto arguments = operand(2);
        emit<Bytecode::Op::CallWithArgumentArray>(dst(), callee, this_value, arguments, instruction.expression_string());
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

    case Opcode::ArrayAppend: {
        // operand(0) = array, operand(1) = value
        emit<Bytecode::Op::ArrayAppend>(operand(0), operand(1), instruction.is_spread());
        break;
    }

    // NewClass
    case Opcode::NewClass: {
        // Operands: [super_class (may be null), element_key0, element_key1, ...]
        Optional<Bytecode::Operand> super_class;
        if (instruction.operands()[0])
            super_class = operand(0);
        size_t element_keys_count = instruction.operands().size() - 1;
        Vector<Optional<Bytecode::Operand>> element_keys;
        for (size_t i = 0; i < element_keys_count; ++i) {
            if (instruction.operands()[i + 1])
                element_keys.append(operand(i + 1));
            else
                element_keys.append(OptionalNone {});
        }
        emit_with_extra_operand_slots<Bytecode::Op::NewClass>(element_keys_count, dst(), super_class, *instruction.class_expression(), instruction.lhs_name(), ReadonlySpan<Optional<Bytecode::Operand>> { element_keys });
        break;
    }

    // NewFunction
    case Opcode::NewFunction:
        emit<Bytecode::Op::NewFunction>(dst(), *instruction.function_node(), instruction.lhs_name(), OptionalNone {});
        break;

    // NewRegExp
    case Opcode::NewRegExp:
        emit<Bytecode::Op::NewRegExp>(dst(), instruction.regex_source_index(), instruction.regex_flags_index(), instruction.regex_index());
        break;

    case Opcode::InitObjectLiteralProperty:
        emit<Bytecode::Op::InitObjectLiteralProperty>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index(), instruction.property_slot());
        break;

    case Opcode::CacheObjectShape:
        emit<Bytecode::Op::CacheObjectShape>(operand(0), instruction.cache_index());
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

    // ConstructWithArgumentArray
    case Opcode::ConstructWithArgumentArray: {
        // IR ConstructWithArgumentArray operands: [callee, this_value, arguments_array]
        auto callee = operand(0);
        auto this_value = operand(1);
        auto arguments = operand(2);
        emit<Bytecode::Op::CallConstructWithArgumentArray>(dst(), callee, this_value, arguments, instruction.expression_string());
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
        // Map the result (and its coalescing representative) to this register
        if (instruction.result()) {
            // Follow coalescing chain to find the representative
            Value const* rep = instruction.result();
            for (;;) {
                auto it = m_coalesce_representative.find(rep);
                if (it == m_coalesce_representative.end())
                    break;
                rep = it->value;
            }
            m_value_to_operand.set(rep, element_reg);
        }
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
                // Skip jump if target is the immediately following block (fallthrough)
                if (target_index != i + 1)
                    emit<Bytecode::Op::Jump>(Bytecode::Label { static_cast<u32>(target_index) });
            }
            break;
        }
        case Opcode::Branch: {
            auto* condition_value = terminator->operands()[0];
            auto* true_target = terminator->true_target();
            auto* false_target = terminator->false_target();

            if (true_target && false_target && false_target != true_target) {
                // Both targets exist and are different - check if we need critical edge splitting
                bool true_needs_moves = needs_phi_moves_for_edge(*ir_block, *true_target);
                bool false_needs_moves = needs_phi_moves_for_edge(*ir_block, *false_target);

                size_t true_index = 0;
                size_t false_index = 0;

                if (true_needs_moves && false_needs_moves) {
                    // Critical edges: both targets need phi moves, so we need trampolines
                    // to avoid phi move conflicts
                    true_index = get_or_create_trampoline(*ir_block, *true_target);
                    false_index = get_or_create_trampoline(*ir_block, *false_target);
                } else if (true_needs_moves) {
                    // Only true target needs phi moves - use trampoline for true
                    true_index = get_or_create_trampoline(*ir_block, *true_target);
                    false_index = m_ir_block_to_bytecode_index.get(false_target).value();
                } else if (false_needs_moves) {
                    // Only false target needs phi moves - use trampoline for false
                    true_index = m_ir_block_to_bytecode_index.get(true_target).value();
                    false_index = get_or_create_trampoline(*ir_block, *false_target);
                } else {
                    // Neither target needs phi moves - jump directly
                    true_index = m_ir_block_to_bytecode_index.get(true_target).value();
                    false_index = m_ir_block_to_bytecode_index.get(false_target).value();
                }

                auto true_label = Bytecode::Label { static_cast<u32>(true_index) };
                auto false_label = Bytecode::Label { static_cast<u32>(false_index) };

                // Check if condition comes from a fusible comparison
                auto* cmp_instr = condition_value->defining_instruction();
                if (cmp_instr && should_fuse_comparison_with_branch(*cmp_instr)) {
                    auto lhs = operand_for_value(*cmp_instr->operands()[0]);
                    auto rhs = operand_for_value(*cmp_instr->operands()[1]);
                    switch (cmp_instr->opcode()) {
                    case Opcode::LessThan:
                        emit<Bytecode::Op::JumpLessThan>(lhs, rhs, true_label, false_label);
                        break;
                    case Opcode::LessThanEquals:
                        emit<Bytecode::Op::JumpLessThanEquals>(lhs, rhs, true_label, false_label);
                        break;
                    case Opcode::GreaterThan:
                        emit<Bytecode::Op::JumpGreaterThan>(lhs, rhs, true_label, false_label);
                        break;
                    case Opcode::GreaterThanEquals:
                        emit<Bytecode::Op::JumpGreaterThanEquals>(lhs, rhs, true_label, false_label);
                        break;
                    case Opcode::LooselyEquals:
                        emit<Bytecode::Op::JumpLooselyEquals>(lhs, rhs, true_label, false_label);
                        break;
                    case Opcode::StrictlyEquals:
                        emit<Bytecode::Op::JumpStrictlyEquals>(lhs, rhs, true_label, false_label);
                        break;
                    case Opcode::LooselyInequals:
                        emit<Bytecode::Op::JumpLooselyInequals>(lhs, rhs, true_label, false_label);
                        break;
                    case Opcode::StrictlyInequals:
                        emit<Bytecode::Op::JumpStrictlyInequals>(lhs, rhs, true_label, false_label);
                        break;
                    default:
                        VERIFY_NOT_REACHED();
                    }
                } else {
                    auto condition = operand_for_value(*condition_value);
                    // Use JumpTrue/JumpFalse when one target is fallthrough for smaller bytecode
                    if (false_index == i + 1) {
                        // False target is fallthrough - use JumpTrue (only jumps if true)
                        emit<Bytecode::Op::JumpTrue>(condition, true_label);
                    } else if (true_index == i + 1) {
                        // True target is fallthrough - use JumpFalse (only jumps if false)
                        emit<Bytecode::Op::JumpFalse>(condition, false_label);
                    } else {
                        emit<Bytecode::Op::JumpIf>(condition, true_label, false_label);
                    }
                }
            } else if (true_target) {
                // Only one target (unconditional after condition eval, or same target)
                if (target_has_phis(*true_target))
                    emit_phi_moves_for_successor(*ir_block, *true_target);
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
        case Opcode::End: {
            auto value = operand_for_value(*terminator->operands()[0]);
            emit<Bytecode::Op::End>(value);
            break;
        }
        case Opcode::Throw: {
            auto value = operand_for_value(*terminator->operands()[0]);
            emit<Bytecode::Op::Throw>(value);
            break;
        }
        case Opcode::Yield: {
            // Yield: operand[0] is the value to yield, true_target is continuation (or null for final yield)
            auto value = operand_for_value(*terminator->operands()[0]);
            auto* continuation = terminator->true_target();
            if (continuation) {
                emit_phi_moves_for_successor(*ir_block, *continuation);
                auto continuation_index = m_ir_block_to_bytecode_index.get(continuation).value();
                emit<Bytecode::Op::Yield>(Bytecode::Label { static_cast<u32>(continuation_index) }, value);
                // The resume value appears in the accumulator (reg0) at runtime
                // Map the Yield's result to reg0 so uses in the continuation block work
                if (terminator->result())
                    m_value_to_operand.set(terminator->result(), Bytecode::Operand(Bytecode::Register::accumulator()));
            } else {
                // Final yield (generator return) - Yield with no continuation label
                emit<Bytecode::Op::Yield>(Optional<Bytecode::Label> {}, value);
            }
            break;
        }
        case Opcode::Await: {
            // Await: operand[0] is the promise/value to await, true_target is continuation
            auto argument = operand_for_value(*terminator->operands()[0]);
            auto* continuation = terminator->true_target();
            VERIFY(continuation);
            emit_phi_moves_for_successor(*ir_block, *continuation);
            auto continuation_index = m_ir_block_to_bytecode_index.get(continuation).value();
            emit<Bytecode::Op::Await>(Bytecode::Label { static_cast<u32>(continuation_index) }, argument);
            // The resume value (resolved promise) appears in the accumulator (reg0) at runtime
            // Map the Await's result to reg0 so uses in the continuation block work
            if (terminator->result())
                m_value_to_operand.set(terminator->result(), Bytecode::Operand(Bytecode::Register::accumulator()));
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
    lowerer.compute_phi_coalescing();
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

    // Copy string table from source executable
    auto string_table = make<Bytecode::StringTable>();
    for (auto const& string : source_executable->string_table->strings())
        string_table->insert(string);

    // Copy regex table from source executable
    auto regex_table = make<Bytecode::RegexTable>();
    for (auto const& regex : source_executable->regex_table->regexes())
        regex_table->insert(regex);

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

    // Copy length_identifier for GetLength instruction support
    executable->length_identifier = source_executable->length_identifier;

    // Remap exception handlers from source executable
    // Maps: source offset -> source block -> IR block -> target block -> target offset
    auto const& source_block_map = function.source_block_map();

    auto source_offset_to_target_offset = [&](size_t source_offset) -> Optional<size_t> {
        // Find source block index for this offset
        auto const& source_offsets = source_executable->basic_block_start_offsets;
        u32 source_block_index = 0;
        for (size_t i = 0; i < source_offsets.size(); ++i) {
            if (source_offsets[i] <= source_offset) {
                source_block_index = static_cast<u32>(i);
            } else {
                break;
            }
        }

        // Find IR block for this source block
        auto ir_block_it = source_block_map.find(source_block_index);
        if (ir_block_it == source_block_map.end())
            return {};
        auto* ir_block = ir_block_it->value;

        // Find target block index for this IR block
        auto target_block_it = lowerer.m_ir_block_to_bytecode_index.find(ir_block);
        if (target_block_it == lowerer.m_ir_block_to_bytecode_index.end())
            return {};

        return basic_block_start_offsets[target_block_it->value];
    };

    for (auto const& source_handler : source_executable->exception_handlers) {
        auto start = source_offset_to_target_offset(source_handler.start_offset);
        auto end = source_offset_to_target_offset(source_handler.end_offset);
        if (!start.has_value() || !end.has_value())
            continue;

        Bytecode::Executable::ExceptionHandlers handler;
        handler.start_offset = *start;
        handler.end_offset = *end;

        if (source_handler.handler_offset.has_value()) {
            handler.handler_offset = source_offset_to_target_offset(*source_handler.handler_offset);
        }
        if (source_handler.finalizer_offset.has_value()) {
            handler.finalizer_offset = source_offset_to_target_offset(*source_handler.finalizer_offset);
        }

        executable->exception_handlers.append(handler);
    }

    return executable;
}

}
