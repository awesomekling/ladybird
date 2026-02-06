/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibJS/IR/BasicBlock.h>
#include <LibJS/IR/Function.h>
#include <LibJS/IR/Instruction.h>
#include <LibJS/IR/Type.h>
#include <LibJS/IR/Value.h>

namespace JS::IR {

Instruction::Instruction(Opcode opcode)
    : m_opcode(opcode)
{
}

TerminatorInstruction::TerminatorInstruction(Opcode opcode)
    : Instruction(opcode)
{
    VERIFY(is_terminator_opcode(opcode));
}

BasicBlock* TerminatorInstruction::true_target() const
{
    if (!m_true_target.has_value())
        return nullptr;
    return parent_block()->parent_function()->block_by_index(*m_true_target);
}

BasicBlock* TerminatorInstruction::false_target() const
{
    if (!m_false_target.has_value())
        return nullptr;
    return parent_block()->parent_function()->block_by_index(*m_false_target);
}

void TerminatorInstruction::set_true_target(BasicBlock* block)
{
    m_true_target = block ? Optional<BlockIndex>(block->index()) : Optional<BlockIndex>();
}

void TerminatorInstruction::set_false_target(BasicBlock* block)
{
    m_false_target = block ? Optional<BlockIndex>(block->index()) : Optional<BlockIndex>();
}

JumpInstruction::JumpInstruction(Opcode opcode, BasicBlock& target)
    : TerminatorInstruction(opcode)
{
    VERIFY(opcode == Opcode::Jump || opcode == Opcode::ContinuePendingUnwind || opcode == Opcode::EnterUnwindContext);
    set_true_target(target.index());
}

void JumpInstruction::set_target(BasicBlock& block)
{
    set_true_target(block.index());
}

NonnullOwnPtr<JumpInstruction> JumpInstruction::create(BasicBlock& target)
{
    return adopt_own(*new JumpInstruction(Opcode::Jump, target));
}

NonnullOwnPtr<JumpInstruction> JumpInstruction::create_continue_pending_unwind(BasicBlock& resume_target)
{
    return adopt_own(*new JumpInstruction(Opcode::ContinuePendingUnwind, resume_target));
}

NonnullOwnPtr<JumpInstruction> JumpInstruction::create_enter_unwind_context(BasicBlock& entry_point)
{
    return adopt_own(*new JumpInstruction(Opcode::EnterUnwindContext, entry_point));
}

BranchInstruction::BranchInstruction(Value* condition, BasicBlock& true_target, BasicBlock& false_target)
    : TerminatorInstruction(Opcode::Branch)
{
    add_operand(condition);
    set_true_target(true_target.index());
    set_false_target(false_target.index());
}

void BranchInstruction::set_true_branch(BasicBlock& block)
{
    set_true_target(block.index());
}

void BranchInstruction::set_false_branch(BasicBlock& block)
{
    set_false_target(block.index());
}

NonnullOwnPtr<BranchInstruction> BranchInstruction::create(Value* condition, BasicBlock& true_target, BasicBlock& false_target)
{
    return adopt_own(*new BranchInstruction(condition, true_target, false_target));
}

PhiInstruction::PhiInstruction()
    : Instruction(Opcode::Phi)
{
}

NonnullOwnPtr<PhiInstruction> PhiInstruction::create()
{
    return adopt_own(*new PhiInstruction());
}

void PhiInstruction::add_incoming(BlockIndex predecessor, Value* value)
{
    add_phi_operand(predecessor, value);
}

void PhiInstruction::remove_incoming(size_t index)
{
    remove_phi_operand(index);
}

void PhiInstruction::remove_incoming_from(BlockIndex predecessor)
{
    for (size_t i = phi_predecessors().size(); i > 0; --i) {
        if (phi_predecessors()[i - 1] == predecessor) {
            remove_phi_operand(i - 1);
            return;
        }
    }
}

Value* PhiInstruction::incoming_value_for(BlockIndex predecessor) const
{
    for (size_t i = 0; i < incoming_count(); ++i) {
        if (incoming_block(i) == predecessor)
            return incoming_value(i);
    }
    return nullptr;
}

Value* PhiInstruction::incoming_value_for(BasicBlock const& predecessor) const
{
    return incoming_value_for(predecessor.index());
}

void PhiInstruction::set_incoming_value_for(BlockIndex predecessor, Value* value)
{
    for (size_t i = 0; i < incoming_count(); ++i) {
        if (incoming_block(i) == predecessor) {
            set_incoming_value(i, value);
            return;
        }
    }
    VERIFY_NOT_REACHED();
}

void PhiInstruction::set_incoming_value_for(BasicBlock const& predecessor, Value* value)
{
    set_incoming_value_for(predecessor.index(), value);
}

void PhiInstruction::set_incoming_block(size_t index, BlockIndex block)
{
    set_phi_predecessor(index, block);
}

void PhiInstruction::set_incoming_value(size_t index, Value* value)
{
    set_operand(index, value);
}

ParallelCopyInstruction::ParallelCopyInstruction()
    : Instruction(Opcode::ParallelCopy)
{
}

NonnullOwnPtr<ParallelCopyInstruction> ParallelCopyInstruction::create()
{
    return adopt_own(*new ParallelCopyInstruction());
}

void ParallelCopyInstruction::add_copy(Value* dst, Value* src)
{
    m_copies.append({ dst, src });
    add_operand(src);
}

CallInstruction::CallInstruction(Opcode opcode, Value* callee, Value* this_value)
    : Instruction(opcode)
{
    VERIFY(is_call_opcode(opcode));
    add_operand(callee);
    add_operand(this_value);
}

NonnullOwnPtr<CallInstruction> CallInstruction::create(Opcode opcode, Value* callee, Value* this_value)
{
    VERIFY(is_call_opcode(opcode));
    return adopt_own(*new CallInstruction(opcode, callee, this_value));
}

GetByIdInstruction::GetByIdInstruction(Value* base, Bytecode::PropertyKeyTableIndex property)
    : Instruction(Opcode::GetById)
{
    add_operand(base);
    set_property_key_index(property);
}

NonnullOwnPtr<GetByIdInstruction> GetByIdInstruction::create(Value* base, Bytecode::PropertyKeyTableIndex property)
{
    return adopt_own(*new GetByIdInstruction(base, property));
}

BinaryOpInstruction::BinaryOpInstruction(Opcode opcode, Value* lhs, Value* rhs)
    : Instruction(opcode)
{
    add_operand(lhs);
    add_operand(rhs);
}

NonnullOwnPtr<BinaryOpInstruction> BinaryOpInstruction::create(Opcode opcode, Value* lhs, Value* rhs)
{
    return adopt_own(*new BinaryOpInstruction(opcode, lhs, rhs));
}

UnaryOpInstruction::UnaryOpInstruction(Opcode opcode, Value* operand)
    : Instruction(opcode)
{
    add_operand(operand);
}

NonnullOwnPtr<UnaryOpInstruction> UnaryOpInstruction::create(Opcode opcode, Value* operand)
{
    return adopt_own(*new UnaryOpInstruction(opcode, operand));
}

static inline size_t to_index(ValueIndex v) { return static_cast<u32>(v); }

Value* Instruction::result() const
{
    if (!m_result.has_value())
        return nullptr;
    return m_function->values()[to_index(*m_result)].ptr();
}

Value* Instruction::operand(size_t index) const
{
    auto const& vi = m_operands[index];
    if (!vi.has_value())
        return nullptr;
    return m_function->values()[to_index(*vi)].ptr();
}

void Instruction::set_result(Value* value)
{
    // Only opcodes that produce results should have a result set.
    // Setting nullptr is always allowed (to clear a result).
    if (value)
        VERIFY(opcode_has_result(m_opcode));
    m_result = value ? Optional<ValueIndex>(value->index()) : Optional<ValueIndex>();
    if (value)
        value->set_defining_instruction(this);
}

void Instruction::add_operand(Value* value)
{
    m_operands.append(value ? Optional<ValueIndex>(value->index()) : Optional<ValueIndex>());
    if (value)
        value->add_use(this);
}

void Instruction::set_operand(size_t index, Value* value)
{
    if (index < m_operands.size() && m_operands[index].has_value()) {
        auto* old_value = m_function->values()[to_index(*m_operands[index])].ptr();
        old_value->remove_use(this);
    }

    if (index >= m_operands.size())
        m_operands.resize(index + 1);

    m_operands[index] = value ? Optional<ValueIndex>(value->index()) : Optional<ValueIndex>();
    if (value)
        value->add_use(this);
}

void Instruction::clear_operand_uses()
{
    for (auto const& op : m_operands) {
        if (op.has_value()) {
            auto* value = m_function->values()[to_index(*op)].ptr();
            value->remove_use(this);
        }
    }
    if (m_result.has_value()) {
        auto* value = m_function->values()[to_index(*m_result)].ptr();
        value->set_defining_instruction(nullptr);
    }
}

void Instruction::remove_phi_operand(size_t index)
{
    if (m_operands[index].has_value()) {
        auto* value = m_function->values()[to_index(*m_operands[index])].ptr();
        value->remove_use(this);
    }
    m_phi_predecessors.remove(index);
    m_operands.remove(index);
}

void Instruction::add_phi_operand(BlockIndex predecessor, Value* value)
{
    VERIFY(m_opcode == Opcode::Phi);
    m_phi_predecessors.append(predecessor);
    add_operand(value);
}

void Instruction::recompute_result_type()
{
    if (!m_result.has_value())
        return;

    auto* result_value = m_function->values()[to_index(*m_result)].ptr();

    auto operand_type = [&](size_t i) -> Type {
        if (i < m_operands.size() && m_operands[i].has_value()) {
            auto* value = m_function->values()[to_index(*m_operands[i])].ptr();
            return value->type();
        }
        return Type::Unknown;
    };

    // Use the static guarantee from OpcodeTraits when available.
    auto guaranteed = opcode_guaranteed_result_type(m_opcode);
    if (guaranteed != Type::Unknown) {
        result_value->set_type(guaranteed);
        return;
    }

    // Operand-dependent cases that OpcodeTraits can't express statically.
    switch (m_opcode) {
    case Opcode::Move:
    case Opcode::LoadConstant:
        result_value->set_type(operand_type(0));
        break;

    case Opcode::ToNumeric: {
        auto t = operand_type(0);
        if (t == Type::Int32)
            result_value->set_type(Type::Int32);
        else if (t == Type::Number)
            result_value->set_type(Type::Number);
        else if (t == Type::BigInt)
            result_value->set_type(Type::BigInt);
        else
            result_value->set_type(Type::Unknown);
        break;
    }

    case Opcode::Add:
    case Opcode::Sub:
    case Opcode::Mul:
    case Opcode::Div:
    case Opcode::Mod:
    case Opcode::Exp:
        if (is_safe_numeric_type(operand_type(0)) && is_safe_numeric_type(operand_type(1)))
            result_value->set_type(Type::Number);
        else
            result_value->set_type(Type::Unknown);
        break;

    default:
        break;
    }
}

bool Instruction::try_invert_comparison()
{
    if (auto inverted = inverted_comparison_opcode(m_opcode); inverted.has_value()) {
        m_opcode = *inverted;
        return true;
    }
    return false;
}

// Check if all operands are safe primitive types (no ToPrimitive calls possible)
static bool all_operands_are_safe_primitives(Instruction const& instruction)
{
    for (size_t i = 0; i < instruction.operand_count(); ++i) {
        auto* op = instruction.operand(i);
        if (!op)
            continue;
        if (!is_safe_primitive_type(op->type()))
            return false;
    }
    return true;
}

Effects Instruction::effects() const
{
    auto const& traits = opcode_traits(m_opcode);

    // For ops marked PureOnSafePrimitives, narrow to None when all operands
    // are safe primitive types (no ToPrimitive calls possible).
    if (traits.refinement == EffectRefinement::PureOnSafePrimitives) {
        if (all_operands_are_safe_primitives(*this))
            return Effects::None;
    }

    return traits.effects;
}

bool Instruction::has_side_effects() const
{
    return has_any_flag(effects(), Effects::MayThrow | Effects::WritesState | Effects::Calls);
}

bool Instruction::is_pure() const
{
    return effects() == Effects::None;
}

bool Instruction::is_hoistable() const
{
    return effects() == Effects::None;
}

bool Instruction::may_throw() const
{
    return has_any_flag(effects(), Effects::MayThrow | Effects::Calls);
}

}
