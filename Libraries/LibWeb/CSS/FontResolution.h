/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

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
    FaceIsUnloaded = 1 << 3,
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

// `@font-feature-values` maps a family to the feature indices an author named for it, and it is
// document state the published table does not carry: the table would have to be rebuilt whenever
// a sheet's condition changed, and the style stage's requests never need it. `to_shape_features`
// reads the map only for `font-variant-alternates`, and the requests the stage builds carry no
// feature data at all. Callers that do - canvas, getComputedStyle - pass a provider; the resolver
// verifies it is never asked for one it was not given.
using FontFeatureValuesProvider = Function<HashMap<FontFeatureValueKey, Vector<u32>> const&(Utf16FlyString const&)>;

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
