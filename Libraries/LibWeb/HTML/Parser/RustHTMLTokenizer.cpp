/*
 * Copyright (c) 2026, Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/HTML/Parser/RustHTMLTokenizer.h>

#ifdef ENABLE_RUST

namespace Web::HTML {

bool rust_html_tokenizer_enabled()
{
    static bool const enabled = getenv("LIBWEB_USE_RUST_TOKENIZER") != nullptr;
    return enabled;
}

RustHTMLTokenizer::RustHTMLTokenizer(Vector<u32> const& code_points)
{
    m_handle = rust_html_tokenizer_create(code_points.data(), code_points.size());
}

RustHTMLTokenizer::~RustHTMLTokenizer()
{
    if (m_handle)
        rust_html_tokenizer_destroy(m_handle);
}

bool RustHTMLTokenizer::next_token(RustFfiToken& out, bool stop_at_insertion_point, bool cdata_allowed)
{
    return rust_html_tokenizer_next_token(m_handle, &out, stop_at_insertion_point, cdata_allowed);
}

void RustHTMLTokenizer::switch_to(HTMLTokenizer::State state)
{
    rust_html_tokenizer_switch_state(m_handle, static_cast<uint8_t>(state));
}

void RustHTMLTokenizer::insert_input(Vector<u32> const& code_points)
{
    rust_html_tokenizer_insert_input(m_handle, code_points.data(), code_points.size());
}

void RustHTMLTokenizer::store_insertion_point()
{
    rust_html_tokenizer_store_insertion_point(m_handle);
}

void RustHTMLTokenizer::restore_insertion_point()
{
    rust_html_tokenizer_restore_insertion_point(m_handle);
}

void RustHTMLTokenizer::update_insertion_point()
{
    rust_html_tokenizer_update_insertion_point(m_handle);
}

void RustHTMLTokenizer::undefine_insertion_point()
{
    rust_html_tokenizer_undefine_insertion_point(m_handle);
}

bool RustHTMLTokenizer::is_insertion_point_defined() const
{
    return rust_html_tokenizer_is_insertion_point_defined(m_handle);
}

void RustHTMLTokenizer::set_blocked(bool blocked)
{
    rust_html_tokenizer_set_blocked(m_handle, blocked);
}

bool RustHTMLTokenizer::is_blocked() const
{
    return rust_html_tokenizer_is_blocked(m_handle);
}

void RustHTMLTokenizer::insert_eof()
{
    rust_html_tokenizer_insert_eof(m_handle);
}

bool RustHTMLTokenizer::is_eof_inserted() const
{
    return rust_html_tokenizer_is_eof_inserted(m_handle);
}

void RustHTMLTokenizer::abort()
{
    rust_html_tokenizer_abort(m_handle);
}

void RustHTMLTokenizer::set_last_start_tag_name(StringView name)
{
    rust_html_tokenizer_set_last_start_tag_name(m_handle, reinterpret_cast<uint8_t const*>(name.characters_without_null_termination()), name.length());
}

}

#endif // ENABLE_RUST
