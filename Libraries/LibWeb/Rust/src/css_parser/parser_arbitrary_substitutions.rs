/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::parser_shared::parser_from_filtered_input;
use super::parser_syntax_media::contains_only_declaration_value;
use super::parser_types::Nested;
use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ArbitrarySubstitutionIfArgsBranch {
    pub(crate) condition: Vec<ComponentValue>,
    pub(crate) value: Vec<ComponentValue>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ArbitrarySubstitutionFunction {
    Attr = 0,
    Env = 1,
    If = 2,
    Inherit = 3,
    Var = 4,
}

impl ArbitrarySubstitutionFunction {
    fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Attr),
            1 => Some(Self::Env),
            2 => Some(Self::If),
            3 => Some(Self::Inherit),
            4 => Some(Self::Var),
            _ => None,
        }
    }
}

fn split_component_values_on_token<F>(
    component_values: &[ComponentValue],
    mut is_separator: F,
) -> Vec<&[ComponentValue]>
where
    F: FnMut(&ComponentValue) -> bool,
{
    let mut groups = Vec::new();
    let mut start = 0;
    for (index, component_value) in component_values.iter().enumerate() {
        if is_separator(component_value) {
            groups.push(&component_values[start..index]);
            start = index + 1;
        }
    }
    groups.push(&component_values[start..]);
    groups
}

fn is_empty_after_trimming(component_values: &[ComponentValue]) -> bool {
    strip_whitespace(component_values).is_empty()
}

pub(crate) fn parse_arbitrary_substitution_function_declaration_value_arguments(
    filtered_input: &[u8],
    function: u8,
) -> Option<Vec<Vec<ComponentValue>>> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let function = ArbitrarySubstitutionFunction::from_u8(function)?;
    match function {
        ArbitrarySubstitutionFunction::Attr
        | ArbitrarySubstitutionFunction::Env
        | ArbitrarySubstitutionFunction::Inherit
        | ArbitrarySubstitutionFunction::Var => {
            let groups = split_component_values_on_token(&component_values, |component_value| {
                matches!(
                    component_value,
                    ComponentValue::PreservedToken(Token {
                        token_type: TokenType::Comma,
                        ..
                    })
                )
            });
            if groups.is_empty() || groups.len() > 2 {
                return None;
            }

            if is_empty_after_trimming(groups[0]) || !contains_only_declaration_value(groups[0], Nested::No) {
                return None;
            }

            if groups.len() == 2 && !contains_only_declaration_value(groups[1], Nested::No) {
                return None;
            }

            Some(groups.into_iter().map(|group| group.to_vec()).collect())
        }
        ArbitrarySubstitutionFunction::If => None,
    }
}

pub(crate) fn parse_arbitrary_substitution_function_if_arguments(
    filtered_input: &[u8],
) -> Option<Vec<ArbitrarySubstitutionIfArgsBranch>> {
    let (mut parser, _) = parser_from_filtered_input(filtered_input);
    let component_values = parser.parse_a_list_of_component_values();
    let mut branches = split_component_values_on_token(&component_values, |component_value| {
        matches!(
            component_value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Semicolon,
                ..
            })
        )
    });

    if branches.is_empty() {
        return None;
    }

    if branches.last().is_some_and(|branch| is_empty_after_trimming(branch)) {
        branches.pop();
    }

    if branches.is_empty() {
        return None;
    }

    let mut parsed_branches = Vec::new();
    for branch in branches {
        let (condition, value) = branch.iter().enumerate().find_map(|(index, component_value)| {
            matches!(
                component_value,
                ComponentValue::PreservedToken(Token {
                    token_type: TokenType::Colon,
                    ..
                })
            )
            .then_some((&branch[..index], &branch[index + 1..]))
        })?;

        if is_empty_after_trimming(condition) || !contains_only_declaration_value(condition, Nested::No) {
            return None;
        }
        if !contains_only_declaration_value(value, Nested::No) {
            return None;
        }

        parsed_branches.push(ArbitrarySubstitutionIfArgsBranch {
            condition: condition.to_vec(),
            value: value.to_vec(),
        });
    }

    Some(parsed_branches)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_declaration_value_arguments() {
        let parsed = parse_arbitrary_substitution_function_declaration_value_arguments(b"foo, bar", 0)
            .expect("expected declaration-value arguments");
        assert_eq!(parsed.len(), 2);
        assert_eq!(strip_whitespace(&parsed[0]).len(), 1);
        assert_eq!(strip_whitespace(&parsed[1]).len(), 1);
    }

    #[test]
    fn rejects_invalid_declaration_value_arguments() {
        assert!(parse_arbitrary_substitution_function_declaration_value_arguments(b"foo, bar, baz", 0).is_none());
        assert!(parse_arbitrary_substitution_function_declaration_value_arguments(b";", 0).is_none());
    }

    #[test]
    fn parses_if_arguments() {
        let parsed =
            parse_arbitrary_substitution_function_if_arguments(b"foo: bar; baz: qux").expect("expected if arguments");
        assert_eq!(parsed.len(), 2);
        assert_eq!(strip_whitespace(&parsed[0].condition).len(), 1);
        assert_eq!(strip_whitespace(&parsed[0].value).len(), 1);
        assert_eq!(strip_whitespace(&parsed[1].condition).len(), 1);
        assert_eq!(strip_whitespace(&parsed[1].value).len(), 1);
    }

    #[test]
    fn rejects_invalid_if_arguments() {
        assert!(parse_arbitrary_substitution_function_if_arguments(b"foo;").is_none());
        assert!(parse_arbitrary_substitution_function_if_arguments(b"foo bar").is_none());
    }
}
