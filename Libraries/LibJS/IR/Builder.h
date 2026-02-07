/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Span.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Bytecode/Instruction.h>
#include <LibJS/Bytecode/PutKind.h>
#include <LibJS/Bytecode/RegexTable.h>
#include <LibJS/Bytecode/StringTable.h>
#include <LibJS/Export.h>
#include <LibJS/IR/Forward.h>
#include <LibJS/Runtime/Completion.h>

namespace JS::IR {

class JS_API Builder {
public:
    explicit Builder(Function& function)
        : m_function(function)
    {
    }

    void set_insertion_block(BasicBlock* block) { m_insertion_block = block; }
    BasicBlock* insertion_block() const { return m_insertion_block; }

    // Arithmetic
    [[nodiscard]] Value& build_add(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_sub(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_mul(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_div(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_mod(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_exp(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_negate(Value& operand);
    [[nodiscard]] Value& build_unary_plus(Value& operand);

    // Bitwise
    [[nodiscard]] Value& build_bitwise_and(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_bitwise_or(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_bitwise_xor(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_bitwise_not(Value& operand);
    [[nodiscard]] Value& build_left_shift(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_right_shift(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_unsigned_right_shift(Value& lhs, Value& rhs);

    // Comparison
    [[nodiscard]] Value& build_less_than(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_less_than_equals(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_greater_than(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_greater_than_equals(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_loosely_equals(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_strictly_equals(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_loosely_inequals(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_strictly_inequals(Value& lhs, Value& rhs);

    // Type ops
    [[nodiscard]] Value& build_typeof(Value& operand);
    [[nodiscard]] Value& build_typeof_binding(Bytecode::IdentifierTableIndex identifier);
    [[nodiscard]] Value& build_to_boolean(Value& operand);
    [[nodiscard]] Value& build_to_number(Value& operand);
    [[nodiscard]] Value& build_to_numeric(Value& operand);
    [[nodiscard]] Value& build_to_string(Value& operand);
    [[nodiscard]] Value& build_to_object(Value& operand);
    [[nodiscard]] Value& build_to_int32(Value& operand);
    [[nodiscard]] Value& build_to_length(Value& operand);
    [[nodiscard]] Value& build_not(Value& operand);
    [[nodiscard]] Value& build_is_undefined(Value& operand);
    [[nodiscard]] Value& build_is_nullish(Value& operand);
    [[nodiscard]] Value& build_is_callable(Value& operand);
    [[nodiscard]] Value& build_is_constructor(Value& operand);

    // Increment/Decrement
    [[nodiscard]] Value& build_increment(Value& operand);
    [[nodiscard]] Value& build_decrement(Value& operand);
    [[nodiscard]] Value& build_postfix_increment(Value& operand);
    [[nodiscard]] Value& build_postfix_decrement(Value& operand);

    // String ops
    [[nodiscard]] Value& build_concat_string(Value& lhs, Value& rhs);

    // Constants
    [[nodiscard]] Value& build_load_constant(JS::Value constant);
    [[nodiscard]] Value& build_load_undefined();
    [[nodiscard]] Value& build_load_null();

    // Property access
    [[nodiscard]] Value& build_get_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    [[nodiscard]] Value& build_get_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property);
    [[nodiscard]] Value& build_get_by_value(Value& base, Value& property, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    [[nodiscard]] Value& build_get_by_value_with_this(Value& base, Value& this_value, Value& property);
    [[nodiscard]] Value& build_get_method(Value& base, Bytecode::PropertyKeyTableIndex property);
    [[nodiscard]] Value& build_get_length(Value& base, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Value& value, Optional<Bytecode::IdentifierTableIndex> base_identifier = {}, Bytecode::PutKind = Bytecode::PutKind::Normal);
    void build_put_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& value, Bytecode::PutKind = Bytecode::PutKind::Normal);
    void build_put_by_value(Value& base, Value& property, Value& value, Optional<Bytecode::IdentifierTableIndex> base_identifier = {}, Bytecode::PutKind = Bytecode::PutKind::Normal);
    void build_put_by_value_with_this(Value& base, Value& this_value, Value& property, Value& value, Bytecode::PutKind = Bytecode::PutKind::Normal);
    [[nodiscard]] Value& build_delete_by_id(Value& base, Bytecode::PropertyKeyTableIndex property);
    [[nodiscard]] Value& build_delete_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property);
    [[nodiscard]] Value& build_delete_by_value(Value& base, Value& property);
    [[nodiscard]] Value& build_delete_by_value_with_this(Value& base, Value& this_value, Value& property);
    [[nodiscard]] Value& build_has_property(Value& object, Value& property);
    [[nodiscard]] Value& build_has_private_id(Value& base, Bytecode::IdentifierTableIndex property);
    [[nodiscard]] Value& build_get_private_by_id(Value& base, Bytecode::IdentifierTableIndex property);
    void build_put_private_by_id(Value& base, Bytecode::IdentifierTableIndex property, Value& value);
    void build_put_getter_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Value& getter, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_setter_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Value& setter, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_prototype_by_id(Value& base, Bytecode::PropertyKeyTableIndex property, Value& prototype, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_getter_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& getter);
    void build_put_setter_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& setter);
    void build_put_prototype_by_id_with_this(Value& base, Value& this_value, Bytecode::PropertyKeyTableIndex property, Value& prototype);
    void build_put_getter_by_value(Value& base, Value& property, Value& getter, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_setter_by_value(Value& base, Value& property, Value& setter, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_prototype_by_value(Value& base, Value& property, Value& prototype, Optional<Bytecode::IdentifierTableIndex> base_identifier = {});
    void build_put_getter_by_value_with_this(Value& base, Value& property, Value& this_value, Value& getter);
    void build_put_setter_by_value_with_this(Value& base, Value& property, Value& this_value, Value& setter);
    void build_put_prototype_by_value_with_this(Value& base, Value& property, Value& this_value, Value& prototype);
    void build_put_by_spread(Value& base, Value& source);

    // Calls
    [[nodiscard]] Value& build_call(Value& callee, Value& this_value, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string = {});
    [[nodiscard]] Value& build_call_builtin(Value& callee, Value& this_value, Span<Value*> arguments, Bytecode::Builtin builtin, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_call_direct_eval(Value& callee, Value& this_value, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_call_direct_eval_with_argument_array(Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_call_with_argument_array(Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_construct(Value& callee, Span<Value*> arguments, Optional<Bytecode::StringTableIndex> expression_string = {});
    [[nodiscard]] Value& build_construct_with_argument_array(Value& callee, Value& this_value, Value& arguments, Optional<Bytecode::StringTableIndex> expression_string);
    [[nodiscard]] Value& build_super_call_with_argument_array(Value& arguments, bool is_synthetic);
    [[nodiscard]] Value& build_import_call(Value& specifier, Value& options);

    // Environment
    [[nodiscard]] Value& build_get_callee_and_this_from_environment(Bytecode::IdentifierTableIndex identifier);
    void build_create_variable(Bytecode::IdentifierTableIndex identifier, Bytecode::Op::EnvironmentMode mode, bool is_immutable, bool is_global, bool is_strict);
    [[nodiscard]] Value& build_create_lexical_environment(u32 capacity);
    void build_create_mutable_binding(Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict);
    void build_create_immutable_binding(Value& environment, Bytecode::IdentifierTableIndex identifier, bool is_strict);
    void build_leave_lexical_environment();
    void build_enter_object_environment(Value& object);
    [[nodiscard]] Value& build_get_binding(Bytecode::IdentifierTableIndex identifier);
    void build_initialize_binding(Bytecode::IdentifierTableIndex identifier, Value& value, Bytecode::Op::EnvironmentMode);
    void build_set_binding(Bytecode::IdentifierTableIndex identifier, Value& value, Bytecode::Op::EnvironmentMode);
    [[nodiscard]] Value& build_get_global(Bytecode::IdentifierTableIndex identifier);
    void build_set_global(Bytecode::IdentifierTableIndex identifier, Value& value);
    [[nodiscard]] Value& build_delete_variable(Bytecode::IdentifierTableIndex identifier);
    void build_resolve_this_binding();
    [[nodiscard]] Value& build_resolve_super_base();
    void build_create_private_environment();
    void build_leave_private_environment();
    void build_add_private_name(Bytecode::IdentifierTableIndex name);
    void build_create_variable_environment(u32 capacity);

    // Object creation
    [[nodiscard]] Value& build_new_object();
    [[nodiscard]] Value& build_new_array(Span<Value*> elements);
    [[nodiscard]] Value& build_new_array_with_length(Value& length);
    void build_array_append(Value& array, Value& value, bool is_spread);
    [[nodiscard]] Value& build_new_class(Value* super_class, Span<Value*> element_keys);
    [[nodiscard]] Value& build_new_function(Value* home_object);
    [[nodiscard]] Value& build_new_regexp(Bytecode::StringTableIndex source, Bytecode::StringTableIndex flags, Bytecode::RegexTableIndex regex);
    [[nodiscard]] Value& build_get_template_object(Span<Value*> strings, u32 cache_index);
    void build_init_object_literal_property(Value& object, Bytecode::PropertyKeyTableIndex property, Value& value, CacheIndex shape_cache_index, PropertySlot property_slot);
    void build_cache_object_shape(Value& object, CacheIndex cache_index);
    [[nodiscard]] Value& build_copy_object_excluding_properties(Value& from_object, Span<Value*> excluded_names);

    // Arguments
    [[nodiscard]] Value& build_create_arguments(Bytecode::Op::ArgumentsKind kind, bool is_immutable, bool needs_dst);
    [[nodiscard]] Value& build_create_rest_params(u32 rest_index);
    [[nodiscard]] Value& build_get_new_target();
    [[nodiscard]] Value& build_get_import_meta();

    // Exception handling
    [[nodiscard]] Value& build_catch();
    void build_enter_unwind_context(BasicBlock& entry_point);
    void build_leave_unwind_context();
    void build_schedule_jump(BasicBlock& finalizer, BasicBlock& deferred_target);
    void build_leave_finally();
    void build_restore_scheduled_jump();
    void build_set_saved_return_value(Value& value);
    void build_prepare_yield(Value& value);
    [[nodiscard]] Value& build_get_exception();
    void build_set_exception(Value& value);

    // Guard operations (may throw but produce no value)
    void build_throw_if_not_object(Value& value);
    void build_throw_if_nullish(Value& value);
    void build_throw_if_tdz(Value& value);

    // Async iterators
    [[nodiscard]] Value& build_create_async_from_sync_iterator(Value& iterator, Value& next_method, Value& done);

    // Iterators
    [[nodiscard]] Value& build_get_iterator(Value& iterable);
    [[nodiscard]] Value& build_get_object_property_iterator(Value& object);
    [[nodiscard]] Value& build_iterator_next(Value& iterator);
    [[nodiscard]] Value& build_iterator_next_unpack(Value& iterator);
    void build_iterator_close(Value& iterator);
    void build_async_iterator_close(Value& iterator);
    [[nodiscard]] Value& build_iterator_to_array(Value& iterator);

    // Special
    [[nodiscard]] Value& build_in(Value& lhs, Value& rhs);
    [[nodiscard]] Value& build_instance_of(Value& lhs, Value& rhs);

    // Copy
    [[nodiscard]] Value& build_move(Value& source);

    // Tuple extraction
    [[nodiscard]] Value& build_extract_value(Value& tuple, u32 index);

    // Control flow (void, no result)
    void build_jump(BasicBlock& to);
    void build_continue_pending_unwind(BasicBlock& resume_target);
    void build_branch(Value& condition, BasicBlock& if_true, BasicBlock& if_false);
    void build_return(Value& value);
    void build_end(Value& value);
    void build_throw(Value& value);

    // Generators/Async - terminators with result (the resume value)
    [[nodiscard]] Value& build_yield(Value& value, BasicBlock* continuation);
    [[nodiscard]] Value& build_await(Value& argument, BasicBlock& continuation);
    [[nodiscard]] Value& build_get_completion_fields(Value& completion);
    void build_set_completion_type(Value& completion, Completion::Type type);
    [[nodiscard]] Value& build_new_type_error(Bytecode::StringTableIndex error_string);

    // SSA
    [[nodiscard]] Value& build_phi(Vector<Value*> values, Vector<BlockIndex> predecessors);

private:
    BasicBlock& current_block();

    Value& build_binary_op(Opcode opcode, Value& lhs, Value& rhs);
    Value& build_unary_op(Opcode opcode, Value& operand);

    // Common helper: creates a result value, attaches it to the instruction,
    // recomputes the result type from opcode metadata, appends, and returns.
    Value& emit_with_result(NonnullOwnPtr<Instruction> instruction);

    Function& m_function;
    BasicBlock* m_insertion_block { nullptr };
};

}
