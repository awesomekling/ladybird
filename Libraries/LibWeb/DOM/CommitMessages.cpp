/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/AnyOf.h>
#include <AK/ScopeGuard.h>
#include <LibGfx/FontCascadeList.h>
#include <LibWeb/CSS/ScrollStateContainerQuery.h>
#include <LibWeb/DOM/CommitMessages.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/Dump.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/NavigableContainer.h>
#include <LibWeb/Page/EventHandler.h>
#include <LibWeb/Painting/BoxSlot.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/SVG/SVGElement.h>

namespace Web::DOM {

// A container that has left the document since its box was committed has no navigable to size.
static HTML::LocalNavigable* local_content_navigable(Painting::BoxSlot const& navigable_container_viewport)
{
    auto* container = as_if<HTML::NavigableContainer>(navigable_container_viewport.dom_node().ptr());
    return container ? as_if<HTML::LocalNavigable>(container->content_navigable().ptr()) : nullptr;
}

void CommitMessages::note_box_presence(NodeIdentity identity, bool has_layout_box, bool has_committed_box)
{
    // OPTIMIZATION: With nothing queued ahead of it, the message would be applied the moment it is
    //               queued, so the bits are set without queuing it. What setting them queues is
    //               applied after them, as the drain below would.
    if (m_messages.is_empty() && !m_applying) {
        {
            m_applying = true;
            ScopeGuard done = [&] { m_applying = false; };
            if (auto node = identity.resolve(m_document))
                node->set_box_presence(has_layout_box, has_committed_box);
        }
        if (!m_messages.is_empty())
            apply_script_free();
        return;
    }
    m_messages.append(Message {
        .identity = identity,
        .kind = Kind::BoxPresence,
        .has_layout_box = has_layout_box,
        .has_committed_box = has_committed_box,
        .pseudo_element = {},
        .custom_property_name = {},
    });
    // The bits answer `Node::is_rendered()`, which DOM code asks in the middle of a layout pass,
    // so they cannot wait for one of the drain points yet.
    apply_script_free();
}

void CommitMessages::note_hover_target_after_scroll(NodeIdentity identity, Optional<HoverEventData> hover_event_data)
{
    m_messages.append(Message {
        .identity = identity,
        .kind = Kind::HoverTargetAfterScroll,
        .layout_tree_update_reason = {},
        .hover_event_data = move(hover_event_data),
        .pseudo_element = {},
        .custom_property_name = {},
    });
    // The hover events a target change ends in are dispatched from whatever decided the target, so
    // that they keep their order relative to the rendering opportunity's steps. That is sooner than
    // any of the drain points, so this kind is applied where it is appended.
    apply();
}

void CommitMessages::note_needs_layout_tree_update(NodeIdentity identity, SetNeedsLayoutTreeUpdateReason reason)
{
    m_messages.append(Message {
        .identity = identity,
        .kind = Kind::NeedsLayoutTreeUpdate,
        .layout_tree_update_reason = reason,
        .hover_event_data = {},
        .pseudo_element = {},
        .custom_property_name = {},
    });
    // The mark decides what the next tree build does, and the DOM side reads that back as soon as
    // the mutation that made it returns.
    apply_script_free();
}

void CommitMessages::note_style_query_custom_property_reference(NodeIdentity identity, Optional<CSS::PseudoElement> pseudo_element, Utf16FlyString name)
{
    m_style_messages.append(Message {
        .identity = identity,
        .kind = Kind::StyleQueryCustomPropertyReference,
        .pseudo_element = pseudo_element,
        .custom_property_name = move(name),
    });
}

void CommitMessages::note_style_container_query_dependencies(NodeIdentity identity, u8 dependencies)
{
    m_style_messages.append(Message {
        .identity = identity,
        .kind = Kind::StyleContainerQueryDependencies,
        .style_container_query_dependencies = dependencies,
        .pseudo_element = {},
        .custom_property_name = {},
    });
}

void CommitMessages::note_style_query_container_usage(NodeIdentity identity, u8 usage)
{
    m_style_messages.append(Message {
        .identity = identity,
        .kind = Kind::StyleQueryContainerUsage,
        .style_query_container_usage = usage,
        .pseudo_element = {},
        .custom_property_name = {},
    });
}

void CommitMessages::note_scroll_state_query_container_usage(NodeIdentity identity)
{
    m_style_messages.append(Message {
        .identity = identity,
        .kind = Kind::ScrollStateQueryContainerUsage,
        .pseudo_element = {},
        .custom_property_name = {},
    });
}

void CommitMessages::note_style_viewport_dependency(NodeIdentity identity)
{
    m_style_messages.append(Message {
        .identity = identity,
        .kind = Kind::StyleViewportDependency,
        .pseudo_element = {},
        .custom_property_name = {},
    });
}

void CommitMessages::apply_style_messages()
{
    if (m_applying)
        return;
    m_applying = true;
    ScopeGuard done = [&] { m_applying = false; };

    while (!m_style_messages.is_empty()) {
        auto messages = move(m_style_messages);
        for (auto const& message : messages)
            apply(message);
    }
}

// A message from the render side names its node by the style node the style tree gave it, with 0
// for the document.
void CommitMessages::append(Layout::RustFFI::FfiCommitMessage const& message)
{
    auto identity = message.style_node == 0
        ? NodeIdentity::of_document()
        : NodeIdentity::of_style_node(CSS::StyleNodeID { message.style_node });
    switch (message.kind) {
    case Layout::RustFFI::FfiCommitMessageKind::NavigableContainerViewportCommitted:
        // A viewport the container's content navigable already has would change nothing where it is applied. Left
        // queued, it would have every read in the documents the container embeds bring this document up to date.
        if (auto box = Painting::BoxSlot::bound_to(m_document, identity)) {
            if (auto* content_navigable = local_content_navigable(box); content_navigable && content_navigable->viewport_size() == Painting::content_size(box))
                return;
        }
        m_messages.append(Message { .identity = identity, .kind = Kind::NavigableContainerViewportCommitted, .pseudo_element = {}, .custom_property_name = {} });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::UnexpectedFragmentedInline:
        m_messages.append(Message { .identity = identity, .kind = Kind::UnexpectedFragmentedInline, .pseudo_element = {}, .custom_property_name = {} });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::LayoutTreeRebuildRequested:
        m_messages.append(Message {
            .identity = identity,
            .kind = Kind::NeedsLayoutTreeUpdate,
            .layout_tree_update_reason = SetNeedsLayoutTreeUpdateReason::PseudoElementBoxEscapedRebuildRoot,
            .hover_event_data = {},
            .pseudo_element = {},
            .custom_property_name = {},
        });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::TopLayerZoneRebuildNeeded:
        m_messages.append(Message { .identity = identity, .kind = Kind::TopLayerZoneRebuildNeeded, .pseudo_element = {}, .custom_property_name = {} });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::PendingFontFaceWanted:
        m_messages.append(Message {
            .identity = identity,
            .kind = Kind::PendingFontFaceWanted,
            .pseudo_element = {},
            .custom_property_name = {},
            .pending_face = message.pending_face,
            .pending_face_has_been_retried = message.pending_face_has_been_retried,
        });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::ListItemCountersStale:
        // An owner that has left the document since the build named it renders nothing.
        m_messages.append(Message {
            .identity = identity,
            .kind = Kind::NeedsLayoutTreeUpdate,
            .layout_tree_update_reason = SetNeedsLayoutTreeUpdateReason::ListItemCounters,
            .hover_event_data = {},
            .pseudo_element = {},
            .custom_property_name = {},
        });
        return;
    case Layout::RustFFI::FfiCommitMessageKind::LayoutTreeReplaced:
        // The layout host takes this one in itself, after the others the delivery carries.
        VERIFY_NOT_REACHED();
    case Layout::RustFFI::FfiCommitMessageKind::SvgResourceReferenced:
        m_messages.append(Message {
            .identity = identity,
            .other_identity = NodeIdentity::of_style_node(CSS::StyleNodeID { message.other_style_node }),
            .kind = Kind::SvgResourceReferenced,
            .pseudo_element = {},
            .custom_property_name = {},
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

void CommitMessages::apply_script_free()
{
    if (m_applying)
        return;
    m_applying = true;
    ScopeGuard done = [&] { m_applying = false; };

    // The continuations stay queued for the next drain point, in the order they arrived.
    Vector<Message> continuations;
    while (!m_messages.is_empty()) {
        auto messages = move(m_messages);
        for (auto& message : messages) {
            if (is_continuation(message.kind))
                continuations.append(move(message));
            else
                apply(message);
        }
    }
    m_messages = move(continuations);
}

bool CommitMessages::has_queued_navigable_container_viewport() const
{
    return any_of(m_messages, [](auto const& message) { return message.kind == Kind::NavigableContainerViewportCommitted; });
}

bool CommitMessages::is_continuation(Kind kind)
{
    return kind == Kind::HoverTargetAfterScroll || kind == Kind::NavigableContainerViewportCommitted;
}

void CommitMessages::apply(Message const& message)
{
    switch (message.kind) {
    case Kind::BoxPresence:
        // A node that left the tree between the message and here has nothing left to tell.
        if (auto node = message.identity.resolve(m_document))
            node->set_box_presence(message.has_layout_box, message.has_committed_box);
        return;
    case Kind::HoverTargetAfterScroll:
        // A node that has left the tree since the hit test named it is nothing to hover, which is
        // also what the hit test naming nothing means.
        if (auto navigable = m_document.navigable())
            navigable->event_handler().apply_hover_target_after_scroll({}, message.identity.resolve(m_document), message.hover_event_data);
        return;
    case Kind::NeedsLayoutTreeUpdate:
        if (auto node = message.identity.resolve(m_document))
            node->set_needs_layout_tree_update(true, message.layout_tree_update_reason);
        return;
    case Kind::NavigableContainerViewportCommitted:
        // A navigable another process hosts learns its viewport from the UI process, which the container tells of
        // the viewport's rect when its document is painted.
        if (auto box = Painting::BoxSlot::bound_to(m_document, message.identity)) {
            if (auto* content_navigable = local_content_navigable(box))
                content_navigable->set_viewport_size(Painting::content_size(box));
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
    case Kind::StyleQueryCustomPropertyReference:
        if (auto* element = as_if<Element>(message.identity.resolve(m_document).ptr()))
            element->record_style_query_custom_property_reference(message.pseudo_element, message.custom_property_name);
        return;
    case Kind::StyleContainerQueryDependencies:
        if (auto* element = as_if<Element>(message.identity.resolve(m_document).ptr())) {
            if (message.style_container_query_dependencies & 1)
                element->set_style_depends_on_size_container_query();
            if (message.style_container_query_dependencies & 2)
                element->set_style_depends_on_style_container_query();
            element->finish_recording_container_query_dependencies();
        }
        return;
    case Kind::StyleQueryContainerUsage:
        if (auto* element = as_if<Element>(message.identity.resolve(m_document).ptr())) {
            if (message.style_query_container_usage & 1)
                element->set_is_size_query_container();
            if (message.style_query_container_usage & 2) {
                element->set_is_style_query_container();
                if (auto* root = m_document.document_element())
                    root->set_is_style_query_container();
            }
        }
        return;
    case Kind::ScrollStateQueryContainerUsage:
        if (auto* element = as_if<Element>(message.identity.resolve(m_document).ptr()))
            m_document.scroll_state_query_containers().snapshot_for_query(*element);
        return;
    case Kind::StyleViewportDependency:
        if (auto* element = as_if<Element>(message.identity.resolve(m_document).ptr()))
            element->set_style_depends_on_viewport_metrics();
        return;
    case Kind::PendingFontFaceWanted:
        Gfx::request_wanted_pending_face(message.pending_face, message.pending_face_has_been_retried);
        return;
    case Kind::TopLayerZoneRebuildNeeded:
        m_document.set_top_layer_needs_layout_zone_rebuild();
        return;
    case Kind::UnexpectedFragmentedInline:
        if (auto box = Painting::BoxSlot::bound_to(m_document, message.identity)) {
            dbgln("FIXME: InlineFormattingContext::dimension_box_on_line got unexpected box in inline context:");
            dump_tree(box);
        }
        return;
    }
    VERIFY_NOT_REACHED();
}

}
