/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/StdLibExtras.h>
#include <AK/Types.h>
#include <LibGfx/DecodedImageFrame.h>

extern "C" {
void const* ladybird_gfx_image_frame_handle_create(void const*);
void const* ladybird_gfx_image_frame_handle_ref(void const*);
void ladybird_gfx_image_frame_handle_unref(void const*);
u64 ladybird_gfx_image_frame_handle_id(void const*);
}

namespace Gfx {

// An owned reference to an immutable decoded-frame record in the shared Rust
// resource service. The numeric id is stable and safe to carry across a
// main-to-render journal entry.
class ImageFrameHandle {
public:
    ImageFrameHandle() = default;

    explicit ImageFrameHandle(DecodedImageFrame const& frame)
        : m_handle(ladybird_gfx_image_frame_handle_create(&frame))
    {
    }

    ImageFrameHandle(ImageFrameHandle const& other)
        : m_handle(ladybird_gfx_image_frame_handle_ref(other.m_handle))
    {
    }

    ImageFrameHandle& operator=(ImageFrameHandle const& other)
    {
        ImageFrameHandle copy(other);
        swap(m_handle, copy.m_handle);
        return *this;
    }

    ImageFrameHandle(ImageFrameHandle&& other)
        : m_handle(exchange(other.m_handle, nullptr))
    {
    }

    ImageFrameHandle& operator=(ImageFrameHandle&& other)
    {
        ImageFrameHandle moved(move(other));
        swap(m_handle, moved.m_handle);
        return *this;
    }

    ~ImageFrameHandle()
    {
        ladybird_gfx_image_frame_handle_unref(m_handle);
    }

    explicit operator bool() const { return m_handle; }
    u64 id() const { return ladybird_gfx_image_frame_handle_id(m_handle); }

private:
    void const* m_handle { nullptr };
};

}
