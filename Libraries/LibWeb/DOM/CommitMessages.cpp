/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/ScopeGuard.h>
#include <LibWeb/CSS/Invalidation/ContainerQueryInvalidator.h>
#include <LibWeb/DOM/CommitMessages.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/Dump.h>
#include <LibWeb/Layout/Box.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/SVG/SVGElement.h>

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

void CommitMessages::note_needs_layout_tree_update(NodeIdentity identity, SetNeedsLayoutTreeUpdateReason reason)
{
    m_messages.append(Message {
        .identity = identity,
        .kind = Kind::NeedsLayoutTreeUpdate,
        .layout_tree_update_reason = reason,
    });
    // The mark decides what the next tree build does, and the DOM side reads that back as soon as
    // the mutation that made it returns.
    apply();
}

// A message from the render side names its node by the style node the style tree gave it, with 0
// for the document.
void CommitMessages::append(Layout::RustFFI::FfiCommitMessage const& message)
{
    auto identity = message.style_node == 0
        ? NodeIdentity::of_document()
        : NodeIdentity::of_style_node(CSS::StyleNodeID { message.style_node });
    switch (message.kind) {
    case Layout::RustFFI::FfiCommitMessageKind::ContentSizeChangedForContainerQueries:
        m_messages.append(Message { .identity = identity, .kind = Kind::ContentSizeChangedForContainerQueries });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::NavigableContainerViewportCommitted:
        m_messages.append(Message { .identity = identity, .kind = Kind::NavigableContainerViewportCommitted });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::UnexpectedFragmentedInline:
        m_messages.append(Message { .identity = identity, .kind = Kind::UnexpectedFragmentedInline });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::LayoutTreeRebuildRequested:
        m_messages.append(Message {
            .identity = identity,
            .kind = Kind::NeedsLayoutTreeUpdate,
            .layout_tree_update_reason = SetNeedsLayoutTreeUpdateReason::PseudoElementBoxEscapedRebuildRoot,
        });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::TopLayerZoneRebuildNeeded:
        m_messages.append(Message { .identity = identity, .kind = Kind::TopLayerZoneRebuildNeeded });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::SvgResourceReferenced:
        m_messages.append(Message {
            .identity = identity,
            .other_identity = NodeIdentity::of_style_node(CSS::StyleNodeID { message.other_style_node }),
            .kind = Kind::SvgResourceReferenced,
        });
        return;
    }
    VERIFY_NOT_REACHED();
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
    case Kind::ContentSizeChangedForContainerQueries:
        // Only an element can be a query container; the viewport names the document, which is not.
        if (auto* element = as_if<Element>(message.identity.resolve(m_document).ptr()))
            CSS::Invalidation::invalidate_descendant_styles_depending_on_size_container_query(*element);
        return;
    case Kind::NeedsLayoutTreeUpdate:
        if (auto node = message.identity.resolve(m_document))
            node->set_needs_layout_tree_update(true, message.layout_tree_update_reason);
        return;
    case Kind::NavigableContainerViewportCommitted:
        // The committed box is the one the identity is bound to in the arena; no DOM node is asked
        // for its layout node.
        if (auto* arena = m_document.layout_node_arena_if_created()) {
            if (auto* box = as_if<Layout::Box>(message.identity.bound_layout_node(*arena)))
                box->notify_content_navigable_of_committed_viewport();
        }
        return;
    case Kind::SvgResourceReferenced: {
        // Either element may have left the document since the build placed the resource box; the
        // registration only matters while both are still here.
        auto* resource = as_if<SVG::SVGElement>(message.identity.resolve(m_document).ptr());
        auto* referencing_element = as_if<Element>(message.other_identity.resolve(m_document).ptr());
        if (resource && referencing_element)
            resource->register_resource_box_referencing_element({}, *referencing_element);
        return;
    }
    case Kind::TopLayerZoneRebuildNeeded:
        m_document.set_top_layer_needs_layout_zone_rebuild();
        return;
    case Kind::UnexpectedFragmentedInline:
        if (auto* arena = m_document.layout_node_arena_if_created()) {
            if (auto* node = message.identity.bound_layout_node(*arena)) {
                dbgln("FIXME: InlineFormattingContext::dimension_box_on_line got unexpected box in inline context:");
                dump_tree(*node);
            }
        }
        return;
    }
    VERIFY_NOT_REACHED();
}

}
