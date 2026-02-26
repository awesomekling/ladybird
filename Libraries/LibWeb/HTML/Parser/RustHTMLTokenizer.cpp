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

bool RustHTMLTokenizer::next_token(RustFfiToken& out)
{
    return rust_html_tokenizer_next_token(m_handle, &out);
}

void RustHTMLTokenizer::switch_to(HTMLTokenizer::State state)
{
    rust_html_tokenizer_switch_state(m_handle, static_cast<uint8_t>(state));
}

}

#endif // ENABLE_RUST
