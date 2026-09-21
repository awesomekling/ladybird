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
#include <AK/Variant.h>
#include <LibGfx/Font/SharedFontProvider.h>
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
class WEBCOMMON_API RendererFontService final
    : public Gfx::SystemFallbackFontService
    , public Gfx::RenderSideFontBroker {
    AK_MAKE_NONCOPYABLE(RendererFontService);
    AK_MAKE_NONMOVABLE(RendererFontService);

public:
    AK_ALLOC_WITH_KMALLOC;

    static ErrorOr<NonnullOwnPtr<RendererFontService>> create(IPC::TransportHandle);
    virtual ~RendererFontService() override;

    virtual RefPtr<Gfx::Font const> match_font_for_code_point(Gfx::SystemFallbackFontKey const&) override;
    virtual void did_change_font_set() override;

    // The questions a font match asks, for a caller that must not use the document thread's
    // connection. Family matching needs these: a catalog face carries no font data, so the first
    // use of any system family has to ask the font service to open it.
    virtual Gfx::BrokeredFont open_font(u64 generation, u64 face_id) override;
    virtual Gfx::BrokeredFont match_font(String const& family, u16 weight, u16 width, u8 slope) override;
    virtual Optional<FlyString> resolve_generic_family(String const& family, u16 weight, u8 slope) override;

private:
    struct OpenFontRequest {
        u64 generation { 0 };
        u64 face_id { 0 };
    };
    struct MatchFontRequest {
        String family;
        u16 weight { 0 };
        u16 width { 0 };
        u8 slope { 0 };
    };
    struct ResolveGenericFamilyRequest {
        String family;
        u16 weight { 0 };
        u8 slope { 0 };
    };
    using Request = Variant<Gfx::SystemFallbackFontKey, OpenFontRequest, MatchFontRequest, ResolveGenericFamilyRequest>;
    using Answer = Variant<Empty, RefPtr<Gfx::Font const>, Gfx::BrokeredFont, Optional<FlyString>>;

    explicit RendererFontService(IPC::TransportHandle);
    intptr_t thread_main();
    Answer ask(Request);

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
    Request m_request { Gfx::SystemFallbackFontKey {} };
    Answer m_answer;

    Atomic<bool> m_font_set_changed { false };

    // Only the service's own thread touches this.
    HashMap<u64, NonnullRefPtr<Gfx::Typeface>> m_typefaces;

    NonnullRefPtr<Threading::Thread> m_thread;
};

}
