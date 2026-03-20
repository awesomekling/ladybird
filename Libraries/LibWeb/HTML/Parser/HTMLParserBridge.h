/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <stddef.h>
#include <stdint.h>

#ifdef ENABLE_RUST

/// Forward declarations for bridge functions called by the Rust HTML parser.
/// These are implemented in HTMLParserBridge.cpp.

extern "C" {

void* html_parser_bridge_create_element(void* document_ptr, uint8_t const* local_name_ptr, size_t local_name_len, uint8_t namespace_id);
void html_parser_bridge_set_attribute(void* element_ptr, uint8_t const* name_ptr, size_t name_len, uint8_t const* value_ptr, size_t value_len);
void html_parser_bridge_set_attribute_ns(void* element_ptr, uint8_t namespace_id, uint8_t const* prefix_ptr, size_t prefix_len, uint8_t const* local_name_ptr, size_t local_name_len, uint8_t const* value_ptr, size_t value_len);

void* html_parser_bridge_create_text_node(void* document_ptr, uint8_t const* data_ptr, size_t data_len);
void* html_parser_bridge_create_comment(void* document_ptr, uint8_t const* data_ptr, size_t data_len);
void html_parser_bridge_append_text(void* text_node_ptr, uint8_t const* data_ptr, size_t data_len);

void html_parser_bridge_insert_doctype(void* document_ptr, uint8_t const* name_ptr, size_t name_len, uint8_t const* public_id_ptr, size_t public_id_len, uint8_t const* system_id_ptr, size_t system_id_len);
void html_parser_bridge_set_quirks_mode(void* document_ptr, uint8_t mode);

void html_parser_bridge_insert_before(void* parent_ptr, void* node_ptr, void* before_sibling_ptr);
void html_parser_bridge_remove_node(void* node_ptr);

void* html_parser_bridge_parent(void* node_ptr);
void* html_parser_bridge_last_child(void* node_ptr);
void* html_parser_bridge_next_sibling(void* node_ptr);
bool html_parser_bridge_is_text_node(void* node_ptr);
bool html_parser_bridge_is_element(void* node_ptr);

void html_parser_bridge_element_local_name(void* element_ptr, uint8_t const** out_ptr, size_t* out_len);
uint8_t html_parser_bridge_element_namespace(void* element_ptr);

void* html_parser_bridge_template_contents(void* element_ptr);
void* html_parser_bridge_document_node(void* document_ptr);

void html_parser_bridge_associate_with_form(void* element_ptr, void* form_ptr);
bool html_parser_bridge_execute_script(void* element_ptr);

/// Set parser document and force_async=false on a script element.
void html_parser_bridge_setup_script_element(void* element_ptr, void* document_ptr);

void html_parser_bridge_visit_node(void* visitor_ptr, void* node_ptr);

} // extern "C"

#endif // ENABLE_RUST
