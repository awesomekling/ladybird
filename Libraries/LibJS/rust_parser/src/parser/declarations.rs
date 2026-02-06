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
                self.register_var_scoped_declaration(decl);
                return decl;
            }
        }

        match self.current_token_type() {
            TokenType::Function => {
                let decl = self.parse_function_declaration();
                self.register_var_scoped_declaration(decl);
                decl
            }
            TokenType::Class => {
                let decl = self.parse_class_declaration();
                self.register_lexical_declaration(decl);
                decl
            }
            TokenType::Let | TokenType::Const => {
                let decl = self.parse_variable_declaration(false);
                self.register_lexical_declaration(decl);
                decl
            }
            _ => {
                self.expected("declaration");
                self.consume();
                self.builder.create_empty_statement(self.span_from(self.position()))
            }
        }
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

        loop {
            let decl_start = self.position();
            // Parse target (identifier or binding pattern)
            let target = if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.builder.create_identifier(self.span_from(decl_start), &value)
            } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                // TODO: Parse binding pattern
                self.parse_expression(2, Associativity::Right, ForbiddenTokens::none())
            } else {
                self.expected("variable name");
                self.consume();
                self.builder.create_identifier(self.span_from(decl_start), &[])
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

            declarators.push(self.builder.create_variable_declarator(self.span_from(decl_start), target, init));

            if !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();
        }

        if !is_for_loop {
            self.consume_or_insert_semicolon();
        }

        self.builder.create_variable_declaration(self.span_from(start), kind as u8, &declarators)
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
            self.builder.create_identifier(self.span_from(start), &value)
        } else {
            NULL_HANDLE
        };

        let (params, function_length) = self.parse_formal_parameters();

        let body = self.parse_function_body(is_async, is_generator);

        let span = self.span_from(start);
        self.builder.create_function_declaration(
            span, name,
            start.2, self.position().2 - start.2,
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
        let name = if self.match_identifier() {
            let tok = self.consume();
            let value = self.token_value(&tok).to_vec();
            self.builder.create_identifier(self.span_from(start), &value)
        } else {
            NULL_HANDLE
        };

        let (params, function_length) = self.parse_formal_parameters();

        let body = self.parse_function_body(is_async, is_generator);

        let span = self.span_from(start);
        self.builder.create_function_expression(
            span, name,
            start.2, self.position().2 - start.2,
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
                self.builder.create_identifier(self.span_from(start), &value)
            } else if expect_name {
                self.expected("class name");
                NULL_HANDLE
            } else {
                NULL_HANDLE
            }
        } else {
            NULL_HANDLE
        };

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
            // TODO: For classes with super_class, generate `constructor(...args) { return super(...args); }`
            let ctor_params = self.builder.create_function_parameters_empty();
            constructor_func = self.builder.create_function_expression(
                self.span_from(start), name,
                start.2, self.position().2 - start.2,
                ctor_body, ctor_params, 0, FunctionKind::Normal as u8,
                true, false,
                true, true, false, false,
            );
        }

        self.builder.create_class_expression(
            self.span_from(start), name,
            start.2, self.position().2 - start.2,
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
        let (key, key_value) = self.parse_property_key();

        // Method
        if self.match_token(TokenType::ParenOpen) {
            let func = self.parse_method_definition(is_async, is_generator, is_getter, is_setter);
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

    pub(crate) fn parse_function_body(&mut self, is_async: bool, is_generator: bool) -> (NodeHandle, bool) {
        let start = self.position();
        let body = self.builder.create_function_body(self.span_from(start));
        self.consume_token(TokenType::CurlyOpen);

        self.push_scope(body, true);

        let in_function_before = self.in_function_context;
        let in_generator_before = self.in_generator_function_context;
        let await_before = self.await_expression_is_valid;
        self.in_function_context = true;
        self.in_generator_function_context = is_generator;
        self.await_expression_is_valid = is_async;

        let has_use_strict = self.parse_directive(body);
        if has_use_strict {
            self.builder.scope_node_set_strict_mode(body);
        }

        self.parse_statement_list(body, false);

        self.in_function_context = in_function_before;
        self.in_generator_function_context = in_generator_before;
        self.await_expression_is_valid = await_before;

        self.builder.scope_node_shrink_to_fit(body);
        self.pop_scope();
        self.consume_token(TokenType::CurlyClose);

        (body, has_use_strict)
    }

    // === Formal parameters ===

    pub(crate) fn parse_formal_parameters(&mut self) -> (NodeHandle, i32) {
        self.consume_token(TokenType::ParenOpen);

        if self.match_token(TokenType::ParenClose) {
            self.consume();
            return (self.builder.create_function_parameters_empty(), 0);
        }

        let mut bindings = Vec::new();
        let mut default_values = Vec::new();
        let mut is_rest = Vec::new();
        let mut function_length: i32 = 0;
        let mut has_seen_default = false;

        loop {
            let param_start = self.position();
            let rest = if self.match_token(TokenType::TripleDot) {
                self.consume();
                true
            } else {
                false
            };

            let binding = if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.builder.create_identifier(self.span_from(param_start), &value)
            } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                self.parse_expression(2, Associativity::Right, ForbiddenTokens::with_in())
            } else {
                self.expected("parameter name");
                self.consume();
                self.builder.create_identifier(self.span_from(param_start), &[])
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

            if rest || !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();

            if self.match_token(TokenType::ParenClose) {
                break;
            }
        }

        self.consume_token(TokenType::ParenClose);

        let params = self.builder.create_function_parameters(&bindings, &default_values, &is_rest);
        (params, function_length)
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
