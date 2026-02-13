/*
 * Copyright (c) 2026, Andreas Kling <andreas@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGC/Heap.h>
#include <LibGfx/Bitmap.h>
#include <LibGfx/ImmutableBitmap.h>
#include <LibJS/Runtime/Realm.h>
#include <LibWeb/HTML/StreamingAnimatedBitmapDecodedImageData.h>
#include <LibWeb/Painting/DisplayListRecorder.h>
#include <LibWeb/Painting/DisplayListRecordingContext.h>
#include <LibWeb/Platform/ImageCodecPlugin.h>

namespace Web::HTML {

GC_DEFINE_ALLOCATOR(StreamingAnimatedBitmapDecodedImageData);

GC::Ref<StreamingAnimatedBitmapDecodedImageData> StreamingAnimatedBitmapDecodedImageData::create(
    JS::Realm& realm,
    i64 session_id,
    u32 frame_count,
    u32 loop_count,
    Gfx::IntSize size,
    Gfx::ColorSpace color_space,
    Vector<u32> durations,
    Vector<NonnullRefPtr<Gfx::Bitmap>> initial_bitmaps)
{
    auto data = realm.create<StreamingAnimatedBitmapDecodedImageData>(
        session_id, frame_count, loop_count, size, move(color_space), move(durations));

    // Place initial bitmaps into the buffer pool.
    for (u32 i = 0; i < initial_bitmaps.size(); ++i) {
        auto& slot = data->m_buffer_slots[i % BUFFER_POOL_SIZE];
        slot.frame_index = i;
        slot.bitmap = Gfx::ImmutableBitmap::create(*initial_bitmaps[i], data->m_color_space);
        slot.generation = ++data->m_write_generation;
    }

    data->m_highest_requested_frame = initial_bitmaps.size();

    if (!initial_bitmaps.is_empty())
        data->m_last_displayed_bitmap = data->m_buffer_slots[0].bitmap;

    return data;
}

StreamingAnimatedBitmapDecodedImageData::StreamingAnimatedBitmapDecodedImageData(
    i64 session_id,
    u32 frame_count,
    u32 loop_count,
    Gfx::IntSize size,
    Gfx::ColorSpace color_space,
    Vector<u32> durations)
    : m_session_id(session_id)
    , m_frame_count(frame_count)
    , m_loop_count(loop_count)
    , m_size(size)
    , m_color_space(move(color_space))
    , m_durations(move(durations))
{
}

StreamingAnimatedBitmapDecodedImageData::~StreamingAnimatedBitmapDecodedImageData() = default;

void StreamingAnimatedBitmapDecodedImageData::finalize()
{
    Base::finalize();
    Platform::ImageCodecPlugin::the().stop_animation_decode(m_session_id);
}

StreamingAnimatedBitmapDecodedImageData::BufferSlot const* StreamingAnimatedBitmapDecodedImageData::find_slot(u32 frame_index) const
{
    for (auto const& slot : m_buffer_slots) {
        if (slot.frame_index == frame_index && slot.bitmap)
            return &slot;
    }
    return nullptr;
}

StreamingAnimatedBitmapDecodedImageData::BufferSlot& StreamingAnimatedBitmapDecodedImageData::evict_oldest_slot()
{
    BufferSlot* oldest = &m_buffer_slots[0];
    for (auto& slot : m_buffer_slots) {
        if (slot.generation < oldest->generation)
            oldest = &slot;
    }
    return *oldest;
}

RefPtr<Gfx::ImmutableBitmap> StreamingAnimatedBitmapDecodedImageData::bitmap(size_t frame_index, Gfx::IntSize) const
{
    if (frame_index >= m_frame_count)
        return m_last_displayed_bitmap;

    if (auto const* slot = find_slot(frame_index)) {
        m_last_displayed_bitmap = slot->bitmap;
        return slot->bitmap;
    }

    // Frame not in pool; return last displayed frame as fallback.
    return m_last_displayed_bitmap;
}

int StreamingAnimatedBitmapDecodedImageData::frame_duration(size_t frame_index) const
{
    if (frame_index >= m_durations.size())
        return 0;
    return m_durations[frame_index];
}

Optional<CSSPixels> StreamingAnimatedBitmapDecodedImageData::intrinsic_width() const
{
    return m_size.width();
}

Optional<CSSPixels> StreamingAnimatedBitmapDecodedImageData::intrinsic_height() const
{
    return m_size.height();
}

Optional<CSSPixelFraction> StreamingAnimatedBitmapDecodedImageData::intrinsic_aspect_ratio() const
{
    return CSSPixels(m_size.width()) / CSSPixels(m_size.height());
}

Optional<Gfx::IntRect> StreamingAnimatedBitmapDecodedImageData::frame_rect(size_t) const
{
    return Gfx::IntRect { {}, m_size };
}

void StreamingAnimatedBitmapDecodedImageData::paint(DisplayListRecordingContext& context, size_t frame_index, Gfx::IntRect dst_rect, Gfx::IntRect clip_rect, Gfx::ScalingMode scaling_mode) const
{
    auto immutable_bitmap = bitmap(frame_index);
    if (!immutable_bitmap)
        return;
    context.display_list_recorder().draw_scaled_immutable_bitmap(dst_rect, clip_rect, *immutable_bitmap, scaling_mode);
}

void StreamingAnimatedBitmapDecodedImageData::receive_frames(Vector<NonnullRefPtr<Gfx::Bitmap>> bitmaps, u32 start_frame_index)
{
    m_request_in_flight = false;

    for (u32 i = 0; i < bitmaps.size(); ++i) {
        u32 frame_index = start_frame_index + i;
        if (frame_index >= m_frame_count)
            break;

        // Check if this frame is already in the pool.
        if (find_slot(frame_index))
            continue;

        auto& slot = evict_oldest_slot();
        slot.frame_index = frame_index;
        slot.bitmap = Gfx::ImmutableBitmap::create(*bitmaps[i], m_color_space);
        slot.generation = ++m_write_generation;
    }
}

void StreamingAnimatedBitmapDecodedImageData::notify_frame_advanced(size_t current_frame_index)
{
    maybe_request_more_frames(current_frame_index);
}

void StreamingAnimatedBitmapDecodedImageData::maybe_request_more_frames(size_t current_frame_index)
{
    if (m_request_in_flight)
        return;

    // Count how many frames ahead of current are in the pool.
    u32 frames_ahead = 0;
    for (u32 offset = 1; offset <= BUFFER_POOL_SIZE; ++offset) {
        u32 future_index = (current_frame_index + offset) % m_frame_count;
        if (find_slot(future_index))
            ++frames_ahead;
        else
            break;
    }

    // Request more if fewer than 2 frames buffered ahead.
    if (frames_ahead >= 2)
        return;

    // Determine which frame to request from.
    u32 request_start = (current_frame_index + frames_ahead + 1) % m_frame_count;
    u32 request_count = BUFFER_POOL_SIZE;

    m_request_in_flight = true;
    m_highest_requested_frame = max(m_highest_requested_frame, request_start + request_count);
    Platform::ImageCodecPlugin::the().request_animation_frames(m_session_id, request_start, request_count);
}

}
