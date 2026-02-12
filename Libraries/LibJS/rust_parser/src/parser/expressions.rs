/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Expression parsing: primary, secondary (binary/postfix), unary, and
//! precedence climbing.

use crate::ast::*;
use crate::parser::{Associativity, ForbiddenTokens, FunctionKind, Parser, Position};
use crate::token::{Token, TokenType};

impl<'a> Parser<'a> {
    // === Expression matching ===

    pub(crate) fn match_expression(&mut self) -> bool {
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
            TokenType::Yield => true,
            TokenType::Await => true,

            TokenType::Import => {
                let next = self.next_token();
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
            TokenType::Period | TokenType::BracketOpen | TokenType::ParenOpen | TokenType::QuestionMarkPeriod => true,
            TokenType::PlusPlus | TokenType::MinusMinus => !self.current_token.trivia_has_line_terminator,
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
            TokenType::QuestionMark => true,
            TokenType::Equals
            | TokenType::PlusEquals | TokenType::MinusEquals
            | TokenType::DoubleAsteriskEquals | TokenType::AsteriskEquals
            | TokenType::SlashEquals | TokenType::PercentEquals
            | TokenType::ShiftLeftEquals | TokenType::ShiftRightEquals
            | TokenType::UnsignedShiftRightEquals
            | TokenType::AmpersandEquals | TokenType::CaretEquals | TokenType::PipeEquals
            | TokenType::DoubleAmpersandEquals | TokenType::DoublePipeEquals
            | TokenType::DoubleQuestionMarkEquals => true,
            TokenType::TemplateLiteralStart => true,
            _ => false,
        }
    }

    // === Main expression parser ===

    pub(crate) fn parse_expression(&mut self, min_precedence: i32, associativity: Associativity, forbidden: ForbiddenTokens) -> Expr {
        if self.match_unary_prefixed_expression() {
            let start = self.position();
            let expr = self.parse_unary_prefixed_expression();

            if self.match_token(TokenType::DoubleAsterisk) {
                self.syntax_error("Unparenthesized unary expression can't appear on the left-hand side of '**'");
            }

            return self.continue_parse_expression(start, expr, min_precedence, associativity, forbidden);
        }

        let lhs_start = self.position();
        let (expr, should_continue) = self.parse_primary_expression();
        if !should_continue {
            return expr;
        }

        let expr = if self.match_token(TokenType::TemplateLiteralStart) {
            let tag_start = self.position();
            let template = self.parse_template_literal(true);
            self.expr(tag_start, Expression::TaggedTemplateLiteral {
                tag: Box::new(expr),
                template_literal: Box::new(template),
            })
        } else {
            expr
        };

        self.continue_parse_expression(lhs_start, expr, min_precedence, associativity, forbidden)
    }

    fn continue_parse_expression(
        &mut self,
        lhs_start: Position,
        mut expr: Expr,
        min_precedence: i32,
        associativity: Associativity,
        mut forbidden: ForbiddenTokens,
    ) -> Expr {
        while self.match_secondary_expression(&forbidden) {
            let new_precedence = Self::operator_precedence(self.current_token_type());
            if new_precedence < min_precedence {
                break;
            }
            if new_precedence == min_precedence && associativity == Associativity::Left {
                break;
            }

            let result = self.parse_secondary_expression(lhs_start, expr, new_precedence, forbidden);
            expr = result.0;
            forbidden = forbidden.merge(result.1);
        }

        if min_precedence <= 1 && self.match_token(TokenType::Comma) && forbidden.allows(TokenType::Comma) {
            let start = self.position();
            let mut expressions = vec![expr];
            while self.match_token(TokenType::Comma) {
                self.consume();
                expressions.push(self.parse_expression(2, Associativity::Right, forbidden));
            }
            self.last_parsed_identifier_is_eval = false;
            return self.expr(start, Expression::Sequence(expressions));
        }

        expr
    }

    // === Primary expression ===

    fn parse_primary_expression(&mut self) -> (Expr, bool) {
        self.last_parsed_identifier_is_eval = false;
        let start = self.position();
        let token = self.current_token().clone();

        match token.token_type {
            TokenType::ParenOpen => {
                let paren_start = self.position();
                self.consume_token(TokenType::ParenOpen);
                if let Some(arrow) = self.try_parse_arrow_function_expression_impl(true, false, Some(paren_start)) {
                    return (arrow, false);
                }
                if self.match_token(TokenType::ParenClose) {
                    self.syntax_error("Unexpected token )");
                    self.consume();
                    return (self.expr(start, Expression::Error), true);
                }
                let expr = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::ParenClose);
                (expr, true)
            }

            TokenType::This => {
                self.consume();
                self.scope_collector.set_uses_this();
                (self.expr(start, Expression::This), true)
            }

            TokenType::Class => {
                let expr = self.parse_class_expression(false);
                (expr, true)
            }

            TokenType::Super => {
                self.consume();
                if self.scope_collector.has_current_scope() {
                    self.scope_collector.set_uses_new_target();
                }
                if self.match_token(TokenType::ParenOpen) {
                    if !self.allow_super_constructor_call {
                        self.syntax_error("'super' keyword unexpected here");
                    }
                    let arguments = self.parse_arguments();
                    (self.expr(start, Expression::SuperCall(SuperCallData {
                        arguments,
                        is_synthetic: false,
                    })), true)
                } else if self.match_token(TokenType::Period) || self.match_token(TokenType::BracketOpen) {
                    if !self.allow_super_property_lookup {
                        self.syntax_error("'super' keyword unexpected here");
                    }
                    (self.expr(start, Expression::Super), true)
                } else {
                    self.syntax_error("'super' keyword unexpected here");
                    (self.expr(start, Expression::Super), true)
                }
            }

            TokenType::NumericLiteral => {
                let tok = self.consume_and_validate_numeric_literal();
                let value_str = self.token_value(&tok);
                let value = parse_numeric_value(value_str);
                (self.expr(start, Expression::NumericLiteral(value)), true)
            }

            TokenType::BigIntLiteral => {
                let tok = self.consume();
                let value = self.token_value(&tok);
                // Strip trailing 'n' from the token value.
                let digits = if value.last() == Some(&(b'n' as u16)) {
                    &value[..value.len() - 1]
                } else {
                    &value[..]
                };
                let value_utf8: String = digits.iter().map(|&c| c as u8 as char).collect();
                (self.expr(start, Expression::BigIntLiteral(value_utf8)), true)
            }

            TokenType::BoolLiteral => {
                let tok = self.consume();
                let value = self.token_value(&tok);
                let is_true = value == utf16!("true");
                (self.expr(start, Expression::BooleanLiteral(is_true)), true)
            }

            TokenType::StringLiteral => {
                let tok = self.consume();
                let (value, has_octal) = self.parse_string_value(&tok);
                if has_octal {
                    if self.strict_mode {
                        self.syntax_error("Octal escape sequence in string literal not allowed in strict mode");
                    } else {
                        self.string_legacy_octal_escape_sequence_in_scope = true;
                    }
                }
                (self.expr(start, Expression::StringLiteral(value)), true)
            }

            TokenType::NullLiteral => {
                self.consume();
                (self.expr(start, Expression::NullLiteral), true)
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
                (expr, true)
            }

            TokenType::Async => {
                let next = self.next_token();
                if next.token_type == TokenType::Function && !next.trivia_has_line_terminator {
                    let expr = self.parse_function_expression();
                    return (expr, true);
                }
                if let Some(arrow) = self.try_parse_arrow_function_expression(next.token_type == TokenType::ParenOpen, true) {
                    return (arrow, false);
                }
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                let id = self.make_identifier(start, value.clone());
                self.scope_collector.register_identifier(&*id as *const Identifier, &value, None);
                (self.expr(start, Expression::Identifier(id)), true)
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
                    self.consume();
                    let meta_token = self.current_token.clone();
                    self.consume_token(TokenType::Identifier);
                    let meta_utf16: [u16; 4] = [b'm' as u16, b'e' as u16, b't' as u16, b'a' as u16];
                    if self.token_original_value(&meta_token) != meta_utf16 {
                        self.syntax_error("Expected 'meta' after 'import.'");
                    }
                    if self.program_type != ProgramType::Module {
                        self.syntax_error("import.meta is only allowed in modules");
                    }
                    (self.expr(start, Expression::MetaProperty(MetaPropertyType::ImportMeta)), true)
                } else if self.match_token(TokenType::ParenOpen) {
                    self.consume();
                    let specifier = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                    let options = if self.match_token(TokenType::Comma) {
                        self.consume();
                        if self.match_token(TokenType::ParenClose) {
                            None
                        } else {
                            let opts = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                            if self.match_token(TokenType::Comma) {
                                self.consume();
                            }
                            Some(Box::new(opts))
                        }
                    } else {
                        None
                    };
                    self.consume_token(TokenType::ParenClose);
                    (self.expr(start, Expression::ImportCall {
                        specifier: Box::new(specifier),
                        options,
                    }), true)
                } else {
                    self.expected("'.' or '('");
                    (self.expr(start, Expression::Error), true)
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
                (self.expr(start, Expression::PrivateIdentifier(PrivateIdentifier {
                    range: self.range_from(start),
                    name: value,
                })), true)
            }

            TokenType::RegexLiteral => {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                let pattern = if value.len() >= 2 {
                    value[1..value.len() - 1].to_vec()
                } else {
                    value
                };
                let flags = if self.match_token(TokenType::RegexFlags) {
                    let ftok = self.consume();
                    self.token_value(&ftok).to_vec()
                } else {
                    Vec::new()
                };
                (self.expr(start, Expression::RegExpLiteral(RegExpLiteralData { pattern, flags })), true)
            }

            TokenType::Slash | TokenType::SlashEquals => {
                let tok = self.lexer.force_slash_as_regex();
                self.current_token = tok;
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                let pattern = if value.len() >= 2 {
                    value[1..value.len() - 1].to_vec()
                } else {
                    value
                };
                let flags = if self.match_token(TokenType::RegexFlags) {
                    let ftok = self.consume();
                    self.token_value(&ftok).to_vec()
                } else {
                    Vec::new()
                };
                (self.expr(start, Expression::RegExpLiteral(RegExpLiteralData { pattern, flags })), true)
            }

            _ => {
                if self.match_identifier() {
                    if let Some(arrow) = self.try_parse_arrow_function_expression(false, false) {
                        return (arrow, false);
                    }
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    if value == utf16!("eval") {
                        self.last_parsed_identifier_is_eval = true;
                    }
                    let id = self.make_identifier(start, value.clone());
                    self.scope_collector.register_identifier(&*id as *const Identifier, &value, None);
                    (self.expr(start, Expression::Identifier(id)), true)
                } else if self.match_token(TokenType::EscapedKeyword) {
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let id = self.make_identifier(start, value.clone());
                    self.scope_collector.register_identifier(&*id as *const Identifier, &value, None);
                    (self.expr(start, Expression::Identifier(id)), true)
                } else {
                    self.expected("expression");
                    self.consume();
                    (self.expr(start, Expression::Error), true)
                }
            }
        }
    }

    // === Secondary expression ===

    fn parse_secondary_expression(&mut self, lhs_start: Position, lhs: Expr, min_precedence: i32, forbidden: ForbiddenTokens) -> (Expr, ForbiddenTokens) {
        let callee_is_eval = self.last_parsed_identifier_is_eval;
        self.last_parsed_identifier_is_eval = false;
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
                (self.expr(start, Expression::Binary {
                    op,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }), ForbiddenTokens::none())
            }

            // === Logical operators ===
            TokenType::DoubleAmpersand => {
                self.consume();
                let new_forbidden = forbidden.forbid(&[TokenType::DoubleQuestionMark]);
                let rhs = self.parse_expression(min_precedence, Associativity::Left, new_forbidden);
                (self.expr(start, Expression::Logical {
                    op: LogicalOp::And,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }), new_forbidden)
            }
            TokenType::DoublePipe => {
                self.consume();
                let new_forbidden = forbidden.forbid(&[TokenType::DoubleQuestionMark]);
                let rhs = self.parse_expression(min_precedence, Associativity::Left, new_forbidden);
                (self.expr(start, Expression::Logical {
                    op: LogicalOp::Or,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }), new_forbidden)
            }
            TokenType::DoubleQuestionMark => {
                self.consume();
                let new_forbidden = forbidden.forbid(&[TokenType::DoubleAmpersand, TokenType::DoublePipe]);
                let rhs = self.parse_expression(min_precedence, Associativity::Left, new_forbidden);
                (self.expr(start, Expression::Logical {
                    op: LogicalOp::NullishCoalescing,
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                }), new_forbidden)
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
                if op == AssignmentOp::Assignment && (Self::is_object_expression(&lhs) || Self::is_array_expression(&lhs)) {
                    if let Some(binding_pattern) = self.synthesize_binding_pattern(lhs_start) {
                        self.consume();
                        let rhs = self.parse_expression(min_precedence, Associativity::Right, forbidden);
                        return (self.expr(lhs_start, Expression::Assignment {
                            op,
                            lhs: AssignmentLhs::Pattern(binding_pattern),
                            rhs: Box::new(rhs),
                        }), ForbiddenTokens::none());
                    }
                }
                let allow_call = !matches!(tt, TokenType::DoubleAmpersandEquals | TokenType::DoublePipeEquals | TokenType::DoubleQuestionMarkEquals);
                if !Self::is_simple_assignment_target(&lhs, allow_call) {
                    self.syntax_error("Invalid left-hand side in assignment");
                }
                self.consume();
                let rhs = self.parse_expression(min_precedence, Associativity::Right, forbidden);
                (self.expr(start, Expression::Assignment {
                    op,
                    lhs: AssignmentLhs::Expression(Box::new(lhs)),
                    rhs: Box::new(rhs),
                }), ForbiddenTokens::none())
            }

            // === Ternary ===
            TokenType::QuestionMark => {
                self.consume();
                let consequent = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::Colon);
                let alternate = self.parse_expression(2, Associativity::Right, forbidden);
                (self.expr(start, Expression::Conditional {
                    test: Box::new(lhs),
                    consequent: Box::new(consequent),
                    alternate: Box::new(alternate),
                }), ForbiddenTokens::none())
            }

            // === Member access ===
            TokenType::Period => {
                self.consume();
                if self.match_token(TokenType::PrivateIdentifier) {
                    let prop_start = self.position();
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let prop = self.expr(prop_start, Expression::PrivateIdentifier(PrivateIdentifier {
                        range: self.range_from(prop_start),
                        name: value,
                    }));
                    (self.expr(start, Expression::Member {
                        object: Box::new(lhs),
                        property: Box::new(prop),
                        computed: false,
                    }), ForbiddenTokens::none())
                } else if self.match_identifier_name() {
                    let prop_start = self.position();
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let prop = self.expr(prop_start, Expression::Identifier(
                        self.make_identifier(prop_start, value),
                    ));
                    (self.expr(start, Expression::Member {
                        object: Box::new(lhs),
                        property: Box::new(prop),
                        computed: false,
                    }), ForbiddenTokens::none())
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
                (self.expr(start, Expression::Member {
                    object: Box::new(lhs),
                    property: Box::new(prop),
                    computed: true,
                }), ForbiddenTokens::none())
            }

            // === Call ===
            TokenType::ParenOpen => {
                let expr = self.parse_call_expression(lhs, callee_is_eval);
                (expr, ForbiddenTokens::none())
            }

            // === Optional chaining ===
            TokenType::QuestionMarkPeriod => {
                let chain = self.parse_optional_chain(start, lhs);
                (chain, ForbiddenTokens::none())
            }

            // === Postfix ===
            TokenType::PlusPlus => {
                if !Self::is_simple_assignment_target(&lhs, true) {
                    self.syntax_error("Invalid left-hand side in postfix operation");
                }
                self.consume();
                (self.expr(start, Expression::Update {
                    op: UpdateOp::Increment,
                    argument: Box::new(lhs),
                    prefixed: false,
                }), ForbiddenTokens::none())
            }
            TokenType::MinusMinus => {
                if !Self::is_simple_assignment_target(&lhs, true) {
                    self.syntax_error("Invalid left-hand side in postfix operation");
                }
                self.consume();
                (self.expr(start, Expression::Update {
                    op: UpdateOp::Decrement,
                    argument: Box::new(lhs),
                    prefixed: false,
                }), ForbiddenTokens::none())
            }

            // === Tagged template literal ===
            TokenType::TemplateLiteralStart => {
                let template = self.parse_template_literal(true);
                (self.expr(start, Expression::TaggedTemplateLiteral {
                    tag: Box::new(lhs),
                    template_literal: Box::new(template),
                }), ForbiddenTokens::none())
            }

            _ => {
                self.expected("secondary expression");
                (lhs, ForbiddenTokens::none())
            }
        }
    }

    // === Unary prefix expression ===

    fn parse_unary_prefixed_expression(&mut self) -> Expr {
        let start = self.position();
        let tt = self.current_token_type();

        match tt {
            TokenType::PlusPlus => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                if !Self::is_simple_assignment_target(&expr, true) {
                    self.syntax_error("Invalid left-hand side in prefix operation");
                }
                self.expr(start, Expression::Update {
                    op: UpdateOp::Increment,
                    argument: Box::new(expr),
                    prefixed: true,
                })
            }
            TokenType::MinusMinus => {
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                if !Self::is_simple_assignment_target(&expr, true) {
                    self.syntax_error("Invalid left-hand side in prefix operation");
                }
                self.expr(start, Expression::Update {
                    op: UpdateOp::Decrement,
                    argument: Box::new(expr),
                    prefixed: true,
                })
            }
            TokenType::ExclamationMark | TokenType::Tilde | TokenType::Plus
            | TokenType::Minus | TokenType::Typeof | TokenType::Void => {
                let op = match tt {
                    TokenType::ExclamationMark => UnaryOp::Not,
                    TokenType::Tilde => UnaryOp::BitwiseNot,
                    TokenType::Plus => UnaryOp::Plus,
                    TokenType::Minus => UnaryOp::Minus,
                    TokenType::Typeof => UnaryOp::Typeof,
                    _ => UnaryOp::Void,
                };
                self.consume();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                self.expr(start, Expression::Unary {
                    op,
                    operand: Box::new(expr),
                })
            }
            TokenType::Delete => {
                self.consume();
                let rhs_start = self.position();
                let expr = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
                if self.strict_mode && Self::is_identifier(&expr) {
                    self.syntax_error_at("Delete of an unqualified identifier in strict mode.", rhs_start.line, rhs_start.column);
                }
                self.expr(start, Expression::Unary {
                    op: UnaryOp::Delete,
                    operand: Box::new(expr),
                })
            }
            _ => {
                self.expected("unary expression");
                self.consume();
                self.expr(start, Expression::Error)
            }
        }
    }

    // === new expression ===

    fn parse_new_expression(&mut self) -> Expr {
        let start = self.position();
        self.consume_token(TokenType::New);

        if self.match_token(TokenType::Period) {
            self.consume();
            self.consume_token(TokenType::Identifier);
            if !self.in_function_context && !self.in_eval_function_context && !self.in_class_static_init_block {
                self.syntax_error("'new.target' not allowed outside of a function");
            }
            if self.scope_collector.has_current_scope() {
                self.scope_collector.set_uses_new_target();
            }
            return self.expr(start, Expression::MetaProperty(MetaPropertyType::NewTarget));
        }

        let callee = if self.match_token(TokenType::New) {
            self.parse_new_expression()
        } else {
            let forbidden = ForbiddenTokens::none().forbid(&[TokenType::ParenOpen, TokenType::QuestionMarkPeriod]);
            self.parse_expression(19, Associativity::Right, forbidden)
        };

        if self.match_token(TokenType::ParenOpen) {
            let arguments = self.parse_arguments();
            self.expr(start, Expression::New(CallExpressionData {
                callee: Box::new(callee),
                arguments,
                is_parenthesized: false,
                is_inside_parens: false,
            }))
        } else {
            self.expr(start, Expression::New(CallExpressionData {
                callee: Box::new(callee),
                arguments: Vec::new(),
                is_parenthesized: false,
                is_inside_parens: false,
            }))
        }
    }

    // === Call expression ===

    pub(crate) fn parse_call_expression(&mut self, callee: Expr, callee_is_eval: bool) -> Expr {
        let start = self.position();
        let arguments = self.parse_arguments();
        if callee_is_eval {
            self.scope_collector.set_contains_direct_call_to_eval();
        }
        self.expr(start, Expression::Call(CallExpressionData {
            callee: Box::new(callee),
            arguments,
            is_parenthesized: false,
            is_inside_parens: false,
        }))
    }

    pub(crate) fn parse_arguments(&mut self) -> Vec<CallArgument> {
        self.consume_token(TokenType::ParenOpen);
        let mut args = Vec::new();

        while !self.match_token(TokenType::ParenClose) && !self.done() {
            let is_spread = if self.match_token(TokenType::TripleDot) {
                self.consume();
                true
            } else {
                false
            };
            let value = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
            args.push(CallArgument { value, is_spread });
            if !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();
        }

        self.consume_token(TokenType::ParenClose);
        args
    }

    // === Optional chaining ===

    fn parse_optional_chain(&mut self, start: Position, base: Expr) -> Expr {
        let mut references = Vec::new();

        loop {
            if self.match_token(TokenType::QuestionMarkPeriod) {
                self.consume();
                match self.current_token_type() {
                    TokenType::ParenOpen => {
                        let arguments = self.parse_arguments();
                        references.push(OptionalChainReference::Call {
                            arguments,
                            mode: OptionalChainMode::Optional,
                        });
                    }
                    TokenType::BracketOpen => {
                        self.consume();
                        let expression = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                        self.consume_token(TokenType::BracketClose);
                        references.push(OptionalChainReference::ComputedReference {
                            expression: Box::new(expression),
                            mode: OptionalChainMode::Optional,
                        });
                    }
                    TokenType::PrivateIdentifier => {
                        let prop_start = self.position();
                        let tok = self.consume();
                        let value = self.token_value(&tok).to_vec();
                        references.push(OptionalChainReference::PrivateMemberReference {
                            private_identifier: PrivateIdentifier {
                                range: self.range_from(prop_start),
                                name: value,
                            },
                            mode: OptionalChainMode::Optional,
                        });
                    }
                    TokenType::TemplateLiteralStart => {
                        self.syntax_error("Invalid tagged template literal after ?.");
                        break;
                    }
                    _ => {
                        if self.match_identifier_name() {
                            let prop_start = self.position();
                            let tok = self.consume();
                            let value = self.token_value(&tok).to_vec();
                            references.push(OptionalChainReference::MemberReference {
                                identifier: self.make_identifier(prop_start, value),
                                mode: OptionalChainMode::Optional,
                            });
                        } else {
                            self.syntax_error("Invalid optional chain reference after ?.");
                            break;
                        }
                    }
                }
            } else if self.match_token(TokenType::ParenOpen) {
                let arguments = self.parse_arguments();
                references.push(OptionalChainReference::Call {
                    arguments,
                    mode: OptionalChainMode::NotOptional,
                });
            } else if self.match_token(TokenType::Period) {
                self.consume();
                if self.match_token(TokenType::PrivateIdentifier) {
                    let prop_start = self.position();
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    references.push(OptionalChainReference::PrivateMemberReference {
                        private_identifier: PrivateIdentifier {
                            range: self.range_from(prop_start),
                            name: value,
                        },
                        mode: OptionalChainMode::NotOptional,
                    });
                } else if self.match_identifier_name() {
                    let prop_start = self.position();
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    references.push(OptionalChainReference::MemberReference {
                        identifier: self.make_identifier(prop_start, value),
                        mode: OptionalChainMode::NotOptional,
                    });
                } else {
                    self.expected("an identifier");
                    break;
                }
            } else if self.match_token(TokenType::TemplateLiteralStart) {
                self.syntax_error("Invalid tagged template literal after optional chain");
                break;
            } else if self.match_token(TokenType::BracketOpen) {
                self.consume();
                let expression = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::BracketClose);
                references.push(OptionalChainReference::ComputedReference {
                    expression: Box::new(expression),
                    mode: OptionalChainMode::NotOptional,
                });
            } else {
                break;
            }

            if self.done() {
                break;
            }
        }

        self.expr(start, Expression::OptionalChain {
            base: Box::new(base),
            references,
        })
    }

    // === Yield expression ===

    fn parse_yield_expression(&mut self) -> Expr {
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
            return self.expr(start, Expression::Yield {
                argument: None,
                is_yield_from: false,
            });
        }

        let is_yield_from = self.match_token(TokenType::Asterisk);
        if is_yield_from {
            self.consume();
        }

        let argument = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
        self.expr(start, Expression::Yield {
            argument: Some(Box::new(argument)),
            is_yield_from,
        })
    }

    // === Await expression ===

    fn parse_await_expression(&mut self) -> Expr {
        let start = self.position();
        self.consume_token(TokenType::Await);
        let argument = self.parse_expression(17, Associativity::Right, ForbiddenTokens::none());
        self.scope_collector.set_contains_await_expression();
        self.expr(start, Expression::Await(Box::new(argument)))
    }

    // === Object expression ===

    fn parse_object_expression(&mut self) -> Expr {
        let start = self.position();
        self.consume_token(TokenType::CurlyOpen);

        let mut properties = Vec::new();
        while !self.match_token(TokenType::CurlyClose) && !self.done() {
            if self.match_token(TokenType::TripleDot) {
                let spread_start = self.position();
                self.consume();
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                properties.push(ObjectProperty {
                    range: self.range_from(spread_start),
                    property_type: ObjectPropertyType::Spread,
                    key: Box::new(expr),
                    value: None,
                    is_method: false,
                });
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
        self.expr(start, Expression::Object(properties))
    }

    fn parse_object_property(&mut self) -> ObjectProperty {
        let start = self.position();
        let mut is_getter = false;
        let mut is_setter = false;
        let mut is_async = false;
        let mut is_generator = false;

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

        let (key, key_value, is_proto) = self.parse_property_key();

        // Method shorthand
        if self.match_token(TokenType::ParenOpen) {
            let func = self.parse_method_definition(is_async, is_generator, is_getter, is_setter, false, start);
            let prop_type = if is_getter { ObjectPropertyType::Getter } else if is_setter { ObjectPropertyType::Setter } else { ObjectPropertyType::KeyValue };
            return ObjectProperty {
                range: self.range_from(start),
                property_type: prop_type,
                key: Box::new(key),
                value: Some(Box::new(func)),
                is_method: true,
            };
        }

        // Getter/setter
        if is_getter || is_setter {
            let func = self.parse_method_definition(false, false, is_getter, is_setter, false, start);
            let prop_type = if is_getter { ObjectPropertyType::Getter } else { ObjectPropertyType::Setter };
            return ObjectProperty {
                range: self.range_from(start),
                property_type: prop_type,
                key: Box::new(key),
                value: Some(Box::new(func)),
                is_method: true,
            };
        }

        // key: value
        if self.match_token(TokenType::Colon) {
            self.consume();
            let value = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
            let prop_type = if is_proto { ObjectPropertyType::ProtoSetter } else { ObjectPropertyType::KeyValue };
            return ObjectProperty {
                range: self.range_from(start),
                property_type: prop_type,
                key: Box::new(key),
                value: Some(Box::new(value)),
                is_method: false,
            };
        }

        // Shorthand property: { x }
        if let Some(kv) = key_value {
            let id = self.make_identifier(start, kv);
            self.scope_collector.register_identifier(&*id as *const Identifier, &id.name, None);
            let value = self.expr(start, Expression::Identifier(id));
            return ObjectProperty {
                range: self.range_from(start),
                property_type: ObjectPropertyType::KeyValue,
                key: Box::new(key),
                value: Some(Box::new(value)),
                is_method: false,
            };
        }

        self.expected("':' or '('");
        ObjectProperty {
            range: self.range_from(start),
            property_type: ObjectPropertyType::KeyValue,
            key: Box::new(key),
            value: None,
            is_method: false,
        }
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

    pub(crate) fn parse_property_key(&mut self) -> (Expr, Option<Vec<u16>>, bool) {
        let proto_name = utf16!("__proto__");
        let start = self.position();
        match self.current_token_type() {
            TokenType::BracketOpen => {
                self.consume();
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                self.consume_token(TokenType::BracketClose);
                (expr, None, false)
            }
            TokenType::StringLiteral => {
                let tok = self.consume();
                let (value, has_octal) = self.parse_string_value(&tok);
                if has_octal {
                    if self.strict_mode {
                        self.syntax_error("Octal escape sequence in string literal not allowed in strict mode");
                    } else {
                        self.string_legacy_octal_escape_sequence_in_scope = true;
                    }
                }
                let is_proto = value == proto_name;
                (self.expr(start, Expression::StringLiteral(value.clone())), Some(value), is_proto)
            }
            TokenType::NumericLiteral => {
                let tok = self.consume_and_validate_numeric_literal();
                let value_str = self.token_value(&tok);
                let value = parse_numeric_value(value_str);
                (self.expr(start, Expression::NumericLiteral(value)), None, false)
            }
            TokenType::PrivateIdentifier => {
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                if value == utf16!("#constructor") {
                    self.syntax_error("Private property with name '#constructor' is not allowed");
                }
                (self.expr(start, Expression::PrivateIdentifier(PrivateIdentifier {
                    range: self.range_from(start),
                    name: value.clone(),
                })), Some(value), false)
            }
            _ => {
                if self.match_identifier_name() {
                    let tok = self.consume();
                    let value = self.token_value(&tok).to_vec();
                    let is_proto = value == proto_name;
                    let key = self.expr(start, Expression::StringLiteral(value.clone()));
                    (key, Some(value), is_proto)
                } else {
                    self.expected("property key");
                    self.consume();
                    (self.expr(start, Expression::StringLiteral(Vec::new())), None, false)
                }
            }
        }
    }

    // === Array expression ===

    fn parse_array_expression(&mut self) -> Expr {
        let start = self.position();
        self.consume_token(TokenType::BracketOpen);

        let mut elements: Vec<Option<Expr>> = Vec::new();
        while !self.match_token(TokenType::BracketClose) && !self.done() {
            if self.match_token(TokenType::Comma) {
                elements.push(None); // Hole
                self.consume();
                continue;
            }
            if self.match_token(TokenType::TripleDot) {
                let spread_start = self.position();
                self.consume();
                let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
                elements.push(Some(self.expr(spread_start, Expression::Spread(Box::new(expr)))));
            } else {
                elements.push(Some(self.parse_expression(2, Associativity::Right, ForbiddenTokens::none())));
            }
            if !self.match_token(TokenType::Comma) {
                break;
            }
            self.consume();
        }

        self.consume_token(TokenType::BracketClose);
        self.expr(start, Expression::Array(elements))
    }

    // === Template literal ===

    pub(crate) fn parse_template_literal(&mut self, is_tagged: bool) -> Expr {
        let start = self.position();
        self.consume_token(TokenType::TemplateLiteralStart);

        let mut expressions = Vec::new();
        let mut raw_strings = Vec::new();

        // Track whether we need to insert an empty string at the beginning.
        let needs_leading_empty = !self.match_token(TokenType::TemplateLiteralString);
        if needs_leading_empty {
            if is_tagged {
                raw_strings.push(Vec::new());
            }
            // Push empty cooked string for the leading position.
            expressions.push(self.expr(start, Expression::StringLiteral(Vec::new())));
        }

        // For non-tagged templates, we collect parts as expressions (alternating
        // string parts and interpolation expressions). For tagged templates, we
        // also collect raw strings separately.
        let mut _last_was_expr = needs_leading_empty;

        loop {
            if self.match_token(TokenType::TemplateLiteralEnd) {
                self.consume();
                break;
            }
            if self.match_token(TokenType::TemplateLiteralString) {
                let tok = self.consume();
                let raw = self.token_value(&tok);
                if is_tagged {
                    let raw_value = raw_template_value(raw);
                    raw_strings.push(raw_value);
                    match self.process_template_escape_sequences(raw) {
                        Some(cooked) => expressions.push(self.expr(start, Expression::StringLiteral(cooked))),
                        None => expressions.push(self.expr(start, Expression::NullLiteral)),
                    }
                } else {
                    let (value, has_octal) = self.process_escape_sequences(raw);
                    if has_octal {
                        self.syntax_error("Octal escape sequence not allowed in template literal");
                    }
                    expressions.push(self.expr(start, Expression::StringLiteral(value)));
                }
                _last_was_expr = false;
            } else if self.match_token(TokenType::TemplateLiteralExprStart) {
                self.consume();
                let expr = self.parse_expression(0, Associativity::Right, ForbiddenTokens::none());
                expressions.push(expr);
                self.consume_token(TokenType::TemplateLiteralExprEnd);
                // After an expression, if no template string follows, insert empty.
                if !self.match_token(TokenType::TemplateLiteralString) {
                    expressions.push(self.expr(start, Expression::StringLiteral(Vec::new())));
                    if is_tagged {
                        raw_strings.push(Vec::new());
                    }
                }
                _last_was_expr = true;
            } else if self.done() {
                self.expected("template literal end");
                break;
            } else {
                self.consume();
            }
        }

        self.expr(start, Expression::TemplateLiteral(TemplateLiteralData {
            expressions,
            raw_strings,
        }))
    }

    fn process_template_escape_sequences(&self, raw: &[u16]) -> Option<Vec<u16>> {
        let mut result = Vec::with_capacity(raw.len());
        let mut i = 0;
        while i < raw.len() {
            if raw[i] == b'\\' as u16 && i + 1 < raw.len() {
                i += 1;
                match raw[i] {
                    c if c == b'n' as u16 => result.push(b'\n' as u16),
                    c if c == b'r' as u16 => result.push(b'\r' as u16),
                    c if c == b't' as u16 => result.push(b'\t' as u16),
                    c if c == b'b' as u16 => result.push(8),
                    c if c == b'f' as u16 => result.push(12),
                    c if c == b'v' as u16 => result.push(11),
                    c if c == b'0' as u16 => {
                        if i + 1 < raw.len() && is_octal_char(raw[i + 1]) {
                            return None;
                        }
                        result.push(0);
                    }
                    c if c >= b'1' as u16 && c <= b'9' as u16 => {
                        return None;
                    }
                    c if c == b'x' as u16 => {
                        if i + 2 < raw.len() {
                            let hi = hex_digit(raw[i + 1]);
                            let lo = hex_digit(raw[i + 2]);
                            if let (Some(h), Some(l)) = (hi, lo) {
                                result.push(h * 16 + l);
                                i += 2;
                            } else {
                                return None;
                            }
                        } else {
                            return None;
                        }
                    }
                    c if c == b'u' as u16 => {
                        if i + 1 < raw.len() && raw[i + 1] == b'{' as u16 {
                            i += 2;
                            let mut code_point: u32 = 0;
                            let mut found_close = false;
                            while i < raw.len() {
                                if raw[i] == b'}' as u16 {
                                    found_close = true;
                                    break;
                                }
                                if let Some(d) = hex_digit(raw[i]) {
                                    code_point = code_point * 16 + d as u32;
                                } else {
                                    return None;
                                }
                                i += 1;
                            }
                            if !found_close || code_point > 0x10FFFF {
                                return None;
                            }
                            if code_point <= 0xFFFF {
                                result.push(code_point as u16);
                            } else {
                                let cp = code_point - 0x10000;
                                result.push((0xD800 + (cp >> 10)) as u16);
                                result.push((0xDC00 + (cp & 0x3FF)) as u16);
                            }
                        } else if i + 4 < raw.len() {
                            let mut code_point: u16 = 0;
                            for j in 1..=4 {
                                if let Some(d) = hex_digit(raw[i + j]) {
                                    code_point = code_point * 16 + d;
                                } else {
                                    return None;
                                }
                            }
                            result.push(code_point);
                            i += 4;
                        } else {
                            return None;
                        }
                    }
                    c if c == b'\n' as u16 => { /* line continuation */ }
                    c if c == b'\r' as u16 => {
                        if i + 1 < raw.len() && raw[i + 1] == b'\n' as u16 {
                            i += 1;
                        }
                    }
                    c if c == 0x2028 || c == 0x2029 => { /* skip LS/PS */ }
                    c => result.push(c),
                }
            } else if raw[i] == b'\r' as u16 {
                result.push(b'\n' as u16);
                if i + 1 < raw.len() && raw[i + 1] == b'\n' as u16 {
                    i += 1;
                }
            } else {
                result.push(raw[i]);
            }
            i += 1;
        }
        Some(result)
    }

    // === String value parsing ===

    pub(crate) fn parse_string_value(&self, token: &Token) -> (Vec<u16>, bool) {
        let raw = self.token_value(token);
        if raw.len() < 2 {
            return (Vec::new(), false);
        }
        let inner = &raw[1..raw.len() - 1];
        self.process_escape_sequences(inner)
    }

    pub(crate) fn process_escape_sequences(&self, inner: &[u16]) -> (Vec<u16>, bool) {
        let mut result = Vec::with_capacity(inner.len());
        let mut has_legacy_octal = false;
        let mut i = 0;
        while i < inner.len() {
            if inner[i] == b'\\' as u16 && i + 1 < inner.len() {
                i += 1;
                match inner[i] {
                    c if c == b'n' as u16 => result.push(b'\n' as u16),
                    c if c == b'r' as u16 => result.push(b'\r' as u16),
                    c if c == b't' as u16 => result.push(b'\t' as u16),
                    c if c == b'b' as u16 => result.push(8),
                    c if c == b'f' as u16 => result.push(12),
                    c if c == b'v' as u16 => result.push(11),
                    c if c == b'0' as u16 => {
                        if i + 1 < inner.len() && is_octal_char(inner[i + 1]) {
                            has_legacy_octal = true;
                            let (val, consumed) = parse_octal_escape(inner, i);
                            result.push(val);
                            i += consumed;
                        } else {
                            result.push(0);
                        }
                    }
                    c if c >= b'1' as u16 && c <= b'7' as u16 => {
                        has_legacy_octal = true;
                        let (val, consumed) = parse_octal_escape(inner, i);
                        result.push(val);
                        i += consumed;
                    }
                    c if c == b'8' as u16 || c == b'9' as u16 => {
                        has_legacy_octal = true;
                        result.push(c);
                    }
                    c if c == b'x' as u16 => {
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
                        if i + 1 < inner.len() && inner[i + 1] == b'{' as u16 {
                            i += 2;
                            let mut code_point: u32 = 0;
                            while i < inner.len() && inner[i] != b'}' as u16 {
                                if let Some(d) = hex_digit(inner[i]) {
                                    code_point = code_point * 16 + d as u32;
                                }
                                i += 1;
                            }
                            if code_point <= 0xFFFF {
                                result.push(code_point as u16);
                            } else {
                                let code_point = code_point - 0x10000;
                                result.push((0xD800 + (code_point >> 10)) as u16);
                                result.push((0xDC00 + (code_point & 0x3FF)) as u16);
                            }
                        } else if i + 4 < inner.len() {
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
                    c if c == b'\n' as u16 => { /* skip */ }
                    c if c == b'\r' as u16 => {
                        if i + 1 < inner.len() && inner[i + 1] == b'\n' as u16 {
                            i += 1;
                        }
                    }
                    c if c == 0x2028 || c == 0x2029 => { /* skip LS/PS */ }
                    c => result.push(c),
                }
            } else {
                result.push(inner[i]);
            }
            i += 1;
        }
        (result, has_legacy_octal)
    }

    // === Arrow function ===

    pub(crate) fn try_parse_arrow_function_expression(&mut self, expect_parens: bool, is_async: bool) -> Option<Expr> {
        self.try_parse_arrow_function_expression_impl(expect_parens, is_async, None)
    }

    pub(crate) fn try_parse_arrow_function_expression_impl(&mut self, expect_parens: bool, is_async: bool, source_start_override: Option<Position>) -> Option<Expr> {
        let start = self.position();

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

        let (params, function_length, param_info, is_simple);

        if expect_parens {
            let previous_errors = self.errors.len();
            let result = self.parse_formal_parameters_without_parens();
            params = result.0;
            function_length = result.1;
            param_info = result.2;
            is_simple = result.3;
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
            if self.match_identifier() {
                let param_start = self.position();
                let tok = self.consume();
                let value = self.token_value(&tok).to_vec();
                let binding = Box::new(Identifier::new(self.range_from(param_start), value.clone()));
                params = vec![FunctionParameter {
                    binding: FunctionParameterBinding::Identifier(binding),
                    default_value: None,
                    is_rest: false,
                }];
                function_length = 1;
                param_info = vec![(value, false, false)];
                is_simple = true;
            } else {
                self.load_state();
                return None;
            }
        }

        if !self.match_token(TokenType::Arrow) || self.current_token.trivia_has_line_terminator {
            self.load_state();
            return None;
        }
        self.consume(); // consume =>

        self.discard_saved_state();

        // Open function scope for arrow function.
        self.scope_collector.open_function_scope(None);
        self.scope_collector.set_is_arrow_function();

        // Register parameters with scope collector.
        self.register_function_params_with_scope(&params, &param_info);

        let fn_kind = if is_async { FunctionKind::Async } else { FunctionKind::Normal };
        let src_start = source_start_override.unwrap_or(start).offset;

        if self.match_token(TokenType::CurlyOpen) {
            let (body, has_use_strict, _insights) = self.parse_function_body(is_async, false, is_simple);

            // Close function scope.
            self.scope_collector.close_scope();

            if has_use_strict || fn_kind != FunctionKind::Normal {
                self.check_parameters_post_body(&param_info, has_use_strict, fn_kind);
            }

            Some(self.expr(start, Expression::Function(Box::new(FunctionData {
                name: None,
                source_text_start: src_start,
                source_text_end: self.source_text_end_offset(),
                body: Box::new(body),
                parameters: params,
                function_length,
                kind: fn_kind,
                is_strict_mode: self.strict_mode || has_use_strict,
                is_arrow_function: true,
                parsing_insights: FunctionParsingInsights::default(),
                is_hoisted: false,
            }))))
        } else {
            let body_start = self.position();
            let expr = self.parse_expression(2, Associativity::Right, ForbiddenTokens::none());
            let return_stmt = Stmt::new(self.range_from(body_start), Statement::Return(Some(Box::new(expr))));
            let mut scope = Box::new(ScopeData::with_children(vec![return_stmt]));
            self.scope_collector.set_scope_node(&mut *scope as *mut ScopeData);
            let body = Stmt::new(self.range_from(body_start), Statement::FunctionBody {
                scope,
                in_strict_mode: self.strict_mode,
            });

            // Close function scope.
            self.scope_collector.close_scope();

            Some(self.expr(start, Expression::Function(Box::new(FunctionData {
                name: None,
                source_text_start: src_start,
                source_text_end: self.source_text_end_offset(),
                body: Box::new(body),
                parameters: params,
                function_length,
                kind: fn_kind,
                is_strict_mode: self.strict_mode,
                is_arrow_function: true,
                parsing_insights: FunctionParsingInsights::default(),
                is_hoisted: false,
            }))))
        }
    }

    /// Parse a method definition for object/class.
    pub(crate) fn parse_method_definition(&mut self, is_async: bool, is_generator: bool, is_getter: bool, is_setter: bool, is_constructor: bool, source_text_start: Position) -> Expr {
        let start = self.position();

        let saved_might_need_arguments = self.function_might_need_arguments_object;
        self.function_might_need_arguments_object = false;

        let fn_kind = match (is_async, is_generator) {
            (true, true) => FunctionKind::AsyncGenerator,
            (true, false) => FunctionKind::Async,
            (false, true) => FunctionKind::Generator,
            (false, false) => FunctionKind::Normal,
        };

        // Open function scope for method.
        self.scope_collector.open_function_scope(None);

        let in_generator_before = self.in_generator_function_context;
        let await_before = self.await_expression_is_valid;
        self.in_generator_function_context = is_generator;
        self.await_expression_is_valid = is_async;

        let (params, function_length, param_info, is_simple) = self.parse_formal_parameters();

        // Register parameters with scope collector.
        self.register_function_params_with_scope(&params, &param_info);

        if is_getter && !param_info.is_empty() {
            self.syntax_error("Getter function must have no arguments");
        }
        if is_setter {
            if param_info.is_empty() || param_info.len() > 1 {
                self.syntax_error("Setter function must have one argument");
            } else if param_info[0].1 {
                self.syntax_error("Setter function must have one argument");
            }
        }

        self.in_generator_function_context = in_generator_before;
        self.await_expression_is_valid = await_before;

        let saved_allow_super_call = self.allow_super_constructor_call;
        if is_constructor && self.class_has_super_class {
            self.allow_super_constructor_call = true;
        } else {
            self.allow_super_constructor_call = false;
        }

        let (body, has_use_strict, _insights) = self.parse_function_body(is_async, is_generator, is_simple);
        self.allow_super_constructor_call = saved_allow_super_call;

        // Close function scope.
        self.scope_collector.close_scope();

        if has_use_strict || fn_kind != FunctionKind::Normal {
            self.check_parameters_post_body(&param_info, has_use_strict, fn_kind);
        }

        let might_need_arguments = self.function_might_need_arguments_object;
        self.function_might_need_arguments_object = saved_might_need_arguments;

        self.expr(start, Expression::Function(Box::new(FunctionData {
            name: None,
            source_text_start: source_text_start.offset,
            source_text_end: self.source_text_end_offset(),
            body: Box::new(body),
            parameters: params,
            function_length,
            kind: fn_kind,
            is_strict_mode: self.strict_mode || has_use_strict,
            is_arrow_function: false,
            parsing_insights: FunctionParsingInsights {
                might_need_arguments_object: might_need_arguments,
                ..FunctionParsingInsights::default()
            },
            is_hoisted: false,
        })))
    }
}

// === Helpers ===

fn hex_digit(c: u16) -> Option<u16> {
    match c {
        0x30..=0x39 => Some(c - 0x30),
        0x41..=0x46 => Some(c - 0x41 + 10),
        0x61..=0x66 => Some(c - 0x61 + 10),
        _ => None,
    }
}

fn is_octal_char(c: u16) -> bool {
    c >= b'0' as u16 && c <= b'7' as u16
}

fn parse_octal_escape(inner: &[u16], i: usize) -> (u16, usize) {
    let first = (inner[i] - b'0' as u16) as u32;
    let mut value = first;
    let mut consumed = 0;

    if i + 1 < inner.len() && is_octal_char(inner[i + 1]) {
        value = value * 8 + (inner[i + 1] - b'0' as u16) as u32;
        consumed = 1;

        if i + 2 < inner.len() && is_octal_char(inner[i + 2]) && first <= 3 {
            value = value * 8 + (inner[i + 2] - b'0' as u16) as u32;
            consumed = 2;
        }
    }
    (value as u16, consumed)
}

fn raw_template_value(raw: &[u16]) -> Vec<u16> {
    let mut result = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'\r' as u16 {
            result.push(b'\n' as u16);
            if i + 1 < raw.len() && raw[i + 1] == b'\n' as u16 {
                i += 1;
            }
        } else {
            result.push(raw[i]);
        }
        i += 1;
    }
    result
}

fn token_to_binary_op(tt: TokenType) -> BinaryOp {
    match tt {
        TokenType::Plus => BinaryOp::Addition,
        TokenType::Minus => BinaryOp::Subtraction,
        TokenType::Asterisk => BinaryOp::Multiplication,
        TokenType::Slash => BinaryOp::Division,
        TokenType::Percent => BinaryOp::Modulo,
        TokenType::DoubleAsterisk => BinaryOp::Exponentiation,
        TokenType::EqualsEqualsEquals => BinaryOp::StrictlyEquals,
        TokenType::ExclamationMarkEqualsEquals => BinaryOp::StrictlyInequals,
        TokenType::EqualsEquals => BinaryOp::LooselyEquals,
        TokenType::ExclamationMarkEquals => BinaryOp::LooselyInequals,
        TokenType::GreaterThan => BinaryOp::GreaterThan,
        TokenType::GreaterThanEquals => BinaryOp::GreaterThanEquals,
        TokenType::LessThan => BinaryOp::LessThan,
        TokenType::LessThanEquals => BinaryOp::LessThanEquals,
        TokenType::Ampersand => BinaryOp::BitwiseAnd,
        TokenType::Pipe => BinaryOp::BitwiseOr,
        TokenType::Caret => BinaryOp::BitwiseXor,
        TokenType::ShiftLeft => BinaryOp::LeftShift,
        TokenType::ShiftRight => BinaryOp::RightShift,
        TokenType::UnsignedShiftRight => BinaryOp::UnsignedRightShift,
        TokenType::In => BinaryOp::In,
        TokenType::Instanceof => BinaryOp::InstanceOf,
        _ => BinaryOp::Addition,
    }
}

fn token_to_assignment_op(tt: TokenType) -> AssignmentOp {
    match tt {
        TokenType::Equals => AssignmentOp::Assignment,
        TokenType::PlusEquals => AssignmentOp::AdditionAssignment,
        TokenType::MinusEquals => AssignmentOp::SubtractionAssignment,
        TokenType::AsteriskEquals => AssignmentOp::MultiplicationAssignment,
        TokenType::SlashEquals => AssignmentOp::DivisionAssignment,
        TokenType::PercentEquals => AssignmentOp::ModuloAssignment,
        TokenType::DoubleAsteriskEquals => AssignmentOp::ExponentiationAssignment,
        TokenType::AmpersandEquals => AssignmentOp::BitwiseAndAssignment,
        TokenType::PipeEquals => AssignmentOp::BitwiseOrAssignment,
        TokenType::CaretEquals => AssignmentOp::BitwiseXorAssignment,
        TokenType::ShiftLeftEquals => AssignmentOp::LeftShiftAssignment,
        TokenType::ShiftRightEquals => AssignmentOp::RightShiftAssignment,
        TokenType::UnsignedShiftRightEquals => AssignmentOp::UnsignedRightShiftAssignment,
        TokenType::DoubleAmpersandEquals => AssignmentOp::AndAssignment,
        TokenType::DoublePipeEquals => AssignmentOp::OrAssignment,
        TokenType::DoubleQuestionMarkEquals => AssignmentOp::NullishAssignment,
        _ => AssignmentOp::Assignment,
    }
}

pub(crate) fn parse_numeric_value(value: &[u16]) -> f64 {
    let s: String = value.iter().filter(|&&c| c != '_' as u16).map(|&c| c as u8 as char).collect();

    if s.starts_with("0x") || s.starts_with("0X") {
        parse_integer_with_radix(&s[2..], 16)
    } else if s.starts_with("0o") || s.starts_with("0O") {
        parse_integer_with_radix(&s[2..], 8)
    } else if s.starts_with("0b") || s.starts_with("0B") {
        parse_integer_with_radix(&s[2..], 2)
    } else if s.starts_with('0') && s.len() > 1 && s.as_bytes()[1].is_ascii_digit() {
        let digits = &s[1..];
        if digits.bytes().all(|b| b >= b'0' && b <= b'7') {
            parse_integer_with_radix(digits, 8)
        } else {
            s.parse::<f64>().unwrap_or(f64::NAN)
        }
    } else {
        s.parse::<f64>().unwrap_or(f64::NAN)
    }
}

fn parse_integer_with_radix(digits: &str, radix: u32) -> f64 {
    if let Ok(v) = u64::from_str_radix(digits, radix) {
        return v as f64;
    }
    let mut result: f64 = 0.0;
    for ch in digits.chars() {
        let digit = ch.to_digit(radix);
        if let Some(d) = digit {
            result = result * (radix as f64) + (d as f64);
        }
    }
    result
}
