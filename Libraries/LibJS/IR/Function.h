/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/Vector.h>
#include <LibGC/Ptr.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class JS_API Function {
    AK_MAKE_NONCOPYABLE(Function);
    AK_MAKE_NONMOVABLE(Function);

public:
    [[nodiscard]] static NonnullOwnPtr<Function> create(GC::Ptr<Bytecode::Executable const> source_executable = nullptr);

    GC::Ptr<Bytecode::Executable const> source_executable() const { return m_source_executable; }

    // Mapping from source bytecode block index to IR block (set by lifter)
    HashMap<u32, BasicBlock*> const& source_block_map() const { return m_source_block_map; }
    void set_source_block_map(HashMap<u32, BasicBlock*> map) { m_source_block_map = move(map); }

    Vector<NonnullOwnPtr<BasicBlock>> const& basic_blocks() const { return m_basic_blocks; }
    Vector<NonnullOwnPtr<BasicBlock>>& basic_blocks() { return m_basic_blocks; }
    Vector<NonnullOwnPtr<Value>> const& values() const { return m_values; }
    Vector<Value*> const& parameters() const { return m_parameters; }

    BasicBlock* entry_block() const { return m_entry_block; }
    void set_entry_block(BasicBlock* block) { m_entry_block = block; }

    // Factory methods for blocks
    [[nodiscard]] BasicBlock& create_block(String name = {});

    // Factory methods for values
    [[nodiscard]] Value& create_parameter(u32 parameter_index);
    [[nodiscard]] Value& create_this();
    [[nodiscard]] Value& create_register_value();
    [[nodiscard]] Value& create_constant(JS::Value constant);
    [[nodiscard]] Value& create_value_for_instruction();

private:
    explicit Function(GC::Ptr<Bytecode::Executable const> source_executable);

    GC::Ptr<Bytecode::Executable const> m_source_executable;
    HashMap<u32, BasicBlock*> m_source_block_map;
    Vector<NonnullOwnPtr<BasicBlock>> m_basic_blocks;
    Vector<NonnullOwnPtr<Value>> m_values;
    Vector<Value*> m_parameters;
    Value* m_this_value { nullptr };
    BasicBlock* m_entry_block { nullptr };
    ValueIndex m_next_value_index { 0 };
    BlockIndex m_next_block_index { 0 };
};

}
