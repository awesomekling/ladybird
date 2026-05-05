/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringBuilder.h>
#include <LibTextCodec/Decoder.h>
#include <LibWeb/CSS/CharacterTypes.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/Parser/RustTokenizer.h>
#include <LibWeb/RustFFI.h>

namespace Web::CSS::Parser {

// U+FFFD REPLACEMENT CHARACTER
static constexpr u32 REPLACEMENT_CHARACTER = 0xFFFD;

static FlyString fly_string_from_ffi_bytes(u8 const* bytes, size_t length)
{
    if (length == 0)
        return {};
    return FlyString::from_utf8_without_validation({ bytes, length });
}

static String decode_and_filter_code_points(StringView input, StringView encoding)
{
    // https://www.w3.org/TR/css-syntax-3/#css-filter-code-points
    auto decoder = TextCodec::decoder_for(encoding);
    VERIFY(decoder.has_value());

    auto decoded_input = MUST(decoder->to_utf8(input));

    // OPTIMIZATION: If the input doesn't contain any filterable characters, we can skip the filtering
    bool const contains_filterable = [&] {
        for (auto code_point : decoded_input.code_points()) {
            if (code_point == '\r' || code_point == '\f' || code_point == 0x00 || is_unicode_surrogate(code_point))
                return true;
        }
        return false;
    }();
    if (!contains_filterable)
        return decoded_input;

    StringBuilder builder { input.length() };
    bool last_was_carriage_return = false;

    // To filter code points from a stream of (unfiltered) code points input:
    for (auto code_point : decoded_input.code_points()) {
        // Replace any U+000D CARRIAGE RETURN (CR) code points,
        // U+000C FORM FEED (FF) code points,
        // or pairs of U+000D CARRIAGE RETURN (CR) followed by U+000A LINE FEED (LF)
        // in input by a single U+000A LINE FEED (LF) code point.
        if (code_point == '\r') {
            if (last_was_carriage_return) {
                builder.append('\n');
            } else {
                last_was_carriage_return = true;
            }
        } else {
            if (last_was_carriage_return)
                builder.append('\n');

            if (code_point == '\n') {
                if (!last_was_carriage_return)
                    builder.append('\n');

            } else if (code_point == '\f') {
                builder.append('\n');
            } else if (code_point == 0x00 || is_unicode_surrogate(code_point)) {
                // Replace any U+0000 NULL or surrogate code points in input with U+FFFD REPLACEMENT CHARACTER.
                builder.append_code_point(REPLACEMENT_CHARACTER);
            } else {
                builder.append_code_point(code_point);
            }

            last_was_carriage_return = false;
        }
    }

    return builder.to_string_without_validation();
}

struct ComponentValueBuilder {
    struct Frame {
        enum class Type : u8 {
            Function,
            SimpleBlock,
        };

        Type type;
        Token start_token;
        Vector<ComponentValue> values;
    };

    Vector<ComponentValue> root_values;
    Vector<Frame> stack;

    void append(ComponentValue component_value)
    {
        if (stack.is_empty()) {
            root_values.append(move(component_value));
            return;
        }
        stack.last().values.append(move(component_value));
    }

    void start_function(Token token)
    {
        stack.append({ Frame::Type::Function, move(token), {} });
    }

    void end_function(Token end_token)
    {
        VERIFY(!stack.is_empty());
        auto frame = stack.take_last();
        VERIFY(frame.type == Frame::Type::Function);

        FlyString name = frame.start_token.function();
        append(ComponentValue { Function { move(name), move(frame.values), move(frame.start_token), move(end_token) } });
    }

    void start_simple_block(Token token)
    {
        stack.append({ Frame::Type::SimpleBlock, move(token), {} });
    }

    void end_simple_block(Token end_token)
    {
        VERIFY(!stack.is_empty());
        auto frame = stack.take_last();
        VERIFY(frame.type == Frame::Type::SimpleBlock);

        append(ComponentValue { SimpleBlock { move(frame.start_token), move(frame.values), move(end_token) } });
    }
};

Vector<ComponentValue> RustComponentValueParser::parse_a_list_of_component_values(StringView input, StringView encoding)
{
    ComponentValueBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_component_values(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<ComponentValueBuilder*>(raw_builder);
            auto token = RustTokenizer::token_from_ffi(component_value->token);
            switch (component_value->kind) {
            case FFI::CssComponentValueKind::Token:
                builder.append(ComponentValue { move(token) });
                break;
            case FFI::CssComponentValueKind::FunctionStart:
                builder.start_function(move(token));
                break;
            case FFI::CssComponentValueKind::FunctionEnd:
                builder.end_function(move(token));
                break;
            case FFI::CssComponentValueKind::SimpleBlockStart:
                builder.start_simple_block(move(token));
                break;
            case FFI::CssComponentValueKind::SimpleBlockEnd:
                builder.end_simple_block(move(token));
                break;
            }
        });

    VERIFY(builder.stack.is_empty());
    return move(builder.root_values);
}

Optional<Declaration> RustComponentValueParser::parse_a_declaration(StringView input, StringView encoding)
{
    struct DeclarationBuilder {
        Optional<Declaration> declaration;
        ComponentValueBuilder component_value_builder;
    };

    DeclarationBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_declaration(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssDeclaration const* ffi_declaration) {
            auto& builder = *static_cast<DeclarationBuilder*>(raw_builder);
            if (!ffi_declaration->is_valid)
                return;

            builder.declaration = Declaration {
                .name = fly_string_from_ffi_bytes(ffi_declaration->name_ptr, ffi_declaration->name_len),
                .value = {},
                .important = ffi_declaration->important ? Important::Yes : Important::No,
            };
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<DeclarationBuilder*>(raw_builder);
            auto token = RustTokenizer::token_from_ffi(component_value->token);
            switch (component_value->kind) {
            case FFI::CssComponentValueKind::Token:
                builder.component_value_builder.append(ComponentValue { move(token) });
                break;
            case FFI::CssComponentValueKind::FunctionStart:
                builder.component_value_builder.start_function(move(token));
                break;
            case FFI::CssComponentValueKind::FunctionEnd:
                builder.component_value_builder.end_function(move(token));
                break;
            case FFI::CssComponentValueKind::SimpleBlockStart:
                builder.component_value_builder.start_simple_block(move(token));
                break;
            case FFI::CssComponentValueKind::SimpleBlockEnd:
                builder.component_value_builder.end_simple_block(move(token));
                break;
            }
        });

    VERIFY(builder.component_value_builder.stack.is_empty());
    if (!builder.declaration.has_value())
        return {};

    builder.declaration->value = move(builder.component_value_builder.root_values);
    return builder.declaration;
}

}
