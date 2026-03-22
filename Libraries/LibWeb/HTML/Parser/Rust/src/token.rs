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

/// Tag-specific data shared between start and end tag tokens.
#[derive(Clone, Debug)]
pub struct TagData {
    pub tag_name: String,
    pub self_closing: bool,
    pub attributes: Vec<Attribute>,
}

impl TagData {
    pub fn new() -> Self {
        TagData {
            tag_name: String::new(),
            self_closing: false,
            attributes: Vec::new(),
        }
    }
}

impl Default for TagData {
    fn default() -> Self {
        Self::new()
    }
}

/// The type-specific data of an HTML token.
#[derive(Clone, Debug)]
pub enum TokenKind {
    Character(u32),
    StartTag(TagData),
    EndTag(TagData),
    Comment(String),
    Doctype(Box<DoctypeData>),
    EndOfFile,
}

impl TokenKind {
    pub fn new_start_tag() -> Self {
        TokenKind::StartTag(TagData::new())
    }

    pub fn new_end_tag() -> Self {
        TokenKind::EndTag(TagData::new())
    }

    pub fn new_comment() -> Self {
        TokenKind::Comment(String::new())
    }

    pub fn new_doctype() -> Self {
        TokenKind::Doctype(Box::new(DoctypeData::default()))
    }
}

/// An HTML token produced by the tokenizer.
#[derive(Clone, Debug)]
pub struct Token {
    pub kind: TokenKind,
    pub start_position: Position,
    pub end_position: Position,
}

impl Default for Token {
    fn default() -> Self {
        Token {
            kind: TokenKind::EndOfFile,
            start_position: Position::default(),
            end_position: Position::default(),
        }
    }
}

impl Token {
    pub fn new_character(code_point: u32) -> Self {
        Token {
            kind: TokenKind::Character(code_point),
            ..Default::default()
        }
    }

    pub fn new_eof() -> Self {
        Token {
            kind: TokenKind::EndOfFile,
            ..Default::default()
        }
    }

    pub fn is_start_tag(&self) -> bool {
        matches!(self.kind, TokenKind::StartTag(_))
    }

    pub fn is_end_tag(&self) -> bool {
        matches!(self.kind, TokenKind::EndTag(_))
    }

    pub fn is_character(&self) -> bool {
        matches!(self.kind, TokenKind::Character(_))
    }

    pub fn is_comment(&self) -> bool {
        matches!(self.kind, TokenKind::Comment(_))
    }

    pub fn is_doctype(&self) -> bool {
        matches!(self.kind, TokenKind::Doctype(_))
    }

    pub fn is_eof(&self) -> bool {
        matches!(self.kind, TokenKind::EndOfFile)
    }

    /// https://html.spec.whatwg.org/multipage/parsing.html#parser-whitespace
    /// ASCII whitespace tokens: U+0009 CHARACTER TABULATION, U+000A LINE FEED (LF),
    /// U+000C FORM FEED (FF), U+000D CARRIAGE RETURN (CR), U+0020 SPACE
    pub fn is_parser_whitespace(&self) -> bool {
        matches!(self.kind, TokenKind::Character(cp) if matches!(cp, 0x0009 | 0x000A | 0x000C | 0x000D | 0x0020))
    }

    pub fn code_point(&self) -> u32 {
        match self.kind {
            TokenKind::Character(cp) => cp,
            _ => 0,
        }
    }

    pub fn tag_name(&self) -> &str {
        match &self.kind {
            TokenKind::StartTag(d) | TokenKind::EndTag(d) => &d.tag_name,
            _ => "",
        }
    }

    pub fn tag_name_mut(&mut self) -> &mut String {
        match &mut self.kind {
            TokenKind::StartTag(d) | TokenKind::EndTag(d) => &mut d.tag_name,
            _ => panic!("tag_name_mut called on non-tag token"),
        }
    }

    pub fn set_self_closing(&mut self, value: bool) {
        match &mut self.kind {
            TokenKind::StartTag(d) | TokenKind::EndTag(d) => d.self_closing = value,
            _ => panic!("set_self_closing called on non-tag token"),
        }
    }

    pub fn is_self_closing(&self) -> bool {
        match &self.kind {
            TokenKind::StartTag(d) | TokenKind::EndTag(d) => d.self_closing,
            _ => false,
        }
    }

    pub fn attributes(&self) -> &[Attribute] {
        match &self.kind {
            TokenKind::StartTag(d) | TokenKind::EndTag(d) => &d.attributes,
            _ => &[],
        }
    }

    pub fn attributes_mut(&mut self) -> &mut Vec<Attribute> {
        match &mut self.kind {
            TokenKind::StartTag(d) | TokenKind::EndTag(d) => &mut d.attributes,
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
        match &self.kind {
            TokenKind::Comment(s) => s,
            _ => "",
        }
    }

    pub fn set_comment_data(&mut self, data: String) {
        match &mut self.kind {
            TokenKind::Comment(s) => *s = data,
            _ => panic!("set_comment_data called on non-comment token"),
        }
    }

    pub fn doctype_data(&self) -> &DoctypeData {
        match &self.kind {
            TokenKind::Doctype(dd) => dd,
            _ => panic!("doctype_data called on non-doctype token"),
        }
    }

    pub fn doctype_data_mut(&mut self) -> &mut DoctypeData {
        match &mut self.kind {
            TokenKind::Doctype(dd) => dd,
            _ => panic!("doctype_data_mut called on non-doctype token"),
        }
    }
}
