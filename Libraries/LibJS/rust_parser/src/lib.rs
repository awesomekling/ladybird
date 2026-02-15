/*
 * Copyright (c) 2026, the Ladybird developers.
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
//! │  Creates Executable from assembled data              │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Module overview
//!
//! - `lib.rs` — Entry point (FFI exports)
//! - `token.rs` — Token types (must match Token.h order)
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
pub mod ast_dump;
pub mod bytecode;
pub mod lexer;
pub mod parser;
pub mod scope_collector;
pub mod token;

use ast::StatementKind;
use parser::{Parser, ProgramType};
use std::cell::RefCell;
use std::ffi::c_void;
use std::rc::Rc;

/// Shared compilation pipeline: local variable setup → codegen → assemble → create Executable.
///
/// Called by all three program-level entry points after parsing and scope analysis.
unsafe fn compile_program_body(
    gen: &mut bytecode::generator::Generator,
    program: &ast::Statement,
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
        if let Some(value) = result {
            gen.emit(bytecode::instruction::Instruction::End {
                value: value.operand(),
            });
        }
        // If result is None, the assembler will add End(undefined) as a
        // fallthrough for unterminated blocks, matching C++ compile().
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

    let scope_ref = if let StatementKind::Program(ref data) = program.inner {
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
    dump_ast: bool,
    use_color: bool,
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

    // Dump AST if requested (after scope analysis so identifier metadata is populated).
    if dump_ast {
        ast_dump::dump_program(&program, use_color);
    }

    let (scope_ref, is_strict) = if let StatementKind::Program(ref data) = program.inner {
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

    let (scope_ref, is_strict) = if let StatementKind::Program(ref data) = program.inner {
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

    // Run scope analysis. Use analyze_as_dynamic_function() to suppress
    // marking identifiers as global, matching the C++ path which parses
    // as a FunctionExpression (no Program scope for globals to bind to).
    parser.scope_collector.analyze_as_dynamic_function();

    if parser.scope_collector.has_errors() {
        return std::ptr::null_mut();
    }

    // Extract the FunctionExpression from the program.
    // The program should contain a single ExpressionStatement wrapping a FunctionExpression.
    let func_data = if let StatementKind::Program(ref data) = program.inner {
        let scope = data.scope.borrow();
        let mut found: Option<ast::FunctionData> = None;
        for child in &scope.children {
            match &child.inner {
                StatementKind::FunctionDeclaration(fd) => {
                    found = Some(fd.as_ref().clone());
                    break;
                }
                StatementKind::Expression(expr) => {
                    if let ast::ExpressionKind::Function(fd) = &expr.inner {
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

    let mut func_data = match func_data {
        Some(fd) => fd,
        None => return std::ptr::null_mut(),
    };

    // Dynamic functions always need an arguments object, matching the C++
    // path in FunctionConstructor::create_dynamic_function.
    func_data.parsing_insights.might_need_arguments_object = true;

    // Determine strictness: the function itself may be strict.
    let is_strict = func_data.is_strict_mode;

    // Create SFD via FFI.
    bytecode::ffi::create_sfd_for_gdi(&func_data, vm_ptr, source_code_ptr, is_strict)
}

/// Recursively collect var-declared names from a statement and all nested
/// statements, excluding function/class bodies (which create new var scopes).
/// Calls `push_name` for each discovered name.
unsafe fn collect_var_names_recursive(
    stmt: &ast::StatementKind,
    ctx: *mut c_void,
    push_name: unsafe extern "C" fn(*mut c_void, *const u16, usize),
) {
    match stmt {
        ast::StatementKind::VariableDeclaration {
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
    use ast::{DeclarationKind, StatementKind};

    // Var names (var declarations at any nesting level + top-level function declarations)
    for child in &scope.children {
        collect_var_names_recursive(&child.inner, ctx, push_var_name);
        if let StatementKind::FunctionDeclaration(func_data) = &child.inner {
            if let Some(ref name) = func_data.name {
                push_var_name(ctx, name.name.as_ptr(), name.name.len());
            }
        }
    }

    // Functions to initialize (reverse order, deduplicated by name).
    let mut seen_names: Vec<Vec<u16>> = Vec::new();
    let mut funcs_to_init: Vec<(&ast::FunctionData, Vec<u16>)> = Vec::new();
    for child in scope.children.iter().rev() {
        if let StatementKind::FunctionDeclaration(func_data) = &child.inner {
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
            StatementKind::VariableDeclaration { kind, declarations }
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
            StatementKind::UsingDeclaration { declarations } => {
                for decl in declarations {
                    let mut names = Vec::new();
                    collect_bound_names_from_target(&decl.target, &mut names);
                    for name in &names {
                        push_lexical_binding(ctx, name.as_ptr(), name.len(), false);
                    }
                }
            }
            StatementKind::ClassDeclaration(class_data) => {
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
    use ast::{DeclarationKind, StatementKind};
    use bytecode::ffi::{
        script_gdi_push_annex_b_name, script_gdi_push_function, script_gdi_push_lexical_binding,
        script_gdi_push_lexical_name, script_gdi_push_var_name, script_gdi_push_var_scoped_name,
    };

    // Lexical names (let/const/using/class at top level) — script-only step.
    for child in &scope.children {
        match &child.inner {
            StatementKind::VariableDeclaration { kind, declarations }
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
            StatementKind::UsingDeclaration { declarations } => {
                for decl in declarations {
                    let mut names = Vec::new();
                    collect_bound_names_from_target(&decl.target, &mut names);
                    for name in &names {
                        script_gdi_push_lexical_name(ctx, name.as_ptr(), name.len());
                    }
                }
            }
            StatementKind::ClassDeclaration(class_data) => {
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
fn for_each_child_statement(stmt: &ast::StatementKind, f: &mut dyn FnMut(&ast::StatementKind)) {
    use ast::StatementKind;

    match stmt {
        StatementKind::Block(scope) => {
            for child in &scope.borrow().children {
                f(&child.inner);
            }
        }
        StatementKind::If { consequent, alternate, .. } => {
            f(&consequent.inner);
            if let Some(alt) = alternate {
                f(&alt.inner);
            }
        }
        StatementKind::While { body, .. }
        | StatementKind::DoWhile { body, .. }
        | StatementKind::With { body, .. } => {
            f(&body.inner);
        }
        StatementKind::For { init, body, .. } => {
            if let Some(init) = init {
                f(&init.inner);
            }
            f(&body.inner);
        }
        StatementKind::ForIn { lhs, body, .. }
        | StatementKind::ForOf { lhs, body, .. }
        | StatementKind::ForAwaitOf { lhs, body, .. } => {
            if let ast::ForInOfLhs::Declaration(decl) = lhs {
                f(&decl.inner);
            }
            f(&body.inner);
        }
        StatementKind::Switch(data) => {
            for case in &data.cases {
                for child in &case.scope.borrow().children {
                    f(&child.inner);
                }
            }
        }
        StatementKind::Try(data) => {
            f(&data.block.inner);
            if let Some(ref handler) = data.handler {
                f(&handler.body.inner);
            }
            if let Some(ref finalizer) = data.finalizer {
                f(&finalizer.inner);
            }
        }
        StatementKind::Labelled { item, .. } => {
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
        StatementKind::FunctionBody { ref scope, .. } => Some(scope),
        StatementKind::Block(ref scope) => Some(scope),
        _ => None,
    };

    // Compute SFD metadata before codegen so the generator can use
    // function_environment_needed to optimize `this` access.
    let sfd_metadata = compute_sfd_metadata(&func_data);

    let mut gen = bytecode::generator::Generator::new();
    gen.strict = func_data.is_strict_mode;
    gen.function_environment_needed = sfd_metadata.function_environment_needed;
    gen.vm_ptr = vm_ptr;
    gen.source_code_ptr = source_code_ptr;
    gen.source = source;
    gen.source_len = source_len;
    gen.enclosing_function_kind = func_data.kind;

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

    // For async (non-generator) functions, emit the initial Yield BEFORE
    // GetLexicalEnvironment so that parameter evaluation errors are caught
    // by the async promise wrapper. This matches C++ ordering.
    if gen.is_in_async_function() && !gen.is_in_generator_function() {
        let start_block = gen.make_block();
        let undef = gen.add_constant_undefined();
        gen.emit(bytecode::instruction::Instruction::Yield {
            continuation_label: Some(bytecode::operand::Label(start_block as u32)),
            value: undef.operand(),
        });
        gen.switch_to_basic_block(start_block);
    }

    // Initialize the lexical environment register.
    {
        use bytecode::operand::{Operand, Register};
        let env_reg = gen.scoped_operand(Operand::register(Register::SAVED_LEXICAL_ENVIRONMENT));
        gen.emit(bytecode::instruction::Instruction::GetLexicalEnvironment {
            dst: env_reg.operand(),
        });
        gen.lexical_environment_register_stack.push(env_reg);
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
    let result = bytecode::codegen::generate_stmt(&func_data.body, &mut gen, None);

    if !gen.is_current_block_terminated() {
        if gen.is_in_generator_or_async_function() {
            // Generator/async functions end with Yield (no continuation = done).
            let undef = gen.add_constant_undefined();
            gen.emit(bytecode::instruction::Instruction::Yield {
                continuation_label: None,
                value: undef.operand(),
            });
        } else if let Some(value) = result {
            gen.emit(bytecode::instruction::Instruction::End {
                value: value.operand(),
            });
        }
        // If result is None, the assembler will add End(undefined) as a
        // fallthrough for unterminated blocks, matching C++ compile().
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

    // Write precomputed FDI runtime metadata to the SFD.
    write_sfd_metadata(sfd_ptr, &sfd_metadata);

    // Create C++ Executable via FFI
    bytecode::ffi::create_executable(&gen, &assembled, vm_ptr, source_code_ptr)
}

/// Metadata computed from scope analysis for a SharedFunctionInstanceData.
struct SfdMetadata {
    uses_this: bool,
    function_environment_needed: bool,
    function_environment_bindings_count: usize,
    might_need_arguments: bool,
    contains_eval: bool,
}

/// Compute FDI runtime metadata matching the C++ SharedFunctionInstanceData
/// constructor (ECMA-262 §10.2.11).
fn compute_sfd_metadata(func_data: &ast::FunctionData) -> SfdMetadata {
    let body_scope = match &func_data.body.inner {
        ast::StatementKind::FunctionBody { ref scope, .. } => Some(scope),
        _ => None,
    };

    let strict = func_data.is_strict_mode;
    let is_arrow = func_data.is_arrow_function;

    // Extract all scope analysis data in one borrow.
    let (uses_this, contains_eval, uses_this_from_env, might_need_arguments,
         fsd_has_function_named_arguments, fsd_has_lexically_declared_arguments,
         fsd_non_local_var_count, fsd_non_local_var_count_for_param_exprs,
         fsd_var_names, annexb_function_names,
         has_arguments_object_local) = if let Some(scope) = &body_scope {
        let sd = scope.borrow();
        let fsd = sd.function_scope_data.as_ref();
        (
            sd.uses_this || func_data.parsing_insights.uses_this,
            sd.contains_direct_call_to_eval,
            sd.uses_this_from_environment
                || func_data.parsing_insights.uses_this_from_environment,
            func_data.parsing_insights.might_need_arguments_object,
            fsd.map_or(false, |f| f.has_function_named_arguments),
            fsd.map_or(false, |f| f.has_lexically_declared_arguments),
            fsd.map_or(0, |f| f.non_local_var_count),
            fsd.map_or(0, |f| f.non_local_var_count_for_parameter_expressions),
            fsd.map(|f| &f.var_names).cloned().unwrap_or_default(),
            sd.annexb_function_names.clone(),
            sd.local_variables.iter().any(|lv| lv.kind == ast::LocalVarKind::ArgumentsObject),
        )
    } else {
        // No scope body (e.g. synthetic class field initializer functions).
        // Fall back to parsing insights for the values we have.
        (func_data.parsing_insights.uses_this,
         func_data.parsing_insights.contains_direct_call_to_eval,
         func_data.parsing_insights.uses_this_from_environment,
         func_data.parsing_insights.might_need_arguments_object,
         false, false, 0, 0, Vec::new(), Vec::new(), false)
    };

    // §10.2.11 step 4: check for parameter expressions.
    let has_parameter_expressions = func_data.parameters.iter().any(|p| {
        p.default_value.is_some()
            || matches!(p.binding, ast::FunctionParameterBinding::BindingPattern(_))
    });

    // §10.2.11 steps 5-8: count non-local unique parameter names.
    let mut parameter_names: Vec<Vec<u16>> = Vec::new();
    let mut parameters_in_environment: usize = 0;
    for param in &func_data.parameters {
        match &param.binding {
            ast::FunctionParameterBinding::Identifier(ident) => {
                if !parameter_names.contains(&ident.name) {
                    parameter_names.push(ident.name.clone());
                    if !ident.is_local() {
                        parameters_in_environment += 1;
                    }
                }
            }
            ast::FunctionParameterBinding::BindingPattern(pattern) => {
                for_each_binding_pattern_identifier(pattern, &mut |ident| {
                    if !parameter_names.contains(&ident.name) {
                        parameter_names.push(ident.name.clone());
                        if !ident.is_local() {
                            parameters_in_environment += 1;
                        }
                    }
                });
            }
        }
    }

    // §10.2.11 steps 15-18: determine if arguments object is needed.
    let mut arguments_object_needed = might_need_arguments;
    if is_arrow {
        arguments_object_needed = false;
    } else if parameter_names.iter().any(|n| n == utf16!("arguments")) {
        arguments_object_needed = false;
    }
    if body_scope.is_none() {
        arguments_object_needed = false;
    } else {
        if !has_parameter_expressions && fsd_has_function_named_arguments {
            arguments_object_needed = false;
        }
        if !has_parameter_expressions && arguments_object_needed
            && fsd_has_lexically_declared_arguments
        {
            arguments_object_needed = false;
        }
    }

    // Arguments object needs an environment binding if it's not a local variable.
    let arguments_object_needs_binding =
        arguments_object_needed && !has_arguments_object_local;

    // --- Compute environment binding counts ---
    let mut function_environment_bindings_count: usize = 0;
    let mut var_environment_bindings_count: usize = 0;
    let mut lex_environment_bindings_count: usize = 0;

    // §10.2.11 step 19: route parameter bindings.
    let env_is_function_env = strict || !has_parameter_expressions;
    if env_is_function_env {
        function_environment_bindings_count += parameters_in_environment;
    }

    // §10.2.11 step 22: arguments binding.
    if arguments_object_needs_binding && env_is_function_env {
        function_environment_bindings_count += 1;
    }

    if body_scope.is_some() {
        if !has_parameter_expressions {
            // §10.2.11 step 27: var env shares function env.
            function_environment_bindings_count += fsd_non_local_var_count;

            // Annex B: function names hoisted from blocks that aren't already vars.
            if !strict {
                for name in &annexb_function_names {
                    if !fsd_var_names.contains(name) {
                        function_environment_bindings_count += 1;
                    }
                }
            }

            // §10.2.11 step 30: lexical environment.
            let non_local_lex_count =
                count_non_local_lex_declarations(body_scope.unwrap());
            if strict {
                // Lex env == var env == function env.
                function_environment_bindings_count += non_local_lex_count;
            } else {
                let can_elide = !contains_eval && non_local_lex_count == 0;
                if !can_elide {
                    lex_environment_bindings_count += non_local_lex_count;
                }
            }
        } else {
            // §10.2.11 step 28: separate var environment.
            var_environment_bindings_count += fsd_non_local_var_count_for_param_exprs;

            if !strict {
                for name in &annexb_function_names {
                    if !fsd_var_names.contains(name) {
                        var_environment_bindings_count += 1;
                    }
                }
            }

            let non_local_lex_count =
                count_non_local_lex_declarations(body_scope.unwrap());
            if strict {
                // Lex env == var env.
                var_environment_bindings_count += non_local_lex_count;
            } else {
                let can_elide = !contains_eval && non_local_lex_count == 0;
                if !can_elide {
                    lex_environment_bindings_count += non_local_lex_count;
                }
            }
        }
    }

    let function_environment_needed = arguments_object_needs_binding
        || function_environment_bindings_count > 0
        || var_environment_bindings_count > 0
        || lex_environment_bindings_count > 0
        || uses_this_from_env
        || contains_eval;

    SfdMetadata {
        uses_this,
        function_environment_needed,
        function_environment_bindings_count,
        might_need_arguments,
        contains_eval,
    }
}

/// Write precomputed SFD metadata to a C++ SharedFunctionInstanceData via FFI.
///
/// # Safety
/// `sfd_ptr` must be a valid `JS::SharedFunctionInstanceData*`.
unsafe fn write_sfd_metadata(sfd_ptr: *mut c_void, metadata: &SfdMetadata) {
    rust_sfd_set_metadata(
        sfd_ptr,
        metadata.uses_this,
        metadata.function_environment_needed,
        metadata.function_environment_bindings_count,
        metadata.might_need_arguments,
        metadata.contains_eval,
    );
}

/// Count non-local lexically-declared identifiers in a function body scope.
/// Matches C++ `ScopeNode::has_non_local_lexical_declarations()` but returns
/// the count (used for environment sizing in the function_environment_needed
/// computation).
fn count_non_local_lex_declarations(scope: &Rc<RefCell<ast::ScopeData>>) -> usize {
    let sd = scope.borrow();
    let mut count = 0;
    for child in &sd.children {
        match &child.inner {
            ast::StatementKind::VariableDeclaration { kind, declarations } => {
                use parser::DeclarationKind;
                if *kind == DeclarationKind::Let || *kind == DeclarationKind::Const {
                    for decl in declarations {
                        count_non_local_names_in_target(&decl.target, &mut count);
                    }
                }
            }
            ast::StatementKind::ClassDeclaration(class_data) => {
                if let Some(ref name_ident) = class_data.name {
                    if !name_ident.is_local() {
                        count += 1;
                    }
                }
            }
            _ => {}
        }
    }
    count
}

fn count_non_local_names_in_target(target: &ast::VariableDeclaratorTarget, count: &mut usize) {
    match target {
        ast::VariableDeclaratorTarget::Identifier(ident) => {
            if !ident.is_local() {
                *count += 1;
            }
        }
        ast::VariableDeclaratorTarget::BindingPattern(pattern) => {
            count_non_local_names_in_binding_pattern(pattern, count);
        }
    }
}

fn count_non_local_names_in_binding_pattern(
    pattern: &ast::BindingPattern,
    count: &mut usize,
) {
    for entry in &pattern.entries {
        match &entry.alias {
            ast::BindingEntryAlias::Identifier(ident) => {
                if !ident.is_local() {
                    *count += 1;
                }
            }
            ast::BindingEntryAlias::BindingPattern(sub) => {
                count_non_local_names_in_binding_pattern(sub, count);
            }
            ast::BindingEntryAlias::Empty => {
                if let ast::BindingEntryName::Identifier(ident) = &entry.name {
                    if !ident.is_local() {
                        *count += 1;
                    }
                }
            }
            ast::BindingEntryAlias::MemberExpression(_) => {}
        }
    }
}

/// Iterate all identifiers bound by a binding pattern.
fn for_each_binding_pattern_identifier(
    pattern: &ast::BindingPattern,
    callback: &mut dyn FnMut(&Rc<ast::Identifier>),
) {
    for entry in &pattern.entries {
        match &entry.alias {
            ast::BindingEntryAlias::Identifier(ident) => callback(ident),
            ast::BindingEntryAlias::BindingPattern(sub) => {
                for_each_binding_pattern_identifier(sub, callback);
            }
            ast::BindingEntryAlias::Empty => {
                if let ast::BindingEntryName::Identifier(ident) = &entry.name {
                    callback(ident);
                }
            }
            ast::BindingEntryAlias::MemberExpression(_) => {}
        }
    }
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
