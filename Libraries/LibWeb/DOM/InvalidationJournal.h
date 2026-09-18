/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/Vector.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/InvalidateDisplayList.h>

namespace Web::DOM {

// What the DOM side has marked dirty on the render side but has not written there yet. An entry
// names a node by identity and says what changed about it, and a second mark on the same node
// merges into the entry it already has, so ten writes to one element cost the render side one
// mark. The document drains the journal before anything can observe what the marks did.
class WEB_API InvalidationJournal {
    AK_MAKE_NONCOPYABLE(InvalidationJournal);
    AK_MAKE_NONMOVABLE(InvalidationJournal);

public:
    AK_ALLOC_WITH_KMALLOC;

    explicit InvalidationJournal(Document& document)
        : m_document(document)
    {
    }

    void note_needs_layout_update(NodeIdentity, SetNeedsLayoutReason, Layout::LayoutUpdatePropagation);
    void note_needs_repaint(NodeIdentity, InvalidateDisplayList);

    // Writes every entry through to the render side and empties the journal.
    void drain();

private:
    struct Entry {
        NodeIdentity identity;
        // The reason of the first layout mark. Only the layout update trace reads it.
        SetNeedsLayoutReason layout_reason { SetNeedsLayoutReason::StyleChange };
        Layout::LayoutUpdatePropagation layout_propagation {};
        InvalidateDisplayList invalidate_display_list { InvalidateDisplayList::No };
        bool needs_layout_update { false };
        bool needs_repaint { false };
    };

    Entry& entry_for(NodeIdentity);
    void drain_if_the_render_side_is_reading();

    Document& m_document;
    Vector<Entry> m_entries;
    HashMap<NodeIdentity, size_t> m_entry_index_by_identity;
};

}
