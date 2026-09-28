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

bool Box::is_partial_relayout_boundary() const
{
    return RustFFI::layout_arena_node_is_partial_relayout_boundary(arena_handle(), Node::slot_id(this));
}

}
