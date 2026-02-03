/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/NonnullOwnPtr.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class JS_API Lifter {
public:
    static NonnullOwnPtr<Function> lift(Bytecode::Executable const&);

private:
    explicit Lifter(Bytecode::Executable const&);

    void lift_basic_blocks();
    void lift_instruction(Bytecode::Instruction const&, BasicBlock&);
    void place_phi_nodes();
    void fill_phi_operands();
    Value* find_reaching_def_for_phi(BasicBlock& from_block, u32 operand_raw, HashTable<BasicBlock*>& visited);
    void connect_control_flow();
    void compute_block_predecessors();
    u32 address_to_block_index(size_t address) const;

    Value& get_or_create_value_for_operand(Bytecode::Operand operand, BasicBlock& block);
    void define_operand(Bytecode::Operand operand, Value& value, BasicBlock& block);

    Bytecode::Executable const& m_executable;
    NonnullOwnPtr<Function> m_function;

    // Maps bytecode basic block index -> IR basic block
    HashMap<u32, BasicBlock*> m_block_map;

    // Per-block definitions: maps block -> (operand raw -> Value* at end of block)
    // NB: This is a cumulative snapshot, not just what's defined in the block
    HashMap<BasicBlock*, HashMap<u32, Value*>> m_block_definitions;

    // Per-block actual definitions: operands ACTUALLY defined in each block (not inherited)
    HashMap<BasicBlock*, HashTable<u32>> m_block_actual_definitions;

    // Current block's working definitions (during lifting)
    HashMap<u32, Value*> m_current_definitions;

    // Track which operands are written to (locals that may need phi nodes)
    HashTable<u32> m_written_operands;

    // Reverse mapping: Value -> bytecode operand raw (for fixing references)
    HashMap<Value*, u32> m_value_to_operand_raw;

    // Block predecessors (computed after control flow is connected)
    HashMap<BasicBlock*, Vector<BasicBlock*>> m_predecessors;

    // Map from (block, operand) to phi value for that operand
    // Used during SSA construction to find existing phis
    HashMap<u64, Value*> m_phi_map;

    static u64 make_phi_key(BasicBlock* block, u32 operand_raw)
    {
        return (reinterpret_cast<uintptr_t>(block) << 16) ^ operand_raw;
    }
};

}
