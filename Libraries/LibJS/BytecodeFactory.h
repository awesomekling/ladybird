/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#ifdef ENABLE_RUST_PARSER

#    include <stddef.h>
#    include <stdint.h>

// FFI types for creating a Bytecode::Executable from Rust.
//
// The Rust bytecode generator assembles instructions into a byte buffer
// matching C++ layout. This FFI layer creates the C++ Executable from
// that data.

// Constant value tags (matches Rust ConstantValue enum discriminants)
#    define CONSTANT_TAG_NUMBER 0
#    define CONSTANT_TAG_BOOLEAN_TRUE 1
#    define CONSTANT_TAG_BOOLEAN_FALSE 2
#    define CONSTANT_TAG_NULL 3
#    define CONSTANT_TAG_UNDEFINED 4
#    define CONSTANT_TAG_EMPTY 5
#    define CONSTANT_TAG_STRING 6
#    define CONSTANT_TAG_BIGINT 7

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
    uint8_t literal_value_kind; // 0=none, 1=number, 2=boolean_true, 3=boolean_false, 4=null, 5=string
    double literal_value_number;
    uint16_t const* literal_value_string;
    size_t literal_value_string_len;
};

#    ifdef __cplusplus
extern "C" {
#    endif

// Callback for reporting parse errors from Rust.
// message is UTF-8, line and column are 1-based.
typedef void (*RustParseErrorCallback)(void* ctx, char const* message, size_t message_len, uint32_t line, uint32_t column);

// Parse, compile, and extract GDI metadata for a script using the Rust
// parser. Populates gdi_context (a ScriptGdiBuilder*) via callbacks.
// On parse failure, calls error_callback for each error, then returns nullptr.
// Returns a Bytecode::Executable* cast to void*, or nullptr on failure.
void* rust_compile_script(
    uint16_t const* source,
    size_t source_len,
    void* vm_ptr,
    void const* source_code_ptr,
    void* gdi_context,
    bool dump_ast,
    bool use_color,
    void* error_context,
    RustParseErrorCallback error_callback,
    uint8_t** ast_dump_output,
    size_t* ast_dump_output_len);

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
    // Regex table: array of pre-compiled RustCompiledRegex* (ownership transfers)
    void* const* compiled_regexes,
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
    size_t source_text_offset,
    size_t source_text_len,
    // Opaque Rust AST pointer (Box<FunctionData>)
    void* rust_function_ast,
    // Parsing insights needed before lazy compilation
    bool uses_this,
    bool uses_this_from_environment);

// Set class_field_initializer_name on a SharedFunctionInstanceData.
// Called after rust_create_sfd for class field initializer functions.
void rust_sfd_set_class_field_initializer_name(
    void* sfd_ptr,
    uint16_t const* name,
    size_t name_len,
    bool is_private);

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
    // VM pointer for creating GC objects (e.g. PrimitiveString)
    void* vm_ptr,
    // Source code object for substring_view
    void const* source_code_ptr,
    // Class name (empty for anonymous)
    uint16_t const* name,
    size_t name_len,
    // Source text of the entire class (for Function.prototype.toString)
    size_t source_text_offset,
    size_t source_text_len,
    // Index into shared_function_data for the constructor
    uint32_t constructor_sfd_index,
    bool has_super_class,
    bool has_name,
    // Array of class element descriptors
    FFIClassElement const* elements,
    size_t element_count);

// Callbacks used by rust_compile_script to populate GDI metadata.
void script_gdi_push_lexical_name(void* ctx, uint16_t const* name, size_t len);
void script_gdi_push_var_name(void* ctx, uint16_t const* name, size_t len);
void script_gdi_push_function(void* ctx, void* sfd, uint16_t const* name, size_t len);
void script_gdi_push_var_scoped_name(void* ctx, uint16_t const* name, size_t len);
void script_gdi_push_annex_b_name(void* ctx, uint16_t const* name, size_t len);
void script_gdi_push_lexical_binding(void* ctx, uint16_t const* name, size_t len, bool is_constant);

// Parse, compile, and extract EDI metadata for eval using the Rust
// parser. Populates gdi_context (an EvalGdiBuilder*) via callbacks.
// Returns a Bytecode::Executable* cast to void*, or nullptr on failure.
void* rust_compile_eval(
    uint16_t const* source,
    size_t source_len,
    void* vm_ptr,
    void const* source_code_ptr,
    void* gdi_context,
    bool starts_in_strict_mode,
    bool in_eval_function_context,
    bool allow_super_property_lookup,
    bool allow_super_constructor_call,
    bool in_class_field_initializer,
    void* error_context,
    RustParseErrorCallback error_callback,
    uint8_t** ast_dump_output,
    size_t* ast_dump_output_len);

// Parse and compile a dynamically-created function (new Function()).
// Validates parameters and body separately per spec, then parses the
// full synthetic source to create a SharedFunctionInstanceData.
//
// Returns a SharedFunctionInstanceData* cast to void*, or nullptr on
// parse failure (caller should throw SyntaxError).
//
// function_kind: 0=Normal, 1=Generator, 2=Async, 3=AsyncGenerator
void* rust_compile_dynamic_function(
    uint16_t const* full_source,
    size_t full_source_len,
    uint16_t const* params_source,
    size_t params_source_len,
    uint16_t const* body_source,
    size_t body_source_len,
    void* vm_ptr,
    void const* source_code_ptr,
    uint8_t function_kind,
    void* error_context,
    RustParseErrorCallback error_callback);

// Callbacks used by rust_compile_eval to populate EDI metadata.
void eval_gdi_set_strict(void* ctx, bool is_strict);
void eval_gdi_push_var_name(void* ctx, uint16_t const* name, size_t len);
void eval_gdi_push_function(void* ctx, void* sfd, uint16_t const* name, size_t len);
void eval_gdi_push_var_scoped_name(void* ctx, uint16_t const* name, size_t len);
void eval_gdi_push_annex_b_name(void* ctx, uint16_t const* name, size_t len);
void eval_gdi_push_lexical_binding(void* ctx, uint16_t const* name, size_t len, bool is_constant);

// Compile a regex pattern+flags. On success, returns a heap-allocated
// opaque object (RustCompiledRegex*) and sets *error_out to nullptr.
// On failure, returns nullptr and sets *error_out to a heap-allocated
// error string (caller must free with rust_free_error_string).
// Successful results must be freed with rust_free_compiled_regex or
// passed to rust_create_executable (which takes ownership).
void* rust_compile_regex(
    uint16_t const* pattern_data, size_t pattern_len, uint16_t const* flags_data, size_t flags_len, char const** error_out);
void rust_free_compiled_regex(void* ptr);
void rust_free_error_string(char const* str);

// Convert a JS number to its UTF-16 string representation using the
// ECMA-262 Number::toString algorithm. Writes up to buffer_len code
// units into buffer and returns the actual length.
size_t rust_number_to_utf16(double value, uint16_t* buffer, size_t buffer_len);

// Free a string allocated by Rust (e.g. AST dump output).
void rust_free_string(uint8_t* ptr, size_t len);

#    ifdef __cplusplus
}
#    endif

#endif // ENABLE_RUST_PARSER
