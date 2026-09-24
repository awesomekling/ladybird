/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Vector.h>
#include <LibWeb/DOM/HoverEventData.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Layout/LayoutRustFFI.h>

namespace Web::DOM {

// Everything the render side wants to tell the main side, in one ordered list. A message names the
// node it is about by identity and says what the render side found out about it; the render side
// itself writes nothing on the main side. The main side applies the list at defined points: at the
// end of a layout pass, at the end of a join, and at the start of the rendering opportunity.
//
// A kind whose main-side readers cannot yet wait that long is applied where it is appended. That
// still goes through the list, so messages take effect in the order the render side produced them.
class WEB_API CommitMessages {
    AK_MAKE_NONCOPYABLE(CommitMessages);
    AK_MAKE_NONMOVABLE(CommitMessages);

public:
    AK_ALLOC_WITH_KMALLOC;

    explicit CommitMessages(Document& document)
        : m_document(document)
    {
    }

    // What boxes a node has now. DOM code reads these as bits instead of looking up the node's row.
    void note_box_presence(NodeIdentity, bool has_layout_box, bool has_committed_box);

    // A message a finished layout pass left for this document.
    void append(Layout::RustFFI::FfiCommitMessage const&);

    // Which node the pointer ended up over once a scroll settled. The hit test that decided this
    // ran on committed data and named the node by identity; the main side learns it here instead
    // of from the code that ran the hit test.
    void note_hover_target_after_scroll(NodeIdentity, Optional<HoverEventData>);

    // The node a rebuild escalates to, because the node that asked for it sits under an anonymous
    // parent and only the render side knows where the escalation stops.
    void note_needs_layout_tree_update(NodeIdentity, SetNeedsLayoutTreeUpdateReason);

    void note_style_query_custom_property_reference(NodeIdentity, Optional<CSS::PseudoElement>, Utf16FlyString);
    void note_style_container_query_dependencies(NodeIdentity, u8 dependencies);
    void note_style_query_container_usage(NodeIdentity, u8 usage);
    void note_scroll_state_query_container_usage(NodeIdentity);
    void note_style_query_needs_evaluation_after_layout(NodeIdentity);
    void note_style_viewport_dependency(NodeIdentity);

    // Applies only style-stage reports, leaving layout and event messages at their existing drains.
    void apply_style_messages();

    // Applies every message in order and empties the list.
    void apply();

private:
    enum class Kind : u8 {
        BoxPresence,
        HoverTargetAfterScroll,
        NavigableContainerViewportCommitted,
        NeedsLayoutTreeUpdate,
        SvgResourceReferenced,
        StyleQueryCustomPropertyReference,
        StyleContainerQueryDependencies,
        StyleQueryContainerUsage,
        ScrollStateQueryContainerUsage,
        StyleQueryNeedsEvaluationAfterLayout,
        StyleViewportDependency,
        PendingFontFaceWanted,
        TopLayerZoneRebuildNeeded,
        UnexpectedFragmentedInline,
    };

    struct Message {
        NodeIdentity identity;
        // The second node a message about a pair names. Only SvgResourceReferenced has one.
        NodeIdentity other_identity {};
        Kind kind;
        bool has_layout_box { false };
        bool has_committed_box { false };
        u8 style_container_query_dependencies { 0 };
        u8 style_query_container_usage { 0 };
        // Only the layout tree update trace reads this.
        SetNeedsLayoutTreeUpdateReason layout_tree_update_reason {};
        // Where the pointer was, for the hover events the target change ends in. Only
        // HoverTargetAfterScroll has one.
        Optional<HoverEventData> hover_event_data {};
        Optional<CSS::PseudoElement> pseudo_element;
        Utf16FlyString custom_property_name;
        // The web font face a PendingFontFaceWanted message names, and whether it was offered before.
        u64 pending_face { 0 };
        bool pending_face_has_been_retried { false };
    };

    void apply(Message const&);

    Document& m_document;
    Vector<Message> m_messages;
    Vector<Message> m_style_messages;
    bool m_applying { false };
};

}
