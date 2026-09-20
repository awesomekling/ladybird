/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/InvalidationJournal.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Layout/TextNode.h>
#include <LibWeb/Painting/BoxViews.h>

namespace Web::DOM {

InvalidationJournal::Entry& InvalidationJournal::entry_for(NodeIdentity identity)
{
    auto index = m_entry_index_by_identity.ensure(identity, [&] {
        m_entries.append(Entry { .identity = identity });
        return m_entries.size() - 1;
    });
    return m_entries[index];
}

void InvalidationJournal::note_needs_layout_update(NodeIdentity identity, SetNeedsLayoutReason reason, Layout::LayoutUpdatePropagation propagation)
{
    auto& entry = entry_for(identity);
    if (!entry.needs_layout_update) {
        entry.needs_layout_update = true;
        entry.layout_reason = reason;
        entry.layout_propagation = propagation;
    } else if (propagation == Layout::LayoutUpdatePropagation::ThroughAncestors) {
        // Marking the ancestors as well covers marking only the node, so the wider mark wins.
        entry.layout_propagation = propagation;
    }
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_needs_repaint(NodeIdentity identity, InvalidateDisplayList invalidate_display_list)
{
    auto& entry = entry_for(identity);
    entry.needs_repaint = true;
    // Each level of display list invalidation covers the one below it, so the widest mark wins.
    entry.invalidate_display_list = max(entry.invalidate_display_list, invalidate_display_list);
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_needs_layout_tree_update(NodeIdentity identity, SetNeedsLayoutTreeUpdateReason reason)
{
    auto& entry = entry_for(identity);
    if (!entry.needs_layout_tree_update) {
        entry.needs_layout_tree_update = true;
        entry.layout_tree_update_reason = reason;
    }
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_dom_paint_facts(NodeIdentity identity, u8 facts)
{
    auto& entry = entry_for(identity);
    entry.has_dom_paint_facts = true;
    entry.dom_paint_facts = facts;
    drain_if_the_render_side_is_reading();
}

// A mark made from inside a layout update is one the render side is about to read, so it goes
// through at once. Outside one, nothing reads what these marks change before the next drain.
void InvalidationJournal::drain_if_the_render_side_is_reading()
{
    if (m_document.is_running_update_layout())
        drain();
}

void InvalidationJournal::drain()
{
    while (!m_entries.is_empty()) {
        auto entries = move(m_entries);
        m_entry_index_by_identity.clear_with_capacity();

        auto* arena = m_document.layout_node_arena_if_created();

        for (auto const& entry : entries) {
            auto node = entry.identity.resolve(m_document);
            if (entry.needs_layout_tree_update && node) {
                // A node that left the tree between the mark and here is on no path the build walks,
                // and the mutation that took it out dirtied the parent it left.
                node->apply_layout_tree_update_mark(entry.layout_tree_update_reason);
            }

            if (!entry.needs_layout_update && !entry.needs_repaint && !entry.has_dom_paint_facts)
                continue;
            // A node whose box went away between the mark and here has nothing left to mark.
            auto* layout_node = arena ? entry.identity.bound_layout_node(*arena) : nullptr;
            if (!layout_node)
                continue;
            if (entry.needs_layout_update)
                layout_node->set_needs_layout_update(entry.layout_reason, entry.layout_propagation);
            if (entry.has_dom_paint_facts) {
                auto changed = Layout::RustFFI::layout_arena_set_node_dom_paint_facts(layout_node->arena_handle(), Layout::Node::slot_id(layout_node), entry.dom_paint_facts);
                if (changed && node)
                    node->set_needs_repaint();
            }
            if (entry.needs_repaint) {
                if (auto* text_node = as_if<Layout::TextNode>(*layout_node))
                    text_node->set_needs_repaint(entry.invalidate_display_list);
                else if (Painting::has_committed_box(*layout_node))
                    Painting::set_needs_repaint(*layout_node, entry.invalidate_display_list);
            }
        }
    }
}

}
