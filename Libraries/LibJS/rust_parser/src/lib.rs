/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

pub mod ast_bridge;
pub mod lexer;
pub mod parser;
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
) -> NodeHandle {
    let source_slice = std::slice::from_raw_parts(source, source_len);
    let pt = if program_type == 1 {
        ProgramType::Module
    } else {
        ProgramType::Script
    };
    let mut parser = Parser::new(source_slice, source_code, pt);
    parser.parse_program(starts_in_strict_mode)
}
