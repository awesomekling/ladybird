/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/InvalidationJournal.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>

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

// A mark made from inside a layout update is one the render side is about to read, so it goes
// through at once. Outside one, nothing reads what these marks change before the next drain.
void InvalidationJournal::drain_if_the_render_side_is_reading()
{
    if (m_document.is_running_update_layout())
        drain();
}

void InvalidationJournal::drain()
{
    if (m_entries.is_empty())
        return;

    auto entries = move(m_entries);
    m_entry_index_by_identity.clear_with_capacity();

    auto* arena = m_document.layout_node_arena_if_created();
    if (!arena)
        return;

    for (auto const& entry : entries) {
        // A node whose box went away between the mark and here has nothing left to mark.
        auto* layout_node = entry.identity.bound_layout_node(*arena);
        if (!layout_node)
            continue;
        if (entry.needs_layout_update)
            layout_node->set_needs_layout_update(entry.layout_reason, entry.layout_propagation);
    }
}

}
