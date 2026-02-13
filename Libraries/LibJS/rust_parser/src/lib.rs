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
//! - `lib.rs` — Entry point (`rust_parse_program`), called from C++
//! - `token.rs` — Token types matching the C++ `TokenType` enum
//! - `lexer.rs` — Tokenizer: UTF-16 input → Token stream
//! - `parser.rs` — Parser state, helpers, token consumption
//! - `parser/expressions.rs` — Expression parsing (precedence climbing)
//! - `parser/statements.rs` — Statement parsing (if, for, while, etc.)
//! - `parser/declarations.rs` — Functions, classes, variables, modules
//! - `ast.rs` — Rust AST type definitions
//! - `bytecode/` — Bytecode generator, instruction types, and FFI
//! - `ast_bridge.rs` — (Legacy) Safe Rust wrappers around C++ factory FFI
//! - `scope_collector.rs` — (Legacy) Scope analysis

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
pub mod ast_bridge;
pub mod bytecode;
pub mod ffi_enums;
pub mod lexer;
pub mod parser;
pub mod scope_collector;
pub mod token;

use ast::Statement;
use ast_bridge::NodeHandle;
use parser::{Parser, ProgramType};
use std::ffi::c_void;

/// Parse a JavaScript program from UTF-16 source code.
///
/// The Rust parser builds a Rust AST. When `USE_RUST_CODEGEN` is set
/// (checked by the C++ caller), the codegen path produces a C++ Executable.
/// Otherwise, this signals errors so Script.cpp falls back to the C++ parser.
///
/// # Safety
/// - `source` must point to a valid UTF-16 buffer of `source_len` elements.
#[no_mangle]
pub unsafe extern "C" fn rust_parse_program(
    source: *const u16,
    source_len: usize,
    _source_code: *const c_void,
    program_type: u8,
    starts_in_strict_mode: bool,
    initiated_by_eval: bool,
    in_eval_function_context: bool,
    allow_super_property_lookup: bool,
    allow_super_constructor_call: bool,
    in_class_field_initializer: bool,
    out_has_errors: *mut bool,
) -> NodeHandle {
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
        parser.allow_super_property_lookup = allow_super_property_lookup;
        parser.allow_super_constructor_call = allow_super_constructor_call;
        parser.in_class_field_initializer = in_class_field_initializer;
    }

    // Run the parser to build the Rust AST.
    let _program = parser.parse_program(starts_in_strict_mode);

    // Always signal errors so the C++ caller falls back to the C++ parser.
    // The Rust codegen path is not yet ready for production use.
    if !out_has_errors.is_null() {
        *out_has_errors = true;
    }
    std::ptr::null_mut()
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
        parser.allow_super_property_lookup = allow_super_property_lookup;
        parser.allow_super_constructor_call = allow_super_constructor_call;
        parser.in_class_field_initializer = in_class_field_initializer;
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

    // Generate bytecode
    let mut gen = bytecode::generator::Generator::new();
    gen.strict = starts_in_strict_mode;
    gen.vm_ptr = vm_ptr;
    gen.source_code_ptr = source_code_ptr;
    gen.source = source;
    gen.source_len = source_len;

    // Copy program's local variables from scope analysis into the generator.
    if let Statement::Program(ref data) = program.inner {
        gen.local_variables = data.scope.local_variables.iter().map(|lv| {
            bytecode::generator::LocalVariable {
                name: lv.name.clone(),
                is_lexically_declared: lv.kind == ast::LocalVarKind::LetOrConst,
                is_initialized_during_declaration_instantiation: false,
            }
        }).collect();
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

    let result = bytecode::codegen::generate_stmt(&program, &mut gen, None);

    if !gen.is_current_block_terminated() {
        let value = result.unwrap_or_else(|| gen.add_constant_undefined());
        gen.emit(bytecode::instruction::Instruction::End {
            value: value.operand(),
        });
    }

    // Assemble
    let assembled = gen.assemble();

    // Create C++ Executable via FFI
    bytecode::ffi::create_executable(&gen, &assembled, vm_ptr, source_code_ptr)
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
        gen.local_variables = scope.local_variables.iter().map(|lv| {
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
            &mut gen, &func_data, scope,
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
        ast::Statement::FunctionBody { ref scope, .. } => Some(scope.as_ref()),
        _ => None,
    };

    let (uses_this, contains_eval, might_need_arguments) = if let Some(scope) = body_scope {
        (
            // Respect both scope analysis AND explicit parsing insights
            // (e.g. for class field initializers / static initializers).
            scope.uses_this || func_data.parsing_insights.uses_this,
            scope.contains_direct_call_to_eval,
            scope.contains_access_to_arguments_object,
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
