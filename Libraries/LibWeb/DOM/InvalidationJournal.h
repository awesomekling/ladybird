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
    void note_needs_layout_tree_update(NodeIdentity, SetNeedsLayoutTreeUpdateReason);
    void note_dom_paint_facts(NodeIdentity, u8 facts);
    void note_canvas_paint_facts(NodeIdentity, bool has_content, i32 content_width, i32 content_height, u64 canvas_id, u64 content_generation);

    // Writes every entry through to the render side and empties the journal.
    void drain();

private:
    struct Entry {
        NodeIdentity identity;
        // The reason of the first layout mark. Only the layout update trace reads it.
        SetNeedsLayoutReason layout_reason { SetNeedsLayoutReason::StyleChange };
        Layout::LayoutUpdatePropagation layout_propagation {};
        // The reason of the tree update mark that made the node dirty. A later mark on an already
        // dirty node changes nothing about the build, so only the first one's reason is kept.
        SetNeedsLayoutTreeUpdateReason layout_tree_update_reason { SetNeedsLayoutTreeUpdateReason::None };
        InvalidateDisplayList invalidate_display_list { InvalidateDisplayList::No };
        bool needs_layout_update { false };
        bool needs_repaint { false };
        bool needs_layout_tree_update { false };
        bool has_dom_paint_facts { false };
        u8 dom_paint_facts { 0 };
        bool has_canvas_paint_facts { false };
        bool canvas_has_content { false };
        i32 canvas_content_width { 0 };
        i32 canvas_content_height { 0 };
        u64 canvas_id { 0 };
        u64 canvas_content_generation { 0 };
    };

    Entry& entry_for(NodeIdentity);
    void drain_if_the_render_side_is_reading();

    Document& m_document;
    Vector<Entry> m_entries;
    HashMap<NodeIdentity, size_t> m_entry_index_by_identity;
};

}
