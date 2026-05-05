/*
 * Copyright (c) 2018-2022, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2020-2021, the SerenityOS developers.
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2022, MacDue <macdue@dueutil.tech>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StdLibExtras.h>
#include <AK/StringBuilder.h>
#include <LibWeb/CSS/CSSFunctionDeclarations.h>
#include <LibWeb/CSS/CSSMediaRule.h>
#include <LibWeb/CSS/CSSNestedDeclarations.h>
#include <LibWeb/CSS/MediaList.h>
#include <LibWeb/CSS/MediaQuery.h>
#include <LibWeb/CSS/Parser/ErrorReporter.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/Serialize.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>

namespace Web::CSS::Parser {

static void serialize_component_value_for_reparsing(StringBuilder& builder, ComponentValue const& component_value)
{
    if (component_value.is_token()) {
        auto original_source_text = component_value.original_source_text();
        builder.append(original_source_text.is_empty() ? component_value.to_string() : original_source_text);
        return;
    }

    if (component_value.is_block()) {
        auto const& block = component_value.block();
        builder.append(block.token.bracket_string());
        for (auto const& child : block.value)
            serialize_component_value_for_reparsing(builder, child);
        builder.append(block.token.bracket_mirror_string());
        return;
    }

    if (component_value.is_function()) {
        auto const& function = component_value.function();
        serialize_an_identifier(builder, function.name);
        builder.append('(');
        for (auto const& child : function.value)
            serialize_component_value_for_reparsing(builder, child);
        builder.append(')');
        return;
    }

    builder.append(component_value.to_string());
}

static String serialize_component_values_for_reparsing(Vector<ComponentValue> const& component_values)
{
    StringBuilder builder;
    for (auto const& component_value : component_values)
        serialize_component_value_for_reparsing(builder, component_value);
    return builder.to_string_without_validation();
}

Vector<NonnullRefPtr<MediaQuery>> Parser::parse_as_media_query_list()
{
    // https://www.w3.org/TR/mediaqueries-4/#mq-list

    // AD-HOC: Ignore whitespace-only queries
    // to make `@media {..}` equivalent to `@media all {..}`
    m_token_stream.discard_whitespace();
    if (!m_token_stream.has_next_token())
        return {};

    return parse_a_media_query_list_from_string(m_input, m_encoding);
}

template<typename T>
Vector<NonnullRefPtr<MediaQuery>> Parser::parse_a_media_query_list(TokenStream<T>& tokens)
{
    // https://www.w3.org/TR/mediaqueries-4/#mq-list

    // AD-HOC: Ignore whitespace-only queries
    // to make `@media {..}` equivalent to `@media all {..}`
    tokens.discard_whitespace();
    if (!tokens.has_next_token())
        return {};

    StringBuilder serialized_media_query_list;
    while (tokens.has_next_token())
        serialized_media_query_list.append(tokens.consume_a_token().original_source_text());

    return parse_a_media_query_list_from_string(serialized_media_query_list.string_view(), "utf-8"sv);
}

Vector<NonnullRefPtr<MediaQuery>> Parser::parse_a_media_query_list_from_string(StringView input, StringView encoding)
{
    AK::Vector<NonnullRefPtr<MediaQuery>> media_queries;
    auto rust_media_queries = RustComponentValueParser::parse_a_media_query_list(input, encoding, [this](RustComponentValueParser::MediaFeatureTest&& media_feature_test, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
        return materialize_rust_media_feature_test(move(media_feature_test));
    });

    for (auto& rust_media_query : rust_media_queries) {
        if (!rust_media_query.is_valid) {
            // "A media query that does not match the grammar in the previous section must be replaced by `not all`
            // during parsing." - https://www.w3.org/TR/mediaqueries-5/#error-handling
            media_queries.append(MediaQuery::create_not_all());
            continue;
        }

        auto media_query = MediaQuery::create();
        media_query->m_negated = rust_media_query.is_negated;
        if (rust_media_query.media_type.has_value())
            media_query->m_media_type = rust_media_query.media_type.release_value();
        if (rust_media_query.media_condition)
            media_query->m_media_condition = move(rust_media_query.media_condition);
        media_queries.append(media_query);
    }

    return media_queries;
}

RefPtr<MediaQuery> Parser::parse_as_media_query()
{
    // https://www.w3.org/TR/cssom-1/#parse-a-media-query
    auto media_query_list = parse_as_media_query_list();
    if (media_query_list.is_empty())
        return MediaQuery::create_not_all();
    if (media_query_list.size() == 1)
        return media_query_list.first();
    return nullptr;
}

OwnPtr<MediaFeature> Parser::materialize_rust_media_feature_test(RustComponentValueParser::MediaFeatureTest&& media_feature_test)
{
    auto media_feature_id_from_rust = [](FFI::CssMediaFeature const& media_feature) -> Optional<MediaFeatureID> {
        return media_feature_id_from_u8(media_feature.id);
    };

    auto parse_rust_media_feature_value = [this](MediaFeatureID media_feature_id, Vector<ComponentValue>& component_values) -> Optional<MediaFeatureValue> {
        TokenStream value_tokens { component_values };
        auto maybe_value = parse_media_feature_value(media_feature_id, value_tokens);
        if (!maybe_value.has_value())
            return {};
        value_tokens.discard_whitespace();
        if (value_tokens.has_next_token())
            return {};
        return maybe_value.release_value();
    };

    auto media_feature_comparison_from_rust = [](FFI::CssMediaFeatureComparison comparison) -> MediaFeature::Comparison {
        switch (comparison) {
        case FFI::CssMediaFeatureComparison::Equal:
            return MediaFeature::Comparison::Equal;
        case FFI::CssMediaFeatureComparison::LessThan:
            return MediaFeature::Comparison::LessThan;
        case FFI::CssMediaFeatureComparison::LessThanOrEqual:
            return MediaFeature::Comparison::LessThanOrEqual;
        case FFI::CssMediaFeatureComparison::GreaterThan:
            return MediaFeature::Comparison::GreaterThan;
        case FFI::CssMediaFeatureComparison::GreaterThanOrEqual:
            return MediaFeature::Comparison::GreaterThanOrEqual;
        }
        VERIFY_NOT_REACHED();
    };

    if (media_feature_test.feature.syntax_kind == FFI::CssMediaFeatureSyntaxKind::Boolean) {
        auto maybe_media_feature_id = media_feature_id_from_rust(media_feature_test.feature);
        if (!maybe_media_feature_id.has_value())
            return nullptr;
        return MediaFeature::boolean(maybe_media_feature_id.value());
    }

    if (media_feature_test.feature.syntax_kind == FFI::CssMediaFeatureSyntaxKind::Plain) {
        auto maybe_media_feature_id = media_feature_id_from_rust(media_feature_test.feature);
        if (!maybe_media_feature_id.has_value())
            return nullptr;
        auto media_feature_id = maybe_media_feature_id.value();
        auto maybe_value = parse_rust_media_feature_value(media_feature_id, media_feature_test.value);
        if (!maybe_value.has_value())
            return nullptr;

        switch (media_feature_test.feature.name_kind) {
        case FFI::CssMediaFeatureNameKind::Normal:
            return MediaFeature::plain(media_feature_id, maybe_value.release_value());
        case FFI::CssMediaFeatureNameKind::Min:
            return MediaFeature::min(media_feature_id, maybe_value.release_value());
        case FFI::CssMediaFeatureNameKind::Max:
            return MediaFeature::max(media_feature_id, maybe_value.release_value());
        }
        VERIFY_NOT_REACHED();
    }

    if (media_feature_test.feature.syntax_kind == FFI::CssMediaFeatureSyntaxKind::HalfRangeNameFirst) {
        auto maybe_media_feature_id = media_feature_id_from_rust(media_feature_test.feature);
        if (!maybe_media_feature_id.has_value())
            return nullptr;
        auto media_feature_id = maybe_media_feature_id.value();
        auto maybe_value = parse_rust_media_feature_value(media_feature_id, media_feature_test.value);
        if (!maybe_value.has_value() || maybe_value->is_ident())
            return nullptr;
        return MediaFeature::half_range(media_feature_id, media_feature_comparison_from_rust(media_feature_test.feature.comparison), maybe_value.release_value());
    }

    if (media_feature_test.feature.syntax_kind == FFI::CssMediaFeatureSyntaxKind::HalfRangeValueFirst) {
        auto maybe_media_feature_id = media_feature_id_from_rust(media_feature_test.feature);
        if (!maybe_media_feature_id.has_value())
            return nullptr;
        auto media_feature_id = maybe_media_feature_id.value();
        auto maybe_value = parse_rust_media_feature_value(media_feature_id, media_feature_test.value);
        if (!maybe_value.has_value())
            return nullptr;
        return MediaFeature::half_range(maybe_value.release_value(), media_feature_comparison_from_rust(media_feature_test.feature.comparison), media_feature_id);
    }

    if (media_feature_test.feature.syntax_kind == FFI::CssMediaFeatureSyntaxKind::Range) {
        auto maybe_media_feature_id = media_feature_id_from_rust(media_feature_test.feature);
        if (!maybe_media_feature_id.has_value())
            return nullptr;
        auto media_feature_id = maybe_media_feature_id.value();
        auto maybe_left_value = parse_rust_media_feature_value(media_feature_id, media_feature_test.left_value);
        if (!maybe_left_value.has_value())
            return nullptr;
        auto maybe_right_value = parse_rust_media_feature_value(media_feature_id, media_feature_test.right_value);
        if (!maybe_right_value.has_value())
            return nullptr;

        auto left_comparison = media_feature_comparison_from_rust(media_feature_test.feature.left_comparison);
        if (left_comparison == MediaFeature::Comparison::Equal || maybe_left_value->is_ident() || maybe_right_value->is_ident())
            return nullptr;
        return MediaFeature::range(maybe_left_value.release_value(), left_comparison, media_feature_id, media_feature_comparison_from_rust(media_feature_test.feature.right_comparison), maybe_right_value.release_value());
    }

    return nullptr;
}

OwnPtr<BooleanExpression> Parser::materialize_rust_media_condition(Vector<ComponentValue> const& component_values)
{
    auto serialized_media_condition = serialize_component_values_for_reparsing(component_values);

    auto media_condition = RustComponentValueParser::parse_a_media_condition(serialized_media_condition.bytes_as_string_view(), "utf-8"sv, [this](RustComponentValueParser::MediaFeatureTest&& media_feature_test, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
        return materialize_rust_media_feature_test(move(media_feature_test));
    });

    return media_condition;
}

OwnPtr<MediaFeature> Parser::materialize_rust_media_feature(Vector<ComponentValue> const& component_values)
{
    auto serialized_media_feature = serialize_component_values_for_reparsing(component_values);

    auto media_feature_test = RustComponentValueParser::parse_a_media_feature(serialized_media_feature.bytes_as_string_view(), "utf-8"sv);
    if (!media_feature_test.has_value())
        return {};

    auto media_feature = materialize_rust_media_feature_test(media_feature_test.release_value());
    if (!media_feature)
        return {};

    return media_feature;
}

static bool is_media_feature_value_token(ComponentValue const& component_value)
{
    if (!component_value.is_token())
        return true;
    switch (component_value.token().type()) {
    case Token::Type::Ident:
    case Token::Type::Function:
    case Token::Type::AtKeyword:
    case Token::Type::Hash:
    case Token::Type::String:
    case Token::Type::BadString:
    case Token::Type::Url:
    case Token::Type::BadUrl:
    case Token::Type::Number:
    case Token::Type::Percentage:
    case Token::Type::Dimension:
    case Token::Type::Whitespace:
    case Token::Type::Comma:
        return true;
    case Token::Type::Delim:
        // FIXME: What list of delimiters should we actually allow here?
        return !first_is_one_of(component_value.token().delim(), static_cast<u32>('<'), static_cast<u32>('>'), static_cast<u32>('='));
    case Token::Type::Invalid:
    case Token::Type::EndOfFile:
    case Token::Type::CDO:
    case Token::Type::CDC:
    case Token::Type::Colon:
    case Token::Type::Semicolon:
    case Token::Type::OpenSquare:
    case Token::Type::CloseSquare:
    case Token::Type::OpenParen:
    case Token::Type::CloseParen:
    case Token::Type::OpenCurly:
    case Token::Type::CloseCurly:
        return false;
    }
    VERIFY_NOT_REACHED();
}

// `<mf-value>`, https://www.w3.org/TR/mediaqueries-4/#typedef-mf-value
Optional<MediaFeatureValue> Parser::parse_media_feature_value(MediaFeatureID media_feature, TokenStream<ComponentValue>& tokens)
{
    {
        auto transaction = tokens.begin_transaction();
        auto value = [this](MediaFeatureID media_feature, TokenStream<ComponentValue>& tokens) -> Optional<MediaFeatureValue> {
            auto context_guard = push_temporary_value_parsing_context(SpecialContext::MediaCondition);

            // One branch for each member of the MediaFeatureValueType enum:
            // Identifiers
            if (tokens.next_token().is(Token::Type::Ident)) {
                auto transaction = tokens.begin_transaction();
                tokens.discard_whitespace();
                auto keyword = parse_keyword_value(tokens);
                if (keyword && media_feature_accepts_keyword(media_feature, keyword->to_keyword())) {
                    transaction.commit();
                    return MediaFeatureValue(MediaFeatureValue::Type::Ident, keyword.release_nonnull());
                }
            }

            // Boolean (<mq-boolean> in the spec: a 1 or 0)
            if (media_feature_accepts_type(media_feature, MediaFeatureValueType::Boolean)) {
                auto transaction = tokens.begin_transaction();
                tokens.discard_whitespace();
                if (auto integer = parse_integer_value(tokens, infinite_integer_range)) {
                    if (integer->is_calculated() || first_is_one_of(integer->as_integer().integer(), 0, 1)) {
                        transaction.commit();
                        return MediaFeatureValue(MediaFeatureValue::Type::Integer, integer.release_nonnull());
                    }
                }
            }

            // Integer
            if (media_feature_accepts_type(media_feature, MediaFeatureValueType::Integer)) {
                auto transaction = tokens.begin_transaction();
                if (auto integer = parse_integer_value(tokens, infinite_integer_range)) {
                    transaction.commit();
                    return MediaFeatureValue(MediaFeatureValue::Type::Integer, integer.release_nonnull());
                }
            }

            // Length
            if (media_feature_accepts_type(media_feature, MediaFeatureValueType::Length)) {
                auto transaction = tokens.begin_transaction();
                tokens.discard_whitespace();
                if (auto length = parse_length_value(tokens, infinite_range)) {
                    transaction.commit();
                    return MediaFeatureValue(MediaFeatureValue::Type::Length, length.release_nonnull());
                }

                // https://drafts.csswg.org/mediaqueries-5/#typedef-mf-value
                // <mf-value> = <number> | <dimension> | <ident> | <ratio>
                //
                // https://drafts.csswg.org/css-values-4/#lengths
                // "For zero lengths the unit identifier is optional"
                //
                // https://drafts.csswg.org/css-values-4/#zero-value
                // "Values of '0' can be written without units, even if the
                // value type doesn't allow 'unitless zeroes'."
                if (tokens.has_next_token()) {
                    auto const& token = tokens.next_token();
                    if (auto calc = parse_calculated_value(token, { .accepted_ranges_by_type = { { ValueType::Number, infinite_range } } }); calc && calc->as_calculated().resolves_to_number()) {
                        if (auto resolved_number = calc->as_calculated().resolve_number({}); resolved_number.has_value() && *resolved_number == 0) {
                            tokens.discard_a_token();
                            transaction.commit();
                            return MediaFeatureValue(MediaFeatureValue::Type::Length, LengthStyleValue::create(Length::make_px(0)));
                        }
                    }
                }
            }

            // Ratio
            if (media_feature_accepts_type(media_feature, MediaFeatureValueType::Ratio)) {
                auto transaction = tokens.begin_transaction();
                tokens.discard_whitespace();
                if (auto ratio = parse_ratio_value(tokens)) {
                    transaction.commit();
                    return MediaFeatureValue(MediaFeatureValue::Type::Ratio, ratio.release_nonnull());
                }
            }

            // Resolution
            if (media_feature_accepts_type(media_feature, MediaFeatureValueType::Resolution)) {
                auto transaction = tokens.begin_transaction();
                tokens.discard_whitespace();
                if (auto resolution = parse_resolution_value(tokens, infinite_range)) {
                    transaction.commit();
                    return MediaFeatureValue(MediaFeatureValue::Type::Resolution, resolution.release_nonnull());
                }
            }

            return {};
        }(media_feature, tokens);

        if (value.has_value()) {
            tokens.discard_whitespace();

            // Only returned the value if there are no trailing tokens.
            // Otherwise, the transaction gets reverted and we consume all the value tokens below.
            if (!is_media_feature_value_token(tokens.next_token())) {
                transaction.commit();
                return value.release_value();
            }
        }
    }

    // Parsing failed somehow, so wrap all the tokens into an "unknown" MediaFeatureValue if possible.

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();
    Vector<ComponentValue> unknown_tokens;

    // Consume any tokens that could be part of a value.
    while (tokens.has_next_token()) {
        if (is_media_feature_value_token(tokens.next_token())) {
            unknown_tokens.append(tokens.consume_a_token());
        } else {
            break;
        }
    }

    if (!unknown_tokens.is_empty()) {
        transaction.commit();
        ErrorReporter::the().report(InvalidValueError {
            .value_type = "<mf-value>"_fly_string,
            .value_string = MUST(String::join(""sv, unknown_tokens)),
            .description = "Unrecognized type"_string,
        });
        // NB: We only use this for serialization so the substitution function presence is irrelevant and we can just
        //     set it to empty.
        return MediaFeatureValue(MediaFeatureValue::Type::Unknown, move(UnresolvedStyleValue::create(move(unknown_tokens), {})));
    }

    return {};
}

template<typename NestedDeclarationsRule>
GC::Ptr<CSSMediaRule> Parser::convert_to_media_rule(AtRule const& rule, Nested nested)
{
    // https://drafts.csswg.org/css-conditional-3/#at-media
    // @media <media-query-list> {
    // <rule-list>
    // }
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@media"_fly_string,
            .prelude = MUST(String::join(""sv, rule.prelude)),
            .description = "Expected a block."_string,
        });
        return nullptr;
    }

    auto media_query_tokens = TokenStream { rule.prelude };
    auto media_query_list = parse_a_media_query_list(media_query_tokens);
    auto media_list = MediaList::create(realm(), move(media_query_list));

    GC::RootVector<GC::Ref<CSSRule>> child_rules { realm().heap() };
    for (auto const& child : rule.child_rules_and_lists_of_declarations) {
        child.visit(
            [&](Rule const& rule) {
                if (auto child_rule = convert_to_rule<NestedDeclarationsRule>(rule, nested))
                    child_rules.append(*child_rule);
            },
            [&](Vector<Declaration> const& declarations) {
                child_rules.append(NestedDeclarationsRule::create(realm(), *this, declarations));
            });
    }
    auto rule_list = CSSRuleList::create(realm(), child_rules);
    return CSSMediaRule::create(realm(), media_list, rule_list);
}

template GC::Ptr<CSSMediaRule> Parser::convert_to_media_rule<CSSNestedDeclarations>(AtRule const&, Parser::Nested);
template GC::Ptr<CSSMediaRule> Parser::convert_to_media_rule<CSSFunctionDeclarations>(AtRule const&, Parser::Nested);

}
