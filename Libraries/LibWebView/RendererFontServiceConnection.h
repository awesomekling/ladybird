/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/AtomicRefCounted.h>
#include <AK/ConditionVariable.h>
#include <AK/Error.h>
#include <AK/Mutex.h>
#include <AK/NonnullRefPtr.h>
#include <AK/Optional.h>
#include <AK/RefPtr.h>
#include <LibCore/Forward.h>
#include <LibIPC/TransportHandle.h>
#include <LibThreading/Forward.h>
#include <LibWebView/Export.h>
#include <LibWebView/Forward.h>

namespace WebView {

// The font service's end of the connection a renderer's render side uses for system fallback.
//
// The UI process answers font questions on its main thread, and it also makes synchronous calls
// into the renderer. Once the render pipeline has a thread of its own, a font question asked from
// a render pass could therefore wait on a UI main thread that is itself waiting on a renderer main
// thread that is waiting on the render pass. This connection is served by a thread that does
// nothing else, which is why no such cycle can form, and it is why the Compositor already has one.
class WEBVIEW_API RendererFontServiceConnection final : public AtomicRefCounted<RendererFontServiceConnection> {
    AK_MAKE_NONCOPYABLE(RendererFontServiceConnection);
    AK_MAKE_NONMOVABLE(RendererFontServiceConnection);

public:
    static ErrorOr<NonnullRefPtr<RendererFontServiceConnection>> create(FontService&);
    ~RendererFontServiceConnection();

    IPC::TransportHandle take_transport_handle();

private:
    explicit RendererFontServiceConnection(FontService&);
    intptr_t thread_main();

    NonnullRefPtr<FontService> m_font_service;
    NonnullRefPtr<Threading::Thread> m_thread;
    Mutex m_mutex;
    ConditionVariable m_initialization_condition { m_mutex };
    bool m_initialized { false };
    Optional<Error> m_initialization_error;
    Optional<IPC::TransportHandle> m_transport_handle;
    RefPtr<Core::WeakEventLoopReference> m_event_loop;
};

}
