/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibCompositing/Scrolling/AsyncScrollingState.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Painting/BoxSlot.h>
#include <LibWeb/TextAffinity.h>
#include <LibWebCommon/PixelUnits.h>

namespace Web::Painting {

enum class ScrollDirection : u8 {
    Horizontal,
    Vertical,
};

enum class ScrollHandled {
    No,
    Yes,
};

// https://drafts.csswg.org/css-scroll-snap-1/#scroll-types
enum class ScrollKind : u8 {
    // A scroll with an intended direction, such as a mouse wheel step or an arrow key press.
    Relative,
    // A scroll with only an intended end position, such as dragging a scrollbar thumb.
    Absolute,
};

enum class ScrollBlockDirection {
    No,
    Yes,
};

CSSPixelPoint scroll_offset(BoxSlot const&);
CSSPixelPoint minimum_scroll_offset(BoxSlot const&);
CSSPixelPoint maximum_scroll_offset(BoxSlot const&);
CSSPixelPoint clamp_scroll_offset(BoxSlot const&, CSSPixelPoint);
CSSPixelRect scroll_snapport_rect(BoxSlot const&);
CSSPixelRect scroll_snapport_rect(BoxSlot const&, CSSPixelRect scrollport);
CSSPixelPoint scroll_offset(DOM::Document const&, DOM::NodeIdentity);
CSSPixelPoint minimum_scroll_offset(DOM::Document const&, DOM::NodeIdentity);
CSSPixelPoint maximum_scroll_offset(DOM::Document const&, DOM::NodeIdentity);
CSSPixelPoint clamp_scroll_offset(DOM::Document const&, DOM::NodeIdentity, CSSPixelPoint);
CSSPixelRect scroll_snapport_rect(DOM::Document const&, DOM::NodeIdentity);
CSSPixelRect scroll_snapport_rect(DOM::Document const&, DOM::NodeIdentity, CSSPixelRect scrollport);
// The overflow the viewport applies to a wheel, in both axes. Both axes come from the same
// element - the root's, or the body's where the root defers to it - so they are resolved together.
struct ViewportWheelOverflow {
    CSS::Overflow x { CSS::Overflow::Auto };
    CSS::Overflow y { CSS::Overflow::Auto };
};

ViewportWheelOverflow overflow_values_applied_to_viewport_for_wheel_scrolling(DOM::Document const&);
struct WheelScrollableAxes {
    bool horizontal { false };
    bool vertical { false };
};

WheelScrollableAxes wheel_scrollable_axes(BoxSlot const&);
bool could_be_scrolled_by_wheel_event(BoxSlot const&);
bool could_be_scrolled_by_wheel_event(BoxSlot const&, ScrollDirection);
WheelScrollableAxes wheel_scrollable_axes(DOM::Document const&, DOM::NodeIdentity);
bool could_be_scrolled_by_wheel_event(DOM::Document const&, DOM::NodeIdentity);
bool could_be_scrolled_by_wheel_event(DOM::Document const&, DOM::NodeIdentity, ScrollDirection);
WEB_API Optional<Compositing::AsyncScrollNodeStableID> async_scroll_node_stable_id(BoxSlot const&);
// The scrolling box the compositor names by the stable id, in the document.
WEB_API BoxSlot scrolling_box_for_async_scroll_node_stable_id(DOM::Document const&, Compositing::AsyncScrollNodeStableID const&);
ScrollHandled set_scroll_offset(BoxSlot const&, CSSPixelPoint);
ScrollHandled set_scroll_offset_from_user_input(BoxSlot const&, CSSPixelPoint, ScrollKind = ScrollKind::Relative);
ScrollHandled scroll_by(BoxSlot const&, double delta_x, double delta_y, ScrollKind = ScrollKind::Relative);
// The box the walk scrolled, if any.
BoxSlot wheel_scroll_along_containing_block_chain(BoxSlot const&, double wheel_delta_x, double wheel_delta_y, ScrollKind = ScrollKind::Relative);

WEB_API BoxSlot scrolling_box_for_scroll_step_in_containing_block_chain(BoxSlot const&, CSSPixelPoint delta);
WEB_API BoxSlot first_wheel_scrollable_box_in_containing_block_chain(BoxSlot const&);
void scroll_text_offset_into_view(DOM::Text const&, size_t offset, TextAffinity = TextAffinity::Downstream, ScrollBlockDirection = ScrollBlockDirection::Yes);

}
