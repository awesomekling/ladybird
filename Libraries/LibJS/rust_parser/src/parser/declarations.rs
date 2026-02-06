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
                return self.parse_function_declaration();
            }
        }

        match self.current_token_type() {
            TokenType::Function => self.parse_function_declaration(),
            TokenType::Class => self.parse_class_declaration(),
            TokenType::Let | TokenType::Const => {
                let decl = self.parse_variable_declaration(false);
                // TODO: Register in scope
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

        // Parse parameters (simplified - just skip to matching paren)
        let params = self.builder.create_function_parameters_empty();
        self.consume_token(TokenType::ParenOpen);
        let mut depth = 1;
        while depth > 0 && !self.done() {
            match self.current_token_type() {
                TokenType::ParenOpen => depth += 1,
                TokenType::ParenClose => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                self.consume();
            }
        }
        self.consume_token(TokenType::ParenClose);

        // Parse body
        let body = self.builder.create_function_body(self.span_from(start));
        self.consume_token(TokenType::CurlyOpen);

        let in_function_before = self.in_function_context;
        let in_generator_before = self.in_generator_function_context;
        let await_before = self.await_expression_is_valid;
        self.in_function_context = true;
        self.in_generator_function_context = is_generator;
        self.await_expression_is_valid = is_async;

        // Parse directive prologue
        let has_use_strict = self.parse_directive(body);
        if has_use_strict {
            self.builder.scope_node_set_strict_mode(body);
        }

        self.parse_statement_list(body, false);

        self.in_function_context = in_function_before;
        self.in_generator_function_context = in_generator_before;
        self.await_expression_is_valid = await_before;

        self.consume_token(TokenType::CurlyClose);

        let span = self.span_from(start);
        self.builder.create_function_declaration(
            span, name,
            start.2, self.position().2 - start.2,
            body, params, 0, kind as u8,
            self.strict_mode || has_use_strict,
            false, false, false, false,
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

        // Parameters (simplified)
        let params = self.builder.create_function_parameters_empty();
        self.consume_token(TokenType::ParenOpen);
        let mut depth = 1;
        while depth > 0 && !self.done() {
            match self.current_token_type() {
                TokenType::ParenOpen => depth += 1,
                TokenType::ParenClose => depth -= 1,
                _ => {}
            }
            if depth > 0 {
                self.consume();
            }
        }
        self.consume_token(TokenType::ParenClose);

        // Body
        let body = self.builder.create_function_body(self.span_from(start));
        self.consume_token(TokenType::CurlyOpen);

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

        self.consume_token(TokenType::CurlyClose);

        let span = self.span_from(start);
        self.builder.create_function_expression(
            span, name,
            start.2, self.position().2 - start.2,
            body, params, 0, kind as u8,
            self.strict_mode || has_use_strict, false,
            false, false, false, false,
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

        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            if self.match_token(TokenType::Semicolon) {
                self.consume();
                continue;
            }

            let element = self.parse_class_element();
            elements.push(element);
        }

        self.consume_token(TokenType::CurlyClose);
        self.strict_mode = strict_before;

        self.builder.create_class_expression(
            self.span_from(start), name,
            start.2, self.position().2 - start.2,
            NULL_HANDLE, super_class,
            &elements,
        )
    }

    pub(crate) fn parse_class_declaration(&mut self) -> NodeHandle {
        let start = self.position();
        let class_expr = self.parse_class_expression(true);
        self.builder.create_class_declaration(self.span_from(start), class_expr)
    }

    fn parse_class_element(&mut self) -> NodeHandle {
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
                return self.builder.create_static_initializer(self.span_from(start), body);
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
        let (key, _key_value) = self.parse_property_key();

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
            return self.builder.create_class_method(self.span_from(start), key, func, method_kind, is_static);
        }

        // Field
        let init = if self.match_token(TokenType::Equals) {
            self.consume();
            self.parse_expression(2, Associativity::Right, ForbiddenTokens::none())
        } else {
            NULL_HANDLE
        };

        self.consume_or_insert_semicolon();
        self.builder.create_class_field(self.span_from(start), key, init, is_static)
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
