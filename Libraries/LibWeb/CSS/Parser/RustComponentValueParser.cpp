/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringBuilder.h>
#include <LibTextCodec/Decoder.h>
#include <LibWeb/CSS/CharacterTypes.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/Parser/RustTokenizer.h>
#include <LibWeb/CSS/Parser/Syntax.h>
#include <LibWeb/CSS/PropertyName.h>
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

static String string_from_ffi_bytes(u8 const* bytes, size_t length)
{
    if (length == 0)
        return {};
    return String::from_utf8_without_validation({ bytes, length });
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

static void set_original_value_text_for_custom_property(Declaration& declaration)
{
    // https://drafts.csswg.org/css-syntax/#consume-declaration
    // If decl’s name is a custom property name string, then set decl’s original text to the
    // segment of the original source text string corresponding to the tokens of decl’s value.
    if (!is_a_custom_property_name_string(declaration.name))
        return;

    // TODO: If the Rust parser emitted the original source segment directly, we could use
    //       that instead of having to reconstruct it.
    StringBuilder original_text;
    for (auto const& value : declaration.value)
        original_text.append(value.original_source_text());
    declaration.original_value_text = original_text.to_string_without_validation();
}

static FFI::CssRuleContext rule_context_to_ffi(RuleContext);

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

Vector<Vector<ComponentValue>> RustComponentValueParser::parse_a_comma_separated_list_of_component_values(StringView input, StringView encoding)
{
    struct CommaSeparatedListBuilder {
        Vector<Vector<ComponentValue>> groups;
        ComponentValueBuilder component_value_builder;
    };

    CommaSeparatedListBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_comma_separated_component_values(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder) {
            auto& builder = *static_cast<CommaSeparatedListBuilder*>(raw_builder);
            VERIFY(builder.component_value_builder.stack.is_empty());
            builder.groups.append(move(builder.component_value_builder.root_values));
            builder.component_value_builder = {};
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<CommaSeparatedListBuilder*>(raw_builder);
            append_component_value_token(builder.component_value_builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
        });

    VERIFY(builder.component_value_builder.stack.is_empty());
    VERIFY(builder.component_value_builder.root_values.is_empty());
    return move(builder.groups);
}

Optional<ComponentValue> RustComponentValueParser::parse_a_component_value(StringView input, StringView encoding)
{
    ComponentValueBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_component_value(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<ComponentValueBuilder*>(raw_builder);
            append_component_value_token(builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
        });

    VERIFY(builder.stack.is_empty());
    if (builder.root_values.is_empty())
        return {};

    VERIFY(builder.root_values.size() == 1);
    return builder.root_values.take_first();
}

FFI::CssValueTypeSyntaxKind RustComponentValueParser::parse_a_value_type(u8 value_type_id, TokenStream<ComponentValue>& tokens)
{
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return FFI::CssValueTypeSyntaxKind::Invalid;

    auto component_value_source = tokens.next_token().original_source_text();
    auto component_value_source_bytes = component_value_source.bytes();
    return FFI::rust_css_parse_value_type(
        component_value_source_bytes.data(),
        component_value_source_bytes.size(),
        value_type_id);
}

struct RustSyntaxNodeBuilder {
    enum class FrameType : u8 {
        Multiplier,
        CommaSeparatedMultiplier,
        Alternatives,
    };

    struct Frame {
        FrameType type;
        Vector<NonnullOwnPtr<SyntaxNode>> children;
    };

    Vector<Frame> stack;
    OwnPtr<SyntaxNode> root;
    CaseSensitivity ident_case_sensitivity { CaseSensitivity::CaseInsensitive };
    bool invalid { false };

    void append_node(NonnullOwnPtr<SyntaxNode> node)
    {
        if (stack.is_empty()) {
            if (root) {
                invalid = true;
                return;
            }
            root = move(node);
            return;
        }

        stack.last().children.append(move(node));
    }

    void end_frame(FrameType expected_type)
    {
        VERIFY(!stack.is_empty());
        auto frame = stack.take_last();
        VERIFY(frame.type == expected_type);

        switch (expected_type) {
        case FrameType::Multiplier:
            if (frame.children.size() != 1) {
                invalid = true;
                return;
            }
            append_node(MultiplierSyntaxNode::create(frame.children.take_first()));
            return;
        case FrameType::CommaSeparatedMultiplier:
            if (frame.children.size() != 1) {
                invalid = true;
                return;
            }
            append_node(CommaSeparatedMultiplierSyntaxNode::create(frame.children.take_first()));
            return;
        case FrameType::Alternatives:
            if (frame.children.is_empty()) {
                invalid = true;
                return;
            }
            append_node(AlternativesSyntaxNode::create(move(frame.children)));
            return;
        }

        VERIFY_NOT_REACHED();
    }
};

OwnPtr<SyntaxNode> RustComponentValueParser::parse_as_syntax(StringView input, StringView encoding, LimitSingleComponentIdentToCustomIdent limit_single_component_ident_to_custom_ident)
{
    RustSyntaxNodeBuilder builder;
    builder.ident_case_sensitivity = limit_single_component_ident_to_custom_ident == LimitSingleComponentIdentToCustomIdent::Yes ? CaseSensitivity::CaseSensitive : CaseSensitivity::CaseInsensitive;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_as_syntax(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        limit_single_component_ident_to_custom_ident == LimitSingleComponentIdentToCustomIdent::Yes,
        &builder,
        [](void* raw_builder, FFI::CssSyntaxNode const* syntax_node) {
            auto& builder = *static_cast<RustSyntaxNodeBuilder*>(raw_builder);
            switch (syntax_node->kind) {
            case FFI::CssSyntaxNodeKind::Invalid:
                builder.invalid = true;
                return;
            case FFI::CssSyntaxNodeKind::Universal:
                builder.append_node(UniversalSyntaxNode::create());
                return;
            case FFI::CssSyntaxNodeKind::Type:
                builder.append_node(TypeSyntaxNode::create(fly_string_from_ffi_bytes(syntax_node->value_ptr, syntax_node->value_len)));
                return;
            case FFI::CssSyntaxNodeKind::Ident:
                builder.append_node(IdentSyntaxNode::create(fly_string_from_ffi_bytes(syntax_node->value_ptr, syntax_node->value_len), builder.ident_case_sensitivity));
                return;
            case FFI::CssSyntaxNodeKind::MultiplierStart:
                builder.stack.append({ RustSyntaxNodeBuilder::FrameType::Multiplier, {} });
                return;
            case FFI::CssSyntaxNodeKind::MultiplierEnd:
                builder.end_frame(RustSyntaxNodeBuilder::FrameType::Multiplier);
                return;
            case FFI::CssSyntaxNodeKind::CommaSeparatedMultiplierStart:
                builder.stack.append({ RustSyntaxNodeBuilder::FrameType::CommaSeparatedMultiplier, {} });
                return;
            case FFI::CssSyntaxNodeKind::CommaSeparatedMultiplierEnd:
                builder.end_frame(RustSyntaxNodeBuilder::FrameType::CommaSeparatedMultiplier);
                return;
            case FFI::CssSyntaxNodeKind::AlternativesStart:
                builder.stack.append({ RustSyntaxNodeBuilder::FrameType::Alternatives, {} });
                return;
            case FFI::CssSyntaxNodeKind::AlternativesEnd:
                builder.end_frame(RustSyntaxNodeBuilder::FrameType::Alternatives);
                return;
            }

            VERIFY_NOT_REACHED();
        });

    VERIFY(builder.stack.is_empty());
    if (builder.invalid)
        return {};
    return move(builder.root);
}

bool RustComponentValueParser::parse_empty_prelude(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_empty_prelude(filtered_input_bytes.data(), filtered_input_bytes.size());
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
    set_original_value_text_for_custom_property(*builder.declaration);
    return builder.declaration;
}

Optional<Declaration> RustComponentValueParser::parse_a_declaration(StringView input, StringView encoding, Vector<RuleContext> const& rule_context)
{
    struct DeclarationBuilder {
        Optional<Declaration> declaration;
        ComponentValueBuilder component_value_builder;
    };

    DeclarationBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    Vector<FFI::CssRuleContext> ffi_rule_context;
    ffi_rule_context.ensure_capacity(rule_context.size());
    for (auto context : rule_context)
        ffi_rule_context.unchecked_append(rule_context_to_ffi(context));

    FFI::rust_css_parse_declaration_with_context(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        ffi_rule_context.data(),
        ffi_rule_context.size(),
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
    set_original_value_text_for_custom_property(*builder.declaration);
    return builder.declaration;
}

struct RustMediaFeatureTestBuilder {
    FFI::CssMediaFeature feature;
    Optional<FFI::CssMediaFeatureValueSyntaxKind> value_syntax_kind;
    Optional<FFI::CssMediaFeatureValueSyntaxKind> left_value_syntax_kind;
    Optional<FFI::CssMediaFeatureValueSyntaxKind> right_value_syntax_kind;
    ComponentValueBuilder value_builder;
    ComponentValueBuilder left_value_builder;
    ComponentValueBuilder right_value_builder;

    RustComponentValueParser::MediaFeatureTest build()
    {
        VERIFY(value_builder.stack.is_empty());
        VERIFY(left_value_builder.stack.is_empty());
        VERIFY(right_value_builder.stack.is_empty());
        return RustComponentValueParser::MediaFeatureTest {
            .feature = feature,
            .value_syntax_kind = value_syntax_kind.value_or(FFI::CssMediaFeatureValueSyntaxKind::Invalid),
            .left_value_syntax_kind = left_value_syntax_kind.value_or(FFI::CssMediaFeatureValueSyntaxKind::Invalid),
            .right_value_syntax_kind = right_value_syntax_kind.value_or(FFI::CssMediaFeatureValueSyntaxKind::Invalid),
            .value = move(value_builder.root_values),
            .left_value = move(left_value_builder.root_values),
            .right_value = move(right_value_builder.root_values),
        };
    }
};

static void set_media_feature_value_syntax_kind(Optional<FFI::CssMediaFeatureValueSyntaxKind>& target, FFI::CssMediaFeatureValueSyntaxKind syntax_kind)
{
    if (target.has_value()) {
        VERIFY(target.value() == syntax_kind);
        return;
    }
    target = syntax_kind;
}

static Optional<MediaQuery::KnownMediaType> media_type_from_rust(FFI::CssMediaTypeKind media_type_kind)
{
    switch (media_type_kind) {
    case FFI::CssMediaTypeKind::None:
    case FFI::CssMediaTypeKind::Unknown:
        return {};
    case FFI::CssMediaTypeKind::All:
        return MediaQuery::KnownMediaType::All;
    case FFI::CssMediaTypeKind::Print:
        return MediaQuery::KnownMediaType::Print;
    case FFI::CssMediaTypeKind::Screen:
        return MediaQuery::KnownMediaType::Screen;
    }
    VERIFY_NOT_REACHED();
}

struct RustBooleanExpressionBuilder {
    enum class FrameType : u8 {
        Not,
        Parens,
        And,
        Or,
        Test,
        GeneralEnclosed,
    };

    struct Frame {
        FrameType type;
        Vector<NonnullOwnPtr<BooleanExpression>> children;
    };

    Vector<Frame> stack;
    OwnPtr<BooleanExpression> root;
    ComponentValueBuilder component_value_builder;
    Optional<RustMediaFeatureTestBuilder> media_feature;
    AK::Function<OwnPtr<BooleanExpression>(Optional<RustComponentValueParser::MediaFeatureTest>&&, Vector<ComponentValue>&&)> parse_test;
    MatchResult result_for_general_enclosed;
    bool invalid { false };

    void append_expression(OwnPtr<BooleanExpression> expression)
    {
        if (!expression) {
            invalid = true;
            return;
        }

        if (stack.is_empty()) {
            if (root) {
                invalid = true;
                return;
            }
            root = expression.release_nonnull();
            return;
        }

        stack.last().children.append(expression.release_nonnull());
    }

    void end_frame(FrameType expected_type)
    {
        VERIFY(!stack.is_empty());
        auto frame = stack.take_last();
        VERIFY(frame.type == expected_type);

        switch (expected_type) {
        case FrameType::Not:
            if (frame.children.size() != 1) {
                invalid = true;
                return;
            }
            append_expression(BooleanNotExpression::create(frame.children.take_first()));
            return;
        case FrameType::Parens:
            if (frame.children.size() != 1) {
                invalid = true;
                return;
            }
            append_expression(BooleanExpressionInParens::create(frame.children.take_first()));
            return;
        case FrameType::And:
            if (frame.children.is_empty()) {
                invalid = true;
                return;
            }
            append_expression(BooleanAndExpression::create(move(frame.children)));
            return;
        case FrameType::Or:
            if (frame.children.is_empty()) {
                invalid = true;
                return;
            }
            append_expression(BooleanOrExpression::create(move(frame.children)));
            return;
        case FrameType::Test:
        case FrameType::GeneralEnclosed:
            VERIFY_NOT_REACHED();
        }
    }

    void end_test()
    {
        VERIFY(!stack.is_empty());
        auto frame = stack.take_last();
        VERIFY(frame.type == FrameType::Test);
        VERIFY(frame.children.is_empty());
        VERIFY(component_value_builder.stack.is_empty());

        Optional<String> general_enclosed_fallback;
        if (component_value_builder.root_values.size() == 1)
            general_enclosed_fallback = component_value_builder.root_values.first().to_string();

        Optional<RustComponentValueParser::MediaFeatureTest> media_feature_test;
        if (media_feature.has_value())
            media_feature_test = media_feature->build();

        auto expression = parse_test(move(media_feature_test), move(component_value_builder.root_values));
        if (!expression && general_enclosed_fallback.has_value())
            expression = GeneralEnclosed::create(general_enclosed_fallback.release_value(), result_for_general_enclosed);
        append_expression(move(expression));
        component_value_builder = {};
        media_feature = {};
    }

    void end_general_enclosed()
    {
        VERIFY(!stack.is_empty());
        auto frame = stack.take_last();
        VERIFY(frame.type == FrameType::GeneralEnclosed);
        VERIFY(frame.children.is_empty());
        VERIFY(component_value_builder.stack.is_empty());
        VERIFY(component_value_builder.root_values.size() == 1);

        auto serialized_contents = component_value_builder.root_values.first().to_string();
        append_expression(GeneralEnclosed::create(move(serialized_contents), result_for_general_enclosed));
        component_value_builder = {};
    }
};

static void process_boolean_expression_event(RustBooleanExpressionBuilder& builder, FFI::CssBooleanExpressionEventKind event)
{
    switch (event) {
    case FFI::CssBooleanExpressionEventKind::Invalid:
        builder.invalid = true;
        break;
    case FFI::CssBooleanExpressionEventKind::NotStart:
        builder.stack.append({ .type = RustBooleanExpressionBuilder::FrameType::Not });
        break;
    case FFI::CssBooleanExpressionEventKind::ParensStart:
        builder.stack.append({ .type = RustBooleanExpressionBuilder::FrameType::Parens });
        break;
    case FFI::CssBooleanExpressionEventKind::AndStart:
        builder.stack.append({ .type = RustBooleanExpressionBuilder::FrameType::And });
        break;
    case FFI::CssBooleanExpressionEventKind::OrStart:
        builder.stack.append({ .type = RustBooleanExpressionBuilder::FrameType::Or });
        break;
    case FFI::CssBooleanExpressionEventKind::TestStart:
        builder.component_value_builder = {};
        builder.media_feature = {};
        builder.stack.append({ .type = RustBooleanExpressionBuilder::FrameType::Test });
        break;
    case FFI::CssBooleanExpressionEventKind::GeneralEnclosedStart:
        builder.component_value_builder = {};
        builder.stack.append({ .type = RustBooleanExpressionBuilder::FrameType::GeneralEnclosed });
        break;
    case FFI::CssBooleanExpressionEventKind::NotEnd:
        builder.end_frame(RustBooleanExpressionBuilder::FrameType::Not);
        break;
    case FFI::CssBooleanExpressionEventKind::ParensEnd:
        builder.end_frame(RustBooleanExpressionBuilder::FrameType::Parens);
        break;
    case FFI::CssBooleanExpressionEventKind::AndEnd:
        builder.end_frame(RustBooleanExpressionBuilder::FrameType::And);
        break;
    case FFI::CssBooleanExpressionEventKind::OrEnd:
        builder.end_frame(RustBooleanExpressionBuilder::FrameType::Or);
        break;
    case FFI::CssBooleanExpressionEventKind::TestEnd:
        builder.end_test();
        break;
    case FFI::CssBooleanExpressionEventKind::GeneralEnclosedEnd:
        builder.end_general_enclosed();
        break;
    }
}

static void set_boolean_expression_media_feature(RustBooleanExpressionBuilder& builder, FFI::CssMediaFeature const* media_feature)
{
    builder.media_feature = RustMediaFeatureTestBuilder {
        .feature = *media_feature,
    };
}

static void append_boolean_expression_media_feature_value(RustBooleanExpressionBuilder& builder, FFI::CssMediaFeatureValue const* media_feature_value)
{
    VERIFY(builder.media_feature.has_value());

    auto append_to_builder = [&](ComponentValueBuilder& component_value_builder) {
        append_component_value_token(component_value_builder, media_feature_value->component_value.kind, RustTokenizer::token_from_ffi(media_feature_value->component_value.token));
    };

    switch (media_feature_value->kind) {
    case FFI::CssMediaFeatureValueKind::Value:
        set_media_feature_value_syntax_kind(builder.media_feature->value_syntax_kind, media_feature_value->syntax_kind);
        append_to_builder(builder.media_feature->value_builder);
        break;
    case FFI::CssMediaFeatureValueKind::LeftValue:
        set_media_feature_value_syntax_kind(builder.media_feature->left_value_syntax_kind, media_feature_value->syntax_kind);
        append_to_builder(builder.media_feature->left_value_builder);
        break;
    case FFI::CssMediaFeatureValueKind::RightValue:
        set_media_feature_value_syntax_kind(builder.media_feature->right_value_syntax_kind, media_feature_value->syntax_kind);
        append_to_builder(builder.media_feature->right_value_builder);
        break;
    }
}

using MediaQueryCallback = void (*)(void*, FFI::CssMediaQuery const*);
using BooleanExpressionEventCallback = void (*)(void*, FFI::CssBooleanExpressionEventKind);
using MediaFeatureCallback = void (*)(void*, FFI::CssMediaFeature const*);
using MediaFeatureValueCallback = void (*)(void*, FFI::CssMediaFeatureValue const*);
using ComponentValueCallback = void (*)(void*, FFI::CssComponentValue const*);

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_boolean_expression(StringView input, StringView encoding, MatchResult result_for_general_enclosed, BooleanExpressionTestParser parse_test, RustBooleanExpressionParser rust_parse_boolean_expression)
{
    RustBooleanExpressionBuilder builder {
        .parse_test = move(parse_test),
        .result_for_general_enclosed = result_for_general_enclosed,
    };
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    rust_parse_boolean_expression(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssBooleanExpressionEventKind event) {
            auto& builder = *static_cast<RustBooleanExpressionBuilder*>(raw_builder);
            process_boolean_expression_event(builder, event);
        },
        [](void* raw_builder, FFI::CssMediaFeature const* media_feature) {
            auto& builder = *static_cast<RustBooleanExpressionBuilder*>(raw_builder);
            set_boolean_expression_media_feature(builder, media_feature);
        },
        [](void* raw_builder, FFI::CssMediaFeatureValue const* media_feature_value) {
            auto& builder = *static_cast<RustBooleanExpressionBuilder*>(raw_builder);
            append_boolean_expression_media_feature_value(builder, media_feature_value);
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<RustBooleanExpressionBuilder*>(raw_builder);
            append_component_value_token(builder.component_value_builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
        });

    if (builder.invalid)
        return nullptr;

    VERIFY(builder.stack.is_empty());
    VERIFY(builder.component_value_builder.stack.is_empty());
    return move(builder.root);
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_supports_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(Vector<ComponentValue>&&)> parse_test)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::False,
        [parse_test = move(parse_test)](Optional<MediaFeatureTest>&&, Vector<ComponentValue>&& component_values) mutable {
            return parse_test(move(component_values));
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto, auto, auto component_value_callback) {
            FFI::rust_css_parse_supports_condition(input, input_size, context, event_callback, component_value_callback);
        });
}

Optional<RustComponentValueParser::SupportsFeature> RustComponentValueParser::parse_a_supports_feature(StringView input, StringView encoding)
{
    Optional<SupportsFeature> feature;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_supports_feature(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &feature,
        [](void* raw_feature, FFI::CssSupportsFeatureKind kind, u8 const* name_ptr, size_t name_len) {
            auto& feature = *static_cast<Optional<SupportsFeature>*>(raw_feature);
            Optional<FlyString> name;
            if (name_ptr)
                name = fly_string_from_ffi_bytes(name_ptr, name_len);
            feature = SupportsFeature { kind, move(name) };
        });

    if (!parsed)
        return {};

    return feature;
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_an_if_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(Vector<ComponentValue>&&)> parse_test)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::False,
        [parse_test = move(parse_test)](Optional<MediaFeatureTest>&&, Vector<ComponentValue>&& component_values) mutable {
            return parse_test(move(component_values));
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto, auto, auto component_value_callback) {
            FFI::rust_css_parse_if_condition(input, input_size, context, event_callback, component_value_callback);
        });
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_container_condition(StringView input, StringView encoding)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::False,
        [](Optional<MediaFeatureTest>&&, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
            return nullptr;
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
            FFI::rust_css_parse_media_condition(input, input_size, context, event_callback, media_feature_callback, media_feature_value_callback, component_value_callback);
        });
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_media_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::Unknown,
        [parse_test = move(parse_test)](Optional<MediaFeatureTest>&& media_feature, Vector<ComponentValue>&&) mutable -> OwnPtr<BooleanExpression> {
            if (!media_feature.has_value())
                return nullptr;
            return parse_test(media_feature.release_value());
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
            FFI::rust_css_parse_media_condition(input, input_size, context, event_callback, media_feature_callback, media_feature_value_callback, component_value_callback);
        });
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_media_test(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::False,
        [parse_test = move(parse_test)](Optional<MediaFeatureTest>&& media_feature, Vector<ComponentValue>&&) mutable -> OwnPtr<BooleanExpression> {
            if (!media_feature.has_value())
                return nullptr;
            return parse_test(media_feature.release_value());
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
            FFI::rust_css_parse_media_test(input, input_size, context, event_callback, media_feature_callback, media_feature_value_callback, component_value_callback);
        });
}

struct MediaQuerySyntaxBuilder {
    Vector<RustComponentValueParser::MediaQuerySyntax> media_queries;
    Optional<RustBooleanExpressionBuilder> media_condition_builder;
    AK::Function<OwnPtr<BooleanExpression>(RustComponentValueParser::MediaFeatureTest&&)> parse_test;

    static RustComponentValueParser::MediaQuerySyntax create_not_all_media_query_syntax()
    {
        return RustComponentValueParser::MediaQuerySyntax {
            .is_negated = true,
            .media_type = MediaQuery::MediaType {
                .name = "all"_fly_string,
                .known_type = MediaQuery::KnownMediaType::All,
            },
        };
    }

    void finish_media_condition()
    {
        if (!media_condition_builder.has_value())
            return;

        VERIFY(!media_queries.is_empty());
        auto& media_query = media_queries.last();
        if (media_condition_builder->invalid || !media_condition_builder->stack.is_empty() || !media_condition_builder->root) {
            media_query = create_not_all_media_query_syntax();
            media_condition_builder = {};
            return;
        }

        VERIFY(media_condition_builder->component_value_builder.stack.is_empty());
        media_query.media_condition = media_condition_builder->root.release_nonnull();
        media_condition_builder = {};
    }

    void start_media_query(FFI::CssMediaQuery const* rust_media_query)
    {
        finish_media_condition();

        Optional<MediaQuery::MediaType> media_type;
        if (rust_media_query->media_type_len > 0) {
            auto media_type_name = fly_string_from_ffi_bytes(rust_media_query->media_type_ptr, rust_media_query->media_type_len);
            media_type = MediaQuery::MediaType {
                .name = media_type_name,
                .known_type = media_type_from_rust(rust_media_query->media_type_kind),
            };
        }

        media_queries.append(RustComponentValueParser::MediaQuerySyntax {
            .is_negated = rust_media_query->is_negated,
            .media_type = media_type,
        });

        if (rust_media_query->has_media_condition) {
            media_condition_builder = RustBooleanExpressionBuilder {
                .parse_test = [this](Optional<RustComponentValueParser::MediaFeatureTest>&& media_feature, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
                    if (!media_feature.has_value())
                        return nullptr;
                    return parse_test(media_feature.release_value());
                },
                .result_for_general_enclosed = MatchResult::Unknown,
            };
        }
    }
};

static void parse_media_query_syntax(
    StringView input,
    StringView encoding,
    MediaQuerySyntaxBuilder& builder,
    AK::Function<void(u8 const*, size_t, void*, MediaQueryCallback, BooleanExpressionEventCallback, MediaFeatureCallback, MediaFeatureValueCallback, ComponentValueCallback)> parse)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    parse(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssMediaQuery const* media_query) {
            auto& builder = *static_cast<MediaQuerySyntaxBuilder*>(raw_builder);
            builder.start_media_query(media_query);
        },
        [](void* raw_builder, FFI::CssBooleanExpressionEventKind event) {
            auto& builder = *static_cast<MediaQuerySyntaxBuilder*>(raw_builder);
            VERIFY(builder.media_condition_builder.has_value());
            process_boolean_expression_event(*builder.media_condition_builder, event);
        },
        [](void* raw_builder, FFI::CssMediaFeature const* media_feature) {
            auto& builder = *static_cast<MediaQuerySyntaxBuilder*>(raw_builder);
            VERIFY(builder.media_condition_builder.has_value());
            set_boolean_expression_media_feature(*builder.media_condition_builder, media_feature);
        },
        [](void* raw_builder, FFI::CssMediaFeatureValue const* media_feature_value) {
            auto& builder = *static_cast<MediaQuerySyntaxBuilder*>(raw_builder);
            VERIFY(builder.media_condition_builder.has_value());
            append_boolean_expression_media_feature_value(*builder.media_condition_builder, media_feature_value);
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<MediaQuerySyntaxBuilder*>(raw_builder);
            VERIFY(builder.media_condition_builder.has_value());
            append_component_value_token(builder.media_condition_builder->component_value_builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
        });

    builder.finish_media_condition();
}

Optional<RustComponentValueParser::MediaQuerySyntax> RustComponentValueParser::parse_a_media_query(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test)
{
    MediaQuerySyntaxBuilder builder {
        .parse_test = move(parse_test),
    };

    auto parsed_media_query = false;
    parse_media_query_syntax(
        input,
        encoding,
        builder,
        [&parsed_media_query](u8 const* input, size_t input_size, void* context, auto media_query_callback, auto event_callback, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
            parsed_media_query = FFI::rust_css_parse_media_query(input, input_size, context, media_query_callback, event_callback, media_feature_callback, media_feature_value_callback, component_value_callback);
        });

    if (!parsed_media_query) {
        VERIFY(builder.media_queries.is_empty());
        return {};
    }

    VERIFY(builder.media_queries.size() == 1);
    return builder.media_queries.take_first();
}

Vector<RustComponentValueParser::MediaQuerySyntax> RustComponentValueParser::parse_a_media_query_list(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test)
{
    MediaQuerySyntaxBuilder builder {
        .parse_test = move(parse_test),
    };

    parse_media_query_syntax(
        input,
        encoding,
        builder,
        [](u8 const* input, size_t input_size, void* context, auto media_query_callback, auto event_callback, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
            FFI::rust_css_parse_media_query_list(input, input_size, context, media_query_callback, event_callback, media_feature_callback, media_feature_value_callback, component_value_callback);
        });

    return move(builder.media_queries);
}

static PagePseudoClass page_pseudo_class_from_rust(FFI::CssPagePseudoClassKind pseudo_class)
{
    switch (pseudo_class) {
    case FFI::CssPagePseudoClassKind::Left:
        return PagePseudoClass::Left;
    case FFI::CssPagePseudoClassKind::Right:
        return PagePseudoClass::Right;
    case FFI::CssPagePseudoClassKind::First:
        return PagePseudoClass::First;
    case FFI::CssPagePseudoClassKind::Blank:
        return PagePseudoClass::Blank;
    }
    VERIFY_NOT_REACHED();
}

struct PageSelectorListBuilder {
    PageSelectorList selectors;
    Optional<FlyString> current_name;
    Vector<PagePseudoClass> current_pseudo_classes;
    bool has_current_selector { false };

    void finish_current_selector()
    {
        if (!has_current_selector)
            return;
        selectors.empend(move(current_name), move(current_pseudo_classes));
        current_name = {};
        current_pseudo_classes.clear();
        has_current_selector = false;
    }

    void start_selector(FFI::CssPageSelector const* selector)
    {
        finish_current_selector();
        has_current_selector = true;
        if (selector->has_name)
            current_name = fly_string_from_ffi_bytes(selector->name_ptr, selector->name_len);
    }
};

Optional<PageSelectorList> RustComponentValueParser::parse_a_page_selector_list(StringView input, StringView encoding)
{
    PageSelectorListBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_page_selector_list(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssPageSelector const* selector) {
            auto& builder = *static_cast<PageSelectorListBuilder*>(raw_builder);
            builder.start_selector(selector);
        },
        [](void* raw_builder, FFI::CssPagePseudoClassKind pseudo_class) {
            auto& builder = *static_cast<PageSelectorListBuilder*>(raw_builder);
            VERIFY(builder.has_current_selector);
            builder.current_pseudo_classes.append(page_pseudo_class_from_rust(pseudo_class));
        });

    if (!parsed)
        return {};

    builder.finish_current_selector();
    return move(builder.selectors);
}

Optional<Vector<Percentage>> RustComponentValueParser::parse_a_keyframe_selector_list(StringView input, StringView encoding)
{
    Vector<Percentage> selectors;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_keyframe_selector_list(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &selectors,
        [](void* raw_selectors, double selector) {
            auto& selectors = *static_cast<Vector<Percentage>*>(raw_selectors);
            selectors.append(Percentage(selector));
        });

    if (!parsed)
        return {};

    return move(selectors);
}

Optional<FlyString> RustComponentValueParser::parse_a_keyframes_name(StringView input, StringView encoding)
{
    Optional<FlyString> name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_keyframes_name(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &name,
        [](void* raw_name, u8 const* name_ptr, size_t name_len) {
            auto& name = *static_cast<Optional<FlyString>*>(raw_name);
            name = fly_string_from_ffi_bytes(name_ptr, name_len);
        });

    if (!parsed)
        return {};

    return name;
}

Optional<FlyString> RustComponentValueParser::parse_a_custom_property_name(StringView input, StringView encoding)
{
    Optional<FlyString> name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_custom_property_name(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &name,
        [](void* raw_name, u8 const* name_ptr, size_t name_len) {
            auto& name = *static_cast<Optional<FlyString>*>(raw_name);
            name = fly_string_from_ffi_bytes(name_ptr, name_len);
        });

    if (!parsed)
        return {};

    return name;
}

Optional<FlyString> RustComponentValueParser::parse_a_custom_ident(StringView input, StringView encoding)
{
    Optional<FlyString> name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_custom_ident(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &name,
        [](void* raw_name, u8 const* name_ptr, size_t name_len) {
            auto& name = *static_cast<Optional<FlyString>*>(raw_name);
            name = fly_string_from_ffi_bytes(name_ptr, name_len);
        });

    if (!parsed)
        return {};

    return name;
}

Optional<FlyString> RustComponentValueParser::parse_a_dashed_ident(StringView input, StringView encoding)
{
    Optional<FlyString> name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_dashed_ident(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &name,
        [](void* raw_name, u8 const* name_ptr, size_t name_len) {
            auto& name = *static_cast<Optional<FlyString>*>(raw_name);
            name = fly_string_from_ffi_bytes(name_ptr, name_len);
        });

    if (!parsed)
        return {};

    return name;
}

static Gfx::UnicodeRange unicode_range_from_rust(FFI::CssUnicodeRange const& unicode_range)
{
    return Gfx::UnicodeRange {
        unicode_range.min_code_point,
        unicode_range.max_code_point,
    };
}

Optional<Gfx::UnicodeRange> RustComponentValueParser::parse_a_unicode_range(StringView input, StringView encoding)
{
    Optional<Gfx::UnicodeRange> unicode_range;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_unicode_range(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &unicode_range,
        [](void* raw_unicode_range, FFI::CssUnicodeRange const* rust_unicode_range) {
            auto& unicode_range = *static_cast<Optional<Gfx::UnicodeRange>*>(raw_unicode_range);
            unicode_range = unicode_range_from_rust(*rust_unicode_range);
        });

    if (!parsed)
        return {};

    return unicode_range;
}

Optional<Vector<Gfx::UnicodeRange>> RustComponentValueParser::parse_a_unicode_range_list(StringView input, StringView encoding)
{
    Vector<Gfx::UnicodeRange> unicode_ranges;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_unicode_range_list(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &unicode_ranges,
        [](void* raw_unicode_ranges, FFI::CssUnicodeRange const* rust_unicode_range) {
            auto& unicode_ranges = *static_cast<Vector<Gfx::UnicodeRange>*>(raw_unicode_ranges);
            unicode_ranges.append(unicode_range_from_rust(*rust_unicode_range));
        });

    if (!parsed)
        return {};

    return unicode_ranges;
}

static URL::Type url_function_type_from_rust(FFI::CssUrlFunctionType function_type)
{
    switch (function_type) {
    case FFI::CssUrlFunctionType::Url:
        return URL::Type::Url;
    case FFI::CssUrlFunctionType::Src:
        return URL::Type::Src;
    }
    VERIFY_NOT_REACHED();
}

static CrossOriginModifierValue cross_origin_modifier_value_from_rust(FFI::CssUrlCrossOriginModifierValue value)
{
    switch (value) {
    case FFI::CssUrlCrossOriginModifierValue::Anonymous:
        return CrossOriginModifierValue::Anonymous;
    case FFI::CssUrlCrossOriginModifierValue::UseCredentials:
        return CrossOriginModifierValue::UseCredentials;
    }
    VERIFY_NOT_REACHED();
}

static ReferrerPolicyModifierValue referrer_policy_modifier_value_from_rust(FFI::CssUrlReferrerPolicyModifierValue value)
{
    switch (value) {
    case FFI::CssUrlReferrerPolicyModifierValue::NoReferrer:
        return ReferrerPolicyModifierValue::NoReferrer;
    case FFI::CssUrlReferrerPolicyModifierValue::NoReferrerWhenDowngrade:
        return ReferrerPolicyModifierValue::NoReferrerWhenDowngrade;
    case FFI::CssUrlReferrerPolicyModifierValue::SameOrigin:
        return ReferrerPolicyModifierValue::SameOrigin;
    case FFI::CssUrlReferrerPolicyModifierValue::Origin:
        return ReferrerPolicyModifierValue::Origin;
    case FFI::CssUrlReferrerPolicyModifierValue::StrictOrigin:
        return ReferrerPolicyModifierValue::StrictOrigin;
    case FFI::CssUrlReferrerPolicyModifierValue::OriginWhenCrossOrigin:
        return ReferrerPolicyModifierValue::OriginWhenCrossOrigin;
    case FFI::CssUrlReferrerPolicyModifierValue::StrictOriginWhenCrossOrigin:
        return ReferrerPolicyModifierValue::StrictOriginWhenCrossOrigin;
    case FFI::CssUrlReferrerPolicyModifierValue::UnsafeUrl:
        return ReferrerPolicyModifierValue::UnsafeUrl;
    }
    VERIFY_NOT_REACHED();
}

static FontTech font_tech_from_rust(FFI::CssFontTech font_tech)
{
    switch (font_tech) {
    case FFI::CssFontTech::Avar2:
        return FontTech::Avar2;
    case FFI::CssFontTech::ColorCbdt:
        return FontTech::ColorCbdt;
    case FFI::CssFontTech::ColorColrv0:
        return FontTech::ColorColrv0;
    case FFI::CssFontTech::ColorColrv1:
        return FontTech::ColorColrv1;
    case FFI::CssFontTech::ColorSbix:
        return FontTech::ColorSbix;
    case FFI::CssFontTech::ColorSvg:
        return FontTech::ColorSvg;
    case FFI::CssFontTech::FeaturesAat:
        return FontTech::FeaturesAat;
    case FFI::CssFontTech::FeaturesGraphite:
        return FontTech::FeaturesGraphite;
    case FFI::CssFontTech::FeaturesOpentype:
        return FontTech::FeaturesOpentype;
    case FFI::CssFontTech::Incremental:
        return FontTech::Incremental;
    case FFI::CssFontTech::Palettes:
        return FontTech::Palettes;
    case FFI::CssFontTech::Variations:
        return FontTech::Variations;
    }
    VERIFY_NOT_REACHED();
}

struct RustURLFunctionBuilder {
    Optional<URL::Type> function_type;
    Optional<String> url;
    Vector<RequestURLModifier> request_url_modifiers;
};

template<typename RustParseURLFunction>
static Optional<URL> parse_url_with_rust(StringView input, StringView encoding, RustParseURLFunction rust_parse_url)
{
    RustURLFunctionBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = rust_parse_url(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssUrlFunction const* rust_url_function) {
            auto& builder = *static_cast<RustURLFunctionBuilder*>(raw_builder);
            builder.function_type = url_function_type_from_rust(rust_url_function->function_type);
            builder.url = string_from_ffi_bytes(rust_url_function->url_ptr, rust_url_function->url_len);
        },
        [](void* raw_builder, FFI::CssUrlModifier const* rust_modifier) {
            auto& builder = *static_cast<RustURLFunctionBuilder*>(raw_builder);
            switch (rust_modifier->kind) {
            case FFI::CssUrlModifierKind::CrossOrigin:
                builder.request_url_modifiers.append(RequestURLModifier::create_cross_origin(cross_origin_modifier_value_from_rust(rust_modifier->cross_origin_value)));
                break;
            case FFI::CssUrlModifierKind::Integrity:
                builder.request_url_modifiers.append(RequestURLModifier::create_integrity(fly_string_from_ffi_bytes(rust_modifier->integrity_ptr, rust_modifier->integrity_len)));
                break;
            case FFI::CssUrlModifierKind::ReferrerPolicy:
                builder.request_url_modifiers.append(RequestURLModifier::create_referrer_policy(referrer_policy_modifier_value_from_rust(rust_modifier->referrer_policy_value)));
                break;
            }
        });

    if (!parsed || !builder.function_type.has_value() || !builder.url.has_value())
        return {};

    return URL { builder.url.release_value(), builder.function_type.release_value(), move(builder.request_url_modifiers) };
}

Optional<URL> RustComponentValueParser::parse_a_url_function(StringView input, StringView encoding)
{
    return parse_url_with_rust(input, encoding, FFI::rust_css_parse_url_function);
}

Optional<URL> RustComponentValueParser::parse_an_import_url(StringView input, StringView encoding)
{
    return parse_url_with_rust(input, encoding, FFI::rust_css_parse_import_url);
}

struct RustFontSourceBuilder {
    Optional<FFI::CssFontSourceKind> source_kind;
    Optional<RustComponentValueParser::FamilyName> family_name;
    Optional<URL::Type> url_function_type;
    Optional<String> url;
    Vector<RequestURLModifier> request_url_modifiers;
    Optional<FlyString> format;
    Vector<FontTech> tech;
};

Optional<RustComponentValueParser::FontSource> RustComponentValueParser::parse_a_font_source(StringView input, StringView encoding)
{
    RustFontSourceBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_source(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssFontSourceKind kind, u8 const* family_name_ptr, size_t family_name_len, bool family_name_is_string) {
            auto& builder = *static_cast<RustFontSourceBuilder*>(raw_builder);
            builder.source_kind = kind;
            if (kind == FFI::CssFontSourceKind::Local) {
                builder.family_name = FamilyName {
                    .name = fly_string_from_ffi_bytes(family_name_ptr, family_name_len),
                    .is_string = family_name_is_string,
                };
            }
        },
        [](void* raw_builder, FFI::CssUrlFunction const* rust_url_function) {
            auto& builder = *static_cast<RustFontSourceBuilder*>(raw_builder);
            builder.url_function_type = url_function_type_from_rust(rust_url_function->function_type);
            builder.url = string_from_ffi_bytes(rust_url_function->url_ptr, rust_url_function->url_len);
        },
        [](void* raw_builder, FFI::CssUrlModifier const* rust_modifier) {
            auto& builder = *static_cast<RustFontSourceBuilder*>(raw_builder);
            switch (rust_modifier->kind) {
            case FFI::CssUrlModifierKind::CrossOrigin:
                builder.request_url_modifiers.append(RequestURLModifier::create_cross_origin(cross_origin_modifier_value_from_rust(rust_modifier->cross_origin_value)));
                break;
            case FFI::CssUrlModifierKind::Integrity:
                builder.request_url_modifiers.append(RequestURLModifier::create_integrity(fly_string_from_ffi_bytes(rust_modifier->integrity_ptr, rust_modifier->integrity_len)));
                break;
            case FFI::CssUrlModifierKind::ReferrerPolicy:
                builder.request_url_modifiers.append(RequestURLModifier::create_referrer_policy(referrer_policy_modifier_value_from_rust(rust_modifier->referrer_policy_value)));
                break;
            }
        },
        [](void* raw_builder, u8 const* format_ptr, size_t format_len) {
            auto& builder = *static_cast<RustFontSourceBuilder*>(raw_builder);
            builder.format = fly_string_from_ffi_bytes(format_ptr, format_len);
        },
        [](void* raw_builder, FFI::CssFontTech rust_font_tech) {
            auto& builder = *static_cast<RustFontSourceBuilder*>(raw_builder);
            builder.tech.append(font_tech_from_rust(rust_font_tech));
        });

    if (!parsed || !builder.source_kind.has_value())
        return {};

    switch (*builder.source_kind) {
    case FFI::CssFontSourceKind::Local:
        if (!builder.family_name.has_value())
            return {};
        return FontSource {
            .source = builder.family_name.release_value(),
            .format = {},
            .tech = {},
        };
    case FFI::CssFontSourceKind::Url:
        if (!builder.url_function_type.has_value() || !builder.url.has_value())
            return {};
        return FontSource {
            .source = URL { builder.url.release_value(), builder.url_function_type.release_value(), move(builder.request_url_modifiers) },
            .format = builder.format,
            .tech = move(builder.tech),
        };
    }
    VERIFY_NOT_REACHED();
}

Optional<RustComponentValueParser::FontLanguageOverride> RustComponentValueParser::parse_a_font_language_override(StringView input, StringView encoding)
{
    Optional<FontLanguageOverride> font_language_override;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_language_override(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &font_language_override,
        [](void* raw_font_language_override, FFI::CssFontLanguageOverrideKind kind, u8 const* value_ptr, size_t value_len) {
            auto& font_language_override = *static_cast<Optional<FontLanguageOverride>*>(raw_font_language_override);
            Optional<FlyString> value;
            if (kind == FFI::CssFontLanguageOverrideKind::String)
                value = fly_string_from_ffi_bytes(value_ptr, value_len);
            font_language_override = FontLanguageOverride {
                .kind = kind,
                .value = move(value),
            };
        });

    if (!parsed)
        return {};

    return font_language_override;
}

Optional<FlyString> RustComponentValueParser::parse_an_opentype_tag(StringView input, StringView encoding)
{
    Optional<FlyString> opentype_tag;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_opentype_tag(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &opentype_tag,
        [](void* raw_opentype_tag, u8 const* value_ptr, size_t value_len) {
            auto& opentype_tag = *static_cast<Optional<FlyString>*>(raw_opentype_tag);
            opentype_tag = fly_string_from_ffi_bytes(value_ptr, value_len);
        });

    if (!parsed)
        return {};

    return opentype_tag;
}

static Optional<RustComponentValueParser::OpenTypeSettings> parse_open_type_settings_impl(StringView input, StringView encoding, bool is_variation_settings)
{
    RustComponentValueParser::OpenTypeSettings settings {};
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = (is_variation_settings ? FFI::rust_css_parse_font_variation_settings : FFI::rust_css_parse_font_feature_settings)(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &settings,
        [](void* raw_settings, FFI::CssOpenTypeSettingsKind kind) {
            auto& settings = *static_cast<RustComponentValueParser::OpenTypeSettings*>(raw_settings);
            settings.kind = kind;
        },
        [](void* raw_settings, u8 const* tag_ptr, size_t tag_len, FFI::CssOpenTypeTaggedValueKind value_kind, u8 const* value_ptr, size_t value_len) {
            auto& settings = *static_cast<RustComponentValueParser::OpenTypeSettings*>(raw_settings);
            Optional<String> value;
            if (value_kind == FFI::CssOpenTypeTaggedValueKind::Value)
                value = string_from_ffi_bytes(value_ptr, value_len);
            settings.tag_values.append({
                .tag = fly_string_from_ffi_bytes(tag_ptr, tag_len),
                .value_kind = value_kind,
                .value = move(value),
            });
        });

    if (!parsed)
        return {};

    return settings;
}

Optional<RustComponentValueParser::OpenTypeSettings> RustComponentValueParser::parse_font_feature_settings(StringView input, StringView encoding)
{
    return parse_open_type_settings_impl(input, encoding, false);
}

Optional<RustComponentValueParser::OpenTypeSettings> RustComponentValueParser::parse_font_variation_settings(StringView input, StringView encoding)
{
    return parse_open_type_settings_impl(input, encoding, true);
}

Optional<RustComponentValueParser::FontStyle> RustComponentValueParser::parse_a_font_style(StringView input, StringView encoding)
{
    Optional<FontStyle> font_style;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_style(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &font_style,
        [](void* raw_font_style, FFI::CssFontStyleKind kind, bool has_angle) {
            auto& font_style = *static_cast<Optional<FontStyle>*>(raw_font_style);
            font_style = FontStyle {
                .kind = kind,
                .has_angle = has_angle,
            };
        });

    if (!parsed)
        return {};

    return font_style;
}

Optional<Vector<RustComponentValueParser::FontVariantAlternatesValue>> RustComponentValueParser::parse_a_font_variant_alternates(StringView input, StringView encoding)
{
    Vector<FontVariantAlternatesValue> values;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_variant_alternates(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &values,
        [](void* raw_values, FFI::CssFontVariantAlternatesValueKind kind) {
            auto& values = *static_cast<Vector<FontVariantAlternatesValue>*>(raw_values);
            values.append({
                .kind = kind,
            });
        },
        [](void* raw_values, u8 const* value_ptr, size_t value_len) {
            auto& values = *static_cast<Vector<FontVariantAlternatesValue>*>(raw_values);
            VERIFY(!values.is_empty());
            values.last().feature_value_names.append(fly_string_from_ffi_bytes(value_ptr, value_len));
        });

    if (!parsed)
        return {};

    return values;
}

Optional<RustComponentValueParser::FontVariant> RustComponentValueParser::parse_a_font_variant(StringView input, StringView encoding)
{
    FontVariant font_variant;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_variant(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &font_variant,
        [](void* raw_font_variant, FFI::CssFontVariantSimpleValueKind kind, u8 const* value_ptr, size_t value_len) {
            auto& font_variant = *static_cast<FontVariant*>(raw_font_variant);
            switch (kind) {
            case FFI::CssFontVariantSimpleValueKind::LigaturesNone:
                font_variant.ligatures_none = true;
                break;
            case FFI::CssFontVariantSimpleValueKind::Caps:
                font_variant.caps = fly_string_from_ffi_bytes(value_ptr, value_len);
                break;
            case FFI::CssFontVariantSimpleValueKind::Emoji:
                font_variant.emoji = fly_string_from_ffi_bytes(value_ptr, value_len);
                break;
            case FFI::CssFontVariantSimpleValueKind::Position:
                font_variant.position = fly_string_from_ffi_bytes(value_ptr, value_len);
                break;
            }
        },
        [](void* raw_font_variant, FFI::CssFontVariantAlternatesValueKind kind) {
            auto& font_variant = *static_cast<FontVariant*>(raw_font_variant);
            if (!font_variant.alternates.has_value())
                font_variant.alternates = Vector<FontVariantAlternatesValue> {};
            font_variant.alternates->append({
                .kind = kind,
            });
        },
        [](void* raw_font_variant, u8 const* value_ptr, size_t value_len) {
            auto& font_variant = *static_cast<FontVariant*>(raw_font_variant);
            VERIFY(font_variant.alternates.has_value());
            VERIFY(!font_variant.alternates->is_empty());
            font_variant.alternates->last().feature_value_names.append(fly_string_from_ffi_bytes(value_ptr, value_len));
        },
        [](void* raw_font_variant, FFI::CssFontVariantEastAsianValueKind kind, u8 const* value_ptr, size_t value_len) {
            auto& font_variant = *static_cast<FontVariant*>(raw_font_variant);
            if (!font_variant.east_asian.has_value())
                font_variant.east_asian = Vector<FontVariantEastAsianValue> {};
            font_variant.east_asian->append({
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
            });
        },
        [](void* raw_font_variant, FFI::CssFontVariantNumericValueKind kind, u8 const* value_ptr, size_t value_len) {
            auto& font_variant = *static_cast<FontVariant*>(raw_font_variant);
            if (!font_variant.numeric.has_value())
                font_variant.numeric = Vector<FontVariantNumericValue> {};
            font_variant.numeric->append({
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
            });
        },
        [](void* raw_font_variant, FFI::CssFontVariantLigaturesValueKind kind, u8 const* value_ptr, size_t value_len) {
            auto& font_variant = *static_cast<FontVariant*>(raw_font_variant);
            if (!font_variant.ligatures.has_value())
                font_variant.ligatures = Vector<FontVariantLigaturesValue> {};
            font_variant.ligatures->append({
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
            });
        });

    if (!parsed)
        return {};

    return font_variant;
}

Optional<Vector<RustComponentValueParser::FontVariantEastAsianValue>> RustComponentValueParser::parse_a_font_variant_east_asian(StringView input, StringView encoding)
{
    Vector<FontVariantEastAsianValue> values;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_variant_east_asian(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &values,
        [](void* raw_values, FFI::CssFontVariantEastAsianValueKind kind, u8 const* value_ptr, size_t value_len) {
            auto& values = *static_cast<Vector<FontVariantEastAsianValue>*>(raw_values);
            values.append({
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
            });
        });

    if (!parsed)
        return {};

    return values;
}

Optional<Vector<RustComponentValueParser::FontVariantNumericValue>> RustComponentValueParser::parse_a_font_variant_numeric(StringView input, StringView encoding)
{
    Vector<FontVariantNumericValue> values;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_variant_numeric(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &values,
        [](void* raw_values, FFI::CssFontVariantNumericValueKind kind, u8 const* value_ptr, size_t value_len) {
            auto& values = *static_cast<Vector<FontVariantNumericValue>*>(raw_values);
            values.append({
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
            });
        });

    if (!parsed)
        return {};

    return values;
}

Optional<Vector<RustComponentValueParser::FontVariantLigaturesValue>> RustComponentValueParser::parse_a_font_variant_ligatures(StringView input, StringView encoding)
{
    Vector<FontVariantLigaturesValue> values;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_variant_ligatures(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &values,
        [](void* raw_values, FFI::CssFontVariantLigaturesValueKind kind, u8 const* value_ptr, size_t value_len) {
            auto& values = *static_cast<Vector<FontVariantLigaturesValue>*>(raw_values);
            values.append({
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
            });
        });

    if (!parsed)
        return {};

    return values;
}

Optional<Vector<RustComponentValueParser::FontFamilyValue>> RustComponentValueParser::parse_font_family_value(StringView input, StringView encoding)
{
    Vector<FontFamilyValue> family_values;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_family_value(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &family_values,
        [](void* raw_family_values, FFI::CssFontFamilyValueKind kind, u8 const* value_ptr, size_t value_len, bool is_string) {
            auto& family_values = *static_cast<Vector<FontFamilyValue>*>(raw_family_values);
            family_values.append({
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
                .is_string = is_string,
            });
        });

    if (!parsed)
        return {};

    return family_values;
}

Optional<FlyString> RustComponentValueParser::parse_a_layer_name(StringView input, StringView encoding, AllowBlankLayerName allow_blank_layer_name)
{
    Optional<FlyString> name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_layer_name(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        allow_blank_layer_name == AllowBlankLayerName::Yes,
        &name,
        [](void* raw_name, u8 const* name_ptr, size_t name_len) {
            auto& name = *static_cast<Optional<FlyString>*>(raw_name);
            name = fly_string_from_ffi_bytes(name_ptr, name_len);
        });

    if (!parsed)
        return {};

    return name;
}

Optional<FlyString> RustComponentValueParser::parse_an_import_layer(StringView input, StringView encoding)
{
    Optional<FlyString> name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_import_layer(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &name,
        [](void* raw_name, u8 const* name_ptr, size_t name_len) {
            auto& name = *static_cast<Optional<FlyString>*>(raw_name);
            name = fly_string_from_ffi_bytes(name_ptr, name_len);
        });

    if (!parsed)
        return {};

    return name;
}

Optional<Vector<FlyString>> RustComponentValueParser::parse_a_layer_name_list(StringView input, StringView encoding)
{
    Vector<FlyString> names;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_layer_name_list(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &names,
        [](void* raw_names, u8 const* name_ptr, size_t name_len) {
            auto& names = *static_cast<Vector<FlyString>*>(raw_names);
            names.append(fly_string_from_ffi_bytes(name_ptr, name_len));
        });

    if (!parsed)
        return {};

    return names;
}

Optional<FlyString> RustComponentValueParser::parse_a_counter_style_name(StringView input, StringView encoding)
{
    Optional<FlyString> name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_counter_style_name(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &name,
        [](void* raw_name, u8 const* name_ptr, size_t name_len) {
            auto& name = *static_cast<Optional<FlyString>*>(raw_name);
            name = fly_string_from_ffi_bytes(name_ptr, name_len);
        });

    if (!parsed)
        return {};

    return name;
}

Optional<RustComponentValueParser::CounterStyle> RustComponentValueParser::parse_a_counter_style(StringView input, StringView encoding)
{
    Optional<CounterStyle> counter_style;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_counter_style(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &counter_style,
        [](void* raw_counter_style, FFI::CssCounterStyleKind kind, FFI::CssCounterStyleSymbolsType symbols_type, u8 const* name_ptr, size_t name_len) {
            auto& counter_style = *static_cast<Optional<CounterStyle>*>(raw_counter_style);
            counter_style = CounterStyle {
                .kind = kind,
                .symbols_type = symbols_type,
                .name = fly_string_from_ffi_bytes(name_ptr, name_len),
                .symbols = {},
            };
        },
        [](void* raw_counter_style, u8 const* symbol_ptr, size_t symbol_len) {
            auto& counter_style = *static_cast<Optional<CounterStyle>*>(raw_counter_style);
            VERIFY(counter_style.has_value());
            counter_style->symbols.append(fly_string_from_ffi_bytes(symbol_ptr, symbol_len));
        });

    if (!parsed || !counter_style.has_value())
        return {};

    return counter_style;
}

Optional<FFI::CssNonnegativeIntegerSymbolPairOrder> RustComponentValueParser::parse_a_nonnegative_integer_symbol_pair(StringView input, StringView encoding)
{
    Optional<FFI::CssNonnegativeIntegerSymbolPairOrder> order;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_nonnegative_integer_symbol_pair(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &order,
        [](void* raw_order, FFI::CssNonnegativeIntegerSymbolPairOrder parsed_order) {
            auto& order = *static_cast<Optional<FFI::CssNonnegativeIntegerSymbolPairOrder>*>(raw_order);
            order = parsed_order;
        });

    if (!parsed || !order.has_value())
        return {};

    return order;
}

Optional<FFI::CssCounterStyleNegativeSymbolCount> RustComponentValueParser::parse_counter_style_negative(StringView input, StringView encoding)
{
    Optional<FFI::CssCounterStyleNegativeSymbolCount> count;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_counter_style_negative(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &count,
        [](void* raw_count, FFI::CssCounterStyleNegativeSymbolCount parsed_count) {
            auto& count = *static_cast<Optional<FFI::CssCounterStyleNegativeSymbolCount>*>(raw_count);
            count = parsed_count;
        });

    if (!parsed || !count.has_value())
        return {};

    return count;
}

Optional<FFI::CssCounterStyleSystemKind> RustComponentValueParser::parse_counter_style_system(StringView input, StringView encoding)
{
    Optional<FFI::CssCounterStyleSystemKind> system;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_counter_style_system(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &system,
        [](void* raw_system, FFI::CssCounterStyleSystemKind parsed_system) {
            auto& system = *static_cast<Optional<FFI::CssCounterStyleSystemKind>*>(raw_system);
            system = parsed_system;
        });

    if (!parsed || !system.has_value())
        return {};

    return system;
}

bool RustComponentValueParser::parse_counter_style_symbol(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_counter_style_symbol(filtered_input_bytes.data(), filtered_input_bytes.size());
}

Optional<size_t> RustComponentValueParser::parse_counter_style_symbols(StringView input, StringView encoding)
{
    Optional<size_t> count;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_counter_style_symbols(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &count,
        [](void* raw_count, size_t parsed_count) {
            auto& count = *static_cast<Optional<size_t>*>(raw_count);
            count = parsed_count;
        });

    if (!parsed || !count.has_value())
        return {};

    return count;
}

Optional<RustComponentValueParser::CounterStyleRangeSyntax> RustComponentValueParser::parse_counter_style_range(StringView input, StringView encoding)
{
    Optional<CounterStyleRangeSyntax> range;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_counter_style_range(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &range,
        [](void* raw_range, FFI::CssCounterStyleRangeKind kind, size_t count) {
            auto& range = *static_cast<Optional<CounterStyleRangeSyntax>*>(raw_range);
            range = CounterStyleRangeSyntax { .kind = kind, .count = count };
        });

    if (!parsed || !range.has_value())
        return {};

    return range;
}

Optional<size_t> RustComponentValueParser::parse_counter_style_additive_symbols(StringView input, StringView encoding)
{
    Optional<size_t> count;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_counter_style_additive_symbols(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &count,
        [](void* raw_count, size_t parsed_count) {
            auto& count = *static_cast<Optional<size_t>*>(raw_count);
            count = parsed_count;
        });

    if (!parsed || !count.has_value())
        return {};

    return count;
}

bool RustComponentValueParser::parse_string_descriptor(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_string_descriptor(filtered_input_bytes.data(), filtered_input_bytes.size());
}

bool RustComponentValueParser::parse_length_descriptor(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_length_descriptor(filtered_input_bytes.data(), filtered_input_bytes.size());
}

bool RustComponentValueParser::parse_positive_percentage_descriptor(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_positive_percentage_descriptor(filtered_input_bytes.data(), filtered_input_bytes.size());
}

bool RustComponentValueParser::parse_page_size_descriptor(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_page_size_descriptor(filtered_input_bytes.data(), filtered_input_bytes.size());
}

bool RustComponentValueParser::parse_optional_declaration_value_descriptor(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_optional_declaration_value_descriptor(filtered_input_bytes.data(), filtered_input_bytes.size());
}

Optional<FFI::CssCropOrCrossKind> RustComponentValueParser::parse_crop_or_cross(StringView input, StringView encoding)
{
    Optional<FFI::CssCropOrCrossKind> kind;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_crop_or_cross(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &kind,
        [](void* raw_kind, FFI::CssCropOrCrossKind parsed_kind) {
            auto& kind = *static_cast<Optional<FFI::CssCropOrCrossKind>*>(raw_kind);
            kind = parsed_kind;
        });

    if (!parsed || !kind.has_value())
        return {};

    return kind;
}

RustComponentValueParser::ColorScheme RustComponentValueParser::parse_color_scheme(StringView input, StringView encoding)
{
    Vector<String> schemes;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed_color_scheme = FFI::rust_css_parse_color_scheme(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &schemes,
        [](void* raw_schemes, u8 const* scheme_ptr, size_t scheme_len) {
            auto& schemes = *static_cast<Vector<String>*>(raw_schemes);
            schemes.append(string_from_ffi_bytes(scheme_ptr, scheme_len));
        });

    return ColorScheme {
        .kind = parsed_color_scheme.kind,
        .only = parsed_color_scheme.only,
        .schemes = move(schemes),
    };
}

RustComponentValueParser::AnchorNameOrScope RustComponentValueParser::parse_anchor_name_or_scope(StringView input, StringView encoding, bool allow_all)
{
    Vector<FlyString> names;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_anchor_name_or_scope(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        allow_all,
        &names,
        [](void* raw_names, u8 const* name_ptr, size_t name_len) {
            auto& names = *static_cast<Vector<FlyString>*>(raw_names);
            names.append(fly_string_from_ffi_bytes(name_ptr, name_len));
        });

    return AnchorNameOrScope {
        .kind = kind,
        .names = move(names),
    };
}

RustComponentValueParser::PositionAnchor RustComponentValueParser::parse_position_anchor(StringView input, StringView encoding)
{
    FlyString name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_position_anchor(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &name,
        [](void* raw_name, u8 const* name_ptr, size_t name_len) {
            auto& name = *static_cast<FlyString*>(raw_name);
            name = fly_string_from_ffi_bytes(name_ptr, name_len);
        });

    return PositionAnchor {
        .kind = kind,
        .name = move(name),
    };
}

RustComponentValueParser::TimelineScope RustComponentValueParser::parse_timeline_scope(StringView input, StringView encoding)
{
    Vector<FlyString> names;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_timeline_scope(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &names,
        [](void* raw_names, u8 const* name_ptr, size_t name_len) {
            auto& names = *static_cast<Vector<FlyString>*>(raw_names);
            names.append(fly_string_from_ffi_bytes(name_ptr, name_len));
        });

    return TimelineScope {
        .kind = kind,
        .names = move(names),
    };
}

RustComponentValueParser::TimelineName RustComponentValueParser::parse_timeline_name(StringView input, StringView encoding)
{
    Vector<TimelineNameItem> names;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_timeline_name(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &names,
        [](void* raw_names, FFI::CssTimelineNameItemKind kind, u8 const* name_ptr, size_t name_len) {
            auto& names = *static_cast<Vector<TimelineNameItem>*>(raw_names);
            names.append(TimelineNameItem {
                .kind = kind,
                .name = fly_string_from_ffi_bytes(name_ptr, name_len),
            });
        });

    return TimelineName {
        .kind = kind,
        .names = move(names),
    };
}

FFI::CssPositionTryOrderValue RustComponentValueParser::parse_position_try_order(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();
    return FFI::rust_css_parse_position_try_order(filtered_input_bytes.data(), filtered_input_bytes.size());
}

FFI::CssPositionVisibilityValue RustComponentValueParser::parse_position_visibility(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();
    return FFI::rust_css_parse_position_visibility(filtered_input_bytes.data(), filtered_input_bytes.size());
}

FFI::CssPaintOrderValue RustComponentValueParser::parse_paint_order(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();
    return FFI::rust_css_parse_paint_order(filtered_input_bytes.data(), filtered_input_bytes.size());
}

FFI::CssTextUnderlinePositionValue RustComponentValueParser::parse_text_underline_position(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();
    return FFI::rust_css_parse_text_underline_position(filtered_input_bytes.data(), filtered_input_bytes.size());
}

FFI::CssTouchActionValue RustComponentValueParser::parse_touch_action(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();
    return FFI::rust_css_parse_touch_action(filtered_input_bytes.data(), filtered_input_bytes.size());
}

FFI::CssScrollbarGutterValueKind RustComponentValueParser::parse_scrollbar_gutter(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();
    return FFI::rust_css_parse_scrollbar_gutter(filtered_input_bytes.data(), filtered_input_bytes.size());
}

RustComponentValueParser::Quotes RustComponentValueParser::parse_quotes(StringView input, StringView encoding)
{
    Vector<FlyString> strings;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_quotes(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &strings,
        [](void* raw_strings, u8 const* string_ptr, size_t string_len) {
            auto& strings = *static_cast<Vector<FlyString>*>(raw_strings);
            strings.append(fly_string_from_ffi_bytes(string_ptr, string_len));
        });

    return Quotes {
        .kind = kind,
        .strings = move(strings),
    };
}

RustComponentValueParser::WillChange RustComponentValueParser::parse_will_change(StringView input, StringView encoding)
{
    Vector<WillChangeFeature> features;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_will_change(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &features,
        [](void* raw_features, FFI::CssWillChangeFeatureKind kind, u8 const* value_ptr, size_t value_len) {
            auto& features = *static_cast<Vector<WillChangeFeature>*>(raw_features);
            features.append(WillChangeFeature {
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
            });
        });

    return WillChange {
        .kind = kind,
        .features = move(features),
    };
}

RustComponentValueParser::TransitionProperty RustComponentValueParser::parse_transition_property(StringView input, StringView encoding)
{
    Vector<FlyString> properties;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_transition_property(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &properties,
        [](void* raw_properties, u8 const* value_ptr, size_t value_len) {
            auto& properties = *static_cast<Vector<FlyString>*>(raw_properties);
            properties.append(fly_string_from_ffi_bytes(value_ptr, value_len));
        });

    return TransitionProperty {
        .kind = kind,
        .properties = move(properties),
    };
}

RustComponentValueParser::AnimationName RustComponentValueParser::parse_animation_name(StringView input, StringView encoding)
{
    Vector<AnimationNameItem> names;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_animation_name(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &names,
        [](void* raw_names, FFI::CssAnimationNameItemKind kind, u8 const* value_ptr, size_t value_len) {
            auto& names = *static_cast<Vector<AnimationNameItem>*>(raw_names);
            names.append(AnimationNameItem {
                .kind = kind,
                .value = fly_string_from_ffi_bytes(value_ptr, value_len),
            });
        });

    return AnimationName {
        .kind = kind,
        .names = move(names),
    };
}

RustComponentValueParser::ViewTransitionName RustComponentValueParser::parse_view_transition_name(StringView input, StringView encoding)
{
    FlyString name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto kind = FFI::rust_css_parse_view_transition_name(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &name,
        [](void* raw_name, u8 const* value_ptr, size_t value_len) {
            auto& name = *static_cast<FlyString*>(raw_name);
            name = fly_string_from_ffi_bytes(value_ptr, value_len);
        });

    return ViewTransitionName {
        .kind = kind,
        .name = move(name),
    };
}

FFI::CssContainValue RustComponentValueParser::parse_contain(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_contain(
        filtered_input_bytes.data(),
        filtered_input_bytes.size());
}

FFI::CssWhiteSpaceTrimValue RustComponentValueParser::parse_white_space_trim(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_white_space_trim(
        filtered_input_bytes.data(),
        filtered_input_bytes.size());
}

FFI::CssContainerTypeValueKind RustComponentValueParser::parse_container_type(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_container_type(
        filtered_input_bytes.data(),
        filtered_input_bytes.size());
}

Optional<size_t> RustComponentValueParser::parse_font_weight_absolute_pair(StringView input, StringView encoding)
{
    Optional<size_t> count;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_weight_absolute_pair(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &count,
        [](void* raw_count, size_t parsed_count) {
            auto& count = *static_cast<Optional<size_t>*>(raw_count);
            count = parsed_count;
        });

    if (!parsed || !count.has_value())
        return {};

    return count;
}

Optional<RustComponentValueParser::FamilyName> RustComponentValueParser::parse_a_family_name(StringView input, StringView encoding)
{
    Optional<FamilyName> family_name;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_family_name(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &family_name,
        [](void* raw_family_name, u8 const* name_ptr, size_t name_len, bool is_string) {
            auto& family_name = *static_cast<Optional<FamilyName>*>(raw_family_name);
            family_name = FamilyName {
                .name = fly_string_from_ffi_bytes(name_ptr, name_len),
                .is_string = is_string,
            };
        });

    if (!parsed)
        return {};

    return family_name;
}

Optional<RustComponentValueParser::NamespaceRulePrelude> RustComponentValueParser::parse_a_namespace_rule_prelude(StringView input, StringView encoding)
{
    NamespaceRulePrelude namespace_rule_prelude;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_namespace_rule_prelude(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &namespace_rule_prelude,
        [](void* raw_namespace_rule_prelude, u8 const* prefix_ptr, size_t prefix_len) {
            auto& namespace_rule_prelude = *static_cast<NamespaceRulePrelude*>(raw_namespace_rule_prelude);
            namespace_rule_prelude.prefix = fly_string_from_ffi_bytes(prefix_ptr, prefix_len);
        },
        [](void* raw_namespace_rule_prelude, u8 const* namespace_uri_ptr, size_t namespace_uri_len) {
            auto& namespace_rule_prelude = *static_cast<NamespaceRulePrelude*>(raw_namespace_rule_prelude);
            namespace_rule_prelude.namespace_uri = fly_string_from_ffi_bytes(namespace_uri_ptr, namespace_uri_len);
        });

    if (!parsed)
        return {};

    return namespace_rule_prelude;
}

Optional<Vector<FlyString>> RustComponentValueParser::parse_font_feature_values_family_name_list(StringView input, StringView encoding)
{
    Vector<FlyString> family_names;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_feature_values_family_name_list(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &family_names,
        [](void* raw_family_names, u8 const* family_name_ptr, size_t family_name_len) {
            auto& family_names = *static_cast<Vector<FlyString>*>(raw_family_names);
            family_names.append(fly_string_from_ffi_bytes(family_name_ptr, family_name_len));
        });

    if (!parsed)
        return {};

    return family_names;
}

Optional<Vector<RustComponentValueParser::ContainerRulePreludeCondition>> RustComponentValueParser::parse_container_rule_prelude(StringView input, StringView encoding)
{
    Vector<ContainerRulePreludeCondition> conditions;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_container_rule_prelude(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &conditions,
        [](void* raw_conditions, bool has_name, u8 const* name_ptr, size_t name_len, bool has_query, u8 const* query_ptr, size_t query_len) {
            auto& conditions = *static_cast<Vector<ContainerRulePreludeCondition>*>(raw_conditions);
            conditions.append({
                .name = has_name ? Optional<FlyString> { fly_string_from_ffi_bytes(name_ptr, name_len) } : OptionalNone {},
                .query = has_query ? Optional<String> { string_from_ffi_bytes(query_ptr, query_len) } : OptionalNone {},
            });
        });

    if (!parsed)
        return {};

    return conditions;
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
        set_original_value_text_for_custom_property(declaration);
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

static FFI::CssRuleContext rule_context_to_ffi(RuleContext context)
{
    switch (context) {
    case RuleContext::Unknown:
        return FFI::CssRuleContext::Unknown;
    case RuleContext::Style:
        return FFI::CssRuleContext::Style;
    case RuleContext::AtContainer:
        return FFI::CssRuleContext::AtContainer;
    case RuleContext::AtCounterStyle:
        return FFI::CssRuleContext::AtCounterStyle;
    case RuleContext::AtMedia:
        return FFI::CssRuleContext::AtMedia;
    case RuleContext::AtFontFace:
        return FFI::CssRuleContext::AtFontFace;
    case RuleContext::AtFontFeatureValues:
        return FFI::CssRuleContext::AtFontFeatureValues;
    case RuleContext::FontFeatureValue:
        return FFI::CssRuleContext::FontFeatureValue;
    case RuleContext::AtFunction:
        return FFI::CssRuleContext::AtFunction;
    case RuleContext::AtKeyframes:
        return FFI::CssRuleContext::AtKeyframes;
    case RuleContext::Keyframe:
        return FFI::CssRuleContext::Keyframe;
    case RuleContext::AtSupports:
        return FFI::CssRuleContext::AtSupports;
    case RuleContext::SupportsCondition:
        return FFI::CssRuleContext::SupportsCondition;
    case RuleContext::AtLayer:
        return FFI::CssRuleContext::AtLayer;
    case RuleContext::AtProperty:
        return FFI::CssRuleContext::AtProperty;
    case RuleContext::AtPage:
        return FFI::CssRuleContext::AtPage;
    case RuleContext::Margin:
        return FFI::CssRuleContext::Margin;
    }
    VERIFY_NOT_REACHED();
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
    Vector<RuleContext> rule_context;
    rule_context.append(RuleContext::Style);
    return parse_a_blocks_contents(input, encoding, rule_context);
}

Vector<RuleOrListOfDeclarations> RustComponentValueParser::parse_a_blocks_contents(StringView input, StringView encoding, Vector<RuleContext> const& rule_context)
{
    RuleEventBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    Vector<FFI::CssRuleContext> ffi_rule_context;
    ffi_rule_context.ensure_capacity(rule_context.size());
    for (auto context : rule_context)
        ffi_rule_context.unchecked_append(rule_context_to_ffi(context));

    FFI::rust_css_parse_block_contents_with_context(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        ffi_rule_context.data(),
        ffi_rule_context.size(),
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

Vector<Rule> RustComponentValueParser::parse_a_stylesheets_contents(StringView input, StringView encoding)
{
    RuleEventBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_stylesheet_contents(
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
    Vector<Rule> rules;
    for (auto& rule_or_list : builder.rules_or_lists_of_declarations) {
        VERIFY(rule_or_list.has<Rule>());
        rules.append(move(rule_or_list.get<Rule>()));
    }
    return rules;
}

}
