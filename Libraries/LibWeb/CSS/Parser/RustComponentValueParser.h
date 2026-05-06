/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/Optional.h>
#include <AK/OwnPtr.h>
#include <AK/StringView.h>
#include <AK/Variant.h>
#include <AK/Vector.h>
#include <LibGfx/Font/UnicodeRange.h>
#include <LibWeb/CSS/BooleanExpression.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/MediaQuery.h>
#include <LibWeb/CSS/PageSelector.h>
#include <LibWeb/CSS/Parser/ComponentValue.h>
#include <LibWeb/CSS/Parser/RuleContext.h>
#include <LibWeb/CSS/Parser/SyntaxParsing.h>
#include <LibWeb/CSS/Parser/TokenStream.h>
#include <LibWeb/CSS/Parser/Types.h>
#include <LibWeb/CSS/Percentage.h>
#include <LibWeb/CSS/URL.h>
#include <LibWeb/Export.h>
#include <LibWeb/RustFFI.h>

namespace Web::CSS::Parser {

class WEB_API RustComponentValueParser {
public:
    struct MediaFeatureTest {
        FFI::CssMediaFeature feature;
        FFI::CssMediaFeatureValueSyntaxKind value_syntax_kind;
        FFI::CssMediaFeatureValueSyntaxKind left_value_syntax_kind;
        FFI::CssMediaFeatureValueSyntaxKind right_value_syntax_kind;
        Vector<ComponentValue> value;
        Vector<ComponentValue> left_value;
        Vector<ComponentValue> right_value;
    };

    struct MediaQuerySyntax {
        bool is_negated { false };
        Optional<MediaQuery::MediaType> media_type;
        OwnPtr<BooleanExpression> media_condition;
    };

    struct NamespaceRulePrelude {
        Optional<FlyString> prefix;
        FlyString namespace_uri;
    };

    struct ContainerRulePreludeCondition {
        Optional<FlyString> name;
        Optional<String> query;
    };

    struct FamilyName {
        FlyString name;
        bool is_string { false };
    };

    struct FontSource {
        Variant<FamilyName, URL> source;
        Optional<FlyString> format;
        Vector<FontTech> tech;
    };

    struct FontLanguageOverride {
        FFI::CssFontLanguageOverrideKind kind;
        Optional<FlyString> value;
    };

    struct FontFamilyValue {
        FFI::CssFontFamilyValueKind kind;
        FlyString value;
        bool is_string { false };
    };

    struct FontStyle {
        FFI::CssFontStyleKind kind;
        bool has_angle { false };
    };

    struct FontVariantAlternatesValue {
        FFI::CssFontVariantAlternatesValueKind kind;
        Vector<FlyString> feature_value_names;
    };

    struct FontVariantEastAsianValue {
        FFI::CssFontVariantEastAsianValueKind kind;
        FlyString value;
    };

    struct FontVariantNumericValue {
        FFI::CssFontVariantNumericValueKind kind;
        FlyString value;
    };

    struct FontVariantLigaturesValue {
        FFI::CssFontVariantLigaturesValueKind kind;
        FlyString value;
    };

    struct FontVariant {
        bool ligatures_none { false };
        Optional<Vector<FontVariantAlternatesValue>> alternates;
        Optional<FlyString> caps;
        Optional<Vector<FontVariantEastAsianValue>> east_asian;
        Optional<FlyString> emoji;
        Optional<Vector<FontVariantLigaturesValue>> ligatures;
        Optional<Vector<FontVariantNumericValue>> numeric;
        Optional<FlyString> position;
    };

    struct OpenTypeTaggedValue {
        FlyString tag;
        FFI::CssOpenTypeTaggedValueKind value_kind;
        Optional<String> value;
    };

    struct OpenTypeSettings {
        FFI::CssOpenTypeSettingsKind kind;
        Vector<OpenTypeTaggedValue> tag_values;
    };

    enum class AllowBlankLayerName : u8 {
        No,
        Yes,
    };

    struct CounterStyle {
        FFI::CssCounterStyleKind kind;
        FFI::CssCounterStyleSymbolsType symbols_type;
        FlyString name;
        Vector<FlyString> symbols;
    };

    static Optional<ComponentValue> parse_a_component_value(StringView input, StringView encoding);
    static Vector<ComponentValue> parse_a_list_of_component_values(StringView input, StringView encoding);
    static Vector<Vector<ComponentValue>> parse_a_comma_separated_list_of_component_values(StringView input, StringView encoding);
    static FFI::CssValueTypeSyntaxKind parse_a_value_type(u8 value_type_id, TokenStream<ComponentValue>&);
    static OwnPtr<SyntaxNode> parse_as_syntax(StringView input, StringView encoding, LimitSingleComponentIdentToCustomIdent);
    static bool parse_empty_prelude(StringView input, StringView encoding);
    static Optional<Declaration> parse_a_declaration(StringView input, StringView encoding);
    static Optional<Declaration> parse_a_declaration(StringView input, StringView encoding, Vector<RuleContext> const& rule_context);
    static OwnPtr<BooleanExpression> parse_a_supports_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(Vector<ComponentValue>&&)> parse_test);
    static OwnPtr<BooleanExpression> parse_an_if_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(Vector<ComponentValue>&&)> parse_test);
    static OwnPtr<BooleanExpression> parse_a_container_condition(StringView input, StringView encoding);
    static OwnPtr<BooleanExpression> parse_a_media_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test);
    static OwnPtr<BooleanExpression> parse_a_media_test(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test);
    static Optional<MediaQuerySyntax> parse_a_media_query(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test);
    static Vector<MediaQuerySyntax> parse_a_media_query_list(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test);
    static Optional<PageSelectorList> parse_a_page_selector_list(StringView input, StringView encoding);
    static Optional<Vector<Percentage>> parse_a_keyframe_selector_list(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_keyframes_name(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_custom_property_name(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_custom_ident(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_dashed_ident(StringView input, StringView encoding);
    static Optional<Gfx::UnicodeRange> parse_a_unicode_range(StringView input, StringView encoding);
    static Optional<Vector<Gfx::UnicodeRange>> parse_a_unicode_range_list(StringView input, StringView encoding);
    static Optional<URL> parse_a_url_function(StringView input, StringView encoding);
    static Optional<FontSource> parse_a_font_source(StringView input, StringView encoding);
    static Optional<FontLanguageOverride> parse_a_font_language_override(StringView input, StringView encoding);
    static Optional<FlyString> parse_an_opentype_tag(StringView input, StringView encoding);
    static Optional<OpenTypeSettings> parse_font_feature_settings(StringView input, StringView encoding);
    static Optional<OpenTypeSettings> parse_font_variation_settings(StringView input, StringView encoding);
    static Optional<FontStyle> parse_a_font_style(StringView input, StringView encoding);
    static Optional<Vector<FontVariantAlternatesValue>> parse_a_font_variant_alternates(StringView input, StringView encoding);
    static Optional<FontVariant> parse_a_font_variant(StringView input, StringView encoding);
    static Optional<Vector<FontVariantEastAsianValue>> parse_a_font_variant_east_asian(StringView input, StringView encoding);
    static Optional<Vector<FontVariantNumericValue>> parse_a_font_variant_numeric(StringView input, StringView encoding);
    static Optional<Vector<FontVariantLigaturesValue>> parse_a_font_variant_ligatures(StringView input, StringView encoding);
    static Optional<Vector<FontFamilyValue>> parse_font_family_value(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_layer_name(StringView input, StringView encoding, AllowBlankLayerName);
    static Optional<Vector<FlyString>> parse_a_layer_name_list(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_counter_style_name(StringView input, StringView encoding);
    static Optional<CounterStyle> parse_a_counter_style(StringView input, StringView encoding);
    static Optional<FFI::CssNonnegativeIntegerSymbolPairOrder> parse_a_nonnegative_integer_symbol_pair(StringView input, StringView encoding);
    static Optional<FFI::CssCounterStyleNegativeSymbolCount> parse_counter_style_negative(StringView input, StringView encoding);
    static Optional<FFI::CssCounterStyleSystemKind> parse_counter_style_system(StringView input, StringView encoding);
    static Optional<size_t> parse_counter_style_symbols(StringView input, StringView encoding);
    static Optional<FamilyName> parse_a_family_name(StringView input, StringView encoding);
    static Optional<NamespaceRulePrelude> parse_a_namespace_rule_prelude(StringView input, StringView encoding);
    static Optional<Vector<FlyString>> parse_font_feature_values_family_name_list(StringView input, StringView encoding);
    static Optional<Vector<ContainerRulePreludeCondition>> parse_container_rule_prelude(StringView input, StringView encoding);
    static Optional<Rule> parse_a_rule(StringView input, StringView encoding);
    static Vector<RuleOrListOfDeclarations> parse_a_blocks_contents(StringView input, StringView encoding);
    static Vector<RuleOrListOfDeclarations> parse_a_blocks_contents(StringView input, StringView encoding, Vector<RuleContext> const& rule_context);
    static Vector<Rule> parse_a_stylesheets_contents(StringView input, StringView encoding);

private:
    using BooleanExpressionEventCallback = void (*)(void*, FFI::CssBooleanExpressionEventKind);
    using MediaQueryCallback = void (*)(void*, FFI::CssMediaQuery const*);
    using MediaFeatureCallback = void (*)(void*, FFI::CssMediaFeature const*);
    using MediaFeatureValueCallback = void (*)(void*, FFI::CssMediaFeatureValue const*);
    using ComponentValueCallback = void (*)(void*, FFI::CssComponentValue const*);
    using BooleanExpressionTestParser = AK::Function<OwnPtr<BooleanExpression>(Optional<MediaFeatureTest>&&, Vector<ComponentValue>&&)>;
    using RustBooleanExpressionParser = AK::Function<void(u8 const*, size_t, void*, BooleanExpressionEventCallback, MediaFeatureCallback, MediaFeatureValueCallback, ComponentValueCallback)>;

    static OwnPtr<BooleanExpression> parse_a_boolean_expression(StringView input, StringView encoding, MatchResult result_for_general_enclosed, BooleanExpressionTestParser parse_test, RustBooleanExpressionParser rust_parse_boolean_expression);
};

}
