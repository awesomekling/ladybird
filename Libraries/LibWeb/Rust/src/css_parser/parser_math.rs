/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::*;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedCalculationNode {
    Numeric(RustOwnedCalculationNumericValue),
    Function {
        name: String,
        arguments: Vec<RustOwnedCalculationNode>,
    },
    Sum(Vec<RustOwnedCalculationNode>),
    Product(Vec<RustOwnedCalculationNode>),
    Negate(Box<RustOwnedCalculationNode>),
    Invert(Box<RustOwnedCalculationNode>),
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum RustOwnedCalculationNumericValue {
    Number(f64),
    Percentage(f64),
    Dimension { value: f64, unit: String },
    Keyword(String),
    TreeCountingFunction(RustOwnedTreeCountingFunction),
}

#[derive(Clone, Debug, PartialEq)]
enum CalculationComponent {
    Value(RustOwnedCalculationNode),
    Operator(char),
}

pub(crate) fn parse_rust_owned_calculation_function(function: &Function) -> Option<RustOwnedCalculationNode> {
    if !is_math_function_name(&function.name) {
        return None;
    }

    if function.name.eq_ignore_ascii_case("calc") {
        return parse_rust_owned_calculation(&function.value);
    }

    let mut arguments = Vec::new();
    for argument in split_calculation_arguments(&function.value) {
        arguments.push(parse_rust_owned_calculation(argument)?);
    }

    Some(RustOwnedCalculationNode::Function {
        name: function.name.clone(),
        arguments,
    })
}

pub(crate) fn parse_rust_owned_calculation(values: &[ComponentValue]) -> Option<RustOwnedCalculationNode> {
    // https://drafts.csswg.org/css-values-4/#parse-a-calculation
    // To parse a calculation:
    //
    // 1. Discard any <whitespace-token>s from values.
    // 2. An item in values is an “operator” if it’s a <delim-token> with the
    //    value "+", "-", "*", or "/". Otherwise, it’s a “value”.
    let mut values = collect_calculation_values(values)?;
    if values.is_empty() {
        return None;
    }
    if matches!(
        (values.first(), values.last()),
        (Some(CalculationComponent::Operator(_)), _) | (_, Some(CalculationComponent::Operator(_)))
    ) {
        return None;
    }

    collect_product_and_invert_nodes(&mut values)?;
    collect_sum_and_negate_nodes(values)
}

pub(crate) fn emit_rust_owned_calculation_tree<C>(node: &RustOwnedCalculationNode, callback: &mut C)
where
    C: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
{
    match node {
        RustOwnedCalculationNode::Numeric(value) => emit_rust_owned_calculation_numeric_value(value, callback),
        RustOwnedCalculationNode::Function { name, arguments } => {
            for argument in arguments {
                emit_rust_owned_calculation_tree(argument, callback);
            }
            callback(
                CssCalculationNodeKind::Function,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                arguments.len() as u32,
                name.as_bytes(),
            );
        }
        RustOwnedCalculationNode::Sum(children) => {
            for child in children {
                emit_rust_owned_calculation_tree(child, callback);
            }
            callback(
                CssCalculationNodeKind::Sum,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                children.len() as u32,
                &[],
            );
        }
        RustOwnedCalculationNode::Product(children) => {
            for child in children {
                emit_rust_owned_calculation_tree(child, callback);
            }
            callback(
                CssCalculationNodeKind::Product,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                children.len() as u32,
                &[],
            );
        }
        RustOwnedCalculationNode::Negate(child) => {
            emit_rust_owned_calculation_tree(child, callback);
            callback(
                CssCalculationNodeKind::Negate,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                1,
                &[],
            );
        }
        RustOwnedCalculationNode::Invert(child) => {
            emit_rust_owned_calculation_tree(child, callback);
            callback(
                CssCalculationNodeKind::Invert,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                1,
                &[],
            );
        }
    }
}

fn emit_rust_owned_calculation_numeric_value<C>(value: &RustOwnedCalculationNumericValue, callback: &mut C)
where
    C: FnMut(CssCalculationNodeKind, CssPrimitiveValueKind, bool, f64, u32, &[u8]),
{
    let (primitive_kind, numeric_value, metadata) = match value {
        RustOwnedCalculationNumericValue::Number(value) => (CssPrimitiveValueKind::Number, *value, ""),
        RustOwnedCalculationNumericValue::Percentage(value) => (CssPrimitiveValueKind::Percentage, *value, ""),
        RustOwnedCalculationNumericValue::Dimension { value, unit } => (
            calculation_dimension_kind(unit).unwrap_or(CssPrimitiveValueKind::Invalid),
            *value,
            unit.as_str(),
        ),
        RustOwnedCalculationNumericValue::Keyword(value) => (CssPrimitiveValueKind::Keyword, 0.0, value.as_str()),
        RustOwnedCalculationNumericValue::TreeCountingFunction(value) => {
            let metadata = match value.function {
                RustOwnedTreeCountingFunctionKind::SiblingCount => "sibling-count",
                RustOwnedTreeCountingFunctionKind::SiblingIndex => "sibling-index",
            };
            callback(
                CssCalculationNodeKind::TreeCountingFunction,
                CssPrimitiveValueKind::Invalid,
                false,
                0.0,
                0,
                metadata.as_bytes(),
            );
            return;
        }
    };

    callback(
        CssCalculationNodeKind::Numeric,
        primitive_kind,
        true,
        numeric_value,
        0,
        metadata.as_bytes(),
    );
}

fn calculation_dimension_kind(unit: &str) -> Option<CssPrimitiveValueKind> {
    match dimension_for_unit(unit)? {
        DimensionType::Angle => Some(CssPrimitiveValueKind::Angle),
        DimensionType::Flex => Some(CssPrimitiveValueKind::Flex),
        DimensionType::Frequency => Some(CssPrimitiveValueKind::Frequency),
        DimensionType::Length => Some(CssPrimitiveValueKind::Length),
        DimensionType::Resolution => Some(CssPrimitiveValueKind::Resolution),
        DimensionType::Time => Some(CssPrimitiveValueKind::Time),
    }
}

fn collect_calculation_values(values: &[ComponentValue]) -> Option<Vec<CalculationComponent>> {
    let mut calculation_values = Vec::new();
    for value in values {
        if matches!(
            value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Whitespace,
                ..
            })
        ) {
            continue;
        }

        if let ComponentValue::PreservedToken(Token {
            token_type: TokenType::Delim { value },
            ..
        }) = value
            && matches!(*value, 0x2b | 0x2d | 0x2a | 0x2f)
        {
            if matches!(calculation_values.last(), Some(CalculationComponent::Operator(_))) {
                return None;
            }
            calculation_values.push(CalculationComponent::Operator(char::from_u32(*value)?));
            continue;
        }

        calculation_values.push(CalculationComponent::Value(process_calculation_leaf(value)?));
    }
    Some(calculation_values)
}

fn collect_product_and_invert_nodes(values: &mut Vec<CalculationComponent>) -> Option<()> {
    // 3. Collect children into Product and Invert nodes.
    //    For every consecutive run of value items in values separated by "*"
    //    or "/" operators:
    while let Some(first_product_operator) = values
        .iter()
        .position(|value| matches!(value, CalculationComponent::Operator('*' | '/')))
    {
        let start_of_run = first_product_operator.checked_sub(1)?;
        let mut end_of_run = first_product_operator + 1;
        for index in ((start_of_run + 1)..values.len()).step_by(2) {
            match values.get(index) {
                Some(CalculationComponent::Operator('*' | '/')) => end_of_run = index + 1,
                _ => break,
            }
        }

        let mut run_values = values.drain(start_of_run..=end_of_run).collect::<Vec<_>>();
        let mut children = Vec::new();
        children.push(component_to_value(run_values.remove(0))?);
        while !run_values.is_empty() {
            let operator = component_to_operator(run_values.remove(0))?;
            let rhs = component_to_value(run_values.remove(0))?;
            if operator == '/' {
                children.push(RustOwnedCalculationNode::Invert(Box::new(rhs)));
            } else {
                children.push(rhs);
            }
        }

        values.insert(
            start_of_run,
            CalculationComponent::Value(RustOwnedCalculationNode::Product(children)),
        );
    }

    Some(())
}

fn collect_sum_and_negate_nodes(mut values: Vec<CalculationComponent>) -> Option<RustOwnedCalculationNode> {
    // 4. Collect children into Sum and Negate nodes.
    //
    // 1. For each "-" operator item in values, replace its right-hand value
    //    item rhs with a Negate node containing rhs as its child.
    let mut index = 0;
    while index < values.len() {
        if matches!(values[index], CalculationComponent::Operator('-')) {
            let rhs_index = index + 1;
            let rhs = component_to_value(values.remove(rhs_index))?;
            values.insert(
                rhs_index,
                CalculationComponent::Value(RustOwnedCalculationNode::Negate(Box::new(rhs))),
            );
        }
        index += 1;
    }

    // 2. If values has only one item, and it is a Product node or a
    //    parenthesized simple block, replace values with that item.
    if values.len() == 1 {
        return component_to_value(values.remove(0));
    }

    // Otherwise, replace values with a Sum node containing the value items of
    // values as its children.
    let mut children = Vec::new();
    let mut operator_count = 0;
    for value in values {
        match value {
            CalculationComponent::Value(value) => children.push(value),
            CalculationComponent::Operator(_) => operator_count += 1,
        }
    }
    if children.is_empty() || operator_count != children.len() - 1 {
        return None;
    }

    Some(RustOwnedCalculationNode::Sum(children))
}

fn process_calculation_leaf(value: &ComponentValue) -> Option<RustOwnedCalculationNode> {
    // 5. At this point values is a tree of Sum, Product, Negate, and Invert
    //    nodes, with other types of values at the leaf nodes. Process the leaf
    //    nodes:
    match value {
        // 1. If leaf is a parenthesized simple block, replace leaf with the
        //    result of parsing a calculation from leaf’s contents.
        ComponentValue::SimpleBlock(block) if is_paren_block(block) => parse_rust_owned_calculation(&block.value),
        // 2. If leaf is a math function, replace leaf with the internal
        //    representation of that math function.
        ComponentValue::Function(function) if is_math_function_name(&function.name) => {
            parse_rust_owned_calculation_function(function)
        }
        ComponentValue::Function(_) => {
            parse_rust_owned_tree_counting_function(PropertyValueType::Number, std::slice::from_ref(value))
                .map(RustOwnedCalculationNumericValue::TreeCountingFunction)
                .map(RustOwnedCalculationNode::Numeric)
        }
        // AD-HOC: As in the C++ parser, token leaves are converted into their
        // numeric representations while processing the calculation tree.
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Ident { value },
            ..
        }) => Some(RustOwnedCalculationNode::Numeric(
            RustOwnedCalculationNumericValue::Keyword(value.clone()),
        )),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Number { number },
            ..
        }) => Some(RustOwnedCalculationNode::Numeric(
            RustOwnedCalculationNumericValue::Number(number.value()),
        )),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Percentage { number },
            ..
        }) => Some(RustOwnedCalculationNode::Numeric(
            RustOwnedCalculationNumericValue::Percentage(number.value()),
        )),
        ComponentValue::PreservedToken(Token {
            token_type: TokenType::Dimension { number, unit },
            ..
        }) => Some(RustOwnedCalculationNode::Numeric(
            RustOwnedCalculationNumericValue::Dimension {
                value: number.value(),
                unit: unit.clone(),
            },
        )),
        _ => None,
    }
}

fn split_calculation_arguments(values: &[ComponentValue]) -> Vec<&[ComponentValue]> {
    let mut groups = Vec::new();
    let mut start = 0;
    for (index, value) in values.iter().enumerate() {
        if matches!(
            value,
            ComponentValue::PreservedToken(Token {
                token_type: TokenType::Comma,
                ..
            })
        ) {
            groups.push(&values[start..index]);
            start = index + 1;
        }
    }
    groups.push(&values[start..]);
    groups
}

fn component_to_value(component: CalculationComponent) -> Option<RustOwnedCalculationNode> {
    match component {
        CalculationComponent::Value(value) => Some(value),
        CalculationComponent::Operator(_) => None,
    }
}

fn component_to_operator(component: CalculationComponent) -> Option<char> {
    match component {
        CalculationComponent::Operator(operator) => Some(operator),
        CalculationComponent::Value(_) => None,
    }
}
