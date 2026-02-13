/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGfx/ImmutableBitmap.h>
#include <LibWeb/CSS/CSSImageResource.h>
#include <LibWeb/CSS/Fetch.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/HTML/DecodedImageData.h>
#include <LibWeb/HTML/SharedResourceRequest.h>
#include <LibWeb/Painting/DisplayListRecorder.h>
#include <LibWeb/Painting/DisplayListRecordingContext.h>
#include <LibWeb/Platform/Timer.h>

namespace Web::CSS {

GC_DEFINE_ALLOCATOR(CSSImageResource);

GC::Ref<CSSImageResource> CSSImageResource::create(GC::Heap& heap, URL const& url, DOM::Document& document)
{
    return heap.allocate<CSSImageResource>(url, document);
}

CSSImageResource::CSSImageResource(URL const& url, DOM::Document& document)
    : m_document(document)
    , m_url(url)
{
    RuleOrDeclaration rule_or_declaration {
        .environment_settings_object = document.relevant_settings_object(),
        .value = RuleOrDeclaration::Rule {
            .parent_style_sheet = nullptr,
        }
    };

    m_resource_request = fetch_an_external_image_for_a_stylesheet(m_url, rule_or_declaration, document);

    if (m_resource_request) {
        m_resource_request->add_callbacks(
            [this] {
                auto image_data = m_resource_request->image_data();
                if (image_data->is_animated() && image_data->frame_count() > 1) {
                    m_timer = Platform::Timer::create(m_document->heap());
                    m_timer->set_interval(image_data->frame_duration(0));
                    m_timer->on_timeout = GC::create_function(m_document->heap(), [this] { animate(); });
                    m_timer->start();
                }
            },
            nullptr);
    }
}

void CSSImageResource::visit_edges(Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_document);
    visitor.visit(m_resource_request);
    visitor.visit(m_timer);
}

GC::Ptr<HTML::DecodedImageData> CSSImageResource::image_data() const
{
    if (!m_resource_request)
        return nullptr;
    return m_resource_request->image_data();
}

bool CSSImageResource::is_paintable() const
{
    return image_data();
}

Optional<CSSPixels> CSSImageResource::natural_width() const
{
    if (auto data = image_data())
        return data->intrinsic_width();
    return {};
}

Optional<CSSPixels> CSSImageResource::natural_height() const
{
    if (auto data = image_data())
        return data->intrinsic_height();
    return {};
}

Optional<CSSPixelFraction> CSSImageResource::natural_aspect_ratio() const
{
    if (auto data = image_data())
        return data->intrinsic_aspect_ratio();
    return {};
}

Gfx::ImmutableBitmap const* CSSImageResource::bitmap(size_t frame_index, Gfx::IntSize size) const
{
    if (auto data = image_data())
        return data->bitmap(frame_index, size);
    return nullptr;
}

void CSSImageResource::paint(DisplayListRecordingContext& context, DevicePixelRect const& dest_rect, ImageRendering image_rendering) const
{
    auto data = image_data();
    if (!data)
        return;

    auto dest_int_rect = dest_rect.to_type<int>();
    auto rect = data->frame_rect(m_current_frame_index).value_or(dest_int_rect);
    auto scaling_mode = to_gfx_scaling_mode(image_rendering, rect.size(), dest_int_rect.size());
    data->paint(context, m_current_frame_index, dest_int_rect, dest_int_rect, scaling_mode);
}

Gfx::ImmutableBitmap const* CSSImageResource::current_frame_bitmap(DevicePixelRect const& dest_rect) const
{
    return bitmap(m_current_frame_index, dest_rect.size().to_type<int>());
}

Optional<Gfx::Color> CSSImageResource::color_if_single_pixel_bitmap() const
{
    if (auto const* b = bitmap(m_current_frame_index)) {
        if (b->width() == 1 && b->height() == 1)
            return b->get_pixel(0, 0);
    }
    return {};
}

void CSSImageResource::animate()
{
    if (!m_resource_request)
        return;
    auto data = image_data();
    if (!data)
        return;

    m_current_frame_index = (m_current_frame_index + 1) % data->frame_count();
    m_current_frame_index = data->notify_frame_advanced(m_current_frame_index);
    auto current_frame_duration = data->frame_duration(m_current_frame_index);

    if (current_frame_duration != m_timer->interval())
        m_timer->restart(current_frame_duration);

    if (m_current_frame_index == data->frame_count() - 1) {
        ++m_loops_completed;
        if (m_loops_completed > 0 && m_loops_completed == data->loop_count())
            m_timer->stop();
    }

    if (on_animate)
        on_animate();
}

}
