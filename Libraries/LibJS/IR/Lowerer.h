/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/NonnullOwnPtr.h>
#include <AK/Optional.h>
#include <LibJS/Bytecode/BasicBlock.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Bytecode/Label.h>
#include <LibJS/Bytecode/Operand.h>
#include <LibJS/Bytecode/Register.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class Lowerer {
public:
    static GC::Ref<Bytecode::Executable> lower(VM&, Function const&);

private:
    explicit Lowerer(VM&, Function const&);

    void lower_blocks();
    void lower_instruction(Instruction const&);
    void emit_phi_moves_for_successor(BasicBlock const& from, BasicBlock const& to);
    bool target_has_phis(BasicBlock const& target) const;
    bool needs_phi_moves_for_edge(BasicBlock const& from, BasicBlock const& to);
    size_t get_or_create_trampoline(BasicBlock const& from, BasicBlock const& to);
    void compute_phi_coalescing();

    template<typename OpType, typename... Args>
    void emit(Args&&... args);

    template<typename OpType, typename... Args>
    void emit_with_extra_operand_slots(size_t extra_operand_slots, Args&&... args);

    Bytecode::Operand operand_for_value(Value const&);
    Bytecode::Operand allocate_register();
    Bytecode::Operand allocate_tuple_registers(Value const&, u32 count);
    Bytecode::Operand operand_for_tuple_element(Value const& tuple, u32 index);
    u32 get_or_add_constant(JS::Value);

    VM& m_vm;
    Function const& m_function;
    Vector<NonnullOwnPtr<Bytecode::BasicBlock>> m_bytecode_blocks;
    Bytecode::BasicBlock* m_current_block { nullptr };
    Vector<Optional<Bytecode::Operand>> m_value_to_operand;
    Vector<Optional<Bytecode::Operand>> m_tuple_base_operand;
    HashMap<BasicBlock const*, size_t> m_ir_block_to_bytecode_index;
    // Phi coalescing: maps values to their coalescing representative (nullptr = self)
    Vector<Value const*> m_coalesce_representative;
    // Maps (from_block, to_block) pairs to trampoline block indices for critical edge splitting
    HashMap<u64, size_t> m_edge_to_trampoline;
    Vector<JS::Value> m_constants;
    u32 m_next_register { Bytecode::Register::reserved_register_count };
    Optional<Bytecode::SourceRecord> m_current_source_record;
    Strict m_strict { Strict::No };
};

}
