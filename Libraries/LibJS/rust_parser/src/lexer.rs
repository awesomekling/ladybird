/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::token::{Token, TokenType};

/// State for tracking template literal nesting.
struct TemplateState {
    in_expr: bool,
    open_bracket_count: u32,
}

pub struct Lexer<'a> {
    source: &'a [u16],
    position: usize,
    current_code_unit: u16,
    eof: bool,
    line_number: u32,
    line_column: u32,
    current_token_type: TokenType,
    regex_is_in_character_class: bool,
    allow_html_comments: bool,
    template_states: Vec<TemplateState>,
}

// Unicode constants
const NO_BREAK_SPACE: u16 = 0x00A0;
const ZERO_WIDTH_NON_JOINER: u32 = 0x200C;
const ZERO_WIDTH_JOINER: u32 = 0x200D;
const LINE_SEPARATOR: u16 = 0x2028;
const PARAGRAPH_SEPARATOR: u16 = 0x2029;
const ZERO_WIDTH_NO_BREAK_SPACE: u16 = 0xFEFF;

fn is_ascii(cu: u16) -> bool {
    cu < 128
}

fn is_ascii_alpha(cp: u32) -> bool {
    matches!(cp as u8, b'A'..=b'Z' | b'a'..=b'z') && cp < 128
}

fn is_ascii_digit(cu: u16) -> bool {
    cu >= b'0' as u16 && cu <= b'9' as u16
}

fn is_ascii_digit_cp(cp: u32) -> bool {
    cp >= b'0' as u32 && cp <= b'9' as u32
}

fn is_ascii_hex_digit(cu: u16) -> bool {
    is_ascii_digit(cu) || (cu >= b'a' as u16 && cu <= b'f' as u16) || (cu >= b'A' as u16 && cu <= b'F' as u16)
}

fn is_ascii_alphanumeric(cp: u32) -> bool {
    is_ascii_alpha(cp) || is_ascii_digit_cp(cp)
}

fn is_ascii_space(cu: u16) -> bool {
    matches!(cu, 0x09 | 0x0A | 0x0B | 0x0C | 0x0D | 0x20)
}

fn is_octal_digit(cu: u16) -> bool {
    cu >= b'0' as u16 && cu <= b'7' as u16
}

fn is_binary_digit(cu: u16) -> bool {
    cu == b'0' as u16 || cu == b'1' as u16
}

fn is_utf16_high_surrogate(cu: u16) -> bool {
    (0xD800..=0xDBFF).contains(&cu)
}

fn is_utf16_low_surrogate(cu: u16) -> bool {
    (0xDC00..=0xDFFF).contains(&cu)
}

/// Decode a code point from a UTF-16 slice starting at `pos`.
/// Returns (code_point, number_of_code_units_consumed).
fn decode_code_point(source: &[u16], pos: usize) -> (u32, usize) {
    if pos >= source.len() {
        return (0xFFFD, 1);
    }
    let cu = source[pos];
    if is_utf16_high_surrogate(cu) && pos + 1 < source.len() && is_utf16_low_surrogate(source[pos + 1]) {
        let hi = cu as u32;
        let lo = source[pos + 1] as u32;
        let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
        (cp, 2)
    } else {
        (cu as u32, 1)
    }
}

fn is_line_terminator_cp(cp: u32) -> bool {
    cp == '\n' as u32 || cp == '\r' as u32 || cp == LINE_SEPARATOR as u32 || cp == PARAGRAPH_SEPARATOR as u32
}

fn is_whitespace_cp(cp: u32) -> bool {
    if cp < 128 {
        return is_ascii_space(cp as u16);
    }
    if cp == NO_BREAK_SPACE as u32 || cp == ZERO_WIDTH_NO_BREAK_SPACE as u32 {
        return true;
    }
    // Unicode Space_Separator category
    matches!(
        cp,
        0x1680 | 0x2000..=0x200A | 0x202F | 0x205F | 0x3000
    )
}

fn is_identifier_start_cp(cp: u32) -> bool {
    if is_ascii_alpha(cp) || cp == '_' as u32 || cp == '$' as u32 {
        return true;
    }
    if cp < 128 {
        return false;
    }
    unicode_id_start(cp)
}

fn is_identifier_continue_cp(cp: u32) -> bool {
    if is_ascii_alphanumeric(cp) || cp == '$' as u32 || cp == '_' as u32 || cp == ZERO_WIDTH_NON_JOINER || cp == ZERO_WIDTH_JOINER {
        return true;
    }
    if cp < 128 {
        return false;
    }
    unicode_id_continue(cp)
}

/// Check if a code point has the Unicode ID_Start property.
fn unicode_id_start(cp: u32) -> bool {
    // Use the unicode-ident crate's logic (inlined check)
    unicode_ident::is_xid_start(unsafe { char::from_u32_unchecked(cp) })
}

/// Check if a code point has the Unicode ID_Continue property.
fn unicode_id_continue(cp: u32) -> bool {
    unicode_ident::is_xid_continue(unsafe { char::from_u32_unchecked(cp) })
}

fn keyword_from_str(s: &[u16]) -> Option<TokenType> {
    // Convert to ASCII bytes for matching (all keywords are ASCII)
    if s.iter().any(|&cu| cu > 127) {
        return None;
    }
    let bytes: Vec<u8> = s.iter().map(|&cu| cu as u8).collect();
    match bytes.as_slice() {
        b"async" => Some(TokenType::Async),
        b"await" => Some(TokenType::Await),
        b"break" => Some(TokenType::Break),
        b"case" => Some(TokenType::Case),
        b"catch" => Some(TokenType::Catch),
        b"class" => Some(TokenType::Class),
        b"const" => Some(TokenType::Const),
        b"continue" => Some(TokenType::Continue),
        b"debugger" => Some(TokenType::Debugger),
        b"default" => Some(TokenType::Default),
        b"delete" => Some(TokenType::Delete),
        b"do" => Some(TokenType::Do),
        b"else" => Some(TokenType::Else),
        b"enum" => Some(TokenType::Enum),
        b"export" => Some(TokenType::Export),
        b"extends" => Some(TokenType::Extends),
        b"false" => Some(TokenType::BoolLiteral),
        b"finally" => Some(TokenType::Finally),
        b"for" => Some(TokenType::For),
        b"function" => Some(TokenType::Function),
        b"if" => Some(TokenType::If),
        b"import" => Some(TokenType::Import),
        b"in" => Some(TokenType::In),
        b"instanceof" => Some(TokenType::Instanceof),
        b"let" => Some(TokenType::Let),
        b"new" => Some(TokenType::New),
        b"null" => Some(TokenType::NullLiteral),
        b"return" => Some(TokenType::Return),
        b"super" => Some(TokenType::Super),
        b"switch" => Some(TokenType::Switch),
        b"this" => Some(TokenType::This),
        b"throw" => Some(TokenType::Throw),
        b"true" => Some(TokenType::BoolLiteral),
        b"try" => Some(TokenType::Try),
        b"typeof" => Some(TokenType::Typeof),
        b"var" => Some(TokenType::Var),
        b"void" => Some(TokenType::Void),
        b"while" => Some(TokenType::While),
        b"with" => Some(TokenType::With),
        b"yield" => Some(TokenType::Yield),
        _ => None,
    }
}

fn single_char_token(ch: u16) -> TokenType {
    match ch as u8 {
        b'&' => TokenType::Ampersand,
        b'*' => TokenType::Asterisk,
        b'[' => TokenType::BracketOpen,
        b']' => TokenType::BracketClose,
        b'^' => TokenType::Caret,
        b':' => TokenType::Colon,
        b',' => TokenType::Comma,
        b'{' => TokenType::CurlyOpen,
        b'}' => TokenType::CurlyClose,
        b'=' => TokenType::Equals,
        b'!' => TokenType::ExclamationMark,
        b'-' => TokenType::Minus,
        b'(' => TokenType::ParenOpen,
        b')' => TokenType::ParenClose,
        b'%' => TokenType::Percent,
        b'.' => TokenType::Period,
        b'|' => TokenType::Pipe,
        b'+' => TokenType::Plus,
        b'?' => TokenType::QuestionMark,
        b';' => TokenType::Semicolon,
        b'/' => TokenType::Slash,
        b'~' => TokenType::Tilde,
        b'<' => TokenType::LessThan,
        b'>' => TokenType::GreaterThan,
        _ => TokenType::Invalid,
    }
}

fn parse_two_char_token(ch0: u16, ch1: u16) -> TokenType {
    match (ch0 as u8, ch1 as u8) {
        (b'=', b'>') => TokenType::Arrow,
        (b'=', b'=') => TokenType::EqualsEquals,
        (b'+', b'=') => TokenType::PlusEquals,
        (b'+', b'+') => TokenType::PlusPlus,
        (b'-', b'=') => TokenType::MinusEquals,
        (b'-', b'-') => TokenType::MinusMinus,
        (b'*', b'=') => TokenType::AsteriskEquals,
        (b'*', b'*') => TokenType::DoubleAsterisk,
        (b'/', b'=') => TokenType::SlashEquals,
        (b'%', b'=') => TokenType::PercentEquals,
        (b'&', b'=') => TokenType::AmpersandEquals,
        (b'&', b'&') => TokenType::DoubleAmpersand,
        (b'|', b'=') => TokenType::PipeEquals,
        (b'|', b'|') => TokenType::DoublePipe,
        (b'^', b'=') => TokenType::CaretEquals,
        (b'<', b'=') => TokenType::LessThanEquals,
        (b'<', b'<') => TokenType::ShiftLeft,
        (b'>', b'=') => TokenType::GreaterThanEquals,
        (b'>', b'>') => TokenType::ShiftRight,
        (b'?', b'?') => TokenType::DoubleQuestionMark,
        (b'?', b'.') => TokenType::QuestionMarkPeriod,
        (b'!', b'=') => TokenType::ExclamationMarkEquals,
        _ => TokenType::Invalid,
    }
}

fn parse_three_char_token(ch0: u16, ch1: u16, ch2: u16) -> TokenType {
    match (ch0 as u8, ch1 as u8, ch2 as u8) {
        (b'<', b'<', b'=') => TokenType::ShiftLeftEquals,
        (b'>', b'>', b'=') => TokenType::ShiftRightEquals,
        (b'>', b'>', b'>') => TokenType::UnsignedShiftRight,
        (b'=', b'=', b'=') => TokenType::EqualsEqualsEquals,
        (b'!', b'=', b'=') => TokenType::ExclamationMarkEqualsEquals,
        (b'.', b'.', b'.') => TokenType::TripleDot,
        (b'*', b'*', b'=') => TokenType::DoubleAsteriskEquals,
        (b'&', b'&', b'=') => TokenType::DoubleAmpersandEquals,
        (b'|', b'|', b'=') => TokenType::DoublePipeEquals,
        (b'?', b'?', b'=') => TokenType::DoubleQuestionMarkEquals,
        _ => TokenType::Invalid,
    }
}

impl<'a> Lexer<'a> {
    pub fn new(source: &'a [u16], line_number: u32, line_column: u32) -> Self {
        let mut lexer = Lexer {
            source,
            position: 0,
            current_code_unit: 0,
            eof: false,
            line_number,
            line_column,
            current_token_type: TokenType::Eof,
            regex_is_in_character_class: false,
            allow_html_comments: true,
            template_states: Vec::new(),
        };
        lexer.consume();
        lexer
    }

    pub fn disallow_html_comments(&mut self) {
        self.allow_html_comments = false;
    }

    fn source_len(&self) -> usize {
        self.source.len()
    }

    fn consume(&mut self) {
        if self.position > self.source_len() {
            return;
        }

        if self.position >= self.source_len() {
            self.eof = true;
            self.current_code_unit = 0;
            self.position = self.source_len() + 1;
            self.line_column += 1;
            return;
        }

        if self.is_line_terminator() {
            let second_char_of_crlf = self.position > 1
                && self.source[self.position - 2] == b'\r' as u16
                && self.current_code_unit == b'\n' as u16;

            if !second_char_of_crlf {
                self.line_number += 1;
                self.line_column = 1;
            }
        } else {
            if is_utf16_high_surrogate(self.current_code_unit)
                && self.position < self.source_len()
                && is_utf16_low_surrogate(self.source[self.position])
            {
                self.position += 1;
                if self.position >= self.source_len() {
                    self.eof = true;
                    self.current_code_unit = 0;
                    self.position = self.source_len() + 1;
                    self.line_column += 1;
                    return;
                }
            }
            self.line_column += 1;
        }

        self.current_code_unit = self.source[self.position];
        self.position += 1;
    }

    fn current_code_point(&self) -> u32 {
        if self.position == 0 {
            return 0xFFFD;
        }
        let (cp, _) = decode_code_point(self.source, self.position - 1);
        cp
    }

    fn is_eof(&self) -> bool {
        self.eof
    }

    fn is_line_terminator(&self) -> bool {
        let cu = self.current_code_unit;
        if cu == b'\n' as u16 || cu == b'\r' as u16 {
            return true;
        }
        if is_ascii(cu) {
            return false;
        }
        is_line_terminator_cp(self.current_code_point())
    }

    fn is_whitespace(&self) -> bool {
        if is_ascii_space(self.current_code_unit) {
            return true;
        }
        if is_ascii(self.current_code_unit) {
            return false;
        }
        is_whitespace_cp(self.current_code_point())
    }

    /// Try to parse a unicode escape sequence at the current position.
    /// Returns (code_point, number_of_code_units_consumed) if successful.
    fn is_identifier_unicode_escape(&self) -> Option<(u32, usize)> {
        // Current code unit should be '\', and we look ahead from position - 1
        let start = self.position - 1;
        if start >= self.source_len() {
            return None;
        }
        // The current code unit is already consumed (it's '\'), so we look at source[start..]
        // which starts after the backslash. Actually, position-1 points to the current_code_unit.
        // So source[position-1] == '\\'. We need 'u' at source[position].
        let pos = self.position;
        if pos >= self.source_len() {
            return None;
        }
        if self.source[pos] != b'u' as u16 {
            return None;
        }
        let pos = pos + 1;
        if pos >= self.source_len() {
            return None;
        }

        if self.source[pos] == b'{' as u16 {
            // u{ CodePoint }
            let mut cp: u32 = 0;
            let mut i = pos + 1;
            if i >= self.source_len() {
                return None;
            }
            while i < self.source_len() && self.source[i] != b'}' as u16 {
                let cu = self.source[i];
                if !is_ascii_hex_digit(cu) {
                    return None;
                }
                cp = cp * 16 + hex_value(cu);
                if cp > 0x10FFFF {
                    return None;
                }
                i += 1;
            }
            if i >= self.source_len() || self.source[i] != b'}' as u16 {
                return None;
            }
            let consumed = i + 1 - (self.position - 1);
            Some((cp, consumed))
        } else {
            // u Hex4Digits
            if pos + 4 > self.source_len() {
                return None;
            }
            let mut cp: u32 = 0;
            for i in 0..4 {
                let cu = self.source[pos + i];
                if !is_ascii_hex_digit(cu) {
                    return None;
                }
                cp = cp * 16 + hex_value(cu);
            }
            let consumed = pos + 4 - (self.position - 1);
            Some((cp, consumed))
        }
    }

    /// Check if the current position starts an identifier start character.
    /// Returns (code_point, number_of_code_units_to_consume).
    fn is_identifier_start(&self) -> Option<(u32, usize)> {
        let cp = self.current_code_point();
        if cp == '\\' as u32 {
            if let Some((escaped_cp, len)) = self.is_identifier_unicode_escape() {
                if is_identifier_start_cp(escaped_cp) {
                    return Some((escaped_cp, len));
                }
            }
            return None;
        }

        if is_identifier_start_cp(cp) {
            // Count how many code units this code point takes
            let len = if cp > 0xFFFF { 2 } else { 1 };
            return Some((cp, len));
        }

        None
    }

    /// Check if the current position continues an identifier.
    /// Returns (code_point, number_of_code_units_to_consume).
    fn is_identifier_middle(&self) -> Option<(u32, usize)> {
        let cp = self.current_code_point();
        if cp == '\\' as u32 {
            if let Some((escaped_cp, len)) = self.is_identifier_unicode_escape() {
                if is_identifier_continue_cp(escaped_cp) {
                    return Some((escaped_cp, len));
                }
            }
            return None;
        }

        if is_identifier_continue_cp(cp) {
            let len = if cp > 0xFFFF { 2 } else { 1 };
            return Some((cp, len));
        }

        None
    }

    fn match2(&self, a: u16, b: u16) -> bool {
        if self.position >= self.source_len() {
            return false;
        }
        self.current_code_unit == a && self.source[self.position] == b
    }

    fn match3(&self, a: u16, b: u16, c: u16) -> bool {
        if self.position + 1 >= self.source_len() {
            return false;
        }
        self.current_code_unit == a && self.source[self.position] == b && self.source[self.position + 1] == c
    }

    fn match4(&self, a: u16, b: u16, c: u16, d: u16) -> bool {
        if self.position + 2 >= self.source_len() {
            return false;
        }
        self.current_code_unit == a
            && self.source[self.position] == b
            && self.source[self.position + 1] == c
            && self.source[self.position + 2] == d
    }

    fn match_numeric_literal_separator_followed_by(&self, check: fn(u16) -> bool) -> bool {
        if self.position >= self.source_len() {
            return false;
        }
        self.current_code_unit == b'_' as u16 && check(self.source[self.position])
    }

    fn is_line_comment_start(&self, line_has_token_yet: bool) -> bool {
        self.match2(b'/' as u16, b'/' as u16)
            || (self.allow_html_comments && self.match4(b'<' as u16, b'!' as u16, b'-' as u16, b'-' as u16))
            || (self.allow_html_comments && !line_has_token_yet && self.match3(b'-' as u16, b'-' as u16, b'>' as u16))
            || (self.match2(b'#' as u16, b'!' as u16) && self.position == 1)
    }

    fn is_block_comment_start(&self) -> bool {
        self.match2(b'/' as u16, b'*' as u16)
    }

    fn is_block_comment_end(&self) -> bool {
        self.match2(b'*' as u16, b'/' as u16)
    }

    fn is_numeric_literal_start(&self) -> bool {
        is_ascii_digit(self.current_code_unit)
            || (self.current_code_unit == b'.' as u16
                && self.position < self.source_len()
                && is_ascii_digit(self.source[self.position]))
    }

    fn slash_means_division(&self) -> bool {
        let tt = self.current_token_type;
        tt.is_identifier_name()
            || tt == TokenType::BigIntLiteral
            || tt == TokenType::BracketClose
            || tt == TokenType::CurlyClose
            || tt == TokenType::MinusMinus
            || tt == TokenType::NumericLiteral
            || tt == TokenType::ParenClose
            || tt == TokenType::PlusPlus
            || tt == TokenType::PrivateIdentifier
            || tt == TokenType::RegexLiteral
            || tt == TokenType::StringLiteral
            || tt == TokenType::TemplateLiteralEnd
    }

    fn consume_decimal_number(&mut self) -> bool {
        if !is_ascii_digit(self.current_code_unit) {
            return false;
        }
        while is_ascii_digit(self.current_code_unit) || self.match_numeric_literal_separator_followed_by(is_ascii_digit) {
            self.consume();
        }
        true
    }

    fn consume_exponent(&mut self) -> bool {
        self.consume();
        if self.current_code_unit == b'-' as u16 || self.current_code_unit == b'+' as u16 {
            self.consume();
        }
        if !is_ascii_digit(self.current_code_unit) {
            return false;
        }
        self.consume_decimal_number()
    }

    fn consume_octal_number(&mut self) -> bool {
        self.consume();
        if !is_octal_digit(self.current_code_unit) {
            return false;
        }
        while is_octal_digit(self.current_code_unit) || self.match_numeric_literal_separator_followed_by(is_octal_digit) {
            self.consume();
        }
        true
    }

    fn consume_hexadecimal_number(&mut self) -> bool {
        self.consume();
        if !is_ascii_hex_digit(self.current_code_unit) {
            return false;
        }
        while is_ascii_hex_digit(self.current_code_unit) || self.match_numeric_literal_separator_followed_by(is_ascii_hex_digit) {
            self.consume();
        }
        true
    }

    fn consume_binary_number(&mut self) -> bool {
        self.consume();
        if !is_binary_digit(self.current_code_unit) {
            return false;
        }
        while is_binary_digit(self.current_code_unit) || self.match_numeric_literal_separator_followed_by(is_binary_digit) {
            self.consume();
        }
        true
    }

    fn consume_regex_literal(&mut self) -> TokenType {
        while !self.is_eof() {
            if self.is_line_terminator() || (!self.regex_is_in_character_class && self.current_code_unit == b'/' as u16) {
                break;
            }

            if self.current_code_unit == b'[' as u16 {
                self.regex_is_in_character_class = true;
            } else if self.current_code_unit == b']' as u16 {
                self.regex_is_in_character_class = false;
            } else if !self.regex_is_in_character_class && self.current_code_unit == b'/' as u16 {
                break;
            }

            if self.match2(b'\\' as u16, b'/' as u16)
                || self.match2(b'\\' as u16, b'[' as u16)
                || self.match2(b'\\' as u16, b'\\' as u16)
                || (self.regex_is_in_character_class && self.match2(b'\\' as u16, b']' as u16))
            {
                self.consume();
            }
            self.consume();
        }

        if self.current_code_unit == b'/' as u16 {
            self.consume();
            TokenType::RegexLiteral
        } else {
            TokenType::UnterminatedRegexLiteral
        }
    }

    /// Advance to the next token. Returns the token.
    pub fn next(&mut self) -> Token {
        let trivia_start = self.position;
        let in_template = !self.template_states.is_empty();
        let mut line_has_token_yet = self.line_column > 1;
        let mut unterminated_comment = false;

        if !in_template || self.template_states.last().unwrap().in_expr {
            // Consume whitespace and comments
            loop {
                if self.is_line_terminator() {
                    line_has_token_yet = false;
                    loop {
                        self.consume();
                        if !self.is_line_terminator() {
                            break;
                        }
                    }
                } else if self.is_whitespace() {
                    loop {
                        self.consume();
                        if !self.is_whitespace() {
                            break;
                        }
                    }
                } else if self.is_line_comment_start(line_has_token_yet) {
                    self.consume();
                    loop {
                        self.consume();
                        if self.is_eof() || self.is_line_terminator() {
                            break;
                        }
                    }
                } else if self.is_block_comment_start() {
                    let start_line_number = self.line_number;
                    self.consume();
                    loop {
                        self.consume();
                        if self.is_eof() || self.is_block_comment_end() {
                            break;
                        }
                    }
                    if self.is_eof() {
                        unterminated_comment = true;
                    }
                    self.consume(); // consume *
                    if self.is_eof() {
                        unterminated_comment = true;
                    }
                    self.consume(); // consume /

                    if start_line_number != self.line_number {
                        line_has_token_yet = false;
                    }
                } else {
                    break;
                }
            }
        }

        let value_start = self.position;
        let value_start_line_number = self.line_number;
        let value_start_column_number = self.line_column;
        let mut token_type = TokenType::Invalid;
        let did_consume_whitespace_or_comments = trivia_start != value_start;

        let mut _identifier_value: Option<Vec<u16>> = None;

        if self.current_token_type == TokenType::RegexLiteral
            && !self.is_eof()
            && (self.current_code_unit < 128 && (self.current_code_unit as u8 as char).is_ascii_alphabetic())
            && !did_consume_whitespace_or_comments
        {
            token_type = TokenType::RegexFlags;
            while !self.is_eof() && self.current_code_unit < 128 && (self.current_code_unit as u8 as char).is_ascii_alphabetic() {
                self.consume();
            }
        } else if self.current_code_unit == b'`' as u16 {
            self.consume();
            if !in_template {
                token_type = TokenType::TemplateLiteralStart;
                self.template_states.push(TemplateState {
                    in_expr: false,
                    open_bracket_count: 0,
                });
            } else if self.template_states.last().unwrap().in_expr {
                self.template_states.push(TemplateState {
                    in_expr: false,
                    open_bracket_count: 0,
                });
                token_type = TokenType::TemplateLiteralStart;
            } else {
                self.template_states.pop();
                token_type = TokenType::TemplateLiteralEnd;
            }
        } else if in_template
            && self.template_states.last().unwrap().in_expr
            && self.template_states.last().unwrap().open_bracket_count == 0
            && self.current_code_unit == b'}' as u16
        {
            self.consume();
            token_type = TokenType::TemplateLiteralExprEnd;
            self.template_states.last_mut().unwrap().in_expr = false;
        } else if in_template && !self.template_states.last().unwrap().in_expr {
            if self.is_eof() {
                token_type = TokenType::UnterminatedTemplateLiteral;
                self.template_states.pop();
            } else if self.match2(b'$' as u16, b'{' as u16) {
                token_type = TokenType::TemplateLiteralExprStart;
                self.consume();
                self.consume();
                self.template_states.last_mut().unwrap().in_expr = true;
            } else {
                while !self.match2(b'$' as u16, b'{' as u16) && self.current_code_unit != b'`' as u16 && !self.is_eof() {
                    if self.match2(b'\\' as u16, b'$' as u16)
                        || self.match2(b'\\' as u16, b'`' as u16)
                        || self.match2(b'\\' as u16, b'\\' as u16)
                    {
                        self.consume();
                    }
                    self.consume();
                }
                if self.is_eof() && !self.template_states.is_empty() {
                    token_type = TokenType::UnterminatedTemplateLiteral;
                } else {
                    token_type = TokenType::TemplateLiteralString;
                }
            }
        } else if self.current_code_unit == b'#' as u16 {
            self.consume();
            if let Some((cp, len)) = self.is_identifier_start() {
                let mut builder: Vec<u16> = Vec::new();
                builder.push(b'#' as u16);
                let mut code_point = cp;
                let mut ident_len = len;
                loop {
                    encode_utf16(code_point, &mut builder);
                    for _ in 0..ident_len {
                        self.consume();
                    }
                    if let Some((next_cp, next_len)) = self.is_identifier_middle() {
                        code_point = next_cp;
                        ident_len = next_len;
                    } else {
                        break;
                    }
                }
                _identifier_value = Some(builder);
                token_type = TokenType::PrivateIdentifier;
            } else {
                token_type = TokenType::Invalid;
                // token_message = StartOfPrivateNameNotFollowedByValidIdentifier
            }
        } else if let Some((cp, len)) = self.is_identifier_start() {
            let mut has_escaped_character = false;
            let mut builder: Vec<u16> = Vec::new();
            let mut code_point = cp;
            let mut ident_len = len;
            loop {
                encode_utf16(code_point, &mut builder);
                for _ in 0..ident_len {
                    self.consume();
                }
                has_escaped_character |= ident_len > 1 && code_point != 0 && code_point <= 0x10FFFF && ident_len != (if code_point > 0xFFFF { 2 } else { 1 });
                if let Some((next_cp, next_len)) = self.is_identifier_middle() {
                    code_point = next_cp;
                    ident_len = next_len;
                } else {
                    break;
                }
            }

            if let Some(kw) = keyword_from_str(&builder) {
                if has_escaped_character {
                    token_type = TokenType::EscapedKeyword;
                } else {
                    token_type = kw;
                }
            } else {
                token_type = TokenType::Identifier;
            }
            _identifier_value = Some(builder);
        } else if self.is_numeric_literal_start() {
            token_type = TokenType::NumericLiteral;
            let mut is_invalid = false;
            if self.current_code_unit == b'0' as u16 {
                self.consume();
                if self.current_code_unit == b'.' as u16 {
                    self.consume();
                    while is_ascii_digit(self.current_code_unit) {
                        self.consume();
                    }
                    if self.current_code_unit == b'e' as u16 || self.current_code_unit == b'E' as u16 {
                        is_invalid = !self.consume_exponent();
                    }
                } else if self.current_code_unit == b'e' as u16 || self.current_code_unit == b'E' as u16 {
                    is_invalid = !self.consume_exponent();
                } else if self.current_code_unit == b'o' as u16 || self.current_code_unit == b'O' as u16 {
                    is_invalid = !self.consume_octal_number();
                    if self.current_code_unit == b'n' as u16 {
                        self.consume();
                        token_type = TokenType::BigIntLiteral;
                    }
                } else if self.current_code_unit == b'b' as u16 || self.current_code_unit == b'B' as u16 {
                    is_invalid = !self.consume_binary_number();
                    if self.current_code_unit == b'n' as u16 {
                        self.consume();
                        token_type = TokenType::BigIntLiteral;
                    }
                } else if self.current_code_unit == b'x' as u16 || self.current_code_unit == b'X' as u16 {
                    is_invalid = !self.consume_hexadecimal_number();
                    if self.current_code_unit == b'n' as u16 {
                        self.consume();
                        token_type = TokenType::BigIntLiteral;
                    }
                } else if self.current_code_unit == b'n' as u16 {
                    self.consume();
                    token_type = TokenType::BigIntLiteral;
                } else if is_ascii_digit(self.current_code_unit) {
                    // Legacy octal without 0o prefix
                    loop {
                        self.consume();
                        if !is_ascii_digit(self.current_code_unit) {
                            break;
                        }
                    }
                }
            } else {
                // 1..9 or period
                while is_ascii_digit(self.current_code_unit) || self.match_numeric_literal_separator_followed_by(is_ascii_digit) {
                    self.consume();
                }
                if self.current_code_unit == b'n' as u16 {
                    self.consume();
                    token_type = TokenType::BigIntLiteral;
                } else {
                    if self.current_code_unit == b'.' as u16 {
                        self.consume();
                        if self.current_code_unit == b'_' as u16 {
                            is_invalid = true;
                        }
                        while is_ascii_digit(self.current_code_unit) || self.match_numeric_literal_separator_followed_by(is_ascii_digit) {
                            self.consume();
                        }
                    }
                    if self.current_code_unit == b'e' as u16 || self.current_code_unit == b'E' as u16 {
                        is_invalid = is_invalid || !self.consume_exponent();
                    }
                }
            }
            if is_invalid {
                token_type = TokenType::Invalid;
                // token_message = InvalidNumericLiteral
            }
        } else if self.current_code_unit == b'"' as u16 || self.current_code_unit == b'\'' as u16 {
            let stop_char = self.current_code_unit;
            self.consume();
            while self.current_code_unit != stop_char
                && self.current_code_unit != b'\r' as u16
                && self.current_code_unit != b'\n' as u16
                && !self.is_eof()
            {
                if self.current_code_unit == b'\\' as u16 {
                    self.consume();
                    if self.current_code_unit == b'\r' as u16
                        && self.position < self.source_len()
                        && self.source[self.position] == b'\n' as u16
                    {
                        self.consume();
                    }
                }
                self.consume();
            }
            if self.current_code_unit != stop_char {
                token_type = TokenType::UnterminatedStringLiteral;
            } else {
                self.consume();
                token_type = TokenType::StringLiteral;
            }
        } else if self.current_code_unit == b'/' as u16 && !self.slash_means_division() {
            self.consume();
            token_type = self.consume_regex_literal();
        } else if self.eof {
            if unterminated_comment {
                token_type = TokenType::Invalid;
                // token_message = UnterminatedMultiLineComment
            } else {
                token_type = TokenType::Eof;
            }
        } else {
            let mut found_token = false;

            // Four-char operator: >>>=
            if self.match4(b'>' as u16, b'>' as u16, b'>' as u16, b'=' as u16) {
                found_token = true;
                token_type = TokenType::UnsignedShiftRightEquals;
                self.consume();
                self.consume();
                self.consume();
                self.consume();
            }

            if !found_token && self.position + 1 < self.source_len() {
                let ch0 = self.current_code_unit;
                let ch1 = self.source[self.position];
                let ch2 = self.source[self.position + 1];
                let tt = parse_three_char_token(ch0, ch1, ch2);
                if tt != TokenType::Invalid {
                    found_token = true;
                    token_type = tt;
                    self.consume();
                    self.consume();
                    self.consume();
                }
            }

            if !found_token && self.position < self.source_len() {
                let ch0 = self.current_code_unit;
                let ch1 = self.source[self.position];
                let tt = parse_two_char_token(ch0, ch1);
                if tt != TokenType::Invalid {
                    // OptionalChainingPunctuator :: ?. [lookahead not in DecimalDigit]
                    if !(tt == TokenType::QuestionMarkPeriod
                        && self.position + 1 < self.source_len()
                        && is_ascii_digit(self.source[self.position + 1]))
                    {
                        found_token = true;
                        token_type = tt;
                        self.consume();
                        self.consume();
                    }
                }
            }

            if !found_token && is_ascii(self.current_code_unit) {
                let tt = single_char_token(self.current_code_unit);
                if tt != TokenType::Invalid {
                    found_token = true;
                    token_type = tt;
                    self.consume();
                }
            }

            if !found_token {
                token_type = TokenType::Invalid;
                self.consume();
            }
        }

        // Track template literal curly braces
        if !self.template_states.is_empty() && self.template_states.last().unwrap().in_expr {
            if token_type == TokenType::CurlyOpen {
                self.template_states.last_mut().unwrap().open_bracket_count += 1;
            } else if token_type == TokenType::CurlyClose {
                self.template_states.last_mut().unwrap().open_bracket_count -= 1;
            }
        }

        self.current_token_type = token_type;

        let trivia_has_line_terminator = if trivia_start > 0 && value_start > trivia_start {
            self.source[trivia_start - 1..value_start - 1]
                .iter()
                .any(|&cu| cu == b'\n' as u16 || cu == b'\r' as u16 || cu == LINE_SEPARATOR || cu == PARAGRAPH_SEPARATOR)
        } else {
            false
        };

        Token {
            token_type,
            trivia_start: trivia_start.saturating_sub(1) as u32,
            trivia_len: (value_start - trivia_start) as u32,
            value_start: value_start.saturating_sub(1) as u32,
            value_len: (self.position - value_start) as u32,
            line_number: value_start_line_number,
            line_column: value_start_column_number,
            offset: value_start.saturating_sub(1) as u32,
            trivia_has_line_terminator,
        }
    }

    /// Re-lex the current Slash or SlashEquals token as a regex literal.
    pub fn force_slash_as_regex(&mut self) -> Token {
        let has_equals = self.current_token_type == TokenType::SlashEquals;

        debug_assert!(self.position > 0);
        let mut value_start = self.position - 1;

        if has_equals {
            value_start -= 1;
            self.position -= 1;
            self.current_code_unit = b'=' as u16;
        }

        let token_type = self.consume_regex_literal();
        self.current_token_type = token_type;

        Token {
            token_type,
            trivia_start: 0,
            trivia_len: 0,
            value_start: value_start.saturating_sub(1) as u32,
            value_len: (self.position - value_start) as u32,
            line_number: 0,
            line_column: 0,
            offset: value_start.saturating_sub(1) as u32,
            trivia_has_line_terminator: false,
        }
    }
}

fn hex_value(cu: u16) -> u32 {
    match cu {
        0x30..=0x39 => (cu - 0x30) as u32,
        0x41..=0x46 => (cu - 0x41 + 10) as u32,
        0x61..=0x66 => (cu - 0x61 + 10) as u32,
        _ => 0,
    }
}

fn encode_utf16(cp: u32, buf: &mut Vec<u16>) {
    if cp <= 0xFFFF {
        buf.push(cp as u16);
    } else {
        let cp = cp - 0x10000;
        buf.push(0xD800 + (cp >> 10) as u16);
        buf.push(0xDC00 + (cp & 0x3FF) as u16);
    }
}
