/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/HashMap.h>
#include <AK/OwnPtr.h>
#include <AK/Vector.h>
#include <LibWeb/CSS/PseudoElement.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/InvalidateDisplayList.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/PixelUnits.h>

namespace Web::Painting {

enum class PaintCacheInvalidation : u8;
class Scrollbar;

}

namespace Web::DOM {

enum class PaintFactsFamily : u8 {
    LayerImage,
    ReplacedImage,
    Video,
    NavigableContainer,
};

// What the DOM side has marked dirty on the render side but has not written there yet. An entry
// names a node by identity and says what changed about it, and a second mark on the same node
// merges into the entry it already has, so ten writes to one element cost the render side one
// mark. The document drains the journal before anything can observe what the marks did.
class WEB_API InvalidationJournal {
    AK_MAKE_NONCOPYABLE(InvalidationJournal);
    AK_MAKE_NONMOVABLE(InvalidationJournal);

public:
    AK_ALLOC_WITH_KMALLOC;

    explicit InvalidationJournal(Document&);
    ~InvalidationJournal();

    void note_needs_layout_update(NodeIdentity, SetNeedsLayoutReason, Layout::LayoutUpdatePropagation);
    void note_needs_repaint(NodeIdentity, InvalidateDisplayList);
    void note_needs_repaint_in_subtree(NodeIdentity);
    void note_needs_layout_tree_update(NodeIdentity, SetNeedsLayoutTreeUpdateReason);
    void note_dom_paint_facts(NodeIdentity, u8 facts);
    void note_canvas_paint_facts(NodeIdentity, bool has_content, i32 content_width, i32 content_height, u64 canvas_id, u64 content_generation);
    void note_form_control_paint_facts(NodeIdentity, bool enabled, bool checked, bool indeterminate, bool being_activated);
    void note_paint_facts(NodeIdentity, PaintFactsFamily, Function<void(Layout::Node const&)>&&);
    // The row, and every row it descends from, is built for no DOM node an entry could name, so
    // the update is made on the row itself at the drain, if it is still live.
    void note_unanchored_paint_facts(Compositing::RustFFI::NodeSlotId, Function<void(Layout::Node const&)>&&);
    void note_paint_cache_invalidation(NodeIdentity, Painting::PaintCacheInvalidation);
    // Whether the node is an editing host, or a text node that produces a fragment when empty,
    // may have changed, and the rows built for it restamp both at the drain.
    void note_editability_stamps(NodeIdentity);
    // Whether the node is in the shadow tree of the focused text control may have changed, and the
    // node's identity and rows publish the new answer at the drain.
    void note_is_in_focused_text_control(NodeIdentity);
    // The document's selection changed, and the rows it covers restamp their selection states from
    // the selection's range as it is at the drain.
    void note_selection_states();
    // The scroll offset the element or the document's viewport stores changed, and the rows built
    // for it publish the new one at the drain.
    void note_scroll_offset(NodeIdentity, bool offset_changed);
    // The render side is where a pseudo-element's scroll offset is stored, so the entry carries it.
    void note_pseudo_element_scroll_offset(NodeIdentity generator, CSS::PseudoElement, CSSPixelPoint, bool offset_changed);
    // Whether the scrollbar is enlarged may have changed, and it publishes the answer it has at the
    // drain to the row it is built for.
    void note_scrollbar_enlarged_state(Painting::Scrollbar&);
    // The text node's data changed, and the mirror and the text's box take what it holds at the
    // drain. Whether it was nothing but ASCII whitespace may have changed as well.
    void note_text_data(Text&, bool whitespace_state_changed);
    // The SVG element's presentation attributes changed, and the rows built for it take the values
    // it parses at the drain.
    void note_svg_attribute_facts(NodeIdentity);
    // The table cell's or column's span attributes changed, and the mirror and the row built for it
    // take the spans it has at the drain.
    void note_table_spans(NodeIdentity);
    // The accumulated visual contexts built from the row's box need the given kind of update.
    void note_visual_context_box_dirty(Compositing::RustFFI::NodeSlotId, Layout::RustFFI::FfiVisualContextBoxDirtyKind);
    // Every accumulated visual context needs rebuilding, for the given reason.
    void note_visual_context_full_rebuild(Layout::RustFFI::FfiVisualContextGlobalRebuildReason);
    // The SVG paint resources may have changed, and the enrolled ones resync at the next visual
    // context update.
    void note_svg_paint_resources_changed();
    // The visual viewport moved or zoomed, and the visual context tree takes the transform it has
    // at the drain.
    void note_visual_viewport_transform();

    // Writes every entry through to the render side and empties the journal.
    void drain();
    bool is_empty() const;

    // The journal the document keeps beside its frame in flight holds the next generation of marks.
    // It never reaches the render side, which the frame owns: nothing it notes writes through, and
    // it is drained only once the document holds the journal again, after the frame is over.
    void set_holds_next_generation(bool);

private:
    struct PseudoElementScrollOffset {
        CSS::PseudoElement type;
        CSSPixelPoint offset;
    };

    // What few entries carry: kept out of line, so the entries every insertion and style change makes stay small
    // and move cheaply as the journal grows.
    struct RareFacts {
        AK_ALLOC_WITH_KMALLOC;

        bool has_canvas_paint_facts { false };
        bool canvas_has_content { false };
        i32 canvas_content_width { 0 };
        i32 canvas_content_height { 0 };
        u64 canvas_id { 0 };
        u64 canvas_content_generation { 0 };
        bool has_form_control_paint_facts { false };
        bool form_control_enabled { false };
        bool form_control_checked { false };
        bool form_control_indeterminate { false };
        bool form_control_being_activated { false };
        Function<void(Layout::Node const&)> layer_image_paint_facts_update;
        Function<void(Layout::Node const&)> replaced_image_paint_facts_update;
        Function<void(Layout::Node const&)> video_paint_facts_update;
        Function<void(Layout::Node const&)> navigable_container_paint_facts_update;
        Vector<PseudoElementScrollOffset, 1> pseudo_element_scroll_offsets;
    };

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
        bool needs_subtree_repaint { false };
        bool needs_layout_tree_update { false };
        bool has_dom_paint_facts { false };
        u8 dom_paint_facts { 0 };
        bool invalidate_paint_and_hit_test_cache { false };
        bool invalidate_propagated_text_decoration_caches { false };
        bool needs_editability_stamps_refresh { false };
        bool needs_focused_text_control_publish { false };
        bool needs_scroll_offset_publish { false };
        bool needs_text_data_publish { false };
        bool text_whitespace_state_changed { false };
        bool needs_svg_attribute_facts_publish { false };
        bool needs_table_spans_publish { false };
        OwnPtr<RareFacts> rare;

        RareFacts& ensure_rare()
        {
            if (!rare)
                rare = make<RareFacts>();
            return *rare;
        }
    };

    Entry& entry_for(NodeIdentity);
    void drain_if_the_render_side_is_reading();
    void publish_scroll_offsets(Node&, Entry const&);
    void publish_selection_states();
    void publish_visual_context_marks();
    void publish_unanchored_paint_facts();

    Document& m_document;
    Vector<Entry> m_entries;
    HashMap<NodeIdentity, size_t> m_entry_index_by_identity;
    // Whether a noted scroll offset changed, so the document's scroll state mirrors a stale one.
    bool m_scroll_state_is_stale { false };
    bool m_selection_states_are_stale { false };
    struct UnanchoredPaintFacts {
        Compositing::RustFFI::NodeSlotId slot;
        Function<void(Layout::Node const&)> update;
    };
    Vector<UnanchoredPaintFacts> m_unanchored_paint_facts;
    Vector<NonnullRefPtr<Painting::Scrollbar>> m_scrollbars_with_stale_enlarged_state;
    struct VisualContextBoxDirtyMark {
        Compositing::RustFFI::NodeSlotId slot;
        Layout::RustFFI::FfiVisualContextBoxDirtyKind kind;
    };
    Vector<VisualContextBoxDirtyMark> m_visual_context_box_dirty_marks;
    Vector<Layout::RustFFI::FfiVisualContextGlobalRebuildReason, 1> m_visual_context_full_rebuild_reasons;
    bool m_svg_paint_resources_changed { false };
    bool m_visual_viewport_transform_is_stale { false };
    bool m_draining { false };
    bool m_holds_next_generation { false };
};

}
