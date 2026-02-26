// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

/// Source position in the input.
#[derive(Clone, Copy, Debug, Default)]
pub struct Position {
    pub line: u64,
    pub column: u64,
}

/// A single attribute on a start or end tag token.
#[derive(Clone, Debug, Default)]
pub struct Attribute {
    pub local_name: String,
    pub value: String,
    pub name_start_position: Position,
    pub name_end_position: Position,
    pub value_start_position: Position,
    pub value_end_position: Position,
}

/// Data specific to DOCTYPE tokens.
#[derive(Clone, Debug, Default)]
pub struct DoctypeData {
    pub name: String,
    pub public_identifier: String,
    pub system_identifier: String,
    pub missing_name: bool,
    pub missing_public_identifier: bool,
    pub missing_system_identifier: bool,
    pub force_quirks: bool,
}

/// The type of an HTML token.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum TokenType {
    Invalid = 0,
    Doctype = 1,
    StartTag = 2,
    EndTag = 3,
    Comment = 4,
    Character = 5,
    EndOfFile = 6,
}

impl Default for TokenType {
    fn default() -> Self {
        TokenType::Invalid
    }
}

/// An HTML token produced by the tokenizer.
#[derive(Clone, Debug, Default)]
pub struct Token {
    pub token_type: TokenType,
    /// For Character tokens: the code point.
    pub code_point: u32,
    /// For StartTag/EndTag tokens: the tag name.
    pub tag_name: String,
    /// For StartTag/EndTag tokens: whether the tag is self-closing.
    pub self_closing: bool,
    /// For StartTag/EndTag tokens: the attributes.
    pub attributes: Vec<Attribute>,
    /// For Comment tokens: the comment data.
    pub comment_data: String,
    /// For DOCTYPE tokens: the doctype data.
    pub doctype_data: Option<DoctypeData>,
    /// Source position where this token starts.
    pub start_position: Position,
    /// Source position where this token ends.
    pub end_position: Position,
}

impl Token {
    pub fn new_character(code_point: u32) -> Self {
        Token {
            token_type: TokenType::Character,
            code_point,
            ..Default::default()
        }
    }

    pub fn new_eof() -> Self {
        Token {
            token_type: TokenType::EndOfFile,
            ..Default::default()
        }
    }
}
