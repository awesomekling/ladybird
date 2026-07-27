/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Expansion of handler aliases and declarative bytecode specializations.

use super::ast::{
    Block, Declaration, ElseContinuation, Expression, ExpressionKind, HandlerDeclaration, Parameter, Pattern, Program,
    ScalarMatchArm, Statement, StatementKind, ValueMatchArm,
};
use super::diagnostic::Diagnostic;
use crate::hash::HashMap;
use crate::types::{BlockTemperature, ParameterMode, Type};
use flap_metadata::{SpecializedInstruction, SpecializedParameterBinding};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ExpansionState {
    Expanding,
    Expanded,
}

pub(crate) fn expand_declarative_specializations(
    filename: &str,
    program: &mut Program,
    specializations: &[SpecializedInstruction],
) -> Result<(), Diagnostic> {
    let handlers = program
        .declarations
        .iter()
        .filter_map(|declaration| {
            let Declaration::Handler(handler) = declaration else {
                return None;
            };
            Some((handler.name.clone(), handler.clone()))
        })
        .collect::<HashMap<_, _>>();
    let fallback_span = handlers
        .values()
        .next()
        .map(|handler| handler.span)
        .expect("specializations require at least one handler");

    for specialization in specializations {
        let mut parameters = Vec::<Parameter>::new();
        for field in &specialization.definition.fields {
            let field_name = field.name.strip_prefix("m_").unwrap_or(&field.name);
            let mut source_parameters = specialization
                .components
                .iter()
                .filter_map(|component| {
                    let handler = handlers.get(&component.bytecode)?;
                    component.parameters.iter().find_map(|(source_name, binding)| {
                        (binding == &SpecializedParameterBinding::Field(field_name.to_string())).then(|| {
                            handler
                                .parameters
                                .iter()
                                .find(|parameter| parameter.name == *source_name)
                                .expect("specialization fields name validated handler parameters")
                        })
                    })
                })
                .collect::<Vec<_>>();
            let Some(first) = source_parameters.first().copied() else {
                return Err(Diagnostic::new(
                    filename,
                    fallback_span,
                    format!(
                        "specialized field '{}.{}' has no source parameter",
                        specialization.definition.name, field_name
                    ),
                ));
            };
            let mode = source_parameters.drain(..).fold(first.mode, |mode, parameter| {
                merge_parameter_modes(mode, parameter.mode)
            });
            parameters.push(Parameter {
                name: field_name.to_string(),
                mode,
                ty: if field.ty == "i32" {
                    Type::InlineInt32
                } else {
                    first.ty.clone()
                },
                is_else: false,
                span: first.span,
            });
        }

        let mut component_bodies = Vec::with_capacity(specialization.components.len());
        for component in &specialization.components {
            let handler = handlers.get(&component.bytecode).ok_or_else(|| {
                Diagnostic::new(
                    filename,
                    fallback_span,
                    format!("specialization references unknown handler '{}'", component.bytecode),
                )
            })?;
            let substitutions = component
                .parameters
                .iter()
                .map(|(source_name, binding)| {
                    let expression = match binding {
                        SpecializedParameterBinding::Field(name) => ExpressionKind::Name(name.clone()),
                        SpecializedParameterBinding::ExactInteger(value) => ExpressionKind::Integer(i64::from(*value)),
                        SpecializedParameterBinding::Undefined => ExpressionKind::Name("Value<Undefined>".to_string()),
                    };
                    (source_name.clone(), expression)
                })
                .collect::<HashMap<_, _>>();
            let mut body = handler.body.clone();
            substitute_block_parameters(&mut body, &substitutions);
            component_bodies.push(body);
        }

        let span = component_bodies.first().map(|body| body.span).unwrap_or(fallback_span);
        let body = if component_bodies.len() == 1 {
            component_bodies.pop().unwrap()
        } else {
            let continuation_names = (0..component_bodies.len())
                .map(|index| format!("__specialization_component_{index}"))
                .collect::<Vec<_>>();
            let mut statements = Vec::with_capacity(component_bodies.len() + 1);
            for (index, mut body) in component_bodies.into_iter().enumerate() {
                if let Some(next) = continuation_names.get(index + 1) {
                    replace_dispatch_next(&mut body, next);
                }
                statements.push(Statement::new(
                    StatementKind::Continuation {
                        name: continuation_names[index].clone(),
                        temperature: BlockTemperature::Default,
                        parameters: Vec::new(),
                        body,
                    },
                    span,
                ));
            }
            statements.push(Statement::new(
                StatementKind::ContinuationJump {
                    target: continuation_names[0].clone(),
                    arguments: Vec::new(),
                },
                span,
            ));
            Block {
                statements,
                value: None,
                span,
            }
        };
        program.declarations.push(Declaration::Handler(HandlerDeclaration {
            name: specialization.definition.name.clone(),
            parameters,
            temperature: BlockTemperature::Default,
            body,
            span,
        }));
    }
    Ok(())
}

fn merge_parameter_modes(lhs: ParameterMode, rhs: ParameterMode) -> ParameterMode {
    match (lhs, rhs) {
        (ParameterMode::In, ParameterMode::In) => ParameterMode::In,
        (ParameterMode::Out, ParameterMode::Out) => ParameterMode::Out,
        _ => ParameterMode::InOut,
    }
}

fn substitute_expression(expression: &mut Expression, substitutions: &HashMap<String, ExpressionKind>) {
    match &mut expression.kind {
        ExpressionKind::Name(name) => {
            if let Some(replacement) = substitutions.get(name) {
                expression.kind = replacement.clone();
            }
        }
        ExpressionKind::Call { callee, arguments } => {
            if callee == "load"
                && let [
                    argument @ Expression {
                        kind: ExpressionKind::Name(name),
                        ..
                    },
                ] = arguments.as_slice()
                && let Some(replacement) = substitutions.get(name)
            {
                match replacement {
                    ExpressionKind::Name(replacement) if replacement.starts_with("Value<") => {
                        expression.kind = ExpressionKind::Name(replacement.clone());
                        return;
                    }
                    ExpressionKind::Integer(value) => {
                        expression.kind = ExpressionKind::Call {
                            callee: "box_i32".to_string(),
                            arguments: vec![Expression {
                                kind: ExpressionKind::Integer(*value),
                                span: argument.span,
                            }],
                        };
                        return;
                    }
                    _ => {}
                }
            }
            for argument in arguments {
                substitute_expression(argument, substitutions);
            }
        }
        ExpressionKind::Memory(arguments) | ExpressionKind::Tuple(arguments) => {
            for argument in arguments {
                substitute_expression(argument, substitutions);
            }
        }
        ExpressionKind::Unary { operand, .. } => substitute_expression(operand, substitutions),
        ExpressionKind::Binary { lhs, rhs, .. } => {
            substitute_expression(lhs, substitutions);
            substitute_expression(rhs, substitutions);
        }
        ExpressionKind::Index { base, index } => {
            substitute_expression(base, substitutions);
            substitute_expression(index, substitutions);
        }
        ExpressionKind::Integer(_) => {}
    }
}

fn substitute_pattern(pattern: &mut Pattern, substitutions: &HashMap<String, ExpressionKind>) {
    match pattern {
        Pattern::Tuple(patterns) | Pattern::Alternatives(patterns) => {
            for pattern in patterns {
                substitute_pattern(pattern, substitutions);
            }
        }
        Pattern::Expression(expression) => substitute_expression(expression, substitutions),
        Pattern::ValueRepresentation { binding, .. } => {
            if let Some(binding) = binding {
                substitute_pattern(binding, substitutions);
            }
        }
        Pattern::Binding { .. } | Pattern::Wildcard => {}
    }
}

fn substitute_else_continuation(continuation: &mut ElseContinuation, substitutions: &HashMap<String, ExpressionKind>) {
    match continuation {
        ElseContinuation::Label(_) => {}
        ElseContinuation::Invocation { arguments, .. } => {
            for argument in arguments {
                substitute_expression(argument, substitutions);
            }
        }
        ElseContinuation::Block { body, .. } => substitute_block_parameters(body, substitutions),
    }
}

fn substitute_block_parameters(block: &mut Block, substitutions: &HashMap<String, ExpressionKind>) {
    for statement in &mut block.statements {
        match &mut statement.kind {
            StatementKind::Let { pattern, initializer } => {
                substitute_pattern(pattern, substitutions);
                if let Some(initializer) = initializer {
                    substitute_expression(initializer, substitutions);
                }
            }
            StatementKind::GuardLet {
                pattern,
                initializer,
                failure,
            } => {
                substitute_pattern(pattern, substitutions);
                substitute_expression(initializer, substitutions);
                substitute_else_continuation(failure, substitutions);
            }
            StatementKind::FallibleCall { arguments, failure, .. } => {
                for argument in arguments {
                    substitute_expression(argument, substitutions);
                }
                substitute_else_continuation(failure, substitutions);
            }
            StatementKind::Assign { name, initializer } => {
                if let Some(ExpressionKind::Name(replacement)) = substitutions.get(name) {
                    *name = replacement.clone();
                }
                substitute_expression(initializer, substitutions);
            }
            StatementKind::IndexAssign { base, index, value } => {
                substitute_expression(base, substitutions);
                substitute_expression(index, substitutions);
                substitute_expression(value, substitutions);
            }
            StatementKind::Expression(expression) => substitute_expression(expression, substitutions),
            StatementKind::ValueMatch {
                value, arms, fallback, ..
            } => {
                substitute_expression(value, substitutions);
                for ValueMatchArm { body, .. } in arms {
                    substitute_block_parameters(body, substitutions);
                }
                substitute_block_parameters(&mut fallback.body, substitutions);
            }
            StatementKind::ScalarMatch { value, arms, fallback } => {
                substitute_expression(value, substitutions);
                for ScalarMatchArm { pattern, body, .. } in arms {
                    substitute_pattern(pattern, substitutions);
                    substitute_block_parameters(body, substitutions);
                }
                substitute_block_parameters(fallback, substitutions);
            }
            StatementKind::Guard { condition, failure } => {
                substitute_expression(condition, substitutions);
                substitute_else_continuation(failure, substitutions);
            }
            StatementKind::Continuation { body, .. } | StatementKind::While { body, .. } => {
                substitute_block_parameters(body, substitutions);
                if let StatementKind::While { condition, .. } = &mut statement.kind {
                    substitute_expression(condition, substitutions);
                }
            }
            StatementKind::ContinuationJump { arguments, .. } => {
                for argument in arguments {
                    substitute_expression(argument, substitutions);
                }
            }
            StatementKind::If {
                condition,
                then_body,
                else_body,
                ..
            } => {
                substitute_expression(condition, substitutions);
                substitute_block_parameters(then_body, substitutions);
                if let Some(else_body) = else_body {
                    substitute_block_parameters(else_body, substitutions);
                }
            }
        }
    }
    if let Some(value) = &mut block.value {
        substitute_expression(value, substitutions);
    }
}

fn replace_dispatch_next(block: &mut Block, next: &str) {
    for statement in &mut block.statements {
        if matches!(
            &statement.kind,
            StatementKind::Expression(Expression {
                kind: ExpressionKind::Name(name),
                ..
            }) if name == "dispatch_next"
        ) || matches!(
            &statement.kind,
            StatementKind::Expression(Expression {
                kind: ExpressionKind::Call { callee, arguments },
                ..
            }) if callee == "dispatch_next" && arguments.is_empty()
        ) {
            statement.kind = StatementKind::ContinuationJump {
                target: next.to_string(),
                arguments: Vec::new(),
            };
            continue;
        }
        match &mut statement.kind {
            StatementKind::GuardLet { failure, .. }
            | StatementKind::FallibleCall { failure, .. }
            | StatementKind::Guard { failure, .. } => replace_dispatch_in_else(failure, next),
            StatementKind::ValueMatch { arms, fallback, .. } => {
                for arm in arms {
                    replace_dispatch_next(&mut arm.body, next);
                }
                replace_dispatch_next(&mut fallback.body, next);
            }
            StatementKind::ScalarMatch { arms, fallback, .. } => {
                for arm in arms {
                    replace_dispatch_next(&mut arm.body, next);
                }
                replace_dispatch_next(fallback, next);
            }
            StatementKind::Continuation { body, .. } | StatementKind::While { body, .. } => {
                replace_dispatch_next(body, next);
            }
            StatementKind::If {
                then_body, else_body, ..
            } => {
                replace_dispatch_next(then_body, next);
                if let Some(else_body) = else_body {
                    replace_dispatch_next(else_body, next);
                }
            }
            _ => {}
        }
    }
}

fn replace_dispatch_in_else(continuation: &mut ElseContinuation, next: &str) {
    if let ElseContinuation::Block { body, .. } = continuation {
        replace_dispatch_next(body, next);
    }
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
