/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(super) fn emit_component_value<F>(component_value: &ComponentValue, filtered_input: &str, callback: &mut F)
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

pub(super) fn emit_token<F>(kind: CssComponentValueKind, token: &Token, filtered_input: &str, callback: &mut F)
where
    F: FnMut(CssComponentValue),
{
    callback(CssComponentValue {
        kind,
        token: token.as_ffi(filtered_input),
    });
}

pub(super) fn emit_selector_list<E, C>(
    selectors: &[SelectorSyntax],
    filtered_input: &str,
    event_callback: &mut E,
    component_value_callback: &mut C,
) where
    E: FnMut(CssSelectorEvent),
    C: FnMut(CssComponentValue),
{
    event_callback(CssSelectorEvent::new(CssSelectorEventKind::SelectorListStart));
    for selector in selectors {
        event_callback(CssSelectorEvent::new(CssSelectorEventKind::SelectorStart));
        for compound_selector in &selector.compound_selectors {
            let mut event = CssSelectorEvent::new(CssSelectorEventKind::CompoundSelectorStart);
            event.combinator = selector_combinator_to_ffi(compound_selector.combinator);
            event_callback(event);

            for simple_selector in &compound_selector.simple_selectors {
                emit_simple_selector(
                    simple_selector,
                    filtered_input,
                    event_callback,
                    component_value_callback,
                );
            }

            event_callback(CssSelectorEvent::new(CssSelectorEventKind::CompoundSelectorEnd));
        }
        event_callback(CssSelectorEvent::new(CssSelectorEventKind::SelectorEnd));
    }
    event_callback(CssSelectorEvent::new(CssSelectorEventKind::SelectorListEnd));
}

pub(super) fn emit_simple_selector<E, C>(
    simple_selector: &SimpleSelectorSyntax,
    filtered_input: &str,
    event_callback: &mut E,
    component_value_callback: &mut C,
) where
    E: FnMut(CssSelectorEvent),
    C: FnMut(CssComponentValue),
{
    match simple_selector {
        SimpleSelectorSyntax::Universal(qualified_name) => {
            emit_qualified_name_simple_selector(CssSimpleSelectorKind::Universal, qualified_name, event_callback);
        }
        SimpleSelectorSyntax::TagName(qualified_name) => {
            emit_qualified_name_simple_selector(CssSimpleSelectorKind::TagName, qualified_name, event_callback);
        }
        SimpleSelectorSyntax::Id(name) => emit_name_simple_selector(CssSimpleSelectorKind::Id, name, event_callback),
        SimpleSelectorSyntax::Class(name) => {
            emit_name_simple_selector(CssSimpleSelectorKind::Class, name, event_callback);
        }
        SimpleSelectorSyntax::Attribute(attribute) => {
            let mut event = selector_event_with_qualified_name(
                CssSelectorEventKind::SimpleSelector,
                CssSimpleSelectorKind::Attribute,
                &attribute.qualified_name,
            );
            event.attribute_match_type = attribute_match_type_to_ffi(attribute.match_type);
            event.attribute_case_type = attribute_case_type_to_ffi(attribute.case_type);
            (event.value_ptr, event.value_len) = string_parts(&attribute.value);
            event_callback(event);
        }
        SimpleSelectorSyntax::PseudoClass(pseudo_class) => {
            let mut event = CssSelectorEvent::new(CssSelectorEventKind::PseudoClassSelectorStart);
            event.pseudo_class_id = pseudo_class.pseudo_class_id as u8;
            if let Some(an_plus_b_pattern) = pseudo_class.an_plus_b_pattern {
                event.has_an_plus_b_pattern = true;
                event.an_plus_b_step_size = an_plus_b_pattern.step_size;
                event.an_plus_b_offset = an_plus_b_pattern.offset;
            }
            event.is_forgiving = pseudo_class.is_forgiving;
            event_callback(event);

            for language in &pseudo_class.languages {
                let mut event = CssSelectorEvent::new(CssSelectorEventKind::PseudoClassArgumentString);
                (event.value_ptr, event.value_len) = string_parts(language);
                event_callback(event);
            }
            if let Some(ident) = &pseudo_class.ident {
                let mut event = CssSelectorEvent::new(CssSelectorEventKind::PseudoClassArgumentString);
                (event.value_ptr, event.value_len) = string_parts(ident);
                event_callback(event);
            }
            for level in &pseudo_class.levels {
                let mut event = CssSelectorEvent::new(CssSelectorEventKind::PseudoClassArgumentNumber);
                event.argument_number = *level;
                event_callback(event);
            }
            emit_selector_list(
                &pseudo_class.argument_selector_list,
                filtered_input,
                event_callback,
                component_value_callback,
            );
            event_callback(CssSelectorEvent::new(CssSelectorEventKind::PseudoClassSelectorEnd));
        }
        SimpleSelectorSyntax::PseudoElement(pseudo_element) => {
            let mut event = CssSelectorEvent::new(CssSelectorEventKind::PseudoElementSelectorStart);
            event.pseudo_element_id = pseudo_element.pseudo_element_id as u8;
            if let Some(name) = &pseudo_element.name {
                (event.name_ptr, event.name_len) = string_parts(name);
            }
            event.pseudo_element_value_kind = pseudo_element_value_kind_to_ffi(&pseudo_element.value);
            if let PseudoElementSelectorValue::PTNameSelector { is_universal, value } = &pseudo_element.value {
                event.is_universal = *is_universal;
                (event.value_ptr, event.value_len) = string_parts(value);
            }
            event_callback(event);

            match &pseudo_element.value {
                PseudoElementSelectorValue::CompoundSelector(selector) => {
                    emit_selector_list(
                        std::slice::from_ref(selector),
                        filtered_input,
                        event_callback,
                        component_value_callback,
                    );
                }
                PseudoElementSelectorValue::IdentList(idents) => {
                    for ident in idents {
                        let mut event = CssSelectorEvent::new(CssSelectorEventKind::PseudoElementArgumentString);
                        (event.value_ptr, event.value_len) = string_parts(ident);
                        event_callback(event);
                    }
                }
                PseudoElementSelectorValue::Empty | PseudoElementSelectorValue::PTNameSelector { .. } => {}
            }

            event_callback(CssSelectorEvent::new(CssSelectorEventKind::PseudoElementSelectorEnd));
        }
        SimpleSelectorSyntax::Nesting => {
            event_callback(CssSelectorEvent {
                simple_selector_kind: CssSimpleSelectorKind::Nesting,
                ..CssSelectorEvent::new(CssSelectorEventKind::SimpleSelector)
            });
        }
        SimpleSelectorSyntax::Invalid(component_values) => {
            event_callback(CssSelectorEvent::new(CssSelectorEventKind::InvalidSelectorStart));
            for component_value in component_values {
                emit_component_value(component_value, filtered_input, component_value_callback);
            }
            event_callback(CssSelectorEvent::new(CssSelectorEventKind::InvalidSelectorEnd));
        }
    }
}

pub(super) fn emit_qualified_name_simple_selector<E>(
    simple_selector_kind: CssSimpleSelectorKind,
    qualified_name: &QualifiedNameSyntax,
    event_callback: &mut E,
) where
    E: FnMut(CssSelectorEvent),
{
    event_callback(selector_event_with_qualified_name(
        CssSelectorEventKind::SimpleSelector,
        simple_selector_kind,
        qualified_name,
    ));
}

pub(super) fn emit_name_simple_selector<E>(
    simple_selector_kind: CssSimpleSelectorKind,
    name: &str,
    event_callback: &mut E,
) where
    E: FnMut(CssSelectorEvent),
{
    let mut event = CssSelectorEvent::new(CssSelectorEventKind::SimpleSelector);
    event.simple_selector_kind = simple_selector_kind;
    (event.name_ptr, event.name_len) = string_parts(name);
    event_callback(event);
}

pub(super) fn selector_event_with_qualified_name(
    event_kind: CssSelectorEventKind,
    simple_selector_kind: CssSimpleSelectorKind,
    qualified_name: &QualifiedNameSyntax,
) -> CssSelectorEvent {
    let mut event = CssSelectorEvent::new(event_kind);
    event.simple_selector_kind = simple_selector_kind;
    event.namespace_type = namespace_type_to_ffi(qualified_name.namespace_type);
    (event.namespace_ptr, event.namespace_len) = string_parts(&qualified_name.namespace);
    (event.name_ptr, event.name_len) = string_parts(&qualified_name.name);
    event
}

pub(super) fn selector_combinator_to_ffi(combinator: SelectorCombinator) -> CssSelectorCombinator {
    match combinator {
        SelectorCombinator::None => CssSelectorCombinator::None,
        SelectorCombinator::ImmediateChild => CssSelectorCombinator::ImmediateChild,
        SelectorCombinator::Descendant => CssSelectorCombinator::Descendant,
        SelectorCombinator::NextSibling => CssSelectorCombinator::NextSibling,
        SelectorCombinator::SubsequentSibling => CssSelectorCombinator::SubsequentSibling,
        SelectorCombinator::Column => CssSelectorCombinator::Column,
    }
}

pub(super) fn namespace_type_to_ffi(namespace_type: NamespaceType) -> CssSelectorNamespaceType {
    match namespace_type {
        NamespaceType::Default => CssSelectorNamespaceType::Default,
        NamespaceType::None => CssSelectorNamespaceType::None,
        NamespaceType::Any => CssSelectorNamespaceType::Any,
        NamespaceType::Named => CssSelectorNamespaceType::Named,
    }
}

pub(super) fn attribute_match_type_to_ffi(match_type: AttributeMatchType) -> CssAttributeMatchType {
    match match_type {
        AttributeMatchType::HasAttribute => CssAttributeMatchType::HasAttribute,
        AttributeMatchType::ExactValueMatch => CssAttributeMatchType::ExactValueMatch,
        AttributeMatchType::ContainsWord => CssAttributeMatchType::ContainsWord,
        AttributeMatchType::ContainsString => CssAttributeMatchType::ContainsString,
        AttributeMatchType::StartsWithSegment => CssAttributeMatchType::StartsWithSegment,
        AttributeMatchType::StartsWithString => CssAttributeMatchType::StartsWithString,
        AttributeMatchType::EndsWithString => CssAttributeMatchType::EndsWithString,
    }
}

pub(super) fn attribute_case_type_to_ffi(case_type: AttributeCaseType) -> CssAttributeCaseType {
    match case_type {
        AttributeCaseType::DefaultMatch => CssAttributeCaseType::DefaultMatch,
        AttributeCaseType::CaseSensitiveMatch => CssAttributeCaseType::CaseSensitiveMatch,
        AttributeCaseType::CaseInsensitiveMatch => CssAttributeCaseType::CaseInsensitiveMatch,
    }
}

pub(super) fn pseudo_element_value_kind_to_ffi(value: &PseudoElementSelectorValue) -> CssPseudoElementValueKind {
    match value {
        PseudoElementSelectorValue::Empty => CssPseudoElementValueKind::Empty,
        PseudoElementSelectorValue::PTNameSelector { .. } => CssPseudoElementValueKind::PTNameSelector,
        PseudoElementSelectorValue::CompoundSelector(_) => CssPseudoElementValueKind::CompoundSelector,
        PseudoElementSelectorValue::IdentList(_) => CssPseudoElementValueKind::IdentList,
    }
}

pub(super) fn emit_syntax_node<C>(syntax_node: &SyntaxNode, callback: &mut C)
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

pub(super) fn emit_rule<E, C>(
    rule: &Rule,
    filtered_input: &str,
    event_callback: &mut E,
    component_value_callback: &mut C,
) where
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
                value_ptr: std::ptr::null(),
                value_len: 0,
                keyframe_selector: 0.0,
                page_pseudo_class: CssPagePseudoClassKind::Left,
                important: false,
                is_block_rule: at_rule.is_block_rule,
            });
            if at_rule.name.eq_ignore_ascii_case("layer") {
                let mut parser = ComponentValueParser::new(at_rule.prelude.clone());
                let parsed_names = if at_rule.is_block_rule {
                    parser.parse_a_layer_name(true).and_then(|name| {
                        parser.discard_whitespace();
                        if parser.has_next_component_value() {
                            return None;
                        }
                        Some(vec![name])
                    })
                } else {
                    parser.parse_a_layer_name_list()
                };
                if let Some(names) = parsed_names {
                    for name in names {
                        let (name_ptr, name_len) = string_parts(&name);
                        event_callback(CssRuleEvent {
                            kind: CssRuleEventKind::LayerName,
                            name_ptr,
                            name_len,
                            value_ptr: std::ptr::null(),
                            value_len: 0,
                            keyframe_selector: 0.0,
                            page_pseudo_class: CssPagePseudoClassKind::Left,
                            important: false,
                            is_block_rule: false,
                        });
                    }
                }
            }
            if (at_rule.name.eq_ignore_ascii_case("keyframes")
                || at_rule.name.eq_ignore_ascii_case("-webkit-keyframes"))
                && let Some(name) = ComponentValueParser::new(at_rule.prelude.clone()).parse_a_keyframes_name()
            {
                let (name_ptr, name_len) = string_parts(&name);
                event_callback(CssRuleEvent {
                    kind: CssRuleEventKind::KeyframesName,
                    name_ptr,
                    name_len,
                    value_ptr: std::ptr::null(),
                    value_len: 0,
                    keyframe_selector: 0.0,
                    page_pseudo_class: CssPagePseudoClassKind::Left,
                    important: false,
                    is_block_rule: false,
                });
            }
            if at_rule.name.eq_ignore_ascii_case("namespace")
                && let Some((prefix, namespace_uri)) =
                    ComponentValueParser::new(at_rule.prelude.clone()).parse_a_namespace_rule_prelude()
            {
                if let Some(prefix) = prefix {
                    let (name_ptr, name_len) = string_parts(&prefix);
                    event_callback(CssRuleEvent {
                        kind: CssRuleEventKind::NamespacePrefix,
                        name_ptr,
                        name_len,
                        value_ptr: std::ptr::null(),
                        value_len: 0,
                        keyframe_selector: 0.0,
                        page_pseudo_class: CssPagePseudoClassKind::Left,
                        important: false,
                        is_block_rule: false,
                    });
                }
                let (name_ptr, name_len) = string_parts(&namespace_uri);
                event_callback(CssRuleEvent {
                    kind: CssRuleEventKind::NamespaceUri,
                    name_ptr,
                    name_len,
                    value_ptr: std::ptr::null(),
                    value_len: 0,
                    keyframe_selector: 0.0,
                    page_pseudo_class: CssPagePseudoClassKind::Left,
                    important: false,
                    is_block_rule: false,
                });
            }
            if at_rule.name.eq_ignore_ascii_case("property")
                && let Some(name) = ComponentValueParser::new(at_rule.prelude.clone()).parse_a_custom_property_name()
            {
                let (name_ptr, name_len) = string_parts(&name);
                event_callback(CssRuleEvent {
                    kind: CssRuleEventKind::CustomPropertyName,
                    name_ptr,
                    name_len,
                    value_ptr: std::ptr::null(),
                    value_len: 0,
                    keyframe_selector: 0.0,
                    page_pseudo_class: CssPagePseudoClassKind::Left,
                    important: false,
                    is_block_rule: false,
                });
            }
            if at_rule.name.eq_ignore_ascii_case("counter-style")
                && let Some(name) = ComponentValueParser::new(at_rule.prelude.clone()).parse_a_counter_style_name()
            {
                let (name_ptr, name_len) = string_parts(&name);
                event_callback(CssRuleEvent {
                    kind: CssRuleEventKind::CounterStyleName,
                    name_ptr,
                    name_len,
                    value_ptr: std::ptr::null(),
                    value_len: 0,
                    keyframe_selector: 0.0,
                    page_pseudo_class: CssPagePseudoClassKind::Left,
                    important: false,
                    is_block_rule: false,
                });
            }
            if at_rule.name.eq_ignore_ascii_case("page")
                && let Some(selectors) = ComponentValueParser::new(at_rule.prelude.clone()).parse_a_page_selector_list()
            {
                event_callback(CssRuleEvent::new(CssRuleEventKind::PageSelectorList));
                for selector in selectors {
                    let (name_ptr, name_len) = selector
                        .name
                        .as_ref()
                        .map_or((std::ptr::null(), 0), |name| string_parts(name));
                    event_callback(CssRuleEvent {
                        kind: CssRuleEventKind::PageSelectorStart,
                        name_ptr,
                        name_len,
                        value_ptr: std::ptr::null(),
                        value_len: 0,
                        keyframe_selector: 0.0,
                        page_pseudo_class: CssPagePseudoClassKind::Left,
                        important: false,
                        is_block_rule: false,
                    });
                    for pseudo_class in selector.pseudo_classes {
                        event_callback(CssRuleEvent {
                            kind: CssRuleEventKind::PagePseudoClass,
                            name_ptr: std::ptr::null(),
                            name_len: 0,
                            value_ptr: std::ptr::null(),
                            value_len: 0,
                            keyframe_selector: 0.0,
                            page_pseudo_class: pseudo_class,
                            important: false,
                            is_block_rule: false,
                        });
                    }
                    event_callback(CssRuleEvent::new(CssRuleEventKind::PageSelectorEnd));
                }
            }
            if at_rule.name.eq_ignore_ascii_case("font-feature-values") {
                let family_names: Option<Vec<_>> = {
                    let groups = split_component_values_on_comma(&at_rule.prelude);
                    if groups.is_empty() {
                        None
                    } else {
                        let mut family_names = Vec::with_capacity(groups.len());
                        for group in groups {
                            let mut parser = ComponentValueParser::new(group.to_vec());
                            let Some(family_name) = parser.parse_a_family_name() else {
                                family_names.clear();
                                break;
                            };
                            parser.discard_whitespace();
                            if parser.has_next_component_value() {
                                family_names.clear();
                                break;
                            }
                            family_names.push(family_name.name);
                        }
                        (!family_names.is_empty()).then_some(family_names)
                    }
                };
                if let Some(family_names) = family_names {
                    for family_name in family_names {
                        let (name_ptr, name_len) = string_parts(&family_name);
                        event_callback(CssRuleEvent {
                            kind: CssRuleEventKind::FontFeatureValuesFamilyName,
                            name_ptr,
                            name_len,
                            value_ptr: std::ptr::null(),
                            value_len: 0,
                            keyframe_selector: 0.0,
                            page_pseudo_class: CssPagePseudoClassKind::Left,
                            important: false,
                            is_block_rule: false,
                        });
                    }
                }
            }
            if at_rule.name.eq_ignore_ascii_case("container") {
                let conditions: Option<Vec<_>> = {
                    let groups = split_component_values_on_comma(&at_rule.prelude);
                    if groups.is_empty() {
                        None
                    } else {
                        let mut conditions = Vec::with_capacity(groups.len());
                        for group in groups {
                            let mut parser = ComponentValueParser::new(group.to_vec());
                            let Some(condition) = parser.parse_container_rule_prelude_item(filtered_input) else {
                                conditions.clear();
                                break;
                            };
                            conditions.push(condition);
                        }
                        (!conditions.is_empty()).then_some(conditions)
                    }
                };
                if let Some(conditions) = conditions {
                    for (name, query) in conditions {
                        let (name_ptr, name_len) =
                            name.as_ref().map_or((std::ptr::null(), 0), |name| string_parts(name));
                        let (value_ptr, value_len) = query
                            .as_ref()
                            .map_or((std::ptr::null(), 0), |query| string_parts(query));
                        event_callback(CssRuleEvent {
                            kind: CssRuleEventKind::ContainerCondition,
                            name_ptr,
                            name_len,
                            value_ptr,
                            value_len,
                            keyframe_selector: 0.0,
                            page_pseudo_class: CssPagePseudoClassKind::Left,
                            important: false,
                            is_block_rule: false,
                        });
                    }
                }
            }
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
            if let Some(selectors) =
                ComponentValueParser::new(qualified_rule.prelude.clone()).parse_a_keyframe_selector_list()
            {
                for selector in selectors {
                    event_callback(CssRuleEvent {
                        kind: CssRuleEventKind::KeyframeSelector,
                        name_ptr: std::ptr::null(),
                        name_len: 0,
                        value_ptr: std::ptr::null(),
                        value_len: 0,
                        keyframe_selector: selector,
                        page_pseudo_class: CssPagePseudoClassKind::Left,
                        important: false,
                        is_block_rule: false,
                    });
                }
            }
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

pub(super) fn emit_rule_or_list_of_declarations_list<E, C>(
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

pub(super) fn emit_declaration<E, C>(
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
        value_ptr: std::ptr::null(),
        value_len: 0,
        keyframe_selector: 0.0,
        page_pseudo_class: CssPagePseudoClassKind::Left,
        important: declaration.important,
        is_block_rule: false,
    });
    for value in &declaration.value {
        emit_component_value(value, filtered_input, component_value_callback);
    }
    event_callback(CssRuleEvent::new(CssRuleEventKind::DeclarationEnd));
}

pub(super) fn emit_component_value_list<E, C>(
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

pub(super) fn emit_boolean_expression<E, C, M, V>(
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

pub(super) fn emit_media_feature_value<C>(
    kind: CssMediaFeatureValueKind,
    syntax_kind: CssMediaFeatureValueSyntaxKind,
    component_values: &[ComponentValue],
    filtered_input: &str,
    callback: &mut C,
) where
    C: FnMut(CssMediaFeatureValue),
{
    let payload = css_media_feature_value_payload(syntax_kind, component_values);
    for component_value in component_values {
        emit_component_value(component_value, filtered_input, &mut |component_value| {
            callback(CssMediaFeatureValue {
                kind,
                syntax_kind,
                payload_kind: payload.kind,
                numeric_value: payload.numeric_value,
                secondary_numeric_value: payload.secondary_numeric_value,
                unit_or_ident_ptr: payload.unit_or_ident.map_or(std::ptr::null(), str::as_ptr),
                unit_or_ident_len: payload.unit_or_ident.map_or(0, str::len),
                component_value,
            });
        });
    }
}

struct CssMediaFeatureValuePayload<'a> {
    kind: CssMediaFeatureValuePayloadKind,
    numeric_value: f64,
    secondary_numeric_value: f64,
    unit_or_ident: Option<&'a str>,
}

impl Default for CssMediaFeatureValuePayload<'_> {
    fn default() -> Self {
        Self {
            kind: CssMediaFeatureValuePayloadKind::None,
            numeric_value: 0.0,
            secondary_numeric_value: 0.0,
            unit_or_ident: None,
        }
    }
}

fn css_media_feature_value_payload(
    syntax_kind: CssMediaFeatureValueSyntaxKind,
    component_values: &[ComponentValue],
) -> CssMediaFeatureValuePayload<'_> {
    let component_values = strip_whitespace(component_values);
    match syntax_kind {
        CssMediaFeatureValueSyntaxKind::Ident => {
            let [
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Ident { value },
                    ..
                }),
            ] = component_values
            else {
                return CssMediaFeatureValuePayload::default();
            };

            CssMediaFeatureValuePayload {
                kind: CssMediaFeatureValuePayloadKind::Ident,
                unit_or_ident: Some(value),
                ..Default::default()
            }
        }
        CssMediaFeatureValueSyntaxKind::Boolean | CssMediaFeatureValueSyntaxKind::Integer => {
            let [
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Number { number },
                    ..
                }),
            ] = component_values
            else {
                return CssMediaFeatureValuePayload::default();
            };
            if !number_is_integer(*number) || number.value() < i32::MIN as f64 || number.value() > i32::MAX as f64 {
                return CssMediaFeatureValuePayload::default();
            }

            CssMediaFeatureValuePayload {
                kind: CssMediaFeatureValuePayloadKind::Integer,
                numeric_value: number.value(),
                ..Default::default()
            }
        }
        CssMediaFeatureValueSyntaxKind::Length => {
            let [component_value] = component_values else {
                return CssMediaFeatureValuePayload::default();
            };

            match component_value {
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Dimension { number, unit },
                    ..
                }) if matches!(dimension_for_unit(unit), Some(DimensionType::Length)) => CssMediaFeatureValuePayload {
                    kind: CssMediaFeatureValuePayloadKind::Length,
                    numeric_value: number.value(),
                    unit_or_ident: Some(unit),
                    ..Default::default()
                },
                // https://drafts.csswg.org/css-values-4/#zero-value
                // Values of 0 can be written without units, even if the value type doesn't allow "unitless zeroes".
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Number { number },
                    ..
                }) if number.value() == 0.0 => CssMediaFeatureValuePayload {
                    kind: CssMediaFeatureValuePayloadKind::Length,
                    numeric_value: 0.0,
                    unit_or_ident: Some("px"),
                    ..Default::default()
                },
                _ => CssMediaFeatureValuePayload::default(),
            }
        }
        CssMediaFeatureValueSyntaxKind::Ratio => {
            let Some((numerator, denominator)) = media_feature_ratio_value(component_values) else {
                return CssMediaFeatureValuePayload::default();
            };

            CssMediaFeatureValuePayload {
                kind: CssMediaFeatureValuePayloadKind::Ratio,
                numeric_value: numerator,
                secondary_numeric_value: denominator,
                ..Default::default()
            }
        }
        CssMediaFeatureValueSyntaxKind::Resolution => {
            let [
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Dimension { number, unit },
                    ..
                }),
            ] = component_values
            else {
                return CssMediaFeatureValuePayload::default();
            };
            if number.value() < 0.0 || !matches!(dimension_for_unit(unit), Some(DimensionType::Resolution)) {
                return CssMediaFeatureValuePayload::default();
            }

            CssMediaFeatureValuePayload {
                kind: CssMediaFeatureValuePayloadKind::Resolution,
                numeric_value: number.value(),
                unit_or_ident: Some(unit),
                ..Default::default()
            }
        }
        CssMediaFeatureValueSyntaxKind::Unknown | CssMediaFeatureValueSyntaxKind::Invalid => {
            CssMediaFeatureValuePayload::default()
        }
    }
}

fn media_feature_ratio_value(component_values: &[ComponentValue]) -> Option<(f64, f64)> {
    let [numerator] = component_values else {
        return media_feature_ratio_with_denominator_value(component_values);
    };
    Some((component_value_non_negative_number_value(numerator)?, 1.0))
}

fn media_feature_ratio_with_denominator_value(component_values: &[ComponentValue]) -> Option<(f64, f64)> {
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
    let [numerator] = numerator else {
        return None;
    };
    let [denominator] = denominator else {
        return None;
    };

    Some((
        component_value_non_negative_number_value(numerator)?,
        component_value_non_negative_number_value(denominator)?,
    ))
}

pub(super) fn emit_media_feature_values<C>(syntax: &MediaFeatureSyntax, filtered_input: &str, callback: &mut C)
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

pub(super) fn css_media_feature_value_syntax_kind_from_syntax(
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

pub(super) fn css_media_feature_from_syntax(syntax: &MediaFeatureSyntax) -> CssMediaFeature {
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

pub(super) fn css_media_feature_name_kind(kind: MediaFeatureNameKind) -> CssMediaFeatureNameKind {
    match kind {
        MediaFeatureNameKind::Normal => CssMediaFeatureNameKind::Normal,
        MediaFeatureNameKind::Min => CssMediaFeatureNameKind::Min,
        MediaFeatureNameKind::Max => CssMediaFeatureNameKind::Max,
    }
}

pub(super) fn css_media_feature_comparison(comparison: MfComparison) -> CssMediaFeatureComparison {
    match comparison {
        MfComparison::Equal => CssMediaFeatureComparison::Equal,
        MfComparison::LessThan => CssMediaFeatureComparison::LessThan,
        MfComparison::LessThanOrEqual => CssMediaFeatureComparison::LessThanOrEqual,
        MfComparison::GreaterThan => CssMediaFeatureComparison::GreaterThan,
        MfComparison::GreaterThanOrEqual => CssMediaFeatureComparison::GreaterThanOrEqual,
    }
}
