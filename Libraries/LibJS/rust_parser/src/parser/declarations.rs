/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Declaration parsing: variables, functions, classes, imports, exports.

use crate::ast_bridge::{NodeHandle, NULL_HANDLE};
use crate::parser::{Associativity, DeclarationKind, ForbiddenTokens, FunctionKind, Parser, ProgramType};
use crate::token::TokenType;

impl<'a> Parser<'a> {
    // === Declarations ===

    pub(crate) fn parse_declaration(&mut self) -> NodeHandle {
        if self.match_token(TokenType::Async) {
            let next = self.next_token();
            if next.token_type == TokenType::Function {
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
                        decl, &[class_name.as_slice()], pos.0, pos.1,
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
            decl, &name, name_id, kind, self.strict_mode, pos.0, pos.1,
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

        loop {
            let decl_start = self.position();
            // Parse target (identifier or binding pattern)
            let (target, is_pattern) = if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
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

        // Register with scope collector.
        if self.scope_collector.has_current_scope() {
            match kind {
                DeclarationKind::Var => {
                    let names: Vec<(&[u16], NodeHandle)> = var_bound_names.iter()
                        .map(|(n, h)| (n.as_slice(), *h))
                        .collect();
                    self.scope_collector.add_var_declaration(decl, &names, start.0, start.1);
                }
                DeclarationKind::Let | DeclarationKind::Const => {
                    let names: Vec<&[u16]> = lexical_bound_names.iter()
                        .map(|n| n.as_slice())
                        .collect();
                    self.scope_collector.add_lexical_declaration(decl, &names, start.0, start.1);
                }
            }
        }

        decl
    }

    // === Function declaration ===

    fn parse_function_declaration(&mut self) -> NodeHandle {
        let start = self.position();

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
        let name = if self.match_identifier() {
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

        let (params, function_length, param_info) = self.parse_formal_parameters();

        // Save function name state before body parsing, which may recursively
        // parse nested function declarations that clobber these fields.
        let saved_fn_name = self.last_function_name.clone();
        let saved_fn_name_id = self.last_function_name_id;
        let saved_fn_kind = self.last_function_kind;

        let body = self.parse_function_body(is_async, is_generator, &param_info);

        // Restore so register_function_declaration_with_scope_collector uses the right values.
        self.last_function_name = saved_fn_name;
        self.last_function_name_id = saved_fn_name_id;
        self.last_function_kind = saved_fn_kind;

        let span = self.span_from(start);
        self.builder.create_function_declaration(
            span, name,
            start.2, self.source_text_end_offset() - start.2,
            body.0, params, function_length, kind as u8,
            self.strict_mode || body.1,
            true, false, false, true,
        )
    }

    // === Function expression ===

    pub(crate) fn parse_function_expression(&mut self) -> NodeHandle {
        let start = self.position();

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

        let (params, function_length, param_info) = self.parse_formal_parameters();

        let body = self.parse_function_body(is_async, is_generator, &param_info);

        let span = self.span_from(start);
        self.builder.create_function_expression(
            span, name,
            start.2, self.source_text_end_offset() - start.2,
            body.0, params, function_length, kind as u8,
            self.strict_mode || body.1, false,
            true, false, false, true,
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

        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            if self.match_token(TokenType::Semicolon) {
                self.consume();
                continue;
            }

            let (element, maybe_ctor) = self.parse_class_element();
            if let Some(ctor) = maybe_ctor {
                constructor_func = ctor;
                // Constructor is NOT included in elements
            } else {
                elements.push(element);
            }
        }

        self.consume_token(TokenType::CurlyClose);
        self.strict_mode = strict_before;

        // Create synthetic constructor if none was declared
        if constructor_func == NULL_HANDLE {
            let ctor_body = self.builder.create_function_body(self.span_from(start));
            if super_class != NULL_HANDLE {
                // Generate: constructor(...args) { return super(...args); }
                let args_name: Vec<u16> = "args".encode_utf16().collect();
                let args_ident = self.builder.create_identifier(self.span_from(start), &args_name);
                let super_call = self.builder.create_synthetic_constructor_super_call(self.span_from(start), args_ident);
                let return_stmt = self.builder.create_return_statement(self.span_from(start), super_call);
                self.builder.scope_node_append(ctor_body, return_stmt);
                let args_binding = self.builder.create_identifier(self.span_from(start), &args_name);
                let ctor_params = self.builder.create_function_parameters(
                    &[args_binding], &[NULL_HANDLE], &[true], &[false],
                );
                constructor_func = self.builder.create_function_expression(
                    self.span_from(start), name,
                    start.2, self.source_text_end_offset() - start.2,
                    ctor_body, ctor_params, 0, FunctionKind::Normal as u8,
                    true, false,
                    true, true, false, false,
                );
            } else {
                let ctor_params = self.builder.create_function_parameters_empty();
                constructor_func = self.builder.create_function_expression(
                    self.span_from(start), name,
                    start.2, self.source_text_end_offset() - start.2,
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
            start.2, self.source_text_end_offset() - start.2,
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
                let in_function_before = self.in_function_context;
                self.in_function_context = true;
                self.parse_statement_list(body, false);
                self.in_function_context = in_function_before;
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
            if value == super::utf16_lit("get") && self.match_property_key_ahead() {
                is_getter = true;
                self.consume();
            } else if value == super::utf16_lit("set") && self.match_property_key_ahead() {
                is_setter = true;
                self.consume();
            } else if value == super::utf16_lit("async") && self.match_property_key_ahead() && !self.current_token.trivia_has_line_terminator {
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

        // Method
        if self.match_token(TokenType::ParenOpen) {
            let func = self.parse_method_definition(is_async, is_generator, is_getter, is_setter, function_start);
            let method_kind = if is_getter {
                1 // Getter
            } else if is_setter {
                2 // Setter
            } else {
                0 // Method
            };

            // Check if this is the constructor
            let ctor_name = super::utf16_lit("constructor");
            let is_constructor = !is_static
                && !is_getter && !is_setter
                && key_value.as_deref() == Some(ctor_name.as_slice());
            let constructor = if is_constructor { Some(func) } else { None };

            return (self.builder.create_class_method(self.span_from(start), key, func, method_kind, is_static), constructor);
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
    pub(crate) fn parse_function_body(&mut self, is_async: bool, is_generator: bool, param_info: &[(Vec<u16>, NodeHandle, bool, bool)]) -> (NodeHandle, bool) {
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
        }

        self.parse_statement_list(body, false);

        self.strict_mode = strict_before;
        self.in_function_context = in_function_before;
        self.in_generator_function_context = in_generator_before;
        self.await_expression_is_valid = await_before;
        self.labels_in_scope = old_labels;

        self.builder.scope_node_shrink_to_fit(body);
        self.scope_collector.close_scope();
        self.consume_token(TokenType::CurlyClose);

        (body, has_use_strict)
    }

    // === Formal parameters ===

    /// Returns (params_node, function_length, param_info).
    /// param_info entries: (name, identifier_handle, is_rest, is_from_pattern).
    pub(crate) fn parse_formal_parameters(&mut self) -> (NodeHandle, i32, Vec<(Vec<u16>, NodeHandle, bool, bool)>) {
        self.consume_token(TokenType::ParenOpen);
        let result = self.parse_formal_parameters_without_parens();
        self.consume_token(TokenType::ParenClose);
        result
    }

    /// Parse formal parameters assuming the opening '(' has already been consumed.
    /// Does NOT consume the closing ')'.
    pub(crate) fn parse_formal_parameters_without_parens(&mut self) -> (NodeHandle, i32, Vec<(Vec<u16>, NodeHandle, bool, bool)>) {
        if self.match_token(TokenType::ParenClose) {
            return (self.builder.create_function_parameters_empty(), 0, Vec::new());
        }

        let mut bindings = Vec::new();
        let mut default_values = Vec::new();
        let mut is_rest = Vec::new();
        let mut is_pattern = Vec::new();
        let mut function_length: i32 = 0;
        let mut has_seen_default = false;
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

            if rest || !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();

            if self.match_token(TokenType::ParenClose) {
                break;
            }
        }

        let params = self.builder.create_function_parameters(&bindings, &default_values, &is_rest, &is_pattern);
        (params, function_length, param_info)
    }

    // === Binding pattern ===

    pub(crate) fn parse_binding_pattern(&mut self) -> NodeHandle {
        let is_object = self.match_token(TokenType::CurlyOpen);
        let is_array = self.match_token(TokenType::BracketOpen);
        if !is_object && !is_array {
            return NULL_HANDLE;
        }
        self.consume();

        let kind: u8 = if is_object { 1 } else { 0 };
        let pattern = self.builder.create_binding_pattern(kind);
        let closing_token = if is_object { TokenType::CurlyClose } else { TokenType::BracketClose };

        while !self.match_token(closing_token) && !self.done() {
            // Array elision: bare comma
            if !is_object && self.match_token(TokenType::Comma) {
                self.consume();
                self.builder.binding_pattern_append_entry(pattern, NULL_HANDLE, 0, NULL_HANDLE, 0, NULL_HANDLE, false);
                continue;
            }

            let is_rest = if self.match_token(TokenType::TripleDot) {
                self.consume();
                true
            } else {
                false
            };

            let mut name = NULL_HANDLE;
            let mut name_type: u8 = 0; // Empty
            let mut alias = NULL_HANDLE;
            let mut alias_type: u8 = 0; // Empty

            if is_object {
                if self.allow_member_expressions && is_rest {
                    // Destructuring assignment: rest target can be MemberExpression or Identifier
                    let expr_start = self.position();
                    let expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none().forbid(&[TokenType::Equals]));
                    if self.builder.is_member_expression(expression) {
                        alias = expression;
                        alias_type = 3; // MemberExpression
                    } else if self.builder.is_identifier(expression) {
                        name = expression;
                        name_type = 1; // Identifier
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
                            let value = self.parse_string_value(&tok);
                            name = self.builder.create_identifier(self.span_from(entry_start), &value);
                            name_type = 1;
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
                            name_type = 1;
                        } else {
                            // Identifier name or numeric literal
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            entry_name_value = value.clone();
                            name = self.builder.create_identifier(self.span_from(entry_start), &value);
                            name_type = 1;
                        }
                    } else if self.match_token(TokenType::BracketOpen) {
                        // Computed property name [expr]
                        self.consume();
                        name = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                        name_type = 2; // Expression
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
                                alias_type = 2; // BindingPattern
                            } else if self.builder.is_member_expression(expression) {
                                alias = expression;
                                alias_type = 3; // MemberExpression
                            } else if self.builder.is_identifier(expression) {
                                alias = expression;
                                alias_type = 1; // Identifier
                            } else {
                                self.syntax_error("Invalid destructuring assignment target");
                                break;
                            }
                        } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                            // Nested binding pattern
                            alias = self.parse_binding_pattern();
                            alias_type = 2; // BindingPattern
                        } else if self.match_identifier_name() {
                            let alias_start = self.position();
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            alias = self.builder.create_identifier(self.span_from(alias_start), &value);
                            self.pattern_bound_names.push((value, alias));
                            alias_type = 1; // Identifier
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
                        alias_type = 2; // BindingPattern
                    } else if self.builder.is_member_expression(expression) {
                        alias = expression;
                        alias_type = 3; // MemberExpression
                    } else if self.builder.is_identifier(expression) {
                        alias = expression;
                        alias_type = 1; // Identifier
                    } else {
                        self.syntax_error("Invalid destructuring assignment target");
                        break;
                    }
                } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                    alias = self.parse_binding_pattern();
                    alias_type = 2; // BindingPattern
                } else if self.match_identifier_name() {
                    let alias_start = self.position();
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    alias = self.builder.create_identifier(self.span_from(alias_start), &value);
                    self.pattern_bound_names.push((value, alias));
                    alias_type = 1; // Identifier
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

        // TODO: Full import statement parsing
        // For now, skip to semicolon
        while !self.match_token(TokenType::Semicolon) && !self.done() {
            self.consume();
        }
        self.consume_or_insert_semicolon();

        // Return an expression statement as placeholder
        self.builder.create_empty_statement(self.span_from(start))
    }

    // === Export statement ===

    pub(crate) fn parse_export_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Export);

        if self.program_type != ProgramType::Module {
            self.syntax_error("Cannot use 'export' outside a module");
        }

        // Handle export default
        if self.match_token(TokenType::Default) {
            self.consume();
            if self.match_token(TokenType::Function) || (self.match_token(TokenType::Async) && self.next_token().token_type == TokenType::Function) {
                let decl = self.parse_function_declaration();
                return decl;
            }
            if self.match_token(TokenType::Class) {
                let decl = self.parse_class_declaration();
                return decl;
            }
            let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
            self.consume_or_insert_semicolon();
            return self.builder.create_expression_statement(self.span_from(start), expr);
        }

        // Handle other export forms
        if self.match_declaration() {
            return self.parse_declaration();
        }

        if self.match_token(TokenType::Var) {
            return self.parse_variable_declaration(false);
        }

        // TODO: Handle export { ... }, export *, export from
        while !self.match_token(TokenType::Semicolon) && !self.done() {
            self.consume();
        }
        self.consume_or_insert_semicolon();

        self.builder.create_empty_statement(self.span_from(start))
    }
}
