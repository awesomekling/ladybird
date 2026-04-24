/*
 * Copyright (c) 2024-2026, Tim Flynn <trflynn89@ladybird.org>
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/ByteBuffer.h>
#include <AK/Vector.h>
#include <LibGC/CellAllocator.h>
#include <LibHTTP/Forward.h>
#include <LibJS/Heap/Cell.h>
#include <LibWeb/Forward.h>

namespace Web::Fetch::Fetching {

class FetchedDataReceiver final : public JS::Cell {
    GC_CELL(FetchedDataReceiver, JS::Cell);
    GC_DECLARE_ALLOCATOR(FetchedDataReceiver);

public:
    virtual ~FetchedDataReceiver() override;

    void set_pending_promise(GC::Ref<WebIDL::Promise>);

    void set_response(GC::Ref<Fetch::Infrastructure::Response const> response) { m_response = response; }
    void set_body(GC::Ref<Fetch::Infrastructure::Body> body);

    enum class NetworkState {
        Ongoing,
        Complete,
        Error,
    };
    void handle_network_bytes(ReadonlyBytes, NetworkState);

private:
    FetchedDataReceiver(GC::Ref<Infrastructure::FetchParams const>, GC::Ref<Streams::ReadableStream>, RefPtr<HTTP::MemoryCache>);

    virtual void visit_edges(Visitor& visitor) override;

    void pull_bytes_into_stream();
    void close_stream();

    bool buffer_is_eof() const { return m_pending_chunks.is_empty(); }

    GC::Ref<Infrastructure::FetchParams const> m_fetch_params;
    GC::Ptr<Fetch::Infrastructure::Response const> m_response;
    GC::Ptr<Fetch::Infrastructure::Body> m_body;

    GC::Ref<Streams::ReadableStream> m_stream;
    GC::Ptr<WebIDL::Promise> m_pending_promise;

    RefPtr<HTTP::MemoryCache> m_http_cache;

    // FIFO of chunks waiting to be pulled into the stream. Each chunk is a fresh,
    // right-sized allocation, avoiding the O(N^2) reallocation of a single growing
    // buffer.
    Vector<ByteBuffer> m_pending_chunks;

    // Accumulated response body for the HTTP memory cache. Only populated when
    // caching is active for this fetch.
    ByteBuffer m_cache_body;
    bool m_accumulate_for_cache { false };

    enum class LifecycleState {
        Receiving,
        CompletePending,
        ReadyToClose,
        Closed,
    };
    LifecycleState m_lifecycle_state { LifecycleState::Receiving };
    bool m_has_unfulfilled_promise { false };
};

}
