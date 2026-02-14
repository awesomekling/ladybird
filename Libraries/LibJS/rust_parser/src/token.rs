/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Token types and Token struct for the lexer.
//!
//! The `TokenType` enum must match the C++ `JS::TokenType` exactly
//! (same variants in the same order) because scope analysis passes
//! token type values across the FFI boundary. The order follows
//! `ENUMERATE_JS_TOKENS` in Token.h (alphabetical).

/// Token types matching the C++ `JS::TokenType` enum exactly.
/// The order must match `ENUMERATE_JS_TOKENS` in Token.h (alphabetical).
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

impl TokenType {
    pub fn category(self) -> TokenCategory {
        match self {
            TokenType::Ampersand
            | TokenType::AmpersandEquals
            | TokenType::Arrow
            | TokenType::Asterisk
            | TokenType::AsteriskEquals
            | TokenType::Caret
            | TokenType::CaretEquals
            | TokenType::DoubleAmpersand
            | TokenType::DoubleAmpersandEquals
            | TokenType::DoubleAsterisk
            | TokenType::DoubleAsteriskEquals
            | TokenType::DoublePipe
            | TokenType::DoublePipeEquals
            | TokenType::DoubleQuestionMark
            | TokenType::DoubleQuestionMarkEquals
            | TokenType::Equals
            | TokenType::EqualsEquals
            | TokenType::EqualsEqualsEquals
            | TokenType::ExclamationMark
            | TokenType::ExclamationMarkEquals
            | TokenType::ExclamationMarkEqualsEquals
            | TokenType::GreaterThan
            | TokenType::GreaterThanEquals
            | TokenType::LessThan
            | TokenType::LessThanEquals
            | TokenType::Minus
            | TokenType::MinusEquals
            | TokenType::MinusMinus
            | TokenType::Percent
            | TokenType::PercentEquals
            | TokenType::Period
            | TokenType::Pipe
            | TokenType::PipeEquals
            | TokenType::Plus
            | TokenType::PlusEquals
            | TokenType::PlusPlus
            | TokenType::QuestionMark
            | TokenType::QuestionMarkPeriod
            | TokenType::ShiftLeft
            | TokenType::ShiftLeftEquals
            | TokenType::ShiftRight
            | TokenType::ShiftRightEquals
            | TokenType::Slash
            | TokenType::SlashEquals
            | TokenType::Tilde
            | TokenType::TripleDot
            | TokenType::UnsignedShiftRight
            | TokenType::UnsignedShiftRightEquals => TokenCategory::Operator,

            TokenType::BoolLiteral
            | TokenType::NullLiteral
            | TokenType::Async
            | TokenType::Await
            | TokenType::Class
            | TokenType::Const
            | TokenType::Debugger
            | TokenType::Delete
            | TokenType::Enum
            | TokenType::Export
            | TokenType::Extends
            | TokenType::Function
            | TokenType::Implements
            | TokenType::Import
            | TokenType::In
            | TokenType::Instanceof
            | TokenType::Interface
            | TokenType::Let
            | TokenType::New
            | TokenType::Package
            | TokenType::Private
            | TokenType::Protected
            | TokenType::Public
            | TokenType::Static
            | TokenType::Super
            | TokenType::This
            | TokenType::Typeof
            | TokenType::Var
            | TokenType::Void => TokenCategory::Keyword,

            TokenType::Break
            | TokenType::Case
            | TokenType::Catch
            | TokenType::Continue
            | TokenType::Default
            | TokenType::Do
            | TokenType::Else
            | TokenType::Finally
            | TokenType::For
            | TokenType::If
            | TokenType::Return
            | TokenType::Switch
            | TokenType::Throw
            | TokenType::Try
            | TokenType::While
            | TokenType::With
            | TokenType::Yield => TokenCategory::ControlKeyword,

            TokenType::BigIntLiteral | TokenType::NumericLiteral => TokenCategory::Number,

            TokenType::RegexFlags
            | TokenType::RegexLiteral
            | TokenType::StringLiteral
            | TokenType::TemplateLiteralEnd
            | TokenType::TemplateLiteralString
            | TokenType::TemplateLiteralStart
            | TokenType::UnterminatedRegexLiteral
            | TokenType::UnterminatedStringLiteral
            | TokenType::UnterminatedTemplateLiteral => TokenCategory::String,

            TokenType::BracketClose
            | TokenType::BracketOpen
            | TokenType::Colon
            | TokenType::Comma
            | TokenType::CurlyClose
            | TokenType::CurlyOpen
            | TokenType::ParenClose
            | TokenType::ParenOpen
            | TokenType::Semicolon
            | TokenType::TemplateLiteralExprEnd
            | TokenType::TemplateLiteralExprStart => TokenCategory::Punctuation,

            TokenType::Identifier
            | TokenType::EscapedKeyword
            | TokenType::PrivateIdentifier => TokenCategory::Identifier,

            TokenType::Trivia => TokenCategory::Trivia,

            TokenType::Eof | TokenType::Invalid => TokenCategory::Invalid,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            TokenType::Ampersand => "Ampersand",
            TokenType::AmpersandEquals => "AmpersandEquals",
            TokenType::Arrow => "Arrow",
            TokenType::Asterisk => "Asterisk",
            TokenType::AsteriskEquals => "AsteriskEquals",
            TokenType::Async => "async",
            TokenType::Await => "await",
            TokenType::BigIntLiteral => "BigIntLiteral",
            TokenType::BoolLiteral => "BoolLiteral",
            TokenType::BracketClose => "BracketClose",
            TokenType::BracketOpen => "BracketOpen",
            TokenType::Break => "break",
            TokenType::Caret => "Caret",
            TokenType::CaretEquals => "CaretEquals",
            TokenType::Case => "case",
            TokenType::Catch => "catch",
            TokenType::Class => "class",
            TokenType::Colon => "Colon",
            TokenType::Comma => "Comma",
            TokenType::Const => "const",
            TokenType::Continue => "continue",
            TokenType::CurlyClose => "CurlyClose",
            TokenType::CurlyOpen => "CurlyOpen",
            TokenType::Debugger => "debugger",
            TokenType::Default => "default",
            TokenType::Delete => "delete",
            TokenType::Do => "do",
            TokenType::DoubleAmpersand => "DoubleAmpersand",
            TokenType::DoubleAmpersandEquals => "DoubleAmpersandEquals",
            TokenType::DoubleAsterisk => "DoubleAsterisk",
            TokenType::DoubleAsteriskEquals => "DoubleAsteriskEquals",
            TokenType::DoublePipe => "DoublePipe",
            TokenType::DoublePipeEquals => "DoublePipeEquals",
            TokenType::DoubleQuestionMark => "DoubleQuestionMark",
            TokenType::DoubleQuestionMarkEquals => "DoubleQuestionMarkEquals",
            TokenType::Else => "else",
            TokenType::Enum => "enum",
            TokenType::Eof => "Eof",
            TokenType::Equals => "Equals",
            TokenType::EqualsEquals => "EqualsEquals",
            TokenType::EqualsEqualsEquals => "EqualsEqualsEquals",
            TokenType::EscapedKeyword => "EscapedKeyword",
            TokenType::ExclamationMark => "ExclamationMark",
            TokenType::ExclamationMarkEquals => "ExclamationMarkEquals",
            TokenType::ExclamationMarkEqualsEquals => "ExclamationMarkEqualsEquals",
            TokenType::Export => "export",
            TokenType::Extends => "extends",
            TokenType::Finally => "finally",
            TokenType::For => "for",
            TokenType::Function => "function",
            TokenType::GreaterThan => "GreaterThan",
            TokenType::GreaterThanEquals => "GreaterThanEquals",
            TokenType::Identifier => "Identifier",
            TokenType::If => "if",
            TokenType::Implements => "implements",
            TokenType::Import => "import",
            TokenType::In => "in",
            TokenType::Instanceof => "instanceof",
            TokenType::Interface => "interface",
            TokenType::Invalid => "Invalid",
            TokenType::LessThan => "LessThan",
            TokenType::LessThanEquals => "LessThanEquals",
            TokenType::Let => "let",
            TokenType::Minus => "Minus",
            TokenType::MinusEquals => "MinusEquals",
            TokenType::MinusMinus => "MinusMinus",
            TokenType::New => "new",
            TokenType::NullLiteral => "null",
            TokenType::NumericLiteral => "NumericLiteral",
            TokenType::Package => "package",
            TokenType::ParenClose => "ParenClose",
            TokenType::ParenOpen => "ParenOpen",
            TokenType::Percent => "Percent",
            TokenType::PercentEquals => "PercentEquals",
            TokenType::Period => "Period",
            TokenType::Pipe => "Pipe",
            TokenType::PipeEquals => "PipeEquals",
            TokenType::Plus => "Plus",
            TokenType::PlusEquals => "PlusEquals",
            TokenType::PlusPlus => "PlusPlus",
            TokenType::Private => "private",
            TokenType::PrivateIdentifier => "PrivateIdentifier",
            TokenType::Protected => "protected",
            TokenType::Public => "public",
            TokenType::QuestionMark => "QuestionMark",
            TokenType::QuestionMarkPeriod => "QuestionMarkPeriod",
            TokenType::RegexFlags => "RegexFlags",
            TokenType::RegexLiteral => "RegexLiteral",
            TokenType::Return => "return",
            TokenType::Semicolon => "Semicolon",
            TokenType::ShiftLeft => "ShiftLeft",
            TokenType::ShiftLeftEquals => "ShiftLeftEquals",
            TokenType::ShiftRight => "ShiftRight",
            TokenType::ShiftRightEquals => "ShiftRightEquals",
            TokenType::Slash => "Slash",
            TokenType::SlashEquals => "SlashEquals",
            TokenType::Static => "static",
            TokenType::StringLiteral => "StringLiteral",
            TokenType::Super => "super",
            TokenType::Switch => "switch",
            TokenType::TemplateLiteralEnd => "TemplateLiteralEnd",
            TokenType::TemplateLiteralExprEnd => "TemplateLiteralExprEnd",
            TokenType::TemplateLiteralExprStart => "TemplateLiteralExprStart",
            TokenType::TemplateLiteralStart => "TemplateLiteralStart",
            TokenType::TemplateLiteralString => "TemplateLiteralString",
            TokenType::This => "this",
            TokenType::Throw => "throw",
            TokenType::Tilde => "Tilde",
            TokenType::TripleDot => "TripleDot",
            TokenType::Trivia => "Trivia",
            TokenType::Try => "try",
            TokenType::Typeof => "typeof",
            TokenType::UnsignedShiftRight => "UnsignedShiftRight",
            TokenType::UnsignedShiftRightEquals => "UnsignedShiftRightEquals",
            TokenType::UnterminatedRegexLiteral => "UnterminatedRegexLiteral",
            TokenType::UnterminatedStringLiteral => "UnterminatedStringLiteral",
            TokenType::UnterminatedTemplateLiteral => "UnterminatedTemplateLiteral",
            TokenType::Var => "var",
            TokenType::Void => "void",
            TokenType::While => "while",
            TokenType::With => "with",
            TokenType::Yield => "yield",
        }
    }

    pub fn is_identifier_name(self) -> bool {
        self != TokenType::PrivateIdentifier
            && matches!(
                self.category(),
                TokenCategory::Identifier | TokenCategory::Keyword | TokenCategory::ControlKeyword
            )
    }
}

/// Token produced by the lexer.
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
