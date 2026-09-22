/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/AtomicRefCounted.h>
#include <AK/Mutex.h>
#include <LibWeb/CSS/FontComputer.h>

namespace Web::CSS {

// The document's `@font-face` table, published: plain data with no way back to the document, so
// that resolving a font from it needs nothing but this table and the process-wide font services.
// NB: These three mirror `FfiFontFaceKey`, `FfiFontFaceRecord` and `FfiFontFaceRange` in
//     `css/style/font_faces.rs` field for field. The Rust side owns the storage; this is the view.

// One entry of the table: a `FontFaceKey` and the run of faces registered under it, in order.
struct FontFaceSnapshotKey {
    u32 family_offset { 0 };
    u32 family_length { 0 };
    i32 weight_min { 0 };
    i32 weight_max { 0 };
    i32 slope { 0 };
    i32 width { 0 };
    u32 first_record { 0 };
    u32 record_count { 0 };
};

struct FontFaceSnapshotRecord {
    u64 face_id { 0 };
    // The address of the loaded `Gfx::Typeface`, or zero while the face is pending.
    u64 typeface { 0 };
    u32 range_offset { 0 };
    u32 range_count { 0 };
    // `Gfx::PendingFontState`, peeked when the snapshot was built.
    u8 pending_state { 0 };
    u8 flags { 0 };
    u8 padding[6] { 0, 0, 0, 0, 0, 0 };
};

enum FontFaceSnapshotFlags : u8 {
    FaceHasUrls = 1 << 0,
    FaceIsUnusable = 1 << 1,
    FaceHasNonDefaultUnicodeRange = 1 << 2,
};

struct FontFaceSnapshotRange {
    u32 first_code_point { 0 };
    u32 last_code_point { 0 };
};

struct FontFaceSnapshotView {
    FontFaceSnapshotKey const* keys { nullptr };
    size_t key_count { 0 };
    FontFaceSnapshotRecord const* records { nullptr };
    size_t record_count { 0 };
    char16_t const* family_text { nullptr };
    size_t family_text_length { 0 };
    FontFaceSnapshotRange const* ranges { nullptr };
    size_t range_count { 0 };
    u64 generation { 0 };

    [[nodiscard]] Utf16View family_name(FontFaceSnapshotKey const& key) const
    {
        return Utf16View { family_text + key.family_offset, key.family_length };
    }
};

// The font service reads a detached copy of the document's `@font-feature-values` table. Callers
// outside the style stage can still supply their own provider.
using FontFeatureValuesProvider = Function<HashMap<FontFeatureValueKey, Vector<u32>> const&(Utf16FlyString const&)>;

// The `font-family` list, as the matcher wants it: generic families kept apart from names, and a
// name's syntax kept so that a custom ident and a string do not compare equal.
[[nodiscard]] Vector<ComputedFontFamily> computed_font_families_from_style_value(StyleValue const& font_family);

// The cascades already resolved from a document's `@font-face` tables. This is retained render
// state, not document state: a memo of a pure function of the published table and the request.
// It is shared rather than owned by the font computer, because the style stage's between-pass
// batch fills it too, and that batch is meant to run off the document thread; its lock is what
// makes that safe. The document reads it back to find the cascades a change to the table makes
// stale, which is the one thing that needs the whole history rather than one generation of it.
class WEB_API FontCascadeMemo : public AtomicRefCounted<FontCascadeMemo> {
public:
    static NonnullRefPtr<FontCascadeMemo> create() { return adopt_ref(*new FontCascadeMemo); }

    [[nodiscard]] NonnullRefPtr<Gfx::FontCascadeList const> resolve(FontFaceSnapshotView const&, ComputedFontCacheKey const&, FontFeatureValuesProvider const* = nullptr);
    void publish_font_feature_values(ScopedFontFeatureValuesTables const&);

    // Answers every remembered resolution, so the caller can decide which a change to the table
    // has made stale, and forgets the ones it says so about.
    void take_matching(Function<bool(ComputedFontCacheKey const&, Gfx::FontCascadeList const&)> const&);

private:
    FontCascadeMemo() = default;

    struct Entry {
        // The font-environment generation of the table this answer came from. The published table
        // an update resolves against can be older than the document's current one, so an answer
        // has to name the table that produced it rather than stand for every later one.
        u64 generation { 0 };
        NonnullRefPtr<Gfx::FontCascadeList const> font_list;
    };

    Mutex m_mutex;
    HashMap<ComputedFontCacheKey, Entry> m_cascades;
    ScopedFontFeatureValuesTables m_font_feature_values;
};

// Resolve a font cascade from the published table and the process-wide font services alone. This
// is the whole of what the style stage's between-pass font batch does.
[[nodiscard]] NonnullRefPtr<Gfx::FontCascadeList const> resolve_font_cascade(
    FontFaceSnapshotView const&,
    ReadonlySpan<ComputedFontFamily const> font_families,
    CSSPixels const& font_size,
    int font_slope,
    double font_weight,
    Percentage const& font_width,
    FontOpticalSizing,
    HashMap<Utf16FlyString, double> const& font_variation_settings,
    FontFeatureData const&,
    FontFeatureValuesProvider const* = nullptr);

}
