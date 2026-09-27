/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Badge.h>
#include <AK/Noncopyable.h>
#include <AK/RefPtr.h>
#include <AK/Vector.h>
#include <LibGC/Cell.h>
#include <LibGC/Ptr.h>
#include <LibGC/WeakHashSet.h>
#include <LibWeb/CSS/StyleEngineIdentifiers.h>
#include <LibWeb/DOM/InvalidationJournal.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Painting/QueryView.h>

namespace Web::CSS {

class StyleEngine;

}

namespace Web::DOM {

class RenderInputsEntrance;

// What the main thread writes of a document for the render side to restyle it and lay it out from: the style engine's
// inputs, the marks of the invalidation journal that can move a box or change what a geometry read converts its rects
// through, the layout tree update marks, and the style and layout work the document owes. A write to any of it leaves
// the query snapshot the document published describing a document that is no longer there, so nothing writes it but
// through Document::render_inputs_for_write(), which takes the snapshot away first. What is read of it goes through
// Document::render_inputs().
//
// This is also the only way the main thread sends the render owner what changed: a style transaction's input goes to the
// owner as a Change through the style engine's write handle, which only render_inputs_for_write() hands out, and the
// Rust side's Change sender takes the proof of that handle along.
class WEB_API RenderInputs {
    AK_MAKE_NONCOPYABLE(RenderInputs);
    AK_MAKE_NONMOVABLE(RenderInputs);

public:
    RenderInputs(Badge<RenderInputsEntrance>, Document&);

    void visit_edges(GC::Cell::Visitor&);

    // The document's style engine, to publish input to or to run a style update on.
    [[nodiscard]] CSS::StyleEngine& style_engine();

    // The marks of the invalidation journal that can move a box, or change what a geometry read converts its rects
    // through. The marks that only repaint are made through Document::invalidation_journal().
    void note_needs_layout_update(NodeIdentity, SetNeedsLayoutReason, Layout::LayoutUpdatePropagation);
    void note_needs_layout_tree_update(NodeIdentity, SetNeedsLayoutTreeUpdateReason);
    void note_editability_stamps(NodeIdentity);
    void note_is_in_focused_text_control(NodeIdentity);
    void note_scroll_offset(NodeIdentity, bool offset_changed);
    void note_pseudo_element_scroll_offset(NodeIdentity generator, CSS::PseudoElement, CSSPixelPoint, bool offset_changed);
    void note_text_data(Text&, bool whitespace_state_changed);
    void note_svg_attribute_facts(NodeIdentity);
    void note_table_spans(NodeIdentity);
    void note_visual_context_box_dirty(Compositing::RustFFI::NodeSlotId, Layout::RustFFI::FfiVisualContextBoxDirtyKind);
    void note_visual_context_full_rebuild(Layout::RustFFI::FfiVisualContextGlobalRebuildReason);
    void note_svg_paint_resources_changed();
    void note_visual_viewport_transform();

    // The layout tree update marks the arena keeps for the nodes of the document. Whether the mark changed.
    bool merge_layout_tree_update_mark(CSS::StyleNodeID, bool value, u8 reuse_reason);
    bool set_child_needs_layout_tree_update(CSS::StyleNodeID, bool value);
    void set_needs_full_layout_tree_update(bool);

    // An element whose style attribute changed, for the next style update to parse.
    void mark_style_attribute_dirty(Element&);
    [[nodiscard]] bool has_elements_with_dirty_style_attributes() const { return !m_elements_with_dirty_style_attributes.is_empty(); }
    [[nodiscard]] GC::WeakHashSet<Element> take_elements_with_dirty_style_attributes();

    [[nodiscard]] bool needs_media_rule_evaluation() const { return m_needs_media_rule_evaluation; }
    void set_needs_media_rule_evaluation(bool value) { m_needs_media_rule_evaluation = value; }

    // The animation effects whose style the next style update samples.
    [[nodiscard]] bool needs_animated_style_update() const { return m_needs_animated_style_update; }
    void set_needs_animated_style_update(bool value) { m_needs_animated_style_update = value; }
    [[nodiscard]] GC::WeakHashSet<Animations::KeyframeEffect> const& effects_needing_animated_style_update() const { return m_effects_needing_animated_style_update; }
    [[nodiscard]] GC::WeakHashSet<Animations::KeyframeEffect>& effects_needing_animated_style_update() { return m_effects_needing_animated_style_update; }
    [[nodiscard]] GC::WeakHashSet<Animations::KeyframeEffect>& effects_needing_animated_style_update_after_current_update() { return m_effects_needing_animated_style_update_after_current_update; }
    // An animation that skipped a per-frame style update catches up on the next read of its target.
    [[nodiscard]] bool has_throttled_animation_style_update() const { return m_has_throttled_animation_style_update; }
    void set_has_throttled_animation_style_update(bool value) { m_has_throttled_animation_style_update = value; }
    [[nodiscard]] bool force_throttled_animation_style_update() const { return m_force_throttled_animation_style_update; }
    void set_force_throttled_animation_style_update(bool value) { m_force_throttled_animation_style_update = value; }

    // The top layer changes the next layout update builds the layout zone of the top layer again for.
    [[nodiscard]] bool has_pending_top_layer_change() const { return !m_elements_with_pending_top_layer_membership_change.is_empty() || m_top_layer_needs_layout_zone_rebuild; }
    void note_top_layer_membership_change(GC::Ref<Element>);
    void set_top_layer_needs_layout_zone_rebuild() { m_top_layer_needs_layout_zone_rebuild = true; }
    [[nodiscard]] Vector<GC::Ref<Element>> take_top_layer_changes();

private:
    Document& m_document;

    GC::WeakHashSet<Element> m_elements_with_dirty_style_attributes;
    bool m_needs_media_rule_evaluation { false };

    bool m_needs_animated_style_update { false };
    GC::WeakHashSet<Animations::KeyframeEffect> m_effects_needing_animated_style_update;
    GC::WeakHashSet<Animations::KeyframeEffect> m_effects_needing_animated_style_update_after_current_update;
    bool m_has_throttled_animation_style_update { false };
    bool m_force_throttled_animation_style_update { false };

    Vector<GC::Ref<Element>> m_elements_with_pending_top_layer_membership_change;
    bool m_top_layer_needs_layout_zone_rebuild { false };
};

// A document's render inputs, and the query snapshot it published over them. A snapshot is present only while nothing
// was written to the inputs since it was published, which is what makes a geometry read of it clean: nothing reaches
// the inputs to write them but for_write(), and it drops the snapshot first.
class RenderInputsEntrance {
    AK_MAKE_NONCOPYABLE(RenderInputsEntrance);
    AK_MAKE_NONMOVABLE(RenderInputsEntrance);

public:
    explicit RenderInputsEntrance(Document& document)
        : m_inputs({}, document)
    {
    }

    [[nodiscard]] RenderInputs const& inputs() const { return m_inputs; }
    [[nodiscard]] RenderInputs& for_write()
    {
        if (m_query_snapshot)
            drop_query_snapshot();
        return m_inputs;
    }

    void visit_edges(GC::Cell::Visitor& visitor) { m_inputs.visit_edges(visitor); }

    [[nodiscard]] RefPtr<Painting::QuerySnapshot const> query_snapshot() const { return m_query_snapshot; }
    [[nodiscard]] bool has_query_snapshot() const { return m_query_snapshot; }
    void publish_query_snapshot(NonnullRefPtr<Painting::QuerySnapshot const> snapshot) { m_query_snapshot = move(snapshot); }

private:
    void drop_query_snapshot();

    RenderInputs m_inputs;
    RefPtr<Painting::QuerySnapshot const> m_query_snapshot;
};

}
