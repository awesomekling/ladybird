/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! AST types for JavaScript.
//!
//! This module defines the Abstract Syntax Tree using idiomatic Rust enums.
//! Every node carries a `SourceRange` for error messages and source maps.
//!
//! ## Design
//!
//! - `ExpressionKind` and `StatementKind` are flat enums — pattern matching
//!   replaces virtual dispatch.
//! - `Node<T>` wraps every AST node with source location info.
//! - `Identifier` uses `Cell` fields for scope analysis results that are
//!   written after parsing (by the scope collector).
//! - Operator enums use `#[repr(u8)]` with ABI-compatible values for
//!   trivial FFI conversion.
//! - `ScopeData` is carried by block-like constructs (Program,
//!   BlockStatement, FunctionBody, etc.) and holds scope analysis results.

use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::fmt;
use std::rc::Rc;

use std::collections::HashMap;

// =============================================================================
// Function table (side table for FunctionData)
// =============================================================================

/// Opaque handle into the `FunctionTable`. Copy + Clone so AST nodes can
/// freely duplicate it without cloning the underlying `FunctionData`.
#[derive(Clone, Copy, Debug, Hash, Eq, PartialEq)]
pub struct FunctionId(u32);

/// Flat side table that owns all `FunctionData` produced during parsing.
///
/// The parser inserts `FunctionData` via `insert()` and receives a
/// `FunctionId`. Later consumers (ast_dump, scope collector, codegen)
/// either borrow via `get()` or take ownership via `take()`.
///
/// `take()` replaces the slot with `None` so each `FunctionData` is
/// moved out exactly once (during codegen / GDI). This eliminates the
/// deep clone that was previously required in `create_shared_function_data`.
pub struct FunctionTable {
    functions: HashMap<FunctionId, Box<FunctionData>>,
    next_id: u32,
}

impl Default for FunctionTable {
    fn default() -> Self {
        Self::new()
    }
}

impl FunctionTable {
    pub fn new() -> Self {
        Self {
            functions: HashMap::default(),
            next_id: 0,
        }
    }

    /// Insert a `FunctionData`, returning a `FunctionId` handle.
    pub fn insert(&mut self, data: FunctionData) -> FunctionId {
        let id = FunctionId(self.next_id);
        self.next_id += 1;
        self.functions.insert(id, Box::new(data));
        id
    }

    /// Borrow the data (for read-only access like ast_dump).
    ///
    /// # Panics
    /// Panics if the slot was already taken.
    pub fn get(&self, id: FunctionId) -> &FunctionData {
        self.functions.get(&id).expect("FunctionTable::get: slot already taken")
    }

    /// Take ownership of the data (for codegen / GDI).
    ///
    /// # Panics
    /// Panics if the slot was already taken.
    pub fn take(&mut self, id: FunctionId) -> Box<FunctionData> {
        self.functions
            .remove(&id)
            .expect("FunctionTable::take: slot already taken")
    }

    /// Take ownership if the slot is still present; returns None if already taken.
    fn try_take(&mut self, id: FunctionId) -> Option<Box<FunctionData>> {
        self.functions.remove(&id)
    }

    /// Insert a `Box<FunctionData>` at a specific id.
    fn insert_at(&mut self, id: FunctionId, data: Box<FunctionData>) {
        self.functions.insert(id, data);
    }

    /// Extract the nested function subtree needed to compile `data` later.
    ///
    /// Parser-created functions carry an explicit child list, so this is
    /// normally proportional to the number of nested functions instead of the
    /// size of the function body.
    pub fn extract_reachable(&mut self, data: &FunctionData) -> FunctionTable {
        let mut subtable = FunctionTable::new();
        if let Some(nested_function_ids) = &data.nested_function_ids {
            for id in nested_function_ids {
                self.transfer(*id, &mut subtable);
            }
        } else {
            // Synthetic function wrappers created during codegen (for example
            // class field initializers) do not come from the parser's function
            // context stack, so keep the structural scan for those rare cases.
            for param in &data.parameters {
                if let Some(ref default) = param.default_value {
                    self.collect_from_expression(default, &mut subtable);
                }
                if let FunctionParameterBinding::BindingPattern(ref pat) = param.binding {
                    self.collect_from_pattern(pat, &mut subtable);
                }
            }
            self.collect_from_statement(&data.body, &mut subtable);
        }
        subtable
    }

    fn transfer(&mut self, id: FunctionId, result: &mut FunctionTable) {
        if let Some(data) = self.try_take(id) {
            if let Some(nested_function_ids) = &data.nested_function_ids {
                for id in nested_function_ids {
                    self.transfer(*id, result);
                }
            } else {
                for param in &data.parameters {
                    if let Some(ref default) = param.default_value {
                        self.collect_from_expression(default, result);
                    }
                    if let FunctionParameterBinding::BindingPattern(ref pat) = param.binding {
                        self.collect_from_pattern(pat, result);
                    }
                }
                self.collect_from_statement(&data.body, result);
            }
            result.insert_at(id, data);
        }
    }

    fn collect_from_statement(&mut self, stmt: &Statement, result: &mut FunctionTable) {
        match &stmt.inner {
            StatementKind::FunctionDeclaration(data) => {
                self.transfer(data.function_id, result);
            }
            StatementKind::Expression(expr) => self.collect_from_expression(expr, result),
            StatementKind::Block(scope) | StatementKind::FunctionBody { scope, .. } => {
                for child in &scope.borrow().children {
                    self.collect_from_statement(child, result);
                }
            }
            StatementKind::Program(data) => {
                for child in &data.scope.borrow().children {
                    self.collect_from_statement(child, result);
                }
            }
            StatementKind::If(data) => {
                self.collect_from_expression(&data.test, result);
                self.collect_from_statement(&data.consequent, result);
                if let Some(alt) = &data.alternate {
                    self.collect_from_statement(alt, result);
                }
            }
            StatementKind::While(data) => {
                self.collect_from_expression(&data.test, result);
                self.collect_from_statement(&data.body, result);
            }
            StatementKind::DoWhile(data) => {
                self.collect_from_statement(&data.body, result);
                self.collect_from_expression(&data.test, result);
            }
            StatementKind::For(data) => {
                if let Some(init) = &data.init {
                    match init {
                        ForInit::Expression(expr) => self.collect_from_expression(expr, result),
                        ForInit::Declaration(decl) => self.collect_from_statement(decl, result),
                    }
                }
                if let Some(test) = &data.test {
                    self.collect_from_expression(test, result);
                }
                if let Some(update) = &data.update {
                    self.collect_from_expression(update, result);
                }
                self.collect_from_statement(&data.body, result);
            }
            StatementKind::ForInOf(data) => {
                match &data.lhs {
                    ForInOfLhs::Declaration(decl) => self.collect_from_statement(decl, result),
                    ForInOfLhs::Expression(expr) => self.collect_from_expression(expr, result),
                    ForInOfLhs::Pattern(pattern) => self.collect_from_pattern(pattern, result),
                }
                self.collect_from_expression(&data.rhs, result);
                self.collect_from_statement(&data.body, result);
            }
            StatementKind::Switch(data) => {
                self.collect_from_expression(&data.discriminant, result);
                for case in &data.cases {
                    if let Some(ref test) = case.test {
                        self.collect_from_expression(test, result);
                    }
                    for child in &case.scope.borrow().children {
                        self.collect_from_statement(child, result);
                    }
                }
            }
            StatementKind::With(data) => {
                self.collect_from_expression(&data.object, result);
                self.collect_from_statement(&data.body, result);
            }
            StatementKind::Labelled(data) => {
                self.collect_from_statement(&data.item, result);
            }
            StatementKind::Return(arg) => {
                if let Some(expr) = arg {
                    self.collect_from_expression(expr, result);
                }
            }
            StatementKind::Throw(expr) => {
                self.collect_from_expression(expr, result);
            }
            StatementKind::Try(data) => {
                self.collect_from_statement(&data.block, result);
                if let Some(ref handler) = data.handler {
                    if let Some(CatchBinding::BindingPattern(ref pat)) = handler.parameter {
                        self.collect_from_pattern(pat, result);
                    }
                    self.collect_from_statement(&handler.body, result);
                }
                if let Some(ref finalizer) = data.finalizer {
                    self.collect_from_statement(finalizer, result);
                }
            }
            StatementKind::VariableDeclaration(data) => {
                for decl in &data.declarations {
                    self.collect_from_target(&decl.target, result);
                    if let Some(ref init) = decl.init {
                        self.collect_from_expression(init, result);
                    }
                }
            }
            StatementKind::UsingDeclaration(declarations) => {
                for decl in declarations.iter() {
                    self.collect_from_target(&decl.target, result);
                    if let Some(ref init) = decl.init {
                        self.collect_from_expression(init, result);
                    }
                }
            }
            StatementKind::ClassDeclaration(class_data) => {
                self.collect_from_class(class_data, result);
            }
            StatementKind::Export(data) => {
                if let Some(ref stmt) = data.statement {
                    self.collect_from_statement(stmt, result);
                }
            }
            StatementKind::ClassFieldInitializer(data) => {
                self.collect_from_expression(&data.expression, result);
            }
            StatementKind::Empty
            | StatementKind::Debugger
            | StatementKind::Break { .. }
            | StatementKind::Continue { .. }
            | StatementKind::Import(_)
            | StatementKind::Error
            | StatementKind::ErrorDeclaration => {}
        }
    }

    fn collect_from_expression(&mut self, expr: &Expression, result: &mut FunctionTable) {
        match &expr.inner {
            ExpressionKind::Function(function_id) => {
                self.transfer(*function_id, result);
            }
            ExpressionKind::Class(class_data) => {
                self.collect_from_class(class_data, result);
            }
            ExpressionKind::Binary(data) => {
                self.collect_from_expression(&data.lhs, result);
                self.collect_from_expression(&data.rhs, result);
            }
            ExpressionKind::Logical(data) => {
                self.collect_from_expression(&data.lhs, result);
                self.collect_from_expression(&data.rhs, result);
            }
            ExpressionKind::Unary { operand, .. } => {
                self.collect_from_expression(operand, result);
            }
            ExpressionKind::Update(data) => {
                self.collect_from_expression(&data.argument, result);
            }
            ExpressionKind::Assignment(data) => {
                match &data.lhs {
                    AssignmentLhs::Expression(expr) => self.collect_from_expression(expr, result),
                    AssignmentLhs::Pattern(pat) => self.collect_from_pattern(pat, result),
                }
                self.collect_from_expression(&data.rhs, result);
            }
            ExpressionKind::Conditional(data) => {
                self.collect_from_expression(&data.test, result);
                self.collect_from_expression(&data.consequent, result);
                self.collect_from_expression(&data.alternate, result);
            }
            ExpressionKind::Sequence(exprs) => {
                for expr in exprs.iter() {
                    self.collect_from_expression(expr, result);
                }
            }
            ExpressionKind::Member(data) => {
                self.collect_from_expression(&data.object, result);
                self.collect_from_expression(&data.property, result);
            }
            ExpressionKind::OptionalChain(data) => {
                self.collect_from_expression(&data.base, result);
                for reference in &data.references {
                    match reference {
                        OptionalChainReference::Call { arguments, .. } => {
                            for arg in arguments {
                                self.collect_from_expression(&arg.value, result);
                            }
                        }
                        OptionalChainReference::ComputedReference { expression, .. } => {
                            self.collect_from_expression(expression, result);
                        }
                        OptionalChainReference::MemberReference { .. }
                        | OptionalChainReference::PrivateMemberReference { .. } => {}
                    }
                }
            }
            ExpressionKind::Call(data) | ExpressionKind::New(data) => {
                self.collect_from_expression(&data.callee, result);
                for arg in &data.arguments {
                    self.collect_from_expression(&arg.value, result);
                }
            }
            ExpressionKind::SuperCall(data) => {
                for arg in &data.arguments {
                    self.collect_from_expression(&arg.value, result);
                }
            }
            ExpressionKind::Spread(expr) | ExpressionKind::Await(expr) => {
                self.collect_from_expression(expr, result);
            }
            ExpressionKind::Array(elements) => {
                for expr in elements.iter().flatten() {
                    self.collect_from_expression(expr, result);
                }
            }
            ExpressionKind::Object(properties) => {
                for prop in properties.iter() {
                    self.collect_from_expression(&prop.key, result);
                    if let Some(ref val) = prop.value {
                        self.collect_from_expression(val, result);
                    }
                }
            }
            ExpressionKind::TemplateLiteral(data) => {
                for expr in &data.expressions {
                    self.collect_from_expression(expr, result);
                }
            }
            ExpressionKind::TaggedTemplateLiteral(data) => {
                self.collect_from_expression(&data.tag, result);
                self.collect_from_expression(&data.template_literal, result);
            }
            ExpressionKind::Yield(data) => {
                if let Some(ref expr) = data.argument {
                    self.collect_from_expression(expr, result);
                }
            }
            ExpressionKind::ImportCall(data) => {
                self.collect_from_expression(&data.specifier, result);
                if let Some(ref opts) = data.options {
                    self.collect_from_expression(opts, result);
                }
            }
            ExpressionKind::NumericLiteral(_)
            | ExpressionKind::StringLiteral(_)
            | ExpressionKind::BooleanLiteral(_)
            | ExpressionKind::NullLiteral
            | ExpressionKind::BigIntLiteral(_)
            | ExpressionKind::RegExpLiteral(_)
            | ExpressionKind::Identifier(_)
            | ExpressionKind::PrivateIdentifier(_)
            | ExpressionKind::This
            | ExpressionKind::Super
            | ExpressionKind::MetaProperty(_)
            | ExpressionKind::Error => {}
        }
    }

    fn collect_from_class(&mut self, class_data: &ClassData, result: &mut FunctionTable) {
        if let Some(ref super_class) = class_data.super_class {
            self.collect_from_expression(super_class, result);
        }
        if let Some(ref constructor) = class_data.constructor {
            self.collect_from_expression(constructor, result);
        }
        for element in &class_data.elements {
            match &element.inner {
                ClassElement::Method { key, function, .. } => {
                    self.collect_from_expression(key, result);
                    self.collect_from_expression(function, result);
                }
                ClassElement::Field { key, initializer, .. } => {
                    self.collect_from_expression(key, result);
                    if let Some(init) = initializer {
                        self.collect_from_expression(init, result);
                    }
                }
                ClassElement::StaticInitializer { body } => {
                    self.collect_from_statement(body, result);
                }
            }
        }
    }

    fn collect_from_pattern(&mut self, pattern: &BindingPattern, result: &mut FunctionTable) {
        for entry in &pattern.entries {
            if let Some(BindingEntryName::Expression(expr)) = entry.name.as_ref() {
                self.collect_from_expression(expr, result);
            }
            if let Some(ref alias) = entry.alias {
                match alias {
                    BindingEntryAlias::BindingPattern(sub) => {
                        self.collect_from_pattern(sub, result);
                    }
                    BindingEntryAlias::MemberExpression(expr) => {
                        self.collect_from_expression(expr, result);
                    }
                    BindingEntryAlias::Identifier(_) => {}
                }
            }
            if let Some(ref init) = entry.initializer {
                self.collect_from_expression(init, result);
            }
        }
    }

    fn collect_from_target(&mut self, target: &VariableDeclaratorTarget, result: &mut FunctionTable) {
        if let VariableDeclaratorTarget::BindingPattern(pat) = target {
            self.collect_from_pattern(pat, result);
        }
    }
}

/// Bundles a `FunctionData` with a subtable of all nested functions
/// reachable from its body. Stored as the raw pointer in C++ SFDs.
pub struct FunctionPayload {
    pub data: FunctionData,
    pub function_table: FunctionTable,
}

// Background workers compile and drop these payloads independently. Detach replaces parser-shared Rc/RefCell/Cell
// state with private copies so each FunctionPayload can safely move through the C++ ThreadPool on its own.
impl FunctionPayload {
    pub fn detach_for_background_compilation(&mut self) {
        detach_function_data(&mut self.data);
        self.function_table.detach_for_background_compilation();
    }
}

impl FunctionTable {
    pub fn detach_for_background_compilation(&mut self) {
        for function_data in self.functions.values_mut() {
            detach_function_data(function_data);
        }
    }
}

fn detach_function_data(data: &mut FunctionData) {
    detach_optional_identifier(&mut data.name);
    detach_statement(&mut data.body);
    for parameter in &mut data.parameters {
        detach_function_parameter(parameter);
    }
}

fn detach_function_parameter(parameter: &mut FunctionParameter) {
    detach_function_parameter_binding(&mut parameter.binding);
    if let Some(default_value) = &mut parameter.default_value {
        detach_expression(default_value);
    }
}

fn detach_function_parameter_binding(binding: &mut FunctionParameterBinding) {
    match binding {
        FunctionParameterBinding::Identifier(identifier) => detach_identifier(identifier),
        FunctionParameterBinding::BindingPattern(pattern) => detach_binding_pattern(pattern),
    }
}

fn detach_identifier(identifier: &mut Rc<Identifier>) {
    let old_identifier = identifier.as_ref();
    let mut name = old_identifier.name.clone();
    detach_shared_utf16_string(&mut name);
    *identifier = Rc::new(Identifier {
        range: old_identifier.range,
        name,
        local_type: Cell::new(old_identifier.local_type.get()),
        local_index: Cell::new(old_identifier.local_index.get()),
        is_global: Cell::new(old_identifier.is_global.get()),
        is_inside_scope_with_eval: Cell::new(old_identifier.is_inside_scope_with_eval.get()),
        declaration_kind: Cell::new(old_identifier.declaration_kind.get()),
    });
}

fn detach_optional_identifier(identifier: &mut Option<Rc<Identifier>>) {
    if let Some(identifier) = identifier {
        detach_identifier(identifier);
    }
}

fn detach_shared_utf16_string(string: &mut SharedUtf16String) {
    *string = SharedUtf16String::from(string.to_utf16_string());
}

fn detach_scope_ref(scope: &mut Rc<RefCell<ScopeData>>) {
    let mut detached_scope = scope.borrow().clone();
    detach_scope_data(&mut detached_scope);
    *scope = Rc::new(RefCell::new(detached_scope));
}

fn detach_scope_data(scope: &mut ScopeData) {
    for child in &mut scope.children {
        detach_statement(child);
    }
}

fn detach_statement(statement: &mut Statement) {
    match &mut statement.inner {
        StatementKind::Expression(expression) => detach_expression(expression),
        StatementKind::Block(scope) | StatementKind::FunctionBody { scope, .. } => detach_scope_ref(scope),
        StatementKind::Program(data) => detach_scope_ref(&mut data.scope),
        StatementKind::If(data) => {
            detach_expression(&mut data.test);
            detach_statement(&mut data.consequent);
            if let Some(alternate) = &mut data.alternate {
                detach_statement(alternate);
            }
        }
        StatementKind::While(data) | StatementKind::DoWhile(data) => {
            detach_expression(&mut data.test);
            detach_statement(&mut data.body);
        }
        StatementKind::For(data) => {
            if let Some(init) = &mut data.init {
                detach_for_init(init);
            }
            if let Some(test) = &mut data.test {
                detach_expression(test);
            }
            if let Some(update) = &mut data.update {
                detach_expression(update);
            }
            detach_statement(&mut data.body);
        }
        StatementKind::ForInOf(data) => {
            detach_for_in_of_lhs(&mut data.lhs);
            detach_expression(&mut data.rhs);
            detach_statement(&mut data.body);
        }
        StatementKind::Switch(data) => {
            detach_scope_ref(&mut data.scope);
            detach_expression(&mut data.discriminant);
            for case in &mut data.cases {
                detach_scope_ref(&mut case.scope);
                if let Some(test) = &mut case.test {
                    detach_expression(test);
                }
            }
        }
        StatementKind::With(data) => {
            detach_expression(&mut data.object);
            detach_statement(&mut data.body);
        }
        StatementKind::Labelled(data) => detach_statement(&mut data.item),
        StatementKind::Return(argument) => {
            if let Some(argument) = argument {
                detach_expression(argument);
            }
        }
        StatementKind::Throw(expression) => detach_expression(expression),
        StatementKind::Try(data) => {
            detach_statement(&mut data.block);
            if let Some(handler) = &mut data.handler {
                if let Some(parameter) = &mut handler.parameter {
                    detach_catch_binding(parameter);
                }
                detach_statement(&mut handler.body);
            }
            if let Some(finalizer) = &mut data.finalizer {
                detach_statement(finalizer);
            }
        }
        StatementKind::VariableDeclaration(data) => {
            for declaration in &mut data.declarations {
                detach_variable_declarator(declaration);
            }
        }
        StatementKind::UsingDeclaration(declarations) => {
            for declaration in declarations.iter_mut() {
                detach_variable_declarator(declaration);
            }
        }
        StatementKind::FunctionDeclaration(data) => {
            detach_optional_identifier(&mut data.name);
            data.is_hoisted = Cell::new(data.is_hoisted.get());
        }
        StatementKind::ClassDeclaration(data) => detach_class_data(data),
        StatementKind::Export(data) => {
            if let Some(statement) = &mut data.statement {
                detach_statement(statement);
            }
        }
        StatementKind::ClassFieldInitializer(data) => detach_expression(&mut data.expression),
        StatementKind::Empty
        | StatementKind::Error
        | StatementKind::Debugger
        | StatementKind::Break { .. }
        | StatementKind::Continue { .. }
        | StatementKind::Import(_)
        | StatementKind::ErrorDeclaration => {}
    }
}

fn detach_expression(expression: &mut Expression) {
    match &mut expression.inner {
        ExpressionKind::RegExpLiteral(data) => {
            // RegExp literals were already validated by the parser, and materialization now uses pattern+flags.
            data.compiled_regex = Rc::new(CompiledRegex::new(std::ptr::null_mut()));
        }
        ExpressionKind::Identifier(identifier) => detach_identifier(identifier),
        ExpressionKind::Binary(data) => {
            detach_expression(&mut data.lhs);
            detach_expression(&mut data.rhs);
        }
        ExpressionKind::Logical(data) => {
            detach_expression(&mut data.lhs);
            detach_expression(&mut data.rhs);
        }
        ExpressionKind::Unary { operand, .. } => detach_expression(operand),
        ExpressionKind::Update(data) => detach_expression(&mut data.argument),
        ExpressionKind::Assignment(data) => {
            detach_assignment_lhs(&mut data.lhs);
            detach_expression(&mut data.rhs);
        }
        ExpressionKind::Conditional(data) => {
            detach_expression(&mut data.test);
            detach_expression(&mut data.consequent);
            detach_expression(&mut data.alternate);
        }
        ExpressionKind::Sequence(expressions) => {
            for expression in expressions.iter_mut() {
                detach_expression(expression);
            }
        }
        ExpressionKind::Member(data) => {
            detach_expression(&mut data.object);
            detach_expression(&mut data.property);
        }
        ExpressionKind::OptionalChain(data) => {
            detach_expression(&mut data.base);
            for reference in &mut data.references {
                detach_optional_chain_reference(reference);
            }
        }
        ExpressionKind::Call(data) | ExpressionKind::New(data) => {
            detach_expression(&mut data.callee);
            for argument in &mut data.arguments {
                detach_call_argument(argument);
            }
        }
        ExpressionKind::SuperCall(data) => {
            for argument in &mut data.arguments {
                detach_call_argument(argument);
            }
        }
        ExpressionKind::Spread(expression) | ExpressionKind::Await(expression) => detach_expression(expression),
        ExpressionKind::Class(data) => detach_class_data(data),
        ExpressionKind::Array(elements) => {
            for expression in elements.iter_mut().flatten() {
                detach_expression(expression);
            }
        }
        ExpressionKind::Object(properties) => {
            for property in properties.iter_mut() {
                detach_object_property(property);
            }
        }
        ExpressionKind::TemplateLiteral(data) => {
            for expression in &mut data.expressions {
                detach_expression(expression);
            }
        }
        ExpressionKind::TaggedTemplateLiteral(data) => {
            detach_expression(&mut data.tag);
            detach_expression(&mut data.template_literal);
        }
        ExpressionKind::ImportCall(data) => {
            detach_expression(&mut data.specifier);
            if let Some(options) = &mut data.options {
                detach_expression(options);
            }
        }
        ExpressionKind::Yield(data) => {
            if let Some(argument) = &mut data.argument {
                detach_expression(argument);
            }
        }
        ExpressionKind::NumericLiteral(_)
        | ExpressionKind::StringLiteral(_)
        | ExpressionKind::BooleanLiteral(_)
        | ExpressionKind::NullLiteral
        | ExpressionKind::BigIntLiteral(_)
        | ExpressionKind::PrivateIdentifier(_)
        | ExpressionKind::This
        | ExpressionKind::Super
        | ExpressionKind::Function(_)
        | ExpressionKind::MetaProperty(_)
        | ExpressionKind::Error => {}
    }
}

fn detach_class_data(data: &mut ClassData) {
    detach_optional_identifier(&mut data.name);
    if let Some(constructor) = &mut data.constructor {
        detach_expression(constructor);
    }
    if let Some(super_class) = &mut data.super_class {
        detach_expression(super_class);
    }
    for element in &mut data.elements {
        match &mut element.inner {
            ClassElement::Method { key, function, .. } => {
                detach_expression(key);
                detach_expression(function);
            }
            ClassElement::Field { key, initializer, .. } => {
                detach_expression(key);
                if let Some(initializer) = initializer {
                    detach_expression(initializer);
                }
            }
            ClassElement::StaticInitializer { body } => detach_statement(body),
        }
    }
}

fn detach_binding_pattern(pattern: &mut BindingPattern) {
    for entry in &mut pattern.entries {
        if let Some(name) = &mut entry.name {
            detach_binding_entry_name(name);
        }
        if let Some(alias) = &mut entry.alias {
            detach_binding_entry_alias(alias);
        }
        if let Some(initializer) = &mut entry.initializer {
            detach_expression(initializer);
        }
    }
}

fn detach_binding_entry_name(name: &mut BindingEntryName) {
    match name {
        BindingEntryName::Identifier(identifier) => detach_identifier(identifier),
        BindingEntryName::Expression(expression) => detach_expression(expression),
    }
}

fn detach_binding_entry_alias(alias: &mut BindingEntryAlias) {
    match alias {
        BindingEntryAlias::Identifier(identifier) => detach_identifier(identifier),
        BindingEntryAlias::BindingPattern(pattern) => detach_binding_pattern(pattern),
        BindingEntryAlias::MemberExpression(expression) => detach_expression(expression),
    }
}

fn detach_variable_declarator(declaration: &mut VariableDeclarator) {
    detach_variable_declarator_target(&mut declaration.target);
    if let Some(init) = &mut declaration.init {
        detach_expression(init);
    }
}

fn detach_variable_declarator_target(target: &mut VariableDeclaratorTarget) {
    match target {
        VariableDeclaratorTarget::Identifier(identifier) => detach_identifier(identifier),
        VariableDeclaratorTarget::BindingPattern(pattern) => detach_binding_pattern(pattern),
    }
}

fn detach_call_argument(argument: &mut CallArgument) {
    detach_expression(&mut argument.value);
}

fn detach_optional_chain_reference(reference: &mut OptionalChainReference) {
    match reference {
        OptionalChainReference::Call { arguments, .. } => {
            for argument in arguments {
                detach_call_argument(argument);
            }
        }
        OptionalChainReference::ComputedReference { expression, .. } => detach_expression(expression),
        OptionalChainReference::MemberReference { identifier, .. } => detach_identifier(identifier),
        OptionalChainReference::PrivateMemberReference { .. } => {}
    }
}

fn detach_object_property(property: &mut ObjectProperty) {
    detach_expression(&mut property.key);
    if let Some(value) = &mut property.value {
        detach_expression(value);
    }
}

fn detach_for_init(init: &mut ForInit) {
    match init {
        ForInit::Declaration(statement) => detach_statement(statement),
        ForInit::Expression(expression) => detach_expression(expression),
    }
}

fn detach_for_in_of_lhs(lhs: &mut ForInOfLhs) {
    match lhs {
        ForInOfLhs::Declaration(statement) => detach_statement(statement),
        ForInOfLhs::Expression(expression) => detach_expression(expression),
        ForInOfLhs::Pattern(pattern) => detach_binding_pattern(pattern),
    }
}

fn detach_assignment_lhs(lhs: &mut AssignmentLhs) {
    match lhs {
        AssignmentLhs::Expression(expression) => detach_expression(expression),
        AssignmentLhs::Pattern(pattern) => detach_binding_pattern(pattern),
    }
}

fn detach_catch_binding(binding: &mut CatchBinding) {
    match binding {
        CatchBinding::Identifier(identifier) => detach_identifier(identifier),
        CatchBinding::BindingPattern(pattern) => detach_binding_pattern(pattern),
    }
}

// =============================================================================
// Source location
// =============================================================================

/// A UTF-16 encoded string.
///
/// Wraps `Vec<u16>` to provide type safety and distinguish UTF-16 text
/// from arbitrary `u16` buffers. Access the inner Vec via `.0` when
/// Vec-specific methods like `push` or `extend` are needed.
#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, Default)]
pub struct Utf16String(pub Vec<u16>);

impl std::ops::Deref for Utf16String {
    type Target = [u16];
    fn deref(&self) -> &[u16] {
        &self.0
    }
}

impl std::ops::DerefMut for Utf16String {
    fn deref_mut(&mut self) -> &mut [u16] {
        &mut self.0
    }
}

impl From<Vec<u16>> for Utf16String {
    fn from(v: Vec<u16>) -> Self {
        Self(v)
    }
}

impl From<&[u16]> for Utf16String {
    fn from(s: &[u16]) -> Self {
        Self(s.to_vec())
    }
}

impl std::borrow::Borrow<[u16]> for Utf16String {
    fn borrow(&self) -> &[u16] {
        &self.0
    }
}

impl AsRef<[u16]> for Utf16String {
    fn as_ref(&self) -> &[u16] {
        &self.0
    }
}

impl PartialEq<[u16]> for Utf16String {
    fn eq(&self, other: &[u16]) -> bool {
        self.0 == other
    }
}

impl PartialEq<&[u16]> for Utf16String {
    fn eq(&self, other: &&[u16]) -> bool {
        self.0.as_slice() == *other
    }
}

impl PartialEq<Vec<u16>> for Utf16String {
    fn eq(&self, other: &Vec<u16>) -> bool {
        self.0 == *other
    }
}

impl FromIterator<u16> for Utf16String {
    fn from_iter<I: IntoIterator<Item = u16>>(iter: I) -> Self {
        Self(iter.into_iter().collect())
    }
}

impl<'a> IntoIterator for &'a Utf16String {
    type Item = &'a u16;
    type IntoIter = std::slice::Iter<'a, u16>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl Utf16String {
    pub fn new() -> Self {
        Self(Vec::new())
    }

    pub fn as_slice(&self) -> &[u16] {
        &self.0
    }
}

#[derive(Clone, Debug, Hash, Eq, PartialEq, Ord, PartialOrd, Default)]
pub struct SharedUtf16String(pub Rc<Utf16String>);

impl SharedUtf16String {
    pub fn new(value: Utf16String) -> Self {
        Self(Rc::new(value))
    }

    pub fn to_utf16_string(&self) -> Utf16String {
        self.0.as_ref().clone()
    }
}

impl From<Utf16String> for SharedUtf16String {
    fn from(value: Utf16String) -> Self {
        Self::new(value)
    }
}

impl From<Vec<u16>> for SharedUtf16String {
    fn from(value: Vec<u16>) -> Self {
        Self::new(Utf16String::from(value))
    }
}

impl From<&[u16]> for SharedUtf16String {
    fn from(value: &[u16]) -> Self {
        Self::new(Utf16String::from(value))
    }
}

impl std::ops::Deref for SharedUtf16String {
    type Target = Utf16String;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref()
    }
}

impl std::borrow::Borrow<[u16]> for SharedUtf16String {
    fn borrow(&self) -> &[u16] {
        self.0.as_slice()
    }
}

impl AsRef<[u16]> for SharedUtf16String {
    fn as_ref(&self) -> &[u16] {
        self.0.as_slice()
    }
}

impl PartialEq<[u16]> for SharedUtf16String {
    fn eq(&self, other: &[u16]) -> bool {
        self.0.as_slice() == other
    }
}

impl PartialEq<&[u16]> for SharedUtf16String {
    fn eq(&self, other: &&[u16]) -> bool {
        self.0.as_slice() == *other
    }
}

impl PartialEq<Utf16String> for SharedUtf16String {
    fn eq(&self, other: &Utf16String) -> bool {
        self.0.as_slice() == other.as_slice()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Position {
    pub line: u32,
    pub column: u32,
    pub offset: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct SourceRange {
    pub start: Position,
    pub end: Position,
}

// =============================================================================
// Node wrapper
// =============================================================================

/// Every AST node wraps its payload with source location.
#[derive(Clone, Debug)]
pub struct Node<T> {
    pub range: SourceRange,
    pub inner: T,
}

pub type Expression = Node<ExpressionKind>;

pub type Statement = Node<StatementKind>;

impl<T> Node<T> {
    pub fn new(range: SourceRange, inner: T) -> Self {
        Self { range, inner }
    }
}

// =============================================================================
// Operator enums — values are ABI-compatible for FFI
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BinaryOp {
    Addition = 0,
    Subtraction = 1,
    Multiplication = 2,
    Division = 3,
    Modulo = 4,
    Exponentiation = 5,
    StrictlyEquals = 6,
    StrictlyInequals = 7,
    LooselyEquals = 8,
    LooselyInequals = 9,
    GreaterThan = 10,
    GreaterThanEquals = 11,
    LessThan = 12,
    LessThanEquals = 13,
    BitwiseAnd = 14,
    BitwiseOr = 15,
    BitwiseXor = 16,
    LeftShift = 17,
    RightShift = 18,
    UnsignedRightShift = 19,
    In = 20,
    InstanceOf = 21,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum LogicalOp {
    And = 0,
    Or = 1,
    NullishCoalescing = 2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UnaryOp {
    BitwiseNot = 0,
    Not = 1,
    Plus = 2,
    Minus = 3,
    Typeof = 4,
    Void = 5,
    Delete = 6,
}

#[derive(Clone, Copy, Debug)]
#[repr(u8)]
pub enum UpdateOp {
    Increment = 0,
    Decrement = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum AssignmentOp {
    Assignment = 0,
    AdditionAssignment = 1,
    SubtractionAssignment = 2,
    MultiplicationAssignment = 3,
    DivisionAssignment = 4,
    ModuloAssignment = 5,
    ExponentiationAssignment = 6,
    BitwiseAndAssignment = 7,
    BitwiseOrAssignment = 8,
    BitwiseXorAssignment = 9,
    LeftShiftAssignment = 10,
    RightShiftAssignment = 11,
    UnsignedRightShiftAssignment = 12,
    AndAssignment = 13,
    OrAssignment = 14,
    NullishAssignment = 15,
}

// =============================================================================
// Kind enums
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum DeclarationKind {
    Var = 1,
    Let = 2,
    Const = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FunctionKind {
    Normal = 0,
    Generator = 1,
    Async = 2,
    AsyncGenerator = 3,
}

impl FunctionKind {
    pub fn from_async_generator(is_async: bool, is_generator: bool) -> Self {
        match (is_async, is_generator) {
            (true, true) => Self::AsyncGenerator,
            (true, false) => Self::Async,
            (false, true) => Self::Generator,
            (false, false) => Self::Normal,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ProgramType {
    Script = 0,
    Module = 1,
}

#[derive(Clone, Copy, Debug)]
pub enum MetaPropertyType {
    NewTarget,
    ImportMeta,
}

// =============================================================================
// Identifier
// =============================================================================

/// Scope analysis result: how this identifier is resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LocalType {
    Argument,
    Variable,
}

/// An identifier reference or binding name.
///
/// Scope analysis results are stored in `Cell` fields because the scope
/// collector writes them after parsing through shared references.
#[derive(Clone, Debug)]
pub struct Identifier {
    pub range: SourceRange,
    pub name: SharedUtf16String,
    // Scope analysis results — set by scope collector after parsing.
    pub local_type: Cell<Option<LocalType>>,
    pub local_index: Cell<u32>,
    pub is_global: Cell<bool>,
    pub is_inside_scope_with_eval: Cell<bool>,
    pub declaration_kind: Cell<Option<DeclarationKind>>,
}

impl Identifier {
    pub fn new(range: SourceRange, name: SharedUtf16String) -> Self {
        Self {
            range,
            name,
            local_type: Cell::new(None),
            local_index: Cell::new(0),
            is_global: Cell::new(false),
            is_inside_scope_with_eval: Cell::new(false),
            declaration_kind: Cell::new(None),
        }
    }

    pub fn is_local(&self) -> bool {
        self.local_type.get().is_some()
    }
}

#[derive(Clone, Debug)]
pub struct PrivateIdentifier {
    pub range: SourceRange,
    pub name: Utf16String,
}

// =============================================================================
// Function support types
// =============================================================================

/// Parsing insights collected during function body parsing.
///
/// The scope collector populates `uses_this`, `uses_this_from_environment`,
/// and `contains_direct_call_to_eval` during scope analysis.
/// `might_need_arguments_object` is set by the parser during body parsing.
#[derive(Clone, Copy, Debug, Default)]
pub struct FunctionParsingInsights {
    pub uses_this: bool,
    pub uses_this_from_environment: bool,
    pub contains_direct_call_to_eval: bool,
    pub might_need_arguments_object: bool,
}

#[derive(Clone, Debug)]
pub struct FunctionParameter {
    pub binding: FunctionParameterBinding,
    pub default_value: Option<Expression>,
    pub is_rest: bool,
}

#[derive(Clone, Debug)]
pub enum FunctionParameterBinding {
    Identifier(Rc<Identifier>),
    BindingPattern(BindingPattern),
}

/// Shared data for FunctionDeclaration and FunctionExpression.
#[derive(Debug)]
pub struct FunctionData {
    pub name: Option<Rc<Identifier>>,
    pub source_text_start: u32,
    pub source_text_end: u32,
    pub body: Box<Statement>,
    pub parameters: Vec<FunctionParameter>,
    pub function_length: i32,
    pub kind: FunctionKind,
    pub is_strict_mode: bool,
    pub is_arrow_function: bool,
    pub parsing_insights: FunctionParsingInsights,
    /// Parser-created functions know their nested function ids up front, so
    /// lazy-compile payload extraction can move only that subtree instead of
    /// re-walking the full body. `None` is reserved for synthetic function
    /// wrappers built during codegen, where the old structural scan is still
    /// needed to discover nested functions inside the wrapped AST.
    pub nested_function_ids: Option<Vec<FunctionId>>,
}

// =============================================================================
// Class support types
// =============================================================================

/// Shared data for ClassDeclaration and ClassExpression.
#[derive(Clone, Debug)]
pub struct ClassData {
    pub name: Option<Rc<Identifier>>,
    pub source_text_start: u32,
    pub source_text_end: u32,
    pub constructor: Option<Box<Expression>>,
    pub super_class: Option<Box<Expression>>,
    pub elements: Vec<Node<ClassElement>>,
}

#[derive(Clone, Debug)]
pub enum ClassElement {
    Method {
        key: Box<Expression>,
        function: Box<Expression>,
        kind: ClassMethodKind,
        is_static: bool,
    },
    Field {
        key: Box<Expression>,
        initializer: Option<Box<Expression>>,
        is_static: bool,
    },
    StaticInitializer {
        body: Box<Statement>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ClassMethodKind {
    Method = 0,
    Getter = 1,
    Setter = 2,
}

// =============================================================================
// Binding pattern types
// =============================================================================

#[derive(Clone, Debug)]
pub struct BindingPattern {
    pub kind: BindingPatternKind,
    pub entries: Vec<BindingEntry>,
}

impl BindingPattern {
    pub fn contains_expression(&self) -> bool {
        for entry in &self.entries {
            if matches!(entry.name, Some(BindingEntryName::Expression(_))) {
                return true;
            }
            if entry.initializer.is_some() {
                return true;
            }
            if let Some(BindingEntryAlias::BindingPattern(ref nested)) = entry.alias
                && nested.contains_expression()
            {
                return true;
            }
        }
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BindingPatternKind {
    Array,
    Object,
}

#[derive(Clone, Debug)]
pub struct BindingEntry {
    pub name: Option<BindingEntryName>,
    pub alias: Option<BindingEntryAlias>,
    pub initializer: Option<Expression>,
    pub is_rest: bool,
}

/// The "name" part of a binding entry.
/// - `None`: elision in array patterns (`[, , x]`)
/// - `Identifier`: object property shorthand (`{ x }`)
/// - `Expression`: computed property key (`{ [expression]: x }`)
#[derive(Clone, Debug)]
pub enum BindingEntryName {
    Identifier(Rc<Identifier>),
    Expression(Box<Expression>),
}

/// The "alias" (target) of a binding entry.
/// - `None`: name is the binding target (`{ x }` — x is both name and alias)
/// - `Identifier`: simple binding (`{ x: y }`)
/// - `BindingPattern`: nested destructuring (`{ x: { a, b } }`)
/// - `MemberExpression`: assignment target (`{ x: obj.property }`)
#[derive(Clone, Debug)]
pub enum BindingEntryAlias {
    Identifier(Rc<Identifier>),
    BindingPattern(Box<BindingPattern>),
    MemberExpression(Box<Expression>),
}

// =============================================================================
// Variable declaration types
// =============================================================================

#[derive(Clone, Debug)]
pub struct VariableDeclarator {
    pub range: SourceRange,
    pub target: VariableDeclaratorTarget,
    pub init: Option<Expression>,
}

#[derive(Clone, Debug)]
pub enum VariableDeclaratorTarget {
    Identifier(Rc<Identifier>),
    BindingPattern(BindingPattern),
}

// =============================================================================
// Object literal types
// =============================================================================

#[derive(Clone, Debug)]
pub struct ObjectProperty {
    pub range: SourceRange,
    pub property_type: ObjectPropertyType,
    pub key: Box<Expression>,
    pub value: Option<Box<Expression>>,
    pub is_method: bool,
    pub is_computed: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ObjectPropertyType {
    KeyValue = 0,
    Getter = 1,
    Setter = 2,
    Spread = 3,
    ProtoSetter = 4,
}

// =============================================================================
// Call expression types
// =============================================================================

#[derive(Clone, Debug)]
pub struct CallArgument {
    pub value: Expression,
    pub is_spread: bool,
}

#[derive(Clone, Debug)]
pub struct CallExpressionData {
    pub callee: Box<Expression>,
    pub arguments: Vec<CallArgument>,
    pub is_parenthesized: bool,
    pub is_inside_parens: bool,
}

#[derive(Clone, Debug)]
pub struct SuperCallData {
    pub arguments: Vec<CallArgument>,
    pub is_synthetic: bool,
}

// =============================================================================
// Optional chain types
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OptionalChainMode {
    Optional,
    NotOptional,
}

#[derive(Clone, Debug)]
pub enum OptionalChainReference {
    Call {
        arguments: Vec<CallArgument>,
        mode: OptionalChainMode,
    },
    ComputedReference {
        expression: Box<Expression>,
        mode: OptionalChainMode,
    },
    MemberReference {
        identifier: Rc<Identifier>,
        mode: OptionalChainMode,
    },
    PrivateMemberReference {
        private_identifier: PrivateIdentifier,
        mode: OptionalChainMode,
    },
}

// =============================================================================
// Template literal types
// =============================================================================

#[derive(Clone, Debug)]
pub struct TemplateLiteralData {
    pub expressions: Vec<Expression>,
    pub raw_strings: Vec<Utf16String>,
}

// =============================================================================
// RegExp literal
// =============================================================================

unsafe extern "C" {
    fn rust_free_compiled_regex(ptr: *mut c_void);
}

/// Handle to a compiled regex from C++.
///
/// Wrapped in `Rc` in `RegExpLiteralData` so that AST clones (e.g. for
/// class field initializers) share the handle cheaply. The first codegen
/// path to call `take()` gets the handle; `Drop` frees it if untaken.
pub struct CompiledRegex(Cell<*mut c_void>);

impl CompiledRegex {
    pub fn new(ptr: *mut c_void) -> Self {
        Self(Cell::new(ptr))
    }

    /// Take ownership of the compiled regex handle, leaving null behind
    /// so the destructor won't free it.
    pub fn take(&self) -> *mut c_void {
        self.0.replace(std::ptr::null_mut())
    }

    /// Set the compiled regex handle (used by deferred compilation).
    pub fn set(&self, ptr: *mut c_void) {
        self.0.set(ptr);
    }
}

impl Drop for CompiledRegex {
    fn drop(&mut self) {
        let ptr = self.0.get();
        if !ptr.is_null() {
            unsafe { rust_free_compiled_regex(ptr) };
        }
    }
}

impl fmt::Debug for CompiledRegex {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "CompiledRegex({:p})", self.0.get())
    }
}

#[derive(Clone, Debug)]
pub struct RegExpLiteralData {
    pub pattern: Utf16String,
    pub flags: Utf16String,
    pub compiled_regex: Rc<CompiledRegex>,
}

// =============================================================================
// Try/Catch types
// =============================================================================

#[derive(Clone, Debug)]
pub struct TryStatementData {
    pub block: Box<Statement>,
    pub handler: Option<CatchClause>,
    pub finalizer: Option<Box<Statement>>,
}

#[derive(Clone, Debug)]
pub struct CatchClause {
    pub range: SourceRange,
    pub parameter: Option<CatchBinding>,
    pub body: Box<Statement>,
}

#[derive(Clone, Debug)]
pub enum CatchBinding {
    Identifier(Rc<Identifier>),
    BindingPattern(BindingPattern),
}

// =============================================================================
// Switch types
// =============================================================================

#[derive(Clone, Debug)]
pub struct SwitchStatementData {
    pub scope: Rc<RefCell<ScopeData>>,
    pub discriminant: Box<Expression>,
    pub cases: Vec<SwitchCase>,
}

#[derive(Clone, Debug)]
pub struct SwitchCase {
    pub range: SourceRange,
    pub scope: Rc<RefCell<ScopeData>>,
    pub test: Option<Expression>,
}

// =============================================================================
// Module types (import/export)
// =============================================================================

#[derive(Clone, Debug)]
pub struct ModuleRequest {
    pub module_specifier: Utf16String,
    pub attributes: Vec<ImportAttribute>,
}

#[derive(Clone, Debug)]
pub struct ImportAttribute {
    pub key: Utf16String,
    pub value: Utf16String,
}

#[derive(Clone, Debug)]
pub struct ImportEntry {
    /// `None` means namespace import (`import * as x`).
    pub import_name: Option<Utf16String>,
    pub local_name: Utf16String,
}

#[derive(Clone, Debug)]
pub struct ImportStatementData {
    pub module_request: ModuleRequest,
    pub entries: Vec<ImportEntry>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum ExportEntryKind {
    NamedExport = 0,
    ModuleRequestAll = 1,
    ModuleRequestAllButDefault = 2,
    EmptyNamedExport = 3,
}

#[derive(Clone, Debug)]
pub struct ExportEntry {
    pub kind: ExportEntryKind,
    pub export_name: Option<Utf16String>,
    pub local_or_import_name: Option<Utf16String>,
}

#[derive(Clone, Debug)]
pub struct ExportStatementData {
    pub statement: Option<Box<Statement>>,
    pub entries: Vec<ExportEntry>,
    pub is_default_export: bool,
    pub module_request: Option<ModuleRequest>,
}

// =============================================================================
// For-in/of LHS
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForInOfKind {
    ForIn,
    ForOf,
    ForAwaitOf,
}

/// Init clause of a for loop: either a declaration or an expression.
/// C++ stores this as a polymorphic `RefPtr<ASTNode>` that can be either
/// an Expression or a VariableDeclaration. We use an explicit enum so that
/// expression inits are NOT wrapped in an ExpressionStatement node.
#[derive(Clone, Debug)]
pub enum ForInit {
    Declaration(Box<Statement>),
    Expression(Box<Expression>),
}

/// Left-hand side of for-in, for-of, for-await-of.
#[derive(Clone, Debug)]
pub enum ForInOfLhs {
    /// A variable declaration (`for (let x of ...)`)
    Declaration(Box<Statement>),
    /// An expression (`for (x in obj)`)
    Expression(Box<Expression>),
    /// A binding pattern (`for ({a, b} of ...)`)
    Pattern(BindingPattern),
}

// =============================================================================
// Assignment LHS
// =============================================================================

#[derive(Clone, Debug)]
pub enum AssignmentLhs {
    Expression(Box<Expression>),
    Pattern(BindingPattern),
}

// =============================================================================
// Scope data
// =============================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum LocalVarKind {
    Var = 0,
    LetOrConst = 1,
    Function = 2,
    ArgumentsObject = 3,
    CatchClauseParameter = 4,
}

#[derive(Clone, Debug)]
pub struct LocalVariable {
    pub name: Utf16String,
    pub kind: LocalVarKind,
}

/// Data shared by all scope-bearing nodes (Program, BlockStatement,
/// FunctionBody, SwitchStatement, SwitchCase).
///
/// Wrapped in `Rc<RefCell<...>>` for interior mutability during the
/// scope collector's analysis phase. Borrow safety: the scope
/// collector's two-phase design (build tree during parsing, then
/// analyze bottom-up) ensures borrows never overlap — the analysis
/// phase only borrows one scope at a time in a bottom-up traversal.
#[derive(Clone, Debug, Default)]
pub struct ScopeData {
    pub children: Vec<Statement>,
    pub local_variables: Vec<LocalVariable>,
    pub function_scope_data: Option<Box<FunctionScopeData>>,
    pub hoisted_functions: Vec<usize>,
    /// Function names hoisted from inner blocks via Annex B.3.3.
    /// The FDI should create `var` bindings initialized to `undefined`
    /// for each name.
    pub annexb_function_names: Vec<Utf16String>,
    // Scope analysis insights, written by the scope collector after analyze().
    pub uses_this: bool,
    pub uses_this_from_environment: bool,
    pub contains_direct_call_to_eval: bool,
    pub contains_access_to_arguments_object: bool,
}

impl ScopeData {
    pub fn new_shared() -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self::default()))
    }

    pub fn shared_with_children(children: Vec<Statement>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            children,
            ..Default::default()
        }))
    }
}

/// Scope analysis data for function bodies, populated by the scope collector.
#[derive(Clone, Debug)]
pub struct FunctionScopeData {
    pub functions_to_initialize: Vec<FunctionToInit>,
    pub vars_to_initialize: Vec<VarToInit>,
    pub var_names: Vec<Utf16String>,
    pub has_function_named_arguments: bool,
    pub has_argument_parameter: bool,
    pub has_lexically_declared_arguments: bool,
    pub non_local_var_count: usize,
    pub non_local_var_count_for_parameter_expressions: usize,
}

/// Reference to a function declaration that needs hoisting/initialization.
/// Stores the index within the parent ScopeData.children.
#[derive(Clone, Debug)]
pub struct FunctionToInit {
    pub child_index: usize,
}

/// A resolved local binding: the operand type and index assigned by scope analysis.
#[derive(Clone, Copy, Debug)]
pub struct LocalBinding {
    pub local_type: LocalType,
    pub index: u32,
}

/// A `var` binding that needs initialization during function entry.
#[derive(Clone, Debug)]
pub struct VarToInit {
    pub name: Utf16String,
    pub is_parameter: bool,
    pub is_function_name: bool,
    /// If the scope analysis optimized this var to a local, stores the binding info.
    pub local: Option<LocalBinding>,
}

// =============================================================================
// Expression data structs (boxed variants)
// =============================================================================

#[derive(Clone, Debug)]
pub struct BinaryExprData {
    pub op: BinaryOp,
    pub lhs: Box<Expression>,
    pub rhs: Box<Expression>,
}

#[derive(Clone, Debug)]
pub struct LogicalExprData {
    pub op: LogicalOp,
    pub lhs: Box<Expression>,
    pub rhs: Box<Expression>,
}

#[derive(Clone, Debug)]
pub struct UpdateExprData {
    pub op: UpdateOp,
    pub argument: Box<Expression>,
    pub prefixed: bool,
}

#[derive(Clone, Debug)]
pub struct AssignmentExprData {
    pub op: AssignmentOp,
    pub lhs: AssignmentLhs,
    pub rhs: Box<Expression>,
}

#[derive(Clone, Debug)]
pub struct ConditionalExprData {
    pub test: Box<Expression>,
    pub consequent: Box<Expression>,
    pub alternate: Box<Expression>,
}

#[derive(Clone, Debug)]
pub struct MemberExprData {
    pub object: Box<Expression>,
    pub property: Box<Expression>,
    pub computed: bool,
}

#[derive(Clone, Debug)]
pub struct OptionalChainData {
    pub base: Box<Expression>,
    pub references: Vec<OptionalChainReference>,
}

#[derive(Clone, Debug)]
pub struct TaggedTemplateData {
    pub tag: Box<Expression>,
    pub template_literal: Box<Expression>,
}

#[derive(Clone, Debug)]
pub struct ImportCallData {
    pub specifier: Box<Expression>,
    pub options: Option<Box<Expression>>,
}

#[derive(Clone, Debug)]
pub struct YieldExprData {
    pub argument: Option<Box<Expression>>,
    pub is_yield_from: bool,
}

// =============================================================================
// Expression enum
// =============================================================================

#[derive(Clone, Debug)]
pub enum ExpressionKind {
    // Literals
    NumericLiteral(f64),
    StringLiteral(Box<Utf16String>),
    BooleanLiteral(bool),
    NullLiteral,
    BigIntLiteral(Box<String>),
    RegExpLiteral(Box<RegExpLiteralData>),

    // Identifiers
    Identifier(Rc<Identifier>),
    PrivateIdentifier(Box<PrivateIdentifier>),

    // Operators
    Binary(Box<BinaryExprData>),
    Logical(Box<LogicalExprData>),
    Unary { op: UnaryOp, operand: Box<Expression> },
    Update(Box<UpdateExprData>),
    Assignment(Box<AssignmentExprData>),
    Conditional(Box<ConditionalExprData>),
    Sequence(Box<Vec<Expression>>),

    // Member access
    Member(Box<MemberExprData>),
    OptionalChain(Box<OptionalChainData>),

    // Calls
    Call(Box<CallExpressionData>),
    New(Box<CallExpressionData>),
    SuperCall(Box<SuperCallData>),

    // Spread
    Spread(Box<Expression>),

    // This / Super
    This,
    Super,

    // Functions
    Function(FunctionId),

    // Classes
    Class(Box<ClassData>),

    // Collections
    Array(Box<Vec<Option<Expression>>>),
    Object(Box<Vec<ObjectProperty>>),

    // Templates
    TemplateLiteral(Box<TemplateLiteralData>),
    TaggedTemplateLiteral(Box<TaggedTemplateData>),

    // Meta
    MetaProperty(MetaPropertyType),
    ImportCall(Box<ImportCallData>),

    // Async / Generator
    Yield(Box<YieldExprData>),
    Await(Box<Expression>),

    // Error recovery
    Error,
}

// =============================================================================
// Statement data structs
// =============================================================================

#[derive(Clone, Debug)]
pub struct IfStatementData {
    pub test: Box<Expression>,
    pub consequent: Box<Statement>,
    pub alternate: Option<Box<Statement>>,
}

#[derive(Clone, Debug)]
pub struct WhileStatementData {
    pub test: Box<Expression>,
    pub body: Box<Statement>,
}

#[derive(Clone, Debug)]
pub struct ForStatementData {
    pub init: Option<ForInit>,
    pub test: Option<Box<Expression>>,
    pub update: Option<Box<Expression>>,
    pub body: Box<Statement>,
}

#[derive(Clone, Debug)]
pub struct ForInOfStatementData {
    pub kind: ForInOfKind,
    pub lhs: ForInOfLhs,
    pub rhs: Box<Expression>,
    pub body: Box<Statement>,
}

#[derive(Clone, Debug)]
pub struct WithStatementData {
    pub object: Box<Expression>,
    pub body: Box<Statement>,
}

#[derive(Clone, Debug)]
pub struct LabelledStatementData {
    pub label: Utf16String,
    pub item: Box<Statement>,
}

#[derive(Clone, Debug)]
pub struct VariableDeclarationData {
    pub kind: DeclarationKind,
    pub declarations: Vec<VariableDeclarator>,
}

#[derive(Clone, Debug)]
pub struct FunctionDeclarationData {
    pub function_id: FunctionId,
    pub name: Option<Rc<Identifier>>,
    pub kind: FunctionKind,
    pub is_hoisted: Cell<bool>,
}

#[derive(Clone, Debug)]
pub struct ClassFieldInitializerData {
    pub expression: Box<Expression>,
    pub field_name: Utf16String,
}

// =============================================================================
// Statement enum
// =============================================================================

#[derive(Clone, Debug)]
pub enum StatementKind {
    // Basic
    Empty,
    Error,
    Expression(Box<Expression>),
    Debugger,

    // Blocks (carry ScopeData for scope analysis)
    Block(Rc<RefCell<ScopeData>>),
    FunctionBody {
        scope: Rc<RefCell<ScopeData>>,
        in_strict_mode: bool,
    },
    Program(Box<ProgramData>),

    // Control flow
    If(Box<IfStatementData>),
    While(Box<WhileStatementData>),
    DoWhile(Box<WhileStatementData>),
    For(Box<ForStatementData>),
    ForInOf(Box<ForInOfStatementData>),
    Switch(Box<SwitchStatementData>),
    With(Box<WithStatementData>),
    Labelled(Box<LabelledStatementData>),

    // Jumps
    Break {
        target_label: Option<Utf16String>,
    },
    Continue {
        target_label: Option<Utf16String>,
    },
    Return(Option<Box<Expression>>),
    Throw(Box<Expression>),
    Try(Box<TryStatementData>),

    // Declarations
    VariableDeclaration(Box<VariableDeclarationData>),
    UsingDeclaration(Box<Vec<VariableDeclarator>>),
    FunctionDeclaration(Box<FunctionDeclarationData>),
    ClassDeclaration(Box<ClassData>),
    ErrorDeclaration,

    // Module
    Import(Box<ImportStatementData>),
    Export(Box<ExportStatementData>),

    // Special
    ClassFieldInitializer(Box<ClassFieldInitializerData>),
}

// =============================================================================
// Program data
// =============================================================================

#[derive(Clone, Debug)]
pub struct ProgramData {
    pub scope: Rc<RefCell<ScopeData>>,
    pub program_type: ProgramType,
    pub is_strict_mode: bool,
    pub has_top_level_await: bool,
}
