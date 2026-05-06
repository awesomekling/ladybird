/*
 * Copyright (c) 2018-2022, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2020-2021, the SerenityOS developers.
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2021, Tobias Christiansen <tobyase@serenityos.org>
 * Copyright (c) 2022, MacDue <macdue@dueutil.tech>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/RustComponentValueParser.h>

namespace Web::CSS::Parser {

Optional<SelectorList> Parser::parse_as_selector(SelectorParsingMode parsing_mode)
{
    return RustComponentValueParser::parse_a_selector_list(
        m_input,
        m_encoding,
        RustComponentValueParser::SelectorType::Standalone,
        parsing_mode == SelectorParsingMode::Forgiving
            ? RustComponentValueParser::SelectorParsingMode::Forgiving
            : RustComponentValueParser::SelectorParsingMode::Normal,
        m_declared_namespaces);
}

Optional<SelectorList> Parser::parse_as_relative_selector(SelectorParsingMode parsing_mode)
{
    return RustComponentValueParser::parse_a_selector_list(
        m_input,
        m_encoding,
        RustComponentValueParser::SelectorType::Relative,
        parsing_mode == SelectorParsingMode::Forgiving
            ? RustComponentValueParser::SelectorParsingMode::Forgiving
            : RustComponentValueParser::SelectorParsingMode::Normal,
        m_declared_namespaces);
}

Optional<Selector::PseudoElementSelector> Parser::parse_as_pseudo_element_selector()
{
    auto is_css_input_whitespace = [](char byte) {
        return byte == ' ' || byte == '\t' || byte == '\n' || byte == '\r' || byte == '\f';
    };
    auto input_bytes = m_input.bytes_as_string_view();
    if (!input_bytes.is_empty() && (is_css_input_whitespace(input_bytes[0]) || is_css_input_whitespace(input_bytes[input_bytes.length() - 1])))
        return {};

    auto maybe_selector_list = RustComponentValueParser::parse_a_selector_list(
        m_input,
        m_encoding,
        RustComponentValueParser::SelectorType::Standalone,
        RustComponentValueParser::SelectorParsingMode::Normal,
        m_declared_namespaces);
    if (!maybe_selector_list.has_value())
        return {};
    auto selector_list = maybe_selector_list.release_value();
    if (selector_list.size() != 1)
        return {};

    auto const& compound_selectors = selector_list.first()->compound_selectors();
    if (compound_selectors.size() != 1)
        return {};
    auto const& simple_selectors = compound_selectors.first().simple_selectors;
    if (simple_selectors.size() != 1)
        return {};
    auto const& simple_selector = simple_selectors.first();
    if (simple_selector.type != Selector::SimpleSelector::Type::PseudoElement)
        return {};
    return simple_selector.pseudo_element();
}

Optional<PageSelectorList> Parser::parse_as_page_selector_list()
{
    return RustComponentValueParser::parse_a_page_selector_list(m_input, m_encoding);
}

}
