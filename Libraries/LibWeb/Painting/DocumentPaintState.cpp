/*
 * Copyright (c) 2026, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/EventTarget.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/DOM/Range.h>
#include <LibWeb/DOM/Text.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/Window.h>
#include <LibWeb/Layout/TextNode.h>
#include <LibWeb/Layout/Viewport.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/PaintingRustBridge.h>
#include <LibWeb/Painting/SvgPaintResources.h>

namespace Web::Painting {

DocumentPaintState::DocumentPaintState(Layout::NodeArena& layout_node_arena)
    : m_layout_node_arena(layout_node_arena)
{
}

void DocumentPaintState::ensure_visual_context_tree(DOM::Document const& document) const
{
    const_cast<DOM::Document&>(document).update_paint_and_hit_testing_properties_if_needed();
}

bool DocumentPaintState::has_visual_context_tree() const
{
    return Layout::RustFFI::layout_arena_has_visual_context_tree(m_layout_node_arena->handle());
}

Compositing::AccumulatedVisualContextTree DocumentPaintState::visual_context_tree_without_update(DOM::Document const& document) const
{
    return Compositing::AccumulatedVisualContextTree::adopt_rust_handle(retain_rust_main_visual_context_tree(document));
}

Compositing::AccumulatedVisualContextTree DocumentPaintState::visual_context_tree(DOM::Document const& document) const
{
    ensure_visual_context_tree(document);
    return visual_context_tree_without_update(document);
}

u64 DocumentPaintState::visual_context_tree_structural_epoch(DOM::Document const& document) const
{
    ensure_visual_context_tree(document);
    return Layout::RustFFI::layout_arena_visual_context_tree_structural_epoch(m_layout_node_arena->handle());
}

BlockingWheelEventRegionState DocumentPaintState::collect_root_blocking_wheel_event_regions(DOM::Document& document)
{
    GC::Ptr<DOM::EventTarget> roots[] = {
        document.navigable() ? document.navigable()->active_window() : nullptr,
        &document,
        document.document_element(),
        document.body(),
    };
    for (auto target : roots) {
        if (target && target->has_blocking_wheel_event_listener()) {
            return {
                .has_blocking_wheel_event_listeners = true,
                .has_blocking_wheel_event_region_covering_viewport = true,
            };
        }
    }
    return {};
}

void DocumentPaintState::viewport_row_was_reset()
{
    m_scroll_state_snapshot = {};
    m_boxes_with_auto_content_visibility.clear();
    m_visual_context_tree_needs_compositor_update = false;
}

void DocumentPaintState::invalidate_scroll_state(DOM::Document& document)
{
    rust_invalidate_scroll_state(document);
}

void DocumentPaintState::update_accumulated_visual_contexts(DOM::Document& document)
{
    bool svg_paint_resources_changed = sync_svg_paint_resources(document);
    auto result = rust_update_accumulated_visual_contexts(document);
    if (result.performed_full_build)
        ++m_accumulated_visual_context_tree_build_count;
    else
        ++m_accumulated_visual_context_tree_incremental_update_count;
    if (result.requires_display_list_recording || svg_paint_resources_changed)
        document.set_needs_to_record_display_list();
    m_visual_context_tree_needs_compositor_update = true;
}

void DocumentPaintState::update_visual_viewport_accumulated_visual_context(DOM::Document& document)
{
    if (!has_visual_context_tree()) {
        update_accumulated_visual_contexts(document);
        return;
    }
    rust_update_visual_viewport_transform(document);
    m_visual_context_tree_needs_compositor_update = true;
}

void DocumentPaintState::begin_compositor_animation_update(DOM::Document& document)
{
    ensure_visual_context_tree(document);
    Layout::RustFFI::layout_arena_begin_compositor_animation_update(m_layout_node_arena->handle());
}

void DocumentPaintState::publish_compositor_animations(DOM::Document& document, PublishPendingCompositorAnimations publish_pending)
{
    ensure_visual_context_tree(document);
    auto outcome = Layout::RustFFI::layout_arena_publish_compositor_animations(m_layout_node_arena->handle(), publish_pending == PublishPendingCompositorAnimations::Yes);
    if (!outcome.published)
        return;
    m_visual_context_tree_needs_compositor_update = true;
    if (outcome.parameters_changed)
        ++document.style_invalidation_counters().compositor_visual_animation_updates;
    if (outcome.timing_anchors_changed)
        ++document.style_invalidation_counters().compositor_visual_animation_timing_anchor_updates;
}

void DocumentPaintState::republish_visual_animations(DOM::Document& document)
{
    if (!Layout::RustFFI::layout_arena_visual_context_tree_has_visual_animations(m_layout_node_arena->handle()))
        return;
    m_visual_context_tree_needs_compositor_update = true;
    ++document.style_invalidation_counters().compositor_visual_animation_updates;
}

void DocumentPaintState::append_paint_command_cache_source_resources(Compositing::DisplayListResourceSet& retained_resources) const
{
    retained_resources.include(m_paint_command_cache_source_referenced_resources);
}

void DocumentPaintState::invalidate_all_cached_paint(DOM::Document& document)
{
    Layout::RustFFI::layout_arena_invalidate_all_paint_caches(m_layout_node_arena->handle());
    Painting::set_needs_repaint(*document.unsafe_layout_node());
}

void DocumentPaintState::refresh_scroll_state(DOM::Document& document)
{
    if (rust_refresh_scroll_state(document, m_scroll_state_snapshot))
        return;

    // LIBWEB_VERIFY_SCROLL_STATE: a skipped refresh must have been skippable. Every producer of a
    // scroll offset invalidates the state, so re-deriving the snapshot from scratch has to
    // reproduce the one kept.
    static bool const verify_scroll_state = getenv("LIBWEB_VERIFY_SCROLL_STATE") != nullptr;
    if (!verify_scroll_state)
        return;
    Compositing::ScrollStateSnapshot rederived_snapshot;
    rust_refresh_scroll_state(document, rederived_snapshot, ForceScrollStateRefresh::Yes);
    VERIFY(rederived_snapshot.device_offsets() == m_scroll_state_snapshot.device_offsets());
}

void DocumentPaintState::reset_selection_states(DOM::Document& document)
{
    Layout::RustFFI::layout_arena_selection_clear(m_layout_node_arena->handle(), viewport_row_slot(document));
}

void DocumentPaintState::recompute_selection_states(DOM::Range& range)
{
    Vector<Layout::RustFFI::FfiSelectionSnapshotNode> nodes;
    auto snapshot = read_selection_snapshot(range, nodes);
    Layout::RustFFI::layout_arena_selection_apply_snapshot(m_layout_node_arena->handle(), &snapshot);
}

// The nodes are the ones the selection states are stamped for, and what excludes them from selection is read here;
// whether a node has a box to stamp is left to the rows it is bound to, which a layout commit may since have rebuilt.
Layout::RustFFI::FfiSelectionSnapshot read_selection_snapshot(DOM::Range& range, Vector<Layout::RustFFI::FfiSelectionSnapshotNode>& nodes)
{
    auto add_node = [&](DOM::Node& node, Layout::RustFFI::FfiSelectionSnapshotRole role) {
        // Only a node with an identity can have a box.
        auto identity = DOM::NodeIdentity::of(node);
        if (!identity)
            return;
        nodes.append({
            .style_node = identity.style_node().value(),
            .is_document = node.is_document(),
            .is_text = is<DOM::Text>(node),
            .is_inert = node.is_inert(),
            .user_select_is_none = node.user_select_used_value() == CSS::UserSelect::None,
            .role = role,
        });
    };

    auto start_container = range.start_container();
    auto end_container = range.end_container();
    add_node(*start_container, Layout::RustFFI::FfiSelectionSnapshotRole::StartContainer);

    // The nodes between the containers are not read when the start container settles the selection by itself: an
    // empty selection, or one inside a text node that nothing excludes from selection, covers nothing between them.
    auto is_settled_by_start_container = start_container == end_container
        && (range.start_offset() == range.end_offset()
            || (is<DOM::Text>(*start_container) && !start_container->is_inert() && start_container->user_select_used_value() != CSS::UserSelect::None));
    if (!is_settled_by_start_container) {
        auto* start_at = start_container->child_at_index(range.start_offset());
        // If the start container has no child at that index, we need to start on the node right after the start container.
        if (!start_at) {
            if (auto* last_child = start_container->last_child()) {
                start_at = last_child->next_in_pre_order();
            } else {
                start_at = start_container->next_in_pre_order();
            }
        }

        DOM::Node* stop_at = end_container->child_at_index(range.end_offset());
        // Only stop at the end container if it has no children that may need to be included.
        for (auto* node = start_at; node && (node != stop_at && !(node == end_container.ptr() && !end_container->has_children())); node = node->next_in_pre_order(end_container.ptr()))
            add_node(*node, Layout::RustFFI::FfiSelectionSnapshotRole::Covered);
    }

    add_node(*end_container, Layout::RustFFI::FfiSelectionSnapshotRole::EndContainer);

    return {
        .nodes = nodes.data(),
        .node_count = nodes.size(),
        .start_offset = range.start_offset(),
        .end_offset = range.end_offset(),
        .starts_and_ends_in_one_container = start_container == end_container,
    };
}

}
