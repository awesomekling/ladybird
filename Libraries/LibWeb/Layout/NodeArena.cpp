/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Assertions.h>
#include <AK/Debug.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Painting/PaintingRustBridge.h>

namespace Web::Layout {

NodeArena::NodeArena()
    : m_render_document(RustFFI::render_owner_create_document())
{
    VERIFY(m_render_document.arena);
    Painting::register_geometry_host(*this);
}

NodeArena::~NodeArena()
{
    RustFFI::render_owner_destroy_document(m_render_document);
}

Compositing::RustFFI::NodeSlotId NodeArena::allocate(RustFFI::FfiNodeConstructionFacts const& construction_facts)
{
    return RustFFI::layout_arena_allocate(handle(), construction_facts);
}

void NodeArena::free_subtree(Compositing::RustFFI::NodeSlotId root)
{
    RustFFI::layout_arena_free_subtree(handle(), root);
}

Node* NodeArena::node_if_live(Compositing::RustFFI::NodeSlotId slot) const
{
    return static_cast<Node*>(RustFFI::layout_arena_node_shell_if_live(handle(), slot));
}

Row NodeArena::row_if_live(Compositing::RustFFI::NodeSlotId slot) const
{
    return { *this, RustFFI::layout_arena_row_if_live(handle(), slot) };
}

Row NodeArena::bound_row(CSS::StyleNodeID style_node, u8 generated_for) const
{
    return { *this, RustFFI::layout_arena_bound_row_of(handle(), style_node.value(), generated_for) };
}

Row NodeArena::bound_viewport_row() const
{
    return { *this, RustFFI::layout_arena_bound_viewport_row(handle()) };
}

Row::Row(NodeArena const& arena, RustFFI::FfiBoundRow const& row)
{
    if (row.slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return;
    m_arena = &arena;
    m_slot = row.slot;
    m_shell = static_cast<Node*>(row.shell);
    m_kind = row.kind;
}

Row::Row(Node const& node)
    : m_arena(&node.node_arena())
    , m_slot(Node::slot_id(&node))
    , m_shell(const_cast<Node*>(&node))
    , m_kind(node.kind())
{
}

void* Row::arena_handle() const
{
    return m_arena->handle();
}

DOM::Document& Row::document() const
{
    auto* document = m_arena->document();
    VERIFY(document);
    return *document;
}

Node& Row::shell() const
{
    if (!m_shell) {
        m_shell = m_arena->node_if_live(m_slot);
        VERIFY(m_shell);
    }
    return *m_shell;
}

Row Row::linked(RustFFI::FfiNodeLink link) const
{
    return { *m_arena, RustFFI::layout_arena_linked_row(m_arena->handle(), m_slot, link) };
}

u32 Row::flags() const
{
    return RustFFI::layout_arena_node_flags(m_arena->handle(), m_slot);
}

DOM::NodeIdentity Row::dom_node_identity() const
{
    if (is_anonymous())
        return {};
    if (m_kind == RustFFI::NodeKind::Viewport)
        return DOM::NodeIdentity::of_document();
    return DOM::NodeIdentity::of_style_node(CSS::StyleNodeID { RustFFI::layout_arena_node_style_node(m_arena->handle(), m_slot) });
}

void const* Row::style_payloads() const
{
    // A text row holds no style of its own.
    VERIFY(!is_text());
    if (m_shell)
        return static_cast<NodeWithStyle const&>(*m_shell).style_payloads();
    return RustFFI::layout_arena_node_style_payloads(m_arena->handle(), m_slot);
}

CSS::Display Row::display() const
{
    return CSS::display_from_ffi_display(NodeWithStyle::style_group_of<CSS::ComputedValues::BoxValues>(style_payloads()).display);
}

void Row::set_needs_layout_update(DOM::SetNeedsLayoutReason reason, LayoutUpdatePropagation propagation) const
{
    if constexpr (UPDATE_LAYOUT_DEBUG) {
        // NOTE: We check some conditions here to avoid debug spam in documents that don't do layout.
        if (!has_flag(RustFFI::NodeFlag::NeedsLayoutUpdate)) {
            auto navigable = document().navigable();
            if (navigable && navigable->active_document() == GC::Ptr { &document() })
                dbgln_if(UPDATE_LAYOUT_DEBUG, "NEED LAYOUT {}", DOM::to_string(reason));
        }
    }
    RustFFI::layout_arena_set_needs_layout_update(m_arena->handle(), m_slot, propagation == LayoutUpdatePropagation::ThroughAncestors);
}

u64 NodeArena::table_cell_measurement_cache_miss_count() const
{
    return RustFFI::layout_arena_table_cell_measurement_cache_miss_count(handle());
}

u64 NodeArena::retained_inline_item_count() const
{
    return RustFFI::layout_arena_retained_inline_item_count(handle());
}

u64 NodeArena::intrinsic_measurement_count() const
{
    return RustFFI::layout_arena_intrinsic_measurement_count(handle());
}

u64 NodeArena::intrinsic_inline_measurement_count() const
{
    return RustFFI::layout_arena_intrinsic_inline_measurement_count(handle());
}

bool destroy_layout_subtree(Node& node)
{
    return RustFFI::layout_arena_detach_and_free_subtree(node.arena_handle(), Node::slot_id(&node));
}

}
