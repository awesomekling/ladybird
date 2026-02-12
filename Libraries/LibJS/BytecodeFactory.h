/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <stddef.h>
#include <stdint.h>

// FFI types for creating a Bytecode::Executable from Rust.
//
// The Rust bytecode generator assembles instructions into a byte buffer
// matching C++ layout. This FFI layer creates the C++ Executable from
// that data.

// Constant value tags (matches Rust ConstantValue enum discriminants)
#define CONSTANT_TAG_NUMBER 0
#define CONSTANT_TAG_BOOLEAN_TRUE 1
#define CONSTANT_TAG_BOOLEAN_FALSE 2
#define CONSTANT_TAG_NULL 3
#define CONSTANT_TAG_UNDEFINED 4
#define CONSTANT_TAG_EMPTY 5
#define CONSTANT_TAG_STRING 6
#define CONSTANT_TAG_BIGINT 7

struct FFIExceptionHandler {
    uint32_t start_offset;
    uint32_t end_offset;
    uint32_t handler_offset;
};

struct FFISourceMapEntry {
    uint32_t bytecode_offset;
    uint32_t source_start;
    uint32_t source_end;
};

// A UTF-16 string slice (pointer + length).
struct FFIUtf16Slice {
    uint16_t const* data;
    size_t length;
};

#ifdef __cplusplus
extern "C" {
#endif

// Parse and compile a JavaScript program using the Rust parser and
// bytecode generator. Returns a Bytecode::Executable* cast to void*,
// or nullptr on failure.
void* rust_compile_program(
    uint16_t const* source,
    size_t source_len,
    void* vm_ptr,
    void const* source_code_ptr,
    uint8_t program_type,
    bool starts_in_strict_mode,
    bool initiated_by_eval,
    bool in_eval_function_context,
    bool allow_super_property_lookup,
    bool allow_super_constructor_call,
    bool in_class_field_initializer);

// Create a C++ Bytecode::Executable from assembled Rust bytecode data.
//
// The source_code parameter is a SourceCode const* cast to void*.
// Returns a GC::Ptr<Executable> cast to void*, or nullptr on failure.
void* rust_create_executable(
    void* vm_ptr,
    void* source_code_ptr,
    // Bytecode
    uint8_t const* bytecode,
    size_t bytecode_len,
    // Tables: arrays of UTF-16 string slices
    FFIUtf16Slice const* identifier_table,
    size_t identifier_count,
    FFIUtf16Slice const* property_key_table,
    size_t property_key_count,
    FFIUtf16Slice const* string_table,
    size_t string_count,
    // Constants: tagged byte array
    // Format: each constant is [u8 tag] followed by tag-specific payload:
    //   NUMBER: 8 bytes (f64 le)
    //   BOOLEAN_TRUE/FALSE/NULL/UNDEFINED/EMPTY: 0 bytes
    //   STRING: 4 bytes (u32 le length) + length*2 bytes (UTF-16 le)
    //   BIGINT: 4 bytes (u32 le length) + length bytes (ASCII)
    uint8_t const* constants_data,
    size_t constants_data_len,
    size_t constants_count,
    // Exception handlers
    FFIExceptionHandler const* exception_handlers,
    size_t exception_handler_count,
    // Source map
    FFISourceMapEntry const* source_map,
    size_t source_map_count,
    // Basic block start offsets
    size_t const* basic_block_offsets,
    size_t basic_block_count,
    // Local variable names
    FFIUtf16Slice const* local_var_names,
    size_t local_var_count,
    // Cache counts
    uint32_t property_lookup_cache_count,
    uint32_t global_variable_cache_count,
    uint32_t template_object_cache_count,
    uint32_t object_shape_cache_count,
    // Register and mode
    uint32_t number_of_registers,
    bool is_strict);

#ifdef __cplusplus
}
#endif
