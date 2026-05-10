/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::{
    BooleanExpression, BooleanExpressionTestKind, ComponentValue, ComponentValueParser, CounterStyle,
    CssAnchorNameOrScopeValueKind, CssAnimationNameItemKind, CssAnimationNameValueKind, CssBackgroundSizeValueKind,
    CssBasicShapeValueKind, CssBooleanExpressionEventKind, CssCalculationNodeKind, CssColorFunctionValueKind,
    CssColorSchemeValue, CssColorSchemeValueKind, CssColorValueKind, CssContainValue, CssContainValueKind,
    CssContainerTypeValueKind, CssCounterStyleKind, CssCounterStyleNegativeSymbolCount, CssCounterStyleRangeKind,
    CssCounterStyleSymbolsType, CssCounterStyleSystemKind, CssCropOrCrossKind, CssDescriptorResultKind,
    CssDescriptorValueType, CssDisplayBox, CssDisplayInside, CssDisplayInternal, CssDisplayListItem, CssDisplayOutside,
    CssDisplayValueKind, CssEasingValueKind, CssFitContentValueKind, CssFontLanguageOverrideKind, CssFontSourceKind,
    CssFontTech, CssFontVariantAlternatesValueKind, CssFontVariantEastAsianValueKind, CssFontVariantLigaturesValueKind,
    CssFontVariantNumericValueKind, CssFontVariantSimpleValueKind, CssGeneratedPropertyValueKind, CssGridAutoFlowAxis,
    CssGridAutoFlowDense, CssGridAutoFlowValueKind, CssGridTrackPlacementValueKind, CssGridTrackSizeListValueKind,
    CssImageSetValueKind, CssMediaFeatureValueKind, CssMediaFeatureValuePayloadKind, CssMediaFeatureValueSyntaxKind,
    CssMediaQuery, CssMediaTypeKind, CssNonnegativeIntegerSymbolPairOrder, CssOpenTypeSettingsKind,
    CssOpenTypeTaggedValueKind, CssPagePseudoClassKind, CssPageSizeKeyword, CssPageSizeOrientation,
    CssPaintOrderKeyword, CssPaintOrderValue, CssPaintOrderValueKind, CssParsedColorKind, CssPositionAnchorValueKind,
    CssPositionTryOrderValue, CssPositionValueKind, CssPositionVisibilityValue, CssPositionVisibilityValueKind,
    CssPrimitiveValueKind, CssPrimitiveValueOptions, CssPrimitiveValueType, CssQuotesValueKind, CssRatioValue,
    CssRatioValueKind, CssRectValueKind, CssRepeatStyleRepetition, CssRepeatStyleValueKind, CssScrollFunctionAxisKind,
    CssScrollFunctionScrollerKind, CssScrollFunctionValue, CssScrollFunctionValueKind, CssScrollbarGutterValueKind,
    CssSelectorEventKind, CssSimpleSelectorKind, CssStyleValueKind, CssSupportsFeatureKind,
    CssTextUnderlinePositionHorizontal, CssTextUnderlinePositionValue, CssTextUnderlinePositionVertical,
    CssTextWrapModeValue, CssTextWrapStyleValue, CssTextWrapValue, CssTextWrapValueKind, CssTimelineNameItemKind,
    CssTimelineNameValueKind, CssTimelineScopeValueKind, CssTouchActionKeyword, CssTouchActionValue,
    CssTouchActionValueKind, CssTransformFunctionValueKind, CssTransformLonghandValueKind,
    CssTransitionBehaviorItemKind, CssTransitionBehaviorValueKind, CssTransitionPropertyValueKind,
    CssUrlCrossOriginModifierValue, CssUrlFunctionType, CssUrlModifierKind, CssValueTypeSyntaxKind,
    CssViewFunctionInsetKind, CssViewFunctionInsetPosition, CssViewFunctionValue, CssViewFunctionValueKind,
    CssViewTimelineInsetValue, CssViewTimelineInsetValueKind, CssViewTransitionNameValueKind, CssWhiteSpaceTrimValue,
    CssWhiteSpaceTrimValueKind, CssWillChangeFeatureKind, CssWillChangeValueKind, DescriptorResultCallbacks,
    FamilyName, FontFamilyValue, FontStyle, FontVariant, FontVariantAlternatesValue, FontVariantEastAsianValue,
    FontVariantLigaturesValue, FontVariantNumericValue, MediaFeatureNameKind, MediaFeatureSyntax,
    MediaFeatureValueSyntaxKind, MediaQueryModifier, MediaQuerySyntax, MfComparison, NamespaceType,
    OpenTypeTaggedValue, Parser, PositionEdge, PseudoElementSelectorValue, Rule, RuleContext, RuleOrListOfDeclarations,
    RustOwnedAnchorFunction, RustOwnedAnchorNameOrScope, RustOwnedAnchorSizeFunction, RustOwnedAnimationName,
    RustOwnedAnimationNameItem, RustOwnedAspectRatio, RustOwnedBackgroundSize, RustOwnedBackgroundSizeList,
    RustOwnedBasicShape, RustOwnedBasicShapeFillRule, RustOwnedBasicShapeKind, RustOwnedBasicShapePolygonPoint,
    RustOwnedBorder, RustOwnedBorderImage, RustOwnedBorderImageOutset, RustOwnedBorderImageOutsetList,
    RustOwnedBorderImageRepeat, RustOwnedBorderImageRepeatList, RustOwnedBorderImageSlice, RustOwnedBorderImageSource,
    RustOwnedBorderImageWidthList, RustOwnedBorderRadius, RustOwnedBorderSpacing, RustOwnedColor, RustOwnedColorScheme,
    RustOwnedColumns, RustOwnedComponentShorthandItem, RustOwnedContain, RustOwnedContainerType, RustOwnedContent,
    RustOwnedContentItem, RustOwnedCoordinatingValueListShorthandItem, RustOwnedCornerShape,
    RustOwnedCounterDefinition, RustOwnedCounterDefinitions, RustOwnedCounterFunction, RustOwnedCounterFunctionKind,
    RustOwnedCounterStyleAdditiveTuple, RustOwnedCounterStylePadDescriptor, RustOwnedCounterStyleRangeDescriptor,
    RustOwnedCounterStyleSystemDescriptor, RustOwnedCursor, RustOwnedCursorImage, RustOwnedDescriptorPrimitiveValue,
    RustOwnedDisplay, RustOwnedExplicitGridTrack, RustOwnedFilterValue, RustOwnedFilterValueList, RustOwnedFitContent,
    RustOwnedFlexBasis, RustOwnedFlexDirection, RustOwnedFlexFlow, RustOwnedFlexShorthand, RustOwnedFlexWrap,
    RustOwnedFontFamilyList, RustOwnedFontLanguageOverride, RustOwnedFontShorthandItem, RustOwnedFontStyle,
    RustOwnedFontVariantLonghand, RustOwnedGeneratedValueList, RustOwnedGeneratedValueListItem, RustOwnedGradientKind,
    RustOwnedGridAutoFlow, RustOwnedGridPlacementShorthandItem, RustOwnedGridRepeat, RustOwnedGridRepeatType,
    RustOwnedGridTemplateShorthandItem, RustOwnedGridTrackPlacement, RustOwnedGridTrackSize,
    RustOwnedGridTrackSizeList, RustOwnedGridTrackSizeListItem, RustOwnedIdentifierValue, RustOwnedImage,
    RustOwnedImageKind, RustOwnedLineStyle, RustOwnedListStyle, RustOwnedListStyleImage, RustOwnedListStylePosition,
    RustOwnedListStyleType, RustOwnedMathDepth, RustOwnedMathFunction, RustOwnedNestedPrimitiveValue,
    RustOwnedOpenTypeSettings, RustOwnedOpenTypeSettingsStyleValue, RustOwnedOpenTypeSettingsStyleValueKind,
    RustOwnedOverflowClipMargin, RustOwnedPageSizeDescriptor, RustOwnedPaint, RustOwnedPaintOrder,
    RustOwnedPlaceShorthand, RustOwnedPosition, RustOwnedPositionAnchor, RustOwnedPositionArea,
    RustOwnedPositionComponent, RustOwnedPositionList, RustOwnedPositionListItem, RustOwnedPositionTryFallback,
    RustOwnedPositionTryFallbacks, RustOwnedPositionTryOrder, RustOwnedPositionVisibility,
    RustOwnedPositionalValueListShorthandItem, RustOwnedPrimitiveValue, RustOwnedRect, RustOwnedRepeatStyle,
    RustOwnedRepeatStyleList, RustOwnedResolvedPosition, RustOwnedScrollTimeline, RustOwnedScrollbarColor,
    RustOwnedScrollbarGutter, RustOwnedShadow, RustOwnedShadowPlacement, RustOwnedShapeBox, RustOwnedShapeOutside,
    RustOwnedSimpleFilterFunction, RustOwnedSingleShadow, RustOwnedStrokeDasharray, RustOwnedStyleValue,
    RustOwnedStyleValueKind, RustOwnedStyleValueList, RustOwnedStyleValueListSeparator, RustOwnedStyleValueParseResult,
    RustOwnedTextDecoration, RustOwnedTextDecorationLine, RustOwnedTextIndent, RustOwnedTextUnderlinePosition,
    RustOwnedTextWrap, RustOwnedTextWrapMode, RustOwnedTextWrapStyle, RustOwnedTimelineName, RustOwnedTimelineNameItem,
    RustOwnedTouchAction, RustOwnedTransformLonghand, RustOwnedTransformLonghandFunction, RustOwnedTransformOrigin,
    RustOwnedTransformation, RustOwnedTransformationArgument, RustOwnedTransitionBehavior, RustOwnedTransitionProperty,
    RustOwnedTreeCountingFunction, RustOwnedTreeCountingFunctionKind, RustOwnedUrl, RustOwnedUrlPayload,
    RustOwnedViewTimeline, RustOwnedViewTimelineInset, RustOwnedWhiteSpace, RustOwnedWhiteSpaceTrim,
    SelectorCombinator, SelectorParsingMode, SelectorSyntax, SelectorType, SimpleSelectorSyntax, SyntaxNode,
    TEXT_DECORATION_LINE_BLINK, TEXT_DECORATION_LINE_LINE_THROUGH, TEXT_DECORATION_LINE_OVERLINE,
    TEXT_DECORATION_LINE_UNDERLINE, TransformFunction, TransformFunctionParameterType, UrlModifier, auto_keyword,
    component_values_parse_as_media_feature, component_values_parse_as_mf_value_syntax,
    component_values_parse_as_property_value_type, component_values_parse_as_property_value_type_with_options,
    component_values_parse_as_syntax, component_values_parse_as_syntax_with_source,
    component_values_parse_as_value_type, emit_rust_owned_style_value, parse_a_counter_style,
    parse_a_counter_style_name, parse_a_custom_ident, parse_a_custom_property_name, parse_a_dashed_ident,
    parse_a_family_name, parse_a_font_family_value, parse_a_font_feature_settings, parse_a_font_language_override,
    parse_a_font_source, parse_a_font_style, parse_a_font_variant, parse_a_font_variant_alternates,
    parse_a_font_variant_east_asian, parse_a_font_variant_ligatures, parse_a_font_variant_numeric,
    parse_a_font_variation_settings, parse_a_keyframe_selector_list, parse_a_keyframes_name, parse_a_layer_name,
    parse_a_layer_name_list, parse_a_media_query, parse_a_media_test, parse_a_namespace_rule_prelude,
    parse_a_nonnegative_integer_symbol_pair, parse_a_page_selector_list, parse_a_supports_feature,
    parse_a_unicode_range, parse_a_unicode_range_list, parse_a_url_function, parse_a_value_type, parse_an_if_condition,
    parse_an_import_layer, parse_an_import_url, parse_an_opentype_tag, parse_anchor_name_or_scope_value,
    parse_animation_name_value, parse_aspect_ratio_value, parse_background_position_longhand_value,
    parse_background_size_value, parse_basic_shape_value, parse_border_radius_shorthand_value,
    parse_border_radius_value, parse_color_function_value, parse_color_scheme_value, parse_color_value,
    parse_columns_value, parse_contain_value, parse_container_rule_prelude, parse_container_type_value,
    parse_content_value, parse_coordinating_value_list_shorthand, parse_counter_style_additive_symbols,
    parse_counter_style_negative, parse_counter_style_range, parse_counter_style_symbol, parse_counter_style_symbols,
    parse_counter_style_system, parse_crop_or_cross, parse_cursor_value, parse_descriptor_result, parse_display_value,
    parse_easing_value, parse_empty_prelude, parse_filter_value_list_value, parse_fit_content_value,
    parse_flex_flow_value, parse_flex_shorthand_value, parse_font_feature_values_family_name_list,
    parse_font_feature_values_feature_value, parse_font_shorthand, parse_font_weight_absolute_pair,
    parse_generated_property_value, parse_grid_auto_flow_value, parse_grid_auto_track_sizes_value,
    parse_grid_placement_shorthand, parse_grid_template_shorthand, parse_grid_track_placement_value,
    parse_grid_track_size_list_value, parse_image_set_value, parse_layer_shorthand, parse_length_descriptor,
    parse_list_style_value, parse_math_depth_value, parse_optional_declaration_value_descriptor,
    parse_overflow_clip_margin_value, parse_page_size_descriptor, parse_paint_order_value, parse_place_content_value,
    parse_place_items_value, parse_place_self_value, parse_position_anchor_value, parse_position_area_value,
    parse_position_try_fallbacks_value, parse_position_try_order_value, parse_position_value,
    parse_position_visibility_value, parse_positional_value_list_shorthand, parse_positive_percentage_descriptor,
    parse_primitive_value, parse_primitive_value_prefix, parse_quotes_value, parse_ratio_value_prefix,
    parse_rect_value, parse_repeat_style_value, parse_rotate_value, parse_rust_owned_coordinating_value_list_shorthand,
    parse_rust_owned_counter_style_additive_symbols_descriptor, parse_rust_owned_counter_style_negative_descriptor,
    parse_rust_owned_counter_style_pad_descriptor, parse_rust_owned_counter_style_range_descriptor,
    parse_rust_owned_counter_style_symbol_descriptor, parse_rust_owned_counter_style_symbols_descriptor,
    parse_rust_owned_counter_style_system_descriptor, parse_rust_owned_filter_value_list_value,
    parse_rust_owned_font_src_list_descriptor, parse_rust_owned_font_weight_absolute_pair_descriptor,
    parse_rust_owned_generated_longhand_value, parse_rust_owned_length_descriptor,
    parse_rust_owned_page_size_descriptor, parse_rust_owned_positional_value_list_shorthand,
    parse_rust_owned_positive_percentage_descriptor, parse_rust_owned_string_descriptor,
    parse_rust_owned_style_value_for_property, parse_rust_owned_view_timeline_inset_value, parse_scale_value,
    parse_scroll_function_value, parse_scrollbar_gutter_value, parse_shadow_value, parse_shape_outside_value,
    parse_simple_color_value, parse_string_descriptor, parse_stroke_dasharray_value,
    parse_style_value_for_property_with_options, parse_text_decoration_line_value, parse_text_decoration_value,
    parse_text_underline_position_value, parse_text_wrap_mode_value, parse_text_wrap_style_value,
    parse_text_wrap_value, parse_timeline_name_value, parse_timeline_scope_value, parse_touch_action_value,
    parse_transform_function_value, parse_transform_origin_value, parse_transition_behavior_value,
    parse_transition_property_value, parse_translate_value, parse_view_function_value, parse_view_timeline_inset_value,
    parse_view_timeline_inset_value_prefix, parse_view_transition_name_value, parse_white_space_trim_value,
    parse_will_change_value, rust_owned_image_style_value_kind, strip_whitespace,
};
use crate::css_tokenizer::{self, TokenType};
use crate::generated_descriptors::{
    AtRuleId, DescriptorId, DescriptorSyntax, at_rule_supports_descriptor,
    descriptor_allows_arbitrary_substitution_functions, for_each_descriptor_syntax,
};
use crate::generated_media_features::{
    MediaFeatureId, MediaFeatureValueType, media_feature_accepts_identifier, media_feature_accepts_type,
    media_feature_identifier_is_falsey,
};
use crate::generated_properties::{
    PropertyId, PropertyNumericRange, PropertyValueType, longhands_for_shorthand,
    property_accepted_range_by_value_type, property_accepts_keyword, property_accepts_value_type,
    property_custom_ident_blacklist, property_id_from_string, property_name, property_resolves_percentages_relative_to,
    property_value_type_name, resolve_legacy_value_alias,
};
use crate::generated_pseudo_classes::{
    PseudoClassId, PseudoClassParameterType, pseudo_class_id_from_string, pseudo_class_metadata, pseudo_class_name,
};
use crate::generated_pseudo_elements::{
    PseudoElementId, PseudoElementParameterType, aliased_pseudo_element_id_from_string, pseudo_element_id_from_string,
    pseudo_element_metadata, pseudo_element_name,
};

fn position_edge(edge: PositionEdge) -> RustOwnedPositionComponent {
    RustOwnedPositionComponent {
        edge: Some(edge),
        offset: None,
    }
}

fn position_offset(offset: RustOwnedNestedPrimitiveValue) -> RustOwnedPositionComponent {
    RustOwnedPositionComponent {
        edge: None,
        offset: Some(offset),
    }
}

fn position_edge_offset(edge: PositionEdge, offset: RustOwnedNestedPrimitiveValue) -> RustOwnedPositionComponent {
    RustOwnedPositionComponent {
        edge: Some(edge),
        offset: Some(offset),
    }
}
use crate::generated_units::{DimensionType, dimension_for_unit};
use crate::generated_value_types::ValueTypeId;

use super::parser_math::{
    RustOwnedCalculationNode, RustOwnedCalculationNumericValue, emit_rust_owned_calculation_tree,
    parse_rust_owned_calculation_function,
};

fn parse_with<T>(input: &str, parse: impl FnOnce(&mut Parser) -> T) -> T {
    let mut tokens = Vec::new();
    css_tokenizer::tokenize(input.as_bytes(), |token, _| tokens.push(token.clone()));
    parse(&mut Parser::new(tokens))
}

fn parse(input: &str) -> Vec<ComponentValue> {
    parse_with(input, Parser::parse_a_list_of_component_values)
}

fn open_type_value_component_values(input: &str, tag: &str) -> Vec<ComponentValue> {
    let mut parser = ComponentValueParser::new(parse(input));
    loop {
        parser.discard_whitespace();
        let Some(component_value) = parser.consume_the_next_component_value() else {
            return vec![];
        };
        let is_matching_tag = matches!(
            component_value,
            ComponentValue::PreservedToken(crate::css_tokenizer::Token {
                token_type: TokenType::String { value },
                ..
            }) if value == tag
        );
        parser.discard_whitespace();
        if is_matching_tag {
            let mut values = Vec::new();
            while let Some(component_value) = parser.next_component_value() {
                if matches!(
                    component_value,
                    ComponentValue::PreservedToken(crate::css_tokenizer::Token {
                        token_type: TokenType::Comma,
                        ..
                    })
                ) {
                    break;
                }
                values.push(parser.consume_the_next_component_value().unwrap());
            }
            return values;
        }
        while parser.has_next_component_value() && !parser.consume_a_comma() {
            parser.consume_the_next_component_value();
        }
    }
}

fn parse_math_ast(input: &str) -> Option<RustOwnedCalculationNode> {
    let component_values = parse(input);
    let [ComponentValue::Function(function)] = component_values.as_slice() else {
        return None;
    };
    parse_rust_owned_calculation_function(function)
}

fn parse_selector_list(input: &str) -> Option<Vec<SelectorSyntax>> {
    let mut parser = ComponentValueParser::new(parse(input));
    parser.parse_a_selector_list(SelectorType::Standalone, SelectorParsingMode::Normal)
}

fn parse_relative_selector_list(input: &str) -> Option<Vec<SelectorSyntax>> {
    let mut parser = ComponentValueParser::new(parse(input));
    parser.parse_a_selector_list(SelectorType::Relative, SelectorParsingMode::Normal)
}

fn parse_forgiving_selector_list(input: &str) -> Option<Vec<SelectorSyntax>> {
    let mut parser = ComponentValueParser::new(parse(input));
    parser.parse_a_selector_list(SelectorType::Standalone, SelectorParsingMode::Forgiving)
}

fn parse_selector_list_with_namespaces(input: &str, declared_namespaces: &[&str]) -> Option<Vec<SelectorSyntax>> {
    let mut parser = ComponentValueParser::with_declared_namespaces(
        parse(input),
        declared_namespaces
            .iter()
            .map(|declared_namespace| declared_namespace.to_string())
            .collect(),
    );
    parser.parse_a_selector_list(SelectorType::Standalone, SelectorParsingMode::Normal)
}

fn event_string<'a>(ptr: *const u8, len: usize) -> &'a str {
    if len == 0 {
        return "";
    }
    unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
}

fn parse_media_feature_syntax(input: &str) -> Option<MediaFeatureSyntax> {
    component_values_parse_as_media_feature(&parse(input))
}

fn parse_media_query_list(input: &str) -> Vec<MediaQuerySyntax> {
    parse_with(input, Parser::parse_a_media_query_list)
}

fn parse_media_query(input: &str) -> (bool, Option<CssMediaQuery>) {
    let mut media_query = None;
    let did_parse = parse_a_media_query(
        input.as_bytes(),
        |parsed_media_query| {
            media_query = Some(parsed_media_query);
        },
        |_| {},
        |_| {},
        |_| {},
        |_| {},
    );
    (did_parse, media_query)
}

fn parse_media_test(input: &str) -> (Vec<CssBooleanExpressionEventKind>, usize) {
    let mut events = Vec::new();
    let mut media_feature_count = 0;
    parse_a_media_test(
        input.as_bytes(),
        |event| events.push(event),
        |_| media_feature_count += 1,
        |_| {},
        |_| {},
    );
    (events, media_feature_count)
}

fn parse_media_test_values(
    input: &str,
) -> Vec<(
    CssMediaFeatureValueKind,
    CssMediaFeatureValueSyntaxKind,
    CssMediaFeatureValuePayloadKind,
    f64,
    f64,
    String,
)> {
    let mut values = Vec::new();
    parse_a_media_test(
        input.as_bytes(),
        |_| {},
        |_| {},
        |value| {
            let unit_or_ident = if value.unit_or_ident_ptr.is_null() {
                String::new()
            } else {
                // SAFETY: The Rust parser calls this callback synchronously while the source string
                // backing the FFI slice is still alive.
                let slice = unsafe { std::slice::from_raw_parts(value.unit_or_ident_ptr, value.unit_or_ident_len) };
                std::str::from_utf8(slice).unwrap().to_string()
            };
            values.push((
                value.kind,
                value.syntax_kind,
                value.payload_kind,
                value.numeric_value,
                value.secondary_numeric_value,
                unit_or_ident,
            ));
        },
        |_| {},
    );
    values
}

fn parse_if_condition(input: &str) -> Vec<CssBooleanExpressionEventKind> {
    let mut events = Vec::new();
    parse_an_if_condition(input.as_bytes(), |event| events.push(event), |_| {});
    events
}

fn parse_supports_feature(input: &str) -> Option<(CssSupportsFeatureKind, Option<String>)> {
    let mut feature = None;
    let parsed = parse_a_supports_feature(input.as_bytes(), |kind, name| {
        feature = Some((kind, name.map(ToOwned::to_owned)));
    });
    parsed.then_some(feature).flatten()
}

fn parse_page_selector_list(input: &str) -> Option<Vec<(Option<String>, Vec<CssPagePseudoClassKind>)>> {
    let selectors = std::cell::RefCell::new(Vec::new());
    let parsed = parse_a_page_selector_list(
        input.as_bytes(),
        |selector| {
            let name = if selector.has_name {
                let bytes = unsafe { std::slice::from_raw_parts(selector.name_ptr, selector.name_len) };
                Some(String::from_utf8(bytes.to_vec()).expect("selector name must be utf-8"))
            } else {
                None
            };
            selectors.borrow_mut().push((name, Vec::new()));
        },
        |pseudo_class| {
            selectors
                .borrow_mut()
                .last_mut()
                .expect("pseudo-class callback must follow a selector callback")
                .1
                .push(pseudo_class);
        },
    );

    parsed.then(|| selectors.into_inner())
}

fn parse_keyframe_selector_list(input: &str) -> Option<Vec<f64>> {
    let mut selectors = Vec::new();
    let parsed = parse_a_keyframe_selector_list(input.as_bytes(), |selector| selectors.push(selector));
    parsed.then_some(selectors)
}

fn parse_keyframes_name(input: &str) -> Option<String> {
    let mut name = None;
    let parsed = parse_a_keyframes_name(input.as_bytes(), |parsed_name| name = Some(parsed_name.to_string()));
    parsed.then_some(name).flatten()
}

fn parse_custom_property_name(input: &str) -> Option<String> {
    let mut name = None;
    let parsed = parse_a_custom_property_name(input.as_bytes(), |parsed_name| name = Some(parsed_name.to_string()));
    parsed.then_some(name).flatten()
}

fn parse_custom_ident(input: &str) -> Option<String> {
    let mut name = None;
    let parsed = parse_a_custom_ident(input.as_bytes(), |parsed_name| name = Some(parsed_name.to_string()));
    parsed.then_some(name).flatten()
}

fn parse_dashed_ident(input: &str) -> Option<String> {
    let mut name = None;
    let parsed = parse_a_dashed_ident(input.as_bytes(), |parsed_name| name = Some(parsed_name.to_string()));
    parsed.then_some(name).flatten()
}

fn parse_unicode_range(input: &str) -> Option<(u32, u32)> {
    let mut range = None;
    let parsed = parse_a_unicode_range(input.as_bytes(), |parsed_range| {
        range = Some((parsed_range.min_code_point, parsed_range.max_code_point))
    });
    parsed.then_some(range).flatten()
}

fn parse_unicode_range_list(input: &str) -> Option<Vec<(u32, u32)>> {
    let mut ranges = Vec::new();
    let parsed = parse_a_unicode_range_list(input.as_bytes(), |parsed_range| {
        ranges.push((parsed_range.min_code_point, parsed_range.max_code_point))
    });
    parsed.then_some(ranges)
}

fn parse_url_function(input: &str) -> Option<(CssUrlFunctionType, String, Vec<CssUrlModifierKind>)> {
    let mut url_function = None;
    let mut modifiers = Vec::new();
    let parsed = parse_a_url_function(
        input.as_bytes(),
        |parsed_url_function| {
            let url = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                    parsed_url_function.url_ptr,
                    parsed_url_function.url_len,
                ))
            };
            url_function = Some((parsed_url_function.function_type, url.to_string()));
        },
        |modifier| {
            modifiers.push(modifier.kind);
        },
    );
    parsed
        .then_some(url_function.map(|(function_type, url)| (function_type, url, modifiers)))
        .flatten()
}

fn parse_import_url(input: &str) -> Option<(CssUrlFunctionType, String, Vec<CssUrlModifierKind>)> {
    let mut url_function = None;
    let mut modifiers = Vec::new();
    let parsed = parse_an_import_url(
        input.as_bytes(),
        |parsed_url_function| {
            let url = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                    parsed_url_function.url_ptr,
                    parsed_url_function.url_len,
                ))
            };
            url_function = Some((parsed_url_function.function_type, url.to_string()));
        },
        |modifier| {
            modifiers.push(modifier.kind);
        },
    );
    parsed
        .then_some(url_function.map(|(function_type, url)| (function_type, url, modifiers)))
        .flatten()
}

fn parse_font_source(input: &str) -> Option<(CssFontSourceKind, Option<String>, Option<String>, Vec<CssFontTech>)> {
    let mut source_kind = None;
    let mut local_name = None;
    let mut url = None;
    let mut format = None;
    let mut tech = Vec::new();
    let parsed = parse_a_font_source(
        input.as_bytes(),
        |kind, family_name| {
            source_kind = Some(kind);
            if let Some(family_name) = family_name {
                local_name = Some(family_name.name.clone());
            }
        },
        |url_function| {
            let parsed_url = unsafe {
                std::str::from_utf8_unchecked(std::slice::from_raw_parts(url_function.url_ptr, url_function.url_len))
            };
            url = Some(parsed_url.to_string());
        },
        |_| {},
        |parsed_format| {
            format = Some(parsed_format.to_string());
        },
        |parsed_tech| {
            tech.push(parsed_tech);
        },
    );
    parsed
        .then_some(source_kind.map(|kind| (kind, local_name.or(url), format, tech)))
        .flatten()
}

fn parse_font_language_override(input: &str) -> Option<(CssFontLanguageOverrideKind, Option<String>)> {
    let mut font_language_override = None;
    let parsed = parse_a_font_language_override(input.as_bytes(), |kind, value| {
        font_language_override = Some((kind, value.map(ToString::to_string)));
    });
    parsed.then_some(font_language_override).flatten()
}

fn parse_opentype_tag(input: &str) -> Option<String> {
    let mut opentype_tag = None;
    let parsed = parse_an_opentype_tag(input.as_bytes(), |value| opentype_tag = Some(value.to_string()));
    parsed.then_some(opentype_tag).flatten()
}

fn parse_font_feature_settings(input: &str) -> Option<(CssOpenTypeSettingsKind, Vec<OpenTypeTaggedValue>)> {
    let mut settings_kind = None;
    let mut tag_values = Vec::new();
    let parsed = parse_a_font_feature_settings(
        input.as_bytes(),
        |kind| settings_kind = Some(kind),
        |tagged_value| tag_values.push(tagged_value.clone()),
    );
    parsed.then(|| {
        (
            settings_kind.expect("font feature settings kind must be parsed"),
            tag_values,
        )
    })
}

fn parse_font_variation_settings(input: &str) -> Option<(CssOpenTypeSettingsKind, Vec<OpenTypeTaggedValue>)> {
    let mut settings_kind = None;
    let mut tag_values = Vec::new();
    let parsed = parse_a_font_variation_settings(
        input.as_bytes(),
        |kind| settings_kind = Some(kind),
        |tagged_value| tag_values.push(tagged_value.clone()),
    );
    parsed.then(|| {
        (
            settings_kind.expect("font variation settings kind must be parsed"),
            tag_values,
        )
    })
}

fn parse_font_style(input: &str) -> Option<FontStyle> {
    let mut font_style = None;
    let parsed = parse_a_font_style(input.as_bytes(), |parsed_font_style| {
        font_style = Some(parsed_font_style);
    });
    parsed.then_some(font_style).flatten()
}

fn parse_font_variant_alternates(input: &str) -> Option<Vec<FontVariantAlternatesValue>> {
    let values = std::cell::RefCell::new(Vec::new());
    let parsed = parse_a_font_variant_alternates(
        input.as_bytes(),
        |kind| {
            values.borrow_mut().push(FontVariantAlternatesValue {
                kind,
                feature_value_names: Vec::new(),
            });
        },
        |feature_value_name| {
            values
                .borrow_mut()
                .last_mut()
                .expect("feature value name callback must follow a value callback")
                .feature_value_names
                .push(feature_value_name.to_string());
        },
    );
    parsed.then(|| values.into_inner())
}

fn parse_font_variant(input: &str) -> Option<FontVariant> {
    let font_variant = std::cell::RefCell::new(FontVariant::default());
    let parsed = parse_a_font_variant(
        input.as_bytes(),
        |kind, value| {
            let mut font_variant = font_variant.borrow_mut();
            match kind {
                CssFontVariantSimpleValueKind::LigaturesNone => font_variant.ligatures_none = true,
                CssFontVariantSimpleValueKind::Caps => font_variant.caps = value.map(ToString::to_string),
                CssFontVariantSimpleValueKind::Emoji => font_variant.emoji = value.map(ToString::to_string),
                CssFontVariantSimpleValueKind::Position => font_variant.position = value.map(ToString::to_string),
            }
        },
        |kind| {
            let mut font_variant = font_variant.borrow_mut();
            font_variant
                .alternates
                .get_or_insert_default()
                .push(FontVariantAlternatesValue {
                    kind,
                    feature_value_names: Vec::new(),
                });
        },
        |feature_value_name| {
            font_variant
                .borrow_mut()
                .alternates
                .as_mut()
                .expect("feature value name callback must follow a value callback")
                .last_mut()
                .expect("feature value name callback must follow a value callback")
                .feature_value_names
                .push(feature_value_name.to_string());
        },
        |value| {
            font_variant
                .borrow_mut()
                .east_asian
                .get_or_insert_default()
                .push(value.clone());
        },
        |value| {
            font_variant
                .borrow_mut()
                .numeric
                .get_or_insert_default()
                .push(value.clone());
        },
        |value| {
            font_variant
                .borrow_mut()
                .ligatures
                .get_or_insert_default()
                .push(value.clone());
        },
    );
    parsed.then(|| font_variant.into_inner())
}

fn parse_font_variant_east_asian(input: &str) -> Option<Vec<FontVariantEastAsianValue>> {
    let mut values = Vec::new();
    let parsed = parse_a_font_variant_east_asian(input.as_bytes(), |value| values.push(value.clone()));
    parsed.then_some(values)
}

fn parse_font_variant_numeric(input: &str) -> Option<Vec<FontVariantNumericValue>> {
    let mut values = Vec::new();
    let parsed = parse_a_font_variant_numeric(input.as_bytes(), |value| values.push(value.clone()));
    parsed.then_some(values)
}

fn parse_font_variant_ligatures(input: &str) -> Option<Vec<FontVariantLigaturesValue>> {
    let mut values = Vec::new();
    let parsed = parse_a_font_variant_ligatures(input.as_bytes(), |value| values.push(value.clone()));
    parsed.then_some(values)
}

fn parse_font_family_value(input: &str) -> Option<Vec<FontFamilyValue>> {
    let mut family_values = Vec::new();
    let parsed = parse_a_font_family_value(input.as_bytes(), |family_value| {
        family_values.push(family_value.clone());
    });
    parsed.then_some(family_values)
}

fn parse_layer_name(input: &str, allow_blank_layer_name: bool) -> Option<String> {
    let mut name = None;
    let parsed = parse_a_layer_name(input.as_bytes(), allow_blank_layer_name, |parsed_name| {
        name = Some(parsed_name.to_string())
    });
    parsed.then_some(name).flatten()
}

fn parse_import_layer(input: &str) -> Option<String> {
    let mut name = None;
    let parsed = parse_an_import_layer(input.as_bytes(), |parsed_name| name = Some(parsed_name.to_string()));
    parsed.then_some(name).flatten()
}

fn parse_layer_name_list(input: &str) -> Option<Vec<String>> {
    let mut names = Vec::new();
    let parsed = parse_a_layer_name_list(input.as_bytes(), |name| names.push(name.to_string()));
    parsed.then_some(names)
}

fn parse_counter_style_name(input: &str) -> Option<String> {
    let mut name = None;
    let parsed = parse_a_counter_style_name(input.as_bytes(), |parsed_name| name = Some(parsed_name.to_string()));
    parsed.then_some(name).flatten()
}

fn parse_counter_style(
    input: &str,
) -> Option<(
    CssCounterStyleKind,
    CssCounterStyleSymbolsType,
    Option<String>,
    Vec<String>,
)> {
    let mut counter_style = None;
    let mut symbols = Vec::new();
    let parsed = parse_a_counter_style(
        input.as_bytes(),
        |kind, symbols_type, name| counter_style = Some((kind, symbols_type, name.map(ToString::to_string))),
        |symbol| symbols.push(symbol.to_string()),
    );
    parsed
        .then_some(counter_style.map(|(kind, symbols_type, name)| (kind, symbols_type, name, symbols)))
        .flatten()
}

fn parse_nonnegative_integer_symbol_pair(input: &str) -> Option<CssNonnegativeIntegerSymbolPairOrder> {
    let mut order = None;
    let parsed = parse_a_nonnegative_integer_symbol_pair(input.as_bytes(), |parsed_order| order = Some(parsed_order));
    parsed.then_some(order).flatten()
}

fn parse_counter_style_negative_descriptor(input: &str) -> Option<CssCounterStyleNegativeSymbolCount> {
    let mut count = None;
    let parsed = parse_counter_style_negative(input.as_bytes(), |parsed_count| count = Some(parsed_count));
    parsed.then_some(count).flatten()
}

fn parse_counter_style_system_descriptor(input: &str) -> Option<CssCounterStyleSystemKind> {
    let mut system = None;
    let parsed = parse_counter_style_system(input.as_bytes(), |parsed_system| system = Some(parsed_system));
    parsed.then_some(system).flatten()
}

fn parse_counter_style_symbols_descriptor(input: &str) -> Option<usize> {
    let mut count = None;
    let parsed = parse_counter_style_symbols(input.as_bytes(), |parsed_count| count = Some(parsed_count));
    parsed.then_some(count).flatten()
}

fn parse_counter_style_symbol_descriptor(input: &str) -> bool {
    parse_counter_style_symbol(input.as_bytes())
}

fn parse_string_descriptor_value(input: &str) -> bool {
    parse_string_descriptor(input.as_bytes())
}

fn parse_length_descriptor_value(input: &str) -> bool {
    parse_length_descriptor(input.as_bytes())
}

fn parse_positive_percentage_descriptor_value(input: &str) -> bool {
    parse_positive_percentage_descriptor(input.as_bytes())
}

fn parse_page_size_descriptor_value(input: &str) -> bool {
    parse_page_size_descriptor(input.as_bytes())
}

fn parse_optional_declaration_value_descriptor_value(input: &str) -> bool {
    parse_optional_declaration_value_descriptor(input.as_bytes())
}

fn ignore_font_source_callback(_: CssFontSourceKind, _: Option<&str>, _: bool) {}

fn ignore_format_callback(_: &str) {}

fn ignore_calculation_callback(_: CssCalculationNodeKind, _: CssPrimitiveValueKind, _: bool, _: f64, _: u32, _: &[u8]) {
}

fn parse_descriptor_result_value(
    input: &str,
    value_type: CssDescriptorValueType,
) -> Option<(CssDescriptorResultKind, Vec<(String, bool)>)> {
    let mut kind = None;
    let mut items = Vec::new();
    let parsed = parse_descriptor_result(
        value_type,
        input.as_bytes(),
        DescriptorResultCallbacks {
            kind_callback: |parsed_kind| kind = Some(parsed_kind),
            source_callback: |_, source: &str, is_string, _, _, _, _, _| {
                items.push((source.to_string(), is_string));
            },
            calculation_callback: ignore_calculation_callback,
            font_source_callback: ignore_font_source_callback,
            url_callback: |_| {},
            modifier_callback: |_| {},
            format_callback: ignore_format_callback,
            tech_callback: |_| {},
        },
    );

    parsed.then_some(kind.map(|kind| (kind, items))).flatten()
}

fn descriptor_string_value(value: &str) -> RustOwnedDescriptorPrimitiveValue {
    RustOwnedDescriptorPrimitiveValue {
        primitive_kind: CssPrimitiveValueKind::String,
        numeric_value: None,
        source_or_unit: value.to_string(),
        calculation: None,
    }
}

fn descriptor_custom_ident_value(value: &str) -> RustOwnedDescriptorPrimitiveValue {
    RustOwnedDescriptorPrimitiveValue {
        primitive_kind: CssPrimitiveValueKind::CustomIdent,
        numeric_value: None,
        source_or_unit: value.to_string(),
        calculation: None,
    }
}

fn descriptor_integer_value(value: i32, source: &str) -> RustOwnedDescriptorPrimitiveValue {
    RustOwnedDescriptorPrimitiveValue {
        primitive_kind: CssPrimitiveValueKind::Integer,
        numeric_value: Some(f64::from(value)),
        source_or_unit: source.to_string(),
        calculation: None,
    }
}

fn descriptor_number_value(value: f64, source: &str) -> RustOwnedDescriptorPrimitiveValue {
    RustOwnedDescriptorPrimitiveValue {
        primitive_kind: CssPrimitiveValueKind::Number,
        numeric_value: Some(value),
        source_or_unit: source.to_string(),
        calculation: None,
    }
}

fn descriptor_keyword_value(value: &str) -> RustOwnedDescriptorPrimitiveValue {
    RustOwnedDescriptorPrimitiveValue {
        primitive_kind: CssPrimitiveValueKind::Keyword,
        numeric_value: None,
        source_or_unit: value.to_string(),
        calculation: None,
    }
}

fn descriptor_math_value(source: &str) -> RustOwnedDescriptorPrimitiveValue {
    RustOwnedDescriptorPrimitiveValue {
        primitive_kind: CssPrimitiveValueKind::Invalid,
        numeric_value: None,
        source_or_unit: source.to_string(),
        calculation: parse_math_ast(source).map(Box::new),
    }
}

fn descriptor_math_integer_value(source: &str) -> RustOwnedDescriptorPrimitiveValue {
    RustOwnedDescriptorPrimitiveValue {
        primitive_kind: CssPrimitiveValueKind::Invalid,
        numeric_value: None,
        source_or_unit: source.to_string(),
        calculation: parse_math_ast(source).map(Box::new),
    }
}

fn parse_counter_style_range_descriptor(input: &str) -> Option<(CssCounterStyleRangeKind, usize)> {
    let mut range = None;
    let parsed = parse_counter_style_range(input.as_bytes(), |kind, count| range = Some((kind, count)));
    parsed.then_some(range).flatten()
}

fn parse_counter_style_additive_symbols_descriptor(input: &str) -> Option<usize> {
    let mut count = None;
    let parsed = parse_counter_style_additive_symbols(input.as_bytes(), |parsed_count| {
        count = Some(parsed_count);
    });
    parsed.then_some(count).flatten()
}

fn parse_crop_or_cross_descriptor(input: &str) -> Option<CssCropOrCrossKind> {
    let mut kind = None;
    let parsed = parse_crop_or_cross(input.as_bytes(), |parsed_kind| kind = Some(parsed_kind));
    parsed.then_some(kind).flatten()
}

fn parse_font_weight_absolute_pair_descriptor(input: &str) -> Option<usize> {
    let mut count = None;
    let parsed = parse_font_weight_absolute_pair(input.as_bytes(), |parsed_count| {
        count = Some(parsed_count);
    });
    parsed.then_some(count).flatten()
}

fn parse_container_type(input: &str) -> CssContainerTypeValueKind {
    parse_container_type_value(input.as_bytes())
}

fn parse_contain(input: &str) -> CssContainValue {
    parse_contain_value(input.as_bytes())
}

fn parse_scroll_function(input: &str) -> CssScrollFunctionValue {
    parse_scroll_function_value(input.as_bytes())
}

fn parse_view_timeline_inset(input: &str) -> CssViewTimelineInsetValue {
    parse_view_timeline_inset_value(input.as_bytes())
}

fn parse_view_timeline_inset_prefix(input: &str) -> CssViewTimelineInsetValue {
    parse_view_timeline_inset_value_prefix(input.as_bytes())
}

fn parse_view_function(input: &str) -> CssViewFunctionValue {
    parse_view_function_value(input.as_bytes())
}

fn parse_rect(input: &str) -> CssRectValueKind {
    parse_rect_value(input.as_bytes())
}

fn parse_ratio_prefix(input: &str) -> CssRatioValue {
    parse_ratio_value_prefix(input.as_bytes())
}

fn parse_primitive_prefix(input: &str, value_type: CssPrimitiveValueType) -> CssPrimitiveValueKind {
    parse_primitive_value_prefix(input.as_bytes(), value_type, CssPrimitiveValueOptions::default())
}

fn parse_primitive_prefix_with_options(
    input: &str,
    value_type: CssPrimitiveValueType,
    options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    parse_primitive_value_prefix(input.as_bytes(), value_type, options)
}

fn parse_primitive(input: &str, value_type: CssPrimitiveValueType) -> CssPrimitiveValueKind {
    parse_primitive_value(input.as_bytes(), value_type, CssPrimitiveValueOptions::default())
}

fn parse_primitive_with_options(
    input: &str,
    value_type: CssPrimitiveValueType,
    options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    parse_primitive_value(input.as_bytes(), value_type, options)
}

fn parse_easing(input: &str) -> CssEasingValueKind {
    parse_easing_value(input.as_bytes())
}

fn parse_transform_function(input: &str) -> CssTransformFunctionValueKind {
    parse_transform_function_value(input.as_bytes())
}

fn parse_position(input: &str) -> CssPositionValueKind {
    parse_position_value(input.as_bytes(), false)
}

fn parse_background_position(input: &str) -> CssPositionValueKind {
    parse_position_value(input.as_bytes(), true)
}

fn parse_background_position_x(input: &str) -> CssPositionValueKind {
    parse_background_position_longhand_value(input.as_bytes(), true)
}

fn parse_background_position_y(input: &str) -> CssPositionValueKind {
    parse_background_position_longhand_value(input.as_bytes(), false)
}

fn parse_background_size(input: &str) -> CssBackgroundSizeValueKind {
    parse_background_size_value(input.as_bytes())
}

fn parse_repeat_style(input: &str) -> CssRepeatStyleValueKind {
    parse_repeat_style_value(input.as_bytes())
}

fn parse_color_function(input: &str) -> CssColorFunctionValueKind {
    parse_color_function_value(input.as_bytes())
}

fn parse_color(input: &str) -> CssColorValueKind {
    parse_color_value(input.as_bytes(), false)
}

fn parse_quirky_color(input: &str) -> CssColorValueKind {
    parse_color_value(input.as_bytes(), true)
}

fn parse_simple_color(input: &str, allow_quirky_color: bool) -> Option<(CssParsedColorKind, u8, u8, u8, u8, String)> {
    let mut parsed_color = None;
    parse_simple_color_value(
        input.as_bytes(),
        allow_quirky_color,
        |kind, red, green, blue, alpha, name| {
            parsed_color = Some((kind, red, green, blue, alpha, name.to_string()));
        },
    )
    .then_some(parsed_color)
    .flatten()
}

fn parse_image_set(input: &str) -> CssImageSetValueKind {
    parse_image_set_value(input.as_bytes())
}

fn parse_translate(input: &str) -> CssTransformLonghandValueKind {
    parse_translate_value(input.as_bytes())
}

fn parse_scale(input: &str) -> CssTransformLonghandValueKind {
    parse_scale_value(input.as_bytes())
}

fn parse_rotate(input: &str) -> CssTransformLonghandValueKind {
    parse_rotate_value(input.as_bytes())
}

fn parse_transform_origin(input: &str) -> CssTransformLonghandValueKind {
    parse_transform_origin_value(input.as_bytes())
}

fn parse_math_depth(input: &str) -> bool {
    parse_math_depth_value(input.as_bytes())
}

fn parse_aspect_ratio(input: &str) -> bool {
    parse_aspect_ratio_value(input.as_bytes())
}

fn parse_border_radius(input: &str) -> bool {
    parse_border_radius_value(input.as_bytes())
}

fn parse_border_radius_shorthand(input: &str) -> bool {
    parse_border_radius_shorthand_value(input.as_bytes())
}

fn parse_columns(input: &str) -> bool {
    parse_columns_value(input.as_bytes())
}

fn parse_cursor(input: &str) -> bool {
    parse_cursor_value(input.as_bytes())
}

fn parse_box_shadow(input: &str) -> bool {
    parse_shadow_value(PropertyId::BoxShadow, input.as_bytes())
}

fn parse_text_shadow(input: &str) -> bool {
    parse_shadow_value(PropertyId::TextShadow, input.as_bytes())
}

fn parse_overflow_clip_margin(input: &str) -> bool {
    parse_overflow_clip_margin_value(input.as_bytes())
}

fn parse_shape_outside(input: &str) -> bool {
    parse_shape_outside_value(input.as_bytes())
}

fn parse_text_decoration(input: &str) -> bool {
    parse_text_decoration_value(input.as_bytes())
}

fn parse_text_decoration_line(input: &str) -> bool {
    parse_text_decoration_line_value(input.as_bytes())
}

fn parse_list_style(input: &str) -> bool {
    parse_list_style_value(input.as_bytes())
}

fn parse_content(input: &str) -> bool {
    parse_content_value(input.as_bytes())
}

fn parse_flex_shorthand(input: &str) -> bool {
    parse_flex_shorthand_value(input.as_bytes())
}

fn parse_flex_flow(input: &str) -> bool {
    parse_flex_flow_value(input.as_bytes())
}

fn parse_filter_value_list(input: &str) -> bool {
    parse_filter_value_list_value(input.as_bytes())
}

fn parse_fit_content(input: &str) -> CssFitContentValueKind {
    parse_fit_content_value(input.as_bytes())
}

fn parse_basic_shape(input: &str) -> CssBasicShapeValueKind {
    parse_basic_shape_value(input.as_bytes())
}

fn parse_grid_auto_flow(input: &str) -> CssGridAutoFlowValueKind {
    parse_grid_auto_flow_value(input.as_bytes())
}

fn parse_grid_track_placement(input: &str) -> CssGridTrackPlacementValueKind {
    parse_grid_track_placement_value(input.as_bytes())
}

fn parse_grid_auto_track_sizes(input: &str) -> CssGridTrackSizeListValueKind {
    parse_grid_auto_track_sizes_value(input.as_bytes())
}

fn parse_grid_track_size_list(input: &str) -> CssGridTrackSizeListValueKind {
    parse_grid_track_size_list_value(input.as_bytes())
}

fn parse_color_scheme(input: &str) -> (CssColorSchemeValueKind, bool, Vec<String>) {
    let mut schemes = Vec::new();
    let parsed = parse_color_scheme_value(input.as_bytes(), |scheme| schemes.push(scheme.to_string()));
    (parsed.kind, parsed.only, schemes)
}

fn parse_anchor_name_or_scope(input: &str, allow_all: bool) -> (CssAnchorNameOrScopeValueKind, Vec<String>) {
    let mut names = Vec::new();
    let kind = parse_anchor_name_or_scope_value(input.as_bytes(), allow_all, |name| names.push(name.to_string()));
    (kind, names)
}

fn parse_position_anchor(input: &str) -> (CssPositionAnchorValueKind, Option<String>) {
    let mut name = None;
    let kind = parse_position_anchor_value(input.as_bytes(), |parsed_name| name = Some(parsed_name.to_string()));
    (kind, name)
}

fn parse_position_area(input: &str) -> bool {
    parse_position_area_value(input.as_bytes())
}

fn parse_position_try_fallbacks(input: &str) -> bool {
    parse_position_try_fallbacks_value(input.as_bytes())
}

fn parse_timeline_scope(input: &str) -> (CssTimelineScopeValueKind, Vec<String>) {
    let mut names = Vec::new();
    let kind = parse_timeline_scope_value(input.as_bytes(), |name| names.push(name.to_string()));
    (kind, names)
}

fn parse_timeline_name(input: &str) -> (CssTimelineNameValueKind, Vec<(CssTimelineNameItemKind, String)>) {
    let mut names = Vec::new();
    let kind = parse_timeline_name_value(input.as_bytes(), |kind, name| {
        names.push((kind, name.to_string()));
    });
    (kind, names)
}

fn parse_position_try_order(input: &str) -> CssPositionTryOrderValue {
    parse_position_try_order_value(input.as_bytes())
}

fn parse_position_visibility(input: &str) -> CssPositionVisibilityValue {
    parse_position_visibility_value(input.as_bytes())
}

fn parse_paint_order(input: &str) -> CssPaintOrderValue {
    parse_paint_order_value(input.as_bytes())
}

fn parse_place_content(input: &str) -> bool {
    parse_place_content_value(input.as_bytes())
}

fn parse_place_items(input: &str) -> bool {
    parse_place_items_value(input.as_bytes())
}

fn parse_place_self(input: &str) -> bool {
    parse_place_self_value(input.as_bytes())
}

fn parse_text_underline_position(input: &str) -> CssTextUnderlinePositionValue {
    parse_text_underline_position_value(input.as_bytes())
}

fn parse_text_wrap(input: &str) -> CssTextWrapValue {
    parse_text_wrap_value(input.as_bytes())
}

fn parse_text_wrap_mode(input: &str) -> CssTextWrapModeValue {
    parse_text_wrap_mode_value(input.as_bytes())
}

fn parse_text_wrap_style(input: &str) -> CssTextWrapStyleValue {
    parse_text_wrap_style_value(input.as_bytes())
}

fn parse_touch_action(input: &str) -> CssTouchActionValue {
    parse_touch_action_value(input.as_bytes())
}

fn parse_scrollbar_gutter(input: &str) -> CssScrollbarGutterValueKind {
    parse_scrollbar_gutter_value(input.as_bytes())
}

fn parse_stroke_dasharray(input: &str) -> bool {
    parse_stroke_dasharray_value(input.as_bytes())
}

fn parse_quotes(input: &str) -> (CssQuotesValueKind, Vec<String>) {
    let mut strings = Vec::new();
    let kind = parse_quotes_value(input.as_bytes(), |string| strings.push(string.to_string()));
    (kind, strings)
}

fn parse_will_change(input: &str) -> (CssWillChangeValueKind, Vec<(CssWillChangeFeatureKind, String)>) {
    let mut features = Vec::new();
    let kind = parse_will_change_value(input.as_bytes(), |kind, value| {
        features.push((kind, value.to_string()));
    });
    (kind, features)
}

fn parse_transition_property(input: &str) -> (CssTransitionPropertyValueKind, Vec<String>) {
    let mut properties = Vec::new();
    let kind = parse_transition_property_value(input.as_bytes(), |property| {
        properties.push(property.to_string());
    });
    (kind, properties)
}

fn parse_transition_behavior(input: &str) -> (CssTransitionBehaviorValueKind, Vec<CssTransitionBehaviorItemKind>) {
    let mut behaviors = Vec::new();
    let kind = parse_transition_behavior_value(input.as_bytes(), |behavior| {
        behaviors.push(behavior);
    });
    (kind, behaviors)
}

fn parse_animation_name(input: &str) -> (CssAnimationNameValueKind, Vec<(CssAnimationNameItemKind, String)>) {
    let mut names = Vec::new();
    let kind = parse_animation_name_value(input.as_bytes(), |kind, value| {
        names.push((kind, value.to_string()));
    });
    (kind, names)
}

fn parse_view_transition_name(input: &str) -> (CssViewTransitionNameValueKind, Option<String>) {
    let mut name = None;
    let kind = parse_view_transition_name_value(input.as_bytes(), |value| {
        name = Some(value.to_string());
    });
    (kind, name)
}

fn parse_white_space_trim(input: &str) -> CssWhiteSpaceTrimValue {
    parse_white_space_trim_value(input.as_bytes())
}

fn parse_namespace_rule_prelude(input: &str) -> Option<(Option<String>, String)> {
    let mut prefix = None;
    let mut namespace_uri = None;
    let parsed = parse_a_namespace_rule_prelude(
        input.as_bytes(),
        |parsed_prefix| prefix = Some(parsed_prefix.to_string()),
        |parsed_namespace_uri| namespace_uri = Some(parsed_namespace_uri.to_string()),
    );
    parsed.then(|| (prefix, namespace_uri.expect("namespace URI must be parsed")))
}

fn parse_font_feature_values_family_names(input: &str) -> Option<Vec<String>> {
    let mut family_names = Vec::new();
    let parsed = parse_font_feature_values_family_name_list(input.as_bytes(), |family_name| {
        family_names.push(family_name.to_string());
    });
    parsed.then_some(family_names)
}

fn parse_font_feature_values_feature_values(input: &str) -> Option<Vec<u32>> {
    let mut values = Vec::new();
    let parsed = parse_font_feature_values_feature_value(input.as_bytes(), |value| {
        values.push(value);
    });
    parsed.then_some(values)
}

fn parse_family_name(input: &str) -> Option<(String, bool)> {
    let mut family_name = None;
    let parsed = parse_a_family_name(input.as_bytes(), |name, is_string| {
        family_name = Some((name.to_string(), is_string));
    });
    parsed.then_some(family_name).flatten()
}

fn parse_container_rule_prelude_items(input: &str) -> Option<Vec<(Option<String>, Option<String>)>> {
    let mut conditions = Vec::new();
    let parsed = parse_container_rule_prelude(input.as_bytes(), |container_name, container_query| {
        conditions.push((
            container_name.map(ToString::to_string),
            container_query.map(ToString::to_string),
        ));
    });
    parsed.then_some(conditions)
}

fn parse_value_type(input: &str, value_type_id: ValueTypeId) -> CssValueTypeSyntaxKind {
    component_values_parse_as_value_type(value_type_id, &parse(input))
}

fn parse_property_keyword(property_ids: &[PropertyId], keyword: &str) -> Option<(PropertyId, String)> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    let mut parsed_keyword = None;
    super::parse_property_keyword_value(&property_ids, keyword.as_bytes(), |property_id, keyword| {
        parsed_keyword = Some((
            crate::generated_properties::property_id_from_u16(property_id).unwrap(),
            keyword.to_string(),
        ));
    });
    parsed_keyword
}

fn property_accepting_type(property_ids: &[PropertyId], value_type: &str) -> Option<PropertyId> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    let mut accepted_property = None;
    super::property_accepting_type(&property_ids, value_type.as_bytes(), |property_id| {
        accepted_property = Some(crate::generated_properties::property_id_from_u16(property_id).unwrap());
    });
    accepted_property
}

fn parse_property_custom_ident(property_ids: &[PropertyId], input: &str) -> Option<(PropertyId, String)> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    let mut parsed_custom_ident = None;
    super::parse_property_custom_ident_value(&property_ids, input.as_bytes(), |property_id, custom_ident| {
        parsed_custom_ident = Some((
            crate::generated_properties::property_id_from_u16(property_id).unwrap(),
            custom_ident.to_string(),
        ));
    });
    parsed_custom_ident
}

fn parse_generated_property(
    property_ids: &[PropertyId],
    input: &str,
) -> Option<(CssGeneratedPropertyValueKind, PropertyId, String, String)> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    let mut parsed_value = None;
    parse_generated_property_value(
        &property_ids,
        input.as_bytes(),
        |kind, property_id, value, value_type| {
            parsed_value = Some((
                kind,
                crate::generated_properties::property_id_from_u16(property_id).unwrap(),
                String::from_utf8(value.to_vec()).unwrap(),
                value_type.to_string(),
            ));
        },
    );
    parsed_value
}

#[derive(Debug, PartialEq)]
struct ParsedStyleValue {
    kind: CssStyleValueKind,
    property_id: PropertyId,
    primitive_kind: CssPrimitiveValueKind,
    numeric_value: Option<f64>,
    secondary_numeric_value: Option<f64>,
    color: Option<(u8, u8, u8, u8)>,
    value: String,
    value_type: String,
}

fn parse_style_value(property_ids: &[PropertyId], input: &str) -> Option<ParsedStyleValue> {
    parse_style_value_with_options(property_ids, input, CssPrimitiveValueOptions::default())
}

fn parse_style_value_with_options(
    property_ids: &[PropertyId],
    input: &str,
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<ParsedStyleValue> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    let mut parsed_value = None;
    parse_style_value_for_property_with_options(
        &property_ids,
        input.as_bytes(),
        primitive_value_options,
        |kind,
         property_id,
         primitive_kind,
         has_numeric_value,
         numeric_value,
         has_secondary_numeric_value,
         secondary_numeric_value,
         color_red,
         color_green,
         color_blue,
         color_alpha,
         value,
         value_type| {
            parsed_value = Some(ParsedStyleValue {
                kind,
                property_id: crate::generated_properties::property_id_from_u16(property_id).unwrap(),
                primitive_kind,
                numeric_value: has_numeric_value.then_some(numeric_value),
                secondary_numeric_value: has_secondary_numeric_value.then_some(secondary_numeric_value),
                color: (kind == CssStyleValueKind::Color).then_some((color_red, color_green, color_blue, color_alpha)),
                value: String::from_utf8(value.to_vec()).unwrap(),
                value_type: value_type.to_string(),
            });
        },
    );
    parsed_value
}

fn emit_style_value(style_value: &RustOwnedStyleValue) -> Option<ParsedStyleValue> {
    let mut parsed_value = None;
    emit_rust_owned_style_value(
        style_value,
        &mut |kind,
              property_id,
              primitive_kind,
              has_numeric_value,
              numeric_value,
              has_secondary_numeric_value,
              secondary_numeric_value,
              color_red,
              color_green,
              color_blue,
              color_alpha,
              value,
              value_type| {
            parsed_value = Some(ParsedStyleValue {
                kind,
                property_id: crate::generated_properties::property_id_from_u16(property_id).unwrap(),
                primitive_kind,
                numeric_value: has_numeric_value.then_some(numeric_value),
                secondary_numeric_value: has_secondary_numeric_value.then_some(secondary_numeric_value),
                color: (kind == CssStyleValueKind::Color).then_some((color_red, color_green, color_blue, color_alpha)),
                value: String::from_utf8(value.to_vec()).unwrap(),
                value_type: value_type.to_string(),
            });
        },
    );
    parsed_value
}

fn parse_rust_owned_style_value(property_ids: &[PropertyId], input: &str) -> Option<RustOwnedStyleValue> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    match parse_rust_owned_style_value_for_property(&property_ids, input.as_bytes()) {
        RustOwnedStyleValueParseResult::Parsed(value) => Some(value),
        RustOwnedStyleValueParseResult::Invalid => None,
    }
}

fn parse_rust_owned_image(input: &str) -> Option<RustOwnedImage> {
    match rust_owned_image_style_value_kind(input.as_bytes(), input)? {
        RustOwnedStyleValueKind::Image(image) => Some(image),
        _ => None,
    }
}

fn parse_coordinating_shorthand(property_ids: &[PropertyId], input: &str) -> Option<Vec<(usize, PropertyId, String)>> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    let mut items = Vec::new();
    parse_coordinating_value_list_shorthand(&property_ids, input.as_bytes(), |layer_index, property_id, value| {
        items.push((
            layer_index,
            crate::generated_properties::property_id_from_u16(property_id).unwrap(),
            value.to_string(),
        ));
    })
    .then_some(items)
}

fn parse_rust_owned_coordinating_shorthand(
    property_ids: &[PropertyId],
    input: &str,
) -> Option<Vec<RustOwnedCoordinatingValueListShorthandItem>> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    parse_rust_owned_coordinating_value_list_shorthand(&property_ids, input.as_bytes())
}

fn parse_layer_shorthand_items(property_id: PropertyId, input: &str) -> Option<Vec<(usize, PropertyId, String)>> {
    let mut items = Vec::new();
    parse_layer_shorthand(
        property_id as u16,
        input.as_bytes(),
        |layer_index, property_id, value| {
            items.push((
                layer_index,
                crate::generated_properties::property_id_from_u16(property_id).unwrap(),
                value.to_string(),
            ));
        },
    )
    .then_some(items)
}

fn parse_font_shorthand_items(input: &str) -> Option<Vec<(PropertyId, String)>> {
    let mut items = Vec::new();
    parse_font_shorthand(input.as_bytes(), |property_id, value| {
        items.push((
            crate::generated_properties::property_id_from_u16(property_id).unwrap(),
            value.to_string(),
        ));
    })
    .then_some(items)
}

fn parse_grid_placement_shorthand_items(property_id: PropertyId, input: &str) -> Option<Vec<(PropertyId, String)>> {
    let mut items = Vec::new();
    parse_grid_placement_shorthand(property_id as u16, input.as_bytes(), |property_id, value| {
        items.push((
            crate::generated_properties::property_id_from_u16(property_id).unwrap(),
            value.to_string(),
        ));
    })
    .then_some(items)
}

fn parse_grid_template_shorthand_items(property_id: PropertyId, input: &str) -> Option<Vec<(PropertyId, String)>> {
    let mut items = Vec::new();
    parse_grid_template_shorthand(property_id as u16, input.as_bytes(), |property_id, value| {
        items.push((
            crate::generated_properties::property_id_from_u16(property_id).unwrap(),
            value.to_string(),
        ));
    })
    .then_some(items)
}

fn parse_positional_shorthand(property_id: PropertyId, input: &str) -> Option<Vec<(usize, String)>> {
    let mut items = Vec::new();
    parse_positional_value_list_shorthand(property_id as u16, input.as_bytes(), |index, value| {
        items.push((index, value.to_string()));
    })
    .then_some(items)
}

fn parse_rust_owned_positional_shorthand(
    property_id: PropertyId,
    input: &str,
) -> Option<Vec<RustOwnedPositionalValueListShorthandItem>> {
    parse_rust_owned_positional_value_list_shorthand(
        property_id as u16,
        input.as_bytes(),
        CssPrimitiveValueOptions::default(),
    )
}

fn parse_rust_owned_positional_shorthand_with_options(
    property_id: PropertyId,
    input: &str,
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<Vec<RustOwnedPositionalValueListShorthandItem>> {
    parse_rust_owned_positional_value_list_shorthand(property_id as u16, input.as_bytes(), primitive_value_options)
}

#[derive(Debug, PartialEq)]
struct PropertyNumericMetadata {
    property_id: PropertyId,
    minimum: f64,
    maximum: f64,
    percentage_range: Option<(f64, f64)>,
    percentages_resolve_to_value_type: bool,
}

fn property_numeric_metadata(property_ids: &[PropertyId], value_type: &str) -> Option<PropertyNumericMetadata> {
    let property_ids: Vec<u16> = property_ids.iter().map(|property_id| *property_id as u16).collect();
    let mut metadata = None;
    super::property_numeric_metadata(
        &property_ids,
        value_type.as_bytes(),
        |property_id,
         minimum,
         maximum,
         has_percentage_range,
         percentage_minimum,
         percentage_maximum,
         percentages_resolve_to_value_type| {
            metadata = Some(PropertyNumericMetadata {
                property_id: crate::generated_properties::property_id_from_u16(property_id).unwrap(),
                minimum,
                maximum,
                percentage_range: has_percentage_range.then_some((percentage_minimum, percentage_maximum)),
                percentages_resolve_to_value_type,
            });
        },
    );
    metadata
}

#[test]
fn generated_property_metadata_matches_property_ids() {
    assert_eq!(property_id_from_string("color"), Some(PropertyId::Color));
    assert_eq!(property_id_from_string("COLOR"), Some(PropertyId::Color));
    assert_eq!(property_id_from_string("--ladybird"), Some(PropertyId::Custom));
    assert_eq!(
        property_id_from_string("-webkit-animation-name"),
        Some(PropertyId::AnimationName)
    );
    assert_eq!(property_name(PropertyId::TextWrap), "text-wrap");
}

#[test]
fn generated_pseudo_class_metadata_matches_pseudo_class_ids() {
    assert_eq!(pseudo_class_id_from_string("hover"), Some(PseudoClassId::Hover));
    assert_eq!(
        pseudo_class_id_from_string("-webkit-autofill"),
        Some(PseudoClassId::Autofill)
    );
    assert_eq!(pseudo_class_name(PseudoClassId::NthChild), "nth-child");

    let metadata = pseudo_class_metadata(PseudoClassId::Hover);
    assert_eq!(metadata.parameter_type, PseudoClassParameterType::None);
    assert!(!metadata.is_valid_as_function);
    assert!(metadata.is_valid_as_identifier);

    let metadata = pseudo_class_metadata(PseudoClassId::NthChild);
    assert_eq!(metadata.parameter_type, PseudoClassParameterType::AnPlusBOf);
    assert!(metadata.is_valid_as_function);
    assert!(!metadata.is_valid_as_identifier);

    let metadata = pseudo_class_metadata(PseudoClassId::Host);
    assert_eq!(metadata.parameter_type, PseudoClassParameterType::CompoundSelector);
    assert!(metadata.is_valid_as_function);
    assert!(metadata.is_valid_as_identifier);
}

#[test]
fn generated_pseudo_element_metadata_matches_pseudo_element_ids() {
    assert_eq!(pseudo_element_id_from_string("before"), Some(PseudoElementId::Before));
    assert_eq!(
        aliased_pseudo_element_id_from_string("-webkit-slider-thumb"),
        Some(PseudoElementId::SliderThumb)
    );
    assert_eq!(pseudo_element_name(PseudoElementId::Slotted), "slotted");

    let metadata = pseudo_element_metadata(PseudoElementId::Before);
    assert_eq!(metadata.parameter_type, PseudoElementParameterType::None);
    assert!(!metadata.is_valid_as_function);
    assert!(metadata.is_valid_as_identifier);

    let metadata = pseudo_element_metadata(PseudoElementId::Slotted);
    assert_eq!(metadata.parameter_type, PseudoElementParameterType::CompoundSelector);
    assert!(metadata.is_valid_as_function);
    assert!(!metadata.is_valid_as_identifier);

    let metadata = pseudo_element_metadata(PseudoElementId::Part);
    assert_eq!(metadata.parameter_type, PseudoElementParameterType::IdentList);
    assert!(metadata.is_valid_as_function);
    assert!(!metadata.is_valid_as_identifier);
}

#[test]
fn parses_basic_selector_syntax() {
    let selectors = parse_selector_list("main#app.content > article + .card ~ *").unwrap();
    assert_eq!(selectors.len(), 1);
    let selector = &selectors[0];
    assert_eq!(selector.compound_selectors.len(), 4);

    assert_eq!(selector.compound_selectors[0].combinator, SelectorCombinator::None);
    assert!(matches!(
        selector.compound_selectors[0].simple_selectors.as_slice(),
        [
            SimpleSelectorSyntax::TagName(tag_name),
            SimpleSelectorSyntax::Id(id),
            SimpleSelectorSyntax::Class(class_name)
        ] if tag_name.name == "main" && id == "app" && class_name == "content"
    ));
    assert_eq!(
        selector.compound_selectors[1].combinator,
        SelectorCombinator::ImmediateChild
    );
    assert_eq!(
        selector.compound_selectors[2].combinator,
        SelectorCombinator::NextSibling
    );
    assert_eq!(
        selector.compound_selectors[3].combinator,
        SelectorCombinator::SubsequentSibling
    );
    assert!(matches!(
        selector.compound_selectors[3].simple_selectors.as_slice(),
        [SimpleSelectorSyntax::Universal(_)]
    ));

    assert!(parse_selector_list("> article").is_none());
    let relative = parse_relative_selector_list("> article").unwrap();
    assert_eq!(
        relative[0].compound_selectors[0].combinator,
        SelectorCombinator::ImmediateChild
    );
}

#[test]
fn parses_selector_qualified_names() {
    let selectors = parse_selector_list_with_namespaces("svg|circle, *|rect, |section", &["svg"]).unwrap();
    assert_eq!(selectors.len(), 3);

    let [SimpleSelectorSyntax::TagName(first)] = selectors[0].compound_selectors[0].simple_selectors.as_slice() else {
        panic!("expected tag-name selector");
    };
    assert_eq!(first.namespace_type, NamespaceType::Named);
    assert_eq!(first.namespace, "svg");
    assert_eq!(first.name, "circle");

    let [SimpleSelectorSyntax::TagName(second)] = selectors[1].compound_selectors[0].simple_selectors.as_slice() else {
        panic!("expected tag-name selector");
    };
    assert_eq!(second.namespace_type, NamespaceType::Any);

    let [SimpleSelectorSyntax::TagName(third)] = selectors[2].compound_selectors[0].simple_selectors.as_slice() else {
        panic!("expected tag-name selector");
    };
    assert_eq!(third.namespace_type, NamespaceType::None);

    assert!(parse_selector_list_with_namespaces("svg|circle", &[]).is_none());
}

#[test]
fn parses_attribute_selector_syntax() {
    let selectors = parse_selector_list(r#"[data-state~="open" i][href]"#).unwrap();
    let simple_selectors = &selectors[0].compound_selectors[0].simple_selectors;
    assert_eq!(simple_selectors.len(), 2);

    let SimpleSelectorSyntax::Attribute(first) = &simple_selectors[0] else {
        panic!("expected attribute selector");
    };
    assert_eq!(first.match_type, super::AttributeMatchType::ContainsWord);
    assert_eq!(first.qualified_name.name, "data-state");
    assert_eq!(first.value, "open");
    assert_eq!(first.case_type, super::AttributeCaseType::CaseInsensitiveMatch);

    assert!(parse_selector_list("[data-state=]").is_none());
    assert!(parse_selector_list("[data-state=open q]").is_none());
}

#[test]
fn parses_pseudo_class_selector_syntax() {
    let selectors = parse_selector_list(":is(article, section)::before").unwrap();
    let simple_selectors = &selectors[0].compound_selectors[0].simple_selectors;
    let SimpleSelectorSyntax::PseudoClass(pseudo_class) = &simple_selectors[0] else {
        panic!("expected pseudo-class selector");
    };
    assert_eq!(pseudo_class.pseudo_class_id, PseudoClassId::Is);
    assert!(pseudo_class.is_forgiving);
    assert_eq!(pseudo_class.argument_selector_list.len(), 2);

    let SimpleSelectorSyntax::PseudoElement(pseudo_element) = &simple_selectors[1] else {
        panic!("expected pseudo-element selector");
    };
    assert_eq!(pseudo_element.pseudo_element_id, PseudoElementId::Before);

    let selectors = parse_selector_list(":nth-child(2n + 1 of .item)").unwrap();
    let [SimpleSelectorSyntax::PseudoClass(pseudo_class)] =
        selectors[0].compound_selectors[0].simple_selectors.as_slice()
    else {
        panic!("expected pseudo-class selector");
    };
    assert_eq!(pseudo_class.pseudo_class_id, PseudoClassId::NthChild);
    assert_eq!(pseudo_class.an_plus_b_pattern.unwrap().step_size, 2);
    assert_eq!(pseudo_class.an_plus_b_pattern.unwrap().offset, 1);
    assert_eq!(pseudo_class.argument_selector_list.len(), 1);

    let selectors = parse_selector_list(":nth-child(4n-1)").unwrap();
    let [SimpleSelectorSyntax::PseudoClass(pseudo_class)] =
        selectors[0].compound_selectors[0].simple_selectors.as_slice()
    else {
        panic!("expected pseudo-class selector");
    };
    assert_eq!(pseudo_class.an_plus_b_pattern.unwrap().step_size, 4);
    assert_eq!(pseudo_class.an_plus_b_pattern.unwrap().offset, -1);

    assert!(parse_selector_list(":has(:has(.nested))").is_none());
}

#[test]
fn parses_pseudo_element_selector_syntax() {
    let selectors = parse_selector_list("::part(foo bar)::before").unwrap();
    let simple_selectors = &selectors[0].compound_selectors[0].simple_selectors;
    let SimpleSelectorSyntax::PseudoElement(part) = &simple_selectors[0] else {
        panic!("expected pseudo-element selector");
    };
    assert_eq!(part.pseudo_element_id, PseudoElementId::Part);
    assert_eq!(
        part.value,
        PseudoElementSelectorValue::IdentList(vec!["foo".to_string(), "bar".to_string()])
    );

    let SimpleSelectorSyntax::PseudoElement(before) = &simple_selectors[1] else {
        panic!("expected pseudo-element selector");
    };
    assert_eq!(before.pseudo_element_id, PseudoElementId::Before);

    let selectors = parse_selector_list("::-webkit-slider-thumb").unwrap();
    let [SimpleSelectorSyntax::PseudoElement(slider_thumb)] =
        selectors[0].compound_selectors[0].simple_selectors.as_slice()
    else {
        panic!("expected pseudo-element selector");
    };
    assert_eq!(slider_thumb.pseudo_element_id, PseudoElementId::SliderThumb);
    assert_eq!(slider_thumb.name.as_deref(), Some("-webkit-slider-thumb"));

    let selectors = parse_selector_list("::-webkit-unknown").unwrap();
    let [SimpleSelectorSyntax::PseudoElement(unknown_webkit)] =
        selectors[0].compound_selectors[0].simple_selectors.as_slice()
    else {
        panic!("expected pseudo-element selector");
    };
    assert_eq!(unknown_webkit.pseudo_element_id, PseudoElementId::UnknownWebKit);

    assert!(parse_selector_list("::before::after").is_none());
    assert!(parse_selector_list("::part(foo)::part(bar)").is_none());
}

#[test]
fn parses_forgiving_selector_list_syntax() {
    let selectors = parse_forgiving_selector_list(".valid, > invalid, .also-valid").unwrap();
    assert_eq!(selectors.len(), 3);
    assert!(matches!(
        selectors[1].compound_selectors[0].simple_selectors.as_slice(),
        [SimpleSelectorSyntax::Invalid(_)]
    ));
}

#[test]
fn emits_selector_syntax_events() {
    let mut events = Vec::new();
    let mut names = Vec::new();
    let mut component_values = Vec::new();
    assert!(super::parse_a_selector_list(
        b"main.content:is(article, section)::before",
        SelectorType::Standalone,
        SelectorParsingMode::Normal,
        Vec::new(),
        |event| {
            names.push(event_string(event.name_ptr, event.name_len).to_string());
            events.push(event);
        },
        |component_value| component_values.push(component_value),
    ));

    assert!(component_values.is_empty());
    assert_eq!(events[0].kind, CssSelectorEventKind::SelectorListStart);
    assert_eq!(events[1].kind, CssSelectorEventKind::SelectorStart);
    assert_eq!(events[2].kind, CssSelectorEventKind::CompoundSelectorStart);
    assert_eq!(events[3].simple_selector_kind, CssSimpleSelectorKind::TagName);
    assert_eq!(names[3], "main");
    assert_eq!(events[4].simple_selector_kind, CssSimpleSelectorKind::Class);
    assert_eq!(names[4], "content");
    assert_eq!(events[5].kind, CssSelectorEventKind::PseudoClassSelectorStart);
    assert_eq!(events[5].pseudo_class_id, PseudoClassId::Is as u8);
    assert!(events[5].is_forgiving);
    assert!(
        events
            .iter()
            .any(|event| event.kind == CssSelectorEventKind::SelectorListEnd)
    );
    assert!(
        events
            .iter()
            .any(|event| event.kind == CssSelectorEventKind::PseudoElementSelectorStart
                && event.pseudo_element_id == PseudoElementId::Before as u8)
    );
    assert_eq!(events.last().unwrap().kind, CssSelectorEventKind::SelectorListEnd);
}

#[test]
fn generated_property_metadata_knows_longhands() {
    assert_eq!(
        longhands_for_shorthand(PropertyId::TextWrap),
        &[PropertyId::TextWrapMode, PropertyId::TextWrapStyle]
    );
    assert_eq!(
        longhands_for_shorthand(PropertyId::Transition),
        &[
            PropertyId::TransitionProperty,
            PropertyId::TransitionDuration,
            PropertyId::TransitionTimingFunction,
            PropertyId::TransitionDelay,
            PropertyId::TransitionBehavior,
        ]
    );
    assert_eq!(
        longhands_for_shorthand(PropertyId::Outline),
        &[
            PropertyId::OutlineColor,
            PropertyId::OutlineStyle,
            PropertyId::OutlineWidth
        ]
    );
    assert_eq!(
        longhands_for_shorthand(PropertyId::BorderBottom),
        &[
            PropertyId::BorderBottomWidth,
            PropertyId::BorderBottomStyle,
            PropertyId::BorderBottomColor,
        ]
    );
}

#[test]
fn ordinary_properties_use_rust_owned_whole_grammar() {
    assert!(super::property_uses_rust_owned_whole_grammar(PropertyId::Width));
    assert!(super::property_uses_rust_owned_whole_grammar(PropertyId::Outline));
    assert!(!super::property_uses_rust_owned_whole_grammar(PropertyId::All));
    assert!(!super::property_uses_rust_owned_whole_grammar(PropertyId::Custom));
}

#[test]
fn generated_property_metadata_knows_accepted_keywords_and_types() {
    assert!(property_accepts_keyword(PropertyId::Display, "block"));
    assert!(property_accepts_keyword(
        PropertyId::AnimationDirection,
        "alternate-reverse"
    ));
    assert!(property_accepts_keyword(PropertyId::ImageRendering, "optimizequality"));
    assert!(property_accepts_keyword(PropertyId::TextWrapMode, "nowrap"));
    assert!(!property_accepts_keyword(
        PropertyId::AnimationDirection,
        "allow-discrete"
    ));

    assert!(property_accepts_value_type(PropertyId::Color, PropertyValueType::Color));
    assert!(property_accepts_value_type(
        PropertyId::AnimationDuration,
        PropertyValueType::Time
    ));
    assert!(!property_accepts_value_type(
        PropertyId::Color,
        PropertyValueType::Length
    ));
}

#[test]
fn generated_property_metadata_knows_ranges_and_aliases() {
    assert_eq!(
        property_accepted_range_by_value_type(PropertyId::AnimationDuration, PropertyValueType::Time),
        Some(PropertyNumericRange {
            minimum: Some(0.0),
            maximum: None,
        })
    );
    assert_eq!(
        property_resolves_percentages_relative_to(PropertyId::BackgroundPositionX),
        Some(PropertyValueType::Length)
    );
    assert_eq!(
        resolve_legacy_value_alias(PropertyId::Overflow, "overlay"),
        Some("auto")
    );
    assert_eq!(property_custom_ident_blacklist(PropertyId::AnimationName), &["none"]);
}

#[test]
fn generated_descriptor_metadata_knows_supported_descriptors() {
    assert!(at_rule_supports_descriptor(
        AtRuleId::CounterStyle,
        DescriptorId::AdditiveSymbols
    ));
    assert!(at_rule_supports_descriptor(AtRuleId::Function, DescriptorId::Custom));
    assert!(!at_rule_supports_descriptor(
        AtRuleId::CounterStyle,
        DescriptorId::FontFamily
    ));
    assert!(!at_rule_supports_descriptor(AtRuleId::FontFace, DescriptorId::Custom));
}

#[test]
fn generated_descriptor_metadata_knows_asf_support() {
    assert!(descriptor_allows_arbitrary_substitution_functions(
        AtRuleId::Function,
        DescriptorId::Result
    ));
    assert!(descriptor_allows_arbitrary_substitution_functions(
        AtRuleId::Function,
        DescriptorId::Custom
    ));
    assert!(!descriptor_allows_arbitrary_substitution_functions(
        AtRuleId::Property,
        DescriptorId::InitialValue
    ));
}

#[test]
fn generated_descriptor_metadata_knows_syntax_options() {
    let mut syntax = Vec::new();
    assert!(for_each_descriptor_syntax(
        AtRuleId::FontFace,
        DescriptorId::FontWeight,
        |item| syntax.push(item)
    ));
    assert!(matches!(syntax[0], DescriptorSyntax::Keyword("auto")));
    assert!(matches!(
        syntax[1],
        DescriptorSyntax::ValueType(CssDescriptorValueType::FontWeightAbsolutePair)
    ));

    syntax.clear();
    assert!(for_each_descriptor_syntax(
        AtRuleId::Page,
        DescriptorId::Margin,
        |item| syntax.push(item)
    ));
    assert!(matches!(syntax[0], DescriptorSyntax::Property(PropertyId::Margin)));
}

#[test]
fn parses_descriptor_results_with_rust_owned_dispatch() {
    assert_eq!(
        parse_descriptor_result_value("2 \"II\", \"I\" 1", CssDescriptorValueType::CounterStyleAdditiveSymbols),
        Some((
            CssDescriptorResultKind::CounterStyleAdditiveSymbols,
            vec![("II".to_string(), false), ("I".to_string(), false)]
        ))
    );
    assert_eq!(
        parse_descriptor_result_value("\"-\" \"\"", CssDescriptorValueType::CounterStyleNegative),
        Some((
            CssDescriptorResultKind::CounterStyleNegative,
            vec![("-".to_string(), false), ("".to_string(), false)]
        ))
    );
    assert_eq!(
        parse_descriptor_result_value(
            "url(example.woff2), local(Example)",
            CssDescriptorValueType::FontSrcList
        ),
        Some((
            CssDescriptorResultKind::FontSrcList,
            vec![
                ("url(example.woff2)".to_string(), false),
                ("local(Example)".to_string(), false)
            ]
        ))
    );
    assert_eq!(
        parse_descriptor_result_value("normal bold", CssDescriptorValueType::FontWeightAbsolutePair),
        Some((
            CssDescriptorResultKind::FontWeightAbsolutePair,
            vec![("normal".to_string(), false), ("bold".to_string(), false)]
        ))
    );
    assert_eq!(
        parse_descriptor_result_value("\"hello\"", CssDescriptorValueType::String),
        Some((CssDescriptorResultKind::String, vec![("hello".to_string(), true)]))
    );
    assert_eq!(
        parse_descriptor_result_value("example", CssDescriptorValueType::FamilyName),
        Some((
            CssDescriptorResultKind::FamilyName,
            vec![("example".to_string(), false)]
        ))
    );
    assert_eq!(
        parse_descriptor_result_value("\"Bongo Sans\"", CssDescriptorValueType::FamilyName),
        Some((
            CssDescriptorResultKind::FamilyName,
            vec![("Bongo Sans".to_string(), true)]
        ))
    );
    assert_eq!(
        parse_descriptor_result_value("calc(50% + 25%)", CssDescriptorValueType::PositivePercentage),
        Some((
            CssDescriptorResultKind::PositivePercentage,
            vec![("calc(50% + 25%)".to_string(), false)]
        ))
    );
}

#[test]
fn parses_property_keyword_values_with_generated_metadata() {
    assert_eq!(
        parse_property_keyword(&[PropertyId::Color, PropertyId::Display], "block"),
        Some((PropertyId::Display, "block".to_string()))
    );
    assert_eq!(
        parse_property_keyword(&[PropertyId::Overflow], "overlay"),
        Some((PropertyId::Overflow, "auto".to_string()))
    );
    assert_eq!(
        parse_property_keyword(&[PropertyId::ImageRendering], "optimizequality"),
        Some((PropertyId::ImageRendering, "optimizequality".to_string()))
    );
    assert_eq!(
        parse_property_keyword(&[PropertyId::AnimationDirection], "allow-discrete"),
        None
    );
}

#[test]
fn selects_property_value_types_with_generated_metadata() {
    assert_eq!(
        property_accepting_type(&[PropertyId::AnimationDirection, PropertyId::Color], "Color"),
        Some(PropertyId::Color)
    );
    assert_eq!(
        property_accepting_type(&[PropertyId::Color, PropertyId::AnimationDuration], "Time"),
        Some(PropertyId::AnimationDuration)
    );
    assert_eq!(property_accepting_type(&[PropertyId::Color], "OpenTypeTag"), None);
}

#[test]
fn parses_property_custom_ident_values_with_generated_metadata() {
    assert_eq!(
        parse_property_custom_ident(&[PropertyId::Color, PropertyId::AnimationName], "slide"),
        Some((PropertyId::AnimationName, "slide".to_string()))
    );
    assert_eq!(parse_property_custom_ident(&[PropertyId::AnimationName], "none"), None);
    assert_eq!(parse_property_custom_ident(&[PropertyId::Color], "slide"), None);
}

#[test]
fn parses_generated_property_values_with_generated_metadata() {
    assert_eq!(
        parse_generated_property(&[PropertyId::Color, PropertyId::Display], "block"),
        Some((
            CssGeneratedPropertyValueKind::Keyword,
            PropertyId::Display,
            "block".to_string(),
            String::new()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::Overflow], "overlay"),
        Some((
            CssGeneratedPropertyValueKind::Keyword,
            PropertyId::Overflow,
            "auto".to_string(),
            String::new()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::AnimationName], "slide"),
        Some((
            CssGeneratedPropertyValueKind::CustomIdent,
            PropertyId::AnimationName,
            "slide".to_string(),
            "CustomIdent".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::Color], "red"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::Color,
            String::new(),
            "Color".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::FontWeight], "bold"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::FontWeight,
            String::new(),
            "FontWeightAbsolute".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::Color, PropertyId::BackgroundPositionX], "10px"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::BackgroundPositionX,
            String::new(),
            "Length".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::FontStyle], "oblique 10deg"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::FontStyle,
            String::new(),
            "FontStyle".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::FontVariantNumeric], "oldstyle-nums tabular-nums"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::FontVariantNumeric,
            String::new(),
            "FontVariantNumeric".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::ListStyleType], "symbols(\"*\" \"**\")"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::ListStyleType,
            String::new(),
            "CounterStyle".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::Transform], "translateX(10px) scale(2)"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::Transform,
            String::new(),
            "TransformList".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::MaskImage], "url(foo.png)"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::MaskImage,
            String::new(),
            "Image".to_string()
        ))
    );
    assert_eq!(
        parse_generated_property(&[PropertyId::MaskImage], "url(#mask)"),
        Some((
            CssGeneratedPropertyValueKind::ValueType,
            PropertyId::MaskImage,
            String::new(),
            "Url".to_string()
        ))
    );
    assert_eq!(parse_generated_property(&[PropertyId::Color], "10px"), None);
}

#[test]
fn parses_style_values_with_rust_owned_ast() {
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Color], "red"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Color,
            value: RustOwnedStyleValueKind::Color(RustOwnedColor::Simple {
                kind: CssParsedColorKind::Rgba,
                red: 255,
                green: 0,
                blue: 0,
                alpha: 255,
                name: Some("red".to_string()),
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::Color], "color-mix(in oklab, red 40%, blue)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Color,
            value: RustOwnedStyleValueKind::Color(RustOwnedColor::Function {
                name,
                source,
                ..
            }),
        }) if name == "color-mix" && source == "color-mix(in oklab, red 40%, blue)"
    ));
    for property_id in [
        PropertyId::AccentColor,
        PropertyId::BorderBlockEndColor,
        PropertyId::BorderBlockStartColor,
        PropertyId::BorderInlineEndColor,
        PropertyId::BorderInlineStartColor,
        PropertyId::CaretColor,
        PropertyId::FloodColor,
        PropertyId::OutlineColor,
        PropertyId::StopColor,
        PropertyId::TextDecorationColor,
    ] {
        assert_eq!(
            parse_rust_owned_style_value(&[property_id], "red"),
            Some(RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Color(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                    name: Some("red".to_string()),
                }),
            })
        );
    }
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AccentColor], "auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AccentColor,
            value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword("auto".to_string())),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextDecorationColor], "red blue"),
        None
    );
    let rust_owned_keyword_longhands = [
        (PropertyId::Appearance, "auto", "auto"),
        (PropertyId::BorderBlockEndStyle, "hidden", "hidden"),
        (PropertyId::BorderBlockStartStyle, "dotted", "dotted"),
        (PropertyId::BorderBottomStyle, "solid", "solid"),
        (PropertyId::BorderCollapse, "collapse", "collapse"),
        (PropertyId::BorderInlineEndStyle, "groove", "groove"),
        (PropertyId::BorderInlineStartStyle, "ridge", "ridge"),
        (PropertyId::BorderLeftStyle, "dashed", "dashed"),
        (PropertyId::BorderRightStyle, "dotted", "dotted"),
        (PropertyId::BorderTopStyle, "double", "double"),
        (PropertyId::BoxSizing, "border-box", "border-box"),
        (PropertyId::CaptionSide, "bottom", "bottom"),
        (PropertyId::Clear, "inline-start", "inline-start"),
        (PropertyId::ClipRule, "evenodd", "evenodd"),
        (PropertyId::ColorInterpolation, "srgb", "srgb"),
        (PropertyId::ColumnSpan, "all", "all"),
        (PropertyId::ContentVisibility, "hidden", "hidden"),
        (PropertyId::Direction, "rtl", "rtl"),
        (PropertyId::DominantBaseline, "alphabetic", "alphabetic"),
        (PropertyId::EmptyCells, "hide", "hide"),
        (PropertyId::FillRule, "evenodd", "evenodd"),
        (PropertyId::FlexDirection, "column", "column"),
        (PropertyId::FlexWrap, "wrap", "wrap"),
        (PropertyId::Float, "inline-end", "inline-end"),
        (PropertyId::ImageRendering, "pixelated", "pixelated"),
        (PropertyId::Isolation, "isolate", "isolate"),
        (PropertyId::ListStylePosition, "inside", "inside"),
        (PropertyId::MathShift, "compact", "compact"),
        (PropertyId::MathStyle, "compact", "compact"),
        (PropertyId::MixBlendMode, "multiply", "multiply"),
        (PropertyId::ObjectFit, "contain", "contain"),
        (PropertyId::OutlineStyle, "auto", "auto"),
        (PropertyId::OverflowX, "hidden", "hidden"),
        (PropertyId::OverflowY, "scroll", "scroll"),
        (PropertyId::PointerEvents, "all", "all"),
        (PropertyId::Position, "sticky", "sticky"),
        (PropertyId::Resize, "block", "block"),
        (PropertyId::ScrollbarWidth, "thin", "thin"),
        (PropertyId::ShapeRendering, "crispedges", "crispedges"),
        (PropertyId::StrokeLinecap, "round", "round"),
        (PropertyId::StrokeLinejoin, "bevel", "bevel"),
        (PropertyId::TableLayout, "fixed", "fixed"),
        (PropertyId::TextAlign, "end", "end"),
        (PropertyId::TextAnchor, "middle", "middle"),
        (PropertyId::TextDecorationSkipInk, "none", "none"),
        (PropertyId::TextDecorationStyle, "wavy", "wavy"),
        (PropertyId::TextJustify, "inter-word", "inter-word"),
        (PropertyId::TextRendering, "optimizelegibility", "optimizelegibility"),
        (PropertyId::TextTransform, "uppercase", "uppercase"),
        (PropertyId::TransformBox, "view-box", "view-box"),
        (PropertyId::UnicodeBidi, "plaintext", "plaintext"),
        (PropertyId::UserSelect, "text", "text"),
        (PropertyId::Visibility, "collapse", "collapse"),
        (PropertyId::WhiteSpaceCollapse, "break-spaces", "break-spaces"),
        (PropertyId::WritingMode, "vertical-rl", "vertical-rl"),
    ];
    for (property_id, input, keyword) in rust_owned_keyword_longhands {
        assert_eq!(
            parse_rust_owned_style_value(&[property_id], input),
            Some(RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(keyword.to_string())),
            })
        );
        assert_eq!(
            parse_rust_owned_style_value(&[property_id], &format!("{input} {input}")),
            None
        );
    }
    for (property_id, input, value, value_type) in [
        (
            PropertyId::FlexGrow,
            "3",
            RustOwnedNestedPrimitiveValue::Number(3.0),
            PropertyValueType::Number,
        ),
        (
            PropertyId::StrokeMiterlimit,
            "4",
            RustOwnedNestedPrimitiveValue::Number(4.0),
            PropertyValueType::Number,
        ),
    ] {
        assert_eq!(
            parse_rust_owned_style_value(&[property_id], input),
            Some(RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested { value, value_type }),
            })
        );
    }
    for property_id in [
        PropertyId::FillOpacity,
        PropertyId::FloodOpacity,
        PropertyId::Opacity,
        PropertyId::ShapeImageThreshold,
        PropertyId::StopOpacity,
        PropertyId::StrokeOpacity,
    ] {
        assert_eq!(
            parse_rust_owned_style_value(&[property_id], "25%"),
            Some(RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                    value: RustOwnedNestedPrimitiveValue::Percentage(25.0),
                    value_type: PropertyValueType::OpacityValue,
                }),
            })
        );
    }
    for (property_id, input, value) in [
        (PropertyId::ColumnCount, "3", 3),
        (PropertyId::Order, "-2147483649", i32::MIN),
        (PropertyId::Orphans, "1", 1),
        (PropertyId::Widows, "2", 2),
        (PropertyId::ZIndex, "2147483648", i32::MAX),
    ] {
        assert_eq!(
            parse_rust_owned_style_value(&[property_id], input),
            Some(RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                    value: RustOwnedNestedPrimitiveValue::Integer(value),
                    value_type: PropertyValueType::Integer,
                }),
            })
        );
    }
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ColumnCount], "auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ColumnCount,
            value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword("auto".to_string())),
        })
    );
    assert_eq!(parse_rust_owned_style_value(&[PropertyId::ColumnCount], "0"), None);
    assert_eq!(parse_rust_owned_style_value(&[PropertyId::Orphans], "0"), None);
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Fill], "none"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Fill,
            value: RustOwnedStyleValueKind::Paint(RustOwnedPaint::None),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Fill], "url(#paint) red"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Fill,
            value: RustOwnedStyleValueKind::Paint(RustOwnedPaint::Url {
                url: RustOwnedUrl {
                    url: Some(RustOwnedUrlPayload {
                        function_type: CssUrlFunctionType::Url,
                        url: "#paint".to_string(),
                        request_url_modifiers: vec![],
                    }),
                },
                fallback_color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                    name: Some("red".to_string()),
                }),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Stroke], "url(#paint) red"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Stroke,
            value: RustOwnedStyleValueKind::Paint(RustOwnedPaint::Url {
                url: RustOwnedUrl {
                    url: Some(RustOwnedUrlPayload {
                        function_type: CssUrlFunctionType::Url,
                        url: "#paint".to_string(),
                        request_url_modifiers: vec![],
                    }),
                },
                fallback_color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                    name: Some("red".to_string()),
                }),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::CornerTopLeftShape], "squircle"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::CornerTopLeftShape,
            value: RustOwnedStyleValueKind::CornerShape(RustOwnedCornerShape {
                value: RustOwnedNestedPrimitiveValue::Keyword("squircle".to_string()),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::CornerTopLeftShape], "superellipse(-infinity)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::CornerTopLeftShape,
            value: RustOwnedStyleValueKind::CornerShape(RustOwnedCornerShape {
                value: RustOwnedNestedPrimitiveValue::Number(f64::NEG_INFINITY),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::CornerStartStartShape], "notch"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::CornerStartStartShape,
            value: RustOwnedStyleValueKind::CornerShape(RustOwnedCornerShape {
                value: RustOwnedNestedPrimitiveValue::Keyword("notch".to_string()),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::CornerEndEndShape], "superellipse(3)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::CornerEndEndShape,
            value: RustOwnedStyleValueKind::CornerShape(RustOwnedCornerShape {
                value: RustOwnedNestedPrimitiveValue::Number(3.0),
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::CornerEndEndShape], "superellipse(random(3, 1))"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::CornerEndEndShape,
            value: RustOwnedStyleValueKind::CornerShape(RustOwnedCornerShape {
                value: RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                    name,
                    source,
                    value_type: PropertyValueType::Number,
                    ..
                }),
            }),
        }) if name == "random" && source == "random(3, 1)"
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ListStyleType], "symbols(\"*\" \"**\")"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ListStyleType,
            value: RustOwnedStyleValueKind::CounterStyle(CounterStyle::SymbolsFunction {
                symbols_type: CssCounterStyleSymbolsType::Symbolic,
                symbols: vec!["*".to_string(), "**".to_string()],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ListStyle], "inside url(marker.png) square"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ListStyle,
            value: RustOwnedStyleValueKind::ListStyle(RustOwnedListStyle {
                position: Some(RustOwnedListStylePosition::Inside),
                image: Some(RustOwnedListStyleImage::Image(Box::new(RustOwnedImage {
                    kind: RustOwnedImageKind::Url,
                    source: None,
                    url: Some(RustOwnedUrlPayload {
                        function_type: CssUrlFunctionType::Url,
                        url: "marker.png".to_string(),
                        request_url_modifiers: vec![],
                    }),
                    gradient: None,
                    image_set: None,
                    component_values: vec![],
                }))),
                list_style_type: Some(RustOwnedListStyleType::CounterStyle(CounterStyle::Name(
                    "square".to_string()
                ))),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AspectRatio], "auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AspectRatio,
            value: RustOwnedStyleValueKind::AspectRatio(RustOwnedAspectRatio {
                has_auto: true,
                numerator: None,
                denominator: None,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AspectRatio], "16 / 9"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AspectRatio,
            value: RustOwnedStyleValueKind::AspectRatio(RustOwnedAspectRatio {
                has_auto: false,
                numerator: Some(RustOwnedNestedPrimitiveValue::Number(16.0)),
                denominator: Some(RustOwnedNestedPrimitiveValue::Number(9.0)),
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::AspectRatio], "auto calc(16 / 9)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AspectRatio,
            value: RustOwnedStyleValueKind::AspectRatio(RustOwnedAspectRatio {
                has_auto: true,
                numerator: Some(RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                    name,
                    source,
                    value_type: PropertyValueType::Number,
                    ..
                })),
                denominator: None,
            }),
        }) if name == "calc" && source == "calc(16 / 9)"
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Content], "counter(section, upper-roman)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Content,
            value: RustOwnedStyleValueKind::Content(RustOwnedContent::Items {
                items: vec![RustOwnedContentItem::Counter(RustOwnedCounterFunction {
                    function: RustOwnedCounterFunctionKind::Counter,
                    counter_name: "section".to_string(),
                    join_string: None,
                    counter_style: Some(CounterStyle::Name("upper-roman".to_string())),
                })],
                alt_text: vec![],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ShapeOutside], "circle(10px) border-box"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ShapeOutside,
            value: RustOwnedStyleValueKind::ShapeOutside(RustOwnedShapeOutside::Shape {
                basic_shape: Some(Box::new(RustOwnedBasicShape {
                    kind: RustOwnedBasicShapeKind::Circle,
                    fill_rule: RustOwnedBasicShapeFillRule::Nonzero,
                    rectangle_components: vec![],
                    rectangle_border_radius: None,
                    radial_shape_radius: vec![RustOwnedNestedPrimitiveValue::Length {
                        value: 10.0,
                        unit: "px".to_string(),
                    }],
                    radial_shape_position: None,
                    polygon_points: vec![],
                    path_data: None,
                })),
                shape_box: Some(RustOwnedShapeBox::Border),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ShapeOutside], "circle()"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ShapeOutside,
            value: RustOwnedStyleValueKind::ShapeOutside(RustOwnedShapeOutside::Shape {
                basic_shape: Some(Box::new(RustOwnedBasicShape {
                    kind: RustOwnedBasicShapeKind::Circle,
                    fill_rule: RustOwnedBasicShapeFillRule::Nonzero,
                    rectangle_components: vec![],
                    rectangle_border_radius: None,
                    radial_shape_radius: vec![],
                    radial_shape_position: None,
                    polygon_points: vec![],
                    path_data: None,
                })),
                shape_box: None,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Flex], "1 1 10em"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Flex,
            value: RustOwnedStyleValueKind::FlexShorthand(RustOwnedFlexShorthand::Longhands {
                flex_grow: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_shrink: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_basis: RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::Length {
                    value: 10.0,
                    unit: "em".to_string(),
                }),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Flex], "0"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Flex,
            value: RustOwnedStyleValueKind::FlexShorthand(RustOwnedFlexShorthand::Longhands {
                flex_grow: RustOwnedNestedPrimitiveValue::Number(0.0),
                flex_shrink: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_basis: RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::Percentage(0.0)),
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::Flex], "calc(10px + 0.5em)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Flex,
            value: RustOwnedStyleValueKind::FlexShorthand(RustOwnedFlexShorthand::Longhands {
                flex_grow: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_shrink: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_basis: RustOwnedFlexBasis::Value(RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                    name,
                    source,
                    value_type: PropertyValueType::Length,
                    ..
                })),
            }),
        }) if name == "calc" && source == "calc(10px + 0.5em)"
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Flex], "1 1 fit-content(20%)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Flex,
            value: RustOwnedStyleValueKind::FlexShorthand(RustOwnedFlexShorthand::Longhands {
                flex_grow: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_shrink: RustOwnedNestedPrimitiveValue::Number(1.0),
                flex_basis: RustOwnedFlexBasis::FitContentFunction(RustOwnedNestedPrimitiveValue::Percentage(20.0)),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FlexFlow], "wrap row-reverse"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FlexFlow,
            value: RustOwnedStyleValueKind::FlexFlow(RustOwnedFlexFlow {
                flex_direction: Some(RustOwnedFlexDirection::RowReverse),
                flex_wrap: Some(RustOwnedFlexWrap::Wrap),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Columns], "3 12em / auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Columns,
            value: RustOwnedStyleValueKind::Columns(RustOwnedColumns {
                column_count: Some(RustOwnedNestedPrimitiveValue::Integer(3)),
                column_width: Some(RustOwnedNestedPrimitiveValue::Length {
                    value: 12.0,
                    unit: "em".to_string(),
                }),
                column_height: Some(auto_keyword()),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PlaceContent], "space-between center"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PlaceContent,
            value: RustOwnedStyleValueKind::PlaceContent(RustOwnedPlaceShorthand {
                align_keywords: vec!["space-between".to_string()],
                justify_keywords: vec!["center".to_string()],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PlaceItems], "normal start"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PlaceItems,
            value: RustOwnedStyleValueKind::PlaceItems(RustOwnedPlaceShorthand {
                align_keywords: vec!["normal".to_string()],
                justify_keywords: vec!["start".to_string()],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PlaceSelf], "safe end unsafe right"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PlaceSelf,
            value: RustOwnedStyleValueKind::PlaceSelf(RustOwnedPlaceShorthand {
                align_keywords: vec!["safe".to_string(), "end".to_string()],
                justify_keywords: vec!["unsafe".to_string(), "right".to_string()],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Filter], "blur(10px) opacity(50%)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Filter,
            value: RustOwnedStyleValueKind::FilterValueList(RustOwnedFilterValueList::Filters(vec![
                RustOwnedFilterValue::Blur {
                    radius: Some(RustOwnedNestedPrimitiveValue::Length {
                        value: 10.0,
                        unit: "px".to_string(),
                    }),
                },
                RustOwnedFilterValue::Simple {
                    function: RustOwnedSimpleFilterFunction::Opacity,
                    amount: Some(RustOwnedNestedPrimitiveValue::Percentage(50.0)),
                },
            ])),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Filter], "url(\"filter.svg\" cross-origin(anonymous))"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Filter,
            value: RustOwnedStyleValueKind::FilterValueList(RustOwnedFilterValueList::Filters(vec![
                RustOwnedFilterValue::Url(RustOwnedUrl {
                    url: Some(RustOwnedUrlPayload {
                        function_type: CssUrlFunctionType::Url,
                        url: "filter.svg".to_string(),
                        request_url_modifiers: vec![UrlModifier::CrossOrigin(
                            CssUrlCrossOriginModifierValue::Anonymous
                        )],
                    }),
                }),
            ])),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnchorName], "--foo, --bar"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnchorName,
            value: RustOwnedStyleValueKind::AnchorNameOrScope(RustOwnedAnchorNameOrScope {
                kind: CssAnchorNameOrScopeValueKind::List,
                names: vec!["--foo".to_string(), "--bar".to_string()],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnchorScope], "all"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnchorScope,
            value: RustOwnedStyleValueKind::AnchorNameOrScope(RustOwnedAnchorNameOrScope {
                kind: CssAnchorNameOrScopeValueKind::All,
                names: vec![],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ColorScheme], "only dark"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ColorScheme,
            value: RustOwnedStyleValueKind::ColorScheme(RustOwnedColorScheme {
                value: CssColorSchemeValue {
                    kind: CssColorSchemeValueKind::List,
                    only: true,
                },
                schemes: vec!["dark".to_string()],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PositionAnchor], "--foo"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PositionAnchor,
            value: RustOwnedStyleValueKind::PositionAnchor(RustOwnedPositionAnchor {
                kind: CssPositionAnchorValueKind::AnchorName,
                name: Some("--foo".to_string()),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontFamily], "serif, \"Bongo Sans\""),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontFamily,
            value: RustOwnedStyleValueKind::FontFamily(RustOwnedFontFamilyList {
                values: vec![
                    FontFamilyValue::Generic("serif".to_string()),
                    FontFamilyValue::FamilyName(FamilyName {
                        name: "Bongo Sans".to_string(),
                        is_string: true,
                    }),
                ],
            }),
        })
    );
    let font_shorthand_component_values = parse("italic bold 16px / 1.5 serif");
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Font], "italic bold 16px / 1.5 serif"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Font,
            value: RustOwnedStyleValueKind::FontShorthand(vec![
                RustOwnedFontShorthandItem {
                    property_id: PropertyId::FontStyle,
                    style_value: parse_rust_owned_style_value(&[PropertyId::FontStyle], "italic").unwrap(),
                    source: "italic".to_string(),
                    component_values: font_shorthand_component_values[0..2].to_vec(),
                },
                RustOwnedFontShorthandItem {
                    property_id: PropertyId::FontWeight,
                    style_value: parse_rust_owned_style_value(&[PropertyId::FontWeight], "bold").unwrap(),
                    source: "bold".to_string(),
                    component_values: font_shorthand_component_values[2..4].to_vec(),
                },
                RustOwnedFontShorthandItem {
                    property_id: PropertyId::FontSize,
                    style_value: parse_rust_owned_style_value(&[PropertyId::FontSize], "16px").unwrap(),
                    source: "16px".to_string(),
                    component_values: font_shorthand_component_values[4..6].to_vec(),
                },
                RustOwnedFontShorthandItem {
                    property_id: PropertyId::LineHeight,
                    style_value: parse_rust_owned_style_value(&[PropertyId::LineHeight], "1.5").unwrap(),
                    source: "1.5".to_string(),
                    component_values: font_shorthand_component_values[8..10].to_vec(),
                },
                RustOwnedFontShorthandItem {
                    property_id: PropertyId::FontFamily,
                    style_value: parse_rust_owned_style_value(&[PropertyId::FontFamily], "serif").unwrap(),
                    source: "serif".to_string(),
                    component_values: font_shorthand_component_values[10..].to_vec(),
                },
            ]),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontFeatureSettings], "\"kern\" on"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontFeatureSettings,
            value: RustOwnedStyleValueKind::OpenTypeSettings(RustOwnedOpenTypeSettingsStyleValue {
                kind: RustOwnedOpenTypeSettingsStyleValueKind::FontFeatureSettings,
                value: RustOwnedOpenTypeSettings {
                    kind: CssOpenTypeSettingsKind::TagValues,
                    tag_values: vec![OpenTypeTaggedValue {
                        tag: "kern".to_string(),
                        value_kind: CssOpenTypeTaggedValueKind::On,
                        value: None,
                        value_component_values: vec![],
                    }],
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontLanguageOverride], "\"KSW\""),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontLanguageOverride,
            value: RustOwnedStyleValueKind::FontLanguageOverride(RustOwnedFontLanguageOverride {
                kind: CssFontLanguageOverrideKind::String,
                value: Some("KSW".to_string()),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontVariant], "small-caps tabular-nums"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontVariant,
            value: RustOwnedStyleValueKind::FontVariant(FontVariant {
                caps: Some("small-caps".to_string()),
                numeric: Some(vec![FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Spacing,
                    value: "tabular-nums".to_string(),
                }]),
                ..FontVariant::default()
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontVariationSettings], "\"wght\" 700"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontVariationSettings,
            value: RustOwnedStyleValueKind::OpenTypeSettings(RustOwnedOpenTypeSettingsStyleValue {
                kind: RustOwnedOpenTypeSettingsStyleValueKind::FontVariationSettings,
                value: RustOwnedOpenTypeSettings {
                    kind: CssOpenTypeSettingsKind::TagValues,
                    tag_values: vec![OpenTypeTaggedValue {
                        tag: "wght".to_string(),
                        value_kind: CssOpenTypeTaggedValueKind::Value,
                        value: Some("700".to_string()),
                        value_component_values: open_type_value_component_values("\"wght\" 700", "wght"),
                    }],
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(
            &[PropertyId::Content],
            "counters(section, \".\", symbols(\"*\" \"**\"))"
        ),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Content,
            value: RustOwnedStyleValueKind::Content(RustOwnedContent::Items {
                items: vec![RustOwnedContentItem::Counter(RustOwnedCounterFunction {
                    function: RustOwnedCounterFunctionKind::Counters,
                    counter_name: "section".to_string(),
                    join_string: Some(".".to_string()),
                    counter_style: Some(CounterStyle::SymbolsFunction {
                        symbols_type: CssCounterStyleSymbolsType::Symbolic,
                        symbols: vec!["*".to_string(), "**".to_string()],
                    }),
                })],
                alt_text: vec![],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Content], "url(marker.png)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Content,
            value: RustOwnedStyleValueKind::Content(RustOwnedContent::Items {
                items: vec![RustOwnedContentItem::Image(RustOwnedImage {
                    kind: RustOwnedImageKind::Url,
                    source: None,
                    url: Some(RustOwnedUrlPayload {
                        function_type: CssUrlFunctionType::Url,
                        url: "marker.png".to_string(),
                        request_url_modifiers: vec![],
                    }),
                    gradient: None,
                    image_set: None,
                    component_values: vec![],
                })],
                alt_text: vec![],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundImage], "image-set(url(example.png) 2x)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundImage,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![RustOwnedGeneratedValueListItem {
                    value_type: PropertyValueType::Image,
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundImage], "url(example.png)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundImage,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![RustOwnedGeneratedValueListItem {
                    value_type: PropertyValueType::Image,
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Top], "anchor(--target bottom, calc(1px + 2%))"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Top,
            value: RustOwnedStyleValueKind::Anchor(RustOwnedAnchorFunction {
                anchor_name: Some("--target".to_string()),
                anchor_side: "bottom".to_string(),
                fallback: Some("calc(1px + 2%)".to_string()),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundImage], "linear-gradient(black, white)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundImage,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![RustOwnedGeneratedValueListItem {
                    value_type: PropertyValueType::Image,
                }],
            }),
        })
    );
    let gradient = parse_rust_owned_image("linear-gradient(to bottom in oklab, black, white)").unwrap();
    assert_eq!(gradient.kind, RustOwnedImageKind::Gradient);
    let gradient = gradient.gradient.as_ref().unwrap();
    assert_eq!(gradient.kind, RustOwnedGradientKind::Linear);
    assert!(!gradient.is_repeating);
    assert!(!gradient.is_webkit_prefixed);
    assert_eq!(gradient.color_stop_group_index, 1);
    let header = gradient.header.as_ref().unwrap();
    assert_eq!(header.component_values.len(), 5);
    assert!(header.color_interpolation_method.is_some());
    assert_eq!(gradient.groups.len(), 3);
    assert_eq!(
        parse_rust_owned_style_value(
            &[PropertyId::BackgroundImage],
            "image-set(\"example.png\" type(\"image/png\"), linear-gradient(black, white) 2x)"
        ),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundImage,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![RustOwnedGeneratedValueListItem {
                    value_type: PropertyValueType::Image,
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Top], "auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Top,
            value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword("auto".to_string())),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::InsetBlockStart], "12px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::InsetBlockStart,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Length {
                    value: 12.0,
                    unit: "px".to_string(),
                },
                value_type: PropertyValueType::Length,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Left], "4.2535287499999996e+38px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Left,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Length {
                    value: f32::MAX as f64,
                    unit: "px".to_string(),
                },
                value_type: PropertyValueType::Length,
            }),
        })
    );
    assert_eq!(parse_rust_owned_style_value(&[PropertyId::Top], "red"), None);
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PaddingTop], "10%"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PaddingTop,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Percentage(10.0),
                value_type: PropertyValueType::Percentage,
            }),
        })
    );
    assert_eq!(parse_rust_owned_style_value(&[PropertyId::PaddingTop], "-1px"), None);
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::MarginLeft], "auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::MarginLeft,
            value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword("auto".to_string())),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderTopWidth], "thick"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderTopWidth,
            value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword("thick".to_string())),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::RowGap], "12px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::RowGap,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Length {
                    value: 12.0,
                    unit: "px".to_string(),
                },
                value_type: PropertyValueType::Length,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontWidth], "condensed"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontWidth,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Token {
                primitive_kind: CssPrimitiveValueKind::Keyword,
                numeric_value: None,
                secondary_numeric_value: None,
                value: "condensed".to_string(),
                value_type: PropertyValueType::FontWidthCss3,
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::TabSize], "calc(10px)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TabSize,
            value: RustOwnedStyleValueKind::MathFunction(RustOwnedMathFunction {
                name,
                source,
                value_type: PropertyValueType::Length,
                ..
            }),
        }) if name == "calc" && source == "calc(10px)"
    ));
    for (property_id, source, keyword) in [
        (PropertyId::OverflowWrap, "break-word", "break-word"),
        (PropertyId::ScrollBehavior, "smooth", "smooth"),
        (PropertyId::TransformStyle, "preserve-3d", "preserve-3d"),
        (PropertyId::WordBreak, "keep-all", "keep-all"),
    ] {
        assert_eq!(
            parse_rust_owned_style_value(&[property_id], source),
            Some(RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(keyword.to_string())),
            })
        );
    }
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ScrollBehavior], "smooth auto"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Width], "anchor-size(--target width, 10px)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Width,
            value: RustOwnedStyleValueKind::AnchorSize(RustOwnedAnchorSizeFunction {
                value_type: PropertyValueType::Length,
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundSize], "cover, auto 10px, 2% calc(3px + 4%)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundSize,
            value: RustOwnedStyleValueKind::BackgroundSize(RustOwnedBackgroundSizeList { values }),
        }) if values.len() == 3
            && matches!(&values[0], RustOwnedBackgroundSize::Cover)
            && matches!(
                &values[1],
                RustOwnedBackgroundSize::Explicit {
                    width: RustOwnedNestedPrimitiveValue::Keyword(keyword),
                    height: Some(RustOwnedNestedPrimitiveValue::Length { value, unit }),
                } if keyword == "auto" && *value == 10.0 && unit == "px"
            )
            && matches!(
                &values[2],
                RustOwnedBackgroundSize::Explicit {
                    width: RustOwnedNestedPrimitiveValue::Percentage(value),
                    height: Some(RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                        name,
                        source,
                        value_type: PropertyValueType::Length,
                        ..
                    })),
                } if *value == 2.0 && name == "calc" && source == "calc(3px + 4%)"
            )
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderTopLeftRadius], "1px / 2%"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderTopLeftRadius,
            value: RustOwnedStyleValueKind::BorderRadius(RustOwnedBorderRadius {
                horizontal_radii: vec![RustOwnedNestedPrimitiveValue::Length {
                    value: 1.0,
                    unit: "px".to_string(),
                }],
                vertical_radii: vec![RustOwnedNestedPrimitiveValue::Percentage(2.0)],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderRadius], "1px 2px / 3px 4px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderRadius,
            value: RustOwnedStyleValueKind::BorderRadius(RustOwnedBorderRadius {
                horizontal_radii: vec![
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 1.0,
                        unit: "px".to_string(),
                    },
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 2.0,
                        unit: "px".to_string(),
                    },
                ],
                vertical_radii: vec![
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 3.0,
                        unit: "px".to_string(),
                    },
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 4.0,
                        unit: "px".to_string(),
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Border], "1px solid red"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Border,
            value: RustOwnedStyleValueKind::Border(RustOwnedBorder {
                width: Some(RustOwnedNestedPrimitiveValue::Length {
                    value: 1.0,
                    unit: "px".to_string(),
                }),
                style: Some(RustOwnedLineStyle::Solid),
                color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                    name: Some("red".to_string()),
                }),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Border], "1px solid black"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Border,
            value: RustOwnedStyleValueKind::Border(RustOwnedBorder {
                width: Some(RustOwnedNestedPrimitiveValue::Length {
                    value: 1.0,
                    unit: "px".to_string(),
                }),
                style: Some(RustOwnedLineStyle::Solid),
                color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 0,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                    name: Some("black".to_string()),
                }),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderBlock], "currentcolor thick dashed"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderBlock,
            value: RustOwnedStyleValueKind::Border(RustOwnedBorder {
                width: Some(RustOwnedNestedPrimitiveValue::Keyword("thick".to_string())),
                style: Some(RustOwnedLineStyle::Dashed),
                color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Keyword,
                    red: 0,
                    green: 0,
                    blue: 0,
                    alpha: 0,
                    name: Some("currentcolor".to_string()),
                }),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderInlineStart], "green double thin"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderInlineStart,
            value: RustOwnedStyleValueKind::Border(RustOwnedBorder {
                width: Some(RustOwnedNestedPrimitiveValue::Keyword("thin".to_string())),
                style: Some(RustOwnedLineStyle::Double),
                color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 0,
                    green: 128,
                    blue: 0,
                    alpha: 255,
                    name: Some("green".to_string()),
                }),
            }),
        })
    );
    let border_bottom_component_values = parse("1px solid #123456");
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderBottom], "1px solid #123456"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderBottom,
            value: RustOwnedStyleValueKind::ComponentShorthand(vec![
                RustOwnedComponentShorthandItem {
                    property_id: PropertyId::BorderBottomWidth,
                    style_value: RustOwnedStyleValue {
                        property_id: PropertyId::BorderBottomWidth,
                        value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                            value: RustOwnedNestedPrimitiveValue::Length {
                                value: 1.0,
                                unit: "px".to_string(),
                            },
                            value_type: PropertyValueType::Length,
                        }),
                    },
                    source: "1px".to_string(),
                    component_values: border_bottom_component_values[0..1].to_vec(),
                },
                RustOwnedComponentShorthandItem {
                    property_id: PropertyId::BorderBottomStyle,
                    style_value: RustOwnedStyleValue {
                        property_id: PropertyId::BorderBottomStyle,
                        value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(
                            "solid".to_string(),
                        )),
                    },
                    source: "solid".to_string(),
                    component_values: border_bottom_component_values[2..3].to_vec(),
                },
                RustOwnedComponentShorthandItem {
                    property_id: PropertyId::BorderBottomColor,
                    style_value: RustOwnedStyleValue {
                        property_id: PropertyId::BorderBottomColor,
                        value: RustOwnedStyleValueKind::Color(RustOwnedColor::Simple {
                            kind: CssParsedColorKind::Rgba,
                            red: 0x12,
                            green: 0x34,
                            blue: 0x56,
                            alpha: 0xff,
                            name: None,
                        }),
                    },
                    source: "#123456".to_string(),
                    component_values: border_bottom_component_values[4..5].to_vec(),
                },
            ]),
        })
    );
    let outline_component_values = parse("auto red thick");
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Outline], "auto red thick"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Outline,
            value: RustOwnedStyleValueKind::ComponentShorthand(vec![
                RustOwnedComponentShorthandItem {
                    property_id: PropertyId::OutlineStyle,
                    style_value: RustOwnedStyleValue {
                        property_id: PropertyId::OutlineStyle,
                        value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(
                            "auto".to_string()
                        )),
                    },
                    source: "auto".to_string(),
                    component_values: outline_component_values[0..1].to_vec(),
                },
                RustOwnedComponentShorthandItem {
                    property_id: PropertyId::OutlineColor,
                    style_value: RustOwnedStyleValue {
                        property_id: PropertyId::OutlineColor,
                        value: RustOwnedStyleValueKind::Color(RustOwnedColor::Simple {
                            kind: CssParsedColorKind::Rgba,
                            red: 255,
                            green: 0,
                            blue: 0,
                            alpha: 255,
                            name: Some("red".to_string()),
                        }),
                    },
                    source: "red".to_string(),
                    component_values: outline_component_values[2..3].to_vec(),
                },
                RustOwnedComponentShorthandItem {
                    property_id: PropertyId::OutlineWidth,
                    style_value: RustOwnedStyleValue {
                        property_id: PropertyId::OutlineWidth,
                        value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(
                            "thick".to_string()
                        )),
                    },
                    source: "thick".to_string(),
                    component_values: outline_component_values[4..5].to_vec(),
                },
            ]),
        })
    );
    assert_eq!(parse_rust_owned_style_value(&[PropertyId::Outline], "auto auto"), None);
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderInline], "solid solid"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageSlice], "10% 20 30% fill"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderImageSlice,
            value: RustOwnedStyleValueKind::BorderImageSlice(RustOwnedBorderImageSlice {
                values: vec![
                    RustOwnedNestedPrimitiveValue::Percentage(10.0),
                    RustOwnedNestedPrimitiveValue::Number(20.0),
                    RustOwnedNestedPrimitiveValue::Percentage(30.0),
                    RustOwnedNestedPrimitiveValue::Number(20.0),
                ],
                fill: true,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageSlice], "10%"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderImageSlice,
            value: RustOwnedStyleValueKind::BorderImageSlice(RustOwnedBorderImageSlice {
                values: vec![
                    RustOwnedNestedPrimitiveValue::Percentage(10.0),
                    RustOwnedNestedPrimitiveValue::Percentage(10.0),
                    RustOwnedNestedPrimitiveValue::Percentage(10.0),
                    RustOwnedNestedPrimitiveValue::Percentage(10.0),
                ],
                fill: false,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageSlice], "10% fill 20"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageOutset], "1px 2 3px 4"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderImageOutset,
            value: RustOwnedStyleValueKind::BorderImageOutset(RustOwnedBorderImageOutsetList {
                values: vec![
                    RustOwnedBorderImageOutset {
                        value: RustOwnedNestedPrimitiveValue::Length {
                            value: 1.0,
                            unit: "px".to_string(),
                        },
                    },
                    RustOwnedBorderImageOutset {
                        value: RustOwnedNestedPrimitiveValue::Number(2.0),
                    },
                    RustOwnedBorderImageOutset {
                        value: RustOwnedNestedPrimitiveValue::Length {
                            value: 3.0,
                            unit: "px".to_string(),
                        },
                    },
                    RustOwnedBorderImageOutset {
                        value: RustOwnedNestedPrimitiveValue::Number(4.0),
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageOutset], "1%"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageWidth], "1px 2% 3 auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderImageWidth,
            value: RustOwnedStyleValueKind::BorderImageWidth(RustOwnedBorderImageWidthList {
                values: vec![
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 1.0,
                        unit: "px".to_string(),
                    },
                    RustOwnedNestedPrimitiveValue::Percentage(2.0),
                    RustOwnedNestedPrimitiveValue::Number(3.0),
                    auto_keyword(),
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageWidth], "-1px"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageRepeat], "stretch round"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderImageRepeat,
            value: RustOwnedStyleValueKind::BorderImageRepeat(RustOwnedBorderImageRepeatList {
                values: vec![RustOwnedBorderImageRepeat::Stretch, RustOwnedBorderImageRepeat::Round],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageRepeat], "repeat round space"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImageRepeat], "no-repeat"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImage], "url(border.png) 10 fill / 2 / 3 round"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderImage,
            value: RustOwnedStyleValueKind::BorderImage(RustOwnedBorderImage {
                source: Some(RustOwnedBorderImageSource::Image(Box::new(RustOwnedImage {
                    kind: RustOwnedImageKind::Url,
                    source: None,
                    url: Some(RustOwnedUrlPayload {
                        function_type: CssUrlFunctionType::Url,
                        url: "border.png".to_string(),
                        request_url_modifiers: vec![],
                    }),
                    gradient: None,
                    image_set: None,
                    component_values: vec![],
                }))),
                slice: Some(RustOwnedBorderImageSlice {
                    values: vec![
                        RustOwnedNestedPrimitiveValue::Number(10.0),
                        RustOwnedNestedPrimitiveValue::Number(10.0),
                        RustOwnedNestedPrimitiveValue::Number(10.0),
                        RustOwnedNestedPrimitiveValue::Number(10.0),
                    ],
                    fill: true,
                }),
                width: Some(vec![RustOwnedNestedPrimitiveValue::Number(2.0)]),
                outset: Some(vec![RustOwnedBorderImageOutset {
                    value: RustOwnedNestedPrimitiveValue::Number(3.0),
                }]),
                repeat: Some(vec![RustOwnedBorderImageRepeat::Round]),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImage], "10 / / 2 stretch round"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderImage,
            value: RustOwnedStyleValueKind::BorderImage(RustOwnedBorderImage {
                source: None,
                slice: Some(RustOwnedBorderImageSlice {
                    values: vec![
                        RustOwnedNestedPrimitiveValue::Number(10.0),
                        RustOwnedNestedPrimitiveValue::Number(10.0),
                        RustOwnedNestedPrimitiveValue::Number(10.0),
                        RustOwnedNestedPrimitiveValue::Number(10.0),
                    ],
                    fill: false,
                }),
                width: None,
                outset: Some(vec![RustOwnedBorderImageOutset {
                    value: RustOwnedNestedPrimitiveValue::Number(2.0),
                }]),
                repeat: Some(vec![
                    RustOwnedBorderImageRepeat::Stretch,
                    RustOwnedBorderImageRepeat::Round,
                ]),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderImage], "1 / -2px"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BoxShadow], "inset 1px 2px 3px red"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BoxShadow,
            value: RustOwnedStyleValueKind::Shadow(RustOwnedShadow::Shadows(vec![RustOwnedSingleShadow {
                color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                    name: Some("red".to_string()),
                }),
                offset_x: RustOwnedNestedPrimitiveValue::Length {
                    value: 1.0,
                    unit: "px".to_string(),
                },
                offset_y: RustOwnedNestedPrimitiveValue::Length {
                    value: 2.0,
                    unit: "px".to_string(),
                },
                blur_radius: Some(RustOwnedNestedPrimitiveValue::Length {
                    value: 3.0,
                    unit: "px".to_string(),
                }),
                spread_distance: None,
                placement: RustOwnedShadowPlacement::Inner,
            }])),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Cursor], "url(cursor.png) 1 2, pointer"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Cursor,
            value: RustOwnedStyleValueKind::Cursor(RustOwnedCursor {
                images: vec![RustOwnedCursorImage {
                    image: RustOwnedImage {
                        kind: RustOwnedImageKind::Url,
                        source: None,
                        url: Some(RustOwnedUrlPayload {
                            function_type: CssUrlFunctionType::Url,
                            url: "cursor.png".to_string(),
                            request_url_modifiers: vec![],
                        }),
                        gradient: None,
                        image_set: None,
                        component_values: vec![],
                    },
                    x: Some(RustOwnedNestedPrimitiveValue::Number(1.0)),
                    y: Some(RustOwnedNestedPrimitiveValue::Number(2.0)),
                }],
                predefined: "pointer".to_string(),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::OverflowClipMarginTop], "2px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::OverflowClipMarginTop,
            value: RustOwnedStyleValueKind::OverflowClipMargin(RustOwnedOverflowClipMargin {
                length: RustOwnedNestedPrimitiveValue::Length {
                    value: 2.0,
                    unit: "px".to_string(),
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::OverflowClipMargin], "2px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::OverflowClipMargin,
            value: RustOwnedStyleValueKind::OverflowClipMargin(RustOwnedOverflowClipMargin {
                length: RustOwnedNestedPrimitiveValue::Length {
                    value: 2.0,
                    unit: "px".to_string(),
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TransitionTimingFunction], "linear(0, 1)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TransitionTimingFunction,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![RustOwnedGeneratedValueListItem {
                    value_type: PropertyValueType::EasingFunction,
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TransitionTimingFunction], "cubic-bezier(0, 0, 1, 1)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TransitionTimingFunction,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![RustOwnedGeneratedValueListItem {
                    value_type: PropertyValueType::EasingFunction,
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TransitionTimingFunction], "steps(2, jump-none)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TransitionTimingFunction,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![RustOwnedGeneratedValueListItem {
                    value_type: PropertyValueType::EasingFunction,
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Width], "fit-content(10px)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Width,
            value: RustOwnedStyleValueKind::FitContent(RustOwnedFitContent {
                value: RustOwnedNestedPrimitiveValue::Length {
                    value: 10.0,
                    unit: "px".to_string(),
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ClipPath], "inset(10px)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ClipPath,
            value: RustOwnedStyleValueKind::BasicShape(Box::new(RustOwnedBasicShape {
                kind: RustOwnedBasicShapeKind::Inset,
                fill_rule: RustOwnedBasicShapeFillRule::Nonzero,
                rectangle_components: vec![RustOwnedNestedPrimitiveValue::Length {
                    value: 10.0,
                    unit: "px".to_string(),
                }],
                rectangle_border_radius: None,
                radial_shape_radius: vec![],
                radial_shape_position: None,
                polygon_points: vec![],
                path_data: None,
            })),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ClipPath], "path(evenodd, \"M 0 0 L 1 1\")"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ClipPath,
            value: RustOwnedStyleValueKind::BasicShape(Box::new(RustOwnedBasicShape {
                kind: RustOwnedBasicShapeKind::Path,
                fill_rule: RustOwnedBasicShapeFillRule::Evenodd,
                rectangle_components: vec![],
                rectangle_border_radius: None,
                radial_shape_radius: vec![],
                radial_shape_position: None,
                polygon_points: vec![],
                path_data: Some("M 0 0 L 1 1".to_string()),
            })),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ClipPath], "polygon(evenodd, 0 0, 100% 0)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ClipPath,
            value: RustOwnedStyleValueKind::BasicShape(Box::new(RustOwnedBasicShape {
                kind: RustOwnedBasicShapeKind::Polygon,
                fill_rule: RustOwnedBasicShapeFillRule::Evenodd,
                rectangle_components: vec![],
                rectangle_border_radius: None,
                radial_shape_radius: vec![],
                radial_shape_position: None,
                polygon_points: vec![
                    RustOwnedBasicShapePolygonPoint {
                        x: RustOwnedNestedPrimitiveValue::Length {
                            value: 0.0,
                            unit: "px".to_string(),
                        },
                        y: RustOwnedNestedPrimitiveValue::Length {
                            value: 0.0,
                            unit: "px".to_string(),
                        },
                    },
                    RustOwnedBasicShapePolygonPoint {
                        x: RustOwnedNestedPrimitiveValue::Percentage(100.0),
                        y: RustOwnedNestedPrimitiveValue::Length {
                            value: 0.0,
                            unit: "px".to_string(),
                        },
                    },
                ],
                path_data: None,
            })),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Clip], "rect(1px, auto, 2px, 3px)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Clip,
            value: RustOwnedStyleValueKind::Rect(RustOwnedRect {
                sides: vec![
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 1.0,
                        unit: "px".to_string(),
                    },
                    RustOwnedNestedPrimitiveValue::Keyword("auto".to_string()),
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 2.0,
                        unit: "px".to_string(),
                    },
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 3.0,
                        unit: "px".to_string(),
                    },
                ],
                requires_commas: true,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnimationTimeline], "scroll(root y)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnimationTimeline,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![RustOwnedGeneratedValueListItem {
                    value_type: PropertyValueType::ScrollFunction,
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ViewTimelineInset], "1px 2px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ViewTimelineInset,
            value: RustOwnedStyleValueKind::ViewTimelineInset(RustOwnedViewTimelineInset {
                insets: vec![vec![
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 1.0,
                        unit: "px".to_string(),
                    },
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 2.0,
                        unit: "px".to_string(),
                    },
                ]],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ViewTimelineInset], "1px 2px, auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ViewTimelineInset,
            value: RustOwnedStyleValueKind::ViewTimelineInset(RustOwnedViewTimelineInset {
                insets: vec![
                    vec![
                        RustOwnedNestedPrimitiveValue::Length {
                            value: 1.0,
                            unit: "px".to_string(),
                        },
                        RustOwnedNestedPrimitiveValue::Length {
                            value: 2.0,
                            unit: "px".to_string(),
                        },
                    ],
                    vec![auto_keyword()],
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BorderSpacing], "1px 2px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BorderSpacing,
            value: RustOwnedStyleValueKind::BorderSpacing(RustOwnedBorderSpacing {
                values: vec![
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 1.0,
                        unit: "px".to_string(),
                    },
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 2.0,
                        unit: "px".to_string(),
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(
            &[PropertyId::ViewTimelineAxis, PropertyId::ViewTimelineInset],
            "1px 2px inline"
        ),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ViewTimelineInset,
            value: RustOwnedStyleValueKind::ViewTimelineInset(RustOwnedViewTimelineInset {
                insets: vec![vec![
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 1.0,
                        unit: "px".to_string(),
                    },
                    RustOwnedNestedPrimitiveValue::Length {
                        value: 2.0,
                        unit: "px".to_string(),
                    },
                ]],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::MarginLeft], "12px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::MarginLeft,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Length {
                    value: 12.0,
                    unit: "px".to_string(),
                },
                value_type: PropertyValueType::Length,
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::MarginLeft], "calc(1px + 2px)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::MarginLeft,
            value: RustOwnedStyleValueKind::MathFunction(RustOwnedMathFunction {
                name,
                source,
                value_type: PropertyValueType::Length,
                ..
            }),
        }) if name == "calc" && source == "calc(1px + 2px)"
    ));
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::Opacity], "clamp(0, 0.5, 1)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Opacity,
            value: RustOwnedStyleValueKind::MathFunction(RustOwnedMathFunction {
                name,
                source,
                value_type: PropertyValueType::OpacityValue,
                ..
            }),
        }) if name == "clamp" && source == "clamp(0, 0.5, 1)"
    ));
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::FontWeight], "calc(600 + 100)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontWeight,
            value: RustOwnedStyleValueKind::MathFunction(RustOwnedMathFunction {
                name,
                source,
                value_type: PropertyValueType::Number,
                ..
            }),
        }) if name == "calc" && source == "calc(600 + 100)"
    ));
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::FontWeight], "sibling-count()"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontWeight,
            value: RustOwnedStyleValueKind::TreeCountingFunction(RustOwnedTreeCountingFunction {
                function: RustOwnedTreeCountingFunctionKind::SiblingCount,
                value_type: PropertyValueType::Number,
                ..
            }),
        })
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ObjectPosition], "left 10px top 20%"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ObjectPosition,
            value: RustOwnedStyleValueKind::Position(RustOwnedPosition {
                value_type: PropertyValueType::Position,
                value: RustOwnedResolvedPosition {
                    x: position_edge_offset(
                        PositionEdge::Left,
                        RustOwnedNestedPrimitiveValue::Length {
                            value: 10.0,
                            unit: "px".to_string(),
                        },
                    ),
                    y: position_edge_offset(PositionEdge::Top, RustOwnedNestedPrimitiveValue::Percentage(20.0)),
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundPosition], "left 10px top, center"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundPosition,
            value: RustOwnedStyleValueKind::PositionList(RustOwnedPositionList {
                value_type: PropertyValueType::BackgroundPosition,
                values: vec![
                    RustOwnedPositionListItem::Position(RustOwnedResolvedPosition {
                        x: position_edge_offset(
                            PositionEdge::Left,
                            RustOwnedNestedPrimitiveValue::Length {
                                value: 10.0,
                                unit: "px".to_string(),
                            },
                        ),
                        y: position_edge(PositionEdge::Top),
                    }),
                    RustOwnedPositionListItem::Position(RustOwnedResolvedPosition {
                        x: position_edge(PositionEdge::Center),
                        y: position_edge(PositionEdge::Center),
                    }),
                ],
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(
            &[PropertyId::BackgroundPosition],
            "calc(0% + 20px) calc(0% + 20px), calc(0% + 40px) calc(0% + 40px)"
        ),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundPosition,
            value: RustOwnedStyleValueKind::PositionList(RustOwnedPositionList {
                value_type: PropertyValueType::BackgroundPosition,
                values,
            }),
        }) if values.len() == 2
            && matches!(
                &values[0],
                RustOwnedPositionListItem::Position(RustOwnedResolvedPosition {
                    x: RustOwnedPositionComponent {
                        offset: Some(RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                            name,
                            source,
                            value_type: PropertyValueType::Length,
                            ..
                        })),
                        ..
                    },
                    y: RustOwnedPositionComponent {
                        offset: Some(RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                            name: name_y,
                            source: source_y,
                            value_type: PropertyValueType::Length,
                            ..
                        })),
                        ..
                    },
                }) if name == "calc" && source == "calc(0% + 20px)" && name_y == "calc" && source_y == "calc(0% + 20px)"
            )
            && matches!(
                &values[1],
                RustOwnedPositionListItem::Position(RustOwnedResolvedPosition {
                    x: RustOwnedPositionComponent {
                        offset: Some(RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                            name,
                            source,
                            value_type: PropertyValueType::Length,
                            ..
                        })),
                        ..
                    },
                    y: RustOwnedPositionComponent {
                        offset: Some(RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                            name: name_y,
                            source: source_y,
                            value_type: PropertyValueType::Length,
                            ..
                        })),
                        ..
                    },
                }) if name == "calc" && source == "calc(0% + 40px)" && name_y == "calc" && source_y == "calc(0% + 40px)"
            )
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundPositionX], "left 10px, center"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundPositionX,
            value: RustOwnedStyleValueKind::PositionList(RustOwnedPositionList {
                value_type: PropertyValueType::BackgroundPosition,
                values: vec![
                    RustOwnedPositionListItem::Component(position_edge_offset(
                        PositionEdge::Left,
                        RustOwnedNestedPrimitiveValue::Length {
                            value: 10.0,
                            unit: "px".to_string(),
                        },
                    )),
                    RustOwnedPositionListItem::Component(position_edge(PositionEdge::Center)),
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundPositionY], "top 20px, 50%"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundPositionY,
            value: RustOwnedStyleValueKind::PositionList(RustOwnedPositionList {
                value_type: PropertyValueType::BackgroundPosition,
                values: vec![
                    RustOwnedPositionListItem::Component(position_edge_offset(
                        PositionEdge::Top,
                        RustOwnedNestedPrimitiveValue::Length {
                            value: 20.0,
                            unit: "px".to_string(),
                        },
                    )),
                    RustOwnedPositionListItem::Component(position_offset(RustOwnedNestedPrimitiveValue::Percentage(
                        50.0
                    ),)),
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::MaskPosition], "left 10px top 20px, center"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::MaskPosition,
            value: RustOwnedStyleValueKind::PositionList(RustOwnedPositionList {
                value_type: PropertyValueType::Position,
                values: vec![
                    RustOwnedPositionListItem::Position(RustOwnedResolvedPosition {
                        x: position_edge_offset(
                            PositionEdge::Left,
                            RustOwnedNestedPrimitiveValue::Length {
                                value: 10.0,
                                unit: "px".to_string(),
                            },
                        ),
                        y: position_edge_offset(
                            PositionEdge::Top,
                            RustOwnedNestedPrimitiveValue::Length {
                                value: 20.0,
                                unit: "px".to_string(),
                            },
                        ),
                    }),
                    RustOwnedPositionListItem::Position(RustOwnedResolvedPosition {
                        x: position_edge(PositionEdge::Center),
                        y: position_edge(PositionEdge::Center),
                    }),
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Contain], "inline-size layout paint"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Contain,
            value: RustOwnedStyleValueKind::Contain(RustOwnedContain {
                value: CssContainValue {
                    kind: CssContainValueKind::List,
                    is_size: false,
                    is_inline_size: true,
                    has_layout: true,
                    has_style: false,
                    has_paint: true,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ContainerType], "inline-size scroll-state"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ContainerType,
            value: RustOwnedStyleValueKind::ContainerType(RustOwnedContainerType {
                value: CssContainerTypeValueKind::InlineSizeAndScrollState,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::CounterIncrement], "chapter page 2"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::CounterIncrement,
            value: RustOwnedStyleValueKind::CounterDefinitions(RustOwnedCounterDefinitions {
                definitions: vec![
                    RustOwnedCounterDefinition {
                        name: "chapter".to_string(),
                        is_reversed: false,
                        value: RustOwnedNestedPrimitiveValue::Integer(1),
                    },
                    RustOwnedCounterDefinition {
                        name: "page".to_string(),
                        is_reversed: false,
                        value: RustOwnedNestedPrimitiveValue::Integer(2),
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::CounterSet], "chapter -1"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::CounterSet,
            value: RustOwnedStyleValueKind::CounterDefinitions(RustOwnedCounterDefinitions {
                definitions: vec![RustOwnedCounterDefinition {
                    name: "chapter".to_string(),
                    is_reversed: false,
                    value: RustOwnedNestedPrimitiveValue::Integer(-1),
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::GridAutoFlow], "dense column"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::GridAutoFlow,
            value: RustOwnedStyleValueKind::GridAutoFlow(RustOwnedGridAutoFlow {
                axis: CssGridAutoFlowAxis::Column,
                dense: CssGridAutoFlowDense::Yes,
            }),
        })
    );
    let Some(RustOwnedStyleValue {
        property_id: PropertyId::Animation,
        value: RustOwnedStyleValueKind::CoordinatingValueListShorthand(animation_items),
    }) = parse_rust_owned_style_value(&[PropertyId::Animation], "1s ease-in 2s slide")
    else {
        panic!("Expected animation to parse as a coordinating value list shorthand");
    };
    assert_eq!(
        animation_items
            .iter()
            .map(|item| (item.layer_index, item.style_value.property_id, item.source.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, PropertyId::AnimationDuration, "1s"),
            (0, PropertyId::AnimationTimingFunction, "ease-in"),
            (0, PropertyId::AnimationDelay, "2s"),
            (0, PropertyId::AnimationName, "slide"),
        ]
    );
    let Some(RustOwnedStyleValue {
        property_id: PropertyId::Transition,
        value: RustOwnedStyleValueKind::CoordinatingValueListShorthand(transition_items),
    }) = parse_rust_owned_style_value(&[PropertyId::Transition], "opacity 200ms ease allow-discrete")
    else {
        panic!("Expected transition to parse as a coordinating value list shorthand");
    };
    assert_eq!(
        transition_items
            .iter()
            .map(|item| (item.layer_index, item.style_value.property_id, item.source.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, PropertyId::TransitionProperty, "opacity"),
            (0, PropertyId::TransitionDuration, "200ms"),
            (0, PropertyId::TransitionTimingFunction, "ease"),
            (0, PropertyId::TransitionBehavior, "allow-discrete"),
        ]
    );
    let Some(RustOwnedStyleValue {
        property_id: PropertyId::Transition,
        value: RustOwnedStyleValueKind::CoordinatingValueListShorthand(transition_items),
    }) = parse_rust_owned_style_value(&[PropertyId::Transition], "allow-discrete display 200ms")
    else {
        panic!("Expected transition to parse as a coordinating value list shorthand");
    };
    assert_eq!(
        transition_items
            .iter()
            .map(|item| (item.layer_index, item.style_value.property_id, item.source.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, PropertyId::TransitionBehavior, "allow-discrete"),
            (0, PropertyId::TransitionProperty, "display"),
            (0, PropertyId::TransitionDuration, "200ms"),
        ]
    );
    let Some(RustOwnedStyleValue {
        property_id: PropertyId::Background,
        value: RustOwnedStyleValueKind::LayerShorthand(background_items),
    }) = parse_rust_owned_style_value(&[PropertyId::Background], "url(bg.png) center / cover no-repeat red")
    else {
        panic!("Expected background to parse as a layer shorthand");
    };
    assert_eq!(
        background_items
            .iter()
            .map(|item| (item.layer_index, item.property_id, item.source.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, PropertyId::BackgroundImage, "url(bg.png)"),
            (0, PropertyId::BackgroundPosition, "center"),
            (0, PropertyId::BackgroundSize, "cover"),
            (0, PropertyId::BackgroundRepeat, "no-repeat"),
            (0, PropertyId::BackgroundColor, "red"),
        ]
    );
    let Some(RustOwnedStyleValue {
        property_id: PropertyId::Mask,
        value: RustOwnedStyleValueKind::LayerShorthand(mask_items),
    }) = parse_rust_owned_style_value(&[PropertyId::Mask], "url(mask.png) left / contain no-repeat alpha")
    else {
        panic!("Expected mask to parse as a layer shorthand");
    };
    assert_eq!(
        mask_items
            .iter()
            .map(|item| (item.layer_index, item.property_id, item.source.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, PropertyId::MaskImage, "url(mask.png)"),
            (0, PropertyId::MaskPosition, "left"),
            (0, PropertyId::MaskSize, "contain"),
            (0, PropertyId::MaskRepeat, "no-repeat"),
            (0, PropertyId::MaskMode, "alpha"),
        ]
    );
    let Some(RustOwnedStyleValue {
        property_id: PropertyId::Margin,
        value: RustOwnedStyleValueKind::PositionalValueListShorthand(margin_items),
    }) = parse_rust_owned_style_value(&[PropertyId::Margin], "1px 2% auto calc(3px + 4%)")
    else {
        panic!("Expected margin to parse as a positional value list shorthand");
    };
    assert_eq!(
        margin_items
            .iter()
            .map(|item| (item.index, item.style_value.property_id, item.source.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (0, PropertyId::MarginTop, "1px"),
            (1, PropertyId::MarginRight, "2%"),
            (2, PropertyId::MarginBottom, "auto"),
            (3, PropertyId::MarginLeft, "calc(3px + 4%)"),
        ]
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::GridColumnStart], "span 2 main"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::GridColumnStart,
            value: RustOwnedStyleValueKind::GridTrackPlacement(RustOwnedGridTrackPlacement::Span {
                line_number: Some(RustOwnedNestedPrimitiveValue::Integer(2)),
                name: Some("main".to_string()),
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::GridColumnStart], "span sibling-count()"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::GridColumnStart,
            value: RustOwnedStyleValueKind::GridTrackPlacement(RustOwnedGridTrackPlacement::Span {
                line_number: Some(RustOwnedNestedPrimitiveValue::TreeCountingFunction(
                    RustOwnedTreeCountingFunction {
                        function: RustOwnedTreeCountingFunctionKind::SiblingCount,
                        value_type: PropertyValueType::Integer,
                        ..
                    }
                )),
                name: None,
            }),
        })
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::GridColumn], "main / span 2"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::GridColumn,
            value: RustOwnedStyleValueKind::GridPlacementShorthand(vec![
                RustOwnedGridPlacementShorthandItem {
                    property_id: PropertyId::GridColumnStart,
                    value: RustOwnedGridTrackPlacement::Line {
                        line_number: None,
                        name: Some("main".to_string()),
                    },
                },
                RustOwnedGridPlacementShorthandItem {
                    property_id: PropertyId::GridColumnEnd,
                    value: RustOwnedGridTrackPlacement::Span {
                        line_number: Some(RustOwnedNestedPrimitiveValue::Integer(2)),
                        name: None,
                    },
                },
            ]),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::GridAutoRows], "10px minmax(1px, 1fr)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::GridAutoRows,
            value: RustOwnedStyleValueKind::GridAutoTrackSizes(RustOwnedGridTrackSizeList::List(vec![
                RustOwnedGridTrackSizeListItem::Track(RustOwnedExplicitGridTrack::Size(
                    RustOwnedGridTrackSize::Breadth(RustOwnedNestedPrimitiveValue::Length {
                        value: 10.0,
                        unit: "px".to_string(),
                    }),
                )),
                RustOwnedGridTrackSizeListItem::Track(RustOwnedExplicitGridTrack::Size(
                    RustOwnedGridTrackSize::MinMax {
                        min: RustOwnedNestedPrimitiveValue::Length {
                            value: 1.0,
                            unit: "px".to_string(),
                        },
                        max: RustOwnedNestedPrimitiveValue::Flex {
                            value: 1.0,
                            unit: "fr".to_string(),
                        },
                    },
                )),
            ])),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::GridAutoRows], "minmax(1px, calc(1fr + 2fr))"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::GridAutoRows,
            value: RustOwnedStyleValueKind::GridAutoTrackSizes(RustOwnedGridTrackSizeList::List(items)),
        }) if matches!(
            items.as_slice(),
            [RustOwnedGridTrackSizeListItem::Track(RustOwnedExplicitGridTrack::Size(
                RustOwnedGridTrackSize::MinMax {
                    max: RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                        name,
                        source,
                        value_type: PropertyValueType::Flex,
                        ..
                    }),
                    ..
                },
            ))] if name == "calc" && source == "calc(1fr + 2fr)"
        )
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Grid], "auto-flow dense 10px / 1fr"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Grid,
            value: RustOwnedStyleValueKind::GridTemplateShorthand(vec![
                RustOwnedGridTemplateShorthandItem {
                    property_id: PropertyId::GridAutoFlow,
                    style_value: parse_rust_owned_style_value(&[PropertyId::GridAutoFlow], "row dense").unwrap(),
                    source: "row dense".to_string(),
                },
                RustOwnedGridTemplateShorthandItem {
                    property_id: PropertyId::GridTemplateColumns,
                    style_value: parse_rust_owned_style_value(&[PropertyId::GridTemplateColumns], "1fr").unwrap(),
                    source: "1fr".to_string(),
                },
                RustOwnedGridTemplateShorthandItem {
                    property_id: PropertyId::GridAutoRows,
                    style_value: parse_rust_owned_style_value(&[PropertyId::GridAutoRows], "10px").unwrap(),
                    source: "10px".to_string(),
                },
            ]),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::GridTemplateColumns], "[a] 10px [b] repeat(2, 1fr)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::GridTemplateColumns,
            value: RustOwnedStyleValueKind::GridTrackSizeList(RustOwnedGridTrackSizeList::List(vec![
                RustOwnedGridTrackSizeListItem::LineNames(vec!["a".to_string()]),
                RustOwnedGridTrackSizeListItem::Track(RustOwnedExplicitGridTrack::Size(
                    RustOwnedGridTrackSize::Breadth(RustOwnedNestedPrimitiveValue::Length {
                        value: 10.0,
                        unit: "px".to_string(),
                    }),
                )),
                RustOwnedGridTrackSizeListItem::LineNames(vec!["b".to_string()]),
                RustOwnedGridTrackSizeListItem::Track(RustOwnedExplicitGridTrack::Repeat(RustOwnedGridRepeat {
                    repeat_type: RustOwnedGridRepeatType::Fixed {
                        count: RustOwnedNestedPrimitiveValue::Integer(2),
                    },
                    track_list: vec![RustOwnedGridTrackSizeListItem::Track(RustOwnedExplicitGridTrack::Size(
                        RustOwnedGridTrackSize::Breadth(RustOwnedNestedPrimitiveValue::Flex {
                            value: 1.0,
                            unit: "fr".to_string(),
                        })
                    ),)],
                },)),
            ])),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PaintOrder], "markers stroke"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PaintOrder,
            value: RustOwnedStyleValueKind::PaintOrder(RustOwnedPaintOrder {
                value: CssPaintOrderValue {
                    kind: CssPaintOrderValueKind::Pair,
                    first: CssPaintOrderKeyword::Markers,
                    second: CssPaintOrderKeyword::Stroke,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::MathDepth], "add(-2)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::MathDepth,
            value: RustOwnedStyleValueKind::MathDepth(RustOwnedMathDepth::Add {
                integer: RustOwnedNestedPrimitiveValue::Integer(-2),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PositionArea], "span-inline-end block-start"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PositionArea,
            value: RustOwnedStyleValueKind::PositionArea(RustOwnedPositionArea::Area {
                first_keyword: "block-start".to_string(),
                second_keyword: Some("span-inline-end".to_string()),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PositionTryFallbacks], "--foo flip-block, top left"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PositionTryFallbacks,
            value: RustOwnedStyleValueKind::PositionTryFallbacks(RustOwnedPositionTryFallbacks::List(vec![
                RustOwnedPositionTryFallback::TryTactic {
                    dashed_ident: Some("--foo".to_string()),
                    has_flip_block: true,
                    has_flip_inline: false,
                    has_flip_start: false,
                    try_tactics: vec!["flip-block".to_string()],
                },
                RustOwnedPositionTryFallback::PositionArea(RustOwnedPositionArea::Area {
                    first_keyword: "left".to_string(),
                    second_keyword: Some("top".to_string()),
                }),
            ])),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PositionTryOrder], "most-inline-size"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PositionTryOrder,
            value: RustOwnedStyleValueKind::PositionTryOrder(RustOwnedPositionTryOrder {
                value: CssPositionTryOrderValue::MostInlineSize,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::PositionVisibility], "no-overflow anchors-visible"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::PositionVisibility,
            value: RustOwnedStyleValueKind::PositionVisibility(RustOwnedPositionVisibility {
                value: CssPositionVisibilityValue {
                    kind: CssPositionVisibilityValueKind::List,
                    has_anchors_valid: false,
                    has_anchors_visible: true,
                    has_no_overflow: true,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TransformOrigin], "right 25% 3px"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TransformOrigin,
            value: RustOwnedStyleValueKind::TransformOrigin(RustOwnedTransformOrigin {
                x: RustOwnedNestedPrimitiveValue::Keyword("right".to_string()),
                y: RustOwnedNestedPrimitiveValue::Percentage(25.0),
                z: RustOwnedNestedPrimitiveValue::Length {
                    value: 3.0,
                    unit: "px".to_string(),
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundRepeat], "repeat space"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundRepeat,
            value: RustOwnedStyleValueKind::RepeatStyle(RustOwnedRepeatStyleList {
                values: vec![RustOwnedRepeatStyle {
                    repeat_x: CssRepeatStyleRepetition::Repeat,
                    repeat_y: CssRepeatStyleRepetition::Space,
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ScrollbarColor], "auto"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ScrollbarColor,
            value: RustOwnedStyleValueKind::ScrollbarColor(RustOwnedScrollbarColor::Auto),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ScrollbarColor], "red CanvasText"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ScrollbarColor,
            value: RustOwnedStyleValueKind::ScrollbarColor(RustOwnedScrollbarColor::Colors {
                thumb_color: RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                    name: Some("red".to_string()),
                },
                track_color: RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Keyword,
                    red: 0,
                    green: 0,
                    blue: 0,
                    alpha: 0,
                    name: Some("CanvasText".to_string()),
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ScrollbarGutter], "stable both-edges"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ScrollbarGutter,
            value: RustOwnedStyleValueKind::ScrollbarGutter(RustOwnedScrollbarGutter {
                value: CssScrollbarGutterValueKind::BothEdges,
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::StrokeDasharray], "2 3px, calc(4%)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::StrokeDasharray,
            value: RustOwnedStyleValueKind::StrokeDasharray(RustOwnedStrokeDasharray::Values(values)),
        }) if values.len() == 3
            && matches!(&values[0], RustOwnedNestedPrimitiveValue::Number(value) if *value == 2.0)
            && matches!(
                &values[1],
                RustOwnedNestedPrimitiveValue::Length { value, unit } if *value == 3.0 && unit == "px"
            )
            && matches!(
                &values[2],
                RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                    name,
                    source,
                    value_type: PropertyValueType::LengthPercentage,
                    ..
                }) if name == "calc" && source == "calc(4%)"
            )
    ));
    assert_eq!(
        super::component_value_parse_as_nested_number_percentage(&parse("sibling-count()")[0], "sibling-count()"),
        Some(RustOwnedNestedPrimitiveValue::TreeCountingFunction(
            RustOwnedTreeCountingFunction {
                function: RustOwnedTreeCountingFunctionKind::SiblingCount,
                value_type: PropertyValueType::Number,
                component_values: parse("sibling-count()"),
            }
        ))
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnimationName], "foo, \"none\", Both"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnimationName,
            value: RustOwnedStyleValueKind::AnimationName(RustOwnedAnimationName {
                kind: CssAnimationNameValueKind::List,
                names: vec![
                    RustOwnedAnimationNameItem {
                        kind: CssAnimationNameItemKind::CustomIdent,
                        value: "foo".to_string(),
                    },
                    RustOwnedAnimationNameItem {
                        kind: CssAnimationNameItemKind::String,
                        value: "none".to_string(),
                    },
                    RustOwnedAnimationNameItem {
                        kind: CssAnimationNameItemKind::CustomIdent,
                        value: "Both".to_string(),
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnimationDelay], "1s, -200ms"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnimationDelay,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Time,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Time,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnimationDuration], "auto, 250ms"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnimationDuration,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Time,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Time,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnimationDirection], "normal, alternate-reverse"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnimationDirection,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::CustomIdent,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::CustomIdent,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnimationIterationCount], "infinite, 2"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnimationIterationCount,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Number,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Number,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnimationPlayState], "running, paused"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnimationPlayState,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::CustomIdent,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::CustomIdent,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::AnimationTimeline], "auto, --track"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::AnimationTimeline,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::DashedIdent,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::DashedIdent,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TransitionTimingFunction], "ease, ease-in"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TransitionTimingFunction,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::EasingFunction,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::EasingFunction,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundImage], "none, url(example.png)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundImage,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Image,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Image,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::MaskImage], "url(#mask), none"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::MaskImage,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Url,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::Image,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::BackgroundAttachment], "fixed, local"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::BackgroundAttachment,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::CustomIdent,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::CustomIdent,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::MaskComposite], "add, subtract"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::MaskComposite,
            value: RustOwnedStyleValueKind::GeneratedValueList(RustOwnedGeneratedValueList {
                items: vec![
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::CustomIdent,
                    },
                    RustOwnedGeneratedValueListItem {
                        value_type: PropertyValueType::CustomIdent,
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TransitionDuration], "-1s"),
        None
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ScrollTimelineName], "none, --track"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ScrollTimelineName,
            value: RustOwnedStyleValueKind::TimelineName(RustOwnedTimelineName {
                kind: CssTimelineNameValueKind::List,
                names: vec![
                    RustOwnedTimelineNameItem {
                        kind: CssTimelineNameItemKind::None,
                        name: String::new(),
                    },
                    RustOwnedTimelineNameItem {
                        kind: CssTimelineNameItemKind::DashedIdent,
                        name: "--track".to_string(),
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ScrollTimeline], "--track inline, none"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ScrollTimeline,
            value: RustOwnedStyleValueKind::ScrollTimeline(RustOwnedScrollTimeline {
                names: vec![
                    RustOwnedTimelineNameItem {
                        kind: CssTimelineNameItemKind::DashedIdent,
                        name: "--track".to_string(),
                    },
                    RustOwnedTimelineNameItem {
                        kind: CssTimelineNameItemKind::None,
                        name: String::new(),
                    },
                ],
                axes: vec![CssScrollFunctionAxisKind::Inline, CssScrollFunctionAxisKind::Block],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ViewTimelineName], "--view"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ViewTimelineName,
            value: RustOwnedStyleValueKind::TimelineName(RustOwnedTimelineName {
                kind: CssTimelineNameValueKind::List,
                names: vec![RustOwnedTimelineNameItem {
                    kind: CssTimelineNameItemKind::DashedIdent,
                    name: "--view".to_string(),
                }],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::ViewTimeline], "--view 1px inline, none"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::ViewTimeline,
            value: RustOwnedStyleValueKind::ViewTimeline(RustOwnedViewTimeline {
                names: vec![
                    RustOwnedTimelineNameItem {
                        kind: CssTimelineNameItemKind::DashedIdent,
                        name: "--view".to_string(),
                    },
                    RustOwnedTimelineNameItem {
                        kind: CssTimelineNameItemKind::None,
                        name: String::new(),
                    },
                ],
                axes: vec![CssScrollFunctionAxisKind::Inline, CssScrollFunctionAxisKind::Block],
                insets: vec![
                    vec![RustOwnedNestedPrimitiveValue::Length {
                        value: 1.0,
                        unit: "px".to_string(),
                    }],
                    vec![auto_keyword()],
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TransitionBehavior], "normal, allow-discrete"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TransitionBehavior,
            value: RustOwnedStyleValueKind::TransitionBehavior(RustOwnedTransitionBehavior {
                kind: CssTransitionBehaviorValueKind::List,
                behaviors: vec![
                    CssTransitionBehaviorItemKind::Normal,
                    CssTransitionBehaviorItemKind::AllowDiscrete,
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TransitionProperty], "all, opacity"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TransitionProperty,
            value: RustOwnedStyleValueKind::TransitionProperty(RustOwnedTransitionProperty {
                kind: CssTransitionPropertyValueKind::List,
                properties: vec!["all".to_string(), "opacity".to_string()],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextWrap], "pretty nowrap"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextWrap,
            value: RustOwnedStyleValueKind::TextWrap(RustOwnedTextWrap {
                value: CssTextWrapValue {
                    kind: CssTextWrapValueKind::Valid,
                    mode: CssTextWrapModeValue::Nowrap,
                    style: CssTextWrapStyleValue::Pretty,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::WhiteSpace], "preserve nowrap discard-inner"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::WhiteSpace,
            value: RustOwnedStyleValueKind::WhiteSpace(RustOwnedWhiteSpace {
                white_space_collapse: "preserve".to_string(),
                text_wrap_mode: CssTextWrapModeValue::Nowrap,
                white_space_trim: CssWhiteSpaceTrimValue {
                    kind: CssWhiteSpaceTrimValueKind::List,
                    has_discard_before: false,
                    has_discard_after: false,
                    has_discard_inner: true,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::WhiteSpace], "pre-line"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::WhiteSpace,
            value: RustOwnedStyleValueKind::WhiteSpace(RustOwnedWhiteSpace {
                white_space_collapse: "preserve-breaks".to_string(),
                text_wrap_mode: CssTextWrapModeValue::Wrap,
                white_space_trim: CssWhiteSpaceTrimValue {
                    kind: CssWhiteSpaceTrimValueKind::None,
                    has_discard_before: false,
                    has_discard_after: false,
                    has_discard_inner: false,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextWrapMode], "nowrap"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextWrapMode,
            value: RustOwnedStyleValueKind::TextWrapMode(RustOwnedTextWrapMode {
                value: CssTextWrapModeValue::Nowrap,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextWrapStyle], "balance"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextWrapStyle,
            value: RustOwnedStyleValueKind::TextWrapStyle(RustOwnedTextWrapStyle {
                value: CssTextWrapStyleValue::Balance,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextDecorationLine], "overline underline"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextDecorationLine,
            value: RustOwnedStyleValueKind::TextDecorationLine(RustOwnedTextDecorationLine {
                bits: TEXT_DECORATION_LINE_UNDERLINE | TEXT_DECORATION_LINE_OVERLINE,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextDecoration], "overline green from-font"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextDecoration,
            value: RustOwnedStyleValueKind::TextDecoration(RustOwnedTextDecoration {
                line: Some(RustOwnedTextDecorationLine {
                    bits: TEXT_DECORATION_LINE_OVERLINE,
                }),
                thickness: Some(RustOwnedNestedPrimitiveValue::Keyword("from-font".to_string())),
                style: None,
                color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 0,
                    green: 128,
                    blue: 0,
                    alpha: 255,
                    name: Some("green".to_string()),
                }),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(
            &[PropertyId::TextDecoration],
            "underline overline line-through blink red"
        ),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextDecoration,
            value: RustOwnedStyleValueKind::TextDecoration(RustOwnedTextDecoration {
                line: Some(RustOwnedTextDecorationLine {
                    bits: TEXT_DECORATION_LINE_UNDERLINE
                        | TEXT_DECORATION_LINE_OVERLINE
                        | TEXT_DECORATION_LINE_LINE_THROUGH
                        | TEXT_DECORATION_LINE_BLINK,
                }),
                thickness: None,
                style: None,
                color: Some(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red: 255,
                    green: 0,
                    blue: 0,
                    alpha: 255,
                    name: Some("red".to_string()),
                }),
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(
            &[PropertyId::TextDecoration],
            "underline overline line-through rgb(255, 0, 0)"
        ),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextDecoration,
            value: RustOwnedStyleValueKind::TextDecoration(RustOwnedTextDecoration {
                color: Some(RustOwnedColor::Function {
                    name,
                    source,
                    ..
                }),
                ..
            }),
        }) if name == "rgb" && source == "rgb(255, 0, 0)"
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextIndent], "hanging 2em each-line"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextIndent,
            value: RustOwnedStyleValueKind::TextIndent(RustOwnedTextIndent {
                length_percentage: RustOwnedNestedPrimitiveValue::Length {
                    value: 2.0,
                    unit: "em".to_string(),
                },
                has_hanging: true,
                has_each_line: true,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextIndent], "10% each-line hanging"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextIndent,
            value: RustOwnedStyleValueKind::TextIndent(RustOwnedTextIndent {
                length_percentage: RustOwnedNestedPrimitiveValue::Percentage(10.0),
                has_hanging: true,
                has_each_line: true,
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TextUnderlinePosition], "under right"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TextUnderlinePosition,
            value: RustOwnedStyleValueKind::TextUnderlinePosition(RustOwnedTextUnderlinePosition {
                value: CssTextUnderlinePositionValue {
                    horizontal: CssTextUnderlinePositionHorizontal::Under,
                    vertical: CssTextUnderlinePositionVertical::Right,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::TouchAction], "pan-y pan-left"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::TouchAction,
            value: RustOwnedStyleValueKind::TouchAction(RustOwnedTouchAction {
                value: CssTouchActionValue {
                    kind: CssTouchActionValueKind::List,
                    first: CssTouchActionKeyword::PanLeft,
                    second: CssTouchActionKeyword::PanY,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::WhiteSpaceTrim], "discard-inner discard-before"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::WhiteSpaceTrim,
            value: RustOwnedStyleValueKind::WhiteSpaceTrim(RustOwnedWhiteSpaceTrim {
                value: CssWhiteSpaceTrimValue {
                    kind: CssWhiteSpaceTrimValueKind::List,
                    has_discard_before: true,
                    has_discard_after: false,
                    has_discard_inner: true,
                },
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Transform], "translateX(10px) scale(2)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Transform,
            value: RustOwnedStyleValueKind::ValueList(RustOwnedStyleValueList {
                values: vec![
                    RustOwnedStyleValueKind::Transformation(RustOwnedTransformation {
                        function: TransformFunction::TranslateX,
                        arguments: vec![RustOwnedTransformationArgument {
                            parameter_type: TransformFunctionParameterType::LengthPercentage,
                            value: RustOwnedNestedPrimitiveValue::Length {
                                value: 10.0,
                                unit: "px".to_string(),
                            },
                        }],
                    }),
                    RustOwnedStyleValueKind::Transformation(RustOwnedTransformation {
                        function: TransformFunction::Scale,
                        arguments: vec![RustOwnedTransformationArgument {
                            parameter_type: TransformFunctionParameterType::NumberPercentage,
                            value: RustOwnedNestedPrimitiveValue::Number(2.0),
                        }],
                    }),
                ],
                separator: RustOwnedStyleValueListSeparator::Space,
                value_type: Some(PropertyValueType::TransformList),
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Translate], "10px 20% 1em"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Translate,
            value: RustOwnedStyleValueKind::TransformLonghand(RustOwnedTransformLonghand::Function {
                function: RustOwnedTransformLonghandFunction::Translate3d,
                arguments: vec![
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::LengthPercentage,
                        value: RustOwnedNestedPrimitiveValue::Length {
                            value: 10.0,
                            unit: "px".to_string(),
                        },
                    },
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::LengthPercentage,
                        value: RustOwnedNestedPrimitiveValue::Percentage(20.0),
                    },
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::Length,
                        value: RustOwnedNestedPrimitiveValue::Length {
                            value: 1.0,
                            unit: "em".to_string(),
                        },
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Scale], "1 50% 2"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Scale,
            value: RustOwnedStyleValueKind::TransformLonghand(RustOwnedTransformLonghand::Function {
                function: RustOwnedTransformLonghandFunction::Scale3d,
                arguments: vec![
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::NumberPercentage,
                        value: RustOwnedNestedPrimitiveValue::Number(1.0),
                    },
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::NumberPercentage,
                        value: RustOwnedNestedPrimitiveValue::Percentage(50.0),
                    },
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::NumberPercentage,
                        value: RustOwnedNestedPrimitiveValue::Number(2.0),
                    },
                ],
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::Scale], "random(0, 10, 5)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Scale,
            value: RustOwnedStyleValueKind::TransformLonghand(RustOwnedTransformLonghand::Function {
                function: RustOwnedTransformLonghandFunction::Scale,
                arguments,
            }),
        }) if arguments.len() == 1
            && matches!(
                &arguments[0],
                RustOwnedTransformationArgument {
                    parameter_type: TransformFunctionParameterType::NumberPercentage,
                    value: RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                        name,
                        source,
                        value_type: PropertyValueType::Number,
                        ..
                    }),
                } if name == "random" && source == "random(0, 10, 5)"
            )
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::Rotate], "1 0 0 45deg"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::Rotate,
            value: RustOwnedStyleValueKind::TransformLonghand(RustOwnedTransformLonghand::Function {
                function: RustOwnedTransformLonghandFunction::Rotate3d,
                arguments: vec![
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::Number,
                        value: RustOwnedNestedPrimitiveValue::Number(1.0),
                    },
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::Number,
                        value: RustOwnedNestedPrimitiveValue::Number(0.0),
                    },
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::Number,
                        value: RustOwnedNestedPrimitiveValue::Number(0.0),
                    },
                    RustOwnedTransformationArgument {
                        parameter_type: TransformFunctionParameterType::Angle,
                        value: RustOwnedNestedPrimitiveValue::Angle {
                            value: 45.0,
                            unit: "deg".to_string(),
                        },
                    },
                ],
            }),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontStyle], "oblique 10deg"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontStyle,
            value: RustOwnedStyleValueKind::FontStyle(RustOwnedFontStyle {
                value: FontStyle::Oblique { has_angle: true },
                angle: Some(RustOwnedNestedPrimitiveValue::Angle {
                    value: 10.0,
                    unit: "deg".to_string(),
                }),
            }),
        })
    );
    assert!(matches!(
        parse_rust_owned_style_value(&[PropertyId::FontStyle], "oblique calc(100deg)"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontStyle,
            value: RustOwnedStyleValueKind::FontStyle(RustOwnedFontStyle {
                value: FontStyle::Oblique { has_angle: true },
                angle: Some(RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                    ref name,
                    ref source,
                    value_type: PropertyValueType::Angle,
                    ..
                })),
            }),
        }) if name == "calc" && source == "calc(100deg)"
    ));
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontVariantNumeric], "tabular-nums slashed-zero"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontVariantNumeric,
            value: RustOwnedStyleValueKind::FontVariantLonghand(RustOwnedFontVariantLonghand::Numeric(vec![
                FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Spacing,
                    value: "tabular-nums".to_string(),
                },
                FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::SlashedZero,
                    value: "slashed-zero".to_string(),
                },
            ])),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontVariantAlternates], "stylistic(foo) historical-forms"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontVariantAlternates,
            value: RustOwnedStyleValueKind::FontVariantLonghand(RustOwnedFontVariantLonghand::Alternates(vec![
                FontVariantAlternatesValue {
                    kind: CssFontVariantAlternatesValueKind::Stylistic,
                    feature_value_names: vec!["foo".to_string()],
                },
                FontVariantAlternatesValue {
                    kind: CssFontVariantAlternatesValueKind::HistoricalForms,
                    feature_value_names: Vec::new(),
                },
            ])),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(&[PropertyId::FontVariantEastAsian], "jis78 proportional-width ruby"),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontVariantEastAsian,
            value: RustOwnedStyleValueKind::FontVariantLonghand(RustOwnedFontVariantLonghand::EastAsian(vec![
                FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Variant,
                    value: "jis78".to_string(),
                },
                FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Width,
                    value: "proportional-width".to_string(),
                },
                FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Ruby,
                    value: "ruby".to_string(),
                },
            ])),
        })
    );
    assert_eq!(
        parse_rust_owned_style_value(
            &[PropertyId::FontVariantLigatures],
            "common-ligatures no-discretionary-ligatures"
        ),
        Some(RustOwnedStyleValue {
            property_id: PropertyId::FontVariantLigatures,
            value: RustOwnedStyleValueKind::FontVariantLonghand(RustOwnedFontVariantLonghand::Ligatures(vec![
                FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Common,
                    value: "common-ligatures".to_string(),
                },
                FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Discretionary,
                    value: "no-discretionary-ligatures".to_string(),
                },
            ])),
        })
    );

    assert_eq!(
        parse_style_value(&[PropertyId::Color], "red"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Color,
            property_id: PropertyId::Color,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: Some((255, 0, 0, 255)),
            value: "red".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(parse_style_value(&[PropertyId::Color], "123abc"), None);
    assert_eq!(
        parse_style_value_with_options(
            &[PropertyId::Color],
            "123abc",
            CssPrimitiveValueOptions {
                allow_quirky_length: false,
                allow_quirky_color: true,
                allow_svg_unitless_length: false,
                allow_svg_unitless_angle: false,
            }
        ),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Color,
            property_id: PropertyId::Color,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: Some((0x12, 0x3a, 0xbc, 255)),
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::BorderTopColor], "#336699cc"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Color,
            property_id: PropertyId::BorderTopColor,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: Some((0x33, 0x66, 0x99, 0xcc)),
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Outline], "auto red thick"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Keyword,
            property_id: PropertyId::OutlineWidth,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "thick".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OverflowBlock], "overlay"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Keyword,
            property_id: PropertyId::OverflowBlock,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "auto".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OverflowInline], "clip"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Keyword,
            property_id: PropertyId::OverflowInline,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "clip".to_string(),
            value_type: String::new(),
        })
    );
    for property_id in [
        PropertyId::AccentColor,
        PropertyId::CaretColor,
        PropertyId::FloodColor,
        PropertyId::OutlineColor,
        PropertyId::StopColor,
        PropertyId::TextDecorationColor,
    ] {
        assert_eq!(
            parse_style_value(&[property_id], "red"),
            Some(ParsedStyleValue {
                kind: CssStyleValueKind::Color,
                property_id,
                primitive_kind: CssPrimitiveValueKind::Invalid,
                numeric_value: None,
                secondary_numeric_value: None,
                color: Some((255, 0, 0, 255)),
                value: "red".to_string(),
                value_type: String::new(),
            })
        );
    }
    assert_eq!(
        parse_style_value(&[PropertyId::CaretColor], "auto"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Keyword,
            property_id: PropertyId::CaretColor,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "auto".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(parse_style_value(&[PropertyId::TextDecorationColor], "red blue"), None);
    for (property_id, input, keyword) in rust_owned_keyword_longhands {
        assert_eq!(
            parse_style_value(&[property_id], input),
            Some(ParsedStyleValue {
                kind: CssStyleValueKind::Keyword,
                property_id,
                primitive_kind: CssPrimitiveValueKind::Invalid,
                numeric_value: None,
                secondary_numeric_value: None,
                color: None,
                value: keyword.to_string(),
                value_type: String::new(),
            })
        );
        assert_eq!(parse_style_value(&[property_id], &format!("{input} {input}")), None);
    }
    for (property_id, input, primitive_kind, numeric_value, value, value_type) in [
        (
            PropertyId::FlexShrink,
            "3",
            CssPrimitiveValueKind::Number,
            3.0,
            "",
            "Number",
        ),
        (
            PropertyId::StrokeMiterlimit,
            "4",
            CssPrimitiveValueKind::Number,
            4.0,
            "",
            "Number",
        ),
    ] {
        assert_eq!(
            parse_style_value(&[property_id], input),
            Some(ParsedStyleValue {
                kind: CssStyleValueKind::Primitive,
                property_id,
                primitive_kind,
                numeric_value: Some(numeric_value),
                secondary_numeric_value: None,
                color: None,
                value: value.to_string(),
                value_type: value_type.to_string(),
            })
        );
    }
    for property_id in [
        PropertyId::FillOpacity,
        PropertyId::FloodOpacity,
        PropertyId::Opacity,
        PropertyId::ShapeImageThreshold,
        PropertyId::StopOpacity,
        PropertyId::StrokeOpacity,
    ] {
        assert_eq!(
            parse_style_value(&[property_id], "25%"),
            Some(ParsedStyleValue {
                kind: CssStyleValueKind::Primitive,
                property_id,
                primitive_kind: CssPrimitiveValueKind::Percentage,
                numeric_value: Some(25.0),
                secondary_numeric_value: None,
                color: None,
                value: String::new(),
                value_type: "OpacityValue".to_string(),
            })
        );
    }
    for (property_id, input, numeric_value) in [
        (PropertyId::ColumnCount, "3", 3.0),
        (PropertyId::Order, "-2147483649", i32::MIN as f64),
        (PropertyId::Orphans, "1", 1.0),
        (PropertyId::Widows, "2", 2.0),
        (PropertyId::ZIndex, "2147483648", i32::MAX as f64),
    ] {
        assert_eq!(
            parse_style_value(&[property_id], input),
            Some(ParsedStyleValue {
                kind: CssStyleValueKind::Primitive,
                property_id,
                primitive_kind: CssPrimitiveValueKind::Integer,
                numeric_value: Some(numeric_value),
                secondary_numeric_value: None,
                color: None,
                value: String::new(),
                value_type: "Integer".to_string(),
            })
        );
    }
    assert_eq!(
        parse_style_value(&[PropertyId::ZIndex], "auto"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Keyword,
            property_id: PropertyId::ZIndex,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "auto".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(parse_style_value(&[PropertyId::ColumnCount], "0"), None);
    assert_eq!(parse_style_value(&[PropertyId::Orphans], "0"), None);
    assert_eq!(
        parse_style_value(&[PropertyId::Color], "currentColor"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::Color,
            primitive_kind: CssPrimitiveValueKind::Keyword,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "currentColor".to_string(),
            value_type: "Color".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Color], "color-mix(in oklab, red 40%, blue)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::ColorFunction,
            property_id: PropertyId::Color,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "color-mix(in oklab, red 40%, blue)".to_string(),
            value_type: "Color".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AnchorName], "--anchor"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::AnchorNameOrScope,
            property_id: PropertyId::AnchorName,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "--anchor".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::FontFeatureSettings], "\"kern\""),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::FontFeatureSettings,
            property_id: PropertyId::FontFeatureSettings,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "kern".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ListStyleType], "disc"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::CounterStyleName,
            property_id: PropertyId::ListStyleType,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "disc".to_string(),
            value_type: "CounterStyle".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ListStyleType], "symbols(\"*\" \"**\")"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::CounterStyle,
            property_id: PropertyId::ListStyleType,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "**".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ClipPath], "url(image.png)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Url,
            property_id: PropertyId::ClipPath,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "image.png".to_string(),
            value_type: "Url".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::BackgroundImage], "url(example.png)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::GeneratedValueList,
            property_id: PropertyId::BackgroundImage,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "Image".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::BackgroundImage], "linear-gradient(black, white)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::GeneratedValueList,
            property_id: PropertyId::BackgroundImage,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "Image".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::BackgroundImage], "image-set(url(example.png) 2x)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::GeneratedValueList,
            property_id: PropertyId::BackgroundImage,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "Image".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OffsetDistance], "12px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::OffsetDistance,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(12.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: "LengthPercentage".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OffsetDistance], "0%"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::OffsetDistance,
            primitive_kind: CssPrimitiveValueKind::Percentage,
            numeric_value: Some(0.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "LengthPercentage".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OffsetPosition], "auto"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Keyword,
            property_id: PropertyId::OffsetPosition,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "auto".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OffsetAnchor], "center"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Position,
            property_id: PropertyId::OffsetAnchor,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OffsetRotate], "45deg"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::OffsetRotate,
            primitive_kind: CssPrimitiveValueKind::Angle,
            numeric_value: Some(45.0),
            secondary_numeric_value: None,
            color: None,
            value: "deg".to_string(),
            value_type: "Angle".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OffsetPath], "path(\"M 1 1\")"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::BasicShape,
            property_id: PropertyId::OffsetPath,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "M 1 1".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::TransitionTimingFunction], "linear(0, 1)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::GeneratedValueList,
            property_id: PropertyId::TransitionTimingFunction,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "EasingFunction".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Width], "fit-content(10px)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::FitContent,
            property_id: PropertyId::Width,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(10.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ClipPath], "inset(10px)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::BasicShape,
            property_id: PropertyId::ClipPath,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(10.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Clip], "rect(1px, auto, 2px, 3px)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Rect,
            property_id: PropertyId::Clip,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(3.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AnimationTimeline], "scroll(root y)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::GeneratedValueList,
            property_id: PropertyId::AnimationTimeline,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "ScrollFunction".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ViewTimelineInset], "1px 2px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::ViewTimelineInset,
            property_id: PropertyId::ViewTimelineInset,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(2.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::BorderSpacing], "1px 2px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::BorderSpacing,
            property_id: PropertyId::BorderSpacing,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(2.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::TextOverflow], "ellipsis"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Keyword,
            property_id: PropertyId::TextOverflow,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "ellipsis".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ScrollTimeline], "--track inline, none"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::ScrollTimeline,
            property_id: PropertyId::ScrollTimeline,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "\u{1}--track\0\u{0}\0".to_string(),
            value_type: "\u{2}\u{1}".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ViewTimeline], "--view 1px inline, none"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::ViewTimeline,
            property_id: PropertyId::ViewTimeline,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "ViewTimelineInset".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::WhiteSpace], "preserve nowrap discard-inner"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::WhiteSpace,
            property_id: PropertyId::WhiteSpace,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "preserve".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AnimationTimeline], "view(y 1px 2px)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::GeneratedValueList,
            property_id: PropertyId::AnimationTimeline,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "ViewFunction".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Color, PropertyId::Display], "block"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Display,
            property_id: PropertyId::Display,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AnimationName], "slide"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::AnimationName,
            property_id: PropertyId::AnimationName,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "\u{1}slide\0".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::FontWeight], "bold"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::FontWeight,
            primitive_kind: CssPrimitiveValueKind::Keyword,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "bold".to_string(),
            value_type: "FontWeightAbsolute".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::FontWeight], "700"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::FontWeight,
            primitive_kind: CssPrimitiveValueKind::Number,
            numeric_value: Some(700.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "Number".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::FontWeight], "sibling-count()"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::TreeCountingFunction,
            property_id: PropertyId::FontWeight,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "Number".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::FontKerning], "normal"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::FontKerning,
            primitive_kind: CssPrimitiveValueKind::Keyword,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "normal".to_string(),
            value_type: "FontKerningValue".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ZIndex], "12"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::ZIndex,
            primitive_kind: CssPrimitiveValueKind::Integer,
            numeric_value: Some(12.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "Integer".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Opacity], "50%"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::Opacity,
            primitive_kind: CssPrimitiveValueKind::Percentage,
            numeric_value: Some(50.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "OpacityValue".to_string(),
        })
    );
    for (property_id, source) in [
        (PropertyId::OverflowWrap, "anywhere"),
        (PropertyId::ScrollBehavior, "smooth"),
        (PropertyId::TransformStyle, "preserve-3d"),
        (PropertyId::WordBreak, "break-all"),
    ] {
        assert_eq!(
            parse_style_value(&[property_id], source),
            Some(ParsedStyleValue {
                kind: CssStyleValueKind::Keyword,
                property_id,
                primitive_kind: CssPrimitiveValueKind::Invalid,
                numeric_value: None,
                secondary_numeric_value: None,
                color: None,
                value: source.to_string(),
                value_type: String::new(),
            })
        );
    }
    assert_eq!(parse_style_value(&[PropertyId::WordBreak], "break all"), None);
    assert_eq!(
        parse_style_value(&[PropertyId::BackgroundPositionX], "50%"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Position,
            property_id: PropertyId::BackgroundPositionX,
            primitive_kind: CssPrimitiveValueKind::Percentage,
            numeric_value: Some(50.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::MarginLeft], "12px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::MarginLeft,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(12.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: "Length".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::MarginLeft], "calc(1px + 2px)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::MathFunction,
            property_id: PropertyId::MarginLeft,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "calc(1px + 2px)".to_string(),
            value_type: "Length".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Top], "anchor(--target bottom, calc(1px + 2%))"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Anchor,
            property_id: PropertyId::Top,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "".to_string(),
            value_type: "Anchor".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Top], "auto"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Keyword,
            property_id: PropertyId::Top,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "auto".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::InsetBlockStart], "12px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::InsetBlockStart,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(12.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: "Length".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::InsetInlineEnd], "25%"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::InsetInlineEnd,
            primitive_kind: CssPrimitiveValueKind::Percentage,
            numeric_value: Some(25.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "Percentage".to_string(),
        })
    );
    assert_eq!(parse_style_value(&[PropertyId::Top], "red"), None);
    assert_eq!(
        parse_style_value(&[PropertyId::Width], "anchor-size(--target width, 10px)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::AnchorSize,
            property_id: PropertyId::Width,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "".to_string(),
            value_type: "Length".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::MarginLeft], "0"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::MarginLeft,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(0.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: "Length".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AnimationDuration], "250ms"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::GeneratedValueList,
            property_id: PropertyId::AnimationDuration,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: "Time".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AnimationName], "\"slide\""),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::AnimationName,
            property_id: PropertyId::AnimationName,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "\u{2}slide\0".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AspectRatio], "16 / 9"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::AspectRatio,
            property_id: PropertyId::AspectRatio,
            primitive_kind: CssPrimitiveValueKind::Number,
            numeric_value: Some(9.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AspectRatio], "1"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::AspectRatio,
            property_id: PropertyId::AspectRatio,
            primitive_kind: CssPrimitiveValueKind::Number,
            numeric_value: Some(1.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::BorderRadius], "1px / 2px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::BorderRadius,
            property_id: PropertyId::BorderRadius,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(2.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Columns], "3 12em / auto"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Columns,
            property_id: PropertyId::Columns,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OverflowClipMargin], "2px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::OverflowClipMargin,
            property_id: PropertyId::OverflowClipMargin,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(2.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::OverflowClipMarginTop], "2px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::OverflowClipMargin,
            property_id: PropertyId::OverflowClipMarginTop,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(2.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ShapeOutside], "circle(10px) border-box"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::ShapeOutside,
            property_id: PropertyId::ShapeOutside,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::TextDecoration], "underline red 2px"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::TextDecoration,
            property_id: PropertyId::TextDecoration,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: Some(3.0),
            secondary_numeric_value: Some(CssParsedColorKind::Rgba as u8 as f64),
            color: None,
            value: "red".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::TextDecorationLine], "overline underline"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::TextDecorationLine,
            property_id: PropertyId::TextDecorationLine,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::ListStyle], "inside url(marker.png) square"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::ListStyle,
            property_id: PropertyId::ListStyle,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "square".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Filter], "blur(10px) opacity(50%)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::FilterValueList,
            property_id: PropertyId::Filter,
            primitive_kind: CssPrimitiveValueKind::Percentage,
            numeric_value: Some(50.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Content], "\"(\" counter(item) \")\""),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Content,
            property_id: PropertyId::Content,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: ")".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Flex], "1 1 10em"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Flex,
            property_id: PropertyId::Flex,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(10.0),
            secondary_numeric_value: None,
            color: None,
            value: "em".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::Transform], "translateX(10px) scale(2)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Transformation,
            property_id: PropertyId::Transform,
            primitive_kind: CssPrimitiveValueKind::Number,
            numeric_value: Some(2.0),
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::FlexFlow], "wrap row-reverse"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::FlexFlow,
            property_id: PropertyId::FlexFlow,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: String::new(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::PlaceContent], "space-between center"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::PlaceContent,
            property_id: PropertyId::PlaceContent,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "center".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::PlaceItems], "normal start"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::PlaceItems,
            property_id: PropertyId::PlaceItems,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "start".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::PlaceSelf], "safe end unsafe right"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::PlaceSelf,
            property_id: PropertyId::PlaceSelf,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "right".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::AlignContent], "safe center"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::KeywordList,
            property_id: PropertyId::AlignContent,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "center".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::JustifyItems], "legacy right"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::KeywordList,
            property_id: PropertyId::JustifyItems,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "right".to_string(),
            value_type: String::new(),
        })
    );
    assert_eq!(parse_style_value(&[PropertyId::Color], "10px"), None);
}

#[test]
fn parses_coordinating_value_list_shorthands() {
    assert_eq!(
        parse_coordinating_shorthand(
            &[
                PropertyId::TransitionProperty,
                PropertyId::TransitionDuration,
                PropertyId::TransitionTimingFunction,
                PropertyId::TransitionDelay,
                PropertyId::TransitionBehavior,
            ],
            "opacity 1s ease-in 250ms allow-discrete, transform 2s"
        ),
        Some(vec![
            (0, PropertyId::TransitionProperty, "opacity".to_string()),
            (0, PropertyId::TransitionDuration, "1s".to_string()),
            (0, PropertyId::TransitionTimingFunction, "ease-in".to_string()),
            (0, PropertyId::TransitionDelay, "250ms".to_string()),
            (0, PropertyId::TransitionBehavior, "allow-discrete".to_string()),
            (1, PropertyId::TransitionProperty, "transform".to_string()),
            (1, PropertyId::TransitionDuration, "2s".to_string()),
        ])
    );
    let rust_items = parse_rust_owned_coordinating_shorthand(
        &[
            PropertyId::TransitionProperty,
            PropertyId::TransitionDuration,
            PropertyId::TransitionTimingFunction,
            PropertyId::TransitionDelay,
            PropertyId::TransitionBehavior,
        ],
        "opacity 1s ease-in 250ms allow-discrete, transform 2s",
    )
    .unwrap();
    assert_eq!(
        rust_items
            .iter()
            .map(|item| (item.layer_index, item.style_value.property_id, item.source.clone()))
            .collect::<Vec<_>>(),
        vec![
            (0, PropertyId::TransitionProperty, "opacity".to_string()),
            (0, PropertyId::TransitionDuration, "1s".to_string()),
            (0, PropertyId::TransitionTimingFunction, "ease-in".to_string()),
            (0, PropertyId::TransitionDelay, "250ms".to_string()),
            (0, PropertyId::TransitionBehavior, "allow-discrete".to_string()),
            (1, PropertyId::TransitionProperty, "transform".to_string()),
            (1, PropertyId::TransitionDuration, "2s".to_string()),
        ]
    );
    assert_eq!(
        rust_items[1].style_value,
        RustOwnedStyleValue {
            property_id: PropertyId::TransitionDuration,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Time {
                    value: 1.0,
                    unit: "s".to_string(),
                },
                value_type: PropertyValueType::Time,
            }),
        }
    );
    assert_eq!(
        parse_coordinating_shorthand(
            &[
                PropertyId::AnimationDuration,
                PropertyId::AnimationTimingFunction,
                PropertyId::AnimationDelay,
                PropertyId::AnimationIterationCount,
                PropertyId::AnimationDirection,
                PropertyId::AnimationFillMode,
                PropertyId::AnimationPlayState,
                PropertyId::AnimationName,
            ],
            "1s ease-in 250ms 2 reverse both paused slide, 2s fade"
        ),
        Some(vec![
            (0, PropertyId::AnimationDuration, "1s".to_string()),
            (0, PropertyId::AnimationTimingFunction, "ease-in".to_string()),
            (0, PropertyId::AnimationDelay, "250ms".to_string()),
            (0, PropertyId::AnimationIterationCount, "2".to_string()),
            (0, PropertyId::AnimationDirection, "reverse".to_string()),
            (0, PropertyId::AnimationFillMode, "both".to_string()),
            (0, PropertyId::AnimationPlayState, "paused".to_string()),
            (0, PropertyId::AnimationName, "slide".to_string()),
            (1, PropertyId::AnimationDuration, "2s".to_string()),
            (1, PropertyId::AnimationName, "fade".to_string()),
        ])
    );
    assert_eq!(
        parse_coordinating_shorthand(
            &[
                PropertyId::AnimationDuration,
                PropertyId::AnimationTimingFunction,
                PropertyId::AnimationDelay,
                PropertyId::AnimationIterationCount,
                PropertyId::AnimationDirection,
                PropertyId::AnimationFillMode,
                PropertyId::AnimationPlayState,
                PropertyId::AnimationName,
            ],
            "cubic-bezier(0, -2, 1, 3)"
        ),
        Some(vec![(
            0,
            PropertyId::AnimationTimingFunction,
            "cubic-bezier(0, -2, 1, 3)".to_string()
        )])
    );
    assert_eq!(
        parse_coordinating_shorthand(
            &[
                PropertyId::TransitionProperty,
                PropertyId::TransitionDuration,
                PropertyId::TransitionTimingFunction,
                PropertyId::TransitionDelay,
                PropertyId::TransitionBehavior,
            ],
            "opacity,"
        ),
        None
    );
    assert_eq!(
        parse_coordinating_shorthand(
            &[
                PropertyId::TransitionProperty,
                PropertyId::TransitionDuration,
                PropertyId::TransitionTimingFunction,
                PropertyId::TransitionDelay,
                PropertyId::TransitionBehavior,
            ],
            "none, opacity 1s"
        ),
        None
    );
}

#[test]
fn parses_layer_shorthands() {
    assert_eq!(
        parse_layer_shorthand_items(
            PropertyId::Background,
            "url(bg.png) left top / cover no-repeat fixed border-box content-box red"
        ),
        Some(vec![
            (0, PropertyId::BackgroundImage, "url(bg.png)".to_string()),
            (0, PropertyId::BackgroundPosition, "left top".to_string()),
            (0, PropertyId::BackgroundSize, "cover".to_string()),
            (0, PropertyId::BackgroundRepeat, "no-repeat".to_string()),
            (0, PropertyId::BackgroundAttachment, "fixed".to_string()),
            (0, PropertyId::BackgroundClip, "border-box".to_string()),
            (0, PropertyId::BackgroundOrigin, "content-box".to_string()),
            (0, PropertyId::BackgroundColor, "red".to_string()),
        ])
    );
    assert_eq!(
        parse_layer_shorthand_items(PropertyId::Background, "url(a.png) no-repeat, blue"),
        Some(vec![
            (0, PropertyId::BackgroundImage, "url(a.png)".to_string()),
            (0, PropertyId::BackgroundRepeat, "no-repeat".to_string()),
            (1, PropertyId::BackgroundColor, "blue".to_string()),
        ])
    );
    assert_eq!(
        parse_layer_shorthand_items(
            PropertyId::Mask,
            "url(#mask) left 10px top 20px / 50% 25% no-repeat border-box no-clip add luminance"
        ),
        Some(vec![
            (0, PropertyId::MaskImage, "url(#mask)".to_string()),
            (0, PropertyId::MaskPosition, "left 10px top 20px".to_string()),
            (0, PropertyId::MaskSize, "50% 25%".to_string()),
            (0, PropertyId::MaskRepeat, "no-repeat".to_string()),
            (0, PropertyId::MaskOrigin, "border-box".to_string()),
            (0, PropertyId::MaskClip, "no-clip".to_string()),
            (0, PropertyId::MaskComposite, "add".to_string()),
            (0, PropertyId::MaskMode, "luminance".to_string()),
        ])
    );

    assert_eq!(
        parse_layer_shorthand_items(PropertyId::Background, "red, url(bg.png)"),
        None
    );
    assert_eq!(
        parse_layer_shorthand_items(PropertyId::Mask, "url(mask.png) / cover"),
        None
    );
}

#[test]
fn parses_font_shorthands() {
    assert_eq!(
        parse_font_shorthand_items("italic small-caps bold condensed 16px / 1.5 Arial, serif"),
        Some(vec![
            (PropertyId::FontStyle, "italic".to_string()),
            (PropertyId::FontVariant, "small-caps".to_string()),
            (PropertyId::FontWeight, "bold".to_string()),
            (PropertyId::FontWidth, "condensed".to_string()),
            (PropertyId::FontSize, "16px".to_string()),
            (PropertyId::LineHeight, "1.5".to_string()),
            (PropertyId::FontFamily, "Arial, serif".to_string()),
        ])
    );
    assert_eq!(
        parse_font_shorthand_items("normal normal 12px sans-serif"),
        Some(vec![
            (PropertyId::FontSize, "12px".to_string()),
            (PropertyId::FontFamily, "sans-serif".to_string()),
        ])
    );
    assert_eq!(
        parse_font_shorthand_items("oblique 10deg 700 14px/20px \"Serenity Sans\""),
        Some(vec![
            (PropertyId::FontStyle, "oblique 10deg".to_string()),
            (PropertyId::FontWeight, "700".to_string()),
            (PropertyId::FontSize, "14px".to_string()),
            (PropertyId::LineHeight, "20px".to_string()),
            (PropertyId::FontFamily, "\"Serenity Sans\"".to_string()),
        ])
    );
    assert_eq!(
        parse_font_shorthand_items("bold normal 20%/1.2 fantasy"),
        Some(vec![
            (PropertyId::FontWeight, "bold".to_string()),
            (PropertyId::FontSize, "20%".to_string()),
            (PropertyId::LineHeight, "1.2".to_string()),
            (PropertyId::FontFamily, "fantasy".to_string()),
        ])
    );
    assert_eq!(
        parse_font_shorthand_items("italic condensed normal calc(30% - 40px)/calc(120% + 1.2em) fantasy"),
        Some(vec![
            (PropertyId::FontStyle, "italic".to_string()),
            (PropertyId::FontWidth, "condensed".to_string()),
            (PropertyId::FontSize, "calc(30% - 40px)".to_string()),
            (PropertyId::LineHeight, "calc(120% + 1.2em)".to_string()),
            (PropertyId::FontFamily, "fantasy".to_string()),
        ])
    );
    assert_eq!(
        parse_font_shorthand_items("oblique calc(200 + 300) 24px Arial"),
        Some(vec![
            (PropertyId::FontStyle, "oblique".to_string()),
            (PropertyId::FontWeight, "calc(200 + 300)".to_string()),
            (PropertyId::FontSize, "24px".to_string()),
            (PropertyId::FontFamily, "Arial".to_string()),
        ])
    );
    assert_eq!(
        parse_font_shorthand_items("oblique calc(30deg + 5deg) 24px Arial"),
        Some(vec![
            (PropertyId::FontStyle, "oblique calc(30deg + 5deg)".to_string()),
            (PropertyId::FontSize, "24px".to_string()),
            (PropertyId::FontFamily, "Arial".to_string()),
        ])
    );

    assert_eq!(
        parse_font_shorthand_items("normal normal normal normal normal 12px serif"),
        None
    );
    assert_eq!(parse_font_shorthand_items("italic bold serif"), None);
    assert_eq!(parse_font_shorthand_items("16px"), None);
}

#[test]
fn parses_positional_value_list_shorthands() {
    assert_eq!(
        parse_positional_shorthand(PropertyId::Margin, "1px 2% auto calc(3px + 4%)"),
        Some(vec![
            (0, "1px".to_string()),
            (1, "2%".to_string()),
            (2, "auto".to_string()),
            (3, "calc(3px + 4%)".to_string()),
        ])
    );
    let rust_items = parse_rust_owned_positional_shorthand(PropertyId::Margin, "1px 2% auto calc(3px + 4%)").unwrap();
    assert_eq!(
        rust_items
            .iter()
            .map(|item| (item.index, item.style_value.property_id, item.source.clone()))
            .collect::<Vec<_>>(),
        vec![
            (0, PropertyId::MarginTop, "1px".to_string()),
            (1, PropertyId::MarginRight, "2%".to_string()),
            (2, PropertyId::MarginBottom, "auto".to_string()),
            (3, PropertyId::MarginLeft, "calc(3px + 4%)".to_string()),
        ]
    );
    assert_eq!(
        rust_items[0].style_value,
        RustOwnedStyleValue {
            property_id: PropertyId::MarginTop,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Length {
                    value: 1.0,
                    unit: "px".to_string(),
                },
                value_type: PropertyValueType::Length,
            }),
        }
    );
    assert_eq!(
        parse_rust_owned_positional_shorthand(PropertyId::Margin, "var(--a) 10px").map(|items| {
            items
                .iter()
                .map(|item| (item.index, item.style_value.property_id, item.source.clone()))
                .collect::<Vec<_>>()
        }),
        Some(vec![
            (0, PropertyId::MarginTop, "var(--a)".to_string()),
            (1, PropertyId::MarginRight, "10px".to_string()),
        ])
    );
    assert_eq!(
        parse_positional_shorthand(PropertyId::BorderBlockWidth, "thin 2px"),
        Some(vec![(0, "thin".to_string()), (1, "2px".to_string())])
    );
    assert_eq!(
        parse_rust_owned_positional_shorthand(PropertyId::BorderBlockColor, "#234 transparent").map(|items| {
            items
                .iter()
                .map(|item| (item.index, item.style_value.property_id, item.source.clone()))
                .collect::<Vec<_>>()
        }),
        Some(vec![
            (0, PropertyId::BorderBlockStartColor, "#234".to_string()),
            (1, PropertyId::BorderBlockEndColor, "transparent".to_string()),
        ])
    );
    assert_eq!(
        parse_rust_owned_positional_shorthand(PropertyId::CornerShape, "round superellipse(2)").map(|items| {
            items
                .iter()
                .map(|item| (item.index, item.style_value.property_id, item.source.clone()))
                .collect::<Vec<_>>()
        }),
        Some(vec![
            (0, PropertyId::CornerTopLeftShape, "round".to_string()),
            (1, PropertyId::CornerTopRightShape, "superellipse(2)".to_string()),
        ])
    );
    let rust_items =
        parse_rust_owned_positional_shorthand(PropertyId::Inset, "anchor(--target bottom, calc(1px + 2%)) 2% auto 4px")
            .unwrap();
    assert_eq!(
        rust_items
            .iter()
            .map(|item| (item.index, item.style_value.property_id, item.source.clone()))
            .collect::<Vec<_>>(),
        vec![
            (
                0,
                PropertyId::Top,
                "anchor(--target bottom, calc(1px + 2%))".to_string()
            ),
            (1, PropertyId::Right, "2%".to_string()),
            (2, PropertyId::Bottom, "auto".to_string()),
            (3, PropertyId::Left, "4px".to_string()),
        ]
    );
    assert_eq!(parse_positional_shorthand(PropertyId::Margin, ""), None);
    assert_eq!(
        parse_positional_shorthand(PropertyId::Margin, "1px 2px 3px 4px 5px"),
        None
    );
    assert_eq!(parse_positional_shorthand(PropertyId::Margin, "1px red"), None);
    assert_eq!(parse_positional_shorthand(PropertyId::Color, "red"), None);
    assert_eq!(
        parse_rust_owned_positional_shorthand(PropertyId::BorderColor, "123"),
        None
    );
    assert_eq!(
        parse_rust_owned_positional_shorthand_with_options(
            PropertyId::BorderColor,
            "123",
            CssPrimitiveValueOptions {
                allow_quirky_color: true,
                ..Default::default()
            }
        )
        .map(|items| {
            items
                .into_iter()
                .map(|item| (item.index, item.style_value.property_id, item.source))
                .collect::<Vec<_>>()
        }),
        Some(vec![(0, PropertyId::BorderTopColor, "123".to_string())])
    );
}

#[test]
fn selects_property_numeric_metadata_with_generated_metadata() {
    assert_eq!(
        property_numeric_metadata(&[PropertyId::Color, PropertyId::AnimationDuration], "Time"),
        Some(PropertyNumericMetadata {
            property_id: PropertyId::AnimationDuration,
            minimum: 0.0,
            maximum: f32::MAX as f64,
            percentage_range: None,
            percentages_resolve_to_value_type: false,
        })
    );
    assert_eq!(
        property_numeric_metadata(&[PropertyId::Color, PropertyId::BackgroundPositionX], "Length"),
        Some(PropertyNumericMetadata {
            property_id: PropertyId::BackgroundPositionX,
            minimum: f32::MIN as f64,
            maximum: f32::MAX as f64,
            percentage_range: Some((f32::MIN as f64, f32::MAX as f64)),
            percentages_resolve_to_value_type: true,
        })
    );
    assert_eq!(
        property_numeric_metadata(&[PropertyId::Color, PropertyId::OffsetDistance], "LengthPercentage"),
        Some(PropertyNumericMetadata {
            property_id: PropertyId::OffsetDistance,
            minimum: f32::MIN as f64,
            maximum: f32::MAX as f64,
            percentage_range: Some((f32::MIN as f64, f32::MAX as f64)),
            percentages_resolve_to_value_type: true,
        })
    );
    assert_eq!(property_numeric_metadata(&[PropertyId::Color], "Length"), None);
}

fn parse_syntax(input: &str) -> Option<SyntaxNode> {
    component_values_parse_as_syntax(&parse(input), false)
}

fn parse_syntax_with_source(input: &str) -> Option<SyntaxNode> {
    let (mut parser, filtered_input) = super::parser_from_filtered_input(input.as_bytes());
    let component_values = parser.parse_a_list_of_component_values();
    component_values_parse_as_syntax_with_source(&component_values, false, Some(filtered_input))
}

fn parse_limited_syntax(input: &str) -> Option<SyntaxNode> {
    component_values_parse_as_syntax(&parse(input), true)
}

fn matches_syntax(input: &str, syntax: &str) -> bool {
    super::component_values_match_syntax(input.as_bytes(), syntax, false)
}

#[test]
fn universal_syntax_matches_optional_declaration_value() {
    assert!(matches_syntax("", "*"));
    assert!(matches_syntax("  ", "*"));
    assert!(matches_syntax("red", "*"));
    assert!(matches_syntax("calc(1px + 2px)", "*"));
    assert!(matches_syntax("(;)", "*"));
    assert!(matches_syntax("foo(!)", "*"));

    assert!(!matches_syntax(";", "*"));
    assert!(!matches_syntax("red;", "*"));
    assert!(!matches_syntax("!important", "*"));
    assert!(!matches_syntax("]", "*"));
    assert!(!matches_syntax("var(, 1px)", "*"));
}

#[test]
fn parses_rust_owned_calculation_operator_tree() {
    assert_eq!(
        parse_math_ast("calc(1px + 2px * 3)"),
        Some(RustOwnedCalculationNode::Sum(vec![
            RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Dimension {
                value: 1.0,
                unit: "px".to_string(),
            }),
            RustOwnedCalculationNode::Product(vec![
                RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Dimension {
                    value: 2.0,
                    unit: "px".to_string(),
                }),
                RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Number(3.0)),
            ]),
        ]))
    );
}

#[test]
fn parses_rust_owned_calculation_nested_blocks() {
    assert_eq!(
        parse_math_ast("calc((1px + 2px) / 2)"),
        Some(RustOwnedCalculationNode::Product(vec![
            RustOwnedCalculationNode::Sum(vec![
                RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Dimension {
                    value: 1.0,
                    unit: "px".to_string(),
                }),
                RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Dimension {
                    value: 2.0,
                    unit: "px".to_string(),
                }),
            ]),
            RustOwnedCalculationNode::Invert(Box::new(RustOwnedCalculationNode::Numeric(
                RustOwnedCalculationNumericValue::Number(2.0)
            ))),
        ]))
    );
}

#[test]
fn parses_rust_owned_calculation_math_functions() {
    assert_eq!(
        parse_math_ast("min(10px, calc(1px + 2px))"),
        Some(RustOwnedCalculationNode::Function {
            name: "min".to_string(),
            arguments: vec![
                RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Dimension {
                    value: 10.0,
                    unit: "px".to_string(),
                }),
                RustOwnedCalculationNode::Sum(vec![
                    RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Dimension {
                        value: 1.0,
                        unit: "px".to_string(),
                    }),
                    RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Dimension {
                        value: 2.0,
                        unit: "px".to_string(),
                    }),
                ]),
            ],
        })
    );
}

#[test]
fn parses_rust_owned_calculation_tree_counting_leaves() {
    assert!(matches!(
        parse_math_ast("calc(sibling-count() + 1)"),
        Some(RustOwnedCalculationNode::Sum(values))
            if matches!(
                values.as_slice(),
                [
                    RustOwnedCalculationNode::Numeric(
                        RustOwnedCalculationNumericValue::TreeCountingFunction(_)
                    ),
                    RustOwnedCalculationNode::Numeric(RustOwnedCalculationNumericValue::Number(1.0)),
                ]
            )
    ));
}

#[test]
fn rejects_invalid_rust_owned_calculations() {
    assert_eq!(parse_math_ast("calc(1px +)"), None);
    assert_eq!(parse_math_ast("calc(1px ** 2)"), None);
    assert_eq!(parse_math_ast("calc(foo(1px))"), None);
    assert_eq!(parse_math_ast("round(1px, 2px, 3px)"), None);
}

#[test]
fn rust_owned_math_functions_carry_calculation_trees() {
    let Some(RustOwnedStyleValue {
        value: RustOwnedStyleValueKind::MathFunction(RustOwnedMathFunction { calculation, .. }),
        ..
    }) = parse_rust_owned_style_value(&[PropertyId::MarginLeft], "calc(1px + 2px)")
    else {
        panic!("expected a math function");
    };

    assert_eq!(*calculation, parse_math_ast("calc(1px + 2px)").unwrap());
}

#[test]
fn emits_rust_owned_calculation_trees_in_postorder() {
    let calculation = parse_math_ast("calc(1px + 2px * 3)").unwrap();
    let mut events = Vec::new();

    emit_rust_owned_calculation_tree(
        &calculation,
        &mut |kind, primitive_kind, has_numeric_value, numeric_value, child_count, metadata| {
            events.push((
                kind,
                primitive_kind,
                has_numeric_value,
                numeric_value,
                child_count,
                String::from_utf8_lossy(metadata).to_string(),
            ));
        },
    );

    assert_eq!(
        events,
        vec![
            (
                CssCalculationNodeKind::Numeric,
                CssPrimitiveValueKind::Length,
                true,
                1.0,
                0,
                "px".to_string(),
            ),
            (
                CssCalculationNodeKind::Numeric,
                CssPrimitiveValueKind::Length,
                true,
                2.0,
                0,
                "px".to_string(),
            ),
            (
                CssCalculationNodeKind::Numeric,
                CssPrimitiveValueKind::Number,
                true,
                3.0,
                0,
                String::new(),
            ),
            (
                CssCalculationNodeKind::Product,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                2,
                String::new(),
            ),
            (
                CssCalculationNodeKind::Sum,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                2,
                String::new(),
            ),
        ]
    );
}

#[test]
fn emits_rust_owned_round_calculations_with_strategy() {
    let calculation = parse_math_ast("round(up, 10px, 3px)").unwrap();
    let mut events = Vec::new();

    emit_rust_owned_calculation_tree(
        &calculation,
        &mut |kind, primitive_kind, has_numeric_value, numeric_value, child_count, metadata| {
            events.push((
                kind,
                primitive_kind,
                has_numeric_value,
                numeric_value,
                child_count,
                String::from_utf8_lossy(metadata).to_string(),
            ));
        },
    );

    assert_eq!(
        events,
        vec![
            (
                CssCalculationNodeKind::Numeric,
                CssPrimitiveValueKind::Length,
                true,
                10.0,
                0,
                "px".to_string(),
            ),
            (
                CssCalculationNodeKind::Numeric,
                CssPrimitiveValueKind::Length,
                true,
                3.0,
                0,
                "px".to_string(),
            ),
            (
                CssCalculationNodeKind::Function,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                2,
                "round up".to_string(),
            ),
        ]
    );
}

#[test]
fn emits_rust_owned_random_calculations_with_value_sharing() {
    let calculation = parse_math_ast("random(--foo element-shared, 10px, 30px, 5px)").unwrap();
    let mut events = Vec::new();

    emit_rust_owned_calculation_tree(
        &calculation,
        &mut |kind, primitive_kind, has_numeric_value, numeric_value, child_count, metadata| {
            events.push((
                kind,
                primitive_kind,
                has_numeric_value,
                numeric_value,
                child_count,
                metadata.to_vec(),
            ));
        },
    );

    assert_eq!(
        events,
        vec![
            (
                CssCalculationNodeKind::Numeric,
                CssPrimitiveValueKind::Length,
                true,
                10.0,
                0,
                b"px".to_vec(),
            ),
            (
                CssCalculationNodeKind::Numeric,
                CssPrimitiveValueKind::Length,
                true,
                30.0,
                0,
                b"px".to_vec(),
            ),
            (
                CssCalculationNodeKind::Numeric,
                CssPrimitiveValueKind::Length,
                true,
                5.0,
                0,
                b"px".to_vec(),
            ),
            (
                CssCalculationNodeKind::Function,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                3,
                b"random\0dashed-ident\01\0--foo".to_vec(),
            ),
        ]
    );
}

#[test]
fn parses_preserved_tokens() {
    let values = parse("a, b");

    assert_eq!(values.len(), 4);
    assert!(matches!(values[0], ComponentValue::PreservedToken(_)));
    assert!(matches!(values[1], ComponentValue::PreservedToken(_)));
    assert!(matches!(values[2], ComponentValue::PreservedToken(_)));
    assert!(matches!(values[3], ComponentValue::PreservedToken(_)));
}

#[test]
fn parses_simple_blocks() {
    let values = parse("{ color: rgb(1 2 3); }");

    let ComponentValue::SimpleBlock(block) = &values[0] else {
        panic!("expected a simple block");
    };
    assert!(matches!(block.token.token_type, TokenType::OpenCurly));
    assert!(matches!(block.end_token.token_type, TokenType::CloseCurly));
    assert!(
        block
            .value
            .iter()
            .any(|value| matches!(value, ComponentValue::Function(function) if function.name == "rgb"))
    );
}

#[test]
fn parses_functions() {
    let values = parse("calc(1px + var(--gap))");

    let ComponentValue::Function(function) = &values[0] else {
        panic!("expected a function");
    };
    assert_eq!(function.name, "calc");
    assert!(matches!(function.end_token.token_type, TokenType::CloseParen));
    assert!(
        function
            .value
            .iter()
            .any(|value| matches!(value, ComponentValue::Function(function) if function.name == "var"))
    );
}

#[test]
fn parses_a_component_value() {
    let value =
        parse_with(" calc(1px + var(--gap)) ", Parser::parse_a_component_value).expect("expected a component value");

    let ComponentValue::Function(function) = value else {
        panic!("expected a function");
    };
    assert_eq!(function.name, "calc");

    assert!(parse_with("", Parser::parse_a_component_value).is_none());
    assert!(parse_with("a b", Parser::parse_a_component_value).is_none());
}

#[test]
fn parses_comma_separated_component_values() {
    let groups = parse_with(
        "calc(1px, 2px),, rgb(1, 2, 3),",
        Parser::parse_a_comma_separated_list_of_component_values,
    );

    assert_eq!(groups.len(), 4);
    assert_eq!(groups[0].len(), 1);
    assert!(groups[1].is_empty());
    assert_eq!(groups[2].len(), 2);
    assert!(groups[3].is_empty());

    let ComponentValue::Function(function) = &groups[0][0] else {
        panic!("expected a function");
    };
    assert_eq!(function.name, "calc");
}

#[test]
fn parses_syntax() {
    assert_eq!(parse_syntax("*"), Some(SyntaxNode::Universal));
    assert_eq!(parse_syntax("thing"), Some(SyntaxNode::Ident("thing".to_string())));
    assert_eq!(parse_syntax("<number>"), Some(SyntaxNode::Type("number".to_string())));
    assert_eq!(
        parse_syntax("<number>+"),
        Some(SyntaxNode::Multiplier(Box::new(SyntaxNode::Type("number".to_string()))))
    );
    assert_eq!(
        parse_syntax("<string>#"),
        Some(SyntaxNode::CommaSeparatedMultiplier(Box::new(SyntaxNode::Type(
            "string".to_string()
        ))))
    );
    assert_eq!(
        parse_syntax("well | <number>+ | <string>#"),
        Some(SyntaxNode::Alternatives(vec![
            SyntaxNode::Ident("well".to_string()),
            SyntaxNode::Multiplier(Box::new(SyntaxNode::Type("number".to_string()))),
            SyntaxNode::CommaSeparatedMultiplier(Box::new(SyntaxNode::Type("string".to_string()))),
        ]))
    );
    assert_eq!(
        parse_syntax(r#""<number>""#),
        Some(SyntaxNode::Type("number".to_string()))
    );
    assert_eq!(
        parse_syntax("<transform-list>"),
        Some(SyntaxNode::Type("transform-list".to_string()))
    );
}

#[test]
fn rejects_invalid_syntax() {
    assert!(parse_syntax("").is_none());
    assert!(parse_syntax(" ").is_none());
    assert!(parse_syntax("<number").is_none());
    assert!(parse_syntax("thing |").is_none());
    assert!(parse_syntax("* | *").is_none());
    assert!(parse_syntax("<transform-list>+").is_none());
    assert!(parse_syntax("<transform-list>#").is_none());
    assert!(parse_syntax("<woozle>").is_none());
    assert!(parse_syntax("<Number>").is_none());
    assert!(parse_syntax("<LENGTH>").is_none());
    assert!(parse_syntax_with_source(r"<\6c ength>").is_none());
    assert!(parse_syntax("<number> <integer>").is_none());
    assert!(parse_syntax("thingy whatsit").is_none());
    assert!(parse_syntax("<number> +").is_none());
    assert!(parse_syntax("<number> #").is_none());
}

#[test]
fn limits_single_component_ident_to_custom_ident_for_syntax() {
    assert_eq!(
        parse_limited_syntax("thing"),
        Some(SyntaxNode::Ident("thing".to_string()))
    );
    assert!(parse_limited_syntax("inherit").is_none());
    assert!(parse_limited_syntax("initial").is_none());
    assert!(parse_limited_syntax("unset").is_none());
    assert!(parse_limited_syntax("revert").is_none());
    assert!(parse_limited_syntax("revert-layer").is_none());
    assert!(parse_limited_syntax("default").is_none());
}

#[test]
fn matches_syntax_against_component_values() {
    assert!(matches_syntax("thing", "thing"));
    assert!(matches_syntax("10px", "<length>"));
    assert!(matches_syntax("red", "<color>"));
    assert!(matches_syntax("green", "<color> | none"));
    assert!(matches_syntax("foo(){}", "*"));
    assert!(matches_syntax("foo, bar", "<custom-ident>#"));
    assert!(matches_syntax("foo", "foo"));
    assert!(!matches_syntax("inherit", "<custom-ident>"));
    assert!(!matches_syntax("auto", "<length>"));
}

#[test]
fn parses_supports_boolean_expression() {
    let component_values = parse("(color: green) and (width: 50px)");
    let mut parser = ComponentValueParser::new(component_values);
    parser.parse_a_boolean_expression(BooleanExpressionTestKind::SupportsFeature);

    let Some(BooleanExpression::And(children)) = parser.boolean_expression else {
        panic!("expected an and expression");
    };
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|child| matches!(child, BooleanExpression::Test(_))));
}

#[test]
fn parses_supports_features() {
    assert_eq!(
        parse_supports_feature("(color: green)"),
        Some((CssSupportsFeatureKind::Declaration, None))
    );
    assert_eq!(
        parse_supports_feature("selector(:has(.foo))"),
        Some((CssSupportsFeatureKind::Selector, None))
    );
    assert_eq!(
        parse_supports_feature("font-tech(color-COLRv1)"),
        Some((CssSupportsFeatureKind::FontTech, Some("color-COLRv1".to_string())))
    );
    assert_eq!(
        parse_supports_feature("font-format(opentype)"),
        Some((CssSupportsFeatureKind::FontFormat, Some("opentype".to_string())))
    );
    assert_eq!(
        parse_supports_feature("env(safe-area-inset-top)"),
        Some((CssSupportsFeatureKind::Env, Some("safe-area-inset-top".to_string())))
    );
}

#[test]
fn rejects_invalid_supports_features() {
    assert_eq!(parse_supports_feature(""), None);
    assert_eq!(parse_supports_feature("width: 1px"), None);
    assert_eq!(parse_supports_feature("font-tech(color-COLRv1 extra)"), None);
    assert_eq!(parse_supports_feature("font-format(\"opentype\")"), None);
    assert_eq!(parse_supports_feature("env()"), None);
    assert_eq!(parse_supports_feature("selector(.foo) extra"), None);
}

#[test]
fn parses_supports_general_enclosed() {
    let component_values = parse("florb(123)");
    let mut parser = ComponentValueParser::new(component_values);
    parser.parse_a_boolean_expression(BooleanExpressionTestKind::SupportsFeature);

    assert!(matches!(
        parser.boolean_expression,
        Some(BooleanExpression::GeneralEnclosed(_))
    ));

    let component_values = parse("(unknown-feature)");
    let mut parser = ComponentValueParser::new(component_values);
    parser.parse_a_boolean_expression(BooleanExpressionTestKind::MediaFeature);

    assert!(matches!(
        parser.boolean_expression,
        Some(BooleanExpression::GeneralEnclosed(_))
    ));
}

#[test]
fn parses_media_boolean_expression() {
    let component_values = parse("(width <= 600px) or (hover)");
    let mut parser = ComponentValueParser::new(component_values);
    parser.parse_a_boolean_expression(BooleanExpressionTestKind::MediaFeature);

    let Some(BooleanExpression::Or(children)) = parser.boolean_expression else {
        panic!("expected an or expression");
    };
    assert_eq!(children.len(), 2);
    assert!(children.iter().all(|child| matches!(child, BooleanExpression::Test(_))));
}

#[test]
fn parses_media_general_enclosed() {
    let component_values = parse("(foo bar)");
    let mut parser = ComponentValueParser::new(component_values);
    parser.parse_a_boolean_expression(BooleanExpressionTestKind::MediaFeature);

    assert!(matches!(
        parser.boolean_expression,
        Some(BooleanExpression::GeneralEnclosed(_))
    ));
}

#[test]
fn parses_media_feature_syntax_nodes() {
    let Some(MediaFeatureSyntax::Boolean(name)) = parse_media_feature_syntax("color") else {
        panic!("expected a boolean media feature");
    };
    assert_eq!(name.kind, MediaFeatureNameKind::Normal);
    assert_eq!(name.id, MediaFeatureId::Color);

    let Some(MediaFeatureSyntax::Plain { name, value }) = parse_media_feature_syntax("width: 100px") else {
        panic!("expected a plain media feature");
    };
    assert_eq!(name.kind, MediaFeatureNameKind::Normal);
    assert_eq!(name.id, MediaFeatureId::Width);
    assert_eq!(strip_whitespace(&value).len(), 1);

    let Some(MediaFeatureSyntax::Plain { name, value }) = parse_media_feature_syntax("min-width: 100px") else {
        panic!("expected a min-prefixed media feature");
    };
    assert_eq!(name.kind, MediaFeatureNameKind::Min);
    assert_eq!(name.id, MediaFeatureId::Width);
    assert_eq!(strip_whitespace(&value).len(), 1);

    let Some(MediaFeatureSyntax::Plain { name, value }) = parse_media_feature_syntax("max-width: 100px") else {
        panic!("expected a max-prefixed media feature");
    };
    assert_eq!(name.kind, MediaFeatureNameKind::Max);
    assert_eq!(name.id, MediaFeatureId::Width);
    assert_eq!(strip_whitespace(&value).len(), 1);
}

#[test]
fn parses_media_feature_range_syntax_nodes() {
    let Some(MediaFeatureSyntax::HalfRangeNameFirst {
        name,
        comparison,
        value,
    }) = parse_media_feature_syntax("width >= 100px")
    else {
        panic!("expected a name-first half-range media feature");
    };
    assert_eq!(name.kind, MediaFeatureNameKind::Normal);
    assert_eq!(name.id, MediaFeatureId::Width);
    assert_eq!(comparison, MfComparison::GreaterThanOrEqual);
    assert_eq!(strip_whitespace(&value).len(), 1);

    let Some(MediaFeatureSyntax::HalfRangeValueFirst {
        value,
        comparison,
        name,
    }) = parse_media_feature_syntax("100px <= width")
    else {
        panic!("expected a value-first half-range media feature");
    };
    assert_eq!(strip_whitespace(&value).len(), 1);
    assert_eq!(comparison, MfComparison::LessThanOrEqual);
    assert_eq!(name.kind, MediaFeatureNameKind::Normal);
    assert_eq!(name.id, MediaFeatureId::Width);

    let Some(MediaFeatureSyntax::Range {
        left_value,
        left_comparison,
        name,
        right_comparison,
        right_value,
    }) = parse_media_feature_syntax("100px <= width <= 200px")
    else {
        panic!("expected a full-range media feature");
    };
    assert_eq!(strip_whitespace(&left_value).len(), 1);
    assert_eq!(left_comparison, MfComparison::LessThanOrEqual);
    assert_eq!(name.kind, MediaFeatureNameKind::Normal);
    assert_eq!(name.id, MediaFeatureId::Width);
    assert_eq!(right_comparison, MfComparison::LessThanOrEqual);
    assert_eq!(strip_whitespace(&right_value).len(), 1);
}

#[test]
fn rejects_invalid_media_feature_syntax_nodes() {
    assert!(parse_media_feature_syntax("min-hover: hover").is_none());
    assert!(parse_media_feature_syntax("hover > 1").is_none());
    assert!(parse_media_feature_syntax("hover = none").is_none());
    assert!(parse_media_feature_syntax("1 < hover").is_none());
    assert!(parse_media_feature_syntax("1 < hover < 2").is_none());
    assert!(parse_media_feature_syntax("100px <= width >= 200px").is_none());
    assert!(parse_media_feature_syntax("100px = width = 200px").is_none());
    assert!(parse_media_feature_syntax("width <> 100px").is_none());
    assert!(parse_media_feature_syntax("resolution > infinite").is_none());
    assert!(parse_media_feature_syntax("infinite < resolution").is_none());
    assert!(parse_media_feature_syntax("infinite < resolution < 200dpi").is_none());
}

#[test]
fn knows_generated_media_feature_value_metadata() {
    assert!(media_feature_accepts_type(
        MediaFeatureId::Width,
        MediaFeatureValueType::Length
    ));
    assert!(!media_feature_accepts_type(
        MediaFeatureId::Width,
        MediaFeatureValueType::Integer
    ));
    assert!(media_feature_accepts_identifier(MediaFeatureId::Hover, "none"));
    assert!(media_feature_accepts_identifier(MediaFeatureId::Hover, "HOVER"));
    assert!(!media_feature_accepts_identifier(MediaFeatureId::Hover, "fine"));
    assert!(media_feature_identifier_is_falsey(MediaFeatureId::Hover, "none"));
    assert!(!media_feature_identifier_is_falsey(MediaFeatureId::Hover, "hover"));
}

#[test]
fn knows_generated_css_unit_metadata() {
    assert_eq!(dimension_for_unit("px"), Some(DimensionType::Length));
    assert_eq!(dimension_for_unit("PX"), Some(DimensionType::Length));
    assert_eq!(dimension_for_unit("dpi"), Some(DimensionType::Resolution));
    assert_eq!(dimension_for_unit("unknown"), None);
}

#[test]
fn parses_media_feature_value_syntax_nodes() {
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Hover, &parse("hover")),
        MediaFeatureValueSyntaxKind::Ident
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Grid, &parse("1")),
        MediaFeatureValueSyntaxKind::Boolean
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Grid, &parse("calc(1)")),
        MediaFeatureValueSyntaxKind::Boolean
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Color, &parse("8")),
        MediaFeatureValueSyntaxKind::Integer
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Color, &parse("calc(8)")),
        MediaFeatureValueSyntaxKind::Integer
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Width, &parse("100px")),
        MediaFeatureValueSyntaxKind::Length
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Width, &parse("calc(100px)")),
        MediaFeatureValueSyntaxKind::Length
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Width, &parse("0")),
        MediaFeatureValueSyntaxKind::Length
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::AspectRatio, &parse("16 / 9")),
        MediaFeatureValueSyntaxKind::Ratio
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::AspectRatio, &parse("calc(16 / 9)")),
        MediaFeatureValueSyntaxKind::Ratio
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::AspectRatio, &parse("calc(16) / calc(9)")),
        MediaFeatureValueSyntaxKind::Ratio
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Resolution, &parse("96dpi")),
        MediaFeatureValueSyntaxKind::Resolution
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Resolution, &parse("calc(96dpi)")),
        MediaFeatureValueSyntaxKind::Resolution
    );
}

#[test]
fn emits_typed_media_feature_value_payloads() {
    assert_eq!(
        parse_media_test_values("(hover: hover)"),
        vec![(
            CssMediaFeatureValueKind::Value,
            CssMediaFeatureValueSyntaxKind::Ident,
            CssMediaFeatureValuePayloadKind::Ident,
            0.0,
            0.0,
            "hover".to_string(),
        )]
    );
    assert_eq!(
        parse_media_test_values("(color: 8)"),
        vec![(
            CssMediaFeatureValueKind::Value,
            CssMediaFeatureValueSyntaxKind::Integer,
            CssMediaFeatureValuePayloadKind::Integer,
            8.0,
            0.0,
            String::new(),
        )]
    );
    assert_eq!(
        parse_media_test_values("(width: 0)"),
        vec![(
            CssMediaFeatureValueKind::Value,
            CssMediaFeatureValueSyntaxKind::Length,
            CssMediaFeatureValuePayloadKind::Length,
            0.0,
            0.0,
            "px".to_string(),
        )]
    );
    assert_eq!(
        parse_media_test_values("(16/9 < aspect-ratio < 2)"),
        vec![
            (
                CssMediaFeatureValueKind::LeftValue,
                CssMediaFeatureValueSyntaxKind::Ratio,
                CssMediaFeatureValuePayloadKind::Ratio,
                16.0,
                9.0,
                String::new(),
            ),
            (
                CssMediaFeatureValueKind::LeftValue,
                CssMediaFeatureValueSyntaxKind::Ratio,
                CssMediaFeatureValuePayloadKind::Ratio,
                16.0,
                9.0,
                String::new(),
            ),
            (
                CssMediaFeatureValueKind::LeftValue,
                CssMediaFeatureValueSyntaxKind::Ratio,
                CssMediaFeatureValuePayloadKind::Ratio,
                16.0,
                9.0,
                String::new(),
            ),
            (
                CssMediaFeatureValueKind::RightValue,
                CssMediaFeatureValueSyntaxKind::Ratio,
                CssMediaFeatureValuePayloadKind::Ratio,
                2.0,
                1.0,
                String::new(),
            ),
        ]
    );
    assert_eq!(
        parse_media_test_values("(resolution: 96dpi)"),
        vec![(
            CssMediaFeatureValueKind::Value,
            CssMediaFeatureValueSyntaxKind::Resolution,
            CssMediaFeatureValuePayloadKind::Resolution,
            96.0,
            0.0,
            "dpi".to_string(),
        )]
    );
    assert_eq!(
        parse_media_test_values("(width: calc(100px))")[0].2,
        CssMediaFeatureValuePayloadKind::None
    );
}

#[test]
fn classifies_unknown_and_invalid_media_feature_value_syntax_nodes() {
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Hover, &parse("fine")),
        MediaFeatureValueSyntaxKind::Unknown
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Grid, &parse("2")),
        MediaFeatureValueSyntaxKind::Unknown
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Width, &parse("1quux")),
        MediaFeatureValueSyntaxKind::Unknown
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Resolution, &parse("-1dpi")),
        MediaFeatureValueSyntaxKind::Unknown
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::AspectRatio, &parse("16 / -9")),
        MediaFeatureValueSyntaxKind::Unknown
    );
    assert_eq!(
        component_values_parse_as_mf_value_syntax(MediaFeatureId::Width, &parse("1 < 2")),
        MediaFeatureValueSyntaxKind::Invalid
    );
}

#[test]
fn parses_font_variant_css2_syntax_nodes() {
    assert_eq!(
        parse_value_type("normal", ValueTypeId::FontVariantCss2),
        CssValueTypeSyntaxKind::FontVariantCss2Normal
    );
    assert_eq!(
        parse_value_type("small-caps", ValueTypeId::FontVariantCss2),
        CssValueTypeSyntaxKind::FontVariantCss2SmallCaps
    );
}

#[test]
fn rejects_invalid_font_variant_css2_syntax_nodes() {
    assert_eq!(
        parse_value_type("all-small-caps", ValueTypeId::FontVariantCss2),
        CssValueTypeSyntaxKind::Invalid
    );
    assert_eq!(
        parse_value_type("small-caps normal", ValueTypeId::FontVariantCss2),
        CssValueTypeSyntaxKind::Invalid
    );
}

#[test]
fn parses_font_weight_absolute_syntax_nodes() {
    assert_eq!(
        parse_value_type("normal", ValueTypeId::FontWeightAbsolute),
        CssValueTypeSyntaxKind::FontWeightAbsoluteNormal
    );
    assert_eq!(
        parse_value_type("bold", ValueTypeId::FontWeightAbsolute),
        CssValueTypeSyntaxKind::FontWeightAbsoluteBold
    );
    assert_eq!(
        parse_value_type("700", ValueTypeId::FontWeightAbsolute),
        CssValueTypeSyntaxKind::FontWeightAbsoluteNumber
    );
    assert_eq!(
        parse_value_type("calc(600 + 100)", ValueTypeId::FontWeightAbsolute),
        CssValueTypeSyntaxKind::FontWeightAbsoluteNumber
    );
}

#[test]
fn rejects_invalid_font_weight_absolute_syntax_nodes() {
    assert_eq!(
        parse_value_type("lighter", ValueTypeId::FontWeightAbsolute),
        CssValueTypeSyntaxKind::Invalid
    );
    assert_eq!(
        parse_value_type("0", ValueTypeId::FontWeightAbsolute),
        CssValueTypeSyntaxKind::Invalid
    );
    assert_eq!(
        parse_value_type("1001", ValueTypeId::FontWeightAbsolute),
        CssValueTypeSyntaxKind::Invalid
    );
    assert_eq!(
        parse_value_type("700 800", ValueTypeId::FontWeightAbsolute),
        CssValueTypeSyntaxKind::Invalid
    );
}

#[test]
fn parses_font_width_css3_syntax_nodes() {
    assert_eq!(
        parse_value_type("normal", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3Normal
    );
    assert_eq!(
        parse_value_type("ultra-condensed", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3UltraCondensed
    );
    assert_eq!(
        parse_value_type("extra-condensed", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3ExtraCondensed
    );
    assert_eq!(
        parse_value_type("condensed", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3Condensed
    );
    assert_eq!(
        parse_value_type("semi-condensed", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3SemiCondensed
    );
    assert_eq!(
        parse_value_type("semi-expanded", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3SemiExpanded
    );
    assert_eq!(
        parse_value_type("expanded", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3Expanded
    );
    assert_eq!(
        parse_value_type("extra-expanded", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3ExtraExpanded
    );
    assert_eq!(
        parse_value_type("ultra-expanded", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::FontWidthCss3UltraExpanded
    );
}

#[test]
fn rejects_invalid_font_width_css3_syntax_nodes() {
    assert_eq!(
        parse_value_type("100%", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::Invalid
    );
    assert_eq!(
        parse_value_type("condensed expanded", ValueTypeId::FontWidthCss3),
        CssValueTypeSyntaxKind::Invalid
    );
}

#[test]
fn parses_font_variant_keyword_syntax_nodes() {
    assert_eq!(
        parse_value_type("all-small-caps", ValueTypeId::FontVariantCapsValue),
        CssValueTypeSyntaxKind::FontVariantCapsValueAllSmallCaps
    );
    assert_eq!(
        parse_value_type("unicode", ValueTypeId::FontVariantEmojiValue),
        CssValueTypeSyntaxKind::FontVariantEmojiValueUnicode
    );
    assert_eq!(
        parse_value_type("super", ValueTypeId::FontVariantPositionValue),
        CssValueTypeSyntaxKind::FontVariantPositionValueSuper
    );
}

#[test]
fn parses_font_keyword_syntax_nodes() {
    assert_eq!(
        parse_value_type("normal", ValueTypeId::FontKerningValue),
        CssValueTypeSyntaxKind::FontKerningValueNormal
    );
    assert_eq!(
        parse_value_type("none", ValueTypeId::FontOpticalSizingValue),
        CssValueTypeSyntaxKind::FontOpticalSizingValueNone
    );
}

#[test]
fn parses_symbol_syntax_nodes() {
    assert_eq!(
        parse_value_type("\"*\"", ValueTypeId::Symbol),
        CssValueTypeSyntaxKind::SymbolString
    );
    assert_eq!(
        parse_value_type("triangle", ValueTypeId::Symbol),
        CssValueTypeSyntaxKind::SymbolCustomIdent
    );
}

#[test]
fn rejects_invalid_symbol_syntax_nodes() {
    assert_eq!(
        parse_value_type("inherit", ValueTypeId::Symbol),
        CssValueTypeSyntaxKind::Invalid
    );
    assert_eq!(
        parse_value_type("default", ValueTypeId::Symbol),
        CssValueTypeSyntaxKind::Invalid
    );
    assert_eq!(
        parse_value_type("triangle square", ValueTypeId::Symbol),
        CssValueTypeSyntaxKind::Invalid
    );
}

#[test]
fn parses_value_type_syntax_for_ffi() {
    assert_eq!(
        parse_a_value_type(b"normal", ValueTypeId::FontWeightAbsolute as u8),
        super::CssValueTypeSyntaxKind::FontWeightAbsoluteNormal
    );
    assert_eq!(
        parse_a_value_type(b"700", ValueTypeId::FontWeightAbsolute as u8),
        super::CssValueTypeSyntaxKind::FontWeightAbsoluteNumber
    );
    assert_eq!(
        parse_a_value_type(b"triangle", ValueTypeId::Symbol as u8),
        super::CssValueTypeSyntaxKind::SymbolCustomIdent
    );
    assert_eq!(
        parse_a_value_type(b"triangle square", ValueTypeId::Symbol as u8),
        super::CssValueTypeSyntaxKind::Invalid
    );
}

#[test]
fn parses_media_query_syntax_nodes() {
    let queries = parse_media_query_list("screen, not print and (width >= 100px), (hover)");
    assert_eq!(queries.len(), 3);

    let MediaQuerySyntax::Valid {
        modifier,
        media_type,
        condition,
    } = &queries[0]
    else {
        panic!("expected a valid media query");
    };
    assert_eq!(*modifier, MediaQueryModifier::None);
    assert_eq!(media_type.as_deref(), Some("screen"));
    assert!(condition.is_none());

    let MediaQuerySyntax::Valid {
        modifier,
        media_type,
        condition,
    } = &queries[1]
    else {
        panic!("expected a valid media query");
    };
    assert_eq!(*modifier, MediaQueryModifier::Not);
    assert_eq!(media_type.as_deref(), Some("print"));
    assert!(condition.is_some());

    let MediaQuerySyntax::Valid {
        modifier,
        media_type,
        condition,
    } = &queries[2]
    else {
        panic!("expected a valid media query");
    };
    assert_eq!(*modifier, MediaQueryModifier::None);
    assert!(media_type.is_none());
    assert!(condition.is_some());
}

#[test]
fn ignores_whitespace_only_media_query_lists() {
    assert!(parse_media_query_list("").is_empty());
    assert!(parse_media_query_list(" \t\n").is_empty());
}

#[test]
fn parses_single_media_queries() {
    let (did_parse, media_query) = parse_media_query("");
    assert!(did_parse);
    let media_query = media_query.expect("expected a media query");
    assert!(media_query.is_negated);
    assert!(!media_query.has_media_condition);
    assert_eq!(media_query.media_type_kind, CssMediaTypeKind::All);

    let (did_parse, media_query) = parse_media_query("screen and (hover)");
    assert!(did_parse);
    let media_query = media_query.expect("expected a media query");
    assert!(!media_query.is_negated);
    assert!(media_query.has_media_condition);
    assert_eq!(media_query.media_type_kind, CssMediaTypeKind::Screen);

    let (did_parse, media_query) = parse_media_query("screen, print");
    assert!(!did_parse);
    assert!(media_query.is_none());
}

#[test]
fn parses_media_tests() {
    let (events, media_feature_count) = parse_media_test("width >= 100px");
    assert_eq!(
        events,
        vec![
            CssBooleanExpressionEventKind::TestStart,
            CssBooleanExpressionEventKind::TestEnd
        ]
    );
    assert_eq!(media_feature_count, 1);

    let (events, media_feature_count) = parse_media_test("(width >= 100px) and (hover)");
    assert_eq!(
        events,
        vec![
            CssBooleanExpressionEventKind::AndStart,
            CssBooleanExpressionEventKind::TestStart,
            CssBooleanExpressionEventKind::TestEnd,
            CssBooleanExpressionEventKind::TestStart,
            CssBooleanExpressionEventKind::TestEnd,
            CssBooleanExpressionEventKind::AndEnd,
        ]
    );
    assert_eq!(media_feature_count, 2);
}

#[test]
fn parses_if_boolean_expression() {
    let events = parse_if_condition("supports(width: 1px) and media(width >= 100px)");
    assert_eq!(
        events,
        vec![
            CssBooleanExpressionEventKind::AndStart,
            CssBooleanExpressionEventKind::TestStart,
            CssBooleanExpressionEventKind::TestEnd,
            CssBooleanExpressionEventKind::TestStart,
            CssBooleanExpressionEventKind::TestEnd,
            CssBooleanExpressionEventKind::AndEnd,
        ]
    );

    let events = parse_if_condition("not style(--foo: bar)");
    assert_eq!(
        events,
        vec![
            CssBooleanExpressionEventKind::NotStart,
            CssBooleanExpressionEventKind::TestStart,
            CssBooleanExpressionEventKind::TestEnd,
            CssBooleanExpressionEventKind::NotEnd,
        ]
    );
}

#[test]
fn rejects_invalid_if_boolean_expression() {
    let events = parse_if_condition("else");
    assert_eq!(events, vec![CssBooleanExpressionEventKind::Invalid]);

    let events = parse_if_condition("supports(width: 1px) or media(width >= 100px) and style(--foo: bar)");
    assert_eq!(events, vec![CssBooleanExpressionEventKind::Invalid]);
}

#[test]
fn parses_page_selector_lists() {
    assert_eq!(
        parse_page_selector_list("invoice:left:first, :blank"),
        Some(vec![
            (
                Some("invoice".to_string()),
                vec![CssPagePseudoClassKind::Left, CssPagePseudoClassKind::First],
            ),
            (None, vec![CssPagePseudoClassKind::Blank]),
        ])
    );

    assert_eq!(parse_page_selector_list(""), Some(vec![]));
}

#[test]
fn rejects_invalid_page_selector_lists() {
    assert_eq!(parse_page_selector_list(","), None);
    assert_eq!(parse_page_selector_list(":unknown"), None);
    assert_eq!(parse_page_selector_list("invoice :left"), None);
}

#[test]
fn parses_keyframe_selector_lists() {
    assert_eq!(
        parse_keyframe_selector_list("from, 50%, to"),
        Some(vec![0.0, 50.0, 100.0])
    );
    assert_eq!(parse_keyframe_selector_list("0%, 100%"), Some(vec![0.0, 100.0]));
}

#[test]
fn rejects_invalid_keyframe_selector_lists() {
    assert_eq!(parse_keyframe_selector_list("0"), None);
    assert_eq!(parse_keyframe_selector_list("from,"), None);
    assert_eq!(parse_keyframe_selector_list("101%"), None);
    assert_eq!(parse_keyframe_selector_list("from, via"), None);
}

#[test]
fn parses_keyframes_names() {
    assert_eq!(parse_keyframes_name("slide"), Some("slide".to_string()));
    assert_eq!(parse_keyframes_name("\"slide\""), Some("slide".to_string()));
}

#[test]
fn rejects_invalid_keyframes_names() {
    assert_eq!(parse_keyframes_name("none"), None);
    assert_eq!(parse_keyframes_name("default"), None);
    assert_eq!(parse_keyframes_name("inherit"), None);
    assert_eq!(parse_keyframes_name("slide extra"), None);
    assert_eq!(parse_keyframes_name("1"), None);
}

#[test]
fn parses_custom_property_names() {
    assert_eq!(parse_custom_property_name("--accent"), Some("--accent".to_string()));
}

#[test]
fn rejects_invalid_custom_property_names() {
    assert_eq!(parse_custom_property_name("--"), None);
    assert_eq!(parse_custom_property_name("color"), None);
    assert_eq!(parse_custom_property_name("--accent extra"), None);
    assert_eq!(parse_custom_property_name("\"--accent\""), None);
}

#[test]
fn parses_custom_idents() {
    assert_eq!(parse_custom_ident("accent"), Some("accent".to_string()));
    assert_eq!(parse_custom_ident("--accent"), Some("--accent".to_string()));
}

#[test]
fn rejects_invalid_custom_idents() {
    assert_eq!(parse_custom_ident("default"), None);
    assert_eq!(parse_custom_ident("inherit"), None);
    assert_eq!(parse_custom_ident("accent extra"), None);
    assert_eq!(parse_custom_ident("\"accent\""), None);
}

#[test]
fn parses_dashed_idents() {
    assert_eq!(parse_dashed_ident("--accent"), Some("--accent".to_string()));
    assert_eq!(parse_dashed_ident("--"), Some("--".to_string()));
    assert_eq!(parse_dashed_ident("--Accent"), Some("--Accent".to_string()));
}

#[test]
fn rejects_invalid_dashed_idents() {
    assert_eq!(parse_dashed_ident("-accent"), None);
    assert_eq!(parse_dashed_ident("accent"), None);
    assert_eq!(parse_dashed_ident("--accent extra"), None);
    assert_eq!(parse_dashed_ident("\"--accent\""), None);
}

#[test]
fn parses_unicode_ranges() {
    assert_eq!(parse_unicode_range("u+a"), Some((0xA, 0xA)));
    assert_eq!(parse_unicode_range("U+abc"), Some((0xABC, 0xABC)));
    assert_eq!(parse_unicode_range("u+a?"), Some((0xA0, 0xAF)));
    assert_eq!(parse_unicode_range("u+?????"), Some((0x0, 0xFFFFF)));
    assert_eq!(parse_unicode_range("u+1e-20"), Some((0x1E, 0x20)));
    assert_eq!(parse_unicode_range("u+0-10ffff"), Some((0x0, 0x10FFFF)));
}

#[test]
fn parses_unicode_range_lists() {
    assert_eq!(
        parse_unicode_range_list("u+0, u+20-7e, u+a?"),
        Some(vec![(0x0, 0x0), (0x20, 0x7E), (0xA0, 0xAF)])
    );
}

#[test]
fn rejects_invalid_unicode_ranges() {
    assert_eq!(parse_unicode_range("u+efg"), None);
    assert_eq!(parse_unicode_range("u+ abc"), None);
    assert_eq!(parse_unicode_range("u+aaaaaaa"), None);
    assert_eq!(parse_unicode_range("u+a?a"), None);
    assert_eq!(parse_unicode_range("u+222222"), None);
    assert_eq!(parse_unicode_range("u+0-110000"), None);
    assert_eq!(parse_unicode_range("u+1-0"), None);
    assert_eq!(parse_unicode_range("u+0 foo"), None);
    assert_eq!(parse_unicode_range_list("u+0, nope"), None);
}

#[test]
fn parses_url_functions() {
    assert_eq!(
        parse_url_function("url(image.png)"),
        Some((CssUrlFunctionType::Url, "image.png".to_string(), vec![]))
    );
    assert_eq!(
        parse_url_function("url(\"image.png\")"),
        Some((CssUrlFunctionType::Url, "image.png".to_string(), vec![]))
    );
    assert_eq!(
        parse_url_function("src(\"image.png\")"),
        Some((CssUrlFunctionType::Src, "image.png".to_string(), vec![]))
    );
    assert_eq!(
        parse_url_function(
            "url(\"image.png\" referrer-policy(no-referrer) integrity(\"sha256-deadbeef\") cross-origin(anonymous))"
        ),
        Some((
            CssUrlFunctionType::Url,
            "image.png".to_string(),
            vec![
                CssUrlModifierKind::CrossOrigin,
                CssUrlModifierKind::Integrity,
                CssUrlModifierKind::ReferrerPolicy,
            ]
        ))
    );
}

#[test]
fn rejects_invalid_url_functions() {
    assert_eq!(parse_url_function("src(image.png)"), None);
    assert_eq!(parse_url_function("url(\"image.png\" unknown())"), None);
    assert_eq!(
        parse_url_function("url(\"image.png\" cross-origin(anonymous) cross-origin(use-credentials))"),
        None
    );
    assert_eq!(parse_url_function("url(\"image.png\" integrity(not-a-string))"), None);
}

#[test]
fn parses_import_urls() {
    assert_eq!(
        parse_import_url("\"sheet.css\""),
        Some((CssUrlFunctionType::Url, "sheet.css".to_string(), vec![]))
    );
    assert_eq!(
        parse_import_url("url(\"sheet.css\")"),
        Some((CssUrlFunctionType::Url, "sheet.css".to_string(), vec![]))
    );
    assert_eq!(
        parse_import_url("src(\"sheet.css\")"),
        Some((CssUrlFunctionType::Src, "sheet.css".to_string(), vec![]))
    );
}

#[test]
fn rejects_invalid_import_urls() {
    assert_eq!(parse_import_url("sheet.css"), None);
    assert_eq!(parse_import_url("\"sheet.css\" extra"), None);
    assert_eq!(parse_import_url("url(\"sheet.css\") extra"), None);
}

#[test]
fn parses_font_sources() {
    assert_eq!(
        parse_font_source("local(\"Ahem\")"),
        Some((CssFontSourceKind::Local, Some("Ahem".to_string()), None, vec![]))
    );
    assert_eq!(
        parse_font_source("url(\"ahem.woff2\") format(woff2) tech(variations, color-COLRv1)"),
        Some((
            CssFontSourceKind::Url,
            Some("ahem.woff2".to_string()),
            Some("woff2".to_string()),
            vec![CssFontTech::Variations, CssFontTech::ColorColrv1],
        ))
    );
    assert_eq!(
        parse_font_source("url(\"ahem.woff2\") format(\"woff2-variations\")"),
        Some((
            CssFontSourceKind::Url,
            Some("ahem.woff2".to_string()),
            Some("woff2".to_string()),
            vec![CssFontTech::Variations],
        ))
    );
}

#[test]
fn rejects_invalid_font_sources() {
    assert_eq!(parse_font_source("local(serif)"), None);
    assert_eq!(
        parse_font_source("url(\"ahem.woff2\") tech(variations) format(woff2)"),
        None
    );
    assert_eq!(parse_font_source("url(\"ahem.woff2\") format(\"unknown\")"), None);
    assert_eq!(parse_font_source("url(\"ahem.woff2\") tech()"), None);
    assert_eq!(parse_font_source("url(\"ahem.woff2\") tech(variations,)"), None);
    assert_eq!(parse_font_source("url(\"ahem.woff2\") tech(unknown)"), None);
}

#[test]
fn parses_font_language_overrides() {
    assert_eq!(
        parse_font_language_override("normal"),
        Some((CssFontLanguageOverrideKind::Normal, None))
    );
    assert_eq!(
        parse_font_language_override("\"KSW\""),
        Some((CssFontLanguageOverrideKind::String, Some("KSW".to_string())))
    );
    assert_eq!(
        parse_font_language_override("\"en  \""),
        Some((CssFontLanguageOverrideKind::String, Some("en".to_string())))
    );
    assert_eq!(
        parse_font_language_override("\" en \""),
        Some((CssFontLanguageOverrideKind::String, Some(" en".to_string())))
    );
}

#[test]
fn rejects_invalid_font_language_overrides() {
    assert_eq!(parse_font_language_override("auto"), None);
    assert_eq!(parse_font_language_override("normal \"ksw\""), None);
    assert_eq!(parse_font_language_override("\"turkish\""), None);
    assert_eq!(parse_font_language_override("\"xøx\""), None);
    assert_eq!(parse_font_language_override("\"\""), None);
    assert_eq!(parse_font_language_override("\"ENG  \""), None);
    assert_eq!(parse_font_language_override("\"    \""), None);
}

#[test]
fn parses_opentype_tags() {
    assert_eq!(parse_opentype_tag("\"dlig\""), Some("dlig".to_string()));
    assert_eq!(parse_opentype_tag("\"AB@D\""), Some("AB@D".to_string()));
    assert_eq!(parse_opentype_tag("\"a cd\""), Some("a cd".to_string()));
}

#[test]
fn rejects_invalid_opentype_tags() {
    assert_eq!(parse_opentype_tag("dlig"), None);
    assert_eq!(parse_opentype_tag("\"dli\""), None);
    assert_eq!(parse_opentype_tag("\"dligx\""), None);
    assert_eq!(parse_opentype_tag("\"abc\u{1f}\""), None);
    assert_eq!(parse_opentype_tag("\"abc\u{7f}\""), None);
    assert_eq!(parse_opentype_tag("\"dlig\" 1"), None);
}

#[test]
fn parses_font_feature_settings() {
    assert_eq!(
        parse_font_feature_settings("normal"),
        Some((CssOpenTypeSettingsKind::Normal, vec![]))
    );
    assert_eq!(
        parse_font_feature_settings("\"dlig\" 1, \"smcp\" on, \"liga\" off, \"c2sc\""),
        Some((
            CssOpenTypeSettingsKind::TagValues,
            vec![
                OpenTypeTaggedValue {
                    tag: "dlig".to_string(),
                    value_kind: CssOpenTypeTaggedValueKind::Value,
                    value: Some("1".to_string()),
                    value_component_values: open_type_value_component_values("\"dlig\" 1", "dlig"),
                },
                OpenTypeTaggedValue {
                    tag: "smcp".to_string(),
                    value_kind: CssOpenTypeTaggedValueKind::On,
                    value: None,
                    value_component_values: vec![],
                },
                OpenTypeTaggedValue {
                    tag: "liga".to_string(),
                    value_kind: CssOpenTypeTaggedValueKind::Off,
                    value: None,
                    value_component_values: vec![],
                },
                OpenTypeTaggedValue {
                    tag: "c2sc".to_string(),
                    value_kind: CssOpenTypeTaggedValueKind::Implicit,
                    value: None,
                    value_component_values: vec![],
                },
            ],
        ))
    );
}

#[test]
fn rejects_invalid_font_feature_settings() {
    assert_eq!(parse_font_feature_settings("normal, \"dlig\""), None);
    assert_eq!(parse_font_feature_settings("\"dli\" 1"), None);
    assert_eq!(parse_font_feature_settings("\"dlig\" on off"), None);
    assert_eq!(parse_font_feature_settings("\"dlig\","), None);
}

#[test]
fn parses_font_variation_settings() {
    assert_eq!(
        parse_font_variation_settings("normal"),
        Some((CssOpenTypeSettingsKind::Normal, vec![]))
    );
    assert_eq!(
        parse_font_variation_settings("\"wght\" 700, \"XHGT\" calc(0.4 + 0.3)"),
        Some((
            CssOpenTypeSettingsKind::TagValues,
            vec![
                OpenTypeTaggedValue {
                    tag: "wght".to_string(),
                    value_kind: CssOpenTypeTaggedValueKind::Value,
                    value: Some("700".to_string()),
                    value_component_values: open_type_value_component_values(
                        "\"wght\" 700, \"XHGT\" calc(0.4 + 0.3)",
                        "wght",
                    ),
                },
                OpenTypeTaggedValue {
                    tag: "XHGT".to_string(),
                    value_kind: CssOpenTypeTaggedValueKind::Value,
                    value: Some("calc(0.4 + 0.3)".to_string()),
                    value_component_values: open_type_value_component_values(
                        "\"wght\" 700, \"XHGT\" calc(0.4 + 0.3)",
                        "XHGT",
                    ),
                },
            ],
        ))
    );
}

#[test]
fn rejects_invalid_font_variation_settings() {
    assert_eq!(parse_font_variation_settings("normal, \"wght\" 700"), None);
    assert_eq!(parse_font_variation_settings("\"wgt\" 700"), None);
    assert_eq!(parse_font_variation_settings("\"wght\""), None);
    assert_eq!(parse_font_variation_settings("\"wght\" 700,"), None);
}

#[test]
fn parses_font_styles() {
    assert_eq!(parse_font_style("normal"), Some(FontStyle::Normal));
    assert_eq!(parse_font_style("italic"), Some(FontStyle::Italic));
    assert_eq!(parse_font_style("left"), Some(FontStyle::Left));
    assert_eq!(parse_font_style("right"), Some(FontStyle::Right));
    assert_eq!(
        parse_font_style("oblique"),
        Some(FontStyle::Oblique { has_angle: false })
    );
    assert_eq!(
        parse_font_style("oblique 10deg"),
        Some(FontStyle::Oblique { has_angle: true })
    );
    assert_eq!(
        parse_font_style("oblique 100grad"),
        Some(FontStyle::Oblique { has_angle: true })
    );
    assert_eq!(
        parse_font_style("oblique -0.25turn"),
        Some(FontStyle::Oblique { has_angle: true })
    );
    assert_eq!(
        parse_font_style("oblique calc(10deg + 1deg)"),
        Some(FontStyle::Oblique { has_angle: true })
    );
}

#[test]
fn rejects_invalid_font_styles() {
    assert_eq!(parse_font_style("normal italic"), None);
    assert_eq!(parse_font_style("italic 10deg"), None);
    assert_eq!(parse_font_style("oblique 10px"), None);
    assert_eq!(parse_font_style("oblique 91deg"), None);
    assert_eq!(parse_font_style("oblique -101grad"), None);
    assert_eq!(parse_font_style("oblique 1turn"), None);
}

#[test]
fn parses_font_variant_alternates_values() {
    assert_eq!(
        parse_font_variant_alternates("stylistic(foo) historical-forms styleset(bar, baz) character-variant(qux)"),
        Some(vec![
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Stylistic,
                feature_value_names: vec!["foo".to_string()]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::HistoricalForms,
                feature_value_names: vec![]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Styleset,
                feature_value_names: vec!["bar".to_string(), "baz".to_string()]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::CharacterVariant,
                feature_value_names: vec!["qux".to_string()]
            },
        ])
    );
    assert_eq!(
        parse_font_variant_alternates("swash(foo) ornaments(bar) annotation(baz)"),
        Some(vec![
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Swash,
                feature_value_names: vec!["foo".to_string()]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Ornaments,
                feature_value_names: vec!["bar".to_string()]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Annotation,
                feature_value_names: vec!["baz".to_string()]
            },
        ])
    );
    assert_eq!(
        parse_font_variant_alternates("annotation(foo) ornaments(bar) swash(baz) historical-forms stylistic(qux)"),
        Some(vec![
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Stylistic,
                feature_value_names: vec!["qux".to_string()]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::HistoricalForms,
                feature_value_names: vec![]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Swash,
                feature_value_names: vec!["baz".to_string()]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Ornaments,
                feature_value_names: vec!["bar".to_string()]
            },
            FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::Annotation,
                feature_value_names: vec!["foo".to_string()]
            },
        ])
    );
}

#[test]
fn rejects_invalid_font_variant_alternates_values() {
    assert_eq!(parse_font_variant_alternates("stylistic(foo) stylistic(bar)"), None);
    assert_eq!(parse_font_variant_alternates("historical-forms historical-forms"), None);
    assert_eq!(parse_font_variant_alternates("stylistic(foo, bar)"), None);
    assert_eq!(parse_font_variant_alternates("swash()"), None);
    assert_eq!(parse_font_variant_alternates("styleset(foo,)"), None);
    assert_eq!(parse_font_variant_alternates("normal stylistic(foo)"), None);
    assert_eq!(parse_font_variant_alternates(""), None);
}

#[test]
fn parses_font_variant_values() {
    assert_eq!(parse_font_variant("normal"), Some(FontVariant::default()));
    assert_eq!(
        parse_font_variant("none"),
        Some(FontVariant {
            ligatures_none: true,
            ..FontVariant::default()
        })
    );
    assert_eq!(
        parse_font_variant(
            "super proportional-width jis83 stacked-fractions tabular-nums oldstyle-nums historical-forms all-small-caps no-contextual no-historical-ligatures no-discretionary-ligatures no-common-ligatures"
        ),
        Some(FontVariant {
            alternates: Some(vec![FontVariantAlternatesValue {
                kind: CssFontVariantAlternatesValueKind::HistoricalForms,
                feature_value_names: vec![],
            }]),
            caps: Some("all-small-caps".to_string()),
            east_asian: Some(vec![
                FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Width,
                    value: "proportional-width".to_string(),
                },
                FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Variant,
                    value: "jis83".to_string(),
                },
            ]),
            ligatures: Some(vec![
                FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Contextual,
                    value: "no-contextual".to_string(),
                },
                FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Historical,
                    value: "no-historical-ligatures".to_string(),
                },
                FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Discretionary,
                    value: "no-discretionary-ligatures".to_string(),
                },
                FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Common,
                    value: "no-common-ligatures".to_string(),
                },
            ]),
            numeric: Some(vec![
                FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Fraction,
                    value: "stacked-fractions".to_string(),
                },
                FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Spacing,
                    value: "tabular-nums".to_string(),
                },
                FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Figure,
                    value: "oldstyle-nums".to_string(),
                },
            ]),
            position: Some("super".to_string()),
            ..FontVariant::default()
        })
    );
}

#[test]
fn rejects_invalid_font_variant_values() {
    assert_eq!(parse_font_variant(""), None);
    assert_eq!(parse_font_variant("normal none"), None);
    assert_eq!(parse_font_variant("none normal"), None);
    assert_eq!(parse_font_variant("small-caps normal"), None);
    assert_eq!(parse_font_variant("normal small-caps"), None);
    assert_eq!(parse_font_variant("none small-caps"), None);
    assert_eq!(parse_font_variant("small-caps all-small-caps"), None);
    assert_eq!(parse_font_variant("sub super"), None);
    assert_eq!(parse_font_variant("text emoji"), None);
    assert_eq!(parse_font_variant("lining-nums oldstyle-nums"), None);
}

#[test]
fn parses_font_variant_east_asian_values() {
    assert_eq!(
        parse_font_variant_east_asian("jis78 proportional-width ruby"),
        Some(vec![
            FontVariantEastAsianValue {
                kind: CssFontVariantEastAsianValueKind::Variant,
                value: "jis78".to_string()
            },
            FontVariantEastAsianValue {
                kind: CssFontVariantEastAsianValueKind::Width,
                value: "proportional-width".to_string()
            },
            FontVariantEastAsianValue {
                kind: CssFontVariantEastAsianValueKind::Ruby,
                value: "ruby".to_string()
            },
        ])
    );
}

#[test]
fn rejects_invalid_font_variant_east_asian_values() {
    assert_eq!(parse_font_variant_east_asian("jis78 jis83"), None);
    assert_eq!(parse_font_variant_east_asian("full-width proportional-width"), None);
    assert_eq!(parse_font_variant_east_asian("normal ruby"), None);
    assert_eq!(parse_font_variant_east_asian(""), None);
}

#[test]
fn parses_font_variant_numeric_values() {
    assert_eq!(
        parse_font_variant_numeric("oldstyle-nums tabular-nums diagonal-fractions ordinal slashed-zero"),
        Some(vec![
            FontVariantNumericValue {
                kind: CssFontVariantNumericValueKind::Figure,
                value: "oldstyle-nums".to_string()
            },
            FontVariantNumericValue {
                kind: CssFontVariantNumericValueKind::Spacing,
                value: "tabular-nums".to_string()
            },
            FontVariantNumericValue {
                kind: CssFontVariantNumericValueKind::Fraction,
                value: "diagonal-fractions".to_string()
            },
            FontVariantNumericValue {
                kind: CssFontVariantNumericValueKind::Ordinal,
                value: "ordinal".to_string()
            },
            FontVariantNumericValue {
                kind: CssFontVariantNumericValueKind::SlashedZero,
                value: "slashed-zero".to_string()
            },
        ])
    );
}

#[test]
fn rejects_invalid_font_variant_numeric_values() {
    assert_eq!(parse_font_variant_numeric("lining-nums oldstyle-nums"), None);
    assert_eq!(parse_font_variant_numeric("tabular-nums proportional-nums"), None);
    assert_eq!(parse_font_variant_numeric("normal lining-nums"), None);
    assert_eq!(parse_font_variant_numeric(""), None);
}

#[test]
fn parses_font_variant_ligatures_values() {
    assert_eq!(
        parse_font_variant_ligatures("common-ligatures discretionary-ligatures historical-ligatures contextual"),
        Some(vec![
            FontVariantLigaturesValue {
                kind: CssFontVariantLigaturesValueKind::Common,
                value: "common-ligatures".to_string()
            },
            FontVariantLigaturesValue {
                kind: CssFontVariantLigaturesValueKind::Discretionary,
                value: "discretionary-ligatures".to_string()
            },
            FontVariantLigaturesValue {
                kind: CssFontVariantLigaturesValueKind::Historical,
                value: "historical-ligatures".to_string()
            },
            FontVariantLigaturesValue {
                kind: CssFontVariantLigaturesValueKind::Contextual,
                value: "contextual".to_string()
            },
        ])
    );
}

#[test]
fn rejects_invalid_font_variant_ligatures_values() {
    assert_eq!(
        parse_font_variant_ligatures("common-ligatures no-common-ligatures"),
        None
    );
    assert_eq!(parse_font_variant_ligatures("contextual no-contextual"), None);
    assert_eq!(parse_font_variant_ligatures("normal common-ligatures"), None);
    assert_eq!(parse_font_variant_ligatures(""), None);
}

#[test]
fn parses_font_family_values() {
    assert_eq!(
        parse_font_family_value("serif, Helvetica, \"Bongo Sans\""),
        Some(vec![
            FontFamilyValue::Generic("serif".to_string()),
            FontFamilyValue::FamilyName(FamilyName {
                name: "Helvetica".to_string(),
                is_string: false,
            }),
            FontFamilyValue::FamilyName(FamilyName {
                name: "Bongo Sans".to_string(),
                is_string: true,
            }),
        ])
    );
    assert_eq!(
        parse_font_family_value("ui-sans-serif, Great Vibes"),
        Some(vec![
            FontFamilyValue::Generic("ui-sans-serif".to_string()),
            FontFamilyValue::FamilyName(FamilyName {
                name: "Great Vibes".to_string(),
                is_string: false,
            }),
        ])
    );
}

#[test]
fn rejects_invalid_font_family_values() {
    assert_eq!(parse_font_family_value(""), None);
    assert_eq!(parse_font_family_value("cursive serif"), None);
    assert_eq!(parse_font_family_value("Red/Black, sans-serif"), None);
    assert_eq!(parse_font_family_value("\"Lucida\" Grande, sans-serif"), None);
    assert_eq!(parse_font_family_value("Ahem!, sans-serif"), None);
    assert_eq!(parse_font_family_value("Ahem,"), None);
}

#[test]
fn parses_layer_names() {
    assert_eq!(parse_layer_name("components", false), Some("components".to_string()));
    assert_eq!(
        parse_layer_name("components.buttons", false),
        Some("components.buttons".to_string())
    );
    assert_eq!(parse_layer_name(" ", true), Some(String::new()));
}

#[test]
fn rejects_invalid_layer_names() {
    assert_eq!(parse_layer_name("", false), None);
    assert_eq!(parse_layer_name("inherit", false), None);
    assert_eq!(parse_layer_name("components . buttons", false), None);
    assert_eq!(parse_layer_name("components, buttons", false), None);
}

#[test]
fn parses_import_layers() {
    assert_eq!(parse_import_layer("layer"), Some(String::new()));
    assert_eq!(parse_import_layer("LaYeR"), Some(String::new()));
    assert_eq!(parse_import_layer("layer(components)"), Some("components".to_string()));
    assert_eq!(
        parse_import_layer("layer(components.buttons)"),
        Some("components.buttons".to_string())
    );
}

#[test]
fn rejects_invalid_import_layers() {
    assert_eq!(parse_import_layer("components"), None);
    assert_eq!(parse_import_layer("layer()"), None);
    assert_eq!(parse_import_layer("layer(components buttons)"), None);
    assert_eq!(parse_import_layer("layer(components) extra"), None);
}

#[test]
fn parses_layer_name_lists() {
    assert_eq!(
        parse_layer_name_list("base, components.buttons, theme"),
        Some(vec![
            "base".to_string(),
            "components.buttons".to_string(),
            "theme".to_string(),
        ])
    );
}

#[test]
fn rejects_invalid_layer_name_lists() {
    assert_eq!(parse_layer_name_list(""), None);
    assert_eq!(parse_layer_name_list("base,"), None);
    assert_eq!(parse_layer_name_list("base components"), None);
    assert_eq!(parse_layer_name_list("base, initial"), None);
}

#[test]
fn parses_counter_style_names() {
    assert_eq!(
        parse_counter_style_name("custom-counter"),
        Some("custom-counter".to_string())
    );
    assert_eq!(parse_counter_style_name("disc"), Some("disc".to_string()));
}

#[test]
fn parses_counter_styles() {
    assert_eq!(
        parse_counter_style("custom-counter"),
        Some((
            CssCounterStyleKind::Name,
            CssCounterStyleSymbolsType::Symbolic,
            Some("custom-counter".to_string()),
            vec![]
        ))
    );
    assert_eq!(
        parse_counter_style("symbols(\"*\" \"**\")"),
        Some((
            CssCounterStyleKind::SymbolsFunction,
            CssCounterStyleSymbolsType::Symbolic,
            None,
            vec!["*".to_string(), "**".to_string()]
        ))
    );
    assert_eq!(
        parse_counter_style("symbols(cyclic \"*\" \"**\")"),
        Some((
            CssCounterStyleKind::SymbolsFunction,
            CssCounterStyleSymbolsType::Cyclic,
            None,
            vec!["*".to_string(), "**".to_string()]
        ))
    );
}

#[test]
fn rejects_invalid_counter_style_names() {
    assert_eq!(parse_counter_style_name("none"), None);
    assert_eq!(parse_counter_style_name("default"), None);
    assert_eq!(parse_counter_style_name("inherit"), None);
    assert_eq!(parse_counter_style_name("custom-counter extra"), None);
    assert_eq!(parse_counter_style_name("\"custom-counter\""), None);
}

#[test]
fn rejects_invalid_counter_styles() {
    assert_eq!(parse_counter_style("none"), None);
    assert_eq!(parse_counter_style("symbols()"), None);
    assert_eq!(parse_counter_style("symbols(numeric \"1\")"), None);
    assert_eq!(parse_counter_style("symbols(\"1\" ident)"), None);
    assert_eq!(parse_counter_style("symbols(\"1\") extra"), None);
}

#[test]
fn parses_nonnegative_integer_symbol_pairs() {
    assert_eq!(
        parse_nonnegative_integer_symbol_pair("1 \"I\""),
        Some(CssNonnegativeIntegerSymbolPairOrder::IntegerFirst)
    );
    assert_eq!(
        parse_nonnegative_integer_symbol_pair("\"I\" 1"),
        Some(CssNonnegativeIntegerSymbolPairOrder::SymbolFirst)
    );
    assert_eq!(
        parse_nonnegative_integer_symbol_pair("calc(1) \"I\""),
        Some(CssNonnegativeIntegerSymbolPairOrder::IntegerFirst)
    );
    assert_eq!(
        parse_nonnegative_integer_symbol_pair("symbol 1"),
        Some(CssNonnegativeIntegerSymbolPairOrder::SymbolFirst)
    );
}

#[test]
fn rejects_invalid_nonnegative_integer_symbol_pairs() {
    assert_eq!(parse_nonnegative_integer_symbol_pair("1"), None);
    assert_eq!(parse_nonnegative_integer_symbol_pair("\"I\""), None);
    assert_eq!(parse_nonnegative_integer_symbol_pair("1 \"I\" extra"), None);
    assert_eq!(parse_nonnegative_integer_symbol_pair("-1 \"I\""), None);
    assert_eq!(parse_nonnegative_integer_symbol_pair("inherit 1"), None);
    assert_eq!(parse_nonnegative_integer_symbol_pair("1 default"), None);
    assert_eq!(parse_nonnegative_integer_symbol_pair("foo(1) \"I\""), None);
    assert_eq!(parse_nonnegative_integer_symbol_pair("sibling-count() \"I\""), None);
}

#[test]
fn parses_counter_style_negative_descriptors() {
    assert_eq!(
        parse_counter_style_negative_descriptor("\"-\""),
        Some(CssCounterStyleNegativeSymbolCount::One)
    );
    assert_eq!(
        parse_counter_style_negative_descriptor("\"-\" \"\""),
        Some(CssCounterStyleNegativeSymbolCount::Two)
    );
    assert_eq!(
        parse_counter_style_negative_descriptor("minus"),
        Some(CssCounterStyleNegativeSymbolCount::One)
    );
}

#[test]
fn rejects_invalid_counter_style_negative_descriptors() {
    assert_eq!(parse_counter_style_negative_descriptor(""), None);
    assert_eq!(parse_counter_style_negative_descriptor("\"-\" \"\" extra"), None);
    assert_eq!(parse_counter_style_negative_descriptor("inherit"), None);
    assert_eq!(parse_counter_style_negative_descriptor("default"), None);
}

#[test]
fn parses_rust_owned_counter_style_negative_descriptors() {
    assert_eq!(
        parse_rust_owned_counter_style_negative_descriptor("\"-\" \"\"".as_bytes()),
        Some(vec![descriptor_string_value("-"), descriptor_string_value("")])
    );
    assert_eq!(
        parse_rust_owned_counter_style_negative_descriptor("minus".as_bytes()),
        Some(vec![descriptor_custom_ident_value("minus")])
    );
}

#[test]
fn parses_counter_style_system_descriptors() {
    assert_eq!(
        parse_counter_style_system_descriptor("cyclic"),
        Some(CssCounterStyleSystemKind::Cyclic)
    );
    assert_eq!(
        parse_counter_style_system_descriptor("numeric"),
        Some(CssCounterStyleSystemKind::Numeric)
    );
    assert_eq!(
        parse_counter_style_system_descriptor("alphabetic"),
        Some(CssCounterStyleSystemKind::Alphabetic)
    );
    assert_eq!(
        parse_counter_style_system_descriptor("symbolic"),
        Some(CssCounterStyleSystemKind::Symbolic)
    );
    assert_eq!(
        parse_counter_style_system_descriptor("additive"),
        Some(CssCounterStyleSystemKind::Additive)
    );
    assert_eq!(
        parse_counter_style_system_descriptor("fixed"),
        Some(CssCounterStyleSystemKind::Fixed)
    );
    assert_eq!(
        parse_counter_style_system_descriptor("fixed 1"),
        Some(CssCounterStyleSystemKind::FixedWithInteger)
    );
    assert_eq!(
        parse_counter_style_system_descriptor("fixed calc(1)"),
        Some(CssCounterStyleSystemKind::FixedWithInteger)
    );
    assert_eq!(
        parse_counter_style_system_descriptor("extends custom-counter"),
        Some(CssCounterStyleSystemKind::Extends)
    );
}

#[test]
fn rejects_invalid_counter_style_system_descriptors() {
    assert_eq!(parse_counter_style_system_descriptor(""), None);
    assert_eq!(parse_counter_style_system_descriptor("unknown"), None);
    assert_eq!(parse_counter_style_system_descriptor("fixed \"1\""), None);
    assert_eq!(parse_counter_style_system_descriptor("fixed 1 extra"), None);
    assert_eq!(parse_counter_style_system_descriptor("extends"), None);
    assert_eq!(parse_counter_style_system_descriptor("extends none"), None);
    assert_eq!(parse_counter_style_system_descriptor("extends default"), None);
    assert_eq!(parse_counter_style_system_descriptor("extends custom extra"), None);
}

#[test]
fn parses_rust_owned_counter_style_system_descriptors() {
    assert_eq!(
        parse_rust_owned_counter_style_system_descriptor("cyclic".as_bytes()),
        Some(RustOwnedCounterStyleSystemDescriptor::Cyclic)
    );
    assert_eq!(
        parse_rust_owned_counter_style_system_descriptor("fixed".as_bytes()),
        Some(RustOwnedCounterStyleSystemDescriptor::Fixed { first_symbol: None })
    );
    assert_eq!(
        parse_rust_owned_counter_style_system_descriptor("fixed 1".as_bytes()),
        Some(RustOwnedCounterStyleSystemDescriptor::Fixed {
            first_symbol: Some(descriptor_integer_value(1, "1")),
        })
    );
    assert_eq!(
        parse_rust_owned_counter_style_system_descriptor("fixed calc(1)".as_bytes()),
        Some(RustOwnedCounterStyleSystemDescriptor::Fixed {
            first_symbol: Some(descriptor_math_integer_value("calc(1)")),
        })
    );
    assert_eq!(
        parse_rust_owned_counter_style_system_descriptor("extends custom".as_bytes()),
        Some(RustOwnedCounterStyleSystemDescriptor::Extends {
            name: "custom".to_string(),
        })
    );
}

#[test]
fn parses_counter_style_symbols_descriptors() {
    assert_eq!(parse_counter_style_symbols_descriptor("\"*\""), Some(1));
    assert_eq!(parse_counter_style_symbols_descriptor("\"*\" \"**\""), Some(2));
    assert_eq!(parse_counter_style_symbols_descriptor("symbol"), Some(1));
    assert_eq!(parse_counter_style_symbols_descriptor("\"*\" symbol"), Some(2));
}

#[test]
fn rejects_invalid_counter_style_symbols_descriptors() {
    assert_eq!(parse_counter_style_symbols_descriptor(""), None);
    assert_eq!(parse_counter_style_symbols_descriptor("1"), None);
    assert_eq!(parse_counter_style_symbols_descriptor("\"*\" 1"), None);
    assert_eq!(parse_counter_style_symbols_descriptor("inherit"), None);
    assert_eq!(parse_counter_style_symbols_descriptor("default"), None);
}

#[test]
fn parses_rust_owned_counter_style_symbols_descriptors() {
    assert_eq!(
        parse_rust_owned_counter_style_symbols_descriptor("\"*\" symbol".as_bytes()),
        Some(vec![
            descriptor_string_value("*"),
            descriptor_custom_ident_value("symbol")
        ])
    );
}

#[test]
fn parses_counter_style_symbol_descriptors() {
    assert!(parse_counter_style_symbol_descriptor("\"*\""));
    assert!(parse_counter_style_symbol_descriptor("\"\""));
    assert!(parse_counter_style_symbol_descriptor("symbol"));
}

#[test]
fn rejects_invalid_counter_style_symbol_descriptors() {
    assert!(!parse_counter_style_symbol_descriptor(""));
    assert!(!parse_counter_style_symbol_descriptor("1"));
    assert!(!parse_counter_style_symbol_descriptor("\"*\" \"**\""));
    assert!(!parse_counter_style_symbol_descriptor("inherit"));
    assert!(!parse_counter_style_symbol_descriptor("default"));
}

#[test]
fn parses_rust_owned_counter_style_symbol_descriptors() {
    assert_eq!(
        parse_rust_owned_counter_style_symbol_descriptor("\"*\"".as_bytes()),
        Some(descriptor_string_value("*"))
    );
    assert_eq!(
        parse_rust_owned_counter_style_symbol_descriptor("symbol".as_bytes()),
        Some(descriptor_custom_ident_value("symbol"))
    );
    assert_eq!(parse_rust_owned_counter_style_symbol_descriptor("1".as_bytes()), None);
}

#[test]
fn parses_string_descriptors() {
    assert!(parse_string_descriptor_value("\"hello\""));
    assert!(parse_string_descriptor_value("\"\""));
}

#[test]
fn rejects_invalid_string_descriptors() {
    assert!(!parse_string_descriptor_value(""));
    assert!(!parse_string_descriptor_value("ident"));
    assert!(!parse_string_descriptor_value("\"hello\" \"world\""));
}

#[test]
fn parses_rust_owned_string_descriptors() {
    assert_eq!(
        parse_rust_owned_string_descriptor("\"hello\"".as_bytes()),
        Some("hello".to_string())
    );
    assert_eq!(parse_rust_owned_string_descriptor("ident".as_bytes()), None);
}

#[test]
fn parses_length_descriptors() {
    assert!(parse_length_descriptor_value("1px"));
    assert!(parse_length_descriptor_value("0"));
    assert!(parse_length_descriptor_value("calc(1px + 2px)"));
}

#[test]
fn rejects_invalid_length_descriptors() {
    assert!(!parse_length_descriptor_value(""));
    assert!(!parse_length_descriptor_value("1%"));
    assert!(!parse_length_descriptor_value("1px 2px"));
    assert!(!parse_length_descriptor_value("auto"));
    assert!(!parse_length_descriptor_value("foo(1px)"));
}

#[test]
fn parses_rust_owned_length_descriptors() {
    assert_eq!(
        parse_rust_owned_length_descriptor("calc(1px + 2px)".as_bytes()),
        Some("calc(1px + 2px)".to_string())
    );
    assert_eq!(parse_rust_owned_length_descriptor("1px 2px".as_bytes()), None);
    assert_eq!(parse_rust_owned_length_descriptor("foo(1px)".as_bytes()), None);
    assert_eq!(parse_rust_owned_length_descriptor("sibling-count()".as_bytes()), None);
}

#[test]
fn parses_positive_percentage_descriptors() {
    assert!(parse_positive_percentage_descriptor_value("0%"));
    assert!(parse_positive_percentage_descriptor_value("100%"));
    assert!(parse_positive_percentage_descriptor_value("calc(50% + 25%)"));
}

#[test]
fn rejects_invalid_positive_percentage_descriptors() {
    assert!(!parse_positive_percentage_descriptor_value(""));
    assert!(!parse_positive_percentage_descriptor_value("-1%"));
    assert!(!parse_positive_percentage_descriptor_value("1px"));
    assert!(!parse_positive_percentage_descriptor_value("1% 2%"));
    assert!(!parse_positive_percentage_descriptor_value("foo(1%)"));
}

#[test]
fn parses_rust_owned_positive_percentage_descriptors() {
    assert_eq!(
        parse_rust_owned_positive_percentage_descriptor("calc(50% + 25%)".as_bytes()),
        Some("calc(50% + 25%)".to_string())
    );
    assert_eq!(parse_rust_owned_positive_percentage_descriptor("-1%".as_bytes()), None);
    assert_eq!(
        parse_rust_owned_positive_percentage_descriptor("foo(1%)".as_bytes()),
        None
    );
    assert_eq!(
        parse_rust_owned_positive_percentage_descriptor("sibling-count()".as_bytes()),
        None
    );
}

#[test]
fn parses_page_size_descriptors() {
    assert!(parse_page_size_descriptor_value("auto"));
    assert!(parse_page_size_descriptor_value("8.5in"));
    assert!(parse_page_size_descriptor_value("0"));
    assert!(parse_page_size_descriptor_value("8.5in 11in"));
    assert!(parse_page_size_descriptor_value("calc(8in + 0.5in) 11in"));
    assert!(parse_page_size_descriptor_value("a4"));
    assert!(parse_page_size_descriptor_value("letter landscape"));
    assert!(parse_page_size_descriptor_value("portrait jis-b5"));
    assert!(parse_page_size_descriptor_value("landscape"));
}

#[test]
fn rejects_invalid_page_size_descriptors() {
    assert!(!parse_page_size_descriptor_value(""));
    assert!(!parse_page_size_descriptor_value("auto landscape"));
    assert!(!parse_page_size_descriptor_value("-1px"));
    assert!(!parse_page_size_descriptor_value("1px 2px 3px"));
    assert!(!parse_page_size_descriptor_value("1%"));
    assert!(!parse_page_size_descriptor_value("foo(1px)"));
    assert!(!parse_page_size_descriptor_value("a4 letter"));
    assert!(!parse_page_size_descriptor_value("landscape portrait"));
    assert!(!parse_page_size_descriptor_value("orange"));
}

#[test]
fn parses_rust_owned_page_size_descriptors() {
    assert_eq!(
        parse_rust_owned_page_size_descriptor("auto".as_bytes()),
        Some(RustOwnedPageSizeDescriptor::Auto)
    );
    assert_eq!(
        parse_rust_owned_page_size_descriptor("8.5in 11in".as_bytes()),
        Some(RustOwnedPageSizeDescriptor::Lengths(vec![
            RustOwnedDescriptorPrimitiveValue {
                primitive_kind: CssPrimitiveValueKind::Length,
                numeric_value: Some(8.5),
                source_or_unit: "in".to_string(),
                calculation: None,
            },
            RustOwnedDescriptorPrimitiveValue {
                primitive_kind: CssPrimitiveValueKind::Length,
                numeric_value: Some(11.0),
                source_or_unit: "in".to_string(),
                calculation: None,
            },
        ]))
    );
    assert_eq!(
        parse_rust_owned_page_size_descriptor("letter landscape".as_bytes()),
        Some(RustOwnedPageSizeDescriptor::PageSizeAndOrientation {
            page_size: Some(CssPageSizeKeyword::Letter),
            orientation: Some(CssPageSizeOrientation::Landscape),
        })
    );
    assert_eq!(parse_rust_owned_page_size_descriptor("auto landscape".as_bytes()), None);
    assert_eq!(parse_rust_owned_page_size_descriptor("foo(1px)".as_bytes()), None);
    assert_eq!(
        parse_rust_owned_page_size_descriptor("sibling-count()".as_bytes()),
        None
    );
}

#[test]
fn parses_optional_declaration_value_descriptors() {
    assert!(parse_optional_declaration_value_descriptor_value(""));
    assert!(parse_optional_declaration_value_descriptor_value("  "));
    assert!(parse_optional_declaration_value_descriptor_value("red"));
    assert!(parse_optional_declaration_value_descriptor_value("calc(1px + 2px)"));
    assert!(parse_optional_declaration_value_descriptor_value("(;)"));
    assert!(parse_optional_declaration_value_descriptor_value("foo(!)"));
}

#[test]
fn rejects_invalid_optional_declaration_value_descriptors() {
    assert!(!parse_optional_declaration_value_descriptor_value(";"));
    assert!(!parse_optional_declaration_value_descriptor_value("red;"));
    assert!(!parse_optional_declaration_value_descriptor_value("!important"));
    assert!(!parse_optional_declaration_value_descriptor_value("]"));
}

#[test]
fn parses_counter_style_range_descriptors() {
    assert_eq!(
        parse_counter_style_range_descriptor("auto"),
        Some((CssCounterStyleRangeKind::Auto, 0))
    );
    assert_eq!(
        parse_counter_style_range_descriptor("infinite infinite"),
        Some((CssCounterStyleRangeKind::List, 1))
    );
    assert_eq!(
        parse_counter_style_range_descriptor("infinite 0"),
        Some((CssCounterStyleRangeKind::List, 1))
    );
    assert_eq!(
        parse_counter_style_range_descriptor("0 infinite"),
        Some((CssCounterStyleRangeKind::List, 1))
    );
    assert_eq!(
        parse_counter_style_range_descriptor("infinite 0, 5 10, 100 infinite"),
        Some((CssCounterStyleRangeKind::List, 3))
    );
    assert_eq!(
        parse_counter_style_range_descriptor("calc(1) calc(2)"),
        Some((CssCounterStyleRangeKind::List, 1))
    );
}

#[test]
fn rejects_invalid_counter_style_range_descriptors() {
    assert_eq!(parse_counter_style_range_descriptor(""), None);
    assert_eq!(parse_counter_style_range_descriptor("auto 1"), None);
    assert_eq!(parse_counter_style_range_descriptor("0"), None);
    assert_eq!(parse_counter_style_range_descriptor("0 1,"), None);
    assert_eq!(parse_counter_style_range_descriptor("0 1 2"), None);
    assert_eq!(parse_counter_style_range_descriptor("infinite"), None);
    assert_eq!(parse_counter_style_range_descriptor("default 1"), None);
}

#[test]
fn parses_rust_owned_counter_style_range_descriptors() {
    assert_eq!(
        parse_rust_owned_counter_style_range_descriptor("auto".as_bytes()),
        Some(RustOwnedCounterStyleRangeDescriptor::Auto)
    );
    assert_eq!(
        parse_rust_owned_counter_style_range_descriptor("infinite 0, 5 10".as_bytes()),
        Some(RustOwnedCounterStyleRangeDescriptor::List(vec![
            descriptor_keyword_value("infinite"),
            descriptor_integer_value(0, "0"),
            descriptor_integer_value(5, "5"),
            descriptor_integer_value(10, "10"),
        ]))
    );
    assert_eq!(
        parse_rust_owned_counter_style_range_descriptor("calc(1) calc(2)".as_bytes()),
        Some(RustOwnedCounterStyleRangeDescriptor::List(vec![
            descriptor_math_value("calc(1)"),
            descriptor_math_value("calc(2)"),
        ]))
    );
}

#[test]
fn parses_counter_style_additive_symbols_descriptors() {
    assert_eq!(parse_counter_style_additive_symbols_descriptor("1 \"I\""), Some(1));
    assert_eq!(parse_counter_style_additive_symbols_descriptor("\"I\" 1"), Some(1));
    assert_eq!(
        parse_counter_style_additive_symbols_descriptor("2 \"II\", 1 \"I\""),
        Some(2)
    );
    assert_eq!(
        parse_counter_style_additive_symbols_descriptor("calc(2) \"II\", 1 \"I\""),
        Some(2)
    );
}

#[test]
fn rejects_invalid_counter_style_additive_symbols_descriptors() {
    assert_eq!(parse_counter_style_additive_symbols_descriptor(""), None);
    assert_eq!(parse_counter_style_additive_symbols_descriptor("1"), None);
    assert_eq!(parse_counter_style_additive_symbols_descriptor("1 \"I\","), None);
    assert_eq!(parse_counter_style_additive_symbols_descriptor("1 \"I\" 2"), None);
    assert_eq!(parse_counter_style_additive_symbols_descriptor("-1 \"I\""), None);
    assert_eq!(parse_counter_style_additive_symbols_descriptor("default 1"), None);
}

#[test]
fn parses_rust_owned_counter_style_additive_symbols_descriptors() {
    assert_eq!(
        parse_rust_owned_counter_style_additive_symbols_descriptor("2 \"II\", \"I\" 1".as_bytes()),
        Some(vec![
            RustOwnedCounterStyleAdditiveTuple {
                order: CssNonnegativeIntegerSymbolPairOrder::IntegerFirst,
                source: "2 \"II\"".to_string(),
                integer: Some(2),
                integer_calculation: None,
                symbol: descriptor_string_value("II"),
            },
            RustOwnedCounterStyleAdditiveTuple {
                order: CssNonnegativeIntegerSymbolPairOrder::SymbolFirst,
                source: "\"I\" 1".to_string(),
                integer: Some(1),
                integer_calculation: None,
                symbol: descriptor_string_value("I"),
            },
        ])
    );
}

#[test]
fn parses_rust_owned_counter_style_pad_descriptors() {
    assert_eq!(
        parse_rust_owned_counter_style_pad_descriptor("2 \"0\"".as_bytes()),
        Some(RustOwnedCounterStylePadDescriptor {
            order: CssNonnegativeIntegerSymbolPairOrder::IntegerFirst,
            source: "2 \"0\"".to_string(),
            integer: Some(2),
            integer_calculation: None,
            symbol: descriptor_string_value("0"),
        })
    );
    assert_eq!(
        parse_rust_owned_counter_style_pad_descriptor("\"0\" 2".as_bytes()),
        Some(RustOwnedCounterStylePadDescriptor {
            order: CssNonnegativeIntegerSymbolPairOrder::SymbolFirst,
            source: "\"0\" 2".to_string(),
            integer: Some(2),
            integer_calculation: None,
            symbol: descriptor_string_value("0"),
        })
    );
    assert_eq!(parse_rust_owned_counter_style_pad_descriptor("2".as_bytes()), None);
}

#[test]
fn parses_crop_or_cross_descriptors() {
    assert_eq!(parse_crop_or_cross_descriptor("crop"), Some(CssCropOrCrossKind::Crop));
    assert_eq!(parse_crop_or_cross_descriptor("cross"), Some(CssCropOrCrossKind::Cross));
    assert_eq!(
        parse_crop_or_cross_descriptor("crop cross"),
        Some(CssCropOrCrossKind::CropAndCross)
    );
    assert_eq!(
        parse_crop_or_cross_descriptor("cross crop"),
        Some(CssCropOrCrossKind::CropAndCross)
    );
}

#[test]
fn rejects_invalid_crop_or_cross_descriptors() {
    assert_eq!(parse_crop_or_cross_descriptor(""), None);
    assert_eq!(parse_crop_or_cross_descriptor("none"), None);
    assert_eq!(parse_crop_or_cross_descriptor("none crop"), None);
    assert_eq!(parse_crop_or_cross_descriptor("crop crop"), None);
    assert_eq!(parse_crop_or_cross_descriptor("cross cross"), None);
    assert_eq!(parse_crop_or_cross_descriptor("cross crop cross"), None);
    assert_eq!(parse_crop_or_cross_descriptor("orange"), None);
    assert_eq!(parse_crop_or_cross_descriptor("auto"), None);
}

#[test]
fn parses_rust_owned_font_src_list_descriptors() {
    let parse_sources = |input: &str| {
        parse_rust_owned_font_src_list_descriptor(input.as_bytes())
            .map(|sources| sources.into_iter().map(|source| source.source).collect::<Vec<_>>())
    };

    assert_eq!(
        parse_sources("url(example.woff2), local(Example)"),
        Some(vec!["url(example.woff2)".to_string(), "local(Example)".to_string()])
    );
    assert_eq!(
        parse_sources(", url(example.woff2)"),
        Some(vec!["url(example.woff2)".to_string()])
    );
    assert_eq!(
        parse_sources("url(example.woff2) format(woff2) tech(variations, color-COLRv1)"),
        Some(vec![
            "url(example.woff2) format(woff2) tech(variations, color-COLRv1)".to_string()
        ])
    );
    assert_eq!(
        parse_rust_owned_font_src_list_descriptor("url(example.woff2) tech(variations) format(woff2)".as_bytes()),
        None
    );
    assert_eq!(parse_rust_owned_font_src_list_descriptor(",".as_bytes()), None);
}

#[test]
fn parses_font_weight_absolute_pair_descriptors() {
    assert_eq!(parse_font_weight_absolute_pair_descriptor("normal"), Some(1));
    assert_eq!(parse_font_weight_absolute_pair_descriptor("bold"), Some(1));
    assert_eq!(parse_font_weight_absolute_pair_descriptor("700"), Some(1));
    assert_eq!(parse_font_weight_absolute_pair_descriptor("calc(600 + 100)"), Some(1));
    assert_eq!(parse_font_weight_absolute_pair_descriptor("normal bold"), Some(2));
    assert_eq!(parse_font_weight_absolute_pair_descriptor("100 900"), Some(2));
    assert_eq!(
        parse_font_weight_absolute_pair_descriptor("calc(100 + 100) bold"),
        Some(2)
    );
}

#[test]
fn parses_rust_owned_font_weight_absolute_pair_descriptors() {
    assert_eq!(
        parse_rust_owned_font_weight_absolute_pair_descriptor("normal bold".as_bytes()),
        Some(vec![
            descriptor_keyword_value("normal"),
            descriptor_keyword_value("bold")
        ])
    );
    assert_eq!(
        parse_rust_owned_font_weight_absolute_pair_descriptor("calc(100 + 100) 900".as_bytes()),
        Some(vec![
            descriptor_math_integer_value("calc(100 + 100)"),
            descriptor_number_value(900.0, "900")
        ])
    );
    assert_eq!(
        parse_rust_owned_font_weight_absolute_pair_descriptor("100 400 900".as_bytes()),
        None
    );
}

#[test]
fn rejects_invalid_font_weight_absolute_pair_descriptors() {
    assert_eq!(parse_font_weight_absolute_pair_descriptor(""), None);
    assert_eq!(parse_font_weight_absolute_pair_descriptor("auto"), None);
    assert_eq!(parse_font_weight_absolute_pair_descriptor("lighter"), None);
    assert_eq!(parse_font_weight_absolute_pair_descriptor("0"), None);
    assert_eq!(parse_font_weight_absolute_pair_descriptor("1001"), None);
    assert_eq!(parse_font_weight_absolute_pair_descriptor("100 400 900"), None);
    assert_eq!(parse_font_weight_absolute_pair_descriptor("100 auto"), None);
}

#[test]
fn parses_namespace_rule_preludes() {
    assert_eq!(
        parse_namespace_rule_prelude("\"https://www.w3.org/1999/xhtml\""),
        Some((None, "https://www.w3.org/1999/xhtml".to_string()))
    );
    assert_eq!(
        parse_namespace_rule_prelude("svg url(http://www.w3.org/2000/svg)"),
        Some((Some("svg".to_string()), "http://www.w3.org/2000/svg".to_string()))
    );
    assert_eq!(
        parse_namespace_rule_prelude("math url(\"http://www.w3.org/1998/Math/MathML\")"),
        Some((
            Some("math".to_string()),
            "http://www.w3.org/1998/Math/MathML".to_string(),
        ))
    );
}

#[test]
fn rejects_invalid_namespace_rule_preludes() {
    assert_eq!(parse_namespace_rule_prelude(""), None);
    assert_eq!(parse_namespace_rule_prelude("svg"), None);
    assert_eq!(
        parse_namespace_rule_prelude("svg url(http://www.w3.org/2000/svg) extra"),
        None
    );
    assert_eq!(
        parse_namespace_rule_prelude("svg url(\"http://www.w3.org/2000/svg\" foo)"),
        None
    );
}

#[test]
fn parses_font_feature_values_family_name_lists() {
    assert_eq!(
        parse_font_feature_values_family_names("bongo"),
        Some(vec!["bongo".to_string()])
    );
    assert_eq!(
        parse_font_feature_values_family_names("\"Bongo Sans\", Great Vibes"),
        Some(vec!["Bongo Sans".to_string(), "Great Vibes".to_string()])
    );
}

#[test]
fn parses_family_names() {
    assert_eq!(parse_family_name("bongo"), Some(("bongo".to_string(), false)));
    assert_eq!(
        parse_family_name("Great Vibes"),
        Some(("Great Vibes".to_string(), false))
    );
    assert_eq!(parse_family_name("system-ui"), Some(("system-ui".to_string(), false)));
    assert_eq!(
        parse_family_name("\"Bongo Sans\""),
        Some(("Bongo Sans".to_string(), true))
    );
}

#[test]
fn rejects_invalid_family_names() {
    assert_eq!(parse_family_name(""), None);
    assert_eq!(parse_family_name("serif"), None);
    assert_eq!(parse_family_name("default"), None);
    assert_eq!(parse_family_name("initial"), None);
    assert_eq!(parse_family_name("\"Bongo\" Sans"), None);
    assert_eq!(parse_family_name("123"), None);
}

#[test]
fn rejects_invalid_font_feature_values_family_name_lists() {
    assert_eq!(parse_font_feature_values_family_names(""), None);
    assert_eq!(parse_font_feature_values_family_names("bongo,"), None);
    assert_eq!(parse_font_feature_values_family_names("serif"), None);
    assert_eq!(parse_font_feature_values_family_names("default"), None);
    assert_eq!(parse_font_feature_values_family_names("initial"), None);
    assert_eq!(parse_font_feature_values_family_names("\"Bongo\" Sans"), None);
    assert_eq!(parse_font_feature_values_family_names("123"), None);
}

#[test]
fn parses_font_feature_values_feature_values() {
    assert_eq!(parse_font_feature_values_feature_values("1"), Some(vec![1]));
    assert_eq!(parse_font_feature_values_feature_values("1 2"), Some(vec![1, 2]));
    assert_eq!(
        parse_font_feature_values_feature_values("0 4294967295"),
        Some(vec![0, u32::MAX])
    );
}

#[test]
fn rejects_invalid_font_feature_values_feature_values() {
    assert_eq!(parse_font_feature_values_feature_values(""), None);
    assert_eq!(parse_font_feature_values_feature_values("-1"), None);
    assert_eq!(parse_font_feature_values_feature_values("1.5"), None);
    assert_eq!(parse_font_feature_values_feature_values("calc(1)"), None);
    assert_eq!(parse_font_feature_values_feature_values("1 foo"), None);
    assert_eq!(parse_font_feature_values_feature_values("4294967296"), None);
}

#[test]
fn parses_empty_preludes() {
    assert!(parse_empty_prelude("".as_bytes()));
    assert!(parse_empty_prelude(" \t\n".as_bytes()));
    assert!(!parse_empty_prelude("foo".as_bytes()));
}

#[test]
fn parses_container_rule_preludes() {
    assert_eq!(
        parse_container_rule_prelude_items("sidebar (width > 20em), (height > 10em), card"),
        Some(vec![
            (Some("sidebar".to_string()), Some("(width > 20em)".to_string())),
            (None, Some("(height > 10em)".to_string())),
            (Some("card".to_string()), None),
        ])
    );
}

#[test]
fn rejects_invalid_container_rule_preludes() {
    assert_eq!(parse_container_rule_prelude_items(""), None);
    assert_eq!(parse_container_rule_prelude_items(","), None);
}

#[test]
fn parses_contain_values() {
    assert_eq!(
        parse_contain("none"),
        CssContainValue {
            kind: CssContainValueKind::None,
            is_size: false,
            is_inline_size: false,
            has_layout: false,
            has_style: false,
            has_paint: false,
        }
    );
    assert_eq!(parse_contain("strict").kind, CssContainValueKind::Strict);
    assert_eq!(parse_contain("content").kind, CssContainValueKind::Content);
    assert_eq!(
        parse_contain("layout paint size"),
        CssContainValue {
            kind: CssContainValueKind::List,
            is_size: true,
            is_inline_size: false,
            has_layout: true,
            has_style: false,
            has_paint: true,
        }
    );
    assert_eq!(
        parse_contain("style inline-size layout paint"),
        CssContainValue {
            kind: CssContainValueKind::List,
            is_size: false,
            is_inline_size: true,
            has_layout: true,
            has_style: true,
            has_paint: true,
        }
    );
}

#[test]
fn rejects_invalid_contain_values() {
    assert_eq!(parse_contain("").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("auto").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("none layout").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("strict paint").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("content style").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("size inline-size").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("layout layout").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("style style").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("paint paint").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("size, paint").kind, CssContainValueKind::Invalid);
    assert_eq!(parse_contain("size nonsense").kind, CssContainValueKind::Invalid);
}

#[test]
fn parses_scroll_function_values() {
    assert_eq!(
        parse_scroll_function("scroll()"),
        CssScrollFunctionValue {
            kind: CssScrollFunctionValueKind::Valid,
            scroller: CssScrollFunctionScrollerKind::None,
            axis: CssScrollFunctionAxisKind::None,
        }
    );
    assert_eq!(
        parse_scroll_function("scroll(root)").scroller,
        CssScrollFunctionScrollerKind::Root
    );
    assert_eq!(
        parse_scroll_function("scroll(self x)").axis,
        CssScrollFunctionAxisKind::X
    );
    assert_eq!(
        parse_scroll_function("scroll(y root)").scroller,
        CssScrollFunctionScrollerKind::Root
    );
    assert_eq!(
        parse_scroll_function("scroll(y root)").axis,
        CssScrollFunctionAxisKind::Y
    );
}

#[test]
fn rejects_invalid_scroll_function_values() {
    assert_eq!(parse_scroll_function("").kind, CssScrollFunctionValueKind::Invalid);
    assert_eq!(parse_scroll_function("root").kind, CssScrollFunctionValueKind::Invalid);
    assert_eq!(
        parse_scroll_function("scroll(root self)").kind,
        CssScrollFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_scroll_function("scroll(x y)").kind,
        CssScrollFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_scroll_function("scroll(root, x)").kind,
        CssScrollFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_scroll_function("scroll(foo)").kind,
        CssScrollFunctionValueKind::Invalid
    );
}

#[test]
fn parses_view_timeline_inset_values() {
    assert_eq!(
        parse_view_timeline_inset("auto"),
        CssViewTimelineInsetValue {
            kind: CssViewTimelineInsetValueKind::Valid,
            count: 1,
        }
    );
    assert_eq!(parse_view_timeline_inset("1px").count, 1);
    assert_eq!(parse_view_timeline_inset("10% auto").count, 2);
    assert_eq!(parse_view_timeline_inset("calc(1px + 2px) 5%").count, 2);
    assert!(matches!(
        parse_rust_owned_view_timeline_inset_value("calc(1px + 2px) 5%".as_bytes()),
        Some(values) if values.len() == 1
            && values[0].len() == 2
            && matches!(
                &values[0][0],
                RustOwnedNestedPrimitiveValue::MathFunction(RustOwnedMathFunction {
                    name,
                    source,
                    value_type: PropertyValueType::Length,
                    ..
                }) if name == "calc" && source == "calc(1px + 2px)"
            )
            && matches!(&values[0][1], RustOwnedNestedPrimitiveValue::Percentage(value) if *value == 5.0)
    ));
    assert_eq!(
        parse_rust_owned_view_timeline_inset_value("10% auto".as_bytes()),
        Some(vec![vec![
            RustOwnedNestedPrimitiveValue::Percentage(10.0),
            auto_keyword(),
        ]])
    );
    assert_eq!(
        parse_rust_owned_view_timeline_inset_value("10% auto, 2px".as_bytes()),
        Some(vec![
            vec![RustOwnedNestedPrimitiveValue::Percentage(10.0), auto_keyword(),],
            vec![RustOwnedNestedPrimitiveValue::Length {
                value: 2.0,
                unit: "px".to_string(),
            },],
        ])
    );
}

#[test]
fn rejects_invalid_view_timeline_inset_values() {
    assert_eq!(
        parse_view_timeline_inset("").kind,
        CssViewTimelineInsetValueKind::Invalid
    );
    assert_eq!(
        parse_view_timeline_inset("auto auto auto").kind,
        CssViewTimelineInsetValueKind::Invalid
    );
    assert_eq!(
        parse_view_timeline_inset("block").kind,
        CssViewTimelineInsetValueKind::Invalid
    );
    assert_eq!(
        parse_view_timeline_inset("1px, 2px").kind,
        CssViewTimelineInsetValueKind::Invalid
    );
}

#[test]
fn parses_view_timeline_inset_value_prefixes() {
    assert_eq!(parse_view_timeline_inset_prefix("auto inline").count, 1);
    assert_eq!(parse_view_timeline_inset_prefix("1px y").count, 1);
    assert_eq!(parse_view_timeline_inset_prefix("1px auto y").count, 2);
    assert_eq!(
        parse_view_timeline_inset_prefix("block").kind,
        CssViewTimelineInsetValueKind::Invalid
    );
}

#[test]
fn parses_view_function_values() {
    assert_eq!(
        parse_view_function("view()"),
        CssViewFunctionValue {
            kind: CssViewFunctionValueKind::Valid,
            axis: CssScrollFunctionAxisKind::None,
            inset: CssViewFunctionInsetKind::None,
            inset_position: CssViewFunctionInsetPosition::None,
        }
    );
    assert_eq!(
        parse_view_function("view(inline)").axis,
        CssScrollFunctionAxisKind::Inline
    );
    assert_eq!(
        parse_view_function("view(y 1px auto)").axis,
        CssScrollFunctionAxisKind::Y
    );
    assert_eq!(
        parse_view_function("view(y 1px auto)").inset,
        CssViewFunctionInsetKind::NonDefault
    );
    assert_eq!(
        parse_view_function("view(y 1px auto)").inset_position,
        CssViewFunctionInsetPosition::AfterAxis
    );
    assert_eq!(
        parse_view_function("view(1px y)").inset_position,
        CssViewFunctionInsetPosition::BeforeAxis
    );
    assert_eq!(
        parse_view_function("view(auto auto)").inset,
        CssViewFunctionInsetKind::Default
    );
    assert_eq!(
        parse_view_function("view(calc(1px + 2%) inline)").axis,
        CssScrollFunctionAxisKind::Inline
    );
}

#[test]
fn rejects_invalid_view_function_values() {
    assert_eq!(parse_view_function("").kind, CssViewFunctionValueKind::Invalid);
    assert_eq!(parse_view_function("view").kind, CssViewFunctionValueKind::Invalid);
    assert_eq!(parse_view_function("view(foo)").kind, CssViewFunctionValueKind::Invalid);
    assert_eq!(parse_view_function("view(x y)").kind, CssViewFunctionValueKind::Invalid);
    assert_eq!(
        parse_view_function("view(1px y auto)").kind,
        CssViewFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_view_function("view(1px 2px 3px)").kind,
        CssViewFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_view_function("view(y 1px, auto)").kind,
        CssViewFunctionValueKind::Invalid
    );
}

#[test]
fn parses_rect_values() {
    assert_eq!(
        parse_rect("rect(auto, 1px, 0, calc(2px + 3px))"),
        CssRectValueKind::Valid
    );
    assert_eq!(parse_rect("rect(auto 1px 0 calc(2px + 3px))"), CssRectValueKind::Valid);
}

#[test]
fn rejects_invalid_rect_values() {
    assert_eq!(parse_rect(""), CssRectValueKind::Invalid);
    assert_eq!(parse_rect("rect(auto, 1px 0, 2px)"), CssRectValueKind::Invalid);
    assert_eq!(parse_rect("rect(auto, 1px, 0)"), CssRectValueKind::Invalid);
    assert_eq!(parse_rect("rect(auto, 1px, 0, 2px,)"), CssRectValueKind::Invalid);
    assert_eq!(parse_rect("rect(auto, 1%, 0, 2px)"), CssRectValueKind::Invalid);
}

#[test]
fn parses_ratio_value_prefixes() {
    assert_eq!(
        parse_ratio_prefix("1"),
        CssRatioValue {
            kind: CssRatioValueKind::Valid,
            has_denominator: false,
            numerator: 1.0,
            denominator: 1.0,
        }
    );
    assert_eq!(parse_ratio_prefix("16 / 9").has_denominator, true);
    assert_eq!(parse_ratio_prefix("16 / 9 auto").has_denominator, true);
}

#[test]
fn rejects_invalid_ratio_value_prefixes() {
    assert_eq!(parse_ratio_prefix("").kind, CssRatioValueKind::Invalid);
    assert_eq!(parse_ratio_prefix("-1").kind, CssRatioValueKind::Invalid);
    assert_eq!(parse_ratio_prefix("1 /").kind, CssRatioValueKind::Invalid);
    assert_eq!(parse_ratio_prefix("1 / auto").kind, CssRatioValueKind::Invalid);
}

#[test]
fn parses_position_values() {
    assert_eq!(parse_position("left"), CssPositionValueKind::Valid);
    assert_eq!(parse_position("center center"), CssPositionValueKind::Valid);
    assert_eq!(parse_position("20% 0%"), CssPositionValueKind::Valid);
    assert_eq!(parse_position("left 12px top 13px"), CssPositionValueKind::Valid);
    assert_eq!(parse_position("center 10px"), CssPositionValueKind::Valid);
    assert_eq!(
        parse_position("calc(10px + 0.5em) calc(10px - 0.5em)"),
        CssPositionValueKind::Valid
    );
}

#[test]
fn parses_background_position_3_value_syntax() {
    assert_eq!(
        parse_background_position("center right 7%"),
        CssPositionValueKind::Valid
    );
    assert_eq!(
        parse_background_position("left 10px center"),
        CssPositionValueKind::Valid
    );
    assert_eq!(
        parse_background_position("top 15px center"),
        CssPositionValueKind::Valid
    );
    assert_eq!(parse_background_position("right top 14%"), CssPositionValueKind::Valid);
    assert_eq!(
        parse_background_position("bottom 16% left"),
        CssPositionValueKind::Valid
    );
    assert_eq!(parse_background_position("left center"), CssPositionValueKind::Valid);
}

#[test]
fn rejects_invalid_position_values() {
    assert_eq!(parse_position(""), CssPositionValueKind::Invalid);
    assert_eq!(parse_position("left right"), CssPositionValueKind::Invalid);
    assert_eq!(parse_position("top bottom"), CssPositionValueKind::Invalid);
    assert_eq!(parse_position("1% center 2px"), CssPositionValueKind::Invalid);
    assert_eq!(parse_position("right 7% 50%"), CssPositionValueKind::Invalid);
    assert_eq!(parse_position("50% top 8px"), CssPositionValueKind::Invalid);
    assert_eq!(parse_position("left 10px 50%"), CssPositionValueKind::Invalid);
    assert_eq!(parse_position("right 11% 100%"), CssPositionValueKind::Invalid);
    assert_eq!(parse_position("left / cover"), CssPositionValueKind::Invalid);
}

#[test]
fn parses_background_position_longhand_values() {
    assert_eq!(parse_background_position_x("center"), CssPositionValueKind::Valid);
    assert_eq!(parse_background_position_x("left -20%"), CssPositionValueKind::Valid);
    assert_eq!(parse_background_position_x("right 10px"), CssPositionValueKind::Valid);
    assert_eq!(parse_background_position_x("x-start"), CssPositionValueKind::Valid);
    assert_eq!(parse_background_position_x("x-end 1px"), CssPositionValueKind::Valid);
    assert_eq!(
        parse_background_position_x("calc(10px - 0.5em)"),
        CssPositionValueKind::Valid
    );

    assert_eq!(parse_background_position_y("center"), CssPositionValueKind::Valid);
    assert_eq!(parse_background_position_y("top -20%"), CssPositionValueKind::Valid);
    assert_eq!(parse_background_position_y("bottom 10px"), CssPositionValueKind::Valid);
    assert_eq!(parse_background_position_y("y-start"), CssPositionValueKind::Valid);
    assert_eq!(parse_background_position_y("y-end 1px"), CssPositionValueKind::Valid);
    assert_eq!(
        parse_background_position_y("calc(10px - 0.5em)"),
        CssPositionValueKind::Valid
    );
}

#[test]
fn rejects_invalid_background_position_longhand_values() {
    assert_eq!(parse_background_position_x("top"), CssPositionValueKind::Invalid);
    assert_eq!(
        parse_background_position_x("center 10px"),
        CssPositionValueKind::Invalid
    );
    assert_eq!(parse_background_position_x("20% left"), CssPositionValueKind::Invalid);
    assert_eq!(parse_background_position_x("right left"), CssPositionValueKind::Invalid);
    assert_eq!(
        parse_background_position_x("x-start center"),
        CssPositionValueKind::Invalid
    );

    assert_eq!(parse_background_position_y("left"), CssPositionValueKind::Invalid);
    assert_eq!(
        parse_background_position_y("center 10px"),
        CssPositionValueKind::Invalid
    );
    assert_eq!(parse_background_position_y("20% top"), CssPositionValueKind::Invalid);
    assert_eq!(parse_background_position_y("bottom top"), CssPositionValueKind::Invalid);
    assert_eq!(
        parse_background_position_y("y-start center"),
        CssPositionValueKind::Invalid
    );
}

#[test]
fn parses_background_size_values() {
    assert_eq!(parse_background_size("auto"), CssBackgroundSizeValueKind::Valid);
    assert_eq!(parse_background_size("cover"), CssBackgroundSizeValueKind::Valid);
    assert_eq!(parse_background_size("contain"), CssBackgroundSizeValueKind::Valid);
    assert_eq!(parse_background_size("1px"), CssBackgroundSizeValueKind::Valid);
    assert_eq!(parse_background_size("2% 3%"), CssBackgroundSizeValueKind::Valid);
    assert_eq!(parse_background_size("auto 1px"), CssBackgroundSizeValueKind::Valid);
    assert_eq!(
        parse_background_size("calc(10px + 5%) auto"),
        CssBackgroundSizeValueKind::Valid
    );
}

#[test]
fn rejects_invalid_background_size_values() {
    assert_eq!(parse_background_size(""), CssBackgroundSizeValueKind::Invalid);
    assert_eq!(parse_background_size("-1px"), CssBackgroundSizeValueKind::Invalid);
    assert_eq!(parse_background_size("2% -3%"), CssBackgroundSizeValueKind::Invalid);
    assert_eq!(parse_background_size("cover 1px"), CssBackgroundSizeValueKind::Invalid);
    assert_eq!(
        parse_background_size("1px 2px 3px"),
        CssBackgroundSizeValueKind::Invalid
    );
}

#[test]
fn parses_repeat_style_values() {
    assert_eq!(parse_repeat_style("repeat-x"), CssRepeatStyleValueKind::Valid);
    assert_eq!(parse_repeat_style("repeat-y"), CssRepeatStyleValueKind::Valid);
    assert_eq!(parse_repeat_style("repeat"), CssRepeatStyleValueKind::Valid);
    assert_eq!(parse_repeat_style("space"), CssRepeatStyleValueKind::Valid);
    assert_eq!(parse_repeat_style("round"), CssRepeatStyleValueKind::Valid);
    assert_eq!(parse_repeat_style("no-repeat"), CssRepeatStyleValueKind::Valid);
    assert_eq!(parse_repeat_style("repeat space"), CssRepeatStyleValueKind::Valid);
    assert_eq!(parse_repeat_style("round no-repeat"), CssRepeatStyleValueKind::Valid);
}

#[test]
fn rejects_invalid_repeat_style_values() {
    assert_eq!(parse_repeat_style(""), CssRepeatStyleValueKind::Invalid);
    assert_eq!(parse_repeat_style("auto"), CssRepeatStyleValueKind::Invalid);
    assert_eq!(parse_repeat_style("repeat-z"), CssRepeatStyleValueKind::Invalid);
    assert_eq!(parse_repeat_style("repeat undefined"), CssRepeatStyleValueKind::Invalid);
    assert_eq!(parse_repeat_style("repeat-x repeat"), CssRepeatStyleValueKind::Invalid);
    assert_eq!(
        parse_repeat_style("repeat space round"),
        CssRepeatStyleValueKind::Invalid
    );
}

#[test]
fn parses_color_function_values() {
    assert_eq!(parse_color_function("rgb(1 2 3)"), CssColorFunctionValueKind::Valid);
    assert_eq!(
        parse_color_function("rgb(1, 2, 3, 50%)"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("hsl(90deg 50% 50% / none)"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("hwb(90 10% 20%)"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("lab(10% 20 30 / 40%)"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("lch(10% 20 30deg)"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("oklab(10% 20 30)"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("oklch(10% 20 30)"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("color(display-p3 1 0.5 0 / 80%)"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("color-mix(in oklab, red 40%, rgb(0 0 255))"),
        CssColorFunctionValueKind::Valid
    );
    assert_eq!(
        parse_color_function("light-dark(Canvas, color(srgb 1 1 1))"),
        CssColorFunctionValueKind::Valid
    );
}

#[test]
fn rejects_invalid_color_function_values() {
    assert_eq!(parse_color_function(""), CssColorFunctionValueKind::Invalid);
    assert_eq!(parse_color_function("red"), CssColorFunctionValueKind::Invalid);
    assert_eq!(parse_color_function("rgb(1 2)"), CssColorFunctionValueKind::Invalid);
    assert_eq!(
        parse_color_function("rgb(none, 2, 3)"),
        CssColorFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_color_function("rgb(1%, 2, 3)"),
        CssColorFunctionValueKind::Invalid
    );
    assert_eq!(parse_color_function("hsl(1, 2, 3)"), CssColorFunctionValueKind::Invalid);
    assert_eq!(
        parse_color_function("color(display-p3 1 2)"),
        CssColorFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_color_function("color(unknown 1 2 3)"),
        CssColorFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_color_function("color-mix(in bogus, red, blue)"),
        CssColorFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_color_function("light-dark(red blue)"),
        CssColorFunctionValueKind::Invalid
    );
}

#[test]
fn parses_color_values() {
    assert_eq!(parse_color("red"), CssColorValueKind::Valid);
    assert_eq!(parse_color("transparent"), CssColorValueKind::Valid);
    assert_eq!(parse_color("currentColor"), CssColorValueKind::Valid);
    assert_eq!(parse_color("CanvasText"), CssColorValueKind::Valid);
    assert_eq!(parse_color("-libweb-palette-window-text"), CssColorValueKind::Valid);
    assert_eq!(parse_color("#abc"), CssColorValueKind::Valid);
    assert_eq!(parse_color("#abcd"), CssColorValueKind::Valid);
    assert_eq!(parse_color("#aabbcc"), CssColorValueKind::Valid);
    assert_eq!(parse_color("#aabbccdd"), CssColorValueKind::Valid);
    assert_eq!(parse_color("rgb(1 2 3)"), CssColorValueKind::Valid);
    assert_eq!(parse_quirky_color("000000"), CssColorValueKind::Valid);
    assert_eq!(parse_quirky_color("123abc"), CssColorValueKind::Valid);
}

#[test]
fn parses_simple_colors_as_rust_owned_values() {
    assert_eq!(
        parse_simple_color("red", false),
        Some((CssParsedColorKind::Rgba, 255, 0, 0, 255, "red".to_string()))
    );
    assert_eq!(
        parse_simple_color("transparent", false),
        Some((CssParsedColorKind::Rgba, 0, 0, 0, 0, "transparent".to_string()))
    );
    assert_eq!(
        parse_simple_color("#0f08", false),
        Some((CssParsedColorKind::Rgba, 0, 255, 0, 136, String::new()))
    );
    assert_eq!(
        parse_simple_color("#336699cc", false),
        Some((CssParsedColorKind::Rgba, 0x33, 0x66, 0x99, 0xcc, String::new()))
    );
    assert_eq!(
        parse_simple_color("currentColor", false),
        Some((CssParsedColorKind::Keyword, 0, 0, 0, 0, "currentColor".to_string()))
    );
    assert_eq!(
        parse_simple_color("CanvasText", false),
        Some((CssParsedColorKind::Keyword, 0, 0, 0, 0, "CanvasText".to_string()))
    );
    assert_eq!(
        parse_simple_color("123abc", true),
        Some((CssParsedColorKind::Rgba, 0x12, 0x3a, 0xbc, 255, String::new()))
    );
    assert_eq!(
        parse_simple_color("abc", true),
        Some((CssParsedColorKind::Rgba, 0xaa, 0xbb, 0xcc, 255, String::new()))
    );
    assert_eq!(parse_simple_color("a", true), None);
    assert_eq!(parse_simple_color("123abc", false), None);
    assert_eq!(parse_simple_color("rgb(1 2 3)", false), None);
}

#[test]
fn rejects_invalid_color_values() {
    assert_eq!(parse_color(""), CssColorValueKind::Invalid);
    assert_eq!(parse_color("not-a-color"), CssColorValueKind::Invalid);
    assert_eq!(parse_color("-libweb-unknown"), CssColorValueKind::Invalid);
    assert_eq!(parse_color("#ab"), CssColorValueKind::Invalid);
    assert_eq!(parse_color("#xyz"), CssColorValueKind::Invalid);
    assert_eq!(parse_color("rgb(1 2)"), CssColorValueKind::Invalid);
    assert_eq!(parse_color("000000"), CssColorValueKind::Invalid);
}

#[test]
fn parses_image_set_values() {
    assert_eq!(
        parse_image_set("image-set(url(example.png))"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(\"example.png\" 2x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(url(example.png) type(\"image/png\"))"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(url(example.png) type(\"image/png\") 2x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("-webkit-image-set(url(example.png) 1dppx)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(linear-gradient(black, white) 1x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(linear-gradient(to bottom in oklab, black, white) 1x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(-webkit-repeating-linear-gradient(top, black, white) 1x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(radial-gradient(circle 1px at center, black, white) 1x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(radial-gradient(black, white) 1x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(radial-gradient(at right center in lch longer hue, black, white) 1x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(radial-gradient(200px 100px ellipse at 25% 50%, yellow, #009966, purple) 1x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(conic-gradient(from 45deg at center, black, white) 1x)"),
        CssImageSetValueKind::Valid
    );
    assert_eq!(
        parse_image_set("image-set(conic-gradient(in oklch longer hue at left 10px top 50em, black, white) 1x)"),
        CssImageSetValueKind::Valid
    );
}

#[test]
fn rejects_invalid_image_set_values() {
    assert_eq!(parse_image_set(""), CssImageSetValueKind::Invalid);
    assert_eq!(parse_image_set("url(example.png)"), CssImageSetValueKind::Invalid);
    assert_eq!(parse_image_set("image-set()"), CssImageSetValueKind::Invalid);
    assert_eq!(parse_image_set("image-set(none)"), CssImageSetValueKind::Invalid);
    assert_eq!(
        parse_image_set("image-set(url(example.png) -1x)"),
        CssImageSetValueKind::Invalid
    );
    assert_eq!(
        parse_image_set("image-set(url(example.png) 1x 2x)"),
        CssImageSetValueKind::Invalid
    );
    assert_eq!(
        parse_image_set("image-set(url(example.png) type(\"image/png\") type(\"image/jpeg\"))"),
        CssImageSetValueKind::Invalid
    );
    assert_eq!(
        parse_image_set("image-set(url(example.png) type(image/png))"),
        CssImageSetValueKind::Invalid
    );
    assert_eq!(
        parse_image_set("image-set(image-set(url(example.png)) 2x)"),
        CssImageSetValueKind::Invalid
    );
    assert_eq!(
        parse_image_set("image-set(linear-gradient(to left right, black, white) 1x)"),
        CssImageSetValueKind::Invalid
    );
    assert_eq!(
        parse_image_set("image-set(linear-gradient(50%, black, white) 1x)"),
        CssImageSetValueKind::Invalid
    );
    assert_eq!(
        parse_image_set("image-set(radial-gradient(circle 1px 2px, black, white) 1x)"),
        CssImageSetValueKind::Invalid
    );
    assert_eq!(
        parse_image_set("image-set(conic-gradient(from 10%, black, white) 1x)"),
        CssImageSetValueKind::Invalid
    );
}

#[test]
fn parses_primitive_value_prefixes() {
    assert_eq!(
        parse_primitive_prefix("1", CssPrimitiveValueType::Integer),
        CssPrimitiveValueKind::Integer
    );
    assert_eq!(
        parse_primitive_prefix("1.5", CssPrimitiveValueType::Number),
        CssPrimitiveValueKind::Number
    );
    assert_eq!(
        parse_primitive_prefix("50%", CssPrimitiveValueType::Percentage),
        CssPrimitiveValueKind::Percentage
    );
    assert_eq!(
        parse_primitive_prefix("1deg", CssPrimitiveValueType::Angle),
        CssPrimitiveValueKind::Angle
    );
    assert_eq!(
        parse_primitive_prefix("1fr", CssPrimitiveValueType::Flex),
        CssPrimitiveValueKind::Flex
    );
    assert_eq!(
        parse_primitive_prefix("1Hz", CssPrimitiveValueType::Frequency),
        CssPrimitiveValueKind::Frequency
    );
    assert_eq!(
        parse_primitive_prefix("1px", CssPrimitiveValueType::Length),
        CssPrimitiveValueKind::Length
    );
    assert_eq!(
        parse_primitive_prefix("96dpi", CssPrimitiveValueType::Resolution),
        CssPrimitiveValueKind::Resolution
    );
    assert_eq!(
        parse_primitive_prefix("\"hello\"", CssPrimitiveValueType::String),
        CssPrimitiveValueKind::String
    );
    assert_eq!(
        parse_primitive_prefix("1s", CssPrimitiveValueType::Time),
        CssPrimitiveValueKind::Time
    );
    assert_eq!(
        parse_primitive_prefix("50%", CssPrimitiveValueType::Opacity),
        CssPrimitiveValueKind::Opacity
    );
    assert_eq!(
        parse_primitive_prefix("random(3, 1)", CssPrimitiveValueType::Number),
        CssPrimitiveValueKind::Number
    );
    assert_eq!(
        parse_primitive_prefix("random(10%, 30%)", CssPrimitiveValueType::Length),
        CssPrimitiveValueKind::Length
    );
}

#[test]
fn parses_primitive_value_prefix_options() {
    assert_eq!(
        parse_primitive_prefix("0", CssPrimitiveValueType::Length),
        CssPrimitiveValueKind::Length
    );
    assert_eq!(
        parse_primitive_prefix("1", CssPrimitiveValueType::Length),
        CssPrimitiveValueKind::Invalid
    );
    assert_eq!(
        parse_primitive_prefix_with_options(
            "1",
            CssPrimitiveValueType::Length,
            CssPrimitiveValueOptions {
                allow_quirky_length: true,
                allow_quirky_color: false,
                allow_svg_unitless_length: false,
                allow_svg_unitless_angle: false,
            }
        ),
        CssPrimitiveValueKind::Length
    );
    assert_eq!(
        parse_primitive_prefix_with_options(
            "1",
            CssPrimitiveValueType::Angle,
            CssPrimitiveValueOptions {
                allow_quirky_length: false,
                allow_quirky_color: false,
                allow_svg_unitless_length: false,
                allow_svg_unitless_angle: true,
            }
        ),
        CssPrimitiveValueKind::Angle
    );
}

#[test]
fn rejects_invalid_primitive_value_prefixes() {
    assert_eq!(
        parse_primitive_prefix("1.5", CssPrimitiveValueType::Integer),
        CssPrimitiveValueKind::Invalid
    );
    assert_eq!(
        parse_primitive_prefix("1px", CssPrimitiveValueType::Percentage),
        CssPrimitiveValueKind::Invalid
    );
    assert_eq!(
        parse_primitive_prefix("-1dpi", CssPrimitiveValueType::Resolution),
        CssPrimitiveValueKind::Invalid
    );
    assert_eq!(
        parse_primitive_prefix("ident", CssPrimitiveValueType::String),
        CssPrimitiveValueKind::Invalid
    );
}

#[test]
fn parses_primitive_values() {
    assert_eq!(
        parse_primitive("1", CssPrimitiveValueType::Integer),
        CssPrimitiveValueKind::Integer
    );
    assert_eq!(
        parse_primitive("1.5", CssPrimitiveValueType::Number),
        CssPrimitiveValueKind::Number
    );
    assert_eq!(
        parse_primitive("50%", CssPrimitiveValueType::Percentage),
        CssPrimitiveValueKind::Percentage
    );
    assert_eq!(
        parse_primitive("1deg", CssPrimitiveValueType::Angle),
        CssPrimitiveValueKind::Angle
    );
    assert_eq!(
        parse_primitive("1fr", CssPrimitiveValueType::Flex),
        CssPrimitiveValueKind::Flex
    );
    assert_eq!(
        parse_primitive("1Hz", CssPrimitiveValueType::Frequency),
        CssPrimitiveValueKind::Frequency
    );
    assert_eq!(
        parse_primitive("1px", CssPrimitiveValueType::Length),
        CssPrimitiveValueKind::Length
    );
    assert_eq!(
        parse_primitive("96dpi", CssPrimitiveValueType::Resolution),
        CssPrimitiveValueKind::Resolution
    );
    assert_eq!(
        parse_primitive("\"hello\"", CssPrimitiveValueType::String),
        CssPrimitiveValueKind::String
    );
    assert_eq!(
        parse_primitive("1s", CssPrimitiveValueType::Time),
        CssPrimitiveValueKind::Time
    );
    assert_eq!(
        parse_primitive("50%", CssPrimitiveValueType::Opacity),
        CssPrimitiveValueKind::Opacity
    );
}

#[test]
fn parses_primitive_value_options() {
    assert_eq!(
        parse_primitive("0", CssPrimitiveValueType::Length),
        CssPrimitiveValueKind::Length
    );
    assert_eq!(
        parse_primitive_with_options(
            "1",
            CssPrimitiveValueType::Length,
            CssPrimitiveValueOptions {
                allow_quirky_length: true,
                allow_quirky_color: false,
                allow_svg_unitless_length: false,
                allow_svg_unitless_angle: false,
            }
        ),
        CssPrimitiveValueKind::Length
    );
    assert_eq!(
        parse_primitive_with_options(
            "1",
            CssPrimitiveValueType::Angle,
            CssPrimitiveValueOptions {
                allow_quirky_length: false,
                allow_quirky_color: false,
                allow_svg_unitless_length: false,
                allow_svg_unitless_angle: true,
            }
        ),
        CssPrimitiveValueKind::Angle
    );
}

#[test]
fn rejects_invalid_primitive_values() {
    assert_eq!(
        parse_primitive("1px 2px", CssPrimitiveValueType::Length),
        CssPrimitiveValueKind::Invalid
    );
    assert_eq!(
        parse_primitive("1.5", CssPrimitiveValueType::Integer),
        CssPrimitiveValueKind::Invalid
    );
    assert_eq!(
        parse_primitive("1px", CssPrimitiveValueType::Percentage),
        CssPrimitiveValueKind::Invalid
    );
    assert_eq!(
        parse_primitive("-1dpi", CssPrimitiveValueType::Resolution),
        CssPrimitiveValueKind::Invalid
    );
    assert_eq!(
        parse_primitive("ident", CssPrimitiveValueType::String),
        CssPrimitiveValueKind::Invalid
    );
}

#[test]
fn parses_primitive_generated_property_value_types() {
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::Angle,
        b"1deg"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::AnglePercentage,
        b"1deg"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::AnglePercentage,
        b"50%"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::Flex,
        b"1fr"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::Frequency,
        b"1Hz"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::FrequencyPercentage,
        b"1Hz"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::FrequencyPercentage,
        b"50%"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::LengthPercentage,
        b"1px"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::LengthPercentage,
        b"50%"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::Resolution,
        b"96dpi"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::TimePercentage,
        b"1s"
    ));
    assert!(component_values_parse_as_property_value_type(
        PropertyValueType::TimePercentage,
        b"50%"
    ));
    assert!(!component_values_parse_as_property_value_type(
        PropertyValueType::Length,
        b"64"
    ));
    assert!(component_values_parse_as_property_value_type_with_options(
        PropertyValueType::Length,
        b"64",
        CssPrimitiveValueOptions {
            allow_quirky_length: false,
            allow_quirky_color: false,
            allow_svg_unitless_length: true,
            allow_svg_unitless_angle: false,
        }
    ));
}

#[test]
fn parses_primitive_generated_property_value_types_as_rust_owned_values() {
    let parse_generated_value = |property_id, value_type, input: &str| {
        parse_rust_owned_generated_longhand_value(property_id, value_type, input.as_bytes(), &parse(input))
    };

    assert_eq!(
        parse_generated_value(PropertyId::Rotate, PropertyValueType::Angle, "1deg"),
        RustOwnedStyleValue {
            property_id: PropertyId::Rotate,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Angle {
                    value: 1.0,
                    unit: "deg".to_string(),
                },
                value_type: PropertyValueType::Angle,
            }),
        }
    );
    assert_eq!(
        parse_generated_value(PropertyId::GridTemplateColumns, PropertyValueType::Flex, "1fr"),
        RustOwnedStyleValue {
            property_id: PropertyId::GridTemplateColumns,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Flex {
                    value: 1.0,
                    unit: "fr".to_string(),
                },
                value_type: PropertyValueType::Flex,
            }),
        }
    );
    assert_eq!(
        parse_generated_value(PropertyId::TransitionDuration, PropertyValueType::Frequency, "1Hz"),
        RustOwnedStyleValue {
            property_id: PropertyId::TransitionDuration,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Frequency {
                    value: 1.0,
                    unit: "Hz".to_string(),
                },
                value_type: PropertyValueType::Frequency,
            }),
        }
    );
    assert_eq!(
        parse_generated_value(PropertyId::Rotate, PropertyValueType::Resolution, "96dpi"),
        RustOwnedStyleValue {
            property_id: PropertyId::Rotate,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Resolution {
                    value: 96.0,
                    unit: "dpi".to_string(),
                },
                value_type: PropertyValueType::Resolution,
            }),
        }
    );
}

#[test]
fn emits_primitive_generated_dimension_property_value_types() {
    let value = |property_id, primitive_kind, numeric_value, unit: &str, value_type| ParsedStyleValue {
        kind: CssStyleValueKind::Primitive,
        property_id,
        primitive_kind,
        numeric_value: Some(numeric_value),
        secondary_numeric_value: None,
        color: None,
        value: unit.to_string(),
        value_type: property_value_type_name(value_type).to_string(),
    };

    assert_eq!(
        emit_style_value(&RustOwnedStyleValue {
            property_id: PropertyId::Rotate,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Angle {
                    value: 1.0,
                    unit: "deg".to_string(),
                },
                value_type: PropertyValueType::Angle,
            }),
        }),
        Some(value(
            PropertyId::Rotate,
            CssPrimitiveValueKind::Angle,
            1.0,
            "deg",
            PropertyValueType::Angle
        ))
    );
    assert_eq!(
        emit_style_value(&RustOwnedStyleValue {
            property_id: PropertyId::GridTemplateColumns,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Flex {
                    value: 1.0,
                    unit: "fr".to_string(),
                },
                value_type: PropertyValueType::Flex,
            }),
        }),
        Some(value(
            PropertyId::GridTemplateColumns,
            CssPrimitiveValueKind::Flex,
            1.0,
            "fr",
            PropertyValueType::Flex
        ))
    );
    assert_eq!(
        emit_style_value(&RustOwnedStyleValue {
            property_id: PropertyId::TransitionDuration,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Frequency {
                    value: 1.0,
                    unit: "Hz".to_string(),
                },
                value_type: PropertyValueType::Frequency,
            }),
        }),
        Some(value(
            PropertyId::TransitionDuration,
            CssPrimitiveValueKind::Frequency,
            1.0,
            "Hz",
            PropertyValueType::Frequency
        ))
    );
    assert_eq!(
        emit_style_value(&RustOwnedStyleValue {
            property_id: PropertyId::Rotate,
            value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Nested {
                value: RustOwnedNestedPrimitiveValue::Resolution {
                    value: 96.0,
                    unit: "dpi".to_string(),
                },
                value_type: PropertyValueType::Resolution,
            }),
        }),
        Some(value(
            PropertyId::Rotate,
            CssPrimitiveValueKind::Resolution,
            96.0,
            "dpi",
            PropertyValueType::Resolution
        ))
    );
}

#[test]
fn emits_svg_unitless_length_style_values_with_parser_options() {
    assert_eq!(parse_style_value(&[PropertyId::Width], "64"), None);
    assert_eq!(
        parse_style_value_with_options(
            &[PropertyId::Width],
            "64",
            CssPrimitiveValueOptions {
                allow_quirky_length: false,
                allow_quirky_color: false,
                allow_svg_unitless_length: true,
                allow_svg_unitless_angle: false,
            }
        ),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::Primitive,
            property_id: PropertyId::Width,
            primitive_kind: CssPrimitiveValueKind::Length,
            numeric_value: Some(64.0),
            secondary_numeric_value: None,
            color: None,
            value: "px".to_string(),
            value_type: "Length".to_string(),
        })
    );
}

#[test]
fn emits_dimension_containing_calc_as_length_when_number_is_also_accepted() {
    assert_eq!(
        parse_style_value(&[PropertyId::StrokeWidth], "calc(2px)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::MathFunction,
            property_id: PropertyId::StrokeWidth,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "calc(2px)".to_string(),
            value_type: "Length".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::LineHeight], "calc(10px + 0.5em)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::MathFunction,
            property_id: PropertyId::LineHeight,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "calc(10px + 0.5em)".to_string(),
            value_type: "Length".to_string(),
        })
    );
}

#[test]
fn emits_percentage_containing_math_as_length_when_percentages_resolve_to_length() {
    assert_eq!(
        parse_style_value(&[PropertyId::StrokeWidth], "random(10%, 30%)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::MathFunction,
            property_id: PropertyId::StrokeWidth,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "random(10%, 30%)".to_string(),
            value_type: "Length".to_string(),
        })
    );
    assert_eq!(
        parse_style_value(&[PropertyId::LineHeight], "calc(200% + 10px)"),
        Some(ParsedStyleValue {
            kind: CssStyleValueKind::MathFunction,
            property_id: PropertyId::LineHeight,
            primitive_kind: CssPrimitiveValueKind::Invalid,
            numeric_value: None,
            secondary_numeric_value: None,
            color: None,
            value: "calc(200% + 10px)".to_string(),
            value_type: "Length".to_string(),
        })
    );
}

#[test]
fn rejects_invalid_primitive_generated_property_value_types() {
    assert!(!component_values_parse_as_property_value_type(
        PropertyValueType::Angle,
        b"50%"
    ));
    assert!(!component_values_parse_as_property_value_type(
        PropertyValueType::Flex,
        b"1px"
    ));
    assert!(!component_values_parse_as_property_value_type(
        PropertyValueType::Frequency,
        b"50%"
    ));
    assert!(!component_values_parse_as_property_value_type(
        PropertyValueType::Resolution,
        b"-1dpi"
    ));
    assert!(!component_values_parse_as_property_value_type(
        PropertyValueType::TimePercentage,
        b"1px"
    ));
}

#[test]
fn parses_easing_values() {
    assert_eq!(parse_easing("step-start"), CssEasingValueKind::Valid);
    assert_eq!(parse_easing("step-end"), CssEasingValueKind::Valid);
    assert_eq!(parse_easing("linear(0, 0.5, 1)"), CssEasingValueKind::Valid);
    assert_eq!(parse_easing("linear(0 5%, 0.5 10%, 1 100%)"), CssEasingValueKind::Valid);
    assert_eq!(parse_easing("linear(5% 0, 1 100%)"), CssEasingValueKind::Valid);
    assert_eq!(parse_easing("linear(0.5 5% 10%)"), CssEasingValueKind::Valid);
    assert_eq!(parse_easing("cubic-bezier(0, 0, 1, 1000)"), CssEasingValueKind::Valid);
    assert_eq!(parse_easing("steps(10, jump-none)"), CssEasingValueKind::Valid);
    assert_eq!(parse_easing("steps(10, end)"), CssEasingValueKind::Valid);
}

#[test]
fn rejects_invalid_easing_values() {
    assert_eq!(parse_easing("linear()"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("linear(a, b, c)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("linear(5 10)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("linear(5% 10%)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("linear(0.5 5% 10)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("cubic-bezier(0, 0, 0)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("cubic-bezier(2, 0, 0, 0)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("cubic-bezier(0, 0, 2, 0)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("steps(1.5)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("steps(-1)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("steps(0, jump-none)"), CssEasingValueKind::Invalid);
    assert_eq!(parse_easing("steps(1, elsewhere)"), CssEasingValueKind::Invalid);
}

#[test]
fn parses_transform_function_values() {
    assert_eq!(
        parse_transform_function("matrix(1, 0, 0, 1, 10, 20)"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("matrix3d(1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 10, 20, 30, 1)"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("translate(10px, 20%)"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("translateX(calc(10px + 5%))"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("translateZ(0)"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("scale(1, 50%)"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("rotate(0)"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("rotate3d(1, 0, 0, 45deg)"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("perspective(none)"),
        CssTransformFunctionValueKind::Valid
    );
    assert_eq!(
        parse_transform_function("skew(10deg, 0)"),
        CssTransformFunctionValueKind::Valid
    );
}

#[test]
fn rejects_invalid_transform_function_values() {
    assert_eq!(parse_transform_function("none"), CssTransformFunctionValueKind::Invalid);
    assert_eq!(
        parse_transform_function("unknown(1)"),
        CssTransformFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_transform_function("translate()"),
        CssTransformFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_transform_function("translate(10px, 20%, 30px)"),
        CssTransformFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_transform_function("translateZ(10%)"),
        CssTransformFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_transform_function("scale()"),
        CssTransformFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_transform_function("rotate(1px)"),
        CssTransformFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_transform_function("matrix(1, 0, 0, 1, 10)"),
        CssTransformFunctionValueKind::Invalid
    );
    assert_eq!(
        parse_transform_function("matrix(1 0 0 1 10 20)"),
        CssTransformFunctionValueKind::Invalid
    );
}

#[test]
fn parses_transform_longhand_values() {
    assert_eq!(parse_translate("none"), CssTransformLonghandValueKind::Valid);
    assert_eq!(parse_translate("10px 20% 30px"), CssTransformLonghandValueKind::Valid);
    assert_eq!(parse_translate("calc(10px + 5%)"), CssTransformLonghandValueKind::Valid);
    assert_eq!(parse_scale("none"), CssTransformLonghandValueKind::Valid);
    assert_eq!(parse_scale("1 50% calc(1 + 2)"), CssTransformLonghandValueKind::Valid);
    assert_eq!(parse_rotate("none"), CssTransformLonghandValueKind::Valid);
    assert_eq!(parse_rotate("45deg"), CssTransformLonghandValueKind::Valid);
    assert_eq!(parse_rotate("45deg x"), CssTransformLonghandValueKind::Valid);
    assert_eq!(parse_rotate("1 0 0 45deg"), CssTransformLonghandValueKind::Valid);
    assert_eq!(
        parse_rotate("random(1, 0) random(1, 0) 1 90deg"),
        CssTransformLonghandValueKind::Valid
    );
    assert_eq!(
        parse_rotate("x random(90deg, 30deg)"),
        CssTransformLonghandValueKind::Valid
    );
    assert_eq!(parse_transform_origin("left"), CssTransformLonghandValueKind::Valid);
    assert_eq!(
        parse_transform_origin("top center"),
        CssTransformLonghandValueKind::Valid
    );
    assert_eq!(
        parse_transform_origin("center left 6px"),
        CssTransformLonghandValueKind::Valid
    );
    assert_eq!(
        parse_transform_origin("-1px -2px -3px"),
        CssTransformLonghandValueKind::Valid
    );
}

#[test]
fn rejects_invalid_transform_longhand_values() {
    assert_eq!(parse_translate("10px 20% 30%"), CssTransformLonghandValueKind::Invalid);
    assert_eq!(
        parse_translate("10px 20% 30px 40px"),
        CssTransformLonghandValueKind::Invalid
    );
    assert_eq!(parse_scale("1px"), CssTransformLonghandValueKind::Invalid);
    assert_eq!(parse_scale("1 2 3 4"), CssTransformLonghandValueKind::Invalid);
    assert_eq!(parse_rotate("x y 45deg"), CssTransformLonghandValueKind::Invalid);
    assert_eq!(parse_rotate("1 2 45deg"), CssTransformLonghandValueKind::Invalid);
    assert_eq!(
        parse_transform_origin("left right"),
        CssTransformLonghandValueKind::Invalid
    );
    assert_eq!(
        parse_transform_origin("top bottom"),
        CssTransformLonghandValueKind::Invalid
    );
    assert_eq!(
        parse_transform_origin("1px left"),
        CssTransformLonghandValueKind::Invalid
    );
    assert_eq!(
        parse_transform_origin("left 1px 2%"),
        CssTransformLonghandValueKind::Invalid
    );
}

#[test]
fn parses_math_depth_values() {
    assert!(parse_math_depth("auto-add"));
    assert!(parse_math_depth("0"));
    assert!(parse_math_depth("-1"));
    assert!(parse_math_depth("add(2)"));
    assert!(parse_math_depth("add(calc(1 + 2))"));
}

#[test]
fn rejects_invalid_math_depth_values() {
    assert!(!parse_math_depth(""));
    assert!(!parse_math_depth("auto-add 1"));
    assert!(!parse_math_depth("add()"));
    assert!(!parse_math_depth("add(1 2)"));
    assert!(!parse_math_depth("1 2"));
    assert!(!parse_math_depth("1px"));
}

#[test]
fn parses_aspect_ratio_values() {
    assert!(parse_aspect_ratio("auto"));
    assert!(parse_aspect_ratio("1"));
    assert!(parse_aspect_ratio("16 / 9"));
    assert!(parse_aspect_ratio("auto 16 / 9"));
    assert!(parse_aspect_ratio("calc(6em / 1px) / calc(3rem / 1px)"));
}

#[test]
fn rejects_invalid_aspect_ratio_values() {
    assert!(!parse_aspect_ratio(""));
    assert!(!parse_aspect_ratio("auto auto"));
    assert!(!parse_aspect_ratio("16 /"));
    assert!(!parse_aspect_ratio("-1 / 1"));
    assert!(!parse_aspect_ratio("16 / 9 auto auto"));
    assert!(!parse_aspect_ratio("16 auto / 9"));
}

#[test]
fn parses_border_radius_values() {
    assert!(parse_border_radius("0"));
    assert!(parse_border_radius("1px 2%"));
    assert!(parse_border_radius("calc(1px + 2%) / 3px"));
}

#[test]
fn rejects_invalid_border_radius_values() {
    assert!(!parse_border_radius(""));
    assert!(!parse_border_radius("-1px"));
    assert!(!parse_border_radius("1px /"));
    assert!(!parse_border_radius("1px 2px 3px"));
    assert!(!parse_border_radius("1px auto"));
}

#[test]
fn parses_border_radius_shorthand_values() {
    assert!(parse_border_radius_shorthand("0"));
    assert!(parse_border_radius_shorthand("1px 2px 3px 4px"));
    assert!(parse_border_radius_shorthand("1px 2px / 3px 4px"));
    assert!(parse_border_radius_shorthand("calc(1px + 2%) / 3px"));
}

#[test]
fn rejects_invalid_border_radius_shorthand_values() {
    assert!(!parse_border_radius_shorthand(""));
    assert!(!parse_border_radius_shorthand("-1px"));
    assert!(!parse_border_radius_shorthand("1px 2px 3px 4px 5px"));
    assert!(!parse_border_radius_shorthand("1px /"));
    assert!(!parse_border_radius_shorthand("1px / 2px / 3px"));
}

#[test]
fn parses_columns_values() {
    assert!(parse_columns("auto"));
    assert!(parse_columns("12em"));
    assert!(parse_columns("3"));
    assert!(parse_columns("12em 3"));
    assert!(parse_columns("3 12em / auto"));
    assert!(parse_columns("auto / 20px"));
}

#[test]
fn rejects_invalid_columns_values() {
    assert!(!parse_columns(""));
    assert!(!parse_columns("/ auto"));
    assert!(!parse_columns("auto auto auto"));
    assert!(!parse_columns("3 4"));
    assert!(!parse_columns("12em 13em"));
    assert!(!parse_columns("auto /"));
}

#[test]
fn parses_cursor_values() {
    assert!(parse_cursor("auto"));
    assert!(parse_cursor("url(cursor.png), pointer"));
    assert!(parse_cursor("url(cursor.png) 1 -2, grab"));
    assert!(parse_cursor("linear-gradient(black, white), crosshair"));
}

#[test]
fn rejects_invalid_cursor_values() {
    assert!(!parse_cursor(""));
    assert!(!parse_cursor("url(cursor.png)"));
    assert!(!parse_cursor("url(cursor.png) 1, pointer"));
    assert!(!parse_cursor("url(cursor.png) 1 2 3, pointer"));
    assert!(!parse_cursor("not-a-cursor"));
}

#[test]
fn parses_shadow_values() {
    assert!(parse_box_shadow("none"));
    assert!(parse_box_shadow("inset 1px 2px 3px 4px red"));
    assert!(parse_box_shadow("red 1px 2px, 3px 4px blue"));
    assert!(parse_text_shadow("1px 2px 3px red"));
}

#[test]
fn rejects_invalid_shadow_values() {
    assert!(!parse_box_shadow(""));
    assert!(!parse_box_shadow("1px"));
    assert!(!parse_box_shadow("1px 2px -3px"));
    assert!(!parse_box_shadow("inset inset 1px 2px"));
    assert!(!parse_text_shadow("inset 1px 2px"));
    assert!(!parse_text_shadow("1px 2px 3px 4px"));
}

#[test]
fn parses_overflow_clip_margin_values() {
    assert!(parse_overflow_clip_margin("0"));
    assert!(parse_overflow_clip_margin("1px"));
    assert!(parse_overflow_clip_margin("calc(1px + 2px)"));
}

#[test]
fn rejects_invalid_overflow_clip_margin_values() {
    assert!(!parse_overflow_clip_margin(""));
    assert!(!parse_overflow_clip_margin("-1px"));
    assert!(!parse_overflow_clip_margin("1%"));
    assert!(!parse_overflow_clip_margin("content-box"));
    assert!(!parse_overflow_clip_margin("1px 2px"));
}

#[test]
fn parses_shape_outside_values() {
    assert!(parse_shape_outside("none"));
    assert!(parse_shape_outside("url(shape.png)"));
    assert!(parse_shape_outside("linear-gradient(black, white)"));
    assert!(parse_shape_outside("inset(10px)"));
    assert!(parse_shape_outside("circle(closest-side at center) border-box"));
    assert!(parse_shape_outside("margin-box ellipse(10px 20px)"));
}

#[test]
fn rejects_invalid_shape_outside_values() {
    assert!(!parse_shape_outside(""));
    assert!(!parse_shape_outside("auto"));
    assert!(!parse_shape_outside("none border-box"));
    assert!(!parse_shape_outside("border-box margin-box"));
    assert!(!parse_shape_outside("inset(10px) circle(10px)"));
    assert!(!parse_shape_outside("url(shape.png) border-box"));
}

#[test]
fn parses_text_decoration_line_values() {
    assert!(parse_text_decoration_line("none"));
    assert!(parse_text_decoration_line("underline"));
    assert!(parse_text_decoration_line("overline underline blink"));
    assert!(parse_text_decoration_line("underline overline line-through blink"));
    assert!(parse_text_decoration_line("spelling-error"));
    assert!(parse_text_decoration_line("grammar-error"));
}

#[test]
fn rejects_invalid_text_decoration_line_values() {
    assert!(!parse_text_decoration_line(""));
    assert!(!parse_text_decoration_line("auto"));
    assert!(!parse_text_decoration_line("none underline"));
    assert!(!parse_text_decoration_line("underline underline"));
    assert!(!parse_text_decoration_line("spelling-error underline"));
    assert!(!parse_text_decoration_line("spelling-error grammar-error"));
}

#[test]
fn parses_text_decoration_values() {
    assert!(parse_text_decoration("none"));
    assert!(parse_text_decoration("solid"));
    assert!(parse_text_decoration("currentcolor"));
    assert!(parse_text_decoration("auto"));
    assert!(parse_text_decoration("from-font"));
    assert!(parse_text_decoration("10px"));
    assert!(parse_text_decoration("underline overline line-through blink red"));
    assert!(parse_text_decoration("rgba(10, 20, 30, 0.4) dotted"));
    assert!(parse_text_decoration("overline green from-font"));
    assert!(parse_text_decoration("underline dashed green 2px"));
}

#[test]
fn rejects_invalid_text_decoration_values() {
    assert!(!parse_text_decoration(""));
    assert!(!parse_text_decoration("solid double"));
    assert!(!parse_text_decoration("red green"));
    assert!(!parse_text_decoration("auto from-font"));
    assert!(!parse_text_decoration("none underline"));
    assert!(!parse_text_decoration("overline blue underline"));
    assert!(!parse_text_decoration("spelling-error underline"));
}

#[test]
fn parses_list_style_values() {
    assert!(parse_list_style("none"));
    assert!(parse_list_style("inside"));
    assert!(parse_list_style("inside disc"));
    assert!(parse_list_style("inside none"));
    assert!(parse_list_style("none inside none"));
    assert!(parse_list_style("url(\"https://example.com/\")"));
    assert!(parse_list_style("url(\"https://example.com/\") disc outside"));
    assert!(parse_list_style("square linear-gradient(red, blue) inside"));
    assert!(parse_list_style("symbols(cyclic \"*\" \"**\") inside"));
    assert!(parse_list_style("\"marker string\" outside"));
    assert!(parse_list_style("inside outside"));
}

#[test]
fn rejects_invalid_list_style_values() {
    assert!(!parse_list_style(""));
    assert!(!parse_list_style("none none none"));
    assert!(!parse_list_style("disc square"));
    assert!(!parse_list_style("url(marker.png) linear-gradient(red, blue)"));
    assert!(!parse_list_style("none disc none"));
    assert!(!parse_list_style("symbols(numeric \"1\")"));
}

#[test]
fn parses_content_values() {
    assert!(parse_content("none"));
    assert!(parse_content("normal"));
    assert!(parse_content("open-quote"));
    assert!(parse_content("close-quote"));
    assert!(parse_content("no-open-quote"));
    assert!(parse_content("no-close-quote"));
    assert!(parse_content("counter(counter-name)"));
    assert!(parse_content("counter(counter-name, counter-style)"));
    assert!(parse_content("counters(counter-name, \".\")"));
    assert!(parse_content("counters(counter-name, \".\", counter-style)"));
    assert!(parse_content("url(\"picture.svg\")"));
    assert!(parse_content("\"hello\""));
    assert!(parse_content("\"hello\" \"world\""));
    assert!(parse_content("counter(counter-name) \"potato\""));
    assert!(parse_content(
        "\"(\" counters(counter-name, \".\", counter-style) \")\""
    ));
    assert!(parse_content("open-quote \"hello\" \"world\" close-quote"));
    assert!(parse_content("url(\"picture.svg\") \"hello\""));
    assert!(parse_content("open-quote / \"alt text\""));
    assert!(parse_content("counter(counter-name) / \"alt\" counter(other)"));
    assert!(parse_content("attr(foo) attr(bar) attr(baz)"));
    assert!(parse_content("  attr(foo)     attr(bar) attr(baz)  "));
}

#[test]
fn rejects_invalid_content_values() {
    assert!(!parse_content(""));
    assert!(!parse_content("none normal"));
    assert!(!parse_content("normal \"hello\""));
    assert!(!parse_content("/ \"alt\""));
    assert!(!parse_content("\"hello\" /"));
    assert!(!parse_content("\"hello\" / no-open-quote"));
    assert!(!parse_content("\"hello\" / url(\"picture.svg\")"));
    assert!(!parse_content("\"hello\" / \"hi\" no-close-quote"));
    assert!(!parse_content("counters(counter-name)"));
    assert!(!parse_content("counter()"));
    assert!(!parse_content("attr()"));
}

#[test]
fn parses_flex_shorthand_values() {
    assert!(parse_flex_shorthand("none"));
    assert!(parse_flex_shorthand("1"));
    assert!(parse_flex_shorthand("2 3"));
    assert!(parse_flex_shorthand("4 5 6px"));
    assert!(parse_flex_shorthand("6px 4 5"));
    assert!(parse_flex_shorthand("6px 4"));
    assert!(parse_flex_shorthand("6px"));
    assert!(parse_flex_shorthand("7% 8"));
    assert!(parse_flex_shorthand("8 auto"));
    assert!(parse_flex_shorthand("1 1 calc(10em)"));
    assert!(parse_flex_shorthand("calc(-1) calc(-1) 0"));
}

#[test]
fn rejects_invalid_flex_shorthand_values() {
    assert!(!parse_flex_shorthand(""));
    assert!(!parse_flex_shorthand("none 1"));
    assert!(!parse_flex_shorthand("5px 7%"));
    assert!(!parse_flex_shorthand("9 none"));
    assert!(!parse_flex_shorthand("0 1 1"));
    assert!(!parse_flex_shorthand("1 2 3 4"));
}

#[test]
fn parses_flex_flow_values() {
    assert!(parse_flex_flow("column nowrap"));
    assert!(parse_flex_flow("nowrap column"));
    assert!(parse_flex_flow("wrap row-reverse"));
    assert!(parse_flex_flow("nowrap"));
    assert!(parse_flex_flow("row nowrap"));
    assert!(parse_flex_flow("wrap"));
    assert!(parse_flex_flow("row wrap"));
}

#[test]
fn rejects_invalid_flex_flow_values() {
    assert!(!parse_flex_flow(""));
    assert!(!parse_flex_flow("nowrap row nowrap"));
    assert!(!parse_flex_flow("column wrap column"));
    assert!(!parse_flex_flow("row column"));
    assert!(!parse_flex_flow("wrap nowrap"));
}

#[test]
fn parses_filter_value_list_values() {
    assert!(parse_filter_value_list("none"));
    assert!(parse_filter_value_list("url(filters.svg#blur)"));
    assert!(parse_filter_value_list("blur()"));
    assert!(parse_filter_value_list("blur(10px)"));
    assert!(parse_filter_value_list("brightness()"));
    assert!(parse_filter_value_list("brightness(0.5)"));
    assert!(parse_filter_value_list("brightness(sibling-count())"));
    assert!(parse_filter_value_list("contrast(150%)"));
    assert!(parse_filter_value_list("drop-shadow(1px 2px)"));
    assert!(parse_filter_value_list("drop-shadow(red 1px 2px 3px)"));
    assert!(parse_filter_value_list("drop-shadow(1px 2px 3px red)"));
    assert!(parse_filter_value_list("hue-rotate()"));
    assert!(parse_filter_value_list("hue-rotate(0)"));
    assert!(parse_filter_value_list("hue-rotate(90deg)"));
    assert!(parse_filter_value_list("sepia(1) saturate(120%) opacity(0.2)"));
}

#[test]
fn parses_rust_owned_drop_shadow_filter_sources() {
    let Some(RustOwnedFilterValueList::Filters(filters)) =
        parse_rust_owned_filter_value_list_value(b"drop-shadow(1px 2px 3px red)")
    else {
        panic!("Expected filter list");
    };

    let [
        RustOwnedFilterValue::DropShadow {
            color,
            offset_x,
            offset_y,
            radius,
        },
    ] = filters.as_slice()
    else {
        panic!("Expected drop-shadow filter");
    };

    assert_eq!(
        color,
        &Some(RustOwnedColor::Simple {
            kind: CssParsedColorKind::Rgba,
            red: 255,
            green: 0,
            blue: 0,
            alpha: 255,
            name: Some("red".to_string()),
        })
    );
    assert_eq!(
        offset_x,
        &RustOwnedNestedPrimitiveValue::Length {
            value: 1.0,
            unit: "px".to_string(),
        }
    );
    assert_eq!(
        offset_y,
        &RustOwnedNestedPrimitiveValue::Length {
            value: 2.0,
            unit: "px".to_string(),
        }
    );
    assert_eq!(
        radius.as_ref(),
        Some(&RustOwnedNestedPrimitiveValue::Length {
            value: 3.0,
            unit: "px".to_string(),
        })
    );
}

#[test]
fn rejects_invalid_filter_value_list_values() {
    assert!(!parse_filter_value_list(""));
    assert!(!parse_filter_value_list("none blur(1px)"));
    assert!(!parse_filter_value_list("auto"));
    assert!(!parse_filter_value_list("blur(10)"));
    assert!(!parse_filter_value_list("blur(-1px)"));
    assert!(!parse_filter_value_list("brightness(-20)"));
    assert!(!parse_filter_value_list("brightness(30px)"));
    assert!(!parse_filter_value_list("drop-shadow(10 20)"));
    assert!(!parse_filter_value_list("drop-shadow(10% 20%)"));
    assert!(!parse_filter_value_list("drop-shadow(1px)"));
    assert!(!parse_filter_value_list("drop-shadow(1px 2px 3px 4px)"));
    assert!(!parse_filter_value_list("drop-shadow(rgb(4, 5, 6))"));
    assert!(!parse_filter_value_list("drop-shadow()"));
    assert!(!parse_filter_value_list("hue-rotate(90)"));
}

#[test]
fn parses_fit_content_values() {
    assert_eq!(parse_fit_content("fit-content"), CssFitContentValueKind::Valid);
    assert_eq!(parse_fit_content("fit-content(10px)"), CssFitContentValueKind::Valid);
    assert_eq!(
        parse_fit_content("fit-content(calc(10px + 5%))"),
        CssFitContentValueKind::Valid
    );
}

#[test]
fn rejects_invalid_fit_content_values() {
    assert_eq!(parse_fit_content("max-content"), CssFitContentValueKind::Invalid);
    assert_eq!(parse_fit_content("fit-content()"), CssFitContentValueKind::Invalid);
    assert_eq!(
        parse_fit_content("fit-content(10px 20px)"),
        CssFitContentValueKind::Invalid
    );
}

#[test]
fn parses_basic_shape_values() {
    assert_eq!(parse_basic_shape("inset(10px)"), CssBasicShapeValueKind::Valid);
    assert_eq!(
        parse_basic_shape("inset(10px 20% 30px 40% round 1px / 2px)"),
        CssBasicShapeValueKind::Valid
    );
    assert_eq!(parse_basic_shape("xywh(0 0 10px 20%)"), CssBasicShapeValueKind::Valid);
    assert_eq!(
        parse_basic_shape("rect(auto 10px 20% 0 round 1px)"),
        CssBasicShapeValueKind::Valid
    );
    assert_eq!(
        parse_basic_shape("circle(closest-side at center)"),
        CssBasicShapeValueKind::Valid
    );
    assert_eq!(
        parse_basic_shape("ellipse(10px 20% at left top)"),
        CssBasicShapeValueKind::Valid
    );
    assert_eq!(
        parse_basic_shape("polygon(evenodd, 0 0, 100% 0, 100% 100%)"),
        CssBasicShapeValueKind::Valid
    );
    assert_eq!(
        parse_basic_shape("path(evenodd, \"M 0 0 L 1 1\")"),
        CssBasicShapeValueKind::Valid
    );
}

#[test]
fn rejects_invalid_basic_shape_values() {
    assert_eq!(parse_basic_shape("none"), CssBasicShapeValueKind::Invalid);
    assert_eq!(parse_basic_shape("inset()"), CssBasicShapeValueKind::Invalid);
    assert_eq!(parse_basic_shape("xywh(0 0 10px)"), CssBasicShapeValueKind::Invalid);
    assert_eq!(
        parse_basic_shape("rect(auto 10px 20%)"),
        CssBasicShapeValueKind::Invalid
    );
    assert_eq!(parse_basic_shape("circle(10px 20px)"), CssBasicShapeValueKind::Invalid);
    assert_eq!(parse_basic_shape("ellipse(10px)"), CssBasicShapeValueKind::Invalid);
    assert_eq!(parse_basic_shape("polygon(evenodd)"), CssBasicShapeValueKind::Invalid);
    assert_eq!(parse_basic_shape("polygon(0 0, 100%)"), CssBasicShapeValueKind::Invalid);
    assert_eq!(parse_basic_shape("path(nonzero)"), CssBasicShapeValueKind::Invalid);
}

#[test]
fn parses_grid_auto_flow_values() {
    assert_eq!(parse_grid_auto_flow("row"), CssGridAutoFlowValueKind::Valid);
    assert_eq!(parse_grid_auto_flow("column dense"), CssGridAutoFlowValueKind::Valid);
    assert_eq!(parse_grid_auto_flow("dense row"), CssGridAutoFlowValueKind::Valid);
}

#[test]
fn rejects_invalid_grid_auto_flow_values() {
    assert_eq!(parse_grid_auto_flow("row column"), CssGridAutoFlowValueKind::Invalid);
    assert_eq!(parse_grid_auto_flow("dense dense"), CssGridAutoFlowValueKind::Invalid);
    assert_eq!(parse_grid_auto_flow("auto"), CssGridAutoFlowValueKind::Invalid);
}

#[test]
fn parses_grid_track_placement_values() {
    assert_eq!(
        parse_grid_track_placement("auto"),
        CssGridTrackPlacementValueKind::Valid
    );
    assert_eq!(
        parse_grid_track_placement("2 foo"),
        CssGridTrackPlacementValueKind::Valid
    );
    assert_eq!(
        parse_grid_track_placement("foo 2"),
        CssGridTrackPlacementValueKind::Valid
    );
    assert_eq!(
        parse_grid_track_placement("span 2 foo"),
        CssGridTrackPlacementValueKind::Valid
    );
    assert_eq!(
        parse_grid_track_placement("span sibling-count()"),
        CssGridTrackPlacementValueKind::Valid
    );
}

#[test]
fn rejects_invalid_grid_track_placement_values() {
    assert_eq!(parse_grid_track_placement("0"), CssGridTrackPlacementValueKind::Invalid);
    assert_eq!(
        parse_grid_track_placement("span -1"),
        CssGridTrackPlacementValueKind::Invalid
    );
    assert_eq!(
        parse_grid_track_placement("auto foo"),
        CssGridTrackPlacementValueKind::Invalid
    );
    assert_eq!(
        parse_grid_track_placement("foo span 2"),
        CssGridTrackPlacementValueKind::Invalid
    );
}

#[test]
fn parses_grid_placement_shorthands() {
    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridColumn, "main"),
        Some(vec![
            (PropertyId::GridColumnStart, "main".to_string()),
            (PropertyId::GridColumnEnd, "main".to_string()),
        ])
    );
    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridRow, "2 / span 3"),
        Some(vec![
            (PropertyId::GridRowStart, "2".to_string()),
            (PropertyId::GridRowEnd, "span 3".to_string()),
        ])
    );
    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridRow, "2"),
        Some(vec![
            (PropertyId::GridRowStart, "2".to_string()),
            (PropertyId::GridRowEnd, "auto".to_string()),
        ])
    );
    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridArea, "sidebar"),
        Some(vec![
            (PropertyId::GridRowStart, "sidebar".to_string()),
            (PropertyId::GridColumnStart, "sidebar".to_string()),
            (PropertyId::GridRowEnd, "sidebar".to_string()),
            (PropertyId::GridColumnEnd, "sidebar".to_string()),
        ])
    );
    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridArea, "1 / main / span 2 / auto"),
        Some(vec![
            (PropertyId::GridRowStart, "1".to_string()),
            (PropertyId::GridColumnStart, "main".to_string()),
            (PropertyId::GridRowEnd, "span 2".to_string()),
            (PropertyId::GridColumnEnd, "auto".to_string()),
        ])
    );

    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridColumn, "1 /"),
        None
    );
    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridColumn, "1 / 2 / 3"),
        None
    );
    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridArea, "/ main"),
        None
    );
    assert_eq!(
        parse_grid_placement_shorthand_items(PropertyId::GridArea, "1 / 2 / 3 / 4 / 5"),
        None
    );
}

#[test]
fn parses_grid_template_shorthands() {
    assert_eq!(
        parse_grid_template_shorthand_items(PropertyId::GridTemplate, "none"),
        Some(vec![])
    );
    assert_eq!(
        parse_grid_template_shorthand_items(PropertyId::GridTemplate, "1fr / [main] 10px"),
        Some(vec![
            (PropertyId::GridTemplateRows, "1fr".to_string()),
            (PropertyId::GridTemplateColumns, "[main] 10px".to_string()),
        ])
    );
    assert_eq!(
        parse_grid_template_shorthand_items(PropertyId::GridTemplate, "\"a a\" 10px [b] \"c c\" / 1fr 2fr"),
        Some(vec![
            (PropertyId::GridTemplateAreas, "\"a a\" \"c c\"".to_string()),
            (PropertyId::GridTemplateRows, "10px [b] auto".to_string()),
            (PropertyId::GridTemplateColumns, "1fr 2fr".to_string()),
        ])
    );
    assert_eq!(
        parse_grid_template_shorthand_items(
            PropertyId::GridTemplate,
            "[header-left] \"head head\" 30px [header-right] [main-left] \"nav main\" 1fr [main-right] [footer-left] \"nav foot\" 30px [footer-right] / 120px 1fr"
        ),
        Some(vec![
            (
                PropertyId::GridTemplateAreas,
                "\"head head\" \"nav main\" \"nav foot\"".to_string()
            ),
            (
                PropertyId::GridTemplateRows,
                "[header-left] 30px [header-right main-left] 1fr [main-right footer-left] 30px [footer-right]"
                    .to_string(),
            ),
            (PropertyId::GridTemplateColumns, "120px 1fr".to_string()),
        ])
    );
    assert_eq!(
        parse_grid_template_shorthand_items(PropertyId::Grid, "auto-flow dense 20px / 1fr 2fr"),
        Some(vec![
            (PropertyId::GridAutoFlow, "row dense".to_string()),
            (PropertyId::GridTemplateColumns, "1fr 2fr".to_string()),
            (PropertyId::GridAutoRows, "20px".to_string()),
        ])
    );
    assert_eq!(
        parse_grid_template_shorthand_items(PropertyId::Grid, "1fr 2fr / dense auto-flow 10px"),
        Some(vec![
            (PropertyId::GridTemplateRows, "1fr 2fr".to_string()),
            (PropertyId::GridAutoFlow, "column dense".to_string()),
            (PropertyId::GridAutoColumns, "10px".to_string()),
        ])
    );

    assert_eq!(
        parse_grid_template_shorthand_items(PropertyId::GridTemplate, "1fr /"),
        None
    );
    assert_eq!(
        parse_grid_template_shorthand_items(PropertyId::GridTemplate, "\"a\" \"b b\""),
        None
    );
    assert_eq!(
        parse_grid_template_shorthand_items(PropertyId::Grid, "dense / 1fr"),
        None
    );
}

#[test]
fn parses_grid_track_size_list_values() {
    assert_eq!(
        parse_grid_auto_track_sizes("10px minmax(1px, 1fr) fit-content(50%)"),
        CssGridTrackSizeListValueKind::Valid
    );
    assert_eq!(parse_grid_track_size_list("none"), CssGridTrackSizeListValueKind::Valid);
    assert_eq!(
        parse_grid_track_size_list("[a] 10px [b] repeat(2, [c] minmax(auto, 1fr))"),
        CssGridTrackSizeListValueKind::Valid
    );
    assert_eq!(
        parse_grid_track_size_list("10px repeat(auto-fit, minmax(10px, 1fr)) 20px"),
        CssGridTrackSizeListValueKind::Valid
    );
    assert_eq!(
        parse_grid_track_size_list("[start] repeat(auto-fill, minmax(100px, 1fr)) [end]"),
        CssGridTrackSizeListValueKind::Valid
    );
}

#[test]
fn rejects_invalid_grid_track_size_list_values() {
    assert_eq!(
        parse_grid_auto_track_sizes("repeat(auto-fit, 10px)"),
        CssGridTrackSizeListValueKind::Invalid
    );
    assert_eq!(
        parse_grid_track_size_list("repeat(auto-fit, 1fr)"),
        CssGridTrackSizeListValueKind::Invalid
    );
    assert_eq!(
        parse_grid_track_size_list("repeat(0, 10px)"),
        CssGridTrackSizeListValueKind::Invalid
    );
    assert_eq!(
        parse_grid_track_size_list("minmax(1fr, 10px)"),
        CssGridTrackSizeListValueKind::Invalid
    );
    assert_eq!(
        parse_grid_track_size_list("[one]"),
        CssGridTrackSizeListValueKind::Invalid
    );
    assert_eq!(
        parse_grid_track_size_list("[one] 10px [two] [three]"),
        CssGridTrackSizeListValueKind::Invalid
    );
    assert_eq!(
        parse_grid_track_size_list("[span] 10px"),
        CssGridTrackSizeListValueKind::Invalid
    );
}

#[test]
fn parses_paint_order_values() {
    assert_eq!(
        parse_paint_order("normal"),
        CssPaintOrderValue {
            kind: CssPaintOrderValueKind::Normal,
            first: CssPaintOrderKeyword::Invalid,
            second: CssPaintOrderKeyword::Invalid,
        }
    );
    assert_eq!(
        parse_paint_order("fill markers stroke"),
        CssPaintOrderValue {
            kind: CssPaintOrderValueKind::Pair,
            first: CssPaintOrderKeyword::Fill,
            second: CssPaintOrderKeyword::Markers,
        }
    );
    assert_eq!(
        parse_paint_order("markers fill stroke"),
        CssPaintOrderValue {
            kind: CssPaintOrderValueKind::Keyword,
            first: CssPaintOrderKeyword::Markers,
            second: CssPaintOrderKeyword::Invalid,
        }
    );
}

#[test]
fn rejects_invalid_paint_order_values() {
    assert_eq!(parse_paint_order("").kind, CssPaintOrderValueKind::Invalid);
    assert_eq!(parse_paint_order("normal stroke").kind, CssPaintOrderValueKind::Invalid);
    assert_eq!(parse_paint_order("fill fill").kind, CssPaintOrderValueKind::Invalid);
    assert_eq!(
        parse_paint_order("markers normal").kind,
        CssPaintOrderValueKind::Invalid
    );
    assert_eq!(parse_paint_order("fill, stroke").kind, CssPaintOrderValueKind::Invalid);
}

#[test]
fn parses_place_shorthand_values() {
    assert!(parse_place_content("center"));
    assert!(parse_place_content("space-between center"));
    assert!(parse_place_content("first baseline safe right"));
    assert!(parse_place_content("stretch"));
    assert!(parse_place_items("normal"));
    assert!(parse_place_items("stretch"));
    assert!(parse_place_items("start"));
    assert!(parse_place_items("normal start"));
    assert!(parse_place_items("first baseline legacy right"));
    assert!(parse_place_self("auto"));
    assert!(parse_place_self("center"));
    assert!(parse_place_self("safe end unsafe right"));
}

#[test]
fn rejects_invalid_place_shorthand_values() {
    assert!(!parse_place_content(""));
    assert!(!parse_place_content("left"));
    assert!(!parse_place_content("center center center"));
    assert!(!parse_place_items("auto"));
    assert!(!parse_place_items("safe"));
    assert!(!parse_place_items("legacy safe center"));
    assert!(!parse_place_self("legacy"));
    assert!(!parse_place_self("safe"));
    assert!(!parse_place_self("left center right"));
}

#[test]
fn parses_text_underline_position_values() {
    assert_eq!(
        parse_text_underline_position("auto"),
        CssTextUnderlinePositionValue {
            horizontal: CssTextUnderlinePositionHorizontal::Auto,
            vertical: CssTextUnderlinePositionVertical::Auto,
        }
    );
    assert_eq!(
        parse_text_underline_position("under"),
        CssTextUnderlinePositionValue {
            horizontal: CssTextUnderlinePositionHorizontal::Under,
            vertical: CssTextUnderlinePositionVertical::Auto,
        }
    );
    assert_eq!(
        parse_text_underline_position("right from-font"),
        CssTextUnderlinePositionValue {
            horizontal: CssTextUnderlinePositionHorizontal::FromFont,
            vertical: CssTextUnderlinePositionVertical::Right,
        }
    );
}

#[test]
fn rejects_invalid_text_underline_position_values() {
    assert_eq!(
        parse_text_underline_position("").horizontal,
        CssTextUnderlinePositionHorizontal::Invalid
    );
    assert_eq!(
        parse_text_underline_position("auto under").horizontal,
        CssTextUnderlinePositionHorizontal::Invalid
    );
    assert_eq!(
        parse_text_underline_position("left right").horizontal,
        CssTextUnderlinePositionHorizontal::Invalid
    );
    assert_eq!(
        parse_text_underline_position("under from-font").horizontal,
        CssTextUnderlinePositionHorizontal::Invalid
    );
    assert_eq!(
        parse_text_underline_position("under, left").horizontal,
        CssTextUnderlinePositionHorizontal::Invalid
    );
}

#[test]
fn parses_text_wrap_values() {
    assert_eq!(
        parse_text_wrap("wrap"),
        CssTextWrapValue {
            kind: CssTextWrapValueKind::Valid,
            mode: CssTextWrapModeValue::Wrap,
            style: CssTextWrapStyleValue::Invalid,
        }
    );
    assert_eq!(
        parse_text_wrap("pretty nowrap"),
        CssTextWrapValue {
            kind: CssTextWrapValueKind::Valid,
            mode: CssTextWrapModeValue::Nowrap,
            style: CssTextWrapStyleValue::Pretty,
        }
    );
    assert_eq!(
        parse_text_wrap("stable"),
        CssTextWrapValue {
            kind: CssTextWrapValueKind::Valid,
            mode: CssTextWrapModeValue::Invalid,
            style: CssTextWrapStyleValue::Stable,
        }
    );
}

#[test]
fn rejects_invalid_text_wrap_values() {
    assert_eq!(parse_text_wrap("").kind, CssTextWrapValueKind::Invalid);
    assert_eq!(parse_text_wrap("wrap nowrap").kind, CssTextWrapValueKind::Invalid);
    assert_eq!(parse_text_wrap("pretty balance").kind, CssTextWrapValueKind::Invalid);
    assert_eq!(parse_text_wrap("wrap, pretty").kind, CssTextWrapValueKind::Invalid);
    assert_eq!(parse_text_wrap("avoid-orphans").kind, CssTextWrapValueKind::Invalid);
    assert_eq!(parse_text_wrap("10px").kind, CssTextWrapValueKind::Invalid);
}

#[test]
fn parses_text_wrap_mode_values() {
    assert_eq!(parse_text_wrap_mode("wrap"), CssTextWrapModeValue::Wrap);
    assert_eq!(parse_text_wrap_mode("nowrap"), CssTextWrapModeValue::Nowrap);
}

#[test]
fn rejects_invalid_text_wrap_mode_values() {
    assert_eq!(parse_text_wrap_mode(""), CssTextWrapModeValue::Invalid);
    assert_eq!(parse_text_wrap_mode("auto"), CssTextWrapModeValue::Invalid);
    assert_eq!(parse_text_wrap_mode("wrap nowrap"), CssTextWrapModeValue::Invalid);
    assert_eq!(parse_text_wrap_mode("wrap, nowrap"), CssTextWrapModeValue::Invalid);
    assert_eq!(parse_text_wrap_mode("10px"), CssTextWrapModeValue::Invalid);
}

#[test]
fn parses_text_wrap_style_values() {
    assert_eq!(parse_text_wrap_style("auto"), CssTextWrapStyleValue::Auto);
    assert_eq!(parse_text_wrap_style("balance"), CssTextWrapStyleValue::Balance);
    assert_eq!(parse_text_wrap_style("stable"), CssTextWrapStyleValue::Stable);
    assert_eq!(parse_text_wrap_style("pretty"), CssTextWrapStyleValue::Pretty);
}

#[test]
fn rejects_invalid_text_wrap_style_values() {
    assert_eq!(parse_text_wrap_style(""), CssTextWrapStyleValue::Invalid);
    assert_eq!(parse_text_wrap_style("wrap"), CssTextWrapStyleValue::Invalid);
    assert_eq!(parse_text_wrap_style("pretty balance"), CssTextWrapStyleValue::Invalid);
    assert_eq!(parse_text_wrap_style("balance, stable"), CssTextWrapStyleValue::Invalid);
    assert_eq!(parse_text_wrap_style("avoid-orphans"), CssTextWrapStyleValue::Invalid);
    assert_eq!(parse_text_wrap_style("10px"), CssTextWrapStyleValue::Invalid);
}

#[test]
fn parses_touch_action_values() {
    assert_eq!(parse_touch_action("auto").kind, CssTouchActionValueKind::Auto);
    assert_eq!(parse_touch_action("none").kind, CssTouchActionValueKind::None);
    assert_eq!(
        parse_touch_action("manipulation").kind,
        CssTouchActionValueKind::Manipulation
    );
    assert_eq!(
        parse_touch_action("pan-y pan-x"),
        CssTouchActionValue {
            kind: CssTouchActionValueKind::List,
            first: CssTouchActionKeyword::PanX,
            second: CssTouchActionKeyword::PanY,
        }
    );
    assert_eq!(
        parse_touch_action("pan-left pan-down"),
        CssTouchActionValue {
            kind: CssTouchActionValueKind::List,
            first: CssTouchActionKeyword::PanLeft,
            second: CssTouchActionKeyword::PanDown,
        }
    );
}

#[test]
fn rejects_invalid_touch_action_values() {
    assert_eq!(parse_touch_action("").kind, CssTouchActionValueKind::Invalid);
    assert_eq!(parse_touch_action("auto none").kind, CssTouchActionValueKind::Invalid);
    assert_eq!(
        parse_touch_action("manipulation pan-x").kind,
        CssTouchActionValueKind::Invalid
    );
    assert_eq!(
        parse_touch_action("pan-y pan-x pan-y").kind,
        CssTouchActionValueKind::Invalid
    );
    assert_eq!(
        parse_touch_action("pan-x, pan-y").kind,
        CssTouchActionValueKind::Invalid
    );
}

#[test]
fn parses_scrollbar_gutter_values() {
    assert_eq!(parse_scrollbar_gutter("auto"), CssScrollbarGutterValueKind::Auto);
    assert_eq!(parse_scrollbar_gutter("stable"), CssScrollbarGutterValueKind::Stable);
    assert_eq!(
        parse_scrollbar_gutter("stable both-edges"),
        CssScrollbarGutterValueKind::BothEdges
    );
    assert_eq!(
        parse_scrollbar_gutter("both-edges stable"),
        CssScrollbarGutterValueKind::BothEdges
    );
}

#[test]
fn rejects_invalid_scrollbar_gutter_values() {
    assert_eq!(parse_scrollbar_gutter(""), CssScrollbarGutterValueKind::Invalid);
    assert_eq!(
        parse_scrollbar_gutter("both-edges"),
        CssScrollbarGutterValueKind::Invalid
    );
    assert_eq!(
        parse_scrollbar_gutter("auto stable"),
        CssScrollbarGutterValueKind::Invalid
    );
    assert_eq!(
        parse_scrollbar_gutter("stable both-edges both-edges"),
        CssScrollbarGutterValueKind::Invalid
    );
    assert_eq!(
        parse_scrollbar_gutter("stable, both-edges"),
        CssScrollbarGutterValueKind::Invalid
    );
}

#[test]
fn parses_stroke_dasharray_values() {
    assert!(parse_stroke_dasharray("none"));
    assert!(parse_stroke_dasharray("2 3px, 4%"));
    assert!(parse_stroke_dasharray("calc(4)"));
    assert!(parse_stroke_dasharray("calc(4%) 2"));
}

#[test]
fn rejects_invalid_stroke_dasharray_values() {
    assert!(!parse_stroke_dasharray(""));
    assert!(!parse_stroke_dasharray("auto"));
    assert!(!parse_stroke_dasharray("none 10px"));
    assert!(!parse_stroke_dasharray("-40px"));
    assert!(!parse_stroke_dasharray("20px / 30px"));
    assert!(!parse_stroke_dasharray("2,"));
}

#[test]
fn parses_quotes_values() {
    assert_eq!(parse_quotes("auto"), (CssQuotesValueKind::Auto, vec![]));
    assert_eq!(parse_quotes("none"), (CssQuotesValueKind::None, vec![]));
    assert_eq!(
        parse_quotes("\"[\" \"]\" \"(\" \")\""),
        (
            CssQuotesValueKind::List,
            vec!["[".to_string(), "]".to_string(), "(".to_string(), ")".to_string()]
        )
    );
}

#[test]
fn rejects_invalid_quotes_values() {
    assert_eq!(parse_quotes("").0, CssQuotesValueKind::Invalid);
    assert_eq!(parse_quotes("auto none").0, CssQuotesValueKind::Invalid);
    assert_eq!(parse_quotes("\"[\"").0, CssQuotesValueKind::Invalid);
    assert_eq!(parse_quotes("\"[\" \"]\" \"(\"").0, CssQuotesValueKind::Invalid);
    assert_eq!(parse_quotes("open close").0, CssQuotesValueKind::Invalid);
    assert_eq!(parse_quotes("\"[\", \"]\"").0, CssQuotesValueKind::Invalid);
}

#[test]
fn parses_will_change_values() {
    assert_eq!(parse_will_change("auto"), (CssWillChangeValueKind::Auto, vec![]));
    assert_eq!(
        parse_will_change("scroll-position, contents, transform"),
        (
            CssWillChangeValueKind::List,
            vec![
                (CssWillChangeFeatureKind::ScrollPosition, String::new()),
                (CssWillChangeFeatureKind::Contents, String::new()),
                (CssWillChangeFeatureKind::CustomIdent, "transform".to_string())
            ]
        )
    );
    assert_eq!(
        parse_will_change("Not-A-Property, --var"),
        (
            CssWillChangeValueKind::List,
            vec![
                (CssWillChangeFeatureKind::CustomIdent, "Not-A-Property".to_string()),
                (CssWillChangeFeatureKind::CustomIdent, "--var".to_string())
            ]
        )
    );
}

#[test]
fn rejects_invalid_will_change_values() {
    assert_eq!(parse_will_change("").0, CssWillChangeValueKind::Invalid);
    assert_eq!(parse_will_change("auto, transform").0, CssWillChangeValueKind::Invalid);
    assert_eq!(parse_will_change("contents auto").0, CssWillChangeValueKind::Invalid);
    assert_eq!(parse_will_change("none").0, CssWillChangeValueKind::Invalid);
    assert_eq!(parse_will_change("all").0, CssWillChangeValueKind::Invalid);
    assert_eq!(parse_will_change("will-change").0, CssWillChangeValueKind::Invalid);
    assert_eq!(parse_will_change("transform,").0, CssWillChangeValueKind::Invalid);
}

#[test]
fn parses_transition_property_values() {
    assert_eq!(
        parse_transition_property("none"),
        (CssTransitionPropertyValueKind::None, vec![])
    );
    assert_eq!(
        parse_transition_property("all"),
        (CssTransitionPropertyValueKind::List, vec!["all".to_string()])
    );
    assert_eq!(
        parse_transition_property("width, ALL, opacity"),
        (
            CssTransitionPropertyValueKind::List,
            vec!["width".to_string(), "ALL".to_string(), "opacity".to_string()]
        )
    );
    assert_eq!(
        parse_transition_property("one-two-three, --custom"),
        (
            CssTransitionPropertyValueKind::List,
            vec!["one-two-three".to_string(), "--custom".to_string()]
        )
    );
}

#[test]
fn rejects_invalid_transition_property_values() {
    assert_eq!(parse_transition_property("").0, CssTransitionPropertyValueKind::Invalid);
    assert_eq!(
        parse_transition_property("none, opacity").0,
        CssTransitionPropertyValueKind::Invalid
    );
    assert_eq!(
        parse_transition_property("width opacity").0,
        CssTransitionPropertyValueKind::Invalid
    );
    assert_eq!(
        parse_transition_property("initial").0,
        CssTransitionPropertyValueKind::Invalid
    );
    assert_eq!(
        parse_transition_property("width,").0,
        CssTransitionPropertyValueKind::Invalid
    );
}

#[test]
fn parses_transition_behavior_values() {
    assert_eq!(
        parse_transition_behavior("normal"),
        (
            CssTransitionBehaviorValueKind::List,
            vec![CssTransitionBehaviorItemKind::Normal]
        )
    );
    assert_eq!(
        parse_transition_behavior("allow-discrete"),
        (
            CssTransitionBehaviorValueKind::List,
            vec![CssTransitionBehaviorItemKind::AllowDiscrete]
        )
    );
    assert_eq!(
        parse_transition_behavior("normal, allow-discrete"),
        (
            CssTransitionBehaviorValueKind::List,
            vec![
                CssTransitionBehaviorItemKind::Normal,
                CssTransitionBehaviorItemKind::AllowDiscrete
            ]
        )
    );
}

#[test]
fn rejects_invalid_transition_behavior_values() {
    assert_eq!(parse_transition_behavior("").0, CssTransitionBehaviorValueKind::Invalid);
    assert_eq!(
        parse_transition_behavior("auto").0,
        CssTransitionBehaviorValueKind::Invalid
    );
    assert_eq!(
        parse_transition_behavior("normal allow-discrete").0,
        CssTransitionBehaviorValueKind::Invalid
    );
    assert_eq!(
        parse_transition_behavior("normal,").0,
        CssTransitionBehaviorValueKind::Invalid
    );
    assert_eq!(
        parse_transition_behavior("10px").0,
        CssTransitionBehaviorValueKind::Invalid
    );
}

#[test]
fn parses_animation_name_values() {
    assert_eq!(
        parse_animation_name("none"),
        (
            CssAnimationNameValueKind::List,
            vec![(CssAnimationNameItemKind::None, String::new())]
        )
    );
    assert_eq!(
        parse_animation_name("foo, \"none\", Both"),
        (
            CssAnimationNameValueKind::List,
            vec![
                (CssAnimationNameItemKind::CustomIdent, "foo".to_string()),
                (CssAnimationNameItemKind::String, "none".to_string()),
                (CssAnimationNameItemKind::CustomIdent, "Both".to_string())
            ]
        )
    );
    assert_eq!(
        parse_animation_name("\"multi word string\""),
        (
            CssAnimationNameValueKind::List,
            vec![(CssAnimationNameItemKind::String, "multi word string".to_string())]
        )
    );
}

#[test]
fn rejects_invalid_animation_name_values() {
    assert_eq!(parse_animation_name("").0, CssAnimationNameValueKind::Invalid);
    assert_eq!(parse_animation_name("12").0, CssAnimationNameValueKind::Invalid);
    assert_eq!(parse_animation_name("one two").0, CssAnimationNameValueKind::Invalid);
    assert_eq!(
        parse_animation_name("one, initial").0,
        CssAnimationNameValueKind::Invalid
    );
    assert_eq!(
        parse_animation_name("default, two").0,
        CssAnimationNameValueKind::Invalid
    );
    assert_eq!(parse_animation_name("\"\"").0, CssAnimationNameValueKind::Invalid);
    assert_eq!(parse_animation_name("one,").0, CssAnimationNameValueKind::Invalid);
}

#[test]
fn parses_view_transition_name_values() {
    assert_eq!(
        parse_view_transition_name("none"),
        (CssViewTransitionNameValueKind::None, None)
    );
    assert_eq!(
        parse_view_transition_name("foo"),
        (CssViewTransitionNameValueKind::CustomIdent, Some("foo".to_string()))
    );
    assert_eq!(
        parse_view_transition_name("match-element"),
        (
            CssViewTransitionNameValueKind::CustomIdent,
            Some("match-element".to_string())
        )
    );
    assert_eq!(
        parse_view_transition_name("maTch-element"),
        (
            CssViewTransitionNameValueKind::CustomIdent,
            Some("maTch-element".to_string())
        )
    );
}

#[test]
fn rejects_invalid_view_transition_name_values() {
    assert_eq!(
        parse_view_transition_name("").0,
        CssViewTransitionNameValueKind::Invalid
    );
    assert_eq!(
        parse_view_transition_name("auto").0,
        CssViewTransitionNameValueKind::Invalid
    );
    assert_eq!(
        parse_view_transition_name("default").0,
        CssViewTransitionNameValueKind::Invalid
    );
    assert_eq!(
        parse_view_transition_name("inherit").0,
        CssViewTransitionNameValueKind::Invalid
    );
    assert_eq!(
        parse_view_transition_name("foo foo").0,
        CssViewTransitionNameValueKind::Invalid
    );
    assert_eq!(
        parse_view_transition_name("\"foo\"").0,
        CssViewTransitionNameValueKind::Invalid
    );
    assert_eq!(
        parse_view_transition_name("12px").0,
        CssViewTransitionNameValueKind::Invalid
    );
}

#[test]
fn parses_display_values() {
    assert_eq!(
        parse_display_value("none".as_bytes()),
        Some(RustOwnedDisplay {
            kind: CssDisplayValueKind::Box,
            box_: CssDisplayBox::None,
            internal: CssDisplayInternal::TableRowGroup,
            outside: CssDisplayOutside::Block,
            inside: CssDisplayInside::Flow,
            list_item: CssDisplayListItem::No,
        })
    );
    assert_eq!(
        parse_display_value("inline-flex".as_bytes()),
        Some(RustOwnedDisplay {
            kind: CssDisplayValueKind::OutsideAndInside,
            box_: CssDisplayBox::Contents,
            internal: CssDisplayInternal::TableRowGroup,
            outside: CssDisplayOutside::Inline,
            inside: CssDisplayInside::Flex,
            list_item: CssDisplayListItem::No,
        })
    );
    assert_eq!(
        parse_display_value("inline flow-root list-item".as_bytes()),
        Some(RustOwnedDisplay {
            kind: CssDisplayValueKind::OutsideAndInside,
            box_: CssDisplayBox::Contents,
            internal: CssDisplayInternal::TableRowGroup,
            outside: CssDisplayOutside::Inline,
            inside: CssDisplayInside::FlowRoot,
            list_item: CssDisplayListItem::Yes,
        })
    );
    assert_eq!(
        parse_display_value("table-row".as_bytes()),
        Some(RustOwnedDisplay {
            kind: CssDisplayValueKind::Internal,
            box_: CssDisplayBox::Contents,
            internal: CssDisplayInternal::TableRow,
            outside: CssDisplayOutside::Block,
            inside: CssDisplayInside::Flow,
            list_item: CssDisplayListItem::No,
        })
    );
}

#[test]
fn rejects_invalid_display_values() {
    assert_eq!(parse_display_value("".as_bytes()), None);
    assert_eq!(parse_display_value("inline block".as_bytes()), None);
    assert_eq!(parse_display_value("flex list-item".as_bytes()), None);
    assert_eq!(parse_display_value("contents list-item".as_bytes()), None);
    assert_eq!(parse_display_value("block, flow".as_bytes()), None);
}

#[test]
fn parses_color_scheme_values() {
    assert_eq!(
        parse_color_scheme("normal"),
        (CssColorSchemeValueKind::Normal, false, vec![])
    );
    assert_eq!(
        parse_color_scheme("light"),
        (CssColorSchemeValueKind::List, false, vec!["light".to_string()])
    );
    assert_eq!(
        parse_color_scheme("dark light"),
        (
            CssColorSchemeValueKind::List,
            false,
            vec!["dark".to_string(), "light".to_string()]
        )
    );
    assert_eq!(
        parse_color_scheme("only dark"),
        (CssColorSchemeValueKind::List, true, vec!["dark".to_string()])
    );
    assert_eq!(
        parse_color_scheme("dark only"),
        (CssColorSchemeValueKind::List, true, vec!["dark".to_string()])
    );
    assert_eq!(
        parse_color_scheme("sepia dark"),
        (
            CssColorSchemeValueKind::List,
            false,
            vec!["sepia".to_string(), "dark".to_string()]
        )
    );
}

#[test]
fn rejects_invalid_color_scheme_values() {
    assert_eq!(parse_color_scheme("").0, CssColorSchemeValueKind::Invalid);
    assert_eq!(parse_color_scheme("normal light").0, CssColorSchemeValueKind::Invalid);
    assert_eq!(parse_color_scheme("only").0, CssColorSchemeValueKind::Invalid);
    assert_eq!(parse_color_scheme("only dark only").0, CssColorSchemeValueKind::Invalid);
    assert_eq!(
        parse_color_scheme("dark only light").0,
        CssColorSchemeValueKind::Invalid
    );
    assert_eq!(parse_color_scheme("default").0, CssColorSchemeValueKind::Invalid);
    assert_eq!(parse_color_scheme("inherit").0, CssColorSchemeValueKind::Invalid);
    assert_eq!(parse_color_scheme("dark, light").0, CssColorSchemeValueKind::Invalid);
}

#[test]
fn parses_anchor_name_and_scope_values() {
    assert_eq!(
        parse_anchor_name_or_scope("none", false),
        (CssAnchorNameOrScopeValueKind::None, vec![])
    );
    assert_eq!(
        parse_anchor_name_or_scope("--foo", false),
        (CssAnchorNameOrScopeValueKind::List, vec!["--foo".to_string()])
    );
    assert_eq!(
        parse_anchor_name_or_scope("--foo, --bar", false),
        (
            CssAnchorNameOrScopeValueKind::List,
            vec!["--foo".to_string(), "--bar".to_string()]
        )
    );
    assert_eq!(
        parse_anchor_name_or_scope("all", true),
        (CssAnchorNameOrScopeValueKind::All, vec![])
    );
}

#[test]
fn rejects_invalid_anchor_name_and_scope_values() {
    assert_eq!(
        parse_anchor_name_or_scope("", false).0,
        CssAnchorNameOrScopeValueKind::Invalid
    );
    assert_eq!(
        parse_anchor_name_or_scope("all", false).0,
        CssAnchorNameOrScopeValueKind::Invalid
    );
    assert_eq!(
        parse_anchor_name_or_scope("none, --foo", false).0,
        CssAnchorNameOrScopeValueKind::Invalid
    );
    assert_eq!(
        parse_anchor_name_or_scope("--foo,", false).0,
        CssAnchorNameOrScopeValueKind::Invalid
    );
    assert_eq!(
        parse_anchor_name_or_scope("--foo --bar", false).0,
        CssAnchorNameOrScopeValueKind::Invalid
    );
    assert_eq!(
        parse_anchor_name_or_scope("foo", false).0,
        CssAnchorNameOrScopeValueKind::Invalid
    );
}

#[test]
fn parses_position_anchor_values() {
    assert_eq!(
        parse_position_anchor("normal"),
        (CssPositionAnchorValueKind::Normal, None)
    );
    assert_eq!(parse_position_anchor("none"), (CssPositionAnchorValueKind::None, None));
    assert_eq!(parse_position_anchor("auto"), (CssPositionAnchorValueKind::Auto, None));
    assert_eq!(
        parse_position_anchor("--foo"),
        (CssPositionAnchorValueKind::AnchorName, Some("--foo".to_string()))
    );
}

#[test]
fn rejects_invalid_position_anchor_values() {
    assert_eq!(parse_position_anchor("").0, CssPositionAnchorValueKind::Invalid);
    assert_eq!(
        parse_position_anchor("normal --foo").0,
        CssPositionAnchorValueKind::Invalid
    );
    assert_eq!(
        parse_position_anchor("--foo, --bar").0,
        CssPositionAnchorValueKind::Invalid
    );
    assert_eq!(parse_position_anchor("foo").0, CssPositionAnchorValueKind::Invalid);
}

#[test]
fn parses_position_area_values() {
    assert!(parse_position_area("none"));
    assert!(parse_position_area("center"));
    assert!(parse_position_area("span-all span-all"));
    assert!(parse_position_area("left top"));
    assert!(parse_position_area("top left"));
    assert!(parse_position_area("block-start inline-end"));
    assert!(parse_position_area("self-inline-end self-block-start"));
    assert!(parse_position_area("span-start center"));
}

#[test]
fn rejects_invalid_position_area_values() {
    assert!(!parse_position_area(""));
    assert!(!parse_position_area("none none"));
    assert!(!parse_position_area("start none"));
    assert!(!parse_position_area("top left top"));
    assert!(!parse_position_area("foobar"));
    assert!(!parse_position_area("left right"));
    assert!(!parse_position_area("block-start block-end"));
}

#[test]
fn parses_position_try_fallbacks_values() {
    assert!(parse_position_try_fallbacks("none"));
    assert!(parse_position_try_fallbacks("--foo"));
    assert!(parse_position_try_fallbacks("flip-block"));
    assert!(parse_position_try_fallbacks("--foo flip-block flip-inline"));
    assert!(parse_position_try_fallbacks("top left"));
    assert!(parse_position_try_fallbacks(
        "--foo, flip-start, block-start inline-end"
    ));
}

#[test]
fn rejects_invalid_position_try_fallbacks_values() {
    assert!(!parse_position_try_fallbacks(""));
    assert!(!parse_position_try_fallbacks("none, --foo"));
    assert!(!parse_position_try_fallbacks("--foo --bar"));
    assert!(!parse_position_try_fallbacks("flip-block flip-block"));
    assert!(!parse_position_try_fallbacks("start none"));
    assert!(!parse_position_try_fallbacks("--foo,"));
}

#[test]
fn parses_timeline_scope_values() {
    assert_eq!(parse_timeline_scope("none"), (CssTimelineScopeValueKind::None, vec![]));
    assert_eq!(parse_timeline_scope("all"), (CssTimelineScopeValueKind::All, vec![]));
    assert_eq!(
        parse_timeline_scope("--foo"),
        (CssTimelineScopeValueKind::List, vec!["--foo".to_string()])
    );
    assert_eq!(
        parse_timeline_scope("--foo, --bar"),
        (
            CssTimelineScopeValueKind::List,
            vec!["--foo".to_string(), "--bar".to_string()]
        )
    );
}

#[test]
fn rejects_invalid_timeline_scope_values() {
    assert_eq!(parse_timeline_scope("").0, CssTimelineScopeValueKind::Invalid);
    assert_eq!(
        parse_timeline_scope("none, --foo").0,
        CssTimelineScopeValueKind::Invalid
    );
    assert_eq!(parse_timeline_scope("all, --foo").0, CssTimelineScopeValueKind::Invalid);
    assert_eq!(parse_timeline_scope("--foo,").0, CssTimelineScopeValueKind::Invalid);
    assert_eq!(
        parse_timeline_scope("--foo --bar").0,
        CssTimelineScopeValueKind::Invalid
    );
    assert_eq!(parse_timeline_scope("foo").0, CssTimelineScopeValueKind::Invalid);
}

#[test]
fn parses_timeline_name_values() {
    assert_eq!(
        parse_timeline_name("none"),
        (
            CssTimelineNameValueKind::List,
            vec![(CssTimelineNameItemKind::None, String::new())]
        )
    );
    assert_eq!(
        parse_timeline_name("--foo, --bar"),
        (
            CssTimelineNameValueKind::List,
            vec![
                (CssTimelineNameItemKind::DashedIdent, "--foo".to_string()),
                (CssTimelineNameItemKind::DashedIdent, "--bar".to_string())
            ]
        )
    );
    assert_eq!(
        parse_timeline_name("--a, none, --b"),
        (
            CssTimelineNameValueKind::List,
            vec![
                (CssTimelineNameItemKind::DashedIdent, "--a".to_string()),
                (CssTimelineNameItemKind::None, String::new()),
                (CssTimelineNameItemKind::DashedIdent, "--b".to_string())
            ]
        )
    );
}

#[test]
fn rejects_invalid_timeline_name_values() {
    assert_eq!(parse_timeline_name("").0, CssTimelineNameValueKind::Invalid);
    assert_eq!(parse_timeline_name("auto").0, CssTimelineNameValueKind::Invalid);
    assert_eq!(parse_timeline_name("abc").0, CssTimelineNameValueKind::Invalid);
    assert_eq!(parse_timeline_name("default").0, CssTimelineNameValueKind::Invalid);
    assert_eq!(parse_timeline_name("--foo --bar").0, CssTimelineNameValueKind::Invalid);
    assert_eq!(parse_timeline_name("--foo,").0, CssTimelineNameValueKind::Invalid);
    assert_eq!(parse_timeline_name("10px").0, CssTimelineNameValueKind::Invalid);
}

#[test]
fn parses_position_try_order_values() {
    assert_eq!(parse_position_try_order("normal"), CssPositionTryOrderValue::Normal);
    assert_eq!(
        parse_position_try_order("most-width"),
        CssPositionTryOrderValue::MostWidth
    );
    assert_eq!(
        parse_position_try_order("most-height"),
        CssPositionTryOrderValue::MostHeight
    );
    assert_eq!(
        parse_position_try_order("most-block-size"),
        CssPositionTryOrderValue::MostBlockSize
    );
    assert_eq!(
        parse_position_try_order("most-inline-size"),
        CssPositionTryOrderValue::MostInlineSize
    );
}

#[test]
fn rejects_invalid_position_try_order_values() {
    assert_eq!(parse_position_try_order(""), CssPositionTryOrderValue::Invalid);
    assert_eq!(parse_position_try_order("auto"), CssPositionTryOrderValue::Invalid);
    assert_eq!(
        parse_position_try_order("normal most-inline-size"),
        CssPositionTryOrderValue::Invalid
    );
    assert_eq!(
        parse_position_try_order("most-block-size most-inline-size"),
        CssPositionTryOrderValue::Invalid
    );
    assert_eq!(
        parse_position_try_order("most-block-size, most-inline-size"),
        CssPositionTryOrderValue::Invalid
    );
    assert_eq!(parse_position_try_order("10px"), CssPositionTryOrderValue::Invalid);
}

#[test]
fn parses_position_visibility_values() {
    assert_eq!(
        parse_position_visibility("always"),
        CssPositionVisibilityValue {
            kind: CssPositionVisibilityValueKind::Always,
            has_anchors_valid: false,
            has_anchors_visible: false,
            has_no_overflow: false,
        }
    );
    assert_eq!(
        parse_position_visibility("anchors-visible"),
        CssPositionVisibilityValue {
            kind: CssPositionVisibilityValueKind::List,
            has_anchors_valid: false,
            has_anchors_visible: true,
            has_no_overflow: false,
        }
    );
    assert_eq!(
        parse_position_visibility("no-overflow anchors-valid anchors-visible"),
        CssPositionVisibilityValue {
            kind: CssPositionVisibilityValueKind::List,
            has_anchors_valid: true,
            has_anchors_visible: true,
            has_no_overflow: true,
        }
    );
}

#[test]
fn rejects_invalid_position_visibility_values() {
    assert_eq!(
        parse_position_visibility("").kind,
        CssPositionVisibilityValueKind::Invalid
    );
    assert_eq!(
        parse_position_visibility("always anchors-valid").kind,
        CssPositionVisibilityValueKind::Invalid
    );
    assert_eq!(
        parse_position_visibility("anchors-valid anchors-valid").kind,
        CssPositionVisibilityValueKind::Invalid
    );
    assert_eq!(
        parse_position_visibility("anchors-visible foobar").kind,
        CssPositionVisibilityValueKind::Invalid
    );
    assert_eq!(
        parse_position_visibility("anchors-valid, anchors-visible").kind,
        CssPositionVisibilityValueKind::Invalid
    );
}

#[test]
fn parses_white_space_trim_values() {
    assert_eq!(
        parse_white_space_trim("none"),
        CssWhiteSpaceTrimValue {
            kind: CssWhiteSpaceTrimValueKind::None,
            has_discard_before: false,
            has_discard_after: false,
            has_discard_inner: false,
        }
    );
    assert_eq!(
        parse_white_space_trim("discard-inner discard-before"),
        CssWhiteSpaceTrimValue {
            kind: CssWhiteSpaceTrimValueKind::List,
            has_discard_before: true,
            has_discard_after: false,
            has_discard_inner: true,
        }
    );
    assert_eq!(
        parse_white_space_trim("discard-after discard-inner discard-before"),
        CssWhiteSpaceTrimValue {
            kind: CssWhiteSpaceTrimValueKind::List,
            has_discard_before: true,
            has_discard_after: true,
            has_discard_inner: true,
        }
    );
}

#[test]
fn rejects_invalid_white_space_trim_values() {
    assert_eq!(parse_white_space_trim("").kind, CssWhiteSpaceTrimValueKind::Invalid);
    assert_eq!(parse_white_space_trim("auto").kind, CssWhiteSpaceTrimValueKind::Invalid);
    assert_eq!(
        parse_white_space_trim("none discard-before").kind,
        CssWhiteSpaceTrimValueKind::Invalid
    );
    assert_eq!(
        parse_white_space_trim("discard-before discard-before").kind,
        CssWhiteSpaceTrimValueKind::Invalid
    );
    assert_eq!(
        parse_white_space_trim("discard-after discard-after").kind,
        CssWhiteSpaceTrimValueKind::Invalid
    );
    assert_eq!(
        parse_white_space_trim("discard-inner discard-inner").kind,
        CssWhiteSpaceTrimValueKind::Invalid
    );
    assert_eq!(
        parse_white_space_trim("discard-inner, discard-before").kind,
        CssWhiteSpaceTrimValueKind::Invalid
    );
}

#[test]
fn parses_container_type_values() {
    assert_eq!(parse_container_type("normal"), CssContainerTypeValueKind::Normal);
    assert_eq!(parse_container_type("size"), CssContainerTypeValueKind::Size);
    assert_eq!(
        parse_container_type("inline-size"),
        CssContainerTypeValueKind::InlineSize
    );
    assert_eq!(
        parse_container_type("scroll-state"),
        CssContainerTypeValueKind::ScrollState
    );
    assert_eq!(
        parse_container_type("size scroll-state"),
        CssContainerTypeValueKind::SizeAndScrollState
    );
    assert_eq!(
        parse_container_type("scroll-state size"),
        CssContainerTypeValueKind::SizeAndScrollState
    );
    assert_eq!(
        parse_container_type("inline-size scroll-state"),
        CssContainerTypeValueKind::InlineSizeAndScrollState
    );
    assert_eq!(
        parse_container_type("scroll-state inline-size"),
        CssContainerTypeValueKind::InlineSizeAndScrollState
    );
}

#[test]
fn rejects_invalid_container_type_values() {
    assert_eq!(parse_container_type(""), CssContainerTypeValueKind::Invalid);
    assert_eq!(parse_container_type("none"), CssContainerTypeValueKind::Invalid);
    assert_eq!(parse_container_type("auto"), CssContainerTypeValueKind::Invalid);
    assert_eq!(parse_container_type("block-size"), CssContainerTypeValueKind::Invalid);
    assert_eq!(
        parse_container_type("normal normal"),
        CssContainerTypeValueKind::Invalid
    );
    assert_eq!(
        parse_container_type("normal inline-size"),
        CssContainerTypeValueKind::Invalid
    );
    assert_eq!(
        parse_container_type("inline-size normal"),
        CssContainerTypeValueKind::Invalid
    );
    assert_eq!(
        parse_container_type("size inline-size"),
        CssContainerTypeValueKind::Invalid
    );
    assert_eq!(
        parse_container_type("inline-size size"),
        CssContainerTypeValueKind::Invalid
    );
    assert_eq!(parse_container_type("size size"), CssContainerTypeValueKind::Invalid);
    assert_eq!(
        parse_container_type("scroll-state scroll-state"),
        CssContainerTypeValueKind::Invalid
    );
    assert_eq!(parse_container_type("style"), CssContainerTypeValueKind::Invalid);
    assert_eq!(
        parse_container_type("size nonsense"),
        CssContainerTypeValueKind::Invalid
    );
    assert_eq!(
        parse_container_type("size, scroll-state"),
        CssContainerTypeValueKind::Invalid
    );
}

#[test]
fn rejects_invalid_media_query_syntax_nodes() {
    let queries = parse_media_query_list("layer, screen or (hover), screen and (hover) or (color)");
    assert_eq!(queries.len(), 3);
    assert!(queries.iter().all(|query| matches!(query, MediaQuerySyntax::Invalid)));
}

#[test]
fn parses_stylesheet_contents() {
    let rules = parse_with("@media screen { body { color: red } } a { color: blue }", |parser| {
        parser.parse_a_stylesheets_contents()
    });

    assert_eq!(rules.len(), 2);
    let Rule::AtRule(at_rule) = &rules[0] else {
        panic!("expected an at-rule");
    };
    assert_eq!(at_rule.name, "media");
    assert!(at_rule.is_block_rule);
    assert_eq!(at_rule.child_rules_and_lists_of_declarations.len(), 1);

    let Rule::QualifiedRule(qualified_rule) = &rules[1] else {
        panic!("expected a qualified rule");
    };
    assert_eq!(qualified_rule.declarations.len(), 1);
    assert_eq!(qualified_rule.declarations[0].name, "color");
}

#[test]
fn parses_block_contents() {
    let rules = parse_with(
        "color: red; @media screen { color: green } & { color: blue }",
        |parser| {
            parser.rule_context.push(RuleContext::Style);
            let rules = parser.parse_a_blocks_contents();
            parser.rule_context.pop();
            rules
        },
    );

    assert_eq!(rules.len(), 3);
    let RuleOrListOfDeclarations::ListOfDeclarations(declarations) = &rules[0] else {
        panic!("expected declarations");
    };
    assert_eq!(declarations.len(), 1);
    assert_eq!(declarations[0].name, "color");

    assert!(matches!(rules[1], RuleOrListOfDeclarations::Rule(Rule::AtRule(_))));
    assert!(matches!(
        rules[2],
        RuleOrListOfDeclarations::Rule(Rule::QualifiedRule(_))
    ));
}

#[test]
fn parses_important_declarations() {
    let declaration = parse_with("color: red ! important", Parser::parse_a_declaration).expect("expected declaration");

    assert_eq!(declaration.name, "color");
    assert!(declaration.important);
    assert!(!declaration
            .value
            .iter()
            .any(|value| matches!(value, ComponentValue::PreservedToken(token) if matches!(token.token_type, TokenType::Delim { value } if value == '!' as u32))));
}

#[test]
fn rejects_non_custom_declarations_with_curly_block_and_other_values() {
    let declaration = parse_with("color: { red } blue", Parser::parse_a_declaration);

    assert!(declaration.is_none());
}
