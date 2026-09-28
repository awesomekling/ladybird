/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StringBuilder.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/PseudoElement.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxSlot.h>

namespace Web::Painting {

static void* arena_of(DOM::Document const& document)
{
    return Layout::document_layout_arena_if_created(document);
}

BoxSlot::BoxSlot(DOM::Document const& document, Layout::RustFFI::FfiBoundRow const& row)
{
    if (row.slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return;
    m_document = const_cast<DOM::Document&>(document);
    m_slot = row.slot;
    m_kind = row.kind;
}

BoxSlot BoxSlot::of(DOM::Document const& document, Compositing::RustFFI::NodeSlotId slot)
{
    auto* arena = arena_of(document);
    if (!arena || slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return {};
    return { document, Layout::RustFFI::layout_arena_row_if_live(arena, slot) };
}

BoxSlot BoxSlot::bound_to(DOM::Document const& document, DOM::NodeIdentity identity, Optional<CSS::PseudoElement> pseudo_element)
{
    auto* arena = arena_of(document);
    if (!arena || !identity)
        return {};
    if (identity == DOM::NodeIdentity::of_document())
        return pseudo_element.has_value() ? BoxSlot {} : viewport_of(document);
    if (identity.style_node().value() == 0)
        return {};
    u8 generated_for = pseudo_element.has_value() ? static_cast<u8>(to_underlying(*pseudo_element) + 1) : 0;
    return { document, Layout::RustFFI::layout_arena_bound_row_of(arena, identity.style_node().value(), generated_for) };
}

BoxSlot BoxSlot::bound_to(DOM::Node const& node)
{
    return bound_to(node.document(), DOM::NodeIdentity::of(node));
}

BoxSlot BoxSlot::of_pseudo_element(DOM::Element const& element, CSS::PseudoElement pseudo_element)
{
    if (CSS::is_synthetic_pseudo_element(pseudo_element))
        return bound_to(element.document(), DOM::NodeIdentity::of(element), pseudo_element);
    if (auto data = element.get_pseudo_element(pseudo_element); data.has_value()) {
        if (auto const* reference = as_if<DOM::ElementReferencePseudoElement>(*data))
            return bound_to(*reference->referenced_element());
    }
    return {};
}

BoxSlot BoxSlot::viewport_of(DOM::Document const& document)
{
    auto* arena = arena_of(document);
    if (!arena)
        return {};
    return { document, Layout::RustFFI::layout_arena_bound_viewport_row(arena) };
}

void* BoxSlot::arena() const
{
    return m_document ? arena_of(*m_document) : nullptr;
}

bool BoxSlot::is_live() const
{
    auto* arena = this->arena();
    return arena && *this && Layout::RustFFI::layout_arena_row_if_live(arena, m_slot).slot.index == m_slot.index;
}

bool BoxSlot::has_committed_box() const
{
    auto* arena = this->arena();
    return arena && *this && Layout::RustFFI::layout_arena_has_committed_box(arena, m_slot);
}

bool BoxSlot::is_atomic_inline() const
{
    auto* arena = this->arena();
    return arena && *this && Layout::RustFFI::layout_arena_node_is_atomic_inline(arena, m_slot);
}

bool BoxSlot::is_scroll_container() const
{
    // NOTE: This isn't in the spec, but we want the viewport to behave like a scroll container.
    if (is_viewport())
        return true;
    auto const* box_values = style_group<CSS::ComputedValues::BoxValues>();
    return box_values
        && (overflow_value_makes_box_a_scroll_container(static_cast<CSS::Overflow>(box_values->overflow_x))
            || overflow_value_makes_box_a_scroll_container(static_cast<CSS::Overflow>(box_values->overflow_y)));
}

StringView BoxSlot::kind_name() const
{
    return layout_node_kind_name(m_kind);
}

u32 BoxSlot::flags() const
{
    auto* arena = this->arena();
    if (!arena || !*this)
        return 0;
    return Layout::RustFFI::layout_arena_node_flags(arena, m_slot);
}

BoxSlot BoxSlot::linked(Layout::RustFFI::FfiNodeLink link) const
{
    auto* arena = this->arena();
    if (!arena || !*this)
        return {};
    return { *m_document, Layout::RustFFI::layout_arena_linked_row(arena, m_slot, link) };
}

BoxSlot BoxSlot::next_in_pre_order(BoxSlot const& root) const
{
    if (auto child = first_child())
        return child;
    for (auto box = *this; box && box != root; box = box.parent()) {
        if (auto sibling = box.next_sibling())
            return sibling;
    }
    return {};
}

BoxSlot BoxSlot::containing_block() const
{
    auto* arena = this->arena();
    if (!arena || !*this)
        return {};
    return of(*m_document, Layout::RustFFI::layout_arena_node_containing_block_slot_if_live(arena, m_slot));
}

CSS::StyleNodeID BoxSlot::style_node() const
{
    auto* arena = this->arena();
    if (!arena || !*this)
        return {};
    return CSS::StyleNodeID { Layout::RustFFI::layout_arena_node_style_node(arena, m_slot) };
}

DOM::NodeIdentity BoxSlot::dom_node_identity() const
{
    if (!*this || is_anonymous())
        return {};
    // The document has no StyleNodeID; its row is the viewport.
    if (is_viewport())
        return DOM::NodeIdentity::of_document();
    // A row kept after its node was removed has a StyleNodeID of 0 and names nothing.
    return DOM::NodeIdentity::of_style_node(style_node());
}

GC::Ptr<DOM::Node> BoxSlot::dom_node() const
{
    if (!*this)
        return nullptr;
    return dom_node_identity().resolve(*m_document);
}

Optional<CSS::PseudoElement> BoxSlot::generated_for_pseudo_element() const
{
    auto* arena = this->arena();
    if (!arena || !*this)
        return {};
    auto generated_for = Layout::RustFFI::layout_arena_node_generated_for(arena, m_slot);
    if (generated_for == 0)
        return {};
    return static_cast<CSS::PseudoElement>(generated_for - 1);
}

DOM::NodeIdentity BoxSlot::pseudo_element_generator_identity() const
{
    // A stale row's StyleNodeID is 0 once its generator disconnects, so it names nothing.
    return DOM::NodeIdentity::of_style_node(style_node());
}

GC::Ptr<DOM::Element> BoxSlot::pseudo_element_generator() const
{
    if (!*this)
        return nullptr;
    return as_if<DOM::Element>(pseudo_element_generator_identity().resolve(*m_document).ptr());
}

void const* BoxSlot::style_payloads() const
{
    auto* arena = this->arena();
    if (!arena || !*this || is_text())
        return nullptr;
    return Layout::RustFFI::layout_arena_node_style_payloads(arena, m_slot);
}

String BoxSlot::debug_description() const
{
    StringBuilder builder;
    builder.append(kind_name());
    append_dom_node_debug_description(builder, dom_node());
    return MUST(builder.to_string());
}

void append_dom_node_debug_description(StringBuilder& builder, GC::Ptr<DOM::Node const> node)
{
    if (!node) {
        builder.append("(anonymous)"sv);
        return;
    }
    builder.appendff("<{}>", node->node_name());
    if (auto const* element = as_if<DOM::Element>(*node)) {
        if (element->id().has_value())
            builder.appendff("#{}", element->id().value());
        for (auto const& class_name : element->class_names())
            builder.appendff(".{}", class_name);
    }
}

void describe_dom_node_for_debug(DOM::Document& document, u32 node, void* sink, void (*append)(void*, u8 const*, size_t))
{
    auto identity = node == 0 ? DOM::NodeIdentity::of_document() : DOM::NodeIdentity::of_style_node(CSS::StyleNodeID { node });
    StringBuilder builder;
    append_dom_node_debug_description(builder, identity.resolve(document));
    auto bytes = builder.string_view().bytes();
    append(sink, bytes.data(), bytes.size());
}

bool overflow_value_makes_box_a_scroll_container(CSS::Overflow overflow)
{
    switch (overflow) {
    case CSS::Overflow::Clip:
    case CSS::Overflow::Visible:
        return false;
    case CSS::Overflow::Auto:
    case CSS::Overflow::Hidden:
    case CSS::Overflow::Scroll:
        return true;
    }
    VERIFY_NOT_REACHED();
}

StringView layout_node_kind_name(Layout::RustFFI::NodeKind kind)
{
#define LAYOUT_NODE_KIND_NAME_CASE(kind_name)  \
    case Layout::RustFFI::NodeKind::kind_name: \
        return #kind_name##sv;
    switch (kind) {
        LAYOUT_NODE_KIND_NAME_CASE(AudioBox)
        LAYOUT_NODE_KIND_NAME_CASE(BlockContainer)
        LAYOUT_NODE_KIND_NAME_CASE(Box)
        LAYOUT_NODE_KIND_NAME_CASE(BreakNode)
        LAYOUT_NODE_KIND_NAME_CASE(CanvasBox)
        LAYOUT_NODE_KIND_NAME_CASE(CheckBox)
        LAYOUT_NODE_KIND_NAME_CASE(FieldSetBox)
        LAYOUT_NODE_KIND_NAME_CASE(GeneratedTextNode)
        LAYOUT_NODE_KIND_NAME_CASE(ImageBox)
        LAYOUT_NODE_KIND_NAME_CASE(InlineNode)
        LAYOUT_NODE_KIND_NAME_CASE(LegendBox)
        LAYOUT_NODE_KIND_NAME_CASE(ListItemBox)
        LAYOUT_NODE_KIND_NAME_CASE(ListItemMarkerBox)
        LAYOUT_NODE_KIND_NAME_CASE(NavigableContainerViewport)
        LAYOUT_NODE_KIND_NAME_CASE(Node)
        LAYOUT_NODE_KIND_NAME_CASE(NodeWithStyle)
        LAYOUT_NODE_KIND_NAME_CASE(RadioButton)
        LAYOUT_NODE_KIND_NAME_CASE(RangeInputBox)
        LAYOUT_NODE_KIND_NAME_CASE(ReplacedBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGClipBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGForeignObjectBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGGeometryBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGGraphicsBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGImageBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGMaskBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGPatternBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGSVGBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGTextBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGTextPathBox)
        LAYOUT_NODE_KIND_NAME_CASE(TableWrapper)
        LAYOUT_NODE_KIND_NAME_CASE(TextAreaBox)
        LAYOUT_NODE_KIND_NAME_CASE(TextInputBox)
        LAYOUT_NODE_KIND_NAME_CASE(TextNode)
        LAYOUT_NODE_KIND_NAME_CASE(VideoBox)
        LAYOUT_NODE_KIND_NAME_CASE(Viewport)
    case Layout::RustFFI::NodeKind::Unset:
        break;
    }
#undef LAYOUT_NODE_KIND_NAME_CASE
    return "Unset"sv;
}

}
