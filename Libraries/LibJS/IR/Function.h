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

    // Instruction builders that return result Value&
    // Arithmetic
    [[nodiscard]] Value& build_add(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_sub(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_mul(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_div(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_mod(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_exp(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_negate(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_unary_plus(BasicBlock& block, Value& operand);

    // Bitwise
    [[nodiscard]] Value& build_bitwise_and(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_bitwise_or(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_bitwise_xor(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_bitwise_not(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_left_shift(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_right_shift(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_unsigned_right_shift(BasicBlock& block, Value& lhs, Value& rhs);

    // Comparison
    [[nodiscard]] Value& build_less_than(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_less_than_equals(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_greater_than(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_greater_than_equals(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_loosely_equals(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_strictly_equals(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_loosely_inequals(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_strictly_inequals(BasicBlock& block, Value& lhs, Value& rhs);

    // Type ops
    [[nodiscard]] Value& build_typeof(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_typeof_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);
    [[nodiscard]] Value& build_to_boolean(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_to_number(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_to_numeric(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_to_string(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_to_object(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_to_int32(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_to_length(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_not(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_is_undefined(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_is_nullish(BasicBlock& block, Value& operand);

    // Increment/Decrement
    [[nodiscard]] Value& build_increment(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_decrement(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_postfix_increment(BasicBlock& block, Value& operand);
    [[nodiscard]] Value& build_postfix_decrement(BasicBlock& block, Value& operand);

    // String ops
    [[nodiscard]] Value& build_concat_string(BasicBlock& block, Value& lhs, Value& rhs);

    // Constants
    [[nodiscard]] Value& build_load_constant(BasicBlock& block, JS::Value constant);
    [[nodiscard]] Value& build_load_undefined(BasicBlock& block);
    [[nodiscard]] Value& build_load_null(BasicBlock& block);

    // Property access
    [[nodiscard]] Value& build_get_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    [[nodiscard]] Value& build_get_by_id_with_this(BasicBlock& block, Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property);
    [[nodiscard]] Value& build_get_by_value(BasicBlock& block, Value& base, Value& property, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    [[nodiscard]] Value& build_get_by_value_with_this(BasicBlock& block, Value& base, Value& this_value, Value& property);
    [[nodiscard]] Value& build_get_length(BasicBlock& block, Value& base);
    void build_put_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& value);
    void build_put_by_value(BasicBlock& block, Value& base, Value& property, Value& value);
    [[nodiscard]] Value& build_delete_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property);
    [[nodiscard]] Value& build_delete_by_value(BasicBlock& block, Value& base, Value& property);
    [[nodiscard]] Value& build_has_property(BasicBlock& block, Value& object, Value& property);
    [[nodiscard]] Value& build_get_private_by_id(BasicBlock& block, Value& base, Bytecode::IdentifierTableIndex property);
    void build_put_private_by_id(BasicBlock& block, Value& base, Bytecode::IdentifierTableIndex property, Value& value);
    void build_put_getter_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& getter, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_setter_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& setter, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_prototype_by_id(BasicBlock& block, Value& base, Bytecode::PropertyKeyTableIndex property, Value& prototype, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_getter_by_id_with_this(BasicBlock& block, Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& getter);
    void build_put_setter_by_id_with_this(BasicBlock& block, Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& setter);
    void build_put_prototype_by_id_with_this(BasicBlock& block, Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& prototype);
    void build_put_getter_by_value(BasicBlock& block, Value& base, Value& property, Value& getter, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_setter_by_value(BasicBlock& block, Value& base, Value& property, Value& setter, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_prototype_by_value(BasicBlock& block, Value& base, Value& property, Value& prototype, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_getter_by_value_with_this(BasicBlock& block, Value& base, Value& property, Value& this_value, Value& getter);
    void build_put_setter_by_value_with_this(BasicBlock& block, Value& base, Value& property, Value& this_value, Value& setter);
    void build_put_prototype_by_value_with_this(BasicBlock& block, Value& base, Value& property, Value& this_value, Value& prototype);
    void build_put_by_spread(BasicBlock& block, Value& base, Value& source);

    // Calls
    [[nodiscard]] Value& build_call(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments);
    [[nodiscard]] Value& build_call_builtin(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments, Bytecode::Builtin builtin, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_call_direct_eval(BasicBlock& block, Value& callee, Value& this_value, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_call_with_argument_array(BasicBlock& block, Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_construct(BasicBlock& block, Value& callee, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string = {});
    [[nodiscard]] Value& build_construct_with_argument_array(BasicBlock& block, Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_super_call_with_argument_array(BasicBlock& block, Value& arguments, bool is_synthetic);
    [[nodiscard]] Value& build_import_call(BasicBlock& block, Value& specifier, Value& options);

    // Environment
    [[nodiscard]] Value& build_get_callee_and_this_from_environment(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);
    void build_create_variable(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Bytecode::Op::EnvironmentMode mode, bool is_immutable, bool is_global, bool is_strict);
    [[nodiscard]] Value& build_create_lexical_environment(BasicBlock& block, u32 capacity);
    void build_create_mutable_binding(BasicBlock& block, Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict);
    void build_create_immutable_binding(BasicBlock& block, Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict);
    void build_leave_lexical_environment(BasicBlock& block);
    void build_enter_object_environment(BasicBlock& block, Value& object);
    [[nodiscard]] Value& build_get_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);
    void build_initialize_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value);
    void build_set_binding(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value);
    [[nodiscard]] Value& build_get_global(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);
    void build_set_global(BasicBlock& block, Bytecode::IdentifierTableIndex identifier, Value& value);
    [[nodiscard]] Value& build_delete_variable(BasicBlock& block, Bytecode::IdentifierTableIndex identifier);
    void build_resolve_this_binding(BasicBlock& block);
    [[nodiscard]] Value& build_resolve_super_base(BasicBlock& block);
    void build_create_private_environment(BasicBlock& block);
    void build_leave_private_environment(BasicBlock& block);
    void build_add_private_name(BasicBlock& block, Bytecode::IdentifierTableIndex name);
    void build_create_variable_environment(BasicBlock& block, u32 capacity);

    // Object creation
    [[nodiscard]] Value& build_new_object(BasicBlock& block);
    [[nodiscard]] Value& build_new_array(BasicBlock& block, Span<Value*> elements);
    [[nodiscard]] Value& build_new_array_with_length(BasicBlock& block, Value& length);
    void build_array_append(BasicBlock& block, Value& array, Value& value, bool is_spread);
    [[nodiscard]] Value& build_new_class(BasicBlock& block, Value* super_class, Span<Value*> element_keys);
    [[nodiscard]] Value& build_new_function(BasicBlock& block, Value* home_object);
    [[nodiscard]] Value& build_new_regexp(BasicBlock& block, Bytecode::StringTableIndex source, Bytecode::StringTableIndex flags, Bytecode::RegexTableIndex regex);
    void build_init_object_literal_property(BasicBlock& block, Value& object, Bytecode::PropertyKeyTableIndex property, Value& value, CacheIndex shape_cache_index, PropertySlot property_slot);
    void build_cache_object_shape(BasicBlock& block, Value& object, CacheIndex cache_index);

    // Arguments
    [[nodiscard]] Value& build_create_arguments(BasicBlock& block, Bytecode::Op::ArgumentsKind kind, bool is_immutable);
    [[nodiscard]] Value& build_create_rest_params(BasicBlock& block, u32 rest_index);
    [[nodiscard]] Value& build_get_new_target(BasicBlock& block);

    // Guard operations (may throw but produce no value)
    void build_throw_if_not_object(BasicBlock& block, Value& value);
    void build_throw_if_nullish(BasicBlock& block, Value& value);
    void build_throw_if_tdz(BasicBlock& block, Value& value);

    // Iterators
    [[nodiscard]] Value& build_get_iterator(BasicBlock& block, Value& iterable);
    [[nodiscard]] Value& build_get_object_property_iterator(BasicBlock& block, Value& object);
    [[nodiscard]] Value& build_iterator_next(BasicBlock& block, Value& iterator);
    [[nodiscard]] Value& build_iterator_next_unpack(BasicBlock& block, Value& iterator);
    void build_iterator_close(BasicBlock& block, Value& iterator);
    [[nodiscard]] Value& build_iterator_to_array(BasicBlock& block, Value& iterator);

    // Special
    [[nodiscard]] Value& build_in(BasicBlock& block, Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_instance_of(BasicBlock& block, Value& lhs, Value& rhs);

    // Copy
    [[nodiscard]] Value& build_move(BasicBlock& block, Value& source);

    // Tuple extraction
    [[nodiscard]] Value& build_extract_value(BasicBlock& block, Value& tuple, u32 index);

    // Control flow (void, no result)
    void build_jump(BasicBlock& from, BasicBlock& to);
    void build_branch(BasicBlock& from, Value& condition, BasicBlock& if_true, BasicBlock& if_false);
    void build_return(BasicBlock& block, Value& value);
    void build_end(BasicBlock& block, Value& value);
    void build_throw(BasicBlock& block, Value& value);

    // Generators/Async - terminators with result (the resume value)
    // For Yield, continuation can be null for final yields (generator return)
    [[nodiscard]] Value& build_yield(BasicBlock& block, Value& value, BasicBlock* continuation);
    [[nodiscard]] Value& build_await(BasicBlock& block, Value& argument, BasicBlock& continuation);
    [[nodiscard]] Value& build_get_completion_fields(BasicBlock& block, Value& completion);

    // SSA
    [[nodiscard]] Value& build_phi(BasicBlock& block, Vector<Value*> values, Vector<BasicBlock*> predecessors);

private:
    explicit Function(GC::Ptr<Bytecode::Executable const> source_executable);

    Value& create_value_for_instruction();
    Value& build_binary_op(BasicBlock& block, Opcode opcode, Value& lhs, Value& rhs);
    Value& build_unary_op(BasicBlock& block, Opcode opcode, Value& operand);

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
