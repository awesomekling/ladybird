/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedStyleValue {
    pub(crate) property_id: PropertyId,
    pub(crate) value: RustOwnedStyleValueKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedStyleValueKind {
    Anchor(RustOwnedAnchorFunction),
    AnchorSize(RustOwnedAnchorSizeFunction),
    AnchorNameOrScope(RustOwnedAnchorNameOrScope),
    Primitive(RustOwnedPrimitiveValue),
    AnimationName(RustOwnedAnimationName),
    AspectRatio(RustOwnedAspectRatio),
    BackgroundSize(RustOwnedBackgroundSizeList),
    Color(RustOwnedColor),
    ColorScheme(RustOwnedColorScheme),
    Contain(RustOwnedContain),
    ContainerType(RustOwnedContainerType),
    CornerShape(RustOwnedCornerShape),
    Counter(RustOwnedCounterFunction),
    CounterStyle(CounterStyle),
    CounterDefinitions(RustOwnedCounterDefinitions),
    BorderRadius(RustOwnedBorderRadius),
    Columns(RustOwnedColumns),
    CoordinatingValueListShorthand(Vec<RustOwnedCoordinatingValueListShorthandItem>),
    Content(RustOwnedContent),
    Cursor(RustOwnedCursor),
    Display(RustOwnedDisplay),
    FlexShorthand(RustOwnedFlexShorthand),
    FlexFlow(RustOwnedFlexFlow),
    FilterValueList(RustOwnedFilterValueList),
    FontShorthand(Vec<RustOwnedFontShorthandItem>),
    ComponentShorthand(Vec<RustOwnedComponentShorthandItem>),
    FontStyle(RustOwnedFontStyle),
    FontVariantLonghand(RustOwnedFontVariantLonghand),
    GridPlacementShorthand(Vec<RustOwnedGridPlacementShorthandItem>),
    GridTemplateShorthand(Vec<RustOwnedGridTemplateShorthandItem>),
    KeywordList(RustOwnedKeywordList),
    PlaceContent(RustOwnedPlaceShorthand),
    PlaceItems(RustOwnedPlaceShorthand),
    PlaceSelf(RustOwnedPlaceShorthand),
    PositionalValueListShorthand(Vec<RustOwnedPositionalValueListShorthandItem>),
    GridAutoFlow(RustOwnedGridAutoFlow),
    GridAutoTrackSizes(RustOwnedGridTrackSizeList),
    GridTemplateAreas(RustOwnedGridTemplateAreas),
    GridTrackPlacement(RustOwnedGridTrackPlacement),
    GridTrackSizeList(RustOwnedGridTrackSizeList),
    GuaranteedInvalid,
    Image(RustOwnedImage),
    ImageSet(RustOwnedImageSet),
    Border(RustOwnedBorder),
    BorderImage(RustOwnedBorderImage),
    BorderImageOutset(RustOwnedBorderImageOutsetList),
    BorderImageRepeat(RustOwnedBorderImageRepeatList),
    BorderImageSlice(RustOwnedBorderImageSlice),
    BorderImageWidth(RustOwnedBorderImageWidthList),
    Identifier(RustOwnedIdentifierValue),
    LayerShorthand(Vec<RustOwnedLayerShorthandItem>),
    ListStyle(RustOwnedListStyle),
    MathDepth(RustOwnedMathDepth),
    Paint(RustOwnedPaint),
    PaintOrder(RustOwnedPaintOrder),
    Position(RustOwnedPosition),
    PositionList(RustOwnedPositionList),
    PositionArea(RustOwnedPositionArea),
    PositionAnchor(RustOwnedPositionAnchor),
    PositionTryFallbacks(RustOwnedPositionTryFallbacks),
    PositionTryOrder(RustOwnedPositionTryOrder),
    PositionVisibility(RustOwnedPositionVisibility),
    Quotes(RustOwnedQuotes),
    RepeatStyle(RustOwnedRepeatStyleList),
    OverflowClipMargin(RustOwnedOverflowClipMargin),
    Shadow(RustOwnedShadow),
    ShapeOutside(RustOwnedShapeOutside),
    TextDecoration(RustOwnedTextDecoration),
    TextDecorationLine(RustOwnedTextDecorationLine),
    ScrollbarColor(RustOwnedScrollbarColor),
    ScrollbarGutter(RustOwnedScrollbarGutter),
    ScrollTimeline(RustOwnedScrollTimeline),
    Shorthand(RustOwnedStyleValueList),
    TimelineName(RustOwnedTimelineName),
    TimelineScope(RustOwnedTimelineScope),
    TextWrap(RustOwnedTextWrap),
    TextWrapMode(RustOwnedTextWrapMode),
    TextWrapStyle(RustOwnedTextWrapStyle),
    TextIndent(RustOwnedTextIndent),
    TextUnderlinePosition(RustOwnedTextUnderlinePosition),
    TouchAction(RustOwnedTouchAction),
    TransformLonghand(RustOwnedTransformLonghand),
    TransformOrigin(RustOwnedTransformOrigin),
    Transformation(RustOwnedTransformation),
    TransitionBehavior(RustOwnedTransitionBehavior),
    TransitionProperty(RustOwnedTransitionProperty),
    ViewTimeline(RustOwnedViewTimeline),
    Tuple(RustOwnedStyleValueList),
    ValueList(RustOwnedStyleValueList),
    Url(RustOwnedUrl),
    EasingFunction(RustOwnedEasingFunction),
    FitContent(RustOwnedFitContent),
    FontFamily(RustOwnedFontFamilyList),
    OpenTypeSettings(RustOwnedOpenTypeSettingsStyleValue),
    FontLanguageOverride(RustOwnedFontLanguageOverride),
    FontVariant(FontVariant),
    BasicShape(Box<RustOwnedBasicShape>),
    BorderSpacing(RustOwnedBorderSpacing),
    Rect(RustOwnedRect),
    StrokeDasharray(RustOwnedStrokeDasharray),
    WhiteSpaceTrim(RustOwnedWhiteSpaceTrim),
    ScrollFunction(RustOwnedScrollFunction),
    ViewTimelineInset(RustOwnedViewTimelineInset),
    ViewFunction(RustOwnedViewFunction),
    ViewTransitionName(RustOwnedViewTransitionName),
    WhiteSpace(RustOwnedWhiteSpace),
    WillChange(RustOwnedWillChange),
    MathFunction(RustOwnedMathFunction),
    TreeCountingFunction(RustOwnedTreeCountingFunction),
    GeneratedValueList(RustOwnedGeneratedValueList),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderSpacing {
    pub(crate) values: Vec<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedPrimitiveValue {
    Nested {
        value: RustOwnedNestedPrimitiveValue,
        value_type: PropertyValueType,
    },
    Ratio {
        numerator: f64,
        denominator: f64,
        has_denominator: bool,
        value_type: PropertyValueType,
    },
    Token {
        primitive_kind: CssPrimitiveValueKind,
        numeric_value: Option<f64>,
        secondary_numeric_value: Option<f64>,
        value: String,
        value_type: PropertyValueType,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedIdentifierValue {
    Keyword(String),
    CustomIdent {
        value: String,
        value_type: PropertyValueType,
    },
    CounterStyleName(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedFontVariantLonghand {
    Alternates(Vec<FontVariantAlternatesValue>),
    EastAsian(Vec<FontVariantEastAsianValue>),
    Ligatures(Vec<FontVariantLigaturesValue>),
    Numeric(Vec<FontVariantNumericValue>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorder {
    pub(crate) width: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) style: Option<RustOwnedLineStyle>,
    pub(crate) color: Option<RustOwnedColor>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFontLanguageOverride {
    pub(crate) kind: CssFontLanguageOverrideKind,
    pub(crate) value: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedScrollFunction {
    pub(crate) scroller: CssScrollFunctionScrollerKind,
    pub(crate) axis: CssScrollFunctionAxisKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedViewTimelineInset {
    pub(crate) insets: Vec<Vec<RustOwnedNestedPrimitiveValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedViewFunction {
    pub(crate) axis: CssScrollFunctionAxisKind,
    pub(crate) inset: CssViewFunctionInsetKind,
    pub(crate) inset_position: CssViewFunctionInsetPosition,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCornerShape {
    pub(crate) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageOutsetList {
    pub(crate) values: Vec<RustOwnedBorderImageOutset>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageRepeatList {
    pub(crate) values: Vec<RustOwnedBorderImageRepeat>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageWidthList {
    pub(crate) values: Vec<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedOverflowClipMargin {
    pub(crate) length: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFitContent {
    pub(crate) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFontFamilyList {
    pub(crate) values: Vec<FontFamilyValue>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedStyleValueParseResult {
    Parsed(RustOwnedStyleValue),
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCoordinatingValueListShorthandItem {
    pub(crate) layer_index: usize,
    pub(crate) style_value: RustOwnedStyleValue,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedLayerShorthandItem {
    pub(crate) layer_index: usize,
    pub(crate) property_id: PropertyId,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFontShorthandItem {
    pub(crate) property_id: PropertyId,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedComponentShorthandItem {
    pub(crate) property_id: PropertyId,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGridPlacementShorthandItem {
    pub(crate) property_id: PropertyId,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGridTemplateShorthandItem {
    pub(crate) property_id: PropertyId,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum GridTemplateRowTrackSourceItem {
    LineNames(Vec<String>),
    Track(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionalValueListShorthandItem {
    pub(crate) index: usize,
    pub(crate) style_value: RustOwnedStyleValue,
    pub(crate) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnchorFunction {
    pub(crate) anchor_name: Option<String>,
    pub(crate) anchor_side: String,
    pub(crate) fallback: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnchorSizeFunction {
    pub(crate) value_type: PropertyValueType,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedMathFunction {
    pub(crate) name: String,
    pub(crate) arguments: Vec<ComponentValue>,
    pub(crate) source: String,
    pub(crate) value_type: PropertyValueType,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTreeCountingFunction {
    pub(crate) function: RustOwnedTreeCountingFunctionKind,
    pub(crate) value_type: PropertyValueType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedTreeCountingFunctionKind {
    SiblingCount,
    SiblingIndex,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCounterFunction {
    pub(crate) function: RustOwnedCounterFunctionKind,
    pub(crate) counter_name: String,
    pub(crate) join_string: Option<String>,
    pub(crate) counter_style: Option<CounterStyle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedCounterFunctionKind {
    Counter,
    Counters,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedImage {
    pub(crate) kind: RustOwnedImageKind,
    pub(crate) source: Option<String>,
    pub(crate) url: Option<RustOwnedUrlPayload>,
    pub(crate) gradient: Option<RustOwnedGradient>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedUrl {
    pub(crate) url: Option<RustOwnedUrlPayload>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedUrlPayload {
    pub(crate) function_type: CssUrlFunctionType,
    pub(crate) url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedImageKind {
    Url,
    Gradient,
    ImageSet,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGradient {
    pub(crate) kind: RustOwnedGradientKind,
    pub(crate) is_repeating: bool,
    pub(crate) is_webkit_prefixed: bool,
    pub(crate) groups: Vec<Vec<ComponentValue>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedGradientKind {
    Linear,
    Radial,
    Conic,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBasicShape {
    pub(crate) kind: RustOwnedBasicShapeKind,
    pub(crate) fill_rule: RustOwnedBasicShapeFillRule,
    pub(crate) rectangle_components: Vec<RustOwnedNestedPrimitiveValue>,
    pub(crate) rectangle_border_radius: Option<RustOwnedBorderRadius>,
    pub(crate) radial_shape_radius: Vec<RustOwnedNestedPrimitiveValue>,
    pub(crate) radial_shape_position: Option<RustOwnedResolvedPosition>,
    pub(crate) polygon_points: Vec<RustOwnedBasicShapePolygonPoint>,
    pub(crate) path_data: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedBasicShapeKind {
    Inset,
    Xywh,
    Rect,
    Circle,
    Ellipse,
    Polygon,
    Path,
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedBasicShapeFillRule {
    Nonzero,
    Evenodd,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBasicShapePolygonPoint {
    pub(crate) x: RustOwnedNestedPrimitiveValue,
    pub(crate) y: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RustOwnedBasicShapeRadialExtent {
    ClosestCorner,
    ClosestSide,
    FarthestCorner,
    FarthestSide,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedImageSet {
    pub(crate) options: Vec<RustOwnedImageSetOption>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedImageSetOption {
    pub(crate) image: RustOwnedImage,
    pub(crate) resolution: Option<String>,
    pub(crate) mime_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedListStyle {
    pub(crate) position: Option<RustOwnedListStylePosition>,
    pub(crate) image: Option<RustOwnedListStyleImage>,
    pub(crate) list_style_type: Option<RustOwnedListStyleType>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedListStylePosition {
    Inside,
    Outside,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedListStyleImage {
    None,
    Image(RustOwnedImage),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedListStyleType {
    None,
    String(String),
    CounterStyle(CounterStyle),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedMathDepth {
    AutoAdd,
    Add { integer: RustOwnedNestedPrimitiveValue },
    Integer { integer: RustOwnedNestedPrimitiveValue },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedEasingFunction {
    pub(crate) value: RustOwnedEasingFunctionValue,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedEasingFunctionValue {
    Keyword(String),
    Linear(Vec<RustOwnedLinearEasingStop>),
    CubicBezier {
        x1: RustOwnedNestedPrimitiveValue,
        y1: RustOwnedNestedPrimitiveValue,
        x2: RustOwnedNestedPrimitiveValue,
        y2: RustOwnedNestedPrimitiveValue,
    },
    Steps {
        intervals: RustOwnedNestedPrimitiveValue,
        position: Option<RustOwnedStepPosition>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedLinearEasingStop {
    pub(crate) output: RustOwnedNestedPrimitiveValue,
    pub(crate) first_stop_length: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) second_stop_length: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum RustOwnedStepPosition {
    JumpStart,
    JumpEnd,
    JumpNone,
    JumpBoth,
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedRect {
    pub(crate) sides: Vec<RustOwnedNestedPrimitiveValue>,
    pub(crate) requires_commas: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFontStyle {
    pub(crate) value: FontStyle,
    pub(crate) angle: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedOpenTypeSettings {
    pub(crate) kind: CssOpenTypeSettingsKind,
    pub(crate) tag_values: Vec<OpenTypeTaggedValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedOpenTypeSettingsStyleValue {
    pub(crate) kind: RustOwnedOpenTypeSettingsStyleValueKind,
    pub(crate) value: RustOwnedOpenTypeSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedOpenTypeSettingsStyleValueKind {
    FontFeatureSettings,
    FontVariationSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnchorNameOrScope {
    pub(crate) kind: CssAnchorNameOrScopeValueKind,
    pub(crate) names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnimationNameItem {
    pub(crate) kind: CssAnimationNameItemKind,
    pub(crate) value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnimationName {
    pub(crate) kind: CssAnimationNameValueKind,
    pub(crate) names: Vec<RustOwnedAnimationNameItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAspectRatio {
    pub(crate) has_auto: bool,
    pub(crate) numerator: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) denominator: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBackgroundSizeList {
    pub(crate) values: Vec<RustOwnedBackgroundSize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderRadius {
    pub(crate) horizontal_radii: Vec<RustOwnedNestedPrimitiveValue>,
    pub(crate) vertical_radii: Vec<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedColor {
    Simple {
        kind: CssParsedColorKind,
        red: u8,
        green: u8,
        blue: u8,
        alpha: u8,
        name: Option<String>,
    },
    Function {
        name: String,
        arguments: Vec<ComponentValue>,
        source: String,
    },
}

pub(crate) const LINE_WIDTH_THIN: u8 = 0;
pub(crate) const LINE_WIDTH_MEDIUM: u8 = 1;
pub(crate) const LINE_WIDTH_THICK: u8 = 2;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedPaint {
    None,
    Color(RustOwnedColor),
    Url {
        url: RustOwnedUrl,
        fallback_color: Option<RustOwnedColor>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedLineStyle {
    None,
    Hidden,
    Dotted,
    Dashed,
    Solid,
    Double,
    Groove,
    Ridge,
    Inset,
    Outset,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageSlice {
    pub(crate) values: Vec<RustOwnedNestedPrimitiveValue>,
    pub(crate) fill: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageOutset {
    pub(crate) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedBackgroundSize {
    Cover,
    Contain,
    Explicit {
        width: RustOwnedNestedPrimitiveValue,
        height: Option<RustOwnedNestedPrimitiveValue>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedColorScheme {
    pub(crate) value: CssColorSchemeValue,
    pub(crate) schemes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCounterDefinition {
    pub(crate) name: String,
    pub(crate) is_reversed: bool,
    pub(crate) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCounterDefinitions {
    pub(crate) definitions: Vec<RustOwnedCounterDefinition>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedDisplay {
    pub(crate) kind: CssDisplayValueKind,
    pub(crate) box_: CssDisplayBox,
    pub(crate) internal: CssDisplayInternal,
    pub(crate) outside: CssDisplayOutside,
    pub(crate) inside: CssDisplayInside,
    pub(crate) list_item: CssDisplayListItem,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGridAutoFlow {
    pub(crate) axis: CssGridAutoFlowAxis,
    pub(crate) dense: CssGridAutoFlowDense,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedGridTrackPlacement {
    Auto,
    Line {
        line_number: Option<RustOwnedNestedPrimitiveValue>,
        name: Option<String>,
    },
    Span {
        line_number: Option<RustOwnedNestedPrimitiveValue>,
        name: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCursor {
    pub(crate) images: Vec<RustOwnedCursorImage>,
    pub(crate) predefined: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCursorImage {
    pub(crate) image: RustOwnedImage,
    pub(crate) x: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) y: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedContent {
    Normal,
    None,
    Items {
        items: Vec<RustOwnedContentItem>,
        alt_text: Vec<RustOwnedContentAltTextItem>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedContentItem {
    Quote(String),
    String(String),
    Image(RustOwnedImage),
    Counter(RustOwnedCounterFunction),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedContentAltTextItem {
    String(String),
    Counter(RustOwnedCounterFunction),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedShapeOutside {
    None,
    Image(RustOwnedImage),
    Shape {
        basic_shape: Option<Box<RustOwnedBasicShape>>,
        shape_box: Option<RustOwnedShapeBox>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedShapeBox {
    Content,
    Padding,
    Border,
    Margin,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedFilterValueList {
    None,
    Filters(Vec<RustOwnedFilterValue>),
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedFilterValue {
    Url(RustOwnedUrl),
    Blur {
        radius: Option<RustOwnedNestedPrimitiveValue>,
    },
    DropShadow {
        color: Option<RustOwnedColor>,
        offset_x: RustOwnedNestedPrimitiveValue,
        offset_y: RustOwnedNestedPrimitiveValue,
        radius: Option<RustOwnedNestedPrimitiveValue>,
    },
    HueRotate {
        angle: Option<RustOwnedNestedPrimitiveValue>,
    },
    Simple {
        function: RustOwnedSimpleFilterFunction,
        amount: Option<RustOwnedNestedPrimitiveValue>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedNestedPrimitiveValue {
    Integer(i32),
    Number(f64),
    Percentage(f64),
    Length { value: f64, unit: String },
    Angle { value: f64, unit: String },
    Flex { value: f64, unit: String },
    Frequency { value: f64, unit: String },
    Resolution { value: f64, unit: String },
    Time { value: f64, unit: String },
    Keyword(String),
    MathFunction(RustOwnedMathFunction),
    TreeCountingFunction(RustOwnedTreeCountingFunction),
    Source(String),
    FlexSource(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedSimpleFilterFunction {
    Brightness,
    Contrast,
    Grayscale,
    Invert,
    Opacity,
    Saturate,
    Sepia,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedGridTrackSizeList {
    None,
    List(Vec<RustOwnedGridTrackSizeListItem>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedGridTemplateAreas {
    None,
    Rows(Vec<Vec<String>>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedGridTrackSizeListItem {
    LineNames(Vec<String>),
    Track(RustOwnedExplicitGridTrack),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedExplicitGridTrack {
    Size(RustOwnedGridTrackSize),
    Repeat(RustOwnedGridRepeat),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedGridTrackSize {
    Breadth(RustOwnedNestedPrimitiveValue),
    MinMax {
        min: RustOwnedNestedPrimitiveValue,
        max: RustOwnedNestedPrimitiveValue,
    },
    FitContent(RustOwnedNestedPrimitiveValue),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGridRepeat {
    pub(crate) repeat_type: RustOwnedGridRepeatType,
    pub(crate) track_list: Vec<RustOwnedGridTrackSizeListItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedGridRepeatType {
    AutoFill,
    AutoFit,
    Fixed { count: RustOwnedNestedPrimitiveValue },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedContain {
    pub(crate) value: CssContainValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedContainerType {
    pub(crate) value: CssContainerTypeValueKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedColumns {
    pub(crate) column_count: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) column_width: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) column_height: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPaintOrder {
    pub(crate) value: CssPaintOrderValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFlexFlow {
    pub(crate) flex_direction: Option<RustOwnedFlexDirection>,
    pub(crate) flex_wrap: Option<RustOwnedFlexWrap>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedFlexDirection {
    Row,
    RowReverse,
    Column,
    ColumnReverse,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedFlexWrap {
    Nowrap,
    Wrap,
    WrapReverse,
}

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedFlexShorthand {
    None,
    Longhands {
        flex_grow: RustOwnedNestedPrimitiveValue,
        flex_shrink: RustOwnedNestedPrimitiveValue,
        flex_basis: RustOwnedFlexBasis,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedFlexBasis {
    Value(RustOwnedNestedPrimitiveValue),
    FitContentFunction(RustOwnedNestedPrimitiveValue),
}

pub(crate) const FLEX_BASIS_KIND_AUTO: u8 = 0;
pub(crate) const FLEX_BASIS_KIND_CONTENT: u8 = 1;
pub(crate) const FLEX_BASIS_KIND_FIT_CONTENT: u8 = 2;
pub(crate) const FLEX_BASIS_KIND_MIN_CONTENT: u8 = 3;
pub(crate) const FLEX_BASIS_KIND_MAX_CONTENT: u8 = 4;
pub(crate) const FLEX_BASIS_KIND_FIT_CONTENT_FUNCTION: u8 = 5;
pub(crate) const FLEX_BASIS_KIND_LENGTH_PERCENTAGE: u8 = 6;
pub(crate) const FLEX_BASIS_KIND_SOURCE: u8 = 7;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPlaceShorthand {
    pub(crate) align_keywords: Vec<String>,
    pub(crate) justify_keywords: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedKeywordList {
    pub(crate) keywords: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionAnchor {
    pub(crate) kind: CssPositionAnchorValueKind,
    pub(crate) name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedPositionArea {
    None,
    Area {
        first_keyword: String,
        second_keyword: Option<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedPositionTryFallbacks {
    None,
    List(Vec<RustOwnedPositionTryFallback>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedPositionTryFallback {
    PositionArea(RustOwnedPositionArea),
    TryTactic {
        dashed_ident: Option<String>,
        has_flip_block: bool,
        has_flip_inline: bool,
        has_flip_start: bool,
        try_tactics: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionTryOrder {
    pub(crate) value: CssPositionTryOrderValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionVisibility {
    pub(crate) value: CssPositionVisibilityValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPosition {
    pub(crate) value_type: PropertyValueType,
    pub(crate) value: RustOwnedResolvedPosition,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionList {
    pub(crate) value_type: PropertyValueType,
    pub(crate) values: Vec<RustOwnedPositionListItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedPositionListItem {
    Position(RustOwnedResolvedPosition),
    Component(RustOwnedPositionComponent),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedResolvedPosition {
    pub(crate) x: RustOwnedPositionComponent,
    pub(crate) y: RustOwnedPositionComponent,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionComponent {
    pub(crate) edge: Option<PositionEdge>,
    pub(crate) offset: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedQuotes {
    pub(crate) kind: CssQuotesValueKind,
    pub(crate) strings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedRepeatStyleList {
    pub(crate) values: Vec<RustOwnedRepeatStyle>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedRepeatStyle {
    pub(crate) repeat_x: CssRepeatStyleRepetition,
    pub(crate) repeat_y: CssRepeatStyleRepetition,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedScrollbarColor {
    Auto,
    Colors {
        thumb_color: RustOwnedColor,
        track_color: RustOwnedColor,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedScrollbarGutter {
    pub(crate) value: CssScrollbarGutterValueKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImage {
    pub(crate) source: Option<RustOwnedBorderImageSource>,
    pub(crate) slice: Option<RustOwnedBorderImageSlice>,
    pub(crate) width: Option<Vec<RustOwnedNestedPrimitiveValue>>,
    pub(crate) outset: Option<Vec<RustOwnedBorderImageOutset>>,
    pub(crate) repeat: Option<Vec<RustOwnedBorderImageRepeat>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedBorderImageSource {
    None,
    Image(RustOwnedImage),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedBorderImageRepeat {
    Stretch,
    Repeat,
    Round,
    Space,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedShadow {
    None,
    Shadows(Vec<RustOwnedSingleShadow>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedSingleShadow {
    pub(crate) color: Option<RustOwnedColor>,
    pub(crate) offset_x: RustOwnedNestedPrimitiveValue,
    pub(crate) offset_y: RustOwnedNestedPrimitiveValue,
    pub(crate) blur_radius: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) spread_distance: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) placement: RustOwnedShadowPlacement,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedShadowPlacement {
    Outer,
    Inner,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedStrokeDasharray {
    None,
    Values(Vec<RustOwnedNestedPrimitiveValue>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextUnderlinePosition {
    pub(crate) value: CssTextUnderlinePositionValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RustOwnedTextDecorationLine {
    pub(crate) bits: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextDecoration {
    pub(crate) line: Option<RustOwnedTextDecorationLine>,
    pub(crate) thickness: Option<RustOwnedNestedPrimitiveValue>,
    pub(crate) style: Option<RustOwnedTextDecorationStyle>,
    pub(crate) color: Option<RustOwnedColor>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedTextDecorationStyle {
    Solid,
    Double,
    Dotted,
    Dashed,
    Wavy,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedTransformLonghand {
    None,
    Function {
        function: RustOwnedTransformLonghandFunction,
        arguments: Vec<RustOwnedTransformationArgument>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedTransformLonghandFunction {
    Rotate,
    RotateX,
    RotateY,
    RotateZ,
    Rotate3d,
    Translate,
    Translate3d,
    Scale,
    Scale3d,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransformOrigin {
    pub(crate) x: RustOwnedNestedPrimitiveValue,
    pub(crate) y: RustOwnedNestedPrimitiveValue,
    pub(crate) z: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransformation {
    pub(crate) function: TransformFunction,
    pub(crate) arguments: Vec<RustOwnedTransformationArgument>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransformationArgument {
    pub(crate) parameter_type: TransformFunctionParameterType,
    pub(crate) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTimelineNameItem {
    pub(crate) kind: CssTimelineNameItemKind,
    pub(crate) name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTimelineName {
    pub(crate) kind: CssTimelineNameValueKind,
    pub(crate) names: Vec<RustOwnedTimelineNameItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedScrollTimeline {
    pub(crate) names: Vec<RustOwnedTimelineNameItem>,
    pub(crate) axes: Vec<CssScrollFunctionAxisKind>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedViewTimeline {
    pub(crate) names: Vec<RustOwnedTimelineNameItem>,
    pub(crate) axes: Vec<CssScrollFunctionAxisKind>,
    pub(crate) insets: Vec<Vec<RustOwnedNestedPrimitiveValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTimelineScope {
    pub(crate) kind: CssTimelineScopeValueKind,
    pub(crate) names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextWrap {
    pub(crate) value: CssTextWrapValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextWrapMode {
    pub(crate) value: CssTextWrapModeValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextWrapStyle {
    pub(crate) value: CssTextWrapStyleValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextIndent {
    pub(crate) length_percentage: RustOwnedNestedPrimitiveValue,
    pub(crate) has_hanging: bool,
    pub(crate) has_each_line: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTouchAction {
    pub(crate) value: CssTouchActionValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransitionBehavior {
    pub(crate) kind: CssTransitionBehaviorValueKind,
    pub(crate) behaviors: Vec<CssTransitionBehaviorItemKind>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransitionProperty {
    pub(crate) kind: CssTransitionPropertyValueKind,
    pub(crate) properties: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedViewTransitionName {
    pub(crate) kind: CssViewTransitionNameValueKind,
    pub(crate) name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedWhiteSpaceTrim {
    pub(crate) value: CssWhiteSpaceTrimValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedWhiteSpace {
    pub(crate) white_space_collapse: String,
    pub(crate) text_wrap_mode: CssTextWrapModeValue,
    pub(crate) white_space_trim: CssWhiteSpaceTrimValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedWillChangeFeature {
    pub(crate) kind: CssWillChangeFeatureKind,
    pub(crate) value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedWillChange {
    pub(crate) kind: CssWillChangeValueKind,
    pub(crate) features: Vec<RustOwnedWillChangeFeature>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedStyleValueList {
    pub(crate) values: Vec<RustOwnedStyleValueKind>,
    pub(crate) separator: RustOwnedStyleValueListSeparator,
    pub(crate) value_type: Option<PropertyValueType>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGeneratedValueList {
    pub(crate) items: Vec<RustOwnedGeneratedValueListItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGeneratedValueListItem {
    pub(crate) source: String,
    pub(crate) value_type: PropertyValueType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedStyleValueListSeparator {
    Space,
    Comma,
    Slash,
}
