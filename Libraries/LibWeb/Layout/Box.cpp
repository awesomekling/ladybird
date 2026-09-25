/*
 * Copyright (c) 2018-2020, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021-2022, Sam Atkins <atkinssj@serenityos.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/HTMLImageElement.h>
#include <LibWeb/HTML/HTMLInputElement.h>
#include <LibWeb/HTML/HTMLObjectElement.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/Box.h>
#include <LibWeb/Layout/ImageProvider.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxViews.h>

namespace Web::Layout {

Box::Box(DOM::Document& document, GC::Ptr<DOM::Node> node, CSS::LayoutStyle style, RustFFI::NodeKind kind)
    : NodeWithStyle(document, node, move(style), kind)
{
}

Box::Box(DOM::Document& document, BindToPreparedArenaSlot bind, Compositing::RustFFI::NodeSlotId slot, RustFFI::NodeKind kind)
    : NodeWithStyle(document, bind, slot, kind)
{
}

Box::Box(DOM::Document& document, BindToPreparedArenaSlot bind, Compositing::RustFFI::NodeSlotId slot, RustFFI::NodeKind kind, CSS::LayoutStyle style)
    : NodeWithStyle(document, bind, slot, kind, move(style))
{
}

Box::~Box()
{
}

static ImageProvider const& image_provider_for_element(DOM::Element const& element)
{
    if (auto const* image = as_if<HTML::HTMLImageElement>(element))
        return *image;
    if (auto const* input = as_if<HTML::HTMLInputElement>(element))
        return *input;
    if (auto const* object = as_if<HTML::HTMLObjectElement>(element))
        return *object;

    VERIFY_NOT_REACHED();
}

ImageProvider* Box::owned_image_provider() const
{
    return static_cast<ImageProvider*>(RustFFI::layout_arena_owned_image_provider(arena_handle(), Node::slot_id(this)));
}

ImageProvider const& Box::image_provider() const
{
    VERIFY(kind() == RustFFI::NodeKind::ImageBox);
    if (auto* owned = owned_image_provider())
        return *owned;

    auto const* element = dom_node();
    VERIFY(element);
    return image_provider_for_element(as<DOM::Element>(*element));
}

void Box::set_owned_image_provider(NonnullOwnPtr<ImageProvider> image_provider)
{
    VERIFY(kind() == RustFFI::NodeKind::ImageBox);
    RustFFI::layout_arena_set_owned_image_provider(arena_handle(), Node::slot_id(this), image_provider.leak_ptr());
}

bool Box::is_partial_relayout_boundary() const
{
    return RustFFI::layout_arena_node_is_partial_relayout_boundary(arena_handle(), Node::slot_id(this));
}

void Box::notify_content_navigable_of_committed_viewport()
{
    // A navigable another process hosts learns its viewport from the UI process, which the container tells of the
    // viewport's rect when its document is painted.
    if (auto* content_navigable = as_if<HTML::LocalNavigable>(as<HTML::NavigableContainer>(*dom_node()).content_navigable().ptr()))
        content_navigable->set_viewport_size(Painting::content_size(*this));
}

}
