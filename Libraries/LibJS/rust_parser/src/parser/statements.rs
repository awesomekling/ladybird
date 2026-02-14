/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Statement parsing: if, for, while, switch, try, etc.

use std::rc::Rc;

use crate::ast::*;
use crate::parser::{Associativity, ForbiddenTokens, Parser, Position};
use crate::token::TokenType;

/// Used in parse_for_statement to track whether the init is a
/// declaration or expression, since they have different Rust types.
enum ForInit {
    Declaration(Stmt),
    Expression(Expr),
}

impl<'a> Parser<'a> {
    pub(crate) fn parse_statement(&mut self, allow_labelled_function: bool) -> Stmt {
        let start = self.position();
        let tt = self.current_token_type();

        match tt {
            TokenType::CurlyOpen => self.parse_block_statement(),
            TokenType::Return => self.parse_return_statement(),
            TokenType::Var => self.parse_variable_declaration(false),
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
                if self.flags.strict_mode {
                    self.syntax_error("'with' statement not allowed in strict mode");
                }
                self.parse_with_statement()
            }
            TokenType::Debugger => self.parse_debugger_statement(),
            TokenType::Semicolon => {
                self.consume();
                self.stmt(start, Statement::Empty)
            }
            TokenType::Slash | TokenType::SlashEquals => {
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
                    self.stmt(start, Statement::Empty)
                }
            }
        }
    }

    // === Block statement ===

    pub(crate) fn parse_block_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::CurlyOpen);

        // Open block scope (scope_data set after children are collected).
        self.scope_collector.open_block_scope(None);

        let mut children = Vec::new();

        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            if self.match_declaration() {
                children.push(self.parse_declaration());
            } else {
                children.push(self.parse_statement(true));
            }
        }

        self.consume_token(TokenType::CurlyClose);
        let scope = ScopeData::shared_with_children(children);
        self.scope_collector.set_scope_node(scope.clone());
        self.scope_collector.close_scope();
        self.stmt(start, Statement::Block(scope))
    }

    // === Expression statement ===

    fn parse_expression_statement(&mut self) -> Stmt {
        let start = self.position();

        if self.match_token(TokenType::Function) || self.match_token(TokenType::Class) {
            let name = self.current_token.token_type.name();
            self.syntax_error(&format!("{} declaration not allowed in single-statement context", name));
        }

        let expr = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_or_insert_semicolon();
        self.stmt(start, Statement::Expression(Box::new(expr)))
    }

    // === Return statement ===

    fn parse_return_statement(&mut self) -> Stmt {
        let start = self.position();
        if !self.flags.in_function_context {
            self.syntax_error("'return' not allowed outside of a function");
        }
        self.consume_token(TokenType::Return);

        if self.current_token.trivia_has_line_terminator
            || self.match_token(TokenType::Semicolon)
            || self.match_token(TokenType::CurlyClose)
            || self.done()
        {
            self.consume_or_insert_semicolon();
            return self.stmt(start, Statement::Return(None));
        }

        let argument = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_or_insert_semicolon();
        self.stmt(start, Statement::Return(Some(Box::new(argument))))
    }

    // === Throw statement ===

    fn parse_throw_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::Throw);

        if self.current_token.trivia_has_line_terminator {
            self.syntax_error("No line break is allowed between 'throw' and its expression");
        }

        let argument = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_or_insert_semicolon();
        self.stmt(start, Statement::Throw(Box::new(argument)))
    }

    // === Break statement ===

    fn parse_break_statement(&mut self) -> Stmt {
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

        if label.is_none() && !self.flags.in_break_context {
            self.syntax_error("Unlabeled 'break' not allowed outside of a loop or switch statement");
        }

        self.stmt(start, Statement::Break { target_label: label })
    }

    // === Continue statement ===

    fn parse_continue_statement(&mut self) -> Stmt {
        let start = self.position();
        if !self.flags.in_continue_context {
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

        self.stmt(start, Statement::Continue { target_label: label })
    }

    // === Debugger statement ===

    fn parse_debugger_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::Debugger);
        self.consume_or_insert_semicolon();
        self.stmt(start, Statement::Debugger)
    }

    // === If statement ===

    fn parse_if_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::If);
        self.consume_token(TokenType::ParenOpen);
        let predicate = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);

        let consequent = if !self.flags.strict_mode && self.match_token(TokenType::Function) {
            self.parse_function_declaration_as_block_statement()
        } else {
            self.parse_statement(false)
        };

        let alternate = if self.match_token(TokenType::Else) {
            self.consume();
            if !self.flags.strict_mode && self.match_token(TokenType::Function) {
                Some(Box::new(self.parse_function_declaration_as_block_statement()))
            } else {
                Some(Box::new(self.parse_statement(false)))
            }
        } else {
            None
        };

        self.stmt(start, Statement::If {
            predicate: Box::new(predicate),
            consequent: Box::new(consequent),
            alternate,
        })
    }

    /// Annex B: Parse a function declaration as if wrapped in a synthetic block.
    /// See https://tc39.es/ecma262/#sec-functiondeclarations-in-ifstatement-statement-clauses
    fn parse_function_declaration_as_block_statement(&mut self) -> Stmt {
        let start = self.position();
        self.scope_collector.open_block_scope(None);
        let decl = self.parse_function_declaration();
        let scope = ScopeData::shared_with_children(vec![decl]);
        self.scope_collector.set_scope_node(scope.clone());
        self.scope_collector.close_scope();
        self.stmt(start, Statement::Block(scope))
    }

    // === While statement ===

    fn parse_while_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::While);
        self.consume_token(TokenType::ParenOpen);
        let test = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);

        let break_before = self.flags.in_break_context;
        let continue_before = self.flags.in_continue_context;
        self.flags.in_break_context = true;
        self.flags.in_continue_context = true;

        let body = self.parse_statement(false);

        self.flags.in_break_context = break_before;
        self.flags.in_continue_context = continue_before;

        self.stmt(start, Statement::While {
            test: Box::new(test),
            body: Box::new(body),
        })
    }

    // === Do-while statement ===

    fn parse_do_while_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::Do);

        let break_before = self.flags.in_break_context;
        let continue_before = self.flags.in_continue_context;
        self.flags.in_break_context = true;
        self.flags.in_continue_context = true;

        let body = self.parse_statement(false);

        self.flags.in_break_context = break_before;
        self.flags.in_continue_context = continue_before;

        self.consume_token(TokenType::While);
        self.consume_token(TokenType::ParenOpen);
        let test = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);
        self.consume_or_insert_semicolon();

        self.stmt(start, Statement::DoWhile {
            test: Box::new(test),
            body: Box::new(body),
        })
    }

    // === For statement ===

    fn parse_for_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::For);

        // Open for-loop scope (for let/const/using declarations).
        // scope_data set after the for-loop body is parsed.
        self.scope_collector.open_for_loop_scope(None);

        let is_await = if self.match_token(TokenType::Await) {
            if !self.flags.await_expression_is_valid {
                self.syntax_error("for-await-of not allowed outside of async context");
            }
            self.consume();
            true
        } else {
            false
        };

        self.consume_token(TokenType::ParenOpen);

        // for (;;)
        if self.match_token(TokenType::Semicolon) && !is_await {
            self.consume();
            let result = self.parse_standard_for_loop(start, None);
            return self.close_for_loop_scope(start, result);
        }

        let init_start = self.position();
        let is_var_init = self.match_token(TokenType::Var);
        let is_using = self.match_for_using_declaration();
        let is_declaration = is_var_init || is_using || self.match_token(TokenType::Let) || self.match_token(TokenType::Const);
        let init = if is_using {
            ForInit::Declaration(self.parse_using_declaration(true))
        } else if is_declaration {
            ForInit::Declaration(self.parse_variable_declaration(true))
        } else {
            let forbidden = ForbiddenTokens::with_in();
            ForInit::Expression(self.parse_expression(0, Associativity::Right, forbidden))
        };

        // Check for in
        if self.match_token(TokenType::In) && !is_await {
            if is_using {
                self.syntax_error("Using declaration not allowed in for-in loop");
            } else if is_declaration {
                if self.for_loop_declaration_count > 1 {
                    self.syntax_error("Multiple declarations not allowed in for..in/of");
                }
                if self.for_loop_declaration_has_init {
                    if !(self.for_loop_declaration_is_var && self.for_loop_declaration_count == 1 && !self.flags.strict_mode) {
                        self.syntax_error("Variable initializer not allowed in for..in/of");
                    }
                }
            } else if let ForInit::Expression(ref expr) = init {
                if !Self::is_identifier(expr) && !Self::is_member_expression(expr)
                    && !Self::is_call_expression(expr)
                    && !Self::is_object_expression(expr) && !Self::is_array_expression(expr) {
                    self.syntax_error("Invalid left-hand side in for-loop");
                }
            }
            self.consume();
            let rhs = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
            self.consume_token(TokenType::ParenClose);

            let break_before = self.flags.in_break_context;
            let continue_before = self.flags.in_continue_context;
            self.flags.in_break_context = true;
            self.flags.in_continue_context = true;
            let body = self.parse_statement(false);
            self.flags.in_break_context = break_before;
            self.flags.in_continue_context = continue_before;

            let lhs = match init {
                ForInit::Declaration(decl) => ForInOfLhs::Declaration(Box::new(decl)),
                ForInit::Expression(expr) => {
                    if Self::is_array_expression(&expr) || Self::is_object_expression(&expr) {
                        if let Some(pattern) = self.synthesize_binding_pattern(init_start) {
                            for (name, id) in self.pattern_bound_names.drain(..) {
                                self.scope_collector.register_identifier(id, &name, None);
                            }
                            ForInOfLhs::Pattern(pattern)
                        } else {
                            ForInOfLhs::Expression(Box::new(expr))
                        }
                    } else {
                        ForInOfLhs::Expression(Box::new(expr))
                    }
                }
            };
            let result = self.stmt(start, Statement::ForIn {
                lhs,
                rhs: Box::new(rhs),
                body: Box::new(body),
            });
            return self.close_for_loop_scope(start, result);
        }

        // Check for of
        if self.match_identifier_name() {
            let value = self.token_value(&self.current_token).to_vec();
            if value == utf16!("of") {
                if is_declaration {
                    if self.for_loop_declaration_count > 1 {
                        self.syntax_error("Multiple declarations not allowed in for..in/of");
                    }
                    if self.for_loop_declaration_has_init {
                        self.syntax_error("Variable initializer not allowed in for..of");
                    }
                } else if let ForInit::Expression(ref expr) = init {
                    if !Self::is_identifier(expr) && !Self::is_member_expression(expr)
                        && !Self::is_call_expression(expr)
                        && !Self::is_object_expression(expr) && !Self::is_array_expression(expr) {
                        self.syntax_error("Invalid left-hand side in for-loop");
                    }
                }
                self.consume();
                let rhs = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::ParenClose);

                let break_before = self.flags.in_break_context;
                let continue_before = self.flags.in_continue_context;
                self.flags.in_break_context = true;
                self.flags.in_continue_context = true;
                let body = self.parse_statement(false);
                self.flags.in_break_context = break_before;
                self.flags.in_continue_context = continue_before;

                let lhs = match init {
                    ForInit::Declaration(decl) => ForInOfLhs::Declaration(Box::new(decl)),
                    ForInit::Expression(expr) => {
                        if Self::is_array_expression(&expr) || Self::is_object_expression(&expr) {
                            if let Some(pattern) = self.synthesize_binding_pattern(init_start) {
                                for (name, id) in self.pattern_bound_names.drain(..) {
                                    self.scope_collector.register_identifier(id, &name, None);
                                }
                                ForInOfLhs::Pattern(pattern)
                            } else {
                                ForInOfLhs::Expression(Box::new(expr))
                            }
                        } else {
                            ForInOfLhs::Expression(Box::new(expr))
                        }
                    }
                };
                if is_await {
                    let result = self.stmt(start, Statement::ForAwaitOf {
                        lhs,
                        rhs: Box::new(rhs),
                        body: Box::new(body),
                    });
                    return self.close_for_loop_scope(start, result);
                }
                let result = self.stmt(start, Statement::ForOf {
                    lhs,
                    rhs: Box::new(rhs),
                    body: Box::new(body),
                });
                return self.close_for_loop_scope(start, result);
            }
        }

        // Standard for loop
        self.consume_token(TokenType::Semicolon);
        let init_stmt = match init {
            ForInit::Declaration(decl) => Some(Box::new(decl)),
            ForInit::Expression(expr) => {
                let range = expr.range;
                Some(Box::new(Stmt::new(range, Statement::Expression(Box::new(expr)))))
            }
        };
        let result = self.parse_standard_for_loop(start, init_stmt);
        self.close_for_loop_scope(start, result)
    }

    /// Close the for-loop scope and wrap the for-loop statement in a Block
    /// with scope data, matching C++ parser behavior.
    fn close_for_loop_scope(&mut self, start: Position, inner: Stmt) -> Stmt {
        let scope = ScopeData::shared_with_children(vec![inner]);
        self.scope_collector.set_scope_node(scope.clone());
        self.scope_collector.close_scope();
        self.stmt(start, Statement::Block(scope))
    }

    fn parse_standard_for_loop(&mut self, start: Position, init: Option<Box<Stmt>>) -> Stmt {
        let test = if self.match_token(TokenType::Semicolon) {
            None
        } else {
            Some(Box::new(self.parse_expression(0, Associativity::Right, ForbiddenTokens::none())))
        };
        self.consume_token(TokenType::Semicolon);

        let update = if self.match_token(TokenType::ParenClose) {
            None
        } else {
            Some(Box::new(self.parse_expression(0, Associativity::Right, ForbiddenTokens::none())))
        };
        self.consume_token(TokenType::ParenClose);

        let break_before = self.flags.in_break_context;
        let continue_before = self.flags.in_continue_context;
        self.flags.in_break_context = true;
        self.flags.in_continue_context = true;

        let body = self.parse_statement(false);

        self.flags.in_break_context = break_before;
        self.flags.in_continue_context = continue_before;

        self.stmt(start, Statement::For {
            init,
            test,
            update,
            body: Box::new(body),
        })
    }

    // === With statement ===

    fn parse_with_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::With);
        self.consume_token(TokenType::ParenOpen);
        let object = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);
        self.scope_collector.open_with_scope(None);
        let body = self.parse_statement(false);
        self.scope_collector.close_scope();
        self.stmt(start, Statement::With {
            object: Box::new(object),
            body: Box::new(body),
        })
    }

    // === Switch statement ===

    fn parse_switch_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::Switch);
        self.consume_token(TokenType::ParenOpen);
        let discriminant = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
        self.consume_token(TokenType::ParenClose);

        self.consume_token(TokenType::CurlyOpen);

        // Open block scope for the switch body (all cases share one scope).
        self.scope_collector.open_block_scope(None);

        let break_before = self.flags.in_break_context;
        self.flags.in_break_context = true;

        let mut cases = Vec::new();
        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            cases.push(self.parse_switch_case());
        }

        self.flags.in_break_context = break_before;

        self.consume_token(TokenType::CurlyClose);

        let scope = ScopeData::new_shared();
        self.scope_collector.set_scope_node(scope.clone());
        self.scope_collector.close_scope();

        self.stmt(start, Statement::Switch(SwitchStatementData {
            scope,
            discriminant: Box::new(discriminant),
            cases,
        }))
    }

    fn parse_switch_case(&mut self) -> SwitchCase {
        let start = self.position();
        let test = if self.match_token(TokenType::Case) {
            self.consume();
            Some(self.parse_expression(0, Associativity::Right, ForbiddenTokens::none()))
        } else if self.match_token(TokenType::Default) {
            self.consume();
            None
        } else {
            self.expected("'case' or 'default'");
            None
        };

        self.consume_token(TokenType::Colon);

        let mut children = Vec::new();
        while !self.match_token(TokenType::CurlyClose)
            && !self.match_token(TokenType::Case)
            && !self.match_token(TokenType::Default)
            && !self.done()
        {
            if self.match_declaration() {
                children.push(self.parse_declaration());
            } else {
                children.push(self.parse_statement(true));
            }
        }

        SwitchCase {
            range: self.range_from(start),
            scope: ScopeData::shared_with_children(children),
            test,
        }
    }

    // === Try statement ===

    fn parse_try_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::Try);

        let block = self.parse_block_statement();

        let handler = if self.match_token(TokenType::Catch) {
            Some(self.parse_catch_clause())
        } else {
            None
        };

        let finalizer = if self.match_token(TokenType::Finally) {
            self.consume();
            Some(Box::new(self.parse_block_statement()))
        } else {
            None
        };

        if handler.is_none() && finalizer.is_none() {
            self.syntax_error("try statement must have a catch or finally clause");
        }

        self.stmt(start, Statement::Try(TryStatementData {
            block: Box::new(block),
            handler,
            finalizer,
        }))
    }

    fn parse_catch_clause(&mut self) -> CatchClause {
        let start = self.position();
        self.consume_token(TokenType::Catch);

        // Open catch scope (wraps parameter + body).
        self.scope_collector.open_catch_scope();

        let parameter = if self.match_token(TokenType::ParenOpen) {
            self.consume();
            let param = if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                let pattern = self.parse_binding_pattern();
                let bound_names: Vec<&[u16]> = self.pattern_bound_names.iter().map(|(n, _)| n.as_slice()).collect();
                self.scope_collector.add_catch_parameter_pattern(&bound_names);
                CatchParameter::BindingPattern(pattern)
            } else if self.match_identifier() {
                let param_start = self.position();
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.check_identifier_name_for_assignment_validity(&value, false);
                let id = Rc::new(Identifier::new(self.range_from(param_start), value.clone()));
                self.scope_collector.add_catch_parameter_identifier(&value, id.clone());
                CatchParameter::Identifier(id)
            } else {
                self.expected("catch parameter");
                CatchParameter::None
            };
            self.consume_token(TokenType::ParenClose);
            param
        } else {
            CatchParameter::None
        };

        let body = self.parse_block_statement();

        self.scope_collector.close_scope();

        CatchClause {
            range: self.range_from(start),
            parameter,
            body: Box::new(body),
        }
    }

    // === Labelled statement ===

    fn try_parse_labelled_statement(&mut self, allow_labelled_function: bool) -> Option<Stmt> {
        let start = self.position();

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

        if self.flags.strict_mode && label == utf16!("let") {
            self.syntax_error("Strict mode reserved word 'let' is not allowed in label");
        }

        if self.labels_in_scope.contains_key(&label) {
            self.syntax_error("Label has already been declared");
        }

        if self.match_token(TokenType::Function) {
            if !allow_labelled_function || self.flags.strict_mode {
                self.syntax_error("Not allowed to declare a function here");
            }
            let next = self.next_token();
            if next.token_type == TokenType::Asterisk {
                self.syntax_error("Generator functions cannot be defined in labelled statements");
            }
        }
        if self.match_token(TokenType::Async) {
            let next = self.next_token();
            if next.token_type == TokenType::Function && !next.trivia_has_line_terminator {
                self.syntax_error("Async functions cannot be defined in labelled statements");
            }
        }

        self.labels_in_scope.insert(label.clone(), None);

        let break_before = self.flags.in_break_context;
        self.flags.in_break_context = true;

        let body_starts_iteration = self.match_iteration_start();
        self.last_inner_label_is_iteration = false;
        let body = self.parse_statement(allow_labelled_function);

        let is_iteration = body_starts_iteration || self.last_inner_label_is_iteration;
        if !is_iteration {
            if let Some(Some((line, col))) = self.labels_in_scope.get(&label) {
                self.syntax_error_at(
                    "labelled continue statement cannot use non iterating statement",
                    *line, *col);
            }
        }

        self.labels_in_scope.remove(&label);
        self.flags.in_break_context = break_before;
        self.last_inner_label_is_iteration = is_iteration;

        Some(self.stmt(start, Statement::Labelled {
            label,
            item: Box::new(body),
        }))
    }

    fn match_for_using_declaration(&mut self) -> bool {
        if !self.match_token(TokenType::Identifier) {
            return false;
        }
        if self.token_value(&self.current_token) != utf16!("using") {
            return false;
        }
        let next = self.next_token();
        if next.trivia_has_line_terminator {
            return false;
        }
        if next.token_type == TokenType::Identifier {
            let next_val = self.token_original_value(&next);
            if next_val == utf16!("of") {
                return false;
            }
        }
        next.token_type.is_identifier_name()
    }
}
