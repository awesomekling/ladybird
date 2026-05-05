/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

// The Rust parser is being staged bottom-up. Keep this module warning-free until
// the C++ bridge starts calling into it.
#![allow(dead_code)]

use crate::css_tokenizer::{CssToken, Token, TokenType};
use crate::generated_media_features::{MediaFeatureId, media_feature_id_from_string, media_feature_type_is_range};

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

#[repr(C)]
pub struct CssComponentValue {
    pub kind: CssComponentValueKind,
    pub token: CssToken,
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
    pub component_value: CssComponentValue,
}

#[repr(C)]
pub struct CssMediaQuery {
    pub is_valid: bool,
    pub is_negated: bool,
    pub has_media_condition: bool,
    pub media_type_ptr: *const u8,
    pub media_type_len: usize,
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

pub(crate) fn parse_a_media_feature<M, V>(
    filtered_input: &[u8],
    mut media_feature_callback: M,
    mut media_feature_value_callback: V,
) where
    M: FnMut(CssMediaFeature),
    V: FnMut(CssMediaFeatureValue),
{
    let (mut parser, filtered_input_string) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let Some(media_feature) = component_values_parse_as_media_feature(&component_values) else {
        return;
    };

    media_feature_callback(css_media_feature_from_syntax(&media_feature));
    emit_media_feature_values(&media_feature, filtered_input_string, &mut media_feature_value_callback);
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
    for media_query in parser.parse_a_media_query_list() {
        match media_query {
            MediaQuerySyntax::Invalid => {
                media_query_callback(CssMediaQuery {
                    is_valid: false,
                    is_negated: false,
                    has_media_condition: false,
                    media_type_ptr: std::ptr::null(),
                    media_type_len: 0,
                });
            }
            MediaQuerySyntax::Valid {
                modifier,
                media_type,
                condition,
            } => {
                media_query_callback(CssMediaQuery {
                    is_valid: true,
                    is_negated: modifier == MediaQueryModifier::Not,
                    has_media_condition: condition.is_some(),
                    media_type_ptr: media_type
                        .as_ref()
                        .map_or(std::ptr::null(), |media_type| media_type.as_ptr()),
                    media_type_len: media_type.as_ref().map_or(0, String::len),
                });
                if let Some(condition) = condition {
                    emit_boolean_expression(
                        &condition,
                        filtered_input_string,
                        &mut event_callback,
                        &mut component_value_callback,
                        &mut media_feature_callback,
                        &mut media_feature_value_callback,
                    );
                }
            }
        }
    }
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
    component_values: &[ComponentValue],
    filtered_input: &str,
    callback: &mut C,
) where
    C: FnMut(CssMediaFeatureValue),
{
    for component_value in component_values {
        emit_component_value(component_value, filtered_input, &mut |component_value| {
            callback(CssMediaFeatureValue { kind, component_value });
        });
    }
}

fn emit_media_feature_values<C>(syntax: &MediaFeatureSyntax, filtered_input: &str, callback: &mut C)
where
    C: FnMut(CssMediaFeatureValue),
{
    match syntax {
        MediaFeatureSyntax::Boolean(_) => {}
        MediaFeatureSyntax::Plain { value, .. }
        | MediaFeatureSyntax::HalfRangeNameFirst { value, .. }
        | MediaFeatureSyntax::HalfRangeValueFirst { value, .. } => {
            emit_media_feature_value(CssMediaFeatureValueKind::Value, value, filtered_input, callback);
        }
        MediaFeatureSyntax::Range {
            left_value,
            right_value,
            ..
        } => {
            emit_media_feature_value(
                CssMediaFeatureValueKind::LeftValue,
                left_value,
                filtered_input,
                callback,
            );
            emit_media_feature_value(
                CssMediaFeatureValueKind::RightValue,
                right_value,
                filtered_input,
                callback,
            );
        }
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
        }
    }

    // https://drafts.csswg.org/css-conditional-5/#typedef-supports-feature
    fn parse_supports_feature(&mut self) -> Option<BooleanExpressionTest> {
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
            return Some(BooleanExpressionTest::SupportsFeature(vec![component_value]));
        }

        let ComponentValue::Function(function) = &component_value else {
            return None;
        };

        // `<supports-selector-fn> = selector( <complex-selector> )`
        if function.name.eq_ignore_ascii_case("selector") {
            self.index += 1;
            return Some(BooleanExpressionTest::SupportsFeature(vec![component_value]));
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
            if matches!(
                ident,
                Some(ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { .. },
                    ..
                }))
            ) && parser.next_component_value().is_none()
            {
                self.index += 1;
                return Some(BooleanExpressionTest::SupportsFeature(vec![component_value]));
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

        if let Some(name) = component_values_parse_as_mf_name(left, AllowMinMaxPrefix::No)
            && component_values_parse_as_mf_value(right)
        {
            return Some(MediaFeatureSyntax::HalfRangeNameFirst {
                name,
                comparison,
                value: right.to_vec(),
            });
        }

        if component_values_parse_as_mf_value(left)
            && let Some(name) = component_values_parse_as_mf_name(right, AllowMinMaxPrefix::No)
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
        if component_values_parse_as_mf_value(left_value)
            && let Some(name) = component_values_parse_as_mf_name(name, AllowMinMaxPrefix::No)
            && component_values_parse_as_mf_value(right_value)
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

fn component_values_parse_as_mf_value(component_values: &[ComponentValue]) -> bool {
    !component_values.is_empty() && component_values.iter().all(is_media_feature_value_component_value)
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

fn strip_whitespace(component_values: &[ComponentValue]) -> &[ComponentValue] {
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

impl Parser {
    pub(crate) fn new(tokens: Vec<Token>) -> Self {
        Self {
            tokens,
            index: 0,
            rule_context: Vec::new(),
        }
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
        self.parse_a_comma_separated_list_of_component_values()
            .into_iter()
            .map(component_values_parse_as_media_query)
            .collect()
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
        BooleanExpression, BooleanExpressionTestKind, ComponentValue, ComponentValueParser, MediaFeatureNameKind,
        MediaFeatureSyntax, MediaQueryModifier, MediaQuerySyntax, MfComparison, Parser, Rule, RuleContext,
        RuleOrListOfDeclarations, component_values_parse_as_media_feature, strip_whitespace,
    };
    use crate::css_tokenizer::{self, TokenType};
    use crate::generated_media_features::MediaFeatureId;

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
        assert!(parse_media_feature_syntax("100px <= width >= 200px").is_none());
        assert!(parse_media_feature_syntax("100px = width = 200px").is_none());
        assert!(parse_media_feature_syntax("width <> 100px").is_none());
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
