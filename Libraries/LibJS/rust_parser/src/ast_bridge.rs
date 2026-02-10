/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Safe wrappers around the C++ AST factory functions.
//!
//! This module is the FFI bridge between the Rust parser and the C++
//! AST node hierarchy. Every function here calls an `extern "C"` factory
//! defined in `ASTFactory.cpp`.
//!
//! ## NodeHandle
//!
//! AST nodes are represented as opaque `NodeHandle` pointers (`*mut c_void`).
//! On the C++ side, these are `ASTNode*` pointers. The nodes are ref-counted
//! C++ objects, and an arena (also on the C++ side) holds refs to keep them
//! alive during parsing. When parsing completes, the final Program node gets
//! an extra ref before the arena is destroyed.
//!
//! ## AstBuilder
//!
//! The `AstBuilder` struct holds the arena handle and source code handle,
//! and provides safe Rust methods that forward to the C++ factory functions.
//! It handles marshalling Rust types (slices, bools, etc.) into C-compatible
//! arguments.
//!
//! ## Span
//!
//! Every AST node needs source location info. The parser tracks positions
//! as `(line, column, offset)` tuples. `AstBuilder` methods accept a `Span`
//! struct with start and end positions, which gets unpacked into the
//! individual u32 arguments that the C functions expect.

use std::ptr;

/// Opaque handle to a C++ AST node.
pub type NodeHandle = *mut std::ffi::c_void;
/// Opaque handle to a C++ SourceCode object.
pub type SourceCodeHandle = *const std::ffi::c_void;
/// Opaque handle to an ASTArena.
pub type ArenaHandle = *mut std::ffi::c_void;

pub const NULL_HANDLE: NodeHandle = ptr::null_mut();

/// Source position info passed to every factory call.
#[derive(Clone, Copy, Debug)]
pub struct Span {
    pub start_line: u32,
    pub start_column: u32,
    pub start_offset: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub end_offset: u32,
}

/// FFI struct for ImportEntry, matching the C++ FFIImportEntry.
/// If import_name_len == usize::MAX, this is a namespace import.
#[repr(C)]
pub struct FFIImportEntry {
    pub import_name: *const u16,
    pub import_name_len: usize,
    pub local_name: *const u16,
    pub local_name_len: usize,
}

/// FFI struct for ExportEntry, matching the C++ FFIExportEntry.
/// kind: 0=NamedExport, 1=ModuleRequestAll, 2=ModuleRequestAllButDefault, 3=EmptyNamedExport
/// If export_name_len == usize::MAX, export_name is None.
/// If local_or_import_name_len == usize::MAX, local_or_import_name is None.
#[repr(C)]
pub struct FFIExportEntry {
    pub kind: u8,
    pub export_name: *const u16,
    pub export_name_len: usize,
    pub local_or_import_name: *const u16,
    pub local_or_import_name_len: usize,
}

// Raw extern "C" declarations. Private to this module — all other code
// should use AstBuilder methods or the safe free functions below.
extern "C" {
    fn ast_arena_create() -> ArenaHandle;
    fn ast_arena_destroy(arena: ArenaHandle);
    fn ast_node_ref(handle: NodeHandle);

    fn ast_create_program(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        program_type: u8,
    ) -> NodeHandle;
    fn ast_create_block_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    fn ast_create_function_body(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    fn ast_scope_node_append(scope_node: NodeHandle, statement: NodeHandle);
    fn ast_scope_node_set_strict_mode(scope_node: NodeHandle);

    fn ast_create_numeric_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        value: f64,
    ) -> NodeHandle;
    fn ast_create_string_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        value: *const u16, value_len: usize,
    ) -> NodeHandle;
    fn ast_create_boolean_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        value: bool,
    ) -> NodeHandle;
    fn ast_create_null_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    fn ast_create_bigint_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        value: *const u8, value_len: usize,
    ) -> NodeHandle;
    fn ast_create_regexp_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: *const u16, pattern_len: usize,
        flags: *const u16, flags_len: usize,
    ) -> NodeHandle;

    fn ast_create_identifier(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        name: *const u16, name_len: usize,
    ) -> NodeHandle;
    fn ast_create_private_identifier(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        name: *const u16, name_len: usize,
    ) -> NodeHandle;

    fn ast_identifier_set_local_variable_index(identifier: NodeHandle, index: u32);
    fn ast_identifier_set_argument_index(identifier: NodeHandle, index: u32);
    fn ast_identifier_set_is_global(identifier: NodeHandle);
    fn ast_identifier_set_is_inside_scope_with_eval(identifier: NodeHandle);
    fn ast_identifier_set_declaration_kind(identifier: NodeHandle, kind: u8);
    fn ast_identifier_is_inside_scope_with_eval(identifier: NodeHandle) -> bool;
    fn ast_scope_node_add_local_variable(
        scope: NodeHandle, name: *const u16, name_len: usize, declaration_kind: u8,
    ) -> u32;

    fn ast_create_this_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    fn ast_create_super_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    fn ast_create_binary_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, lhs: NodeHandle, rhs: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_logical_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, lhs: NodeHandle, rhs: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_unary_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, operand: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_update_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, argument: NodeHandle, prefixed: bool,
    ) -> NodeHandle;
    fn ast_create_assignment_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, lhs: NodeHandle, rhs: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_assignment_expression_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, pattern: NodeHandle, rhs: NodeHandle,
    ) -> NodeHandle;
    fn ast_is_identifier(handle: NodeHandle) -> bool;
    fn ast_is_member_expression(handle: NodeHandle) -> bool;
    fn ast_is_object_expression(handle: NodeHandle) -> bool;
    fn ast_is_array_expression(handle: NodeHandle) -> bool;
    fn ast_is_call_expression(handle: NodeHandle) -> bool;
    fn ast_create_conditional_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        test: NodeHandle, consequent: NodeHandle, alternate: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_sequence_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        expressions: *const NodeHandle, count: usize,
    ) -> NodeHandle;
    fn ast_create_member_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        object: NodeHandle, property: NodeHandle, computed: bool,
    ) -> NodeHandle;
    fn ast_create_call_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        callee: NodeHandle,
        argument_values: *const NodeHandle, argument_is_spread: *const bool,
        argument_count: usize,
    ) -> NodeHandle;
    fn ast_create_super_call(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument_values: *const NodeHandle, argument_is_spread: *const bool,
        argument_count: usize,
    ) -> NodeHandle;
    fn ast_create_synthetic_constructor_super_call(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument_identifier: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_new_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        callee: NodeHandle,
        argument_values: *const NodeHandle, argument_is_spread: *const bool,
        argument_count: usize,
    ) -> NodeHandle;
    fn ast_create_spread_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        target: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_yield_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument: NodeHandle, is_yield_from: bool,
    ) -> NodeHandle;
    fn ast_create_await_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_import_call(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        specifier: NodeHandle, options: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_meta_property(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        meta_type: u8,
    ) -> NodeHandle;

    fn ast_create_expression_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        expression: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_empty_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    fn ast_create_return_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_throw_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_break_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        label: *const u16, label_len: usize,
    ) -> NodeHandle;
    fn ast_create_continue_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        label: *const u16, label_len: usize,
    ) -> NodeHandle;
    fn ast_create_debugger_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    fn ast_create_if_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        predicate: NodeHandle, consequent: NodeHandle, alternate: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_while_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        test: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_do_while_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        test: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_for_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        init: NodeHandle, test: NodeHandle, update: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_for_in_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_for_of_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_for_await_of_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_with_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        object: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_labelled_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        label: *const u16, label_len: usize, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_switch_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        discriminant: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_switch_case(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        test: NodeHandle,
    ) -> NodeHandle;
    fn ast_switch_statement_add_case(switch_stmt: NodeHandle, switch_case: NodeHandle);
    fn ast_create_try_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        block: NodeHandle, handler: NodeHandle, finalizer: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_catch_clause(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        parameter: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;

    fn ast_create_variable_declaration(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        kind: u8, declarators: *const NodeHandle, declarator_count: usize,
    ) -> NodeHandle;
    fn ast_create_using_declaration(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        declarators: *const NodeHandle, declarator_count: usize,
    ) -> NodeHandle;
    fn ast_create_variable_declarator(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        target: NodeHandle, init: NodeHandle,
    ) -> NodeHandle;

    fn ast_create_object_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        properties: *const NodeHandle, property_count: usize,
    ) -> NodeHandle;
    fn ast_create_object_property(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        key: NodeHandle, value: NodeHandle, prop_type: u8, is_method: bool,
    ) -> NodeHandle;
    fn ast_create_array_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        elements: *const NodeHandle, element_count: usize,
    ) -> NodeHandle;

    fn ast_create_template_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        expressions: *const NodeHandle, expression_count: usize,
    ) -> NodeHandle;
    fn ast_create_template_literal_with_raw_strings(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        expressions: *const NodeHandle, expression_count: usize,
        raw_strings: *const NodeHandle, raw_string_count: usize,
    ) -> NodeHandle;
    fn ast_create_tagged_template_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        tag: NodeHandle, template_literal: NodeHandle,
    ) -> NodeHandle;

    fn ast_create_function_parameters_empty() -> NodeHandle;
    fn ast_create_function_parameters(
        arena: ArenaHandle,
        bindings: *const NodeHandle, default_values: *const NodeHandle,
        is_rest: *const bool, is_pattern: *const bool, count: usize,
    ) -> NodeHandle;
    fn ast_create_function_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        name: NodeHandle,
        source_text_start: u32, source_text_len: u32,
        body: NodeHandle, parameters: NodeHandle,
        function_length: i32, kind: u8,
        is_strict_mode: bool, is_arrow_function: bool,
        uses_this: bool, uses_this_from_environment: bool,
        contains_direct_call_to_eval: bool, might_need_arguments_object: bool,
    ) -> NodeHandle;
    fn ast_create_function_declaration(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        name: NodeHandle,
        source_text_start: u32, source_text_len: u32,
        body: NodeHandle, parameters: NodeHandle,
        function_length: i32, kind: u8,
        is_strict_mode: bool,
        uses_this: bool, uses_this_from_environment: bool,
        contains_direct_call_to_eval: bool, might_need_arguments_object: bool,
    ) -> NodeHandle;

    fn ast_create_class_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        name: NodeHandle,
        source_text_start: u32, source_text_len: u32,
        constructor: NodeHandle, super_class: NodeHandle,
        elements: *const NodeHandle, element_count: usize,
    ) -> NodeHandle;
    fn ast_create_class_declaration(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        class_expression: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_class_method(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        key: NodeHandle, function: NodeHandle, kind: u8, is_static: bool,
    ) -> NodeHandle;
    fn ast_create_class_field(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        key: NodeHandle, init: NodeHandle, is_static: bool,
    ) -> NodeHandle;
    fn ast_create_static_initializer(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        function_body: NodeHandle,
    ) -> NodeHandle;

    fn ast_scope_node_add_var_scoped_declaration(scope_node: NodeHandle, declaration: NodeHandle);
    fn ast_scope_node_add_lexical_declaration(scope_node: NodeHandle, declaration: NodeHandle);
    fn ast_scope_node_add_hoisted_function(scope_node: NodeHandle, function_declaration: NodeHandle);
    fn ast_scope_node_shrink_to_fit(scope_node: NodeHandle);

    fn ast_scope_build_function_scope_data(
        scope_node: NodeHandle,
        var_names_data: *const u16,
        var_name_offsets: *const u32,
        var_name_lengths: *const u32,
        var_identifiers: *const NodeHandle,
        var_is_parameter: *const u8,
        var_count: usize,
        has_argument_parameter: u8,
    );

    fn ast_switch_case_append(switch_case: NodeHandle, statement: NodeHandle);

    fn ast_create_binding_pattern(arena: ArenaHandle, kind: u8) -> NodeHandle;
    fn ast_binding_pattern_append_entry(
        pattern: NodeHandle,
        name: NodeHandle, name_type: u8,
        alias: NodeHandle, alias_type: u8,
        initializer: NodeHandle, is_rest: bool,
    );
    fn ast_create_variable_declarator_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, init: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_catch_clause_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_for_in_statement_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_for_of_statement_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    fn ast_create_for_await_of_statement_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;

    fn ast_create_optional_chain_builder() -> *mut std::ffi::c_void;
    fn ast_optional_chain_builder_append_member(
        builder: *mut std::ffi::c_void,
        identifier: NodeHandle, is_optional: bool,
    );
    fn ast_optional_chain_builder_append_computed(
        builder: *mut std::ffi::c_void,
        expression: NodeHandle, is_optional: bool,
    );
    fn ast_optional_chain_builder_append_call(
        builder: *mut std::ffi::c_void,
        argument_values: *const NodeHandle, argument_is_spread: *const bool,
        argument_count: usize, is_optional: bool,
    );
    fn ast_optional_chain_builder_append_private_member(
        builder: *mut std::ffi::c_void,
        private_identifier: NodeHandle, is_optional: bool,
    );
    fn ast_create_optional_chain(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        base: NodeHandle, builder: *mut std::ffi::c_void,
    ) -> NodeHandle;

    fn ast_create_import_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        module_specifier: *const u16, module_specifier_len: usize,
        entries: *const FFIImportEntry, entry_count: usize,
    ) -> NodeHandle;

    fn ast_create_export_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        statement_or_null: NodeHandle,
        entries: *const FFIExportEntry, entry_count: usize,
        is_default: bool,
        from_specifier: *const u16, from_specifier_len: usize,
    ) -> NodeHandle;

    fn ast_import_statement_add_attribute(
        import_stmt: NodeHandle,
        key: *const u16, key_len: usize,
        value: *const u16, value_len: usize,
    );

    fn ast_export_statement_add_attribute(
        export_stmt: NodeHandle,
        key: *const u16, key_len: usize,
        value: *const u16, value_len: usize,
    );

    fn ast_program_append_import(program: NodeHandle, import_stmt: NodeHandle);
    fn ast_program_append_export(program: NodeHandle, export_stmt: NodeHandle);

    fn ast_get_declaration_export_names(
        declaration: NodeHandle,
        out_names: *mut FFIUtf16Slice, max_names: usize,
    ) -> usize;

    fn ast_get_function_name(function_decl: NodeHandle) -> FFIUtf16Slice;
    fn ast_get_class_name(class_decl: NodeHandle) -> FFIUtf16Slice;
    fn ast_function_has_name(function_decl: NodeHandle) -> bool;
}

/// FFI struct matching C++ FFIUtf16Slice.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FFIUtf16Slice {
    pub data: *const u16,
    pub len: usize,
}

// === Safe free functions for scope analysis ===
//
// These wrap the raw FFI calls so that scope_collector.rs never needs `unsafe`.

pub fn identifier_set_local_variable_index(identifier: NodeHandle, index: u32) {
    unsafe { ast_identifier_set_local_variable_index(identifier, index) }
}

pub fn identifier_set_argument_index(identifier: NodeHandle, index: u32) {
    unsafe { ast_identifier_set_argument_index(identifier, index) }
}

pub fn identifier_set_is_global(identifier: NodeHandle) {
    unsafe { ast_identifier_set_is_global(identifier) }
}

pub fn identifier_set_is_inside_scope_with_eval(identifier: NodeHandle) {
    unsafe { ast_identifier_set_is_inside_scope_with_eval(identifier) }
}

pub fn identifier_set_declaration_kind(identifier: NodeHandle, kind: u8) {
    unsafe { ast_identifier_set_declaration_kind(identifier, kind) }
}

pub fn identifier_is_inside_scope_with_eval(identifier: NodeHandle) -> bool {
    unsafe { ast_identifier_is_inside_scope_with_eval(identifier) }
}

pub fn scope_node_add_local_variable(scope: NodeHandle, name: &[u16], declaration_kind: u8) -> u32 {
    unsafe { ast_scope_node_add_local_variable(scope, name.as_ptr(), name.len(), declaration_kind) }
}

pub fn scope_node_add_var_scoped_declaration(scope_node: NodeHandle, declaration: NodeHandle) {
    unsafe { ast_scope_node_add_var_scoped_declaration(scope_node, declaration) }
}

pub fn scope_node_add_lexical_declaration(scope_node: NodeHandle, declaration: NodeHandle) {
    unsafe { ast_scope_node_add_lexical_declaration(scope_node, declaration) }
}

pub fn scope_node_add_hoisted_function(scope_node: NodeHandle, function_declaration: NodeHandle) {
    unsafe { ast_scope_node_add_hoisted_function(scope_node, function_declaration) }
}

pub fn scope_build_function_scope_data(
    scope_node: NodeHandle,
    names_data: &[u16],
    name_offsets: &[u32],
    name_lengths: &[u32],
    identifiers: &[NodeHandle],
    is_parameter: &[u8],
    has_argument_parameter: u8,
) {
    unsafe {
        ast_scope_build_function_scope_data(
            scope_node,
            names_data.as_ptr(), name_offsets.as_ptr(), name_lengths.as_ptr(),
            identifiers.as_ptr(), is_parameter.as_ptr(), identifiers.len(),
            has_argument_parameter,
        )
    }
}

/// Builder wrapping the C++ AST factory with a simpler interface.
pub struct AstBuilder {
    arena: ArenaHandle,
    source_code: SourceCodeHandle,
}

impl AstBuilder {
    pub fn new(source_code: SourceCodeHandle) -> Self {
        let arena = unsafe { ast_arena_create() };
        Self { arena, source_code }
    }

    pub fn arena(&self) -> ArenaHandle {
        self.arena
    }

    /// Add an extra ref to a node so it survives arena destruction.
    /// The caller takes ownership of the extra ref.
    pub fn ref_node(&self, handle: NodeHandle) {
        unsafe { ast_node_ref(handle) }
    }

    // === Helpers ===

    fn s(&self, span: Span) -> (ArenaHandle, SourceCodeHandle, u32, u32, u32, u32, u32, u32) {
        (self.arena, self.source_code,
         span.start_line, span.start_column, span.start_offset,
         span.end_line, span.end_column, span.end_offset)
    }

    // === Program / ScopeNode ===

    pub fn create_program(&self, span: Span, program_type: u8) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_program(a, sc, sl, scol, so, el, ecol, eo, program_type) }
    }

    pub fn create_block_statement(&self, span: Span) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_block_statement(a, sc, sl, scol, so, el, ecol, eo) }
    }

    pub fn create_function_body(&self, span: Span) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_function_body(a, sc, sl, scol, so, el, ecol, eo) }
    }

    pub fn scope_node_append(&self, scope_node: NodeHandle, statement: NodeHandle) {
        unsafe { ast_scope_node_append(scope_node, statement) }
    }

    pub fn scope_node_set_strict_mode(&self, scope_node: NodeHandle) {
        unsafe { ast_scope_node_set_strict_mode(scope_node) }
    }

    // === Literals ===

    pub fn create_numeric_literal(&self, span: Span, value: f64) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_numeric_literal(a, sc, sl, scol, so, el, ecol, eo, value) }
    }

    pub fn create_string_literal(&self, span: Span, value: &[u16]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_string_literal(a, sc, sl, scol, so, el, ecol, eo, value.as_ptr(), value.len()) }
    }

    pub fn create_boolean_literal(&self, span: Span, value: bool) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_boolean_literal(a, sc, sl, scol, so, el, ecol, eo, value) }
    }

    pub fn create_null_literal(&self, span: Span) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_null_literal(a, sc, sl, scol, so, el, ecol, eo) }
    }

    pub fn create_bigint_literal(&self, span: Span, value: &[u8]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_bigint_literal(a, sc, sl, scol, so, el, ecol, eo, value.as_ptr(), value.len()) }
    }

    pub fn create_regexp_literal(&self, span: Span, pattern: &[u16], flags: &[u16]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_regexp_literal(a, sc, sl, scol, so, el, ecol, eo, pattern.as_ptr(), pattern.len(), flags.as_ptr(), flags.len()) }
    }

    // === Identifiers ===

    pub fn create_identifier(&self, span: Span, name: &[u16]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_identifier(a, sc, sl, scol, so, el, ecol, eo, name.as_ptr(), name.len()) }
    }

    pub fn create_private_identifier(&self, span: Span, name: &[u16]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_private_identifier(a, sc, sl, scol, so, el, ecol, eo, name.as_ptr(), name.len()) }
    }

    // === Expressions ===

    pub fn create_this_expression(&self, span: Span) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_this_expression(a, sc, sl, scol, so, el, ecol, eo) }
    }

    pub fn create_super_expression(&self, span: Span) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_super_expression(a, sc, sl, scol, so, el, ecol, eo) }
    }

    pub fn create_binary_expression(&self, span: Span, op: u8, lhs: NodeHandle, rhs: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_binary_expression(a, sc, sl, scol, so, el, ecol, eo, op, lhs, rhs) }
    }

    pub fn create_logical_expression(&self, span: Span, op: u8, lhs: NodeHandle, rhs: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_logical_expression(a, sc, sl, scol, so, el, ecol, eo, op, lhs, rhs) }
    }

    pub fn create_unary_expression(&self, span: Span, op: u8, operand: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_unary_expression(a, sc, sl, scol, so, el, ecol, eo, op, operand) }
    }

    pub fn create_update_expression(&self, span: Span, op: u8, argument: NodeHandle, prefixed: bool) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_update_expression(a, sc, sl, scol, so, el, ecol, eo, op, argument, prefixed) }
    }

    pub fn create_assignment_expression(&self, span: Span, op: u8, lhs: NodeHandle, rhs: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_assignment_expression(a, sc, sl, scol, so, el, ecol, eo, op, lhs, rhs) }
    }

    pub fn create_assignment_expression_with_pattern(&self, span: Span, op: u8, pattern: NodeHandle, rhs: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_assignment_expression_with_pattern(a, sc, sl, scol, so, el, ecol, eo, op, pattern, rhs) }
    }

    pub fn is_identifier(&self, handle: NodeHandle) -> bool {
        unsafe { ast_is_identifier(handle) }
    }

    pub fn is_member_expression(&self, handle: NodeHandle) -> bool {
        unsafe { ast_is_member_expression(handle) }
    }

    pub fn is_object_expression(&self, handle: NodeHandle) -> bool {
        unsafe { ast_is_object_expression(handle) }
    }

    pub fn is_array_expression(&self, handle: NodeHandle) -> bool {
        unsafe { ast_is_array_expression(handle) }
    }

    pub fn is_call_expression(&self, handle: NodeHandle) -> bool {
        unsafe { ast_is_call_expression(handle) }
    }

    pub fn create_conditional_expression(&self, span: Span, test: NodeHandle, consequent: NodeHandle, alternate: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_conditional_expression(a, sc, sl, scol, so, el, ecol, eo, test, consequent, alternate) }
    }

    pub fn create_sequence_expression(&self, span: Span, expressions: &[NodeHandle]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_sequence_expression(a, sc, sl, scol, so, el, ecol, eo, expressions.as_ptr(), expressions.len()) }
    }

    pub fn create_member_expression(&self, span: Span, object: NodeHandle, property: NodeHandle, computed: bool) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_member_expression(a, sc, sl, scol, so, el, ecol, eo, object, property, computed) }
    }

    pub fn create_call_expression(&self, span: Span, callee: NodeHandle, argument_values: &[NodeHandle], argument_is_spread: &[bool]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_call_expression(a, sc, sl, scol, so, el, ecol, eo, callee, argument_values.as_ptr(), argument_is_spread.as_ptr(), argument_values.len()) }
    }

    pub fn create_super_call(&self, span: Span, argument_values: &[NodeHandle], argument_is_spread: &[bool]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_super_call(a, sc, sl, scol, so, el, ecol, eo, argument_values.as_ptr(), argument_is_spread.as_ptr(), argument_values.len()) }
    }

    pub fn create_synthetic_constructor_super_call(&self, span: Span, argument_identifier: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_synthetic_constructor_super_call(a, sc, sl, scol, so, el, ecol, eo, argument_identifier) }
    }

    pub fn create_new_expression(&self, span: Span, callee: NodeHandle, argument_values: &[NodeHandle], argument_is_spread: &[bool]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_new_expression(a, sc, sl, scol, so, el, ecol, eo, callee, argument_values.as_ptr(), argument_is_spread.as_ptr(), argument_values.len()) }
    }

    pub fn create_spread_expression(&self, span: Span, target: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_spread_expression(a, sc, sl, scol, so, el, ecol, eo, target) }
    }

    pub fn create_yield_expression(&self, span: Span, argument: NodeHandle, is_yield_from: bool) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_yield_expression(a, sc, sl, scol, so, el, ecol, eo, argument, is_yield_from) }
    }

    pub fn create_await_expression(&self, span: Span, argument: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_await_expression(a, sc, sl, scol, so, el, ecol, eo, argument) }
    }

    pub fn create_import_call(&self, span: Span, specifier: NodeHandle, options: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_import_call(a, sc, sl, scol, so, el, ecol, eo, specifier, options) }
    }

    pub fn create_meta_property(&self, span: Span, meta_type: u8) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_meta_property(a, sc, sl, scol, so, el, ecol, eo, meta_type) }
    }

    // === Statements ===

    pub fn create_expression_statement(&self, span: Span, expression: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_expression_statement(a, sc, sl, scol, so, el, ecol, eo, expression) }
    }

    pub fn create_empty_statement(&self, span: Span) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_empty_statement(a, sc, sl, scol, so, el, ecol, eo) }
    }

    pub fn create_return_statement(&self, span: Span, argument: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_return_statement(a, sc, sl, scol, so, el, ecol, eo, argument) }
    }

    pub fn create_throw_statement(&self, span: Span, argument: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_throw_statement(a, sc, sl, scol, so, el, ecol, eo, argument) }
    }

    pub fn create_break_statement(&self, span: Span, label: Option<&[u16]>) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        let (ptr, len) = match label {
            Some(l) => (l.as_ptr(), l.len()),
            None => (ptr::null(), 0),
        };
        unsafe { ast_create_break_statement(a, sc, sl, scol, so, el, ecol, eo, ptr, len) }
    }

    pub fn create_continue_statement(&self, span: Span, label: Option<&[u16]>) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        let (ptr, len) = match label {
            Some(l) => (l.as_ptr(), l.len()),
            None => (ptr::null(), 0),
        };
        unsafe { ast_create_continue_statement(a, sc, sl, scol, so, el, ecol, eo, ptr, len) }
    }

    pub fn create_debugger_statement(&self, span: Span) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_debugger_statement(a, sc, sl, scol, so, el, ecol, eo) }
    }

    pub fn create_if_statement(&self, span: Span, predicate: NodeHandle, consequent: NodeHandle, alternate: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_if_statement(a, sc, sl, scol, so, el, ecol, eo, predicate, consequent, alternate) }
    }

    pub fn create_while_statement(&self, span: Span, test: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_while_statement(a, sc, sl, scol, so, el, ecol, eo, test, body) }
    }

    pub fn create_do_while_statement(&self, span: Span, test: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_do_while_statement(a, sc, sl, scol, so, el, ecol, eo, test, body) }
    }

    pub fn create_for_statement(&self, span: Span, init: NodeHandle, test: NodeHandle, update: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_for_statement(a, sc, sl, scol, so, el, ecol, eo, init, test, update, body) }
    }

    pub fn create_for_in_statement(&self, span: Span, lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_for_in_statement(a, sc, sl, scol, so, el, ecol, eo, lhs, rhs, body) }
    }

    pub fn create_for_of_statement(&self, span: Span, lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_for_of_statement(a, sc, sl, scol, so, el, ecol, eo, lhs, rhs, body) }
    }

    pub fn create_for_await_of_statement(&self, span: Span, lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_for_await_of_statement(a, sc, sl, scol, so, el, ecol, eo, lhs, rhs, body) }
    }

    pub fn create_with_statement(&self, span: Span, object: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_with_statement(a, sc, sl, scol, so, el, ecol, eo, object, body) }
    }

    pub fn create_labelled_statement(&self, span: Span, label: &[u16], body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_labelled_statement(a, sc, sl, scol, so, el, ecol, eo, label.as_ptr(), label.len(), body) }
    }

    pub fn create_switch_statement(&self, span: Span, discriminant: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_switch_statement(a, sc, sl, scol, so, el, ecol, eo, discriminant) }
    }

    pub fn create_switch_case(&self, span: Span, test: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_switch_case(a, sc, sl, scol, so, el, ecol, eo, test) }
    }

    pub fn switch_statement_add_case(&self, switch_stmt: NodeHandle, switch_case: NodeHandle) {
        unsafe { ast_switch_statement_add_case(switch_stmt, switch_case) }
    }

    pub fn create_try_statement(&self, span: Span, block: NodeHandle, handler: NodeHandle, finalizer: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_try_statement(a, sc, sl, scol, so, el, ecol, eo, block, handler, finalizer) }
    }

    pub fn create_catch_clause(&self, span: Span, parameter: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_catch_clause(a, sc, sl, scol, so, el, ecol, eo, parameter, body) }
    }

    // === Declarations ===

    pub fn create_variable_declaration(&self, span: Span, kind: u8, declarators: &[NodeHandle]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_variable_declaration(a, sc, sl, scol, so, el, ecol, eo, kind, declarators.as_ptr(), declarators.len()) }
    }

    pub fn create_variable_declarator(&self, span: Span, target: NodeHandle, init: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_variable_declarator(a, sc, sl, scol, so, el, ecol, eo, target, init) }
    }

    pub fn create_using_declaration(&self, span: Span, declarators: &[NodeHandle]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_using_declaration(a, sc, sl, scol, so, el, ecol, eo, declarators.as_ptr(), declarators.len()) }
    }

    // === Object/Array ===

    pub fn create_object_expression(&self, span: Span, properties: &[NodeHandle]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_object_expression(a, sc, sl, scol, so, el, ecol, eo, properties.as_ptr(), properties.len()) }
    }

    pub fn create_object_property(&self, span: Span, key: NodeHandle, value: NodeHandle, prop_type: u8, is_method: bool) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_object_property(a, sc, sl, scol, so, el, ecol, eo, key, value, prop_type, is_method) }
    }

    pub fn create_array_expression(&self, span: Span, elements: &[NodeHandle]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_array_expression(a, sc, sl, scol, so, el, ecol, eo, elements.as_ptr(), elements.len()) }
    }

    // === Template literals ===

    pub fn create_template_literal(&self, span: Span, expressions: &[NodeHandle]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_template_literal(a, sc, sl, scol, so, el, ecol, eo, expressions.as_ptr(), expressions.len()) }
    }

    pub fn create_template_literal_with_raw_strings(&self, span: Span, expressions: &[NodeHandle], raw_strings: &[NodeHandle]) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_template_literal_with_raw_strings(a, sc, sl, scol, so, el, ecol, eo, expressions.as_ptr(), expressions.len(), raw_strings.as_ptr(), raw_strings.len()) }
    }

    pub fn create_tagged_template_literal(&self, span: Span, tag: NodeHandle, template_literal: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_tagged_template_literal(a, sc, sl, scol, so, el, ecol, eo, tag, template_literal) }
    }

    // === Functions ===

    pub fn create_function_parameters_empty(&self) -> NodeHandle {
        unsafe { ast_create_function_parameters_empty() }
    }

    pub fn create_function_parameters(&self, bindings: &[NodeHandle], default_values: &[NodeHandle], is_rest: &[bool], is_pattern: &[bool]) -> NodeHandle {
        assert_eq!(bindings.len(), default_values.len());
        assert_eq!(bindings.len(), is_rest.len());
        assert_eq!(bindings.len(), is_pattern.len());
        if bindings.is_empty() {
            return self.create_function_parameters_empty();
        }
        unsafe {
            ast_create_function_parameters(
                self.arena, bindings.as_ptr(), default_values.as_ptr(),
                is_rest.as_ptr(), is_pattern.as_ptr(), bindings.len(),
            )
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_function_expression(
        &self, span: Span, name: NodeHandle,
        source_text_start: u32, source_text_len: u32,
        body: NodeHandle, parameters: NodeHandle,
        function_length: i32, kind: u8,
        is_strict_mode: bool, is_arrow_function: bool,
        uses_this: bool, uses_this_from_environment: bool,
        contains_direct_call_to_eval: bool, might_need_arguments_object: bool,
    ) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe {
            ast_create_function_expression(
                a, sc, sl, scol, so, el, ecol, eo,
                name, source_text_start, source_text_len,
                body, parameters, function_length, kind,
                is_strict_mode, is_arrow_function,
                uses_this, uses_this_from_environment,
                contains_direct_call_to_eval, might_need_arguments_object,
            )
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn create_function_declaration(
        &self, span: Span, name: NodeHandle,
        source_text_start: u32, source_text_len: u32,
        body: NodeHandle, parameters: NodeHandle,
        function_length: i32, kind: u8,
        is_strict_mode: bool,
        uses_this: bool, uses_this_from_environment: bool,
        contains_direct_call_to_eval: bool, might_need_arguments_object: bool,
    ) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe {
            ast_create_function_declaration(
                a, sc, sl, scol, so, el, ecol, eo,
                name, source_text_start, source_text_len,
                body, parameters, function_length, kind,
                is_strict_mode,
                uses_this, uses_this_from_environment,
                contains_direct_call_to_eval, might_need_arguments_object,
            )
        }
    }

    // === Classes ===

    pub fn create_class_expression(
        &self, span: Span, name: NodeHandle,
        source_text_start: u32, source_text_len: u32,
        constructor: NodeHandle, super_class: NodeHandle,
        elements: &[NodeHandle],
    ) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe {
            ast_create_class_expression(
                a, sc, sl, scol, so, el, ecol, eo,
                name, source_text_start, source_text_len,
                constructor, super_class,
                elements.as_ptr(), elements.len(),
            )
        }
    }

    pub fn create_class_declaration(&self, span: Span, class_expression: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_class_declaration(a, sc, sl, scol, so, el, ecol, eo, class_expression) }
    }

    pub fn create_class_method(&self, span: Span, key: NodeHandle, function: NodeHandle, kind: u8, is_static: bool) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_class_method(a, sc, sl, scol, so, el, ecol, eo, key, function, kind, is_static) }
    }

    pub fn create_class_field(&self, span: Span, key: NodeHandle, init: NodeHandle, is_static: bool) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_class_field(a, sc, sl, scol, so, el, ecol, eo, key, init, is_static) }
    }

    pub fn create_static_initializer(&self, span: Span, function_body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_static_initializer(a, sc, sl, scol, so, el, ecol, eo, function_body) }
    }

    // === Scope ===

    pub fn scope_node_add_var_scoped_declaration(&self, scope_node: NodeHandle, declaration: NodeHandle) {
        unsafe { ast_scope_node_add_var_scoped_declaration(scope_node, declaration) }
    }

    pub fn scope_node_add_lexical_declaration(&self, scope_node: NodeHandle, declaration: NodeHandle) {
        unsafe { ast_scope_node_add_lexical_declaration(scope_node, declaration) }
    }

    pub fn scope_node_add_hoisted_function(&self, scope_node: NodeHandle, function_declaration: NodeHandle) {
        unsafe { ast_scope_node_add_hoisted_function(scope_node, function_declaration) }
    }

    pub fn scope_node_shrink_to_fit(&self, scope_node: NodeHandle) {
        unsafe { ast_scope_node_shrink_to_fit(scope_node) }
    }

    pub fn switch_case_append(&self, switch_case: NodeHandle, statement: NodeHandle) {
        unsafe { ast_switch_case_append(switch_case, statement) }
    }

    // === BindingPattern ===

    pub fn create_binding_pattern(&self, kind: u8) -> NodeHandle {
        unsafe { ast_create_binding_pattern(self.arena, kind) }
    }

    pub fn binding_pattern_append_entry(
        &self, pattern: NodeHandle,
        name: NodeHandle, name_type: u8,
        alias: NodeHandle, alias_type: u8,
        initializer: NodeHandle, is_rest: bool,
    ) {
        unsafe { ast_binding_pattern_append_entry(pattern, name, name_type, alias, alias_type, initializer, is_rest) }
    }

    pub fn create_variable_declarator_with_pattern(&self, span: Span, pattern: NodeHandle, init: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_variable_declarator_with_pattern(a, sc, sl, scol, so, el, ecol, eo, pattern, init) }
    }

    pub fn create_catch_clause_with_pattern(&self, span: Span, pattern: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_catch_clause_with_pattern(a, sc, sl, scol, so, el, ecol, eo, pattern, body) }
    }

    pub fn create_for_in_statement_with_pattern(&self, span: Span, pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_for_in_statement_with_pattern(a, sc, sl, scol, so, el, ecol, eo, pattern, rhs, body) }
    }

    pub fn create_for_of_statement_with_pattern(&self, span: Span, pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_for_of_statement_with_pattern(a, sc, sl, scol, so, el, ecol, eo, pattern, rhs, body) }
    }

    pub fn create_for_await_of_statement_with_pattern(&self, span: Span, pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_for_await_of_statement_with_pattern(a, sc, sl, scol, so, el, ecol, eo, pattern, rhs, body) }
    }

    // === OptionalChain ===

    pub fn create_optional_chain_builder(&self) -> *mut std::ffi::c_void {
        unsafe { ast_create_optional_chain_builder() }
    }

    pub fn optional_chain_builder_append_member(&self, builder: *mut std::ffi::c_void, identifier: NodeHandle, is_optional: bool) {
        unsafe { ast_optional_chain_builder_append_member(builder, identifier, is_optional) }
    }

    pub fn optional_chain_builder_append_computed(&self, builder: *mut std::ffi::c_void, expression: NodeHandle, is_optional: bool) {
        unsafe { ast_optional_chain_builder_append_computed(builder, expression, is_optional) }
    }

    pub fn optional_chain_builder_append_call(&self, builder: *mut std::ffi::c_void, argument_values: &[NodeHandle], argument_is_spread: &[bool], is_optional: bool) {
        unsafe { ast_optional_chain_builder_append_call(builder, argument_values.as_ptr(), argument_is_spread.as_ptr(), argument_values.len(), is_optional) }
    }

    pub fn optional_chain_builder_append_private_member(&self, builder: *mut std::ffi::c_void, private_identifier: NodeHandle, is_optional: bool) {
        unsafe { ast_optional_chain_builder_append_private_member(builder, private_identifier, is_optional) }
    }

    pub fn create_optional_chain(&self, span: Span, base: NodeHandle, builder: *mut std::ffi::c_void) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe { ast_create_optional_chain(a, sc, sl, scol, so, el, ecol, eo, base, builder) }
    }

    // === Import/Export ===

    pub fn create_import_statement(
        &self, span: Span,
        module_specifier: &[u16],
        entries: &[FFIImportEntry],
    ) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        unsafe {
            ast_create_import_statement(
                a, sc, sl, scol, so, el, ecol, eo,
                module_specifier.as_ptr(), module_specifier.len(),
                entries.as_ptr(), entries.len(),
            )
        }
    }

    pub fn create_export_statement(
        &self, span: Span,
        statement: NodeHandle,
        entries: &[FFIExportEntry],
        is_default: bool,
        from_specifier: Option<&[u16]>,
    ) -> NodeHandle {
        let (a, sc, sl, scol, so, el, ecol, eo) = self.s(span);
        let (from_ptr, from_len) = match from_specifier {
            Some(s) => (s.as_ptr(), s.len()),
            None => (ptr::null(), usize::MAX),
        };
        unsafe {
            ast_create_export_statement(
                a, sc, sl, scol, so, el, ecol, eo,
                statement, entries.as_ptr(), entries.len(),
                is_default, from_ptr, from_len,
            )
        }
    }

    pub fn import_statement_add_attribute(&self, import_stmt: NodeHandle, key: &[u16], value: &[u16]) {
        unsafe { ast_import_statement_add_attribute(import_stmt, key.as_ptr(), key.len(), value.as_ptr(), value.len()) }
    }

    pub fn export_statement_add_attribute(&self, export_stmt: NodeHandle, key: &[u16], value: &[u16]) {
        unsafe { ast_export_statement_add_attribute(export_stmt, key.as_ptr(), key.len(), value.as_ptr(), value.len()) }
    }

    pub fn program_append_import(&self, program: NodeHandle, import_stmt: NodeHandle) {
        unsafe { ast_program_append_import(program, import_stmt) }
    }

    pub fn program_append_export(&self, program: NodeHandle, export_stmt: NodeHandle) {
        unsafe { ast_program_append_export(program, export_stmt) }
    }

    pub fn get_declaration_export_names(&self, declaration: NodeHandle) -> Vec<Vec<u16>> {
        let mut buf = [FFIUtf16Slice { data: std::ptr::null(), len: 0 }; 64];
        let count = unsafe {
            ast_get_declaration_export_names(declaration, buf.as_mut_ptr(), buf.len())
        };
        let mut result = Vec::with_capacity(count);
        for i in 0..count.min(buf.len()) {
            let slice = &buf[i];
            let s = unsafe { std::slice::from_raw_parts(slice.data, slice.len) };
            result.push(s.to_vec());
        }
        result
    }

    pub fn get_function_name(&self, handle: NodeHandle) -> Vec<u16> {
        let slice = unsafe { ast_get_function_name(handle) };
        if slice.data.is_null() || slice.len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(slice.data, slice.len) }.to_vec()
        }
    }

    pub fn get_class_name(&self, handle: NodeHandle) -> Vec<u16> {
        let slice = unsafe { ast_get_class_name(handle) };
        if slice.data.is_null() || slice.len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(slice.data, slice.len) }.to_vec()
        }
    }

    pub fn function_has_name(&self, handle: NodeHandle) -> bool {
        unsafe { ast_function_has_name(handle) }
    }
}

impl Drop for AstBuilder {
    fn drop(&mut self) {
        unsafe { ast_arena_destroy(self.arena) }
    }
}
