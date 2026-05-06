/*
 * Copyright (c) 2018-2022, Andreas Kling <andreas@ladybird.org>
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

#include <LibGC/HeapVector.h>
#include <LibWeb/CSS/CSSContainerRule.h>
#include <LibWeb/CSS/CSSCounterStyleRule.h>
#include <LibWeb/CSS/CSSFontFaceRule.h>
#include <LibWeb/CSS/CSSFontFeatureValuesRule.h>
#include <LibWeb/CSS/CSSFunctionDeclarations.h>
#include <LibWeb/CSS/CSSFunctionRule.h>
#include <LibWeb/CSS/CSSImportRule.h>
#include <LibWeb/CSS/CSSKeyframeRule.h>
#include <LibWeb/CSS/CSSKeyframesRule.h>
#include <LibWeb/CSS/CSSLayerBlockRule.h>
#include <LibWeb/CSS/CSSLayerStatementRule.h>
#include <LibWeb/CSS/CSSMarginRule.h>
#include <LibWeb/CSS/CSSMediaRule.h>
#include <LibWeb/CSS/CSSNamespaceRule.h>
#include <LibWeb/CSS/CSSNestedDeclarations.h>
#include <LibWeb/CSS/CSSPageRule.h>
#include <LibWeb/CSS/CSSPropertyRule.h>
#include <LibWeb/CSS/CSSStyleProperties.h>
#include <LibWeb/CSS/CSSStyleRule.h>
#include <LibWeb/CSS/CSSSupportsRule.h>
#include <LibWeb/CSS/ContainerQuery.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/FontFace.h>
#include <LibWeb/CSS/Parser/ErrorReporter.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/Parser/Syntax.h>
#include <LibWeb/CSS/Parser/SyntaxParsing.h>
#include <LibWeb/CSS/PropertyName.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/OpenTypeTaggedStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>

namespace Web::CSS::Parser {

// A helper that ensures only the last instance of each descriptor is included, while also handling shorthands.
class DescriptorList {
public:
    DescriptorList(AtRuleID at_rule)
        : m_at_rule(at_rule)
    {
    }

    void append(Descriptor&& descriptor)
    {
        if (is_shorthand(m_at_rule, descriptor.descriptor_name_and_id)) {
            for_each_expanded_longhand(m_at_rule, descriptor.descriptor_name_and_id, descriptor.value, [this](auto longhand_id, auto longhand_value) {
                append_internal(Descriptor { longhand_id, longhand_value.release_nonnull() });
            });
            return;
        }

        append_internal(move(descriptor));
    }

    Vector<Descriptor> release_descriptors()
    {
        return move(m_descriptors);
    }

private:
    void append_internal(Descriptor&& descriptor)
    {
        if (m_seen_descriptor_ids.contains(descriptor.descriptor_name_and_id)) {
            m_descriptors.remove_first_matching([&descriptor](Descriptor const& existing) {
                return existing.descriptor_name_and_id == descriptor.descriptor_name_and_id;
            });
        } else {
            m_seen_descriptor_ids.set(descriptor.descriptor_name_and_id);
        }
        m_descriptors.append(move(descriptor));
    }

    AtRuleID m_at_rule;
    Vector<Descriptor> m_descriptors;
    HashTable<DescriptorNameAndID> m_seen_descriptor_ids;
};

template<typename NestedDeclarationsRule>
GC::Ptr<CSSRule> Parser::convert_to_rule(Rule const& rule, Nested nested)
{
    return rule.visit(
        [this, nested](AtRule const& at_rule) -> GC::Ptr<CSSRule> {
            // https://compat.spec.whatwg.org/#css-at-rules
            // @-webkit-keyframes must be supported as an alias of @keyframes.
            if (at_rule.name.equals_ignoring_ascii_case("keyframes"sv) || at_rule.name.equals_ignoring_ascii_case("-webkit-keyframes"sv))
                return convert_to_keyframes_rule(at_rule);

            if (has_ignored_vendor_prefix(at_rule.name))
                return {};

            if (at_rule.name.equals_ignoring_ascii_case("container"sv))
                return convert_to_container_rule<NestedDeclarationsRule>(at_rule, nested);

            if (at_rule.name.equals_ignoring_ascii_case("counter-style"sv))
                return convert_to_counter_style_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("font-face"sv))
                return convert_to_font_face_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("font-feature-values"sv))
                return convert_to_font_feature_values_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("function"sv))
                return convert_to_function_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("import"sv))
                return convert_to_import_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("layer"sv))
                return convert_to_layer_rule<NestedDeclarationsRule>(at_rule, nested);

            if (is_margin_rule_name(at_rule.name))
                return convert_to_margin_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("media"sv))
                return convert_to_media_rule<NestedDeclarationsRule>(at_rule, nested);

            if (at_rule.name.equals_ignoring_ascii_case("namespace"sv))
                return convert_to_namespace_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("page"sv))
                return convert_to_page_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("property"sv))
                return convert_to_property_rule(at_rule);

            if (at_rule.name.equals_ignoring_ascii_case("supports"sv))
                return convert_to_supports_rule<NestedDeclarationsRule>(at_rule, nested);

            // FIXME: More at rules!
            ErrorReporter::the().report(UnknownRuleError { .rule_name = MUST(String::formatted("@{}", at_rule.name)) });
            return {};
        },
        [this, nested](QualifiedRule const& qualified_rule) -> GC::Ptr<CSSRule> {
            return convert_to_style_rule(qualified_rule, nested);
        });
}

GC::Ptr<CSSStyleRule> Parser::convert_to_style_rule(QualifiedRule const& qualified_rule, Nested nested)
{
    TokenStream prelude_stream { qualified_rule.prelude };

    auto maybe_selectors = parse_a_selector_list(prelude_stream,
        nested == Nested::Yes ? SelectorType::Relative : SelectorType::Standalone);

    if (maybe_selectors.is_error()) {
        if (maybe_selectors.error() == ParseError::SyntaxError) {
            ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                .rule_name = "style"_fly_string,
                .prelude = prelude_stream.dump_string(),
                .description = "Selectors invalid."_string,
            });
        }
        return {};
    }

    if (maybe_selectors.value().is_empty()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "style"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Empty selector."_string,
        });
        return {};
    }

    SelectorList selectors = maybe_selectors.release_value();
    if (nested == Nested::Yes)
        selectors = adapt_nested_relative_selector_list(selectors);

    auto declaration = convert_to_style_declaration(qualified_rule.declarations);

    GC::RootVector<GC::Ref<CSSRule>> child_rules { realm().heap() };
    for (auto& child : qualified_rule.child_rules) {
        child.visit(
            [&](Rule const& rule) {
                // "In addition to nested style rules, this specification allows nested group rules inside of style rules:
                // any at-rule whose body contains style rules can be nested inside of a style rule as well."
                // https://drafts.csswg.org/css-nesting-1/#nested-group-rules
                if (auto converted_rule = convert_to_rule<CSSNestedDeclarations>(rule, Nested::Yes)) {
                    if (is<CSSGroupingRule>(*converted_rule)) {
                        child_rules.append(*converted_rule);
                    } else {
                        ErrorReporter::the().report(InvalidRuleLocationError {
                            .outer_rule_name = "style"_fly_string,
                            .inner_rule_name = MUST(FlyString::from_utf8(converted_rule->class_name())),
                        });
                    }
                }
            },
            [&](Vector<Declaration> const& declarations) {
                child_rules.append(CSSNestedDeclarations::create(realm(), *this, declarations));
            });
    }
    auto nested_rules = CSSRuleList::create(realm(), child_rules);
    return CSSStyleRule::create(realm(), move(selectors), *declaration, *nested_rules);
}

GC::Ptr<CSSImportRule> Parser::convert_to_import_rule(AtRule const& rule)
{
    // https://drafts.csswg.org/css-cascade-5/#at-import
    // @import [ <url> | <string> ]
    //         [ layer | layer(<layer-name>) ]?
    //         <import-conditions> ;
    //
    // <import-conditions> = [ supports( [ <supports-condition> | <declaration> ] ) ]?
    //                      <media-query-list>?
    TokenStream tokens { rule.prelude };

    if (rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@import"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Must be a statement, not a block."_string,
        });
        return {};
    }

    if (rule.prelude.is_empty()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@import"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Empty prelude."_string,
        });
        return {};
    }

    tokens.discard_whitespace();

    Optional<URL> url;
    if (tokens.has_next_token()) {
        auto const& component_value = tokens.next_token();
        auto serialized_import_url = serialize_component_values_for_reparsing({ &component_value, 1 });
        url = RustComponentValueParser::parse_an_import_url(serialized_import_url.bytes_as_string_view(), "utf-8"sv);
        if (url.has_value())
            tokens.discard_a_token();
    }

    if (!url.has_value()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@import"_fly_string,
            .prelude = tokens.dump_string(),
            .description = MUST(String::formatted("Unable to parse `{}` as URL.", tokens.next_token().to_debug_string())),
        });
        return {};
    }

    tokens.discard_whitespace();
    Optional<FlyString> layer;
    // [ layer | layer(<layer-name>) ]?
    if (tokens.has_next_token()) {
        auto const& component_value = tokens.next_token();
        auto serialized_import_layer = serialize_component_values_for_reparsing({ &component_value, 1 });
        auto name = RustComponentValueParser::parse_an_import_layer(serialized_import_layer.bytes_as_string_view(), "utf-8"sv);
        if (name.has_value()) {
            tokens.discard_a_token();
            layer = name.release_value();
        } else if (component_value.is_function("layer"sv)) {
            ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                .rule_name = "@import"_fly_string,
                .prelude = tokens.dump_string(),
                .description = MUST(String::formatted("Unable to parse `{}` as a valid layer.", component_value.function().original_source_text())),
            });
        }
    }

    // <import-conditions> = [ supports( [ <supports-condition> | <declaration> ] ) ]?
    //                      <media-query-list>?
    tokens.discard_whitespace();
    RefPtr<Supports> supports {};
    if (tokens.next_token().is_function("supports"sv)) {
        auto component_value = tokens.consume_a_token();
        TokenStream supports_tokens { component_value.function().value };
        supports = parse_a_supports(supports_tokens);
        if (!supports) {
            m_rule_context.append(RuleContext::SupportsCondition);
            auto supports_declaration = parse_supports_declaration(supports_tokens);
            m_rule_context.take_last();
            if (supports_declaration)
                supports = Supports::create(supports_declaration.release_nonnull<BooleanExpression>());
        }
    }

    auto media_query_list = parse_a_media_query_list(tokens);

    if (tokens.has_next_token()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@import"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Trailing tokens in prelude."_string,
        });
        return {};
    }

    return CSSImportRule::create(realm(), url.release_value(), const_cast<DOM::Document*>(m_document.ptr()), move(layer), move(supports), MediaList::create(realm(), move(media_query_list)));
}

template<typename NestedDeclarationsRule>
GC::Ptr<CSSRule> Parser::convert_to_layer_rule(AtRule const& rule, Nested nested)
{
    // https://drafts.csswg.org/css-cascade-5/#at-layer
    if (rule.is_block_rule) {
        // CSSLayerBlockRule
        // @layer <layer-name>? {
        //   <rule-list>
        // }

        // First, the name
        FlyString layer_name = {};
        auto serialized_layer_name = serialize_component_values_for_reparsing(rule.prelude);
        if (auto maybe_name = RustComponentValueParser::parse_a_layer_name(serialized_layer_name.bytes_as_string_view(), "utf-8"sv, RustComponentValueParser::AllowBlankLayerName::Yes); maybe_name.has_value()) {
            layer_name = maybe_name.release_value();
        } else {
            ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                .rule_name = "@layer"_fly_string,
                .prelude = serialized_layer_name,
                .description = "Not a valid layer name."_string,
            });
            return {};
        }

        // Then the rules
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
        return CSSLayerBlockRule::create(realm(), layer_name, rule_list);
    }

    // CSSLayerStatementRule
    // @layer <layer-name>#;
    auto serialized_layer_names = serialize_component_values_for_reparsing(rule.prelude);
    auto layer_names = RustComponentValueParser::parse_a_layer_name_list(serialized_layer_names.bytes_as_string_view(), "utf-8"sv);
    if (!layer_names.has_value()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@layer"_fly_string,
            .prelude = serialized_layer_names,
            .description = "Contains invalid layer name."_string,
        });
        return {};
    }

    return CSSLayerStatementRule::create(realm(), layer_names.release_value());
}

GC::Ptr<CSSKeyframesRule> Parser::convert_to_keyframes_rule(AtRule const& rule)
{
    // https://drafts.csswg.org/css-animations/#keyframes
    // @keyframes = @keyframes <keyframes-name> { <qualified-rule-list> }
    // <keyframes-name> = <custom-ident> | <string>
    // <keyframe-block> = <keyframe-selector># { <declaration-list> }
    // <keyframe-selector> = from | to | <percentage [0,100]>
    auto prelude_stream = TokenStream { rule.prelude };
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@keyframes"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return nullptr;
    }

    if (rule.prelude.is_empty()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@keyframes"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Empty prelude."_string,
        });
        return {};
    }

    auto serialized_keyframes_name = serialize_component_values_for_reparsing(rule.prelude);
    auto name = RustComponentValueParser::parse_a_keyframes_name(serialized_keyframes_name.bytes_as_string_view(), "utf-8"sv);
    if (!name.has_value()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@keyframes"_fly_string,
            .prelude = serialized_keyframes_name,
            .description = "Invalid name."_string,
        });
        return {};
    }

    GC::RootVector<GC::Ref<CSSRule>> keyframes(realm().heap());
    rule.for_each_as_qualified_rule_list([&](auto& qualified_rule) {
        if (!qualified_rule.child_rules.is_empty()) {
            for (auto const& child_rule : qualified_rule.child_rules) {
                ErrorReporter::the().report(InvalidRuleLocationError {
                    .outer_rule_name = "@keyframes"_fly_string,
                    .inner_rule_name = child_rule.visit(
                        [](Rule const& rule) {
                            return rule.visit(
                                [](AtRule const& at_rule) { return MUST(String::formatted("@{}", at_rule.name)); },
                                [](QualifiedRule const&) { return "qualified-rule"_string; });
                        },
                        [](auto&) {
                            return "list-of-declarations"_string;
                        }),
                });
            }
        }

        auto serialized_keyframe_selector_list = serialize_component_values_for_reparsing(qualified_rule.prelude);
        auto maybe_selectors = RustComponentValueParser::parse_a_keyframe_selector_list(serialized_keyframe_selector_list.bytes_as_string_view(), "utf-8"sv);
        if (!maybe_selectors.has_value()) {
            ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                .rule_name = "keyframe"_fly_string,
                .prelude = serialized_keyframe_selector_list,
                .description = "Invalid selector."_string,
            });
            return;
        }

        PropertiesAndCustomProperties properties;
        qualified_rule.for_each_as_declaration_list("keyframe"_fly_string, [&](auto const& declaration) {
            extract_property(declaration, properties);
        });
        auto style = CSSStyleProperties::create(realm(), move(properties.properties), move(properties.custom_properties));
        for (auto& selector : maybe_selectors.value()) {
            auto keyframe_rule = CSSKeyframeRule::create(realm(), selector, *style);
            keyframes.append(keyframe_rule);
        }
    });

    return CSSKeyframesRule::create(realm(), name.release_value(), CSSRuleList::create(realm(), keyframes));
}

GC::Ptr<CSSNamespaceRule> Parser::convert_to_namespace_rule(AtRule const& rule)
{
    // https://drafts.csswg.org/css-namespaces/#syntax
    // @namespace <namespace-prefix>? [ <string> | <url> ] ;
    // <namespace-prefix> = <ident>
    auto tokens = TokenStream { rule.prelude };
    if (rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@namespace"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Must be a statement, not a block."_string,
        });
        return {};
    }

    if (rule.prelude.is_empty()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@namespace"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Empty prelude."_string,
        });
        return {};
    }

    auto serialized_namespace_rule_prelude = serialize_component_values_for_reparsing(rule.prelude);
    auto namespace_rule_prelude = RustComponentValueParser::parse_a_namespace_rule_prelude(serialized_namespace_rule_prelude.bytes_as_string_view(), "utf-8"sv);
    if (!namespace_rule_prelude.has_value()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@namespace"_fly_string,
            .prelude = serialized_namespace_rule_prelude,
            .description = "Unable to parse prelude."_string,
        });
        return {};
    }

    return CSSNamespaceRule::create(realm(), move(namespace_rule_prelude->prefix), namespace_rule_prelude->namespace_uri);
}

template<typename NestedDeclarationsRule>
GC::Ptr<CSSSupportsRule> Parser::convert_to_supports_rule(AtRule const& rule, Nested nested)
{
    // https://drafts.csswg.org/css-conditional-3/#at-supports
    // @supports <supports-condition> {
    //   <rule-list>
    // }
    auto supports_tokens = TokenStream { rule.prelude };
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@supports"_fly_string,
            .prelude = supports_tokens.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return {};
    }

    if (rule.prelude.is_empty()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@supports"_fly_string,
            .prelude = supports_tokens.dump_string(),
            .description = "Empty prelude."_string,
        });
        return {};
    }

    auto supports = parse_a_supports(supports_tokens);
    if (!supports) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@supports"_fly_string,
            .prelude = supports_tokens.dump_string(),
            .description = "Supports clause invalid."_string,
        });
        return {};
    }

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
    return CSSSupportsRule::create(realm(), supports.release_nonnull(), rule_list);
}

GC::Ptr<CSSPropertyRule> Parser::convert_to_property_rule(AtRule const& rule)
{
    // https://drafts.css-houdini.org/css-properties-values-api-1/#at-ruledef-property
    // @property <custom-property-name> {
    // <declaration-list>
    // }
    auto prelude_stream = TokenStream { rule.prelude };
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@property"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return {};
    }

    if (rule.prelude.is_empty()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@property"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Empty prelude."_string,
        });
        return {};
    }

    auto serialized_custom_property_name = serialize_component_values_for_reparsing(rule.prelude);
    auto name = RustComponentValueParser::parse_a_custom_property_name(serialized_custom_property_name.bytes_as_string_view(), "utf-8"sv);
    if (!name.has_value()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@property"_fly_string,
            .prelude = serialized_custom_property_name,
            .description = "Name must be an ident starting with '--'."_string,
        });
        return {};
    }

    Optional<FlyString> syntax_maybe;
    Optional<bool> inherits_maybe;
    RefPtr<StyleValue const> initial_value_maybe;

    rule.for_each_as_declaration_list([&](auto& declaration) {
        if (auto descriptor = convert_to_descriptor(AtRuleID::Property, declaration); descriptor.has_value()) {
            if (descriptor->descriptor_name_and_id.id() == DescriptorID::Syntax) {
                if (descriptor->value->is_string())
                    syntax_maybe = descriptor->value->as_string().string_value();
                return;
            }
            if (descriptor->descriptor_name_and_id.id() == DescriptorID::Inherits) {
                switch (descriptor->value->to_keyword()) {
                case Keyword::True:
                    inherits_maybe = true;
                    break;
                case Keyword::False:
                    inherits_maybe = false;
                    break;
                default:
                    break;
                }
                return;
            }
            if (descriptor->descriptor_name_and_id.id() == DescriptorID::InitialValue) {
                initial_value_maybe = *descriptor->value;
                return;
            }
        }
    });

    // @property rules require a syntax and inherits descriptor; if either are missing, the entire rule is invalid and must be ignored.
    if (!syntax_maybe.has_value() || syntax_maybe->is_empty() || !inherits_maybe.has_value()) {
        return {};
    }

    CSS::Parser::ParsingParams parsing_params;
    if (document())
        parsing_params = CSS::Parser::ParsingParams { *document() };
    else
        parsing_params = CSS::Parser::ParsingParams { realm() };

    auto syntax_component_values = parse_component_values_list(parsing_params, syntax_maybe.value());
    auto maybe_syntax = parse_as_syntax(syntax_component_values, LimitSingleComponentIdentToCustomIdent::Yes);

    // If the provided string is not a valid syntax string (if it returns failure when consume
    // a syntax definition is called on it), the descriptor is invalid and must be ignored.
    if (!maybe_syntax) {
        return {};
    }
    // The initial-value descriptor is optional only if the syntax is the universal syntax definition,
    // otherwise the descriptor is required; if it’s missing, the entire rule is invalid and must be ignored.
    if (!initial_value_maybe && maybe_syntax->type() != CSS::Parser::SyntaxNode::NodeType::Universal) {
        return {};
    }

    if (initial_value_maybe) {
        initial_value_maybe = Web::CSS::Parser::parse_with_a_syntax(parsing_params, initial_value_maybe->tokenize(), *maybe_syntax);

        // Otherwise, if the value of the syntax descriptor is not the universal syntax definition,
        // the following conditions must be met for the @property rule to be valid:
        if (maybe_syntax->type() != CSS::Parser::SyntaxNode::NodeType::Universal) {
            //  - The initial-value descriptor must be present.
            //  - The initial-value descriptor’s value must parse successfully according to the grammar specified by the syntax definition.
            //  - The initial-value must be computationally independent.
            if (!initial_value_maybe || initial_value_maybe->is_guaranteed_invalid() || !initial_value_maybe->is_computationally_independent())
                return {};
        }
    }

    return CSSPropertyRule::create(realm(), name.release_value(), syntax_maybe.value(), inherits_maybe.value(), move(initial_value_maybe));
}

// https://drafts.csswg.org/css-conditional-5/#container-rule
template<typename NestedDeclarationsRule>
GC::Ptr<CSSContainerRule> Parser::convert_to_container_rule(AtRule const& rule, Nested nested)
{
    // @container <container-condition># {
    //   <block-contents>
    // }
    // <container-condition> = [ <container-name>? <container-query>? ]!
    // <container-name> = <custom-ident>

    TokenStream prelude_stream { rule.prelude };
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@container"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return nullptr;
    }

    auto serialized_prelude = serialize_component_values_for_reparsing(rule.prelude);
    auto prelude_conditions = RustComponentValueParser::parse_container_rule_prelude(serialized_prelude.bytes_as_string_view(), "utf-8"sv);
    if (!prelude_conditions.has_value()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@container"_fly_string,
            .prelude = serialized_prelude,
            .description = "Empty prelude."_string,
        });
        return nullptr;
    }

    Vector<CSSContainerRule::Condition> conditions;
    conditions.ensure_capacity(prelude_conditions->size());

    for (auto& prelude_condition : prelude_conditions.value()) {
        RefPtr<ContainerQuery> container_query;
        if (prelude_condition.query.has_value()) {
            auto query_condition = RustComponentValueParser::parse_a_container_condition(prelude_condition.query.value().bytes_as_string_view(), "utf-8"sv);
            if (!query_condition) {
                ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                    .rule_name = "@container"_fly_string,
                    .prelude = serialized_prelude,
                    .description = prelude_condition.name.has_value() ? "Trailing tokens after name and query."_string : "Missing container name or query."_string,
                });
                return nullptr;
            }
            container_query = ContainerQuery::create(query_condition.release_nonnull());
        }

        if (!prelude_condition.name.has_value() && !container_query) {
            ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                .rule_name = "@container"_fly_string,
                .prelude = serialized_prelude,
                .description = "Missing container name or query."_string,
            });
            return nullptr;
        }

        conditions.unchecked_empend(move(prelude_condition.name), move(container_query));
    }

    GC::RootVector<GC::Ref<CSSRule>> child_rules { realm().heap() };
    for (auto const& child : rule.child_rules_and_lists_of_declarations) {
        child.visit(
            [&](Rule const& child_rule) {
                if (auto converted_rule = convert_to_rule<NestedDeclarationsRule>(child_rule, nested))
                    child_rules.append(*converted_rule);
            },
            [&](Vector<Declaration> const& declarations) {
                child_rules.append(CSSNestedDeclarations::create(realm(), *convert_to_style_declaration(declarations)));
            });
    }

    auto rule_list = CSSRuleList::create(realm(), child_rules);
    return CSSContainerRule::create(realm(), move(conditions), rule_list);
}

GC::Ptr<CSSCounterStyleRule> Parser::convert_to_counter_style_rule(AtRule const& rule)
{
    // https://drafts.csswg.org/css-counter-styles-3/#the-counter-style-rule
    TokenStream prelude_stream { rule.prelude };
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@counter-style"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return nullptr;
    }

    if (rule.prelude.is_empty()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@counter-style"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Empty prelude."_string,
        });
        return nullptr;
    }

    auto serialized_counter_style_name = serialize_component_values_for_reparsing(rule.prelude);
    auto name = RustComponentValueParser::parse_a_counter_style_name(serialized_counter_style_name.bytes_as_string_view(), "utf-8"sv);
    if (!name.has_value()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@counter-style"_fly_string,
            .prelude = serialized_counter_style_name,
            .description = "Missing counter style name."_string,
        });
        return nullptr;
    }

    // https://drafts.csswg.org/css-counter-styles-3/#the-counter-style-rule
    // Counter style names are case-sensitive. However, the names defined in this specification are ASCII lowercased
    // on parse wherever they are used as counter styles, e.g. in the list-style set of properties, in the
    // @counter-style rule, and in the counter() functions.
    auto const& keyword = keyword_from_string(name.value());
    if (keyword.has_value() && keyword_to_counter_style_name_keyword(keyword.value()).has_value())
        name = name->to_ascii_lowercase();

    // https://drafts.csswg.org/css-counter-styles-3/#typedef-counter-style-name
    // When used here, to define a counter style, it also cannot be any of the non-overridable counter-style names
    // FIXME: We should allow these in the UA stylesheet in order to initially define them.
    if (CSSCounterStyleRule::matches_non_overridable_counter_style_name(name.value()) && m_is_ua_style_sheet != IsUAStyleSheet::Yes) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@counter-style"_fly_string,
            .prelude = serialized_counter_style_name,
            .description = "Non-overridable counter style name."_string,
        });
        return nullptr;
    }

    RefPtr<StyleValue const> system;
    RefPtr<StyleValue const> negative;
    RefPtr<StyleValue const> prefix;
    RefPtr<StyleValue const> suffix;
    RefPtr<StyleValue const> range;
    RefPtr<StyleValue const> pad;
    RefPtr<StyleValue const> fallback;
    RefPtr<StyleValue const> symbols;
    RefPtr<StyleValue const> additive_symbols;
    RefPtr<StyleValue const> speak_as;

    rule.for_each_as_declaration_list([&](auto& declaration) {
        auto const& descriptor = convert_to_descriptor(AtRuleID::CounterStyle, declaration);
        if (!descriptor.has_value())
            return;

        switch (descriptor->descriptor_name_and_id.id()) {
        case DescriptorID::System:
            system = descriptor->value;
            break;
        case DescriptorID::Negative:
            negative = descriptor->value;
            break;
        case DescriptorID::Prefix:
            prefix = descriptor->value;
            break;
        case DescriptorID::Suffix:
            suffix = descriptor->value;
            break;
        case DescriptorID::Range:
            range = descriptor->value;
            break;
        case DescriptorID::Pad:
            pad = descriptor->value;
            break;
        case DescriptorID::Fallback:
            fallback = descriptor->value;
            break;
        case DescriptorID::Symbols:
            symbols = descriptor->value;
            break;
        case DescriptorID::AdditiveSymbols:
            additive_symbols = descriptor->value;
            break;
        case DescriptorID::SpeakAs:
            speak_as = descriptor->value;
            break;
        default:
            VERIFY_NOT_REACHED();
        }
    });

    return CSSCounterStyleRule::create(realm(), name.release_value(), move(system), move(negative), move(prefix), move(suffix), move(range), move(pad), move(fallback), move(symbols), move(additive_symbols), move(speak_as));
}

GC::Ptr<CSSFontFaceRule> Parser::convert_to_font_face_rule(AtRule const& rule)
{
    // https://drafts.csswg.org/css-fonts/#font-face-rule
    TokenStream prelude_stream { rule.prelude };
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@font-face"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return nullptr;
    }

    auto serialized_prelude = serialize_component_values_for_reparsing(rule.prelude);
    if (!RustComponentValueParser::parse_empty_prelude(serialized_prelude.bytes_as_string_view(), "utf-8"sv)) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@font-face"_fly_string,
            .prelude = serialized_prelude,
            .description = "Prelude is not allowed."_string,
        });
        return {};
    }

    DescriptorList descriptors { AtRuleID::FontFace };
    rule.for_each_as_declaration_list([&](auto& declaration) {
        if (auto descriptor = convert_to_descriptor(AtRuleID::FontFace, declaration); descriptor.has_value()) {
            descriptors.append(descriptor.release_value());
        }
    });

    return CSSFontFaceRule::create(realm(), CSSFontFaceDescriptors::create(realm(), descriptors.release_descriptors()));
}

GC::Ptr<CSSFontFeatureValuesRule> Parser::convert_to_font_feature_values_rule(AtRule const& rule)
{
    // https://drafts.csswg.org/css-fonts-4/#font-feature-values-syntax
    // @font-feature-values = @font-feature-values <family-name># { <declaration-rule-list> }
    auto prelude_stream = TokenStream { rule.prelude };
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@font-feature-values"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return nullptr;
    }

    auto serialized_prelude = serialize_component_values_for_reparsing(rule.prelude);
    auto family_names = RustComponentValueParser::parse_font_feature_values_family_name_list(serialized_prelude.bytes_as_string_view(), "utf-8"sv);

    if (!family_names.has_value()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@font-feature-values"_fly_string,
            .prelude = serialized_prelude,
            .description = "Invalid family name list."_string,
        });
        return nullptr;
    }

    auto font_feature_values_rule = CSSFontFeatureValuesRule::create(realm(), family_names.release_value());

    rule.for_each_as_declaration_rule_list(
        [&](AtRule const& at_rule) {
            // <font-feature-value-type> = <@stylistic> | <@historical-forms> | <@styleset> | <@character-variant> | <@swash> | <@ornaments> | <@annotation>
            // @stylistic = @stylistic { <declaration-list> }
            // @historical-forms = @historical-forms { <declaration-list> }
            // @styleset = @styleset { <declaration-list> }
            // @character-variant = @character-variant { <declaration-list> }
            // @swash = @swash { <declaration-list> }
            // @ornaments = @ornaments { <declaration-list> }
            // @annotation = @annotation { <declaration-list> }

            GC::Ptr<CSSFontFeatureValuesMap> feature_values_map;
            size_t max_value_count = 1;

            if (at_rule.name.equals_ignoring_ascii_case("stylistic"sv)) {
                feature_values_map = font_feature_values_rule->stylistic();
            } else if (at_rule.name.equals_ignoring_ascii_case("historical-forms"sv)) {
                feature_values_map = font_feature_values_rule->historical_forms();
            } else if (at_rule.name.equals_ignoring_ascii_case("styleset"sv)) {
                feature_values_map = font_feature_values_rule->styleset();
                max_value_count = NumericLimits<size_t>::max();
            } else if (at_rule.name.equals_ignoring_ascii_case("character-variant"sv)) {
                feature_values_map = font_feature_values_rule->character_variant();
                max_value_count = 2;
            } else if (at_rule.name.equals_ignoring_ascii_case("swash"sv)) {
                feature_values_map = font_feature_values_rule->swash();
            } else if (at_rule.name.equals_ignoring_ascii_case("ornaments"sv)) {
                feature_values_map = font_feature_values_rule->ornaments();
            } else if (at_rule.name.equals_ignoring_ascii_case("annotation"sv)) {
                feature_values_map = font_feature_values_rule->annotation();
            } else {
                // NB: Other at-rules are disallowed in this context and should have already been dropped
                VERIFY_NOT_REACHED();
            }

            at_rule.for_each_as_declaration_list([&](Declaration const& declaration) {
                auto value_stream = TokenStream { declaration.value };

                if (declaration.important == Important::Yes) {
                    ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                        .rule_name = MUST(String::formatted("@{}", at_rule.name)),
                        .prelude = value_stream.dump_string(),
                        .description = "Declarations in @font-feature-values rules cannot be marked !important."_string,
                    });
                    return;
                }

                value_stream.discard_whitespace();

                if (!value_stream.has_next_token()) {
                    ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                        .rule_name = MUST(String::formatted("@{}", at_rule.name)),
                        .prelude = value_stream.dump_string(),
                        .description = "Empty feature value."_string,
                    });
                    return;
                }

                Vector<u32> values;

                while (value_stream.has_next_token()) {
                    auto token = value_stream.consume_a_token();

                    // FIXME: Support calc()
                    if (!token.is(Token::Type::Number) || !token.token().is_integer() || token.token().to_integer() < 0) {
                        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                            .rule_name = MUST(String::formatted("@{}", at_rule.name)),
                            .prelude = value_stream.dump_string(),
                            .description = "Feature value entry must be a non-negative integer."_string,
                        });

                        return;
                    }

                    values.append(token.token().to_integer());

                    value_stream.discard_whitespace();
                }

                if (values.size() > max_value_count) {
                    ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                        .rule_name = MUST(String::formatted("@{}", at_rule.name)),
                        .prelude = value_stream.dump_string(),
                        .description = MUST(String::formatted("Too many feature values provided (maximum {})."_string, max_value_count)),
                    });

                    return;
                }

                MUST(feature_values_map->set(declaration.name.to_string(), move(values)));
            });
        },
        [&](Declaration const&) {
            // FIXME: Handle the `font-display` descriptor here, see
            //        https://drafts.csswg.org/css-fonts-4/#font-display-font-feature-values
        });

    return font_feature_values_rule;
}

static OwnPtr<SyntaxNode> parse_css_type(TokenStream<ComponentValue>& tokens)
{
    // https://drafts.csswg.org/css-mixins-1/#function-rule
    // <css-type> = <syntax-component> | <type()>
    // <type()> = type( <syntax> )

    auto transaction = tokens.begin_transaction();
    tokens.discard_whitespace();

    // <syntax-component>
    if (auto maybe_syntax_component = parse_syntax_component(tokens)) {
        transaction.commit();
        return maybe_syntax_component;
    }

    // <type()>
    auto maybe_type_function_token = tokens.consume_a_token();

    if (!maybe_type_function_token.is_function("type"sv))
        return nullptr;

    if (auto maybe_type_function_syntax = parse_as_syntax(maybe_type_function_token.function().value)) {
        transaction.commit();
        return maybe_type_function_syntax;
    }

    return nullptr;
}

Optional<Parser::FunctionPrelude> Parser::parse_function_prelude(TokenStream<ComponentValue>& tokens)
{
    // https://drafts.csswg.org/css-mixins-1/#function-rule
    // <function-token> <function-parameter>#? ) [ returns <css-type> ]?
    // <function-parameter> = <custom-property-name> <css-type>? [ : <default-value> ]?
    // <default-value> = <declaration-value>
    auto transaction = tokens.begin_transaction();

    tokens.discard_whitespace();

    auto const& function_token = tokens.consume_a_token();

    if (!function_token.is_function()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@function"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Prelude must start with a function token."_string,
        });
        return {};
    }

    auto function_name = function_token.function().name;

    // The <function-token> production must start with two dashes (U+002D HYPHEN-MINUS), similar to <dashed-ident>, or
    // else the definition is invalid.
    if (!function_name.starts_with_bytes("--"sv)) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@function"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Function name must start with two dashes."_string,
        });
        return {};
    }

    Vector<FunctionParameterInternal> parsed_parameters;

    TokenStream parameters_tokens { function_token.function().value };
    parameters_tokens.discard_whitespace();
    auto parameters_component_values = parse_a_comma_separated_list_of_component_values(parameters_tokens);

    // <function-parameter>#?
    for (auto const& parameter_component_values : parameters_component_values) {
        TokenStream parameter_tokens { parameter_component_values };
        parameter_tokens.discard_whitespace();

        // <custom-property-name>
        auto maybe_name = parse_dashed_ident(parameter_tokens);
        if (!maybe_name.has_value() || !is_a_custom_property_name_string(maybe_name.value())) {
            ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                .rule_name = "@function"_fly_string,
                .prelude = parameter_tokens.dump_string(),
                .description = "Parameter must have a name."_string,
            });
            return {};
        }

        // <css-type>?
        NonnullOwnPtr<SyntaxNode> type = UniversalSyntaxNode::create();
        if (auto maybe_type = parse_css_type(parameter_tokens))
            type = maybe_type.release_nonnull();

        parameter_tokens.discard_whitespace();

        // [ : <default-value> ]?
        Optional<Vector<ComponentValue>> default_value;
        if (parameter_tokens.next_token().is(Token::Type::Colon)) {
            parameter_tokens.discard_a_token(); // :
            parameter_tokens.discard_whitespace();

            auto maybe_default_value = parse_declaration_value(parameter_tokens);

            if (!maybe_default_value.has_value()) {
                ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                    .rule_name = "@function"_fly_string,
                    .prelude = parameter_tokens.dump_string(),
                    .description = "Expected default value after ':' in parameter"_string,
                });
                return {};
            }

            // If a default value and a parameter type are both provided, then the default value must parse successfully
            // according to that parameter type’s syntax. Otherwise, the @function rule is invalid.
            // FIXME: Chrome allows ASFs regardless of the parameter's type
            TokenStream default_value_token_stream { maybe_default_value.value() };
            if (!parse_according_to_syntax_node(default_value_token_stream, *type) || !default_value_token_stream.is_empty())
                return {};

            default_value = maybe_default_value;
        }

        parameter_tokens.discard_whitespace();

        if (!parameter_tokens.is_empty()) {
            ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                .rule_name = "@function"_fly_string,
                .prelude = parameter_tokens.dump_string(),
                .description = "Trailing tokens after parameter"_string,
            });
            return {};
        }

        parsed_parameters.append({ maybe_name.release_value(), move(type), move(default_value) });
    }

    tokens.discard_whitespace();

    NonnullOwnPtr<SyntaxNode> return_type = UniversalSyntaxNode::create();
    if (tokens.next_token().is_ident("returns"sv)) {
        tokens.discard_a_token();
        tokens.discard_whitespace();

        auto maybe_return_type = parse_css_type(tokens);

        if (!maybe_return_type) {
            ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
                .rule_name = "@function"_fly_string,
                .prelude = tokens.dump_string(),
                .description = "Expected return type after 'returns' in prelude."_string,
            });
            return {};
        }

        return_type = maybe_return_type.release_nonnull();
    }

    tokens.discard_whitespace();

    if (tokens.has_next_token()) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@function"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Trailing tokens in prelude."_string,
        });
        return {};
    }

    transaction.commit();
    return FunctionPrelude { move(function_name), move(parsed_parameters), move(return_type) };
}

GC::Ptr<CSSFunctionRule> Parser::convert_to_function_rule(AtRule const& function_rule)
{
    // https://drafts.csswg.org/css-mixins-1/#function-rule
    TokenStream prelude_stream { function_rule.prelude };

    if (!function_rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@function"_fly_string,
            .prelude = prelude_stream.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return nullptr;
    }

    auto prelude = parse_function_prelude(prelude_stream);

    if (!prelude.has_value())
        return nullptr;

    Vector<GC::Ref<CSSRule>> child_rules {};

    // https://drafts.csswg.org/css-mixins-1/#function-body
    for (auto const& child : function_rule.child_rules_and_lists_of_declarations) {
        child.visit(
            [&](Rule const& rule) {
                if (auto child_rule = convert_to_rule<CSSFunctionDeclarations>(rule, Nested::Yes))
                    child_rules.append(*child_rule);
            },
            [&](Vector<Declaration> const& declarations) {
                child_rules.append(CSSFunctionDeclarations::create(realm(), *this, declarations));
            });
    }

    return CSSFunctionRule::create(realm(), CSSRuleList::create(realm(), child_rules), move(prelude->name), move(prelude->parameters), move(prelude->return_type));
}

GC::Ptr<CSSPageRule> Parser::convert_to_page_rule(AtRule const& page_rule)
{
    // https://drafts.csswg.org/css-page-3/#syntax-page-selector
    // @page = @page <page-selector-list>? { <declaration-rule-list> }
    TokenStream tokens { page_rule.prelude };
    if (!page_rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = "@page"_fly_string,
            .prelude = tokens.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return nullptr;
    }

    auto page_selector_start = tokens.current_index();
    while (tokens.has_next_token())
        tokens.discard_a_token();

    auto serialized_page_selector_list = serialize_component_values_for_reparsing(tokens.tokens_since(page_selector_start));
    auto page_selectors = RustComponentValueParser::parse_a_page_selector_list(serialized_page_selector_list.bytes_as_string_view(), "utf-8"sv);
    if (!page_selectors.has_value()) {
        ErrorReporter::the().report(InvalidSelectorError {
            .rule_name = "@page"_fly_string,
            .value_string = serialized_page_selector_list,
            .description = "Invalid page selector list."_string,
        });
        return nullptr;
    }

    GC::RootVector<GC::Ref<CSSRule>> child_rules { realm().heap() };
    DescriptorList descriptors { AtRuleID::Page };
    page_rule.for_each_as_declaration_rule_list(
        [&](auto& at_rule) {
            if (auto converted_rule = convert_to_rule<CSSNestedDeclarations>(at_rule, Nested::No)) {
                if (is<CSSMarginRule>(*converted_rule)) {
                    child_rules.append(*converted_rule);
                } else {
                    ErrorReporter::the().report(InvalidRuleLocationError {
                        .outer_rule_name = "@page"_fly_string,
                        .inner_rule_name = MUST(FlyString::from_utf8(converted_rule->class_name())),
                    });
                }
            }
        },
        [&](auto& declaration) {
            if (auto descriptor = convert_to_descriptor(AtRuleID::Page, declaration); descriptor.has_value()) {
                descriptors.append(descriptor.release_value());
            }
        });

    auto rule_list = CSSRuleList::create(realm(), child_rules);
    return CSSPageRule::create(realm(), page_selectors.release_value(), CSSPageDescriptors::create(realm(), descriptors.release_descriptors()), rule_list);
}

GC::Ptr<CSSMarginRule> Parser::convert_to_margin_rule(AtRule const& rule)
{
    TokenStream prelude_stream { rule.prelude };
    if (!rule.is_block_rule) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = MUST(String::formatted("@{}", rule.name)),
            .prelude = prelude_stream.dump_string(),
            .description = "Must be a block, not a statement."_string,
        });
        return nullptr;
    }

    auto serialized_prelude = serialize_component_values_for_reparsing(rule.prelude);
    if (!RustComponentValueParser::parse_empty_prelude(serialized_prelude.bytes_as_string_view(), "utf-8"sv)) {
        ErrorReporter::the().report(CSS::Parser::InvalidRuleError {
            .rule_name = MUST(String::formatted("@{}", rule.name)),
            .prelude = serialized_prelude,
            .description = "Prelude is not allowed."_string,
        });
        return {};
    }

    // https://drafts.csswg.org/css-page-3/#syntax-page-selector
    // There are lots of these, but they're all in the format:
    // @foo = @foo { <declaration-list> };

    // FIXME: The declaration list should be a CSSMarginDescriptors, but that has no spec definition:
    //        https://github.com/w3c/csswg-drafts/issues/10106
    //        So, we just parse a CSSStyleProperties instead for now.
    PropertiesAndCustomProperties properties;
    rule.for_each_as_declaration_list([&](auto const& declaration) {
        extract_property(declaration, properties);
    });
    auto style = CSSStyleProperties::create(realm(), move(properties.properties), move(properties.custom_properties));
    return CSSMarginRule::create(realm(), rule.name, style);
}

template<typename Descriptors>
GC::Ref<Descriptors> Parser::convert_to_descriptors(AtRuleID at_rule_id, Vector<Declaration> const& declarations)
{
    DescriptorList descriptor_list { at_rule_id };

    for (auto const& declaration : declarations) {
        if (auto descriptor = convert_to_descriptor(at_rule_id, declaration); descriptor.has_value())
            descriptor_list.append(descriptor.release_value());
    }

    return Descriptors::create(realm(), descriptor_list.release_descriptors());
}

template GC::Ref<CSSFunctionDescriptors> Parser::convert_to_descriptors(AtRuleID at_rule_id, Vector<Declaration> const& declarations);

template GC::Ptr<CSSRule> Parser::convert_to_rule<CSSNestedDeclarations>(Rule const&, Nested);
template GC::Ptr<CSSRule> Parser::convert_to_rule<CSSFunctionDeclarations>(Rule const&, Nested);

template GC::Ptr<CSSContainerRule> Parser::convert_to_container_rule<CSSNestedDeclarations>(AtRule const&, Nested);
template GC::Ptr<CSSContainerRule> Parser::convert_to_container_rule<CSSFunctionDeclarations>(AtRule const&, Nested);

template GC::Ptr<CSSRule> Parser::convert_to_layer_rule<CSSNestedDeclarations>(AtRule const& rule, Nested);

template GC::Ptr<CSSSupportsRule> Parser::convert_to_supports_rule<CSSNestedDeclarations>(AtRule const&, Nested);
template GC::Ptr<CSSSupportsRule> Parser::convert_to_supports_rule<CSSFunctionDeclarations>(AtRule const&, Nested);

}
