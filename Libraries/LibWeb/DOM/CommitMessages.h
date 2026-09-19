/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Vector.h>
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

    // The node a rebuild escalates to, because the node that asked for it sits under an anonymous
    // parent and only the render side knows where the escalation stops.
    void note_needs_layout_tree_update(NodeIdentity, SetNeedsLayoutTreeUpdateReason);

    // Applies every message in order and empties the list.
    void apply();

private:
    enum class Kind : u8 {
        BoxPresence,
        ContentSizeChangedForContainerQueries,
        NavigableContainerViewportCommitted,
        NeedsLayoutTreeUpdate,
        SvgResourceReferenced,
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
        // Only the layout tree update trace reads this.
        SetNeedsLayoutTreeUpdateReason layout_tree_update_reason {};
    };

    void apply(Message const&);

    Document& m_document;
    Vector<Message> m_messages;
    bool m_applying { false };
};

}
