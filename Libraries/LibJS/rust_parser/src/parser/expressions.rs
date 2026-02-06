/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Expression parsing: primary, secondary (binary/postfix), unary, and
//! precedence climbing.

use crate::ast_bridge::{NodeHandle, NULL_HANDLE};
use crate::parser::{Associativity, ForbiddenTokens, FunctionKind, Parser};
use crate::token::{Token, TokenType};

impl<'a> Parser<'a> {
    // === Expression matching ===

    pub(crate) fn match_expression(&self) -> bool {
        match self.current_token_type() {
            TokenType::BoolLiteral
            | TokenType::NumericLiteral
            | TokenType::BigIntLiteral
            | TokenType::StringLiteral
            | TokenType::NullLiteral
            | TokenType::RegexLiteral
            | TokenType::TemplateLiteralStart
            | TokenType::This
            | TokenType::Super
            | TokenType::New
            | TokenType::Class
            | TokenType::Function
            | TokenType::ParenOpen
            | TokenType::CurlyOpen
            | TokenType::BracketOpen
            | TokenType::PrivateIdentifier => true,

            TokenType::Async => true,

            TokenType::Yield => self.in_generator_function_context,
            TokenType::Await => self.await_expression_is_valid || self.program_type == super::ProgramType::Module,

            TokenType::Import => {
                // import( and import. are expressions
                let next = unsafe {
                    let ptr = self as *const Self as *mut Self;
                    (*ptr).next_token()
                };
                next.token_type == TokenType::ParenOpen || next.token_type == TokenType::Period
            }

            _ => {
                if self.match_identifier() {
                    return true;
                }
                self.match_unary_prefixed_expression()
            }
        }
    }

    pub(crate) fn match_unary_prefixed_expression(&self) -> bool {
        matches!(
            self.current_token_type(),
            TokenType::PlusPlus
                | TokenType::MinusMinus
                | TokenType::ExclamationMark
                | TokenType::Tilde
                | TokenType::Plus
                | TokenType::Minus
                | TokenType::Typeof
                | TokenType::Void
                | TokenType::Delete
        )
    }

    pub(crate) fn match_secondary_expression(&self, forbidden: &ForbiddenTokens) -> bool {
        let tt = self.current_token_type();
        if !forbidden.allows(tt) {
            return false;
        }
        match tt {
            // Member access and call
            TokenType::Period | TokenType::BracketOpen | TokenType::ParenOpen | TokenType::QuestionMarkPeriod => true,

            // Postfix (no line terminator)
            TokenType::PlusPlus | TokenType::MinusMinus => {
                !self.current_token.trivia_has_line_terminator
            }

            // Binary
            TokenType::DoubleAsterisk
            | TokenType::Asterisk | TokenType::Slash | TokenType::Percent
            | TokenType::Plus | TokenType::Minus
            | TokenType::ShiftLeft | TokenType::ShiftRight | TokenType::UnsignedShiftRight
            | TokenType::LessThan | TokenType::LessThanEquals | TokenType::GreaterThan | TokenType::GreaterThanEquals
            | TokenType::In | TokenType::Instanceof
            | TokenType::EqualsEquals | TokenType::ExclamationMarkEquals
            | TokenType::EqualsEqualsEquals | TokenType::ExclamationMarkEqualsEquals
            | TokenType::Ampersand | TokenType::Caret | TokenType::Pipe
            | TokenType::DoubleQuestionMark | TokenType::DoubleAmpersand | TokenType::DoublePipe => true,

            // Ternary
            TokenType::QuestionMark => true,

            // Assignment
            TokenType::Equals
            | TokenType::PlusEquals | TokenType::MinusEquals
            | TokenType::DoubleAsteriskEquals | TokenType::AsteriskEquals
            | TokenType::SlashEquals | TokenType::PercentEquals
            | TokenType::ShiftLeftEquals | TokenType::ShiftRightEquals
            | TokenType::UnsignedShiftRightEquals
            | TokenType::AmpersandEquals | TokenType::CaretEquals | TokenType::PipeEquals
            | TokenType::DoubleAmpersandEquals | TokenType::DoublePipeEquals
            | TokenType::DoubleQuestionMarkEquals => true,

            // Template literal (tagged)
            TokenType::TemplateLiteralStart => true,

            _ => false,
        }
    }

    // === Main expression parser with precedence climbing ===

    pub(crate) fn parse_expression(&mut self, min_precedence: i32, associativity: Associativity, forbidden: ForbiddenTokens) -> NodeHandle {
        if self.match_unary_prefixed_expression() {
            let start = self.position();
            let expr = self.parse_unary_prefixed_expression();

            // Check for ** after unary (not allowed without parens)
            if self.match_token(TokenType::DoubleAsterisk) {
                self.syntax_error("Unparenthesized unary expression can't appear on the left-hand side of '**'");
            }

            return self.continue_parse_expression(start, expr, min_precedence, associativity, forbidden);
        }

        let (expr, should_continue) = self.parse_primary_expression();
        if !should_continue {
            return expr;
        }

        // Handle tagged template literal
        let expr = if self.match_token(TokenType::TemplateLiteralStart) {
            let start = self.position();
            let template = self.parse_template_literal(true);
            let span = self.span_from(start);
            self.builder.create_tagged_template_literal(span, expr, template)
        } else {
            expr
        };

        self.continue_parse_expression(self.position(), expr, min_precedence, associativity, forbidden)
    }

    fn continue_parse_expression(
        &mut self,
        _start: (u32, u32, u32),
        mut expr: NodeHandle,
        min_precedence: i32,
        associativity: Associativity,
        forbidden: ForbiddenTokens,
    ) -> NodeHandle {
        while self.match_secondary_expression(&forbidden) {
            let new_precedence = Self::operator_precedence(self.current_token_type());
            if new_precedence < min_precedence {
                break;
            }
            if new_precedence == min_precedence && associativity == Associativity::Left {
                break;
            }

            let result = self.parse_secondary_expression(expr, new_precedence, forbidden);
            expr = result.0;
            let new_forbidden = result.1;
            let _ = new_forbidden; // Forbidden tokens from secondary expression (e.g., ?? forbids &&/||)
        }

        // Handle comma (sequence expression)
        if min_precedence <= 1 && self.match_token(TokenType::Comma) && forbidden.allows(TokenType::Comma) {
            let start = self.position();
            let mut expressions = vec![expr];
            while self.match_token(TokenType::Comma) {
                self.consume();
                expressions.push(self.parse_expression(2, Associativity::Right, forbidden));
            }
            let span = self.span_from(start);
            return self.builder.create_sequence_expression(span, &expressions);
        }

        expr
    }

    // === Primary expression parsing ===

    fn parse_primary_expression(&mut self) -> (NodeHandle, bool) {
        let start = self.position();
        let token = self.current_token().clone();

        match token.token_type {
            TokenType::ParenOpen => {
                // Could be arrow function or parenthesized expression.
                // Consume '(' first, then try arrow function (which expects '(' already consumed).
                self.consume_token(TokenType::ParenOpen);
                if let Some(arrow) = self.try_parse_arrow_function_expression(true, false) {
                    return (arrow, false);
                }
                if self.match_token(TokenType::ParenClose) {
                    self.syntax_error("Unexpected token )");
                    self.consume();
                    return (self.builder.create_identifier(self.span_from(start), &[]), true);
                }
                let expr = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::ParenClose);
                (expr, true)
            }

            TokenType::This => {
                self.consume();
                (self.builder.create_this_expression(self.span_from(start)), true)
            }

            TokenType::Class => {
                let expr = self.parse_class_expression(false);
                (expr, true)
            }

            TokenType::Super => {
                self.consume();
                if self.match_token(TokenType::ParenOpen) {
                    // super(...) - SuperCall
                    let (arg_values, arg_spreads) = self.parse_arguments();
                    (self.builder.create_super_call(self.span_from(start), &arg_values, &arg_spreads), true)
                } else {
                    if !self.allow_super_property_lookup {
                        self.syntax_error("'super' keyword unexpected here");
                    }
                    (self.builder.create_super_expression(self.span_from(start)), true)
                }
            }

            TokenType::NumericLiteral => {
                let tok = self.consume_and_validate_numeric_literal();
                let value_str = self.token_value(&tok);
                let value = parse_numeric_value(value_str);
                (self.builder.create_numeric_literal(self.span_from(start), value), true)
            }

            TokenType::BigIntLiteral => {
                let tok = self.consume();
                let value = self.token_value(&tok);
                // BigInt value is the string without the trailing 'n'
                let value_utf8: String = value.iter()
                    .take(value.len().saturating_sub(1))
                    .map(|&c| c as u8 as char)
                    .collect();
                (self.builder.create_bigint_literal(self.span_from(start), value_utf8.as_bytes()), true)
            }

            TokenType::BoolLiteral => {
                let tok = self.consume();
                let value = self.token_value(&tok);
                let is_true = value == super::utf16_lit("true");
                (self.builder.create_boolean_literal(self.span_from(start), is_true), true)
            }

            TokenType::StringLiteral => {
                let tok = self.consume();
                let value = self.parse_string_value(&tok);
                (self.builder.create_string_literal(self.span_from(start), &value), true)
            }

            TokenType::NullLiteral => {
                self.consume();
                (self.builder.create_null_literal(self.span_from(start)), true)
            }

            TokenType::CurlyOpen => {
                let expr = self.parse_object_expression();
                (expr, true)
            }

            TokenType::BracketOpen => {
                let expr = self.parse_array_expression();
                (expr, true)
            }

            TokenType::Function => {
                let expr = self.parse_function_expression();
                (expr, false)
            }

            TokenType::Async => {
                let next = self.next_token();
                if next.token_type == TokenType::Function && !next.trivia_has_line_terminator {
                    let expr = self.parse_function_expression();
                    return (expr, false);
                }
                // async arrow function: arrow parser will consume 'async' and optional '('
                if let Some(arrow) = self.try_parse_arrow_function_expression(next.token_type == TokenType::ParenOpen, true) {
                    return (arrow, false);
                }
                // Just an identifier "async"
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                (self.builder.create_identifier(self.span_from(start), &value), true)
            }

            TokenType::TemplateLiteralStart => {
                let expr = self.parse_template_literal(false);
                (expr, true)
            }

            TokenType::New => {
                let expr = self.parse_new_expression();
                (expr, true)
            }

            TokenType::Import => {
                self.consume();
                if self.match_token(TokenType::Period) {
                    // import.meta
                    self.consume();
                    // Expect "meta"
                    self.consume_token(TokenType::Identifier);
                    (self.builder.create_meta_property(self.span_from(start), 1), true)
                } else if self.match_token(TokenType::ParenOpen) {
                    // import()
                    self.consume();
                    let specifier = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                    let options = if self.match_token(TokenType::Comma) {
                        self.consume();
                        if self.match_token(TokenType::ParenClose) {
                            NULL_HANDLE
                        } else {
                            let opts = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                            // Allow trailing comma
                            if self.match_token(TokenType::Comma) {
                                self.consume();
                            }
                            opts
                        }
                    } else {
                        NULL_HANDLE
                    };
                    self.consume_token(TokenType::ParenClose);
                    (self.builder.create_import_call(self.span_from(start), specifier, options), true)
                } else {
                    self.expected("'.' or '('");
                    (self.builder.create_identifier(self.span_from(start), &[]), true)
                }
            }

            TokenType::Yield if self.in_generator_function_context => {
                let expr = self.parse_yield_expression();
                (expr, false)
            }

            TokenType::Await if self.await_expression_is_valid => {
                let expr = self.parse_await_expression();
                (expr, false)
            }

            TokenType::PrivateIdentifier => {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                (self.builder.create_private_identifier(self.span_from(start), &value), true)
            }

            TokenType::RegexLiteral => {
                let tok = self.consume();
                let _value = self.token_value(&tok);
                // TODO: Create RegExpLiteral node
                (self.builder.create_identifier(self.span_from(start), &super::utf16_lit("undefined")), true)
            }

            TokenType::Slash | TokenType::SlashEquals => {
                // Re-lex as regex
                let tok = self.lexer.force_slash_as_regex();
                self.current_token = tok;
                let tok = self.consume();
                let _value = self.token_value(&tok);
                // TODO: Create RegExpLiteral node
                (self.builder.create_identifier(self.span_from(start), &super::utf16_lit("undefined")), true)
            }

            _ => {
                if self.match_identifier() {
                    // Try arrow function first for single identifier
                    if let Some(arrow) = self.try_parse_arrow_function_expression(false, false) {
                        return (arrow, false);
                    }
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    (self.builder.create_identifier(self.span_from(start), &value), true)
                } else if self.match_token(TokenType::EscapedKeyword) {
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    (self.builder.create_identifier(self.span_from(start), &value), true)
                } else {
                    self.expected("expression");
                    self.consume();
                    (self.builder.create_identifier(self.span_from(start), &[]), true)
                }
            }
        }
    }

    // === Secondary expression parsing ===

    fn parse_secondary_expression(&mut self, lhs: NodeHandle, min_precedence: i32, forbidden: ForbiddenTokens) -> (NodeHandle, ForbiddenTokens) {
        let start = self.position();
        let tt = self.current_token_type();

        match tt {
            // === Binary operators ===
            TokenType::Plus | TokenType::Minus | TokenType::Asterisk | TokenType::Slash
            | TokenType::Percent | TokenType::DoubleAsterisk
            | TokenType::ShiftLeft | TokenType::ShiftRight | TokenType::UnsignedShiftRight
            | TokenType::Ampersand | TokenType::Caret | TokenType::Pipe
            | TokenType::LessThan | TokenType::LessThanEquals
            | TokenType::GreaterThan | TokenType::GreaterThanEquals
            | TokenType::EqualsEquals | TokenType::ExclamationMarkEquals
            | TokenType::EqualsEqualsEquals | TokenType::ExclamationMarkEqualsEquals
            | TokenType::In | TokenType::Instanceof => {
                let op = token_to_binary_op(tt);
                self.consume();
                let rhs = self.parse_expression(min_precedence, Self::operator_associativity(tt), forbidden);
                let span = self.span_from(start);
                (self.builder.create_binary_expression(span, op, lhs, rhs), ForbiddenTokens::none())
            }

            // === Logical operators ===
            TokenType::DoubleAmpersand => {
                self.consume();
                let new_forbidden = forbidden.forbid(&[TokenType::DoubleQuestionMark]);
                let rhs = self.parse_expression(min_precedence, Associativity::Left, new_forbidden);
                let span = self.span_from(start);
                (self.builder.create_logical_expression(span, 0, lhs, rhs), new_forbidden) // And = 0
            }
            TokenType::DoublePipe => {
                self.consume();
                let new_forbidden = forbidden.forbid(&[TokenType::DoubleQuestionMark]);
                let rhs = self.parse_expression(min_precedence, Associativity::Left, new_forbidden);
                let span = self.span_from(start);
                (self.builder.create_logical_expression(span, 1, lhs, rhs), new_forbidden) // Or = 1
            }
            TokenType::DoubleQuestionMark => {
                self.consume();
                let new_forbidden = forbidden.forbid(&[TokenType::DoubleAmpersand, TokenType::DoublePipe]);
                let rhs = self.parse_expression(min_precedence, Associativity::Left, new_forbidden);
                let span = self.span_from(start);
                (self.builder.create_logical_expression(span, 2, lhs, rhs), new_forbidden) // NullishCoalescing = 2
            }

            // === Assignment ===
            TokenType::Equals | TokenType::PlusEquals | TokenType::MinusEquals
            | TokenType::DoubleAsteriskEquals | TokenType::AsteriskEquals
            | TokenType::SlashEquals | TokenType::PercentEquals
            | TokenType::ShiftLeftEquals | TokenType::ShiftRightEquals
            | TokenType::UnsignedShiftRightEquals | TokenType::AmpersandEquals
            | TokenType::CaretEquals | TokenType::PipeEquals
            | TokenType::DoubleAmpersandEquals | TokenType::DoublePipeEquals
            | TokenType::DoubleQuestionMarkEquals => {
                let op = token_to_assignment_op(tt);
                self.consume();
                let rhs = self.parse_expression(min_precedence, Associativity::Right, forbidden);
                let span = self.span_from(start);
                (self.builder.create_assignment_expression(span, op, lhs, rhs), ForbiddenTokens::none())
            }

            // === Ternary ===
            TokenType::QuestionMark => {
                self.consume();
                let consequent = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::Colon);
                let alternate = self.parse_expression(2, Associativity::Right, forbidden);
                let span = self.span_from(start);
                (self.builder.create_conditional_expression(span, lhs, consequent, alternate), ForbiddenTokens::none())
            }

            // === Member access ===
            TokenType::Period => {
                self.consume();
                if self.match_token(TokenType::PrivateIdentifier) {
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let prop = self.builder.create_private_identifier(self.span_from(start), &value);
                    let span = self.span_from(start);
                    (self.builder.create_member_expression(span, lhs, prop, false), ForbiddenTokens::none())
                } else if self.match_identifier_name() {
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let prop = self.builder.create_identifier(self.span_from(start), &value);
                    let span = self.span_from(start);
                    (self.builder.create_member_expression(span, lhs, prop, false), ForbiddenTokens::none())
                } else {
                    self.expected("property name");
                    (lhs, ForbiddenTokens::none())
                }
            }

            // === Computed member access ===
            TokenType::BracketOpen => {
                self.consume();
                let prop = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::BracketClose);
                let span = self.span_from(start);
                (self.builder.create_member_expression(span, lhs, prop, true), ForbiddenTokens::none())
            }

            // === Call ===
            TokenType::ParenOpen => {
                let expr = self.parse_call_expression(lhs);
                (expr, ForbiddenTokens::none())
            }

            // === Optional chaining ===
            TokenType::QuestionMarkPeriod => {
                // TODO: parse_optional_chain
                self.consume();
                if self.match_identifier_name() {
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let prop = self.builder.create_identifier(self.span_from(start), &value);
                    let span = self.span_from(start);
                    (self.builder.create_member_expression(span, lhs, prop, false), ForbiddenTokens::none())
                } else {
                    self.expected("property name");
                    (lhs, ForbiddenTokens::none())
                }
            }

            // === Postfix ===
            TokenType::PlusPlus => {
                self.consume();
                let span = self.span_from(start);
                (self.builder.create_update_expression(span, 0, lhs, false), ForbiddenTokens::none()) // Increment = 0
            }
            TokenType::MinusMinus => {
                self.consume();
                let span = self.span_from(start);
                (self.builder.create_update_expression(span, 1, lhs, false), ForbiddenTokens::none()) // Decrement = 1
            }

            // === Tagged template literal ===
            TokenType::TemplateLiteralStart => {
                let template = self.parse_template_literal(true);
                let span = self.span_from(start);
                (self.builder.create_tagged_template_literal(span, lhs, template), ForbiddenTokens::none())
            }

            _ => {
                self.expected("secondary expression");
                (lhs, ForbiddenTokens::none())
            }
        }
    }

    // === Unary prefix expression ===

    fn parse_unary_prefixed_expression(&mut self) -> NodeHandle {
        let start = self.position();
        let tt = self.current_token_type();

        match tt {
            TokenType::PlusPlus => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_update_expression(self.span_from(start), 0, expr, true) // Increment = 0
            }
            TokenType::MinusMinus => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_update_expression(self.span_from(start), 1, expr, true) // Decrement = 1
            }
            TokenType::ExclamationMark => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_unary_expression(self.span_from(start), 0, expr) // Not = 0
            }
            TokenType::Tilde => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_unary_expression(self.span_from(start), 1, expr) // BitwiseNot = 1
            }
            TokenType::Plus => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_unary_expression(self.span_from(start), 2, expr) // Plus = 2
            }
            TokenType::Minus => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_unary_expression(self.span_from(start), 3, expr) // Minus = 3
            }
            TokenType::Typeof => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_unary_expression(self.span_from(start), 4, expr) // Typeof = 4
            }
            TokenType::Void => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_unary_expression(self.span_from(start), 5, expr) // Void = 5
            }
            TokenType::Delete => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.builder.create_unary_expression(self.span_from(start), 6, expr) // Delete = 6
            }
            _ => {
                self.expected("unary expression");
                self.consume();
                self.builder.create_identifier(self.span_from(start), &[])
            }
        }
    }

    // === new expression ===

    fn parse_new_expression(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::New);

        // new.target
        if self.match_token(TokenType::Period) {
            self.consume();
            self.consume_token(TokenType::Identifier); // "target"
            return self.builder.create_meta_property(self.span_from(start), 0); // NewTarget = 0
        }

        // new new ... or new expr(args)
        if self.match_token(TokenType::New) {
            let inner = self.parse_new_expression();
            return inner;
        }

        let forbidden = ForbiddenTokens::none().forbid(&[TokenType::ParenOpen, TokenType::QuestionMarkPeriod]);
        let callee = self.parse_expression(19, Associativity::Right, forbidden);

        if self.match_token(TokenType::ParenOpen) {
            let (arg_values, arg_spreads) = self.parse_arguments();
            let span = self.span_from(start);
            self.builder.create_new_expression(span, callee, &arg_values, &arg_spreads)
        } else {
            let span = self.span_from(start);
            self.builder.create_new_expression(span, callee, &[], &[])
        }
    }

    // === Call expression ===

    pub(crate) fn parse_call_expression(&mut self, callee: NodeHandle) -> NodeHandle {
        let start = self.position();
        let (arg_values, arg_spreads) = self.parse_arguments();
        let span = self.span_from(start);
        self.builder.create_call_expression(span, callee, &arg_values, &arg_spreads)
    }

    /// Parse function call arguments: (arg1, arg2, ...rest)
    pub(crate) fn parse_arguments(&mut self) -> (Vec<NodeHandle>, Vec<bool>) {
        self.consume_token(TokenType::ParenOpen);
        let mut values = Vec::new();
        let mut spreads = Vec::new();

        while !self.match_token(TokenType::ParenClose) && !self.done() {
            if self.match_token(TokenType::TripleDot) {
                self.consume();
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                values.push(expr);
                spreads.push(true);
            } else {
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                values.push(expr);
                spreads.push(false);
            }
            if !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();
        }

        self.consume_token(TokenType::ParenClose);
        (values, spreads)
    }

    // === Yield expression ===

    fn parse_yield_expression(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Yield);

        if self.current_token.trivia_has_line_terminator
            || self.match_token(TokenType::Semicolon)
            || self.done()
            || self.match_token(TokenType::CurlyClose)
            || self.match_token(TokenType::ParenClose)
            || self.match_token(TokenType::BracketClose)
            || self.match_token(TokenType::Comma)
            || self.match_token(TokenType::Colon)
        {
            return self.builder.create_yield_expression(self.span_from(start), NULL_HANDLE, false);
        }

        let is_yield_from = self.match_token(TokenType::Asterisk);
        if is_yield_from {
            self.consume();
        }

        let argument = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
        self.builder.create_yield_expression(self.span_from(start), argument, is_yield_from)
    }

    // === Await expression ===

    fn parse_await_expression(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::Await);
        let argument = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
        self.builder.create_await_expression(self.span_from(start), argument)
    }

    // === Object expression ===

    fn parse_object_expression(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::CurlyOpen);

        let mut properties = Vec::new();
        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            if self.match_token(TokenType::TripleDot) {
                // Spread property
                let spread_start = self.position();
                self.consume();
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                let span = self.span_from(spread_start);
                properties.push(self.builder.create_object_property(span, NULL_HANDLE, expr, 3, false)); // Spread = 3
            } else {
                let prop = self.parse_object_property();
                properties.push(prop);
            }

            if !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();
        }

        self.consume_token(TokenType::CurlyClose);
        self.builder.create_object_expression(self.span_from(start), &properties)
    }

    fn parse_object_property(&mut self) -> NodeHandle {
        let start = self.position();
        let mut is_getter = false;
        let mut is_setter = false;
        let mut is_async = false;
        let mut is_generator = false;

        // Check for async/get/set/generator modifiers
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
                if self.match_token(TokenType::Asterisk) {
                    is_generator = true;
                    self.consume();
                }
            }
        }

        if !is_getter && !is_setter && !is_async && self.match_token(TokenType::Asterisk) {
            is_generator = true;
            self.consume();
        }

        // Parse property key
        let (key, key_value) = self.parse_property_key();

        // Method shorthand
        if self.match_token(TokenType::ParenOpen) {
            let func = self.parse_method_definition(is_async, is_generator, is_getter, is_setter);
            let prop_type = if is_getter { 1 } else if is_setter { 2 } else { 0 }; // KeyValue=0, Getter=1, Setter=2
            return self.builder.create_object_property(self.span_from(start), key, func, prop_type, true);
        }

        // Getter/setter
        if is_getter || is_setter {
            let func = self.parse_method_definition(false, false, is_getter, is_setter);
            let prop_type = if is_getter { 1 } else { 2 };
            return self.builder.create_object_property(self.span_from(start), key, func, prop_type, true);
        }

        // key: value
        if self.match_token(TokenType::Colon) {
            self.consume();
            let value = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
            return self.builder.create_object_property(self.span_from(start), key, value, 0, false); // KeyValue = 0
        }

        // Shorthand property: { x } is equivalent to { x: x }
        if let Some(kv) = key_value {
            let value = self.builder.create_identifier(self.span_from(start), &kv);
            return self.builder.create_object_property(self.span_from(start), key, value, 0, false);
        }

        self.expected("':' or '('");
        self.builder.create_object_property(self.span_from(start), key, NULL_HANDLE, 0, false)
    }

    pub(crate) fn match_property_key_ahead(&mut self) -> bool {
        let next = self.next_token();
        matches!(
            next.token_type,
            TokenType::BracketOpen
                | TokenType::StringLiteral
                | TokenType::NumericLiteral
                | TokenType::BigIntLiteral
                | TokenType::PrivateIdentifier
        ) || next.token_type.is_identifier_name()
    }

    /// Parse a property key, returning (key_handle, shorthand_identifier_value).
    pub(crate) fn parse_property_key(&mut self) -> (NodeHandle, Option<Vec<u16>>) {
        let start = self.position();
        match self.current_token_type() {
            TokenType::BracketOpen => {
                self.consume();
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::BracketClose);
                (expr, None)
            }
            TokenType::StringLiteral => {
                let tok = self.consume();
                let value = self.parse_string_value(&tok);
                (self.builder.create_string_literal(self.span_from(start), &value), None)
            }
            TokenType::NumericLiteral => {
                let tok = self.consume_and_validate_numeric_literal();
                let value_str = self.token_value(&tok);
                let value = parse_numeric_value(value_str);
                (self.builder.create_numeric_literal(self.span_from(start), value), None)
            }
            _ => {
                if self.match_identifier_name() {
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let key = self.builder.create_string_literal(self.span_from(start), &value);
                    (key, Some(value))
                } else {
                    self.expected("property key");
                    self.consume();
                    (self.builder.create_string_literal(self.span_from(start), &[]), None)
                }
            }
        }
    }

    // === Array expression ===

    fn parse_array_expression(&mut self) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::BracketOpen);

        let mut elements = Vec::new();
        while !self.match_token(TokenType::BracketClose) && !self.done() {
            if self.match_token(TokenType::Comma) {
                // Hole
                elements.push(NULL_HANDLE);
                self.consume();
                continue;
            }
            if self.match_token(TokenType::TripleDot) {
                let spread_start = self.position();
                self.consume();
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                elements.push(self.builder.create_spread_expression(self.span_from(spread_start), expr));
            } else {
                elements.push(self.parse_expression(2, Associativity::Right, ForbiddenTokens::none()));
            }
            if !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();
        }

        self.consume_token(TokenType::BracketClose);
        self.builder.create_array_expression(self.span_from(start), &elements)
    }

    // === Template literal ===

    pub(crate) fn parse_template_literal(&mut self, _is_tagged: bool) -> NodeHandle {
        let start = self.position();
        self.consume_token(TokenType::TemplateLiteralStart);

        let mut parts = Vec::new();
        loop {
            if self.match_token(TokenType::TemplateLiteralEnd) {
                self.consume();
                parts.push(self.builder.create_string_literal(self.span_from(start), &[]));
                break;
            }
            if self.match_token(TokenType::TemplateLiteralString) {
                let tok = self.consume();
                let value = self.process_template_string_value(&tok);
                parts.push(self.builder.create_string_literal(self.span_from(start), &value));
            } else if self.match_token(TokenType::TemplateLiteralExprStart) {
                self.consume();
                let expr = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                parts.push(expr);
                self.consume_token(TokenType::TemplateLiteralExprEnd);
            } else if self.done() {
                self.expected("template literal end");
                break;
            } else {
                self.consume();
            }
        }

        self.builder.create_template_literal(self.span_from(start), &parts)
    }

    /// Process a template literal string value — no quote stripping, just escape processing.
    fn process_template_string_value(&self, token: &Token) -> Vec<u16> {
        let raw = self.token_value(token);
        self.process_escape_sequences(raw)
    }

    // === String value parsing ===

    pub(crate) fn parse_string_value(&self, token: &Token) -> Vec<u16> {
        let raw = self.token_value(token);
        if raw.len() < 2 {
            return Vec::new();
        }
        // Strip surrounding quotes
        let inner = &raw[1..raw.len() - 1];
        self.process_escape_sequences(inner)
    }

    fn process_escape_sequences(&self, inner: &[u16]) -> Vec<u16> {
        let mut result = Vec::with_capacity(inner.len());
        let mut i = 0;
        while i < inner.len() {
            if inner[i] == b'\\' as u16 && i + 1 < inner.len() {
                i += 1;
                match inner[i] {
                    c if c == b'n' as u16 => result.push(b'\n' as u16),
                    c if c == b'r' as u16 => result.push(b'\r' as u16),
                    c if c == b't' as u16 => result.push(b'\t' as u16),
                    c if c == b'0' as u16 => result.push(0),
                    c if c == b'b' as u16 => result.push(8),
                    c if c == b'f' as u16 => result.push(12),
                    c if c == b'v' as u16 => result.push(11),
                    c if c == b'x' as u16 => {
                        // Hex escape: \xHH
                        if i + 2 < inner.len() {
                            let hi = hex_digit(inner[i + 1]);
                            let lo = hex_digit(inner[i + 2]);
                            if let (Some(h), Some(l)) = (hi, lo) {
                                result.push(h * 16 + l);
                                i += 2;
                            } else {
                                result.push(inner[i]);
                            }
                        } else {
                            result.push(inner[i]);
                        }
                    }
                    c if c == b'u' as u16 => {
                        // Unicode escape: \uHHHH or \u{H+}
                        if i + 1 < inner.len() && inner[i + 1] == b'{' as u16 {
                            // \u{H+}
                            i += 2;
                            let mut code_point: u32 = 0;
                            while i < inner.len() && inner[i] != b'}' as u16 {
                                if let Some(d) = hex_digit(inner[i]) {
                                    code_point = code_point * 16 + d as u32;
                                }
                                i += 1;
                            }
                            // Encode as UTF-16
                            if code_point <= 0xFFFF {
                                result.push(code_point as u16);
                            } else {
                                // Surrogate pair
                                let code_point = code_point - 0x10000;
                                result.push((0xD800 + (code_point >> 10)) as u16);
                                result.push((0xDC00 + (code_point & 0x3FF)) as u16);
                            }
                        } else if i + 4 < inner.len() {
                            // \uHHHH
                            let mut code_point: u16 = 0;
                            for j in 1..=4 {
                                if let Some(d) = hex_digit(inner[i + j]) {
                                    code_point = code_point * 16 + d;
                                }
                            }
                            result.push(code_point);
                            i += 4;
                        } else {
                            result.push(inner[i]);
                        }
                    }
                    c => result.push(c),
                }
            } else {
                result.push(inner[i]);
            }
            i += 1;
        }
        result
    }

    // === Arrow function ===

    /// Try to parse an arrow function expression.
    /// When `expect_parens` is true and `is_async` is false, the caller must
    /// have already consumed '('. For async arrows, this function consumes both
    /// 'async' and '(' itself.
    pub(crate) fn try_parse_arrow_function_expression(&mut self, expect_parens: bool, is_async: bool) -> Option<NodeHandle> {
        let start = self.position();

        // Fast path: single identifier => arrow
        if !expect_parens && !is_async {
            if !self.match_identifier() {
                return None;
            }
            let next = self.next_token();
            if next.token_type != TokenType::Arrow || next.trivia_has_line_terminator {
                return None;
            }
        }

        self.save_state();

        if is_async {
            self.consume(); // consume 'async'
            if self.current_token.trivia_has_line_terminator {
                self.load_state();
                return None;
            }
            if expect_parens {
                self.consume_token(TokenType::ParenOpen);
            }
        }

        let (params, function_length);

        if expect_parens {
            // '(' already consumed (by caller or above for async case).
            let previous_errors = self.errors.len();
            let result = self.parse_formal_parameters_without_parens();
            params = result.0;
            function_length = result.1;
            // If there were new syntax errors during parameter parsing, abort.
            if self.errors.len() > previous_errors {
                self.load_state();
                return None;
            }
            if !self.match_token(TokenType::ParenClose) {
                self.load_state();
                return None;
            }
            self.consume(); // consume ')'
        } else {
            // Single parameter (identifier)
            if self.match_identifier() {
                let param_start = self.position();
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                let binding = self.builder.create_identifier(self.span_from(param_start), &value);
                params = self.builder.create_function_parameters(&[binding], &[NULL_HANDLE], &[false]);
                function_length = 1;
            } else {
                self.load_state();
                return None;
            }
        }

        // Check for =>
        if !self.match_token(TokenType::Arrow) || self.current_token.trivia_has_line_terminator {
            self.load_state();
            return None;
        }
        self.consume(); // consume =>

        // Discard saved state - we're committed to arrow function
        self.discard_saved_state();

        let kind = if is_async { FunctionKind::Async as u8 } else { FunctionKind::Normal as u8 };

        if self.match_token(TokenType::CurlyOpen) {
            let body = self.parse_function_body(is_async, false);

            let span = self.span_from(start);
            Some(self.builder.create_function_expression(
                span, NULL_HANDLE,
                start.2, self.position().2 - start.2,
                body.0, params, function_length, kind,
                self.strict_mode || body.1, true,
                true, false, false, false,
            ))
        } else {
            // Expression body
            let body = self.builder.create_function_body(self.span_from(start));
            let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
            let return_stmt = self.builder.create_return_statement(self.span_from(start), expr);
            self.builder.scope_node_append(body, return_stmt);

            let span = self.span_from(start);
            Some(self.builder.create_function_expression(
                span, NULL_HANDLE,
                start.2, self.position().2 - start.2,
                body, params, function_length, kind,
                self.strict_mode, true,
                true, false, false, false,
            ))
        }
    }

    /// Parse a method definition for object/class.
    pub(crate) fn parse_method_definition(&mut self, is_async: bool, is_generator: bool, _is_getter: bool, _is_setter: bool) -> NodeHandle {
        let start = self.position();

        let kind = match (is_async, is_generator) {
            (true, true) => FunctionKind::AsyncGenerator as u8,
            (true, false) => FunctionKind::Async as u8,
            (false, true) => FunctionKind::Generator as u8,
            (false, false) => FunctionKind::Normal as u8,
        };

        let (params, function_length) = self.parse_formal_parameters();

        let body = self.parse_function_body(is_async, is_generator);

        let span = self.span_from(start);
        self.builder.create_function_expression(
            span, NULL_HANDLE,
            start.2, self.position().2 - start.2,
            body.0, params, function_length, kind,
            self.strict_mode || body.1, false,
            true, false, false, true,
        )
    }
}

// === Helpers ===

fn hex_digit(c: u16) -> Option<u16> {
    match c {
        0x30..=0x39 => Some(c - 0x30),       // '0'-'9'
        0x41..=0x46 => Some(c - 0x41 + 10),   // 'A'-'F'
        0x61..=0x66 => Some(c - 0x61 + 10),   // 'a'-'f'
        _ => None,
    }
}

fn token_to_binary_op(tt: TokenType) -> u8 {
    match tt {
        TokenType::Plus => 0,      // Addition
        TokenType::Minus => 1,     // Subtraction
        TokenType::Asterisk => 2,  // Multiplication
        TokenType::Slash => 3,     // Division
        TokenType::Percent => 4,   // Modulo
        TokenType::DoubleAsterisk => 5, // Exponentiation
        TokenType::EqualsEquals => 6,   // LooselyEquals
        TokenType::ExclamationMarkEquals => 7, // LooselyInequals
        TokenType::EqualsEqualsEquals => 8,    // StrictlyEquals
        TokenType::ExclamationMarkEqualsEquals => 9, // StrictlyInequals
        TokenType::GreaterThan => 10,   // GreaterThan
        TokenType::GreaterThanEquals => 11, // GreaterThanEquals
        TokenType::LessThan => 12,      // LessThan
        TokenType::LessThanEquals => 13, // LessThanEquals
        TokenType::Ampersand => 14,     // BitwiseAnd
        TokenType::Pipe => 15,          // BitwiseOr
        TokenType::Caret => 16,         // BitwiseXor
        TokenType::ShiftLeft => 17,     // LeftShift
        TokenType::ShiftRight => 18,    // RightShift
        TokenType::UnsignedShiftRight => 19, // UnsignedRightShift
        TokenType::In => 20,            // In
        TokenType::Instanceof => 21,    // InstanceOf
        _ => 0,
    }
}

fn token_to_assignment_op(tt: TokenType) -> u8 {
    match tt {
        TokenType::Equals => 0,           // Assignment
        TokenType::PlusEquals => 1,       // AdditionAssignment
        TokenType::MinusEquals => 2,      // SubtractionAssignment
        TokenType::AsteriskEquals => 3,   // MultiplicationAssignment
        TokenType::SlashEquals => 4,      // DivisionAssignment
        TokenType::PercentEquals => 5,    // ModuloAssignment
        TokenType::DoubleAsteriskEquals => 6, // ExponentiationAssignment
        TokenType::AmpersandEquals => 7,  // BitwiseAndAssignment
        TokenType::PipeEquals => 8,       // BitwiseOrAssignment
        TokenType::CaretEquals => 9,      // BitwiseXorAssignment
        TokenType::ShiftLeftEquals => 10, // LeftShiftAssignment
        TokenType::ShiftRightEquals => 11, // RightShiftAssignment
        TokenType::UnsignedShiftRightEquals => 12, // UnsignedRightShiftAssignment
        TokenType::DoubleAmpersandEquals => 13, // AndAssignment
        TokenType::DoublePipeEquals => 14,     // OrAssignment
        TokenType::DoubleQuestionMarkEquals => 15, // NullishAssignment
        _ => 0,
    }
}

fn parse_numeric_value(value: &[u16]) -> f64 {
    // Convert UTF-16 to ASCII string for parsing
    let s: String = value.iter().filter(|&&c| c != '_' as u16).map(|&c| c as u8 as char).collect();

    if s.starts_with("0x") || s.starts_with("0X") {
        i64::from_str_radix(&s[2..], 16).unwrap_or(0) as f64
    } else if s.starts_with("0o") || s.starts_with("0O") {
        i64::from_str_radix(&s[2..], 8).unwrap_or(0) as f64
    } else if s.starts_with("0b") || s.starts_with("0B") {
        i64::from_str_radix(&s[2..], 2).unwrap_or(0) as f64
    } else {
        s.parse::<f64>().unwrap_or(f64::NAN)
    }
}
