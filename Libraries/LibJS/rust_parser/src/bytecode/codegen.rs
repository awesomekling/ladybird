/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Bytecode generation from Rust AST.
//!
//! This module walks the Rust AST and emits bytecode instructions via
//! the `Generator`, mirroring C++ `ASTCodegen.cpp`.
//!
//! Each AST node's codegen returns `Option<ScopedOperand>`:
//! - `Some(op)` if the node produces a value (expressions)
//! - `None` for statements that don't produce values

use crate::ast::*;

use super::generator::{choose_dst, BlockBoundaryType, FinallyContext, Generator, ScopedOperand};
use super::instruction::Instruction;
use super::operand::*;

/// Generate bytecode for an expression.
pub fn generate_expr(
    expr: &Expr,
    gen: &mut Generator,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    gen.current_source_start = expr.range.start.offset;
    gen.current_source_end = expr.range.end.offset;

    match &expr.inner {
        // === Literals ===
        Expression::NumericLiteral(value) => Some(gen.add_constant_number(*value)),

        Expression::BooleanLiteral(value) => Some(gen.add_constant_boolean(*value)),

        Expression::NullLiteral => Some(gen.add_constant_null()),

        Expression::StringLiteral(value) => Some(gen.add_constant_string(value.clone())),

        Expression::BigIntLiteral(value) => Some(gen.add_constant_bigint(value.clone())),

        Expression::RegExpLiteral(data) => {
            let source_index = gen.intern_string(data.pattern.clone());
            let flags_index = gen.intern_string(data.flags.clone());
            let regex_index = gen.intern_regex(data.pattern.clone(), data.flags.clone());
            let dst = choose_dst(gen, preferred_dst);
            gen.emit(Instruction::NewRegExp {
                dst: dst.operand(),
                source_index,
                flags_index,
                regex_index,
            });
            Some(dst)
        }

        // === Identifiers ===
        Expression::Identifier(ident) => generate_identifier(ident, gen, preferred_dst),

        // === This ===
        Expression::This => {
            gen.emit(Instruction::ResolveThisBinding);
            Some(gen.this_value())
        }

        // === Unary ===
        Expression::Unary { op, operand } => {
            // typeof and delete on identifiers need special handling BEFORE
            // evaluating the operand to avoid throwing on unresolvable references.
            if *op == UnaryOp::Typeof {
                if let Expression::Identifier(ident) = &operand.inner {
                    if !ident.is_local() {
                        let dst = choose_dst(gen, preferred_dst);
                        let id = gen.intern_identifier(ident.name.clone());
                        gen.emit(Instruction::TypeofBinding {
                            dst: dst.operand(),
                            identifier: id,
                            cache: EnvironmentCoordinate::empty(),
                        });
                        return Some(dst);
                    }
                }
            }
            if *op == UnaryOp::Delete {
                return Some(emit_delete_reference(gen, operand, preferred_dst));
            }

            let value = generate_expr(operand, gen, None)?;
            let dst = choose_dst(gen, preferred_dst);
            match op {
                UnaryOp::BitwiseNot => {
                    gen.emit(Instruction::BitwiseNot {
                        dst: dst.operand(),
                        src: value.operand(),
                    });
                }
                UnaryOp::Not => {
                    gen.emit(Instruction::Not {
                        dst: dst.operand(),
                        src: value.operand(),
                    });
                }
                UnaryOp::Plus => {
                    gen.emit(Instruction::UnaryPlus {
                        dst: dst.operand(),
                        src: value.operand(),
                    });
                }
                UnaryOp::Minus => {
                    gen.emit(Instruction::UnaryMinus {
                        dst: dst.operand(),
                        src: value.operand(),
                    });
                }
                UnaryOp::Typeof => {
                    gen.emit(Instruction::Typeof {
                        dst: dst.operand(),
                        src: value.operand(),
                    });
                }
                UnaryOp::Void => {
                    return Some(gen.add_constant_undefined());
                }
                UnaryOp::Delete => unreachable!(),
            }
            Some(dst)
        }

        // === Binary ===
        Expression::Binary { op, lhs, rhs } => {
            let lhs_val = generate_expr(lhs, gen, None)?;
            let rhs_val = generate_expr(rhs, gen, None)?;
            let dst = choose_dst(gen, preferred_dst);
            emit_binary_op(gen, *op, &dst, &lhs_val, &rhs_val);
            Some(dst)
        }

        // === Logical (short-circuit) ===
        Expression::Logical { op, lhs, rhs } => {
            generate_logical(gen, *op, lhs, rhs, preferred_dst)
        }

        // === Conditional (ternary) ===
        Expression::Conditional {
            test,
            consequent,
            alternate,
        } => generate_conditional(gen, test, consequent, alternate, preferred_dst),

        // === Sequence ===
        Expression::Sequence(exprs) => {
            let mut last = None;
            for expr in exprs {
                last = generate_expr(expr, gen, None);
                if gen.is_current_block_terminated() {
                    break;
                }
            }
            last
        }

        // === Function expressions ===
        Expression::Function(data) => {
            let has_name = data.name.is_some();
            let mut name_id = None;

            // Named function expressions get an intermediate scope so the name
            // is visible inside the function body but not outside.
            if has_name {
                let parent = gen.lexical_environment_register_stack.last().cloned()
                    .unwrap_or_else(|| gen.add_constant_undefined());
                let new_env = gen.allocate_register();
                gen.start_boundary(BlockBoundaryType::LeaveLexicalEnvironment);
                gen.emit(Instruction::CreateLexicalEnvironment {
                    dst: new_env.operand(),
                    parent: parent.operand(),
                    capacity: 0,
                });
                gen.lexical_environment_register_stack.push(new_env);

                let id = gen.intern_identifier(data.name.as_ref().unwrap().name.clone());
                gen.emit(Instruction::CreateVariable {
                    identifier: id,
                    mode: 0, // Lexical
                    is_immutable: true,
                    is_global: false,
                    is_strict: false,
                });
                name_id = Some(id);
            }

            let dst = choose_dst(gen, preferred_dst);
            // For anonymous function expressions, use the pending LHS name
            // as the function's .name property.
            let lhs_name = if !has_name { gen.pending_lhs_name.take() } else { None };
            let lhs_name_str: Option<Vec<u16>> = lhs_name.map(|idx| gen.identifier_table[idx.0 as usize].clone());
            let name_override = if !has_name {
                lhs_name_str.as_deref()
            } else {
                None
            };
            let shared_function_data_index = emit_new_function(gen, data, name_override);
            gen.emit(Instruction::NewFunction {
                dst: dst.operand(),
                shared_function_data_index,
                lhs_name,
                home_object: None,
            });

            if has_name {
                gen.emit(Instruction::InitializeLexicalBinding {
                    identifier: name_id.unwrap(),
                    src: dst.operand(),
                    cache: EnvironmentCoordinate::empty(),
                });

                gen.end_boundary(BlockBoundaryType::LeaveLexicalEnvironment);
                gen.lexical_environment_register_stack.pop();

                if !gen.is_current_block_terminated() {
                    let parent = gen.lexical_environment_register_stack.last().cloned()
                        .unwrap_or_else(|| gen.add_constant_undefined());
                    gen.emit(Instruction::SetLexicalEnvironment {
                        environment: parent.operand(),
                    });
                }
            }
            Some(dst)
        }

        // === Array ===
        Expression::Array(elements) => {
            let dst = choose_dst(gen, preferred_dst);

            // Find the first spread element.
            let first_spread = elements.iter().position(|e| {
                matches!(e, Some(elem) if matches!(elem.inner, Expression::Spread(_)))
            });

            // Collect elements before the first spread into a NewArray.
            let pre_spread_count = first_spread.unwrap_or(elements.len());
            let mut scoped_args: Vec<ScopedOperand> = Vec::new();
            for elem in &elements[..pre_spread_count] {
                match elem {
                    Some(e) => {
                        let val = generate_expr(e, gen, None).unwrap_or_else(|| {
                            gen.add_constant_undefined()
                        });
                        scoped_args.push(val);
                    }
                    None => {
                        scoped_args.push(gen.add_constant_empty());
                    }
                }
            }
            let args: Vec<Operand> = scoped_args.iter().map(|s| s.operand()).collect();
            gen.emit(Instruction::NewArray {
                dst: dst.operand(),
                element_count: args.len() as u32,
                elements: args,
            });
            drop(scoped_args);

            // Append elements after the first spread using ArrayAppend.
            if let Some(spread_idx) = first_spread {
                for elem in &elements[spread_idx..] {
                    match elem {
                        None => {
                            let empty = gen.add_constant_empty();
                            gen.emit(Instruction::ArrayAppend {
                                dst: dst.operand(),
                                src: empty.operand(),
                                is_spread: false,
                            });
                        }
                        Some(e) => {
                            let is_spread = matches!(e.inner, Expression::Spread(_));
                            let val = generate_expr(e, gen, None).unwrap_or_else(|| {
                                gen.add_constant_undefined()
                            });
                            gen.emit(Instruction::ArrayAppend {
                                dst: dst.operand(),
                                src: val.operand(),
                                is_spread,
                            });
                        }
                    }
                }
            }

            Some(dst)
        }

        // === Member access ===
        Expression::Member {
            object,
            property,
            computed,
        } => {
            let obj = generate_expr(object, gen, None)?;
            let base_id = intern_base_identifier(gen, object);
            let dst = choose_dst(gen, preferred_dst);
            if *computed {
                let prop = generate_expr(property, gen, None)?;
                gen.emit(Instruction::GetByValue {
                    dst: dst.operand(),
                    base: obj.operand(),
                    property: prop.operand(),
                    base_identifier: base_id,
                });
            } else {
                // Non-computed: property must be an Identifier
                if let Expression::Identifier(ident) = &property.inner {
                    emit_get_by_id(gen, &dst, &obj, &ident.name, base_id);
                } else if let Expression::PrivateIdentifier(priv_ident) = &property.inner {
                    let id = gen.intern_identifier(priv_ident.name.clone());
                    gen.emit(Instruction::GetPrivateById {
                        dst: dst.operand(),
                        base: obj.operand(),
                        property: id,
                    });
                }
            }
            Some(dst)
        }

        // === Call ===
        Expression::Call(data) => {
            generate_call_expression(gen, data, preferred_dst, false)
        }

        // === New ===
        Expression::New(data) => {
            generate_call_expression(gen, data, preferred_dst, true)
        }

        // === Spread ===
        Expression::Spread(inner) => {
            // Spread is handled by the caller (Call, Array, Object)
            generate_expr(inner, gen, preferred_dst)
        }

        // === Yield ===
        Expression::Yield {
            argument,
            is_yield_from: _,
        } => {
            let value = if let Some(arg) = argument {
                generate_expr(arg, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined())
            } else {
                gen.add_constant_undefined()
            };

            if !gen.is_in_async_generator_function() {
                // Regular generator: yield + completion protocol.
                Some(generate_regular_yield(gen, value, preferred_dst))
            } else {
                // Async generator: full yield protocol.
                Some(generate_async_generator_yield(gen, value, preferred_dst))
            }
        }

        // === Await ===
        Expression::Await(inner) => {
            let value = generate_expr(inner, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            Some(generate_await(gen, value))
        }

        // === MetaProperty ===
        Expression::MetaProperty(MetaPropertyType::NewTarget) => {
            let dst = choose_dst(gen, preferred_dst);
            gen.emit(Instruction::GetNewTarget { dst: dst.operand() });
            Some(dst)
        }

        Expression::MetaProperty(MetaPropertyType::ImportMeta) => {
            let dst = choose_dst(gen, preferred_dst);
            gen.emit(Instruction::GetImportMeta { dst: dst.operand() });
            Some(dst)
        }

        // === ImportCall ===
        Expression::ImportCall { specifier, options } => {
            let spec = generate_expr(specifier, gen, None)?;
            let opts = match options {
                Some(o) => generate_expr(o, gen, None)?,
                None => gen.add_constant_undefined(),
            };
            let dst = choose_dst(gen, preferred_dst);
            gen.emit(Instruction::ImportCall {
                dst: dst.operand(),
                specifier: spec.operand(),
                options: opts.operand(),
            });
            Some(dst)
        }

        // === Update (++/--) ===
        Expression::Update {
            op,
            argument,
            prefixed,
        } => generate_update_expression(gen, *op, argument, *prefixed, preferred_dst),

        // === Assignment ===
        Expression::Assignment { op, lhs, rhs } => {
            generate_assignment_expression(gen, *op, lhs, rhs, preferred_dst)
        }

        // === Template literals ===
        Expression::TemplateLiteral(data) => {
            generate_template_literal(gen, data, preferred_dst)
        }

        // === Tagged template literals ===
        Expression::TaggedTemplateLiteral { tag, template_literal } => {
            generate_tagged_template_literal(gen, tag, template_literal, preferred_dst)
        }

        // === Object ===
        Expression::Object(data) => {
            generate_object_expression(gen, data, preferred_dst)
        }

        // === OptionalChain ===
        Expression::OptionalChain { base, references } => {
            generate_optional_chain(gen, base, references, preferred_dst)
        }

        // === SuperCall ===
        Expression::SuperCall(data) => {
            let dst = choose_dst(gen, preferred_dst);
            // Build arguments array — keep ScopedOperands alive.
            let args_array_dst = gen.allocate_register();
            let mut arg_holders = Vec::new();
            for arg in &data.arguments {
                let val = generate_expr(&arg.value, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined());
                arg_holders.push(val);
            }
            let arg_ops: Vec<Operand> = arg_holders.iter().map(|a| a.operand()).collect();
            gen.emit(Instruction::NewArray {
                dst: args_array_dst.operand(),
                element_count: arg_ops.len() as u32,
                elements: arg_ops,
            });
            gen.emit(Instruction::SuperCallWithArgumentArray {
                dst: dst.operand(),
                arguments: args_array_dst.operand(),
                is_synthetic: data.is_synthetic,
            });
            Some(dst)
        }

        Expression::Super => {
            // super keyword as an expression (for super.foo, super[foo])
            // Returns the home object's prototype
            let dst = choose_dst(gen, preferred_dst);
            gen.emit(Instruction::ResolveSuperBase { dst: dst.operand() });
            Some(dst)
        }

        Expression::Class(data) => {
            generate_class_expression(gen, data, preferred_dst)
        }

        Expression::PrivateIdentifier(_) => {
            // Private identifiers are handled by member access codegen
            None
        }

        Expression::Error => None,
    }
}

/// Generate bytecode for a statement.
pub fn generate_stmt(
    stmt: &Stmt,
    gen: &mut Generator,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    gen.current_source_start = stmt.range.start.offset;
    gen.current_source_end = stmt.range.end.offset;

    match &stmt.inner {
        Statement::Empty | Statement::Error | Statement::ErrorDeclaration => None,
        Statement::Debugger => None,

        // === ExpressionStatement ===
        Statement::Expression(expr) => generate_expr(expr, gen, preferred_dst),

        // === Block ===
        Statement::Block(scope) => generate_block_statement(gen, scope, preferred_dst),

        // === FunctionBody ===
        Statement::FunctionBody { scope, .. } => generate_scope_children(gen, scope, preferred_dst),

        // === Program ===
        Statement::Program(data) => generate_scope_children(gen, &data.scope, preferred_dst),

        // === If ===
        Statement::If {
            predicate,
            consequent,
            alternate,
        } => generate_if_statement(gen, predicate, consequent, alternate.as_deref(), preferred_dst),

        // === While ===
        Statement::While { test, body } => {
            generate_while_statement(gen, test, body, preferred_dst)
        }

        // === DoWhile ===
        Statement::DoWhile { test, body } => {
            generate_do_while_statement(gen, test, body, preferred_dst)
        }

        // === For ===
        Statement::For {
            init,
            test,
            update,
            body,
        } => generate_for_statement(gen, init.as_deref(), test.as_deref(), update.as_deref(), body, preferred_dst),

        // === Return ===
        Statement::Return(value) => {
            let mut val = match value {
                Some(expr) => generate_expr(expr, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined()),
                None => gen.add_constant_undefined(),
            };
            // Async functions implicitly await the return value.
            if gen.is_in_async_function() {
                val = generate_await(gen, val);
            }
            gen.generate_return(&val);
            None
        }

        // === Throw ===
        Statement::Throw(expr) => {
            let val = generate_expr(expr, gen, None)?;
            gen.emit(Instruction::Throw { src: val.operand() });
            None
        }

        // === Variable declarations ===
        Statement::VariableDeclaration { kind, declarations } => {
            generate_variable_declaration(gen, *kind, declarations);
            None
        }

        // === Break ===
        Statement::Break { target_label } => {
            gen.generate_break(target_label.as_deref());
            None
        }

        // === Continue ===
        Statement::Continue { target_label } => {
            gen.generate_continue(target_label.as_deref());
            None
        }

        // === Labelled ===
        Statement::Labelled { label, item } => {
            generate_labelled_statement(gen, label, item, preferred_dst)
        }

        // === Switch ===
        Statement::Switch(data) => {
            generate_switch_statement(gen, data, preferred_dst)
        }

        // === Try ===
        Statement::Try(data) => {
            generate_try_statement(gen, data, preferred_dst)
        }

        // === FunctionDeclaration (hoisted, noop at declaration site) ===
        Statement::FunctionDeclaration(_) => None,

        // === With ===
        Statement::With { object, body } => {
            let obj = generate_expr(object, gen, None)?;
            let object_environment = gen.allocate_register();
            gen.emit(Instruction::EnterObjectEnvironment { dst: object_environment.operand(), object: obj.operand() });
            gen.lexical_environment_register_stack.push(object_environment);
            gen.start_boundary(BlockBoundaryType::LeaveLexicalEnvironment);

            let result = generate_stmt(body, gen, preferred_dst);

            gen.end_boundary(BlockBoundaryType::LeaveLexicalEnvironment);
            gen.lexical_environment_register_stack.pop();

            if !gen.is_current_block_terminated() {
                let parent = gen.lexical_environment_register_stack.last().cloned()
                    .unwrap_or_else(|| gen.add_constant_undefined());
                gen.emit(Instruction::SetLexicalEnvironment {
                    environment: parent.operand(),
                });
            }
            result
        }

        // === ForIn ===
        Statement::ForIn { lhs, rhs, body } => {
            generate_for_in_statement(gen, lhs, rhs, body, preferred_dst)
        }

        // === ForOf ===
        Statement::ForOf { lhs, rhs, body } => {
            generate_for_of_statement(gen, lhs, rhs, body, preferred_dst)
        }

        // === ForAwaitOf ===
        Statement::ForAwaitOf { lhs, rhs, body } => {
            generate_for_await_of_statement(gen, lhs, rhs, body, preferred_dst)
        }

        // === UsingDeclaration ===
        Statement::UsingDeclaration { declarations, .. } => {
            generate_variable_declaration(gen, DeclarationKind::Let, declarations);
            None
        }

        // === ClassDeclaration ===
        Statement::ClassDeclaration(data) => {
            let value = generate_class_expression(gen, data, None);
            // Bind the class name in the outer scope (classes are lexically scoped).
            // Use InitializeLexicalBinding since the name was registered as
            // an uninitialized lexical binding by the scope collector.
            if let (Some(name_ident), Some(val)) = (&data.name, &value) {
                if name_ident.is_local() {
                    let local_index = name_ident.local_index.get();
                    let local = match name_ident.local_type.get() {
                        LocalType::Argument => gen.scoped_operand(Operand::argument(local_index)),
                        LocalType::Variable => gen.local(local_index),
                        LocalType::None => unreachable!(),
                    };
                    gen.emit_mov(&local, &val);
                    gen.mark_local_initialized(local_index);
                } else {
                    let id = gen.intern_identifier(name_ident.name.clone());
                    gen.emit(Instruction::InitializeLexicalBinding {
                        identifier: id,
                        src: val.operand(),
                        cache: EnvironmentCoordinate::empty(),
                    });
                }
            }
            None
        }

        // === Import/Export ===
        Statement::Import(_) => None, // Handled by module loading
        Statement::Export(_) => None, // Handled by module loading

        // === ClassFieldInitializer ===
        Statement::ClassFieldInitializer { expression, .. } => {
            let value = generate_expr(expression, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::Return {
                value: value.operand(),
            });
            None
        }
    }
}

// =============================================================================
// Await helper
// =============================================================================

/// Emit an Await instruction and handle the received completion.
///
/// After the await resumes, checks the completion type:
/// - Normal: continue with the received value
/// - Throw: re-throw the received value
fn generate_await(gen: &mut Generator, argument: ScopedOperand) -> ScopedOperand {
    let received_completion = gen.allocate_register();
    let received_completion_type = gen.allocate_register();
    let received_completion_value = gen.allocate_register();
    generate_await_with_completions(
        gen, argument,
        &received_completion, &received_completion_type, &received_completion_value,
    )
}

// Completion::Type constants matching C++ enum.
const COMPLETION_TYPE_NORMAL: f64 = 1.0;
const COMPLETION_TYPE_RETURN: f64 = 4.0;
const COMPLETION_TYPE_THROW: f64 = 5.0;

/// Like generate_await but uses caller-provided completion registers.
///
/// Returns the received_completion_value on the normal path.
/// Emits a Throw on the throw path.
fn generate_await_with_completions(
    gen: &mut Generator,
    argument: ScopedOperand,
    received_completion: &ScopedOperand,
    received_completion_type: &ScopedOperand,
    received_completion_value: &ScopedOperand,
) -> ScopedOperand {
    let continuation = gen.make_block();
    gen.emit(Instruction::Await {
        continuation_label: Label(continuation as u32),
        argument: argument.operand(),
    });
    gen.switch_to_basic_block(continuation);

    let acc = gen.accumulator();
    gen.emit_mov(received_completion, &acc);
    gen.emit(Instruction::GetCompletionFields {
        type_dst: received_completion_type.operand(),
        value_dst: received_completion_value.operand(),
        completion: received_completion.operand(),
    });

    let normal_block = gen.make_block();
    let throw_block = gen.make_block();
    let is_normal = gen.allocate_register();
    let normal_type = gen.add_constant_number(COMPLETION_TYPE_NORMAL);
    gen.emit(Instruction::StrictlyEquals {
        dst: is_normal.operand(),
        lhs: received_completion_type.operand(),
        rhs: normal_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_normal.operand(),
        true_target: Label(normal_block as u32),
        false_target: Label(throw_block as u32),
    });

    gen.switch_to_basic_block(throw_block);
    gen.emit(Instruction::Throw {
        src: received_completion_value.operand(),
    });

    gen.switch_to_basic_block(normal_block);
    received_completion_value.clone()
}

/// Regular generator yield with completion protocol.
///
/// After the yield resumes, the accumulator contains a CompletionCell.
/// We extract the completion type and value, then branch:
/// - Normal: continue with the extracted value
/// - Throw: throw the value
/// - Return: yield the value without continuation (done)
fn generate_regular_yield(
    gen: &mut Generator,
    value: ScopedOperand,
    preferred_dst: Option<&ScopedOperand>,
) -> ScopedOperand {
    let continuation = gen.make_block();
    gen.emit(Instruction::Yield {
        continuation_label: Some(Label(continuation as u32)),
        value: value.operand(),
    });
    gen.switch_to_basic_block(continuation);

    // Save the accumulator (CompletionCell) and extract type + value.
    let received_completion = gen.allocate_register();
    let received_completion_type = gen.allocate_register();
    let received_completion_value = gen.allocate_register();
    let acc = gen.accumulator();
    gen.emit_mov(&received_completion, &acc);
    gen.emit(Instruction::GetCompletionFields {
        type_dst: received_completion_type.operand(),
        value_dst: received_completion_value.operand(),
        completion: received_completion.operand(),
    });

    // Check: type == Normal(1)?
    let normal_block = gen.make_block();
    let not_normal_block = gen.make_block();
    let type_is_normal = gen.allocate_register();
    let normal_type = gen.add_constant_number(COMPLETION_TYPE_NORMAL);
    gen.emit(Instruction::StrictlyEquals {
        dst: type_is_normal.operand(),
        lhs: received_completion_type.operand(),
        rhs: normal_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: type_is_normal.operand(),
        true_target: Label(normal_block as u32),
        false_target: Label(not_normal_block as u32),
    });

    // Not normal: check Throw(5) vs Return(4).
    gen.switch_to_basic_block(not_normal_block);
    let throw_block = gen.make_block();
    let return_block = gen.make_block();
    let type_is_throw = gen.allocate_register();
    let throw_type = gen.add_constant_number(COMPLETION_TYPE_THROW);
    gen.emit(Instruction::StrictlyEquals {
        dst: type_is_throw.operand(),
        lhs: received_completion_type.operand(),
        rhs: throw_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: type_is_throw.operand(),
        true_target: Label(throw_block as u32),
        false_target: Label(return_block as u32),
    });

    // Throw block: throw the value.
    gen.switch_to_basic_block(throw_block);
    gen.emit(Instruction::Throw {
        src: received_completion_value.operand(),
    });

    // Return block: yield the value without continuation (done).
    gen.switch_to_basic_block(return_block);
    gen.emit(Instruction::Yield {
        continuation_label: None,
        value: received_completion_value.operand(),
    });

    // Normal block: the yield expression evaluates to the extracted value.
    gen.switch_to_basic_block(normal_block);
    let dst = choose_dst(gen, preferred_dst);
    gen.emit_mov(&dst, &received_completion_value);
    dst
}

/// Full async generator yield protocol (AsyncGeneratorYield + UnwrapYieldResumption).
///
/// 1. Await the value before yielding
/// 2. Yield the awaited value
/// 3. On resume, handle AsyncGeneratorUnwrapYieldResumption:
///    - If not return: jump to main continuation with the completion
///    - If return: await the return value, then handle throw/normal
/// 4. At main continuation, extract completion type/value
/// 5. Normal → return value, Throw → throw, Return → yield-return
fn generate_async_generator_yield(
    gen: &mut Generator,
    value: ScopedOperand,
    preferred_dst: Option<&ScopedOperand>,
) -> ScopedOperand {
    let received_completion = gen.allocate_register();
    let received_completion_type = gen.allocate_register();
    let received_completion_value = gen.allocate_register();

    // Step 1: Await the value before yielding.
    let awaited_value = generate_await_with_completions(
        gen, value,
        &received_completion, &received_completion_type, &received_completion_value,
    );

    // Step 2: Yield the awaited value.
    let unwrap_block = gen.make_block();
    gen.emit(Instruction::Yield {
        continuation_label: Some(Label(unwrap_block as u32)),
        value: awaited_value.operand(),
    });
    gen.switch_to_basic_block(unwrap_block);

    // Step 3: AsyncGeneratorUnwrapYieldResumption.
    // Get the completion from the accumulator.
    let acc = gen.accumulator();
    gen.emit_mov(&received_completion, &acc);
    gen.emit(Instruction::GetCompletionFields {
        type_dst: received_completion_type.operand(),
        value_dst: received_completion_value.operand(),
        completion: received_completion.operand(),
    });

    // If resumptionValue.[[Type]] is not return, jump to main continuation.
    let main_continuation = gen.make_block();
    let return_block = gen.make_block();
    let is_not_return = gen.allocate_register();
    let return_type = gen.add_constant_number(COMPLETION_TYPE_RETURN);
    gen.emit(Instruction::StrictlyInequals {
        dst: is_not_return.operand(),
        lhs: received_completion_type.operand(),
        rhs: return_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_not_return.operand(),
        true_target: Label(main_continuation as u32),
        false_target: Label(return_block as u32),
    });

    // Return path: Await(resumptionValue.[[Value]]).
    gen.switch_to_basic_block(return_block);
    generate_await_with_completions(
        gen, received_completion_value.clone(),
        &received_completion, &received_completion_type, &received_completion_value,
    );

    // If awaited.[[Type]] is throw, jump to main continuation (which will handle it).
    let awaited_normal_block = gen.make_block();
    let is_throw = gen.allocate_register();
    let throw_type = gen.add_constant_number(COMPLETION_TYPE_THROW);
    gen.emit(Instruction::StrictlyEquals {
        dst: is_throw.operand(),
        lhs: received_completion_type.operand(),
        rhs: throw_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_throw.operand(),
        true_target: Label(main_continuation as u32),
        false_target: Label(awaited_normal_block as u32),
    });

    // awaited.[[Type]] is normal: set type to Return and jump to main continuation.
    gen.switch_to_basic_block(awaited_normal_block);
    gen.emit(Instruction::SetCompletionType {
        completion: received_completion.operand(),
        completion_type: 4, // Completion::Type::Return
    });
    gen.emit(Instruction::Jump {
        target: Label(main_continuation as u32),
    });

    // Step 4: Main continuation.
    gen.switch_to_basic_block(main_continuation);
    gen.emit_mov(&received_completion, &acc);
    gen.emit(Instruction::GetCompletionFields {
        type_dst: received_completion_type.operand(),
        value_dst: received_completion_value.operand(),
        completion: received_completion.operand(),
    });

    // Step 5: Check completion type.
    let normal_cont = gen.make_block();
    let throw_cont = gen.make_block();
    let is_normal = gen.allocate_register();
    let normal_type = gen.add_constant_number(COMPLETION_TYPE_NORMAL);
    gen.emit(Instruction::StrictlyEquals {
        dst: is_normal.operand(),
        lhs: received_completion_type.operand(),
        rhs: normal_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_normal.operand(),
        true_target: Label(normal_cont as u32),
        false_target: Label(throw_cont as u32),
    });

    // Throw/return path.
    gen.switch_to_basic_block(throw_cont);
    let return_value_block = gen.make_block();
    let throw_value_block = gen.make_block();
    let is_throw2 = gen.allocate_register();
    gen.emit(Instruction::StrictlyEquals {
        dst: is_throw2.operand(),
        lhs: received_completion_type.operand(),
        rhs: throw_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_throw2.operand(),
        true_target: Label(throw_value_block as u32),
        false_target: Label(return_value_block as u32),
    });

    // Throw: re-throw the value.
    gen.switch_to_basic_block(throw_value_block);
    gen.emit(Instruction::Throw {
        src: received_completion_value.operand(),
    });

    // Return: yield with no continuation (done).
    gen.switch_to_basic_block(return_value_block);
    gen.emit(Instruction::Yield {
        continuation_label: None,
        value: received_completion_value.operand(),
    });

    // Normal: return the value.
    gen.switch_to_basic_block(normal_cont);
    let dst = choose_dst(gen, preferred_dst);
    gen.emit_mov(&dst, &received_completion_value);
    dst
}

// =============================================================================
// Identifier codegen
// =============================================================================

fn generate_identifier(
    ident: &Identifier,
    gen: &mut Generator,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    if ident.is_local() {
        let local_index = ident.local_index.get();
        let local = match ident.local_type.get() {
            LocalType::Argument => gen.scoped_operand(Operand::argument(local_index)),
            LocalType::Variable => gen.local(local_index),
            LocalType::None => unreachable!(),
        };
        // Check TDZ for uninitialized locals
        if !gen.is_local_initialized(local_index)
            && ident.declaration_kind.get() != IdentDeclarationKind::Var
        {
            gen.emit(Instruction::ThrowIfTDZ {
                src: local.operand(),
            });
        }
        return Some(local);
    }

    let dst = choose_dst(gen, preferred_dst);
    if ident.is_global.get() {
        let id = gen.intern_identifier(ident.name.clone());
        let cache = gen.next_global_variable_cache();
        gen.emit(Instruction::GetGlobal {
            dst: dst.operand(),
            identifier: id,
            cache_index: cache,
        });
    } else {
        let id = gen.intern_identifier(ident.name.clone());
        gen.emit(Instruction::GetBinding {
            dst: dst.operand(),
            identifier: id,
            cache: EnvironmentCoordinate::empty(),
        });
    }
    Some(dst)
}

// =============================================================================
// Binary operator emission
// =============================================================================

fn emit_binary_op(
    gen: &mut Generator,
    op: BinaryOp,
    dst: &ScopedOperand,
    lhs: &ScopedOperand,
    rhs: &ScopedOperand,
) {
    let d = dst.operand();
    let l = lhs.operand();
    let r = rhs.operand();
    match op {
        BinaryOp::Addition => gen.emit(Instruction::Add { dst: d, lhs: l, rhs: r }),
        BinaryOp::Subtraction => gen.emit(Instruction::Sub { dst: d, lhs: l, rhs: r }),
        BinaryOp::Multiplication => gen.emit(Instruction::Mul { dst: d, lhs: l, rhs: r }),
        BinaryOp::Division => gen.emit(Instruction::Div { dst: d, lhs: l, rhs: r }),
        BinaryOp::Modulo => gen.emit(Instruction::Mod { dst: d, lhs: l, rhs: r }),
        BinaryOp::Exponentiation => gen.emit(Instruction::Exp { dst: d, lhs: l, rhs: r }),
        BinaryOp::StrictlyEquals => gen.emit(Instruction::StrictlyEquals { dst: d, lhs: l, rhs: r }),
        BinaryOp::StrictlyInequals => gen.emit(Instruction::StrictlyInequals { dst: d, lhs: l, rhs: r }),
        BinaryOp::LooselyEquals => gen.emit(Instruction::LooselyEquals { dst: d, lhs: l, rhs: r }),
        BinaryOp::LooselyInequals => gen.emit(Instruction::LooselyInequals { dst: d, lhs: l, rhs: r }),
        BinaryOp::GreaterThan => gen.emit(Instruction::GreaterThan { dst: d, lhs: l, rhs: r }),
        BinaryOp::GreaterThanEquals => gen.emit(Instruction::GreaterThanEquals { dst: d, lhs: l, rhs: r }),
        BinaryOp::LessThan => gen.emit(Instruction::LessThan { dst: d, lhs: l, rhs: r }),
        BinaryOp::LessThanEquals => gen.emit(Instruction::LessThanEquals { dst: d, lhs: l, rhs: r }),
        BinaryOp::BitwiseAnd => gen.emit(Instruction::BitwiseAnd { dst: d, lhs: l, rhs: r }),
        BinaryOp::BitwiseOr => gen.emit(Instruction::BitwiseOr { dst: d, lhs: l, rhs: r }),
        BinaryOp::BitwiseXor => gen.emit(Instruction::BitwiseXor { dst: d, lhs: l, rhs: r }),
        BinaryOp::LeftShift => gen.emit(Instruction::LeftShift { dst: d, lhs: l, rhs: r }),
        BinaryOp::RightShift => gen.emit(Instruction::RightShift { dst: d, lhs: l, rhs: r }),
        BinaryOp::UnsignedRightShift => gen.emit(Instruction::UnsignedRightShift { dst: d, lhs: l, rhs: r }),
        BinaryOp::In => gen.emit(Instruction::In { dst: d, lhs: l, rhs: r }),
        BinaryOp::InstanceOf => gen.emit(Instruction::InstanceOf { dst: d, lhs: l, rhs: r }),
    }
}

// =============================================================================
// Logical expression (short-circuit)
// =============================================================================

fn generate_logical(
    gen: &mut Generator,
    op: LogicalOp,
    lhs: &Expr,
    rhs: &Expr,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let dst = choose_dst(gen, preferred_dst);
    let lhs_val = generate_expr(lhs, gen, Some(&dst))?;
    gen.emit_mov(&dst, &lhs_val);

    let end_block = gen.make_block();
    let rhs_block = gen.make_block();

    match op {
        LogicalOp::And => {
            // If lhs is falsy, short-circuit to end
            gen.emit(Instruction::JumpIf {
                condition: dst.operand(),
                true_target: Label(rhs_block as u32),
                false_target: Label(end_block as u32),
            });
        }
        LogicalOp::Or => {
            // If lhs is truthy, short-circuit to end
            gen.emit(Instruction::JumpIf {
                condition: dst.operand(),
                true_target: Label(end_block as u32),
                false_target: Label(rhs_block as u32),
            });
        }
        LogicalOp::NullishCoalescing => {
            gen.emit(Instruction::JumpNullish {
                condition: dst.operand(),
                true_target: Label(rhs_block as u32),
                false_target: Label(end_block as u32),
            });
        }
    }

    gen.switch_to_basic_block(rhs_block);
    let rhs_val = generate_expr(rhs, gen, Some(&dst));
    if let Some(rhs_val) = &rhs_val {
        gen.emit_mov(&dst, rhs_val);
    }
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(end_block as u32),
        });
    }

    gen.switch_to_basic_block(end_block);
    Some(dst)
}

// =============================================================================
// Conditional expression (ternary)
// =============================================================================

fn generate_conditional(
    gen: &mut Generator,
    test: &Expr,
    consequent: &Expr,
    alternate: &Expr,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let dst = choose_dst(gen, preferred_dst);
    let predicate = generate_expr(test, gen, None)?;

    let true_block = gen.make_block();
    let false_block = gen.make_block();
    let end_block = gen.make_block();

    gen.emit(Instruction::JumpIf {
        condition: predicate.operand(),
        true_target: Label(true_block as u32),
        false_target: Label(false_block as u32),
    });

    gen.switch_to_basic_block(true_block);
    let cons_val = generate_expr(consequent, gen, Some(&dst));
    if let Some(val) = &cons_val {
        gen.emit_mov(&dst, val);
    }
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(end_block as u32),
        });
    }

    gen.switch_to_basic_block(false_block);
    let alt_val = generate_expr(alternate, gen, Some(&dst));
    if let Some(val) = &alt_val {
        gen.emit_mov(&dst, val);
    }
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(end_block as u32),
        });
    }

    gen.switch_to_basic_block(end_block);
    Some(dst)
}

// =============================================================================
// If statement
// =============================================================================

fn generate_if_statement(
    gen: &mut Generator,
    predicate: &Expr,
    consequent: &Stmt,
    alternate: Option<&Stmt>,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let pred = generate_expr(predicate, gen, None).unwrap_or_else(|| gen.add_constant_undefined());

    let true_block = gen.make_block();
    let false_block = gen.make_block();
    let has_alternate = alternate.is_some();
    let end_block = if has_alternate { gen.make_block() } else { false_block };

    gen.emit(Instruction::JumpIf {
        condition: pred.operand(),
        true_target: Label(true_block as u32),
        false_target: Label(false_block as u32),
    });

    gen.switch_to_basic_block(true_block);
    let _cons_result = generate_stmt(consequent, gen, preferred_dst);
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(end_block as u32),
        });
    }

    if let Some(alt) = alternate {
        gen.switch_to_basic_block(false_block);
        let _alt_result = generate_stmt(alt, gen, preferred_dst);
        if !gen.is_current_block_terminated() {
            gen.emit(Instruction::Jump {
                target: Label(end_block as u32),
            });
        }
    }

    gen.switch_to_basic_block(end_block);
    None
}

// =============================================================================
// While statement
// =============================================================================

fn generate_while_statement(
    gen: &mut Generator,
    test: &Expr,
    body: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let test_block = gen.make_block();
    let body_block = gen.make_block();
    let end_block = gen.make_block();

    gen.emit(Instruction::Jump {
        target: Label(test_block as u32),
    });

    gen.switch_to_basic_block(test_block);
    let test_val = generate_expr(test, gen, None).unwrap_or_else(|| gen.add_constant_undefined());
    gen.emit(Instruction::JumpIf {
        condition: test_val.operand(),
        true_target: Label(body_block as u32),
        false_target: Label(end_block as u32),
    });

    gen.switch_to_basic_block(body_block);
    gen.begin_continuable_scope(Label(test_block as u32), Vec::new());
    gen.begin_breakable_scope(Label(end_block as u32), Vec::new());
    let _body_result = generate_stmt(body, gen, preferred_dst);
    gen.end_breakable_scope();
    gen.end_continuable_scope();
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(test_block as u32),
        });
    }

    gen.switch_to_basic_block(end_block);
    None
}

// =============================================================================
// DoWhile statement
// =============================================================================

fn generate_do_while_statement(
    gen: &mut Generator,
    test: &Expr,
    body: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let body_block = gen.make_block();
    let test_block = gen.make_block();
    let end_block = gen.make_block();

    gen.emit(Instruction::Jump {
        target: Label(body_block as u32),
    });

    gen.switch_to_basic_block(body_block);
    gen.begin_continuable_scope(Label(test_block as u32), Vec::new());
    gen.begin_breakable_scope(Label(end_block as u32), Vec::new());
    let _body_result = generate_stmt(body, gen, preferred_dst);
    gen.end_breakable_scope();
    gen.end_continuable_scope();
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(test_block as u32),
        });
    }

    gen.switch_to_basic_block(test_block);
    let test_val = generate_expr(test, gen, None).unwrap_or_else(|| gen.add_constant_undefined());
    gen.emit(Instruction::JumpIf {
        condition: test_val.operand(),
        true_target: Label(body_block as u32),
        false_target: Label(end_block as u32),
    });

    gen.switch_to_basic_block(end_block);
    None
}

// =============================================================================
// For statement
// =============================================================================

fn generate_for_statement(
    gen: &mut Generator,
    init: Option<&Stmt>,
    test: Option<&Expr>,
    update: Option<&Expr>,
    body: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    // Check if init is a lexical declaration (let/const) with non-local variables.
    // If so, we need to create a lexical environment for the loop variables and
    // implement per-iteration copy semantics (CreatePerIterationEnvironment).
    let mut has_lexical_environment = false;
    let mut per_iteration_binding_names: Vec<Vec<u16>> = Vec::new();

    if let Some(init) = init {
        if let Statement::VariableDeclaration { kind, declarations } = &init.inner {
            if *kind == DeclarationKind::Let || *kind == DeclarationKind::Const {
                let mut non_local_names: Vec<(Vec<u16>, bool)> = Vec::new();
                for decl in declarations {
                    collect_target_names(&decl.target, &mut non_local_names);
                }
                if !non_local_names.is_empty() {
                    has_lexical_environment = true;
                    let is_const = *kind == DeclarationKind::Const;

                    // begin_variable_scope: CreateLexicalEnvironment
                    let parent = gen.lexical_environment_register_stack.last().cloned()
                        .unwrap_or_else(|| gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT)));
                    let new_env = gen.allocate_register();
                    gen.emit(Instruction::CreateLexicalEnvironment {
                        dst: new_env.operand(),
                        parent: parent.operand(),
                        capacity: non_local_names.len() as u32,
                    });
                    gen.lexical_environment_register_stack.push(new_env);

                    for (name, _) in &non_local_names {
                        let id = gen.intern_identifier(name.clone());
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: ENV_MODE_LEXICAL,
                            is_immutable: is_const,
                            is_global: false,
                            is_strict: false,
                        });
                        if !is_const {
                            per_iteration_binding_names.push(name.clone());
                        }
                    }
                }
            }
        }
    }

    // Init
    if let Some(init) = init {
        generate_stmt(init, gen, None);
    }

    // CreatePerIterationEnvironment after init (first iteration setup).
    emit_per_iteration_bindings(gen, &per_iteration_binding_names);

    let test_block = gen.make_block();
    let body_block = gen.make_block();
    let update_block = gen.make_block();
    let end_block = gen.make_block();

    gen.emit(Instruction::Jump {
        target: Label(test_block as u32),
    });

    // Test
    gen.switch_to_basic_block(test_block);
    if let Some(test) = test {
        let test_val = generate_expr(test, gen, None).unwrap_or_else(|| gen.add_constant_undefined());
        gen.emit(Instruction::JumpIf {
            condition: test_val.operand(),
            true_target: Label(body_block as u32),
            false_target: Label(end_block as u32),
        });
    } else {
        gen.emit(Instruction::Jump {
            target: Label(body_block as u32),
        });
    }

    // Body
    gen.switch_to_basic_block(body_block);
    gen.begin_continuable_scope(Label(update_block as u32), Vec::new());
    gen.begin_breakable_scope(Label(end_block as u32), Vec::new());
    let _body_result = generate_stmt(body, gen, preferred_dst);
    gen.end_breakable_scope();
    gen.end_continuable_scope();
    if !gen.is_current_block_terminated() {
        // CreatePerIterationEnvironment at end of each iteration.
        emit_per_iteration_bindings(gen, &per_iteration_binding_names);
        gen.emit(Instruction::Jump {
            target: Label(update_block as u32),
        });
    }

    // Update
    gen.switch_to_basic_block(update_block);
    if let Some(update) = update {
        generate_expr(update, gen, None);
    }
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(test_block as u32),
        });
    }

    gen.switch_to_basic_block(end_block);

    // end_variable_scope: restore parent environment
    if has_lexical_environment {
        gen.lexical_environment_register_stack.pop();
        if !gen.is_current_block_terminated() {
            let parent = gen.lexical_environment_register_stack.last().cloned()
                .unwrap_or_else(|| gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT)));
            gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
        }
    }

    None
}

/// Emit CreatePerIterationEnvironment: save current binding values, pop env,
/// push new env, re-create variables, and re-initialize from saved values.
/// This implements per-iteration lexical scoping for `for (let ...)` loops.
fn emit_per_iteration_bindings(gen: &mut Generator, bindings: &[Vec<u16>]) {
    if bindings.is_empty() {
        return;
    }

    // Save current values into registers.
    let mut saved: Vec<(ScopedOperand, IdentifierTableIndex)> = Vec::with_capacity(bindings.len());
    for name in bindings {
        let id = gen.intern_identifier(name.clone());
        let reg = gen.allocate_register();
        gen.emit(Instruction::GetBinding {
            dst: reg.operand(),
            identifier: id,
            cache: EnvironmentCoordinate::empty(),
        });
        saved.push((reg, id));
    }

    // Pop current environment (end_variable_scope).
    gen.lexical_environment_register_stack.pop();
    let parent = gen.lexical_environment_register_stack.last().cloned()
        .unwrap_or_else(|| gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT)));
    gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });

    // Push new environment (begin_variable_scope).
    let new_env = gen.allocate_register();
    gen.emit(Instruction::CreateLexicalEnvironment {
        dst: new_env.operand(),
        parent: parent.operand(),
        capacity: bindings.len() as u32,
    });
    gen.lexical_environment_register_stack.push(new_env);

    // Re-create variables and initialize from saved values.
    for (reg, id) in &saved {
        gen.emit(Instruction::CreateVariable {
            identifier: *id,
            mode: ENV_MODE_LEXICAL,
            is_immutable: false,
            is_global: false,
            is_strict: false,
        });
        gen.emit(Instruction::InitializeLexicalBinding {
            identifier: *id,
            src: reg.operand(),
            cache: EnvironmentCoordinate::empty(),
        });
    }
}

// =============================================================================
// Scope children (Block, FunctionBody, Program)
// =============================================================================

fn generate_scope_children(
    gen: &mut Generator,
    scope: &ScopeData,
    _preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let mut last_result = None;
    for child in &scope.children {
        let result = generate_stmt(child, gen, None);
        if result.is_some() {
            last_result = result;
        }
        if gen.is_current_block_terminated() {
            break;
        }
    }
    last_result
}

/// Generate bytecode for a block statement, creating a lexical environment
/// if the block has non-local lexical declarations (let/const/class).
fn generate_block_statement(
    gen: &mut Generator,
    scope: &ScopeData,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let did_create_env = emit_block_declaration_instantiation(gen, scope);
    let result = generate_scope_children(gen, scope, preferred_dst);

    if did_create_env {
        gen.lexical_environment_register_stack.pop();
        if !gen.is_current_block_terminated() {
            let parent = gen.lexical_environment_register_stack.last().cloned()
                .unwrap_or_else(|| gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT)));
            gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
        }
    }
    result
}

/// Create a lexical environment for a block with non-local lexical declarations.
/// Returns true if an environment was created.
fn emit_block_declaration_instantiation(gen: &mut Generator, scope: &ScopeData) -> bool {
    if !has_non_local_lexical_decls(scope) {
        return false;
    }

    let parent = gen.lexical_environment_register_stack.last().cloned()
        .unwrap_or_else(|| gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT)));
    let new_env = gen.allocate_register();
    gen.emit(Instruction::CreateLexicalEnvironment {
        dst: new_env.operand(),
        parent: parent.operand(),
        capacity: 0,
    });
    gen.lexical_environment_register_stack.push(new_env);

    // Create bindings for non-local lexical declarations.
    for child in &scope.children {
        match &child.inner {
            Statement::VariableDeclaration { kind, declarations } => {
                if *kind == DeclarationKind::Let || *kind == DeclarationKind::Const {
                    let is_constant = *kind == DeclarationKind::Const;
                    for decl in declarations {
                        if let VariableDeclaratorTarget::Identifier(ident) = &decl.target {
                            if !ident.is_local() {
                                let id = gen.intern_identifier(ident.name.clone());
                                gen.emit(Instruction::CreateVariable {
                                    identifier: id,
                                    mode: ENV_MODE_LEXICAL,
                                    is_immutable: is_constant,
                                    is_global: false,
                                    is_strict: is_constant,
                                });
                            }
                        }
                    }
                }
            }
            Statement::ClassDeclaration(class_data) => {
                if let Some(ref name_ident) = class_data.name {
                    if !name_ident.is_local() {
                        let id = gen.intern_identifier(name_ident.name.clone());
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: ENV_MODE_LEXICAL,
                            is_immutable: false,
                            is_global: false,
                            is_strict: false,
                        });
                    }
                }
            }
            Statement::FunctionDeclaration(_) => {
                // Function declarations in blocks need block-scoped bindings too.
                // (Handled by FDI for the function body scope.)
            }
            _ => {}
        }
    }

    true
}

// =============================================================================
// Variable declaration
// =============================================================================

fn generate_variable_declaration(
    gen: &mut Generator,
    kind: DeclarationKind,
    declarations: &[VariableDeclarator],
) {
    for decl in declarations {
        // Set pending LHS name for function name inference.
        if let VariableDeclaratorTarget::Identifier(ident) = &decl.target {
            gen.pending_lhs_name = Some(gen.intern_identifier(ident.name.clone()));
        }
        let init_value = decl.init.as_ref().and_then(|init| generate_expr(init, gen, None));
        gen.pending_lhs_name = None;

        match &decl.target {
            VariableDeclaratorTarget::Identifier(ident) => {
                let value = init_value.unwrap_or_else(|| gen.add_constant_undefined());
                if ident.is_local() {
                    let local_index = ident.local_index.get();
                    let local = match ident.local_type.get() {
                        LocalType::Argument => gen.scoped_operand(Operand::argument(local_index)),
                        LocalType::Variable => gen.local(local_index),
                        LocalType::None => unreachable!(),
                    };
                    gen.emit_mov(&local, &value);
                    gen.mark_local_initialized(local_index);
                } else {
                    let id = gen.intern_identifier(ident.name.clone());
                    match kind {
                        DeclarationKind::Var => {
                            // Var declarations use Set mode (not Initialize) because
                            // FDI already initialized the binding.
                            gen.emit(Instruction::SetVariableBinding {
                                identifier: id,
                                src: value.operand(),
                                cache: EnvironmentCoordinate::empty(),
                            });
                        }
                        DeclarationKind::Let | DeclarationKind::Const => {
                            gen.emit(Instruction::InitializeLexicalBinding {
                                identifier: id,
                                src: value.operand(),
                                cache: EnvironmentCoordinate::empty(),
                            });
                        }
                    }
                }
            }
            VariableDeclaratorTarget::BindingPattern(pattern) => {
                if let Some(value) = init_value {
                    let mode = match kind {
                        DeclarationKind::Var => BindingMode::Set,
                        DeclarationKind::Let | DeclarationKind::Const => {
                            BindingMode::InitializeLexical
                        }
                    };
                    generate_binding_pattern_bytecode(gen, pattern, mode, &value);
                }
            }
        }
    }
}

// =============================================================================
// Call expression
// =============================================================================

fn generate_call_expression(
    gen: &mut Generator,
    data: &CallExpressionData,
    preferred_dst: Option<&ScopedOperand>,
    is_new: bool,
) -> Option<ScopedOperand> {
    let dst = choose_dst(gen, preferred_dst);

    // Compute expression_string for error messages (e.g. "true is not a function (evaluated from 'a')").
    let expression_string: Option<StringTableIndex> = match &data.callee.inner {
        Expression::Identifier(ident) => {
            Some(gen.intern_string(ident.name.clone()))
        }
        Expression::Member { object, property, computed } => {
            // Approximate the member expression as a string (e.g. "o.a", "o[key]").
            let mut s = expression_to_string_approximation(object);
            if *computed {
                s.extend_from_slice(utf16!("["));
                s.extend(expression_to_string_approximation(property));
                s.extend_from_slice(utf16!("]"));
            } else {
                s.extend_from_slice(utf16!("."));
                s.extend(expression_to_string_approximation(property));
            }
            Some(gen.intern_string(s))
        }
        _ => None,
    };

    // Detect direct eval calls: bare identifier "eval" as callee.
    let is_direct_eval = !is_new
        && matches!(&data.callee.inner, Expression::Identifier(ident) if ident.name == utf16!("eval"));

    // For method calls (obj.method()), we need to use the object as `this`.
    let (callee, this_value) = if !is_new {
        match &data.callee.inner {
            Expression::Member {
                object,
                property,
                computed,
            } => {
                let obj = generate_expr(object, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined());
                let method = gen.allocate_register();
                if *computed {
                    let prop = generate_expr(property, gen, None)
                        .unwrap_or_else(|| gen.add_constant_undefined());
                    gen.emit(Instruction::GetByValue {
                        dst: method.operand(),
                        base: obj.operand(),
                        property: prop.operand(),
                        base_identifier: None,
                    });
                } else if let Expression::Identifier(ident) = &property.inner {
                    emit_get_by_id(gen, &method, &obj, &ident.name, None);
                }
                (method, Some(obj))
            }
            Expression::Identifier(ident) if !ident.is_local() && !ident.is_global.get() => {
                // Non-local, non-global identifier: use GetCalleeAndThisFromEnvironment
                // to properly handle with-statement bindings and eval.
                let callee_reg = gen.allocate_register();
                let this_reg = gen.allocate_register();
                let id = gen.intern_identifier(ident.name.clone());
                gen.emit(Instruction::GetCalleeAndThisFromEnvironment {
                    callee: callee_reg.operand(),
                    this_value: this_reg.operand(),
                    identifier: id,
                    cache: EnvironmentCoordinate::empty(),
                });
                (callee_reg, Some(this_reg))
            }
            _ => {
                let callee = generate_expr(&data.callee, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined());
                (callee, None)
            }
        }
    } else {
        let callee = generate_expr(&data.callee, gen, None)
            .unwrap_or_else(|| gen.add_constant_undefined());
        (callee, None)
    };

    let has_spread = data.arguments.iter().any(|a| a.is_spread);

    if has_spread {
        // Build an arguments array using NewArray + ArrayAppend for spread elements.
        let args_array = gen.allocate_register();
        let first_spread = data.arguments.iter().position(|a| a.is_spread).unwrap_or(0);

        let mut pre_holders = Vec::new();
        for arg in &data.arguments[..first_spread] {
            let val = generate_expr(&arg.value, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            pre_holders.push(val);
        }
        let pre_args: Vec<Operand> = pre_holders.iter().map(|a| a.operand()).collect();
        gen.emit(Instruction::NewArray {
            dst: args_array.operand(),
            element_count: pre_args.len() as u32,
            elements: pre_args,
        });
        drop(pre_holders);

        for arg in &data.arguments[first_spread..] {
            let val = generate_expr(&arg.value, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::ArrayAppend {
                dst: args_array.operand(),
                src: val.operand(),
                is_spread: arg.is_spread,
            });
        }

        if is_new {
            let this_op = this_value.unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::CallConstructWithArgumentArray {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_op.operand(),
                arguments: args_array.operand(),
                expression_string,
            });
        } else if is_direct_eval {
            let this_op = this_value.unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::CallDirectEvalWithArgumentArray {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_op.operand(),
                arguments: args_array.operand(),
                expression_string,
            });
        } else {
            let this_op = this_value.unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::CallWithArgumentArray {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_op.operand(),
                arguments: args_array.operand(),
                expression_string,
            });
        }
    } else {
        // Keep ScopedOperands alive until the Call instruction is emitted,
        // so argument registers don't get freed and reused between evaluations.
        let mut arg_holders = Vec::new();
        for arg in &data.arguments {
            let val = generate_expr(&arg.value, gen, None).unwrap_or_else(|| gen.add_constant_undefined());
            arg_holders.push(val);
        }
        let args: Vec<Operand> = arg_holders.iter().map(|a| a.operand()).collect();

        if is_new {
            gen.emit(Instruction::CallConstruct {
                dst: dst.operand(),
                callee: callee.operand(),
                argument_count: args.len() as u32,
                expression_string,
                arguments: args,
            });
        } else if is_direct_eval {
            let this_op = this_value.unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::CallDirectEval {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_op.operand(),
                argument_count: args.len() as u32,
                expression_string,
                arguments: args,
            });
        } else {
            let this_op = this_value.unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::Call {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_op.operand(),
                argument_count: args.len() as u32,
                expression_string,
                arguments: args,
            });
        }
    }

    Some(dst)
}

// =============================================================================
// Update expression (++/--)
// =============================================================================

fn generate_update_expression(
    gen: &mut Generator,
    op: UpdateOp,
    argument: &Expr,
    prefixed: bool,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    // Load the value, keeping track of the base for member expressions
    // so we can store back without re-evaluating.
    match &argument.inner {
        Expression::Identifier(ident) => {
            let value = generate_identifier(ident, gen, None)?;
            if prefixed {
                match op {
                    UpdateOp::Increment => gen.emit(Instruction::Increment { dst: value.operand() }),
                    UpdateOp::Decrement => gen.emit(Instruction::Decrement { dst: value.operand() }),
                }
                emit_set_variable(gen, ident, &value);
                Some(value)
            } else {
                let dst = choose_dst(gen, preferred_dst);
                match op {
                    UpdateOp::Increment => gen.emit(Instruction::PostfixIncrement {
                        dst: dst.operand(),
                        src: value.operand(),
                    }),
                    UpdateOp::Decrement => gen.emit(Instruction::PostfixDecrement {
                        dst: dst.operand(),
                        src: value.operand(),
                    }),
                }
                emit_set_variable(gen, ident, &value);
                Some(dst)
            }
        }
        Expression::Member { object, property, computed } => {
            let base = generate_expr(object, gen, None)?;
            let value = gen.allocate_register();
            // Load the member value
            if *computed {
                let prop = generate_expr(property, gen, None)?;
                gen.emit(Instruction::GetByValue {
                    dst: value.operand(),
                    base: base.operand(),
                    property: prop.operand(),
                    base_identifier: None,
                });
                if prefixed {
                    match op {
                        UpdateOp::Increment => gen.emit(Instruction::Increment { dst: value.operand() }),
                        UpdateOp::Decrement => gen.emit(Instruction::Decrement { dst: value.operand() }),
                    }
                    gen.emit(Instruction::PutNormalByValue {
                        base: base.operand(),
                        property: prop.operand(),
                        src: value.operand(),
                        base_identifier: None,
                    });
                    Some(value)
                } else {
                    let dst = choose_dst(gen, preferred_dst);
                    match op {
                        UpdateOp::Increment => gen.emit(Instruction::PostfixIncrement {
                            dst: dst.operand(),
                            src: value.operand(),
                        }),
                        UpdateOp::Decrement => gen.emit(Instruction::PostfixDecrement {
                            dst: dst.operand(),
                            src: value.operand(),
                        }),
                    }
                    gen.emit(Instruction::PutNormalByValue {
                        base: base.operand(),
                        property: prop.operand(),
                        src: value.operand(),
                        base_identifier: None,
                    });
                    Some(dst)
                }
            } else if let Expression::Identifier(prop_ident) = &property.inner {
                emit_get_by_id(gen, &value, &base, &prop_ident.name, None);
                let key = gen.intern_property_key(prop_ident.name.clone());
                if prefixed {
                    match op {
                        UpdateOp::Increment => gen.emit(Instruction::Increment { dst: value.operand() }),
                        UpdateOp::Decrement => gen.emit(Instruction::Decrement { dst: value.operand() }),
                    }
                } else {
                    let dst = choose_dst(gen, preferred_dst);
                    match op {
                        UpdateOp::Increment => gen.emit(Instruction::PostfixIncrement {
                            dst: dst.operand(),
                            src: value.operand(),
                        }),
                        UpdateOp::Decrement => gen.emit(Instruction::PostfixDecrement {
                            dst: dst.operand(),
                            src: value.operand(),
                        }),
                    }
                    let cache2 = gen.next_property_lookup_cache();
                    gen.emit(Instruction::PutNormalById {
                        base: base.operand(),
                        property: key,
                        src: value.operand(),
                        cache_index: cache2,
                        base_identifier: None,
                    });
                    return Some(dst);
                }
                let cache2 = gen.next_property_lookup_cache();
                gen.emit(Instruction::PutNormalById {
                    base: base.operand(),
                    property: key,
                    src: value.operand(),
                    cache_index: cache2,
                    base_identifier: None,
                });
                Some(value)
            } else {
                // Fallback: just evaluate, no store-back
                Some(value)
            }
        }
        _ => {
            // Fallback for other expressions (shouldn't normally happen)
            let value = generate_expr(argument, gen, None)?;
            if prefixed {
                match op {
                    UpdateOp::Increment => gen.emit(Instruction::Increment { dst: value.operand() }),
                    UpdateOp::Decrement => gen.emit(Instruction::Decrement { dst: value.operand() }),
                }
                Some(value)
            } else {
                let dst = choose_dst(gen, preferred_dst);
                match op {
                    UpdateOp::Increment => gen.emit(Instruction::PostfixIncrement {
                        dst: dst.operand(),
                        src: value.operand(),
                    }),
                    UpdateOp::Decrement => gen.emit(Instruction::PostfixDecrement {
                        dst: dst.operand(),
                        src: value.operand(),
                    }),
                }
                Some(dst)
            }
        }
    }
}

// =============================================================================
// Assignment expression
// =============================================================================

fn generate_assignment_expression(
    gen: &mut Generator,
    op: AssignmentOp,
    lhs: &AssignmentLhs,
    rhs: &Expr,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    match lhs {
        AssignmentLhs::Expression(lhs_expr) => {
            // Simple assignment to identifier
            if let Expression::Identifier(ident) = &lhs_expr.inner {
                if op == AssignmentOp::Assignment {
                    gen.pending_lhs_name = Some(gen.intern_identifier(ident.name.clone()));
                }
                let rhs_val = generate_expr(rhs, gen, preferred_dst)?;
                gen.pending_lhs_name = None;
                if op == AssignmentOp::Assignment {
                    emit_set_variable(gen, ident, &rhs_val);
                    return Some(rhs_val);
                }
                // Compound assignment
                let lhs_val = generate_identifier(ident, gen, None)?;
                let dst = choose_dst(gen, preferred_dst);
                emit_compound_assignment(gen, op, &dst, &lhs_val, &rhs_val);
                emit_set_variable(gen, ident, &dst);
                return Some(dst);
            }
            // Member expression LHS (e.g., obj.foo = x, obj[key] = x)
            if let Expression::Member { object, property, computed } = &lhs_expr.inner {
                let base = generate_expr(object, gen, None)?;
                if op == AssignmentOp::Assignment {
                    let rhs_val = generate_expr(rhs, gen, preferred_dst)?;
                    emit_put_to_member(gen, &base, property, *computed, &rhs_val);
                    return Some(rhs_val);
                }
                // Compound member assignment
                let old_val = gen.allocate_register();
                if *computed {
                    let prop = generate_expr(property, gen, None)?;
                    gen.emit(Instruction::GetByValue {
                        dst: old_val.operand(),
                        base: base.operand(),
                        property: prop.operand(),
                        base_identifier: None,
                    });
                    let rhs_val = generate_expr(rhs, gen, None)?;
                    let dst = choose_dst(gen, preferred_dst);
                    emit_compound_assignment(gen, op, &dst, &old_val, &rhs_val);
                    gen.emit(Instruction::PutNormalByValue {
                        base: base.operand(),
                        property: prop.operand(),
                        src: dst.operand(),
                        base_identifier: None,
                    });
                    return Some(dst);
                } else {
                    if let Expression::Identifier(ident) = &property.inner {
                        emit_get_by_id(gen, &old_val, &base, &ident.name, None);
                        let rhs_val = generate_expr(rhs, gen, None)?;
                        let dst = choose_dst(gen, preferred_dst);
                        emit_compound_assignment(gen, op, &dst, &old_val, &rhs_val);
                        let key = gen.intern_property_key(ident.name.clone());
                        let cache2 = gen.next_property_lookup_cache();
                        gen.emit(Instruction::PutNormalById {
                            base: base.operand(),
                            property: key,
                            src: dst.operand(),
                            cache_index: cache2,
                            base_identifier: None,
                        });
                        return Some(dst);
                    }
                }
            }
            // Fallback: just evaluate RHS
            let rhs_val = generate_expr(rhs, gen, preferred_dst)?;
            Some(rhs_val)
        }
        AssignmentLhs::Pattern(pattern) => {
            let rhs_val = generate_expr(rhs, gen, preferred_dst)?;
            generate_binding_pattern_bytecode(gen, pattern, BindingMode::Set, &rhs_val);
            Some(rhs_val)
        }
    }
}

/// Emit a property access by name, using GetLength for the "length" property.
fn emit_get_by_id(
    gen: &mut Generator,
    dst: &ScopedOperand,
    base: &ScopedOperand,
    property_name: &[u16],
    base_identifier: Option<IdentifierTableIndex>,
) {
    let key = gen.intern_property_key(property_name.to_vec());
    if property_name == utf16!("length") {
        gen.length_identifier = Some(key);
        let cache = gen.next_property_lookup_cache();
        gen.emit(Instruction::GetLength {
            dst: dst.operand(),
            base: base.operand(),
            base_identifier,
            cache_index: cache,
        });
    } else {
        let cache = gen.next_property_lookup_cache();
        gen.emit(Instruction::GetById {
            dst: dst.operand(),
            base: base.operand(),
            property: key,
            base_identifier,
            cache_index: cache,
        });
    }
}

fn emit_set_variable(gen: &mut Generator, ident: &Identifier, value: &ScopedOperand) {
    if ident.is_local() {
        if ident.declaration_kind.get() == IdentDeclarationKind::Const {
            gen.emit(Instruction::ThrowConstAssignment {});
            return;
        }
        let local_index = ident.local_index.get();
        let local = match ident.local_type.get() {
            LocalType::Argument => gen.scoped_operand(Operand::argument(local_index)),
            LocalType::Variable => gen.local(local_index),
            LocalType::None => unreachable!(),
        };
        gen.emit_mov(&local, value);
    } else if ident.is_global.get() {
        let id = gen.intern_identifier(ident.name.clone());
        let cache = gen.next_global_variable_cache();
        gen.emit(Instruction::SetGlobal {
            identifier: id,
            src: value.operand(),
            cache_index: cache,
        });
    } else {
        // Non-local, non-global: use SetLexicalBinding which searches
        // the lexical environment chain (important for with-statement support).
        let id = gen.intern_identifier(ident.name.clone());
        gen.emit(Instruction::SetLexicalBinding {
            identifier: id,
            src: value.operand(),
            cache: EnvironmentCoordinate::empty(),
        });
    }
}

fn emit_put_to_member(
    gen: &mut Generator,
    base: &ScopedOperand,
    property: &Expr,
    computed: bool,
    value: &ScopedOperand,
) {
    if computed {
        let prop = generate_expr(property, gen, None)
            .unwrap_or_else(|| gen.add_constant_undefined());
        gen.emit(Instruction::PutNormalByValue {
            base: base.operand(),
            property: prop.operand(),
            src: value.operand(),
            base_identifier: None,
        });
    } else if let Expression::Identifier(ident) = &property.inner {
        let key = gen.intern_property_key(ident.name.clone());
        let cache = gen.next_property_lookup_cache();
        gen.emit(Instruction::PutNormalById {
            base: base.operand(),
            property: key,
            src: value.operand(),
            cache_index: cache,
            base_identifier: None,
        });
    } else if let Expression::PrivateIdentifier(priv_ident) = &property.inner {
        let id = gen.intern_identifier(priv_ident.name.clone());
        gen.emit(Instruction::PutPrivateById {
            base: base.operand(),
            property: id,
            src: value.operand(),
        });
    }
}

/// Emit bytecode for `delete <expression>`.
fn emit_delete_reference(
    gen: &mut Generator,
    operand: &Expr,
    preferred_dst: Option<&ScopedOperand>,
) -> ScopedOperand {
    match &operand.inner {
        Expression::Identifier(ident) => {
            if ident.is_local() {
                return gen.add_constant_boolean(false);
            }
            let dst = choose_dst(gen, preferred_dst);
            let id = gen.intern_identifier(ident.name.clone());
            gen.emit(Instruction::DeleteVariable {
                dst: dst.operand(),
                identifier: id,
            });
            dst
        }
        Expression::Member { object, property, computed } => {
            let base = generate_expr(object, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            let dst = choose_dst(gen, preferred_dst);
            if *computed {
                let key = generate_expr(property, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined());
                gen.emit(Instruction::DeleteByValue {
                    dst: dst.operand(),
                    base: base.operand(),
                    property: key.operand(),
                });
            } else if let Expression::Identifier(prop_ident) = &property.inner {
                let key = gen.intern_property_key(prop_ident.name.clone());
                gen.emit(Instruction::DeleteById {
                    dst: dst.operand(),
                    base: base.operand(),
                    property: key,
                });
            } else {
                return gen.add_constant_boolean(true);
            }
            dst
        }
        _ => {
            // delete on non-reference: evaluate for side effects, return true
            generate_expr(operand, gen, None);
            gen.add_constant_boolean(true)
        }
    }
}

fn emit_store_to_reference(
    gen: &mut Generator,
    target: &Expr,
    value: &ScopedOperand,
) {
    match &target.inner {
        Expression::Identifier(ident) => {
            emit_set_variable(gen, ident, value);
        }
        Expression::Member { object, property, computed } => {
            let base = generate_expr(object, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            emit_put_to_member(gen, &base, property, *computed, value);
        }
        _ => {}
    }
}

fn emit_compound_assignment(
    gen: &mut Generator,
    op: AssignmentOp,
    dst: &ScopedOperand,
    lhs: &ScopedOperand,
    rhs: &ScopedOperand,
) {
    let d = dst.operand();
    let l = lhs.operand();
    let r = rhs.operand();
    match op {
        AssignmentOp::AdditionAssignment => gen.emit(Instruction::Add { dst: d, lhs: l, rhs: r }),
        AssignmentOp::SubtractionAssignment => gen.emit(Instruction::Sub { dst: d, lhs: l, rhs: r }),
        AssignmentOp::MultiplicationAssignment => gen.emit(Instruction::Mul { dst: d, lhs: l, rhs: r }),
        AssignmentOp::DivisionAssignment => gen.emit(Instruction::Div { dst: d, lhs: l, rhs: r }),
        AssignmentOp::ModuloAssignment => gen.emit(Instruction::Mod { dst: d, lhs: l, rhs: r }),
        AssignmentOp::ExponentiationAssignment => gen.emit(Instruction::Exp { dst: d, lhs: l, rhs: r }),
        AssignmentOp::BitwiseAndAssignment => gen.emit(Instruction::BitwiseAnd { dst: d, lhs: l, rhs: r }),
        AssignmentOp::BitwiseOrAssignment => gen.emit(Instruction::BitwiseOr { dst: d, lhs: l, rhs: r }),
        AssignmentOp::BitwiseXorAssignment => gen.emit(Instruction::BitwiseXor { dst: d, lhs: l, rhs: r }),
        AssignmentOp::LeftShiftAssignment => gen.emit(Instruction::LeftShift { dst: d, lhs: l, rhs: r }),
        AssignmentOp::RightShiftAssignment => gen.emit(Instruction::RightShift { dst: d, lhs: l, rhs: r }),
        AssignmentOp::UnsignedRightShiftAssignment => gen.emit(Instruction::UnsignedRightShift { dst: d, lhs: l, rhs: r }),
        // Logical assignments (these shouldn't reach here, handled separately)
        AssignmentOp::AndAssignment | AssignmentOp::OrAssignment | AssignmentOp::NullishAssignment => {}
        AssignmentOp::Assignment => unreachable!("plain assignment in compound path"),
    }
}

// =============================================================================
// Template literal
// =============================================================================

fn generate_template_literal(
    gen: &mut Generator,
    data: &TemplateLiteralData,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    // The parser stores ALL parts (string segments AND interpolated expressions)
    // in data.expressions. raw_strings is only populated for tagged templates.
    if data.expressions.is_empty() {
        return Some(gen.add_constant_string(Vec::new()));
    }

    if data.expressions.len() == 1 {
        if let Expression::StringLiteral(s) = &data.expressions[0].inner {
            return Some(gen.add_constant_string(s.clone()));
        }
    }

    let dst = choose_dst(gen, preferred_dst);
    let mut first = true;
    for expr in &data.expressions {
        let val = generate_expr(expr, gen, None).unwrap_or_else(|| gen.add_constant_undefined());
        if first {
            gen.emit_mov(&dst, &val);
            first = false;
        } else {
            gen.emit(Instruction::ConcatString {
                dst: dst.operand(),
                src: val.operand(),
            });
        }
    }

    Some(dst)
}

// =============================================================================
// Tagged template literal
// =============================================================================

fn generate_tagged_template_literal(
    gen: &mut Generator,
    tag: &Expr,
    template_literal: &Expr,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    // Resolve tag and this_value based on the tag expression type.
    let (tag_reg, this_value) = match &tag.inner {
        Expression::Member { object, property, computed } => {
            let obj = generate_expr(object, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            let method = gen.allocate_register();
            if *computed {
                let prop = generate_expr(property, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined());
                gen.emit(Instruction::GetByValue {
                    dst: method.operand(),
                    base: obj.operand(),
                    property: prop.operand(),
                    base_identifier: None,
                });
            } else if let Expression::Identifier(ident) = &property.inner {
                emit_get_by_id(gen, &method, &obj, &ident.name, None);
            }
            (method, Some(obj))
        }
        Expression::Identifier(ident) if ident.is_local() || ident.is_global.get() => {
            let tag_val = generate_expr(tag, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            (tag_val, None)
        }
        Expression::Identifier(ident) => {
            // Non-local, non-global identifier: use GetCalleeAndThisFromEnvironment
            // to properly handle with-statement bindings.
            let callee_reg = gen.allocate_register();
            let this_reg = gen.allocate_register();
            let id = gen.intern_identifier(ident.name.clone());
            gen.emit(Instruction::GetCalleeAndThisFromEnvironment {
                callee: callee_reg.operand(),
                this_value: this_reg.operand(),
                identifier: id,
                cache: EnvironmentCoordinate::empty(),
            });
            (callee_reg, Some(this_reg))
        }
        _ => {
            let tag_val = generate_expr(tag, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            (tag_val, None)
        }
    };

    // Build template strings for GetTemplateObject.
    // expressions has alternating: string_0, expr_0, string_1, expr_1, ..., string_n
    let data = match &template_literal.inner {
        Expression::TemplateLiteral(d) => d,
        _ => unreachable!("TaggedTemplateLiteral template must be TemplateLiteral"),
    };

    // Collect cooked strings (even indices). NullLiteral means invalid escape → undefined.
    let mut string_regs = Vec::new();
    for i in (0..data.expressions.len()).step_by(2) {
        if matches!(&data.expressions[i].inner, Expression::NullLiteral) {
            string_regs.push(gen.add_constant_undefined());
        } else {
            let val = generate_expr(&data.expressions[i], gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            string_regs.push(val);
        }
    }

    // Append raw strings.
    for raw in &data.raw_strings {
        let val = gen.add_constant_string(raw.clone());
        string_regs.push(val);
    }

    // Emit GetTemplateObject.
    let strings_array = gen.allocate_register();
    let string_ops: Vec<Operand> = string_regs.iter().map(|s| s.operand()).collect();
    let cache_index = gen.next_template_object_cache();
    gen.emit(Instruction::GetTemplateObject {
        dst: strings_array.operand(),
        strings_count: string_ops.len() as u32,
        cache_index,
        strings: string_ops,
    });

    // Build arguments: [template_object, ...interpolated_expressions]
    let mut arg_regs = vec![strings_array];
    for i in (1..data.expressions.len()).step_by(2) {
        let val = generate_expr(&data.expressions[i], gen, None)
            .unwrap_or_else(|| gen.add_constant_undefined());
        arg_regs.push(val);
    }

    let dst = choose_dst(gen, preferred_dst);
    let this_op = this_value.unwrap_or_else(|| gen.add_constant_undefined());
    let args: Vec<Operand> = arg_regs.iter().map(|a| a.operand()).collect();
    gen.emit(Instruction::Call {
        dst: dst.operand(),
        callee: tag_reg.operand(),
        this_value: this_op.operand(),
        argument_count: args.len() as u32,
        expression_string: None,
        arguments: args,
    });

    Some(dst)
}

// =============================================================================
// Switch statement
// =============================================================================

fn generate_switch_statement(
    gen: &mut Generator,
    data: &SwitchStatementData,
    _preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let discriminant = generate_expr(&data.discriminant, gen, None)?;
    let end_block = gen.make_block();
    gen.begin_breakable_scope(Label(end_block as u32), Vec::new());

    // Create blocks for each case
    let case_blocks: Vec<usize> = data.cases.iter().map(|_| gen.make_block()).collect();

    // Find default block first (it may appear before or after other cases).
    let default_block = data.cases.iter().enumerate()
        .find(|(_, c)| c.test.is_none())
        .map(|(i, _)| case_blocks[i]);
    let fallthrough_target = default_block.unwrap_or(end_block);

    // Emit comparison chain
    for (i, case) in data.cases.iter().enumerate() {
        if let Some(test) = &case.test {
            let test_val = generate_expr(test, gen, None)?;
            let cmp = gen.allocate_register();
            gen.emit(Instruction::StrictlyEquals {
                dst: cmp.operand(),
                lhs: discriminant.operand(),
                rhs: test_val.operand(),
            });
            let next_check = gen.make_block();
            gen.emit(Instruction::JumpIf {
                condition: cmp.operand(),
                true_target: Label(case_blocks[i] as u32),
                false_target: Label(next_check as u32),
            });
            gen.switch_to_basic_block(next_check);
        }
    }

    // After all comparisons fail, jump to default or end.
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(fallthrough_target as u32),
        });
    }

    // Emit case bodies (fall-through by default)
    for (i, case) in data.cases.iter().enumerate() {
        gen.switch_to_basic_block(case_blocks[i]);
        for child in &case.scope.children {
            generate_stmt(child, gen, None);
            if gen.is_current_block_terminated() {
                break;
            }
        }
        // Fall through to next case
        if !gen.is_current_block_terminated() && i + 1 < case_blocks.len() {
            gen.emit(Instruction::Jump {
                target: Label(case_blocks[i + 1] as u32),
            });
        } else if !gen.is_current_block_terminated() {
            gen.emit(Instruction::Jump {
                target: Label(end_block as u32),
            });
        }
    }

    gen.end_breakable_scope();
    gen.switch_to_basic_block(end_block);
    None
}

// =============================================================================
// Try statement
// =============================================================================

// =============================================================================
// Object expression
// =============================================================================

fn generate_object_expression(
    gen: &mut Generator,
    properties: &[ObjectProperty],
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let dst = choose_dst(gen, preferred_dst);
    let cache_index = gen.next_object_shape_cache();
    gen.emit(Instruction::NewObject {
        dst: dst.operand(),
        cache_index,
    });

    if properties.is_empty() {
        return Some(dst);
    }

    for (slot, prop) in properties.iter().enumerate() {
        match prop.property_type {
            ObjectPropertyType::Spread => {
                // For spread, the source expression is in `key`, not `value`.
                let src = generate_expr(&prop.key, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined());
                gen.emit(Instruction::PutBySpread {
                    base: dst.operand(),
                    src: src.operand(),
                });
                continue;
            }
            _ => {}
        }

        // For computed keys, evaluate key before value (spec evaluation order).
        // ComputedPropertyName calls ToPropertyKey, which includes ToPrimitive(hint: string).
        // The ToPrimitive is the only user-observable step; after this, the ToPrimitive
        // inside PutOwnByValue's to_property_key is a no-op.
        let computed_key = if prop.is_computed {
            let key = generate_expr(&prop.key, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::ToPrimitiveWithStringHint {
                dst: key.operand(),
                value: key.operand(),
            });
            Some(key)
        } else {
            None
        };

        // Set pending LHS name for function name inference on non-computed properties.
        if !prop.is_computed && prop.property_type == ObjectPropertyType::KeyValue {
            if let Expression::StringLiteral(s) = &prop.key.inner {
                gen.pending_lhs_name = Some(gen.intern_identifier(s.clone()));
            } else if let Expression::Identifier(ident) = &prop.key.inner {
                gen.pending_lhs_name = Some(gen.intern_identifier(ident.name.clone()));
            }
        }
        let value = prop.value.as_ref().and_then(|v| generate_expr(v, gen, None))
            .unwrap_or_else(|| gen.add_constant_undefined());
        gen.pending_lhs_name = None;

        match prop.property_type {
            ObjectPropertyType::Spread => unreachable!(),
            ObjectPropertyType::KeyValue => {
                if let Some(key_val) = &computed_key {
                    gen.emit(Instruction::PutOwnByValue {
                        base: dst.operand(),
                        property: key_val.operand(),
                        src: value.operand(),
                        base_identifier: None,
                    });
                } else {
                    emit_object_property_set_by_key(gen, &dst, &prop.key, &value, slot as u32, cache_index, false);
                }
            }
            ObjectPropertyType::Getter => {
                if let Some(key_val) = &computed_key {
                    gen.emit(Instruction::PutGetterByValue {
                        base: dst.operand(),
                        property: key_val.operand(),
                        src: value.operand(),
                        base_identifier: None,
                    });
                } else {
                    emit_object_accessor_by_key(gen, &dst, &prop.key, &value, true, false);
                }
            }
            ObjectPropertyType::Setter => {
                if let Some(key_val) = &computed_key {
                    gen.emit(Instruction::PutSetterByValue {
                        base: dst.operand(),
                        property: key_val.operand(),
                        src: value.operand(),
                        base_identifier: None,
                    });
                } else {
                    emit_object_accessor_by_key(gen, &dst, &prop.key, &value, false, false);
                }
            }
            ObjectPropertyType::ProtoSetter => {
                let key = gen.intern_property_key(utf16!("__proto__").to_vec());
                let cache = gen.next_property_lookup_cache();
                gen.emit(Instruction::PutPrototypeById {
                    base: dst.operand(),
                    property: key,
                    src: value.operand(),
                    cache_index: cache,
                    base_identifier: None,
                });
            }
        }
    }

    Some(dst)
}

/// Emit a property set for an object literal key (static or computed).
fn emit_object_property_set_by_key(
    gen: &mut Generator,
    object: &ScopedOperand,
    key: &Expr,
    value: &ScopedOperand,
    slot: u32,
    cache_index: u32,
    is_computed: bool,
) {
    if is_computed {
        let key_val = generate_expr(key, gen, None)
            .unwrap_or_else(|| gen.add_constant_undefined());
        gen.emit(Instruction::PutOwnByValue {
            base: object.operand(),
            property: key_val.operand(),
            src: value.operand(),
            base_identifier: None,
        });
        return;
    }
    match &key.inner {
        Expression::Identifier(ident) => {
            let prop_key = gen.intern_property_key(ident.name.clone());
            gen.emit(Instruction::InitObjectLiteralProperty {
                object: object.operand(),
                property: prop_key,
                src: value.operand(),
                shape_cache_index: cache_index,
                property_slot: slot,
            });
        }
        Expression::StringLiteral(s) => {
            let prop_key = gen.intern_property_key(s.clone());
            gen.emit(Instruction::InitObjectLiteralProperty {
                object: object.operand(),
                property: prop_key,
                src: value.operand(),
                shape_cache_index: cache_index,
                property_slot: slot,
            });
        }
        Expression::NumericLiteral(n) => {
            let key_val = gen.add_constant_number(*n);
            gen.emit(Instruction::PutOwnByValue {
                base: object.operand(),
                property: key_val.operand(),
                src: value.operand(),
                base_identifier: None,
            });
        }
        _ => {
            // Computed key
            let key_val = generate_expr(key, gen, None)
                .unwrap_or_else(|| gen.add_constant_undefined());
            gen.emit(Instruction::PutOwnByValue {
                base: object.operand(),
                property: key_val.operand(),
                src: value.operand(),
                base_identifier: None,
            });
        }
    }
}

/// Emit a getter/setter for an object literal key.
fn emit_object_accessor_by_key(
    gen: &mut Generator,
    object: &ScopedOperand,
    key: &Expr,
    value: &ScopedOperand,
    is_getter: bool,
    is_computed: bool,
) {
    let emit_by_id = |gen: &mut Generator, name: &[u16]| {
        let prop_key = gen.intern_property_key(name.to_vec());
        let cache = gen.next_property_lookup_cache();
        if is_getter {
            gen.emit(Instruction::PutGetterById {
                base: object.operand(),
                property: prop_key,
                src: value.operand(),
                cache_index: cache,
                base_identifier: None,
            });
        } else {
            gen.emit(Instruction::PutSetterById {
                base: object.operand(),
                property: prop_key,
                src: value.operand(),
                cache_index: cache,
                base_identifier: None,
            });
        }
    };

    let emit_by_value = |gen: &mut Generator, key: &Expr| {
        let key_val = generate_expr(key, gen, None)
            .unwrap_or_else(|| gen.add_constant_undefined());
        if is_getter {
            gen.emit(Instruction::PutGetterByValue {
                base: object.operand(),
                property: key_val.operand(),
                src: value.operand(),
                base_identifier: None,
            });
        } else {
            gen.emit(Instruction::PutSetterByValue {
                base: object.operand(),
                property: key_val.operand(),
                src: value.operand(),
                base_identifier: None,
            });
        }
    };

    if is_computed {
        emit_by_value(gen, key);
        return;
    }

    match &key.inner {
        Expression::Identifier(ident) => emit_by_id(gen, &ident.name),
        Expression::StringLiteral(s) => emit_by_id(gen, s),
        _ => emit_by_value(gen, key),
    }
}

// =============================================================================
// Optional chain
// =============================================================================

fn generate_optional_chain(
    gen: &mut Generator,
    base: &Expr,
    references: &[OptionalChainReference],
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let end_block = gen.make_block();
    let dst = choose_dst(gen, preferred_dst);
    let undef = gen.add_constant_undefined();

    let mut current = generate_expr(base, gen, None)?;

    for reference in references {
        let is_optional = match reference {
            OptionalChainReference::Call { mode, .. }
            | OptionalChainReference::ComputedReference { mode, .. }
            | OptionalChainReference::MemberReference { mode, .. }
            | OptionalChainReference::PrivateMemberReference { mode, .. } => {
                *mode == OptionalChainMode::Optional
            }
        };

        if is_optional {
            let continue_block = gen.make_block();
            let short_circuit_block = gen.make_block();
            gen.emit(Instruction::JumpNullish {
                condition: current.operand(),
                true_target: Label(short_circuit_block as u32),
                false_target: Label(continue_block as u32),
            });
            gen.switch_to_basic_block(short_circuit_block);
            gen.emit_mov(&dst, &undef);
            gen.emit(Instruction::Jump {
                target: Label(end_block as u32),
            });
            gen.switch_to_basic_block(continue_block);
        }

        match reference {
            OptionalChainReference::MemberReference { identifier, .. } => {
                let next = gen.allocate_register();
                emit_get_by_id(gen, &next, &current, &identifier.name, None);
                current = next;
            }
            OptionalChainReference::ComputedReference { expression, .. } => {
                let next = gen.allocate_register();
                let prop = generate_expr(expression, gen, None)?;
                gen.emit(Instruction::GetByValue {
                    dst: next.operand(),
                    base: current.operand(),
                    property: prop.operand(),
                    base_identifier: None,
                });
                current = next;
            }
            OptionalChainReference::Call { arguments, .. } => {
                let next = gen.allocate_register();
                let mut arg_holders = Vec::new();
                for arg in arguments {
                    let val = generate_expr(&arg.value, gen, None)
                        .unwrap_or_else(|| gen.add_constant_undefined());
                    arg_holders.push(val);
                }
                let arg_ops: Vec<Operand> = arg_holders.iter().map(|a| a.operand()).collect();
                let this_value = gen.add_constant_undefined();
                gen.emit(Instruction::Call {
                    dst: next.operand(),
                    callee: current.operand(),
                    this_value: this_value.operand(),
                    argument_count: arg_ops.len() as u32,
                    expression_string: None,
                    arguments: arg_ops,
                });
                current = next;
            }
            OptionalChainReference::PrivateMemberReference { private_identifier, .. } => {
                let next = gen.allocate_register();
                let id = gen.intern_identifier(private_identifier.name.clone());
                gen.emit(Instruction::GetPrivateById {
                    dst: next.operand(),
                    base: current.operand(),
                    property: id,
                });
                current = next;
            }
        }
    }

    gen.emit_mov(&dst, &current);
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(end_block as u32),
        });
    }

    gen.switch_to_basic_block(end_block);
    Some(dst)
}

// =============================================================================
// Class expression
// =============================================================================

fn generate_class_expression(
    gen: &mut Generator,
    data: &ClassData,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let dst = choose_dst(gen, preferred_dst);
    let has_super = data.super_class.is_some();

    // Step 2: Save parent environment, create class lexical environment.
    let parent_env = gen.allocate_register();
    gen.emit(Instruction::GetLexicalEnvironment {
        dst: parent_env.operand(),
    });
    let class_env = gen.allocate_register();
    gen.emit(Instruction::CreateLexicalEnvironment {
        dst: class_env.operand(),
        parent: parent_env.operand(),
        capacity: 0,
    });

    // Step 3.a: Create binding for the class name in the class environment.
    // For named classes, this is the class name (e.g. "A" in "class A {}").
    // For anonymous classes without lhs_name, create binding for empty name.
    // FIXME: When lhs_name support is added, only create for named classes
    //        or classes without lhs_name (matching C++ logic).
    {
        let name = if let Some(name_ident) = &data.name {
            name_ident.name.clone()
        } else {
            Vec::new()
        };
        let name_id = gen.intern_identifier(name);
        gen.emit(Instruction::CreateVariable {
            identifier: name_id,
            mode: 0, // Lexical
            is_immutable: true,
            is_global: false,
            is_strict: false,
        });
    }

    // Evaluate super class if present
    let super_class = if let Some(super_expr) = &data.super_class {
        generate_expr(super_expr, gen, None)
    } else {
        None
    };

    // Create private environment for private class elements.
    let mut has_private_env = false;
    for elem_node in &data.elements {
        let priv_name = match &elem_node.inner {
            ClassElement::Method { key, .. } | ClassElement::Field { key, .. } => {
                if let Expression::PrivateIdentifier(ident) = &key.inner {
                    Some(ident.name.clone())
                } else {
                    None
                }
            }
            ClassElement::StaticInitializer { .. } => None,
        };
        if let Some(name) = priv_name {
            if !has_private_env {
                gen.emit(Instruction::CreatePrivateEnvironment);
                has_private_env = true;
            }
            let name_id = gen.intern_identifier(name);
            gen.emit(Instruction::AddPrivateName { name: name_id });
        }
    }

    // Create SharedFunctionInstanceData for constructor
    let constructor_sfd_index = if let Some(ctor_expr) = &data.constructor {
        // Explicit constructor — extract FunctionData from the expression
        if let Expression::Function(func_data) = &ctor_expr.inner {
            emit_new_function(gen, func_data, None)
        } else {
            // Fallback: synthesize a default constructor
            emit_default_constructor(gen, has_super)
        }
    } else {
        // No explicit constructor — synthesize a default one
        emit_default_constructor(gen, has_super)
    };

    // Process class elements.
    let mut ffi_elements = Vec::new();
    let mut element_keys: Vec<Option<ScopedOperand>> = Vec::new();

    for elem_node in &data.elements {
        match &elem_node.inner {
            ClassElement::Method {
                key,
                function,
                kind,
                is_static,
            } => {
                let ffi_kind = match kind {
                    ClassMethodKind::Method => 0u8,
                    ClassMethodKind::Getter => 1u8,
                    ClassMethodKind::Setter => 2u8,
                };

                // Extract key name for the SFD (methods need their name set from the key).
                // Getters and setters have "get "/"set " prefixed to the name.
                let method_name = match &key.inner {
                    Expression::Identifier(ident) => Some(ident.name.clone()),
                    Expression::StringLiteral(s) => Some(s.clone()),
                    _ => None,
                }.map(|name| {
                    match kind {
                        ClassMethodKind::Getter => {
                            let mut prefixed: Vec<u16> = utf16!("get ").to_vec();
                            prefixed.extend_from_slice(&name);
                            prefixed
                        }
                        ClassMethodKind::Setter => {
                            let mut prefixed: Vec<u16> = utf16!("set ").to_vec();
                            prefixed.extend_from_slice(&name);
                            prefixed
                        }
                        ClassMethodKind::Method => name,
                    }
                });

                // Create SFD for the method function
                let sfd_index = if let Expression::Function(func_data) = &function.inner {
                    emit_new_function(
                        gen,
                        func_data,
                        method_name.as_deref(),
                    ) as i32
                } else {
                    -1i32
                };

                // Handle computed vs static keys
                let (is_private, _priv_name) = check_private_key(key);

                // Evaluate the key for non-private elements
                if !is_private {
                    let key_val = generate_expr(key, gen, None);
                    element_keys.push(key_val);
                } else {
                    element_keys.push(None);
                }

                // Point directly into the AST's PrivateIdentifier name (stable address).
                let (priv_ptr, priv_len) = get_private_identifier_ptr(key);

                ffi_elements.push(super::ffi::FFIClassElement {
                    kind: ffi_kind,
                    is_static: *is_static,
                    is_private,
                    private_identifier: priv_ptr,
                    private_identifier_len: priv_len,
                    shared_function_data_index: sfd_index,
                    has_initializer: false,
                });
            }
            ClassElement::Field {
                key,
                initializer,
                is_static,
            } => {
                // For fields with initializers, wrap the initializer in a
                // synthetic function so the runtime can call it to get the
                // initial value. This mirrors how C++ wraps initializers in
                // ClassFieldInitializerStatement.
                let sfd_index = if let Some(init_expr) = initializer {
                    // Determine field name for anonymous function naming.
                    let field_name = match &key.inner {
                        Expression::Identifier(ident) => ident.name.clone(),
                        Expression::StringLiteral(s) => s.clone(),
                        _ => Vec::new(),
                    };

                    // Wrap the expression in a ClassFieldInitializer statement.
                    let body_stmt = Stmt::new(
                        init_expr.range,
                        Statement::ClassFieldInitializer {
                            expression: Box::new(init_expr.as_ref().clone()),
                            field_name,
                        },
                    );
                    let wrapper_body = Stmt::new(
                        init_expr.range,
                        Statement::Block(Box::new(ScopeData::with_children(vec![body_stmt]))),
                    );

                    let func_data = FunctionData {
                        name: None,
                        source_text_start: init_expr.range.start.offset,
                        source_text_end: init_expr.range.end.offset,
                        body: Box::new(wrapper_body),
                        parameters: Vec::new(),
                        function_length: 0,
                        kind: FunctionKind::Normal,
                        is_strict_mode: gen.strict,
                        is_arrow_function: false,
                        parsing_insights: FunctionParsingInsights {
                            uses_this: true,
                            uses_this_from_environment: true,
                            ..Default::default()
                        },
                        is_hoisted: false,
                    };
                    emit_new_function(gen, &func_data, Some(utf16!("field"))) as i32
                } else {
                    -1i32
                };

                let (is_private, _priv_name) = check_private_key(key);

                if !is_private {
                    let key_val = generate_expr(key, gen, None);
                    element_keys.push(key_val);
                } else {
                    element_keys.push(None);
                }

                let (priv_ptr, priv_len) = get_private_identifier_ptr(key);

                ffi_elements.push(super::ffi::FFIClassElement {
                    kind: 3u8, // Field
                    is_static: *is_static,
                    is_private,
                    private_identifier: priv_ptr,
                    private_identifier_len: priv_len,
                    shared_function_data_index: sfd_index,
                    has_initializer: initializer.is_some(),
                });
            }
            ClassElement::StaticInitializer { body } => {
                // Wrap the static block body in a function.
                let func_data = FunctionData {
                    name: None,
                    source_text_start: body.range.start.offset,
                    source_text_end: body.range.end.offset,
                    body: body.clone(),
                    parameters: Vec::new(),
                    function_length: 0,
                    kind: FunctionKind::Normal,
                    is_strict_mode: gen.strict,
                    is_arrow_function: false,
                    parsing_insights: FunctionParsingInsights {
                        uses_this: true,
                        uses_this_from_environment: true,
                        ..Default::default()
                    },
                    is_hoisted: false,
                };
                let sfd_index = emit_new_function(gen, &func_data, None) as i32;

                element_keys.push(None);
                ffi_elements.push(super::ffi::FFIClassElement {
                    kind: 4u8, // StaticInitializer
                    is_static: true,
                    is_private: false,
                    private_identifier: std::ptr::null(),
                    private_identifier_len: 0,
                    shared_function_data_index: sfd_index,
                    has_initializer: false,
                });
            }
        }
    }

    // Get class name and source text
    let class_name = data.name.as_ref().map(|n| n.name.as_slice());
    let has_name = data.name.is_some();
    let (name_ptr, name_len) = class_name
        .map(|n| (n.as_ptr(), n.len()))
        .unwrap_or((std::ptr::null(), 0));

    let source_start = data.source_text_start as usize;
    let source_end = data.source_text_end as usize;
    let source_text_ptr = unsafe { gen.source.add(source_start) };
    let source_text_len = source_end - source_start;

    // Create the ClassBlueprint via FFI
    let bp_ptr = unsafe {
        super::ffi::rust_create_class_blueprint(
            name_ptr,
            name_len,
            source_text_ptr,
            source_text_len,
            constructor_sfd_index,
            has_super,
            has_name,
            ffi_elements.as_ptr(),
            ffi_elements.len(),
        )
    };
    assert!(!bp_ptr.is_null(), "rust_create_class_blueprint returned null");
    let blueprint_index = gen.register_class_blueprint(bp_ptr);

    // Build element_keys operands for the NewClass instruction
    let element_key_ops: Vec<Option<Operand>> = element_keys
        .iter()
        .map(|k| k.as_ref().map(|s| s.operand()))
        .collect();

    // Restore parent environment before emitting NewClass.
    gen.emit(Instruction::SetLexicalEnvironment { environment: parent_env.operand() });

    // Emit NewClass instruction
    gen.emit(Instruction::NewClass {
        dst: dst.operand(),
        super_class: super_class.as_ref().map(|s| s.operand()),
        class_environment: class_env.operand(),
        class_blueprint_index: blueprint_index,
        lhs_name: None,
        element_keys_count: element_key_ops.len() as u32,
        element_keys: element_key_ops,
    });

    if has_private_env {
        gen.emit(Instruction::LeavePrivateEnvironment);
    }

    Some(dst)
}

/// Synthesize a default constructor SharedFunctionInstanceData.
fn emit_default_constructor(gen: &mut Generator, has_super: bool) -> u32 {
    // Default constructor source:
    // - Base class: "constructor() {}"
    // - Derived class: "constructor(...args) { super(...args); }"
    let source: &[u16] = if has_super {
        &utf16!("constructor(...args) { super(...args); }")[..]
    } else {
        &utf16!("constructor() {}")[..]
    };

    let sfd_ptr = unsafe {
        super::ffi::rust_create_shared_function_data(
            gen.vm_ptr,
            gen.source_code_ptr,
            source.as_ptr(),
            source.len(),
            std::ptr::null(),
            0,
            gen.strict,
        )
    };
    assert!(
        !sfd_ptr.is_null(),
        "default constructor creation returned null"
    );
    gen.register_shared_function_data(sfd_ptr)
}

/// Check if a key expression is a private identifier, return (is_private, private_name).
fn check_private_key(key: &Expr) -> (bool, Option<Vec<u16>>) {
    if let Expression::PrivateIdentifier(ident) = &key.inner {
        (true, Some(ident.name.clone()))
    } else {
        (false, None)
    }
}

/// Get a pointer directly into the AST's PrivateIdentifier name.
/// The pointer remains valid as long as the AST is alive.
fn get_private_identifier_ptr(key: &Expr) -> (*const u16, usize) {
    if let Expression::PrivateIdentifier(ident) = &key.inner {
        (ident.name.as_ptr(), ident.name.len())
    } else {
        (std::ptr::null(), 0)
    }
}

/// Check if a for-in/for-of LHS is a `let`/`const` declaration with non-local identifiers,
/// meaning we need a per-iteration lexical environment.
fn for_in_of_needs_lexical_env(lhs: &ForInOfLhs) -> bool {
    if let ForInOfLhs::Declaration(stmt) = lhs {
        if let Statement::VariableDeclaration { kind, declarations } = &stmt.inner {
            if *kind == DeclarationKind::Let || *kind == DeclarationKind::Const {
                let mut names = Vec::new();
                for decl in declarations {
                    collect_target_names(&decl.target, &mut names);
                }
                return !names.is_empty();
            }
        }
    }
    false
}

/// Collect all non-local binding names from a variable declarator target.
fn collect_target_names(target: &VariableDeclaratorTarget, names: &mut Vec<(Vec<u16>, bool)>) {
    match target {
        VariableDeclaratorTarget::Identifier(ident) => {
            if !ident.is_local() {
                names.push((ident.name.clone(), false));
            }
        }
        VariableDeclaratorTarget::BindingPattern(pattern) => {
            collect_pattern_binding_names(pattern, names);
        }
    }
}

/// Collect all non-local binding names from a binding pattern (recursive).
fn collect_pattern_binding_names(pattern: &BindingPattern, names: &mut Vec<(Vec<u16>, bool)>) {
    for entry in &pattern.entries {
        match &entry.alias {
            BindingEntryAlias::Identifier(ident) => {
                if !ident.is_local() {
                    names.push((ident.name.clone(), false));
                }
            }
            BindingEntryAlias::BindingPattern(sub) => {
                collect_pattern_binding_names(sub, names);
            }
            BindingEntryAlias::Empty => {
                if let BindingEntryName::Identifier(ident) = &entry.name {
                    if !ident.is_local() {
                        names.push((ident.name.clone(), false));
                    }
                }
            }
            BindingEntryAlias::MemberExpression(_) => {}
        }
    }
}

/// Create a per-iteration lexical environment for for-in/for-of `let`/`const` declarations.
/// Returns the parent environment register so we can restore it later.
fn create_for_in_of_lexical_env(gen: &mut Generator, lhs: &ForInOfLhs) -> ScopedOperand {
    let parent = gen.lexical_environment_register_stack.last().cloned()
        .unwrap_or_else(|| gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT)));

    // Collect all binding names to determine capacity.
    let mut binding_names: Vec<(Vec<u16>, bool)> = Vec::new();
    let mut is_constant = false;
    if let ForInOfLhs::Declaration(stmt) = lhs {
        if let Statement::VariableDeclaration { kind, declarations } = &stmt.inner {
            is_constant = *kind == DeclarationKind::Const;
            for decl in declarations {
                collect_target_names(&decl.target, &mut binding_names);
            }
        }
    }

    let new_env = gen.allocate_register();
    gen.emit(Instruction::CreateLexicalEnvironment {
        dst: new_env.operand(),
        parent: parent.operand(),
        capacity: binding_names.len().max(1) as u32,
    });
    gen.lexical_environment_register_stack.push(new_env);

    // Create variable bindings in the new environment.
    for (name, _) in &binding_names {
        let id = gen.intern_identifier(name.clone());
        gen.emit(Instruction::CreateVariable {
            identifier: id,
            mode: ENV_MODE_LEXICAL,
            is_immutable: is_constant,
            is_global: false,
            is_strict: is_constant,
        });
    }

    parent
}

/// Check if a key is a "static" key (identifier or string literal — not computed).
// =============================================================================
// For-in statement
// =============================================================================

fn generate_for_in_statement(
    gen: &mut Generator,
    lhs: &ForInOfLhs,
    rhs: &Expr,
    body: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let object = generate_expr(rhs, gen, None).unwrap_or_else(|| gen.add_constant_undefined());
    let end_block = gen.make_block();
    let needs_lexical_env = for_in_of_needs_lexical_env(lhs);

    // Check for null/undefined
    let nullish_block = gen.make_block();
    let continue_block = gen.make_block();
    gen.emit(Instruction::JumpNullish {
        condition: object.operand(),
        true_target: Label(nullish_block as u32),
        false_target: Label(continue_block as u32),
    });

    gen.switch_to_basic_block(nullish_block);
    gen.emit(Instruction::Jump {
        target: Label(end_block as u32),
    });

    gen.switch_to_basic_block(continue_block);

    // Get property iterator
    let iterator_object = gen.allocate_register();
    let iterator_next_method = gen.allocate_register();
    let iterator_done = gen.allocate_register();
    gen.emit(Instruction::GetObjectPropertyIterator {
        dst_iterator_object: iterator_object.operand(),
        dst_iterator_next: iterator_next_method.operand(),
        dst_iterator_done: iterator_done.operand(),
        object: object.operand(),
    });

    let loop_block = gen.make_block();
    let update_block = gen.make_block();
    gen.emit(Instruction::Jump {
        target: Label(update_block as u32),
    });

    // Update: get next value
    gen.switch_to_basic_block(update_block);
    let next_value = gen.allocate_register();
    let done = gen.allocate_register();
    gen.emit(Instruction::IteratorNextUnpack {
        dst_value: next_value.operand(),
        dst_done: done.operand(),
        iterator_object: iterator_object.operand(),
        iterator_next: iterator_next_method.operand(),
        iterator_done: iterator_done.operand(),
    });

    let loop_continue_block = gen.make_block();
    gen.emit(Instruction::JumpIf {
        condition: done.operand(),
        true_target: Label(end_block as u32),
        false_target: Label(loop_continue_block as u32),
    });
    gen.switch_to_basic_block(loop_continue_block);

    // Create per-iteration lexical environment for let/const declarations.
    let parent_env = if needs_lexical_env {
        Some(create_for_in_of_lexical_env(gen, lhs))
    } else {
        None
    };

    // Assign to LHS
    assign_to_for_in_of_lhs(gen, lhs, &next_value);

    // Jump to body
    gen.emit(Instruction::Jump {
        target: Label(loop_block as u32),
    });

    // Body — use cleanup blocks for break/continue if we have a lexical env.
    gen.switch_to_basic_block(loop_block);
    let (break_target, continue_target) = if needs_lexical_env {
        (gen.make_block(), gen.make_block())
    } else {
        (end_block, update_block)
    };
    gen.begin_continuable_scope(Label(continue_target as u32), Vec::new());
    gen.begin_breakable_scope(Label(break_target as u32), Vec::new());
    generate_stmt(body, gen, preferred_dst);
    gen.end_breakable_scope();
    gen.end_continuable_scope();

    if needs_lexical_env {
        let parent = parent_env.as_ref().unwrap();
        if !gen.is_current_block_terminated() {
            gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
            gen.emit(Instruction::Jump { target: Label(update_block as u32) });
        }
        gen.lexical_environment_register_stack.pop();

        // Break cleanup: restore environment then jump to end.
        gen.switch_to_basic_block(break_target);
        gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
        gen.emit(Instruction::Jump { target: Label(end_block as u32) });

        // Continue cleanup: restore environment then jump to update.
        gen.switch_to_basic_block(continue_target);
        gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
        gen.emit(Instruction::Jump { target: Label(update_block as u32) });
    } else {
        if !gen.is_current_block_terminated() {
            gen.emit(Instruction::Jump { target: Label(update_block as u32) });
        }
    }

    gen.switch_to_basic_block(end_block);
    None
}

// =============================================================================
// Labelled statement
// =============================================================================

fn generate_labelled_statement(
    gen: &mut Generator,
    label: &[u16],
    item: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    // Collect all labels from nested Labelled statements.
    let mut labels = vec![label.to_vec()];
    let mut inner = item;
    while let Statement::Labelled { label: next_label, item: next_item } = &inner.inner {
        labels.push(next_label.clone());
        inner = next_item;
    }

    // For iteration/switch statements, set pending_labels so that
    // begin_breakable_scope/begin_continuable_scope pick them up.
    let is_iteration_or_switch = matches!(
        &inner.inner,
        Statement::For { .. }
            | Statement::ForOf { .. }
            | Statement::ForIn { .. }
            | Statement::ForAwaitOf { .. }
            | Statement::While { .. }
            | Statement::DoWhile { .. }
            | Statement::Switch(_)
    );

    if is_iteration_or_switch {
        let prev_labels = std::mem::replace(&mut gen.pending_labels, labels);
        let result = generate_stmt(inner, gen, preferred_dst);
        gen.pending_labels = prev_labels;
        result
    } else {
        // Non-iteration: wrap in a breakable scope so `break label;` works.
        let end_block = gen.make_block();
        gen.begin_breakable_scope(Label(end_block as u32), labels);
        let result = generate_stmt(inner, gen, preferred_dst);
        gen.end_breakable_scope();
        if !gen.is_current_block_terminated() {
            gen.emit(Instruction::Jump {
                target: Label(end_block as u32),
            });
        }
        gen.switch_to_basic_block(end_block);
        result
    }
}

// =============================================================================
// For-of statement
// =============================================================================

fn generate_for_of_statement(
    gen: &mut Generator,
    lhs: &ForInOfLhs,
    rhs: &Expr,
    body: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    generate_for_of_statement_inner(gen, lhs, rhs, body, preferred_dst, false)
}

/// Shared implementation for for-of and for-await-of with iterator close.
fn generate_for_of_statement_inner(
    gen: &mut Generator,
    lhs: &ForInOfLhs,
    rhs: &Expr,
    body: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
    is_await: bool,
) -> Option<ScopedOperand> {
    let object = generate_expr(rhs, gen, None).unwrap_or_else(|| gen.add_constant_undefined());
    let end_block = gen.make_block();
    let needs_lexical_env = for_in_of_needs_lexical_env(lhs);
    let old_handler = gen.current_unwind_handler;

    // Get iterator
    let iterator_object = gen.allocate_register();
    let iterator_next_method = gen.allocate_register();
    let iterator_done = gen.allocate_register();
    gen.emit(Instruction::GetIterator {
        dst_iterator_object: iterator_object.operand(),
        dst_iterator_next: iterator_next_method.operand(),
        dst_iterator_done: iterator_done.operand(),
        iterable: object.operand(),
        hint: if is_await { 1 } else { 0 },
    });

    // Set up iterator close via synthetic FinallyContext.
    let close_completion_type = gen.allocate_register();
    let close_completion_value = gen.allocate_register();
    let exception_preamble_block = gen.make_block();
    let iterator_close_body_block = gen.make_block();
    let lexical_env_at_entry = gen.lexical_environment_register_stack.last().cloned();

    let parent_index = gen.current_finally_context;
    gen.push_finally_context(FinallyContext {
        completion_type: close_completion_type.clone(),
        completion_value: close_completion_value.clone(),
        finally_body: Label(iterator_close_body_block as u32),
        exception_preamble: Label(exception_preamble_block as u32),
        parent_index,
        registered_jumps: Vec::new(),
        next_jump_index: FinallyContext::FIRST_JUMP_INDEX,
        lexical_environment_at_entry: lexical_env_at_entry.clone(),
    });

    // Break scope wraps the ReturnToFinally so break hits ReturnToFinally first.
    gen.begin_breakable_scope(Label(end_block as u32), Vec::new());
    gen.start_boundary(BlockBoundaryType::ReturnToFinally);

    let update_block = gen.make_block();
    gen.emit(Instruction::Jump {
        target: Label(update_block as u32),
    });

    // Update: get next value
    gen.switch_to_basic_block(update_block);
    let next_value = gen.allocate_register();
    let done = gen.allocate_register();

    if is_await {
        // For-await-of: Call iterator.next(), await the result, then unpack.
        let next_result = gen.allocate_register();
        gen.emit(Instruction::IteratorNext {
            dst: next_result.operand(),
            iterator_object: iterator_object.operand(),
            iterator_next: iterator_next_method.operand(),
            iterator_done: iterator_done.operand(),
        });
        // Await the next result.
        let awaited = generate_await(gen, next_result.clone());
        gen.emit_mov(&next_result, &awaited);
        // Type check
        gen.emit(Instruction::ThrowIfNotObject {
            src: next_result.operand(),
        });
        // IteratorComplete — get .done property
        emit_get_by_id(gen, &done, &next_result, utf16!("done"), None);

        let loop_continue_block = gen.make_block();
        gen.emit(Instruction::JumpIf {
            condition: done.operand(),
            true_target: Label(end_block as u32),
            false_target: Label(loop_continue_block as u32),
        });
        gen.switch_to_basic_block(loop_continue_block);

        // IteratorValue — get .value property
        emit_get_by_id(gen, &next_value, &next_result, utf16!("value"), None);
    } else {
        gen.emit(Instruction::IteratorNextUnpack {
            dst_value: next_value.operand(),
            dst_done: done.operand(),
            iterator_object: iterator_object.operand(),
            iterator_next: iterator_next_method.operand(),
            iterator_done: iterator_done.operand(),
        });

        let loop_continue_block = gen.make_block();
        gen.emit(Instruction::JumpIf {
            condition: done.operand(),
            true_target: Label(end_block as u32),
            false_target: Label(loop_continue_block as u32),
        });
        gen.switch_to_basic_block(loop_continue_block);
    }

    // Set up exception handler AFTER iterator-next section.
    // Per spec, exceptions from IteratorNext/Await/IteratorComplete/IteratorValue
    // propagate directly; only LHS assignment and body exceptions trigger close.
    gen.current_unwind_handler = Some(exception_preamble_block);
    let loop_body_block = gen.make_block();
    gen.emit(Instruction::Jump {
        target: Label(loop_body_block as u32),
    });
    gen.switch_to_basic_block(loop_body_block);

    // Create per-iteration lexical environment for let/const declarations.
    let parent_env = if needs_lexical_env {
        Some(create_for_in_of_lexical_env(gen, lhs))
    } else {
        None
    };

    // Assign to LHS
    assign_to_for_in_of_lhs(gen, lhs, &next_value);

    // Body
    gen.begin_continuable_scope(Label(update_block as u32), Vec::new());
    generate_stmt(body, gen, preferred_dst);

    // Restore lexical env before continuing
    if needs_lexical_env {
        gen.lexical_environment_register_stack.pop();
    }
    gen.end_continuable_scope();

    gen.end_boundary(BlockBoundaryType::ReturnToFinally);
    gen.end_breakable_scope();

    // Pop the FinallyContext.
    let finally_ctx_index = gen.current_finally_context.unwrap();
    gen.current_finally_context = gen.finally_contexts[finally_ctx_index].parent_index;

    // Restore unwind handler
    gen.current_unwind_handler = old_handler;

    if !gen.is_current_block_terminated() {
        if needs_lexical_env {
            let parent = parent_env.as_ref().unwrap();
            gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
        }
        gen.emit(Instruction::Jump { target: Label(update_block as u32) });
    }

    // --- Exception preamble: catch thrown exception, route to iterator close ---
    gen.switch_to_basic_block(exception_preamble_block);
    gen.emit(Instruction::Catch {
        dst: close_completion_value.operand(),
    });
    if let Some(env) = &lexical_env_at_entry {
        gen.emit(Instruction::SetLexicalEnvironment {
            environment: env.operand(),
        });
    }
    let throw_const = gen.add_constant_i32(FinallyContext::THROW);
    gen.emit_mov(&close_completion_type, &throw_const);
    gen.emit(Instruction::Jump {
        target: Label(iterator_close_body_block as u32),
    });

    // --- Iterator close body: dispatch based on completion type ---
    gen.switch_to_basic_block(iterator_close_body_block);

    // THROW path
    let throw_close_block = gen.make_block();
    let non_throw_close_block = gen.make_block();
    let throw_check_const = gen.add_constant_i32(FinallyContext::THROW);
    gen.emit(Instruction::JumpStrictlyEquals {
        lhs: close_completion_type.operand(),
        rhs: throw_check_const.operand(),
        true_target: Label(throw_close_block as u32),
        false_target: Label(non_throw_close_block as u32),
    });

    // Non-throw close: IteratorClose with Normal completion, then dispatch.
    gen.switch_to_basic_block(non_throw_close_block);
    let undef = gen.add_constant_undefined();
    gen.emit(Instruction::IteratorClose {
        iterator_object: iterator_object.operand(),
        iterator_next: iterator_next_method.operand(),
        iterator_done: iterator_done.operand(),
        completion_type: 1, // Completion::Type::Normal
        completion_value: undef.operand(),
    });

    // Dispatch registered jumps (break/continue targets).
    let registered_jumps = std::mem::take(&mut gen.finally_contexts[finally_ctx_index].registered_jumps);
    for jump in &registered_jumps {
        let after_check = gen.make_block();
        let jump_const = gen.add_constant_i32(jump.index);
        gen.emit(Instruction::JumpStrictlyEquals {
            lhs: close_completion_type.operand(),
            rhs: jump_const.operand(),
            true_target: jump.target,
            false_target: Label(after_check as u32),
        });
        gen.switch_to_basic_block(after_check);
    }

    // RETURN path
    let return_block = gen.make_block();
    let unreachable_block = gen.make_block();
    let return_const = gen.add_constant_i32(FinallyContext::RETURN);
    gen.emit(Instruction::JumpStrictlyEquals {
        lhs: close_completion_type.operand(),
        rhs: return_const.operand(),
        true_target: Label(return_block as u32),
        false_target: Label(unreachable_block as u32),
    });

    gen.switch_to_basic_block(return_block);
    if let Some(outer_idx) = gen.current_finally_context {
        let outer_ct = gen.finally_contexts[outer_idx].completion_type.clone();
        let outer_cv = gen.finally_contexts[outer_idx].completion_value.clone();
        let outer_fb = gen.finally_contexts[outer_idx].finally_body;
        gen.emit_mov(&outer_ct, &close_completion_type);
        gen.emit_mov(&outer_cv, &close_completion_value);
        gen.emit(Instruction::Jump { target: outer_fb });
    } else if gen.is_in_generator_or_async_function() {
        gen.emit(Instruction::Yield {
            continuation_label: None,
            value: close_completion_value.operand(),
        });
    } else {
        gen.emit(Instruction::Return {
            value: close_completion_value.operand(),
        });
    }

    // Unreachable default: throw the value.
    gen.switch_to_basic_block(unreachable_block);
    gen.emit(Instruction::Throw {
        src: close_completion_value.operand(),
    });

    // Throw close: IteratorClose with Throw completion, then rethrow.
    gen.switch_to_basic_block(throw_close_block);
    gen.emit(Instruction::IteratorClose {
        iterator_object: iterator_object.operand(),
        iterator_next: iterator_next_method.operand(),
        iterator_done: iterator_done.operand(),
        completion_type: 5, // Completion::Type::Throw
        completion_value: close_completion_value.operand(),
    });
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Throw {
            src: close_completion_value.operand(),
        });
    }

    gen.switch_to_basic_block(end_block);
    None
}

fn generate_for_await_of_statement(
    gen: &mut Generator,
    lhs: &ForInOfLhs,
    rhs: &Expr,
    body: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    generate_for_of_statement_inner(gen, lhs, rhs, body, preferred_dst, true)
}

fn assign_to_for_in_of_lhs(
    gen: &mut Generator,
    lhs: &ForInOfLhs,
    value: &ScopedOperand,
) {
    match lhs {
        ForInOfLhs::Declaration(stmt) => {
            // The declaration is a VariableDeclaration with a single declarator
            if let Statement::VariableDeclaration { kind, declarations } = &stmt.inner {
                if let Some(decl) = declarations.first() {
                    let mode = match kind {
                        DeclarationKind::Var => BindingMode::InitializeVariable,
                        DeclarationKind::Let | DeclarationKind::Const => {
                            BindingMode::InitializeLexical
                        }
                    };
                    match &decl.target {
                        VariableDeclaratorTarget::Identifier(ident) => {
                            emit_set_variable_with_mode(gen, ident, value, mode);
                        }
                        VariableDeclaratorTarget::BindingPattern(pattern) => {
                            generate_binding_pattern_bytecode(gen, pattern, mode, value);
                        }
                    }
                }
            }
        }
        ForInOfLhs::Expression(expr) => {
            emit_store_to_reference(gen, expr, value);
        }
        ForInOfLhs::Pattern(pattern) => {
            generate_binding_pattern_bytecode(gen, pattern, BindingMode::Set, value);
        }
    }
}

// =============================================================================
// Binding pattern destructuring
// =============================================================================

/// Whether we are initializing a new binding or setting an existing one.
#[derive(Clone, Copy, PartialEq, Eq)]
enum BindingMode {
    /// `const` or `let` declarations: emit InitializeLexicalBinding.
    InitializeLexical,
    /// `var` declarations: emit InitializeVariableBinding.
    InitializeVariable,
    /// Assignment expressions: emit SetLexicalBinding or SetGlobal.
    Set,
}

fn generate_binding_pattern_bytecode(
    gen: &mut Generator,
    pattern: &BindingPattern,
    mode: BindingMode,
    input_value: &ScopedOperand,
) {
    match pattern.kind {
        BindingPatternKind::Array => {
            generate_array_binding_pattern(gen, pattern, mode, input_value);
        }
        BindingPatternKind::Object => {
            generate_object_binding_pattern(gen, pattern, mode, input_value);
        }
    }
}

fn emit_set_variable_with_mode(
    gen: &mut Generator,
    ident: &Identifier,
    value: &ScopedOperand,
    mode: BindingMode,
) {
    if ident.is_local() {
        let local_index = ident.local_index.get();
        let local = match ident.local_type.get() {
            LocalType::Argument => gen.scoped_operand(Operand::argument(local_index)),
            LocalType::Variable => gen.local(local_index),
            LocalType::None => unreachable!(),
        };
        gen.emit_mov(&local, value);
        if mode != BindingMode::Set {
            gen.mark_local_initialized(local_index);
        }
    } else {
        let id = gen.intern_identifier(ident.name.clone());
        match mode {
            BindingMode::InitializeLexical => {
                gen.emit(Instruction::InitializeLexicalBinding {
                    identifier: id,
                    src: value.operand(),
                    cache: EnvironmentCoordinate::empty(),
                });
            }
            BindingMode::InitializeVariable => {
                gen.emit(Instruction::InitializeVariableBinding {
                    identifier: id,
                    src: value.operand(),
                    cache: EnvironmentCoordinate::empty(),
                });
            }
            BindingMode::Set => {
                if ident.is_global.get() {
                    let cache = gen.next_global_variable_cache();
                    gen.emit(Instruction::SetGlobal {
                        identifier: id,
                        src: value.operand(),
                        cache_index: cache,
                    });
                } else {
                    gen.emit(Instruction::SetLexicalBinding {
                        identifier: id,
                        src: value.operand(),
                        cache: EnvironmentCoordinate::empty(),
                    });
                }
            }
        }
    }
}

fn assign_binding_entry_alias(
    gen: &mut Generator,
    entry: &BindingEntry,
    value: &ScopedOperand,
    mode: BindingMode,
) {
    match &entry.alias {
        BindingEntryAlias::Empty => {
            // Name IS the binding target (e.g., `{ x }` or array element).
            if let BindingEntryName::Identifier(ident) = &entry.name {
                emit_set_variable_with_mode(gen, ident, value, mode);
            }
        }
        BindingEntryAlias::Identifier(ident) => {
            emit_set_variable_with_mode(gen, ident, value, mode);
        }
        BindingEntryAlias::BindingPattern(sub_pattern) => {
            generate_binding_pattern_bytecode(gen, sub_pattern, mode, value);
        }
        BindingEntryAlias::MemberExpression(expr) => {
            emit_store_to_reference(gen, expr, value);
        }
    }
}

fn generate_array_binding_pattern(
    gen: &mut Generator,
    pattern: &BindingPattern,
    mode: BindingMode,
    input_array: &ScopedOperand,
) {
    let is_exhausted = gen.allocate_register();
    let false_val = gen.add_constant_boolean(false);
    gen.emit_mov(&is_exhausted, &false_val);

    let iterator_object = gen.allocate_register();
    let iterator_next = gen.allocate_register();
    let iterator_done = gen.allocate_register();
    gen.emit(Instruction::GetIterator {
        dst_iterator_object: iterator_object.operand(),
        dst_iterator_next: iterator_next.operand(),
        dst_iterator_done: iterator_done.operand(),
        iterable: input_array.operand(),
        hint: 0, // Sync
    });

    let mut first = true;
    for entry in &pattern.entries {
        if entry.is_rest {
            // Rest element: collect remaining into array.
            let value = gen.allocate_register();
            if first {
                gen.emit(Instruction::IteratorToArray {
                    dst: value.operand(),
                    iterator_object: iterator_object.operand(),
                    iterator_next_method: iterator_next.operand(),
                    iterator_done_property: iterator_done.operand(),
                });
            } else {
                let if_exhausted = gen.make_block();
                let if_not_exhausted = gen.make_block();
                let continuation = gen.make_block();

                gen.emit(Instruction::JumpIf {
                    condition: is_exhausted.operand(),
                    true_target: Label(if_exhausted as u32),
                    false_target: Label(if_not_exhausted as u32),
                });

                gen.switch_to_basic_block(if_exhausted);
                gen.emit(Instruction::NewArray {
                    dst: value.operand(),
                    element_count: 0,
                    elements: Vec::new(),
                });
                gen.emit(Instruction::Jump {
                    target: Label(continuation as u32),
                });

                gen.switch_to_basic_block(if_not_exhausted);
                gen.emit(Instruction::IteratorToArray {
                    dst: value.operand(),
                    iterator_object: iterator_object.operand(),
                    iterator_next_method: iterator_next.operand(),
                    iterator_done_property: iterator_done.operand(),
                });
                gen.emit(Instruction::Jump {
                    target: Label(continuation as u32),
                });

                gen.switch_to_basic_block(continuation);
            }

            assign_binding_entry_alias(gen, entry, &value, mode);
            return; // rest consumes the iterator
        }

        // For elisions (BindingEntryName::Empty), we still advance the iterator
        // but don't bind anything.
        let is_elision = matches!(entry.name, BindingEntryName::Empty)
            && matches!(entry.alias, BindingEntryAlias::Empty);

        let exhausted_block = gen.make_block();

        if !first {
            let not_exhausted_block = gen.make_block();
            gen.emit(Instruction::JumpIf {
                condition: is_exhausted.operand(),
                true_target: Label(exhausted_block as u32),
                false_target: Label(not_exhausted_block as u32),
            });
            gen.switch_to_basic_block(not_exhausted_block);
        }

        let value = gen.allocate_register();
        gen.emit(Instruction::IteratorNextUnpack {
            dst_value: value.operand(),
            dst_done: is_exhausted.operand(),
            iterator_object: iterator_object.operand(),
            iterator_next: iterator_next.operand(),
            iterator_done: iterator_done.operand(),
        });

        // Check if iterator got exhausted by this step.
        let no_bail_block = gen.make_block();
        gen.emit(Instruction::JumpIf {
            condition: is_exhausted.operand(),
            true_target: Label(exhausted_block as u32),
            false_target: Label(no_bail_block as u32),
        });

        gen.switch_to_basic_block(no_bail_block);
        let create_binding_block = gen.make_block();
        gen.emit(Instruction::Jump {
            target: Label(create_binding_block as u32),
        });

        // Exhausted: load undefined.
        gen.switch_to_basic_block(exhausted_block);
        let undef = gen.add_constant_undefined();
        gen.emit_mov(&value, &undef);
        gen.emit(Instruction::Jump {
            target: Label(create_binding_block as u32),
        });

        gen.switch_to_basic_block(create_binding_block);

        // Handle default initializer.
        if let Some(ref initializer) = entry.initializer {
            let if_undefined = gen.make_block();
            let if_not_undefined = gen.make_block();
            gen.emit(Instruction::JumpUndefined {
                condition: value.operand(),
                true_target: Label(if_undefined as u32),
                false_target: Label(if_not_undefined as u32),
            });
            gen.switch_to_basic_block(if_undefined);
            if let Some(default_value) = generate_expr(initializer, gen, None) {
                gen.emit_mov(&value, &default_value);
            }
            gen.emit(Instruction::Jump {
                target: Label(if_not_undefined as u32),
            });
            gen.switch_to_basic_block(if_not_undefined);
        }

        if !is_elision {
            assign_binding_entry_alias(gen, entry, &value, mode);
        }

        first = false;
    }

    // Close iterator if not exhausted.
    let done_block = gen.make_block();
    let not_done_block = gen.make_block();
    gen.emit(Instruction::JumpIf {
        condition: is_exhausted.operand(),
        true_target: Label(done_block as u32),
        false_target: Label(not_done_block as u32),
    });
    gen.switch_to_basic_block(not_done_block);
    let undef = gen.add_constant_undefined();
    gen.emit(Instruction::IteratorClose {
        iterator_object: iterator_object.operand(),
        iterator_next: iterator_next.operand(),
        iterator_done: iterator_done.operand(),
        completion_type: COMPLETION_TYPE_NORMAL as u32,
        completion_value: undef.operand(),
    });
    gen.emit(Instruction::Jump {
        target: Label(done_block as u32),
    });
    gen.switch_to_basic_block(done_block);
}

fn generate_object_binding_pattern(
    gen: &mut Generator,
    pattern: &BindingPattern,
    mode: BindingMode,
    object: &ScopedOperand,
) {
    gen.emit(Instruction::ThrowIfNullish {
        src: object.operand(),
    });

    let mut excluded_names: Vec<ScopedOperand> = Vec::new();
    let has_rest = pattern
        .entries
        .last()
        .map_or(false, |e| e.is_rest);

    for entry in &pattern.entries {
        if entry.is_rest {
            // Rest element: copy object excluding already-destructured properties.
            let copy = gen.allocate_register();
            gen.emit(Instruction::CopyObjectExcludingProperties {
                dst: copy.operand(),
                from_object: object.operand(),
                excluded_names_count: excluded_names.len() as u32,
                excluded_names: excluded_names.iter().map(|o| o.operand()).collect(),
            });
            assign_binding_entry_alias(gen, entry, &copy, mode);
            return;
        }

        let value = gen.allocate_register();

        match &entry.name {
            BindingEntryName::Identifier(ident) => {
                let key = gen.intern_property_key(ident.name.clone());
                let cache_index = gen.next_property_lookup_cache();
                gen.emit(Instruction::GetById {
                    dst: value.operand(),
                    base: object.operand(),
                    property: key,
                    cache_index,
                    base_identifier: None,
                });
                if has_rest {
                    let name_val = gen.add_constant_string(ident.name.clone());
                    excluded_names.push(name_val);
                }
            }
            BindingEntryName::Expression(expr) => {
                let property_name = generate_expr(expr, gen, None)
                    .unwrap_or_else(|| gen.add_constant_undefined());
                if has_rest {
                    let excluded_name = gen.allocate_register();
                    gen.emit_mov(&excluded_name, &property_name);
                    excluded_names.push(excluded_name);
                }
                gen.emit(Instruction::GetByValue {
                    dst: value.operand(),
                    base: object.operand(),
                    property: property_name.operand(),
                    base_identifier: None,
                });
            }
            BindingEntryName::Empty => {
                // Should not happen for object patterns
                continue;
            }
        }

        // Handle default initializer.
        if let Some(ref initializer) = entry.initializer {
            let if_undefined = gen.make_block();
            let if_not_undefined = gen.make_block();
            gen.emit(Instruction::JumpUndefined {
                condition: value.operand(),
                true_target: Label(if_undefined as u32),
                false_target: Label(if_not_undefined as u32),
            });
            gen.switch_to_basic_block(if_undefined);
            if let Some(default_value) = generate_expr(initializer, gen, None) {
                gen.emit_mov(&value, &default_value);
            }
            gen.emit(Instruction::Jump {
                target: Label(if_not_undefined as u32),
            });
            gen.switch_to_basic_block(if_not_undefined);
        }

        assign_binding_entry_alias(gen, entry, &value, mode);
    }
}

fn generate_try_statement(
    gen: &mut Generator,
    data: &TryStatementData,
    _preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let old_handler = gen.current_unwind_handler;
    let saved_block = gen.current_block_index();

    // Save lexical environment for restoration in catch/exception handler.
    let saved_env = gen.lexical_environment_register_stack.last().cloned()
        .unwrap_or_else(|| gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT)));

    let mut next_block: Option<usize> = None;

    // --- Set up FinallyContext if we have a finalizer ---
    let has_finally = data.finalizer.is_some();
    let mut finally_body_block: Option<usize> = None;

    if has_finally {
        let completion_type = gen.allocate_register();
        let completion_value = gen.allocate_register();

        let exception_preamble_block = gen.make_block();
        let fb_block = gen.make_block();
        finally_body_block = Some(fb_block);

        // Save the parent FinallyContext and install new one.
        let parent_index = gen.current_finally_context;
        gen.push_finally_context(FinallyContext {
            completion_type,
            completion_value,
            finally_body: Label(fb_block as u32),
            exception_preamble: Label(exception_preamble_block as u32),
            parent_index,
            registered_jumps: Vec::new(),
            next_jump_index: FinallyContext::FIRST_JUMP_INDEX,
            lexical_environment_at_entry: Some(saved_env.clone()),
        });

        // Generate exception preamble block:
        //   Catch → completion_value
        //   SetLexicalEnvironment (restore to entry)
        //   completion_type = THROW
        //   Jump → finally_body
        gen.switch_to_basic_block(exception_preamble_block);
        let ctx_idx = gen.current_finally_context.unwrap();
        let cv = gen.finally_contexts[ctx_idx].completion_value.clone();
        let ct = gen.finally_contexts[ctx_idx].completion_type.clone();
        gen.emit(Instruction::Catch { dst: cv.operand() });
        gen.emit(Instruction::SetLexicalEnvironment {
            environment: saved_env.operand(),
        });
        let throw_const = gen.add_constant_i32(FinallyContext::THROW);
        gen.emit_mov(&ct, &throw_const);
        gen.emit(Instruction::Jump {
            target: Label(fb_block as u32),
        });

        // Set exception_preamble as default handler for blocks created below.
        // The catch body gets this as its handler (exceptions in catch → finally).
        gen.current_unwind_handler = Some(exception_preamble_block);
        gen.start_boundary(BlockBoundaryType::ReturnToFinally);
    }

    // --- Generate catch handler block (if present) ---
    let mut handler_block: Option<usize> = None;
    if let Some(catch) = &data.handler {
        let hb = gen.make_block();
        handler_block = Some(hb);
        gen.switch_to_basic_block(hb);

        let caught_value = gen.allocate_register();
        gen.emit(Instruction::Catch {
            dst: caught_value.operand(),
        });
        gen.emit(Instruction::SetLexicalEnvironment {
            environment: saved_env.operand(),
        });

        // Bind the catch parameter.
        let mut created_catch_scope = false;
        match &catch.parameter {
            CatchParameter::Identifier(ident) => {
                if ident.is_local() {
                    let local = gen.local(ident.local_index.get());
                    gen.emit_mov(&local, &caught_value);
                    gen.mark_local_initialized(ident.local_index.get());
                } else {
                    let parent = gen.lexical_environment_register_stack.last().cloned();
                    let parent = parent.unwrap_or_else(|| {
                        gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT))
                    });
                    let new_env = gen.allocate_register();
                    gen.emit(Instruction::CreateLexicalEnvironment {
                        dst: new_env.operand(),
                        parent: parent.operand(),
                        capacity: 1,
                    });
                    gen.lexical_environment_register_stack.push(new_env);
                    created_catch_scope = true;

                    let id = gen.intern_identifier(ident.name.clone());
                    gen.emit(Instruction::CreateVariable {
                        identifier: id,
                        mode: ENV_MODE_LEXICAL,
                        is_immutable: false,
                        is_global: false,
                        is_strict: false,
                    });
                    gen.emit(Instruction::InitializeLexicalBinding {
                        identifier: id,
                        src: caught_value.operand(),
                        cache: EnvironmentCoordinate::empty(),
                    });
                }
            }
            CatchParameter::BindingPattern(_) => {
                // Destructuring catch: TODO
            }
            CatchParameter::None => {}
        }

        generate_stmt(&catch.body, gen, None);

        if created_catch_scope {
            gen.lexical_environment_register_stack.pop();
        }

        if !gen.is_current_block_terminated() {
            if has_finally {
                // Normal exit from catch → completion_type = NORMAL, jump to finally.
                let ctx_idx = gen.current_finally_context.unwrap();
                let ct = gen.finally_contexts[ctx_idx].completion_type.clone();
                let fb = gen.finally_contexts[ctx_idx].finally_body;
                let normal_const = gen.add_constant_i32(FinallyContext::NORMAL);
                gen.emit_mov(&ct, &normal_const);
                gen.emit(Instruction::Jump { target: fb });
            } else {
                if next_block.is_none() {
                    next_block = Some(gen.make_block());
                }
                gen.emit(Instruction::Jump {
                    target: Label(next_block.unwrap() as u32),
                });
            }
        }
    }

    if has_finally {
        gen.end_boundary(BlockBoundaryType::ReturnToFinally);
    }

    // --- Generate try body ---

    // Set handler BEFORE creating the try body block, so make_block()
    // captures the correct handler for exception routing.
    // For try-catch-finally: catch handler is inner (exceptions → catch → exception_preamble → finally).
    // For try-catch: catch handler.
    // For try-finally: exception_preamble.
    if let Some(hb) = handler_block {
        gen.current_unwind_handler = Some(hb);
    } else if has_finally {
        if let Some(ctx_idx) = gen.current_finally_context {
            let ep = match gen.finally_contexts[ctx_idx].exception_preamble {
                Label(idx) => idx as usize,
            };
            gen.current_unwind_handler = Some(ep);
        }
    }

    let try_body_block = gen.make_block();
    gen.switch_to_basic_block(saved_block);
    gen.emit(Instruction::Jump {
        target: Label(try_body_block as u32),
    });

    if has_finally {
        gen.start_boundary(BlockBoundaryType::ReturnToFinally);
    }

    gen.switch_to_basic_block(try_body_block);

    generate_stmt(&data.block, gen, None);

    if !gen.is_current_block_terminated() {
        if has_finally {
            // Normal exit from try → completion_type = NORMAL, jump to finally.
            let ctx_idx = gen.current_finally_context.unwrap();
            let ct = gen.finally_contexts[ctx_idx].completion_type.clone();
            let fb = gen.finally_contexts[ctx_idx].finally_body;
            let normal_const = gen.add_constant_i32(FinallyContext::NORMAL);
            gen.emit_mov(&ct, &normal_const);
            gen.emit(Instruction::Jump { target: fb });
        } else {
            gen.current_unwind_handler = old_handler;
            if next_block.is_none() {
                next_block = Some(gen.make_block());
            }
            gen.emit(Instruction::Jump {
                target: Label(next_block.unwrap() as u32),
            });
        }
    }

    if has_finally {
        gen.end_boundary(BlockBoundaryType::ReturnToFinally);
    }

    // Restore old unwind handler.
    gen.current_unwind_handler = old_handler;

    // --- Generate finally body and after-finally dispatch ---
    if let Some(fb_block) = finally_body_block {
        // Pop FinallyContext.
        let ctx_index = gen.current_finally_context.unwrap();
        gen.current_finally_context = gen.finally_contexts[ctx_index].parent_index;

        // Extract fields needed for dispatch (to avoid borrow conflicts).
        let ctx_ct = gen.finally_contexts[ctx_index].completion_type.clone();
        let ctx_cv = gen.finally_contexts[ctx_index].completion_value.clone();

        gen.switch_to_basic_block(fb_block);
        gen.start_boundary(BlockBoundaryType::LeaveFinally);

        // Generate the finally body.
        if let Some(finalizer) = &data.finalizer {
            generate_stmt(finalizer, gen, None);
        }

        gen.end_boundary(BlockBoundaryType::LeaveFinally);

        if !gen.is_current_block_terminated() {
            if next_block.is_none() {
                next_block = Some(gen.make_block());
            }
            let nb = next_block.unwrap();

            // After-finally dispatch chain:
            // 1. NORMAL → next block
            let after_normal_check = gen.make_block();
            let normal_const = gen.add_constant_i32(FinallyContext::NORMAL);
            gen.emit(Instruction::JumpStrictlyEquals {
                lhs: ctx_ct.operand(),
                rhs: normal_const.operand(),
                true_target: Label(nb as u32),
                false_target: Label(after_normal_check as u32),
            });
            gen.switch_to_basic_block(after_normal_check);

            // 2. Registered break/continue jumps
            let registered_jumps = std::mem::take(&mut gen.finally_contexts[ctx_index].registered_jumps);
            for jump in &registered_jumps {
                let after_jump_check = gen.make_block();
                let jump_const = gen.add_constant_i32(jump.index);
                gen.emit(Instruction::JumpStrictlyEquals {
                    lhs: ctx_ct.operand(),
                    rhs: jump_const.operand(),
                    true_target: jump.target,
                    false_target: Label(after_jump_check as u32),
                });
                gen.switch_to_basic_block(after_jump_check);
            }

            // 3. RETURN → actually return the completion_value
            let return_block = gen.make_block();
            let rethrow_block = gen.make_block();
            let return_const = gen.add_constant_i32(FinallyContext::RETURN);
            gen.emit(Instruction::JumpStrictlyEquals {
                lhs: ctx_ct.operand(),
                rhs: return_const.operand(),
                true_target: Label(return_block as u32),
                false_target: Label(rethrow_block as u32),
            });

            // Generate return block.
            gen.switch_to_basic_block(return_block);
            if let Some(outer_idx) = gen.current_finally_context {
                // Nested finally: copy completion record to outer and jump to outer finally.
                let outer_ct = gen.finally_contexts[outer_idx].completion_type.clone();
                let outer_cv = gen.finally_contexts[outer_idx].completion_value.clone();
                let outer_fb = gen.finally_contexts[outer_idx].finally_body;
                gen.emit_mov(&outer_ct, &ctx_ct);
                gen.emit_mov(&outer_cv, &ctx_cv);
                gen.emit(Instruction::Jump { target: outer_fb });
            } else {
                if gen.is_in_generator_or_async_function() {
                    gen.emit(Instruction::Yield {
                        continuation_label: None,
                        value: ctx_cv.operand(),
                    });
                } else {
                    gen.emit(Instruction::Return {
                        value: ctx_cv.operand(),
                    });
                }
            }

            // 4. Default → rethrow the exception.
            gen.switch_to_basic_block(rethrow_block);
            gen.emit(Instruction::Throw {
                src: ctx_cv.operand(),
            });
        }
    }

    // Switch to the next block for code after the try statement.
    if let Some(nb) = next_block {
        gen.switch_to_basic_block(nb);
    } else {
        // No next block means all paths through try are terminated (return/throw).
        // Create a dead block to keep the generator pointing somewhere valid.
        let dead = gen.make_block();
        gen.switch_to_basic_block(dead);
    }

    None
}

/// Create a SharedFunctionInstanceData for a function expression/declaration
/// and register it with the generator.
///
/// Clones the FunctionData and stores it in the SFD for lazy compilation
/// through the Rust pipeline. No C++ AST is created.
///
/// Returns the shared_function_data_index for use in NewFunction instructions.
fn emit_new_function(
    gen: &mut Generator,
    data: &FunctionData,
    name_override: Option<&[u16]>,
) -> u32 {
    let source_start = data.source_text_start as usize;
    let source_end = data.source_text_end as usize;

    // Get the function source text pointer from the original source buffer.
    assert!(
        !gen.source.is_null() && gen.source_len > 0,
        "Generator must have source set for function compilation"
    );
    assert!(
        source_end <= gen.source_len,
        "Function source range out of bounds: {}..{} (source len {})",
        source_start,
        source_end,
        gen.source_len
    );

    let source_text_ptr = unsafe { gen.source.add(source_start) };
    let source_text_len = source_end - source_start;

    // Get function name.
    let (name_ptr, name_len) = if let Some(name) = name_override {
        (name.as_ptr(), name.len())
    } else if let Some(name_ident) = &data.name {
        (name_ident.name.as_ptr(), name_ident.name.len())
    } else {
        (std::ptr::null(), 0)
    };

    // Compute has_simple_parameter_list (IsSimpleParameterList).
    let has_simple_parameter_list = data.parameters.iter().all(|p| {
        !p.is_rest
            && p.default_value.is_none()
            && matches!(p.binding, FunctionParameterBinding::Identifier(_))
    });

    // Extract parameter names for mapped arguments (only if simple params).
    let param_name_slices: Vec<super::ffi::FFIUtf16Slice> = if has_simple_parameter_list {
        data.parameters
            .iter()
            .map(|p| {
                if let FunctionParameterBinding::Identifier(ref id) = p.binding {
                    super::ffi::FFIUtf16Slice {
                        data: id.name.as_ptr(),
                        length: id.name.len(),
                    }
                } else {
                    unreachable!()
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    // Clone the FunctionData into a Box for storage in the SFD.
    let cloned = Box::new(data.clone());
    let rust_ast_ptr = Box::into_raw(cloned) as *mut std::ffi::c_void;

    let function_kind = data.kind as u8;
    let strict = data.is_strict_mode || gen.strict;

    // Create SFD via FFI with pre-computed metadata + Rust AST.
    let sfd_ptr = unsafe {
        super::ffi::rust_create_sfd(
            gen.vm_ptr,
            gen.source_code_ptr,
            name_ptr,
            name_len,
            function_kind,
            data.function_length,
            data.parameters.len() as u32,
            strict,
            data.is_arrow_function,
            has_simple_parameter_list,
            param_name_slices.as_ptr(),
            param_name_slices.len(),
            source_text_ptr,
            source_text_len,
            rust_ast_ptr,
        )
    };

    assert!(!sfd_ptr.is_null(), "rust_create_sfd returned null");

    gen.register_shared_function_data(sfd_ptr)
}

// =============================================================================
// FunctionDeclarationInstantiation (FDI)
// =============================================================================

const ENV_MODE_LEXICAL: u32 = 0;
const ENV_MODE_VAR: u32 = 1;
const ARGUMENTS_KIND_MAPPED: u32 = 0;
const ARGUMENTS_KIND_UNMAPPED: u32 = 1;

/// Emit FDI bytecode for a function body.
///
/// This is a port of `Generator::emit_function_declaration_instantiation`
/// from C++. It creates environment bindings, initializes parameters,
/// creates arguments objects, and hoists function declarations.
pub fn emit_function_declaration_instantiation(
    gen: &mut Generator,
    func_data: &FunctionData,
    body_scope: &ScopeData,
) {
    let strict = func_data.is_strict_mode || gen.strict;
    let is_arrow = func_data.is_arrow_function;

    // --- Compute FDI metadata ---

    // Check for parameter expressions (default values or binding patterns with defaults).
    let has_parameter_expressions = func_data.parameters.iter().any(|p| {
        p.default_value.is_some()
            || matches!(p.binding, FunctionParameterBinding::BindingPattern(_))
    });

    // Build parameter_names map and check for duplicates.
    let mut parameter_names: Vec<(Vec<u16>, bool)> = Vec::new(); // (name, is_local)
    let mut has_duplicates = false;

    for param in &func_data.parameters {
        match &param.binding {
            FunctionParameterBinding::Identifier(ident) => {
                let name = ident.name.clone();
                let is_local = ident.is_local();
                let already_exists = parameter_names.iter().any(|(n, _)| *n == name);
                if already_exists {
                    has_duplicates = true;
                } else {
                    parameter_names.push((name, is_local));
                }
            }
            FunctionParameterBinding::BindingPattern(pattern) => {
                collect_binding_pattern_names(pattern, &mut parameter_names, &mut has_duplicates);
            }
        }
    }

    // Determine if arguments object is needed (from scope analysis).
    let mut arguments_object_needed = body_scope.contains_access_to_arguments_object;

    if is_arrow {
        arguments_object_needed = false;
    } else if parameter_names.iter().any(|(n, _)| *n == utf16!("arguments")) {
        arguments_object_needed = false;
    }

    let function_scope_data = body_scope.function_scope_data.as_ref();

    if let Some(fsd) = function_scope_data {
        if !has_parameter_expressions && fsd.has_function_named_arguments {
            arguments_object_needed = false;
        }
        if !has_parameter_expressions && arguments_object_needed && fsd.has_lexically_declared_arguments
        {
            arguments_object_needed = false;
        }
    }

    // Check if arguments object needs an environment binding (not a local variable).
    let _arguments_object_needs_binding = arguments_object_needed
        && !gen
            .local_variables
            .iter()
            .any(|lv| lv.name == utf16!("arguments") && !lv.is_lexically_declared);

    // --- Step 1: Parameter scope for parameter expressions ---

    if has_parameter_expressions {
        let has_non_local_params = parameter_names.iter().any(|(_, is_local)| !is_local);
        if has_non_local_params {
            let parent = gen.lexical_environment_register_stack.last().cloned();
            let parent = parent.unwrap_or_else(|| {
                gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT))
            });
            let new_env = gen.allocate_register();
            gen.emit(Instruction::CreateLexicalEnvironment {
                dst: new_env.operand(),
                parent: parent.operand(),
                capacity: 0,
            });
            gen.lexical_environment_register_stack.push(new_env);
        }
    }

    // --- Step 2: Create bindings for non-local parameters ---

    for (name, is_local) in &parameter_names {
        if !is_local {
            let id = gen.intern_identifier(name.clone());
            gen.emit(Instruction::CreateVariable {
                identifier: id,
                mode: ENV_MODE_LEXICAL,
                is_immutable: false,
                is_global: false,
                is_strict: false,
            });
            if has_duplicates {
                let undef = gen.add_constant_undefined();
                gen.emit(Instruction::InitializeLexicalBinding {
                    identifier: id,
                    src: undef.operand(),
                    cache: EnvironmentCoordinate::empty(),
                });
            }
        }
    }

    // --- Step 3: Create arguments object ---

    if arguments_object_needed {
        // Find local variable index for ArgumentsObject, if any.
        let args_local_index = gen.local_variables.iter().position(|lv| {
            lv.name == utf16!("arguments") && !lv.is_lexically_declared
        });

        let dst = args_local_index.map(|idx| Operand::local(idx as u32));

        let kind = if strict || !func_data.parameters.iter().all(|p| {
            !p.is_rest
                && p.default_value.is_none()
                && matches!(p.binding, FunctionParameterBinding::Identifier(_))
        }) {
            ARGUMENTS_KIND_UNMAPPED
        } else {
            ARGUMENTS_KIND_MAPPED
        };

        gen.emit(Instruction::CreateArguments {
            dst,
            kind,
            is_immutable: strict,
        });

        if let Some(idx) = args_local_index {
            gen.mark_local_initialized(idx as u32);
        }
    }

    // --- Step 4: Bind formal parameters ---

    for (param_index, param) in func_data.parameters.iter().enumerate() {
        let param_idx = param_index as u32;

        if param.is_rest {
            let dst = gen.scoped_operand(Operand::argument(param_idx));
            gen.emit(Instruction::CreateRestParams {
                dst: dst.operand(),
                rest_index: param_idx,
            });
        } else if param.default_value.is_some() {
            let if_undefined_block = gen.make_block();
            let if_not_undefined_block = gen.make_block();

            gen.emit(Instruction::JumpUndefined {
                condition: Operand::argument(param_idx),
                true_target: Label(if_undefined_block as u32),
                false_target: Label(if_not_undefined_block as u32),
            });

            gen.switch_to_basic_block(if_undefined_block);
            if let Some(value) = generate_expr(param.default_value.as_ref().unwrap(), gen, None) {
                gen.emit_mov_raw(Operand::argument(param_idx), value.operand());
            }
            gen.emit(Instruction::Jump {
                target: Label(if_not_undefined_block as u32),
            });

            gen.switch_to_basic_block(if_not_undefined_block);
        }

        match &param.binding {
            FunctionParameterBinding::Identifier(ident) => {
                if ident.is_local() {
                    let local_idx = ident.local_index.get();
                    match ident.local_type.get() {
                        LocalType::Variable => gen.mark_local_initialized(local_idx),
                        LocalType::Argument => gen.mark_local_initialized(local_idx),
                        _ => {}
                    }
                } else {
                    let id = gen.intern_identifier(ident.name.clone());
                    if has_duplicates {
                        gen.emit(Instruction::SetLexicalBinding {
                            identifier: id,
                            src: Operand::argument(param_idx),
                            cache: EnvironmentCoordinate::empty(),
                        });
                    } else {
                        gen.emit(Instruction::InitializeLexicalBinding {
                            identifier: id,
                            src: Operand::argument(param_idx),
                            cache: EnvironmentCoordinate::empty(),
                        });
                    }
                }
            }
            FunctionParameterBinding::BindingPattern(pattern) => {
                let arg = gen.scoped_operand(Operand::argument(param_idx));
                let mode = if has_duplicates {
                    BindingMode::Set
                } else {
                    BindingMode::InitializeLexical
                };
                generate_binding_pattern_bytecode(gen, pattern, mode, &arg);
            }
        }
    }

    // --- Step 5: Initialize var bindings ---

    if let Some(fsd) = function_scope_data {
        if !has_parameter_expressions {
            // Simple case: vars share the parameter environment.
            for var in &fsd.vars_to_initialize {
                if var.is_parameter {
                    continue;
                }
                if arguments_object_needed && var.name == utf16!("arguments") {
                    continue;
                }

                // Check if this var is a local variable.
                if let Some(local_idx) = find_local_var(gen, &var.name) {
                    let undef = gen.add_constant_undefined();
                    let local = gen.local(local_idx);
                    gen.emit_mov(&local, &undef);
                } else {
                    let id = gen.intern_identifier(var.name.clone());
                    let undef = gen.add_constant_undefined();
                    gen.emit(Instruction::CreateVariable {
                        identifier: id,
                        mode: ENV_MODE_VAR,
                        is_immutable: false,
                        is_global: false,
                        is_strict: false,
                    });
                    gen.emit(Instruction::InitializeVariableBinding {
                        identifier: id,
                        src: undef.operand(),
                        cache: EnvironmentCoordinate::empty(),
                    });
                }
            }
        } else {
            // Parameter expressions: vars get a separate environment.
            let has_non_local_vars = fsd.vars_to_initialize.iter().any(|v| {
                find_local_var(gen, &v.name).is_none()
            });

            if has_non_local_vars {
                gen.emit(Instruction::CreateVariableEnvironment {
                    capacity: fsd.non_local_var_count_for_parameter_expressions as u32,
                });
            }

            for var in &fsd.vars_to_initialize {
                let is_in_parameter_bindings = var.is_parameter
                    || (arguments_object_needed && var.name == utf16!("arguments"));

                let initial_value = if !is_in_parameter_bindings || var.is_function_name {
                    gen.add_constant_undefined()
                } else if let Some(local_idx) = find_local_var(gen, &var.name) {
                    let local = gen.local(local_idx);
                    let tmp = gen.allocate_register();
                    gen.emit_mov(&tmp, &local);
                    tmp
                } else {
                    let id = gen.intern_identifier(var.name.clone());
                    let tmp = gen.allocate_register();
                    gen.emit(Instruction::GetBinding {
                        dst: tmp.operand(),
                        identifier: id,
                        cache: EnvironmentCoordinate::empty(),
                    });
                    tmp
                };

                if let Some(local_idx) = find_local_var(gen, &var.name) {
                    let local = gen.local(local_idx);
                    gen.emit_mov(&local, &initial_value);
                } else {
                    let id = gen.intern_identifier(var.name.clone());
                    gen.emit(Instruction::CreateVariable {
                        identifier: id,
                        mode: ENV_MODE_VAR,
                        is_immutable: false,
                        is_global: false,
                        is_strict: false,
                    });
                    gen.emit(Instruction::InitializeVariableBinding {
                        identifier: id,
                        src: initial_value.operand(),
                        cache: EnvironmentCoordinate::empty(),
                    });
                }
            }
        }
    }

    // --- Step 6: AnnexB function name bindings (non-strict only) ---
    // TODO: Implement AnnexB function hoisting once scope collector tracks it.

    // --- Step 7: Lexical environment for non-local declarations ---

    let has_non_local_lexical_declarations = has_non_local_lexical_decls(body_scope);

    if !strict && has_non_local_lexical_declarations {
        let parent = gen.lexical_environment_register_stack.last().cloned();
        let parent = parent.unwrap_or_else(|| {
            gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT))
        });
        let new_env = gen.allocate_register();
        gen.emit(Instruction::CreateLexicalEnvironment {
            dst: new_env.operand(),
            parent: parent.operand(),
            capacity: 0,
        });
        gen.lexical_environment_register_stack.push(new_env);
    }

    // --- Step 8: Create lexical bindings ---

    for child in &body_scope.children {
        match &child.inner {
            Statement::VariableDeclaration { kind, declarations } => {
                if *kind == DeclarationKind::Let || *kind == DeclarationKind::Const {
                    let is_constant = *kind == DeclarationKind::Const;
                    for decl in declarations {
                        let mut names = Vec::new();
                        collect_target_names(&decl.target, &mut names);
                        for (name, _) in names {
                            let id = gen.intern_identifier(name);
                            gen.emit(Instruction::CreateVariable {
                                identifier: id,
                                mode: ENV_MODE_LEXICAL,
                                is_immutable: is_constant,
                                is_global: false,
                                is_strict: is_constant,
                            });
                        }
                    }
                }
            }
            Statement::ClassDeclaration(class_data) => {
                // Class declarations are lexically scoped (like const).
                if let Some(ref name_ident) = class_data.name {
                    if !name_ident.is_local() {
                        let id = gen.intern_identifier(name_ident.name.clone());
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: ENV_MODE_LEXICAL,
                            is_immutable: false,
                            is_global: false,
                            is_strict: false,
                        });
                    }
                }
            }
            _ => {}
        }
    }

    // --- Step 9: Initialize hoisted function declarations ---

    if let Some(fsd) = function_scope_data {
        for func_to_init in &fsd.functions_to_initialize {
            let child = &body_scope.children[func_to_init.child_index];
            if let Statement::FunctionDeclaration(ref inner_func_data) = child.inner {
                let sfd_index = emit_new_function(gen, inner_func_data, None);

                // Check if the function name identifier is local.
                if let Some(ref name_ident) = inner_func_data.name {
                    if name_ident.is_local() {
                        let local_idx = name_ident.local_index.get();
                        let local = gen.local(local_idx);
                        gen.emit(Instruction::NewFunction {
                            dst: local.operand(),
                            shared_function_data_index: sfd_index,
                            home_object: None,
                            lhs_name: None,
                        });
                        gen.mark_local_initialized(local_idx);
                    } else {
                        let func_reg = gen.allocate_register();
                        gen.emit(Instruction::NewFunction {
                            dst: func_reg.operand(),
                            shared_function_data_index: sfd_index,
                            home_object: None,
                            lhs_name: None,
                        });
                        let id = gen.intern_identifier(name_ident.name.clone());
                        gen.emit(Instruction::SetVariableBinding {
                            identifier: id,
                            src: func_reg.operand(),
                            cache: EnvironmentCoordinate::empty(),
                        });
                    }
                }
            }
        }
    }
}

/// Check if a scope has any non-local lexical declarations.
fn has_non_local_lexical_decls(scope: &ScopeData) -> bool {
    for child in &scope.children {
        match &child.inner {
            Statement::VariableDeclaration { kind, declarations } => {
                if *kind == DeclarationKind::Let || *kind == DeclarationKind::Const {
                    for decl in declarations {
                        let mut names = Vec::new();
                        collect_target_names(&decl.target, &mut names);
                        if !names.is_empty() {
                            return true;
                        }
                    }
                }
            }
            Statement::ClassDeclaration(class_data) => {
                if let Some(ref name_ident) = class_data.name {
                    if !name_ident.is_local() {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Find a local variable index by name.
fn find_local_var(gen: &Generator, name: &[u16]) -> Option<u32> {
    gen.local_variables
        .iter()
        .position(|lv| lv.name == name)
        .map(|i| i as u32)
}

/// Collect bound names from a binding pattern into the parameter_names list.
fn collect_binding_pattern_names(
    pattern: &BindingPattern,
    parameter_names: &mut Vec<(Vec<u16>, bool)>,
    has_duplicates: &mut bool,
) {
    for entry in &pattern.entries {
        // The bound name can be in the alias (for object patterns) or name (for array patterns).
        match &entry.alias {
            BindingEntryAlias::Identifier(ident) => {
                let name = ident.name.clone();
                let is_local = ident.is_local();
                if parameter_names.iter().any(|(n, _)| *n == name) {
                    *has_duplicates = true;
                } else {
                    parameter_names.push((name, is_local));
                }
            }
            BindingEntryAlias::BindingPattern(sub_pattern) => {
                collect_binding_pattern_names(sub_pattern, parameter_names, has_duplicates);
            }
            BindingEntryAlias::Empty => {
                // No alias — the name itself is the binding.
                if let BindingEntryName::Identifier(ident) = &entry.name {
                    let name = ident.name.clone();
                    let is_local = ident.is_local();
                    if parameter_names.iter().any(|(n, _)| *n == name) {
                        *has_duplicates = true;
                    } else {
                        parameter_names.push((name, is_local));
                    }
                }
            }
            BindingEntryAlias::MemberExpression(_) => {}
        }
    }
}

/// Approximate a source expression as a string for error messages.
/// Intern the base expression as an identifier for error messages like
/// "Cannot access property X on null object Y".
fn intern_base_identifier(gen: &mut Generator, base: &Expr) -> Option<IdentifierTableIndex> {
    match &base.inner {
        Expression::Identifier(_)
        | Expression::Member { .. }
        | Expression::This => {
            let s = expression_to_string_approximation(base);
            Some(gen.intern_identifier(s))
        }
        _ => None,
    }
}

fn expression_to_string_approximation(expr: &Expr) -> Vec<u16> {
    match &expr.inner {
        Expression::Identifier(ident) => ident.name.clone(),
        Expression::Member { object, property, computed } => {
            let mut s = expression_to_string_approximation(object);
            if *computed {
                s.extend_from_slice(utf16!("["));
                s.extend(expression_to_string_approximation(property));
                s.extend_from_slice(utf16!("]"));
            } else {
                s.extend_from_slice(utf16!("."));
                s.extend(expression_to_string_approximation(property));
            }
            s
        }
        Expression::StringLiteral(s) => s.clone(),
        Expression::NumericLiteral(n) => {
            let s = format!("{}", n);
            s.encode_utf16().collect()
        }
        Expression::This => utf16!("this").to_vec(),
        _ => utf16!("<expression>").to_vec(),
    }
}
