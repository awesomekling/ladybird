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
        m_entries.append(Entry {
            .identity = identity,
            .layer_image_paint_facts_update = {},
            .replaced_image_paint_facts_update = {},
            .video_paint_facts_update = {},
        });
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

void InvalidationJournal::note_canvas_paint_facts(NodeIdentity identity, bool has_content, i32 content_width, i32 content_height, u64 canvas_id, u64 content_generation)
{
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
    auto& entry = entry_for(identity);
    switch (family) {
    case PaintFactsFamily::LayerImage:
        entry.layer_image_paint_facts_update = move(update);
        break;
    case PaintFactsFamily::ReplacedImage:
        entry.replaced_image_paint_facts_update = move(update);
        break;
    case PaintFactsFamily::Video:
        entry.video_paint_facts_update = move(update);
        break;
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

    auto* publication_arena = m_document.layout_node_arena_if_created();
    if (publication_arena)
        Layout::RustFFI::layout_arena_before_invalidation_journal_drain(publication_arena->handle());

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

            if (!entry.needs_layout_update && !entry.needs_repaint && !entry.has_dom_paint_facts && !entry.has_canvas_paint_facts && !entry.has_form_control_paint_facts && !entry.layer_image_paint_facts_update && !entry.replaced_image_paint_facts_update && !entry.video_paint_facts_update)
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
                    Painting::invalidate_paint_cache(*layout_node);
            }
            if (entry.has_form_control_paint_facts) {
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
            if (entry.needs_repaint) {
                if (auto* text_node = as_if<Layout::TextNode>(*layout_node))
                    text_node->set_needs_repaint(entry.invalidate_display_list);
                else if (Painting::has_committed_box(*layout_node))
                    Painting::set_needs_repaint(*layout_node, entry.invalidate_display_list);
            }
        }
    }

    if (publication_arena)
        Layout::RustFFI::layout_arena_after_invalidation_journal_drain(publication_arena->handle());
}

}
