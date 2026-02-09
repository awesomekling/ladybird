/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Safe wrappers around the C++ AST factory functions.
//! Every function here calls an `extern "C"` factory defined in ASTFactory.cpp.

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

extern "C" {
    // Arena
    pub fn ast_arena_create() -> ArenaHandle;
    pub fn ast_arena_destroy(arena: ArenaHandle);
    pub fn ast_node_ref(handle: NodeHandle);

    // Program / ScopeNode
    pub fn ast_create_program(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        program_type: u8,
    ) -> NodeHandle;
    pub fn ast_create_block_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    pub fn ast_create_function_body(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    pub fn ast_scope_node_append(scope_node: NodeHandle, statement: NodeHandle);
    pub fn ast_scope_node_set_strict_mode(scope_node: NodeHandle);

    // Literals
    pub fn ast_create_numeric_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        value: f64,
    ) -> NodeHandle;
    pub fn ast_create_string_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        value: *const u16, value_len: usize,
    ) -> NodeHandle;
    pub fn ast_create_boolean_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        value: bool,
    ) -> NodeHandle;
    pub fn ast_create_null_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    pub fn ast_create_bigint_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        value: *const u8, value_len: usize,
    ) -> NodeHandle;
    pub fn ast_create_regexp_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: *const u16, pattern_len: usize,
        flags: *const u16, flags_len: usize,
    ) -> NodeHandle;

    // Identifiers
    pub fn ast_create_identifier(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        name: *const u16, name_len: usize,
    ) -> NodeHandle;
    pub fn ast_create_private_identifier(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        name: *const u16, name_len: usize,
    ) -> NodeHandle;

    // Scope analysis
    pub fn ast_identifier_set_local_variable_index(identifier: NodeHandle, index: u32);
    pub fn ast_identifier_set_argument_index(identifier: NodeHandle, index: u32);
    pub fn ast_identifier_set_is_global(identifier: NodeHandle);
    pub fn ast_identifier_set_is_inside_scope_with_eval(identifier: NodeHandle);
    pub fn ast_identifier_set_declaration_kind(identifier: NodeHandle, kind: u8);
    pub fn ast_identifier_is_local(identifier: NodeHandle) -> bool;
    pub fn ast_identifier_is_inside_scope_with_eval(identifier: NodeHandle) -> bool;
    pub fn ast_scope_node_add_local_variable(
        scope: NodeHandle,
        name: *const u16,
        name_len: usize,
        declaration_kind: u8,
    ) -> u32;

    // Expressions
    pub fn ast_create_this_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    pub fn ast_create_super_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    pub fn ast_create_binary_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, lhs: NodeHandle, rhs: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_logical_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, lhs: NodeHandle, rhs: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_unary_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, operand: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_update_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, argument: NodeHandle, prefixed: bool,
    ) -> NodeHandle;
    pub fn ast_create_assignment_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        op: u8, lhs: NodeHandle, rhs: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_conditional_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        test: NodeHandle, consequent: NodeHandle, alternate: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_sequence_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        expressions: *const NodeHandle, count: usize,
    ) -> NodeHandle;
    pub fn ast_create_member_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        object: NodeHandle, property: NodeHandle, computed: bool,
    ) -> NodeHandle;
    pub fn ast_create_call_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        callee: NodeHandle,
        argument_values: *const NodeHandle, argument_is_spread: *const bool,
        argument_count: usize,
    ) -> NodeHandle;
    pub fn ast_create_super_call(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument_values: *const NodeHandle, argument_is_spread: *const bool,
        argument_count: usize,
    ) -> NodeHandle;
    pub fn ast_create_synthetic_constructor_super_call(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument_identifier: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_new_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        callee: NodeHandle,
        argument_values: *const NodeHandle, argument_is_spread: *const bool,
        argument_count: usize,
    ) -> NodeHandle;
    pub fn ast_create_spread_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        target: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_yield_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument: NodeHandle, is_yield_from: bool,
    ) -> NodeHandle;
    pub fn ast_create_await_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_import_call(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        specifier: NodeHandle, options: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_meta_property(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        meta_type: u8,
    ) -> NodeHandle;

    // Statements
    pub fn ast_create_expression_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        expression: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_empty_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    pub fn ast_create_return_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_throw_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        argument: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_break_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        label: *const u16, label_len: usize,
    ) -> NodeHandle;
    pub fn ast_create_continue_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        label: *const u16, label_len: usize,
    ) -> NodeHandle;
    pub fn ast_create_debugger_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
    ) -> NodeHandle;
    pub fn ast_create_if_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        predicate: NodeHandle, consequent: NodeHandle, alternate: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_while_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        test: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_do_while_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        test: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_for_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        init: NodeHandle, test: NodeHandle, update: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_for_in_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_for_of_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_for_await_of_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        lhs: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_with_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        object: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_labelled_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        label: *const u16, label_len: usize, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_switch_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        discriminant: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_switch_case(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        test: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_switch_statement_add_case(switch_stmt: NodeHandle, switch_case: NodeHandle);
    pub fn ast_create_try_statement(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        block: NodeHandle, handler: NodeHandle, finalizer: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_catch_clause(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        parameter: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;

    // Declarations
    pub fn ast_create_variable_declaration(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        kind: u8, declarators: *const NodeHandle, declarator_count: usize,
    ) -> NodeHandle;
    pub fn ast_create_variable_declarator(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        target: NodeHandle, init: NodeHandle,
    ) -> NodeHandle;

    // Object/Array expressions
    pub fn ast_create_object_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        properties: *const NodeHandle, property_count: usize,
    ) -> NodeHandle;
    pub fn ast_create_object_property(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        key: NodeHandle, value: NodeHandle, prop_type: u8, is_method: bool,
    ) -> NodeHandle;
    pub fn ast_create_array_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        elements: *const NodeHandle, element_count: usize,
    ) -> NodeHandle;

    // Template literals
    pub fn ast_create_template_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        expressions: *const NodeHandle, expression_count: usize,
    ) -> NodeHandle;
    pub fn ast_create_tagged_template_literal(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        tag: NodeHandle, template_literal: NodeHandle,
    ) -> NodeHandle;

    // Functions
    pub fn ast_create_function_parameters_empty() -> NodeHandle;
    pub fn ast_create_function_parameters(
        arena: ArenaHandle,
        bindings: *const NodeHandle, default_values: *const NodeHandle,
        is_rest: *const bool, is_pattern: *const bool, count: usize,
    ) -> NodeHandle;
    pub fn ast_create_function_expression(
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
    pub fn ast_create_function_declaration(
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

    // Classes
    pub fn ast_create_class_expression(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        name: NodeHandle,
        source_text_start: u32, source_text_len: u32,
        constructor: NodeHandle, super_class: NodeHandle,
        elements: *const NodeHandle, element_count: usize,
    ) -> NodeHandle;
    pub fn ast_create_class_declaration(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        class_expression: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_class_method(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        key: NodeHandle, function: NodeHandle, kind: u8, is_static: bool,
    ) -> NodeHandle;
    pub fn ast_create_class_field(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        key: NodeHandle, init: NodeHandle, is_static: bool,
    ) -> NodeHandle;
    pub fn ast_create_static_initializer(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        function_body: NodeHandle,
    ) -> NodeHandle;

    // Scope declarations
    pub fn ast_scope_node_add_var_scoped_declaration(scope_node: NodeHandle, declaration: NodeHandle);
    pub fn ast_scope_node_add_lexical_declaration(scope_node: NodeHandle, declaration: NodeHandle);
    pub fn ast_scope_node_add_hoisted_function(scope_node: NodeHandle, function_declaration: NodeHandle);
    pub fn ast_scope_node_shrink_to_fit(scope_node: NodeHandle);

    // SwitchCase
    pub fn ast_switch_case_append(switch_case: NodeHandle, statement: NodeHandle);

    // BindingPattern
    pub fn ast_create_binding_pattern(arena: ArenaHandle, kind: u8) -> NodeHandle;
    pub fn ast_binding_pattern_append_entry(
        pattern: NodeHandle,
        name: NodeHandle, name_type: u8,
        alias: NodeHandle, alias_type: u8,
        initializer: NodeHandle, is_rest: bool,
    );
    pub fn ast_create_variable_declarator_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, init: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_catch_clause_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_for_in_statement_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_for_of_statement_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
    pub fn ast_create_for_await_of_statement_with_pattern(
        arena: ArenaHandle, source_code: SourceCodeHandle,
        start_line: u32, start_column: u32, start_offset: u32,
        end_line: u32, end_column: u32, end_offset: u32,
        pattern: NodeHandle, rhs: NodeHandle, body: NodeHandle,
    ) -> NodeHandle;
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
}

impl Drop for AstBuilder {
    fn drop(&mut self) {
        unsafe { ast_arena_destroy(self.arena) }
    }
}
