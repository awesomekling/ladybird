/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! JavaScript parser: recursive descent with precedence climbing.
//!
//! This is the core parser module. It contains the `Parser` struct (parser
//! state + helpers) and delegates actual parsing to submodules:
//!
//! - `expressions` — `parse_expression()`, primary/secondary expressions
//! - `statements` — `parse_statement()`, control flow
//! - `declarations` — functions, classes, variables, import/export
//!
//! ## How parsing works
//!
//! The parser is a single-pass, recursive-descent parser. Expression parsing
//! uses precedence climbing (Pratt-style): `parse_expression(min_precedence)`
//! parses a primary expression, then loops consuming binary/postfix operators
//! whose precedence is >= `min_precedence`.
//!
//! The parser reads tokens one at a time from the Lexer. The "current token"
//! is always available via `self.current_token`. Calling `consume()` returns
//! the current token and advances to the next one.
//!
//! ## Backtracking
//!
//! Some constructs require speculative parsing (e.g., arrow functions:
//! `(a, b) =>` looks like a parenthesized expression until `=>` is seen).
//! The parser supports this via `save_state()` / `load_state()`, which
//! save and restore the full parser state including lexer position, current
//! token, error list, and all boolean flags.

use std::collections::HashMap;

use std::rc::Rc;

use crate::ast::{
    BindingPattern, Expr, Expression, Identifier,
    SourceRange, Stmt, Statement, ScopeData, ProgramData,
};
use crate::lexer::Lexer;
use crate::scope_collector::ScopeCollector;
use crate::token::{Token, TokenType};

mod declarations;
mod expressions;
mod statements;

// Re-export ast types used throughout the parser submodules.
pub use crate::ast::Position;
pub use crate::ast::DeclarationKind;
pub use crate::ast::FunctionKind;
pub use crate::ast::ProgramType;
pub use crate::ast::FunctionParsingInsights;

/// Associativity for operator precedence.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Associativity {
    Left,
    Right,
}

/// Tracks which tokens are forbidden in the current expression context.
///
/// This is threaded through `parse_expression()` to prevent ambiguity:
/// - `forbid_in`: in for-loop init position (`for (x in ...)` is for-in, not comparison)
/// - `forbid_logical/forbid_coalesce`: `&&`/`||` and `??` cannot be mixed without parens
/// - `forbid_paren_open`: prevents consuming `(` as call in `new Foo()` callee position
/// - `forbid_question_mark_period`: prevents `?.` in `new Foo?.bar`
/// - `forbid_equals`: prevents `=` from being consumed as assignment in certain contexts
#[derive(Clone, Copy, Default)]
pub struct ForbiddenTokens {
    pub forbid_in: bool,
    pub forbid_logical: bool,
    pub forbid_coalesce: bool,
    pub forbid_paren_open: bool,
    pub forbid_question_mark_period: bool,
    pub forbid_equals: bool,
}

impl ForbiddenTokens {
    pub fn none() -> Self {
        Self::default()
    }

    pub fn with_in() -> Self {
        Self { forbid_in: true, ..Self::default() }
    }

    pub fn allows(&self, token: TokenType) -> bool {
        match token {
            TokenType::In => !self.forbid_in,
            TokenType::DoubleAmpersand | TokenType::DoublePipe => !self.forbid_logical,
            TokenType::DoubleQuestionMark => !self.forbid_coalesce,
            TokenType::ParenOpen => !self.forbid_paren_open,
            TokenType::QuestionMarkPeriod => !self.forbid_question_mark_period,
            TokenType::Equals => !self.forbid_equals,
            _ => true,
        }
    }

    pub fn merge(&self, other: ForbiddenTokens) -> ForbiddenTokens {
        ForbiddenTokens {
            forbid_in: self.forbid_in || other.forbid_in,
            forbid_logical: self.forbid_logical || other.forbid_logical,
            forbid_coalesce: self.forbid_coalesce || other.forbid_coalesce,
            forbid_paren_open: self.forbid_paren_open || other.forbid_paren_open,
            forbid_question_mark_period: self.forbid_question_mark_period || other.forbid_question_mark_period,
            forbid_equals: self.forbid_equals || other.forbid_equals,
        }
    }

    pub fn forbid(&self, tokens: &[TokenType]) -> ForbiddenTokens {
        let mut result = *self;
        for &t in tokens {
            match t {
                TokenType::In => result.forbid_in = true,
                TokenType::DoubleAmpersand | TokenType::DoublePipe => result.forbid_logical = true,
                TokenType::DoubleQuestionMark => result.forbid_coalesce = true,
                TokenType::ParenOpen => result.forbid_paren_open = true,
                TokenType::QuestionMarkPeriod => result.forbid_question_mark_period = true,
                TokenType::Equals => result.forbid_equals = true,
                _ => {}
            }
        }
        result
    }
}

/// Parser error collected during parsing.
pub struct ParserError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

/// Boolean flags that are saved/restored during speculative parsing.
#[derive(Clone, Copy, Default)]
#[allow(dead_code)]
pub(crate) struct ParserFlags {
    pub strict_mode: bool,
    pub allow_super_property_lookup: bool,
    pub allow_super_constructor_call: bool,
    pub in_function_context: bool,
    pub in_formal_parameter_context: bool,
    pub in_generator_function_context: bool,
    pub await_expression_is_valid: bool,
    pub in_arrow_function_context: bool,
    pub in_break_context: bool,
    pub in_continue_context: bool,
    pub string_legacy_octal_escape_sequence_in_scope: bool,
    pub in_class_field_initializer: bool,
    pub in_class_static_init_block: bool,
    pub function_might_need_arguments_object: bool,
    pub previous_token_was_period: bool,
}

/// Snapshot of parser state for speculative parsing (backtracking).
struct SavedState {
    token: Token,
    errors_len: usize,
    flags: ParserFlags,
    scope_collector_state: (usize, Option<usize>, usize),
}

/// The main JavaScript parser.
///
/// Produces a Rust AST. Parsing methods live in the `expressions`,
/// `statements`, and `declarations` submodules (all `impl Parser`).
pub struct Parser<'a> {
    /// Tokenizer that feeds us tokens one at a time.
    lexer: Lexer<'a>,
    /// The token currently being examined. `consume()` returns this
    /// and advances to the next token.
    current_token: Token,
    /// Syntax errors accumulated during parsing.
    errors: Vec<ParserError>,
    /// Stack of saved states for speculative parsing (backtracking).
    saved_states: Vec<SavedState>,
    /// Whether we're parsing a Script or Module.
    program_type: ProgramType,
    /// The original UTF-16 source text.
    source: &'a [u16],

    // --- Parser state flags (saved/restored during speculative parsing) ---
    pub(crate) flags: ParserFlags,

    // --- Flags NOT saved/restored during speculative parsing ---
    pub(crate) initiated_by_eval: bool,
    #[allow(dead_code)]
    pub(crate) in_eval_function_context: bool,

    /// Labels currently in scope. Value is Some(line, col) if a `continue`
    /// statement referenced this label, None otherwise.
    labels_in_scope: HashMap<Vec<u16>, Option<(u32, u32)>>,

    /// Set by try_parse_labelled_statement to propagate iteration-ness
    /// through nested labels (e.g., `a: b: for(...)`).
    last_inner_label_is_iteration: bool,

    /// Last function declaration name, set by parse_function_declaration.
    last_function_name: Vec<u16>,
    last_function_kind: FunctionKind,
    last_class_name: Vec<u16>,

    /// Bound names collected during parse_binding_pattern.
    /// Caller drains this after calling parse_binding_pattern.
    /// Each entry is (name, identifier) — allows scope analysis to annotate
    /// binding pattern identifiers with local variable info.
    pub(crate) pattern_bound_names: Vec<(Vec<u16>, Rc<Identifier>)>,

    /// Set by parse_primary_expression when the result is a bare Identifier("eval").
    /// Read and cleared by parse_secondary_expression for the ParenOpen (call) case.
    last_parsed_identifier_is_eval: bool,

    /// Set during synthesize_binding_pattern to allow MemberExpressions as binding targets.
    allow_member_expressions: bool,

    /// True while parsing a class body that has an `extends` clause.
    pub(crate) class_has_super_class: bool,
    /// Depth counter for class bodies — used to reject `#name` outside classes.
    pub(crate) class_scope_depth: u32,
    pub(crate) has_default_export_name: bool,

    /// Set by parse_variable_declaration when is_for_loop is true.
    pub(crate) for_loop_declaration_count: usize,
    pub(crate) for_loop_declaration_has_init: bool,
    pub(crate) for_loop_declaration_is_var: bool,

    /// Scope collector for scope analysis (variable resolution, local optimization).
    pub scope_collector: ScopeCollector,
}

impl<'a> Parser<'a> {
    /// Create a new parser for the given UTF-16 source code.
    pub fn new(source: &'a [u16], program_type: ProgramType) -> Self {
        let mut lexer = Lexer::new(source, 1, 0);
        if program_type == ProgramType::Module {
            lexer.disallow_html_comments();
        }
        let first_token = lexer.next();
        Self {
            lexer,
            current_token: first_token,
            errors: Vec::new(),
            saved_states: Vec::new(),
            program_type,
            source,
            flags: ParserFlags::default(),
            initiated_by_eval: false,
            in_eval_function_context: false,
            labels_in_scope: HashMap::new(),
            last_inner_label_is_iteration: false,
            last_function_name: Vec::new(),
            last_function_kind: FunctionKind::Normal,
            last_parsed_identifier_is_eval: false,
            last_class_name: Vec::new(),
            pattern_bound_names: Vec::new(),
            allow_member_expressions: false,
            class_has_super_class: false,
            class_scope_depth: 0,
            has_default_export_name: false,
            for_loop_declaration_count: 0,
            for_loop_declaration_has_init: false,
            for_loop_declaration_is_var: false,
            scope_collector: ScopeCollector::new(),
        }
    }

    // === AST construction helpers ===

    pub(crate) fn range_from(&self, start: Position) -> SourceRange {
        let end = self.position();
        SourceRange {
            start: Position { line: start.line, column: start.column, offset: start.offset },
            end: Position { line: end.line, column: end.column, offset: end.offset },
        }
    }

    pub(crate) fn expr(&self, start: Position, expression: Expression) -> Expr {
        Expr::new(self.range_from(start), expression)
    }

    pub(crate) fn stmt(&self, start: Position, statement: Statement) -> Stmt {
        Stmt::new(self.range_from(start), statement)
    }

    pub(crate) fn make_identifier(&self, start: Position, name: Vec<u16>) -> Rc<Identifier> {
        Rc::new(Identifier::new(self.range_from(start), name))
    }

    /// Register function parameters with the scope collector.
    pub(crate) fn register_function_params_with_scope(
        &mut self,
        params: &[crate::ast::FunctionParameter],
        param_info: &[(Vec<u16>, bool, bool, Option<Rc<Identifier>>)],
    ) {
        use crate::ast::FunctionParameterBinding;
        let mut entries: Vec<(Vec<u16>, Option<Rc<Identifier>>, bool, bool)> = Vec::new();
        let mut info_idx = 0;
        for param in params {
            match &param.binding {
                FunctionParameterBinding::Identifier(id) => {
                    let (name, is_rest, is_from_pattern) = if info_idx < param_info.len() {
                        let pi = &param_info[info_idx];
                        info_idx += 1;
                        (pi.0.clone(), pi.1, pi.2)
                    } else {
                        (id.name.clone(), param.is_rest, false)
                    };
                    entries.push((name, Some(id.clone()), is_rest, is_from_pattern));
                }
                FunctionParameterBinding::BindingPattern(_pat) => {
                    // Pattern parameters have multiple bound names in param_info.
                    while info_idx < param_info.len() && param_info[info_idx].2 {
                        let pi = &param_info[info_idx];
                        entries.push((pi.0.clone(), pi.3.clone(), pi.1, true));
                        info_idx += 1;
                    }
                }
            }
        }
        self.scope_collector.set_function_parameters(&entries);
    }

    // === Token access ===

    pub(crate) fn current_token(&self) -> &Token {
        &self.current_token
    }

    pub(crate) fn current_token_type(&self) -> TokenType {
        self.current_token.token_type
    }

    pub(crate) fn match_token(&self, tt: TokenType) -> bool {
        self.current_token.token_type == tt
    }

    pub(crate) fn done(&self) -> bool {
        self.match_token(TokenType::Eof)
    }

    // === Token consumption ===

    pub(crate) fn consume(&mut self) -> Token {
        let old = std::mem::replace(&mut self.current_token, self.lexer.next());
        self.check_arguments_or_eval(&old);
        self.flags.previous_token_was_period = old.token_type == TokenType::Period;
        old
    }

    pub(crate) fn consume_token(&mut self, expected: TokenType) -> Token {
        if self.current_token.token_type != expected {
            self.expected(expected.name());
        }
        self.consume()
    }

    pub(crate) fn eat(&mut self, tt: TokenType) -> bool {
        if self.match_token(tt) {
            self.consume();
            true
        } else {
            false
        }
    }

    #[allow(dead_code)]
    pub(crate) fn consume_and_allow_division(&mut self) -> Token {
        let old = std::mem::replace(&mut self.current_token, self.lexer.next());
        self.check_arguments_or_eval(&old);
        self.flags.previous_token_was_period = old.token_type == TokenType::Period;
        old
    }

    fn check_arguments_or_eval(&mut self, token: &Token) {
        if token.token_type == TokenType::Identifier && !self.flags.previous_token_was_period {
            let value: &[u16] = if let Some(ref v) = token.identifier_value {
                v
            } else {
                let start = token.value_start as usize;
                let end = start + token.value_len as usize;
                if end <= self.source.len() { &self.source[start..end] } else { &[] }
            };
            if value == utf16!("arguments") {
                if self.flags.in_class_field_initializer {
                    self.syntax_error("'arguments' is not allowed in class field initializer");
                }
                self.flags.function_might_need_arguments_object = true;
                if !self.flags.strict_mode {
                    self.scope_collector.set_contains_access_to_arguments_object_in_non_strict_mode();
                }
            } else if value == utf16!("eval") {
                self.flags.function_might_need_arguments_object = true;
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn consume_identifier(&mut self) -> Token {
        if self.match_identifier() {
            return self.consume();
        }
        self.expected("identifier");
        self.consume()
    }

    #[allow(dead_code)]
    pub(crate) fn consume_identifier_reference(&mut self) -> Token {
        if self.match_identifier() {
            return self.consume();
        }
        if !self.flags.strict_mode {
            if self.match_token(TokenType::Yield) && !self.flags.in_generator_function_context {
                return self.consume();
            }
            if self.match_token(TokenType::Await) && !self.flags.await_expression_is_valid {
                return self.consume();
            }
        }
        self.expected("identifier");
        self.consume()
    }

    pub(crate) fn consume_and_validate_numeric_literal(&mut self) -> Token {
        let tok = self.consume();
        if self.flags.strict_mode {
            let value = self.token_value(&tok);
            if value.len() > 1 && value[0] == b'0' as u16
                && value[1] >= b'0' as u16 && value[1] <= b'9' as u16
            {
                self.syntax_error("Unprefixed octal number not allowed in strict mode");
            }
        }
        if self.match_identifier_name() && self.current_token.trivia_len == 0 {
            self.syntax_error("Numeric literal must not be immediately followed by identifier");
        }
        tok
    }

    pub(crate) fn consume_or_insert_semicolon(&mut self) {
        if self.match_token(TokenType::Semicolon) {
            self.consume();
            return;
        }
        if self.current_token.trivia_has_line_terminator
            || self.match_token(TokenType::CurlyClose)
            || self.done()
        {
            return;
        }
        self.expected("semicolon");
    }

    // === Lookahead ===

    pub(crate) fn next_token(&mut self) -> Token {
        self.lexer.save_state();
        let token = self.lexer.next();
        self.lexer.load_state();
        token
    }

    // === Position ===

    pub(crate) fn position(&self) -> Position {
        Position {
            line: self.current_token.line_number,
            column: self.current_token.line_column,
            offset: self.current_token.offset,
        }
    }

    pub(crate) fn source_text_end_offset(&self) -> u32 {
        self.current_token.offset - self.current_token.trivia_len
    }

    // === Error reporting ===

    pub(crate) fn syntax_error(&mut self, message: &str) {
        self.errors.push(ParserError {
            message: message.to_string(),
            line: self.current_token.line_number,
            column: self.current_token.line_column,
        });
    }

    pub(crate) fn syntax_error_at(&mut self, message: &str, line: u32, column: u32) {
        self.errors.push(ParserError {
            message: message.to_string(),
            line,
            column,
        });
    }

    pub(crate) fn syntax_error_at_position(&mut self, message: &str, pos: Position) {
        self.syntax_error_at(message, pos.line, pos.column);
    }

    pub(crate) fn expected(&mut self, what: &str) {
        let msg = format!(
            "Unexpected token {}. Expected {}",
            self.current_token.token_type.name(),
            what
        );
        self.syntax_error(&msg);
    }

    pub(crate) fn validate_regex_flags(&mut self, flags: &[u16]) {
        let valid_flags: &[u16] = &[b'd' as u16, b'g' as u16, b'i' as u16, b'm' as u16, b's' as u16, b'u' as u16, b'v' as u16, b'y' as u16];
        let mut seen = [false; 128];
        for &ch in flags {
            if ch >= 128 || !valid_flags.contains(&ch) {
                self.syntax_error(&format!("Invalid RegExp flag '{}'", char::from_u32(ch as u32).unwrap_or('?')));
                return;
            }
            if seen[ch as usize] {
                self.syntax_error(&format!("Repeated RegExp flag '{}'", char::from_u32(ch as u32).unwrap_or('?')));
                return;
            }
            seen[ch as usize] = true;
        }
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    pub fn error_messages(&self) -> Vec<String> {
        self.errors
            .iter()
            .map(|e| format!("{}:{}: {}", e.line, e.column, e.message))
            .collect()
    }

    // === State save/restore for backtracking ===

    pub(crate) fn save_state(&mut self) {
        self.lexer.save_state();
        self.saved_states.push(SavedState {
            token: self.current_token.clone(),
            errors_len: self.errors.len(),
            flags: self.flags,
            scope_collector_state: self.scope_collector.save_state(),
        });
    }

    pub(crate) fn load_state(&mut self) {
        let state = self.saved_states.pop().expect("No saved state to restore");
        self.current_token = state.token;
        self.errors.truncate(state.errors_len);
        self.flags = state.flags;
        self.scope_collector.load_state(state.scope_collector_state);
        self.lexer.load_state();
    }

    pub(crate) fn discard_saved_state(&mut self) {
        self.saved_states.pop();
        self.lexer.saved_states.pop();
    }

    // === Token matching helpers ===

    pub(crate) fn match_identifier(&self) -> bool {
        let tt = self.current_token.token_type;
        tt == TokenType::Identifier
            || (tt == TokenType::EscapedKeyword && !self.match_invalid_escaped_keyword())
            || (tt == TokenType::Let && !self.flags.strict_mode)
            || (tt == TokenType::Yield && !self.flags.strict_mode && !self.flags.in_generator_function_context)
            || (tt == TokenType::Await && !self.flags.await_expression_is_valid && self.program_type != ProgramType::Module)
            || tt == TokenType::Async
    }

    pub(crate) fn match_identifier_name(&self) -> bool {
        self.current_token.token_type.is_identifier_name()
            || self.match_identifier()
    }

    pub(crate) fn match_invalid_escaped_keyword(&self) -> bool {
        if self.current_token.token_type != TokenType::EscapedKeyword {
            return false;
        }
        let value = self.token_value(&self.current_token);
        if value == utf16!("await") {
            return self.program_type == ProgramType::Module || self.flags.await_expression_is_valid;
        }
        if value == utf16!("async") {
            return false;
        }
        if value == utf16!("yield") {
            return self.flags.in_generator_function_context;
        }
        if self.flags.strict_mode {
            return true;
        }
        // In non-strict mode, "let" and "static" are context-sensitive
        // keywords that are valid as escaped identifiers. All other
        // escaped keywords (break, for, etc.) are always invalid.
        value != utf16!("let") && value != utf16!("static")
    }

    #[allow(dead_code)]
    pub(crate) fn match_property_key(&self) -> bool {
        matches!(
            self.current_token_type(),
            TokenType::BracketOpen
                | TokenType::StringLiteral
                | TokenType::NumericLiteral
                | TokenType::BigIntLiteral
                | TokenType::PrivateIdentifier
        ) || self.match_identifier_name()
    }

    pub(crate) fn check_identifier_name_for_assignment_validity(&mut self, name: &[u16], force_strict: bool) {
        if self.flags.strict_mode || force_strict {
            if name == utf16!("arguments") || name == utf16!("eval") {
                self.syntax_error("Binding pattern target may not be called 'arguments' or 'eval' in strict mode");
            } else if is_strict_reserved_word(name) {
                let name_str = String::from_utf16_lossy(name);
                self.syntax_error(&format!("Binding pattern target may not be called '{}' in strict mode", name_str));
            }
        }
    }

    /// Post-body check for function parameters when 'use strict' was found in the
    /// body or the function is a generator/async.
    pub(crate) fn check_parameters_post_body(
        &mut self,
        param_info: &[(Vec<u16>, bool, bool, Option<Rc<Identifier>>)],
        force_strict: bool,
        _kind: FunctionKind,
    ) {
        let mut seen_names: Vec<&[u16]> = Vec::new();
        for (name, _, _, _) in param_info {
            if name.is_empty() {
                continue;
            }
            self.check_identifier_name_for_assignment_validity(name, force_strict);
            for &prev_name in &seen_names {
                if prev_name == name.as_slice() {
                    let name_str = String::from_utf16_lossy(name);
                    self.syntax_error(&format!(
                        "Duplicate parameter '{}' not allowed in strict mode",
                        name_str
                    ));
                    break;
                }
            }
            seen_names.push(name);
        }
    }

    /// Extract the value of a token as a UTF-16 slice from the source.
    pub(crate) fn token_value<'b>(&'b self, token: &'b Token) -> &'b [u16] {
        if let Some(ref value) = token.identifier_value {
            return value;
        }
        let start = token.value_start as usize;
        let end = start + token.value_len as usize;
        if end <= self.source.len() {
            &self.source[start..end]
        } else {
            &[]
        }
    }

    #[allow(dead_code)]
    pub(crate) fn token_trivia(&self, token: &Token) -> &[u16] {
        let start = token.trivia_start as usize;
        let end = start + token.trivia_len as usize;
        if end <= self.source.len() {
            &self.source[start..end]
        } else {
            &[]
        }
    }

    pub(crate) fn token_original_value(&self, token: &Token) -> &'a [u16] {
        let start = token.value_start as usize;
        let end = (token.value_start + token.value_len) as usize;
        if end <= self.source.len() {
            &self.source[start..end]
        } else {
            &[]
        }
    }

    /// Re-parse the source range starting at `start` as a binding pattern
    /// with member expressions allowed (for destructuring assignment patterns).
    pub(crate) fn synthesize_binding_pattern(&mut self, start: Position) -> Option<BindingPattern> {
        let saved_lexer = std::mem::replace(
            &mut self.lexer,
            Lexer::new_at_offset(self.source, start.offset as usize, start.line, start.column),
        );
        let saved_token = std::mem::replace(&mut self.current_token, Token::new(TokenType::Eof));
        let saved_allow = self.allow_member_expressions;

        self.current_token = self.lexer.next();
        self.allow_member_expressions = true;

        let pattern = self.parse_binding_pattern();

        self.lexer = saved_lexer;
        self.current_token = saved_token;
        self.allow_member_expressions = saved_allow;

        Some(pattern)
    }

    /// Check if a node is a valid simple assignment target.
    pub(crate) fn is_simple_assignment_target(expr: &Expr, allow_call_expression: bool) -> bool {
        matches!(&expr.inner,
            Expression::Identifier(_)
            | Expression::Member { .. }
        ) || (allow_call_expression && matches!(&expr.inner, Expression::Call(_)))
    }

    fn is_object_expression(expr: &Expr) -> bool {
        matches!(&expr.inner, Expression::Object(_))
    }

    fn is_array_expression(expr: &Expr) -> bool {
        matches!(&expr.inner, Expression::Array(_))
    }

    fn is_identifier(expr: &Expr) -> bool {
        matches!(&expr.inner, Expression::Identifier(_))
    }

    fn is_member_expression(expr: &Expr) -> bool {
        matches!(&expr.inner, Expression::Member { .. })
    }

    fn is_call_expression(expr: &Expr) -> bool {
        matches!(&expr.inner, Expression::Call(_))
    }

    // === Main entry point ===

    pub fn parse_program(&mut self, starts_in_strict_mode: bool) -> Stmt {
        let start = self.position();

        if self.program_type == ProgramType::Script {
            let (children, is_strict) = self.parse_script(starts_in_strict_mode);
            let scope = ScopeData::shared_with_children(children);
            // Scope was opened in parse_script via open_program_scope.
            // Now close it after children are set.
            self.scope_collector.set_scope_node(scope.clone());
            self.scope_collector.close_scope();
            self.stmt(start, Statement::Program(ProgramData {
                scope,
                program_type: ProgramType::Script,
                is_strict_mode: is_strict,
                has_top_level_await: false,
            }))
        } else {
            let (children, has_top_level_await) = self.parse_module();
            let scope = ScopeData::shared_with_children(children);
            self.scope_collector.set_scope_node(scope.clone());
            self.scope_collector.close_scope();
            self.stmt(start, Statement::Program(ProgramData {
                scope,
                program_type: ProgramType::Module,
                is_strict_mode: true,
                has_top_level_await,
            }))
        }
    }

    fn parse_script(&mut self, starts_in_strict_mode: bool) -> (Vec<Stmt>, bool) {
        // Open program scope — will be closed in parse_program after ScopeData is created.
        self.scope_collector.open_program_scope(ProgramType::Script);

        let strict_before = self.flags.strict_mode;
        if starts_in_strict_mode {
            self.flags.strict_mode = true;
        }

        let (has_use_strict, mut children) = self.parse_directive();

        if self.flags.strict_mode || has_use_strict {
            self.flags.strict_mode = true;
        }

        children.extend(self.parse_statement_list(true));
        if !self.done() {
            self.expected("statement or declaration");
            self.consume();
        }

        let is_strict = self.flags.strict_mode;
        self.flags.strict_mode = strict_before;
        (children, is_strict)
    }

    fn parse_module(&mut self) -> (Vec<Stmt>, bool) {
        // Open program scope — will be closed in parse_program after ScopeData is created.
        self.scope_collector.open_program_scope(ProgramType::Module);

        let strict_before = self.flags.strict_mode;
        let await_before = self.flags.await_expression_is_valid;
        self.flags.strict_mode = true;
        self.flags.await_expression_is_valid = true;

        let mut children = Vec::new();

        while !self.done() {
            children.extend(self.parse_statement_list(true));

            if self.done() {
                break;
            }

            if self.match_export_or_import() {
                if self.match_token(TokenType::Export) {
                    children.push(self.parse_export_statement());
                } else {
                    children.push(self.parse_import_statement());
                }
            } else {
                self.expected("statement or declaration");
                self.consume();
            }
        }

        self.flags.strict_mode = strict_before;
        self.flags.await_expression_is_valid = await_before;
        // has_top_level_await would need scope analysis to determine correctly.
        (children, false)
    }

    pub(crate) fn parse_directive(&mut self) -> (bool, Vec<Stmt>) {
        let mut found_use_strict = false;
        let mut stmts = Vec::new();
        while !self.done() && self.match_token(TokenType::StringLiteral) {
            let raw_value = self.token_original_value(&self.current_token);
            let statement = self.parse_statement(false);
            stmts.push(statement);

            if is_use_strict(raw_value) {
                found_use_strict = true;
                if self.flags.string_legacy_octal_escape_sequence_in_scope {
                    self.syntax_error("Octal escape sequence in string literal not allowed in strict mode");
                }
                break;
            }
        }
        self.flags.string_legacy_octal_escape_sequence_in_scope = false;
        (found_use_strict, stmts)
    }

    pub(crate) fn parse_statement_list(&mut self, allow_labelled_functions: bool) -> Vec<Stmt> {
        let mut stmts = Vec::new();
        while !self.done() {
            if self.match_export_or_import() {
                break;
            }
            if self.match_declaration() {
                stmts.push(self.parse_declaration());
            } else if self.match_statement() {
                stmts.push(self.parse_statement(allow_labelled_functions));
            } else {
                break;
            }
        }
        stmts
    }

    pub(crate) fn match_statement(&mut self) -> bool {
        matches!(
            self.current_token_type(),
            TokenType::CurlyOpen
                | TokenType::Return
                | TokenType::Var
                | TokenType::For
                | TokenType::If
                | TokenType::Throw
                | TokenType::Try
                | TokenType::Break
                | TokenType::Continue
                | TokenType::Switch
                | TokenType::Do
                | TokenType::While
                | TokenType::With
                | TokenType::Debugger
                | TokenType::Semicolon
        ) || self.match_expression()
    }

    pub(crate) fn match_declaration(&mut self) -> bool {
        match self.current_token_type() {
            TokenType::Function | TokenType::Class | TokenType::Const => true,
            TokenType::Let => {
                if !self.flags.strict_mode {
                    self.try_match_let_declaration()
                } else {
                    true
                }
            }
            TokenType::Async => {
                let next = self.next_token();
                next.token_type == TokenType::Function && !next.trivia_has_line_terminator
            }
            TokenType::Identifier => {
                let value = self.token_value(&self.current_token);
                if value != utf16!("using") {
                    return false;
                }
                let next = self.next_token();
                !next.trivia_has_line_terminator && next.token_type.is_identifier_name()
            }
            _ => false,
        }
    }

    fn try_match_let_declaration(&mut self) -> bool {
        let next = self.next_token();
        if next.token_type.is_identifier_name() && self.token_value(&next) != utf16!("in") {
            return true;
        }
        if next.token_type == TokenType::CurlyOpen || next.token_type == TokenType::BracketOpen {
            return true;
        }
        false
    }

    fn match_iteration_start(&self) -> bool {
        matches!(self.current_token_type(),
            TokenType::For | TokenType::While | TokenType::Do)
    }

    pub(crate) fn match_export_or_import(&mut self) -> bool {
        if self.match_token(TokenType::Export) {
            return true;
        }
        if self.match_token(TokenType::Import) {
            let next = self.next_token();
            return next.token_type != TokenType::ParenOpen
                && next.token_type != TokenType::Period;
        }
        false
    }

    // === Operator precedence ===

    pub(crate) fn operator_precedence(tt: TokenType) -> i32 {
        match tt {
            TokenType::Period | TokenType::BracketOpen | TokenType::ParenOpen | TokenType::QuestionMarkPeriod => 20,
            TokenType::New => 19,
            TokenType::PlusPlus | TokenType::MinusMinus => 18,
            TokenType::ExclamationMark | TokenType::Tilde | TokenType::Typeof | TokenType::Void | TokenType::Delete | TokenType::Await => 17,
            TokenType::DoubleAsterisk => 16,
            TokenType::Asterisk | TokenType::Slash | TokenType::Percent => 15,
            TokenType::Plus | TokenType::Minus => 14,
            TokenType::ShiftLeft | TokenType::ShiftRight | TokenType::UnsignedShiftRight => 13,
            TokenType::LessThan | TokenType::LessThanEquals | TokenType::GreaterThan | TokenType::GreaterThanEquals | TokenType::In | TokenType::Instanceof => 12,
            TokenType::EqualsEquals | TokenType::ExclamationMarkEquals | TokenType::EqualsEqualsEquals | TokenType::ExclamationMarkEqualsEquals => 11,
            TokenType::Ampersand => 10,
            TokenType::Caret => 9,
            TokenType::Pipe => 8,
            TokenType::DoubleQuestionMark => 7,
            TokenType::DoubleAmpersand => 6,
            TokenType::DoublePipe => 5,
            TokenType::QuestionMark => 4,
            TokenType::Equals | TokenType::PlusEquals | TokenType::MinusEquals
            | TokenType::DoubleAsteriskEquals | TokenType::AsteriskEquals | TokenType::SlashEquals
            | TokenType::PercentEquals | TokenType::ShiftLeftEquals | TokenType::ShiftRightEquals
            | TokenType::UnsignedShiftRightEquals | TokenType::AmpersandEquals | TokenType::CaretEquals
            | TokenType::PipeEquals | TokenType::DoubleAmpersandEquals | TokenType::DoublePipeEquals
            | TokenType::DoubleQuestionMarkEquals => 3,
            TokenType::Yield => 2,
            TokenType::Comma => 1,
            _ => 0,
        }
    }

    pub(crate) fn operator_associativity(tt: TokenType) -> Associativity {
        match tt {
            TokenType::Period | TokenType::BracketOpen | TokenType::ParenOpen | TokenType::QuestionMarkPeriod
            | TokenType::Asterisk | TokenType::Slash | TokenType::Percent
            | TokenType::Plus | TokenType::Minus
            | TokenType::ShiftLeft | TokenType::ShiftRight | TokenType::UnsignedShiftRight
            | TokenType::LessThan | TokenType::LessThanEquals | TokenType::GreaterThan | TokenType::GreaterThanEquals
            | TokenType::In | TokenType::Instanceof
            | TokenType::EqualsEquals | TokenType::ExclamationMarkEquals | TokenType::EqualsEqualsEquals | TokenType::ExclamationMarkEqualsEquals
            | TokenType::Typeof | TokenType::Void | TokenType::Delete | TokenType::Await
            | TokenType::Ampersand | TokenType::Caret | TokenType::Pipe
            | TokenType::DoubleQuestionMark | TokenType::DoubleAmpersand | TokenType::DoublePipe
            | TokenType::Comma => Associativity::Left,
            _ => Associativity::Right,
        }
    }
}

// === Helpers ===

fn is_use_strict(raw: &[u16]) -> bool {
    raw == utf16!("'use strict'") || raw == utf16!("\"use strict\"")
}

fn is_strict_reserved_word(name: &[u16]) -> bool {
    name == utf16!("implements")
        || name == utf16!("interface")
        || name == utf16!("let")
        || name == utf16!("package")
        || name == utf16!("private")
        || name == utf16!("protected")
        || name == utf16!("public")
        || name == utf16!("static")
        || name == utf16!("yield")
}
