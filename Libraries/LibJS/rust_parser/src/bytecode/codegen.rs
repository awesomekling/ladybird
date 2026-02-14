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

use std::collections::HashSet;

use crate::ast::*;

use super::generator::{choose_dst, BlockBoundaryType, ConstantValue, FinallyContext, Generator, ScopedOperand};
use super::instruction::Instruction;
use super::operand::*;

/// Generate bytecode for an expression.
pub fn generate_expr(
    expr: &Expr,
    gen: &mut Generator,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let saved_source_start = gen.current_source_start;
    let saved_source_end = gen.current_source_end;
    gen.current_source_start = expr.range.start.offset;
    gen.current_source_end = expr.range.end.offset;

    let result = match &expr.inner {
        // === Literals ===
        Expression::NumericLiteral(value) => Some(gen.add_constant_number(*value)),

        Expression::BooleanLiteral(value) => Some(gen.add_constant_boolean(*value)),

        Expression::NullLiteral => Some(gen.add_constant_null()),

        Expression::StringLiteral(value) => Some(gen.add_constant_string(value.clone())),

        Expression::BigIntLiteral(value) => Some(gen.add_constant_bigint(value.clone())),

        Expression::RegExpLiteral(data) => {
            let source_index = gen.intern_string(&data.pattern);
            let flags_index = gen.intern_string(&data.flags);
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
                        let id = gen.intern_identifier(&ident.name);
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
            // Special case: `#privateId in obj` uses HasPrivateId instead of In.
            if *op == BinaryOp::In {
                if let Expression::PrivateIdentifier(priv_ident) = &lhs.inner {
                    let base = generate_expr(rhs, gen, None)?;
                    let dst = choose_dst(gen, preferred_dst);
                    let id = gen.intern_identifier(&priv_ident.name);
                    gen.emit(Instruction::HasPrivateId {
                        dst: dst.operand(),
                        base: base.operand(),
                        property: id,
                    });
                    return Some(dst);
                }
            }
            let lhs_val = generate_expr(lhs, gen, None)?;
            let rhs_val = generate_expr(rhs, gen, None)?;
            // OPTIMIZATION: constant folding for binary operations on constants.
            if let Some(folded) = try_constant_fold_binary(gen, *op, &lhs_val, &rhs_val) {
                return Some(folded);
            }
            let dst = choose_dst(gen, preferred_dst);
            emit_binary_op(gen, *op, &dst, &lhs_val, &rhs_val);
            Some(dst)
        }

        // === Logical (short-circuit) ===
        Expression::Logical { op, lhs, rhs } => {
            // Logical expressions are not named evaluations.
            gen.pending_lhs_name = None;
            generate_logical(gen, *op, lhs, rhs, preferred_dst)
        }

        // === Conditional (ternary) ===
        Expression::Conditional {
            test,
            consequent,
            alternate,
        } => {
            // Conditional expressions are not named evaluations.
            gen.pending_lhs_name = None;
            generate_conditional(gen, test, consequent, alternate, preferred_dst)
        }

        // === Sequence ===
        Expression::Sequence(exprs) => {
            // Sequence expressions are not named evaluations.
            gen.pending_lhs_name = None;
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

                let id = gen.intern_identifier(&data.name.as_ref().unwrap().name);
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
            let home_object = gen.home_objects.last().map(|ho| ho.operand());
            gen.emit(Instruction::NewFunction {
                dst: dst.operand(),
                shared_function_data_index,
                lhs_name,
                home_object,
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
            // Array literals are not named evaluations.
            gen.pending_lhs_name = None;
            let dst = choose_dst(gen, preferred_dst);

            // If all elements are constant primitives, emit NewPrimitiveArray.
            if !elements.is_empty() && elements.iter().all(|e| match e {
                None => true, // holes
                Some(e) => matches!(e.inner, Expression::NumericLiteral(_) | Expression::BooleanLiteral(_) | Expression::NullLiteral),
            }) {
                let values: Vec<u64> = elements.iter().map(|e| match e {
                    None => nanboxed_empty(),
                    Some(e) => match &e.inner {
                        Expression::NumericLiteral(n) => nanboxed_number(*n),
                        Expression::BooleanLiteral(b) => nanboxed_boolean(*b),
                        Expression::NullLiteral => nanboxed_null(),
                        _ => unreachable!(),
                    },
                }).collect();
                gen.emit(Instruction::NewPrimitiveArray {
                    dst: dst.operand(),
                    element_count: values.len() as u32,
                    elements: values,
                });
                return Some(dst);
            }

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
                        let val = generate_expr_or_undefined(e, gen, None);
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
                            let val = generate_expr_or_undefined(e, gen, None);
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
            let is_super = matches!(object.inner, Expression::Super);
            let obj = generate_expr(object, gen, None)?;
            let base_id = intern_base_identifier(gen, object);
            let dst = choose_dst(gen, preferred_dst);
            if is_super {
                let this_value = emit_resolve_this_binding(gen);
                emit_super_get(gen, &dst, &obj, property, *computed, &this_value);
            } else if *computed {
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
                    let id = gen.intern_identifier(&priv_ident.name);
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
            Some(generate_expr_or_undefined(inner, gen, preferred_dst))
        }

        // === Yield ===
        Expression::Yield {
            argument,
            is_yield_from,
        } => {
            let value = if let Some(arg) = argument {
                generate_expr_or_undefined(arg, gen, None)
            } else {
                gen.add_constant_undefined()
            };

            if *is_yield_from {
                Some(generate_yield_from(gen, value, preferred_dst))
            } else if !gen.is_in_async_generator_function() {
                // Regular generator: yield + completion protocol.
                Some(generate_regular_yield(gen, value, preferred_dst))
            } else {
                // Async generator: full yield protocol.
                Some(generate_async_generator_yield(gen, value, preferred_dst))
            }
        }

        // === Await ===
        Expression::Await(inner) => {
            let value = generate_expr_or_undefined(inner, gen, None);
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
            generate_optional_chain(gen, base, references, preferred_dst).map(|(v, _)| v)
        }

        // === SuperCall ===
        Expression::SuperCall(data) => {
            let dst = choose_dst(gen, preferred_dst);
            let args_array_dst = gen.allocate_register();

            if data.is_synthetic {
                // Synthetic constructor: super(...args) — single spread arg,
                // don't call @@iterator on %Array.prototype%.
                assert!(data.arguments.len() == 1 && data.arguments[0].is_spread);
                let val = generate_expr_or_undefined(&data.arguments[0].value, gen, None);
                gen.emit_mov(&args_array_dst, &val);
            } else {
                // Build arguments array with proper spread handling.
                let first_spread = data.arguments.iter()
                    .position(|a| a.is_spread)
                    .unwrap_or(data.arguments.len());

                let mut pre_holders = Vec::new();
                for arg in &data.arguments[..first_spread] {
                    let val = generate_expr_or_undefined(&arg.value, gen, None);
                    pre_holders.push(gen.copy_if_needed_to_preserve_evaluation_order(&val));
                }
                let pre_ops: Vec<Operand> = pre_holders.iter().map(|a| a.operand()).collect();
                gen.emit(Instruction::NewArray {
                    dst: args_array_dst.operand(),
                    element_count: pre_ops.len() as u32,
                    elements: pre_ops,
                });
                drop(pre_holders);

                for arg in &data.arguments[first_spread..] {
                    let val = generate_expr_or_undefined(&arg.value, gen, None);
                    gen.emit(Instruction::ArrayAppend {
                        dst: args_array_dst.operand(),
                        src: val.operand(),
                        is_spread: arg.is_spread,
                    });
                }
            }

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
    };

    gen.current_source_start = saved_source_start;
    gen.current_source_end = saved_source_end;
    result
}

fn generate_expr_or_undefined(
    expr: &Expr,
    gen: &mut Generator,
    preferred_dst: Option<&ScopedOperand>,
) -> ScopedOperand {
    generate_expr(expr, gen, preferred_dst)
        .unwrap_or_else(|| gen.add_constant_undefined())
}

/// Generate bytecode for a statement.
pub fn generate_stmt(
    stmt: &Stmt,
    gen: &mut Generator,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    let saved_source_start = gen.current_source_start;
    let saved_source_end = gen.current_source_end;
    gen.current_source_start = stmt.range.start.offset;
    gen.current_source_end = stmt.range.end.offset;

    let result = match &stmt.inner {
        Statement::Empty | Statement::Error | Statement::ErrorDeclaration => None,
        Statement::Debugger => None,

        // === ExpressionStatement ===
        Statement::Expression(expr) => generate_expr(expr, gen, preferred_dst),

        // === Block ===
        Statement::Block(ref scope) => generate_block_statement(gen, &scope.borrow(), preferred_dst),

        // === FunctionBody ===
        Statement::FunctionBody { ref scope, .. } => generate_scope_children(gen, &scope.borrow(), preferred_dst),

        // === Program ===
        // Note: GlobalDeclarationInstantiation (GDI) runs before this bytecode.
        // GDI already hoists top-level function declarations and handles Annex B
        // at the global scope. We only need to generate the program body here.
        Statement::Program(ref data) => {
            // Populate annexb_function_names so switch codegen can emit
            // GetBinding + SetVariableBinding for AnnexB-hoisted functions.
            let scope = data.scope.borrow();
            for name in &scope.annexb_function_names {
                gen.annexb_function_names.insert(name.clone());
            }
            generate_scope_children(gen, &scope, preferred_dst)
        }

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
                Some(expr) => generate_expr_or_undefined(expr, gen, None),
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

        // === FunctionDeclaration ===
        Statement::FunctionDeclaration(func_data) => {
            if func_data.is_hoisted {
                // Annex B.3.3: Copy the function from the lexical (block) scope
                // to the var scope.
                if let Some(ref name_ident) = func_data.name {
                    let id = gen.intern_identifier(&name_ident.name);
                    let value = gen.allocate_register();
                    gen.emit(Instruction::GetBinding {
                        dst: value.operand(),
                        identifier: id,
                        cache: EnvironmentCoordinate::empty(),
                    });
                    gen.emit(Instruction::SetVariableBinding {
                        identifier: id,
                        src: value.operand(),
                        cache: EnvironmentCoordinate::empty(),
                    });
                }
            }
            None
        }

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
            // Per spec 13.11.7 step 10: if body completion value is empty,
            // return NormalCompletion(undefined).
            Some(result.unwrap_or_else(|| gen.add_constant_undefined()))
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
        Statement::UsingDeclaration { .. } => {
            // Disposal semantics are not yet implemented in the Rust codegen.
            // Emit the same TODO error as the C++ codegen to match behavior.
            let error = gen.allocate_register();
            let msg = gen.intern_string(utf16!("TODO: UsingDeclaration"));
            gen.emit(Instruction::NewTypeError {
                dst: error.operand(),
                error_string: msg,
            });
            gen.emit(Instruction::Throw { src: error.operand() });
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
                    let id = gen.intern_identifier(&name_ident.name);
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
        Statement::ClassFieldInitializer { expression, field_name } => {
            if !field_name.is_empty() {
                gen.pending_lhs_name = Some(gen.intern_identifier(&field_name));
            }
            let value = generate_expr_or_undefined(expression, gen, None);
            gen.pending_lhs_name = None;
            gen.emit(Instruction::Return {
                value: value.operand(),
            });
            None
        }
    };

    gen.current_source_start = saved_source_start;
    gen.current_source_end = saved_source_end;
    result
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

/// Completion::Type values matching C++ enum.
#[repr(u32)]
enum CompletionType {
    Normal = 1,
    Return = 4,
    Throw = 5,
}

impl CompletionType {
    fn to_f64(self) -> f64 {
        self as u32 as f64
    }
}

/// Environment binding mode matching C++ EnvironmentMode.
#[repr(u32)]
enum EnvironmentMode {
    Lexical = 0,
    Var = 1,
}

/// Arguments object creation mode.
#[repr(u32)]
enum ArgumentsKind {
    Mapped = 0,
    Unmapped = 1,
}

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
    let normal_type = gen.add_constant_number(CompletionType::Normal.to_f64());
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
    let normal_type = gen.add_constant_number(CompletionType::Normal.to_f64());
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
    let throw_type = gen.add_constant_number(CompletionType::Throw.to_f64());
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

    // Return block: route through finally context if active, else yield done.
    gen.switch_to_basic_block(return_block);
    gen.generate_return(&received_completion_value);

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
    let return_type = gen.add_constant_number(CompletionType::Return.to_f64());
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
    let throw_type = gen.add_constant_number(CompletionType::Throw.to_f64());
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
    let normal_type = gen.add_constant_number(CompletionType::Normal.to_f64());
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

    // Return: route through finally context if active, else yield done.
    gen.switch_to_basic_block(return_value_block);
    gen.generate_return(&received_completion_value);

    // Normal: return the value.
    gen.switch_to_basic_block(normal_cont);
    let dst = choose_dst(gen, preferred_dst);
    gen.emit_mov(&dst, &received_completion_value);
    dst
}

/// Yield* (yield from) delegation.
///
/// Implements the iterator delegation protocol from
/// https://tc39.es/ecma262/#sec-generator-function-definitions-runtime-semantics-evaluation
///
/// The delegating generator forwards next/throw/return to the inner iterator.
fn generate_yield_from(
    gen: &mut Generator,
    value: ScopedOperand,
    preferred_dst: Option<&ScopedOperand>,
) -> ScopedOperand {
    let is_async = gen.is_in_async_generator_function();

    let received_completion = gen.allocate_register();
    let received_completion_type = gen.allocate_register();
    let received_completion_value = gen.allocate_register();

    // 4. Let iteratorRecord be ? GetIterator(value, generatorKind).
    let iterator = gen.allocate_register();
    let next_method = gen.allocate_register();
    let iterator_done_property = gen.allocate_register();
    let hint: u32 = if is_async { 1 } else { 0 };
    gen.emit(Instruction::GetIterator {
        dst_iterator_object: iterator.operand(),
        dst_iterator_next: next_method.operand(),
        dst_iterator_done: iterator_done_property.operand(),
        iterable: value.operand(),
        hint,
    });

    // 6. Let received be NormalCompletion(undefined).
    let normal_const = gen.add_constant_number(CompletionType::Normal.to_f64());
    gen.emit_mov(&received_completion_type, &normal_const);
    let undef = gen.add_constant_undefined();
    gen.emit_mov(&received_completion_value, &undef);

    // 7. Repeat,
    let loop_block = gen.make_block();
    let continuation_block = gen.make_block();
    let loop_end_block = gen.make_block();
    gen.emit(Instruction::Jump {
        target: Label(loop_block as u32),
    });
    gen.switch_to_basic_block(loop_block);

    // Branch on received.[[Type]].
    let type_is_normal_block = gen.make_block();
    let is_type_throw_block = gen.make_block();
    let is_normal = gen.allocate_register();
    gen.emit(Instruction::StrictlyEquals {
        dst: is_normal.operand(),
        lhs: received_completion_type.operand(),
        rhs: normal_const.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_normal.operand(),
        true_target: Label(type_is_normal_block as u32),
        false_target: Label(is_type_throw_block as u32),
    });

    // =========================================================================
    // a. If received.[[Type]] is normal, then
    // =========================================================================
    gen.switch_to_basic_block(type_is_normal_block);

    // i. Let innerResult be ? Call(next, iterator, « received.[[Value]] »).
    let inner_result = gen.allocate_register();
    gen.emit(Instruction::Call {
        dst: inner_result.operand(),
        callee: next_method.operand(),
        this_value: iterator.operand(),
        argument_count: 1,
        expression_string: None,
        arguments: vec![received_completion_value.operand()],
    });

    // ii. If generatorKind is async, set innerResult to ? Await(innerResult).
    if is_async {
        let awaited = generate_await_with_completions(
            gen,
            inner_result.clone(),
            &received_completion,
            &received_completion_type,
            &received_completion_value,
        );
        gen.emit_mov(&inner_result, &awaited);
    }

    // iii. If innerResult is not an Object, throw a TypeError exception.
    gen.emit(Instruction::ThrowIfNotObject {
        src: inner_result.operand(),
    });

    // iv. Let done be ? IteratorComplete(innerResult).
    let done = gen.allocate_register();
    emit_get_by_id(gen, &done, &inner_result, utf16!("done"), None);

    // v. If done is true, then return ? IteratorValue(innerResult).
    let type_is_normal_done_block = gen.make_block();
    let type_is_normal_not_done_block = gen.make_block();
    gen.emit(Instruction::JumpIf {
        condition: done.operand(),
        true_target: Label(type_is_normal_done_block as u32),
        false_target: Label(type_is_normal_not_done_block as u32),
    });

    gen.switch_to_basic_block(type_is_normal_done_block);
    let return_value = gen.allocate_register();
    emit_get_by_id(gen, &return_value, &inner_result, utf16!("value"), None);
    gen.emit(Instruction::Jump {
        target: Label(loop_end_block as u32),
    });

    // vi/vii. Yield IteratorValue(innerResult), receive new completion.
    gen.switch_to_basic_block(type_is_normal_not_done_block);
    let current_value = gen.allocate_register();
    emit_get_by_id(gen, &current_value, &inner_result, utf16!("value"), None);

    generate_yield_for_yield_from(
        gen,
        Label(continuation_block as u32),
        &current_value,
        &received_completion,
        &received_completion_type,
        &received_completion_value,
        is_async,
    );

    // =========================================================================
    // b. Else if received.[[Type]] is throw, then
    // =========================================================================
    gen.switch_to_basic_block(is_type_throw_block);
    let type_is_throw_block = gen.make_block();
    let type_is_return_block = gen.make_block();
    let throw_const = gen.add_constant_number(CompletionType::Throw.to_f64());
    let is_throw = gen.allocate_register();
    gen.emit(Instruction::StrictlyEquals {
        dst: is_throw.operand(),
        lhs: received_completion_type.operand(),
        rhs: throw_const.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_throw.operand(),
        true_target: Label(type_is_throw_block as u32),
        false_target: Label(type_is_return_block as u32),
    });

    gen.switch_to_basic_block(type_is_throw_block);

    // i. Let throw be ? GetMethod(iterator, "throw").
    let throw_method = gen.allocate_register();
    let throw_key = gen.intern_property_key(utf16!("throw"));
    gen.emit(Instruction::GetMethod {
        dst: throw_method.operand(),
        object: iterator.operand(),
        property: throw_key,
    });

    // ii. If throw is not undefined, then
    let throw_method_defined_block = gen.make_block();
    let throw_method_undefined_block = gen.make_block();
    gen.emit(Instruction::JumpUndefined {
        condition: throw_method.operand(),
        true_target: Label(throw_method_undefined_block as u32),
        false_target: Label(throw_method_defined_block as u32),
    });

    gen.switch_to_basic_block(throw_method_defined_block);

    // 1. Let innerResult be ? Call(throw, iterator, « received.[[Value]] »).
    gen.emit(Instruction::Call {
        dst: inner_result.operand(),
        callee: throw_method.operand(),
        this_value: iterator.operand(),
        argument_count: 1,
        expression_string: None,
        arguments: vec![received_completion_value.operand()],
    });

    // 2. If generatorKind is async, set innerResult to ? Await(innerResult).
    if is_async {
        let awaited = generate_await_with_completions(
            gen,
            inner_result.clone(),
            &received_completion,
            &received_completion_type,
            &received_completion_value,
        );
        gen.emit_mov(&inner_result, &awaited);
    }

    // 4. If innerResult is not an Object, throw a TypeError exception.
    gen.emit(Instruction::ThrowIfNotObject {
        src: inner_result.operand(),
    });

    // 5. Let done be ? IteratorComplete(innerResult).
    emit_get_by_id(gen, &done, &inner_result, utf16!("done"), None);

    // 6. If done is true, return ? IteratorValue(innerResult).
    let type_is_throw_done_block = gen.make_block();
    let type_is_throw_not_done_block = gen.make_block();
    gen.emit(Instruction::JumpIf {
        condition: done.operand(),
        true_target: Label(type_is_throw_done_block as u32),
        false_target: Label(type_is_throw_not_done_block as u32),
    });

    gen.switch_to_basic_block(type_is_throw_done_block);
    emit_get_by_id(gen, &return_value, &inner_result, utf16!("value"), None);
    gen.emit(Instruction::Jump {
        target: Label(loop_end_block as u32),
    });

    // 7/8. Yield IteratorValue(innerResult), receive new completion.
    gen.switch_to_basic_block(type_is_throw_not_done_block);
    let yield_value = gen.allocate_register();
    emit_get_by_id(gen, &yield_value, &inner_result, utf16!("value"), None);
    generate_yield_for_yield_from(
        gen,
        Label(continuation_block as u32),
        &yield_value,
        &received_completion,
        &received_completion_type,
        &received_completion_value,
        is_async,
    );

    // throw is undefined: close iterator, throw TypeError.
    gen.switch_to_basic_block(throw_method_undefined_block);

    if is_async {
        // AsyncIteratorClose: get return method, call it, await, check object.
        let return_method = gen.allocate_register();
        let return_key = gen.intern_property_key(utf16!("return"));
        gen.emit(Instruction::GetMethod {
            dst: return_method.operand(),
            object: iterator.operand(),
            property: return_key,
        });

        let call_return_block = gen.make_block();
        let after_close = gen.make_block();
        gen.emit(Instruction::JumpUndefined {
            condition: return_method.operand(),
            true_target: Label(after_close as u32),
            false_target: Label(call_return_block as u32),
        });
        gen.switch_to_basic_block(call_return_block);

        let close_result = gen.allocate_register();
        gen.emit(Instruction::Call {
            dst: close_result.operand(),
            callee: return_method.operand(),
            this_value: iterator.operand(),
            argument_count: 0,
            expression_string: None,
            arguments: vec![],
        });

        let awaited = generate_await_with_completions(
            gen,
            close_result,
            &received_completion,
            &received_completion_type,
            &received_completion_value,
        );
        gen.emit(Instruction::ThrowIfNotObject {
            src: awaited.operand(),
        });

        gen.emit(Instruction::Jump {
            target: Label(after_close as u32),
        });
        gen.switch_to_basic_block(after_close);
    } else {
        // Sync: IteratorClose with Normal completion.
        let undef = gen.add_constant_undefined();
        gen.emit(Instruction::IteratorClose {
            iterator_object: iterator.operand(),
            iterator_next: next_method.operand(),
            iterator_done: done.operand(),
            completion_type: CompletionType::Normal as u32,
            completion_value: undef.operand(),
        });
    }

    // Throw a TypeError: iterator does not have a throw method.
    let exception = gen.allocate_register();
    let error_msg = utf16!("yield* protocol violation: iterator must have a throw method").to_vec();
    let error_string = gen.intern_string(&error_msg);
    gen.emit(Instruction::NewTypeError {
        dst: exception.operand(),
        error_string,
    });
    gen.emit(Instruction::Throw {
        src: exception.operand(),
    });

    // =========================================================================
    // c. Else (received.[[Type]] is return)
    // =========================================================================
    gen.switch_to_basic_block(type_is_return_block);

    // ii. Let return be ? GetMethod(iterator, "return").
    let return_method = gen.allocate_register();
    let return_key = gen.intern_property_key(utf16!("return"));
    gen.emit(Instruction::GetMethod {
        dst: return_method.operand(),
        object: iterator.operand(),
        property: return_key,
    });

    // iii. If return is undefined, then return received.[[Value]].
    let return_is_undefined_block = gen.make_block();
    let return_is_defined_block = gen.make_block();
    gen.emit(Instruction::JumpUndefined {
        condition: return_method.operand(),
        true_target: Label(return_is_undefined_block as u32),
        false_target: Label(return_is_defined_block as u32),
    });

    gen.switch_to_basic_block(return_is_undefined_block);
    // 1. If generatorKind is async, set received.[[Value]] to ? Await(received.[[Value]]).
    if is_async {
        generate_await_with_completions(
            gen,
            received_completion_value.clone(),
            &received_completion,
            &received_completion_type,
            &received_completion_value,
        );
    }
    // 2. Return received (return completion).
    gen.generate_return(&received_completion_value);

    gen.switch_to_basic_block(return_is_defined_block);

    // iv. Let innerReturnResult be ? Call(return, iterator, « received.[[Value]] »).
    let inner_return_result = gen.allocate_register();
    gen.emit(Instruction::Call {
        dst: inner_return_result.operand(),
        callee: return_method.operand(),
        this_value: iterator.operand(),
        argument_count: 1,
        expression_string: None,
        arguments: vec![received_completion_value.operand()],
    });

    // v. If generatorKind is async, set innerReturnResult to ? Await(innerReturnResult).
    if is_async {
        let awaited = generate_await_with_completions(
            gen,
            inner_return_result.clone(),
            &received_completion,
            &received_completion_type,
            &received_completion_value,
        );
        gen.emit_mov(&inner_return_result, &awaited);
    }

    // vi. If innerReturnResult is not an Object, throw a TypeError exception.
    gen.emit(Instruction::ThrowIfNotObject {
        src: inner_return_result.operand(),
    });

    // vii. Let done be ? IteratorComplete(innerReturnResult).
    emit_get_by_id(gen, &done, &inner_return_result, utf16!("done"), None);

    // viii. If done is true, return IteratorValue(innerReturnResult).
    let type_is_return_done_block = gen.make_block();
    let type_is_return_not_done_block = gen.make_block();
    gen.emit(Instruction::JumpIf {
        condition: done.operand(),
        true_target: Label(type_is_return_done_block as u32),
        false_target: Label(type_is_return_not_done_block as u32),
    });

    gen.switch_to_basic_block(type_is_return_done_block);
    let inner_return_result_value = gen.allocate_register();
    emit_get_by_id(gen, &inner_return_result_value, &inner_return_result, utf16!("value"), None);
    gen.generate_return(&inner_return_result_value);

    // ix/x. Yield IteratorValue(innerReturnResult), receive new completion.
    gen.switch_to_basic_block(type_is_return_not_done_block);
    let received = gen.allocate_register();
    emit_get_by_id(gen, &received, &inner_return_result, utf16!("value"), None);
    generate_yield_for_yield_from(
        gen,
        Label(continuation_block as u32),
        &received,
        &received_completion,
        &received_completion_type,
        &received_completion_value,
        is_async,
    );

    // =========================================================================
    // Continuation block: resume after any yield, extract completion, loop back.
    // =========================================================================
    gen.switch_to_basic_block(continuation_block);
    let acc = gen.accumulator();
    gen.emit_mov(&received_completion, &acc);
    gen.emit(Instruction::GetCompletionFields {
        type_dst: received_completion_type.operand(),
        value_dst: received_completion_value.operand(),
        completion: received_completion.operand(),
    });
    gen.emit(Instruction::Jump {
        target: Label(loop_block as u32),
    });

    // =========================================================================
    // Loop end: return the accumulated return_value.
    // =========================================================================
    gen.switch_to_basic_block(loop_end_block);
    let dst = choose_dst(gen, preferred_dst);
    gen.emit_mov(&dst, &return_value);
    dst
}

/// Yield within yield* delegation, handling both sync and async generators.
///
/// For sync generators: just emit Yield to the continuation label.
/// For async generators: emit Yield, then handle AsyncGeneratorUnwrapYieldResumption
/// (if type is return, await the value, adjust type accordingly).
fn generate_yield_for_yield_from(
    gen: &mut Generator,
    continuation_label: Label,
    argument: &ScopedOperand,
    received_completion: &ScopedOperand,
    received_completion_type: &ScopedOperand,
    received_completion_value: &ScopedOperand,
    is_async: bool,
) {
    if !is_async {
        // Sync generator: just yield directly.
        gen.emit(Instruction::Yield {
            continuation_label: Some(continuation_label),
            value: argument.operand(),
        });
        return;
    }

    // Async generator: Yield, then UnwrapYieldResumption.
    let unwrap_block = gen.make_block();
    gen.emit(Instruction::Yield {
        continuation_label: Some(Label(unwrap_block as u32)),
        value: argument.operand(),
    });
    gen.switch_to_basic_block(unwrap_block);

    let acc = gen.accumulator();
    gen.emit_mov(received_completion, &acc);
    gen.emit(Instruction::GetCompletionFields {
        type_dst: received_completion_type.operand(),
        value_dst: received_completion_value.operand(),
        completion: received_completion.operand(),
    });

    // If resumptionValue.[[Type]] is not return, jump to continuation.
    let return_block = gen.make_block();
    let is_not_return = gen.allocate_register();
    let return_type = gen.add_constant_number(CompletionType::Return.to_f64());
    gen.emit(Instruction::StrictlyInequals {
        dst: is_not_return.operand(),
        lhs: received_completion_type.operand(),
        rhs: return_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_not_return.operand(),
        true_target: continuation_label,
        false_target: Label(return_block as u32),
    });

    // Return path: Await(resumptionValue.[[Value]]).
    gen.switch_to_basic_block(return_block);
    generate_await_with_completions(
        gen,
        received_completion_value.clone(),
        received_completion,
        received_completion_type,
        received_completion_value,
    );

    // If awaited.[[Type]] is throw, jump to continuation (which will handle it).
    let awaited_normal_block = gen.make_block();
    let is_throw = gen.allocate_register();
    let throw_type = gen.add_constant_number(CompletionType::Throw.to_f64());
    gen.emit(Instruction::StrictlyEquals {
        dst: is_throw.operand(),
        lhs: received_completion_type.operand(),
        rhs: throw_type.operand(),
    });
    gen.emit(Instruction::JumpIf {
        condition: is_throw.operand(),
        true_target: continuation_label,
        false_target: Label(awaited_normal_block as u32),
    });

    // awaited.[[Type]] is normal: set type to Return and jump to continuation.
    gen.switch_to_basic_block(awaited_normal_block);
    gen.emit(Instruction::SetCompletionType {
        completion: received_completion.operand(),
        completion_type: CompletionType::Return as u32,
    });
    gen.emit(Instruction::Jump {
        target: continuation_label,
    });
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
            if ident.local_type.get() == LocalType::Argument {
                // Arguments are initialized to undefined by default, so we
                // need to replace the value with the empty sentinel to
                // trigger the TDZ check.
                let empty = gen.add_constant_empty();
                gen.emit_mov(&local, &empty);
            }
            gen.emit(Instruction::ThrowIfTDZ {
                src: local.operand(),
            });
        }
        return Some(local);
    }

    let dst = choose_dst(gen, preferred_dst);
    if ident.is_global.get() {
        let id = gen.intern_identifier(&ident.name);
        let cache = gen.next_global_variable_cache();
        gen.emit(Instruction::GetGlobal {
            dst: dst.operand(),
            identifier: id,
            cache_index: cache,
        });
    } else {
        let id = gen.intern_identifier(&ident.name);
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
    let pred = generate_expr_or_undefined(predicate, gen, None);

    let completion = if gen.must_propagate_completion {
        let reg = choose_dst(gen, preferred_dst);
        let undef = gen.add_constant_undefined();
        gen.emit_mov(&reg, &undef);
        Some(reg)
    } else {
        None
    };

    let true_block = gen.make_block();
    let false_block = gen.make_block();
    let has_alternate = alternate.is_some();
    let end_block = if has_alternate { gen.make_block() } else { false_block };

    gen.emit(Instruction::JumpIf {
        condition: pred.operand(),
        true_target: Label(true_block as u32),
        false_target: Label(false_block as u32),
    });

    // Consequent
    gen.switch_to_basic_block(true_block);
    let saved_completion = gen.current_completion_register.clone();
    if let Some(ref c) = completion {
        gen.current_completion_register = Some(c.clone());
    }
    let cons_result = generate_stmt(consequent, gen, preferred_dst);
    if !gen.is_current_block_terminated() {
        if let (Some(ref c), Some(ref val)) = (&completion, &cons_result) {
            gen.emit_mov(c, val);
        }
        gen.emit(Instruction::Jump {
            target: Label(end_block as u32),
        });
    }
    gen.current_completion_register = saved_completion.clone();

    // Alternate
    if let Some(alt) = alternate {
        gen.switch_to_basic_block(false_block);
        if let Some(ref c) = completion {
            gen.current_completion_register = Some(c.clone());
        }
        let alt_result = generate_stmt(alt, gen, preferred_dst);
        if !gen.is_current_block_terminated() {
            if let (Some(ref c), Some(ref val)) = (&completion, &alt_result) {
                gen.emit_mov(c, val);
            }
            gen.emit(Instruction::Jump {
                target: Label(end_block as u32),
            });
        }
        gen.current_completion_register = saved_completion;
    }

    gen.switch_to_basic_block(end_block);
    completion
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

    let completion = if gen.must_propagate_completion {
        let reg = gen.allocate_register();
        let undef = gen.add_constant_undefined();
        gen.emit_mov(&reg, &undef);
        Some(reg)
    } else {
        None
    };

    gen.emit(Instruction::Jump {
        target: Label(test_block as u32),
    });

    gen.switch_to_basic_block(test_block);
    let test_val = generate_expr_or_undefined(test, gen, None);
    gen.emit(Instruction::JumpIf {
        condition: test_val.operand(),
        true_target: Label(body_block as u32),
        false_target: Label(end_block as u32),
    });

    gen.switch_to_basic_block(body_block);
    let labels = std::mem::take(&mut gen.pending_labels);
    gen.begin_continuable_scope(Label(test_block as u32), labels.clone(), completion.clone());
    gen.begin_breakable_scope(Label(end_block as u32), labels, completion.clone());

    let saved_completion = gen.current_completion_register.clone();
    if let Some(ref c) = completion {
        gen.current_completion_register = Some(c.clone());
    }
    let body_result = generate_stmt(body, gen, preferred_dst);
    if !gen.is_current_block_terminated() {
        if let (Some(ref c), Some(ref body_val)) = (&completion, &body_result) {
            gen.emit_mov(c, body_val);
        }
    }
    gen.current_completion_register = saved_completion;

    gen.end_breakable_scope();
    gen.end_continuable_scope();
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(test_block as u32),
        });
    }

    gen.switch_to_basic_block(end_block);
    completion
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

    let completion = if gen.must_propagate_completion {
        let reg = gen.allocate_register();
        let undef = gen.add_constant_undefined();
        gen.emit_mov(&reg, &undef);
        Some(reg)
    } else {
        None
    };

    gen.emit(Instruction::Jump {
        target: Label(body_block as u32),
    });

    gen.switch_to_basic_block(body_block);
    let labels = std::mem::take(&mut gen.pending_labels);
    gen.begin_continuable_scope(Label(test_block as u32), labels.clone(), completion.clone());
    gen.begin_breakable_scope(Label(end_block as u32), labels, completion.clone());

    let saved_completion = gen.current_completion_register.clone();
    if let Some(ref c) = completion {
        gen.current_completion_register = Some(c.clone());
    }
    let body_result = generate_stmt(body, gen, preferred_dst);
    if !gen.is_current_block_terminated() {
        if let (Some(ref c), Some(ref body_val)) = (&completion, &body_result) {
            gen.emit_mov(c, body_val);
        }
    }
    gen.current_completion_register = saved_completion;

    gen.end_breakable_scope();
    gen.end_continuable_scope();
    if !gen.is_current_block_terminated() {
        gen.emit(Instruction::Jump {
            target: Label(test_block as u32),
        });
    }

    gen.switch_to_basic_block(test_block);
    let test_val = generate_expr_or_undefined(test, gen, None);
    gen.emit(Instruction::JumpIf {
        condition: test_val.operand(),
        true_target: Label(body_block as u32),
        false_target: Label(end_block as u32),
    });

    gen.switch_to_basic_block(end_block);
    completion
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
                    let parent = gen.current_lexical_environment();
                    let new_env = gen.allocate_register();
                    gen.emit(Instruction::CreateLexicalEnvironment {
                        dst: new_env.operand(),
                        parent: parent.operand(),
                        capacity: non_local_names.len() as u32,
                    });
                    gen.lexical_environment_register_stack.push(new_env);

                    for (name, _) in &non_local_names {
                        let id = gen.intern_identifier(&name);
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: EnvironmentMode::Lexical as u32,
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

    let completion = if gen.must_propagate_completion {
        let reg = gen.allocate_register();
        let undef = gen.add_constant_undefined();
        gen.emit_mov(&reg, &undef);
        Some(reg)
    } else {
        None
    };

    gen.emit(Instruction::Jump {
        target: Label(test_block as u32),
    });

    // Test
    gen.switch_to_basic_block(test_block);
    if let Some(test) = test {
        let test_val = generate_expr_or_undefined(test, gen, None);
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
    let labels = std::mem::take(&mut gen.pending_labels);
    gen.begin_continuable_scope(Label(update_block as u32), labels.clone(), completion.clone());
    gen.begin_breakable_scope(Label(end_block as u32), labels, completion.clone());

    let saved_completion = gen.current_completion_register.clone();
    if let Some(ref c) = completion {
        gen.current_completion_register = Some(c.clone());
    }
    let body_result = generate_stmt(body, gen, preferred_dst);
    if !gen.is_current_block_terminated() {
        if let (Some(ref c), Some(ref body_val)) = (&completion, &body_result) {
            gen.emit_mov(c, body_val);
        }
    }
    gen.current_completion_register = saved_completion;

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
            let parent = gen.current_lexical_environment();
            gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
        }
    }

    completion
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
        let id = gen.intern_identifier(&name);
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
    let parent = gen.current_lexical_environment();
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
            mode: EnvironmentMode::Lexical as u32,
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
        if gen.must_propagate_completion {
            if let Some(ref val) = result {
                last_result = result.clone();
                if !gen.is_current_block_terminated() {
                    if let Some(ref completion_reg) = gen.current_completion_register.clone() {
                        gen.emit_mov(completion_reg, val);
                    }
                }
            }
        } else if result.is_some() {
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
    if did_create_env {
        gen.start_boundary(BlockBoundaryType::LeaveLexicalEnvironment);
    }
    let result = generate_scope_children(gen, scope, preferred_dst);

    if did_create_env {
        gen.end_boundary(BlockBoundaryType::LeaveLexicalEnvironment);
        gen.lexical_environment_register_stack.pop();
        if !gen.is_current_block_terminated() {
            let parent = gen.current_lexical_environment();
            gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
        }
    }
    result
}

/// Create a lexical environment for a block with non-local lexical declarations.
/// Returns true if an environment was created.
fn create_lexical_bindings_for_block<'a>(gen: &mut Generator, children: impl Iterator<Item = &'a Stmt>) {
    let mut func_binding_created: HashSet<Vec<u16>> = HashSet::new();
    for child in children {
        match &child.inner {
            Statement::VariableDeclaration { kind, declarations } => {
                if *kind == DeclarationKind::Let || *kind == DeclarationKind::Const {
                    let is_constant = *kind == DeclarationKind::Const;
                    for decl in declarations {
                        let mut names = Vec::new();
                        collect_target_names(&decl.target, &mut names);
                        for (name, _) in &names {
                            let id = gen.intern_identifier(&name);
                            gen.emit(Instruction::CreateVariable {
                                identifier: id,
                                mode: EnvironmentMode::Lexical as u32,
                                is_immutable: is_constant,
                                is_global: false,
                                is_strict: is_constant,
                            });
                        }
                    }
                }
            }
            Statement::ClassDeclaration(class_data) => {
                if let Some(ref name_ident) = class_data.name {
                    if !name_ident.is_local() {
                        let id = gen.intern_identifier(&name_ident.name);
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: EnvironmentMode::Lexical as u32,
                            is_immutable: false,
                            is_global: false,
                            is_strict: false,
                        });
                    }
                }
            }
            Statement::FunctionDeclaration(func_data) => {
                if let Some(ref name_ident) = func_data.name {
                    if !name_ident.is_local() && func_binding_created.insert(name_ident.name.clone()) {
                        let id = gen.intern_identifier(&name_ident.name);
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: EnvironmentMode::Lexical as u32,
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
}

fn emit_block_declaration_instantiation(gen: &mut Generator, scope: &ScopeData) -> bool {
    if !needs_block_declaration_instantiation(scope) {
        return false;
    }

    let parent = gen.current_lexical_environment();
    let new_env = gen.allocate_register();
    gen.emit(Instruction::CreateLexicalEnvironment {
        dst: new_env.operand(),
        parent: parent.operand(),
        capacity: 0,
    });
    gen.lexical_environment_register_stack.push(new_env);

    create_lexical_bindings_for_block(gen, scope.children.iter());

    // Pass 2: Instantiate function declarations.
    // For duplicate names, only instantiate the last declaration (it wins).
    // Process in reverse to find which index is "last" per name.
    let mut last_func_indices: Vec<(Vec<u16>, usize)> = Vec::new();
    for (i, child) in scope.children.iter().enumerate().rev() {
        if let Statement::FunctionDeclaration(func_data) = &child.inner {
            if let Some(ref name_ident) = func_data.name {
                if !last_func_indices.iter().any(|(n, _)| *n == name_ident.name) {
                    last_func_indices.push((name_ident.name.clone(), i));
                }
            }
        }
    }
    // Emit in forward order.
    last_func_indices.reverse();
    for (_, idx) in &last_func_indices {
        if let Statement::FunctionDeclaration(func_data) = &scope.children[*idx].inner {
            let sfd_index = emit_new_function(gen, func_data, None);
            let fo = gen.allocate_register();
            gen.emit(Instruction::NewFunction {
                dst: fo.operand(),
                shared_function_data_index: sfd_index,
                home_object: None,
                lhs_name: None,
            });
            if let Some(ref name_ident) = func_data.name {
                if name_ident.is_local() {
                    let local_idx = name_ident.local_index.get();
                    let local = gen.local(local_idx);
                    gen.emit_mov(&local, &fo);
                    gen.mark_local_initialized(local_idx);
                } else {
                    let id = gen.intern_identifier(&name_ident.name);
                    gen.emit(Instruction::InitializeLexicalBinding {
                        identifier: id,
                        src: fo.operand(),
                        cache: EnvironmentCoordinate::empty(),
                    });
                }
            }
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
            gen.pending_lhs_name = Some(gen.intern_identifier(&ident.name));
        }
        let init_value = decl.init.as_ref().and_then(|init| generate_expr(init, gen, None));
        gen.pending_lhs_name = None;

        match &decl.target {
            VariableDeclaratorTarget::Identifier(ident) => {
                // var declarations without initializer don't need to assign undefined.
                // The FDI already handles initialization for var bindings.
                if init_value.is_none() && kind == DeclarationKind::Var {
                    if ident.is_local() {
                        gen.mark_local_initialized(ident.local_index.get());
                    }
                    continue;
                }
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
                    let id = gen.intern_identifier(&ident.name);
                    match kind {
                        DeclarationKind::Var => {
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
            Some(gen.intern_string(&ident.name))
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
            Some(gen.intern_string(&s))
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
                let is_super = matches!(object.inner, Expression::Super);
                let obj = generate_expr_or_undefined(object, gen, None);
                let base_id = intern_base_identifier(gen, object);
                let method = gen.allocate_register();
                if *computed {
                    let prop = generate_expr_or_undefined(property, gen, None);
                    gen.emit(Instruction::GetByValue {
                        dst: method.operand(),
                        base: obj.operand(),
                        property: prop.operand(),
                        base_identifier: base_id,
                    });
                } else if let Expression::Identifier(ident) = &property.inner {
                    emit_get_by_id(gen, &method, &obj, &ident.name, base_id);
                } else if let Expression::PrivateIdentifier(priv_ident) = &property.inner {
                    let id = gen.intern_identifier(&priv_ident.name);
                    gen.emit(Instruction::GetPrivateById {
                        dst: method.operand(),
                        base: obj.operand(),
                        property: id,
                    });
                }
                // For super.method(), the this value should be the
                // current this binding, not the super base.
                let this_val = if is_super {
                    gen.emit(Instruction::ResolveThisBinding);
                    gen.this_value()
                } else {
                    obj
                };
                (method, Some(this_val))
            }
            Expression::Identifier(ident) if !ident.is_local() && !ident.is_global.get() => {
                // Non-local, non-global identifier: use GetCalleeAndThisFromEnvironment
                // to properly handle with-statement bindings and eval.
                let callee_reg = gen.allocate_register();
                let this_reg = gen.allocate_register();
                let id = gen.intern_identifier(&ident.name);
                gen.emit(Instruction::GetCalleeAndThisFromEnvironment {
                    callee: callee_reg.operand(),
                    this_value: this_reg.operand(),
                    identifier: id,
                    cache: EnvironmentCoordinate::empty(),
                });
                (callee_reg, Some(this_reg))
            }
            Expression::OptionalChain { base, references } => {
                let (callee, this_val) = generate_optional_chain(gen, base, references, None)?;
                (callee, Some(this_val))
            }
            _ => {
                let callee = generate_expr_or_undefined(&data.callee, gen, None);
                (callee, None)
            }
        }
    } else {
        let callee = generate_expr_or_undefined(&data.callee, gen, None);
        (callee, None)
    };

    // Copy callee/this into fresh registers so argument evaluation
    // cannot mutate them (e.g. `foo.bar(foo = null)`).
    let this_value = this_value.map(|tv| gen.copy_if_needed_to_preserve_evaluation_order(&tv));
    let callee = gen.copy_if_needed_to_preserve_evaluation_order(&callee);

    // Unwrap this_value at function scope so its register lifetime matches C++
    // (where original_this_value is a function-scope local that outlives argument temporaries).
    let this_value = this_value.unwrap_or_else(|| gen.add_constant_undefined());

    let has_spread = data.arguments.iter().any(|a| a.is_spread);

    if has_spread {
        // Build an arguments array using NewArray + ArrayAppend for spread elements.
        let args_array = gen.allocate_register();
        let first_spread = data.arguments.iter().position(|a| a.is_spread).unwrap_or(0);

        let mut pre_holders = Vec::new();
        for arg in &data.arguments[..first_spread] {
            let val = generate_expr_or_undefined(&arg.value, gen, None);
            pre_holders.push(gen.copy_if_needed_to_preserve_evaluation_order(&val));
        }
        let pre_args: Vec<Operand> = pre_holders.iter().map(|a| a.operand()).collect();
        gen.emit(Instruction::NewArray {
            dst: args_array.operand(),
            element_count: pre_args.len() as u32,
            elements: pre_args,
        });
        drop(pre_holders);

        for arg in &data.arguments[first_spread..] {
            let val = generate_expr_or_undefined(&arg.value, gen, None);
            gen.emit(Instruction::ArrayAppend {
                dst: args_array.operand(),
                src: val.operand(),
                is_spread: arg.is_spread,
            });
        }

        if is_new {
            gen.emit(Instruction::CallConstructWithArgumentArray {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_value.operand(),
                arguments: args_array.operand(),
                expression_string,
            });
        } else if is_direct_eval {
            gen.emit(Instruction::CallDirectEvalWithArgumentArray {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_value.operand(),
                arguments: args_array.operand(),
                expression_string,
            });
        } else {
            gen.emit(Instruction::CallWithArgumentArray {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_value.operand(),
                arguments: args_array.operand(),
                expression_string,
            });
        }
    } else {
        // Copy local variables into fresh registers so that evaluating
        // later arguments cannot mutate earlier argument values (e.g.
        // `bar(i, i++)` — the first arg must be the pre-increment value).
        let mut arg_holders = Vec::new();
        for arg in &data.arguments {
            let val = generate_expr_or_undefined(&arg.value, gen, None);
            arg_holders.push(gen.copy_if_needed_to_preserve_evaluation_order(&val));
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
            gen.emit(Instruction::CallDirectEval {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_value.operand(),
                argument_count: args.len() as u32,
                expression_string,
                arguments: args,
            });
        } else {
            gen.emit(Instruction::Call {
                dst: dst.operand(),
                callee: callee.operand(),
                this_value: this_value.operand(),
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
            let is_super = matches!(object.inner, Expression::Super);
            let base = generate_expr(object, gen, None)?;
            let base_id = intern_base_identifier(gen, object);
            let value = gen.allocate_register();

            if is_super {
                // For super property references, use WithThis variants
                // with this_value = ResolveThisBinding.
                let this_value = emit_resolve_this_binding(gen);
                let computed_key = emit_super_get(gen, &value, &base, property, *computed, &this_value);
                if prefixed {
                    match op {
                        UpdateOp::Increment => gen.emit(Instruction::Increment { dst: value.operand() }),
                        UpdateOp::Decrement => gen.emit(Instruction::Decrement { dst: value.operand() }),
                    }
                    emit_super_put(gen, &base, property, *computed, &this_value, &value, computed_key.as_ref());
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
                    emit_super_put(gen, &base, property, *computed, &this_value, &value, computed_key.as_ref());
                    Some(dst)
                }
            } else if *computed {
                let prop = generate_expr(property, gen, None)?;
                gen.emit(Instruction::GetByValue {
                    dst: value.operand(),
                    base: base.operand(),
                    property: prop.operand(),
                    base_identifier: base_id,
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
                        base_identifier: base_id,
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
                        base_identifier: base_id,
                    });
                    Some(dst)
                }
            } else if let Expression::Identifier(prop_ident) = &property.inner {
                emit_get_by_id(gen, &value, &base, &prop_ident.name, None);
                let key = gen.intern_property_key(&prop_ident.name);
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
                        base_identifier: base_id,
                    });
                    return Some(dst);
                }
                let cache2 = gen.next_property_lookup_cache();
                gen.emit(Instruction::PutNormalById {
                    base: base.operand(),
                    property: key,
                    src: value.operand(),
                    cache_index: cache2,
                    base_identifier: base_id,
                });
                Some(value)
            } else {
                // Fallback: just evaluate, no store-back
                Some(value)
            }
        }
        _ => {
            // Invalid update target (e.g. foo()++). Per spec, evaluate the
            // expression first, then throw ReferenceError.
            generate_expr(argument, gen, None);
            let exception = gen.allocate_register();
            let error_msg = utf16!("Invalid left-hand side in assignment").to_vec();
            let error_string = gen.intern_string(&error_msg);
            gen.emit(Instruction::NewReferenceError {
                dst: exception.operand(),
                error_string,
            });
            gen.emit(Instruction::Throw { src: exception.operand() });
            let dead_block = gen.make_block();
            gen.switch_to_basic_block(dead_block);
            Some(gen.add_constant_undefined())
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
                    gen.pending_lhs_name = Some(gen.intern_identifier(&ident.name));
                    let rhs_val = generate_expr(rhs, gen, preferred_dst)?;
                    gen.pending_lhs_name = None;
                    emit_set_variable(gen, ident, &rhs_val);
                    return Some(rhs_val);
                }

                // Load LHS value first (needed for both compound and logical assignments).
                let lhs_val = generate_identifier(ident, gen, None)?;

                let is_logical = matches!(op, AssignmentOp::AndAssignment | AssignmentOp::OrAssignment | AssignmentOp::NullishAssignment);
                if is_logical {
                    // Logical assignments short-circuit: evaluate RHS only if condition met.
                    let rhs_block = gen.make_block();
                    let lhs_block = gen.make_block();
                    let end_block = gen.make_block();
                    let dst = choose_dst(gen, preferred_dst);
                    match op {
                        AssignmentOp::AndAssignment => {
                            gen.emit(Instruction::JumpIf {
                                condition: lhs_val.operand(),
                                true_target: Label(rhs_block as u32),
                                false_target: Label(lhs_block as u32),
                            });
                        }
                        AssignmentOp::OrAssignment => {
                            gen.emit(Instruction::JumpIf {
                                condition: lhs_val.operand(),
                                true_target: Label(lhs_block as u32),
                                false_target: Label(rhs_block as u32),
                            });
                        }
                        AssignmentOp::NullishAssignment => {
                            gen.emit(Instruction::JumpNullish {
                                condition: lhs_val.operand(),
                                true_target: Label(rhs_block as u32),
                                false_target: Label(lhs_block as u32),
                            });
                        }
                        _ => unreachable!(),
                    }
                    // RHS block: evaluate RHS, assign, jump to end.
                    gen.switch_to_basic_block(rhs_block);
                    gen.pending_lhs_name = Some(gen.intern_identifier(&ident.name));
                    let rhs_val = generate_expr(rhs, gen, None)?;
                    gen.pending_lhs_name = None;
                    gen.emit_mov(&dst, &rhs_val);
                    emit_set_variable(gen, ident, &dst);
                    gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                    // LHS block: keep original value.
                    gen.switch_to_basic_block(lhs_block);
                    gen.emit_mov(&dst, &lhs_val);
                    gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                    gen.switch_to_basic_block(end_block);
                    return Some(dst);
                }

                // Regular compound assignment (+=, -=, etc.)
                let rhs_val = generate_expr(rhs, gen, None)?;
                let dst = choose_dst(gen, preferred_dst);
                emit_compound_assignment(gen, op, &dst, &lhs_val, &rhs_val);
                emit_set_variable(gen, ident, &dst);
                return Some(dst);
            }
            // Member expression LHS (e.g., obj.foo = x, obj[key] = x)
            if let Expression::Member { object, property, computed } = &lhs_expr.inner {
                let is_super = matches!(object.inner, Expression::Super);
                let base_raw = generate_expr(object, gen, None)?;
                // Copy base into a fresh register so the RHS can't mutate it.
                let base = gen.allocate_register();
                gen.emit_mov(&base, &base_raw);

                // For super property references, resolve this binding once.
                let super_this = if is_super {
                    Some(emit_resolve_this_binding(gen))
                } else {
                    None
                };

                if op == AssignmentOp::Assignment {
                    if is_super {
                        // For computed super, evaluate the key before the RHS.
                        let computed_key = if *computed {
                            let key_val = generate_expr_or_undefined(property, gen, None);
                            let saved = gen.allocate_register();
                            gen.emit_mov(&saved, &key_val);
                            Some(saved)
                        } else {
                            None
                        };
                        let rhs_val = generate_expr(rhs, gen, preferred_dst)?;
                        emit_super_put(gen, &base, property, *computed, super_this.as_ref().unwrap(), &rhs_val, computed_key.as_ref());
                        return Some(rhs_val);
                    }
                    // For computed properties, evaluate the key BEFORE the RHS
                    // (spec: LHS is fully evaluated before RHS).
                    let precomputed_key = if *computed {
                        let key_val = generate_expr_or_undefined(property, gen, None);
                        // Copy into a fresh register so the RHS can't mutate it.
                        let saved = gen.allocate_register();
                        gen.emit_mov(&saved, &key_val);
                        Some(saved)
                    } else {
                        None
                    };
                    let rhs_val = generate_expr(rhs, gen, preferred_dst)?;
                    if let Some(key) = precomputed_key {
                        let base_id = intern_base_identifier(gen, object);
                        gen.emit(Instruction::PutNormalByValue {
                            base: base.operand(),
                            property: key.operand(),
                            src: rhs_val.operand(),
                            base_identifier: base_id,
                        });
                    } else {
                        emit_put_to_member(gen, &base, property, false, &rhs_val, Some(object));
                    }
                    return Some(rhs_val);
                }

                // Compound member assignment: load old value, then apply op.
                let old_val = gen.allocate_register();
                let base_id = intern_base_identifier(gen, object);
                let is_logical = matches!(op, AssignmentOp::AndAssignment | AssignmentOp::OrAssignment | AssignmentOp::NullishAssignment);

                if is_super {
                    let computed_key = emit_super_get(gen, &old_val, &base, property, *computed, super_this.as_ref().unwrap());
                    if is_logical {
                        let rhs_block = gen.make_block();
                        let lhs_block = gen.make_block();
                        let end_block = gen.make_block();
                        let dst = choose_dst(gen, preferred_dst);
                        emit_logical_jump(gen, op, &old_val, rhs_block, lhs_block);
                        gen.switch_to_basic_block(rhs_block);
                        let rhs_val = generate_expr(rhs, gen, None)?;
                        gen.emit_mov(&dst, &rhs_val);
                        emit_super_put(gen, &base, property, *computed, super_this.as_ref().unwrap(), &dst, computed_key.as_ref());
                        gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                        gen.switch_to_basic_block(lhs_block);
                        gen.emit_mov(&dst, &old_val);
                        gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                        gen.switch_to_basic_block(end_block);
                        return Some(dst);
                    }
                    let rhs_val = generate_expr(rhs, gen, None)?;
                    let dst = choose_dst(gen, preferred_dst);
                    emit_compound_assignment(gen, op, &dst, &old_val, &rhs_val);
                    emit_super_put(gen, &base, property, *computed, super_this.as_ref().unwrap(), &dst, computed_key.as_ref());
                    return Some(dst);
                }

                if *computed {
                    let prop_raw = generate_expr(property, gen, None)?;
                    // Copy key into a fresh register so the RHS can't mutate it.
                    let prop = gen.allocate_register();
                    gen.emit_mov(&prop, &prop_raw);
                    gen.emit(Instruction::GetByValue {
                        dst: old_val.operand(),
                        base: base.operand(),
                        property: prop.operand(),
                        base_identifier: base_id,
                    });
                    if is_logical {
                        let rhs_block = gen.make_block();
                        let lhs_block = gen.make_block();
                        let end_block = gen.make_block();
                        let dst = choose_dst(gen, preferred_dst);
                        emit_logical_jump(gen, op, &old_val, rhs_block, lhs_block);
                        gen.switch_to_basic_block(rhs_block);
                        let rhs_val = generate_expr(rhs, gen, None)?;
                        gen.emit_mov(&dst, &rhs_val);
                        gen.emit(Instruction::PutNormalByValue {
                            base: base.operand(),
                            property: prop.operand(),
                            src: dst.operand(),
                            base_identifier: base_id,
                        });
                        gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                        gen.switch_to_basic_block(lhs_block);
                        gen.emit_mov(&dst, &old_val);
                        gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                        gen.switch_to_basic_block(end_block);
                        return Some(dst);
                    }
                    let rhs_val = generate_expr(rhs, gen, None)?;
                    let dst = choose_dst(gen, preferred_dst);
                    emit_compound_assignment(gen, op, &dst, &old_val, &rhs_val);
                    gen.emit(Instruction::PutNormalByValue {
                        base: base.operand(),
                        property: prop.operand(),
                        src: dst.operand(),
                        base_identifier: base_id,
                    });
                    return Some(dst);
                } else {
                    if let Expression::Identifier(ident) = &property.inner {
                        emit_get_by_id(gen, &old_val, &base, &ident.name, None);
                        if is_logical {
                            let rhs_block = gen.make_block();
                            let lhs_block = gen.make_block();
                            let end_block = gen.make_block();
                            let dst = choose_dst(gen, preferred_dst);
                            emit_logical_jump(gen, op, &old_val, rhs_block, lhs_block);
                            gen.switch_to_basic_block(rhs_block);
                            let rhs_val = generate_expr(rhs, gen, None)?;
                            gen.emit_mov(&dst, &rhs_val);
                            let key = gen.intern_property_key(&ident.name);
                            let cache2 = gen.next_property_lookup_cache();
                            gen.emit(Instruction::PutNormalById {
                                base: base.operand(),
                                property: key,
                                src: dst.operand(),
                                cache_index: cache2,
                                base_identifier: base_id,
                            });
                            gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                            gen.switch_to_basic_block(lhs_block);
                            gen.emit_mov(&dst, &old_val);
                            gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                            gen.switch_to_basic_block(end_block);
                            return Some(dst);
                        }
                        let rhs_val = generate_expr(rhs, gen, None)?;
                        let dst = choose_dst(gen, preferred_dst);
                        emit_compound_assignment(gen, op, &dst, &old_val, &rhs_val);
                        let key = gen.intern_property_key(&ident.name);
                        let cache2 = gen.next_property_lookup_cache();
                        gen.emit(Instruction::PutNormalById {
                            base: base.operand(),
                            property: key,
                            src: dst.operand(),
                            cache_index: cache2,
                            base_identifier: base_id,
                        });
                        return Some(dst);
                    } else if let Expression::PrivateIdentifier(priv_ident) = &property.inner {
                        let id = gen.intern_identifier(&priv_ident.name);
                        gen.emit(Instruction::GetPrivateById {
                            dst: old_val.operand(),
                            base: base.operand(),
                            property: id,
                        });
                        if is_logical {
                            let rhs_block = gen.make_block();
                            let lhs_block = gen.make_block();
                            let end_block = gen.make_block();
                            let dst = choose_dst(gen, preferred_dst);
                            emit_logical_jump(gen, op, &old_val, rhs_block, lhs_block);
                            gen.switch_to_basic_block(rhs_block);
                            let rhs_val = generate_expr(rhs, gen, None)?;
                            gen.emit_mov(&dst, &rhs_val);
                            let id2 = gen.intern_identifier(&priv_ident.name);
                            gen.emit(Instruction::PutPrivateById {
                                base: base.operand(),
                                property: id2,
                                src: dst.operand(),
                            });
                            gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                            gen.switch_to_basic_block(lhs_block);
                            gen.emit_mov(&dst, &old_val);
                            gen.emit(Instruction::Jump { target: Label(end_block as u32) });
                            gen.switch_to_basic_block(end_block);
                            return Some(dst);
                        }
                        let rhs_val = generate_expr(rhs, gen, None)?;
                        let dst = choose_dst(gen, preferred_dst);
                        emit_compound_assignment(gen, op, &dst, &old_val, &rhs_val);
                        let id2 = gen.intern_identifier(&priv_ident.name);
                        gen.emit(Instruction::PutPrivateById {
                            base: base.operand(),
                            property: id2,
                            src: dst.operand(),
                        });
                        return Some(dst);
                    }
                }
            }
            // LHS is not an identifier or member expression (e.g. a function call).
            // Per spec 13.15.2 step 1b, evaluate the LHS, then throw ReferenceError
            // before evaluating the RHS.
            generate_expr(lhs_expr, gen, None);
            let exception = gen.allocate_register();
            let error_msg = utf16!("Invalid left-hand side in assignment").to_vec();
            let error_string = gen.intern_string(&error_msg);
            gen.emit(Instruction::NewReferenceError {
                dst: exception.operand(),
                error_string,
            });
            gen.emit(Instruction::Throw { src: exception.operand() });
            let dead_block = gen.make_block();
            gen.switch_to_basic_block(dead_block);
            Some(gen.add_constant_undefined())
        }
        AssignmentLhs::Pattern(pattern) => {
            let rhs_val = generate_expr(rhs, gen, preferred_dst)?;
            generate_binding_pattern_bytecode(gen, pattern, BindingMode::Set, &rhs_val);
            Some(rhs_val)
        }
    }
}

/// Emit ResolveThisBinding and return the this value register.
fn emit_resolve_this_binding(gen: &mut Generator) -> ScopedOperand {
    gen.emit(Instruction::ResolveThisBinding);
    gen.this_value()
}

/// Emit a super property get (uses WithThis variants).
/// For computed access, evaluates the property expression.
/// Returns the evaluated property operand for computed access (so callers
/// can reuse it for a subsequent put).
fn emit_super_get(
    gen: &mut Generator,
    dst: &ScopedOperand,
    base: &ScopedOperand,
    property: &Expr,
    computed: bool,
    this_value: &ScopedOperand,
) -> Option<ScopedOperand> {
    if computed {
        let prop = generate_expr_or_undefined(property, gen, None);
        gen.emit(Instruction::GetByValueWithThis {
            dst: dst.operand(),
            base: base.operand(),
            property: prop.operand(),
            this_value: this_value.operand(),
        });
        Some(prop)
    } else if let Expression::Identifier(ident) = &property.inner {
        let key = gen.intern_property_key(&ident.name);
        let cache = gen.next_property_lookup_cache();
        gen.emit(Instruction::GetByIdWithThis {
            dst: dst.operand(),
            base: base.operand(),
            property: key,
            this_value: this_value.operand(),
            cache_index: cache,
        });
        None
    } else {
        None
    }
}

/// Emit a super property put (uses WithThis variants).
/// For computed access, `computed_key` should be the operand returned by
/// `emit_super_get` so the property is not re-evaluated. If `None` for
/// computed access, the property expression will be evaluated.
fn emit_super_put(
    gen: &mut Generator,
    base: &ScopedOperand,
    property: &Expr,
    computed: bool,
    this_value: &ScopedOperand,
    value: &ScopedOperand,
    computed_key: Option<&ScopedOperand>,
) {
    if computed {
        let prop = match computed_key {
            Some(k) => k.clone(),
            None => generate_expr_or_undefined(property, gen, None),
        };
        gen.emit(Instruction::PutNormalByValueWithThis {
            base: base.operand(),
            property: prop.operand(),
            this_value: this_value.operand(),
            src: value.operand(),
        });
    } else if let Expression::Identifier(ident) = &property.inner {
        let key = gen.intern_property_key(&ident.name);
        let cache = gen.next_property_lookup_cache();
        gen.emit(Instruction::PutNormalByIdWithThis {
            base: base.operand(),
            this_value: this_value.operand(),
            property: key,
            src: value.operand(),
            cache_index: cache,
        });
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
    let key = gen.intern_property_key(property_name);
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
        // TDZ check: throw ReferenceError if assigning to an uninitialized let binding.
        if !gen.is_local_initialized(local_index)
            && ident.declaration_kind.get() != IdentDeclarationKind::Var
        {
            gen.emit(Instruction::ThrowIfTDZ {
                src: local.operand(),
            });
        }
        gen.emit_mov(&local, value);
    } else if ident.is_global.get() {
        let id = gen.intern_identifier(&ident.name);
        let cache = gen.next_global_variable_cache();
        gen.emit(Instruction::SetGlobal {
            identifier: id,
            src: value.operand(),
            cache_index: cache,
        });
    } else {
        // Non-local, non-global: use SetLexicalBinding which searches
        // the lexical environment chain (important for with-statement support).
        let id = gen.intern_identifier(&ident.name);
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
    base_object: Option<&Expr>,
) {
    let base_id = base_object.and_then(|obj| intern_base_identifier(gen, obj));
    if computed {
        let prop = generate_expr_or_undefined(property, gen, None);
        gen.emit(Instruction::PutNormalByValue {
            base: base.operand(),
            property: prop.operand(),
            src: value.operand(),
            base_identifier: base_id,
        });
    } else if let Expression::Identifier(ident) = &property.inner {
        let key = gen.intern_property_key(&ident.name);
        let cache = gen.next_property_lookup_cache();
        gen.emit(Instruction::PutNormalById {
            base: base.operand(),
            property: key,
            src: value.operand(),
            cache_index: cache,
            base_identifier: base_id,
        });
    } else if let Expression::PrivateIdentifier(priv_ident) = &property.inner {
        let id = gen.intern_identifier(&priv_ident.name);
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
            let id = gen.intern_identifier(&ident.name);
            gen.emit(Instruction::DeleteVariable {
                dst: dst.operand(),
                identifier: id,
            });
            dst
        }
        Expression::Member { object, property, computed } => {
            let base = generate_expr_or_undefined(object, gen, None);
            let dst = choose_dst(gen, preferred_dst);
            if *computed {
                let key = generate_expr_or_undefined(property, gen, None);
                gen.emit(Instruction::DeleteByValue {
                    dst: dst.operand(),
                    base: base.operand(),
                    property: key.operand(),
                });
            } else if let Expression::Identifier(prop_ident) = &property.inner {
                let key = gen.intern_property_key(&prop_ident.name);
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


/// Pre-evaluated reference operands for deferred store.
/// Used when the spec requires evaluating the assignment target reference
/// before performing some other operation (like iterating a spread element).
enum EvaluatedReference {
    Member {
        base: ScopedOperand,
        property: ScopedOperand,
        base_identifier: Option<IdentifierTableIndex>,
    },
    MemberId {
        base: ScopedOperand,
        property: PropertyKeyTableIndex,
        cache: u32,
        base_identifier: Option<IdentifierTableIndex>,
    },
    PrivateMember {
        base: ScopedOperand,
        property: IdentifierTableIndex,
    },
    SuperMember {
        base: ScopedOperand,
        property: ScopedOperand,
        this_value: ScopedOperand,
    },
    SuperMemberId {
        base: ScopedOperand,
        property: PropertyKeyTableIndex,
        cache: u32,
        this_value: ScopedOperand,
    },
}

/// Evaluate a member expression target to get pre-computed reference operands
/// without performing a load. This implements the "Let lref be ? Evaluation of
/// DestructuringAssignmentTarget" step from the spec.
fn emit_evaluate_member_reference(gen: &mut Generator, target: &Expr) -> EvaluatedReference {
    if let Expression::Member { object, property, computed } = &target.inner {
        let is_super = matches!(object.inner, Expression::Super);
        let base = generate_expr_or_undefined(object, gen, None);

        if is_super {
            let this_value = emit_resolve_this_binding(gen);
            if *computed {
                let prop = generate_expr_or_undefined(property, gen, None);
                let saved_prop = gen.allocate_register();
                gen.emit_mov(&saved_prop, &prop);
                EvaluatedReference::SuperMember { base, property: saved_prop, this_value }
            } else if let Expression::Identifier(ident) = &property.inner {
                let key = gen.intern_property_key(&ident.name);
                let cache = gen.next_property_lookup_cache();
                EvaluatedReference::SuperMemberId { base, property: key, cache, this_value }
            } else {
                unreachable!()
            }
        } else {
            let base_id = intern_base_identifier(gen, object);
            if *computed {
                let prop = generate_expr_or_undefined(property, gen, None);
                let saved_prop = gen.allocate_register();
                gen.emit_mov(&saved_prop, &prop);
                EvaluatedReference::Member { base, property: saved_prop, base_identifier: base_id }
            } else if let Expression::Identifier(ident) = &property.inner {
                let key = gen.intern_property_key(&ident.name);
                let cache = gen.next_property_lookup_cache();
                EvaluatedReference::MemberId { base, property: key, cache, base_identifier: base_id }
            } else if let Expression::PrivateIdentifier(priv_ident) = &property.inner {
                let id = gen.intern_identifier(&priv_ident.name);
                EvaluatedReference::PrivateMember { base, property: id }
            } else {
                unreachable!()
            }
        }
    } else {
        unreachable!("emit_evaluate_member_reference called on non-member expression")
    }
}

/// Store a value to a pre-evaluated reference.
fn emit_store_to_evaluated_reference(gen: &mut Generator, reference: &EvaluatedReference, value: &ScopedOperand) {
    match reference {
        EvaluatedReference::Member { base, property, base_identifier } => {
            gen.emit(Instruction::PutNormalByValue {
                base: base.operand(),
                property: property.operand(),
                src: value.operand(),
                base_identifier: *base_identifier,
            });
        }
        EvaluatedReference::MemberId { base, property, cache, base_identifier } => {
            gen.emit(Instruction::PutNormalById {
                base: base.operand(),
                property: *property,
                src: value.operand(),
                cache_index: *cache,
                base_identifier: *base_identifier,
            });
        }
        EvaluatedReference::PrivateMember { base, property } => {
            gen.emit(Instruction::PutPrivateById {
                base: base.operand(),
                property: *property,
                src: value.operand(),
            });
        }
        EvaluatedReference::SuperMember { base, property, this_value } => {
            gen.emit(Instruction::PutNormalByValueWithThis {
                base: base.operand(),
                property: property.operand(),
                this_value: this_value.operand(),
                src: value.operand(),
            });
        }
        EvaluatedReference::SuperMemberId { base, property, cache, this_value } => {
            gen.emit(Instruction::PutNormalByIdWithThis {
                base: base.operand(),
                this_value: this_value.operand(),
                property: *property,
                src: value.operand(),
                cache_index: *cache,
            });
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
            let is_super = matches!(object.inner, Expression::Super);
            let base = generate_expr_or_undefined(object, gen, None);
            if is_super {
                let this_value = emit_resolve_this_binding(gen);
                emit_super_put(gen, &base, property, *computed, &this_value, value, None);
            } else {
                emit_put_to_member(gen, &base, property, *computed, value, Some(object));
            }
        }
        _ => {
            // Evaluate the expression for side effects, then throw ReferenceError.
            generate_expr(target, gen, None);
            let exception = gen.allocate_register();
            let error_msg = utf16!("Invalid left-hand side in assignment").to_vec();
            let error_string = gen.intern_string(&error_msg);
            gen.emit(Instruction::NewReferenceError {
                dst: exception.operand(),
                error_string,
            });
            gen.emit(Instruction::Throw { src: exception.operand() });
            let dead_block = gen.make_block();
            gen.switch_to_basic_block(dead_block);
        }
    }
}

/// Emit the conditional jump for a logical assignment (&&=, ||=, ??=).
fn emit_logical_jump(gen: &mut Generator, op: AssignmentOp, condition: &ScopedOperand, rhs_block: usize, lhs_block: usize) {
    match op {
        AssignmentOp::AndAssignment => {
            gen.emit(Instruction::JumpIf {
                condition: condition.operand(),
                true_target: Label(rhs_block as u32),
                false_target: Label(lhs_block as u32),
            });
        }
        AssignmentOp::OrAssignment => {
            gen.emit(Instruction::JumpIf {
                condition: condition.operand(),
                true_target: Label(lhs_block as u32),
                false_target: Label(rhs_block as u32),
            });
        }
        AssignmentOp::NullishAssignment => {
            gen.emit(Instruction::JumpNullish {
                condition: condition.operand(),
                true_target: Label(rhs_block as u32),
                false_target: Label(lhs_block as u32),
            });
        }
        _ => unreachable!(),
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

    // OPTIMIZATION: Filter out empty string segments (matching C++ behavior).
    let segments: Vec<&Expr> = data.expressions.iter().filter(|e| {
        !matches!(&e.inner, Expression::StringLiteral(s) if s.is_empty())
    }).collect();

    if segments.is_empty() {
        return Some(gen.add_constant_string(Vec::new()));
    }

    if segments.len() == 1 {
        let val = generate_expr(segments[0], gen, None)?;
        // If it's a constant, return directly.
        if val.operand().is_constant() {
            return Some(val);
        }
        // Otherwise, emit ToString.
        let dst = choose_dst(gen, preferred_dst);
        gen.emit(Instruction::ToString {
            dst: dst.operand(),
            value: val.operand(),
        });
        return Some(dst);
    }

    let dst = choose_dst(gen, preferred_dst);
    let mut first = true;
    for expr in &segments {
        let val = generate_expr_or_undefined(expr, gen, None);
        if first {
            if matches!(&expr.inner, Expression::StringLiteral(_)) {
                gen.emit_mov(&dst, &val);
            } else {
                gen.emit(Instruction::ToString {
                    dst: dst.operand(),
                    value: val.operand(),
                });
            }
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
            let obj = generate_expr_or_undefined(object, gen, None);
            let method = gen.allocate_register();
            if *computed {
                let prop = generate_expr_or_undefined(property, gen, None);
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
            let tag_val = generate_expr_or_undefined(tag, gen, None);
            (tag_val, None)
        }
        Expression::Identifier(ident) => {
            // Non-local, non-global identifier: use GetCalleeAndThisFromEnvironment
            // to properly handle with-statement bindings.
            let callee_reg = gen.allocate_register();
            let this_reg = gen.allocate_register();
            let id = gen.intern_identifier(&ident.name);
            gen.emit(Instruction::GetCalleeAndThisFromEnvironment {
                callee: callee_reg.operand(),
                this_value: this_reg.operand(),
                identifier: id,
                cache: EnvironmentCoordinate::empty(),
            });
            (callee_reg, Some(this_reg))
        }
        _ => {
            let tag_val = generate_expr_or_undefined(tag, gen, None);
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
            let val = generate_expr_or_undefined(&data.expressions[i], gen, None);
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
        let val = generate_expr_or_undefined(&data.expressions[i], gen, None);
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
    let labels = std::mem::take(&mut gen.pending_labels);

    let completion = if gen.must_propagate_completion {
        let reg = gen.allocate_register();
        let undef = gen.add_constant_undefined();
        gen.emit_mov(&reg, &undef);
        Some(reg)
    } else {
        None
    };

    gen.begin_breakable_scope(Label(end_block as u32), labels, completion.clone());

    // Block declaration instantiation: create lexical environment for
    // function declarations and let/const across all switch cases.
    let did_create_env = emit_switch_block_declaration_instantiation(gen, data);

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

        let saved_completion = gen.current_completion_register.clone();
        if let Some(ref c) = completion {
            gen.current_completion_register = Some(c.clone());
        }

        let case_scope = case.scope.borrow();
        for child in &case_scope.children {
            // For function declarations in switch cases: emit AnnexB hoisting
            // only if the scope collector approved it (name is in annexb_function_names).
            if did_create_env {
                if let Statement::FunctionDeclaration(func_data) = &child.inner {
                    if let Some(ref name_ident) = func_data.name {
                        if gen.annexb_function_names.contains(&name_ident.name) {
                            let id = gen.intern_identifier(&name_ident.name);
                            let value = gen.allocate_register();
                            gen.emit(Instruction::GetBinding {
                                dst: value.operand(),
                                identifier: id,
                                cache: EnvironmentCoordinate::empty(),
                            });
                            gen.emit(Instruction::SetVariableBinding {
                                identifier: id,
                                src: value.operand(),
                                cache: EnvironmentCoordinate::empty(),
                            });
                        }
                    }
                }
            }
            let result = generate_stmt(child, gen, None);
            if gen.is_current_block_terminated() {
                break;
            }
            if gen.must_propagate_completion {
                if let Some(ref c) = completion {
                    if let Some(ref val) = result {
                        gen.emit_mov(c, val);
                    } else {
                        let undef = gen.add_constant_undefined();
                        gen.emit_mov(c, &undef);
                    }
                }
            }
        }

        gen.current_completion_register = saved_completion;

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

    if did_create_env {
        gen.lexical_environment_register_stack.pop();
        if !gen.is_current_block_terminated() {
            let parent = gen.current_lexical_environment();
            gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
        }
    }
    completion
}

/// Create block declaration instantiation for switch statements.
/// Function declarations and let/const declarations across all cases
/// share a single lexical environment.
fn emit_switch_block_declaration_instantiation(
    gen: &mut Generator,
    data: &SwitchStatementData,
) -> bool {
    // Collect all statements across all cases.
    let case_scopes: Vec<_> = data.cases.iter().map(|c| c.scope.borrow()).collect();
    let all_children: Vec<&Stmt> = case_scopes.iter()
        .flat_map(|scope| scope.children.iter())
        .collect();

    // Check if we need a lexical environment.
    let needs_env = all_children.iter().any(|child| match &child.inner {
        Statement::FunctionDeclaration(_) => true,
        Statement::VariableDeclaration { kind, .. } => {
            *kind == DeclarationKind::Let || *kind == DeclarationKind::Const
        }
        Statement::ClassDeclaration(class_data) => {
            class_data.name.as_ref().is_some_and(|n| !n.is_local())
        }
        _ => false,
    });

    if !needs_env {
        return false;
    }

    let parent = gen.current_lexical_environment();
    let new_env = gen.allocate_register();
    gen.emit(Instruction::CreateLexicalEnvironment {
        dst: new_env.operand(),
        parent: parent.operand(),
        capacity: 0,
    });
    gen.lexical_environment_register_stack.push(new_env);

    create_lexical_bindings_for_block(gen, all_children.iter().copied());

    // Pass 2: Instantiate function declarations (last one wins for duplicates).
    let mut last_func_indices: Vec<(Vec<u16>, usize)> = Vec::new();
    for (i, child) in all_children.iter().enumerate().rev() {
        if let Statement::FunctionDeclaration(func_data) = &child.inner {
            if let Some(ref name_ident) = func_data.name {
                if !last_func_indices.iter().any(|(n, _)| *n == name_ident.name) {
                    last_func_indices.push((name_ident.name.clone(), i));
                }
            }
        }
    }
    for (i, child) in all_children.iter().enumerate() {
        if let Statement::FunctionDeclaration(func_data) = &child.inner {
            if let Some(ref name_ident) = func_data.name {
                if last_func_indices.iter().any(|(n, idx)| *n == name_ident.name && *idx == i) {
                    let sfd_index = emit_new_function(gen, func_data, None);
                    let fo = gen.allocate_register();
                    gen.emit(Instruction::NewFunction {
                        dst: fo.operand(),
                        shared_function_data_index: sfd_index,
                        home_object: None,
                        lhs_name: None,
                    });
                    if name_ident.is_local() {
                        let local = gen.local(name_ident.local_index.get());
                        gen.emit_mov(&local, &fo);
                    } else {
                        let id = gen.intern_identifier(&name_ident.name);
                        gen.emit(Instruction::InitializeLexicalBinding {
                            identifier: id,
                            src: fo.operand(),
                            cache: EnvironmentCoordinate::empty(),
                        });
                    }
                }
            }
        }
    }

    true
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
        if prop.property_type == ObjectPropertyType::Spread {
            // For spread, the source expression is in `key`, not `value`.
            let src = generate_expr_or_undefined(&prop.key, gen, None);
            gen.emit(Instruction::PutBySpread {
                base: dst.operand(),
                src: src.operand(),
            });
            continue;
        }

        // For computed keys, evaluate key before value (spec evaluation order).
        // ComputedPropertyName calls ToPropertyKey, which includes ToPrimitive(hint: string).
        // The ToPrimitive is the only user-observable step; after this, the ToPrimitive
        // inside PutOwnByValue's to_property_key is a no-op.
        let computed_key = if prop.is_computed {
            let key = generate_expr_or_undefined(&prop.key, gen, None);
            gen.emit(Instruction::ToPrimitiveWithStringHint {
                dst: key.operand(),
                value: key.operand(),
            });
            Some(key)
        } else {
            None
        };

        // Set pending LHS name for function name inference on non-computed properties.
        // ProtoSetter (__proto__) skips NamedEvaluation per spec.
        if !prop.is_computed && prop.property_type != ObjectPropertyType::ProtoSetter {
            let base_name = match &prop.key.inner {
                Expression::StringLiteral(s) => Some(s.clone()),
                Expression::Identifier(ident) => Some(ident.name.clone()),
                Expression::NumericLiteral(n) => Some(number_to_utf16(*n)),
                _ => None,
            };
            if let Some(name) = base_name {
                let full_name = match prop.property_type {
                    ObjectPropertyType::Getter => {
                        let mut prefixed: Vec<u16> = utf16!("get ").to_vec();
                        prefixed.extend_from_slice(&name);
                        prefixed
                    }
                    ObjectPropertyType::Setter => {
                        let mut prefixed: Vec<u16> = utf16!("set ").to_vec();
                        prefixed.extend_from_slice(&name);
                        prefixed
                    }
                    _ => name,
                };
                gen.pending_lhs_name = Some(gen.intern_identifier(&full_name));
            } else {
                gen.pending_lhs_name = None;
            }
        } else {
            gen.pending_lhs_name = None;
        }
        // Methods, getters, and setters need the object as their [[HomeObject]]
        // so that super property lookups work.
        let is_method_like = prop.is_method
            || prop.property_type == ObjectPropertyType::Getter
            || prop.property_type == ObjectPropertyType::Setter;
        if is_method_like {
            gen.home_objects.push(dst.clone());
        }
        let value = prop.value.as_ref().and_then(|v| generate_expr(v, gen, None))
            .unwrap_or_else(|| gen.add_constant_undefined());
        if is_method_like {
            gen.home_objects.pop();
        }
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
                let key = gen.intern_property_key(utf16!("__proto__"));
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
        let key_val = generate_expr_or_undefined(key, gen, None);
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
            let prop_key = gen.intern_property_key(&ident.name);
            gen.emit(Instruction::InitObjectLiteralProperty {
                object: object.operand(),
                property: prop_key,
                src: value.operand(),
                shape_cache_index: cache_index,
                property_slot: slot,
            });
        }
        Expression::StringLiteral(s) => {
            let prop_key = gen.intern_property_key(&s);
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
            let key_val = generate_expr_or_undefined(key, gen, None);
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
        let prop_key = gen.intern_property_key(name);
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
        let key_val = generate_expr_or_undefined(key, gen, None);
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
) -> Option<(ScopedOperand, ScopedOperand)> {
    let end_block = gen.make_block();
    let dst = choose_dst(gen, preferred_dst);
    let undef = gen.add_constant_undefined();
    let this_value = gen.allocate_register();
    gen.emit_mov(&this_value, &undef);

    // When the base is a MemberExpression, we need to separately track the
    // object (for use as `this` in calls) and the property value.
    // When the base is a nested OptionalChain, recursively generate it to
    // propagate its this_value.
    let mut current = match &base.inner {
        Expression::Member { object, property, computed } => {
            let obj = generate_expr(object, gen, None)?;
            gen.emit_mov(&this_value, &obj);
            let val = gen.allocate_register();
            if *computed {
                let prop = generate_expr(property, gen, None)?;
                gen.emit(Instruction::GetByValue {
                    dst: val.operand(),
                    base: obj.operand(),
                    property: prop.operand(),
                    base_identifier: None,
                });
            } else if let Expression::Identifier(ident) = &property.inner {
                emit_get_by_id(gen, &val, &obj, &ident.name, None);
            } else if let Expression::PrivateIdentifier(name) = &property.inner {
                let id = gen.intern_identifier(&name.name);
                gen.emit(Instruction::GetPrivateById {
                    dst: val.operand(),
                    base: obj.operand(),
                    property: id,
                });
            } else {
                let prop = generate_expr(property, gen, None)?;
                gen.emit(Instruction::GetByValue {
                    dst: val.operand(),
                    base: obj.operand(),
                    property: prop.operand(),
                    base_identifier: None,
                });
            }
            val
        }
        Expression::OptionalChain { base: inner_base, references: inner_refs } => {
            let (val, inner_this) = generate_optional_chain(gen, inner_base, inner_refs, None)?;
            gen.emit_mov(&this_value, &inner_this);
            val
        }
        _ => generate_expr(base, gen, None)?,
    };

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
                gen.emit_mov(&this_value, &current);
                let next = gen.allocate_register();
                emit_get_by_id(gen, &next, &current, &identifier.name, None);
                current = next;
            }
            OptionalChainReference::ComputedReference { expression, .. } => {
                gen.emit_mov(&this_value, &current);
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
                let has_spread = arguments.iter().any(|a| a.is_spread);
                if has_spread {
                    let args_array = gen.allocate_register();
                    let first_spread = arguments.iter().position(|a| a.is_spread).unwrap_or(0);
                    let mut pre_holders = Vec::new();
                    for arg in &arguments[..first_spread] {
                        let val = generate_expr_or_undefined(&arg.value, gen, None);
                        pre_holders.push(val);
                    }
                    let pre_args: Vec<Operand> = pre_holders.iter().map(|a| a.operand()).collect();
                    gen.emit(Instruction::NewArray {
                        dst: args_array.operand(),
                        element_count: pre_args.len() as u32,
                        elements: pre_args,
                    });
                    drop(pre_holders);
                    for arg in &arguments[first_spread..] {
                        let val = generate_expr_or_undefined(&arg.value, gen, None);
                        gen.emit(Instruction::ArrayAppend {
                            dst: args_array.operand(),
                            src: val.operand(),
                            is_spread: arg.is_spread,
                        });
                    }
                    gen.emit(Instruction::CallWithArgumentArray {
                        dst: next.operand(),
                        callee: current.operand(),
                        this_value: this_value.operand(),
                        arguments: args_array.operand(),
                        expression_string: None,
                    });
                } else {
                    let mut arg_holders = Vec::new();
                    for arg in arguments {
                        let val = generate_expr_or_undefined(&arg.value, gen, None);
                        arg_holders.push(val);
                    }
                    let arg_ops: Vec<Operand> = arg_holders.iter().map(|a| a.operand()).collect();
                    gen.emit(Instruction::Call {
                        dst: next.operand(),
                        callee: current.operand(),
                        this_value: this_value.operand(),
                        argument_count: arg_ops.len() as u32,
                        expression_string: None,
                        arguments: arg_ops,
                    });
                }
                gen.emit_mov(&this_value, &undef);
                current = next;
            }
            OptionalChainReference::PrivateMemberReference { private_identifier, .. } => {
                gen.emit_mov(&this_value, &current);
                let next = gen.allocate_register();
                let id = gen.intern_identifier(&private_identifier.name);
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
    Some((dst, this_value))
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
    let lhs_name = if data.name.is_none() { gen.pending_lhs_name.take() } else { None };

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
    {
        let name = if let Some(name_ident) = &data.name {
            name_ident.name.clone()
        } else {
            Vec::new()
        };
        let name_id = gen.intern_identifier(&name);
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
            let name_id = gen.intern_identifier(&name);
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

                // Create SFD for the method function.
                // Don't set the method name here — the runtime's update_function_name
                // in construct_class sets it from the evaluated property key, which
                // correctly handles computed keys (Symbols, etc).
                let sfd_index = if let Expression::Function(func_data) = &function.inner {
                    emit_new_function(
                        gen,
                        func_data,
                        None,
                    ) as i32
                } else {
                    -1i32
                };

                // Handle computed vs static keys
                let is_private = is_private_key(key);

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
                        Expression::PrivateIdentifier(p) => p.name.clone(),
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
                        Statement::Block(ScopeData::shared_with_children(vec![body_stmt])),
                    );

                    // Class bodies are always strict mode.
                    let func_data = FunctionData {
                        name: None,
                        source_text_start: init_expr.range.start.offset,
                        source_text_end: init_expr.range.end.offset,
                        body: Box::new(wrapper_body),
                        parameters: Vec::new(),
                        function_length: 0,
                        kind: FunctionKind::Normal,
                        is_strict_mode: true,
                        is_arrow_function: false,
                        parsing_insights: FunctionParsingInsights {
                            uses_this: true,
                            uses_this_from_environment: true,
                            ..Default::default()
                        },
                        is_hoisted: false,
                    };
                    let idx = emit_new_function(gen, &func_data, Some(utf16!("field")));

                    // Set class_field_initializer_name on the SFD so that
                    // eval("arguments") inside field initializers correctly
                    // throws a SyntaxError.
                    let sfd_ptr = gen.shared_function_data[idx as usize];
                    let key_is_private = is_private_key(key);
                    let key_name: Vec<u16> = match &key.inner {
                        Expression::PrivateIdentifier(ident) => ident.name.clone(),
                        Expression::Identifier(ident) => ident.name.clone(),
                        Expression::StringLiteral(s) => s.clone(),
                        Expression::NumericLiteral(n) => {
                            let s = format!("{}", n);
                            s.encode_utf16().collect()
                        }
                        _ => Vec::new(),
                    };
                    if !key_name.is_empty() {
                        unsafe {
                            super::ffi::rust_sfd_set_class_field_initializer_name(
                                sfd_ptr,
                                key_name.as_ptr(),
                                key_name.len(),
                                key_is_private,
                            );
                        }
                    }

                    idx as i32
                } else {
                    -1i32
                };

                let is_private = is_private_key(key);

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
                // Class bodies are always strict mode.
                let func_data = FunctionData {
                    name: None,
                    source_text_start: body.range.start.offset,
                    source_text_end: body.range.end.offset,
                    body: body.clone(),
                    parameters: Vec::new(),
                    function_length: 0,
                    kind: FunctionKind::Normal,
                    is_strict_mode: true,
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
    let source_text_len = source_end - source_start;

    // Create the ClassBlueprint via FFI
    let bp_ptr = unsafe {
        super::ffi::rust_create_class_blueprint(
            gen.source_code_ptr,
            name_ptr,
            name_len,
            source_start,
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
        lhs_name,
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
fn is_private_key(key: &Expr) -> bool {
    matches!(&key.inner, Expression::PrivateIdentifier(_))
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
    let parent = gen.current_lexical_environment();

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
        let id = gen.intern_identifier(&name);
        gen.emit(Instruction::CreateVariable {
            identifier: id,
            mode: EnvironmentMode::Lexical as u32,
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

/// Create a TDZ environment for lexical declarations in for-in/for-of heads.
/// Returns true if a TDZ scope was entered (must call leave_for_in_of_head_tdz after RHS eval).
fn enter_for_in_of_head_tdz(gen: &mut Generator, lhs: &ForInOfLhs) -> bool {
    if let ForInOfLhs::Declaration(stmt) = lhs {
        if let Statement::VariableDeclaration { kind, declarations } = &stmt.inner {
            if *kind == DeclarationKind::Let || *kind == DeclarationKind::Const {
                let mut names = Vec::new();
                for decl in declarations {
                    collect_target_names(&decl.target, &mut names);
                }
                if !names.is_empty() {
                    let parent = gen.current_lexical_environment();
                    let new_env = gen.allocate_register();
                    gen.emit(Instruction::CreateLexicalEnvironment {
                        dst: new_env.operand(),
                        parent: parent.operand(),
                        capacity: names.len() as u32,
                    });
                    gen.lexical_environment_register_stack.push(new_env);
                    for (name, _) in &names {
                        let id = gen.intern_identifier(&name);
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: EnvironmentMode::Lexical as u32,
                            is_immutable: false,
                            is_global: false,
                            is_strict: false,
                        });
                    }
                    return true;
                }
            }
        }
    }
    false
}

/// Tear down the TDZ environment after RHS evaluation.
fn leave_for_in_of_head_tdz(gen: &mut Generator) {
    gen.lexical_environment_register_stack.pop();
    if !gen.is_current_block_terminated() {
        let parent = gen.current_lexical_environment();
        gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
    }
}

fn generate_for_in_statement(
    gen: &mut Generator,
    lhs: &ForInOfLhs,
    rhs: &Expr,
    body: &Stmt,
    preferred_dst: Option<&ScopedOperand>,
) -> Option<ScopedOperand> {
    // B.3.5 Initializers in ForIn Statement Heads
    // Evaluate the initializer for `for (var x = init in obj)` before the RHS.
    if let ForInOfLhs::Declaration(stmt) = lhs {
        if let Statement::VariableDeclaration { kind: DeclarationKind::Var, declarations } = &stmt.inner {
            if let Some(decl) = declarations.first() {
                if let (VariableDeclaratorTarget::Identifier(ident), Some(init)) = (&decl.target, &decl.init) {
                    let value = generate_expr_or_undefined(init, gen, None);
                    emit_set_variable(gen, ident, &value);
                }
            }
        }
    }

    // Create TDZ for lexical declarations before evaluating the RHS expression.
    let entered_tdz = enter_for_in_of_head_tdz(gen, lhs);
    let object = generate_expr_or_undefined(rhs, gen, None);
    if entered_tdz {
        leave_for_in_of_head_tdz(gen);
    }
    let end_block = gen.make_block();
    let needs_lexical_env = for_in_of_needs_lexical_env(lhs);

    let completion = if gen.must_propagate_completion {
        let reg = gen.allocate_register();
        let undef = gen.add_constant_undefined();
        gen.emit_mov(&reg, &undef);
        Some(reg)
    } else {
        None
    };

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
    let labels = std::mem::take(&mut gen.pending_labels);
    gen.begin_continuable_scope(Label(continue_target as u32), labels.clone(), completion.clone());
    gen.begin_breakable_scope(Label(break_target as u32), labels, completion.clone());

    let saved_completion = gen.current_completion_register.clone();
    if let Some(ref c) = completion {
        gen.current_completion_register = Some(c.clone());
    }
    let body_result = generate_stmt(body, gen, preferred_dst);
    if !gen.is_current_block_terminated() {
        if let (Some(ref c), Some(ref body_val)) = (&completion, &body_result) {
            gen.emit_mov(c, body_val);
        }
    }
    gen.current_completion_register = saved_completion;

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
    completion
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
    // NB: The Rust parser wraps for/for-in/for-of loops in a Block for scope
    // management, so we look through single-child Block wrappers.
    let block_scope_borrow;
    let effective_inner = if let Statement::Block(ref scope) = inner.inner {
        block_scope_borrow = scope.borrow();
        if block_scope_borrow.children.len() == 1 {
            &block_scope_borrow.children[0]
        } else {
            inner
        }
    } else {
        inner
    };
    let is_iteration_or_switch = matches!(
        &effective_inner.inner,
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
        gen.begin_breakable_scope(Label(end_block as u32), labels, None);
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
    // Create TDZ for lexical declarations before evaluating the RHS expression.
    let entered_tdz = enter_for_in_of_head_tdz(gen, lhs);
    let object = generate_expr_or_undefined(rhs, gen, None);
    if entered_tdz {
        leave_for_in_of_head_tdz(gen);
    }
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

    let completion = if gen.must_propagate_completion {
        let reg = gen.allocate_register();
        let undef = gen.add_constant_undefined();
        gen.emit_mov(&reg, &undef);
        Some(reg)
    } else {
        None
    };

    // Break scope wraps the ReturnToFinally so break hits ReturnToFinally first.
    let labels = std::mem::take(&mut gen.pending_labels);
    gen.begin_breakable_scope(Label(end_block as u32), labels.clone(), completion.clone());
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
    gen.begin_continuable_scope(Label(update_block as u32), labels, completion.clone());

    let saved_completion = gen.current_completion_register.clone();
    if let Some(ref c) = completion {
        gen.current_completion_register = Some(c.clone());
    }
    let body_result = generate_stmt(body, gen, preferred_dst);
    if !gen.is_current_block_terminated() {
        if let (Some(ref c), Some(ref body_val)) = (&completion, &body_result) {
            gen.emit_mov(c, body_val);
        }
    }
    gen.current_completion_register = saved_completion;

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
    completion
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
                    // For var: FDI already initialized the binding, so use Set.
                    // For let/const: per-iteration env created new bindings needing Initialize.
                    let mode = match kind {
                        DeclarationKind::Var => BindingMode::Set,
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
    /// Assignment expressions / var iteration: emit SetLexicalBinding or SetGlobal.
    Set,
}

fn set_pending_lhs_name_for_entry(gen: &mut Generator, entry: &BindingEntry) {
    let name = match &entry.alias {
        BindingEntryAlias::Identifier(id) => Some(&id.name),
        BindingEntryAlias::Empty => {
            if let BindingEntryName::Identifier(id) = &entry.name {
                Some(&id.name)
            } else {
                None
            }
        }
        _ => None,
    };
    if let Some(name) = name {
        gen.pending_lhs_name = Some(gen.intern_identifier(&name));
    }
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
        let id = gen.intern_identifier(&ident.name);
        match mode {
            BindingMode::InitializeLexical => {
                gen.emit(Instruction::InitializeLexicalBinding {
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
            // 13.15.5.3 AssignmentRestElement: ... DestructuringAssignmentTarget
            // Step 1: Evaluate the reference BEFORE iterating remaining elements.
            let evaluated_ref = if let BindingEntryAlias::MemberExpression(expr) = &entry.alias {
                Some(emit_evaluate_member_reference(gen, expr))
            } else {
                None
            };

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

            if let Some(ref eref) = evaluated_ref {
                emit_store_to_evaluated_reference(gen, eref, &value);
            } else {
                assign_binding_entry_alias(gen, entry, &value, mode);
            }
            return; // rest consumes the iterator
        }

        // 13.15.5.5 AssignmentElement: DestructuringAssignmentTarget Initializer(opt)
        // Step 1: Evaluate the reference BEFORE calling IteratorStepValue.
        let evaluated_ref = if let BindingEntryAlias::MemberExpression(expr) = &entry.alias {
            Some(emit_evaluate_member_reference(gen, expr))
        } else {
            None
        };

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
            set_pending_lhs_name_for_entry(gen, entry);
            if let Some(default_value) = generate_expr(initializer, gen, None) {
                gen.emit_mov(&value, &default_value);
            }
            gen.emit(Instruction::Jump {
                target: Label(if_not_undefined as u32),
            });
            gen.switch_to_basic_block(if_not_undefined);
        }

        if !is_elision {
            if let Some(ref eref) = evaluated_ref {
                emit_store_to_evaluated_reference(gen, eref, &value);
            } else {
                assign_binding_entry_alias(gen, entry, &value, mode);
            }
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
        completion_type: CompletionType::Normal as u32,
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
        .is_some_and(|e| e.is_rest);

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
                emit_get_by_id(gen, &value, object, &ident.name, None);
                if has_rest {
                    let name_val = gen.add_constant_string(ident.name.clone());
                    excluded_names.push(name_val);
                }
            }
            BindingEntryName::Expression(expr) => {
                let property_name = generate_expr_or_undefined(expr, gen, None);
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
            set_pending_lhs_name_for_entry(gen, entry);
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
    let saved_env = gen.current_lexical_environment();

    let mut next_block: Option<usize> = None;
    let mut completion: Option<ScopedOperand> = None;

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
                    let parent = gen.current_lexical_environment();
                    let new_env = gen.allocate_register();
                    gen.emit(Instruction::CreateLexicalEnvironment {
                        dst: new_env.operand(),
                        parent: parent.operand(),
                        capacity: 1,
                    });
                    gen.lexical_environment_register_stack.push(new_env);
                    created_catch_scope = true;

                    let id = gen.intern_identifier(&ident.name);
                    gen.emit(Instruction::CreateVariable {
                        identifier: id,
                        mode: EnvironmentMode::Lexical as u32,
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
            CatchParameter::BindingPattern(pattern) => {
                let mut names: Vec<(Vec<u16>, bool)> = Vec::new();
                collect_pattern_binding_names(pattern, &mut names);

                if !names.is_empty() {
                    let parent = gen.current_lexical_environment();
                    let new_env = gen.allocate_register();
                    gen.emit(Instruction::CreateLexicalEnvironment {
                        dst: new_env.operand(),
                        parent: parent.operand(),
                        capacity: names.len() as u32,
                    });
                    gen.lexical_environment_register_stack.push(new_env);
                    created_catch_scope = true;

                    for (name, _) in &names {
                        let id = gen.intern_identifier(&name);
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: EnvironmentMode::Lexical as u32,
                            is_immutable: false,
                            is_global: false,
                            is_strict: false,
                        });
                    }
                }

                generate_binding_pattern_bytecode(gen, pattern, BindingMode::InitializeLexical, &caught_value);
            }
            CatchParameter::None => {}
        }

        // Catch body gets its own completion register to prevent
        // break/continue inside catch from leaking values.
        let mut catch_completion: Option<ScopedOperand> = None;
        {
            let saved_completion = gen.current_completion_register.clone();
            if gen.must_propagate_completion {
                let reg = gen.allocate_register();
                let undef = gen.add_constant_undefined();
                gen.emit_mov(&reg, &undef);
                gen.current_completion_register = Some(reg.clone());
                catch_completion = Some(reg);
            }
            generate_stmt(&catch.body, gen, None);
            gen.current_completion_register = saved_completion;
        }

        if created_catch_scope {
            gen.lexical_environment_register_stack.pop();
            if !gen.is_current_block_terminated() {
                let parent = gen.current_lexical_environment();
                gen.emit(Instruction::SetLexicalEnvironment { environment: parent.operand() });
            }
        }

        if gen.must_propagate_completion {
            if let Some(ref cc) = catch_completion {
                if !gen.is_current_block_terminated() {
                    let reg = gen.allocate_register();
                    gen.emit_mov(&reg, cc);
                    completion = Some(reg);
                }
            }
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

    // Try body gets its own completion register to prevent
    // break/continue inside try from leaking values.
    {
        let saved_completion = gen.current_completion_register.clone();
        let mut try_completion: Option<ScopedOperand> = None;
        if gen.must_propagate_completion {
            let reg = gen.allocate_register();
            let undef = gen.add_constant_undefined();
            gen.emit_mov(&reg, &undef);
            gen.current_completion_register = Some(reg.clone());
            try_completion = Some(reg);
        }
        generate_stmt(&data.block, gen, None);
        gen.current_completion_register = saved_completion;

        if !gen.is_current_block_terminated() {
            if gen.must_propagate_completion {
                if let Some(ref tc) = try_completion {
                    let reg = gen.allocate_register();
                    gen.emit_mov(&reg, tc);
                    completion = Some(reg);
                }
            }
        }
    }

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

        // Generate the finally body with a throwaway completion register
        // to prevent break/continue in finally from leaking the try/catch
        // completion value.
        if let Some(finalizer) = &data.finalizer {
            let saved_completion = gen.current_completion_register.clone();
            if gen.must_propagate_completion {
                let finally_completion = gen.allocate_register();
                let undef = gen.add_constant_undefined();
                gen.emit_mov(&finally_completion, &undef);
                gen.current_completion_register = Some(finally_completion);
            }
            generate_stmt(finalizer, gen, None);
            gen.current_completion_register = saved_completion;
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

    if gen.must_propagate_completion {
        if completion.is_none() {
            return Some(gen.add_constant_undefined());
        }
    }
    completion
}

/// Create a SharedFunctionInstanceData for a function expression/declaration
/// and register it with the generator.
///
/// Returns the shared_function_data_index for use in NewFunction instructions.
fn emit_new_function(
    gen: &mut Generator,
    data: &FunctionData,
    name_override: Option<&[u16]>,
) -> u32 {
    assert!(
        data.source_text_end as usize <= gen.source_len,
        "Function source range out of bounds: {}..{} (source len {})",
        data.source_text_start,
        data.source_text_end,
        gen.source_len
    );

    let sfd_ptr = unsafe {
        super::ffi::create_shared_function_data(
            data,
            gen.vm_ptr,
            gen.source_code_ptr,
            gen.strict,
            name_override,
        )
    };

    gen.register_shared_function_data(sfd_ptr)
}

// =============================================================================
// FunctionDeclarationInstantiation (FDI)
// =============================================================================

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

    // Determine if arguments object is needed (from parsing insights).
    let mut arguments_object_needed = func_data.parsing_insights.might_need_arguments_object;

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
            let parent = gen.current_lexical_environment();
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
            let id = gen.intern_identifier(&name);
            gen.emit(Instruction::CreateVariable {
                identifier: id,
                mode: EnvironmentMode::Lexical as u32,
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
            ArgumentsKind::Unmapped as u32
        } else {
            ArgumentsKind::Mapped as u32
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
                    let id = gen.intern_identifier(&ident.name);
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

                if let Some((local_type, idx)) = var.local {
                    let undef = gen.add_constant_undefined();
                    let local = var_local_operand(gen, local_type, idx);
                    gen.emit_mov(&local, &undef);
                } else {
                    let id = gen.intern_identifier(&var.name);
                    let undef = gen.add_constant_undefined();
                    gen.emit(Instruction::CreateVariable {
                        identifier: id,
                        mode: EnvironmentMode::Var as u32,
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
            let has_non_local_vars = fsd.vars_to_initialize.iter().any(|v| v.local.is_none());

            if has_non_local_vars {
                gen.emit(Instruction::CreateVariableEnvironment {
                    capacity: fsd.non_local_var_count_for_parameter_expressions as u32,
                });
                // After CreateVariableEnvironment, re-read the lexical environment
                // (which was also updated) and push it onto the register stack.
                // This ensures subsequent CreateLexicalEnvironment instructions
                // (e.g. Step 7) use the var environment as their parent, not the
                // parameter scope.
                let var_env = gen.allocate_register();
                gen.emit(Instruction::GetLexicalEnvironment {
                    dst: var_env.operand(),
                });
                gen.lexical_environment_register_stack.push(var_env);
            }

            for var in &fsd.vars_to_initialize {
                let is_in_parameter_bindings = var.is_parameter
                    || (arguments_object_needed && var.name == utf16!("arguments"));

                let initial_value = if !is_in_parameter_bindings || var.is_function_name {
                    gen.add_constant_undefined()
                } else if let Some((local_type, idx)) = var.local {
                    let local = var_local_operand(gen, local_type, idx);
                    let tmp = gen.allocate_register();
                    gen.emit_mov(&tmp, &local);
                    tmp
                } else {
                    let id = gen.intern_identifier(&var.name);
                    let tmp = gen.allocate_register();
                    gen.emit(Instruction::GetBinding {
                        dst: tmp.operand(),
                        identifier: id,
                        cache: EnvironmentCoordinate::empty(),
                    });
                    tmp
                };

                if let Some((local_type, idx)) = var.local {
                    let local = var_local_operand(gen, local_type, idx);
                    gen.emit_mov(&local, &initial_value);
                } else {
                    let id = gen.intern_identifier(&var.name);
                    gen.emit(Instruction::CreateVariable {
                        identifier: id,
                        mode: EnvironmentMode::Var as u32,
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
    if !strict {
        for name in &body_scope.annexb_function_names {
            gen.annexb_function_names.insert(name.clone());
            let id = gen.intern_identifier(&name);
            gen.emit(Instruction::CreateVariable {
                identifier: id,
                mode: EnvironmentMode::Var as u32,
                is_immutable: false,
                is_global: false,
                is_strict: false,
            });
            let undef = gen.add_constant_undefined();
            gen.emit(Instruction::InitializeVariableBinding {
                identifier: id,
                src: undef.operand(),
                cache: EnvironmentCoordinate::empty(),
            });
        }
    }

    // --- Step 7: Lexical environment for non-local declarations ---

    let has_non_local_lexical_declarations = needs_block_declaration_instantiation(body_scope);

    if !strict && has_non_local_lexical_declarations {
        let parent = gen.current_lexical_environment();
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
                            let id = gen.intern_identifier(&name);
                            gen.emit(Instruction::CreateVariable {
                                identifier: id,
                                mode: EnvironmentMode::Lexical as u32,
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
                        let id = gen.intern_identifier(&name_ident.name);
                        gen.emit(Instruction::CreateVariable {
                            identifier: id,
                            mode: EnvironmentMode::Lexical as u32,
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
                        let id = gen.intern_identifier(&name_ident.name);
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

/// Check if a block needs block declaration instantiation.
/// True when the block has function declarations or non-local let/const/class.
fn needs_block_declaration_instantiation(scope: &ScopeData) -> bool {
    for child in &scope.children {
        match &child.inner {
            Statement::FunctionDeclaration(_) => {
                return true;
            }
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

/// Create a ScopedOperand for a VarToInit's local variable (argument or variable).
fn var_local_operand(gen: &mut Generator, local_type: LocalType, idx: u32) -> ScopedOperand {
    match local_type {
        LocalType::Variable => gen.local(idx),
        LocalType::Argument => gen.scoped_operand(Operand::argument(idx)),
        LocalType::None => unreachable!("var_local_operand called with LocalType::None"),
    }
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

/// Try to constant-fold a binary operation when both operands are constants.
fn try_constant_fold_binary(
    gen: &mut Generator,
    op: BinaryOp,
    lhs: &ScopedOperand,
    rhs: &ScopedOperand,
) -> Option<ScopedOperand> {
    let lhs_const = gen.get_constant(lhs)?;
    let rhs_const = gen.get_constant(rhs)?;
    match op {
        BinaryOp::Addition => {
            match (lhs_const, rhs_const) {
                (ConstantValue::String(a), ConstantValue::String(b)) => {
                    let mut result = a.clone();
                    result.extend_from_slice(b);
                    Some(gen.add_constant_string(result))
                }
                (ConstantValue::Number(a), ConstantValue::Number(b)) => {
                    Some(gen.add_constant_number(a + b))
                }
                // String + Number or Number + String coercion
                (ConstantValue::String(a), ConstantValue::Number(b)) => {
                    let mut result = a.clone();
                    result.extend(number_to_utf16(*b));
                    Some(gen.add_constant_string(result))
                }
                (ConstantValue::Number(a), ConstantValue::String(b)) => {
                    let mut result = number_to_utf16(*a);
                    result.extend_from_slice(b);
                    Some(gen.add_constant_string(result))
                }
                _ => None,
            }
        }
        BinaryOp::Subtraction => {
            if let (ConstantValue::Number(a), ConstantValue::Number(b)) = (lhs_const, rhs_const) {
                return Some(gen.add_constant_number(a - b));
            }
            None
        }
        BinaryOp::Multiplication => {
            if let (ConstantValue::Number(a), ConstantValue::Number(b)) = (lhs_const, rhs_const) {
                return Some(gen.add_constant_number(a * b));
            }
            None
        }
        BinaryOp::Division => {
            if let (ConstantValue::Number(a), ConstantValue::Number(b)) = (lhs_const, rhs_const) {
                return Some(gen.add_constant_number(a / b));
            }
            None
        }
        BinaryOp::Modulo => {
            if let (ConstantValue::Number(a), ConstantValue::Number(b)) = (lhs_const, rhs_const) {
                return Some(gen.add_constant_number(a % b));
            }
            None
        }
        BinaryOp::Exponentiation => {
            if let (ConstantValue::Number(a), ConstantValue::Number(b)) = (lhs_const, rhs_const) {
                return Some(gen.add_constant_number(a.powf(*b)));
            }
            None
        }
        BinaryOp::StrictlyEquals => {
            match (lhs_const, rhs_const) {
                (ConstantValue::Number(a), ConstantValue::Number(b)) => Some(gen.add_constant_boolean(a == b)),
                (ConstantValue::String(a), ConstantValue::String(b)) => Some(gen.add_constant_boolean(a == b)),
                (ConstantValue::Boolean(a), ConstantValue::Boolean(b)) => Some(gen.add_constant_boolean(a == b)),
                (ConstantValue::Null, ConstantValue::Null) => Some(gen.add_constant_boolean(true)),
                (ConstantValue::Undefined, ConstantValue::Undefined) => Some(gen.add_constant_boolean(true)),
                _ => None,
            }
        }
        BinaryOp::StrictlyInequals => {
            match (lhs_const, rhs_const) {
                (ConstantValue::Number(a), ConstantValue::Number(b)) => Some(gen.add_constant_boolean(a != b)),
                (ConstantValue::String(a), ConstantValue::String(b)) => Some(gen.add_constant_boolean(a != b)),
                (ConstantValue::Boolean(a), ConstantValue::Boolean(b)) => Some(gen.add_constant_boolean(a != b)),
                (ConstantValue::Null, ConstantValue::Null) => Some(gen.add_constant_boolean(false)),
                (ConstantValue::Undefined, ConstantValue::Undefined) => Some(gen.add_constant_boolean(false)),
                _ => None,
            }
        }
        _ => None,
    }
}

fn number_to_utf16(n: f64) -> Vec<u16> {
    // Simple number-to-string for constant folding purposes.
    if n == 0.0 && n.is_sign_positive() {
        return vec![b'0' as u16];
    }
    let s = if n == (n as i64 as f64) && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        format!("{}", n)
    };
    s.encode_utf16().collect()
}

// NanBoxed Value encoding helpers matching C++ GC::NanBoxedValue.
// Used by NewPrimitiveArray to encode constant primitive values inline.
const NANBOX_TAG_SHIFT: u64 = 48;
const NANBOX_BASE_TAG: u64 = 0x7FF8;
const NANBOX_INT32_TAG: u64 = 0b010 | NANBOX_BASE_TAG;
const NANBOX_BOOLEAN_TAG: u64 = 0b001 | NANBOX_BASE_TAG;
const NANBOX_NULL_TAG: u64 = 0b111 | NANBOX_BASE_TAG;
const NANBOX_EMPTY_TAG: u64 = 0b011 | NANBOX_BASE_TAG;
const NEGATIVE_ZERO_BITS: u64 = 1u64 << 63;

fn nanboxed_number(value: f64) -> u64 {
    let is_negative_zero = value.to_bits() == NEGATIVE_ZERO_BITS;
    if value >= i32::MIN as f64
        && value <= i32::MAX as f64
        && value.trunc() == value
        && !is_negative_zero
    {
        (NANBOX_INT32_TAG << NANBOX_TAG_SHIFT) | ((value as i32 as u32) as u64)
    } else if value.is_nan() {
        // Canon NaN
        0x7FF8_0000_0000_0000u64
    } else {
        value.to_bits()
    }
}

fn nanboxed_boolean(value: bool) -> u64 {
    (NANBOX_BOOLEAN_TAG << NANBOX_TAG_SHIFT) | (value as u64)
}

fn nanboxed_null() -> u64 {
    NANBOX_NULL_TAG << NANBOX_TAG_SHIFT
}

fn nanboxed_empty() -> u64 {
    NANBOX_EMPTY_TAG << NANBOX_TAG_SHIFT
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
            Some(gen.intern_identifier(&s))
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
        Expression::StringLiteral(s) => {
            let mut result = utf16!("'").to_vec();
            result.extend_from_slice(s);
            result.extend_from_slice(utf16!("'"));
            result
        }
        Expression::NumericLiteral(n) => {
            let s = format!("{}", n);
            s.encode_utf16().collect()
        }
        Expression::This => utf16!("this").to_vec(),
        _ => utf16!("<object>").to_vec(),
    }
}
