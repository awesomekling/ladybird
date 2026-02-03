/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/Span.h>
#include <AK/Vector.h>
#include <LibGC/Ptr.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Bytecode/Instruction.h>
#include <LibJS/Bytecode/RegexTable.h>
#include <LibJS/Bytecode/StringTable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>

namespace JS::IR {

class JS_API Function {
    AK_MAKE_NONCOPYABLE(Function);
    AK_MAKE_NONMOVABLE(Function);

public:
    static NonnullOwnPtr<Function> create(GC::Ptr<Bytecode::Executable const> source_executable = nullptr);

    GC::Ptr<Bytecode::Executable const> source_executable() const { return m_source_executable; }

    Vector<NonnullOwnPtr<BasicBlock>> const& basic_blocks() const { return m_basic_blocks; }
    Vector<NonnullOwnPtr<BasicBlock>>& basic_blocks() { return m_basic_blocks; }
    Vector<NonnullOwnPtr<Value>> const& values() const { return m_values; }
    Vector<Value*> const& parameters() const { return m_parameters; }

    BasicBlock* entry_block() const { return m_entry_block; }
    void set_entry_block(BasicBlock* block) { m_entry_block = block; }

    // Factory methods for blocks
    BasicBlock& create_block(String name = {});

    // Factory methods for values
    Value& create_parameter(u32 parameter_index);
    Value& create_this();
    Value& create_register_value();
    Value& create_constant(JS::Value constant);

    // Instruction builders that return result Value&
    // Arithmetic
    Value& build_add(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_sub(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_mul(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_div(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_mod(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_exp(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_negate(BasicBlock& block, Value& operand);
    Value& build_unary_plus(BasicBlock& block, Value& operand);

    // Bitwise
    Value& build_bitwise_and(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_bitwise_or(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_bitwise_xor(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_bitwise_not(BasicBlock& block, Value& operand);
    Value& build_left_shift(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_right_shift(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_unsigned_right_shift(BasicBlock& block, Value& lhs, Value& rhs);

    // Comparison
    Value& build_less_than(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_less_than_equals(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_greater_than(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_greater_than_equals(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_loosely_equals(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_strictly_equals(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_loosely_inequals(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_strictly_inequals(BasicBlock& block, Value& lhs, Value& rhs);

    // Type ops
    Value& build_typeof(BasicBlock& block, Value& operand);
    Value& build_typeof_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);
    Value& build_to_boolean(BasicBlock& block, Value& operand);
    Value& build_to_number(BasicBlock& block, Value& operand);
    Value& build_to_string(BasicBlock& block, Value& operand);
    Value& build_to_object(BasicBlock& block, Value& operand);
    Value& build_to_int32(BasicBlock& block, Value& operand);
    Value& build_to_length(BasicBlock& block, Value& operand);
    Value& build_not(BasicBlock& block, Value& operand);

    // Increment/Decrement
    Value& build_increment(BasicBlock& block, Value& operand);
    Value& build_decrement(BasicBlock& block, Value& operand);
    Value& build_postfix_increment(BasicBlock& block, Value& operand);
    Value& build_postfix_decrement(BasicBlock& block, Value& operand);

    // String ops
    Value& build_concat_string(BasicBlock& block, Value& lhs, Value& rhs);

    // Constants
    Value& build_load_constant(BasicBlock& block, JS::Value constant);
    Value& build_load_undefined(BasicBlock& block);
    Value& build_load_null(BasicBlock& block);

    // Property access
    Value& build_get_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property);
    Value& build_get_by_value(BasicBlock& block, Value& base, Value& property);
    Value& build_get_length(BasicBlock& block, Value& base);
    void build_put_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& value);
    void build_put_by_value(BasicBlock& block, Value& base, Value& property, Value& value);
    Value& build_delete_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property);
    Value& build_delete_by_value(BasicBlock& block, Value& base, Value& property);
    Value& build_has_property(BasicBlock& block, Value& object, Value& property);

    // Calls
    Value& build_call(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments);
    Value& build_call_builtin(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments, Bytecode::Builtin builtin, Optional<Bytecode::StringTableIndex> expression_string);
    Value& build_call_direct_eval(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string);
    Value& build_call_with_argument_array(BasicBlock& block, Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string);
    Value& build_construct(BasicBlock& block, Value& callee, Span<Value*> arguments);
    Value& build_construct_with_argument_array(BasicBlock& block, Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string);

    // Environment
    void build_create_variable(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Bytecode::Op::EnvironmentMode mode, bool is_immutable, bool is_global, bool is_strict);
    Value& build_create_lexical_environment(BasicBlock& block, u32 capacity);
    void build_create_mutable_binding(BasicBlock& block, Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict);
    void build_create_immutable_binding(BasicBlock& block, Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict);
    void build_leave_lexical_environment(BasicBlock& block);
    Value& build_get_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);
    void build_initialize_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value);
    void build_set_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value);
    Value& build_get_global(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);
    void build_set_global(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value);
    Value& build_delete_variable(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);

    // Object creation
    Value& build_new_object(BasicBlock& block);
    Value& build_new_array(BasicBlock& block, Span<Value*> elements);
    Value& build_new_function(BasicBlock& block);
    Value& build_new_regexp(BasicBlock& block, Bytecode::StringTableIndex source, Bytecode::StringTableIndex flags, Bytecode::RegexTableIndex regex);
    void build_init_object_literal_property(BasicBlock& block, Value& object, Bytecode::PropertyKeyTableIndex property, Value& value, u32 shape_cache_index, u32 property_slot);
    void build_cache_object_shape(BasicBlock& block, Value& object, u32 cache_index);

    // Iterators
    Value& build_get_iterator(BasicBlock& block, Value& iterable);
    Value& build_iterator_next(BasicBlock& block, Value& iterator);
    Value& build_iterator_next_unpack(BasicBlock& block, Value& iterator);
    void build_iterator_close(BasicBlock& block, Value& iterator);
    Value& build_iterator_to_array(BasicBlock& block, Value& iterator);

    // Special
    Value& build_in(BasicBlock& block, Value& lhs, Value& rhs);
    Value& build_instance_of(BasicBlock& block, Value& lhs, Value& rhs);

    // Copy
    Value& build_move(BasicBlock& block, Value& source);

    // Tuple extraction
    Value& build_extract_value(BasicBlock& block, Value& tuple, u32 index);

    // Control flow (void, no result)
    void build_jump(BasicBlock& from, BasicBlock& to);
    void build_branch(BasicBlock& from, Value& condition, BasicBlock& if_true, BasicBlock& if_false);
    void build_return(BasicBlock& block, Value& value);
    void build_throw(BasicBlock& block, Value& value);

    // SSA
    Value& build_phi(BasicBlock& block, Vector<Value*> values, Vector<BasicBlock*> predecessors);

private:
    explicit Function(GC::Ptr<Bytecode::Executable const> source_executable);

    Value& create_value_for_instruction();
    Value& build_binary_op(BasicBlock& block, Opcode opcode, Value& lhs, Value& rhs);
    Value& build_unary_op(BasicBlock& block, Opcode opcode, Value& operand);

    GC::Ptr<Bytecode::Executable const> m_source_executable;
    Vector<NonnullOwnPtr<BasicBlock>> m_basic_blocks;
    Vector<NonnullOwnPtr<Value>> m_values;
    Vector<Value*> m_parameters;
    Value* m_this_value { nullptr };
    BasicBlock* m_entry_block { nullptr };
    u32 m_next_value_index { 0 };
    u32 m_next_block_index { 0 };
};

}
