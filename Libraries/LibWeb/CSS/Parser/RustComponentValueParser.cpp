/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringBuilder.h>
#include <LibTextCodec/Decoder.h>
#include <LibWeb/CSS/CharacterTypes.h>
#include <LibWeb/CSS/ContainerQuery.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/Keyword.h>
#include <LibWeb/CSS/Length.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>
#include <LibWeb/CSS/Parser/RustTokenizer.h>
#include <LibWeb/CSS/Parser/Syntax.h>
#include <LibWeb/CSS/PropertyName.h>
#include <LibWeb/CSS/Ratio.h>
#include <LibWeb/CSS/Resolution.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/RatioStyleValue.h>
#include <LibWeb/CSS/StyleValues/ResolutionStyleValue.h>
#include <LibWeb/CSS/Supports.h>
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

static RustComponentValueParser::RustStyleColor style_color_from_callback_payload(bool is_simple, double kind, u8 red, u8 green, u8 blue, u8 alpha, u8 const* value_ptr, size_t value_len)
{
    if (!is_simple) {
        return {
            .source = string_from_ffi_bytes(value_ptr, value_len),
        };
    }

    auto name = string_from_ffi_bytes(value_ptr, value_len);
    return {
        .is_simple = true,
        .kind = static_cast<FFI::CssParsedColorKind>(static_cast<u8>(kind)),
        .red = red,
        .green = green,
        .blue = blue,
        .alpha = alpha,
        .name = name.is_empty() ? Optional<String> {} : Optional<String> { move(name) },
    };
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

static void set_original_value_text(Declaration& declaration)
{
    // https://drafts.csswg.org/css-syntax/#consume-declaration
    // If decl’s name is a custom property name string, then set decl’s original text to the
    // segment of the original source text string corresponding to the tokens of decl’s value.
    //
    // NB: We preserve this for all declarations so downstream property and descriptor parsing
    //     can pass the original value source back to Rust without serializing component values.
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

static Selector::Combinator selector_combinator_from_ffi(FFI::CssSelectorCombinator combinator)
{
    switch (combinator) {
    case FFI::CssSelectorCombinator::None:
        return Selector::Combinator::None;
    case FFI::CssSelectorCombinator::ImmediateChild:
        return Selector::Combinator::ImmediateChild;
    case FFI::CssSelectorCombinator::Descendant:
        return Selector::Combinator::Descendant;
    case FFI::CssSelectorCombinator::NextSibling:
        return Selector::Combinator::NextSibling;
    case FFI::CssSelectorCombinator::SubsequentSibling:
        return Selector::Combinator::SubsequentSibling;
    case FFI::CssSelectorCombinator::Column:
        return Selector::Combinator::Column;
    }
    VERIFY_NOT_REACHED();
}

static Selector::SimpleSelector::QualifiedName::NamespaceType selector_namespace_type_from_ffi(FFI::CssSelectorNamespaceType namespace_type)
{
    switch (namespace_type) {
    case FFI::CssSelectorNamespaceType::Default:
        return Selector::SimpleSelector::QualifiedName::NamespaceType::Default;
    case FFI::CssSelectorNamespaceType::None:
        return Selector::SimpleSelector::QualifiedName::NamespaceType::None;
    case FFI::CssSelectorNamespaceType::Any:
        return Selector::SimpleSelector::QualifiedName::NamespaceType::Any;
    case FFI::CssSelectorNamespaceType::Named:
        return Selector::SimpleSelector::QualifiedName::NamespaceType::Named;
    }
    VERIFY_NOT_REACHED();
}

static Selector::SimpleSelector::Attribute::MatchType selector_attribute_match_type_from_ffi(FFI::CssAttributeMatchType match_type)
{
    switch (match_type) {
    case FFI::CssAttributeMatchType::HasAttribute:
        return Selector::SimpleSelector::Attribute::MatchType::HasAttribute;
    case FFI::CssAttributeMatchType::ExactValueMatch:
        return Selector::SimpleSelector::Attribute::MatchType::ExactValueMatch;
    case FFI::CssAttributeMatchType::ContainsWord:
        return Selector::SimpleSelector::Attribute::MatchType::ContainsWord;
    case FFI::CssAttributeMatchType::ContainsString:
        return Selector::SimpleSelector::Attribute::MatchType::ContainsString;
    case FFI::CssAttributeMatchType::StartsWithSegment:
        return Selector::SimpleSelector::Attribute::MatchType::StartsWithSegment;
    case FFI::CssAttributeMatchType::StartsWithString:
        return Selector::SimpleSelector::Attribute::MatchType::StartsWithString;
    case FFI::CssAttributeMatchType::EndsWithString:
        return Selector::SimpleSelector::Attribute::MatchType::EndsWithString;
    }
    VERIFY_NOT_REACHED();
}

static Selector::SimpleSelector::Attribute::CaseType selector_attribute_case_type_from_ffi(FFI::CssAttributeCaseType case_type)
{
    switch (case_type) {
    case FFI::CssAttributeCaseType::DefaultMatch:
        return Selector::SimpleSelector::Attribute::CaseType::DefaultMatch;
    case FFI::CssAttributeCaseType::CaseSensitiveMatch:
        return Selector::SimpleSelector::Attribute::CaseType::CaseSensitiveMatch;
    case FFI::CssAttributeCaseType::CaseInsensitiveMatch:
        return Selector::SimpleSelector::Attribute::CaseType::CaseInsensitiveMatch;
    }
    VERIFY_NOT_REACHED();
}

static Selector::SimpleSelector::QualifiedName selector_qualified_name_from_ffi(FFI::CssSelectorEvent const& event)
{
    return Selector::SimpleSelector::QualifiedName {
        .namespace_type = selector_namespace_type_from_ffi(event.namespace_type),
        .namespace_ = fly_string_from_ffi_bytes(event.namespace_ptr, event.namespace_len),
        .name = fly_string_from_ffi_bytes(event.name_ptr, event.name_len),
    };
}

struct SelectorBuilder {
    struct SelectorListFrame {
        SelectorList selectors;
        Vector<Selector::CompoundSelector> compound_selectors;
        Optional<Selector::CompoundSelector> current_compound_selector;
    };

    struct PendingPseudoSelector {
        enum class Type : u8 {
            PseudoClass,
            PseudoElement,
        };

        Type type;
        Selector::SimpleSelector::PseudoClassSelector pseudo_class {};
        PseudoElement pseudo_element { PseudoElement::KnownPseudoElementCount };
        String pseudo_element_name;
        FFI::CssPseudoElementValueKind pseudo_element_value_kind { FFI::CssPseudoElementValueKind::Empty };
        Selector::PseudoElementSelector::Value pseudo_element_value {};
    };

    Optional<SelectorList> root_selector_list;
    Vector<SelectorListFrame> selector_list_stack;
    Vector<PendingPseudoSelector> pending_pseudo_selectors;
    Optional<ComponentValueBuilder> invalid_selector_builder;
    bool failed { false };

    void fail()
    {
        failed = true;
    }

    SelectorListFrame& current_selector_list_frame()
    {
        VERIFY(!selector_list_stack.is_empty());
        return selector_list_stack.last();
    }

    Selector::CompoundSelector& current_compound_selector()
    {
        auto& frame = current_selector_list_frame();
        VERIFY(frame.current_compound_selector.has_value());
        return *frame.current_compound_selector;
    }

    void append_simple_selector(Selector::SimpleSelector simple_selector)
    {
        current_compound_selector().simple_selectors.append(move(simple_selector));
    }

    void finish_invalid_selector()
    {
        VERIFY(invalid_selector_builder.has_value());
        auto component_values = move(invalid_selector_builder->root_values);
        invalid_selector_builder.clear();

        while (!component_values.is_empty() && component_values.first().is(Token::Type::Whitespace))
            component_values.take_first();
        while (!component_values.is_empty() && component_values.last().is(Token::Type::Whitespace))
            component_values.take_last();

        append_simple_selector(Selector::SimpleSelector {
            .type = Selector::SimpleSelector::Type::Invalid,
            .value = Selector::SimpleSelector::Invalid {
                .component_values = move(component_values),
            },
        });
    }

    void finish_selector_list(SelectorList selector_list)
    {
        if (selector_list_stack.is_empty() && pending_pseudo_selectors.is_empty()) {
            root_selector_list = move(selector_list);
            return;
        }

        if (pending_pseudo_selectors.is_empty()) {
            fail();
            return;
        }

        auto& pending_pseudo_selector = pending_pseudo_selectors.last();
        switch (pending_pseudo_selector.type) {
        case PendingPseudoSelector::Type::PseudoClass:
            pending_pseudo_selector.pseudo_class.argument_selector_list = move(selector_list);
            break;
        case PendingPseudoSelector::Type::PseudoElement:
            if (pending_pseudo_selector.pseudo_element_value_kind != FFI::CssPseudoElementValueKind::CompoundSelector || selector_list.size() != 1) {
                fail();
                return;
            }
            pending_pseudo_selector.pseudo_element_value = selector_list.take_first();
            break;
        }
    }
    void finish_pseudo_class_selector()
    {
        VERIFY(!pending_pseudo_selectors.is_empty());
        auto pending_pseudo_selector = pending_pseudo_selectors.take_last();
        VERIFY(pending_pseudo_selector.type == PendingPseudoSelector::Type::PseudoClass);

        append_simple_selector(Selector::SimpleSelector {
            .type = Selector::SimpleSelector::Type::PseudoClass,
            .value = move(pending_pseudo_selector.pseudo_class),
        });
    }

    void finish_pseudo_element_selector()
    {
        VERIFY(!pending_pseudo_selectors.is_empty());
        auto pending_pseudo_selector = pending_pseudo_selectors.take_last();
        VERIFY(pending_pseudo_selector.type == PendingPseudoSelector::Type::PseudoElement);

        Selector::PseudoElementSelector pseudo_element_selector = pending_pseudo_selector.pseudo_element_name.is_empty()
            ? Selector::PseudoElementSelector { pending_pseudo_selector.pseudo_element, move(pending_pseudo_selector.pseudo_element_value) }
            : Selector::PseudoElementSelector { pending_pseudo_selector.pseudo_element, move(pending_pseudo_selector.pseudo_element_name), move(pending_pseudo_selector.pseudo_element_value) };

        append_simple_selector(Selector::SimpleSelector {
            .type = Selector::SimpleSelector::Type::PseudoElement,
            .value = move(pseudo_element_selector),
        });
    }

    void handle_event(FFI::CssSelectorEvent const& event)
    {
        if (failed)
            return;

        switch (event.kind) {
        case FFI::CssSelectorEventKind::SelectorListStart:
            selector_list_stack.append({});
            break;
        case FFI::CssSelectorEventKind::SelectorListEnd: {
            if (selector_list_stack.is_empty() || selector_list_stack.last().current_compound_selector.has_value()) {
                fail();
                return;
            }
            auto selector_list = move(selector_list_stack.take_last().selectors);
            finish_selector_list(move(selector_list));
            break;
        }
        case FFI::CssSelectorEventKind::SelectorStart:
            current_selector_list_frame().compound_selectors.clear();
            break;
        case FFI::CssSelectorEventKind::SelectorEnd: {
            auto& frame = current_selector_list_frame();
            if (frame.current_compound_selector.has_value()) {
                fail();
                return;
            }
            frame.selectors.append(Selector::create(move(frame.compound_selectors)));
            break;
        }
        case FFI::CssSelectorEventKind::CompoundSelectorStart:
            current_selector_list_frame().current_compound_selector = Selector::CompoundSelector {
                .combinator = selector_combinator_from_ffi(event.combinator),
            };
            break;
        case FFI::CssSelectorEventKind::CompoundSelectorEnd: {
            auto& frame = current_selector_list_frame();
            if (!frame.current_compound_selector.has_value()) {
                fail();
                return;
            }
            frame.compound_selectors.append(frame.current_compound_selector.release_value());
            break;
        }
        case FFI::CssSelectorEventKind::SimpleSelector:
            handle_simple_selector_event(event);
            break;
        case FFI::CssSelectorEventKind::PseudoClassSelectorStart: {
            Selector::SimpleSelector::PseudoClassSelector pseudo_class {
                .type = static_cast<PseudoClass>(event.pseudo_class_id),
                .is_forgiving = event.is_forgiving,
            };
            if (event.has_an_plus_b_pattern) {
                pseudo_class.an_plus_b_pattern = {
                    .step_size = event.an_plus_b_step_size,
                    .offset = event.an_plus_b_offset,
                };
            }
            pending_pseudo_selectors.append(PendingPseudoSelector {
                .type = PendingPseudoSelector::Type::PseudoClass,
                .pseudo_class = move(pseudo_class),
            });
            break;
        }
        case FFI::CssSelectorEventKind::PseudoClassSelectorEnd:
            finish_pseudo_class_selector();
            break;
        case FFI::CssSelectorEventKind::PseudoClassArgumentString:
            handle_pseudo_class_argument_string(event);
            break;
        case FFI::CssSelectorEventKind::PseudoClassArgumentNumber: {
            if (pending_pseudo_selectors.is_empty() || pending_pseudo_selectors.last().type != PendingPseudoSelector::Type::PseudoClass) {
                fail();
                return;
            }
            pending_pseudo_selectors.last().pseudo_class.levels.append(event.argument_number);
            break;
        }
        case FFI::CssSelectorEventKind::PseudoElementSelectorStart:
            handle_pseudo_element_selector_start(event);
            break;
        case FFI::CssSelectorEventKind::PseudoElementSelectorEnd:
            finish_pseudo_element_selector();
            break;
        case FFI::CssSelectorEventKind::PseudoElementArgumentString: {
            if (pending_pseudo_selectors.is_empty() || pending_pseudo_selectors.last().type != PendingPseudoSelector::Type::PseudoElement) {
                fail();
                return;
            }
            auto& pending_pseudo_selector = pending_pseudo_selectors.last();
            auto ident_list = pending_pseudo_selector.pseudo_element_value.get_pointer<Selector::PseudoElementSelector::IdentList>();
            if (ident_list == nullptr) {
                fail();
                return;
            }
            ident_list->append(fly_string_from_ffi_bytes(event.value_ptr, event.value_len));
            break;
        }
        case FFI::CssSelectorEventKind::InvalidSelectorStart:
            invalid_selector_builder = ComponentValueBuilder {};
            break;
        case FFI::CssSelectorEventKind::InvalidSelectorEnd:
            finish_invalid_selector();
            break;
        }
    }

    void handle_component_value(FFI::CssComponentValue const& component_value)
    {
        if (!invalid_selector_builder.has_value()) {
            fail();
            return;
        }
        append_component_value_token(*invalid_selector_builder, component_value.kind, RustTokenizer::token_from_ffi(component_value.token));
    }

    void handle_simple_selector_event(FFI::CssSelectorEvent const& event)
    {
        switch (event.simple_selector_kind) {
        case FFI::CssSimpleSelectorKind::Universal:
            append_simple_selector(Selector::SimpleSelector {
                .type = Selector::SimpleSelector::Type::Universal,
                .value = selector_qualified_name_from_ffi(event),
            });
            break;
        case FFI::CssSimpleSelectorKind::TagName:
            append_simple_selector(Selector::SimpleSelector {
                .type = Selector::SimpleSelector::Type::TagName,
                .value = selector_qualified_name_from_ffi(event),
            });
            break;
        case FFI::CssSimpleSelectorKind::Id:
            append_simple_selector(Selector::SimpleSelector {
                .type = Selector::SimpleSelector::Type::Id,
                .value = Selector::SimpleSelector::Name { fly_string_from_ffi_bytes(event.name_ptr, event.name_len) },
            });
            break;
        case FFI::CssSimpleSelectorKind::Class:
            append_simple_selector(Selector::SimpleSelector {
                .type = Selector::SimpleSelector::Type::Class,
                .value = Selector::SimpleSelector::Name { fly_string_from_ffi_bytes(event.name_ptr, event.name_len) },
            });
            break;
        case FFI::CssSimpleSelectorKind::Attribute:
            append_simple_selector(Selector::SimpleSelector {
                .type = Selector::SimpleSelector::Type::Attribute,
                .value = Selector::SimpleSelector::Attribute {
                    .match_type = selector_attribute_match_type_from_ffi(event.attribute_match_type),
                    .qualified_name = selector_qualified_name_from_ffi(event),
                    .value = string_from_ffi_bytes(event.value_ptr, event.value_len),
                    .case_type = selector_attribute_case_type_from_ffi(event.attribute_case_type),
                },
            });
            break;
        case FFI::CssSimpleSelectorKind::Nesting:
            append_simple_selector(Selector::SimpleSelector {
                .type = Selector::SimpleSelector::Type::Nesting,
            });
            break;
        }
    }

    void handle_pseudo_class_argument_string(FFI::CssSelectorEvent const& event)
    {
        if (pending_pseudo_selectors.is_empty() || pending_pseudo_selectors.last().type != PendingPseudoSelector::Type::PseudoClass) {
            fail();
            return;
        }

        auto& pseudo_class = pending_pseudo_selectors.last().pseudo_class;
        switch (pseudo_class_metadata(pseudo_class.type).parameter_type) {
        case PseudoClassMetadata::ParameterType::Ident: {
            auto string_value = fly_string_from_ffi_bytes(event.value_ptr, event.value_len);
            pseudo_class.ident = Selector::SimpleSelector::PseudoClassSelector::Ident {
                .keyword = keyword_from_string(string_value).value_or(Keyword::Invalid),
                .string_value = move(string_value),
            };
            break;
        }
        case PseudoClassMetadata::ParameterType::LanguageRanges:
            pseudo_class.languages.append(fly_string_from_ffi_bytes(event.value_ptr, event.value_len));
            break;
        case PseudoClassMetadata::ParameterType::ANPlusB:
        case PseudoClassMetadata::ParameterType::ANPlusBOf:
        case PseudoClassMetadata::ParameterType::CompoundSelector:
        case PseudoClassMetadata::ParameterType::ForgivingRelativeSelectorList:
        case PseudoClassMetadata::ParameterType::ForgivingSelectorList:
        case PseudoClassMetadata::ParameterType::LevelList:
        case PseudoClassMetadata::ParameterType::RelativeSelectorList:
        case PseudoClassMetadata::ParameterType::SelectorList:
        case PseudoClassMetadata::ParameterType::None:
            fail();
            break;
        }
    }

    void handle_pseudo_element_selector_start(FFI::CssSelectorEvent const& event)
    {
        Selector::PseudoElementSelector::Value value;
        switch (event.pseudo_element_value_kind) {
        case FFI::CssPseudoElementValueKind::Empty:
        case FFI::CssPseudoElementValueKind::CompoundSelector:
            value = Empty {};
            break;
        case FFI::CssPseudoElementValueKind::PTNameSelector:
            value = Selector::PseudoElementSelector::PTNameSelector {
                .is_universal = event.is_universal,
                .value = fly_string_from_ffi_bytes(event.value_ptr, event.value_len),
            };
            break;
        case FFI::CssPseudoElementValueKind::IdentList:
            value = Selector::PseudoElementSelector::IdentList {};
            break;
        }

        pending_pseudo_selectors.append(PendingPseudoSelector {
            .type = PendingPseudoSelector::Type::PseudoElement,
            .pseudo_element = static_cast<PseudoElement>(event.pseudo_element_id),
            .pseudo_element_name = string_from_ffi_bytes(event.name_ptr, event.name_len),
            .pseudo_element_value_kind = event.pseudo_element_value_kind,
            .pseudo_element_value = move(value),
        });
    }
};

Optional<SelectorList> RustComponentValueParser::parse_a_selector_list(StringView input, StringView encoding, SelectorType selector_type, SelectorParsingMode parsing_mode, HashTable<FlyString> const& declared_namespaces)
{
    Vector<FFI::CssSelectorNamespace> ffi_declared_namespaces;
    ffi_declared_namespaces.ensure_capacity(declared_namespaces.size());
    for (auto const& namespace_ : declared_namespaces) {
        auto bytes = namespace_.bytes_as_string_view().bytes();
        ffi_declared_namespaces.unchecked_append(FFI::CssSelectorNamespace {
            .prefix_ptr = bytes.data(),
            .prefix_len = bytes.size(),
        });
    }

    SelectorBuilder builder;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    bool const did_parse = FFI::rust_css_parse_selector_list(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        static_cast<u8>(selector_type),
        static_cast<u8>(parsing_mode),
        ffi_declared_namespaces.data(),
        ffi_declared_namespaces.size(),
        &builder,
        [](void* raw_builder, FFI::CssSelectorEvent const* event) {
            auto& builder = *static_cast<SelectorBuilder*>(raw_builder);
            builder.handle_event(*event);
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            auto& builder = *static_cast<SelectorBuilder*>(raw_builder);
            builder.handle_component_value(*component_value);
        });

    if (!did_parse || builder.failed || !builder.root_selector_list.has_value())
        return {};

    VERIFY(builder.selector_list_stack.is_empty());
    VERIFY(builder.pending_pseudo_selectors.is_empty());
    VERIFY(!builder.invalid_selector_builder.has_value());
    return builder.root_selector_list.release_value();
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

Optional<RustComponentValueParser::PropertyKeyword> RustComponentValueParser::parse_property_keyword_value(ReadonlySpan<PropertyID> property_ids, StringView keyword)
{
    Vector<u16, 4> ffi_property_ids;
    for (auto property_id : property_ids)
        ffi_property_ids.append(static_cast<u16>(to_underlying(property_id)));

    Optional<PropertyKeyword> property_keyword;
    auto keyword_bytes = keyword.bytes();
    FFI::rust_css_parse_property_keyword_value(
        ffi_property_ids.data(),
        ffi_property_ids.size(),
        keyword_bytes.data(),
        keyword_bytes.size(),
        &property_keyword,
        [](void* raw_property_keyword, u16 property_id, u8 const* keyword_ptr, size_t keyword_len) {
            auto& property_keyword = *static_cast<Optional<PropertyKeyword>*>(raw_property_keyword);
            auto keyword = keyword_from_string({ keyword_ptr, keyword_len });
            if (!keyword.has_value())
                return;
            property_keyword = PropertyKeyword {
                .property_id = static_cast<PropertyID>(property_id),
                .keyword = keyword.release_value(),
            };
        });

    return property_keyword;
}

static DescriptorMetadata::ValueType descriptor_value_type_from_ffi(FFI::CssDescriptorValueType value_type)
{
    switch (value_type) {
    case FFI::CssDescriptorValueType::CounterStyleSystem:
        return DescriptorMetadata::ValueType::CounterStyleSystem;
    case FFI::CssDescriptorValueType::CounterStyleAdditiveSymbols:
        return DescriptorMetadata::ValueType::CounterStyleAdditiveSymbols;
    case FFI::CssDescriptorValueType::CounterStyleName:
        return DescriptorMetadata::ValueType::CounterStyleName;
    case FFI::CssDescriptorValueType::CounterStyleNegative:
        return DescriptorMetadata::ValueType::CounterStyleNegative;
    case FFI::CssDescriptorValueType::CounterStylePad:
        return DescriptorMetadata::ValueType::CounterStylePad;
    case FFI::CssDescriptorValueType::CounterStyleRange:
        return DescriptorMetadata::ValueType::CounterStyleRange;
    case FFI::CssDescriptorValueType::CropOrCross:
        return DescriptorMetadata::ValueType::CropOrCross;
    case FFI::CssDescriptorValueType::FamilyName:
        return DescriptorMetadata::ValueType::FamilyName;
    case FFI::CssDescriptorValueType::FontSrcList:
        return DescriptorMetadata::ValueType::FontSrcList;
    case FFI::CssDescriptorValueType::FontWeightAbsolutePair:
        return DescriptorMetadata::ValueType::FontWeightAbsolutePair;
    case FFI::CssDescriptorValueType::Length:
        return DescriptorMetadata::ValueType::Length;
    case FFI::CssDescriptorValueType::OptionalDeclarationValue:
        return DescriptorMetadata::ValueType::OptionalDeclarationValue;
    case FFI::CssDescriptorValueType::PageSize:
        return DescriptorMetadata::ValueType::PageSize;
    case FFI::CssDescriptorValueType::PositivePercentage:
        return DescriptorMetadata::ValueType::PositivePercentage;
    case FFI::CssDescriptorValueType::String:
        return DescriptorMetadata::ValueType::String;
    case FFI::CssDescriptorValueType::Symbol:
        return DescriptorMetadata::ValueType::Symbol;
    case FFI::CssDescriptorValueType::Symbols:
        return DescriptorMetadata::ValueType::Symbols;
    case FFI::CssDescriptorValueType::UnicodeRangeTokens:
        return DescriptorMetadata::ValueType::UnicodeRangeTokens;
    }
    VERIFY_NOT_REACHED();
}

bool RustComponentValueParser::at_rule_supports_descriptor(AtRuleID at_rule_id, DescriptorID descriptor_id)
{
    return FFI::rust_css_at_rule_supports_descriptor(
        static_cast<u8>(to_underlying(at_rule_id)),
        static_cast<u8>(to_underlying(descriptor_id)));
}

bool RustComponentValueParser::descriptor_allows_arbitrary_substitution_functions(AtRuleID at_rule_id, DescriptorID descriptor_id)
{
    return FFI::rust_css_descriptor_allows_arbitrary_substitution_functions(
        static_cast<u8>(to_underlying(at_rule_id)),
        static_cast<u8>(to_underlying(descriptor_id)));
}

static URL::Type url_function_type_from_rust(FFI::CssUrlFunctionType);
static CrossOriginModifierValue cross_origin_modifier_value_from_rust(FFI::CssUrlCrossOriginModifierValue);
static ReferrerPolicyModifierValue referrer_policy_modifier_value_from_rust(FFI::CssUrlReferrerPolicyModifierValue);
static FontTech font_tech_from_rust(FFI::CssFontTech);
static Gfx::UnicodeRange unicode_range_from_rust(FFI::CssUnicodeRange const&);

Optional<RustComponentValueParser::DescriptorValue> RustComponentValueParser::parse_descriptor(AtRuleID at_rule_id, DescriptorID descriptor_id, StringView input, StringView encoding)
{
    Optional<DescriptorValue> descriptor_value;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_descriptor(
        static_cast<u8>(to_underlying(at_rule_id)),
        static_cast<u8>(to_underlying(descriptor_id)),
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &descriptor_value,
        [](void* raw_descriptor_value, FFI::CssDescriptorSyntaxKind kind, u16 property_id, FFI::CssDescriptorValueType value_type, u8 const* value_ptr, size_t value_len) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            switch (kind) {
            case FFI::CssDescriptorSyntaxKind::Keyword: {
                auto keyword = keyword_from_string({ value_ptr, value_len });
                if (!keyword.has_value())
                    return;
                descriptor_value = DescriptorValue { .syntax = keyword.release_value() };
                return;
            }
            case FFI::CssDescriptorSyntaxKind::Property:
                descriptor_value = DescriptorValue { .syntax = static_cast<PropertyID>(property_id) };
                return;
            case FFI::CssDescriptorSyntaxKind::ValueType:
                descriptor_value = DescriptorValue { .syntax = descriptor_value_type_from_ffi(value_type) };
                return;
            }
        },
        [](void* raw_descriptor_value, FFI::CssDescriptorResultKind kind) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            descriptor_value->result = DescriptorResult { .kind = kind };
        },
        [](void* raw_descriptor_value, FFI::CssNonnegativeIntegerSymbolPairOrder order, u8 const* source_ptr, size_t source_len, bool is_string, FFI::CssPrimitiveValueKind primitive_kind, bool has_numeric_value, double numeric_value, u8 page_size_keyword, u8 page_size_orientation) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            VERIFY(descriptor_value->result.has_value());
            descriptor_value->result->items.append(DescriptorResultItem {
                .order = order,
                .source = string_from_ffi_bytes(source_ptr, source_len),
                .is_string = is_string,
                .primitive_kind = primitive_kind,
                .has_numeric_value = has_numeric_value,
                .numeric_value = numeric_value,
                .page_size_keyword = page_size_keyword,
                .page_size_orientation = page_size_orientation,
            });
        },
        [](void* raw_descriptor_value, FFI::CssCalculationNodeKind kind, FFI::CssPrimitiveValueKind primitive_kind, bool has_numeric_value, double numeric_value, u32 child_count, u8 const* metadata_ptr, size_t metadata_len) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            VERIFY(descriptor_value->result.has_value());
            VERIFY(!descriptor_value->result->items.is_empty());
            descriptor_value->result->items.last().calculation_node_events.append(RustCalculationNodeEvent {
                .kind = kind,
                .primitive_kind = primitive_kind,
                .numeric_value = has_numeric_value ? Optional<double> { numeric_value } : Optional<double> {},
                .child_count = child_count,
                .metadata = string_from_ffi_bytes(metadata_ptr, metadata_len),
            });
        },
        [](void* raw_descriptor_value, FFI::CssFontSourceKind kind, u8 const* family_name_ptr, size_t family_name_len, bool family_name_is_string) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            VERIFY(descriptor_value->result.has_value());
            VERIFY(!descriptor_value->result->items.is_empty());
            auto& item = descriptor_value->result->items.last();
            item.font_source_kind = kind;
            if (kind == FFI::CssFontSourceKind::Local) {
                item.font_source_family_name = FamilyName {
                    .name = fly_string_from_ffi_bytes(family_name_ptr, family_name_len),
                    .is_string = family_name_is_string,
                };
            }
        },
        [](void* raw_descriptor_value, FFI::CssUrlFunction const* rust_url_function) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            VERIFY(descriptor_value->result.has_value());
            VERIFY(!descriptor_value->result->items.is_empty());
            auto& item = descriptor_value->result->items.last();
            item.url_function_type = url_function_type_from_rust(rust_url_function->function_type);
            item.url = string_from_ffi_bytes(rust_url_function->url_ptr, rust_url_function->url_len);
        },
        [](void* raw_descriptor_value, FFI::CssUrlModifier const* rust_modifier) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            VERIFY(descriptor_value->result.has_value());
            VERIFY(!descriptor_value->result->items.is_empty());
            auto& item = descriptor_value->result->items.last();
            switch (rust_modifier->kind) {
            case FFI::CssUrlModifierKind::CrossOrigin:
                item.request_url_modifiers.append(RequestURLModifier::create_cross_origin(cross_origin_modifier_value_from_rust(rust_modifier->cross_origin_value)));
                break;
            case FFI::CssUrlModifierKind::Integrity:
                item.request_url_modifiers.append(RequestURLModifier::create_integrity(fly_string_from_ffi_bytes(rust_modifier->integrity_ptr, rust_modifier->integrity_len)));
                break;
            case FFI::CssUrlModifierKind::ReferrerPolicy:
                item.request_url_modifiers.append(RequestURLModifier::create_referrer_policy(referrer_policy_modifier_value_from_rust(rust_modifier->referrer_policy_value)));
                break;
            }
        },
        [](void* raw_descriptor_value, u8 const* format_ptr, size_t format_len) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            VERIFY(descriptor_value->result.has_value());
            VERIFY(!descriptor_value->result->items.is_empty());
            descriptor_value->result->items.last().font_source_format = fly_string_from_ffi_bytes(format_ptr, format_len);
        },
        [](void* raw_descriptor_value, FFI::CssFontTech rust_font_tech) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            VERIFY(descriptor_value->result.has_value());
            VERIFY(!descriptor_value->result->items.is_empty());
            descriptor_value->result->items.last().font_source_tech.append(font_tech_from_rust(rust_font_tech));
        },
        [](void* raw_descriptor_value, FFI::CssUnicodeRange const* rust_unicode_range) {
            auto& descriptor_value = *static_cast<Optional<DescriptorValue>*>(raw_descriptor_value);
            VERIFY(descriptor_value.has_value());
            VERIFY(descriptor_value->result.has_value());
            descriptor_value->result->items.append(DescriptorResultItem {
                .unicode_range = unicode_range_from_rust(*rust_unicode_range),
            });
        });

    if (!parsed || !descriptor_value.has_value())
        return {};

    return descriptor_value;
}

Optional<PropertyID> RustComponentValueParser::property_accepting_type(ReadonlySpan<PropertyID> property_ids, ValueType value_type)
{
    Vector<u16, 4> ffi_property_ids;
    for (auto property_id : property_ids)
        ffi_property_ids.append(static_cast<u16>(to_underlying(property_id)));

    Optional<PropertyID> accepted_property_id;
    auto value_type_string = value_type_to_string(value_type);
    auto value_type_bytes = value_type_string.bytes();
    FFI::rust_css_property_accepting_type(
        ffi_property_ids.data(),
        ffi_property_ids.size(),
        value_type_bytes.data(),
        value_type_bytes.size(),
        &accepted_property_id,
        [](void* raw_property_id, u16 property_id) {
            auto& accepted_property_id = *static_cast<Optional<PropertyID>*>(raw_property_id);
            accepted_property_id = static_cast<PropertyID>(property_id);
        });

    return accepted_property_id;
}

Optional<RustComponentValueParser::PropertyCustomIdent> RustComponentValueParser::parse_property_custom_ident_value(ReadonlySpan<PropertyID> property_ids, StringView input)
{
    Vector<u16, 4> ffi_property_ids;
    for (auto property_id : property_ids)
        ffi_property_ids.append(static_cast<u16>(to_underlying(property_id)));

    Optional<PropertyCustomIdent> property_custom_ident;
    auto input_bytes = input.bytes();
    FFI::rust_css_parse_property_custom_ident_value(
        ffi_property_ids.data(),
        ffi_property_ids.size(),
        input_bytes.data(),
        input_bytes.size(),
        &property_custom_ident,
        [](void* raw_property_custom_ident, u16 property_id, u8 const* custom_ident_ptr, size_t custom_ident_len) {
            auto& property_custom_ident = *static_cast<Optional<PropertyCustomIdent>*>(raw_property_custom_ident);
            property_custom_ident = PropertyCustomIdent {
                .property_id = static_cast<PropertyID>(property_id),
                .custom_ident = fly_string_from_ffi_bytes(custom_ident_ptr, custom_ident_len),
            };
        });

    return property_custom_ident;
}

static Optional<ValueType> value_type_from_rust_property_value_type_name(StringView name)
{
#define __TRY_VALUE_TYPE(value_type)                                                              \
    do {                                                                                          \
        if (name.equals_ignoring_ascii_case(StringView { #value_type, sizeof(#value_type) - 1 })) \
            return ValueType::value_type;                                                         \
    } while (false)

    __TRY_VALUE_TYPE(Anchor);
    __TRY_VALUE_TYPE(AnchorSize);
    __TRY_VALUE_TYPE(Angle);
    __TRY_VALUE_TYPE(AnglePercentage);
    __TRY_VALUE_TYPE(BackgroundPosition);
    __TRY_VALUE_TYPE(BasicShape);
    __TRY_VALUE_TYPE(Color);
    __TRY_VALUE_TYPE(CornerShape);
    __TRY_VALUE_TYPE(Counter);
    __TRY_VALUE_TYPE(CounterStyle);
    __TRY_VALUE_TYPE(CustomIdent);
    __TRY_VALUE_TYPE(DashedIdent);
    __TRY_VALUE_TYPE(EasingFunction);
    __TRY_VALUE_TYPE(FilterValueList);
    __TRY_VALUE_TYPE(FitContent);
    __TRY_VALUE_TYPE(Flex);
    __TRY_VALUE_TYPE(FontKerningValue);
    __TRY_VALUE_TYPE(FontOpticalSizingValue);
    __TRY_VALUE_TYPE(FontStyle);
    __TRY_VALUE_TYPE(FontVariantAlternates);
    __TRY_VALUE_TYPE(FontVariantCapsValue);
    __TRY_VALUE_TYPE(FontVariantEastAsian);
    __TRY_VALUE_TYPE(FontVariantEmojiValue);
    __TRY_VALUE_TYPE(FontVariantLigatures);
    __TRY_VALUE_TYPE(FontVariantNumeric);
    __TRY_VALUE_TYPE(FontVariantPositionValue);
    __TRY_VALUE_TYPE(FontWeightAbsolute);
    __TRY_VALUE_TYPE(FontWidthCss3);
    __TRY_VALUE_TYPE(Frequency);
    __TRY_VALUE_TYPE(FrequencyPercentage);
    __TRY_VALUE_TYPE(Image);
    __TRY_VALUE_TYPE(Integer);
    __TRY_VALUE_TYPE(Length);
    __TRY_VALUE_TYPE(LengthPercentage);
    __TRY_VALUE_TYPE(Number);
    __TRY_VALUE_TYPE(OpacityValue);
    __TRY_VALUE_TYPE(OpentypeTag);
    __TRY_VALUE_TYPE(Paint);
    __TRY_VALUE_TYPE(Percentage);
    __TRY_VALUE_TYPE(Position);
    __TRY_VALUE_TYPE(Ratio);
    __TRY_VALUE_TYPE(Rect);
    __TRY_VALUE_TYPE(Resolution);
    __TRY_VALUE_TYPE(ScrollFunction);
    __TRY_VALUE_TYPE(String);
    __TRY_VALUE_TYPE(Time);
    __TRY_VALUE_TYPE(TimePercentage);
    __TRY_VALUE_TYPE(TransformFunction);
    __TRY_VALUE_TYPE(TransformList);
    __TRY_VALUE_TYPE(Url);
    __TRY_VALUE_TYPE(ViewFunction);
    __TRY_VALUE_TYPE(ViewTimelineInset);

#undef __TRY_VALUE_TYPE

    return {};
}

Optional<RustComponentValueParser::RustStyleValue> RustComponentValueParser::parse_style_value_for_property(ReadonlySpan<PropertyID> property_ids, StringView input,
    bool allow_quirky_length, bool allow_quirky_color, bool allow_svg_unitless_length, bool allow_svg_unitless_angle)
{
    Vector<u16, 4> ffi_property_ids;
    for (auto property_id : property_ids)
        ffi_property_ids.append(static_cast<u16>(to_underlying(property_id)));

    struct StyleValueParseContext {
        enum class SourceComponentValueTarget : u8 {
            None,
            Discard,
            FlexBasis,
            Image,
            ImageSetResolution,
            NestedPrimitive,
            OpenTypeTagValue,
            SecondaryNestedPrimitive,
            ShorthandItem,
            StyleColor,
        };

        enum : u8 {
            SourceComponentValueListFlexBasis = 1,
            SourceComponentValueListStyleColor = 2,
            SourceComponentValueListImage = 3,
            SourceComponentValueListImageSetResolution = 4,
            SourceComponentValueListNestedPrimitive = 5,
            SourceComponentValueListShorthandItem = 6,
            SourceComponentValueListOpenTypeTagValue = 7,
            SourceComponentValueListSecondaryNestedPrimitive = 8,
            SourceComponentValueListGradientColorInterpolationMethod = 9,
            SourceComponentValueListGradientLinearAngle = 10,
            SourceComponentValueListGradientConicFromAngle = 11,
            SourceComponentValueListGradientConicPosition = 12,
            SourceComponentValueListGradientRadialPosition = 13,
            SourceComponentValueListGradientRadialSizeComponent = 14,
        };

        Optional<RustStyleValue> style_value;
        ComponentValueBuilder source_component_value_builder;
        SourceComponentValueTarget source_component_value_target { SourceComponentValueTarget::None };
        RustStyleColor* style_color_source_component_value_target { nullptr };
        Vector<ComponentValue>* source_component_values_target { nullptr };
        Vector<ComponentValue> pending_nested_primitive_source_component_values;
        Vector<ComponentValue> pending_secondary_nested_primitive_source_component_values;
        RustGradient* active_gradient { nullptr };

        void flush_source_component_values()
        {
            if (source_component_value_target == SourceComponentValueTarget::None)
                return;
            VERIFY(source_component_value_builder.stack.is_empty());
            switch (source_component_value_target) {
            case SourceComponentValueTarget::None:
                VERIFY_NOT_REACHED();
            case SourceComponentValueTarget::Discard:
                break;
            case SourceComponentValueTarget::FlexBasis:
                VERIFY(style_value.has_value());
                VERIFY(style_value->flex_basis_kind == RustFlexBasisKind::Source);
                style_value->flex_basis_source_component_values = move(source_component_value_builder.root_values);
                break;
            case SourceComponentValueTarget::Image:
                VERIFY(source_component_values_target);
                *source_component_values_target = move(source_component_value_builder.root_values);
                source_component_values_target = nullptr;
                break;
            case SourceComponentValueTarget::ImageSetResolution:
                VERIFY(style_value.has_value());
                if (style_value->kind == FFI::CssStyleValueKind::Content) {
                    VERIFY(!style_value->content_events.is_empty());
                    VERIFY(!style_value->content_events.last().image_set_options.is_empty());
                    style_value->content_events.last().image_set_options.last().resolution_component_values = move(source_component_value_builder.root_values);
                } else if (style_value->kind == FFI::CssStyleValueKind::Cursor) {
                    VERIFY(!style_value->cursor_images.is_empty());
                    VERIFY(style_value->cursor_images.last().image_kind == RustImageKind::ImageSet);
                    VERIFY(!style_value->cursor_images.last().image_set_options.is_empty());
                    style_value->cursor_images.last().image_set_options.last().resolution_component_values = move(source_component_value_builder.root_values);
                } else if (style_value->kind == FFI::CssStyleValueKind::ListStyle) {
                    VERIFY(!style_value->list_style_image_source_image_set_options.is_empty());
                    style_value->list_style_image_source_image_set_options.last().resolution_component_values = move(source_component_value_builder.root_values);
                } else if (style_value->kind == FFI::CssStyleValueKind::BorderImage) {
                    VERIFY(!style_value->border_image_source_source_image_set_options.is_empty());
                    style_value->border_image_source_source_image_set_options.last().resolution_component_values = move(source_component_value_builder.root_values);
                } else if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                    VERIFY(!style_value->shape_outside_image_source_image_set_options.is_empty());
                    style_value->shape_outside_image_source_image_set_options.last().resolution_component_values = move(source_component_value_builder.root_values);
                } else if (style_value->kind == FFI::CssStyleValueKind::LayerShorthand) {
                    VERIFY(!style_value->layer_shorthand_items.is_empty());
                    VERIFY(!style_value->layer_shorthand_items.last().image_set_options.is_empty());
                    style_value->layer_shorthand_items.last().image_set_options.last().resolution_component_values = move(source_component_value_builder.root_values);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Image);
                    VERIFY(style_value->image_kind == RustImageKind::ImageSet);
                    VERIFY(!style_value->image_set_options.is_empty());
                    style_value->image_set_options.last().resolution_component_values = move(source_component_value_builder.root_values);
                }
                break;
            case SourceComponentValueTarget::NestedPrimitive:
                pending_nested_primitive_source_component_values = move(source_component_value_builder.root_values);
                break;
            case SourceComponentValueTarget::OpenTypeTagValue:
                VERIFY(style_value.has_value());
                VERIFY(!style_value->open_type_tag_values.is_empty());
                style_value->open_type_tag_values.last().value_component_values = move(source_component_value_builder.root_values);
                break;
            case SourceComponentValueTarget::SecondaryNestedPrimitive:
                pending_secondary_nested_primitive_source_component_values = move(source_component_value_builder.root_values);
                break;
            case SourceComponentValueTarget::ShorthandItem:
                VERIFY(style_value.has_value());
                switch (style_value->kind) {
                case FFI::CssStyleValueKind::ComponentShorthand:
                    VERIFY(!style_value->component_shorthand_items.is_empty());
                    style_value->component_shorthand_items.last().value_component_values = move(source_component_value_builder.root_values);
                    break;
                case FFI::CssStyleValueKind::CoordinatingValueListShorthand:
                    VERIFY(!style_value->coordinating_value_list_shorthand_items.is_empty());
                    style_value->coordinating_value_list_shorthand_items.last().value_component_values = move(source_component_value_builder.root_values);
                    break;
                case FFI::CssStyleValueKind::FontShorthand:
                    VERIFY(!style_value->font_shorthand_items.is_empty());
                    style_value->font_shorthand_items.last().value_component_values = move(source_component_value_builder.root_values);
                    break;
                case FFI::CssStyleValueKind::LayerShorthand:
                    VERIFY(!style_value->layer_shorthand_items.is_empty());
                    style_value->layer_shorthand_items.last().value_component_values = move(source_component_value_builder.root_values);
                    break;
                case FFI::CssStyleValueKind::PositionalValueListShorthand:
                    VERIFY(!style_value->positional_value_list_shorthand_items.is_empty());
                    style_value->positional_value_list_shorthand_items.last().value_component_values = move(source_component_value_builder.root_values);
                    break;
                default:
                    VERIFY_NOT_REACHED();
                }
                break;
            case SourceComponentValueTarget::StyleColor:
                if (style_color_source_component_value_target) {
                    style_color_source_component_value_target->source_component_values = move(source_component_value_builder.root_values);
                    style_color_source_component_value_target = nullptr;
                } else {
                    VERIFY(source_component_values_target);
                    *source_component_values_target = move(source_component_value_builder.root_values);
                    source_component_values_target = nullptr;
                }
                break;
            }
            source_component_value_builder = {};
            source_component_value_target = SourceComponentValueTarget::None;
        }

        void start_source_component_values(u8 kind)
        {
            flush_source_component_values();
            switch (kind) {
            case SourceComponentValueListFlexBasis:
                VERIFY(style_value.has_value());
                VERIFY(style_value->kind == FFI::CssStyleValueKind::Flex);
                VERIFY(style_value->flex_basis_kind == RustFlexBasisKind::Source);
                source_component_value_target = SourceComponentValueTarget::FlexBasis;
                return;
            case SourceComponentValueListStyleColor:
                source_component_value_target = (style_color_source_component_value_target || source_component_values_target)
                    ? SourceComponentValueTarget::StyleColor
                    : SourceComponentValueTarget::Discard;
                return;
            case SourceComponentValueListImage:
                source_component_value_target = source_component_values_target
                    ? SourceComponentValueTarget::Image
                    : SourceComponentValueTarget::Discard;
                return;
            case SourceComponentValueListImageSetResolution:
                source_component_value_target = SourceComponentValueTarget::ImageSetResolution;
                return;
            case SourceComponentValueListNestedPrimitive:
                source_component_value_target = SourceComponentValueTarget::NestedPrimitive;
                return;
            case SourceComponentValueListSecondaryNestedPrimitive:
                source_component_value_target = SourceComponentValueTarget::SecondaryNestedPrimitive;
                return;
            case SourceComponentValueListShorthandItem:
                source_component_value_target = SourceComponentValueTarget::ShorthandItem;
                return;
            case SourceComponentValueListOpenTypeTagValue:
                source_component_value_target = SourceComponentValueTarget::OpenTypeTagValue;
                return;
            case SourceComponentValueListGradientColorInterpolationMethod:
                VERIFY(active_gradient);
                source_component_values_target = &active_gradient->color_interpolation_method_component_values;
                source_component_value_target = SourceComponentValueTarget::Image;
                return;
            case SourceComponentValueListGradientLinearAngle:
                VERIFY(active_gradient);
                source_component_values_target = &active_gradient->linear_angle_component_values;
                source_component_value_target = SourceComponentValueTarget::Image;
                return;
            case SourceComponentValueListGradientConicFromAngle:
                VERIFY(active_gradient);
                source_component_values_target = &active_gradient->conic_from_angle_component_values;
                source_component_value_target = SourceComponentValueTarget::Image;
                return;
            case SourceComponentValueListGradientConicPosition:
                VERIFY(active_gradient);
                source_component_values_target = &active_gradient->conic_position_component_values;
                source_component_value_target = SourceComponentValueTarget::Image;
                return;
            case SourceComponentValueListGradientRadialPosition:
                VERIFY(active_gradient);
                source_component_values_target = &active_gradient->radial_position_component_values;
                source_component_value_target = SourceComponentValueTarget::Image;
                return;
            case SourceComponentValueListGradientRadialSizeComponent:
                VERIFY(active_gradient);
                VERIFY(!active_gradient->radial_size_components.is_empty());
                source_component_values_target = &active_gradient->radial_size_components.last().length_percentage_component_values;
                source_component_value_target = SourceComponentValueTarget::Image;
                return;
            default:
                VERIFY_NOT_REACHED();
            }
        }
    };

    StyleValueParseContext context;
    auto input_bytes = input.bytes();
    FFI::rust_css_parse_style_value_for_property(
        ffi_property_ids.data(),
        ffi_property_ids.size(),
        input_bytes.data(),
        input_bytes.size(),
        allow_quirky_length,
        allow_quirky_color,
        allow_svg_unitless_length,
        allow_svg_unitless_angle,
        &context,
        [](void* raw_style_value, FFI::CssStyleValueKind kind, u16 property_id, FFI::CssPrimitiveValueKind primitive_kind, bool has_numeric_value, double numeric_value, bool has_secondary_numeric_value, double secondary_numeric_value, u8 color_red, u8 color_green, u8 color_blue, u8 color_alpha, u8 const* value_ptr, size_t value_len, u8 const* value_type_ptr, size_t value_type_len) {
            auto& context = *static_cast<StyleValueParseContext*>(raw_style_value);
            context.flush_source_component_values();
            auto& style_value = context.style_value;
            RustStyleValue value {
                .kind = kind,
                .property_id = static_cast<PropertyID>(property_id),
                .primitive_kind = primitive_kind,
                .color_red = color_red,
                .color_green = color_green,
                .color_blue = color_blue,
                .color_alpha = color_alpha,
            };
            auto nested_primitive_value_from_callback_payload = [&]() {
                RustNestedPrimitiveValue nested_value {
                    .primitive_kind = primitive_kind,
                    .source_or_unit = string_from_ffi_bytes(value_ptr, value_len),
                    .source_component_values = move(context.pending_nested_primitive_source_component_values),
                };
                if (has_numeric_value)
                    nested_value.numeric_value = numeric_value;
                return nested_value;
            };
            auto secondary_nested_primitive_value_from_callback_payload = [&]() {
                RustNestedPrimitiveValue nested_value {
                    .primitive_kind = static_cast<FFI::CssPrimitiveValueKind>(color_alpha),
                    .source_or_unit = value_type_len == 0 ? String {} : string_from_ffi_bytes(value_type_ptr, value_type_len),
                    .source_component_values = move(context.pending_secondary_nested_primitive_source_component_values),
                };
                if (has_secondary_numeric_value)
                    nested_value.numeric_value = secondary_numeric_value;
                return nested_value;
            };
            auto note_style_color_source_component_target = [&](Optional<RustStyleColor>& color) {
                if (color.has_value() && !color->is_simple)
                    context.style_color_source_component_value_target = &*color;
            };
            auto note_source_component_values_target = [&](Vector<ComponentValue>& component_values) {
                context.source_component_values_target = &component_values;
            };
            auto note_active_gradient = [&](RustGradient& gradient) {
                context.active_gradient = &gradient;
            };
            auto active_gradient_target = [&]() -> RustGradient* {
                if (!style_value.has_value())
                    return nullptr;

                if (style_value->kind == FFI::CssStyleValueKind::Image) {
                    if (style_value->image_kind == RustImageKind::ImageSet) {
                        if (style_value->image_set_options.is_empty())
                            return nullptr;
                        return style_value->image_set_options.last().gradient.ptr();
                    }
                    return style_value->image_gradient.ptr();
                }

                if (style_value->kind == FFI::CssStyleValueKind::Content) {
                    if (style_value->content_events.is_empty())
                        return nullptr;
                    if (style_value->content_events.last().image_kind == RustImageKind::ImageSet) {
                        if (style_value->content_events.last().image_set_options.is_empty())
                            return nullptr;
                        return style_value->content_events.last().image_set_options.last().gradient.ptr();
                    }
                    return style_value->content_events.last().gradient.ptr();
                }

                if (style_value->kind == FFI::CssStyleValueKind::Cursor) {
                    if (style_value->cursor_images.is_empty())
                        return nullptr;
                    if (style_value->cursor_images.last().image_kind == RustImageKind::ImageSet) {
                        if (style_value->cursor_images.last().image_set_options.is_empty())
                            return nullptr;
                        return style_value->cursor_images.last().image_set_options.last().gradient.ptr();
                    }
                    return style_value->cursor_images.last().gradient.ptr();
                }

                if (style_value->kind == FFI::CssStyleValueKind::ListStyle) {
                    if (style_value->list_style_image_source_kind == RustImageKind::ImageSet) {
                        if (style_value->list_style_image_source_image_set_options.is_empty())
                            return nullptr;
                        return style_value->list_style_image_source_image_set_options.last().gradient.ptr();
                    }
                    return style_value->list_style_image_gradient.ptr();
                }

                if (style_value->kind == FFI::CssStyleValueKind::BorderImage) {
                    if (style_value->border_image_source_source_kind == RustImageKind::ImageSet) {
                        if (style_value->border_image_source_source_image_set_options.is_empty())
                            return nullptr;
                        return style_value->border_image_source_source_image_set_options.last().gradient.ptr();
                    }
                    return style_value->border_image_source_gradient.ptr();
                }

                if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                    if (style_value->shape_outside_image_source_kind == RustImageKind::ImageSet) {
                        if (style_value->shape_outside_image_source_image_set_options.is_empty())
                            return nullptr;
                        return style_value->shape_outside_image_source_image_set_options.last().gradient.ptr();
                    }
                    return style_value->shape_outside_image_gradient.ptr();
                }

                if (style_value->kind == FFI::CssStyleValueKind::LayerShorthand) {
                    if (style_value->layer_shorthand_items.is_empty())
                        return nullptr;
                    if (style_value->layer_shorthand_items.last().image_kind == RustImageKind::ImageSet) {
                        if (style_value->layer_shorthand_items.last().image_set_options.is_empty())
                            return nullptr;
                        return style_value->layer_shorthand_items.last().image_set_options.last().gradient.ptr();
                    }
                    return style_value->layer_shorthand_items.last().gradient.ptr();
                }

                return nullptr;
            };
            auto image_url_from_callback_payload = [&]() -> Optional<URL> {
                enum : u8 {
                    NoURL,
                    URLFunction,
                    SrcFunction,
                };

                URL::Type url_type;
                switch (color_alpha) {
                case NoURL:
                    return {};
                case URLFunction:
                    url_type = URL::Type::Url;
                    break;
                case SrcFunction:
                    url_type = URL::Type::Src;
                    break;
                default:
                    VERIFY_NOT_REACHED();
                }

                return URL { string_from_ffi_bytes(value_ptr, value_len), url_type, {} };
            };
            auto image_set_option_metadata_from_callback_payload = [&](u8 image_kind) {
                Optional<String> resolution;
                Optional<String> type;
                if (value_type_len > 0) {
                    size_t metadata_index = 0;
                    for (auto metadata : StringView { value_type_ptr, value_type_len }.split_view('\0', SplitBehavior::KeepEmpty)) {
                        if (metadata_index == 0 && !metadata.is_empty())
                            resolution = String::from_utf8_without_validation(metadata.bytes());
                        else if (metadata_index == 1 && !metadata.is_empty())
                            type = String::from_utf8_without_validation(metadata.bytes());
                        ++metadata_index;
                    }
                }
                auto option = RustImageSetOption {
                    .image_kind = static_cast<RustImageKind>(image_kind),
                    .image_source = string_from_ffi_bytes(value_ptr, value_len),
                    .image_url = image_url_from_callback_payload(),
                    .resolution = move(resolution),
                    .type = move(type),
                };
                if (option.image_kind == RustImageKind::Gradient)
                    option.gradient = RustGradient {};
                return option;
            };
            auto shorthand_property_id_from_callback_payload = [&]() {
                return static_cast<PropertyID>(static_cast<u16>(color_red) | (static_cast<u16>(color_green) << 8));
            };
            auto layer_index_from_callback_payload = [&]() {
                return static_cast<size_t>(static_cast<u16>(color_blue) | (static_cast<u16>(color_alpha) << 8));
            };

            if (kind == FFI::CssStyleValueKind::Gradient) {
                auto event_kind = static_cast<RustGradientEventKind>(color_red);
                if (event_kind == RustGradientEventKind::Header) {
                    auto* gradient = active_gradient_target();
                    if (!gradient)
                        return;

                    gradient->kind = static_cast<RustGradientKind>(color_green);
                    gradient->is_repeating = color_blue != 0;
                    gradient->is_webkit_prefixed = color_alpha != 0;
                    gradient->color_stop_group_index = has_numeric_value ? static_cast<size_t>(numeric_value) : 0;
                    note_active_gradient(*gradient);
                    return;
                }

                auto* gradient = context.active_gradient;
                if (!gradient)
                    return;

                if (event_kind == RustGradientEventKind::ColorInterpolationMethod) {
                    return;
                }

                if (event_kind == RustGradientEventKind::LinearDirection) {
                    gradient->linear_direction_kind = static_cast<RustLinearGradientDirectionKind>(color_green);
                    if (*gradient->linear_direction_kind == RustLinearGradientDirectionKind::SideOrCorner)
                        gradient->linear_side_or_corner = static_cast<RustGradientSideOrCorner>(color_blue);
                    return;
                }

                if (event_kind == RustGradientEventKind::ConicFromAngle || event_kind == RustGradientEventKind::ConicPosition || event_kind == RustGradientEventKind::RadialPosition)
                    return;

                if (event_kind == RustGradientEventKind::RadialShape) {
                    gradient->radial_shape = static_cast<RustRadialGradientShape>(color_green);
                    return;
                }

                if (event_kind == RustGradientEventKind::RadialSizeComponent) {
                    auto component_kind = static_cast<RustGradientRadialSizeComponentKind>(color_green);
                    RustGradientRadialSizeComponent component {
                        .kind = component_kind,
                    };
                    if (component_kind == RustGradientRadialSizeComponentKind::Extent)
                        component.extent = static_cast<RustBasicShapeRadialExtent>(color_blue);
                    gradient->radial_size_components.append(move(component));
                    return;
                }

                VERIFY_NOT_REACHED();
            }

            if (style_value.has_value() && style_value->kind == FFI::CssStyleValueKind::ComponentShorthand && kind != FFI::CssStyleValueKind::ComponentShorthand) {
                VERIFY(!style_value->component_shorthand_items.is_empty());
                auto& item = style_value->component_shorthand_items.last();
                VERIFY(item.property_id == static_cast<PropertyID>(property_id));

                if (kind == FFI::CssStyleValueKind::Keyword) {
                    auto keyword = keyword_from_string({ value_ptr, value_len });
                    if (!keyword.has_value())
                        return;
                    item.keyword = keyword.release_value();
                    return;
                }

                if (kind == FFI::CssStyleValueKind::Color) {
                    item.has_color = true;
                    item.color_is_simple = true;
                    item.color_red = color_red;
                    item.color_green = color_green;
                    item.color_blue = color_blue;
                    item.color_alpha = color_alpha;
                    auto color_name = string_from_ffi_bytes(value_ptr, value_len);
                    if (!color_name.is_empty())
                        item.color_name_or_source = move(color_name);
                    return;
                }

                if (kind == FFI::CssStyleValueKind::ColorFunction) {
                    item.has_color = true;
                    item.color_name_or_source = string_from_ffi_bytes(value_ptr, value_len);
                    note_source_component_values_target(item.color_source_component_values);
                    return;
                }

                if (first_is_one_of(kind, FFI::CssStyleValueKind::Primitive, FFI::CssStyleValueKind::MathFunction)) {
                    auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                    if (!value_type.has_value())
                        return;
                    item.primitive_kind = primitive_kind;
                    if (has_numeric_value)
                        item.primitive_numeric_value = numeric_value;
                    item.primitive_source_or_unit = string_from_ffi_bytes(value_ptr, value_len);
                    item.primitive_source_component_values = move(context.pending_nested_primitive_source_component_values);
                    item.primitive_value_type = value_type.release_value();
                    return;
                }
            }

            if (style_value.has_value() && style_value->kind == FFI::CssStyleValueKind::PositionalValueListShorthand && kind != FFI::CssStyleValueKind::PositionalValueListShorthand) {
                VERIFY(!style_value->positional_value_list_shorthand_items.is_empty());
                auto& item = style_value->positional_value_list_shorthand_items.last();
                VERIFY(item.property_id == static_cast<PropertyID>(property_id));

                if (kind == FFI::CssStyleValueKind::Keyword) {
                    auto keyword = keyword_from_string({ value_ptr, value_len });
                    if (!keyword.has_value())
                        return;
                    item.keyword = keyword.release_value();
                    return;
                }

                if (first_is_one_of(kind, FFI::CssStyleValueKind::Primitive, FFI::CssStyleValueKind::MathFunction)) {
                    auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                    if (!value_type.has_value())
                        return;
                    item.primitive_kind = primitive_kind;
                    if (has_numeric_value)
                        item.primitive_numeric_value = numeric_value;
                    item.primitive_source_or_unit = string_from_ffi_bytes(value_ptr, value_len);
                    item.primitive_source_component_values = move(context.pending_nested_primitive_source_component_values);
                    item.primitive_value_type = value_type.release_value();
                    return;
                }

                if (kind == FFI::CssStyleValueKind::CornerShape) {
                    if (primitive_kind == FFI::CssPrimitiveValueKind::Keyword) {
                        auto keyword = keyword_from_string({ value_ptr, value_len });
                        if (!keyword.has_value())
                            return;
                        item.corner_shape_keyword = keyword.release_value();
                    } else {
                        item.has_corner_shape_superellipse_parameter = true;
                        item.primitive_kind = primitive_kind;
                        if (has_numeric_value)
                            item.primitive_numeric_value = numeric_value;
                        item.primitive_source_or_unit = string_from_ffi_bytes(value_ptr, value_len);
                        item.primitive_source_component_values = move(context.pending_nested_primitive_source_component_values);
                    }
                    return;
                }

                return;
            }

            if (style_value.has_value() && style_value->kind == FFI::CssStyleValueKind::CoordinatingValueListShorthand && kind != FFI::CssStyleValueKind::CoordinatingValueListShorthand) {
                VERIFY(!style_value->coordinating_value_list_shorthand_items.is_empty());
                auto& item = style_value->coordinating_value_list_shorthand_items.last();
                VERIFY(item.property_id == static_cast<PropertyID>(property_id));

                if (kind == FFI::CssStyleValueKind::Keyword) {
                    auto keyword = keyword_from_string({ value_ptr, value_len });
                    if (!keyword.has_value())
                        return;
                    item.keyword = keyword.release_value();
                    return;
                }

                if (kind == FFI::CssStyleValueKind::CustomIdent) {
                    item.custom_ident = fly_string_from_ffi_bytes(value_ptr, value_len);
                    return;
                }

                if (first_is_one_of(kind, FFI::CssStyleValueKind::Primitive, FFI::CssStyleValueKind::MathFunction)) {
                    auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                    if (!value_type.has_value())
                        return;
                    item.primitive_kind = primitive_kind;
                    if (has_numeric_value)
                        item.primitive_numeric_value = numeric_value;
                    item.primitive_source_or_unit = string_from_ffi_bytes(value_ptr, value_len);
                    item.primitive_value_type = value_type.release_value();
                    return;
                }

                if (kind == FFI::CssStyleValueKind::EasingFunction) {
                    item.easing_function_kind = color_red;
                    item.last_calculation_node_target = RustCalculationNodeTarget::None;
                    enum : u8 {
                        Keyword,
                        Linear,
                        CubicBezier,
                        Steps,
                    };
                    enum : u8 {
                        LinearOutput,
                        LinearFirstStopLength,
                        LinearSecondStopLength,
                    };
                    if (color_red == Keyword) {
                        auto keyword = StringView { value_ptr, value_len };
                        if (keyword.equals_ignoring_ascii_case("step-start"sv))
                            item.easing_function_step_position = StepPosition::Start;
                        else if (keyword.equals_ignoring_ascii_case("step-end"sv))
                            item.easing_function_step_position = StepPosition::End;
                        else
                            return;
                    } else if (color_red == Linear) {
                        if (color_green == LinearOutput) {
                            item.linear_easing_stops.append({
                                .output = nested_primitive_value_from_callback_payload(),
                            });
                            item.last_calculation_node_target = RustCalculationNodeTarget::LinearEasingOutput;
                        } else {
                            VERIFY(!item.linear_easing_stops.is_empty());
                            if (color_green == LinearFirstStopLength)
                                item.linear_easing_stops.last().first_stop_length = nested_primitive_value_from_callback_payload();
                            else {
                                VERIFY(color_green == LinearSecondStopLength);
                                item.linear_easing_stops.last().second_stop_length = nested_primitive_value_from_callback_payload();
                            }
                            item.last_calculation_node_target = color_green == LinearFirstStopLength
                                ? RustCalculationNodeTarget::LinearEasingFirstStopLength
                                : RustCalculationNodeTarget::LinearEasingSecondStopLength;
                        }
                    } else if (color_red == CubicBezier) {
                        item.easing_function_values.append(nested_primitive_value_from_callback_payload());
                        item.last_calculation_node_target = RustCalculationNodeTarget::EasingFunctionValue;
                    } else {
                        VERIFY(color_red == Steps);
                        item.easing_function_step_position = static_cast<StepPosition>(color_green);
                        item.easing_function_values.append(nested_primitive_value_from_callback_payload());
                        item.last_calculation_node_target = RustCalculationNodeTarget::EasingFunctionValue;
                    }
                    return;
                }

                if (kind == FFI::CssStyleValueKind::AnimationName) {
                    item.animation_name_kind = static_cast<FFI::CssAnimationNameValueKind>(color_red);
                    for (size_t offset = 0; offset < value_len;) {
                        auto item_kind = static_cast<FFI::CssAnimationNameItemKind>(value_ptr[offset++]);
                        auto name_start = offset;
                        while (offset < value_len && value_ptr[offset] != 0)
                            ++offset;
                        item.animation_name_item_kinds.append(item_kind);
                        item.animation_names.append(FlyString::from_utf8_without_validation({ value_ptr + name_start, offset - name_start }));
                        if (offset < value_len)
                            ++offset;
                    }
                    return;
                }

                if (kind == FFI::CssStyleValueKind::TransitionBehavior) {
                    item.transition_behaviors.ensure_capacity(value_len);
                    for (size_t i = 0; i < value_len; ++i)
                        item.transition_behaviors.unchecked_append(static_cast<FFI::CssTransitionBehaviorItemKind>(value_ptr[i]));
                    return;
                }

                if (kind == FFI::CssStyleValueKind::TransitionProperty) {
                    item.transition_property_kind = static_cast<FFI::CssTransitionPropertyValueKind>(color_red);
                    for (auto property : StringView { value_ptr, value_len }.split_view('\0'))
                        item.transition_properties.append(FlyString::from_utf8_without_validation(property.bytes()));
                    return;
                }

                return;
            }

            if (style_value.has_value() && style_value->kind == FFI::CssStyleValueKind::LayerShorthand && kind != FFI::CssStyleValueKind::LayerShorthand) {
                VERIFY(!style_value->layer_shorthand_items.is_empty());
                auto& item = style_value->layer_shorthand_items.last();
                VERIFY(item.property_id == static_cast<PropertyID>(property_id));
                item.last_calculation_node_target = RustCalculationNodeTarget::None;

                if (kind == FFI::CssStyleValueKind::Keyword) {
                    auto keyword = keyword_from_string({ value_ptr, value_len });
                    if (!keyword.has_value())
                        return;
                    item.keyword = keyword.release_value();
                    return;
                }

                if (kind == FFI::CssStyleValueKind::Color) {
                    item.has_color = true;
                    item.color_is_simple = true;
                    item.color_red = color_red;
                    item.color_green = color_green;
                    item.color_blue = color_blue;
                    item.color_alpha = color_alpha;
                    auto color_name = string_from_ffi_bytes(value_ptr, value_len);
                    if (!color_name.is_empty())
                        item.color_name_or_source = move(color_name);
                    return;
                }

                if (kind == FFI::CssStyleValueKind::ColorFunction) {
                    item.has_color = true;
                    item.color_name_or_source = string_from_ffi_bytes(value_ptr, value_len);
                    note_source_component_values_target(item.color_source_component_values);
                    return;
                }

                if (kind == FFI::CssStyleValueKind::Image) {
                    if (static_cast<RustImageKind>(color_red) == RustImageKind::ImageSet) {
                        item.image_kind = RustImageKind::ImageSet;
                        item.image_set_options.append(image_set_option_metadata_from_callback_payload(color_green));
                        note_source_component_values_target(item.image_set_options.last().image_source_component_values);
                    } else {
                        item.image_kind = static_cast<RustImageKind>(color_red);
                        item.image_source = string_from_ffi_bytes(value_ptr, value_len);
                        item.image_url = image_url_from_callback_payload();
                        if (*item.image_kind == RustImageKind::Gradient)
                            item.gradient = RustGradient {};
                        note_source_component_values_target(item.image_source_component_values);
                    }
                    return;
                }

                if (kind == FFI::CssStyleValueKind::RepeatStyle) {
                    item.repeat_x_values.append(color_red);
                    item.repeat_y_values.append(color_green);
                    return;
                }

                if (kind == FFI::CssStyleValueKind::BackgroundSize) {
                    enum : u8 {
                        Keyword,
                        Width,
                        Height,
                    };
                    if (color_red == Keyword) {
                        auto keyword = keyword_from_string({ value_ptr, value_len });
                        if (!keyword.has_value())
                            return;
                        item.background_sizes.append({
                            .keyword = keyword.release_value(),
                        });
                    } else if (color_red == Width) {
                        item.background_sizes.append({
                            .width = nested_primitive_value_from_callback_payload(),
                        });
                        item.last_calculation_node_target = RustCalculationNodeTarget::BackgroundSizeWidth;
                    } else {
                        VERIFY(color_red == Height);
                        VERIFY(!item.background_sizes.is_empty());
                        VERIFY(item.background_sizes.last().width.has_value());
                        VERIFY(!item.background_sizes.last().keyword.has_value());
                        item.background_sizes.last().height = nested_primitive_value_from_callback_payload();
                        item.last_calculation_node_target = RustCalculationNodeTarget::BackgroundSizeHeight;
                    }
                    return;
                }

                if (kind == FFI::CssStyleValueKind::Position) {
                    enum : u8 {
                        Header,
                        BeginPosition,
                        PositionX,
                        PositionY,
                        LonghandComponent,
                    };

                    auto position_component_from_callback_payload = [&]() {
                        RustPositionComponent component {
                            .edge = static_cast<RustPositionEdge>(color_green),
                        };
                        if (color_blue != 0)
                            component.offset = nested_primitive_value_from_callback_payload();
                        return component;
                    };

                    if (color_red == Header) {
                        auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                        if (!value_type.has_value())
                            return;
                        item.position_value_type = value_type.release_value();
                    } else if (color_red == BeginPosition) {
                        item.positions.append({});
                    } else if (color_red == PositionX) {
                        VERIFY(!item.positions.is_empty());
                        item.positions.last().x = position_component_from_callback_payload();
                        if (item.positions.last().x.offset.has_value())
                            item.last_calculation_node_target = RustCalculationNodeTarget::PositionXOffset;
                    } else if (color_red == PositionY) {
                        VERIFY(!item.positions.is_empty());
                        item.positions.last().y = position_component_from_callback_payload();
                        if (item.positions.last().y.offset.has_value())
                            item.last_calculation_node_target = RustCalculationNodeTarget::PositionYOffset;
                    } else {
                        VERIFY(color_red == LonghandComponent);
                    }
                    return;
                }

                return;
            }

            if (kind == FFI::CssStyleValueKind::Keyword) {
                auto keyword = keyword_from_string({ value_ptr, value_len });
                if (!keyword.has_value())
                    return;
                value.keyword = keyword.release_value();
            } else if (kind == FFI::CssStyleValueKind::CustomIdent) {
                value.custom_ident = fly_string_from_ffi_bytes(value_ptr, value_len);
            } else if (kind == FFI::CssStyleValueKind::Image) {
                value.image_kind = static_cast<RustImageKind>(color_red);
                if (value.image_kind == RustImageKind::ImageSet) {
                    if (!style_value.has_value()) {
                        style_value = move(value);
                    } else {
                        VERIFY(style_value->kind == FFI::CssStyleValueKind::Image);
                        VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                        VERIFY(style_value->image_kind == RustImageKind::ImageSet);
                    }

                    style_value->image_set_options.append(image_set_option_metadata_from_callback_payload(color_green));
                    note_source_component_values_target(style_value->image_set_options.last().image_source_component_values);
                    return;
                }
                value.image_source = string_from_ffi_bytes(value_ptr, value_len);
                value.image_url = image_url_from_callback_payload();
                if (value.image_kind == RustImageKind::Gradient)
                    value.image_gradient = RustGradient {};
                style_value = move(value);
                note_source_component_values_target(style_value->image_source_component_values);
                return;
            } else if (kind == FFI::CssStyleValueKind::Color) {
                if (value_len > 0)
                    value.string = fly_string_from_ffi_bytes(value_ptr, value_len);
            } else if (kind == FFI::CssStyleValueKind::Url) {
                value.string = fly_string_from_ffi_bytes(value_ptr, value_len);
                value.url = image_url_from_callback_payload();
            } else if (kind == FFI::CssStyleValueKind::CounterStyleName) {
                value.string = fly_string_from_ffi_bytes(value_ptr, value_len);
            } else if (kind == FFI::CssStyleValueKind::ColorFunction) {
                auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                if (!value_type.has_value())
                    return;
                value.value_type = value_type.release_value();
                if (value_len > 0)
                    value.string = fly_string_from_ffi_bytes(value_ptr, value_len);
                style_value = move(value);
                note_source_component_values_target(style_value->source_component_values);
                return;
            } else if (kind == FFI::CssStyleValueKind::EasingFunction) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::EasingFunction);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->easing_function_kind = color_red;
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;
                enum : u8 {
                    Keyword,
                    Linear,
                    CubicBezier,
                    Steps,
                };
                enum : u8 {
                    LinearOutput,
                    LinearFirstStopLength,
                    LinearSecondStopLength,
                };
                if (color_red == Keyword) {
                    auto keyword = StringView { value_ptr, value_len };
                    if (keyword.equals_ignoring_ascii_case("step-start"sv))
                        style_value->easing_function_step_position = StepPosition::Start;
                    else if (keyword.equals_ignoring_ascii_case("step-end"sv))
                        style_value->easing_function_step_position = StepPosition::End;
                    else
                        return;
                } else if (color_red == Linear) {
                    if (color_green == LinearOutput) {
                        style_value->linear_easing_stops.append({
                            .output = nested_primitive_value_from_callback_payload(),
                        });
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::LinearEasingOutput;
                    } else {
                        VERIFY(!style_value->linear_easing_stops.is_empty());
                        if (color_green == LinearFirstStopLength)
                            style_value->linear_easing_stops.last().first_stop_length = nested_primitive_value_from_callback_payload();
                        else {
                            VERIFY(color_green == LinearSecondStopLength);
                            style_value->linear_easing_stops.last().second_stop_length = nested_primitive_value_from_callback_payload();
                        }
                        style_value->last_calculation_node_target = color_green == LinearFirstStopLength
                            ? RustCalculationNodeTarget::LinearEasingFirstStopLength
                            : RustCalculationNodeTarget::LinearEasingSecondStopLength;
                    }
                } else if (color_red == CubicBezier) {
                    style_value->easing_function_values.append(nested_primitive_value_from_callback_payload());
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::EasingFunctionValue;
                } else {
                    VERIFY(color_red == Steps);
                    style_value->easing_function_step_position = static_cast<StepPosition>(color_green);
                    style_value->easing_function_values.append(nested_primitive_value_from_callback_payload());
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::EasingFunctionValue;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::BasicShape) {
                value.basic_shape_kind = static_cast<RustBasicShapeKind>(color_red);
                enum : u8 {
                    BasicShapeComponentHeader,
                    BasicShapeComponentPolygonPointX,
                    BasicShapeComponentPolygonPointY,
                    BasicShapeComponentRectangleLengthPercentage,
                    BasicShapeComponentRectangleAuto,
                    BasicShapeComponentRectangleBorderRadiusHorizontal,
                    BasicShapeComponentRectangleBorderRadiusVertical,
                    BasicShapeComponentRadialExtent,
                    BasicShapeComponentRadialLengthPercentage,
                    BasicShapeComponentRadialPositionX,
                    BasicShapeComponentRadialPositionY,
                };
                auto radial_position_component_from_callback_payload = [&]() {
                    RustPositionComponent component {
                        .edge = static_cast<RustPositionEdge>(color_green),
                    };
                    if (color_alpha != 0)
                        component.offset = nested_primitive_value_from_callback_payload();
                    return component;
                };
                if (value.basic_shape_kind == RustBasicShapeKind::Circle || value.basic_shape_kind == RustBasicShapeKind::Ellipse) {
                    if (!style_value.has_value())
                        style_value = move(value);
                    else {
                        VERIFY(style_value->kind == FFI::CssStyleValueKind::BasicShape);
                        VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                    }
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                    style_value->basic_shape_kind = static_cast<RustBasicShapeKind>(color_red);
                    style_value->basic_shape_radial_shape_is_typed = true;
                    if (color_blue == BasicShapeComponentRadialExtent) {
                        style_value->basic_shape_radial_shape_radius.append({
                            .is_radial_extent = true,
                            .radial_extent = static_cast<RustBasicShapeRadialExtent>(color_alpha),
                        });
                    } else if (color_blue == BasicShapeComponentRadialLengthPercentage) {
                        style_value->basic_shape_radial_shape_radius.append({ .length_percentage = nested_primitive_value_from_callback_payload() });
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapeRadius;
                    } else if (color_blue == BasicShapeComponentRadialPositionX) {
                        if (!style_value->basic_shape_radial_shape_position.has_value())
                            style_value->basic_shape_radial_shape_position = RustPosition {};
                        style_value->basic_shape_radial_shape_position->x = radial_position_component_from_callback_payload();
                        if (style_value->basic_shape_radial_shape_position->x.offset.has_value())
                            style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapePositionX;
                    } else if (color_blue == BasicShapeComponentRadialPositionY) {
                        if (!style_value->basic_shape_radial_shape_position.has_value())
                            style_value->basic_shape_radial_shape_position = RustPosition {};
                        style_value->basic_shape_radial_shape_position->y = radial_position_component_from_callback_payload();
                        if (style_value->basic_shape_radial_shape_position->y.offset.has_value())
                            style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapePositionY;
                    } else {
                        VERIFY(color_blue == BasicShapeComponentHeader);
                    }
                    return;
                }
                if (value.basic_shape_kind == RustBasicShapeKind::Inset || value.basic_shape_kind == RustBasicShapeKind::Xywh || value.basic_shape_kind == RustBasicShapeKind::Rect) {
                    if (!style_value.has_value())
                        style_value = move(value);
                    else {
                        VERIFY(style_value->kind == FFI::CssStyleValueKind::BasicShape);
                        VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                    }
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                    style_value->basic_shape_kind = static_cast<RustBasicShapeKind>(color_red);
                    if (color_blue == BasicShapeComponentRectangleLengthPercentage) {
                        style_value->basic_shape_rectangle_components.append({ .value = nested_primitive_value_from_callback_payload() });
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapeRectangleComponent;
                    } else if (color_blue == BasicShapeComponentRectangleAuto) {
                        style_value->basic_shape_rectangle_components.append({ .is_auto = true });
                    } else if (color_blue == BasicShapeComponentRectangleBorderRadiusHorizontal) {
                        style_value->basic_shape_rectangle_border_radius_horizontal_radii.append(nested_primitive_value_from_callback_payload());
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapeRectangleRadiusHorizontal;
                    } else if (color_blue == BasicShapeComponentRectangleBorderRadiusVertical) {
                        style_value->basic_shape_rectangle_border_radius_vertical_radii.append(nested_primitive_value_from_callback_payload());
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapeRectangleRadiusVertical;
                    } else {
                        VERIFY(color_blue == BasicShapeComponentHeader);
                    }
                    return;
                }
                if (value.basic_shape_kind == RustBasicShapeKind::Polygon) {
                    if (!style_value.has_value())
                        style_value = move(value);
                    else {
                        VERIFY(style_value->kind == FFI::CssStyleValueKind::BasicShape);
                        VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                        VERIFY(style_value->basic_shape_kind == RustBasicShapeKind::Polygon);
                    }
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                    style_value->basic_shape_fill_rule = color_green;
                    if (color_blue == BasicShapeComponentPolygonPointX || color_blue == BasicShapeComponentPolygonPointY) {
                        style_value->basic_shape_polygon_coordinates.append(nested_primitive_value_from_callback_payload());
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapePolygonCoordinate;
                    } else {
                        VERIFY(color_blue == BasicShapeComponentHeader);
                    }
                    return;
                }
                if (value.basic_shape_kind == RustBasicShapeKind::Path) {
                    value.basic_shape_fill_rule = color_green;
                    value.basic_shape_path_data = string_from_ffi_bytes(value_ptr, value_len);
                }
            } else if (kind == FFI::CssStyleValueKind::FitContent) {
                value.fit_content_kind = static_cast<RustFitContentKind>(color_red);
                if (value.fit_content_kind == RustFitContentKind::Function) {
                    value.fit_content_argument = nested_primitive_value_from_callback_payload();
                    value.last_calculation_node_target = RustCalculationNodeTarget::FitContentArgument;
                }
            } else if (kind == FFI::CssStyleValueKind::Rect) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Rect);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->rect_requires_commas = color_red != 0;
                style_value->rect_sides.append(nested_primitive_value_from_callback_payload());
                return;
            } else if (kind == FFI::CssStyleValueKind::AnchorNameOrScope) {
                value.anchor_name_or_scope_kind = static_cast<FFI::CssAnchorNameOrScopeValueKind>(color_red);
                for (auto name : StringView { value_ptr, value_len }.split_view('\0'))
                    value.anchor_names.append(FlyString::from_utf8_without_validation(name.bytes()));
            } else if (kind == FFI::CssStyleValueKind::AnimationName) {
                value.animation_name_kind = static_cast<FFI::CssAnimationNameValueKind>(color_red);
                for (size_t offset = 0; offset < value_len;) {
                    auto item_kind = static_cast<FFI::CssAnimationNameItemKind>(value_ptr[offset++]);
                    auto name_start = offset;
                    while (offset < value_len && value_ptr[offset] != 0)
                        ++offset;
                    value.animation_name_item_kinds.append(item_kind);
                    value.animation_names.append(FlyString::from_utf8_without_validation({ value_ptr + name_start, offset - name_start }));
                    if (offset < value_len)
                        ++offset;
                }
            } else if (kind == FFI::CssStyleValueKind::ColorScheme) {
                value.color_scheme_kind = static_cast<FFI::CssColorSchemeValueKind>(color_red);
                value.color_scheme_only = color_green != 0;
                for (auto scheme : StringView { value_ptr, value_len }.split_view('\0'))
                    value.color_scheme_schemes.append(String::from_utf8_without_validation(scheme.bytes()));
            } else if (kind == FFI::CssStyleValueKind::FontVariantAlternates) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FontVariantAlternates);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                FontVariantAlternatesValue font_variant_alternates_value {
                    .kind = static_cast<FFI::CssFontVariantAlternatesValueKind>(color_red),
                };
                for (auto feature_value_name : StringView { value_ptr, value_len }.split_view('\0'))
                    font_variant_alternates_value.feature_value_names.append(fly_string_from_ffi_bytes(feature_value_name.bytes().data(), feature_value_name.bytes().size()));
                style_value->font_variant_alternates.append(move(font_variant_alternates_value));
                return;
            } else if (kind == FFI::CssStyleValueKind::FontVariantEastAsian) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FontVariantEastAsian);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->font_variant_east_asian.append(FontVariantEastAsianValue {
                    .kind = static_cast<FFI::CssFontVariantEastAsianValueKind>(color_red),
                    .value = fly_string_from_ffi_bytes(value_ptr, value_len),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::FontVariantLigatures) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FontVariantLigatures);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->font_variant_ligatures.append(FontVariantLigaturesValue {
                    .kind = static_cast<FFI::CssFontVariantLigaturesValueKind>(color_red),
                    .value = fly_string_from_ffi_bytes(value_ptr, value_len),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::FontVariantNumeric) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FontVariantNumeric);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->font_variant_numeric.append(FontVariantNumericValue {
                    .kind = static_cast<FFI::CssFontVariantNumericValueKind>(color_red),
                    .value = fly_string_from_ffi_bytes(value_ptr, value_len),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::FontFamily) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FontFamily);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->font_family.append(FontFamilyValue {
                    .kind = static_cast<FFI::CssFontFamilyValueKind>(color_red),
                    .value = fly_string_from_ffi_bytes(value_ptr, value_len),
                    .is_string = color_green != 0,
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::FontShorthand) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                    style_value->property_id = PropertyID::Font;
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FontShorthand);
                    VERIFY(style_value->property_id == PropertyID::Font);
                }

                style_value->font_shorthand_items.append(FontShorthandItem {
                    .property_id = static_cast<PropertyID>(property_id),
                    .value = string_from_ffi_bytes(value_ptr, value_len),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::ComponentShorthand) {
                enum : u8 {
                    ItemStart = 255,
                };

                if (!style_value.has_value()) {
                    style_value = move(value);
                    style_value->property_id = shorthand_property_id_from_callback_payload();
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::ComponentShorthand);
                    VERIFY(style_value->property_id == shorthand_property_id_from_callback_payload());
                }

                VERIFY(color_blue == ItemStart);
                style_value->component_shorthand_items.append(ComponentShorthandItem {
                    .property_id = static_cast<PropertyID>(property_id),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::CoordinatingValueListShorthand) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                    style_value->property_id = shorthand_property_id_from_callback_payload();
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::CoordinatingValueListShorthand);
                    VERIFY(style_value->property_id == shorthand_property_id_from_callback_payload());
                }

                style_value->coordinating_value_list_shorthand_items.append(CoordinatingValueListShorthandItem {
                    .layer_index = layer_index_from_callback_payload(),
                    .property_id = static_cast<PropertyID>(property_id),
                    .value = string_from_ffi_bytes(value_ptr, value_len),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::LayerShorthand) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                    style_value->property_id = shorthand_property_id_from_callback_payload();
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::LayerShorthand);
                    VERIFY(style_value->property_id == shorthand_property_id_from_callback_payload());
                }

                style_value->layer_shorthand_items.append(LayerShorthandItem {
                    .layer_index = layer_index_from_callback_payload(),
                    .property_id = static_cast<PropertyID>(property_id),
                    .value = string_from_ffi_bytes(value_ptr, value_len),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::PositionalValueListShorthand) {
                enum : u8 {
                    ItemStart = 255,
                };

                if (!style_value.has_value()) {
                    style_value = move(value);
                    style_value->property_id = shorthand_property_id_from_callback_payload();
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::PositionalValueListShorthand);
                    VERIFY(style_value->property_id == shorthand_property_id_from_callback_payload());
                }

                VERIFY(color_alpha == ItemStart);
                style_value->positional_value_list_shorthand_items.append(PositionalValueListShorthandItem {
                    .index = color_blue,
                    .property_id = static_cast<PropertyID>(property_id),
                    .value = string_from_ffi_bytes(value_ptr, value_len),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::GridPlacementShorthand) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                    style_value->property_id = shorthand_property_id_from_callback_payload();
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::GridPlacementShorthand);
                    VERIFY(style_value->property_id == shorthand_property_id_from_callback_payload());
                }

                style_value->grid_placement_shorthand_items.append(GridPlacementShorthandItem {
                    .property_id = static_cast<PropertyID>(property_id),
                    .value = RustGridTrackPlacement {
                        .kind = static_cast<RustGridTrackPlacementKind>(color_blue),
                        .line_number = has_numeric_value || value_len != 0
                            ? Optional<RustNestedPrimitiveValue> { nested_primitive_value_from_callback_payload() }
                            : Optional<RustNestedPrimitiveValue> {},
                        .name = value_type_len == 0 ? Optional<String> {} : string_from_ffi_bytes(value_type_ptr, value_type_len),
                    },
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::GridTemplateShorthand) {
                enum : u8 {
                    Empty = 254,
                    ItemStart = 255,
                };

                if (!style_value.has_value()) {
                    style_value = move(value);
                    style_value->property_id = shorthand_property_id_from_callback_payload();
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::GridTemplateShorthand);
                }

                if (color_blue == Empty)
                    return;

                if (color_blue == ItemStart) {
                    VERIFY(style_value->property_id == shorthand_property_id_from_callback_payload());
                    style_value->grid_template_shorthand_items.append(GridTemplateShorthandItem {
                        .property_id = static_cast<PropertyID>(property_id),
                    });
                    return;
                }

                VERIFY(!style_value->grid_template_shorthand_items.is_empty());
                auto& item = style_value->grid_template_shorthand_items.last();
                VERIFY(item.property_id == static_cast<PropertyID>(property_id));

                switch (item.property_id) {
                case PropertyID::GridAutoFlow:
                    item.grid_auto_flow_axis = color_red;
                    item.grid_auto_flow_dense = color_green;
                    break;
                case PropertyID::GridTemplateAreas:
                    if (color_red == 0) {
                        item.grid_template_areas_is_none = true;
                    } else {
                        VERIFY(color_red == 1);
                        item.grid_template_area_rows.append(string_from_ffi_bytes(value_ptr, value_len));
                    }
                    break;
                case PropertyID::GridAutoColumns:
                case PropertyID::GridAutoRows:
                case PropertyID::GridTemplateColumns:
                case PropertyID::GridTemplateRows:
                    enum : u8 {
                        SecondaryCalculationTarget = 7,
                    };
                    if (color_red == SecondaryCalculationTarget) {
                        item.last_calculation_node_target = RustCalculationNodeTarget::GridTrackSecondaryValue;
                        break;
                    }
                    if (color_red == 0) {
                        item.grid_track_size_list_is_none = true;
                        break;
                    }
                    item.last_calculation_node_target = RustCalculationNodeTarget::GridTrackValue;
                    item.grid_track_size_list_events.append(RustGridTrackSizeListEvent {
                        .kind = static_cast<RustGridTrackSizeListEventKind>(color_red),
                        .repeat_type = static_cast<RustGridRepeatType>(color_green),
                        .breadth_kind = static_cast<RustGridTrackBreadthKind>(color_blue),
                        .secondary_breadth_kind = static_cast<RustGridTrackBreadthKind>(color_green),
                        .value = nested_primitive_value_from_callback_payload(),
                        .secondary_value = secondary_nested_primitive_value_from_callback_payload(),
                        .source = string_from_ffi_bytes(value_ptr, value_len),
                    });
                    break;
                default:
                    VERIFY_NOT_REACHED();
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::FontLanguageOverride) {
                value.font_language_override_kind = static_cast<FFI::CssFontLanguageOverrideKind>(color_red);
                if (value_len > 0)
                    value.font_language_override = fly_string_from_ffi_bytes(value_ptr, value_len);
            } else if (kind == FFI::CssStyleValueKind::FontStyle) {
                value.font_style = FontStyle {
                    .kind = static_cast<FFI::CssFontStyleKind>(color_red),
                    .has_angle = color_green != 0,
                };
                if (value.font_style.has_angle)
                    value.font_style.angle = nested_primitive_value_from_callback_payload();
            } else if (kind == FFI::CssStyleValueKind::FontFeatureSettings || kind == FFI::CssStyleValueKind::FontVariationSettings) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == kind);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->open_type_settings_kind = static_cast<FFI::CssOpenTypeSettingsKind>(color_red);
                if (value_len > 0) {
                    VERIFY(value_len >= 4);
                    Optional<String> tag_value;
                    if (color_green == static_cast<u8>(FFI::CssOpenTypeTaggedValueKind::Value))
                        tag_value = string_from_ffi_bytes(value_ptr + 4, value_len - 4);
                    style_value->open_type_tag_values.append(OpenTypeTaggedValue {
                        .tag = fly_string_from_ffi_bytes(value_ptr, 4),
                        .value_kind = static_cast<FFI::CssOpenTypeTaggedValueKind>(color_green),
                        .value = move(tag_value),
                    });
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::StrokeDasharray) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::StrokeDasharray);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red != 0) {
                    style_value->stroke_dasharray_none = true;
                } else {
                    style_value->stroke_dasharray_values.append(nested_primitive_value_from_callback_payload());
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::StrokeDasharray;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::BorderSpacing) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::BorderSpacing);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->border_spacing_values.append(nested_primitive_value_from_callback_payload());
                return;
            } else if (kind == FFI::CssStyleValueKind::KeywordList) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::KeywordList);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->keyword_list.append(string_from_ffi_bytes(value_ptr, value_len));
                return;
            } else if (first_is_one_of(kind, FFI::CssStyleValueKind::PlaceContent, FFI::CssStyleValueKind::PlaceItems, FFI::CssStyleValueKind::PlaceSelf)) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == kind);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == 0)
                    style_value->place_align_keywords.append(string_from_ffi_bytes(value_ptr, value_len));
                else
                    style_value->place_justify_keywords.append(string_from_ffi_bytes(value_ptr, value_len));
                return;
            } else if (kind == FFI::CssStyleValueKind::OverflowClipMargin) {
                value.overflow_clip_margin = nested_primitive_value_from_callback_payload();
            } else if (kind == FFI::CssStyleValueKind::Columns) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Columns);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == 0) {
                    if (color_green != 0)
                        style_value->column_count_is_auto = true;
                    else
                        style_value->column_count = nested_primitive_value_from_callback_payload();
                } else if (color_red == 1) {
                    if (color_green != 0)
                        style_value->column_width_is_auto = true;
                    else
                        style_value->column_width = nested_primitive_value_from_callback_payload();
                } else {
                    if (color_green != 0)
                        style_value->column_height_is_auto = true;
                    else
                        style_value->column_height = nested_primitive_value_from_callback_payload();
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::FlexFlow) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FlexFlow);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == 0)
                    style_value->flex_direction = static_cast<FlexDirection>(color_green);
                else
                    style_value->flex_wrap = static_cast<FlexWrap>(color_green);
                return;
            } else if (kind == FFI::CssStyleValueKind::Flex) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Flex);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == 0) {
                    style_value->flex_shorthand_is_none = true;
                } else if (color_red == 1) {
                    style_value->flex_grow = nested_primitive_value_from_callback_payload();
                    style_value->last_flex_calculation_component = RustFlexCalculationComponent::Grow;
                } else if (color_red == 2) {
                    style_value->flex_shrink = nested_primitive_value_from_callback_payload();
                    style_value->last_flex_calculation_component = RustFlexCalculationComponent::Shrink;
                } else {
                    style_value->flex_basis_kind = static_cast<RustFlexBasisKind>(color_green);
                    if (*style_value->flex_basis_kind == RustFlexBasisKind::LengthPercentage || *style_value->flex_basis_kind == RustFlexBasisKind::FitContentFunction)
                        style_value->flex_basis = nested_primitive_value_from_callback_payload();
                    else if (*style_value->flex_basis_kind == RustFlexBasisKind::Source)
                        style_value->flex_basis_source = string_from_ffi_bytes(value_ptr, value_len);
                    style_value->last_flex_calculation_component = RustFlexCalculationComponent::Basis;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::TextDecoration) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::TextDecoration);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                auto component_kind = primitive_kind == FFI::CssPrimitiveValueKind::Invalid && has_numeric_value && has_secondary_numeric_value ? static_cast<u8>(numeric_value) : color_red;
                if (component_kind == 0) {
                    style_value->text_decoration_line_bits = color_green;
                } else if (component_kind == 1) {
                    style_value->text_decoration_thickness_kind = static_cast<RustTextDecorationThicknessKind>(color_green);
                    if (*style_value->text_decoration_thickness_kind == RustTextDecorationThicknessKind::LengthPercentage)
                        style_value->text_decoration_thickness = nested_primitive_value_from_callback_payload();
                } else if (component_kind == 2) {
                    style_value->text_decoration_style = static_cast<RustTextDecorationStyle>(color_green);
                } else {
                    style_value->text_decoration_color = style_color_from_callback_payload(has_numeric_value, secondary_numeric_value, color_red, color_green, color_blue, color_alpha, value_ptr, value_len);
                    note_style_color_source_component_target(style_value->text_decoration_color);
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::ListStyle) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::ListStyle);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == 0)
                    style_value->list_style_position = static_cast<RustListStylePosition>(color_green);
                else if (color_red == 1) {
                    enum : u8 {
                        ImageNone,
                        ImageSource,
                        ImageSetOption,
                    };
                    if (color_green == ImageSetOption) {
                        style_value->list_style_image_kind = RustListStyleImageKind::Source;
                        style_value->list_style_image_source_kind = RustImageKind::ImageSet;
                        style_value->list_style_image_source_image_set_options.append(image_set_option_metadata_from_callback_payload(color_blue));
                        note_source_component_values_target(style_value->list_style_image_source_image_set_options.last().image_source_component_values);
                        return;
                    }
                    style_value->list_style_image_kind = static_cast<RustListStyleImageKind>(color_green);
                    if (*style_value->list_style_image_kind == RustListStyleImageKind::Source) {
                        style_value->list_style_image_source_kind = static_cast<RustImageKind>(color_blue);
                        style_value->list_style_image_source = string_from_ffi_bytes(value_ptr, value_len);
                        style_value->list_style_image_source_url = image_url_from_callback_payload();
                        if (*style_value->list_style_image_source_kind == RustImageKind::Gradient)
                            style_value->list_style_image_gradient = RustGradient {};
                        note_source_component_values_target(style_value->list_style_image_source_component_values);
                    }
                } else {
                    auto type_kind = static_cast<RustListStyleTypeKind>(color_green);
                    style_value->list_style_type_kind = type_kind;
                    switch (type_kind) {
                    case RustListStyleTypeKind::None:
                        break;
                    case RustListStyleTypeKind::String:
                        style_value->list_style_type_string = string_from_ffi_bytes(value_ptr, value_len);
                        break;
                    case RustListStyleTypeKind::CounterStyleName:
                        style_value->list_style_type_counter_style = CounterStyle {
                            .kind = FFI::CssCounterStyleKind::Name,
                            .symbols_type = FFI::CssCounterStyleSymbolsType::Symbolic,
                            .name = fly_string_from_ffi_bytes(value_ptr, value_len),
                            .symbols = {},
                        };
                        break;
                    case RustListStyleTypeKind::CounterStyleSymbols:
                        style_value->list_style_type_counter_style = CounterStyle {
                            .kind = FFI::CssCounterStyleKind::SymbolsFunction,
                            .symbols_type = static_cast<FFI::CssCounterStyleSymbolsType>(color_blue),
                            .name = {},
                            .symbols = {},
                        };
                        break;
                    case RustListStyleTypeKind::CounterStyleSymbol:
                        VERIFY(style_value->list_style_type_counter_style.has_value());
                        style_value->list_style_type_counter_style->symbols.append(fly_string_from_ffi_bytes(value_ptr, value_len));
                        break;
                    }
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::MathDepth) {
                value.color_red = color_red;
                if (color_red != 0)
                    value.math_depth_integer = nested_primitive_value_from_callback_payload();
            } else if (kind == FFI::CssStyleValueKind::AspectRatio) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::AspectRatio);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_green != 0)
                    style_value->aspect_ratio_has_auto = true;
                if (color_red == 0)
                    style_value->aspect_ratio_numerator = nested_primitive_value_from_callback_payload();
                else if (color_red == 1)
                    style_value->aspect_ratio_denominator = nested_primitive_value_from_callback_payload();
                return;
            } else if (kind == FFI::CssStyleValueKind::BorderRadius) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::BorderRadius);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == 0) {
                    style_value->border_radius_horizontal_radii.append(nested_primitive_value_from_callback_payload());
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderRadiusHorizontal;
                } else {
                    style_value->border_radius_vertical_radii.append(nested_primitive_value_from_callback_payload());
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderRadiusVertical;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::Border) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Border);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                auto component_kind = primitive_kind == FFI::CssPrimitiveValueKind::Invalid && has_numeric_value && has_secondary_numeric_value ? static_cast<u8>(numeric_value) : color_red;
                switch (component_kind) {
                case 0:
                    if (color_blue != 0) {
                        style_value->border_width_length = nested_primitive_value_from_callback_payload();
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderWidthLength;
                    } else {
                        style_value->border_width_keyword = static_cast<LineWidth>(color_green);
                    }
                    break;
                case 1:
                    style_value->border_style = static_cast<LineStyle>(color_green);
                    break;
                case 2:
                    style_value->border_color = style_color_from_callback_payload(has_numeric_value, secondary_numeric_value, color_red, color_green, color_blue, color_alpha, value_ptr, value_len);
                    note_style_color_source_component_target(style_value->border_color);
                    break;
                default:
                    VERIFY_NOT_REACHED();
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::BorderImageSlice) {
                if (!style_value.has_value()) {
                    value.border_image_slice_fill = color_green != 0;
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::BorderImageSlice);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderImageSlice;
                style_value->border_image_slices.append(nested_primitive_value_from_callback_payload());
                return;
            } else if (kind == FFI::CssStyleValueKind::BorderImage) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::BorderImage);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                switch (color_red) {
                case 0:
                    style_value->border_image_source_kind = static_cast<RustBorderImageSourceKind>(color_green);
                    if (*style_value->border_image_source_kind == RustBorderImageSourceKind::Source) {
                        style_value->border_image_source_source_kind = static_cast<RustImageKind>(color_blue);
                        style_value->border_image_source_source = string_from_ffi_bytes(value_ptr, value_len);
                        style_value->border_image_source_source_url = image_url_from_callback_payload();
                        if (*style_value->border_image_source_source_kind == RustImageKind::Gradient)
                            style_value->border_image_source_gradient = RustGradient {};
                        note_source_component_values_target(style_value->border_image_source_source_component_values);
                    }
                    break;
                case 5:
                    style_value->border_image_source_kind = RustBorderImageSourceKind::Source;
                    style_value->border_image_source_source_kind = RustImageKind::ImageSet;
                    style_value->border_image_source_source_image_set_options.append(image_set_option_metadata_from_callback_payload(color_green));
                    note_source_component_values_target(style_value->border_image_source_source_image_set_options.last().image_source_component_values);
                    break;
                case 1:
                    style_value->border_image_shorthand_has_slice = true;
                    style_value->border_image_slice_fill = color_green != 0;
                    style_value->border_image_slices.append(nested_primitive_value_from_callback_payload());
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderImageSlice;
                    break;
                case 2:
                    style_value->border_image_shorthand_has_width = true;
                    style_value->border_image_widths.append(RustBorderImageWidth {
                        .is_auto = color_green != 0,
                        .value = color_green == 0 ? nested_primitive_value_from_callback_payload() : RustNestedPrimitiveValue {},
                    });
                    if (color_green == 0)
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderImageWidth;
                    break;
                case 3:
                    style_value->border_image_shorthand_has_outset = true;
                    style_value->border_image_outsets.append(RustBorderImageOutset {
                        .value = nested_primitive_value_from_callback_payload(),
                    });
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderImageOutset;
                    break;
                case 4:
                    style_value->border_image_shorthand_has_repeat = true;
                    style_value->border_image_repeats.append(static_cast<RustBorderImageRepeat>(color_green));
                    break;
                default:
                    VERIFY_NOT_REACHED();
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::BorderImageRepeat) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::BorderImageRepeat);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->border_image_repeats.append(static_cast<RustBorderImageRepeat>(color_green));
                return;
            } else if (kind == FFI::CssStyleValueKind::BorderImageOutset || kind == FFI::CssStyleValueKind::BorderImageWidth) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == kind);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;
                if (kind == FFI::CssStyleValueKind::BorderImageOutset) {
                    style_value->border_image_outsets.append(RustBorderImageOutset {
                        .value = nested_primitive_value_from_callback_payload(),
                    });
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderImageOutset;
                } else {
                    style_value->border_image_widths.append(RustBorderImageWidth {
                        .is_auto = color_green != 0,
                        .value = color_green == 0 ? nested_primitive_value_from_callback_payload() : RustNestedPrimitiveValue {},
                    });
                    if (color_green == 0)
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::BorderImageWidth;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::TransformOrigin) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::TransformOrigin);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                if (color_red == 0) {
                    style_value->transform_origin_x = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::TransformOriginX;
                } else if (color_red == 1) {
                    style_value->transform_origin_y = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::TransformOriginY;
                } else {
                    style_value->transform_origin_z = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::TransformOriginZ;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::TransformLonghand) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::TransformLonghand);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                enum : u8 {
                    None,
                    Function,
                };

                if (color_red == None) {
                    style_value->transform_longhand_is_none = true;
                    return;
                }

                VERIFY(color_red == Function);
                style_value->transform_longhand_function = static_cast<RustTransformLonghandFunction>(color_green);
                style_value->transform_longhand_arguments.append(RustTransformationArgument {
                    .parameter_type = static_cast<TransformFunctionParameterType>(color_blue),
                    .value = nested_primitive_value_from_callback_payload(),
                });
                style_value->last_calculation_node_target = RustCalculationNodeTarget::TransformLonghandArgument;
                return;
            } else if (kind == FFI::CssStyleValueKind::Transformation) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Transformation);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                enum : u8 {
                    BeginFunction,
                    Argument,
                };

                if (color_red == BeginFunction) {
                    style_value->transformations.append(RustTransformation {
                        .function = static_cast<TransformFunction>(color_green),
                    });
                    return;
                }

                VERIFY(color_red == Argument);
                VERIFY(!style_value->transformations.is_empty());
                VERIFY(style_value->transformations.last().function == static_cast<TransformFunction>(color_green));
                style_value->transformations.last().arguments.append(RustTransformationArgument {
                    .parameter_type = static_cast<TransformFunctionParameterType>(color_blue),
                    .value = nested_primitive_value_from_callback_payload(),
                });
                style_value->last_calculation_node_target = RustCalculationNodeTarget::TransformationArgument;
                return;
            } else if (kind == FFI::CssStyleValueKind::Shadow) {
                enum : u8 {
                    None,
                    BeginShadow,
                    Color,
                    OffsetX,
                    OffsetY,
                    BlurRadius,
                    SpreadDistance,
                };
                enum : u8 {
                    Outer,
                    Inner,
                };

                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Shadow);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                auto component_kind = primitive_kind == FFI::CssPrimitiveValueKind::Invalid && has_numeric_value && has_secondary_numeric_value ? static_cast<u8>(numeric_value) : color_red;
                if (component_kind == None) {
                    style_value->shadow_is_none = true;
                    return;
                }

                if (component_kind == BeginShadow) {
                    style_value->shadows.append(RustShadow {
                        .placement = color_green == Inner ? RustShadowPlacement::Inner : RustShadowPlacement::Outer,
                    });
                    return;
                }

                VERIFY(!style_value->shadows.is_empty());
                auto& shadow = style_value->shadows.last();
                if (component_kind == Color) {
                    shadow.color = style_color_from_callback_payload(has_numeric_value, secondary_numeric_value, color_red, color_green, color_blue, color_alpha, value_ptr, value_len);
                    note_style_color_source_component_target(shadow.color);
                } else if (component_kind == OffsetX) {
                    shadow.offset_x = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::ShadowOffsetX;
                } else if (component_kind == OffsetY) {
                    shadow.offset_y = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::ShadowOffsetY;
                } else if (component_kind == BlurRadius) {
                    shadow.blur_radius = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::ShadowBlurRadius;
                } else if (component_kind == SpreadDistance) {
                    shadow.spread_distance = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::ShadowSpreadDistance;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::ShapeOutside) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::ShapeOutside);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                switch (static_cast<RustShapeOutsideEventKind>(color_red)) {
                case RustShapeOutsideEventKind::None:
                    style_value->shape_outside_is_none = true;
                    break;
                case RustShapeOutsideEventKind::Image:
                    style_value->shape_outside_image_source_kind = static_cast<RustImageKind>(color_green);
                    style_value->shape_outside_image_source = string_from_ffi_bytes(value_ptr, value_len);
                    style_value->shape_outside_image_source_url = image_url_from_callback_payload();
                    if (*style_value->shape_outside_image_source_kind == RustImageKind::Gradient)
                        style_value->shape_outside_image_gradient = RustGradient {};
                    note_source_component_values_target(style_value->shape_outside_image_source_component_values);
                    break;
                case RustShapeOutsideEventKind::ImageSetOption:
                    style_value->shape_outside_image_source_kind = RustImageKind::ImageSet;
                    style_value->shape_outside_image_source_image_set_options.append(image_set_option_metadata_from_callback_payload(color_green));
                    note_source_component_values_target(style_value->shape_outside_image_source_image_set_options.last().image_source_component_values);
                    break;
                case RustShapeOutsideEventKind::BasicShape: {
                    style_value->shape_outside_basic_shape_kind = static_cast<RustBasicShapeKind>(color_green);
                    enum : u8 {
                        BasicShapeComponentHeader,
                        BasicShapeComponentPolygonPointX,
                        BasicShapeComponentPolygonPointY,
                        BasicShapeComponentRectangleLengthPercentage,
                        BasicShapeComponentRectangleAuto,
                        BasicShapeComponentRectangleBorderRadiusHorizontal,
                        BasicShapeComponentRectangleBorderRadiusVertical,
                        BasicShapeComponentRadialExtent,
                        BasicShapeComponentRadialLengthPercentage,
                        BasicShapeComponentRadialPositionX,
                        BasicShapeComponentRadialPositionY,
                    };
                    auto shape_outside_radial_position_component_from_callback_payload = [&]() {
                        RustPositionComponent component {
                            .edge = static_cast<RustPositionEdge>(color_alpha & 0x7f),
                        };
                        if ((color_alpha & 0x80) != 0)
                            component.offset = nested_primitive_value_from_callback_payload();
                        return component;
                    };
                    if (*style_value->shape_outside_basic_shape_kind == RustBasicShapeKind::Circle || *style_value->shape_outside_basic_shape_kind == RustBasicShapeKind::Ellipse) {
                        style_value->shape_outside_basic_shape_radial_shape_is_typed = true;
                        if (color_blue == BasicShapeComponentRadialExtent) {
                            style_value->shape_outside_basic_shape_radial_shape_radius.append({
                                .is_radial_extent = true,
                                .radial_extent = static_cast<RustBasicShapeRadialExtent>(color_alpha),
                            });
                        } else if (color_blue == BasicShapeComponentRadialLengthPercentage) {
                            style_value->shape_outside_basic_shape_radial_shape_radius.append({ .length_percentage = nested_primitive_value_from_callback_payload() });
                            style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapeRadius;
                        } else if (color_blue == BasicShapeComponentRadialPositionX) {
                            if (!style_value->shape_outside_basic_shape_radial_shape_position.has_value())
                                style_value->shape_outside_basic_shape_radial_shape_position = RustPosition {};
                            style_value->shape_outside_basic_shape_radial_shape_position->x = shape_outside_radial_position_component_from_callback_payload();
                            if (style_value->shape_outside_basic_shape_radial_shape_position->x.offset.has_value())
                                style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapePositionX;
                        } else if (color_blue == BasicShapeComponentRadialPositionY) {
                            if (!style_value->shape_outside_basic_shape_radial_shape_position.has_value())
                                style_value->shape_outside_basic_shape_radial_shape_position = RustPosition {};
                            style_value->shape_outside_basic_shape_radial_shape_position->y = shape_outside_radial_position_component_from_callback_payload();
                            if (style_value->shape_outside_basic_shape_radial_shape_position->y.offset.has_value())
                                style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapePositionY;
                        } else {
                            VERIFY(color_blue == BasicShapeComponentHeader);
                        }
                    } else if (*style_value->shape_outside_basic_shape_kind == RustBasicShapeKind::Inset || *style_value->shape_outside_basic_shape_kind == RustBasicShapeKind::Xywh || *style_value->shape_outside_basic_shape_kind == RustBasicShapeKind::Rect) {
                        if (color_blue == BasicShapeComponentRectangleLengthPercentage) {
                            style_value->shape_outside_basic_shape_rectangle_components.append({ .value = nested_primitive_value_from_callback_payload() });
                            style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapeRectangleComponent;
                        } else if (color_blue == BasicShapeComponentRectangleAuto) {
                            style_value->shape_outside_basic_shape_rectangle_components.append({ .is_auto = true });
                        } else if (color_blue == BasicShapeComponentRectangleBorderRadiusHorizontal) {
                            style_value->shape_outside_basic_shape_rectangle_border_radius_horizontal_radii.append(nested_primitive_value_from_callback_payload());
                            style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapeRectangleRadiusHorizontal;
                        } else if (color_blue == BasicShapeComponentRectangleBorderRadiusVertical) {
                            style_value->shape_outside_basic_shape_rectangle_border_radius_vertical_radii.append(nested_primitive_value_from_callback_payload());
                            style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapeRectangleRadiusVertical;
                        } else {
                            VERIFY(color_blue == BasicShapeComponentHeader);
                        }
                    } else if (*style_value->shape_outside_basic_shape_kind == RustBasicShapeKind::Polygon) {
                        style_value->shape_outside_basic_shape_fill_rule = color_alpha;
                        if (color_blue == BasicShapeComponentPolygonPointX || color_blue == BasicShapeComponentPolygonPointY) {
                            style_value->shape_outside_basic_shape_polygon_coordinates.append(nested_primitive_value_from_callback_payload());
                            style_value->last_calculation_node_target = RustCalculationNodeTarget::BasicShapePolygonCoordinate;
                        } else {
                            VERIFY(color_blue == BasicShapeComponentHeader);
                        }
                    } else if (*style_value->shape_outside_basic_shape_kind == RustBasicShapeKind::Path) {
                        style_value->shape_outside_basic_shape_fill_rule = color_blue;
                        style_value->shape_outside_basic_shape_path_data = string_from_ffi_bytes(value_ptr, value_len);
                    }
                    break;
                }
                case RustShapeOutsideEventKind::ShapeBox:
                    style_value->shape_outside_shape_box = static_cast<ShapeBox>(color_green);
                    break;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::Content) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Content);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                switch (static_cast<RustContentEventKind>(color_red)) {
                case RustContentEventKind::Normal:
                    style_value->content_keyword = Keyword::Normal;
                    break;
                case RustContentEventKind::None:
                    style_value->content_keyword = Keyword::None;
                    break;
                case RustContentEventKind::ItemQuote:
                case RustContentEventKind::ItemString:
                case RustContentEventKind::ItemImage:
                case RustContentEventKind::AltTextString:
                    style_value->content_events.append({
                        .kind = static_cast<RustContentEventKind>(color_red),
                        .image_kind = static_cast<RustImageKind>(color_green),
                        .source = string_from_ffi_bytes(value_ptr, value_len),
                        .image_url = image_url_from_callback_payload(),
                    });
                    if (static_cast<RustContentEventKind>(color_red) == RustContentEventKind::ItemImage) {
                        if (style_value->content_events.last().image_kind == RustImageKind::Gradient)
                            style_value->content_events.last().gradient = RustGradient {};
                        note_source_component_values_target(style_value->content_events.last().image_source_component_values);
                    }
                    break;
                case RustContentEventKind::ItemCounter:
                case RustContentEventKind::AltTextCounter:
                    style_value->content_events.append({
                        .kind = static_cast<RustContentEventKind>(color_red),
                        .counter_function = static_cast<RustCounterFunctionKind>(color_green),
                        .counter_name = fly_string_from_ffi_bytes(value_ptr, value_len),
                    });
                    break;
                case RustContentEventKind::CounterJoinString:
                    VERIFY(!style_value->content_events.is_empty());
                    style_value->content_events.last().counter_join_string = fly_string_from_ffi_bytes(value_ptr, value_len);
                    break;
                case RustContentEventKind::CounterStyleName:
                    VERIFY(!style_value->content_events.is_empty());
                    style_value->content_events.last().counter_style = CounterStyle {
                        .kind = FFI::CssCounterStyleKind::Name,
                        .symbols_type = FFI::CssCounterStyleSymbolsType::Symbolic,
                        .name = fly_string_from_ffi_bytes(value_ptr, value_len),
                    };
                    break;
                case RustContentEventKind::CounterStyleSymbols:
                    VERIFY(!style_value->content_events.is_empty());
                    style_value->content_events.last().counter_style = CounterStyle {
                        .kind = FFI::CssCounterStyleKind::SymbolsFunction,
                        .symbols_type = static_cast<FFI::CssCounterStyleSymbolsType>(color_green),
                    };
                    break;
                case RustContentEventKind::CounterStyleSymbol:
                    VERIFY(!style_value->content_events.is_empty());
                    VERIFY(style_value->content_events.last().counter_style.has_value());
                    style_value->content_events.last().counter_style->symbols.append(fly_string_from_ffi_bytes(value_ptr, value_len));
                    break;
                case RustContentEventKind::ImageSetOption:
                    VERIFY(!style_value->content_events.is_empty());
                    VERIFY(style_value->content_events.last().kind == RustContentEventKind::ItemImage);
                    VERIFY(style_value->content_events.last().image_kind == RustImageKind::ImageSet);
                    style_value->content_events.last().image_set_options.append(image_set_option_metadata_from_callback_payload(color_green));
                    note_source_component_values_target(style_value->content_events.last().image_set_options.last().image_source_component_values);
                    break;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::FontVariant) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FontVariant);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                switch (static_cast<RustFontVariantEventKind>(color_red)) {
                case RustFontVariantEventKind::Normal:
                    break;
                case RustFontVariantEventKind::Simple:
                    switch (static_cast<FFI::CssFontVariantSimpleValueKind>(color_green)) {
                    case FFI::CssFontVariantSimpleValueKind::LigaturesNone:
                        style_value->font_variant_ligatures_none = true;
                        break;
                    case FFI::CssFontVariantSimpleValueKind::Caps:
                        style_value->font_variant_caps = fly_string_from_ffi_bytes(value_ptr, value_len);
                        break;
                    case FFI::CssFontVariantSimpleValueKind::Emoji:
                        style_value->font_variant_emoji = fly_string_from_ffi_bytes(value_ptr, value_len);
                        break;
                    case FFI::CssFontVariantSimpleValueKind::Position:
                        style_value->font_variant_position = fly_string_from_ffi_bytes(value_ptr, value_len);
                        break;
                    }
                    break;
                case RustFontVariantEventKind::AlternatesValue:
                    style_value->font_variant_alternates.append({
                        .kind = static_cast<FFI::CssFontVariantAlternatesValueKind>(color_green),
                    });
                    break;
                case RustFontVariantEventKind::AlternatesFeatureValueName:
                    VERIFY(!style_value->font_variant_alternates.is_empty());
                    style_value->font_variant_alternates.last().feature_value_names.append(fly_string_from_ffi_bytes(value_ptr, value_len));
                    break;
                case RustFontVariantEventKind::EastAsianValue:
                    style_value->font_variant_east_asian.append({
                        .kind = static_cast<FFI::CssFontVariantEastAsianValueKind>(color_green),
                        .value = fly_string_from_ffi_bytes(value_ptr, value_len),
                    });
                    break;
                case RustFontVariantEventKind::NumericValue:
                    style_value->font_variant_numeric.append({
                        .kind = static_cast<FFI::CssFontVariantNumericValueKind>(color_green),
                        .value = fly_string_from_ffi_bytes(value_ptr, value_len),
                    });
                    break;
                case RustFontVariantEventKind::LigaturesValue:
                    style_value->font_variant_ligatures.append({
                        .kind = static_cast<FFI::CssFontVariantLigaturesValueKind>(color_green),
                        .value = fly_string_from_ffi_bytes(value_ptr, value_len),
                    });
                    break;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::FilterValueList) {
                enum : u8 {
                    None,
                };

                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::FilterValueList);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                auto component_kind = primitive_kind == FFI::CssPrimitiveValueKind::Invalid && has_numeric_value && has_secondary_numeric_value ? static_cast<u8>(numeric_value) : color_red;
                if (component_kind == None) {
                    style_value->filter_value_list_is_none = true;
                    return;
                }

                auto filter_event_kind = static_cast<RustFilterValueListEventKind>(component_kind);
                if (filter_event_kind == RustFilterValueListEventKind::DropShadowRadius) {
                    VERIFY(!style_value->filter_value_list_events.is_empty());
                    VERIFY(style_value->filter_value_list_events.last().kind == RustFilterValueListEventKind::DropShadow);
                    style_value->filter_value_list_events.last().drop_shadow_radius = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::FilterDropShadowRadius;
                    return;
                }
                if (filter_event_kind == RustFilterValueListEventKind::DropShadowColor) {
                    VERIFY(!style_value->filter_value_list_events.is_empty());
                    VERIFY(style_value->filter_value_list_events.last().kind == RustFilterValueListEventKind::DropShadow);
                    style_value->filter_value_list_events.last().drop_shadow_color = style_color_from_callback_payload(has_numeric_value, secondary_numeric_value, color_red, color_green, color_blue, color_alpha, value_ptr, value_len);
                    note_style_color_source_component_target(style_value->filter_value_list_events.last().drop_shadow_color);
                    return;
                }

                style_value->filter_value_list_events.append(RustFilterValueListEvent {
                    .kind = filter_event_kind,
                    .simple_function = static_cast<RustSimpleFilterFunction>(color_green),
                    .has_value = color_blue != 0,
                    .has_secondary_value = filter_event_kind == RustFilterValueListEventKind::DropShadow,
                    .value = nested_primitive_value_from_callback_payload(),
                    .secondary_value = secondary_nested_primitive_value_from_callback_payload(),
                    .source = string_from_ffi_bytes(value_ptr, value_len),
                    .url = filter_event_kind == RustFilterValueListEventKind::Url
                        ? image_url_from_callback_payload()
                        : OptionalNone {},
                });
                if (!style_value->filter_value_list_events.last().has_secondary_value && style_value->filter_value_list_events.last().has_value)
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::FilterValue;
                return;
            } else if (kind == FFI::CssStyleValueKind::Cursor) {
                enum : u8 {
                    Image,
                    Predefined,
                    ImageCoordinateX,
                    ImageCoordinateY,
                    ImageSetOption,
                };

                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Cursor);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == Predefined) {
                    style_value->cursor_predefined = fly_string_from_ffi_bytes(value_ptr, value_len);
                    return;
                }

                if (color_red == ImageCoordinateX) {
                    VERIFY(!style_value->cursor_images.is_empty());
                    style_value->cursor_images.last().x = nested_primitive_value_from_callback_payload();
                    return;
                }

                if (color_red == ImageCoordinateY) {
                    VERIFY(!style_value->cursor_images.is_empty());
                    style_value->cursor_images.last().y = nested_primitive_value_from_callback_payload();
                    return;
                }

                if (color_red == ImageSetOption) {
                    VERIFY(!style_value->cursor_images.is_empty());
                    VERIFY(style_value->cursor_images.last().image_kind == RustImageKind::ImageSet);
                    style_value->cursor_images.last().image_set_options.append(image_set_option_metadata_from_callback_payload(color_green));
                    note_source_component_values_target(style_value->cursor_images.last().image_set_options.last().image_source_component_values);
                    return;
                }

                VERIFY(color_red == Image);
                RustCursorImage image {
                    .image_kind = static_cast<RustImageKind>(color_green),
                    .image_source = string_from_ffi_bytes(value_ptr, value_len),
                    .image_url = image_url_from_callback_payload(),
                };
                if (image.image_kind == RustImageKind::Gradient)
                    image.gradient = RustGradient {};
                style_value->cursor_images.append(move(image));
                note_source_component_values_target(style_value->cursor_images.last().image_source_component_values);
                return;
            } else if (first_is_one_of(kind,
                           FFI::CssStyleValueKind::GridAutoTrackSizes,
                           FFI::CssStyleValueKind::GridTrackSizeList)) {
                enum : u8 {
                    None,
                    SecondaryCalculationTarget = 7,
                };

                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == kind);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == None) {
                    style_value->grid_track_size_list_is_none = true;
                    return;
                }

                if (color_red == SecondaryCalculationTarget) {
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::GridTrackSecondaryValue;
                    return;
                }

                style_value->last_calculation_node_target = RustCalculationNodeTarget::GridTrackValue;
                style_value->grid_track_size_list_events.append(RustGridTrackSizeListEvent {
                    .kind = static_cast<RustGridTrackSizeListEventKind>(color_red),
                    .repeat_type = static_cast<RustGridRepeatType>(color_green),
                    .breadth_kind = static_cast<RustGridTrackBreadthKind>(color_blue),
                    .secondary_breadth_kind = static_cast<RustGridTrackBreadthKind>(color_green),
                    .value = nested_primitive_value_from_callback_payload(),
                    .secondary_value = secondary_nested_primitive_value_from_callback_payload(),
                    .source = string_from_ffi_bytes(value_ptr, value_len),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::GridTrackPlacement) {
                value.grid_track_placement = RustGridTrackPlacement {
                    .kind = static_cast<RustGridTrackPlacementKind>(color_red),
                    .line_number = has_numeric_value || value_len != 0
                        ? Optional<RustNestedPrimitiveValue> { nested_primitive_value_from_callback_payload() }
                        : Optional<RustNestedPrimitiveValue> {},
                    .name = value_type_len == 0 ? Optional<String> {} : string_from_ffi_bytes(value_type_ptr, value_type_len),
                };
            } else if (kind == FFI::CssStyleValueKind::GridTemplateAreas) {
                enum : u8 {
                    None,
                    Row,
                };

                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == kind);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == None) {
                    style_value->grid_template_areas_is_none = true;
                    return;
                }

                VERIFY(color_red == Row);
                style_value->grid_template_area_rows.append(string_from_ffi_bytes(value_ptr, value_len));
                return;
            } else if (kind == FFI::CssStyleValueKind::PositionArea) {
                enum : u8 {
                    None,
                    Area,
                };

                if (color_red == None) {
                    value.position_area_is_none = true;
                } else if (color_red == Area) {
                    value.position_area = RustPositionArea {
                        .first_keyword = fly_string_from_ffi_bytes(value_ptr, value_len),
                        .second_keyword = value_type_len == 0 ? Optional<FlyString> {} : fly_string_from_ffi_bytes(value_type_ptr, value_type_len),
                    };
                }
            } else if (kind == FFI::CssStyleValueKind::PositionTryFallbacks) {
                enum : u8 {
                    None,
                    PositionArea,
                    TryTactic,
                };

                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::PositionTryFallbacks);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == None) {
                    style_value->position_try_fallbacks_is_none = true;
                    return;
                }

                if (color_red == PositionArea) {
                    style_value->position_try_fallbacks.append(RustPositionTryFallback {
                        .kind = RustPositionTryFallbackKind::PositionArea,
                        .position_area = {
                            .first_keyword = fly_string_from_ffi_bytes(value_ptr, value_len),
                            .second_keyword = value_type_len == 0 ? Optional<FlyString> {} : fly_string_from_ffi_bytes(value_type_ptr, value_type_len),
                        },
                    });
                    return;
                }

                VERIFY(color_red == TryTactic);
                style_value->position_try_fallbacks.append(RustPositionTryFallback {
                    .kind = RustPositionTryFallbackKind::TryTactic,
                    .dashed_ident = value_len == 0 ? Optional<FlyString> {} : fly_string_from_ffi_bytes(value_ptr, value_len),
                    .has_flip_block = color_green != 0,
                    .has_flip_inline = color_blue != 0,
                    .has_flip_start = color_alpha != 0,
                });
                if (value_type_len > 0) {
                    for (auto try_tactic : StringView { value_type_ptr, value_type_len }.split_view(' '))
                        style_value->position_try_fallbacks.last().try_tactics.append(fly_string_from_ffi_bytes(try_tactic.bytes().data(), try_tactic.bytes().size()));
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::ScrollFunction) {
                value.scroll_function_scroller = static_cast<FFI::CssScrollFunctionScrollerKind>(color_red);
                value.scroll_function_axis = static_cast<FFI::CssScrollFunctionAxisKind>(color_green);
            } else if (kind == FFI::CssStyleValueKind::Contain) {
                value.contain = FFI::CssContainValue {
                    .kind = static_cast<FFI::CssContainValueKind>(color_red),
                    .is_size = static_cast<bool>(color_green & 1),
                    .is_inline_size = static_cast<bool>(color_green & 2),
                    .has_layout = static_cast<bool>(color_green & 4),
                    .has_style = static_cast<bool>(color_green & 8),
                    .has_paint = static_cast<bool>(color_green & 16),
                };
            } else if (kind == FFI::CssStyleValueKind::ContainerType) {
                value.container_type = static_cast<FFI::CssContainerTypeValueKind>(color_red);
            } else if (first_is_one_of(kind, FFI::CssStyleValueKind::Counter, FFI::CssStyleValueKind::CounterStyle)) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == kind);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                switch (static_cast<RustCounterEventKind>(color_red)) {
                case RustCounterEventKind::Function:
                    style_value->counter_function = static_cast<RustCounterFunctionKind>(color_green);
                    style_value->counter_name = fly_string_from_ffi_bytes(value_ptr, value_len);
                    break;
                case RustCounterEventKind::JoinString:
                    style_value->counter_join_string = fly_string_from_ffi_bytes(value_ptr, value_len);
                    break;
                case RustCounterEventKind::StyleName:
                    style_value->counter_style = CounterStyle {
                        .kind = FFI::CssCounterStyleKind::Name,
                        .symbols_type = FFI::CssCounterStyleSymbolsType::Symbolic,
                        .name = fly_string_from_ffi_bytes(value_ptr, value_len),
                    };
                    break;
                case RustCounterEventKind::StyleSymbols:
                    style_value->counter_style = CounterStyle {
                        .kind = FFI::CssCounterStyleKind::SymbolsFunction,
                        .symbols_type = static_cast<FFI::CssCounterStyleSymbolsType>(color_green),
                    };
                    break;
                case RustCounterEventKind::StyleSymbol:
                    VERIFY(style_value->counter_style.has_value());
                    style_value->counter_style->symbols.append(fly_string_from_ffi_bytes(value_ptr, value_len));
                    break;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::CounterDefinitions) {
                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::CounterDefinitions);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                style_value->counter_definitions.append(CounterDefinition {
                    .name = fly_string_from_ffi_bytes(value_ptr, value_len),
                    .is_reversed = color_red != 0,
                    .value = nullptr,
                });
                style_value->counter_definition_values.append(RustNestedPrimitiveValue {
                    .primitive_kind = primitive_kind,
                    .numeric_value = has_numeric_value ? Optional<double> { numeric_value } : Optional<double> {},
                    .source_or_unit = string_from_ffi_bytes(value_type_ptr, value_type_len),
                    .source_component_values = move(context.pending_nested_primitive_source_component_values),
                });
                return;
            } else if (kind == FFI::CssStyleValueKind::Display) {
                value.display_kind = static_cast<RustDisplayValueKind>(color_red);
                value.display_value = color_green;
                value.display_inside = static_cast<RustDisplayInside>(color_blue);
                value.display_list_item = static_cast<RustDisplayListItem>(color_alpha);
            } else if (kind == FFI::CssStyleValueKind::GridAutoFlow) {
                value.grid_auto_flow_axis = color_red;
                value.grid_auto_flow_dense = color_green;
            } else if (kind == FFI::CssStyleValueKind::CornerShape) {
                if (primitive_kind == FFI::CssPrimitiveValueKind::Keyword) {
                    auto keyword = keyword_from_string({ value_ptr, value_len });
                    if (!keyword.has_value())
                        return;
                    value.corner_shape_keyword = keyword.release_value();
                } else {
                    value.corner_shape_superellipse_parameter = nested_primitive_value_from_callback_payload();
                    value.last_calculation_node_target = RustCalculationNodeTarget::CornerShapeParameter;
                }
            } else if (kind == FFI::CssStyleValueKind::Paint) {
                enum : u8 {
                    None,
                    Color,
                    Url,
                    FallbackColor = 4,
                };

                if (!style_value.has_value()) {
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Paint);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                auto event_kind = has_numeric_value ? static_cast<u8>(numeric_value) : color_red;
                switch (event_kind) {
                case None:
                    style_value->paint_is_none = true;
                    break;
                case Color:
                    style_value->paint_color = style_color_from_callback_payload(has_secondary_numeric_value, secondary_numeric_value, color_red, color_green, color_blue, color_alpha, value_ptr, value_len);
                    note_style_color_source_component_target(style_value->paint_color);
                    break;
                case Url:
                    style_value->paint_url_source = string_from_ffi_bytes(value_ptr, value_len);
                    style_value->paint_url = image_url_from_callback_payload();
                    break;
                case FallbackColor:
                    style_value->paint_fallback_color = style_color_from_callback_payload(has_secondary_numeric_value, secondary_numeric_value, color_red, color_green, color_blue, color_alpha, value_ptr, value_len);
                    note_style_color_source_component_target(style_value->paint_fallback_color);
                    break;
                default:
                    VERIFY_NOT_REACHED();
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::PaintOrder) {
                value.paint_order = FFI::CssPaintOrderValue {
                    .kind = static_cast<FFI::CssPaintOrderValueKind>(color_red),
                    .first = static_cast<FFI::CssPaintOrderKeyword>(color_green),
                    .second = static_cast<FFI::CssPaintOrderKeyword>(color_blue),
                };
            } else if (kind == FFI::CssStyleValueKind::Position) {
                enum : u8 {
                    Header,
                    BeginPosition,
                    PositionX,
                    PositionY,
                    LonghandComponent,
                };

                auto position_component_from_callback_payload = [&]() {
                    RustPositionComponent component {
                        .edge = static_cast<RustPositionEdge>(color_green),
                    };
                    if (color_blue != 0)
                        component.offset = nested_primitive_value_from_callback_payload();
                    return component;
                };

                if (!style_value.has_value()) {
                    auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                    if (!value_type.has_value())
                        return;
                    value.value_type = value_type.release_value();
                    style_value = move(value);
                } else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::Position);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;

                if (color_red == Header) {
                    auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                    if (!value_type.has_value())
                        return;
                    style_value->value_type = value_type.release_value();
                } else if (color_red == BeginPosition) {
                    style_value->positions.append({});
                } else if (color_red == PositionX) {
                    VERIFY(!style_value->positions.is_empty());
                    style_value->positions.last().x = position_component_from_callback_payload();
                    if (style_value->positions.last().x.offset.has_value())
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::PositionXOffset;
                } else if (color_red == PositionY) {
                    VERIFY(!style_value->positions.is_empty());
                    style_value->positions.last().y = position_component_from_callback_payload();
                    if (style_value->positions.last().y.offset.has_value())
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::PositionYOffset;
                } else {
                    VERIFY(color_red == LonghandComponent);
                    style_value->position_components.append(position_component_from_callback_payload());
                    if (style_value->position_components.last().offset.has_value())
                        style_value->last_calculation_node_target = RustCalculationNodeTarget::PositionComponentOffset;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::PositionAnchor) {
                value.position_anchor_kind = static_cast<FFI::CssPositionAnchorValueKind>(color_red);
                value.position_anchor_name = fly_string_from_ffi_bytes(value_ptr, value_len);
            } else if (kind == FFI::CssStyleValueKind::PositionTryOrder) {
                value.position_try_order = static_cast<FFI::CssPositionTryOrderValue>(color_red);
            } else if (kind == FFI::CssStyleValueKind::PositionVisibility) {
                value.position_visibility = FFI::CssPositionVisibilityValue {
                    .kind = static_cast<FFI::CssPositionVisibilityValueKind>(color_red),
                    .has_anchors_valid = static_cast<bool>(color_green & 1),
                    .has_anchors_visible = static_cast<bool>(color_green & 2),
                    .has_no_overflow = static_cast<bool>(color_green & 4),
                };
            } else if (kind == FFI::CssStyleValueKind::Quotes) {
                value.quotes_kind = static_cast<FFI::CssQuotesValueKind>(color_red);
                for (auto string : StringView { value_ptr, value_len }.split_view('\0'))
                    value.quotes_strings.append(FlyString::from_utf8_without_validation(string.bytes()));
            } else if (kind == FFI::CssStyleValueKind::BackgroundSize) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::BackgroundSize);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->last_calculation_node_target = RustCalculationNodeTarget::None;
                enum : u8 {
                    Keyword,
                    Width,
                    Height,
                };
                if (color_red == Keyword) {
                    auto keyword = keyword_from_string({ value_ptr, value_len });
                    if (!keyword.has_value())
                        return;
                    style_value->background_sizes.append({
                        .keyword = keyword.release_value(),
                    });
                } else if (color_red == Width) {
                    style_value->background_sizes.append({
                        .width = nested_primitive_value_from_callback_payload(),
                    });
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::BackgroundSizeWidth;
                } else {
                    VERIFY(color_red == Height);
                    VERIFY(!style_value->background_sizes.is_empty());
                    VERIFY(style_value->background_sizes.last().width.has_value());
                    VERIFY(!style_value->background_sizes.last().keyword.has_value());
                    style_value->background_sizes.last().height = nested_primitive_value_from_callback_payload();
                    style_value->last_calculation_node_target = RustCalculationNodeTarget::BackgroundSizeHeight;
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::RepeatStyle) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::RepeatStyle);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                style_value->repeat_x_values.append(color_red);
                style_value->repeat_y_values.append(color_green);
                return;
            } else if (kind == FFI::CssStyleValueKind::ScrollbarColor) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::ScrollbarColor);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                auto component_kind = primitive_kind == FFI::CssPrimitiveValueKind::Invalid && has_numeric_value && has_secondary_numeric_value ? static_cast<u8>(numeric_value) : color_red;
                if (component_kind == 1) {
                    style_value->scrollbar_color_kind = 1;
                } else if (component_kind == 2) {
                    style_value->scrollbar_color_kind = 2;
                    style_value->scrollbar_thumb_color = style_color_from_callback_payload(has_numeric_value, secondary_numeric_value, color_red, color_green, color_blue, color_alpha, value_ptr, value_len);
                    note_style_color_source_component_target(style_value->scrollbar_thumb_color);
                } else if (component_kind == 3) {
                    style_value->scrollbar_color_kind = 2;
                    style_value->scrollbar_track_color = style_color_from_callback_payload(has_numeric_value, secondary_numeric_value, color_red, color_green, color_blue, color_alpha, value_ptr, value_len);
                    note_style_color_source_component_target(style_value->scrollbar_track_color);
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::ScrollbarGutter) {
                value.scrollbar_gutter = static_cast<FFI::CssScrollbarGutterValueKind>(color_red);
            } else if (kind == FFI::CssStyleValueKind::ScrollTimeline) {
                for (size_t offset = 0; offset < value_len;) {
                    auto item_kind = static_cast<FFI::CssTimelineNameItemKind>(value_ptr[offset++]);
                    auto name_start = offset;
                    while (offset < value_len && value_ptr[offset] != 0)
                        ++offset;
                    value.timeline_name_item_kinds.append(item_kind);
                    value.timeline_names.append(FlyString::from_utf8_without_validation({ value_ptr + name_start, offset - name_start }));
                    if (offset < value_len)
                        ++offset;
                }
                value.scroll_timeline_axes.ensure_capacity(value_type_len);
                for (size_t i = 0; i < value_type_len; ++i)
                    value.scroll_timeline_axes.unchecked_append(static_cast<FFI::CssScrollFunctionAxisKind>(value_type_ptr[i]));
            } else if (kind == FFI::CssStyleValueKind::TextWrap) {
                value.text_wrap = FFI::CssTextWrapValue {
                    .kind = FFI::CssTextWrapValueKind::Valid,
                    .mode = static_cast<FFI::CssTextWrapModeValue>(color_red),
                    .style = static_cast<FFI::CssTextWrapStyleValue>(color_green),
                };
            } else if (kind == FFI::CssStyleValueKind::TextWrapMode) {
                value.text_wrap_mode = static_cast<FFI::CssTextWrapModeValue>(color_red);
            } else if (kind == FFI::CssStyleValueKind::TextWrapStyle) {
                value.text_wrap_style = static_cast<FFI::CssTextWrapStyleValue>(color_red);
            } else if (kind == FFI::CssStyleValueKind::TextIndent) {
                auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                if (!value_type.has_value())
                    return;
                value.value_type = value_type.release_value();
                value.text_indent_has_hanging = color_red != 0;
                value.text_indent_has_each_line = color_green != 0;
                value.text_indent = nested_primitive_value_from_callback_payload();
                value.last_calculation_node_target = RustCalculationNodeTarget::TextIndent;
            } else if (kind == FFI::CssStyleValueKind::TextUnderlinePosition) {
                value.text_underline_position_horizontal = static_cast<FFI::CssTextUnderlinePositionHorizontal>(color_red);
                value.text_underline_position_vertical = static_cast<FFI::CssTextUnderlinePositionVertical>(color_green);
            } else if (kind == FFI::CssStyleValueKind::TimelineName) {
                value.timeline_name_kind = static_cast<FFI::CssTimelineNameValueKind>(color_red);
                for (size_t offset = 0; offset < value_len;) {
                    auto item_kind = static_cast<FFI::CssTimelineNameItemKind>(value_ptr[offset++]);
                    auto name_start = offset;
                    while (offset < value_len && value_ptr[offset] != 0)
                        ++offset;
                    value.timeline_name_item_kinds.append(item_kind);
                    value.timeline_names.append(FlyString::from_utf8_without_validation({ value_ptr + name_start, offset - name_start }));
                    if (offset < value_len)
                        ++offset;
                }
            } else if (kind == FFI::CssStyleValueKind::TimelineScope) {
                value.timeline_scope_kind = static_cast<FFI::CssTimelineScopeValueKind>(color_red);
                for (auto name : StringView { value_ptr, value_len }.split_view('\0'))
                    value.timeline_scope_names.append(FlyString::from_utf8_without_validation(name.bytes()));
            } else if (kind == FFI::CssStyleValueKind::TouchAction) {
                value.touch_action = FFI::CssTouchActionValue {
                    .kind = static_cast<FFI::CssTouchActionValueKind>(color_red),
                    .first = static_cast<FFI::CssTouchActionKeyword>(color_green),
                    .second = static_cast<FFI::CssTouchActionKeyword>(color_blue),
                };
            } else if (kind == FFI::CssStyleValueKind::TransitionBehavior) {
                value.transition_behaviors.ensure_capacity(value_len);
                for (size_t i = 0; i < value_len; ++i)
                    value.transition_behaviors.unchecked_append(static_cast<FFI::CssTransitionBehaviorItemKind>(value_ptr[i]));
            } else if (kind == FFI::CssStyleValueKind::TransitionProperty) {
                value.transition_property_kind = static_cast<FFI::CssTransitionPropertyValueKind>(color_red);
                for (auto property : StringView { value_ptr, value_len }.split_view('\0'))
                    value.transition_properties.append(FlyString::from_utf8_without_validation(property.bytes()));
            } else if (kind == FFI::CssStyleValueKind::ViewTimeline) {
                enum : u8 {
                    Header,
                    InsetCount,
                    InsetAuto,
                    InsetLengthPercentage,
                };

                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::ViewTimeline);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == Header) {
                    for (size_t offset = 0; offset < value_len;) {
                        auto item_kind = static_cast<FFI::CssTimelineNameItemKind>(value_ptr[offset++]);
                        auto name_start = offset;
                        while (offset < value_len && value_ptr[offset] != 0)
                            ++offset;
                        style_value->timeline_name_item_kinds.append(item_kind);
                        style_value->timeline_names.append(FlyString::from_utf8_without_validation({ value_ptr + name_start, offset - name_start }));
                        if (offset < value_len)
                            ++offset;
                    }

                    if (value_type_len != style_value->timeline_names.size())
                        return;

                    style_value->scroll_timeline_axes.ensure_capacity(style_value->timeline_names.size());
                    for (size_t i = 0; i < style_value->timeline_names.size(); ++i)
                        style_value->scroll_timeline_axes.unchecked_append(static_cast<FFI::CssScrollFunctionAxisKind>(value_type_ptr[i]));
                } else if (color_red == InsetCount) {
                    style_value->view_timeline_inset_counts.append(color_green);
                } else if (color_red == InsetAuto) {
                    style_value->view_timeline_insets.append({ .is_auto = true });
                } else {
                    VERIFY(color_red == InsetLengthPercentage);
                    style_value->view_timeline_insets.append({ .length_percentage = nested_primitive_value_from_callback_payload() });
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::ViewTimelineInset) {
                enum : u8 {
                    Auto,
                    LengthPercentage,
                    InsetCount,
                };

                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::ViewTimelineInset);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }

                if (color_red == InsetCount) {
                    style_value->view_timeline_inset_counts.append(color_green);
                } else if (color_red == Auto) {
                    style_value->view_timeline_insets.append({ .is_auto = true });
                } else {
                    VERIFY(color_red == LengthPercentage);
                    style_value->view_timeline_insets.append({ .length_percentage = nested_primitive_value_from_callback_payload() });
                }
                return;
            } else if (kind == FFI::CssStyleValueKind::ViewFunction) {
                value.scroll_function_axis = static_cast<FFI::CssScrollFunctionAxisKind>(color_red);
                value.view_function_inset = static_cast<FFI::CssViewFunctionInsetKind>(color_green);
                value.view_function_inset_position = static_cast<FFI::CssViewFunctionInsetPosition>(color_blue);
            } else if (kind == FFI::CssStyleValueKind::ViewTransitionName) {
                value.view_transition_name_kind = static_cast<FFI::CssViewTransitionNameValueKind>(color_red);
                value.view_transition_name = fly_string_from_ffi_bytes(value_ptr, value_len);
            } else if (kind == FFI::CssStyleValueKind::WhiteSpace) {
                value.white_space_collapse = fly_string_from_ffi_bytes(value_ptr, value_len);
                value.text_wrap_mode = static_cast<FFI::CssTextWrapModeValue>(color_red);
                value.white_space_trim = FFI::CssWhiteSpaceTrimValue {
                    .kind = static_cast<FFI::CssWhiteSpaceTrimValueKind>(color_green),
                    .has_discard_before = static_cast<bool>(color_blue & 1),
                    .has_discard_after = static_cast<bool>(color_blue & 2),
                    .has_discard_inner = static_cast<bool>(color_blue & 4),
                };
            } else if (kind == FFI::CssStyleValueKind::WhiteSpaceTrim) {
                value.white_space_trim = FFI::CssWhiteSpaceTrimValue {
                    .kind = static_cast<FFI::CssWhiteSpaceTrimValueKind>(color_red),
                    .has_discard_before = static_cast<bool>(color_green & 1),
                    .has_discard_after = static_cast<bool>(color_green & 2),
                    .has_discard_inner = static_cast<bool>(color_green & 4),
                };
            } else if (kind == FFI::CssStyleValueKind::WillChange) {
                value.will_change_kind = static_cast<FFI::CssWillChangeValueKind>(color_red);
                for (size_t offset = 0; offset < value_len;) {
                    auto feature_kind = static_cast<FFI::CssWillChangeFeatureKind>(value_ptr[offset++]);
                    auto feature_start = offset;
                    while (offset < value_len && value_ptr[offset] != 0)
                        ++offset;
                    value.will_change_feature_kinds.append(feature_kind);
                    value.will_change_features.append(FlyString::from_utf8_without_validation({ value_ptr + feature_start, offset - feature_start }));
                    if (offset < value_len)
                        ++offset;
                }
            } else if (kind == FFI::CssStyleValueKind::GeneratedValueList) {
                if (!style_value.has_value())
                    style_value = move(value);
                else {
                    VERIFY(style_value->kind == FFI::CssStyleValueKind::GeneratedValueList);
                    VERIFY(style_value->property_id == static_cast<PropertyID>(property_id));
                }
                auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                if (!value_type.has_value())
                    return;
                style_value->generated_value_list_value_types.append(value_type.release_value());
                return;
            } else if (first_is_one_of(kind, FFI::CssStyleValueKind::Anchor, FFI::CssStyleValueKind::AnchorSize, FFI::CssStyleValueKind::Primitive, FFI::CssStyleValueKind::MathFunction, FFI::CssStyleValueKind::TreeCountingFunction)) {
                auto value_type = value_type_from_rust_property_value_type_name({ value_type_ptr, value_type_len });
                if (!value_type.has_value())
                    return;
                value.value_type = value_type.release_value();
                if ((kind == FFI::CssStyleValueKind::Anchor || kind == FFI::CssStyleValueKind::AnchorSize || kind == FFI::CssStyleValueKind::MathFunction) && value_len > 0)
                    value.string = fly_string_from_ffi_bytes(value_ptr, value_len);
                if (kind == FFI::CssStyleValueKind::TreeCountingFunction) {
                    if (color_red == static_cast<u8>(RustTreeCountingFunction::SiblingCount))
                        value.tree_counting_function = RustTreeCountingFunction::SiblingCount;
                    else if (color_red == static_cast<u8>(RustTreeCountingFunction::SiblingIndex))
                        value.tree_counting_function = RustTreeCountingFunction::SiblingIndex;
                    else
                        return;
                }
                if (has_numeric_value)
                    value.numeric_value = numeric_value;
                if (has_secondary_numeric_value)
                    value.secondary_numeric_value = secondary_numeric_value;
                if (primitive_kind == FFI::CssPrimitiveValueKind::Keyword) {
                    auto keyword = keyword_from_string({ value_ptr, value_len });
                    if (!keyword.has_value())
                        return;
                    value.keyword = keyword.release_value();
                } else if (primitive_kind == FFI::CssPrimitiveValueKind::CustomIdent) {
                    value.custom_ident = fly_string_from_ffi_bytes(value_ptr, value_len);
                } else if (primitive_kind == FFI::CssPrimitiveValueKind::String) {
                    value.string = fly_string_from_ffi_bytes(value_ptr, value_len);
                } else if (first_is_one_of(primitive_kind,
                               FFI::CssPrimitiveValueKind::Angle,
                               FFI::CssPrimitiveValueKind::Flex,
                               FFI::CssPrimitiveValueKind::Frequency,
                               FFI::CssPrimitiveValueKind::Length,
                               FFI::CssPrimitiveValueKind::Resolution,
                               FFI::CssPrimitiveValueKind::Time)) {
                    value.dimension_unit = fly_string_from_ffi_bytes(value_ptr, value_len);
                } else if (primitive_kind == FFI::CssPrimitiveValueKind::Ratio) {
                    value.ratio_has_denominator = StringView { value_ptr, value_len } == "has-denominator"sv;
                }
                value.source_component_values = move(context.pending_nested_primitive_source_component_values);
            }

            style_value = move(value);
        },
        [](void* raw_style_value, FFI::CssCalculationNodeKind kind, FFI::CssPrimitiveValueKind primitive_kind, bool has_numeric_value, double numeric_value, u32 child_count, u8 const* metadata_ptr, size_t metadata_len) {
            auto& context = *static_cast<StyleValueParseContext*>(raw_style_value);
            context.flush_source_component_values();
            auto& style_value = context.style_value;
            VERIFY(style_value.has_value());
            auto event = RustCalculationNodeEvent {
                .kind = kind,
                .primitive_kind = primitive_kind,
                .numeric_value = has_numeric_value ? Optional<double> { numeric_value } : Optional<double> {},
                .child_count = child_count,
                .metadata = string_from_ffi_bytes(metadata_ptr, metadata_len),
            };
            if (style_value->kind == FFI::CssStyleValueKind::CoordinatingValueListShorthand) {
                VERIFY(!style_value->coordinating_value_list_shorthand_items.is_empty());
                auto& item = style_value->coordinating_value_list_shorthand_items.last();
                switch (item.last_calculation_node_target) {
                case RustCalculationNodeTarget::EasingFunctionValue:
                    VERIFY(!item.easing_function_values.is_empty());
                    item.easing_function_values.last().calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::LinearEasingOutput:
                    VERIFY(!item.linear_easing_stops.is_empty());
                    item.linear_easing_stops.last().output.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::LinearEasingFirstStopLength:
                    VERIFY(!item.linear_easing_stops.is_empty());
                    VERIFY(item.linear_easing_stops.last().first_stop_length.has_value());
                    item.linear_easing_stops.last().first_stop_length->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::LinearEasingSecondStopLength:
                    VERIFY(!item.linear_easing_stops.is_empty());
                    VERIFY(item.linear_easing_stops.last().second_stop_length.has_value());
                    item.linear_easing_stops.last().second_stop_length->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
                style_value->coordinating_value_list_shorthand_items.last().calculation_node_events.append(move(event));
                return;
            }
            if (style_value->kind == FFI::CssStyleValueKind::ComponentShorthand) {
                VERIFY(!style_value->component_shorthand_items.is_empty());
                style_value->component_shorthand_items.last().calculation_node_events.append(move(event));
                return;
            }
            if (style_value->kind == FFI::CssStyleValueKind::Flex) {
                switch (style_value->last_flex_calculation_component) {
                case RustFlexCalculationComponent::Grow:
                    style_value->flex_grow_calculation_node_events.append(move(event));
                    return;
                case RustFlexCalculationComponent::Shrink:
                    style_value->flex_shrink_calculation_node_events.append(move(event));
                    return;
                case RustFlexCalculationComponent::Basis:
                    style_value->flex_basis_calculation_node_events.append(move(event));
                    return;
                case RustFlexCalculationComponent::None:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::BackgroundSize) {
                VERIFY(!style_value->background_sizes.is_empty());
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::BackgroundSizeWidth:
                    VERIFY(style_value->background_sizes.last().width.has_value());
                    style_value->background_sizes.last().width->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::BackgroundSizeHeight:
                    VERIFY(style_value->background_sizes.last().height.has_value());
                    style_value->background_sizes.last().height->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::Position) {
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::PositionXOffset:
                    VERIFY(!style_value->positions.is_empty());
                    VERIFY(style_value->positions.last().x.offset.has_value());
                    style_value->positions.last().x.offset->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::PositionYOffset:
                    VERIFY(!style_value->positions.is_empty());
                    VERIFY(style_value->positions.last().y.offset.has_value());
                    style_value->positions.last().y.offset->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::PositionComponentOffset:
                    VERIFY(!style_value->position_components.is_empty());
                    VERIFY(style_value->position_components.last().offset.has_value());
                    style_value->position_components.last().offset->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::StrokeDasharray) {
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::StrokeDasharray:
                    VERIFY(!style_value->stroke_dasharray_values.is_empty());
                    style_value->stroke_dasharray_values.last().calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (first_is_one_of(style_value->kind, FFI::CssStyleValueKind::BorderRadius, FFI::CssStyleValueKind::Border, FFI::CssStyleValueKind::BorderImage, FFI::CssStyleValueKind::BorderImageSlice, FFI::CssStyleValueKind::BorderImageOutset, FFI::CssStyleValueKind::BorderImageWidth)) {
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::BorderRadiusHorizontal:
                    VERIFY(!style_value->border_radius_horizontal_radii.is_empty());
                    style_value->border_radius_horizontal_radii.last().calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::BorderRadiusVertical:
                    VERIFY(!style_value->border_radius_vertical_radii.is_empty());
                    style_value->border_radius_vertical_radii.last().calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::BorderWidthLength:
                    VERIFY(style_value->border_width_length.has_value());
                    style_value->border_width_length->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::BorderImageSlice:
                    VERIFY(!style_value->border_image_slices.is_empty());
                    style_value->border_image_slices.last().calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::BorderImageWidth:
                    VERIFY(!style_value->border_image_widths.is_empty());
                    style_value->border_image_widths.last().value.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::BorderImageOutset:
                    VERIFY(!style_value->border_image_outsets.is_empty());
                    style_value->border_image_outsets.last().value.calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (first_is_one_of(style_value->kind, FFI::CssStyleValueKind::TransformOrigin, FFI::CssStyleValueKind::TransformLonghand, FFI::CssStyleValueKind::Transformation, FFI::CssStyleValueKind::Shadow, FFI::CssStyleValueKind::FilterValueList)) {
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::TransformOriginX:
                    VERIFY(style_value->transform_origin_x.has_value());
                    style_value->transform_origin_x->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::TransformOriginY:
                    VERIFY(style_value->transform_origin_y.has_value());
                    style_value->transform_origin_y->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::TransformOriginZ:
                    VERIFY(style_value->transform_origin_z.has_value());
                    style_value->transform_origin_z->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::TransformLonghandArgument:
                    VERIFY(!style_value->transform_longhand_arguments.is_empty());
                    style_value->transform_longhand_arguments.last().value.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::TransformationArgument:
                    VERIFY(!style_value->transformations.is_empty());
                    VERIFY(!style_value->transformations.last().arguments.is_empty());
                    style_value->transformations.last().arguments.last().value.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::ShadowOffsetX:
                    VERIFY(!style_value->shadows.is_empty());
                    style_value->shadows.last().offset_x.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::ShadowOffsetY:
                    VERIFY(!style_value->shadows.is_empty());
                    style_value->shadows.last().offset_y.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::ShadowBlurRadius:
                    VERIFY(!style_value->shadows.is_empty());
                    VERIFY(style_value->shadows.last().blur_radius.has_value());
                    style_value->shadows.last().blur_radius->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::ShadowSpreadDistance:
                    VERIFY(!style_value->shadows.is_empty());
                    VERIFY(style_value->shadows.last().spread_distance.has_value());
                    style_value->shadows.last().spread_distance->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::FilterValue:
                    VERIFY(!style_value->filter_value_list_events.is_empty());
                    style_value->filter_value_list_events.last().value.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::FilterDropShadowRadius:
                    VERIFY(!style_value->filter_value_list_events.is_empty());
                    VERIFY(style_value->filter_value_list_events.last().drop_shadow_radius.has_value());
                    style_value->filter_value_list_events.last().drop_shadow_radius->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::EasingFunction) {
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::EasingFunctionValue:
                    VERIFY(!style_value->easing_function_values.is_empty());
                    style_value->easing_function_values.last().calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::LinearEasingOutput:
                    VERIFY(!style_value->linear_easing_stops.is_empty());
                    style_value->linear_easing_stops.last().output.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::LinearEasingFirstStopLength:
                    VERIFY(!style_value->linear_easing_stops.is_empty());
                    VERIFY(style_value->linear_easing_stops.last().first_stop_length.has_value());
                    style_value->linear_easing_stops.last().first_stop_length->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::LinearEasingSecondStopLength:
                    VERIFY(!style_value->linear_easing_stops.is_empty());
                    VERIFY(style_value->linear_easing_stops.last().second_stop_length.has_value());
                    style_value->linear_easing_stops.last().second_stop_length->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::GridTemplateShorthand) {
                VERIFY(!style_value->grid_template_shorthand_items.is_empty());
                auto& item = style_value->grid_template_shorthand_items.last();
                VERIFY(!item.grid_track_size_list_events.is_empty());
                switch (item.last_calculation_node_target) {
                case RustCalculationNodeTarget::GridTrackValue:
                    item.grid_track_size_list_events.last().value.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::GridTrackSecondaryValue:
                    item.grid_track_size_list_events.last().secondary_value.calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (first_is_one_of(style_value->kind, FFI::CssStyleValueKind::GridAutoTrackSizes, FFI::CssStyleValueKind::GridTrackSizeList)) {
                VERIFY(!style_value->grid_track_size_list_events.is_empty());
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::GridTrackValue:
                    style_value->grid_track_size_list_events.last().value.calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::GridTrackSecondaryValue:
                    style_value->grid_track_size_list_events.last().secondary_value.calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::TextIndent) {
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::TextIndent:
                    VERIFY(style_value->text_indent.has_value());
                    style_value->text_indent->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::CornerShape) {
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::CornerShapeParameter:
                    VERIFY(style_value->corner_shape_superellipse_parameter.has_value());
                    style_value->corner_shape_superellipse_parameter->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (first_is_one_of(style_value->kind, FFI::CssStyleValueKind::BasicShape, FFI::CssStyleValueKind::ShapeOutside, FFI::CssStyleValueKind::FitContent)) {
                switch (style_value->last_calculation_node_target) {
                case RustCalculationNodeTarget::BasicShapeRadius:
                    if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                        VERIFY(!style_value->shape_outside_basic_shape_radial_shape_radius.is_empty());
                        style_value->shape_outside_basic_shape_radial_shape_radius.last().length_percentage.calculation_node_events.append(move(event));
                    } else {
                        VERIFY(!style_value->basic_shape_radial_shape_radius.is_empty());
                        style_value->basic_shape_radial_shape_radius.last().length_percentage.calculation_node_events.append(move(event));
                    }
                    return;
                case RustCalculationNodeTarget::BasicShapePositionX:
                    if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                        VERIFY(style_value->shape_outside_basic_shape_radial_shape_position.has_value());
                        VERIFY(style_value->shape_outside_basic_shape_radial_shape_position->x.offset.has_value());
                        style_value->shape_outside_basic_shape_radial_shape_position->x.offset->calculation_node_events.append(move(event));
                    } else {
                        VERIFY(style_value->basic_shape_radial_shape_position.has_value());
                        VERIFY(style_value->basic_shape_radial_shape_position->x.offset.has_value());
                        style_value->basic_shape_radial_shape_position->x.offset->calculation_node_events.append(move(event));
                    }
                    return;
                case RustCalculationNodeTarget::BasicShapePositionY:
                    if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                        VERIFY(style_value->shape_outside_basic_shape_radial_shape_position.has_value());
                        VERIFY(style_value->shape_outside_basic_shape_radial_shape_position->y.offset.has_value());
                        style_value->shape_outside_basic_shape_radial_shape_position->y.offset->calculation_node_events.append(move(event));
                    } else {
                        VERIFY(style_value->basic_shape_radial_shape_position.has_value());
                        VERIFY(style_value->basic_shape_radial_shape_position->y.offset.has_value());
                        style_value->basic_shape_radial_shape_position->y.offset->calculation_node_events.append(move(event));
                    }
                    return;
                case RustCalculationNodeTarget::BasicShapeRectangleComponent:
                    if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                        VERIFY(!style_value->shape_outside_basic_shape_rectangle_components.is_empty());
                        style_value->shape_outside_basic_shape_rectangle_components.last().value.calculation_node_events.append(move(event));
                    } else {
                        VERIFY(!style_value->basic_shape_rectangle_components.is_empty());
                        style_value->basic_shape_rectangle_components.last().value.calculation_node_events.append(move(event));
                    }
                    return;
                case RustCalculationNodeTarget::BasicShapeRectangleRadiusHorizontal:
                    if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                        VERIFY(!style_value->shape_outside_basic_shape_rectangle_border_radius_horizontal_radii.is_empty());
                        style_value->shape_outside_basic_shape_rectangle_border_radius_horizontal_radii.last().calculation_node_events.append(move(event));
                    } else {
                        VERIFY(!style_value->basic_shape_rectangle_border_radius_horizontal_radii.is_empty());
                        style_value->basic_shape_rectangle_border_radius_horizontal_radii.last().calculation_node_events.append(move(event));
                    }
                    return;
                case RustCalculationNodeTarget::BasicShapeRectangleRadiusVertical:
                    if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                        VERIFY(!style_value->shape_outside_basic_shape_rectangle_border_radius_vertical_radii.is_empty());
                        style_value->shape_outside_basic_shape_rectangle_border_radius_vertical_radii.last().calculation_node_events.append(move(event));
                    } else {
                        VERIFY(!style_value->basic_shape_rectangle_border_radius_vertical_radii.is_empty());
                        style_value->basic_shape_rectangle_border_radius_vertical_radii.last().calculation_node_events.append(move(event));
                    }
                    return;
                case RustCalculationNodeTarget::BasicShapePolygonCoordinate:
                    if (style_value->kind == FFI::CssStyleValueKind::ShapeOutside) {
                        VERIFY(!style_value->shape_outside_basic_shape_polygon_coordinates.is_empty());
                        style_value->shape_outside_basic_shape_polygon_coordinates.last().calculation_node_events.append(move(event));
                    } else {
                        VERIFY(!style_value->basic_shape_polygon_coordinates.is_empty());
                        style_value->basic_shape_polygon_coordinates.last().calculation_node_events.append(move(event));
                    }
                    return;
                case RustCalculationNodeTarget::FitContentArgument:
                    VERIFY(style_value->fit_content_argument.has_value());
                    style_value->fit_content_argument->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::LayerShorthand) {
                VERIFY(!style_value->layer_shorthand_items.is_empty());
                auto& item = style_value->layer_shorthand_items.last();
                switch (item.last_calculation_node_target) {
                case RustCalculationNodeTarget::BackgroundSizeWidth:
                    VERIFY(!item.background_sizes.is_empty());
                    VERIFY(item.background_sizes.last().width.has_value());
                    item.background_sizes.last().width->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::BackgroundSizeHeight:
                    VERIFY(!item.background_sizes.is_empty());
                    VERIFY(item.background_sizes.last().height.has_value());
                    item.background_sizes.last().height->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::PositionXOffset:
                    VERIFY(!item.positions.is_empty());
                    VERIFY(item.positions.last().x.offset.has_value());
                    item.positions.last().x.offset->calculation_node_events.append(move(event));
                    return;
                case RustCalculationNodeTarget::PositionYOffset:
                    VERIFY(!item.positions.is_empty());
                    VERIFY(item.positions.last().y.offset.has_value());
                    item.positions.last().y.offset->calculation_node_events.append(move(event));
                    return;
                default:
                    break;
                }
            }
            if (style_value->kind == FFI::CssStyleValueKind::PositionalValueListShorthand) {
                VERIFY(!style_value->positional_value_list_shorthand_items.is_empty());
                style_value->positional_value_list_shorthand_items.last().calculation_node_events.append(move(event));
                return;
            }
            style_value->calculation_node_events.append(move(event));
        },
        [](void* raw_style_value, FFI::CssUrlModifier const* rust_modifier) {
            auto& context = *static_cast<StyleValueParseContext*>(raw_style_value);
            context.flush_source_component_values();
            auto& style_value = context.style_value;
            VERIFY(style_value.has_value());

            RequestURLModifier modifier = [&]() {
                switch (rust_modifier->kind) {
                case FFI::CssUrlModifierKind::CrossOrigin:
                    return RequestURLModifier::create_cross_origin(cross_origin_modifier_value_from_rust(rust_modifier->cross_origin_value));
                case FFI::CssUrlModifierKind::Integrity:
                    return RequestURLModifier::create_integrity(fly_string_from_ffi_bytes(rust_modifier->integrity_ptr, rust_modifier->integrity_len));
                case FFI::CssUrlModifierKind::ReferrerPolicy:
                    return RequestURLModifier::create_referrer_policy(referrer_policy_modifier_value_from_rust(rust_modifier->referrer_policy_value));
                }
                VERIFY_NOT_REACHED();
            }();

            VERIFY(style_value->kind == FFI::CssStyleValueKind::FilterValueList);
            VERIFY(!style_value->filter_value_list_events.is_empty());
            VERIFY(style_value->filter_value_list_events.last().kind == RustFilterValueListEventKind::Url);
            style_value->filter_value_list_events.last().request_url_modifiers.append(move(modifier));
        },
        [](void* raw_style_value, u8 kind) {
            auto& context = *static_cast<StyleValueParseContext*>(raw_style_value);
            context.start_source_component_values(kind);
        },
        [](void* raw_style_value, FFI::CssComponentValue const* component_value) {
            auto& context = *static_cast<StyleValueParseContext*>(raw_style_value);
            VERIFY(context.source_component_value_target != StyleValueParseContext::SourceComponentValueTarget::None);
            if (context.source_component_value_target == StyleValueParseContext::SourceComponentValueTarget::Discard)
                return;
            append_component_value_token(context.source_component_value_builder, component_value->kind, RustTokenizer::token_from_ffi(component_value->token));
        });

    context.flush_source_component_values();
    return context.style_value;
}

Optional<RustComponentValueParser::PropertyNumericMetadata> RustComponentValueParser::property_numeric_metadata(ReadonlySpan<PropertyID> property_ids, ValueType value_type)
{
    Vector<u16, 4> ffi_property_ids;
    for (auto property_id : property_ids)
        ffi_property_ids.append(static_cast<u16>(to_underlying(property_id)));

    Optional<PropertyNumericMetadata> metadata;
    auto value_type_string = value_type_to_string(value_type);
    auto value_type_bytes = value_type_string.bytes();
    FFI::rust_css_property_numeric_metadata(
        ffi_property_ids.data(),
        ffi_property_ids.size(),
        value_type_bytes.data(),
        value_type_bytes.size(),
        &metadata,
        [](void* raw_metadata, u16 property_id, double minimum, double maximum, bool has_percentage_range, double percentage_minimum, double percentage_maximum, bool percentages_resolve_to_value_type) {
            auto& metadata = *static_cast<Optional<PropertyNumericMetadata>*>(raw_metadata);
            metadata = PropertyNumericMetadata {
                .property_id = static_cast<PropertyID>(property_id),
                .range = { minimum, maximum },
                .percentage_range = has_percentage_range ? Optional<NumericRange> { { percentage_minimum, percentage_maximum } } : Optional<NumericRange> {},
                .percentages_resolve_to_value_type = percentages_resolve_to_value_type,
            };
        });

    return metadata;
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

bool RustComponentValueParser::syntax_matches(StringView input, StringView syntax, LimitSingleComponentIdentToCustomIdent limit_single_component_ident_to_custom_ident)
{
    auto filtered_input = decode_and_filter_code_points(input, "utf-8"sv);
    auto filtered_syntax = decode_and_filter_code_points(syntax, "utf-8"sv);
    return FFI::rust_css_syntax_matches(
        filtered_input.bytes().data(),
        filtered_input.bytes().size(),
        filtered_syntax.bytes().data(),
        filtered_syntax.bytes().size(),
        limit_single_component_ident_to_custom_ident == LimitSingleComponentIdentToCustomIdent::Yes);
}

Optional<RustComponentValueParser::SyntaxComponent> RustComponentValueParser::parse_syntax_component(StringView input, StringView encoding, LimitSingleComponentIdentToCustomIdent limit_single_component_ident_to_custom_ident)
{
    RustSyntaxNodeBuilder builder;
    builder.ident_case_sensitivity = limit_single_component_ident_to_custom_ident == LimitSingleComponentIdentToCustomIdent::Yes ? CaseSensitivity::CaseSensitive : CaseSensitivity::CaseInsensitive;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto consumed_byte_length = FFI::rust_css_parse_syntax_component_prefix(
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
    if (builder.invalid || consumed_byte_length == 0)
        return {};
    return SyntaxComponent { move(builder.root), consumed_byte_length };
}

Optional<RustComponentValueParser::SyntaxComponent> RustComponentValueParser::parse_css_type(StringView input, StringView encoding, LimitSingleComponentIdentToCustomIdent limit_single_component_ident_to_custom_ident)
{
    RustSyntaxNodeBuilder builder;
    builder.ident_case_sensitivity = limit_single_component_ident_to_custom_ident == LimitSingleComponentIdentToCustomIdent::Yes ? CaseSensitivity::CaseSensitive : CaseSensitivity::CaseInsensitive;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto consumed_byte_length = FFI::rust_css_parse_css_type_prefix(
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
    if (builder.invalid || consumed_byte_length == 0)
        return {};
    return SyntaxComponent { move(builder.root), consumed_byte_length };
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
    set_original_value_text(*builder.declaration);
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
    set_original_value_text(*builder.declaration);
    return builder.declaration;
}

struct RustMediaFeatureTestBuilder {
    FFI::CssMediaFeature feature;
    Optional<FFI::CssMediaFeatureValueSyntaxKind> value_syntax_kind;
    Optional<FFI::CssMediaFeatureValueSyntaxKind> left_value_syntax_kind;
    Optional<FFI::CssMediaFeatureValueSyntaxKind> right_value_syntax_kind;
    Optional<MediaFeatureValue> value;
    Optional<MediaFeatureValue> left_value;
    Optional<MediaFeatureValue> right_value;
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
            .value = {
                .syntax_kind = value_syntax_kind.value_or(FFI::CssMediaFeatureValueSyntaxKind::Invalid),
                .parsed_value = move(value),
                .component_values = move(value_builder.root_values),
            },
            .left_value = {
                .syntax_kind = left_value_syntax_kind.value_or(FFI::CssMediaFeatureValueSyntaxKind::Invalid),
                .parsed_value = move(left_value),
                .component_values = move(left_value_builder.root_values),
            },
            .right_value = {
                .syntax_kind = right_value_syntax_kind.value_or(FFI::CssMediaFeatureValueSyntaxKind::Invalid),
                .parsed_value = move(right_value),
                .component_values = move(right_value_builder.root_values),
            },
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

static Optional<MediaFeatureValue> media_feature_value_from_rust(FFI::CssMediaFeatureValue const& value)
{
    switch (value.payload_kind) {
    case FFI::CssMediaFeatureValuePayloadKind::None:
        return {};
    case FFI::CssMediaFeatureValuePayloadKind::Ident: {
        auto keyword = keyword_from_string({ value.unit_or_ident_ptr, value.unit_or_ident_len });
        if (!keyword.has_value())
            return {};
        return MediaFeatureValue(MediaFeatureValue::Type::Ident, KeywordStyleValue::create(keyword.release_value()));
    }
    case FFI::CssMediaFeatureValuePayloadKind::Integer:
        if (value.numeric_value < AK::NumericLimits<i32>::min() || value.numeric_value > AK::NumericLimits<i32>::max())
            return {};
        return MediaFeatureValue(MediaFeatureValue::Type::Integer, IntegerStyleValue::create(static_cast<i32>(value.numeric_value)));
    case FFI::CssMediaFeatureValuePayloadKind::Length: {
        auto length_unit = string_to_length_unit({ value.unit_or_ident_ptr, value.unit_or_ident_len });
        if (!length_unit.has_value())
            return {};
        return MediaFeatureValue(MediaFeatureValue::Type::Length, LengthStyleValue::create(Length { value.numeric_value, length_unit.release_value() }));
    }
    case FFI::CssMediaFeatureValuePayloadKind::Ratio:
        return MediaFeatureValue(MediaFeatureValue::Type::Ratio, RatioStyleValue::create(NumberStyleValue::create(value.numeric_value), NumberStyleValue::create(value.secondary_numeric_value)));
    case FFI::CssMediaFeatureValuePayloadKind::Resolution: {
        auto resolution_unit = string_to_resolution_unit({ value.unit_or_ident_ptr, value.unit_or_ident_len });
        if (!resolution_unit.has_value())
            return {};
        return MediaFeatureValue(MediaFeatureValue::Type::Resolution, ResolutionStyleValue::create(Resolution { value.numeric_value, resolution_unit.release_value() }));
    }
    }
    VERIFY_NOT_REACHED();
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
    Optional<RustComponentValueParser::SupportsFeature> supports_feature;
    AK::Function<OwnPtr<BooleanExpression>(Optional<RustComponentValueParser::MediaFeatureTest>&&, Optional<RustComponentValueParser::SupportsFeature>&&, Vector<ComponentValue>&&)> parse_test;
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

        auto expression = parse_test(move(media_feature_test), move(supports_feature), move(component_value_builder.root_values));
        if (!expression && general_enclosed_fallback.has_value())
            expression = GeneralEnclosed::create(general_enclosed_fallback.release_value(), result_for_general_enclosed);
        append_expression(move(expression));
        component_value_builder = {};
        media_feature = {};
        supports_feature = {};
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
        builder.supports_feature = {};
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

static void set_boolean_expression_supports_feature(RustBooleanExpressionBuilder& builder, FFI::CssSupportsFeatureKind kind, u8 const* name_ptr, size_t name_len, u8 const* value_ptr, size_t value_len, bool important)
{
    Optional<FlyString> name;
    if (name_len > 0)
        name = fly_string_from_ffi_bytes(name_ptr, name_len);
    Optional<String> value;
    if (value_len > 0)
        value = string_from_ffi_bytes(value_ptr, value_len);
    builder.supports_feature = RustComponentValueParser::SupportsFeature {
        kind,
        move(name),
        move(value),
        important ? Important::Yes : Important::No,
    };
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

    auto set_resolved_value = [&](Optional<MediaFeatureValue>& target) {
        if (target.has_value())
            return;
        target = media_feature_value_from_rust(*media_feature_value);
    };

    auto append_to_builder = [&](ComponentValueBuilder& component_value_builder) {
        append_component_value_token(component_value_builder, media_feature_value->component_value.kind, RustTokenizer::token_from_ffi(media_feature_value->component_value.token));
    };

    switch (media_feature_value->kind) {
    case FFI::CssMediaFeatureValueKind::Value:
        set_media_feature_value_syntax_kind(builder.media_feature->value_syntax_kind, media_feature_value->syntax_kind);
        set_resolved_value(builder.media_feature->value);
        append_to_builder(builder.media_feature->value_builder);
        break;
    case FFI::CssMediaFeatureValueKind::LeftValue:
        set_media_feature_value_syntax_kind(builder.media_feature->left_value_syntax_kind, media_feature_value->syntax_kind);
        set_resolved_value(builder.media_feature->left_value);
        append_to_builder(builder.media_feature->left_value_builder);
        break;
    case FFI::CssMediaFeatureValueKind::RightValue:
        set_media_feature_value_syntax_kind(builder.media_feature->right_value_syntax_kind, media_feature_value->syntax_kind);
        set_resolved_value(builder.media_feature->right_value);
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
        [](void* raw_builder, FFI::CssSupportsFeatureKind kind, u8 const* name_ptr, size_t name_len, u8 const* value_ptr, size_t value_len, bool important) {
            auto& builder = *static_cast<RustBooleanExpressionBuilder*>(raw_builder);
            set_boolean_expression_supports_feature(builder, kind, name_ptr, name_len, value_ptr, value_len, important);
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

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_supports_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(Optional<SupportsFeature>&&, Vector<ComponentValue>&&)> parse_test)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::False,
        [parse_test = move(parse_test)](Optional<MediaFeatureTest>&&, Optional<SupportsFeature>&& supports_feature, Vector<ComponentValue>&& component_values) mutable {
            return parse_test(move(supports_feature), move(component_values));
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto supports_feature_callback, auto, auto, auto component_value_callback) {
            FFI::rust_css_parse_supports_condition(input, input_size, context, event_callback, supports_feature_callback, component_value_callback);
        });
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_an_if_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(Optional<MediaFeatureTest>&&, Optional<SupportsFeature>&&, Vector<ComponentValue>&&)> parse_test)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::False,
        move(parse_test),
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto supports_feature_callback, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
            FFI::rust_css_parse_if_condition(input, input_size, context, event_callback, supports_feature_callback, media_feature_callback, media_feature_value_callback, component_value_callback);
        });
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_container_condition(StringView input, StringView encoding)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::False,
        [](Optional<MediaFeatureTest>&&, Optional<SupportsFeature>&&, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
            return nullptr;
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
            FFI::rust_css_parse_media_condition(input, input_size, context, event_callback, media_feature_callback, media_feature_value_callback, component_value_callback);
        });
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_media_condition(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::Unknown,
        [parse_test = move(parse_test)](Optional<MediaFeatureTest>&& media_feature, Optional<SupportsFeature>&&, Vector<ComponentValue>&&) mutable -> OwnPtr<BooleanExpression> {
            if (!media_feature.has_value())
                return nullptr;
            return parse_test(media_feature.release_value());
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
            FFI::rust_css_parse_media_condition(input, input_size, context, event_callback, media_feature_callback, media_feature_value_callback, component_value_callback);
        });
}

OwnPtr<BooleanExpression> RustComponentValueParser::parse_a_media_test(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_test)
{
    return parse_a_boolean_expression(
        input,
        encoding,
        MatchResult::False,
        [parse_test = move(parse_test)](Optional<MediaFeatureTest>&& media_feature, Optional<SupportsFeature>&&, Vector<ComponentValue>&&) mutable -> OwnPtr<BooleanExpression> {
            if (!media_feature.has_value())
                return nullptr;
            return parse_test(media_feature.release_value());
        },
        [](u8 const* input, size_t input_size, void* context, auto event_callback, auto, auto media_feature_callback, auto media_feature_value_callback, auto component_value_callback) {
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
                .parse_test = [this](Optional<RustComponentValueParser::MediaFeatureTest>&& media_feature, Optional<RustComponentValueParser::SupportsFeature>&&, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
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

Optional<RustComponentValueParser::CounterFunction> RustComponentValueParser::parse_a_counter(StringView input, StringView encoding)
{
    Optional<CounterFunction> counter;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_counter(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &counter,
        [](void* raw_counter, u8 function, u8 const* name_ptr, size_t name_len, u8 const* join_string_ptr, size_t join_string_len) {
            auto& counter = *static_cast<Optional<CounterFunction>*>(raw_counter);
            counter = CounterFunction {
                .function = static_cast<RustCounterFunctionKind>(function),
                .name = fly_string_from_ffi_bytes(name_ptr, name_len),
                .join_string = join_string_len > 0 ? fly_string_from_ffi_bytes(join_string_ptr, join_string_len) : FlyString {},
            };
        },
        [](void* raw_counter, FFI::CssCounterStyleKind kind, FFI::CssCounterStyleSymbolsType symbols_type, u8 const* name_ptr, size_t name_len) {
            auto& counter = *static_cast<Optional<CounterFunction>*>(raw_counter);
            VERIFY(counter.has_value());
            counter->counter_style = CounterStyle {
                .kind = kind,
                .symbols_type = symbols_type,
                .name = fly_string_from_ffi_bytes(name_ptr, name_len),
                .symbols = {},
            };
        },
        [](void* raw_counter, u8 const* symbol_ptr, size_t symbol_len) {
            auto& counter = *static_cast<Optional<CounterFunction>*>(raw_counter);
            VERIFY(counter.has_value());
            VERIFY(counter->counter_style.has_value());
            counter->counter_style->symbols.append(fly_string_from_ffi_bytes(symbol_ptr, symbol_len));
        });

    if (!parsed || !counter.has_value())
        return {};

    return counter;
}

bool RustComponentValueParser::parse_optional_declaration_value_descriptor(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_optional_declaration_value_descriptor(filtered_input_bytes.data(), filtered_input_bytes.size());
}

RustComponentValueParser::ScrollFunction RustComponentValueParser::parse_scroll_function(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_scroll_function(
        filtered_input_bytes.data(),
        filtered_input_bytes.size());

    return {
        .kind = parsed.kind,
        .scroller = parsed.scroller,
        .axis = parsed.axis,
    };
}

RustComponentValueParser::ViewTimelineInset RustComponentValueParser::parse_view_timeline_inset_prefix(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_view_timeline_inset_prefix(
        filtered_input_bytes.data(),
        filtered_input_bytes.size());

    return {
        .kind = parsed.kind,
        .count = parsed.count,
    };
}

RustComponentValueParser::ViewFunction RustComponentValueParser::parse_view_function(StringView input, StringView encoding)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_view_function(
        filtered_input_bytes.data(),
        filtered_input_bytes.size());

    return {
        .kind = parsed.kind,
        .axis = parsed.axis,
        .inset = parsed.inset,
        .inset_position = parsed.inset_position,
    };
}

FFI::CssPrimitiveValueKind RustComponentValueParser::parse_primitive_value_prefix(StringView input, StringView encoding, FFI::CssPrimitiveValueType value_type, FFI::CssPrimitiveValueOptions options)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_primitive_value_prefix(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        value_type,
        options);
}

FFI::CssPrimitiveValueKind RustComponentValueParser::parse_primitive_value(StringView input, StringView encoding, FFI::CssPrimitiveValueType value_type, FFI::CssPrimitiveValueOptions options)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_primitive_value(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        value_type,
        options);
}

FFI::CssColorValueKind RustComponentValueParser::parse_color(StringView input, StringView encoding, bool allow_quirky_color)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    return FFI::rust_css_parse_color(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        allow_quirky_color);
}

Optional<RustComponentValueParser::SimpleColor> RustComponentValueParser::parse_simple_color(StringView input, StringView encoding, bool allow_quirky_color)
{
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    Optional<SimpleColor> color;
    FFI::rust_css_parse_simple_color(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        allow_quirky_color,
        &color,
        [](void* raw_color, FFI::CssParsedColorKind kind, u8 red, u8 green, u8 blue, u8 alpha, u8 const* name_ptr, size_t name_len) {
            auto& color = *static_cast<Optional<SimpleColor>*>(raw_color);
            color = SimpleColor {
                .kind = kind,
                .red = red,
                .green = green,
                .blue = blue,
                .alpha = alpha,
                .name = name_len > 0 ? Optional<FlyString> { fly_string_from_ffi_bytes(name_ptr, name_len) } : Optional<FlyString> {},
            };
        });

    return color;
}

Optional<Vector<u32>> RustComponentValueParser::parse_font_feature_values_feature_value(StringView input, StringView encoding)
{
    Vector<u32> values;
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    auto parsed = FFI::rust_css_parse_font_feature_values_feature_value(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &values,
        [](void* raw_values, u32 value) {
            auto& values = *static_cast<Vector<u32>*>(raw_values);
            values.append(value);
        });

    if (!parsed)
        return {};

    return values;
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
    Optional<RustBooleanExpressionBuilder> media_condition_builder;
    Optional<RustBooleanExpressionBuilder> supports_condition_builder;
    Optional<RustBooleanExpressionBuilder> container_condition_builder;
    AK::Function<OwnPtr<BooleanExpression>(RustComponentValueParser::MediaFeatureTest&&)> parse_media_feature_test;
    AK::Function<OwnPtr<BooleanExpression>(Optional<RustComponentValueParser::SupportsFeature>&&, Vector<ComponentValue>&&)> parse_supports_feature;
    AK::Function<bool(Declaration const&)> supports_declaration_is_supported;
    Optional<FlyString> current_page_selector_name;
    Vector<PagePseudoClass> current_page_selector_pseudo_classes;
    Optional<URL::Type> current_import_url_type;
    Optional<String> current_import_url;
    Vector<RequestURLModifier> current_import_url_modifiers;
    Optional<String> current_import_supports_declaration_source;

    RustBooleanExpressionBuilder* current_boolean_expression_builder()
    {
        if (media_condition_builder.has_value())
            return &*media_condition_builder;
        if (supports_condition_builder.has_value())
            return &*supports_condition_builder;
        if (container_condition_builder.has_value())
            return &*container_condition_builder;
        return nullptr;
    }

    void finish_media_condition()
    {
        if (!media_condition_builder.has_value())
            return;

        VERIFY(!stack.is_empty());
        auto& rule = stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        auto& media_query_list = at_rule.name.equals_ignoring_ascii_case("import"sv) ? at_rule.rust_import_media_query_list : at_rule.rust_media_query_list;
        VERIFY(media_query_list.has_value());
        VERIFY(!media_query_list->is_empty());

        auto& media_query = media_query_list->last();
        if (media_condition_builder->invalid || !media_condition_builder->stack.is_empty() || !media_condition_builder->root) {
            media_query->set_negated_for_parser(true);
            media_query->set_media_type_for_parser(MediaQuery::MediaType {
                .name = "all"_fly_string,
                .known_type = MediaQuery::KnownMediaType::All,
            });
            media_query->set_media_condition_for_parser(nullptr);
            media_condition_builder = {};
            return;
        }

        VERIFY(media_condition_builder->component_value_builder.stack.is_empty());
        media_query->set_media_condition_for_parser(media_condition_builder->root.release_nonnull());
        media_condition_builder = {};
    }

    void finish_media_query_list()
    {
        VERIFY(!stack.is_empty());
        auto& rule = stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        auto& media_query_list = at_rule.name.equals_ignoring_ascii_case("import"sv) ? at_rule.rust_import_media_query_list : at_rule.rust_media_query_list;
        if (!media_query_list.has_value())
            media_query_list = Vector<NonnullRefPtr<MediaQuery>> {};
        finish_media_condition();
    }

    void finish_supports_condition()
    {
        if (!supports_condition_builder.has_value())
            return;

        VERIFY(!stack.is_empty());
        auto& rule = stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        if (supports_condition_builder->invalid || !supports_condition_builder->stack.is_empty() || !supports_condition_builder->root) {
            supports_condition_builder = {};
            return;
        }

        VERIFY(supports_condition_builder->component_value_builder.stack.is_empty());
        auto supports = Supports::create(supports_condition_builder->root.release_nonnull());
        if (at_rule.name.equals_ignoring_ascii_case("import"sv))
            at_rule.rust_import_supports_condition = move(supports);
        else
            at_rule.rust_supports_condition = move(supports);
        supports_condition_builder = {};
    }

    void finish_container_condition()
    {
        if (!container_condition_builder.has_value())
            return;

        VERIFY(!stack.is_empty());
        auto& rule = stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        VERIFY(at_rule.rust_container_rule_prelude_conditions.has_value());
        VERIFY(!at_rule.rust_container_rule_prelude_conditions->is_empty());
        if (container_condition_builder->invalid || !container_condition_builder->stack.is_empty() || !container_condition_builder->root) {
            container_condition_builder = {};
            return;
        }

        VERIFY(container_condition_builder->component_value_builder.stack.is_empty());
        at_rule.rust_container_rule_prelude_conditions->last().query = ContainerQuery::create(container_condition_builder->root.release_nonnull());
        container_condition_builder = {};
    }

    void finish_import_url()
    {
        if (!current_import_url.has_value())
            return;

        VERIFY(!stack.is_empty());
        auto& rule = stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        VERIFY(current_import_url_type.has_value());
        at_rule.rust_import_url = URL { current_import_url.release_value(), current_import_url_type.release_value(), move(current_import_url_modifiers) };
        current_import_url_type = {};
        current_import_url_modifiers.clear();
    }

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
                .rust_layer_names = {},
                .rust_keyframes_name = {},
                .rust_namespace_prefix = {},
                .rust_namespace_uri = {},
                .rust_custom_property_name = {},
                .rust_counter_style_name = {},
                .rust_page_selectors = {},
                .rust_font_feature_values_family_names = {},
                .rust_container_rule_prelude_conditions = {},
                .rust_media_query_list = {},
                .rust_supports_condition = {},
                .rust_import_url = {},
                .rust_import_layer = {},
                .rust_import_supports_condition = {},
                .rust_import_media_query_list = {},
                .is_block_rule = event.is_block_rule,
            } },
        });
        break;
    case FFI::CssRuleEventKind::AtRuleEnd: {
        VERIFY(!builder.stack.is_empty());
        builder.finish_media_condition();
        builder.finish_supports_condition();
        builder.finish_container_condition();
        builder.finish_import_url();
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
                .rust_keyframe_selectors = {},
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
        set_original_value_text(declaration);
        builder.component_value_builder = {};
        builder.append_declaration(move(declaration));
        break;
    }
    case FFI::CssRuleEventKind::LayerName: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        if (!at_rule.rust_layer_names.has_value())
            at_rule.rust_layer_names = Vector<FlyString> {};
        at_rule.rust_layer_names->append(fly_string_from_ffi_bytes(event.name_ptr, event.name_len));
        break;
    }
    case FFI::CssRuleEventKind::KeyframesName: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        at_rule.rust_keyframes_name = fly_string_from_ffi_bytes(event.name_ptr, event.name_len);
        break;
    }
    case FFI::CssRuleEventKind::KeyframeSelector: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& qualified_rule = rule->get<QualifiedRule>();
        if (!qualified_rule.rust_keyframe_selectors.has_value())
            qualified_rule.rust_keyframe_selectors = Vector<Percentage> {};
        qualified_rule.rust_keyframe_selectors->append(Percentage(event.keyframe_selector));
        break;
    }
    case FFI::CssRuleEventKind::NamespacePrefix: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        at_rule.rust_namespace_prefix = fly_string_from_ffi_bytes(event.name_ptr, event.name_len);
        break;
    }
    case FFI::CssRuleEventKind::NamespaceUri: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        at_rule.rust_namespace_uri = fly_string_from_ffi_bytes(event.name_ptr, event.name_len);
        break;
    }
    case FFI::CssRuleEventKind::CustomPropertyName: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        at_rule.rust_custom_property_name = fly_string_from_ffi_bytes(event.name_ptr, event.name_len);
        break;
    }
    case FFI::CssRuleEventKind::CounterStyleName: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        at_rule.rust_counter_style_name = fly_string_from_ffi_bytes(event.name_ptr, event.name_len);
        break;
    }
    case FFI::CssRuleEventKind::PageSelectorList: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        at_rule.rust_page_selectors = PageSelectorList {};
        break;
    }
    case FFI::CssRuleEventKind::PageSelectorStart: {
        if (event.name_len > 0)
            builder.current_page_selector_name = fly_string_from_ffi_bytes(event.name_ptr, event.name_len);
        else
            builder.current_page_selector_name = {};
        builder.current_page_selector_pseudo_classes.clear();
        break;
    }
    case FFI::CssRuleEventKind::PageSelectorEnd: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        VERIFY(at_rule.rust_page_selectors.has_value());
        at_rule.rust_page_selectors->empend(move(builder.current_page_selector_name), move(builder.current_page_selector_pseudo_classes));
        break;
    }
    case FFI::CssRuleEventKind::PagePseudoClass:
        builder.current_page_selector_pseudo_classes.append(page_pseudo_class_from_rust(event.page_pseudo_class));
        break;
    case FFI::CssRuleEventKind::FontFeatureValuesFamilyName: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        if (!at_rule.rust_font_feature_values_family_names.has_value())
            at_rule.rust_font_feature_values_family_names = Vector<FlyString> {};
        at_rule.rust_font_feature_values_family_names->append(fly_string_from_ffi_bytes(event.name_ptr, event.name_len));
        break;
    }
    case FFI::CssRuleEventKind::ContainerCondition: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        if (!at_rule.rust_container_rule_prelude_conditions.has_value())
            at_rule.rust_container_rule_prelude_conditions = Vector<RustContainerRulePreludeCondition> {};
        at_rule.rust_container_rule_prelude_conditions->append({
            .name = event.name_len > 0 ? Optional<FlyString> { fly_string_from_ffi_bytes(event.name_ptr, event.name_len) } : OptionalNone {},
        });
        break;
    }
    case FFI::CssRuleEventKind::ContainerConditionEnd:
        builder.finish_container_condition();
        break;
    case FFI::CssRuleEventKind::ImportUrl:
        builder.current_import_url_type = event.important ? URL::Type::Src : URL::Type::Url;
        builder.current_import_url = string_from_ffi_bytes(event.name_ptr, event.name_len);
        builder.current_import_url_modifiers.clear();
        break;
    case FFI::CssRuleEventKind::ImportUrlModifier:
        switch (static_cast<FFI::CssUrlModifierKind>(event.name_len)) {
        case FFI::CssUrlModifierKind::CrossOrigin:
            builder.current_import_url_modifiers.append(RequestURLModifier::create_cross_origin(cross_origin_modifier_value_from_rust(static_cast<FFI::CssUrlCrossOriginModifierValue>(event.value_len))));
            break;
        case FFI::CssUrlModifierKind::Integrity:
            builder.current_import_url_modifiers.append(RequestURLModifier::create_integrity(fly_string_from_ffi_bytes(event.value_ptr, event.value_len)));
            break;
        case FFI::CssUrlModifierKind::ReferrerPolicy:
            builder.current_import_url_modifiers.append(RequestURLModifier::create_referrer_policy(referrer_policy_modifier_value_from_rust(static_cast<FFI::CssUrlReferrerPolicyModifierValue>(event.value_len))));
            break;
        }
        break;
    case FFI::CssRuleEventKind::ImportLayer: {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        at_rule.rust_import_layer = fly_string_from_ffi_bytes(event.name_ptr, event.name_len);
        break;
    }
    case FFI::CssRuleEventKind::ImportSupportsDeclarationStart:
        builder.component_value_builder = {};
        builder.current_import_supports_declaration_source = string_from_ffi_bytes(event.value_ptr, event.value_len);
        builder.stack.append({
            .type = RuleEventBuilder::FrameType::Declaration,
            .declaration = Declaration {
                .name = fly_string_from_ffi_bytes(event.name_ptr, event.name_len),
                .value = {},
                .important = event.important ? Important::Yes : Important::No,
            },
        });
        break;
    case FFI::CssRuleEventKind::ImportSupportsDeclarationEnd: {
        VERIFY(!builder.stack.is_empty());
        auto frame = builder.stack.take_last();
        VERIFY(frame.type == RuleEventBuilder::FrameType::Declaration);
        VERIFY(builder.component_value_builder.stack.is_empty());
        VERIFY(builder.current_import_supports_declaration_source.has_value());
        auto declaration = frame.declaration.release_value();
        declaration.value = move(builder.component_value_builder.root_values);
        set_original_value_text(declaration);
        builder.component_value_builder = {};

        auto supports = Supports::create(Supports::Declaration::create(builder.current_import_supports_declaration_source.release_value(), builder.supports_declaration_is_supported(declaration)));

        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        at_rule.rust_import_supports_condition = move(supports);
        break;
    }
    case FFI::CssRuleEventKind::ImportSupportsConditionEnd:
        builder.finish_supports_condition();
        break;
    case FFI::CssRuleEventKind::ImportMediaQueryListEnd:
        builder.finish_media_query_list();
        break;
    case FFI::CssRuleEventKind::MediaQueryListEnd:
        builder.finish_media_query_list();
        break;
    case FFI::CssRuleEventKind::SupportsConditionEnd:
        builder.finish_supports_condition();
        break;
    }
}

static void apply_rule_media_query(RuleEventBuilder& builder, FFI::CssMediaQuery const* rust_media_query)
{
    builder.finish_media_condition();

    VERIFY(!builder.stack.is_empty());
    auto& rule = builder.stack.last().rule;
    VERIFY(rule.has_value());
    auto& at_rule = rule->get<AtRule>();
    VERIFY(at_rule.name.equals_ignoring_ascii_case("media"sv) || at_rule.name.equals_ignoring_ascii_case("import"sv));
    auto& media_query_list = at_rule.name.equals_ignoring_ascii_case("import"sv) ? at_rule.rust_import_media_query_list : at_rule.rust_media_query_list;
    if (!media_query_list.has_value())
        media_query_list = Vector<NonnullRefPtr<MediaQuery>> {};

    auto media_query = MediaQuery::create();
    media_query->set_negated_for_parser(rust_media_query->is_negated);
    if (rust_media_query->media_type_len > 0) {
        auto media_type_name = fly_string_from_ffi_bytes(rust_media_query->media_type_ptr, rust_media_query->media_type_len);
        media_query->set_media_type_for_parser(MediaQuery::MediaType {
            .name = media_type_name,
            .known_type = media_type_from_rust(rust_media_query->media_type_kind),
        });
    }

    media_query_list->append(move(media_query));

    if (rust_media_query->has_media_condition) {
        builder.media_condition_builder = RustBooleanExpressionBuilder {
            .parse_test = [&builder](Optional<RustComponentValueParser::MediaFeatureTest>&& media_feature, Optional<RustComponentValueParser::SupportsFeature>&&, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
                if (!media_feature.has_value())
                    return nullptr;
                return builder.parse_media_feature_test(media_feature.release_value());
            },
            .result_for_general_enclosed = MatchResult::Unknown,
        };
    }
}

static void apply_rule_supports_condition_start(RuleEventBuilder& builder)
{
    builder.finish_supports_condition();

    VERIFY(!builder.stack.is_empty());
    auto& rule = builder.stack.last().rule;
    VERIFY(rule.has_value());
    auto& at_rule = rule->get<AtRule>();
    VERIFY(at_rule.name.equals_ignoring_ascii_case("supports"sv) || at_rule.name.equals_ignoring_ascii_case("import"sv));

    builder.supports_condition_builder = RustBooleanExpressionBuilder {
        .parse_test = [&builder](Optional<RustComponentValueParser::MediaFeatureTest>&&, Optional<RustComponentValueParser::SupportsFeature>&& supports_feature, Vector<ComponentValue>&& component_values) -> OwnPtr<BooleanExpression> {
            return builder.parse_supports_feature(move(supports_feature), move(component_values));
        },
        .result_for_general_enclosed = MatchResult::False,
    };
}

static void apply_rule_container_condition_start(RuleEventBuilder& builder)
{
    builder.finish_container_condition();

    VERIFY(!builder.stack.is_empty());
    auto& rule = builder.stack.last().rule;
    VERIFY(rule.has_value());
    auto& at_rule = rule->get<AtRule>();
    VERIFY(at_rule.name.equals_ignoring_ascii_case("container"sv));

    builder.container_condition_builder = RustBooleanExpressionBuilder {
        .parse_test = [](Optional<RustComponentValueParser::MediaFeatureTest>&&, Optional<RustComponentValueParser::SupportsFeature>&&, Vector<ComponentValue>&&) -> OwnPtr<BooleanExpression> {
            return nullptr;
        },
        .result_for_general_enclosed = MatchResult::False,
    };
}

static void apply_rule_boolean_expression_event(RuleEventBuilder& builder, FFI::CssBooleanExpressionEventKind event)
{
    if (!builder.current_boolean_expression_builder()) {
        VERIFY(!builder.stack.is_empty());
        auto& rule = builder.stack.last().rule;
        VERIFY(rule.has_value());
        auto& at_rule = rule->get<AtRule>();
        if (at_rule.name.equals_ignoring_ascii_case("supports"sv) || at_rule.name.equals_ignoring_ascii_case("import"sv))
            apply_rule_supports_condition_start(builder);
        if (at_rule.name.equals_ignoring_ascii_case("container"sv))
            apply_rule_container_condition_start(builder);
    }

    auto boolean_expression_builder = builder.current_boolean_expression_builder();
    VERIFY(boolean_expression_builder);
    process_boolean_expression_event(*boolean_expression_builder, event);
}

static void apply_rule_supports_feature(RuleEventBuilder& builder, FFI::CssSupportsFeatureKind kind, u8 const* name_ptr, size_t name_len, u8 const* value_ptr, size_t value_len, bool important)
{
    auto boolean_expression_builder = builder.current_boolean_expression_builder();
    VERIFY(boolean_expression_builder);
    set_boolean_expression_supports_feature(*boolean_expression_builder, kind, name_ptr, name_len, value_ptr, value_len, important);
}

static void apply_rule_media_feature(RuleEventBuilder& builder, FFI::CssMediaFeature const* media_feature)
{
    auto boolean_expression_builder = builder.current_boolean_expression_builder();
    VERIFY(boolean_expression_builder);
    set_boolean_expression_media_feature(*boolean_expression_builder, media_feature);
}

static void apply_rule_media_feature_value(RuleEventBuilder& builder, FFI::CssMediaFeatureValue const* media_feature_value)
{
    auto boolean_expression_builder = builder.current_boolean_expression_builder();
    VERIFY(boolean_expression_builder);
    append_boolean_expression_media_feature_value(*boolean_expression_builder, media_feature_value);
}

static void apply_rule_component_value(RuleEventBuilder& builder, FFI::CssComponentValue const* component_value)
{
    auto token = RustTokenizer::token_from_ffi(component_value->token);
    if (auto* boolean_expression_builder = builder.current_boolean_expression_builder()) {
        append_component_value_token(boolean_expression_builder->component_value_builder, component_value->kind, move(token));
        return;
    }
    append_component_value_token(builder.component_value_builder, component_value->kind, move(token));
}

static void verify_rule_event_builder_is_empty(RuleEventBuilder const& builder)
{
    VERIFY(builder.stack.is_empty());
    VERIFY(!builder.media_condition_builder.has_value());
    VERIFY(!builder.supports_condition_builder.has_value());
    VERIFY(!builder.container_condition_builder.has_value());
    VERIFY(!builder.current_import_url_type.has_value());
    VERIFY(!builder.current_import_url.has_value());
    VERIFY(builder.current_import_url_modifiers.is_empty());
    VERIFY(!builder.current_import_supports_declaration_source.has_value());
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

Optional<Rule> RustComponentValueParser::parse_a_rule(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_media_feature_test, AK::Function<OwnPtr<BooleanExpression>(Optional<SupportsFeature>&&, Vector<ComponentValue>&&)> parse_supports_feature, AK::Function<bool(Declaration const&)> supports_declaration_is_supported)
{
    RuleEventBuilder builder {
        .parse_media_feature_test = move(parse_media_feature_test),
        .parse_supports_feature = move(parse_supports_feature),
        .supports_declaration_is_supported = move(supports_declaration_is_supported),
    };
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_rule(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssRuleEvent const* event) {
            apply_rule_event(*static_cast<RuleEventBuilder*>(raw_builder), *event);
        },
        [](void* raw_builder, FFI::CssMediaQuery const* media_query) {
            apply_rule_media_query(*static_cast<RuleEventBuilder*>(raw_builder), media_query);
        },
        [](void* raw_builder, FFI::CssBooleanExpressionEventKind event) {
            apply_rule_boolean_expression_event(*static_cast<RuleEventBuilder*>(raw_builder), event);
        },
        [](void* raw_builder, FFI::CssSupportsFeatureKind kind, u8 const* name_ptr, size_t name_len, u8 const* value_ptr, size_t value_len, bool important) {
            apply_rule_supports_feature(*static_cast<RuleEventBuilder*>(raw_builder), kind, name_ptr, name_len, value_ptr, value_len, important);
        },
        [](void* raw_builder, FFI::CssMediaFeature const* media_feature) {
            apply_rule_media_feature(*static_cast<RuleEventBuilder*>(raw_builder), media_feature);
        },
        [](void* raw_builder, FFI::CssMediaFeatureValue const* media_feature_value) {
            apply_rule_media_feature_value(*static_cast<RuleEventBuilder*>(raw_builder), media_feature_value);
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            apply_rule_component_value(*static_cast<RuleEventBuilder*>(raw_builder), component_value);
        });

    verify_rule_event_builder_is_empty(builder);
    return builder.rule;
}

Vector<RuleOrListOfDeclarations> RustComponentValueParser::parse_a_blocks_contents(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_media_feature_test, AK::Function<OwnPtr<BooleanExpression>(Optional<SupportsFeature>&&, Vector<ComponentValue>&&)> parse_supports_feature, AK::Function<bool(Declaration const&)> supports_declaration_is_supported)
{
    Vector<RuleContext> rule_context;
    rule_context.append(RuleContext::Style);
    return parse_a_blocks_contents(input, encoding, rule_context, move(parse_media_feature_test), move(parse_supports_feature), move(supports_declaration_is_supported));
}

Vector<RuleOrListOfDeclarations> RustComponentValueParser::parse_a_blocks_contents(StringView input, StringView encoding, Vector<RuleContext> const& rule_context, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_media_feature_test, AK::Function<OwnPtr<BooleanExpression>(Optional<SupportsFeature>&&, Vector<ComponentValue>&&)> parse_supports_feature, AK::Function<bool(Declaration const&)> supports_declaration_is_supported)
{
    RuleEventBuilder builder {
        .parse_media_feature_test = move(parse_media_feature_test),
        .parse_supports_feature = move(parse_supports_feature),
        .supports_declaration_is_supported = move(supports_declaration_is_supported),
    };
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
        [](void* raw_builder, FFI::CssMediaQuery const* media_query) {
            apply_rule_media_query(*static_cast<RuleEventBuilder*>(raw_builder), media_query);
        },
        [](void* raw_builder, FFI::CssBooleanExpressionEventKind event) {
            apply_rule_boolean_expression_event(*static_cast<RuleEventBuilder*>(raw_builder), event);
        },
        [](void* raw_builder, FFI::CssSupportsFeatureKind kind, u8 const* name_ptr, size_t name_len, u8 const* value_ptr, size_t value_len, bool important) {
            apply_rule_supports_feature(*static_cast<RuleEventBuilder*>(raw_builder), kind, name_ptr, name_len, value_ptr, value_len, important);
        },
        [](void* raw_builder, FFI::CssMediaFeature const* media_feature) {
            apply_rule_media_feature(*static_cast<RuleEventBuilder*>(raw_builder), media_feature);
        },
        [](void* raw_builder, FFI::CssMediaFeatureValue const* media_feature_value) {
            apply_rule_media_feature_value(*static_cast<RuleEventBuilder*>(raw_builder), media_feature_value);
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            apply_rule_component_value(*static_cast<RuleEventBuilder*>(raw_builder), component_value);
        });

    verify_rule_event_builder_is_empty(builder);
    return move(builder.rules_or_lists_of_declarations);
}

Vector<Rule> RustComponentValueParser::parse_a_stylesheets_contents(StringView input, StringView encoding, AK::Function<OwnPtr<BooleanExpression>(MediaFeatureTest&&)> parse_media_feature_test, AK::Function<OwnPtr<BooleanExpression>(Optional<SupportsFeature>&&, Vector<ComponentValue>&&)> parse_supports_feature, AK::Function<bool(Declaration const&)> supports_declaration_is_supported)
{
    RuleEventBuilder builder {
        .parse_media_feature_test = move(parse_media_feature_test),
        .parse_supports_feature = move(parse_supports_feature),
        .supports_declaration_is_supported = move(supports_declaration_is_supported),
    };
    auto filtered_input = decode_and_filter_code_points(input, encoding);
    auto filtered_input_bytes = filtered_input.bytes();

    FFI::rust_css_parse_stylesheet_contents(
        filtered_input_bytes.data(),
        filtered_input_bytes.size(),
        &builder,
        [](void* raw_builder, FFI::CssRuleEvent const* event) {
            apply_rule_event(*static_cast<RuleEventBuilder*>(raw_builder), *event);
        },
        [](void* raw_builder, FFI::CssMediaQuery const* media_query) {
            apply_rule_media_query(*static_cast<RuleEventBuilder*>(raw_builder), media_query);
        },
        [](void* raw_builder, FFI::CssBooleanExpressionEventKind event) {
            apply_rule_boolean_expression_event(*static_cast<RuleEventBuilder*>(raw_builder), event);
        },
        [](void* raw_builder, FFI::CssSupportsFeatureKind kind, u8 const* name_ptr, size_t name_len, u8 const* value_ptr, size_t value_len, bool important) {
            apply_rule_supports_feature(*static_cast<RuleEventBuilder*>(raw_builder), kind, name_ptr, name_len, value_ptr, value_len, important);
        },
        [](void* raw_builder, FFI::CssMediaFeature const* media_feature) {
            apply_rule_media_feature(*static_cast<RuleEventBuilder*>(raw_builder), media_feature);
        },
        [](void* raw_builder, FFI::CssMediaFeatureValue const* media_feature_value) {
            apply_rule_media_feature_value(*static_cast<RuleEventBuilder*>(raw_builder), media_feature_value);
        },
        [](void* raw_builder, FFI::CssComponentValue const* component_value) {
            apply_rule_component_value(*static_cast<RuleEventBuilder*>(raw_builder), component_value);
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
