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
//!
//! ## Scope tracking
//!
//! During parsing, the `ScopeCollector` builds a tree of scope records.
//! Each scope tracks variable declarations, function declarations, and
//! identifier references. After parsing completes, `scope_collector.analyze()`
//! runs bottom-up to resolve identifiers, propagate `eval` poisoning, and
//! hoist functions (including Annex B).
//!
//! ## ForbiddenTokens
//!
//! Some expression contexts restrict which operators are valid. For example,
//! the `in` operator is forbidden in for-loop headers (`for (x in ...)` is
//! for-in, not a comparison). `ForbiddenTokens` tracks these restrictions
//! and is threaded through expression parsing.

use std::collections::HashMap;

use crate::ast_bridge::{AstBuilder, NodeHandle, NULL_HANDLE, SourceCodeHandle, Span};
use crate::lexer::Lexer;
use crate::scope_collector::ScopeCollector;
use crate::token::{Token, TokenType};

mod declarations;
mod expressions;
mod statements;

/// A source position: line number, column, and byte offset.
#[derive(Clone, Copy)]
pub struct Position {
    pub line: u32,
    pub column: u32,
    pub offset: u32,
}

/// Program type: Script or Module.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProgramType {
    Script = 0,
    Module = 1,
}

/// Declaration kind for variable declarations.
/// Values must match C++ `DeclarationKind` enum (used across FFI).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Var = 0,
    Let = 1,
    Const = 2,
}

/// Function kind for function node creation.
/// Values must match C++ `FunctionKind` enum (used across FFI).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum FunctionKind {
    Normal = 0,
    Generator = 1,
    Async = 2,
    AsyncGenerator = 3,
}

/// Parsing insights collected from a function body's scope.
pub struct FunctionParsingInsights {
    pub uses_this: bool,
    pub uses_this_from_environment: bool,
    pub contains_direct_call_to_eval: bool,
}

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

/// Snapshot of parser state for speculative parsing (backtracking).
///
/// When the parser needs to try parsing a construct that might fail
/// (e.g., arrow function parameters), it calls `save_state()` to push
/// a `SavedState` onto the stack. If parsing fails, `load_state()`
/// restores everything — including the error list and lexer position.
/// If parsing succeeds, `discard_saved_state()` drops the snapshot.
struct SavedState {
    token: Token,
    errors_len: usize,
    strict_mode: bool,
    allow_super_property_lookup: bool,
    allow_super_constructor_call: bool,
    in_function_context: bool,
    in_formal_parameter_context: bool,
    in_generator_function_context: bool,
    await_expression_is_valid: bool,
    in_arrow_function_context: bool,
    in_break_context: bool,
    in_continue_context: bool,
    string_legacy_octal_escape_sequence_in_scope: bool,
    in_class_field_initializer: bool,
    in_class_static_init_block: bool,
    function_might_need_arguments_object: bool,
    previous_token_was_period: bool,
}

/// The main JavaScript parser.
///
/// Owns the lexer, AST builder (FFI to C++), and scope collector.
/// Parsing methods live in the `expressions`, `statements`, and
/// `declarations` submodules (all `impl Parser`).
pub struct Parser<'a> {
    /// Tokenizer that feeds us tokens one at a time.
    lexer: Lexer<'a>,
    /// FFI bridge to the C++ AST factory. Creates C++ AST nodes.
    pub(crate) builder: AstBuilder,
    /// The token currently being examined. `consume()` returns this
    /// and advances to the next token.
    current_token: Token,
    /// Syntax errors accumulated during parsing. On successful parse
    /// this is empty. Errors are reported to C++ after parsing.
    errors: Vec<ParserError>,
    /// Stack of saved states for speculative parsing (backtracking).
    saved_states: Vec<SavedState>,
    /// Whether we're parsing a Script or Module.
    program_type: ProgramType,
    /// The original UTF-16 source text. Used by `token_value()` to
    /// extract string slices for token values.
    source: &'a [u16],
    /// Builds a tree of scope records during parsing, then resolves
    /// identifiers and hoists functions in a post-parse analysis pass.
    pub(crate) scope_collector: ScopeCollector,

    // --- Parser state flags ---
    // These mirror the C++ Parser::ParserState fields. They track what
    // kind of syntactic context we're currently inside, which affects
    // what constructs are legal.
    pub(crate) strict_mode: bool,
    pub(crate) allow_super_property_lookup: bool,
    pub(crate) allow_super_constructor_call: bool,
    pub(crate) in_function_context: bool,
    pub(crate) initiated_by_eval: bool,
    #[allow(dead_code)]
    pub(crate) in_eval_function_context: bool,
    pub(crate) in_formal_parameter_context: bool,
    pub(crate) in_generator_function_context: bool,
    pub(crate) await_expression_is_valid: bool,
    pub(crate) in_arrow_function_context: bool,
    pub(crate) in_break_context: bool,
    pub(crate) in_continue_context: bool,
    /// Set when a string literal with a legacy octal escape (\1-\7, \8, \9)
    /// is parsed in non-strict mode. If a 'use strict' directive is later
    /// found in the same scope, this triggers a retroactive syntax error.
    pub(crate) string_legacy_octal_escape_sequence_in_scope: bool,
    pub(crate) in_class_field_initializer: bool,
    pub(crate) in_class_static_init_block: bool,
    /// Tracks whether the current function body references `arguments` or
    /// `eval` as a freestanding identifier (not after `.`). Each function
    /// saves and resets this flag before parsing its body, then reads the
    /// accumulated value. Arrow functions do NOT save/restore — they let
    /// the flag propagate to the enclosing function (since arrows don't
    /// have their own `arguments` object).
    pub(crate) function_might_need_arguments_object: bool,
    /// True when the previously consumed token was `.` — used by
    /// `check_arguments_or_eval()` to avoid flagging `obj.arguments`
    /// as a reference to the `arguments` identifier.
    pub(crate) previous_token_was_period: bool,

    /// Labels currently in scope. Value is Some(line, col) if a `continue`
    /// statement referenced this label, None otherwise.
    labels_in_scope: HashMap<Vec<u16>, Option<(u32, u32)>>,

    /// Set by try_parse_labelled_statement to propagate iteration-ness
    /// through nested labels (e.g., `a: b: for(...)`).
    last_inner_label_is_iteration: bool,

    /// Last function declaration name, set by parse_function_declaration.
    last_function_name: Vec<u16>,
    last_function_name_id: NodeHandle,

    /// Bound names collected during parse_binding_pattern.
    /// Caller drains this after calling parse_binding_pattern.
    pub(crate) pattern_bound_names: Vec<(Vec<u16>, NodeHandle)>,
    last_function_kind: FunctionKind,
    last_class_name: Vec<u16>,
    last_class_name_id: NodeHandle,

    /// Set by parse_primary_expression when the result is a bare Identifier("eval").
    /// Read and cleared by parse_secondary_expression for the ParenOpen (call) case.
    last_parsed_identifier_is_eval: bool,

    /// Set during synthesize_binding_pattern to allow MemberExpressions as binding targets.
    allow_member_expressions: bool,

    /// True while parsing a class body that has an `extends` clause.
    /// Used to enable `allow_super_constructor_call` for constructors.
    pub(crate) class_has_super_class: bool,

    /// Set by parse_variable_declaration when is_for_loop is true.
    /// Used by for-in/of parsing to validate constraints.
    pub(crate) for_loop_declaration_count: usize,
    pub(crate) for_loop_declaration_has_init: bool,
    pub(crate) for_loop_declaration_is_var: bool,
}

impl<'a> Parser<'a> {
    /// Create a new parser for the given UTF-16 source code.
    pub fn new(source: &'a [u16], source_code: SourceCodeHandle, program_type: ProgramType) -> Self {
        let mut lexer = Lexer::new(source, 1, 0);
        if program_type == ProgramType::Module {
            lexer.disallow_html_comments();
        }
        let first_token = lexer.next();
        let builder = AstBuilder::new(source_code);
        Self {
            lexer,
            builder,
            current_token: first_token,
            errors: Vec::new(),
            saved_states: Vec::new(),
            program_type,
            source,
            scope_collector: ScopeCollector::new(),
            strict_mode: false,
            allow_super_property_lookup: false,
            allow_super_constructor_call: false,
            in_function_context: false,
            initiated_by_eval: false,
            in_eval_function_context: false,
            in_formal_parameter_context: false,
            in_generator_function_context: false,
            await_expression_is_valid: false,
            in_arrow_function_context: false,
            in_break_context: false,
            in_continue_context: false,
            string_legacy_octal_escape_sequence_in_scope: false,
            in_class_field_initializer: false,
            in_class_static_init_block: false,
            function_might_need_arguments_object: false,
            previous_token_was_period: false,
            labels_in_scope: HashMap::new(),
            last_inner_label_is_iteration: false,
            last_function_name: Vec::new(),
            last_function_name_id: NULL_HANDLE,
            last_function_kind: FunctionKind::Normal,
            last_parsed_identifier_is_eval: false,
            last_class_name: Vec::new(),
            last_class_name_id: NULL_HANDLE,
            pattern_bound_names: Vec::new(),
            allow_member_expressions: false,
            class_has_super_class: false,
            for_loop_declaration_count: 0,
            for_loop_declaration_has_init: false,
            for_loop_declaration_is_var: false,
        }
    }

    // === Token access ===

    pub(crate) fn current_token(&self) -> &Token {
        &self.current_token
    }

    pub(crate) fn current_token_type(&self) -> TokenType {
        self.current_token.token_type
    }

    /// Check if the current token matches a specific type.
    pub(crate) fn match_token(&self, tt: TokenType) -> bool {
        self.current_token.token_type == tt
    }

    /// Check if parsing is complete (at EOF).
    pub(crate) fn done(&self) -> bool {
        self.match_token(TokenType::Eof)
    }

    // === Token consumption ===

    /// Consume the current token and advance to the next one.
    pub(crate) fn consume(&mut self) -> Token {
        let old = self.current_token.clone();
        self.check_arguments_or_eval(&old);
        self.previous_token_was_period = old.token_type == TokenType::Period;
        self.current_token = self.lexer.next();
        old
    }

    /// Consume a token of the expected type, or emit a syntax error.
    pub(crate) fn consume_token(&mut self, expected: TokenType) -> Token {
        if self.current_token.token_type != expected {
            self.expected(expected.name());
        }
        self.consume()
    }

    /// Consume and re-lex for regex if needed (when `/` or `/=` appears in expression position).
    #[allow(dead_code)]
    pub(crate) fn consume_and_allow_division(&mut self) -> Token {
        let old = self.current_token.clone();
        self.check_arguments_or_eval(&old);
        self.previous_token_was_period = old.token_type == TokenType::Period;
        self.current_token = self.lexer.next();
        old
    }

    /// Check if the token being consumed is a freestanding `arguments` or `eval` identifier.
    /// If so, mark the current function as potentially needing an arguments object.
    fn check_arguments_or_eval(&mut self, token: &Token) {
        if token.token_type == TokenType::Identifier && !self.previous_token_was_period {
            let value: &[u16] = if let Some(ref v) = token.identifier_value {
                v
            } else {
                let start = token.value_start as usize;
                let end = start + token.value_len as usize;
                if end <= self.source.len() { &self.source[start..end] } else { &[] }
            };
            if value == utf16!("arguments") || value == utf16!("eval") {
                self.function_might_need_arguments_object = true;
            }
        }
    }

    /// Consume an identifier token.
    #[allow(dead_code)]
    pub(crate) fn consume_identifier(&mut self) -> Token {
        if self.match_identifier() {
            return self.consume();
        }
        self.expected("identifier");
        self.consume()
    }

    /// Consume an identifier reference (allows yield/await in some contexts).
    #[allow(dead_code)]
    pub(crate) fn consume_identifier_reference(&mut self) -> Token {
        if self.match_identifier() {
            return self.consume();
        }
        // In non-strict mode, yield and await can be identifiers
        if !self.strict_mode {
            if self.match_token(TokenType::Yield) && !self.in_generator_function_context {
                return self.consume();
            }
            if self.match_token(TokenType::Await) && !self.await_expression_is_valid {
                return self.consume();
            }
        }
        self.expected("identifier");
        self.consume()
    }

    /// Consume a numeric literal and validate.
    pub(crate) fn consume_and_validate_numeric_literal(&mut self) -> Token {
        // TODO: Add validation for numeric literal (e.g., no legacy octal in strict mode)
        self.consume()
    }

    /// Consume a semicolon, or insert one automatically (ASI).
    pub(crate) fn consume_or_insert_semicolon(&mut self) {
        if self.match_token(TokenType::Semicolon) {
            self.consume();
            return;
        }
        // ASI: Insert semicolon if:
        // 1. There is a line terminator before the current token
        // 2. The current token is }
        // 3. The current token is EOF
        if self.current_token.trivia_has_line_terminator
            || self.match_token(TokenType::CurlyClose)
            || self.done()
        {
            return;
        }
        self.expected("semicolon");
    }

    // === Lookahead ===

    /// Peek at the next token without consuming the current one.
    pub(crate) fn next_token(&mut self) -> Token {
        // Save full lexer state, get next token, restore
        self.lexer.save_state();
        let token = self.lexer.next();
        self.lexer.load_state();
        token
    }

    // === Position / Span ===

    pub(crate) fn position(&self) -> Position {
        Position {
            line: self.current_token.line_number,
            column: self.current_token.line_column,
            offset: self.current_token.offset,
        }
    }

    /// Returns the offset just past the last consumed token, excluding the
    /// current token's leading trivia. Use for function source text end.
    pub(crate) fn source_text_end_offset(&self) -> u32 {
        self.current_token.offset - self.current_token.trivia_len
    }

    #[allow(dead_code)]
    pub(crate) fn token_span(&self, token: &Token) -> Span {
        Span {
            start_line: token.line_number,
            start_column: token.line_column,
            start_offset: token.offset,
            end_line: token.line_number,
            end_column: token.line_column + token.value_len,
            end_offset: token.offset + token.value_len,
        }
    }

    pub(crate) fn span_from(&self, start: Position) -> Span {
        let end = self.position();
        Span {
            start_line: start.line,
            start_column: start.column,
            start_offset: start.offset,
            end_line: end.line,
            end_column: end.column,
            end_offset: end.offset,
        }
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

    pub(crate) fn expected(&mut self, what: &str) {
        let msg = format!(
            "Unexpected token {}. Expected {}",
            self.current_token.token_type.name(),
            what
        );
        self.syntax_error(&msg);
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }

    // === State save/restore for backtracking ===

    pub(crate) fn save_state(&mut self) {
        self.lexer.save_state();
        self.saved_states.push(SavedState {
            token: self.current_token.clone(),
            errors_len: self.errors.len(),
            strict_mode: self.strict_mode,
            allow_super_property_lookup: self.allow_super_property_lookup,
            allow_super_constructor_call: self.allow_super_constructor_call,
            in_function_context: self.in_function_context,
            in_formal_parameter_context: self.in_formal_parameter_context,
            in_generator_function_context: self.in_generator_function_context,
            await_expression_is_valid: self.await_expression_is_valid,
            in_arrow_function_context: self.in_arrow_function_context,
            in_break_context: self.in_break_context,
            in_continue_context: self.in_continue_context,
            string_legacy_octal_escape_sequence_in_scope: self.string_legacy_octal_escape_sequence_in_scope,
            in_class_field_initializer: self.in_class_field_initializer,
            in_class_static_init_block: self.in_class_static_init_block,
            function_might_need_arguments_object: self.function_might_need_arguments_object,
            previous_token_was_period: self.previous_token_was_period,
        });
    }

    pub(crate) fn load_state(&mut self) {
        let state = self.saved_states.pop().expect("No saved state to restore");
        self.current_token = state.token;
        self.errors.truncate(state.errors_len);
        self.strict_mode = state.strict_mode;
        self.allow_super_property_lookup = state.allow_super_property_lookup;
        self.allow_super_constructor_call = state.allow_super_constructor_call;
        self.in_function_context = state.in_function_context;
        self.in_formal_parameter_context = state.in_formal_parameter_context;
        self.in_generator_function_context = state.in_generator_function_context;
        self.await_expression_is_valid = state.await_expression_is_valid;
        self.in_arrow_function_context = state.in_arrow_function_context;
        self.in_break_context = state.in_break_context;
        self.in_continue_context = state.in_continue_context;
        self.string_legacy_octal_escape_sequence_in_scope = state.string_legacy_octal_escape_sequence_in_scope;
        self.in_class_field_initializer = state.in_class_field_initializer;
        self.in_class_static_init_block = state.in_class_static_init_block;
        self.function_might_need_arguments_object = state.function_might_need_arguments_object;
        self.previous_token_was_period = state.previous_token_was_period;
        self.lexer.load_state();
    }

    pub(crate) fn discard_saved_state(&mut self) {
        self.saved_states.pop();
        self.lexer.saved_states.pop();
    }

    // === Token matching helpers ===

    /// Check if current token is an identifier (not a reserved word in current context).
    pub(crate) fn match_identifier(&self) -> bool {
        if self.current_token.token_type == TokenType::Identifier {
            return true;
        }
        if self.current_token.token_type == TokenType::EscapedKeyword {
            // Escaped keywords are valid identifiers in some contexts
            return !self.match_invalid_escaped_keyword();
        }
        // In non-strict mode, some keywords can be identifiers
        if self.current_token.token_type == TokenType::Let && !self.strict_mode {
            return true;
        }
        if self.current_token.token_type == TokenType::Yield && !self.strict_mode && !self.in_generator_function_context {
            return true;
        }
        if self.current_token.token_type == TokenType::Await && !self.await_expression_is_valid && self.program_type != ProgramType::Module {
            return true;
        }
        if self.current_token.token_type == TokenType::Async {
            return true;
        }
        false
    }

    pub(crate) fn match_identifier_name(&self) -> bool {
        self.current_token.token_type.is_identifier_name()
            || self.match_identifier()
    }

    fn match_invalid_escaped_keyword(&self) -> bool {
        if self.current_token.token_type != TokenType::EscapedKeyword {
            return false;
        }
        let value = self.token_value(&self.current_token);
        if value == utf16!("await") {
            return self.program_type == ProgramType::Module || self.await_expression_is_valid;
        }
        if value == utf16!("async") {
            return false;
        }
        if value == utf16!("yield") {
            return self.in_generator_function_context;
        }
        if self.strict_mode {
            return true;
        }
        // Non-strict: only some escaped keywords are invalid
        let non_strict_valid = [
            utf16!("implements"), utf16!("interface"), utf16!("package"),
            utf16!("private"), utf16!("protected"), utf16!("public"),
        ];
        for kw in &non_strict_valid {
            if value == *kw {
                return false;
            }
        }
        true
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

    /// Check if an identifier name is valid as an assignment target.
    /// In strict mode (or with force_strict), "eval" and "arguments" are forbidden,
    /// as are strict reserved words like "implements", "interface", etc.
    pub(crate) fn check_identifier_name_for_assignment_validity(&mut self, name: &[u16], force_strict: bool) {
        if self.strict_mode || force_strict {
            if name == utf16!("arguments") || name == utf16!("eval") {
                self.syntax_error("Binding pattern target may not be called 'arguments' or 'eval' in strict mode");
            } else if is_strict_reserved_word(name) {
                let name_str = String::from_utf16_lossy(name);
                self.syntax_error(&format!("Binding pattern target may not be called '{}' in strict mode", name_str));
            }
        }
    }

    /// Post-body check for function parameters when 'use strict' was found in the
    /// body or the function is a generator/async. Re-validates parameter names and
    /// checks for duplicates.
    pub(crate) fn check_parameters_post_body(
        &mut self,
        param_info: &[(Vec<u16>, NodeHandle, bool, bool)],
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
    /// For identifiers with unicode escape sequences, returns the decoded value.
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

    /// Extract the trivia of a token as a UTF-16 slice.
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

    /// Extract the original value (trivia + value) of a token.
    pub(crate) fn token_original_value(&self, token: &Token) -> &[u16] {
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
    /// Re-parse an already-parsed expression as a binding pattern.
    ///
    /// This is needed for destructuring assignment: `({ a, b } = obj)`.
    /// When the parser first sees `{ a, b }`, it parses it as an object
    /// expression. Only when `=` follows does it realize this was actually
    /// a destructuring pattern. At that point, we re-lex from the start
    /// of the expression and parse it as a binding pattern instead.
    ///
    /// The scope collector is shared (same parser instance), so identifiers
    /// registered by the re-parse are added to the current scope.
    pub(crate) fn synthesize_binding_pattern(&mut self, start: Position) -> NodeHandle {
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

        pattern
    }

    // === Main entry point ===

    /// Parse the complete program.
    pub fn parse_program(&mut self, starts_in_strict_mode: bool) -> NodeHandle {
        let start = self.position();
        let program = self.builder.create_program(self.span_from(start), self.program_type as u8);

        self.scope_collector.open_program_scope(program, self.program_type);

        if self.program_type == ProgramType::Script {
            self.parse_script(program, starts_in_strict_mode);
        } else {
            self.parse_module(program);
        }

        self.builder.scope_node_shrink_to_fit(program);
        self.scope_collector.close_scope();
        self.scope_collector.analyze(self.initiated_by_eval);
        program
    }

    fn parse_script(&mut self, program: NodeHandle, starts_in_strict_mode: bool) {
        let strict_before = self.strict_mode;
        if starts_in_strict_mode {
            self.strict_mode = true;
        }

        let has_use_strict = self.parse_directive(program);

        if self.strict_mode || has_use_strict {
            self.builder.scope_node_set_strict_mode(program);
            self.strict_mode = true;
        }

        self.parse_statement_list(program, true);
        if !self.done() {
            self.expected("statement or declaration");
            self.consume();
        }

        self.strict_mode = strict_before;
    }

    fn parse_module(&mut self, program: NodeHandle) {
        self.builder.scope_node_set_strict_mode(program);
        let strict_before = self.strict_mode;
        let await_before = self.await_expression_is_valid;
        self.strict_mode = true;
        self.await_expression_is_valid = true;

        while !self.done() {
            self.parse_statement_list(program, true);

            if self.done() {
                break;
            }

            if self.match_export_or_import() {
                if self.match_token(TokenType::Export) {
                    let stmt = self.parse_export_statement();
                    self.builder.scope_node_append(program, stmt);
                } else {
                    let stmt = self.parse_import_statement();
                    self.builder.scope_node_append(program, stmt);
                }
            } else {
                self.expected("statement or declaration");
                self.consume();
            }
        }

        self.strict_mode = strict_before;
        self.await_expression_is_valid = await_before;
    }

    fn parse_directive(&mut self, body: NodeHandle) -> bool {
        let mut found_use_strict = false;
        while !self.done() && self.match_token(TokenType::StringLiteral) {
            let raw_value = self.token_original_value(&self.current_token).to_vec();
            let statement = self.parse_statement(false);
            self.builder.scope_node_append(body, statement);

            // Check if the raw source was 'use strict' or "use strict"
            if is_use_strict(&raw_value) {
                found_use_strict = true;
                if self.string_legacy_octal_escape_sequence_in_scope {
                    self.syntax_error("Octal escape sequence in string literal not allowed in strict mode");
                }
                break;
            }
        }
        self.string_legacy_octal_escape_sequence_in_scope = false;
        found_use_strict
    }

    pub(crate) fn parse_statement_list(&mut self, output_node: NodeHandle, allow_labelled_functions: bool) {
        while !self.done() {
            if self.match_export_or_import() {
                break;
            }
            if self.match_declaration() {
                let decl = self.parse_declaration();
                self.builder.scope_node_append(output_node, decl);
            } else if self.match_statement() {
                let stmt = self.parse_statement(allow_labelled_functions);
                self.builder.scope_node_append(output_node, stmt);
            } else {
                break;
            }
        }
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
                if !self.strict_mode {
                    // In non-strict mode, `let` can be an identifier (e.g., label).
                    // Check lookahead to distinguish `let x` (declaration) from `let:` (label).
                    self.try_match_let_declaration()
                } else {
                    true
                }
            }
            TokenType::Async => {
                // async [no LineTerminator here] function
                let next = self.next_token();
                next.token_type == TokenType::Function && !next.trivia_has_line_terminator
            }
            TokenType::Identifier => {
                // "using" declaration
                let value = self.token_value(&self.current_token);
                value == utf16!("using")
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

    /// Check if the current token starts an iteration statement.
    fn match_iteration_start(&self) -> bool {
        matches!(self.current_token_type(),
            TokenType::For | TokenType::While | TokenType::Do)
    }

    pub(crate) fn match_export_or_import(&mut self) -> bool {
        if self.match_token(TokenType::Export) {
            return true;
        }
        if self.match_token(TokenType::Import) {
            // `import(` and `import.` are expressions, not import declarations
            let next = self.next_token();
            return next.token_type != TokenType::ParenOpen
                && next.token_type != TokenType::Period;
        }
        false
    }

    // === Operator precedence ===

    /// Returns a numeric precedence level for an operator token.
    /// Higher values bind tighter: `.` (20) > `*` (15) > `+` (14) > `,` (1).
    /// Used by the precedence climbing loop in `parse_expression()`.
    pub(crate) fn operator_precedence(tt: TokenType) -> i32 {
        match tt {
            // 20: Member access, call, optional chain (tightest binding)
            TokenType::Period | TokenType::BracketOpen | TokenType::ParenOpen | TokenType::QuestionMarkPeriod => 20,
            // 19: new (binds tighter than unary so `new Foo()` works)
            TokenType::New => 19,
            // 18: Postfix ++/--
            TokenType::PlusPlus | TokenType::MinusMinus => 18,
            // 17: Unary prefix (!, ~, typeof, void, delete, await)
            TokenType::ExclamationMark | TokenType::Tilde | TokenType::Typeof | TokenType::Void | TokenType::Delete | TokenType::Await => 17,
            // 16: Exponentiation (**)
            TokenType::DoubleAsterisk => 16,
            // 15: Multiplicative (*, /, %)
            TokenType::Asterisk | TokenType::Slash | TokenType::Percent => 15,
            // 14: Additive (+, -)
            TokenType::Plus | TokenType::Minus => 14,
            // 13: Bitwise shift (<<, >>, >>>)
            TokenType::ShiftLeft | TokenType::ShiftRight | TokenType::UnsignedShiftRight => 13,
            // 12: Relational (<, <=, >, >=, in, instanceof)
            TokenType::LessThan | TokenType::LessThanEquals | TokenType::GreaterThan | TokenType::GreaterThanEquals | TokenType::In | TokenType::Instanceof => 12,
            // 11: Equality (==, !=, ===, !==)
            TokenType::EqualsEquals | TokenType::ExclamationMarkEquals | TokenType::EqualsEqualsEquals | TokenType::ExclamationMarkEqualsEquals => 11,
            // 10: Bitwise AND (&)
            TokenType::Ampersand => 10,
            // 9: Bitwise XOR (^)
            TokenType::Caret => 9,
            // 8: Bitwise OR (|)
            TokenType::Pipe => 8,
            // 7: Nullish coalescing (??)
            TokenType::DoubleQuestionMark => 7,
            // 6: Logical AND (&&)
            TokenType::DoubleAmpersand => 6,
            // 5: Logical OR (||)
            TokenType::DoublePipe => 5,
            // 4: Conditional/ternary (?:)
            TokenType::QuestionMark => 4,
            // 3: Assignment (=, +=, -=, etc.)
            TokenType::Equals | TokenType::PlusEquals | TokenType::MinusEquals
            | TokenType::DoubleAsteriskEquals | TokenType::AsteriskEquals | TokenType::SlashEquals
            | TokenType::PercentEquals | TokenType::ShiftLeftEquals | TokenType::ShiftRightEquals
            | TokenType::UnsignedShiftRightEquals | TokenType::AmpersandEquals | TokenType::CaretEquals
            | TokenType::PipeEquals | TokenType::DoubleAmpersandEquals | TokenType::DoublePipeEquals
            | TokenType::DoubleQuestionMarkEquals => 3,
            // 2: yield
            TokenType::Yield => 2,
            // 1: Comma/sequence (loosest binding)
            TokenType::Comma => 1,
            // 0: Not an operator (stops the precedence climbing loop)
            _ => 0,
        }
    }

    /// Returns the associativity of an operator, which determines how
    /// equal-precedence operators are grouped. Most are left-associative
    /// (`a + b + c` = `(a + b) + c`). Assignment and exponentiation are
    /// right-associative (`a = b = c` = `a = (b = c)`).
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

/// Check if a raw token value is 'use strict' or "use strict".
fn is_use_strict(raw: &[u16]) -> bool {
    raw == utf16!("'use strict'") || raw == utf16!("\"use strict\"")
}

/// Check if a name is a strict-mode reserved word.
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

