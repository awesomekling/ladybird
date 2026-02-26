// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

use std::collections::VecDeque;

use crate::entities::NamedCharacterReferenceMatcher;
use crate::token::{Attribute, DoctypeData, Position, Token, TokenType};

/// Tokenizer states per the WHATWG HTML spec section 13.2.5.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum State {
    Data = 0,
    RCDATA,
    RAWTEXT,
    ScriptData,
    PLAINTEXT,
    TagOpen,
    EndTagOpen,
    TagName,
    RCDATALessThanSign,
    RCDATAEndTagOpen,
    RCDATAEndTagName,
    RAWTEXTLessThanSign,
    RAWTEXTEndTagOpen,
    RAWTEXTEndTagName,
    ScriptDataLessThanSign,
    ScriptDataEndTagOpen,
    ScriptDataEndTagName,
    ScriptDataEscapeStart,
    ScriptDataEscapeStartDash,
    ScriptDataEscaped,
    ScriptDataEscapedDash,
    ScriptDataEscapedDashDash,
    ScriptDataEscapedLessThanSign,
    ScriptDataEscapedEndTagOpen,
    ScriptDataEscapedEndTagName,
    ScriptDataDoubleEscapeStart,
    ScriptDataDoubleEscaped,
    ScriptDataDoubleEscapedDash,
    ScriptDataDoubleEscapedDashDash,
    ScriptDataDoubleEscapedLessThanSign,
    ScriptDataDoubleEscapeEnd,
    BeforeAttributeName,
    AttributeName,
    AfterAttributeName,
    BeforeAttributeValue,
    AttributeValueDoubleQuoted,
    AttributeValueSingleQuoted,
    AttributeValueUnquoted,
    AfterAttributeValueQuoted,
    SelfClosingStartTag,
    BogusComment,
    MarkupDeclarationOpen,
    CommentStart,
    CommentStartDash,
    Comment,
    CommentLessThanSign,
    CommentLessThanSignBang,
    CommentLessThanSignBangDash,
    CommentLessThanSignBangDashDash,
    CommentEndDash,
    CommentEnd,
    CommentEndBang,
    DOCTYPE,
    BeforeDOCTYPEName,
    DOCTYPEName,
    AfterDOCTYPEName,
    AfterDOCTYPEPublicKeyword,
    BeforeDOCTYPEPublicIdentifier,
    DOCTYPEPublicIdentifierDoubleQuoted,
    DOCTYPEPublicIdentifierSingleQuoted,
    AfterDOCTYPEPublicIdentifier,
    BetweenDOCTYPEPublicAndSystemIdentifiers,
    AfterDOCTYPESystemKeyword,
    BeforeDOCTYPESystemIdentifier,
    DOCTYPESystemIdentifierDoubleQuoted,
    DOCTYPESystemIdentifierSingleQuoted,
    AfterDOCTYPESystemIdentifier,
    BogusDOCTYPE,
    CDATASection,
    CDATASectionBracket,
    CDATASectionEnd,
    CharacterReference,
    NamedCharacterReference,
    AmbiguousAmpersand,
    NumericCharacterReference,
    HexadecimalCharacterReferenceStart,
    DecimalCharacterReferenceStart,
    HexadecimalCharacterReference,
    DecimalCharacterReference,
    NumericCharacterReferenceEnd,
}

/// The HTML tokenizer state machine.
///
/// Implements the WHATWG HTML tokenizer specification (section 13.2.5).
pub struct HtmlTokenizer {
    state: State,
    return_state: State,
    input: Vec<u32>,
    current_offset: usize,
    prev_offset: usize,
    current_token: Token,
    current_builder: String,
    queued_tokens: VecDeque<Token>,
    temporary_buffer: Vec<u32>,
    character_reference_code: u32,
    last_emitted_start_tag_name: Option<String>,
    source_positions: Vec<Position>,
    has_emitted_eof: bool,
    aborted: bool,
    insertion_point: Option<usize>,
    old_insertion_point: Option<usize>,
    explicit_eof_inserted: bool,
    blocked: bool,
    stop_at_insertion_point: bool,
    cdata_allowed: bool,
    entity_matcher: NamedCharacterReferenceMatcher,
}

#[inline]
fn is_ascii_alpha(c: u32) -> bool {
    matches!(c, 0x41..=0x5A | 0x61..=0x7A)
}

#[inline]
fn is_ascii_upper_alpha(c: u32) -> bool {
    matches!(c, 0x41..=0x5A)
}

fn is_ascii_digit(c: u32) -> bool {
    matches!(c, 0x30..=0x39)
}

fn is_ascii_hex_digit(c: u32) -> bool {
    matches!(c, 0x30..=0x39 | 0x41..=0x46 | 0x61..=0x66)
}

fn is_ascii_alphanumeric(c: u32) -> bool {
    is_ascii_alpha(c) || is_ascii_digit(c)
}

#[inline]
fn to_ascii_lowercase(c: u32) -> u32 {
    if is_ascii_upper_alpha(c) {
        c + 0x20
    } else {
        c
    }
}

#[inline]
fn is_whitespace(c: u32) -> bool {
    matches!(c, 0x09 | 0x0A | 0x0C | 0x20)
}

/// Numeric character reference replacement table (spec section 13.2.5.5).
fn numeric_char_ref_replacement(code: u32) -> Option<u32> {
    match code {
        0x80 => Some(0x20AC),
        0x82 => Some(0x201A),
        0x83 => Some(0x0192),
        0x84 => Some(0x201E),
        0x85 => Some(0x2026),
        0x86 => Some(0x2020),
        0x87 => Some(0x2021),
        0x88 => Some(0x02C6),
        0x89 => Some(0x2030),
        0x8A => Some(0x0160),
        0x8B => Some(0x2039),
        0x8C => Some(0x0152),
        0x8E => Some(0x017D),
        0x91 => Some(0x2018),
        0x92 => Some(0x2019),
        0x93 => Some(0x201C),
        0x94 => Some(0x201D),
        0x95 => Some(0x2022),
        0x96 => Some(0x2013),
        0x97 => Some(0x2014),
        0x98 => Some(0x02DC),
        0x99 => Some(0x2122),
        0x9A => Some(0x0161),
        0x9B => Some(0x203A),
        0x9C => Some(0x0153),
        0x9E => Some(0x017E),
        0x9F => Some(0x0178),
        _ => None,
    }
}

/// Check if a code point is a Unicode surrogate.
fn is_surrogate(c: u32) -> bool {
    (0xD800..=0xDFFF).contains(&c)
}

/// Check if a code point is a Unicode noncharacter.
fn is_noncharacter(c: u32) -> bool {
    (0xFDD0..=0xFDEF).contains(&c) || matches!(c & 0xFFFF, 0xFFFE | 0xFFFF)
}

fn is_ascii_lower_alpha(c: u32) -> bool {
    matches!(c, 0x61..=0x7A)
}

fn cp_to_char(cp: u32) -> char {
    char::from_u32(cp).unwrap_or('\u{FFFD}')
}

impl HtmlTokenizer {
    /// Create a new tokenizer for the given input (as UTF-32 code points).
    pub fn new(input: Vec<u32>) -> Self {
        HtmlTokenizer {
            state: State::Data,
            return_state: State::Data,
            input,
            current_offset: 0,
            prev_offset: 0,
            current_token: Token::default(),
            current_builder: String::new(),
            queued_tokens: VecDeque::new(),
            temporary_buffer: Vec::new(),
            character_reference_code: 0,
            last_emitted_start_tag_name: None,
            source_positions: vec![Position { line: 0, column: 0 }],
            has_emitted_eof: false,
            aborted: false,
            insertion_point: None,
            old_insertion_point: None,
            explicit_eof_inserted: false,
            blocked: false,
            stop_at_insertion_point: false,
            cdata_allowed: false,
            entity_matcher: NamedCharacterReferenceMatcher::new(),
        }
    }

    /// Set the tokenizer state.
    pub fn switch_to(&mut self, state: State) {
        self.state = state;
    }

    /// Set the last emitted start tag name (for RCDATA/ScriptData end tag matching).
    pub fn set_last_start_tag(&mut self, name: &str) {
        self.last_emitted_start_tag_name = Some(name.to_string());
    }

    // -- Insertion point management --

    pub fn store_insertion_point(&mut self) {
        self.old_insertion_point = self.insertion_point;
    }

    pub fn restore_insertion_point(&mut self) {
        self.insertion_point = self.old_insertion_point.take();
    }

    pub fn update_insertion_point(&mut self) {
        self.insertion_point = Some(self.current_offset);
    }

    pub fn undefine_insertion_point(&mut self) {
        self.insertion_point = None;
    }

    pub fn is_insertion_point_defined(&self) -> bool {
        self.insertion_point.is_some()
    }

    pub fn insert_input_at_insertion_point(&mut self, code_points: &[u32]) {
        if let Some(ip) = self.insertion_point {
            let ip = ip.min(self.input.len());
            self.input.splice(ip..ip, code_points.iter().copied());
            self.insertion_point = Some(ip + code_points.len());
        }
    }

    pub fn set_blocked(&mut self, blocked: bool) {
        self.blocked = blocked;
    }

    pub fn is_blocked(&self) -> bool {
        self.blocked
    }

    pub fn insert_eof(&mut self) {
        self.explicit_eof_inserted = true;
    }

    pub fn is_eof_inserted(&self) -> bool {
        self.explicit_eof_inserted
    }

    pub fn abort(&mut self) {
        self.aborted = true;
    }

    // -- Input helpers --

    #[inline]
    fn peek_code_point(&self, offset: isize) -> Option<u32> {
        let idx = self.current_offset as isize + offset;
        if idx < 0 || idx as usize >= self.input.len() {
            return None;
        }
        if self.stop_at_insertion_point {
            if let Some(ip) = self.insertion_point {
                if idx as usize >= ip {
                    return None;
                }
            }
        }
        Some(self.input[idx as usize])
    }

    #[inline]
    fn skip(&mut self, count: usize) {
        if !self.source_positions.is_empty() {
            let last = *self.source_positions.last().unwrap();
            self.source_positions.push(last);
        }
        for _ in 0..count {
            self.prev_offset = self.current_offset;
            let code_point = self.input[self.current_offset];
            if let Some(pos) = self.source_positions.last_mut() {
                if code_point == 0x0A {
                    pos.column = 0;
                    pos.line += 1;
                } else {
                    pos.column += 1;
                }
            }
            self.current_offset += 1;
        }
    }

    fn next_code_point(&mut self) -> Option<u32> {
        if self.current_offset >= self.input.len() {
            self.prev_offset = self.current_offset;
            return None;
        }
        let cp = self.input[self.current_offset];
        if cp != 0x0D {
            self.skip(1);
            return Some(cp);
        }
        // Slow path: CR normalization
        let next = self.peek_code_point(1).unwrap_or(0);
        if next == 0x0A {
            self.skip(2);
        } else {
            self.skip(1);
        }
        Some(0x0A)
    }

    #[inline]
    fn nth_last_position(&self, n: usize) -> Position {
        if n + 1 > self.source_positions.len() {
            return Position { line: 0, column: 0 };
        }
        self.source_positions[self.source_positions.len() - 1 - n]
    }

    fn restore_to(&mut self, offset: usize) {
        while self.current_offset > offset && self.source_positions.len() > 1 {
            self.source_positions.pop();
            self.current_offset -= 1;
        }
        self.current_offset = offset;
    }

    fn consume_current_builder(&mut self) -> String {
        std::mem::take(&mut self.current_builder)
    }

    fn create_new_token(&mut self, token_type: TokenType) {
        self.current_token = Token::default();
        self.current_token.token_type = token_type;
        let pos = match token_type {
            TokenType::StartTag | TokenType::EndTag => self.nth_last_position(1),
            _ => self.nth_last_position(0),
        };
        self.current_token.start_position = pos;
    }

    fn current_end_tag_token_is_appropriate(&self) -> bool {
        if self.current_token.token_type != TokenType::EndTag {
            return false;
        }
        match &self.last_emitted_start_tag_name {
            Some(name) => self.current_token.tag_name == *name,
            None => false,
        }
    }

    fn consumed_as_part_of_an_attribute(&self) -> bool {
        matches!(
            self.return_state,
            State::AttributeValueDoubleQuoted
                | State::AttributeValueSingleQuoted
                | State::AttributeValueUnquoted
        )
    }

    fn will_emit(&mut self, token_idx: usize) {
        // token_idx: 0 = current_token, 1+ = queued_tokens index
        // For simplicity, we handle position setting here.
        if token_idx == 0 {
            if self.current_token.token_type == TokenType::StartTag {
                self.last_emitted_start_tag_name =
                    Some(self.current_token.tag_name.clone());
            }
            let is_start_or_end_tag = self.current_token.token_type == TokenType::StartTag
                || self.current_token.token_type == TokenType::EndTag;
            self.current_token.end_position =
                self.nth_last_position(if is_start_or_end_tag { 1 } else { 0 });
        }
    }

    fn emit_current_token(&mut self) {
        self.will_emit(0);
        let token = std::mem::take(&mut self.current_token);
        self.queued_tokens.push_back(token);
    }

    #[inline]
    fn return_character_token(&mut self, code_point: u32) -> Option<Token> {
        let token = self.make_character_token(code_point);
        if self.queued_tokens.is_empty() {
            Some(token)
        } else {
            self.queued_tokens.push_back(token);
            self.queued_tokens.pop_front()
        }
    }

    fn emit_eof(&mut self) {
        if self.has_emitted_eof {
            return;
        }
        self.has_emitted_eof = true;
        self.create_new_token(TokenType::EndOfFile);
        self.emit_current_token();
    }

    fn emit_current_token_followed_by_eof(&mut self) {
        self.emit_current_token();
        self.has_emitted_eof = true;
        self.create_new_token(TokenType::EndOfFile);
        self.emit_current_token();
    }

    fn flush_codepoints_consumed_as_character_reference(&mut self) {
        for i in 0..self.temporary_buffer.len() {
            let code_point = self.temporary_buffer[i];
            if self.consumed_as_part_of_an_attribute() {
                self.current_builder.push(cp_to_char(code_point));
            } else {
                let mut token = Token::new_character(code_point);
                token.start_position = self.nth_last_position(0);
                self.queued_tokens.push_back(token);
            }
        }
    }

    /// Check if we ran out of characters due to the insertion point
    /// (as opposed to plain EOF).
    fn is_at_insertion_point_at(&self, idx: usize) -> bool {
        if self.stop_at_insertion_point {
            if let Some(ip) = self.insertion_point {
                return idx >= ip;
            }
        }
        false
    }

    /// Case-insensitive match of upcoming input against a string.
    /// Returns Some(true) if matched and consumed, Some(false) if no match,
    /// None if ran out of characters at the insertion point.
    fn consume_next_if_match(&mut self, s: &str) -> Option<bool> {
        for (i, b) in s.bytes().enumerate() {
            match self.peek_code_point(i as isize) {
                None => {
                    if self.is_at_insertion_point_at(self.current_offset + i) {
                        return None;
                    }
                    return Some(false);
                }
                Some(cp) => {
                    if to_ascii_lowercase(cp) != to_ascii_lowercase(b as u32) {
                        return Some(false);
                    }
                }
            }
        }
        // All matched, consume them
        self.skip(s.len());
        Some(true)
    }

    /// Case-sensitive match of upcoming input against a string.
    fn consume_next_if_match_exact(&mut self, s: &str) -> Option<bool> {
        for (i, b) in s.bytes().enumerate() {
            match self.peek_code_point(i as isize) {
                None => {
                    if self.is_at_insertion_point_at(self.current_offset + i) {
                        return None;
                    }
                    return Some(false);
                }
                Some(cp) => {
                    if cp != b as u32 {
                        return Some(false);
                    }
                }
            }
        }
        self.skip(s.len());
        Some(true)
    }

    /// Get the next token from the tokenizer.
    pub fn next_token(&mut self, stop_at_insertion_point: bool, cdata_allowed: bool) -> Option<Token> {
        self.stop_at_insertion_point = stop_at_insertion_point;
        self.cdata_allowed = cdata_allowed;

        // Return queued tokens first.
        {
            let last = *self.source_positions.last().unwrap_or(&Position::default());
            self.source_positions.clear();
            self.source_positions.push(last);
        }

        if let Some(token) = self.queued_tokens.pop_front() {
            return Some(token);
        }

        if self.aborted {
            return None;
        }

        loop {
            // Check insertion point before consuming.
            if stop_at_insertion_point {
                if let Some(ip) = self.insertion_point {
                    if self.current_offset >= ip {
                        return None;
                    }
                }
            }

            let current_input_character = self.next_code_point();

            match self.state {
                // 13.2.5.1 Data state
                State::Data => {
                    match current_input_character {
                        Some(0x26) => {
                            // '&'
                            self.return_state = State::Data;
                            self.state = State::CharacterReference;
                            continue;
                        }
                        Some(0x3C) => {
                            // '<'
                            self.state = State::TagOpen;
                            continue;
                        }
                        Some(0x00) => {
                            // NULL - parse error
                            return self.return_character_token(0x00);
                        }
                        None => {
                            self.emit_eof();
                            return self.queued_tokens.pop_front();
                        }
                        Some(cp) => {
                            return self.return_character_token(cp);
                        }
                    }
                }

                // 13.2.5.2 RCDATA state
                State::RCDATA => match current_input_character {
                    Some(0x26) => {
                        self.return_state = State::RCDATA;
                        self.state = State::CharacterReference;
                        continue;
                    }
                    Some(0x3C) => {
                        self.state = State::RCDATALessThanSign;
                        continue;
                    }
                    Some(0x00) => {
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        return self.return_character_token(cp);
                    }
                },

                // 13.2.5.3 RAWTEXT state
                State::RAWTEXT => match current_input_character {
                    Some(0x3C) => {
                        self.state = State::RAWTEXTLessThanSign;
                        continue;
                    }
                    Some(0x00) => {
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        return self.return_character_token(cp);
                    }
                },

                // 13.2.5.4 Script data state
                State::ScriptData => match current_input_character {
                    Some(0x3C) => {
                        self.state = State::ScriptDataLessThanSign;
                        continue;
                    }
                    Some(0x00) => {
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        return self.return_character_token(cp);
                    }
                },

                // 13.2.5.5 PLAINTEXT state
                State::PLAINTEXT => match current_input_character {
                    Some(0x00) => {
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        return self.return_character_token(cp);
                    }
                },

                // 13.2.5.6 Tag open state
                State::TagOpen => match current_input_character {
                    Some(0x21) => {
                        self.state = State::MarkupDeclarationOpen;
                        continue;
                    }
                    Some(0x2F) => {
                        self.state = State::EndTagOpen;
                        continue;
                    }
                    Some(cp) if is_ascii_alpha(cp) => {
                        self.create_new_token(TokenType::StartTag);
                        self.reconsume(State::TagName);
                        continue;
                    }
                    Some(0x3F) => {
                        // '?' - parse error
                        self.create_new_token(TokenType::Comment);
                        self.current_token.start_position = self.nth_last_position(2);
                        self.reconsume(State::BogusComment);
                        continue;
                    }
                    None => {
                        // parse error
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        // parse error
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.reconsume(State::Data);
                        continue;
                    }
                },

                // 13.2.5.7 End tag open state
                State::EndTagOpen => match current_input_character {
                    Some(cp) if is_ascii_alpha(cp) => {
                        self.create_new_token(TokenType::EndTag);
                        self.reconsume(State::TagName);
                        continue;
                    }
                    Some(0x3E) => {
                        // '>' - parse error
                        self.state = State::Data;
                        continue;
                    }
                    None => {
                        // parse error
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.queued_tokens
                            .push_back(self.make_character_token(0x2F));
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        // parse error
                        self.create_new_token(TokenType::Comment);
                        self.reconsume(State::BogusComment);
                        continue;
                    }
                },

                // 13.2.5.8 Tag name state
                State::TagName => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        self.current_token.tag_name = self.consume_current_builder();
                        self.current_token.end_position = self.nth_last_position(1);
                        self.state = State::BeforeAttributeName;
                        continue;
                    }
                    Some(0x2F) => {
                        self.current_token.tag_name = self.consume_current_builder();
                        self.current_token.end_position = self.nth_last_position(0);
                        self.state = State::SelfClosingStartTag;
                        continue;
                    }
                    Some(0x3E) => {
                        self.current_token.tag_name = self.consume_current_builder();
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) if is_ascii_upper_alpha(cp) => {
                        self.current_builder
                            .push(char::from_u32(to_ascii_lowercase(cp)).unwrap());
                        self.current_token.end_position = self.nth_last_position(0);
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        self.current_token.end_position = self.nth_last_position(0);
                        continue;
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        self.current_token.end_position = self.nth_last_position(0);
                        continue;
                    }
                },

                // 13.2.5.32 Before attribute name state
                State::BeforeAttributeName => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        continue;
                    }
                    Some(0x2F) | Some(0x3E) | None => {
                        self.reconsume(State::AfterAttributeName);
                        continue;
                    }
                    Some(0x3D) => {
                        // '=' - parse error
                        self.current_token.attributes.push(Attribute::default());
                        self.current_builder.push('=');
                        self.state = State::AttributeName;
                        continue;
                    }
                    Some(_) => {
                        self.current_token.attributes.push(Attribute::default());
                        self.reconsume(State::AttributeName);
                        continue;
                    }
                },

                // 13.2.5.33 Attribute name state
                State::AttributeName => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        self.set_attribute_name();
                        self.reconsume(State::AfterAttributeName);
                        continue;
                    }
                    Some(0x2F) | Some(0x3E) | None => {
                        self.set_attribute_name();
                        self.reconsume(State::AfterAttributeName);
                        continue;
                    }
                    Some(0x3D) => {
                        self.set_attribute_name();
                        self.state = State::BeforeAttributeValue;
                        continue;
                    }
                    Some(cp) if is_ascii_upper_alpha(cp) => {
                        self.current_builder
                            .push(char::from_u32(to_ascii_lowercase(cp)).unwrap());
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    Some(cp @ (0x22 | 0x27 | 0x3C)) => {
                        // '"', '\'', '<' - parse error
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.34 After attribute name state
                State::AfterAttributeName => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        continue;
                    }
                    Some(0x2F) => {
                        self.state = State::SelfClosingStartTag;
                        continue;
                    }
                    Some(0x3D) => {
                        self.state = State::BeforeAttributeValue;
                        continue;
                    }
                    Some(0x3E) => {
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        self.current_token.attributes.push(Attribute::default());
                        self.reconsume(State::AttributeName);
                        continue;
                    }
                },

                // 13.2.5.35 Before attribute value state
                State::BeforeAttributeValue => {
                    let pos = self.nth_last_position(1);
                    if let Some(attr) = self.current_token.attributes.last_mut() {
                        attr.value_start_position = pos;
                    }
                    match current_input_character {
                        Some(cp) if is_whitespace(cp) => {
                            continue;
                        }
                        Some(0x22) => {
                            self.state = State::AttributeValueDoubleQuoted;
                            continue;
                        }
                        Some(0x27) => {
                            self.state = State::AttributeValueSingleQuoted;
                            continue;
                        }
                        Some(0x3E) => {
                            // parse error
                            self.state = State::Data;
                            self.emit_current_token();
                            return self.queued_tokens.pop_front();
                        }
                        _ => {
                            self.reconsume(State::AttributeValueUnquoted);
                            continue;
                        }
                    }
                }

                // 13.2.5.36 Attribute value (double-quoted) state
                State::AttributeValueDoubleQuoted => match current_input_character {
                    Some(0x22) => {
                        self.set_attribute_value();
                        self.state = State::AfterAttributeValueQuoted;
                        continue;
                    }
                    Some(0x26) => {
                        self.return_state = State::AttributeValueDoubleQuoted;
                        self.state = State::CharacterReference;
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.37 Attribute value (single-quoted) state
                State::AttributeValueSingleQuoted => match current_input_character {
                    Some(0x27) => {
                        self.set_attribute_value();
                        self.state = State::AfterAttributeValueQuoted;
                        continue;
                    }
                    Some(0x26) => {
                        self.return_state = State::AttributeValueSingleQuoted;
                        self.state = State::CharacterReference;
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.38 Attribute value (unquoted) state
                State::AttributeValueUnquoted => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        self.set_attribute_value();
                        self.state = State::BeforeAttributeName;
                        continue;
                    }
                    Some(0x26) => {
                        self.return_state = State::AttributeValueUnquoted;
                        self.state = State::CharacterReference;
                        continue;
                    }
                    Some(0x3E) => {
                        self.set_attribute_value();
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    Some(cp @ (0x22 | 0x27 | 0x3C | 0x3D | 0x60)) => {
                        // parse error, treat as anything else
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.39 After attribute value (quoted) state
                State::AfterAttributeValueQuoted => {
                    let pos = self.nth_last_position(1);
                    if let Some(attr) = self.current_token.attributes.last_mut() {
                        attr.value_end_position = pos;
                    }
                    match current_input_character {
                        Some(cp) if is_whitespace(cp) => {
                            self.state = State::BeforeAttributeName;
                            continue;
                        }
                        Some(0x2F) => {
                            self.state = State::SelfClosingStartTag;
                            continue;
                        }
                        Some(0x3E) => {
                            self.state = State::Data;
                            self.emit_current_token();
                            return self.queued_tokens.pop_front();
                        }
                        None => {
                            self.emit_eof();
                            return self.queued_tokens.pop_front();
                        }
                        Some(_) => {
                            // parse error
                            self.reconsume(State::BeforeAttributeName);
                            continue;
                        }
                    }
                }

                // 13.2.5.40 Self-closing start tag state
                State::SelfClosingStartTag => match current_input_character {
                    Some(0x3E) => {
                        self.current_token.self_closing = true;
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        // parse error
                        self.reconsume(State::BeforeAttributeName);
                        continue;
                    }
                },

                // 13.2.5.41 Bogus comment state
                State::BogusComment => match current_input_character {
                    Some(0x3E) => {
                        self.current_token.comment_data = self.consume_current_builder();
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        self.current_token.comment_data = self.consume_current_builder();
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.42 Markup declaration open state
                State::MarkupDeclarationOpen => {
                    // Don't consume next input character (reconsume).
                    if current_input_character.is_some() {
                        self.restore_to(self.prev_offset);
                    }

                    match self.consume_next_if_match_exact("--") {
                        Some(true) => {
                            self.create_new_token(TokenType::Comment);
                            self.current_token.start_position = self.nth_last_position(3);
                            self.state = State::CommentStart;
                            continue;
                        }
                        None => return None,
                        _ => {}
                    }
                    match self.consume_next_if_match("DOCTYPE") {
                        Some(true) => {
                            self.state = State::DOCTYPE;
                            continue;
                        }
                        None => return None,
                        _ => {}
                    }
                    match self.consume_next_if_match_exact("[CDATA[") {
                        Some(true) => {
                            if self.cdata_allowed {
                                self.state = State::CDATASection;
                            } else {
                                self.create_new_token(TokenType::Comment);
                                self.current_builder.push_str("[CDATA[");
                                self.state = State::BogusComment;
                            }
                            continue;
                        }
                        None => return None,
                        _ => {}
                    }
                    // parse error
                    self.create_new_token(TokenType::Comment);
                    self.state = State::BogusComment;
                    continue;
                }

                // 13.2.5.43 Comment start state
                State::CommentStart => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::CommentStartDash;
                        continue;
                    }
                    Some(0x3E) => {
                        // parse error
                        self.current_token.comment_data = self.consume_current_builder();
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    _ => {
                        self.reconsume(State::Comment);
                        continue;
                    }
                },

                // 13.2.5.44 Comment start dash state
                State::CommentStartDash => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::CommentEnd;
                        continue;
                    }
                    Some(0x3E) => {
                        // parse error
                        self.current_token.comment_data = self.consume_current_builder();
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        self.current_token.comment_data = self.consume_current_builder();
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        self.current_builder.push('-');
                        self.reconsume(State::Comment);
                        continue;
                    }
                },

                // 13.2.5.45 Comment state
                State::Comment => match current_input_character {
                    Some(0x3C) => {
                        self.current_builder.push('<');
                        self.state = State::CommentLessThanSign;
                        continue;
                    }
                    Some(0x2D) => {
                        self.state = State::CommentEndDash;
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    None => {
                        self.current_token.comment_data = self.consume_current_builder();
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.46 Comment less-than sign state
                State::CommentLessThanSign => match current_input_character {
                    Some(0x21) => {
                        self.current_builder.push('!');
                        self.state = State::CommentLessThanSignBang;
                        continue;
                    }
                    Some(0x3C) => {
                        self.current_builder.push('<');
                        continue;
                    }
                    _ => {
                        self.reconsume(State::Comment);
                        continue;
                    }
                },

                // 13.2.5.47 Comment less-than sign bang state
                State::CommentLessThanSignBang => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::CommentLessThanSignBangDash;
                        continue;
                    }
                    _ => {
                        self.reconsume(State::Comment);
                        continue;
                    }
                },

                // 13.2.5.48 Comment less-than sign bang dash state
                State::CommentLessThanSignBangDash => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::CommentLessThanSignBangDashDash;
                        continue;
                    }
                    _ => {
                        self.reconsume(State::CommentEndDash);
                        continue;
                    }
                },

                // 13.2.5.49 Comment less-than sign bang dash dash state
                State::CommentLessThanSignBangDashDash => match current_input_character {
                    Some(0x3E) | None => {
                        self.reconsume(State::CommentEnd);
                        continue;
                    }
                    _ => {
                        // parse error
                        self.reconsume(State::CommentEnd);
                        continue;
                    }
                },

                // 13.2.5.50 Comment end dash state
                State::CommentEndDash => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::CommentEnd;
                        continue;
                    }
                    None => {
                        self.current_token.comment_data = self.consume_current_builder();
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        self.current_builder.push('-');
                        self.reconsume(State::Comment);
                        continue;
                    }
                },

                // 13.2.5.51 Comment end state
                State::CommentEnd => match current_input_character {
                    Some(0x3E) => {
                        self.current_token.comment_data = self.consume_current_builder();
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    Some(0x21) => {
                        self.state = State::CommentEndBang;
                        continue;
                    }
                    Some(0x2D) => {
                        self.current_builder.push('-');
                        continue;
                    }
                    None => {
                        self.current_token.comment_data = self.consume_current_builder();
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        self.current_builder.push_str("--");
                        self.reconsume(State::Comment);
                        continue;
                    }
                },

                // 13.2.5.52 Comment end bang state
                State::CommentEndBang => match current_input_character {
                    Some(0x2D) => {
                        self.current_builder.push_str("--!");
                        self.state = State::CommentEndDash;
                        continue;
                    }
                    Some(0x3E) => {
                        // parse error
                        self.current_token.comment_data = self.consume_current_builder();
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        self.current_token.comment_data = self.consume_current_builder();
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        self.current_builder.push_str("--!");
                        self.reconsume(State::Comment);
                        continue;
                    }
                },

                // 13.2.5.53 DOCTYPE state
                State::DOCTYPE => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        self.state = State::BeforeDOCTYPEName;
                        continue;
                    }
                    Some(0x3E) => {
                        self.reconsume(State::BeforeDOCTYPEName);
                        continue;
                    }
                    None => {
                        self.create_new_token(TokenType::Doctype);
                        self.current_token.doctype_data = Some(DoctypeData {
                            force_quirks: true,
                            missing_name: true,
                            missing_public_identifier: true,
                            missing_system_identifier: true,
                            ..Default::default()
                        });
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        // parse error
                        self.reconsume(State::BeforeDOCTYPEName);
                        continue;
                    }
                },

                // 13.2.5.54 Before DOCTYPE name state
                State::BeforeDOCTYPEName => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        continue;
                    }
                    Some(cp) if is_ascii_upper_alpha(cp) => {
                        self.create_new_token(TokenType::Doctype);
                        self.current_token.doctype_data = Some(DoctypeData {
                            missing_name: false,
                            missing_public_identifier: true,
                            missing_system_identifier: true,
                            ..Default::default()
                        });
                        self.current_builder
                            .push(char::from_u32(to_ascii_lowercase(cp)).unwrap());
                        self.state = State::DOCTYPEName;
                        continue;
                    }
                    Some(0x00) => {
                        self.create_new_token(TokenType::Doctype);
                        self.current_token.doctype_data = Some(DoctypeData {
                            missing_name: false,
                            missing_public_identifier: true,
                            missing_system_identifier: true,
                            ..Default::default()
                        });
                        self.current_builder.push('\u{FFFD}');
                        self.state = State::DOCTYPEName;
                        continue;
                    }
                    Some(0x3E) => {
                        // parse error
                        self.create_new_token(TokenType::Doctype);
                        self.current_token.doctype_data = Some(DoctypeData {
                            force_quirks: true,
                            missing_name: true,
                            missing_public_identifier: true,
                            missing_system_identifier: true,
                            ..Default::default()
                        });
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        self.create_new_token(TokenType::Doctype);
                        self.current_token.doctype_data = Some(DoctypeData {
                            force_quirks: true,
                            missing_name: true,
                            missing_public_identifier: true,
                            missing_system_identifier: true,
                            ..Default::default()
                        });
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.create_new_token(TokenType::Doctype);
                        self.current_token.doctype_data = Some(DoctypeData {
                            missing_name: false,
                            missing_public_identifier: true,
                            missing_system_identifier: true,
                            ..Default::default()
                        });
                        self.current_builder
                            .push(cp_to_char(cp));
                        self.state = State::DOCTYPEName;
                        continue;
                    }
                },

                // 13.2.5.55 DOCTYPE name state
                State::DOCTYPEName => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        {
                            let name = self.consume_current_builder();
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.name = name;
                            }
                        }
                        self.state = State::AfterDOCTYPEName;
                        continue;
                    }
                    Some(0x3E) => {
                        {
                            let name = self.consume_current_builder();
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.name = name;
                            }
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) if is_ascii_upper_alpha(cp) => {
                        self.current_builder
                            .push(char::from_u32(to_ascii_lowercase(cp)).unwrap());
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    None => {
                        let name = self.consume_current_builder();
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.name = name;
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.56 After DOCTYPE name state
                State::AfterDOCTYPEName => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        continue;
                    }
                    Some(0x3E) => {
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        // Reconsume and try PUBLIC/SYSTEM
                        if current_input_character.is_some() {
                            self.restore_to(self.prev_offset);
                        }
                        match self.consume_next_if_match("PUBLIC") {
                            Some(true) => {
                                self.state = State::AfterDOCTYPEPublicKeyword;
                                continue;
                            }
                            None => return None,
                            _ => {}
                        }
                        match self.consume_next_if_match("SYSTEM") {
                            Some(true) => {
                                self.state = State::AfterDOCTYPESystemKeyword;
                                continue;
                            }
                            None => return None,
                            _ => {}
                        }
                        // Re-consume the character we put back
                        let _ = self.next_code_point();
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.reconsume(State::BogusDOCTYPE);
                        continue;
                    }
                },

                // 13.2.5.57 After DOCTYPE public keyword state
                State::AfterDOCTYPEPublicKeyword => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        self.state = State::BeforeDOCTYPEPublicIdentifier;
                        continue;
                    }
                    Some(0x22) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.missing_public_identifier = false;
                        }
                        self.state = State::DOCTYPEPublicIdentifierDoubleQuoted;
                        continue;
                    }
                    Some(0x27) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.missing_public_identifier = false;
                        }
                        self.state = State::DOCTYPEPublicIdentifierSingleQuoted;
                        continue;
                    }
                    Some(0x3E) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.reconsume(State::BogusDOCTYPE);
                        continue;
                    }
                },

                // 13.2.5.58 Before DOCTYPE public identifier state
                State::BeforeDOCTYPEPublicIdentifier => match current_input_character {
                    Some(cp) if is_whitespace(cp) => continue,
                    Some(0x22) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.missing_public_identifier = false;
                        }
                        self.state = State::DOCTYPEPublicIdentifierDoubleQuoted;
                        continue;
                    }
                    Some(0x27) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.missing_public_identifier = false;
                        }
                        self.state = State::DOCTYPEPublicIdentifierSingleQuoted;
                        continue;
                    }
                    Some(0x3E) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.reconsume(State::BogusDOCTYPE);
                        continue;
                    }
                },

                // 13.2.5.59 DOCTYPE public identifier (double-quoted) state
                State::DOCTYPEPublicIdentifierDoubleQuoted => match current_input_character {
                    Some(0x22) => {
                        {
                            let val = self.consume_current_builder();
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.public_identifier = val;
                            }
                        }
                        self.state = State::AfterDOCTYPEPublicIdentifier;
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    Some(0x3E) => {
                        {
                            let val = self.consume_current_builder();
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.public_identifier = val;
                                dd.force_quirks = true;
                            }
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        {
                            let val = self.consume_current_builder();
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.public_identifier = val;
                                dd.force_quirks = true;
                            }
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.60 DOCTYPE public identifier (single-quoted) state
                State::DOCTYPEPublicIdentifierSingleQuoted => match current_input_character {
                    Some(0x27) => {
                        {
                            let val = self.consume_current_builder();
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.public_identifier = val;
                            }
                        }
                        self.state = State::AfterDOCTYPEPublicIdentifier;
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    Some(0x3E) => {
                        {
                            let val = self.consume_current_builder();
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.public_identifier = val;
                                dd.force_quirks = true;
                            }
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        {
                            let val = self.consume_current_builder();
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.public_identifier = val;
                                dd.force_quirks = true;
                            }
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.61 After DOCTYPE public identifier state
                State::AfterDOCTYPEPublicIdentifier => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        self.state = State::BetweenDOCTYPEPublicAndSystemIdentifiers;
                        continue;
                    }
                    Some(0x3E) => {
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    Some(0x22) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = String::new();
                            dd.missing_system_identifier = false;
                        }
                        self.state = State::DOCTYPESystemIdentifierDoubleQuoted;
                        continue;
                    }
                    Some(0x27) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = String::new();
                            dd.missing_system_identifier = false;
                        }
                        self.state = State::DOCTYPESystemIdentifierSingleQuoted;
                        continue;
                    }
                    None => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.reconsume(State::BogusDOCTYPE);
                        continue;
                    }
                },

                // 13.2.5.62 Between DOCTYPE public and system identifiers state
                State::BetweenDOCTYPEPublicAndSystemIdentifiers => {
                    match current_input_character {
                        Some(cp) if is_whitespace(cp) => continue,
                        Some(0x3E) => {
                            self.state = State::Data;
                            self.emit_current_token();
                            return self.queued_tokens.pop_front();
                        }
                        Some(0x22) => {
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.system_identifier = String::new();
                                dd.missing_system_identifier = false;
                            }
                            self.state = State::DOCTYPESystemIdentifierDoubleQuoted;
                            continue;
                        }
                        Some(0x27) => {
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.system_identifier = String::new();
                                dd.missing_system_identifier = false;
                            }
                            self.state = State::DOCTYPESystemIdentifierSingleQuoted;
                            continue;
                        }
                        None => {
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.force_quirks = true;
                            }
                            self.emit_current_token_followed_by_eof();
                            return self.queued_tokens.pop_front();
                        }
                        Some(_) => {
                            if let Some(ref mut dd) = self.current_token.doctype_data {
                                dd.force_quirks = true;
                            }
                            self.reconsume(State::BogusDOCTYPE);
                            continue;
                        }
                    }
                }

                // 13.2.5.63 After DOCTYPE system keyword state
                State::AfterDOCTYPESystemKeyword => match current_input_character {
                    Some(cp) if is_whitespace(cp) => {
                        self.state = State::BeforeDOCTYPESystemIdentifier;
                        continue;
                    }
                    Some(0x22) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = String::new();
                            dd.missing_system_identifier = false;
                        }
                        self.state = State::DOCTYPESystemIdentifierDoubleQuoted;
                        continue;
                    }
                    Some(0x27) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = String::new();
                            dd.missing_system_identifier = false;
                        }
                        self.state = State::DOCTYPESystemIdentifierSingleQuoted;
                        continue;
                    }
                    Some(0x3E) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.reconsume(State::BogusDOCTYPE);
                        continue;
                    }
                },

                // 13.2.5.64 Before DOCTYPE system identifier state
                State::BeforeDOCTYPESystemIdentifier => match current_input_character {
                    Some(cp) if is_whitespace(cp) => continue,
                    Some(0x22) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = String::new();
                            dd.missing_system_identifier = false;
                        }
                        self.state = State::DOCTYPESystemIdentifierDoubleQuoted;
                        continue;
                    }
                    Some(0x27) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = String::new();
                            dd.missing_system_identifier = false;
                        }
                        self.state = State::DOCTYPESystemIdentifierSingleQuoted;
                        continue;
                    }
                    Some(0x3E) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.reconsume(State::BogusDOCTYPE);
                        continue;
                    }
                },

                // 13.2.5.65 DOCTYPE system identifier (double-quoted) state
                State::DOCTYPESystemIdentifierDoubleQuoted => match current_input_character {
                    Some(0x22) => {
                        let sys_id = self.consume_current_builder();
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = sys_id;
                        }
                        self.state = State::AfterDOCTYPESystemIdentifier;
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    Some(0x3E) => {
                        let sys_id = self.consume_current_builder();
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = sys_id;
                            dd.force_quirks = true;
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        let sys_id = self.consume_current_builder();
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = sys_id;
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.66 DOCTYPE system identifier (single-quoted) state
                State::DOCTYPESystemIdentifierSingleQuoted => match current_input_character {
                    Some(0x27) => {
                        let sys_id = self.consume_current_builder();
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = sys_id;
                        }
                        self.state = State::AfterDOCTYPESystemIdentifier;
                        continue;
                    }
                    Some(0x00) => {
                        self.current_builder.push('\u{FFFD}');
                        continue;
                    }
                    Some(0x3E) => {
                        let sys_id = self.consume_current_builder();
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = sys_id;
                            dd.force_quirks = true;
                        }
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        let sys_id = self.consume_current_builder();
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.system_identifier = sys_id;
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.current_builder
                            .push(cp_to_char(cp));
                        continue;
                    }
                },

                // 13.2.5.67 After DOCTYPE system identifier state
                State::AfterDOCTYPESystemIdentifier => match current_input_character {
                    Some(cp) if is_whitespace(cp) => continue,
                    Some(0x3E) => {
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    None => {
                        if let Some(ref mut dd) = self.current_token.doctype_data {
                            dd.force_quirks = true;
                        }
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => {
                        // parse error - do NOT set force_quirks
                        self.reconsume(State::BogusDOCTYPE);
                        continue;
                    }
                },

                // 13.2.5.68 Bogus DOCTYPE state
                State::BogusDOCTYPE => match current_input_character {
                    Some(0x3E) => {
                        self.state = State::Data;
                        self.emit_current_token();
                        return self.queued_tokens.pop_front();
                    }
                    Some(0x00) => continue,
                    None => {
                        self.emit_current_token_followed_by_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(_) => continue,
                },

                // 13.2.5.69 CDATA section state
                State::CDATASection => match current_input_character {
                    Some(0x5D) => {
                        self.state = State::CDATASectionBracket;
                        continue;
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        return self.return_character_token(cp);
                    }
                },

                // 13.2.5.70 CDATA section bracket state
                State::CDATASectionBracket => match current_input_character {
                    Some(0x5D) => {
                        self.state = State::CDATASectionEnd;
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x5D));
                        self.reconsume(State::CDATASection);
                        continue;
                    }
                },

                // 13.2.5.71 CDATA section end state
                State::CDATASectionEnd => match current_input_character {
                    Some(0x5D) => {
                        return self.return_character_token(0x5D);
                    }
                    Some(0x3E) => {
                        self.state = State::Data;
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x5D));
                        self.queued_tokens
                            .push_back(self.make_character_token(0x5D));
                        self.reconsume(State::CDATASection);
                        continue;
                    }
                },

                // 13.2.5.72 Character reference state
                State::CharacterReference => {
                    self.temporary_buffer.clear();
                    self.temporary_buffer.push(0x26); // '&'
                    self.entity_matcher = NamedCharacterReferenceMatcher::new();

                    match current_input_character {
                        Some(cp) if is_ascii_alphanumeric(cp) => {
                            self.reconsume(State::NamedCharacterReference);
                            continue;
                        }
                        Some(0x23) => {
                            self.temporary_buffer.push(0x23);
                            self.state = State::NumericCharacterReference;
                            continue;
                        }
                        _ => {
                            self.flush_codepoints_consumed_as_character_reference();
                            self.reconsume_in_return_state(current_input_character);
                            if !self.queued_tokens.is_empty() {
                                return self.queued_tokens.pop_front();
                            }
                            continue;
                        }
                    }
                }

                // 13.2.5.73 Named character reference state
                State::NamedCharacterReference => {
                    // Insertion-point path: feed one character at a time.
                    if self.stop_at_insertion_point && self.insertion_point.is_some() {
                        if let Some(cp) = current_input_character {
                            if self.entity_matcher.try_consume_code_point(cp) {
                                self.temporary_buffer.push(cp);
                                continue;
                            }
                            // Character not accepted by matcher. Reconsume it.
                            self.restore_to(self.prev_offset);
                        } else if self.is_at_insertion_point_at(self.current_offset) {
                            // At insertion point with no more chars -- pause.
                            return None;
                        }
                        // Fall through to resolution.
                    } else {
                        // Normal path: feed all remaining chars in a tight loop.
                        if current_input_character.is_some() {
                            self.restore_to(self.prev_offset);
                        }
                        let limit = self.input.len();
                        while self.current_offset < limit {
                            let cp = self.input[self.current_offset];
                            if !self.entity_matcher.try_consume_code_point(cp) {
                                break;
                            }
                            self.temporary_buffer.push(cp);
                            self.skip(1);
                        }
                    }

                    // Resolution: backtrack overconsumed characters.
                    let overconsumed = self.entity_matcher.overconsumed_code_points() as usize;
                    if overconsumed > 0 {
                        self.restore_to(self.current_offset - overconsumed);
                        self.temporary_buffer.truncate(
                            self.temporary_buffer.len() - overconsumed,
                        );
                    }

                    if let Some((first_cp, second_cp)) = self.entity_matcher.code_points() {
                        let ends_with_semi = self.entity_matcher.last_match_ends_with_semicolon();

                        // Check special attribute handling.
                        let next_cp = self.peek_code_point(0);
                        if self.consumed_as_part_of_an_attribute()
                            && !ends_with_semi
                            && next_cp.map_or(false, |c| {
                                c == 0x3D || is_ascii_alphanumeric(c)
                            })
                        {
                            self.flush_codepoints_consumed_as_character_reference();
                            self.state = self.return_state;
                            if !self.queued_tokens.is_empty() {
                                return self.queued_tokens.pop_front();
                            }
                            continue;
                        }

                        if !ends_with_semi {
                            // parse error
                        }

                        self.temporary_buffer.clear();
                        self.temporary_buffer.push(first_cp);
                        if second_cp != 0 {
                            self.temporary_buffer.push(second_cp);
                        }
                        self.flush_codepoints_consumed_as_character_reference();
                        self.state = self.return_state;
                        if !self.queued_tokens.is_empty() {
                            return self.queued_tokens.pop_front();
                        }
                        continue;
                    } else {
                        // No match found.
                        self.flush_codepoints_consumed_as_character_reference();
                        self.state = State::AmbiguousAmpersand;
                        continue;
                    }
                }

                // 13.2.5.74 Ambiguous ampersand state
                State::AmbiguousAmpersand => match current_input_character {
                    Some(cp) if is_ascii_alphanumeric(cp) => {
                        if self.consumed_as_part_of_an_attribute() {
                            self.current_builder
                                .push(cp_to_char(cp));
                            continue;
                        } else {
                            return self.return_character_token(cp);
                        }
                    }
                    Some(0x3B) => {
                        // parse error
                        self.reconsume_in_return_state(current_input_character);
                        if !self.queued_tokens.is_empty() {
                            return self.queued_tokens.pop_front();
                        }
                        continue;
                    }
                    _ => {
                        self.reconsume_in_return_state(current_input_character);
                        if !self.queued_tokens.is_empty() {
                            return self.queued_tokens.pop_front();
                        }
                        continue;
                    }
                },

                // 13.2.5.75 Numeric character reference state
                State::NumericCharacterReference => {
                    self.character_reference_code = 0;
                    match current_input_character {
                        Some(0x78) | Some(0x58) => {
                            // 'x' or 'X'
                            self.temporary_buffer
                                .push(current_input_character.unwrap());
                            self.state = State::HexadecimalCharacterReferenceStart;
                            continue;
                        }
                        _ => {
                            self.reconsume(State::DecimalCharacterReferenceStart);
                            continue;
                        }
                    }
                }

                // 13.2.5.76 Hexadecimal character reference start state
                State::HexadecimalCharacterReferenceStart => match current_input_character {
                    Some(cp) if is_ascii_hex_digit(cp) => {
                        self.reconsume(State::HexadecimalCharacterReference);
                        continue;
                    }
                    _ => {
                        // parse error
                        self.flush_codepoints_consumed_as_character_reference();
                        self.reconsume_in_return_state(current_input_character);
                        if !self.queued_tokens.is_empty() {
                            return self.queued_tokens.pop_front();
                        }
                        continue;
                    }
                },

                // 13.2.5.77 Decimal character reference start state
                State::DecimalCharacterReferenceStart => match current_input_character {
                    Some(cp) if is_ascii_digit(cp) => {
                        self.reconsume(State::DecimalCharacterReference);
                        continue;
                    }
                    _ => {
                        // parse error
                        self.flush_codepoints_consumed_as_character_reference();
                        self.reconsume_in_return_state(current_input_character);
                        if !self.queued_tokens.is_empty() {
                            return self.queued_tokens.pop_front();
                        }
                        continue;
                    }
                },

                // 13.2.5.78 Hexadecimal character reference state
                State::HexadecimalCharacterReference => match current_input_character {
                    Some(cp) if is_ascii_digit(cp) => {
                        self.character_reference_code =
                            self.character_reference_code.wrapping_mul(16).wrapping_add(cp - 0x30);
                        continue;
                    }
                    Some(cp @ 0x41..=0x46) => {
                        self.character_reference_code = self
                            .character_reference_code
                            .wrapping_mul(16)
                            .wrapping_add(cp - 0x37);
                        continue;
                    }
                    Some(cp @ 0x61..=0x66) => {
                        self.character_reference_code = self
                            .character_reference_code
                            .wrapping_mul(16)
                            .wrapping_add(cp - 0x57);
                        continue;
                    }
                    Some(0x3B) => {
                        self.state = State::NumericCharacterReferenceEnd;
                        continue;
                    }
                    _ => {
                        // parse error
                        self.reconsume(State::NumericCharacterReferenceEnd);
                        continue;
                    }
                },

                // 13.2.5.79 Decimal character reference state
                State::DecimalCharacterReference => match current_input_character {
                    Some(cp) if is_ascii_digit(cp) => {
                        self.character_reference_code =
                            self.character_reference_code.wrapping_mul(10).wrapping_add(cp - 0x30);
                        continue;
                    }
                    Some(0x3B) => {
                        self.state = State::NumericCharacterReferenceEnd;
                        continue;
                    }
                    _ => {
                        // parse error
                        self.reconsume(State::NumericCharacterReferenceEnd);
                        continue;
                    }
                },

                // 13.2.5.80 Numeric character reference end state
                State::NumericCharacterReferenceEnd => {
                    // Don't consume
                    if current_input_character.is_some() {
                        self.restore_to(self.prev_offset);
                    }

                    let code = self.character_reference_code;
                    let code = if code == 0 {
                        0xFFFD
                    } else if code > 0x10FFFF {
                        0xFFFD
                    } else if is_surrogate(code) {
                        0xFFFD
                    } else if is_noncharacter(code) {
                        code // parse error but keep value
                    } else if let Some(replacement) = numeric_char_ref_replacement(code) {
                        replacement
                    } else if code == 0x0D {
                        0x000D // parse error but keep
                    } else if code != 0 && code <= 0x1F && code != 0x09 && code != 0x0A && code != 0x0C {
                        code // control char, parse error but keep
                    } else if (0x7F..=0x9F).contains(&code) {
                        if let Some(replacement) = numeric_char_ref_replacement(code) {
                            replacement
                        } else {
                            code
                        }
                    } else {
                        code
                    };

                    self.temporary_buffer.clear();
                    self.temporary_buffer.push(code);
                    self.flush_codepoints_consumed_as_character_reference();
                    self.state = self.return_state;
                    if !self.queued_tokens.is_empty() {
                        return self.queued_tokens.pop_front();
                    }
                    continue;
                }

                // RCDATA less-than sign state
                State::RCDATALessThanSign => match current_input_character {
                    Some(0x2F) => {
                        self.temporary_buffer.clear();
                        self.state = State::RCDATAEndTagOpen;
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.reconsume(State::RCDATA);
                        continue;
                    }
                },

                // RCDATA end tag open state
                State::RCDATAEndTagOpen => match current_input_character {
                    Some(cp) if is_ascii_alpha(cp) => {
                        self.create_new_token(TokenType::EndTag);
                        self.reconsume(State::RCDATAEndTagName);
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.queued_tokens
                            .push_back(self.make_character_token(0x2F));
                        self.reconsume(State::RCDATA);
                        continue;
                    }
                },

                // RCDATA end tag name state
                State::RCDATAEndTagName => {
                    self.handle_rawtext_end_tag_name(current_input_character, State::RCDATA);
                    if !self.queued_tokens.is_empty() {
                        return self.queued_tokens.pop_front();
                    }
                    continue;
                }

                // RAWTEXT less-than sign state
                State::RAWTEXTLessThanSign => match current_input_character {
                    Some(0x2F) => {
                        self.temporary_buffer.clear();
                        self.state = State::RAWTEXTEndTagOpen;
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.reconsume(State::RAWTEXT);
                        continue;
                    }
                },

                // RAWTEXT end tag open state
                State::RAWTEXTEndTagOpen => match current_input_character {
                    Some(cp) if is_ascii_alpha(cp) => {
                        self.create_new_token(TokenType::EndTag);
                        self.reconsume(State::RAWTEXTEndTagName);
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.queued_tokens
                            .push_back(self.make_character_token(0x2F));
                        self.reconsume(State::RAWTEXT);
                        continue;
                    }
                },

                // RAWTEXT end tag name state
                State::RAWTEXTEndTagName => {
                    self.handle_rawtext_end_tag_name(
                        current_input_character,
                        State::RAWTEXT,
                    );
                    if !self.queued_tokens.is_empty() {
                        return self.queued_tokens.pop_front();
                    }
                    continue;
                }

                // Script data less-than sign state
                State::ScriptDataLessThanSign => match current_input_character {
                    Some(0x2F) => {
                        self.temporary_buffer.clear();
                        self.state = State::ScriptDataEndTagOpen;
                        continue;
                    }
                    Some(0x21) => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.queued_tokens
                            .push_back(self.make_character_token(0x21));
                        self.state = State::ScriptDataEscapeStart;
                        return self.queued_tokens.pop_front();
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.reconsume(State::ScriptData);
                        continue;
                    }
                },

                // Script data end tag open state
                State::ScriptDataEndTagOpen => match current_input_character {
                    Some(cp) if is_ascii_alpha(cp) => {
                        self.create_new_token(TokenType::EndTag);
                        self.reconsume(State::ScriptDataEndTagName);
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.queued_tokens
                            .push_back(self.make_character_token(0x2F));
                        self.reconsume(State::ScriptData);
                        continue;
                    }
                },

                // Script data end tag name state
                State::ScriptDataEndTagName => {
                    self.handle_rawtext_end_tag_name(
                        current_input_character,
                        State::ScriptData,
                    );
                    if !self.queued_tokens.is_empty() {
                        return self.queued_tokens.pop_front();
                    }
                    continue;
                }

                // Script data escape start state
                State::ScriptDataEscapeStart => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::ScriptDataEscapeStartDash;
                        return self.return_character_token(0x2D);
                    }
                    _ => {
                        self.reconsume(State::ScriptData);
                        continue;
                    }
                },

                // Script data escape start dash state
                State::ScriptDataEscapeStartDash => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::ScriptDataEscapedDashDash;
                        return self.return_character_token(0x2D);
                    }
                    _ => {
                        self.reconsume(State::ScriptData);
                        continue;
                    }
                },

                // Script data escaped state
                State::ScriptDataEscaped => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::ScriptDataEscapedDash;
                        return self.return_character_token(0x2D);
                    }
                    Some(0x3C) => {
                        self.state = State::ScriptDataEscapedLessThanSign;
                        continue;
                    }
                    Some(0x00) => {
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        return self.return_character_token(cp);
                    }
                },

                // Script data escaped dash state
                State::ScriptDataEscapedDash => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::ScriptDataEscapedDashDash;
                        return self.return_character_token(0x2D);
                    }
                    Some(0x3C) => {
                        self.state = State::ScriptDataEscapedLessThanSign;
                        continue;
                    }
                    Some(0x00) => {
                        self.state = State::ScriptDataEscaped;
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.state = State::ScriptDataEscaped;
                        return self.return_character_token(cp);
                    }
                },

                // Script data escaped dash dash state
                State::ScriptDataEscapedDashDash => match current_input_character {
                    Some(0x2D) => {
                        return self.return_character_token(0x2D);
                    }
                    Some(0x3C) => {
                        self.state = State::ScriptDataEscapedLessThanSign;
                        continue;
                    }
                    Some(0x3E) => {
                        self.state = State::ScriptData;
                        return self.return_character_token(0x3E);
                    }
                    Some(0x00) => {
                        self.state = State::ScriptDataEscaped;
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.state = State::ScriptDataEscaped;
                        return self.return_character_token(cp);
                    }
                },

                // Script data escaped less-than sign state
                State::ScriptDataEscapedLessThanSign => match current_input_character {
                    Some(0x2F) => {
                        self.temporary_buffer.clear();
                        self.state = State::ScriptDataEscapedEndTagOpen;
                        continue;
                    }
                    Some(cp) if is_ascii_alpha(cp) => {
                        self.temporary_buffer.clear();
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.reconsume(State::ScriptDataDoubleEscapeStart);
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.reconsume(State::ScriptDataEscaped);
                        continue;
                    }
                },

                // Script data escaped end tag open state
                State::ScriptDataEscapedEndTagOpen => match current_input_character {
                    Some(cp) if is_ascii_alpha(cp) => {
                        self.create_new_token(TokenType::EndTag);
                        self.reconsume(State::ScriptDataEscapedEndTagName);
                        continue;
                    }
                    _ => {
                        self.queued_tokens
                            .push_back(self.make_character_token(0x3C));
                        self.queued_tokens
                            .push_back(self.make_character_token(0x2F));
                        self.reconsume(State::ScriptDataEscaped);
                        continue;
                    }
                },

                // Script data escaped end tag name state
                State::ScriptDataEscapedEndTagName => {
                    self.handle_rawtext_end_tag_name(
                        current_input_character,
                        State::ScriptDataEscaped,
                    );
                    if !self.queued_tokens.is_empty() {
                        return self.queued_tokens.pop_front();
                    }
                    continue;
                }

                // Script data double escape start state
                State::ScriptDataDoubleEscapeStart => match current_input_character {
                    Some(cp) if is_whitespace(cp) || cp == 0x2F || cp == 0x3E => {
                        if self.temporary_buffer_equals_script() {
                            self.state = State::ScriptDataDoubleEscaped;
                        } else {
                            self.state = State::ScriptDataEscaped;
                        }
                        return self.return_character_token(cp);
                    }
                    Some(cp) if is_ascii_upper_alpha(cp) => {
                        self.temporary_buffer.push(to_ascii_lowercase(cp));
                        return self.return_character_token(cp);
                    }
                    Some(cp) if is_ascii_lower_alpha(cp) => {
                        self.temporary_buffer.push(cp);
                        return self.return_character_token(cp);
                    }
                    _ => {
                        self.reconsume(State::ScriptDataEscaped);
                        continue;
                    }
                },

                // Script data double escaped state
                State::ScriptDataDoubleEscaped => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::ScriptDataDoubleEscapedDash;
                        return self.return_character_token(0x2D);
                    }
                    Some(0x3C) => {
                        self.state = State::ScriptDataDoubleEscapedLessThanSign;
                        return self.return_character_token(0x3C);
                    }
                    Some(0x00) => {
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        return self.return_character_token(cp);
                    }
                },

                // Script data double escaped dash state
                State::ScriptDataDoubleEscapedDash => match current_input_character {
                    Some(0x2D) => {
                        self.state = State::ScriptDataDoubleEscapedDashDash;
                        return self.return_character_token(0x2D);
                    }
                    Some(0x3C) => {
                        self.state = State::ScriptDataDoubleEscapedLessThanSign;
                        return self.return_character_token(0x3C);
                    }
                    Some(0x00) => {
                        self.state = State::ScriptDataDoubleEscaped;
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.state = State::ScriptDataDoubleEscaped;
                        return self.return_character_token(cp);
                    }
                },

                // Script data double escaped dash dash state
                State::ScriptDataDoubleEscapedDashDash => match current_input_character {
                    Some(0x2D) => {
                        return self.return_character_token(0x2D);
                    }
                    Some(0x3C) => {
                        self.state = State::ScriptDataDoubleEscapedLessThanSign;
                        return self.return_character_token(0x3C);
                    }
                    Some(0x3E) => {
                        self.state = State::ScriptData;
                        return self.return_character_token(0x3E);
                    }
                    Some(0x00) => {
                        self.state = State::ScriptDataDoubleEscaped;
                        return self.return_character_token(0xFFFD);
                    }
                    None => {
                        self.emit_eof();
                        return self.queued_tokens.pop_front();
                    }
                    Some(cp) => {
                        self.state = State::ScriptDataDoubleEscaped;
                        return self.return_character_token(cp);
                    }
                },

                // Script data double escaped less-than sign state
                State::ScriptDataDoubleEscapedLessThanSign => match current_input_character {
                    Some(0x2F) => {
                        self.temporary_buffer.clear();
                        self.state = State::ScriptDataDoubleEscapeEnd;
                        return self.return_character_token(0x2F);
                    }
                    _ => {
                        self.reconsume(State::ScriptDataDoubleEscaped);
                        continue;
                    }
                },

                // Script data double escape end state
                State::ScriptDataDoubleEscapeEnd => match current_input_character {
                    Some(cp) if is_whitespace(cp) || cp == 0x2F || cp == 0x3E => {
                        if self.temporary_buffer_equals_script() {
                            self.state = State::ScriptDataEscaped;
                        } else {
                            self.state = State::ScriptDataDoubleEscaped;
                        }
                        return self.return_character_token(cp);
                    }
                    Some(cp) if is_ascii_upper_alpha(cp) => {
                        self.temporary_buffer.push(to_ascii_lowercase(cp));
                        return self.return_character_token(cp);
                    }
                    Some(cp) if is_ascii_lower_alpha(cp) => {
                        self.temporary_buffer.push(cp);
                        return self.return_character_token(cp);
                    }
                    _ => {
                        self.reconsume(State::ScriptDataDoubleEscaped);
                        continue;
                    }
                },
            }
        }
    }

    // -- Helper methods --

    fn reconsume(&mut self, new_state: State) {
        self.state = new_state;
        if self.current_offset > 0 {
            self.restore_to(self.prev_offset);
        }
    }

    fn reconsume_in_return_state(&mut self, current_input_character: Option<u32>) {
        self.state = self.return_state;
        if current_input_character.is_some() {
            self.restore_to(self.prev_offset);
        }
    }

    #[inline]
    fn make_character_token(&self, code_point: u32) -> Token {
        let mut token = Token::new_character(code_point);
        token.start_position = self.nth_last_position(0);
        token
    }

    fn set_attribute_name(&mut self) {
        let name = self.consume_current_builder();
        let name_start = self.nth_last_position(1);
        let name_end = self.nth_last_position(0);
        if let Some(attr) = self.current_token.attributes.last_mut() {
            attr.local_name = name;
            attr.name_start_position = name_start;
            attr.name_end_position = name_end;
        }
    }

    fn set_attribute_value(&mut self) {
        let value = self.consume_current_builder();
        let value_end = self.nth_last_position(0);
        if let Some(attr) = self.current_token.attributes.last_mut() {
            attr.value = value;
            attr.value_end_position = value_end;
        }
    }

    fn temporary_buffer_equals_script(&self) -> bool {
        self.temporary_buffer == [0x73, 0x63, 0x72, 0x69, 0x70, 0x74]
    }

    /// Handle the common end-tag-name pattern shared by RCDATA, RAWTEXT,
    /// ScriptData, and ScriptDataEscaped end tag name states.
    fn handle_rawtext_end_tag_name(
        &mut self,
        current_input_character: Option<u32>,
        fallback_state: State,
    ) {
        match current_input_character {
            Some(cp) if is_whitespace(cp) || cp == 0x2F || cp == 0x3E => {
                self.current_token.tag_name = self.consume_current_builder();
                if self.current_end_tag_token_is_appropriate() {
                    match cp {
                        _ if is_whitespace(cp) => {
                            self.state = State::BeforeAttributeName;
                            return;
                        }
                        0x2F => {
                            self.state = State::SelfClosingStartTag;
                            return;
                        }
                        0x3E => {
                            self.state = State::Data;
                            self.emit_current_token();
                            return;
                        }
                        _ => {}
                    }
                }
                // Not appropriate - emit buffered chars
                self.queued_tokens
                    .push_back(self.make_character_token(0x3C));
                self.queued_tokens
                    .push_back(self.make_character_token(0x2F));
                for i in 0..self.temporary_buffer.len() {
                    let cp = self.temporary_buffer[i];
                    self.queued_tokens
                        .push_back(self.make_character_token(cp));
                }
                self.current_builder.clear();
                self.reconsume(fallback_state);
            }
            Some(cp) if is_ascii_upper_alpha(cp) => {
                self.current_builder
                    .push(char::from_u32(to_ascii_lowercase(cp)).unwrap());
                self.temporary_buffer.push(cp);
            }
            Some(cp) if is_ascii_lower_alpha(cp) => {
                self.current_builder
                    .push(char::from_u32(cp).unwrap());
                self.temporary_buffer.push(cp);
            }
            _ => {
                self.queued_tokens
                    .push_back(self.make_character_token(0x3C));
                self.queued_tokens
                    .push_back(self.make_character_token(0x2F));
                for i in 0..self.temporary_buffer.len() {
                    let cp = self.temporary_buffer[i];
                    self.queued_tokens
                        .push_back(self.make_character_token(cp));
                }
                self.current_builder.clear();
                self.reconsume(fallback_state);
            }
        }
    }
}
