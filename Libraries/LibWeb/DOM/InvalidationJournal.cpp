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
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/Scrollbar.h>
#include <LibWeb/SVG/SVGElement.h>
#include <LibWeb/Selection/Selection.h>

namespace Web::DOM {

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
    auto index = m_entry_index_by_identity.ensure(identity, [&] {
        m_entries.append(Entry { .identity = identity, .rare = {} });
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
    auto& rare = entry_for(identity).ensure_rare();
    rare.has_canvas_paint_facts = true;
    rare.canvas_has_content = has_content;
    rare.canvas_content_width = content_width;
    rare.canvas_content_height = content_height;
    rare.canvas_id = canvas_id;
    rare.canvas_content_generation = content_generation;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_form_control_paint_facts(NodeIdentity identity, bool enabled, bool checked, bool indeterminate, bool being_activated)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    auto& rare = entry_for(identity).ensure_rare();
    rare.has_form_control_paint_facts = true;
    rare.form_control_enabled = enabled;
    rare.form_control_checked = checked;
    rare.form_control_indeterminate = indeterminate;
    rare.form_control_being_activated = being_activated;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_paint_facts(NodeIdentity identity, PaintFactsFamily family, PaintFactsUpdate&& update)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    auto& entry = entry_for(identity);
    auto& rare = entry.ensure_rare();
    switch (family) {
    case PaintFactsFamily::LayerImage:
        rare.layer_image_paint_facts_update = move(update);
        entry.clears_layer_image_paint_facts = false;
        break;
    case PaintFactsFamily::ReplacedImage:
        rare.replaced_image_paint_facts_update = move(update);
        // Applying changed replaced-image facts requests a repaint; see note_dom_paint_facts().
        m_document.request_frame_for_journalled_repaint({});
        break;
    case PaintFactsFamily::Video:
        rare.video_paint_facts_update = move(update);
        // Applying changed video facts requests a repaint; see note_dom_paint_facts().
        m_document.request_frame_for_journalled_repaint({});
        break;
    case PaintFactsFamily::NavigableContainer:
        rare.navigable_container_paint_facts_update = move(update);
        // Applying changed navigable container facts invalidates the paint cache, which only a
        // rendering update repaints; see note_dom_paint_facts().
        m_document.request_frame_for_journalled_repaint({});
        break;
    }
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_layer_image_paint_facts_cleared(NodeIdentity identity)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    auto& entry = entry_for(identity);
    entry.clears_layer_image_paint_facts = true;
    if (entry.rare)
        entry.rare->layer_image_paint_facts_update = nullptr;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_unanchored_paint_facts(Compositing::RustFFI::NodeSlotId slot, PaintFactsUpdate&& update)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::PaintFacts);
    m_unanchored_paint_facts.append({ slot, move(update) });
    m_document.request_frame_for_journalled_repaint({});
    drain_if_the_render_side_is_reading();
}

// These go through in the order they were noted, since no entry merges them. What they mark for
// repaint lands in the entries, so they go through ahead of them.
void InvalidationJournal::publish_unanchored_paint_facts()
{
    auto updates = move(m_unanchored_paint_facts);
    if (!m_document.layout_arena_handle())
        return;
    for (auto const& [slot, update] : updates) {
        if (auto box = Painting::BoxSlot::of(m_document, slot))
            update(box);
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
    auto box = Painting::BoxSlot::bound_to(node);
    if (!box)
        return;
    auto is_editing_host = node.is_editing_host();
    if (box.has_flag(Layout::RustFFI::NodeFlag::IsEditingHost) != is_editing_host) {
        Layout::RustFFI::layout_arena_set_node_flag(box.arena(), box.slot(), Layout::RustFFI::HostNodeFlag::IsEditingHost, is_editing_host);
        node.set_needs_layout_update(SetNeedsLayoutReason::EditableStateChange);
    }
    if (box.is_text() && Layout::update_empty_line_box_fragment_flag_of_box(box))
        node.set_needs_layout_update(SetNeedsLayoutReason::EditableStateChange);
}

void InvalidationJournal::note_selection_states()
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::Selection);
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
    auto& offsets = entry_for(generator).ensure_rare().pseudo_element_scroll_offsets;
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
    m_visual_context_box_dirty_marks.append({ slot, kind });
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_visual_context_full_rebuild(Layout::RustFFI::FfiVisualContextGlobalRebuildReason reason)
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::VisualContext);
    if (!m_visual_context_full_rebuild_reasons.contains_slow(reason))
        m_visual_context_full_rebuild_reasons.append(reason);
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_svg_paint_resources_changed()
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::VisualContext);
    m_svg_paint_resources_changed = true;
    drain_if_the_render_side_is_reading();
}

void InvalidationJournal::note_visual_viewport_transform()
{
    count_entry_during_flight(HTML::EventLoop::JournalEntryKind::VisualContext);
    m_visual_viewport_transform_is_stale = true;
    drain_if_the_render_side_is_reading();
}

// A mark on a row that was freed since lands nowhere, and one on a row reused since marks a box
// the next update revisits for nothing.
void InvalidationJournal::publish_visual_context_marks()
{
    auto* arena = m_document.layout_arena_handle();
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
        Layout::RustFFI::layout_arena_visual_context_request_full_rebuild(arena, reason);
    for (auto const& mark : box_dirty_marks)
        Layout::RustFFI::layout_arena_visual_context_note_box_dirty(arena, mark.slot, mark.kind);
    if (exchange(m_svg_paint_resources_changed, false) && Layout::RustFFI::layout_arena_note_svg_paint_resources_changed(arena))
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
    if (auto box = Painting::BoxSlot::bound_to(element))
        Layout::synchronize_table_spans_of_box(box);
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
    auto text_box = Painting::BoxSlot::bound_to(text);
    if (text_box && Layout::RustFFI::layout_arena_text_has_source_range(text_box.arena(), text_box.slot())) {
        // First-letter source ranges are determined while building the layout tree.
        if (auto* parent = text.parent())
            parent->set_needs_layout_tree_update(true, SetNeedsLayoutTreeUpdateReason::CharacterDataReplaceData);
    } else if (text_box) {
        // NB: Since the text node's data has changed, we need to invalidate the text for rendering.
        //     This ensures that the new text is reflected in layout, even if we don't end up doing a full layout
        //     tree rebuild.
        auto& inputs = text.document().render_inputs_for_write();
        inputs.invalidate_text_content(text_box.slot());

        // We also need to relayout.
        inputs.set_needs_layout_update(text_box.slot(), SetNeedsLayoutReason::CharacterDataReplaceData);

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
void InvalidationJournal::publish_scroll_offsets(Node& node, Entry const& entry)
{
    if (auto* document = as_if<Document>(node)) {
        if (auto box = Painting::BoxSlot::viewport_of(*document))
            Layout::publish_scroll_offset_of_box(box);
        return;
    }
    auto* element = as_if<Element>(node);
    if (!element)
        return;
    for (auto const& [type, offset] : entry.rare ? entry.rare->pseudo_element_scroll_offsets.span() : ReadonlySpan<PseudoElementScrollOffset> {}) {
        auto pseudo_element = element->get_synthetic_pseudo_element(type);
        if (!pseudo_element.has_value())
            continue;
        pseudo_element->set_scroll_offset(offset);
        if (auto box = Painting::BoxSlot::of_pseudo_element(*element, type))
            Layout::publish_scroll_offset_of_box(box);
    }
    if (entry.needs_scroll_offset_publish) {
        Layout::publish_element_scroll_offset(*element);
        if (auto box = Painting::BoxSlot::bound_to(*element))
            Layout::publish_scroll_offset_of_box(box);
    }
}

// A mark made from inside a layout frame is one the frame is about to read, so it goes through at
// once. Between frames, and beside a frame in flight, nothing reads what these marks change before
// the next drain, and the next frame starts with one.
void InvalidationJournal::drain_if_the_render_side_is_reading()
{
    if (!m_holds_next_generation && !m_defers_write_through && m_document.is_running_update_layout())
        drain();
}

InvalidationJournal::WriteThroughDeferral::WriteThroughDeferral(InvalidationJournal& journal)
    : m_journal(journal)
    , m_was_deferring(exchange(journal.m_defers_write_through, true))
{
}

InvalidationJournal::WriteThroughDeferral::~WriteThroughDeferral()
{
    m_journal.m_defers_write_through = m_was_deferring;
    if (!m_was_deferring && !m_journal.is_empty())
        m_journal.drain_if_the_render_side_is_reading();
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
    if (auto* arena = m_document.layout_arena_handle()) {
        auto location = SourceLocation::current();
        Layout::RustFFI::layout_arena_join_frame_owning_arena(arena, reinterpret_cast<u8 const*>(location.filename().characters_without_null_termination()), location.filename().length(), location.line_number());
        // Taking the frame in hands the document what was marked beside it, in the journal it drains from now on, and
        // this journal holds the marks made beside the next frame. What the drain was asked for is in the other one:
        // an up-to-date answer read after this drain would hide those marks from the read.
        if (m_holds_next_generation) {
            m_document.drain_invalidation_journal();
            return;
        }
    }
    if (is_empty() || m_draining)
        return;
    TemporaryChange draining { m_draining, true };

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
            auto const* rare = entry.rare.ptr();
            if (node && (entry.needs_scroll_offset_publish || (rare && !rare->pseudo_element_scroll_offsets.is_empty())))
                publish_scroll_offsets(*node, entry);

            if (!entry.needs_layout_update && !entry.needs_repaint && !entry.needs_subtree_repaint && !entry.has_dom_paint_facts && !rare && !entry.clears_layer_image_paint_facts && !entry.invalidate_paint_and_hit_test_cache && !entry.invalidate_propagated_text_decoration_caches)
                continue;
            // A node whose box went away between the mark and here has nothing left to mark.
            auto box = Painting::BoxSlot::bound_to(m_document, entry.identity);
            if (!box)
                continue;
            if (entry.needs_layout_update)
                m_document.render_inputs_for_write().set_needs_layout_update(box.slot(), entry.layout_reason, entry.layout_propagation);
            if (entry.has_dom_paint_facts) {
                // The owner takes the facts in with its next unit, so whether they changed is not known here: the
                // node repaints either way.
                Layout::RustFFI::layout_arena_set_node_dom_paint_facts(box.arena(), box.slot(), entry.dom_paint_facts);
                if (node)
                    node->set_needs_repaint();
            }
            if (rare && rare->has_canvas_paint_facts) {
                Layout::RustFFI::FfiCanvasPaintFacts facts {
                    .has_content = rare->canvas_has_content,
                    .content_width = rare->canvas_content_width,
                    .content_height = rare->canvas_content_height,
                    .canvas_id = rare->canvas_id,
                    .content_generation = rare->canvas_content_generation,
                };
                // Where the facts changed, the render owner damages the box's paint and hit-test caches.
                Layout::RustFFI::layout_arena_set_canvas_paint_facts(box.arena(), box.slot(), facts);
            }
            if (rare && rare->has_form_control_paint_facts && (box.kind() == Layout::RustFFI::NodeKind::CheckBox || box.kind() == Layout::RustFFI::NodeKind::RadioButton)) {
                Layout::RustFFI::FfiFormControlPaintFacts facts {
                    .enabled = rare->form_control_enabled,
                    .checked = rare->form_control_checked,
                    .indeterminate = rare->form_control_indeterminate,
                    .being_activated = rare->form_control_being_activated,
                };
                // The render owner compares the facts; the control repaints as if they changed.
                Layout::RustFFI::layout_arena_set_form_control_paint_facts(box.arena(), box.slot(), facts);
                Painting::set_needs_repaint(box, InvalidateDisplayList::PaintCommands);
            }
            if (entry.clears_layer_image_paint_facts)
                Layout::RustFFI::layout_arena_set_layer_image_paint_facts(box.arena(), box.slot(), nullptr, 0);
            if (rare && rare->layer_image_paint_facts_update)
                rare->layer_image_paint_facts_update(box);
            if (rare && rare->replaced_image_paint_facts_update)
                rare->replaced_image_paint_facts_update(box);
            if (rare && rare->video_paint_facts_update)
                rare->video_paint_facts_update(box);
            if (rare && rare->navigable_container_paint_facts_update)
                rare->navigable_container_paint_facts_update(box);
            if (entry.invalidate_paint_and_hit_test_cache)
                Painting::apply_paint_cache_invalidation(box, Painting::PaintCacheInvalidation::PaintAndHitTest);
            if (entry.invalidate_propagated_text_decoration_caches)
                Painting::apply_paint_cache_invalidation(box, Painting::PaintCacheInvalidation::PropagatedTextDecorations);
            if (entry.needs_subtree_repaint)
                Painting::apply_subtree_repaint_damage(box);
            if (entry.needs_repaint) {
                if (box.is_text())
                    Painting::apply_text_repaint_damage(box, entry.invalidate_display_list);
                else if (Painting::has_committed_box(box))
                    Painting::apply_repaint_damage(box, entry.invalidate_display_list);
            }
        }
        // The next generation reuses the storage, unless what the drain wrote through noted more.
        if (m_entries.is_empty()) {
            entries.clear_with_capacity();
            m_entries = move(entries);
        }
    }

    publish_visual_context_marks();

    if (exchange(m_scroll_state_is_stale, false))
        m_document.invalidate_scroll_state();
}

}
