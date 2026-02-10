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
// Sets out_has_errors to true if the Rust parser detected syntax errors.
NonnullRefPtr<Program> rust_parse(
    NonnullRefPtr<SourceCode const> source_code,
    Program::Type program_type,
    bool starts_in_strict_mode,
    bool initiated_by_eval,
    bool in_eval_function_context,
    bool allow_super_property_lookup,
    bool allow_super_constructor_call,
    bool in_class_field_initializer,
    bool& out_has_errors);
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

ASTNodeHandle ast_create_regexp_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* pattern, size_t pattern_len,
    uint16_t const* flags, size_t flags_len);

// === Identifiers ===
ASTNodeHandle ast_create_identifier(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* name, size_t name_len);

ASTNodeHandle ast_create_private_identifier(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* name, size_t name_len);

// Scope analysis: mark identifiers as local variables or arguments
void ast_identifier_set_local_variable_index(ASTNodeHandle identifier, uint32_t index);
void ast_identifier_set_argument_index(ASTNodeHandle identifier, uint32_t index);
void ast_identifier_set_is_global(ASTNodeHandle identifier);
void ast_identifier_set_is_inside_scope_with_eval(ASTNodeHandle identifier);
void ast_identifier_set_declaration_kind(ASTNodeHandle identifier, uint8_t kind);
bool ast_identifier_is_local(ASTNodeHandle identifier);
bool ast_identifier_is_inside_scope_with_eval(ASTNodeHandle identifier);

// Scope analysis: add local variables to scope nodes
// declaration_kind: 0=Var, 1=LetOrConst, 2=Function, 3=ArgumentsObject, 4=CatchClauseParameter
uint32_t ast_scope_node_add_local_variable(ASTNodeHandle scope, uint16_t const* name, size_t name_len, uint8_t declaration_kind);

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

ASTNodeHandle ast_create_super_call(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle const* argument_values, bool const* argument_is_spread, size_t argument_count);

ASTNodeHandle ast_create_synthetic_constructor_super_call(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle argument_identifier);

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

ASTNodeHandle ast_create_using_declaration(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle const* declarators, size_t declarator_count);

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

ASTNodeHandle ast_create_template_literal_with_raw_strings(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle const* expressions, size_t expression_count,
    ASTNodeHandle const* raw_strings, size_t raw_string_count);

ASTNodeHandle ast_create_tagged_template_literal(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle tag, ASTNodeHandle template_literal);

// === Functions ===
// Creates FunctionParameters from a list of parameter descriptions
ASTNodeHandle ast_create_function_parameters_empty();
ASTNodeHandle ast_create_function_parameters(ASTArenaHandle arena,
    ASTNodeHandle const* bindings, ASTNodeHandle const* default_values,
    bool const* is_rest, bool const* is_pattern, size_t count);

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

// Build FunctionScopeData for a function scope, using scope variable information.
// var_names_data: concatenated UTF-16 variable names
// var_name_offsets: start offset of each name in var_names_data
// var_name_lengths: length of each name
// var_identifiers: Identifier handle for each var
// var_is_parameter: whether each var shadows a parameter
// var_count: number of variables
// has_argument_parameter: whether "arguments" is a parameter name
void ast_scope_build_function_scope_data(
    ASTNodeHandle scope_node,
    uint16_t const* var_names_data,
    uint32_t const* var_name_offsets,
    uint32_t const* var_name_lengths,
    ASTNodeHandle const* var_identifiers,
    uint8_t const* var_is_parameter,
    size_t var_count,
    uint8_t has_argument_parameter);

// === SwitchCase ===
void ast_switch_case_append(ASTNodeHandle switch_case, ASTNodeHandle statement);

// === BindingPattern ===
// BindingPattern is NOT an ASTNode -- its handle is a BindingPattern* cast to void*.
// kind: 0 = Array, 1 = Object
ASTNodeHandle ast_create_binding_pattern(ASTArenaHandle arena, uint8_t kind);

// name_type: 0=Empty, 1=Identifier, 2=Expression
// alias_type: 0=Empty, 1=Identifier, 2=BindingPattern, 3=MemberExpression
void ast_binding_pattern_append_entry(
    ASTNodeHandle pattern,
    ASTNodeHandle name, uint8_t name_type,
    ASTNodeHandle alias, uint8_t alias_type,
    ASTNodeHandle initializer, bool is_rest);

ASTNodeHandle ast_create_variable_declarator_with_pattern(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle pattern, ASTNodeHandle init);

ASTNodeHandle ast_create_catch_clause_with_pattern(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle pattern, ASTNodeHandle body);

ASTNodeHandle ast_create_for_in_statement_with_pattern(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle pattern, ASTNodeHandle rhs, ASTNodeHandle body);

ASTNodeHandle ast_create_for_of_statement_with_pattern(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle pattern, ASTNodeHandle rhs, ASTNodeHandle body);

ASTNodeHandle ast_create_for_await_of_statement_with_pattern(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle pattern, ASTNodeHandle rhs, ASTNodeHandle body);

ASTNodeHandle ast_create_assignment_expression_with_pattern(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint8_t op, ASTNodeHandle pattern, ASTNodeHandle rhs);

// === OptionalChain ===
// Builder pattern: create builder, append references, then finalize into OptionalChain.
typedef void* OptionalChainBuilderHandle;
OptionalChainBuilderHandle ast_create_optional_chain_builder();
void ast_optional_chain_builder_append_member(OptionalChainBuilderHandle builder,
    ASTNodeHandle identifier, bool is_optional);
void ast_optional_chain_builder_append_computed(OptionalChainBuilderHandle builder,
    ASTNodeHandle expression, bool is_optional);
void ast_optional_chain_builder_append_call(OptionalChainBuilderHandle builder,
    ASTNodeHandle const* argument_values, bool const* argument_is_spread,
    size_t argument_count, bool is_optional);
void ast_optional_chain_builder_append_private_member(OptionalChainBuilderHandle builder,
    ASTNodeHandle private_identifier, bool is_optional);
ASTNodeHandle ast_create_optional_chain(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle base, OptionalChainBuilderHandle builder);

// Node type checking (returns true if the handle is the given type)
bool ast_is_identifier(ASTNodeHandle handle);
bool ast_is_member_expression(ASTNodeHandle handle);
bool ast_is_object_expression(ASTNodeHandle handle);
bool ast_is_array_expression(ASTNodeHandle handle);
bool ast_is_call_expression(ASTNodeHandle handle);

// === Import/Export ===

// FFI struct for ImportEntry.
// If import_name_len == SIZE_MAX, this is a namespace import (no import_name).
struct FFIImportEntry {
    uint16_t const* import_name;
    size_t import_name_len;
    uint16_t const* local_name;
    size_t local_name_len;
};

// FFI struct for ExportEntry.
// kind: 0 = NamedExport, 1 = ModuleRequestAll, 2 = ModuleRequestAllButDefault, 3 = EmptyNamedExport
// If export_name_len == SIZE_MAX, export_name is empty (Optional<> has no value).
// If local_or_import_name_len == SIZE_MAX, local_or_import_name is empty.
struct FFIExportEntry {
    uint8_t kind;
    uint16_t const* export_name;
    size_t export_name_len;
    uint16_t const* local_or_import_name;
    size_t local_or_import_name_len;
};

ASTNodeHandle ast_create_import_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    uint16_t const* module_specifier, size_t module_specifier_len,
    struct FFIImportEntry const* entries, size_t entry_count);

ASTNodeHandle ast_create_export_statement(ASTArenaHandle arena, SourceCodeHandle source_code,
    uint32_t start_line, uint32_t start_column, uint32_t start_offset,
    uint32_t end_line, uint32_t end_column, uint32_t end_offset,
    ASTNodeHandle statement_or_null,
    struct FFIExportEntry const* entries, size_t entry_count,
    bool is_default,
    uint16_t const* from_specifier, size_t from_specifier_len);

// Add an import attribute (key, value) to an ImportStatement's module request.
void ast_import_statement_add_attribute(ASTNodeHandle import_stmt,
    uint16_t const* key, size_t key_len,
    uint16_t const* value, size_t value_len);

// Add an import attribute (key, value) to an ExportStatement's module request.
void ast_export_statement_add_attribute(ASTNodeHandle export_stmt,
    uint16_t const* key, size_t key_len,
    uint16_t const* value, size_t value_len);

// Append import/export to program (registers in both statement list and import/export list).
void ast_program_append_import(ASTNodeHandle program, ASTNodeHandle import_stmt);
void ast_program_append_export(ASTNodeHandle program, ASTNodeHandle export_stmt);

// Mark a program as having top-level await (for modules).
void ast_program_set_has_top_level_await(ASTNodeHandle program);

// Extract bound names from a declaration node for export.
// For FunctionDeclaration/ClassDeclaration: returns 1 name.
// For VariableDeclaration: returns all bound names (including from patterns).
// Writes names into out_names (each as utf16 ptr+len pair), returns count.
// out_names must be at least max_names entries. Each entry is a pair of (ptr, len)
// pointing into the AST node's storage (valid as long as the node is alive).
typedef struct {
    uint16_t const* data;
    size_t len;
} FFIUtf16Slice;
size_t ast_get_declaration_export_names(ASTNodeHandle declaration,
    FFIUtf16Slice* out_names, size_t max_names);

// Get function name from a FunctionDeclaration node.
// Returns empty slice if anonymous.
FFIUtf16Slice ast_get_function_name(ASTNodeHandle function_decl);

// Get class name from a ClassDeclaration node.
FFIUtf16Slice ast_get_class_name(ASTNodeHandle class_decl);

// Check if a FunctionDeclaration has a name.
bool ast_function_has_name(ASTNodeHandle function_decl);

#ifdef __cplusplus
}
#endif
