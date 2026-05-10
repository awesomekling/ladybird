/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(super) fn component_value_is_ident(component_value: Option<&ComponentValue>, expected: &str) -> bool {
    matches!(
        component_value,
        Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) if value.eq_ignore_ascii_case(expected)
    )
}

pub(super) fn component_value_is_delim(component_value: Option<&ComponentValue>, expected: char) -> bool {
    matches!(
        component_value,
        Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Delim { value },
            ..
        })) if *value == expected as u32
    )
}

pub(super) fn component_value_is_comma(component_value: Option<&ComponentValue>) -> bool {
    matches!(
        component_value,
        Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Comma,
            ..
        }))
    )
}

pub(super) fn is_paren_block(block: &SimpleBlock) -> bool {
    matches!(block.token.token_type, TokenType::OpenParen)
}

pub(super) fn contains_only_any_value(component_values: &[ComponentValue]) -> bool {
    for component_value in component_values {
        match component_value {
            ComponentValue::Function(function) => {
                if !contains_only_any_value(&function.value) {
                    return false;
                }
            }
            ComponentValue::SimpleBlock(block) => {
                if !contains_only_any_value(&block.value) {
                    return false;
                }
            }
            ComponentValue::PreservedToken(token) => match token.token_type {
                TokenType::EndOfFile
                | TokenType::BadString
                | TokenType::BadUrl
                | TokenType::Function { .. }
                | TokenType::OpenCurly
                | TokenType::OpenParen
                | TokenType::OpenSquare
                | TokenType::CloseCurly
                | TokenType::CloseParen
                | TokenType::CloseSquare => return false,
                _ => {}
            },
        }
    }

    true
}

pub(super) fn contains_only_declaration_value(component_values: &[ComponentValue], nested: Nested) -> bool {
    for component_value in component_values {
        match component_value {
            ComponentValue::Function(function) => {
                if !contains_only_declaration_value(&function.value, Nested::Yes) {
                    return false;
                }
            }
            ComponentValue::SimpleBlock(block) => {
                if !contains_only_declaration_value(&block.value, Nested::Yes) {
                    return false;
                }
            }
            ComponentValue::PreservedToken(token) => match token.token_type {
                TokenType::EndOfFile
                | TokenType::BadString
                | TokenType::BadUrl
                | TokenType::Function { .. }
                | TokenType::OpenCurly
                | TokenType::OpenParen
                | TokenType::OpenSquare
                | TokenType::CloseCurly
                | TokenType::CloseParen
                | TokenType::CloseSquare => return false,
                TokenType::Semicolon if nested == Nested::No => return false,
                TokenType::Delim { value } if nested == Nested::No && value == u32::from(b'!') => return false,
                _ => {}
            },
        }
    }

    true
}

pub(super) fn component_values_parse_as_media_feature(
    component_values: &[ComponentValue],
) -> Option<MediaFeatureSyntax> {
    let component_values = strip_whitespace(component_values);
    if let Some(boolean) = component_values_parse_as_mf_boolean(component_values) {
        return Some(boolean);
    }

    if let Some(plain) = component_values_parse_as_mf_plain(component_values) {
        return Some(plain);
    }

    if let Some(range) = component_values_parse_as_mf_range(component_values) {
        return Some(range);
    }

    None
}

pub(super) fn component_values_parse_as_media_query(component_values: Vec<ComponentValue>) -> MediaQuerySyntax {
    // https://drafts.csswg.org/mediaqueries-5/#typedef-media-query
    // <media-query> = <media-condition>
    //              | [ not | only ]? <media-type> [ and <media-condition-without-or> ]?
    let mut parser = ComponentValueParser::new(component_values.clone());
    if let Some(condition) = parser.parse_media_condition()
        && !parser.has_next_component_value()
    {
        return MediaQuerySyntax::Valid {
            modifier: MediaQueryModifier::None,
            media_type: None,
            condition: Some(Box::new(condition)),
        };
    }

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();
    let modifier = parser.parse_media_query_modifier();
    parser.discard_whitespace();

    // https://drafts.csswg.org/mediaqueries-5/#typedef-media-type
    // <media-type> = <ident>
    let Some(media_type) = parser.parse_media_type() else {
        return MediaQuerySyntax::Invalid;
    };
    parser.discard_whitespace();
    if !parser.has_next_component_value() {
        return MediaQuerySyntax::Valid {
            modifier,
            media_type: Some(media_type),
            condition: None,
        };
    }

    if !component_value_is_ident(parser.next_component_value(), "and") {
        return MediaQuerySyntax::Invalid;
    }
    parser.index += 1;
    parser.discard_whitespace();

    // https://drafts.csswg.org/mediaqueries-5/#typedef-media-condition-without-or
    // <media-condition-without-or> = <media-not> | <media-in-parens> <media-and>*
    let Some(condition) = parser.parse_media_condition_without_or() else {
        return MediaQuerySyntax::Invalid;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return MediaQuerySyntax::Invalid;
    }

    MediaQuerySyntax::Valid {
        modifier,
        media_type: Some(media_type),
        condition: Some(Box::new(condition)),
    }
}

pub(super) fn component_values_parse_as_value_type(
    value_type_id: ValueTypeId,
    component_values: &[ComponentValue],
) -> CssValueTypeSyntaxKind {
    component_values_parse_as_generated_value_type(value_type_id, component_values)
}

pub(super) fn page_pseudo_class_from_string(input: &str) -> Option<CssPagePseudoClassKind> {
    if input.eq_ignore_ascii_case("left") {
        return Some(CssPagePseudoClassKind::Left);
    }
    if input.eq_ignore_ascii_case("right") {
        return Some(CssPagePseudoClassKind::Right);
    }
    if input.eq_ignore_ascii_case("first") {
        return Some(CssPagePseudoClassKind::First);
    }
    if input.eq_ignore_ascii_case("blank") {
        return Some(CssPagePseudoClassKind::Blank);
    }
    None
}

// https://drafts.csswg.org/css-values-5/#typedef-syntax
pub(super) fn component_values_parse_as_syntax(
    component_values: &[ComponentValue],
    limit_single_component_ident_to_custom_ident: bool,
) -> Option<SyntaxNode> {
    component_values_parse_as_syntax_with_source(component_values, limit_single_component_ident_to_custom_ident, None)
}

// https://drafts.csswg.org/css-values-5/#typedef-syntax
pub(super) fn component_values_parse_as_syntax_with_source(
    component_values: &[ComponentValue],
    limit_single_component_ident_to_custom_ident: bool,
    filtered_input: Option<&str>,
) -> Option<SyntaxNode> {
    // <syntax> = '*' | <syntax-component> [ <syntax-combinator> <syntax-component> ]* | <syntax-string>
    // <syntax-component> = <syntax-single-component> <syntax-multiplier>?
    //                    | '<' transform-list '>'
    // <syntax-single-component> = '<' <syntax-type-name> '>' | <ident>
    // <syntax-type-name> = angle | color | custom-ident | image | integer
    //                    | length | length-percentage | number
    //                    | percentage | resolution | string | time
    //                    | url | transform-function
    // <syntax-combinator> = '|'
    // <syntax-multiplier> = [ '#' | '+' ]
    //
    // <syntax-string> = <string>
    // FIXME: Eventually, extend this to also parse *any* CSS grammar, not just for the <syntax> type.
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();

    // '*'
    if component_value_is_delim(parser.next_component_value(), '*') {
        parser.index += 1;
        parser.discard_whitespace();
        if parser.next_component_value().is_some() {
            return None;
        }
        return Some(SyntaxNode::Universal);
    }

    // <syntax-string> = <string>
    // A <syntax-string> is a <string> whose value successfully parses as a <syntax>, and represents the same value as
    // that <syntax> would.
    // NB: For now, this is the only time a string is allowed in a <syntax>.
    if let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::String { value },
        ..
    })) = parser.next_component_value()
    {
        let value = value.clone();
        parser.index += 1;
        parser.discard_whitespace();
        if parser.next_component_value().is_some() {
            return None;
        }
        return parse_as_syntax_string(&value, limit_single_component_ident_to_custom_ident);
    }

    // <syntax-component> [ <syntax-combinator> <syntax-component> ]*
    let mut syntax_components = vec![parse_syntax_component(
        &mut parser,
        limit_single_component_ident_to_custom_ident,
        filtered_input,
    )?];

    parser.discard_whitespace();
    while parser.next_component_value().is_some() {
        let combinator = parse_syntax_combinator(&mut parser);
        parser.discard_whitespace();
        let component = parse_syntax_component(
            &mut parser,
            limit_single_component_ident_to_custom_ident,
            filtered_input,
        );
        parser.discard_whitespace();
        if combinator.is_none() || component.is_none() {
            return None;
        }

        // FIXME: Make this logic smarter once we have more than one type of combinator.
        // For now, assume we're always making an AlternativesSyntaxNode.
        debug_assert_eq!(combinator, Some('|'));

        syntax_components.push(component.expect("checked above"));
    }

    if syntax_components.len() == 1 {
        return syntax_components.pop();
    }
    Some(SyntaxNode::Alternatives(syntax_components))
}

pub(super) fn parse_syntax_component(
    parser: &mut ComponentValueParser,
    limit_single_component_ident_to_custom_ident: bool,
    filtered_input: Option<&str>,
) -> Option<SyntaxNode> {
    // <syntax-component> = <syntax-single-component> <syntax-multiplier>?
    //                    | '<' transform-list '>'
    let saved_index = parser.index;
    parser.discard_whitespace();

    // '<' transform-list '>'
    if component_value_is_delim(parser.next_component_value(), '<') {
        parser.index += 1;
        let ident_token = parser.consume_the_next_component_value();
        let end_token = parser.consume_the_next_component_value();

        if let Some(ComponentValue::PreservedToken(token)) = ident_token
            && let TokenType::Ident { value } = &token.token_type
            && value == "transform-list"
            && syntax_type_name_source_matches_value(&token, value, filtered_input)
            && component_value_is_delim(end_token.as_ref(), '>')
        {
            return Some(SyntaxNode::Type("transform-list".to_string()));
        }

        parser.index = saved_index;
    }

    // <syntax-single-component> <syntax-multiplier>?
    let syntax_single_component =
        parse_syntax_single_component(parser, limit_single_component_ident_to_custom_ident, filtered_input)?;

    match parse_syntax_multiplier(parser) {
        None => Some(syntax_single_component),
        Some('#') => Some(SyntaxNode::CommaSeparatedMultiplier(Box::new(syntax_single_component))),
        Some('+') => Some(SyntaxNode::Multiplier(Box::new(syntax_single_component))),
        _ => None,
    }
}

pub(super) fn parse_syntax_single_component(
    parser: &mut ComponentValueParser,
    limit_single_component_ident_to_custom_ident: bool,
    filtered_input: Option<&str>,
) -> Option<SyntaxNode> {
    // <syntax-single-component> = '<' <syntax-type-name> '>' | <ident>
    // <syntax-type-name> = angle | color | custom-ident | image | integer
    //                    | length | length-percentage | number
    //                    | percentage | resolution | string | time
    //                    | url | transform-function
    let saved_index = parser.index;
    parser.discard_whitespace();

    // <ident>
    if let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    })) = parser.next_component_value()
    {
        let value = value.clone();
        // AD-HOC: Some users (i.e. the @property syntax descriptor) only allow custom idents here,
        //         https://github.com/w3c/csswg-drafts/issues/13614
        if limit_single_component_ident_to_custom_ident
            && (matches_css_wide_keyword(&value) || value.eq_ignore_ascii_case("default"))
        {
            return None;
        }

        parser.index += 1;
        return Some(SyntaxNode::Ident(value));
    }

    // '<' <syntax-type-name> '>'
    if component_value_is_delim(parser.next_component_value(), '<') {
        parser.index += 1;
        let type_name = parser.consume_the_next_component_value();
        let end_token = parser.consume_the_next_component_value();

        if let Some(ComponentValue::PreservedToken(token)) = type_name
            && let TokenType::Ident { value } = &token.token_type
            && component_value_is_delim(end_token.as_ref(), '>')
            && is_syntax_type_name(value)
            && syntax_type_name_source_matches_value(&token, value, filtered_input)
        {
            return Some(SyntaxNode::Type(value.clone()));
        }
    }

    parser.index = saved_index;
    None
}

pub(super) fn parse_css_type(
    parser: &mut ComponentValueParser,
    limit_single_component_ident_to_custom_ident: bool,
    filtered_input: Option<&str>,
) -> Option<SyntaxNode> {
    // https://drafts.csswg.org/css-mixins-1/#function-rule
    // <css-type> = <syntax-component> | <type()>
    // <type()> = type( <syntax> )
    let saved_index = parser.index;

    // <syntax-component>
    if let Some(syntax_component) =
        parse_syntax_component(parser, limit_single_component_ident_to_custom_ident, filtered_input)
    {
        return Some(syntax_component);
    }

    parser.index = saved_index;
    parser.discard_whitespace();

    // <type()> = type( <syntax> )
    if let Some(ComponentValue::Function(function)) = parser.next_component_value()
        && function.name.eq_ignore_ascii_case("type")
    {
        let syntax = component_values_parse_as_syntax_with_source(
            &function.value,
            limit_single_component_ident_to_custom_ident,
            filtered_input,
        )?;
        parser.index += 1;
        return Some(syntax);
    }

    parser.index = saved_index;
    None
}

static SYNTAX_TYPE_NAMES: &[&str] = &[
    "angle",
    "color",
    "custom-ident",
    "image",
    "integer",
    "length",
    "length-percentage",
    "number",
    "percentage",
    "resolution",
    "string",
    "time",
    "url",
    "transform-function",
];

pub(super) fn is_syntax_type_name(value: &str) -> bool {
    SYNTAX_TYPE_NAMES.contains(&value)
}

pub(super) fn syntax_type_name_source_matches_value(token: &Token, value: &str, filtered_input: Option<&str>) -> bool {
    let Some(filtered_input) = filtered_input else {
        return true;
    };
    token
        .original_source(filtered_input)
        .is_some_and(|source| source == value)
}

pub(super) fn parse_syntax_multiplier(parser: &mut ComponentValueParser) -> Option<char> {
    // <syntax-multiplier> = [ '#' | '+' ]
    let saved_index = parser.index;
    let delim = parser.consume_the_next_component_value();
    if component_value_is_delim(delim.as_ref(), '#') {
        return Some('#');
    }
    if component_value_is_delim(delim.as_ref(), '+') {
        return Some('+');
    }

    parser.index = saved_index;
    None
}

pub(super) fn parse_syntax_combinator(parser: &mut ComponentValueParser) -> Option<char> {
    // <syntax-combinator> = '|'
    let saved_index = parser.index;
    parser.discard_whitespace();

    if component_value_is_delim(parser.next_component_value(), '|') {
        parser.index += 1;
        return Some('|');
    }

    parser.index = saved_index;
    None
}

pub(super) fn component_values_parse_as_mf_boolean(component_values: &[ComponentValue]) -> Option<MediaFeatureSyntax> {
    // <mf-boolean> = <mf-name>
    component_values_parse_as_mf_name(component_values, AllowMinMaxPrefix::No).map(MediaFeatureSyntax::Boolean)
}

pub(super) fn component_values_parse_as_mf_plain(component_values: &[ComponentValue]) -> Option<MediaFeatureSyntax> {
    // <mf-plain> = <mf-name> : <mf-value>
    let colon_index = component_values.iter().position(|component_value| {
        matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Colon,
                ..
            })
        )
    })?;

    let name = strip_whitespace(&component_values[..colon_index]);
    let value = strip_whitespace(&component_values[colon_index + 1..]);
    let name = component_values_parse_as_mf_name(name, AllowMinMaxPrefix::Yes)?;
    if !component_values_parse_as_mf_value(value) {
        return None;
    }

    Some(MediaFeatureSyntax::Plain {
        name,
        value: value.to_vec(),
    })
}

pub(super) fn component_values_parse_as_mf_range(component_values: &[ComponentValue]) -> Option<MediaFeatureSyntax> {
    // <mf-range> = <mf-name> <mf-comparison> <mf-value>
    //             | <mf-value> <mf-comparison> <mf-name>
    //             | <mf-value> <mf-lt> <mf-name> <mf-lt> <mf-value>
    //             | <mf-value> <mf-gt> <mf-name> <mf-gt> <mf-value>
    let comparison_indices: Vec<usize> = component_values
        .iter()
        .enumerate()
        .filter_map(|(index, _)| parse_mf_comparison_at(component_values, index).map(|(_, end)| (index, end)))
        .scan(0, |minimum_index, (index, end)| {
            if index < *minimum_index {
                return Some(None);
            }
            *minimum_index = end;
            Some(Some(index))
        })
        .flatten()
        .collect();

    if comparison_indices.len() == 1 {
        let comparison_index = comparison_indices[0];
        let (comparison, comparison_end) = parse_mf_comparison_at(component_values, comparison_index)
            .expect("comparison index must parse as a comparison");
        let left = strip_whitespace(&component_values[..comparison_index]);
        let right = strip_whitespace(&component_values[comparison_end..]);

        if let Some(name) = component_values_parse_as_mf_range_name(left)
            && component_values_parse_as_mf_range_value(name.id, right)
        {
            return Some(MediaFeatureSyntax::HalfRangeNameFirst {
                name,
                comparison,
                value: right.to_vec(),
            });
        }

        if component_values_parse_as_mf_value(left)
            && let Some(name) = component_values_parse_as_mf_range_name(right)
            && component_values_parse_as_mf_range_value(name.id, left)
        {
            return Some(MediaFeatureSyntax::HalfRangeValueFirst {
                value: left.to_vec(),
                comparison,
                name,
            });
        }

        return None;
    }

    if comparison_indices.len() == 2 {
        let left_comparison_index = comparison_indices[0];
        let (left_comparison, left_comparison_end) = parse_mf_comparison_at(component_values, left_comparison_index)
            .expect("comparison index must parse as a comparison");
        let right_comparison_index = comparison_indices[1];
        let (right_comparison, right_comparison_end) = parse_mf_comparison_at(component_values, right_comparison_index)
            .expect("comparison index must parse as a comparison");

        if !mf_comparisons_are_range_compatible(left_comparison, right_comparison) {
            return None;
        }

        let left_value = strip_whitespace(&component_values[..left_comparison_index]);
        let name = strip_whitespace(&component_values[left_comparison_end..right_comparison_index]);
        let right_value = strip_whitespace(&component_values[right_comparison_end..]);
        if let Some(name) = component_values_parse_as_mf_range_name(name)
            && component_values_parse_as_mf_range_value(name.id, left_value)
            && component_values_parse_as_mf_range_value(name.id, right_value)
        {
            return Some(MediaFeatureSyntax::Range {
                left_value: left_value.to_vec(),
                left_comparison,
                name,
                right_comparison,
                right_value: right_value.to_vec(),
            });
        }
    }

    None
}

#[derive(Clone, Copy)]
enum AllowMinMaxPrefix {
    No,
    Yes,
}

pub(crate) fn component_values_parse_as_ident(component_values: &[ComponentValue], expected: &str) -> bool {
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
    else {
        return false;
    };

    value.eq_ignore_ascii_case(expected)
}

pub(crate) fn component_values_parse_as_number(component_values: &[ComponentValue], min: f64, max: f64) -> bool {
    let [component_value] = component_values else {
        return false;
    };

    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() >= min && number.value() <= max,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(_) => true,
        _ => false,
    }
}

pub(crate) fn component_values_number_value(component_values: &[ComponentValue], min: f64, max: f64) -> Option<f64> {
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }),
    ] = component_values
    else {
        return None;
    };

    if number.value() < min || number.value() > max {
        return None;
    }

    Some(number.value())
}

pub(super) fn component_value_parse_as_angle(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) => matches!(dimension_for_unit(unit), Some(DimensionType::Angle)),
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(_) => true,
        _ => false,
    }
}

pub(crate) fn component_values_parse_as_string(component_values: &[ComponentValue]) -> bool {
    matches!(
        component_values,
        [ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { .. },
            ..
        })]
    )
}

pub(crate) fn component_values_string_value(component_values: &[ComponentValue]) -> Option<&str> {
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { value },
            ..
        }),
    ] = component_values
    else {
        return None;
    };

    Some(value)
}

pub(crate) fn component_values_parse_as_custom_ident(component_values: &[ComponentValue]) -> bool {
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
    else {
        return false;
    };

    // The CSS-wide keywords are not valid <custom-ident>s.
    !matches_css_wide_keyword(value)
        // The default keyword is reserved and is also not a valid <custom-ident>.
        && !value.eq_ignore_ascii_case("default")
}

pub(crate) fn component_values_custom_ident_value(component_values: &[ComponentValue]) -> Option<&str> {
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
    else {
        return None;
    };

    if component_values_parse_as_custom_ident(component_values) {
        Some(value)
    } else {
        None
    }
}

pub(super) fn is_valid_custom_ident(value: &str, blacklist: &[&str]) -> bool {
    // The CSS-wide keywords are not valid <custom-ident>s.
    if matches_css_wide_keyword(value) {
        return false;
    }

    // The default keyword is reserved and is also not a valid <custom-ident>.
    if value.eq_ignore_ascii_case("default") {
        return false;
    }

    !blacklist.iter().any(|keyword| value.eq_ignore_ascii_case(keyword))
}

pub(super) fn matches_css_wide_keyword(value: &str) -> bool {
    value.eq_ignore_ascii_case("inherit")
        || value.eq_ignore_ascii_case("initial")
        || value.eq_ignore_ascii_case("unset")
        || value.eq_ignore_ascii_case("revert")
        || value.eq_ignore_ascii_case("revert-layer")
}

pub(super) fn matches_generic_font_family_keyword(value: &str) -> bool {
    value.eq_ignore_ascii_case("serif")
        || value.eq_ignore_ascii_case("sans-serif")
        || value.eq_ignore_ascii_case("cursive")
        || value.eq_ignore_ascii_case("fantasy")
        || value.eq_ignore_ascii_case("math")
        || value.eq_ignore_ascii_case("monospace")
        || value.eq_ignore_ascii_case("ui-serif")
        || value.eq_ignore_ascii_case("ui-sans-serif")
        || value.eq_ignore_ascii_case("ui-monospace")
        || value.eq_ignore_ascii_case("ui-rounded")
}

pub(super) fn matches_east_asian_variant_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("jis78")
        || value.eq_ignore_ascii_case("jis83")
        || value.eq_ignore_ascii_case("jis90")
        || value.eq_ignore_ascii_case("jis04")
        || value.eq_ignore_ascii_case("simplified")
        || value.eq_ignore_ascii_case("traditional")
}

pub(super) fn matches_east_asian_width_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("full-width") || value.eq_ignore_ascii_case("proportional-width")
}

pub(super) fn matches_numeric_figure_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("lining-nums") || value.eq_ignore_ascii_case("oldstyle-nums")
}

pub(super) fn matches_numeric_spacing_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("proportional-nums") || value.eq_ignore_ascii_case("tabular-nums")
}

pub(super) fn matches_numeric_fraction_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("diagonal-fractions") || value.eq_ignore_ascii_case("stacked-fractions")
}

pub(super) fn matches_common_lig_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("common-ligatures") || value.eq_ignore_ascii_case("no-common-ligatures")
}

pub(super) fn matches_discretionary_lig_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("discretionary-ligatures") || value.eq_ignore_ascii_case("no-discretionary-ligatures")
}

pub(super) fn matches_historical_lig_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("historical-ligatures") || value.eq_ignore_ascii_case("no-historical-ligatures")
}

pub(super) fn matches_contextual_alt_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("contextual") || value.eq_ignore_ascii_case("no-contextual")
}

pub(super) fn matches_font_variant_caps_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("small-caps")
        || value.eq_ignore_ascii_case("all-small-caps")
        || value.eq_ignore_ascii_case("petite-caps")
        || value.eq_ignore_ascii_case("all-petite-caps")
        || value.eq_ignore_ascii_case("unicase")
        || value.eq_ignore_ascii_case("titling-caps")
}

pub(super) fn matches_font_variant_emoji_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("text") || value.eq_ignore_ascii_case("emoji") || value.eq_ignore_ascii_case("unicode")
}

pub(super) fn matches_font_variant_position_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("sub") || value.eq_ignore_ascii_case("super")
}

pub(super) fn is_a_custom_property_name_string(value: &str) -> bool {
    value.starts_with("--") && value != "--"
}

fn component_values_parse_as_mf_name(
    component_values: &[ComponentValue],
    allow_min_max_prefix: AllowMinMaxPrefix,
) -> Option<MediaFeatureName> {
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
    else {
        return None;
    };

    if let Some(id) = media_feature_id_from_string(value) {
        return Some(MediaFeatureName {
            kind: MediaFeatureNameKind::Normal,
            id,
        });
    }

    if matches!(allow_min_max_prefix, AllowMinMaxPrefix::No) {
        return None;
    }

    let prefix = value.get(..4);
    if !matches!(prefix, Some(prefix) if prefix.eq_ignore_ascii_case("min-") || prefix.eq_ignore_ascii_case("max-")) {
        return None;
    }

    let adjusted_name = &value[4..];
    let id = media_feature_id_from_string(adjusted_name)?;
    if !media_feature_type_is_range(id) {
        return None;
    }

    Some(MediaFeatureName {
        kind: if prefix.expect("prefix must be present").eq_ignore_ascii_case("min-") {
            MediaFeatureNameKind::Min
        } else {
            MediaFeatureNameKind::Max
        },
        id,
    })
}

pub(super) fn component_values_parse_as_mf_range_name(component_values: &[ComponentValue]) -> Option<MediaFeatureName> {
    let name = component_values_parse_as_mf_name(component_values, AllowMinMaxPrefix::No)?;

    // The only significant difference between the two types is that “range” media features
    // can be evaluated in a range context and accept “min-” and “max-” prefixes on their name.
    if !media_feature_type_is_range(name.id) {
        return None;
    }

    Some(name)
}

pub(super) fn component_values_parse_as_mf_value(component_values: &[ComponentValue]) -> bool {
    !component_values.is_empty() && component_values.iter().all(is_media_feature_value_component_value)
}

pub(super) fn component_values_parse_as_mf_range_value(
    media_feature_id: MediaFeatureId,
    component_values: &[ComponentValue],
) -> bool {
    component_values_parse_as_mf_value(component_values)
        && component_values_parse_as_mf_value_syntax(media_feature_id, component_values)
            != MediaFeatureValueSyntaxKind::Ident
}

pub(super) fn component_values_parse_as_mf_value_syntax(
    media_feature_id: MediaFeatureId,
    component_values: &[ComponentValue],
) -> MediaFeatureValueSyntaxKind {
    let component_values = strip_whitespace(component_values);
    if !component_values_parse_as_mf_value(component_values) {
        return MediaFeatureValueSyntaxKind::Invalid;
    }

    // https://drafts.csswg.org/mediaqueries-5/#typedef-mf-value
    // <mf-value> = <number> | <dimension> | <ident> | <ratio>
    if let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
        && media_feature_accepts_identifier(media_feature_id, value)
    {
        return MediaFeatureValueSyntaxKind::Ident;
    }

    if media_feature_accepts_type(media_feature_id, MediaFeatureValueType::Boolean)
        && (component_values_parse_as_mq_boolean(component_values)
            || component_values_parse_as_math_backed_mf_value(component_values))
    {
        return MediaFeatureValueSyntaxKind::Boolean;
    }

    if media_feature_accepts_type(media_feature_id, MediaFeatureValueType::Integer)
        && (component_values_parse_as_integer(component_values)
            || component_values_parse_as_math_backed_mf_value(component_values))
    {
        return MediaFeatureValueSyntaxKind::Integer;
    }

    if media_feature_accepts_type(media_feature_id, MediaFeatureValueType::Length)
        && (component_values_parse_as_length(component_values)
            || component_values_parse_as_math_backed_mf_value(component_values))
    {
        return MediaFeatureValueSyntaxKind::Length;
    }

    if media_feature_accepts_type(media_feature_id, MediaFeatureValueType::Ratio)
        && component_values_parse_as_ratio(component_values)
    {
        return MediaFeatureValueSyntaxKind::Ratio;
    }

    if media_feature_accepts_type(media_feature_id, MediaFeatureValueType::Resolution)
        && (component_values_parse_as_resolution(component_values)
            || component_values_parse_as_math_backed_mf_value(component_values))
    {
        return MediaFeatureValueSyntaxKind::Resolution;
    }

    MediaFeatureValueSyntaxKind::Unknown
}

pub(super) fn component_values_parse_as_math_backed_mf_value(component_values: &[ComponentValue]) -> bool {
    matches!(component_values, [ComponentValue::Function(_)])
}

pub(super) fn component_values_parse_as_mq_boolean(component_values: &[ComponentValue]) -> bool {
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }),
    ] = component_values
    else {
        return false;
    };

    number_is_integer(*number) && matches!(number.value(), 0.0 | 1.0)
}

pub(super) fn component_values_parse_as_integer(component_values: &[ComponentValue]) -> bool {
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }),
    ] = component_values
    else {
        return false;
    };

    number_is_integer(*number)
}

pub(super) fn component_values_parse_as_length(component_values: &[ComponentValue]) -> bool {
    let [component_value] = component_values else {
        return false;
    };

    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) => matches!(dimension_for_unit(unit), Some(DimensionType::Length)),
        // https://drafts.csswg.org/css-values-4/#zero-value
        // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() == 0.0,
        _ => false,
    }
}

pub(super) fn component_value_parse_as_length_percentage(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) => matches!(dimension_for_unit(unit), Some(DimensionType::Length)),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { .. },
            ..
        }) => true,
        // https://drafts.csswg.org/css-values-4/#zero-value
        // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() == 0.0,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => is_math_function_name(&function.name),
        _ => false,
    }
}

pub(super) fn component_value_parse_as_length(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) => matches!(dimension_for_unit(unit), Some(DimensionType::Length)),
        // https://drafts.csswg.org/css-values-4/#zero-value
        // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() == 0.0,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => {
            is_math_function_name(&function.name) || function.name.eq_ignore_ascii_case("anchor-size")
        }
        _ => false,
    }
}

pub(super) fn component_value_parse_as_length_descriptor(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) => matches!(dimension_for_unit(unit), Some(DimensionType::Length)),
        // https://drafts.csswg.org/css-values-4/#zero-value
        // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() == 0.0,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => is_math_function_name(&function.name),
        _ => false,
    }
}

pub(super) fn component_value_parse_as_nonnegative_length_descriptor(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) => number.value() >= 0.0 && matches!(dimension_for_unit(unit), Some(DimensionType::Length)),
        // https://drafts.csswg.org/css-values-4/#zero-value
        // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() == 0.0,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => is_math_function_name(&function.name),
        _ => false,
    }
}

pub(super) fn component_value_parse_as_positive_percentage_descriptor(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { number },
            ..
        }) => number.value() >= 0.0,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => is_math_function_name(&function.name),
        _ => false,
    }
}

pub(super) fn is_math_function_name(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "abs"
            | "acos"
            | "asin"
            | "atan"
            | "atan2"
            | "calc"
            | "clamp"
            | "cos"
            | "exp"
            | "hypot"
            | "log"
            | "max"
            | "min"
            | "mod"
            | "pow"
            | "random"
            | "rem"
            | "round"
            | "sign"
            | "sin"
            | "sqrt"
            | "tan"
    )
}

pub(super) fn scroll_function_scroller_from_string(input: &str) -> Option<CssScrollFunctionScrollerKind> {
    if input.eq_ignore_ascii_case("nearest") {
        return Some(CssScrollFunctionScrollerKind::Nearest);
    }
    if input.eq_ignore_ascii_case("root") {
        return Some(CssScrollFunctionScrollerKind::Root);
    }
    if input.eq_ignore_ascii_case("self") {
        return Some(CssScrollFunctionScrollerKind::Self_);
    }
    None
}

pub(super) fn scroll_function_axis_from_string(input: &str) -> Option<CssScrollFunctionAxisKind> {
    if input.eq_ignore_ascii_case("block") {
        return Some(CssScrollFunctionAxisKind::Block);
    }
    if input.eq_ignore_ascii_case("inline") {
        return Some(CssScrollFunctionAxisKind::Inline);
    }
    if input.eq_ignore_ascii_case("x") {
        return Some(CssScrollFunctionAxisKind::X);
    }
    if input.eq_ignore_ascii_case("y") {
        return Some(CssScrollFunctionAxisKind::Y);
    }
    None
}

pub(super) fn is_page_size_keyword(input: &str) -> bool {
    // https://drafts.csswg.org/css-page-3/#typedef-page-size-page-size
    // <page-size> = A5 | A4 | A3 | B5 | B4 | JIS-B5 | JIS-B4 | letter | legal | ledger
    page_size_keyword_from_string(input).is_some()
}

pub(super) fn page_size_keyword_from_string(input: &str) -> Option<CssPageSizeKeyword> {
    match input.to_ascii_lowercase().as_str() {
        "a5" => Some(CssPageSizeKeyword::A5),
        "a4" => Some(CssPageSizeKeyword::A4),
        "a3" => Some(CssPageSizeKeyword::A3),
        "b5" => Some(CssPageSizeKeyword::B5),
        "b4" => Some(CssPageSizeKeyword::B4),
        "jis-b5" => Some(CssPageSizeKeyword::JisB5),
        "jis-b4" => Some(CssPageSizeKeyword::JisB4),
        "letter" => Some(CssPageSizeKeyword::Letter),
        "legal" => Some(CssPageSizeKeyword::Legal),
        "ledger" => Some(CssPageSizeKeyword::Ledger),
        _ => None,
    }
}

pub(super) fn page_size_orientation_from_string(input: &str) -> Option<CssPageSizeOrientation> {
    match input.to_ascii_lowercase().as_str() {
        "portrait" => Some(CssPageSizeOrientation::Portrait),
        "landscape" => Some(CssPageSizeOrientation::Landscape),
        _ => None,
    }
}

pub(super) fn component_values_parse_as_resolution(component_values: &[ComponentValue]) -> bool {
    let [component_value] = component_values else {
        return false;
    };

    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) => number.value() >= 0.0 && matches!(dimension_for_unit(unit), Some(DimensionType::Resolution)),
        _ => false,
    }
}

pub(super) fn component_values_parse_as_ratio(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-values-4/#ratios
    // <ratio> = <number [0,∞]> [ / <number [0,∞]> ]?
    let component_values = strip_whitespace(component_values);
    let [numerator] = component_values else {
        return component_values_parse_as_ratio_with_denominator(component_values);
    };

    component_value_parse_as_non_negative_number(numerator)
}

pub(super) fn component_values_parse_as_ratio_with_denominator(component_values: &[ComponentValue]) -> bool {
    let Some((slash_index, _)) = component_values.iter().enumerate().find(|(_, component_value)| {
        matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Delim { value },
                ..
            }) if *value == '/' as u32
        )
    }) else {
        return false;
    };

    let numerator = strip_whitespace(&component_values[..slash_index]);
    let denominator = strip_whitespace(&component_values[slash_index + 1..]);
    let [numerator] = numerator else {
        return false;
    };
    let [denominator] = denominator else {
        return false;
    };

    component_value_parse_as_non_negative_number(numerator) && component_value_parse_as_non_negative_number(denominator)
}

pub(super) fn component_value_parse_as_non_negative_number(component_value: &ComponentValue) -> bool {
    component_value_non_negative_number_value(component_value).is_some()
        || matches!(component_value, ComponentValue::Function(_))
}

pub(super) fn component_value_non_negative_number_value(component_value: &ComponentValue) -> Option<f64> {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if number.value() >= 0.0
    )
    .then(|| match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value(),
        _ => 0.0,
    })
}

pub(super) fn number_is_integer(number: NumericValue) -> bool {
    matches!(
        number.number_type(),
        CssNumberType::Integer | CssNumberType::IntegerWithExplicitSign
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MfComparison {
    Equal,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

pub(super) fn parse_mf_comparison_at(
    component_values: &[ComponentValue],
    index: usize,
) -> Option<(MfComparison, usize)> {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Delim { value },
        ..
    }) = component_values.get(index)?
    else {
        return None;
    };

    match *value {
        value if value == '=' as u32 => Some((MfComparison::Equal, index + 1)),
        value if value == '<' as u32 => {
            if matches!(
                component_values.get(index + 1),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Delim { value },
                    ..
                })) if *value == '=' as u32
            ) {
                Some((MfComparison::LessThanOrEqual, index + 2))
            } else {
                Some((MfComparison::LessThan, index + 1))
            }
        }
        value if value == '>' as u32 => {
            if matches!(
                component_values.get(index + 1),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Delim { value },
                    ..
                })) if *value == '=' as u32
            ) {
                Some((MfComparison::GreaterThanOrEqual, index + 2))
            } else {
                Some((MfComparison::GreaterThan, index + 1))
            }
        }
        _ => None,
    }
}

pub(super) fn mf_comparisons_are_range_compatible(left: MfComparison, right: MfComparison) -> bool {
    matches!(
        (left, right),
        (
            MfComparison::LessThan | MfComparison::LessThanOrEqual,
            MfComparison::LessThan | MfComparison::LessThanOrEqual
        ) | (
            MfComparison::GreaterThan | MfComparison::GreaterThanOrEqual,
            MfComparison::GreaterThan | MfComparison::GreaterThanOrEqual
        )
    )
}

pub(super) fn is_media_feature_value_component_value(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::Function(_) | ComponentValue::SimpleBlock(_) => true,
        ComponentValue::PreservedToken(token) => match token.token_type {
            TokenType::Ident { .. }
            | TokenType::Function { .. }
            | TokenType::AtKeyword { .. }
            | TokenType::Hash { .. }
            | TokenType::String { .. }
            | TokenType::BadString
            | TokenType::Url { .. }
            | TokenType::BadUrl
            | TokenType::Number { .. }
            | TokenType::Percentage { .. }
            | TokenType::Dimension { .. }
            | TokenType::Whitespace
            | TokenType::Comma => true,
            TokenType::Delim { value } => {
                !matches!(value, value if value == '<' as u32 || value == '>' as u32 || value == '=' as u32)
            }
            TokenType::EndOfFile
            | TokenType::Cdo
            | TokenType::Cdc
            | TokenType::Colon
            | TokenType::Semicolon
            | TokenType::OpenSquare
            | TokenType::CloseSquare
            | TokenType::OpenParen
            | TokenType::CloseParen
            | TokenType::OpenCurly
            | TokenType::CloseCurly => false,
        },
    }
}
