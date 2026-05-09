/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(super) fn parse_rust_owned_math_function(
    value_type: PropertyValueType,
    component_values: &[ComponentValue],
    filtered_input: &[u8],
) -> Option<RustOwnedMathFunction> {
    // https://drafts.csswg.org/css-values-4/#math
    // A math function represents a numeric value.
    let [ComponentValue::Function(function)] = component_values else {
        return None;
    };
    if !is_math_function_name(&function.name) {
        return None;
    }

    let filtered_input = std::str::from_utf8(filtered_input)
        .expect("rust_css_parse_component_values received non-UTF-8 input after C++ decoding");
    let source = serialize_component_values_for_reparsing(component_values, filtered_input)?;

    let calculation = parse_rust_owned_calculation_function(function)?;

    Some(RustOwnedMathFunction {
        name: function.name.clone(),
        arguments: function.value.clone(),
        calculation: Box::new(calculation),
        source,
        value_type,
    })
}

pub(super) fn parse_rust_owned_tree_counting_function(
    value_type: PropertyValueType,
    component_values: &[ComponentValue],
) -> Option<RustOwnedTreeCountingFunction> {
    // https://drafts.csswg.org/css-values-5/#tree-counting
    // <tree-counting-function> = <sibling-index()> | <sibling-count()>
    let [ComponentValue::Function(function)] = component_values else {
        return None;
    };
    if !function.name.eq_ignore_ascii_case("sibling-index") && !function.name.eq_ignore_ascii_case("sibling-count") {
        return None;
    }
    if !strip_whitespace(&function.value).is_empty() {
        return None;
    }

    Some(RustOwnedTreeCountingFunction {
        function: if function.name.eq_ignore_ascii_case("sibling-index") {
            RustOwnedTreeCountingFunctionKind::SiblingIndex
        } else {
            RustOwnedTreeCountingFunctionKind::SiblingCount
        },
        value_type,
    })
}

pub(super) fn filtered_input_to_string(filtered_input: &[u8]) -> String {
    String::from_utf8_lossy(filtered_input).to_string()
}

pub(super) fn split_component_values_on_comma(component_values: &[ComponentValue]) -> Vec<&[ComponentValue]> {
    let mut groups = Vec::new();
    let mut start = 0;
    for (index, component_value) in component_values.iter().enumerate() {
        if matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            })
        ) {
            groups.push(&component_values[start..index]);
            start = index + 1;
        }
    }
    groups.push(&component_values[start..]);
    groups
}

pub(crate) fn parse_style_value_for_property<C>(property_ids: &[u16], filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    parse_style_value_for_property_with_options(
        property_ids,
        filtered_input,
        CssPrimitiveValueOptions::default(),
        &mut callback,
    )
}

pub(crate) fn parse_style_value_for_property_with_options<C>(
    property_ids: &[u16],
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
    mut callback: C,
) -> bool
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
{
    parse_style_value_for_property_with_options_and_calculation_callback(
        property_ids,
        filtered_input,
        primitive_value_options,
        &mut callback,
        |_, _, _, _, _, _| {},
    )
}

pub(crate) fn parse_style_value_for_property_with_options_and_calculation_callback<C, D>(
    property_ids: &[u16],
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
    mut callback: C,
    mut calculation_callback: D,
) -> bool
where
    C: FnMut(CssStyleValueKind, u16, CssPrimitiveValueKind, bool, f64, bool, f64, u8, u8, u8, u8, &[u8], &str),
    D: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
{
    let RustOwnedStyleValueParseResult::Parsed(style_value) =
        parse_rust_owned_style_value_for_property_with_options(property_ids, filtered_input, primitive_value_options)
    else {
        return false;
    };

    emit_rust_owned_style_value(&style_value, &mut callback);
    if let RustOwnedStyleValueKind::MathFunction(value) = &style_value.value {
        emit_rust_owned_calculation_tree(&value.calculation, &mut calculation_callback);
    }
    true
}

pub(super) fn numeric_range_limit_to_f64(limit: Option<f64>, value_type: PropertyValueType, is_minimum: bool) -> f64 {
    match (limit, value_type, is_minimum) {
        (Some(limit), _, _) => limit,
        (None, PropertyValueType::Integer, true) => i32::MIN as f64,
        (None, PropertyValueType::Integer, false) => i32::MAX as f64,
        (None, _, true) => f32::MIN as f64,
        (None, _, false) => f32::MAX as f64,
    }
}

pub(super) fn numeric_range_to_f64(range: PropertyNumericRange, value_type: PropertyValueType) -> (f64, f64) {
    (
        numeric_range_limit_to_f64(range.minimum, value_type, true),
        numeric_range_limit_to_f64(range.maximum, value_type, false),
    )
}

pub(crate) fn property_numeric_metadata<C>(property_ids: &[u16], value_type: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16, f64, f64, bool, f64, f64, bool),
{
    let Ok(value_type) = std::str::from_utf8(value_type) else {
        return false;
    };
    let Some(value_type) = property_value_type_from_css_value_type_name(value_type) else {
        return false;
    };

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        if !property_accepts_value_type(property_id, value_type) {
            continue;
        }
        let range = property_accepted_range_by_value_type(property_id, value_type).unwrap_or(PropertyNumericRange {
            minimum: None,
            maximum: None,
        });

        let (minimum, maximum) = numeric_range_to_f64(range, value_type);
        let percentages_resolve_to_value_type =
            match (property_resolves_percentages_relative_to(property_id), value_type) {
                (Some(relative_to), _) if relative_to == value_type => true,
                (Some(PropertyValueType::Angle), PropertyValueType::AnglePercentage)
                | (Some(PropertyValueType::Frequency), PropertyValueType::FrequencyPercentage)
                | (Some(PropertyValueType::Length), PropertyValueType::LengthPercentage)
                | (Some(PropertyValueType::Time), PropertyValueType::TimePercentage) => true,
                _ => false,
            };
        if percentages_resolve_to_value_type {
            let percentage_range = property_accepted_range_by_value_type(property_id, PropertyValueType::Percentage)
                .unwrap_or(PropertyNumericRange {
                    minimum: None,
                    maximum: None,
                });
            let (percentage_minimum, percentage_maximum) =
                numeric_range_to_f64(percentage_range, PropertyValueType::Percentage);
            callback(
                property_id as u16,
                minimum,
                maximum,
                true,
                percentage_minimum,
                percentage_maximum,
                true,
            );
            return true;
        }

        callback(property_id as u16, minimum, maximum, false, 0.0, 0.0, false);
        return true;
    }

    false
}

pub(super) fn parse_as_syntax_string(
    input: &str,
    limit_single_component_ident_to_custom_ident: bool,
) -> Option<SyntaxNode> {
    let (mut parser, _) = parser_from_filtered_input(input.as_bytes());
    let component_values = parser.parse_a_list_of_component_values();
    component_values_parse_as_syntax_with_source(
        &component_values,
        limit_single_component_ident_to_custom_ident,
        Some(input),
    )
}

pub(crate) fn parse_as_syntax<C>(
    filtered_input: &[u8],
    limit_single_component_ident_to_custom_ident: bool,
    mut callback: C,
) where
    C: FnMut(CssSyntaxNode),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let Some(syntax_node) = component_values_parse_as_syntax_with_source(
        &component_values,
        limit_single_component_ident_to_custom_ident,
        Some(filtered_input_string),
    ) else {
        callback(CssSyntaxNode::new(CssSyntaxNodeKind::Invalid));
        return;
    };

    emit_syntax_node(&syntax_node, &mut callback);
}

pub(crate) fn parse_syntax_component_prefix<C>(
    filtered_input: &[u8],
    limit_single_component_ident_to_custom_ident: bool,
    mut callback: C,
) -> usize
where
    C: FnMut(CssSyntaxNode),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let Some(syntax_node) = parse_syntax_component(
        &mut parser,
        limit_single_component_ident_to_custom_ident,
        Some(filtered_input_string),
    ) else {
        callback(CssSyntaxNode::new(CssSyntaxNodeKind::Invalid));
        return 0;
    };

    let Some(consumed_input) =
        serialize_component_values_for_reparsing(&parser.component_values[..parser.index], filtered_input_string)
    else {
        callback(CssSyntaxNode::new(CssSyntaxNodeKind::Invalid));
        return 0;
    };

    emit_syntax_node(&syntax_node, &mut callback);
    consumed_input.len()
}

pub(crate) fn parse_css_type_prefix<C>(
    filtered_input: &[u8],
    limit_single_component_ident_to_custom_ident: bool,
    mut callback: C,
) -> usize
where
    C: FnMut(CssSyntaxNode),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let Some(syntax_node) = parse_css_type(
        &mut parser,
        limit_single_component_ident_to_custom_ident,
        Some(filtered_input_string),
    ) else {
        callback(CssSyntaxNode::new(CssSyntaxNodeKind::Invalid));
        return 0;
    };

    let Some(consumed_input) =
        serialize_component_values_for_reparsing(&parser.component_values[..parser.index], filtered_input_string)
    else {
        callback(CssSyntaxNode::new(CssSyntaxNodeKind::Invalid));
        return 0;
    };

    emit_syntax_node(&syntax_node, &mut callback);
    consumed_input.len()
}

pub(crate) fn parse_a_supports_condition<E, C>(
    filtered_input: &[u8],
    mut event_callback: E,
    mut component_value_callback: C,
) where
    E: FnMut(CssBooleanExpressionEventKind),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    parser.rule_context.push(RuleContext::SupportsCondition);
    let component_values = parser.parse_a_list_of_component_values();
    parser.rule_context.pop();

    let mut parser = ComponentValueParser::new(component_values);
    if parser
        .parse_a_boolean_expression(BooleanExpressionTestKind::SupportsFeature)
        .is_none()
        || parser.has_next_component_value()
    {
        event_callback(CssBooleanExpressionEventKind::Invalid);
        return;
    }

    let boolean_expression = parser
        .boolean_expression
        .take()
        .expect("parsed expression must be present");
    emit_boolean_expression(
        &boolean_expression,
        filtered_input_string,
        &mut event_callback,
        &mut component_value_callback,
        &mut |_| {},
        &mut |_| {},
    );
}

pub(crate) fn parse_a_supports_feature<F>(filtered_input: &[u8], mut feature_callback: F) -> bool
where
    F: FnMut(CssSupportsFeatureKind, Option<&str>),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    parser.rule_context.push(RuleContext::SupportsCondition);
    let component_values = parser.parse_a_list_of_component_values();
    parser.rule_context.pop();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();
    let Some((feature, _)) = parser.parse_supports_feature_syntax() else {
        return false;
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    match feature {
        SupportsFeature::Declaration => feature_callback(CssSupportsFeatureKind::Declaration, None),
        SupportsFeature::Selector => feature_callback(CssSupportsFeatureKind::Selector, None),
        SupportsFeature::FontTech(name) => feature_callback(CssSupportsFeatureKind::FontTech, Some(&name)),
        SupportsFeature::FontFormat(name) => feature_callback(CssSupportsFeatureKind::FontFormat, Some(&name)),
        SupportsFeature::Env(name) => feature_callback(CssSupportsFeatureKind::Env, Some(&name)),
    }

    true
}

pub(crate) fn parse_an_if_condition<E, C>(filtered_input: &[u8], mut event_callback: E, mut component_value_callback: C)
where
    E: FnMut(CssBooleanExpressionEventKind),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    if parser
        .parse_a_boolean_expression(BooleanExpressionTestKind::IfTest)
        .is_none()
        || parser.has_next_component_value()
    {
        event_callback(CssBooleanExpressionEventKind::Invalid);
        return;
    }

    let boolean_expression = parser
        .boolean_expression
        .take()
        .expect("parsed expression must be present");
    emit_boolean_expression(
        &boolean_expression,
        filtered_input_string,
        &mut event_callback,
        &mut component_value_callback,
        &mut |_| {},
        &mut |_| {},
    );
}

pub(crate) fn parse_a_page_selector_list<S, P>(
    filtered_input: &[u8],
    mut selector_callback: S,
    mut pseudo_class_callback: P,
) -> bool
where
    S: FnMut(CssPageSelector),
    P: FnMut(CssPagePseudoClassKind),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(selectors) = parser.parse_a_page_selector_list() else {
        return false;
    };

    for selector in selectors {
        if let Some(name) = &selector.name {
            let (name_ptr, name_len) = string_parts(name);
            selector_callback(CssPageSelector {
                has_name: true,
                name_ptr,
                name_len,
            });
        } else {
            selector_callback(CssPageSelector {
                has_name: false,
                name_ptr: std::ptr::null(),
                name_len: 0,
            });
        }

        for pseudo_class in selector.pseudo_classes {
            pseudo_class_callback(pseudo_class);
        }
    }

    true
}

pub(crate) fn parse_a_selector_list<E, C>(
    filtered_input: &[u8],
    selector_type: SelectorType,
    parsing_mode: SelectorParsingMode,
    declared_namespaces: Vec<String>,
    mut event_callback: E,
    mut component_value_callback: C,
) -> bool
where
    E: FnMut(CssSelectorEvent),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::with_declared_namespaces(component_values, declared_namespaces);
    let Some(selectors) = parser.parse_a_selector_list(selector_type, parsing_mode) else {
        return false;
    };
    parser.discard_whitespace();
    if parser.next_component_value().is_some() {
        return false;
    }

    emit_selector_list(
        &selectors,
        filtered_input_string,
        &mut event_callback,
        &mut component_value_callback,
    );
    true
}

pub(crate) fn parse_a_keyframe_selector_list<S>(filtered_input: &[u8], mut selector_callback: S) -> bool
where
    S: FnMut(KeyframeSelector),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(selectors) = parser.parse_a_keyframe_selector_list() else {
        return false;
    };

    for selector in selectors {
        selector_callback(selector);
    }

    true
}

pub(crate) fn parse_a_keyframes_name<N>(filtered_input: &[u8], mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(name) = parser.parse_a_keyframes_name() else {
        return false;
    };

    name_callback(&name);
    true
}

pub(crate) fn parse_a_custom_property_name<N>(filtered_input: &[u8], mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(name) = parser.parse_a_custom_property_name() else {
        return false;
    };

    name_callback(&name);
    true
}

pub(crate) fn parse_a_custom_ident<N>(filtered_input: &[u8], mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(name) = parser.parse_a_custom_ident(&[]) else {
        return false;
    };

    name_callback(&name);
    true
}

pub(crate) fn parse_a_dashed_ident<N>(filtered_input: &[u8], mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(name) = parser.parse_a_dashed_ident() else {
        return false;
    };

    name_callback(&name);
    true
}

pub(crate) fn parse_a_unicode_range<R>(filtered_input: &[u8], mut range_callback: R) -> bool
where
    R: FnMut(CssUnicodeRange),
{
    let (mut parser, filtered_input) = parser_from_filtered_input(filtered_input);
    let Some(unicode_range) = parser.parse_a_unicode_range(filtered_input) else {
        return false;
    };

    range_callback(unicode_range);
    true
}

pub(crate) fn parse_a_unicode_range_list<R>(filtered_input: &[u8], mut range_callback: R) -> bool
where
    R: FnMut(CssUnicodeRange),
{
    let (mut parser, filtered_input) = parser_from_filtered_input(filtered_input);
    let Some(unicode_ranges) = parser.parse_a_unicode_range_list(filtered_input) else {
        return false;
    };

    for unicode_range in unicode_ranges {
        range_callback(unicode_range);
    }
    true
}

pub(crate) fn parse_a_url_function<U, M>(filtered_input: &[u8], mut url_callback: U, mut modifier_callback: M) -> bool
where
    U: FnMut(CssUrlFunction),
    M: FnMut(CssUrlModifier),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(url_function) = parser.parse_a_url_function() else {
        return false;
    };

    url_callback(CssUrlFunction {
        function_type: url_function.function_type,
        url_ptr: url_function.url.as_ptr(),
        url_len: url_function.url.len(),
    });
    for modifier in &url_function.request_url_modifiers {
        modifier_callback(modifier.as_ffi());
    }
    true
}

pub(crate) fn parse_an_import_url<U, M>(filtered_input: &[u8], mut url_callback: U, mut modifier_callback: M) -> bool
where
    U: FnMut(CssUrlFunction),
    M: FnMut(CssUrlModifier),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(url_function) = parser.parse_an_import_url() else {
        return false;
    };

    url_callback(CssUrlFunction {
        function_type: url_function.function_type,
        url_ptr: url_function.url.as_ptr(),
        url_len: url_function.url.len(),
    });
    for modifier in &url_function.request_url_modifiers {
        modifier_callback(modifier.as_ffi());
    }
    true
}

pub(crate) fn parse_import_rule_prelude<U, M, L, S, Q>(
    filtered_input: &[u8],
    mut url_callback: U,
    mut modifier_callback: M,
    mut layer_callback: L,
    mut supports_callback: S,
    mut media_query_list_callback: Q,
) -> bool
where
    U: FnMut(CssUrlFunction),
    M: FnMut(CssUrlModifier),
    L: FnMut(&str),
    S: FnMut(&str),
    Q: FnMut(&str),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);

    // https://drafts.csswg.org/css-cascade-5/#at-import
    // @import [ <url> | <string> ]
    //         [ layer | layer(<layer-name>) ]?
    //         <import-conditions> ;
    //
    // <import-conditions> = [ supports( [ <supports-condition> | <declaration> ] ) ]?
    //                      <media-query-list>?
    let Some(url_function) = parser.parse_an_import_url_prefix() else {
        return false;
    };

    url_callback(CssUrlFunction {
        function_type: url_function.function_type,
        url_ptr: url_function.url.as_ptr(),
        url_len: url_function.url.len(),
    });
    for modifier in &url_function.request_url_modifiers {
        modifier_callback(modifier.as_ffi());
    }

    // [ layer | layer(<layer-name>) ]?
    parser.discard_whitespace();
    let saved_index = parser.index;
    if let Some(layer_name) = parser.parse_an_import_layer_prefix() {
        layer_callback(&layer_name);
    } else {
        parser.index = saved_index;
    }

    // <import-conditions> = [ supports( [ <supports-condition> | <declaration> ] ) ]?
    //                      <media-query-list>?
    parser.discard_whitespace();
    if let Some(ComponentValue::Function(function)) = parser.next_component_value()
        && function.name.eq_ignore_ascii_case("supports")
    {
        let Some(serialized_supports) =
            serialize_component_values_for_reparsing(&function.value, filtered_input_string)
        else {
            return false;
        };
        parser.index += 1;
        supports_callback(&serialized_supports);
    }

    let Some(serialized_media_query_list) =
        serialize_component_values_for_reparsing(parser.remaining_component_values(), filtered_input_string)
    else {
        return false;
    };
    media_query_list_callback(&serialized_media_query_list);
    true
}

pub(crate) fn parse_a_font_source<S, U, M, F, T>(
    filtered_input: &[u8],
    mut source_callback: S,
    mut url_callback: U,
    mut modifier_callback: M,
    mut format_callback: F,
    mut tech_callback: T,
) -> bool
where
    S: FnMut(CssFontSourceKind, Option<&FamilyName>),
    U: FnMut(CssUrlFunction),
    M: FnMut(CssUrlModifier),
    F: FnMut(&str),
    T: FnMut(CssFontTech),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(font_source) = parser.parse_a_font_source() else {
        return false;
    };

    match &font_source {
        FontSource::Local(family_name) => {
            source_callback(CssFontSourceKind::Local, Some(family_name));
        }
        FontSource::Url {
            url_function,
            format,
            tech,
        } => {
            source_callback(CssFontSourceKind::Url, None);
            url_callback(CssUrlFunction {
                function_type: url_function.function_type,
                url_ptr: url_function.url.as_ptr(),
                url_len: url_function.url.len(),
            });
            for modifier in &url_function.request_url_modifiers {
                modifier_callback(modifier.as_ffi());
            }
            if let Some(format) = format {
                format_callback(format);
            }
            for tech in tech {
                tech_callback(*tech);
            }
        }
    }

    true
}

pub(crate) fn parse_a_font_language_override<F>(filtered_input: &[u8], mut font_language_override_callback: F) -> bool
where
    F: FnMut(CssFontLanguageOverrideKind, Option<&str>),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(font_language_override) = parser.parse_a_font_language_override() else {
        return false;
    };

    match &font_language_override {
        FontLanguageOverride::Normal => {
            font_language_override_callback(CssFontLanguageOverrideKind::Normal, None);
        }
        FontLanguageOverride::String(value) => {
            font_language_override_callback(CssFontLanguageOverrideKind::String, Some(value));
        }
    }

    true
}

pub(crate) fn parse_an_opentype_tag<F>(filtered_input: &[u8], mut opentype_tag_callback: F) -> bool
where
    F: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();
    let Some(opentype_tag) = parse_opentype_tag(&mut parser) else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    opentype_tag_callback(&opentype_tag);
    true
}

pub(crate) fn parse_a_font_feature_settings<K, V>(
    filtered_input: &[u8],
    mut settings_callback: K,
    mut tagged_value_callback: V,
) -> bool
where
    K: FnMut(CssOpenTypeSettingsKind),
    V: FnMut(&OpenTypeTaggedValue),
{
    let (mut parser, filtered_input) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(font_feature_settings) = parser.parse_a_font_feature_settings(filtered_input) else {
        return false;
    };

    emit_open_type_settings(
        font_feature_settings,
        &mut settings_callback,
        &mut tagged_value_callback,
    );
    true
}

pub(crate) fn parse_a_font_variation_settings<K, V>(
    filtered_input: &[u8],
    mut settings_callback: K,
    mut tagged_value_callback: V,
) -> bool
where
    K: FnMut(CssOpenTypeSettingsKind),
    V: FnMut(&OpenTypeTaggedValue),
{
    let (mut parser, filtered_input) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(font_variation_settings) = parser.parse_a_font_variation_settings(filtered_input) else {
        return false;
    };

    emit_open_type_settings(
        font_variation_settings,
        &mut settings_callback,
        &mut tagged_value_callback,
    );
    true
}

pub(crate) fn parse_a_font_style<F>(filtered_input: &[u8], mut font_style_callback: F) -> bool
where
    F: FnMut(FontStyle),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(font_style) = parser.parse_a_font_style() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    font_style_callback(font_style);
    true
}

pub(crate) fn parse_a_font_variant_east_asian<V>(filtered_input: &[u8], mut value_callback: V) -> bool
where
    V: FnMut(&FontVariantEastAsianValue),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(values) = parser.parse_a_font_variant_east_asian() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    for value in &values {
        value_callback(value);
    }
    true
}

pub(crate) fn parse_a_font_variant_numeric<V>(filtered_input: &[u8], mut value_callback: V) -> bool
where
    V: FnMut(&FontVariantNumericValue),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(values) = parser.parse_a_font_variant_numeric() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    for value in &values {
        value_callback(value);
    }
    true
}

pub(crate) fn parse_a_font_variant_ligatures<V>(filtered_input: &[u8], mut value_callback: V) -> bool
where
    V: FnMut(&FontVariantLigaturesValue),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(values) = parser.parse_a_font_variant_ligatures() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    for value in &values {
        value_callback(value);
    }
    true
}

pub(crate) fn parse_a_font_variant_alternates<V, N>(
    filtered_input: &[u8],
    mut value_callback: V,
    mut feature_value_name_callback: N,
) -> bool
where
    V: FnMut(CssFontVariantAlternatesValueKind),
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(values) = parser.parse_a_font_variant_alternates() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    for value in values {
        value_callback(value.kind);
        for feature_value_name in &value.feature_value_names {
            feature_value_name_callback(feature_value_name);
        }
    }
    true
}

pub(crate) fn parse_a_font_variant<S, A, N, E, U, L>(
    filtered_input: &[u8],
    mut simple_value_callback: S,
    mut alternates_value_callback: A,
    mut alternates_feature_value_name_callback: N,
    mut east_asian_value_callback: E,
    mut numeric_value_callback: U,
    mut ligatures_value_callback: L,
) -> bool
where
    S: FnMut(CssFontVariantSimpleValueKind, Option<&str>),
    A: FnMut(CssFontVariantAlternatesValueKind),
    N: FnMut(&str),
    E: FnMut(&FontVariantEastAsianValue),
    U: FnMut(&FontVariantNumericValue),
    L: FnMut(&FontVariantLigaturesValue),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(font_variant) = parser.parse_a_font_variant() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    if font_variant.ligatures_none {
        simple_value_callback(CssFontVariantSimpleValueKind::LigaturesNone, None);
    }
    if let Some(alternates) = font_variant.alternates {
        for value in alternates {
            alternates_value_callback(value.kind);
            for feature_value_name in &value.feature_value_names {
                alternates_feature_value_name_callback(feature_value_name);
            }
        }
    }
    if let Some(caps) = &font_variant.caps {
        simple_value_callback(CssFontVariantSimpleValueKind::Caps, Some(caps));
    }
    if let Some(east_asian) = &font_variant.east_asian {
        for value in east_asian {
            east_asian_value_callback(value);
        }
    }
    if let Some(emoji) = &font_variant.emoji {
        simple_value_callback(CssFontVariantSimpleValueKind::Emoji, Some(emoji));
    }
    if let Some(ligatures) = &font_variant.ligatures {
        for value in ligatures {
            ligatures_value_callback(value);
        }
    }
    if let Some(numeric) = &font_variant.numeric {
        for value in numeric {
            numeric_value_callback(value);
        }
    }
    if let Some(position) = &font_variant.position {
        simple_value_callback(CssFontVariantSimpleValueKind::Position, Some(position));
    }
    true
}

pub(crate) fn parse_a_font_family_value<F>(filtered_input: &[u8], mut family_callback: F) -> bool
where
    F: FnMut(&FontFamilyValue),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let groups = parser.parse_a_comma_separated_list_of_component_values();
    if groups.is_empty() {
        return false;
    }

    let mut family_values = Vec::with_capacity(groups.len());
    for group in groups {
        let mut parser = ComponentValueParser::new(group);
        let Some(family_value) = parser.parse_a_font_family_item() else {
            return false;
        };
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return false;
        }
        family_values.push(family_value);
    }

    for family_value in &family_values {
        family_callback(family_value);
    }
    true
}

pub(super) fn emit_open_type_settings<K, V>(
    open_type_settings: OpenTypeSettings,
    settings_callback: &mut K,
    tagged_value_callback: &mut V,
) where
    K: FnMut(CssOpenTypeSettingsKind),
    V: FnMut(&OpenTypeTaggedValue),
{
    match &open_type_settings {
        OpenTypeSettings::Normal => {
            settings_callback(CssOpenTypeSettingsKind::Normal);
        }
        OpenTypeSettings::TagValues(tag_values) => {
            settings_callback(CssOpenTypeSettingsKind::TagValues);
            for tag_value in tag_values {
                tagged_value_callback(tag_value);
            }
        }
    }
}

pub(crate) fn parse_a_layer_name<N>(filtered_input: &[u8], allow_blank_layer_name: bool, mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(name) = parser.parse_a_layer_name(allow_blank_layer_name) else {
        return false;
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    name_callback(&name);
    true
}

pub(crate) fn parse_an_import_layer<N>(filtered_input: &[u8], mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(name) = parser.parse_an_import_layer() else {
        return false;
    };

    name_callback(&name);
    true
}

pub(crate) fn parse_a_layer_name_list<N>(filtered_input: &[u8], mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(names) = parser.parse_a_layer_name_list() else {
        return false;
    };

    for name in names {
        name_callback(&name);
    }
    true
}

pub(crate) fn parse_container_type_value(filtered_input: &[u8]) -> CssContainerTypeValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-conditional-5/#propdef-container-type
    // normal | [ [ size | inline-size ] || scroll-state ]
    if parser.consume_ident_matching("normal") {
        if parser.has_next_component_value() {
            return CssContainerTypeValueKind::Invalid;
        }
        return CssContainerTypeValueKind::Normal;
    }

    let mut size_value = None;
    let mut has_scroll_state = false;
    while parser.has_next_component_value() {
        let Some(value) = parser.consume_an_ident() else {
            return CssContainerTypeValueKind::Invalid;
        };

        if value.eq_ignore_ascii_case("size") {
            if size_value.is_some() {
                return CssContainerTypeValueKind::Invalid;
            }
            size_value = Some(CssContainerTypeValueKind::Size);
        } else if value.eq_ignore_ascii_case("inline-size") {
            if size_value.is_some() {
                return CssContainerTypeValueKind::Invalid;
            }
            size_value = Some(CssContainerTypeValueKind::InlineSize);
        } else if value.eq_ignore_ascii_case("scroll-state") {
            if has_scroll_state {
                return CssContainerTypeValueKind::Invalid;
            }
            has_scroll_state = true;
        } else {
            return CssContainerTypeValueKind::Invalid;
        }
    }

    match (size_value, has_scroll_state) {
        (Some(CssContainerTypeValueKind::Size), false) => CssContainerTypeValueKind::Size,
        (Some(CssContainerTypeValueKind::InlineSize), false) => CssContainerTypeValueKind::InlineSize,
        (None, true) => CssContainerTypeValueKind::ScrollState,
        (Some(CssContainerTypeValueKind::Size), true) => CssContainerTypeValueKind::SizeAndScrollState,
        (Some(CssContainerTypeValueKind::InlineSize), true) => CssContainerTypeValueKind::InlineSizeAndScrollState,
        _ => CssContainerTypeValueKind::Invalid,
    }
}

pub(crate) fn parse_contain_value(filtered_input: &[u8]) -> CssContainValue {
    let invalid = CssContainValue {
        kind: CssContainValueKind::Invalid,
        is_size: false,
        is_inline_size: false,
        has_layout: false,
        has_style: false,
        has_paint: false,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-contain-2/#contain-property
    // none | strict | content | [ [size | inline-size] || layout || style || paint ]
    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssContainValue {
            kind: CssContainValueKind::None,
            ..invalid
        };
    }
    if parser.consume_ident_matching("strict") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssContainValue {
            kind: CssContainValueKind::Strict,
            ..invalid
        };
    }
    if parser.consume_ident_matching("content") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssContainValue {
            kind: CssContainValueKind::Content,
            ..invalid
        };
    }

    let mut value = CssContainValue {
        kind: CssContainValueKind::List,
        ..invalid
    };
    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        if ident.eq_ignore_ascii_case("size") {
            if value.is_size || value.is_inline_size {
                return invalid;
            }
            value.is_size = true;
        } else if ident.eq_ignore_ascii_case("inline-size") {
            if value.is_size || value.is_inline_size {
                return invalid;
            }
            value.is_inline_size = true;
        } else if ident.eq_ignore_ascii_case("layout") {
            if value.has_layout {
                return invalid;
            }
            value.has_layout = true;
        } else if ident.eq_ignore_ascii_case("style") {
            if value.has_style {
                return invalid;
            }
            value.has_style = true;
        } else if ident.eq_ignore_ascii_case("paint") {
            if value.has_paint {
                return invalid;
            }
            value.has_paint = true;
        } else {
            return invalid;
        }
    }

    if !value.is_size && !value.is_inline_size && !value.has_layout && !value.has_style && !value.has_paint {
        return invalid;
    }

    value
}

pub(crate) fn parse_white_space_trim_value(filtered_input: &[u8]) -> CssWhiteSpaceTrimValue {
    let invalid = CssWhiteSpaceTrimValue {
        kind: CssWhiteSpaceTrimValueKind::Invalid,
        has_discard_before: false,
        has_discard_after: false,
        has_discard_inner: false,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-text-4/#white-space-trim
    // none | discard-before || discard-after || discard-inner
    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssWhiteSpaceTrimValue {
            kind: CssWhiteSpaceTrimValueKind::None,
            ..invalid
        };
    }

    let mut value = CssWhiteSpaceTrimValue {
        kind: CssWhiteSpaceTrimValueKind::List,
        ..invalid
    };
    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        if ident.eq_ignore_ascii_case("discard-before") {
            if value.has_discard_before {
                return invalid;
            }
            value.has_discard_before = true;
        } else if ident.eq_ignore_ascii_case("discard-after") {
            if value.has_discard_after {
                return invalid;
            }
            value.has_discard_after = true;
        } else if ident.eq_ignore_ascii_case("discard-inner") {
            if value.has_discard_inner {
                return invalid;
            }
            value.has_discard_inner = true;
        } else {
            return invalid;
        }
    }

    if !value.has_discard_before && !value.has_discard_after && !value.has_discard_inner {
        return invalid;
    }

    value
}

pub(crate) fn parse_color_scheme_value<S>(filtered_input: &[u8], mut scheme_callback: S) -> CssColorSchemeValue
where
    S: FnMut(&str),
{
    let invalid = CssColorSchemeValue {
        kind: CssColorSchemeValueKind::Invalid,
        only: false,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-color-adjust-1/#color-scheme-prop
    // normal | [ light | dark | <custom-ident> ]+ && only?
    if parser.consume_ident_matching("normal") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssColorSchemeValue {
            kind: CssColorSchemeValueKind::Normal,
            only: false,
        };
    }

    let mut only = false;
    if parser.consume_ident_matching("only") {
        only = true;
    }

    let mut schemes = Vec::new();
    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        if ident.eq_ignore_ascii_case("only") {
            if only {
                return invalid;
            }
            only = true;
            break;
        }

        if !ident.eq_ignore_ascii_case("light")
            && !ident.eq_ignore_ascii_case("dark")
            && !is_valid_custom_ident(&ident, &["normal", "light", "dark", "only"])
        {
            return invalid;
        }

        schemes.push(ident);
    }

    if parser.has_next_component_value() || schemes.is_empty() {
        return invalid;
    }

    for scheme in schemes {
        scheme_callback(&scheme);
    }

    CssColorSchemeValue {
        kind: CssColorSchemeValueKind::List,
        only,
    }
}

// https://drafts.csswg.org/css-display-3/#the-display-properties
pub(crate) fn parse_display_value(filtered_input: &[u8]) -> Option<RustOwnedDisplay> {
    fn display_box_from_ident(ident: &str) -> Option<CssDisplayBox> {
        if ident.eq_ignore_ascii_case("contents") {
            Some(CssDisplayBox::Contents)
        } else if ident.eq_ignore_ascii_case("none") {
            Some(CssDisplayBox::None)
        } else {
            None
        }
    }

    fn display_inside_from_ident(ident: &str) -> Option<CssDisplayInside> {
        if ident.eq_ignore_ascii_case("flow") {
            Some(CssDisplayInside::Flow)
        } else if ident.eq_ignore_ascii_case("flow-root") {
            Some(CssDisplayInside::FlowRoot)
        } else if ident.eq_ignore_ascii_case("table") {
            Some(CssDisplayInside::Table)
        } else if ident.eq_ignore_ascii_case("flex") {
            Some(CssDisplayInside::Flex)
        } else if ident.eq_ignore_ascii_case("grid") {
            Some(CssDisplayInside::Grid)
        } else if ident.eq_ignore_ascii_case("ruby") {
            Some(CssDisplayInside::Ruby)
        } else if ident.eq_ignore_ascii_case("math") {
            Some(CssDisplayInside::Math)
        } else {
            None
        }
    }

    fn display_internal_from_ident(ident: &str) -> Option<CssDisplayInternal> {
        if ident.eq_ignore_ascii_case("table-row-group") {
            Some(CssDisplayInternal::TableRowGroup)
        } else if ident.eq_ignore_ascii_case("table-header-group") {
            Some(CssDisplayInternal::TableHeaderGroup)
        } else if ident.eq_ignore_ascii_case("table-footer-group") {
            Some(CssDisplayInternal::TableFooterGroup)
        } else if ident.eq_ignore_ascii_case("table-row") {
            Some(CssDisplayInternal::TableRow)
        } else if ident.eq_ignore_ascii_case("table-cell") {
            Some(CssDisplayInternal::TableCell)
        } else if ident.eq_ignore_ascii_case("table-column-group") {
            Some(CssDisplayInternal::TableColumnGroup)
        } else if ident.eq_ignore_ascii_case("table-column") {
            Some(CssDisplayInternal::TableColumn)
        } else if ident.eq_ignore_ascii_case("table-caption") {
            Some(CssDisplayInternal::TableCaption)
        } else if ident.eq_ignore_ascii_case("ruby-base") {
            Some(CssDisplayInternal::RubyBase)
        } else if ident.eq_ignore_ascii_case("ruby-text") {
            Some(CssDisplayInternal::RubyText)
        } else if ident.eq_ignore_ascii_case("ruby-base-container") {
            Some(CssDisplayInternal::RubyBaseContainer)
        } else if ident.eq_ignore_ascii_case("ruby-text-container") {
            Some(CssDisplayInternal::RubyTextContainer)
        } else {
            None
        }
    }

    fn display_outside_from_ident(ident: &str) -> Option<CssDisplayOutside> {
        if ident.eq_ignore_ascii_case("block") {
            Some(CssDisplayOutside::Block)
        } else if ident.eq_ignore_ascii_case("inline") {
            Some(CssDisplayOutside::Inline)
        } else if ident.eq_ignore_ascii_case("run-in") {
            Some(CssDisplayOutside::RunIn)
        } else {
            None
        }
    }

    fn display_outside_and_inside(
        outside: CssDisplayOutside,
        inside: CssDisplayInside,
        list_item: CssDisplayListItem,
    ) -> RustOwnedDisplay {
        RustOwnedDisplay {
            kind: CssDisplayValueKind::OutsideAndInside,
            box_: CssDisplayBox::Contents,
            internal: CssDisplayInternal::TableRowGroup,
            outside,
            inside,
            list_item,
        }
    }

    fn display_box(box_: CssDisplayBox) -> RustOwnedDisplay {
        RustOwnedDisplay {
            kind: CssDisplayValueKind::Box,
            box_,
            internal: CssDisplayInternal::TableRowGroup,
            outside: CssDisplayOutside::Block,
            inside: CssDisplayInside::Flow,
            list_item: CssDisplayListItem::No,
        }
    }

    fn display_internal(internal: CssDisplayInternal) -> RustOwnedDisplay {
        RustOwnedDisplay {
            kind: CssDisplayValueKind::Internal,
            box_: CssDisplayBox::Contents,
            internal,
            outside: CssDisplayOutside::Block,
            inside: CssDisplayInside::Flow,
            list_item: CssDisplayListItem::No,
        }
    }

    fn parse_single_component_display(ident: &str) -> Option<RustOwnedDisplay> {
        // https://drafts.csswg.org/css-display-3/#typedef-display-legacy
        // <display-legacy> = inline-block | inline-table | inline-flex | inline-grid
        if ident.eq_ignore_ascii_case("inline-block") {
            return Some(display_outside_and_inside(
                CssDisplayOutside::Inline,
                CssDisplayInside::FlowRoot,
                CssDisplayListItem::No,
            ));
        }
        if ident.eq_ignore_ascii_case("inline-table") {
            return Some(display_outside_and_inside(
                CssDisplayOutside::Inline,
                CssDisplayInside::Table,
                CssDisplayListItem::No,
            ));
        }
        if ident.eq_ignore_ascii_case("inline-flex") {
            return Some(display_outside_and_inside(
                CssDisplayOutside::Inline,
                CssDisplayInside::Flex,
                CssDisplayListItem::No,
            ));
        }
        if ident.eq_ignore_ascii_case("inline-grid") {
            return Some(display_outside_and_inside(
                CssDisplayOutside::Inline,
                CssDisplayInside::Grid,
                CssDisplayListItem::No,
            ));
        }

        // https://drafts.csswg.org/css-display-3/#typedef-display-listitem
        // <display-listitem> = <display-outside>? && [ flow | flow-root ]? && list-item
        if ident.eq_ignore_ascii_case("list-item") {
            return Some(display_outside_and_inside(
                CssDisplayOutside::Block,
                CssDisplayInside::Flow,
                CssDisplayListItem::Yes,
            ));
        }

        if let Some(outside) = display_outside_from_ident(ident) {
            return Some(display_outside_and_inside(
                outside,
                CssDisplayInside::Flow,
                CssDisplayListItem::No,
            ));
        }

        if let Some(inside) = display_inside_from_ident(ident) {
            // NOTE: The MathML Core specification does not mention what the outside value for `display: math`
            //       should be, but other browsers use `inline`.
            //       https://w3c.github.io/mathml-core/#new-display-math-value
            let outside = if inside == CssDisplayInside::Ruby || inside == CssDisplayInside::Math {
                CssDisplayOutside::Inline
            } else {
                CssDisplayOutside::Block
            };
            return Some(display_outside_and_inside(outside, inside, CssDisplayListItem::No));
        }

        if let Some(internal) = display_internal_from_ident(ident) {
            return Some(display_internal(internal));
        }

        if let Some(box_) = display_box_from_ident(ident) {
            return Some(display_box(box_));
        }

        None
    }

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut idents = Vec::new();
    for component_value in &component_values {
        match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) => idents.push(value.as_str()),
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Whitespace,
                ..
            }) => {}
            _ => return None,
        }
    }

    if idents.is_empty() {
        return None;
    }

    if idents.len() == 1 {
        return parse_single_component_display(idents[0]);
    }

    let mut list_item = CssDisplayListItem::No;
    let mut inside = None;
    let mut outside = None;

    for ident in idents {
        if ident.eq_ignore_ascii_case("list-item") {
            if list_item == CssDisplayListItem::Yes {
                return None;
            }
            list_item = CssDisplayListItem::Yes;
            continue;
        }

        if let Some(inside_value) = display_inside_from_ident(ident) {
            if inside.is_some() {
                return None;
            }
            inside = Some(inside_value);
            continue;
        }

        if let Some(outside_value) = display_outside_from_ident(ident) {
            if outside.is_some() {
                return None;
            }
            outside = Some(outside_value);
            continue;
        }

        return None;
    }

    // https://drafts.csswg.org/css-display-3/#typedef-display-listitem
    // <display-listitem> = <display-outside>? && [ flow | flow-root ]? && list-item
    if list_item == CssDisplayListItem::Yes
        && let Some(inside) = inside
        && !matches!(inside, CssDisplayInside::Flow | CssDisplayInside::FlowRoot)
    {
        return None;
    }

    Some(display_outside_and_inside(
        outside.unwrap_or(CssDisplayOutside::Block),
        inside.unwrap_or(CssDisplayInside::Flow),
        list_item,
    ))
}

pub(crate) fn parse_scroll_function_value(filtered_input: &[u8]) -> CssScrollFunctionValue {
    let invalid = CssScrollFunctionValue {
        kind: CssScrollFunctionValueKind::Invalid,
        scroller: CssScrollFunctionScrollerKind::None,
        axis: CssScrollFunctionAxisKind::None,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let [ComponentValue::Function(function)] = component_values else {
        return invalid;
    };
    if !function.name.eq_ignore_ascii_case("scroll") {
        return invalid;
    }

    // https://drafts.csswg.org/scroll-animations-1/#funcdef-scroll
    // <scroll()> = scroll( [ <scroller> || <axis> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut scroller = CssScrollFunctionScrollerKind::None;
    let mut axis = CssScrollFunctionAxisKind::None;

    while parser.has_next_component_value() {
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            break;
        }

        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        if let Some(parsed_scroller) = scroll_function_scroller_from_string(&ident) {
            if scroller != CssScrollFunctionScrollerKind::None {
                return invalid;
            }
            scroller = parsed_scroller;
            continue;
        }

        if let Some(parsed_axis) = scroll_function_axis_from_string(&ident) {
            if axis != CssScrollFunctionAxisKind::None {
                return invalid;
            }
            axis = parsed_axis;
            continue;
        }

        return invalid;
    }

    CssScrollFunctionValue {
        kind: CssScrollFunctionValueKind::Valid,
        scroller,
        axis,
    }
}

pub(crate) fn parse_view_timeline_inset_value(filtered_input: &[u8]) -> CssViewTimelineInsetValue {
    let invalid = CssViewTimelineInsetValue {
        kind: CssViewTimelineInsetValueKind::Invalid,
        count: 0,
    };
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/scroll-animations-1/#view-timeline-inset
    // [ [ auto | <length-percentage> ]{1,2} ]
    let mut parser = ComponentValueParser::new(component_values);
    let Some(inset) = parse_view_timeline_inset_prefix(&mut parser, None) else {
        return invalid;
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return invalid;
    }

    CssViewTimelineInsetValue {
        kind: CssViewTimelineInsetValueKind::Valid,
        count: inset.count,
    }
}

pub(crate) fn parse_view_timeline_inset_value_prefix(filtered_input: &[u8]) -> CssViewTimelineInsetValue {
    let invalid = CssViewTimelineInsetValue {
        kind: CssViewTimelineInsetValueKind::Invalid,
        count: 0,
    };
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/scroll-animations-1/#view-timeline-inset
    // [ [ auto | <length-percentage> ]{1,2} ]
    let mut parser = ComponentValueParser::new(component_values);
    let Some(inset) = parse_view_timeline_inset_prefix(&mut parser, None) else {
        return invalid;
    };

    CssViewTimelineInsetValue {
        kind: CssViewTimelineInsetValueKind::Valid,
        count: inset.count,
    }
}

pub(super) fn parse_rust_owned_view_timeline_inset_value(
    filtered_input: &[u8],
) -> Option<Vec<Vec<RustOwnedNestedPrimitiveValue>>> {
    let filtered_input_string = filtered_input_to_string(filtered_input);

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let groups = split_component_values_on_comma(&component_values);
    let mut insets = Vec::with_capacity(groups.len());

    // https://drafts.csswg.org/scroll-animations-1/#view-timeline-inset
    // Value: [ [ auto | <length-percentage> ]{1,2} ]#
    for group in groups {
        let mut parser = ComponentValueParser::new(group.to_vec());
        let inset = parse_view_timeline_inset_prefix(&mut parser, Some(&filtered_input_string))?;
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }
        insets.push(inset.values);
    }

    Some(insets)
}

pub(super) fn parse_rust_owned_view_timeline_inset_value_prefix(
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedNestedPrimitiveValue>> {
    parse_rust_owned_view_timeline_inset_value_prefix_impl(filtered_input).map(|(values, _)| values)
}

pub(super) fn parse_rust_owned_view_timeline_inset_value_prefix_impl(
    filtered_input: &[u8],
) -> Option<(Vec<RustOwnedNestedPrimitiveValue>, bool)> {
    let filtered_input_string = filtered_input_to_string(filtered_input);

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/scroll-animations-1/#view-timeline-inset
    // [ [ auto | <length-percentage> ]{1,2} ]
    let mut parser = ComponentValueParser::new(component_values);
    let inset = parse_view_timeline_inset_prefix(&mut parser, Some(&filtered_input_string))?;

    parser.discard_whitespace();
    Some((inset.values, parser.has_next_component_value()))
}

pub(crate) fn parse_view_function_value(filtered_input: &[u8]) -> CssViewFunctionValue {
    let invalid = CssViewFunctionValue {
        kind: CssViewFunctionValueKind::Invalid,
        axis: CssScrollFunctionAxisKind::None,
        inset: CssViewFunctionInsetKind::None,
        inset_position: CssViewFunctionInsetPosition::None,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let [ComponentValue::Function(function)] = component_values else {
        return invalid;
    };
    if !function.name.eq_ignore_ascii_case("view") {
        return invalid;
    }

    // https://drafts.csswg.org/scroll-animations-1/#funcdef-view
    // <view()> = view( [ <axis> || <'view-timeline-inset'> ]? )
    if strip_whitespace(&function.value).is_empty() {
        return CssViewFunctionValue {
            kind: CssViewFunctionValueKind::Valid,
            axis: CssScrollFunctionAxisKind::None,
            inset: CssViewFunctionInsetKind::None,
            inset_position: CssViewFunctionInsetPosition::None,
        };
    }

    if let Some(parsed) = parse_view_function_value_with_axis_first(function.value.clone()) {
        return parsed;
    }

    if let Some(parsed) = parse_view_function_value_with_inset_first(function.value.clone()) {
        return parsed;
    }

    invalid
}

pub(crate) fn parse_rect_value(filtered_input: &[u8]) -> CssRectValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let [ComponentValue::Function(function)] = component_values else {
        return CssRectValueKind::Invalid;
    };
    if !function.name.eq_ignore_ascii_case("rect") {
        return CssRectValueKind::Invalid;
    }

    // https://www.w3.org/TR/CSS2/visufx.html#value-def-shape
    // In CSS 2.1, the only valid <shape> value is:
    // rect(<top>, <right>, <bottom>, <left>)
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut requires_commas = None;

    for side in 0..4 {
        if !parse_rect_side(&mut parser) {
            return CssRectValueKind::Invalid;
        }

        parser.discard_whitespace();

        if side == 3 {
            return if parser.has_next_component_value() {
                CssRectValueKind::Invalid
            } else {
                CssRectValueKind::Valid
            };
        }

        let next_is_comma = parser.consume_a_comma();
        match requires_commas {
            Some(true) if !next_is_comma => return CssRectValueKind::Invalid,
            Some(false) if next_is_comma => return CssRectValueKind::Invalid,
            None => requires_commas = Some(next_is_comma),
            _ => {}
        }
    }

    CssRectValueKind::Invalid
}

pub(crate) fn parse_ratio_value_prefix(filtered_input: &[u8]) -> CssRatioValue {
    let invalid = CssRatioValue {
        kind: CssRatioValueKind::Invalid,
        has_denominator: false,
        numerator: 0.0,
        denominator: 0.0,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    // https://drafts.csswg.org/css-values-4/#ratios
    // <ratio> = <number [0,∞]> [ / <number [0,∞]> ]?
    let Some(numerator) = parse_non_negative_number_prefix_value(&mut parser) else {
        return invalid;
    };

    parser.discard_whitespace();
    if !parser.consume_a_delim('/') {
        return CssRatioValue {
            kind: CssRatioValueKind::Valid,
            has_denominator: false,
            numerator,
            denominator: 1.0,
        };
    }

    let Some(denominator) = parse_non_negative_number_prefix_value(&mut parser) else {
        return invalid;
    };

    CssRatioValue {
        kind: CssRatioValueKind::Valid,
        has_denominator: true,
        numerator,
        denominator,
    }
}

pub(crate) fn parse_primitive_value_prefix(
    filtered_input: &[u8],
    value_type: CssPrimitiveValueType,
    options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    let Some(component_value) = parser.next_component_value() else {
        return CssPrimitiveValueKind::Invalid;
    };

    match value_type {
        CssPrimitiveValueType::Integer => parse_integer_value_prefix(component_value),
        CssPrimitiveValueType::Number => parse_number_value_prefix(component_value),
        CssPrimitiveValueType::Percentage => parse_percentage_value_prefix(component_value),
        CssPrimitiveValueType::Angle => parse_angle_value_prefix(component_value, options),
        CssPrimitiveValueType::Flex => parse_flex_value_prefix(component_value),
        CssPrimitiveValueType::Frequency => parse_frequency_value_prefix(component_value),
        CssPrimitiveValueType::Length => parse_length_value_prefix(component_value, options),
        CssPrimitiveValueType::Resolution => parse_resolution_value_prefix(component_value),
        CssPrimitiveValueType::String => parse_string_value_prefix(component_value),
        CssPrimitiveValueType::Time => parse_time_value_prefix(component_value),
        CssPrimitiveValueType::Opacity => parse_opacity_value_prefix(component_value),
    }
}

pub(crate) fn parse_primitive_value(
    filtered_input: &[u8],
    value_type: CssPrimitiveValueType,
    options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    let [component_value] = component_values else {
        return CssPrimitiveValueKind::Invalid;
    };

    match value_type {
        CssPrimitiveValueType::Integer => parse_integer_value_prefix(component_value),
        CssPrimitiveValueType::Number => parse_number_value_prefix(component_value),
        CssPrimitiveValueType::Percentage => parse_percentage_value_prefix(component_value),
        CssPrimitiveValueType::Angle => parse_angle_value_prefix(component_value, options),
        CssPrimitiveValueType::Flex => parse_flex_value_prefix(component_value),
        CssPrimitiveValueType::Frequency => parse_frequency_value_prefix(component_value),
        CssPrimitiveValueType::Length => parse_length_value_prefix(component_value, options),
        CssPrimitiveValueType::Resolution => parse_resolution_value_prefix(component_value),
        CssPrimitiveValueType::String => parse_string_value_prefix(component_value),
        CssPrimitiveValueType::Time => parse_time_value_prefix(component_value),
        CssPrimitiveValueType::Opacity => parse_opacity_value_prefix(component_value),
    }
}

fn function_can_represent_number(function: &Function) -> bool {
    is_math_function_name(&function.name)
        || function.name.eq_ignore_ascii_case("random")
        || function.name.eq_ignore_ascii_case("sibling-index")
        || function.name.eq_ignore_ascii_case("sibling-count")
}

fn function_can_represent_dimension(function: &Function) -> bool {
    is_math_function_name(&function.name) || function.name.eq_ignore_ascii_case("random")
}

fn function_can_represent_length(function: &Function) -> bool {
    function_can_represent_dimension(function) || function.name.eq_ignore_ascii_case("anchor-size")
}

pub(super) fn parse_integer_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#integers
    // <integer> = [-+]? [0-9]+
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if number_is_integer(*number) => CssPrimitiveValueKind::Integer,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math and tree-counting functions still happens in C++.
        ComponentValue::Function(function) if function_can_represent_number(function) => CssPrimitiveValueKind::Integer,
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_number_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#numbers
    // <number> = <integer> | <number-token>
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { .. },
            ..
        }) => CssPrimitiveValueKind::Number,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math and tree-counting functions still happens in C++.
        ComponentValue::Function(function) if function_can_represent_number(function) => CssPrimitiveValueKind::Number,
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_percentage_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#percentages
    // <percentage> = <number-token>%
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { .. },
            ..
        }) => CssPrimitiveValueKind::Percentage,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) if function_can_represent_dimension(function) => {
            CssPrimitiveValueKind::Percentage
        }
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_angle_value_prefix(
    component_value: &ComponentValue,
    options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#angles
    // <angle> = <dimension-token>
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Angle)) => CssPrimitiveValueKind::Angle,
        // https://svgwg.org/svg2-draft/types.html#presentation-attribute-css-value
        // When parsing an SVG attribute, an angle is allowed without a unit.
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { .. },
            ..
        }) if options.allow_svg_unitless_angle => CssPrimitiveValueKind::Angle,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) if function_can_represent_dimension(function) => {
            CssPrimitiveValueKind::Angle
        }
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_flex_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#flex
    // <flex> = <dimension-token>
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Flex)) => CssPrimitiveValueKind::Flex,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) if function_can_represent_dimension(function) => CssPrimitiveValueKind::Flex,
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_frequency_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#frequency
    // <frequency> = <dimension-token>
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Frequency)) => CssPrimitiveValueKind::Frequency,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) if function_can_represent_dimension(function) => {
            CssPrimitiveValueKind::Frequency
        }
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_length_value_prefix(
    component_value: &ComponentValue,
    options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#lengths
    // <length> = <dimension-token>
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Length)) => CssPrimitiveValueKind::Length,
        // https://drafts.csswg.org/css-values-4/#zero-value
        // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if number.value() == 0.0 || options.allow_quirky_length || options.allow_svg_unitless_length => {
            CssPrimitiveValueKind::Length
        }
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions and anchor-size() still happens in C++.
        ComponentValue::Function(function) if function_can_represent_length(function) => CssPrimitiveValueKind::Length,
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_resolution_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#resolution
    // <resolution> = <dimension-token>
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) if number.value() >= 0.0 && matches!(dimension_for_unit(unit), Some(DimensionType::Resolution)) => {
            CssPrimitiveValueKind::Resolution
        }
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) if function_can_represent_dimension(function) => {
            CssPrimitiveValueKind::Resolution
        }
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_string_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#strings
    // <string> = <string-token>
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { .. },
            ..
        }) => CssPrimitiveValueKind::String,
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_time_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-values-4/#time
    // <time> = <dimension-token>
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Time)) => CssPrimitiveValueKind::Time,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) if function_can_represent_dimension(function) => CssPrimitiveValueKind::Time,
        _ => CssPrimitiveValueKind::Invalid,
    }
}

pub(super) fn parse_opacity_value_prefix(component_value: &ComponentValue) -> CssPrimitiveValueKind {
    // https://drafts.csswg.org/css-color-4/#typedef-opacity-opacity-value
    // <opacity-value> = <number> | <percentage>
    match parse_number_value_prefix(component_value) {
        CssPrimitiveValueKind::Number => CssPrimitiveValueKind::Opacity,
        _ => match parse_percentage_value_prefix(component_value) {
            CssPrimitiveValueKind::Percentage => CssPrimitiveValueKind::Opacity,
            _ => CssPrimitiveValueKind::Invalid,
        },
    }
}

pub(crate) fn parse_easing_value(filtered_input: &[u8]) -> CssEasingValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    match component_values {
        [
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }),
        ] if value.eq_ignore_ascii_case("step-start") || value.eq_ignore_ascii_case("step-end") => {
            CssEasingValueKind::Valid
        }
        [ComponentValue::Function(function)] if function.name.eq_ignore_ascii_case("linear") => {
            parse_linear_easing_function(function)
        }
        [ComponentValue::Function(function)] if function.name.eq_ignore_ascii_case("cubic-bezier") => {
            parse_cubic_bezier_easing_function(function)
        }
        [ComponentValue::Function(function)] if function.name.eq_ignore_ascii_case("steps") => {
            parse_steps_easing_function(function)
        }
        _ => CssEasingValueKind::Invalid,
    }
}

pub(super) fn parse_linear_easing_function(function: &Function) -> CssEasingValueKind {
    // https://drafts.csswg.org/css-easing-2/#typedef-linear-easing-function
    // <linear-easing-function> = linear( [ <number> && <linear-stop-length>? ]# )
    // <linear-stop-length> = <percentage>{1,2}
    let groups = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        let component_values: Vec<_> = component_values
            .iter()
            .filter(|component_value| {
                !matches!(
                    component_value,
                    ComponentValue::PreservedToken(Token {
                        token_type: TokenType::Whitespace,
                        ..
                    })
                )
            })
            .collect();

        if component_values_parse_as_linear_stop(&component_values) {
            return Some(());
        }
        None
    });

    if matches!(groups, Some(groups) if !groups.is_empty()) {
        return CssEasingValueKind::Valid;
    }

    CssEasingValueKind::Invalid
}

pub(super) fn component_values_parse_as_linear_stop(component_values: &[&ComponentValue]) -> bool {
    let mut component_values = component_values.iter().copied().peekable();
    let mut output = false;

    if component_values
        .peek()
        .is_some_and(|component_value| component_value_parse_as_number_prefix(component_value))
    {
        output = true;
        component_values.next();
    }

    for _ in 0..2 {
        if component_values.peek().is_some_and(|component_value| {
            parse_percentage_value_prefix(component_value) == CssPrimitiveValueKind::Percentage
        }) {
            component_values.next();
        }
    }

    if component_values
        .peek()
        .is_some_and(|component_value| component_value_parse_as_number_prefix(component_value))
    {
        if output {
            return false;
        }
        output = true;
        component_values.next();
    }

    output && component_values.next().is_none()
}

pub(super) fn parse_cubic_bezier_easing_function(function: &Function) -> CssEasingValueKind {
    // https://drafts.csswg.org/css-easing-2/#typedef-cubic-bezier-easing-function
    // <cubic-bezier-easing-function> = cubic-bezier( <number [0,1]> , <number> , <number [0,1]> , <number> )
    let groups = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        let [component_value] = strip_whitespace(&component_values) else {
            return None;
        };
        Some(component_value.clone())
    });
    let Some(groups) = groups else {
        return CssEasingValueKind::Invalid;
    };
    let [x1, y1, x2, y2] = groups.as_slice() else {
        return CssEasingValueKind::Invalid;
    };

    if !component_value_parse_as_number_in_range(x1, 0.0, 1.0)
        || !component_value_parse_as_number_prefix(y1)
        || !component_value_parse_as_number_in_range(x2, 0.0, 1.0)
        || !component_value_parse_as_number_prefix(y2)
    {
        return CssEasingValueKind::Invalid;
    }

    CssEasingValueKind::Valid
}

pub(super) fn parse_steps_easing_function(function: &Function) -> CssEasingValueKind {
    // https://drafts.csswg.org/css-easing-2/#typedef-step-easing-function
    // <step-easing-function> = steps( <integer> , <step-position>? )
    let groups = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        let [component_value] = strip_whitespace(&component_values) else {
            return None;
        };
        Some(component_value.clone())
    });
    let Some(groups) = groups else {
        return CssEasingValueKind::Invalid;
    };
    if groups.is_empty() || groups.len() > 2 {
        return CssEasingValueKind::Invalid;
    }

    let mut min_intervals = 1.0;
    if let Some(step_position) = groups.get(1) {
        let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) = step_position
        else {
            return CssEasingValueKind::Invalid;
        };
        if !is_step_position_keyword(value) {
            return CssEasingValueKind::Invalid;
        }
        if value.eq_ignore_ascii_case("jump-none") {
            min_intervals = 2.0;
        }
    }

    if component_value_parse_as_integer_in_range(&groups[0], min_intervals, f64::INFINITY) {
        return CssEasingValueKind::Valid;
    }

    CssEasingValueKind::Invalid
}

pub(super) fn component_value_parse_as_number_prefix(component_value: &ComponentValue) -> bool {
    matches!(
        parse_number_value_prefix(component_value),
        CssPrimitiveValueKind::Number
    )
}

pub(super) fn component_value_parse_as_number_in_range(component_value: &ComponentValue, min: f64, max: f64) -> bool {
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

pub(super) fn component_value_parse_as_integer_in_range(component_value: &ComponentValue, min: f64, max: f64) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number_is_integer(*number) && number.value() >= min && number.value() <= max,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(_) => true,
        _ => false,
    }
}

pub(super) fn is_step_position_keyword(input: &str) -> bool {
    // https://drafts.csswg.org/css-easing-2/#typedef-step-position
    // <step-position> = jump-start | jump-end | jump-none | jump-both | start | end
    input.eq_ignore_ascii_case("jump-start")
        || input.eq_ignore_ascii_case("jump-end")
        || input.eq_ignore_ascii_case("jump-none")
        || input.eq_ignore_ascii_case("jump-both")
        || input.eq_ignore_ascii_case("start")
        || input.eq_ignore_ascii_case("end")
}

pub(super) fn rust_owned_step_position(input: &str) -> Option<RustOwnedStepPosition> {
    if input.eq_ignore_ascii_case("jump-start") {
        return Some(RustOwnedStepPosition::JumpStart);
    }
    if input.eq_ignore_ascii_case("jump-end") {
        return Some(RustOwnedStepPosition::JumpEnd);
    }
    if input.eq_ignore_ascii_case("jump-none") {
        return Some(RustOwnedStepPosition::JumpNone);
    }
    if input.eq_ignore_ascii_case("jump-both") {
        return Some(RustOwnedStepPosition::JumpBoth);
    }
    if input.eq_ignore_ascii_case("start") {
        return Some(RustOwnedStepPosition::Start);
    }
    if input.eq_ignore_ascii_case("end") {
        return Some(RustOwnedStepPosition::End);
    }
    None
}

pub(crate) fn parse_transform_function_value(filtered_input: &[u8]) -> CssTransformFunctionValueKind {
    // https://drafts.csswg.org/css-transforms-1/#typedef-transform-function
    // <transform-function> = <matrix()> | <translate()> | <translateX()> | <translateY()> | <scale()> | <scaleX()> | <scaleY()> | <rotate()> | <skew()> | <skewX()> | <skewY()>
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let [ComponentValue::Function(function)] = strip_whitespace(&component_values) else {
        return CssTransformFunctionValueKind::Invalid;
    };

    let Some(parameters) = transform_function_parameters_from_name(&function.name) else {
        return CssTransformFunctionValueKind::Invalid;
    };

    let arguments = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        let [component_value] = strip_whitespace(&component_values) else {
            return None;
        };
        Some(component_value.clone())
    });
    let Some(arguments) = arguments else {
        return CssTransformFunctionValueKind::Invalid;
    };

    if arguments.len() > parameters.len() {
        return CssTransformFunctionValueKind::Invalid;
    }

    if arguments.len() < parameters.len() && parameters[arguments.len()].required {
        return CssTransformFunctionValueKind::Invalid;
    }

    for (argument, parameter) in arguments.iter().zip(parameters) {
        if !component_value_matches_transform_function_parameter(argument, parameter.parameter_type) {
            return CssTransformFunctionValueKind::Invalid;
        }
    }

    CssTransformFunctionValueKind::Valid
}

pub(super) fn component_value_matches_transform_function_parameter(
    component_value: &ComponentValue,
    parameter_type: TransformFunctionParameterType,
) -> bool {
    match parameter_type {
        TransformFunctionParameterType::Angle => {
            parse_angle_value_prefix(component_value, CssPrimitiveValueOptions::default())
                == CssPrimitiveValueKind::Angle
                || component_value_is_zero_number(component_value)
        }
        TransformFunctionParameterType::Length => {
            parse_length_value_prefix(component_value, CssPrimitiveValueOptions::default())
                == CssPrimitiveValueKind::Length
        }
        TransformFunctionParameterType::LengthNone => {
            parse_length_value_prefix(component_value, CssPrimitiveValueOptions::default())
                == CssPrimitiveValueKind::Length
                || component_value_ref_is_ident(component_value, "none")
        }
        TransformFunctionParameterType::LengthPercentage => {
            parse_length_value_prefix(component_value, CssPrimitiveValueOptions::default())
                == CssPrimitiveValueKind::Length
                || parse_percentage_value_prefix(component_value) == CssPrimitiveValueKind::Percentage
        }
        TransformFunctionParameterType::Number => {
            parse_number_value_prefix(component_value) == CssPrimitiveValueKind::Number
        }
        TransformFunctionParameterType::NumberPercentage => {
            parse_number_value_prefix(component_value) == CssPrimitiveValueKind::Number
                || parse_percentage_value_prefix(component_value) == CssPrimitiveValueKind::Percentage
        }
    }
}

pub(super) fn component_value_parse_as_nested_transform_function_argument(
    component_value: &ComponentValue,
    parameter_type: TransformFunctionParameterType,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if !component_value_matches_transform_function_parameter(component_value, parameter_type) {
        return None;
    }

    match parameter_type {
        TransformFunctionParameterType::Angle => {
            if component_value_is_zero_number(component_value) {
                return Some(RustOwnedNestedPrimitiveValue::Angle {
                    value: 0.0,
                    unit: "deg".to_string(),
                });
            }
            component_value_parse_as_nested_angle(component_value, filtered_input_string)
        }
        TransformFunctionParameterType::Length => {
            component_value_parse_as_nested_length(component_value, filtered_input_string)
        }
        TransformFunctionParameterType::LengthNone => {
            if component_value_ref_is_ident(component_value, "none") {
                return Some(RustOwnedNestedPrimitiveValue::Keyword("none".to_string()));
            }
            component_value_parse_as_nested_length(component_value, filtered_input_string)
        }
        TransformFunctionParameterType::LengthPercentage => {
            component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)
        }
        TransformFunctionParameterType::Number => {
            component_value_parse_as_nested_number(component_value, filtered_input_string)
        }
        TransformFunctionParameterType::NumberPercentage => {
            component_value_parse_as_nested_number_percentage(component_value, filtered_input_string)
        }
    }
}

pub(super) fn component_value_is_zero_number(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if number.value() == 0.0
    )
}

pub(super) fn component_value_ref_is_ident(component_value: &ComponentValue, ident: &str) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case(ident)
    )
}

pub(super) fn component_value_ident(component_value: &ComponentValue) -> Option<&str> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) => Some(value),
        _ => None,
    }
}

pub(crate) fn parse_fit_content_value(filtered_input: &[u8]) -> CssFitContentValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    match component_values {
        [
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }),
        ] if value.eq_ignore_ascii_case("fit-content") => CssFitContentValueKind::Valid,
        [ComponentValue::Function(function)] if function.name.eq_ignore_ascii_case("fit-content") => {
            // https://drafts.csswg.org/css-sizing-3/#funcdef-width-fit-content
            // fit-content() = fit-content( <length-percentage [0,∞]> )
            if component_values_parse_as_single_length_percentage(&function.value) {
                CssFitContentValueKind::Valid
            } else {
                CssFitContentValueKind::Invalid
            }
        }
        _ => CssFitContentValueKind::Invalid,
    }
}

pub(crate) fn parse_basic_shape_value(filtered_input: &[u8]) -> CssBasicShapeValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let [ComponentValue::Function(function)] = strip_whitespace(&component_values) else {
        return CssBasicShapeValueKind::Invalid;
    };

    let is_valid = if function.name.eq_ignore_ascii_case("inset") {
        parse_inset_basic_shape_function(function)
    } else if function.name.eq_ignore_ascii_case("xywh") {
        parse_xywh_basic_shape_function(function)
    } else if function.name.eq_ignore_ascii_case("rect") {
        parse_rect_basic_shape_function(function)
    } else if function.name.eq_ignore_ascii_case("circle") {
        parse_circle_or_ellipse_basic_shape_function(function, BasicShapeRadialFunction::Circle)
    } else if function.name.eq_ignore_ascii_case("ellipse") {
        parse_circle_or_ellipse_basic_shape_function(function, BasicShapeRadialFunction::Ellipse)
    } else if function.name.eq_ignore_ascii_case("polygon") {
        parse_polygon_basic_shape_function(function)
    } else if function.name.eq_ignore_ascii_case("path") {
        parse_path_basic_shape_function(function).is_some()
    } else {
        false
    };

    if is_valid {
        CssBasicShapeValueKind::Valid
    } else {
        CssBasicShapeValueKind::Invalid
    }
}

pub(super) fn parse_inset_basic_shape_function(function: &Function) -> bool {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-inset
    // inset() = inset( <length-percentage>{1,4} [ round <'border-radius'> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut offset_count = 0;
    while offset_count < 4 && consume_length_percentage_component_value(&mut parser) {
        offset_count += 1;
    }

    offset_count > 0 && consume_optional_round_border_radius_and_end(&mut parser)
}

pub(super) fn parse_xywh_basic_shape_function(function: &Function) -> bool {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-xywh
    // xywh() = xywh( <length-percentage>{2} <length-percentage [0,∞]>{2} [ round <'border-radius'> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    for _ in 0..4 {
        if !consume_length_percentage_component_value(&mut parser) {
            return false;
        }
    }

    consume_optional_round_border_radius_and_end(&mut parser)
}

pub(super) fn parse_rect_basic_shape_function(function: &Function) -> bool {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-rect
    // rect() = rect( [ <length-percentage> | auto ]{4} [ round <'border-radius'> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    for _ in 0..4 {
        parser.discard_whitespace();
        if parser.consume_ident_matching("auto") {
            continue;
        }
        if !consume_length_percentage_component_value(&mut parser) {
            return false;
        }
    }

    consume_optional_round_border_radius_and_end(&mut parser)
}

pub(super) struct ParsedRectangleBasicShapeFunction {
    pub(super) components: Vec<RustOwnedNestedPrimitiveValue>,
    pub(super) border_radius: Option<RustOwnedBorderRadius>,
}

pub(super) fn parse_owned_inset_basic_shape_function(
    function: &Function,
    filtered_input_string: &str,
) -> Option<ParsedRectangleBasicShapeFunction> {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-inset
    // inset() = inset( <length-percentage>{1,4} [ round <'border-radius'> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut components = vec![];
    while components.len() < 4 {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };
        let Some(value) = component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)
        else {
            break;
        };
        parser.index += 1;
        components.push(value);
    }

    if components.is_empty() {
        return None;
    }

    Some(ParsedRectangleBasicShapeFunction {
        components,
        border_radius: consume_optional_owned_round_border_radius_and_end(&mut parser, filtered_input_string)?,
    })
}

pub(super) fn parse_owned_xywh_basic_shape_function(
    function: &Function,
    filtered_input_string: &str,
) -> Option<ParsedRectangleBasicShapeFunction> {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-xywh
    // xywh() = xywh( <length-percentage>{2} <length-percentage [0,∞]>{2} [ round <'border-radius'> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut components = vec![];
    for _ in 0..4 {
        parser.discard_whitespace();
        let component_value = parser.next_component_value()?;
        let value = component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)?;
        parser.index += 1;
        components.push(value);
    }

    Some(ParsedRectangleBasicShapeFunction {
        components,
        border_radius: consume_optional_owned_round_border_radius_and_end(&mut parser, filtered_input_string)?,
    })
}

pub(super) fn parse_owned_rect_basic_shape_function(
    function: &Function,
    filtered_input_string: &str,
) -> Option<ParsedRectangleBasicShapeFunction> {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-rect
    // rect() = rect( [ <length-percentage> | auto ]{4} [ round <'border-radius'> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut components = vec![];
    for _ in 0..4 {
        parser.discard_whitespace();
        if parser.consume_ident_matching("auto") {
            components.push(auto_keyword());
            continue;
        }

        let component_value = parser.next_component_value()?;
        let value = component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)?;
        parser.index += 1;
        components.push(value);
    }

    Some(ParsedRectangleBasicShapeFunction {
        components,
        border_radius: consume_optional_owned_round_border_radius_and_end(&mut parser, filtered_input_string)?,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BasicShapeRadialFunction {
    Circle,
    Ellipse,
}

pub(super) fn parse_circle_or_ellipse_basic_shape_function(
    function: &Function,
    radial_function: BasicShapeRadialFunction,
) -> bool {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-circle
    // circle() = circle( <shape-radius>? [ at <position> ]? )
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-ellipse
    // ellipse() = ellipse( <shape-radius>{2}? [ at <position> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut radius_count = 0;
    while radius_count < 2 {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };
        if component_value_parse_as_radial_size_component(component_value) {
            parser.index += 1;
            radius_count += 1;
            continue;
        }
        break;
    }

    if radial_function == BasicShapeRadialFunction::Circle && radius_count > 1 {
        return false;
    }
    if radial_function == BasicShapeRadialFunction::Ellipse && radius_count == 1 {
        return false;
    }

    parser.discard_whitespace();
    if parser.consume_ident_matching("at") {
        parser.discard_whitespace();
        // AD-HOC: C++ still owns the full <position> parser and materialization here.
        if !parser.has_next_component_value() {
            return false;
        }
        return true;
    }

    !parser.has_next_component_value()
}

pub(super) struct ParsedRadialBasicShapeFunction {
    pub(super) radius: Vec<RustOwnedNestedPrimitiveValue>,
    pub(super) position: Option<RustOwnedResolvedPosition>,
}

pub(super) fn parse_owned_circle_or_ellipse_basic_shape_function(
    function: &Function,
    radial_function: BasicShapeRadialFunction,
    filtered_input_string: &str,
) -> Option<ParsedRadialBasicShapeFunction> {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-circle
    // circle() = circle( <shape-radius>? [ at <position> ]? )
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-ellipse
    // ellipse() = ellipse( <shape-radius>{2}? [ at <position> ]? )
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut radius = vec![];
    while radius.len() < 2 {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };
        let Some(component) =
            component_value_parse_as_owned_radial_size_component(component_value, filtered_input_string)
        else {
            break;
        };
        parser.index += 1;
        radius.push(component);
    }

    if radial_function == BasicShapeRadialFunction::Circle && radius.len() > 1 {
        return None;
    }
    if radial_function == BasicShapeRadialFunction::Ellipse && radius.len() == 1 {
        return None;
    }

    parser.discard_whitespace();
    let position = if parser.consume_ident_matching("at") {
        let position_component_values = parser.component_values[parser.index..].to_vec();
        let position = parse_rust_owned_position_value(&position_component_values, filtered_input_string, false)?;
        parser.index = parser.component_values.len();
        Some(position)
    } else {
        None
    };

    parser.discard_whitespace();
    (!parser.has_next_component_value()).then_some(ParsedRadialBasicShapeFunction { radius, position })
}

pub(super) fn component_value_parse_as_owned_radial_size_component(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if let Some(extent) = component_value_parse_as_radial_extent(component_value) {
        return Some(RustOwnedNestedPrimitiveValue::Keyword(
            radial_extent_keyword(extent).to_string(),
        ));
    }

    component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)
}

pub(super) fn radial_extent_keyword(extent: RustOwnedBasicShapeRadialExtent) -> &'static str {
    match extent {
        RustOwnedBasicShapeRadialExtent::ClosestCorner => "closest-corner",
        RustOwnedBasicShapeRadialExtent::ClosestSide => "closest-side",
        RustOwnedBasicShapeRadialExtent::FarthestCorner => "farthest-corner",
        RustOwnedBasicShapeRadialExtent::FarthestSide => "farthest-side",
    }
}

pub(super) fn radial_extent_from_keyword(keyword: &str) -> Option<RustOwnedBasicShapeRadialExtent> {
    if keyword == "closest-corner" {
        return Some(RustOwnedBasicShapeRadialExtent::ClosestCorner);
    }
    if keyword == "closest-side" {
        return Some(RustOwnedBasicShapeRadialExtent::ClosestSide);
    }
    if keyword == "farthest-corner" {
        return Some(RustOwnedBasicShapeRadialExtent::FarthestCorner);
    }
    if keyword == "farthest-side" {
        return Some(RustOwnedBasicShapeRadialExtent::FarthestSide);
    }
    None
}

pub(super) fn component_value_parse_as_radial_extent(
    component_value: &ComponentValue,
) -> Option<RustOwnedBasicShapeRadialExtent> {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    }) = component_value
    else {
        return None;
    };

    if value.eq_ignore_ascii_case("closest-corner") {
        return Some(RustOwnedBasicShapeRadialExtent::ClosestCorner);
    }
    if value.eq_ignore_ascii_case("closest-side") {
        return Some(RustOwnedBasicShapeRadialExtent::ClosestSide);
    }
    if value.eq_ignore_ascii_case("farthest-corner") {
        return Some(RustOwnedBasicShapeRadialExtent::FarthestCorner);
    }
    if value.eq_ignore_ascii_case("farthest-side") {
        return Some(RustOwnedBasicShapeRadialExtent::FarthestSide);
    }
    None
}

pub(super) fn parse_polygon_basic_shape_function(function: &Function) -> bool {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-polygon
    // polygon() = polygon( <'fill-rule'>? , [<length-percentage> <length-percentage>]# )
    let Some(mut arguments) = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        Some(remove_whitespace_component_values(&component_values))
    }) else {
        return false;
    };

    if arguments.is_empty() {
        return false;
    }

    if component_values_parse_as_fill_rule(&arguments[0]) {
        arguments.remove(0);
    }
    if arguments.is_empty() {
        return false;
    }

    arguments.iter().all(|argument| {
        let [x, y] = argument.as_slice() else {
            return false;
        };
        component_value_parse_as_length_percentage(x) && component_value_parse_as_length_percentage(y)
    })
}

pub(super) struct ParsedPolygonBasicShapeFunction {
    pub(super) fill_rule: RustOwnedBasicShapeFillRule,
    pub(super) points: Vec<RustOwnedBasicShapePolygonPoint>,
}

pub(super) fn parse_owned_polygon_basic_shape_function(
    function: &Function,
    filtered_input_string: &str,
) -> Option<ParsedPolygonBasicShapeFunction> {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-polygon
    // polygon() = polygon( <'fill-rule'>? , [<length-percentage> <length-percentage>]# )
    let mut arguments = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        Some(remove_whitespace_component_values(&component_values))
    })?;

    if arguments.is_empty() {
        return None;
    }

    let fill_rule = if let Some(fill_rule) = component_values_fill_rule(&arguments[0]) {
        arguments.remove(0);
        fill_rule
    } else {
        RustOwnedBasicShapeFillRule::Nonzero
    };
    if arguments.is_empty() {
        return None;
    }

    let mut points = Vec::with_capacity(arguments.len());
    for argument in arguments {
        let [x, y] = argument.as_slice() else {
            return None;
        };
        points.push(RustOwnedBasicShapePolygonPoint {
            x: component_value_parse_as_nested_length_percentage(x, filtered_input_string)?,
            y: component_value_parse_as_nested_length_percentage(y, filtered_input_string)?,
        });
    }

    Some(ParsedPolygonBasicShapeFunction { fill_rule, points })
}

pub(super) struct ParsedPathBasicShapeFunction {
    pub(super) fill_rule: RustOwnedBasicShapeFillRule,
    pub(super) path_data: String,
}

pub(super) fn parse_path_basic_shape_function(function: &Function) -> Option<ParsedPathBasicShapeFunction> {
    // https://drafts.csswg.org/css-shapes-1/#funcdef-basic-shape-path
    // <path()> = path( <'fill-rule'>?, <string> )
    let arguments = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        Some(strip_whitespace(&component_values).to_vec())
    })?;

    let (fill_rule, path) = match arguments.as_slice() {
        [path] => (RustOwnedBasicShapeFillRule::Nonzero, path),
        [fill_rule, path] => (component_values_fill_rule(fill_rule)?, path),
        _ => return None,
    };

    Some(ParsedPathBasicShapeFunction {
        fill_rule,
        path_data: component_values_string_value(path)?.to_string(),
    })
}

pub(super) fn component_values_fill_rule(component_values: &[ComponentValue]) -> Option<RustOwnedBasicShapeFillRule> {
    let [component_value] = component_values else {
        return None;
    };
    if component_value_is_ident(Some(component_value), "nonzero") {
        return Some(RustOwnedBasicShapeFillRule::Nonzero);
    }
    if component_value_is_ident(Some(component_value), "evenodd") {
        return Some(RustOwnedBasicShapeFillRule::Evenodd);
    }

    None
}

pub(super) fn consume_optional_round_border_radius_and_end(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    if parser.consume_ident_matching("round") && !consume_border_radius_rect_component_values(parser) {
        return false;
    }

    parser.discard_whitespace();
    !parser.has_next_component_value()
}

pub(super) fn consume_optional_owned_round_border_radius_and_end(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<Option<RustOwnedBorderRadius>> {
    parser.discard_whitespace();
    if !parser.consume_ident_matching("round") {
        parser.discard_whitespace();
        return (!parser.has_next_component_value()).then_some(None);
    }

    let component_values = remove_whitespace_component_values(&parser.component_values[parser.index..]);
    parser.index = parser.component_values.len();
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    let slash_positions = component_values
        .iter()
        .enumerate()
        .filter_map(|(index, component_value)| component_value_is_delim(Some(component_value), '/').then_some(index))
        .collect::<Vec<_>>();

    if slash_positions.len() > 1 {
        return None;
    }

    let (horizontal_radii, vertical_radii) = if let Some(slash_position) = slash_positions.first() {
        (
            &component_values[..*slash_position],
            Some(&component_values[*slash_position + 1..]),
        )
    } else {
        (component_values.as_slice(), None)
    };

    let horizontal_radii = rust_owned_border_radius_shorthand_side_values(horizontal_radii, filtered_input_string)?;
    let vertical_radii = if let Some(vertical_radii) = vertical_radii {
        rust_owned_border_radius_shorthand_side_values(vertical_radii, filtered_input_string)?
    } else {
        vec![]
    };

    Some(Some(RustOwnedBorderRadius {
        horizontal_radii,
        vertical_radii,
    }))
}

pub(super) fn consume_border_radius_rect_component_values(parser: &mut ComponentValueParser) -> bool {
    let mut horizontal_count = 0;
    let mut vertical_count = 0;
    let mut reading_vertical = false;

    while parser.has_next_component_value() {
        parser.discard_whitespace();
        if parser.consume_a_delim('/') {
            if reading_vertical || horizontal_count == 0 {
                return false;
            }
            reading_vertical = true;
            continue;
        }

        if !consume_length_percentage_component_value(parser) {
            break;
        }

        if reading_vertical {
            vertical_count += 1;
        } else {
            horizontal_count += 1;
        }

        if horizontal_count > 4 || vertical_count > 4 {
            return false;
        }
    }

    horizontal_count > 0 && (!reading_vertical || vertical_count > 0)
}

pub(super) fn consume_length_percentage_component_value(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };

    if component_value_parse_as_length_percentage(component_value) {
        parser.index += 1;
        return true;
    }

    false
}

pub(super) fn component_values_parse_as_single_length_percentage(component_values: &[ComponentValue]) -> bool {
    let [component_value] = strip_whitespace(component_values) else {
        return false;
    };
    component_value_parse_as_length_percentage(component_value)
}

pub(super) fn remove_whitespace_component_values(component_values: &[ComponentValue]) -> Vec<ComponentValue> {
    component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
        .cloned()
        .collect()
}

pub(super) fn component_value_parse_as_radial_size_component(component_value: &ComponentValue) -> bool {
    component_value_parse_as_length_percentage(component_value)
        || component_value_is_ident(Some(component_value), "closest-corner")
        || component_value_is_ident(Some(component_value), "closest-side")
        || component_value_is_ident(Some(component_value), "farthest-corner")
        || component_value_is_ident(Some(component_value), "farthest-side")
}

pub(super) fn component_values_parse_as_fill_rule(component_values: &[ComponentValue]) -> bool {
    component_values_fill_rule(component_values).is_some()
}

pub(crate) fn parse_grid_auto_flow_value(filtered_input: &[u8]) -> CssGridAutoFlowValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    // https://www.w3.org/TR/css-grid-1/#grid-auto-flow-property
    // grid-auto-flow = [ row | column ] || dense
    let axis = consume_optional_grid_auto_flow_axis(&mut parser);
    let dense = consume_optional_ident_matching(&mut parser, "dense");
    let axis_after_dense = if axis {
        false
    } else {
        consume_optional_grid_auto_flow_axis(&mut parser)
    };

    parser.discard_whitespace();
    if (axis || dense || axis_after_dense) && !parser.has_next_component_value() {
        CssGridAutoFlowValueKind::Valid
    } else {
        CssGridAutoFlowValueKind::Invalid
    }
}

pub(super) fn consume_optional_grid_auto_flow_axis(parser: &mut ComponentValueParser) -> bool {
    consume_optional_ident_matching(parser, "row") || consume_optional_ident_matching(parser, "column")
}

pub(super) fn consume_optional_ident_matching(parser: &mut ComponentValueParser, expected: &str) -> bool {
    parser.discard_whitespace();
    parser.consume_ident_matching(expected)
}

pub(super) fn consume_optional_ident_matching_source(
    parser: &mut ComponentValueParser,
    expected: &str,
) -> Option<String> {
    parser.discard_whitespace();
    parser.consume_ident_matching(expected).then(|| expected.to_string())
}

pub(crate) fn parse_grid_track_placement_value(filtered_input: &[u8]) -> CssGridTrackPlacementValueKind {
    if parse_rust_owned_grid_track_placement_value(filtered_input).is_some() {
        CssGridTrackPlacementValueKind::Valid
    } else {
        CssGridTrackPlacementValueKind::Invalid
    }
}

pub(super) fn parse_rust_owned_grid_track_placement_value(
    filtered_input: &[u8],
) -> Option<RustOwnedGridTrackPlacement> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input_string = filtered_input_to_string(filtered_input);

    // https://www.w3.org/TR/css-grid-2/#line-placement
    // <grid-line> =
    //     auto |
    //     <custom-ident> |
    //     [ [ <integer [-∞,-1]> | <integer [1,∞]> ] && <custom-ident>? ] |
    //     [ span && [ <integer [1,∞]> || <custom-ident> ] ]
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();
    if parser.consume_ident_matching("auto") {
        parser.discard_whitespace();
        return (!parser.has_next_component_value()).then_some(RustOwnedGridTrackPlacement::Auto);
    }

    let mut is_span = false;
    let mut parsed_custom_ident = None;
    let mut parsed_integer = None;
    let mut parsed_integer_value = 0.0;
    let mut parsed_integer_has_known_value = false;

    while parser.has_next_component_value() {
        if parser.consume_ident_matching("span") {
            if is_span {
                return None;
            }

            // NOTE: "span" must not appear in between <custom-ident> and <integer>.
            parser.discard_whitespace();
            if parser.has_next_component_value() && (parsed_custom_ident.is_some() || parsed_integer.is_some()) {
                return None;
            }

            is_span = true;
            continue;
        }

        if let Some(custom_ident) = consume_grid_line_custom_ident(&mut parser) {
            if parsed_custom_ident.is_some() {
                return None;
            }
            parsed_custom_ident = Some(custom_ident);
            continue;
        }

        if let Some((integer, integer_value)) =
            consume_integer_component_value_payload(&mut parser, &filtered_input_string)
        {
            if parsed_integer.is_some() {
                return None;
            }
            parsed_integer = Some(integer);
            parsed_integer_value = integer_value;
            parsed_integer_has_known_value = true;
            continue;
        }

        if let Some(integer) =
            consume_integer_math_function_component_value_payload(&mut parser, &filtered_input_string)
        {
            if parsed_integer.is_some() {
                return None;
            }
            parsed_integer = Some(integer);
            continue;
        }

        return None;
    }

    if !is_span && (parsed_integer.is_some() || parsed_custom_ident.is_some()) {
        return (parsed_integer.is_none() || !parsed_integer_has_known_value || parsed_integer_value != 0.0).then_some(
            RustOwnedGridTrackPlacement::Line {
                line_number: parsed_integer,
                name: parsed_custom_ident,
            },
        );
    }

    if is_span && (parsed_integer.is_some() || parsed_custom_ident.is_some()) {
        return (parsed_integer.is_none() || !parsed_integer_has_known_value || parsed_integer_value > 0.0).then_some(
            RustOwnedGridTrackPlacement::Span {
                line_number: parsed_integer,
                name: parsed_custom_ident,
            },
        );
    }

    None
}

pub(super) fn consume_grid_line_custom_ident(parser: &mut ComponentValueParser) -> Option<String> {
    parser.discard_whitespace();
    let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    })) = parser.next_component_value()
    else {
        return None;
    };

    if !is_valid_custom_ident(value, &["auto"]) {
        return None;
    }

    let value = value.clone();
    parser.index += 1;
    Some(value)
}

pub(super) fn consume_integer_component_value(parser: &mut ComponentValueParser) -> Option<f64> {
    parser.discard_whitespace();
    let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Number { number },
        ..
    })) = parser.next_component_value()
    else {
        return None;
    };

    if !number_is_integer(*number) {
        return None;
    }

    let value = number.value();
    parser.index += 1;
    Some(value)
}

pub(super) fn consume_integer_component_value_payload(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<(RustOwnedNestedPrimitiveValue, f64)> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Number { number },
        ..
    }) = component_value
    else {
        return None;
    };

    if !number_is_integer(*number) {
        return None;
    }

    let source =
        serialize_component_values_for_reparsing(std::slice::from_ref(component_value), filtered_input_string)?;
    let value = number.value();
    parser.index += 1;
    let integer = if value >= i32::MIN as f64 && value <= i32::MAX as f64 {
        RustOwnedNestedPrimitiveValue::Integer(value as i32)
    } else {
        RustOwnedNestedPrimitiveValue::Source(source)
    };
    Some((integer, value))
}

pub(super) fn consume_integer_math_function_component_value_payload(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    parser.discard_whitespace();
    let Some(component_value @ ComponentValue::Function(function)) = parser.next_component_value() else {
        return None;
    };

    if !is_math_function_name(&function.name)
        && !function.name.eq_ignore_ascii_case("sibling-index")
        && !function.name.eq_ignore_ascii_case("sibling-count")
    {
        return None;
    }

    let value = parse_rust_owned_math_function(
        PropertyValueType::Integer,
        std::slice::from_ref(component_value),
        filtered_input_string.as_bytes(),
    )
    .map(RustOwnedNestedPrimitiveValue::MathFunction)
    .or_else(|| {
        parse_rust_owned_tree_counting_function(PropertyValueType::Integer, std::slice::from_ref(component_value))
            .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
    })?;
    parser.index += 1;
    Some(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum GridTrackSizeListSyntax {
    TrackSizeList,
    TrackList,
}

pub(crate) fn parse_grid_auto_track_sizes_value(filtered_input: &[u8]) -> CssGridTrackSizeListValueKind {
    if parse_rust_owned_grid_track_size_list_value(filtered_input, GridTrackSizeListSyntax::TrackSizeList).is_some() {
        CssGridTrackSizeListValueKind::Valid
    } else {
        CssGridTrackSizeListValueKind::Invalid
    }
}

pub(crate) fn parse_grid_track_size_list_value(filtered_input: &[u8]) -> CssGridTrackSizeListValueKind {
    if parse_rust_owned_grid_track_size_list_value(filtered_input, GridTrackSizeListSyntax::TrackList).is_some() {
        CssGridTrackSizeListValueKind::Valid
    } else {
        CssGridTrackSizeListValueKind::Invalid
    }
}

pub(super) fn parse_rust_owned_grid_track_size_list_value(
    filtered_input: &[u8],
    syntax: GridTrackSizeListSyntax,
) -> Option<RustOwnedGridTrackSizeList> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let stripped_component_values = strip_whitespace(&component_values);
    let filtered_input_string = filtered_input_to_string(filtered_input);

    if syntax == GridTrackSizeListSyntax::TrackList
        && matches!(stripped_component_values, [component_value] if component_value_is_ident(Some(component_value), "none"))
    {
        return Some(RustOwnedGridTrackSizeList::None);
    }

    let mut parser = ComponentValueParser::new(component_values);
    let items = match syntax {
        GridTrackSizeListSyntax::TrackSizeList => {
            parse_one_or_more_grid_track_sizes(&mut parser, &filtered_input_string)
        }
        GridTrackSizeListSyntax::TrackList => parse_grid_auto_track_list(&mut parser, &filtered_input_string)
            .or_else(|| parse_grid_track_list(&mut parser, &filtered_input_string)),
    }?;

    parser.discard_whitespace();
    (!parser.has_next_component_value()).then_some(RustOwnedGridTrackSizeList::List(items))
}

pub(super) fn parse_one_or_more_grid_track_sizes(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<Vec<RustOwnedGridTrackSizeListItem>> {
    // https://www.w3.org/TR/css-grid-2/#auto-tracks
    // <track-size>+
    let mut items = Vec::new();
    while let Some(track_size) = parse_grid_track_size(parser, filtered_input_string) {
        items.push(RustOwnedGridTrackSizeListItem::Track(RustOwnedExplicitGridTrack::Size(
            track_size,
        )));
    }
    (!items.is_empty()).then_some(items)
}

pub(super) fn parse_grid_track_list(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<Vec<RustOwnedGridTrackSizeListItem>> {
    // https://www.w3.org/TR/css-grid-2/#typedef-track-list
    // <track-list> = [ <line-names>? [ <track-size> | <track-repeat> ] ]+ <line-names>?
    parse_track_list_impl(parser, filtered_input_string, |parser, filtered_input_string| {
        parse_grid_track_repeat(parser, filtered_input_string)
            .or_else(|| parse_grid_track_size(parser, filtered_input_string).map(RustOwnedExplicitGridTrack::Size))
    })
}

pub(super) fn parse_grid_auto_track_list(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<Vec<RustOwnedGridTrackSizeListItem>> {
    // https://www.w3.org/TR/css-grid-2/#typedef-auto-track-list
    // <auto-track-list> = [ <line-names>? [ <fixed-size> | <fixed-repeat> ] ]* <line-names>? <auto-repeat>
    //                     [ <line-names>? [ <fixed-size> | <fixed-repeat> ] ]* <line-names>?
    let start = parser.index;
    let mut items = if let Some(items) =
        parse_track_list_impl(parser, filtered_input_string, |parser, filtered_input_string| {
            parse_grid_fixed_repeat(parser, filtered_input_string)
                .or_else(|| parse_grid_fixed_size(parser, filtered_input_string).map(RustOwnedExplicitGridTrack::Size))
        }) {
        items
    } else {
        let mut items = Vec::new();
        for line_names in parse_grid_line_names_list(parser) {
            items.push(RustOwnedGridTrackSizeListItem::LineNames(line_names));
        }
        items
    };
    let Some(auto_repeat) = parse_grid_auto_repeat(parser, filtered_input_string) else {
        parser.index = start;
        return None;
    };
    items.push(RustOwnedGridTrackSizeListItem::Track(auto_repeat));
    if let Some(mut trailing_items) =
        parse_track_list_impl(parser, filtered_input_string, |parser, filtered_input_string| {
            parse_grid_fixed_repeat(parser, filtered_input_string)
                .or_else(|| parse_grid_fixed_size(parser, filtered_input_string).map(RustOwnedExplicitGridTrack::Size))
        })
    {
        items.append(&mut trailing_items);
    } else if let Some(line_names) = parse_grid_line_names(parser).filter(|line_names| !line_names.is_empty()) {
        items.push(RustOwnedGridTrackSizeListItem::LineNames(line_names));
    }
    Some(items)
}

pub(super) fn parse_track_list_impl<F>(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
    mut track_parser: F,
) -> Option<Vec<RustOwnedGridTrackSizeListItem>>
where
    F: FnMut(&mut ComponentValueParser, &str) -> Option<RustOwnedExplicitGridTrack>,
{
    let mut items = Vec::new();
    let mut has_track = false;
    loop {
        let before_track = parser.index;
        let line_names = parse_grid_line_names_list(parser);
        let Some(track) = track_parser(parser, filtered_input_string) else {
            parser.index = before_track;
            break;
        };
        for line_names in line_names {
            items.push(RustOwnedGridTrackSizeListItem::LineNames(line_names));
        }
        items.push(RustOwnedGridTrackSizeListItem::Track(track));
        has_track = true;
    }

    if has_track && let Some(line_names) = parse_grid_line_names(parser).filter(|line_names| !line_names.is_empty()) {
        items.push(RustOwnedGridTrackSizeListItem::LineNames(line_names));
    }

    has_track.then_some(items)
}

pub(super) fn parse_grid_line_names_list(parser: &mut ComponentValueParser) -> Vec<Vec<String>> {
    let mut line_names_list = Vec::new();
    while let Some(line_names) = parse_grid_line_names(parser) {
        if !line_names.is_empty() {
            line_names_list.push(line_names);
        }
    }
    line_names_list
}

pub(super) fn parse_grid_line_names(parser: &mut ComponentValueParser) -> Option<Vec<String>> {
    // https://www.w3.org/TR/css-grid-2/#typedef-line-names
    // <line-names> = '[' <custom-ident>* ']'
    parser.discard_whitespace();
    let Some(ComponentValue::SimpleBlock(block)) = parser.next_component_value() else {
        return None;
    };
    if !is_square_block(block) {
        return None;
    }

    let mut line_names = Vec::new();
    let mut block_parser = ComponentValueParser::new(block.value.clone());
    while block_parser.has_next_component_value() {
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = block_parser.next_component_value()
        else {
            return None;
        };
        if !is_valid_custom_ident(value, &["span", "auto"]) {
            return None;
        }
        line_names.push(value.clone());
        block_parser.index += 1;
    }

    parser.index += 1;
    Some(line_names)
}

pub(super) fn parse_grid_track_repeat(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedExplicitGridTrack> {
    // https://www.w3.org/TR/css-grid-2/#typedef-track-repeat
    // <track-repeat> = repeat( [ <integer [1,∞]> ] , [ <line-names>? <track-size> ]+ <line-names>? )
    parse_grid_repeat_function(
        parser,
        filtered_input_string,
        parse_positive_integer_component_values,
        |parser, filtered_input_string| {
            parse_track_list_impl(parser, filtered_input_string, |parser, filtered_input_string| {
                parse_grid_track_size(parser, filtered_input_string).map(RustOwnedExplicitGridTrack::Size)
            })
        },
    )
}

pub(super) fn parse_grid_auto_repeat(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedExplicitGridTrack> {
    // https://www.w3.org/TR/css-grid-2/#typedef-auto-repeat
    // <auto-repeat> = repeat( [ auto-fill | auto-fit ] , [ <line-names>? <fixed-size> ]+ <line-names>? )
    parse_grid_repeat_function(
        parser,
        filtered_input_string,
        |component_values| {
            let [component_value] = strip_whitespace(component_values) else {
                return false;
            };
            component_value_is_ident(Some(component_value), "auto-fill")
                || component_value_is_ident(Some(component_value), "auto-fit")
        },
        |parser, filtered_input_string| {
            parse_track_list_impl(parser, filtered_input_string, |parser, filtered_input_string| {
                parse_grid_fixed_size(parser, filtered_input_string).map(RustOwnedExplicitGridTrack::Size)
            })
        },
    )
}

pub(super) fn parse_grid_fixed_repeat(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedExplicitGridTrack> {
    // https://www.w3.org/TR/css-grid-2/#typedef-fixed-repeat
    // <fixed-repeat> = repeat( [ <integer [1,∞]> ] , [ <line-names>? <fixed-size> ]+ <line-names>? )
    parse_grid_repeat_function(
        parser,
        filtered_input_string,
        parse_positive_integer_component_values,
        |parser, filtered_input_string| {
            parse_track_list_impl(parser, filtered_input_string, |parser, filtered_input_string| {
                parse_grid_fixed_size(parser, filtered_input_string).map(RustOwnedExplicitGridTrack::Size)
            })
        },
    )
}

pub(super) fn parse_grid_repeat_function<F, G>(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
    repeat_type_parser: F,
    repeat_track_parser: G,
) -> Option<RustOwnedExplicitGridTrack>
where
    F: Fn(&[ComponentValue]) -> bool,
    G: Fn(&mut ComponentValueParser, &str) -> Option<Vec<RustOwnedGridTrackSizeListItem>>,
{
    parser.discard_whitespace();
    let start = parser.index;
    let Some(ComponentValue::Function(function)) = parser.next_component_value().cloned() else {
        return None;
    };
    if !function.name.eq_ignore_ascii_case("repeat") {
        return None;
    }

    let arguments = parse_comma_separated_component_values(function.value, Some)?;
    let [repeat_type, repeat_track_list] = arguments.as_slice() else {
        return None;
    };
    if !repeat_type_parser(repeat_type) {
        return None;
    }

    let mut repeat_track_list_parser = ComponentValueParser::new(repeat_track_list.clone());
    let track_list = repeat_track_parser(&mut repeat_track_list_parser, filtered_input_string)?;
    repeat_track_list_parser.discard_whitespace();
    if repeat_track_list_parser.has_next_component_value() {
        parser.index = start;
        return None;
    }

    let repeat_type = parse_grid_repeat_type(repeat_type, filtered_input_string)?;
    parser.index += 1;
    Some(RustOwnedExplicitGridTrack::Repeat(RustOwnedGridRepeat {
        repeat_type,
        track_list,
    }))
}

pub(super) fn parse_grid_repeat_type(
    component_values: &[ComponentValue],
    filtered_input_string: &str,
) -> Option<RustOwnedGridRepeatType> {
    let stripped_component_values = strip_whitespace(component_values);
    let [component_value] = stripped_component_values else {
        return None;
    };
    if component_value_is_ident(Some(component_value), "auto-fill") {
        return Some(RustOwnedGridRepeatType::AutoFill);
    }
    if component_value_is_ident(Some(component_value), "auto-fit") {
        return Some(RustOwnedGridRepeatType::AutoFit);
    }
    if parse_positive_integer_component_values(component_values) {
        let count = component_value_parse_as_nested_integer(component_value, filtered_input_string)?;
        return Some(RustOwnedGridRepeatType::Fixed { count });
    }
    None
}

pub(super) fn parse_grid_track_size(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedGridTrackSize> {
    // https://www.w3.org/TR/css-grid-2/#typedef-track-size
    // <track-size> = <track-breadth> | minmax( <inflexible-breadth> , <track-breadth> ) | fit-content( <length-percentage [0,∞]> )
    let start = parser.index;
    if let Some(source) = parse_grid_track_breadth(parser, filtered_input_string) {
        return Some(RustOwnedGridTrackSize::Breadth(source));
    }

    if let Some((min, max)) = parse_grid_minmax_function(
        parser,
        filtered_input_string,
        parse_grid_inflexible_breadth,
        parse_grid_track_breadth,
    ) {
        return Some(RustOwnedGridTrackSize::MinMax { min, max });
    }

    parser.index = start;
    parse_grid_fit_content_function(parser, filtered_input_string).map(RustOwnedGridTrackSize::FitContent)
}

pub(super) fn parse_grid_fixed_size(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedGridTrackSize> {
    // https://www.w3.org/TR/css-grid-2/#typedef-fixed-size
    // <fixed-size> = <fixed-breadth> | minmax( <fixed-breadth> , <track-breadth> ) | minmax( <inflexible-breadth> , <fixed-breadth> )
    let start = parser.index;
    if let Some(source) = parse_grid_fixed_breadth(parser, filtered_input_string) {
        return Some(RustOwnedGridTrackSize::Breadth(source));
    }

    if let Some((min, max)) = parse_grid_minmax_function(
        parser,
        filtered_input_string,
        parse_grid_fixed_breadth,
        parse_grid_track_breadth,
    ) {
        return Some(RustOwnedGridTrackSize::MinMax { min, max });
    }

    parser.index = start;
    parse_grid_minmax_function(
        parser,
        filtered_input_string,
        parse_grid_inflexible_breadth,
        parse_grid_fixed_breadth,
    )
    .map(|(min, max)| RustOwnedGridTrackSize::MinMax { min, max })
}

pub(super) fn parse_grid_minmax_function<F, G>(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
    min_parser: F,
    max_parser: G,
) -> Option<(RustOwnedNestedPrimitiveValue, RustOwnedNestedPrimitiveValue)>
where
    F: Fn(&mut ComponentValueParser, &str) -> Option<RustOwnedNestedPrimitiveValue>,
    G: Fn(&mut ComponentValueParser, &str) -> Option<RustOwnedNestedPrimitiveValue>,
{
    parser.discard_whitespace();
    let start = parser.index;
    let Some(ComponentValue::Function(function)) = parser.next_component_value().cloned() else {
        return None;
    };
    if !function.name.eq_ignore_ascii_case("minmax") {
        return None;
    }

    let arguments = parse_comma_separated_component_values(function.value, Some)?;
    let [min_value, max_value] = arguments.as_slice() else {
        return None;
    };

    let mut min_value_parser = ComponentValueParser::new(min_value.clone());
    let mut max_value_parser = ComponentValueParser::new(max_value.clone());
    let Some(min) = min_parser(&mut min_value_parser, filtered_input_string) else {
        parser.index = start;
        return None;
    };
    min_value_parser.discard_whitespace();
    if min_value_parser.has_next_component_value() {
        parser.index = start;
        return None;
    }
    let Some(max) = max_parser(&mut max_value_parser, filtered_input_string) else {
        parser.index = start;
        return None;
    };
    max_value_parser.discard_whitespace();
    if max_value_parser.has_next_component_value() {
        parser.index = start;
        return None;
    }

    parser.index += 1;
    Some((min, max))
}

pub(super) fn parse_grid_fit_content_function(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    parser.discard_whitespace();
    let Some(ComponentValue::Function(function)) = parser.next_component_value() else {
        return None;
    };
    if !function.name.eq_ignore_ascii_case("fit-content") {
        return None;
    }
    let [component_value] = strip_whitespace(&function.value) else {
        return None;
    };
    let value = component_value_parse_as_nested_non_negative_length_percentage(component_value, filtered_input_string)?;

    parser.index += 1;
    Some(value)
}

pub(super) fn parse_grid_track_breadth(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    // https://www.w3.org/TR/css-grid-2/#typedef-track-breadth
    // <track-breadth> = <length-percentage [0,∞]> | <flex [0,∞]> | min-content | max-content | auto
    parse_grid_inflexible_breadth(parser, filtered_input_string)
        .or_else(|| consume_grid_flex_component_value(parser, filtered_input_string))
}

pub(super) fn parse_grid_inflexible_breadth(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    // https://www.w3.org/TR/css-grid-2/#typedef-inflexible-breadth
    // <inflexible-breadth>  = <length-percentage [0,∞]> | min-content | max-content | auto
    parse_grid_fixed_breadth(parser, filtered_input_string)
        .or_else(|| {
            consume_optional_ident_matching(parser, "min-content")
                .then_some(RustOwnedNestedPrimitiveValue::Keyword("min-content".to_string()))
        })
        .or_else(|| {
            consume_optional_ident_matching(parser, "max-content")
                .then_some(RustOwnedNestedPrimitiveValue::Keyword("max-content".to_string()))
        })
        .or_else(|| consume_optional_ident_matching(parser, "auto").then_some(auto_keyword()))
}

pub(super) fn parse_grid_fixed_breadth(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    // https://www.w3.org/TR/css-grid-2/#typedef-fixed-breadth
    // <fixed-breadth> = <length-percentage [0,∞]>
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    let value = component_value_parse_as_nested_non_negative_length_percentage(component_value, filtered_input_string)?;
    parser.index += 1;
    Some(value)
}

pub(super) fn component_value_parse_as_nested_non_negative_length_percentage(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if !component_value_parse_as_non_negative_length_percentage(component_value) {
        return None;
    }
    // AD-HOC: The Rust side does not yet compute math-function result types.
    // Grid track breadth needs to distinguish calc(1fr) from calc(1px), so
    // classify math functions containing flex dimensions as <flex> instead.
    if matches!(component_value, ComponentValue::Function(function) if is_math_function_name(&function.name))
        && component_value_contains_flex_dimension(component_value)
    {
        return None;
    }
    component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)
}

pub(super) fn component_value_parse_as_non_negative_length_percentage(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) => number.value() >= 0.0 && matches!(dimension_for_unit(unit), Some(DimensionType::Length)),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { number },
            ..
        }) => number.value() >= 0.0,
        // https://drafts.csswg.org/css-values-4/#zero-value
        // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() == 0.0,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => {
            is_math_function_name(&function.name) || function.name.eq_ignore_ascii_case("random")
        }
        _ => false,
    }
}

pub(super) fn component_value_parse_as_non_negative_length(component_value: &ComponentValue) -> bool {
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
        ComponentValue::Function(function) => {
            is_math_function_name(&function.name) || function.name.eq_ignore_ascii_case("random")
        }
        _ => false,
    }
}

pub(super) fn consume_grid_flex_component_value(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    if !component_value_parse_as_non_negative_flex(component_value) {
        return None;
    }

    let value = component_value_parse_as_nested_non_negative_flex(component_value, filtered_input_string)?;
    parser.index += 1;
    Some(value)
}

pub(super) fn component_value_parse_as_nested_non_negative_flex(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Flex)) => {
            Some(RustOwnedNestedPrimitiveValue::Flex {
                value: number.value(),
                unit: unit.to_string(),
            })
        }
        ComponentValue::Function(_) => {
            if !component_value_contains_flex_dimension(component_value) {
                return None;
            }
            // AD-HOC: The Rust side only recognizes the syntactic branch here.
            // Materializing and range-checking math functions still happens in C++.
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), filtered_input_string)
                .map(RustOwnedNestedPrimitiveValue::FlexSource)
        }
        _ => None,
    }
}

pub(super) fn component_value_contains_flex_dimension(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) => unit.eq_ignore_ascii_case("fr"),
        ComponentValue::Function(function) => function.value.iter().any(component_value_contains_flex_dimension),
        ComponentValue::SimpleBlock(block) => block.value.iter().any(component_value_contains_flex_dimension),
        _ => false,
    }
}

pub(super) fn component_value_parse_as_non_negative_flex(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) => number.value() >= 0.0 && unit.eq_ignore_ascii_case("fr"),
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => {
            (is_math_function_name(&function.name) || function.name.eq_ignore_ascii_case("random"))
                && component_value_contains_flex_dimension(component_value)
        }
        _ => false,
    }
}

pub(crate) fn parse_flex_shorthand_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://drafts.csswg.org/css-flexbox-1/#flex-property
    // Value: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]
    match component_values.as_slice() {
        [component_value] if component_value_is_ident(Some(component_value), "none") => return true,
        [] => return false,
        _ if component_values.len() > 3 => return false,
        _ => {}
    }

    flex_shorthand_component_values_match(&component_values, &["flex-grow"])
        || flex_shorthand_component_values_match(&component_values, &["flex-basis"])
        || flex_shorthand_component_values_match(&component_values, &["flex-grow", "flex-shrink"])
        || flex_shorthand_component_values_match(&component_values, &["flex-grow", "flex-basis"])
        || flex_shorthand_component_values_match(&component_values, &["flex-basis", "flex-grow"])
        || flex_shorthand_component_values_match(&component_values, &["flex-grow", "flex-shrink", "flex-basis"])
        || flex_shorthand_component_values_match(&component_values, &["flex-basis", "flex-grow", "flex-shrink"])
}

pub(super) fn flex_shorthand_component_values_match(component_values: &[ComponentValue], pattern: &[&str]) -> bool {
    component_values.len() == pattern.len()
        && component_values
            .iter()
            .zip(pattern)
            .all(|(component_value, component_pattern)| match *component_pattern {
                "flex-grow" | "flex-shrink" => component_value_parse_as_non_negative_number(component_value),
                "flex-basis" => {
                    if pattern.first() == Some(&"flex-basis") && component_values.len() > 1 {
                        component_value_parse_as_flex_basis_before_flex_factors(component_value)
                    } else {
                        component_value_parse_as_flex_basis(component_value)
                    }
                }
                _ => false,
            })
}

pub(super) fn component_value_parse_as_flex_basis(component_value: &ComponentValue) -> bool {
    // https://drafts.csswg.org/css-flexbox-1/#flex-basis-property
    // Value: content | <'width'>
    //
    // https://drafts.csswg.org/css-sizing-3/#propdef-width
    // Value: auto | <length-percentage [0,∞]> | min-content | max-content | fit-content(<length-percentage [0,∞]>) | <calc-size()> | <anchor-size()>
    component_value_is_ident(Some(component_value), "auto")
        || component_value_is_ident(Some(component_value), "content")
        || component_value_is_ident(Some(component_value), "fit-content")
        || component_value_is_ident(Some(component_value), "min-content")
        || component_value_is_ident(Some(component_value), "max-content")
        || component_value_parse_as_non_negative_length_percentage(component_value)
        || matches!(
            component_value,
            ComponentValue::Function(function)
                if function.name.eq_ignore_ascii_case("fit-content")
                    || function.name.eq_ignore_ascii_case("calc-size")
        )
}

pub(super) fn component_value_parse_as_flex_basis_before_flex_factors(component_value: &ComponentValue) -> bool {
    // NOTE: Unitless zero can be a <length-percentage>, but the legacy flex
    // shorthand parser gives flex factors precedence for numeric prefixes.
    // https://drafts.csswg.org/css-flexbox-1/#flex-property
    !matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { .. },
            ..
        })
    ) && component_value_parse_as_flex_basis(component_value)
}

pub(crate) fn parse_flex_flow_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://drafts.csswg.org/css-flexbox-1/#flex-flow-property
    // Value: <'flex-direction'> || <'flex-wrap'>
    if component_values.is_empty() || component_values.len() > 2 {
        return false;
    }

    let mut has_flex_direction = false;
    let mut has_flex_wrap = false;

    for component_value in &component_values {
        if component_value_parse_as_flex_direction(component_value) {
            if has_flex_direction {
                return false;
            }
            has_flex_direction = true;
            continue;
        }

        if component_value_parse_as_flex_wrap(component_value) {
            if has_flex_wrap {
                return false;
            }
            has_flex_wrap = true;
            continue;
        }

        return false;
    }

    true
}

pub(super) fn component_value_parse_as_flex_direction(component_value: &ComponentValue) -> bool {
    // https://drafts.csswg.org/css-flexbox-1/#flex-direction-property
    // Value: row | row-reverse | column | column-reverse
    ["row", "row-reverse", "column", "column-reverse"]
        .iter()
        .any(|keyword| component_value_is_ident(Some(component_value), keyword))
}

pub(super) fn component_value_parse_as_flex_wrap(component_value: &ComponentValue) -> bool {
    // https://drafts.csswg.org/css-flexbox-1/#flex-wrap-property
    // Value: nowrap | wrap | wrap-reverse
    ["nowrap", "wrap", "wrap-reverse"]
        .iter()
        .any(|keyword| component_value_is_ident(Some(component_value), keyword))
}

pub(super) fn parse_positive_integer_component_values(component_values: &[ComponentValue]) -> bool {
    let [component_value] = strip_whitespace(component_values) else {
        return false;
    };
    component_value_parse_as_integer_in_range(component_value, 1.0, f64::INFINITY)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PositionEdge {
    Left,
    Right,
    Top,
    Bottom,
    Center,
}

pub(crate) fn parse_position_value(
    filtered_input: &[u8],
    allow_background_position_3_value_syntax: bool,
) -> CssPositionValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    if component_values_parse_as_position(&component_values, allow_background_position_3_value_syntax) {
        CssPositionValueKind::Valid
    } else {
        CssPositionValueKind::Invalid
    }
}

pub(super) fn component_values_parse_as_position(
    component_values: &[ComponentValue],
    allow_background_position_3_value_syntax: bool,
) -> bool {
    // https://www.w3.org/TR/css-values-4/#position
    // <position> = [
    //   [ left | center | right | top | bottom | <length-percentage> ]
    // |
    //   [ left | center | right ] && [ top | center | bottom ]
    // |
    //   [ left | center | right | <length-percentage> ]
    //   [ top | center | bottom | <length-percentage> ]
    // |
    //   [ [ left | right ] <length-percentage> ] &&
    //   [ [ top | bottom ] <length-percentage> ]
    // ]
    let mut parser = ComponentValueParser::new(component_values.to_vec());

    // Note: The alternatives must be attempted in this order since shorter alternatives can match a prefix of longer ones.
    if consume_position_alternative_4(&mut parser) {
        return parser_has_no_remaining_component_values(&mut parser);
    }

    if allow_background_position_3_value_syntax && consume_position_alternative_5_for_background_position(&mut parser) {
        return parser_has_no_remaining_component_values(&mut parser);
    }

    if consume_position_alternative_3(&mut parser) {
        return parser_has_no_remaining_component_values(&mut parser);
    }

    if consume_position_alternative_2(&mut parser) {
        return parser_has_no_remaining_component_values(&mut parser);
    }

    consume_position_alternative_1(&mut parser) && parser_has_no_remaining_component_values(&mut parser)
}

pub(super) fn parse_rust_owned_position_value(
    component_values: &[ComponentValue],
    source: &str,
    allow_background_position_3_value_syntax: bool,
) -> Option<RustOwnedResolvedPosition> {
    // Note: The alternatives must be attempted in this order since shorter alternatives can match a prefix of longer ones.
    if let Some(position) = parse_rust_owned_position_with_alternative(component_values, |parser| {
        parse_rust_owned_position_alternative_4(parser, source)
    }) {
        return Some(position);
    }

    if allow_background_position_3_value_syntax
        && let Some(position) = parse_rust_owned_position_with_alternative(component_values, |parser| {
            parse_rust_owned_position_alternative_5_for_background_position(parser, source)
        })
    {
        return Some(position);
    }

    if let Some(position) = parse_rust_owned_position_with_alternative(component_values, |parser| {
        parse_rust_owned_position_alternative_3(parser, source)
    }) {
        return Some(position);
    }

    if let Some(position) =
        parse_rust_owned_position_with_alternative(component_values, parse_rust_owned_position_alternative_2)
    {
        return Some(position);
    }

    parse_rust_owned_position_with_alternative(component_values, |parser| {
        parse_rust_owned_position_alternative_1(parser, source)
    })
}

pub(super) fn parse_rust_owned_position_with_alternative(
    component_values: &[ComponentValue],
    alternative: impl FnOnce(&mut ComponentValueParser) -> Option<RustOwnedResolvedPosition>,
) -> Option<RustOwnedResolvedPosition> {
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    let position = alternative(&mut parser)?;
    parser_has_no_remaining_component_values(&mut parser).then_some(position)
}

pub(super) fn parse_rust_owned_position_alternative_1(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedResolvedPosition> {
    let start = parser.index;

    // [ left | center | right | top | bottom | <length-percentage> ]
    if let Some(edge) = consume_position_edge(parser) {
        if is_horizontal_position_edge(edge, false) {
            return Some(RustOwnedResolvedPosition {
                x: rust_owned_position_edge_component(edge),
                y: rust_owned_position_edge_component(PositionEdge::Center),
            });
        }

        if is_vertical_position_edge(edge, false) {
            return Some(RustOwnedResolvedPosition {
                x: rust_owned_position_edge_component(PositionEdge::Center),
                y: rust_owned_position_edge_component(edge),
            });
        }

        return Some(RustOwnedResolvedPosition {
            x: rust_owned_position_edge_component(PositionEdge::Center),
            y: rust_owned_position_edge_component(PositionEdge::Center),
        });
    }

    if let Some(offset) = consume_rust_owned_length_percentage_component_value(parser, source) {
        return Some(RustOwnedResolvedPosition {
            x: RustOwnedPositionComponent {
                edge: None,
                offset: Some(offset),
            },
            y: rust_owned_position_edge_component(PositionEdge::Center),
        });
    }

    parser.index = start;
    None
}

pub(super) fn parse_rust_owned_position_alternative_2(
    parser: &mut ComponentValueParser,
) -> Option<RustOwnedResolvedPosition> {
    let start = parser.index;

    // [ left | center | right ] && [ top | center | bottom ]
    let Some(mut first_edge) = consume_position_edge(parser) else {
        parser.index = start;
        return None;
    };

    let Some(mut second_edge) = consume_position_edge(parser) else {
        parser.index = start;
        return None;
    };

    // If 'left' or 'right' is given, that position is X and the other is Y.
    // Conversely -
    // If 'top' or 'bottom' is given, that position is Y and the other is X.
    if is_vertical_position_edge(first_edge, false) || is_horizontal_position_edge(second_edge, false) {
        std::mem::swap(&mut first_edge, &mut second_edge);
    }

    // [ left | center | right ] [ top | bottom | center ]
    if is_horizontal_position_edge(first_edge, true) && is_vertical_position_edge(second_edge, true) {
        return Some(RustOwnedResolvedPosition {
            x: rust_owned_position_edge_component(first_edge),
            y: rust_owned_position_edge_component(second_edge),
        });
    }

    parser.index = start;
    None
}

pub(super) fn parse_rust_owned_position_alternative_3(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedResolvedPosition> {
    let start = parser.index;

    // [ left | center | right | <length-percentage> ]
    let Some(x) = consume_rust_owned_position_or_length(parser, source, PositionAxis::Horizontal) else {
        parser.index = start;
        return None;
    };

    // [ top | center | bottom | <length-percentage> ]
    let Some(y) = consume_rust_owned_position_or_length(parser, source, PositionAxis::Vertical) else {
        parser.index = start;
        return None;
    };

    Some(RustOwnedResolvedPosition { x, y })
}

pub(super) fn parse_rust_owned_position_alternative_4(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedResolvedPosition> {
    let start = parser.index;

    // [ [ left | right ] <length-percentage> ] &&
    // [ [ top | bottom ] <length-percentage> ]
    let Some(group1) = consume_rust_owned_position_and_length(parser, source) else {
        parser.index = start;
        return None;
    };

    let Some(group2) = consume_rust_owned_position_and_length(parser, source) else {
        parser.index = start;
        return None;
    };

    if is_horizontal_position_edge(group1.edge?, false) && is_vertical_position_edge(group2.edge?, false) {
        return Some(RustOwnedResolvedPosition { x: group1, y: group2 });
    }

    if is_vertical_position_edge(group1.edge?, false) && is_horizontal_position_edge(group2.edge?, false) {
        return Some(RustOwnedResolvedPosition { x: group2, y: group1 });
    }

    parser.index = start;
    None
}

pub(super) fn parse_rust_owned_position_alternative_5_for_background_position(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedResolvedPosition> {
    let start = parser.index;

    // The extra 3-value syntax that's allowed for background-position:
    // [ center | [ left | right ] <length-percentage>? ] &&
    // [ center | [ top | bottom ] <length-percentage>? ]
    let Some(mut group1) = consume_rust_owned_position_and_maybe_length(parser, source) else {
        parser.index = start;
        return None;
    };

    let Some(mut group2) = consume_rust_owned_position_and_maybe_length(parser, source) else {
        parser.index = start;
        return None;
    };

    if group1.offset.is_some() == group2.offset.is_some() {
        parser.index = start;
        return None;
    }

    if is_vertical_position_edge(group1.edge?, false) || is_horizontal_position_edge(group2.edge?, false) {
        std::mem::swap(&mut group1, &mut group2);
    }

    if !is_horizontal_position_edge(group1.edge?, true) || !is_vertical_position_edge(group2.edge?, true) {
        parser.index = start;
        return None;
    }

    Some(RustOwnedResolvedPosition {
        x: rust_owned_position_component_for_background_position(group1),
        y: rust_owned_position_component_for_background_position(group2),
    })
}

pub(super) fn parser_has_no_remaining_component_values(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    !parser.has_next_component_value()
}

pub(super) fn consume_position_alternative_1(parser: &mut ComponentValueParser) -> bool {
    let start = parser.index;

    // [ left | center | right | top | bottom | <length-percentage> ]
    if consume_position_edge(parser).is_some() || consume_length_percentage_component_value(parser) {
        return true;
    }

    parser.index = start;
    false
}

pub(super) fn consume_position_alternative_2(parser: &mut ComponentValueParser) -> bool {
    let start = parser.index;

    // [ left | center | right ] && [ top | center | bottom ]
    let Some(mut first_edge) = consume_position_edge(parser) else {
        parser.index = start;
        return false;
    };

    let Some(mut second_edge) = consume_position_edge(parser) else {
        parser.index = start;
        return false;
    };

    // If 'left' or 'right' is given, that position is X and the other is Y.
    // Conversely -
    // If 'top' or 'bottom' is given, that position is Y and the other is X.
    if is_vertical_position_edge(first_edge, false) || is_horizontal_position_edge(second_edge, false) {
        std::mem::swap(&mut first_edge, &mut second_edge);
    }

    // [ left | center | right ] [ top | bottom | center ]
    if is_horizontal_position_edge(first_edge, true) && is_vertical_position_edge(second_edge, true) {
        return true;
    }

    parser.index = start;
    false
}

pub(super) fn consume_position_alternative_3(parser: &mut ComponentValueParser) -> bool {
    let start = parser.index;

    // [ left | center | right | <length-percentage> ]
    if !consume_position_or_length(parser, PositionAxis::Horizontal) {
        parser.index = start;
        return false;
    }

    // [ top | center | bottom | <length-percentage> ]
    if !consume_position_or_length(parser, PositionAxis::Vertical) {
        parser.index = start;
        return false;
    }

    true
}

pub(super) fn consume_position_alternative_4(parser: &mut ComponentValueParser) -> bool {
    let start = parser.index;

    // [ [ left | right ] <length-percentage> ] &&
    // [ [ top | bottom ] <length-percentage> ]
    let Some(group1) = consume_position_and_length(parser) else {
        parser.index = start;
        return false;
    };

    let Some(group2) = consume_position_and_length(parser) else {
        parser.index = start;
        return false;
    };

    // [ [ left | right ] <length-percentage> ] [ [ top | bottom ] <length-percentage> ]
    if is_horizontal_position_edge(group1, false) && is_vertical_position_edge(group2, false) {
        return true;
    }

    // [ [ top | bottom ] <length-percentage> ] [ [ left | right ] <length-percentage> ]
    if is_vertical_position_edge(group1, false) && is_horizontal_position_edge(group2, false) {
        return true;
    }

    parser.index = start;
    false
}

pub(super) fn consume_position_alternative_5_for_background_position(parser: &mut ComponentValueParser) -> bool {
    let start = parser.index;

    // The extra 3-value syntax that's allowed for background-position:
    // [ center | [ left | right ] <length-percentage>? ] &&
    // [ center | [ top | bottom ] <length-percentage>? ]
    let Some(mut group1) = consume_position_and_maybe_length(parser) else {
        parser.index = start;
        return false;
    };

    let Some(mut group2) = consume_position_and_maybe_length(parser) else {
        parser.index = start;
        return false;
    };

    // 2-value or 4-value if both <length-percentage>s are present or missing.
    if group1.has_length == group2.has_length {
        parser.index = start;
        return false;
    }

    // If 'left' or 'right' is given, that position is X and the other is Y.
    // Conversely -
    // If 'top' or 'bottom' is given, that position is Y and the other is X.
    if is_vertical_position_edge(group1.edge, false) || is_horizontal_position_edge(group2.edge, false) {
        std::mem::swap(&mut group1, &mut group2);
    }

    // [ center | [ left | right ] ]
    if !is_horizontal_position_edge(group1.edge, true) {
        parser.index = start;
        return false;
    }

    // [ center | [ top | bottom ] ]
    if !is_vertical_position_edge(group2.edge, true) {
        parser.index = start;
        return false;
    }

    true
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PositionAxis {
    Horizontal,
    Vertical,
}

pub(super) fn consume_rust_owned_position_or_length(
    parser: &mut ComponentValueParser,
    source: &str,
    axis: PositionAxis,
) -> Option<RustOwnedPositionComponent> {
    let start = parser.index;

    if let Some(edge) = consume_position_edge(parser) {
        let valid = match axis {
            PositionAxis::Horizontal => is_horizontal_position_edge(edge, true),
            PositionAxis::Vertical => is_vertical_position_edge(edge, true),
        };
        if valid {
            return Some(rust_owned_position_edge_component(edge));
        }

        parser.index = start;
        return None;
    }

    consume_rust_owned_length_percentage_component_value(parser, source).map(|offset| RustOwnedPositionComponent {
        edge: None,
        offset: Some(offset),
    })
}

pub(super) fn consume_rust_owned_position_and_length(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedPositionComponent> {
    let start = parser.index;

    let Some(edge) = consume_position_edge(parser) else {
        parser.index = start;
        return None;
    };

    let Some(offset) = consume_rust_owned_length_percentage_component_value(parser, source) else {
        parser.index = start;
        return None;
    };

    Some(RustOwnedPositionComponent {
        edge: Some(edge),
        offset: Some(offset),
    })
}

pub(super) fn consume_rust_owned_position_and_maybe_length(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedPositionComponent> {
    let start = parser.index;

    let Some(edge) = consume_position_edge(parser) else {
        parser.index = start;
        return None;
    };

    let offset = consume_rust_owned_length_percentage_component_value(parser, source);
    if offset.is_some() && edge == PositionEdge::Center {
        parser.index = start;
        return None;
    }

    Some(RustOwnedPositionComponent {
        edge: Some(edge),
        offset,
    })
}

pub(super) fn consume_rust_owned_length_percentage_component_value(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    if !component_value_parse_as_length_percentage(component_value) {
        return None;
    }

    let value = component_value_parse_as_nested_length_percentage(component_value, source)?;
    parser.index += 1;
    Some(value)
}

pub(super) fn rust_owned_position_component_for_background_position(
    component: RustOwnedPositionComponent,
) -> RustOwnedPositionComponent {
    if component.edge == Some(PositionEdge::Center) {
        return rust_owned_position_edge_component(PositionEdge::Center);
    }

    component
}

pub(super) fn rust_owned_position_edge_component(edge: PositionEdge) -> RustOwnedPositionComponent {
    RustOwnedPositionComponent {
        edge: Some(edge),
        offset: None,
    }
}

pub(super) fn consume_position_or_length(parser: &mut ComponentValueParser, axis: PositionAxis) -> bool {
    let start = parser.index;

    if let Some(edge) = consume_position_edge(parser) {
        let valid = match axis {
            PositionAxis::Horizontal => is_horizontal_position_edge(edge, true),
            PositionAxis::Vertical => is_vertical_position_edge(edge, true),
        };
        if valid {
            return true;
        }

        parser.index = start;
        return false;
    }

    consume_length_percentage_component_value(parser)
}

pub(super) fn consume_position_and_length(parser: &mut ComponentValueParser) -> Option<PositionEdge> {
    let start = parser.index;

    let Some(position) = consume_position_edge(parser) else {
        parser.index = start;
        return None;
    };

    if !consume_length_percentage_component_value(parser) {
        parser.index = start;
        return None;
    }

    Some(position)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct PositionAndMaybeLength {
    pub(super) edge: PositionEdge,
    pub(super) has_length: bool,
}

pub(super) fn consume_position_and_maybe_length(parser: &mut ComponentValueParser) -> Option<PositionAndMaybeLength> {
    let start = parser.index;

    let Some(edge) = consume_position_edge(parser) else {
        parser.index = start;
        return None;
    };

    let has_length = consume_length_percentage_component_value(parser);
    if has_length && edge == PositionEdge::Center {
        parser.index = start;
        return None;
    }

    Some(PositionAndMaybeLength { edge, has_length })
}

pub(super) fn consume_position_edge(parser: &mut ComponentValueParser) -> Option<PositionEdge> {
    parser.discard_whitespace();
    let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    })) = parser.next_component_value()
    else {
        return None;
    };

    let edge = if value.eq_ignore_ascii_case("left") {
        PositionEdge::Left
    } else if value.eq_ignore_ascii_case("right") {
        PositionEdge::Right
    } else if value.eq_ignore_ascii_case("top") {
        PositionEdge::Top
    } else if value.eq_ignore_ascii_case("bottom") {
        PositionEdge::Bottom
    } else if value.eq_ignore_ascii_case("center") {
        PositionEdge::Center
    } else {
        return None;
    };

    parser.index += 1;
    Some(edge)
}

pub(super) fn is_horizontal_position_edge(edge: PositionEdge, accept_center: bool) -> bool {
    matches!(edge, PositionEdge::Left | PositionEdge::Right) || (accept_center && edge == PositionEdge::Center)
}

pub(super) fn is_vertical_position_edge(edge: PositionEdge, accept_center: bool) -> bool {
    matches!(edge, PositionEdge::Top | PositionEdge::Bottom) || (accept_center && edge == PositionEdge::Center)
}

pub(crate) fn parse_background_position_longhand_value(
    filtered_input: &[u8],
    is_horizontal: bool,
) -> CssPositionValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    if consume_background_position_longhand_value(&mut parser, is_horizontal)
        && parser_has_no_remaining_component_values(&mut parser)
    {
        CssPositionValueKind::Valid
    } else {
        CssPositionValueKind::Invalid
    }
}

pub(super) fn parse_rust_owned_background_position_longhand_value(
    component_values: &[ComponentValue],
    source: &str,
    is_horizontal: bool,
) -> Option<RustOwnedPositionComponent> {
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    let component = consume_rust_owned_background_position_longhand_value(&mut parser, source, is_horizontal)?;
    parser_has_no_remaining_component_values(&mut parser).then_some(component)
}

pub(super) fn consume_background_position_longhand_value(
    parser: &mut ComponentValueParser,
    is_horizontal: bool,
) -> bool {
    // https://drafts.csswg.org/css-backgrounds-4/#propdef-background-position-x
    // background-position-x = [ center | [ left | right | x-start | x-end ]? <length-percentage>? ]#
    //
    // https://drafts.csswg.org/css-backgrounds-4/#propdef-background-position-y
    // background-position-y = [ center | [ top | bottom | y-start | y-end ]? <length-percentage>? ]#
    if consume_optional_ident_matching(parser, "center") {
        return true;
    }

    let parsed_edge = if is_horizontal {
        consume_optional_ident_matching(parser, "left")
            || consume_optional_ident_matching(parser, "right")
            || consume_optional_ident_matching(parser, "x-start")
            || consume_optional_ident_matching(parser, "x-end")
    } else {
        consume_optional_ident_matching(parser, "top")
            || consume_optional_ident_matching(parser, "bottom")
            || consume_optional_ident_matching(parser, "y-start")
            || consume_optional_ident_matching(parser, "y-end")
    };

    let parsed_offset = consume_length_percentage_component_value(parser);
    parsed_edge || parsed_offset
}

pub(super) fn consume_rust_owned_background_position_longhand_value(
    parser: &mut ComponentValueParser,
    source: &str,
    is_horizontal: bool,
) -> Option<RustOwnedPositionComponent> {
    // https://drafts.csswg.org/css-backgrounds-4/#propdef-background-position-x
    // background-position-x = [ center | [ left | right | x-start | x-end ]? <length-percentage>? ]#
    //
    // https://drafts.csswg.org/css-backgrounds-4/#propdef-background-position-y
    // background-position-y = [ center | [ top | bottom | y-start | y-end ]? <length-percentage>? ]#
    if consume_optional_ident_matching(parser, "center") {
        return Some(rust_owned_position_edge_component(PositionEdge::Center));
    }

    let edge = if is_horizontal {
        if consume_optional_ident_matching(parser, "left") {
            Some(PositionEdge::Left)
        } else if consume_optional_ident_matching(parser, "right") {
            Some(PositionEdge::Right)
        } else {
            None
        }
    } else if consume_optional_ident_matching(parser, "top") {
        Some(PositionEdge::Top)
    } else if consume_optional_ident_matching(parser, "bottom") {
        Some(PositionEdge::Bottom)
    } else {
        None
    };

    let offset = consume_rust_owned_length_percentage_component_value(parser, source);
    if edge.is_none() && offset.is_none() {
        return None;
    }

    Some(RustOwnedPositionComponent { edge, offset })
}

pub(crate) fn parse_background_size_value(filtered_input: &[u8]) -> CssBackgroundSizeValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    if consume_background_size_value(&mut parser) && parser_has_no_remaining_component_values(&mut parser) {
        CssBackgroundSizeValueKind::Valid
    } else {
        CssBackgroundSizeValueKind::Invalid
    }
}

pub(super) fn consume_background_size_value(parser: &mut ComponentValueParser) -> bool {
    // https://drafts.csswg.org/css-backgrounds-3/#typedef-bg-size
    // <bg-size> = [ <length-percentage [0,∞]> | auto ]{1,2} | cover | contain
    if consume_optional_ident_matching(parser, "cover") || consume_optional_ident_matching(parser, "contain") {
        return true;
    }

    if !consume_background_size_component(parser) {
        return false;
    }

    consume_background_size_component(parser);
    true
}

pub(super) fn consume_background_size_component(parser: &mut ComponentValueParser) -> bool {
    consume_optional_ident_matching(parser, "auto") || consume_non_negative_length_percentage_component_value(parser)
}

pub(super) fn consume_non_negative_length_percentage_component_value(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };

    if component_value_parse_as_non_negative_length_percentage(component_value) {
        parser.index += 1;
        return true;
    }

    false
}

pub(crate) fn parse_repeat_style_value(filtered_input: &[u8]) -> CssRepeatStyleValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    if consume_repeat_style_value(&mut parser) && parser_has_no_remaining_component_values(&mut parser) {
        CssRepeatStyleValueKind::Valid
    } else {
        CssRepeatStyleValueKind::Invalid
    }
}

pub(super) fn consume_repeat_style_value(parser: &mut ComponentValueParser) -> bool {
    // https://drafts.csswg.org/css-backgrounds-3/#typedef-repeat-style
    // <repeat-style> = repeat-x | repeat-y | [ repeat | space | round | no-repeat ]{1,2}
    if consume_optional_ident_matching(parser, "repeat-x") || consume_optional_ident_matching(parser, "repeat-y") {
        return true;
    }

    if !consume_non_directional_repeat_style_value(parser) {
        return false;
    }

    consume_non_directional_repeat_style_value(parser);
    true
}

pub(super) fn consume_non_directional_repeat_style_value(parser: &mut ComponentValueParser) -> bool {
    consume_optional_ident_matching(parser, "repeat")
        || consume_optional_ident_matching(parser, "space")
        || consume_optional_ident_matching(parser, "round")
        || consume_optional_ident_matching(parser, "no-repeat")
}

pub(crate) fn parse_color_function_value(filtered_input: &[u8]) -> CssColorFunctionValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    let [ComponentValue::Function(function)] = component_values else {
        return CssColorFunctionValueKind::Invalid;
    };

    if component_value_parse_as_color_function(function) {
        CssColorFunctionValueKind::Valid
    } else {
        CssColorFunctionValueKind::Invalid
    }
}

pub(crate) fn parse_color_value(filtered_input: &[u8], allow_quirky_color: bool) -> CssColorValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    let [component_value] = component_values else {
        return CssColorValueKind::Invalid;
    };

    if component_value_parse_as_color_value(component_value)
        || component_value_parse_as_quirky_color(component_value, allow_quirky_color)
    {
        CssColorValueKind::Valid
    } else {
        CssColorValueKind::Invalid
    }
}

pub(crate) fn parse_simple_color_value<C>(filtered_input: &[u8], allow_quirky_color: bool, mut callback: C) -> bool
where
    C: FnMut(CssParsedColorKind, u8, u8, u8, u8, &str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    let [component_value] = component_values else {
        return false;
    };

    let Some(color) = simple_color_from_component_value(component_value, allow_quirky_color) else {
        return false;
    };

    match color {
        ParsedSimpleColor::Rgba {
            red,
            green,
            blue,
            alpha,
            name,
        } => {
            callback(CssParsedColorKind::Rgba, red, green, blue, alpha, name.unwrap_or(""));
        }
        ParsedSimpleColor::Keyword { name } => {
            callback(CssParsedColorKind::Keyword, 0, 0, 0, 0, name);
        }
    }

    true
}

pub(crate) fn parse_image_set_value(filtered_input: &[u8]) -> CssImageSetValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    let [ComponentValue::Function(function)] = component_values else {
        return CssImageSetValueKind::Invalid;
    };

    if component_value_parse_as_image_set_function(function) {
        CssImageSetValueKind::Valid
    } else {
        CssImageSetValueKind::Invalid
    }
}

pub(super) fn component_value_parse_as_image_set_function(function: &Function) -> bool {
    // https://drafts.csswg.org/css-images-4/#image-set-notation
    // image-set() = image-set( <image-set-option># )
    // <image-set-option> = [ <image> | <string> ] [ <resolution> || type(<string>) ]
    // https://compat.spec.whatwg.org/#css-%27-webkit-image-set%27-alias
    // Implementations must accept -webkit-image-set() as a parse-time alias of image-set().
    if !function.name.eq_ignore_ascii_case("image-set") && !function.name.eq_ignore_ascii_case("-webkit-image-set") {
        return false;
    }

    let Some(options) = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        component_values_parse_as_image_set_option(&component_values).then_some(())
    }) else {
        return false;
    };

    !options.is_empty()
}

pub(super) fn component_values_parse_as_image_set_option(component_values: &[ComponentValue]) -> bool {
    let component_values = strip_whitespace(component_values);
    if component_values.is_empty() {
        return false;
    }

    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();

    let Some(image) = parser.next_component_value() else {
        return false;
    };

    if component_value_parse_as_image_set_string(image)
        || component_value_parse_as_image_set_image(image)
        || component_value_parse_as_image_set_gradient(image)
    {
        parser.index += 1;
    } else {
        return false;
    }

    let mut has_resolution = false;
    let mut has_type = false;
    loop {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };

        if !has_resolution && component_value_parse_as_image_set_resolution(component_value) {
            has_resolution = true;
            parser.index += 1;
            continue;
        }

        if !has_type && component_value_parse_as_image_set_type(component_value) {
            has_type = true;
            parser.index += 1;
            continue;
        }

        return false;
    }

    true
}

pub(super) fn component_value_parse_as_image_set_resolution(component_value: &ComponentValue) -> bool {
    // https://drafts.csswg.org/css-images-4/#typedef-image-set-option
    // <image-set-option> = [ <image> | <string> ] [ <resolution> || type(<string>) ]
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) => number.value() >= 0.0 && matches!(dimension_for_unit(unit), Some(DimensionType::Resolution)),
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => is_math_function_name(&function.name),
        _ => false,
    }
}

pub(super) fn component_value_parse_as_image_set_string(component_value: &ComponentValue) -> bool {
    // https://drafts.csswg.org/css-images-4/#image-set-notation
    // "For legacy reasons, <string> can be used instead of <url>, and is
    // treated identically to url(<string>)."
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { .. },
            ..
        })
    )
}

pub(super) fn component_value_parse_as_image_set_image(component_value: &ComponentValue) -> bool {
    let mut parser = ComponentValueParser::new(vec![component_value.clone()]);
    parser.parse_a_url_function().is_some()
}

pub(super) fn component_value_parse_as_image_url(component_value: &ComponentValue) -> bool {
    let mut parser = ComponentValueParser::new(vec![component_value.clone()]);
    let Some(url) = parser.parse_a_url_function() else {
        return false;
    };

    // If the value is a 'url(..)' parse as image, but if it is just a reference 'url(#xx)', leave it alone,
    // so we can parse as URL further on. These URLs are used as references inside SVG documents for masks.
    // FIXME: Remove this special case once mask-image accepts `<image>`.
    !url.url.starts_with('#')
}

pub(super) fn component_values_parse_as_fragment_url(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut component_value_parser = ComponentValueParser::new(component_values);
    component_value_parser.discard_whitespace();
    let Some(url) = component_value_parser.parse_a_url_function() else {
        return false;
    };

    url.url.starts_with('#')
}

pub(super) fn component_value_parse_as_image_gradient(component_value: &ComponentValue) -> bool {
    component_value_parse_as_image_set_gradient(component_value)
}

pub(super) fn component_value_parse_as_image_gradient_value(
    component_value: &ComponentValue,
) -> Option<RustOwnedGradient> {
    let ComponentValue::Function(function) = component_value else {
        return None;
    };

    parse_gradient_function(function)
}

pub(super) fn component_value_parse_as_image_set_gradient(component_value: &ComponentValue) -> bool {
    let ComponentValue::Function(function) = component_value else {
        return false;
    };

    parse_gradient_function(function).is_some()
}

fn parse_gradient_function(function: &Function) -> Option<RustOwnedGradient> {
    let name = function.name.to_ascii_lowercase();
    let (name, is_webkit_prefixed) = name
        .strip_prefix("-webkit-")
        .map(|name| (name, true))
        .unwrap_or((&name, false));
    let (name, is_repeating) = if let Some(name) = name.strip_prefix("repeating-") {
        (name, true)
    } else {
        (name, false)
    };

    let kind = match name {
        "linear-gradient" => RustOwnedGradientKind::Linear,
        "radial-gradient" if !is_webkit_prefixed => RustOwnedGradientKind::Radial,
        "conic-gradient" if !is_webkit_prefixed => RustOwnedGradientKind::Conic,
        _ => return None,
    };

    let valid = match kind {
        RustOwnedGradientKind::Linear => parse_linear_gradient_function(function, is_webkit_prefixed),
        RustOwnedGradientKind::Radial => parse_radial_gradient_function(function),
        RustOwnedGradientKind::Conic => parse_conic_gradient_function(function),
    };
    if !valid {
        return None;
    }

    let groups = split_component_values_on_comma(&function.value)
        .into_iter()
        .map(|group| group.to_vec())
        .collect();

    Some(RustOwnedGradient {
        kind,
        is_repeating,
        is_webkit_prefixed,
        groups,
    })
}

fn parse_linear_gradient_function(function: &Function, is_webkit_prefixed: bool) -> bool {
    // https://drafts.csswg.org/css-images-4/#typedef-linear-gradient-syntax
    // <linear-gradient-syntax> = [ [ <angle> | <zero> | to <side-or-corner> ] || <color-interpolation-method> ]? , <color-stop-list>
    let groups = split_component_values_on_comma(&function.value);
    if groups.is_empty() {
        return false;
    }

    if parse_linear_color_stop_list(&groups) {
        return true;
    }

    groups.len() > 1
        && component_values_parse_as_linear_gradient_header(groups[0], is_webkit_prefixed)
        && parse_linear_color_stop_list(&groups[1..])
}

fn parse_conic_gradient_function(function: &Function) -> bool {
    // https://drafts.csswg.org/css-images-4/#typedef-conic-gradient-syntax
    // conic-gradient( [ [ [ from [ <angle> | <zero> ] ]? [ at <position> ]? ] || <color-interpolation-method> ]? , <angular-color-stop-list> )
    let groups = split_component_values_on_comma(&function.value);
    if groups.is_empty() {
        return false;
    }

    if parse_angular_color_stop_list(&groups) {
        return true;
    }

    groups.len() > 1
        && component_values_parse_as_conic_gradient_header(groups[0])
        && parse_angular_color_stop_list(&groups[1..])
}

fn parse_radial_gradient_function(function: &Function) -> bool {
    // https://drafts.csswg.org/css-images-4/#typedef-radial-gradient-syntax
    // <radial-gradient-syntax> = [ [ [ <radial-shape> || <radial-size> ]? [ at <position> ]? ] || <color-interpolation-method> ]? , <color-stop-list>
    let groups = split_component_values_on_comma(&function.value);
    if groups.is_empty() {
        return false;
    }

    if parse_linear_color_stop_list(&groups) {
        return true;
    }

    groups.len() > 1
        && component_values_parse_as_radial_gradient_header(groups[0])
        && parse_linear_color_stop_list(&groups[1..])
}

fn parse_linear_color_stop_list(groups: &[&[ComponentValue]]) -> bool {
    // https://drafts.csswg.org/css-images-4/#color-stop-syntax
    // <color-stop-list> = <linear-color-stop> , [ <linear-color-hint>? , <linear-color-stop> ]#
    parse_color_stop_list(groups, component_value_parse_as_length_percentage)
}

fn parse_angular_color_stop_list(groups: &[&[ComponentValue]]) -> bool {
    // https://drafts.csswg.org/css-images-4/#color-stop-syntax
    // <angular-color-stop-list> = <angular-color-stop> , [ <angular-color-hint>? , <angular-color-stop> ]#
    parse_color_stop_list(groups, component_value_parse_as_angle_percentage_or_zero)
}

fn parse_color_stop_list(groups: &[&[ComponentValue]], parse_position: fn(&ComponentValue) -> bool) -> bool {
    if groups.is_empty() || !component_values_parse_as_color_stop(groups[0], parse_position) {
        return false;
    }

    let mut index = 1;
    while index < groups.len() {
        if component_values_parse_as_color_stop(groups[index], parse_position) {
            index += 1;
            continue;
        }

        if index + 1 < groups.len()
            && component_values_parse_as_color_hint(groups[index], parse_position)
            && component_values_parse_as_color_stop(groups[index + 1], parse_position)
        {
            index += 2;
            continue;
        }

        return false;
    }

    true
}

fn component_values_parse_as_color_hint(
    component_values: &[ComponentValue],
    parse_position: fn(&ComponentValue) -> bool,
) -> bool {
    let component_values = remove_whitespace_component_values(component_values);
    let component_values = component_values.as_slice();
    matches!(component_values, [position] if parse_position(position))
}

fn component_values_parse_as_color_stop(
    component_values: &[ComponentValue],
    parse_position: fn(&ComponentValue) -> bool,
) -> bool {
    let component_values = remove_whitespace_component_values(component_values);
    let component_values = component_values.as_slice();
    let Some(first) = component_values.first() else {
        return false;
    };

    if parse_position(first) {
        return matches!(component_values, [_, color] if component_value_parse_as_color_value(color));
    }

    if !component_value_parse_as_color_value(first) {
        return false;
    }

    component_values[1..].len() <= 2 && component_values[1..].iter().all(parse_position)
}

fn component_value_parse_as_angle_percentage_or_zero(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { .. },
            ..
        })
    ) || component_value_parse_as_angle_or_zero(component_value)
}

fn component_value_parse_as_angle_or_zero(component_value: &ComponentValue) -> bool {
    match component_value {
        // https://drafts.csswg.org/css-values-4/#zero-value
        // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() == 0.0,
        ComponentValue::Function(function) => is_math_function_name(&function.name),
        _ => component_value_parse_as_angle(component_value),
    }
}

fn component_values_parse_as_linear_gradient_header(
    component_values: &[ComponentValue],
    is_webkit_prefixed: bool,
) -> bool {
    let component_values = remove_whitespace_component_values(component_values);
    let component_values = component_values.as_slice();
    if component_values.is_empty() {
        return false;
    }

    component_values_parse_as_linear_gradient_direction(component_values, is_webkit_prefixed)
        || component_values_parse_as_color_interpolation_method(component_values)
        || component_values_parse_as_linear_gradient_header_pair(
            component_values,
            |component_values| {
                component_values_parse_as_linear_gradient_direction(component_values, is_webkit_prefixed)
            },
            component_values_parse_as_color_interpolation_method,
        )
        || component_values_parse_as_linear_gradient_header_pair(
            component_values,
            component_values_parse_as_color_interpolation_method,
            |component_values| {
                component_values_parse_as_linear_gradient_direction(component_values, is_webkit_prefixed)
            },
        )
}

fn component_values_parse_as_linear_gradient_header_pair(
    component_values: &[ComponentValue],
    parse_a: impl Fn(&[ComponentValue]) -> bool,
    parse_b: impl Fn(&[ComponentValue]) -> bool,
) -> bool {
    (1..component_values.len()).any(|split| parse_a(&component_values[..split]) && parse_b(&component_values[split..]))
}

fn component_values_parse_as_linear_gradient_direction(
    component_values: &[ComponentValue],
    is_webkit_prefixed: bool,
) -> bool {
    let component_values = remove_whitespace_component_values(component_values);
    let component_values = component_values.as_slice();
    if matches!(component_values, [component_value] if component_value_parse_as_angle_or_zero(component_value)) {
        return true;
    }

    let sides = if is_webkit_prefixed {
        component_values
    } else {
        let [to, sides @ ..] = component_values else {
            return false;
        };
        if !component_value_is_ident(Some(to), "to") {
            return false;
        }
        sides
    };

    matches!(sides, [side] if component_value_parse_as_gradient_side(side).is_some())
        || matches!(sides, [side_a, side_b] if gradient_sides_form_corner(side_a, side_b))
}

fn component_value_parse_as_gradient_side(component_value: &ComponentValue) -> Option<GradientSide> {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    }) = component_value
    else {
        return None;
    };

    if value.eq_ignore_ascii_case("top") {
        return Some(GradientSide::Top);
    }
    if value.eq_ignore_ascii_case("bottom") {
        return Some(GradientSide::Bottom);
    }
    if value.eq_ignore_ascii_case("left") {
        return Some(GradientSide::Left);
    }
    if value.eq_ignore_ascii_case("right") {
        return Some(GradientSide::Right);
    }
    None
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum GradientSide {
    Top,
    Bottom,
    Left,
    Right,
}

fn gradient_sides_form_corner(side_a: &ComponentValue, side_b: &ComponentValue) -> bool {
    matches!(
        (
            component_value_parse_as_gradient_side(side_a),
            component_value_parse_as_gradient_side(side_b),
        ),
        (
            Some(GradientSide::Top | GradientSide::Bottom),
            Some(GradientSide::Left | GradientSide::Right)
        ) | (
            Some(GradientSide::Left | GradientSide::Right),
            Some(GradientSide::Top | GradientSide::Bottom)
        )
    )
}

fn component_values_parse_as_conic_gradient_header(component_values: &[ComponentValue]) -> bool {
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    let mut has_from_angle = false;
    let mut has_at_position = false;
    let mut has_interpolation_method = false;

    while parser.has_next_component_value() {
        if parser.consume_ident_matching("from") {
            if has_from_angle || has_at_position || !consume_angle_or_zero(&mut parser) {
                return false;
            }
            has_from_angle = true;
            continue;
        }

        if parser.consume_ident_matching("at") {
            if has_at_position || !consume_position_until_color_interpolation_method(&mut parser) {
                return false;
            }
            has_at_position = true;
            continue;
        }

        if !has_interpolation_method && consume_color_interpolation_method(&mut parser) {
            has_interpolation_method = true;
            continue;
        }

        return false;
    }

    has_from_angle || has_at_position || has_interpolation_method
}

fn component_values_parse_as_radial_gradient_header(component_values: &[ComponentValue]) -> bool {
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    let mut shape = None;
    let mut radial_size_component_count = 0;
    let mut first_radial_size_component_is_extent = false;
    let mut has_at_position = false;
    let mut has_interpolation_method = false;

    if consume_color_interpolation_method(&mut parser) {
        has_interpolation_method = true;
    }

    for _ in 0..2 {
        if shape.is_none()
            && let Some(parsed_shape) = consume_radial_gradient_shape(&mut parser)
        {
            shape = Some(parsed_shape);
            continue;
        }

        if radial_size_component_count < 2
            && let Some(component_is_extent) = consume_radial_gradient_size_component(&mut parser)
        {
            if radial_size_component_count == 0 {
                first_radial_size_component_is_extent = component_is_extent;
            }
            radial_size_component_count += 1;
            continue;
        }

        break;
    }

    if shape.is_none()
        && let Some(parsed_shape) = consume_radial_gradient_shape(&mut parser)
    {
        shape = Some(parsed_shape);
    }

    if shape.is_some()
        && radial_size_component_count == 1
        && let Some(_) = consume_radial_gradient_size_component(&mut parser)
    {
        radial_size_component_count += 1;
    }

    parser.discard_whitespace();
    if parser.consume_ident_matching("at") {
        if !consume_position_until_color_interpolation_method(&mut parser) {
            return false;
        }
        has_at_position = true;
    }

    if !has_interpolation_method && consume_color_interpolation_method(&mut parser) {
        has_interpolation_method = true;
    }

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    (shape.is_some() || radial_size_component_count > 0 || has_at_position || has_interpolation_method)
        && radial_gradient_shape_and_size_match(
            shape,
            radial_size_component_count,
            first_radial_size_component_is_extent,
        )
}

fn radial_gradient_shape_and_size_match(
    shape: Option<RadialGradientShape>,
    radial_size_component_count: usize,
    first_radial_size_component_is_extent: bool,
) -> bool {
    match shape {
        Some(RadialGradientShape::Circle) => radial_size_component_count <= 1,
        Some(RadialGradientShape::Ellipse) => {
            radial_size_component_count == 0
                || radial_size_component_count == 2
                || first_radial_size_component_is_extent
        }
        None => true,
    }
}

#[derive(Clone, Copy)]
enum RadialGradientShape {
    Circle,
    Ellipse,
}

fn consume_radial_gradient_shape(parser: &mut ComponentValueParser) -> Option<RadialGradientShape> {
    parser.discard_whitespace();
    if parser.consume_ident_matching("circle") {
        return Some(RadialGradientShape::Circle);
    }
    if parser.consume_ident_matching("ellipse") {
        return Some(RadialGradientShape::Ellipse);
    }
    None
}

fn consume_radial_gradient_size_component(parser: &mut ComponentValueParser) -> Option<bool> {
    // https://drafts.csswg.org/css-images-4/#radial-size
    // <radial-size> = <radial-extent>{1,2} | <length-percentage [0,∞]>{1,2}
    // <radial-extent> = closest-corner | closest-side | farthest-corner | farthest-side
    // AD-HOC: This accepts mixed <radial-extent> and <length-percentage> pairs for compatibility.
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;

    if component_value_parse_as_radial_extent(component_value).is_some() {
        parser.index += 1;
        return Some(true);
    }

    if component_value_parse_as_non_negative_length_percentage(component_value) {
        parser.index += 1;
        return Some(false);
    }

    None
}

fn consume_angle_or_zero(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };
    if !component_value_parse_as_angle_or_zero(component_value) {
        return false;
    }
    parser.index += 1;
    true
}

fn consume_color_interpolation_method(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let remaining = parser.remaining_component_values();
    for length in (1..=remaining.len()).rev() {
        if component_values_parse_as_color_interpolation_method(&remaining[..length]) {
            parser.index += length;
            return true;
        }
    }
    false
}

fn consume_position_until_color_interpolation_method(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let remaining = parser.remaining_component_values();
    for length in (1..=remaining.len()).rev() {
        if component_values_parse_as_position(&remaining[..length], false) {
            parser.index += length;
            return true;
        }
    }
    false
}

pub(super) fn component_value_parse_as_image_set_type(component_value: &ComponentValue) -> bool {
    image_set_type_value(component_value).is_some()
}

pub(super) fn image_set_type_value(component_value: &ComponentValue) -> Option<String> {
    let ComponentValue::Function(function) = component_value else {
        return None;
    };

    if !function.name.eq_ignore_ascii_case("type") {
        return None;
    }

    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { value },
            ..
        }),
    ] = strip_whitespace(&function.value)
    else {
        return None;
    };

    Some(value.clone())
}

pub(super) fn component_value_parse_as_color_function(function: &Function) -> bool {
    match function.name.to_ascii_lowercase().as_str() {
        "rgb" | "rgba" => component_values_parse_as_rgb_color_function(&function.value),
        "hsl" | "hsla" => component_values_parse_as_hsl_color_function(&function.value),
        "hwb" => component_values_parse_as_hwb_color_function(&function.value),
        "lab" | "oklab" => component_values_parse_as_lab_like_color_function(&function.value),
        "lch" | "oklch" => component_values_parse_as_lch_like_color_function(&function.value),
        "color" => component_values_parse_as_color_color_function(&function.value),
        "color-mix" => component_values_parse_as_color_mix_function(&function.value),
        "light-dark" => component_values_parse_as_light_dark_color_function(&function.value),
        _ => false,
    }
}

pub(super) fn component_values_parse_as_rgb_color_function(component_values: &[ComponentValue]) -> bool {
    // https://www.w3.org/TR/css-color-4/#funcdef-rgb
    // rgb() = [ <legacy-rgb-syntax> | <modern-rgb-syntax> ]
    // rgba() = [ <legacy-rgba-syntax> | <modern-rgba-syntax> ]
    // <legacy-rgb-syntax> = rgb( <percentage>#{3} , <alpha-value>? ) |
    //                       rgb( <number>#{3} , <alpha-value>? )
    // <legacy-rgba-syntax> = rgba( <percentage>#{3} , <alpha-value>? ) |
    //                        rgba( <number>#{3} , <alpha-value>? )
    // <modern-rgb-syntax> = rgb(
    //     [ <number> | <percentage> | none]{3}
    //     [ / [<alpha-value> | none] ]?  )
    // <modern-rgba-syntax> = rgba(
    //     [ <number> | <percentage> | none]{3}
    //     [ / [<alpha-value> | none] ]?  )
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();
    let first_channel_is_none = matches!(
        parser.next_component_value(),
        Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) if value.eq_ignore_ascii_case("none")
    );
    let Some(first_channel_is_percentage) =
        consume_number_percentage_none_component_value_with_percentage_kind(&mut parser)
    else {
        return false;
    };
    parser.discard_whitespace();

    if parser.consume_a_comma() {
        if first_channel_is_none {
            return false;
        }
        return component_values_parse_as_legacy_rgb_color_function_after_first_comma(
            &mut parser,
            first_channel_is_percentage,
        );
    }

    for _ in 0..2 {
        if !consume_number_percentage_none_component_value(&mut parser) {
            return false;
        }
    }

    consume_optional_solidus_and_alpha_value(&mut parser) && !parser.has_next_component_value()
}

pub(super) fn component_values_parse_as_legacy_rgb_color_function_after_first_comma(
    parser: &mut ComponentValueParser,
    first_channel_is_percentage: Option<bool>,
) -> bool {
    let mut channel_is_percentage = Vec::new();
    channel_is_percentage.push(first_channel_is_percentage);
    parser.discard_whitespace();
    let Some(is_green_percentage) = consume_color_number_percentage_component_value_with_percentage_kind(parser) else {
        return false;
    };
    channel_is_percentage.push(is_green_percentage);

    for _ in 0..1 {
        parser.discard_whitespace();
        if !parser.consume_a_comma() {
            return false;
        }
        parser.discard_whitespace();
        let Some(is_percentage) = consume_color_number_percentage_component_value_with_percentage_kind(parser) else {
            return false;
        };
        channel_is_percentage.push(is_percentage);
    }

    parser.discard_whitespace();
    if parser.consume_a_comma() && !consume_number_percentage_component_value(parser) {
        return false;
    }

    let mut concrete_channel_kinds = channel_is_percentage.iter().flatten();
    let first_concrete_channel_kind = concrete_channel_kinds.next().copied();
    !parser.has_next_component_value()
        && concrete_channel_kinds.all(|value| Some(*value) == first_concrete_channel_kind)
}

pub(super) fn component_values_parse_as_hsl_color_function(component_values: &[ComponentValue]) -> bool {
    // https://www.w3.org/TR/css-color-4/#funcdef-hsl
    // hsl() = [ <legacy-hsl-syntax> | <modern-hsl-syntax> ]
    // hsla() = [ <legacy-hsla-syntax> | <modern-hsla-syntax> ]
    // <modern-hsl-syntax> = hsl(
    //     [<hue> | none]
    //     [<percentage> | <number> | none]
    //     [<percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )
    // <modern-hsla-syntax> = hsla(
    //     [<hue> | none]
    //     [<percentage> | <number> | none]
    //     [<percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )
    // <legacy-hsl-syntax> = hsl( <hue>, <percentage>, <percentage>, <alpha-value>? )
    // <legacy-hsla-syntax> = hsla( <hue>, <percentage>, <percentage>, <alpha-value>? )
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();
    if !consume_hue_none_component_value(&mut parser) {
        return false;
    }
    parser.discard_whitespace();

    if parser.consume_a_comma() {
        for component_count in 0..2 {
            parser.discard_whitespace();
            if !consume_percentage_component_value(&mut parser) {
                return false;
            }
            parser.discard_whitespace();
            if component_count == 0 && !parser.consume_a_comma() {
                return false;
            }
        }
        parser.discard_whitespace();
        if parser.consume_a_comma() && !consume_number_percentage_component_value(&mut parser) {
            return false;
        }
        return !parser.has_next_component_value();
    }

    for _ in 0..2 {
        if !consume_number_percentage_none_component_value(&mut parser) {
            return false;
        }
    }

    consume_optional_solidus_and_alpha_value(&mut parser) && !parser.has_next_component_value()
}

pub(super) fn component_values_parse_as_hwb_color_function(component_values: &[ComponentValue]) -> bool {
    // https://www.w3.org/TR/css-color-4/#funcdef-hwb
    // hwb() = hwb(
    //     [<hue> | none]
    //     [<percentage> | <number> | none]
    //     [<percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    if !consume_hue_none_component_value(&mut parser) {
        return false;
    }
    if !consume_number_percentage_none_component_value(&mut parser) {
        return false;
    }
    if !consume_number_percentage_none_component_value(&mut parser) {
        return false;
    }

    consume_optional_solidus_and_alpha_value(&mut parser) && !parser.has_next_component_value()
}

pub(super) fn component_values_parse_as_lab_like_color_function(component_values: &[ComponentValue]) -> bool {
    // https://www.w3.org/TR/css-color-4/#funcdef-lab
    // lab() = lab( [<percentage> | <number> | none]
    //      [ <percentage> | <number> | none]
    //      [ <percentage> | <number> | none]
    //      [ / [<alpha-value> | none] ]? )
    // https://www.w3.org/TR/css-color-4/#funcdef-oklab
    // oklab() = oklab( [ <percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ / [<alpha-value> | none] ]? )
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    for _ in 0..3 {
        if !consume_number_percentage_none_component_value(&mut parser) {
            return false;
        }
    }

    consume_optional_solidus_and_alpha_value(&mut parser) && !parser.has_next_component_value()
}

pub(super) fn component_values_parse_as_lch_like_color_function(component_values: &[ComponentValue]) -> bool {
    // https://www.w3.org/TR/css-color-4/#funcdef-lch
    // lch() = lch( [<percentage> | <number> | none]
    //      [ <percentage> | <number> | none]
    //      [ <hue> | none]
    //      [ / [<alpha-value> | none] ]? )
    // https://www.w3.org/TR/css-color-4/#funcdef-oklch
    // oklch() = oklch( [ <percentage> | <number> | none]
    //     [ <percentage> | <number> | none]
    //     [ <hue> | none]
    //     [ / [<alpha-value> | none] ]? )
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    if !consume_number_percentage_none_component_value(&mut parser) {
        return false;
    }
    if !consume_number_percentage_none_component_value(&mut parser) {
        return false;
    }
    if !consume_hue_none_component_value(&mut parser) {
        return false;
    }

    consume_optional_solidus_and_alpha_value(&mut parser) && !parser.has_next_component_value()
}

pub(super) fn component_values_parse_as_color_color_function(component_values: &[ComponentValue]) -> bool {
    // https://www.w3.org/TR/css-color-4/#funcdef-color
    // color() = color( <colorspace-params> [ / [ <alpha-value> | none ] ]? )
    //     <colorspace-params> = [ <predefined-rgb-params> | <xyz-params>]
    //     <predefined-rgb-params> = <predefined-rgb> [ <number> | <percentage> | none ]{3}
    //     <predefined-rgb> = srgb | srgb-linear | display-p3 | a98-rgb | prophoto-rgb | rec2020
    //     <xyz-params> = <xyz-space> [ <number> | <percentage> | none ]{3}
    //     <xyz-space> = xyz | xyz-d50 | xyz-d65
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();
    let Some(color_space) = parser.consume_an_ident() else {
        return false;
    };
    if !is_color_function_color_space(&color_space) {
        return false;
    }

    for _ in 0..3 {
        if !consume_number_percentage_none_component_value(&mut parser) {
            return false;
        }
    }

    consume_optional_solidus_and_alpha_value(&mut parser) && !parser.has_next_component_value()
}

pub(super) fn component_values_parse_as_color_mix_function(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-color-5/#color-mix
    // color-mix() = color-mix( <color-interpolation-method>? , [ <color> && <percentage [0,100]>? ]#)
    // FIXME: Update color-mix to accept 1+ colors instead of exactly 2.
    let Some(groups) = parse_comma_separated_component_values(component_values.to_vec(), |component_values| {
        Some(component_values.to_vec())
    }) else {
        return false;
    };

    if groups.len() != 2 && groups.len() != 3 {
        return false;
    }

    let color_groups = if groups.len() == 3 {
        if !component_values_parse_as_color_interpolation_method(&groups[0]) {
            return false;
        }
        &groups[1..]
    } else {
        &groups[..]
    };

    color_groups
        .iter()
        .all(|group| component_values_parse_as_color_mix_component(group))
}

pub(super) fn component_values_parse_as_light_dark_color_function(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-color-5/#funcdef-light-dark
    // light-dark() = light-dark( <color>, <color> )
    let Some(groups) = parse_comma_separated_component_values(component_values.to_vec(), |component_values| {
        Some(component_values.to_vec())
    }) else {
        return false;
    };

    let [light, dark] = groups.as_slice() else {
        return false;
    };

    component_values_parse_as_color_value(light) && component_values_parse_as_color_value(dark)
}

pub(super) fn component_values_parse_as_color_mix_component(component_values: &[ComponentValue]) -> bool {
    let mut component_values = strip_whitespace(component_values);
    let mut percentage_count = 0;

    if let Some((first, rest)) = component_values.split_first()
        && component_value_parse_as_percentage_prefix(first)
    {
        percentage_count += 1;
        component_values = strip_whitespace(rest);
    }

    let [color] = component_values else {
        if let Some((last, rest)) = component_values.split_last()
            && component_value_parse_as_percentage_prefix(last)
        {
            percentage_count += 1;
            component_values = strip_whitespace(rest);
        }
        let [color] = component_values else {
            return false;
        };
        return percentage_count <= 1 && component_value_parse_as_color_value(color);
    };

    percentage_count <= 1 && component_value_parse_as_color_value(color)
}

pub(super) fn component_values_parse_as_color_interpolation_method(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-color-5/#color-interpolation-method
    // <rectangular-color-space> = srgb | srgb-linear | display-p3 | display-p3-linear | a98-rgb | prophoto-rgb | rec2020 | lab | oklab | <xyz-space>
    // <polar-color-space> = hsl | hwb | lch | oklch
    // <custom-color-space> = <dashed-ident>
    // <hue-interpolation-method> = [ shorter | longer | increasing | decreasing ] hue
    // <color-interpolation-method> = in [ <rectangular-color-space> | <polar-color-space> <hue-interpolation-method>? | <custom-color-space> ]
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    if !consume_optional_ident_matching(&mut parser, "in") {
        return false;
    }
    parser.discard_whitespace();
    let Some(color_space) = parser.consume_an_ident() else {
        return false;
    };

    if is_rectangular_color_space(&color_space) {
        return !parser.has_next_component_value();
    }

    if is_polar_color_space(&color_space) {
        if !parser.has_next_component_value() {
            return true;
        }
        parser.discard_whitespace();
        let Some(hue_interpolation_method) = parser.consume_an_ident() else {
            return false;
        };
        if !is_hue_interpolation_method(&hue_interpolation_method) {
            return false;
        }
        consume_optional_ident_matching(&mut parser, "hue") && !parser.has_next_component_value()
    } else {
        false
    }
}

pub(super) fn component_values_parse_as_color_value(component_values: &[ComponentValue]) -> bool {
    let component_values = strip_whitespace(component_values);
    let [component_value] = component_values else {
        return false;
    };

    component_value_parse_as_color_value(component_value)
}

pub(super) fn component_value_parse_as_color_value(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::Function(function) => component_value_parse_as_color_function(function),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Hash { value, .. },
            ..
        }) => matches!(value.len(), 3 | 4 | 6 | 8) && value.chars().all(|c| c.is_ascii_hexdigit()),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { .. },
            ..
        }) => component_value_parse_as_color_identifier(component_value),
        _ => false,
    }
}

pub(super) fn component_value_parse_as_color_identifier(component_value: &ComponentValue) -> bool {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    }) = component_value
    else {
        return false;
    };

    // https://www.w3.org/TR/css-color-4/#typedef-named-color
    // <named-color> = <ident>
    is_named_color(value)
        // https://www.w3.org/TR/css-color-4/#css-system-colors
        // <system-color> = AccentColor | AccentColorText | ActiveText | ButtonBorder | ButtonFace |
        //                  ButtonText | Canvas | CanvasText | Field | FieldText | GrayText |
        //                  Highlight | HighlightText | LinkText | Mark | MarkText | SelectedItem |
        //                  SelectedItemText | VisitedText
        || is_system_color(value)
        // https://www.w3.org/TR/css-color-4/#transparent-color
        // transparent
        || value.eq_ignore_ascii_case("transparent")
        // https://www.w3.org/TR/css-color-4/#currentcolor-color
        // currentcolor
        || value.eq_ignore_ascii_case("currentcolor")
}

pub(super) fn component_value_parse_as_quirky_color(
    component_value: &ComponentValue,
    allow_quirky_color: bool,
) -> bool {
    if !allow_quirky_color {
        return false;
    }

    // https://drafts.csswg.org/css-color-4/#quirky-color
    // "When CSS is being parsed in quirks mode, <quirky-color> is a type of
    // <color> that is only valid in certain properties:"
    // AD-HOC: The exact hashless-hex conversion still happens in C++.
    // Rust accepts the token shapes here only after C++ has materialized a
    // color value for the same consumed token slice.
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { .. },
            ..
        }) => true,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() >= 0.0 && number_is_integer(*number),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, .. },
            ..
        }) => number.value() >= 0.0 && number_is_integer(*number),
        _ => false,
    }
}

pub(super) enum ParsedSimpleColor<'a> {
    Rgba {
        red: u8,
        green: u8,
        blue: u8,
        alpha: u8,
        name: Option<&'a str>,
    },
    Keyword {
        name: &'a str,
    },
}

pub(super) fn simple_color_from_component_value(
    component_value: &ComponentValue,
    allow_quirky_color: bool,
) -> Option<ParsedSimpleColor<'_>> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) => simple_color_from_ident(value, allow_quirky_color),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Hash { value, .. },
            ..
        }) => color_from_hex_string(value).map(|(red, green, blue, alpha)| ParsedSimpleColor::Rgba {
            red,
            green,
            blue,
            alpha,
            name: None,
        }),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if allow_quirky_color => {
            if !number_is_integer(*number) || number.value() < 0.0 {
                return None;
            }
            simple_color_from_quirky_serialization(&format!("{:.0}", number.value()))
        }
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) if allow_quirky_color => {
            if !number_is_integer(*number) || number.value() < 0.0 {
                return None;
            }
            simple_color_from_quirky_serialization(&format!("{:.0}{unit}", number.value()))
        }
        _ => None,
    }
}

pub(super) fn simple_color_from_ident(value: &str, allow_quirky_color: bool) -> Option<ParsedSimpleColor<'_>> {
    if value.eq_ignore_ascii_case("transparent") {
        return Some(ParsedSimpleColor::Rgba {
            red: 0,
            green: 0,
            blue: 0,
            alpha: 0,
            name: Some(value),
        });
    }

    if let Some((red, green, blue)) = named_color_rgb(value) {
        return Some(ParsedSimpleColor::Rgba {
            red,
            green,
            blue,
            alpha: 255,
            name: Some(value),
        });
    }

    if value.eq_ignore_ascii_case("currentcolor") || is_system_color(value) {
        return Some(ParsedSimpleColor::Keyword { name: value });
    }

    if allow_quirky_color {
        return simple_color_from_quirky_identifier(value);
    }

    None
}

pub(super) fn simple_color_from_quirky_identifier(serialization: &str) -> Option<ParsedSimpleColor<'static>> {
    if !matches!(serialization.len(), 3 | 6) || !serialization.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    color_from_hex_string(serialization).map(|(red, green, blue, alpha)| ParsedSimpleColor::Rgba {
        red,
        green,
        blue,
        alpha,
        name: None,
    })
}

pub(super) fn simple_color_from_quirky_serialization(serialization: &str) -> Option<ParsedSimpleColor<'static>> {
    let mut serialization = serialization.to_string();
    if serialization.len() < 6 {
        serialization = format!("{serialization:0>6}");
    }
    if !matches!(serialization.len(), 3 | 6) || !serialization.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }

    color_from_hex_string(&serialization).map(|(red, green, blue, alpha)| ParsedSimpleColor::Rgba {
        red,
        green,
        blue,
        alpha,
        name: None,
    })
}

pub(super) fn color_from_hex_string(value: &str) -> Option<(u8, u8, u8, u8)> {
    fn hex_nibble_to_u8(nibble: char) -> Option<u8> {
        nibble.to_digit(16).and_then(|value| u8::try_from(value).ok())
    }

    let mut chars = value.chars();
    match value.len() {
        3 => {
            let red = hex_nibble_to_u8(chars.next()?)?;
            let green = hex_nibble_to_u8(chars.next()?)?;
            let blue = hex_nibble_to_u8(chars.next()?)?;
            Some((red * 17, green * 17, blue * 17, 255))
        }
        4 => {
            let red = hex_nibble_to_u8(chars.next()?)?;
            let green = hex_nibble_to_u8(chars.next()?)?;
            let blue = hex_nibble_to_u8(chars.next()?)?;
            let alpha = hex_nibble_to_u8(chars.next()?)?;
            Some((red * 17, green * 17, blue * 17, alpha * 17))
        }
        6 | 8 => {
            let bytes = value.as_bytes();
            let red = u8::from_str_radix(std::str::from_utf8(&bytes[0..2]).ok()?, 16).ok()?;
            let green = u8::from_str_radix(std::str::from_utf8(&bytes[2..4]).ok()?, 16).ok()?;
            let blue = u8::from_str_radix(std::str::from_utf8(&bytes[4..6]).ok()?, 16).ok()?;
            let alpha = if value.len() == 8 {
                u8::from_str_radix(std::str::from_utf8(&bytes[6..8]).ok()?, 16).ok()?
            } else {
                255
            };
            Some((red, green, blue, alpha))
        }
        _ => None,
    }
}

pub(super) fn named_color_rgb(input: &str) -> Option<(u8, u8, u8)> {
    match input.to_ascii_lowercase().as_str() {
        "black" => Some((0x00, 0x00, 0x00)),
        "silver" => Some((0xc0, 0xc0, 0xc0)),
        "gray" => Some((0x80, 0x80, 0x80)),
        "white" => Some((0xff, 0xff, 0xff)),
        "maroon" => Some((0x80, 0x00, 0x00)),
        "red" => Some((0xff, 0x00, 0x00)),
        "purple" => Some((0x80, 0x00, 0x80)),
        "fuchsia" => Some((0xff, 0x00, 0xff)),
        "green" => Some((0x00, 0x80, 0x00)),
        "lime" => Some((0x00, 0xff, 0x00)),
        "olive" => Some((0x80, 0x80, 0x00)),
        "yellow" => Some((0xff, 0xff, 0x00)),
        "navy" => Some((0x00, 0x00, 0x80)),
        "blue" => Some((0x00, 0x00, 0xff)),
        "teal" => Some((0x00, 0x80, 0x80)),
        "aqua" => Some((0x00, 0xff, 0xff)),
        "orange" => Some((0xff, 0xa5, 0x00)),
        "aliceblue" => Some((0xf0, 0xf8, 0xff)),
        "antiquewhite" => Some((0xfa, 0xeb, 0xd7)),
        "aquamarine" => Some((0x7f, 0xff, 0xd4)),
        "azure" => Some((0xf0, 0xff, 0xff)),
        "beige" => Some((0xf5, 0xf5, 0xdc)),
        "bisque" => Some((0xff, 0xe4, 0xc4)),
        "blanchedalmond" => Some((0xff, 0xeb, 0xcd)),
        "blueviolet" => Some((0x8a, 0x2b, 0xe2)),
        "brown" => Some((0xa5, 0x2a, 0x2a)),
        "burlywood" => Some((0xde, 0xb8, 0x87)),
        "cadetblue" => Some((0x5f, 0x9e, 0xa0)),
        "chartreuse" => Some((0x7f, 0xff, 0x00)),
        "chocolate" => Some((0xd2, 0x69, 0x1e)),
        "coral" => Some((0xff, 0x7f, 0x50)),
        "cornflowerblue" => Some((0x64, 0x95, 0xed)),
        "cornsilk" => Some((0xff, 0xf8, 0xdc)),
        "crimson" => Some((0xdc, 0x14, 0x3c)),
        "cyan" => Some((0x00, 0xff, 0xff)),
        "darkblue" => Some((0x00, 0x00, 0x8b)),
        "darkcyan" => Some((0x00, 0x8b, 0x8b)),
        "darkgoldenrod" => Some((0xb8, 0x86, 0x0b)),
        "darkgray" => Some((0xa9, 0xa9, 0xa9)),
        "darkgreen" => Some((0x00, 0x64, 0x00)),
        "darkgrey" => Some((0xa9, 0xa9, 0xa9)),
        "darkkhaki" => Some((0xbd, 0xb7, 0x6b)),
        "darkmagenta" => Some((0x8b, 0x00, 0x8b)),
        "darkolivegreen" => Some((0x55, 0x6b, 0x2f)),
        "darkorange" => Some((0xff, 0x8c, 0x00)),
        "darkorchid" => Some((0x99, 0x32, 0xcc)),
        "darkred" => Some((0x8b, 0x00, 0x00)),
        "darksalmon" => Some((0xe9, 0x96, 0x7a)),
        "darkseagreen" => Some((0x8f, 0xbc, 0x8f)),
        "darkslateblue" => Some((0x48, 0x3d, 0x8b)),
        "darkslategray" => Some((0x2f, 0x4f, 0x4f)),
        "darkslategrey" => Some((0x2f, 0x4f, 0x4f)),
        "darkturquoise" => Some((0x00, 0xce, 0xd1)),
        "darkviolet" => Some((0x94, 0x00, 0xd3)),
        "deeppink" => Some((0xff, 0x14, 0x93)),
        "deepskyblue" => Some((0x00, 0xbf, 0xff)),
        "dimgray" => Some((0x69, 0x69, 0x69)),
        "dimgrey" => Some((0x69, 0x69, 0x69)),
        "dodgerblue" => Some((0x1e, 0x90, 0xff)),
        "firebrick" => Some((0xb2, 0x22, 0x22)),
        "floralwhite" => Some((0xff, 0xfa, 0xf0)),
        "forestgreen" => Some((0x22, 0x8b, 0x22)),
        "gainsboro" => Some((0xdc, 0xdc, 0xdc)),
        "ghostwhite" => Some((0xf8, 0xf8, 0xff)),
        "gold" => Some((0xff, 0xd7, 0x00)),
        "goldenrod" => Some((0xda, 0xa5, 0x20)),
        "greenyellow" => Some((0xad, 0xff, 0x2f)),
        "grey" => Some((0x80, 0x80, 0x80)),
        "honeydew" => Some((0xf0, 0xff, 0xf0)),
        "hotpink" => Some((0xff, 0x69, 0xb4)),
        "indianred" => Some((0xcd, 0x5c, 0x5c)),
        "indigo" => Some((0x4b, 0x00, 0x82)),
        "ivory" => Some((0xff, 0xff, 0xf0)),
        "khaki" => Some((0xf0, 0xe6, 0x8c)),
        "lavender" => Some((0xe6, 0xe6, 0xfa)),
        "lavenderblush" => Some((0xff, 0xf0, 0xf5)),
        "lawngreen" => Some((0x7c, 0xfc, 0x00)),
        "lemonchiffon" => Some((0xff, 0xfa, 0xcd)),
        "lightblue" => Some((0xad, 0xd8, 0xe6)),
        "lightcoral" => Some((0xf0, 0x80, 0x80)),
        "lightcyan" => Some((0xe0, 0xff, 0xff)),
        "lightgoldenrodyellow" => Some((0xfa, 0xfa, 0xd2)),
        "lightgray" => Some((0xd3, 0xd3, 0xd3)),
        "lightgreen" => Some((0x90, 0xee, 0x90)),
        "lightgrey" => Some((0xd3, 0xd3, 0xd3)),
        "lightpink" => Some((0xff, 0xb6, 0xc1)),
        "lightsalmon" => Some((0xff, 0xa0, 0x7a)),
        "lightseagreen" => Some((0x20, 0xb2, 0xaa)),
        "lightskyblue" => Some((0x87, 0xce, 0xfa)),
        "lightslategray" => Some((0x77, 0x88, 0x99)),
        "lightslategrey" => Some((0x77, 0x88, 0x99)),
        "lightsteelblue" => Some((0xb0, 0xc4, 0xde)),
        "lightyellow" => Some((0xff, 0xff, 0xe0)),
        "limegreen" => Some((0x32, 0xcd, 0x32)),
        "linen" => Some((0xfa, 0xf0, 0xe6)),
        "magenta" => Some((0xff, 0x00, 0xff)),
        "mediumaquamarine" => Some((0x66, 0xcd, 0xaa)),
        "mediumblue" => Some((0x00, 0x00, 0xcd)),
        "mediumorchid" => Some((0xba, 0x55, 0xd3)),
        "mediumpurple" => Some((0x93, 0x70, 0xdb)),
        "mediumseagreen" => Some((0x3c, 0xb3, 0x71)),
        "mediumslateblue" => Some((0x7b, 0x68, 0xee)),
        "mediumspringgreen" => Some((0x00, 0xfa, 0x9a)),
        "mediumturquoise" => Some((0x48, 0xd1, 0xcc)),
        "mediumvioletred" => Some((0xc7, 0x15, 0x85)),
        "midnightblue" => Some((0x19, 0x19, 0x70)),
        "mintcream" => Some((0xf5, 0xff, 0xfa)),
        "mistyrose" => Some((0xff, 0xe4, 0xe1)),
        "moccasin" => Some((0xff, 0xe4, 0xb5)),
        "navajowhite" => Some((0xff, 0xde, 0xad)),
        "oldlace" => Some((0xfd, 0xf5, 0xe6)),
        "olivedrab" => Some((0x6b, 0x8e, 0x23)),
        "orangered" => Some((0xff, 0x45, 0x00)),
        "orchid" => Some((0xda, 0x70, 0xd6)),
        "palegoldenrod" => Some((0xee, 0xe8, 0xaa)),
        "palegreen" => Some((0x98, 0xfb, 0x98)),
        "paleturquoise" => Some((0xaf, 0xee, 0xee)),
        "palevioletred" => Some((0xdb, 0x70, 0x93)),
        "papayawhip" => Some((0xff, 0xef, 0xd5)),
        "peachpuff" => Some((0xff, 0xda, 0xb9)),
        "peru" => Some((0xcd, 0x85, 0x3f)),
        "pink" => Some((0xff, 0xc0, 0xcb)),
        "plum" => Some((0xdd, 0xa0, 0xdd)),
        "powderblue" => Some((0xb0, 0xe0, 0xe6)),
        "rosybrown" => Some((0xbc, 0x8f, 0x8f)),
        "royalblue" => Some((0x41, 0x69, 0xe1)),
        "saddlebrown" => Some((0x8b, 0x45, 0x13)),
        "salmon" => Some((0xfa, 0x80, 0x72)),
        "sandybrown" => Some((0xf4, 0xa4, 0x60)),
        "seagreen" => Some((0x2e, 0x8b, 0x57)),
        "seashell" => Some((0xff, 0xf5, 0xee)),
        "sienna" => Some((0xa0, 0x52, 0x2d)),
        "skyblue" => Some((0x87, 0xce, 0xeb)),
        "slateblue" => Some((0x6a, 0x5a, 0xcd)),
        "slategray" => Some((0x70, 0x80, 0x90)),
        "slategrey" => Some((0x70, 0x80, 0x90)),
        "snow" => Some((0xff, 0xfa, 0xfa)),
        "springgreen" => Some((0x00, 0xff, 0x7f)),
        "steelblue" => Some((0x46, 0x82, 0xb4)),
        "tan" => Some((0xd2, 0xb4, 0x8c)),
        "thistle" => Some((0xd8, 0xbf, 0xd8)),
        "tomato" => Some((0xff, 0x63, 0x47)),
        "turquoise" => Some((0x40, 0xe0, 0xd0)),
        "violet" => Some((0xee, 0x82, 0xee)),
        "wheat" => Some((0xf5, 0xde, 0xb3)),
        "whitesmoke" => Some((0xf5, 0xf5, 0xf5)),
        "yellowgreen" => Some((0x9a, 0xcd, 0x32)),
        "rebeccapurple" => Some((0x66, 0x33, 0x99)),
        _ => None,
    }
}

pub(super) fn is_named_color(input: &str) -> bool {
    matches!(
        input.to_ascii_lowercase().as_str(),
        "aliceblue"
            | "antiquewhite"
            | "aqua"
            | "aquamarine"
            | "azure"
            | "beige"
            | "bisque"
            | "black"
            | "blanchedalmond"
            | "blue"
            | "blueviolet"
            | "brown"
            | "burlywood"
            | "cadetblue"
            | "chartreuse"
            | "chocolate"
            | "coral"
            | "cornflowerblue"
            | "cornsilk"
            | "crimson"
            | "cyan"
            | "darkblue"
            | "darkcyan"
            | "darkgoldenrod"
            | "darkgray"
            | "darkgreen"
            | "darkgrey"
            | "darkkhaki"
            | "darkmagenta"
            | "darkolivegreen"
            | "darkorange"
            | "darkorchid"
            | "darkred"
            | "darksalmon"
            | "darkseagreen"
            | "darkslateblue"
            | "darkslategray"
            | "darkslategrey"
            | "darkturquoise"
            | "darkviolet"
            | "deeppink"
            | "deepskyblue"
            | "dimgray"
            | "dimgrey"
            | "dodgerblue"
            | "firebrick"
            | "floralwhite"
            | "forestgreen"
            | "fuchsia"
            | "gainsboro"
            | "ghostwhite"
            | "gold"
            | "goldenrod"
            | "gray"
            | "green"
            | "greenyellow"
            | "grey"
            | "honeydew"
            | "hotpink"
            | "indianred"
            | "indigo"
            | "ivory"
            | "khaki"
            | "lavender"
            | "lavenderblush"
            | "lawngreen"
            | "lemonchiffon"
            | "lightblue"
            | "lightcoral"
            | "lightcyan"
            | "lightgoldenrodyellow"
            | "lightgray"
            | "lightgreen"
            | "lightgrey"
            | "lightpink"
            | "lightsalmon"
            | "lightseagreen"
            | "lightskyblue"
            | "lightslategray"
            | "lightslategrey"
            | "lightsteelblue"
            | "lightyellow"
            | "lime"
            | "limegreen"
            | "linen"
            | "magenta"
            | "maroon"
            | "mediumaquamarine"
            | "mediumblue"
            | "mediumorchid"
            | "mediumpurple"
            | "mediumseagreen"
            | "mediumslateblue"
            | "mediumspringgreen"
            | "mediumturquoise"
            | "mediumvioletred"
            | "midnightblue"
            | "mintcream"
            | "mistyrose"
            | "moccasin"
            | "navajowhite"
            | "navy"
            | "oldlace"
            | "olive"
            | "olivedrab"
            | "orange"
            | "orangered"
            | "orchid"
            | "palegoldenrod"
            | "palegreen"
            | "paleturquoise"
            | "palevioletred"
            | "papayawhip"
            | "peachpuff"
            | "peru"
            | "pink"
            | "plum"
            | "powderblue"
            | "purple"
            | "rebeccapurple"
            | "red"
            | "rosybrown"
            | "royalblue"
            | "saddlebrown"
            | "salmon"
            | "sandybrown"
            | "seagreen"
            | "seashell"
            | "sienna"
            | "silver"
            | "skyblue"
            | "slateblue"
            | "slategray"
            | "slategrey"
            | "snow"
            | "springgreen"
            | "steelblue"
            | "tan"
            | "teal"
            | "thistle"
            | "tomato"
            | "turquoise"
            | "violet"
            | "wheat"
            | "white"
            | "whitesmoke"
            | "yellow"
            | "yellowgreen"
    )
}

pub(super) fn is_system_color(input: &str) -> bool {
    matches!(
        input.to_ascii_lowercase().as_str(),
        "accentcolor"
            | "accentcolortext"
            | "activeborder"
            | "activecaption"
            | "activetext"
            | "appworkspace"
            | "background"
            | "buttonborder"
            | "buttonface"
            | "buttonhighlight"
            | "buttonshadow"
            | "buttontext"
            | "canvas"
            | "canvastext"
            | "captiontext"
            | "field"
            | "fieldtext"
            | "graytext"
            | "highlight"
            | "highlighttext"
            | "inactiveborder"
            | "inactivecaption"
            | "inactivecaptiontext"
            | "infobackground"
            | "infotext"
            | "linktext"
            | "mark"
            | "marktext"
            | "menu"
            | "menutext"
            | "scrollbar"
            | "selecteditem"
            | "selecteditemtext"
            | "threeddarkshadow"
            | "threedface"
            | "threedhighlight"
            | "threedlightshadow"
            | "threedshadow"
            | "visitedtext"
            | "window"
            | "windowframe"
            | "windowtext"
            | "-libweb-buttonfacedisabled"
            | "-libweb-buttonfacehover"
            | "-libweb-link"
            | "-libweb-palette-active-link"
            | "-libweb-palette-active-window-border1"
            | "-libweb-palette-active-window-border2"
            | "-libweb-palette-active-window-title"
            | "-libweb-palette-base"
            | "-libweb-palette-base-text"
            | "-libweb-palette-button"
            | "-libweb-palette-button-text"
            | "-libweb-palette-desktop-background"
            | "-libweb-palette-focus-outline"
            | "-libweb-palette-highlight-window-border1"
            | "-libweb-palette-highlight-window-border2"
            | "-libweb-palette-highlight-window-title"
            | "-libweb-palette-hover-highlight"
            | "-libweb-palette-inactive-selection"
            | "-libweb-palette-inactive-selection-text"
            | "-libweb-palette-inactive-window-border1"
            | "-libweb-palette-inactive-window-border2"
            | "-libweb-palette-inactive-window-title"
            | "-libweb-palette-link"
            | "-libweb-palette-menu-base"
            | "-libweb-palette-menu-base-text"
            | "-libweb-palette-menu-selection"
            | "-libweb-palette-menu-selection-text"
            | "-libweb-palette-menu-stripe"
            | "-libweb-palette-moving-window-border1"
            | "-libweb-palette-moving-window-border2"
            | "-libweb-palette-moving-window-title"
            | "-libweb-palette-rubber-band-border"
            | "-libweb-palette-rubber-band-fill"
            | "-libweb-palette-ruler"
            | "-libweb-palette-ruler-active-text"
            | "-libweb-palette-ruler-border"
            | "-libweb-palette-ruler-inactive-text"
            | "-libweb-palette-selection"
            | "-libweb-palette-selection-text"
            | "-libweb-palette-syntax-comment"
            | "-libweb-palette-syntax-control-keyword"
            | "-libweb-palette-syntax-identifier"
            | "-libweb-palette-syntax-keyword"
            | "-libweb-palette-syntax-number"
            | "-libweb-palette-syntax-operator"
            | "-libweb-palette-syntax-preprocessor-statement"
            | "-libweb-palette-syntax-preprocessor-value"
            | "-libweb-palette-syntax-punctuation"
            | "-libweb-palette-syntax-string"
            | "-libweb-palette-syntax-type"
            | "-libweb-palette-text-cursor"
            | "-libweb-palette-threed-highlight"
            | "-libweb-palette-threed-shadow1"
            | "-libweb-palette-threed-shadow2"
            | "-libweb-palette-visited-link"
            | "-libweb-palette-window"
            | "-libweb-palette-window-text"
    )
}

pub(super) fn consume_number_percentage_none_component_value(parser: &mut ComponentValueParser) -> bool {
    consume_number_percentage_none_component_value_with_percentage_kind(parser).is_some()
}

pub(super) fn consume_number_percentage_none_component_value_with_percentage_kind(
    parser: &mut ComponentValueParser,
) -> Option<Option<bool>> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    let percentage_kind = match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("none") => None,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { .. },
            ..
        }) => Some(false),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { .. },
            ..
        }) => Some(true),
        ComponentValue::Function(function) if is_math_function_name(&function.name) => None,
        _ => return None,
    };
    parser.index += 1;
    Some(percentage_kind)
}

pub(super) fn consume_color_number_percentage_component_value_with_percentage_kind(
    parser: &mut ComponentValueParser,
) -> Option<Option<bool>> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    let is_percentage = match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { .. },
            ..
        }) => Some(false),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { .. },
            ..
        }) => Some(true),
        ComponentValue::Function(function) if is_math_function_name(&function.name) => None,
        _ => return None,
    };
    parser.index += 1;
    Some(is_percentage)
}

pub(super) fn consume_percentage_component_value(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };
    if component_value_parse_as_percentage_prefix(component_value) {
        parser.index += 1;
        return true;
    }
    false
}

pub(super) fn consume_hue_none_component_value(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };
    if component_value_parse_as_hue_none(component_value) {
        parser.index += 1;
        return true;
    }
    false
}

pub(super) fn consume_optional_solidus_and_alpha_value(parser: &mut ComponentValueParser) -> bool {
    // https://www.w3.org/TR/css-color-4/#typedef-color-alpha-value
    // [ / [<alpha-value> | none] ]?
    // <alpha-value> = <number> | <percentage>
    parser.discard_whitespace();
    if !parser.has_next_component_value() {
        return true;
    }

    if !parser.consume_a_delim('/') {
        return false;
    }

    parser.discard_whitespace();
    consume_number_percentage_none_component_value(parser)
}

pub(super) fn component_value_parse_as_number_percentage_none(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("none")
    ) || matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { .. } | TokenType::Percentage { .. },
            ..
        })
    ) || matches!(component_value, ComponentValue::Function(function) if is_math_function_name(&function.name))
}

pub(super) fn component_value_parse_as_hue_none(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("none")
    ) || matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { .. },
            ..
        })
    ) || matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Angle))
    ) || matches!(component_value, ComponentValue::Function(function) if is_math_function_name(&function.name))
}

pub(super) fn component_value_parse_as_percentage_prefix(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { .. },
            ..
        })
    ) || matches!(component_value, ComponentValue::Function(function) if is_math_function_name(&function.name))
}

pub(super) fn is_color_function_color_space(input: &str) -> bool {
    matches!(
        input.to_ascii_lowercase().as_str(),
        "srgb"
            | "srgb-linear"
            | "display-p3"
            | "display-p3-linear"
            | "a98-rgb"
            | "prophoto-rgb"
            | "rec2020"
            | "xyz"
            | "xyz-d50"
            | "xyz-d65"
    )
}

pub(super) fn is_rectangular_color_space(input: &str) -> bool {
    matches!(
        input.to_ascii_lowercase().as_str(),
        "srgb"
            | "srgb-linear"
            | "display-p3"
            | "display-p3-linear"
            | "a98-rgb"
            | "prophoto-rgb"
            | "rec2020"
            | "lab"
            | "oklab"
            | "xyz"
            | "xyz-d50"
            | "xyz-d65"
    )
}

pub(super) fn is_polar_color_space(input: &str) -> bool {
    matches!(input.to_ascii_lowercase().as_str(), "hsl" | "hwb" | "lch" | "oklch")
}

pub(super) fn is_hue_interpolation_method(input: &str) -> bool {
    matches!(
        input.to_ascii_lowercase().as_str(),
        "shorter" | "longer" | "increasing" | "decreasing"
    )
}

pub(crate) fn parse_translate_value(filtered_input: &[u8]) -> CssTransformLonghandValueKind {
    if parse_rust_owned_translate_value(filtered_input).is_some() {
        CssTransformLonghandValueKind::Valid
    } else {
        CssTransformLonghandValueKind::Invalid
    }
}

pub(super) fn parse_rust_owned_translate_value(filtered_input: &[u8]) -> Option<RustOwnedTransformLonghand> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let filtered_input_string = filtered_input_to_string(filtered_input);

    // https://drafts.csswg.org/css-transforms-2/#propdef-translate
    // translate = none | <length-percentage> [ <length-percentage> <length>? ]?
    if consume_optional_ident_matching(&mut parser, "none") {
        parser.discard_whitespace();
        return (!parser.has_next_component_value()).then_some(RustOwnedTransformLonghand::None);
    }

    let x = consume_component_value_as_nested_transform_function_argument(
        &mut parser,
        &filtered_input_string,
        TransformFunctionParameterType::LengthPercentage,
    )?;

    parser.discard_whitespace();
    if !parser.has_next_component_value() {
        return Some(RustOwnedTransformLonghand::Function {
            function: RustOwnedTransformLonghandFunction::Translate,
            arguments: vec![
                x,
                RustOwnedTransformationArgument {
                    parameter_type: TransformFunctionParameterType::LengthPercentage,
                    value: zero_pixel_length(),
                },
            ],
        });
    }

    let y = consume_component_value_as_nested_transform_function_argument(
        &mut parser,
        &filtered_input_string,
        TransformFunctionParameterType::LengthPercentage,
    )?;

    parser.discard_whitespace();
    if !parser.has_next_component_value() {
        return Some(RustOwnedTransformLonghand::Function {
            function: RustOwnedTransformLonghandFunction::Translate,
            arguments: vec![x, y],
        });
    }

    let z = consume_component_value_as_nested_transform_function_argument(
        &mut parser,
        &filtered_input_string,
        TransformFunctionParameterType::Length,
    )?;

    parser.discard_whitespace();
    (!parser.has_next_component_value()).then_some(RustOwnedTransformLonghand::Function {
        function: RustOwnedTransformLonghandFunction::Translate3d,
        arguments: vec![x, y, z],
    })
}

pub(crate) fn parse_scale_value(filtered_input: &[u8]) -> CssTransformLonghandValueKind {
    if parse_rust_owned_scale_value(filtered_input).is_some() {
        CssTransformLonghandValueKind::Valid
    } else {
        CssTransformLonghandValueKind::Invalid
    }
}

pub(super) fn parse_rust_owned_scale_value(filtered_input: &[u8]) -> Option<RustOwnedTransformLonghand> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let filtered_input_string = filtered_input_to_string(filtered_input);

    // https://drafts.csswg.org/css-transforms-2/#propdef-scale
    // scale = none | [ <number> | <percentage> ]{1,3}
    if consume_optional_ident_matching(&mut parser, "none") {
        parser.discard_whitespace();
        return (!parser.has_next_component_value()).then_some(RustOwnedTransformLonghand::None);
    }

    let mut arguments = Vec::new();
    for _ in 0..3 {
        let argument = consume_component_value_as_nested_transform_function_argument(
            &mut parser,
            &filtered_input_string,
            TransformFunctionParameterType::NumberPercentage,
        )?;
        arguments.push(argument);
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            return Some(RustOwnedTransformLonghand::Function {
                function: if arguments.len() == 3 {
                    RustOwnedTransformLonghandFunction::Scale3d
                } else {
                    RustOwnedTransformLonghandFunction::Scale
                },
                arguments,
            });
        }
    }

    None
}

pub(crate) fn parse_rotate_value(filtered_input: &[u8]) -> CssTransformLonghandValueKind {
    if parse_rust_owned_rotate_value(filtered_input).is_some() {
        CssTransformLonghandValueKind::Valid
    } else {
        CssTransformLonghandValueKind::Invalid
    }
}

pub(super) fn parse_rust_owned_rotate_value(filtered_input: &[u8]) -> Option<RustOwnedTransformLonghand> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let filtered_input_string = filtered_input_to_string(filtered_input);

    // https://drafts.csswg.org/css-transforms-2/#propdef-rotate
    // rotate = none | <angle> | [ x | y | z | <number>{3} ] && <angle>
    if consume_optional_ident_matching(&mut parser, "none") {
        parser.discard_whitespace();
        return (!parser.has_next_component_value()).then_some(RustOwnedTransformLonghand::None);
    }

    let angle =
        consume_component_value_as_nested_transform_longhand_angle_argument(&mut parser, &filtered_input_string);
    parser.discard_whitespace();
    if let Some(angle) = angle.clone()
        && !parser.has_next_component_value()
    {
        return Some(RustOwnedTransformLonghand::Function {
            function: RustOwnedTransformLonghandFunction::Rotate,
            arguments: vec![angle],
        });
    }

    if let Some(axis) = consume_rotate_axis(&mut parser) {
        parser.discard_whitespace();
        let angle = angle.or_else(|| {
            consume_component_value_as_nested_transform_longhand_angle_argument(&mut parser, &filtered_input_string)
        });
        if let Some(angle) = angle {
            parser.discard_whitespace();
            if parser.has_next_component_value() {
                return None;
            }
            return Some(RustOwnedTransformLonghand::Function {
                function: match axis {
                    RotateAxis::X => RustOwnedTransformLonghandFunction::RotateX,
                    RotateAxis::Y => RustOwnedTransformLonghandFunction::RotateY,
                    RotateAxis::Z => RustOwnedTransformLonghandFunction::RotateZ,
                },
                arguments: vec![angle],
            });
        }
        return None;
    }

    let mut numbers = Vec::new();
    for _ in 0..3 {
        numbers.push(consume_component_value_as_nested_transform_function_argument(
            &mut parser,
            &filtered_input_string,
            TransformFunctionParameterType::Number,
        )?);
    }

    parser.discard_whitespace();
    let angle = angle.or_else(|| {
        consume_component_value_as_nested_transform_longhand_angle_argument(&mut parser, &filtered_input_string)
    })?;

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    numbers.push(angle);
    Some(RustOwnedTransformLonghand::Function {
        function: RustOwnedTransformLonghandFunction::Rotate3d,
        arguments: numbers,
    })
}

pub(super) fn consume_component_value_as_nested_transform_function_argument(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
    parameter_type: TransformFunctionParameterType,
) -> Option<RustOwnedTransformationArgument> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    let value = component_value_parse_as_nested_transform_function_argument(
        component_value,
        parameter_type,
        filtered_input_string,
    )?;
    parser.index += 1;
    Some(RustOwnedTransformationArgument { parameter_type, value })
}

pub(super) fn consume_component_value_as_nested_transform_longhand_angle_argument(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedTransformationArgument> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    if !component_value_parse_as_angle_for_transform_longhand(component_value) {
        return None;
    }
    let value = if component_value_is_zero_number(component_value) {
        RustOwnedNestedPrimitiveValue::Angle {
            value: 0.0,
            unit: "deg".to_string(),
        }
    } else {
        component_value_parse_as_nested_angle(component_value, filtered_input_string)?
    };
    parser.index += 1;
    Some(RustOwnedTransformationArgument {
        parameter_type: TransformFunctionParameterType::Angle,
        value,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RotateAxis {
    X,
    Y,
    Z,
}

pub(super) fn consume_rotate_axis(parser: &mut ComponentValueParser) -> Option<RotateAxis> {
    if consume_optional_ident_matching(parser, "x") {
        Some(RotateAxis::X)
    } else if consume_optional_ident_matching(parser, "y") {
        Some(RotateAxis::Y)
    } else if consume_optional_ident_matching(parser, "z") {
        Some(RotateAxis::Z)
    } else {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum TransformOriginAxis {
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TransformOriginComponent {
    pub(super) axis: Option<TransformOriginAxis>,
    pub(super) is_offset: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct RustOwnedTransformOriginComponent {
    pub(super) axis: Option<TransformOriginAxis>,
    pub(super) is_offset: bool,
    pub(super) value: RustOwnedNestedPrimitiveValue,
}

pub(super) fn transform_origin_component(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedTransformOriginComponent> {
    if component_value_parse_as_length_percentage(component_value) {
        return Some(RustOwnedTransformOriginComponent {
            axis: None,
            is_offset: true,
            value: component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)?,
        });
    }

    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    }) = component_value
    else {
        return None;
    };

    let axis = if value.eq_ignore_ascii_case("left") || value.eq_ignore_ascii_case("right") {
        Some(TransformOriginAxis::X)
    } else if value.eq_ignore_ascii_case("top") || value.eq_ignore_ascii_case("bottom") {
        Some(TransformOriginAxis::Y)
    } else if value.eq_ignore_ascii_case("center") {
        None
    } else {
        return None;
    };

    Some(RustOwnedTransformOriginComponent {
        axis,
        is_offset: false,
        value: RustOwnedNestedPrimitiveValue::Keyword(value.to_ascii_lowercase()),
    })
}

pub(crate) fn parse_transform_origin_value(filtered_input: &[u8]) -> CssTransformLonghandValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    // https://www.w3.org/TR/css-transforms-1/#propdef-transform-origin
    // transform-origin =
    //     [ left | center | right | top | bottom | <length-percentage> ] |
    //     [ left | center | right | <length-percentage> ]
    //     [ top | center | bottom | <length-percentage> ] <length>? |
    //     [[ center | left | right ] && [ center | top | bottom ]] <length>?
    let Some(first_value) = consume_transform_origin_component(&mut parser) else {
        return CssTransformLonghandValueKind::Invalid;
    };

    parser.discard_whitespace();
    if !parser.has_next_component_value() {
        return CssTransformLonghandValueKind::Valid;
    }

    let Some(second_value) = consume_transform_origin_component(&mut parser) else {
        return CssTransformLonghandValueKind::Invalid;
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() && !consume_length_component_value(&mut parser) {
        return CssTransformLonghandValueKind::Invalid;
    }

    parser.discard_whitespace();
    if parser.has_next_component_value()
        || (first_value.is_offset && second_value.axis == Some(TransformOriginAxis::X))
        || (second_value.is_offset && first_value.axis == Some(TransformOriginAxis::Y))
    {
        return CssTransformLonghandValueKind::Invalid;
    }

    let mut x_value = if first_value.axis == Some(TransformOriginAxis::X) {
        Some(first_value)
    } else {
        None
    };
    let mut y_value = if first_value.axis == Some(TransformOriginAxis::Y) {
        Some(first_value)
    } else {
        None
    };

    match second_value.axis {
        Some(TransformOriginAxis::X) => {
            if x_value.is_some() {
                return CssTransformLonghandValueKind::Invalid;
            }
            x_value = Some(second_value);
            y_value = Some(first_value);
        }
        Some(TransformOriginAxis::Y) => {
            if y_value.is_some() {
                return CssTransformLonghandValueKind::Invalid;
            }
            y_value = Some(second_value);
            x_value = Some(first_value);
        }
        None => {
            if x_value.is_some() {
                y_value = Some(second_value);
            } else {
                x_value = Some(second_value);
            }
        }
    }

    if first_value.axis.is_none() && second_value.axis.is_none() {
        x_value = Some(first_value);
        y_value = Some(second_value);
    }

    if x_value.is_some() && y_value.is_some() {
        CssTransformLonghandValueKind::Valid
    } else {
        CssTransformLonghandValueKind::Invalid
    }
}

pub(crate) fn parse_math_depth_value(filtered_input: &[u8]) -> bool {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://w3c.github.io/mathml-core/#propdef-math-depth
    // Value: auto-add | add(<integer>) | <integer>
    if parser.consume_ident_matching("auto-add") {
        return !parser.has_next_component_value();
    }

    let Some(component_value) = parser.consume_the_next_component_value() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    if let ComponentValue::Function(function) = &component_value
        && function.name.eq_ignore_ascii_case("add")
    {
        let Some(source) = serialize_component_values_for_reparsing(&function.value, filtered_input_string) else {
            return false;
        };
        return component_values_parse_as_property_value_type(PropertyValueType::Integer, source.as_bytes());
    }

    let Some(source) = serialize_component_values_for_reparsing(&[component_value], filtered_input_string) else {
        return false;
    };
    component_values_parse_as_property_value_type(PropertyValueType::Integer, source.as_bytes())
}

pub(crate) fn parse_aspect_ratio_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://www.w3.org/TR/css-sizing-4/#aspect-ratio
    // auto || <ratio>
    match component_values.as_slice() {
        [component_value] if component_value_is_ident(Some(component_value), "auto") => true,
        component_values if component_values_parse_as_exact_ratio(component_values) => true,
        [first, rest @ ..] if component_value_is_ident(Some(first), "auto") => {
            component_values_parse_as_exact_ratio(rest)
        }
        [rest @ .., last] if component_value_is_ident(Some(last), "auto") => {
            component_values_parse_as_exact_ratio(rest)
        }
        _ => false,
    }
}

pub(crate) fn parse_border_radius_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://drafts.csswg.org/css-borders-4/#typedef-border-radius
    // <border-radius> = <slash-separated-border-radius-syntax> | <legacy-border-radius-syntax>
    // <slash-separated-border-radius-syntax> = <length-percentage [0,∞]> [ / <length-percentage [0,∞]> ]?
    // <legacy-border-radius-syntax> = <length-percentage [0,∞]>{1,2}
    match component_values.as_slice() {
        [horizontal] => component_value_parse_as_non_negative_length_percentage(horizontal),
        [horizontal, vertical] => {
            component_value_parse_as_non_negative_length_percentage(horizontal)
                && component_value_parse_as_non_negative_length_percentage(vertical)
        }
        [horizontal, slash, vertical] => {
            component_value_parse_as_non_negative_length_percentage(horizontal)
                && component_value_is_delim(Some(slash), '/')
                && component_value_parse_as_non_negative_length_percentage(vertical)
        }
        _ => false,
    }
}

pub(crate) fn parse_border_radius_shorthand_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    let slash_positions = component_values
        .iter()
        .enumerate()
        .filter_map(|(index, component_value)| component_value_is_delim(Some(component_value), '/').then_some(index))
        .collect::<Vec<_>>();

    if slash_positions.len() > 1 {
        return false;
    }

    let (horizontal_radii, vertical_radii) = if let Some(slash_position) = slash_positions.first() {
        (
            &component_values[..*slash_position],
            Some(&component_values[*slash_position + 1..]),
        )
    } else {
        (component_values.as_slice(), None)
    };

    // https://drafts.csswg.org/css-backgrounds-3/#border-radius
    // <'border-radius'> = <length-percentage [0,∞]>{1,4} [ / <length-percentage [0,∞]>{1,4} ]?
    parse_border_radius_shorthand_side(horizontal_radii)
        && vertical_radii.is_none_or(parse_border_radius_shorthand_side)
}

pub(crate) fn parse_columns_value(filtered_input: &[u8]) -> bool {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    let slash_positions = component_values
        .iter()
        .enumerate()
        .filter_map(|(index, component_value)| component_value_is_delim(Some(component_value), '/').then_some(index))
        .collect::<Vec<_>>();

    if slash_positions.len() > 1 {
        return false;
    }

    let (columns, column_height) = if let Some(slash_position) = slash_positions.first() {
        (
            &component_values[..*slash_position],
            Some(&component_values[*slash_position + 1..]),
        )
    } else {
        (component_values.as_slice(), None)
    };

    // https://drafts.csswg.org/css-multicol-2/#propdef-columns
    // <'column-width'> || <'column-count'> [ / <'column-height'> ]?
    if columns.is_empty() || columns.len() > 2 {
        return false;
    }

    if let Some(column_height) = column_height
        && !parse_single_column_component_value(PropertyId::ColumnHeight, column_height, filtered_input_string)
    {
        return false;
    }

    let mut found_autos = 0_u8;
    let mut has_column_count = false;
    let mut has_column_width = false;
    for component_value in columns {
        if component_value_is_ident(Some(component_value), "auto") {
            found_autos += 1;
            continue;
        }

        let component_values = std::slice::from_ref(component_value);
        if !has_column_width
            && parse_single_column_component_value(PropertyId::ColumnWidth, component_values, filtered_input_string)
        {
            has_column_width = true;
            continue;
        }

        if !has_column_count
            && parse_single_column_component_value(PropertyId::ColumnCount, component_values, filtered_input_string)
        {
            has_column_count = true;
            continue;
        }

        return false;
    }

    found_autos <= 2
}

pub(crate) fn parse_cursor_value(filtered_input: &[u8]) -> bool {
    parse_rust_owned_cursor_value(filtered_input).is_some()
}

pub(super) fn parse_rust_owned_cursor_value(filtered_input: &[u8]) -> Option<RustOwnedCursor> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let groups = parse_comma_separated_component_values(component_values, Some)?;

    // https://drafts.csswg.org/css-ui-4/#cursor
    // [ [ <url> | <url-set> ] [ <number> <number> ]? , ]* <cursor-predefined>
    let (last, cursor_images) = groups.split_last()?;
    let predefined = parse_cursor_predefined(last)?;
    let images = cursor_images
        .iter()
        .map(|component_values| parse_cursor_image(component_values, filtered_input_string))
        .collect::<Option<Vec<_>>>()?;

    Some(RustOwnedCursor { images, predefined })
}

pub(crate) fn parse_shadow_value(property_id: PropertyId, filtered_input: &[u8]) -> bool {
    parse_rust_owned_shadow_value(property_id, filtered_input).is_some()
}

pub(super) fn parse_rust_owned_shadow_value(property_id: PropertyId, filtered_input: &[u8]) -> Option<RustOwnedShadow> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let filtered_input_string = filtered_input_to_string(filtered_input);

    // https://drafts.csswg.org/css-backgrounds-3/#typedef-shadow
    // <shadow> = <color>? && [<length>{2} <length [0,∞]>? <length>?] && inset?
    let component_values_without_whitespace = strip_whitespace(&component_values);
    if matches!(&component_values_without_whitespace, [component_value] if component_value_is_ident(Some(component_value), "none"))
    {
        return Some(RustOwnedShadow::None);
    }

    let is_box_shadow = property_id == PropertyId::BoxShadow;
    let shadows = parse_comma_separated_component_values(component_values, |component_values| {
        parse_single_shadow_value(component_values, &filtered_input_string, is_box_shadow)
    })?;

    (!shadows.is_empty()).then_some(RustOwnedShadow::Shadows(shadows))
}

pub(crate) fn parse_overflow_clip_margin_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-overflow-4/#overflow-clip-margin
    // <visual-box> || <length [0,∞]>
    // FIXME: Implement the <visual-box> part of this.
    matches!(component_values, [component_value] if component_value_parse_as_non_negative_length(component_value))
}

pub(crate) fn parse_shape_outside_value(filtered_input: &[u8]) -> bool {
    parse_rust_owned_shape_outside_value(filtered_input).is_some()
}

pub(super) fn parse_rust_owned_shape_outside_value(filtered_input: &[u8]) -> Option<RustOwnedShapeOutside> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://drafts.csswg.org/css-shapes-1/#shape-outside-property
    // none | [ <basic-shape> || <shape-box> ] | <image>
    match component_values.as_slice() {
        [component_value] if component_value_is_ident(Some(component_value), "none") => {
            return Some(RustOwnedShapeOutside::None);
        }
        [] => return None,
        _ => {}
    }

    if let Some(value) = rust_owned_image_style_value_kind(filtered_input, filtered_input_string) {
        return match value {
            RustOwnedStyleValueKind::Image(image) => Some(RustOwnedShapeOutside::Image(image)),
            RustOwnedStyleValueKind::ImageSet(_) => Some(RustOwnedShapeOutside::Image(RustOwnedImage {
                kind: RustOwnedImageKind::ImageSet,
                source: Some(filtered_input_to_string(filtered_input)),
                url: None,
                gradient: None,
            })),
            _ => None,
        };
    }

    let mut basic_shape = None;
    let mut shape_box = None;
    for component_value in &component_values {
        let component_values = std::slice::from_ref(component_value);
        if basic_shape.is_none()
            && let Some(source) = serialize_component_values_for_reparsing(component_values, filtered_input_string)
            && let Some(RustOwnedStyleValueKind::BasicShape(value)) =
                rust_owned_basic_shape_style_value_kind(source.as_bytes(), &source)
        {
            basic_shape = Some(value);
            continue;
        }

        if shape_box.is_none()
            && let Some(value) = rust_owned_shape_box_from_component_value(component_value)
        {
            shape_box = Some(value);
            continue;
        }

        return None;
    }

    if basic_shape.is_none() && shape_box.is_none() {
        return None;
    }

    Some(RustOwnedShapeOutside::Shape { basic_shape, shape_box })
}

pub(crate) fn parse_text_decoration_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    if component_values.is_empty() {
        return false;
    }

    let mut text_decoration_line = Vec::new();
    let mut has_text_decoration_style = false;
    let mut has_text_decoration_color = false;
    let mut has_text_decoration_thickness = false;
    let mut saw_non_line_after_line = false;

    for component_value in &component_values {
        if component_value_is_text_decoration_line(component_value) {
            if saw_non_line_after_line {
                return false;
            }
            text_decoration_line.push(component_value.clone());
            continue;
        }

        if !text_decoration_line.is_empty() {
            saw_non_line_after_line = true;
        }

        if !has_text_decoration_style && component_value_is_text_decoration_style(component_value) {
            has_text_decoration_style = true;
            continue;
        }

        if !has_text_decoration_color && component_value_parse_as_color_value(component_value) {
            has_text_decoration_color = true;
            continue;
        }

        if !has_text_decoration_thickness && component_value_parse_as_text_decoration_thickness(component_value) {
            has_text_decoration_thickness = true;
            continue;
        }

        return false;
    }

    // https://drafts.csswg.org/css-text-decor-4/#text-decoration-property
    // <'text-decoration-line'> || <'text-decoration-thickness'> || <'text-decoration-style'> || <'text-decoration-color'>
    text_decoration_line.is_empty() || component_values_parse_as_text_decoration_line(&text_decoration_line).is_some()
}

pub(crate) fn parse_text_decoration_line_value(filtered_input: &[u8]) -> bool {
    parse_text_decoration_line_bits(filtered_input).is_some()
}

pub(super) fn parse_text_decoration_line_bits(filtered_input: &[u8]) -> Option<u8> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    component_values_parse_as_text_decoration_line(&component_values)
}

pub(crate) fn parse_list_style_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    if component_values.is_empty() {
        return false;
    }

    let mut has_list_style_position = false;
    let mut has_list_style_image = false;
    let mut has_list_style_type = false;
    let mut found_nones = 0;

    for component_value in &component_values {
        if component_value_is_ident(Some(component_value), "none") {
            found_nones += 1;
            continue;
        }

        if !has_list_style_image && component_value_parse_as_list_style_image(component_value) {
            has_list_style_image = true;
            continue;
        }

        if !has_list_style_position && component_value_is_list_style_position(component_value) {
            has_list_style_position = true;
            continue;
        }

        if !has_list_style_type && component_value_parse_as_list_style_type(component_value) {
            has_list_style_type = true;
            continue;
        }

        return false;
    }

    if found_nones > 2 {
        return false;
    }

    // https://drafts.csswg.org/css-lists-3/#propdef-list-style
    // <'list-style-position'> || <'list-style-image'> || <'list-style-type'>
    //
    // Since `none` is valid for both list-style-image and list-style-type, the
    // shorthand needs to defer assigning it until the unambiguous components are
    // known.
    if found_nones == 2 {
        return !has_list_style_image && !has_list_style_type;
    }

    found_nones == 0 || !has_list_style_image || !has_list_style_type
}

pub(super) fn component_values_parse_as_exact_ratio(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-values-4/#ratios
    // <ratio> = <number [0,∞]> [ / <number [0,∞]> ]?
    match component_values {
        [numerator] => component_value_parse_as_non_negative_number(numerator),
        [numerator, slash, denominator] => {
            component_value_parse_as_non_negative_number(numerator)
                && component_value_is_delim(Some(slash), '/')
                && component_value_parse_as_non_negative_number(denominator)
        }
        _ => false,
    }
}

pub(super) fn parse_border_radius_shorthand_side(component_values: &[ComponentValue]) -> bool {
    matches!(component_values.len(), 1..=4)
        && component_values
            .iter()
            .all(component_value_parse_as_non_negative_length_percentage)
}

pub(super) fn rust_owned_border_radius_shorthand_side_values(
    component_values: &[ComponentValue],
    source: &str,
) -> Option<Vec<RustOwnedNestedPrimitiveValue>> {
    if !parse_border_radius_shorthand_side(component_values) {
        return None;
    }

    component_values
        .iter()
        .map(|component_value| component_value_parse_as_nested_length_percentage(component_value, source))
        .collect()
}

pub(super) fn parse_single_column_component_value(
    property_id: PropertyId,
    component_values: &[ComponentValue],
    source: &str,
) -> bool {
    let Some(source) = serialize_component_values_for_reparsing(component_values, source) else {
        return false;
    };

    matches!(
        parse_rust_owned_style_value_for_property(&[property_id as u16], source.as_bytes()),
        RustOwnedStyleValueParseResult::Parsed(_)
    )
}

pub(super) fn component_value_is_shape_box(component_value: &ComponentValue) -> bool {
    rust_owned_shape_box_from_component_value(component_value).is_some()
}

pub(super) fn rust_owned_shape_box_from_component_value(component_value: &ComponentValue) -> Option<RustOwnedShapeBox> {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    }) = component_value
    else {
        return None;
    };

    // https://drafts.csswg.org/css-shapes-1/#typedef-shape-box
    // <shape-box> = <box> | margin-box
    if value.eq_ignore_ascii_case("content-box") {
        return Some(RustOwnedShapeBox::Content);
    }
    if value.eq_ignore_ascii_case("padding-box") {
        return Some(RustOwnedShapeBox::Padding);
    }
    if value.eq_ignore_ascii_case("border-box") {
        return Some(RustOwnedShapeBox::Border);
    }
    if value.eq_ignore_ascii_case("margin-box") {
        return Some(RustOwnedShapeBox::Margin);
    }
    None
}

pub(super) const TEXT_DECORATION_LINE_NONE: u8 = 1 << 0;
pub(super) const TEXT_DECORATION_LINE_UNDERLINE: u8 = 1 << 1;
pub(super) const TEXT_DECORATION_LINE_OVERLINE: u8 = 1 << 2;
pub(super) const TEXT_DECORATION_LINE_LINE_THROUGH: u8 = 1 << 3;
pub(super) const TEXT_DECORATION_LINE_BLINK: u8 = 1 << 4;
pub(super) const TEXT_DECORATION_LINE_SPELLING_ERROR: u8 = 1 << 5;
pub(super) const TEXT_DECORATION_LINE_GRAMMAR_ERROR: u8 = 1 << 6;

pub(super) fn component_values_parse_as_text_decoration_line(component_values: &[ComponentValue]) -> Option<u8> {
    // https://drafts.csswg.org/css-text-decor-4/#text-decoration-line-property
    // none | [ underline || overline || line-through || blink ] | spelling-error | grammar-error
    match component_values {
        [component_value] if component_value_is_ident(Some(component_value), "none") => {
            return Some(TEXT_DECORATION_LINE_NONE);
        }
        [component_value] if component_value_is_ident(Some(component_value), "spelling-error") => {
            return Some(TEXT_DECORATION_LINE_SPELLING_ERROR);
        }
        [component_value] if component_value_is_ident(Some(component_value), "grammar-error") => {
            return Some(TEXT_DECORATION_LINE_GRAMMAR_ERROR);
        }
        [] => return None,
        _ => {}
    }

    let mut has_underline = false;
    let mut has_overline = false;
    let mut has_line_through = false;
    let mut has_blink = false;

    for component_value in component_values {
        if component_value_is_ident(Some(component_value), "underline") {
            if has_underline {
                return None;
            }
            has_underline = true;
            continue;
        }

        if component_value_is_ident(Some(component_value), "overline") {
            if has_overline {
                return None;
            }
            has_overline = true;
            continue;
        }

        if component_value_is_ident(Some(component_value), "line-through") {
            if has_line_through {
                return None;
            }
            has_line_through = true;
            continue;
        }

        if component_value_is_ident(Some(component_value), "blink") {
            if has_blink {
                return None;
            }
            has_blink = true;
            continue;
        }

        return None;
    }

    let mut bits = 0;
    if has_underline {
        bits |= TEXT_DECORATION_LINE_UNDERLINE;
    }
    if has_overline {
        bits |= TEXT_DECORATION_LINE_OVERLINE;
    }
    if has_line_through {
        bits |= TEXT_DECORATION_LINE_LINE_THROUGH;
    }
    if has_blink {
        bits |= TEXT_DECORATION_LINE_BLINK;
    }

    (bits != 0).then_some(bits)
}

pub(super) fn component_value_is_text_decoration_line(component_value: &ComponentValue) -> bool {
    [
        "none",
        "underline",
        "overline",
        "line-through",
        "blink",
        "spelling-error",
        "grammar-error",
    ]
    .iter()
    .any(|line| component_value_is_ident(Some(component_value), line))
}

pub(super) fn component_value_is_text_decoration_style(component_value: &ComponentValue) -> bool {
    ["solid", "double", "dotted", "dashed", "wavy"]
        .iter()
        .any(|style| component_value_is_ident(Some(component_value), style))
}

pub(super) fn rust_owned_text_decoration_style_from_component_value(
    component_value: &ComponentValue,
) -> Option<RustOwnedTextDecorationStyle> {
    let ident = component_value_ident(component_value)?;
    if ident.eq_ignore_ascii_case("solid") {
        return Some(RustOwnedTextDecorationStyle::Solid);
    }
    if ident.eq_ignore_ascii_case("double") {
        return Some(RustOwnedTextDecorationStyle::Double);
    }
    if ident.eq_ignore_ascii_case("dotted") {
        return Some(RustOwnedTextDecorationStyle::Dotted);
    }
    if ident.eq_ignore_ascii_case("dashed") {
        return Some(RustOwnedTextDecorationStyle::Dashed);
    }
    if ident.eq_ignore_ascii_case("wavy") {
        return Some(RustOwnedTextDecorationStyle::Wavy);
    }
    None
}

pub(super) fn component_value_parse_as_text_decoration_thickness(component_value: &ComponentValue) -> bool {
    // https://drafts.csswg.org/css-text-decor-4/#text-decoration-thickness-property
    // auto | from-font | <length> | <percentage>
    component_value_is_ident(Some(component_value), "auto")
        || component_value_is_ident(Some(component_value), "from-font")
        || component_value_parse_as_length(component_value)
        || parse_percentage_value_prefix(component_value) == CssPrimitiveValueKind::Percentage
}

pub(super) fn rust_owned_text_decoration_thickness_from_component_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if component_value_is_ident(Some(component_value), "auto") {
        return Some(auto_keyword());
    }
    if component_value_is_ident(Some(component_value), "from-font") {
        return Some(RustOwnedNestedPrimitiveValue::Keyword("from-font".to_string()));
    }
    component_value_parse_as_nested_length_percentage(component_value, source)
}

pub(super) fn component_value_is_list_style_position(component_value: &ComponentValue) -> bool {
    component_value_is_ident(Some(component_value), "inside")
        || component_value_is_ident(Some(component_value), "outside")
}

pub(super) fn rust_owned_list_style_position_from_component_value(
    component_value: &ComponentValue,
) -> Option<RustOwnedListStylePosition> {
    let ident = component_value_ident(component_value)?;
    if ident.eq_ignore_ascii_case("inside") {
        return Some(RustOwnedListStylePosition::Inside);
    }
    if ident.eq_ignore_ascii_case("outside") {
        return Some(RustOwnedListStylePosition::Outside);
    }
    None
}

pub(super) fn component_value_parse_as_list_style_image(component_value: &ComponentValue) -> bool {
    component_value_parse_as_image_set_image(component_value)
        || component_value_parse_as_image_gradient(component_value)
        || matches!(
            component_value,
            ComponentValue::Function(function) if component_value_parse_as_image_set_function(function)
        )
}

pub(super) fn rust_owned_list_style_image_from_component_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedListStyleImage> {
    if component_value_parse_as_list_style_image(component_value) {
        return rust_owned_image_from_component_value(component_value, source).map(RustOwnedListStyleImage::Image);
    }
    None
}

pub(super) fn component_value_parse_as_list_style_type(component_value: &ComponentValue) -> bool {
    parse_string_value_prefix(component_value) == CssPrimitiveValueKind::String || {
        let mut parser = ComponentValueParser::new(vec![component_value.clone()]);
        parser.parse_a_counter_style().is_some()
    }
}

pub(super) fn rust_owned_list_style_type_from_component_value(
    component_value: &ComponentValue,
) -> Option<RustOwnedListStyleType> {
    if let ComponentValue::PreservedToken(Token {
        token_type: TokenType::String { value },
        ..
    }) = component_value
    {
        return Some(RustOwnedListStyleType::String(value.clone()));
    }

    let mut parser = ComponentValueParser::new(vec![component_value.clone()]);
    Some(RustOwnedListStyleType::CounterStyle(parser.parse_a_counter_style()?))
}

pub(crate) fn parse_filter_value_list_value(filtered_input: &[u8]) -> bool {
    parse_rust_owned_filter_value_list_value(filtered_input).is_some()
}

pub(super) fn parse_rust_owned_filter_value_list_value(filtered_input: &[u8]) -> Option<RustOwnedFilterValueList> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    if component_values.is_empty() {
        return None;
    }

    if component_values.len() == 1 && component_value_is_ident(component_values.first(), "none") {
        return Some(RustOwnedFilterValueList::None);
    }

    // https://drafts.fxtf.org/filter-effects-1/#typedef-filter-value-list
    // <filter-value-list> = [ <filter-function> | <url> ]+
    let mut filters = Vec::new();
    for component_value in component_values {
        let mut url_parser = ComponentValueParser::new(vec![component_value.clone()]);
        if url_parser.parse_a_url_function().is_some() {
            filters.push(RustOwnedFilterValue::Url(rust_owned_url_from_component_value(
                &component_value,
                filtered_input_string,
            )?));
            continue;
        }

        let ComponentValue::Function(function) = component_value else {
            return None;
        };

        filters.push(component_value_parse_as_filter_function(
            &function,
            filtered_input_string,
        )?);
    }

    Some(RustOwnedFilterValueList::Filters(filters))
}

pub(super) fn component_value_parse_as_filter_function(
    function: &Function,
    source: &str,
) -> Option<RustOwnedFilterValue> {
    // https://drafts.fxtf.org/filter-effects-1/#typedef-filter-function
    // <blur()> | <brightness()> | <contrast()> | <drop-shadow()> | <grayscale()> | <hue-rotate()> | <invert()> | <opacity()> | <sepia()> | <saturate()>
    if function.name.eq_ignore_ascii_case("blur") {
        return component_values_parse_as_blur_function(&function.value, source);
    }
    if function.name.eq_ignore_ascii_case("drop-shadow") {
        return component_values_parse_as_drop_shadow_function(&function.value, source);
    }
    if function.name.eq_ignore_ascii_case("hue-rotate") {
        return component_values_parse_as_hue_rotate_function(&function.value, source);
    }

    let simple_function = if function.name.eq_ignore_ascii_case("brightness") {
        RustOwnedSimpleFilterFunction::Brightness
    } else if function.name.eq_ignore_ascii_case("contrast") {
        RustOwnedSimpleFilterFunction::Contrast
    } else if function.name.eq_ignore_ascii_case("grayscale") {
        RustOwnedSimpleFilterFunction::Grayscale
    } else if function.name.eq_ignore_ascii_case("invert") {
        RustOwnedSimpleFilterFunction::Invert
    } else if function.name.eq_ignore_ascii_case("opacity") {
        RustOwnedSimpleFilterFunction::Opacity
    } else if function.name.eq_ignore_ascii_case("saturate") {
        RustOwnedSimpleFilterFunction::Saturate
    } else if function.name.eq_ignore_ascii_case("sepia") {
        RustOwnedSimpleFilterFunction::Sepia
    } else {
        return None;
    };

    component_values_parse_as_simple_filter_function(&function.value, simple_function, source)
}

pub(super) fn component_values_parse_as_blur_function(
    component_values: &[ComponentValue],
    source: &str,
) -> Option<RustOwnedFilterValue> {
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-blur
    // blur( <length>? )
    let component_values = strip_whitespace(component_values);
    match component_values {
        [] => Some(RustOwnedFilterValue::Blur { radius: None }),
        [component_value] if component_value_parse_as_non_negative_length(component_value) => {
            Some(RustOwnedFilterValue::Blur {
                radius: Some(component_value_parse_as_nested_length(component_value, source)?),
            })
        }
        _ => None,
    }
}

pub(super) fn component_values_parse_as_drop_shadow_function(
    component_values: &[ComponentValue],
    source: &str,
) -> Option<RustOwnedFilterValue> {
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-drop-shadow
    // drop-shadow( [ <color>? && <length>{2,3} ] )
    let component_values = strip_whitespace(component_values);
    if component_values.is_empty() {
        return None;
    }

    let mut parser = ComponentValueParser::new(component_values.to_vec());
    let color_before_lengths = consume_filter_drop_shadow_color(&mut parser, source);
    let offset_x = consume_filter_drop_shadow_length(&mut parser, source)?;
    let offset_y = consume_filter_drop_shadow_length(&mut parser, source)?;

    let radius = consume_filter_drop_shadow_length(&mut parser, source);
    let color = if color_before_lengths.is_some() {
        color_before_lengths
    } else {
        consume_filter_drop_shadow_color(&mut parser, source)
    };

    (!parser.has_next_component_value()).then_some(RustOwnedFilterValue::DropShadow {
        color,
        offset_x,
        offset_y,
        radius,
    })
}

pub(super) fn consume_filter_drop_shadow_color(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedColor> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value().cloned()?;

    if let Some(color) = rust_owned_color_from_component_value(&component_value, source) {
        parser.index += 1;
        return Some(color);
    }

    None
}

pub(super) fn consume_filter_drop_shadow_length(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value().cloned()?;

    if component_value_parse_as_length(&component_value) {
        parser.index += 1;
        return component_value_parse_as_nested_length(&component_value, source);
    }

    None
}

pub(super) fn component_values_parse_as_hue_rotate_function(
    component_values: &[ComponentValue],
    source: &str,
) -> Option<RustOwnedFilterValue> {
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-hue-rotate
    // hue-rotate( [ <angle> | <zero> ]? )
    let component_values = strip_whitespace(component_values);
    match component_values {
        [] => Some(RustOwnedFilterValue::HueRotate { angle: None }),
        [component_value] => {
            if component_value_parse_as_angle(component_value) {
                return Some(RustOwnedFilterValue::HueRotate {
                    angle: Some(component_value_parse_as_nested_angle(component_value, source)?),
                });
            }

            matches!(
                component_value,
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Number { number },
                    ..
                }) if number_is_integer(*number) && number.value() == 0.0
            )
            .then_some(RustOwnedFilterValue::HueRotate { angle: None })
        }
        _ => None,
    }
}

pub(super) fn component_values_parse_as_simple_filter_function(
    component_values: &[ComponentValue],
    function: RustOwnedSimpleFilterFunction,
    source: &str,
) -> Option<RustOwnedFilterValue> {
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-brightness
    // brightness( [ <number> | <percentage> ]? )
    //
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-contrast
    // contrast( [ <number> | <percentage> ]? )
    //
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-grayscale
    // grayscale( [ <number> | <percentage> ]? )
    //
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-invert
    // invert( [ <number> | <percentage> ]? )
    //
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-opacity
    // opacity( [ <number> | <percentage> ]? )
    //
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-sepia
    // sepia( [ <number> | <percentage> ]? )
    //
    // https://drafts.fxtf.org/filter-effects-1/#funcdef-filter-saturate
    // saturate( [ <number> | <percentage> ]? )
    let component_values = strip_whitespace(component_values);
    match component_values {
        [] => Some(RustOwnedFilterValue::Simple { function, amount: None }),
        [component_value] if component_value_parse_as_filter_amount(component_value) => {
            Some(RustOwnedFilterValue::Simple {
                function,
                amount: Some(component_value_parse_as_nested_number_percentage(
                    component_value,
                    source,
                )?),
            })
        }
        _ => None,
    }
}

pub(super) fn component_value_parse_as_nested_length(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Length)) => {
            Some(RustOwnedNestedPrimitiveValue::Length {
                value: number.value(),
                unit: unit.to_string(),
            })
        }
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if number.value() == 0.0 => Some(RustOwnedNestedPrimitiveValue::Length {
            value: 0.0,
            unit: "px".to_string(),
        }),
        ComponentValue::Function(_) => parse_rust_owned_math_function(
            PropertyValueType::Length,
            std::slice::from_ref(component_value),
            source.as_bytes(),
        )
        .map(RustOwnedNestedPrimitiveValue::MathFunction)
        .or_else(|| {
            parse_rust_owned_tree_counting_function(PropertyValueType::Length, std::slice::from_ref(component_value))
                .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
        })
        .or_else(|| {
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source)
                .map(RustOwnedNestedPrimitiveValue::Source)
        }),
        _ => None,
    }
}

pub(super) fn zero_pixel_length() -> RustOwnedNestedPrimitiveValue {
    RustOwnedNestedPrimitiveValue::Length {
        value: 0.0,
        unit: "px".to_string(),
    }
}

pub(super) fn auto_keyword() -> RustOwnedNestedPrimitiveValue {
    RustOwnedNestedPrimitiveValue::Keyword("auto".to_string())
}

pub(super) fn component_value_parse_as_nested_length_percentage(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { number },
            ..
        }) => Some(RustOwnedNestedPrimitiveValue::Percentage(number.value())),
        _ => component_value_parse_as_nested_length(component_value, source),
    }
}

pub(super) fn component_value_parse_as_nested_number(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => Some(RustOwnedNestedPrimitiveValue::Number(number.value())),
        ComponentValue::Function(_) => parse_rust_owned_math_function(
            PropertyValueType::Number,
            std::slice::from_ref(component_value),
            source.as_bytes(),
        )
        .map(RustOwnedNestedPrimitiveValue::MathFunction)
        .or_else(|| {
            parse_rust_owned_tree_counting_function(PropertyValueType::Number, std::slice::from_ref(component_value))
                .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
        })
        .or_else(|| {
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source)
                .map(RustOwnedNestedPrimitiveValue::Source)
        }),
        _ => None,
    }
}

pub(super) fn component_value_parse_as_nested_angle(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) if matches!(dimension_for_unit(unit), Some(DimensionType::Angle)) => {
            Some(RustOwnedNestedPrimitiveValue::Angle {
                value: number.value(),
                unit: unit.to_string(),
            })
        }
        ComponentValue::Function(_) => parse_rust_owned_math_function(
            PropertyValueType::Angle,
            std::slice::from_ref(component_value),
            source.as_bytes(),
        )
        .map(RustOwnedNestedPrimitiveValue::MathFunction)
        .or_else(|| {
            parse_rust_owned_tree_counting_function(PropertyValueType::Angle, std::slice::from_ref(component_value))
                .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
        })
        .or_else(|| {
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source)
                .map(RustOwnedNestedPrimitiveValue::Source)
        }),
        _ => None,
    }
}

pub(super) fn component_value_parse_as_nested_number_percentage(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => Some(RustOwnedNestedPrimitiveValue::Number(number.value())),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { number },
            ..
        }) => Some(RustOwnedNestedPrimitiveValue::Percentage(number.value())),
        ComponentValue::Function(_) => parse_rust_owned_math_function(
            PropertyValueType::Number,
            std::slice::from_ref(component_value),
            source.as_bytes(),
        )
        .map(RustOwnedNestedPrimitiveValue::MathFunction)
        .or_else(|| {
            parse_rust_owned_tree_counting_function(PropertyValueType::Number, std::slice::from_ref(component_value))
                .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
        })
        .or_else(|| {
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source)
                .map(RustOwnedNestedPrimitiveValue::Source)
        }),
        _ => None,
    }
}

pub(super) fn component_value_parse_as_nested_non_negative_number_percentage(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if !component_value_parse_as_non_negative_number_percentage(component_value) {
        return None;
    }
    component_value_parse_as_nested_number_percentage(component_value, source)
}

pub(super) fn component_value_parse_as_nested_non_negative_length(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if !component_value_parse_as_non_negative_length(component_value) {
        return None;
    }
    component_value_parse_as_nested_length(component_value, source)
}

pub(super) fn component_value_parse_as_nested_non_negative_number_length_percentage(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if component_value_parse_as_non_negative_number(component_value) {
        return component_value_parse_as_nested_non_negative_number(component_value, source);
    }
    if component_value_parse_as_non_negative_length_percentage(component_value) {
        return component_value_parse_as_nested_length_percentage(component_value, source);
    }
    None
}

pub(super) fn component_value_parse_as_nested_dasharray_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if number.value() >= 0.0 => Some(RustOwnedNestedPrimitiveValue::Number(number.value())),
        ComponentValue::Function(_) => parse_rust_owned_math_function(
            PropertyValueType::LengthPercentage,
            std::slice::from_ref(component_value),
            source.as_bytes(),
        )
        .map(RustOwnedNestedPrimitiveValue::MathFunction)
        .or_else(|| {
            parse_rust_owned_tree_counting_function(
                PropertyValueType::LengthPercentage,
                std::slice::from_ref(component_value),
            )
            .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
        })
        .or_else(|| {
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source)
                .map(RustOwnedNestedPrimitiveValue::Source)
        }),
        _ => component_value_parse_as_nested_length_percentage(component_value, source),
    }
}

pub(super) fn component_value_parse_as_nested_integer(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => numeric_value_to_i32(*number).map(RustOwnedNestedPrimitiveValue::Integer),
        ComponentValue::Function(_) => parse_rust_owned_math_function(
            PropertyValueType::Number,
            std::slice::from_ref(component_value),
            source.as_bytes(),
        )
        .map(RustOwnedNestedPrimitiveValue::MathFunction)
        .or_else(|| {
            parse_rust_owned_tree_counting_function(PropertyValueType::Number, std::slice::from_ref(component_value))
                .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
        })
        .or_else(|| {
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source)
                .map(RustOwnedNestedPrimitiveValue::Source)
        }),
        _ => None,
    }
}

pub(super) fn component_value_parse_as_nested_percentage(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { number },
            ..
        }) => Some(RustOwnedNestedPrimitiveValue::Percentage(number.value())),
        ComponentValue::Function(_) => parse_rust_owned_math_function(
            PropertyValueType::Number,
            std::slice::from_ref(component_value),
            source.as_bytes(),
        )
        .map(RustOwnedNestedPrimitiveValue::MathFunction)
        .or_else(|| {
            parse_rust_owned_tree_counting_function(PropertyValueType::Number, std::slice::from_ref(component_value))
                .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
        })
        .or_else(|| {
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source)
                .map(RustOwnedNestedPrimitiveValue::Source)
        }),
        _ => None,
    }
}

pub(super) fn component_value_parse_as_nested_non_negative_number(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if number.value() >= 0.0 => Some(RustOwnedNestedPrimitiveValue::Number(number.value())),
        ComponentValue::Function(_) => parse_rust_owned_math_function(
            PropertyValueType::Number,
            std::slice::from_ref(component_value),
            source.as_bytes(),
        )
        .map(RustOwnedNestedPrimitiveValue::MathFunction)
        .or_else(|| {
            parse_rust_owned_tree_counting_function(PropertyValueType::Number, std::slice::from_ref(component_value))
                .map(RustOwnedNestedPrimitiveValue::TreeCountingFunction)
        })
        .or_else(|| {
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source)
                .map(RustOwnedNestedPrimitiveValue::Source)
        }),
        _ => None,
    }
}

pub(super) fn component_value_parse_as_filter_amount(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::Function(_) => true,
        _ => component_value_parse_as_non_negative_number_percentage(component_value),
    }
}

pub(super) fn component_value_parse_as_non_negative_number_percentage(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => number.value() >= 0.0,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { number },
            ..
        }) => number.value() >= 0.0,
        // AD-HOC: The Rust side only recognizes the syntactic branch here.
        // Materializing and range-checking math functions still happens in C++.
        ComponentValue::Function(function) => {
            is_math_function_name(&function.name) || function.name.eq_ignore_ascii_case("random")
        }
        _ => false,
    }
}

pub(crate) fn parse_content_value(filtered_input: &[u8]) -> bool {
    parse_rust_owned_content_value(filtered_input).is_some()
        || collect_substitution_function_presence(filtered_input).is_some_and(|presence| presence.0)
}

pub(super) fn parse_rust_owned_content_value(filtered_input: &[u8]) -> Option<RustOwnedContent> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    match component_values.as_slice() {
        [component_value] if component_value_is_ident(Some(component_value), "normal") => {
            return Some(RustOwnedContent::Normal);
        }
        [component_value] if component_value_is_ident(Some(component_value), "none") => {
            return Some(RustOwnedContent::None);
        }
        [] => return None,
        _ => {}
    }

    let mut content_values = Vec::new();
    let mut alt_text_values = Vec::new();
    let mut in_alt_text = false;

    for component_value in &component_values {
        if component_value_is_delim(Some(component_value), '/') {
            if in_alt_text || content_values.is_empty() {
                return None;
            }
            in_alt_text = true;
            continue;
        }

        if in_alt_text {
            alt_text_values.push(component_value_parse_as_content_alt_text(
                component_value,
                filtered_input_string,
            )?);
            continue;
        }

        content_values.push(component_value_parse_as_content_item(
            component_value,
            filtered_input_string,
        )?);
    }

    if content_values.is_empty() || (in_alt_text && alt_text_values.is_empty()) {
        return None;
    }

    Some(RustOwnedContent::Items {
        items: content_values,
        alt_text: alt_text_values,
    })
}

pub(super) fn component_value_parse_as_content_item(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedContentItem> {
    // https://drafts.csswg.org/css-content-3/#content-property
    // content: normal | none | [ <content-replacement> | <content-list> ] [/ [ <string> | <counter> | <attr()> ]+ ]?
    //
    // https://drafts.csswg.org/css-content-3/#typedef-content-list
    // <content-list> = [ <string> | contents | <image> | <quote> | <target> | <leader()> ]+
    //
    // https://drafts.csswg.org/css-content-3/#typedef-quote
    // <quote> = open-quote | close-quote | no-open-quote | no-close-quote
    if component_value_parse_as_content_quote(component_value) {
        return serialize_component_values_for_reparsing(std::slice::from_ref(component_value), filtered_input_string)
            .map(RustOwnedContentItem::Quote);
    }

    if parse_string_value_prefix(component_value) == CssPrimitiveValueKind::String {
        return component_values_string_value(std::slice::from_ref(component_value))
            .map(|value| RustOwnedContentItem::String(value.to_string()));
    }

    if component_value_parse_as_content_image(component_value) {
        return rust_owned_image_from_component_value(component_value, filtered_input_string)
            .map(RustOwnedContentItem::Image);
    }

    if let Some(counter) = component_value_parse_as_content_counter(component_value, filtered_input_string) {
        return Some(RustOwnedContentItem::Counter(counter));
    }

    None
}

pub(super) fn component_value_parse_as_content_alt_text(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedContentAltTextItem> {
    // https://drafts.csswg.org/css-content-3/#content-property
    // / [ <string> | <counter> | <attr()> ]+
    //
    // NB: <attr()> is handled as an arbitrary substitution function before
    // property-specific Rust-owned value parsing.
    if parse_string_value_prefix(component_value) == CssPrimitiveValueKind::String {
        return component_values_string_value(std::slice::from_ref(component_value))
            .map(|value| RustOwnedContentAltTextItem::String(value.to_string()));
    }

    if let Some(counter) = component_value_parse_as_content_counter(component_value, filtered_input_string) {
        return Some(RustOwnedContentAltTextItem::Counter(counter));
    }

    None
}

pub(super) fn component_value_parse_as_content_quote(component_value: &ComponentValue) -> bool {
    ["open-quote", "close-quote", "no-open-quote", "no-close-quote"]
        .iter()
        .any(|quote| component_value_is_ident(Some(component_value), quote))
}

pub(super) fn component_value_parse_as_content_image(component_value: &ComponentValue) -> bool {
    component_value_parse_as_image_url(component_value)
        || component_value_parse_as_image_gradient(component_value)
        || matches!(
            component_value,
            ComponentValue::Function(function) if component_value_parse_as_image_set_function(function)
        )
}

pub(super) fn component_value_parse_as_content_counter(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedCounterFunction> {
    let ComponentValue::Function(function) = component_value else {
        return None;
    };

    if function.name.eq_ignore_ascii_case("counter") {
        return rust_owned_counter_function_value(function, filtered_input_string);
    }

    if function.name.eq_ignore_ascii_case("counters") {
        return rust_owned_counters_function_value(function, filtered_input_string);
    }

    None
}

pub(super) fn parse_cursor_predefined(component_values: &[ComponentValue]) -> Option<String> {
    let component_values = remove_whitespace_component_values(component_values);
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values.as_slice()
    else {
        return None;
    };

    property_accepts_keyword(PropertyId::Cursor, value).then(|| value.to_string())
}

pub(super) fn parse_cursor_image(component_values: &[ComponentValue], source: &str) -> Option<RustOwnedCursorImage> {
    let component_values = remove_whitespace_component_values(component_values);
    let (image, coordinates) = component_values.split_first()?;

    let image = rust_owned_image_from_component_value(image, source)?;

    match coordinates {
        [] => Some(RustOwnedCursorImage {
            image,
            x: None,
            y: None,
        }),
        [x, y] if component_value_parse_as_number_prefix(x) && component_value_parse_as_number_prefix(y) => {
            Some(RustOwnedCursorImage {
                image,
                x: Some(component_value_parse_as_nested_number(x, source)?),
                y: Some(component_value_parse_as_nested_number(y, source)?),
            })
        }
        _ => None,
    }
}

pub(super) fn parse_single_shadow_value(
    component_values: Vec<ComponentValue>,
    filtered_input_string: &str,
    is_box_shadow: bool,
) -> Option<RustOwnedSingleShadow> {
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    let mut color = None;
    let mut offset_x = None;
    let mut offset_y = None;
    let mut blur_radius = None;
    let mut spread_distance = None;
    let mut placement = RustOwnedShadowPlacement::Outer;
    let mut has_placement = false;

    while parser.has_next_component_value() {
        let component_value = parser.next_component_value().unwrap();
        if color.is_none()
            && let Some(value) = rust_owned_color_from_component_value(component_value, filtered_input_string)
        {
            color = Some(value);
            parser.index += 1;
            continue;
        }

        if is_box_shadow
            && !has_placement
            && let ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { .. },
                ..
            }) = component_value
            && parser.consume_ident_matching("inset")
        {
            placement = RustOwnedShadowPlacement::Inner;
            has_placement = true;
            continue;
        }

        if offset_x.is_none() {
            let parsed_offset_x =
                consume_nested_length_matching(&mut parser, filtered_input_string, component_value_parse_as_length)?;
            let parsed_offset_y =
                consume_nested_length_matching(&mut parser, filtered_input_string, component_value_parse_as_length)?;

            let parsed_blur_radius = consume_nested_length_matching(
                &mut parser,
                filtered_input_string,
                component_value_parse_as_non_negative_length,
            );
            let parsed_spread_distance = if parsed_blur_radius.is_some() && is_box_shadow {
                consume_nested_length_matching(&mut parser, filtered_input_string, component_value_parse_as_length)
            } else {
                None
            };

            offset_x = Some(parsed_offset_x);
            offset_y = Some(parsed_offset_y);
            blur_radius = parsed_blur_radius;
            spread_distance = parsed_spread_distance;
            continue;
        }

        return None;
    }

    Some(RustOwnedSingleShadow {
        color,
        offset_x: offset_x?,
        offset_y: offset_y?,
        blur_radius,
        spread_distance,
        placement,
    })
}

pub(super) fn consume_nested_length_matching<F>(
    parser: &mut ComponentValueParser,
    source: &str,
    predicate: F,
) -> Option<RustOwnedNestedPrimitiveValue>
where
    F: Fn(&ComponentValue) -> bool,
{
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    if predicate(component_value) {
        let value = component_value_parse_as_nested_length(component_value, source)?;
        parser.index += 1;
        return Some(value);
    }

    None
}

pub(super) fn consume_transform_origin_component(
    parser: &mut ComponentValueParser,
) -> Option<TransformOriginComponent> {
    parser.discard_whitespace();
    let component_value = parser.next_component_value()?;
    if component_value_parse_as_length_percentage(component_value) {
        parser.index += 1;
        return Some(TransformOriginComponent {
            axis: None,
            is_offset: true,
        });
    }

    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    }) = component_value
    else {
        return None;
    };

    let axis = if value.eq_ignore_ascii_case("left") || value.eq_ignore_ascii_case("right") {
        Some(TransformOriginAxis::X)
    } else if value.eq_ignore_ascii_case("top") || value.eq_ignore_ascii_case("bottom") {
        Some(TransformOriginAxis::Y)
    } else if value.eq_ignore_ascii_case("center") {
        None
    } else {
        return None;
    };

    parser.index += 1;
    Some(TransformOriginComponent { axis, is_offset: false })
}

pub(super) fn consume_length_component_value(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };
    if !component_value_parse_as_length(component_value) {
        return false;
    }
    parser.index += 1;
    true
}

pub(super) fn consume_non_negative_length_component_value(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };
    if !component_value_parse_as_non_negative_length(component_value) {
        return false;
    }
    parser.index += 1;
    true
}

pub(super) fn consume_number_component_value(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };
    if !component_value_parse_as_number_prefix(component_value) {
        return false;
    }
    parser.index += 1;
    true
}

pub(super) fn consume_number_percentage_component_value(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();
    let Some(component_value) = parser.next_component_value() else {
        return false;
    };
    if !component_value_parse_as_number_percentage(component_value) {
        return false;
    }
    parser.index += 1;
    true
}

pub(super) fn component_value_parse_as_angle_for_transform_longhand(component_value: &ComponentValue) -> bool {
    match component_value {
        ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("random") => {
            component_values_contain_angle_dimension(&function.value)
        }
        _ => component_value_parse_as_angle(component_value),
    }
}

pub(super) fn component_values_contain_angle_dimension(component_values: &[ComponentValue]) -> bool {
    component_values.iter().any(|component_value| match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { unit, .. },
            ..
        }) => matches!(dimension_for_unit(unit), Some(DimensionType::Angle)),
        ComponentValue::Function(function) => component_values_contain_angle_dimension(&function.value),
        ComponentValue::SimpleBlock(block) => component_values_contain_angle_dimension(&block.value),
        _ => false,
    })
}

pub(super) fn component_value_parse_as_number_percentage(component_value: &ComponentValue) -> bool {
    component_value_parse_as_number_prefix(component_value)
        || matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Percentage { .. },
                ..
            }) | ComponentValue::Function(_)
        )
}

pub(super) fn parse_view_function_value_with_axis_first(
    component_values: Vec<ComponentValue>,
) -> Option<CssViewFunctionValue> {
    let mut parser = ComponentValueParser::new(component_values);
    let axis = parse_view_function_axis(&mut parser);
    let inset = parse_view_timeline_inset_prefix(&mut parser, None);

    parser.discard_whitespace();
    if parser.has_next_component_value() || (axis.is_none() && inset.is_none()) {
        return None;
    }

    Some(CssViewFunctionValue {
        kind: CssViewFunctionValueKind::Valid,
        axis: axis.unwrap_or(CssScrollFunctionAxisKind::None),
        inset: inset
            .as_ref()
            .map(|inset| inset.kind)
            .unwrap_or(CssViewFunctionInsetKind::None),
        inset_position: inset.as_ref().map_or(CssViewFunctionInsetPosition::None, |_| {
            if axis.is_some() {
                CssViewFunctionInsetPosition::AfterAxis
            } else {
                CssViewFunctionInsetPosition::BeforeAxis
            }
        }),
    })
}

pub(super) fn parse_view_function_value_with_inset_first(
    component_values: Vec<ComponentValue>,
) -> Option<CssViewFunctionValue> {
    let mut parser = ComponentValueParser::new(component_values);
    let inset = parse_view_timeline_inset_prefix(&mut parser, None);
    let axis = parse_view_function_axis(&mut parser);

    parser.discard_whitespace();
    if parser.has_next_component_value() || (axis.is_none() && inset.is_none()) {
        return None;
    }

    Some(CssViewFunctionValue {
        kind: CssViewFunctionValueKind::Valid,
        axis: axis.unwrap_or(CssScrollFunctionAxisKind::None),
        inset: inset
            .as_ref()
            .map(|inset| inset.kind)
            .unwrap_or(CssViewFunctionInsetKind::None),
        inset_position: inset
            .as_ref()
            .map(|_| CssViewFunctionInsetPosition::BeforeAxis)
            .unwrap_or(CssViewFunctionInsetPosition::None),
    })
}

pub(super) fn parse_rect_side(parser: &mut ComponentValueParser) -> bool {
    parser.discard_whitespace();

    if parser.consume_ident_matching("auto") {
        return true;
    }

    let Some(component_value) = parser.next_component_value() else {
        return false;
    };

    if component_value_parse_as_length(component_value) {
        parser.index += 1;
        return true;
    }

    false
}

pub(super) fn parse_non_negative_number_prefix(parser: &mut ComponentValueParser) -> bool {
    parse_non_negative_number_prefix_value(parser).is_some()
}

pub(super) fn parse_non_negative_number_prefix_value(parser: &mut ComponentValueParser) -> Option<f64> {
    parser.discard_whitespace();

    let component_value = parser.next_component_value()?;

    if let Some(value) = component_value_non_negative_number_value(component_value) {
        parser.index += 1;
        return Some(value);
    }

    if component_value_parse_as_non_negative_number(component_value) {
        parser.index += 1;
        return Some(0.0);
    }

    None
}

pub(super) struct ViewTimelineInsetPrefix {
    pub(super) kind: CssViewFunctionInsetKind,
    pub(super) count: usize,
    pub(super) values: Vec<RustOwnedNestedPrimitiveValue>,
}

pub(super) fn parse_view_function_axis(parser: &mut ComponentValueParser) -> Option<CssScrollFunctionAxisKind> {
    parser.discard_whitespace();
    let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    })) = parser.next_component_value()
    else {
        return None;
    };
    let axis = scroll_function_axis_from_string(value)?;
    parser.index += 1;
    Some(axis)
}

pub(super) fn parse_view_timeline_inset_prefix(
    parser: &mut ComponentValueParser,
    filtered_input: Option<&str>,
) -> Option<ViewTimelineInsetPrefix> {
    let mut count = 0;
    let mut all_auto = true;
    let mut values = Vec::new();

    while count < 2 {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };

        if component_value_is_ident(Some(component_value), "auto") {
            values.push(auto_keyword());
            parser.index += 1;
            count += 1;
            continue;
        }

        if component_value_parse_as_length_percentage(component_value) {
            if let Some(filtered_input) = filtered_input {
                values.push(component_value_parse_as_nested_length_percentage(
                    component_value,
                    filtered_input,
                )?);
            }
            parser.index += 1;
            count += 1;
            all_auto = false;
            continue;
        }

        break;
    }

    if count == 0 {
        return None;
    }

    Some(ViewTimelineInsetPrefix {
        kind: if all_auto {
            CssViewFunctionInsetKind::Default
        } else {
            CssViewFunctionInsetKind::NonDefault
        },
        count,
        values,
    })
}

pub(crate) fn parse_anchor_name_or_scope_value<N>(
    filtered_input: &[u8],
    allow_all: bool,
    mut name_callback: N,
) -> CssAnchorNameOrScopeValueKind
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values.clone());
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-anchor-position-1/#name
    // Value: none | <dashed-ident>#
    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return CssAnchorNameOrScopeValueKind::Invalid;
        }
        return CssAnchorNameOrScopeValueKind::None;
    }

    // https://drafts.csswg.org/css-anchor-position-1/#anchor-scope
    // Value: none | all | <dashed-ident>#
    if allow_all && parser.consume_ident_matching("all") {
        if parser.has_next_component_value() {
            return CssAnchorNameOrScopeValueKind::Invalid;
        }
        return CssAnchorNameOrScopeValueKind::All;
    }

    let Some(names) = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);
        parser.parse_a_dashed_ident()
    }) else {
        return CssAnchorNameOrScopeValueKind::Invalid;
    };

    if names.is_empty() {
        return CssAnchorNameOrScopeValueKind::Invalid;
    }

    for name in names {
        name_callback(&name);
    }

    CssAnchorNameOrScopeValueKind::List
}

pub(crate) fn parse_position_anchor_value<N>(filtered_input: &[u8], mut name_callback: N) -> CssPositionAnchorValueKind
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-anchor-position-1/#position-anchor
    // Value: normal | none | auto | <anchor-name>
    if parser.consume_ident_matching("normal") {
        if parser.has_next_component_value() {
            return CssPositionAnchorValueKind::Invalid;
        }
        return CssPositionAnchorValueKind::Normal;
    }

    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return CssPositionAnchorValueKind::Invalid;
        }
        return CssPositionAnchorValueKind::None;
    }

    if parser.consume_ident_matching("auto") {
        if parser.has_next_component_value() {
            return CssPositionAnchorValueKind::Invalid;
        }
        return CssPositionAnchorValueKind::Auto;
    }

    let Some(name) = parser.parse_a_dashed_ident() else {
        return CssPositionAnchorValueKind::Invalid;
    };

    name_callback(&name);
    CssPositionAnchorValueKind::AnchorName
}

pub(crate) fn parse_position_area_value(filtered_input: &[u8]) -> bool {
    parse_rust_owned_position_area_value(filtered_input).is_some()
}

pub(super) fn parse_rust_owned_position_area_value(filtered_input: &[u8]) -> Option<RustOwnedPositionArea> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values.clone());
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-anchor-position-1/#position-area-property
    // Value: none | <position-area>
    if parser.consume_ident_matching("none") {
        return (!parser.has_next_component_value()).then_some(RustOwnedPositionArea::None);
    }

    parse_position_area_component_values(component_values)
}

pub(crate) fn parse_position_try_fallbacks_value(filtered_input: &[u8]) -> bool {
    parse_rust_owned_position_try_fallbacks_value(filtered_input).is_some()
}

pub(super) fn parse_rust_owned_position_try_fallbacks_value(
    filtered_input: &[u8],
) -> Option<RustOwnedPositionTryFallbacks> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values.clone());
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-anchor-position-1/#position-try-fallbacks
    // Value: none | [ [<dashed-ident> || <try-tactic>] | <position-area> ]#
    if parser.consume_ident_matching("none") {
        return (!parser.has_next_component_value()).then_some(RustOwnedPositionTryFallbacks::None);
    }

    let fallbacks = parse_comma_separated_component_values(component_values, |component_values| {
        parse_single_position_try_fallbacks_component_values(component_values)
    })?;

    (!fallbacks.is_empty()).then_some(RustOwnedPositionTryFallbacks::List(fallbacks))
}

pub(super) fn parse_single_position_try_fallbacks_component_values(
    component_values: Vec<ComponentValue>,
) -> Option<RustOwnedPositionTryFallback> {
    // [ [<dashed-ident> || <try-tactic>] | <position-area> ]
    if let Some(position_area) = parse_position_area_component_values(component_values.clone()) {
        return Some(RustOwnedPositionTryFallback::PositionArea(position_area));
    }

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    let mut dashed_ident = None;
    let mut has_flip_block = false;
    let mut has_flip_inline = false;
    let mut has_flip_start = false;
    let mut try_tactics = Vec::new();
    let mut saw_try_tactic = false;
    let mut dashed_ident_after_try_tactic = false;

    // https://drafts.csswg.org/css-anchor-position-1/#typedef-position-try-fallbacks-try-tactic
    // <try-tactic> = flip-block || flip-inline || flip-start
    while parser.has_next_component_value() {
        let ident = parser.consume_an_ident()?;

        if ident.starts_with("--") && is_valid_custom_ident(&ident, &[]) {
            if dashed_ident.is_some() {
                return None;
            }
            dashed_ident_after_try_tactic = saw_try_tactic;
            dashed_ident = Some(ident);
        } else if ident.eq_ignore_ascii_case("flip-block") {
            if dashed_ident_after_try_tactic || has_flip_block {
                return None;
            }
            saw_try_tactic = true;
            has_flip_block = true;
            try_tactics.push(ident);
        } else if ident.eq_ignore_ascii_case("flip-inline") {
            if dashed_ident_after_try_tactic || has_flip_inline {
                return None;
            }
            saw_try_tactic = true;
            has_flip_inline = true;
            try_tactics.push(ident);
        } else if ident.eq_ignore_ascii_case("flip-start") {
            if dashed_ident_after_try_tactic || has_flip_start {
                return None;
            }
            saw_try_tactic = true;
            has_flip_start = true;
            try_tactics.push(ident);
        } else {
            return None;
        }

        parser.discard_whitespace();
    }

    (dashed_ident.is_some() || has_flip_block || has_flip_inline || has_flip_start).then_some(
        RustOwnedPositionTryFallback::TryTactic {
            dashed_ident,
            has_flip_block,
            has_flip_inline,
            has_flip_start,
            try_tactics,
        },
    )
}

pub(super) fn parse_position_area_component_values(
    component_values: Vec<ComponentValue>,
) -> Option<RustOwnedPositionArea> {
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-anchor-position-1/#typedef-position-area
    // <position-area> = [
    //   [ left | center | right | span-left | span-right | x-start | x-end | span-x-start | span-x-end | x-self-start | x-self-end | span-x-self-start | span-x-self-end | span-all ]
    //   ||
    //   [ top | center | bottom | span-top | span-bottom | y-start | y-end | span-y-start | span-y-end | y-self-start | y-self-end | span-y-self-start | span-y-self-end | span-all ]
    // |
    //   [ block-start | center | block-end | span-block-start | span-block-end | span-all ]
    //   ||
    //   [ inline-start | center | inline-end | span-inline-start | span-inline-end | span-all ]
    // |
    //   [ self-block-start | center | self-block-end | span-self-block-start | span-self-block-end | span-all ]
    //   ||
    //   [ self-inline-start | center | self-inline-end | span-self-inline-start | span-self-inline-end | span-all ]
    // |
    //   [ start | center | end | span-start | span-end | span-all ]{1,2}
    // |
    //   [ self-start | center | self-end | span-self-start | span-self-end | span-all ]{1,2}
    // ]
    let first = parser.consume_an_ident()?;

    parser.discard_whitespace();
    let Some(second) = parser.consume_an_ident() else {
        return (!parser.has_next_component_value() && is_position_area_keyword(&first)).then_some(
            RustOwnedPositionArea::Area {
                first_keyword: first,
                second_keyword: None,
            },
        );
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    let normalized_keywords = if is_position_area_x_keyword(&first) && is_position_area_y_keyword(&second) {
        Some((first, second))
    } else if is_position_area_y_keyword(&first) && is_position_area_x_keyword(&second) {
        Some((second, first))
    } else if is_position_area_block_keyword(&first) && is_position_area_inline_keyword(&second) {
        Some((first, second))
    } else if is_position_area_inline_keyword(&first) && is_position_area_block_keyword(&second) {
        Some((second, first))
    } else if is_position_area_self_block_keyword(&first) && is_position_area_self_inline_keyword(&second) {
        Some((first, second))
    } else if is_position_area_self_inline_keyword(&first) && is_position_area_self_block_keyword(&second) {
        Some((second, first))
    } else if is_position_area_start_end_keyword(&first) && is_position_area_start_end_keyword(&second)
        || is_position_area_self_start_end_keyword(&first) && is_position_area_self_start_end_keyword(&second)
    {
        Some((first, second))
    } else {
        None
    }?;

    let (first, second) = normalized_keywords;
    if !is_position_area_axis_ambiguous(&first) && second.eq_ignore_ascii_case("span-all") {
        return Some(RustOwnedPositionArea::Area {
            first_keyword: first,
            second_keyword: None,
        });
    }
    if !is_position_area_axis_ambiguous(&second) && first.eq_ignore_ascii_case("span-all") {
        return Some(RustOwnedPositionArea::Area {
            first_keyword: second,
            second_keyword: None,
        });
    }

    Some(RustOwnedPositionArea::Area {
        first_keyword: first,
        second_keyword: Some(second),
    })
}

pub(super) fn is_position_area_keyword(value: &str) -> bool {
    is_position_area_x_keyword(value)
        || is_position_area_y_keyword(value)
        || is_position_area_block_keyword(value)
        || is_position_area_inline_keyword(value)
        || is_position_area_self_block_keyword(value)
        || is_position_area_self_inline_keyword(value)
        || is_position_area_start_end_keyword(value)
        || is_position_area_self_start_end_keyword(value)
}

pub(super) fn is_position_area_x_keyword(value: &str) -> bool {
    matches_any_ignore_ascii_case(
        value,
        &[
            "left",
            "center",
            "right",
            "span-left",
            "span-right",
            "x-start",
            "x-end",
            "span-x-start",
            "span-x-end",
            "self-x-start",
            "self-x-end",
            "span-self-x-start",
            "span-self-x-end",
            "span-all",
        ],
    )
}

pub(super) fn is_position_area_y_keyword(value: &str) -> bool {
    matches_any_ignore_ascii_case(
        value,
        &[
            "top",
            "center",
            "bottom",
            "span-top",
            "span-bottom",
            "y-start",
            "y-end",
            "span-y-start",
            "span-y-end",
            "self-y-start",
            "self-y-end",
            "span-self-y-start",
            "span-self-y-end",
            "span-all",
        ],
    )
}

pub(super) fn is_position_area_block_keyword(value: &str) -> bool {
    matches_any_ignore_ascii_case(
        value,
        &[
            "block-start",
            "center",
            "block-end",
            "span-block-start",
            "span-block-end",
            "span-all",
        ],
    )
}

pub(super) fn is_position_area_inline_keyword(value: &str) -> bool {
    matches_any_ignore_ascii_case(
        value,
        &[
            "inline-start",
            "center",
            "inline-end",
            "span-inline-start",
            "span-inline-end",
            "span-all",
        ],
    )
}

pub(super) fn is_position_area_self_block_keyword(value: &str) -> bool {
    matches_any_ignore_ascii_case(
        value,
        &[
            "self-block-start",
            "center",
            "self-block-end",
            "span-self-block-start",
            "span-self-block-end",
            "span-all",
        ],
    )
}

pub(super) fn is_position_area_self_inline_keyword(value: &str) -> bool {
    matches_any_ignore_ascii_case(
        value,
        &[
            "self-inline-start",
            "center",
            "self-inline-end",
            "span-self-inline-start",
            "span-self-inline-end",
            "span-all",
        ],
    )
}

pub(super) fn is_position_area_start_end_keyword(value: &str) -> bool {
    matches_any_ignore_ascii_case(value, &["start", "center", "end", "span-start", "span-end", "span-all"])
}

pub(super) fn is_position_area_self_start_end_keyword(value: &str) -> bool {
    matches_any_ignore_ascii_case(
        value,
        &[
            "self-start",
            "center",
            "self-end",
            "span-self-start",
            "span-self-end",
            "span-all",
        ],
    )
}

pub(super) fn is_position_area_axis_ambiguous(value: &str) -> bool {
    matches_any_ignore_ascii_case(
        value,
        &[
            "center",
            "span-all",
            "start",
            "end",
            "self-start",
            "self-end",
            "span-start",
            "span-end",
            "span-self-start",
            "span-self-end",
        ],
    )
}

pub(super) fn matches_any_ignore_ascii_case(value: &str, candidates: &[&str]) -> bool {
    candidates.iter().any(|candidate| value.eq_ignore_ascii_case(candidate))
}

pub(crate) fn parse_timeline_scope_value<N>(filtered_input: &[u8], mut name_callback: N) -> CssTimelineScopeValueKind
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values.clone());
    parser.discard_whitespace();

    // https://drafts.csswg.org/scroll-animations-1/#propdef-timeline-scope
    // Value: none | all | <dashed-ident>#
    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return CssTimelineScopeValueKind::Invalid;
        }
        return CssTimelineScopeValueKind::None;
    }

    if parser.consume_ident_matching("all") {
        if parser.has_next_component_value() {
            return CssTimelineScopeValueKind::Invalid;
        }
        return CssTimelineScopeValueKind::All;
    }

    let Some(names) = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);
        parser.parse_a_dashed_ident()
    }) else {
        return CssTimelineScopeValueKind::Invalid;
    };

    if names.is_empty() {
        return CssTimelineScopeValueKind::Invalid;
    }

    for name in names {
        name_callback(&name);
    }

    CssTimelineScopeValueKind::List
}

pub(crate) fn parse_timeline_name_value<N>(filtered_input: &[u8], mut name_callback: N) -> CssTimelineNameValueKind
where
    N: FnMut(CssTimelineNameItemKind, &str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/scroll-animations-1/#scroll-timeline-name
    // Value: [ none | <dashed-ident> ]#
    //
    // https://drafts.csswg.org/scroll-animations-1/#view-timeline-name
    // Value: [ none | <dashed-ident> ]#
    let Some(names) = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);
        parser.discard_whitespace();

        if parser.consume_ident_matching("none") {
            if parser.has_next_component_value() {
                return None;
            }
            return Some((CssTimelineNameItemKind::None, String::new()));
        }

        let name = parser.parse_a_dashed_ident()?;
        Some((CssTimelineNameItemKind::DashedIdent, name))
    }) else {
        return CssTimelineNameValueKind::Invalid;
    };

    if names.is_empty() {
        return CssTimelineNameValueKind::Invalid;
    }

    for (kind, name) in names {
        name_callback(kind, &name);
    }

    CssTimelineNameValueKind::List
}

pub(crate) fn parse_position_try_order_value(filtered_input: &[u8]) -> CssPositionTryOrderValue {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-anchor-position-1/#position-try-order-property
    // Value: normal | <try-size>
    //
    // https://drafts.csswg.org/css-anchor-position-1/#typedef-try-size
    // <try-size> = most-width | most-height | most-block-size | most-inline-size
    let Some(ident) = parser.consume_an_ident() else {
        return CssPositionTryOrderValue::Invalid;
    };

    let value = if ident.eq_ignore_ascii_case("normal") {
        CssPositionTryOrderValue::Normal
    } else if ident.eq_ignore_ascii_case("most-width") {
        CssPositionTryOrderValue::MostWidth
    } else if ident.eq_ignore_ascii_case("most-height") {
        CssPositionTryOrderValue::MostHeight
    } else if ident.eq_ignore_ascii_case("most-block-size") {
        CssPositionTryOrderValue::MostBlockSize
    } else if ident.eq_ignore_ascii_case("most-inline-size") {
        CssPositionTryOrderValue::MostInlineSize
    } else {
        return CssPositionTryOrderValue::Invalid;
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return CssPositionTryOrderValue::Invalid;
    }

    value
}

pub(crate) fn parse_position_visibility_value(filtered_input: &[u8]) -> CssPositionVisibilityValue {
    let invalid = CssPositionVisibilityValue {
        kind: CssPositionVisibilityValueKind::Invalid,
        has_anchors_valid: false,
        has_anchors_visible: false,
        has_no_overflow: false,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-anchor-position-1/#position-visibility
    // Value: always | [ anchors-valid || anchors-visible || no-overflow ]
    if parser.consume_ident_matching("always") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssPositionVisibilityValue {
            kind: CssPositionVisibilityValueKind::Always,
            has_anchors_valid: false,
            has_anchors_visible: false,
            has_no_overflow: false,
        };
    }

    let mut value = CssPositionVisibilityValue {
        kind: CssPositionVisibilityValueKind::List,
        has_anchors_valid: false,
        has_anchors_visible: false,
        has_no_overflow: false,
    };

    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        if ident.eq_ignore_ascii_case("anchors-valid") {
            if value.has_anchors_valid {
                return invalid;
            }
            value.has_anchors_valid = true;
        } else if ident.eq_ignore_ascii_case("anchors-visible") {
            if value.has_anchors_visible {
                return invalid;
            }
            value.has_anchors_visible = true;
        } else if ident.eq_ignore_ascii_case("no-overflow") {
            if value.has_no_overflow {
                return invalid;
            }
            value.has_no_overflow = true;
        } else {
            return invalid;
        }
    }

    if !value.has_anchors_valid && !value.has_anchors_visible && !value.has_no_overflow {
        return invalid;
    }

    value
}

pub(crate) fn parse_paint_order_value(filtered_input: &[u8]) -> CssPaintOrderValue {
    let invalid = CssPaintOrderValue {
        kind: CssPaintOrderValueKind::Invalid,
        first: CssPaintOrderKeyword::Invalid,
        second: CssPaintOrderKeyword::Invalid,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://svgwg.org/svg2-draft/painting.html#PaintOrder
    // Value: normal | [ fill || stroke || markers ]
    if parser.consume_ident_matching("normal") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssPaintOrderValue {
            kind: CssPaintOrderValueKind::Normal,
            first: CssPaintOrderKeyword::Invalid,
            second: CssPaintOrderKeyword::Invalid,
        };
    }

    let mut keywords = Vec::new();
    let mut has_fill = false;
    let mut has_stroke = false;
    let mut has_markers = false;
    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        let keyword = if ident.eq_ignore_ascii_case("fill") {
            if has_fill {
                return invalid;
            }
            has_fill = true;
            CssPaintOrderKeyword::Fill
        } else if ident.eq_ignore_ascii_case("stroke") {
            if has_stroke {
                return invalid;
            }
            has_stroke = true;
            CssPaintOrderKeyword::Stroke
        } else if ident.eq_ignore_ascii_case("markers") {
            if has_markers {
                return invalid;
            }
            has_markers = true;
            CssPaintOrderKeyword::Markers
        } else {
            return invalid;
        };

        keywords.push(keyword);
    }

    let Some(first) = keywords.first().copied() else {
        return invalid;
    };

    if keywords.len() == 1 {
        return CssPaintOrderValue {
            kind: CssPaintOrderValueKind::Keyword,
            first,
            second: CssPaintOrderKeyword::Invalid,
        };
    }

    if keywords.len() > 3 {
        return invalid;
    }

    let second = keywords[1];
    let expected_second_keyword = if first == CssPaintOrderKeyword::Fill {
        CssPaintOrderKeyword::Stroke
    } else {
        CssPaintOrderKeyword::Fill
    };

    if second == expected_second_keyword {
        return CssPaintOrderValue {
            kind: CssPaintOrderValueKind::Keyword,
            first,
            second: CssPaintOrderKeyword::Invalid,
        };
    }

    CssPaintOrderValue {
        kind: CssPaintOrderValueKind::Pair,
        first,
        second,
    }
}

pub(crate) fn parse_place_content_value(filtered_input: &[u8]) -> bool {
    parse_place_shorthand_value(
        filtered_input,
        component_values_parse_as_align_content,
        component_values_parse_as_justify_content,
    )
}

pub(crate) fn parse_place_items_value(filtered_input: &[u8]) -> bool {
    parse_place_shorthand_value(
        filtered_input,
        component_values_parse_as_align_items,
        component_values_parse_as_justify_items,
    )
}

pub(crate) fn parse_place_self_value(filtered_input: &[u8]) -> bool {
    parse_place_shorthand_value(
        filtered_input,
        component_values_parse_as_align_self,
        component_values_parse_as_justify_self,
    )
}

pub(super) fn parse_place_shorthand_value(
    filtered_input: &[u8],
    parse_align_value: fn(&[ComponentValue]) -> bool,
    parse_justify_value: fn(&[ComponentValue]) -> bool,
) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    if component_values.is_empty() {
        return false;
    }

    if parse_align_value(&component_values) && parse_justify_value(&component_values) {
        return true;
    }

    for split_index in 1..component_values.len() {
        if parse_align_value(&component_values[..split_index]) && parse_justify_value(&component_values[split_index..])
        {
            return true;
        }
    }

    false
}

pub(super) fn component_values_parse_as_align_content(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-align-3/#propdef-align-content
    // Value: normal | <baseline-position> | <content-distribution> | <overflow-position>? <content-position>
    component_values_parse_as_single_ident(component_values, &["normal"])
        || component_values_parse_as_baseline_position(component_values)
        || component_values_parse_as_content_distribution(component_values)
        || component_values_parse_as_content_position_with_optional_overflow(component_values, false)
}

pub(super) fn component_values_parse_as_justify_content(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-align-3/#propdef-justify-content
    // Value: normal | <content-distribution> | <overflow-position>? [ <content-position> | left | right ]
    component_values_parse_as_single_ident(component_values, &["normal"])
        || component_values_parse_as_content_distribution(component_values)
        || component_values_parse_as_content_position_with_optional_overflow(component_values, true)
}

pub(super) fn component_values_parse_as_align_items(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-align-3/#propdef-align-items
    // Value: normal | stretch | <baseline-position> | [ <overflow-position>? <self-position> ]
    component_values_parse_as_single_ident(component_values, &["normal", "stretch"])
        || component_values_parse_as_baseline_position(component_values)
        || component_values_parse_as_self_position_with_optional_overflow(component_values, false, false)
}

pub(super) fn component_values_parse_as_justify_items(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-align-3/#propdef-justify-items
    // Value: normal | stretch | <baseline-position> | <overflow-position>? [ <self-position> | left | right ] | legacy | legacy && [ left | right | center ]
    component_values_parse_as_single_ident(component_values, &["normal", "stretch", "legacy"])
        || component_values_parse_as_baseline_position(component_values)
        || component_values_parse_as_self_position_with_optional_overflow(component_values, true, false)
        || component_values_parse_as_legacy_justify_items(component_values)
}

pub(super) fn component_values_parse_as_align_self(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-align-3/#propdef-align-self
    // Value: auto | normal | stretch | <baseline-position> | <overflow-position>? <self-position>
    component_values_parse_as_single_ident(component_values, &["auto", "normal", "stretch"])
        || component_values_parse_as_baseline_position(component_values)
        || component_values_parse_as_self_position_with_optional_overflow(component_values, false, false)
}

pub(super) fn component_values_parse_as_justify_self(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-align-3/#propdef-justify-self
    // Value: auto | normal | stretch | <baseline-position> | <overflow-position>? [ <self-position> | left | right ]
    component_values_parse_as_single_ident(component_values, &["auto", "normal", "stretch"])
        || component_values_parse_as_baseline_position(component_values)
        || component_values_parse_as_self_position_with_optional_overflow(component_values, true, false)
}

pub(super) fn component_values_parse_as_single_ident(component_values: &[ComponentValue], keywords: &[&str]) -> bool {
    let [component_value] = component_values else {
        return false;
    };
    keywords
        .iter()
        .any(|keyword| component_value_is_ident(Some(component_value), keyword))
}

pub(super) fn component_values_parse_as_baseline_position(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-align-3/#typedef-baseline-position
    // <baseline-position> = [ first | last ]? && baseline
    component_values_parse_as_single_ident(component_values, &["baseline"])
        || component_values_match_idents(component_values, &["first", "baseline"])
        || component_values_match_idents(component_values, &["last", "baseline"])
}

pub(super) fn component_values_parse_as_content_distribution(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-align-3/#typedef-content-distribution
    // <content-distribution> = space-between | space-around | space-evenly | stretch
    component_values_parse_as_single_ident(
        component_values,
        &["space-between", "space-around", "space-evenly", "stretch"],
    )
}

pub(super) fn component_values_parse_as_content_position_with_optional_overflow(
    component_values: &[ComponentValue],
    allow_left_right: bool,
) -> bool {
    // https://drafts.csswg.org/css-align-3/#typedef-overflow-position
    // <overflow-position> = unsafe | safe
    //
    // https://drafts.csswg.org/css-align-3/#typedef-content-position
    // <content-position> = center | start | end | flex-start | flex-end
    component_values_parse_as_single_content_position(component_values, allow_left_right)
        || component_values_parse_as_overflow_position_and(component_values, |component_value| {
            component_value_parse_as_content_position(component_value, allow_left_right)
        })
}

pub(super) fn component_values_parse_as_self_position_with_optional_overflow(
    component_values: &[ComponentValue],
    allow_left_right: bool,
    allow_auto: bool,
) -> bool {
    // https://drafts.csswg.org/css-align-3/#typedef-self-position
    // <self-position> = center | start | end | self-start | self-end | flex-start | flex-end
    component_values_parse_as_single_self_position(component_values, allow_left_right, allow_auto)
        || component_values_parse_as_overflow_position_and(component_values, |component_value| {
            component_value_parse_as_self_position(component_value, allow_left_right, allow_auto)
        })
}

pub(super) fn component_values_parse_as_single_content_position(
    component_values: &[ComponentValue],
    allow_left_right: bool,
) -> bool {
    let [component_value] = component_values else {
        return false;
    };
    component_value_parse_as_content_position(component_value, allow_left_right)
}

pub(super) fn component_values_parse_as_single_self_position(
    component_values: &[ComponentValue],
    allow_left_right: bool,
    allow_auto: bool,
) -> bool {
    let [component_value] = component_values else {
        return false;
    };
    component_value_parse_as_self_position(component_value, allow_left_right, allow_auto)
}

pub(super) fn component_values_parse_as_overflow_position_and(
    component_values: &[ComponentValue],
    parse_position: impl FnOnce(&ComponentValue) -> bool,
) -> bool {
    let [overflow_position, position] = component_values else {
        return false;
    };
    (component_value_is_ident(Some(overflow_position), "safe")
        || component_value_is_ident(Some(overflow_position), "unsafe"))
        && parse_position(position)
}

pub(super) fn component_value_parse_as_content_position(
    component_value: &ComponentValue,
    allow_left_right: bool,
) -> bool {
    ["center", "start", "end", "flex-start", "flex-end"]
        .iter()
        .any(|keyword| component_value_is_ident(Some(component_value), keyword))
        || (allow_left_right
            && ["left", "right"]
                .iter()
                .any(|keyword| component_value_is_ident(Some(component_value), keyword)))
}

pub(super) fn component_value_parse_as_self_position(
    component_value: &ComponentValue,
    allow_left_right: bool,
    allow_auto: bool,
) -> bool {
    [
        "center",
        "start",
        "end",
        "self-start",
        "self-end",
        "flex-start",
        "flex-end",
    ]
    .iter()
    .any(|keyword| component_value_is_ident(Some(component_value), keyword))
        || (allow_auto && component_value_is_ident(Some(component_value), "auto"))
        || (allow_left_right
            && ["left", "right"]
                .iter()
                .any(|keyword| component_value_is_ident(Some(component_value), keyword)))
}

pub(super) fn component_values_parse_as_legacy_justify_items(component_values: &[ComponentValue]) -> bool {
    component_values_match_idents(component_values, &["legacy", "left"])
        || component_values_match_idents(component_values, &["legacy", "right"])
        || component_values_match_idents(component_values, &["legacy", "center"])
        || component_values_match_idents(component_values, &["left", "legacy"])
        || component_values_match_idents(component_values, &["right", "legacy"])
        || component_values_match_idents(component_values, &["center", "legacy"])
}

pub(super) fn component_values_match_idents(component_values: &[ComponentValue], keywords: &[&str]) -> bool {
    component_values.len() == keywords.len()
        && component_values
            .iter()
            .zip(keywords)
            .all(|(component_value, keyword)| component_value_is_ident(Some(component_value), keyword))
}

pub(crate) fn parse_text_underline_position_value(filtered_input: &[u8]) -> CssTextUnderlinePositionValue {
    let invalid = CssTextUnderlinePositionValue {
        horizontal: CssTextUnderlinePositionHorizontal::Invalid,
        vertical: CssTextUnderlinePositionVertical::Invalid,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-text-decor-4/#text-underline-position-property
    // Value: auto | [ from-font | under ] || [ left | right ]
    if parser.consume_ident_matching("auto") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssTextUnderlinePositionValue {
            horizontal: CssTextUnderlinePositionHorizontal::Auto,
            vertical: CssTextUnderlinePositionVertical::Auto,
        };
    }

    let mut horizontal = CssTextUnderlinePositionHorizontal::Auto;
    let mut vertical = CssTextUnderlinePositionVertical::Auto;
    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        if ident.eq_ignore_ascii_case("from-font") {
            if horizontal != CssTextUnderlinePositionHorizontal::Auto {
                return invalid;
            }
            horizontal = CssTextUnderlinePositionHorizontal::FromFont;
        } else if ident.eq_ignore_ascii_case("under") {
            if horizontal != CssTextUnderlinePositionHorizontal::Auto {
                return invalid;
            }
            horizontal = CssTextUnderlinePositionHorizontal::Under;
        } else if ident.eq_ignore_ascii_case("left") {
            if vertical != CssTextUnderlinePositionVertical::Auto {
                return invalid;
            }
            vertical = CssTextUnderlinePositionVertical::Left;
        } else if ident.eq_ignore_ascii_case("right") {
            if vertical != CssTextUnderlinePositionVertical::Auto {
                return invalid;
            }
            vertical = CssTextUnderlinePositionVertical::Right;
        } else {
            return invalid;
        }
    }

    if horizontal == CssTextUnderlinePositionHorizontal::Auto && vertical == CssTextUnderlinePositionVertical::Auto {
        return invalid;
    }

    CssTextUnderlinePositionValue { horizontal, vertical }
}

pub(crate) fn parse_text_wrap_mode_value(filtered_input: &[u8]) -> CssTextWrapModeValue {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-text-4/#text-wrap-mode
    // Value: wrap | nowrap
    let Some(ident) = parser.consume_an_ident() else {
        return CssTextWrapModeValue::Invalid;
    };

    let value = if ident.eq_ignore_ascii_case("wrap") {
        CssTextWrapModeValue::Wrap
    } else if ident.eq_ignore_ascii_case("nowrap") {
        CssTextWrapModeValue::Nowrap
    } else {
        return CssTextWrapModeValue::Invalid;
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return CssTextWrapModeValue::Invalid;
    }

    value
}

pub(crate) fn parse_text_wrap_style_value(filtered_input: &[u8]) -> CssTextWrapStyleValue {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-text-4/#text-wrap-style
    // Value: auto | balance | stable | pretty | avoid-orphans
    //
    // AD-HOC: The generated C++ parser only accepts the keywords from
    // Enums.json, which does not include avoid-orphans yet.
    let Some(ident) = parser.consume_an_ident() else {
        return CssTextWrapStyleValue::Invalid;
    };

    let value = if ident.eq_ignore_ascii_case("auto") {
        CssTextWrapStyleValue::Auto
    } else if ident.eq_ignore_ascii_case("balance") {
        CssTextWrapStyleValue::Balance
    } else if ident.eq_ignore_ascii_case("stable") {
        CssTextWrapStyleValue::Stable
    } else if ident.eq_ignore_ascii_case("pretty") {
        CssTextWrapStyleValue::Pretty
    } else {
        return CssTextWrapStyleValue::Invalid;
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return CssTextWrapStyleValue::Invalid;
    }

    value
}

pub(crate) fn parse_text_wrap_value(filtered_input: &[u8]) -> CssTextWrapValue {
    let invalid = CssTextWrapValue {
        kind: CssTextWrapValueKind::Invalid,
        mode: CssTextWrapModeValue::Invalid,
        style: CssTextWrapStyleValue::Invalid,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    let mut mode = CssTextWrapModeValue::Invalid;
    let mut style = CssTextWrapStyleValue::Invalid;

    // https://drafts.csswg.org/css-text-4/#text-wrap
    // Value: <'text-wrap-mode'> || <'text-wrap-style'>
    //
    // AD-HOC: text-wrap-style does not accept avoid-orphans yet.
    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        if ident.eq_ignore_ascii_case("wrap") {
            if mode != CssTextWrapModeValue::Invalid {
                return invalid;
            }
            mode = CssTextWrapModeValue::Wrap;
        } else if ident.eq_ignore_ascii_case("nowrap") {
            if mode != CssTextWrapModeValue::Invalid {
                return invalid;
            }
            mode = CssTextWrapModeValue::Nowrap;
        } else if ident.eq_ignore_ascii_case("auto") {
            if style != CssTextWrapStyleValue::Invalid {
                return invalid;
            }
            style = CssTextWrapStyleValue::Auto;
        } else if ident.eq_ignore_ascii_case("balance") {
            if style != CssTextWrapStyleValue::Invalid {
                return invalid;
            }
            style = CssTextWrapStyleValue::Balance;
        } else if ident.eq_ignore_ascii_case("stable") {
            if style != CssTextWrapStyleValue::Invalid {
                return invalid;
            }
            style = CssTextWrapStyleValue::Stable;
        } else if ident.eq_ignore_ascii_case("pretty") {
            if style != CssTextWrapStyleValue::Invalid {
                return invalid;
            }
            style = CssTextWrapStyleValue::Pretty;
        } else {
            return invalid;
        }

        parser.discard_whitespace();
    }

    if mode == CssTextWrapModeValue::Invalid && style == CssTextWrapStyleValue::Invalid {
        return invalid;
    }

    CssTextWrapValue {
        kind: CssTextWrapValueKind::Valid,
        mode,
        style,
    }
}

pub(crate) fn parse_touch_action_value(filtered_input: &[u8]) -> CssTouchActionValue {
    let invalid = CssTouchActionValue {
        kind: CssTouchActionValueKind::Invalid,
        first: CssTouchActionKeyword::Invalid,
        second: CssTouchActionKeyword::Invalid,
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://www.w3.org/TR/pointerevents/#the-touch-action-css-property
    // Value: auto | none | [ [ pan-x | pan-left | pan-right ] || [ pan-y | pan-up | pan-down ] ] | manipulation
    if parser.consume_ident_matching("auto") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssTouchActionValue {
            kind: CssTouchActionValueKind::Auto,
            first: CssTouchActionKeyword::Invalid,
            second: CssTouchActionKeyword::Invalid,
        };
    }
    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssTouchActionValue {
            kind: CssTouchActionValueKind::None,
            first: CssTouchActionKeyword::Invalid,
            second: CssTouchActionKeyword::Invalid,
        };
    }
    if parser.consume_ident_matching("manipulation") {
        if parser.has_next_component_value() {
            return invalid;
        }
        return CssTouchActionValue {
            kind: CssTouchActionValueKind::Manipulation,
            first: CssTouchActionKeyword::Invalid,
            second: CssTouchActionKeyword::Invalid,
        };
    }

    let mut horizontal = CssTouchActionKeyword::Invalid;
    let mut vertical = CssTouchActionKeyword::Invalid;
    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return invalid;
        };

        if ident.eq_ignore_ascii_case("pan-x") {
            if horizontal != CssTouchActionKeyword::Invalid {
                return invalid;
            }
            horizontal = CssTouchActionKeyword::PanX;
        } else if ident.eq_ignore_ascii_case("pan-left") {
            if horizontal != CssTouchActionKeyword::Invalid {
                return invalid;
            }
            horizontal = CssTouchActionKeyword::PanLeft;
        } else if ident.eq_ignore_ascii_case("pan-right") {
            if horizontal != CssTouchActionKeyword::Invalid {
                return invalid;
            }
            horizontal = CssTouchActionKeyword::PanRight;
        } else if ident.eq_ignore_ascii_case("pan-y") {
            if vertical != CssTouchActionKeyword::Invalid {
                return invalid;
            }
            vertical = CssTouchActionKeyword::PanY;
        } else if ident.eq_ignore_ascii_case("pan-up") {
            if vertical != CssTouchActionKeyword::Invalid {
                return invalid;
            }
            vertical = CssTouchActionKeyword::PanUp;
        } else if ident.eq_ignore_ascii_case("pan-down") {
            if vertical != CssTouchActionKeyword::Invalid {
                return invalid;
            }
            vertical = CssTouchActionKeyword::PanDown;
        } else {
            return invalid;
        }
    }

    if horizontal == CssTouchActionKeyword::Invalid && vertical == CssTouchActionKeyword::Invalid {
        return invalid;
    }

    if horizontal == CssTouchActionKeyword::Invalid {
        return CssTouchActionValue {
            kind: CssTouchActionValueKind::List,
            first: vertical,
            second: CssTouchActionKeyword::Invalid,
        };
    }

    CssTouchActionValue {
        kind: CssTouchActionValueKind::List,
        first: horizontal,
        second: vertical,
    }
}

pub(crate) fn parse_scrollbar_gutter_value(filtered_input: &[u8]) -> CssScrollbarGutterValueKind {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-overflow-4/#propdef-scrollbar-gutter
    // Value: auto | stable && both-edges?
    if parser.consume_ident_matching("auto") {
        if parser.has_next_component_value() {
            return CssScrollbarGutterValueKind::Invalid;
        }
        return CssScrollbarGutterValueKind::Auto;
    }

    let mut stable = false;
    let mut both_edges = false;
    while parser.has_next_component_value() {
        let Some(ident) = parser.consume_an_ident() else {
            return CssScrollbarGutterValueKind::Invalid;
        };

        if ident.eq_ignore_ascii_case("stable") {
            if stable {
                return CssScrollbarGutterValueKind::Invalid;
            }
            stable = true;
        } else if ident.eq_ignore_ascii_case("both-edges") {
            if both_edges {
                return CssScrollbarGutterValueKind::Invalid;
            }
            both_edges = true;
        } else {
            return CssScrollbarGutterValueKind::Invalid;
        }
    }

    if !stable {
        return CssScrollbarGutterValueKind::Invalid;
    }

    if both_edges {
        CssScrollbarGutterValueKind::BothEdges
    } else {
        CssScrollbarGutterValueKind::Stable
    }
}

pub(crate) fn parse_stroke_dasharray_value(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    // https://svgwg.org/svg2-draft/painting.html#StrokeDashing
    // Value: none | <dasharray>
    parser.discard_whitespace();
    if parser.consume_ident_matching("none") {
        parser.discard_whitespace();
        return !parser.has_next_component_value();
    }

    // https://svgwg.org/svg2-draft/painting.html#DataTypeDasharray
    // <dasharray> = [ [ <length-percentage> | <number> ]+ ]#
    let mut saw_value = false;
    loop {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };

        // A <dasharray> is a list of comma and/or white space separated <number> or <length-percentage> values. A <number> value represents a value in user units.
        // If any value in the list is negative, the <dasharray> value is invalid.
        if !component_value_parse_as_non_negative_number(component_value)
            && !component_value_parse_as_non_negative_length_percentage(component_value)
        {
            return false;
        }

        parser.index += 1;
        saw_value = true;

        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            break;
        }
        if parser.consume_a_comma() {
            parser.discard_whitespace();
            if !parser.has_next_component_value() {
                return false;
            }
        }
    }

    saw_value
}

pub(crate) fn parse_quotes_value<S>(filtered_input: &[u8], mut string_callback: S) -> CssQuotesValueKind
where
    S: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-content-3/#propdef-quotes
    // Value: auto | none | [ <string> <string> ]+
    if parser.consume_ident_matching("auto") {
        if parser.has_next_component_value() {
            return CssQuotesValueKind::Invalid;
        }
        return CssQuotesValueKind::Auto;
    }

    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return CssQuotesValueKind::Invalid;
        }
        return CssQuotesValueKind::None;
    }

    let mut strings = Vec::new();
    while parser.has_next_component_value() {
        parser.discard_whitespace();
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { value },
            ..
        })) = parser.next_component_value()
        else {
            return CssQuotesValueKind::Invalid;
        };
        strings.push(value.clone());
        parser.index += 1;
    }

    if strings.is_empty() || strings.len() % 2 != 0 {
        return CssQuotesValueKind::Invalid;
    }

    for string in strings {
        string_callback(&string);
    }

    CssQuotesValueKind::List
}

pub(crate) fn parse_will_change_value<F>(filtered_input: &[u8], mut feature_callback: F) -> CssWillChangeValueKind
where
    F: FnMut(CssWillChangeFeatureKind, &str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values.clone());
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-will-change/#will-change
    // Value: auto | <animateable-feature>#
    if parser.consume_ident_matching("auto") {
        if parser.has_next_component_value() {
            return CssWillChangeValueKind::Invalid;
        }
        return CssWillChangeValueKind::Auto;
    }

    let Some(features) = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);
        parser.discard_whitespace();

        // https://drafts.csswg.org/css-will-change/#typedef-animateable-feature
        // <animateable-feature> = scroll-position | contents | <custom-ident>
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = parser.next_component_value()
        else {
            return None;
        };

        let feature = if value.eq_ignore_ascii_case("scroll-position") {
            (CssWillChangeFeatureKind::ScrollPosition, String::new())
        } else if value.eq_ignore_ascii_case("contents") {
            (CssWillChangeFeatureKind::Contents, String::new())
        } else if is_valid_custom_ident(
            value,
            &["all", "auto", "contents", "none", "scroll-position", "will-change"],
        ) {
            (CssWillChangeFeatureKind::CustomIdent, value.clone())
        } else {
            return None;
        };

        parser.index += 1;
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }

        Some(feature)
    }) else {
        return CssWillChangeValueKind::Invalid;
    };

    if features.is_empty() {
        return CssWillChangeValueKind::Invalid;
    }

    for (kind, value) in features {
        feature_callback(kind, &value);
    }

    CssWillChangeValueKind::List
}

pub(crate) fn parse_transition_property_value<N>(
    filtered_input: &[u8],
    mut property_callback: N,
) -> CssTransitionPropertyValueKind
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values.clone());
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-transitions/#transition-property-property
    // Value: none | <single-transition-property>#
    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return CssTransitionPropertyValueKind::Invalid;
        }
        return CssTransitionPropertyValueKind::None;
    }

    let Some(properties) = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);

        // https://drafts.csswg.org/css-transitions/#single-transition-property
        // <single-transition-property> = all | <custom-ident>
        parser.parse_a_custom_ident(&["none"])
    }) else {
        return CssTransitionPropertyValueKind::Invalid;
    };

    if properties.is_empty() {
        return CssTransitionPropertyValueKind::Invalid;
    }

    for property in properties {
        property_callback(&property);
    }

    CssTransitionPropertyValueKind::List
}

pub(crate) fn parse_transition_behavior_value<N>(
    filtered_input: &[u8],
    mut behavior_callback: N,
) -> CssTransitionBehaviorValueKind
where
    N: FnMut(CssTransitionBehaviorItemKind),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/css-transitions-2/#transition-behavior-property
    // Value: <transition-behavior-value>#
    //
    // https://drafts.csswg.org/css-transitions-2/#typedef-transition-behavior-value
    // <transition-behavior-value> = normal | allow-discrete
    let Some(behaviors) = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);
        parser.discard_whitespace();

        let behavior = if parser.consume_ident_matching("normal") {
            CssTransitionBehaviorItemKind::Normal
        } else if parser.consume_ident_matching("allow-discrete") {
            CssTransitionBehaviorItemKind::AllowDiscrete
        } else {
            return None;
        };

        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }

        Some(behavior)
    }) else {
        return CssTransitionBehaviorValueKind::Invalid;
    };

    if behaviors.is_empty() {
        return CssTransitionBehaviorValueKind::Invalid;
    }

    for behavior in behaviors {
        behavior_callback(behavior);
    }

    CssTransitionBehaviorValueKind::List
}

pub(crate) fn parse_animation_name_value<N>(filtered_input: &[u8], mut name_callback: N) -> CssAnimationNameValueKind
where
    N: FnMut(CssAnimationNameItemKind, &str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let Some(names) = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);
        parser.discard_whitespace();

        // https://drafts.csswg.org/css-animations-1/#propdef-animation-name
        // Value: [ none | <keyframes-name> ]#
        if parser.consume_ident_matching("none") {
            if parser.has_next_component_value() {
                return None;
            }
            return Some((CssAnimationNameItemKind::None, String::new()));
        }

        let Some(ComponentValue::PreservedToken(Token {
            token_type: token_type @ (TokenType::Ident { .. } | TokenType::String { .. }),
            ..
        })) = parser.next_component_value()
        else {
            return None;
        };

        let name = match token_type {
            TokenType::Ident { value } => {
                // https://drafts.csswg.org/css-animations-1/#typedef-keyframes-name
                // <keyframes-name> = <custom-ident> | <string>
                if !is_valid_custom_ident(value, &["none"]) {
                    return None;
                }
                (CssAnimationNameItemKind::CustomIdent, value.clone())
            }
            TokenType::String { value } => {
                if value.is_empty() {
                    return None;
                }
                (CssAnimationNameItemKind::String, value.clone())
            }
            _ => unreachable!(),
        };

        parser.index += 1;
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }

        Some(name)
    }) else {
        return CssAnimationNameValueKind::Invalid;
    };

    if names.is_empty() {
        return CssAnimationNameValueKind::Invalid;
    }

    for (kind, name) in names {
        name_callback(kind, &name);
    }

    CssAnimationNameValueKind::List
}

pub(crate) fn parse_view_transition_name_value<N>(
    filtered_input: &[u8],
    mut name_callback: N,
) -> CssViewTransitionNameValueKind
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-view-transitions-1/#view-transition-name-prop
    // Value: none | <custom-ident>
    if parser.consume_ident_matching("none") {
        if parser.has_next_component_value() {
            return CssViewTransitionNameValueKind::Invalid;
        }
        return CssViewTransitionNameValueKind::None;
    }

    let Some(name) = parser.parse_a_custom_ident(&["auto", "none"]) else {
        return CssViewTransitionNameValueKind::Invalid;
    };

    // AD-HOC: The current property metadata accepts match-element as a
    // custom ident. The specification now excludes it from <custom-ident>.
    // Keep matching the generated parser until CSSOM represents the
    // match-element value separately.
    name_callback(&name);
    CssViewTransitionNameValueKind::CustomIdent
}

pub(crate) fn parse_a_namespace_rule_prelude<P, U>(
    filtered_input: &[u8],
    mut prefix_callback: P,
    mut namespace_uri_callback: U,
) -> bool
where
    P: FnMut(&str),
    U: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some((prefix, namespace_uri)) = parser.parse_a_namespace_rule_prelude() else {
        return false;
    };

    if let Some(prefix) = prefix {
        prefix_callback(&prefix);
    }
    namespace_uri_callback(&namespace_uri);
    true
}

pub(crate) fn parse_font_feature_values_family_name_list<F>(filtered_input: &[u8], mut family_callback: F) -> bool
where
    F: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let groups = parser.parse_a_comma_separated_list_of_component_values();
    if groups.is_empty() {
        return false;
    }

    for group in groups {
        let mut parser = ComponentValueParser::new(group);
        let Some(family_name) = parser.parse_a_family_name() else {
            return false;
        };
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return false;
        }
        family_callback(&family_name.name);
    }

    true
}

pub(crate) fn parse_font_feature_values_feature_value<V>(filtered_input: &[u8], mut value_callback: V) -> bool
where
    V: FnMut(u32),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(values) = parser.parse_font_feature_values_feature_value() else {
        return false;
    };

    for value in values {
        value_callback(value);
    }
    true
}

pub(crate) fn parse_a_family_name<F>(filtered_input: &[u8], mut family_callback: F) -> bool
where
    F: FnMut(&str, bool),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(family_name) = parser.parse_a_family_name() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    family_callback(&family_name.name, family_name.is_string);
    true
}

pub(crate) fn parse_container_rule_prelude<C>(filtered_input: &[u8], mut condition_callback: C) -> bool
where
    C: FnMut(Option<&str>, Option<&str>),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let groups = parser.parse_a_comma_separated_list_of_component_values();
    if groups.is_empty() {
        return false;
    }

    for group in groups {
        let mut parser = ComponentValueParser::new(group);
        let Some((container_name, container_query)) = parser.parse_container_rule_prelude_item(filtered_input_string)
        else {
            return false;
        };
        condition_callback(container_name.as_deref(), container_query.as_deref());
    }

    true
}

pub(crate) fn parse_a_media_condition<E, M, V, C>(
    filtered_input: &[u8],
    mut event_callback: E,
    mut media_feature_callback: M,
    mut media_feature_value_callback: V,
    mut component_value_callback: C,
) where
    E: FnMut(CssBooleanExpressionEventKind),
    M: FnMut(CssMediaFeature),
    V: FnMut(CssMediaFeatureValue),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/mediaqueries-5/#typedef-media-condition
    // <media-condition> = <media-not> | <media-in-parens> [ <media-and>* | <media-or>* ]
    let mut parser = ComponentValueParser::new(component_values);
    if parser
        .parse_a_boolean_expression(BooleanExpressionTestKind::MediaFeature)
        .is_none()
        || parser.has_next_component_value()
    {
        event_callback(CssBooleanExpressionEventKind::Invalid);
        return;
    }

    let boolean_expression = parser
        .boolean_expression
        .take()
        .expect("parsed expression must be present");
    emit_boolean_expression(
        &boolean_expression,
        filtered_input_string,
        &mut event_callback,
        &mut component_value_callback,
        &mut media_feature_callback,
        &mut media_feature_value_callback,
    );
}

pub(crate) fn parse_a_media_test<E, M, V, C>(
    filtered_input: &[u8],
    mut event_callback: E,
    mut media_feature_callback: M,
    mut media_feature_value_callback: V,
    mut component_value_callback: C,
) where
    E: FnMut(CssBooleanExpressionEventKind),
    M: FnMut(CssMediaFeature),
    V: FnMut(CssMediaFeatureValue),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/css-values-5/#typedef-if-test
    // media( <media-feature> | <media-condition> )
    //
    // https://drafts.csswg.org/mediaqueries-5/#typedef-media-feature
    // <media-feature> = [ <mf-plain> | <mf-boolean> | <mf-range> ]
    if let Some(media_feature) = component_values_parse_as_media_feature(&component_values) {
        event_callback(CssBooleanExpressionEventKind::TestStart);
        media_feature_callback(css_media_feature_from_syntax(&media_feature));
        emit_media_feature_values(&media_feature, filtered_input_string, &mut media_feature_value_callback);
        for component_value in component_values {
            emit_component_value(&component_value, filtered_input_string, &mut component_value_callback);
        }
        event_callback(CssBooleanExpressionEventKind::TestEnd);
        return;
    }

    let mut parser = ComponentValueParser::new(component_values);
    if parser
        .parse_a_boolean_expression(BooleanExpressionTestKind::MediaFeature)
        .is_none()
        || parser.has_next_component_value()
    {
        event_callback(CssBooleanExpressionEventKind::Invalid);
        return;
    }

    let boolean_expression = parser
        .boolean_expression
        .take()
        .expect("parsed expression must be present");
    emit_boolean_expression(
        &boolean_expression,
        filtered_input_string,
        &mut event_callback,
        &mut component_value_callback,
        &mut media_feature_callback,
        &mut media_feature_value_callback,
    );
}

pub(crate) fn parse_a_media_query_list<Q, E, M, V, C>(
    filtered_input: &[u8],
    mut media_query_callback: Q,
    mut event_callback: E,
    mut media_feature_callback: M,
    mut media_feature_value_callback: V,
    mut component_value_callback: C,
) where
    Q: FnMut(CssMediaQuery),
    E: FnMut(CssBooleanExpressionEventKind),
    M: FnMut(CssMediaFeature),
    V: FnMut(CssMediaFeatureValue),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);

    // https://drafts.csswg.org/mediaqueries-5/#typedef-media-query-list
    // To parse a <media-query-list> production,
    // parse a comma-separated list of component values,
    // then parse each entry in the returned list as a <media-query>.
    // Its value is the list of <media-query>s so produced.
    for media_query in parser.parse_a_media_query_list() {
        emit_media_query_syntax(
            media_query,
            filtered_input_string,
            &mut media_query_callback,
            &mut event_callback,
            &mut media_feature_callback,
            &mut media_feature_value_callback,
            &mut component_value_callback,
        );
    }
}

pub(crate) fn parse_a_media_query<Q, E, M, V, C>(
    filtered_input: &[u8],
    mut media_query_callback: Q,
    mut event_callback: E,
    mut media_feature_callback: M,
    mut media_feature_value_callback: V,
    mut component_value_callback: C,
) -> bool
where
    Q: FnMut(CssMediaQuery),
    E: FnMut(CssBooleanExpressionEventKind),
    M: FnMut(CssMediaFeature),
    V: FnMut(CssMediaFeatureValue),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let media_query_list = parser.parse_a_media_query_list();

    // https://www.w3.org/TR/cssom-1/#parse-a-media-query
    // To parse a media query from a string string:
    // 1. Let list be the result of parse a media query list for string.
    // 2. If list is empty, return a MediaQuery object representing "not all".
    if media_query_list.is_empty() {
        emit_not_all_media_query(&mut media_query_callback);
        return true;
    }

    // 3. If list contains more than one MediaQuery object, return null.
    if media_query_list.len() > 1 {
        return false;
    }

    // 4. Return a MediaQuery object representing the sole media query in list.
    emit_media_query_syntax(
        media_query_list.into_iter().next().expect("list must contain one item"),
        filtered_input_string,
        &mut media_query_callback,
        &mut event_callback,
        &mut media_feature_callback,
        &mut media_feature_value_callback,
        &mut component_value_callback,
    );
    true
}

pub(super) fn emit_not_all_media_query<Q>(media_query_callback: &mut Q)
where
    Q: FnMut(CssMediaQuery),
{
    const ALL_MEDIA_TYPE: &[u8] = b"all";

    media_query_callback(CssMediaQuery {
        is_negated: true,
        has_media_condition: false,
        media_type_kind: CssMediaTypeKind::All,
        media_type_ptr: ALL_MEDIA_TYPE.as_ptr(),
        media_type_len: ALL_MEDIA_TYPE.len(),
    });
}

pub(super) fn emit_media_query_syntax<Q, E, M, V, C>(
    media_query: MediaQuerySyntax,
    filtered_input_string: &str,
    media_query_callback: &mut Q,
    event_callback: &mut E,
    media_feature_callback: &mut M,
    media_feature_value_callback: &mut V,
    component_value_callback: &mut C,
) where
    Q: FnMut(CssMediaQuery),
    E: FnMut(CssBooleanExpressionEventKind),
    M: FnMut(CssMediaFeature),
    V: FnMut(CssMediaFeatureValue),
    C: FnMut(CssComponentValue),
{
    match media_query {
        MediaQuerySyntax::Invalid => {
            // https://www.w3.org/TR/mediaqueries-5/#error-handling
            // A media query that does not match the grammar in the previous section must be
            // replaced by `not all` during parsing.
            emit_not_all_media_query(media_query_callback);
        }
        MediaQuerySyntax::Valid {
            modifier,
            media_type,
            condition,
        } => {
            media_query_callback(CssMediaQuery {
                is_negated: modifier == MediaQueryModifier::Not,
                has_media_condition: condition.is_some(),
                media_type_kind: media_type
                    .as_ref()
                    .map_or(CssMediaTypeKind::None, |media_type| css_media_type_kind(media_type)),
                media_type_ptr: media_type
                    .as_ref()
                    .map_or(std::ptr::null(), |media_type| media_type.as_ptr()),
                media_type_len: media_type.as_ref().map_or(0, String::len),
            });
            if let Some(condition) = condition {
                emit_boolean_expression(
                    &condition,
                    filtered_input_string,
                    event_callback,
                    component_value_callback,
                    media_feature_callback,
                    media_feature_value_callback,
                );
            }
        }
    }
}

pub(super) fn css_media_type_kind(media_type: &str) -> CssMediaTypeKind {
    if media_type.eq_ignore_ascii_case("all") {
        return CssMediaTypeKind::All;
    }
    if media_type.eq_ignore_ascii_case("print") {
        return CssMediaTypeKind::Print;
    }
    if media_type.eq_ignore_ascii_case("screen") {
        return CssMediaTypeKind::Screen;
    }
    CssMediaTypeKind::Unknown
}

pub(crate) fn parse_a_component_value<F>(filtered_input: &[u8], mut callback: F)
where
    F: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let Some(component_value) = parser.parse_a_component_value() else {
        return;
    };
    emit_component_value(&component_value, filtered_input_string, &mut callback);
}

pub(crate) fn parse_a_declaration<D, C>(
    filtered_input: &[u8],
    mut declaration_callback: D,
    mut component_value_callback: C,
) where
    D: FnMut(CssDeclaration),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let Some(declaration) = parser.parse_a_declaration() else {
        declaration_callback(CssDeclaration {
            is_valid: false,
            name_ptr: std::ptr::null(),
            name_len: 0,
            important: false,
        });
        return;
    };

    let (name_ptr, name_len) = string_parts(&declaration.name);
    declaration_callback(CssDeclaration {
        is_valid: true,
        name_ptr,
        name_len,
        important: declaration.important,
    });

    for component_value in declaration.value {
        emit_component_value(&component_value, filtered_input_string, &mut component_value_callback);
    }
}

pub(crate) fn parse_a_declaration_with_context<D, C>(
    filtered_input: &[u8],
    rule_context: &[CssRuleContext],
    mut declaration_callback: D,
    mut component_value_callback: C,
) where
    D: FnMut(CssDeclaration),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    parser.rule_context = rule_context.iter().map(|context| RuleContext::from(*context)).collect();
    let Some(declaration) = parser.parse_a_declaration_with_current_context() else {
        declaration_callback(CssDeclaration {
            is_valid: false,
            name_ptr: std::ptr::null(),
            name_len: 0,
            important: false,
        });
        return;
    };

    let (name_ptr, name_len) = string_parts(&declaration.name);
    declaration_callback(CssDeclaration {
        is_valid: true,
        name_ptr,
        name_len,
        important: declaration.important,
    });

    for component_value in declaration.value {
        emit_component_value(&component_value, filtered_input_string, &mut component_value_callback);
    }
}

pub(crate) fn parse_a_rule<E, C>(filtered_input: &[u8], mut event_callback: E, mut component_value_callback: C)
where
    E: FnMut(CssRuleEvent),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let Some(rule) = parser.parse_a_rule() else {
        event_callback(CssRuleEvent::new(CssRuleEventKind::Invalid));
        return;
    };

    emit_rule(
        &rule,
        filtered_input_string,
        &mut event_callback,
        &mut component_value_callback,
    );
}

pub(crate) fn parse_a_blocks_contents<E, C>(
    filtered_input: &[u8],
    mut event_callback: E,
    mut component_value_callback: C,
) where
    E: FnMut(CssRuleEvent),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    parser.rule_context.push(RuleContext::Style);
    let rules_or_lists_of_declarations = parser.parse_a_blocks_contents();
    parser.rule_context.pop();
    emit_rule_or_list_of_declarations_list(
        &rules_or_lists_of_declarations,
        filtered_input_string,
        &mut event_callback,
        &mut component_value_callback,
    );
}

pub(crate) fn parse_a_blocks_contents_with_context<E, C>(
    filtered_input: &[u8],
    rule_context: &[CssRuleContext],
    mut event_callback: E,
    mut component_value_callback: C,
) where
    E: FnMut(CssRuleEvent),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    parser.rule_context = rule_context.iter().map(|context| RuleContext::from(*context)).collect();
    let rules_or_lists_of_declarations = parser.parse_a_blocks_contents();
    emit_rule_or_list_of_declarations_list(
        &rules_or_lists_of_declarations,
        filtered_input_string,
        &mut event_callback,
        &mut component_value_callback,
    );
}

pub(crate) fn parse_a_stylesheets_contents<E, C>(
    filtered_input: &[u8],
    mut event_callback: E,
    mut component_value_callback: C,
) where
    E: FnMut(CssRuleEvent),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let rules = parser.parse_a_stylesheets_contents();
    event_callback(CssRuleEvent::new(CssRuleEventKind::ChildRulesStart));
    for rule in &rules {
        emit_rule(
            rule,
            filtered_input_string,
            &mut event_callback,
            &mut component_value_callback,
        );
    }
    event_callback(CssRuleEvent::new(CssRuleEventKind::ChildRulesEnd));
}
