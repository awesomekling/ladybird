/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Atomic.h>
#include <AK/ConditionVariable.h>
#include <AK/Error.h>
#include <AK/HashMap.h>
#include <AK/Mutex.h>
#include <AK/NonnullOwnPtr.h>
#include <AK/NonnullRefPtr.h>
#include <AK/Optional.h>
#include <LibGfx/Font/SystemFallbackFonts.h>
#include <LibIPC/TransportHandle.h>
#include <LibThreading/Forward.h>
#include <LibWebCommon/Export.h>

namespace WebView {

// The render side's own connection to the font service, living in the renderer process.
//
// A system fallback miss has to leave the process, and the connection it used to leave on belongs
// to the document thread: reaching it from a render pass would race the document's own traffic
// today and deadlock once the pass runs on a thread of its own, because the reply is pumped by the
// event loop of the very thread the pass is keeping busy.
//
// LibIPC pins a connection to the thread that constructed it, and that thread must have a
// Core::EventLoop, so this service owns a thread that does nothing else. Callers need neither a
// connection nor an event loop; they hand over a code point and a style and block until the answer
// comes back.
class WEBCOMMON_API RendererFontService final : public Gfx::SystemFallbackFontService {
    AK_MAKE_NONCOPYABLE(RendererFontService);
    AK_MAKE_NONMOVABLE(RendererFontService);

public:
    AK_ALLOC_WITH_KMALLOC;

    static ErrorOr<NonnullOwnPtr<RendererFontService>> create(IPC::TransportHandle);
    virtual ~RendererFontService() override;

    virtual RefPtr<Gfx::Font const> match_font_for_code_point(Gfx::SystemFallbackFontKey const&) override;
    virtual void did_change_font_set() override;

private:
    explicit RendererFontService(IPC::TransportHandle);
    intptr_t thread_main();

    // Serializes callers, so that one question is in flight at a time. Misses are rare enough that
    // a queue would only be machinery nobody exercises.
    Mutex m_call_mutex;

    Mutex m_mutex;
    ConditionVariable m_initialization_condition { m_mutex };
    ConditionVariable m_request_condition { m_mutex };
    ConditionVariable m_answer_condition { m_mutex };
    Optional<IPC::TransportHandle> m_transport_handle;
    bool m_initialized { false };
    bool m_connected { false };
    bool m_should_quit { false };
    bool m_has_request { false };
    Gfx::SystemFallbackFontKey m_request;
    RefPtr<Gfx::Font const> m_answer;

    Atomic<bool> m_font_set_changed { false };

    // Only the service's own thread touches this.
    HashMap<u64, NonnullRefPtr<Gfx::Typeface>> m_typefaces;

    NonnullRefPtr<Threading::Thread> m_thread;
};

}
