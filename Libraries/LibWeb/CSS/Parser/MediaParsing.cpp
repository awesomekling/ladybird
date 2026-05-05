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
    return parse_a_media_query_list_from_string(m_input, m_encoding);
}

template<typename T>
Vector<NonnullRefPtr<MediaQuery>> Parser::parse_a_media_query_list(TokenStream<T>& tokens)
{
    // https://www.w3.org/TR/mediaqueries-4/#mq-list

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

    auto materialize_unknown_media_feature_value = [](Vector<ComponentValue> const& component_values) -> MediaFeatureValue {
        ErrorReporter::the().report(InvalidValueError {
            .value_type = "<mf-value>"_fly_string,
            .value_string = MUST(String::join(""sv, component_values)),
            .description = "Unrecognized type"_string,
        });

        // NB: We only use this for serialization so the substitution function presence is irrelevant and we can just
        //     set it to empty.
        Vector<ComponentValue> unknown_tokens;
        unknown_tokens.ensure_capacity(component_values.size());
        for (auto const& component_value : component_values)
            unknown_tokens.unchecked_append(component_value);
        return MediaFeatureValue(MediaFeatureValue::Type::Unknown, UnresolvedStyleValue::create(move(unknown_tokens), {}));
    };

    auto materialize_rust_media_feature_value = [this, &materialize_unknown_media_feature_value](FFI::CssMediaFeatureValueSyntaxKind syntax_kind, Vector<ComponentValue>& component_values) -> Optional<MediaFeatureValue> {
        if (syntax_kind == FFI::CssMediaFeatureValueSyntaxKind::Invalid)
            return {};

        if (syntax_kind == FFI::CssMediaFeatureValueSyntaxKind::Unknown)
            return materialize_unknown_media_feature_value(component_values);

        TokenStream value_tokens { component_values };
        auto maybe_value = [&]() -> Optional<MediaFeatureValue> {
            auto context_guard = push_temporary_value_parsing_context(SpecialContext::MediaCondition);

            switch (syntax_kind) {
            case FFI::CssMediaFeatureValueSyntaxKind::Ident: {
                value_tokens.discard_whitespace();
                auto keyword = parse_keyword_value(value_tokens);
                if (keyword)
                    return MediaFeatureValue(MediaFeatureValue::Type::Ident, keyword.release_nonnull());
                return {};
            }
            case FFI::CssMediaFeatureValueSyntaxKind::Boolean: {
                value_tokens.discard_whitespace();
                if (auto integer = parse_integer_value(value_tokens, infinite_integer_range)) {
                    if (integer->is_calculated() || first_is_one_of(integer->as_integer().integer(), 0, 1))
                        return MediaFeatureValue(MediaFeatureValue::Type::Integer, integer.release_nonnull());
                }
                return {};
            }
            case FFI::CssMediaFeatureValueSyntaxKind::Integer:
                if (auto integer = parse_integer_value(value_tokens, infinite_integer_range))
                    return MediaFeatureValue(MediaFeatureValue::Type::Integer, integer.release_nonnull());
                return {};
            case FFI::CssMediaFeatureValueSyntaxKind::Length: {
                value_tokens.discard_whitespace();
                if (auto length = parse_length_value(value_tokens, infinite_range))
                    return MediaFeatureValue(MediaFeatureValue::Type::Length, length.release_nonnull());

                if (value_tokens.has_next_token()) {
                    auto const& token = value_tokens.next_token();
                    if (auto calc = parse_calculated_value(token, { .accepted_ranges_by_type = { { ValueType::Number, infinite_range } } }); calc && calc->as_calculated().resolves_to_number()) {
                        if (auto resolved_number = calc->as_calculated().resolve_number({}); resolved_number.has_value() && *resolved_number == 0) {
                            value_tokens.discard_a_token();
                            return MediaFeatureValue(MediaFeatureValue::Type::Length, LengthStyleValue::create(Length::make_px(0)));
                        }
                    }
                }
                return {};
            }
            case FFI::CssMediaFeatureValueSyntaxKind::Ratio: {
                value_tokens.discard_whitespace();
                if (auto ratio = parse_ratio_value(value_tokens))
                    return MediaFeatureValue(MediaFeatureValue::Type::Ratio, ratio.release_nonnull());
                return {};
            }
            case FFI::CssMediaFeatureValueSyntaxKind::Resolution: {
                value_tokens.discard_whitespace();
                if (auto resolution = parse_resolution_value(value_tokens, infinite_range))
                    return MediaFeatureValue(MediaFeatureValue::Type::Resolution, resolution.release_nonnull());
                return {};
            }
            case FFI::CssMediaFeatureValueSyntaxKind::Unknown:
            case FFI::CssMediaFeatureValueSyntaxKind::Invalid:
                VERIFY_NOT_REACHED();
            }
            VERIFY_NOT_REACHED();
        }();

        if (!maybe_value.has_value())
            return materialize_unknown_media_feature_value(component_values);
        value_tokens.discard_whitespace();
        if (value_tokens.has_next_token())
            return materialize_unknown_media_feature_value(component_values);
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
        auto maybe_value = materialize_rust_media_feature_value(media_feature_test.value_syntax_kind, media_feature_test.value);
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
        auto maybe_value = materialize_rust_media_feature_value(media_feature_test.value_syntax_kind, media_feature_test.value);
        if (!maybe_value.has_value())
            return nullptr;
        return MediaFeature::half_range(media_feature_id, media_feature_comparison_from_rust(media_feature_test.feature.comparison), maybe_value.release_value());
    }

    if (media_feature_test.feature.syntax_kind == FFI::CssMediaFeatureSyntaxKind::HalfRangeValueFirst) {
        auto maybe_media_feature_id = media_feature_id_from_rust(media_feature_test.feature);
        if (!maybe_media_feature_id.has_value())
            return nullptr;
        auto media_feature_id = maybe_media_feature_id.value();
        auto maybe_value = materialize_rust_media_feature_value(media_feature_test.value_syntax_kind, media_feature_test.value);
        if (!maybe_value.has_value())
            return nullptr;
        return MediaFeature::half_range(maybe_value.release_value(), media_feature_comparison_from_rust(media_feature_test.feature.comparison), media_feature_id);
    }

    if (media_feature_test.feature.syntax_kind == FFI::CssMediaFeatureSyntaxKind::Range) {
        auto maybe_media_feature_id = media_feature_id_from_rust(media_feature_test.feature);
        if (!maybe_media_feature_id.has_value())
            return nullptr;
        auto media_feature_id = maybe_media_feature_id.value();
        auto maybe_left_value = materialize_rust_media_feature_value(media_feature_test.left_value_syntax_kind, media_feature_test.left_value);
        if (!maybe_left_value.has_value())
            return nullptr;
        auto maybe_right_value = materialize_rust_media_feature_value(media_feature_test.right_value_syntax_kind, media_feature_test.right_value);
        if (!maybe_right_value.has_value())
            return nullptr;

        auto left_comparison = media_feature_comparison_from_rust(media_feature_test.feature.left_comparison);
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

OwnPtr<BooleanExpression> Parser::materialize_rust_media_test(Vector<ComponentValue> const& component_values)
{
    auto serialized_media_test = serialize_component_values_for_reparsing(component_values);

    return RustComponentValueParser::parse_a_media_test(serialized_media_test.bytes_as_string_view(), "utf-8"sv, [this](RustComponentValueParser::MediaFeatureTest&& media_feature_test, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
        return materialize_rust_media_feature_test(move(media_feature_test));
    });
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
