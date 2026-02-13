/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Scope analysis for the Rust parser.
//!
//! Mirrors the C++ `ScopeCollector` class. This is a two-phase system:
//!
//! ## Phase 1: Build scope tree (during parsing)
//!
//! As the parser encounters scopes (functions, blocks, for-loops, etc.),
//! it calls `open_*_scope()` to push a `ScopeRecord` onto the tree, and
//! the scope is closed when parsing of the construct finishes. During
//! parsing, declarations and identifier references are registered:
//!
//! - `add_var_declaration()` — `var` bindings (hoist to function scope)
//! - `add_lexical_declaration()` — `let`/`const` (block-scoped)
//! - `add_function_declaration()` — function declarations (may Annex-B hoist)
//! - `register_identifier()` — any identifier reference
//!
//! ## Phase 2: Analyze (after parsing)
//!
//! `analyze()` walks the scope tree bottom-up and for each scope:
//!
//! 1. **Resolves identifiers**: matches identifier references to their
//!    declarations (var, let/const, function, parameter, catch binding).
//!    Unresolved identifiers are marked as global.
//!
//! 2. **Propagates eval poisoning**: if a scope contains a direct call
//!    to `eval()`, all ancestor scopes must know (they can't optimize
//!    away their environment records).
//!
//! 3. **Hoists functions (Annex B)**: in non-strict sloppy mode,
//!    function declarations inside blocks can create `var` bindings
//!    in the enclosing function scope.
//!
//! 4. **Builds local variable lists**: populates `FunctionScopeData`
//!    on Rust AST ScopeData nodes, enabling the bytecode generator
//!    to use indexed locals.
//!
//! ## Key data structures
//!
//! - `ScopeRecord` — one scope (function, block, etc.) with its
//!   variables, identifiers, and child scopes
//! - `ScopeVariable` — a declared name within a scope (flags track
//!   whether it's var/lexical/function/catch/parameter)
//! - `IdentifierGroup` — a set of identifier references with the same
//!   name within one scope (multiple `foo` refs are grouped together)

use std::collections::HashMap;

use crate::ast::{
    FunctionScopeData, Identifier, IdentDeclarationKind, LocalVarKind,
    LocalVariable, ScopeData, VarToInit,
};
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
// Bit flags on ScopeVariable that track how a name was declared.
// A single name can accumulate multiple flags (e.g., a `var` that
// shadows a parameter gets both FLAG_IS_VAR and FLAG_IS_FORBIDDEN_LEXICAL).

const FLAG_IS_VAR: u16 = 1 << 0;             // `var` declaration
const FLAG_IS_LEXICAL: u16 = 1 << 1;         // `let` or `const` declaration
const FLAG_IS_FUNCTION: u16 = 1 << 2;        // `function` declaration
const FLAG_IS_CATCH_PARAMETER: u16 = 1 << 3; // `catch (e)` binding
const FLAG_IS_FORBIDDEN_LEXICAL: u16 = 1 << 4; // parameter name that can't be re-declared with let/const
const FLAG_IS_FORBIDDEN_VAR: u16 = 1 << 5;   // lexical name that blocks var hoisting
const FLAG_IS_BOUND: u16 = 1 << 6;           // function expression name or class declaration name
const FLAG_IS_PARAMETER_CANDIDATE: u16 = 1 << 7; // formal parameter name (candidate for local optimization)

// === Data structures ===

/// A declared name within a scope. Multiple declaration forms can share
/// the same name (e.g., `var x` and `function x`), so flags are ORed together.
struct ScopeVariable {
    /// Bit flags describing how this name was declared (FLAG_IS_* constants).
    flags: u16,
    /// The Identifier AST node for the `var` declaration (used to build
    /// FunctionScopeData). Null if not a var.
    var_identifier: *const Identifier,
}

impl Default for ScopeVariable {
    fn default() -> Self {
        Self {
            flags: 0,
            var_identifier: std::ptr::null(),
        }
    }
}

/// Groups all Identifier AST nodes that share the same name within a scope.
/// During analysis, the group is resolved to a local variable, parameter,
/// or propagated to the parent scope if unresolved.
struct IdentifierGroup {
    /// True if any identifier in this group is referenced from a nested
    /// function (prevents local variable optimization).
    captured_by_nested_function: bool,
    /// True if any identifier in this group is inside a `with` block
    /// (prevents local variable optimization since `with` can shadow anything).
    used_inside_with_statement: bool,
    /// All Identifier AST nodes with this name in this scope.
    identifiers: Vec<*const Identifier>,
    /// If this name was declared (var/let/const), tracks the declaration kind
    /// so we can annotate each Identifier AST node.
    declaration_kind: Option<DeclarationKind>,
}

/// A function to hoist, with its name and child index in the ScopeData.
struct HoistableFunction {
    name: Vec<u16>,
    declaration_index: usize,
}

struct ScopeRecord {
    scope_type: ScopeType,
    scope_level: ScopeLevel,
    scope_data: *mut ScopeData,

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
    fn new(scope_type: ScopeType, scope_level: ScopeLevel, scope_data: *mut ScopeData) -> Self {
        Self {
            scope_type,
            scope_level,
            scope_data,
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

    /// Get or create a variable entry, only allocating the key when inserting.
    fn variable(&mut self, name: &[u16]) -> &mut ScopeVariable {
        if !self.variables.contains_key(name) {
            self.variables.insert(name.to_vec(), ScopeVariable::default());
        }
        self.variables.get_mut(name).unwrap()
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

    /// Save scope collector state for speculative parsing.
    pub fn save_state(&self) -> (usize, Option<usize>, usize) {
        (self.records.len(), self.current, self.errors.len())
    }

    /// Restore scope collector state after failed speculative parse.
    pub fn load_state(&mut self, state: (usize, Option<usize>, usize)) {
        let saved_len = state.0;
        self.records.truncate(saved_len);
        self.current = state.1;
        self.errors.truncate(state.2);
        // Remove any child indices that pointed to now-truncated records.
        if let Some(current_idx) = self.current {
            self.records[current_idx].children.retain(|&c| c < saved_len);
        }
    }

    // === Open/close scopes ===

    fn open_scope(&mut self, scope_type: ScopeType, scope_data: *mut ScopeData, scope_level: ScopeLevel) {
        let idx = self.records.len();
        let mut record = ScopeRecord::new(scope_type, scope_level, scope_data);
        record.parent = self.current;

        if scope_type != ScopeType::Function && scope_data.is_null() {
            if let Some(parent_idx) = self.current {
                record.scope_data = self.records[parent_idx].scope_data;
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

    pub fn open_program_scope(&mut self, scope_data: *mut ScopeData, program_type: ProgramType) {
        let level = if program_type == ProgramType::Script {
            ScopeLevel::ScriptTopLevel
        } else {
            ScopeLevel::ModuleTopLevel
        };
        self.open_scope(ScopeType::Program, scope_data, level);
    }

    pub fn open_function_scope(&mut self, function_name: Option<&[u16]>) {
        self.open_scope(ScopeType::Function, std::ptr::null_mut(), ScopeLevel::FunctionTopLevel);
        if let Some(name) = function_name {
            let idx = self.current.unwrap();
            self.records[idx].variable(name).flags |= FLAG_IS_BOUND;
        }
    }

    pub fn open_block_scope(&mut self, scope_data: *mut ScopeData) {
        self.open_scope(ScopeType::Block, scope_data, ScopeLevel::NotTopLevel);
    }

    pub fn open_for_loop_scope(&mut self, scope_data: *mut ScopeData) {
        self.open_scope(ScopeType::ForLoop, scope_data, ScopeLevel::NotTopLevel);
    }

    pub fn open_with_scope(&mut self, scope_data: *mut ScopeData) {
        self.open_scope(ScopeType::With, scope_data, ScopeLevel::NotTopLevel);
    }

    pub fn open_catch_scope(&mut self) {
        self.open_scope(ScopeType::Catch, std::ptr::null_mut(), ScopeLevel::NotTopLevel);
    }

    pub fn open_static_init_scope(&mut self, scope_data: *mut ScopeData) {
        self.open_scope(ScopeType::ClassStaticInit, scope_data, ScopeLevel::StaticInitTopLevel);
    }

    pub fn open_class_field_scope(&mut self, scope_data: *mut ScopeData) {
        self.open_scope(ScopeType::ClassField, scope_data, ScopeLevel::NotTopLevel);
    }

    pub fn open_class_declaration_scope(&mut self, class_name: Option<&[u16]>) {
        self.open_scope(ScopeType::ClassDeclaration, std::ptr::null_mut(), ScopeLevel::NotTopLevel);
        if let Some(name) = class_name {
            let idx = self.current.unwrap();
            self.records[idx].variable(name).flags |= FLAG_IS_BOUND;
        }
    }

    // === Declaration registration ===

    pub fn add_lexical_declaration(
        &mut self,
        bound_names: &[&[u16]],
        decl_line: u32,
        decl_column: u32,
    ) {
        let idx = self.current.unwrap();

        for name in bound_names {
            let var = self.records[idx].variable(name);
            if var.flags & (FLAG_IS_VAR | FLAG_IS_FORBIDDEN_LEXICAL | FLAG_IS_FUNCTION | FLAG_IS_LEXICAL) != 0 {
                self.errors.push(ScopeError {
                    message: format!("Identifier '{}' already declared", String::from_utf16_lossy(name)),
                    line: decl_line,
                    column: decl_column,
                });
            }
            var.flags |= FLAG_IS_LEXICAL;
        }
    }

    pub fn add_var_declaration(
        &mut self,
        bound_names: &[(&[u16], *const Identifier)],
        decl_line: u32,
        decl_column: u32,
    ) {
        let idx = self.current.unwrap();

        for &(name, identifier) in bound_names {
            // Register the declaration identifier so it participates in scope analysis.
            if !identifier.is_null() {
                self.register_identifier(identifier, name, Some(DeclarationKind::Var));
            }

            let mut scope_idx = idx;
            loop {
                let var = self.records[scope_idx].variable(name);
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
    }

    pub fn add_function_declaration(
        &mut self,
        name: &[u16],
        name_identifier: *const Identifier,
        declaration_index: usize,
        function_kind: FunctionKind,
        strict_mode: bool,
        decl_line: u32,
        decl_column: u32,
    ) {
        let idx = self.current.unwrap();
        let scope_level = self.records[idx].scope_level;

        // Register the name identifier so it participates in scope analysis.
        if !name_identifier.is_null() {
            self.register_identifier(name_identifier, name, None);
        }

        if scope_level != ScopeLevel::NotTopLevel && scope_level != ScopeLevel::ModuleTopLevel {
            let var = self.records[idx].variable(name);
            var.flags |= FLAG_IS_VAR;
            var.var_identifier = name_identifier;
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
                self.records[idx].variable(name).flags |= FLAG_IS_LEXICAL;
                return;
            }

            if existing_flags & FLAG_IS_LEXICAL == 0 {
                self.records[idx].functions_to_hoist.push(HoistableFunction {
                    name: name.to_vec(),
                    declaration_index,
                });
            }

            let var = self.records[idx].variable(name);
            var.flags |= FLAG_IS_FUNCTION;
        }
    }

    pub fn add_catch_parameter_pattern(&mut self, bound_names: &[&[u16]]) {
        let idx = self.current.unwrap();
        for name in bound_names {
            let var = self.records[idx].variable(name);
            var.flags |= FLAG_IS_FORBIDDEN_VAR | FLAG_IS_BOUND | FLAG_IS_CATCH_PARAMETER;
        }
    }

    pub fn add_catch_parameter_identifier(&mut self, name: &[u16], identifier: *const Identifier) {
        let idx = self.current.unwrap();
        let var = self.records[idx].variable(name);
        var.flags |= FLAG_IS_VAR | FLAG_IS_BOUND | FLAG_IS_CATCH_PARAMETER;
        var.var_identifier = identifier;
    }

    // === Identifier registration ===

    pub fn register_identifier(&mut self, id: *const Identifier, name: &[u16], declaration_kind: Option<DeclarationKind>) {
        let idx = self.current.unwrap();
        if let Some(group) = self.records[idx].identifier_groups.get_mut(name) {
            group.identifiers.push(id);
        } else {
            self.records[idx].identifier_groups.insert(name.to_vec(), IdentifierGroup {
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
        entries: &[(Vec<u16>, *const Identifier, bool, bool)],
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
            if !identifier.is_null() {
                self.register_identifier(*identifier, name, None);
            }
            let var = self.records[idx].variables.entry(name.clone()).or_default();
            var.flags |= FLAG_IS_PARAMETER_CANDIDATE | FLAG_IS_FORBIDDEN_LEXICAL;
        }
    }

    // === Scope node ===

    pub fn set_scope_node(&mut self, scope_data: *mut ScopeData) {
        let idx = self.current.unwrap();
        self.records[idx].scope_data = scope_data;
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

    /// Analyze a scope and all its descendants, bottom-up.
    /// Children are analyzed first so that unresolved identifiers bubble up
    /// to their parent, and eval poisoning propagates outward.
    fn analyze_recursive(&mut self, idx: usize, initiated_by_eval: bool) {
        // Process children first (bottom-up traversal).
        let children = std::mem::take(&mut self.records[idx].children);
        for child_idx in children {
            self.analyze_recursive(child_idx, initiated_by_eval);
        }

        // Steps 1-3 must run even for scopes without scope_data (e.g. catch
        // scopes), so that identifier groups propagate through the scope chain.
        // Without this, captured variables inside catch blocks are invisible
        // to enclosing scopes and get incorrectly optimized as locals.

        // 1. Propagate eval() flags from children to parent.
        Self::propagate_eval_poisoning(&mut self.records, idx);
        // 2. Match identifier references to declarations; optimize as locals.
        Self::resolve_identifiers(&mut self.records, idx, initiated_by_eval);
        // 3. Annex B: hoist block-scoped functions to enclosing function scope.
        Self::hoist_functions(&mut self.records, idx);

        // 4. For function scopes, build the var declaration list that the
        //    bytecode generator uses to initialize function-scoped variables.
        if !self.records[idx].scope_data.is_null()
            && self.records[idx].scope_type == ScopeType::Function
            && self.records[idx].has_function_parameters
        {
            Self::build_function_scope_data(&self.records, idx);
        }
    }

    /// Propagate eval-related flags from a child scope to its parent.
    ///
    /// Three separate flags track eval impact:
    /// - `contains_direct_call_to_eval`: this scope itself has `eval()`
    /// - `screwed_by_eval_in_scope_chain`: some descendant has eval, so
    ///   this scope can't optimize away its environment record
    /// - `eval_in_current_function`: eval exists somewhere in the current
    ///   function (propagates through blocks but stops at function boundaries)
    fn propagate_eval_poisoning(records: &mut [ScopeRecord], idx: usize) {
        if let Some(parent_idx) = records[idx].parent {
            if records[idx].contains_direct_call_to_eval || records[idx].screwed_by_eval_in_scope_chain {
                records[parent_idx].screwed_by_eval_in_scope_chain = true;
            }
            // eval_in_current_function propagates upward through blocks but
            // stops at function boundaries (each function is independent).
            if records[idx].eval_in_current_function && records[idx].scope_type != ScopeType::Function {
                records[parent_idx].eval_in_current_function = true;
            }
        }
    }

    /// Try to resolve each identifier group in this scope to a local variable.
    ///
    /// For each named group, this function:
    /// 1. Annotates identifiers with their declaration kind (var/let/const)
    /// 2. Determines if the name can be optimized to a local variable index
    /// 3. If not resolvable here, propagates the group to the parent scope
    ///
    /// An identifier is optimized to a local when:
    /// - It's declared in this scope (var, let/const, function, parameter, catch)
    /// - It's NOT captured by a nested function
    /// - It's NOT used inside a `with` statement
    /// - The scope chain is NOT poisoned by `eval()`
    fn resolve_identifiers(records: &mut [ScopeRecord], idx: usize, initiated_by_eval: bool) {
        let groups = std::mem::take(&mut records[idx].identifier_groups);
        let mut propagate_to_parent: Vec<(Vec<u16>, IdentifierGroup)> = Vec::new();
        for (name, mut group) in groups {
            // Annotate each Identifier AST node with its declaration kind,
            // so the bytecode generator knows how to handle TDZ checks, etc.
            if let Some(dk) = group.declaration_kind {
                let kind = match dk {
                    DeclarationKind::Var => IdentDeclarationKind::Var,
                    DeclarationKind::Let => IdentDeclarationKind::Let,
                    DeclarationKind::Const => IdentDeclarationKind::Const,
                };
                for &id in &group.identifiers {
                    unsafe { (*id).declaration_kind.set(kind) };
                }
            }

            let var_flags = records[idx].variables.get(&name).map_or(0, |v| v.flags);

            // Determine what kind of local variable this is (if any).
            // Priority: var (at top-level) > let/const > function declaration.
            let mut local_var_kind: Option<LocalVarKind> = None;
            if records[idx].is_top_level() && (var_flags & FLAG_IS_VAR) != 0 {
                local_var_kind = Some(LocalVarKind::Var);
            } else if (var_flags & FLAG_IS_LEXICAL) != 0 {
                local_var_kind = Some(LocalVarKind::LetOrConst);
            } else if (var_flags & FLAG_IS_FUNCTION) != 0 {
                local_var_kind = Some(LocalVarKind::Function);
            }

            // Non-arrow functions implicitly declare `arguments` as a local.
            // Arrow functions inherit `arguments` from their enclosing function.
            if records[idx].scope_type == ScopeType::Function
                && !records[idx].is_arrow_function
                && name == utf16!("arguments")
            {
                local_var_kind = Some(LocalVarKind::ArgumentsObject);
            }

            if records[idx].scope_type == ScopeType::Catch
                && (var_flags & FLAG_IS_CATCH_PARAMETER) != 0
            {
                // Catch parameters are handled by the catch codegen, not as
                // local variables. Skip this group entirely so it doesn't
                // get optimized to a local or propagated further.
                continue;
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
                    unsafe { (*id).is_inside_scope_with_eval.set(true) };
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
                        let is_eval_scope = unsafe { (*id).is_inside_scope_with_eval.get() };
                        if !is_eval_scope {
                            unsafe { (*id).is_global.set(true) };
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
                        let scope_data = records[ls].scope_data;

                        if is_function_parameter {
                            let arg_index = records[ls].get_parameter_index(&name);
                            if let Some(ai) = arg_index {
                                for &id in &group.identifiers {
                                    unsafe {
                                        (*id).local_index.set(ai);
                                        (*id).local_type.set(crate::ast::LocalType::Argument);
                                    }
                                }
                            } else {
                                let lvi = unsafe {
                                    let sd = &mut *scope_data;
                                    let index = sd.local_variables.len() as u32;
                                    sd.local_variables.push(LocalVariable {
                                        name: name.clone(),
                                        kind: LocalVarKind::Var,
                                    });
                                    index
                                };
                                for &id in &group.identifiers {
                                    unsafe {
                                        (*id).local_index.set(lvi);
                                        (*id).local_type.set(crate::ast::LocalType::Variable);
                                    }
                                }
                            }
                        } else {
                            let kind = local_var_kind.unwrap();
                            let lvi = unsafe {
                                let sd = &mut *scope_data;
                                let index = sd.local_variables.len() as u32;
                                sd.local_variables.push(LocalVariable {
                                    name: name.clone(),
                                    kind,
                                });
                                index
                            };
                            for &id in &group.identifiers {
                                unsafe {
                                    (*id).local_index.set(lvi);
                                    (*id).local_type.set(crate::ast::LocalType::Variable);
                                }
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
                        unsafe { (*id).is_inside_scope_with_eval.set(true) };
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
        let scope_data = record.scope_data;
        if scope_data.is_null() {
            return;
        }

        let has_argument_parameter = record.variables.get(utf16!("arguments") as &[u16])
            .is_some_and(|v| v.flags & FLAG_IS_FORBIDDEN_LEXICAL != 0);

        // Collect IS_VAR variables for FunctionScopeData.
        let mut vars_to_initialize = Vec::new();
        let mut var_names = Vec::new();
        let mut has_function_named_arguments = false;
        let mut has_lexically_declared_arguments = false;
        let mut non_local_var_count: usize = 0;

        // Build functions_to_initialize by scanning children for FunctionDeclarations.
        // Walk in reverse order, deduplicating by name (like C++ ensure_function_scope_data).
        let mut functions_to_initialize: Vec<crate::ast::FunctionToInit> = Vec::new();
        let mut seen_function_names: Vec<Vec<u16>> = Vec::new();
        unsafe {
            let children = &(*scope_data).children;
            for i in (0..children.len()).rev() {
                if let crate::ast::Statement::FunctionDeclaration(ref func_data) = children[i].inner {
                    if let Some(ref name_ident) = func_data.name {
                        if !seen_function_names.contains(&name_ident.name) {
                            seen_function_names.push(name_ident.name.clone());
                            functions_to_initialize.push(crate::ast::FunctionToInit {
                                child_index: i,
                            });
                        }
                    }
                }
            }
        }

        for (name, var) in &record.variables {
            if var.flags & FLAG_IS_VAR == 0 {
                continue;
            }

            var_names.push(name.clone());

            let is_parameter = var.flags & FLAG_IS_FORBIDDEN_LEXICAL != 0;
            let is_function_name = var.flags & FLAG_IS_BOUND != 0;

            // Check if this var has been optimized to a local
            let local_info = if !var.var_identifier.is_null() {
                let ident = unsafe { &*var.var_identifier };
                if ident.is_local() {
                    Some((ident.local_type.get(), ident.local_index.get()))
                } else {
                    None
                }
            } else {
                None
            };

            if local_info.is_none() {
                non_local_var_count += 1;
            }

            vars_to_initialize.push(VarToInit {
                name: name.clone(),
                is_parameter,
                is_function_name,
                local: local_info,
            });

        }

        // Check if any function declaration is named "arguments".
        if seen_function_names.iter().any(|n| n == utf16!("arguments")) {
            has_function_named_arguments = true;
        }

        // Check for lexically declared arguments
        if record.variables.get(utf16!("arguments") as &[u16])
            .is_some_and(|v| v.flags & FLAG_IS_LEXICAL != 0)
        {
            has_lexically_declared_arguments = true;
        }

        let fsd = FunctionScopeData {
            functions_to_initialize,
            vars_to_initialize,
            var_names,
            has_function_named_arguments,
            has_argument_parameter,
            has_lexically_declared_arguments,
            non_local_var_count,
            non_local_var_count_for_parameter_expressions: 0,
        };

        unsafe {
            (*scope_data).function_scope_data = Some(Box::new(fsd));

            // Write scope analysis insights to ScopeData so they can be read
            // during lazy compilation (write_sfd_metadata, FDI emission).
            (*scope_data).uses_this = record.uses_this;
            (*scope_data).uses_this_from_environment = record.uses_this_from_environment;
            (*scope_data).contains_direct_call_to_eval = record.contains_direct_call_to_eval
                || record.screwed_by_eval_in_scope_chain;
            (*scope_data).contains_access_to_arguments_object =
                record.contains_access_to_arguments_object_in_non_strict_mode;
        }
    }

    /// Annex B function hoisting: in sloppy mode, function declarations inside
    /// blocks can create `var` bindings in the enclosing function scope.
    ///
    /// For example:
    /// ```js
    /// function f() {
    ///     if (true) { function g() {} }  // g is hoisted to f's scope
    ///     g(); // works in sloppy mode!
    /// }
    /// ```
    ///
    /// The function propagates upward through block scopes until it reaches
    /// a function/program scope (top level) or is blocked by an existing
    /// lexical or function declaration with the same name.
    fn hoist_functions(records: &mut [ScopeRecord], idx: usize) {
        let functions = std::mem::take(&mut records[idx].functions_to_hoist);

        for func in functions {
            // A let/const or forbidden var with the same name blocks hoisting.
            if records[idx].has_flag(&func.name, FLAG_IS_LEXICAL | FLAG_IS_FORBIDDEN_VAR) {
                continue;
            }

            if records[idx].is_top_level() {
                // Reached function/program scope — register the hoisted function.
                let scope_data = records[idx].scope_data;
                if !scope_data.is_null() {
                    unsafe {
                        (*scope_data).hoisted_functions.push(func.declaration_index);
                    }
                }
            } else if let Some(parent_idx) = records[idx].parent {
                // Not yet at top level — keep propagating upward unless blocked.
                if !records[parent_idx].has_flag(&func.name, FLAG_IS_LEXICAL | FLAG_IS_FUNCTION) {
                    records[parent_idx].functions_to_hoist.push(func);
                }
            }
        }
    }
}
