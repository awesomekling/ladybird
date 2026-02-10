/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! JavaScript parser: recursive descent with precedence climbing.

use std::collections::HashMap;

use crate::ast_bridge::{AstBuilder, NodeHandle, NULL_HANDLE, SourceCodeHandle, Span};
use crate::lexer::Lexer;
use crate::scope_collector::ScopeCollector;
use crate::token::{Token, TokenType};

mod declarations;
mod expressions;
mod statements;

/// Program type: Script or Module.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ProgramType {
    Script = 0,
    Module = 1,
}

/// DeclarationKind for variable declarations.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DeclarationKind {
    Var = 0,
    Let = 1,
    Const = 2,
}

/// FunctionKind for function node creation.
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

/// Tracks which tokens are forbidden in expression parsing to prevent
/// ambiguous mixing of &&/|| with ??.
#[derive(Clone, Copy)]
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
        Self {
            forbid_in: false,
            forbid_logical: false,
            forbid_coalesce: false,
            forbid_paren_open: false,
            forbid_question_mark_period: false,
            forbid_equals: false,
        }
    }

    pub fn with_in() -> Self {
        let mut f = Self::none();
        f.forbid_in = true;
        f
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

/// Parser state that can be saved/restored for backtracking.
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
pub struct Parser<'a> {
    lexer: Lexer<'a>,
    pub(crate) builder: AstBuilder,
    current_token: Token,
    errors: Vec<ParserError>,
    saved_states: Vec<SavedState>,
    program_type: ProgramType,
    source: &'a [u16],
    pub(crate) scope_collector: ScopeCollector,

    // Parser state flags (mirrors C++ ParserState)
    pub(crate) strict_mode: bool,
    pub(crate) allow_super_property_lookup: bool,
    pub(crate) allow_super_constructor_call: bool,
    pub(crate) in_function_context: bool,
    pub(crate) initiated_by_eval: bool,
    pub(crate) in_eval_function_context: bool,
    pub(crate) in_formal_parameter_context: bool,
    pub(crate) in_generator_function_context: bool,
    pub(crate) await_expression_is_valid: bool,
    pub(crate) in_arrow_function_context: bool,
    pub(crate) in_break_context: bool,
    pub(crate) in_continue_context: bool,
    pub(crate) string_legacy_octal_escape_sequence_in_scope: bool,
    pub(crate) in_class_field_initializer: bool,
    pub(crate) in_class_static_init_block: bool,
    pub(crate) function_might_need_arguments_object: bool,
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
            const ARGUMENTS: [u16; 9] = [b'a' as u16, b'r' as u16, b'g' as u16, b'u' as u16, b'm' as u16, b'e' as u16, b'n' as u16, b't' as u16, b's' as u16];
            const EVAL: [u16; 4] = [b'e' as u16, b'v' as u16, b'a' as u16, b'l' as u16];
            if value == ARGUMENTS || value == EVAL {
                self.function_might_need_arguments_object = true;
            }
        }
    }

    /// Consume an identifier token.
    pub(crate) fn consume_identifier(&mut self) -> Token {
        if self.match_identifier() {
            return self.consume();
        }
        self.expected("identifier");
        self.consume()
    }

    /// Consume an identifier reference (allows yield/await in some contexts).
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

    pub(crate) fn position(&self) -> (u32, u32, u32) {
        // Use the current token's end position for the "current position"
        (
            self.current_token.line_number,
            self.current_token.line_column,
            self.current_token.offset,
        )
    }

    /// Returns the offset just past the last consumed token, excluding the
    /// current token's leading trivia. Use for function source text end.
    pub(crate) fn source_text_end_offset(&self) -> u32 {
        self.current_token.offset - self.current_token.trivia_len
    }

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

    pub(crate) fn span_from(&self, start: (u32, u32, u32)) -> Span {
        let (line, col, offset) = self.position();
        Span {
            start_line: start.0,
            start_column: start.1,
            start_offset: start.2,
            end_line: line,
            end_column: col,
            end_offset: offset,
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
        if value == utf16_lit("await") {
            return self.program_type == ProgramType::Module || self.await_expression_is_valid;
        }
        if value == utf16_lit("async") {
            return false;
        }
        if value == utf16_lit("yield") {
            return self.in_generator_function_context;
        }
        if self.strict_mode {
            return true;
        }
        // Non-strict: only some escaped keywords are invalid
        let strict_reserved = [
            "implements", "interface", "package", "private", "protected", "public",
        ];
        for &kw in &strict_reserved {
            if value == utf16_lit(kw) {
                return false;
            }
        }
        true
    }

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
    pub(crate) fn synthesize_binding_pattern(&mut self, start: (u32, u32, u32)) -> NodeHandle {
        let saved_lexer = std::mem::replace(
            &mut self.lexer,
            Lexer::new_at_offset(self.source, start.2 as usize, start.0, start.1),
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
                value == utf16_lit("using")
            }
            _ => false,
        }
    }

    fn try_match_let_declaration(&mut self) -> bool {
        let next = self.next_token();
        if next.token_type.is_identifier_name() && self.token_value(&next) != utf16_lit("in") {
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

/// Check if a raw token value is 'use strict' or "use strict".
fn is_use_strict(raw: &[u16]) -> bool {
    raw == utf16_lit("'use strict'") || raw == utf16_lit("\"use strict\"")
}

/// Convert an ASCII string literal to a UTF-16 slice at compile time.
/// This is a runtime conversion helper (not actually compile-time in this impl).
fn utf16_lit(s: &str) -> Vec<u16> {
    s.encode_utf16().collect()
}
