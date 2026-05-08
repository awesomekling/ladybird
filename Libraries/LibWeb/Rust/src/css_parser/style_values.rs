/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedStyleValue {
    pub(super) property_id: PropertyId,
    pub(super) value: RustOwnedStyleValueKind,
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
    Content(RustOwnedContent),
    Cursor(RustOwnedCursor),
    Display(RustOwnedDisplay),
    FlexShorthand(RustOwnedFlexShorthand),
    FlexFlow(RustOwnedFlexFlow),
    FilterValueList(RustOwnedFilterValueList),
    FontStyle(RustOwnedFontStyle),
    FontVariantLonghand(RustOwnedFontVariantLonghand),
    PlaceContent(RustOwnedPlaceShorthand),
    PlaceItems(RustOwnedPlaceShorthand),
    PlaceSelf(RustOwnedPlaceShorthand),
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
    SourceBacked(RustOwnedSourceBackedValue),
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
    Rect(RustOwnedRect),
    StrokeDasharray(RustOwnedStrokeDasharray),
    WhiteSpaceTrim(RustOwnedWhiteSpaceTrim),
    ScrollFunction(RustOwnedScrollFunction),
    ViewTimelineInset(RustOwnedViewTimelineInset),
    ViewFunction(RustOwnedViewFunction),
    ViewTransitionName(RustOwnedViewTransitionName),
    WhiteSpace(RustOwnedWhiteSpace),
    WillChange(RustOwnedWillChange),
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
    pub(super) width: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) style: Option<RustOwnedLineStyle>,
    pub(super) color: Option<RustOwnedColor>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFontLanguageOverride {
    pub(super) kind: CssFontLanguageOverrideKind,
    pub(super) value: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedScrollFunction {
    pub(super) scroller: CssScrollFunctionScrollerKind,
    pub(super) axis: CssScrollFunctionAxisKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedViewTimelineInset {
    pub(super) values: Vec<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedViewFunction {
    pub(super) axis: CssScrollFunctionAxisKind,
    pub(super) inset: CssViewFunctionInsetKind,
    pub(super) inset_position: CssViewFunctionInsetPosition,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCornerShape {
    pub(super) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageOutsetList {
    pub(super) values: Vec<RustOwnedBorderImageOutset>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageRepeatList {
    pub(super) values: Vec<RustOwnedBorderImageRepeat>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageWidthList {
    pub(super) values: Vec<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedOverflowClipMargin {
    pub(super) length: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFitContent {
    pub(super) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFontFamilyList {
    pub(super) values: Vec<FontFamilyValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedStyleValueParseResult {
    Parsed(RustOwnedStyleValue),
    Invalid,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCoordinatingValueListShorthandItem {
    pub(super) layer_index: usize,
    pub(super) style_value: RustOwnedStyleValue,
    pub(super) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedLayerShorthandItem {
    pub(super) layer_index: usize,
    pub(super) property_id: PropertyId,
    pub(super) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFontShorthandItem {
    pub(super) property_id: PropertyId,
    pub(super) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGridPlacementShorthandItem {
    pub(super) property_id: PropertyId,
    pub(super) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGridTemplateShorthandItem {
    pub(super) property_id: PropertyId,
    pub(super) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum GridTemplateRowTrackSourceItem {
    LineNames(Vec<String>),
    Track(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionalValueListShorthandItem {
    pub(super) index: usize,
    pub(super) style_value: RustOwnedStyleValue,
    pub(super) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnchorFunction {
    pub(super) anchor_name: Option<String>,
    pub(super) anchor_side: String,
    pub(super) fallback: Option<String>,
    pub(super) source: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnchorSizeFunction {
    pub(super) source: String,
    pub(super) value_type: PropertyValueType,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedSourceBackedValue {
    pub(super) kind: RustOwnedSourceBackedValueKind,
    pub(super) source: String,
    pub(super) value_type: PropertyValueType,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedSourceBackedValueKind {
    MathFunction { name: String },
    TreeCountingFunction,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCounterFunction {
    pub(super) function: RustOwnedCounterFunctionKind,
    pub(super) counter_name: String,
    pub(super) join_string: Option<String>,
    pub(super) counter_style: Option<CounterStyle>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedCounterFunctionKind {
    Counter,
    Counters,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedImage {
    pub(super) kind: RustOwnedImageKind,
    pub(super) source: String,
    pub(super) url: Option<RustOwnedUrlPayload>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedUrl {
    pub(super) source: String,
    pub(super) url: Option<RustOwnedUrlPayload>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedUrlPayload {
    pub(super) function_type: CssUrlFunctionType,
    pub(super) url: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum RustOwnedImageKind {
    Url,
    Gradient,
    ImageSet,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBasicShape {
    pub(super) kind: RustOwnedBasicShapeKind,
    pub(super) fill_rule: RustOwnedBasicShapeFillRule,
    pub(super) rectangle_components: Vec<RustOwnedNestedPrimitiveValue>,
    pub(super) rectangle_border_radius: Option<RustOwnedBorderRadius>,
    pub(super) radial_shape_radius: Vec<RustOwnedNestedPrimitiveValue>,
    pub(super) radial_shape_position: Option<RustOwnedResolvedPosition>,
    pub(super) polygon_points: Vec<RustOwnedBasicShapePolygonPoint>,
    pub(super) path_data: Option<String>,
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
    pub(super) x: RustOwnedNestedPrimitiveValue,
    pub(super) y: RustOwnedNestedPrimitiveValue,
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
    pub(super) options: Vec<RustOwnedImageSetOption>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedImageSetOption {
    pub(super) image_is_string: bool,
    pub(super) image_source: String,
    pub(super) resolution: Option<String>,
    pub(super) mime_type: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedListStyle {
    pub(super) position: Option<RustOwnedListStylePosition>,
    pub(super) image: Option<RustOwnedListStyleImage>,
    pub(super) list_style_type: Option<RustOwnedListStyleType>,
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
    pub(super) value: RustOwnedEasingFunctionValue,
}

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
    pub(super) output: RustOwnedNestedPrimitiveValue,
    pub(super) first_stop_length: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) second_stop_length: Option<RustOwnedNestedPrimitiveValue>,
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
    pub(super) sides: Vec<RustOwnedNestedPrimitiveValue>,
    pub(super) requires_commas: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFontStyle {
    pub(super) value: FontStyle,
    pub(super) angle: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedOpenTypeSettings {
    pub(super) kind: CssOpenTypeSettingsKind,
    pub(super) tag_values: Vec<OpenTypeTaggedValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedOpenTypeSettingsStyleValue {
    pub(super) kind: RustOwnedOpenTypeSettingsStyleValueKind,
    pub(super) value: RustOwnedOpenTypeSettings,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedOpenTypeSettingsStyleValueKind {
    FontFeatureSettings,
    FontVariationSettings,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnchorNameOrScope {
    pub(super) kind: CssAnchorNameOrScopeValueKind,
    pub(super) names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnimationNameItem {
    pub(super) kind: CssAnimationNameItemKind,
    pub(super) value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAnimationName {
    pub(super) kind: CssAnimationNameValueKind,
    pub(super) names: Vec<RustOwnedAnimationNameItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedAspectRatio {
    pub(super) has_auto: bool,
    pub(super) numerator: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) denominator: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBackgroundSizeList {
    pub(super) values: Vec<RustOwnedBackgroundSize>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderRadius {
    pub(super) horizontal_radii: Vec<RustOwnedNestedPrimitiveValue>,
    pub(super) vertical_radii: Vec<RustOwnedNestedPrimitiveValue>,
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
    Source(String),
}

pub(super) const LINE_WIDTH_THIN: u8 = 0;
pub(super) const LINE_WIDTH_MEDIUM: u8 = 1;
pub(super) const LINE_WIDTH_THICK: u8 = 2;

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
    pub(super) values: Vec<RustOwnedNestedPrimitiveValue>,
    pub(super) fill: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImageOutset {
    pub(super) value: RustOwnedNestedPrimitiveValue,
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
    pub(super) value: CssColorSchemeValue,
    pub(super) schemes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCounterDefinition {
    pub(super) name: String,
    pub(super) is_reversed: bool,
    pub(super) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCounterDefinitions {
    pub(super) definitions: Vec<RustOwnedCounterDefinition>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedDisplay {
    pub(super) kind: CssDisplayValueKind,
    pub(super) box_: CssDisplayBox,
    pub(super) internal: CssDisplayInternal,
    pub(super) outside: CssDisplayOutside,
    pub(super) inside: CssDisplayInside,
    pub(super) list_item: CssDisplayListItem,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedGridAutoFlow {
    pub(super) axis: CssGridAutoFlowAxis,
    pub(super) dense: CssGridAutoFlowDense,
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
    pub(super) images: Vec<RustOwnedCursorImage>,
    pub(super) predefined: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedCursorImage {
    pub(super) image: RustOwnedImage,
    pub(super) x: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) y: Option<RustOwnedNestedPrimitiveValue>,
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
    pub(super) repeat_type: RustOwnedGridRepeatType,
    pub(super) track_list: Vec<RustOwnedGridTrackSizeListItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedGridRepeatType {
    AutoFill,
    AutoFit,
    Fixed { count: RustOwnedNestedPrimitiveValue },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedContain {
    pub(super) value: CssContainValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedContainerType {
    pub(super) value: CssContainerTypeValueKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedColumns {
    pub(super) column_count: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) column_width: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) column_height: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPaintOrder {
    pub(super) value: CssPaintOrderValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedFlexFlow {
    pub(super) flex_direction: Option<RustOwnedFlexDirection>,
    pub(super) flex_wrap: Option<RustOwnedFlexWrap>,
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

pub(super) const FLEX_BASIS_KIND_AUTO: u8 = 0;
pub(super) const FLEX_BASIS_KIND_CONTENT: u8 = 1;
pub(super) const FLEX_BASIS_KIND_FIT_CONTENT: u8 = 2;
pub(super) const FLEX_BASIS_KIND_MIN_CONTENT: u8 = 3;
pub(super) const FLEX_BASIS_KIND_MAX_CONTENT: u8 = 4;
pub(super) const FLEX_BASIS_KIND_FIT_CONTENT_FUNCTION: u8 = 5;
pub(super) const FLEX_BASIS_KIND_LENGTH_PERCENTAGE: u8 = 6;
pub(super) const FLEX_BASIS_KIND_SOURCE: u8 = 7;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPlaceShorthand {
    pub(super) align_keywords: Vec<String>,
    pub(super) justify_keywords: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionAnchor {
    pub(super) kind: CssPositionAnchorValueKind,
    pub(super) name: Option<String>,
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
    pub(super) value: CssPositionTryOrderValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionVisibility {
    pub(super) value: CssPositionVisibilityValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPosition {
    pub(super) value_type: PropertyValueType,
    pub(super) value: RustOwnedResolvedPosition,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionList {
    pub(super) value_type: PropertyValueType,
    pub(super) values: Vec<RustOwnedPositionListItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedPositionListItem {
    Position(RustOwnedResolvedPosition),
    Component(RustOwnedPositionComponent),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedResolvedPosition {
    pub(super) x: RustOwnedPositionComponent,
    pub(super) y: RustOwnedPositionComponent,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedPositionComponent {
    pub(super) edge: Option<PositionEdge>,
    pub(super) offset: Option<RustOwnedNestedPrimitiveValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedQuotes {
    pub(super) kind: CssQuotesValueKind,
    pub(super) strings: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedRepeatStyleList {
    pub(super) values: Vec<RustOwnedRepeatStyle>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedRepeatStyle {
    pub(super) repeat_x: CssRepeatStyleRepetition,
    pub(super) repeat_y: CssRepeatStyleRepetition,
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
    pub(super) value: CssScrollbarGutterValueKind,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedBorderImage {
    pub(super) source: Option<RustOwnedBorderImageSource>,
    pub(super) slice: Option<RustOwnedBorderImageSlice>,
    pub(super) width: Option<Vec<RustOwnedNestedPrimitiveValue>>,
    pub(super) outset: Option<Vec<RustOwnedBorderImageOutset>>,
    pub(super) repeat: Option<Vec<RustOwnedBorderImageRepeat>>,
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
    pub(super) color: Option<RustOwnedColor>,
    pub(super) offset_x: RustOwnedNestedPrimitiveValue,
    pub(super) offset_y: RustOwnedNestedPrimitiveValue,
    pub(super) blur_radius: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) spread_distance: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) placement: RustOwnedShadowPlacement,
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
    pub(super) value: CssTextUnderlinePositionValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RustOwnedTextDecorationLine {
    pub(super) bits: u8,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextDecoration {
    pub(super) line: Option<RustOwnedTextDecorationLine>,
    pub(super) thickness: Option<RustOwnedNestedPrimitiveValue>,
    pub(super) style: Option<RustOwnedTextDecorationStyle>,
    pub(super) color: Option<RustOwnedColor>,
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
    pub(super) x: RustOwnedNestedPrimitiveValue,
    pub(super) y: RustOwnedNestedPrimitiveValue,
    pub(super) z: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransformation {
    pub(super) function: TransformFunction,
    pub(super) arguments: Vec<RustOwnedTransformationArgument>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransformationArgument {
    pub(super) parameter_type: TransformFunctionParameterType,
    pub(super) value: RustOwnedNestedPrimitiveValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTimelineNameItem {
    pub(super) kind: CssTimelineNameItemKind,
    pub(super) name: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTimelineName {
    pub(super) kind: CssTimelineNameValueKind,
    pub(super) names: Vec<RustOwnedTimelineNameItem>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedScrollTimeline {
    pub(super) names: Vec<RustOwnedTimelineNameItem>,
    pub(super) axes: Vec<CssScrollFunctionAxisKind>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedViewTimeline {
    pub(super) names: Vec<RustOwnedTimelineNameItem>,
    pub(super) axes: Vec<CssScrollFunctionAxisKind>,
    pub(super) insets: Vec<Vec<RustOwnedNestedPrimitiveValue>>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTimelineScope {
    pub(super) kind: CssTimelineScopeValueKind,
    pub(super) names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextWrap {
    pub(super) value: CssTextWrapValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextWrapMode {
    pub(super) value: CssTextWrapModeValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextWrapStyle {
    pub(super) value: CssTextWrapStyleValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTextIndent {
    pub(super) length_percentage: RustOwnedNestedPrimitiveValue,
    pub(super) has_hanging: bool,
    pub(super) has_each_line: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTouchAction {
    pub(super) value: CssTouchActionValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransitionBehavior {
    pub(super) kind: CssTransitionBehaviorValueKind,
    pub(super) behaviors: Vec<CssTransitionBehaviorItemKind>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedTransitionProperty {
    pub(super) kind: CssTransitionPropertyValueKind,
    pub(super) properties: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedViewTransitionName {
    pub(super) kind: CssViewTransitionNameValueKind,
    pub(super) name: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedWhiteSpaceTrim {
    pub(super) value: CssWhiteSpaceTrimValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedWhiteSpace {
    pub(super) white_space_collapse: String,
    pub(super) text_wrap_mode: CssTextWrapModeValue,
    pub(super) white_space_trim: CssWhiteSpaceTrimValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedWillChangeFeature {
    pub(super) kind: CssWillChangeFeatureKind,
    pub(super) value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedWillChange {
    pub(super) kind: CssWillChangeValueKind,
    pub(super) features: Vec<RustOwnedWillChangeFeature>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RustOwnedStyleValueList {
    pub(super) values: Vec<RustOwnedStyleValueKind>,
    pub(super) separator: RustOwnedStyleValueListSeparator,
    pub(super) value_type: Option<PropertyValueType>,
    pub(super) source: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RustOwnedStyleValueListSeparator {
    Space,
    Comma,
    Slash,
}
