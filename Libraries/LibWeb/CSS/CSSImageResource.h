/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/HashTable.h>
#include <LibGC/Cell.h>
#include <LibGC/CellAllocator.h>
#include <LibGC/Ptr.h>
#include <LibGfx/Forward.h>
#include <LibURL/URL.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/URL.h>
#include <LibWeb/Forward.h>
#include <LibWeb/PixelUnits.h>

namespace Web::CSS {

class CSSImageResource final : public GC::Cell {
    GC_CELL(CSSImageResource, GC::Cell);
    GC_DECLARE_ALLOCATOR(CSSImageResource);

public:
    static GC::Ref<CSSImageResource> create(GC::Heap&, URL const&, DOM::Document&);

    GC::Ptr<HTML::DecodedImageData> image_data() const;
    bool is_paintable() const;

    Optional<CSSPixels> natural_width() const;
    Optional<CSSPixels> natural_height() const;
    Optional<CSSPixelFraction> natural_aspect_ratio() const;

    void paint(DisplayListRecordingContext&, DevicePixelRect const& dest_rect, ImageRendering) const;
    Gfx::ImmutableBitmap const* current_frame_bitmap(DevicePixelRect const& dest_rect) const;
    Optional<Gfx::Color> color_if_single_pixel_bitmap() const;

    size_t current_frame_index() const { return m_current_frame_index; }

    Function<void()> on_load;
    Function<void()> on_animate;

private:
    CSSImageResource(URL const&, DOM::Document&);

    virtual void visit_edges(Visitor&) override;

    void animate();
    Gfx::ImmutableBitmap const* bitmap(size_t frame_index, Gfx::IntSize = {}) const;

    GC::Ref<DOM::Document> m_document;
    GC::Ptr<HTML::SharedResourceRequest> m_resource_request;
    GC::Ptr<Platform::Timer> m_timer;

    URL m_url;

    size_t m_current_frame_index { 0 };
    size_t m_loops_completed { 0 };
};

}
