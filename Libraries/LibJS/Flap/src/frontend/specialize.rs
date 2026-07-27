/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Expansion of handlers defined as specializations of other handlers.

use super::ast::{Declaration, ExpressionKind, HandlerDeclaration, Program, StatementKind};
use super::diagnostic::Diagnostic;
use crate::hash::HashMap;
use crate::types::{ParameterMode, Type};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExpansionState {
    Expanding,
    Expanded,
}

pub(crate) fn expand_handler_specializations(filename: &str, program: &mut Program) -> Result<(), Diagnostic> {
    let handlers = program
        .declarations
        .iter()
        .enumerate()
        .filter_map(|(index, declaration)| {
            let Declaration::Handler(handler) = declaration else {
                return None;
            };
            Some((handler.name.clone(), index))
        })
        .collect::<HashMap<_, _>>();
    let mut states = HashMap::default();
    for index in handlers.values().copied() {
        expand_handler(filename, program, &handlers, &mut states, index)?;
    }
    Ok(())
}

fn specialization_target(handler: &HandlerDeclaration) -> Option<(&str, &[super::ast::Expression])> {
    let [statement] = handler.body.statements.as_slice() else {
        return None;
    };
    let StatementKind::Expression(expression) = &statement.kind else {
        return None;
    };
    let ExpressionKind::Call { callee, arguments } = &expression.kind else {
        return None;
    };
    Some((callee, arguments))
}

fn expand_handler(
    filename: &str,
    program: &mut Program,
    handlers: &HashMap<String, usize>,
    states: &mut HashMap<usize, ExpansionState>,
    index: usize,
) -> Result<(), Diagnostic> {
    match states.get(&index) {
        Some(ExpansionState::Expanded) => return Ok(()),
        Some(ExpansionState::Expanding) => {
            let Declaration::Handler(handler) = &program.declarations[index] else {
                unreachable!()
            };
            return Err(Diagnostic::new(
                filename,
                handler.span,
                format!("cyclic handler specialization involving '{}'", handler.name),
            ));
        }
        None => {}
    }
    states.insert(index, ExpansionState::Expanding);

    let target = {
        let Declaration::Handler(handler) = &program.declarations[index] else {
            unreachable!()
        };
        specialization_target(handler)
            .and_then(|(callee, arguments)| handlers.get(callee).map(|target| (*target, arguments.to_vec())))
    };
    let Some((target_index, arguments)) = target else {
        states.insert(index, ExpansionState::Expanded);
        return Ok(());
    };
    expand_handler(filename, program, handlers, states, target_index)?;

    let (specialized_name, specialized_span, specialized_parameters) = {
        let Declaration::Handler(handler) = &program.declarations[index] else {
            unreachable!()
        };
        (handler.name.clone(), handler.span, handler.parameters.clone())
    };
    let (target_name, target_parameters, target_body) = {
        let Declaration::Handler(handler) = &program.declarations[target_index] else {
            unreachable!()
        };
        (handler.name.clone(), handler.parameters.clone(), handler.body.clone())
    };

    if arguments.len() != target_parameters.len() || specialized_parameters.len() != target_parameters.len() {
        return Err(Diagnostic::new(
            filename,
            specialized_span,
            format!("handler specialization '{specialized_name}' must preserve the parameter count of '{target_name}'"),
        ));
    }

    for ((argument, specialized), target) in arguments.iter().zip(&specialized_parameters).zip(&target_parameters) {
        let ExpressionKind::Name(argument_name) = &argument.kind else {
            return Err(Diagnostic::new(
                filename,
                argument.span,
                "handler specialization arguments must be parameter names",
            ));
        };
        if argument_name != &specialized.name || specialized.name != target.name {
            return Err(Diagnostic::new(
                filename,
                argument.span,
                "handler specialization parameters must preserve names and order",
            ));
        }
        let compatible = specialized.mode == target.mode
            && (specialized.ty == target.ty
                || (target.mode == ParameterMode::In
                    && target.ty == Type::Operand
                    && specialized.ty == Type::InlineInt32));
        if !compatible {
            return Err(Diagnostic::new(
                filename,
                specialized.span,
                format!(
                    "parameter '{}' cannot specialize {} as {}",
                    specialized.name, target.ty, specialized.ty
                ),
            ));
        }
    }

    let Declaration::Handler(handler) = &mut program.declarations[index] else {
        unreachable!()
    };
    handler.body = target_body;
    states.insert(index, ExpansionState::Expanded);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frontend::parser;

    #[test]
    fn expands_int32_handler_specialization() {
        let mut program = parser::parse(
            "test.flap",
            "
handler Add(dst: out Operand, lhs: in Operand, rhs: in Operand) {
    store(dst, load(lhs));
    let value = load(rhs);
    dispatch_next;
}
handler AddInt32(dst: out Operand, lhs: in Operand, rhs: Int32) = Add(dst, lhs, rhs);
",
        )
        .unwrap();
        expand_handler_specializations("test.flap", &mut program).unwrap();
        let Declaration::Handler(handler) = &program.declarations[1] else {
            unreachable!()
        };
        assert_eq!(handler.body.statements.len(), 3);
    }

    #[test]
    fn rejects_handler_specialization_cycles() {
        let mut program = parser::parse(
            "test.flap",
            "
handler First(value: in Operand) = Second(value);
handler Second(value: in Operand) = First(value);
",
        )
        .unwrap();
        let error = expand_handler_specializations("test.flap", &mut program).unwrap_err();
        assert!(error.message.contains("cyclic handler specialization"));
    }
}
