/*
 * Copyright (c) 2023-2024, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Array.h>
#include <AK/Function.h>
#include <AK/RefCounted.h>
#include <LibGfx/Font/Font.h>
#include <LibGfx/Font/UnicodeRange.h>

namespace Gfx {

enum class EmojiPresentation : u8 {
    Text,
    Emoji,
};

enum class ForcedPresentation : u8 {
    No,
    Yes,
};

struct EmojiPresentationResult {
    EmojiPresentation presentation { EmojiPresentation::Text };
    ForcedPresentation forced { ForcedPresentation::No };
};

struct FontCascadeMetrics {
    u64 generation { 0 };
    Font const* first_available_font { nullptr };
    float ascent { 0 };
    float descent { 0 };
    float x_height { 0 };
};

EmojiPresentationResult emoji_presentation_for_code_point(u32 code_point, Optional<u32> next_code_point);

class FontCascadeList : public RefCounted<FontCascadeList> {
public:
    using SystemFontFallbackCallback = Function<RefPtr<Font const>(u32, EmojiPresentation, Font const&)>;

    static NonnullRefPtr<FontCascadeList> create()
    {
        return adopt_ref(*new FontCascadeList());
    }

    bool is_empty() const { return m_fonts.is_empty() && m_pending_faces.is_empty() && !m_last_resort_font; }
    bool has_pending_faces() const { return !m_pending_faces.is_empty(); }
    Font const& first() const { return !m_fonts.is_empty() ? *m_fonts.first().font : *m_last_resort_font; }

    template<typename Callback>
    void for_each_font_entry(Callback callback) const
    {
        for (auto const& font : m_fonts)
            callback(font);
    }

    void add(NonnullRefPtr<Font const> font);
    void add(NonnullRefPtr<Font const> font, Vector<UnicodeRange> unicode_ranges);

    // Register an unloaded face covering `unicode_ranges`. The cascade invokes
    // `start_load` the first time a rendered codepoint falls within one of the ranges.
    void add_pending_face(Vector<UnicodeRange> unicode_ranges, Function<void()> start_load);

    void extend(FontCascadeList const& other);

    void extend_fallback(FontCascadeList const& other);

    // Replace the resolved environment while preserving this cascade's identity for computed styles.
    void update_from(FontCascadeList&&) const;
    void release_retired_fonts() const { m_retired_fonts.clear(); }
    u64 generation() const { return m_generation; }

    Gfx::Font const& first_available_font() const;
    FontCascadeMetrics const& metrics() const;
    Gfx::Font const& font_for_code_point(u32 code_point, EmojiPresentationResult = {}) const;

    bool equals(FontCascadeList const& other) const;

    struct Entry {
        NonnullRefPtr<Font const> font;
        struct RangeData {
            // The enclosing range is the union of all Unicode ranges. Used for fast skipping.
            UnicodeRange enclosing_range;

            Vector<UnicodeRange> unicode_ranges;
        };
        Optional<RangeData> range_data;
    };

    class PendingFace : public RefCounted<PendingFace> {
    public:
        PendingFace(UnicodeRange enclosing, Vector<UnicodeRange> ranges, Function<void()> start_load)
            : m_enclosing_range(enclosing)
            , m_unicode_ranges(move(ranges))
            , m_start_load(move(start_load))
        {
        }

        bool covers(u32 code_point) const
        {
            if (!m_enclosing_range.contains(code_point))
                return false;
            for (auto const& range : m_unicode_ranges) {
                if (range.contains(code_point))
                    return true;
            }
            return false;
        }

        void start_load() { m_start_load(); }

    private:
        UnicodeRange m_enclosing_range;
        Vector<UnicodeRange> m_unicode_ranges;
        Function<void()> m_start_load;
    };

    void set_last_resort_font(NonnullRefPtr<Font> font)
    {
        m_first_available_font_cache = nullptr;
        m_metrics.generation = 0;
        m_last_resort_font = move(font);
    }
    void set_system_font_fallback_callback(SystemFontFallbackCallback callback) { m_system_font_fallback_callback = move(callback); }

private:
    mutable RefPtr<Font const> m_last_resort_font;
    mutable Vector<Entry> m_fonts;
    mutable Vector<Entry> m_fallback_fonts;
    mutable Vector<NonnullRefPtr<PendingFace>> m_pending_faces;
    mutable SystemFontFallbackCallback m_system_font_fallback_callback;

    // Layout snapshots may still hold raw pointers from an earlier generation until the next relayout.
    mutable Vector<NonnullRefPtr<Font const>> m_retired_fonts;
    mutable u64 m_generation { 1 };

    // OPTIMIZATION: Cache of resolved fonts for ASCII code points. Since m_fonts only grows and the cascade returns
    //               the first matching font, a cached hit can never become stale.
    mutable Array<Font const*, 128> m_ascii_cache {};

    // This cannot share m_ascii_cache because the first available font does not need to contain a space glyph.
    mutable Font const* m_first_available_font_cache { nullptr };
    mutable FontCascadeMetrics m_metrics;
};

}
