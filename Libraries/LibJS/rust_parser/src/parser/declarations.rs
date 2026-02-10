/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Declaration parsing: variables, functions, classes, imports, exports.
//!
//! ## Function parsing flow
//!
//! Parsing a function (declaration, expression, method, or arrow) follows
//! this sequence:
//!
//! 1. Save `function_might_need_arguments_object`, reset to `false`
//! 2. Consume `async`, `function`, `*`, and optional name
//! 3. Open a function scope in the scope collector
//! 4. Set generator/async context flags (needed for default parameter
//!    expressions, e.g., `function f(x = yield)` must know the context)
//! 5. Parse formal parameters → `(params, function_length, param_info)`
//! 6. Restore generator/async flags (body parsing saves/restores these)
//! 7. Parse function body → `(body, has_use_strict, insights)`
//! 8. If `has_use_strict` or non-normal kind: retroactively check
//!    function name and parameter names for strict mode violations
//! 9. Read `function_might_need_arguments_object` and restore saved value
//! 10. Create the AST node via the builder
//!
//! Arrow functions differ: they use speculative parsing (save/load state)
//! and do NOT save/restore `function_might_need_arguments_object` (arrows
//! don't have their own `arguments` object, so the flag propagates up).
//!
//! ## Scope collector interaction
//!
//! Variable and function declarations are registered with the scope
//! collector during parsing so that scope analysis can resolve identifiers.
//! - `var` declarations: `add_var_declaration()` (hoists to function scope)
//! - `let`/`const` declarations: `add_lexical_declaration()` (block-scoped)
//! - Function declarations: `add_function_declaration()` (hoists, Annex B)
//! - Class declarations: `add_lexical_declaration()` (block-scoped)

use crate::ast_bridge::{FFIExportEntry, FFIImportEntry, NodeHandle, NULL_HANDLE};
use crate::ffi_enums::{BindingEntryAlias, BindingEntryName, BindingPattern, ClassMethod};
use crate::parser::{Associativity, DeclarationKind, ForbiddenTokens, FunctionKind, FunctionParsingInsights, Parser, ProgramType};
use crate::token::TokenType;

impl<'a> Parser<'a> {
    // === Declarations ===

    pub(crate) fn parse_declaration(&mut self) -> NodeHandle {
        if self.match_token(TokenType::Async) {
            let next = self.next_token();
            if next.token_type == TokenType::Function && !next.trivia_has_line_terminator {
                let decl = self.parse_function_declaration();
                self.register_function_declaration_with_scope_collector(decl);
                return decl;
            }
        }

        match self.current_token_type() {
            TokenType::Function => {
                let decl = self.parse_function_declaration();
                self.register_function_declaration_with_scope_collector(decl);
                decl
            }
            TokenType::Class => {
                let decl = self.parse_class_declaration();
                let class_name = std::mem::take(&mut self.last_class_name);
                let class_name_id = self.last_class_name_id;
                if !class_name.is_empty() {
                    let pos = self.position();
                    self.scope_collector.add_lexical_declaration(
                        decl, &[class_name.as_slice()], pos.line, pos.column,
                    );
                    if class_name_id != NULL_HANDLE {
                        self.scope_collector.register_identifier(class_name_id, &class_name, Some(DeclarationKind::Let));
                    }
                }
                decl
            }
            TokenType::Let | TokenType::Const => {
                // Scope collector registration happens inside parse_variable_declaration.
                self.parse_variable_declaration(false)
            }
            TokenType::Identifier if self.token_value(&self.current_token) == utf16!("using") => {
                if !self.scope_collector.can_have_using_declaration() {
                    self.syntax_error("'using' not allowed outside of block, for loop or function");
                }
                self.parse_using_declaration(false)
            }
            _ => {
                self.expected("declaration");
                self.consume();
                self.builder.create_empty_statement(self.span_from(self.position()))
            }
        }
    }

    /// Register a function declaration with the scope collector.
    fn register_function_declaration_with_scope_collector(&mut self, decl: NodeHandle) {
        let name = std::mem::take(&mut self.last_function_name);
        let name_id = self.last_function_name_id;
        let kind = self.last_function_kind;
        let pos = self.position();
        self.scope_collector.add_function_declaration(
            decl, &name, name_id, kind, self.strict_mode, pos.line, pos.column,
        );
    }

    // === Variable declaration ===

    pub(crate) fn parse_variable_declaration(&mut self, is_for_loop: bool) -> NodeHandle {
        let start = self.position();

        let kind = match self.current_token_type() {
            TokenType::Var => DeclarationKind::Var,
            TokenType::Let => DeclarationKind::Let,
            TokenType::Const => DeclarationKind::Const,
            _ => {
                self.expected("variable declaration keyword");
                DeclarationKind::Var
            }
        };
        self.consume();

        let mut declarators = Vec::new();
        let mut var_bound_names: Vec<(Vec<u16>, NodeHandle)> = Vec::new();
        let mut lexical_bound_names: Vec<Vec<u16>> = Vec::new();
        let mut any_init = false;

        loop {
            let decl_start = self.position();
            // Parse target (identifier or binding pattern)
            let (target, is_pattern) = if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.check_identifier_name_for_assignment_validity(&value, false);
                let id = self.builder.create_identifier(self.span_from(decl_start), &value);
                // Track bound names for scope collector
                if kind == DeclarationKind::Var {
                    var_bound_names.push((value.clone(), id));
                } else {
                    lexical_bound_names.push(value.clone());
                }
                // Register identifier reference
                self.scope_collector.register_identifier(id, &value, Some(kind));
                (id, false)
            } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                let pat = self.parse_binding_pattern();
                for (n, id) in std::mem::take(&mut self.pattern_bound_names) {
                    if kind == DeclarationKind::Var {
                        var_bound_names.push((n, id));
                    } else {
                        self.scope_collector.register_identifier(id, &n, Some(kind));
                        lexical_bound_names.push(n);
                    }
                }
                (pat, true)
            } else {
                self.expected("variable name");
                self.consume();
                (self.builder.create_identifier(self.span_from(decl_start), &[]), false)
            };

            // Parse optional initializer
            let init = if self.match_token(TokenType::Equals) {
                self.consume();
                any_init = true;
                let forbidden = if is_for_loop {
                    ForbiddenTokens::with_in()
                } else {
                    ForbiddenTokens::none()
                };
                self.parse_expression(2, Associativity::Right, forbidden)
            } else {
                NULL_HANDLE
            };

            let declarator = if is_pattern {
                self.builder.create_variable_declarator_with_pattern(self.span_from(decl_start), target, init)
            } else {
                self.builder.create_variable_declarator(self.span_from(decl_start), target, init)
            };
            declarators.push(declarator);

            if !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();
        }

        if !is_for_loop {
            self.consume_or_insert_semicolon();
        }

        let decl = self.builder.create_variable_declaration(self.span_from(start), kind as u8, &declarators);

        // Track info for for-in/of validation.
        if is_for_loop {
            self.for_loop_declaration_count = declarators.len();
            self.for_loop_declaration_has_init = any_init;
            self.for_loop_declaration_is_var = kind == DeclarationKind::Var;
        }

        // Register with scope collector.
        if self.scope_collector.has_current_scope() {
            match kind {
                DeclarationKind::Var => {
                    let names: Vec<(&[u16], NodeHandle)> = var_bound_names.iter()
                        .map(|(n, h)| (n.as_slice(), *h))
                        .collect();
                    self.scope_collector.add_var_declaration(decl, &names, start.line, start.column);
                }
                DeclarationKind::Let | DeclarationKind::Const => {
                    let names: Vec<&[u16]> = lexical_bound_names.iter()
                        .map(|n| n.as_slice())
                        .collect();
                    self.scope_collector.add_lexical_declaration(decl, &names, start.line, start.column);
                }
            }
        }

        decl
    }

    // === Using declaration ===

    pub(crate) fn parse_using_declaration(&mut self, is_for_loop: bool) -> NodeHandle {
        let start = self.position();

        // Consume "using" identifier.
        self.consume(); // consume 'using'

        let mut declarators = Vec::new();
        let mut bound_names: Vec<Vec<u16>> = Vec::new();

        loop {
            let decl_start = self.position();

            // Parse binding: must be a simple identifier.
            if !self.match_identifier() {
                self.expected("identifier");
                break;
            }
            let tok = self.consume();
            let name = self.token_value(&tok).to_vec();

            self.check_identifier_name_for_assignment_validity(&name, false);
            if name == utf16!("let") {
                self.syntax_error("Lexical binding may not be called 'let'");
            }

            let identifier = self.builder.create_identifier(self.span_from(decl_start), &name);
            self.scope_collector.register_identifier(identifier, &name, Some(DeclarationKind::Const));
            bound_names.push(name);

            // Parse initializer.
            let init = if self.match_token(TokenType::Equals) {
                self.consume();
                if is_for_loop {
                    self.parse_expression(2, Associativity::Right, ForbiddenTokens::with_in())
                } else {
                    self.parse_expression(2, Associativity::Right, ForbiddenTokens::none())
                }
            } else if !is_for_loop {
                // Initializer is required outside of for-loop context.
                self.consume_token(TokenType::Equals);
                NULL_HANDLE
            } else {
                NULL_HANDLE
            };

            declarators.push(self.builder.create_variable_declarator(
                self.span_from(decl_start), identifier, init));

            if self.match_token(TokenType::Comma) {
                self.consume();
                continue;
            }
            break;
        }

        if !is_for_loop {
            self.consume_or_insert_semicolon();
        }

        let decl = self.builder.create_using_declaration(self.span_from(start), &declarators);

        // Register with scope collector as a lexical declaration.
        let names: Vec<&[u16]> = bound_names.iter().map(|n| n.as_slice()).collect();
        self.scope_collector.add_lexical_declaration(decl, &names, start.line, start.column);

        decl
    }

    // === Function declaration ===

    fn parse_function_declaration(&mut self) -> NodeHandle {
        let start = self.position();

        // Save and reset function_might_need_arguments_object for this function scope.
        let saved_might_need_arguments = self.function_might_need_arguments_object;
        self.function_might_need_arguments_object = false;

        let is_async = if self.match_token(TokenType::Async) {
            self.consume();
            true
        } else {
            false
        };

        self.consume_token(TokenType::Function);

        let is_generator = if self.match_token(TokenType::Asterisk) {
            self.consume();
            true
        } else {
            false
        };

        let kind = match (is_async, is_generator) {
            (true, true) => FunctionKind::AsyncGenerator,
            (true, false) => FunctionKind::Async,
            (false, true) => FunctionKind::Generator,
            (false, false) => FunctionKind::Normal,
        };

        // Parse function name
        let name = if self.has_default_export_name && !self.match_identifier() {
            // export default function() {} -- use *default* as name.
            let default_name = utf16!("*default*");
            self.last_function_name = default_name.to_vec();
            let id = self.builder.create_identifier(self.span_from(start), default_name);
            self.last_function_name_id = id;
            id
        } else if self.match_identifier() {
            let tok = self.consume();
            let value = self.token_value(&tok).to_vec();
            self.last_function_name = value.clone();
            let id = self.builder.create_identifier(self.span_from(start), &value);
            self.last_function_name_id = id;
            id
        } else {
            self.last_function_name.clear();
            self.last_function_name_id = NULL_HANDLE;
            NULL_HANDLE
        };
        self.last_function_kind = kind;

        let fn_name = self.last_function_name.clone();
        let fn_name_ref = if fn_name.is_empty() { None } else { Some(fn_name.as_slice()) };
        self.scope_collector.open_function_scope(fn_name_ref);
        self.scope_collector.set_is_function_declaration();

        // Set generator/async context before parsing parameters, since default
        // parameter values are evaluated in the function's context (e.g. `yield`
        // should be an identifier in a non-generator nested inside a generator).
        let in_generator_before = self.in_generator_function_context;
        let await_before = self.await_expression_is_valid;
        self.in_generator_function_context = is_generator;
        self.await_expression_is_valid = is_async;

        let (params, function_length, param_info, is_simple) = self.parse_formal_parameters();

        // Restore before parse_function_body (which saves/restores these itself).
        self.in_generator_function_context = in_generator_before;
        self.await_expression_is_valid = await_before;

        // Save function name state before body parsing, which may recursively
        // parse nested function declarations that clobber these fields.
        let saved_fn_name = self.last_function_name.clone();
        let saved_fn_name_id = self.last_function_name_id;
        let saved_fn_kind = self.last_function_kind;

        let (body, has_use_strict, insights) = self.parse_function_body(is_async, is_generator, &param_info, is_simple);

        // Retroactive strict mode checks on function name and parameters.
        if has_use_strict || kind != FunctionKind::Normal {
            let force_strict = has_use_strict;
            if name != NULL_HANDLE {
                self.check_identifier_name_for_assignment_validity(&fn_name, force_strict);
            }
            self.check_parameters_post_body(&param_info, force_strict, kind);
        }

        // Restore so register_function_declaration_with_scope_collector uses the right values.
        self.last_function_name = saved_fn_name;
        self.last_function_name_id = saved_fn_name_id;
        self.last_function_kind = saved_fn_kind;

        let might_need_arguments = self.function_might_need_arguments_object;
        self.function_might_need_arguments_object = saved_might_need_arguments;

        let span = self.span_from(start);
        self.builder.create_function_declaration(
            span, name,
            start.offset, self.source_text_end_offset() - start.offset,
            body, params, function_length, kind as u8,
            self.strict_mode || has_use_strict,
            insights.uses_this, insights.uses_this_from_environment,
            insights.contains_direct_call_to_eval, might_need_arguments,
        )
    }

    // === Function expression ===

    pub(crate) fn parse_function_expression(&mut self) -> NodeHandle {
        let start = self.position();

        // Save and reset function_might_need_arguments_object for this function scope.
        let saved_might_need_arguments = self.function_might_need_arguments_object;
        self.function_might_need_arguments_object = false;

        let is_async = if self.match_token(TokenType::Async) {
            self.consume();
            true
        } else {
            false
        };

        self.consume_token(TokenType::Function);

        let is_generator = if self.match_token(TokenType::Asterisk) {
            self.consume();
            true
        } else {
            false
        };

        let kind = match (is_async, is_generator) {
            (true, true) => FunctionKind::AsyncGenerator,
            (true, false) => FunctionKind::Async,
            (false, true) => FunctionKind::Generator,
            (false, false) => FunctionKind::Normal,
        };

        // Optional name
        let mut fn_name_value: Vec<u16> = Vec::new();
        let name = if self.match_identifier() {
            let tok = self.consume();
            fn_name_value = self.token_value(&tok).to_vec();
            self.builder.create_identifier(self.span_from(start), &fn_name_value)
        } else {
            NULL_HANDLE
        };

        let fn_name = if fn_name_value.is_empty() { None } else { Some(fn_name_value.as_slice()) };
        self.scope_collector.open_function_scope(fn_name);

        let in_generator_before = self.in_generator_function_context;
        let await_before = self.await_expression_is_valid;
        self.in_generator_function_context = is_generator;
        self.await_expression_is_valid = is_async;

        let (params, function_length, param_info, is_simple) = self.parse_formal_parameters();

        self.in_generator_function_context = in_generator_before;
        self.await_expression_is_valid = await_before;

        let (body, has_use_strict, insights) = self.parse_function_body(is_async, is_generator, &param_info, is_simple);

        // Retroactive strict mode checks on function name and parameters.
        if has_use_strict || kind != FunctionKind::Normal {
            let force_strict = has_use_strict;
            if name != NULL_HANDLE {
                self.check_identifier_name_for_assignment_validity(&fn_name_value, force_strict);
            }
            self.check_parameters_post_body(&param_info, force_strict, kind);
        }

        let might_need_arguments = self.function_might_need_arguments_object;
        self.function_might_need_arguments_object = saved_might_need_arguments;

        let span = self.span_from(start);
        self.builder.create_function_expression(
            span, name,
            start.offset, self.source_text_end_offset() - start.offset,
            body, params, function_length, kind as u8,
            self.strict_mode || has_use_strict, false,
            insights.uses_this, insights.uses_this_from_environment,
            insights.contains_direct_call_to_eval, might_need_arguments,
        )
    }

    // === Class ===

    pub(crate) fn parse_class_expression(&mut self, expect_name: bool) -> NodeHandle {
        let start = self.position();

        let strict_before = self.strict_mode;
        self.strict_mode = true;

        self.consume_token(TokenType::Class);

        // Optional name
        let name = if expect_name || self.match_identifier() {
            if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.last_class_name = value.clone();
                let id = self.builder.create_identifier(self.span_from(start), &value);
                self.last_class_name_id = id;
                id
            } else if expect_name {
                self.expected("class name");
                self.last_class_name.clear();
                self.last_class_name_id = NULL_HANDLE;
                NULL_HANDLE
            } else {
                self.last_class_name.clear();
                self.last_class_name_id = NULL_HANDLE;
                NULL_HANDLE
            }
        } else {
            self.last_class_name.clear();
            self.last_class_name_id = NULL_HANDLE;
            NULL_HANDLE
        };

        // Save the class name before parsing extends/body, which may recursively
        // call parse_class_expression and clobber last_class_name/last_class_name_id.
        let saved_class_name = self.last_class_name.clone();
        let saved_class_name_id = self.last_class_name_id;

        // Open class declaration scope — makes the class name available as a const
        // binding inside the class body (for both class declarations and expressions).
        let class_name_for_scope = if !saved_class_name.is_empty() { Some(saved_class_name.as_slice()) } else { None };
        self.scope_collector.open_class_declaration_scope(class_name_for_scope);

        // Optional extends
        let super_class = if self.match_token(TokenType::Extends) {
            self.consume();
            self.parse_expression(0, Associativity::Right, ForbiddenTokens::none())
        } else {
            NULL_HANDLE
        };

        // Class body
        self.consume_token(TokenType::CurlyOpen);
        let mut elements = Vec::new();
        let mut constructor_func = NULL_HANDLE;

        let saved_class_has_super = self.class_has_super_class;
        self.class_has_super_class = super_class != NULL_HANDLE;

        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            if self.match_token(TokenType::Semicolon) {
                self.consume();
                continue;
            }

            let (element, maybe_ctor) = self.parse_class_element();
            if let Some(ctor) = maybe_ctor {
                if constructor_func != NULL_HANDLE {
                    self.syntax_error("Classes may not have more than one constructor");
                }
                constructor_func = ctor;
            } else {
                elements.push(element);
            }
        }

        self.consume_token(TokenType::CurlyClose);
        self.class_has_super_class = saved_class_has_super;
        self.strict_mode = strict_before;

        // If no explicit constructor was declared, synthesize a default one.
        // Per the spec, derived classes get `constructor(...args) { super(...args); }`
        // and base classes get an empty `constructor() {}`.
        if constructor_func == NULL_HANDLE {
            let ctor_body = self.builder.create_function_body(self.span_from(start));
            if super_class != NULL_HANDLE {
                // Derived class: constructor(...args) { return super(...args); }
                let args_name: Vec<u16> = "args".encode_utf16().collect();
                let args_ident = self.builder.create_identifier(self.span_from(start), &args_name);
                let super_call = self.builder.create_synthetic_constructor_super_call(self.span_from(start), args_ident);
                let return_stmt = self.builder.create_return_statement(self.span_from(start), super_call);
                self.builder.scope_node_append(ctor_body, return_stmt);
                let args_binding = self.builder.create_identifier(self.span_from(start), &args_name);
                let ctor_params = self.builder.create_function_parameters(
                    &[args_binding], &[NULL_HANDLE],
                    &[true],   // is_rest: ...args
                    &[false],  // has_default
                );
                //                       FunctionKind, is_strict, is_arrow,
                //                       uses_this, uses_this_from_env, eval, arguments_object
                constructor_func = self.builder.create_function_expression(
                    self.span_from(start), name,
                    start.offset, self.source_text_end_offset() - start.offset,
                    ctor_body, ctor_params, 0, FunctionKind::Normal as u8,
                    true, false,
                    true, true, false, false,
                );
            } else {
                // Base class: empty constructor() {}
                let ctor_params = self.builder.create_function_parameters_empty();
                constructor_func = self.builder.create_function_expression(
                    self.span_from(start), name,
                    start.offset, self.source_text_end_offset() - start.offset,
                    ctor_body, ctor_params, 0, FunctionKind::Normal as u8,
                    true, false,
                    true, true, false, false,
                );
            }
        }

        // Close the class declaration scope.
        self.scope_collector.close_scope();

        // Restore class name so parse_class_declaration can use it.
        self.last_class_name = saved_class_name;
        self.last_class_name_id = saved_class_name_id;

        self.builder.create_class_expression(
            self.span_from(start), name,
            start.offset, self.source_text_end_offset() - start.offset,
            constructor_func, super_class,
            &elements,
        )
    }

    pub(crate) fn parse_class_declaration(&mut self) -> NodeHandle {
        let start = self.position();
        let class_expr = self.parse_class_expression(true);
        self.builder.create_class_declaration(self.span_from(start), class_expr)
    }

    fn parse_class_element(&mut self) -> (NodeHandle, Option<NodeHandle>) {
        let start = self.position();
        let is_static = if self.match_token(TokenType::Static) {
            self.consume();
            // static { } block
            if self.match_token(TokenType::CurlyOpen) {
                let body = self.builder.create_function_body(self.span_from(start));
                self.consume_token(TokenType::CurlyOpen);
                let saved_break = self.in_break_context;
                let saved_continue = self.in_continue_context;
                let saved_function = self.in_function_context;
                let saved_generator = self.in_generator_function_context;
                let saved_await = self.await_expression_is_valid;
                let saved_field_init = self.in_class_field_initializer;
                let saved_static_init = self.in_class_static_init_block;
                let saved_super = self.allow_super_property_lookup;
                self.in_break_context = false;
                self.in_continue_context = false;
                self.in_function_context = false;
                self.in_generator_function_context = false;
                self.await_expression_is_valid = false;
                self.in_class_field_initializer = true;
                self.in_class_static_init_block = true;
                self.allow_super_property_lookup = true;
                self.scope_collector.open_static_init_scope(body);
                self.parse_statement_list(body, false);
                self.scope_collector.close_scope();
                self.in_break_context = saved_break;
                self.in_continue_context = saved_continue;
                self.in_function_context = saved_function;
                self.in_generator_function_context = saved_generator;
                self.await_expression_is_valid = saved_await;
                self.in_class_field_initializer = saved_field_init;
                self.in_class_static_init_block = saved_static_init;
                self.allow_super_property_lookup = saved_super;
                self.consume_token(TokenType::CurlyClose);
                return (self.builder.create_static_initializer(self.span_from(start), body), None);
            }
            true
        } else {
            false
        };

        let mut is_async = false;
        let mut is_generator = false;
        let mut is_getter = false;
        let mut is_setter = false;
        let function_start = self.position();

        // Check modifiers
        if self.match_identifier_name() {
            let value = self.token_value(&self.current_token).to_vec();
            if value == utf16!("get") && self.match_property_key_ahead() {
                is_getter = true;
                self.consume();
            } else if value == utf16!("set") && self.match_property_key_ahead() {
                is_setter = true;
                self.consume();
            } else if value == utf16!("async") && self.match_property_key_ahead() && !self.current_token.trivia_has_line_terminator {
                is_async = true;
                self.consume();
            }
        }

        if self.match_token(TokenType::Asterisk) {
            is_generator = true;
            self.consume();
        }

        // Parse key
        let (key, key_value, _is_proto) = self.parse_property_key();

        // Static properties may not be named "prototype".
        if is_static && key_value.as_deref() == Some(utf16!("prototype")) {
            self.syntax_error("Classes may not have a static property named 'prototype'");
        }

        // Method
        if self.match_token(TokenType::ParenOpen) {
            let ctor_name = utf16!("constructor");
            let is_constructor = !is_static
                && !is_getter && !is_setter
                && key_value.as_deref() == Some(ctor_name);

            if is_constructor {
                if is_getter || is_setter {
                    self.syntax_error("Class constructor may not be an accessor");
                }
                if is_generator {
                    self.syntax_error("Class constructor may not be a generator");
                }
                if is_async {
                    self.syntax_error("Class constructor may not be async");
                }
            }

            let func = self.parse_method_definition(is_async, is_generator, is_getter, is_setter, is_constructor, function_start);
            let method_kind = if is_getter {
                ClassMethod::GETTER
            } else if is_setter {
                ClassMethod::SETTER
            } else {
                ClassMethod::METHOD
            };

            let constructor = if is_constructor { Some(func) } else { None };

            return (self.builder.create_class_method(self.span_from(start), key, func, method_kind, is_static), constructor);
        }

        // Field named "constructor" is not allowed.
        if !is_static && key_value.as_deref() == Some(utf16!("constructor")) {
            self.syntax_error("Class cannot have field named 'constructor'");
        }

        // Field
        let init = if self.match_token(TokenType::Equals) {
            self.consume();
            self.parse_expression(2, Associativity::Right, ForbiddenTokens::none())
        } else {
            NULL_HANDLE
        };

        self.consume_or_insert_semicolon();
        (self.builder.create_class_field(self.span_from(start), key, init, is_static), None)
    }

    // === Function body ===

    /// Parse a function body. The caller must have already opened the function
    /// scope (via open_function_scope) before parsing formal parameters, so that
    /// default parameter expressions can resolve identifiers in the function scope.
    /// Returns (body, has_use_strict, parsing_insights).
    pub(crate) fn parse_function_body(&mut self, is_async: bool, is_generator: bool, param_info: &[(Vec<u16>, NodeHandle, bool, bool)], is_simple_parameters: bool) -> (NodeHandle, bool, FunctionParsingInsights) {
        let start = self.position();
        let body = self.builder.create_function_body(self.span_from(start));
        self.consume_token(TokenType::CurlyOpen);

        self.scope_collector.set_scope_node(body);
        self.scope_collector.set_function_parameters(param_info);

        let in_function_before = self.in_function_context;
        let in_generator_before = self.in_generator_function_context;
        let await_before = self.await_expression_is_valid;
        let old_labels = std::mem::take(&mut self.labels_in_scope);
        self.in_function_context = true;
        self.in_generator_function_context = is_generator;
        self.await_expression_is_valid = is_async;

        let has_use_strict = self.parse_directive(body);
        if has_use_strict || self.strict_mode {
            self.builder.scope_node_set_strict_mode(body);
        }

        let strict_before = self.strict_mode;
        if has_use_strict {
            self.strict_mode = true;
            if !is_simple_parameters {
                self.syntax_error("Illegal 'use strict' directive in function with non-simple parameter list");
            }
        }

        self.parse_statement_list(body, false);

        self.strict_mode = strict_before;
        self.in_function_context = in_function_before;
        self.in_generator_function_context = in_generator_before;
        self.await_expression_is_valid = await_before;
        self.labels_in_scope = old_labels;

        // Read scope insights before closing the scope.
        let insights = FunctionParsingInsights {
            uses_this: self.scope_collector.uses_this(),
            uses_this_from_environment: self.scope_collector.uses_this_from_environment(),
            contains_direct_call_to_eval: self.scope_collector.contains_direct_call_to_eval(),
        };

        self.builder.scope_node_shrink_to_fit(body);
        self.scope_collector.close_scope();
        self.consume_token(TokenType::CurlyClose);

        (body, has_use_strict, insights)
    }

    // === Formal parameters ===

    /// Returns (params_node, function_length, param_info).
    /// param_info entries: (name, identifier_handle, is_rest, is_from_pattern).
    pub(crate) fn parse_formal_parameters(&mut self) -> (NodeHandle, i32, Vec<(Vec<u16>, NodeHandle, bool, bool)>, bool) {
        self.consume_token(TokenType::ParenOpen);
        let result = self.parse_formal_parameters_without_parens();
        self.consume_token(TokenType::ParenClose);
        result
    }

    /// Parse formal parameters assuming the opening '(' has already been consumed.
    /// Does NOT consume the closing ')'.
    pub(crate) fn parse_formal_parameters_without_parens(&mut self) -> (NodeHandle, i32, Vec<(Vec<u16>, NodeHandle, bool, bool)>, bool) {
        if self.match_token(TokenType::ParenClose) {
            return (self.builder.create_function_parameters_empty(), 0, Vec::new(), true);
        }

        let mut bindings = Vec::new();
        let mut default_values = Vec::new();
        let mut is_rest = Vec::new();
        let mut is_pattern = Vec::new();
        let mut function_length: i32 = 0;
        let mut has_seen_default = false;
        let mut has_seen_rest = false;
        let mut param_info: Vec<(Vec<u16>, NodeHandle, bool, bool)> = Vec::new();

        loop {
            let param_start = self.position();
            let rest = if self.match_token(TokenType::TripleDot) {
                self.consume();
                true
            } else {
                false
            };

            let (binding, is_pat) = if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.check_identifier_name_for_assignment_validity(&value, false);
                // Check for duplicate parameter names.
                for (prev_name, _, _, _) in &param_info {
                    if *prev_name == value {
                        if self.strict_mode {
                            let name_str = String::from_utf16_lossy(&value);
                            self.syntax_error(&format!("Duplicate parameter '{}' not allowed in strict mode", name_str));
                        } else if has_seen_default {
                            let name_str = String::from_utf16_lossy(&value);
                            self.syntax_error(&format!("Duplicate parameter '{}' not allowed in function with default parameter", name_str));
                        } else if has_seen_rest {
                            let name_str = String::from_utf16_lossy(&value);
                            self.syntax_error(&format!("Duplicate parameter '{}' not allowed in function with rest parameter", name_str));
                        }
                        break;
                    }
                }
                let id = self.builder.create_identifier(self.span_from(param_start), &value);
                param_info.push((value, id, rest, false));
                (id, false)
            } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                let pat = self.parse_binding_pattern();
                for (n, id) in std::mem::take(&mut self.pattern_bound_names) {
                    param_info.push((n, id, rest, true));
                }
                (pat, true)
            } else {
                self.expected("parameter name");
                self.consume();
                (self.builder.create_identifier(self.span_from(param_start), &[]), false)
            };

            let default_value = if !rest && self.match_token(TokenType::Equals) {
                self.consume();
                has_seen_default = true;
                self.parse_expression(2, Associativity::Right, ForbiddenTokens::with_in())
            } else {
                NULL_HANDLE
            };

            if !rest && !has_seen_default && default_value == NULL_HANDLE {
                function_length += 1;
            }

            bindings.push(binding);
            default_values.push(default_value);
            is_rest.push(rest);
            is_pattern.push(is_pat);
            if rest {
                has_seen_rest = true;
            }

            if rest || !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();

            if self.match_token(TokenType::ParenClose) {
                break;
            }
        }

        let is_simple = !has_seen_default && !has_seen_rest && !is_pattern.iter().any(|&p| p);
        let params = self.builder.create_function_parameters(&bindings, &default_values, &is_rest, &is_pattern);
        (params, function_length, param_info, is_simple)
    }

    // === Binding pattern ===

    pub(crate) fn parse_binding_pattern(&mut self) -> NodeHandle {
        let is_object = self.match_token(TokenType::CurlyOpen);
        let is_array = self.match_token(TokenType::BracketOpen);
        if !is_object && !is_array {
            return NULL_HANDLE;
        }
        self.consume();

        let kind: u8 = if is_object { BindingPattern::OBJECT } else { BindingPattern::ARRAY };
        let pattern = self.builder.create_binding_pattern(kind);
        let closing_token = if is_object { TokenType::CurlyClose } else { TokenType::BracketClose };

        while !self.match_token(closing_token) && !self.done() {
            // Array elision: bare comma
            if !is_object && self.match_token(TokenType::Comma) {
                self.consume();
                self.builder.binding_pattern_append_entry(pattern, NULL_HANDLE, BindingEntryName::EMPTY, NULL_HANDLE, BindingEntryAlias::EMPTY, NULL_HANDLE, false);
                continue;
            }

            let is_rest = if self.match_token(TokenType::TripleDot) {
                self.consume();
                true
            } else {
                false
            };

            let mut name = NULL_HANDLE;
            let mut name_type: u8 = BindingEntryName::EMPTY;
            let mut alias = NULL_HANDLE;
            let mut alias_type: u8 = BindingEntryAlias::EMPTY;

            if is_object {
                if self.allow_member_expressions && is_rest {
                    // Destructuring assignment: rest target can be MemberExpression or Identifier
                    let _expr_start = self.position();
                    let expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none().forbid(&[TokenType::Equals]));
                    if self.builder.is_member_expression(expression) {
                        alias = expression;
                        alias_type = BindingEntryAlias::MEMBER_EXPRESSION;
                    } else if self.builder.is_identifier(expression) {
                        name = expression;
                        name_type = BindingEntryName::IDENTIFIER;
                    } else {
                        self.syntax_error("Invalid destructuring assignment target");
                        break;
                    }
                } else {
                    // Object binding pattern entry
                    let mut needs_alias = false;
                    let mut entry_name_value: Vec<u16> = Vec::new();

                    if self.match_identifier_name() || self.match_token(TokenType::StringLiteral) || self.match_token(TokenType::NumericLiteral) || self.match_token(TokenType::BigIntLiteral) {
                        let entry_start = self.position();

                        if self.match_token(TokenType::StringLiteral) || self.match_token(TokenType::NumericLiteral) {
                            needs_alias = true;
                        }

                        if self.match_token(TokenType::StringLiteral) {
                            let tok = self.consume();
                            let (value, _has_octal) = self.parse_string_value(&tok);
                            name = self.builder.create_identifier(self.span_from(entry_start), &value);
                            name_type = BindingEntryName::IDENTIFIER;
                        } else if self.match_token(TokenType::BigIntLiteral) {
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            // Strip trailing 'n' for the identifier name
                            let name_value = if value.last() == Some(&(b'n' as u16)) {
                                &value[..value.len() - 1]
                            } else {
                                &value
                            };
                            name = self.builder.create_identifier(self.span_from(entry_start), name_value);
                            name_type = BindingEntryName::IDENTIFIER;
                        } else {
                            // Identifier name or numeric literal
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            entry_name_value = value.clone();
                            name = self.builder.create_identifier(self.span_from(entry_start), &value);
                            name_type = BindingEntryName::IDENTIFIER;
                        }
                    } else if self.match_token(TokenType::BracketOpen) {
                        // Computed property name [expr]
                        self.consume();
                        name = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                        name_type = BindingEntryName::EXPRESSION;
                        self.consume_token(TokenType::BracketClose);
                    } else {
                        self.expected("identifier or computed property name");
                        break;
                    }

                    // Check for alias after ':'
                    if !is_rest && self.match_token(TokenType::Colon) {
                        self.consume();
                        if self.allow_member_expressions {
                            // Destructuring assignment: alias can be expression
                            let expr_start = self.position();
                            let expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none().forbid(&[TokenType::Equals]));
                            if self.builder.is_object_expression(expression) || self.builder.is_array_expression(expression) {
                                alias = self.synthesize_binding_pattern(expr_start);
                                alias_type = BindingEntryAlias::BINDING_PATTERN;
                            } else if self.builder.is_member_expression(expression) {
                                alias = expression;
                                alias_type = BindingEntryAlias::MEMBER_EXPRESSION;
                            } else if self.builder.is_identifier(expression) {
                                alias = expression;
                                alias_type = BindingEntryAlias::IDENTIFIER;
                            } else {
                                self.syntax_error("Invalid destructuring assignment target");
                                break;
                            }
                        } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                            // Nested binding pattern
                            alias = self.parse_binding_pattern();
                            alias_type = BindingEntryAlias::BINDING_PATTERN;
                        } else if self.match_identifier_name() {
                            let alias_start = self.position();
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            alias = self.builder.create_identifier(self.span_from(alias_start), &value);
                            self.pattern_bound_names.push((value, alias));
                            alias_type = BindingEntryAlias::IDENTIFIER;
                        } else {
                            self.expected("identifier or binding pattern");
                            break;
                        }
                    } else if needs_alias {
                        self.expected("alias for string or numeric literal name");
                        break;
                    } else if !entry_name_value.is_empty() {
                        // Shorthand: name is the bound identifier.
                        self.pattern_bound_names.push((entry_name_value, name));
                    }
                }
            } else {
                // Array binding pattern entry (name is always Empty)
                if self.allow_member_expressions {
                    // Destructuring assignment: element can be expression
                    let expr_start = self.position();
                    let expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none().forbid(&[TokenType::Equals]));
                    if self.builder.is_object_expression(expression) || self.builder.is_array_expression(expression) {
                        alias = self.synthesize_binding_pattern(expr_start);
                        alias_type = BindingEntryAlias::BINDING_PATTERN;
                    } else if self.builder.is_member_expression(expression) {
                        alias = expression;
                        alias_type = BindingEntryAlias::MEMBER_EXPRESSION;
                    } else if self.builder.is_identifier(expression) {
                        alias = expression;
                        alias_type = BindingEntryAlias::IDENTIFIER;
                    } else {
                        self.syntax_error("Invalid destructuring assignment target");
                        break;
                    }
                } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                    alias = self.parse_binding_pattern();
                    alias_type = BindingEntryAlias::BINDING_PATTERN;
                } else if self.match_identifier_name() {
                    let alias_start = self.position();
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    alias = self.builder.create_identifier(self.span_from(alias_start), &value);
                    self.pattern_bound_names.push((value, alias));
                    alias_type = BindingEntryAlias::IDENTIFIER;
                } else {
                    self.expected("identifier or binding pattern");
                    break;
                }
            }

            // Optional initializer
            let initializer = if self.match_token(TokenType::Equals) {
                self.consume();
                self.parse_expression(2, Associativity::Right, ForbiddenTokens::none())
            } else {
                NULL_HANDLE
            };

            self.builder.binding_pattern_append_entry(pattern, name, name_type, alias, alias_type, initializer, is_rest);

            if self.match_token(TokenType::Comma) {
                self.consume();
            } else if is_object && !self.match_token(closing_token) {
                self.consume_token(TokenType::Comma);
            }
        }

        // Consume trailing commas for arrays
        if !is_object {
            while self.match_token(TokenType::Comma) {
                self.consume();
            }
        }

        self.consume_token(closing_token);
        pattern
    }

    // === Import statement ===

    pub(crate) fn parse_import_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Import);

        if self.program_type != ProgramType::Module {
            self.syntax_error("Cannot use 'import' outside a module");
        }

        // import ModuleSpecifier ;
        if self.match_token(TokenType::StringLiteral) {
            let module_specifier = self.consume_module_specifier();
            let node = self.builder.create_import_statement(
                self.span_from(start), &module_specifier, &[]);
            self.parse_with_clause(node, true);
            self.consume_or_insert_semicolon();
            return node;
        }

        // Representation: (import_name or None for namespace, local_name)
        struct ImportEntryData {
            import_name: Option<Vec<u16>>,
            local_name: Vec<u16>,
        }

        let mut entries: Vec<ImportEntryData> = Vec::new();
        let mut continue_parsing = true;

        // ImportedDefaultBinding
        if self.match_imported_binding() {
            let tok = self.consume();
            let local_name = self.token_value(&tok).to_vec();
            entries.push(ImportEntryData {
                import_name: Some(utf16!("default").to_vec()),
                local_name,
            });
            if self.match_token(TokenType::Comma) {
                self.consume();
            } else {
                continue_parsing = false;
            }
        }

        if continue_parsing {
            if self.match_token(TokenType::Asterisk) {
                // NameSpaceImport: * as ImportedBinding
                self.consume();
                if !self.match_as() {
                    self.expected("'as'");
                }
                self.consume(); // consume 'as'
                if self.match_imported_binding() {
                    let tok = self.consume();
                    let namespace_name = self.token_value(&tok).to_vec();
                    entries.push(ImportEntryData {
                        import_name: None, // namespace
                        local_name: namespace_name,
                    });
                } else {
                    self.expected("identifier");
                }
            } else if self.match_token(TokenType::CurlyOpen) {
                // NamedImports: { ImportSpecifier, ... }
                self.consume();
                while !self.done() && !self.match_token(TokenType::CurlyClose) {
                    if self.match_identifier_name() {
                        let require_as = !self.match_imported_binding();
                        let name_pos = self.position();
                        let tok = self.consume();
                        let name = self.token_value(&tok).to_vec();

                        if self.match_as() {
                            self.consume(); // consume 'as'
                            let alias_tok = self.consume_identifier();
                            let alias = self.token_value(&alias_tok).to_vec();
                            self.check_identifier_name_for_assignment_validity(&alias, false);
                            entries.push(ImportEntryData {
                                import_name: Some(name),
                                local_name: alias,
                            });
                        } else if require_as {
                            self.syntax_error_at_position(
                                &format!("Unexpected reserved word '{}'", String::from_utf16_lossy(&name)),
                                name_pos,
                            );
                        } else {
                            self.check_identifier_name_for_assignment_validity(&name, false);
                            entries.push(ImportEntryData {
                                import_name: Some(name.clone()),
                                local_name: name,
                            });
                        }
                    } else if self.match_token(TokenType::StringLiteral) {
                        // ImportSpecifier: ModuleExportName as ImportedBinding
                        let tok = self.consume();
                        let (name, _) = self.parse_string_value(&tok);

                        if !self.match_as() {
                            self.expected("'as'");
                        }
                        self.consume(); // consume 'as'

                        let alias_tok = self.consume_identifier();
                        let alias = self.token_value(&alias_tok).to_vec();
                        self.check_identifier_name_for_assignment_validity(&alias, false);
                        entries.push(ImportEntryData {
                            import_name: Some(name),
                            local_name: alias,
                        });
                    } else {
                        self.expected("identifier");
                        break;
                    }

                    if !self.match_token(TokenType::Comma) {
                        break;
                    }
                    self.consume();
                }
                self.consume_token(TokenType::CurlyClose);
            } else {
                self.expected("import clauses");
            }
        }

        // 'from' ModuleSpecifier
        if !self.match_from() {
            self.expected("'from'");
        }
        self.consume(); // consume 'from'

        let module_specifier = self.consume_module_specifier();

        // Build FFI entries.
        let ffi_entries: Vec<FFIImportEntry> = entries.iter().map(|e| {
            FFIImportEntry {
                import_name: match &e.import_name {
                    Some(n) => n.as_ptr(),
                    None => std::ptr::null(),
                },
                import_name_len: match &e.import_name {
                    Some(n) => n.len(),
                    None => usize::MAX,
                },
                local_name: e.local_name.as_ptr(),
                local_name_len: e.local_name.len(),
            }
        }).collect();

        let node = self.builder.create_import_statement(
            self.span_from(start), &module_specifier, &ffi_entries);
        self.parse_with_clause(node, true);
        self.consume_or_insert_semicolon();
        node
    }

    // === Export statement ===

    pub(crate) fn parse_export_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Export);

        if self.program_type != ProgramType::Module {
            self.syntax_error("Cannot use 'export' outside a module");
        }

        struct ExportEntryData {
            kind: u8, // 0=Named, 1=ModuleRequestAll, 2=AllButDefault, 3=EmptyNamed
            export_name: Option<Vec<u16>>,
            local_or_import_name: Option<Vec<u16>>,
        }

        let mut entries: Vec<ExportEntryData> = Vec::new();
        let mut expression: NodeHandle = NULL_HANDLE;
        let mut is_default = false;
        let mut from_specifier: Option<Vec<u16>> = None;

        if self.match_token(TokenType::Default) {
            is_default = true;
            self.consume();

            let mut local_name: Option<Vec<u16>> = None;

            // Detect function declaration (with or without name).
            let matches_function = self.match_function_declaration_for_export();

            if matches_function != MatchesFunctionDeclaration::No {
                let has_default_name = matches_function == MatchesFunctionDeclaration::WithoutName;
                let decl = self.parse_function_declaration_for_export(has_default_name);
                self.register_function_declaration_with_scope_collector(decl);
                if !has_default_name {
                    // Function has a name - extract it from the declaration.
                    local_name = Some(self.builder.get_function_name(decl));
                }
                expression = decl;
            } else if self.match_token(TokenType::Class) {
                let next = self.next_token();
                if next.token_type != TokenType::CurlyOpen && next.token_type != TokenType::Extends {
                    // Named class declaration.
                    let decl = self.parse_class_declaration();
                    local_name = Some(self.builder.get_class_name(decl));
                    expression = decl;
                } else {
                    // Unnamed class expression.
                    let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                    self.consume_or_insert_semicolon();
                    expression = expr;
                }
            } else if self.match_expression() {
                let special_case = self.match_token(TokenType::Class)
                    || self.match_token(TokenType::Function)
                    || (self.match_token(TokenType::Async) && {
                        let nt = self.next_token();
                        nt.token_type == TokenType::Function && !nt.trivia_has_line_terminator
                    });
                expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                if !special_case {
                    self.consume_or_insert_semicolon();
                }
            } else {
                self.expected("declaration or assignment expression");
            }

            if local_name.is_none() {
                local_name = Some(utf16!("*default*").to_vec());
            }

            entries.push(ExportEntryData {
                kind: 0, // NamedExport
                export_name: Some(utf16!("default").to_vec()),
                local_or_import_name: local_name,
            });
        } else {
            #[derive(PartialEq)]
            enum FromSpecifier { NotAllowed, Optional, Required }
            let mut check_for_from = FromSpecifier::NotAllowed;

            if self.match_token(TokenType::Asterisk) {
                self.consume();
                if self.match_as() {
                    // * as ModuleExportName
                    self.consume(); // consume 'as'
                    let (exported_name, _) = self.parse_module_export_name();
                    entries.push(ExportEntryData {
                        kind: 1, // ModuleRequestAll
                        export_name: Some(exported_name),
                        local_or_import_name: None,
                    });
                } else {
                    entries.push(ExportEntryData {
                        kind: 2, // ModuleRequestAllButDefault
                        export_name: None,
                        local_or_import_name: None,
                    });
                }
                check_for_from = FromSpecifier::Required;
            } else if self.match_declaration() {
                let declaration = self.parse_declaration();
                // The declaration is the expression; extract export names from it.
                let names = self.builder.get_declaration_export_names(declaration);
                for name in &names {
                    entries.push(ExportEntryData {
                        kind: 0,
                        export_name: Some(name.clone()),
                        local_or_import_name: Some(name.clone()),
                    });
                }
                expression = declaration;
            } else if self.match_token(TokenType::Var) {
                let var_decl = self.parse_variable_declaration(false);
                let names = self.builder.get_declaration_export_names(var_decl);
                for name in &names {
                    entries.push(ExportEntryData {
                        kind: 0,
                        export_name: Some(name.clone()),
                        local_or_import_name: Some(name.clone()),
                    });
                }
                expression = var_decl;
            } else if self.match_token(TokenType::CurlyOpen) {
                self.consume();
                check_for_from = FromSpecifier::Optional;

                while !self.done() && !self.match_token(TokenType::CurlyClose) {
                    let (identifier, was_string) = self.parse_module_export_name();
                    if was_string {
                        // String on LHS requires `from`.
                        check_for_from = FromSpecifier::Required;
                    }

                    if self.match_as() {
                        self.consume(); // consume 'as'
                        let (export_name, _) = self.parse_module_export_name();
                        entries.push(ExportEntryData {
                            kind: 0,
                            export_name: Some(export_name),
                            local_or_import_name: Some(identifier),
                        });
                    } else {
                        entries.push(ExportEntryData {
                            kind: 0,
                            export_name: Some(identifier.clone()),
                            local_or_import_name: Some(identifier),
                        });
                    }

                    if !self.match_token(TokenType::Comma) {
                        break;
                    }
                    self.consume();
                }

                if entries.is_empty() {
                    entries.push(ExportEntryData {
                        kind: 3, // EmptyNamedExport
                        export_name: None,
                        local_or_import_name: None,
                    });
                }

                self.consume_token(TokenType::CurlyClose);
            } else {
                self.syntax_error("Unexpected token 'export'");
            }

            if check_for_from != FromSpecifier::NotAllowed && self.match_from() {
                self.consume(); // consume 'from'
                from_specifier = Some(self.consume_module_specifier());
            } else if check_for_from == FromSpecifier::Required {
                self.expected("'from'");
            }

            if check_for_from != FromSpecifier::NotAllowed {
                self.consume_or_insert_semicolon();
            }
        }

        // Build FFI entries.
        let ffi_entries: Vec<FFIExportEntry> = entries.iter().map(|e| {
            FFIExportEntry {
                kind: e.kind,
                export_name: match &e.export_name {
                    Some(n) => n.as_ptr(),
                    None => std::ptr::null(),
                },
                export_name_len: match &e.export_name {
                    Some(n) => n.len(),
                    None => usize::MAX,
                },
                local_or_import_name: match &e.local_or_import_name {
                    Some(n) => n.as_ptr(),
                    None => std::ptr::null(),
                },
                local_or_import_name_len: match &e.local_or_import_name {
                    Some(n) => n.len(),
                    None => usize::MAX,
                },
            }
        }).collect();

        let node = self.builder.create_export_statement(
            self.span_from(start),
            expression,
            &ffi_entries,
            is_default,
            from_specifier.as_deref(),
        );

        if from_specifier.is_some() {
            self.parse_with_clause(node, false);
        }

        node
    }

    // === Import/Export helpers ===

    /// Check if the current token is a valid ImportedBinding (identifier, yield, or await).
    fn match_imported_binding(&self) -> bool {
        self.match_identifier() || self.match_token(TokenType::Yield) || self.match_token(TokenType::Await)
    }

    /// Check if the current token is the contextual keyword "as".
    fn match_as(&self) -> bool {
        self.match_token(TokenType::Identifier) && self.token_original_value(&self.current_token) == utf16!("as")
    }

    /// Check if the current token is the contextual keyword "from".
    fn match_from(&self) -> bool {
        self.match_token(TokenType::Identifier) && self.token_original_value(&self.current_token) == utf16!("from")
    }

    /// Consume a string literal as a module specifier and return its value.
    fn consume_module_specifier(&mut self) -> Vec<u16> {
        if !self.match_token(TokenType::StringLiteral) {
            self.expected("module specifier (string)");
            return utf16!("!!invalid!!").to_vec();
        }
        let tok = self.consume();
        let (value, _) = self.parse_string_value(&tok);
        value
    }

    /// Parse a ModuleExportName (IdentifierName or StringLiteral).
    /// Returns (name, was_string_literal).
    fn parse_module_export_name(&mut self) -> (Vec<u16>, bool) {
        if self.match_identifier_name() {
            let tok = self.consume();
            (self.token_value(&tok).to_vec(), false)
        } else if self.match_token(TokenType::StringLiteral) {
            let tok = self.consume();
            let (value, _) = self.parse_string_value(&tok);
            (value, true)
        } else {
            self.expected("export specifier (string or identifier)");
            (Vec::new(), false)
        }
    }

    /// Parse a `with { ... }` clause for import/export attributes.
    fn parse_with_clause(&mut self, node: NodeHandle, is_import: bool) {
        if !self.match_token(TokenType::With) {
            return;
        }
        self.consume();
        self.consume_token(TokenType::CurlyOpen);

        while !self.done() && !self.match_token(TokenType::CurlyClose) {
            let key: Vec<u16>;
            if self.match_token(TokenType::StringLiteral) {
                let tok = self.consume();
                let (value, _) = self.parse_string_value(&tok);
                key = value;
            } else if self.match_identifier_name() {
                let tok = self.consume();
                key = self.token_value(&tok).to_vec();
            } else {
                self.expected("identifier or string as attribute key");
                self.consume();
                continue;
            }

            self.consume_token(TokenType::Colon);

            if self.match_token(TokenType::StringLiteral) {
                let tok = self.consume();
                let (value, _) = self.parse_string_value(&tok);
                if is_import {
                    self.builder.import_statement_add_attribute(node, &key, &value);
                } else {
                    self.builder.export_statement_add_attribute(node, &key, &value);
                }
            } else {
                self.expected("string as attribute value");
                self.consume();
            }

            if self.match_token(TokenType::Comma) {
                self.consume();
            } else {
                break;
            }
        }
        self.consume_token(TokenType::CurlyClose);
    }

    /// Detect if the current position matches a function declaration for `export default`.
    fn match_function_declaration_for_export(&mut self) -> MatchesFunctionDeclaration {
        if self.match_token(TokenType::Function) {
            let next = self.next_token();
            if next.token_type == TokenType::Asterisk {
                // function * [name?]
                self.save_state();
                self.consume(); // function
                self.consume(); // *
                let result = if self.match_token(TokenType::ParenOpen) {
                    MatchesFunctionDeclaration::WithoutName
                } else {
                    MatchesFunctionDeclaration::Yes
                };
                self.load_state();
                return result;
            }
            return if next.token_type == TokenType::ParenOpen {
                MatchesFunctionDeclaration::WithoutName
            } else {
                MatchesFunctionDeclaration::Yes
            };
        }

        if self.match_token(TokenType::Async) {
            let next = self.next_token();
            if next.token_type != TokenType::Function || next.trivia_has_line_terminator {
                return MatchesFunctionDeclaration::No;
            }
            // async function [*?] [name?]
            self.save_state();
            self.consume(); // async
            self.consume(); // function
            let has_asterisk = self.match_token(TokenType::Asterisk);
            if has_asterisk {
                self.consume(); // *
            }
            let result = if self.match_token(TokenType::ParenOpen) {
                MatchesFunctionDeclaration::WithoutName
            } else {
                MatchesFunctionDeclaration::Yes
            };
            self.load_state();
            return result;
        }

        MatchesFunctionDeclaration::No
    }

    /// Parse a function declaration for `export default`, optionally with a default export name.
    fn parse_function_declaration_for_export(&mut self, has_default_name: bool) -> NodeHandle {
        if has_default_name {
            self.parse_function_declaration_with_default_export_name()
        } else {
            self.parse_function_declaration()
        }
    }

    /// Parse a function declaration where if the function has no name,
    /// it gets `*default*` as its name (for `export default function() {}`).
    fn parse_function_declaration_with_default_export_name(&mut self) -> NodeHandle {
        self.has_default_export_name = true;
        let result = self.parse_function_declaration();
        self.has_default_export_name = false;
        result
    }
}

#[derive(PartialEq)]
enum MatchesFunctionDeclaration {
    No,
    Yes,
    WithoutName,
}
