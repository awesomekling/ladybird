/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibCore/Environment.h>
#include <LibWeb/Compositor/NavigablePresenter.h>

namespace Web::Compositor {

static Optional<bool> s_render_presents_for_testing;

bool render_presents()
{
    static bool const enabled = [] {
        auto value = Core::Environment::get("LIBWEB_RENDER_PRESENTS"sv);
        return value.has_value() && *value == "1"sv;
    }();
    return s_render_presents_for_testing.value_or(enabled);
}

void set_render_presents_for_testing(Optional<bool> enabled)
{
    s_render_presents_for_testing = enabled;
}

static Compositing::DisplayListResourceSet resources_to_hand_compositor(Compositing::DisplayListResourceStorage const& resource_storage, Compositing::DisplayListResourceSet const& paint_command_cache_source_resources, Compositing::DisplayListResourceSet const& display_list_command_resources, Compositing::AccumulatedVisualContextTree const& visual_context_tree)
{
    auto resources = display_list_command_resources;
    // A recording downgraded to cache-read-only leaves the retained source and the cached ranges
    // into it live, so the resources they reference must survive the pruning that follows.
    resources.include(paint_command_cache_source_resources);
    resources.include(resource_storage.collect_referenced_resources(visual_context_tree));
    return resources;
}

void NavigablePresenter::did_hand_display_list_to_compositor(NonnullRefPtr<Compositing::DisplayList> display_list, HTML::PaintConfig paint_config, Compositing::DisplayListResourceSet command_resources, Compositing::DisplayListResourceSet resources)
{
    m_compositor_display_list_visual_context_tree_structural_epoch = display_list->compatible_visual_context_tree_structural_epoch();
    m_resource_storage.retain_only(resources);
    m_compositor_display_list = move(display_list);
    m_compositor_display_list_command_resources = move(command_resources);
    m_compositor_display_list_resources = move(resources);
    m_compositor_display_list_paint_config = paint_config;
}

void NavigablePresenter::did_hand_visual_context_tree_to_compositor(Compositing::DisplayListResourceSet resources)
{
    m_resource_storage.retain_only(resources);
    m_compositor_display_list_resources = move(resources);
}

void NavigablePresenter::forget_compositor_display_list()
{
    m_compositor_display_list_paint_config.clear();
    m_compositor_display_list = nullptr;
    m_compositor_display_list_resources = {};
    m_compositor_display_list_command_resources = {};
}

CompositorFrame NavigablePresenter::build_frame(PresentationInputs& inputs, PresentationSource& source, Optional<PublishedDisplayList> published)
{
    auto& keyboard_scroll_state = inputs.keyboard_scroll_state;
    bool const recorded = published.has_value();
    bool const compositor_display_list_is_unchanged = recorded && m_compositor_display_list == published->display_list.ptr();

    Optional<Compositing::AccumulatedVisualContextTree> visual_context_tree;
    Compositing::DisplayListResourceSet display_list_resources;
    Compositing::DisplayListResourceTransaction resource_transaction;
    if (recorded && !compositor_display_list_is_unchanged) {
        visual_context_tree = source.visual_context_tree(m_resource_storage);
        display_list_resources = resources_to_hand_compositor(m_resource_storage, inputs.paint_command_cache_source_resources, published->command_resources, *visual_context_tree);
        resource_transaction = m_resource_storage.create_transaction(m_compositor_display_list_resources, display_list_resources);
    }

    auto visual_context_tree_needs_compositor_update = source.visual_context_tree_needs_compositor_update();

    auto scroll_state_snapshot = source.scroll_state_snapshot();

    // Keyboard eligibility belongs to this publication, not to the cached paint commands. Refresh it even if
    // recording was skipped or returned the same display list, and send it with the corresponding scroll state.
    auto& published_display_list = recorded ? *published->display_list : *m_compositor_display_list;
    if (keyboard_scroll_state.visual_context_tree_structural_epoch == keyboard_scroll_epoch_placeholder)
        keyboard_scroll_state.visual_context_tree_structural_epoch = published_display_list.compatible_visual_context_tree_structural_epoch();
    auto async_scrolling_metadata = published_display_list.async_scrolling_metadata().value_or({});
    async_scrolling_metadata.keyboard_scroll_state = keyboard_scroll_state;
    published_display_list.set_async_scrolling_metadata(move(async_scrolling_metadata));

    CompositorFrame frame;
    frame.context_id = inputs.context_id;
    if (recorded && !compositor_display_list_is_unchanged) {
        frame.display_list_update = CompositorFrame::DisplayListUpdate {
            .display_list = published->display_list,
            .visual_context_tree = visual_context_tree.release_value(),
            .resource_transaction = move(resource_transaction),
            .scroll_state_snapshot = move(scroll_state_snapshot),
        };
        source.did_update_visual_context_tree_in_compositor();
        did_hand_display_list_to_compositor(published->display_list, inputs.paint_config, move(published->command_resources), move(display_list_resources));
    } else {
        if (compositor_display_list_is_unchanged) {
            m_compositor_display_list_paint_config = inputs.paint_config;
            // NB: A tree update below retains what the updated tree references, which can be more than the
            //     compositor holds yet.
            if (!visual_context_tree_needs_compositor_update && m_resource_storage.has_resources_added_since_last_retain())
                m_resource_storage.retain_only(m_compositor_display_list_resources);
        }
        if (visual_context_tree_needs_compositor_update) {
            auto updated_visual_context_tree = source.visual_context_tree(m_resource_storage);
            VERIFY(updated_visual_context_tree.structural_epoch() == m_compositor_display_list_visual_context_tree_structural_epoch);
            auto updated_display_list_resources = resources_to_hand_compositor(m_resource_storage, inputs.paint_command_cache_source_resources, m_compositor_display_list_command_resources, updated_visual_context_tree);
            auto updated_resource_transaction = m_resource_storage.create_transaction(m_compositor_display_list_resources, updated_display_list_resources);
            frame.visual_context_tree_update = CompositorFrame::VisualContextTreeUpdate {
                .visual_context_tree = move(updated_visual_context_tree),
                .resource_transaction = move(updated_resource_transaction),
            };
            source.did_update_visual_context_tree_in_compositor();
            did_hand_visual_context_tree_to_compositor(move(updated_display_list_resources));
        }
        frame.scroll_state_update = CompositorFrame::ScrollStateUpdate {
            .scroll_state_snapshot = move(scroll_state_snapshot),
            .keyboard_scroll_state = move(keyboard_scroll_state),
        };
    }
    frame.present_viewport_rect = inputs.present_viewport_rect;
    return frame;
}

}
