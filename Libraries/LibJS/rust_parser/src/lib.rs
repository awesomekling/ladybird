/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! # LibJS Rust Parser
//!
//! A JavaScript parser written in Rust that produces C++ AST nodes via FFI.
//! It is a slot-in replacement for the C++ `Parser` class in LibJS.
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
//! │  Calls AstBuilder to create C++ AST nodes           │
//! │  Drives ScopeCollector for scope analysis           │
//! └───────┬─────────────────────┬───────────────────────┘
//!         │                     │
//!         ▼                     ▼
//! ┌───────────────┐   ┌─────────────────────────────────┐
//! │  AstBuilder    │   │  ScopeCollector                 │
//! │  (ast_bridge)  │   │  (scope_collector.rs)           │
//! │  FFI to C++    │   │  Builds scope tree during parse │
//! │  ASTFactory    │   │  Resolves identifiers after     │
//! └───────┬────────┘   └─────────────────────────────────┘
//!         │
//!         ▼
//! ┌─────────────────────────────────────────────────────┐
//! │  C++ ASTFactory (ASTFactory.cpp)                    │
//! │  Creates actual AST nodes (RefCounted C++ objects)  │
//! │  Nodes have generate_bytecode() methods             │
//! └─────────────────────────────────────────────────────┘
//! ```
//!
//! ## Key design decisions
//!
//! - **Raw `extern "C"` FFI** rather than cxx, because the AK types
//!   (Vector, Variant, NonnullRefPtr, Utf16FlyString) are too complex
//!   for cxx bindings.
//!
//! - **No intermediate Rust AST.** The parser calls C++ factory functions
//!   directly during parsing. AST nodes are created as C++ objects and
//!   passed around as opaque `NodeHandle` pointers (`*mut c_void`).
//!
//! - **UTF-16 throughout.** Source code is received as `*const u16` and
//!   string values are passed to the C++ factory the same way. The C++
//!   side creates `Utf16FlyString` from these.
//!
//! - **Arena-based memory management.** The `AstBuilder` creates an arena
//!   on the C++ side. All AST nodes created during parsing are ref-held
//!   by the arena. The final Program node gets an extra ref before the
//!   arena is destroyed, so it (and its tree) survive.
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
//! - `ast_bridge.rs` — Safe Rust wrappers around C++ factory FFI
//! - `scope_collector.rs` — Scope analysis (identifier resolution, hoisting)

pub mod ast_bridge;
pub mod lexer;
pub mod parser;
pub mod scope_collector;
pub mod token;

use ast_bridge::NodeHandle;
use parser::{Parser, ProgramType};
use std::ffi::c_void;

/// Parse a JavaScript program from UTF-16 source code.
///
/// # Safety
/// - `source` must point to a valid UTF-16 buffer of `source_len` elements.
/// - `source_code` must be a valid SourceCode C++ object pointer.
/// - The returned NodeHandle is an opaque pointer to a C++ AST node owned by
///   the arena inside the parser. The caller must keep the arena alive.
#[no_mangle]
pub unsafe extern "C" fn rust_parse_program(
    source: *const u16,
    source_len: usize,
    source_code: *const c_void,
    program_type: u8,
    starts_in_strict_mode: bool,
    out_has_errors: *mut bool,
) -> NodeHandle {
    let source_slice = std::slice::from_raw_parts(source, source_len);
    let pt = if program_type == 1 {
        ProgramType::Module
    } else {
        ProgramType::Script
    };
    let mut parser = Parser::new(source_slice, source_code, pt);
    let program = parser.parse_program(starts_in_strict_mode);
    // Add an extra ref so the program node survives arena destruction.
    // The C++ caller adopts this ref.
    parser.builder.ref_node(program);
    if !out_has_errors.is_null() {
        *out_has_errors = parser.has_errors();
    }
    program
}
