/*
 * Copyright (c) 2018-2023, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/Layout/Viewport.h>

namespace Web::Layout {

Viewport::Viewport(DOM::Document& document, CSS::LayoutStyle style)
    : Box(document, &document, style, RustFFI::NodeKind::Viewport)
{
    // The base constructor could not have asked for the navigable's offset, nor named the
    // document: it does not yet know this box is the viewport's.
    publish_scroll_offset();
    publish_unique_node_id();
}

Viewport::~Viewport() = default;

DOM::Document const& Viewport::dom_node() const
{
    return static_cast<DOM::Document const&>(*Node::dom_node());
}

}
