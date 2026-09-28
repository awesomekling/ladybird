/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Format.h>
#include <AK/HashMap.h>
#include <AK/Mutex.h>
#include <AK/OwnPtr.h>
#include <AK/Singleton.h>
#include <AK/StringView.h>
#include <LibGfx/Font/Font.h>
#include <LibGfx/Font/FontDatabase.h>
#include <LibGfx/Font/SystemFallbackFonts.h>

namespace Gfx {

namespace {

struct SystemFallbackFontKeyTraits : public DefaultTraits<SystemFallbackFontKey> {
    static unsigned hash(SystemFallbackFontKey const& key)
    {
        auto style = pair_int_hash(pair_int_hash(key.weight, key.width), pair_int_hash(key.slope, key.prefer_color_emoji));
        return pair_int_hash(pair_int_hash(key.code_point, style), pair_int_hash(bit_cast<u32>(key.point_size), 0));
    }
};

struct SystemFallbackFontCache {
    Mutex mutex;
    // A miss is an answer too: without it every code point no family covers asks the provider again
    // on every lookup, and for the shared provider that is an IPC round trip.
    HashMap<SystemFallbackFontKey, RefPtr<Font const>, SystemFallbackFontKeyTraits> fonts;
    OwnPtr<SystemFallbackFontService> render_side_service;
};

Singleton<SystemFallbackFontCache> s_system_fallback_font_cache;

SystemFallbackFontCache& system_fallback_font_cache()
{
    return *s_system_fallback_font_cache;
}

void report_miss(SystemFallbackFontKey const& key, bool served_by_the_render_side)
{
    static bool const census = getenv("LADYBIRD_FONT_FALLBACK_CENSUS") != nullptr;
    if (!census)
        return;
    dbgln("FONT FALLBACK MISS: path={} code_point=U+{:04X} weight={} width={} slope={} emoji={} size={}",
        served_by_the_render_side ? "render_side"sv : "document_thread"sv,
        key.code_point, key.weight, key.width, key.slope, key.prefer_color_emoji, key.point_size);
}

enum class MissPath {
    DocumentThread,
    RenderSide,
};

RefPtr<Font const> system_fallback_font(SystemFallbackFontKey const& key, MissPath path)
{
    auto& cache = system_fallback_font_cache();
    // NB: The lookup runs under the lock rather than beside it, so one code point is matched once
    //     even when several threads want it. The cost falls on misses only.
    MutexLocker locker(cache.mutex);
    if (auto cached = cache.fonts.get(key); cached.has_value())
        return *cached;

    auto* service = path == MissPath::RenderSide ? cache.render_side_service.ptr() : nullptr;
    RefPtr<Font const> font;
    if (service) {
        font = service->match_font_for_code_point(key);
    } else {
        font = FontDatabase::the().get_font_for_code_point(
            key.code_point, key.point_size, key.weight, key.width, key.slope, key.prefer_color_emoji);
    }
    cache.fonts.set(key, font);
    report_miss(key, service != nullptr);
    return font;
}

}

// Key function for SystemFallbackFontService, to emit the vtable here.
SystemFallbackFontService::~SystemFallbackFontService() = default;

void install_render_side_system_fallback_font_service(NonnullOwnPtr<SystemFallbackFontService> service)
{
    auto& cache = system_fallback_font_cache();
    MutexLocker locker(cache.mutex);
    VERIFY(!cache.render_side_service);
    cache.render_side_service = move(service);
}

bool has_render_side_system_fallback_font_service()
{
    auto& cache = system_fallback_font_cache();
    MutexLocker locker(cache.mutex);
    return cache.render_side_service;
}

RefPtr<Font const> system_fallback_font(SystemFallbackFontKey const& key)
{
    return system_fallback_font(key, MissPath::DocumentThread);
}

RefPtr<Font const> system_fallback_font_from_render_side(SystemFallbackFontKey const& key)
{
    return system_fallback_font(key, MissPath::RenderSide);
}

void clear_system_fallback_font_cache()
{
    auto& cache = system_fallback_font_cache();
    MutexLocker locker(cache.mutex);
    cache.fonts.clear();
    if (cache.render_side_service)
        cache.render_side_service->did_change_font_set();
}

size_t system_fallback_font_cache_size()
{
    auto& cache = system_fallback_font_cache();
    MutexLocker locker(cache.mutex);
    return cache.fonts.size();
}

}

extern "C" {
void const* ladybird_gfx_system_fallback_font(u32, u16, u16, u8, bool, float);
void const* ladybird_gfx_font_invisible_variant(void const*);
}

// Only a render stage calls this; the document thread's callers go through the live cascade's own
// fallback callback. The caller interns the answer, which takes its own reference; the memo keeps
// it live until then, and forever after, so handing back a borrowed pointer is safe.
extern "C" void const* ladybird_gfx_system_fallback_font(u32 code_point, u16 weight, u16 width, u8 slope, bool prefer_color_emoji, float point_size)
{
    return Gfx::system_fallback_font_from_render_side({ .code_point = code_point,
                                                          .weight = weight,
                                                          .width = width,
                                                          .slope = slope,
                                                          .prefer_color_emoji = prefer_color_emoji,
                                                          .point_size = point_size })
        .ptr();
}

// https://drafts.csswg.org/css-fonts-4/#invisible-fallback
// An anonymous face with the selected face's metrics and no ink. Transfers one reference to the
// caller, which owns the variant and memoizes it for as long as it needs it.
extern "C" void const* ladybird_gfx_font_invisible_variant(void const* font)
{
    VERIFY(font);
    return &static_cast<Gfx::Font const*>(font)->invisible_variant().leak_ref();
}
