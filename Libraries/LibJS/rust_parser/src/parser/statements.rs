/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Statement parsing: if, for, while, switch, try, etc.
//!
//! Each `parse_*_statement()` method consumes tokens for one statement
//! and returns a `NodeHandle` to the created C++ AST node.
//!
//! ## Labelled statements
//!
//! Labels are tracked in `parser.labels_in_scope` so that `break` and
//! `continue` can be validated. The `try_parse_labelled_statement()`
//! method detects `identifier:` at statement position and wraps the
//! inner statement in a `LabelledStatement` node.
//!
//! ## For loops
//!
//! `parse_for_statement()` handles all three forms: `for(;;)`,
//! `for(x in obj)`, and `for(x of iter)`. It starts by parsing the
//! initializer, then disambiguates based on whether `in` or `of` follows.

use crate::ast_bridge::{NodeHandle, NULL_HANDLE};
use crate::parser::{Associativity, ForbiddenTokens, Parser};
use crate::token::TokenType;

impl<'a> Parser<'a> {
    /// Parse a statement.
    pub(crate) fn parse_statement(&mut self, allow_labelled_function: bool) -> NodeHandle {
        let start = self.position();
        let tt = self.current_token_type();

        match tt {
            TokenType::CurlyOpen => self.parse_block_statement(),
            TokenType::Return => self.parse_return_statement(),
            TokenType::Var => {
                // Scope collector registration happens inside parse_variable_declaration.
                self.parse_variable_declaration(false)
            }
            TokenType::For => self.parse_for_statement(),
            TokenType::If => self.parse_if_statement(),
            TokenType::Throw => self.parse_throw_statement(),
            TokenType::Try => self.parse_try_statement(),
            TokenType::Break => self.parse_break_statement(),
            TokenType::Continue => self.parse_continue_statement(),
            TokenType::Switch => self.parse_switch_statement(),
            TokenType::Do => self.parse_do_while_statement(),
            TokenType::While => self.parse_while_statement(),
            TokenType::With => {
                if self.strict_mode {
                    self.syntax_error("'with' statement not allowed in strict mode");
                }
                self.parse_with_statement()
            }
            TokenType::Debugger => self.parse_debugger_statement(),
            TokenType::Semicolon => {
                self.consume();
                self.builder.create_empty_statement(self.span_from(start))
            }
            TokenType::Slash | TokenType::SlashEquals => {
                // Re-lex as regex
                let tok = self.lexer.force_slash_as_regex();
                self.current_token = tok;
                self.parse_expression_statement()
            }
            _ => {
                if self.match_identifier_name() {
                    if let Some(labelled) = self.try_parse_labelled_statement(allow_labelled_function) {
                        return labelled;
                    }
                }
                if self.match_expression() {
                    self.parse_expression_statement()
                } else {
                    self.expected("statement");
                    self.consume();
                    self.builder.create_empty_statement(self.span_from(start))
                }
            }
        }
    }

    // === Block statement ===

    pub(crate) fn parse_block_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::CurlyOpen);
        let block = self.builder.create_block_statement(self.span_from(start));

        self.scope_collector.open_block_scope(block);

        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            if self.match_declaration() {
                let decl = self.parse_declaration();
                self.builder.scope_node_append(block, decl);
            } else {
                let stmt = self.parse_statement(true);
                self.builder.scope_node_append(block, stmt);
            }
        }

        self.builder.scope_node_shrink_to_fit(block);
        self.scope_collector.close_scope();
        self.consume_token(TokenType::CurlyClose);
        block
    }

    // === Expression statement ===

    fn parse_expression_statement(&mut self) -> NodeHandle {
        let start = self.position();

        // Validation: function/class declarations not allowed in single-statement context
        if self.match_token(TokenType::Function) || self.match_token(TokenType::Class) {
            let name = self.current_token.token_type.name();
            self.syntax_error(&format!("{} declaration not allowed in single-statement context", name));
        }

        let expr = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_or_insert_semicolon();
        self.builder.create_expression_statement(self.span_from(start), expr)
    }

    // === Return statement ===

    fn parse_return_statement(&mut self) -> NodeHandle {
        let start = self.position();
        if !self.in_function_context {
            self.syntax_error("'return' not allowed outside of a function");
        }
        self.consume_token(TokenType::Return);

        // Check for value
        if self.current_token.trivia_has_line_terminator
            || self.match_token(TokenType::Semicolon)
            || self.match_token(TokenType::CurlyClose)
            || self.done()
        {
            self.consume_or_insert_semicolon();
            return self.builder.create_return_statement(self.span_from(start), NULL_HANDLE);
        }

        let argument = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_or_insert_semicolon();
        self.builder.create_return_statement(self.span_from(start), argument)
    }

    // === Throw statement ===

    fn parse_throw_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Throw);

        if self.current_token.trivia_has_line_terminator {
            self.syntax_error("No line break is allowed between 'throw' and its expression");
        }

        let argument = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_or_insert_semicolon();
        self.builder.create_throw_statement(self.span_from(start), argument)
    }

    // === Break statement ===

    fn parse_break_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Break);

        let label = if self.match_token(TokenType::Semicolon) {
            self.consume();
            None
        } else if !self.current_token.trivia_has_line_terminator
            && !self.match_token(TokenType::CurlyClose)
            && !self.done()
            && self.match_identifier()
        {
            let tok = self.consume();
            let label_value = self.token_value(&tok).to_vec();

            if !self.labels_in_scope.contains_key(&label_value) {
                self.syntax_error("Label not found");
            }

            self.consume_or_insert_semicolon();
            Some(label_value)
        } else {
            self.consume_or_insert_semicolon();
            None
        };

        if label.is_none() && !self.in_break_context {
            self.syntax_error("Unlabeled 'break' not allowed outside of a loop or switch statement");
        }

        self.builder.create_break_statement(self.span_from(start), label.as_deref())
    }

    // === Continue statement ===

    fn parse_continue_statement(&mut self) -> NodeHandle {
        let start = self.position();
        if !self.in_continue_context {
            self.syntax_error("'continue' not allow outside of a loop");
        }
        self.consume_token(TokenType::Continue);

        let label = if self.match_token(TokenType::Semicolon) {
            None
        } else if !self.current_token.trivia_has_line_terminator
            && !self.match_token(TokenType::CurlyClose)
            && !self.done()
            && self.match_identifier()
        {
            let label_line = self.current_token.line_number;
            let label_col = self.current_token.line_column;
            let tok = self.consume();
            let label_value = self.token_value(&tok).to_vec();

            if let Some(entry) = self.labels_in_scope.get_mut(&label_value) {
                *entry = Some((label_line, label_col));
            } else {
                self.syntax_error("Label not found or invalid");
            }

            Some(label_value)
        } else {
            None
        };

        self.consume_or_insert_semicolon();

        self.builder.create_continue_statement(self.span_from(start), label.as_deref())
    }

    // === Debugger statement ===

    fn parse_debugger_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Debugger);
        self.consume_or_insert_semicolon();
        self.builder.create_debugger_statement(self.span_from(start))
    }

    // === If statement ===

    fn parse_if_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::If);
        self.consume_token(TokenType::ParenOpen);
        let predicate = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);

        let consequent = self.parse_statement(false);

        let alternate = if self.match_token(TokenType::Else) {
            self.consume();
            self.parse_statement(false)
        } else {
            NULL_HANDLE
        };

        self.builder.create_if_statement(self.span_from(start), predicate, consequent, alternate)
    }

    // === While statement ===

    fn parse_while_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::While);
        self.consume_token(TokenType::ParenOpen);
        let test = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);

        let break_before = self.in_break_context;
        let continue_before = self.in_continue_context;
        self.in_break_context = true;
        self.in_continue_context = true;

        let body = self.parse_statement(false);

        self.in_break_context = break_before;
        self.in_continue_context = continue_before;

        self.builder.create_while_statement(self.span_from(start), test, body)
    }

    // === Do-while statement ===

    fn parse_do_while_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Do);

        let break_before = self.in_break_context;
        let continue_before = self.in_continue_context;
        self.in_break_context = true;
        self.in_continue_context = true;

        let body = self.parse_statement(false);

        self.in_break_context = break_before;
        self.in_continue_context = continue_before;

        self.consume_token(TokenType::While);
        self.consume_token(TokenType::ParenOpen);
        let test = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);
        self.consume_or_insert_semicolon();

        self.builder.create_do_while_statement(self.span_from(start), test, body)
    }

    // === For statement ===

    /// Parse a for statement, which has four possible forms:
    /// - `for (init; test; update) body`     (standard for)
    /// - `for (lhs in rhs) body`              (for-in)
    /// - `for (lhs of rhs) body`              (for-of)
    /// - `for await (lhs of rhs) body`        (for-await-of)
    ///
    /// The disambiguation happens after parsing the init expression:
    /// if `in` or `of` follows, it's a for-in/of loop. Otherwise, it's
    /// a standard for loop.
    fn parse_for_statement(&mut self) -> NodeHandle {
        let start = self.position();
        // For loops get their own block scope for `let`/`const` declarations.
        let loop_scope_node = self.builder.create_block_statement(self.span_from(start));
        self.scope_collector.open_for_loop_scope(loop_scope_node);

        self.consume_token(TokenType::For);

        let is_await = if self.match_token(TokenType::Await) {
            if !self.await_expression_is_valid {
                self.syntax_error("for-await-of not allowed outside of async context");
            }
            self.consume();
            true
        } else {
            false
        };

        self.consume_token(TokenType::ParenOpen);

        // Check for for-in/of vs standard for
        if self.match_token(TokenType::Semicolon) && !is_await {
            // for (;;)
            self.consume();
            let result = self.parse_standard_for_loop(start, NULL_HANDLE);
            self.scope_collector.close_scope();
            return result;
        }

        // Parse the init expression/declaration.
        let init_start = self.position();
        let is_var_init = self.match_token(TokenType::Var);
        let is_declaration = is_var_init || self.match_token(TokenType::Let) || self.match_token(TokenType::Const);
        let init = if is_declaration {
            self.parse_variable_declaration(true)
        } else {
            // Forbid `in` as a binary operator here so that `for (x in y)`
            // is parsed as for-in rather than `for ((x in y); ...)`
            let forbidden = ForbiddenTokens::with_in();
            self.parse_expression(0, Associativity::Right, forbidden)
        };

        // Check for in/of
        if self.match_token(TokenType::In) && !is_await {
            self.consume();
            let rhs = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
            self.consume_token(TokenType::ParenClose);

            let break_before = self.in_break_context;
            let continue_before = self.in_continue_context;
            self.in_break_context = true;
            self.in_continue_context = true;
            let body = self.parse_statement(false);
            self.in_break_context = break_before;
            self.in_continue_context = continue_before;

            self.scope_collector.close_scope();
            // If the LHS was an array/object expression (not a declaration),
            // it's actually a destructuring assignment pattern like:
            //   for ({ a, b } in obj) ...
            // Re-parse it as a binding pattern.
            if !is_declaration && (self.builder.is_array_expression(init) || self.builder.is_object_expression(init)) {
                let pattern = self.synthesize_binding_pattern(init_start);
                for (name, id) in std::mem::take(&mut self.pattern_bound_names) {
                    self.scope_collector.register_identifier(id, &name, None);
                }
                if pattern != NULL_HANDLE {
                    return self.builder.create_for_in_statement_with_pattern(self.span_from(start), pattern, rhs, body);
                }
            }
            return self.builder.create_for_in_statement(self.span_from(start), init, rhs, body);
        }

        if self.match_identifier_name() {
            let value = self.token_value(&self.current_token).to_vec();
            if value == super::utf16_lit("of") {
                self.consume();
                let rhs = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::ParenClose);

                let break_before = self.in_break_context;
                let continue_before = self.in_continue_context;
                self.in_break_context = true;
                self.in_continue_context = true;
                let body = self.parse_statement(false);
                self.in_break_context = break_before;
                self.in_continue_context = continue_before;

                self.scope_collector.close_scope();
                // Synthesize binding pattern for destructuring assignment in for-of LHS.
                if !is_declaration && (self.builder.is_array_expression(init) || self.builder.is_object_expression(init)) {
                    let pattern = self.synthesize_binding_pattern(init_start);
                    for (name, id) in std::mem::take(&mut self.pattern_bound_names) {
                        self.scope_collector.register_identifier(id, &name, None);
                    }
                    if pattern != NULL_HANDLE {
                        if is_await {
                            return self.builder.create_for_await_of_statement_with_pattern(self.span_from(start), pattern, rhs, body);
                        }
                        return self.builder.create_for_of_statement_with_pattern(self.span_from(start), pattern, rhs, body);
                    }
                }
                if is_await {
                    return self.builder.create_for_await_of_statement(self.span_from(start), init, rhs, body);
                }
                return self.builder.create_for_of_statement(self.span_from(start), init, rhs, body);
            }
        }

        // Standard for loop
        self.consume_token(TokenType::Semicolon);
        let result = self.parse_standard_for_loop(start, init);
        self.scope_collector.close_scope();
        result
    }

    fn parse_standard_for_loop(&mut self, start: (u32, u32, u32), init: NodeHandle) -> NodeHandle {
        let test = if self.match_token(TokenType::Semicolon) {
            NULL_HANDLE
        } else {
            self.parse_expression(0, Associativity::Right, ForbiddenTokens::none())
        };
        self.consume_token(TokenType::Semicolon);

        let update = if self.match_token(TokenType::ParenClose) {
            NULL_HANDLE
        } else {
            self.parse_expression(0, Associativity::Right, ForbiddenTokens::none())
        };
        self.consume_token(TokenType::ParenClose);

        let break_before = self.in_break_context;
        let continue_before = self.in_continue_context;
        self.in_break_context = true;
        self.in_continue_context = true;

        let body = self.parse_statement(false);

        self.in_break_context = break_before;
        self.in_continue_context = continue_before;

        self.builder.create_for_statement(self.span_from(start), init, test, update, body)
    }

    // === With statement ===

    fn parse_with_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::With);
        self.consume_token(TokenType::ParenOpen);
        let object = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);
        let block = self.builder.create_block_statement(self.span_from(start));
        self.scope_collector.open_with_scope(block);
        let body = self.parse_statement(false);
        self.builder.scope_node_append(block, body);
        self.scope_collector.close_scope();
        self.builder.create_with_statement(self.span_from(start), object, block)
    }

    // === Switch statement ===

    fn parse_switch_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Switch);
        self.consume_token(TokenType::ParenOpen);
        let discriminant = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);

        let switch_stmt = self.builder.create_switch_statement(self.span_from(start), discriminant);

        self.consume_token(TokenType::CurlyOpen);

        self.scope_collector.open_block_scope(switch_stmt);

        let break_before = self.in_break_context;
        self.in_break_context = true;

        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            let case = self.parse_switch_case();
            self.builder.switch_statement_add_case(switch_stmt, case);
        }

        self.in_break_context = break_before;

        self.scope_collector.close_scope();
        self.consume_token(TokenType::CurlyClose);

        switch_stmt
    }

    fn parse_switch_case(&mut self) -> NodeHandle {
        let start = self.position();
        let test = if self.match_token(TokenType::Case) {
            self.consume();
            self.parse_expression(0, Associativity::Right, ForbiddenTokens::none())
        } else if self.match_token(TokenType::Default) {
            self.consume();
            NULL_HANDLE
        } else {
            self.expected("'case' or 'default'");
            NULL_HANDLE
        };

        self.consume_token(TokenType::Colon);

        let case = self.builder.create_switch_case(self.span_from(start), test);

        while !self.match_token(TokenType::CurlyClose)
            && !self.match_token(TokenType::Case)
            && !self.match_token(TokenType::Default)
            && !self.done()
        {
            if self.match_declaration() {
                let decl = self.parse_declaration();
                self.builder.switch_case_append(case, decl);
            } else {
                let stmt = self.parse_statement(true);
                self.builder.switch_case_append(case, stmt);
            }
        }

        case
    }

    // === Try statement ===

    fn parse_try_statement(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Try);

        let block = self.parse_block_statement();

        let handler = if self.match_token(TokenType::Catch) {
            self.parse_catch_clause()
        } else {
            NULL_HANDLE
        };

        let finalizer = if self.match_token(TokenType::Finally) {
            self.consume();
            self.parse_block_statement()
        } else {
            NULL_HANDLE
        };

        if handler == NULL_HANDLE && finalizer == NULL_HANDLE {
            self.syntax_error("try statement must have a catch or finally clause");
        }

        self.builder.create_try_statement(self.span_from(start), block, handler, finalizer)
    }

    fn parse_catch_clause(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Catch);

        self.scope_collector.open_catch_scope();

        if self.match_token(TokenType::ParenOpen) {
            self.consume();
            if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                let pattern = self.parse_binding_pattern();
                let bound = std::mem::take(&mut self.pattern_bound_names);
                let names: Vec<&[u16]> = bound.iter().map(|(n, _)| n.as_slice()).collect();
                self.scope_collector.add_catch_parameter_pattern(&names);
                for (name, id) in &bound {
                    self.scope_collector.register_identifier(*id, name, None);
                }
                self.consume_token(TokenType::ParenClose);
                let body = self.parse_block_statement();
                self.scope_collector.close_scope();
                return self.builder.create_catch_clause_with_pattern(self.span_from(start), pattern, body);
            }
            let param = if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                let id = self.builder.create_identifier(self.span_from(start), &value);
                self.scope_collector.add_catch_parameter_identifier(&value, id);
                self.scope_collector.register_identifier(id, &value, None);
                id
            } else {
                self.expected("catch parameter");
                NULL_HANDLE
            };
            self.consume_token(TokenType::ParenClose);
            let body = self.parse_block_statement();
            self.scope_collector.close_scope();
            self.builder.create_catch_clause(self.span_from(start), param, body)
        } else {
            let body = self.parse_block_statement();
            self.scope_collector.close_scope();
            self.builder.create_catch_clause(self.span_from(start), NULL_HANDLE, body)
        }
    }

    // === Labelled statement ===

    fn try_parse_labelled_statement(&mut self, allow_labelled_function: bool) -> Option<NodeHandle> {
        let start = self.position();

        // Quick check: identifier followed by colon
        if !self.match_identifier_name() {
            return None;
        }

        self.save_state();
        let tok = self.consume();
        let label = self.token_value(&tok).to_vec();

        if !self.match_token(TokenType::Colon) {
            self.load_state();
            return None;
        }
        self.discard_saved_state();
        self.consume(); // consume :

        if self.labels_in_scope.contains_key(&label) {
            self.syntax_error("Label has already been declared");
        }

        self.labels_in_scope.insert(label.clone(), None);

        let break_before = self.in_break_context;
        self.in_break_context = true;

        // Check if body is an iteration statement (possibly through nested labels).
        let body_starts_iteration = self.match_iteration_start();
        self.last_inner_label_is_iteration = false;
        let body = self.parse_statement(allow_labelled_function);

        // If this label is NOT on an iteration statement and a `continue`
        // referenced it, that's a syntax error.
        let is_iteration = body_starts_iteration || self.last_inner_label_is_iteration;
        if !is_iteration {
            if let Some(Some((line, col))) = self.labels_in_scope.get(&label) {
                self.syntax_error_at(
                    "labelled continue statement cannot use non iterating statement",
                    *line, *col);
            }
        }

        self.labels_in_scope.remove(&label);
        self.in_break_context = break_before;
        // Propagate iteration info for nested labels.
        self.last_inner_label_is_iteration = is_iteration;

        Some(self.builder.create_labelled_statement(self.span_from(start), &label, body))
    }
}
