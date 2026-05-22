/*
 * Copyright (c) 2022-2026, Gregory Bertilson <gregory@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGfx/YUVData.h>
#include <LibIPC/Decoder.h>
#include <LibIPC/Encoder.h>

#include "VideoFrame.h"

#include <LibCore/AnonymousBuffer.h>
#include <LibMedia/Color/CodingIndependentCodePoints.h>

namespace Media {

VideoFrame::VideoFrame(
    AK::Duration timestamp,
    AK::Duration duration,
    Gfx::Size<u32> size,
    u8 bit_depth,
    Gfx::ColorSpace color_space,
    NonnullOwnPtr<Gfx::YUVData> yuv_data)
    : m_timestamp(timestamp)
    , m_duration(duration)
    , m_size(size)
    , m_bit_depth(bit_depth)
    , m_color_space(move(color_space))
    , m_yuv_data(move(yuv_data))
{
}

VideoFrame::~VideoFrame() = default;

}

namespace IPC {

static bool color_primaries_ipc_value_valid(Media::ColorPrimaries color_primaries)
{
    return color_primaries == Media::ColorPrimaries::Unspecified
        || Media::color_primaries_valid(color_primaries);
}

static bool transfer_characteristics_ipc_value_valid(Media::TransferCharacteristics transfer_characteristics)
{
    return transfer_characteristics == Media::TransferCharacteristics::Unspecified
        || Media::transfer_characteristics_valid(transfer_characteristics);
}

static bool matrix_coefficients_ipc_value_valid(Media::MatrixCoefficients matrix_coefficients)
{
    return matrix_coefficients == Media::MatrixCoefficients::Unspecified
        || Media::matrix_coefficients_valid(matrix_coefficients);
}

static bool video_full_range_flag_ipc_value_valid(Media::VideoFullRangeFlag video_full_range_flag)
{
    return video_full_range_flag == Media::VideoFullRangeFlag::Unspecified
        || Media::video_full_range_flag_valid(video_full_range_flag);
}

template<>
ErrorOr<void> encode(Encoder& encoder, Media::VideoFrame const& frame)
{
    auto const& yuv_data = frame.yuv_data();
    TRY(encoder.encode(yuv_data.anonymous_buffer()));
    TRY(encoder.encode(frame.color_space()));
    TRY(encoder.encode(frame.timestamp()));
    TRY(encoder.encode(frame.duration()));
    TRY(encoder.encode(yuv_data.size()));
    TRY(encoder.encode(yuv_data.bit_depth()));
    TRY(encoder.encode(yuv_data.subsampling().x()));
    TRY(encoder.encode(yuv_data.subsampling().y()));
    TRY(encoder.encode(yuv_data.cicp().color_primaries()));
    TRY(encoder.encode(yuv_data.cicp().transfer_characteristics()));
    TRY(encoder.encode(yuv_data.cicp().matrix_coefficients()));
    TRY(encoder.encode(yuv_data.cicp().video_full_range_flag()));
    return {};
}

template<>
ErrorOr<void> encode(Encoder& encoder, NonnullRefPtr<Media::VideoFrame const> const& frame)
{
    return encoder.encode(*frame);
}

template<>
ErrorOr<NonnullRefPtr<Media::VideoFrame const>> decode(Decoder& decoder)
{
    auto yuv_data_buffer = TRY(decoder.decode<Core::AnonymousBuffer>());
    if (!yuv_data_buffer.is_valid())
        return Error::from_string_literal("IPC: VideoFrame contained invalid YUV data");

    auto color_space = TRY(decoder.decode<Gfx::ColorSpace>());
    auto timestamp = TRY(decoder.decode<AK::Duration>());
    auto duration = TRY(decoder.decode<AK::Duration>());
    auto size = TRY(decoder.decode<Gfx::IntSize>());
    auto bit_depth = TRY(decoder.decode<u8>());
    auto subsampling = Media::Subsampling {
        TRY(decoder.decode<bool>()),
        TRY(decoder.decode<bool>()),
    };
    auto cicp = Media::CodingIndependentCodePoints {
        TRY(decoder.decode<Media::ColorPrimaries>()),
        TRY(decoder.decode<Media::TransferCharacteristics>()),
        TRY(decoder.decode<Media::MatrixCoefficients>()),
        TRY(decoder.decode<Media::VideoFullRangeFlag>()),
    };

    if (!color_primaries_ipc_value_valid(cicp.color_primaries())
        || !transfer_characteristics_ipc_value_valid(cicp.transfer_characteristics())
        || !matrix_coefficients_ipc_value_valid(cicp.matrix_coefficients())
        || !video_full_range_flag_ipc_value_valid(cicp.video_full_range_flag()))
        return Error::from_string_literal("IPC: VideoFrame contained invalid CICP metadata");

    auto yuv_data = TRY(Gfx::YUVData::create_from_anonymous_buffer(size, bit_depth, subsampling, cicp, move(yuv_data_buffer)));
    auto frame = TRY(try_make_ref_counted<Media::VideoFrame>(timestamp, duration, size.to_type<u32>(), bit_depth, move(color_space), move(yuv_data)));
    return NonnullRefPtr<Media::VideoFrame const> { *frame };
}

}
