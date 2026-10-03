/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Noncopyable.h>
#include <AK/RefCounted.h>
#include <LibWeb/Layout/TreeBuilderRustFFI.h>

namespace Web::Layout {

// A document's render state as the host holds it: the DocumentHost that names the state. The state owns the document's
// style engine and layout arena, so the style engine bridge and the layout node arena both keep it alive, and it goes
// when the last of them does.
class RenderDocument : public RefCounted<RenderDocument> {
    AK_MAKE_NONCOPYABLE(RenderDocument);
    AK_MAKE_NONMOVABLE(RenderDocument);

public:
    static NonnullRefPtr<RenderDocument> create(u8 device_class)
    {
        return adopt_ref(*new RenderDocument(device_class));
    }

    ~RenderDocument()
    {
        RustFFI::document_host_destroy(m_host);
    }

    RustFFI::DocumentHost* host() const { return m_host; }

private:
    explicit RenderDocument(u8 device_class)
        : m_host(RustFFI::document_host_create(device_class))
    {
    }

    RustFFI::DocumentHost* m_host { nullptr };
};

// A scope of a read of a document's render state that the host waits for: a script API call's where `by_script`, and
// the host's own otherwise. The read's first job spends it, and only a read takes a frame in flight in. A scope begun
// inside the document's open read belongs to that read.
class ForcedReadScope {
    AK_MAKE_NONCOPYABLE(ForcedReadScope);
    AK_MAKE_NONMOVABLE(ForcedReadScope);

public:
    ForcedReadScope(RustFFI::DocumentHost* host, bool by_script)
        : m_host(host)
    {
        RustFFI::document_host_begin_forced_read(host, by_script);
    }

    // A read of the render state of `document`.
    ForcedReadScope(DOM::Document const& document, bool by_script);

    ~ForcedReadScope()
    {
        RustFFI::document_host_end_forced_read(m_host);
    }

private:
    RustFFI::DocumentHost* m_host { nullptr };
};

}
