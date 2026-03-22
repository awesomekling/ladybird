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

/// Type-specific data for an HTML token.
#[derive(Clone, Debug)]
pub enum TokenPayload {
    None,
    Tag {
        tag_name: String,
        self_closing: bool,
        attributes: Vec<Attribute>,
    },
    Comment(String),
    Doctype(Box<DoctypeData>),
}

impl Default for TokenPayload {
    fn default() -> Self {
        TokenPayload::None
    }
}

/// An HTML token produced by the tokenizer.
#[derive(Clone, Debug, Default)]
pub struct Token {
    pub token_type: TokenType,
    pub code_point: u32,
    pub payload: TokenPayload,
    pub start_position: Position,
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

    pub fn is_start_tag(&self) -> bool {
        self.token_type == TokenType::StartTag
    }

    pub fn is_end_tag(&self) -> bool {
        self.token_type == TokenType::EndTag
    }

    pub fn is_character(&self) -> bool {
        self.token_type == TokenType::Character
    }

    pub fn is_comment(&self) -> bool {
        self.token_type == TokenType::Comment
    }

    pub fn is_doctype(&self) -> bool {
        self.token_type == TokenType::Doctype
    }

    pub fn is_eof(&self) -> bool {
        self.token_type == TokenType::EndOfFile
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parser-whitespace
    /// ASCII whitespace tokens: U+0009 CHARACTER TABULATION, U+000A LINE FEED (LF),
    /// U+000C FORM FEED (FF), U+000D CARRIAGE RETURN (CR), U+0020 SPACE
    pub fn is_parser_whitespace(&self) -> bool {
        self.token_type == TokenType::Character
            && matches!(self.code_point, 0x0009 | 0x000A | 0x000C | 0x000D | 0x0020)
    }

    pub fn tag_name(&self) -> &str {
        match &self.payload {
            TokenPayload::Tag { tag_name, .. } => tag_name,
            _ => "",
        }
    }

    pub fn tag_name_mut(&mut self) -> &mut String {
        match &mut self.payload {
            TokenPayload::Tag { tag_name, .. } => tag_name,
            _ => panic!("tag_name_mut called on non-tag token"),
        }
    }

    pub fn set_self_closing(&mut self, value: bool) {
        match &mut self.payload {
            TokenPayload::Tag { self_closing, .. } => *self_closing = value,
            _ => panic!("set_self_closing called on non-tag token"),
        }
    }

    pub fn is_self_closing(&self) -> bool {
        match &self.payload {
            TokenPayload::Tag { self_closing, .. } => *self_closing,
            _ => false,
        }
    }

    pub fn attributes(&self) -> &[Attribute] {
        match &self.payload {
            TokenPayload::Tag { attributes, .. } => attributes,
            _ => &[],
        }
    }

    pub fn attributes_mut(&mut self) -> &mut Vec<Attribute> {
        match &mut self.payload {
            TokenPayload::Tag { attributes, .. } => attributes,
            _ => panic!("attributes_mut called on non-tag token"),
        }
    }

    pub fn has_attribute(&self, name: &str) -> bool {
        self.attributes().iter().any(|a| a.local_name == name)
    }

    pub fn get_attribute(&self, name: &str) -> Option<&str> {
        self.attributes()
            .iter()
            .find(|a| a.local_name == name)
            .map(|a| a.value.as_str())
    }

    pub fn comment_data(&self) -> &str {
        match &self.payload {
            TokenPayload::Comment(s) => s,
            _ => "",
        }
    }

    pub fn set_comment_data(&mut self, data: String) {
        match &mut self.payload {
            TokenPayload::Comment(s) => *s = data,
            _ => panic!("set_comment_data called on non-comment token"),
        }
    }

    pub fn doctype_data(&self) -> &DoctypeData {
        match &self.payload {
            TokenPayload::Doctype(dd) => dd,
            _ => panic!("doctype_data called on non-doctype token"),
        }
    }

    pub fn doctype_data_mut(&mut self) -> &mut DoctypeData {
        match &mut self.payload {
            TokenPayload::Doctype(dd) => dd,
            _ => panic!("doctype_data_mut called on non-doctype token"),
        }
    }
}
