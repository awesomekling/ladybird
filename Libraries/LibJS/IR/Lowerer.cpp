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
        for (auto const* p : path)
            m_coalesce_representative.set(p, v);
        return v;
    };

    auto coalesce = [&](Value const* a, Value const* b) {
        auto const* rep_a = find_representative(a);
        auto const* rep_b = find_representative(b);
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

            // Helper to check if coalescing this operand with the phi result would
            // create a conflict with another operand of the same phi.
            // Two operands of the same phi represent different values from different
            // control flow paths and must NOT share a register.
            auto would_conflict_with_other_operand = [&](Value const* operand_to_check) -> bool {
                auto const* rep_to_check = find_representative(operand_to_check);
                auto const* phi_rep = find_representative(phi_result);

                for (auto* other_operand : instruction->operands()) {
                    if (!other_operand || other_operand == operand_to_check)
                        continue;
                    if (other_operand->is_constant() || other_operand->is_parameter() || other_operand->is_this())
                        continue;

                    auto const* other_rep = find_representative(other_operand);

                    // If this operand is already in the same equivalence class as
                    // another operand, coalescing would make phi_result share a
                    // register with multiple different incoming values.
                    if (rep_to_check == other_rep)
                        return true;

                    // If another operand is already coalesced with phi_result,
                    // coalescing this one too would make both operands share a register.
                    if (other_rep == phi_rep)
                        return true;
                }
                return false;
            };

            auto const& phi = static_cast<PhiInstruction const&>(*instruction);

            for (size_t i = 0; i < phi.incoming_count(); ++i) {
                auto* operand = phi.incoming_value(i);
                if (!operand)
                    continue;

                // Can't coalesce constants, parameters, or this
                if (operand->is_constant() || operand->is_parameter() || operand->is_this())
                    continue;

                // Back-edge operands (from the same block as the phi) cannot be
                // coalesced with the phi result. The operand is defined later in
                // the block while the phi result is still live, so they have
                // overlapping lifetimes.
                if (phi.incoming_block(i) == block.ptr())
                    continue;

                // Check for conflicts before any coalescing
                if (would_conflict_with_other_operand(operand))
                    continue;

                // Check if the phi result is still live in the incoming block after
                // the point where the incoming value is defined. If so, they have
                // overlapping lifetimes and cannot share a register.
                auto* incoming_block = phi.incoming_block(i);
                if (auto* defining = operand->defining_instruction()) {
                    bool has_interference = false;
                    bool past_definition = false;
                    for (auto const& inst : incoming_block->instructions()) {
                        if (inst.ptr() == defining) {
                            past_definition = true;
                            continue;
                        }
                        if (past_definition) {
                            for (auto* use_operand : inst->operands()) {
                                if (use_operand == phi_result) {
                                    has_interference = true;
                                    break;
                                }
                            }
                            if (has_interference)
                                break;
                        }
                    }

                    // Also check if phi_result is used as a phi input from the
                    // same predecessor for a sibling phi in this block.
                    // If the operand's defining instruction is in the incoming
                    // block, it would overwrite phi_result's register before the
                    // phi moves read it for the sibling phi.
                    if (!has_interference && defining->parent_block() == incoming_block) {
                        for (auto const& sibling : block->instructions()) {
                            if (sibling->opcode() != Opcode::Phi)
                                break;
                            if (sibling.ptr() == instruction.ptr())
                                continue;
                            auto const& sibling_phi = static_cast<PhiInstruction const&>(*sibling);
                            for (size_t k = 0; k < sibling_phi.incoming_count(); ++k) {
                                if (sibling_phi.incoming_block(k) == incoming_block && sibling_phi.incoming_value(k) == phi_result) {
                                    has_interference = true;
                                    break;
                                }
                            }
                            if (has_interference)
                                break;
                        }
                    }

                    if (has_interference)
                        continue;
                }

                // Chain coalescing: if operand is a phi result in a DIFFERENT block,
                // coalesce the two phis. This makes chains like:
                // block1: v14 = Phi[v0,v2]
                // block2: v15 = Phi[v14,v4]
                // all share the same register.
                //
                // However, we must not coalesce two phis that form a swap cycle.
                // E.g. v1 = Phi[.., v2], v2 = Phi[.., v1] — if coalesced, both
                // map to the same register and the swap becomes a no-op.
                //
                // We also must not chain-coalesce phi results in the SAME block.
                // Two phi results in the same block both become live at the block
                // entry and may hold different values, so they need separate
                // registers unless the standard check proves otherwise.
                if (auto* def = operand->defining_instruction(); def && def->opcode() == Opcode::Phi && def->parent_block() != block.ptr()) {
                    auto const& other_phi = static_cast<PhiInstruction const&>(*def);
                    bool forms_cycle = false;
                    for (size_t j = 0; j < other_phi.incoming_count(); ++j) {
                        auto* other_input = other_phi.incoming_value(j);
                        if (!other_input)
                            continue;
                        if (find_representative(other_input) == find_representative(phi_result)) {
                            forms_cycle = true;
                            break;
                        }
                    }
                    if (!forms_cycle) {
                        // Verify the operand is dead after this phi edge.
                        // If it has uses outside its defining block (other than
                        // this phi), its live range extends through blocks where
                        // phi_result may hold a different value due to back-edge
                        // updates, making register sharing unsafe.
                        bool has_external_use = false;
                        for (auto* use : operand->uses()) {
                            if (use == instruction.ptr())
                                continue;
                            if (use->parent_block() != def->parent_block()) {
                                has_external_use = true;
                                break;
                            }
                        }
                        if (!has_external_use) {
                            coalesce(operand, phi_result);
                            continue;
                        }
                    }
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
    // Check if already allocated (idempotent)
    if (auto it = m_tuple_base_operand.find(&tuple); it != m_tuple_base_operand.end())
        return it->value;

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
    static_cast<OpType*>(slot)->set_strict(m_strict);
    if (m_current_source_record.has_value())
        m_current_block->add_source_map_entry(static_cast<u32>(slot_offset), *m_current_source_record);
}

template<typename OpType, typename... Args>
void Lowerer::emit_with_extra_operand_slots(size_t extra_operand_slots, Args&&... args)
{
    VERIFY(m_current_block);
    size_t size_to_allocate = round_up_to_power_of_two(sizeof(OpType) + (extra_operand_slots * sizeof(Bytecode::Operand)), alignof(void*));
    size_t slot_offset = m_current_block->size();
    m_current_block->grow(size_to_allocate);
    void* slot = m_current_block->data() + slot_offset;
    new (slot) OpType(forward<Args>(args)...);
    static_cast<OpType*>(slot)->set_strict(m_strict);
    if (m_current_source_record.has_value())
        m_current_block->add_source_map_entry(static_cast<u32>(slot_offset), *m_current_source_record);
}

void Lowerer::emit_phi_moves_for_successor(BasicBlock const& from, BasicBlock const& to)
{
    // Collect all (dst, src) moves for phi nodes on this edge.
    struct PhiMove {
        Bytecode::Operand dst;
        Bytecode::Operand src;
    };
    Vector<PhiMove> moves;

    for (auto const& instruction : to.instructions()) {
        if (instruction->opcode() != Opcode::Phi)
            break;

        auto const& phi = static_cast<PhiInstruction const&>(*instruction);

        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            if (phi.incoming_block(i) == &from) {
                if (!phi.incoming_value(i) || !phi.result())
                    continue;
                auto src = operand_for_value(*phi.incoming_value(i));
                auto dst = operand_for_value(*phi.result());
                if (src != dst)
                    moves.append({ dst, src });
                break;
            }
        }
    }

    // Check if any source would be clobbered by another move's destination.
    // If so, save it to a temp register first (parallel move resolution).
    HashMap<u32, Bytecode::Operand> saved;
    for (auto const& move : moves) {
        bool is_clobbered = false;
        for (auto const& other : moves) {
            if (&other != &move && other.dst == move.src) {
                is_clobbered = true;
                break;
            }
        }
        if (is_clobbered && !saved.contains(move.src.index())) {
            auto tmp = allocate_register();
            emit<Bytecode::Op::Mov>(tmp, move.src);
            saved.set(move.src.index(), tmp);
        }
    }

    // Emit moves, substituting saved temps where needed.
    for (auto const& move : moves) {
        auto actual_src = move.src;
        if (auto it = saved.find(move.src.index()); it != saved.end())
            actual_src = it->value;
        emit<Bytecode::Op::Mov>(move.dst, actual_src);
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

        auto const& phi = static_cast<PhiInstruction const&>(*instruction);

        for (size_t i = 0; i < phi.incoming_count(); ++i) {
            if (phi.incoming_block(i) == &from) {
                if (!phi.incoming_value(i) || !phi.result())
                    continue;
                auto src = operand_for_value(*phi.incoming_value(i));
                auto dst = operand_for_value(*phi.result());
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

    // Helper to collect variable-length operands starting from a given index
    auto collect_varargs = [&](size_t start_index) -> Vector<Bytecode::Operand> {
        Vector<Bytecode::Operand> args;
        for (size_t i = start_index; i < instruction.operands().size(); ++i)
            args.append(operand(i));
        return args;
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
    case Opcode::ToNumeric:
        emit<Bytecode::Op::ToNumeric>(dst(), operand(0));
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
        emit<Bytecode::Op::GetLength>(dst(), operand(0), instruction.base_identifier(), instruction.cache_index().value());
        break;

    // Property access
    case Opcode::GetById:
        emit<Bytecode::Op::GetById>(dst(), operand(0), instruction.property_key_index(), instruction.base_identifier(), instruction.cache_index().value());
        break;
    case Opcode::GetByIdWithThis:
        emit<Bytecode::Op::GetByIdWithThis>(dst(), operand(0), instruction.property_key_index(), operand(1), instruction.cache_index().value());
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
        emit<Bytecode::Op::PutNormalById>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index().value(), instruction.base_identifier());
        break;
    case Opcode::PutByIdWithThis:
        // operands: base (0), this_value (1), value (2)
        emit<Bytecode::Op::PutNormalByIdWithThis>(operand(0), operand(1), instruction.property_key_index(), operand(2), instruction.cache_index().value());
        break;
    case Opcode::PutByValue:
        emit<Bytecode::Op::PutNormalByValue>(operand(0), operand(1), operand(2), instruction.base_identifier());
        break;
    case Opcode::PutByValueWithThis:
        // operands: base (0), this_value (1), property (2), value (3)
        emit<Bytecode::Op::PutNormalByValueWithThis>(operand(0), operand(2), operand(1), operand(3));
        break;
    case Opcode::DeleteById:
        emit<Bytecode::Op::DeleteById>(dst(), operand(0), instruction.property_key_index());
        break;
    case Opcode::DeleteByIdWithThis:
        // operands: base (0), this_value (1)
        emit<Bytecode::Op::DeleteByIdWithThis>(dst(), operand(0), operand(1), instruction.property_key_index());
        break;
    case Opcode::DeleteByValue:
        emit<Bytecode::Op::DeleteByValue>(dst(), operand(0), operand(1));
        break;
    case Opcode::DeleteByValueWithThis:
        // operands: base (0), this_value (1), property (2)
        emit<Bytecode::Op::DeleteByValueWithThis>(dst(), operand(0), operand(1), operand(2));
        break;
    case Opcode::HasProperty:
        emit<Bytecode::Op::In>(dst(), operand(1), operand(0));
        break;
    case Opcode::HasPrivateId:
        emit<Bytecode::Op::HasPrivateId>(dst(), operand(0), instruction.identifier_index());
        break;
    case Opcode::GetPrivateById:
        emit<Bytecode::Op::GetPrivateById>(dst(), operand(0), instruction.identifier_index());
        break;
    case Opcode::PutPrivateById:
        emit<Bytecode::Op::PutPrivateById>(operand(0), instruction.identifier_index(), operand(1));
        break;
    case Opcode::PutGetterById:
        emit<Bytecode::Op::PutGetterById>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index().value(), instruction.base_identifier());
        break;
    case Opcode::PutSetterById:
        emit<Bytecode::Op::PutSetterById>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index().value(), instruction.base_identifier());
        break;
    case Opcode::PutPrototypeById:
        emit<Bytecode::Op::PutPrototypeById>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index().value(), instruction.base_identifier());
        break;
    case Opcode::PutGetterByIdWithThis:
        emit<Bytecode::Op::PutGetterByIdWithThis>(operand(0), operand(1), instruction.property_key_index(), operand(2), instruction.cache_index().value());
        break;
    case Opcode::PutSetterByIdWithThis:
        emit<Bytecode::Op::PutSetterByIdWithThis>(operand(0), operand(1), instruction.property_key_index(), operand(2), instruction.cache_index().value());
        break;
    case Opcode::PutPrototypeByIdWithThis:
        emit<Bytecode::Op::PutPrototypeByIdWithThis>(operand(0), operand(1), instruction.property_key_index(), operand(2), instruction.cache_index().value());
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
    case Opcode::PutGetterByValueWithThis:
        emit<Bytecode::Op::PutGetterByValueWithThis>(operand(0), operand(1), operand(2), operand(3));
        break;
    case Opcode::PutSetterByValueWithThis:
        emit<Bytecode::Op::PutSetterByValueWithThis>(operand(0), operand(1), operand(2), operand(3));
        break;
    case Opcode::PutPrototypeByValueWithThis:
        emit<Bytecode::Op::PutPrototypeByValueWithThis>(operand(0), operand(1), operand(2), operand(3));
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
    case Opcode::CreatePrivateEnvironment:
        emit<Bytecode::Op::CreatePrivateEnvironment>();
        break;
    case Opcode::LeavePrivateEnvironment:
        emit<Bytecode::Op::LeavePrivateEnvironment>();
        break;
    case Opcode::AddPrivateName:
        emit<Bytecode::Op::AddPrivateName>(instruction.identifier_index());
        break;
    case Opcode::CreateVariableEnvironment:
        emit<Bytecode::Op::CreateVariableEnvironment>(instruction.capacity());
        break;
    case Opcode::GetBinding:
        emit<Bytecode::Op::GetBinding>(dst(), instruction.identifier_index());
        break;
    case Opcode::InitializeBinding:
        if (instruction.environment_mode() == Bytecode::Op::EnvironmentMode::Var)
            emit<Bytecode::Op::InitializeVariableBinding>(instruction.identifier_index(), operand(0));
        else
            emit<Bytecode::Op::InitializeLexicalBinding>(instruction.identifier_index(), operand(0));
        break;
    case Opcode::SetBinding:
        if (instruction.environment_mode() == Bytecode::Op::EnvironmentMode::Var)
            emit<Bytecode::Op::SetVariableBinding>(instruction.identifier_index(), operand(0));
        else
            emit<Bytecode::Op::SetLexicalBinding>(instruction.identifier_index(), operand(0));
        break;
    case Opcode::GetGlobal:
        emit<Bytecode::Op::GetGlobal>(dst(), instruction.identifier_index(), instruction.cache_index().value());
        break;
    case Opcode::SetGlobal:
        emit<Bytecode::Op::SetGlobal>(instruction.identifier_index(), operand(0), instruction.cache_index().value());
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
        emit<Bytecode::Op::NewObject>(dst(), instruction.cache_index().value());
        break;

    // Postfix increment/decrement (dst gets old value, src gets mutated)
    case Opcode::PostfixIncrement:
        emit<Bytecode::Op::PostfixIncrement>(dst(), operand(0));
        break;
    case Opcode::PostfixDecrement:
        emit<Bytecode::Op::PostfixDecrement>(dst(), operand(0));
        break;

    // Exception handling (non-terminator)
    case Opcode::LeaveUnwindContext:
        emit<Bytecode::Op::LeaveUnwindContext>();
        break;
    case Opcode::LeaveFinally:
        emit<Bytecode::Op::LeaveFinally>();
        break;
    case Opcode::RestoreScheduledJump:
        emit<Bytecode::Op::RestoreScheduledJump>();
        break;
    case Opcode::SetSavedReturnValue:
        emit<Bytecode::Op::Mov>(
            Bytecode::Operand(Bytecode::Register::saved_return_value()),
            operand(0));
        break;
    case Opcode::GetException:
        emit<Bytecode::Op::Mov>(dst(), Bytecode::Operand(Bytecode::Register::exception()));
        break;
    case Opcode::SetException:
        emit<Bytecode::Op::Mov>(Bytecode::Operand(Bytecode::Register::exception()), operand(0));
        break;

    // Control flow - handled separately
    case Opcode::Jump:
    case Opcode::ContinuePendingUnwind:
    case Opcode::EnterUnwindContext:
    case Opcode::ScheduleJump:
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
    case Opcode::SetCompletionType: {
        emit<Bytecode::Op::SetCompletionType>(operand(0), instruction.completion_type());
        break;
    }
    case Opcode::NewTypeError: {
        emit<Bytecode::Op::NewTypeError>(dst(), instruction.string_table_index());
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

    // Exception handling
    case Opcode::Catch:
        emit<Bytecode::Op::Catch>(dst());
        break;

    case Opcode::SuperCallWithArgumentArray:
        emit<Bytecode::Op::SuperCallWithArgumentArray>(dst(), operand(0), instruction.is_synthetic());
        break;
    case Opcode::ImportCall:
        emit<Bytecode::Op::ImportCall>(dst(), operand(0), operand(1));
        break;

    // Call (variable-length arguments)
    case Opcode::Call: {
        // IR operands: [callee, this_value, arg0, arg1, ...]
        auto args = collect_varargs(2);
        emit_with_extra_operand_slots<Bytecode::Op::Call>(args.size(), dst(), operand(0), operand(1), instruction.expression_string(), ReadonlySpan<Bytecode::Operand> { args });
        break;
    }

    // CallBuiltin (variable-length arguments)
    case Opcode::CallBuiltin: {
        // IR operands: [callee, this_value, arg0, arg1, ...]
        auto args = collect_varargs(2);
        emit_with_extra_operand_slots<Bytecode::Op::CallBuiltin>(args.size(), dst(), operand(0), operand(1), instruction.builtin(), instruction.expression_string(), ReadonlySpan<Bytecode::Operand> { args });
        break;
    }

    // CallDirectEval (variable-length arguments)
    case Opcode::CallDirectEval: {
        // IR operands: [callee, this_value, arg0, arg1, ...]
        auto args = collect_varargs(2);
        emit_with_extra_operand_slots<Bytecode::Op::CallDirectEval>(args.size(), dst(), operand(0), operand(1), instruction.expression_string(), ReadonlySpan<Bytecode::Operand> { args });
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
        auto elements = collect_varargs(0);
        emit_with_extra_operand_slots<Bytecode::Op::NewArray>(elements.size(), dst(), ReadonlySpan<Bytecode::Operand> { elements });
        break;
    }

    case Opcode::NewArrayWithLength: {
        emit<Bytecode::Op::NewArrayWithLength>(dst(), operand(0));
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
    case Opcode::NewFunction: {
        Optional<Bytecode::Operand> home_object;
        if (!instruction.operands().is_empty())
            home_object = operand(0);
        emit<Bytecode::Op::NewFunction>(dst(), *instruction.function_node(), instruction.lhs_name(), home_object);
        break;
    }

    // NewRegExp
    case Opcode::NewRegExp:
        emit<Bytecode::Op::NewRegExp>(dst(), instruction.regex_source_index(), instruction.regex_flags_index(), instruction.regex_index());
        break;

    case Opcode::GetTemplateObject: {
        auto strings = collect_varargs(0);
        emit_with_extra_operand_slots<Bytecode::Op::GetTemplateObject>(strings.size(), dst(), instruction.cache_index().value(), ReadonlySpan<Bytecode::Operand> { strings });
        break;
    }

    case Opcode::InitObjectLiteralProperty:
        emit<Bytecode::Op::InitObjectLiteralProperty>(operand(0), instruction.property_key_index(), operand(1), instruction.cache_index().value(), instruction.property_slot().value());
        break;

    case Opcode::CacheObjectShape:
        emit<Bytecode::Op::CacheObjectShape>(operand(0), instruction.cache_index().value());
        break;

    case Opcode::CopyObjectExcludingProperties: {
        // operand(0) = from_object, operand(1..) = excluded_names
        auto excluded_names = collect_varargs(1);
        emit_with_extra_operand_slots<Bytecode::Op::CopyObjectExcludingProperties>(excluded_names.size(), dst(), operand(0), ReadonlySpan<Bytecode::Operand> { excluded_names });
        break;
    }

    // Construct (variable-length arguments)
    case Opcode::Construct: {
        // IR operands: [callee, arg0, arg1, ...]
        auto args = collect_varargs(1);
        emit_with_extra_operand_slots<Bytecode::Op::CallConstruct>(args.size(), dst(), operand(0), instruction.expression_string(), ReadonlySpan<Bytecode::Operand> { args });
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
    case Opcode::GetObjectPropertyIterator: {
        // GetObjectPropertyIterator produces a tuple of (iterator_object, iterator_next, iterator_done)
        // Allocate 3 consecutive registers for the tuple result
        auto tuple_base = allocate_tuple_registers(*instruction.result(), 3);
        auto object = operand(0);
        emit<Bytecode::Op::GetObjectPropertyIterator>(
            Bytecode::Operand(Bytecode::Register(tuple_base.index())),
            Bytecode::Operand(Bytecode::Register(tuple_base.index() + 1)),
            Bytecode::Operand(Bytecode::Register(tuple_base.index() + 2)),
            object);
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
    case Opcode::AsyncIteratorClose: {
        // Operands: [iterator_object, iterator_next, iterator_done]
        emit<Bytecode::Op::AsyncIteratorClose>(operand(0), operand(1), operand(2), Completion::Type::Normal, OptionalNone {});
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

    // Guard operations (may throw but produce no value)
    case Opcode::ThrowIfNotObject:
        emit<Bytecode::Op::ThrowIfNotObject>(operand(0));
        break;
    case Opcode::ThrowIfNullish:
        emit<Bytecode::Op::ThrowIfNullish>(operand(0));
        break;
    case Opcode::ThrowIfTDZ:
        emit<Bytecode::Op::ThrowIfTDZ>(operand(0));
        break;

    case Opcode::__Count:
        VERIFY_NOT_REACHED();
    }
    // NB: No default case - all IR opcodes must be explicitly handled above.
    // This ensures new opcodes cause a compile error rather than being silently skipped.
}

void Lowerer::lower_blocks()
{
    // Build ordered block list with entry block first.
    // After optimization passes, the entry block may not be first in the vector,
    // but bytecode execution always starts at block index 0.
    Vector<BasicBlock const*> ordered_blocks;
    auto* entry = m_function.entry_block();
    if (entry)
        ordered_blocks.append(entry);
    for (auto const& block : m_function.basic_blocks()) {
        if (block.ptr() != entry)
            ordered_blocks.append(block.ptr());
    }

    // First pass: create bytecode blocks and map IR blocks to indices
    for (size_t i = 0; i < ordered_blocks.size(); ++i) {
        m_ir_block_to_bytecode_index.set(ordered_blocks[i], i);
        auto bc_block = Bytecode::BasicBlock::create(static_cast<u32>(i), ordered_blocks[i]->name());
        m_bytecode_blocks.append(move(bc_block));
    }

    // Pre-pass: allocate tuple registers and map ExtractValue results
    // This ensures that when we encounter uses of ExtractValue results in the lowering pass,
    // the mappings are already in place (even if the ExtractValue is in a later block).
    // We do this in two phases: first allocate all tuple registers, then map all ExtractValue results.

    // Phase 1: Allocate tuple registers for all tuple-producing instructions
    for (auto const& ir_block : m_function.basic_blocks()) {
        for (auto const& instruction : ir_block->instructions()) {
            switch (instruction->opcode()) {
            case Opcode::GetIterator:
            case Opcode::GetObjectPropertyIterator:
                if (instruction->result())
                    allocate_tuple_registers(*instruction->result(), 3);
                break;
            case Opcode::IteratorNextUnpack:
            case Opcode::GetCalleeAndThisFromEnvironment:
            case Opcode::GetCompletionFields:
                if (instruction->result())
                    allocate_tuple_registers(*instruction->result(), 2);
                break;
            default:
                break;
            }
        }
    }

    // Phase 2: Map ExtractValue results to their tuple element registers
    for (auto const& ir_block : m_function.basic_blocks()) {
        for (auto const& instruction : ir_block->instructions()) {
            if (instruction->opcode() == Opcode::ExtractValue) {
                auto* tuple_value = instruction->operands()[0];
                if (tuple_value && instruction->result()) {
                    auto element_reg = operand_for_tuple_element(*tuple_value, instruction->extract_index());
                    // Follow coalescing chain to find the representative
                    Value const* rep = instruction->result();
                    for (;;) {
                        auto it = m_coalesce_representative.find(rep);
                        if (it == m_coalesce_representative.end())
                            break;
                        rep = it->value;
                    }
                    m_value_to_operand.set(rep, element_reg);
                }
            }
        }
    }

    // Main pass: lower each block
    for (size_t i = 0; i < ordered_blocks.size(); ++i) {
        auto const& ir_block = *ordered_blocks[i];
        m_current_block = m_bytecode_blocks[i].ptr();

        // Lower non-terminator instructions
        for (auto const& instruction : ir_block.instructions()) {
            if (instruction->is_terminator())
                continue;
            m_current_source_record = instruction->source_record();
            lower_instruction(*instruction);
        }
        m_current_source_record = {};

        // Handle terminator
        auto* terminator = ir_block.terminator();
        if (!terminator)
            continue;

        m_current_source_record = terminator->source_record();

        switch (terminator->opcode()) {
        case Opcode::Jump: {
            auto* target = terminator->true_target();
            if (target) {
                emit_phi_moves_for_successor(ir_block, *target);
                auto target_index = m_ir_block_to_bytecode_index.get(target).value();
                // Skip jump if target is the immediately following block (fallthrough)
                if (target_index != i + 1)
                    emit<Bytecode::Op::Jump>(Bytecode::Label { static_cast<u32>(target_index) });
            }
            break;
        }
        case Opcode::ContinuePendingUnwind: {
            auto* target = terminator->true_target();
            VERIFY(target);
            emit_phi_moves_for_successor(ir_block, *target);
            auto target_index = m_ir_block_to_bytecode_index.get(target).value();
            emit<Bytecode::Op::ContinuePendingUnwind>(Bytecode::Label { static_cast<u32>(target_index) });
            break;
        }
        case Opcode::EnterUnwindContext: {
            auto* target = terminator->true_target();
            VERIFY(target);
            auto target_index = m_ir_block_to_bytecode_index.get(target).value();
            emit<Bytecode::Op::EnterUnwindContext>(Bytecode::Label { static_cast<u32>(target_index) });
            break;
        }
        case Opcode::ScheduleJump: {
            // false_target is the deferred jump target (stored in the instruction)
            auto* deferred_target = terminator->false_target();
            VERIFY(deferred_target);
            auto target_index = m_ir_block_to_bytecode_index.get(deferred_target).value();
            emit<Bytecode::Op::ScheduleJump>(Bytecode::Label { static_cast<u32>(target_index) });
            // NB: The finalizer (true_target) is not encoded in the ScheduleJump instruction —
            // the interpreter finds it via exception_handlers_for_offset at runtime.
            break;
        }
        case Opcode::Branch: {
            auto* condition_value = terminator->operands()[0];
            auto* true_target = terminator->true_target();
            auto* false_target = terminator->false_target();

            if (true_target && false_target && false_target != true_target) {
                // Both targets exist and are different - check if we need critical edge splitting
                bool true_needs_moves = needs_phi_moves_for_edge(ir_block, *true_target);
                bool false_needs_moves = needs_phi_moves_for_edge(ir_block, *false_target);

                size_t true_index = 0;
                size_t false_index = 0;

                if (true_needs_moves && false_needs_moves) {
                    // Critical edges: both targets need phi moves, so we need trampolines
                    // to avoid phi move conflicts
                    true_index = get_or_create_trampoline(ir_block, *true_target);
                    false_index = get_or_create_trampoline(ir_block, *false_target);
                } else if (true_needs_moves) {
                    // Only true target needs phi moves - use trampoline for true
                    true_index = get_or_create_trampoline(ir_block, *true_target);
                    false_index = m_ir_block_to_bytecode_index.get(false_target).value();
                } else if (false_needs_moves) {
                    // Only false target needs phi moves - use trampoline for false
                    true_index = m_ir_block_to_bytecode_index.get(true_target).value();
                    false_index = get_or_create_trampoline(ir_block, *false_target);
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
                    emit_phi_moves_for_successor(ir_block, *true_target);
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
                emit_phi_moves_for_successor(ir_block, *continuation);
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
            emit_phi_moves_for_successor(ir_block, *continuation);
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
    if (function.source_executable())
        lowerer.m_strict = function.source_executable()->is_strict_mode ? Strict::Yes : Strict::No;
    lowerer.compute_phi_coalescing();
    lowerer.lower_blocks();

    auto source_executable = function.source_executable();
    auto const number_of_registers = lowerer.m_next_register;
    auto const number_of_constants = lowerer.m_constants.size();

    // Combine all bytecode blocks into one flat buffer, tracking block offsets and label positions
    Vector<u8> bytecode;
    Vector<size_t> basic_block_start_offsets;
    Vector<size_t> label_offsets;
    Vector<Bytecode::SourceMapEntry> source_map;
    HashMap<Bytecode::BasicBlock const*, size_t> block_offsets;

    for (auto const& block : lowerer.m_bytecode_blocks) {
        basic_block_start_offsets.append(bytecode.size());
        block_offsets.set(block.ptr(), bytecode.size());

        // Merge per-block source map entries, adjusting offsets to flat buffer positions
        for (auto const& entry : block->source_map())
            source_map.append({ static_cast<u32>(bytecode.size()) + entry.bytecode_offset, entry.source_record });

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
    executable->source_map = move(source_map);
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

    // Generate exception handlers from IR block annotations.
    // Each IR block may have an exception_handler and/or finalizer pointer
    // that was set during lifting and preserved through optimization passes.
    for (auto const& ir_block : function.basic_blocks()) {
        auto* handler_block = ir_block->exception_handler();
        auto* finalizer_block = ir_block->finalizer();

        if (!handler_block && !finalizer_block)
            continue;

        auto block_idx_it = lowerer.m_ir_block_to_bytecode_index.find(ir_block.ptr());
        if (block_idx_it == lowerer.m_ir_block_to_bytecode_index.end())
            continue;
        auto block_idx = block_idx_it->value;

        auto start_offset = executable->basic_block_start_offsets[block_idx];
        auto end_offset = (block_idx + 1 < executable->basic_block_start_offsets.size())
            ? executable->basic_block_start_offsets[block_idx + 1]
            : executable->bytecode.size();

        if (start_offset == end_offset)
            continue;

        Bytecode::Executable::ExceptionHandlers handler;
        handler.start_offset = start_offset;
        handler.end_offset = end_offset;

        if (handler_block) {
            auto it = lowerer.m_ir_block_to_bytecode_index.find(handler_block);
            if (it != lowerer.m_ir_block_to_bytecode_index.end())
                handler.handler_offset = executable->basic_block_start_offsets[it->value];
        }
        if (finalizer_block) {
            auto it = lowerer.m_ir_block_to_bytecode_index.find(finalizer_block);
            if (it != lowerer.m_ir_block_to_bytecode_index.end())
                handler.finalizer_offset = executable->basic_block_start_offsets[it->value];
        }

        executable->exception_handlers.append(handler);
    }

    return executable;
}

}
