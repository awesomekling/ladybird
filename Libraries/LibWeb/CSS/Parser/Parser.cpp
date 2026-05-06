/*
 * Copyright (c) 2018-2024, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2020-2021, the SerenityOS developers.
 * Copyright (c) 2021-2026, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2022, MacDue <macdue@dueutil.tech>
 * Copyright (c) 2024, Shannon Booth <shannon@serenityos.org>
 * Copyright (c) 2024, Tommy van der Vorst <tommy@pixelspark.nl>
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>
 * Copyright (c) 2024, Glenn Skrzypczak <glenn.skrzypczak@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Debug.h>
#include <AK/StringBuilder.h>
#include <LibGfx/ImmutableBitmap.h>
#include <LibURL/Parser.h>
#include <LibWeb/CSS/CSSFontFeatureValuesRule.h>
#include <LibWeb/CSS/CSSFunctionDeclarations.h>
#include <LibWeb/CSS/CSSMarginRule.h>
#include <LibWeb/CSS/CSSStyleDeclaration.h>
#include <LibWeb/CSS/CSSStyleProperties.h>
#include <LibWeb/CSS/CSSStyleSheet.h>
#include <LibWeb/CSS/ContainerQuery.h>
#include <LibWeb/CSS/FontFace.h>
#include <LibWeb/CSS/MediaList.h>
#include <LibWeb/CSS/Parser/ArbitrarySubstitutionFunctions.h>
#include <LibWeb/CSS/Parser/ErrorReporter.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/PropertyName.h>
#include <LibWeb/CSS/PropertyNameAndID.h>
#include <LibWeb/CSS/Serialize.h>
#include <LibWeb/CSS/Sizing.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/Dump.h>
#include <LibWeb/HTML/HTMLImageElement.h>

static void log_parse_error(SourceLocation const& location = SourceLocation::current())
{
    dbgln_if(CSS_PARSER_DEBUG, "Parse error (CSS) {}", location);
}

namespace Web::CSS::Parser {

ParsingParams::ParsingParams(ParsingMode mode)
    : mode(mode)
{
}

ParsingParams::ParsingParams(ValueParsingContext value_context)
    : value_context(Vector { move(value_context) })
{
}

ParsingParams::ParsingParams(JS::Realm& realm, ParsingMode mode)
    : realm(realm)
    , mode(mode)
{
}

ParsingParams::ParsingParams(JS::Realm& realm, IsUAStyleSheet is_ua_style_sheet)
    : realm(realm)
    , is_ua_style_sheet(is_ua_style_sheet)
{
}

ParsingParams::ParsingParams(DOM::Document const& document, ParsingMode mode)
    : realm(const_cast<JS::Realm&>(document.realm()))
    , document(&document)
    , mode(mode)
{
}

Parser Parser::create(ParsingParams const& context, StringView input, StringView encoding)
{
    return Parser {
        context,
        String::from_utf8_without_validation(input.bytes()),
        String::from_utf8_without_validation(encoding.bytes())
    };
}

Parser::Parser(ParsingParams const& context, String input, String encoding)
    : m_document(context.document)
    , m_realm(context.realm)
    , m_parsing_mode(context.mode)
    , m_is_ua_style_sheet(context.is_ua_style_sheet)
    , m_input(move(input))
    , m_encoding(move(encoding))
    , m_value_context(move(context.value_context))
    , m_rule_context(move(context.rule_context))
    , m_declared_namespaces(move(context.declared_namespaces))
{
}

GC::RootVector<GC::Ref<CSSRule>> Parser::convert_rules(Vector<Rule> const& raw_rules)
{
    bool import_rules_valid = true;
    bool namespace_rules_valid = true;

    // Interpret all of the resulting top-level qualified rules as style rules, defined below.
    GC::RootVector<GC::Ref<CSSRule>> rules(realm().heap());
    for (auto const& raw_rule : raw_rules) {
        auto rule = convert_to_rule<CSSNestedDeclarations>(raw_rule, Nested::No);
        // If any style rule is invalid, or any at-rule is not recognized or is invalid according to its grammar or context, it’s a parse error.
        // Discard that rule.
        if (!rule) {
            log_parse_error();
            continue;
        }

        // "Any @import rules must precede all other valid at-rules and style rules in a style sheet
        // (ignoring @charset and @layer statement rules) and must not have any other valid at-rules
        // or style rules between it and previous @import rules, or else the @import rule is invalid."
        // https://drafts.csswg.org/css-cascade-5/#at-import
        //
        // "Any @namespace rules must follow all @charset and @import rules and precede all other
        // non-ignored at-rules and style rules in a style sheet.
        // ...
        // A syntactically invalid @namespace rule (whether malformed or misplaced) must be ignored."
        // https://drafts.csswg.org/css-namespaces/#syntax
        switch (rule->type()) {
        case CSSRule::Type::LayerStatement:
            break;
        case CSSRule::Type::Import:
            if (!import_rules_valid)
                continue;
            break;
        case CSSRule::Type::Namespace:
            import_rules_valid = false;

            if (!namespace_rules_valid)
                continue;

            m_declared_namespaces.set(as<CSSNamespaceRule>(*rule).prefix());
            break;
        default:
            import_rules_valid = false;
            namespace_rules_valid = false;
            break;
        }

        rules.append(*rule);
    }

    return rules;
}

GC::RootVector<GC::Ref<CSSRule>> Parser::parse_as_stylesheet_contents()
{
    return convert_rules(RustComponentValueParser::parse_a_stylesheets_contents(m_input, m_encoding));
}

// https://drafts.csswg.org/css-syntax/#parse-a-css-stylesheet
GC::Ref<CSS::CSSStyleSheet> Parser::parse_as_css_stylesheet(Optional<::URL::URL> location, GC::Ptr<MediaList> media_list)
{
    // To parse a CSS stylesheet, first parse a stylesheet.
    auto rules = RustComponentValueParser::parse_a_stylesheets_contents(m_input, m_encoding);

    auto rule_list = CSSRuleList::create(realm(), convert_rules(rules));
    if (!media_list)
        media_list = MediaList::create(realm(), {});
    return CSSStyleSheet::create(realm(), rule_list, *media_list, move(location));
}

RefPtr<Supports> Parser::parse_as_supports()
{
    return parse_a_supports_from_string(m_input, m_encoding);
}

RefPtr<Supports> Parser::parse_a_supports_from_string(StringView input, StringView encoding)
{
    m_rule_context.append(RuleContext::SupportsCondition);
    auto maybe_condition = RustComponentValueParser::parse_a_supports_condition(input, encoding, [this](Vector<ComponentValue>&& component_values) -> OwnPtr<BooleanExpression> {
        TokenStream<ComponentValue> token_stream { component_values };
        return parse_supports_feature(token_stream);
    });
    m_rule_context.take_last();
    if (maybe_condition)
        return Supports::create(maybe_condition.release_nonnull());

    return {};
}

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

String Parser::serialize_component_values_for_reparsing(ReadonlySpan<ComponentValue const> component_values)
{
    StringBuilder builder;
    for (auto const& component_value : component_values)
        serialize_component_value_for_reparsing(builder, component_value);
    return builder.to_string_without_validation();
}

OwnPtr<BooleanExpression> Parser::materialize_rust_supports_condition(Vector<ComponentValue> const& component_values)
{
    auto serialized_supports_condition = serialize_component_values_for_reparsing(component_values);

    m_rule_context.append(RuleContext::SupportsCondition);
    auto maybe_condition = RustComponentValueParser::parse_a_supports_condition(serialized_supports_condition.bytes_as_string_view(), "utf-8"sv, [this](Vector<ComponentValue>&& component_values) -> OwnPtr<BooleanExpression> {
        TokenStream<ComponentValue> token_stream { component_values };
        return parse_supports_feature(token_stream);
    });
    m_rule_context.take_last();

    return maybe_condition;
}

// https://drafts.csswg.org/css-conditional-5/#typedef-supports-feature
OwnPtr<BooleanExpression> Parser::parse_supports_feature(TokenStream<ComponentValue>& tokens)
{
    // <supports-feature> = <supports-selector-fn> | <supports-font-tech-fn>
    //                    | <supports-font-format-fn> | <supports-env-fn>
    //                    | <supports-decl>
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    auto feature_start = tokens.current_index();
    while (tokens.has_next_token())
        tokens.discard_a_token();

    auto serialized_feature = serialize_component_values_for_reparsing(tokens.tokens_since(feature_start));
    auto feature = RustComponentValueParser::parse_a_supports_feature(serialized_feature.bytes_as_string_view(), "utf-8"sv);
    if (!feature.has_value())
        return {};

    auto component_values = Vector<ComponentValue> { tokens.tokens_since(feature_start) };
    TokenStream<ComponentValue> feature_tokens { component_values };
    feature_tokens.discard_whitespace();
    auto const& first_token = feature_tokens.consume_a_token();

    switch (feature->kind) {
    case FFI::CssSupportsFeatureKind::Declaration: {
        VERIFY(first_token.is_block() && first_token.block().is_paren());
        TokenStream block_tokens { first_token.block().value };
        if (auto declaration = parse_supports_declaration(block_tokens)) {
            transaction.commit();
            return BooleanExpressionInParens::create(declaration.release_nonnull<BooleanExpression>());
        }
        return {};
    }
    case FFI::CssSupportsFeatureKind::Selector: {
        VERIFY(first_token.is_function("selector"sv));
        // FIXME: Parsing and then converting back to a string is weird.
        StringBuilder builder;
        for (auto const& item : first_token.function().value)
            builder.append(item.to_string());
        transaction.commit();
        TokenStream selector_tokens { first_token.function().value };
        auto maybe_selector = parse_complex_selector(selector_tokens, SelectorType::Standalone);
        // A CSS processor is considered to support a CSS selector if it accepts that all aspects of that selector,
        // recursively, (rather than considering any of its syntax to be unknown or invalid) and that selector doesn’t
        // contain unknown -webkit- pseudo-elements.
        // https://drafts.csswg.org/css-conditional-4/#dfn-support-selector
        bool matches = !maybe_selector.is_error() && !maybe_selector.value()->contains_unknown_webkit_pseudo_element();
        return Supports::Selector::create(builder.to_string_without_validation(), matches);
    }
    case FFI::CssSupportsFeatureKind::FontTech: {
        VERIFY(feature->name.has_value());
        transaction.commit();
        auto tech_name = feature->name.release_value();
        bool matches = font_tech_is_supported(tech_name);
        return Supports::FontTech::create(move(tech_name), matches);
    }
    case FFI::CssSupportsFeatureKind::FontFormat: {
        VERIFY(feature->name.has_value());
        transaction.commit();
        auto format_name = feature->name.release_value();
        bool matches = font_format_is_supported(format_name);
        return Supports::FontFormat::create(move(format_name), matches);
    }
    case FFI::CssSupportsFeatureKind::Env: {
        VERIFY(feature->name.has_value());
        transaction.commit();
        auto variable_name = feature->name.release_value();
        // https://drafts.csswg.org/css-conditional-5/#support-definition-env
        // A CSS processor is considered to support an environment variable if the <ident> is a supported environment
        // variable.
        bool matches = environment_variable_from_string(variable_name).has_value();
        return Supports::Env::create(move(variable_name), matches);
    }
    }

    VERIFY_NOT_REACHED();
}

// https://drafts.csswg.org/css-conditional-5/#typedef-supports-decl
OwnPtr<Supports::Declaration> Parser::parse_supports_declaration(TokenStream<ComponentValue>& tokens)
{
    // `<supports-decl> = ( <declaration> )`
    // NB: Here, we only care about the <declaration> part.
    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    auto declaration_start = tokens.current_index();
    while (tokens.has_next_token()) {
        if (tokens.next_token().is(Token::Type::Semicolon))
            return {};
        tokens.discard_a_token();
    }

    auto serialized_declaration = serialize_component_values_for_reparsing(tokens.tokens_since(declaration_start));
    auto declaration = RustComponentValueParser::parse_a_declaration(serialized_declaration.bytes_as_string_view(), "utf-8"sv, m_rule_context);
    if (!declaration.has_value())
        return {};

    transaction.commit();
    return Supports::Declaration::create(move(serialized_declaration), convert_to_style_property(declaration.value()).has_value());
}

Vector<ComponentValue> Parser::consume_a_list_of_component_values(TokenStream<ComponentValue>& input, Optional<Token::Type> stop_token)
{
    // To consume a list of component values from a token stream input, given an optional token stop token
    // and an optional boolean nested (default false):

    // Let values be an empty list of component values.
    Vector<ComponentValue> values;

    // Process input:
    for (;;) {
        auto& token = input.next_token();

        // <eof-token>
        // stop token (if passed)
        if (token.is(Token::Type::EndOfFile) || (stop_token.has_value() && token.is(*stop_token))) {
            // Return values.
            return values;
        }

        // <}-token>
        if (token.is(Token::Type::CloseCurly)) {
            // If nested is true, return values.
            // Otherwise, this is a parse error. Consume a token from input and append the result to values.
            log_parse_error();
            values.append(input.consume_a_token());
            continue;
        }

        // anything else
        {
            // Consume a component value from input, and append the result to values.
            values.append(input.consume_a_token());
        }
    }
}

CSSRule* Parser::parse_as_css_rule()
{
    if (auto maybe_rule = RustComponentValueParser::parse_a_rule(m_input, m_encoding); maybe_rule.has_value())
        return convert_to_rule<CSSNestedDeclarations>(maybe_rule.value(), Nested::No);
    return {};
}

Optional<Rule> Parser::parse_as_rule()
{
    return RustComponentValueParser::parse_a_rule(m_input, m_encoding);
}

Optional<Declaration> Parser::parse_as_declaration()
{
    return RustComponentValueParser::parse_a_declaration(m_input, m_encoding);
}

Vector<RuleOrListOfDeclarations> Parser::parse_as_blocks_contents()
{
    return RustComponentValueParser::parse_a_blocks_contents(m_input, m_encoding);
}

Vector<Rule> Parser::parse_as_rules()
{
    return RustComponentValueParser::parse_a_stylesheets_contents(m_input, m_encoding);
}

Optional<StyleProperty> Parser::parse_as_supports_condition()
{
    m_rule_context.append(RuleContext::SupportsCondition);
    auto maybe_declaration = RustComponentValueParser::parse_a_declaration(m_input, m_encoding, m_rule_context);
    m_rule_context.take_last();
    if (maybe_declaration.has_value()) {
        if (auto maybe_property_and_name = convert_to_style_property(maybe_declaration.release_value()); maybe_property_and_name.has_value())
            return maybe_property_and_name->property;
    }
    return {};
}

Optional<ComponentValue> Parser::parse_as_component_value()
{
    return RustComponentValueParser::parse_a_component_value(m_input, m_encoding);
}

// https://drafts.csswg.org/css-syntax/#parse-comma-separated-list-of-component-values
Vector<Vector<ComponentValue>> Parser::parse_a_comma_separated_list_of_component_values(TokenStream<ComponentValue>& input)
{
    // To parse a comma-separated list of component values from input:

    // 1. Normalize input, and set input to the result.
    // Note: This is done when initializing the Parser.

    auto remaining_tokens = input.remaining_tokens();

    // AD-HOC: Re-parsing substituted component values through Rust would lose
    // C++-side attr() taint metadata until that metadata is carried over FFI.
    if (!remaining_tokens.first_matching([](auto const& component_value) { return component_value.contains_attr_tainted_value(); }).has_value()) {
        auto serialized_input = serialize_component_values_for_reparsing(remaining_tokens);
        while (input.has_next_token())
            input.discard_a_token();
        return RustComponentValueParser::parse_a_comma_separated_list_of_component_values(serialized_input.bytes_as_string_view(), "utf-8"sv);
    }

    // 2. Let groups be an empty list.
    Vector<Vector<ComponentValue>> groups;

    // 3. While input is not empty:
    bool just_consumed_comma = false;
    while (!input.is_empty()) {

        // 1. Consume a list of component values from input, with <comma-token> as the stop token, and append the result to groups.
        groups.append(consume_a_list_of_component_values(input, Token::Type::Comma));

        // 2. Discard a token from input.
        just_consumed_comma = input.consume_a_token().is(Token::Type::Comma);
    }

    // AD-HOC: Also append an empty group if there was a trailing comma.
    // Some related spec discussion: https://github.com/w3c/csswg-drafts/issues/11254
    if (just_consumed_comma)
        groups.append({});

    // 4. Return groups.
    return groups;
}

// https://drafts.csswg.org/cssom/#parse-a-css-declaration-block
Parser::PropertiesAndCustomProperties Parser::parse_as_property_declaration_block()
{
    auto expand_shorthands = [&](Vector<StyleProperty>& properties) -> Vector<StyleProperty> {
        Vector<StyleProperty> expanded_properties;
        for (auto& property : properties) {
            if (property_is_shorthand(property.property_id)) {
                StyleComputer::for_each_property_expanding_shorthands(property.property_id, *property.value, [&](PropertyID longhand_property_id, StyleValue const& longhand_value) {
                    expanded_properties.append(CSS::StyleProperty {
                        .important = property.important,
                        .property_id = longhand_property_id,
                        .value = longhand_value,
                    });
                });
            } else {
                expanded_properties.append(property);
            }
        }
        return expanded_properties;
    };

    // 1. Let declarations be the returned declarations from invoking parse a block’s contents with string.
    auto declarations_and_at_rules = RustComponentValueParser::parse_a_blocks_contents(m_input, m_encoding, m_rule_context);

    // 2. Let parsed declarations be a new empty list.
    PropertiesAndCustomProperties parsed_declarations;

    // 3. For each item declaration in declarations, follow these substeps:
    for (auto const& rule_or_list : declarations_and_at_rules) {
        if (rule_or_list.has<Rule>())
            continue;

        auto& rule_declarations = rule_or_list.get<Vector<Declaration>>();
        for (auto const& declaration : rule_declarations) {
            // 1. Let parsed declaration be the result of parsing declaration according to the appropriate CSS
            //    specifications, dropping parts that are said to be ignored. If the whole declaration is dropped, let
            //    parsed declaration be null.
            // 2. If parsed declaration is not null, append it to parsed declarations.
            extract_property(declaration, parsed_declarations);
        }
    }
    parsed_declarations.properties = expand_shorthands(parsed_declarations.properties);

    // 4. Return parsed declarations.
    return parsed_declarations;
}

// https://drafts.csswg.org/cssom/#parse-a-css-declaration-block
Vector<Descriptor> Parser::parse_as_descriptor_declaration_block(AtRuleID at_rule_id)
{
    auto context_type = [at_rule_id] {
        switch (at_rule_id) {
        case AtRuleID::FontFace:
            return RuleContext::AtFontFace;
        case AtRuleID::Function:
            return RuleContext::AtFunction;
        case AtRuleID::Page:
            return RuleContext::AtPage;
        case AtRuleID::Property:
            return RuleContext::AtProperty;
        case AtRuleID::CounterStyle:
            // NB: We don't actually have a `CSSDescriptors` for `@counter-style` so this function shouldn't ever be
            //     called with `AtRuleID::CounterStyle`.
            VERIFY_NOT_REACHED();
        }
        VERIFY_NOT_REACHED();
    }();

    // 1. Let declarations be the returned declarations from invoking parse a block’s contents with string.
    m_rule_context.append(context_type);
    auto declarations_and_at_rules = RustComponentValueParser::parse_a_blocks_contents(m_input, m_encoding, m_rule_context);
    m_rule_context.take_last();

    // 2. Let parsed declarations be a new empty list.
    Vector<Descriptor> parsed_declarations;

    // 3. For each item declaration in declarations, follow these substeps:
    for (auto const& rule_or_list : declarations_and_at_rules) {
        if (rule_or_list.has<Rule>())
            continue;

        auto& rule_declarations = rule_or_list.get<Vector<Declaration>>();
        for (auto const& declaration : rule_declarations) {
            // 1. Let parsed declaration be the result of parsing declaration according to the appropriate CSS
            //    specifications, dropping parts that are said to be ignored. If the whole declaration is dropped, let
            //    parsed declaration be null.
            // 2. If parsed declaration is not null, append it to parsed declarations.
            if (auto parsed_declaration = convert_to_descriptor(at_rule_id, declaration); parsed_declaration.has_value())
                parsed_declarations.append(parsed_declaration.release_value());
        }
    }

    // 4. Return parsed declarations.
    return parsed_declarations;
}

bool Parser::is_valid_in_the_current_context(Declaration const& declaration) const
{
    // TODO: Determine if this *particular* declaration is valid here, not just declarations in general.

    // Declarations can't appear at the top level
    if (m_rule_context.is_empty())
        return false;

    switch (m_rule_context.last()) {
    case RuleContext::Unknown:
        // If the context is an unknown type, we don't accept anything.
        return false;

    case RuleContext::Style:
        // Style rules contain property declarations
        return true;

    case RuleContext::Keyframe: {
        // https://drafts.csswg.org/css-animations-1/#keyframes
        // The <declaration-list> inside of <keyframe-block> accepts any CSS property except those defined in this
        // specification, but does accept the animation-timing-function property and interprets it specially
        // NB: animation-composition is defined in CSS Animations Level 2, so it is not excluded by this rule.
        auto property = PropertyNameAndID::from_name(declaration.name);
        if (!property.has_value())
            return true;
        switch (property->id()) {
        case PropertyID::Animation:
        case PropertyID::AnimationDelay:
        case PropertyID::AnimationDirection:
        case PropertyID::AnimationDuration:
        case PropertyID::AnimationFillMode:
        case PropertyID::AnimationIterationCount:
        case PropertyID::AnimationName:
        case PropertyID::AnimationPlayState:
        case PropertyID::AnimationTimeline:
            return false;
        default:
            return true;
        }
    }

    case RuleContext::AtContainer:
    case RuleContext::AtLayer:
    case RuleContext::AtMedia:
    case RuleContext::AtSupports:
        // Grouping rules can contain declarations if they are themselves inside a style or function rule
        return m_rule_context.contains([](auto const& context) { return context == RuleContext::Style || context == RuleContext::AtFunction; });

    case RuleContext::FontFeatureValue:
        // Each feature value block accepts a list of declarations
        return true;

    case RuleContext::AtFunction:
        // @function rules contain descriptor declarations
        return true;

    case RuleContext::AtCounterStyle:
    case RuleContext::AtFontFace:
    case RuleContext::AtFontFeatureValues:
    case RuleContext::AtPage:
    case RuleContext::AtProperty:
    case RuleContext::Margin:
        // These have descriptor declarations
        return true;

    case RuleContext::AtKeyframes:
        // @keyframes can only contain keyframe rules
        return false;

    case RuleContext::SupportsCondition:
        // @supports conditions accept all declarations
        return true;
    }

    VERIFY_NOT_REACHED();
}

bool Parser::is_valid_in_the_current_context(AtRule const& at_rule) const
{
    // All at-rules can appear at the top level, except margin rules
    if (m_rule_context.is_empty())
        return !is_margin_rule_name(at_rule.name);

    // Only grouping rules can be nested within style rules
    if (m_rule_context.contains_slow(RuleContext::Style))
        return first_is_one_of(at_rule.name, "container", "layer", "media", "supports");

    if (m_rule_context.contains_slow(RuleContext::AtFunction)) {
        // https://drafts.csswg.org/css-mixins-1/#function-body
        // The body of a @function rule accepts conditional group rules
        return first_is_one_of(at_rule.name, "container", "media", "supports");
    }

    switch (m_rule_context.last()) {
    case RuleContext::Unknown:
        // If the context is an unknown type, we don't accept anything.
        return false;

    case RuleContext::Style:
        // Already handled above
        VERIFY_NOT_REACHED();

    case RuleContext::AtContainer:
    case RuleContext::AtLayer:
    case RuleContext::AtMedia:
    case RuleContext::AtSupports:
        // Grouping rules can contain anything except @import or @namespace
        return !first_is_one_of(at_rule.name, "import", "namespace");

    case RuleContext::SupportsCondition:
        // @supports cannot check for at-rules
        return false;

    case RuleContext::AtPage:
        // @page rules can contain margin rules
        return is_margin_rule_name(at_rule.name);

    case RuleContext::AtCounterStyle:
    case RuleContext::AtFontFace:
    case RuleContext::FontFeatureValue:
    case RuleContext::AtKeyframes:
    case RuleContext::Keyframe:
    case RuleContext::AtProperty:
    case RuleContext::Margin:
        // These can't contain any at-rules
        return false;
    case RuleContext::AtFontFeatureValues:
        return CSSFontFeatureValuesRule::is_font_feature_value_type_at_keyword(at_rule.name);
    case RuleContext::AtFunction:
        // Already handled above
        VERIFY_NOT_REACHED();
    }

    VERIFY_NOT_REACHED();
}

bool Parser::is_valid_in_the_current_context(QualifiedRule const&) const
{
    // TODO: Different places accept different kinds of qualified rules. How do we tell them apart? Can we?

    // Top level can contain style rules
    if (m_rule_context.is_empty())
        return true;

    switch (m_rule_context.last()) {
    case RuleContext::Unknown:
        // If the context is an unknown type, we don't accept anything.
        return false;

    case RuleContext::Style:
        // Style rules can contain style rules
        return true;

    case RuleContext::AtContainer:
    case RuleContext::AtLayer:
    case RuleContext::AtMedia:
    case RuleContext::AtSupports:
        // Grouping rules can contain style rules
        return true;

    case RuleContext::AtKeyframes:
        // @keyframes can contain keyframe rules
        return true;

    case RuleContext::SupportsCondition:
        // @supports cannot check qualified rules
        return false;

    case RuleContext::AtCounterStyle:
    case RuleContext::AtFontFace:
    case RuleContext::AtFontFeatureValues:
    case RuleContext::FontFeatureValue:
    case RuleContext::AtFunction:
    case RuleContext::AtPage:
    case RuleContext::AtProperty:
    case RuleContext::Keyframe:
    case RuleContext::Margin:
        // These can't contain qualified rules
        return false;
    }

    VERIFY_NOT_REACHED();
}

void Parser::extract_property(Declaration const& declaration, PropertiesAndCustomProperties& dest)
{
    if (auto maybe_property_and_name = convert_to_style_property(declaration); maybe_property_and_name.has_value()) {
        auto property = maybe_property_and_name->property;
        if (property.property_id == PropertyID::Custom) {
            dest.custom_properties.set(maybe_property_and_name->name, property);
        } else {
            dest.properties.append(move(property));
        }
    }
}

GC::Ref<CSSStyleProperties> Parser::convert_to_style_declaration(Vector<Declaration> const& declarations)
{
    PropertiesAndCustomProperties properties;
    PropertiesAndCustomProperties& dest = properties;
    for (auto const& declaration : declarations) {
        extract_property(declaration, dest);
    }
    return CSSStyleProperties::create(realm(), move(properties.properties), move(properties.custom_properties));
}

Optional<StylePropertyAndName> Parser::convert_to_style_property(Declaration const& declaration)
{
    auto property = PropertyNameAndID::from_name(declaration.name);

    if (!property.has_value()) {
        if (has_ignored_vendor_prefix(declaration.name)) {
            return {};
        }
        ErrorReporter::the().report(UnknownPropertyError { .property_name = declaration.name });
        return {};
    }

    auto value_token_stream = TokenStream(declaration.value);
    auto value = parse_css_value(property->id(), value_token_stream, declaration.original_value_text);
    if (value.is_error()) {
        if (value.error() == ParseError::SyntaxError) {
            ErrorReporter::the().report(InvalidPropertyError {
                .property_name = property->name(),
                .value_string = value_token_stream.dump_string(),
                .description = "Failed to parse."_string,
            });
        }
        return {};
    }

    if (property->is_custom_property())
        return StylePropertyAndName {
            StyleProperty { declaration.important, property->id(), value.release_value() },
            property->name()
        };

    return StylePropertyAndName {
        StyleProperty { declaration.important, property->id(), value.release_value() }
    };
}

RefPtr<StyleValue const> Parser::parse_source_size_value(TokenStream<ComponentValue>& tokens)
{
    if (tokens.next_token().is_ident("auto"sv)) {
        tokens.discard_a_token(); // auto
        return KeywordStyleValue::create(Keyword::Auto);
    }

    // https://html.spec.whatwg.org/multipage/images.html#valid-source-size-list
    // "A <source-size-value> that is a <length> must not be negative,
    // and must not use CSS functions other than the math functions."
    if (auto parsed = parse_length_value(tokens, non_negative_range)) {
        // FIXME: It seems odd that we disallow infinite calculated values here rather than clamping as we do for all
        //        other values - is this correct?
        if (parsed->is_calculated()) {
            // https://drafts.csswg.org/css-values-4/#calc-range
            // "the value resulting from a top-level calculation must be
            // clamped to the range allowed in the target context."
            auto raw_length = parsed->as_calculated().resolve_raw_length({});
            if (raw_length.has_value() && !isfinite(*raw_length))
                return {};
        }

        return parsed;
    }

    return {};
}

bool Parser::context_allows_quirky_length() const
{
    if (!in_quirks_mode())
        return false;

    // https://drafts.csswg.org/css-values-4/#deprecated-quirky-length
    // "When CSS is being parsed in quirks mode, <quirky-length> is a type of <length> that is only valid in certain properties:"
    // (NOTE: List skipped for brevity; quirks data is assigned in Properties.json)
    // "It is not valid in properties that include or reference these properties, such as the background shorthand,
    // or inside functional notations such as calc(), except that they must be allowed in rect() in the clip property."

    // So, it must be allowed in the top-level ValueParsingContext, and then not disallowed by any child contexts.

    Optional<PropertyID> top_level_property;
    if (!m_value_context.is_empty()) {
        top_level_property = m_value_context.first().visit(
            [](PropertyID const& property_id) -> Optional<PropertyID> { return property_id; },
            [](auto const&) -> Optional<PropertyID> { return OptionalNone {}; });
    }

    bool unitless_length_allowed = top_level_property.has_value() && property_has_quirk(top_level_property.value(), Quirk::UnitlessLength);
    for (auto i = 1u; i < m_value_context.size() && unitless_length_allowed; i++) {
        unitless_length_allowed = m_value_context[i].visit(
            [](PropertyID const& property_id) { return property_has_quirk(property_id, Quirk::UnitlessLength); },
            [top_level_property](FunctionContext const& function_context) {
                return function_context.name == "rect"sv && top_level_property == PropertyID::Clip;
            },
            [](auto const&) { return false; });
    }

    return unitless_length_allowed;
}

bool Parser::context_allows_tree_counting_functions() const
{
    for (auto context : m_value_context) {
        if (context.has<DescriptorContext>())
            return false;

        if (auto const* special_context = context.get_pointer<SpecialContext>(); special_context && first_is_one_of(*special_context, SpecialContext::CanvasContextGenericValue, SpecialContext::DOMMatrixInitString, SpecialContext::MediaCondition))
            return false;

        // TODO: Handle other contexts where tree counting functions are not allowed
    }

    return true;
}

bool Parser::context_allows_random_functions() const
{
    if (auto const* special_context = m_value_context.first().get_pointer<SpecialContext>(); special_context && first_is_one_of(*special_context, SpecialContext::CanvasContextGenericValue, SpecialContext::OnScreenCanvasContextFontValue))
        return false;

    // For now we only allow random functions within property contexts, see https://drafts.csswg.org/css-values-5/#issue-cd071f29
    // FIXME: Should this instead check that the top-level context is a property context (our current configuration
    //        allows these within DOMMatrixInitString for example)
    return m_value_context.contains([](ValueParsingContext context) { return context.has<PropertyID>(); });
}

FlyString Parser::random_value_sharing_auto_name() const
{
    auto top_level_property_context_index = m_value_context.find_first_index_if([](ValueParsingContext const& context) { return context.has<PropertyID>(); });

    auto property_name = string_from_property_id(m_value_context[top_level_property_context_index.value()].get<PropertyID>());

    return MUST(String::formatted("{} {}", property_name, m_random_function_index));
}

Vector<ComponentValue> Parser::parse_as_list_of_component_values()
{
    return RustComponentValueParser::parse_a_list_of_component_values(m_input, m_encoding);
}

RefPtr<StyleValue const> Parser::parse_as_css_value(PropertyID property_id)
{
    auto component_values = RustComponentValueParser::parse_a_list_of_component_values(m_input, m_encoding);
    auto tokens = TokenStream(component_values);
    auto parsed_value = parse_css_value(property_id, tokens);
    if (parsed_value.is_error())
        return nullptr;
    return parsed_value.release_value();
}

RefPtr<StyleValue const> Parser::parse_as_descriptor_value(AtRuleID at_rule_id, DescriptorNameAndID const& descriptor_name_and_id)
{
    auto component_values = RustComponentValueParser::parse_a_list_of_component_values(m_input, m_encoding);
    auto tokens = TokenStream(component_values);
    auto parsed_value = parse_descriptor_value(at_rule_id, descriptor_name_and_id, tokens);
    if (parsed_value.is_error())
        return nullptr;
    return parsed_value.release_value();
}

RefPtr<StyleValue const> Parser::parse_as_type(ValueType value_type)
{
    auto component_values = RustComponentValueParser::parse_a_list_of_component_values(m_input, m_encoding);
    TokenStream tokens { component_values };
    return parse_value(value_type, tokens);
}

// https://html.spec.whatwg.org/multipage/images.html#parsing-a-sizes-attribute
NonnullRefPtr<StyleValue const> Parser::parse_as_sizes_attribute(DOM::Element const& element, HTML::HTMLImageElement const* img)
{
    // When asked to parse a sizes attribute from an element element, with an img element or null img:

    // AD-HOC: If element has no sizes attribute, this algorithm always logs a parse error and then returns 100vw.
    //         The attribute is optional, so avoid spamming the debug log with false positives by just returning early.
    if (!element.has_attribute(HTML::AttributeNames::sizes))
        return LengthStyleValue::create(Length(100, LengthUnit::Vw));

    // 1. Let unparsed sizes list be the result of parsing a comma-separated list of component values
    //    from the value of element's sizes attribute (or the empty string, if the attribute is absent).
    auto unparsed_sizes_list = RustComponentValueParser::parse_a_comma_separated_list_of_component_values(m_input, m_encoding);

    // 2. Let size be null.
    RefPtr<StyleValue const> size;

    auto remove_all_consecutive_whitespace_tokens_from_the_end_of = [](auto& tokens) {
        while (!tokens.is_empty() && tokens.last().is_token() && tokens.last().token().is(Token::Type::Whitespace))
            tokens.take_last();
    };

    // 3. For each unparsed size in unparsed sizes list:
    for (auto i = 0u; i < unparsed_sizes_list.size(); i++) {
        auto& unparsed_size = unparsed_sizes_list[i];

        // 1. Remove all consecutive <whitespace-token>s from the end of unparsed size.
        //    If unparsed size is now empty, that is a parse error; continue.
        remove_all_consecutive_whitespace_tokens_from_the_end_of(unparsed_size);
        if (unparsed_size.is_empty()) {
            log_parse_error();
            ErrorReporter::the().report(InvalidValueError {
                .value_type = "sizes attribute"_fly_string,
                .value_string = m_input,
                .description = "Failed in step 3.1; all whitespace"_string,
            });
            continue;
        }

        // 2. If the last component value in unparsed size is a valid non-negative <source-size-value>,
        //    then set size to its value and remove the component value from unparsed size.
        //    Any CSS function other than the math functions is invalid.
        //    Otherwise, there is a parse error; continue.
        auto last_value_stream = TokenStream<ComponentValue>::of_single_token(unparsed_size.last());
        if (auto source_size_value = parse_source_size_value(last_value_stream)) {
            size = source_size_value.release_nonnull();
            unparsed_size.take_last();
        } else {
            log_parse_error();
            ErrorReporter::the().report(InvalidValueError {
                .value_type = "sizes attribute"_fly_string,
                .value_string = m_input,
                .description = "Failed in step 3.2; couldn't parse {} as a <source-size-value>"_string,
            });
            continue;
        }

        // 3. If size is auto, and img is not null, and img is being rendered, and img allows auto-sizes,
        //    then set size to the concrete object size width of img, in CSS pixels.
        // FIXME: "img is being rendered" - we just see if it has a bitmap for now
        if (size->has_auto() && img && img->immutable_bitmap() && img->allows_auto_sizes()) {
            // FIXME: The spec doesn't seem to tell us how to determine the concrete size of an <img>, so use the default sizing algorithm.
            //        Should this use some of the methods from FormattingContext?
            auto concrete_size = run_default_sizing_algorithm(
                img->width(), img->height(),
                { img->natural_width(), img->natural_height(), img->intrinsic_aspect_ratio() },
                // NOTE: https://html.spec.whatwg.org/multipage/rendering.html#img-contain-size
                CSSPixelSize { 300, 150 });
            size = LengthStyleValue::create(Length::make_px(concrete_size.width()));
        }

        // 4. Remove all consecutive <whitespace-token>s from the end of unparsed size.
        //    If unparsed size is now empty:
        remove_all_consecutive_whitespace_tokens_from_the_end_of(unparsed_size);
        if (unparsed_size.is_empty()) {
            // 1. If this was not the last item in unparsed sizes list, that is a parse error.
            if (i != unparsed_sizes_list.size() - 1) {
                log_parse_error();
                ErrorReporter::the().report(InvalidValueError {
                    .value_type = "sizes attribute"_fly_string,
                    .value_string = m_input,
                    .description = MUST(String::formatted("Failed in step 3.4.1; is unparsed size #{}, count {}", i, unparsed_sizes_list.size())),
                });
            }

            // 2. If size is not auto, then return size. Otherwise, continue.
            if (!size->has_auto())
                return size.release_nonnull();
            continue;
        }

        // 5. Parse the remaining component values in unparsed size as a <media-condition>.
        //    If it does not parse correctly, or it does parse correctly but the <media-condition> evaluates to false, continue.
        auto serialized_media_condition = serialize_component_values_for_reparsing(unparsed_size);
        auto media_condition = RustComponentValueParser::parse_a_media_condition(serialized_media_condition.bytes_as_string_view(), "utf-8"sv, [this](RustComponentValueParser::MediaFeatureTest&& media_feature_test) -> OwnPtr<BooleanExpression> {
            return materialize_rust_media_feature_test(move(media_feature_test));
        });
        if (!media_condition)
            continue;

        // https://drafts.csswg.org/mediaqueries-5/#evaluating
        // "If the result of any of the above productions is used in any
        // context that expects a two-valued boolean, 'unknown' must be
        // converted to 'false'."
        if (m_document && !media_condition->evaluate_to_boolean(m_document))
            continue;

        // 5. If size is not auto, then return size. Otherwise, continue.
        if (!size->has_auto())
            return size.release_nonnull();
    }

    // 4. Return 100vw.
    return LengthStyleValue::create(Length(100, LengthUnit::Vw));
}

Parser::ParseErrorOr<void> Parser::collect_arbitrary_substitution_function_presence(Vector<ComponentValue> const& component_values, SubstitutionFunctionsPresence& presence)
{
    for (auto const& component_value : component_values) {
        if (collect_arbitrary_substitution_function_presence(component_value, presence).is_error())
            return ParseError::SyntaxError;
    }

    return {};
}

Parser::ParseErrorOr<void> Parser::collect_arbitrary_substitution_function_presence(ComponentValue const& component_value, SubstitutionFunctionsPresence& presence)
{
    if (component_value.is_function()) {
        auto const& function = component_value.function();
        if (auto arbitrary_substitution_function = to_arbitrary_substitution_function(function.name); arbitrary_substitution_function.has_value()) {
            if (!parse_according_to_argument_grammar(arbitrary_substitution_function.value(), function.value).has_value())
                return ParseError::SyntaxError;

            switch (arbitrary_substitution_function.value()) {
            case ArbitrarySubstitutionFunction::Attr:
                presence.attr = true;
                break;
            case ArbitrarySubstitutionFunction::Env:
                presence.env = true;
                break;
            case ArbitrarySubstitutionFunction::If:
                presence.if_ = true;
                break;
            case ArbitrarySubstitutionFunction::Inherit:
                presence.inherit = true;
                break;
            case ArbitrarySubstitutionFunction::Var:
                presence.var = true;
                break;
            }
        }

        return collect_arbitrary_substitution_function_presence(function.value, presence);
    }

    if (component_value.is_block())
        return collect_arbitrary_substitution_function_presence(component_value.block().value, presence);

    return {};
}

bool Parser::has_ignored_vendor_prefix(StringView string)
{
    if (!string.starts_with('-'))
        return false;
    if (string.starts_with("--"sv))
        return false;
    if (string.starts_with("-libweb-"sv))
        return false;
    if (string.count('-') == 1)
        return false;
    return true;
}

DOM::Document const* Parser::document() const
{
    return m_document;
}

HTML::Window const* Parser::window() const
{
    if (!m_document)
        return nullptr;
    return m_document->window();
}

JS::Realm& Parser::realm() const
{
    VERIFY(m_realm);
    return *m_realm;
}

bool Parser::in_quirks_mode() const
{
    return m_document ? m_document->in_quirks_mode() : false;
}

bool Parser::is_parsing_svg_presentation_attribute() const
{
    return m_parsing_mode == ParsingMode::SVGPresentationAttribute;
}

}
