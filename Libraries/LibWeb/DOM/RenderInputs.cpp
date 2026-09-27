/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/Animations/KeyframeEffect.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/RenderInputs.h>
#include <LibWeb/Layout/NodeArena.h>

namespace Web::DOM {

RenderInputs::RenderInputs(Badge<RenderInputsEntrance>, Document& document)
    : m_document(document)
{
}

void RenderInputs::visit_edges(GC::Cell::Visitor& visitor)
{
    visitor.visit(m_elements_with_pending_top_layer_membership_change);
}

CSS::StyleEngine& RenderInputs::style_engine()
{
    return m_document.style_computer().m_style_engine.engine_for_write();
}

void RenderInputs::note_needs_layout_update(NodeIdentity identity, SetNeedsLayoutReason reason, Layout::LayoutUpdatePropagation propagation)
{
    m_document.invalidation_journal().note_needs_layout_update(identity, reason, propagation);
}

void RenderInputs::note_needs_layout_tree_update(NodeIdentity identity, SetNeedsLayoutTreeUpdateReason reason)
{
    m_document.invalidation_journal().note_needs_layout_tree_update(identity, reason);
}

void RenderInputs::note_editability_stamps(NodeIdentity identity)
{
    m_document.invalidation_journal().note_editability_stamps(identity);
}

void RenderInputs::note_is_in_focused_text_control(NodeIdentity identity)
{
    m_document.invalidation_journal().note_is_in_focused_text_control(identity);
}

void RenderInputs::note_scroll_offset(NodeIdentity identity, bool offset_changed)
{
    m_document.invalidation_journal().note_scroll_offset(identity, offset_changed);
}

void RenderInputs::note_pseudo_element_scroll_offset(NodeIdentity generator, CSS::PseudoElement type, CSSPixelPoint offset, bool offset_changed)
{
    m_document.invalidation_journal().note_pseudo_element_scroll_offset(generator, type, offset, offset_changed);
}

void RenderInputs::note_text_data(Text& text, bool whitespace_state_changed)
{
    m_document.invalidation_journal().note_text_data(text, whitespace_state_changed);
}

void RenderInputs::note_svg_attribute_facts(NodeIdentity identity)
{
    m_document.invalidation_journal().note_svg_attribute_facts(identity);
}

void RenderInputs::note_table_spans(NodeIdentity identity)
{
    m_document.invalidation_journal().note_table_spans(identity);
}

void RenderInputs::note_visual_context_box_dirty(Compositing::RustFFI::NodeSlotId slot, Layout::RustFFI::FfiVisualContextBoxDirtyKind kind)
{
    m_document.invalidation_journal().note_visual_context_box_dirty(slot, kind);
}

void RenderInputs::note_visual_context_full_rebuild(Layout::RustFFI::FfiVisualContextGlobalRebuildReason reason)
{
    m_document.invalidation_journal().note_visual_context_full_rebuild(reason);
}

void RenderInputs::note_svg_paint_resources_changed()
{
    m_document.invalidation_journal().note_svg_paint_resources_changed();
}

void RenderInputs::note_visual_viewport_transform()
{
    m_document.invalidation_journal().note_visual_viewport_transform();
}

// The arena's layout tree update marks are written through here, and nowhere else.
static Layout::RustFFI::LayoutTreeUpdateMarksHandle layout_tree_update_marks(Layout::NodeArena& arena)
{
    return { .arena = arena.handle() };
}

bool RenderInputs::merge_layout_tree_update_mark(CSS::StyleNodeID style_node, bool value, u8 reuse_reason)
{
    return Layout::RustFFI::layout_arena_merge_layout_tree_update_mark(layout_tree_update_marks(m_document.layout_node_arena()), style_node.value(), value, reuse_reason);
}

bool RenderInputs::set_child_needs_layout_tree_update(CSS::StyleNodeID style_node, bool value)
{
    return Layout::RustFFI::layout_arena_set_child_needs_layout_tree_update(layout_tree_update_marks(m_document.layout_node_arena()), style_node.value(), value);
}

// A document without an arena has no layout nodes, so its next build creates every box anyway.
void RenderInputs::set_needs_full_layout_tree_update(bool value)
{
    if (auto* arena = m_document.layout_node_arena_if_created())
        Layout::RustFFI::layout_arena_set_needs_full_layout_tree_update(layout_tree_update_marks(*arena), value);
}

void RenderInputs::mark_style_attribute_dirty(Element& element)
{
    m_elements_with_dirty_style_attributes.set(element);
}

GC::WeakHashSet<Element> RenderInputs::take_elements_with_dirty_style_attributes()
{
    return move(m_elements_with_dirty_style_attributes);
}

void RenderInputs::note_top_layer_membership_change(GC::Ref<Element> element)
{
    m_elements_with_pending_top_layer_membership_change.append(element);
}

Vector<GC::Ref<Element>> RenderInputs::take_top_layer_changes()
{
    m_top_layer_needs_layout_zone_rebuild = false;
    return move(m_elements_with_pending_top_layer_membership_change);
}

void RenderInputsEntrance::drop_query_snapshot()
{
    m_query_snapshot = nullptr;
}

}
