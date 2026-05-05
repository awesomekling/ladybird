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
