/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashMap.h>
#include <AK/Mutex.h>
#include <AK/Singleton.h>
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
};

Singleton<SystemFallbackFontCache> s_system_fallback_font_cache;

SystemFallbackFontCache& system_fallback_font_cache()
{
    return *s_system_fallback_font_cache;
}

}

RefPtr<Font const> system_fallback_font(SystemFallbackFontKey const& key)
{
    auto& cache = system_fallback_font_cache();
    // NB: The lookup runs under the lock rather than beside it, so one code point is matched once
    //     even when several threads want it. The cost falls on misses only.
    // DEBT: SharedFontProvider answers a miss with a synchronous IPC round trip on the client's
    //       connection, which belongs to the document thread. Until the render side has a font
    //       service connection of its own, only cache hits are genuinely available off it.
    MutexLocker locker(cache.mutex);
    if (auto cached = cache.fonts.get(key); cached.has_value())
        return *cached;
    RefPtr<Font const> font = FontDatabase::the().get_font_for_code_point(
        key.code_point, key.point_size, key.weight, key.width, key.slope, key.prefer_color_emoji);
    cache.fonts.set(key, font);
    return font;
}

void clear_system_fallback_font_cache()
{
    auto& cache = system_fallback_font_cache();
    MutexLocker locker(cache.mutex);
    cache.fonts.clear();
}

size_t system_fallback_font_cache_size()
{
    auto& cache = system_fallback_font_cache();
    MutexLocker locker(cache.mutex);
    return cache.fonts.size();
}

}
