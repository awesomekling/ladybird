/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Array.h>
#include <LibGfx/ColorSpace.h>
#include <LibGfx/Forward.h>
#include <LibWeb/HTML/DecodedImageData.h>

namespace Web::HTML {

class StreamingAnimatedBitmapDecodedImageData final : public DecodedImageData {
    GC_CELL(StreamingAnimatedBitmapDecodedImageData, DecodedImageData);
    GC_DECLARE_ALLOCATOR(StreamingAnimatedBitmapDecodedImageData);

public:
    static GC::Ref<StreamingAnimatedBitmapDecodedImageData> create(
        JS::Realm&,
        i64 session_id,
        u32 frame_count,
        u32 loop_count,
        Gfx::IntSize,
        Gfx::ColorSpace,
        Vector<u32> durations,
        Vector<NonnullRefPtr<Gfx::Bitmap>> initial_bitmaps);

    virtual ~StreamingAnimatedBitmapDecodedImageData() override;
    virtual void finalize() override;

    virtual RefPtr<Gfx::ImmutableBitmap> bitmap(size_t frame_index, Gfx::IntSize = {}) const override;
    virtual int frame_duration(size_t frame_index) const override;

    virtual size_t frame_count() const override { return m_frame_count; }
    virtual size_t loop_count() const override { return m_loop_count; }
    virtual bool is_animated() const override { return true; }

    virtual Optional<CSSPixels> intrinsic_width() const override;
    virtual Optional<CSSPixels> intrinsic_height() const override;
    virtual Optional<CSSPixelFraction> intrinsic_aspect_ratio() const override;

    virtual Optional<Gfx::IntRect> frame_rect(size_t frame_index) const override;
    virtual void paint(DisplayListRecordingContext&, size_t frame_index, Gfx::IntRect dst_rect, Gfx::IntRect clip_rect, Gfx::ScalingMode) const override;

    virtual void notify_frame_advanced(size_t current_frame_index) override;

    void receive_frames(Vector<NonnullRefPtr<Gfx::Bitmap>>, u32 start_frame_index);

    i64 session_id() const { return m_session_id; }

private:
    static constexpr u32 BUFFER_POOL_SIZE = 4;

    struct BufferSlot {
        Optional<u32> frame_index;
        RefPtr<Gfx::ImmutableBitmap> bitmap;
        u64 generation { 0 };
    };

    StreamingAnimatedBitmapDecodedImageData(
        i64 session_id,
        u32 frame_count,
        u32 loop_count,
        Gfx::IntSize,
        Gfx::ColorSpace,
        Vector<u32> durations);

    BufferSlot const* find_slot(u32 frame_index) const;
    BufferSlot& evict_oldest_slot();
    void maybe_request_more_frames(size_t current_frame_index);

    i64 m_session_id;
    u32 m_frame_count;
    u32 m_loop_count;
    Gfx::IntSize m_size;
    Gfx::ColorSpace m_color_space;
    Vector<u32> m_durations;

    Array<BufferSlot, BUFFER_POOL_SIZE> m_buffer_slots;
    mutable RefPtr<Gfx::ImmutableBitmap> m_last_displayed_bitmap;
    u64 m_write_generation { 0 };
    bool m_request_in_flight { false };
    u32 m_highest_requested_frame { 0 };
};

}
