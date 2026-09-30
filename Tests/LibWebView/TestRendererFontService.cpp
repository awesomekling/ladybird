/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Atomic.h>
#include <LibCore/EventLoop.h>
#include <LibGfx/Font/Font.h>
#include <LibGfx/Font/SharedFontProvider.h>
#include <LibGfx/Font/SystemFallbackFonts.h>
#include <LibTest/TestCase.h>
#include <LibThreading/Thread.h>
#include <LibWebCommon/WebView/RendererFontService.h>
#include <LibWebView/FontService.h>
#include <LibWebView/RendererFontServiceConnection.h>

namespace {

Gfx::SystemFallbackFontKey key_for(u32 code_point)
{
    return {
        .code_point = code_point,
        .weight = 400,
        .width = Gfx::FontWidth::Normal,
        .slope = 0,
        .prefer_color_emoji = false,
        .point_size = 16,
    };
}

struct RenderSideFontService {
    NonnullRefPtr<WebView::FontService> font_service;
    NonnullRefPtr<WebView::RendererFontServiceConnection> connection;
};

// One service per process, as in a renderer. Leaked, because a test binary must not run its
// destructor after the test bodies have gone.
RenderSideFontService& render_side_font_service()
{
    static RenderSideFontService& service = *[] {
        auto font_service = WebView::FontService::create({});
        auto connection = MUST(WebView::RendererFontServiceConnection::create(*font_service));
        auto client = MUST(WebView::RendererFontService::create(connection->take_transport_handle()));
        Gfx::install_render_side_font_broker(*client);
        Gfx::install_render_side_system_fallback_font_service(move(client));
        return new RenderSideFontService { move(font_service), move(connection) };
    }();
    Gfx::clear_system_fallback_font_cache();
    return service;
}

}

// The shape the render thread will have: the main thread is inside a join, pumping nothing, while
// the pass it is waiting for needs a code point no installed family in its cascade covers. The
// answer has to come from a connection that is not the document thread's, or this never returns.
TEST_CASE(a_miss_from_another_thread_is_answered_while_the_main_thread_is_blocked)
{
    auto& installed = render_side_font_service();
    EXPECT(Gfx::has_render_side_system_fallback_font_service());

    // The document thread's loop exists and is not running, exactly as it will be during a join.
    Core::EventLoop event_loop;

    IGNORE_USE_IN_ESCAPING_LAMBDA Array<Gfx::Font const*, 4> matched {};
    IGNORE_USE_IN_ESCAPING_LAMBDA Atomic<bool> finished { false };
    IGNORE_USE_IN_ESCAPING_LAMBDA Array<u32, 4> code_points { 'A', 0x4e2d, 0x0416, 0x05d0 };

    auto worker = Threading::Thread::construct("RenderSideFontMiss"sv, [&] {
        for (size_t index = 0; index < code_points.size(); ++index)
            matched[index] = Gfx::system_fallback_font_from_render_side(key_for(code_points[index])).ptr();
        finished.store(true);
        return 0;
    });
    worker->start();
    (void)worker->join();

    EXPECT(finished.load());
    EXPECT_EQ(Gfx::system_fallback_font_cache_size(), code_points.size());

    // A machine with no font at all for basic Latin cannot run this suite, so the first of these
    // came back with a font that the service on the other end of the connection brokered.
    EXPECT(matched[0] != nullptr);

    // And the connection answered for the rest rather than the memo handing back a null it made
    // up: the service that brokers the fonts agrees about which code points it can cover.
    for (size_t index = 0; index < code_points.size(); ++index) {
        auto brokered = installed.font_service->match_font_for_code_point(code_points[index], 400, Gfx::FontWidth::Normal, 0, false);
        EXPECT_EQ(brokered.face_id != 0, matched[index] != nullptr);
    }
}

// Hits and misses at the same time, from threads the document thread is not one of.
TEST_CASE(render_side_hits_and_misses_run_on_several_threads_at_once)
{
    (void)render_side_font_service();

    IGNORE_USE_IN_ESCAPING_LAMBDA auto warm_key = key_for('A');
    auto warm_font = Gfx::system_fallback_font_from_render_side(warm_key).ptr();

    // Distinct per thread, so that every thread has misses of its own to interleave with the hits.
    IGNORE_USE_IN_ESCAPING_LAMBDA Array<u32, 4> first_code_points { 0x4e2d, 0x0416, 0x05d0, 0x0627 };
    IGNORE_USE_IN_ESCAPING_LAMBDA Array<u32, 4> second_code_points { 0x0e01, 0x0905, 0x3042, 0xac00 };
    IGNORE_USE_IN_ESCAPING_LAMBDA Array<Gfx::Font const*, 2> warm_answers {};

    Vector<NonnullRefPtr<Threading::Thread>> threads;
    for (size_t thread_index = 0; thread_index < 2; ++thread_index) {
        auto thread = Threading::Thread::construct("RenderSideFontMix"sv, [&, thread_index] {
            auto const& code_points = thread_index == 0 ? first_code_points : second_code_points;
            for (auto code_point : code_points) {
                warm_answers[thread_index] = Gfx::system_fallback_font_from_render_side(warm_key).ptr();
                (void)Gfx::system_fallback_font_from_render_side(key_for(code_point));
            }
            return 0;
        });
        thread->start();
        threads.append(move(thread));
    }
    for (auto& thread : threads)
        (void)thread->join();

    // A hit is the same object however many threads ask for it at once.
    for (auto const* answer : warm_answers)
        EXPECT_EQ(answer, warm_font);
    EXPECT_EQ(Gfx::system_fallback_font_cache_size(), 1u + first_code_points.size() + second_code_points.size());
}

// A catalog face carries a face id and no font data, so the first use of any system family has to
// ask the font service to open the file. That question used to go out on the document thread's own
// connection, which is the one a render pass must not touch.
TEST_CASE(a_cold_family_lookup_from_another_thread_is_answered_while_the_main_thread_is_blocked)
{
    auto& installed = render_side_font_service();
    EXPECT(Gfx::has_render_side_font_broker());

    // NB: CoreText resolves no generic family names, so ask for a family every macOS has there.
    auto family = installed.font_service->resolve_generic_family("sans-serif"_string, 400, 0);
    if (!family.has_value())
        family = "Helvetica"_fly_string;

    // The provider's own callbacks stand for the document thread's connection: a question that
    // takes them from inside a render-side scope is the regression this test is about.
    IGNORE_USE_IN_ESCAPING_LAMBDA Atomic<u32> questions_on_the_document_connection { 0 };
    auto make_provider = [&] {
        auto catalog = MUST(installed.font_service->clone_catalog());
        Gfx::SharedFontProviderCallbacks callbacks;
        callbacks.open_font = [&](u64 generation, u64 face_id) {
            questions_on_the_document_connection.fetch_add(1);
            return installed.font_service->open_font(generation, face_id);
        };
        callbacks.match_font = [&](String const& family, u16 weight, u16 width, u8 slope) {
            questions_on_the_document_connection.fetch_add(1);
            return installed.font_service->match_font(family, weight, width, slope);
        };
        return MUST(Gfx::SharedFontProvider::create_from_catalog_file_or_empty(move(catalog.file), catalog.size, catalog.generation, move(callbacks)));
    };

    // The document thread's loop exists and is not running, exactly as it will be during a join.
    Core::EventLoop event_loop;

    IGNORE_USE_IN_ESCAPING_LAMBDA auto render_side_provider = make_provider();
    IGNORE_USE_IN_ESCAPING_LAMBDA Atomic<u32> typefaces_seen { 0 };

    auto worker = Threading::Thread::construct("RenderSideFamilyMatch"sv, [&] {
        Gfx::RenderSideFontScope scope;
        render_side_provider->for_each_typeface_with_family_name(*family, [&](Gfx::Typeface const&) {
            typefaces_seen.fetch_add(1);
        });
        return 0;
    });
    worker->start();
    (void)worker->join();

    // A machine with no sans-serif family at all cannot run this suite, so the lookup found faces,
    // and every file it needed was opened without the main thread pumping anything.
    EXPECT(typefaces_seen.load() > 0u);
    EXPECT_EQ(questions_on_the_document_connection.load(), 0u);

    // The control: the same cold lookup outside a render-side scope still takes the callbacks, so
    // the count above is zero because the broker answered, not because nothing was asked.
    auto document_side_provider = make_provider();
    u32 typefaces_seen_on_the_document_side = 0;
    document_side_provider->for_each_typeface_with_family_name(*family, [&](Gfx::Typeface const&) {
        ++typefaces_seen_on_the_document_side;
    });
    EXPECT_EQ(typefaces_seen_on_the_document_side, typefaces_seen.load());
    EXPECT(questions_on_the_document_connection.load() > 0u);
}
