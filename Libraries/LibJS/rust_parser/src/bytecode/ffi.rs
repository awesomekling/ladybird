/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! FFI bridge for creating a `Bytecode::Executable` from assembled bytecode.

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
pub struct FFIUtf16Slice {
    pub data: *const u16,
    pub length: usize,
}

#[repr(C)]
pub struct FFIClassElement {
    pub kind: u8, // 0=Method, 1=Getter, 2=Setter, 3=Field, 4=StaticInitializer
    pub is_static: bool,
    pub is_private: bool,
    pub private_identifier: *const u16,
    pub private_identifier_len: usize,
    pub shared_function_data_index: i32, // -1 for none
    pub has_initializer: bool,
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
        length_identifier: i32, // -1 for none, otherwise PropertyKeyTableIndex
        shared_function_data: *const *const c_void,
        shared_function_data_count: usize,
        class_blueprints: *const *mut c_void,
        class_blueprint_count: usize,
        compiled_regexes: *const *mut c_void,
        regex_count: usize,
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

    pub fn rust_create_sfd(
        vm_ptr: *mut c_void,
        source_code_ptr: *const c_void,
        name: *const u16,
        name_len: usize,
        function_kind: u8,
        function_length: i32,
        formal_parameter_count: u32,
        strict: bool,
        is_arrow: bool,
        has_simple_parameter_list: bool,
        param_names: *const FFIUtf16Slice,
        param_name_count: usize,
        source_text_offset: usize,
        source_text_len: usize,
        rust_function_ast: *mut c_void,
        uses_this: bool,
        uses_this_from_environment: bool,
    ) -> *mut c_void;

    pub fn rust_sfd_set_class_field_initializer_name(
        sfd_ptr: *mut c_void,
        name: *const u16,
        name_len: usize,
        is_private: bool,
    );

    pub fn rust_create_class_blueprint(
        source_code_ptr: *const c_void,
        name: *const u16,
        name_len: usize,
        source_text_offset: usize,
        source_text_len: usize,
        constructor_sfd_index: u32,
        has_super_class: bool,
        has_name: bool,
        elements: *const FFIClassElement,
        element_count: usize,
    ) -> *mut c_void;

    // Callbacks for populating Script GDI data from Rust.
    pub fn script_gdi_push_lexical_name(ctx: *mut c_void, name: *const u16, len: usize);
    pub fn script_gdi_push_var_name(ctx: *mut c_void, name: *const u16, len: usize);
    pub fn script_gdi_push_function(ctx: *mut c_void, sfd: *mut c_void, name: *const u16, len: usize);
    pub fn script_gdi_push_var_scoped_name(ctx: *mut c_void, name: *const u16, len: usize);
    pub fn script_gdi_push_annex_b_name(ctx: *mut c_void, name: *const u16, len: usize);
    pub fn script_gdi_push_lexical_binding(ctx: *mut c_void, name: *const u16, len: usize, is_constant: bool);

    // Callbacks for populating eval EDI data from Rust.
    pub fn eval_gdi_set_strict(ctx: *mut c_void, is_strict: bool);
    pub fn eval_gdi_push_var_name(ctx: *mut c_void, name: *const u16, len: usize);
    pub fn eval_gdi_push_function(ctx: *mut c_void, sfd: *mut c_void, name: *const u16, len: usize);
    pub fn eval_gdi_push_var_scoped_name(ctx: *mut c_void, name: *const u16, len: usize);
    pub fn eval_gdi_push_annex_b_name(ctx: *mut c_void, name: *const u16, len: usize);
    pub fn eval_gdi_push_lexical_binding(ctx: *mut c_void, name: *const u16, len: usize, is_constant: bool);

    pub fn rust_compile_regex(
        pattern_data: *const u16,
        pattern_len: usize,
        flags_data: *const u16,
        flags_len: usize,
        error_out: *mut *const std::os::raw::c_char,
    ) -> *mut c_void;

    pub fn rust_free_error_string(str: *const std::os::raw::c_char);

    pub fn rust_number_to_utf16(value: f64, buffer: *mut u16, buffer_len: usize) -> usize;
}

/// Create a SharedFunctionInstanceData from a FunctionData.
///
/// Computes has_simple_parameter_list, builds parameter name slices,
/// clones the AST into a Box, and calls rust_create_sfd.
///
/// Used by both `emit_new_function` in codegen.rs (for function
/// expressions/declarations) and `create_sfd_for_gdi` below (for
/// top-level GDI function initialization).
///
/// # Safety
/// `vm_ptr` and `source_code_ptr` must be valid pointers.
pub unsafe fn create_shared_function_data(
    func_data: &crate::ast::FunctionData,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    is_strict: bool,
    name_override: Option<&[u16]>,
) -> *mut c_void {
    use crate::ast::FunctionParameterBinding;

    let source_start = func_data.source_text_start as usize;
    let source_end = func_data.source_text_end as usize;
    let source_text_len = source_end - source_start;

    let (name_ptr, name_len) = if let Some(name) = name_override {
        (name.as_ptr(), name.len())
    } else if let Some(ref name_ident) = func_data.name {
        (name_ident.name.as_ptr(), name_ident.name.len())
    } else {
        (std::ptr::null(), 0)
    };

    let has_simple_parameter_list = func_data.parameters.iter().all(|p| {
        !p.is_rest
            && p.default_value.is_none()
            && matches!(p.binding, FunctionParameterBinding::Identifier(_))
    });

    let param_name_slices: Vec<FFIUtf16Slice> = if has_simple_parameter_list {
        func_data
            .parameters
            .iter()
            .map(|p| {
                if let FunctionParameterBinding::Identifier(ref id) = p.binding {
                    FFIUtf16Slice {
                        data: id.name.as_ptr(),
                        length: id.name.len(),
                    }
                } else {
                    unreachable!()
                }
            })
            .collect()
    } else {
        Vec::new()
    };

    let cloned = Box::new(func_data.clone());
    let rust_ast_ptr = Box::into_raw(cloned) as *mut c_void;

    let function_kind = func_data.kind as u8;
    let strict = func_data.is_strict_mode || is_strict;

    let sfd_ptr = rust_create_sfd(
        vm_ptr,
        source_code_ptr,
        name_ptr,
        name_len,
        function_kind,
        func_data.function_length,
        func_data.parameters.len() as u32,
        strict,
        func_data.is_arrow_function,
        has_simple_parameter_list,
        param_name_slices.as_ptr(),
        param_name_slices.len(),
        source_start,
        source_text_len,
        rust_ast_ptr,
        func_data.parsing_insights.uses_this,
        func_data.parsing_insights.uses_this_from_environment,
    );

    assert!(!sfd_ptr.is_null(), "create_shared_function_data: rust_create_sfd returned null");
    sfd_ptr
}

/// Create a SharedFunctionInstanceData for GDI use (no name override).
///
/// # Safety
/// `vm_ptr` and `source_code_ptr` must be valid pointers.
pub unsafe fn create_sfd_for_gdi(
    func_data: &crate::ast::FunctionData,
    vm_ptr: *mut c_void,
    source_code_ptr: *const c_void,
    is_strict: bool,
) -> *mut c_void {
    create_shared_function_data(func_data, vm_ptr, source_code_ptr, is_strict, None)
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

    // Collect class blueprint pointers
    let bp_ptrs: Vec<*mut c_void> = gen.class_blueprints.clone();

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
        gen.length_identifier.map_or(-1i32, |index| index.0 as i32),
        sfd_ptrs.as_ptr(),
        sfd_ptrs.len(),
        bp_ptrs.as_ptr(),
        bp_ptrs.len(),
        gen.compiled_regexes.as_ptr(),
        gen.compiled_regexes.len(),
    )
}

/// Convert a JS number to its UTF-16 string representation using the
/// ECMA-262 Number::toString algorithm (via C++ runtime).
pub fn js_number_to_utf16(value: f64) -> Vec<u16> {
    let mut buffer = [0u16; 64];
    let len = unsafe { rust_number_to_utf16(value, buffer.as_mut_ptr(), buffer.len()) };
    buffer[..len].to_vec()
}

/// Compile a regex pattern+flags using the C++ regex engine.
///
/// On success, returns an opaque handle to the compiled regex (a C++
/// RustCompiledRegex*). On failure, returns the error message.
pub fn compile_regex(pattern: &[u16], flags: &[u16]) -> Result<*mut c_void, String> {
    unsafe {
        let mut error: *const std::os::raw::c_char = std::ptr::null();
        let handle = rust_compile_regex(
            pattern.as_ptr(), pattern.len(),
            flags.as_ptr(), flags.len(),
            &mut error,
        );
        if error.is_null() {
            Ok(handle)
        } else {
            let msg = std::ffi::CStr::from_ptr(error).to_string_lossy().into_owned();
            rust_free_error_string(error);
            Err(msg)
        }
    }
}
