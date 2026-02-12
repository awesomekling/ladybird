/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! FFI bridge for creating a C++ Executable from assembled Rust bytecode.

use std::ffi::c_void;

use super::generator::{AssembledBytecode, ConstantValue, Generator};

/// Opaque pointer returned from rust_create_executable.
pub type ExecutableHandle = *mut c_void;

// FFI types matching BytecodeFactory.h
#[repr(C)]
struct FFIExceptionHandler {
    start_offset: u32,
    end_offset: u32,
    handler_offset: u32,
}

#[repr(C)]
struct FFISourceMapEntry {
    bytecode_offset: u32,
    source_start: u32,
    source_end: u32,
}

#[repr(C)]
struct FFIUtf16Slice {
    data: *const u16,
    length: usize,
}

extern "C" {
    fn rust_create_executable(
        vm_ptr: *mut c_void,
        source_code_ptr: *const c_void,
        bytecode: *const u8,
        bytecode_len: usize,
        identifier_table: *const FFIUtf16Slice,
        identifier_count: usize,
        property_key_table: *const FFIUtf16Slice,
        property_key_count: usize,
        string_table: *const FFIUtf16Slice,
        string_count: usize,
        constants_data: *const u8,
        constants_data_len: usize,
        constants_count: usize,
        exception_handlers: *const FFIExceptionHandler,
        exception_handler_count: usize,
        source_map: *const FFISourceMapEntry,
        source_map_count: usize,
        basic_block_offsets: *const usize,
        basic_block_count: usize,
        local_var_names: *const FFIUtf16Slice,
        local_var_count: usize,
        property_lookup_cache_count: u32,
        global_variable_cache_count: u32,
        template_object_cache_count: u32,
        object_shape_cache_count: u32,
        number_of_registers: u32,
        is_strict: bool,
        shared_function_data: *const *const c_void,
        shared_function_data_count: usize,
    ) -> *mut c_void;

    pub fn rust_create_shared_function_data(
        vm_ptr: *mut c_void,
        source_code_ptr: *const c_void,
        source_text: *const u16,
        source_text_len: usize,
        name: *const u16,
        name_len: usize,
        strict_mode: bool,
    ) -> *mut c_void;
}

/// Encode constants into a tagged byte buffer for FFI.
fn encode_constants(constants: &[ConstantValue]) -> Vec<u8> {
    let mut buf = Vec::new();
    for c in constants {
        match c {
            ConstantValue::Number(v) => {
                buf.push(0); // CONSTANT_TAG_NUMBER
                buf.extend_from_slice(&v.to_le_bytes());
            }
            ConstantValue::Boolean(true) => buf.push(1),
            ConstantValue::Boolean(false) => buf.push(2),
            ConstantValue::Null => buf.push(3),
            ConstantValue::Undefined => buf.push(4),
            ConstantValue::Empty => buf.push(5),
            ConstantValue::String(s) => {
                buf.push(6); // CONSTANT_TAG_STRING
                let len = s.len() as u32;
                buf.extend_from_slice(&len.to_le_bytes());
                for &code_unit in s {
                    buf.extend_from_slice(&code_unit.to_le_bytes());
                }
            }
            ConstantValue::BigInt(s) => {
                buf.push(7); // CONSTANT_TAG_BIGINT
                let len = s.len() as u32;
                buf.extend_from_slice(&len.to_le_bytes());
                buf.extend_from_slice(s.as_bytes());
            }
        }
    }
    buf
}

/// Create a C++ Executable from the Rust generator's assembled output.
///
/// # Safety
/// `vm_ptr` must be a valid `JS::VM*` and `source_code_ptr` a valid
/// `JS::SourceCode const*`.
pub unsafe fn create_executable(
    gen: &Generator,
    assembled: &AssembledBytecode,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
) -> ExecutableHandle {
    // Build FFI slices for tables
    let ident_slices: Vec<FFIUtf16Slice> = gen
        .identifier_table
        .iter()
        .map(|s| FFIUtf16Slice {
            data: s.as_ptr(),
            length: s.len(),
        })
        .collect();

    let prop_key_slices: Vec<FFIUtf16Slice> = gen
        .property_key_table
        .iter()
        .map(|s| FFIUtf16Slice {
            data: s.as_ptr(),
            length: s.len(),
        })
        .collect();

    let string_slices: Vec<FFIUtf16Slice> = gen
        .string_table
        .iter()
        .map(|s| FFIUtf16Slice {
            data: s.as_ptr(),
            length: s.len(),
        })
        .collect();

    // Encode constants
    let constants_buf = encode_constants(&gen.constants);

    // Build FFI exception handlers
    let ffi_handlers: Vec<FFIExceptionHandler> = assembled
        .exception_handlers
        .iter()
        .map(|h| FFIExceptionHandler {
            start_offset: h.start_offset,
            end_offset: h.end_offset,
            handler_offset: h.handler_offset,
        })
        .collect();

    // Build FFI source map
    let ffi_source_map: Vec<FFISourceMapEntry> = assembled
        .source_map
        .iter()
        .map(|e| FFISourceMapEntry {
            bytecode_offset: e.bytecode_offset,
            source_start: e.source_start,
            source_end: e.source_end,
        })
        .collect();

    // Build local variable name slices
    let local_var_slices: Vec<FFIUtf16Slice> = gen
        .local_variables
        .iter()
        .map(|v| FFIUtf16Slice {
            data: v.name.as_ptr(),
            length: v.name.len(),
        })
        .collect();

    // Collect shared function data pointers
    let sfd_ptrs: Vec<*const c_void> = gen
        .shared_function_data
        .iter()
        .map(|ptr| *ptr as *const c_void)
        .collect();

    rust_create_executable(
        vm_ptr,
        source_code_ptr,
        assembled.bytecode.as_ptr(),
        assembled.bytecode.len(),
        ident_slices.as_ptr(),
        ident_slices.len(),
        prop_key_slices.as_ptr(),
        prop_key_slices.len(),
        string_slices.as_ptr(),
        string_slices.len(),
        constants_buf.as_ptr(),
        constants_buf.len(),
        gen.constants.len(),
        ffi_handlers.as_ptr(),
        ffi_handlers.len(),
        ffi_source_map.as_ptr(),
        ffi_source_map.len(),
        assembled.basic_block_start_offsets.as_ptr(),
        assembled.basic_block_start_offsets.len(),
        local_var_slices.as_ptr(),
        local_var_slices.len(),
        gen.next_property_lookup_cache,
        gen.next_global_variable_cache,
        gen.next_template_object_cache,
        gen.next_object_shape_cache,
        assembled.number_of_registers,
        gen.strict,
        sfd_ptrs.as_ptr(),
        sfd_ptrs.len(),
    )
}
