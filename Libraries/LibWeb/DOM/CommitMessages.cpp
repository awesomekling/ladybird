/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/ScopeGuard.h>
#include <LibWeb/DOM/CommitMessages.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Node.h>

namespace Web::DOM {

void CommitMessages::note_box_presence(NodeIdentity identity, bool has_layout_box, bool has_committed_box)
{
    m_messages.append(Message {
        .identity = identity,
        .kind = Kind::BoxPresence,
        .has_layout_box = has_layout_box,
        .has_committed_box = has_committed_box,
    });
    // The bits answer `Node::is_rendered()`, which DOM code asks in the middle of a layout pass,
    // so they cannot wait for one of the drain points yet.
    apply();
}

void CommitMessages::apply()
{
    // A message appended while the list is being applied belongs after the ones already in flight,
    // and the loop below reaches it there.
    if (m_applying)
        return;
    m_applying = true;
    ScopeGuard done = [&] { m_applying = false; };

    while (!m_messages.is_empty()) {
        auto messages = move(m_messages);
        for (auto const& message : messages)
            apply(message);
    }
}

void CommitMessages::apply(Message const& message)
{
    switch (message.kind) {
    case Kind::BoxPresence:
        // A node that left the tree between the message and here has nothing left to tell.
        if (auto node = message.identity.resolve(m_document))
            node->set_box_presence(message.has_layout_box, message.has_committed_box);
        return;
    }
    VERIFY_NOT_REACHED();
}

}
