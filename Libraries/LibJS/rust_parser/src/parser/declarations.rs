/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Declaration parsing: variables, functions, classes, imports, exports.

use std::rc::Rc;

use crate::ast::*;
use crate::parser::{Associativity, DeclarationKind, ForbiddenTokens, FunctionKind, Parser, Position, ProgramType};
use crate::token::TokenType;

/// Extract an Identifier from an Expression::Identifier node.
fn expr_into_identifier(expr: Expr) -> Rc<Identifier> {
    match expr.inner {
        Expression::Identifier(id) => id,
        _ => unreachable!("expected Identifier expression"),
    }
}

/// Extract bound names from a declaration for export statements.
fn get_declaration_export_names(stmt: &Stmt) -> Vec<Vec<u16>> {
    match &stmt.inner {
        Statement::VariableDeclaration { declarations, .. } => {
            let mut names = Vec::new();
            for decl in declarations {
                collect_declarator_names(&decl.target, &mut names);
            }
            names
        }
        Statement::UsingDeclaration { declarations } => {
            let mut names = Vec::new();
            for decl in declarations {
                if let VariableDeclaratorTarget::Identifier(id) = &decl.target {
                    names.push(id.name.clone());
                }
            }
            names
        }
        Statement::FunctionDeclaration(func) => {
            if let Some(ref name) = func.name {
                vec![name.name.clone()]
            } else {
                Vec::new()
            }
        }
        Statement::ClassDeclaration(class) => {
            if let Some(ref name) = class.name {
                vec![name.name.clone()]
            } else {
                Vec::new()
            }
        }
        _ => Vec::new(),
    }
}

fn collect_declarator_names(target: &VariableDeclaratorTarget, names: &mut Vec<Vec<u16>>) {
    match target {
        VariableDeclaratorTarget::Identifier(id) => names.push(id.name.clone()),
        VariableDeclaratorTarget::BindingPattern(pat) => collect_pattern_names(pat, names),
    }
}

fn collect_pattern_names(pat: &BindingPattern, names: &mut Vec<Vec<u16>>) {
    for entry in &pat.entries {
        match &entry.alias {
            BindingEntryAlias::Identifier(id) => names.push(id.name.clone()),
            BindingEntryAlias::BindingPattern(nested) => collect_pattern_names(nested, names),
            _ => {}
        }
        if matches!(&entry.alias, BindingEntryAlias::Empty) {
            if let BindingEntryName::Identifier(id) = &entry.name {
                names.push(id.name.clone());
            }
        }
    }
}

impl<'a> Parser<'a> {
    // === Declarations ===

    pub(crate) fn parse_declaration(&mut self) -> Stmt {
        if self.match_token(TokenType::Async) {
            let next = self.next_token();
            if next.token_type == TokenType::Function && !next.trivia_has_line_terminator {
                return self.parse_function_declaration();
            }
        }

        match self.current_token_type() {
            TokenType::Function => self.parse_function_declaration(),
            TokenType::Class => self.parse_class_declaration(),
            TokenType::Let | TokenType::Const => self.parse_variable_declaration(false),
            TokenType::Identifier if self.token_value(&self.current_token) == utf16!("using") => {
                self.parse_using_declaration(false)
            }
            _ => {
                self.expected("declaration");
                let start = self.position();
                self.consume();
                self.stmt(start, Statement::Empty)
            }
        }
    }

    // === Variable declaration ===

    pub(crate) fn parse_variable_declaration(&mut self, is_for_loop: bool) -> Stmt {
        let start = self.position();
        let decl_line = self.current_token().line_number;
        let decl_column = self.current_token().line_column;

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

        let mut declarators: Vec<VariableDeclarator> = Vec::new();
        let mut any_init = false;

        loop {
            let decl_start = self.position();

            let target = if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.check_identifier_name_for_assignment_validity(&value, false);
                let id = self.make_identifier(decl_start, value.clone());

                // Register with scope collector.
                if kind == DeclarationKind::Var {
                    self.scope_collector.add_var_declaration(
                        &[(&value, Some(id.clone()))],
                        decl_line, decl_column,
                    );
                } else {
                    self.scope_collector.add_lexical_declaration(
                        &[&value as &[u16]],
                        decl_line, decl_column,
                    );
                    self.scope_collector.register_identifier(
                        id.clone(), &value, Some(kind),
                    );
                }

                VariableDeclaratorTarget::Identifier(id)
            } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                let pat = self.parse_binding_pattern();
                let bound_names = std::mem::take(&mut self.pattern_bound_names);

                // Register bound names with scope collector.
                if kind == DeclarationKind::Var {
                    let entries: Vec<(&[u16], Option<Rc<Identifier>>)> = bound_names.iter()
                        .map(|(n, id)| (n.as_slice(), Some(id.clone())))
                        .collect();
                    self.scope_collector.add_var_declaration(&entries, decl_line, decl_column);
                } else {
                    let refs: Vec<&[u16]> = bound_names.iter().map(|(n, _)| n.as_slice()).collect();
                    self.scope_collector.add_lexical_declaration(&refs, decl_line, decl_column);
                    // Register each binding pattern identifier for scope analysis
                    // so they get is_local() annotations.
                    for (name, id) in &bound_names {
                        self.scope_collector.register_identifier(id.clone(), name, Some(kind));
                    }
                }

                VariableDeclaratorTarget::BindingPattern(pat)
            } else {
                self.expected("variable name");
                self.consume();
                let id = self.make_identifier(decl_start, Vec::new());
                VariableDeclaratorTarget::Identifier(id)
            };

            let init = if self.match_token(TokenType::Equals) {
                self.consume();
                any_init = true;
                let forbidden = if is_for_loop {
                    ForbiddenTokens::with_in()
                } else {
                    ForbiddenTokens::none()
                };
                Some(self.parse_expression(2, Associativity::Right, forbidden))
            } else {
                None
            };

            declarators.push(VariableDeclarator {
                range: self.range_from(decl_start),
                target,
                init,
            });

            if !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();
        }

        if !is_for_loop {
            self.consume_or_insert_semicolon();
        }

        if is_for_loop {
            self.for_loop_declaration_count = declarators.len();
            self.for_loop_declaration_has_init = any_init;
            self.for_loop_declaration_is_var = kind == DeclarationKind::Var;
        }

        self.stmt(start, Statement::VariableDeclaration {
            kind,
            declarations: declarators,
        })
    }

    // === Using declaration ===

    pub(crate) fn parse_using_declaration(&mut self, is_for_loop: bool) -> Stmt {
        let start = self.position();
        let decl_line = self.current_token().line_number;
        let decl_column = self.current_token().line_column;
        self.consume(); // consume 'using'

        let mut declarators: Vec<VariableDeclarator> = Vec::new();

        loop {
            let decl_start = self.position();

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

            let id = self.make_identifier(decl_start, name.clone());

            // Register as lexical declaration.
            self.scope_collector.add_lexical_declaration(&[&name as &[u16]], decl_line, decl_column);
            self.scope_collector.register_identifier(
                id.clone(), &name, Some(DeclarationKind::Const),
            );

            let init = if self.match_token(TokenType::Equals) {
                self.consume();
                if is_for_loop {
                    Some(self.parse_expression(2, Associativity::Right, ForbiddenTokens::with_in()))
                } else {
                    Some(self.parse_expression(2, Associativity::Right, ForbiddenTokens::none()))
                }
            } else if !is_for_loop {
                self.consume_token(TokenType::Equals);
                None
            } else {
                None
            };

            declarators.push(VariableDeclarator {
                range: self.range_from(decl_start),
                target: VariableDeclaratorTarget::Identifier(id),
                init,
            });

            if self.match_token(TokenType::Comma) {
                self.consume();
                continue;
            }
            break;
        }

        if !is_for_loop {
            self.consume_or_insert_semicolon();
        }

        self.stmt(start, Statement::UsingDeclaration {
            declarations: declarators,
        })
    }

    // === Function declaration ===

    fn parse_function_declaration(&mut self) -> Stmt {
        let start = self.position();
        let decl_line = self.current_token().line_number;
        let decl_column = self.current_token().line_column;

        let saved_might_need_arguments = self.flags.function_might_need_arguments_object;
        self.flags.function_might_need_arguments_object = false;

        let is_async = self.eat(TokenType::Async);
        self.consume_token(TokenType::Function);
        let is_generator = self.eat(TokenType::Asterisk);

        let kind = FunctionKind::from_async_generator(is_async, is_generator);

        // Parse function name.
        let (name, fn_name) = if self.has_default_export_name && !self.match_identifier() {
            let default_name = utf16!("*default*").to_vec();
            self.last_function_name = default_name.clone();
            (Some(self.make_identifier(start, default_name.clone())), default_name)
        } else if self.match_identifier() {
            let tok = self.consume();
            let value = self.token_value(&tok).to_vec();
            self.last_function_name = value.clone();
            (Some(self.make_identifier(start, value.clone())), value)
        } else {
            self.last_function_name.clear();
            (None, Vec::new())
        };
        self.last_function_kind = kind;

        // Register function declaration in parent scope (before opening function scope).
        self.scope_collector.add_function_declaration(
            &fn_name, name.clone(),
            kind, self.flags.strict_mode, decl_line, decl_column,
        );

        // Open function scope.
        let fn_name_for_scope = if fn_name.is_empty() { None } else { Some(fn_name.as_slice()) };
        self.scope_collector.open_function_scope(fn_name_for_scope);
        self.scope_collector.set_is_function_declaration();

        let in_generator_before = self.flags.in_generator_function_context;
        let await_before = self.flags.await_expression_is_valid;
        self.flags.in_generator_function_context = is_generator;
        self.flags.await_expression_is_valid = is_async;

        let (params, function_length, param_info, is_simple) = self.parse_formal_parameters();

        // Register function parameters with scope collector.
        self.register_function_params_with_scope(&params, &param_info);

        self.flags.in_generator_function_context = in_generator_before;
        self.flags.await_expression_is_valid = await_before;

        let (body, has_use_strict, _insights) = self.parse_function_body(is_async, is_generator, is_simple);

        // Close function scope.
        self.scope_collector.close_scope();

        if has_use_strict || kind != FunctionKind::Normal {
            let force_strict = has_use_strict;
            if name.is_some() {
                self.check_identifier_name_for_assignment_validity(&fn_name, force_strict);
            }
            self.check_parameters_post_body(&param_info, force_strict, kind);
        }

        let might_need_arguments = self.flags.function_might_need_arguments_object;
        self.flags.function_might_need_arguments_object = saved_might_need_arguments;

        self.stmt(start, Statement::FunctionDeclaration(Box::new(FunctionData {
            name,
            source_text_start: start.offset,
            source_text_end: self.source_text_end_offset(),
            body: Box::new(body),
            parameters: params,
            function_length,
            kind,
            is_strict_mode: self.flags.strict_mode || has_use_strict,
            is_arrow_function: false,
            parsing_insights: FunctionParsingInsights {
                might_need_arguments_object: might_need_arguments,
                ..FunctionParsingInsights::default()
            },
            is_hoisted: false,
        })))
    }

    // === Function expression ===

    pub(crate) fn parse_function_expression(&mut self) -> Expr {
        let start = self.position();

        let saved_might_need_arguments = self.flags.function_might_need_arguments_object;
        self.flags.function_might_need_arguments_object = false;

        let is_async = self.eat(TokenType::Async);
        self.consume_token(TokenType::Function);
        let is_generator = self.eat(TokenType::Asterisk);

        let kind = FunctionKind::from_async_generator(is_async, is_generator);

        // Optional name.
        let mut fn_name_value: Vec<u16> = Vec::new();
        let name = if self.match_identifier() {
            let tok = self.consume();
            fn_name_value = self.token_value(&tok).to_vec();
            Some(self.make_identifier(start, fn_name_value.clone()))
        } else {
            None
        };

        // Open function scope (function expression name is bound within its own scope).
        let fn_name_for_scope = if fn_name_value.is_empty() { None } else { Some(fn_name_value.as_slice()) };
        self.scope_collector.open_function_scope(fn_name_for_scope);

        let in_generator_before = self.flags.in_generator_function_context;
        let await_before = self.flags.await_expression_is_valid;
        self.flags.in_generator_function_context = is_generator;
        self.flags.await_expression_is_valid = is_async;

        let (params, function_length, param_info, is_simple) = self.parse_formal_parameters();

        // Register function parameters with scope collector.
        self.register_function_params_with_scope(&params, &param_info);

        self.flags.in_generator_function_context = in_generator_before;
        self.flags.await_expression_is_valid = await_before;

        let (body, has_use_strict, _insights) = self.parse_function_body(is_async, is_generator, is_simple);

        // Close function scope.
        self.scope_collector.close_scope();

        if has_use_strict || kind != FunctionKind::Normal {
            let force_strict = has_use_strict;
            if name.is_some() {
                self.check_identifier_name_for_assignment_validity(&fn_name_value, force_strict);
            }
            self.check_parameters_post_body(&param_info, force_strict, kind);
        }

        let might_need_arguments = self.flags.function_might_need_arguments_object;
        self.flags.function_might_need_arguments_object = saved_might_need_arguments;

        self.expr(start, Expression::Function(Box::new(FunctionData {
            name,
            source_text_start: start.offset,
            source_text_end: self.source_text_end_offset(),
            body: Box::new(body),
            parameters: params,
            function_length,
            kind,
            is_strict_mode: self.flags.strict_mode || has_use_strict,
            is_arrow_function: false,
            parsing_insights: FunctionParsingInsights {
                might_need_arguments_object: might_need_arguments,
                ..FunctionParsingInsights::default()
            },
            is_hoisted: false,
        })))
    }

    // === Class ===

    pub(crate) fn parse_class_expression(&mut self, expect_name: bool) -> Expr {
        let start = self.position();

        let strict_before = self.flags.strict_mode;
        self.flags.strict_mode = true;

        self.consume_token(TokenType::Class);

        // Optional name.
        let (name_id, name_value) = if expect_name || self.match_identifier() {
            if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.last_class_name = value.clone();
                (Some(self.make_identifier(start, value.clone())), value)
            } else if expect_name {
                self.expected("class name");
                self.last_class_name.clear();
                (None, Vec::new())
            } else {
                self.last_class_name.clear();
                (None, Vec::new())
            }
        } else {
            self.last_class_name.clear();
            (None, Vec::new())
        };

        let saved_class_name = self.last_class_name.clone();

        // Open class declaration scope.
        let class_name_for_scope = if name_value.is_empty() { None } else { Some(name_value.as_slice()) };
        self.scope_collector.open_class_declaration_scope(class_name_for_scope);

        // Optional extends.
        let super_class = if self.match_token(TokenType::Extends) {
            self.consume();
            Some(Box::new(self.parse_expression(0, Associativity::Right, ForbiddenTokens::none())))
        } else {
            None
        };

        // Class body.
        self.consume_token(TokenType::CurlyOpen);
        let mut elements: Vec<Node<ClassElement>> = Vec::new();
        let mut constructor: Option<Expr> = None;

        let saved_class_has_super = self.class_has_super_class;
        self.class_has_super_class = super_class.is_some();
        self.class_scope_depth += 1;

        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            if self.match_token(TokenType::Semicolon) {
                self.consume();
                continue;
            }

            let (element, maybe_ctor) = self.parse_class_element();
            if let Some(ctor) = maybe_ctor {
                if constructor.is_some() {
                    self.syntax_error("Classes may not have more than one constructor");
                }
                constructor = Some(ctor);
            } else if let Some(elem) = element {
                elements.push(elem);
            }
        }

        self.consume_token(TokenType::CurlyClose);
        self.class_scope_depth -= 1;
        self.class_has_super_class = saved_class_has_super;
        self.flags.strict_mode = strict_before;

        // Close class declaration scope.
        self.scope_collector.close_scope();

        // Synthesize default constructor if none was declared.
        if constructor.is_none() {
            constructor = Some(self.synthesize_default_constructor(
                start, &name_value, super_class.is_some(),
            ));
        }

        self.last_class_name = saved_class_name;

        self.expr(start, Expression::Class(Box::new(ClassData {
            name: name_id,
            source_text_start: start.offset,
            source_text_end: self.source_text_end_offset(),
            constructor: constructor.map(Box::new),
            super_class,
            elements,
        })))
    }

    pub(crate) fn parse_class_declaration(&mut self) -> Stmt {
        let start = self.position();
        let class_expr = self.parse_class_expression(true);
        // Convert the class expression into a class declaration by extracting ClassData.
        match class_expr.inner {
            Expression::Class(data) => {
                // Register class name as lexical declaration in the outer scope.
                // The inner class scope (opened/closed inside parse_class_expression)
                // only has the name as IsBound. The outer scope needs IsLexical so
                // FDI creates the binding.
                if let Some(ref name_ident) = data.name {
                    self.scope_collector.add_lexical_declaration(
                        &[&name_ident.name as &[u16]],
                        start.line, start.column,
                    );
                    self.scope_collector.register_identifier(
                        name_ident.clone(),
                        &name_ident.name,
                        Some(DeclarationKind::Let),
                    );
                }
                self.stmt(start, Statement::ClassDeclaration(data))
            }
            _ => unreachable!("parse_class_expression must return Expression::Class"),
        }
    }

    fn synthesize_default_constructor(&mut self, start: Position, class_name: &[u16], has_super: bool) -> Expr {
        let ctor_name = if !class_name.is_empty() {
            Some(self.make_identifier(start, class_name.to_vec()))
        } else {
            None
        };

        // Note: No scope collector calls here. The synthesized constructor is
        // compiled lazily via C++ re-parsing, so Rust scope analysis is irrelevant.

        if has_super {
            // Derived class: constructor(...args) { return super(...args); }
            let args_name: Vec<u16> = "args".encode_utf16().collect();

            let args_ref = Rc::new(Identifier::new(self.range_from(start), args_name.clone()));
            let args_expr = self.expr(start, Expression::Identifier(args_ref));
            let spread_expr = self.expr(start, Expression::Spread(Box::new(args_expr)));

            let super_call = self.expr(start, Expression::SuperCall(SuperCallData {
                arguments: vec![CallArgument { value: spread_expr, is_spread: true }],
                is_synthetic: true,
            }));
            let return_stmt = self.stmt(start, Statement::Return(Some(Box::new(super_call))));
            let body = self.stmt(start, Statement::FunctionBody {
                scope: ScopeData::shared_with_children(vec![return_stmt]),
                in_strict_mode: true,
            });

            let args_binding = Rc::new(Identifier::new(self.range_from(start), args_name));
            let params = vec![FunctionParameter {
                binding: FunctionParameterBinding::Identifier(args_binding),
                default_value: None,
                is_rest: true,
            }];

            self.expr(start, Expression::Function(Box::new(FunctionData {
                name: ctor_name,
                source_text_start: start.offset,
                source_text_end: self.source_text_end_offset(),
                body: Box::new(body),
                parameters: params,
                function_length: 0,
                kind: FunctionKind::Normal,
                is_strict_mode: true,
                is_arrow_function: false,
                parsing_insights: FunctionParsingInsights {
                    uses_this: true,
                    uses_this_from_environment: true,
                    ..FunctionParsingInsights::default()
                },
                is_hoisted: false,
            })))
        } else {
            // Base class: empty constructor() {}
            let body = self.stmt(start, Statement::FunctionBody {
                scope: ScopeData::shared_with_children(Vec::new()),
                in_strict_mode: true,
            });

            self.expr(start, Expression::Function(Box::new(FunctionData {
                name: ctor_name,
                source_text_start: start.offset,
                source_text_end: self.source_text_end_offset(),
                body: Box::new(body),
                parameters: Vec::new(),
                function_length: 0,
                kind: FunctionKind::Normal,
                is_strict_mode: true,
                is_arrow_function: false,
                parsing_insights: FunctionParsingInsights {
                    uses_this: true,
                    uses_this_from_environment: true,
                    ..FunctionParsingInsights::default()
                },
                is_hoisted: false,
            })))
        }
    }

    fn parse_class_element(&mut self) -> (Option<Node<ClassElement>>, Option<Expr>) {
        let start = self.position();
        let is_static = if self.match_token(TokenType::Static) {
            self.consume();
            // static { } block
            if self.match_token(TokenType::CurlyOpen) {
                self.consume(); // consume '{'
                let saved_break = self.flags.in_break_context;
                let saved_continue = self.flags.in_continue_context;
                let saved_function = self.flags.in_function_context;
                let saved_generator = self.flags.in_generator_function_context;
                let saved_await = self.flags.await_expression_is_valid;
                let saved_field_init = self.flags.in_class_field_initializer;
                let saved_static_init = self.flags.in_class_static_init_block;
                let saved_super = self.flags.allow_super_property_lookup;
                self.flags.in_break_context = false;
                self.flags.in_continue_context = false;
                self.flags.in_function_context = false;
                self.flags.in_generator_function_context = false;
                self.flags.await_expression_is_valid = false;
                self.flags.in_class_field_initializer = true;
                self.flags.in_class_static_init_block = true;
                self.flags.allow_super_property_lookup = true;
                self.scope_collector.open_static_init_scope(None);
                let children = self.parse_statement_list(false);
                self.flags.in_break_context = saved_break;
                self.flags.in_continue_context = saved_continue;
                self.flags.in_function_context = saved_function;
                self.flags.in_generator_function_context = saved_generator;
                self.flags.await_expression_is_valid = saved_await;
                self.flags.in_class_field_initializer = saved_field_init;
                self.flags.in_class_static_init_block = saved_static_init;
                self.flags.allow_super_property_lookup = saved_super;
                self.consume_token(TokenType::CurlyClose);
                let scope = ScopeData::shared_with_children(children);
                self.scope_collector.set_scope_node(scope.clone());
                self.scope_collector.close_scope();
                let body = self.stmt(start, Statement::FunctionBody {
                    scope,
                    in_strict_mode: self.flags.strict_mode,
                });
                return (Some(Node::new(self.range_from(start), ClassElement::StaticInitializer {
                    body: Box::new(body),
                })), None);
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

        // Check modifiers.
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

        // Parse key.
        let (key, key_value, _is_proto, _is_computed) = self.parse_property_key();

        if is_static && key_value.as_deref() == Some(utf16!("prototype")) {
            self.syntax_error("Classes may not have a static property named 'prototype'");
        }

        // Method.
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
                ClassMethodKind::Getter
            } else if is_setter {
                ClassMethodKind::Setter
            } else {
                ClassMethodKind::Method
            };

            if is_constructor {
                return (None, Some(func));
            }

            return (Some(Node::new(self.range_from(start), ClassElement::Method {
                key: Box::new(key),
                function: Box::new(func),
                kind: method_kind,
                is_static,
            })), None);
        }

        // Field named "constructor" is not allowed.
        if !is_static && key_value.as_deref() == Some(utf16!("constructor")) {
            self.syntax_error("Class cannot have field named 'constructor'");
        }

        // Field.
        let init = if self.match_token(TokenType::Equals) {
            self.consume();
            let saved_field_init = self.flags.in_class_field_initializer;
            self.flags.in_class_field_initializer = true;
            self.scope_collector.open_class_field_scope(None);
            let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
            self.scope_collector.close_scope();
            self.flags.in_class_field_initializer = saved_field_init;
            Some(Box::new(expr))
        } else {
            None
        };

        self.consume_or_insert_semicolon();
        (Some(Node::new(self.range_from(start), ClassElement::Field {
            key: Box::new(key),
            initializer: init,
            is_static,
        })), None)
    }

    // === Function body ===

    pub(crate) fn parse_function_body(&mut self, is_async: bool, is_generator: bool, is_simple: bool) -> (Stmt, bool, FunctionParsingInsights) {
        let start = self.position();
        self.consume_token(TokenType::CurlyOpen);

        let in_function_before = self.flags.in_function_context;
        let in_generator_before = self.flags.in_generator_function_context;
        let await_before = self.flags.await_expression_is_valid;
        let old_labels = std::mem::take(&mut self.labels_in_scope);
        self.flags.in_function_context = true;
        self.flags.in_generator_function_context = is_generator;
        self.flags.await_expression_is_valid = is_async;

        let (has_use_strict, mut children) = self.parse_directive();
        let body_is_strict = has_use_strict || self.flags.strict_mode;

        let strict_before = self.flags.strict_mode;
        if has_use_strict {
            self.flags.strict_mode = true;
            if !is_simple {
                self.syntax_error("Illegal 'use strict' directive in function with non-simple parameter list");
            }
        }

        children.extend(self.parse_statement_list(false));

        self.flags.strict_mode = strict_before;
        self.flags.in_function_context = in_function_before;
        self.flags.in_generator_function_context = in_generator_before;
        self.flags.await_expression_is_valid = await_before;
        self.labels_in_scope = old_labels;

        let insights = FunctionParsingInsights::default();

        self.consume_token(TokenType::CurlyClose);

        let scope = ScopeData::shared_with_children(children);
        self.scope_collector.set_scope_node(scope.clone());

        let body = self.stmt(start, Statement::FunctionBody {
            scope,
            in_strict_mode: body_is_strict,
        });

        (body, has_use_strict, insights)
    }

    // === Formal parameters ===

    pub(crate) fn parse_formal_parameters(&mut self) -> (Vec<FunctionParameter>, i32, Vec<(Vec<u16>, bool, bool, Option<Rc<Identifier>>)>, bool) {
        self.consume_token(TokenType::ParenOpen);
        let result = self.parse_formal_parameters_without_parens();
        self.consume_token(TokenType::ParenClose);
        result
    }

    pub(crate) fn parse_formal_parameters_without_parens(&mut self) -> (Vec<FunctionParameter>, i32, Vec<(Vec<u16>, bool, bool, Option<Rc<Identifier>>)>, bool) {
        if self.match_token(TokenType::ParenClose) {
            return (Vec::new(), 0, Vec::new(), true);
        }

        let mut params: Vec<FunctionParameter> = Vec::new();
        let mut function_length: i32 = 0;
        let mut has_seen_default = false;
        let mut has_seen_rest = false;
        let mut param_info: Vec<(Vec<u16>, bool, bool, Option<Rc<Identifier>>)> = Vec::new();

        loop {
            let param_start = self.position();
            let rest = self.eat(TokenType::TripleDot);

            let (binding, _is_pat) = if self.match_identifier() {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                self.check_identifier_name_for_assignment_validity(&value, false);
                // Check for duplicate parameter names.
                for (prev_name, _, _, _) in &param_info {
                    if *prev_name == value {
                        if self.flags.strict_mode {
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
                let id = Rc::new(Identifier::new(self.range_from(param_start), value.clone()));
                param_info.push((value, rest, false, Some(id.clone())));
                (FunctionParameterBinding::Identifier(id), false)
            } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                let pat = self.parse_binding_pattern();
                for (n, id) in std::mem::take(&mut self.pattern_bound_names) {
                    param_info.push((n, rest, true, Some(id)));
                }
                (FunctionParameterBinding::BindingPattern(pat), true)
            } else {
                self.expected("parameter name");
                self.consume();
                let id = Rc::new(Identifier::new(self.range_from(param_start), Vec::new()));
                (FunctionParameterBinding::Identifier(id), false)
            };

            let default_value = if !rest && self.match_token(TokenType::Equals) {
                self.consume();
                has_seen_default = true;
                Some(self.parse_expression(2, Associativity::Right, ForbiddenTokens::with_in()))
            } else {
                None
            };

            if !rest && !has_seen_default && default_value.is_none() {
                function_length += 1;
            }

            params.push(FunctionParameter {
                binding,
                default_value,
                is_rest: rest,
            });

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

        let is_simple = !has_seen_default && !has_seen_rest && !params.iter().any(|p| matches!(&p.binding, FunctionParameterBinding::BindingPattern(_)));
        (params, function_length, param_info, is_simple)
    }

    // === Binding pattern ===

    pub(crate) fn parse_binding_pattern(&mut self) -> BindingPattern {
        let is_object = self.match_token(TokenType::CurlyOpen);
        let is_array = self.match_token(TokenType::BracketOpen);
        if !is_object && !is_array {
            return BindingPattern { kind: BindingPatternKind::Object, entries: Vec::new() };
        }
        self.consume();

        let kind = if is_object { BindingPatternKind::Object } else { BindingPatternKind::Array };
        let closing_token = if is_object { TokenType::CurlyClose } else { TokenType::BracketClose };
        let mut entries: Vec<BindingEntry> = Vec::new();

        while !self.match_token(closing_token) && !self.done() {
            // Array elision: bare comma.
            if !is_object && self.match_token(TokenType::Comma) {
                self.consume();
                entries.push(BindingEntry {
                    name: BindingEntryName::Empty,
                    alias: BindingEntryAlias::Empty,
                    initializer: None,
                    is_rest: false,
                });
                continue;
            }

            let is_rest = self.eat(TokenType::TripleDot);

            let mut entry_name = BindingEntryName::Empty;
            let mut entry_alias = BindingEntryAlias::Empty;

            if is_object {
                if self.allow_member_expressions && is_rest {
                    // Destructuring assignment: rest target can be MemberExpression or Identifier.
                    let expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none().forbid(&[TokenType::Equals]));
                    if Self::is_member_expression(&expression) {
                        entry_alias = BindingEntryAlias::MemberExpression(Box::new(expression));
                    } else if Self::is_identifier(&expression) {
                        entry_name = BindingEntryName::Identifier(expr_into_identifier(expression));
                    } else {
                        self.syntax_error("Invalid destructuring assignment target");
                        break;
                    }
                } else {
                    // Object binding pattern entry: parse name.
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
                            entry_name = BindingEntryName::Identifier(self.make_identifier(entry_start, value));
                        } else if self.match_token(TokenType::BigIntLiteral) {
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            let name_value = if value.last() == Some(&(b'n' as u16)) {
                                value[..value.len() - 1].to_vec()
                            } else {
                                value
                            };
                            entry_name = BindingEntryName::Identifier(self.make_identifier(entry_start, name_value));
                        } else {
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            entry_name_value = value.clone();
                            entry_name = BindingEntryName::Identifier(self.make_identifier(entry_start, value));
                        }
                    } else if self.match_token(TokenType::BracketOpen) {
                        self.consume();
                        let expr = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                        entry_name = BindingEntryName::Expression(Box::new(expr));
                        self.consume_token(TokenType::BracketClose);
                    } else {
                        self.expected("identifier or computed property name");
                        break;
                    }

                    // Check for alias after ':'.
                    if !is_rest && self.match_token(TokenType::Colon) {
                        self.consume();
                        if self.allow_member_expressions {
                            let expr_start = self.position();
                            let expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none().forbid(&[TokenType::Equals]));
                            if Self::is_object_expression(&expression) || Self::is_array_expression(&expression) {
                                if let Some(pattern) = self.synthesize_binding_pattern(expr_start) {
                                    entry_alias = BindingEntryAlias::BindingPattern(Box::new(pattern));
                                }
                            } else if Self::is_member_expression(&expression) {
                                entry_alias = BindingEntryAlias::MemberExpression(Box::new(expression));
                            } else if Self::is_identifier(&expression) {
                                entry_alias = BindingEntryAlias::Identifier(expr_into_identifier(expression));
                            } else {
                                self.syntax_error("Invalid destructuring assignment target");
                                break;
                            }
                        } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                            let nested = self.parse_binding_pattern();
                            entry_alias = BindingEntryAlias::BindingPattern(Box::new(nested));
                        } else if self.match_identifier_name() {
                            let alias_start = self.position();
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            let id = self.make_identifier(alias_start, value.clone());
                            self.pattern_bound_names.push((value, id.clone()));
                            entry_alias = BindingEntryAlias::Identifier(id);
                        } else {
                            self.expected("identifier or binding pattern");
                            break;
                        }
                    } else if needs_alias {
                        self.expected("alias for string or numeric literal name");
                        break;
                    } else if !entry_name_value.is_empty() {
                        // Shorthand: name is the bound identifier.
                        // The identifier is in entry_name (BindingEntryName::Identifier).
                        if let BindingEntryName::Identifier(ref id) = entry_name {
                            self.pattern_bound_names.push((entry_name_value, id.clone()));
                        }
                    }
                }
            } else {
                // Array binding pattern entry.
                if self.allow_member_expressions {
                    let expr_start = self.position();
                    let expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none().forbid(&[TokenType::Equals]));
                    if Self::is_object_expression(&expression) || Self::is_array_expression(&expression) {
                        if let Some(pattern) = self.synthesize_binding_pattern(expr_start) {
                            entry_alias = BindingEntryAlias::BindingPattern(Box::new(pattern));
                        }
                    } else if Self::is_member_expression(&expression) {
                        entry_alias = BindingEntryAlias::MemberExpression(Box::new(expression));
                    } else if Self::is_identifier(&expression) {
                        let id = expr_into_identifier(expression);
                        self.pattern_bound_names.push((id.name.clone(), id.clone()));
                        entry_alias = BindingEntryAlias::Identifier(id);
                    } else {
                        self.syntax_error("Invalid destructuring assignment target");
                        break;
                    }
                } else if self.match_token(TokenType::CurlyOpen) || self.match_token(TokenType::BracketOpen) {
                    let nested = self.parse_binding_pattern();
                    entry_alias = BindingEntryAlias::BindingPattern(Box::new(nested));
                } else if self.match_identifier_name() {
                    let alias_start = self.position();
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let id = self.make_identifier(alias_start, value.clone());
                    self.pattern_bound_names.push((value, id.clone()));
                    entry_alias = BindingEntryAlias::Identifier(id);
                } else {
                    self.expected("identifier or binding pattern");
                    break;
                }
            }

            // Optional initializer.
            let initializer = if self.match_token(TokenType::Equals) {
                self.consume();
                Some(self.parse_expression(2, Associativity::Right, ForbiddenTokens::none()))
            } else {
                None
            };

            entries.push(BindingEntry {
                name: entry_name,
                alias: entry_alias,
                initializer,
                is_rest,
            });

            if self.match_token(TokenType::Comma) {
                self.consume();
            } else if is_object && !self.match_token(closing_token) {
                self.consume_token(TokenType::Comma);
            }
        }

        // Consume trailing commas for arrays.
        if !is_object {
            while self.match_token(TokenType::Comma) {
                self.consume();
            }
        }

        self.consume_token(closing_token);
        BindingPattern { kind, entries }
    }

    // === Import statement ===

    pub(crate) fn parse_import_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::Import);

        if self.program_type != ProgramType::Module {
            self.syntax_error("Cannot use 'import' outside a module");
        }

        // import ModuleSpecifier ;
        if self.match_token(TokenType::StringLiteral) {
            let module_specifier = self.consume_module_specifier();
            let attributes = self.parse_with_clause();
            self.consume_or_insert_semicolon();
            return self.stmt(start, Statement::Import(ImportStatementData {
                module_request: ModuleRequest { module_specifier, attributes },
                entries: Vec::new(),
            }));
        }

        let mut entries: Vec<ImportEntry> = Vec::new();
        let mut continue_parsing = true;

        // ImportedDefaultBinding.
        if self.match_imported_binding() {
            let tok = self.consume();
            let local_name = self.token_value(&tok).to_vec();
            entries.push(ImportEntry {
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
                    entries.push(ImportEntry {
                        import_name: None,
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
                            entries.push(ImportEntry {
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
                            entries.push(ImportEntry {
                                import_name: Some(name.clone()),
                                local_name: name,
                            });
                        }
                    } else if self.match_token(TokenType::StringLiteral) {
                        let tok = self.consume();
                        let (name, _) = self.parse_string_value(&tok);

                        if !self.match_as() {
                            self.expected("'as'");
                        }
                        self.consume(); // consume 'as'

                        let alias_tok = self.consume_identifier();
                        let alias = self.token_value(&alias_tok).to_vec();
                        self.check_identifier_name_for_assignment_validity(&alias, false);
                        entries.push(ImportEntry {
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
        let attributes = self.parse_with_clause();
        self.consume_or_insert_semicolon();

        self.stmt(start, Statement::Import(ImportStatementData {
            module_request: ModuleRequest { module_specifier, attributes },
            entries,
        }))
    }

    // === Export statement ===

    pub(crate) fn parse_export_statement(&mut self) -> Stmt {
        let start = self.position();
        self.consume_token(TokenType::Export);

        if self.program_type != ProgramType::Module {
            self.syntax_error("Cannot use 'export' outside a module");
        }

        let mut entries: Vec<ExportEntry> = Vec::new();
        let mut statement: Option<Box<Stmt>> = None;
        let mut is_default = false;
        let mut from_specifier: Option<Vec<u16>> = None;

        if self.match_token(TokenType::Default) {
            is_default = true;
            self.consume();

            let mut local_name: Option<Vec<u16>> = None;

            let matches_function = self.match_function_declaration_for_export();

            if matches_function != MatchesFunctionDeclaration::No {
                let has_default_name = matches_function == MatchesFunctionDeclaration::WithoutName;
                let decl = self.parse_function_declaration_for_export(has_default_name);
                if !has_default_name {
                    if let Statement::FunctionDeclaration(ref func) = decl.inner {
                        if let Some(ref name_id) = func.name {
                            local_name = Some(name_id.name.clone());
                        }
                    }
                }
                statement = Some(Box::new(decl));
            } else if self.match_token(TokenType::Class) {
                let next = self.next_token();
                if next.token_type != TokenType::CurlyOpen && next.token_type != TokenType::Extends {
                    // Named class declaration.
                    let decl = self.parse_class_declaration();
                    if let Statement::ClassDeclaration(ref class) = decl.inner {
                        if let Some(ref name_id) = class.name {
                            local_name = Some(name_id.name.clone());
                        }
                    }
                    statement = Some(Box::new(decl));
                } else {
                    // Unnamed class expression.
                    let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                    self.consume_or_insert_semicolon();
                    let expr_range = expr.range;
                    statement = Some(Box::new(Stmt::new(expr_range, Statement::Expression(Box::new(expr)))));
                }
            } else if self.match_expression() {
                let special_case = self.match_token(TokenType::Class)
                    || self.match_token(TokenType::Function)
                    || (self.match_token(TokenType::Async) && {
                        let nt = self.next_token();
                        nt.token_type == TokenType::Function && !nt.trivia_has_line_terminator
                    });
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                if !special_case {
                    self.consume_or_insert_semicolon();
                }
                let expr_range = expr.range;
                statement = Some(Box::new(Stmt::new(expr_range, Statement::Expression(Box::new(expr)))));
            } else {
                self.expected("declaration or assignment expression");
            }

            if local_name.is_none() {
                local_name = Some(utf16!("*default*").to_vec());
            }

            entries.push(ExportEntry {
                kind: ExportEntryKind::NamedExport,
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
                    self.consume(); // consume 'as'
                    let (exported_name, _) = self.parse_module_export_name();
                    entries.push(ExportEntry {
                        kind: ExportEntryKind::ModuleRequestAll,
                        export_name: Some(exported_name),
                        local_or_import_name: None,
                    });
                } else {
                    entries.push(ExportEntry {
                        kind: ExportEntryKind::ModuleRequestAllButDefault,
                        export_name: None,
                        local_or_import_name: None,
                    });
                }
                check_for_from = FromSpecifier::Required;
            } else if self.match_declaration() {
                let declaration = self.parse_declaration();
                let names = get_declaration_export_names(&declaration);
                for name in &names {
                    entries.push(ExportEntry {
                        kind: ExportEntryKind::NamedExport,
                        export_name: Some(name.clone()),
                        local_or_import_name: Some(name.clone()),
                    });
                }
                statement = Some(Box::new(declaration));
            } else if self.match_token(TokenType::Var) {
                let var_decl = self.parse_variable_declaration(false);
                let names = get_declaration_export_names(&var_decl);
                for name in &names {
                    entries.push(ExportEntry {
                        kind: ExportEntryKind::NamedExport,
                        export_name: Some(name.clone()),
                        local_or_import_name: Some(name.clone()),
                    });
                }
                statement = Some(Box::new(var_decl));
            } else if self.match_token(TokenType::CurlyOpen) {
                self.consume();
                check_for_from = FromSpecifier::Optional;

                while !self.done() && !self.match_token(TokenType::CurlyClose) {
                    let (identifier, was_string) = self.parse_module_export_name();
                    if was_string {
                        check_for_from = FromSpecifier::Required;
                    }

                    if self.match_as() {
                        self.consume(); // consume 'as'
                        let (export_name, _) = self.parse_module_export_name();
                        entries.push(ExportEntry {
                            kind: ExportEntryKind::NamedExport,
                            export_name: Some(export_name),
                            local_or_import_name: Some(identifier),
                        });
                    } else {
                        entries.push(ExportEntry {
                            kind: ExportEntryKind::NamedExport,
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
                    entries.push(ExportEntry {
                        kind: ExportEntryKind::EmptyNamedExport,
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

        let module_request = if let Some(specifier) = from_specifier {
            let attributes = self.parse_with_clause();
            Some(ModuleRequest { module_specifier: specifier, attributes })
        } else {
            None
        };

        self.stmt(start, Statement::Export(ExportStatementData {
            statement,
            entries,
            is_default_export: is_default,
            module_request,
        }))
    }

    // === Import/Export helpers ===

    fn match_imported_binding(&self) -> bool {
        self.match_identifier() || self.match_token(TokenType::Yield) || self.match_token(TokenType::Await)
    }

    fn match_as(&self) -> bool {
        self.match_token(TokenType::Identifier) && self.token_original_value(&self.current_token) == utf16!("as")
    }

    fn match_from(&self) -> bool {
        self.match_token(TokenType::Identifier) && self.token_original_value(&self.current_token) == utf16!("from")
    }

    fn consume_module_specifier(&mut self) -> Vec<u16> {
        if !self.match_token(TokenType::StringLiteral) {
            self.expected("module specifier (string)");
            return utf16!("!!invalid!!").to_vec();
        }
        let tok = self.consume();
        let (value, _) = self.parse_string_value(&tok);
        value
    }

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

    fn parse_with_clause(&mut self) -> Vec<ImportAttribute> {
        if !self.match_token(TokenType::With) {
            return Vec::new();
        }
        self.consume();
        self.consume_token(TokenType::CurlyOpen);

        let mut attributes = Vec::new();
        while !self.done() && !self.match_token(TokenType::CurlyClose) {
            let key = if self.match_token(TokenType::StringLiteral) {
                let tok = self.consume();
                let (value, _) = self.parse_string_value(&tok);
                value
            } else if self.match_identifier_name() {
                let tok = self.consume();
                self.token_value(&tok).to_vec()
            } else {
                self.expected("identifier or string as attribute key");
                self.consume();
                continue;
            };

            self.consume_token(TokenType::Colon);

            if self.match_token(TokenType::StringLiteral) {
                let tok = self.consume();
                let (value, _) = self.parse_string_value(&tok);
                attributes.push(ImportAttribute { key, value });
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
        attributes
    }

    fn match_function_declaration_for_export(&mut self) -> MatchesFunctionDeclaration {
        if self.match_token(TokenType::Function) {
            let next = self.next_token();
            if next.token_type == TokenType::Asterisk {
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
            self.save_state();
            self.consume(); // async
            self.consume(); // function
            if self.match_token(TokenType::Asterisk) {
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

    fn parse_function_declaration_for_export(&mut self, has_default_name: bool) -> Stmt {
        if has_default_name {
            self.has_default_export_name = true;
            let result = self.parse_function_declaration();
            self.has_default_export_name = false;
            result
        } else {
            self.parse_function_declaration()
        }
    }
}

#[derive(PartialEq)]
enum MatchesFunctionDeclaration {
    No,
    Yes,
    WithoutName,
}
