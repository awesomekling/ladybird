/*
 * Copyright (c) 2026, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Token types and Token struct for the lexer.
//!
//! The `TokenType` enum must have the same variants in the same order
//! as `ENUMERATE_JS_TOKENS` in Token.h (alphabetical) because token
//! type values are passed across the FFI boundary.

/// Token types. Order must match `ENUMERATE_JS_TOKENS` in Token.h.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum TokenType {
    Ampersand,
    AmpersandEquals,
    Arrow,
    Asterisk,
    AsteriskEquals,
    Async,
    Await,
    BigIntLiteral,
    BoolLiteral,
    BracketClose,
    BracketOpen,
    Break,
    Caret,
    CaretEquals,
    Case,
    Catch,
    Class,
    Colon,
    Comma,
    Const,
    Continue,
    CurlyClose,
    CurlyOpen,
    Debugger,
    Default,
    Delete,
    Do,
    DoubleAmpersand,
    DoubleAmpersandEquals,
    DoubleAsterisk,
    DoubleAsteriskEquals,
    DoublePipe,
    DoublePipeEquals,
    DoubleQuestionMark,
    DoubleQuestionMarkEquals,
    Else,
    Enum,
    Eof,
    Equals,
    EqualsEquals,
    EqualsEqualsEquals,
    EscapedKeyword,
    ExclamationMark,
    ExclamationMarkEquals,
    ExclamationMarkEqualsEquals,
    Export,
    Extends,
    Finally,
    For,
    Function,
    GreaterThan,
    GreaterThanEquals,
    Identifier,
    If,
    Implements,
    Import,
    In,
    Instanceof,
    Interface,
    Invalid,
    LessThan,
    LessThanEquals,
    Let,
    Minus,
    MinusEquals,
    MinusMinus,
    New,
    NullLiteral,
    NumericLiteral,
    Package,
    ParenClose,
    ParenOpen,
    Percent,
    PercentEquals,
    Period,
    Pipe,
    PipeEquals,
    Plus,
    PlusEquals,
    PlusPlus,
    Private,
    PrivateIdentifier,
    Protected,
    Public,
    QuestionMark,
    QuestionMarkPeriod,
    RegexFlags,
    RegexLiteral,
    Return,
    Semicolon,
    ShiftLeft,
    ShiftLeftEquals,
    ShiftRight,
    ShiftRightEquals,
    Slash,
    SlashEquals,
    Static,
    StringLiteral,
    Super,
    Switch,
    TemplateLiteralEnd,
    TemplateLiteralExprEnd,
    TemplateLiteralExprStart,
    TemplateLiteralStart,
    TemplateLiteralString,
    This,
    Throw,
    Tilde,
    TripleDot,
    Trivia,
    Try,
    Typeof,
    UnsignedShiftRight,
    UnsignedShiftRightEquals,
    UnterminatedRegexLiteral,
    UnterminatedStringLiteral,
    UnterminatedTemplateLiteral,
    Var,
    Void,
    While,
    With,
    Yield,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenCategory {
    Invalid,
    Trivia,
    Number,
    String,
    Punctuation,
    Operator,
    Keyword,
    ControlKeyword,
    Identifier,
}

const TOKEN_COUNT: usize = 122;

use TokenCategory::*;

// Indexed by TokenType as u8. Order must match the TokenType enum.
const TOKEN_CATEGORIES: [TokenCategory; TOKEN_COUNT] = [
    Operator,        // Ampersand
    Operator,        // AmpersandEquals
    Operator,        // Arrow
    Operator,        // Asterisk
    Operator,        // AsteriskEquals
    Keyword,         // Async
    Keyword,         // Await
    Number,          // BigIntLiteral
    Keyword,         // BoolLiteral
    Punctuation,     // BracketClose
    Punctuation,     // BracketOpen
    ControlKeyword,  // Break
    Operator,        // Caret
    Operator,        // CaretEquals
    ControlKeyword,  // Case
    ControlKeyword,  // Catch
    Keyword,         // Class
    Punctuation,     // Colon
    Punctuation,     // Comma
    Keyword,         // Const
    ControlKeyword,  // Continue
    Punctuation,     // CurlyClose
    Punctuation,     // CurlyOpen
    Keyword,         // Debugger
    ControlKeyword,  // Default
    Keyword,         // Delete
    ControlKeyword,  // Do
    Operator,        // DoubleAmpersand
    Operator,        // DoubleAmpersandEquals
    Operator,        // DoubleAsterisk
    Operator,        // DoubleAsteriskEquals
    Operator,        // DoublePipe
    Operator,        // DoublePipeEquals
    Operator,        // DoubleQuestionMark
    Operator,        // DoubleQuestionMarkEquals
    ControlKeyword,  // Else
    Keyword,         // Enum
    Invalid,         // Eof
    Operator,        // Equals
    Operator,        // EqualsEquals
    Operator,        // EqualsEqualsEquals
    Identifier,      // EscapedKeyword
    Operator,        // ExclamationMark
    Operator,        // ExclamationMarkEquals
    Operator,        // ExclamationMarkEqualsEquals
    Keyword,         // Export
    Keyword,         // Extends
    ControlKeyword,  // Finally
    ControlKeyword,  // For
    Keyword,         // Function
    Operator,        // GreaterThan
    Operator,        // GreaterThanEquals
    Identifier,      // Identifier
    ControlKeyword,  // If
    Keyword,         // Implements
    Keyword,         // Import
    Keyword,         // In
    Keyword,         // Instanceof
    Keyword,         // Interface
    Invalid,         // Invalid
    Operator,        // LessThan
    Operator,        // LessThanEquals
    Keyword,         // Let
    Operator,        // Minus
    Operator,        // MinusEquals
    Operator,        // MinusMinus
    Keyword,         // New
    Keyword,         // NullLiteral
    Number,          // NumericLiteral
    Keyword,         // Package
    Punctuation,     // ParenClose
    Punctuation,     // ParenOpen
    Operator,        // Percent
    Operator,        // PercentEquals
    Operator,        // Period
    Operator,        // Pipe
    Operator,        // PipeEquals
    Operator,        // Plus
    Operator,        // PlusEquals
    Operator,        // PlusPlus
    Keyword,         // Private
    Identifier,      // PrivateIdentifier
    Keyword,         // Protected
    Keyword,         // Public
    Operator,        // QuestionMark
    Operator,        // QuestionMarkPeriod
    String,          // RegexFlags
    String,          // RegexLiteral
    ControlKeyword,  // Return
    Punctuation,     // Semicolon
    Operator,        // ShiftLeft
    Operator,        // ShiftLeftEquals
    Operator,        // ShiftRight
    Operator,        // ShiftRightEquals
    Operator,        // Slash
    Operator,        // SlashEquals
    Keyword,         // Static
    String,          // StringLiteral
    Keyword,         // Super
    ControlKeyword,  // Switch
    String,          // TemplateLiteralEnd
    Punctuation,     // TemplateLiteralExprEnd
    Punctuation,     // TemplateLiteralExprStart
    String,          // TemplateLiteralStart
    String,          // TemplateLiteralString
    Keyword,         // This
    ControlKeyword,  // Throw
    Operator,        // Tilde
    Operator,        // TripleDot
    Trivia,          // Trivia
    ControlKeyword,  // Try
    Keyword,         // Typeof
    Operator,        // UnsignedShiftRight
    Operator,        // UnsignedShiftRightEquals
    String,          // UnterminatedRegexLiteral
    String,          // UnterminatedStringLiteral
    String,          // UnterminatedTemplateLiteral
    Keyword,         // Var
    Keyword,         // Void
    ControlKeyword,  // While
    ControlKeyword,  // With
    ControlKeyword,  // Yield
];

// Indexed by TokenType as u8. Order must match the TokenType enum.
const TOKEN_NAMES: [&str; TOKEN_COUNT] = [
    "Ampersand",
    "AmpersandEquals",
    "Arrow",
    "Asterisk",
    "AsteriskEquals",
    "async",
    "await",
    "BigIntLiteral",
    "BoolLiteral",
    "BracketClose",
    "BracketOpen",
    "break",
    "Caret",
    "CaretEquals",
    "case",
    "catch",
    "class",
    "Colon",
    "Comma",
    "const",
    "continue",
    "CurlyClose",
    "CurlyOpen",
    "debugger",
    "default",
    "delete",
    "do",
    "DoubleAmpersand",
    "DoubleAmpersandEquals",
    "DoubleAsterisk",
    "DoubleAsteriskEquals",
    "DoublePipe",
    "DoublePipeEquals",
    "DoubleQuestionMark",
    "DoubleQuestionMarkEquals",
    "else",
    "enum",
    "Eof",
    "Equals",
    "EqualsEquals",
    "EqualsEqualsEquals",
    "EscapedKeyword",
    "ExclamationMark",
    "ExclamationMarkEquals",
    "ExclamationMarkEqualsEquals",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "GreaterThan",
    "GreaterThanEquals",
    "Identifier",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "Invalid",
    "LessThan",
    "LessThanEquals",
    "let",
    "Minus",
    "MinusEquals",
    "MinusMinus",
    "new",
    "null",
    "NumericLiteral",
    "package",
    "ParenClose",
    "ParenOpen",
    "Percent",
    "PercentEquals",
    "Period",
    "Pipe",
    "PipeEquals",
    "Plus",
    "PlusEquals",
    "PlusPlus",
    "private",
    "PrivateIdentifier",
    "protected",
    "public",
    "QuestionMark",
    "QuestionMarkPeriod",
    "RegexFlags",
    "RegexLiteral",
    "return",
    "Semicolon",
    "ShiftLeft",
    "ShiftLeftEquals",
    "ShiftRight",
    "ShiftRightEquals",
    "Slash",
    "SlashEquals",
    "static",
    "StringLiteral",
    "super",
    "switch",
    "TemplateLiteralEnd",
    "TemplateLiteralExprEnd",
    "TemplateLiteralExprStart",
    "TemplateLiteralStart",
    "TemplateLiteralString",
    "this",
    "throw",
    "Tilde",
    "TripleDot",
    "Trivia",
    "try",
    "typeof",
    "UnsignedShiftRight",
    "UnsignedShiftRightEquals",
    "UnterminatedRegexLiteral",
    "UnterminatedStringLiteral",
    "UnterminatedTemplateLiteral",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

impl TokenType {
    pub fn category(self) -> TokenCategory {
        TOKEN_CATEGORIES[self as usize]
    }

    pub fn name(self) -> &'static str {
        TOKEN_NAMES[self as usize]
    }

    pub fn is_identifier_name(self) -> bool {
        self != TokenType::PrivateIdentifier
            && matches!(
                self.category(),
                TokenCategory::Identifier | TokenCategory::Keyword | TokenCategory::ControlKeyword
            )
    }
}

#[derive(Debug, Clone)]
pub struct Token {
    pub token_type: TokenType,
    pub trivia_start: u32,
    pub trivia_len: u32,
    pub value_start: u32,
    pub value_len: u32,
    pub line_number: u32,
    pub line_column: u32,
    pub offset: u32,
    pub trivia_has_line_terminator: bool,
    /// Decoded identifier value, set when the identifier contains unicode
    /// escape sequences (e.g. `l\u0065t` → `let`).
    pub identifier_value: Option<Vec<u16>>,
}

impl Token {
    pub fn new(token_type: TokenType) -> Self {
        Token {
            token_type,
            trivia_start: 0,
            trivia_len: 0,
            value_start: 0,
            value_len: 0,
            line_number: 0,
            line_column: 0,
            offset: 0,
            trivia_has_line_terminator: false,
            identifier_value: None,
        }
    }
}
