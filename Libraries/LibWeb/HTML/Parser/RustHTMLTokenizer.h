/*
 * Copyright (c) 2026, Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#ifdef ENABLE_RUST

#include <AK/StringView.h>
#include <AK/Vector.h>
#include <LibWeb/HTML/Parser/HTMLTokenizer.h>

#include <stddef.h>
#include <stdint.h>

struct RustFfiAttribute {
    uint8_t const* name_ptr;
    size_t name_len;
    uint8_t const* value_ptr;
    size_t value_len;
    uint64_t name_start_line;
    uint64_t name_start_column;
    uint64_t name_end_line;
    uint64_t name_end_column;
    uint64_t value_start_line;
    uint64_t value_start_column;
    uint64_t value_end_line;
    uint64_t value_end_column;
};

struct RustFfiToken {
    uint8_t token_type;
    uint32_t code_point;
    bool self_closing;

    uint8_t const* tag_name_ptr;
    size_t tag_name_len;

    uint8_t const* comment_ptr;
    size_t comment_len;

    uint8_t const* doctype_name_ptr;
    size_t doctype_name_len;
    uint8_t const* public_id_ptr;
    size_t public_id_len;
    uint8_t const* system_id_ptr;
    size_t system_id_len;
    bool force_quirks;
    bool missing_name;
    bool missing_public_id;
    bool missing_system_id;

    RustFfiAttribute const* attributes_ptr;
    size_t attributes_len;

    uint64_t start_line;
    uint64_t start_column;
    uint64_t end_line;
    uint64_t end_column;
};

extern "C" {
void* rust_html_tokenizer_create(uint32_t const* input, size_t len);
bool rust_html_tokenizer_next_token(void* handle, RustFfiToken* out);
void rust_html_tokenizer_switch_state(void* handle, uint8_t state);
void rust_html_tokenizer_destroy(void* handle);
}

namespace Web::HTML {

bool rust_html_tokenizer_enabled();

class RustHTMLTokenizer {
public:
    explicit RustHTMLTokenizer(Vector<u32> const& code_points);
    ~RustHTMLTokenizer();

    RustHTMLTokenizer(RustHTMLTokenizer const&) = delete;
    RustHTMLTokenizer& operator=(RustHTMLTokenizer const&) = delete;

    bool next_token(RustFfiToken& out);
    void switch_to(HTMLTokenizer::State state);

private:
    void* m_handle { nullptr };
};

}

#endif // ENABLE_RUST
