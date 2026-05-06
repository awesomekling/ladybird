/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// The Rust parser is being staged bottom-up. Keep this module warning-free until
// the C++ bridge starts calling into it.
#![allow(dead_code)]

use crate::css_tokenizer::{CssNumberType, CssToken, NumericValue, Token, TokenType};
use crate::generated_media_features::{
    MediaFeatureId, MediaFeatureValueType, media_feature_accepts_identifier, media_feature_accepts_type,
    media_feature_id_from_string, media_feature_type_is_range,
};
use crate::generated_properties::{
    PropertyNumericRange, PropertyValueType, property_accepted_range_by_value_type, property_accepts_keyword,
    property_accepts_value_type, property_custom_ident_blacklist, property_id_from_u16,
    property_resolves_percentages_relative_to, property_value_type_from_css_value_type_name,
    resolve_legacy_value_alias,
};
use crate::generated_units::{DimensionType, dimension_for_unit};
use crate::generated_value_types::{
    ValueTypeId, component_values_parse_as_generated_value_type, value_type_id_from_u8,
};

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ComponentValue {
    PreservedToken(Token),
    Function(Function),
    SimpleBlock(SimpleBlock),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Function {
    pub(crate) name: String,
    pub(crate) value: Vec<ComponentValue>,
    pub(crate) name_token: Token,
    pub(crate) end_token: Token,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SimpleBlock {
    pub(crate) token: Token,
    pub(crate) value: Vec<ComponentValue>,
    pub(crate) end_token: Token,
}

// https://drafts.csswg.org/css-syntax/#css-rule
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Rule {
    AtRule(AtRule),
    QualifiedRule(QualifiedRule),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RuleOrListOfDeclarations {
    Rule(Rule),
    ListOfDeclarations(Vec<Declaration>),
}

// https://drafts.csswg.org/css-syntax/#ref-for-at-rule%E2%91%A0%E2%91%A1
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AtRule {
    pub(crate) name: String,
    pub(crate) prelude: Vec<ComponentValue>,
    pub(crate) child_rules_and_lists_of_declarations: Vec<RuleOrListOfDeclarations>,
    pub(crate) is_block_rule: bool,
}

// https://drafts.csswg.org/css-syntax/#qualified-rule
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QualifiedRule {
    pub(crate) prelude: Vec<ComponentValue>,
    pub(crate) declarations: Vec<Declaration>,
    pub(crate) child_rules: Vec<RuleOrListOfDeclarations>,
}

// https://drafts.csswg.org/css-syntax/#declaration
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Declaration {
    pub(crate) name: String,
    pub(crate) value: Vec<ComponentValue>,
    pub(crate) important: bool,
}

#[derive(Clone, Debug, PartialEq)]
struct UrlFunction {
    function_type: CssUrlFunctionType,
    url: String,
    request_url_modifiers: Vec<UrlModifier>,
}

#[derive(Clone, Debug, PartialEq)]
enum UrlModifier {
    CrossOrigin(CssUrlCrossOriginModifierValue),
    Integrity(String),
    ReferrerPolicy(CssUrlReferrerPolicyModifierValue),
}

impl UrlModifier {
    fn kind(&self) -> CssUrlModifierKind {
        match self {
            UrlModifier::CrossOrigin(_) => CssUrlModifierKind::CrossOrigin,
            UrlModifier::Integrity(_) => CssUrlModifierKind::Integrity,
            UrlModifier::ReferrerPolicy(_) => CssUrlModifierKind::ReferrerPolicy,
        }
    }

    fn as_ffi(&self) -> CssUrlModifier {
        match self {
            UrlModifier::CrossOrigin(value) => CssUrlModifier {
                kind: CssUrlModifierKind::CrossOrigin,
                cross_origin_value: *value,
                referrer_policy_value: CssUrlReferrerPolicyModifierValue::NoReferrer,
                integrity_ptr: std::ptr::null(),
                integrity_len: 0,
            },
            UrlModifier::Integrity(value) => CssUrlModifier {
                kind: CssUrlModifierKind::Integrity,
                cross_origin_value: CssUrlCrossOriginModifierValue::Anonymous,
                referrer_policy_value: CssUrlReferrerPolicyModifierValue::NoReferrer,
                integrity_ptr: value.as_ptr(),
                integrity_len: value.len(),
            },
            UrlModifier::ReferrerPolicy(value) => CssUrlModifier {
                kind: CssUrlModifierKind::ReferrerPolicy,
                cross_origin_value: CssUrlCrossOriginModifierValue::Anonymous,
                referrer_policy_value: *value,
                integrity_ptr: std::ptr::null(),
                integrity_len: 0,
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum FontSource {
    Local(FamilyName),
    Url {
        url_function: UrlFunction,
        format: Option<String>,
        tech: Vec<CssFontTech>,
    },
}

#[derive(Clone, Debug, PartialEq)]
enum FontLanguageOverride {
    Normal,
    String(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum CounterStyle {
    Name(String),
    SymbolsFunction {
        symbols_type: CssCounterStyleSymbolsType,
        symbols: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpenTypeTaggedValue {
    pub(crate) tag: String,
    pub(crate) value_kind: CssOpenTypeTaggedValueKind,
    pub(crate) value: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
enum OpenTypeSettings {
    Normal,
    TagValues(Vec<OpenTypeTaggedValue>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FontStyle {
    Normal,
    Italic,
    Left,
    Right,
    Oblique { has_angle: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FontVariantAlternatesValue {
    pub(crate) kind: CssFontVariantAlternatesValueKind,
    pub(crate) feature_value_names: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FontVariantEastAsianValue {
    pub(crate) kind: CssFontVariantEastAsianValueKind,
    pub(crate) value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FontVariantNumericValue {
    pub(crate) kind: CssFontVariantNumericValueKind,
    pub(crate) value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FontVariantLigaturesValue {
    pub(crate) kind: CssFontVariantLigaturesValueKind,
    pub(crate) value: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct FontVariant {
    pub(crate) ligatures_none: bool,
    pub(crate) alternates: Option<Vec<FontVariantAlternatesValue>>,
    pub(crate) caps: Option<String>,
    pub(crate) east_asian: Option<Vec<FontVariantEastAsianValue>>,
    pub(crate) emoji: Option<String>,
    pub(crate) ligatures: Option<Vec<FontVariantLigaturesValue>>,
    pub(crate) numeric: Option<Vec<FontVariantNumericValue>>,
    pub(crate) position: Option<String>,
}

impl FontVariant {
    fn has_any_value(&self) -> bool {
        self.ligatures_none
            || self.alternates.is_some()
            || self.caps.is_some()
            || self.east_asian.is_some()
            || self.emoji.is_some()
            || self.ligatures.is_some()
            || self.numeric.is_some()
            || self.position.is_some()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FontFamilyValue {
    Generic(String),
    FamilyName(FamilyName),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BooleanExpression {
    Not(Box<BooleanExpression>),
    Parens(Box<BooleanExpression>),
    And(Vec<BooleanExpression>),
    Or(Vec<BooleanExpression>),
    Test(BooleanExpressionTest),
    GeneralEnclosed(ComponentValue),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum BooleanExpressionTest {
    SupportsFeature(Vec<ComponentValue>),
    MediaFeature(Box<MediaFeatureTest>),
    IfTest(Vec<ComponentValue>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SupportsFeature {
    Declaration,
    Selector,
    FontTech(String),
    FontFormat(String),
    Env(String),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SyntaxNode {
    Universal,
    Type(String),
    Ident(String),
    Multiplier(Box<SyntaxNode>),
    CommaSeparatedMultiplier(Box<SyntaxNode>),
    Alternatives(Vec<SyntaxNode>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MediaQuerySyntax {
    Valid {
        modifier: MediaQueryModifier,
        media_type: Option<String>,
        condition: Option<Box<BooleanExpression>>,
    },
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaQueryModifier {
    None,
    Not,
    Only,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PageSelector {
    name: Option<String>,
    pseudo_classes: Vec<CssPagePseudoClassKind>,
}

pub(crate) type KeyframeSelector = f64;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MediaFeatureTest {
    component_value: ComponentValue,
    kind: MediaFeatureSyntax,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum MediaFeatureSyntax {
    Boolean(MediaFeatureName),
    Plain {
        name: MediaFeatureName,
        value: Vec<ComponentValue>,
    },
    HalfRangeNameFirst {
        name: MediaFeatureName,
        comparison: MfComparison,
        value: Vec<ComponentValue>,
    },
    HalfRangeValueFirst {
        value: Vec<ComponentValue>,
        comparison: MfComparison,
        name: MediaFeatureName,
    },
    Range {
        left_value: Vec<ComponentValue>,
        left_comparison: MfComparison,
        name: MediaFeatureName,
        right_comparison: MfComparison,
        right_value: Vec<ComponentValue>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaFeatureValueSyntaxKind {
    Ident,
    Boolean,
    Integer,
    Length,
    Ratio,
    Resolution,
    Unknown,
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MediaFeatureName {
    kind: MediaFeatureNameKind,
    id: MediaFeatureId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaFeatureNameKind {
    Normal,
    Min,
    Max,
}

struct ComponentValueParser {
    component_values: Vec<ComponentValue>,
    index: usize,
    boolean_expression: Option<BooleanExpression>,
}

#[derive(Clone, Copy)]
enum BooleanExpressionTestKind {
    SupportsFeature,
    MediaFeature,
    IfTest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Nested {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuleContext {
    Unknown,
    Style,
    AtContainer,
    AtCounterStyle,
    AtMedia,
    AtFontFace,
    AtFontFeatureValues,
    FontFeatureValue,
    AtFunction,
    AtKeyframes,
    Keyframe,
    AtSupports,
    SupportsCondition,
    AtLayer,
    AtProperty,
    AtPage,
    Margin,
}

// NB: Keep this in sync with Web::CSS::Parser::RuleContext in RuleContext.h.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssRuleContext {
    Unknown,
    Style,
    AtContainer,
    AtCounterStyle,
    AtMedia,
    AtFontFace,
    AtFontFeatureValues,
    FontFeatureValue,
    AtFunction,
    AtKeyframes,
    Keyframe,
    AtSupports,
    SupportsCondition,
    AtLayer,
    AtProperty,
    AtPage,
    Margin,
}

pub(crate) struct Parser {
    tokens: Vec<Token>,
    index: usize,
    rule_context: Vec<RuleContext>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssComponentValueKind {
    Token,
    FunctionStart,
    FunctionEnd,
    SimpleBlockStart,
    SimpleBlockEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPagePseudoClassKind {
    Left,
    Right,
    First,
    Blank,
}

#[repr(C)]
pub struct CssPageSelector {
    pub has_name: bool,
    pub name_ptr: *const u8,
    pub name_len: usize,
}

#[repr(C)]
pub struct CssComponentValue {
    pub kind: CssComponentValueKind,
    pub token: CssToken,
}

#[repr(C)]
pub struct CssUnicodeRange {
    pub min_code_point: u32,
    pub max_code_point: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssUrlFunctionType {
    Url,
    Src,
}

#[repr(C)]
pub struct CssUrlFunction {
    pub function_type: CssUrlFunctionType,
    pub url_ptr: *const u8,
    pub url_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub enum CssUrlModifierKind {
    CrossOrigin,
    Integrity,
    ReferrerPolicy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssUrlCrossOriginModifierValue {
    Anonymous,
    UseCredentials,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssUrlReferrerPolicyModifierValue {
    NoReferrer,
    NoReferrerWhenDowngrade,
    SameOrigin,
    Origin,
    StrictOrigin,
    OriginWhenCrossOrigin,
    StrictOriginWhenCrossOrigin,
    UnsafeUrl,
}

#[repr(C)]
pub struct CssUrlModifier {
    pub kind: CssUrlModifierKind,
    pub cross_origin_value: CssUrlCrossOriginModifierValue,
    pub referrer_policy_value: CssUrlReferrerPolicyModifierValue,
    pub integrity_ptr: *const u8,
    pub integrity_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontSourceKind {
    Local,
    Url,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontTech {
    Avar2,
    ColorCbdt,
    ColorColrv0,
    ColorColrv1,
    ColorSbix,
    ColorSvg,
    FeaturesAat,
    FeaturesGraphite,
    FeaturesOpentype,
    Incremental,
    Palettes,
    Variations,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontLanguageOverrideKind {
    Normal,
    String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssCounterStyleKind {
    Name,
    SymbolsFunction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssCounterStyleSymbolsType {
    Cyclic,
    Numeric,
    Alphabetic,
    Symbolic,
    Fixed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssNonnegativeIntegerSymbolPairOrder {
    IntegerFirst,
    SymbolFirst,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssCounterStyleNegativeSymbolCount {
    One,
    Two,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssCounterStyleSystemKind {
    Cyclic,
    Numeric,
    Alphabetic,
    Symbolic,
    Additive,
    Fixed,
    FixedWithInteger,
    Extends,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssCounterStyleRangeKind {
    Auto,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssCropOrCrossKind {
    Crop,
    Cross,
    CropAndCross,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssContainerTypeValueKind {
    Invalid,
    Normal,
    Size,
    InlineSize,
    ScrollState,
    SizeAndScrollState,
    InlineSizeAndScrollState,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssContainValueKind {
    Invalid,
    None,
    Strict,
    Content,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssContainValue {
    pub kind: CssContainValueKind,
    pub is_size: bool,
    pub is_inline_size: bool,
    pub has_layout: bool,
    pub has_style: bool,
    pub has_paint: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssWhiteSpaceTrimValueKind {
    Invalid,
    None,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssWhiteSpaceTrimValue {
    pub kind: CssWhiteSpaceTrimValueKind,
    pub has_discard_before: bool,
    pub has_discard_after: bool,
    pub has_discard_inner: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssColorSchemeValueKind {
    Invalid,
    Normal,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssColorSchemeValue {
    pub kind: CssColorSchemeValueKind,
    pub only: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssAnchorNameOrScopeValueKind {
    Invalid,
    None,
    All,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPositionAnchorValueKind {
    Invalid,
    Normal,
    None,
    Auto,
    AnchorName,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPositionVisibilityValueKind {
    Invalid,
    Always,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPositionTryOrderValue {
    Invalid,
    Normal,
    MostWidth,
    MostHeight,
    MostBlockSize,
    MostInlineSize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssPositionVisibilityValue {
    pub kind: CssPositionVisibilityValueKind,
    pub has_anchors_valid: bool,
    pub has_anchors_visible: bool,
    pub has_no_overflow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPaintOrderValueKind {
    Invalid,
    Normal,
    Keyword,
    Pair,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPaintOrderKeyword {
    Invalid,
    Fill,
    Stroke,
    Markers,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssPaintOrderValue {
    pub kind: CssPaintOrderValueKind,
    pub first: CssPaintOrderKeyword,
    pub second: CssPaintOrderKeyword,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTextUnderlinePositionHorizontal {
    Invalid,
    Auto,
    FromFont,
    Under,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTextUnderlinePositionVertical {
    Invalid,
    Auto,
    Left,
    Right,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssTextUnderlinePositionValue {
    pub horizontal: CssTextUnderlinePositionHorizontal,
    pub vertical: CssTextUnderlinePositionVertical,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTextWrapModeValue {
    Invalid,
    Wrap,
    Nowrap,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTextWrapStyleValue {
    Invalid,
    Auto,
    Balance,
    Stable,
    Pretty,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTextWrapValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssTextWrapValue {
    pub kind: CssTextWrapValueKind,
    pub mode: CssTextWrapModeValue,
    pub style: CssTextWrapStyleValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTouchActionValueKind {
    Invalid,
    Auto,
    None,
    Manipulation,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTouchActionKeyword {
    Invalid,
    PanX,
    PanLeft,
    PanRight,
    PanY,
    PanUp,
    PanDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssTouchActionValue {
    pub kind: CssTouchActionValueKind,
    pub first: CssTouchActionKeyword,
    pub second: CssTouchActionKeyword,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTimelineScopeValueKind {
    Invalid,
    None,
    All,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTimelineNameValueKind {
    Invalid,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTimelineNameItemKind {
    None,
    DashedIdent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssScrollbarGutterValueKind {
    Invalid,
    Auto,
    Stable,
    BothEdges,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssQuotesValueKind {
    Invalid,
    Auto,
    None,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssWillChangeValueKind {
    Invalid,
    Auto,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssWillChangeFeatureKind {
    ScrollPosition,
    Contents,
    CustomIdent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTransitionPropertyValueKind {
    Invalid,
    None,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTransitionBehaviorValueKind {
    Invalid,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTransitionBehaviorItemKind {
    Normal,
    AllowDiscrete,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssAnimationNameValueKind {
    Invalid,
    List,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssAnimationNameItemKind {
    None,
    CustomIdent,
    String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssViewTransitionNameValueKind {
    Invalid,
    None,
    CustomIdent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontFamilyValueKind {
    Generic,
    FamilyName,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontStyleKind {
    Normal,
    Italic,
    Left,
    Right,
    Oblique,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontVariantAlternatesValueKind {
    Stylistic,
    HistoricalForms,
    Styleset,
    CharacterVariant,
    Swash,
    Ornaments,
    Annotation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontVariantEastAsianValueKind {
    Variant,
    Width,
    Ruby,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontVariantNumericValueKind {
    Figure,
    Spacing,
    Fraction,
    Ordinal,
    SlashedZero,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontVariantLigaturesValueKind {
    Common,
    Discretionary,
    Historical,
    Contextual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFontVariantSimpleValueKind {
    LigaturesNone,
    Caps,
    Emoji,
    Position,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssOpenTypeSettingsKind {
    Normal,
    TagValues,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssOpenTypeTaggedValueKind {
    Implicit,
    On,
    Off,
    Value,
}

#[repr(C)]
pub struct CssDeclaration {
    pub is_valid: bool,
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub important: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssRuleEventKind {
    Invalid,
    AtRuleStart,
    AtRuleEnd,
    QualifiedRuleStart,
    QualifiedRuleEnd,
    PreludeStart,
    PreludeEnd,
    ChildRulesStart,
    ChildRulesEnd,
    DeclarationsStart,
    DeclarationsEnd,
    ListOfDeclarationsStart,
    ListOfDeclarationsEnd,
    DeclarationStart,
    DeclarationEnd,
}

#[repr(C)]
pub struct CssRuleEvent {
    pub kind: CssRuleEventKind,
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub important: bool,
    pub is_block_rule: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssBooleanExpressionEventKind {
    Invalid,
    NotStart,
    NotEnd,
    ParensStart,
    ParensEnd,
    AndStart,
    AndEnd,
    OrStart,
    OrEnd,
    TestStart,
    TestEnd,
    GeneralEnclosedStart,
    GeneralEnclosedEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssSupportsFeatureKind {
    Declaration,
    Selector,
    FontTech,
    FontFormat,
    Env,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssMediaFeatureSyntaxKind {
    Boolean,
    Plain,
    HalfRangeNameFirst,
    HalfRangeValueFirst,
    Range,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssMediaFeatureNameKind {
    Normal,
    Min,
    Max,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssMediaFeatureComparison {
    Equal,
    LessThan,
    LessThanOrEqual,
    GreaterThan,
    GreaterThanOrEqual,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssMediaFeatureValueKind {
    Value,
    LeftValue,
    RightValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssMediaFeatureValueSyntaxKind {
    Ident,
    Boolean,
    Integer,
    Length,
    Ratio,
    Resolution,
    Unknown,
    Invalid,
}

#[repr(C)]
pub struct CssMediaFeature {
    pub syntax_kind: CssMediaFeatureSyntaxKind,
    pub name_kind: CssMediaFeatureNameKind,
    pub id: u8,
    pub comparison: CssMediaFeatureComparison,
    pub left_comparison: CssMediaFeatureComparison,
    pub right_comparison: CssMediaFeatureComparison,
}

#[repr(C)]
pub struct CssMediaFeatureValue {
    pub kind: CssMediaFeatureValueKind,
    pub syntax_kind: CssMediaFeatureValueSyntaxKind,
    pub component_value: CssComponentValue,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssMediaTypeKind {
    None,
    All,
    Print,
    Screen,
    Unknown,
}

#[repr(C)]
pub struct CssMediaQuery {
    pub is_negated: bool,
    pub has_media_condition: bool,
    pub media_type_kind: CssMediaTypeKind,
    pub media_type_ptr: *const u8,
    pub media_type_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssValueTypeSyntaxKind {
    Invalid,
    FontKerningValueAuto,
    FontKerningValueNormal,
    FontKerningValueNone,
    FontOpticalSizingValueAuto,
    FontOpticalSizingValueNone,
    FontVariantCapsValueNormal,
    FontVariantCapsValueSmallCaps,
    FontVariantCapsValueAllSmallCaps,
    FontVariantCapsValuePetiteCaps,
    FontVariantCapsValueAllPetiteCaps,
    FontVariantCapsValueUnicase,
    FontVariantCapsValueTitlingCaps,
    FontVariantCss2Normal,
    FontVariantCss2SmallCaps,
    FontVariantEmojiValueNormal,
    FontVariantEmojiValueText,
    FontVariantEmojiValueEmoji,
    FontVariantEmojiValueUnicode,
    FontVariantPositionValueNormal,
    FontVariantPositionValueSub,
    FontVariantPositionValueSuper,
    FontWeightAbsoluteNormal,
    FontWeightAbsoluteBold,
    FontWeightAbsoluteNumber,
    FontWidthCss3Normal,
    FontWidthCss3UltraCondensed,
    FontWidthCss3ExtraCondensed,
    FontWidthCss3Condensed,
    FontWidthCss3SemiCondensed,
    FontWidthCss3SemiExpanded,
    FontWidthCss3Expanded,
    FontWidthCss3ExtraExpanded,
    FontWidthCss3UltraExpanded,
    SymbolString,
    SymbolCustomIdent,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssSyntaxNodeKind {
    Invalid,
    Universal,
    Type,
    Ident,
    MultiplierStart,
    MultiplierEnd,
    CommaSeparatedMultiplierStart,
    CommaSeparatedMultiplierEnd,
    AlternativesStart,
    AlternativesEnd,
}

#[repr(C)]
pub struct CssSyntaxNode {
    pub kind: CssSyntaxNodeKind,
    pub value_ptr: *const u8,
    pub value_len: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FamilyName {
    pub(crate) name: String,
    pub(crate) is_string: bool,
}

pub(crate) fn parse_a_list_of_component_values<F>(filtered_input: &[u8], mut callback: F)
where
    F: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    for component_value in parser.parse_a_list_of_component_values() {
        emit_component_value(&component_value, filtered_input_string, &mut callback);
    }
}

pub(crate) fn parse_a_comma_separated_list_of_component_values<G, C>(
    filtered_input: &[u8],
    mut group_callback: G,
    mut component_value_callback: C,
) where
    G: FnMut(),
    C: FnMut(CssComponentValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    for group in parser.parse_a_comma_separated_list_of_component_values() {
        for component_value in group {
            emit_component_value(&component_value, filtered_input_string, &mut component_value_callback);
        }
        group_callback();
    }
}

pub(crate) fn parse_empty_prelude(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    strip_whitespace(&component_values).is_empty()
}

pub(crate) fn parse_a_value_type(filtered_input: &[u8], value_type_id: u8) -> CssValueTypeSyntaxKind {
    let Some(value_type_id) = value_type_id_from_u8(value_type_id) else {
        return CssValueTypeSyntaxKind::Invalid;
    };

    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    component_values_parse_as_value_type(value_type_id, &component_values)
}

pub(crate) fn parse_property_keyword_value<C>(property_ids: &[u16], keyword: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16, &str),
{
    let Ok(keyword) = std::str::from_utf8(keyword) else {
        return false;
    };

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        if !property_accepts_keyword(property_id, keyword) {
            continue;
        }

        let resolved_keyword = resolve_legacy_value_alias(property_id, keyword).unwrap_or(keyword);
        callback(property_id as u16, resolved_keyword);
        return true;
    }

    false
}

pub(crate) fn property_accepting_type<C>(property_ids: &[u16], value_type: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16),
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

        callback(property_id as u16);
        return true;
    }

    false
}

pub(crate) fn parse_property_custom_ident_value<C>(property_ids: &[u16], filtered_input: &[u8], mut callback: C) -> bool
where
    C: FnMut(u16, &str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    for property_id in property_ids {
        let Some(property_id) = property_id_from_u16(*property_id) else {
            continue;
        };
        if !property_accepts_value_type(property_id, crate::generated_properties::PropertyValueType::CustomIdent) {
            continue;
        }

        let mut parser = ComponentValueParser::new(component_values.clone());
        let Some(name) = parser.parse_a_custom_ident(property_custom_ident_blacklist(property_id)) else {
            continue;
        };

        callback(property_id as u16, &name);
        return true;
    }

    false
}

fn numeric_range_limit_to_f64(limit: Option<f64>, value_type: PropertyValueType, is_minimum: bool) -> f64 {
    match (limit, value_type, is_minimum) {
        (Some(limit), _, _) => limit,
        (None, PropertyValueType::Integer, true) => i32::MIN as f64,
        (None, PropertyValueType::Integer, false) => i32::MAX as f64,
        (None, _, true) => f32::MIN as f64,
        (None, _, false) => f32::MAX as f64,
    }
}

fn numeric_range_to_f64(range: PropertyNumericRange, value_type: PropertyValueType) -> (f64, f64) {
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
        let Some(range) = property_accepted_range_by_value_type(property_id, value_type) else {
            continue;
        };

        let (minimum, maximum) = numeric_range_to_f64(range, value_type);
        let percentages_resolve_to_value_type =
            property_resolves_percentages_relative_to(property_id) == Some(value_type);
        if percentages_resolve_to_value_type {
            let Some(percentage_range) =
                property_accepted_range_by_value_type(property_id, PropertyValueType::Percentage)
            else {
                continue;
            };
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

fn parse_as_syntax_string(input: &str, limit_single_component_ident_to_custom_ident: bool) -> Option<SyntaxNode> {
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

fn emit_open_type_settings<K, V>(
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

pub(crate) fn parse_a_counter_style_name<N>(filtered_input: &[u8], mut name_callback: N) -> bool
where
    N: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(name) = parser.parse_a_counter_style_name() else {
        return false;
    };

    name_callback(&name);
    true
}

pub(crate) fn parse_a_counter_style<C, S>(
    filtered_input: &[u8],
    mut counter_style_callback: C,
    mut symbol_callback: S,
) -> bool
where
    C: FnMut(CssCounterStyleKind, CssCounterStyleSymbolsType, Option<&str>),
    S: FnMut(&str),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(counter_style) = parser.parse_a_counter_style() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    match counter_style {
        CounterStyle::Name(name) => {
            counter_style_callback(
                CssCounterStyleKind::Name,
                CssCounterStyleSymbolsType::Symbolic,
                Some(&name),
            );
        }
        CounterStyle::SymbolsFunction { symbols_type, symbols } => {
            counter_style_callback(CssCounterStyleKind::SymbolsFunction, symbols_type, None);
            for symbol in &symbols {
                symbol_callback(symbol);
            }
        }
    }

    true
}

pub(crate) fn parse_a_nonnegative_integer_symbol_pair<O>(filtered_input: &[u8], mut order_callback: O) -> bool
where
    O: FnMut(CssNonnegativeIntegerSymbolPairOrder),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(order) = parser.parse_a_nonnegative_integer_symbol_pair() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    order_callback(order);
    true
}

pub(crate) fn parse_counter_style_negative<N>(filtered_input: &[u8], mut count_callback: N) -> bool
where
    N: FnMut(CssCounterStyleNegativeSymbolCount),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(count) = parser.parse_counter_style_negative() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    count_callback(count);
    true
}

pub(crate) fn parse_counter_style_system<S>(filtered_input: &[u8], mut system_callback: S) -> bool
where
    S: FnMut(CssCounterStyleSystemKind),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(system) = parser.parse_counter_style_system() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    system_callback(system);
    true
}

pub(crate) fn parse_counter_style_symbols<C>(filtered_input: &[u8], mut count_callback: C) -> bool
where
    C: FnMut(usize),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(count) = parser.parse_counter_style_symbols() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    count_callback(count);
    true
}

pub(crate) fn parse_counter_style_symbol(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    if parser.parse_counter_style_symbol().is_none() {
        return false;
    }
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    true
}

pub(crate) fn parse_string_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    // https://drafts.csswg.org/css-values-4/#strings
    // <string>
    component_values_parse_as_string(strip_whitespace(&component_values))
}

pub(crate) fn parse_length_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-values-4/#lengths
    // <length>
    matches!(component_values, [component_value] if component_value_parse_as_length_descriptor(component_value))
}

pub(crate) fn parse_positive_percentage_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-values-4/#percentages
    // <percentage [0,∞]>
    matches!(component_values, [component_value] if component_value_parse_as_positive_percentage_descriptor(component_value))
}

pub(crate) fn parse_page_size_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-page-3/#page-size-prop
    // <length [0,∞]>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    if parser.parse_page_size_descriptor().is_none() {
        return false;
    }
    parser.discard_whitespace();
    !parser.has_next_component_value()
}

pub(crate) fn parse_optional_declaration_value_descriptor(filtered_input: &[u8]) -> bool {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let component_values = strip_whitespace(&component_values);

    // https://drafts.csswg.org/css-syntax/#typedef-declaration-value
    // The <declaration-value> production matches any sequence of one or more tokens, so long as the sequence does
    // not contain <bad-string-token>, <bad-url-token>, unmatched <)-token>, <]-token>, or <}-token>, or top-level
    // <semicolon-token> tokens or <delim-token> tokens with a value of "!". It represents the entirety of what a
    // valid declaration can have as its value.
    //
    // https://drafts.css-houdini.org/css-properties-values-api/#the-initial-value-descriptor
    // <declaration-value>?
    component_values.is_empty() || contains_only_declaration_value(component_values, Nested::No)
}

pub(crate) fn parse_counter_style_range<R>(filtered_input: &[u8], mut range_callback: R) -> bool
where
    R: FnMut(CssCounterStyleRangeKind, usize),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some((kind, count)) = parser.parse_counter_style_range() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    range_callback(kind, count);
    true
}

pub(crate) fn parse_counter_style_additive_symbols<C>(filtered_input: &[u8], mut count_callback: C) -> bool
where
    C: FnMut(usize),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(count) = parser.parse_counter_style_additive_symbols() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    count_callback(count);
    true
}

pub(crate) fn parse_crop_or_cross<C>(filtered_input: &[u8], mut kind_callback: C) -> bool
where
    C: FnMut(CssCropOrCrossKind),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(kind) = parser.parse_crop_or_cross() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    kind_callback(kind);
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

pub(crate) fn parse_font_weight_absolute_pair<C>(filtered_input: &[u8], mut count_callback: C) -> bool
where
    C: FnMut(usize),
{
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();

    let mut parser = ComponentValueParser::new(component_values);
    let Some(count) = parser.parse_font_weight_absolute_pair() else {
        return false;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return false;
    }

    count_callback(count);
    true
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

fn emit_not_all_media_query<Q>(media_query_callback: &mut Q)
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

fn emit_media_query_syntax<Q, E, M, V, C>(
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

fn css_media_type_kind(media_type: &str) -> CssMediaTypeKind {
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

fn parser_from_filtered_input(filtered_input: &[u8]) -> (Parser, &str) {
    let mut tokens = Vec::new();
    let filtered_input_string = std::str::from_utf8(filtered_input)
        .expect("rust_css_parse_component_values received non-UTF-8 input after C++ decoding");
    crate::css_tokenizer::tokenize(filtered_input, |token, _| {
        tokens.push(token.clone());
    });

    (Parser::new(tokens), filtered_input_string)
}

fn token_is_delim(token: &Token, value: char) -> bool {
    matches!(token.token_type, TokenType::Delim { value: delimiter } if delimiter == value as u32)
}

fn token_original_source<'a>(token: &Token, filtered_input: &'a str) -> Option<&'a str> {
    token.original_source(filtered_input)
}

impl Token {
    fn is_unicode_range_ending_token(&self) -> bool {
        matches!(
            self.token_type,
            TokenType::EndOfFile | TokenType::Comma | TokenType::Semicolon | TokenType::Whitespace
        )
    }
}

// https://www.w3.org/TR/css-syntax-3/#urange-syntax
fn parse_unicode_range_text(text: &str) -> Option<CssUnicodeRange> {
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

fn parse_url_or_src_function_contents(
    function_type: CssUrlFunctionType,
    component_values: &[ComponentValue],
) -> Option<UrlFunction> {
    let mut parser = ComponentValueParser::new(component_values.to_vec());
    parser.discard_whitespace();
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::String { value: url },
        ..
    }) = parser.consume_the_next_component_value()?
    else {
        return None;
    };
    parser.discard_whitespace();

    // NB: Currently <request-url-modifier> is the only kind of <url-modifier>
    // https://drafts.csswg.org/css-values-5/#request-url-modifiers
    // <request-url-modifier> = <cross-origin-modifier> | <integrity-modifier> | <referrer-policy-modifier>
    let mut request_url_modifiers = Vec::new();
    while parser.has_next_component_value() {
        let modifier = parse_request_url_modifier(&parser.consume_the_next_component_value()?)?;

        // AD-HOC: This isn't mentioned in the spec, but WPT expects modifiers to be unique (one per type).
        // Spec issue: https://github.com/w3c/csswg-drafts/issues/12151
        if request_url_modifiers
            .iter()
            .any(|existing_modifier: &UrlModifier| existing_modifier.kind() == modifier.kind())
        {
            return None;
        }
        request_url_modifiers.push(modifier);
        parser.discard_whitespace();
    }

    // AD-HOC: This isn't mentioned in the spec, but WPT expects modifiers to be sorted alphabetically.
    // Spec issue: https://github.com/w3c/csswg-drafts/issues/12151
    request_url_modifiers.sort_by_key(UrlModifier::kind);

    Some(UrlFunction {
        function_type,
        url,
        request_url_modifiers,
    })
}

fn parse_request_url_modifier(component_value: &ComponentValue) -> Option<UrlModifier> {
    match component_value {
        ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("cross-origin") => {
            parse_cross_origin_modifier(function)
        }
        ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("integrity") => {
            parse_integrity_modifier(function)
        }
        ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("referrer-policy") => {
            parse_referrer_policy_modifier(function)
        }
        _ => None,
    }
}

fn parse_cross_origin_modifier(function: &Function) -> Option<UrlModifier> {
    // <cross-origin-modifier> = cross-origin(anonymous | use-credentials)
    let ident = parse_single_ident_from_function(function)?;
    let value = match ident.as_str() {
        value if value.eq_ignore_ascii_case("anonymous") => CssUrlCrossOriginModifierValue::Anonymous,
        value if value.eq_ignore_ascii_case("use-credentials") => CssUrlCrossOriginModifierValue::UseCredentials,
        _ => return None,
    };
    Some(UrlModifier::CrossOrigin(value))
}

fn parse_integrity_modifier(function: &Function) -> Option<UrlModifier> {
    // <integrity-modifier> = integrity(<string>)
    let mut parser = ComponentValueParser::new(function.value.clone());
    parser.discard_whitespace();
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::String { value: integrity },
        ..
    }) = parser.consume_the_next_component_value()?
    else {
        return None;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }
    Some(UrlModifier::Integrity(integrity))
}

fn parse_referrer_policy_modifier(function: &Function) -> Option<UrlModifier> {
    // <referrer-policy-modifier> = (no-referrer | no-referrer-when-downgrade | same-origin | origin | strict-origin | origin-when-cross-origin | strict-origin-when-cross-origin | unsafe-url)
    let ident = parse_single_ident_from_function(function)?;
    let value = match ident.as_str() {
        value if value.eq_ignore_ascii_case("no-referrer") => CssUrlReferrerPolicyModifierValue::NoReferrer,
        value if value.eq_ignore_ascii_case("no-referrer-when-downgrade") => {
            CssUrlReferrerPolicyModifierValue::NoReferrerWhenDowngrade
        }
        value if value.eq_ignore_ascii_case("same-origin") => CssUrlReferrerPolicyModifierValue::SameOrigin,
        value if value.eq_ignore_ascii_case("origin") => CssUrlReferrerPolicyModifierValue::Origin,
        value if value.eq_ignore_ascii_case("strict-origin") => CssUrlReferrerPolicyModifierValue::StrictOrigin,
        value if value.eq_ignore_ascii_case("origin-when-cross-origin") => {
            CssUrlReferrerPolicyModifierValue::OriginWhenCrossOrigin
        }
        value if value.eq_ignore_ascii_case("strict-origin-when-cross-origin") => {
            CssUrlReferrerPolicyModifierValue::StrictOriginWhenCrossOrigin
        }
        value if value.eq_ignore_ascii_case("unsafe-url") => CssUrlReferrerPolicyModifierValue::UnsafeUrl,
        _ => return None,
    };
    Some(UrlModifier::ReferrerPolicy(value))
}

fn parse_font_format_function(function: &Function) -> Option<(String, Vec<CssFontTech>)> {
    // <font-format> = [ <string> | collection | embedded-opentype | opentype | svg | truetype | woff | woff2 ]
    let mut parser = ComponentValueParser::new(function.value.clone());
    parser.discard_whitespace();
    let format_name_token = parser.consume_the_next_component_value()?;
    let (format, tech) = match format_name_token {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) => (value, Vec::new()),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { value },
            ..
        }) => parse_font_format_string(&value)?,
        _ => return None,
    };

    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }
    Some((format, tech))
}

fn parse_font_format_string(value: &str) -> Option<(String, Vec<CssFontTech>)> {
    // https://drafts.csswg.org/css-fonts-4/#font-face-src-parsing
    // There's a fixed set of strings allowed here, which we'll assume are case-insensitive:
    // format("woff2")                 -> format(woff2)
    // format("woff")                  -> format(woff)
    // format("truetype")              -> format(truetype)
    // format("opentype")              -> format(opentype)
    // format("collection")            -> format(collection)
    // format("woff2-variations")      -> format(woff2) tech(variations)
    // format("woff-variations")       -> format(woff) tech(variations)
    // format("truetype-variations")   -> format(truetype) tech(variations)
    // format("opentype-variations")   -> format(opentype) tech(variations)
    let (format, has_variations) = match value {
        value if value.eq_ignore_ascii_case("woff2") => ("woff2", false),
        value if value.eq_ignore_ascii_case("woff") => ("woff", false),
        value if value.eq_ignore_ascii_case("truetype") => ("truetype", false),
        value if value.eq_ignore_ascii_case("opentype") => ("opentype", false),
        value if value.eq_ignore_ascii_case("collection") => ("collection", false),
        value if value.eq_ignore_ascii_case("woff2-variations") => ("woff2", true),
        value if value.eq_ignore_ascii_case("woff-variations") => ("woff", true),
        value if value.eq_ignore_ascii_case("truetype-variations") => ("truetype", true),
        value if value.eq_ignore_ascii_case("opentype-variations") => ("opentype", true),
        _ => return None,
    };

    Some((
        format.to_string(),
        if has_variations {
            vec![CssFontTech::Variations]
        } else {
            Vec::new()
        },
    ))
}

fn parse_font_tech_function(function: &Function) -> Option<Vec<CssFontTech>> {
    // <font-tech> = features-opentype | features-aat | features-graphite | variations | color-COLRv0 | color-COLRv1
    //             | color-SVG | color-sbix | color-CBDT | palettes | incremental
    let mut groups = Vec::new();
    let mut current_group = Vec::new();
    for component_value in &function.value {
        if matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            })
        ) {
            groups.push(current_group);
            current_group = Vec::new();
            continue;
        }
        current_group.push(component_value.clone());
    }
    groups.push(current_group);

    if groups.is_empty() {
        return None;
    }

    let mut tech = Vec::new();
    for group in groups {
        let mut parser = ComponentValueParser::new(group);
        parser.discard_whitespace();
        let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) = parser.consume_the_next_component_value()?
        else {
            return None;
        };
        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }

        tech.push(parse_font_tech_name(&value)?);
    }

    Some(tech)
}

fn parse_comma_separated_component_values<T, F>(
    component_values: Vec<ComponentValue>,
    mut parse_group: F,
) -> Option<Vec<T>>
where
    F: FnMut(Vec<ComponentValue>) -> Option<T>,
{
    let mut groups = Vec::new();
    let mut current_group = Vec::new();
    for component_value in component_values {
        if matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            })
        ) {
            groups.push(current_group);
            current_group = Vec::new();
            continue;
        }
        current_group.push(component_value);
    }
    groups.push(current_group);

    if groups.is_empty() {
        return None;
    }

    groups.into_iter().map(&mut parse_group).collect()
}

fn parse_font_variant_alternates_feature_value_names(component_values: Vec<ComponentValue>) -> Option<Vec<String>> {
    let groups = parse_comma_separated_component_values(component_values, |component_values| {
        let mut parser = ComponentValueParser::new(component_values);
        parser.parse_a_custom_ident(&[])
    })?;

    (!groups.is_empty()).then_some(groups)
}

// https://drafts.csswg.org/css-fonts/#typedef-opentype-tag
fn parse_opentype_tag(parser: &mut ComponentValueParser) -> Option<String> {
    // <opentype-tag> = <string>
    // The <opentype-tag> is a case-sensitive OpenType feature tag.
    // As specified in the OpenType specification [OPENTYPE], feature tags contain four ASCII characters.
    // Tag strings longer or shorter than four characters, or containing characters outside the U+20–7E codepoint range are invalid.
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::String { value },
        ..
    }) = parser.consume_the_next_component_value()?
    else {
        return None;
    };

    (value.len() == 4 && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))).then_some(value)
}

fn parse_feature_tag_value(component_values: Vec<ComponentValue>, filtered_input: &str) -> Option<OpenTypeTaggedValue> {
    // <feature-tag-value> = <opentype-tag> [ <integer [0,∞]> | on | off ]?
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();
    let tag = parse_opentype_tag(&mut parser)?;
    parser.discard_whitespace();

    if !parser.has_next_component_value() {
        // "If the value is omitted, a value of 1 is assumed."
        return Some(OpenTypeTaggedValue {
            tag,
            value_kind: CssOpenTypeTaggedValueKind::Implicit,
            value: None,
        });
    }

    if let Some(ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value },
        ..
    })) = parser.next_component_value()
    {
        // A value of on is synonymous with 1 and off is synonymous with 0.
        let value_kind = if value.eq_ignore_ascii_case("on") {
            CssOpenTypeTaggedValueKind::On
        } else if value.eq_ignore_ascii_case("off") {
            CssOpenTypeTaggedValueKind::Off
        } else {
            CssOpenTypeTaggedValueKind::Value
        };

        if value_kind != CssOpenTypeTaggedValueKind::Value {
            parser.index += 1;
            parser.discard_whitespace();
            if parser.has_next_component_value() {
                return None;
            }
            return Some(OpenTypeTaggedValue {
                tag,
                value_kind,
                value: None,
            });
        }
    }

    let value = serialize_component_values_for_reparsing(parser.remaining_component_values(), filtered_input)?;
    Some(OpenTypeTaggedValue {
        tag,
        value_kind: CssOpenTypeTaggedValueKind::Value,
        value: Some(value),
    })
}

fn parse_variation_tag_value(
    component_values: Vec<ComponentValue>,
    filtered_input: &str,
) -> Option<OpenTypeTaggedValue> {
    // [ <opentype-tag> <number>]
    let mut parser = ComponentValueParser::new(component_values);
    parser.discard_whitespace();
    let tag = parse_opentype_tag(&mut parser)?;
    parser.discard_whitespace();

    if !parser.has_next_component_value() {
        return None;
    }

    let value = serialize_component_values_for_reparsing(parser.remaining_component_values(), filtered_input)?;
    Some(OpenTypeTaggedValue {
        tag,
        value_kind: CssOpenTypeTaggedValueKind::Value,
        value: Some(value),
    })
}

fn parse_font_tech_name(value: &str) -> Option<CssFontTech> {
    match value {
        value if value.eq_ignore_ascii_case("avar2") => Some(CssFontTech::Avar2),
        value if value.eq_ignore_ascii_case("color-cbdt") => Some(CssFontTech::ColorCbdt),
        value if value.eq_ignore_ascii_case("color-colrv0") => Some(CssFontTech::ColorColrv0),
        value if value.eq_ignore_ascii_case("color-colrv1") => Some(CssFontTech::ColorColrv1),
        value if value.eq_ignore_ascii_case("color-sbix") => Some(CssFontTech::ColorSbix),
        value if value.eq_ignore_ascii_case("color-svg") => Some(CssFontTech::ColorSvg),
        value if value.eq_ignore_ascii_case("features-aat") => Some(CssFontTech::FeaturesAat),
        value if value.eq_ignore_ascii_case("features-graphite") => Some(CssFontTech::FeaturesGraphite),
        value if value.eq_ignore_ascii_case("features-opentype") => Some(CssFontTech::FeaturesOpentype),
        value if value.eq_ignore_ascii_case("incremental") => Some(CssFontTech::Incremental),
        value if value.eq_ignore_ascii_case("palettes") => Some(CssFontTech::Palettes),
        value if value.eq_ignore_ascii_case("variations") => Some(CssFontTech::Variations),
        _ => None,
    }
}

fn parse_font_language_override_string_value(value: &str) -> Option<String> {
    // https://drafts.csswg.org/css-fonts/#propdef-font-language-override
    // This is `normal | <string>` but with the constraint that the string has to be 4 characters long:
    // Shorter strings are right-padded with spaces before use, and longer strings are invalid.
    let length = value.len();
    if length == 0 || length > 4 || !value.is_ascii() {
        return None;
    }

    // We're expected to always serialize without any trailing spaces, so remove them now for convenience.
    let trimmed = value.trim_end_matches(|code_point: char| code_point.is_ascii_whitespace());
    if trimmed.is_empty() {
        return None;
    }

    Some(trimmed.to_string())
}

fn parse_single_ident_from_function(function: &Function) -> Option<String> {
    let mut parser = ComponentValueParser::new(function.value.clone());
    parser.discard_whitespace();
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Ident { value: ident },
        ..
    }) = parser.consume_the_next_component_value()?
    else {
        return None;
    };
    parser.discard_whitespace();
    if parser.has_next_component_value() {
        return None;
    }
    Some(ident)
}

fn string_parts(string: &str) -> (*const u8, usize) {
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
    fn new(kind: CssRuleEventKind) -> Self {
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
    fn new(kind: CssSyntaxNodeKind) -> Self {
        Self {
            kind,
            value_ptr: std::ptr::null(),
            value_len: 0,
        }
    }
}

fn emit_component_value<F>(component_value: &ComponentValue, filtered_input: &str, callback: &mut F)
where
    F: FnMut(CssComponentValue),
{
    match component_value {
        ComponentValue::PreservedToken(token) => {
            emit_token(CssComponentValueKind::Token, token, filtered_input, callback);
        }
        ComponentValue::Function(function) => {
            emit_token(
                CssComponentValueKind::FunctionStart,
                &function.name_token,
                filtered_input,
                callback,
            );
            for value in &function.value {
                emit_component_value(value, filtered_input, callback);
            }
            emit_token(
                CssComponentValueKind::FunctionEnd,
                &function.end_token,
                filtered_input,
                callback,
            );
        }
        ComponentValue::SimpleBlock(block) => {
            emit_token(
                CssComponentValueKind::SimpleBlockStart,
                &block.token,
                filtered_input,
                callback,
            );
            for value in &block.value {
                emit_component_value(value, filtered_input, callback);
            }
            emit_token(
                CssComponentValueKind::SimpleBlockEnd,
                &block.end_token,
                filtered_input,
                callback,
            );
        }
    }
}

fn emit_token<F>(kind: CssComponentValueKind, token: &Token, filtered_input: &str, callback: &mut F)
where
    F: FnMut(CssComponentValue),
{
    callback(CssComponentValue {
        kind,
        token: token.as_ffi(filtered_input),
    });
}

fn emit_syntax_node<C>(syntax_node: &SyntaxNode, callback: &mut C)
where
    C: FnMut(CssSyntaxNode),
{
    match syntax_node {
        SyntaxNode::Universal => callback(CssSyntaxNode::new(CssSyntaxNodeKind::Universal)),
        SyntaxNode::Type(type_name) => {
            let (value_ptr, value_len) = string_parts(type_name);
            callback(CssSyntaxNode {
                kind: CssSyntaxNodeKind::Type,
                value_ptr,
                value_len,
            });
        }
        SyntaxNode::Ident(ident) => {
            let (value_ptr, value_len) = string_parts(ident);
            callback(CssSyntaxNode {
                kind: CssSyntaxNodeKind::Ident,
                value_ptr,
                value_len,
            });
        }
        SyntaxNode::Multiplier(child) => {
            callback(CssSyntaxNode::new(CssSyntaxNodeKind::MultiplierStart));
            emit_syntax_node(child, callback);
            callback(CssSyntaxNode::new(CssSyntaxNodeKind::MultiplierEnd));
        }
        SyntaxNode::CommaSeparatedMultiplier(child) => {
            callback(CssSyntaxNode::new(CssSyntaxNodeKind::CommaSeparatedMultiplierStart));
            emit_syntax_node(child, callback);
            callback(CssSyntaxNode::new(CssSyntaxNodeKind::CommaSeparatedMultiplierEnd));
        }
        SyntaxNode::Alternatives(children) => {
            callback(CssSyntaxNode::new(CssSyntaxNodeKind::AlternativesStart));
            for child in children {
                emit_syntax_node(child, callback);
            }
            callback(CssSyntaxNode::new(CssSyntaxNodeKind::AlternativesEnd));
        }
    }
}

fn emit_rule<E, C>(rule: &Rule, filtered_input: &str, event_callback: &mut E, component_value_callback: &mut C)
where
    E: FnMut(CssRuleEvent),
    C: FnMut(CssComponentValue),
{
    match rule {
        Rule::AtRule(at_rule) => {
            let (name_ptr, name_len) = string_parts(&at_rule.name);
            event_callback(CssRuleEvent {
                kind: CssRuleEventKind::AtRuleStart,
                name_ptr,
                name_len,
                important: false,
                is_block_rule: at_rule.is_block_rule,
            });
            emit_component_value_list(
                &at_rule.prelude,
                filtered_input,
                event_callback,
                component_value_callback,
            );
            emit_rule_or_list_of_declarations_list(
                &at_rule.child_rules_and_lists_of_declarations,
                filtered_input,
                event_callback,
                component_value_callback,
            );
            event_callback(CssRuleEvent::new(CssRuleEventKind::AtRuleEnd));
        }
        Rule::QualifiedRule(qualified_rule) => {
            event_callback(CssRuleEvent::new(CssRuleEventKind::QualifiedRuleStart));
            emit_component_value_list(
                &qualified_rule.prelude,
                filtered_input,
                event_callback,
                component_value_callback,
            );
            event_callback(CssRuleEvent::new(CssRuleEventKind::DeclarationsStart));
            for declaration in &qualified_rule.declarations {
                emit_declaration(declaration, filtered_input, event_callback, component_value_callback);
            }
            event_callback(CssRuleEvent::new(CssRuleEventKind::DeclarationsEnd));
            emit_rule_or_list_of_declarations_list(
                &qualified_rule.child_rules,
                filtered_input,
                event_callback,
                component_value_callback,
            );
            event_callback(CssRuleEvent::new(CssRuleEventKind::QualifiedRuleEnd));
        }
    }
}

fn emit_rule_or_list_of_declarations_list<E, C>(
    rules_or_lists_of_declarations: &[RuleOrListOfDeclarations],
    filtered_input: &str,
    event_callback: &mut E,
    component_value_callback: &mut C,
) where
    E: FnMut(CssRuleEvent),
    C: FnMut(CssComponentValue),
{
    event_callback(CssRuleEvent::new(CssRuleEventKind::ChildRulesStart));
    for rule_or_list_of_declarations in rules_or_lists_of_declarations {
        match rule_or_list_of_declarations {
            RuleOrListOfDeclarations::Rule(rule) => {
                emit_rule(rule, filtered_input, event_callback, component_value_callback);
            }
            RuleOrListOfDeclarations::ListOfDeclarations(declarations) => {
                event_callback(CssRuleEvent::new(CssRuleEventKind::ListOfDeclarationsStart));
                for declaration in declarations {
                    emit_declaration(declaration, filtered_input, event_callback, component_value_callback);
                }
                event_callback(CssRuleEvent::new(CssRuleEventKind::ListOfDeclarationsEnd));
            }
        }
    }
    event_callback(CssRuleEvent::new(CssRuleEventKind::ChildRulesEnd));
}

fn emit_declaration<E, C>(
    declaration: &Declaration,
    filtered_input: &str,
    event_callback: &mut E,
    component_value_callback: &mut C,
) where
    E: FnMut(CssRuleEvent),
    C: FnMut(CssComponentValue),
{
    let (name_ptr, name_len) = string_parts(&declaration.name);
    event_callback(CssRuleEvent {
        kind: CssRuleEventKind::DeclarationStart,
        name_ptr,
        name_len,
        important: declaration.important,
        is_block_rule: false,
    });
    for value in &declaration.value {
        emit_component_value(value, filtered_input, component_value_callback);
    }
    event_callback(CssRuleEvent::new(CssRuleEventKind::DeclarationEnd));
}

fn emit_component_value_list<E, C>(
    component_values: &[ComponentValue],
    filtered_input: &str,
    event_callback: &mut E,
    component_value_callback: &mut C,
) where
    E: FnMut(CssRuleEvent),
    C: FnMut(CssComponentValue),
{
    event_callback(CssRuleEvent::new(CssRuleEventKind::PreludeStart));
    for component_value in component_values {
        emit_component_value(component_value, filtered_input, component_value_callback);
    }
    event_callback(CssRuleEvent::new(CssRuleEventKind::PreludeEnd));
}

fn emit_boolean_expression<E, C, M, V>(
    expression: &BooleanExpression,
    filtered_input: &str,
    event_callback: &mut E,
    component_value_callback: &mut C,
    media_feature_callback: &mut M,
    media_feature_value_callback: &mut V,
) where
    E: FnMut(CssBooleanExpressionEventKind),
    C: FnMut(CssComponentValue),
    M: FnMut(CssMediaFeature),
    V: FnMut(CssMediaFeatureValue),
{
    match expression {
        BooleanExpression::Not(child) => {
            event_callback(CssBooleanExpressionEventKind::NotStart);
            emit_boolean_expression(
                child,
                filtered_input,
                event_callback,
                component_value_callback,
                media_feature_callback,
                media_feature_value_callback,
            );
            event_callback(CssBooleanExpressionEventKind::NotEnd);
        }
        BooleanExpression::Parens(child) => {
            event_callback(CssBooleanExpressionEventKind::ParensStart);
            emit_boolean_expression(
                child,
                filtered_input,
                event_callback,
                component_value_callback,
                media_feature_callback,
                media_feature_value_callback,
            );
            event_callback(CssBooleanExpressionEventKind::ParensEnd);
        }
        BooleanExpression::And(children) => {
            event_callback(CssBooleanExpressionEventKind::AndStart);
            for child in children {
                emit_boolean_expression(
                    child,
                    filtered_input,
                    event_callback,
                    component_value_callback,
                    media_feature_callback,
                    media_feature_value_callback,
                );
            }
            event_callback(CssBooleanExpressionEventKind::AndEnd);
        }
        BooleanExpression::Or(children) => {
            event_callback(CssBooleanExpressionEventKind::OrStart);
            for child in children {
                emit_boolean_expression(
                    child,
                    filtered_input,
                    event_callback,
                    component_value_callback,
                    media_feature_callback,
                    media_feature_value_callback,
                );
            }
            event_callback(CssBooleanExpressionEventKind::OrEnd);
        }
        BooleanExpression::Test(BooleanExpressionTest::SupportsFeature(component_values)) => {
            event_callback(CssBooleanExpressionEventKind::TestStart);
            for component_value in component_values {
                emit_component_value(component_value, filtered_input, component_value_callback);
            }
            event_callback(CssBooleanExpressionEventKind::TestEnd);
        }
        BooleanExpression::Test(BooleanExpressionTest::IfTest(component_values)) => {
            event_callback(CssBooleanExpressionEventKind::TestStart);
            for component_value in component_values {
                emit_component_value(component_value, filtered_input, component_value_callback);
            }
            event_callback(CssBooleanExpressionEventKind::TestEnd);
        }
        BooleanExpression::Test(BooleanExpressionTest::MediaFeature(media_feature)) => {
            event_callback(CssBooleanExpressionEventKind::TestStart);
            media_feature_callback(css_media_feature_from_syntax(&media_feature.kind));
            emit_media_feature_values(&media_feature.kind, filtered_input, media_feature_value_callback);
            emit_component_value(&media_feature.component_value, filtered_input, component_value_callback);
            event_callback(CssBooleanExpressionEventKind::TestEnd);
        }
        BooleanExpression::GeneralEnclosed(component_value) => {
            event_callback(CssBooleanExpressionEventKind::GeneralEnclosedStart);
            emit_component_value(component_value, filtered_input, component_value_callback);
            event_callback(CssBooleanExpressionEventKind::GeneralEnclosedEnd);
        }
    }
}

fn emit_media_feature_value<C>(
    kind: CssMediaFeatureValueKind,
    syntax_kind: CssMediaFeatureValueSyntaxKind,
    component_values: &[ComponentValue],
    filtered_input: &str,
    callback: &mut C,
) where
    C: FnMut(CssMediaFeatureValue),
{
    for component_value in component_values {
        emit_component_value(component_value, filtered_input, &mut |component_value| {
            callback(CssMediaFeatureValue {
                kind,
                syntax_kind,
                component_value,
            });
        });
    }
}

fn emit_media_feature_values<C>(syntax: &MediaFeatureSyntax, filtered_input: &str, callback: &mut C)
where
    C: FnMut(CssMediaFeatureValue),
{
    match syntax {
        MediaFeatureSyntax::Boolean(_) => {}
        MediaFeatureSyntax::Plain { name, value }
        | MediaFeatureSyntax::HalfRangeNameFirst { name, value, .. }
        | MediaFeatureSyntax::HalfRangeValueFirst { name, value, .. } => {
            emit_media_feature_value(
                CssMediaFeatureValueKind::Value,
                css_media_feature_value_syntax_kind_from_syntax(component_values_parse_as_mf_value_syntax(
                    name.id, value,
                )),
                value,
                filtered_input,
                callback,
            );
        }
        MediaFeatureSyntax::Range {
            left_value,
            name,
            right_value,
            ..
        } => {
            emit_media_feature_value(
                CssMediaFeatureValueKind::LeftValue,
                css_media_feature_value_syntax_kind_from_syntax(component_values_parse_as_mf_value_syntax(
                    name.id, left_value,
                )),
                left_value,
                filtered_input,
                callback,
            );
            emit_media_feature_value(
                CssMediaFeatureValueKind::RightValue,
                css_media_feature_value_syntax_kind_from_syntax(component_values_parse_as_mf_value_syntax(
                    name.id,
                    right_value,
                )),
                right_value,
                filtered_input,
                callback,
            );
        }
    }
}

fn css_media_feature_value_syntax_kind_from_syntax(
    syntax_kind: MediaFeatureValueSyntaxKind,
) -> CssMediaFeatureValueSyntaxKind {
    match syntax_kind {
        MediaFeatureValueSyntaxKind::Ident => CssMediaFeatureValueSyntaxKind::Ident,
        MediaFeatureValueSyntaxKind::Boolean => CssMediaFeatureValueSyntaxKind::Boolean,
        MediaFeatureValueSyntaxKind::Integer => CssMediaFeatureValueSyntaxKind::Integer,
        MediaFeatureValueSyntaxKind::Length => CssMediaFeatureValueSyntaxKind::Length,
        MediaFeatureValueSyntaxKind::Ratio => CssMediaFeatureValueSyntaxKind::Ratio,
        MediaFeatureValueSyntaxKind::Resolution => CssMediaFeatureValueSyntaxKind::Resolution,
        MediaFeatureValueSyntaxKind::Unknown => CssMediaFeatureValueSyntaxKind::Unknown,
        MediaFeatureValueSyntaxKind::Invalid => CssMediaFeatureValueSyntaxKind::Invalid,
    }
}

fn css_media_feature_from_syntax(syntax: &MediaFeatureSyntax) -> CssMediaFeature {
    let (syntax_kind, name, comparison, left_comparison, right_comparison) = match syntax {
        MediaFeatureSyntax::Boolean(name) => (
            CssMediaFeatureSyntaxKind::Boolean,
            *name,
            MfComparison::Equal,
            MfComparison::Equal,
            MfComparison::Equal,
        ),
        MediaFeatureSyntax::Plain { name, .. } => (
            CssMediaFeatureSyntaxKind::Plain,
            *name,
            MfComparison::Equal,
            MfComparison::Equal,
            MfComparison::Equal,
        ),
        MediaFeatureSyntax::HalfRangeNameFirst { name, comparison, .. } => (
            CssMediaFeatureSyntaxKind::HalfRangeNameFirst,
            *name,
            *comparison,
            MfComparison::Equal,
            MfComparison::Equal,
        ),
        MediaFeatureSyntax::HalfRangeValueFirst { comparison, name, .. } => (
            CssMediaFeatureSyntaxKind::HalfRangeValueFirst,
            *name,
            *comparison,
            MfComparison::Equal,
            MfComparison::Equal,
        ),
        MediaFeatureSyntax::Range {
            left_comparison,
            name,
            right_comparison,
            ..
        } => (
            CssMediaFeatureSyntaxKind::Range,
            *name,
            MfComparison::Equal,
            *left_comparison,
            *right_comparison,
        ),
    };

    CssMediaFeature {
        syntax_kind,
        name_kind: css_media_feature_name_kind(name.kind),
        id: name.id as u8,
        comparison: css_media_feature_comparison(comparison),
        left_comparison: css_media_feature_comparison(left_comparison),
        right_comparison: css_media_feature_comparison(right_comparison),
    }
}

fn css_media_feature_name_kind(kind: MediaFeatureNameKind) -> CssMediaFeatureNameKind {
    match kind {
        MediaFeatureNameKind::Normal => CssMediaFeatureNameKind::Normal,
        MediaFeatureNameKind::Min => CssMediaFeatureNameKind::Min,
        MediaFeatureNameKind::Max => CssMediaFeatureNameKind::Max,
    }
}

fn css_media_feature_comparison(comparison: MfComparison) -> CssMediaFeatureComparison {
    match comparison {
        MfComparison::Equal => CssMediaFeatureComparison::Equal,
        MfComparison::LessThan => CssMediaFeatureComparison::LessThan,
        MfComparison::LessThanOrEqual => CssMediaFeatureComparison::LessThanOrEqual,
        MfComparison::GreaterThan => CssMediaFeatureComparison::GreaterThan,
        MfComparison::GreaterThanOrEqual => CssMediaFeatureComparison::GreaterThanOrEqual,
    }
}

impl ComponentValueParser {
    fn new(component_values: Vec<ComponentValue>) -> Self {
        Self {
            component_values,
            index: 0,
            boolean_expression: None,
        }
    }

    fn next_component_value(&self) -> Option<&ComponentValue> {
        self.component_values.get(self.index)
    }

    fn consume_the_next_component_value(&mut self) -> Option<ComponentValue> {
        let component_value = self.next_component_value()?.clone();
        self.index += 1;
        Some(component_value)
    }

    fn consume_ident_matching(&mut self, expected: &str) -> bool {
        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
            && value.eq_ignore_ascii_case(expected)
        {
            self.index += 1;
            return true;
        }

        false
    }

    fn consume_an_ident(&mut self) -> Option<String> {
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
        else {
            return None;
        };
        let value = value.clone();
        self.index += 1;
        Some(value)
    }

    fn remaining_component_values(&self) -> &[ComponentValue] {
        &self.component_values[self.index..]
    }

    fn discard_whitespace(&mut self) {
        while matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Whitespace,
                ..
            }))
        ) {
            self.index += 1;
        }
    }

    fn has_next_component_value(&mut self) -> bool {
        self.discard_whitespace();
        self.next_component_value().is_some()
    }

    fn parse_a_boolean_expression(&mut self, test_kind: BooleanExpressionTestKind) -> Option<()> {
        self.boolean_expression = self.parse_boolean_expression(test_kind);
        self.boolean_expression.as_ref()?;
        Some(())
    }

    fn parse_media_condition(&mut self) -> Option<BooleanExpression> {
        // <media-condition> = <media-not> | <media-and> | <media-or> | <media-in-parens>
        self.parse_boolean_expression(BooleanExpressionTestKind::MediaFeature)
    }

    fn parse_media_condition_without_or(&mut self) -> Option<BooleanExpression> {
        // <media-condition-without-or> = <media-not> | <media-and> | <media-in-parens>
        let expression = self.parse_media_condition()?;
        if matches!(expression, BooleanExpression::Or(_)) {
            return None;
        }
        Some(expression)
    }

    fn parse_media_query_modifier(&mut self) -> MediaQueryModifier {
        // [ not | only ]?
        if component_value_is_ident(self.next_component_value(), "not") {
            self.index += 1;
            return MediaQueryModifier::Not;
        }
        if component_value_is_ident(self.next_component_value(), "only") {
            self.index += 1;
            return MediaQueryModifier::Only;
        }
        MediaQueryModifier::None
    }

    fn parse_media_type(&mut self) -> Option<String> {
        // <media-type> = <ident>
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
        else {
            return None;
        };

        // https://drafts.csswg.org/mediaqueries-3/#error-handling
        // "However, an exception is made for media types ‘layer’, ‘not’, ‘and’, ‘only’, and ‘or’.
        // Even though they do match the IDENT production, they must not be treated as unknown media
        // types, but rather trigger the malformed query clause."
        if ["layer", "not", "and", "only", "or"]
            .iter()
            .any(|reserved| value.eq_ignore_ascii_case(reserved))
        {
            return None;
        }

        let media_type = value.clone();
        self.index += 1;
        Some(media_type)
    }

    // https://drafts.csswg.org/css-values-5/#typedef-boolean-expr
    fn parse_boolean_expression(&mut self, test_kind: BooleanExpressionTestKind) -> Option<BooleanExpression> {
        // <boolean-expr[ <test> ]> = not <boolean-expr-group> | <boolean-expr-group>
        //                            [ [ and <boolean-expr-group> ]*
        //                            | [ or <boolean-expr-group> ]* ]
        let saved_index = self.index;
        self.discard_whitespace();

        // `not <boolean-expr-group>`
        if component_value_is_ident(self.next_component_value(), "not") {
            self.index += 1;
            self.discard_whitespace();

            let child = self.parse_boolean_expression_group(test_kind)?;
            self.discard_whitespace();
            return Some(BooleanExpression::Not(Box::new(child)));
        }

        // `<boolean-expr-group>
        //   [ [ and <boolean-expr-group> ]*
        //   | [ or <boolean-expr-group> ]* ]`
        #[derive(Clone, Copy, PartialEq, Eq)]
        enum Combinator {
            And,
            Or,
        }

        let mut children = Vec::new();
        let mut combinator = None;

        while self.next_component_value().is_some() {
            if !children.is_empty() {
                let maybe_combinator = if component_value_is_ident(self.next_component_value(), "and") {
                    Some(Combinator::And)
                } else if component_value_is_ident(self.next_component_value(), "or") {
                    Some(Combinator::Or)
                } else {
                    None
                };

                let maybe_combinator = maybe_combinator?;
                if let Some(combinator) = combinator {
                    if maybe_combinator != combinator {
                        self.index = saved_index;
                        return None;
                    }
                } else {
                    combinator = Some(maybe_combinator);
                }
                self.index += 1;
            }

            self.discard_whitespace();
            children.push(self.parse_boolean_expression_group(test_kind)?);
            self.discard_whitespace();
        }

        if children.is_empty() {
            self.index = saved_index;
            return None;
        }

        if children.len() == 1 {
            return children.pop();
        }

        match combinator.expect("multiple children must have a combinator") {
            Combinator::And => Some(BooleanExpression::And(children)),
            Combinator::Or => Some(BooleanExpression::Or(children)),
        }
    }

    fn parse_boolean_expression_group(&mut self, test_kind: BooleanExpressionTestKind) -> Option<BooleanExpression> {
        // <boolean-expr-group> = <test> | ( <boolean-expr[ <test> ]> ) | <general-enclosed>

        // `( <boolean-expr[ <test> ]> )`
        if let Some(ComponentValue::SimpleBlock(block)) = self.next_component_value().cloned()
            && is_paren_block(&block)
        {
            let saved_index = self.index;
            self.index += 1;
            let mut child_parser = ComponentValueParser::new(block.value);
            if let Some(expression) = child_parser.parse_boolean_expression(test_kind)
                && !child_parser.has_next_component_value()
            {
                return Some(BooleanExpression::Parens(Box::new(expression)));
            }
            self.index = saved_index;
        }

        // `<test>`
        if let Some(test) = self.parse_test(test_kind) {
            return Some(BooleanExpression::Test(test));
        }

        // `<general-enclosed>`
        if let Some(general_enclosed) = self.parse_general_enclosed() {
            return Some(BooleanExpression::GeneralEnclosed(general_enclosed));
        }

        None
    }

    fn parse_test(&mut self, test_kind: BooleanExpressionTestKind) -> Option<BooleanExpressionTest> {
        match test_kind {
            BooleanExpressionTestKind::SupportsFeature => self.parse_supports_feature(),
            BooleanExpressionTestKind::MediaFeature => self.parse_media_feature(),
            BooleanExpressionTestKind::IfTest => self.parse_if_test(),
        }
    }

    // https://drafts.csswg.org/css-conditional-5/#typedef-supports-feature
    fn parse_supports_feature(&mut self) -> Option<BooleanExpressionTest> {
        // <supports-feature> = <supports-selector-fn> | <supports-font-tech-fn>
        //                    | <supports-font-format-fn> | <supports-env-fn>
        //                    | <supports-decl>
        let (_, component_value) = self.parse_supports_feature_syntax()?;
        Some(BooleanExpressionTest::SupportsFeature(vec![component_value]))
    }

    // https://drafts.csswg.org/css-conditional-5/#typedef-supports-feature
    fn parse_supports_feature_syntax(&mut self) -> Option<(SupportsFeature, ComponentValue)> {
        // <supports-feature> = <supports-selector-fn> | <supports-font-tech-fn>
        //                    | <supports-font-format-fn> | <supports-env-fn>
        //                    | <supports-decl>
        let component_value = self.next_component_value()?.clone();

        // `<supports-decl> = ( <declaration> )`
        if let ComponentValue::SimpleBlock(block) = &component_value
            && is_paren_block(block)
            && component_values_start_like_a_declaration(&block.value)
        {
            self.index += 1;
            return Some((SupportsFeature::Declaration, component_value));
        }

        let ComponentValue::Function(function) = &component_value else {
            return None;
        };

        // `<supports-selector-fn> = selector( <complex-selector> )`
        if function.name.eq_ignore_ascii_case("selector") {
            self.index += 1;
            return Some((SupportsFeature::Selector, component_value));
        }

        // `<supports-font-tech-fn> = font-tech( <font-tech> )`
        // `<supports-font-format-fn> = font-format( <font-format> )`
        // `<supports-env-fn> = env( <ident> )`
        if function.name.eq_ignore_ascii_case("font-tech")
            || function.name.eq_ignore_ascii_case("font-format")
            || function.name.eq_ignore_ascii_case("env")
        {
            let mut parser = ComponentValueParser::new(function.value.clone());
            parser.discard_whitespace();
            let ident = parser.consume_the_next_component_value();
            parser.discard_whitespace();
            if let Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) = ident
                && parser.next_component_value().is_none()
            {
                let feature = if function.name.eq_ignore_ascii_case("font-tech") {
                    SupportsFeature::FontTech(value)
                } else if function.name.eq_ignore_ascii_case("font-format") {
                    SupportsFeature::FontFormat(value)
                } else {
                    SupportsFeature::Env(value)
                };
                self.index += 1;
                return Some((feature, component_value));
            }
        }

        None
    }

    // https://drafts.csswg.org/mediaqueries-5/#typedef-media-feature
    fn parse_media_feature(&mut self) -> Option<BooleanExpressionTest> {
        // <media-feature> = [ <mf-plain> | <mf-boolean> | <mf-range> ]
        let component_value = self.next_component_value()?.clone();
        if let ComponentValue::SimpleBlock(block) = &component_value
            && is_paren_block(block)
            && let Some(kind) = component_values_parse_as_media_feature(&block.value)
        {
            self.index += 1;
            return Some(BooleanExpressionTest::MediaFeature(Box::new(MediaFeatureTest {
                component_value,
                kind,
            })));
        }

        None
    }

    // https://drafts.csswg.org/css-values-5/#typedef-if-condition
    fn parse_if_test(&mut self) -> Option<BooleanExpressionTest> {
        // <if-test> =
        //   supports( [ <ident> : <declaration-value> ] | <supports-condition> ) |
        //   media( <media-feature> | <media-condition> ) |
        //   style( <style-query> )
        let component_value = self.next_component_value()?.clone();
        let ComponentValue::Function(function) = &component_value else {
            return None;
        };

        if function.name.eq_ignore_ascii_case("supports")
            || function.name.eq_ignore_ascii_case("media")
            || function.name.eq_ignore_ascii_case("style")
        {
            self.index += 1;
            return Some(BooleanExpressionTest::IfTest(vec![component_value]));
        }

        None
    }

    // https://drafts.csswg.org/css-page-3/#syntax-page-selector
    fn parse_a_page_selector_list(&mut self) -> Option<Vec<PageSelector>> {
        // <page-selector-list> = <page-selector>#
        // <page-selector> = [ <ident-token>? <pseudo-page>* ]!
        // <pseudo-page> = : [ left | right | first | blank ]
        let mut selector_list = Vec::new();

        self.discard_whitespace();
        while self.has_next_component_value() {
            let name = if let Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) = self.next_component_value()
            {
                let name = value.clone();
                self.index += 1;
                Some(name)
            } else {
                None
            };

            let mut pseudo_classes = Vec::new();
            while matches!(
                self.next_component_value(),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Colon,
                    ..
                }))
            ) {
                self.index += 1;
                let Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                })) = self.next_component_value()
                else {
                    return None;
                };

                let pseudo_class = page_pseudo_class_from_string(value)?;
                self.index += 1;
                pseudo_classes.push(pseudo_class);
            }

            if name.is_none() && pseudo_classes.is_empty() {
                return None;
            }
            selector_list.push(PageSelector { name, pseudo_classes });

            self.discard_whitespace();
            if matches!(
                self.next_component_value(),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Comma,
                    ..
                }))
            ) {
                self.index += 1;
                self.discard_whitespace();
                if !self.has_next_component_value() {
                    return None;
                }
            } else if self.has_next_component_value() {
                return None;
            }
        }

        Some(selector_list)
    }

    // https://drafts.csswg.org/css-animations-1/#typedef-keyframe-selector
    fn parse_a_keyframe_selector_list(&mut self) -> Option<Vec<KeyframeSelector>> {
        // <keyframe-selector> = from | to | <percentage [0,100]>
        //
        // The <<keyframe-selector>> for a <<keyframe-block>> consists of a comma-separated list of percentage values or
        // the keywords ''from'' or ''to''.
        let mut selector_list = Vec::new();

        self.discard_whitespace();
        loop {
            let selector = match self.next_component_value() {
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                })) if value.eq_ignore_ascii_case("from") => Some(0.0),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                })) if value.eq_ignore_ascii_case("to") => Some(100.0),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Percentage { number },
                    ..
                })) if (0.0..=100.0).contains(&number.value()) => Some(number.value()),
                _ => None,
            }?;
            self.index += 1;
            selector_list.push(selector);

            self.discard_whitespace();
            if matches!(
                self.next_component_value(),
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Comma,
                    ..
                }))
            ) {
                self.index += 1;
                self.discard_whitespace();
                if !self.has_next_component_value() {
                    return None;
                }
            } else {
                break;
            }
        }

        if self.has_next_component_value() {
            return None;
        }

        Some(selector_list)
    }

    // https://drafts.csswg.org/css-animations-1/#typedef-keyframes-name
    fn parse_a_keyframes_name(&mut self) -> Option<String> {
        // <keyframes-name> = <custom-ident> | <string>
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            }) => value.clone(),
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_valid_custom_ident(value, &["none"]) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-values-4/#custom-idents
    fn parse_a_custom_ident(&mut self, blacklist: &[&str]) -> Option<String> {
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_valid_custom_ident(value, blacklist) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-values-4/#typedef-dashed-ident
    fn parse_a_dashed_ident(&mut self) -> Option<String> {
        // The <dashed-ident> production is a <custom-ident>, with all the case-sensitivity that implies, with the
        // additional restriction that it must start with two dashes (U+002D HYPHEN-MINUS).
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if value.starts_with("--") && is_valid_custom_ident(value, &[]) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-values-4/#url-value
    fn parse_a_url_function(&mut self) -> Option<UrlFunction> {
        let url_function = self.parse_a_url_function_component()?;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(url_function)
    }

    fn parse_a_url_function_component(&mut self) -> Option<UrlFunction> {
        // <url> = <url()> | <src()>
        // <url()> = url( <string> <url-modifier>* ) | <url-token>
        // <src()> = src( <string> <url-modifier>* )
        let url_function = match self.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Url { value },
                ..
            }) => UrlFunction {
                function_type: CssUrlFunctionType::Url,
                url: value,
                request_url_modifiers: Vec::new(),
            },
            ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("url") => {
                parse_url_or_src_function_contents(CssUrlFunctionType::Url, &function.value)?
            }
            ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("src") => {
                parse_url_or_src_function_contents(CssUrlFunctionType::Src, &function.value)?
            }
            _ => return None,
        };

        Some(url_function)
    }

    // https://drafts.csswg.org/css-cascade-5/#at-import
    fn parse_an_import_url(&mut self) -> Option<UrlFunction> {
        // @import [ <url> | <string> ]
        let url_function = self.parse_an_import_url_prefix()?;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(url_function)
    }

    fn parse_an_import_url_prefix(&mut self) -> Option<UrlFunction> {
        // @import [ <url> | <string> ]
        self.discard_whitespace();

        let url_function = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            }) => {
                let value = value.clone();
                self.index += 1;
                UrlFunction {
                    function_type: CssUrlFunctionType::Url,
                    url: value,
                    request_url_modifiers: Vec::new(),
                }
            }
            _ => self.parse_a_url_function_component()?,
        };

        Some(url_function)
    }

    // https://drafts.csswg.org/css-fonts/#font-face-src-parsing
    fn parse_a_font_source(&mut self) -> Option<FontSource> {
        // <font-src> = <url> [ format(<font-format>)]? [ tech( <font-tech>#)]? | local(<family-name>)
        self.discard_whitespace();

        // local(<family-name>)
        if let Some(ComponentValue::Function(function)) = self.next_component_value()
            && function.name.eq_ignore_ascii_case("local")
        {
            let mut function_parser = ComponentValueParser::new(function.value.clone());
            let family_name = function_parser.parse_a_family_name()?;
            function_parser.discard_whitespace();
            if function_parser.has_next_component_value() {
                return None;
            }

            self.index += 1;
            self.discard_whitespace();
            if self.has_next_component_value() {
                return None;
            }
            return Some(FontSource::Local(family_name));
        }

        // <url> [ format(<font-format>)]? [ tech( <font-tech>#)]?
        let url_function = self.parse_a_url_function_component()?;
        let mut format = None;
        let mut tech = Vec::new();

        self.discard_whitespace();

        // [ format(<font-format>)]?
        if let Some(ComponentValue::Function(function)) = self.next_component_value()
            && function.name.eq_ignore_ascii_case("format")
        {
            let (parsed_format, parsed_tech) = parse_font_format_function(function)?;
            format = Some(parsed_format);
            tech.extend(parsed_tech);
            self.index += 1;
        }

        self.discard_whitespace();

        // [ tech( <font-tech>#)]?
        if let Some(ComponentValue::Function(function)) = self.next_component_value()
            && function.name.eq_ignore_ascii_case("tech")
        {
            tech.extend(parse_font_tech_function(function)?);
            self.index += 1;
        }

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(FontSource::Url {
            url_function,
            format,
            tech,
        })
    }

    // https://drafts.csswg.org/css-fonts/#propdef-font-language-override
    fn parse_a_font_language_override(&mut self) -> Option<FontLanguageOverride> {
        // normal | <string>
        self.discard_whitespace();

        let font_language_override = match self.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if value.eq_ignore_ascii_case("normal") => FontLanguageOverride::Normal,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            }) => FontLanguageOverride::String(parse_font_language_override_string_value(&value)?),
            _ => return None,
        };

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(font_language_override)
    }

    // https://drafts.csswg.org/css-fonts/#propdef-font-feature-settings
    fn parse_a_font_feature_settings(&mut self, filtered_input: &str) -> Option<OpenTypeSettings> {
        // normal | <feature-tag-value>#
        self.discard_whitespace();

        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
            && value.eq_ignore_ascii_case("normal")
        {
            self.index += 1;
            self.discard_whitespace();
            return (!self.has_next_component_value()).then_some(OpenTypeSettings::Normal);
        }

        // <feature-tag-value>#
        let tag_values =
            parse_comma_separated_component_values(self.remaining_component_values().to_vec(), |component_values| {
                parse_feature_tag_value(component_values, filtered_input)
            })?;
        self.index = self.component_values.len();

        Some(OpenTypeSettings::TagValues(tag_values))
    }

    // https://drafts.csswg.org/css-fonts/#propdef-font-variation-settings
    fn parse_a_font_variation_settings(&mut self, filtered_input: &str) -> Option<OpenTypeSettings> {
        // normal | [ <opentype-tag> <number> ]#
        self.discard_whitespace();

        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
            && value.eq_ignore_ascii_case("normal")
        {
            self.index += 1;
            self.discard_whitespace();
            return (!self.has_next_component_value()).then_some(OpenTypeSettings::Normal);
        }

        // [ <opentype-tag> <number>]#
        let tag_values =
            parse_comma_separated_component_values(self.remaining_component_values().to_vec(), |component_values| {
                parse_variation_tag_value(component_values, filtered_input)
            })?;
        self.index = self.component_values.len();

        Some(OpenTypeSettings::TagValues(tag_values))
    }

    // https://drafts.csswg.org/css-fonts-4/#font-style-prop
    fn parse_a_font_style(&mut self) -> Option<FontStyle> {
        // normal | italic | left | right | oblique <angle [-90deg,90deg]>?
        self.discard_whitespace();

        if self.consume_ident_matching("normal") {
            return Some(FontStyle::Normal);
        }
        if self.consume_ident_matching("italic") {
            return Some(FontStyle::Italic);
        }
        if self.consume_ident_matching("left") {
            return Some(FontStyle::Left);
        }
        if self.consume_ident_matching("right") {
            return Some(FontStyle::Right);
        }
        if self.consume_ident_matching("oblique") {
            self.discard_whitespace();
            if self.next_component_value().is_some_and(component_value_parse_as_angle) {
                self.index += 1;
                return Some(FontStyle::Oblique { has_angle: true });
            }
            return Some(FontStyle::Oblique { has_angle: false });
        }

        None
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant-alternates
    fn parse_a_font_variant_alternates(&mut self) -> Option<Vec<FontVariantAlternatesValue>> {
        // [ stylistic(<feature-value-name>) || historical-forms || styleset(<feature-value-name>#) || character-variant(<feature-value-name>#) || swash(<feature-value-name>) || ornaments(<feature-value-name>) || annotation(<feature-value-name>) ]
        // <feature-value-name> = <ident>
        let mut stylistic = None;
        let mut historical_forms = None;
        let mut styleset = None;
        let mut character_variant = None;
        let mut swash = None;
        let mut ornaments = None;
        let mut annotation = None;

        loop {
            self.discard_whitespace();

            if self.consume_ident_matching("historical-forms") {
                if historical_forms.is_some() {
                    return None;
                }
                historical_forms = Some(FontVariantAlternatesValue {
                    kind: CssFontVariantAlternatesValueKind::HistoricalForms,
                    feature_value_names: Vec::new(),
                });
                continue;
            }

            let Some(ComponentValue::Function(function)) = self.next_component_value() else {
                break;
            };

            let kind = if function.name.eq_ignore_ascii_case("stylistic") {
                if stylistic.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Stylistic
            } else if function.name.eq_ignore_ascii_case("styleset") {
                if styleset.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Styleset
            } else if function.name.eq_ignore_ascii_case("character-variant") {
                if character_variant.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::CharacterVariant
            } else if function.name.eq_ignore_ascii_case("swash") {
                if swash.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Swash
            } else if function.name.eq_ignore_ascii_case("ornaments") {
                if ornaments.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Ornaments
            } else if function.name.eq_ignore_ascii_case("annotation") {
                if annotation.is_some() {
                    return None;
                }
                CssFontVariantAlternatesValueKind::Annotation
            } else {
                break;
            };

            let feature_value_names = parse_font_variant_alternates_feature_value_names(function.value.clone())?;
            if !matches!(
                kind,
                CssFontVariantAlternatesValueKind::Styleset | CssFontVariantAlternatesValueKind::CharacterVariant
            ) && feature_value_names.len() != 1
            {
                return None;
            }

            self.index += 1;
            let value = FontVariantAlternatesValue {
                kind,
                feature_value_names,
            };
            match kind {
                CssFontVariantAlternatesValueKind::Stylistic => stylistic = Some(value),
                CssFontVariantAlternatesValueKind::Styleset => styleset = Some(value),
                CssFontVariantAlternatesValueKind::CharacterVariant => character_variant = Some(value),
                CssFontVariantAlternatesValueKind::Swash => swash = Some(value),
                CssFontVariantAlternatesValueKind::Ornaments => ornaments = Some(value),
                CssFontVariantAlternatesValueKind::Annotation => annotation = Some(value),
                CssFontVariantAlternatesValueKind::HistoricalForms => unreachable!(),
            }
        }

        let values = [
            stylistic,
            historical_forms,
            styleset,
            character_variant,
            swash,
            ornaments,
            annotation,
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        (!values.is_empty()).then_some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant
    fn parse_a_font_variant(&mut self) -> Option<FontVariant> {
        // normal |
        // none |
        // [
        //   [ <common-lig-values> || <discretionary-lig-values> || <historical-lig-values> || <contextual-alt-values> ] ||
        //   [ small-caps | all-small-caps | petite-caps | all-petite-caps | unicase | titling-caps ] ||
        //   [ stylistic(<feature-value-name>) || historical-forms || styleset(<feature-value-name>#) || character-variant(<feature-value-name>#) || swash(<feature-value-name>) || ornaments(<feature-value-name>) || annotation(<feature-value-name>) ] ||
        //   [ <numeric-figure-values> || <numeric-spacing-values> || <numeric-fraction-values> || ordinal || slashed-zero ] ||
        //   [ <east-asian-variant-values> || <east-asian-width-values> || ruby ] ||
        //   [ sub | super ] ||
        //   [ text | emoji | unicode ]
        // ]
        self.discard_whitespace();
        if self.consume_ident_matching("normal") {
            self.discard_whitespace();
            return (!self.has_next_component_value()).then_some(FontVariant::default());
        }

        if self.consume_ident_matching("none") {
            self.discard_whitespace();
            return (!self.has_next_component_value()).then_some(FontVariant {
                ligatures_none: true,
                ..FontVariant::default()
            });
        }

        let mut font_variant = FontVariant::default();
        while self.has_next_component_value() {
            let start = self.index;
            if let Some(ligatures) = self.parse_a_font_variant_ligatures() {
                if font_variant.ligatures.is_some() {
                    return None;
                }
                font_variant.ligatures = Some(ligatures);
                continue;
            }
            self.index = start;

            if let Some(alternates) = self.parse_a_font_variant_alternates() {
                if font_variant.alternates.is_some() {
                    return None;
                }
                font_variant.alternates = Some(alternates);
                continue;
            }
            self.index = start;

            if let Some(numeric) = self.parse_a_font_variant_numeric() {
                if font_variant.numeric.is_some() {
                    return None;
                }
                font_variant.numeric = Some(numeric);
                continue;
            }
            self.index = start;

            if let Some(east_asian) = self.parse_a_font_variant_east_asian() {
                if font_variant.east_asian.is_some() {
                    return None;
                }
                font_variant.east_asian = Some(east_asian);
                continue;
            }
            self.index = start;

            self.discard_whitespace();
            let Some(value) = self.consume_an_ident() else {
                break;
            };
            let value = value.to_ascii_lowercase();

            if matches_font_variant_caps_value(&value) {
                if font_variant.caps.is_some() {
                    return None;
                }
                font_variant.caps = Some(value);
                continue;
            }

            if matches_font_variant_emoji_value(&value) {
                if font_variant.emoji.is_some() {
                    return None;
                }
                font_variant.emoji = Some(value);
                continue;
            }

            if matches_font_variant_position_value(&value) {
                if font_variant.position.is_some() {
                    return None;
                }
                font_variant.position = Some(value);
                continue;
            }

            self.index = start;
            break;
        }

        font_variant.has_any_value().then_some(font_variant)
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant-east-asian
    fn parse_a_font_variant_east_asian(&mut self) -> Option<Vec<FontVariantEastAsianValue>> {
        // [ <east-asian-variant-values> || <east-asian-width-values> || ruby ]
        // <east-asian-variant-values> = [ jis78 | jis83 | jis90 | jis04 | simplified | traditional ]
        // <east-asian-width-values>   = [ full-width | proportional-width ]
        let mut variant = false;
        let mut width = false;
        let mut ruby = false;
        let mut values = Vec::new();

        loop {
            self.discard_whitespace();
            let start = self.index;
            let Some(value) = self.consume_an_ident() else {
                break;
            };
            let value = value.to_ascii_lowercase();

            if value == "ruby" {
                if ruby {
                    return None;
                }
                ruby = true;
                values.push(FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Ruby,
                    value,
                });
                continue;
            }

            if matches_east_asian_width_value(&value) {
                if width {
                    return None;
                }
                width = true;
                values.push(FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Width,
                    value,
                });
                continue;
            }

            if matches_east_asian_variant_value(&value) {
                if variant {
                    return None;
                }
                variant = true;
                values.push(FontVariantEastAsianValue {
                    kind: CssFontVariantEastAsianValueKind::Variant,
                    value,
                });
                continue;
            }

            self.index = start;
            break;
        }

        (!values.is_empty()).then_some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant-numeric
    fn parse_a_font_variant_numeric(&mut self) -> Option<Vec<FontVariantNumericValue>> {
        // [ <numeric-figure-values> || <numeric-spacing-values> || <numeric-fraction-values> || ordinal || slashed-zero]
        // <numeric-figure-values>       = [ lining-nums | oldstyle-nums ]
        // <numeric-spacing-values>      = [ proportional-nums | tabular-nums ]
        // <numeric-fraction-values>     = [ diagonal-fractions | stacked-fractions ]
        let mut figure = false;
        let mut spacing = false;
        let mut fraction = false;
        let mut ordinal = false;
        let mut slashed_zero = false;
        let mut values = Vec::new();

        loop {
            self.discard_whitespace();
            let start = self.index;
            let Some(value) = self.consume_an_ident() else {
                break;
            };
            let value = value.to_ascii_lowercase();

            if matches_numeric_figure_value(&value) {
                if figure {
                    return None;
                }
                figure = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Figure,
                    value,
                });
                continue;
            }

            if matches_numeric_spacing_value(&value) {
                if spacing {
                    return None;
                }
                spacing = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Spacing,
                    value,
                });
                continue;
            }

            if matches_numeric_fraction_value(&value) {
                if fraction {
                    return None;
                }
                fraction = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Fraction,
                    value,
                });
                continue;
            }

            if value == "ordinal" {
                if ordinal {
                    return None;
                }
                ordinal = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::Ordinal,
                    value,
                });
                continue;
            }

            if value == "slashed-zero" {
                if slashed_zero {
                    return None;
                }
                slashed_zero = true;
                values.push(FontVariantNumericValue {
                    kind: CssFontVariantNumericValueKind::SlashedZero,
                    value,
                });
                continue;
            }

            self.index = start;
            break;
        }

        (!values.is_empty()).then_some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#propdef-font-variant-ligatures
    fn parse_a_font_variant_ligatures(&mut self) -> Option<Vec<FontVariantLigaturesValue>> {
        // [ <common-lig-values> || <discretionary-lig-values> || <historical-lig-values> || <contextual-alt-values> ]
        // <common-lig-values>       = [ common-ligatures | no-common-ligatures ]
        // <discretionary-lig-values> = [ discretionary-ligatures | no-discretionary-ligatures ]
        // <historical-lig-values>   = [ historical-ligatures | no-historical-ligatures ]
        // <contextual-alt-values>   = [ contextual | no-contextual ]
        let mut common = false;
        let mut discretionary = false;
        let mut historical = false;
        let mut contextual = false;
        let mut values = Vec::new();

        loop {
            self.discard_whitespace();
            let start = self.index;
            let Some(value) = self.consume_an_ident() else {
                break;
            };
            let value = value.to_ascii_lowercase();

            if matches_common_lig_value(&value) {
                if common {
                    return None;
                }
                common = true;
                values.push(FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Common,
                    value,
                });
                continue;
            }

            if matches_discretionary_lig_value(&value) {
                if discretionary {
                    return None;
                }
                discretionary = true;
                values.push(FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Discretionary,
                    value,
                });
                continue;
            }

            if matches_historical_lig_value(&value) {
                if historical {
                    return None;
                }
                historical = true;
                values.push(FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Historical,
                    value,
                });
                continue;
            }

            if matches_contextual_alt_value(&value) {
                if contextual {
                    return None;
                }
                contextual = true;
                values.push(FontVariantLigaturesValue {
                    kind: CssFontVariantLigaturesValueKind::Contextual,
                    value,
                });
                continue;
            }

            self.index = start;
            break;
        }

        (!values.is_empty()).then_some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#font-family-prop
    fn parse_a_font_family_item(&mut self) -> Option<FontFamilyValue> {
        // [ <family-name> | <generic-family> ]#
        self.discard_whitespace();

        // <generic-family>
        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
            && matches_generic_font_family_keyword(value)
        {
            let generic_family = value.to_ascii_lowercase();
            self.index += 1;
            return Some(FontFamilyValue::Generic(generic_family));
        }

        // <family-name>
        self.parse_a_family_name().map(FontFamilyValue::FamilyName)
    }

    // https://drafts.csswg.org/css-variables-2/#typedef-custom-property-name
    fn parse_a_custom_property_name(&mut self) -> Option<String> {
        // The <custom-property-name> production corresponds to this: it’s defined as any <dashed-ident>
        // (a valid identifier that starts with two dashes), except -- itself, which is reserved for future use by CSS.
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_a_custom_property_name_string(value) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-cascade-5/#typedef-layer-name
    fn parse_a_layer_name(&mut self, allow_blank_layer_name: bool) -> Option<String> {
        // <layer-name> = <ident> [ '.' <ident> ]*
        self.discard_whitespace();
        if allow_blank_layer_name && !self.has_next_component_value() {
            return Some(String::new());
        }

        let mut name = self.consume_a_layer_name_part()?;
        while component_value_is_delim(self.next_component_value(), '.') {
            self.index += 1;
            let name_part = self.consume_a_layer_name_part()?;
            name.push('.');
            name.push_str(&name_part);
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-cascade-5/#at-import
    fn parse_an_import_layer(&mut self) -> Option<String> {
        // [ layer | layer(<layer-name>) ]?
        let layer = self.parse_an_import_layer_prefix()?;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(layer)
    }

    fn parse_an_import_layer_prefix(&mut self) -> Option<String> {
        // [ layer | layer(<layer-name>) ]?
        self.discard_whitespace();

        let layer = match self.consume_the_next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if value.eq_ignore_ascii_case("layer") => String::new(),
            ComponentValue::Function(function) if function.name.eq_ignore_ascii_case("layer") => {
                let mut function_parser = ComponentValueParser::new(function.value);
                let name = function_parser.parse_a_layer_name(false)?;
                function_parser.discard_whitespace();
                if function_parser.has_next_component_value() {
                    return None;
                }
                name
            }
            _ => return None,
        };

        Some(layer)
    }

    // https://drafts.csswg.org/css-cascade-5/#layering
    fn parse_a_layer_name_list(&mut self) -> Option<Vec<String>> {
        // @layer <layer-name>#;
        let mut names = Vec::new();
        self.discard_whitespace();
        if !self.has_next_component_value() {
            return None;
        }

        loop {
            let name = self.parse_a_layer_name(false)?;
            names.push(name);
            self.discard_whitespace();

            if !self.has_next_component_value() {
                break;
            }

            if !component_value_is_comma(self.next_component_value()) {
                return None;
            }
            self.index += 1;
            self.discard_whitespace();
            if !self.has_next_component_value() {
                return None;
            }
        }

        Some(names)
    }

    fn consume_a_layer_name_part(&mut self) -> Option<String> {
        // "The CSS-wide keywords are reserved for future use, and cause the rule to be invalid at parse time
        // if used as an <ident> in the <layer-name>."
        let name_part = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if !matches_css_wide_keyword(value) => value.clone(),
            _ => return None,
        };
        self.index += 1;
        Some(name_part)
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style-name
    fn parse_a_counter_style_name(&mut self) -> Option<String> {
        // <counter-style-name> is a <custom-ident> that is not an ASCII case-insensitive match for none.
        self.discard_whitespace();
        let name = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) if is_valid_custom_ident(value, &["none"]) => value.clone(),
            _ => return None,
        };
        self.index += 1;

        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some(name)
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style
    fn parse_a_counter_style(&mut self) -> Option<CounterStyle> {
        // <counter-style> = <counter-style-name> | <symbols()>
        let saved_index = self.index;
        if let Some(name) = self.parse_a_counter_style_name() {
            return Some(CounterStyle::Name(name));
        }
        self.index = saved_index;

        // <symbols()> = symbols( <symbols-type>? [ <string> | <image> ]+ )
        let ComponentValue::Function(Function { name, value, .. }) = self.consume_the_next_component_value()? else {
            return None;
        };
        if !name.eq_ignore_ascii_case("symbols") {
            return None;
        }

        let mut parser = ComponentValueParser::new(value);
        parser.discard_whitespace();

        // <symbols-type> = cyclic | numeric | alphabetic | symbolic | fixed
        // NB: <symbols-type> defaults to symbolic if not provided.
        let symbols_type = if parser.consume_ident_matching("cyclic") {
            CssCounterStyleSymbolsType::Cyclic
        } else if parser.consume_ident_matching("numeric") {
            CssCounterStyleSymbolsType::Numeric
        } else if parser.consume_ident_matching("alphabetic") {
            CssCounterStyleSymbolsType::Alphabetic
        } else if parser.consume_ident_matching("symbolic") {
            CssCounterStyleSymbolsType::Symbolic
        } else if parser.consume_ident_matching("fixed") {
            CssCounterStyleSymbolsType::Fixed
        } else {
            CssCounterStyleSymbolsType::Symbolic
        };

        // AD-HOC: In line with <symbol>, we don't support <image> here since
        // that part of the grammar is at-risk and unsupported by other engines.
        let mut symbols = Vec::new();
        loop {
            parser.discard_whitespace();
            let Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value },
                ..
            })) = parser.next_component_value()
            else {
                break;
            };
            symbols.push(value.clone());
            parser.index += 1;
        }

        parser.discard_whitespace();
        if parser.has_next_component_value() {
            return None;
        }

        // https://drafts.csswg.org/css-counter-styles-3/#symbols-function
        // If the system is alphabetic or numeric, there must be at least two
        // <string>s or <image>s, or else the function is invalid.
        if symbols.is_empty()
            || (matches!(
                symbols_type,
                CssCounterStyleSymbolsType::Alphabetic | CssCounterStyleSymbolsType::Numeric
            ) && symbols.len() < 2)
        {
            return None;
        }

        Some(CounterStyle::SymbolsFunction { symbols_type, symbols })
    }

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-system
    fn parse_counter_style_system(&mut self) -> Option<CssCounterStyleSystemKind> {
        // cyclic | numeric | alphabetic | symbolic | additive | [fixed <integer>?] | [ extends <counter-style-name> ]
        self.discard_whitespace();

        if self.consume_ident_matching("cyclic") {
            return Some(CssCounterStyleSystemKind::Cyclic);
        }
        if self.consume_ident_matching("numeric") {
            return Some(CssCounterStyleSystemKind::Numeric);
        }
        if self.consume_ident_matching("alphabetic") {
            return Some(CssCounterStyleSystemKind::Alphabetic);
        }
        if self.consume_ident_matching("symbolic") {
            return Some(CssCounterStyleSystemKind::Symbolic);
        }
        if self.consume_ident_matching("additive") {
            return Some(CssCounterStyleSystemKind::Additive);
        }

        if self.consume_ident_matching("fixed") {
            self.discard_whitespace();
            if self.consume_integer_syntax() {
                return Some(CssCounterStyleSystemKind::FixedWithInteger);
            }
            return Some(CssCounterStyleSystemKind::Fixed);
        }

        if self.consume_ident_matching("extends") {
            self.discard_whitespace();
            if self.consume_counter_style_name_syntax() {
                return Some(CssCounterStyleSystemKind::Extends);
            }
        }

        None
    }

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-negative
    fn parse_counter_style_negative(&mut self) -> Option<CssCounterStyleNegativeSymbolCount> {
        // <symbol> <symbol>?
        self.discard_whitespace();
        if !self.consume_symbol_syntax() {
            return None;
        }

        self.discard_whitespace();
        if !self.consume_symbol_syntax() {
            return Some(CssCounterStyleNegativeSymbolCount::One);
        }

        Some(CssCounterStyleNegativeSymbolCount::Two)
    }

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-symbols
    fn parse_counter_style_symbols(&mut self) -> Option<usize> {
        // <symbol>+
        let mut count = 0;
        loop {
            self.discard_whitespace();
            if !self.consume_symbol_syntax() {
                break;
            }
            count += 1;
        }

        if count == 0 {
            return None;
        }

        Some(count)
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-symbol
    fn parse_counter_style_symbol(&mut self) -> Option<()> {
        // <symbol> = <string> | <image> | <custom-ident>
        self.discard_whitespace();
        self.consume_symbol_syntax().then_some(())
    }

    // https://drafts.csswg.org/css-counter-styles-3/#counter-style-range
    fn parse_counter_style_range(&mut self) -> Option<(CssCounterStyleRangeKind, usize)> {
        // [ [ <integer> | infinite ]{2} ]# | auto
        self.discard_whitespace();
        if self.consume_ident_matching("auto") {
            return Some((CssCounterStyleRangeKind::Auto, 0));
        }

        let mut count = 0;
        loop {
            self.discard_whitespace();
            if !self.consume_counter_style_range_bound_syntax() {
                break;
            }

            self.discard_whitespace();
            if !self.consume_counter_style_range_bound_syntax() {
                return None;
            }

            count += 1;
            self.discard_whitespace();
            if !self.consume_comma() {
                break;
            }
            self.discard_whitespace();
            if !self.has_next_component_value() {
                return None;
            }
        }

        if count == 0 {
            return None;
        }

        Some((CssCounterStyleRangeKind::List, count))
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-additive-symbols
    fn parse_counter_style_additive_symbols(&mut self) -> Option<usize> {
        // <additive-symbols> = <additive-tuple>#
        let mut count = 0;
        loop {
            self.discard_whitespace();
            self.parse_a_nonnegative_integer_symbol_pair()?;

            count += 1;
            self.discard_whitespace();
            if !self.consume_comma() {
                break;
            }
            self.discard_whitespace();
            if !self.has_next_component_value() {
                return None;
            }
        }

        if count == 0 {
            return None;
        }

        Some(count)
    }

    // https://drafts.csswg.org/css-page-3/#marks
    fn parse_crop_or_cross(&mut self) -> Option<CssCropOrCrossKind> {
        // crop || cross
        self.discard_whitespace();

        let first_is_crop = if self.consume_ident_matching("crop") {
            true
        } else if self.consume_ident_matching("cross") {
            false
        } else {
            return None;
        };

        self.discard_whitespace();
        let has_both = if first_is_crop {
            self.consume_ident_matching("cross")
        } else {
            self.consume_ident_matching("crop")
        };

        if has_both {
            return Some(CssCropOrCrossKind::CropAndCross);
        }

        Some(if first_is_crop {
            CssCropOrCrossKind::Crop
        } else {
            CssCropOrCrossKind::Cross
        })
    }

    // https://drafts.csswg.org/css-fonts-4/#font-prop-desc
    fn parse_font_weight_absolute_pair(&mut self) -> Option<usize> {
        // <font-weight-absolute>{1,2}
        let mut count = 0;
        for _ in 0..2 {
            self.discard_whitespace();
            if !self.consume_font_weight_absolute_syntax() {
                break;
            }
            count += 1;
        }

        if count == 0 {
            return None;
        }

        Some(count)
    }

    // https://drafts.csswg.org/css-page-3/#page-size-prop
    fn parse_page_size_descriptor(&mut self) -> Option<()> {
        // <length [0,∞]>{1,2} | auto | [ <page-size> || [ portrait | landscape ] ]
        self.discard_whitespace();
        if self.consume_ident_matching("auto") {
            return Some(());
        }

        let saved_index = self.index;
        let mut length_count = 0;
        for _ in 0..2 {
            self.discard_whitespace();
            if !self.consume_nonnegative_length_descriptor_syntax() {
                break;
            }
            length_count += 1;
        }
        if length_count > 0 {
            return Some(());
        }
        self.index = saved_index;

        let mut page_size = false;
        let mut orientation = false;

        for _ in 0..2 {
            self.discard_whitespace();
            let Some(ident) = self.consume_an_ident() else {
                break;
            };

            if is_page_size_keyword(&ident) {
                if page_size {
                    return None;
                }
                page_size = true;
            } else if ident.eq_ignore_ascii_case("portrait") || ident.eq_ignore_ascii_case("landscape") {
                if orientation {
                    return None;
                }
                orientation = true;
            } else {
                return None;
            }
        }

        (page_size || orientation).then_some(())
    }

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-additive-tuple
    fn parse_a_nonnegative_integer_symbol_pair(&mut self) -> Option<CssNonnegativeIntegerSymbolPairOrder> {
        // <additive-tuple> = [ <integer [0,∞]> && <symbol> ]
        let saved_index = self.index;
        if self.consume_nonnegative_integer_syntax() {
            self.discard_whitespace();
            if self.consume_symbol_syntax() {
                return Some(CssNonnegativeIntegerSymbolPairOrder::IntegerFirst);
            }
        }
        self.index = saved_index;

        if self.consume_symbol_syntax() {
            self.discard_whitespace();
            if self.consume_nonnegative_integer_syntax() {
                return Some(CssNonnegativeIntegerSymbolPairOrder::SymbolFirst);
            }
        }
        self.index = saved_index;
        None
    }

    fn consume_nonnegative_integer_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        let is_nonnegative_integer = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Number { number },
                ..
            }) => number_is_integer(*number) && number.value() >= 0.0,
            // AD-HOC: The Rust side only recognizes the syntactic branch here.
            // Materializing and range-checking math functions still happens in C++.
            ComponentValue::Function(_) => true,
            _ => false,
        };

        if is_nonnegative_integer {
            self.index += 1;
        }
        is_nonnegative_integer
    }

    fn consume_integer_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        let is_integer = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Number { number },
                ..
            }) => number_is_integer(*number),
            // AD-HOC: The Rust side only recognizes the syntactic branch here.
            // Materializing math functions still happens in C++.
            ComponentValue::Function(_) => true,
            _ => false,
        };

        if is_integer {
            self.index += 1;
        }
        is_integer
    }

    fn consume_counter_style_name_syntax(&mut self) -> bool {
        let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
        else {
            return false;
        };

        // <counter-style-name> is a <custom-ident> that is not an ASCII
        // case-insensitive match for none.
        if !is_valid_custom_ident(value, &["none"]) {
            return false;
        }

        self.index += 1;
        true
    }

    fn consume_counter_style_range_bound_syntax(&mut self) -> bool {
        if self.consume_ident_matching("infinite") {
            return true;
        }

        self.consume_integer_syntax()
    }

    fn consume_font_weight_absolute_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        let is_font_weight_absolute = component_values_parse_as_value_type(
            ValueTypeId::FontWeightAbsolute,
            std::slice::from_ref(component_value),
        ) != CssValueTypeSyntaxKind::Invalid;

        if is_font_weight_absolute {
            self.index += 1;
        }
        is_font_weight_absolute
    }

    fn consume_nonnegative_length_descriptor_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        let is_nonnegative_length = component_value_parse_as_nonnegative_length_descriptor(component_value);
        if is_nonnegative_length {
            self.index += 1;
        }
        is_nonnegative_length
    }

    fn consume_symbol_syntax(&mut self) -> bool {
        let Some(component_value) = self.next_component_value() else {
            return false;
        };

        // <symbol> = <string> | <image> | <custom-ident>
        let is_symbol = match component_value {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { .. },
                ..
            }) => true,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            }) => is_valid_custom_ident(value, &[]),
            // AD-HOC: In line with the generated <symbol> parser, we don't
            // support <image> here since that part of the grammar is at-risk
            // and unsupported by other engines.
            _ => false,
        };

        if is_symbol {
            self.index += 1;
        }
        is_symbol
    }

    fn consume_comma(&mut self) -> bool {
        if !matches!(
            self.next_component_value(),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            }))
        ) {
            return false;
        }

        self.index += 1;
        true
    }

    // https://drafts.csswg.org/css-namespaces/#syntax
    fn parse_a_namespace_rule_prelude(&mut self) -> Option<(Option<String>, String)> {
        // @namespace <namespace-prefix>? [ <string> | <url> ] ;
        // <namespace-prefix> = <ident>
        self.discard_whitespace();

        let prefix = match self.next_component_value() {
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) => {
                let prefix = value.clone();
                self.index += 1;
                self.discard_whitespace();
                Some(prefix)
            }
            _ => None,
        };

        let namespace_uri = self.consume_namespace_uri()?;
        self.discard_whitespace();
        if self.has_next_component_value() {
            return None;
        }

        Some((prefix, namespace_uri))
    }

    fn consume_namespace_uri(&mut self) -> Option<String> {
        // "A URI string parsed from the URI syntax must be treated as a literal string: as with the STRING syntax, no
        // URI-specific normalization is applied."
        // https://drafts.csswg.org/css-namespaces/#syntax
        let namespace_uri = match self.next_component_value()? {
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::String { value } | TokenType::Url { value },
                ..
            }) => value.clone(),
            ComponentValue::Function(function)
                if function.name.eq_ignore_ascii_case("url") || function.name.eq_ignore_ascii_case("src") =>
            {
                let mut function_parser = ComponentValueParser::new(function.value.clone());
                function_parser.discard_whitespace();
                let namespace_uri = match function_parser.next_component_value()? {
                    ComponentValue::PreservedToken(Token {
                        token_type: TokenType::String { value },
                        ..
                    }) => value.clone(),
                    _ => return None,
                };
                function_parser.index += 1;
                function_parser.discard_whitespace();
                if function_parser.has_next_component_value() {
                    return None;
                }
                namespace_uri
            }
            _ => return None,
        };
        self.index += 1;
        Some(namespace_uri)
    }

    // https://drafts.csswg.org/css-fonts-4/#font-feature-values-syntax
    fn parse_font_feature_values_feature_value(&mut self) -> Option<Vec<u32>> {
        // <feature-value-declaration> = <custom-ident> : <integer [0,∞]>+;
        self.discard_whitespace();

        let mut values = Vec::new();
        while let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        })) = self.next_component_value()
        {
            if !number_is_integer(*number) || number.value() < 0.0 || number.value() > u32::MAX as f64 {
                return None;
            }

            values.push(number.value() as u32);
            self.index += 1;
            self.discard_whitespace();
        }

        if values.is_empty() || self.has_next_component_value() {
            return None;
        }

        Some(values)
    }

    // https://drafts.csswg.org/css-fonts-4/#font-family-name-syntax
    fn parse_a_family_name(&mut self) -> Option<FamilyName> {
        // <font-family-name> = <string> | <custom-ident>+
        self.discard_whitespace();

        if let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::String { value },
            ..
        })) = self.next_component_value()
        {
            let family_name = value.clone();
            self.index += 1;
            return Some(FamilyName {
                name: family_name,
                is_string: true,
            });
        }

        let mut parts = Vec::new();
        while let Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) = self.next_component_value()
        {
            parts.push(value.clone());
            self.index += 1;
            self.discard_whitespace();
        }

        if parts.is_empty() {
            return None;
        }

        if parts.len() == 1 {
            // Any identifier which could be misinterpreted as a pre-defined keyword in the font-family value
            // definition, or the CSS-wide keywords, is not allowed.
            // AD-HOC: We allow all <ident>'s rather than just <custom-ident>, although we check that the whole value
            //         isn't a CSS-wide keyword, see https://github.com/w3c/csswg-drafts/issues/13692
            let part = &parts[0];
            if !is_valid_custom_ident(part, &[]) || matches_generic_font_family_keyword(part) {
                return None;
            }
        }

        Some(FamilyName {
            name: parts.join(" "),
            is_string: false,
        })
    }

    fn parse_container_rule_prelude_item(&mut self, filtered_input: &str) -> Option<(Option<String>, Option<String>)> {
        // https://drafts.csswg.org/css-conditional-5/#container-rule
        // <container-condition> = [ <container-name>? <container-query>? ]!
        // https://drafts.csswg.org/css-conditional-5/#container-name
        // <container-name> = <custom-ident>
        self.discard_whitespace();

        let container_name = match self.next_component_value() {
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { value },
                ..
            })) if is_valid_custom_ident(value, &["none", "and", "not", "or"]) => {
                let container_name = value.clone();
                self.index += 1;
                self.discard_whitespace();
                Some(container_name)
            }
            _ => None,
        };

        let container_query = if self.has_next_component_value() {
            Some(serialize_component_values_for_reparsing(
                &self.component_values[self.index..],
                filtered_input,
            )?)
        } else {
            None
        };

        if container_name.is_none() && container_query.is_none() {
            return None;
        }

        Some((container_name, container_query))
    }

    // https://drafts.csswg.org/mediaqueries-5/#typedef-general-enclosed
    fn parse_general_enclosed(&mut self) -> Option<ComponentValue> {
        // <general-enclosed> = [ <function-token> <any-value>? ) ] | [ ( <any-value>? ) ]
        //
        // https://drafts.csswg.org/css-syntax-3/#typedef-any-value
        // "The <any-value> production is identical to <declaration-value>",
        // and <declaration-value> does not contain "<<bad-string-token>>,
        // <<bad-url-token>>, unmatched <<)-token>>, <<]-token>>, or
        // <<}-token>>".
        let component_value = self.next_component_value()?.clone();
        let contains_only_any_value = match &component_value {
            ComponentValue::Function(function) => contains_only_any_value(&function.value),
            ComponentValue::SimpleBlock(block) if is_paren_block(block) => contains_only_any_value(&block.value),
            _ => false,
        };

        if contains_only_any_value {
            self.index += 1;
            return Some(component_value);
        }

        None
    }
}

fn component_value_is_ident(component_value: Option<&ComponentValue>, expected: &str) -> bool {
    matches!(
        component_value,
        Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        })) if value.eq_ignore_ascii_case(expected)
    )
}

fn component_value_is_delim(component_value: Option<&ComponentValue>, expected: char) -> bool {
    matches!(
        component_value,
        Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Delim { value },
            ..
        })) if *value == expected as u32
    )
}

fn component_value_is_comma(component_value: Option<&ComponentValue>) -> bool {
    matches!(
        component_value,
        Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Comma,
            ..
        }))
    )
}

fn is_paren_block(block: &SimpleBlock) -> bool {
    matches!(block.token.token_type, TokenType::OpenParen)
}

fn component_values_start_like_a_declaration(component_values: &[ComponentValue]) -> bool {
    let mut non_whitespace = component_values
        .iter()
        .filter(|component_value| !is_whitespace_component_value(component_value));

    matches!(
        (non_whitespace.next(), non_whitespace.next()),
        (
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { .. },
                ..
            })),
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Colon,
                ..
            }))
        )
    )
}

fn contains_only_any_value(component_values: &[ComponentValue]) -> bool {
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

fn contains_only_declaration_value(component_values: &[ComponentValue], nested: Nested) -> bool {
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

fn component_values_parse_as_media_feature(component_values: &[ComponentValue]) -> Option<MediaFeatureSyntax> {
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

fn component_values_parse_as_media_query(component_values: Vec<ComponentValue>) -> MediaQuerySyntax {
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

fn component_values_parse_as_value_type(
    value_type_id: ValueTypeId,
    component_values: &[ComponentValue],
) -> CssValueTypeSyntaxKind {
    component_values_parse_as_generated_value_type(value_type_id, component_values)
}

fn page_pseudo_class_from_string(input: &str) -> Option<CssPagePseudoClassKind> {
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
fn component_values_parse_as_syntax(
    component_values: &[ComponentValue],
    limit_single_component_ident_to_custom_ident: bool,
) -> Option<SyntaxNode> {
    component_values_parse_as_syntax_with_source(component_values, limit_single_component_ident_to_custom_ident, None)
}

// https://drafts.csswg.org/css-values-5/#typedef-syntax
fn component_values_parse_as_syntax_with_source(
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

fn parse_syntax_component(
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

fn parse_syntax_single_component(
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

fn parse_css_type(
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

fn is_syntax_type_name(value: &str) -> bool {
    SYNTAX_TYPE_NAMES.contains(&value)
}

fn syntax_type_name_source_matches_value(token: &Token, value: &str, filtered_input: Option<&str>) -> bool {
    let Some(filtered_input) = filtered_input else {
        return true;
    };
    token
        .original_source(filtered_input)
        .is_some_and(|source| source == value)
}

fn parse_syntax_multiplier(parser: &mut ComponentValueParser) -> Option<char> {
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

fn parse_syntax_combinator(parser: &mut ComponentValueParser) -> Option<char> {
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

fn component_values_parse_as_mf_boolean(component_values: &[ComponentValue]) -> Option<MediaFeatureSyntax> {
    // <mf-boolean> = <mf-name>
    component_values_parse_as_mf_name(component_values, AllowMinMaxPrefix::No).map(MediaFeatureSyntax::Boolean)
}

fn component_values_parse_as_mf_plain(component_values: &[ComponentValue]) -> Option<MediaFeatureSyntax> {
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

fn component_values_parse_as_mf_range(component_values: &[ComponentValue]) -> Option<MediaFeatureSyntax> {
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

fn component_value_parse_as_angle(component_value: &ComponentValue) -> bool {
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

fn is_valid_custom_ident(value: &str, blacklist: &[&str]) -> bool {
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

fn matches_css_wide_keyword(value: &str) -> bool {
    value.eq_ignore_ascii_case("inherit")
        || value.eq_ignore_ascii_case("initial")
        || value.eq_ignore_ascii_case("unset")
        || value.eq_ignore_ascii_case("revert")
        || value.eq_ignore_ascii_case("revert-layer")
}

fn matches_generic_font_family_keyword(value: &str) -> bool {
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

fn matches_east_asian_variant_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("jis78")
        || value.eq_ignore_ascii_case("jis83")
        || value.eq_ignore_ascii_case("jis90")
        || value.eq_ignore_ascii_case("jis04")
        || value.eq_ignore_ascii_case("simplified")
        || value.eq_ignore_ascii_case("traditional")
}

fn matches_east_asian_width_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("full-width") || value.eq_ignore_ascii_case("proportional-width")
}

fn matches_numeric_figure_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("lining-nums") || value.eq_ignore_ascii_case("oldstyle-nums")
}

fn matches_numeric_spacing_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("proportional-nums") || value.eq_ignore_ascii_case("tabular-nums")
}

fn matches_numeric_fraction_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("diagonal-fractions") || value.eq_ignore_ascii_case("stacked-fractions")
}

fn matches_common_lig_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("common-ligatures") || value.eq_ignore_ascii_case("no-common-ligatures")
}

fn matches_discretionary_lig_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("discretionary-ligatures") || value.eq_ignore_ascii_case("no-discretionary-ligatures")
}

fn matches_historical_lig_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("historical-ligatures") || value.eq_ignore_ascii_case("no-historical-ligatures")
}

fn matches_contextual_alt_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("contextual") || value.eq_ignore_ascii_case("no-contextual")
}

fn matches_font_variant_caps_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("small-caps")
        || value.eq_ignore_ascii_case("all-small-caps")
        || value.eq_ignore_ascii_case("petite-caps")
        || value.eq_ignore_ascii_case("all-petite-caps")
        || value.eq_ignore_ascii_case("unicase")
        || value.eq_ignore_ascii_case("titling-caps")
}

fn matches_font_variant_emoji_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("text") || value.eq_ignore_ascii_case("emoji") || value.eq_ignore_ascii_case("unicode")
}

fn matches_font_variant_position_value(value: &str) -> bool {
    value.eq_ignore_ascii_case("sub") || value.eq_ignore_ascii_case("super")
}

fn is_a_custom_property_name_string(value: &str) -> bool {
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

fn component_values_parse_as_mf_range_name(component_values: &[ComponentValue]) -> Option<MediaFeatureName> {
    let name = component_values_parse_as_mf_name(component_values, AllowMinMaxPrefix::No)?;

    // The only significant difference between the two types is that “range” media features
    // can be evaluated in a range context and accept “min-” and “max-” prefixes on their name.
    if !media_feature_type_is_range(name.id) {
        return None;
    }

    Some(name)
}

fn component_values_parse_as_mf_value(component_values: &[ComponentValue]) -> bool {
    !component_values.is_empty() && component_values.iter().all(is_media_feature_value_component_value)
}

fn component_values_parse_as_mf_range_value(
    media_feature_id: MediaFeatureId,
    component_values: &[ComponentValue],
) -> bool {
    component_values_parse_as_mf_value(component_values)
        && component_values_parse_as_mf_value_syntax(media_feature_id, component_values)
            != MediaFeatureValueSyntaxKind::Ident
}

fn component_values_parse_as_mf_value_syntax(
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

fn component_values_parse_as_math_backed_mf_value(component_values: &[ComponentValue]) -> bool {
    matches!(component_values, [ComponentValue::Function(_)])
}

fn component_values_parse_as_mq_boolean(component_values: &[ComponentValue]) -> bool {
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

fn component_values_parse_as_integer(component_values: &[ComponentValue]) -> bool {
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

fn component_values_parse_as_length(component_values: &[ComponentValue]) -> bool {
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

fn component_value_parse_as_length_descriptor(component_value: &ComponentValue) -> bool {
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

fn component_value_parse_as_nonnegative_length_descriptor(component_value: &ComponentValue) -> bool {
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

fn component_value_parse_as_positive_percentage_descriptor(component_value: &ComponentValue) -> bool {
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

fn is_math_function_name(name: &str) -> bool {
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

fn is_page_size_keyword(input: &str) -> bool {
    // https://drafts.csswg.org/css-page-3/#typedef-page-size-page-size
    // <page-size> = A5 | A4 | A3 | B5 | B4 | JIS-B5 | JIS-B4 | letter | legal | ledger
    matches!(
        input.to_ascii_lowercase().as_str(),
        "a5" | "a4" | "a3" | "b5" | "b4" | "jis-b5" | "jis-b4" | "letter" | "legal" | "ledger"
    )
}

fn component_values_parse_as_resolution(component_values: &[ComponentValue]) -> bool {
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

fn component_values_parse_as_ratio(component_values: &[ComponentValue]) -> bool {
    // https://drafts.csswg.org/css-values-4/#ratios
    // <ratio> = <number [0,∞]> [ / <number [0,∞]> ]?
    let component_values = strip_whitespace(component_values);
    let [numerator] = component_values else {
        return component_values_parse_as_ratio_with_denominator(component_values);
    };

    component_value_parse_as_non_negative_number(numerator)
}

fn component_values_parse_as_ratio_with_denominator(component_values: &[ComponentValue]) -> bool {
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

fn component_value_parse_as_non_negative_number(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) if number.value() >= 0.0
    ) || matches!(component_value, ComponentValue::Function(_))
}

fn number_is_integer(number: NumericValue) -> bool {
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

fn parse_mf_comparison_at(component_values: &[ComponentValue], index: usize) -> Option<(MfComparison, usize)> {
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

fn mf_comparisons_are_range_compatible(left: MfComparison, right: MfComparison) -> bool {
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

fn is_media_feature_value_component_value(component_value: &ComponentValue) -> bool {
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

fn serialize_component_values_for_reparsing(
    component_values: &[ComponentValue],
    filtered_input: &str,
) -> Option<String> {
    let mut output = String::new();
    for component_value in component_values {
        serialize_component_value_for_reparsing(component_value, filtered_input, &mut output)?;
    }
    Some(output)
}

fn serialize_component_value_for_reparsing(
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

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            index: 0,
            rule_context: Vec::new(),
        }
    }

    // https://www.w3.org/TR/css-syntax-3/#urange-syntax
    fn parse_a_unicode_range(&mut self, filtered_input: &str) -> Option<CssUnicodeRange> {
        // <urange> =
        //  u '+' <ident-token> '?'* |
        //  u <dimension-token> '?'* |
        //  u <number-token> '?'* |
        //  u <number-token> <dimension-token> |
        //  u <number-token> <number-token> |
        //  u '+' '?'+
        // (All with no whitespace in between tokens.)
        self.discard_whitespace();
        let unicode_range = self.consume_a_unicode_range(filtered_input)?;
        self.discard_whitespace();
        if !matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return None;
        }
        Some(unicode_range)
    }

    // https://www.w3.org/TR/css-syntax-3/#urange-syntax
    fn parse_a_unicode_range_list(&mut self, filtered_input: &str) -> Option<Vec<CssUnicodeRange>> {
        let mut unicode_ranges = Vec::new();

        loop {
            self.discard_whitespace();
            unicode_ranges.push(self.consume_a_unicode_range(filtered_input)?);
            self.discard_whitespace();

            match self.next_input_token().token_type {
                TokenType::Comma => self.discard_a_token(),
                TokenType::EndOfFile => break,
                _ => return None,
            }
        }

        Some(unicode_ranges)
    }

    // https://www.w3.org/TR/css-syntax-3/#urange-syntax
    fn consume_a_unicode_range(&mut self, filtered_input: &str) -> Option<CssUnicodeRange> {
        let u = self.consume_the_next_input_token();
        if !matches!(u.token_type, TokenType::Ident { ref value } if value.eq_ignore_ascii_case("u")) {
            return None;
        }

        let second_token = self.consume_the_next_input_token();

        //  u '+' <ident-token> '?'* |
        //  u '+' '?'+
        if token_is_delim(&second_token, '+') {
            let mut text = token_original_source(&second_token, filtered_input)?.to_string();
            let third_token = self.consume_the_next_input_token();
            if matches!(third_token.token_type, TokenType::Ident { .. }) || token_is_delim(&third_token, '?') {
                text.push_str(token_original_source(&third_token, filtered_input)?);
                while token_is_delim(self.next_input_token(), '?') {
                    text.push_str(token_original_source(
                        &self.consume_the_next_input_token(),
                        filtered_input,
                    )?);
                }
                if self.next_input_token().is_unicode_range_ending_token() {
                    return parse_unicode_range_text(&text);
                }
            }
        }

        //  u <dimension-token> '?'*
        if matches!(second_token.token_type, TokenType::Dimension { .. }) {
            let mut text = token_original_source(&second_token, filtered_input)?.to_string();
            while token_is_delim(self.next_input_token(), '?') {
                text.push_str(token_original_source(
                    &self.consume_the_next_input_token(),
                    filtered_input,
                )?);
            }
            if self.next_input_token().is_unicode_range_ending_token() {
                return parse_unicode_range_text(&text);
            }
        }

        //  u <number-token> '?'* |
        //  u <number-token> <dimension-token> |
        //  u <number-token> <number-token>
        if matches!(second_token.token_type, TokenType::Number { .. }) {
            let mut text = token_original_source(&second_token, filtered_input)?.to_string();

            if self.next_input_token().is_unicode_range_ending_token() {
                return parse_unicode_range_text(&text);
            }

            let third_token = self.consume_the_next_input_token();
            if token_is_delim(&third_token, '?') {
                text.push_str(token_original_source(&third_token, filtered_input)?);
                while token_is_delim(self.next_input_token(), '?') {
                    text.push_str(token_original_source(
                        &self.consume_the_next_input_token(),
                        filtered_input,
                    )?);
                }
                if self.next_input_token().is_unicode_range_ending_token() {
                    return parse_unicode_range_text(&text);
                }
            } else if matches!(
                third_token.token_type,
                TokenType::Dimension { .. } | TokenType::Number { .. }
            ) {
                text.push_str(token_original_source(&third_token, filtered_input)?);
                if self.next_input_token().is_unicode_range_ending_token() {
                    return parse_unicode_range_text(&text);
                }
            }
        }

        None
    }

    // https://drafts.csswg.org/css-syntax/#parse-a-stylesheets-contents
    pub(crate) fn parse_a_stylesheets_contents(&mut self) -> Vec<Rule> {
        // To parse a stylesheet’s contents from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Consume a stylesheet’s contents from input, and return the result.
        self.consume_a_stylesheets_contents()
    }

    // https://drafts.csswg.org/css-syntax/#parse-block-contents
    pub(crate) fn parse_a_blocks_contents(&mut self) -> Vec<RuleOrListOfDeclarations> {
        // To parse a block’s contents from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Consume a block’s contents from input, and return the result.
        self.consume_a_blocks_contents()
    }

    // https://drafts.csswg.org/css-syntax/#parse-rule
    pub(crate) fn parse_a_rule(&mut self) -> Option<Rule> {
        // To parse a rule from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Discard whitespace from input.
        self.discard_whitespace();

        // 3. If the next token from input is an <EOF-token>, return a syntax error.
        let rule = if matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return None;
        }
        // Otherwise, if the next token from input is an <at-keyword-token>, consume an at-rule from input,
        // and let rule be the return value.
        else if matches!(self.next_input_token().token_type, TokenType::AtKeyword { .. }) {
            Rule::AtRule(self.consume_an_at_rule(Nested::No)?)
        }
        // Otherwise, consume a qualified rule from input and let rule be the return value.
        // If nothing or an invalid rule error was returned, return a syntax error.
        else {
            Rule::QualifiedRule(self.consume_a_qualified_rule(None, Nested::No)?)
        };

        // 4. Discard whitespace from input.
        self.discard_whitespace();

        // 5. If the next token from input is an <EOF-token>, return rule. Otherwise, return a syntax error.
        if matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return Some(rule);
        }
        None
    }

    // https://drafts.csswg.org/css-syntax/#parse-declaration
    pub(crate) fn parse_a_declaration(&mut self) -> Option<Declaration> {
        // To parse a declaration from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        self.rule_context.push(RuleContext::Style);
        let declaration = self.parse_a_declaration_with_current_context();
        self.rule_context.pop();
        declaration
    }

    fn parse_a_declaration_with_current_context(&mut self) -> Option<Declaration> {
        // 2. Discard whitespace from input.
        self.discard_whitespace();

        // 3. Consume a declaration from input. If anything was returned, return it.
        // Otherwise, return a syntax error.
        self.consume_a_declaration(Nested::No)
    }

    // https://drafts.csswg.org/css-syntax/#parse-list-of-component-values
    pub(crate) fn parse_a_list_of_component_values(&mut self) -> Vec<ComponentValue> {
        // To parse a list of component values from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Consume a list of component values from input, and return the result.
        self.consume_a_list_of_component_values(None, Nested::No)
    }

    // https://drafts.csswg.org/css-syntax/#parse-comma-separated-list-of-component-values
    pub(crate) fn parse_a_comma_separated_list_of_component_values(&mut self) -> Vec<Vec<ComponentValue>> {
        // To parse a comma-separated list of component values from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Let groups be an empty list.
        let mut groups = Vec::new();

        // 3. While input is not empty:
        let mut just_consumed_comma = false;
        while !matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            // 1. Consume a list of component values from input, with <comma-token> as the stop token,
            // and append the result to groups.
            groups.push(self.consume_a_list_of_component_values(Some(TokenType::Comma), Nested::No));

            // 2. Discard a token from input.
            just_consumed_comma = matches!(self.consume_the_next_input_token().token_type, TokenType::Comma);
        }

        // AD-HOC: Also append an empty group if there was a trailing comma.
        // Some related spec discussion: https://github.com/w3c/csswg-drafts/issues/11254
        if just_consumed_comma {
            groups.push(Vec::new());
        }

        // 4. Return groups.
        groups
    }

    pub(crate) fn parse_a_media_query_list(&mut self) -> Vec<MediaQuerySyntax> {
        // https://drafts.csswg.org/mediaqueries-5/#typedef-media-query-list
        // To parse a <media-query-list> production,
        // parse a comma-separated list of component values,
        // then parse each entry in the returned list as a <media-query>.
        // Its value is the list of <media-query>s so produced.
        let groups = self.parse_a_comma_separated_list_of_component_values();

        // AD-HOC: Ignore whitespace-only queries
        // to make `@media {..}` equivalent to `@media all {..}`.
        if groups.len() == 1 && strip_whitespace(&groups[0]).is_empty() {
            return Vec::new();
        }

        groups.into_iter().map(component_values_parse_as_media_query).collect()
    }

    // https://drafts.csswg.org/css-syntax/#parse-component-value
    pub(crate) fn parse_a_component_value(&mut self) -> Option<ComponentValue> {
        // To parse a component value from input:
        // 1. Normalize input, and set input to the result.
        // NOTE: This is done automatically before creating the Parser.

        // 2. Discard whitespace from input.
        self.discard_whitespace();

        // 3. If input is empty, return a syntax error.
        if matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return None;
        }

        // 4. Consume a component value from input and let value be the return value.
        let component_value = self.consume_a_component_value();

        // 5. Discard whitespace from input.
        self.discard_whitespace();

        // 6. If input is empty, return value. Otherwise, return a syntax error.
        if matches!(self.next_input_token().token_type, TokenType::EndOfFile) {
            return Some(component_value);
        }
        None
    }

    fn next_input_token(&self) -> &Token {
        self.tokens
            .get(self.index)
            .or_else(|| self.tokens.last())
            .expect("CSS parser requires an EOF token")
    }

    fn consume_the_next_input_token(&mut self) -> Token {
        let token = self.next_input_token().clone();
        self.index += 1;
        token
    }

    fn discard_a_token(&mut self) {
        self.index += 1;
    }

    fn discard_whitespace(&mut self) {
        while matches!(self.next_input_token().token_type, TokenType::Whitespace) {
            self.discard_a_token();
        }
    }

    fn peek_token(&self, offset: usize) -> &Token {
        self.tokens
            .get(self.index + offset)
            .or_else(|| self.tokens.last())
            .expect("CSS parser requires an EOF token")
    }

    // https://drafts.csswg.org/css-syntax/#consume-stylesheet-contents
    fn consume_a_stylesheets_contents(&mut self) -> Vec<Rule> {
        // To consume a stylesheet’s contents from a token stream input:
        // Let rules be an initially empty list of rules.
        let mut rules = Vec::new();

        // Process input:
        loop {
            let token = self.next_input_token();

            // <whitespace-token>
            if matches!(token.token_type, TokenType::Whitespace) {
                // Discard a token from input.
                self.discard_a_token();
                continue;
            }

            // <EOF-token>
            if matches!(token.token_type, TokenType::EndOfFile) {
                // Return rules.
                return rules;
            }

            // <CDO-token>
            // <CDC-token>
            if matches!(token.token_type, TokenType::Cdo | TokenType::Cdc) {
                // Discard a token from input.
                self.discard_a_token();
                continue;
            }

            // <at-keyword-token>
            if matches!(token.token_type, TokenType::AtKeyword { .. }) {
                // Consume an at-rule from input. If anything is returned, append it to rules.
                if let Some(rule) = self.consume_an_at_rule(Nested::No) {
                    rules.push(Rule::AtRule(rule));
                }
                continue;
            }

            // anything else
            // Consume a qualified rule from input. If a rule is returned, append it to rules.
            if let Some(rule) = self.consume_a_qualified_rule(None, Nested::No) {
                rules.push(Rule::QualifiedRule(rule));
            }
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-at-rule
    fn consume_an_at_rule(&mut self, nested: Nested) -> Option<AtRule> {
        // To consume an at-rule from a token stream input, given an optional bool nested (default false):
        // Assert: The next token is an <at-keyword-token>.
        assert!(matches!(
            self.next_input_token().token_type,
            TokenType::AtKeyword { .. }
        ));

        // Consume a token from input, and let rule be a new at-rule with its name set to the returned token’s value,
        // its prelude initially set to an empty list, and no declarations or child rules.
        let token = self.consume_the_next_input_token();
        let TokenType::AtKeyword { name } = token.token_type else {
            unreachable!("consume_an_at_rule requires an at-keyword token");
        };
        let mut rule = AtRule {
            name,
            prelude: Vec::new(),
            child_rules_and_lists_of_declarations: Vec::new(),
            is_block_rule: false,
        };

        // Process input:
        loop {
            let token = self.next_input_token();

            // <semicolon-token>
            // <EOF-token>
            if matches!(token.token_type, TokenType::Semicolon | TokenType::EndOfFile) {
                // Discard a token from input. If rule is valid in the current context, return it;
                // otherwise return nothing.
                self.discard_a_token();
                if self.is_at_rule_valid_in_the_current_context(&rule) {
                    return Some(rule);
                }
                return None;
            }

            // <}-token>
            if matches!(token.token_type, TokenType::CloseCurly) {
                // If nested is true:
                if nested == Nested::Yes {
                    // If rule is valid in the current context, return it.
                    if self.is_at_rule_valid_in_the_current_context(&rule) {
                        return Some(rule);
                    }
                    return None;
                }
                // Otherwise, consume a token and append the result to rule’s prelude.
                rule.prelude
                    .push(ComponentValue::PreservedToken(self.consume_the_next_input_token()));
                continue;
            }

            // <{-token>
            if matches!(token.token_type, TokenType::OpenCurly) {
                // Consume a block from input, and assign the result to rule’s child rules.
                self.rule_context.push(rule_context_type_for_at_rule(&rule.name));
                rule.child_rules_and_lists_of_declarations = self.consume_a_block();
                self.rule_context.pop();
                rule.is_block_rule = true;

                // If rule is valid in the current context, return it. Otherwise, return nothing.
                if self.is_at_rule_valid_in_the_current_context(&rule) {
                    return Some(rule);
                }
                return None;
            }

            // anything else
            // Consume a component value from input and append the returned value to rule’s prelude.
            rule.prelude.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-qualified-rule
    fn consume_a_qualified_rule(&mut self, stop_token: Option<TokenType>, nested: Nested) -> Option<QualifiedRule> {
        // To consume a qualified rule, from a token stream input, given an optional token stop token
        // and an optional bool nested (default false):

        // Let rule be a new qualified rule with its prelude, declarations, and child rules all initially set to empty lists.
        let mut rule = QualifiedRule {
            prelude: Vec::new(),
            declarations: Vec::new(),
            child_rules: Vec::new(),
        };

        // NOTE: Qualified rules inside @keyframes are a keyframe rule.
        //       We'll assume all others are style rules.
        let type_of_qualified_rule = if self.rule_context.last() == Some(&RuleContext::AtKeyframes) {
            RuleContext::Keyframe
        } else {
            RuleContext::Style
        };

        // Process input:
        loop {
            let token = self.next_input_token();

            // <EOF-token>
            // stop token (if passed)
            if matches!(token.token_type, TokenType::EndOfFile)
                || stop_token
                    .as_ref()
                    .is_some_and(|stop_token| token.token_type == *stop_token)
            {
                // This is a parse error. Return nothing.
                return None;
            }

            // <}-token>
            if matches!(token.token_type, TokenType::CloseCurly) {
                // This is a parse error. If nested is true, return nothing.
                // Otherwise, consume a token and append the result to rule’s prelude.
                if nested == Nested::Yes {
                    return None;
                }
                rule.prelude
                    .push(ComponentValue::PreservedToken(self.consume_the_next_input_token()));
                continue;
            }

            // <{-token>
            if matches!(token.token_type, TokenType::OpenCurly) {
                // If the first two non-<whitespace-token> values of rule’s prelude are an <ident-token>
                // whose value starts with "--" followed by a <colon-token>, then:
                let mut prelude = rule
                    .prelude
                    .iter()
                    .filter(|value| !is_whitespace_component_value(value));
                let starts_like_custom_property_declaration = matches!(
                    (prelude.next(), prelude.next()),
                    (
                        Some(ComponentValue::PreservedToken(Token {
                            token_type: TokenType::Ident { value },
                            ..
                        })),
                        Some(ComponentValue::PreservedToken(Token {
                            token_type: TokenType::Colon,
                            ..
                        }))
                    ) if value.starts_with("--")
                );

                if starts_like_custom_property_declaration {
                    // If nested is true, consume the remnants of a bad declaration from input,
                    // with nested set to true, and return nothing.
                    if nested == Nested::Yes {
                        self.consume_the_remnants_of_a_bad_declaration(Nested::Yes);
                        return None;
                    }

                    // If nested is false, consume a block from input, and return nothing.
                    let _ = self.consume_a_block();
                    return None;
                }

                // Otherwise, consume a block from input, and let child rules be the result.
                self.rule_context.push(type_of_qualified_rule);
                rule.child_rules = self.consume_a_block();
                self.rule_context.pop();

                // If the first item of child rules is a list of declarations, remove it from child rules
                // and assign it to rule’s declarations.
                if matches!(
                    rule.child_rules.first(),
                    Some(RuleOrListOfDeclarations::ListOfDeclarations(_))
                ) && let RuleOrListOfDeclarations::ListOfDeclarations(declarations) = rule.child_rules.remove(0)
                {
                    rule.declarations = declarations;
                }

                // If rule is valid in the current context, return it; otherwise return an invalid rule error.
                if self.is_qualified_rule_valid_in_the_current_context() {
                    return Some(rule);
                }
                return None;
            }

            // anything else
            // Consume a component value from input and append the result to rule’s prelude.
            rule.prelude.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-block
    fn consume_a_block(&mut self) -> Vec<RuleOrListOfDeclarations> {
        // To consume a block, from a token stream input:
        // Assert: The next token is a <{-token>.
        assert!(matches!(self.next_input_token().token_type, TokenType::OpenCurly));

        // Discard a token from input.
        self.discard_a_token();

        // Consume a block’s contents from input and let rules be the result.
        let rules = self.consume_a_blocks_contents();

        // Discard a token from input.
        self.discard_a_token();

        // Return rules.
        rules
    }

    // https://drafts.csswg.org/css-syntax/#consume-block-contents
    fn consume_a_blocks_contents(&mut self) -> Vec<RuleOrListOfDeclarations> {
        // To consume a block’s contents from a token stream input:
        // Let rules be an empty list, containing either rules or lists of declarations.
        let mut rules = Vec::new();

        // Let decls be an empty list of declarations.
        let mut declarations = Vec::new();

        // Process input:
        loop {
            let token = self.next_input_token();

            // <whitespace-token>
            // <semicolon-token>
            if matches!(token.token_type, TokenType::Whitespace | TokenType::Semicolon) {
                // Discard a token from input.
                self.discard_a_token();
                continue;
            }

            // <EOF-token>
            // <}-token>
            if matches!(token.token_type, TokenType::EndOfFile | TokenType::CloseCurly) {
                // AD-HOC: If decls is not empty, append it to rules.
                // Spec issue: https://github.com/w3c/csswg-drafts/issues/11017
                if !declarations.is_empty() {
                    rules.push(RuleOrListOfDeclarations::ListOfDeclarations(declarations));
                }
                // Return rules.
                return rules;
            }

            // <at-keyword-token>
            if matches!(token.token_type, TokenType::AtKeyword { .. }) {
                // If decls is not empty, append it to rules, and set decls to a fresh empty list of declarations.
                if !declarations.is_empty() {
                    rules.push(RuleOrListOfDeclarations::ListOfDeclarations(declarations));
                    declarations = Vec::new();
                }

                // Consume an at-rule from input, with nested set to true.
                // If a rule was returned, append it to rules.
                if let Some(rule) = self.consume_an_at_rule(Nested::Yes) {
                    rules.push(RuleOrListOfDeclarations::Rule(Rule::AtRule(rule)));
                }
                continue;
            }

            // anything else
            // OPTIMIZATION: Look ahead to determine if this can be a declaration (ident whitespace* ':').
            // If not, skip straight to qualified rule parsing.
            let could_be_declaration = if matches!(token.token_type, TokenType::Ident { .. }) {
                let mut lookahead = 1;
                while matches!(self.peek_token(lookahead).token_type, TokenType::Whitespace) {
                    lookahead += 1;
                }
                matches!(self.peek_token(lookahead).token_type, TokenType::Colon)
            } else {
                false
            };

            if could_be_declaration {
                // Mark input.
                let mark = self.index;

                // Consume a declaration from input, with nested set to true.
                if let Some(declaration) = self.consume_a_declaration(Nested::Yes) {
                    // If anything was returned, append it to decls.
                    declarations.push(declaration);
                    continue;
                }

                // Otherwise, restore input.
                self.index = mark;
            }

            // Consume a qualified rule from input, with nested set to true, and with <semicolon-token> as the stop token.
            // If a rule was returned, append it to rules.
            // If an invalid rule error was returned, append decls to rules and set decls to a fresh empty list of declarations.
            if let Some(rule) = self.consume_a_qualified_rule(Some(TokenType::Semicolon), Nested::Yes) {
                if !declarations.is_empty() {
                    rules.push(RuleOrListOfDeclarations::ListOfDeclarations(declarations));
                    declarations = Vec::new();
                }
                rules.push(RuleOrListOfDeclarations::Rule(Rule::QualifiedRule(rule)));
            } else if !declarations.is_empty() {
                rules.push(RuleOrListOfDeclarations::ListOfDeclarations(declarations));
                declarations = Vec::new();
            }
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-declaration
    fn consume_a_declaration(&mut self, nested: Nested) -> Option<Declaration> {
        // To consume a declaration from a token stream input, given an optional bool nested (default false):

        // Let decl be a new declaration, with an initially empty name and a value set to an empty list.
        let mut declaration = Declaration {
            name: String::new(),
            value: Vec::new(),
            important: false,
        };

        // 1. If the next token is an <ident-token>, consume a token from input and set decl’s name to the token’s value.
        if matches!(self.next_input_token().token_type, TokenType::Ident { .. }) {
            let token = self.consume_the_next_input_token();
            let TokenType::Ident { value } = token.token_type else {
                unreachable!("declaration names require ident tokens")
            };
            declaration.name = value;
        }
        // Otherwise, consume the remnants of a bad declaration from input, with nested, and return nothing.
        else {
            self.consume_the_remnants_of_a_bad_declaration(nested);
            return None;
        }

        // 2. Discard whitespace from input.
        self.discard_whitespace();

        // 3. If the next token is a <colon-token>, discard a token from input.
        if matches!(self.next_input_token().token_type, TokenType::Colon) {
            self.discard_a_token();
        }
        // Otherwise, consume the remnants of a bad declaration from input, with nested, and return nothing.
        else {
            self.consume_the_remnants_of_a_bad_declaration(nested);
            return None;
        }

        // 4. Discard whitespace from input.
        self.discard_whitespace();

        // 5. Consume a list of component values from input, with nested, and with <semicolon-token> as the stop token,
        // and set decl’s value to the result.
        declaration.value = self.consume_a_list_of_component_values(Some(TokenType::Semicolon), nested);

        // 6. If the last two non-<whitespace-token>s in decl’s value are a <delim-token> with the value "!"
        // followed by an <ident-token> with a value that is an ASCII case-insensitive match for "important",
        // remove them from decl’s value and set decl’s important flag.
        if let Some(important_index) = declaration
            .value
            .iter()
            .rposition(|value| is_ident_component_value(value, "important"))
        {
            let has_only_whitespace_after_important = declaration.value[important_index + 1..]
                .iter()
                .all(is_whitespace_component_value);
            if has_only_whitespace_after_important
                && let Some(bang_index) = declaration.value[..important_index]
                    .iter()
                    .rposition(is_bang_component_value)
            {
                let has_only_whitespace_between_bang_and_important = declaration.value[bang_index + 1..important_index]
                    .iter()
                    .all(is_whitespace_component_value);
                if has_only_whitespace_between_bang_and_important {
                    declaration.value.remove(important_index);
                    declaration.value.remove(bang_index);
                    declaration.important = true;
                }
            }
        }

        // 7. While the last item in decl’s value is a <whitespace-token>, remove that token.
        while declaration.value.last().is_some_and(is_whitespace_component_value) {
            declaration.value.pop();
        }

        // 8. If decl’s name is a custom property name string, then set decl’s original text to the segment
        // of the original source text string corresponding to the tokens of decl’s value.
        if declaration.name.starts_with("--") {
            // TODO: Preserve original text once the rule/declaration FFI surface exists.
        }
        // Otherwise, if decl’s value contains a top-level simple block with an associated token of <{-token>,
        // and also contains any other non-<whitespace-token> value, return nothing.
        else if contains_a_curly_block_and_non_whitespace(&declaration.value) {
            return None;
        }
        // Otherwise, if decl’s name is an ASCII case-insensitive match for "unicode-range", consume the value of
        // a unicode-range descriptor from the segment of the original source text string corresponding to the
        // tokens returned by the consume a list of component values call, and replace decl’s value with the result.
        else if declaration.name.eq_ignore_ascii_case("unicode-range") {
            // FIXME: Special unicode-range handling.
        }

        // 9. If decl is valid in the current context, return it; otherwise return nothing.
        if self.is_declaration_valid_in_the_current_context(&declaration) {
            return Some(declaration);
        }
        None
    }

    // https://drafts.csswg.org/css-syntax/#consume-the-remnants-of-a-bad-declaration
    fn consume_the_remnants_of_a_bad_declaration(&mut self, nested: Nested) {
        // To consume the remnants of a bad declaration from a token stream input, given a bool nested:
        // Process input:
        loop {
            let token = self.next_input_token();

            // <eof-token>
            // <semicolon-token>
            if matches!(token.token_type, TokenType::EndOfFile | TokenType::Semicolon) {
                // Discard a token from input, and return nothing.
                self.discard_a_token();
                return;
            }

            // <}-token>
            if matches!(token.token_type, TokenType::CloseCurly) {
                // If nested is true, return nothing. Otherwise, discard a token.
                if nested == Nested::Yes {
                    return;
                }
                self.discard_a_token();
                continue;
            }

            // anything else
            // Consume a component value from input, and do nothing.
            self.consume_a_component_value();
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-list-of-component-values
    fn consume_a_list_of_component_values(
        &mut self,
        stop_token: Option<TokenType>,
        nested: Nested,
    ) -> Vec<ComponentValue> {
        // To consume a list of component values from a token stream input, given an optional token stop token
        // and an optional boolean nested (default false):
        // Let values be an empty list of component values.
        let mut values = Vec::new();

        // Process input:
        loop {
            let token = self.next_input_token();

            // <eof-token>
            // stop token (if passed)
            if matches!(token.token_type, TokenType::EndOfFile)
                || stop_token
                    .as_ref()
                    .is_some_and(|stop_token| token.token_type == *stop_token)
            {
                // Return values.
                return values;
            }

            // <}-token>
            if matches!(token.token_type, TokenType::CloseCurly) && nested == Nested::Yes {
                // If nested is true, return values.
                return values;
            }

            // anything else
            // Consume a component value from input, and append the result to values.
            values.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-component-value
    fn consume_a_component_value(&mut self) -> ComponentValue {
        // To consume a component value from a stream of CSS component values input:
        // Consume the next input token.
        let token = self.consume_the_next_input_token();

        match token.token_type {
            // <{-token>, <[-token>, <(-token>
            TokenType::OpenCurly | TokenType::OpenSquare | TokenType::OpenParen => {
                // Consume a simple block and return it.
                ComponentValue::SimpleBlock(self.consume_a_simple_block(token))
            }

            // <function-token>
            TokenType::Function { .. } => {
                // Consume a function and return it.
                ComponentValue::Function(self.consume_a_function(token))
            }

            // anything else
            _ => {
                // Return the current input token.
                ComponentValue::PreservedToken(token)
            }
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-simple-block
    fn consume_a_simple_block(&mut self, token: Token) -> SimpleBlock {
        // To consume a simple block from a stream of CSS component values input:
        // The ending token is the mirror variant of the current input token.
        let ending_token_type = mirror_variant(&token.token_type);

        // Let value be an initially empty list of component values.
        let mut value = Vec::new();

        loop {
            // Repeatedly consume the next input token and process it as follows:
            let next_token = self.next_input_token();

            // ending token
            if next_token.token_type == ending_token_type {
                // Return the block.
                return SimpleBlock {
                    token,
                    value,
                    end_token: self.consume_the_next_input_token(),
                };
            }

            // <eof-token>
            if matches!(next_token.token_type, TokenType::EndOfFile) {
                // This is a parse error. Return the block.
                return SimpleBlock {
                    token,
                    value,
                    end_token: self.consume_the_next_input_token(),
                };
            }

            // anything else
            // Reconsume the current input token. Consume a component value and append the returned value to the block’s value.
            value.push(self.consume_a_component_value());
        }
    }

    // https://drafts.csswg.org/css-syntax/#consume-function
    fn consume_a_function(&mut self, token: Token) -> Function {
        // To consume a function from a stream of CSS component values input:
        // Let function be a function with its name equal to the value of the current input token,
        // and with a value set to an empty list.
        let name = match &token.token_type {
            TokenType::Function { name } => name.clone(),
            _ => unreachable!("consume_a_function requires a function token"),
        };
        let mut value = Vec::new();

        loop {
            // Repeatedly consume the next input token and process it as follows:
            let next_token = self.next_input_token();

            // <)-token>
            if matches!(next_token.token_type, TokenType::CloseParen) {
                // Return the function.
                return Function {
                    name,
                    value,
                    name_token: token,
                    end_token: self.consume_the_next_input_token(),
                };
            }

            // <eof-token>
            if matches!(next_token.token_type, TokenType::EndOfFile) {
                // This is a parse error. Return the function.
                return Function {
                    name,
                    value,
                    name_token: token,
                    end_token: self.consume_the_next_input_token(),
                };
            }

            // anything else
            // Reconsume the current input token. Consume a component value and append the returned value to the function’s value.
            value.push(self.consume_a_component_value());
        }
    }
}

fn mirror_variant(token_type: &TokenType) -> TokenType {
    match token_type {
        TokenType::OpenCurly => TokenType::CloseCurly,
        TokenType::OpenSquare => TokenType::CloseSquare,
        TokenType::OpenParen => TokenType::CloseParen,
        _ => unreachable!("CSS simple blocks must start with a grouping token"),
    }
}

fn is_whitespace_component_value(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Whitespace,
            ..
        })
    )
}

fn is_ident_component_value(component_value: &ComponentValue, ident: &str) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) if value.eq_ignore_ascii_case(ident)
    )
}

fn is_bang_component_value(component_value: &ComponentValue) -> bool {
    matches!(
        component_value,
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Delim { value },
            ..
        }) if *value == '!' as u32
    )
}

fn contains_a_curly_block_and_non_whitespace(declaration_value: &[ComponentValue]) -> bool {
    let mut contains_curly_block = false;
    let mut contains_non_whitespace = false;
    for value in declaration_value {
        if let ComponentValue::SimpleBlock(block) = value
            && matches!(block.token.token_type, TokenType::OpenCurly)
        {
            if contains_non_whitespace {
                return true;
            }
            contains_curly_block = true;
            continue;
        }

        if !is_whitespace_component_value(value) {
            if contains_curly_block {
                return true;
            }
            contains_non_whitespace = true;
        }
    }
    false
}

impl Parser {
    fn is_declaration_valid_in_the_current_context(&self, declaration: &Declaration) -> bool {
        let Some(context) = self.rule_context.last() else {
            return false;
        };

        match context {
            RuleContext::Unknown => false,
            RuleContext::Style => true,
            RuleContext::Keyframe => {
                // https://drafts.csswg.org/css-animations-1/#keyframes
                // The <declaration-list> inside of <keyframe-block> accepts any CSS property except those defined in
                // this specification, but does accept the animation-timing-function property and interprets it specially
                // NB: animation-composition is defined in CSS Animations Level 2, so it is not excluded by this rule.
                !is_animation_property_disallowed_in_keyframe(&declaration.name)
            }
            RuleContext::AtContainer | RuleContext::AtLayer | RuleContext::AtMedia | RuleContext::AtSupports => self
                .rule_context
                .iter()
                .any(|context| matches!(context, RuleContext::Style | RuleContext::AtFunction)),
            RuleContext::FontFeatureValue => true,
            RuleContext::AtFunction => true,
            RuleContext::AtCounterStyle
            | RuleContext::AtFontFace
            | RuleContext::AtFontFeatureValues
            | RuleContext::AtPage
            | RuleContext::AtProperty
            | RuleContext::Margin => true,
            RuleContext::AtKeyframes => false,
            RuleContext::SupportsCondition => true,
        }
    }

    fn is_at_rule_valid_in_the_current_context(&self, at_rule: &AtRule) -> bool {
        if self.rule_context.is_empty() {
            return !is_margin_rule_name(&at_rule.name);
        }

        if self
            .rule_context
            .iter()
            .any(|context| matches!(context, RuleContext::Style))
        {
            return first_is_one_of(&at_rule.name, &["container", "layer", "media", "supports"]);
        }

        if self
            .rule_context
            .iter()
            .any(|context| matches!(context, RuleContext::AtFunction))
        {
            return first_is_one_of(&at_rule.name, &["container", "media", "supports"]);
        }

        match self.rule_context.last().expect("checked non-empty context") {
            RuleContext::Unknown => false,
            RuleContext::Style => unreachable!("style context handled above"),
            RuleContext::AtContainer | RuleContext::AtLayer | RuleContext::AtMedia | RuleContext::AtSupports => {
                !first_is_one_of(&at_rule.name, &["import", "namespace"])
            }
            RuleContext::SupportsCondition => false,
            RuleContext::AtPage => is_margin_rule_name(&at_rule.name),
            RuleContext::AtCounterStyle
            | RuleContext::AtFontFace
            | RuleContext::FontFeatureValue
            | RuleContext::AtKeyframes
            | RuleContext::Keyframe
            | RuleContext::AtProperty
            | RuleContext::Margin => false,
            RuleContext::AtFontFeatureValues => is_font_feature_value_type_at_keyword(&at_rule.name),
            RuleContext::AtFunction => unreachable!("function context handled above"),
        }
    }

    fn is_qualified_rule_valid_in_the_current_context(&self) -> bool {
        let Some(context) = self.rule_context.last() else {
            return true;
        };

        match context {
            RuleContext::Unknown => false,
            RuleContext::Style
            | RuleContext::AtContainer
            | RuleContext::AtLayer
            | RuleContext::AtMedia
            | RuleContext::AtSupports
            | RuleContext::AtKeyframes => true,
            RuleContext::SupportsCondition
            | RuleContext::AtCounterStyle
            | RuleContext::AtFontFace
            | RuleContext::AtFontFeatureValues
            | RuleContext::FontFeatureValue
            | RuleContext::AtFunction
            | RuleContext::AtPage
            | RuleContext::AtProperty
            | RuleContext::Keyframe
            | RuleContext::Margin => false,
        }
    }
}

fn rule_context_type_for_at_rule(name: &str) -> RuleContext {
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

fn first_is_one_of(name: &str, values: &[&str]) -> bool {
    values.iter().any(|value| name.eq_ignore_ascii_case(value))
}

fn is_margin_rule_name(name: &str) -> bool {
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

fn is_font_feature_value_type_at_keyword(name: &str) -> bool {
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

fn is_animation_property_disallowed_in_keyframe(name: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::{
        BooleanExpression, BooleanExpressionTestKind, ComponentValue, ComponentValueParser,
        CssAnchorNameOrScopeValueKind, CssAnimationNameItemKind, CssAnimationNameValueKind,
        CssBooleanExpressionEventKind, CssColorSchemeValueKind, CssContainValue, CssContainValueKind,
        CssContainerTypeValueKind, CssCounterStyleKind, CssCounterStyleNegativeSymbolCount, CssCounterStyleRangeKind,
        CssCounterStyleSymbolsType, CssCounterStyleSystemKind, CssCropOrCrossKind, CssFontLanguageOverrideKind,
        CssFontSourceKind, CssFontTech, CssFontVariantAlternatesValueKind, CssFontVariantEastAsianValueKind,
        CssFontVariantLigaturesValueKind, CssFontVariantNumericValueKind, CssFontVariantSimpleValueKind, CssMediaQuery,
        CssMediaTypeKind, CssNonnegativeIntegerSymbolPairOrder, CssOpenTypeSettingsKind, CssOpenTypeTaggedValueKind,
        CssPagePseudoClassKind, CssPaintOrderKeyword, CssPaintOrderValue, CssPaintOrderValueKind,
        CssPositionAnchorValueKind, CssPositionTryOrderValue, CssPositionVisibilityValue,
        CssPositionVisibilityValueKind, CssQuotesValueKind, CssScrollbarGutterValueKind, CssSupportsFeatureKind,
        CssTextUnderlinePositionHorizontal, CssTextUnderlinePositionValue, CssTextUnderlinePositionVertical,
        CssTextWrapModeValue, CssTextWrapStyleValue, CssTextWrapValue, CssTextWrapValueKind, CssTimelineNameItemKind,
        CssTimelineNameValueKind, CssTimelineScopeValueKind, CssTouchActionKeyword, CssTouchActionValue,
        CssTouchActionValueKind, CssTransitionBehaviorItemKind, CssTransitionBehaviorValueKind,
        CssTransitionPropertyValueKind, CssUrlFunctionType, CssUrlModifierKind, CssValueTypeSyntaxKind,
        CssViewTransitionNameValueKind, CssWhiteSpaceTrimValue, CssWhiteSpaceTrimValueKind, CssWillChangeFeatureKind,
        CssWillChangeValueKind, FamilyName, FontFamilyValue, FontStyle, FontVariant, FontVariantAlternatesValue,
        FontVariantEastAsianValue, FontVariantLigaturesValue, FontVariantNumericValue, MediaFeatureNameKind,
        MediaFeatureSyntax, MediaFeatureValueSyntaxKind, MediaQueryModifier, MediaQuerySyntax, MfComparison,
        OpenTypeTaggedValue, Parser, Rule, RuleContext, RuleOrListOfDeclarations, SyntaxNode,
        component_values_parse_as_media_feature, component_values_parse_as_mf_value_syntax,
        component_values_parse_as_syntax, component_values_parse_as_syntax_with_source,
        component_values_parse_as_value_type, parse_a_counter_style, parse_a_counter_style_name, parse_a_custom_ident,
        parse_a_custom_property_name, parse_a_dashed_ident, parse_a_family_name, parse_a_font_family_value,
        parse_a_font_feature_settings, parse_a_font_language_override, parse_a_font_source, parse_a_font_style,
        parse_a_font_variant, parse_a_font_variant_alternates, parse_a_font_variant_east_asian,
        parse_a_font_variant_ligatures, parse_a_font_variant_numeric, parse_a_font_variation_settings,
        parse_a_keyframe_selector_list, parse_a_keyframes_name, parse_a_layer_name, parse_a_layer_name_list,
        parse_a_media_query, parse_a_media_test, parse_a_namespace_rule_prelude,
        parse_a_nonnegative_integer_symbol_pair, parse_a_page_selector_list, parse_a_supports_feature,
        parse_a_unicode_range, parse_a_unicode_range_list, parse_a_url_function, parse_a_value_type,
        parse_an_if_condition, parse_an_import_layer, parse_an_import_url, parse_an_opentype_tag,
        parse_anchor_name_or_scope_value, parse_animation_name_value, parse_color_scheme_value, parse_contain_value,
        parse_container_rule_prelude, parse_container_type_value, parse_counter_style_additive_symbols,
        parse_counter_style_negative, parse_counter_style_range, parse_counter_style_symbol,
        parse_counter_style_symbols, parse_counter_style_system, parse_crop_or_cross, parse_empty_prelude,
        parse_font_feature_values_family_name_list, parse_font_feature_values_feature_value,
        parse_font_weight_absolute_pair, parse_length_descriptor, parse_optional_declaration_value_descriptor,
        parse_page_size_descriptor, parse_paint_order_value, parse_position_anchor_value,
        parse_position_try_order_value, parse_position_visibility_value, parse_positive_percentage_descriptor,
        parse_quotes_value, parse_scrollbar_gutter_value, parse_string_descriptor, parse_text_underline_position_value,
        parse_text_wrap_mode_value, parse_text_wrap_style_value, parse_text_wrap_value, parse_timeline_name_value,
        parse_timeline_scope_value, parse_touch_action_value, parse_transition_behavior_value,
        parse_transition_property_value, parse_view_transition_name_value, parse_white_space_trim_value,
        parse_will_change_value, strip_whitespace,
    };
    use crate::css_tokenizer::{self, TokenType};
    use crate::generated_media_features::{
        MediaFeatureId, MediaFeatureValueType, media_feature_accepts_identifier, media_feature_accepts_type,
        media_feature_identifier_is_falsey,
    };
    use crate::generated_properties::{
        PropertyId, PropertyNumericRange, PropertyValueType, longhands_for_shorthand,
        property_accepted_range_by_value_type, property_accepts_keyword, property_accepts_value_type,
        property_custom_ident_blacklist, property_id_from_string, property_name,
        property_resolves_percentages_relative_to, resolve_legacy_value_alias,
    };
    use crate::generated_pseudo_classes::{
        PseudoClassId, PseudoClassParameterType, pseudo_class_id_from_string, pseudo_class_metadata, pseudo_class_name,
    };
    use crate::generated_pseudo_elements::{
        PseudoElementId, PseudoElementParameterType, aliased_pseudo_element_id_from_string,
        pseudo_element_id_from_string, pseudo_element_metadata, pseudo_element_name,
    };
    use crate::generated_units::{DimensionType, dimension_for_unit};
    use crate::generated_value_types::ValueTypeId;

    fn parse_with<T>(input: &str, parse: impl FnOnce(&mut Parser) -> T) -> T {
        let mut tokens = Vec::new();
        css_tokenizer::tokenize(input.as_bytes(), |token, _| tokens.push(token.clone()));
        parse(&mut Parser::new(tokens))
    }

    fn parse(input: &str) -> Vec<ComponentValue> {
        parse_with(input, Parser::parse_a_list_of_component_values)
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
                    std::str::from_utf8_unchecked(std::slice::from_raw_parts(
                        url_function.url_ptr,
                        url_function.url_len,
                    ))
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
        let parsed =
            parse_a_nonnegative_integer_symbol_pair(input.as_bytes(), |parsed_order| order = Some(parsed_order));
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
        let value = parse_with(" calc(1px + var(--gap)) ", Parser::parse_a_component_value)
            .expect("expected a component value");

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
                        value: Some("1".to_string())
                    },
                    OpenTypeTaggedValue {
                        tag: "smcp".to_string(),
                        value_kind: CssOpenTypeTaggedValueKind::On,
                        value: None
                    },
                    OpenTypeTaggedValue {
                        tag: "liga".to_string(),
                        value_kind: CssOpenTypeTaggedValueKind::Off,
                        value: None
                    },
                    OpenTypeTaggedValue {
                        tag: "c2sc".to_string(),
                        value_kind: CssOpenTypeTaggedValueKind::Implicit,
                        value: None
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
                        value: Some("700".to_string())
                    },
                    OpenTypeTaggedValue {
                        tag: "XHGT".to_string(),
                        value_kind: CssOpenTypeTaggedValueKind::Value,
                        value: Some("calc(0.4 + 0.3)".to_string())
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
            parse_font_style("oblique calc(10deg + 1deg)"),
            Some(FontStyle::Oblique { has_angle: true })
        );
    }

    #[test]
    fn rejects_invalid_font_styles() {
        assert_eq!(parse_font_style("normal italic"), None);
        assert_eq!(parse_font_style("italic 10deg"), None);
        assert_eq!(parse_font_style("oblique 10px"), None);
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
        let declaration =
            parse_with("color: red ! important", Parser::parse_a_declaration).expect("expected declaration");

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
}
