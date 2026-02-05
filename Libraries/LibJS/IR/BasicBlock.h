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
    [[nodiscard]] static NonnullOwnPtr<BasicBlock> create(BlockIndex index, String name = {});

    BlockIndex index() const { return m_index; }
    String const& name() const { return m_name; }

    Function* parent_function() const { return m_parent_function; }
    void set_parent_function(Function* function) { m_parent_function = function; }

    Vector<NonnullOwnPtr<Instruction>> const& instructions() const { return m_instructions; }
    Vector<NonnullOwnPtr<Instruction>>& instructions() { return m_instructions; }

    void append(NonnullOwnPtr<Instruction> instruction);
    void prepend(NonnullOwnPtr<Instruction> instruction);
    Instruction* last_instruction() const;

    // Returns the terminator instruction if this block is terminated, nullptr otherwise.
    // Use this instead of last_instruction() when you need to access CFG targets.
    TerminatorInstruction* terminator() const;

    Vector<BasicBlock*> const& predecessors() const { return m_predecessors; }
    void clear_predecessors() { m_predecessors.clear(); }

    // Exception handling (block-level)
    BasicBlock* exception_handler() const { return m_exception_handler; }
    BasicBlock* finalizer() const { return m_finalizer; }

    bool is_terminated() const;

private:
    friend class CFG;
    friend class Lifter;

    // CFG/EH mutation methods - only accessible through CFG:: helpers or Lifter
    void add_predecessor(BasicBlock* block);
    void remove_predecessor(BasicBlock* block);
    void remove_phi_operands_for_predecessor(BasicBlock* predecessor);
    void replace_phi_predecessor(BasicBlock* old_pred, BasicBlock* new_pred);
    void set_exception_handler(BasicBlock* handler) { m_exception_handler = handler; }
    void set_finalizer(BasicBlock* finalizer) { m_finalizer = finalizer; }

    BasicBlock(BlockIndex index, String name);

    BlockIndex m_index;
    String m_name;
    Function* m_parent_function { nullptr };
    Vector<NonnullOwnPtr<Instruction>> m_instructions;
    Vector<BasicBlock*> m_predecessors;

    BasicBlock* m_exception_handler { nullptr };
    BasicBlock* m_finalizer { nullptr };
};

}
