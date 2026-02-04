/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/String.h>
#include <AK/Vector.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class JS_API BasicBlock {
    AK_MAKE_NONCOPYABLE(BasicBlock);
    AK_MAKE_NONMOVABLE(BasicBlock);

public:
    static NonnullOwnPtr<BasicBlock> create(u32 index, String name = {});

    u32 index() const { return m_index; }
    String const& name() const { return m_name; }

    Function* parent_function() const { return m_parent_function; }
    void set_parent_function(Function* function) { m_parent_function = function; }

    Vector<NonnullOwnPtr<Instruction>> const& instructions() const { return m_instructions; }
    Vector<NonnullOwnPtr<Instruction>>& instructions() { return m_instructions; }

    void append(NonnullOwnPtr<Instruction> instruction);
    void prepend(NonnullOwnPtr<Instruction> instruction);
    Instruction* last_instruction() const;

    Vector<BasicBlock*> const& predecessors() const { return m_predecessors; }
    void add_predecessor(BasicBlock* block);
    void remove_predecessor(BasicBlock* block);

    // CFG update helpers for phi node consistency
    // Remove phi operands for edges from the given predecessor
    void remove_phi_operands_for_predecessor(BasicBlock* predecessor);
    // Replace a predecessor in all phi nodes (used when merging blocks)
    void replace_phi_predecessor(BasicBlock* old_pred, BasicBlock* new_pred);

    // Exception handling (block-level)
    BasicBlock* exception_handler() const { return m_exception_handler; }
    void set_exception_handler(BasicBlock* handler) { m_exception_handler = handler; }

    BasicBlock* finalizer() const { return m_finalizer; }
    void set_finalizer(BasicBlock* finalizer) { m_finalizer = finalizer; }

    bool is_terminated() const;

private:
    BasicBlock(u32 index, String name);

    u32 m_index { 0 };
    String m_name;
    Function* m_parent_function { nullptr };
    Vector<NonnullOwnPtr<Instruction>> m_instructions;
    Vector<BasicBlock*> m_predecessors;

    BasicBlock* m_exception_handler { nullptr };
    BasicBlock* m_finalizer { nullptr };
};

}
