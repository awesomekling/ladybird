/*
 * Copyright (c) 2026, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include "ContainerQuery.h"
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/DOM/AbstractElement.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/Dump.h>

namespace Web::CSS {

NonnullRefPtr<ContainerConditions> ContainerConditions::create(Parser::ValueParserFFI::ContainerConditionsData const* data)
{
    return adopt_ref(*new ContainerConditions(data));
}

ContainerConditions::ContainerConditions(Parser::ValueParserFFI::ContainerConditionsData const* data)
    : m_data(Parser::ValueParserFFI::rust_container_conditions_retain(data))
{
    VERIFY(m_data);
}

ContainerConditions::~ContainerConditions()
{
    Parser::ValueParserFFI::rust_container_conditions_release(m_data);
}

Vector<ContainerConditions::Condition> const& ContainerConditions::entries() const
{
    if (!m_entries.has_value()) {
        Vector<Condition> entries;
        auto count = Parser::ValueParserFFI::rust_container_conditions_count(m_data);
        entries.ensure_capacity(count);
        for (size_t index = 0; index < count; ++index) {
            auto name = Parser::ValueParserFFI::rust_container_conditions_name(m_data, index);
            auto const* query = Parser::ValueParserFFI::rust_container_conditions_query(m_data, index);
            Condition condition;
            // A container name is a nonempty custom identifier; an empty view means it is absent.
            if (name.length)
                condition.container_name = Utf16FlyString::from_utf16({ reinterpret_cast<char16_t const*>(name.utf16), name.length });
            if (query)
                condition.container_query = ContainerQuery::create(RustQueryHandle::retained(query));
            entries.unchecked_append(move(condition));
        }
        m_entries = move(entries);
    }
    return *m_entries;
}

bool ContainerConditions::contains_size_feature() const
{
    return any_of(entries(), [](auto const& condition) { return condition.container_query && condition.container_query->contains_size_feature(); });
}

bool ContainerConditions::contains_style_feature() const
{
    return any_of(entries(), [](auto const& condition) { return condition.container_query && condition.container_query->contains_style_feature(); });
}

void ContainerConditions::mark_element_style_dependencies(DOM::AbstractElement& element) const
{
    if (contains_size_feature())
        element.element().set_style_depends_on_size_container_query();
    if (contains_style_feature())
        element.element().set_style_depends_on_style_container_query();
}

NonnullRefPtr<ContainerQuery> ContainerQuery::create(RustQueryHandle handle)
{
    return adopt_ref(*new ContainerQuery(move(handle)));
}

static ContainerQueryFeatureRequirements container_query_requirements(Parser::ValueParserFFI::FfiQueryHandle const* query)
{
    auto requirements = Parser::ValueParserFFI::css_query_container_requirements(query);
    return {
        .requires_width_container = static_cast<bool>(requirements & Parser::ValueParserFFI::CONTAINER_QUERY_REQUIRES_WIDTH),
        .requires_height_container = static_cast<bool>(requirements & Parser::ValueParserFFI::CONTAINER_QUERY_REQUIRES_HEIGHT),
        .requires_inline_size_container = static_cast<bool>(requirements & Parser::ValueParserFFI::CONTAINER_QUERY_REQUIRES_INLINE_SIZE),
        .requires_block_size_container = static_cast<bool>(requirements & Parser::ValueParserFFI::CONTAINER_QUERY_REQUIRES_BLOCK_SIZE),
        .requires_style_container = static_cast<bool>(requirements & Parser::ValueParserFFI::CONTAINER_QUERY_REQUIRES_STYLE),
        .requires_scroll_state_container = static_cast<bool>(requirements & Parser::ValueParserFFI::CONTAINER_QUERY_REQUIRES_SCROLL_STATE),
        .has_unknown_or_unsupported_feature = static_cast<bool>(requirements & Parser::ValueParserFFI::CONTAINER_QUERY_HAS_UNKNOWN_FEATURE),
    };
}

ContainerQuery::ContainerQuery(RustQueryHandle handle)
    : m_rust_query_handle(move(handle))
    , m_feature_requirements(container_query_requirements(m_rust_query_handle.data()))
{
}

Utf16String ContainerQuery::to_string() const
{
    Utf16String serialized;
    auto set_serialized_query = [](void* context, u16 const* code_units, size_t length) {
        *static_cast<Utf16String*>(context) = Utf16String::from_utf16({ reinterpret_cast<char16_t const*>(code_units), length });
    };
    VERIFY(Parser::ValueParserFFI::css_query_serialize_condition(m_rust_query_handle.data(), &serialized, set_serialized_query));
    return serialized;
}

void ContainerQuery::dump(StringBuilder& builder, int indent_levels) const
{
    dump_indent(builder, indent_levels);
    builder.appendff("Container query: `{}`\n", to_string());
}

}
