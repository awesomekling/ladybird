// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

pub mod active_formatting_elements;
pub mod dom_bridge;
pub mod entities;
pub mod parser;
pub mod stack_of_open_elements;
pub mod token;
pub mod tokenizer;

use std::ffi::c_void;
use std::ptr;

use parser::HtmlParser;
use token::{Attribute, Position, TokenPayload, TokenType};
use tokenizer::{HtmlTokenizer, State};

// =======================================================================
// Rust HTML Parser FFI
// =======================================================================

/// Opaque handle for the Rust HTML parser, passed across the FFI boundary.
pub struct HtmlParserHandle {
    parser: HtmlParser,
}

/// Create a new Rust HTML parser.
///
/// # Safety
/// `document` must be a valid pointer to a C++ DOM::Document.
/// `input` must point to `input_len` valid u32 code points.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_create(
    document: *mut c_void,
    input: *const u32,
    input_len: usize,
    scripting_enabled: bool,
) -> *mut HtmlParserHandle {
    let code_points = if input.is_null() || input_len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(input, input_len) }.to_vec()
    };

    let handle = Box::new(HtmlParserHandle {
        parser: HtmlParser::new(
            dom_bridge::DomHandle(document),
            code_points,
            scripting_enabled,
        ),
    });

    Box::into_raw(handle)
}

/// Run the Rust HTML parser.
///
/// # Safety
/// `handle` must be a valid pointer from `rust_html_parser_create`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_run(handle: *mut HtmlParserHandle) {
    let handle = unsafe { &mut *handle };
    handle.parser.run();
}

/// Visit all DOM handles held by the parser for garbage collection.
///
/// # Safety
/// `handle` must be a valid pointer from `rust_html_parser_create`.
/// `visitor` must be a valid pointer to a C++ GC::Cell::Visitor.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_visit_dom_handles(
    handle: *mut HtmlParserHandle,
    visitor: *mut c_void,
) {
    let handle = unsafe { &*handle };
    handle.parser.visit_dom_handles(visitor);
}

/// Insert input at the tokenizer's insertion point (for document.write).
///
/// # Safety
/// `handle` must be a valid pointer. `input` must point to `input_len` valid u32 values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_insert_input(
    handle: *mut HtmlParserHandle,
    input: *const u32,
    input_len: usize,
) {
    let handle = unsafe { &mut *handle };
    let code_points = if input.is_null() || input_len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(input, input_len) }
    };
    handle.parser.tokenizer.insert_input_at_insertion_point(code_points);
}

/// Run the Rust parser, stopping at the insertion point.
///
/// # Safety
/// `handle` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_run_stop_at_insertion_point(
    handle: *mut HtmlParserHandle,
) {
    let handle = unsafe { &mut *handle };
    handle.parser.run_stop_at_insertion_point();
}

/// Destroy a Rust HTML parser.
///
/// # Safety
/// `handle` must be a valid pointer from `rust_html_parser_create`,
/// and must not be used after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_destroy(handle: *mut HtmlParserHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}

// =======================================================================
// Rust HTML Tokenizer FFI (kept for backward compatibility with C++ HTMLParser)
// =======================================================================

/// Opaque handle for the Rust tokenizer, passed across the FFI boundary.
pub struct FfiTokenizerHandle {
    tokenizer: HtmlTokenizer,
    last_tag_name: Vec<u8>,
    last_comment: Vec<u8>,
    last_doctype_name: Vec<u8>,
    last_public_id: Vec<u8>,
    last_system_id: Vec<u8>,
    last_attributes: Vec<FfiAttribute>,
    last_attr_names: Vec<Vec<u8>>,
    last_attr_values: Vec<Vec<u8>>,
}

#[repr(C)]
pub struct FfiToken {
    pub token_type: u8,
    pub code_point: u32,
    pub self_closing: bool,
    pub tag_name_ptr: *const u8,
    pub tag_name_len: usize,
    pub comment_ptr: *const u8,
    pub comment_len: usize,
    pub doctype_name_ptr: *const u8,
    pub doctype_name_len: usize,
    pub public_id_ptr: *const u8,
    pub public_id_len: usize,
    pub system_id_ptr: *const u8,
    pub system_id_len: usize,
    pub force_quirks: bool,
    pub missing_name: bool,
    pub missing_public_id: bool,
    pub missing_system_id: bool,
    pub attributes_ptr: *const FfiAttribute,
    pub attributes_len: usize,
    pub start_line: u64,
    pub start_column: u64,
    pub end_line: u64,
    pub end_column: u64,
}

#[repr(C)]
pub struct FfiAttribute {
    pub name_ptr: *const u8,
    pub name_len: usize,
    pub value_ptr: *const u8,
    pub value_len: usize,
    pub name_start_line: u64,
    pub name_start_column: u64,
    pub name_end_line: u64,
    pub name_end_column: u64,
    pub value_start_line: u64,
    pub value_start_column: u64,
    pub value_end_line: u64,
    pub value_end_column: u64,
}

impl Default for FfiToken {
    fn default() -> Self {
        FfiToken {
            token_type: TokenType::Invalid as u8,
            code_point: 0,
            self_closing: false,
            tag_name_ptr: ptr::null(),
            tag_name_len: 0,
            comment_ptr: ptr::null(),
            comment_len: 0,
            doctype_name_ptr: ptr::null(),
            doctype_name_len: 0,
            public_id_ptr: ptr::null(),
            public_id_len: 0,
            system_id_ptr: ptr::null(),
            system_id_len: 0,
            force_quirks: false,
            missing_name: true,
            missing_public_id: true,
            missing_system_id: true,
            attributes_ptr: ptr::null(),
            attributes_len: 0,
            start_line: 0,
            start_column: 0,
            end_line: 0,
            end_column: 0,
        }
    }
}

fn position_to_ffi(pos: &Position) -> (u64, u64) {
    (pos.line, pos.column)
}

fn attribute_to_ffi(attr: &Attribute, name_buf: &[u8], value_buf: &[u8]) -> FfiAttribute {
    let (nsl, nsc) = position_to_ffi(&attr.name_start_position);
    let (nel, nec) = position_to_ffi(&attr.name_end_position);
    let (vsl, vsc) = position_to_ffi(&attr.value_start_position);
    let (vel, vec_) = position_to_ffi(&attr.value_end_position);
    FfiAttribute {
        name_ptr: name_buf.as_ptr(),
        name_len: name_buf.len(),
        value_ptr: value_buf.as_ptr(),
        value_len: value_buf.len(),
        name_start_line: nsl,
        name_start_column: nsc,
        name_end_line: nel,
        name_end_column: nec,
        value_start_line: vsl,
        value_start_column: vsc,
        value_end_line: vel,
        value_end_column: vec_,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_create(
    input: *const u32,
    len: usize,
) -> *mut FfiTokenizerHandle {
    let code_points = if input.is_null() || len == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(input, len) }.to_vec()
    };
    let handle = Box::new(FfiTokenizerHandle {
        tokenizer: HtmlTokenizer::new(code_points),
        last_tag_name: Vec::new(),
        last_comment: Vec::new(),
        last_doctype_name: Vec::new(),
        last_public_id: Vec::new(),
        last_system_id: Vec::new(),
        last_attributes: Vec::new(),
        last_attr_names: Vec::new(),
        last_attr_values: Vec::new(),
    });
    Box::into_raw(handle)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_next_token(
    handle: *mut FfiTokenizerHandle,
    out: *mut FfiToken,
    stop_at_insertion_point: bool,
    cdata_allowed: bool,
) -> bool {
    let handle = unsafe { &mut *handle };
    let out = unsafe { &mut *out };
    let token = match handle
        .tokenizer
        .next_token(stop_at_insertion_point, cdata_allowed)
    {
        Some(t) => t,
        None => return false,
    };
    match token.token_type {
        TokenType::Character | TokenType::EndOfFile => {
            out.token_type = token.token_type as u8;
            out.code_point = token.code_point;
            out.start_line = token.start_position.line;
            out.start_column = token.start_position.column;
            out.end_line = token.end_position.line;
            out.end_column = token.end_position.column;
            return true;
        }
        _ => {}
    }
    *out = FfiToken::default();
    out.token_type = token.token_type as u8;
    let (sl, sc) = position_to_ffi(&token.start_position);
    let (el, ec) = position_to_ffi(&token.end_position);
    out.start_line = sl;
    out.start_column = sc;
    out.end_line = el;
    out.end_column = ec;
    match token.payload {
        TokenPayload::Tag {
            tag_name,
            self_closing,
            attributes,
        } => {
            out.self_closing = self_closing;
            handle.last_tag_name = tag_name.into_bytes();
            out.tag_name_ptr = handle.last_tag_name.as_ptr();
            out.tag_name_len = handle.last_tag_name.len();
            handle.last_attr_names.clear();
            handle.last_attr_values.clear();
            handle.last_attributes.clear();
            for attr in &attributes {
                handle
                    .last_attr_names
                    .push(attr.local_name.clone().into_bytes());
                handle
                    .last_attr_values
                    .push(attr.value.clone().into_bytes());
            }
            for (i, attr) in attributes.iter().enumerate() {
                let ffi_attr = attribute_to_ffi(
                    attr,
                    &handle.last_attr_names[i],
                    &handle.last_attr_values[i],
                );
                handle.last_attributes.push(ffi_attr);
            }
            out.attributes_ptr = handle.last_attributes.as_ptr();
            out.attributes_len = handle.last_attributes.len();
        }
        TokenPayload::Comment(data) => {
            handle.last_comment = data.into_bytes();
            out.comment_ptr = handle.last_comment.as_ptr();
            out.comment_len = handle.last_comment.len();
        }
        TokenPayload::Doctype(doctype) => {
            handle.last_doctype_name = doctype.name.into_bytes();
            handle.last_public_id = doctype.public_identifier.into_bytes();
            handle.last_system_id = doctype.system_identifier.into_bytes();
            out.doctype_name_ptr = handle.last_doctype_name.as_ptr();
            out.doctype_name_len = handle.last_doctype_name.len();
            out.public_id_ptr = handle.last_public_id.as_ptr();
            out.public_id_len = handle.last_public_id.len();
            out.system_id_ptr = handle.last_system_id.as_ptr();
            out.system_id_len = handle.last_system_id.len();
            out.force_quirks = doctype.force_quirks;
            out.missing_name = doctype.missing_name;
            out.missing_public_id = doctype.missing_public_identifier;
            out.missing_system_id = doctype.missing_system_identifier;
        }
        TokenPayload::None => {}
    }
    true
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_switch_state(
    handle: *mut FfiTokenizerHandle,
    state: u8,
) {
    let handle = unsafe { &mut *handle };
    let state: State = unsafe { std::mem::transmute(state) };
    handle.tokenizer.switch_to(state);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_insert_input(
    handle: *mut FfiTokenizerHandle,
    input: *const u32,
    len: usize,
) {
    let handle = unsafe { &mut *handle };
    let code_points = if input.is_null() || len == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(input, len) }
    };
    handle
        .tokenizer
        .insert_input_at_insertion_point(code_points);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_store_insertion_point(
    handle: *mut FfiTokenizerHandle,
) {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.store_insertion_point();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_restore_insertion_point(
    handle: *mut FfiTokenizerHandle,
) {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.restore_insertion_point();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_update_insertion_point(
    handle: *mut FfiTokenizerHandle,
) {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.update_insertion_point();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_undefine_insertion_point(
    handle: *mut FfiTokenizerHandle,
) {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.undefine_insertion_point();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_is_insertion_point_defined(
    handle: *mut FfiTokenizerHandle,
) -> bool {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.is_insertion_point_defined()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_set_blocked(
    handle: *mut FfiTokenizerHandle,
    blocked: bool,
) {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.set_blocked(blocked);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_is_blocked(
    handle: *mut FfiTokenizerHandle,
) -> bool {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.is_blocked()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_insert_eof(handle: *mut FfiTokenizerHandle) {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.insert_eof();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_is_eof_inserted(
    handle: *mut FfiTokenizerHandle,
) -> bool {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.is_eof_inserted()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_abort(handle: *mut FfiTokenizerHandle) {
    let handle = unsafe { &mut *handle };
    handle.tokenizer.abort();
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_set_last_start_tag_name(
    handle: *mut FfiTokenizerHandle,
    ptr: *const u8,
    len: usize,
) {
    let handle = unsafe { &mut *handle };
    let name = if ptr.is_null() || len == 0 {
        ""
    } else {
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(ptr, len)) }
    };
    handle.tokenizer.set_last_start_tag(name);
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_tokenizer_destroy(handle: *mut FfiTokenizerHandle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}
