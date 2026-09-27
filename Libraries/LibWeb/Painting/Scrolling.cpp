/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibCompositing/DisplayList/DisplayListCommand.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/EventTarget.h>
#include <LibWeb/DOM/Text.h>
#include <LibWeb/HTML/EventNames.h>
#include <LibWeb/HTML/HTMLBodyElement.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/PaintingRustBridge.h>
#include <LibWeb/Painting/Scrolling.h>

namespace Web::Painting {

static BoxSlot scroll_row(DOM::Document const& document, DOM::NodeIdentity identity)
{
    return BoxSlot::bound_to(document, identity);
}

CSSPixelPoint scroll_offset(BoxSlot const& row)
{
    if (!has_committed_box(row))
        return {};

    // The box publishes the offset stored for what it is the box of, so this reads the arena
    // rather than asking the element, the pseudo-element or the navigable where that store is.
    // A write still in the journal lands first.
    row.document().drain_invalidation_journal();
    return Layout::RustFFI::layout_arena_published_scroll_offset(row.arena(), row.slot());
}

CSSPixelPoint minimum_scroll_offset(BoxSlot const& row)
{
    if (!row)
        return {};
    return Layout::RustFFI::layout_arena_paintable_minimum_scroll_offset(row.arena(), row.slot());
}

CSSPixelPoint maximum_scroll_offset(BoxSlot const& row)
{
    if (!row)
        return {};
    return Layout::RustFFI::layout_arena_paintable_maximum_scroll_offset(row.arena(), row.slot());
}

CSSPixelPoint clamp_scroll_offset(BoxSlot const& row, CSSPixelPoint offset)
{
    if (!scrollable_overflow_rect(row).has_value())
        return offset;

    auto minimum_offset = minimum_scroll_offset(row);
    auto maximum_offset = maximum_scroll_offset(row);
    return {
        clamp(offset.x(), minimum_offset.x(), maximum_offset.x()),
        clamp(offset.y(), minimum_offset.y(), maximum_offset.y()),
    };
}

CSSPixelRect scroll_snapport_rect(BoxSlot const& row, CSSPixelRect scrollport)
{
    if (!has_committed_box(row))
        return scrollport;
    return Layout::RustFFI::layout_arena_scroll_snapport_rect(row.arena(), row.slot(), scrollport);
}

CSSPixelRect scroll_snapport_rect(BoxSlot const& row)
{
    if (!has_committed_box(row))
        return {};
    return scroll_snapport_rect(row, absolute_padding_box_rect(row));
}

CSSPixelPoint scroll_offset(DOM::Document const& document, DOM::NodeIdentity identity)
{
    return scroll_offset(scroll_row(document, identity));
}

CSSPixelPoint minimum_scroll_offset(DOM::Document const& document, DOM::NodeIdentity identity)
{
    return minimum_scroll_offset(scroll_row(document, identity));
}

CSSPixelPoint maximum_scroll_offset(DOM::Document const& document, DOM::NodeIdentity identity)
{
    return maximum_scroll_offset(scroll_row(document, identity));
}

CSSPixelPoint clamp_scroll_offset(DOM::Document const& document, DOM::NodeIdentity identity, CSSPixelPoint offset)
{
    return clamp_scroll_offset(scroll_row(document, identity), offset);
}

CSSPixelRect scroll_snapport_rect(DOM::Document const& document, DOM::NodeIdentity identity)
{
    return scroll_snapport_rect(scroll_row(document, identity));
}

CSSPixelRect scroll_snapport_rect(DOM::Document const& document, DOM::NodeIdentity identity, CSSPixelRect scrollport)
{
    return scroll_snapport_rect(scroll_row(document, identity), scrollport);
}

ViewportWheelOverflow overflow_values_applied_to_viewport_for_wheel_scrolling(DOM::Document const& document)
{
    auto has_containment = [](CSS::ComputedValues::BoxValues const& style) {
        return style.size_containment || style.inline_size_containment || style.layout_containment || style.style_containment || style.paint_containment;
    };

    auto* root_element = document.document_element();
    auto const* root_style = root_element ? root_element->style_group<CSS::ComputedValues::BoxValues>() : nullptr;
    if (!root_style)
        return {};

    auto const* overflow_origin = root_style;
    if (root_element->is_html_html_element() && !has_containment(*root_style)) {
        auto root_overflow_x = static_cast<CSS::Overflow>(root_style->overflow_x);
        auto root_overflow_y = static_cast<CSS::Overflow>(root_style->overflow_y);
        if (root_overflow_x == CSS::Overflow::Visible && root_overflow_y == CSS::Overflow::Visible) {
            auto* body_element = root_element->first_child_of_type<HTML::HTMLBodyElement>();
            auto const* body_style = body_element ? body_element->style_group<CSS::ComputedValues::BoxValues>() : nullptr;
            if (body_style && !has_containment(*body_style))
                overflow_origin = body_style;
        }
    }

    auto applied = [](CSS::Overflow overflow) {
        if (overflow == CSS::Overflow::Visible)
            return CSS::Overflow::Auto;
        if (overflow == CSS::Overflow::Clip)
            return CSS::Overflow::Hidden;
        return overflow;
    };
    return {
        applied(static_cast<CSS::Overflow>(overflow_origin->overflow_x)),
        applied(static_cast<CSS::Overflow>(overflow_origin->overflow_y)),
    };
}

WheelScrollableAxes wheel_scrollable_axes(BoxSlot const& row)
{
    if (!row)
        return {};
    auto overflow = overflow_values_applied_to_viewport_for_wheel_scrolling(row.document());
    auto axes = Layout::RustFFI::layout_arena_paintable_wheel_scrollable_axes(
        row.arena(), row.slot(), to_underlying(overflow.x), to_underlying(overflow.y));
    return { axes.horizontal, axes.vertical };
}

WheelScrollableAxes wheel_scrollable_axes(DOM::Document const& document, DOM::NodeIdentity identity)
{
    return wheel_scrollable_axes(scroll_row(document, identity));
}

bool could_be_scrolled_by_wheel_event(BoxSlot const& node, ScrollDirection direction)
{
    auto axes = wheel_scrollable_axes(node);
    return direction == ScrollDirection::Horizontal ? axes.horizontal : axes.vertical;
}

bool could_be_scrolled_by_wheel_event(DOM::Document const& document, DOM::NodeIdentity identity, ScrollDirection direction)
{
    auto axes = wheel_scrollable_axes(document, identity);
    return direction == ScrollDirection::Horizontal ? axes.horizontal : axes.vertical;
}

bool could_be_scrolled_by_wheel_event(BoxSlot const& node)
{
    auto axes = wheel_scrollable_axes(node);
    return axes.horizontal || axes.vertical;
}

bool could_be_scrolled_by_wheel_event(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto axes = wheel_scrollable_axes(document, identity);
    return axes.horizontal || axes.vertical;
}

static GC::Ptr<DOM::EventTarget> scroll_event_target(BoxSlot const& node)
{
    if (node.generated_for_pseudo_element().has_value())
        return node.pseudo_element_generator();
    return node.dom_node();
}

ScrollHandled set_scroll_offset(BoxSlot const& node, CSSPixelPoint offset)
{
    if (!has_committed_box(node))
        return ScrollHandled::No;

    if (!Painting::scrollable_overflow_rect(node).has_value())
        return ScrollHandled::No;

    offset = clamp_scroll_offset(node, offset);

    if (scroll_offset(node) == offset)
        return ScrollHandled::No;

    if (node.is_viewport()) {
        auto navigable = node.document().navigable();
        VERIFY(navigable);
        navigable->perform_scroll_of_viewport_scrolling_box(offset);
        return ScrollHandled::Yes;
    }

    if (auto pseudo_element = node.generated_for_pseudo_element(); pseudo_element.has_value()) {
        auto generator = node.pseudo_element_generator();
        if (!generator)
            return ScrollHandled::No;
        generator->set_scroll_offset(*pseudo_element, offset);
    } else if (auto* element = as_if<DOM::Element>(node.dom_node().ptr())) {
        element->set_scroll_offset({}, offset);
    } else {
        return ScrollHandled::No;
    }

    // https://drafts.csswg.org/cssom-view-1/#scrolling-events
    // Whenever an element gets scrolled (whether in response to user interaction or by an API),
    // the user agent must run these steps:

    // 1. Let doc be the element’s node document.
    auto& document = node.document();

    // FIXME: 2. If the element is a snap container, run the steps to update snapchanging targets for the element with
    //           the element’s eventual snap target in the block axis as newBlockTarget and the element’s eventual snap
    //           target in the inline axis as newInlineTarget.

    auto event_target = scroll_event_target(node);
    if (!event_target)
        return ScrollHandled::Yes;

    // 3. If (element, "scroll") is already in doc’s pending scroll events, abort these steps.
    // 4. Append (element, "scroll") to doc’s pending scroll events.
    if (!document.append_pending_scroll_event({ *event_target, HTML::EventNames::scroll }))
        return ScrollHandled::Yes;

    set_needs_repaint(node, InvalidateDisplayList::No);
    return ScrollHandled::Yes;
}

ScrollHandled scroll_by(BoxSlot const& node, double delta_x, double delta_y, ScrollKind scroll_kind)
{
    if (!has_committed_box(node))
        return ScrollHandled::No;
    // A scroll by nothing is not user input: it neither takes over a scroll in flight nor keeps a gesture going.
    if (delta_x == 0 && delta_y == 0)
        return ScrollHandled::No;
    return set_scroll_offset_from_user_input(node, scroll_offset(node).translated(CSSPixels::nearest_value_for(delta_x), CSSPixels::nearest_value_for(delta_y)), scroll_kind);
}

Optional<Compositing::AsyncScrollNodeStableID> async_scroll_node_stable_id(BoxSlot const& node)
{
    if (!node)
        return {};
    if (node.is_viewport()) {
        return Compositing::AsyncScrollNodeStableID {
            .node_id = node.document().unique_id(),
            .kind = Compositing::async_scroll_node_kind_for(Compositing::CompositorScrollNodeKind::Viewport),
            .pseudo_element_type = 0,
        };
    }
    if (auto pseudo_element = node.generated_for_pseudo_element(); pseudo_element.has_value()) {
        auto generator = node.pseudo_element_generator();
        if (!generator)
            return {};
        return Compositing::AsyncScrollNodeStableID {
            .node_id = generator->unique_id(),
            .kind = Compositing::async_scroll_node_kind_for(Compositing::CompositorScrollNodeKind::PseudoElement),
            .pseudo_element_type = static_cast<u8>(to_underlying(*pseudo_element)),
        };
    }
    auto dom_node = node.dom_node();
    if (!dom_node || !is<DOM::Element>(*dom_node))
        return {};
    return Compositing::AsyncScrollNodeStableID {
        .node_id = dom_node->unique_id(),
        .kind = Compositing::async_scroll_node_kind_for(Compositing::CompositorScrollNodeKind::Element),
        .pseudo_element_type = 0,
    };
}

BoxSlot scrolling_box_for_async_scroll_node_stable_id(DOM::Document const& document, Compositing::AsyncScrollNodeStableID const& stable_node_id)
{
    if (stable_node_id.kind == Compositing::AsyncScrollNodeKind::Viewport) {
        if (stable_node_id.node_id != document.unique_id())
            return {};
        return BoxSlot::viewport_of(document);
    }
    auto* element = as_if<DOM::Element>(DOM::Node::from_unique_id(stable_node_id.node_id));
    if (!element || &element->document() != &document)
        return {};
    if (stable_node_id.kind == Compositing::AsyncScrollNodeKind::PseudoElement) {
        if (stable_node_id.pseudo_element_type >= to_underlying(CSS::PseudoElement::KnownPseudoElementCount))
            return {};
        return BoxSlot::of_pseudo_element(*element, static_cast<CSS::PseudoElement>(stable_node_id.pseudo_element_type));
    }
    return BoxSlot::bound_to(*element);
}

ScrollHandled set_scroll_offset_from_user_input(BoxSlot const& node, CSSPixelPoint offset, ScrollKind scroll_kind)
{
    if (!has_committed_box(node))
        return ScrollHandled::No;

    auto navigable = node.document().navigable();
    auto stable_node_id = async_scroll_node_stable_id(node);

    auto scroll_offset_before_scroll = scroll_offset(node);

    if (navigable && stable_node_id.has_value())
        navigable->abort_in_flight_smooth_scrolls_taken_over_by_user_input(*stable_node_id, scroll_offset_before_scroll);

    auto scroll_handled = set_scroll_offset(node, offset);
    if (scroll_handled == ScrollHandled::Yes && scroll_kind == ScrollKind::Relative)
        node.document().scroll_state_query_containers().did_scroll_relatively(node, scroll_offset(node) - scroll_offset_before_scroll);
    if (!navigable)
        return scroll_handled;

    if (scroll_handled == ScrollHandled::Yes) {
        if (auto event_target = scroll_event_target(node))
            navigable->queue_scrollend_event_after_user_scroll(*event_target, stable_node_id, scroll_offset_before_scroll);
    } else {
        // User input keeps the scroll gesture in progress even when it does not move the scrolling box.
        navigable->defer_user_scroll_settlement();
    }
    return scroll_handled;
}

struct ViewportWheelOverflowValues {
    u8 x;
    u8 y;
};

static ViewportWheelOverflowValues viewport_wheel_overflow(DOM::Document const& document)
{
    auto overflow = overflow_values_applied_to_viewport_for_wheel_scrolling(document);
    return { .x = to_underlying(overflow.x), .y = to_underlying(overflow.y) };
}

BoxSlot wheel_scroll_along_containing_block_chain(BoxSlot const& node, double wheel_delta_x, double wheel_delta_y, ScrollKind scroll_kind)
{
    if (!node)
        return {};
    struct WheelScrollableBox {
        Compositing::RustFFI::NodeSlotId slot;
        double accepted_delta_x;
        double accepted_delta_y;
    };
    Vector<WheelScrollableBox, 4> wheel_scrollable_boxes;
    // The chain is walked by the offsets the boxes publish, so a write still in the journal lands
    // first.
    node.document().drain_invalidation_journal();
    auto overflow = viewport_wheel_overflow(node.document());
    Layout::RustFFI::layout_arena_for_each_wheel_scrollable_box_in_containing_block_chain(
        node.arena(), node.slot(), wheel_delta_x, wheel_delta_y, overflow.x, overflow.y,
        &wheel_scrollable_boxes, [](void* context, Compositing::RustFFI::NodeSlotId slot, double accepted_delta_x, double accepted_delta_y) {
            static_cast<Vector<WheelScrollableBox, 4>*>(context)->append({ slot, accepted_delta_x, accepted_delta_y });
        });
    for (auto const& wheel_scrollable_box : wheel_scrollable_boxes) {
        auto box = BoxSlot::of(node.document(), wheel_scrollable_box.slot);
        if (scroll_by(box, wheel_scrollable_box.accepted_delta_x, wheel_scrollable_box.accepted_delta_y, scroll_kind) == ScrollHandled::Yes)
            return box;
    }
    return {};
}

BoxSlot scrolling_box_for_scroll_step_in_containing_block_chain(BoxSlot const& target, CSSPixelPoint delta)
{
    if (!target)
        return {};
    // As for the wheel, the chain is walked by the offsets the boxes publish.
    target.document().drain_invalidation_journal();
    auto overflow = viewport_wheel_overflow(target.document());
    return BoxSlot::of(target.document(), Layout::RustFFI::layout_arena_scrolling_box_for_scroll_step(target.arena(), target.slot(), viewport_row_slot(target.document()), delta, overflow.x, overflow.y));
}

BoxSlot first_wheel_scrollable_box_in_containing_block_chain(BoxSlot const& node)
{
    if (!node)
        return {};
    node.document().drain_invalidation_journal();
    auto overflow = viewport_wheel_overflow(node.document());
    return BoxSlot::of(node.document(), Layout::RustFFI::layout_arena_first_wheel_scrollable_box_in_containing_block_chain(node.arena(), node.slot(), overflow.x, overflow.y));
}

static void scroll_into_view(BoxSlot const& node, CSSPixelRect rect)
{
    if (!has_committed_box(node))
        return;

    auto snapport = scroll_snapport_rect(node);
    auto current_offset = scroll_offset(node);

    // Both rect and snapport are in layout coordinate space (not scroll-adjusted).
    auto content_rect = rect.translated(-snapport.x(), -snapport.y());
    auto new_offset = current_offset;

    if (content_rect.right() > current_offset.x() + snapport.width())
        new_offset.set_x(content_rect.right() - snapport.width());
    else if (content_rect.left() < current_offset.x())
        new_offset.set_x(content_rect.left());

    if (content_rect.bottom() > current_offset.y() + snapport.height())
        new_offset.set_y(content_rect.bottom() - snapport.height());
    else if (content_rect.top() < current_offset.y())
        new_offset.set_y(content_rect.top());

    set_scroll_offset(node, new_offset);
}

void scroll_text_offset_into_view(DOM::Text const& text, size_t offset, TextAffinity affinity, ScrollBlockDirection scroll_block_direction)
{
    auto text_box = BoxSlot::bound_to(text);
    if (!text_box)
        return;
    auto result = Layout::RustFFI::layout_arena_text_caret_rect_for_position(
        text_box.arena(), text_box.slot(), offset,
        affinity == TextAffinity::Downstream);
    if (!result.found)
        return;
    auto style_source = BoxSlot::of(text_box.document(), result.style_source);
    auto const* inherited_box = style_source.style_group<CSS::ComputedValues::InheritedBoxValues>();
    if (!inherited_box)
        return;
    auto writing_mode = static_cast<CSS::WritingMode>(inherited_box->writing_mode);
    auto direction = static_cast<CSS::Direction>(inherited_box->direction);
    bool inline_axis_is_reverse = writing_mode == CSS::WritingMode::SidewaysLr ? direction == CSS::Direction::Ltr : direction == CSS::Direction::Rtl;

    auto cursor_rect = result.rect;
    if (writing_mode == CSS::WritingMode::HorizontalTb) {
        if (inline_axis_is_reverse)
            cursor_rect.set_x(cursor_rect.x() - 1);
        cursor_rect.set_width(1);
    } else {
        if (inline_axis_is_reverse)
            cursor_rect.set_y(cursor_rect.y() - 1);
        cursor_rect.set_height(1);
    }
    for (auto ancestor = committed_box(text_box.document(), result.owner_paintable); ancestor;) {
        if (Painting::has_scrollable_overflow(ancestor)) {
            if (scroll_block_direction == ScrollBlockDirection::No) {
                auto snapport = scroll_snapport_rect(ancestor);
                if (writing_mode == CSS::WritingMode::HorizontalTb) {
                    cursor_rect.set_y(snapport.y() + scroll_offset(ancestor).y());
                    cursor_rect.set_height(snapport.height());
                } else {
                    cursor_rect.set_x(snapport.x() + scroll_offset(ancestor).x());
                    cursor_rect.set_width(snapport.width());
                }
            }
            scroll_into_view(ancestor, cursor_rect);
            return;
        }
        auto containing_block_box = ancestor.containing_block();
        ancestor = containing_block_box && has_committed_box(containing_block_box) ? containing_block_box : BoxSlot {};
    }
}

}
