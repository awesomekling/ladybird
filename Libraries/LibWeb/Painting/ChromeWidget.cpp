/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/GenericShorthands.h>
#include <LibCompositing/Scrolling/ScrollState.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/ChromeMetrics.h>
#include <LibWeb/Painting/ChromeWidget.h>
#include <LibWeb/Painting/ResizeHandle.h>
#include <LibWeb/Painting/Scrollbar.h>

namespace Web::Painting {

ChromeWidgetRegistry::ChromeWidgetRegistry() = default;

ChromeWidgetRegistry::~ChromeWidgetRegistry()
{
    clear();
}

RefPtr<Scrollbar> ChromeWidgetRegistry::scrollbar(Compositing::RustFFI::NodeSlotId slot, ScrollDirection direction) const
{
    auto entry = m_entries.find(slot.index);
    if (entry == m_entries.end())
        return nullptr;
    auto scrollbar = direction == ScrollDirection::Horizontal ? entry->value.horizontal_scrollbar : entry->value.vertical_scrollbar;
    return scrollbar && scrollbar->is_current() ? scrollbar : nullptr;
}

NonnullRefPtr<Scrollbar> ChromeWidgetRegistry::get_or_create_scrollbar(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot, ScrollDirection direction)
{
    auto& entry = m_entries.ensure(slot.index);
    auto& scrollbar = direction == ScrollDirection::Horizontal ? entry.horizontal_scrollbar : entry.vertical_scrollbar;
    if (scrollbar && !scrollbar->is_current()) {
        scrollbar->detach({});
        scrollbar = nullptr;
    }
    if (!scrollbar)
        scrollbar = Scrollbar::create(document, slot, direction);
    return *scrollbar;
}

RefPtr<ResizeHandle> ChromeWidgetRegistry::resize_handle(Compositing::RustFFI::NodeSlotId slot) const
{
    auto entry = m_entries.find(slot.index);
    if (entry == m_entries.end())
        return nullptr;
    return entry->value.resize_handle && entry->value.resize_handle->is_current() ? entry->value.resize_handle : nullptr;
}

NonnullRefPtr<ResizeHandle> ChromeWidgetRegistry::get_or_create_resize_handle(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot)
{
    auto& entry = m_entries.ensure(slot.index);
    if (entry.resize_handle && !entry.resize_handle->is_current()) {
        entry.resize_handle->detach({});
        entry.resize_handle = nullptr;
    }
    if (!entry.resize_handle)
        entry.resize_handle = ResizeHandle::create(document, slot);
    return *entry.resize_handle;
}

void ChromeWidgetRegistry::drop_widgets_of_reset_rows()
{
    // A reset moves the row's reset version, so a row's widgets are the ones to drop when any of them is not current.
    m_entries.remove_all_matching([](u32, Entry& entry) {
        auto is_stale = [](RefPtr<ChromeWidget> const& widget) { return widget && !widget->is_current(); };
        if (!is_stale(entry.horizontal_scrollbar) && !is_stale(entry.vertical_scrollbar) && !is_stale(entry.resize_handle))
            return false;
        if (entry.horizontal_scrollbar)
            entry.horizontal_scrollbar->detach({});
        if (entry.vertical_scrollbar)
            entry.vertical_scrollbar->detach({});
        if (entry.resize_handle)
            entry.resize_handle->detach({});
        return true;
    });
}

void ChromeWidgetRegistry::clear()
{
    for (auto& entry : m_entries) {
        if (entry.value.horizontal_scrollbar)
            entry.value.horizontal_scrollbar->detach({});
        if (entry.value.vertical_scrollbar)
            entry.value.vertical_scrollbar->detach({});
        if (entry.value.resize_handle)
            entry.value.resize_handle->detach({});
    }
    m_entries.clear();
}

ChromeWidget::ChromeWidget(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot)
    : m_document(document)
    , m_slot(slot)
    , m_row_reset_version(committed_row_reset_version(document, slot))
{
}

BoxSlot ChromeWidget::box() const
{
    if (!is_current())
        return {};
    return committed_box(*m_document, m_slot);
}

void* ChromeWidget::arena() const
{
    return m_document ? Layout::document_layout_arena_if_created(*m_document) : nullptr;
}

bool ChromeWidget::is_current() const
{
    if (m_slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX || !m_document)
        return false;
    auto current_version = committed_row_reset_version(*m_document, m_slot);
    if (current_version == m_row_reset_version)
        return true;
    return false;
}

void ChromeWidget::detach(Badge<ChromeWidgetRegistry>)
{
    did_detach();
    m_slot = Compositing::RustFFI::NodeSlotId_INVALID;
}

Optional<ScrollbarData> compute_scrollbar_data(BoxSlot const& node, ScrollDirection direction, ChromeMetrics const& metrics, Compositing::ScrollStateSnapshot const* scroll_state_snapshot, ScrollbarSizing scrollbar_sizing)
{
    if (!node)
        return {};
    auto& document = node.document();
    auto viewport_overflow = overflow_values_applied_to_viewport_for_wheel_scrolling(document);
    auto overflow_x = viewport_overflow.x;
    auto overflow_y = viewport_overflow.y;
    float device_scroll_offset = 0;
    if (scroll_state_snapshot) {
        auto own_offset = scroll_state_snapshot->device_offset_for_index(own_scroll_node_index(node));
        device_scroll_offset = direction == ScrollDirection::Horizontal ? -own_offset.x() : -own_offset.y();
    }
    auto result = Layout::RustFFI::layout_arena_paintable_compute_scrollbar_data(
        node.arena(), node.slot(), static_cast<Layout::RustFFI::ScrollDirection>(direction),
        metrics, to_underlying(overflow_x), to_underlying(overflow_y), scrollbar_sizing == ScrollbarSizing::Enlarged,
        scroll_state_snapshot, device_scroll_offset, document.page().client().device_pixels_per_css_pixel());
    if (!result.has_value)
        return {};
    return ScrollbarData {
        .gutter_rect = result.value.gutter_rect,
        .thumb_rect = result.value.thumb_rect,
        .track_rect = result.value.track_rect,
        .thumb_travel_to_scroll_ratio = result.value.thumb_travel_to_scroll_ratio,
    };
}

}
