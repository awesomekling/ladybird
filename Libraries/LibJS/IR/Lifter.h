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
    void insert_phi_nodes();
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
    HashMap<BasicBlock*, HashMap<u32, Value*>> m_block_definitions;

    // Current block's working definitions (during lifting)
    HashMap<u32, Value*> m_current_definitions;

    // Track which operands are written to (locals that may need phi nodes)
    HashTable<u32> m_written_operands;

    // Block predecessors (computed after control flow is connected)
    HashMap<BasicBlock*, Vector<BasicBlock*>> m_predecessors;
};

}
