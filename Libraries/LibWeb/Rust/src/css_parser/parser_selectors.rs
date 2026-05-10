/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

pub(super) fn create_invalid_selector_syntax(
    combinator: SelectorCombinator,
    component_values: Vec<ComponentValue>,
) -> SelectorSyntax {
    SelectorSyntax {
        compound_selectors: vec![CompoundSelectorSyntax {
            combinator,
            simple_selectors: vec![SimpleSelectorSyntax::Invalid(
                strip_whitespace(&component_values).to_vec(),
            )],
        }],
    }
}

pub(super) fn validate_pseudo_element_chain(compound_selectors: &[CompoundSelectorSyntax]) -> Option<()> {
    for compound_selector in compound_selectors
        .iter()
        .take(compound_selectors.len().saturating_sub(1))
    {
        if compound_selector
            .simple_selectors
            .iter()
            .any(|simple_selector| matches!(simple_selector, SimpleSelectorSyntax::PseudoElement(_)))
        {
            return None;
        }
    }

    let pseudo_elements: Vec<_> = compound_selectors
        .last()
        .into_iter()
        .flat_map(|compound_selector| compound_selector.simple_selectors.iter())
        .filter_map(|simple_selector| match simple_selector {
            SimpleSelectorSyntax::PseudoElement(pseudo_element) => Some(pseudo_element.pseudo_element_id),
            _ => None,
        })
        .collect();

    if pseudo_elements.len() <= 1 {
        return Some(());
    }

    if pseudo_elements.len() == 2
        && pseudo_elements[0] == PseudoElementId::Part
        && pseudo_elements[1] != PseudoElementId::Part
    {
        return Some(());
    }

    None
}

pub(super) fn selector_contains_unknown_webkit_pseudo_element(selector: &SelectorSyntax) -> bool {
    selector.compound_selectors.iter().any(|compound_selector| {
        compound_selector
            .simple_selectors
            .iter()
            .any(|simple_selector| match simple_selector {
                SimpleSelectorSyntax::PseudoClass(pseudo_class) => pseudo_class
                    .argument_selector_list
                    .iter()
                    .any(selector_contains_unknown_webkit_pseudo_element),
                SimpleSelectorSyntax::PseudoElement(pseudo_element) => {
                    pseudo_element.pseudo_element_id == PseudoElementId::UnknownWebKit
                }
                _ => false,
            })
    })
}

pub(super) fn selector_component_value_ends_selector(component_value: Option<&ComponentValue>) -> bool {
    matches!(
        component_value,
        None | Some(ComponentValue::PreservedToken(Token {
            token_type: TokenType::Whitespace | TokenType::Comma,
            ..
        }))
    )
}

pub(super) fn selector_component_value_is_name(component_value: Option<&ComponentValue>) -> bool {
    component_value_is_delim(component_value, '*')
        || matches!(
            component_value,
            Some(ComponentValue::PreservedToken(Token {
                token_type: TokenType::Ident { .. },
                ..
            }))
        )
}

pub(super) fn selector_component_value_name(component_value: &ComponentValue) -> Option<String> {
    match component_value {
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Delim { value },
            ..
        }) if *value == u32::from(b'*') => Some("*".to_string()),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) => Some(value.clone()),
        _ => None,
    }
}

pub(super) fn component_value_is_one_of_delims(component_value: Option<&ComponentValue>, delimiters: &[char]) -> bool {
    delimiters
        .iter()
        .any(|delimiter| component_value_is_delim(component_value, *delimiter))
}

pub(super) fn is_square_block(block: &SimpleBlock) -> bool {
    matches!(block.token.token_type, TokenType::OpenSquare)
}

pub(super) fn is_legacy_single_colon_pseudo_element(pseudo_element_id: PseudoElementId) -> bool {
    matches!(
        pseudo_element_id,
        PseudoElementId::After | PseudoElementId::Before | PseudoElementId::FirstLetter | PseudoElementId::FirstLine
    )
}

pub(super) fn split_component_values_by_comma(component_values: Vec<ComponentValue>) -> Vec<Vec<ComponentValue>> {
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
        } else {
            current_group.push(component_value);
        }
    }

    groups.push(current_group);
    groups
}

pub(super) fn parse_integer_component_value(component_value: ComponentValue) -> Option<i32> {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Number { number },
        ..
    }) = component_value
    else {
        return None;
    };
    numeric_value_to_i32(number)
}

pub(super) fn parse_signed_integer_component_value(component_value: ComponentValue) -> Option<i32> {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Number { number },
        ..
    }) = component_value
    else {
        return None;
    };
    if number.number_type() != CssNumberType::IntegerWithExplicitSign {
        return None;
    }
    numeric_value_to_i32(number)
}

pub(super) fn parse_signless_integer_component_value(component_value: ComponentValue) -> Option<i32> {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Number { number },
        ..
    }) = component_value
    else {
        return None;
    };
    if number.number_type() != CssNumberType::Integer {
        return None;
    }
    numeric_value_to_i32(number)
}

pub(super) fn numeric_value_to_i32(number: NumericValue) -> Option<i32> {
    if !matches!(
        number.number_type(),
        CssNumberType::Integer | CssNumberType::IntegerWithExplicitSign
    ) {
        return None;
    }

    if number.value() < f64::from(i32::MIN) || number.value() > f64::from(i32::MAX) {
        return None;
    }
    Some(number.value() as i32)
}

pub(super) fn parse_an_plus_b_dimension(component_value: &ComponentValue) -> Option<(i32, i32)> {
    let ComponentValue::PreservedToken(Token {
        token_type: TokenType::Dimension { number, unit },
        ..
    }) = component_value
    else {
        return None;
    };
    let step_size = numeric_value_to_i32(*number)?;

    if unit.eq_ignore_ascii_case("n") {
        return Some((step_size, 0));
    }
    if unit.eq_ignore_ascii_case("n-") {
        return Some((step_size, i32::MIN));
    }
    parse_ndashdigit_ident(unit, "n-").map(|offset| (step_size, offset))
}

pub(super) fn parse_ndashdigit_ident(string: &str, prefix: &str) -> Option<i32> {
    if string.len() <= prefix.len() || !string[..prefix.len()].eq_ignore_ascii_case(prefix) {
        return None;
    }
    let digits = &string[prefix.len()..];
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let value = digits.parse::<i32>().ok()?;
    Some(-value)
}
