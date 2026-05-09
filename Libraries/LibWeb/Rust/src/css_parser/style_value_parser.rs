/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(crate) fn parse_rust_owned_style_value_for_property(
    property_ids: &[u16],
    filtered_input: &[u8],
) -> RustOwnedStyleValueParseResult {
    parse_rust_owned_style_value_for_property_with_options(
        property_ids,
        filtered_input,
        CssPrimitiveValueOptions::default(),
    )
}

pub(crate) fn parse_rust_owned_style_value_for_property_with_options(
    property_ids: &[u16],
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
) -> RustOwnedStyleValueParseResult {
    parse_rust_owned_style_value_for_property_with_mode(property_ids, filtered_input, false, primitive_value_options)
}

pub(super) fn parse_rust_owned_style_value_for_property_with_mode(
    property_ids: &[u16],
    filtered_input: &[u8],
    is_coordinating_shorthand_item: bool,
    primitive_value_options: CssPrimitiveValueOptions,
) -> RustOwnedStyleValueParseResult {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        // AD-HOC: These list-valued longhands parse to complete StyleValueLists
        // when materialized in C++. During coordinating shorthand parsing we
        // need one layer item at a time, since the shorthand parser wraps each
        // longhand's layer items into the final outer comma-separated list.
        //
        // This also keeps `animation-name`, which accepts arbitrary custom
        // identifiers, from stealing keywords such as `ease-in` from the other
        // animation longhands while parsing the `animation` shorthand.
        if is_coordinating_shorthand_item && property_parses_as_coordinating_shorthand_item(property_id) {
            continue;
        }
        // AD-HOC: The `background` and `mask` shorthands parse one layer at a
        // time and then wrap those layer values into the final comma-separated
        // longhand lists in C++. Keep their position components on the
        // generated value-type path so they materialize as a single
        // `PositionStyleValue`, while direct longhand parsing below keeps
        // owning the full comma-separated layer list in Rust.
        if property_ids.len() > 1
            && matches!(
                property_id,
                PropertyId::BackgroundPosition
                    | PropertyId::BackgroundPositionX
                    | PropertyId::BackgroundPositionY
                    | PropertyId::MaskPosition
            )
        {
            continue;
        }
        if let Some(value) =
            parse_rust_owned_property_specific_longhand_value(property_id, filtered_input, primitive_value_options)
        {
            return RustOwnedStyleValueParseResult::Parsed(RustOwnedStyleValue { property_id, value });
        }
    }

    if let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
    {
        for property_id in property_ids {
            let Some(property_id) = property_id_from_u16(*property_id) else {
                continue;
            };
            if property_accepts_keyword(property_id, value) {
                let resolved_keyword = resolve_legacy_value_alias(property_id, value).unwrap_or(value);
                return RustOwnedStyleValueParseResult::Parsed(RustOwnedStyleValue {
                    property_id,
                    value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(
                        resolved_keyword.to_string(),
                    )),
                });
            }
        }
    }

    if property_ids.len() == 1
        && let Some(property_id) = property_id_from_u16(property_ids[0])
        && property_uses_rust_owned_whole_grammar(property_id)
        && !(is_coordinating_shorthand_item && property_parses_as_coordinating_shorthand_item(property_id))
    {
        return RustOwnedStyleValueParseResult::Invalid;
    }

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        if is_coordinating_shorthand_item && property_parses_as_coordinating_shorthand_item(property_id) {
            continue;
        }
        if !property_accepts_value_type(property_id, PropertyValueType::CustomIdent) {
            continue;
        }

        let mut parser = ComponentValueParser::new(component_values.to_vec());
        if let Some(name) = parser.parse_a_custom_ident(property_custom_ident_blacklist(property_id)) {
            return RustOwnedStyleValueParseResult::Parsed(RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::CustomIdent {
                    value: name,
                    value_type: PropertyValueType::CustomIdent,
                }),
            });
        }
    }

    for value_type in generated_property_value_type_order() {
        for property_id in property_ids {
            let Some(property_id) = property_id_from_u16(*property_id) else {
                continue;
            };
            if !property_accepts_value_type(property_id, *value_type) {
                continue;
            }
            if property_ids.len() != 1 && *value_type == PropertyValueType::FontStyle {
                continue;
            }
            if property_ids.len() != 1 && *value_type == PropertyValueType::ViewTimelineInset {
                if let Some(values) = parse_rust_owned_view_timeline_inset_value_prefix(filtered_input) {
                    return RustOwnedStyleValueParseResult::Parsed(RustOwnedStyleValue {
                        property_id,
                        value: RustOwnedStyleValueKind::ViewTimelineInset(RustOwnedViewTimelineInset { values }),
                    });
                }
                continue;
            }
            let value_type_matches = if *value_type == PropertyValueType::Url {
                match property_id {
                    PropertyId::ClipPath => parse_a_url_function(filtered_input, |_| {}, |_| {}),
                    PropertyId::MaskImage => component_values_parse_as_fragment_url(filtered_input),
                    _ => false,
                }
            } else {
                component_values_parse_as_property_value_type_with_options(
                    *value_type,
                    filtered_input,
                    primitive_value_options,
                )
            };
            if !value_type_matches {
                continue;
            }

            return RustOwnedStyleValueParseResult::Parsed(parse_rust_owned_generated_longhand_value_with_options(
                property_id,
                *value_type,
                filtered_input,
                component_values,
                primitive_value_options,
            ));
        }
    }

    if is_coordinating_shorthand_item {
        for property_id in property_ids {
            let Some(property_id) = property_id_from_u16(*property_id) else {
                continue;
            };
            if !property_parses_as_coordinating_shorthand_item(property_id)
                || !property_accepts_value_type(property_id, PropertyValueType::CustomIdent)
            {
                continue;
            }

            let mut parser = ComponentValueParser::new(component_values.to_vec());
            if let Some(name) = parser.parse_a_custom_ident(property_custom_ident_blacklist(property_id)) {
                return RustOwnedStyleValueParseResult::Parsed(RustOwnedStyleValue {
                    property_id,
                    value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::CustomIdent {
                        value: name,
                        value_type: PropertyValueType::CustomIdent,
                    }),
                });
            }
        }
    }

    RustOwnedStyleValueParseResult::Invalid
}

fn property_parses_as_coordinating_shorthand_item(property_id: PropertyId) -> bool {
    matches!(
        property_id,
        PropertyId::AnimationName
            | PropertyId::AnimationDelay
            | PropertyId::AnimationTimingFunction
            | PropertyId::ScrollTimelineName
            | PropertyId::TransitionBehavior
            | PropertyId::TransitionDelay
            | PropertyId::TransitionDuration
            | PropertyId::TransitionProperty
            | PropertyId::TransitionTimingFunction
            | PropertyId::ViewTimelineName
    )
}

fn property_uses_rust_owned_whole_grammar(property_id: PropertyId) -> bool {
    matches!(
        property_id,
        PropertyId::AccentColor
            | PropertyId::AnchorName
            | PropertyId::AnchorScope
            | PropertyId::AnimationName
            | PropertyId::AnimationDelay
            | PropertyId::AnimationTimingFunction
            | PropertyId::Appearance
            | PropertyId::AspectRatio
            | PropertyId::BlockSize
            | PropertyId::BackgroundRepeat
            | PropertyId::BackgroundPosition
            | PropertyId::BackgroundPositionX
            | PropertyId::BackgroundPositionY
            | PropertyId::BackgroundSize
            | PropertyId::BackgroundColor
            | PropertyId::Border
            | PropertyId::BorderBlock
            | PropertyId::BorderImage
            | PropertyId::BorderImageOutset
            | PropertyId::BorderImageRepeat
            | PropertyId::BorderImageSlice
            | PropertyId::BorderImageSource
            | PropertyId::BorderImageWidth
            | PropertyId::BorderInline
            | PropertyId::BorderBottomColor
            | PropertyId::BorderBottomStyle
            | PropertyId::BorderCollapse
            | PropertyId::BorderBottomLeftRadius
            | PropertyId::BorderBottomRightRadius
            | PropertyId::BorderBlockEndWidth
            | PropertyId::BorderBlockStartWidth
            | PropertyId::BorderBottomWidth
            | PropertyId::BorderEndEndRadius
            | PropertyId::BorderEndStartRadius
            | PropertyId::BorderInlineEndWidth
            | PropertyId::BorderInlineStartWidth
            | PropertyId::BorderLeftColor
            | PropertyId::BorderLeftStyle
            | PropertyId::BorderLeftWidth
            | PropertyId::BorderRadius
            | PropertyId::BorderRightColor
            | PropertyId::BorderRightStyle
            | PropertyId::BorderRightWidth
            | PropertyId::BorderStartEndRadius
            | PropertyId::BorderStartStartRadius
            | PropertyId::BorderTopLeftRadius
            | PropertyId::BorderTopRightRadius
            | PropertyId::BorderTopColor
            | PropertyId::BorderTopStyle
            | PropertyId::BorderTopWidth
            | PropertyId::Bottom
            | PropertyId::BoxShadow
            | PropertyId::BoxSizing
            | PropertyId::BackdropFilter
            | PropertyId::CaptionSide
            | PropertyId::CaretColor
            | PropertyId::Clear
            | PropertyId::ClipRule
            | PropertyId::ColorInterpolation
            | PropertyId::ColorScheme
            | PropertyId::Color
            | PropertyId::ColumnCount
            | PropertyId::ColumnSpan
            | PropertyId::ColumnWidth
            | PropertyId::Columns
            | PropertyId::Contain
            | PropertyId::ContainerType
            | PropertyId::Content
            | PropertyId::ContentVisibility
            | PropertyId::CounterIncrement
            | PropertyId::CounterReset
            | PropertyId::CounterSet
            | PropertyId::Cursor
            | PropertyId::Cx
            | PropertyId::Cy
            | PropertyId::Direction
            | PropertyId::Display
            | PropertyId::DominantBaseline
            | PropertyId::EmptyCells
            | PropertyId::Fill
            | PropertyId::FillOpacity
            | PropertyId::FillRule
            | PropertyId::Filter
            | PropertyId::Flex
            | PropertyId::FlexBasis
            | PropertyId::FlexDirection
            | PropertyId::FlexFlow
            | PropertyId::FlexWrap
            | PropertyId::Float
            | PropertyId::FontFamily
            | PropertyId::FontFeatureSettings
            | PropertyId::FontLanguageOverride
            | PropertyId::FontSize
            | PropertyId::FontVariant
            | PropertyId::FontVariationSettings
            | PropertyId::FontWeight
            | PropertyId::FontWidth
            | PropertyId::FloodColor
            | PropertyId::FloodOpacity
            | PropertyId::FlexGrow
            | PropertyId::FlexShrink
            | PropertyId::ColumnGap
            | PropertyId::GridAutoColumns
            | PropertyId::GridAutoFlow
            | PropertyId::GridAutoRows
            | PropertyId::GridColumnEnd
            | PropertyId::GridColumnStart
            | PropertyId::GridRowEnd
            | PropertyId::GridRowStart
            | PropertyId::GridTemplateAreas
            | PropertyId::GridTemplateColumns
            | PropertyId::GridTemplateRows
            | PropertyId::Height
            | PropertyId::InlineSize
            | PropertyId::InsetBlockEnd
            | PropertyId::InsetBlockStart
            | PropertyId::InsetInlineEnd
            | PropertyId::InsetInlineStart
            | PropertyId::ImageRendering
            | PropertyId::Isolation
            | PropertyId::LetterSpacing
            | PropertyId::Left
            | PropertyId::ListStyle
            | PropertyId::ListStyleImage
            | PropertyId::ListStylePosition
            | PropertyId::MarginBlockEnd
            | PropertyId::MarginBlockStart
            | PropertyId::MarginBottom
            | PropertyId::MarginInlineEnd
            | PropertyId::MarginInlineStart
            | PropertyId::MarginLeft
            | PropertyId::MarginRight
            | PropertyId::MarginTop
            | PropertyId::MaskRepeat
            | PropertyId::MaskPosition
            | PropertyId::MaskSize
            | PropertyId::MathDepth
            | PropertyId::MathShift
            | PropertyId::MathStyle
            | PropertyId::MaxBlockSize
            | PropertyId::MaxHeight
            | PropertyId::MaxInlineSize
            | PropertyId::MaxWidth
            | PropertyId::MinBlockSize
            | PropertyId::MinHeight
            | PropertyId::MinInlineSize
            | PropertyId::MinWidth
            | PropertyId::MixBlendMode
            | PropertyId::ObjectFit
            | PropertyId::ObjectPosition
            | PropertyId::OverflowWrap
            | PropertyId::OverflowClipMargin
            | PropertyId::OverflowClipMarginBlock
            | PropertyId::OverflowClipMarginBlockEnd
            | PropertyId::OverflowClipMarginBlockStart
            | PropertyId::OverflowClipMarginBottom
            | PropertyId::OverflowClipMarginInline
            | PropertyId::OverflowClipMarginInlineEnd
            | PropertyId::OverflowClipMarginInlineStart
            | PropertyId::OverflowClipMarginLeft
            | PropertyId::OverflowClipMarginRight
            | PropertyId::OverflowClipMarginTop
            | PropertyId::OverflowX
            | PropertyId::OverflowY
            | PropertyId::Opacity
            | PropertyId::Order
            | PropertyId::OutlineColor
            | PropertyId::OutlineOffset
            | PropertyId::OutlineStyle
            | PropertyId::OutlineWidth
            | PropertyId::Orphans
            | PropertyId::PaddingBlockEnd
            | PropertyId::PaddingBlockStart
            | PropertyId::PaddingBottom
            | PropertyId::PaddingInlineEnd
            | PropertyId::PaddingInlineStart
            | PropertyId::PaddingLeft
            | PropertyId::PaddingRight
            | PropertyId::PaddingTop
            | PropertyId::PaintOrder
            | PropertyId::Perspective
            | PropertyId::PerspectiveOrigin
            | PropertyId::PointerEvents
            | PropertyId::Position
            | PropertyId::PlaceContent
            | PropertyId::PlaceItems
            | PropertyId::PlaceSelf
            | PropertyId::PositionAnchor
            | PropertyId::PositionArea
            | PropertyId::PositionTryFallbacks
            | PropertyId::PositionTryOrder
            | PropertyId::PositionVisibility
            | PropertyId::Quotes
            | PropertyId::R
            | PropertyId::Resize
            | PropertyId::Right
            | PropertyId::Rotate
            | PropertyId::RowGap
            | PropertyId::Rx
            | PropertyId::Ry
            | PropertyId::Scale
            | PropertyId::ScrollBehavior
            | PropertyId::ScrollTimeline
            | PropertyId::ScrollTimelineName
            | PropertyId::ScrollbarColor
            | PropertyId::ScrollbarGutter
            | PropertyId::ScrollbarWidth
            | PropertyId::ShapeRendering
            | PropertyId::ShapeMargin
            | PropertyId::ShapeOutside
            | PropertyId::ShapeImageThreshold
            | PropertyId::StopColor
            | PropertyId::StopOpacity
            | PropertyId::Stroke
            | PropertyId::StrokeDasharray
            | PropertyId::StrokeDashoffset
            | PropertyId::StrokeLinecap
            | PropertyId::StrokeLinejoin
            | PropertyId::StrokeMiterlimit
            | PropertyId::StrokeOpacity
            | PropertyId::StrokeWidth
            | PropertyId::TabSize
            | PropertyId::TableLayout
            | PropertyId::TextAlign
            | PropertyId::TextAnchor
            | PropertyId::TextDecoration
            | PropertyId::TextDecorationColor
            | PropertyId::TextDecorationLine
            | PropertyId::TextDecorationSkipInk
            | PropertyId::TextDecorationStyle
            | PropertyId::TextDecorationThickness
            | PropertyId::TextIndent
            | PropertyId::TextJustify
            | PropertyId::TextRendering
            | PropertyId::TextShadow
            | PropertyId::TextTransform
            | PropertyId::TextUnderlineOffset
            | PropertyId::TextUnderlinePosition
            | PropertyId::TextWrap
            | PropertyId::TextWrapMode
            | PropertyId::TextWrapStyle
            | PropertyId::TimelineScope
            | PropertyId::Top
            | PropertyId::TouchAction
            | PropertyId::Transform
            | PropertyId::TransformBox
            | PropertyId::TransformOrigin
            | PropertyId::TransformStyle
            | PropertyId::TransitionBehavior
            | PropertyId::TransitionDelay
            | PropertyId::TransitionDuration
            | PropertyId::TransitionProperty
            | PropertyId::TransitionTimingFunction
            | PropertyId::Translate
            | PropertyId::UnicodeBidi
            | PropertyId::UserSelect
            | PropertyId::VerticalAlign
            | PropertyId::ViewTimeline
            | PropertyId::ViewTimelineName
            | PropertyId::ViewTransitionName
            | PropertyId::Visibility
            | PropertyId::WhiteSpace
            | PropertyId::WhiteSpaceCollapse
            | PropertyId::WhiteSpaceTrim
            | PropertyId::Widows
            | PropertyId::Width
            | PropertyId::WebkitTextFillColor
            | PropertyId::WillChange
            | PropertyId::WordBreak
            | PropertyId::WordSpacing
            | PropertyId::WritingMode
            | PropertyId::X
            | PropertyId::Y
            | PropertyId::ZIndex
    )
}

fn parse_rust_owned_property_specific_longhand_value(
    property_id: PropertyId,
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<RustOwnedStyleValueKind> {
    match property_id {
        PropertyId::AnchorName => rust_owned_anchor_name_or_scope_style_value_kind(filtered_input, false),
        PropertyId::AnchorScope => rust_owned_anchor_name_or_scope_style_value_kind(filtered_input, true),
        PropertyId::Inset
        | PropertyId::Top
        | PropertyId::Right
        | PropertyId::Bottom
        | PropertyId::Left
        | PropertyId::InsetBlock
        | PropertyId::InsetBlockStart
        | PropertyId::InsetBlockEnd
        | PropertyId::InsetInline
        | PropertyId::InsetInlineStart
        | PropertyId::InsetInlineEnd => {
            rust_owned_inset_property_style_value_kind(property_id, filtered_input, primitive_value_options)
        }
        PropertyId::BackgroundSize | PropertyId::MaskSize => {
            rust_owned_background_size_style_value_kind(filtered_input)
        }
        PropertyId::BackgroundPosition => {
            rust_owned_position_list_style_value_kind(PropertyValueType::BackgroundPosition, filtered_input)
        }
        PropertyId::BackgroundPositionX | PropertyId::BackgroundPositionY => {
            rust_owned_background_position_longhand_list_style_value_kind(property_id, filtered_input)
        }
        PropertyId::AnimationName => rust_owned_animation_name_style_value_kind(filtered_input),
        PropertyId::AnimationDelay
        | PropertyId::AnimationTimingFunction
        | PropertyId::TransitionDelay
        | PropertyId::TransitionDuration
        | PropertyId::TransitionTimingFunction => {
            rust_owned_generated_value_list_style_value_kind(property_id, filtered_input)
        }
        PropertyId::AspectRatio => rust_owned_aspect_ratio_style_value_kind(filtered_input),
        PropertyId::Border | PropertyId::BorderBlock | PropertyId::BorderInline => {
            rust_owned_border_shorthand_style_value_kind(property_id, filtered_input)
        }
        PropertyId::BorderRadius => rust_owned_border_radius_shorthand_style_value_kind(filtered_input),
        PropertyId::BorderImage => rust_owned_border_image_shorthand_style_value_kind(filtered_input),
        PropertyId::BorderImageOutset => rust_owned_border_image_outset_style_value_kind(filtered_input),
        PropertyId::BorderImageRepeat => rust_owned_border_image_repeat_style_value_kind(filtered_input),
        PropertyId::BorderImageSlice => rust_owned_border_image_slice_style_value_kind(filtered_input),
        PropertyId::BorderImageWidth => rust_owned_border_image_width_style_value_kind(filtered_input),
        PropertyId::BorderBottomLeftRadius
        | PropertyId::BorderBottomRightRadius
        | PropertyId::BorderEndEndRadius
        | PropertyId::BorderEndStartRadius
        | PropertyId::BorderStartEndRadius
        | PropertyId::BorderStartStartRadius
        | PropertyId::BorderTopLeftRadius
        | PropertyId::BorderTopRightRadius => rust_owned_border_radius_style_value_kind(filtered_input),
        PropertyId::BoxShadow | PropertyId::TextShadow => {
            rust_owned_shadow_style_value_kind(property_id, filtered_input)
        }
        PropertyId::ColorScheme => rust_owned_color_scheme_style_value_kind(filtered_input),
        PropertyId::Contain => rust_owned_contain_style_value_kind(filtered_input),
        PropertyId::ContainerType => rust_owned_container_type_style_value_kind(filtered_input),
        PropertyId::AccentColor
        | PropertyId::BackgroundColor
        | PropertyId::BorderBottomColor
        | PropertyId::BorderLeftColor
        | PropertyId::BorderRightColor
        | PropertyId::BorderTopColor
        | PropertyId::CaretColor
        | PropertyId::Color
        | PropertyId::WebkitTextFillColor
        | PropertyId::Fill
        | PropertyId::FillOpacity
        | PropertyId::FloodColor
        | PropertyId::FloodOpacity
        | PropertyId::BlockSize
        | PropertyId::BorderBlockEndWidth
        | PropertyId::BorderBlockStartWidth
        | PropertyId::BorderBottomWidth
        | PropertyId::BorderInlineEndWidth
        | PropertyId::BorderInlineStartWidth
        | PropertyId::BorderLeftWidth
        | PropertyId::BorderRightWidth
        | PropertyId::BorderTopWidth
        | PropertyId::ColumnGap
        | PropertyId::ColumnWidth
        | PropertyId::Cx
        | PropertyId::Cy
        | PropertyId::FlexBasis
        | PropertyId::FlexGrow
        | PropertyId::FlexShrink
        | PropertyId::FontSize
        | PropertyId::FontWeight
        | PropertyId::FontWidth
        | PropertyId::Height
        | PropertyId::InlineSize
        | PropertyId::LetterSpacing
        | PropertyId::MarginBlockEnd
        | PropertyId::MarginBlockStart
        | PropertyId::MarginBottom
        | PropertyId::MarginInlineEnd
        | PropertyId::MarginInlineStart
        | PropertyId::MarginLeft
        | PropertyId::MarginRight
        | PropertyId::MarginTop
        | PropertyId::MaxBlockSize
        | PropertyId::MaxHeight
        | PropertyId::MaxInlineSize
        | PropertyId::MaxWidth
        | PropertyId::MinBlockSize
        | PropertyId::MinHeight
        | PropertyId::MinInlineSize
        | PropertyId::MinWidth
        | PropertyId::Opacity
        | PropertyId::ColumnCount
        | PropertyId::Order
        | PropertyId::OutlineColor
        | PropertyId::OutlineOffset
        | PropertyId::OutlineWidth
        | PropertyId::Orphans
        | PropertyId::PaddingBlockEnd
        | PropertyId::PaddingBlockStart
        | PropertyId::PaddingBottom
        | PropertyId::PaddingInlineEnd
        | PropertyId::PaddingInlineStart
        | PropertyId::PaddingLeft
        | PropertyId::PaddingRight
        | PropertyId::PaddingTop
        | PropertyId::Perspective
        | PropertyId::R
        | PropertyId::RowGap
        | PropertyId::Rx
        | PropertyId::Ry
        | PropertyId::ShapeMargin
        | PropertyId::ShapeImageThreshold
        | PropertyId::StopColor
        | PropertyId::StopOpacity
        | PropertyId::Stroke
        | PropertyId::StrokeDashoffset
        | PropertyId::StrokeMiterlimit
        | PropertyId::StrokeOpacity
        | PropertyId::StrokeWidth
        | PropertyId::TabSize
        | PropertyId::TextDecorationColor
        | PropertyId::TextDecorationThickness
        | PropertyId::TextUnderlineOffset
        | PropertyId::VerticalAlign
        | PropertyId::Widows
        | PropertyId::Width
        | PropertyId::X
        | PropertyId::Y
        | PropertyId::WordSpacing
        | PropertyId::ZIndex => rust_owned_generated_property_specific_style_value_kind(
            property_id,
            filtered_input,
            primitive_value_options,
        ),
        PropertyId::Columns => rust_owned_columns_style_value_kind(filtered_input),
        PropertyId::Content => rust_owned_content_style_value_kind(filtered_input),
        PropertyId::CounterIncrement => rust_owned_counter_definitions_style_value_kind(filtered_input, false, 1),
        PropertyId::CounterReset => rust_owned_counter_definitions_style_value_kind(filtered_input, true, 0),
        PropertyId::CounterSet => rust_owned_counter_definitions_style_value_kind(filtered_input, false, 0),
        PropertyId::Display => rust_owned_display_style_value_kind(filtered_input),
        PropertyId::Flex => rust_owned_flex_shorthand_style_value_kind(filtered_input),
        PropertyId::FlexFlow => rust_owned_flex_flow_style_value_kind(filtered_input),
        PropertyId::BackdropFilter | PropertyId::Filter => {
            rust_owned_filter_value_list_style_value_kind(filtered_input)
        }
        PropertyId::FontFamily => rust_owned_font_family_style_value_kind(filtered_input),
        PropertyId::FontFeatureSettings => rust_owned_font_feature_settings_style_value_kind(filtered_input),
        PropertyId::FontLanguageOverride => rust_owned_font_language_override_style_value_kind(filtered_input),
        PropertyId::FontVariant => rust_owned_font_variant_style_value_kind(filtered_input),
        PropertyId::FontVariationSettings => rust_owned_font_variation_settings_style_value_kind(filtered_input),
        PropertyId::GridAutoColumns | PropertyId::GridAutoRows => {
            rust_owned_grid_auto_track_sizes_style_value_kind(filtered_input)
        }
        PropertyId::GridAutoFlow => rust_owned_grid_auto_flow_style_value_kind(filtered_input),
        PropertyId::GridColumnEnd | PropertyId::GridColumnStart | PropertyId::GridRowEnd | PropertyId::GridRowStart => {
            rust_owned_grid_track_placement_style_value_kind(filtered_input)
        }
        PropertyId::GridTemplateAreas => rust_owned_grid_template_areas_style_value_kind(filtered_input),
        PropertyId::GridTemplateColumns | PropertyId::GridTemplateRows => {
            rust_owned_grid_track_size_list_style_value_kind(filtered_input)
        }
        PropertyId::ListStyle => rust_owned_list_style_style_value_kind(filtered_input),
        PropertyId::BorderImageSource | PropertyId::ListStyleImage => {
            rust_owned_generated_property_specific_style_value_kind(
                property_id,
                filtered_input,
                primitive_value_options,
            )
        }
        PropertyId::ObjectPosition | PropertyId::PerspectiveOrigin => {
            rust_owned_position_style_value_kind(PropertyValueType::Position, filtered_input_to_string(filtered_input))
        }
        PropertyId::MaskPosition => {
            rust_owned_position_list_style_value_kind(PropertyValueType::Position, filtered_input)
        }
        PropertyId::Cursor => rust_owned_cursor_style_value_kind(filtered_input),
        PropertyId::MathDepth => rust_owned_math_depth_style_value_kind(filtered_input),
        PropertyId::OverflowClipMarginBlockEnd
        | PropertyId::OverflowClipMarginBlockStart
        | PropertyId::OverflowClipMarginBottom
        | PropertyId::OverflowClipMarginInlineEnd
        | PropertyId::OverflowClipMarginInlineStart
        | PropertyId::OverflowClipMarginLeft
        | PropertyId::OverflowClipMarginRight
        | PropertyId::OverflowClipMarginTop => rust_owned_overflow_clip_margin_style_value_kind(filtered_input),
        PropertyId::OverflowClipMargin | PropertyId::OverflowClipMarginBlock | PropertyId::OverflowClipMarginInline => {
            rust_owned_overflow_clip_margin_shorthand_style_value_kind(filtered_input)
        }
        PropertyId::PaintOrder => rust_owned_paint_order_style_value_kind(filtered_input),
        PropertyId::PlaceContent => rust_owned_place_content_style_value_kind(filtered_input),
        PropertyId::PlaceItems => rust_owned_place_items_style_value_kind(filtered_input),
        PropertyId::PlaceSelf => rust_owned_place_self_style_value_kind(filtered_input),
        PropertyId::PositionArea => rust_owned_position_area_style_value_kind(filtered_input),
        PropertyId::PositionAnchor => rust_owned_position_anchor_style_value_kind(filtered_input),
        PropertyId::PositionTryFallbacks => rust_owned_position_try_fallbacks_style_value_kind(filtered_input),
        PropertyId::PositionTryOrder => rust_owned_position_try_order_style_value_kind(filtered_input),
        PropertyId::PositionVisibility => rust_owned_position_visibility_style_value_kind(filtered_input),
        PropertyId::Quotes => rust_owned_quotes_style_value_kind(filtered_input),
        PropertyId::BackgroundRepeat | PropertyId::MaskRepeat => {
            rust_owned_repeat_style_style_value_kind(filtered_input)
        }
        PropertyId::ScrollbarColor => rust_owned_scrollbar_color_style_value_kind(filtered_input),
        PropertyId::ScrollbarGutter => rust_owned_scrollbar_gutter_style_value_kind(filtered_input),
        PropertyId::ShapeOutside => rust_owned_shape_outside_style_value_kind(filtered_input),
        PropertyId::ScrollTimeline => rust_owned_scroll_timeline_style_value_kind(filtered_input),
        PropertyId::TextDecoration => rust_owned_text_decoration_style_value_kind(filtered_input),
        PropertyId::TextDecorationLine => rust_owned_text_decoration_line_style_value_kind(filtered_input),
        PropertyId::StrokeDasharray => rust_owned_stroke_dasharray_style_value_kind(filtered_input),
        PropertyId::ScrollTimelineName | PropertyId::ViewTimelineName => {
            rust_owned_timeline_name_style_value_kind(filtered_input)
        }
        PropertyId::TimelineScope => rust_owned_timeline_scope_style_value_kind(filtered_input),
        PropertyId::TextWrap => rust_owned_text_wrap_style_value_kind(filtered_input),
        PropertyId::TextWrapMode => rust_owned_text_wrap_mode_style_value_kind(filtered_input),
        PropertyId::TextWrapStyle => rust_owned_text_wrap_style_style_value_kind(filtered_input),
        PropertyId::TextIndent => rust_owned_text_indent_style_value_kind(filtered_input),
        PropertyId::TextUnderlinePosition => rust_owned_text_underline_position_style_value_kind(filtered_input),
        PropertyId::TouchAction => rust_owned_touch_action_style_value_kind(filtered_input),
        PropertyId::Transform => {
            let (mut parser, _) = parser_from_filtered_input(filtered_input);
            let component_values = parser.parse_a_list_of_component_values();
            let component_values = strip_whitespace(&component_values);
            if matches!(component_values, [component_value] if component_value_is_ident(Some(component_value), "none"))
            {
                Some(RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(
                    "none".to_string(),
                )))
            } else {
                rust_owned_transform_list_style_value_kind(filtered_input, &filtered_input_to_string(filtered_input))
            }
        }
        PropertyId::TransformOrigin => rust_owned_transform_origin_style_value_kind(filtered_input),
        PropertyId::Rotate | PropertyId::Scale | PropertyId::Translate => {
            rust_owned_transform_longhand_style_value_kind(property_id, filtered_input)
        }
        PropertyId::TransitionBehavior => rust_owned_transition_behavior_style_value_kind(filtered_input),
        PropertyId::TransitionProperty => rust_owned_transition_property_style_value_kind(filtered_input),
        PropertyId::ViewTimeline => rust_owned_view_timeline_style_value_kind(filtered_input),
        PropertyId::ViewTransitionName => rust_owned_view_transition_name_style_value_kind(filtered_input),
        PropertyId::WhiteSpace => rust_owned_white_space_style_value_kind(filtered_input),
        PropertyId::WhiteSpaceTrim => rust_owned_white_space_trim_style_value_kind(filtered_input),
        PropertyId::WillChange => rust_owned_will_change_style_value_kind(filtered_input),
        _ => None,
    }
}

fn rust_owned_inset_property_style_value_kind(
    property_id: PropertyId,
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-position-3/#insets
    // Value: auto | <length-percentage>
    //
    // https://drafts.csswg.org/css-anchor-position-1/#anchor-pos
    // The anchor() function resolves to a <length>.
    if let Some(value) = rust_owned_anchor_function_style_value_kind(filtered_input) {
        return Some(value);
    }

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    if let [
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }),
    ] = component_values
        && property_accepts_keyword(property_id, value)
    {
        let resolved_keyword = resolve_legacy_value_alias(property_id, value).unwrap_or(value);
        return Some(RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::Keyword(
            resolved_keyword.to_string(),
        )));
    }

    for value_type in [
        PropertyValueType::Length,
        PropertyValueType::Percentage,
        PropertyValueType::Anchor,
    ] {
        if !property_accepts_value_type(property_id, value_type) {
            continue;
        }
        if !component_values_parse_as_property_value_type_with_options(
            value_type,
            filtered_input,
            primitive_value_options,
        ) {
            continue;
        }
        return Some(
            parse_rust_owned_generated_longhand_value_with_options(
                property_id,
                value_type,
                filtered_input,
                component_values,
                primitive_value_options,
            )
            .value,
        );
    }

    None
}

fn rust_owned_generated_property_specific_style_value_kind(
    property_id: PropertyId,
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // AD-HOC: Prefer the <length> arm for math expressions containing a dimension.
    // The generated grammar accepts calc(10px) as <number> for properties that
    // accept both <number> and <length>, but the CSS parser has historically
    // materialized it as a length.
    if matches!(
        property_id,
        PropertyId::StrokeDashoffset | PropertyId::StrokeWidth | PropertyId::TabSize
    ) && component_values_contain_dimension(component_values)
        && component_values_parse_as_property_value_type_with_options(
            PropertyValueType::Length,
            filtered_input,
            primitive_value_options,
        )
        && component_values_satisfy_property_numeric_range(
            property_id,
            PropertyValueType::Length,
            component_values,
            primitive_value_options,
        )
    {
        return Some(
            parse_rust_owned_generated_longhand_value_with_options(
                property_id,
                PropertyValueType::Length,
                filtered_input,
                component_values,
                primitive_value_options,
            )
            .value,
        );
    }

    // AD-HOC: Prefer the <length> arm for math expressions containing a
    // percentage when the property's percentages resolve against a length.
    // The generated grammar accepts random(10%, 30%) as <number> for
    // properties that accept both <number> and <percentage>, but the CSS parser
    // has historically materialized it through length-percentage parsing.
    if matches!(property_id, PropertyId::StrokeDashoffset | PropertyId::StrokeWidth)
        && component_values_contain_percentage(component_values)
        && component_values_parse_as_property_value_type_with_options(
            PropertyValueType::Length,
            filtered_input,
            primitive_value_options,
        )
        && component_values_satisfy_property_numeric_range(
            property_id,
            PropertyValueType::Length,
            component_values,
            primitive_value_options,
        )
    {
        return Some(
            parse_rust_owned_generated_longhand_value_with_options(
                property_id,
                PropertyValueType::Length,
                filtered_input,
                component_values,
                primitive_value_options,
            )
            .value,
        );
    }

    if property_id == PropertyId::FontWeight
        && component_values_parse_as_property_value_type_with_options(
            PropertyValueType::Number,
            filtered_input,
            primitive_value_options,
        )
        && component_values_satisfy_property_numeric_range(
            property_id,
            PropertyValueType::Number,
            component_values,
            primitive_value_options,
        )
    {
        // AD-HOC: Keep calculated font-weight numbers on the plain <number>
        // path so variable-dependent math keeps the same computed-value
        // behavior as the existing C++ parser.
        return Some(
            parse_rust_owned_generated_longhand_value_with_options(
                property_id,
                PropertyValueType::Number,
                filtered_input,
                component_values,
                primitive_value_options,
            )
            .value,
        );
    }

    for value_type in generated_property_value_type_order() {
        if !property_accepts_value_type(property_id, *value_type) {
            continue;
        }
        if !component_values_parse_as_property_value_type_with_options(
            *value_type,
            filtered_input,
            primitive_value_options,
        ) {
            continue;
        }
        if !component_values_satisfy_property_numeric_range(
            property_id,
            *value_type,
            component_values,
            primitive_value_options,
        ) {
            continue;
        }
        return Some(
            parse_rust_owned_generated_longhand_value_with_options(
                property_id,
                *value_type,
                filtered_input,
                component_values,
                primitive_value_options,
            )
            .value,
        );
    }

    None
}

fn rust_owned_generated_value_list_style_value_kind(
    property_id: PropertyId,
    filtered_input: &[u8],
) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let groups = split_component_values_on_comma(&component_values);
    if groups.is_empty() {
        return None;
    }

    let mut items = Vec::new();
    for group in groups {
        let component_values = strip_whitespace(group);
        if component_values.is_empty() {
            return None;
        }
        let source = serialize_component_values_for_reparsing(component_values, filtered_input_string)?;

        let mut matching_value_type = if let [
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }),
        ] = component_values
            && property_accepts_keyword(property_id, value)
            && property_accepts_value_type(property_id, PropertyValueType::EasingFunction)
        {
            Some(PropertyValueType::EasingFunction)
        } else {
            None
        };
        for value_type in generated_property_value_type_order() {
            if matching_value_type.is_some() {
                break;
            }
            if !property_accepts_value_type(property_id, *value_type) {
                continue;
            }
            if !component_values_parse_as_property_value_type(*value_type, source.as_bytes()) {
                continue;
            }
            if !component_values_satisfy_property_numeric_range(
                property_id,
                *value_type,
                component_values,
                CssPrimitiveValueOptions::default(),
            ) {
                continue;
            }
            matching_value_type = Some(*value_type);
            break;
        }

        items.push(RustOwnedGeneratedValueListItem {
            source,
            value_type: matching_value_type?,
        });
    }

    Some(RustOwnedStyleValueKind::GeneratedValueList(
        RustOwnedGeneratedValueList { items },
    ))
}

fn component_values_satisfy_property_numeric_range(
    property_id: PropertyId,
    value_type: PropertyValueType,
    component_values: &[ComponentValue],
    primitive_value_options: CssPrimitiveValueOptions,
) -> bool {
    let Some(mut numeric_value) = style_value_numeric_value(value_type, component_values, primitive_value_options)
    else {
        return true;
    };
    numeric_value = normalize_css_numeric_token_value(value_type, numeric_value);
    let Some(range) = property_accepted_range_by_value_type(property_id, value_type) else {
        return true;
    };

    let (minimum, maximum) = numeric_range_to_f64(range, value_type);
    minimum <= numeric_value && numeric_value <= maximum
}

fn component_values_contain_dimension(component_values: &[ComponentValue]) -> bool {
    component_values.iter().any(|component_value| match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { .. },
            ..
        }) => true,
        ComponentValue::Function(function) => component_values_contain_dimension(&function.value),
        ComponentValue::SimpleBlock(block) => component_values_contain_dimension(&block.value),
        _ => false,
    })
}

fn component_values_contain_percentage(component_values: &[ComponentValue]) -> bool {
    component_values.iter().any(|component_value| match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { .. },
            ..
        }) => true,
        ComponentValue::Function(function) => component_values_contain_percentage(&function.value),
        ComponentValue::SimpleBlock(block) => component_values_contain_percentage(&block.value),
        _ => false,
    })
}

fn normalize_css_numeric_token_value(value_type: PropertyValueType, numeric_value: f64) -> f64 {
    // CSS numeric token payloads are ultimately materialized through C++ float-backed value types.
    // Keep Rust-owned primitive payloads inside the same boundary before range checks and FFI.
    if value_type == PropertyValueType::Integer {
        numeric_value.clamp(i32::MIN as f64, i32::MAX as f64)
    } else {
        numeric_value.clamp(f32::MIN as f64, f32::MAX as f64)
    }
}

pub(super) fn parse_rust_owned_generated_longhand_value(
    property_id: PropertyId,
    value_type: PropertyValueType,
    filtered_input: &[u8],
    component_values: &[ComponentValue],
) -> RustOwnedStyleValue {
    parse_rust_owned_generated_longhand_value_with_options(
        property_id,
        value_type,
        filtered_input,
        component_values,
        CssPrimitiveValueOptions::default(),
    )
}

pub(super) fn parse_rust_owned_generated_longhand_value_with_options(
    property_id: PropertyId,
    value_type: PropertyValueType,
    filtered_input: &[u8],
    component_values: &[ComponentValue],
    primitive_value_options: CssPrimitiveValueOptions,
) -> RustOwnedStyleValue {
    if value_type == PropertyValueType::Color
        && let [component_value] = component_values
        && let Some(color) =
            simple_color_from_component_value(component_value, primitive_value_options.allow_quirky_color)
    {
        return match color {
            ParsedSimpleColor::Rgba {
                red,
                green,
                blue,
                alpha,
                name,
            } => RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Color(RustOwnedColor::Simple {
                    kind: CssParsedColorKind::Rgba,
                    red,
                    green,
                    blue,
                    alpha,
                    name: name.map(ToString::to_string),
                }),
            },
            ParsedSimpleColor::Keyword { name } => RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Token {
                    primitive_kind: CssPrimitiveValueKind::Keyword,
                    numeric_value: None,
                    secondary_numeric_value: None,
                    value: name.to_string(),
                    value_type,
                }),
            },
        };
    }

    if value_type == PropertyValueType::Color
        && let [ComponentValue::Function(function)] = component_values
    {
        // https://drafts.csswg.org/css-color-4/#typedef-color
        // <color> = <absolute-color-base> | currentcolor | <system-color> | <contrast-color()> | <device-cmyk()>
        if component_value_parse_as_color_function(function)
            && let Some(source) =
                serialize_component_values_for_reparsing(component_values, &filtered_input_to_string(filtered_input))
        {
            return RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Color(RustOwnedColor::Source(source)),
            };
        }
    }

    if value_type == PropertyValueType::DashedIdent {
        let mut name = None;
        if parse_a_dashed_ident(filtered_input, |parsed_name| {
            name = Some(parsed_name.to_string());
        }) && let Some(name) = name
        {
            return RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Token {
                    primitive_kind: CssPrimitiveValueKind::CustomIdent,
                    numeric_value: None,
                    secondary_numeric_value: None,
                    value: name,
                    value_type,
                }),
            };
        }
    }

    if value_type == PropertyValueType::OpentypeTag {
        let mut tag = None;
        if parse_an_opentype_tag(filtered_input, |parsed_tag| {
            tag = Some(parsed_tag.to_string());
        }) && let Some(tag) = tag
        {
            return RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Primitive(RustOwnedPrimitiveValue::Token {
                    primitive_kind: CssPrimitiveValueKind::String,
                    numeric_value: None,
                    secondary_numeric_value: None,
                    value: tag,
                    value_type,
                }),
            };
        }
    }

    if value_type == PropertyValueType::CounterStyle
        && let Some(counter_style) =
            parse_all_component_values(filtered_input, ComponentValueParser::parse_a_counter_style)
    {
        return match counter_style {
            CounterStyle::Name(counter_style_name) => RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::Identifier(RustOwnedIdentifierValue::CounterStyleName(
                    counter_style_name,
                )),
            },
            CounterStyle::SymbolsFunction { .. } => RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::CounterStyle(counter_style),
            },
        };
    }

    if value_type == PropertyValueType::Counter
        && let Some(value) = rust_owned_counter_function_style_value_kind(filtered_input)
    {
        return RustOwnedStyleValue { property_id, value };
    }

    if value_type == PropertyValueType::CornerShape
        && let Some(value) = rust_owned_corner_shape_style_value_kind(filtered_input)
    {
        return RustOwnedStyleValue { property_id, value };
    }

    if value_type == PropertyValueType::Image
        && let Some(value) =
            rust_owned_image_style_value_kind(filtered_input, &filtered_input_to_string(filtered_input))
    {
        return RustOwnedStyleValue { property_id, value };
    }

    if value_type == PropertyValueType::Paint
        && let Some(value) = rust_owned_paint_style_value_kind(filtered_input)
    {
        return RustOwnedStyleValue { property_id, value };
    }

    if value_type == PropertyValueType::Anchor
        && let Some(value) = rust_owned_anchor_function_style_value_kind(filtered_input)
    {
        return RustOwnedStyleValue { property_id, value };
    }

    if value_type == PropertyValueType::Url
        && (property_id == PropertyId::ClipPath
            || (property_id == PropertyId::MaskImage && component_values_parse_as_fragment_url(filtered_input)))
    {
        return RustOwnedStyleValue {
            property_id,
            value: RustOwnedStyleValueKind::Url(rust_owned_url_from_source(&filtered_input_to_string(filtered_input))),
        };
    }

    match value_type {
        PropertyValueType::EasingFunction => {
            if let Some(value) =
                rust_owned_easing_function_style_value_kind(filtered_input, &filtered_input_to_string(filtered_input))
            {
                return RustOwnedStyleValue { property_id, value };
            }

            return RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::GuaranteedInvalid,
            };
        }
        PropertyValueType::FitContent => {
            if let Some(value) =
                rust_owned_fit_content_style_value_kind(filtered_input, &filtered_input_to_string(filtered_input))
            {
                return RustOwnedStyleValue { property_id, value };
            }

            return RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::GuaranteedInvalid,
            };
        }
        PropertyValueType::BasicShape => {
            if let Some(value) =
                rust_owned_basic_shape_style_value_kind(filtered_input, &filtered_input_to_string(filtered_input))
            {
                return RustOwnedStyleValue { property_id, value };
            }

            return RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::GuaranteedInvalid,
            };
        }
        PropertyValueType::Rect => {
            if let Some(value) =
                rust_owned_rect_style_value_kind(filtered_input, &filtered_input_to_string(filtered_input))
            {
                return RustOwnedStyleValue { property_id, value };
            }

            return RustOwnedStyleValue {
                property_id,
                value: RustOwnedStyleValueKind::GuaranteedInvalid,
            };
        }
        PropertyValueType::ScrollFunction => {
            let scroll_function = parse_scroll_function_value(filtered_input);
            if scroll_function.kind == CssScrollFunctionValueKind::Valid {
                return RustOwnedStyleValue {
                    property_id,
                    value: RustOwnedStyleValueKind::ScrollFunction(RustOwnedScrollFunction {
                        scroller: scroll_function.scroller,
                        axis: scroll_function.axis,
                    }),
                };
            }
        }
        PropertyValueType::ViewTimelineInset => {
            if let Some(values) = parse_rust_owned_view_timeline_inset_value(filtered_input) {
                return RustOwnedStyleValue {
                    property_id,
                    value: RustOwnedStyleValueKind::ViewTimelineInset(RustOwnedViewTimelineInset { values }),
                };
            }
        }
        PropertyValueType::ViewFunction => {
            let view_function = parse_view_function_value(filtered_input);
            if view_function.kind == CssViewFunctionValueKind::Valid {
                return RustOwnedStyleValue {
                    property_id,
                    value: RustOwnedStyleValueKind::ViewFunction(RustOwnedViewFunction {
                        axis: view_function.axis,
                        inset: view_function.inset,
                        inset_position: view_function.inset_position,
                    }),
                };
            }
        }
        _ => {}
    }

    if let Some(function) = parse_rust_owned_math_function(value_type, component_values, filtered_input) {
        return RustOwnedStyleValue {
            property_id,
            value: RustOwnedStyleValueKind::SourceBacked(function),
        };
    }

    if let Some(function) = parse_rust_owned_tree_counting_function(value_type, component_values, filtered_input) {
        return RustOwnedStyleValue {
            property_id,
            value: RustOwnedStyleValueKind::SourceBacked(function),
        };
    }

    if let Some(function) = parse_rust_owned_anchor_size_function(value_type, component_values, filtered_input) {
        return RustOwnedStyleValue {
            property_id,
            value: RustOwnedStyleValueKind::AnchorSize(function),
        };
    }

    let generated_style_value = generated_value_type_id_for_property_value_type(value_type).and_then(|value_type_id| {
        let syntax_kind = component_values_parse_as_generated_value_type(value_type_id, component_values);
        let style_value = generated_value_type_style_value(syntax_kind, component_values);
        if style_value.kind == GeneratedValueTypeStyleValueKind::Invalid {
            None
        } else {
            Some(style_value)
        }
    });
    let primitive_kind = if let Some(generated_style_value) = generated_style_value.as_ref() {
        match generated_style_value.kind {
            GeneratedValueTypeStyleValueKind::Invalid => CssPrimitiveValueKind::Invalid,
            GeneratedValueTypeStyleValueKind::Keyword => CssPrimitiveValueKind::Keyword,
            GeneratedValueTypeStyleValueKind::Number => CssPrimitiveValueKind::Number,
            GeneratedValueTypeStyleValueKind::String => CssPrimitiveValueKind::String,
            GeneratedValueTypeStyleValueKind::CustomIdent => CssPrimitiveValueKind::CustomIdent,
        }
    } else {
        style_value_primitive_kind(value_type, component_values, primitive_value_options)
    };
    let numeric_value = generated_style_value
        .as_ref()
        .and_then(|style_value| style_value.numeric_value)
        .or_else(|| style_value_numeric_value(value_type, component_values, primitive_value_options))
        .map(|numeric_value| normalize_css_numeric_token_value(value_type, numeric_value));
    let secondary_numeric_value = style_value_secondary_numeric_value(value_type, component_values);
    let value = if let Some(generated_style_value) = generated_style_value.as_ref()
        && let Some(value) = generated_style_value.value
    {
        value.to_string()
    } else if primitive_kind == CssPrimitiveValueKind::Ratio {
        if style_value_ratio_has_denominator(value_type, component_values) {
            "has-denominator".to_string()
        } else {
            String::new()
        }
    } else if primitive_kind == CssPrimitiveValueKind::String {
        string_token_value(component_values).unwrap_or("").to_string()
    } else if matches!(
        primitive_kind,
        CssPrimitiveValueKind::Angle
            | CssPrimitiveValueKind::Flex
            | CssPrimitiveValueKind::Frequency
            | CssPrimitiveValueKind::Length
            | CssPrimitiveValueKind::Resolution
            | CssPrimitiveValueKind::Time
    ) {
        style_value_dimension_unit(value_type, component_values, primitive_value_options)
            .unwrap_or("")
            .to_string()
    } else {
        String::new()
    };

    if primitive_kind == CssPrimitiveValueKind::Invalid {
        RustOwnedStyleValue {
            property_id,
            value: rust_owned_source_backed_style_value_kind(value_type, filtered_input_to_string(filtered_input)),
        }
    } else {
        RustOwnedStyleValue {
            property_id,
            value: rust_owned_primitive_style_value_kind(
                value_type,
                primitive_kind,
                numeric_value,
                secondary_numeric_value,
                value,
            ),
        }
    }
}

fn rust_owned_source_backed_style_value_kind(value_type: PropertyValueType, source: String) -> RustOwnedStyleValueKind {
    match value_type {
        PropertyValueType::Anchor => {
            rust_owned_anchor_function_style_value_kind(source.as_bytes()).unwrap_or_else(|| unreachable!())
        }
        PropertyValueType::Counter => {
            rust_owned_counter_function_style_value_kind(source.as_bytes()).unwrap_or_else(|| unreachable!())
        }
        PropertyValueType::Image => {
            rust_owned_image_style_value_kind(source.as_bytes(), &source).unwrap_or_else(|| unreachable!())
        }
        PropertyValueType::FontStyle => {
            rust_owned_font_style_style_value_kind(source).unwrap_or_else(|| unreachable!())
        }
        PropertyValueType::Position | PropertyValueType::BackgroundPosition => {
            rust_owned_position_style_value_kind(value_type, source).unwrap_or_else(|| unreachable!())
        }
        PropertyValueType::TransformList => {
            if let Some(value) = rust_owned_transform_list_style_value_kind(source.as_bytes(), &source) {
                value
            } else {
                unreachable!("valid <transform-list> should have a Rust-owned representation")
            }
        }
        PropertyValueType::FontVariantAlternates => {
            rust_owned_font_variant_alternates_style_value_kind(source).unwrap_or_else(|| unreachable!())
        }
        PropertyValueType::FontVariantEastAsian => {
            rust_owned_font_variant_east_asian_style_value_kind(source).unwrap_or_else(|| unreachable!())
        }
        PropertyValueType::FontVariantLigatures => {
            rust_owned_font_variant_ligatures_style_value_kind(source).unwrap_or_else(|| unreachable!())
        }
        PropertyValueType::FontVariantNumeric => {
            rust_owned_font_variant_numeric_style_value_kind(source).unwrap_or_else(|| unreachable!())
        }
        _ => RustOwnedStyleValueKind::GuaranteedInvalid,
    }
}

fn rust_owned_transform_list_style_value_kind(
    filtered_input: &[u8],
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-transforms-1/#typedef-transform-list
    // <transform-list> = <transform-function>+
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    if component_values.is_empty() {
        return None;
    }

    let mut values = Vec::new();
    for component_value in component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
    {
        let ComponentValue::Function(function) = component_value else {
            return None;
        };
        values.push(RustOwnedStyleValueKind::Transformation(
            rust_owned_transformation_style_value_kind(function, component_value, filtered_input_string)?,
        ));
    }

    Some(RustOwnedStyleValueKind::ValueList(RustOwnedStyleValueList {
        values,
        separator: RustOwnedStyleValueListSeparator::Space,
        value_type: Some(PropertyValueType::TransformList),
        source: Some(filtered_input_string.to_string()),
    }))
}

pub(super) fn rust_owned_anchor_function_style_value_kind(filtered_input: &[u8]) -> Option<RustOwnedStyleValueKind> {
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let [ComponentValue::Function(function)] = strip_whitespace(&component_values) else {
        return None;
    };

    rust_owned_anchor_function_from_function(function, filtered_input_string)
}

fn rust_owned_anchor_function_from_function(
    function: &Function,
    filtered_input_string: &str,
) -> Option<RustOwnedStyleValueKind> {
    // https://drafts.csswg.org/css-anchor-position-1/#funcdef-anchor
    // <anchor()> = anchor( <anchor-name>? && <anchor-side>, <length-percentage>? )
    if !function.name.eq_ignore_ascii_case("anchor") {
        return None;
    }

    let groups = split_component_values_on_comma(&function.value);
    if groups.is_empty() || groups.len() > 2 {
        return None;
    }

    let mut anchor_name = None;
    let mut anchor_side = None;
    for component_value in strip_whitespace(groups[0])
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
    {
        if let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) = component_value
            && value.starts_with("--")
            && is_valid_custom_ident(value, &[])
        {
            if anchor_name.is_some() {
                return None;
            }
            anchor_name = Some(value.clone());
            continue;
        }

        if anchor_side.is_some() || !component_value_parse_as_anchor_side(component_value) {
            return None;
        }
        anchor_side = Some(serialize_component_values_for_reparsing(
            std::slice::from_ref(component_value),
            filtered_input_string,
        )?);
    }

    let anchor_side = anchor_side?;
    let fallback = if groups.len() == 2 {
        let fallback = strip_whitespace(groups[1]);
        if !component_values_parse_as_anchor_fallback(fallback, filtered_input_string) {
            return None;
        }
        Some(serialize_component_values_for_reparsing(
            fallback,
            filtered_input_string,
        )?)
    } else {
        None
    };

    Some(RustOwnedStyleValueKind::Anchor(RustOwnedAnchorFunction {
        anchor_name,
        anchor_side,
        fallback,
        source: serialize_component_values_for_reparsing(
            &[ComponentValue::Function(function.clone())],
            filtered_input_string,
        )?,
    }))
}

fn parse_rust_owned_anchor_size_function(
    value_type: PropertyValueType,
    component_values: &[ComponentValue],
    filtered_input: &[u8],
) -> Option<RustOwnedAnchorSizeFunction> {
    if !matches!(
        value_type,
        PropertyValueType::Length | PropertyValueType::LengthPercentage
    ) {
        return None;
    }

    let [ComponentValue::Function(function)] = strip_whitespace(component_values) else {
        return None;
    };

    // https://drafts.csswg.org/css-anchor-position-1/#funcdef-anchor-size
    // anchor-size() = anchor-size( [ <anchor-name> || <anchor-size> ]? , <length-percentage>? )
    if !function.name.eq_ignore_ascii_case("anchor-size") {
        return None;
    }

    let filtered_input_string = filtered_input_to_string(filtered_input);
    // AD-HOC: Rust classifies the function shape here, while C++ still
    // validates the full grammar and property context during materialization.
    Some(RustOwnedAnchorSizeFunction {
        source: serialize_component_values_for_reparsing(
            &[ComponentValue::Function(function.clone())],
            &filtered_input_string,
        )?,
        value_type,
    })
}

fn component_value_parse_as_anchor_side(component_value: &ComponentValue) -> bool {
    // <anchor-side> = inside | outside
    //               | top | left | right | bottom
    //               | start | end | self-start | self-end
    //               | <percentage> | center
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) => matches!(
            value.to_ascii_lowercase().as_str(),
            "inside"
                | "outside"
                | "top"
                | "left"
                | "right"
                | "bottom"
                | "start"
                | "end"
                | "self-start"
                | "self-end"
                | "center"
        ),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { .. },
            ..
        }) => true,
        // AD-HOC: Match the existing C++ parser's calc handling for
        // anchor-side. It parses a length-percentage here to allow math
        // functions, then materializes and range-checks the value in C++.
        ComponentValue::Function(function) => is_math_function_name(&function.name),
        _ => false,
    }
}

fn component_values_parse_as_anchor_fallback(component_values: &[ComponentValue], filtered_input_string: &str) -> bool {
    let [component_value] = component_values else {
        return false;
    };
    component_value_parse_as_length_percentage(component_value)
        || matches!(
            component_value,
            ComponentValue::Function(function)
                if rust_owned_anchor_function_from_function(function, filtered_input_string).is_some()
        )
}

fn style_value_numeric_value(
    value_type: PropertyValueType,
    component_values: &[ComponentValue],
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<f64> {
    if value_type == PropertyValueType::Ratio {
        return style_value_ratio_values(value_type, component_values).map(|(numerator, _)| numerator);
    }

    let [ComponentValue::PreservedToken(token)] = component_values else {
        return None;
    };

    match (&token.token_type, value_type) {
        (TokenType::Number { number }, PropertyValueType::Integer) if number_is_integer(*number) => {
            Some(number.value())
        }
        (TokenType::Number { number }, PropertyValueType::Number | PropertyValueType::OpacityValue) => {
            Some(number.value())
        }
        (
            TokenType::Percentage { number },
            PropertyValueType::AnglePercentage
            | PropertyValueType::FrequencyPercentage
            | PropertyValueType::LengthPercentage
            | PropertyValueType::Percentage
            | PropertyValueType::TimePercentage
            | PropertyValueType::OpacityValue,
        ) => Some(number.value()),
        (
            TokenType::Dimension { number, .. },
            PropertyValueType::Angle
            | PropertyValueType::AnglePercentage
            | PropertyValueType::Flex
            | PropertyValueType::Frequency
            | PropertyValueType::FrequencyPercentage
            | PropertyValueType::Length
            | PropertyValueType::LengthPercentage
            | PropertyValueType::Resolution
            | PropertyValueType::Time
            | PropertyValueType::TimePercentage,
        ) => Some(number.value()),
        (TokenType::Number { number }, PropertyValueType::Length | PropertyValueType::LengthPercentage)
            if number.value() == 0.0
                || primitive_value_options.allow_quirky_length
                || primitive_value_options.allow_svg_unitless_length =>
        {
            Some(number.value())
        }
        _ => None,
    }
}

fn style_value_secondary_numeric_value(
    value_type: PropertyValueType,
    component_values: &[ComponentValue],
) -> Option<f64> {
    style_value_ratio_values(value_type, component_values).map(|(_, denominator)| denominator)
}

fn style_value_ratio_values(value_type: PropertyValueType, component_values: &[ComponentValue]) -> Option<(f64, f64)> {
    if value_type != PropertyValueType::Ratio {
        return None;
    }

    let component_values = strip_whitespace(component_values);
    match component_values {
        [
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Number { number },
                ..
            }),
        ] if number.value() >= 0.0 => Some((number.value(), 1.0)),
        _ => {
            let (slash_index, _) = component_values.iter().enumerate().find(|(_, component_value)| {
                matches!(
                    component_value,
                    ComponentValue::PreservedToken(Token {
                        token_type: TokenType::Delim { value },
                        ..
                    }) if *value == '/' as u32
                )
            })?;

            let numerator = strip_whitespace(&component_values[..slash_index]);
            let denominator = strip_whitespace(&component_values[slash_index + 1..]);
            let [
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Number { number: numerator },
                    ..
                }),
            ] = numerator
            else {
                return None;
            };
            let [
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Number { number: denominator },
                    ..
                }),
            ] = denominator
            else {
                return None;
            };

            (numerator.value() >= 0.0 && denominator.value() >= 0.0).then_some((numerator.value(), denominator.value()))
        }
    }
}

fn style_value_ratio_has_denominator(value_type: PropertyValueType, component_values: &[ComponentValue]) -> bool {
    if value_type != PropertyValueType::Ratio {
        return false;
    }

    style_value_ratio_values(value_type, component_values).is_some()
        && component_values_parse_as_ratio_with_denominator(component_values)
}

fn style_value_dimension_unit(
    value_type: PropertyValueType,
    component_values: &[ComponentValue],
    primitive_value_options: CssPrimitiveValueOptions,
) -> Option<&str> {
    let [ComponentValue::PreservedToken(token)] = component_values else {
        return None;
    };

    match (&token.token_type, value_type) {
        (
            TokenType::Dimension { unit, .. },
            PropertyValueType::Angle
            | PropertyValueType::AnglePercentage
            | PropertyValueType::Flex
            | PropertyValueType::Frequency
            | PropertyValueType::FrequencyPercentage
            | PropertyValueType::Length
            | PropertyValueType::LengthPercentage
            | PropertyValueType::Resolution
            | PropertyValueType::Time
            | PropertyValueType::TimePercentage,
        ) => Some(unit),
        (TokenType::Number { number }, PropertyValueType::Length | PropertyValueType::LengthPercentage)
            if number.value() == 0.0
                || primitive_value_options.allow_quirky_length
                || primitive_value_options.allow_svg_unitless_length =>
        {
            Some("px")
        }
        _ => None,
    }
}

fn style_value_primitive_kind(
    value_type: PropertyValueType,
    component_values: &[ComponentValue],
    primitive_value_options: CssPrimitiveValueOptions,
) -> CssPrimitiveValueKind {
    if value_type == PropertyValueType::Ratio {
        return if style_value_ratio_values(value_type, component_values).is_some() {
            CssPrimitiveValueKind::Ratio
        } else {
            CssPrimitiveValueKind::Invalid
        };
    }

    let [component_value] = component_values else {
        return CssPrimitiveValueKind::Invalid;
    };

    match value_type {
        PropertyValueType::Integer => parse_integer_value_prefix(component_value),
        PropertyValueType::Number => parse_number_value_prefix(component_value),
        PropertyValueType::Angle => parse_angle_value_prefix(component_value, primitive_value_options),
        PropertyValueType::AnglePercentage => {
            match parse_angle_value_prefix(component_value, primitive_value_options) {
                CssPrimitiveValueKind::Angle => CssPrimitiveValueKind::Angle,
                _ => parse_percentage_value_prefix(component_value),
            }
        }
        PropertyValueType::Flex => parse_flex_value_prefix(component_value),
        PropertyValueType::Frequency => parse_frequency_value_prefix(component_value),
        PropertyValueType::FrequencyPercentage => match parse_frequency_value_prefix(component_value) {
            CssPrimitiveValueKind::Frequency => CssPrimitiveValueKind::Frequency,
            _ => parse_percentage_value_prefix(component_value),
        },
        PropertyValueType::Length => parse_length_value_prefix(component_value, primitive_value_options),
        PropertyValueType::LengthPercentage => {
            match parse_length_value_prefix(component_value, primitive_value_options) {
                CssPrimitiveValueKind::Length => CssPrimitiveValueKind::Length,
                _ => parse_percentage_value_prefix(component_value),
            }
        }
        PropertyValueType::Resolution => parse_resolution_value_prefix(component_value),
        PropertyValueType::Time => parse_time_value_prefix(component_value),
        PropertyValueType::TimePercentage => match parse_time_value_prefix(component_value) {
            CssPrimitiveValueKind::Time => CssPrimitiveValueKind::Time,
            _ => parse_percentage_value_prefix(component_value),
        },
        PropertyValueType::Percentage => parse_percentage_value_prefix(component_value),
        PropertyValueType::String => parse_string_value_prefix(component_value),
        PropertyValueType::OpacityValue => match parse_number_value_prefix(component_value) {
            CssPrimitiveValueKind::Number => CssPrimitiveValueKind::Number,
            _ => parse_percentage_value_prefix(component_value),
        },
        _ => CssPrimitiveValueKind::Invalid,
    }
}

fn string_token_value(component_values: &[ComponentValue]) -> Option<&str> {
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

pub(super) fn generated_property_value_type_order() -> &'static [PropertyValueType] {
    // This follows Parser::parse_css_value_for_properties(), which is ordered to
    // preserve CSS grammar precedence, for example <integer>/<number> before
    // <length> so a unitless zero is not captured as a length when both are
    // accepted.
    &[
        PropertyValueType::Anchor,
        PropertyValueType::Color,
        PropertyValueType::CornerShape,
        PropertyValueType::Counter,
        PropertyValueType::CounterStyle,
        PropertyValueType::DashedIdent,
        PropertyValueType::EasingFunction,
        PropertyValueType::FontStyle,
        PropertyValueType::FontKerningValue,
        PropertyValueType::FontOpticalSizingValue,
        PropertyValueType::FontWeightAbsolute,
        PropertyValueType::FontWidthCss3,
        PropertyValueType::FontVariantAlternates,
        PropertyValueType::FontVariantCapsValue,
        PropertyValueType::FontVariantEastAsian,
        PropertyValueType::FontVariantEmojiValue,
        PropertyValueType::FontVariantLigatures,
        PropertyValueType::FontVariantNumeric,
        PropertyValueType::FontVariantPositionValue,
        PropertyValueType::Image,
        PropertyValueType::Position,
        PropertyValueType::BackgroundPosition,
        PropertyValueType::BasicShape,
        PropertyValueType::Ratio,
        PropertyValueType::OpacityValue,
        PropertyValueType::OpentypeTag,
        PropertyValueType::Rect,
        PropertyValueType::ScrollFunction,
        PropertyValueType::String,
        PropertyValueType::TransformList,
        PropertyValueType::Url,
        PropertyValueType::ViewFunction,
        PropertyValueType::ViewTimelineInset,
        PropertyValueType::Integer,
        PropertyValueType::Number,
        PropertyValueType::FitContent,
        PropertyValueType::Angle,
        PropertyValueType::AnglePercentage,
        PropertyValueType::Flex,
        PropertyValueType::Frequency,
        PropertyValueType::FrequencyPercentage,
        PropertyValueType::Length,
        PropertyValueType::LengthPercentage,
        PropertyValueType::Resolution,
        PropertyValueType::Time,
        PropertyValueType::TimePercentage,
        PropertyValueType::Percentage,
        PropertyValueType::Paint,
    ]
}

pub(super) fn component_values_parse_as_property_value_type(
    value_type: PropertyValueType,
    filtered_input: &[u8],
) -> bool {
    component_values_parse_as_property_value_type_with_options(
        value_type,
        filtered_input,
        CssPrimitiveValueOptions::default(),
    )
}

pub(super) fn component_values_parse_as_property_value_type_with_options(
    value_type: PropertyValueType,
    filtered_input: &[u8],
    primitive_value_options: CssPrimitiveValueOptions,
) -> bool {
    match value_type {
        PropertyValueType::Anchor => rust_owned_anchor_function_style_value_kind(filtered_input).is_some(),
        PropertyValueType::Color => {
            parse_color_value(filtered_input, primitive_value_options.allow_quirky_color) == CssColorValueKind::Valid
        }
        PropertyValueType::Counter => rust_owned_counter_function_style_value_kind(filtered_input).is_some(),
        PropertyValueType::CornerShape => rust_owned_corner_shape_style_value_kind(filtered_input).is_some(),
        PropertyValueType::DashedIdent => parse_a_dashed_ident(filtered_input, |_| {}),
        PropertyValueType::EasingFunction => parse_easing_value(filtered_input) == CssEasingValueKind::Valid,
        PropertyValueType::FitContent => parse_fit_content_value(filtered_input) == CssFitContentValueKind::Valid,
        PropertyValueType::BasicShape => parse_basic_shape_value(filtered_input) == CssBasicShapeValueKind::Valid,
        PropertyValueType::CounterStyle => parse_a_counter_style(filtered_input, |_, _, _| {}, |_| {}),
        PropertyValueType::FontStyle => parse_a_font_style(filtered_input, |_| {}),
        PropertyValueType::FontVariantAlternates => parse_a_font_variant_alternates(filtered_input, |_| {}, |_| {}),
        PropertyValueType::FontVariantEastAsian => parse_a_font_variant_east_asian(filtered_input, |_| {}),
        PropertyValueType::FontVariantLigatures => parse_a_font_variant_ligatures(filtered_input, |_| {}),
        PropertyValueType::FontVariantNumeric => parse_a_font_variant_numeric(filtered_input, |_| {}),
        PropertyValueType::Position => parse_position_value(filtered_input, false) == CssPositionValueKind::Valid,
        PropertyValueType::BackgroundPosition => {
            parse_position_value(filtered_input, true) == CssPositionValueKind::Valid
        }
        PropertyValueType::Ratio => parse_ratio_value_prefix(filtered_input).kind == CssRatioValueKind::Valid,
        PropertyValueType::OpacityValue => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Opacity, primitive_value_options)
                == CssPrimitiveValueKind::Opacity
        }
        PropertyValueType::OpentypeTag => parse_an_opentype_tag(filtered_input, |_| {}),
        PropertyValueType::Rect => parse_rect_value(filtered_input) == CssRectValueKind::Valid,
        PropertyValueType::ScrollFunction => {
            parse_scroll_function_value(filtered_input).kind == CssScrollFunctionValueKind::Valid
        }
        PropertyValueType::String => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::String, primitive_value_options)
                == CssPrimitiveValueKind::String
        }
        PropertyValueType::TransformList => parse_transform_list_value(filtered_input),
        // AD-HOC: Keep <url> on the C++ fallback path until Rust owns <image>
        // materialization as well. Some properties accept both <image> and <url>,
        // and C++ deliberately parses non-fragment url() values as images first.
        PropertyValueType::Url => false,
        PropertyValueType::ViewFunction => {
            parse_view_function_value(filtered_input).kind == CssViewFunctionValueKind::Valid
        }
        PropertyValueType::ViewTimelineInset => {
            parse_view_timeline_inset_value(filtered_input).kind == CssViewTimelineInsetValueKind::Valid
        }
        PropertyValueType::Integer => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Integer, primitive_value_options)
                == CssPrimitiveValueKind::Integer
        }
        PropertyValueType::Number => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Number, primitive_value_options)
                == CssPrimitiveValueKind::Number
        }
        PropertyValueType::Angle => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Angle, primitive_value_options)
                == CssPrimitiveValueKind::Angle
        }
        PropertyValueType::AnglePercentage => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Angle, primitive_value_options)
                == CssPrimitiveValueKind::Angle
                || parse_primitive_value(
                    filtered_input,
                    CssPrimitiveValueType::Percentage,
                    primitive_value_options,
                ) == CssPrimitiveValueKind::Percentage
        }
        PropertyValueType::Flex => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Flex, primitive_value_options)
                == CssPrimitiveValueKind::Flex
        }
        PropertyValueType::Frequency => {
            parse_primitive_value(
                filtered_input,
                CssPrimitiveValueType::Frequency,
                primitive_value_options,
            ) == CssPrimitiveValueKind::Frequency
        }
        PropertyValueType::FrequencyPercentage => {
            parse_primitive_value(
                filtered_input,
                CssPrimitiveValueType::Frequency,
                primitive_value_options,
            ) == CssPrimitiveValueKind::Frequency
                || parse_primitive_value(
                    filtered_input,
                    CssPrimitiveValueType::Percentage,
                    primitive_value_options,
                ) == CssPrimitiveValueKind::Percentage
        }
        PropertyValueType::Length => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Length, primitive_value_options)
                == CssPrimitiveValueKind::Length
        }
        PropertyValueType::LengthPercentage => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Length, primitive_value_options)
                == CssPrimitiveValueKind::Length
                || parse_primitive_value(
                    filtered_input,
                    CssPrimitiveValueType::Percentage,
                    primitive_value_options,
                ) == CssPrimitiveValueKind::Percentage
        }
        PropertyValueType::Resolution => {
            parse_primitive_value(
                filtered_input,
                CssPrimitiveValueType::Resolution,
                primitive_value_options,
            ) == CssPrimitiveValueKind::Resolution
        }
        PropertyValueType::Time => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Time, primitive_value_options)
                == CssPrimitiveValueKind::Time
        }
        PropertyValueType::TimePercentage => {
            parse_primitive_value(filtered_input, CssPrimitiveValueType::Time, primitive_value_options)
                == CssPrimitiveValueKind::Time
                || parse_primitive_value(
                    filtered_input,
                    CssPrimitiveValueType::Percentage,
                    primitive_value_options,
                ) == CssPrimitiveValueKind::Percentage
        }
        PropertyValueType::Percentage => {
            parse_primitive_value(
                filtered_input,
                CssPrimitiveValueType::Percentage,
                primitive_value_options,
            ) == CssPrimitiveValueKind::Percentage
        }
        PropertyValueType::FontKerningValue => {
            component_values_parse_as_generated_property_value_type(ValueTypeId::FontKerningValue, filtered_input)
        }
        PropertyValueType::FontOpticalSizingValue => {
            component_values_parse_as_generated_property_value_type(ValueTypeId::FontOpticalSizingValue, filtered_input)
        }
        PropertyValueType::FontWeightAbsolute => {
            component_values_parse_as_generated_property_value_type(ValueTypeId::FontWeightAbsolute, filtered_input)
        }
        PropertyValueType::FontWidthCss3 => {
            component_values_parse_as_generated_property_value_type(ValueTypeId::FontWidthCss3, filtered_input)
        }
        PropertyValueType::FontVariantCapsValue => {
            component_values_parse_as_generated_property_value_type(ValueTypeId::FontVariantCapsValue, filtered_input)
        }
        PropertyValueType::FontVariantEmojiValue => {
            component_values_parse_as_generated_property_value_type(ValueTypeId::FontVariantEmojiValue, filtered_input)
        }
        PropertyValueType::FontVariantPositionValue => component_values_parse_as_generated_property_value_type(
            ValueTypeId::FontVariantPositionValue,
            filtered_input,
        ),
        PropertyValueType::Image => {
            rust_owned_image_style_value_kind(filtered_input, &filtered_input_to_string(filtered_input)).is_some()
        }
        PropertyValueType::Paint => rust_owned_paint_style_value_kind(filtered_input).is_some(),
        _ => false,
    }
}

fn generated_value_type_id_for_property_value_type(value_type: PropertyValueType) -> Option<ValueTypeId> {
    match value_type {
        PropertyValueType::FontKerningValue => Some(ValueTypeId::FontKerningValue),
        PropertyValueType::FontOpticalSizingValue => Some(ValueTypeId::FontOpticalSizingValue),
        PropertyValueType::FontWeightAbsolute => Some(ValueTypeId::FontWeightAbsolute),
        PropertyValueType::FontWidthCss3 => Some(ValueTypeId::FontWidthCss3),
        PropertyValueType::FontVariantCapsValue => Some(ValueTypeId::FontVariantCapsValue),
        PropertyValueType::FontVariantEmojiValue => Some(ValueTypeId::FontVariantEmojiValue),
        PropertyValueType::FontVariantPositionValue => Some(ValueTypeId::FontVariantPositionValue),
        _ => None,
    }
}

pub(super) fn component_values_parse_as_generated_property_value_type(
    value_type_id: ValueTypeId,
    filtered_input: &[u8],
) -> bool {
    parse_a_value_type(filtered_input, value_type_id as u8) != CssValueTypeSyntaxKind::Invalid
}

fn parse_transform_list_value(filtered_input: &[u8]) -> bool {
    // https://drafts.csswg.org/css-transforms-1/#typedef-transform-list
    // <transform-list> = <transform-function>+
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);
    if component_values.is_empty() {
        return false;
    }

    component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value))
        .all(|component_value| {
            let ComponentValue::Function(function) = component_value else {
                return false;
            };
            component_value_parse_as_transform_function(function)
        })
}

fn component_value_parse_as_transform_function(function: &Function) -> bool {
    let Some(parameters) = transform_function_parameters_from_name(&function.name) else {
        return false;
    };

    let Some(arguments) = parse_comma_separated_component_values(function.value.clone(), |component_values| {
        let [component_value] = strip_whitespace(&component_values) else {
            return None;
        };
        Some(component_value.clone())
    }) else {
        return false;
    };

    if arguments.len() > parameters.len() {
        return false;
    }
    if arguments.len() < parameters.len() && parameters[arguments.len()].required {
        return false;
    }

    arguments.iter().zip(parameters).all(|(argument, parameter)| {
        component_value_matches_transform_function_parameter(argument, parameter.parameter_type)
    })
}
