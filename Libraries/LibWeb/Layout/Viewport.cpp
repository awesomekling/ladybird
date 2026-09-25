/*
 * Copyright (c) 2018-2023, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/Layout/Viewport.h>

namespace Web::Layout {

// The build stamps the viewport's row with the document's style and the navigable's scroll offset,
// which it is handed before it starts, and this shell is materialised for the row after the build.
Viewport::Viewport(DOM::Document& document, BindToPreparedArenaSlot bind, Compositing::RustFFI::NodeSlotId slot, RustFFI::NodeKind kind)
    : Box(document, bind, slot, kind)
{
    initialize_stamped_style_record();
}

Viewport::~Viewport() = default;

DOM::Document const& Viewport::dom_node() const
{
    return static_cast<DOM::Document const&>(*Node::dom_node());
}

}
