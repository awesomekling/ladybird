/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
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
    u32 address_to_block_index(size_t address) const;

    Value& get_or_create_value_for_operand(Bytecode::Operand operand);
    Value& create_value_for_destination(Bytecode::Operand operand);

    Bytecode::Executable const& m_executable;
    NonnullOwnPtr<Function> m_function;

    // Maps bytecode basic block index -> IR basic block
    HashMap<u32, BasicBlock*> m_block_map;

    // Maps operand raw value -> current SSA value
    // This gets updated as we process instructions
    HashMap<u32, Value*> m_operand_to_value;
};

}
