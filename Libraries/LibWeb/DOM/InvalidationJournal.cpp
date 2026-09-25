/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/TemporaryChange.h>
#include <LibWeb/CSS/StyleEngineInput.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/InvalidationJournal.h>
#include <LibWeb/DOM/Range.h>
#include <LibWeb/DOM/Text.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Layout/TextNode.h>
#include <LibWeb/Layout/Viewport.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/Scrollbar.h>
#include <LibWeb/SVG/SVGElement.h>
#include <LibWeb/Selection/Selection.h>

namespace Web::DOM {

// The main-side access census counts how often the DOM side reaches render-owned state while the
// journal holds marks the render side has not taken yet. It learns that from here, and only while
// it counts.
static void report_journal_pending_to_census(Document& document, bool pending)
{
    static bool const census_enabled = Layout::RustFFI::layout_main_side_census_enabled();
    if (!census_enabled)
        return;
    if (auto* arena = document.layout_node_arena_if_created())
        Layout::RustFFI::layout_arena_note_invalidation_journal_pending(arena->handle(), pending);
}

// A frame in flight never waits for the journal, so what is journaled while one is in flight is main-side work that
// ran beside the frame.
static void count_entry_during_flight(HTML::EventLoop::JournalEntryKind kind)
{
    if (HTML::EventLoop::a_frame_is_in_flight())
        HTML::main_thread_event_loop().note_journal_entry_during_flight(kind);
}

InvalidationJournal::InvalidationJournal(Document& document)
    : m_document(document)
{
}

InvalidationJournal::~InvalidationJournal() = default;

// The census reads its flag through the arena's host tables, which wait for the frame in flight, so
// the next generation's marks count as pending once the document holds them.
void InvalidationJournal::set_holds_next_generation(bool holds_next_generation)
{
    m_holds_next_generation = holds_next_generation;
    if (!holds_next_generation && !is_empty())
        report_journal_pending_to_census(m_document, true);
}

bool InvalidationJournal::is_empty() const
{
    return m_entries.is_empty()
        && !m_selection_states_are_stale
        && m_unanchored_paint_facts.is_empty()
        && m_scrollbars_with_stale_enlarged_state.is_empty()
        && m_visual_context_box_dirty_marks.is_empty()
        && m_visual_context_full_rebuild_reasons.is_empty()
        && !m_svg_paint_resources_changed
        && !m_visual_viewport_transform_is_stale;
}

InvalidationJournal::Entry& InvalidationJournal::entry_for(NodeIdentity identity)
{
    if (is_empty() && !m_holds_next_generation)
        report_journal_pending_to_census(m_document, true);
    auto index = m_entry_index_by_identity.ensure(identity, [&] {
        m_entries.append(Entry {
            .identity = identity,
            .layer_image_paint_facts_update = {},
            .replaced_image_paint_facts_update = {},
            .video_paint_facts_update = {},
            .navigable_container_paint_facts_update = {},
            .pseudo_element_scroll_offsets = {},
        });
        return m_entries.size() - 1;
    });
    return m_entries[index];
}

void InvalidationJournal::note_needs_layout_update(NodeIdentity identity, SetNeedsLayoutReason reason, Layout::LayoutUpdatePropagation propagation)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::LayoutUpdate);
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
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::Repaint);
    auto& entry = entry_for(identity);
    entry.needs_repaint = true;
    // Each level of display list invalidation covers the one below it, so the widest mark wins.
    entry.invalidate_display_list = max(entry.invalidate_display_list, invalidate_display_list);
    m_document.request_frame_for_journalled_repaint({});
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_needs_repaint_in_subtree(NodeIdentity identity)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::Repaint);
    auto& entry = entry_for(identity);
    entry.needs_subtree_repaint = true;
    entry.needs_repaint = true;
    entry.invalidate_display_list = InvalidateDisplayList::PaintCommandsAndHitTestList;
    m_document.request_frame_for_journalled_repaint({});
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_needs_layout_tree_update(NodeIdentity identity, SetNeedsLayoutTreeUpdateReason reason)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::LayoutTreeUpdate);
    auto& entry = entry_for(identity);
    if (!entry.needs_layout_tree_update) {
        entry.needs_layout_tree_update = true;
        entry.layout_tree_update_reason = reason;
    }
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_dom_paint_facts(NodeIdentity identity, u8 facts)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    auto& entry = entry_for(identity);
    entry.has_dom_paint_facts = true;
    entry.dom_paint_facts = facts;
    // Applying changed DOM paint facts requests a repaint. Keep the request at mark time so the
    // rendering update that drains the facts cannot wait for the repaint decision in that drain.
    m_document.request_frame_for_journalled_repaint({});
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_canvas_paint_facts(NodeIdentity identity, bool has_content, i32 content_width, i32 content_height, u64 canvas_id, u64 content_generation)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    auto& entry = entry_for(identity);
    entry.has_canvas_paint_facts = true;
    entry.canvas_has_content = has_content;
    entry.canvas_content_width = content_width;
    entry.canvas_content_height = content_height;
    entry.canvas_id = canvas_id;
    entry.canvas_content_generation = content_generation;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_form_control_paint_facts(NodeIdentity identity, bool enabled, bool checked, bool indeterminate, bool being_activated)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    auto& entry = entry_for(identity);
    entry.has_form_control_paint_facts = true;
    entry.form_control_enabled = enabled;
    entry.form_control_checked = checked;
    entry.form_control_indeterminate = indeterminate;
    entry.form_control_being_activated = being_activated;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_paint_facts(NodeIdentity identity, PaintFactsFamily family, Function<void(Layout::Node const&)>&& update)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    auto& entry = entry_for(identity);
    switch (family) {
    case PaintFactsFamily::LayerImage:
        entry.layer_image_paint_facts_update = move(update);
        break;
    case PaintFactsFamily::ReplacedImage:
        entry.replaced_image_paint_facts_update = move(update);
        // Applying changed replaced-image facts requests a repaint; see note_dom_paint_facts().
        m_document.request_frame_for_journalled_repaint({});
        break;
    case PaintFactsFamily::Video:
        entry.video_paint_facts_update = move(update);
        // Applying changed video facts requests a repaint; see note_dom_paint_facts().
        m_document.request_frame_for_journalled_repaint({});
        break;
    case PaintFactsFamily::NavigableContainer:
        entry.navigable_container_paint_facts_update = move(update);
        // Applying changed navigable container facts invalidates the paint cache, which only a
        // rendering update repaints; see note_dom_paint_facts().
        m_document.request_frame_for_journalled_repaint({});
        break;
    }
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_unanchored_paint_facts(Compositing::RustFFI::NodeSlotId slot, Function<void(Layout::Node const&)>&& update)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    if (is_empty() && !m_holds_next_generation)
        report_journal_pending_to_census(m_document, true);
    m_unanchored_paint_facts.append({ slot, move(update) });
    m_document.request_frame_for_journalled_repaint({});
    drain_if_the_render_side_is_reading();
}

// These go through in the order they were noted, since no entry merges them. What they mark for
// repaint lands in the entries, so they go through ahead of them.
void InvalidationJournal::publish_unanchored_paint_facts()
{
    auto updates = move(m_unanchored_paint_facts);
    auto* arena = m_document.layout_node_arena_if_created();
    if (!arena)
        return;
    for (auto const& [slot, update] : updates) {
        if (auto* layout_node = arena->node_if_live(slot))
            update(*layout_node);
    }
}

void InvalidationJournal::note_paint_cache_invalidation(NodeIdentity identity, Painting::PaintCacheInvalidation invalidation)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintCache);
    auto& entry = entry_for(identity);
    switch (invalidation) {
    case Painting::PaintCacheInvalidation::PaintAndHitTest:
        entry.invalidate_paint_and_hit_test_cache = true;
        break;
    case Painting::PaintCacheInvalidation::PropagatedTextDecorations:
        entry.invalidate_propagated_text_decoration_caches = true;
        break;
    }
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_editability_stamps(NodeIdentity identity)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::Editability);
    entry_for(identity).needs_editability_stamps_refresh = true;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_is_in_focused_text_control(NodeIdentity identity)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::Editability);
    entry_for(identity).needs_focused_text_control_publish = true;
    drain_if_the_render_side_is_reading();
}

// Editing-host status and the empty-text fragment behavior of text nodes are stamped into layout
// NodeData at layout node construction; contenteditable and designMode changes reach here without
// a layout tree rebuild, so the stamps must be refreshed. A flipped stamp changes geometry (an
// editing host gains a minimum block size, an empty editable text node gains a zero-width
// fragment), so the affected node also needs a relayout.
static void refresh_editability_stamps(Node& node)
{
    auto* layout_node = node.unsafe_layout_node();
    if (!layout_node)
        return;
    auto is_editing_host = node.is_editing_host();
    if (layout_node->is_editing_host() != is_editing_host) {
        layout_node->set_is_editing_host(is_editing_host);
        node.set_needs_layout_update(SetNeedsLayoutReason::EditableStateChange);
    }
    if (auto* layout_text_node = as_if<Layout::TextNode>(*layout_node)) {
        if (layout_text_node->update_produces_line_box_fragment_when_empty_flag())
            node.set_needs_layout_update(SetNeedsLayoutReason::EditableStateChange);
    }
}

void InvalidationJournal::note_selection_states()
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::Selection);
    if (is_empty() && !m_holds_next_generation)
        report_journal_pending_to_census(m_document, true);
    m_selection_states_are_stale = true;
    drain_if_the_render_side_is_reading();
}

// The selection's range is the one the last association or boundary change left, so restamping
// from it lands every write since the last drain at once.
void InvalidationJournal::publish_selection_states()
{
    if (!m_document.has_committed_viewport_box())
        return;
    auto selection = m_document.get_selection();
    if (auto range = selection ? selection->range() : nullptr)
        m_document.paint_state().recompute_selection_states(*range);
    else
        m_document.paint_state().reset_selection_states(m_document);
}

void InvalidationJournal::note_scroll_offset(NodeIdentity identity, bool offset_changed)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::ScrollOffset);
    entry_for(identity).needs_scroll_offset_publish = true;
    m_scroll_state_is_stale |= offset_changed;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_pseudo_element_scroll_offset(NodeIdentity generator, CSS::PseudoElement type, CSSPixelPoint offset, bool offset_changed)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::ScrollOffset);
    auto& offsets = entry_for(generator).pseudo_element_scroll_offsets;
    if (auto existing = offsets.find_if([&](auto const& pending) { return pending.type == type; }); existing != offsets.end())
        existing->offset = offset;
    else
        offsets.append({ type, offset });
    m_scroll_state_is_stale |= offset_changed;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_scrollbar_enlarged_state(Painting::Scrollbar& scrollbar)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::Scrollbar);
    if (is_empty() && !m_holds_next_generation)
        report_journal_pending_to_census(m_document, true);
    if (!m_scrollbars_with_stale_enlarged_state.contains_slow(NonnullRefPtr { scrollbar }))
        m_scrollbars_with_stale_enlarged_state.append(scrollbar);
    // Publishing a changed state damages the scrollbar's overlay, which only a rendering update
    // repaints.
    m_document.request_frame_for_journalled_repaint({});
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_visual_context_box_dirty(Compositing::RustFFI::NodeSlotId slot, Layout::RustFFI::FfiVisualContextBoxDirtyKind kind)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::VisualContext);
    if (is_empty() && !m_holds_next_generation)
        report_journal_pending_to_census(m_document, true);
    m_visual_context_box_dirty_marks.append({ slot, kind });
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_visual_context_full_rebuild(Layout::RustFFI::FfiVisualContextGlobalRebuildReason reason)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::VisualContext);
    if (is_empty() && !m_holds_next_generation)
        report_journal_pending_to_census(m_document, true);
    if (!m_visual_context_full_rebuild_reasons.contains_slow(reason))
        m_visual_context_full_rebuild_reasons.append(reason);
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_svg_paint_resources_changed()
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::VisualContext);
    if (is_empty() && !m_holds_next_generation)
        report_journal_pending_to_census(m_document, true);
    m_svg_paint_resources_changed = true;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_visual_viewport_transform()
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::VisualContext);
    if (is_empty() && !m_holds_next_generation)
        report_journal_pending_to_census(m_document, true);
    m_visual_viewport_transform_is_stale = true;
    drain_if_the_render_side_is_reading();
}

// A mark on a row that was freed since lands nowhere, and one on a row reused since marks a box
// the next update revisits for nothing.
void InvalidationJournal::publish_visual_context_marks()
{
    auto* arena = m_document.layout_node_arena_if_created();
    auto box_dirty_marks = move(m_visual_context_box_dirty_marks);
    auto full_rebuild_reasons = move(m_visual_context_full_rebuild_reasons);
    if (exchange(m_visual_viewport_transform_is_stale, false)) {
        if (m_document.has_committed_viewport_box() && m_document.paint_state().has_visual_context_tree())
            m_document.paint_state().update_visual_viewport_accumulated_visual_context(m_document);
        else {
            if (!full_rebuild_reasons.contains_slow(Layout::RustFFI::FfiVisualContextGlobalRebuildReason::FirstBuild))
                full_rebuild_reasons.append(Layout::RustFFI::FfiVisualContextGlobalRebuildReason::FirstBuild);
            m_document.set_needs_accumulated_visual_contexts_update(true);
        }
    }
    if (!arena)
        return;
    for (auto reason : full_rebuild_reasons)
        Layout::RustFFI::layout_arena_visual_context_request_full_rebuild(arena->handle(), reason);
    for (auto const& mark : box_dirty_marks)
        Layout::RustFFI::layout_arena_visual_context_note_box_dirty(arena->handle(), mark.slot, mark.kind);
    if (exchange(m_svg_paint_resources_changed, false) && Layout::RustFFI::layout_arena_note_svg_paint_resources_changed(arena->handle()))
        m_document.set_needs_accumulated_visual_contexts_update(true);
}

void InvalidationJournal::note_text_data(Text& text, bool whitespace_state_changed)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::TextData);
    // A text node outside the document's tree has no mirror row and no box to take its data.
    auto identity = NodeIdentity::of(text);
    if (!identity)
        return;
    auto& entry = entry_for(identity);
    entry.needs_text_data_publish = true;
    // A later change can flip the state back, which costs the drain no more than a tree update.
    entry.text_whitespace_state_changed |= whitespace_state_changed;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_svg_attribute_facts(NodeIdentity identity)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::SVGAttributes);
    entry_for(identity).needs_svg_attribute_facts_publish = true;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_table_spans(NodeIdentity identity)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::TableSpans);
    entry_for(identity).needs_table_spans_publish = true;
    drain_if_the_render_side_is_reading();
}

// The spans go under the element's identity for the rows the build has yet to stamp, and onto the
// row it already has, which relays out if they moved.
static void publish_table_spans(Element& element)
{
    Layout::publish_table_spans(element);
    if (auto* layout_node = element.unsafe_layout_node()) {
        if (layout_node->synchronize_table_span_data())
            layout_node->set_needs_layout_update(SetNeedsLayoutReason::TableSpanAttributeChange);
    }
}

// The mirror holds the characters the layout tree build renders, and a text box that already
// exists renders them again once its content is invalidated.
static void publish_text_data(Text& text, bool whitespace_state_changed)
{
    CSS::record_text_data_changed(text);
    if (auto* parent = text.parent()) {
        if (auto* first_letter_owner = parent->first_letter_owner_for_layout_subtree_from(*parent))
            first_letter_owner->set_needs_layout_tree_update(true, SetNeedsLayoutTreeUpdateReason::CharacterDataReplaceData);
    }
    if (whitespace_state_changed)
        CSS::record_text_whitespace_state_changed(text);
    auto* text_layout_node = as_if<Layout::TextNode>(text.unsafe_layout_node());
    if (text_layout_node && Layout::RustFFI::layout_arena_text_has_source_range(text_layout_node->arena_handle(), Layout::Node::slot_id(text_layout_node))) {
        // First-letter source ranges are determined while building the layout tree.
        if (auto* parent = text.parent())
            parent->set_needs_layout_tree_update(true, SetNeedsLayoutTreeUpdateReason::CharacterDataReplaceData);
    } else if (text_layout_node) {
        // NB: Since the text node's data has changed, we need to invalidate the text for rendering.
        //     This ensures that the new text is reflected in layout, even if we don't end up doing a full layout
        //     tree rebuild.
        text_layout_node->invalidate_text_for_rendering();

        // We also need to relayout.
        text_layout_node->set_needs_layout_update(SetNeedsLayoutReason::CharacterDataReplaceData);

        if (whitespace_state_changed)
            text.set_needs_layout_tree_update(true, SetNeedsLayoutTreeUpdateReason::CharacterDataReplaceData);
    } else if (whitespace_state_changed && text.is_connected()) {
        if (auto* parent = text.parent())
            parent->set_needs_layout_tree_update(true, SetNeedsLayoutTreeUpdateReason::CharacterDataReplaceData);
    }
}

// The offsets are read from where the DOM side stores them now, except a pseudo-element's, which
// the entry carries to the render side that stores it.
//
// The layout arena measures a box that holds a scroll offset eagerly after a full commit, so the
// current box re-derives that fact whenever the stored offset changes. Layout need not be up to
// date for that: the box is only annotated, not read, and a box that a pending layout tree rebuild
// replaces is never consulted again, while its replacement derives the fact when it is constructed.
// That is why the unchecked layout node accessor is the right one here.
void InvalidationJournal::publish_scroll_offsets(Node& node, Entry const& entry)
{
    if (auto* document = as_if<Document>(node)) {
        if (auto* layout_node = document->unsafe_layout_node())
            layout_node->publish_scroll_offset();
        return;
    }
    auto* element = as_if<Element>(node);
    if (!element)
        return;
    for (auto const& [type, offset] : entry.pseudo_element_scroll_offsets) {
        auto pseudo_element = element->get_synthetic_pseudo_element(type);
        if (!pseudo_element.has_value())
            continue;
        pseudo_element->set_scroll_offset(offset);
        if (auto* layout_node = pseudo_element->unsafe_layout_node())
            layout_node->publish_scroll_offset();
    }
    if (entry.needs_scroll_offset_publish) {
        Layout::publish_element_scroll_offset(*element);
        if (auto* layout_node = element->unsafe_layout_node())
            layout_node->publish_scroll_offset();
    }
}

// A mark made from inside a layout frame is one the frame is about to read, so it goes through at
// once. Between frames, and beside a frame in flight, nothing reads what these marks change before
// the next drain, and the next frame starts with one.
void InvalidationJournal::drain_if_the_render_side_is_reading()
{
    if (!m_holds_next_generation && m_document.is_running_update_layout())
        drain();
}

void InvalidationJournal::drain()
{
    // Publishing a pseudo-element's offset reads it back, and that read drains. The drain already
    // running takes whatever such a read would have.
    // The drain writes what the frame reads, so a frame in flight is waited for first. Its end hands
    // this journal what was marked beside it.
    // NB: A style pass alone reads nothing the drain writes: it does not own the arena, and what the
    //     drain asks of the style engine joins it at the engine's own entrances.
    VERIFY(!m_holds_next_generation);
    if (auto* arena = m_document.layout_node_arena_if_created()) {
        // An empty journal writes nothing, and a read beside the recording in flight finds it so.
        if (is_empty() && Layout::RustFFI::rust_stage_thread_reads_beside_recording_of(arena->handle()))
            return;
        auto location = SourceLocation::current();
        Layout::RustFFI::layout_arena_join_frame_owning_arena(arena->handle(), reinterpret_cast<u8 const*>(location.filename().characters_without_null_termination()), location.filename().length(), location.line_number());
    }
    if (is_empty() || m_draining)
        return;
    TemporaryChange draining { m_draining, true };

    auto* publication_arena = m_document.layout_node_arena_if_created();
    if (publication_arena)
        Layout::RustFFI::layout_arena_before_invalidation_journal_drain(publication_arena->handle());

    if (exchange(m_selection_states_are_stale, false))
        publish_selection_states();

    // A scrollbar whose row was reset since the mark publishes nothing, and the reset row starts
    // out with no scrollbar enlarged.
    for (auto& scrollbar : exchange(m_scrollbars_with_stale_enlarged_state, {}))
        scrollbar->publish_enlarged_state({});

    publish_unanchored_paint_facts();

    while (!m_entries.is_empty()) {
        auto entries = move(m_entries);
        m_entry_index_by_identity.clear_with_capacity();

        auto* arena = m_document.layout_node_arena_if_created();
        if (!publication_arena && arena) {
            Layout::RustFFI::layout_arena_before_invalidation_journal_drain(arena->handle());
            publication_arena = arena;
        }

        for (auto const& entry : entries) {
            auto node = entry.identity.resolve(m_document);
            if (entry.needs_layout_tree_update && node) {
                // A node that left the tree between the mark and here is on no path the build walks,
                // and the mutation that took it out dirtied the parent it left.
                node->apply_layout_tree_update_mark(entry.layout_tree_update_reason);
            }
            if (entry.needs_text_data_publish && node) {
                if (auto* text = as_if<Text>(*node))
                    publish_text_data(*text, entry.text_whitespace_state_changed);
            }
            if (entry.needs_svg_attribute_facts_publish && node) {
                if (auto* svg_element = as_if<SVG::SVGElement>(*node))
                    Layout::publish_svg_attribute_facts(*svg_element);
            }
            if (entry.needs_table_spans_publish && node) {
                if (auto* element = as_if<Element>(*node))
                    publish_table_spans(*element);
            }
            if (entry.needs_editability_stamps_refresh && node)
                refresh_editability_stamps(*node);
            if (entry.needs_focused_text_control_publish && node)
                Layout::publish_is_in_focused_text_control(*node);
            if (node && (entry.needs_scroll_offset_publish || !entry.pseudo_element_scroll_offsets.is_empty()))
                publish_scroll_offsets(*node, entry);

            if (!entry.needs_layout_update && !entry.needs_repaint && !entry.needs_subtree_repaint && !entry.has_dom_paint_facts && !entry.has_canvas_paint_facts && !entry.has_form_control_paint_facts && !entry.invalidate_paint_and_hit_test_cache && !entry.invalidate_propagated_text_decoration_caches && !entry.layer_image_paint_facts_update && !entry.replaced_image_paint_facts_update && !entry.video_paint_facts_update && !entry.navigable_container_paint_facts_update)
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
            if (entry.has_canvas_paint_facts) {
                Layout::RustFFI::FfiCanvasPaintFacts facts {
                    .has_content = entry.canvas_has_content,
                    .content_width = entry.canvas_content_width,
                    .content_height = entry.canvas_content_height,
                    .canvas_id = entry.canvas_id,
                    .content_generation = entry.canvas_content_generation,
                };
                auto changed = Layout::RustFFI::layout_arena_set_canvas_paint_facts(layout_node->arena_handle(), Layout::Node::slot_id(layout_node), facts);
                if (changed && Painting::has_committed_box(*layout_node))
                    Painting::apply_paint_cache_invalidation(*layout_node, Painting::PaintCacheInvalidation::PaintAndHitTest, Painting::PaintCacheInvalidationStage::JournalDrain);
            }
            if (entry.has_form_control_paint_facts && (layout_node->kind() == Layout::RustFFI::NodeKind::CheckBox || layout_node->kind() == Layout::RustFFI::NodeKind::RadioButton)) {
                Layout::RustFFI::FfiFormControlPaintFacts facts {
                    .enabled = entry.form_control_enabled,
                    .checked = entry.form_control_checked,
                    .indeterminate = entry.form_control_indeterminate,
                    .being_activated = entry.form_control_being_activated,
                };
                auto changed = Layout::RustFFI::layout_arena_set_form_control_paint_facts(layout_node->arena_handle(), Layout::Node::slot_id(layout_node), facts);
                if (changed && Painting::has_committed_box(*layout_node))
                    Painting::set_needs_repaint(*layout_node, InvalidateDisplayList::PaintCommands);
            }
            if (entry.layer_image_paint_facts_update)
                entry.layer_image_paint_facts_update(*layout_node);
            if (entry.replaced_image_paint_facts_update)
                entry.replaced_image_paint_facts_update(*layout_node);
            if (entry.video_paint_facts_update)
                entry.video_paint_facts_update(*layout_node);
            if (entry.navigable_container_paint_facts_update)
                entry.navigable_container_paint_facts_update(*layout_node);
            if (entry.invalidate_paint_and_hit_test_cache)
                Painting::apply_paint_cache_invalidation(*layout_node, Painting::PaintCacheInvalidation::PaintAndHitTest, Painting::PaintCacheInvalidationStage::JournalDrain);
            if (entry.invalidate_propagated_text_decoration_caches)
                Painting::apply_paint_cache_invalidation(*layout_node, Painting::PaintCacheInvalidation::PropagatedTextDecorations, Painting::PaintCacheInvalidationStage::JournalDrain);
            if (entry.needs_subtree_repaint)
                Painting::apply_subtree_repaint_damage(*layout_node, Painting::RepaintDamageStage::JournalDrain);
            if (entry.needs_repaint) {
                if (auto* text_node = as_if<Layout::TextNode>(*layout_node))
                    Painting::apply_repaint_damage(*text_node, entry.invalidate_display_list, Painting::RepaintDamageStage::JournalDrain);
                else if (Painting::has_committed_box(*layout_node))
                    Painting::apply_repaint_damage(*layout_node, entry.invalidate_display_list, Painting::RepaintDamageStage::JournalDrain);
            }
        }
    }

    publish_visual_context_marks();

    if (exchange(m_scroll_state_is_stale, false))
        m_document.invalidate_scroll_state();

    if (publication_arena)
        Layout::RustFFI::layout_arena_after_invalidation_journal_drain(publication_arena->handle());
    report_journal_pending_to_census(m_document, false);
}

}
