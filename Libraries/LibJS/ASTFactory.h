/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <stddef.h>
#include <stdint.h>

// Opaque handle types for FFI
typedef void* ASTNodeHandle;
typedef void* SourceCodeHandle;

#ifdef __cplusplus

#include <AK/NonnullRefPtr.h>

namespace JS {
class Program;
class SourceCode;

// High-level entry point: parse a script/module using the Rust parser.
NonnullRefPtr<Program> rust_parse(NonnullRefPtr<SourceCode const> source_code, Program::Type program_type, bool starts_in_strict_mode = false);
}

extern "C" {
#endif

// Arena for holding ref-counted AST nodes alive across FFI boundary.
typedef void* ASTArenaHandle;
ASTArenaHandle ast_arena_create();
void ast_arena_destroy(ASTArenaHandle arena);
void ast_node_ref(ASTNodeHandle handle);

// SourceRange construction helper.
// All factory functions take source range info inline.
// source_code is a SourceCodeHandle (SourceCode const*).

// === Program / ScopeNode ===
ASTNodeHandle ast_create_program(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t program_type); // 0 = Script, 1 = Module

ASTNodeHandle ast_create_block_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset);

ASTNodeHandle ast_create_function_body(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset);

// Append a statement to a scope node (Program, BlockStatement, FunctionBody)
void ast_scope_node_append(ASTNodeHandle scope_node, ASTNodeHandle statement);
void ast_scope_node_set_strict_mode(ASTNodeHandle scope_node);

// === Literals ===
ASTNodeHandle ast_create_numeric_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    double value);

ASTNodeHandle ast_create_string_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* value, size_t value_len);

ASTNodeHandle ast_create_boolean_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    bool value);

ASTNodeHandle ast_create_null_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset);

ASTNodeHandle ast_create_bigint_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    char const* value, size_t value_len);

// === Identifiers ===
ASTNodeHandle ast_create_identifier(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* name, size_t name_len);

ASTNodeHandle ast_create_private_identifier(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* name, size_t name_len);

// === Expressions ===
ASTNodeHandle ast_create_this_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset);

ASTNodeHandle ast_create_super_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset);

ASTNodeHandle ast_create_binary_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t op, ASTNodeHandle lhs, ASTNodeHandle rhs);

ASTNodeHandle ast_create_logical_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t op, ASTNodeHandle lhs, ASTNodeHandle rhs);

ASTNodeHandle ast_create_unary_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t op, ASTNodeHandle operand);

ASTNodeHandle ast_create_update_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t op, ASTNodeHandle argument, bool prefixed);

ASTNodeHandle ast_create_assignment_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t op, ASTNodeHandle lhs, ASTNodeHandle rhs);

ASTNodeHandle ast_create_conditional_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle test, ASTNodeHandle consequent, ASTNodeHandle alternate);

ASTNodeHandle ast_create_sequence_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle const* expressions, size_t count);

ASTNodeHandle ast_create_member_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle object, ASTNodeHandle property, bool computed);

ASTNodeHandle ast_create_call_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle callee,
    ASTNodeHandle const* argument_values, bool const* argument_is_spread, size_t argument_count);

ASTNodeHandle ast_create_new_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle callee,
    ASTNodeHandle const* argument_values, bool const* argument_is_spread, size_t argument_count);

ASTNodeHandle ast_create_spread_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle target);

ASTNodeHandle ast_create_yield_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle argument, bool is_yield_from);

ASTNodeHandle ast_create_await_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle argument);

ASTNodeHandle ast_create_import_call(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle specifier, ASTNodeHandle options);

ASTNodeHandle ast_create_meta_property(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t type); // 0 = NewTarget, 1 = ImportMeta

// === Statements ===
ASTNodeHandle ast_create_expression_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle expression);

ASTNodeHandle ast_create_empty_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset);

ASTNodeHandle ast_create_return_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle argument); // may be null

ASTNodeHandle ast_create_throw_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle argument);

ASTNodeHandle ast_create_break_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* label, size_t label_len); // label may be null

ASTNodeHandle ast_create_continue_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* label, size_t label_len); // label may be null

ASTNodeHandle ast_create_debugger_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset);

ASTNodeHandle ast_create_if_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle predicate, ASTNodeHandle consequent, ASTNodeHandle alternate); // alternate may be null

ASTNodeHandle ast_create_while_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle test, ASTNodeHandle body);

ASTNodeHandle ast_create_do_while_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle test, ASTNodeHandle body);

ASTNodeHandle ast_create_for_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle init, ASTNodeHandle test, ASTNodeHandle update, ASTNodeHandle body);

ASTNodeHandle ast_create_for_in_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle lhs, ASTNodeHandle rhs, ASTNodeHandle body);

ASTNodeHandle ast_create_for_of_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle lhs, ASTNodeHandle rhs, ASTNodeHandle body);

ASTNodeHandle ast_create_for_await_of_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle lhs, ASTNodeHandle rhs, ASTNodeHandle body);

ASTNodeHandle ast_create_with_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle object, ASTNodeHandle body);

ASTNodeHandle ast_create_labelled_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* label, size_t label_len, ASTNodeHandle body);

ASTNodeHandle ast_create_switch_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle discriminant);

ASTNodeHandle ast_create_switch_case(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle test); // may be null for default case

void ast_switch_statement_add_case(ASTNodeHandle switch_stmt, ASTNodeHandle switch_case);

ASTNodeHandle ast_create_try_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle block, ASTNodeHandle handler, ASTNodeHandle finalizer);

ASTNodeHandle ast_create_catch_clause(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle parameter, ASTNodeHandle body); // parameter may be null

// === Declarations ===
ASTNodeHandle ast_create_variable_declaration(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t kind, // 0=Var, 1=Let, 2=Const
    ASTNodeHandle const* declarators, size_t declarator_count);

ASTNodeHandle ast_create_variable_declarator(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle target, ASTNodeHandle init); // init may be null

// === Object/Array expressions ===
ASTNodeHandle ast_create_object_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle const* properties, size_t property_count);

ASTNodeHandle ast_create_object_property(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle key, ASTNodeHandle value, uint8_t type, bool is_method);

ASTNodeHandle ast_create_array_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle const* elements, size_t element_count); // elements may contain null entries

// === Template literals ===
ASTNodeHandle ast_create_template_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle const* expressions, size_t expression_count);

ASTNodeHandle ast_create_tagged_template_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle tag, ASTNodeHandle template_literal);

// === Functions ===
// Creates FunctionParameters from a list of parameter descriptions
ASTNodeHandle ast_create_function_parameters_empty();
ASTNodeHandle ast_create_function_parameters(ASTArenaHandle arena,
    ASTNodeHandle const* bindings, ASTNodeHandle const* default_values,
    bool const* is_rest, size_t count);

ASTNodeHandle ast_create_function_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle name, // Identifier, may be null
    uint32_t source_text_start, uint32_t source_text_len,
    ASTNodeHandle body, ASTNodeHandle parameters,
    int32_t function_length, uint8_t kind,
    bool is_strict_mode, bool is_arrow_function,
    bool uses_this, bool uses_this_from_environment,
    bool contains_direct_call_to_eval, bool might_need_arguments_object);

ASTNodeHandle ast_create_function_declaration(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle name, // Identifier, may be null
    uint32_t source_text_start, uint32_t source_text_len,
    ASTNodeHandle body, ASTNodeHandle parameters,
    int32_t function_length, uint8_t kind,
    bool is_strict_mode,
    bool uses_this, bool uses_this_from_environment,
    bool contains_direct_call_to_eval, bool might_need_arguments_object);

// === Classes ===
ASTNodeHandle ast_create_class_expression(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle name, // Identifier, may be null
    uint32_t source_text_start, uint32_t source_text_len,
    ASTNodeHandle constructor, // FunctionExpression, may be null
    ASTNodeHandle super_class, // Expression, may be null
    ASTNodeHandle const* elements, size_t element_count);

ASTNodeHandle ast_create_class_declaration(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle class_expression);

ASTNodeHandle ast_create_class_method(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle key, ASTNodeHandle function, uint8_t kind, bool is_static);

ASTNodeHandle ast_create_class_field(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle key, ASTNodeHandle init, bool is_static);

ASTNodeHandle ast_create_static_initializer(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle function_body);

// === Scope declarations ===
void ast_scope_node_add_var_scoped_declaration(ASTNodeHandle scope_node, ASTNodeHandle declaration);
void ast_scope_node_add_lexical_declaration(ASTNodeHandle scope_node, ASTNodeHandle declaration);
void ast_scope_node_add_hoisted_function(ASTNodeHandle scope_node, ASTNodeHandle function_declaration);
void ast_scope_node_shrink_to_fit(ASTNodeHandle scope_node);

// === SwitchCase ===
void ast_switch_case_append(ASTNodeHandle switch_case, ASTNodeHandle statement);

#ifdef __cplusplus
}
#endif
