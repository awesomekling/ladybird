/*
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Atomic.h>
#include <AK/AtomicRefCounted.h>
#include <AK/Forward.h>
#include <AK/Function.h>
#include <AK/RedBlackTree.h>
#include <AK/RefPtr.h>
#include <AK/Time.h>
#include <AK/Vector.h>
#include <LibCore/Forward.h>
#include <LibMedia/DecoderError.h>
#include <LibMedia/Export.h>
#include <LibMedia/MediaStream.h>
#include <LibThreading/ConditionVariable.h>
#include <LibThreading/Mutex.h>

namespace Media {

class MEDIA_API IncrementallyPopulatedStream : public MediaStream {
public:
    static NonnullRefPtr<IncrementallyPopulatedStream> create_empty();
    static NonnullRefPtr<IncrementallyPopulatedStream> create_from_data(ReadonlyBytes);
    static NonnullRefPtr<IncrementallyPopulatedStream> create_from_buffer(ByteBuffer const&);

    ~IncrementallyPopulatedStream();

    virtual NonnullRefPtr<MediaStreamCursor> create_cursor() override;

    // Callback invoked when data at a specific offset is needed but not available.
    // The callback receives the desired offset position and is invoked on the provided event loop.
    using DataRequestCallback = Function<void(u64 offset)>;
    void set_data_request_callback(DataRequestCallback);

    void add_chunk_at(u64 offset, ByteBuffer);
    u64 next_chunk_start() const { return m_last_chunk_end; }

    void close();

    u64 size();
    void set_expected_size(u64);
    Optional<u64> expected_size() const;

    class MEDIA_API Cursor : public MediaStreamCursor {
    public:
        ~Cursor();

        virtual DecoderErrorOr<void> seek(i64 offset, SeekMode mode) override;
        virtual DecoderErrorOr<size_t> read_into(Bytes bytes) override;

        virtual size_t position() const override { return m_position; }
        virtual size_t size() const override { return m_stream->size(); }

        virtual void abort() override;
        virtual void reset_abort() override { m_aborted = false; }
        virtual bool is_aborted() const override { return m_aborted; }

        virtual bool is_blocked() const override { return m_blocked; }

    private:
        friend class IncrementallyPopulatedStream;

        Cursor(NonnullRefPtr<IncrementallyPopulatedStream> const& stream);

        NonnullRefPtr<IncrementallyPopulatedStream> m_stream;
        size_t m_position { 0 };
        bool m_aborted { false };
        Atomic<bool> m_blocked { false };
        MonotonicTime m_active_timeout { MonotonicTime::now_coarse() };
    };

private:
    class DataChunk {
    public:
        DataChunk(u64 offset, ByteBuffer&& data)
            : m_offset(offset)
            , m_size(data.size())
        {
            auto fragment_size = data.size();
            m_fragments.append(move(data));
            m_fragment_ends.append(fragment_size);
        }

        u64 offset() const { return m_offset; }
        u64 size() const { return m_size; }
        u64 end() const { return m_offset + m_size; }
        Vector<ByteBuffer> const& fragments() const { return m_fragments; }
        Vector<u64> const& fragment_ends() const { return m_fragment_ends; }

        // Appends a fragment to the end of this chunk. Caller must ensure that
        // the fragment abuts the current end of the chunk. This does not copy
        // any existing data.
        void append_fragment(ByteBuffer&& fragment)
        {
            m_size += fragment.size();
            m_fragments.append(move(fragment));
            m_fragment_ends.append(m_size);
        }

        bool contains(u64 position) const { return position >= m_offset && position < end(); }

    private:
        u64 m_offset { 0 };
        u64 m_size { 0 };
        // Data is stored as a list of fragments so that sequential arrivals can be
        // recorded without any memcpy of previously accumulated data. m_fragment_ends
        // stores the cumulative size (relative to m_offset) at the end of each
        // fragment, so we can binary-search to find the fragment covering any
        // position in O(log F).
        Vector<ByteBuffer> m_fragments;
        Vector<u64> m_fragment_ends;
    };

    IncrementallyPopulatedStream();

    friend class Cursor;

    using Chunks = AK::RedBlackTree<u64, DataChunk>;

    DecoderErrorOr<size_t> read_at(Cursor&, size_t position, Bytes&);

    void begin_new_request_while_locked(u64 position);
    bool check_if_data_is_available_or_begin_request_while_locked(MonotonicTime now, u64 position, u64 length);
    size_t read_from_chunks_while_locked(u64 position, Bytes& bytes) const;

    mutable Threading::Mutex m_mutex;
    Vector<Cursor&> m_cursors;
    Threading::ConditionVariable m_state_changed { m_mutex };

    Chunks m_chunks;
    Optional<u64> m_expected_size;
    bool m_closed { false };

    RefPtr<Core::WeakEventLoopReference> m_callback_event_loop;
    DataRequestCallback m_data_request_callback;
    u64 m_currently_requested_position { 0 };
    u64 m_last_chunk_end { 0 };
};

}
