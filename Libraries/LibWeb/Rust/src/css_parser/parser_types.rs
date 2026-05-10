/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ComponentValue {
    PreservedToken(Token),
    Function(Function),
    SimpleBlock(SimpleBlock),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Function {
    pub(super) name: String,
    pub(super) value: Vec<ComponentValue>,
    pub(super) name_token: Token,
    pub(super) end_token: Token,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SimpleBlock {
    pub(super) token: Token,
    pub(super) value: Vec<ComponentValue>,
    pub(super) end_token: Token,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectorType {
    Standalone,
    Relative,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectorParsingMode {
    Normal,
    Forgiving,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SelectorSyntax {
    pub(super) compound_selectors: Vec<CompoundSelectorSyntax>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct CompoundSelectorSyntax {
    pub(super) combinator: SelectorCombinator,
    pub(super) simple_selectors: Vec<SimpleSelectorSyntax>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SelectorCombinator {
    None,
    ImmediateChild,
    Descendant,
    NextSibling,
    SubsequentSibling,
    Column,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum SimpleSelectorSyntax {
    Universal(QualifiedNameSyntax),
    TagName(QualifiedNameSyntax),
    Id(String),
    Class(String),
    Attribute(AttributeSelectorSyntax),
    PseudoClass(PseudoClassSelectorSyntax),
    PseudoElement(PseudoElementSelectorSyntax),
    Nesting,
    Invalid(Vec<ComponentValue>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QualifiedNameSyntax {
    pub(super) namespace_type: NamespaceType,
    pub(super) namespace: String,
    pub(super) name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NamespaceType {
    Default,
    None,
    Any,
    Named,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct AttributeSelectorSyntax {
    pub(super) match_type: AttributeMatchType,
    pub(super) qualified_name: QualifiedNameSyntax,
    pub(super) value: String,
    pub(super) case_type: AttributeCaseType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AttributeMatchType {
    HasAttribute,
    ExactValueMatch,
    ContainsWord,
    ContainsString,
    StartsWithSegment,
    StartsWithString,
    EndsWithString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum AttributeCaseType {
    DefaultMatch,
    CaseSensitiveMatch,
    CaseInsensitiveMatch,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PseudoClassSelectorSyntax {
    pub(super) pseudo_class_id: PseudoClassId,
    pub(super) an_plus_b_pattern: Option<ANPlusBPattern>,
    pub(super) is_forgiving: bool,
    pub(super) argument_selector_list: Vec<SelectorSyntax>,
    pub(super) languages: Vec<String>,
    pub(super) ident: Option<String>,
    pub(super) levels: Vec<i64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ANPlusBPattern {
    pub(super) step_size: i32,
    pub(super) offset: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PseudoElementSelectorSyntax {
    pub(super) pseudo_element_id: PseudoElementId,
    pub(super) name: Option<String>,
    pub(super) value: PseudoElementSelectorValue,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum PseudoElementSelectorValue {
    Empty,
    PTNameSelector { is_universal: bool, value: String },
    CompoundSelector(Box<SelectorSyntax>),
    IdentList(Vec<String>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssSelectorEventKind {
    SelectorListStart,
    SelectorListEnd,
    SelectorStart,
    SelectorEnd,
    CompoundSelectorStart,
    CompoundSelectorEnd,
    SimpleSelector,
    PseudoClassSelectorStart,
    PseudoClassSelectorEnd,
    PseudoClassArgumentString,
    PseudoClassArgumentNumber,
    PseudoElementSelectorStart,
    PseudoElementSelectorEnd,
    PseudoElementArgumentString,
    InvalidSelectorStart,
    InvalidSelectorEnd,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssSimpleSelectorKind {
    Universal,
    TagName,
    Id,
    Class,
    Attribute,
    Nesting,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssSelectorCombinator {
    None,
    ImmediateChild,
    Descendant,
    NextSibling,
    SubsequentSibling,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssSelectorNamespaceType {
    Default,
    None,
    Any,
    Named,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssAttributeMatchType {
    HasAttribute,
    ExactValueMatch,
    ContainsWord,
    ContainsString,
    StartsWithSegment,
    StartsWithString,
    EndsWithString,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::enum_variant_names)]
#[repr(C)]
pub enum CssAttributeCaseType {
    DefaultMatch,
    CaseSensitiveMatch,
    CaseInsensitiveMatch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPseudoElementValueKind {
    Empty,
    PTNameSelector,
    CompoundSelector,
    IdentList,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssSelectorEvent {
    pub kind: CssSelectorEventKind,
    pub combinator: CssSelectorCombinator,
    pub simple_selector_kind: CssSimpleSelectorKind,
    pub namespace_type: CssSelectorNamespaceType,
    pub attribute_match_type: CssAttributeMatchType,
    pub attribute_case_type: CssAttributeCaseType,
    pub pseudo_element_value_kind: CssPseudoElementValueKind,
    pub pseudo_class_id: u8,
    pub pseudo_element_id: u8,
    pub has_an_plus_b_pattern: bool,
    pub an_plus_b_step_size: i32,
    pub an_plus_b_offset: i32,
    pub argument_number: i64,
    pub is_forgiving: bool,
    pub is_universal: bool,
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub namespace_ptr: *const u8,
    pub namespace_len: usize,
    pub value_ptr: *const u8,
    pub value_len: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssSelectorNamespace {
    pub prefix_ptr: *const u8,
    pub prefix_len: usize,
}

impl CssSelectorEvent {
    pub(super) fn new(kind: CssSelectorEventKind) -> Self {
        Self {
            kind,
            combinator: CssSelectorCombinator::None,
            simple_selector_kind: CssSimpleSelectorKind::Universal,
            namespace_type: CssSelectorNamespaceType::Default,
            attribute_match_type: CssAttributeMatchType::HasAttribute,
            attribute_case_type: CssAttributeCaseType::DefaultMatch,
            pseudo_element_value_kind: CssPseudoElementValueKind::Empty,
            pseudo_class_id: 0,
            pseudo_element_id: 0,
            has_an_plus_b_pattern: false,
            an_plus_b_step_size: 0,
            an_plus_b_offset: 0,
            argument_number: 0,
            is_forgiving: false,
            is_universal: false,
            name_ptr: std::ptr::null(),
            name_len: 0,
            namespace_ptr: std::ptr::null(),
            namespace_len: 0,
            value_ptr: std::ptr::null(),
            value_len: 0,
        }
    }
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
    pub(super) name: String,
    pub(super) prelude: Vec<ComponentValue>,
    pub(super) child_rules_and_lists_of_declarations: Vec<RuleOrListOfDeclarations>,
    pub(super) is_block_rule: bool,
}

// https://drafts.csswg.org/css-syntax/#qualified-rule
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct QualifiedRule {
    pub(super) prelude: Vec<ComponentValue>,
    pub(super) declarations: Vec<Declaration>,
    pub(super) child_rules: Vec<RuleOrListOfDeclarations>,
}

// https://drafts.csswg.org/css-syntax/#declaration
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Declaration {
    pub(super) name: String,
    pub(super) value: Vec<ComponentValue>,
    pub(super) important: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct UrlFunction {
    pub(super) function_type: CssUrlFunctionType,
    pub(super) url: String,
    pub(super) request_url_modifiers: Vec<UrlModifier>,
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum UrlModifier {
    CrossOrigin(CssUrlCrossOriginModifierValue),
    Integrity(String),
    ReferrerPolicy(CssUrlReferrerPolicyModifierValue),
}

impl UrlModifier {
    pub(super) fn kind(&self) -> CssUrlModifierKind {
        match self {
            UrlModifier::CrossOrigin(_) => CssUrlModifierKind::CrossOrigin,
            UrlModifier::Integrity(_) => CssUrlModifierKind::Integrity,
            UrlModifier::ReferrerPolicy(_) => CssUrlModifierKind::ReferrerPolicy,
        }
    }

    pub(crate) fn as_ffi(&self) -> CssUrlModifier {
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
pub(crate) enum FontSource {
    Local(FamilyName),
    Url {
        url_function: UrlFunction,
        format: Option<String>,
        tech: Vec<CssFontTech>,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum FontLanguageOverride {
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
    pub(crate) value_component_values: Vec<ComponentValue>,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) enum OpenTypeSettings {
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
    pub(super) kind: CssFontVariantAlternatesValueKind,
    pub(super) feature_value_names: Vec<String>,
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
    pub(super) ligatures_none: bool,
    pub(super) alternates: Option<Vec<FontVariantAlternatesValue>>,
    pub(super) caps: Option<String>,
    pub(super) east_asian: Option<Vec<FontVariantEastAsianValue>>,
    pub(super) emoji: Option<String>,
    pub(super) ligatures: Option<Vec<FontVariantLigaturesValue>>,
    pub(super) numeric: Option<Vec<FontVariantNumericValue>>,
    pub(super) position: Option<String>,
}

impl FontVariant {
    pub(super) fn has_any_value(&self) -> bool {
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
    pub(super) name: Option<String>,
    pub(super) pseudo_classes: Vec<CssPagePseudoClassKind>,
}

pub(crate) type KeyframeSelector = f64;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct MediaFeatureTest {
    pub(super) component_value: ComponentValue,
    pub(super) kind: MediaFeatureSyntax,
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
    pub(super) kind: MediaFeatureNameKind,
    pub(super) id: MediaFeatureId,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MediaFeatureNameKind {
    Normal,
    Min,
    Max,
}

pub(super) struct ComponentValueParser {
    pub(super) component_values: Vec<ComponentValue>,
    pub(super) index: usize,
    pub(super) boolean_expression: Option<BooleanExpression>,
    pub(super) declared_namespaces: Vec<String>,
    pub(super) pseudo_class_context: Vec<PseudoClassId>,
}

#[derive(Clone, Copy)]
pub(super) enum BooleanExpressionTestKind {
    SupportsFeature,
    MediaFeature,
    IfTest,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum AllowWildcardName {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Nested {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RuleContext {
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
    pub(super) tokens: Vec<Token>,
    pub(super) index: usize,
    pub(super) rule_context: Vec<RuleContext>,
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
pub enum CssPageSizeDescriptorKind {
    Auto,
    Lengths,
    PageSizeAndOrientation,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPageSizeKeyword {
    A5,
    A4,
    A3,
    B5,
    B4,
    JisB5,
    JisB4,
    Letter,
    Legal,
    Ledger,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPageSizeOrientation {
    Portrait,
    Landscape,
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
pub enum CssDescriptorResultKind {
    CounterStyleAdditiveSymbols,
    CounterStyleNegative,
    CounterStyleSystemCyclic,
    CounterStyleSystemNumeric,
    CounterStyleSystemAlphabetic,
    CounterStyleSystemSymbolic,
    CounterStyleSystemAdditive,
    CounterStyleSystemFixed,
    CounterStyleSystemFixedWithInteger,
    CounterStyleSystemExtends,
    CounterStyleName,
    CounterStylePad,
    CounterStyleRangeAuto,
    CounterStyleRangeList,
    Crop,
    Cross,
    CropAndCross,
    FamilyName,
    FontSrcList,
    FontWeightAbsolutePair,
    Length,
    OptionalDeclarationValue,
    PageSizeAuto,
    PageSizeLengths,
    PageSizeAndOrientation,
    PositivePercentage,
    String,
    Symbol,
    Symbols,
    UnicodeRangeTokens,
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
pub enum CssScrollFunctionValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssScrollFunctionScrollerKind {
    None,
    Nearest,
    Root,
    Self_,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssScrollFunctionAxisKind {
    None,
    Block,
    Inline,
    X,
    Y,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssScrollFunctionValue {
    pub kind: CssScrollFunctionValueKind,
    pub scroller: CssScrollFunctionScrollerKind,
    pub axis: CssScrollFunctionAxisKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssViewTimelineInsetValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssViewTimelineInsetValue {
    pub kind: CssViewTimelineInsetValueKind,
    pub count: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssViewFunctionValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssViewFunctionInsetKind {
    None,
    Default,
    NonDefault,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssViewFunctionInsetPosition {
    None,
    BeforeAxis,
    AfterAxis,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct CssViewFunctionValue {
    pub kind: CssViewFunctionValueKind,
    pub axis: CssScrollFunctionAxisKind,
    pub inset: CssViewFunctionInsetKind,
    pub inset_position: CssViewFunctionInsetPosition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssRectValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssRatioValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(C)]
pub struct CssRatioValue {
    pub kind: CssRatioValueKind,
    pub has_denominator: bool,
    pub numerator: f64,
    pub denominator: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPrimitiveValueType {
    Integer,
    Number,
    Percentage,
    Angle,
    Flex,
    Frequency,
    Length,
    Resolution,
    String,
    Time,
    Opacity,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPrimitiveValueKind {
    Invalid,
    Integer,
    Number,
    Percentage,
    Angle,
    Flex,
    Frequency,
    Length,
    Resolution,
    String,
    Time,
    Opacity,
    Keyword,
    CustomIdent,
    Ratio,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssCalculationNodeKind {
    Numeric,
    Function,
    Sum,
    Product,
    Negate,
    Invert,
    TreeCountingFunction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(C)]
pub struct CssPrimitiveValueOptions {
    pub allow_quirky_length: bool,
    pub allow_quirky_color: bool,
    pub allow_svg_unitless_length: bool,
    pub allow_svg_unitless_angle: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssEasingValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTransformFunctionValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssFitContentValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssBasicShapeValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssGridAutoFlowValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssGridAutoFlowAxis {
    Row,
    Column,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssGridAutoFlowDense {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssGridTrackPlacementValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssGridTrackSizeListValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssDisplayValueKind {
    Invalid,
    Box,
    Internal,
    OutsideAndInside,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssDisplayBox {
    Contents,
    None,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssDisplayInside {
    Flow,
    FlowRoot,
    Table,
    Flex,
    Grid,
    Ruby,
    Math,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssDisplayInternal {
    TableRowGroup,
    TableHeaderGroup,
    TableFooterGroup,
    TableRow,
    TableCell,
    TableColumnGroup,
    TableColumn,
    TableCaption,
    RubyBase,
    RubyText,
    RubyBaseContainer,
    RubyTextContainer,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssDisplayOutside {
    Block,
    Inline,
    RunIn,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssDisplayListItem {
    No,
    Yes,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssTransformLonghandValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssPositionValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssBackgroundSizeValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssRepeatStyleValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssRepeatStyleRepetition {
    NoRepeat,
    Repeat,
    Round,
    Space,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssColorFunctionValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssColorValueKind {
    Invalid,
    Valid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssParsedColorKind {
    Invalid,
    Rgba,
    Keyword,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssImageSetValueKind {
    Invalid,
    Valid,
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
    LayerName,
    KeyframesName,
    KeyframeSelector,
    NamespacePrefix,
    NamespaceUri,
    CustomPropertyName,
    CounterStyleName,
    PageSelectorList,
    PageSelectorStart,
    PageSelectorEnd,
    PagePseudoClass,
    FontFeatureValuesFamilyName,
    ContainerCondition,
}

#[repr(C)]
pub struct CssRuleEvent {
    pub kind: CssRuleEventKind,
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub value_ptr: *const u8,
    pub value_len: usize,
    pub keyframe_selector: f64,
    pub page_pseudo_class: CssPagePseudoClassKind,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssMediaFeatureValuePayloadKind {
    None,
    Ident,
    Integer,
    Length,
    Ratio,
    Resolution,
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
    pub payload_kind: CssMediaFeatureValuePayloadKind,
    pub numeric_value: f64,
    pub secondary_numeric_value: f64,
    pub unit_or_ident_ptr: *const u8,
    pub unit_or_ident_len: usize,
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
pub enum CssGeneratedPropertyValueKind {
    Invalid,
    Keyword,
    CustomIdent,
    ValueType,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub enum CssStyleValueKind {
    Invalid,
    Keyword,
    CustomIdent,
    Primitive,
    Color,
    Url,
    CounterStyleName,
    CounterStyle,
    EasingFunction,
    FitContent,
    Image,
    ColorFunction,
    FontFamily,
    FontFeatureSettings,
    FontLanguageOverride,
    FontStyle,
    FontVariant,
    FontVariantAlternates,
    FontVariantEastAsian,
    FontVariantLigatures,
    FontVariantNumeric,
    FontVariationSettings,
    FilterValueList,
    KeywordList,
    BasicShape,
    Rect,
    AspectRatio,
    AnimationName,
    Anchor,
    AnchorSize,
    AnchorNameOrScope,
    BackgroundSize,
    Border,
    BorderImage,
    BorderImageOutset,
    BorderImageRepeat,
    BorderImageSlice,
    BorderImageWidth,
    ColorScheme,
    Contain,
    ContainerType,
    CornerShape,
    Counter,
    CounterDefinitions,
    BorderRadius,
    Columns,
    CoordinatingValueListShorthand,
    Content,
    Cursor,
    Display,
    Flex,
    FlexFlow,
    GridAutoFlow,
    GridAutoTrackSizes,
    GridTemplateAreas,
    GridTrackPlacement,
    GridTrackSizeList,
    LayerShorthand,
    ListStyle,
    MathDepth,
    Paint,
    PaintOrder,
    PlaceContent,
    PlaceItems,
    PlaceSelf,
    PositionalValueListShorthand,
    Position,
    PositionArea,
    PositionAnchor,
    PositionTryFallbacks,
    PositionTryOrder,
    PositionVisibility,
    Quotes,
    RepeatStyle,
    OverflowClipMargin,
    Shadow,
    ShapeOutside,
    TextDecoration,
    TextDecorationLine,
    ScrollFunction,
    ScrollbarColor,
    ScrollbarGutter,
    StrokeDasharray,
    ScrollTimeline,
    TimelineName,
    TimelineScope,
    TextWrap,
    TextWrapMode,
    TextWrapStyle,
    TextIndent,
    TextUnderlinePosition,
    TouchAction,
    Transformation,
    TransformLonghand,
    TransformOrigin,
    TransitionBehavior,
    TransitionProperty,
    ViewTimelineInset,
    ViewTimeline,
    ViewFunction,
    ViewTransitionName,
    WhiteSpace,
    WhiteSpaceTrim,
    WillChange,
    MathFunction,
    TreeCountingFunction,
    BorderSpacing,
    FontShorthand,
    ComponentShorthand,
    GridPlacementShorthand,
    GridTemplateShorthand,
    GeneratedValueList,
    Gradient,
}
