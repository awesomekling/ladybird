/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Vector.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>

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

    // Applies every message in order and empties the list.
    void apply();

private:
    enum class Kind : u8 {
        BoxPresence,
    };

    struct Message {
        NodeIdentity identity;
        Kind kind;
        bool has_layout_box { false };
        bool has_committed_box { false };
    };

    void apply(Message const&);

    Document& m_document;
    Vector<Message> m_messages;
    bool m_applying { false };
};

}
