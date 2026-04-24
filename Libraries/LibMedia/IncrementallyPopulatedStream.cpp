/*
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/RefPtr.h>
#include <LibCore/EventLoop.h>
#include <LibMedia/IncrementallyPopulatedStream.h>

namespace Media {

static constexpr u64 PRECEDING_DATA_SIZE = 1 * KiB;
static constexpr u64 FORWARD_REQUEST_THRESHOLD = 1 * MiB;
static constexpr AK::Duration CURSOR_ACTIVE_TIME = AK::Duration::from_milliseconds(50);

NonnullRefPtr<IncrementallyPopulatedStream> IncrementallyPopulatedStream::create_empty()
{
    return adopt_ref(*new IncrementallyPopulatedStream());
}

NonnullRefPtr<IncrementallyPopulatedStream> IncrementallyPopulatedStream::create_from_data(ReadonlyBytes data)
{
    auto stream = create_empty();
    auto size = data.size();
    stream->add_chunk_at(0, MUST(ByteBuffer::copy(data)));
    stream->close();
    VERIFY(stream->size() == size);
    return stream;
}

NonnullRefPtr<IncrementallyPopulatedStream> IncrementallyPopulatedStream::create_from_buffer(ByteBuffer const& buffer)
{
    return create_from_data(buffer.bytes());
}

IncrementallyPopulatedStream::IncrementallyPopulatedStream() = default;

IncrementallyPopulatedStream::~IncrementallyPopulatedStream() = default;

void IncrementallyPopulatedStream::set_data_request_callback(DataRequestCallback callback)
{
    Threading::MutexLocker locker { m_mutex };

    if (!callback) {
        m_callback_event_loop = nullptr;
        m_data_request_callback = nullptr;
        return;
    }

    m_callback_event_loop = Core::EventLoop::current_weak();
    m_data_request_callback = move(callback);
}

void IncrementallyPopulatedStream::add_chunk_at(u64 offset, ByteBuffer data)
{
    VERIFY(!data.is_empty());
    auto new_chunk_end = offset + data.size();
    m_last_chunk_end = new_chunk_end;

    Threading::MutexLocker locker { m_mutex };

    auto previous_chunk_iter = m_chunks.find_largest_not_above_iterator(offset);

    if (!previous_chunk_iter.is_end() && previous_chunk_iter->end() == offset) {
        // Hot path: the new data is a direct continuation of the previous chunk. Append
        // it as a fragment without copying any existing data.
        previous_chunk_iter->append_fragment(move(data));
    } else if (previous_chunk_iter.is_end() || previous_chunk_iter->end() < offset) {
        // No previous chunk, or the previous chunk ends before our start with a gap.
        // Insert a new chunk.
        m_chunks.insert(offset, DataChunk { offset, move(data) });
    } else {
        // The previous chunk overlaps this new data. Rare path: typically only happens
        // when a pre-fetched range races with a new arrival.
        if (previous_chunk_iter->end() >= new_chunk_end) {
            // Already fully covered.
            begin_new_request_while_locked(previous_chunk_iter->end());
            return;
        }
        // Append just the non-overlapping suffix as a new fragment.
        auto suffix_start_in_data = previous_chunk_iter->end() - offset;
        auto suffix = MUST(ByteBuffer::copy(data.span().slice(suffix_start_in_data)));
        previous_chunk_iter->append_fragment(move(suffix));
    }

    // After insertion/extension, merge with the next chunk if it now abuts or
    // overlaps. Rare: only happens for pre-fetched ranges meeting new arrivals.
    auto chunk_iter = m_chunks.find_largest_not_above_iterator(offset);
    VERIFY(!chunk_iter.is_end());
    auto next_iter = chunk_iter;
    ++next_iter;
    while (!next_iter.is_end() && next_iter->offset() <= chunk_iter->end()) {
        auto next_offset = next_iter->offset();
        auto next_end = next_iter->end();

        if (next_end > chunk_iter->end()) {
            // Copy only the tail of next_chunk that extends beyond chunk_iter,
            // assembled from the relevant fragments.
            auto gap_start = chunk_iter->end();
            auto tail_size = next_end - gap_start;
            auto tail = MUST(ByteBuffer::create_uninitialized(tail_size));
            auto tail_bytes = tail.bytes();
            u64 local_pos = gap_start - next_offset;
            u64 bytes_needed = tail_size;
            u64 out = 0;
            auto const& frags = next_iter->fragments();
            auto const& frag_ends = next_iter->fragment_ends();
            for (size_t i = 0; i < frags.size() && bytes_needed > 0; ++i) {
                auto frag_start = (i == 0) ? u64 { 0 } : frag_ends[i - 1];
                auto frag_end = frag_ends[i];
                if (frag_end <= local_pos)
                    continue;
                auto start_in_frag = local_pos - frag_start;
                auto available = frag_end - local_pos;
                auto to_copy = min<u64>(available, bytes_needed);
                frags[i].span().slice(start_in_frag, to_copy).copy_to(tail_bytes.slice(out, to_copy));
                out += to_copy;
                local_pos += to_copy;
                bytes_needed -= to_copy;
            }
            chunk_iter->append_fragment(move(tail));
        }

        VERIFY(m_chunks.remove(next_offset));
        next_iter = chunk_iter;
        ++next_iter;
        begin_new_request_while_locked(chunk_iter->end());
    }

    m_state_changed.broadcast();
}

void IncrementallyPopulatedStream::close()
{
    Threading::MutexLocker locker { m_mutex };
    m_expected_size = m_last_chunk_end;
    m_closed = true;
    m_state_changed.broadcast();
}

u64 IncrementallyPopulatedStream::size()
{
    Threading::MutexLocker locker { m_mutex };
    while (!m_expected_size.has_value())
        m_state_changed.wait();
    return m_expected_size.value();
}

void IncrementallyPopulatedStream::set_expected_size(u64 expected_size)
{
    Threading::MutexLocker locker { m_mutex };
    m_expected_size = expected_size;
    m_state_changed.broadcast();
}

Optional<u64> IncrementallyPopulatedStream::expected_size() const
{
    Threading::MutexLocker locker { m_mutex };
    return m_expected_size;
}

void IncrementallyPopulatedStream::begin_new_request_while_locked(u64 position)
{
    if (position == m_currently_requested_position)
        return;

    m_currently_requested_position = position;
    m_last_chunk_end = position;

    if (m_expected_size.has_value() && position >= m_expected_size.value())
        return;

    auto event_loop = m_callback_event_loop->take();
    if (!event_loop)
        return;
    event_loop->deferred_invoke([stream = NonnullRefPtr(*this), position] {
        if (stream->m_data_request_callback)
            stream->m_data_request_callback(position);
    });
}

static u64 adjust_request_position(u64 position)
{
    if (position > PRECEDING_DATA_SIZE)
        return position - PRECEDING_DATA_SIZE;
    return 0;
}

bool IncrementallyPopulatedStream::check_if_data_is_available_or_begin_request_while_locked(MonotonicTime now, u64 position, u64 length)
{
    auto* chunk = m_chunks.find_largest_not_above(position);
    if (!chunk)
        return m_closed;

    VERIFY(position >= chunk->offset());

    auto potential_request_position = adjust_request_position(position);
    potential_request_position = max(chunk->end(), position);
    for (size_t i = 0; i < m_cursors.size(); i++) {
        auto const& other_cursor = m_cursors[i];
        if (now >= other_cursor.m_active_timeout && !other_cursor.m_blocked)
            continue;
        if (other_cursor.m_position < potential_request_position) {
            auto* other_cursor_chunk = m_chunks.find_largest_not_above(other_cursor.m_position);
            if (other_cursor_chunk && other_cursor_chunk->end() >= other_cursor.m_position) {
                potential_request_position = other_cursor_chunk->end();
                continue;
            }
            potential_request_position = other_cursor.m_position;
        }
    }
    if (m_currently_requested_position > potential_request_position || potential_request_position > m_last_chunk_end + FORWARD_REQUEST_THRESHOLD)
        begin_new_request_while_locked(potential_request_position);

    u64 end = position + length;
    if (m_closed && end > m_expected_size.value())
        end = m_expected_size.value();
    return end <= chunk->end();
}

size_t IncrementallyPopulatedStream::read_from_chunks_while_locked(u64 position, Bytes& bytes) const
{
    auto chunk_iterator = m_chunks.find_largest_not_above_iterator(position);
    VERIFY(!chunk_iterator.is_end());
    auto const* chunk = &*chunk_iterator;
    VERIFY(position >= chunk->offset());

    u64 read_end = position + bytes.size();
    if (m_closed && m_expected_size.has_value() && read_end > m_expected_size.value())
        read_end = m_expected_size.value();

    u64 copy_size = read_end > chunk->end() ? chunk->end() - position : read_end - position;

    // Binary-search for the fragment containing `position` within the chunk. After
    // that we just walk fragments forward until copy_size is satisfied.
    u64 local_pos = position - chunk->offset();
    auto const& fragments = chunk->fragments();
    auto const& fragment_ends = chunk->fragment_ends();

    size_t lo = 0;
    size_t hi = fragments.size();
    while (lo < hi) {
        auto mid = lo + (hi - lo) / 2;
        if (fragment_ends[mid] <= local_pos)
            lo = mid + 1;
        else
            hi = mid;
    }

    u64 remaining = copy_size;
    u64 out = 0;
    for (size_t i = lo; i < fragments.size() && remaining > 0; ++i) {
        auto frag_start = (i == 0) ? u64 { 0 } : fragment_ends[i - 1];
        auto frag_end = fragment_ends[i];
        auto start_in_frag = local_pos - frag_start;
        auto available = frag_end - local_pos;
        auto to_copy = min<u64>(available, remaining);
        fragments[i].span().slice(start_in_frag, to_copy).copy_to(bytes.slice(out, to_copy));
        out += to_copy;
        local_pos += to_copy;
        remaining -= to_copy;
    }
    VERIFY(remaining == 0);

    return copy_size;
}

DecoderErrorOr<size_t> IncrementallyPopulatedStream::read_at(Cursor& cursor, size_t position, Bytes& bytes)
{
    Threading::MutexLocker locker { m_mutex };

    auto now = MonotonicTime::now_coarse();
    cursor.m_active_timeout = now + CURSOR_ACTIVE_TIME;

    while (!cursor.m_aborted) {
        if (check_if_data_is_available_or_begin_request_while_locked(now, position, bytes.size()))
            break;

        cursor.m_blocked = true;
        m_state_changed.wait();
        cursor.m_blocked = false;
    }

    if (cursor.m_aborted)
        return DecoderError::with_description(DecoderErrorCategory::Aborted, "Blocking read was aborted"sv);

    if (m_closed && position >= m_expected_size.value())
        return DecoderError::with_description(DecoderErrorCategory::EndOfStream, "Blocking read reached end of stream"sv);

    if (bytes.size() == 0)
        return 0;

    return read_from_chunks_while_locked(position, bytes);
}

NonnullRefPtr<MediaStreamCursor> IncrementallyPopulatedStream::create_cursor()
{
    return adopt_ref(*new Cursor(NonnullRefPtr { *this }));
}

IncrementallyPopulatedStream::Cursor::Cursor(NonnullRefPtr<IncrementallyPopulatedStream> const& stream)
    : m_stream(stream)
{
    Threading::MutexLocker locker { m_stream->m_mutex };
    m_stream->m_cursors.append(*this);
}

IncrementallyPopulatedStream::Cursor::~Cursor()
{
    Threading::MutexLocker locker { m_stream->m_mutex };
    VERIFY(m_stream->m_cursors.remove_first_matching([&](Cursor const& cursor) { return this == &cursor; }));
}

DecoderErrorOr<void> IncrementallyPopulatedStream::Cursor::seek(i64 offset, SeekMode mode)
{
    switch (mode) {
    case SeekMode::SetPosition:
        m_position = offset;
        break;
    case SeekMode::FromCurrentPosition:
        m_position += offset;
        break;
    case SeekMode::FromEndPosition:
        m_position = this->size() + offset;
        break;
    default:
        VERIFY_NOT_REACHED();
    }

    m_active_timeout = MonotonicTime::now_coarse() + CURSOR_ACTIVE_TIME;
    return {};
}

DecoderErrorOr<size_t> IncrementallyPopulatedStream::Cursor::read_into(Bytes bytes)
{
    auto read_count = TRY(m_stream->read_at(*this, m_position, bytes));
    m_position += read_count;
    return read_count;
}

void IncrementallyPopulatedStream::Cursor::abort()
{
    Threading::MutexLocker locker { m_stream->m_mutex };
    m_aborted = true;
    m_stream->m_state_changed.broadcast();
}

}
