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
#include <AK/Vector.h>
#include <LibWeb/CSS/BooleanExpression.h>
#include <LibWeb/CSS/MediaQuery.h>
#include <LibWeb/CSS/PageSelector.h>
#include <LibWeb/CSS/Parser/ComponentValue.h>
#include <LibWeb/CSS/Parser/RuleContext.h>
#include <LibWeb/CSS/Parser/SyntaxParsing.h>
#include <LibWeb/CSS/Parser/TokenStream.h>
#include <LibWeb/CSS/Parser/Types.h>
#include <LibWeb/CSS/Percentage.h>
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

    enum class AllowBlankLayerName : u8 {
        No,
        Yes,
    };

    static Optional<ComponentValue> parse_a_component_value(StringView input, StringView encoding);
    static Vector<ComponentValue> parse_a_list_of_component_values(StringView input, StringView encoding);
    static Vector<Vector<ComponentValue>> parse_a_comma_separated_list_of_component_values(StringView input, StringView encoding);
    static FFI::CssValueTypeSyntaxKind parse_a_value_type(u8 value_type_id, TokenStream<ComponentValue>&);
    static OwnPtr<SyntaxNode> parse_as_syntax(StringView input, StringView encoding, LimitSingleComponentIdentToCustomIdent);
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
    static Optional<FlyString> parse_a_layer_name(StringView input, StringView encoding, AllowBlankLayerName);
    static Optional<Vector<FlyString>> parse_a_layer_name_list(StringView input, StringView encoding);
    static Optional<FlyString> parse_a_counter_style_name(StringView input, StringView encoding);
    static Optional<NamespaceRulePrelude> parse_a_namespace_rule_prelude(StringView input, StringView encoding);
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
