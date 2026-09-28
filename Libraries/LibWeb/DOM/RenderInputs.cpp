/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Debug.h>
#include <LibWeb/Animations/KeyframeEffect.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/RenderInputs.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxSlot.h>

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

// The arena's layout update marks, and the caches and natural sizes that go with them, are written through here,
// and nowhere else. What is written names a row of the document's arena by its slot.
static Layout::RustFFI::LayoutUpdateMarksHandle layout_update_marks(Document& document)
{
    return { .arena = Layout::document_layout_arena(document) };
}

void RenderInputs::set_needs_layout_update(Compositing::RustFFI::NodeSlotId slot, SetNeedsLayoutReason reason, Layout::LayoutUpdatePropagation propagation)
{
    if constexpr (UPDATE_LAYOUT_DEBUG) {
        // NOTE: We check some conditions here to avoid debug spam in documents that don't do layout.
        if (!Painting::BoxSlot::of(m_document, slot).has_flag(Layout::RustFFI::NodeFlag::NeedsLayoutUpdate)) {
            auto navigable = m_document.navigable();
            if (navigable && navigable->active_document() == GC::Ptr { &m_document })
                dbgln_if(UPDATE_LAYOUT_DEBUG, "NEED LAYOUT {}", to_string(reason));
        }
    }
    Layout::RustFFI::layout_arena_set_needs_layout_update(layout_update_marks(m_document), slot, propagation == Layout::LayoutUpdatePropagation::ThroughAncestors);
}

void RenderInputs::set_needs_own_geometry_update(Compositing::RustFFI::NodeSlotId slot)
{
    Layout::RustFFI::layout_arena_set_needs_own_geometry_update(layout_update_marks(m_document), slot);
}

void RenderInputs::reset_intrinsic_size_caches_of_self_and_ancestors(Compositing::RustFFI::NodeSlotId slot)
{
    Layout::RustFFI::layout_arena_bump_fragment_cache_epoch_of_self_and_ancestors(layout_update_marks(m_document), slot);
    Layout::RustFFI::layout_arena_reset_cached_intrinsic_sizes_of_self_and_ancestors(layout_update_marks(m_document), slot);
}

void RenderInputs::set_owned_image_natural_size(Compositing::RustFFI::NodeSlotId slot, Layout::RustFFI::FfiReplacedContentFacts const& facts)
{
    Layout::RustFFI::layout_arena_set_owned_image_natural_size(layout_update_marks(m_document), slot, facts);
}

Layout::RustFFI::FfiRemovedBoxDetach RenderInputs::detach_removed_box_in_place(Layout::RustFFI::FfiRemovedBoxPlace const& place)
{
    return Layout::RustFFI::rust_detach_removed_box_in_place(layout_update_marks(m_document), &place);
}

void RenderInputs::defer_child_list_insertion_layout_update(Compositing::RustFFI::NodeSlotId slot)
{
    Layout::RustFFI::layout_arena_defer_child_list_insertion_layout_update(layout_update_marks(m_document), slot);
}

void RenderInputs::invalidate_text_content(Compositing::RustFFI::NodeSlotId slot)
{
    Layout::RustFFI::layout_arena_invalidate_text_content(layout_update_marks(m_document), slot);
}

void RenderInputs::enroll_text_after_language_change(Compositing::RustFFI::NodeSlotId slot)
{
    Layout::RustFFI::layout_arena_enroll_text_after_language_change(layout_update_marks(m_document), slot);
}

// The arena's layout tree update marks are written through here, and nowhere else.
static Layout::RustFFI::LayoutTreeUpdateMarksHandle layout_tree_update_marks(Document& document)
{
    return { .arena = Layout::document_layout_arena(document) };
}

bool RenderInputs::merge_layout_tree_update_mark(CSS::StyleNodeID style_node, bool value, u8 reuse_reason)
{
    return Layout::RustFFI::layout_arena_merge_layout_tree_update_mark(layout_tree_update_marks(m_document), style_node.value(), value, reuse_reason);
}

bool RenderInputs::set_child_needs_layout_tree_update(CSS::StyleNodeID style_node, bool value)
{
    return Layout::RustFFI::layout_arena_set_child_needs_layout_tree_update(layout_tree_update_marks(m_document), style_node.value(), value);
}

// A document without an arena has no layout nodes, so its next build creates every box anyway.
void RenderInputs::set_needs_full_layout_tree_update(bool value)
{
    if (m_document.layout_arena_handle())
        Layout::RustFFI::layout_arena_set_needs_full_layout_tree_update(layout_tree_update_marks(m_document), value);
}

static void apply_style_node_change(Node& node, CSS::StyleNodeID old_style_node, CSS::StyleNodeID new_style_node)
{
    auto& document = node.document();
    if (auto* arena = document.layout_arena_handle()) {
        // The node's rows, and those of its pseudo-elements, take its new identity along with their
        // bindings. Both are still keyed by the old identity here, so this precedes retiring it.
        if (old_style_node != 0 && new_style_node != 0) {
            auto old_identity = NodeIdentity::of_style_node(old_style_node);
            if (auto box = Painting::BoxSlot::bound_to(document, old_identity))
                Layout::RustFFI::layout_arena_set_style_node_of_rows_sharing_dom_node_with(arena, box.slot(), new_style_node.value());
            if (auto* element = as_if<Element>(node)) {
                element->for_each_synthetic_pseudo_element([&](CSS::PseudoElement pseudo_element, SyntheticPseudoElement const&) {
                    if (auto box = Painting::BoxSlot::bound_to(document, old_identity, pseudo_element))
                        Layout::RustFFI::layout_arena_set_style_node_of_generated_subtree(arena, box.slot(), new_style_node.value());
                });
            }
            // What the node's pseudo-elements have scrolled to is keyed by the same pair, and
            // takes the node's new identity along with their bindings.
            Layout::RustFFI::layout_arena_move_pseudo_element_scroll_offsets(arena, old_style_node.value(), new_style_node.value());
            // So does what the element itself has scrolled to, which is keyed by the identity
            // alone; the element still holds the offset, so it is simply republished.
            if (auto* element = as_if<Element>(node))
                Layout::RustFFI::layout_arena_set_element_scroll_offset(arena, new_style_node.value(), element->scroll_offset({}));
        }
        // A retired identity may be reused, so it leaves every row carrying it, including rows of a
        // removed subtree that outlive the disconnection.
        if (old_style_node != 0)
            Layout::RustFFI::layout_arena_forget_style_node(arena, old_style_node.value());
    }
    // The arena names the node it tells about a binding change by identity, so a node changing
    // identity is one the arena cannot name. Its box-presence bits are re-committed here instead,
    // from the row its new identity reaches.
    auto box = Painting::BoxSlot::bound_to(node);
    node.set_box_presence(static_cast<bool>(box), box.has_committed_box());
}

void RenderInputs::note_style_node_changed(Node& node, CSS::StyleNodeID old_style_node)
{
    auto new_style_node = NodeIdentity::of(node).style_node();
    // A mark the new identity's previous holder left does not carry over to this node: it was keyed by the identity
    // alone, and may sit in this arena after that node moved to another document. The marks are the host's, so this
    // goes through beside a frame too, ahead of any mark made under the new identity.
    if (auto* arena = m_document.layout_arena_handle(); arena && new_style_node != 0)
        Layout::RustFFI::layout_arena_clear_layout_tree_update_marks(arena, new_style_node.value());
    // A layout pass or clock tick in flight has what the document publishes to the engine wait for it, so the node
    // goes on under its new identity beside it, and the arena takes the change in once the frame has been taken in.
    if (HTML::FrameScheduler::arena_changes_wait_for_frame(m_document)) {
        HTML::main_thread_event_loop().frame_scheduler().defer_arena_change(GC::create_function(node.heap(), [node = GC::Ref { node }, old_style_node, new_style_node] {
            apply_style_node_change(node, old_style_node, new_style_node);
        }));
        return;
    }
    apply_style_node_change(node, old_style_node, new_style_node);
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
