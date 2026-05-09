/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(super) fn parser_from_filtered_input(filtered_input: &[u8]) -> (Parser, &str) {
    let mut tokens = Vec::new();
    let filtered_input_string = std::str::from_utf8(filtered_input)
        .expect("rust_css_parse_component_values received non-UTF-8 input after C++ decoding");
    crate::css_tokenizer::tokenize(filtered_input, |token, _| {
        tokens.push(token.clone());
    });

    (Parser::new(tokens), filtered_input_string)
}

pub(super) fn token_is_delim(token: &Token, value: char) -> bool {
    matches!(token.token_type, TokenType::Delim { value: delimiter } if delimiter == value as u32)
}

pub(super) fn token_original_source<'a>(token: &Token, filtered_input: &'a str) -> Option<&'a str> {
    token.original_source(filtered_input)
}

impl Token {
    pub(super) fn is_unicode_range_ending_token(&self) -> bool {
        matches!(
            self.token_type,
            TokenType::EndOfFile | TokenType::Comma | TokenType::Semicolon | TokenType::Whitespace
        )
    }
}

// https://www.w3.org/TR/css-syntax-3/#urange-syntax
pub(super) fn parse_unicode_range_text(text: &str) -> Option<CssUnicodeRange> {
    fn make_valid_unicode_range(start_value: u32, end_value: u32) -> Option<CssUnicodeRange> {
        // https://www.w3.org/TR/css-syntax-3/#maximum-allowed-code-point
        const MAXIMUM_ALLOWED_CODE_POINT: u32 = 0x10FFFF;

        // To determine what codepoints the <urange> represents:
        // 1. If end value is greater than the maximum allowed code point,
        //    the <urange> is invalid and a syntax error.
        if end_value > MAXIMUM_ALLOWED_CODE_POINT {
            return None;
        }

        // 2. If start value is greater than end value, the <urange> is invalid and a syntax error.
        if start_value > end_value {
            return None;
        }

        // 3. Otherwise, the <urange> represents a contiguous range of codepoints from start value to end value, inclusive.
        Some(CssUnicodeRange {
            min_code_point: start_value,
            max_code_point: end_value,
        })
    }

    // 1. Skipping the first u token, concatenate the representations of all the tokens in the production together.
    //    Let this be text.
    // NOTE: The concatenation is already done by the caller.
    let mut input = text;

    // 2. If the first character of text is U+002B PLUS SIGN, consume it.
    //    Otherwise, this is an invalid <urange>, and this algorithm must exit.
    input = input.strip_prefix('+')?;

    // 3. Consume as many hex digits from text as possible.
    //    then consume as many U+003F QUESTION MARK (?) code points as possible.
    let hex_digits_len = input.bytes().take_while(|byte| byte.is_ascii_hexdigit()).count();
    let after_hex_digits = &input[hex_digits_len..];
    let question_marks_len = after_hex_digits.bytes().take_while(|byte| *byte == b'?').count();
    let consumed_code_points = hex_digits_len + question_marks_len;

    //    If zero code points were consumed, or more than six code points were consumed,
    //    this is an invalid <urange>, and this algorithm must exit.
    if consumed_code_points == 0 || consumed_code_points > 6 {
        return None;
    }

    let start_value_code_points = &input[..consumed_code_points];
    input = &input[consumed_code_points..];

    //    If any U+003F QUESTION MARK (?) code points were consumed, then:
    if question_marks_len > 0 {
        // 1. If there are any code points left in text, this is an invalid <urange>,
        //    and this algorithm must exit.
        if !input.is_empty() {
            return None;
        }

        // 2. Interpret the consumed code points as a hexadecimal number,
        //    with the U+003F QUESTION MARK (?) code points replaced by U+0030 DIGIT ZERO (0) code points.
        //    This is the start value.
        let start_value_string = start_value_code_points.replace('?', "0");
        let start_value = u32::from_str_radix(&start_value_string, 16).ok()?;

        // 3. Interpret the consumed code points as a hexadecimal number again,
        //    with the U+003F QUESTION MARK (?) code points replaced by U+0046 LATIN CAPITAL LETTER F (F) code points.
        //    This is the end value.
        let end_value_string = start_value_code_points.replace('?', "F");
        let end_value = u32::from_str_radix(&end_value_string, 16).ok()?;

        // 4. Exit this algorithm.
        return make_valid_unicode_range(start_value, end_value);
    }

    //   Otherwise, interpret the consumed code points as a hexadecimal number. This is the start value.
    let start_value = u32::from_str_radix(start_value_code_points, 16).ok()?;

    // 4. If there are no code points left in text, The end value is the same as the start value.
    //    Exit this algorithm.
    if input.is_empty() {
        return make_valid_unicode_range(start_value, start_value);
    }

    // 5. If the next code point in text is U+002D HYPHEN-MINUS (-), consume it.
    //    Otherwise, this is an invalid <urange>, and this algorithm must exit.
    input = input.strip_prefix('-')?;

    // 6. Consume as many hex digits as possible from text.
    let end_hex_digits_len = input.bytes().take_while(|byte| byte.is_ascii_hexdigit()).count();

    //   If zero hex digits were consumed, or more than 6 hex digits were consumed,
    //   this is an invalid <urange>, and this algorithm must exit.
    if end_hex_digits_len == 0 || end_hex_digits_len > 6 {
        return None;
    }

    let end_hex_digits = &input[..end_hex_digits_len];
    input = &input[end_hex_digits_len..];

    //   If there are any code points left in text, this is an invalid <urange>, and this algorithm must exit.
    if !input.is_empty() {
        return None;
    }

    // 7. Interpret the consumed code points as a hexadecimal number. This is the end value.
    let end_value = u32::from_str_radix(end_hex_digits, 16).ok()?;

    make_valid_unicode_range(start_value, end_value)
}

pub(super) fn string_parts(string: &str) -> (*const u8, usize) {
    (string.as_ptr(), string.len())
}

impl From<CssRuleContext> for RuleContext {
    fn from(value: CssRuleContext) -> Self {
        match value {
            CssRuleContext::Unknown => Self::Unknown,
            CssRuleContext::Style => Self::Style,
            CssRuleContext::AtContainer => Self::AtContainer,
            CssRuleContext::AtCounterStyle => Self::AtCounterStyle,
            CssRuleContext::AtMedia => Self::AtMedia,
            CssRuleContext::AtFontFace => Self::AtFontFace,
            CssRuleContext::AtFontFeatureValues => Self::AtFontFeatureValues,
            CssRuleContext::FontFeatureValue => Self::FontFeatureValue,
            CssRuleContext::AtFunction => Self::AtFunction,
            CssRuleContext::AtKeyframes => Self::AtKeyframes,
            CssRuleContext::Keyframe => Self::Keyframe,
            CssRuleContext::AtSupports => Self::AtSupports,
            CssRuleContext::SupportsCondition => Self::SupportsCondition,
            CssRuleContext::AtLayer => Self::AtLayer,
            CssRuleContext::AtProperty => Self::AtProperty,
            CssRuleContext::AtPage => Self::AtPage,
            CssRuleContext::Margin => Self::Margin,
        }
    }
}

impl CssRuleEvent {
    pub(super) fn new(kind: CssRuleEventKind) -> Self {
        Self {
            kind,
            name_ptr: std::ptr::null(),
            name_len: 0,
            important: false,
            is_block_rule: false,
        }
    }
}

impl CssSyntaxNode {
    pub(super) fn new(kind: CssSyntaxNodeKind) -> Self {
        Self {
            kind,
            value_ptr: std::ptr::null(),
            value_len: 0,
        }
    }
}

pub(crate) fn strip_whitespace(component_values: &[ComponentValue]) -> &[ComponentValue] {
    let mut start = 0;
    let mut end = component_values.len();
    while start < end && is_whitespace_component_value(&component_values[start]) {
        start += 1;
    }
    while start < end && is_whitespace_component_value(&component_values[end - 1]) {
        end -= 1;
    }
    &component_values[start..end]
}

pub(crate) fn serialize_component_values_for_reparsing(
    component_values: &[ComponentValue],
    filtered_input: &str,
) -> Option<String> {
    let mut output = String::new();
    for component_value in component_values {
        serialize_component_value_for_reparsing(component_value, filtered_input, &mut output)?;
    }
    Some(output)
}

pub(super) fn serialize_component_values_for_reparsing_separated_by_spaces(
    component_values: &[ComponentValue],
    filtered_input: &str,
) -> Option<String> {
    let mut output = String::new();
    for (index, component_value) in component_values.iter().enumerate() {
        if index > 0 {
            output.push(' ');
        }
        serialize_component_value_for_reparsing(component_value, filtered_input, &mut output)?;
    }
    Some(output)
}

pub(super) fn serialize_component_value_for_reparsing(
    component_value: &ComponentValue,
    filtered_input: &str,
    output: &mut String,
) -> Option<()> {
    match component_value {
        ComponentValue::PreservedToken(token) => output.push_str(token.original_source(filtered_input)?),
        ComponentValue::Function(function) => {
            output.push_str(function.name_token.original_source(filtered_input)?);
            for component_value in &function.value {
                serialize_component_value_for_reparsing(component_value, filtered_input, output)?;
            }
            output.push_str(function.end_token.original_source(filtered_input)?);
        }
        ComponentValue::SimpleBlock(block) => {
            output.push_str(block.token.original_source(filtered_input)?);
            for component_value in &block.value {
                serialize_component_value_for_reparsing(component_value, filtered_input, output)?;
            }
            output.push_str(block.end_token.original_source(filtered_input)?);
        }
    }
    Some(())
}

pub(super) fn rule_context_type_for_at_rule(name: &str) -> RuleContext {
    if name.eq_ignore_ascii_case("media") {
        return RuleContext::AtMedia;
    }
    if name.eq_ignore_ascii_case("container") {
        return RuleContext::AtContainer;
    }
    if name.eq_ignore_ascii_case("counter-style") {
        return RuleContext::AtCounterStyle;
    }
    if name.eq_ignore_ascii_case("font-face") {
        return RuleContext::AtFontFace;
    }
    if name.eq_ignore_ascii_case("keyframes") || name.eq_ignore_ascii_case("-webkit-keyframes") {
        return RuleContext::AtKeyframes;
    }
    if name.eq_ignore_ascii_case("font-feature-values") {
        return RuleContext::AtFontFeatureValues;
    }
    if name.eq_ignore_ascii_case("function") {
        return RuleContext::AtFunction;
    }
    if is_font_feature_value_type_at_keyword(name) {
        return RuleContext::FontFeatureValue;
    }
    if name.eq_ignore_ascii_case("supports") {
        return RuleContext::AtSupports;
    }
    if name.eq_ignore_ascii_case("layer") {
        return RuleContext::AtLayer;
    }
    if name.eq_ignore_ascii_case("property") {
        return RuleContext::AtProperty;
    }
    if name.eq_ignore_ascii_case("page") {
        return RuleContext::AtPage;
    }
    if is_margin_rule_name(name) {
        return RuleContext::Margin;
    }
    RuleContext::Unknown
}

pub(super) fn first_is_one_of(name: &str, values: &[&str]) -> bool {
    values.iter().any(|value| name.eq_ignore_ascii_case(value))
}

pub(super) fn is_margin_rule_name(name: &str) -> bool {
    first_is_one_of(
        name,
        &[
            "top-left-corner",
            "top-left",
            "top-center",
            "top-right",
            "top-right-corner",
            "bottom-left-corner",
            "bottom-left",
            "bottom-center",
            "bottom-right",
            "bottom-right-corner",
            "left-top",
            "left-middle",
            "left-bottom",
            "right-top",
            "right-middle",
            "right-bottom",
        ],
    )
}

pub(super) fn is_font_feature_value_type_at_keyword(name: &str) -> bool {
    first_is_one_of(
        name,
        &[
            "stylistic",
            "historical-forms",
            "styleset",
            "character-variant",
            "swash",
            "ornaments",
            "annotation",
        ],
    )
}

pub(super) fn is_animation_property_disallowed_in_keyframe(name: &str) -> bool {
    first_is_one_of(
        name,
        &[
            "animation",
            "animation-delay",
            "animation-direction",
            "animation-duration",
            "animation-fill-mode",
            "animation-iteration-count",
            "animation-name",
            "animation-play-state",
            "animation-timeline",
            "-webkit-animation-delay",
            "-webkit-animation-direction",
            "-webkit-animation-duration",
            "-webkit-animation-fill-mode",
            "-webkit-animation-iteration-count",
            "-webkit-animation-name",
            "-webkit-animation-play-state",
        ],
    )
}
