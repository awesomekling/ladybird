/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Scope analysis for the Rust parser.
//!
//! Mirrors the C++ ScopeCollector: builds a tree of scope records during
//! parsing, then runs bottom-up analysis (resolve identifiers, hoist
//! functions) after parsing completes.

use std::collections::HashMap;

use crate::ast_bridge::{self, NodeHandle, NULL_HANDLE};
use crate::parser::{DeclarationKind, FunctionKind, ProgramType};

// === Enums ===

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeType {
    Function,
    Program,
    Block,
    ForLoop,
    With,
    Catch,
    ClassStaticInit,
    ClassField,
    ClassDeclaration,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScopeLevel {
    NotTopLevel,
    ScriptTopLevel,
    ModuleTopLevel,
    FunctionTopLevel,
    StaticInitTopLevel,
}

impl ScopeLevel {
    fn is_top_level(self) -> bool {
        self != ScopeLevel::NotTopLevel
    }
}

// === Variable flags ===

const FLAG_IS_VAR: u16 = 1 << 0;
const FLAG_IS_LEXICAL: u16 = 1 << 1;
const FLAG_IS_FUNCTION: u16 = 1 << 2;
const FLAG_IS_CATCH_PARAMETER: u16 = 1 << 3;
const FLAG_IS_FORBIDDEN_LEXICAL: u16 = 1 << 4;
const FLAG_IS_FORBIDDEN_VAR: u16 = 1 << 5;
const FLAG_IS_BOUND: u16 = 1 << 6;
const FLAG_IS_PARAMETER_CANDIDATE: u16 = 1 << 7;

// === LocalVariable::DeclarationKind (for add_local_variable FFI) ===

const LOCAL_VAR: u8 = 0;
const LOCAL_LET_OR_CONST: u8 = 1;
const LOCAL_FUNCTION: u8 = 2;
const LOCAL_ARGUMENTS_OBJECT: u8 = 3;
const LOCAL_CATCH_CLAUSE_PARAMETER: u8 = 4;

// === C++ DeclarationKind (for set_declaration_kind FFI) ===

const DECL_KIND_VAR: u8 = 1;
const DECL_KIND_LET: u8 = 2;
const DECL_KIND_CONST: u8 = 3;

// === Data structures ===

struct ScopeVariable {
    flags: u16,
    var_identifier: NodeHandle,
    function_declaration: NodeHandle,
}

impl Default for ScopeVariable {
    fn default() -> Self {
        Self {
            flags: 0,
            var_identifier: NULL_HANDLE,
            function_declaration: NULL_HANDLE,
        }
    }
}

struct IdentifierGroup {
    captured_by_nested_function: bool,
    used_inside_with_statement: bool,
    identifiers: Vec<NodeHandle>,
    declaration_kind: Option<DeclarationKind>,
}

/// A function to hoist, with its name.
struct HoistableFunction {
    name: Vec<u16>,
    declaration: NodeHandle,
}

struct ScopeRecord {
    scope_type: ScopeType,
    scope_level: ScopeLevel,
    ast_node: NodeHandle,

    variables: HashMap<Vec<u16>, ScopeVariable>,
    identifier_groups: HashMap<Vec<u16>, IdentifierGroup>,
    functions_to_hoist: Vec<HoistableFunction>,

    // Parameter tracking
    has_function_parameters: bool,
    parameter_names: Vec<(Vec<u16>, bool)>, // (name, is_rest)

    // Flags
    contains_access_to_arguments_object_in_non_strict_mode: bool,
    contains_direct_call_to_eval: bool,
    contains_await_expression: bool,
    screwed_by_eval_in_scope_chain: bool,
    eval_in_current_function: bool,
    uses_this_from_environment: bool,
    uses_this: bool,
    is_arrow_function: bool,
    is_function_declaration: bool,

    // Tree (indices into ScopeCollector::records)
    parent: Option<usize>,
    top_level: Option<usize>,
    children: Vec<usize>,
}

impl ScopeRecord {
    fn new(scope_type: ScopeType, scope_level: ScopeLevel, ast_node: NodeHandle) -> Self {
        Self {
            scope_type,
            scope_level,
            ast_node,
            variables: HashMap::new(),
            identifier_groups: HashMap::new(),
            functions_to_hoist: Vec::new(),
            has_function_parameters: false,
            parameter_names: Vec::new(),
            contains_access_to_arguments_object_in_non_strict_mode: false,
            contains_direct_call_to_eval: false,
            contains_await_expression: false,
            screwed_by_eval_in_scope_chain: false,
            eval_in_current_function: false,
            uses_this_from_environment: false,
            uses_this: false,
            is_arrow_function: false,
            is_function_declaration: false,
            parent: None,
            top_level: None,
            children: Vec::new(),
        }
    }

    fn is_top_level(&self) -> bool {
        self.scope_level.is_top_level()
    }

    fn has_flag(&self, name: &[u16], flags: u16) -> bool {
        self.variables.get(name).is_some_and(|v| v.flags & flags != 0)
    }

    fn get_parameter_index(&self, name: &[u16]) -> Option<u32> {
        // Iterate backwards to return the last parameter with the same name,
        // matching the semantics of duplicate parameter names in non-strict mode.
        for (i, (pname, _is_rest)) in self.parameter_names.iter().enumerate().rev() {
            if pname == name {
                return Some(i as u32);
            }
        }
        None
    }

    fn has_rest_parameter_with_name(&self, name: &[u16]) -> bool {
        self.parameter_names.iter().any(|(pname, is_rest)| *is_rest && pname == name)
    }

    fn has_hoistable_function_named(&self, name: &[u16]) -> bool {
        self.functions_to_hoist.iter().any(|f| f.name == name)
    }
}

fn last_function_scope(idx: usize, records: &[ScopeRecord]) -> Option<usize> {
    let mut current = Some(idx);
    while let Some(i) = current {
        let scope_type = records[i].scope_type;
        if scope_type == ScopeType::Function || scope_type == ScopeType::ClassStaticInit {
            return Some(i);
        }
        current = records[i].parent;
    }
    None
}

// === ScopeCollector ===

pub struct ScopeError {
    pub message: String,
    pub line: u32,
    pub column: u32,
}

pub struct ScopeCollector {
    records: Vec<ScopeRecord>,
    current: Option<usize>,
    errors: Vec<ScopeError>,
}

impl ScopeCollector {
    pub fn new() -> Self {
        Self {
            records: Vec::new(),
            current: None,
            errors: Vec::new(),
        }
    }

    pub fn drain_errors(&mut self) -> Vec<ScopeError> {
        std::mem::take(&mut self.errors)
    }

    pub fn has_current_scope(&self) -> bool {
        self.current.is_some()
    }

    // === Open/close scopes ===

    fn open_scope(&mut self, scope_type: ScopeType, ast_node: NodeHandle, scope_level: ScopeLevel) {
        let idx = self.records.len();
        let mut record = ScopeRecord::new(scope_type, scope_level, ast_node);
        record.parent = self.current;

        if scope_type != ScopeType::Function && ast_node == NULL_HANDLE {
            if let Some(parent_idx) = self.current {
                record.ast_node = self.records[parent_idx].ast_node;
            }
        }

        if scope_level == ScopeLevel::NotTopLevel {
            if let Some(parent_idx) = self.current {
                record.top_level = self.records[parent_idx].top_level;
            }
        } else {
            record.top_level = Some(idx);
        }

        self.records.push(record);
        if let Some(parent_idx) = self.current {
            self.records[parent_idx].children.push(idx);
        }
        self.current = Some(idx);
    }

    pub fn close_scope(&mut self) {
        let idx = self.current.expect("close_scope with no current scope");

        if let Some(parent_idx) = self.records[idx].parent {
            if !self.records[idx].has_function_parameters {
                let c = &self.records[idx];
                let args = c.contains_access_to_arguments_object_in_non_strict_mode;
                let eval = c.contains_direct_call_to_eval;
                let aw = c.contains_await_expression;
                self.records[parent_idx].contains_access_to_arguments_object_in_non_strict_mode |= args;
                self.records[parent_idx].contains_direct_call_to_eval |= eval;
                self.records[parent_idx].contains_await_expression |= aw;
            }
        }

        self.current = self.records[idx].parent;
    }

    pub fn open_program_scope(&mut self, program: NodeHandle, program_type: ProgramType) {
        let level = if program_type == ProgramType::Script {
            ScopeLevel::ScriptTopLevel
        } else {
            ScopeLevel::ModuleTopLevel
        };
        self.open_scope(ScopeType::Program, program, level);
    }

    pub fn open_function_scope(&mut self, function_name: Option<&[u16]>) {
        self.open_scope(ScopeType::Function, NULL_HANDLE, ScopeLevel::FunctionTopLevel);
        if let Some(name) = function_name {
            let idx = self.current.unwrap();
            self.records[idx].variables.entry(name.to_vec()).or_default().flags |= FLAG_IS_BOUND;
        }
    }

    pub fn open_block_scope(&mut self, node: NodeHandle) {
        self.open_scope(ScopeType::Block, node, ScopeLevel::NotTopLevel);
    }

    pub fn open_for_loop_scope(&mut self, node: NodeHandle) {
        self.open_scope(ScopeType::ForLoop, node, ScopeLevel::NotTopLevel);
    }

    pub fn open_with_scope(&mut self, node: NodeHandle) {
        self.open_scope(ScopeType::With, node, ScopeLevel::NotTopLevel);
    }

    pub fn open_catch_scope(&mut self) {
        self.open_scope(ScopeType::Catch, NULL_HANDLE, ScopeLevel::NotTopLevel);
    }

    pub fn open_static_init_scope(&mut self, node: NodeHandle) {
        self.open_scope(ScopeType::ClassStaticInit, node, ScopeLevel::StaticInitTopLevel);
    }

    pub fn open_class_field_scope(&mut self, node: NodeHandle) {
        self.open_scope(ScopeType::ClassField, node, ScopeLevel::NotTopLevel);
    }

    pub fn open_class_declaration_scope(&mut self, class_name: Option<&[u16]>) {
        self.open_scope(ScopeType::ClassDeclaration, NULL_HANDLE, ScopeLevel::NotTopLevel);
        if let Some(name) = class_name {
            let idx = self.current.unwrap();
            self.records[idx].variables.entry(name.to_vec()).or_default().flags |= FLAG_IS_BOUND;
        }
    }

    // === Declaration registration ===

    pub fn add_lexical_declaration(
        &mut self,
        declaration: NodeHandle,
        bound_names: &[&[u16]],
        decl_line: u32,
        decl_column: u32,
    ) {
        let idx = self.current.unwrap();

        for name in bound_names {
            let var = self.records[idx].variables.entry(name.to_vec()).or_default();
            if var.flags & (FLAG_IS_VAR | FLAG_IS_FORBIDDEN_LEXICAL | FLAG_IS_FUNCTION | FLAG_IS_LEXICAL) != 0 {
                self.errors.push(ScopeError {
                    message: format!("Identifier '{}' already declared", String::from_utf16_lossy(name)),
                    line: decl_line,
                    column: decl_column,
                });
            }
            var.flags |= FLAG_IS_LEXICAL;
        }

        let ast_node = self.records[idx].ast_node;
        unsafe { ast_bridge::ast_scope_node_add_lexical_declaration(ast_node, declaration) };
    }

    pub fn add_var_declaration(
        &mut self,
        declaration: NodeHandle,
        bound_names: &[(&[u16], NodeHandle)],
        decl_line: u32,
        decl_column: u32,
    ) {
        let idx = self.current.unwrap();

        for &(name, identifier) in bound_names {
            // Register the declaration identifier so it participates in scope analysis.
            if identifier != NULL_HANDLE {
                self.register_identifier(identifier, name, Some(DeclarationKind::Var));
            }

            let mut scope_idx = idx;
            loop {
                let var = self.records[scope_idx].variables.entry(name.to_vec()).or_default();
                if var.flags & (FLAG_IS_LEXICAL | FLAG_IS_FUNCTION | FLAG_IS_FORBIDDEN_VAR) != 0 {
                    self.errors.push(ScopeError {
                        message: format!("Identifier '{}' already declared", String::from_utf16_lossy(name)),
                        line: decl_line,
                        column: decl_column,
                    });
                }
                var.flags |= FLAG_IS_VAR;
                var.var_identifier = identifier;
                if self.records[scope_idx].is_top_level() {
                    break;
                }
                scope_idx = self.records[scope_idx].parent.unwrap();
            }
        }

        // Register declaration on top-level scope node once.
        if let Some(top_idx) = self.records[idx].top_level {
            let top_ast_node = self.records[top_idx].ast_node;
            unsafe { ast_bridge::ast_scope_node_add_var_scoped_declaration(top_ast_node, declaration) };
        }
    }

    pub fn add_function_declaration(
        &mut self,
        declaration: NodeHandle,
        name: &[u16],
        name_identifier: NodeHandle,
        function_kind: FunctionKind,
        strict_mode: bool,
        decl_line: u32,
        decl_column: u32,
    ) {
        let idx = self.current.unwrap();
        let scope_level = self.records[idx].scope_level;

        // Register the name identifier so it participates in scope analysis.
        if name_identifier != NULL_HANDLE {
            self.register_identifier(name_identifier, name, None);
        }

        if scope_level != ScopeLevel::NotTopLevel && scope_level != ScopeLevel::ModuleTopLevel {
            let var = self.records[idx].variables.entry(name.to_vec()).or_default();
            var.flags |= FLAG_IS_VAR;
            var.var_identifier = name_identifier;

            let ast_node = self.records[idx].ast_node;
            unsafe { ast_bridge::ast_scope_node_add_var_scoped_declaration(ast_node, declaration) };
        } else {
            // Check flags first, then modify. This avoids borrow checker issues
            // since we need to access both variables and functions_to_hoist.
            let existing_flags = self.records[idx].variables.get(name).map_or(0, |v| v.flags);

            if existing_flags & (FLAG_IS_VAR | FLAG_IS_LEXICAL) != 0 {
                self.errors.push(ScopeError {
                    message: format!("Identifier '{}' already declared", String::from_utf16_lossy(name)),
                    line: decl_line,
                    column: decl_column,
                });
            }

            if function_kind != FunctionKind::Normal || strict_mode {
                if existing_flags & FLAG_IS_FUNCTION != 0 {
                    self.errors.push(ScopeError {
                        message: format!("Identifier '{}' already declared", String::from_utf16_lossy(name)),
                        line: decl_line,
                        column: decl_column,
                    });
                }
                self.records[idx].variables.entry(name.to_vec()).or_default().flags |= FLAG_IS_LEXICAL;
                let ast_node = self.records[idx].ast_node;
                unsafe { ast_bridge::ast_scope_node_add_lexical_declaration(ast_node, declaration) };
                return;
            }

            if existing_flags & FLAG_IS_LEXICAL == 0 {
                self.records[idx].functions_to_hoist.push(HoistableFunction {
                    name: name.to_vec(),
                    declaration,
                });
            }

            let var = self.records[idx].variables.entry(name.to_vec()).or_default();
            var.flags |= FLAG_IS_FUNCTION;
            var.function_declaration = declaration;

            let ast_node = self.records[idx].ast_node;
            unsafe { ast_bridge::ast_scope_node_add_lexical_declaration(ast_node, declaration) };
        }
    }

    pub fn add_catch_parameter_pattern(&mut self, bound_names: &[&[u16]]) {
        let idx = self.current.unwrap();
        for name in bound_names {
            let var = self.records[idx].variables.entry(name.to_vec()).or_default();
            var.flags |= FLAG_IS_FORBIDDEN_VAR | FLAG_IS_BOUND | FLAG_IS_CATCH_PARAMETER;
        }
    }

    pub fn add_catch_parameter_identifier(&mut self, name: &[u16], identifier: NodeHandle) {
        let idx = self.current.unwrap();
        let var = self.records[idx].variables.entry(name.to_vec()).or_default();
        var.flags |= FLAG_IS_VAR | FLAG_IS_BOUND | FLAG_IS_CATCH_PARAMETER;
        var.var_identifier = identifier;
    }

    // === Identifier registration ===

    pub fn register_identifier(&mut self, id: NodeHandle, name: &[u16], declaration_kind: Option<DeclarationKind>) {
        let idx = self.current.unwrap();
        let name_vec = name.to_vec();
        if let Some(group) = self.records[idx].identifier_groups.get_mut(&name_vec) {
            group.identifiers.push(id);
        } else {
            self.records[idx].identifier_groups.insert(name_vec, IdentifierGroup {
                captured_by_nested_function: false,
                used_inside_with_statement: false,
                identifiers: vec![id],
                declaration_kind,
            });
        }
    }

    // === Function parameters ===

    pub fn set_function_parameters(
        &mut self,
        entries: &[(Vec<u16>, NodeHandle, bool, bool)],
    ) {
        let idx = self.current.unwrap();
        self.records[idx].has_function_parameters = true;

        let mut prev_was_pattern = false;
        for (name, identifier, is_rest, is_from_pattern) in entries {
            if *is_from_pattern {
                if !prev_was_pattern {
                    // First bound name from a pattern parameter — push one
                    // empty placeholder so subsequent non-pattern parameters
                    // get the correct positional index.
                    self.records[idx].parameter_names.push((Vec::new(), false));
                }
                prev_was_pattern = true;
            } else {
                self.records[idx].parameter_names.push((name.clone(), *is_rest));
                prev_was_pattern = false;
            }
            self.register_identifier(*identifier, name, None);
            let var = self.records[idx].variables.entry(name.clone()).or_default();
            var.flags |= FLAG_IS_PARAMETER_CANDIDATE | FLAG_IS_FORBIDDEN_LEXICAL;
        }
    }

    // === Scope node ===

    pub fn set_scope_node(&mut self, node: NodeHandle) {
        let idx = self.current.unwrap();
        self.records[idx].ast_node = node;
    }

    // === Flag setters ===

    pub fn set_contains_direct_call_to_eval(&mut self) {
        let idx = self.current.unwrap();
        self.records[idx].contains_direct_call_to_eval = true;
        self.records[idx].screwed_by_eval_in_scope_chain = true;
        self.records[idx].eval_in_current_function = true;
    }

    pub fn set_contains_access_to_arguments_object_in_non_strict_mode(&mut self) {
        let idx = self.current.unwrap();
        self.records[idx].contains_access_to_arguments_object_in_non_strict_mode = true;
    }

    pub fn set_contains_await_expression(&mut self) {
        let idx = self.current.unwrap();
        self.records[idx].contains_await_expression = true;
    }

    pub fn set_uses_this(&mut self) {
        let idx = self.current.unwrap();
        let closest_fn = last_function_scope(idx, &self.records);
        let this_from_env = closest_fn.is_some_and(|fi| self.records[fi].is_arrow_function);

        let mut scope_idx = Some(idx);
        while let Some(si) = scope_idx {
            if self.records[si].scope_type == ScopeType::Function {
                self.records[si].uses_this = true;
                if this_from_env {
                    self.records[si].uses_this_from_environment = true;
                }
            }
            scope_idx = self.records[si].parent;
        }
    }

    pub fn set_uses_new_target(&mut self) {
        let idx = self.current.unwrap();
        let mut scope_idx = Some(idx);
        while let Some(si) = scope_idx {
            if self.records[si].scope_type == ScopeType::Function {
                self.records[si].uses_this = true;
                self.records[si].uses_this_from_environment = true;
            }
            scope_idx = self.records[si].parent;
        }
    }

    pub fn set_is_arrow_function(&mut self) {
        let idx = self.current.unwrap();
        self.records[idx].is_arrow_function = true;
    }

    pub fn set_is_function_declaration(&mut self) {
        let idx = self.current.unwrap();
        self.records[idx].is_function_declaration = true;
    }

    // === Getters ===

    pub fn contains_direct_call_to_eval(&self) -> bool {
        self.current.is_some_and(|idx| self.records[idx].contains_direct_call_to_eval)
    }

    pub fn uses_this_from_environment(&self) -> bool {
        self.current.is_some_and(|idx| self.records[idx].uses_this_from_environment)
    }

    pub fn uses_this(&self) -> bool {
        self.current.is_some_and(|idx| self.records[idx].uses_this)
    }

    pub fn contains_await_expression(&self) -> bool {
        self.current.is_some_and(|idx| self.records[idx].contains_await_expression)
    }

    pub fn scope_type(&self) -> Option<ScopeType> {
        self.current.map(|idx| self.records[idx].scope_type)
    }

    pub fn can_have_using_declaration(&self) -> bool {
        self.current.is_some_and(|idx| self.records[idx].scope_level != ScopeLevel::ScriptTopLevel)
    }

    pub fn has_declaration(&self, name: &[u16]) -> bool {
        if let Some(idx) = self.current {
            if self.records[idx].has_flag(name, FLAG_IS_LEXICAL | FLAG_IS_VAR) {
                return true;
            }
            return self.records[idx].has_hoistable_function_named(name);
        }
        false
    }

    pub fn has_declaration_in_current_function(&self, name: &[u16]) -> bool {
        if let Some(idx) = self.current {
            let fn_scope = last_function_scope(idx, &self.records);
            let stop = fn_scope.and_then(|fi| self.records[fi].parent);
            let mut scope_idx = Some(idx);
            while scope_idx != stop {
                if let Some(si) = scope_idx {
                    if self.records[si].has_flag(name, FLAG_IS_LEXICAL | FLAG_IS_VAR | FLAG_IS_PARAMETER_CANDIDATE) {
                        return true;
                    }
                    if self.records[si].has_hoistable_function_named(name) {
                        return true;
                    }
                    scope_idx = self.records[si].parent;
                } else {
                    break;
                }
            }
        }
        false
    }

    // === Post-parse analysis ===

    pub fn analyze(&mut self, initiated_by_eval: bool) {
        if !self.records.is_empty() {
            self.analyze_recursive(0, initiated_by_eval);
        }
    }

    fn analyze_recursive(&mut self, idx: usize, initiated_by_eval: bool) {
        let children: Vec<usize> = self.records[idx].children.clone();
        for child_idx in children {
            self.analyze_recursive(child_idx, initiated_by_eval);
        }

        if self.records[idx].ast_node == NULL_HANDLE {
            return;
        }

        Self::propagate_eval_poisoning(&mut self.records, idx);
        Self::resolve_identifiers(&mut self.records, idx, initiated_by_eval);
        Self::hoist_functions(&mut self.records, idx);

        if self.records[idx].scope_type == ScopeType::Function && self.records[idx].has_function_parameters {
            Self::build_function_scope_data(&self.records, idx);
        }
    }

    fn propagate_eval_poisoning(records: &mut [ScopeRecord], idx: usize) {
        if let Some(parent_idx) = records[idx].parent {
            if records[idx].contains_direct_call_to_eval || records[idx].screwed_by_eval_in_scope_chain {
                records[parent_idx].screwed_by_eval_in_scope_chain = true;
            }
            if records[idx].eval_in_current_function && records[idx].scope_type != ScopeType::Function {
                records[parent_idx].eval_in_current_function = true;
            }
        }
    }

    fn resolve_identifiers(records: &mut [ScopeRecord], idx: usize, initiated_by_eval: bool) {
        let groups = std::mem::take(&mut records[idx].identifier_groups);
        let mut propagate_to_parent: Vec<(Vec<u16>, IdentifierGroup)> = Vec::new();
        let arguments_name: Vec<u16> = "arguments".encode_utf16().collect();

        for (name, mut group) in groups {
            // Set declaration_kind on all identifiers in the group.
            if let Some(dk) = group.declaration_kind {
                let ffi_kind = match dk {
                    DeclarationKind::Var => DECL_KIND_VAR,
                    DeclarationKind::Let => DECL_KIND_LET,
                    DeclarationKind::Const => DECL_KIND_CONST,
                };
                for &id in &group.identifiers {
                    unsafe { ast_bridge::ast_identifier_set_declaration_kind(id, ffi_kind) };
                }
            }

            let var_flags = records[idx].variables.get(&name).map_or(0, |v| v.flags);

            // Determine local variable declaration kind.
            let mut local_var_kind: Option<u8> = None;
            if records[idx].is_top_level() && (var_flags & FLAG_IS_VAR) != 0 {
                local_var_kind = Some(LOCAL_VAR);
            } else if (var_flags & FLAG_IS_LEXICAL) != 0 {
                local_var_kind = Some(LOCAL_LET_OR_CONST);
            } else if (var_flags & FLAG_IS_FUNCTION) != 0 {
                local_var_kind = Some(LOCAL_FUNCTION);
            }

            if records[idx].scope_type == ScopeType::Function
                && !records[idx].is_arrow_function
                && name == arguments_name
            {
                local_var_kind = Some(LOCAL_ARGUMENTS_OBJECT);
            }

            if records[idx].scope_type == ScopeType::Catch
                && (var_flags & FLAG_IS_CATCH_PARAMETER) != 0
            {
                local_var_kind = Some(LOCAL_CATCH_CLAUSE_PARAMETER);
            }

            let hoistable = records[idx].has_hoistable_function_named(&name);

            // ClassDeclaration with IsBound: skip entirely.
            if records[idx].scope_type == ScopeType::ClassDeclaration
                && (var_flags & FLAG_IS_BOUND) != 0
            {
                continue;
            }

            // Function expression name binding.
            if records[idx].scope_type == ScopeType::Function
                && !records[idx].is_function_declaration
                && (var_flags & FLAG_IS_BOUND) != 0
            {
                for &id in &group.identifiers {
                    unsafe { ast_bridge::ast_identifier_set_is_inside_scope_with_eval(id) };
                }
            }

            if records[idx].scope_type == ScopeType::ClassDeclaration {
                local_var_kind = None;
            }

            // Function parameter handling.
            let mut is_function_parameter = false;
            if records[idx].scope_type == ScopeType::Function {
                if (var_flags & FLAG_IS_PARAMETER_CANDIDATE) != 0
                    && (!records[idx].contains_access_to_arguments_object_in_non_strict_mode
                        || records[idx].has_rest_parameter_with_name(&name))
                {
                    is_function_parameter = true;
                } else if (var_flags & FLAG_IS_FORBIDDEN_LEXICAL) != 0 {
                    continue;
                }
            }

            if records[idx].scope_type == ScopeType::Function && hoistable {
                continue;
            }

            if records[idx].scope_type == ScopeType::Program {
                let can_use_global = !(group.used_inside_with_statement || initiated_by_eval);
                if can_use_global {
                    for &id in &group.identifiers {
                        let is_eval_scope = unsafe { ast_bridge::ast_identifier_is_inside_scope_with_eval(id) };
                        if !is_eval_scope {
                            unsafe { ast_bridge::ast_identifier_set_is_global(id) };
                        }
                    }
                }
            } else if local_var_kind.is_some() || is_function_parameter {
                if hoistable {
                    continue;
                }

                if !group.captured_by_nested_function && !group.used_inside_with_statement {
                    if records[idx].screwed_by_eval_in_scope_chain {
                        continue;
                    }

                    let mut local_scope = last_function_scope(idx, records);
                    if local_scope.is_none() {
                        if group.declaration_kind == Some(DeclarationKind::Var) {
                            continue;
                        }
                        local_scope = records[idx].top_level;
                    }

                    if let Some(ls) = local_scope {
                        let scope_ast_node = records[ls].ast_node;

                        if is_function_parameter {
                            let arg_index = records[ls].get_parameter_index(&name);
                            if let Some(ai) = arg_index {
                                for &id in &group.identifiers {
                                    unsafe { ast_bridge::ast_identifier_set_argument_index(id, ai) };
                                }
                            } else {
                                let lvi = unsafe {
                                    ast_bridge::ast_scope_node_add_local_variable(
                                        scope_ast_node, name.as_ptr(), name.len(), LOCAL_VAR,
                                    )
                                };
                                for &id in &group.identifiers {
                                    unsafe { ast_bridge::ast_identifier_set_local_variable_index(id, lvi) };
                                }
                            }
                        } else {
                            let kind = local_var_kind.unwrap();
                            let lvi = unsafe {
                                ast_bridge::ast_scope_node_add_local_variable(
                                    scope_ast_node, name.as_ptr(), name.len(), kind,
                                )
                            };
                            for &id in &group.identifiers {
                                unsafe { ast_bridge::ast_identifier_set_local_variable_index(id, lvi) };
                            }
                        }
                    }
                }
            } else {
                // Not resolved here: propagate to parent.
                if records[idx].has_function_parameters
                    || records[idx].scope_type == ScopeType::ClassField
                    || records[idx].scope_type == ScopeType::ClassStaticInit
                {
                    group.captured_by_nested_function = true;
                }

                if records[idx].scope_type == ScopeType::With {
                    group.used_inside_with_statement = true;
                }

                if records[idx].eval_in_current_function {
                    for &id in &group.identifiers {
                        unsafe { ast_bridge::ast_identifier_set_is_inside_scope_with_eval(id) };
                    }
                }

                propagate_to_parent.push((name, group));
            }
        }

        if let Some(parent_idx) = records[idx].parent {
            for (name, group) in propagate_to_parent {
                if let Some(parent_group) = records[parent_idx].identifier_groups.get_mut(&name) {
                    parent_group.identifiers.extend(group.identifiers);
                    if group.captured_by_nested_function {
                        parent_group.captured_by_nested_function = true;
                    }
                    if group.used_inside_with_statement {
                        parent_group.used_inside_with_statement = true;
                    }
                } else {
                    records[parent_idx].identifier_groups.insert(name, group);
                }
            }
        }
    }

    fn build_function_scope_data(records: &[ScopeRecord], idx: usize) {
        let record = &records[idx];
        let scope_node = record.ast_node;
        if scope_node == NULL_HANDLE {
            return;
        }

        let arguments_name: Vec<u16> = "arguments".encode_utf16().collect();
        let has_argument_parameter = record.variables.get(&arguments_name)
            .is_some_and(|v| v.flags & FLAG_IS_FORBIDDEN_LEXICAL != 0);

        // Collect IS_VAR variables.
        let mut names_data: Vec<u16> = Vec::new();
        let mut name_offsets: Vec<u32> = Vec::new();
        let mut name_lengths: Vec<u32> = Vec::new();
        let mut identifiers: Vec<NodeHandle> = Vec::new();
        let mut is_parameter: Vec<u8> = Vec::new();

        for (name, var) in &record.variables {
            if var.flags & FLAG_IS_VAR == 0 {
                continue;
            }
            if var.var_identifier == NULL_HANDLE {
                continue;
            }
            name_offsets.push(names_data.len() as u32);
            name_lengths.push(name.len() as u32);
            names_data.extend_from_slice(name);
            identifiers.push(var.var_identifier);
            is_parameter.push(if var.flags & FLAG_IS_FORBIDDEN_LEXICAL != 0 { 1 } else { 0 });
        }

        unsafe {
            ast_bridge::ast_scope_build_function_scope_data(
                scope_node,
                names_data.as_ptr(),
                name_offsets.as_ptr(),
                name_lengths.as_ptr(),
                identifiers.as_ptr(),
                is_parameter.as_ptr(),
                identifiers.len(),
                if has_argument_parameter { 1 } else { 0 },
            );
        }
    }

    fn hoist_functions(records: &mut [ScopeRecord], idx: usize) {
        let functions = std::mem::take(&mut records[idx].functions_to_hoist);

        for func in functions {
            if records[idx].has_flag(&func.name, FLAG_IS_LEXICAL | FLAG_IS_FORBIDDEN_VAR) {
                continue;
            }

            if records[idx].is_top_level() {
                let ast_node = records[idx].ast_node;
                unsafe { ast_bridge::ast_scope_node_add_hoisted_function(ast_node, func.declaration) };
            } else if let Some(parent_idx) = records[idx].parent {
                if !records[parent_idx].has_flag(&func.name, FLAG_IS_LEXICAL | FLAG_IS_FUNCTION) {
                    records[parent_idx].functions_to_hoist.push(func);
                }
            }
        }
    }
}
