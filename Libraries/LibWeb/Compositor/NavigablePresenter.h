/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Atomic.h>
#include <AK/AtomicRefCounted.h>
#include <AK/NonnullRefPtr.h>
#include <AK/NumericLimits.h>
#include <AK/Optional.h>
#include <AK/RefPtr.h>
#include <LibCompositing/DisplayList/DisplayList.h>
#include <LibCompositing/DisplayList/DisplayListResourceStorage.h>
#include <LibCompositing/Scrolling/ScrollState.h>
#include <LibWeb/Compositor/CompositorFrame.h>
#include <LibWeb/Export.h>
#include <LibWeb/HTML/PaintConfig.h>

namespace Web::Painting {

struct PendingDisplayListRecording;

}

namespace Web::Compositor {

class NavigablePresenter;

// Whether the frame a navigable presents is sealed when the rendering update begins it and presented from the Rendering
// thread, rather than read from its document where the frame is finished (LIBWEB_RENDER_PRESENTS=0).
WEB_API bool render_presents();
// Test only: overrides LIBWEB_RENDER_PRESENTS for the frames begun from now on, or stops overriding it.
WEB_API void set_render_presents_for_testing(Optional<bool>);

// Where a frame's keyboard scroll state goes with the epoch of a display list not recorded yet.
inline constexpr u64 keyboard_scroll_epoch_placeholder = NumericLimits<u64>::max();

// What a published display list's async scrolling metadata is stamped with.
struct AsyncScrollingStamp {
    u64 wheel_event_listener_state_generation { 0 };
    double device_pixels_per_css_pixel { 1.0 };
};

// What a presentation reads of the document it presents: live, where the frame is finished on the main thread, or
// sealed where it was begun, for a presentation that does not reach the document.
class WEB_API PresentationSource {
public:
    virtual ~PresentationSource() = default;

    // The visual context tree a display list the recording publishes anew is wrapped with.
    virtual Compositing::AccumulatedVisualContextTree published_display_list_visual_context_tree() = 0;
    // What a published display list is stamped with, or nothing if the document has no navigable anymore.
    virtual Optional<AsyncScrollingStamp> async_scrolling_stamp() = 0;
    // The recording has been published.
    virtual void did_publish_recording() = 0;
    // The visual context tree the compositor gets with a new display list or as an update. What it references is in
    // `resource_storage` once this returns.
    virtual Compositing::AccumulatedVisualContextTree visual_context_tree(Compositing::DisplayListResourceStorage& resource_storage) = 0;
    virtual bool visual_context_tree_needs_compositor_update() = 0;
    virtual void did_update_visual_context_tree_in_compositor() = 0;
    virtual Compositing::ScrollStateSnapshot scroll_state_snapshot() = 0;
};

// A presentation source sealed where the frame was begun: what the frame shows is the document as the recording was
// cut from it.
class WEB_API SealedPresentationSource final : public PresentationSource {
public:
    SealedPresentationSource(Optional<Compositing::AccumulatedVisualContextTree> visual_context_tree, Optional<AsyncScrollingStamp> async_scrolling_stamp, bool visual_context_tree_needs_compositor_update, Compositing::ScrollStateSnapshot scroll_state_snapshot)
        : m_visual_context_tree(move(visual_context_tree))
        , m_async_scrolling_stamp(async_scrolling_stamp)
        , m_visual_context_tree_needs_compositor_update(visual_context_tree_needs_compositor_update)
        , m_scroll_state_snapshot(move(scroll_state_snapshot))
    {
    }

    virtual Compositing::AccumulatedVisualContextTree published_display_list_visual_context_tree() override { return *m_visual_context_tree; }
    virtual Optional<AsyncScrollingStamp> async_scrolling_stamp() override { return m_async_scrolling_stamp; }
    virtual void did_publish_recording() override { }
    virtual Compositing::AccumulatedVisualContextTree visual_context_tree(Compositing::DisplayListResourceStorage&) override { return *m_visual_context_tree; }
    virtual bool visual_context_tree_needs_compositor_update() override { return m_visual_context_tree_needs_compositor_update; }
    virtual void did_update_visual_context_tree_in_compositor() override { m_did_update_visual_context_tree_in_compositor = true; }
    virtual Compositing::ScrollStateSnapshot scroll_state_snapshot() override { return m_scroll_state_snapshot; }

    // Whether the frame took the tree to the compositor, which the seal marked as done already.
    bool did_update_visual_context_tree_in_compositor() const { return m_did_update_visual_context_tree_in_compositor; }
    // A render clock tick presents with the tree its layout left, and the scroll offsets of its nodes.
    void replace_visual_context_tree(Compositing::AccumulatedVisualContextTree visual_context_tree, bool needs_compositor_update)
    {
        m_visual_context_tree = move(visual_context_tree);
        m_visual_context_tree_needs_compositor_update = needs_compositor_update;
    }
    void replace_scroll_state_snapshot(Compositing::ScrollStateSnapshot scroll_state_snapshot) { m_scroll_state_snapshot = move(scroll_state_snapshot); }

private:
    Optional<Compositing::AccumulatedVisualContextTree> m_visual_context_tree;
    Optional<AsyncScrollingStamp> m_async_scrolling_stamp;
    bool m_visual_context_tree_needs_compositor_update { false };
    bool m_did_update_visual_context_tree_in_compositor { false };
    Compositing::ScrollStateSnapshot m_scroll_state_snapshot;
};

// The display list a recording published, with what its commands reference.
struct PublishedDisplayList {
    NonnullRefPtr<Compositing::DisplayList> display_list;
    Compositing::DisplayListResourceSet command_resources;
    // Whether the recording returned the paint command cache source.
    bool is_paint_command_cache_source { false };
    // Whether the recording's display list becomes the paint command cache source.
    bool becomes_paint_command_cache_source { false };
};

// What a frame is built from besides its source.
struct PresentationInputs {
    Compositing::CompositorContextId context_id;
    HTML::PaintConfig paint_config;
    Compositing::KeyboardScrollState keyboard_scroll_state;
    // The resources the paint command cache source references once the recording is published.
    Compositing::DisplayListResourceSet paint_command_cache_source_resources;
    Optional<Gfx::IntRect> present_viewport_rect;
};

// A frame's presentation, sealed where the rendering update begins the frame: what the frame is presented from.
class WEB_API Presentation : public AtomicRefCounted<Presentation> {
public:
    Presentation(SealedPresentationSource source, PresentationInputs inputs, RefPtr<Compositing::DisplayList> paint_command_cache_source)
        : source(move(source))
        , inputs(move(inputs))
        , paint_command_cache_source(move(paint_command_cache_source))
    {
    }
    ~Presentation();

    SealedPresentationSource source;
    PresentationInputs inputs;
    // The paint command cache source a recording that is identical to it returns.
    RefPtr<Compositing::DisplayList> paint_command_cache_source;

    // What the frame in flight presents with, once the rendering update has handed it the presentation: the presenter
    // it presents from (lent to it until the frame is taken in), the recording it publishes (owned by the frame
    // scheduler's ticket), and the sink it hands the frame to.
    RefPtr<NavigablePresenter> presenter;
    Painting::PendingDisplayListRecording* recording { nullptr };
    // The ticket of the recording, if it was submitted with one: the presentation publishes the recording from it, without
    // reaching the document's arena. Released with the presentation.
    void const* recording_ticket { nullptr };
    u64 render_state_generation { 0 };
    RefPtr<CompositorFrameSink> frame_sink;
    bool is_presented_by_frame_in_flight { false };
    // The epoch of the scene the frame in flight presented, if it carried one.
    Optional<u64> presented_scene_epoch;
    // What the frame in flight published, for the main thread to take in.
    Optional<PublishedDisplayList> published;
};

// What a navigable presents to its compositor context from: the resource storage its recordings add to, and the
// display list the compositor context holds with the resources it holds for it. The presenter holds no GC pointer, so
// the frame that presents a recording can take it along.
class WEB_API NavigablePresenter : public AtomicRefCounted<NavigablePresenter> {
public:
    static NonnullRefPtr<NavigablePresenter> create() { return adopt_ref(*new NavigablePresenter); }

    Compositing::DisplayListResourceStorage& resource_storage() { return m_resource_storage; }
    Compositing::DisplayListResourceStorage const& resource_storage() const { return m_resource_storage; }

    // The display list the compositor context holds, and the paint config it was recorded with.
    RefPtr<Compositing::DisplayList> const& compositor_display_list() const { return m_compositor_display_list; }
    Optional<HTML::PaintConfig> const& compositor_display_list_paint_config() const { return m_compositor_display_list_paint_config; }
    void set_compositor_display_list_paint_config(HTML::PaintConfig paint_config) { m_compositor_display_list_paint_config = paint_config; }
    u64 compositor_display_list_visual_context_tree_structural_epoch() const { return m_compositor_display_list_visual_context_tree_structural_epoch; }

    // The resources the compositor context holds for its display list and visual context tree, and those the display
    // list's commands reference.
    Compositing::DisplayListResourceSet const& compositor_display_list_resources() const { return m_compositor_display_list_resources; }
    Compositing::DisplayListResourceSet const& compositor_display_list_command_resources() const { return m_compositor_display_list_command_resources; }

    // The compositor context now holds `display_list`, recorded with `paint_config`.
    void did_hand_display_list_to_compositor(NonnullRefPtr<Compositing::DisplayList>, HTML::PaintConfig, Compositing::DisplayListResourceSet command_resources, Compositing::DisplayListResourceSet resources);
    // The compositor context now holds a new visual context tree for its display list.
    void did_hand_visual_context_tree_to_compositor(Compositing::DisplayListResourceSet resources);
    // Forgets what the compositor context holds: a new compositor process holds nothing.
    void forget_compositor_display_list();

    // Main thread only: whether the frame in flight presents from this presenter, which it owns until the main thread
    // takes the frame in.
    bool is_lent_to_frame_in_flight() const { return m_lent_to_frame_in_flight; }
    void lend_to_frame_in_flight()
    {
        VERIFY(!m_lent_to_frame_in_flight);
        m_lent_to_frame_in_flight = true;
    }
    void take_back_from_frame_in_flight()
    {
        VERIFY(m_lent_to_frame_in_flight);
        m_lent_to_frame_in_flight = false;
    }

    // How many scenes (display lists and visual context trees) this presenter has handed its compositor context. Read
    // from any thread: the render side hands them over beside the main thread.
    u64 presented_scene_epoch() const { return m_presented_scene_epoch.load(); }
    // Called once a frame that carries a scene has been handed over. Returns the scene's epoch.
    u64 did_present_scene() { return m_presented_scene_epoch.fetch_add(1) + 1; }
    // Main thread only: the epoch of the last scene the main thread took in, which its hit-test list was made with.
    // Until it takes in the frame that presented a later one, what is on screen is ahead of what it hit tests.
    u64 adopted_scene_epoch() const { return m_adopted_scene_epoch; }
    // A frame taken in late never takes the epoch back: the main thread may have presented a later scene meanwhile.
    void did_adopt_scene(u64 epoch) { m_adopted_scene_epoch = max(m_adopted_scene_epoch, epoch); }
    bool has_scene_to_adopt() const { return presented_scene_epoch() > m_adopted_scene_epoch; }
    // How many animations the compositor runs on the last visual context tree this presenter handed it. Read from any
    // thread, as the scene epoch.
    u64 compositor_visual_animation_count() const { return m_compositor_visual_animation_count.load(); }

    // Builds the frame that brings the compositor context up to date with `published` (or, if the frame recorded
    // nothing, with its source's tree and scroll state). Reaches no document but through `source`.
    CompositorFrame build_frame(PresentationInputs&, PresentationSource&, Optional<PublishedDisplayList> published);

private:
    NavigablePresenter() = default;

    Compositing::DisplayListResourceStorage m_resource_storage;
    Optional<HTML::PaintConfig> m_compositor_display_list_paint_config;
    RefPtr<Compositing::DisplayList> m_compositor_display_list;
    u64 m_compositor_display_list_visual_context_tree_structural_epoch { 0 };
    Compositing::DisplayListResourceSet m_compositor_display_list_resources;
    Compositing::DisplayListResourceSet m_compositor_display_list_command_resources;
    bool m_lent_to_frame_in_flight { false };
    Atomic<u64> m_presented_scene_epoch { 0 };
    Atomic<u64> m_compositor_visual_animation_count { 0 };
    u64 m_adopted_scene_epoch { 0 };
};

}
