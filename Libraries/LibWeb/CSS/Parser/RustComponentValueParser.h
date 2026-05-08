/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/HashTable.h>
#include <AK/Optional.h>
#include <AK/OwnPtr.h>
#include <AK/StringView.h>
#include <AK/Variant.h>
#include <AK/Vector.h>
#include <LibGfx/Font/UnicodeRange.h>
#include <LibWeb/CSS/BooleanExpression.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/MediaQuery.h>
#include <LibWeb/CSS/NumericRange.h>
#include <LibWeb/CSS/PageSelector.h>
#include <LibWeb/CSS/Parser/ComponentValue.h>
#include <LibWeb/CSS/Parser/RuleContext.h>
#include <LibWeb/CSS/Parser/SyntaxParsing.h>
#include <LibWeb/CSS/Parser/TokenStream.h>
#include <LibWeb/CSS/Parser/Types.h>
#include <LibWeb/CSS/Percentage.h>
#include <LibWeb/CSS/Selector.h>
#include <LibWeb/CSS/StyleValues/CounterDefinitionsStyleValue.h>
#include <LibWeb/CSS/URL.h>
#include <LibWeb/CSS/ValueType.h>
#include <LibWeb/Export.h>
#include <LibWeb/RustFFI.h>

namespace Web::CSS::Parser {

class WEB_API RustComponentValueParser {
public:
    struct MediaFeatureTest {
        FFI::CssMediaFeature feature;
        FFI::CssMediaFeatureValueSyntaxKind value_syntax_kind;
        FFI::CssMediaFeatureValueSyntaxKind left_value_syntax_kind;
        FFI::CssMediaFeatureValueSyntaxKind right_value_syntax_kind;
        Vector<ComponentValue> value;
        Vector<ComponentValue> left_value;
        Vector<ComponentValue> right_value;
    };

    struct MediaQuerySyntax {
        bool is_negated { false };
        Optional<MediaQuery::MediaType> media_type;
        OwnPtr<BooleanExpression> media_condition;
    };

    struct SupportsFeature {
        FFI::CssSupportsFeatureKind kind;
        Optional<FlyString> name;
    };

    struct PropertyKeyword {
        PropertyID property_id;
        Keyword keyword;
    };

    struct PropertyCustomIdent {
        PropertyID property_id;
        FlyString custom_ident;
    };

    struct GeneratedPropertyValue {
        FFI::CssGeneratedPropertyValueKind kind;
        PropertyID property_id;
        Optional<Keyword> keyword;
        Optional<FlyString> custom_ident;
        Optional<ValueType> value_type;
    };

    enum class RustDisplayValueKind : u8 {
        Invalid,
        Box,
        Internal,
        OutsideAndInside,
    };

    enum class RustDisplayInside : u8 {
        Flow,
        FlowRoot,
        Table,
        Flex,
        Grid,
        Ruby,
        Math,
    };

    enum class RustDisplayListItem : u8 {
        No,
        Yes,
    };

    struct FontVariantAlternatesValue {
        FFI::CssFontVariantAlternatesValueKind kind;
        Vector<FlyString> feature_value_names;
    };

    struct FontVariantEastAsianValue {
        FFI::CssFontVariantEastAsianValueKind kind;
        FlyString value;
    };

    struct FontVariantNumericValue {
        FFI::CssFontVariantNumericValueKind kind;
        FlyString value;
    };

    struct FontVariantLigaturesValue {
        FFI::CssFontVariantLigaturesValueKind kind;
        FlyString value;
    };

    struct FontFamilyValue {
        FFI::CssFontFamilyValueKind kind;
        FlyString value;
        bool is_string { false };
    };

    struct FontStyle {
        FFI::CssFontStyleKind kind;
        bool has_angle { false };
    };

    struct OpenTypeTaggedValue {
        FlyString tag;
        FFI::CssOpenTypeTaggedValueKind value_kind;
        Optional<String> value;
    };

    enum class RustShadowPlacement : u8 {
        Outer,
        Inner,
    };

    struct RustShadow {
        Optional<String> color_source;
        String offset_x_source;
        String offset_y_source;
        Optional<String> blur_radius_source;
        Optional<String> spread_distance_source;
        RustShadowPlacement placement { RustShadowPlacement::Outer };
    };

    enum class RustPositionTryFallbackKind : u8 {
        PositionArea,
        TryTactic,
    };

    struct RustPositionArea {
        FlyString first_keyword;
        Optional<FlyString> second_keyword;
    };

    struct RustPositionTryFallback {
        RustPositionTryFallbackKind kind { RustPositionTryFallbackKind::PositionArea };
        RustPositionArea position_area;
        Optional<FlyString> dashed_ident;
        bool has_flip_block { false };
        bool has_flip_inline { false };
        bool has_flip_start { false };
        Vector<FlyString> try_tactics;
    };

    enum class RustGridTrackPlacementKind : u8 {
        Auto,
        Line,
        Span,
    };

    struct RustGridTrackPlacement {
        RustGridTrackPlacementKind kind { RustGridTrackPlacementKind::Auto };
        Optional<String> line_number_source;
        Optional<String> name;
    };

    enum class RustGridTrackSizeListEventKind : u8 {
        None,
        LineNames,
        Breadth,
        MinMax,
        FitContent,
        RepeatBegin,
        RepeatEnd,
    };

    enum class RustGridRepeatType : u8 {
        AutoFill,
        AutoFit,
        Fixed,
    };

    struct RustGridTrackSizeListEvent {
        RustGridTrackSizeListEventKind kind { RustGridTrackSizeListEventKind::None };
        RustGridRepeatType repeat_type { RustGridRepeatType::AutoFill };
        String source;
        String secondary_source;
    };

    struct RustCursorImage {
        String image_source;
        Optional<String> x_source;
        Optional<String> y_source;
    };

    enum class RustFilterValueListEventKind : u8 {
        None,
        Url,
        Blur,
        DropShadow,
        HueRotate,
        Simple,
        DropShadowRadius,
        DropShadowColor,
    };

    enum class RustSimpleFilterFunction : u8 {
        Brightness,
        Contrast,
        Grayscale,
        Invert,
        Opacity,
        Saturate,
        Sepia,
    };

    enum class RustTransformLonghandFunction : u8 {
        Rotate,
        RotateX,
        RotateY,
        RotateZ,
        Rotate3d,
        Translate,
        Translate3d,
        Scale,
        Scale3d,
    };

    enum class RustFontVariantEventKind : u8 {
        Normal,
        Simple,
        AlternatesValue,
        AlternatesFeatureValueName,
        EastAsianValue,
        NumericValue,
        LigaturesValue,
    };

    enum class RustContentEventKind : u8 {
        Normal,
        None,
        ItemQuote,
        ItemString,
        ItemImage,
        ItemCounter,
        AltTextString,
        AltTextCounter,
    };

    enum class RustShapeOutsideEventKind : u8 {
        None,
        Image,
        BasicShape,
        ShapeBox,
    };

    enum class RustBasicShapeKind : u8 {
        Inset,
        Xywh,
        Rect,
        Circle,
        Ellipse,
        Polygon,
        Path,
    };

    enum class RustFitContentKind : u8 {
        Keyword,
        Function,
    };

    struct RustContentEvent {
        RustContentEventKind kind { RustContentEventKind::Normal };
        String source;
    };

    struct RustFilterValueListEvent {
        RustFilterValueListEventKind kind { RustFilterValueListEventKind::None };
        RustSimpleFilterFunction simple_function { RustSimpleFilterFunction::Brightness };
        bool has_value { false };
        bool has_secondary_value { false };
        String source;
        String secondary_source;
        Optional<String> drop_shadow_radius_source;
        Optional<String> drop_shadow_color_source;
    };

    struct RustStyleValue {
        FFI::CssStyleValueKind kind;
        PropertyID property_id;
        FFI::CssPrimitiveValueKind primitive_kind { FFI::CssPrimitiveValueKind::Invalid };
        Optional<Keyword> keyword;
        Optional<FlyString> custom_ident;
        Optional<ValueType> value_type;
        Optional<double> numeric_value;
        Optional<double> secondary_numeric_value;
        bool ratio_has_denominator { false };
        u8 color_red { 0 };
        u8 color_green { 0 };
        u8 color_blue { 0 };
        u8 color_alpha { 0 };
        Optional<FlyString> dimension_unit;
        Optional<FlyString> string;
        FFI::CssScrollFunctionScrollerKind scroll_function_scroller { FFI::CssScrollFunctionScrollerKind::None };
        FFI::CssScrollFunctionAxisKind scroll_function_axis { FFI::CssScrollFunctionAxisKind::None };
        bool stroke_dasharray_none { false };
        Vector<String> stroke_dasharray_values;
        String place_align_source;
        String place_justify_source;
        String overflow_clip_margin_source;
        Optional<String> column_count_source;
        Optional<String> column_width_source;
        Optional<String> column_height_source;
        bool flex_shorthand_is_none { false };
        Optional<String> flex_grow_source;
        Optional<String> flex_shrink_source;
        Optional<String> flex_basis_source;
        Optional<String> flex_direction_source;
        Optional<String> flex_wrap_source;
        Optional<String> text_decoration_line_source;
        Optional<String> text_decoration_thickness_source;
        Optional<String> text_decoration_style_source;
        Optional<String> text_decoration_color_source;
        Optional<String> list_style_position_source;
        Optional<String> list_style_image_source;
        Optional<String> list_style_type_source;
        Optional<String> math_depth_integer_source;
        bool aspect_ratio_has_auto { false };
        Optional<String> aspect_ratio_numerator_source;
        Optional<String> aspect_ratio_denominator_source;
        Vector<String> border_radius_horizontal_sources;
        Vector<String> border_radius_vertical_sources;
        Optional<String> border_width_source;
        Optional<String> border_style_source;
        Optional<String> border_color_source;
        Optional<String> border_image_source_source;
        Optional<String> border_image_shorthand_slice_source;
        Optional<String> border_image_shorthand_width_source;
        Optional<String> border_image_shorthand_outset_source;
        Optional<String> border_image_shorthand_repeat_source;
        Vector<String> border_image_outset_sources;
        Vector<String> border_image_repeat_sources;
        Vector<String> border_image_slice_sources;
        bool border_image_slice_fill { false };
        Vector<String> border_image_width_sources;
        FFI::CssContainValue contain {};
        FFI::CssContainerTypeValueKind container_type { FFI::CssContainerTypeValueKind::Invalid };
        Vector<CounterDefinition> counter_definitions;
        Vector<String> counter_definition_value_sources;
        Vector<String> background_size_sources;
        u8 easing_function_kind { 0 };
        Vector<String> easing_function_sources;
        RustBasicShapeKind basic_shape_kind { RustBasicShapeKind::Inset };
        Vector<String> basic_shape_argument_groups;
        RustFitContentKind fit_content_kind { RustFitContentKind::Keyword };
        Optional<String> fit_content_argument_source;
        Vector<String> rect_sources;
        bool rect_requires_commas { false };
        RustDisplayValueKind display_kind { RustDisplayValueKind::Invalid };
        u8 display_value { 0 };
        RustDisplayInside display_inside { RustDisplayInside::Flow };
        RustDisplayListItem display_list_item { RustDisplayListItem::No };
        FFI::CssAnchorNameOrScopeValueKind anchor_name_or_scope_kind { FFI::CssAnchorNameOrScopeValueKind::Invalid };
        Vector<FlyString> anchor_names;
        FFI::CssAnimationNameValueKind animation_name_kind { FFI::CssAnimationNameValueKind::Invalid };
        Vector<FFI::CssAnimationNameItemKind> animation_name_item_kinds;
        Vector<FlyString> animation_names;
        FFI::CssColorSchemeValueKind color_scheme_kind { FFI::CssColorSchemeValueKind::Invalid };
        bool color_scheme_only { false };
        Vector<String> color_scheme_schemes;
        Vector<FontFamilyValue> font_family;
        FFI::CssFontLanguageOverrideKind font_language_override_kind { FFI::CssFontLanguageOverrideKind::Normal };
        Optional<FlyString> font_language_override;
        FontStyle font_style;
        Optional<String> font_style_angle;
        FFI::CssOpenTypeSettingsKind open_type_settings_kind { FFI::CssOpenTypeSettingsKind::Normal };
        Vector<OpenTypeTaggedValue> open_type_tag_values;
        Vector<FontVariantAlternatesValue> font_variant_alternates;
        Vector<FontVariantEastAsianValue> font_variant_east_asian;
        Vector<FontVariantLigaturesValue> font_variant_ligatures;
        Vector<FontVariantNumericValue> font_variant_numeric;
        Optional<FlyString> font_variant_caps;
        Optional<FlyString> font_variant_emoji;
        Optional<FlyString> font_variant_position;
        bool font_variant_ligatures_none { false };
        Optional<Keyword> content_keyword;
        Vector<RustContentEvent> content_events;
        bool shape_outside_is_none { false };
        Optional<String> shape_outside_image_source;
        Optional<RustBasicShapeKind> shape_outside_basic_shape_kind;
        Vector<String> shape_outside_basic_shape_argument_groups;
        Optional<String> shape_outside_shape_box_source;
        u8 grid_auto_flow_axis { 0 };
        u8 grid_auto_flow_dense { 0 };
        FFI::CssPaintOrderValue paint_order {};
        Vector<String> position_sources;
        FFI::CssPositionAnchorValueKind position_anchor_kind { FFI::CssPositionAnchorValueKind::Invalid };
        FlyString position_anchor_name;
        FFI::CssPositionTryOrderValue position_try_order { FFI::CssPositionTryOrderValue::Invalid };
        FFI::CssPositionVisibilityValue position_visibility {};
        Vector<u8> repeat_x_values;
        Vector<u8> repeat_y_values;
        u8 scrollbar_color_kind { 0 };
        FFI::CssScrollbarGutterValueKind scrollbar_gutter { FFI::CssScrollbarGutterValueKind::Invalid };
        FFI::CssTextWrapValue text_wrap {};
        FFI::CssTextWrapModeValue text_wrap_mode { FFI::CssTextWrapModeValue::Invalid };
        FFI::CssTextWrapStyleValue text_wrap_style { FFI::CssTextWrapStyleValue::Invalid };
        bool text_indent_has_hanging { false };
        bool text_indent_has_each_line { false };
        FFI::CssTextUnderlinePositionHorizontal text_underline_position_horizontal { FFI::CssTextUnderlinePositionHorizontal::Invalid };
        FFI::CssTextUnderlinePositionVertical text_underline_position_vertical { FFI::CssTextUnderlinePositionVertical::Invalid };
        bool shadow_is_none { false };
        Vector<RustShadow> shadows;
        Vector<RustCursorImage> cursor_images;
        Optional<FlyString> cursor_predefined;
        bool filter_value_list_is_none { false };
        Vector<RustFilterValueListEvent> filter_value_list_events;
        bool position_area_is_none { false };
        Optional<RustPositionArea> position_area;
        bool position_try_fallbacks_is_none { false };
        Vector<RustPositionTryFallback> position_try_fallbacks;
        bool grid_template_areas_is_none { false };
        Vector<String> grid_template_area_rows;
        Optional<RustGridTrackPlacement> grid_track_placement;
        bool grid_track_size_list_is_none { false };
        Vector<RustGridTrackSizeListEvent> grid_track_size_list_events;
        bool transform_longhand_is_none { false };
        Optional<RustTransformLonghandFunction> transform_longhand_function;
        Vector<String> transform_longhand_arguments;
        Optional<String> transform_origin_x_source;
        Optional<String> transform_origin_y_source;
        Optional<String> transform_origin_z_source;
        FFI::CssTimelineNameValueKind timeline_name_kind { FFI::CssTimelineNameValueKind::Invalid };
        Vector<FFI::CssTimelineNameItemKind> timeline_name_item_kinds;
        Vector<FlyString> timeline_names;
        Vector<FFI::CssScrollFunctionAxisKind> scroll_timeline_axes;
        FFI::CssTimelineScopeValueKind timeline_scope_kind { FFI::CssTimelineScopeValueKind::Invalid };
        Vector<FlyString> timeline_scope_names;
        FFI::CssTouchActionValue touch_action {};
        Vector<FFI::CssTransitionBehaviorItemKind> transition_behaviors;
        FFI::CssTransitionPropertyValueKind transition_property_kind { FFI::CssTransitionPropertyValueKind::Invalid };
        Vector<FlyString> transition_properties;
        Vector<u8> view_timeline_inset_source_counts;
        Vector<String> view_timeline_inset_sources;
        FFI::CssViewFunctionInsetKind view_function_inset { FFI::CssViewFunctionInsetKind::None };
        FFI::CssViewFunctionInsetPosition view_function_inset_position { FFI::CssViewFunctionInsetPosition::None };
        FFI::CssViewTransitionNameValueKind view_transition_name_kind { FFI::CssViewTransitionNameValueKind::Invalid };
        FlyString view_transition_name;
        FlyString white_space_collapse;
        FFI::CssWhiteSpaceTrimValue white_space_trim {};
        FFI::CssQuotesValueKind quotes_kind { FFI::CssQuotesValueKind::Invalid };
        Vector<FlyString> quotes_strings;
        FFI::CssWillChangeValueKind will_change_kind { FFI::CssWillChangeValueKind::Invalid };
        Vector<FFI::CssWillChangeFeatureKind> will_change_feature_kinds;
        Vector<FlyString> will_change_features;
    };

    struct SimpleColor {
        FFI::CssParsedColorKind kind { FFI::CssParsedColorKind::Invalid };
        u8 red { 0 };
        u8 green { 0 };
        u8 blue { 0 };
        u8 alpha { 0 };
        Optional<FlyString> name;
    };

    struct CoordinatingValueListShorthandItem {
        size_t layer_index { 0 };
        PropertyID property_id;
        String value;
    };

    using LayerShorthandItem = CoordinatingValueListShorthandItem;

    struct FontShorthandItem {
        PropertyID property_id;
        String value;
    };

    using GridPlacementShorthandItem = FontShorthandItem;
    using GridTemplateShorthandItem = FontShorthandItem;

    struct PositionalValueListShorthandItem {
        size_t index { 0 };
        String value;
    };

    struct PropertyNumericMetadata {
        PropertyID property_id;
        NumericRange range;
        Optional<NumericRange> percentage_range;
        bool percentages_resolve_to_value_type { false };
    };

    struct NamespaceRulePrelude {
        Optional<FlyString> prefix;
        FlyString namespace_uri;
    };

    struct ImportRulePrelude {
        URL url;
        Optional<FlyString> layer;
        Optional<String> supports;
        String media_query_list;
    };

    struct ContainerRulePreludeCondition {
        Optional<FlyString> name;
        Optional<String> query;
    };

    struct FamilyName {
        FlyString name;
        bool is_string { false };
    };

    struct FontSource {
        Variant<FamilyName, URL> source;
        Optional<FlyString> format;
        Vector<FontTech> tech;
    };

    struct ScrollFunction {
        FFI::CssScrollFunctionValueKind kind;
        FFI::CssScrollFunctionScrollerKind scroller;
        FFI::CssScrollFunctionAxisKind axis;
    };

    struct ViewTimelineInset {
        FFI::CssViewTimelineInsetValueKind kind;
        size_t count { 0 };
    };

    struct ViewFunction {
        FFI::CssViewFunctionValueKind kind;
        FFI::CssScrollFunctionAxisKind axis;
        FFI::CssViewFunctionInsetKind inset;
        FFI::CssViewFunctionInsetPosition inset_position;
    };

    struct TimelineScope {
        FFI::CssTimelineScopeValueKind kind;
        Vector<FlyString> names;
    };

    enum class AllowBlankLayerName : u8 {
        No,
        Yes,
    };

    enum class SelectorType : u8 {
        Standalone,
        Relative,
    };

    enum class SelectorParsingMode : u8 {
        Normal,
        Forgiving,
    };

    struct CounterStyle {
        FFI::CssCounterStyleKind kind;
        FFI::CssCounterStyleSymbolsType symbols_type;
        FlyString name;
        Vector<FlyString> symbols;
    };

    struct CounterStyleRangeSyntax {
        FFI::CssCounterStyleRangeKind kind;
        size_t count { 0 };
    };

    struct SyntaxComponent {
        OwnPtr<SyntaxNode> syntax;
        size_t consumed_byte_length { 0 };
    };

    static Optional<ComponentValue> parse_a_component_value(StringView input, StringView encoding);
    static Vector<ComponentValue> parse_a_list_of_component_values(StringView input, StringView encoding);
    static Vector<Vector<ComponentValue>> parse_a_comma_separated_list_of_component_values(StringView input, StringView encoding);
    static Optional<SelectorList> parse_a_selector_list(StringView input, StringView encoding, SelectorType, SelectorParsingMode, HashTable<FlyString> const& declared_namespaces);
    static FFI::CssValueTypeSyntaxKind parse_a_value_type(u8 value_type_id, TokenStream<ComponentValue>&);
    static Optional<PropertyKeyword> parse_property_keyword_value(ReadonlySpan<PropertyID>, StringView keyword);
    static bool property_accepts_keyword(PropertyID, Keyword);
    static Optional<PropertyID> property_accepting_type(ReadonlySpan<PropertyID>, ValueType);
    static Optional<PropertyCustomIdent> parse_property_custom_ident_value(ReadonlySpan<PropertyID>, StringView input);
    static Optional<GeneratedPropertyValue> parse_generated_property_value(ReadonlySpan<PropertyID>, StringView input);
    static Optional<RustStyleValue> parse_style_value_for_property(ReadonlySpan<PropertyID>, StringView input);
    static Optional<SimpleColor> parse_simple_color(StringView input, StringView encoding, bool allow_quirky_color);
    static Optional<Vector<CoordinatingValueListShorthandItem>> parse_coordinating_value_list_shorthand(ReadonlySpan<PropertyID>, StringView input);
    static Optional<Vector<LayerShorthandItem>> parse_layer_shorthand(PropertyID, StringView input);
    static Optional<Vector<FontShorthandItem>> parse_font_shorthand(StringView input);
    static Optional<Vector<GridPlacementShorthandItem>> parse_grid_placement_shorthand(PropertyID, StringView input);
    static Optional<Vector<GridTemplateShorthandItem>> parse_grid_template_shorthand(PropertyID, StringView input);
    static Optional<Vector<PositionalValueListShorthandItem>> parse_positional_value_list_shorthand(PropertyID, StringView input);
    static Optional<PropertyNumericMetadata> property_numeric_metadata(ReadonlySpan<PropertyID>, ValueType);
    static OwnPtr<SyntaxNode> parse_as_syntax(StringView input, StringView encoding, LimitSingleComponentIdentToCustomIdent);
    static Optional<SyntaxComponent> parse_syntax_component(StringView input, StringView encoding, LimitSingleComponentIdentToCustomIdent);
    static Optional<SyntaxComponent> parse_css_type(StringView input, StringView encoding, LimitSingleComponentIdentToCustomIdent);
    static bool parse_empty_prelude(StringView input, StringView encoding);
    static Optional<Declaration> parse_a_declaration(StringView input, StringView encoding);
    static Optional<Declaration> parse_a_declaration(StringView input, StringView encoding, Vector<RuleContext> const& rule_context);
    static OwnPtr<BooleanExpression> parse_a_supports_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(Vector<ComponentValue>&&)> parse_test);
    static Optional<SupportsFeature> parse_a_supports_feature(StringView input, StringView encoding);
    static OwnPtr<BooleanExpression> parse_an_if_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(Vector<ComponentValue>&&)> parse_test);
    static OwnPtr<BooleanExpression> parse_a_container_condition(StringView input, StringView encoding);
    static OwnPtr<BooleanExpression> parse_a_media_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test);
    static OwnPtr<BooleanExpression> parse_a_media_test(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test);
    static Optional<MediaQuerySyntax> parse_a_media_query(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test);
    static Vector<MediaQuerySyntax> parse_a_media_query_list(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test);
    static Optional<PageSelectorList> parse_a_page_selector_list(StringView input, StringView encoding);
    static Optional<Vector<Percentage>> parse_a_keyframe_selector_list(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_keyframes_name(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_custom_property_name(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_custom_ident(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_dashed_ident(StringView input, StringView encoding);
    static Optional<Gfx::UnicodeRange> parse_a_unicode_range(StringView input, StringView encoding);
    static Optional<Vector<Gfx::UnicodeRange>> parse_a_unicode_range_list(StringView input, StringView encoding);
    static Optional<URL> parse_a_url_function(StringView input, StringView encoding);
    static Optional<URL> parse_an_import_url(StringView input, StringView encoding);
    static Optional<ImportRulePrelude> parse_an_import_rule_prelude(StringView input, StringView encoding);
    static Optional<FontSource> parse_a_font_source(StringView input, StringView encoding);
    static Optional<FlyString> parse_an_opentype_tag(StringView input, StringView encoding);
    static Optional<FontStyle> parse_a_font_style(StringView input, StringView encoding);
    static Optional<Vector<FontVariantAlternatesValue>> parse_a_font_variant_alternates(StringView input, StringView encoding);
    static Optional<Vector<FontVariantEastAsianValue>> parse_a_font_variant_east_asian(StringView input, StringView encoding);
    static Optional<Vector<FontVariantNumericValue>> parse_a_font_variant_numeric(StringView input, StringView encoding);
    static Optional<Vector<FontVariantLigaturesValue>> parse_a_font_variant_ligatures(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_layer_name(StringView input, StringView encoding, AllowBlankLayerName);
    static Optional<FlyString> parse_an_import_layer(StringView input, StringView encoding);
    static Optional<Vector<FlyString>> parse_a_layer_name_list(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_counter_style_name(StringView input, StringView encoding);
    static Optional<CounterStyle> parse_a_counter_style(StringView input, StringView encoding);
    static Optional<FFI::CssNonnegativeIntegerSymbolPairOrder> parse_a_nonnegative_integer_symbol_pair(StringView input, StringView encoding);
    static Optional<FFI::CssCounterStyleNegativeSymbolCount> parse_counter_style_negative(StringView input, StringView encoding);
    static Optional<FFI::CssCounterStyleSystemKind> parse_counter_style_system(StringView input, StringView encoding);
    static bool parse_counter_style_symbol(StringView input, StringView encoding);
    static Optional<size_t> parse_counter_style_symbols(StringView input, StringView encoding);
    static Optional<CounterStyleRangeSyntax> parse_counter_style_range(StringView input, StringView encoding);
    static Optional<size_t> parse_counter_style_additive_symbols(StringView input, StringView encoding);
    static Optional<FFI::CssCropOrCrossKind> parse_crop_or_cross(StringView input, StringView encoding);
    static TimelineScope parse_timeline_scope(StringView input, StringView encoding);
    static ScrollFunction parse_scroll_function(StringView input, StringView encoding);
    static ViewTimelineInset parse_view_timeline_inset(StringView input, StringView encoding);
    static ViewTimelineInset parse_view_timeline_inset_prefix(StringView input, StringView encoding);
    static ViewFunction parse_view_function(StringView input, StringView encoding);
    static FFI::CssRectValueKind parse_rect(StringView input, StringView encoding);
    static FFI::CssRatioValue parse_ratio_prefix(StringView input, StringView encoding);
    static FFI::CssPrimitiveValueKind parse_primitive_value_prefix(StringView input, StringView encoding, FFI::CssPrimitiveValueType, FFI::CssPrimitiveValueOptions);
    static FFI::CssPrimitiveValueKind parse_primitive_value(StringView input, StringView encoding, FFI::CssPrimitiveValueType, FFI::CssPrimitiveValueOptions);
    static FFI::CssEasingValueKind parse_easing(StringView input, StringView encoding);
    static FFI::CssTransformFunctionValueKind parse_transform_function(StringView input, StringView encoding);
    static FFI::CssFitContentValueKind parse_fit_content(StringView input, StringView encoding);
    static FFI::CssBasicShapeValueKind parse_basic_shape(StringView input, StringView encoding);
    static FFI::CssGridAutoFlowValueKind parse_grid_auto_flow(StringView input, StringView encoding);
    static FFI::CssPositionValueKind parse_position(StringView input, StringView encoding, bool allow_background_position_3_value_syntax);
    static FFI::CssPositionValueKind parse_background_position_longhand(StringView input, StringView encoding, bool is_horizontal);
    static FFI::CssBackgroundSizeValueKind parse_background_size(StringView input, StringView encoding);
    static FFI::CssRepeatStyleValueKind parse_repeat_style(StringView input, StringView encoding);
    static FFI::CssColorFunctionValueKind parse_color_function(StringView input, StringView encoding);
    static FFI::CssColorValueKind parse_color(StringView input, StringView encoding, bool allow_quirky_color);
    static FFI::CssImageSetValueKind parse_image_set(StringView input, StringView encoding);
    static Optional<size_t> parse_font_weight_absolute_pair(StringView input, StringView encoding);
    static bool parse_string_descriptor(StringView input, StringView encoding);
    static bool parse_length_descriptor(StringView input, StringView encoding);
    static bool parse_positive_percentage_descriptor(StringView input, StringView encoding);
    static bool parse_page_size_descriptor(StringView input, StringView encoding);
    static bool parse_optional_declaration_value_descriptor(StringView input, StringView encoding);
    static Optional<FamilyName> parse_a_family_name(StringView input, StringView encoding);
    static Optional<NamespaceRulePrelude> parse_a_namespace_rule_prelude(StringView input, StringView encoding);
    static Optional<Vector<FlyString>> parse_font_feature_values_family_name_list(StringView input, StringView encoding);
    static Optional<Vector<u32>> parse_font_feature_values_feature_value(StringView input, StringView encoding);
    static Optional<Vector<ContainerRulePreludeCondition>> parse_container_rule_prelude(StringView input, StringView encoding);
    static Optional<Rule> parse_a_rule(StringView input, StringView encoding);
    static Vector<RuleOrListOfDeclarations> parse_a_blocks_contents(StringView input, StringView encoding);
    static Vector<RuleOrListOfDeclarations> parse_a_blocks_contents(StringView input, StringView encoding, Vector<RuleContext> const& rule_context);
    static Vector<Rule> parse_a_stylesheets_contents(StringView input, StringView encoding);

private:
    using BooleanExpressionEventCallback = void (*)(void*, FFI::CssBooleanExpressionEventKind);
    using MediaQueryCallback = void (*)(void*, FFI::CssMediaQuery const*);
    using MediaFeatureCallback = void (*)(void*, FFI::CssMediaFeature const*);
    using MediaFeatureValueCallback = void (*)(void*, FFI::CssMediaFeatureValue const*);
    using ComponentValueCallback = void (*)(void*, FFI::CssComponentValue const*);
    using BooleanExpressionTestParser = AK::Function<OwnPtr<BooleanExpression>(Optional<MediaFeatureTest>&&, Vector<ComponentValue>&&)>;
    using RustBooleanExpressionParser = AK::Function<void(u8 const*, size_t, void*, BooleanExpressionEventCallback, MediaFeatureCallback, MediaFeatureValueCallback, ComponentValueCallback)>;

    static OwnPtr<BooleanExpression> parse_a_boolean_expression(StringView input, StringView encoding, MatchResult result_for_general_enclosed, BooleanExpressionTestParser parse_test, RustBooleanExpressionParser rust_parse_boolean_expression);
};

}
