/*
 * Copyright (c) 2026, Jelle Raaijmakers <jelle@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/DocumentFragment.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Page/AutoScrollHandler.h>
#include <LibWeb/Page/EventHandler.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/Scrolling.h>

namespace Web {

static constexpr int auto_scroll_interval_ms = 16;
static constexpr CSSPixels auto_scroll_edge_threshold { 7 };
static constexpr CSSPixels viewport_edge_inset { 25 };

// Returns the scrollport shrunk by per-side effective auto scroll edge thresholds. Sides close to a viewport edge get
// a larger inset so the distance-based speed ramp works predictably, even if the user's mouse is limited in reach (e.g.
// by the window/screen boundary).
static CSSPixelRect compute_effective_auto_scroll_edge(CSSPixelRect const& scrollport, CSSPixelRect const& viewport_rect)
{
    auto effective = [](CSSPixels distance_to_viewport_edge) {
        return auto_scroll_edge_threshold + max(CSSPixels(0), viewport_edge_inset - distance_to_viewport_edge);
    };
    return scrollport.shrunken(
        effective(scrollport.top() - viewport_rect.top()),
        effective(viewport_rect.right() - scrollport.right()),
        effective(viewport_rect.bottom() - scrollport.bottom()),
        effective(scrollport.left() - viewport_rect.left()));
}

static Optional<CSSPixelRect> scrollport_rect_in_viewport(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto scrollport = Painting::absolute_padding_box_rect(document, identity);

    // The viewport's scrollport is already in viewport coordinates.
    if (Painting::is_viewport_paintable(document, identity))
        return scrollport;

    return Painting::transform_rect_to_viewport(document, identity, scrollport);
}

// Returns scroll speed in CSS pixels per second for each axis, based on how far the mouse is past the auto scroll edge.
static CSSPixelPoint compute_auto_scroll_speed(CSSPixelPoint mouse, CSSPixelRect const& edge)
{
    static CSSPixels constexpr distance_to_speed_factor { 100 };
    static CSSPixels constexpr max_speed_per_second { 5000 };

    auto compute_axis_speed = [&](CSSPixels mouse_pos, CSSPixels edge_start, CSSPixels edge_end) -> CSSPixels {
        if (mouse_pos < edge_start) {
            auto distance = edge_start - mouse_pos;
            return -min(distance * distance_to_speed_factor, max_speed_per_second);
        }
        if (mouse_pos > edge_end) {
            auto distance = mouse_pos - edge_end;
            return min(distance * distance_to_speed_factor, max_speed_per_second);
        }
        return 0;
    };

    return {
        compute_axis_speed(mouse.x(), edge.x(), edge.x() + edge.width()),
        compute_axis_speed(mouse.y(), edge.y(), edge.y() + edge.height()),
    };
}

AutoScrollHandler::AutoScrollHandler(HTML::LocalNavigable& navigable, DOM::Element& container)
    : m_navigable(navigable)
    , m_container_element(container)
{
}

AutoScrollHandler::~AutoScrollHandler() = default;

void AutoScrollHandler::visit_edges(JS::Cell::Visitor& visitor) const
{
    visitor.visit(m_navigable);
    visitor.visit(m_container_element);
}

CSSPixelPoint AutoScrollHandler::process(CSSPixelPoint mouse_position)
{
    m_mouse_position = mouse_position;

    m_container_element->document().update_layout(DOM::UpdateLayoutReason::AutoScrollSelection);

    auto box = auto_scroll_box(m_container_element);
    if (!box)
        return mouse_position;

    auto scrollport = scrollport_rect_in_viewport(m_container_element->document(), box.dom_node_identity());
    if (!scrollport.has_value())
        return mouse_position;

    CSSPixelRect viewport_rect { {}, m_navigable->viewport_size() };
    auto effective_edge = compute_effective_auto_scroll_edge(*scrollport, viewport_rect);
    if (effective_edge.contains(mouse_position)) {
        deactivate();
        return mouse_position;
    }

    activate();
    return constrained(mouse_position, *scrollport);
}

GC::Ptr<DOM::Element> AutoScrollHandler::find_scrollable_ancestor(Painting::BoxSlot const& box)
{
    auto scrollable_box = Painting::first_wheel_scrollable_box_in_containing_block_chain(box);
    if (!scrollable_box)
        return {};

    // The viewport is always a potential scroll container, but may not report has_scrollable_overflow() and its DOM
    // node is Document (not Element).
    if (scrollable_box.is_viewport())
        return const_cast<DOM::Element*>(scrollable_box.document().scrolling_element().ptr());

    return as_if<DOM::Element>(scrollable_box.dom_node().ptr());
}

// Returns the box that manages the scrollport for an auto-scroll container element. When the element is the
// document's scrolling element, the viewport box is the scroll container.
Painting::BoxSlot AutoScrollHandler::auto_scroll_box(DOM::Element& element)
{
    auto box = Painting::BoxSlot::bound_to(element);
    if (element.document().scrolling_element().ptr() == &element)
        box = Painting::BoxSlot::viewport_of(element.document());
    return box && Painting::has_committed_box(box) ? box : Painting::BoxSlot {};
}

void AutoScrollHandler::activate()
{
    m_active = true;

    // Moving the mouse back inside the scrollport pauses the scrolling without ending the selection drag it belongs
    // to, so the hold outlives deactivation and is released when the handler is torn down.
    if (!m_scroll_gesture_hold)
        m_scroll_gesture_hold = make<HTML::UserScrollGestureHold>(m_navigable);
}

void AutoScrollHandler::deactivate()
{
    m_active = false;
    m_fractional_delta = {};
}

void AutoScrollHandler::perform_tick()
{
    if (!m_active)
        return;

    if (!m_navigable->event_handler().is_handling_mouse_selection()) {
        deactivate();
        return;
    }

    auto& document = *m_navigable->active_document();
    document.update_layout(DOM::UpdateLayoutReason::AutoScrollSelection);

    auto box = auto_scroll_box(m_container_element);
    if (!box || !document.has_committed_viewport_box()) {
        deactivate();
        return;
    }

    auto scrollport = scrollport_rect_in_viewport(m_container_element->document(), box.dom_node_identity());
    if (!scrollport.has_value()) {
        deactivate();
        return;
    }

    CSSPixelRect viewport_rect { {}, m_navigable->viewport_size() };
    auto effective_edge = compute_effective_auto_scroll_edge(*scrollport, viewport_rect);
    if (effective_edge.contains(m_mouse_position)) {
        deactivate();
        return;
    }

    auto elapsed_seconds = static_cast<double>(auto_scroll_interval_ms) / 1000.0;
    auto speed = compute_auto_scroll_speed(m_mouse_position, effective_edge);

    // Accumulate sub-pixel deltas across ticks, since scroll_by() only accepts whole pixels.
    m_fractional_delta += CSSPixelPoint {
        speed.x() * CSSPixels(elapsed_seconds),
        speed.y() * CSSPixels(elapsed_seconds),
    };
    int scroll_x = m_fractional_delta.x().to_int();
    int scroll_y = m_fractional_delta.y().to_int();
    m_fractional_delta -= CSSPixelPoint { scroll_x, scroll_y };

    m_navigable->note_user_scroll_input_intent(Compositing::SnapSelectionStrategy::Type::EndPosition);
    if (Painting::scroll_by(box, scroll_x, scroll_y) == Painting::ScrollHandled::No)
        return;

    m_navigable->event_handler().apply_mouse_selection(constrained(m_mouse_position, *scrollport));
}

}
