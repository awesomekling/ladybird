/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Assertions.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Painting/PaintingRustBridge.h>

namespace Web::Layout {

Row::Row(DOM::Document const& document, RustFFI::FfiBoundRow const& row)
{
    if (row.slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return;
    m_document = &document;
    m_slot = row.slot;
    m_shell = static_cast<Node*>(row.shell);
    m_kind = row.kind;
}

Row::Row(Node const& node)
    : m_document(&node.document())
    , m_slot(Node::slot_id(&node))
    , m_shell(const_cast<Node*>(&node))
    , m_kind(node.kind())
{
}

void* Row::arena_handle() const
{
    return m_document->layout_arena_handle();
}

Node& Row::shell() const
{
    if (!m_shell) {
        m_shell = static_cast<Node*>(RustFFI::layout_arena_node_shell_if_live(arena_handle(), m_slot));
        VERIFY(m_shell);
    }
    return *m_shell;
}

Row Row::linked(RustFFI::FfiNodeLink link) const
{
    return { *m_document, RustFFI::layout_arena_linked_row(arena_handle(), m_slot, link) };
}

u32 Row::flags() const
{
    return RustFFI::layout_arena_node_flags(arena_handle(), m_slot);
}

DOM::NodeIdentity Row::dom_node_identity() const
{
    if (is_anonymous())
        return {};
    if (m_kind == RustFFI::NodeKind::Viewport)
        return DOM::NodeIdentity::of_document();
    return DOM::NodeIdentity::of_style_node(CSS::StyleNodeID { RustFFI::layout_arena_node_style_node(arena_handle(), m_slot) });
}

void const* Row::style_payloads() const
{
    // A text row holds no style of its own.
    VERIFY(!is_text());
    if (m_shell)
        return static_cast<NodeWithStyle const&>(*m_shell).style_payloads();
    return RustFFI::layout_arena_node_style_payloads(arena_handle(), m_slot);
}

CSS::Display Row::display() const
{
    return CSS::display_from_ffi_display(NodeWithStyle::style_group_of<CSS::ComputedValues::BoxValues>(style_payloads()).display);
}

bool destroy_layout_subtree(Node& node)
{
    return RustFFI::layout_arena_detach_and_free_subtree(node.arena_handle(), Node::slot_id(&node));
}

}
