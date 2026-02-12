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

// Class element descriptor for FFI (matches ClassElementDescriptor::Kind).
// Kind values: 0=Method, 1=Getter, 2=Setter, 3=Field, 4=StaticInitializer
struct FFIClassElement {
    uint8_t kind;
    bool is_static;
    bool is_private;
    uint16_t const* private_identifier;
    size_t private_identifier_len;
    int32_t shared_function_data_index; // -1 for none
    bool has_initializer;
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
    bool is_strict,
    // Length identifier: PropertyKeyTableIndex for "length", or -1
    int32_t length_identifier,
    // Shared function data (inner functions)
    // Array of SharedFunctionInstanceData* cast to void*.
    void const* const* shared_function_data,
    size_t shared_function_data_count,
    // Class blueprints (heap-allocated ClassBlueprint*, ownership transfers)
    void* const* class_blueprints,
    size_t class_blueprint_count,
    // Regex table: parallel arrays of pattern and flags strings
    FFIUtf16Slice const* regex_patterns,
    FFIUtf16Slice const* regex_flags,
    size_t regex_count);

// Create a SharedFunctionInstanceData by re-parsing a function's source text
// with the C++ parser. The function body is NOT compiled — it will compile
// lazily on first call.
//
// Returns a SharedFunctionInstanceData* cast to void*.
void* rust_create_shared_function_data(
    void* vm_ptr,
    void const* source_code_ptr,
    // Source text of the function (e.g. "function foo(a) { return a; }")
    uint16_t const* source_text,
    size_t source_text_len,
    // Function name override (empty for anonymous)
    uint16_t const* name,
    size_t name_len,
    // Whether to parse in strict mode context
    bool strict_mode);

// Create a SharedFunctionInstanceData from pre-computed metadata (Rust pipeline).
// Stores an opaque Rust AST pointer for lazy compilation.
//
// Returns a SharedFunctionInstanceData* cast to void*.
void* rust_create_sfd(
    void* vm_ptr,
    void const* source_code_ptr,
    // Function name
    uint16_t const* name,
    size_t name_len,
    // Metadata
    uint8_t function_kind,
    int32_t function_length,
    uint32_t formal_parameter_count,
    bool strict,
    bool is_arrow,
    bool has_simple_parameter_list,
    // Parameter names for mapped arguments (only for simple parameter lists)
    FFIUtf16Slice const* param_names,
    size_t param_name_count,
    // Source text range (for Function.prototype.toString)
    uint16_t const* source_text,
    size_t source_text_len,
    // Opaque Rust AST pointer (Box<FunctionData>)
    void* rust_function_ast);

// Compile a function body using the Rust pipeline.
// Takes ownership of the Rust AST (frees it after compilation).
//
// Writes FDI runtime metadata to the SFD via the sfd_ptr parameter.
// Returns a Bytecode::Executable* cast to void*, or nullptr on failure.
void* rust_compile_function(
    void* vm_ptr,
    void const* source_code_ptr,
    uint16_t const* source,
    size_t source_len,
    void* sfd_ptr,
    void* rust_function_ast);

// Free a Rust Box<FunctionData> (called from SFD destructor).
void rust_free_function_ast(void* ast);

// Set FDI runtime metadata on a SharedFunctionInstanceData.
// Called from Rust after compiling a function body.
void rust_sfd_set_metadata(
    void* sfd_ptr,
    bool uses_this,
    bool function_environment_needed,
    size_t function_environment_bindings_count,
    bool might_need_arguments_object,
    bool contains_direct_call_to_eval);

// Create a ClassBlueprint on the heap. Ownership transfers to the
// caller; pass the pointer to rust_create_executable which will move
// the blueprint into the Executable.
//
// Returns a heap-allocated ClassBlueprint* cast to void*.
void* rust_create_class_blueprint(
    // Class name (empty for anonymous)
    uint16_t const* name,
    size_t name_len,
    // Source text of the entire class (for Function.prototype.toString)
    uint16_t const* source_text,
    size_t source_text_len,
    // Index into shared_function_data for the constructor
    uint32_t constructor_sfd_index,
    bool has_super_class,
    bool has_name,
    // Array of class element descriptors
    FFIClassElement const* elements,
    size_t element_count);

#ifdef __cplusplus
}
#endif
