/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Utf16String.h>
#include <AK/Utf16View.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Bytecode/IdentifierTable.h>
#include <LibJS/Bytecode/PropertyKeyTable.h>
#include <LibJS/Bytecode/RegexTable.h>
#include <LibJS/Bytecode/StringTable.h>
#include <LibJS/BytecodeFactory.h>
#include <LibJS/Runtime/BigInt.h>
#include <LibJS/Runtime/PrimitiveString.h>
#include <LibJS/Runtime/VM.h>
#include <LibJS/SourceCode.h>

static Utf16View view_from_ffi(FFIUtf16Slice slice)
{
    return Utf16View { reinterpret_cast<char16_t const*>(slice.data), slice.length };
}

static Utf16String utf16_from_ffi(FFIUtf16Slice slice)
{
    return Utf16String::from_utf16(view_from_ffi(slice));
}

static Utf16FlyString utf16_fly_from_ffi(FFIUtf16Slice slice)
{
    return Utf16FlyString::from_utf16(view_from_ffi(slice));
}

static JS::Value decode_constant(JS::VM& vm, uint8_t const*& cursor, uint8_t const* end)
{
    VERIFY(cursor < end);
    auto tag = *cursor++;

    switch (tag) {
    case CONSTANT_TAG_NUMBER: {
        VERIFY(cursor + 8 <= end);
        double value;
        memcpy(&value, cursor, 8);
        cursor += 8;
        return JS::Value(value);
    }
    case CONSTANT_TAG_BOOLEAN_TRUE:
        return JS::Value(true);
    case CONSTANT_TAG_BOOLEAN_FALSE:
        return JS::Value(false);
    case CONSTANT_TAG_NULL:
        return JS::js_null();
    case CONSTANT_TAG_UNDEFINED:
        return JS::js_undefined();
    case CONSTANT_TAG_EMPTY:
        return JS::js_special_empty_value();
    case CONSTANT_TAG_STRING: {
        VERIFY(cursor + 4 <= end);
        uint32_t len;
        memcpy(&len, cursor, 4);
        cursor += 4;
        VERIFY(cursor + len * 2 <= end);
        auto str = Utf16String::from_utf16(Utf16View(reinterpret_cast<char16_t const*>(cursor), len));
        cursor += len * 2;
        return JS::PrimitiveString::create(vm, move(str));
    }
    case CONSTANT_TAG_BIGINT: {
        VERIFY(cursor + 4 <= end);
        uint32_t len;
        memcpy(&len, cursor, 4);
        cursor += 4;
        VERIFY(cursor + len <= end);
        auto ascii = StringView(reinterpret_cast<char const*>(cursor), len);
        cursor += len;
        auto integer = MUST(Crypto::SignedBigInteger::from_base(10, ascii));
        return JS::BigInt::create(vm, move(integer));
    }
    default:
        VERIFY_NOT_REACHED();
    }
}

extern "C" void* rust_create_executable(
    void* vm_ptr,
    void* source_code_ptr,
    uint8_t const* bytecode,
    size_t bytecode_len,
    FFIUtf16Slice const* identifier_table_entries,
    size_t identifier_count,
    FFIUtf16Slice const* property_key_table_entries,
    size_t property_key_count,
    FFIUtf16Slice const* string_table_entries,
    size_t string_count,
    uint8_t const* constants_data,
    size_t constants_data_len,
    size_t constants_count,
    FFIExceptionHandler const* exception_handlers,
    size_t exception_handler_count,
    FFISourceMapEntry const* source_map,
    size_t source_map_count,
    size_t const* basic_block_offsets,
    size_t basic_block_count,
    FFIUtf16Slice const* local_var_names,
    size_t local_var_count,
    uint32_t property_lookup_cache_count,
    uint32_t global_variable_cache_count,
    uint32_t template_object_cache_count,
    uint32_t object_shape_cache_count,
    uint32_t number_of_registers,
    bool is_strict)
{
    auto& vm = *static_cast<JS::VM*>(vm_ptr);
    auto& source_code = *static_cast<JS::SourceCode const*>(source_code_ptr);

    // Build bytecode vector
    Vector<u8> bytecode_vec;
    bytecode_vec.append(bytecode, bytecode_len);

    // Build identifier table
    auto ident_table = make<JS::Bytecode::IdentifierTable>();
    for (size_t i = 0; i < identifier_count; ++i) {
        ident_table->insert(utf16_fly_from_ffi(identifier_table_entries[i]));
    }

    // Build property key table
    auto prop_key_table = make<JS::Bytecode::PropertyKeyTable>();
    for (size_t i = 0; i < property_key_count; ++i) {
        prop_key_table->insert(utf16_fly_from_ffi(property_key_table_entries[i]));
    }

    // Build string table
    auto str_table = make<JS::Bytecode::StringTable>();
    for (size_t i = 0; i < string_count; ++i) {
        str_table->insert(utf16_from_ffi(string_table_entries[i]));
    }

    // Build regex table (empty for now — regex compilation needs more FFI work)
    auto regex_tbl = make<JS::Bytecode::RegexTable>();

    // Decode constants
    Vector<JS::Value> constants_vec;
    constants_vec.ensure_capacity(constants_count);
    auto const* cursor = constants_data;
    auto const* end = constants_data + constants_data_len;
    for (size_t i = 0; i < constants_count; ++i) {
        constants_vec.append(decode_constant(vm, cursor, end));
    }
    VERIFY(cursor == end);

    // Create executable
    auto executable = vm.heap().allocate<JS::Bytecode::Executable>(
        move(bytecode_vec),
        move(ident_table),
        move(prop_key_table),
        move(str_table),
        move(regex_tbl),
        move(constants_vec),
        source_code,
        property_lookup_cache_count,
        global_variable_cache_count,
        template_object_cache_count,
        object_shape_cache_count,
        number_of_registers,
        is_strict ? JS::Strict::Yes : JS::Strict::No);

    // Set exception handlers
    for (size_t i = 0; i < exception_handler_count; ++i) {
        executable->exception_handlers.append({
            exception_handlers[i].start_offset,
            exception_handlers[i].end_offset,
            exception_handlers[i].handler_offset,
        });
    }

    // Set source map
    for (size_t i = 0; i < source_map_count; ++i) {
        executable->source_map.append({
            source_map[i].bytecode_offset,
            { source_map[i].source_start, source_map[i].source_end },
        });
    }

    // Set basic block offsets
    for (size_t i = 0; i < basic_block_count; ++i) {
        executable->basic_block_start_offsets.append(basic_block_offsets[i]);
    }

    // Set local variable names
    for (size_t i = 0; i < local_var_count; ++i) {
        executable->local_variable_names.append({
            .name = utf16_fly_from_ffi(local_var_names[i]),
            .declaration_kind = JS::LocalVariable::DeclarationKind::Var,
        });
    }

    // Set layout indices
    executable->local_index_base = number_of_registers;
    executable->argument_index_base = number_of_registers + local_var_count + constants_count;
    executable->registers_and_locals_count = number_of_registers + local_var_count;
    executable->registers_and_locals_and_constants_count = number_of_registers + local_var_count + constants_count;

    return executable.ptr();
}
