/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/IR/Builder.h>
#include <LibJS/IR/DominatorTree.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class SSAConstruction {
public:
    SSAConstruction(Function&, DominatorTree const&, Bytecode::Executable const&, HashTable<u32> const& written_operands, Vector<HashTable<u32>> const& block_actual_definitions, Vector<HashMap<u32, Value*>>& block_definitions, Vector<Optional<u32>>& value_to_operand_raw);

    void run();

private:
    void place_phi_nodes();
    void fill_phi_operands();
    void rename_ssa(BasicBlock& block, HashMap<u32, Vector<Value*>>& stacks);

    Function& m_function;
    DominatorTree const& m_dominators;
    Bytecode::Executable const& m_executable;
    HashTable<u32> const& m_written_operands;
    Vector<HashTable<u32>> const& m_block_actual_definitions;
    Vector<HashMap<u32, Value*>>& m_block_definitions;
    Vector<Optional<u32>>& m_value_to_operand_raw;
    Builder m_builder;
};

}
