/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Utf16String.h>
#include <AK/Utf16View.h>
#include <LibJS/AST.h>
#include <LibJS/Bytecode/ClassBlueprint.h>
#include <LibJS/Bytecode/Executable.h>
#include <LibJS/Bytecode/IdentifierTable.h>
#include <LibJS/Bytecode/PropertyKeyTable.h>
#include <LibJS/Bytecode/RegexTable.h>
#include <LibJS/Bytecode/StringTable.h>
#include <LibJS/BytecodeFactory.h>
#include <LibJS/Lexer.h>
#include <LibJS/Parser.h>
#include <LibJS/Runtime/BigInt.h>
#include <LibJS/Runtime/PrimitiveString.h>
#include <LibJS/Runtime/RegExpObject.h>
#include <LibJS/Runtime/SharedFunctionInstanceData.h>
#include <LibJS/Runtime/VM.h>
#include <LibJS/SourceCode.h>

struct RustCompiledRegex {
    regex::Parser::Result parsed_regex;
    String parsed_pattern;
    regex::RegexOptions<ECMAScriptFlags> flags;
};

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
        auto integer = [&] {
            if (len >= 3 && ascii[0] == '0') {
                if (ascii[1] == 'x' || ascii[1] == 'X')
                    return MUST(Crypto::SignedBigInteger::from_base(16, ascii.substring_view(2)));
                if (ascii[1] == 'o' || ascii[1] == 'O')
                    return MUST(Crypto::SignedBigInteger::from_base(8, ascii.substring_view(2)));
                if (ascii[1] == 'b' || ascii[1] == 'B')
                    return MUST(Crypto::SignedBigInteger::from_base(2, ascii.substring_view(2)));
            }
            return MUST(Crypto::SignedBigInteger::from_base(10, ascii));
        }();
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
    bool is_strict,
    int32_t length_identifier, // -1 for none
    void const* const* shared_function_data,
    size_t shared_function_data_count,
    void* const* class_blueprints,
    size_t class_blueprint_count,
    void* const* compiled_regexes,
    size_t regex_count)
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

    // Build regex table from pre-compiled regex objects
    auto regex_tbl = make<JS::Bytecode::RegexTable>();
    for (size_t i = 0; i < regex_count; ++i) {
        auto* cr = static_cast<RustCompiledRegex*>(compiled_regexes[i]);
        regex_tbl->insert(JS::Bytecode::ParsedRegex { move(cr->parsed_regex), move(cr->parsed_pattern), cr->flags });
        delete cr;
    }

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

    // Set length identifier (for GetLength optimization)
    if (length_identifier >= 0)
        executable->length_identifier = JS::Bytecode::PropertyKeyTableIndex(static_cast<u32>(length_identifier));

    // Set shared function data (inner function definitions)
    for (size_t i = 0; i < shared_function_data_count; ++i) {
        auto* data = const_cast<JS::SharedFunctionInstanceData*>(
            static_cast<JS::SharedFunctionInstanceData const*>(shared_function_data[i]));
        executable->shared_function_data.append(data);
    }

    // Set class blueprints (move from heap-allocated objects)
    for (size_t i = 0; i < class_blueprint_count; ++i) {
        auto* bp = static_cast<JS::Bytecode::ClassBlueprint*>(class_blueprints[i]);
        executable->class_blueprints.append(move(*bp));
        delete bp;
    }

    return executable.ptr();
}

extern "C" void* rust_create_shared_function_data(
    void* vm_ptr,
    void const* source_code_ptr,
    uint16_t const* source_text,
    size_t source_text_len,
    uint16_t const* name,
    size_t name_len,
    bool strict_mode)
{
    auto& vm = *static_cast<JS::VM*>(vm_ptr);
    auto& original_source_code = *static_cast<JS::SourceCode const*>(source_code_ptr);

    // Re-parse the source text to get a FunctionNode.
    // We keep the parsed program alive so the function node pointer stays valid.
    //
    // Try two wrapping strategies:
    //   1. `(<source>)\n`          — works for function expressions, declarations, arrows
    //   2. `(function <source>)\n` — works for class methods (no "function" keyword in source)
    JS::FunctionNode const* function_node = nullptr;
    NonnullRefPtr<JS::Program> program = [&]() {
        // Strategy 1: wrap in `(<source>)`
        Vector<char16_t> wrapped;
        wrapped.append(u'(');
        wrapped.append(reinterpret_cast<char16_t const*>(source_text), source_text_len);
        wrapped.append(u')');
        wrapped.append(u'\n');

        auto wrapped_utf16 = Utf16String::from_utf16(Utf16View(wrapped.data(), wrapped.size()));
        auto temp_source_code = JS::SourceCode::create(""_string, move(wrapped_utf16));
        auto parser = JS::Parser(JS::Lexer(temp_source_code));
        return parser.parse_program(strict_mode);
    }();

    for (auto const& child : program->children()) {
        if (is<JS::ExpressionStatement>(*child)) {
            auto const& expr_stmt = static_cast<JS::ExpressionStatement const&>(*child);
            auto const& expr = expr_stmt.expression();
            if (is<JS::FunctionExpression>(expr))
                function_node = static_cast<JS::FunctionExpression const*>(&expr);
        }
    }

    // Strategy 2: wrap in `(function <source>)` (for class methods without "function" keyword)
    if (!function_node) {
        Vector<char16_t> wrapped;
        wrapped.append(u'(');
        wrapped.append(u'f');
        wrapped.append(u'u');
        wrapped.append(u'n');
        wrapped.append(u'c');
        wrapped.append(u't');
        wrapped.append(u'i');
        wrapped.append(u'o');
        wrapped.append(u'n');
        wrapped.append(u' ');
        wrapped.append(reinterpret_cast<char16_t const*>(source_text), source_text_len);
        wrapped.append(u')');
        wrapped.append(u'\n');

        auto wrapped_utf16 = Utf16String::from_utf16(Utf16View(wrapped.data(), wrapped.size()));
        auto temp_source_code = JS::SourceCode::create(""_string, move(wrapped_utf16));
        auto parser = JS::Parser(JS::Lexer(temp_source_code));
        program = parser.parse_program(strict_mode);

        for (auto const& child : program->children()) {
            if (is<JS::ExpressionStatement>(*child)) {
                auto const& expr_stmt = static_cast<JS::ExpressionStatement const&>(*child);
                auto const& expr = expr_stmt.expression();
                if (is<JS::FunctionExpression>(expr))
                    function_node = static_cast<JS::FunctionExpression const*>(&expr);
            }
        }
    }

    // Strategy 3: wrap in `({ <source> })` (for getters/setters: "get x() {}" or "set x(v) {}")
    if (!function_node) {
        Vector<char16_t> wrapped;
        wrapped.append(u'(');
        wrapped.append(u'{');
        wrapped.append(reinterpret_cast<char16_t const*>(source_text), source_text_len);
        wrapped.append(u'}');
        wrapped.append(u')');
        wrapped.append(u'\n');

        auto wrapped_utf16 = Utf16String::from_utf16(Utf16View(wrapped.data(), wrapped.size()));
        auto temp_source_code = JS::SourceCode::create(""_string, move(wrapped_utf16));
        auto parser = JS::Parser(JS::Lexer(temp_source_code));
        program = parser.parse_program(strict_mode);

        // Walk children looking for ObjectExpression → ObjectProperty → FunctionExpression
        for (auto const& child : program->children()) {
            if (!is<JS::ExpressionStatement>(*child))
                continue;
            auto const& expr = static_cast<JS::ExpressionStatement const&>(*child).expression();
            if (!is<JS::ObjectExpression>(expr))
                continue;
            auto const& obj = static_cast<JS::ObjectExpression const&>(expr);
            for (auto const& prop : obj.properties()) {
                if (is<JS::FunctionExpression>(prop->value())) {
                    function_node = static_cast<JS::FunctionExpression const*>(&prop->value());
                    break;
                }
            }
        }
    }

    if (!function_node) {
        dbgln("rust_create_shared_function_data: failed to extract function node from re-parsed source");
        return nullptr;
    }

    // Create SharedFunctionInstanceData from the extracted FunctionNode.
    auto fn_name = name_len > 0
        ? Utf16FlyString::from_utf16(Utf16View(reinterpret_cast<char16_t const*>(name), name_len))
        : function_node->name();
    auto shared = JS::SharedFunctionInstanceData::create_for_function_node(vm, *function_node, move(fn_name));

    // Fix up source_code to point to the original source (not our temp wrapper),
    // so that Function.prototype.toString() returns the correct source text.
    shared->m_source_code = &original_source_code;
    auto const& code_view = original_source_code.code_view();
    // The source_text pointer is into the original source buffer (a UTF-16 array
    // passed from Rust). Compute the offset and create a view into the SourceCode.
    auto original_start = reinterpret_cast<uint16_t const*>(
        code_view.is_ascii() ? static_cast<void const*>(code_view.ascii_span().data())
                             : static_cast<void const*>(code_view.utf16_span().data()));
    if (source_text >= original_start && source_text + source_text_len <= original_start + code_view.length_in_code_units()) {
        auto offset = source_text - original_start;
        shared->m_source_text = code_view.substring_view(offset, source_text_len);
    }

    return shared.ptr();
}

extern "C" void* rust_create_sfd(
    void* vm_ptr,
    void const* source_code_ptr,
    uint16_t const* name,
    size_t name_len,
    uint8_t function_kind,
    int32_t function_length,
    uint32_t formal_parameter_count,
    bool strict,
    bool is_arrow,
    bool has_simple_parameter_list,
    FFIUtf16Slice const* param_names,
    size_t param_name_count,
    size_t source_text_offset,
    size_t source_text_len,
    void* rust_function_ast,
    bool uses_this,
    bool uses_this_from_environment)
{
    auto& vm = *static_cast<JS::VM*>(vm_ptr);
    auto& source_code = *static_cast<JS::SourceCode const*>(source_code_ptr);

    auto fn_name = name_len > 0
        ? Utf16FlyString::from_utf16(Utf16View(reinterpret_cast<char16_t const*>(name), name_len))
        : Utf16FlyString {};

    Vector<Utf16FlyString> mapped_param_names;
    if (has_simple_parameter_list) {
        mapped_param_names.ensure_capacity(param_name_count);
        for (size_t i = 0; i < param_name_count; ++i)
            mapped_param_names.append(utf16_fly_from_ffi(param_names[i]));
    }

    auto shared = vm.heap().allocate<JS::SharedFunctionInstanceData>(
        vm,
        static_cast<JS::FunctionKind>(function_kind),
        move(fn_name),
        function_length,
        formal_parameter_count,
        strict,
        is_arrow,
        has_simple_parameter_list,
        move(mapped_param_names),
        rust_function_ast);

    // Set parsing insights that must be available before lazy compilation.
    shared->m_uses_this = uses_this;
    if (uses_this_from_environment)
        shared->m_function_environment_needed = true;

    // Set source text as a view into the original source code.
    shared->m_source_code = &source_code;
    if (source_text_len > 0) {
        auto const& code_view = source_code.code_view();
        shared->m_source_text = code_view.substring_view(source_text_offset, source_text_len);
    }

    return shared.ptr();
}

extern "C" void rust_sfd_set_metadata(
    void* sfd_ptr,
    bool uses_this,
    bool function_environment_needed,
    size_t function_environment_bindings_count,
    bool might_need_arguments_object,
    bool contains_direct_call_to_eval)
{
    auto& shared = *static_cast<JS::SharedFunctionInstanceData*>(sfd_ptr);
    shared.m_uses_this = uses_this;
    shared.m_function_environment_needed = function_environment_needed;
    shared.m_function_environment_bindings_count = function_environment_bindings_count;
    shared.m_might_need_arguments_object = might_need_arguments_object;
    shared.m_contains_direct_call_to_eval = contains_direct_call_to_eval;
}

extern "C" void rust_sfd_set_class_field_initializer_name(
    void* sfd_ptr,
    uint16_t const* name,
    size_t name_len,
    bool is_private)
{
    auto& shared = *static_cast<JS::SharedFunctionInstanceData*>(sfd_ptr);
    auto utf16_name = Utf16FlyString::from_utf16(Utf16View(reinterpret_cast<char16_t const*>(name), name_len));
    if (is_private) {
        shared.m_class_field_initializer_name = JS::PrivateName(0, utf16_name);
    } else {
        shared.m_class_field_initializer_name = JS::PropertyKey(utf16_name.to_utf16_string());
    }
}

extern "C" void* rust_create_class_blueprint(
    void* vm_ptr,
    void const* source_code_ptr,
    uint16_t const* name,
    size_t name_len,
    size_t source_text_offset,
    size_t source_text_len,
    uint32_t constructor_sfd_index,
    bool has_super_class,
    bool has_name,
    FFIClassElement const* elements,
    size_t element_count)
{
    auto* blueprint = new JS::Bytecode::ClassBlueprint();
    blueprint->constructor_shared_function_data_index = constructor_sfd_index;
    blueprint->has_super_class = has_super_class;
    blueprint->has_name = has_name;

    if (name_len > 0)
        blueprint->name = Utf16FlyString::from_utf16(Utf16View(reinterpret_cast<char16_t const*>(name), name_len));

    // Store source text as a view into the SourceCode buffer.
    if (source_text_len > 0) {
        auto& source_code = *static_cast<JS::SourceCode const*>(source_code_ptr);
        auto const& code_view = source_code.code_view();
        blueprint->source_text = code_view.substring_view(source_text_offset, source_text_len);
    }

    for (size_t i = 0; i < element_count; ++i) {
        auto const& elem = elements[i];
        JS::Bytecode::ClassElementDescriptor desc;
        desc.kind = static_cast<JS::Bytecode::ClassElementDescriptor::Kind>(elem.kind);
        desc.is_static = elem.is_static;
        desc.is_private = elem.is_private;
        if (elem.private_identifier_len > 0)
            desc.private_identifier = Utf16FlyString::from_utf16(Utf16View(reinterpret_cast<char16_t const*>(elem.private_identifier), elem.private_identifier_len));
        if (elem.shared_function_data_index >= 0)
            desc.shared_function_data_index = static_cast<u32>(elem.shared_function_data_index);
        desc.has_initializer = elem.has_initializer;
        switch (elem.literal_value_kind) {
        case 0: // none
            break;
        case 1: // number
            desc.literal_value = JS::Value(elem.literal_value_number);
            break;
        case 2: // boolean true
            desc.literal_value = JS::Value(true);
            break;
        case 3: // boolean false
            desc.literal_value = JS::Value(false);
            break;
        case 4: // null
            desc.literal_value = JS::js_null();
            break;
        case 5: { // string
            auto& vm = *static_cast<JS::VM*>(vm_ptr);
            auto str_view = Utf16View(reinterpret_cast<char16_t const*>(elem.literal_value_string), elem.literal_value_string_len);
            desc.literal_value = JS::Value(JS::PrimitiveString::create(vm, str_view));
            break;
        }
        }
        blueprint->elements.append(desc);
    }

    return blueprint;
}

// Compile a regex pattern+flags. On success, returns a heap-allocated
// RustCompiledRegex* (cast to void*) and sets *error_out to nullptr.
// On failure, returns nullptr and sets *error_out to a heap-allocated
// error string (caller must free with rust_free_error_string).
extern "C" void* rust_compile_regex(
    uint16_t const* pattern_data, size_t pattern_len,
    uint16_t const* flags_data, size_t flags_len,
    char const** error_out)
{
    *error_out = nullptr;
    auto pattern = Utf16View { reinterpret_cast<char16_t const*>(pattern_data), pattern_len };
    auto flags_view = Utf16View { reinterpret_cast<char16_t const*>(flags_data), flags_len };
    auto parsed_flags = JS::regex_flags_from_string(flags_view);
    auto ecma_flags = parsed_flags.is_error() ? regex::RegexOptions<ECMAScriptFlags> {} : parsed_flags.release_value();
    auto parsed_pattern = JS::parse_regex_pattern(pattern, ecma_flags.has_flag_set(ECMAScriptFlags::Unicode), ecma_flags.has_flag_set(ECMAScriptFlags::UnicodeSets));
    if (parsed_pattern.is_error()) {
        auto msg = MUST(String::formatted("RegExp compile error: {}", parsed_pattern.release_error().error));
        auto* buf = static_cast<char*>(malloc(msg.byte_count() + 1));
        memcpy(buf, msg.bytes().data(), msg.byte_count());
        buf[msg.byte_count()] = '\0';
        *error_out = buf;
        return nullptr;
    }
    auto pattern_str = parsed_pattern.release_value();
    auto parsed_regex = Regex<ECMA262>::parse_pattern(pattern_str, ecma_flags);
    if (parsed_regex.error != regex::Error::NoError) {
        auto error_string = Regex<ECMA262>(parsed_regex, ""sv, ecma_flags).error_string();
        auto msg = MUST(String::formatted("RegExp compile error: {}", error_string));
        auto* buf = static_cast<char*>(malloc(msg.byte_count() + 1));
        memcpy(buf, msg.bytes().data(), msg.byte_count());
        buf[msg.byte_count()] = '\0';
        *error_out = buf;
        return nullptr;
    }
    return new RustCompiledRegex { move(parsed_regex), move(pattern_str), ecma_flags };
}

extern "C" void rust_free_compiled_regex(void* ptr)
{
    delete static_cast<RustCompiledRegex*>(ptr);
}

extern "C" void rust_free_error_string(char const* str)
{
    free(const_cast<char*>(str));
}

extern "C" size_t rust_number_to_utf16(double value, uint16_t* buffer, size_t buffer_len)
{
    auto str = JS::number_to_utf16_string(value);
    auto view = str.utf16_view();
    auto len = min(view.length_in_code_units(), buffer_len);
    for (size_t i = 0; i < len; ++i)
        buffer[i] = view.code_unit_at(i);
    return len;
}
