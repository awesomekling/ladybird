// Copyright (c) 2026, Ladybird developers.
// SPDX-License-Identifier: BSD-2-Clause

pub mod active_formatting_elements;
pub mod dom_bridge;
pub mod entities;
pub mod parser;
pub mod stack_of_open_elements;
pub mod tag_names;
pub mod token;
pub mod tokenizer;

use std::ffi::c_void;

use parser::HtmlParser;
use tokenizer::State;

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

/// Set the context element for fragment parsing.
///
/// # Safety
/// `handle` must be a valid pointer from `rust_html_parser_create`.
/// `context_element` must be a valid pointer to a C++ DOM::Element.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_set_context_element(
    handle: *mut HtmlParserHandle,
    context_element: *mut c_void,
) {
    let handle = unsafe { &mut *handle };
    handle
        .parser
        .set_context_element(dom_bridge::DomHandle(context_element));
}

/// Set the tokenizer state.
///
/// # Safety
/// `handle` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_set_tokenizer_state(
    handle: *mut HtmlParserHandle,
    state: u8,
) {
    let handle = unsafe { &mut *handle };
    let state = State::from_u8(state).expect("invalid tokenizer state");
    handle.parser.tokenizer.switch_to(state);
}

/// Set the form element for fragment parsing.
///
/// # Safety
/// `handle` must be a valid pointer.
/// `form_element` must be a valid pointer to a C++ DOM::HTMLFormElement.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_set_form_element(
    handle: *mut HtmlParserHandle,
    form_element: *mut c_void,
) {
    let handle = unsafe { &mut *handle };
    handle
        .parser
        .set_form_element(dom_bridge::DomHandle(form_element));
}

/// Push an element onto the parser's stack of open elements.
///
/// # Safety
/// `handle` must be a valid pointer. `element` must be a valid DOM element pointer.
/// `tag_name` must point to `tag_name_len` valid UTF-8 bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_push_element(
    handle: *mut HtmlParserHandle,
    element: *mut c_void,
    tag_name: *const u8,
    tag_name_len: usize,
    namespace: u8,
) {
    let handle = unsafe { &mut *handle };
    let name = std::str::from_utf8(unsafe { std::slice::from_raw_parts(tag_name, tag_name_len) })
        .expect("invalid UTF-8 for tag name");
    handle.parser.push_onto_open_elements(
        dom_bridge::DomHandle(element),
        name,
        dom_bridge::DomNamespace::from_u8(namespace),
    );
}

/// Push "in template" onto the stack of template insertion modes.
///
/// # Safety
/// `handle` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_push_template_insertion_mode(
    handle: *mut HtmlParserHandle,
) {
    let handle = unsafe { &mut *handle };
    handle
        .parser
        .push_template_insertion_mode(parser::InsertionMode::InTemplate);
}

/// Reset the parser's insertion mode appropriately.
///
/// # Safety
/// `handle` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_reset_insertion_mode(handle: *mut HtmlParserHandle) {
    let handle = unsafe { &mut *handle };
    handle.parser.reset_insertion_mode();
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

/// Check if the tokenizer's insertion point is defined.
///
/// # Safety
/// `handle` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_is_insertion_point_defined(
    handle: *mut HtmlParserHandle,
) -> bool {
    let handle = unsafe { &*handle };
    handle.parser.tokenizer.is_insertion_point_defined()
}

/// Insert an EOF marker into the tokenizer's input stream.
///
/// # Safety
/// `handle` must be a valid pointer.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_html_parser_insert_eof(handle: *mut HtmlParserHandle) {
    let handle = unsafe { &mut *handle };
    handle.parser.tokenizer.insert_eof();
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