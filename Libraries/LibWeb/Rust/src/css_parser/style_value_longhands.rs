/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(super) fn rust_owned_counter_function_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-lists-3/#counter-functions
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let [ComponentValue::Function(function)] = strip_whitespace(&component_values) else {
        return None;
    };

    if function.name.eq_ignore_ascii_case("counter") {
        // counter() = counter( <counter-name>, <counter-style>? )
        return Some(RustOwnedStyleValueKind::Counter(rust_owned_counter_function_value(
            function,
            filtered_input_string,
        )?));
    }

    if function.name.eq_ignore_ascii_case("counters") {
        // counters() = counters( <counter-name>, <string>, <counter-style>? )
        return Some(RustOwnedStyleValueKind::Counter(rust_owned_counters_function_value(
            function,
            filtered_input_string,
        )?));
    }

    None
}

pub(super) fn rust_owned_counter_function_value(
    function: &Function,
    filtered_input_string: &str,
) -> Option<RustOwnedCounterFunction> {
    let groups = split_component_values_on_comma(&function.value);
    if groups.is_empty() || groups.len() > 2 {
        return None;
    }

    let counter_name = component_values_counter_name(strip_whitespace(groups[0]))?;
    let counter_style = if groups.len() == 2 {
        Some(component_values_counter_style(
            strip_whitespace(groups[1]),
            filtered_input_string,
        )?)
    } else {
        None
    };

    Some(RustOwnedCounterFunction {
        function: RustOwnedCounterFunctionKind::Counter,
        counter_name,
        join_string: None,
        counter_style,
    })
}

pub(super) fn rust_owned_counters_function_value(
    function: &Function,
    filtered_input_string: &str,
) -> Option<RustOwnedCounterFunction> {
    let groups = split_component_values_on_comma(&function.value);
    if groups.len() < 2 || groups.len() > 3 {
        return None;
    }

    let counter_name = component_values_counter_name(strip_whitespace(groups[0]))?;
    let join_string = component_values_string_value(strip_whitespace(groups[1]))?.to_string();
    let counter_style = if groups.len() == 3 {
        Some(component_values_counter_style(
            strip_whitespace(groups[2]),
            filtered_input_string,
        )?)
    } else {
        None
    };

    Some(RustOwnedCounterFunction {
        function: RustOwnedCounterFunctionKind::Counters,
        counter_name,
        join_string: Some(join_string),
        counter_style,
    })
}

pub(super) fn component_values_counter_name(component_values: &[ComponentValue]) -> Option<String> {
    // https://drafts.csswg.org/css-lists-3/#typedef-counter-name
    // Counters are referred to in CSS syntax using the <counter-name> type, which represents
    // their name as a <custom-ident>. A <counter-name> name cannot match the keyword none;
    // such an identifier is invalid as a <counter-name>.
    let name = component_values_custom_ident_value(component_values)?;
    if name.eq_ignore_ascii_case("none") {
        return None;
    }
    Some(name.to_string())
}

pub(super) fn component_values_counter_style(
    component_values: &[ComponentValue],
    filtered_input_string: &str,
) -> Option<CounterStyle> {
    let serialized_counter_style = serialize_component_values_for_reparsing(component_values, filtered_input_string)?;
    parse_all_component_values(
        serialized_counter_style.as_bytes(),
        ComponentValueParser::parse_a_counter_style,
    )
}

pub(super) fn rust_owned_basic_shape_style_value_kind(
    filtered_input: &[u8],
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let [ComponentValue::Function(function)] = strip_whitespace(&component_values) else {
        return None;
    };

    // https://drafts.csswg.org/css-shapes-1/#typedef-basic-shape
    // <basic-shape> = <inset()> | <circle()> | <ellipse()> | <polygon()> | <path()> | <rect()> | <xywh()>
    let mut fill_rule = RustOwnedBasicShapeFillRule::Nonzero;
    let mut rectangle_components = vec![];
    let mut rectangle_border_radius = None;
    let mut radial_shape_radius = vec![];
    let mut radial_shape_position = None;
    let mut polygon_points = vec![];
    let mut path_data = None;
    let kind = if function.name.eq_ignore_ascii_case("inset") {
        let rectangle = parse_owned_inset_basic_shape_function(function, filtered_input_string)?;
        rectangle_components = rectangle.components;
        rectangle_border_radius = rectangle.border_radius;
        Some(RustOwnedBasicShapeKind::Inset)
    } else if function.name.eq_ignore_ascii_case("xywh") {
        let rectangle = parse_owned_xywh_basic_shape_function(function, filtered_input_string)?;
        rectangle_components = rectangle.components;
        rectangle_border_radius = rectangle.border_radius;
        Some(RustOwnedBasicShapeKind::Xywh)
    } else if function.name.eq_ignore_ascii_case("rect") {
        let rectangle = parse_owned_rect_basic_shape_function(function, filtered_input_string)?;
        rectangle_components = rectangle.components;
        rectangle_border_radius = rectangle.border_radius;
        Some(RustOwnedBasicShapeKind::Rect)
    } else if function.name.eq_ignore_ascii_case("circle") {
        let radial_shape = parse_owned_circle_or_ellipse_basic_shape_function(
            function,
            BasicShapeRadialFunction::Circle,
            filtered_input_string,
        )?;
        radial_shape_radius = radial_shape.radius;
        radial_shape_position = radial_shape.position;
        Some(RustOwnedBasicShapeKind::Circle)
    } else if function.name.eq_ignore_ascii_case("ellipse") {
        let radial_shape = parse_owned_circle_or_ellipse_basic_shape_function(
            function,
            BasicShapeRadialFunction::Ellipse,
            filtered_input_string,
        )?;
        radial_shape_radius = radial_shape.radius;
        radial_shape_position = radial_shape.position;
        Some(RustOwnedBasicShapeKind::Ellipse)
    } else if function.name.eq_ignore_ascii_case("polygon") {
        let polygon = parse_owned_polygon_basic_shape_function(function, filtered_input_string)?;
        fill_rule = polygon.fill_rule;
        polygon_points = polygon.points;
        Some(RustOwnedBasicShapeKind::Polygon)
    } else if function.name.eq_ignore_ascii_case("path") {
        let path = parse_path_basic_shape_function(function)?;
        fill_rule = path.fill_rule;
        path_data = Some(path.path_data);
        Some(RustOwnedBasicShapeKind::Path)
    } else {
        None
    }?;

    Some(RustOwnedStyleValueKind::BasicShape(Box::new(RustOwnedBasicShape {
        kind,
        fill_rule,
        rectangle_components,
        rectangle_border_radius,
        radial_shape_radius,
        radial_shape_position,
        polygon_points,
        path_data,
    })))
}

pub(super) fn rust_owned_transformation_style_value_kind(
    function: &Function,
    _component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedTransformation> {
    // https://drafts.csswg.org/css-transforms-1/#typedef-transform-function
    // <transform-function> = <matrix()> | <translate()> | <translateX()> | <translateY()> | <scale()> | <scaleX()> | <scaleY()> | <rotate()> | <skew()> | <skewX()> | <skewY()>
    let transform_function = transform_function_from_name(&function.name)?;
    let parameters = transform_function_parameters(transform_function);
    let arguments = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        let [component_value] = strip_whitespace(&component_values) else {
            return None;
        };
        Some(component_value.clone())
    })?;

    if arguments.len() > parameters.len() {
        return None;
    }
    if arguments.len() < parameters.len() && parameters[arguments.len()].required {
        return None;
    }

    let mut rust_owned_arguments = Vec::new();
    for (argument, parameter) in arguments.iter().zip(parameters) {
        if !component_value_matches_transform_function_parameter(argument, parameter.parameter_type) {
            return None;
        }

        rust_owned_arguments.push(RustOwnedTransformationArgument {
            parameter_type: parameter.parameter_type,
            value: component_value_parse_as_nested_transform_function_argument(
                argument,
                parameter.parameter_type,
                filtered_input_string,
            )?,
        });
    }

    Some(RustOwnedTransformation {
        function: transform_function,
        arguments: rust_owned_arguments,
    })
}

pub(super) fn parse_all_component_values<T>(
    filtered_input: &[u8],
    parse: impl FnOnce(&mut ComponentValueParser) -> Option<T>,
) -> Option<T> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let values = parse(&mut parser)?;
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    Some(values)
}

pub(super) fn rust_owned_font_style_style_value_kind(source: String) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(source.as_bytes());
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let value = parser.parse_a_font_style()?;
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    let angle = if matches!(value, FontStyle::Oblique { has_angle: true }) {
        let angle_component_value = strip_whitespace(&parser.component_values)
            .iter()
            .filter(|component_value| !is_whitespace_component_value(component_value))
            .nth(1)?;
        Some(serialize_component_values_for_reparsing(
            std::slice::from_ref(angle_component_value),
            filtered_input_string,
        )?)
    } else {
        None
    };

    Some(RustOwnedStyleValueKind::FontStyle(RustOwnedFontStyle { value, angle }))
}

pub(super) fn rust_owned_font_family_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let mut values = Vec::new();
    if !parse_a_font_family_value(filtered_input, |value| values.push(value.clone())) {
        return None;
    }
    Some(RustOwnedStyleValueKind::FontFamily(RustOwnedFontFamilyList { values }))
}

pub(super) fn rust_owned_font_feature_settings_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let mut kind = CssOpenTypeSettingsKind::Normal;
    let mut tag_values = Vec::new();
    if !parse_a_font_feature_settings(
        filtered_input,
        |parsed_kind| kind = parsed_kind,
        |value| {
            tag_values.push(value.clone());
        },
    ) {
        return None;
    }
    Some(RustOwnedStyleValueKind::OpenTypeSettings(
        RustOwnedOpenTypeSettingsStyleValue {
            kind: RustOwnedOpenTypeSettingsStyleValueKind::FontFeatureSettings,
            value: RustOwnedOpenTypeSettings { kind, tag_values },
        },
    ))
}

pub(super) fn rust_owned_font_language_override_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let mut kind = CssFontLanguageOverrideKind::Normal;
    let mut value = None;
    if !parse_a_font_language_override(filtered_input, |parsed_kind, parsed_value| {
        kind = parsed_kind;
        value = parsed_value.map(ToString::to_string);
    }) {
        return None;
    }
    Some(RustOwnedStyleValueKind::FontLanguageOverride(
        RustOwnedFontLanguageOverride { kind, value },
    ))
}

pub(super) fn rust_owned_font_variant_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let font_variant = parser.parse_a_font_variant()?;
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }

    Some(RustOwnedStyleValueKind::FontVariant(font_variant))
}

pub(super) fn rust_owned_font_variation_settings_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let mut kind = CssOpenTypeSettingsKind::Normal;
    let mut tag_values = Vec::new();
    if !parse_a_font_variation_settings(
        filtered_input,
        |parsed_kind| kind = parsed_kind,
        |value| {
            tag_values.push(value.clone());
        },
    ) {
        return None;
    }
    Some(RustOwnedStyleValueKind::OpenTypeSettings(
        RustOwnedOpenTypeSettingsStyleValue {
            kind: RustOwnedOpenTypeSettingsStyleValueKind::FontVariationSettings,
            value: RustOwnedOpenTypeSettings { kind, tag_values },
        },
    ))
}

pub(super) fn rust_owned_anchor_name_or_scope_style_value_kind(
    filtered_input: &[u8],
    allow_all: bool,
) -> Option<RustOwnedStyleValueKind> {
    let mut names = Vec::new();
    let kind = parse_anchor_name_or_scope_value(filtered_input, allow_all, |name| names.push(name.to_string()));
    if kind == CssAnchorNameOrScopeValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::AnchorNameOrScope(RustOwnedAnchorNameOrScope {
        kind,
        names,
    }))
}

pub(super) fn rust_owned_animation_name_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let mut names = Vec::new();
    let kind = parse_animation_name_value(filtered_input, |kind, value| {
        names.push(RustOwnedAnimationNameItem {
            kind,
            value: value.to_string(),
        });
    });
    if kind == CssAnimationNameValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::AnimationName(RustOwnedAnimationName {
        kind,
        names,
    }))
}

pub(super) fn rust_owned_color_scheme_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let mut schemes = Vec::new();
    let value = parse_color_scheme_value(filtered_input, |scheme| schemes.push(scheme.to_string()));
    if value.kind == CssColorSchemeValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::ColorScheme(RustOwnedColorScheme {
        value,
        schemes,
    }))
}

pub(super) fn rust_owned_display_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    parse_display_value(filtered_input).map(RustOwnedStyleValueKind::Display)
}

pub(super) fn rust_owned_flex_shorthand_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    let flex_factor_from_component_value = |component_value: &ComponentValue| {
        component_value_parse_as_nested_non_negative_number(component_value, filtered_input_string)
    };
    let flex_basis_from_component_value = |component_value: &ComponentValue| {
        rust_owned_flex_basis_from_component_value(component_value, filtered_input_string)
    };

    // https://drafts.csswg.org/css-flexbox-1/#flex-property
    // Value: none | [ <'flex-grow'> <'flex-shrink'>? || <'flex-basis'> ]
    let value = match component_values.as_slice() {
        [component_value] if component_value_is_ident(Some(component_value), "none") => RustOwnedFlexShorthand::None,
        [component_value] if component_value_parse_as_non_negative_number(component_value) => {
            // NOTE: The spec says that flex-basis should be 0 here, but other engines currently use 0%.
            // https://github.com/w3c/csswg-drafts/issues/5742
            RustOwnedFlexShorthand::Longhands {
                flex_grow: flex_factor_from_component_value(component_value)?,
                flex_shrink: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_basis: RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::Percentage(0.0)),
            }
        }
        [component_value] if component_value_parse_as_flex_basis(component_value) => {
            RustOwnedFlexShorthand::Longhands {
                flex_grow: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_shrink: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_basis: flex_basis_from_component_value(component_value)?,
            }
        }
        [flex_grow, flex_shrink]
            if component_value_parse_as_non_negative_number(flex_grow)
                && component_value_parse_as_non_negative_number(flex_shrink) =>
        {
            // NOTE: The spec says that flex-basis should be 0 here, but other engines currently use 0%.
            // https://github.com/w3c/csswg-drafts/issues/5742
            RustOwnedFlexShorthand::Longhands {
                flex_grow: flex_factor_from_component_value(flex_grow)?,
                flex_shrink: flex_factor_from_component_value(flex_shrink)?,
                flex_basis: RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::Percentage(0.0)),
            }
        }
        [flex_grow, flex_basis]
            if component_value_parse_as_non_negative_number(flex_grow)
                && component_value_parse_as_flex_basis(flex_basis) =>
        {
            RustOwnedFlexShorthand::Longhands {
                flex_grow: flex_factor_from_component_value(flex_grow)?,
                flex_shrink: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_basis: flex_basis_from_component_value(flex_basis)?,
            }
        }
        [flex_basis, flex_grow]
            if component_value_parse_as_flex_basis_before_flex_factors(flex_basis)
                && component_value_parse_as_non_negative_number(flex_grow) =>
        {
            RustOwnedFlexShorthand::Longhands {
                flex_grow: flex_factor_from_component_value(flex_grow)?,
                flex_shrink: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_basis: flex_basis_from_component_value(flex_basis)?,
            }
        }
        [flex_grow, flex_shrink, flex_basis]
            if component_value_parse_as_non_negative_number(flex_grow)
                && component_value_parse_as_non_negative_number(flex_shrink)
                && component_value_parse_as_flex_basis(flex_basis) =>
        {
            RustOwnedFlexShorthand::Longhands {
                flex_grow: flex_factor_from_component_value(flex_grow)?,
                flex_shrink: flex_factor_from_component_value(flex_shrink)?,
                flex_basis: flex_basis_from_component_value(flex_basis)?,
            }
        }
        [flex_basis, flex_grow, flex_shrink]
            if component_value_parse_as_flex_basis_before_flex_factors(flex_basis)
                && component_value_parse_as_non_negative_number(flex_grow)
                && component_value_parse_as_non_negative_number(flex_shrink) =>
        {
            RustOwnedFlexShorthand::Longhands {
                flex_grow: flex_factor_from_component_value(flex_grow)?,
                flex_shrink: flex_factor_from_component_value(flex_shrink)?,
                flex_basis: flex_basis_from_component_value(flex_basis)?,
            }
        }
        _ => return None,
    };

    Some(RustOwnedStyleValueKind::FlexShorthand(value))
}

pub(super) fn rust_owned_flex_basis_from_component_value(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedFlexBasis> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("auto") => {
            return Some(RustOwnedFlexBasis::Value(auto_keyword()));
        }
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("content") => {
            return Some(RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::Keyword(
                "content".to_string(),
            )));
        }
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("fit-content") => {
            return Some(RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::Keyword(
                "fit-content".to_string(),
            )));
        }
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("min-content") => {
            return Some(RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::Keyword(
                "min-content".to_string(),
            )));
        }
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("max-content") => {
            return Some(RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::Keyword(
                "max-content".to_string(),
            )));
        }
        _ => {}
    }

    if component_value_parse_as_non_negative_length_percentage(component_value) {
        return component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)
            .map(RustOwnedFlexBasis::Value);
    }

    if let ComponentValue::Function(function) = component_value
        && function.name.eq_ignore_ascii_case("fit-content")
    {
        // https://drafts.csswg.org/css-sizing-3/#funcdef-width-fit-content
        // fit-content() = fit-content( <length-percentage [0,∞]> )
        let [component_value] = strip_whitespace(&function.value) else {
            return None;
        };
        if !component_value_parse_as_non_negative_length_percentage(component_value) {
            return None;
        }
        return component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)
            .map(RustOwnedFlexBasis::FitContentFunction);
    }

    serialize_component_values_for_reparsing(std::slice::from_ref(component_value), filtered_input_string)
        .map(RustOwnedNestedPrimitiveValue::Source)
        .map(RustOwnedFlexBasis::Value)
}

pub(super) fn rust_owned_flex_flow_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://drafts.csswg.org/css-flexbox-1/#flex-flow-property
    // Value: <'flex-direction'> || <'flex-wrap'>
    if component_values.is_empty() || component_values.len() > 2 {
        return None;
    }

    let mut flex_direction = None;
    let mut flex_wrap = None;

    for component_value in &component_values {
        if let Some(value) = rust_owned_flex_direction_from_component_value(component_value) {
            if flex_direction.is_some() {
                return None;
            }
            flex_direction = Some(value);
            continue;
        }

        if let Some(value) = rust_owned_flex_wrap_from_component_value(component_value) {
            if flex_wrap.is_some() {
                return None;
            }
            flex_wrap = Some(value);
            continue;
        }

        return None;
    }

    Some(RustOwnedStyleValueKind::FlexFlow(RustOwnedFlexFlow {
        flex_direction,
        flex_wrap,
    }))
}

pub(super) fn rust_owned_flex_direction_from_component_value(
    component_value: &ComponentValue,
) -> Option<RustOwnedFlexDirection> {
    let ident = component_value_ident(component_value)?;
    if ident.eq_ignore_ascii_case("row") {
        return Some(RustOwnedFlexDirection::Row);
    }
    if ident.eq_ignore_ascii_case("row-reverse") {
        return Some(RustOwnedFlexDirection::RowReverse);
    }
    if ident.eq_ignore_ascii_case("column") {
        return Some(RustOwnedFlexDirection::Column);
    }
    if ident.eq_ignore_ascii_case("column-reverse") {
        return Some(RustOwnedFlexDirection::ColumnReverse);
    }
    None
}

pub(super) fn rust_owned_flex_wrap_from_component_value(component_value: &ComponentValue) -> Option<RustOwnedFlexWrap> {
    let ident = component_value_ident(component_value)?;
    if ident.eq_ignore_ascii_case("nowrap") {
        return Some(RustOwnedFlexWrap::Nowrap);
    }
    if ident.eq_ignore_ascii_case("wrap") {
        return Some(RustOwnedFlexWrap::Wrap);
    }
    if ident.eq_ignore_ascii_case("wrap-reverse") {
        return Some(RustOwnedFlexWrap::WrapReverse);
    }
    None
}

pub(super) fn rust_owned_filter_value_list_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    parse_rust_owned_filter_value_list_value(filtered_input).map(RustOwnedStyleValueKind::FilterValueList)
}

pub(super) fn rust_owned_contain_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_contain_value(filtered_input);
    if value.kind == CssContainValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::Contain(RustOwnedContain { value }))
}

pub(super) fn rust_owned_container_type_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_container_type_value(filtered_input);
    if value == CssContainerTypeValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::ContainerType(RustOwnedContainerType { value }))
}

pub(super) fn rust_owned_counter_definitions_style_value_kind(
    filtered_input: &[u8],
    allow_reversed: bool,
    default_value_if_not_reversed: i32,
) -> Option<RustOwnedStyleValueKind> {
    let source = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    if let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values.as_slice()
        && value.eq_ignore_ascii_case("none")
    {
        return Some(RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(
            "none".to_string(),
        )));
    }

    let mut parser = ComponentValueParser::new(component_values);
    let mut definitions = Vec::new();

    loop {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };

        // https://drafts.csswg.org/css-lists-3/#typedef-counter-name
        // <counter-name> = <custom-ident>
        // A <counter-name> name cannot match the keyword none; such an identifier is invalid as a <counter-name>.
        let (name, is_reversed) = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_valid_custom_ident(value, &["none"]) => {
                let name = value.clone();
                parser.index += 1;
                (name, false)
            }
            // AD-HOC: Match the existing C++ parser, which currently disables reversed() counter parsing.
            ComponentValue::Function(_) if allow_reversed => return None,
            _ => return None,
        };
        parser.discard_whitespace();

        let mut value = None;
        if let Some(component_value) = parser.next_component_value()
            && parse_integer_value_prefix(component_value) == CssPrimitiveValueKind::Integer
        {
            value = Some(component_value_parse_as_nested_integer(component_value, &source)?);
            parser.index += 1;
        }

        definitions.push(RustOwnedCounterDefinition {
            name,
            is_reversed,
            value: value.unwrap_or(RustOwnedNestedPrimitiveValue::Integer(default_value_if_not_reversed)),
        });
    }

    if definitions.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::CounterDefinitions(
        RustOwnedCounterDefinitions { definitions },
    ))
}

pub(super) fn rust_owned_grid_auto_flow_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    // https://www.w3.org/TR/css-grid-1/#grid-auto-flow-property
    // grid-auto-flow = [ row | column ] || dense
    let axis = consume_optional_grid_auto_flow_axis_value(&mut parser);
    let dense = consume_optional_ident_matching(&mut parser, "dense");
    let axis_after_dense = if axis.is_some() {
        None
    } else {
        consume_optional_grid_auto_flow_axis_value(&mut parser)
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() || (axis.is_none() && !dense && axis_after_dense.is_none()) {
        return None;
    }

    let axis = axis.or(axis_after_dense).unwrap_or(CssGridAutoFlowAxis::Row);
    let dense = if dense {
        CssGridAutoFlowDense::Yes
    } else {
        CssGridAutoFlowDense::No
    };

    Some(RustOwnedStyleValueKind::GridAutoFlow(RustOwnedGridAutoFlow {
        axis,
        dense,
    }))
}

pub(super) fn consume_optional_grid_auto_flow_axis_value(
    parser: &mut ComponentValueParser,
) -> Option<CssGridAutoFlowAxis> {
    parser.discard_whitespace();
    if parser.consume_ident_matching("row") {
        return Some(CssGridAutoFlowAxis::Row);
    }
    if parser.consume_ident_matching("column") {
        return Some(CssGridAutoFlowAxis::Column);
    }
    None
}

pub(super) fn rust_owned_grid_track_placement_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    Some(RustOwnedStyleValueKind::GridTrackPlacement(
        parse_rust_owned_grid_track_placement_value(filtered_input)?,
    ))
}

pub(super) fn rust_owned_grid_auto_track_sizes_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    Some(RustOwnedStyleValueKind::GridAutoTrackSizes(
        parse_rust_owned_grid_track_size_list_value(filtered_input, GridTrackSizeListSyntax::TrackSizeList)?,
    ))
}

pub(super) fn rust_owned_grid_template_areas_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    Some(RustOwnedStyleValueKind::GridTemplateAreas(
        parse_rust_owned_grid_template_areas_value(filtered_input)?,
    ))
}

pub(super) fn rust_owned_grid_track_size_list_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    Some(RustOwnedStyleValueKind::GridTrackSizeList(
        parse_rust_owned_grid_track_size_list_value(filtered_input, GridTrackSizeListSyntax::TrackList)?,
    ))
}

pub(super) fn rust_owned_list_style_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    if component_values.is_empty() {
        return None;
    }

    let mut position = None;
    let mut image = None;
    let mut list_style_type = None;
    let mut found_nones = 0_u8;

    for component_value in &component_values {
        if component_value_is_ident(Some(component_value), "none") {
            found_nones += 1;
            continue;
        }

        if image.is_none()
            && let Some(value) =
                rust_owned_list_style_image_from_component_value(component_value, filtered_input_string)
        {
            image = Some(value);
            continue;
        }

        if position.is_none() && component_value_is_list_style_position(component_value) {
            position = Some(rust_owned_list_style_position_from_component_value(component_value)?);
            continue;
        }

        if list_style_type.is_none()
            && let Some(value) = rust_owned_list_style_type_from_component_value(component_value)
        {
            list_style_type = Some(value);
            continue;
        }

        return None;
    }

    if found_nones > 2 {
        return None;
    }

    // https://drafts.csswg.org/css-lists-3/#propdef-list-style
    // <'list-style-position'> || <'list-style-image'> || <'list-style-type'>
    //
    // Since `none` is valid for both list-style-image and list-style-type, the
    // shorthand needs to defer assigning it until the unambiguous components are
    // known.
    if found_nones == 2 {
        if image.is_some() || list_style_type.is_some() {
            return None;
        }
        image = Some(RustOwnedListStyleImage::None);
        list_style_type = Some(RustOwnedListStyleType::None);
    } else if found_nones == 1 {
        if image.is_some() && list_style_type.is_some() {
            return None;
        }
        if image.is_none() {
            image = Some(RustOwnedListStyleImage::None);
        }
        if list_style_type.is_none() {
            list_style_type = Some(RustOwnedListStyleType::None);
        }
    }

    Some(RustOwnedStyleValueKind::ListStyle(RustOwnedListStyle {
        position,
        image,
        list_style_type,
    }))
}

pub(super) fn rust_owned_math_depth_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://w3c.github.io/mathml-core/#propdef-math-depth
    // Value: auto-add | add(<integer>) | <integer>
    if let [component_value] = component_values.as_slice()
        && component_value_is_ident(Some(component_value), "auto-add")
    {
        return Some(RustOwnedStyleValueKind::MathDepth(RustOwnedMathDepth::AutoAdd));
    }

    if let [ComponentValue::Function(function)] = component_values.as_slice()
        && function.name.eq_ignore_ascii_case("add")
    {
        let integer_source = serialize_component_values_for_reparsing(&function.value, filtered_input_string)?;
        if !component_values_parse_as_property_value_type(PropertyValueType::Integer, integer_source.as_bytes()) {
            return None;
        }
        let integer = match remove_whitespace_component_values(&function.value).as_slice() {
            [component_value] => component_value_parse_as_nested_integer(component_value, filtered_input_string)?,
            // AD-HOC: The Rust side only recognizes the syntactic branch here.
            // Materializing and range-checking function values still happens in C++.
            _ => RustOwnedNestedPrimitiveValue::Source(integer_source),
        };
        return Some(RustOwnedStyleValueKind::MathDepth(RustOwnedMathDepth::Add { integer }));
    }

    if component_values.len() != 1 {
        return None;
    }

    let integer_source = serialize_component_values_for_reparsing(&component_values, filtered_input_string)?;
    if !component_values_parse_as_property_value_type(PropertyValueType::Integer, integer_source.as_bytes()) {
        return None;
    }
    Some(RustOwnedStyleValueKind::MathDepth(RustOwnedMathDepth::Integer {
        integer: component_value_parse_as_nested_integer(&component_values[0], filtered_input_string)?,
    }))
}

pub(super) fn rust_owned_paint_order_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_paint_order_value(filtered_input);
    if value.kind == CssPaintOrderValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::PaintOrder(RustOwnedPaintOrder { value }))
}

pub(super) fn rust_owned_align_content_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_keyword_list_style_value_kind(filtered_input, component_values_parse_as_align_content)
}

pub(super) fn rust_owned_justify_content_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_keyword_list_style_value_kind(filtered_input, component_values_parse_as_justify_content)
}

pub(super) fn rust_owned_align_items_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_keyword_list_style_value_kind(filtered_input, component_values_parse_as_align_items)
}

pub(super) fn rust_owned_justify_items_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_keyword_list_style_value_kind(filtered_input, component_values_parse_as_justify_items)
}

pub(super) fn rust_owned_align_self_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_keyword_list_style_value_kind(filtered_input, component_values_parse_as_align_self)
}

pub(super) fn rust_owned_justify_self_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_keyword_list_style_value_kind(filtered_input, component_values_parse_as_justify_self)
}

pub(super) fn rust_owned_keyword_list_style_value_kind(
    filtered_input: &[u8],
    parse_value: fn(&[ComponentValue]) -> bool,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    if !parse_value(&component_values) {
        return None;
    }

    Some(RustOwnedStyleValueKind::KeywordList(RustOwnedKeywordList {
        keywords: component_values_to_ident_keywords(&component_values)?,
    }))
}

pub(super) fn rust_owned_place_content_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_place_shorthand_style_value_kind(
        filtered_input,
        component_values_parse_as_align_content,
        component_values_parse_as_justify_content,
        RustOwnedStyleValueKind::PlaceContent,
    )
}

pub(super) fn rust_owned_place_items_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_place_shorthand_style_value_kind(
        filtered_input,
        component_values_parse_as_align_items,
        component_values_parse_as_justify_items,
        RustOwnedStyleValueKind::PlaceItems,
    )
}

pub(super) fn rust_owned_place_self_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    rust_owned_place_shorthand_style_value_kind(
        filtered_input,
        component_values_parse_as_align_self,
        component_values_parse_as_justify_self,
        RustOwnedStyleValueKind::PlaceSelf,
    )
}

pub(super) fn rust_owned_place_shorthand_style_value_kind(
    filtered_input: &[u8],
    parse_align_value: fn(&[ComponentValue]) -> bool,
    parse_justify_value: fn(&[ComponentValue]) -> bool,
    style_value_kind: impl FnOnce(RustOwnedPlaceShorthand) -> RustOwnedStyleValueKind,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let non_whitespace_component_values = remove_whitespace_component_values(component_values);

    if non_whitespace_component_values.is_empty() {
        return None;
    }

    if parse_align_value(&non_whitespace_component_values) && parse_justify_value(&non_whitespace_component_values) {
        let keywords = component_values_to_ident_keywords(&non_whitespace_component_values)?;
        return Some(style_value_kind(RustOwnedPlaceShorthand {
            align_keywords: keywords.clone(),
            justify_keywords: keywords,
        }));
    }

    let non_whitespace_component_indices = component_values
        .iter()
        .enumerate()
        .filter_map(|(index, component_value)| (!is_whitespace_component_value(component_value)).then_some(index))
        .collect::<Vec<_>>();

    for component_split_index in non_whitespace_component_indices.iter().skip(1) {
        let component_split_index = *component_split_index;
        let align_component_values = strip_whitespace(&component_values[..component_split_index]);
        let justify_component_values = strip_whitespace(&component_values[component_split_index..]);
        let align_values = remove_whitespace_component_values(align_component_values);
        let justify_values = remove_whitespace_component_values(justify_component_values);
        if parse_align_value(&align_values) && parse_justify_value(&justify_values) {
            return Some(style_value_kind(RustOwnedPlaceShorthand {
                align_keywords: component_values_to_ident_keywords(&align_values)?,
                justify_keywords: component_values_to_ident_keywords(&justify_values)?,
            }));
        }
    }

    None
}

pub(super) fn component_values_to_ident_keywords(component_values: &[ComponentValue]) -> Option<Vec<String>> {
    let mut keywords = Vec::with_capacity(component_values.len());
    for component_value in component_values {
        let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) = component_value
        else {
            return None;
        };
        keywords.push(value.to_ascii_lowercase());
    }
    Some(keywords)
}

pub(super) fn rust_owned_position_anchor_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let mut name = None;
    let kind = parse_position_anchor_value(filtered_input, |value| name = Some(value.to_string()));
    if kind == CssPositionAnchorValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::PositionAnchor(RustOwnedPositionAnchor {
        kind,
        name,
    }))
}

pub(super) fn rust_owned_position_area_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    Some(RustOwnedStyleValueKind::PositionArea(
        parse_rust_owned_position_area_value(filtered_input)?,
    ))
}

pub(super) fn rust_owned_position_try_fallbacks_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    Some(RustOwnedStyleValueKind::PositionTryFallbacks(
        parse_rust_owned_position_try_fallbacks_value(filtered_input)?,
    ))
}

pub(super) fn rust_owned_position_try_order_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_position_try_order_value(filtered_input);
    if value == CssPositionTryOrderValue::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::PositionTryOrder(RustOwnedPositionTryOrder {
        value,
    }))
}

pub(super) fn rust_owned_position_visibility_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let value = parse_position_visibility_value(filtered_input);
    if value.kind == CssPositionVisibilityValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::PositionVisibility(
        RustOwnedPositionVisibility { value },
    ))
}

pub(super) fn rust_owned_position_style_value_kind(
    value_type: PropertyValueType,
    source: String,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(source.as_bytes());
    let component_values = parser.parse_a_list_of_component_values();
    let allow_background_position_3_value_syntax = value_type == PropertyValueType::BackgroundPosition;
    let value = parse_rust_owned_position_value(
        strip_whitespace(&component_values),
        &source,
        allow_background_position_3_value_syntax,
    )?;

    Some(RustOwnedStyleValueKind::Position(RustOwnedPosition {
        value_type,
        value,
    }))
}

pub(super) fn rust_owned_position_list_style_value_kind(
    value_type: PropertyValueType,
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let source = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let values = parse_comma_separated_component_values(component_values, |component_values| {
        serialize_component_values_for_reparsing(strip_whitespace(&component_values), &source)?;
        let allow_background_position_3_value_syntax = value_type == PropertyValueType::BackgroundPosition;
        parse_rust_owned_position_value(
            strip_whitespace(&component_values),
            &source,
            allow_background_position_3_value_syntax,
        )
        .map(RustOwnedPositionListItem::Position)
    })?;

    if values.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::PositionList(RustOwnedPositionList {
        value_type,
        values,
    }))
}

pub(super) fn rust_owned_background_position_longhand_list_style_value_kind(
    property_id: PropertyId,
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let source = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let is_horizontal = property_id == PropertyId::BackgroundPositionX;
    let values = parse_comma_separated_component_values(component_values, |component_values| {
        serialize_component_values_for_reparsing(strip_whitespace(&component_values), &source)?;
        parse_rust_owned_background_position_longhand_value(strip_whitespace(&component_values), &source, is_horizontal)
            .map(RustOwnedPositionListItem::Component)
    })?;

    if values.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::PositionList(RustOwnedPositionList {
        value_type: PropertyValueType::BackgroundPosition,
        values,
    }))
}

pub(super) fn rust_owned_quotes_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let mut strings = Vec::new();
    let kind = parse_quotes_value(filtered_input, |string| strings.push(string.to_string()));
    if kind == CssQuotesValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::Quotes(RustOwnedQuotes { kind, strings }))
}

pub(super) fn rust_owned_repeat_style_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let values = parse_comma_separated_component_values(component_values, |component_values| {
        rust_owned_repeat_style_from_component_values(component_values)
    })?;

    if values.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::RepeatStyle(RustOwnedRepeatStyleList {
        values,
    }))
}

pub(super) fn rust_owned_repeat_style_from_component_values(
    component_values: Vec<ComponentValue>,
) -> Option<RustOwnedRepeatStyle> {
    let mut parser = ComponentValueParser::new(component_values);

    // https://drafts.csswg.org/css-backgrounds-3/#typedef-repeat-style
    // <repeat-style> = repeat-x | repeat-y | [ repeat | space | round | no-repeat ]{1,2}
    if consume_optional_ident_matching(&mut parser, "repeat-x") && parser_has_no_remaining_component_values(&mut parser)
    {
        return Some(RustOwnedRepeatStyle {
            repeat_x: CssRepeatStyleRepetition::Repeat,
            repeat_y: CssRepeatStyleRepetition::NoRepeat,
        });
    }

    if consume_optional_ident_matching(&mut parser, "repeat-y") && parser_has_no_remaining_component_values(&mut parser)
    {
        return Some(RustOwnedRepeatStyle {
            repeat_x: CssRepeatStyleRepetition::NoRepeat,
            repeat_y: CssRepeatStyleRepetition::Repeat,
        });
    }

    let repeat_x = consume_non_directional_repeat_style_value_kind(&mut parser)?;
    let repeat_y = consume_non_directional_repeat_style_value_kind(&mut parser).unwrap_or(repeat_x);
    if !parser_has_no_remaining_component_values(&mut parser) {
        return None;
    }

    Some(RustOwnedRepeatStyle { repeat_x, repeat_y })
}

pub(super) fn consume_non_directional_repeat_style_value_kind(
    parser: &mut ComponentValueParser,
) -> Option<CssRepeatStyleRepetition> {
    parser.discard_whitespace();
    if parser.consume_ident_matching("repeat") {
        return Some(CssRepeatStyleRepetition::Repeat);
    }
    if parser.consume_ident_matching("space") {
        return Some(CssRepeatStyleRepetition::Space);
    }
    if parser.consume_ident_matching("round") {
        return Some(CssRepeatStyleRepetition::Round);
    }
    if parser.consume_ident_matching("no-repeat") {
        return Some(CssRepeatStyleRepetition::NoRepeat);
    }
    None
}

pub(super) fn rust_owned_background_size_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let source = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/css-backgrounds-3/#typedef-bg-size
    // <bg-size> = [ <length-percentage [0,∞]> | auto ]{1,2} | cover | contain
    let values = parse_comma_separated_component_values(component_values, |component_values| {
        rust_owned_background_size_from_component_values(component_values, &source)
    })?;

    if values.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::BackgroundSize(RustOwnedBackgroundSizeList {
        values,
    }))
}

pub(super) fn rust_owned_background_size_from_component_values(
    component_values: Vec<ComponentValue>,
    source: &str,
) -> Option<RustOwnedBackgroundSize> {
    let component_values = remove_whitespace_component_values(&component_values);

    if let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values.as_slice()
    {
        if value.eq_ignore_ascii_case("cover") {
            return Some(RustOwnedBackgroundSize::Cover);
        }
        if value.eq_ignore_ascii_case("contain") {
            return Some(RustOwnedBackgroundSize::Contain);
        }
    }

    let [width] = component_values.as_slice() else {
        let [width, height] = component_values.as_slice() else {
            return None;
        };
        let width = rust_owned_background_size_component_from_component_value(width, source)?;
        let height = rust_owned_background_size_component_from_component_value(height, source)?;
        return Some(RustOwnedBackgroundSize::Explicit {
            width,
            height: Some(height),
        });
    };

    Some(RustOwnedBackgroundSize::Explicit {
        width: rust_owned_background_size_component_from_component_value(width, source)?,
        height: None,
    })
}

pub(super) fn rust_owned_background_size_component_from_component_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    }) = component_value
        && value.eq_ignore_ascii_case("auto")
    {
        return Some(RustOwnedNestedPrimitiveValue::Keyword("auto".to_string()));
    }

    if component_value_parse_as_non_negative_length_percentage(component_value) {
        return component_value_parse_as_nested_length_percentage(component_value, source);
    }

    None
}

pub(super) fn rust_owned_aspect_ratio_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    if component_values.is_empty() {
        return None;
    }

    let mut has_auto = false;
    let ratio_component_values = match component_values.as_slice() {
        [component_value] if component_value_is_ident(Some(component_value), "auto") => {
            return Some(RustOwnedStyleValueKind::AspectRatio(RustOwnedAspectRatio {
                has_auto: true,
                numerator: None,
                denominator: None,
            }));
        }
        [first, rest @ ..] if component_value_is_ident(Some(first), "auto") => {
            has_auto = true;
            rest
        }
        [rest @ .., last] if component_value_is_ident(Some(last), "auto") => {
            has_auto = true;
            rest
        }
        _ => component_values.as_slice(),
    };

    // https://www.w3.org/TR/css-sizing-4/#aspect-ratio
    // auto || <ratio>
    //
    // https://drafts.csswg.org/css-values-4/#ratios
    // <ratio> = <number [0,∞]> [ / <number [0,∞]> ]?
    let (numerator, denominator) = match ratio_component_values {
        [numerator] => (numerator, None),
        [numerator, slash, denominator] if component_value_is_delim(Some(slash), '/') => (numerator, Some(denominator)),
        _ => return None,
    };

    if !component_value_parse_as_non_negative_number(numerator) {
        return None;
    }
    let numerator = component_value_parse_as_nested_non_negative_number(numerator, filtered_input_string)?;

    let denominator = if let Some(denominator) = denominator {
        if !component_value_parse_as_non_negative_number(denominator) {
            return None;
        }
        Some(component_value_parse_as_nested_non_negative_number(
            denominator,
            filtered_input_string,
        )?)
    } else {
        None
    };

    Some(RustOwnedStyleValueKind::AspectRatio(RustOwnedAspectRatio {
        has_auto,
        numerator: Some(numerator),
        denominator,
    }))
}

pub(super) fn rust_owned_border_radius_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://drafts.csswg.org/css-borders-4/#typedef-border-radius
    // <border-radius> = <slash-separated-border-radius-syntax> | <legacy-border-radius-syntax>
    // <slash-separated-border-radius-syntax> = <length-percentage [0,∞]> [ / <length-percentage [0,∞]> ]?
    // <legacy-border-radius-syntax> = <length-percentage [0,∞]>{1,2}
    let (horizontal, vertical) = match component_values.as_slice() {
        [horizontal] => (horizontal, None),
        [horizontal, vertical] => (horizontal, Some(vertical)),
        [horizontal, slash, vertical] if component_value_is_delim(Some(slash), '/') => (horizontal, Some(vertical)),
        _ => return None,
    };

    if !component_value_parse_as_non_negative_length_percentage(horizontal) {
        return None;
    }
    let horizontal_radius = component_value_parse_as_nested_length_percentage(horizontal, filtered_input_string)?;

    let vertical_radii = if let Some(vertical) = vertical {
        if !component_value_parse_as_non_negative_length_percentage(vertical) {
            return None;
        }
        vec![component_value_parse_as_nested_length_percentage(
            vertical,
            filtered_input_string,
        )?]
    } else {
        vec![]
    };

    Some(RustOwnedStyleValueKind::BorderRadius(RustOwnedBorderRadius {
        horizontal_radii: vec![horizontal_radius],
        vertical_radii,
    }))
}

pub(super) fn rust_owned_one_to_four_sources<F>(filtered_input: &[u8], predicate: F) -> Option<Vec<String>>
where
    F: Fn(&ComponentValue) -> bool,
{
    let source = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let mut sources = vec![];

    for component_value in component_values {
        if is_whitespace_component_value(component_value) {
            continue;
        }

        if sources.len() == 4 || !predicate(component_value) {
            return None;
        }

        sources.push(serialize_component_values_for_reparsing(
            std::slice::from_ref(component_value),
            &source,
        )?);
    }

    if sources.is_empty() {
        return None;
    }

    Some(sources)
}

pub(super) fn rust_owned_one_to_four_values<T>(
    filtered_input: &[u8],
    mut parse_value: impl FnMut(&ComponentValue) -> Option<T>,
) -> Option<Vec<T>> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let mut values = vec![];

    for component_value in component_values {
        if is_whitespace_component_value(component_value) {
            continue;
        }

        if values.len() == 4 {
            return None;
        }

        values.push(parse_value(component_value)?);
    }

    if values.is_empty() {
        return None;
    }

    Some(values)
}

pub(super) fn serialize_consumed_component_values(
    parser: &ComponentValueParser,
    start: usize,
    source: &str,
) -> Option<String> {
    serialize_component_values_for_reparsing(strip_whitespace(&parser.component_values[start..parser.index]), source)
}

pub(super) fn border_shorthand_component_properties(
    property_id: PropertyId,
) -> Option<(PropertyId, PropertyId, PropertyId)> {
    match property_id {
        PropertyId::Border => Some((
            PropertyId::BorderWidth,
            PropertyId::BorderStyle,
            PropertyId::BorderColor,
        )),
        PropertyId::BorderBlock => Some((
            PropertyId::BorderBlockWidth,
            PropertyId::BorderBlockStyle,
            PropertyId::BorderBlockColor,
        )),
        PropertyId::BorderBlockEnd => Some((
            PropertyId::BorderBlockEndWidth,
            PropertyId::BorderBlockEndStyle,
            PropertyId::BorderBlockEndColor,
        )),
        PropertyId::BorderBlockStart => Some((
            PropertyId::BorderBlockStartWidth,
            PropertyId::BorderBlockStartStyle,
            PropertyId::BorderBlockStartColor,
        )),
        PropertyId::BorderInline => Some((
            PropertyId::BorderInlineWidth,
            PropertyId::BorderInlineStyle,
            PropertyId::BorderInlineColor,
        )),
        PropertyId::BorderInlineEnd => Some((
            PropertyId::BorderInlineEndWidth,
            PropertyId::BorderInlineEndStyle,
            PropertyId::BorderInlineEndColor,
        )),
        PropertyId::BorderInlineStart => Some((
            PropertyId::BorderInlineStartWidth,
            PropertyId::BorderInlineStartStyle,
            PropertyId::BorderInlineStartColor,
        )),
        _ => None,
    }
}

pub(super) fn rust_owned_component_shorthand_style_value_kind(
    property_id: PropertyId,
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<RustOwnedStyleValueKind> {
    let longhands = longhands_for_shorthand(property_id);
    if longhands.is_empty() {
        return None;
    }

    let source = filtered_input_to_string(filtered_input);
    let (mut input_parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = input_parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let mut remaining_longhands = longhands.to_vec();
    let mut items = Vec::new();

    while parser.has_next_component_value() {
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            break;
        }

        let start = parser.index;
        parser.index += 1;

        let component_source = serialize_consumed_component_values(&parser, start, &source)?;
        let matching_longhand_index = remaining_longhands.iter().position(|longhand| {
            let property_ids = [*longhand as u16];
            matches!(
                parse_rust_owned_style_value_for_property_with_options(
                    &property_ids,
                    component_source.as_bytes(),
                    primitive_value_options,
                ),
                RustOwnedStyleValueParseResult::Parsed(_)
            )
        })?;

        items.push(RustOwnedComponentShorthandItem {
            property_id: remaining_longhands.remove(matching_longhand_index),
            source: component_source,
        });
    }

    if items.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::ComponentShorthand(items))
}

pub(super) fn rust_owned_border_shorthand_style_value_kind(
    property_id: PropertyId,
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-backgrounds-3/#propdef-border
    // <line-width> || <line-style> || <color>
    let (_, _, color_property) = border_shorthand_component_properties(property_id)?;
    let component_property_ids = [color_property as u16];

    let source = filtered_input_to_string(filtered_input);
    let (mut input_parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = input_parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    let mut width = None;
    let mut style = None;
    let mut color = None;

    while parser.has_next_component_value() {
        parser.discard_whitespace();
        let start = parser.index;
        parser.index += 1;

        let component_source = serialize_consumed_component_values(&parser, start, &source)?;
        let component_value = &parser.component_values[start];
        if width.is_none()
            && let Some(value) = rust_owned_border_width_from_component_value(component_value, &source)
        {
            width = Some(value);
        } else if style.is_none()
            && let Some(value) = rust_owned_line_style_from_component_value(component_value)
        {
            style = Some(value);
        } else if color.is_none()
            && let Some(value) = rust_owned_color_from_component_value(component_value, &source)
            && matches!(
                parse_rust_owned_style_value_for_property(&component_property_ids, component_source.as_bytes()),
                RustOwnedStyleValueParseResult::Parsed(_)
            )
        {
            color = Some(value);
        } else {
            return None;
        }
    }

    if width.is_none() && style.is_none() && color.is_none() {
        return None;
    }

    Some(RustOwnedStyleValueKind::Border(RustOwnedBorder { width, style, color }))
}

pub(super) fn rust_owned_color_from_component_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedColor> {
    if let Some(color) = simple_color_from_component_value(component_value, false) {
        return Some(match color {
            ParsedSimpleColor::Rgba {
                red,
                green,
                blue,
                alpha,
                name,
            } => RustOwnedColor::Simple {
                kind: CssParsedColorKind::Rgba,
                red,
                green,
                blue,
                alpha,
                name: name.map(str::to_string),
            },
            ParsedSimpleColor::Keyword { name } => RustOwnedColor::Simple {
                kind: CssParsedColorKind::Keyword,
                red: 0,
                green: 0,
                blue: 0,
                alpha: 0,
                name: Some(name.to_string()),
            },
        });
    }

    let ComponentValue::Function(function) = component_value else {
        return None;
    };
    component_value_parse_as_color_value(component_value)
        .then(|| serialize_component_values_for_reparsing(std::slice::from_ref(component_value), source))
        .flatten()
        .map(|source| RustOwnedColor::Function {
            name: function.name.clone(),
            arguments: function.value.clone(),
            source,
        })
}

pub(super) fn rust_owned_paint_color_or_none_from_component_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<Option<RustOwnedColor>> {
    if let Some(color) = rust_owned_color_from_component_value(component_value, source) {
        return Some(Some(color));
    }

    // NOTE: <color> also accepts identifiers, so we do this identifier check last.
    if let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    }) = component_value
        && value.eq_ignore_ascii_case("none")
    {
        return Some(None);
    }

    None
}

pub(super) fn rust_owned_paint_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    // https://svgwg.org/svg2-draft/painting.html#SpecifyingPaint
    // `<paint> = none | <color> | <url> [none | <color>]? | context-fill | context-stroke`
    // FIXME: Accept `context-fill` and `context-stroke`.
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values)
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
        .collect::<Vec<_>>();

    if let [component_value] = component_values.as_slice()
        && let Some(color_or_none) =
            rust_owned_paint_color_or_none_from_component_value(component_value, filtered_input_string)
    {
        return Some(RustOwnedStyleValueKind::Paint(match color_or_none {
            Some(color) => RustOwnedPaint::Color(color),
            None => RustOwnedPaint::None,
        }));
    }

    let [url_component_value, fallback @ ..] = component_values.as_slice() else {
        return None;
    };
    let url = rust_owned_url_from_component_value(url_component_value, filtered_input_string)?;
    let fallback_color = match fallback {
        [] => None,
        [fallback_component_value] => {
            rust_owned_paint_color_or_none_from_component_value(fallback_component_value, filtered_input_string)?
        }
        _ => return None,
    };

    Some(RustOwnedStyleValueKind::Paint(RustOwnedPaint::Url {
        url,
        fallback_color,
    }))
}

pub(super) fn rust_owned_corner_shape_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-borders-4/#typedef-corner-shape-value
    // <corner-shape-value> = round | scoop | bevel | notch | square | squircle | <superellipse()>
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = strip_whitespace(&component_values)
    else {
        return rust_owned_superellipse_style_value_kind(strip_whitespace(&component_values), filtered_input_string);
    };

    if first_is_one_of(
        value.as_str(),
        &["round", "scoop", "bevel", "notch", "square", "squircle"],
    ) {
        return Some(RustOwnedStyleValueKind::CornerShape(RustOwnedCornerShape {
            value: RustOwnedNestedPrimitiveValue::Keyword(value.to_string()),
        }));
    }

    None
}

pub(super) fn rust_owned_superellipse_style_value_kind(
    component_values: &[ComponentValue],
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    let [ComponentValue::Function(function)] = component_values else {
        return None;
    };
    if !function.name.eq_ignore_ascii_case("superellipse") {
        return None;
    }

    // https://drafts.csswg.org/css-borders-4/#funcdef-superellipse
    // superellipse() = superellipse(<number> | infinity | -infinity)
    let parameter_component_values = strip_whitespace(&function.value)
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
        .collect::<Vec<_>>();
    let [parameter_component_value] = parameter_component_values.as_slice() else {
        return None;
    };

    let parameter = match parameter_component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("infinity") => RustOwnedNestedPrimitiveValue::Number(f64::INFINITY),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("-infinity") => RustOwnedNestedPrimitiveValue::Number(f64::NEG_INFINITY),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => RustOwnedNestedPrimitiveValue::Number(number.value()),
        ComponentValue::Function(_) => {
            let source = serialize_component_values_for_reparsing(
                std::slice::from_ref(*parameter_component_value),
                filtered_input_string,
            )?;
            if !component_values_parse_as_property_value_type(PropertyValueType::Number, source.as_bytes()) {
                return None;
            }
            RustOwnedNestedPrimitiveValue::Source(source)
        }
        _ => return None,
    };

    Some(RustOwnedStyleValueKind::CornerShape(RustOwnedCornerShape {
        value: parameter,
    }))
}

pub(super) fn rust_owned_border_width_from_component_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if let Some(line_width) = rust_owned_line_width_keyword_from_component_value(component_value) {
        return Some(RustOwnedNestedPrimitiveValue::Keyword(line_width.to_string()));
    }

    component_value_parse_as_nested_non_negative_length(component_value, source)
}

pub(super) fn rust_owned_line_width_keyword_from_component_value(
    component_value: &ComponentValue,
) -> Option<&'static str> {
    let ident = component_value_ident(component_value)?;
    if ident.eq_ignore_ascii_case("thin") {
        return Some("thin");
    }
    if ident.eq_ignore_ascii_case("medium") {
        return Some("medium");
    }
    if ident.eq_ignore_ascii_case("thick") {
        return Some("thick");
    }
    None
}

pub(super) fn rust_owned_line_style_from_component_value(
    component_value: &ComponentValue,
) -> Option<RustOwnedLineStyle> {
    let ident = component_value_ident(component_value)?;
    if ident.eq_ignore_ascii_case("none") {
        return Some(RustOwnedLineStyle::None);
    }
    if ident.eq_ignore_ascii_case("hidden") {
        return Some(RustOwnedLineStyle::Hidden);
    }
    if ident.eq_ignore_ascii_case("dotted") {
        return Some(RustOwnedLineStyle::Dotted);
    }
    if ident.eq_ignore_ascii_case("dashed") {
        return Some(RustOwnedLineStyle::Dashed);
    }
    if ident.eq_ignore_ascii_case("solid") {
        return Some(RustOwnedLineStyle::Solid);
    }
    if ident.eq_ignore_ascii_case("double") {
        return Some(RustOwnedLineStyle::Double);
    }
    if ident.eq_ignore_ascii_case("groove") {
        return Some(RustOwnedLineStyle::Groove);
    }
    if ident.eq_ignore_ascii_case("ridge") {
        return Some(RustOwnedLineStyle::Ridge);
    }
    if ident.eq_ignore_ascii_case("inset") {
        return Some(RustOwnedLineStyle::Inset);
    }
    if ident.eq_ignore_ascii_case("outset") {
        return Some(RustOwnedLineStyle::Outset);
    }
    None
}

pub(super) fn component_value_parse_as_border_image_source(component_value: &ComponentValue) -> bool {
    // https://drafts.csswg.org/css-backgrounds-3/#border-image-source
    // <'border-image-source'> = none | <image>
    if component_value_is_ident(Some(component_value), "none") {
        return true;
    }

    component_value_parse_as_image_url(component_value)
        || component_value_parse_as_image_gradient(component_value)
        || matches!(component_value, ComponentValue::Function(function) if component_value_parse_as_image_set_function(function))
}

pub(super) fn consume_border_image_source(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedBorderImageSource> {
    parser.discard_whitespace();
    let start = parser.index;
    let component_value = parser.next_component_value()?;
    if !component_value_parse_as_border_image_source(component_value) {
        return None;
    }

    let is_none = component_value_is_ident(Some(component_value), "none");
    parser.index += 1;
    if is_none {
        return Some(RustOwnedBorderImageSource::None);
    }

    let source = serialize_consumed_component_values(parser, start, source)?;
    match rust_owned_image_style_value_kind(source.as_bytes(), &source)? {
        RustOwnedStyleValueKind::Image(image) => Some(RustOwnedBorderImageSource::Image(image)),
        RustOwnedStyleValueKind::ImageSet(_) => Some(RustOwnedBorderImageSource::Image(RustOwnedImage {
            kind: RustOwnedImageKind::ImageSet,
            url: None,
            gradient: None,
            source,
        })),
        _ => None,
    }
}

pub(super) fn component_value_parse_as_border_image_outset(component_value: &ComponentValue) -> bool {
    component_value_parse_as_non_negative_number(component_value)
        || component_value_parse_as_non_negative_length(component_value)
}

pub(super) fn rust_owned_border_image_outset_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-backgrounds-3/#border-image-outset
    // <'border-image-outset'> = [ <length [0,∞]> | <number [0,∞]> ]{1,4}
    let values = rust_owned_one_to_four_border_image_outsets(filtered_input)?;
    Some(RustOwnedStyleValueKind::BorderImageOutset(
        RustOwnedBorderImageOutsetList { values },
    ))
}

pub(super) fn rust_owned_one_to_four_border_image_outsets(
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedBorderImageOutset>> {
    let source = filtered_input_to_string(filtered_input);
    rust_owned_one_to_four_values(filtered_input, |component_value| {
        if !component_value_parse_as_border_image_outset(component_value) {
            return None;
        }
        rust_owned_border_image_outset_from_component_value(component_value, &source)
    })
}

pub(super) fn rust_owned_border_image_outset_from_component_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedBorderImageOutset> {
    if component_value_parse_as_non_negative_number(component_value) {
        return Some(RustOwnedBorderImageOutset {
            value: component_value_parse_as_nested_non_negative_number(component_value, source)?,
        });
    }

    Some(RustOwnedBorderImageOutset {
        value: component_value_parse_as_nested_non_negative_length(component_value, source)?,
    })
}

pub(super) fn component_value_parse_as_border_image_width(component_value: &ComponentValue) -> bool {
    component_value_is_ident(Some(component_value), "auto")
        || component_value_parse_as_non_negative_length_percentage(component_value)
        || component_value_parse_as_non_negative_number(component_value)
}

pub(super) fn consume_border_image_width_values(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<Vec<RustOwnedNestedPrimitiveValue>> {
    parser.discard_whitespace();
    let mut values = Vec::new();

    while values.len() < 4 {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };
        if !component_value_parse_as_border_image_width(component_value) {
            break;
        }

        values.push(rust_owned_border_image_width_from_component_value(
            component_value,
            source,
        )?);
        parser.index += 1;
    }

    if values.is_empty() {
        return None;
    }

    Some(values)
}

pub(super) fn consume_border_image_outset_values(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<Vec<RustOwnedBorderImageOutset>> {
    parser.discard_whitespace();
    let mut values = Vec::new();

    while values.len() < 4 {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };
        if !component_value_parse_as_border_image_outset(component_value) {
            break;
        }

        values.push(rust_owned_border_image_outset_from_component_value(
            component_value,
            source,
        )?);
        parser.index += 1;
    }

    if values.is_empty() {
        return None;
    }

    Some(values)
}

pub(super) fn rust_owned_border_image_width_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-backgrounds-3/#border-image-width
    // <'border-image-width'> = [ <length-percentage [0,∞]> | <number [0,∞]> | auto ]{1,4}
    let values = rust_owned_one_to_four_border_image_widths(filtered_input)?;
    Some(RustOwnedStyleValueKind::BorderImageWidth(
        RustOwnedBorderImageWidthList { values },
    ))
}

pub(super) fn rust_owned_one_to_four_border_image_widths(
    filtered_input: &[u8],
) -> Option<Vec<RustOwnedNestedPrimitiveValue>> {
    let source = filtered_input_to_string(filtered_input);
    rust_owned_one_to_four_values(filtered_input, |component_value| {
        if !component_value_parse_as_border_image_width(component_value) {
            return None;
        }
        rust_owned_border_image_width_from_component_value(component_value, &source)
    })
}

pub(super) fn rust_owned_border_image_width_from_component_value(
    component_value: &ComponentValue,
    source: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    if component_value_is_ident(Some(component_value), "auto") {
        return Some(auto_keyword());
    }

    component_value_parse_as_nested_non_negative_number_length_percentage(component_value, source)
}

pub(super) fn component_value_parse_as_border_image_repeat(component_value: &ComponentValue) -> bool {
    component_value_is_ident(Some(component_value), "stretch")
        || component_value_is_ident(Some(component_value), "repeat")
        || component_value_is_ident(Some(component_value), "round")
        || component_value_is_ident(Some(component_value), "space")
}

pub(super) fn rust_owned_border_image_repeat_from_component_value(
    component_value: &ComponentValue,
) -> Option<RustOwnedBorderImageRepeat> {
    let ident = component_value_ident(component_value)?;
    if ident.eq_ignore_ascii_case("stretch") {
        return Some(RustOwnedBorderImageRepeat::Stretch);
    }
    if ident.eq_ignore_ascii_case("repeat") {
        return Some(RustOwnedBorderImageRepeat::Repeat);
    }
    if ident.eq_ignore_ascii_case("round") {
        return Some(RustOwnedBorderImageRepeat::Round);
    }
    if ident.eq_ignore_ascii_case("space") {
        return Some(RustOwnedBorderImageRepeat::Space);
    }
    None
}

pub(super) fn expand_one_to_four_values<T: Clone>(values: &[T]) -> Vec<T> {
    match values {
        [top] => vec![top.clone(), top.clone(), top.clone(), top.clone()],
        [top, right] => vec![top.clone(), right.clone(), top.clone(), right.clone()],
        [top, right, bottom] => vec![top.clone(), right.clone(), bottom.clone(), right.clone()],
        [top, right, bottom, left] => vec![top.clone(), right.clone(), bottom.clone(), left.clone()],
        _ => vec![],
    }
}

pub(super) fn consume_border_image_repeat_values(
    parser: &mut ComponentValueParser,
) -> Option<Vec<RustOwnedBorderImageRepeat>> {
    parser.discard_whitespace();
    let mut values = Vec::new();

    let component_value = parser.next_component_value()?;
    if !component_value_parse_as_border_image_repeat(component_value) {
        return None;
    }
    values.push(rust_owned_border_image_repeat_from_component_value(component_value)?);
    parser.index += 1;

    parser.discard_whitespace();
    if let Some(component_value) = parser.next_component_value()
        && component_value_parse_as_border_image_repeat(component_value)
    {
        values.push(rust_owned_border_image_repeat_from_component_value(component_value)?);
        parser.index += 1;
    }

    Some(values)
}

pub(super) fn rust_owned_border_image_repeat_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let mut values = vec![];

    for component_value in component_values {
        if is_whitespace_component_value(component_value) {
            continue;
        }

        if values.len() == 2 || !component_value_parse_as_border_image_repeat(component_value) {
            return None;
        }

        values.push(rust_owned_border_image_repeat_from_component_value(component_value)?);
    }

    if values.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::BorderImageRepeat(
        RustOwnedBorderImageRepeatList { values },
    ))
}

pub(super) fn consume_border_image_slice(
    parser: &mut ComponentValueParser,
    source: &str,
) -> Option<RustOwnedBorderImageSlice> {
    parser.discard_whitespace();
    let mut fill = false;
    let mut values = Vec::new();

    loop {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };

        if component_value_is_ident(Some(component_value), "fill") {
            if fill {
                break;
            }

            fill = true;
            parser.index += 1;
            if !values.is_empty() {
                break;
            }
            continue;
        }

        if values.len() < 4 && component_value_parse_as_non_negative_number_percentage(component_value) {
            values.push(component_value_parse_as_nested_non_negative_number_percentage(
                component_value,
                source,
            )?);
            parser.index += 1;
            continue;
        }

        break;
    }

    if values.is_empty() {
        return None;
    }

    Some(RustOwnedBorderImageSlice {
        values: expand_one_to_four_values(&values),
        fill,
    })
}

pub(super) fn rust_owned_border_image_shorthand_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-backgrounds-3/#border-image
    // <'border-image'> = <'border-image-source'> || <'border-image-slice'> [ / <'border-image-width'> | / <'border-image-width'>? / <'border-image-outset'> ]? || <'border-image-repeat'>
    let source = filtered_input_to_string(filtered_input);
    let (mut input_parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = input_parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    let mut border_image_source = None;
    let mut slice = None;
    let mut width = None;
    let mut outset = None;
    let mut repeat = None;
    let mut parsed_anything = false;

    while parser.has_next_component_value() {
        if border_image_source.is_none()
            && let Some(value) = consume_border_image_source(&mut parser, &source)
        {
            border_image_source = Some(value);
            parsed_anything = true;
            continue;
        }

        if repeat.is_none()
            && let Some(value) = consume_border_image_repeat_values(&mut parser)
        {
            repeat = Some(value);
            parsed_anything = true;
            continue;
        }

        if slice.is_none()
            && let Some(value) = consume_border_image_slice(&mut parser, &source)
        {
            slice = Some(value);
            parsed_anything = true;
            parser.discard_whitespace();

            if parser.consume_a_delim('/') {
                parser.discard_whitespace();
                if !matches!(
                    parser.next_component_value(),
                    Some(ComponentValue::PreservedToken(Token {
                        token_type: TokenType::Delim { value },
                        ..
                    })) if *value == '/' as u32
                ) {
                    width = Some(consume_border_image_width_values(&mut parser, &source)?);
                    parser.discard_whitespace();
                }

                if parser.consume_a_delim('/') {
                    outset = Some(consume_border_image_outset_values(&mut parser, &source)?);
                }
            }
            continue;
        }

        return None;
    }

    parsed_anything.then_some(RustOwnedStyleValueKind::BorderImage(RustOwnedBorderImage {
        source: border_image_source,
        slice,
        width,
        outset,
        repeat,
    }))
}

pub(super) fn rust_owned_border_image_slice_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let source = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let mut fill = false;
    let mut values = vec![];
    let mut index = 0;

    // https://drafts.csswg.org/css-backgrounds-3/#border-image-slice
    // <'border-image-slice'> = [<number [0,∞]> | <percentage [0,∞]>]{1,4} && fill?
    while index < component_values.len() && is_whitespace_component_value(&component_values[index]) {
        index += 1;
    }
    if component_value_is_ident(component_values.get(index), "fill") {
        fill = true;
        index += 1;
    }

    while index < component_values.len() {
        let component_value = &component_values[index];
        if is_whitespace_component_value(component_value) {
            index += 1;
            continue;
        }

        if values.len() == 4 || !component_value_parse_as_non_negative_number_percentage(component_value) {
            break;
        }

        values.push(component_value_parse_as_nested_non_negative_number_percentage(
            component_value,
            &source,
        )?);
        index += 1;
    }

    while index < component_values.len() && is_whitespace_component_value(&component_values[index]) {
        index += 1;
    }
    if component_value_is_ident(component_values.get(index), "fill") {
        if fill {
            return None;
        }
        fill = true;
        index += 1;
    }
    while index < component_values.len() && is_whitespace_component_value(&component_values[index]) {
        index += 1;
    }
    if index != component_values.len() {
        return None;
    }

    let values = expand_one_to_four_values(&values);

    Some(RustOwnedStyleValueKind::BorderImageSlice(RustOwnedBorderImageSlice {
        values,
        fill,
    }))
}

pub(super) fn rust_owned_border_radius_shorthand_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
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

    // https://drafts.csswg.org/css-backgrounds-3/#border-radius
    // <'border-radius'> = <length-percentage [0,∞]>{1,4} [ / <length-percentage [0,∞]>{1,4} ]?
    let horizontal_radii = rust_owned_border_radius_shorthand_side_values(horizontal_radii, filtered_input_string)?;
    let vertical_radii = if let Some(vertical_radii) = vertical_radii {
        rust_owned_border_radius_shorthand_side_values(vertical_radii, filtered_input_string)?
    } else {
        vec![]
    };

    Some(RustOwnedStyleValueKind::BorderRadius(RustOwnedBorderRadius {
        horizontal_radii,
        vertical_radii,
    }))
}

pub(super) fn rust_owned_columns_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    let slash_positions = component_values
        .iter()
        .enumerate()
        .filter_map(|(index, component_value)| component_value_is_delim(Some(component_value), '/').then_some(index))
        .collect::<Vec<_>>();

    if slash_positions.len() > 1 {
        return None;
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
        return None;
    }

    let column_height = if let Some(column_height) = column_height {
        if column_height.len() != 1 {
            return None;
        }
        if component_value_is_ident(column_height.first(), "auto") {
            Some(auto_keyword())
        } else if parse_single_column_component_value(PropertyId::ColumnHeight, column_height, filtered_input_string) {
            Some(component_value_parse_as_nested_length(
                &column_height[0],
                filtered_input_string,
            )?)
        } else {
            return None;
        }
    } else {
        None
    };

    let mut found_autos = 0_u8;
    let mut column_count = None;
    let mut column_width = None;
    for component_value in columns {
        if component_value_is_ident(Some(component_value), "auto") {
            found_autos += 1;
            continue;
        }

        let component_values = std::slice::from_ref(component_value);
        if column_width.is_none()
            && parse_single_column_component_value(PropertyId::ColumnWidth, component_values, filtered_input_string)
        {
            column_width = Some(component_value_parse_as_nested_length(
                component_value,
                filtered_input_string,
            )?);
            continue;
        }

        if column_count.is_none()
            && parse_single_column_component_value(PropertyId::ColumnCount, component_values, filtered_input_string)
        {
            column_count = Some(component_value_parse_as_nested_integer(
                component_value,
                filtered_input_string,
            )?);
            continue;
        }

        return None;
    }

    if found_autos > 2 {
        return None;
    }
    if found_autos > 0 {
        if column_count.is_none() {
            column_count = Some(auto_keyword());
        }
        if column_width.is_none() {
            column_width = Some(auto_keyword());
        }
    }

    Some(RustOwnedStyleValueKind::Columns(RustOwnedColumns {
        column_count,
        column_width,
        column_height,
    }))
}

pub(super) fn rust_owned_content_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    parse_rust_owned_content_value(filtered_input).map(RustOwnedStyleValueKind::Content)
}

pub(super) fn rust_owned_cursor_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    parse_rust_owned_cursor_value(filtered_input).map(RustOwnedStyleValueKind::Cursor)
}

pub(super) fn rust_owned_overflow_clip_margin_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    if !parse_overflow_clip_margin_value(filtered_input) {
        return None;
    }

    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    let [length] = component_values.as_slice() else {
        return None;
    };

    Some(RustOwnedStyleValueKind::OverflowClipMargin(
        RustOwnedOverflowClipMargin {
            length: component_value_parse_as_nested_length(length, filtered_input_string)?,
        },
    ))
}

pub(super) fn rust_owned_overflow_clip_margin_shorthand_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    if !parse_overflow_clip_margin_value(filtered_input) {
        return None;
    }

    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    let [length] = component_values.as_slice() else {
        return None;
    };

    Some(RustOwnedStyleValueKind::OverflowClipMargin(
        RustOwnedOverflowClipMargin {
            length: component_value_parse_as_nested_length(length, filtered_input_string)?,
        },
    ))
}

pub(super) fn rust_owned_shadow_style_value_kind(
    property_id: PropertyId,
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    Some(RustOwnedStyleValueKind::Shadow(parse_rust_owned_shadow_value(
        property_id,
        filtered_input,
    )?))
}

pub(super) fn rust_owned_shape_outside_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    parse_rust_owned_shape_outside_value(filtered_input).map(RustOwnedStyleValueKind::ShapeOutside)
}

pub(super) fn rust_owned_text_decoration_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);
    if component_values.is_empty() {
        return None;
    }

    let mut line_component_values = Vec::new();
    let mut thickness = None;
    let mut style = None;
    let mut color = None;
    let mut saw_non_line_after_line = false;

    for component_value in &component_values {
        if component_value_is_text_decoration_line(component_value) {
            if saw_non_line_after_line {
                return None;
            }
            line_component_values.push(component_value.clone());
            continue;
        }

        if !line_component_values.is_empty() {
            saw_non_line_after_line = true;
        }

        if style.is_none() && component_value_is_text_decoration_style(component_value) {
            style = Some(rust_owned_text_decoration_style_from_component_value(component_value)?);
            continue;
        }

        if color.is_none()
            && let Some(value) = rust_owned_color_from_component_value(component_value, filtered_input_string)
        {
            color = Some(value);
            continue;
        }

        if thickness.is_none() && component_value_parse_as_text_decoration_thickness(component_value) {
            thickness = Some(rust_owned_text_decoration_thickness_from_component_value(
                component_value,
                filtered_input_string,
            )?);
            continue;
        }

        return None;
    }

    // https://drafts.csswg.org/css-text-decor-4/#text-decoration-property
    // <'text-decoration-line'> || <'text-decoration-thickness'> || <'text-decoration-style'> || <'text-decoration-color'>
    let line = if line_component_values.is_empty() {
        None
    } else {
        Some(RustOwnedTextDecorationLine {
            bits: component_values_parse_as_text_decoration_line(&line_component_values)?,
        })
    };

    Some(RustOwnedStyleValueKind::TextDecoration(RustOwnedTextDecoration {
        line,
        thickness,
        style,
        color,
    }))
}

pub(super) fn rust_owned_text_decoration_line_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let bits = parse_text_decoration_line_bits(filtered_input)?;
    Some(RustOwnedStyleValueKind::TextDecorationLine(
        RustOwnedTextDecorationLine { bits },
    ))
}

pub(super) fn rust_owned_scrollbar_color_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let source = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-scrollbars/#propdef-scrollbar-color
    // auto | <color>{2}
    if let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
        && value.eq_ignore_ascii_case("auto")
    {
        return Some(RustOwnedStyleValueKind::ScrollbarColor(RustOwnedScrollbarColor::Auto));
    }

    let colors: Vec<_> = component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
        .collect();
    let [thumb_color, track_color] = colors.as_slice() else {
        return None;
    };

    let thumb_color = rust_owned_color_from_component_value(thumb_color, &source)?;
    let track_color = rust_owned_color_from_component_value(track_color, &source)?;

    Some(RustOwnedStyleValueKind::ScrollbarColor(
        RustOwnedScrollbarColor::Colors {
            thumb_color,
            track_color,
        },
    ))
}

pub(super) fn rust_owned_scrollbar_gutter_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_scrollbar_gutter_value(filtered_input);
    if value == CssScrollbarGutterValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::ScrollbarGutter(RustOwnedScrollbarGutter {
        value,
    }))
}

pub(super) fn rust_owned_stroke_dasharray_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);

    // https://svgwg.org/svg2-draft/painting.html#StrokeDashing
    // Value: none | <dasharray>
    parser.discard_whitespace();
    if parser.consume_ident_matching("none") {
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }
        return Some(RustOwnedStyleValueKind::StrokeDasharray(RustOwnedStrokeDasharray::None));
    }

    // https://svgwg.org/svg2-draft/painting.html#DataTypeDasharray
    // <dasharray> = [ [ <length-percentage> | <number> ]+ ]#
    let mut values = Vec::new();
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
            return None;
        }

        values.push(component_value_parse_as_nested_dasharray_value(
            component_value,
            filtered_input_string,
        )?);
        parser.index += 1;

        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            break;
        }
        if parser.consume_a_comma() {
            parser.discard_whitespace();
            if !parser.has_next_component_value() {
                return None;
            }
        }
    }

    if values.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::StrokeDasharray(
        RustOwnedStrokeDasharray::Values(values),
    ))
}

pub(super) fn rust_owned_border_spacing_style_value_kind(
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let component_values = component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
        .collect::<Vec<_>>();

    // https://drafts.csswg.org/css-tables-3/#border-spacing-property
    // Value: <length>{1,2}
    if component_values.is_empty() || component_values.len() > 2 {
        return None;
    }

    let mut values = Vec::with_capacity(component_values.len());
    for component_value in component_values {
        if let Some(value) = component_value_parse_as_nested_non_negative_length(component_value, filtered_input_string)
        {
            values.push(value);
            continue;
        }

        if parse_length_value_prefix(component_value, primitive_value_options) != CssPrimitiveValueKind::Length {
            return None;
        }
        let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) = component_value
        else {
            return None;
        };
        if number.value() < 0.0
            || !(primitive_value_options.allow_quirky_length || primitive_value_options.allow_svg_unitless_length)
        {
            return None;
        }
        values.push(RustOwnedNestedPrimitiveValue::Source(
            serialize_component_values_for_reparsing(std::slice::from_ref(component_value), filtered_input_string)?,
        ));
    }

    Some(RustOwnedStyleValueKind::BorderSpacing(RustOwnedBorderSpacing {
        values,
    }))
}

pub(super) fn rust_owned_text_underline_position_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let value = parse_text_underline_position_value(filtered_input);
    if value.horizontal == CssTextUnderlinePositionHorizontal::Invalid
        || value.vertical == CssTextUnderlinePositionVertical::Invalid
    {
        return None;
    }

    Some(RustOwnedStyleValueKind::TextUnderlinePosition(
        RustOwnedTextUnderlinePosition { value },
    ))
}

pub(super) fn rust_owned_timeline_name_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let mut names = Vec::new();
    let kind = parse_timeline_name_value(filtered_input, |kind, name| {
        names.push(RustOwnedTimelineNameItem {
            kind,
            name: name.to_string(),
        });
    });
    if kind == CssTimelineNameValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::TimelineName(RustOwnedTimelineName {
        kind,
        names,
    }))
}

pub(super) fn rust_owned_scroll_timeline_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let mut names = Vec::new();
    let mut axes = Vec::new();

    // https://drafts.csswg.org/scroll-animations-1/#scroll-timeline-shorthand
    // <'scroll-timeline'> = [ <'scroll-timeline-name'> <'scroll-timeline-axis'>? ]#
    loop {
        parser.discard_whitespace();
        names.push(consume_scroll_timeline_name_item(&mut parser)?);
        parser.discard_whitespace();

        if parser.consume_a_comma() {
            axes.push(CssScrollFunctionAxisKind::Block);
            parser.discard_whitespace();
            if !parser.has_next_component_value() {
                return None;
            }
            continue;
        }

        if !parser.has_next_component_value() {
            axes.push(CssScrollFunctionAxisKind::Block);
            break;
        }

        let axis = parser.consume_an_ident()?;
        axes.push(scroll_function_axis_from_string(&axis)?);
        parser.discard_whitespace();

        if parser.consume_a_comma() {
            parser.discard_whitespace();
            if !parser.has_next_component_value() {
                return None;
            }
            continue;
        }

        if parser.has_next_component_value() {
            return None;
        }
        break;
    }

    if names.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::ScrollTimeline(RustOwnedScrollTimeline {
        names,
        axes,
    }))
}

pub(super) fn consume_scroll_timeline_name_item(
    parser: &mut ComponentValueParser,
) -> Option<RustOwnedTimelineNameItem> {
    // https://drafts.csswg.org/scroll-animations-1/#scroll-timeline-name
    // <'scroll-timeline-name'> = [ none | <dashed-ident> ]#
    if parser.consume_ident_matching("none") {
        return Some(RustOwnedTimelineNameItem {
            kind: CssTimelineNameItemKind::None,
            name: String::new(),
        });
    }

    let name = match parser.next_component_value()? {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.starts_with("--") && is_valid_custom_ident(value, &[]) => value.clone(),
        _ => return None,
    };
    parser.index += 1;

    Some(RustOwnedTimelineNameItem {
        kind: CssTimelineNameItemKind::DashedIdent,
        name,
    })
}

pub(super) fn rust_owned_view_timeline_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let filtered_input_string = filtered_input_to_string(filtered_input);
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    let mut names = Vec::new();
    let mut axes = Vec::new();
    let mut insets = Vec::new();

    // https://drafts.csswg.org/scroll-animations-1/#view-timeline-shorthand
    // <'view-timeline'> = [ <'view-timeline-name'> [ <'view-timeline-axis'> || <'view-timeline-inset'> ]? ]#
    loop {
        parser.discard_whitespace();
        names.push(consume_view_timeline_name_item(&mut parser)?);
        parser.discard_whitespace();

        let mut axis = None;
        let mut inset = None;
        while parser.has_next_component_value() && !component_value_is_comma(parser.next_component_value()) {
            if axis.is_none()
                && let Some(parsed_axis) = parse_view_function_axis(&mut parser)
            {
                axis = Some(parsed_axis);
                continue;
            }

            if inset.is_none()
                && let Some(parsed_inset) = parse_view_timeline_inset_prefix(&mut parser, Some(&filtered_input_string))
            {
                inset = Some(parsed_inset.values);
                continue;
            }

            return None;
        }

        axes.push(axis.unwrap_or(CssScrollFunctionAxisKind::Block));
        insets.push(inset.unwrap_or_else(|| vec![auto_keyword()]));

        if parser.consume_a_comma() {
            parser.discard_whitespace();
            if !parser.has_next_component_value() {
                return None;
            }
            continue;
        }

        if parser.has_next_component_value() {
            return None;
        }
        break;
    }

    if names.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::ViewTimeline(RustOwnedViewTimeline {
        names,
        axes,
        insets,
    }))
}

pub(super) fn consume_view_timeline_name_item(parser: &mut ComponentValueParser) -> Option<RustOwnedTimelineNameItem> {
    // https://drafts.csswg.org/scroll-animations-1/#view-timeline-name
    // <'view-timeline-name'> = [ none | <dashed-ident> ]#
    consume_scroll_timeline_name_item(parser)
}

pub(super) fn rust_owned_timeline_scope_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let mut names = Vec::new();
    let kind = parse_timeline_scope_value(filtered_input, |name| names.push(name.to_string()));
    if kind == CssTimelineScopeValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::TimelineScope(RustOwnedTimelineScope {
        kind,
        names,
    }))
}

pub(super) fn rust_owned_text_wrap_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_text_wrap_value(filtered_input);
    if value.kind == CssTextWrapValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::TextWrap(RustOwnedTextWrap { value }))
}

pub(super) fn rust_owned_text_wrap_mode_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_text_wrap_mode_value(filtered_input);
    if value == CssTextWrapModeValue::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::TextWrapMode(RustOwnedTextWrapMode { value }))
}

pub(super) fn rust_owned_text_wrap_style_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_text_wrap_style_value(filtered_input);
    if value == CssTextWrapStyleValue::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::TextWrapStyle(RustOwnedTextWrapStyle { value }))
}

pub(super) fn rust_owned_text_indent_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let filtered_input_string = String::from_utf8_lossy(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    if component_values.is_empty() {
        return None;
    }

    // https://drafts.csswg.org/css-text-3/#propdef-text-indent
    // <length-percentage> && hanging? && each-line?
    let mut length_percentage = None;
    let mut has_hanging = false;
    let mut has_each_line = false;

    for component_value in component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
    {
        if length_percentage.is_none()
            && let Some(value) =
                component_value_parse_as_nested_length_percentage(component_value, &filtered_input_string)
        {
            length_percentage = Some(value);
            continue;
        }

        let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) = component_value
        else {
            return None;
        };

        if !has_hanging && value.eq_ignore_ascii_case("hanging") {
            has_hanging = true;
        } else if !has_each_line && value.eq_ignore_ascii_case("each-line") {
            has_each_line = true;
        } else {
            return None;
        }
    }

    Some(RustOwnedStyleValueKind::TextIndent(RustOwnedTextIndent {
        length_percentage: length_percentage?,
        has_hanging,
        has_each_line,
    }))
}

pub(super) fn rust_owned_touch_action_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_touch_action_value(filtered_input);
    if value.kind == CssTouchActionValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::TouchAction(RustOwnedTouchAction { value }))
}

pub(super) fn rust_owned_transform_origin_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = remove_whitespace_component_values(&component_values);

    // https://www.w3.org/TR/css-transforms-1/#propdef-transform-origin
    // transform-origin =
    //     [ left | center | right | top | bottom | <length-percentage> ] |
    //     [ left | center | right | <length-percentage> ]
    //     [ top | center | bottom | <length-percentage> ] <length>? |
    //     [[ center | left | right ] && [ center | top | bottom ]] <length>?
    let [first_value] = component_values.as_slice() else {
        return rust_owned_transform_origin_two_or_three_value_kind(&component_values, filtered_input_string)
            .map(RustOwnedStyleValueKind::TransformOrigin);
    };

    let first_value = transform_origin_component(first_value, filtered_input_string)?;

    let value = match first_value.axis {
        Some(TransformOriginAxis::Y) => RustOwnedTransformOrigin {
            x: RustOwnedNestedPrimitiveValue::Keyword("center".to_string()),
            y: first_value.value,
            z: zero_pixel_length(),
        },
        Some(TransformOriginAxis::X) | None => RustOwnedTransformOrigin {
            x: first_value.value,
            y: RustOwnedNestedPrimitiveValue::Keyword("center".to_string()),
            z: zero_pixel_length(),
        },
    };

    Some(RustOwnedStyleValueKind::TransformOrigin(value))
}

pub(super) fn rust_owned_transform_origin_two_or_three_value_kind(
    component_values: &[ComponentValue],
    filtered_input_string: &str,
) -> Option<RustOwnedTransformOrigin> {
    let (first_value, second_value, z) = match component_values {
        [first_value, second_value] => (first_value, second_value, zero_pixel_length()),
        [first_value, second_value, third_value] => (
            first_value,
            second_value,
            component_value_parse_as_nested_length(third_value, filtered_input_string)?,
        ),
        _ => return None,
    };

    let first_value = transform_origin_component(first_value, filtered_input_string)?;
    let second_value = transform_origin_component(second_value, filtered_input_string)?;

    if (first_value.is_offset && second_value.axis == Some(TransformOriginAxis::X))
        || (second_value.is_offset && first_value.axis == Some(TransformOriginAxis::Y))
    {
        return None;
    }

    let mut x = if first_value.axis == Some(TransformOriginAxis::X) {
        Some(first_value.value.clone())
    } else {
        None
    };
    let mut y = if first_value.axis == Some(TransformOriginAxis::Y) {
        Some(first_value.value.clone())
    } else {
        None
    };

    match second_value.axis {
        Some(TransformOriginAxis::X) => {
            if x.is_some() {
                return None;
            }
            x = Some(second_value.value.clone());
            y = Some(first_value.value.clone());
        }
        Some(TransformOriginAxis::Y) => {
            if y.is_some() {
                return None;
            }
            y = Some(second_value.value.clone());
            x = Some(first_value.value.clone());
        }
        None => {
            if x.is_some() {
                y = Some(second_value.value.clone());
            } else {
                x = Some(second_value.value.clone());
            }
        }
    }

    // If two or more values are defined and either no value is a keyword, or the only used keyword is center,
    // then the first value represents the horizontal position (or offset) and the second represents the vertical position (or offset).
    // A third value always represents the Z position (or offset) and must be of type <length>.
    if first_value.axis.is_none() && second_value.axis.is_none() {
        x = Some(first_value.value);
        y = Some(second_value.value);
    }

    Some(RustOwnedTransformOrigin { x: x?, y: y?, z })
}

pub(super) fn rust_owned_transform_longhand_style_value_kind(
    property_id: PropertyId,
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let value = match property_id {
        PropertyId::Rotate => parse_rust_owned_rotate_value(filtered_input),
        PropertyId::Scale => parse_rust_owned_scale_value(filtered_input),
        PropertyId::Translate => parse_rust_owned_translate_value(filtered_input),
        _ => unreachable!(),
    };
    Some(RustOwnedStyleValueKind::TransformLonghand(value?))
}

pub(super) fn rust_owned_transition_behavior_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let mut behaviors = Vec::new();
    let kind = parse_transition_behavior_value(filtered_input, |behavior| behaviors.push(behavior));
    if kind == CssTransitionBehaviorValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::TransitionBehavior(
        RustOwnedTransitionBehavior { kind, behaviors },
    ))
}

pub(super) fn rust_owned_transition_property_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let mut properties = Vec::new();
    let kind = parse_transition_property_value(filtered_input, |property| properties.push(property.to_string()));
    if kind == CssTransitionPropertyValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::TransitionProperty(
        RustOwnedTransitionProperty { kind, properties },
    ))
}

pub(super) fn rust_owned_view_transition_name_style_value_kind(
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let mut name = None;
    let kind = parse_view_transition_name_value(filtered_input, |value| name = Some(value.to_string()));
    if kind == CssViewTransitionNameValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::ViewTransitionName(
        RustOwnedViewTransitionName { kind, name },
    ))
}

pub(super) fn rust_owned_white_space_trim_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let value = parse_white_space_trim_value(filtered_input);
    if value.kind == CssWhiteSpaceTrimValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::WhiteSpaceTrim(RustOwnedWhiteSpaceTrim {
        value,
    }))
}

pub(super) fn rust_owned_white_space_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();

    // https://drafts.csswg.org/css-text-4/#white-space-property
    // normal | pre | pre-wrap | pre-line | <'white-space-collapse'> || <'text-wrap-mode'> || <'white-space-trim'>
    if let Some(value) = parse_legacy_white_space_keyword(&mut parser) {
        parser.discard_whitespace();
        if !parser.has_next_component_value() {
            return Some(RustOwnedStyleValueKind::WhiteSpace(value));
        }
        return None;
    }

    let mut white_space_collapse = None;
    let mut text_wrap_mode = None;
    let mut white_space_trim = None;

    while parser.has_next_component_value() {
        if white_space_collapse.is_none()
            && let Some(value) = parse_white_space_collapse_keyword(&mut parser)
        {
            white_space_collapse = Some(value);
            continue;
        }

        if text_wrap_mode.is_none()
            && let Some(value) = parse_text_wrap_mode_keyword(&mut parser)
        {
            text_wrap_mode = Some(value);
            continue;
        }

        if white_space_trim.is_none()
            && let Some(value) = parse_white_space_trim_prefix(&mut parser)
        {
            white_space_trim = Some(value);
            continue;
        }

        return None;
    }

    Some(RustOwnedStyleValueKind::WhiteSpace(RustOwnedWhiteSpace {
        white_space_collapse: white_space_collapse.unwrap_or_else(|| "collapse".to_string()),
        text_wrap_mode: text_wrap_mode.unwrap_or(CssTextWrapModeValue::Wrap),
        white_space_trim: white_space_trim.unwrap_or(CssWhiteSpaceTrimValue {
            kind: CssWhiteSpaceTrimValueKind::None,
            has_discard_before: false,
            has_discard_after: false,
            has_discard_inner: false,
        }),
    }))
}

pub(super) fn parse_legacy_white_space_keyword(parser: &mut ComponentValueParser) -> Option<RustOwnedWhiteSpace> {
    let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value: ident },
        ..
    })) = parser.next_component_value()
    else {
        return None;
    };
    let (white_space_collapse, text_wrap_mode) = if ident.eq_ignore_ascii_case("normal") {
        ("collapse", CssTextWrapModeValue::Wrap)
    } else if ident.eq_ignore_ascii_case("pre") {
        ("preserve", CssTextWrapModeValue::Nowrap)
    } else if ident.eq_ignore_ascii_case("pre-wrap") {
        ("preserve", CssTextWrapModeValue::Wrap)
    } else if ident.eq_ignore_ascii_case("pre-line") {
        ("preserve-breaks", CssTextWrapModeValue::Wrap)
    } else {
        return None;
    };
    parser.index += 1;

    Some(RustOwnedWhiteSpace {
        white_space_collapse: white_space_collapse.to_string(),
        text_wrap_mode,
        white_space_trim: CssWhiteSpaceTrimValue {
            kind: CssWhiteSpaceTrimValueKind::None,
            has_discard_before: false,
            has_discard_after: false,
            has_discard_inner: false,
        },
    })
}

pub(super) fn parse_white_space_collapse_keyword(parser: &mut ComponentValueParser) -> Option<String> {
    let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    })) = parser.next_component_value()
    else {
        return None;
    };

    if !property_accepts_keyword(PropertyId::WhiteSpaceCollapse, value) {
        return None;
    }

    let value = value.to_string();
    parser.index += 1;
    Some(value)
}

pub(super) fn parse_text_wrap_mode_keyword(parser: &mut ComponentValueParser) -> Option<CssTextWrapModeValue> {
    let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    })) = parser.next_component_value()
    else {
        return None;
    };

    let value = if value.eq_ignore_ascii_case("wrap") {
        CssTextWrapModeValue::Wrap
    } else if value.eq_ignore_ascii_case("nowrap") {
        CssTextWrapModeValue::Nowrap
    } else {
        return None;
    };
    parser.index += 1;
    Some(value)
}

pub(super) fn parse_white_space_trim_prefix(parser: &mut ComponentValueParser) -> Option<CssWhiteSpaceTrimValue> {
    if parser.consume_ident_matching("none") {
        return Some(CssWhiteSpaceTrimValue {
            kind: CssWhiteSpaceTrimValueKind::None,
            has_discard_before: false,
            has_discard_after: false,
            has_discard_inner: false,
        });
    }

    let mut value = CssWhiteSpaceTrimValue {
        kind: CssWhiteSpaceTrimValueKind::List,
        has_discard_before: false,
        has_discard_after: false,
        has_discard_inner: false,
    };
    let mut parsed_any = false;
    while let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value: ident },
        ..
    })) = parser.next_component_value()
    {
        if ident.eq_ignore_ascii_case("discard-before") {
            if value.has_discard_before {
                return None;
            }
            value.has_discard_before = true;
        } else if ident.eq_ignore_ascii_case("discard-after") {
            if value.has_discard_after {
                return None;
            }
            value.has_discard_after = true;
        } else if ident.eq_ignore_ascii_case("discard-inner") {
            if value.has_discard_inner {
                return None;
            }
            value.has_discard_inner = true;
        } else {
            break;
        }
        parser.index += 1;
        parser.discard_whitespace();
        parsed_any = true;
    }

    parsed_any.then_some(value)
}

pub(super) fn rust_owned_will_change_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let mut features = Vec::new();
    let kind = parse_will_change_value(filtered_input, |kind, value| {
        features.push(RustOwnedWillChangeFeature {
            kind,
            value: value.to_string(),
        });
    });
    if kind == CssWillChangeValueKind::Invalid {
        return None;
    }

    Some(RustOwnedStyleValueKind::WillChange(RustOwnedWillChange {
        kind,
        features,
    }))
}

pub(super) fn rust_owned_font_variant_alternates_style_value_kind(source: String) -> Option<RustOwnedStyleValueKind> {
    let values = parse_all_component_values(source.as_bytes(), ComponentValueParser::parse_a_font_variant_alternates)?;
    Some(RustOwnedStyleValueKind::FontVariantLonghand(
        RustOwnedFontVariantLonghand::Alternates(values),
    ))
}

pub(super) fn rust_owned_font_variant_east_asian_style_value_kind(source: String) -> Option<RustOwnedStyleValueKind> {
    let values = parse_all_component_values(source.as_bytes(), ComponentValueParser::parse_a_font_variant_east_asian)?;
    Some(RustOwnedStyleValueKind::FontVariantLonghand(
        RustOwnedFontVariantLonghand::EastAsian(values),
    ))
}

pub(super) fn rust_owned_font_variant_ligatures_style_value_kind(source: String) -> Option<RustOwnedStyleValueKind> {
    let values = parse_all_component_values(source.as_bytes(), ComponentValueParser::parse_a_font_variant_ligatures)?;
    Some(RustOwnedStyleValueKind::FontVariantLonghand(
        RustOwnedFontVariantLonghand::Ligatures(values),
    ))
}

pub(super) fn rust_owned_font_variant_numeric_style_value_kind(source: String) -> Option<RustOwnedStyleValueKind> {
    let values = parse_all_component_values(source.as_bytes(), ComponentValueParser::parse_a_font_variant_numeric)?;
    Some(RustOwnedStyleValueKind::FontVariantLonghand(
        RustOwnedFontVariantLonghand::Numeric(values),
    ))
}

pub(super) fn rust_owned_image_style_value_kind(
    filtered_input: &[u8],
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    if let Some(value) = rust_owned_image_set_style_value_kind(filtered_input, filtered_input_string) {
        return Some(value);
    }

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let [component_value] = strip_whitespace(&component_values) else {
        return None;
    };

    if component_value_parse_as_image_url(component_value) {
        return Some(RustOwnedStyleValueKind::Image(RustOwnedImage {
            kind: RustOwnedImageKind::Url,
            url: rust_owned_url_payload_from_component_value(component_value),
            gradient: None,
            source: serialize_component_values_for_reparsing(
                std::slice::from_ref(component_value),
                filtered_input_string,
            )?,
        }));
    }

    if component_value_parse_as_image_gradient(component_value) {
        let gradient = component_value_parse_as_image_gradient_value(component_value)?;
        return Some(RustOwnedStyleValueKind::Image(RustOwnedImage {
            kind: RustOwnedImageKind::Gradient,
            url: None,
            gradient: Some(gradient),
            source: serialize_component_values_for_reparsing(
                std::slice::from_ref(component_value),
                filtered_input_string,
            )?,
        }));
    }

    None
}

pub(super) fn rust_owned_image_from_component_value(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedImage> {
    let _ = filtered_input_string;
    if component_value_parse_as_image_url(component_value) {
        return Some(RustOwnedImage {
            kind: RustOwnedImageKind::Url,
            url: rust_owned_url_payload_from_component_value(component_value),
            gradient: None,
            source: serialize_component_values_for_reparsing(
                std::slice::from_ref(component_value),
                filtered_input_string,
            )?,
        });
    }

    if component_value_parse_as_image_gradient(component_value) {
        return Some(RustOwnedImage {
            kind: RustOwnedImageKind::Gradient,
            url: None,
            gradient: component_value_parse_as_image_gradient_value(component_value),
            source: serialize_component_values_for_reparsing(
                std::slice::from_ref(component_value),
                filtered_input_string,
            )?,
        });
    }

    if let ComponentValue::Function(function) = component_value
        && component_value_parse_as_image_set_function(function)
    {
        return Some(RustOwnedImage {
            kind: RustOwnedImageKind::ImageSet,
            url: None,
            gradient: None,
            source: serialize_component_values_for_reparsing(
                std::slice::from_ref(component_value),
                filtered_input_string,
            )?,
        });
    }

    None
}

pub(super) fn rust_owned_url_payload_from_component_value(
    component_value: &ComponentValue,
) -> Option<RustOwnedUrlPayload> {
    let mut parser = ComponentValueParser::new(vec![component_value.clone()]);
    let url_function = parser.parse_a_url_function()?;
    parser.discard_whitespace();
    if parser.has_next_component_value() || !url_function.request_url_modifiers.is_empty() {
        return None;
    }
    Some(RustOwnedUrlPayload {
        function_type: url_function.function_type,
        url: url_function.url,
    })
}

pub(super) fn rust_owned_url_from_component_value(
    component_value: &ComponentValue,
    filtered_input_string: &str,
) -> Option<RustOwnedUrl> {
    let source =
        serialize_component_values_for_reparsing(std::slice::from_ref(component_value), filtered_input_string)?;
    Some(rust_owned_url_from_source(&source))
}

pub(super) fn rust_owned_url_from_source(source: &str) -> RustOwnedUrl {
    let (mut parser, _) = parser_from_filtered_input(source.as_bytes());
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let url = if let [component_value] = component_values {
        rust_owned_url_payload_from_component_value(component_value)
    } else {
        None
    };
    RustOwnedUrl {
        source: source.to_string(),
        url,
    }
}

pub(super) fn rust_owned_image_set_style_value_kind(
    filtered_input: &[u8],
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    let [ComponentValue::Function(function)] = component_values else {
        return None;
    };

    if !function.name.eq_ignore_ascii_case("image-set") && !function.name.eq_ignore_ascii_case("-webkit-image-set") {
        return None;
    }

    // https://drafts.csswg.org/css-images-4/#image-set-notation
    // image-set() = image-set( <image-set-option># )
    // <image-set-option> = [ <image> | <string> ] [ <resolution> || type(<string>) ]
    // https://compat.spec.whatwg.org/#css-%27-webkit-image-set%27-alias
    // Implementations must accept -webkit-image-set() as a parse-time alias of image-set().
    let options = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        rust_owned_image_set_option(component_values, filtered_input_string)
    })?;
    if options.is_empty() {
        return None;
    }

    Some(RustOwnedStyleValueKind::ImageSet(RustOwnedImageSet { options }))
}

pub(super) fn rust_owned_image_set_option(
    component_values: Vec<ComponentValue>,
    filtered_input_string: &str,
) -> Option<RustOwnedImageSetOption> {
    let component_values = strip_whitespace(&component_values);
    if component_values.is_empty() {
        return None;
    }

    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();

    let image = parser.next_component_value()?;
    let image = if component_value_parse_as_image_set_string(image) {
        let source = component_values_string_value(std::slice::from_ref(image))?.to_string();
        RustOwnedImage {
            kind: RustOwnedImageKind::Url,
            url: Some(RustOwnedUrlPayload {
                function_type: CssUrlFunctionType::Url,
                url: source.clone(),
            }),
            gradient: None,
            source,
        }
    } else if component_value_parse_as_image_set_image(image) || component_value_parse_as_image_set_gradient(image) {
        rust_owned_image_from_component_value(image, filtered_input_string)?
    } else {
        return None;
    };
    parser.index += 1;

    let mut resolution = None;
    let mut mime_type = None;
    loop {
        parser.discard_whitespace();
        let Some(component_value) = parser.next_component_value() else {
            break;
        };

        if resolution.is_none() && component_value_parse_as_image_set_resolution(component_value) {
            resolution = Some(serialize_component_values_for_reparsing(
                std::slice::from_ref(component_value),
                filtered_input_string,
            )?);
            parser.index += 1;
            continue;
        }

        if mime_type.is_none()
            && let Some(value) = image_set_type_value(component_value)
        {
            mime_type = Some(value);
            parser.index += 1;
            continue;
        }

        return None;
    }

    Some(RustOwnedImageSetOption {
        image,
        resolution,
        mime_type,
    })
}

pub(super) fn rust_owned_easing_function_style_value_kind(
    filtered_input: &[u8],
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    let value = match component_values {
        [
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }),
        ] if value.eq_ignore_ascii_case("step-start") || value.eq_ignore_ascii_case("step-end") => {
            RustOwnedEasingFunctionValue::Keyword(value.clone())
        }
        [ComponentValue::Function(function)] if function.name.eq_ignore_ascii_case("linear") => {
            RustOwnedEasingFunctionValue::Linear(rust_owned_linear_easing_stops(function, filtered_input_string)?)
        }
        [ComponentValue::Function(function)] if function.name.eq_ignore_ascii_case("cubic-bezier") => {
            let groups = rust_owned_single_component_function_arguments(function, filtered_input_string)?;
            let [x1, y1, x2, y2] = groups.as_slice() else {
                return None;
            };
            if !component_value_parse_as_number_in_range(x1, 0.0, 1.0)
                || !component_value_parse_as_number_prefix(y1)
                || !component_value_parse_as_number_in_range(x2, 0.0, 1.0)
                || !component_value_parse_as_number_prefix(y2)
            {
                return None;
            }
            RustOwnedEasingFunctionValue::CubicBezier {
                x1: component_value_parse_as_nested_number(x1, filtered_input_string)?,
                y1: component_value_parse_as_nested_number(y1, filtered_input_string)?,
                x2: component_value_parse_as_nested_number(x2, filtered_input_string)?,
                y2: component_value_parse_as_nested_number(y2, filtered_input_string)?,
            }
        }
        [ComponentValue::Function(function)] if function.name.eq_ignore_ascii_case("steps") => {
            let groups = rust_owned_single_component_function_arguments(function, filtered_input_string)?;
            if groups.is_empty() || groups.len() > 2 {
                return None;
            }

            let mut min_intervals = 1.0;
            let position = if let Some(step_position) = groups.get(1) {
                let ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                }) = step_position
                else {
                    return None;
                };
                if !is_step_position_keyword(value) {
                    return None;
                }
                if value.eq_ignore_ascii_case("jump-none") {
                    min_intervals = 2.0;
                }
                Some(rust_owned_step_position(value)?)
            } else {
                None
            };

            if !component_value_parse_as_integer_in_range(&groups[0], min_intervals, f64::INFINITY) {
                return None;
            }

            RustOwnedEasingFunctionValue::Steps {
                intervals: component_value_parse_as_nested_integer(&groups[0], filtered_input_string)?,
                position,
            }
        }
        _ => return None,
    };

    Some(RustOwnedStyleValueKind::EasingFunction(RustOwnedEasingFunction {
        value,
    }))
}

pub(super) fn rust_owned_linear_easing_stops(
    function: &Function,
    filtered_input_string: &str,
) -> Option<Vec<RustOwnedLinearEasingStop>> {
    // https://drafts.csswg.org/css-easing-2/#typedef-linear-easing-function
    // <linear-easing-function> = linear( [ <number> && <linear-stop-length>? ]# )
    // <linear-stop-length> = <percentage>{1,2}
    let stops = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        rust_owned_linear_easing_stop(component_values, filtered_input_string)
    })?;

    (!stops.is_empty()).then_some(stops)
}

pub(super) fn rust_owned_linear_easing_stop(
    component_values: Vec<ComponentValue>,
    filtered_input_string: &str,
) -> Option<RustOwnedLinearEasingStop> {
    let component_values: Vec<_> = component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
        .cloned()
        .collect();
    let mut component_values = component_values.into_iter().peekable();

    let mut output = None;
    if component_values
        .peek()
        .is_some_and(component_value_parse_as_number_prefix)
    {
        output = Some(component_value_parse_as_nested_number(
            &component_values.next()?,
            filtered_input_string,
        )?);
    }

    let mut stop_lengths = Vec::new();
    for _ in 0..2 {
        if component_values.peek().is_some_and(|component_value| {
            parse_percentage_value_prefix(component_value) == CssPrimitiveValueKind::Percentage
        }) {
            stop_lengths.push(component_value_parse_as_nested_percentage(
                &component_values.next()?,
                filtered_input_string,
            )?);
        }
    }

    if component_values
        .peek()
        .is_some_and(component_value_parse_as_number_prefix)
    {
        if output.is_some() {
            return None;
        }
        output = Some(component_value_parse_as_nested_number(
            &component_values.next()?,
            filtered_input_string,
        )?);
    }

    if component_values.next().is_some() {
        return None;
    }

    Some(RustOwnedLinearEasingStop {
        output: output?,
        first_stop_length: stop_lengths.first().cloned(),
        second_stop_length: stop_lengths.get(1).cloned(),
    })
}

pub(super) fn rust_owned_single_component_function_arguments(
    function: &Function,
    filtered_input_string: &str,
) -> Option<Vec<ComponentValue>> {
    parse_comma_separated_component_values(function.value.clone(), |component_values| {
        let [component_value] = strip_whitespace(&component_values) else {
            return None;
        };
        serialize_component_values_for_reparsing(std::slice::from_ref(component_value), filtered_input_string)?;
        Some(component_value.clone())
    })
}

pub(super) fn rust_owned_fit_content_style_value_kind(
    filtered_input: &[u8],
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    let value = match component_values {
        [
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }),
        ] if value.eq_ignore_ascii_case("fit-content") => {
            RustOwnedNestedPrimitiveValue::Keyword("fit-content".to_string())
        }
        [ComponentValue::Function(function)] if function.name.eq_ignore_ascii_case("fit-content") => {
            // https://drafts.csswg.org/css-sizing-3/#funcdef-width-fit-content
            // fit-content() = fit-content( <length-percentage [0,∞]> )
            let [component_value] = strip_whitespace(&function.value) else {
                return None;
            };
            if !component_value_parse_as_length_percentage(component_value) {
                return None;
            }
            component_value_parse_as_nested_length_percentage(component_value, filtered_input_string)?
        }
        _ => return None,
    };

    Some(RustOwnedStyleValueKind::FitContent(RustOwnedFitContent { value }))
}

pub(super) fn rust_owned_rect_style_value_kind(
    filtered_input: &[u8],
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    let [ComponentValue::Function(function)] = component_values else {
        return None;
    };
    if !function.name.eq_ignore_ascii_case("rect") {
        return None;
    }

    // https://www.w3.org/TR/CSS2/visufx.html#value-def-shape
    // In CSS 2.1, the only valid <shape> value is:
    // rect(<top>, <right>, <bottom>, <left>)
    let mut parser = ComponentValueParser::new(function.value.clone());
    let mut sides = Vec::with_capacity(4);
    let mut requires_commas = None;

    for side in 0..4 {
        sides.push(rust_owned_rect_side(&mut parser, filtered_input_string)?);

        parser.discard_whitespace();
        if side == 3 {
            if parser.has_next_component_value() {
                return None;
            }
            break;
        }

        let next_is_comma = parser.consume_a_comma();
        match requires_commas {
            Some(true) if !next_is_comma => return None,
            Some(false) if next_is_comma => return None,
            None => requires_commas = Some(next_is_comma),
            _ => {}
        }
    }

    Some(RustOwnedStyleValueKind::Rect(RustOwnedRect {
        sides,
        requires_commas: requires_commas.unwrap_or(false),
    }))
}

pub(super) fn rust_owned_rect_side(
    parser: &mut ComponentValueParser,
    filtered_input_string: &str,
) -> Option<RustOwnedNestedPrimitiveValue> {
    parser.discard_whitespace();

    let component_value = parser.next_component_value()?;
    if matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case("auto")
    ) {
        parser.index += 1;
        return Some(RustOwnedNestedPrimitiveValue::Keyword("auto".to_string()));
    }

    let value = component_value_parse_as_nested_length(component_value, filtered_input_string)?;
    parser.index += 1;
    Some(value)
}

pub(super) fn rust_owned_primitive_style_value_kind(
    value_type: PropertyValueType,
    primitive_kind: CssPrimitiveValueKind,
    numeric_value: Option<f64>,
    secondary_numeric_value: Option<f64>,
    value: String,
) -> RustOwnedStyleValueKind {
    if numeric_value.is_none()
        && matches!(
            primitive_kind,
            CssPrimitiveValueKind::Integer
                | CssPrimitiveValueKind::Number
                | CssPrimitiveValueKind::Percentage
                | CssPrimitiveValueKind::Angle
                | CssPrimitiveValueKind::Flex
                | CssPrimitiveValueKind::Frequency
                | CssPrimitiveValueKind::Length
                | CssPrimitiveValueKind::Resolution
                | CssPrimitiveValueKind::Time
                | CssPrimitiveValueKind::Ratio
        )
    {
        return RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Token {
            primitive_kind,
            numeric_value,
            secondary_numeric_value,
            value,
            value_type,
        });
    }

    match primitive_kind {
        CssPrimitiveValueKind::Integer => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Integer(numeric_value.unwrap_or(0.0) as i32),
            value_type,
        }),
        CssPrimitiveValueKind::Number if value_type == PropertyValueType::OpacityValue => {
            RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Number(numeric_value.unwrap_or(0.0)),
                value_type,
            })
        }
        CssPrimitiveValueKind::Number => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Number(numeric_value.unwrap_or(0.0)),
            value_type,
        }),
        CssPrimitiveValueKind::Percentage if value_type == PropertyValueType::OpacityValue => {
            RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Percentage(numeric_value.unwrap_or(0.0)),
                value_type,
            })
        }
        CssPrimitiveValueKind::Percentage => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Percentage(numeric_value.unwrap_or(0.0)),
            value_type,
        }),
        CssPrimitiveValueKind::Angle => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Angle {
                value: numeric_value.unwrap_or(0.0),
                unit: value,
            },
            value_type,
        }),
        CssPrimitiveValueKind::Flex => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Flex {
                value: numeric_value.unwrap_or(0.0),
                unit: value,
            },
            value_type,
        }),
        CssPrimitiveValueKind::Frequency => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Frequency {
                value: numeric_value.unwrap_or(0.0),
                unit: value,
            },
            value_type,
        }),
        CssPrimitiveValueKind::Length => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Length {
                value: numeric_value.unwrap_or(0.0),
                unit: value,
            },
            value_type,
        }),
        CssPrimitiveValueKind::Resolution => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Resolution {
                value: numeric_value.unwrap_or(0.0),
                unit: value,
            },
            value_type,
        }),
        CssPrimitiveValueKind::Time => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
            value: RustOwnedNestedPrimitiveValue::Time {
                value: numeric_value.unwrap_or(0.0),
                unit: value,
            },
            value_type,
        }),
        CssPrimitiveValueKind::Ratio => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Ratio {
            numerator: numeric_value.unwrap_or(0.0),
            denominator: secondary_numeric_value.unwrap_or(1.0),
            has_denominator: value == "has-denominator",
            value_type,
        }),
        CssPrimitiveValueKind::String => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Token {
            primitive_kind,
            numeric_value,
            secondary_numeric_value,
            value,
            value_type,
        }),
        _ => RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Token {
            primitive_kind,
            numeric_value,
            secondary_numeric_value,
            value,
            value_type,
        }),
    }
}
