/*
 * Copyright (c) 2026, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/HTML/Parser/HTMLParserBridge.h>

#include <AK/FlyString.h>
#include <AK/String.h>
#include <LibGC/Cell.h>
#include <LibGC/Heap.h>
#include <LibWeb/DOM/Comment.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/DocumentType.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/ElementFactory.h>
#include <LibWeb/DOM/Text.h>
#include <LibWeb/Bindings/ExceptionOrUtils.h>
#include <LibWeb/Bindings/MainThreadVM.h>
#include <LibWeb/HTML/CustomElements/CustomElementDefinition.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/Scripting/SimilarOriginWindowAgent.h>
#include <LibWeb/HTML/HTMLMediaElement.h>
#include <LibWeb/HTML/FormAssociatedElement.h>
#include <LibWeb/HTML/HTMLFormElement.h>
#include <LibWeb/HTML/HTMLLinkElement.h>
#include <LibWeb/HTML/HTMLOptionElement.h>
#include <LibWeb/HTML/HTMLScriptElement.h>
#include <LibWeb/SVG/SVGScriptElement.h>
#include <LibWeb/HTML/HTMLTemplateElement.h>
#include <LibWeb/HTML/Parser/HTMLParser.h>
#include <LibWeb/Namespace.h>

#ifdef ENABLE_RUST

/// Bridge functions called by the Rust HTML parser to manipulate the C++ DOM.
/// These are the C++ implementations of the extern "C" functions declared in dom_bridge.rs.

static Optional<FlyString> namespace_from_u8(uint8_t ns)
{
    switch (ns) {
    case 0:
        return Web::Namespace::HTML;
    case 1:
        return Web::Namespace::SVG;
    case 2:
        return Web::Namespace::MathML;
    case 3:
        return Web::Namespace::XLink;
    case 4:
        return Web::Namespace::XML;
    case 5:
        return Web::Namespace::XMLNS;
    default:
        return Web::Namespace::HTML;
    }
}

static uint8_t namespace_to_u8(Optional<FlyString> const& ns)
{
    if (!ns.has_value())
        return 0; // HTML
    if (ns.value() == Web::Namespace::HTML)
        return 0;
    if (ns.value() == Web::Namespace::SVG)
        return 1;
    if (ns.value() == Web::Namespace::MathML)
        return 2;
    if (ns.value() == Web::Namespace::XLink)
        return 3;
    if (ns.value() == Web::Namespace::XML)
        return 4;
    if (ns.value() == Web::Namespace::XMLNS)
        return 5;
    return 0;
}

static StringView string_view_from_rust(uint8_t const* ptr, size_t len)
{
    return { reinterpret_cast<char const*>(ptr), len };
}

extern "C" {

void* html_parser_bridge_create_element(void* document_ptr, uint8_t const* local_name_ptr, size_t local_name_len, uint8_t namespace_id, uint8_t const* is_value_ptr, size_t is_value_len)
{
    auto& document = *static_cast<Web::DOM::Document*>(document_ptr);
    auto local_name_sv = string_view_from_rust(local_name_ptr, local_name_len);
    auto local_name = MUST(FlyString::from_utf8(local_name_sv));
    auto const& ns = namespace_from_u8(namespace_id);

    Optional<String> is_value;
    if (is_value_ptr && is_value_len > 0) {
        auto is_sv = string_view_from_rust(is_value_ptr, is_value_len);
        is_value = MUST(String::from_utf8(is_sv));
    }

    auto definition = document.lookup_custom_element_definition(ns, local_name, is_value);
    auto element = Web::HTML::HTMLParser::create_element_for_rust_parser(document, local_name, ns, is_value, definition);

    return element.ptr();
}

void html_parser_bridge_set_attribute(void* element_ptr, uint8_t const* name_ptr, size_t name_len, uint8_t const* value_ptr, size_t value_len)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);
    auto name_sv = string_view_from_rust(name_ptr, name_len);
    auto value_sv = string_view_from_rust(value_ptr, value_len);

    auto name = MUST(FlyString::from_utf8(name_sv));
    auto value = MUST(String::from_utf8(value_sv));

    element.set_attribute_value(name, value);
}

void html_parser_bridge_append_attribute(void* element_ptr, uint8_t const* name_ptr, size_t name_len, uint8_t const* value_ptr, size_t value_len)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);
    auto name_sv = string_view_from_rust(name_ptr, name_len);
    auto value_sv = string_view_from_rust(value_ptr, value_len);

    auto name = MUST(FlyString::from_utf8(name_sv));

    // Only add the attribute if it doesn't already exist (first one wins per spec).
    if (!element.has_attribute(name)) {
        auto value = MUST(String::from_utf8(value_sv));
        element.append_attribute(name, value);
    }
}

void html_parser_bridge_set_attribute_ns(void* element_ptr, uint8_t namespace_id, uint8_t const* prefix_ptr, size_t prefix_len, uint8_t const* local_name_ptr, size_t local_name_len, uint8_t const* value_ptr, size_t value_len)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);
    auto prefix_sv = string_view_from_rust(prefix_ptr, prefix_len);
    auto local_name_sv = string_view_from_rust(local_name_ptr, local_name_len);
    auto value_sv = string_view_from_rust(value_ptr, value_len);

    auto const& ns = namespace_from_u8(namespace_id);
    auto prefix = prefix_sv.is_empty() ? Optional<FlyString> {} : Optional<FlyString> { MUST(FlyString::from_utf8(prefix_sv)) };
    auto local_name = MUST(FlyString::from_utf8(local_name_sv));
    auto value = MUST(String::from_utf8(value_sv));

    element.set_attribute_value(local_name, value, prefix, ns);
}

void* html_parser_bridge_create_text_node(void* document_ptr, uint8_t const* data_ptr, size_t data_len)
{
    auto& document = *static_cast<Web::DOM::Document*>(document_ptr);
    auto data_sv = string_view_from_rust(data_ptr, data_len);
    auto text = document.create_text_node(Utf16String::from_utf8(data_sv));
    return text.ptr();
}

void* html_parser_bridge_create_comment(void* document_ptr, uint8_t const* data_ptr, size_t data_len)
{
    auto& document = *static_cast<Web::DOM::Document*>(document_ptr);
    auto data_sv = string_view_from_rust(data_ptr, data_len);
    auto comment = document.realm().create<Web::DOM::Comment>(document, Utf16String::from_utf8(data_sv));
    return comment.ptr();
}

void html_parser_bridge_append_text(void* text_node_ptr, uint8_t const* data_ptr, size_t data_len)
{
    auto& text_node = *static_cast<Web::DOM::Text*>(text_node_ptr);
    auto data_sv = string_view_from_rust(data_ptr, data_len);
    MUST(text_node.append_data(Utf16String::from_utf8(data_sv)));
}

void html_parser_bridge_insert_doctype(void* document_ptr, uint8_t const* name_ptr, size_t name_len, uint8_t const* public_id_ptr, size_t public_id_len, uint8_t const* system_id_ptr, size_t system_id_len)
{
    auto& document = *static_cast<Web::DOM::Document*>(document_ptr);
    auto name_sv = string_view_from_rust(name_ptr, name_len);
    auto public_id_sv = string_view_from_rust(public_id_ptr, public_id_len);
    auto system_id_sv = string_view_from_rust(system_id_ptr, system_id_len);

    auto doctype = document.realm().create<Web::DOM::DocumentType>(document);
    doctype->set_name(MUST(String::from_utf8(name_sv)));
    doctype->set_public_id(MUST(String::from_utf8(public_id_sv)));
    doctype->set_system_id(MUST(String::from_utf8(system_id_sv)));
    MUST(document.append_child(*doctype));
}

void html_parser_bridge_set_quirks_mode(void* document_ptr, uint8_t mode)
{
    auto& document = *static_cast<Web::DOM::Document*>(document_ptr);
    switch (mode) {
    case 0:
        document.set_quirks_mode(Web::DOM::QuirksMode::No);
        break;
    case 1:
        document.set_quirks_mode(Web::DOM::QuirksMode::Limited);
        break;
    case 2:
        document.set_quirks_mode(Web::DOM::QuirksMode::Yes);
        break;
    }
}

void html_parser_bridge_insert_before(void* parent_ptr, void* node_ptr, void* before_sibling_ptr)
{
    auto& parent = *static_cast<Web::DOM::Node*>(parent_ptr);
    auto& node = *static_cast<Web::DOM::Node*>(node_ptr);
    GC::Ptr<Web::DOM::Node> before_sibling;
    if (before_sibling_ptr)
        before_sibling = static_cast<Web::DOM::Node*>(before_sibling_ptr);
    parent.insert_before(node, before_sibling);
}

void html_parser_bridge_remove_node(void* node_ptr)
{
    auto& node = *static_cast<Web::DOM::Node*>(node_ptr);
    node.remove();
}

void* html_parser_bridge_parent(void* node_ptr)
{
    auto& node = *static_cast<Web::DOM::Node*>(node_ptr);
    return node.parent();
}

void* html_parser_bridge_last_child(void* node_ptr)
{
    auto& node = *static_cast<Web::DOM::Node*>(node_ptr);
    return node.last_child();
}

void* html_parser_bridge_next_sibling(void* node_ptr)
{
    auto& node = *static_cast<Web::DOM::Node*>(node_ptr);
    return node.next_sibling();
}

bool html_parser_bridge_is_text_node(void* node_ptr)
{
    auto& node = *static_cast<Web::DOM::Node*>(node_ptr);
    return node.is_text();
}

bool html_parser_bridge_is_element(void* node_ptr)
{
    auto& node = *static_cast<Web::DOM::Node*>(node_ptr);
    return node.is_element();
}

void html_parser_bridge_element_local_name(void* element_ptr, uint8_t const** out_ptr, size_t* out_len)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);
    auto const& name = element.local_name();
    auto bytes = name.bytes();
    *out_ptr = reinterpret_cast<uint8_t const*>(bytes.data());
    *out_len = bytes.size();
}

uint8_t html_parser_bridge_element_namespace(void* element_ptr)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);
    return namespace_to_u8(element.namespace_uri());
}

void* html_parser_bridge_template_contents(void* element_ptr)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);
    if (auto* tmpl = as_if<Web::HTML::HTMLTemplateElement>(&element))
        return tmpl->content().ptr();
    return nullptr;
}

void* html_parser_bridge_document_node(void* document_ptr)
{
    // A Document IS a Node, so just return the same pointer.
    return document_ptr;
}

void html_parser_bridge_associate_with_form(void* element_ptr, void* form_ptr)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);

    auto* form = static_cast<Web::HTML::HTMLFormElement*>(form_ptr);
    Web::HTML::HTMLParser::associate_with_form(element, *form);
}

bool html_parser_bridge_execute_script(void* element_ptr, void* document_ptr)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);
    auto& document = *static_cast<Web::DOM::Document*>(document_ptr);

    // Perform a microtask checkpoint before script preparation.
    // This processes queued custom element reactions (e.g., connectedCallback).
    auto& vm = Web::HTML::main_thread_event_loop().vm();
    if (vm.execution_context_stack().is_empty())
        Web::HTML::perform_a_microtask_checkpoint();

    if (auto* script = as_if<Web::HTML::HTMLScriptElement>(&element)) {
        Web::HTML::HTMLParser::handle_script_end_tag(*script, document);
    } else if (auto* svg_script = as_if<Web::SVG::SVGScriptElement>(&element)) {
        svg_script->process_the_script_element();
    }

    return false;
}

void html_parser_bridge_setup_script_element(void* element_ptr, void* document_ptr)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);
    auto& document = *static_cast<Web::DOM::Document*>(document_ptr);
    if (auto* script = as_if<Web::HTML::HTMLScriptElement>(&element)) {
        Web::HTML::HTMLParser::setup_script_element(*script, document);
    }
}

void html_parser_bridge_post_create_element(void* element_ptr)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);

    // AD-HOC: Let <link> elements know which document they were originally parsed for.
    if (auto* link = as_if<Web::HTML::HTMLLinkElement>(&element)) {
        Web::HTML::HTMLParser::setup_link_element(*link);
    }

    // Mark SVG script elements as parser-inserted so they don't execute on insertion.
    if (as_if<Web::SVG::SVGScriptElement>(&element)) {
        Web::HTML::HTMLParser::mark_svg_script_as_parser_inserted(static_cast<Web::SVG::SVGScriptElement&>(element));
    }

    // https://html.spec.whatwg.org/multipage/media.html#user-interface:attr-media-muted
    // When a media element is created, if the element has a muted content attribute specified,
    // then the muted IDL attribute should be set to true.
    if (element.is_html_media_element() && element.has_attribute("muted"_fly_string)) {
        auto& media_element = static_cast<Web::HTML::HTMLMediaElement&>(element);
        media_element.set_muted(true);
    }
}

void html_parser_bridge_element_popped(void* element_ptr)
{
    auto& element = *static_cast<Web::DOM::Element*>(element_ptr);

    // https://html.spec.whatwg.org/multipage/form-elements.html#the-option-element
    // When an option element is popped off the stack of open elements,
    // the user agent must run maybe clone an option into selectedcontent.
    if (auto* option = as_if<Web::HTML::HTMLOptionElement>(&element)) {
        MUST(option->maybe_clone_into_selectedcontent());
    }
}

bool html_parser_bridge_handle_declarative_shadow_template(
    void* template_element_ptr,
    void* host_element_ptr,
    void* document_ptr,
    uint8_t const* shadowrootmode_ptr, size_t shadowrootmode_len,
    bool shadowrootclonable,
    bool shadowrootserializable,
    bool shadowrootdelegatesfocus)
{
    auto& template_element = *static_cast<Web::DOM::Element*>(template_element_ptr);
    auto& host_element = *static_cast<Web::DOM::Element*>(host_element_ptr);
    auto& document = *static_cast<Web::DOM::Document*>(document_ptr);
    auto mode_sv = string_view_from_rust(shadowrootmode_ptr, shadowrootmode_len);

    auto* tmpl = as_if<Web::HTML::HTMLTemplateElement>(&template_element);
    if (!tmpl)
        return false;

    Optional<Web::Bindings::ShadowRootMode> shadowrootmode;
    if (mode_sv == "open"sv)
        shadowrootmode = Web::Bindings::ShadowRootMode::Open;
    else if (mode_sv == "closed"sv)
        shadowrootmode = Web::Bindings::ShadowRootMode::Closed;

    if (!shadowrootmode.has_value() || !document.allow_declarative_shadow_roots())
        return false;

    if (host_element.is_shadow_host())
        return false;

    auto result = host_element.attach_a_shadow_root(*shadowrootmode, shadowrootclonable, shadowrootserializable, shadowrootdelegatesfocus, Web::Bindings::SlotAssignmentMode::Named);
    if (result.is_error())
        return false;

    auto& shadow = *host_element.shadow_root();
    shadow.set_declarative(true);
    tmpl->set_template_contents(shadow);
    shadow.set_available_to_element_internals(true);
    return true;
}

void html_parser_bridge_reparent_children(void* from_ptr, void* to_ptr)
{
    auto& from = *static_cast<Web::DOM::Node*>(from_ptr);
    auto& to = *static_cast<Web::DOM::Node*>(to_ptr);
    while (auto* child = from.first_child()) {
        MUST(to.append_child(MUST(from.remove_child(*child))));
    }
}

void html_parser_bridge_visit_node(void* visitor_ptr, void* node_ptr)
{
    if (!visitor_ptr || !node_ptr)
        return;
    auto& visitor = *static_cast<GC::Cell::Visitor*>(visitor_ptr);
    visitor.visit(*static_cast<Web::DOM::Node*>(node_ptr));
}

} // extern "C"

#endif // ENABLE_RUST
