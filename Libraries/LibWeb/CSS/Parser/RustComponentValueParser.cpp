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

static void append_component_value_token(ComponentValueBuilder& builder, FFI::CssComponentValueKind kind, Token token)
{
    switch (kind) {
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
}

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
            append_component_value_token(builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
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
            append_component_value_token(builder.component_value_builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
        });

    VERIFY(builder.component_value_builder.stack.is_empty());
    if (!builder.declaration.has_value())
        return {};

    builder.declaration->value = move(builder.component_value_builder.root_values);
    return builder.declaration;
}

struct RuleEventBuilder {
    enum class FrameType : u8 {
        AtRule,
        QualifiedRule,
        Declaration,
        ListOfDeclarations,
        Prelude,
        ChildRules,
        Declarations,
    };

    struct Frame {
        FrameType type;
        Optional<Rule> rule;
        Optional<Declaration> declaration;
        Vector<Declaration, 0> declarations;
    };

    Optional<Rule> rule;
    Vector<RuleOrListOfDeclarations> rules_or_lists_of_declarations;
    Vector<Frame> stack;
    ComponentValueBuilder component_value_builder;

    void append_rule(Rule completed_rule)
    {
        if (stack.is_empty()) {
            VERIFY(!rule.has_value());
            rule = move(completed_rule);
            return;
        }

        VERIFY(stack.last().type == FrameType::ChildRules);
        if (stack.size() == 1) {
            rules_or_lists_of_declarations.append(RuleOrListOfDeclarations { move(completed_rule) });
            return;
        }

        auto& parent = stack[stack.size() - 2];
        parent.rule->visit(
            [&](AtRule& at_rule) {
                at_rule.child_rules_and_lists_of_declarations.append(RuleOrListOfDeclarations { move(completed_rule) });
            },
            [&](QualifiedRule& qualified_rule) {
                qualified_rule.child_rules.append(RuleOrListOfDeclarations { move(completed_rule) });
            });
    }

    void append_declaration(Declaration completed_declaration)
    {
        VERIFY(!stack.is_empty());
        auto& parent = stack.last();
        switch (parent.type) {
        case FrameType::Declarations:
            VERIFY(stack.size() >= 2);
            stack[stack.size() - 2].rule->get<QualifiedRule>().declarations.append(move(completed_declaration));
            break;
        case FrameType::ListOfDeclarations:
            parent.declarations.append(move(completed_declaration));
            break;
        default:
            VERIFY_NOT_REACHED();
        }
    }

    void append_list_of_declarations(Vector<Declaration, 0> declarations)
    {
        VERIFY(!stack.is_empty());
        VERIFY(stack.last().type == FrameType::ChildRules);
        if (stack.size() == 1) {
            rules_or_lists_of_declarations.append(RuleOrListOfDeclarations { move(declarations) });
            return;
        }

        auto& parent = stack[stack.size() - 2];
        parent.rule->visit(
            [&](AtRule& at_rule) {
                at_rule.child_rules_and_lists_of_declarations.append(RuleOrListOfDeclarations { move(declarations) });
            },
            [&](QualifiedRule& qualified_rule) {
                qualified_rule.child_rules.append(RuleOrListOfDeclarations { move(declarations) });
            });
    }
};

static void apply_rule_event(RuleEventBuilder& builder, FFI::CssRuleEvent const& event)
{
    switch (event.kind) {
    case FFI::CssRuleEventKind::Invalid:
        builder.rule = {};
        break;
    case FFI::CssRuleEventKind::AtRuleStart:
        builder.stack.append({
            .type = RuleEventBuilder::FrameType::AtRule,
            .rule = Rule { AtRule {
                .name = fly_string_from_ffi_bytes(event.name_ptr, event.name_len),
                .prelude = {},
                .child_rules_and_lists_of_declarations = {},
                .is_block_rule = event.is_block_rule,
            } },
        });
        break;
    case FFI::CssRuleEventKind::AtRuleEnd: {
        VERIFY(!builder.stack.is_empty());
        auto frame = builder.stack.take_last();
        VERIFY(frame.type == RuleEventBuilder::FrameType::AtRule);
        builder.append_rule(frame.rule.release_value());
        break;
    }
    case FFI::CssRuleEventKind::QualifiedRuleStart:
        builder.stack.append({
            .type = RuleEventBuilder::FrameType::QualifiedRule,
            .rule = Rule { QualifiedRule {
                .prelude = {},
                .declarations = {},
                .child_rules = {},
            } },
        });
        break;
    case FFI::CssRuleEventKind::QualifiedRuleEnd: {
        VERIFY(!builder.stack.is_empty());
        auto frame = builder.stack.take_last();
        VERIFY(frame.type == RuleEventBuilder::FrameType::QualifiedRule);
        builder.append_rule(frame.rule.release_value());
        break;
    }
    case FFI::CssRuleEventKind::PreludeStart:
        builder.component_value_builder = {};
        builder.stack.append({ .type = RuleEventBuilder::FrameType::Prelude });
        break;
    case FFI::CssRuleEventKind::PreludeEnd: {
        VERIFY(!builder.stack.is_empty());
        auto frame = builder.stack.take_last();
        VERIFY(frame.type == RuleEventBuilder::FrameType::Prelude);
        VERIFY(builder.component_value_builder.stack.is_empty());
        VERIFY(!builder.stack.is_empty());
        builder.stack.last().rule->visit(
            [&](AtRule& at_rule) {
                at_rule.prelude = move(builder.component_value_builder.root_values);
            },
            [&](QualifiedRule& qualified_rule) {
                qualified_rule.prelude = move(builder.component_value_builder.root_values);
            });
        builder.component_value_builder = {};
        break;
    }
    case FFI::CssRuleEventKind::ChildRulesStart:
        builder.stack.append({ .type = RuleEventBuilder::FrameType::ChildRules });
        break;
    case FFI::CssRuleEventKind::ChildRulesEnd: {
        VERIFY(!builder.stack.is_empty());
        auto frame = builder.stack.take_last();
        VERIFY(frame.type == RuleEventBuilder::FrameType::ChildRules);
        break;
    }
    case FFI::CssRuleEventKind::DeclarationsStart:
        builder.stack.append({ .type = RuleEventBuilder::FrameType::Declarations });
        break;
    case FFI::CssRuleEventKind::DeclarationsEnd: {
        VERIFY(!builder.stack.is_empty());
        auto frame = builder.stack.take_last();
        VERIFY(frame.type == RuleEventBuilder::FrameType::Declarations);
        break;
    }
    case FFI::CssRuleEventKind::ListOfDeclarationsStart:
        builder.stack.append({ .type = RuleEventBuilder::FrameType::ListOfDeclarations });
        break;
    case FFI::CssRuleEventKind::ListOfDeclarationsEnd: {
        VERIFY(!builder.stack.is_empty());
        auto frame = builder.stack.take_last();
        VERIFY(frame.type == RuleEventBuilder::FrameType::ListOfDeclarations);
        builder.append_list_of_declarations(move(frame.declarations));
        break;
    }
    case FFI::CssRuleEventKind::DeclarationStart:
        builder.component_value_builder = {};
        builder.stack.append({
            .type = RuleEventBuilder::FrameType::Declaration,
            .declaration = Declaration {
                .name = fly_string_from_ffi_bytes(event.name_ptr, event.name_len),
                .value = {},
                .important = event.important ? Important::Yes : Important::No,
            },
        });
        break;
    case FFI::CssRuleEventKind::DeclarationEnd: {
        VERIFY(!builder.stack.is_empty());
        auto frame = builder.stack.take_last();
        VERIFY(frame.type == RuleEventBuilder::FrameType::Declaration);
        VERIFY(builder.component_value_builder.stack.is_empty());
        auto declaration = frame.declaration.release_value();
        declaration.value = move(builder.component_value_builder.root_values);
        builder.component_value_builder = {};
        builder.append_declaration(move(declaration));
        break;
    }
    }
}

static void verify_rule_event_builder_is_empty(RuleEventBuilder const& builder)
{
    VERIFY(builder.stack.is_empty());
    VERIFY(builder.component_value_builder.stack.is_empty());
}

Optional<Rule> RustComponentValueParser::parse_a_rule(StringView input, StringView encoding)
{
    RuleEventBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_rule(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssRuleEvent const* event) {
            apply_rule_event(*static_cast<RuleEventBuilder*>(raw_builder), *event);
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<RuleEventBuilder*>(raw_builder);
            append_component_value_token(builder.component_value_builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
        });

    verify_rule_event_builder_is_empty(builder);
    return builder.rule;
}

Vector<RuleOrListOfDeclarations> RustComponentValueParser::parse_a_blocks_contents(StringView input, StringView encoding)
{
    RuleEventBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_block_contents(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssRuleEvent const* event) {
            apply_rule_event(*static_cast<RuleEventBuilder*>(raw_builder), *event);
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<RuleEventBuilder*>(raw_builder);
            append_component_value_token(builder.component_value_builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
        });

    verify_rule_event_builder_is_empty(builder);
    return move(builder.rules_or_lists_of_declarations);
}

}
