/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibCore/EventLoop.h>
#include <LibCore/System.h>
#include <LibGfx/Font/Font.h>
#include <LibGfx/Font/SharedFontProvider.h>
#include <LibIPC/ConnectionToServer.h>
#include <LibIPC/Transport.h>
#include <LibThreading/Thread.h>
#include <LibWebCommon/WebView/RendererFontService.h>
#include <WebContent/RendererFontClientEndpoint.h>
#include <WebContent/RendererFontServerEndpoint.h>

namespace WebView {

namespace {

class RendererFontClient final : public IPC::ConnectionToServer<RendererFontClientEndpoint, RendererFontServerEndpoint> {
    C_OBJECT(RendererFontClient);

public:
    virtual ~RendererFontClient() override = default;

private:
    explicit RendererFontClient(NonnullOwnPtr<IPC::Transport> transport)
        : IPC::ConnectionToServer<RendererFontClientEndpoint, RendererFontServerEndpoint>(*this, move(transport))
    {
    }

    // The font service going away is not fatal to a renderer. Every later question answers null,
    // which is the same answer a code point no installed family covers has always had.
    virtual void die() override { }
};

}

ErrorOr<NonnullOwnPtr<RendererFontService>> RendererFontService::create(IPC::TransportHandle handle)
{
    auto service = adopt_own(*new RendererFontService(move(handle)));

    bool connected = false;
    {
        MutexLocker locker(service->m_mutex);
        service->m_initialization_condition.wait_while([&] { return !service->m_initialized; });
        connected = service->m_connected;
    }

    if (!connected)
        return Error::from_string_literal("Unable to connect to the font service");
    return service;
}

RendererFontService::RendererFontService(IPC::TransportHandle handle)
    : m_transport_handle(move(handle))
    , m_thread(Threading::Thread::construct("Renderer font service"sv, [this] {
        return thread_main();
    }))
{
    m_thread->start();
}

RendererFontService::~RendererFontService()
{
    {
        MutexLocker locker(m_mutex);
        m_should_quit = true;
        m_request_condition.broadcast();
    }

    if (m_thread->needs_to_be_joined())
        (void)m_thread->join();
}

void RendererFontService::did_change_font_set()
{
    m_font_set_changed.store(true, AK::MemoryOrder::memory_order_release);
}

RefPtr<Gfx::Font const> RendererFontService::match_font_for_code_point(Gfx::SystemFallbackFontKey const& key)
{
    MutexLocker call_locker(m_call_mutex);
    MutexLocker locker(m_mutex);
    if (!m_connected || m_should_quit)
        return {};

    m_request = key;
    m_has_request = true;
    m_request_condition.signal();
    m_answer_condition.wait_while([&] { return m_has_request; });
    return move(m_answer);
}

intptr_t RendererFontService::thread_main()
{
    // The loop below never runs. It exists because LibIPC needs one on this thread: a connection
    // registers a notifier when it is constructed, and defers a call after every drain.
    Core::EventLoop event_loop;

    RefPtr<RendererFontClient> client;
    {
        MutexLocker locker(m_mutex);
        if (auto transport = m_transport_handle->create_transport(); !transport.is_error())
            client = RendererFontClient::construct(transport.release_value());
        m_transport_handle.clear();
        m_connected = client != nullptr;
        m_initialized = true;
        m_initialization_condition.broadcast();
    }

    if (!client)
        return 1;

#ifdef AK_OS_WINDOWS
    if (auto response = client->send_sync_but_allow_failure<Messages::RendererFontServer::InitTransport>(Core::System::getpid()))
        client->transport().set_peer_pid(response->peer_pid());
#endif

    for (;;) {
        Gfx::SystemFallbackFontKey key;
        {
            MutexLocker locker(m_mutex);
            m_request_condition.wait_while([&] { return !m_has_request && !m_should_quit; });
            if (m_should_quit)
                break;
            key = m_request;
        }

        if (m_font_set_changed.exchange(false, AK::MemoryOrder::memory_order_acquire))
            m_typefaces.clear();

        RefPtr<Gfx::Font const> font;
        if (auto response = client->send_sync_but_allow_failure<Messages::RendererFontServer::MatchSystemFontForCodePoint>(key.code_point, key.weight, key.width, key.slope, key.prefer_color_emoji)) {
            auto generation = response->generation();
            auto brokered_font = response->take_font();
            if (auto typeface = m_typefaces.get(brokered_font.face_id); typeface.has_value()) {
                font = (*typeface)->font(key.point_size, {});
            } else if (brokered_font.face_id != 0) {
                RefPtr<Gfx::Typeface> loaded;
                brokered_font.source.visit(
                    [](Empty) {},
                    [&](Gfx::BrokeredFontFile& font_file) {
                        loaded = Gfx::load_typeface_from_font_file(font_file.ttc_index, font_file.format, move(font_file.file));
                    },
                    [&](Gfx::SystemFontReference const& reference) {
                        loaded = Gfx::load_typeface_from_system_font_reference(reference);
                    });
                if (loaded) {
                    loaded->set_system_font_identifier({ generation, brokered_font.face_id });
                    m_typefaces.set(brokered_font.face_id, *loaded);
                    font = loaded->font(key.point_size, {});
                }
            }
        }

        // The drain that delivered the reply deferred a call to this thread's loop. Nothing else
        // ever runs it, so run it here rather than let one accumulate per answer.
        event_loop.pump(Core::EventLoop::WaitMode::PollForEvents);

        {
            MutexLocker locker(m_mutex);
            m_answer = move(font);
            m_has_request = false;
            m_answer_condition.broadcast();
        }
    }

    if (client->is_open())
        client->shutdown();
    return 0;
}

}
