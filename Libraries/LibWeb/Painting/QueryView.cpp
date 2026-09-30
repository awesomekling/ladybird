/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/QueryView.h>

namespace Web::Painting {

RefPtr<QuerySnapshot const> QuerySnapshot::publish(DOM::Document const& document, QueryVisualContexts visual_contexts)
{
    auto* arena = Layout::document_layout_arena_if_created(document);
    if (!arena)
        return nullptr;
    auto const* handle = Layout::RustFFI::layout_arena_publish_query_snapshot(arena, viewport_of(document, visual_contexts));
    if (!handle)
        return nullptr;
    return adopt_ref(*new QuerySnapshot(handle, visual_contexts));
}

Layout::RustFFI::FfiQuerySnapshotViewport QuerySnapshot::viewport_of(DOM::Document const& document, QueryVisualContexts visual_contexts)
{
    auto navigable = document.navigable();
    // A document that never painted has no scroll state yet, and no visual contexts to convert rects through.
    ReadonlySpan<Gfx::FloatPoint> device_scroll_offsets;
    if (document.has_paint_state())
        device_scroll_offsets = document.paint_state().scroll_state_snapshot().device_offsets();
    return {
        .has_committed_viewport_box = document.has_committed_viewport_box(),
        .visual_contexts_are_up_to_date = visual_contexts == QueryVisualContexts::UpToDate,
        .viewport_scroll_offset_is_zero = !navigable || navigable->viewport_scroll_offset().is_zero(),
        .device_scroll_offsets = device_scroll_offsets.data(),
        .device_scroll_offsets_len = device_scroll_offsets.size(),
        .device_pixels_per_css_pixel = static_cast<float>(document.page().client().device_pixels_per_css_pixel()),
    };
}

RefPtr<QuerySnapshot const> QuerySnapshot::adopt(void const* handle, QueryVisualContexts visual_contexts)
{
    if (!handle)
        return nullptr;
    return adopt_ref(*new QuerySnapshot(handle, visual_contexts));
}

QuerySnapshot::QuerySnapshot(void const* handle, QueryVisualContexts visual_contexts)
    : m_handle(handle)
    , m_visual_contexts(visual_contexts)
{
}

QuerySnapshot::~QuerySnapshot()
{
    Layout::RustFFI::query_snapshot_release(m_handle);
}

static Optional<QueryBox> query_box(Layout::RustFFI::FfiQueryBox ffi)
{
    if (ffi.index == NumericLimits<u32>::max())
        return {};
    return QueryBox { ffi };
}

Optional<QueryBox> QueryView::box_of(DOM::Element const& element) const
{
    auto style_node = element.style_node_id();
    if (!style_node || !element.is_connected())
        return {};
    return query_box(Layout::RustFFI::query_snapshot_element_box(m_snapshot->m_handle, style_node.value()));
}

Optional<QueryBox> QueryView::viewport_box() const
{
    return query_box(Layout::RustFFI::query_snapshot_viewport_box(m_snapshot->m_handle));
}

Optional<QueryBox> QueryView::principal_box_of(DOM::Element const& element) const
{
    auto style_node = element.style_node_id();
    if (!style_node || !element.is_connected())
        return {};
    return query_box(Layout::RustFFI::query_snapshot_principal_box(m_snapshot->m_handle, style_node.value()));
}

QueryBoxFacts QueryView::facts(QueryBox box) const
{
    auto facts = Layout::RustFFI::query_snapshot_box_facts(m_snapshot->m_handle, box.ffi);
    return {
        .has_committed_box = facts.has_committed_box,
        .position = static_cast<CSS::Positioning>(facts.position),
        .is_positioned_for_painting = facts.is_positioned,
        .establishes_an_absolute_positioning_containing_block = facts.establishes_an_absolute_positioning_containing_block,
        .establishes_a_fixed_positioning_containing_block = facts.establishes_a_fixed_positioning_containing_block,
    };
}

bool QueryView::any_ancestor_establishes_a_fixed_position_containing_block(QueryBox box) const
{
    return Layout::RustFFI::query_snapshot_any_ancestor_establishes_a_fixed_position_containing_block(m_snapshot->m_handle, box.ffi);
}

CSSPixelRect QueryView::absolute_border_box_rect(QueryBox box) const
{
    return Layout::RustFFI::query_snapshot_absolute_border_box_rect(m_snapshot->m_handle, box.ffi);
}

CSSPixelRect QueryView::absolute_padding_box_rect(QueryBox box) const
{
    return Layout::RustFFI::query_snapshot_absolute_padding_box_rect(m_snapshot->m_handle, box.ffi);
}

CSSPixelRect QueryView::absolute_rect(QueryBox box) const
{
    return Layout::RustFFI::query_snapshot_absolute_rect(m_snapshot->m_handle, box.ffi);
}

UsedBoxGeometry QueryView::used_box_geometry(QueryBox box) const
{
    auto geometry = Layout::RustFFI::query_snapshot_used_box_geometry(m_snapshot->m_handle, box.ffi);
    auto pixel_box = [](Layout::RustFFI::FfiPixelBox const& box) -> PixelBox { return { box.top, box.right, box.bottom, box.left }; };
    return {
        .content_size = geometry.content_size,
        .absolute_border_box_rect = geometry.absolute_border_box_rect,
        .box_model = {
            .margin = pixel_box(geometry.box_model.margin),
            .padding = pixel_box(geometry.box_model.padding),
            .border = pixel_box(geometry.box_model.border),
            .inset = pixel_box(geometry.box_model.inset),
        },
    };
}

Optional<CSSPixelPoint> QueryView::mouse_event_offset(QueryBox box, CSSPixelPoint position) const
{
    CSSPixelPoint offset;
    if (!Layout::RustFFI::query_snapshot_mouse_event_offset(m_snapshot->m_handle, box.ffi, position, &offset))
        return {};
    return offset;
}

Optional<Vector<CSSPixelRect>> QueryView::client_rects(QueryBox box) const
{
    Vector<CSSPixelRect> rects;
    auto converted = Layout::RustFFI::query_snapshot_client_rects(m_snapshot->m_handle, box.ffi, &rects, [](void* context, CSSPixelRect rect) {
        static_cast<Vector<CSSPixelRect>*>(context)->append(rect);
    });
    if (!converted)
        return {};
    return rects;
}

Optional<Utf16String> QueryView::rendered_text(Compositing::RustFFI::NodeSlotId slot, bool collapse_whitespace) const
{
    Utf16String text;
    auto has_text = Layout::RustFFI::query_snapshot_rendered_text(m_snapshot->m_handle, slot, collapse_whitespace, &text,
        [](void* context, Layout::RustFFI::FfiRenderedTextView view) {
            *static_cast<Utf16String*>(context) = Utf16String::from_utf16({ reinterpret_cast<char16_t const*>(view.text), view.length_in_code_units });
        });
    if (!has_text)
        return {};
    return text;
}

Optional<CSSPixelRect> QueryView::bounding_client_rect(QueryBox box) const
{
    CSSPixelRect rect;
    if (!Layout::RustFFI::query_snapshot_bounding_client_rect(m_snapshot->m_handle, box.ffi, &rect))
        return {};
    return rect;
}

}
