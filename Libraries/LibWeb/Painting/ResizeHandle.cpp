/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Page/ElementResizeAction.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/ResizeHandle.h>
#include <LibWeb/UIEvents/EventNames.h>
#include <LibWeb/UIEvents/PointerEvent.h>
#include <LibWebCommon/UIEvents/MouseButton.h>

namespace Web::Painting {

NonnullRefPtr<ResizeHandle> ResizeHandle::create(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot)
{
    return adopt_ref(*new ResizeHandle(document, slot));
}

ResizeHandle::ResizeHandle(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot)
    : ChromeWidget(document, slot)
    , m_element(dom_node_identity_of_committed_slot(document, slot))
{
}

Optional<CSS::CursorPredefined> ResizeHandle::cursor() const
{
    if (!is_current())
        return {};
    auto axes = Layout::RustFFI::layout_arena_paintable_physical_resize_axes(arena(), slot());
    if (axes.vertical) {
        if (axes.horizontal) {
            if (Layout::RustFFI::layout_arena_paintable_is_chrome_mirrored(arena(), slot()))
                return CSS::CursorPredefined::SwResize;
            return CSS::CursorPredefined::SeResize;
        }
        return CSS::CursorPredefined::NsResize;
    }
    return CSS::CursorPredefined::EwResize;
}

MouseAction ResizeHandle::handle_pointer_event(Utf16FlyString const& type, unsigned button, CSSPixelPoint visual_viewport_position)
{
    if (type == UIEvents::EventNames::pointermove) {
        if (!m_resize_action)
            return MouseAction::None;
    } else if (button != UIEvents::MouseButton::Primary) {
        return MouseAction::None;
    }

    auto document = this->document();
    auto* element = document ? as_if<DOM::Element>(m_element.resolve(*document).ptr()) : nullptr;
    if (!element || !element->is_connected()) {
        m_resize_action.clear();
        return MouseAction::None;
    }

    if (!m_resize_action)
        m_resize_action = make<ElementResizeAction>(*element, visual_viewport_position);
    else
        m_resize_action->handle_pointer_move(visual_viewport_position);

    if (type == UIEvents::EventNames::pointerup) {
        m_resize_action.clear();
        return MouseAction::None;
    }

    return MouseAction::CaptureInput;
}

}
