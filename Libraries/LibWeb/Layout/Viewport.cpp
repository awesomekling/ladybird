/*
 * Copyright (c) 2018-2023, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/Layout/Viewport.h>

namespace Web::Layout {

// The build stamps the viewport's row and materialises this shell for it. The row carries no
// style, since the document has no identity in the style mirror to hold one: the shell is what
// asks the style computer for the document's style, where the build used to ask before it built
// the box.
Viewport::Viewport(DOM::Document& document, BindToPreparedArenaSlot bind, Compositing::RustFFI::NodeSlotId slot, RustFFI::NodeKind kind)
    : Box(document, bind, slot, kind, CSS::LayoutStyle { document.style_computer().create_document_style() })
{
    // As in the DOM-backed constructor: the base constructor could not have asked for the
    // navigable's offset, because `is_viewport()` does not answer yes until this box's own
    // constructor runs. The rows the document already has take the navigable's offset along with
    // this one.
    publish_scroll_offset();
}

Viewport::~Viewport() = default;

DOM::Document const& Viewport::dom_node() const
{
    return static_cast<DOM::Document const&>(*Node::dom_node());
}

}
