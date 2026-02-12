/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! # LibJS Rust Parser
//!
//! A JavaScript parser written in Rust that produces a Rust AST.
//! Currently, the Rust AST cannot yet be consumed by the C++ bytecode
//! generator, so this entry point always signals errors to cause the
//! C++ caller (Script.cpp) to fall back to the C++ parser.
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
pub mod ffi_enums;
pub mod lexer;
pub mod parser;
pub mod scope_collector;
pub mod token;

use ast_bridge::NodeHandle;
use parser::{Parser, ProgramType};
use std::ffi::c_void;

/// Parse a JavaScript program from UTF-16 source code.
///
/// The Rust parser now builds a Rust AST, which cannot yet be consumed
/// by the C++ bytecode generator. This function always sets
/// `out_has_errors = true` so that Script.cpp falls back to the C++
/// parser. The Rust parser is still exercised (parsing runs to
/// completion), which lets us verify it compiles and doesn't panic.
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

    // Run the parser to build the Rust AST (exercises the parser code).
    let _program = parser.parse_program(starts_in_strict_mode);

    // Always signal errors so the C++ caller falls back to the C++ parser.
    // The Rust AST is not yet bridged to C++ bytecode generation.
    if !out_has_errors.is_null() {
        *out_has_errors = true;
    }
    std::ptr::null_mut()
}
