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
#include <AK/Vector.h>
#include <LibGC/Ptr.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

enum class IRStage : u8 {
    RawCFG,
    SSA,
    OptimizedSSA,
    PostSSA,
    Lowered,
};

struct SsaConstructionData {
    HashTable<u32> written_operands;
    Vector<HashTable<u32>> block_actual_definitions;
    Vector<HashMap<u32, Value*>> block_definitions;
    Vector<Optional<u32>> value_to_operand_raw;
};

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

    BasicBlock* block_by_index(BlockIndex index) const;
    void remove_block_index(BlockIndex index);
    Vector<NonnullOwnPtr<Value>> const& values() const { return m_values; }
    Vector<Value*> const& parameters() const { return m_parameters; }

    IRStage stage() const { return m_stage; }
    void set_stage(IRStage stage) { m_stage = stage; }

    SsaConstructionData* ssa_construction_data() { return m_ssa_construction_data.ptr(); }
    void set_ssa_construction_data(OwnPtr<SsaConstructionData> data) { m_ssa_construction_data = move(data); }
    void clear_ssa_construction_data() { m_ssa_construction_data = nullptr; }

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
    IRStage m_stage { IRStage::RawCFG };
    OwnPtr<SsaConstructionData> m_ssa_construction_data;
    HashMap<u32, BasicBlock*> m_source_block_map;
    Vector<NonnullOwnPtr<BasicBlock>> m_basic_blocks;
    HashMap<BlockIndex, BasicBlock*> m_block_index_map;
    Vector<NonnullOwnPtr<Value>> m_values;
    Vector<Value*> m_parameters;
    Value* m_this_value { nullptr };
    BasicBlock* m_entry_block { nullptr };
    ValueIndex m_next_value_index { 0 };
    BlockIndex m_next_block_index { 0 };
};

}
