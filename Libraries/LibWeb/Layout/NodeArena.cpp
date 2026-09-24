/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Assertions.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Painting/PaintingRustBridge.h>

namespace Web::Layout {

NodeArena::NodeArena()
    : m_handle(RustFFI::layout_arena_create())
{
    VERIFY(m_handle);
    Painting::register_geometry_host(*this);
}

NodeArena::~NodeArena()
{
    RustFFI::layout_arena_destroy(m_handle);
}

Compositing::RustFFI::NodeSlotId NodeArena::allocate(RustFFI::FfiNodeConstructionFacts const& construction_facts)
{
    return RustFFI::layout_arena_allocate(m_handle, construction_facts);
}

void NodeArena::free_subtree(Compositing::RustFFI::NodeSlotId root)
{
    RustFFI::layout_arena_free_subtree(m_handle, root);
}

Node* NodeArena::node_if_live(Compositing::RustFFI::NodeSlotId slot) const
{
    return static_cast<Node*>(RustFFI::layout_arena_node_shell_if_live(m_handle, slot));
}

u64 NodeArena::table_cell_measurement_cache_miss_count() const
{
    return RustFFI::layout_arena_table_cell_measurement_cache_miss_count(m_handle);
}

u64 NodeArena::intrinsic_measurement_count() const
{
    return RustFFI::layout_arena_intrinsic_measurement_count(m_handle);
}

u64 NodeArena::intrinsic_inline_measurement_count() const
{
    return RustFFI::layout_arena_intrinsic_inline_measurement_count(m_handle);
}

bool destroy_layout_subtree(Node& node)
{
    return RustFFI::layout_arena_detach_and_free_subtree(node.arena_handle(), Node::slot_id(&node));
}

}
