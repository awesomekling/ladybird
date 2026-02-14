/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! # LibJS Rust Parser
//!
//! A JavaScript parser written in Rust that produces a Rust AST.
//!
//! ## Architecture
//!
//! ```text
//! Source code (UTF-16)
//!     │
//!     ▼
//! ┌─────────────────────────────────────────────────────┐
//! │  Lexer (lexer.rs)                                   │
//! │  Tokenizes UTF-16 source into Token stream          │
//! └──────────────────────┬──────────────────────────────┘
//!                        │ tokens
//!                        ▼
//! ┌─────────────────────────────────────────────────────┐
//! │  Parser (parser.rs + parser/*.rs)                   │
//! │  Recursive descent with precedence climbing         │
//! │  Builds Rust AST (ast.rs)                           │
//! └──────────────────────┬──────────────────────────────┘
//!                        │ Rust AST
//!                        ▼
//! ┌─────────────────────────────────────────────────────┐
//! │  Codegen (bytecode/codegen.rs)                      │
//! │  Walks AST, emits bytecode via Generator            │
//! └──────────────────────┬──────────────────────────────┘
//!                        │ assembled bytecode
//!                        ▼
//! ┌─────────────────────────────────────────────────────┐
//! │  FFI (bytecode/ffi.rs → BytecodeFactory.cpp)        │
//! │  Creates C++ Executable from assembled data         │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module overview
//!
//! - `lib.rs` — Entry point, called from C++
//! - `token.rs` — Token types matching the C++ `TokenType` enum
//! - `lexer.rs` — Tokenizer: UTF-16 input → Token stream
//! - `parser.rs` — Parser state, helpers, token consumption
//! - `parser/expressions.rs` — Expression parsing (precedence climbing)
//! - `parser/statements.rs` — Statement parsing (if, for, while, etc.)
//! - `parser/declarations.rs` — Functions, classes, variables, modules
//! - `ast.rs` — Rust AST type definitions
//! - `bytecode/` — Bytecode generator, instruction types, and FFI
//! - `scope_collector.rs` — Scope analysis

/// Compile-time conversion of an ASCII string literal to `&'static [u16]`.
///
/// Replaces the old `utf16_lit()` function which allocated a `Vec<u16>` on
/// every call. This macro produces a static array, so comparisons like
/// `value == utf16!("eval")` involve zero heap allocation.
///
/// # Panics (at compile time)
/// Panics if the string contains non-ASCII characters. All JS keywords
/// and identifiers we compare against are pure ASCII.
macro_rules! utf16 {
    ($s:literal) => {{
        const VALUE: &[u16; $s.len()] = &{
            let bytes = $s.as_bytes();
            let mut arr = [0u16; $s.len()];
            let mut i = 0;
            while i < bytes.len() {
                assert!(bytes[i] < 128, "utf16! only supports ASCII literals");
                arr[i] = bytes[i] as u16;
                i += 1;
            }
            arr
        };
        VALUE.as_slice()
    }};
}

pub mod ast;
pub mod bytecode;
pub mod lexer;
pub mod parser;
pub mod scope_collector;
pub mod token;

use ast::Statement;
use parser::{Parser, ProgramType};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

/// Shared compilation pipeline: local variable setup → codegen → assemble → create Executable.
///
/// Called by all three program-level entry points after parsing and scope analysis.
unsafe fn compile_program_body(
    gen: &mut bytecode::generator::Generator,
    program: &ast::Stmt,
    scope_ref: &Rc<RefCell<ast::ScopeData>>,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
) -> *mut c_void {
    // Copy local variables from scope analysis into the generator.
    {
        let scope = scope_ref.borrow();
        gen.local_variables = scope
            .local_variables
            .iter()
            .map(|lv| bytecode::generator::LocalVariable {
                name: lv.name.clone(),
                is_lexically_declared: lv.kind == ast::LocalVarKind::LetOrConst,
                is_initialized_during_declaration_instantiation: false,
            })
            .collect();
    }

    let entry_block = gen.make_block();
    gen.switch_to_basic_block(entry_block);

    // Initialize the lexical environment register.
    {
        use bytecode::operand::{Operand, Register};
        let env_reg = gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT));
        gen.emit(bytecode::instruction::Instruction::GetLexicalEnvironment {
            dst: env_reg.operand(),
        });
        gen.lexical_environment_register_stack.push(env_reg);
    }

    let result = bytecode::codegen::generate_stmt(program, gen, None);

    if !gen.is_current_block_terminated() {
        let value = result.unwrap_or_else(|| gen.add_constant_undefined());
        gen.emit(bytecode::instruction::Instruction::End {
            value: value.operand(),
        });
    }

    let assembled = gen.assemble();
    bytecode::ffi::create_executable(gen, &assembled, vm_ptr, source_code_ptr)
}

/// Compile a JavaScript program using the Rust parser and bytecode generator.
///
/// This is the full Rust pipeline: parse → codegen → assemble → create Executable.
/// Called from C++ when `USE_RUST_CODEGEN=1` is set.
///
/// Returns a `GC::Ptr<Bytecode::Executable>` cast to `void*`, or nullptr on failure.
///
/// # Safety
/// - `source` must point to a valid UTF-16 buffer of `source_len` elements.
/// - `vm_ptr` must be a valid `JS::VM*`.
/// - `source_code_ptr` must be a valid `JS::SourceCode const*`.
#[no_mangle]
pub unsafe extern "C" fn rust_compile_program(
    source: *const u16,
    source_len: usize,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    program_type: u8,
    starts_in_strict_mode: bool,
    initiated_by_eval: bool,
    in_eval_function_context: bool,
    allow_super_property_lookup: bool,
    allow_super_constructor_call: bool,
    in_class_field_initializer: bool,
) -> *mut c_void {
    let source_slice = std::slice::from_raw_parts(source, source_len);
    let pt = if program_type == 1 {
        ProgramType::Module
    } else {
        ProgramType::Script
    };
    let mut parser = Parser::new(source_slice, pt);
    if initiated_by_eval {
        parser.initiated_by_eval = true;
        parser.in_eval_function_context = in_eval_function_context;
        parser.flags.allow_super_property_lookup = allow_super_property_lookup;
        parser.flags.allow_super_constructor_call = allow_super_constructor_call;
        parser.flags.in_class_field_initializer = in_class_field_initializer;
    }

    // Parse
    let program = parser.parse_program(starts_in_strict_mode);

    // Check for parse errors
    if parser.has_errors() {
        for msg in parser.error_messages() {
            eprintln!("[rust_compile_program] parse error: {}", msg);
        }
        return std::ptr::null_mut();
    }

    // Run scope analysis
    parser.scope_collector.analyze(initiated_by_eval);

    if parser.scope_collector.has_errors() {
        for err in parser.scope_collector.drain_errors() {
            eprintln!("[rust_compile_program] scope error: {}", err.message);
        }
        return std::ptr::null_mut();
    }

    let scope_ref = if let Statement::Program(ref data) = program.inner {
        data.scope.clone()
    } else {
        return std::ptr::null_mut();
    };

    let mut gen = bytecode::generator::Generator::new();
    gen.strict = starts_in_strict_mode;
    gen.must_propagate_completion = true;
    gen.vm_ptr = vm_ptr;
    gen.source_code_ptr = source_code_ptr;
    gen.source = source;
    gen.source_len = source_len;

    compile_program_body(&mut gen, &program, &scope_ref, vm_ptr, source_code_ptr)
}

/// Compile a script and extract GDI (GlobalDeclarationInstantiation) metadata.
///
/// This is the Rust-only path for scripts. It:
/// 1. Parses the program
/// 2. Runs scope analysis
/// 3. Generates bytecode → creates Executable
/// 4. Extracts GDI metadata from the program AST
/// 5. Populates the C++ ScriptGdiBuilder via callbacks
///
/// Returns the `Executable*` as `void*`, or nullptr on failure.
///
/// # Safety
/// - `source` must point to a valid UTF-16 buffer of `source_len` elements.
/// - `vm_ptr` must be a valid `JS::VM*`.
/// - `source_code_ptr` must be a valid `JS::SourceCode const*`.
/// - `gdi_context` must be a valid pointer to a C++ ScriptGdiBuilder.
#[no_mangle]
pub unsafe extern "C" fn rust_compile_script(
    source: *const u16,
    source_len: usize,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    gdi_context: *mut c_void,
) -> *mut c_void {
    let source_slice = std::slice::from_raw_parts(source, source_len);
    let mut parser = Parser::new(source_slice, ProgramType::Script);

    // Parse
    let program = parser.parse_program(false);

    // Check for parse errors
    if parser.has_errors() {
        return std::ptr::null_mut();
    }

    // Run scope analysis
    parser.scope_collector.analyze(false);

    if parser.scope_collector.has_errors() {
        return std::ptr::null_mut();
    }

    let (scope_ref, is_strict) = if let Statement::Program(ref data) = program.inner {
        (data.scope.clone(), data.is_strict_mode)
    } else {
        return std::ptr::null_mut();
    };

    let mut gen = bytecode::generator::Generator::new();
    gen.strict = is_strict;
    gen.must_propagate_completion = true;
    gen.vm_ptr = vm_ptr;
    gen.source_code_ptr = source_code_ptr;
    gen.source = source;
    gen.source_len = source_len;

    let exec_ptr = compile_program_body(&mut gen, &program, &scope_ref, vm_ptr, source_code_ptr);
    if exec_ptr.is_null() {
        return std::ptr::null_mut();
    }

    // Extract GDI metadata and populate via callbacks.
    extract_script_gdi(&scope_ref.borrow(), is_strict, vm_ptr, source_code_ptr, gdi_context);

    exec_ptr
}

/// Compile an eval script and extract EDI (EvalDeclarationInstantiation) metadata.
///
/// This is the Rust-only path for eval(). It:
/// 1. Parses the program with eval flags
/// 2. Runs scope analysis with initiated_by_eval=true
/// 3. Generates bytecode → creates Executable
/// 4. Extracts EDI metadata from the program AST
/// 5. Populates the C++ EvalGdiBuilder via callbacks
///
/// Returns the `Executable*` as `void*`, or nullptr on failure.
///
/// # Safety
/// - `source` must point to a valid UTF-16 buffer of `source_len` elements.
/// - `vm_ptr` must be a valid `JS::VM*`.
/// - `source_code_ptr` must be a valid `JS::SourceCode const*`.
/// - `gdi_context` must be a valid pointer to a C++ EvalGdiBuilder.
#[no_mangle]
pub unsafe extern "C" fn rust_compile_eval(
    source: *const u16,
    source_len: usize,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    gdi_context: *mut c_void,
    starts_in_strict_mode: bool,
    in_eval_function_context: bool,
    allow_super_property_lookup: bool,
    allow_super_constructor_call: bool,
    in_class_field_initializer: bool,
) -> *mut c_void {
    let source_slice = std::slice::from_raw_parts(source, source_len);
    let mut parser = Parser::new(source_slice, ProgramType::Script);
    parser.initiated_by_eval = true;
    parser.in_eval_function_context = in_eval_function_context;
    parser.flags.allow_super_property_lookup = allow_super_property_lookup;
    parser.flags.allow_super_constructor_call = allow_super_constructor_call;
    parser.flags.in_class_field_initializer = in_class_field_initializer;

    // Parse
    let program = parser.parse_program(starts_in_strict_mode);

    // Check for parse errors
    if parser.has_errors() {
        return std::ptr::null_mut();
    }

    // Run scope analysis
    parser.scope_collector.analyze(true);

    if parser.scope_collector.has_errors() {
        return std::ptr::null_mut();
    }

    let (scope_ref, is_strict) = if let Statement::Program(ref data) = program.inner {
        (data.scope.clone(), data.is_strict_mode)
    } else {
        return std::ptr::null_mut();
    };

    let mut gen = bytecode::generator::Generator::new();
    gen.strict = is_strict;
    gen.must_propagate_completion = true;
    gen.vm_ptr = vm_ptr;
    gen.source_code_ptr = source_code_ptr;
    gen.source = source;
    gen.source_len = source_len;

    let exec_ptr = compile_program_body(&mut gen, &program, &scope_ref, vm_ptr, source_code_ptr);
    if exec_ptr.is_null() {
        return std::ptr::null_mut();
    }

    // Extract EDI metadata and populate via callbacks.
    extract_eval_gdi(&scope_ref.borrow(), is_strict, vm_ptr, source_code_ptr, gdi_context);

    exec_ptr
}

/// Compile a dynamically-created function (new Function()).
///
/// Validates parameters and body separately per spec, then parses
/// the full synthetic source to create a SharedFunctionInstanceData.
///
/// Returns a `SharedFunctionInstanceData*` as `void*`, or nullptr on
/// parse failure.
///
/// # Safety
/// - All source pointers must be valid UTF-16 buffers.
/// - `vm_ptr` must be a valid `JS::VM*`.
/// - `source_code_ptr` must be a valid `JS::SourceCode const*`.
#[no_mangle]
pub unsafe extern "C" fn rust_compile_dynamic_function(
    full_source: *const u16,
    full_source_len: usize,
    params_source: *const u16,
    params_source_len: usize,
    body_source: *const u16,
    body_source_len: usize,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    function_kind: u8,
) -> *mut c_void {
    // Validate parameters: wrap in the appropriate function kind and parse.
    {
        let mut validate_src: Vec<u16> = Vec::new();
        match function_kind {
            1 => validate_src.extend_from_slice(utf16!("function* test(")),
            2 => validate_src.extend_from_slice(utf16!("async function test(")),
            3 => validate_src.extend_from_slice(utf16!("async function* test(")),
            _ => validate_src.extend_from_slice(utf16!("function test(")),
        }
        let params_slice = std::slice::from_raw_parts(params_source, params_source_len);
        validate_src.extend_from_slice(params_slice);
        validate_src.extend_from_slice(utf16!(") {}"));
        let mut parser = Parser::new(&validate_src, ProgramType::Script);
        parser.parse_program(false);
        if parser.has_errors() {
            return std::ptr::null_mut();
        }
    }

    // Validate body: wrap in "function test() { BODY }" and parse.
    {
        let mut validate_src: Vec<u16> = Vec::new();
        match function_kind {
            1 => validate_src.extend_from_slice(utf16!("function* test() {")),
            2 => validate_src.extend_from_slice(utf16!("async function test() {")),
            3 => validate_src.extend_from_slice(utf16!("async function* test() {")),
            _ => validate_src.extend_from_slice(utf16!("function test() {")),
        }
        let body_slice = std::slice::from_raw_parts(body_source, body_source_len);
        validate_src.extend_from_slice(body_slice);
        validate_src.extend_from_slice(utf16!("}"));
        let mut parser = Parser::new(&validate_src, ProgramType::Script);
        parser.parse_program(false);
        if parser.has_errors() {
            return std::ptr::null_mut();
        }
    }

    // Parse the full source.
    let full_slice = std::slice::from_raw_parts(full_source, full_source_len);
    let mut parser = Parser::new(full_slice, ProgramType::Script);
    let program = parser.parse_program(false);

    if parser.has_errors() {
        return std::ptr::null_mut();
    }

    // Run scope analysis.
    parser.scope_collector.analyze(false);

    if parser.scope_collector.has_errors() {
        return std::ptr::null_mut();
    }

    // Extract the FunctionExpression from the program.
    // The program should contain a single ExpressionStatement wrapping a FunctionExpression.
    let func_data = if let Statement::Program(ref data) = program.inner {
        let scope = data.scope.borrow();
        let mut found: Option<ast::FunctionData> = None;
        for child in &scope.children {
            match &child.inner {
                Statement::FunctionDeclaration(fd) => {
                    found = Some(fd.as_ref().clone());
                    break;
                }
                Statement::Expression(expr) => {
                    if let ast::Expression::Function(fd) = &expr.inner {
                        found = Some(fd.as_ref().clone());
                        break;
                    }
                }
                _ => {}
            }
        }
        found
    } else {
        None
    };

    let func_data = match func_data {
        Some(fd) => fd,
        None => return std::ptr::null_mut(),
    };

    // Determine strictness: the function itself may be strict.
    let is_strict = func_data.is_strict_mode;

    // Create SFD via FFI.
    bytecode::ffi::create_sfd_for_gdi(&func_data, vm_ptr, source_code_ptr, is_strict)
}

/// Recursively collect var-declared names from a statement and all nested
/// statements, excluding function/class bodies (which create new var scopes).
/// Calls `push_name` for each discovered name.
unsafe fn collect_var_names_recursive(
    stmt: &ast::Statement,
    ctx: *mut c_void,
    push_name: unsafe extern "C" fn(*mut c_void, *const u16, usize),
) {
    match stmt {
        ast::Statement::VariableDeclaration {
            kind: ast::DeclarationKind::Var,
            declarations,
        } => {
            for decl in declarations {
                let mut names = Vec::new();
                collect_bound_names_from_target(&decl.target, &mut names);
                for name in &names {
                    push_name(ctx, name.as_ptr(), name.len());
                }
            }
        }
        _ => {
            for_each_child_statement(stmt, &mut |child| {
                collect_var_names_recursive(child, ctx, push_name);
            });
        }
    }
}

/// Collect var names + function declaration names, deduplicated function
/// initializations, var-scoped names, annex B names, and lexical bindings.
///
/// Shared by both script and eval GDI extraction.
unsafe fn extract_gdi_common(
    scope: &ast::ScopeData,
    is_strict: bool,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    ctx: *mut c_void,
    push_var_name: unsafe extern "C" fn(*mut c_void, *const u16, usize),
    push_function: unsafe extern "C" fn(*mut c_void, *mut c_void, *const u16, usize),
    push_var_scoped_name: unsafe extern "C" fn(*mut c_void, *const u16, usize),
    push_annex_b_name: unsafe extern "C" fn(*mut c_void, *const u16, usize),
    push_lexical_binding: unsafe extern "C" fn(*mut c_void, *const u16, usize, bool),
) {
    use ast::{DeclarationKind, Statement};

    // Var names (var declarations at any nesting level + top-level function declarations)
    for child in &scope.children {
        collect_var_names_recursive(&child.inner, ctx, push_var_name);
        if let Statement::FunctionDeclaration(func_data) = &child.inner {
            if let Some(ref name) = func_data.name {
                push_var_name(ctx, name.name.as_ptr(), name.name.len());
            }
        }
    }

    // Functions to initialize (reverse order, deduplicated by name).
    let mut seen_names: Vec<Vec<u16>> = Vec::new();
    let mut funcs_to_init: Vec<(&ast::FunctionData, Vec<u16>)> = Vec::new();
    for child in scope.children.iter().rev() {
        if let Statement::FunctionDeclaration(func_data) = &child.inner {
            if let Some(ref name_ident) = func_data.name {
                if !seen_names.iter().any(|n| *n == name_ident.name) {
                    seen_names.push(name_ident.name.clone());
                    funcs_to_init.push((func_data, name_ident.name.clone()));
                }
            }
        }
    }
    for (func_data, name) in &funcs_to_init {
        let sfd_ptr = bytecode::ffi::create_sfd_for_gdi(func_data, vm_ptr, source_code_ptr, is_strict);
        push_function(ctx, sfd_ptr, name.as_ptr(), name.len());
    }

    // Var-scoped names (var VariableDeclaration names, excluding function declarations)
    for child in &scope.children {
        collect_var_names_recursive(&child.inner, ctx, push_var_scoped_name);
    }

    // Annex B candidate names
    for name in &scope.annexb_function_names {
        push_annex_b_name(ctx, name.as_ptr(), name.len());
    }

    // Lexical bindings (name + is_constant)
    for child in &scope.children {
        match &child.inner {
            Statement::VariableDeclaration { kind, declarations }
                if *kind != DeclarationKind::Var =>
            {
                let is_constant = *kind == DeclarationKind::Const;
                for decl in declarations {
                    let mut names = Vec::new();
                    collect_bound_names_from_target(&decl.target, &mut names);
                    for name in &names {
                        push_lexical_binding(ctx, name.as_ptr(), name.len(), is_constant);
                    }
                }
            }
            Statement::UsingDeclaration { declarations } => {
                for decl in declarations {
                    let mut names = Vec::new();
                    collect_bound_names_from_target(&decl.target, &mut names);
                    for name in &names {
                        push_lexical_binding(ctx, name.as_ptr(), name.len(), false);
                    }
                }
            }
            Statement::ClassDeclaration(class_data) => {
                if let Some(ref name) = class_data.name {
                    push_lexical_binding(ctx, name.name.as_ptr(), name.name.len(), false);
                }
            }
            _ => {}
        }
    }
}

/// Extract EDI metadata from a program-level ScopeData and populate
/// the C++ EvalGdiBuilder via callbacks.
unsafe fn extract_eval_gdi(
    scope: &ast::ScopeData,
    is_strict: bool,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    ctx: *mut c_void,
) {
    use bytecode::ffi::{
        eval_gdi_push_annex_b_name, eval_gdi_push_function, eval_gdi_push_lexical_binding,
        eval_gdi_push_var_name, eval_gdi_push_var_scoped_name, eval_gdi_set_strict,
    };

    eval_gdi_set_strict(ctx, is_strict);

    extract_gdi_common(
        scope, is_strict, vm_ptr, source_code_ptr, ctx,
        eval_gdi_push_var_name,
        eval_gdi_push_function,
        eval_gdi_push_var_scoped_name,
        eval_gdi_push_annex_b_name,
        eval_gdi_push_lexical_binding,
    );
}

/// Extract GDI metadata from a program-level ScopeData and populate
/// the C++ ScriptGdiBuilder via callbacks.
unsafe fn extract_script_gdi(
    scope: &ast::ScopeData,
    is_strict: bool,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    ctx: *mut c_void,
) {
    use ast::{DeclarationKind, Statement};
    use bytecode::ffi::{
        script_gdi_push_annex_b_name, script_gdi_push_function, script_gdi_push_lexical_binding,
        script_gdi_push_lexical_name, script_gdi_push_var_name, script_gdi_push_var_scoped_name,
    };

    // Lexical names (let/const/using/class at top level) — script-only step.
    for child in &scope.children {
        match &child.inner {
            Statement::VariableDeclaration { kind, declarations }
                if *kind != DeclarationKind::Var =>
            {
                for decl in declarations {
                    let mut names = Vec::new();
                    collect_bound_names_from_target(&decl.target, &mut names);
                    for name in &names {
                        script_gdi_push_lexical_name(ctx, name.as_ptr(), name.len());
                    }
                }
            }
            Statement::UsingDeclaration { declarations } => {
                for decl in declarations {
                    let mut names = Vec::new();
                    collect_bound_names_from_target(&decl.target, &mut names);
                    for name in &names {
                        script_gdi_push_lexical_name(ctx, name.as_ptr(), name.len());
                    }
                }
            }
            Statement::ClassDeclaration(class_data) => {
                if let Some(ref name) = class_data.name {
                    script_gdi_push_lexical_name(ctx, name.name.as_ptr(), name.name.len());
                }
            }
            _ => {}
        }
    }

    extract_gdi_common(
        scope, is_strict, vm_ptr, source_code_ptr, ctx,
        script_gdi_push_var_name,
        script_gdi_push_function,
        script_gdi_push_var_scoped_name,
        script_gdi_push_annex_b_name,
        script_gdi_push_lexical_binding,
    );
}

/// Visit each child statement of a statement, excluding function/class bodies
/// (which create new var scopes). This enables recursive var-declaration walking.
fn for_each_child_statement(stmt: &ast::Statement, f: &mut dyn FnMut(&ast::Statement)) {
    use ast::Statement;

    match stmt {
        Statement::Block(scope) => {
            for child in &scope.borrow().children {
                f(&child.inner);
            }
        }
        Statement::If { consequent, alternate, .. } => {
            f(&consequent.inner);
            if let Some(alt) = alternate {
                f(&alt.inner);
            }
        }
        Statement::While { body, .. }
        | Statement::DoWhile { body, .. }
        | Statement::With { body, .. } => {
            f(&body.inner);
        }
        Statement::For { init, body, .. } => {
            if let Some(init) = init {
                f(&init.inner);
            }
            f(&body.inner);
        }
        Statement::ForIn { lhs, body, .. }
        | Statement::ForOf { lhs, body, .. }
        | Statement::ForAwaitOf { lhs, body, .. } => {
            if let ast::ForInOfLhs::Declaration(decl) = lhs {
                f(&decl.inner);
            }
            f(&body.inner);
        }
        Statement::Switch(data) => {
            for case in &data.cases {
                for child in &case.scope.borrow().children {
                    f(&child.inner);
                }
            }
        }
        Statement::Try(data) => {
            f(&data.block.inner);
            if let Some(ref handler) = data.handler {
                f(&handler.body.inner);
            }
            if let Some(ref finalizer) = data.finalizer {
                f(&finalizer.inner);
            }
        }
        Statement::Labelled { item, .. } => {
            f(&item.inner);
        }
        // Don't recurse into function/class bodies (new var scopes)
        _ => {}
    }
}

/// Collect bound names from a variable declarator target.
fn collect_bound_names_from_target(target: &ast::VariableDeclaratorTarget, names: &mut Vec<Vec<u16>>) {
    match target {
        ast::VariableDeclaratorTarget::Identifier(id) => {
            names.push(id.name.clone());
        }
        ast::VariableDeclaratorTarget::BindingPattern(pattern) => {
            collect_bound_names_from_pattern(pattern, names);
        }
    }
}

/// Collect bound names from a binding pattern (recursively).
fn collect_bound_names_from_pattern(pattern: &ast::BindingPattern, names: &mut Vec<Vec<u16>>) {
    for entry in &pattern.entries {
        match &entry.alias {
            ast::BindingEntryAlias::Empty => {
                if let ast::BindingEntryName::Identifier(id) = &entry.name {
                    names.push(id.name.clone());
                }
            }
            ast::BindingEntryAlias::Identifier(id) => {
                names.push(id.name.clone());
            }
            ast::BindingEntryAlias::BindingPattern(inner) => {
                collect_bound_names_from_pattern(inner, names);
            }
            ast::BindingEntryAlias::MemberExpression(_) => {}
        }
    }
}

/// Free a Rust `Box<FunctionData>` stored in a C++ SharedFunctionInstanceData.
///
/// Called from the SFD's `finalize()` or `clear_compile_inputs()` when the
/// Rust AST is no longer needed.
///
/// # Safety
/// `ast` must be a valid pointer returned by `Box::into_raw(Box<FunctionData>)`.
#[no_mangle]
pub unsafe extern "C" fn rust_free_function_ast(ast: *mut c_void) {
    if !ast.is_null() {
        drop(Box::from_raw(ast as *mut ast::FunctionData));
    }
}

/// Compile a function body using the Rust pipeline.
///
/// Takes ownership of the Rust `Box<FunctionData>` and compiles it into a
/// C++ `Bytecode::Executable`. Also populates FDI runtime metadata on the
/// `SharedFunctionInstanceData`.
///
/// # Safety
/// - `vm_ptr` must be a valid `JS::VM*`.
/// - `source_code_ptr` must be a valid `JS::SourceCode const*`.
/// - `sfd_ptr` must be a valid `JS::SharedFunctionInstanceData*`.
/// - `rust_function_ast` must be a valid `Box<FunctionData>` pointer.
#[no_mangle]
pub unsafe extern "C" fn rust_compile_function(
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    source: *const u16,
    source_len: usize,
    sfd_ptr: *mut c_void,
    rust_function_ast: *mut c_void,
) -> *mut c_void {
    let func_data = Box::from_raw(rust_function_ast as *mut ast::FunctionData);

    // Extract local variables from the function body's scope data.
    let body_scope = match &func_data.body.inner {
        Statement::FunctionBody { ref scope, .. } => Some(scope),
        _ => None,
    };

    let mut gen = bytecode::generator::Generator::new();
    gen.strict = func_data.is_strict_mode;
    gen.vm_ptr = vm_ptr;
    gen.source_code_ptr = source_code_ptr;
    gen.source = source;
    gen.source_len = source_len;
    gen.enclosing_function_kind = match func_data.kind {
        ast::FunctionKind::Normal => bytecode::generator::FunctionKind::Normal,
        ast::FunctionKind::Generator => bytecode::generator::FunctionKind::Generator,
        ast::FunctionKind::Async => bytecode::generator::FunctionKind::Async,
        ast::FunctionKind::AsyncGenerator => bytecode::generator::FunctionKind::AsyncGenerator,
    };

    if let Some(scope) = body_scope {
        let sd = scope.borrow();
        gen.local_variables = sd.local_variables.iter().map(|lv| {
            bytecode::generator::LocalVariable {
                name: lv.name.clone(),
                is_lexically_declared: lv.kind == ast::LocalVarKind::LetOrConst,
                is_initialized_during_declaration_instantiation: false,
            }
        }).collect();
    }

    let entry_block = gen.make_block();
    gen.switch_to_basic_block(entry_block);

    // Initialize the lexical environment register (like C++ ensure_lexical_environment_register_initialized).
    {
        use bytecode::operand::{Operand, Register};
        let env_reg = gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT));
        gen.emit(bytecode::instruction::Instruction::GetLexicalEnvironment {
            dst: env_reg.operand(),
        });
        gen.lexical_environment_register_stack.push(env_reg);
    }

    // For async (non-generator) functions, emit the initial Yield BEFORE FDI
    // so that parameter evaluation errors are caught by the async promise wrapper.
    if gen.is_in_async_function() && !gen.is_in_generator_function() {
        let start_block = gen.make_block();
        let undef = gen.add_constant_undefined();
        gen.emit(bytecode::instruction::Instruction::Yield {
            continuation_label: Some(bytecode::operand::Label(start_block as u32)),
            value: undef.operand(),
        });
        gen.switch_to_basic_block(start_block);
    }

    // Emit FDI (FunctionDeclarationInstantiation) bytecode.
    if let Some(scope) = body_scope {
        bytecode::codegen::emit_function_declaration_instantiation(
            &mut gen, &func_data, &scope.borrow(),
        );
    }

    // For generator functions (including async generators), emit the initial Yield
    // AFTER FDI. Parameter evaluation happens synchronously before the generator starts.
    if gen.is_in_generator_function() {
        let start_block = gen.make_block();
        let undef = gen.add_constant_undefined();
        gen.emit(bytecode::instruction::Instruction::Yield {
            continuation_label: Some(bytecode::operand::Label(start_block as u32)),
            value: undef.operand(),
        });
        gen.switch_to_basic_block(start_block);
    }

    // Generate bytecode for the function body.
    let _result = bytecode::codegen::generate_stmt(&func_data.body, &mut gen, None);

    if !gen.is_current_block_terminated() {
        let undef = gen.add_constant_undefined();
        if gen.is_in_generator_or_async_function() {
            // Generator/async functions end with Yield (no continuation = done).
            gen.emit(bytecode::instruction::Instruction::Yield {
                continuation_label: None,
                value: undef.operand(),
            });
        } else {
            gen.emit(bytecode::instruction::Instruction::End {
                value: undef.operand(),
            });
        }
    }

    // For generator/async functions, terminate all unterminated blocks with Yield.
    if gen.is_in_generator_or_async_function() {
        let block_count = gen.basic_block_count();
        for i in 0..block_count {
            if gen.is_block_terminated(i) {
                continue;
            }
            gen.switch_to_basic_block(i);
            let undef = gen.add_constant_undefined();
            gen.emit(bytecode::instruction::Instruction::Yield {
                continuation_label: None,
                value: undef.operand(),
            });
        }
    }

    // Assemble
    let assembled = gen.assemble();

    // Write FDI runtime metadata to the SFD.
    // For now, set conservative defaults. Full FDI will be implemented
    // when we port emit_function_declaration_instantiation to Rust.
    write_sfd_metadata(sfd_ptr, &func_data);

    // Create C++ Executable via FFI
    bytecode::ffi::create_executable(&gen, &assembled, vm_ptr, source_code_ptr)
}

/// Write FDI runtime metadata to a C++ SharedFunctionInstanceData.
///
/// This sets fields that `prepare_for_ordinary_call()` and `internal_call()`
/// read at runtime. Reads scope analysis insights from the function body's
/// ScopeData (populated by the scope collector after analyze()).
unsafe fn write_sfd_metadata(sfd_ptr: *mut c_void, func_data: &ast::FunctionData) {
    let body_scope = match &func_data.body.inner {
        ast::Statement::FunctionBody { ref scope, .. } => Some(scope),
        _ => None,
    };

    let (uses_this, contains_eval, might_need_arguments) = if let Some(scope) = body_scope {
        let sd = scope.borrow();
        (
            // Respect both scope analysis AND explicit parsing insights
            // (e.g. for class field initializers / static initializers).
            sd.uses_this || func_data.parsing_insights.uses_this,
            sd.contains_direct_call_to_eval,
            sd.contains_access_to_arguments_object,
        )
    } else {
        // Conservative defaults if no scope data.
        (true, false, true)
    };
    rust_sfd_set_metadata(
        sfd_ptr,
        uses_this,
        // Conservative: always create function environment.
        true,
        0,
        might_need_arguments,
        contains_eval,
    );
}

extern "C" {
    fn rust_sfd_set_metadata(
        sfd_ptr: *mut c_void,
        uses_this: bool,
        function_environment_needed: bool,
        function_environment_bindings_count: usize,
        might_need_arguments_object: bool,
        contains_direct_call_to_eval: bool,
    );
}
