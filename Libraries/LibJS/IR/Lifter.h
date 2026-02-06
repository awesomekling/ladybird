/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/NonnullOwnPtr.h>
#include <AK/OwnPtr.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Builder.h>
#include <LibJS/IR/DominatorTree.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class JS_API Lifter {
public:
    static NonnullOwnPtr<Function> lift(Bytecode::Executable const&);

private:
    explicit Lifter(Bytecode::Executable const&);

    void lift_basic_blocks();
    void lift_instruction(Bytecode::Instruction const&, BasicBlock&);
    void connect_control_flow();
    void compute_block_predecessors();
    void compute_dominators();
    void eliminate_unreachable_blocks();
    u32 address_to_block_index(size_t address) const;

    Value& get_or_create_value_for_operand(Bytecode::Operand operand, BasicBlock& block);
    void define_operand(Bytecode::Operand operand, Value& value, BasicBlock& block);

    // Lifting helpers to reduce boilerplate for common instruction patterns
    template<typename BytecodeOp>
    void lift_binary_op(Bytecode::Instruction const&, BasicBlock&, Value& (Builder::*)(Value&, Value&));

    template<typename BytecodeOp>
    void lift_unary_op_src(Bytecode::Instruction const&, BasicBlock&, Value& (Builder::*)(Value&));

    template<typename BytecodeOp>
    void lift_unary_op_value(Bytecode::Instruction const&, BasicBlock&, Value& (Builder::*)(Value&));

    Bytecode::Executable const& m_executable;
    NonnullOwnPtr<Function> m_function;
    Builder m_builder;

    // Maps bytecode basic block index -> IR basic block (first block for each bytecode block)
    HashMap<u32, BasicBlock*> m_block_map;

    // Maps bytecode basic block index -> final IR block (after EH splits)
    // Used by connect_control_flow() to add terminators to the correct block
    HashMap<u32, BasicBlock*> m_final_ir_block;

    // Per-block definitions: indexed by block index -> (operand raw -> Value* at end of block)
    // NB: This is a cumulative snapshot, not just what's defined in the block
    Vector<HashMap<u32, Value*>> m_block_definitions;

    // Per-block actual definitions: operands ACTUALLY defined in each block (not inherited)
    Vector<HashTable<u32>> m_block_actual_definitions;

    // Current block's working definitions (during lifting)
    HashMap<u32, Value*> m_current_definitions;

    // Track which operands are written to (locals that may need phi nodes)
    HashTable<u32> m_written_operands;

    // Reverse mapping: Value index -> bytecode operand raw (for fixing references)
    Vector<Optional<u32>> m_value_to_operand_raw;

    // Dominator information for proper SSA construction
    OwnPtr<DominatorTree> m_dominators;
};

}
