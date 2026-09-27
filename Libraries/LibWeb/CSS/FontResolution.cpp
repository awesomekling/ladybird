/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Math.h>
#include <AK/QuickSort.h>
#include <AK/Singleton.h>
#include <LibGfx/Font/FontDatabase.h>
#include <LibGfx/Font/SystemFallbackFonts.h>
#include <LibGfx/Font/Typeface.h>
#include <LibGfx/Font/TypefaceSkia.h>
#include <LibWeb/CSS/FontFaceState.h>
#include <LibWeb/CSS/FontResolution.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/Platform/FontPlugin.h>

extern "C" {
void ladybird_libweb_font_cascade_memo_ref(void const*);
void ladybird_libweb_font_cascade_memo_unref(void const*);
}

namespace Web::CSS {

extern "C" void rust_font_face_snapshot_view(void const*, FontFaceSnapshotView*);

NonnullRefPtr<Gfx::FontCascadeList const> resolve_font_for_style_values(FontComputer const& font_computer, ComputedFontCacheKey key)
{
    auto& memo = font_computer.font_cascade_memo();
    memo.publish_font_feature_values(font_computer.published_font_feature_values());
    FontFaceSnapshotView snapshot;
    rust_font_face_snapshot_view(font_computer.published_font_faces(), &snapshot);
    auto font_list = memo.resolve(snapshot, move(key));
    (void)request_wanted_web_faces();
    return font_list;
}

// The Rust side owns the storage these describe, so a layout difference would be silent.
static_assert(sizeof(FontFaceSnapshotKey) == 32 && alignof(FontFaceSnapshotKey) == 4);
static_assert(sizeof(FontFaceSnapshotRecord) == 32 && alignof(FontFaceSnapshotRecord) == 8);
static_assert(sizeof(FontFaceSnapshotRange) == 8 && alignof(FontFaceSnapshotRange) == 4);
static_assert(sizeof(FontFaceSnapshotView) == 72 && alignof(FontFaceSnapshotView) == 8);

static unsigned font_width_bucket_from_percentage(double percentage)
{
    // Maps a font-width Percentage to the nearest standard Gfx::FontWidth bucket.

    struct Bucket {
        double percentage;
        unsigned width;
    };
    static constexpr Array<Bucket, 9> buckets = { {
        { 50.0, Gfx::FontWidth::UltraCondensed },
        { 62.5, Gfx::FontWidth::ExtraCondensed },
        { 75.0, Gfx::FontWidth::Condensed },
        { 87.5, Gfx::FontWidth::SemiCondensed },
        { 100.0, Gfx::FontWidth::Normal },
        { 112.5, Gfx::FontWidth::SemiExpanded },
        { 125.0, Gfx::FontWidth::Expanded },
        { 150.0, Gfx::FontWidth::ExtraExpanded },
        { 200.0, Gfx::FontWidth::UltraExpanded },
    } };
    auto best = buckets[0];
    auto best_distance = AK::fabs(percentage - best.percentage);
    for (size_t i = 1; i < buckets.size(); ++i) {
        auto distance = AK::fabs(percentage - buckets[i].percentage);
        if (distance < best_distance) {
            best_distance = distance;
            best = buckets[i];
        }
    }
    return best.width;
}

static FlyString font_family_name_for_platform(Utf16View family_name)
{
    auto family_name_utf8 = MUST(family_name.to_utf8());
    return FlyString::from_utf8_without_validation(family_name_utf8.bytes());
}

#ifdef AK_OS_MACOS
static Optional<Gfx::SystemUIFontKind> macos_system_ui_font_kind_from_family_name(StringView family)
{
    if (family.is_one_of("-apple-system"sv, "-apple-system-font"sv, "-webkit-system-font"sv, "system-ui"sv, "ui-sans-serif"sv))
        return Gfx::SystemUIFontKind::System;
    if (family == "ui-serif"sv)
        return Gfx::SystemUIFontKind::Serif;
    if (family == "ui-monospace"sv)
        return Gfx::SystemUIFontKind::Monospace;
    if (family == "ui-rounded"sv)
        return Gfx::SystemUIFontKind::Rounded;
    return {};
}
#endif

static Singleton<HashMap<FontFeatureValueKey, Vector<u32>>> s_no_font_feature_values;

static void add_pending_face_from_snapshot(Gfx::FontCascadeList&, FontFaceSnapshotView const&, FontFaceSnapshotRecord const&, float point_size, Gfx::FontVariationSettings const&, Gfx::ShapeFeatures const&);

static Vector<Gfx::UnicodeRange> unicode_ranges_of(FontFaceSnapshotView const& snapshot, FontFaceSnapshotRecord const& record)
{
    Vector<Gfx::UnicodeRange> unicode_ranges;
    unicode_ranges.ensure_capacity(record.range_count);
    for (u32 index = 0; index < record.range_count; ++index) {
        auto const& range = snapshot.ranges[record.range_offset + index];
        unicode_ranges.unchecked_append(Gfx::UnicodeRange { range.first_code_point, range.last_code_point });
    }
    return unicode_ranges;
}

// What FontFaceState::font_with_point_size() answers, read out of the published table instead of
// out of the face. A face still waiting on its load becomes a pending entry carrying the display
// state the table recorded and the number the document knows the face by, so that wanting it is a
// message rather than a call into the document.
static RefPtr<Gfx::FontCascadeList const> font_for_face(FontFaceSnapshotView const& snapshot, FontFaceSnapshotRecord const& record, float point_size, Gfx::FontVariationSettings const& variations, Gfx::ShapeFeatures const& shape_features)
{
    if (record.flags & FaceIsUnusable)
        return {};
    auto font_list = Gfx::FontCascadeList::create();
    if (record.typeface != 0) {
        auto const& typeface = *reinterpret_cast<Gfx::Typeface const*>(record.typeface);
        font_list->add(typeface.font(point_size, variations, shape_features), unicode_ranges_of(snapshot, record));
    } else if (record.flags & FaceHasUrls) {
        add_pending_face_from_snapshot(font_list, snapshot, record, point_size, variations, shape_features);
    }
    if (font_list->is_empty())
        return {};
    return font_list;
}

// A face still waiting on its load: the entry carries the display state the table recorded and
// the number the document knows the face by, so that wanting it is a message.
static void add_pending_face_from_snapshot(Gfx::FontCascadeList& font_list, FontFaceSnapshotView const& snapshot, FontFaceSnapshotRecord const& record, float point_size, Gfx::FontVariationSettings const& variations, Gfx::ShapeFeatures const& shape_features)
{
    auto face_id = record.face_id;
    auto pending_state = static_cast<Gfx::PendingFontState>(record.pending_state);
    font_list.add_pending_face(
        unicode_ranges_of(snapshot, record),
        [face_id, pending_state] {
            // NB: Building the entry needs no document, but selecting the face for a rendered code
            //     point happens later and on the document thread, where resolving the face starts
            //     its fetch, arms its display-period timer and engages the load-event delayer. A
            //     style update is the one caller that cannot have that happen underneath it.
            if (web_face_loads_are_deferred()) {
                note_wanted_web_face(face_id, WantedWebFace::Render);
                return pending_state;
            }
            if (auto face = FontFaceState::with_id(face_id))
                return face->resolve_for_rendering();
            return pending_state;
        },
        [face_id, point_size, variations, shape_features]() -> RefPtr<Gfx::Font const> {
            // The published table said this face was pending, and while the update that read it
            // is running that stays the answer. Afterwards the face may have settled, and a
            // cascade the document still holds should say so.
            if (web_face_loads_are_deferred())
                return {};
            if (auto face = FontFaceState::with_id(face_id))
                return face->font_for_rendering(point_size, variations, shape_features);
            return {};
        },
        [pending_state] { return pending_state; });
}

// One candidate of the font-matching algorithm: either a run of `@font-face`s registered under
// one key of the published table, or a single system typeface.
struct MatchingFontCandidate {
    Optional<u32> key_index;
    FontWeightRange weight;
    int slope { 0 };
    unsigned width { Gfx::FontWidth::Normal };
    Gfx::Typeface const* system_typeface { nullptr };

    [[nodiscard]] RefPtr<Gfx::FontCascadeList const> font_with_point_size(FontFaceSnapshotView const& snapshot, float point_size, Gfx::FontVariationSettings const& variations, FontFeatureData const& font_feature_data, HashMap<FontFeatureValueKey, Vector<u32>> const& font_feature_values) const
    {
        auto const& shape_features = font_feature_data.to_shape_features(font_feature_values);

        if (system_typeface) {
            auto font_list = Gfx::FontCascadeList::create();
            font_list->add(system_typeface->font(point_size, variations, shape_features));
            return font_list;
        }

        if (!key_index.has_value())
            return {};
        auto const& key = snapshot.keys[*key_index];

        auto font_list = Gfx::FontCascadeList::create();
        for (u32 index = 0; index < key.record_count; ++index) {
            auto const& record = snapshot.records[key.first_record + index];
            // https://drafts.csswg.org/css-font-loading/#font-face-load
            // User agents can initiate font loads on their own, whenever they determine that a given font face is
            // necessary to render something on the page. When this happens, they must act as if they had called the
            // corresponding FontFace’s load() method described here.
            // NB: An unloaded face with no subsetting unicode-range starts loading once a style actually selects
            //     it, but the load itself mutates the face and the document. A resolution only leaves the face's
            //     number behind; request_wanted_web_faces() performs the load afterwards, and skips a face that
            //     is past "unloaded" already. The published table therefore says nothing about load status, which
            //     is one fewer thing that has to bump the font-environment generation.
            if ((record.flags & FaceHasUrls) && !(record.flags & FaceHasNonDefaultUnicodeRange))
                note_wanted_web_face(record.face_id, WantedWebFace::Load);
            if (auto face_fonts = font_for_face(snapshot, record, point_size, variations, shape_features)) {
                font_list->extend(*face_fonts);
                continue;
            }
            // Unloaded subset face: surface it as a pending entry so the fetch only
            // fires once font_for_code_point() sees a codepoint in its unicode-range.
            if ((record.flags & FaceHasUrls) && (record.flags & FaceHasNonDefaultUnicodeRange))
                add_pending_face_from_snapshot(font_list, snapshot, record, point_size, variations, shape_features);
        }
        if (font_list->is_empty())
            return {};
        return font_list;
    }
};

static RefPtr<Gfx::FontCascadeList const> find_matching_font_weight_ascending(FontFaceSnapshotView const& snapshot, Vector<MatchingFontCandidate> const& candidates, int target_weight, float font_size_in_pt, Gfx::FontVariationSettings const& variations, FontFeatureData const& font_feature_data, HashMap<FontFeatureValueKey, Vector<u32>> const& font_feature_values, bool inclusive)
{
    using Fn = AK::Function<bool(MatchingFontCandidate const&)>;
    auto pred = inclusive ? Fn([&](auto const& matching_font_candidate) { return matching_font_candidate.weight.min >= target_weight; })
                          : Fn([&](auto const& matching_font_candidate) { return matching_font_candidate.weight.min > target_weight; });
    auto it = find_if(candidates.begin(), candidates.end(), pred);
    for (; it != candidates.end(); ++it) {
        if (auto found_font = it->font_with_point_size(snapshot, font_size_in_pt, variations, font_feature_data, font_feature_values))
            return found_font;
    }
    return {};
}

static RefPtr<Gfx::FontCascadeList const> find_matching_font_weight_descending(FontFaceSnapshotView const& snapshot, Vector<MatchingFontCandidate> const& candidates, int target_weight, float font_size_in_pt, Gfx::FontVariationSettings const& variations, FontFeatureData const& font_feature_data, HashMap<FontFeatureValueKey, Vector<u32>> const& font_feature_values, bool inclusive)
{
    using Fn = AK::Function<bool(MatchingFontCandidate const&)>;
    auto pred = inclusive ? Fn([&](auto const& matching_font_candidate) { return matching_font_candidate.weight.max <= target_weight; })
                          : Fn([&](auto const& matching_font_candidate) { return matching_font_candidate.weight.max < target_weight; });
    auto it = find_if(candidates.rbegin(), candidates.rend(), pred);
    for (; it != candidates.rend(); ++it) {
        if (auto found_font = it->font_with_point_size(snapshot, font_size_in_pt, variations, font_feature_data, font_feature_values))
            return found_font;
    }
    return {};
}

// Partial implementation of the font-matching algorithm: https://www.w3.org/TR/css-fonts-4/#font-matching-algorithm
// FIXME: This should be replaced by the full CSS font selection algorithm.
static RefPtr<Gfx::FontCascadeList const> font_matching_algorithm(FontFaceSnapshotView const& snapshot, Utf16FlyString const& family_name, int weight, Percentage const& font_width, int slope, float font_size_in_pt, Gfx::FontVariationSettings const& variations, FontFeatureData const& font_feature_data, HashMap<FontFeatureValueKey, Vector<u32>> const& font_feature_values)
{
    // If a font family match occurs, the user agent assembles the set of font faces in that family and then
    // narrows the set to a single face using other font properties in the order given below.
    Vector<MatchingFontCandidate> matching_family_fonts;
    for (u32 key_index = 0; key_index < snapshot.key_count; ++key_index) {
        auto const& key = snapshot.keys[key_index];
        if (!snapshot.family_name(key).equals_ignoring_ascii_case(family_name.view()))
            continue;
        matching_family_fonts.append({
            .key_index = key_index,
            .weight = { key.weight_min, key.weight_max },
            .slope = key.slope,
            .width = font_width_bucket_from_percentage(key.width),
        });
    }
    if (matching_family_fonts.is_empty()) {
        Gfx::FontDatabase::the().for_each_typeface_with_family_name(font_family_name_for_platform(family_name.view()), [&](Gfx::Typeface const& typeface) {
            matching_family_fonts.append({
                .key_index = {},
                // FIXME: Support system fonts that have a range of weights, etc.
                .weight = { static_cast<int>(typeface.weight()), static_cast<int>(typeface.weight()) },
                .slope = typeface.slope(),
                .width = typeface.width(),
                .system_typeface = &typeface,
            });
        });
    }

    if (matching_family_fonts.is_empty())
        return {};

    // 1. font-width is tried first.
    auto desired_width = font_width_bucket_from_percentage(font_width.value());
    auto width_it = find_if(matching_family_fonts.begin(), matching_family_fonts.end(),
        [&](auto const& matching_font_candidate) { return matching_font_candidate.width == desired_width; });
    if (width_it != matching_family_fonts.end()) {
        matching_family_fonts.remove_all_matching([&](auto const& matching_font_candidate) {
            return matching_font_candidate.width != desired_width;
        });
    }

    quick_sort(matching_family_fonts, [](auto const& a, auto const& b) {
        return a.weight.min < b.weight.min;
    });
    // 2. font-style is tried next.
    // We don't have complete support of italic and oblique fonts, so matching on font-style can be simplified to:
    // If a matching slope is found, all faces which don't have that matching slope are excluded from the matching set.
    auto style_it = find_if(matching_family_fonts.begin(), matching_family_fonts.end(),
        [&](auto const& matching_font_candidate) { return matching_font_candidate.slope == slope; });
    if (style_it != matching_family_fonts.end()) {
        matching_family_fonts.remove_all_matching([&](auto const& matching_font_candidate) {
            return matching_font_candidate.slope != slope;
        });
    }
    // 3. font-weight is matched next.
    // If a font does not have any concept of varying strengths of weights, its weight is mapped according list in the
    // property definition. If bolder/lighter relative weights are used, the effective weight is calculated based on
    // the inherited weight value, as described in the definition of the font-weight property.
    // FIXME: "varying strengths of weights"
    // If the matching set after performing the steps above includes faces with weight values containing the
    // font-weight desired value, faces with weight values which do not include the desired font-weight value are
    // removed from the matching set.

    // FIXME: This whole function currently just returns the first match instead of progressing further, so we'll do that here too.
    auto matching_weight_it = matching_family_fonts.find_if([weight](auto const& candidate) {
        return candidate.weight.contains_inclusive(weight);
    });
    for (; matching_weight_it != matching_family_fonts.end(); ++matching_weight_it) {
        if (auto found_font = matching_weight_it->font_with_point_size(snapshot, font_size_in_pt, variations, font_feature_data, font_feature_values))
            return found_font;
    }

    // If there is no face which contains the desired value, a weight value is chosen using the rules below:

    // - If the desired weight is inclusively between 400 and 500, weights greater than or equal to the target weight
    //   are checked in ascending order until 500 is hit and checked, followed by weights less than the target weight
    //   in descending order, followed by weights greater than 500, until a match is found.
    if (weight >= 400 && weight <= 500) {
        auto it = find_if(matching_family_fonts.begin(), matching_family_fonts.end(),
            [&](auto const& matching_font_candidate) { return matching_font_candidate.weight.min >= weight; });
        for (; it != matching_family_fonts.end() && it->weight.min <= 500; ++it) {
            if (auto found_font = it->font_with_point_size(snapshot, font_size_in_pt, variations, font_feature_data, font_feature_values))
                return found_font;
        }
        if (auto found_font = find_matching_font_weight_descending(snapshot, matching_family_fonts, weight, font_size_in_pt, variations, font_feature_data, font_feature_values, false))
            return found_font;
        for (; it != matching_family_fonts.end(); ++it) {
            if (auto found_font = it->font_with_point_size(snapshot, font_size_in_pt, variations, font_feature_data, font_feature_values))
                return found_font;
        }
    }
    // - If the desired weight is less than 400, weights less than or equal to the desired weight are checked in
    //   descending order followed by weights above the desired weight in ascending order until a match is found.
    if (weight < 400) {
        if (auto found_font = find_matching_font_weight_descending(snapshot, matching_family_fonts, weight, font_size_in_pt, variations, font_feature_data, font_feature_values, true))
            return found_font;
        if (auto found_font = find_matching_font_weight_ascending(snapshot, matching_family_fonts, weight, font_size_in_pt, variations, font_feature_data, font_feature_values, false))
            return found_font;
    }
    // - If the desired weight is greater than 500, weights greater than or equal to the desired weight are checked in
    //   ascending order followed by weights below the desired weight in descending order until a match is found.
    if (weight > 500) {
        if (auto found_font = find_matching_font_weight_ascending(snapshot, matching_family_fonts, weight, font_size_in_pt, variations, font_feature_data, font_feature_values, true))
            return found_font;
        if (auto found_font = find_matching_font_weight_descending(snapshot, matching_family_fonts, weight, font_size_in_pt, variations, font_feature_data, font_feature_values, false))
            return found_font;
    }

    return {};
}

NonnullRefPtr<Gfx::FontCascadeList const> FontCascadeMemo::resolve(FontFaceSnapshotView const& snapshot, ComputedFontCacheKey key, FontFeatureValuesProvider const* font_feature_values_provider)
{
    // A tree scope reaches the resolution only through the @font-feature-values it publishes, and only a request whose
    // font-variant-alternates names feature values reads those. Any other request resolves the same in every tree scope,
    // so it is remembered once for all of them. That keeps one cascade for equal fonts in different shadow trees, which
    // is what lets their elements' styles, and the styles of everything inheriting from them, compare equal and share.
    if (!key.font_feature_data.font_variant_alternates.has_value() || key.font_feature_data.font_variant_alternates->font_feature_value_entries.is_empty())
        key.tree_scope = 0;

    MutexLocker locker { m_mutex };
    auto it = m_cascades.find(key);
    if (it != m_cascades.end() && it->value.generation == snapshot.generation)
        return it->value.font_list;

    FontFeatureValuesProvider published_provider = [this, &key](Utf16FlyString const& family) -> HashMap<FontFeatureValueKey, Vector<u32>> const& {
        auto scope = m_font_feature_values.find(key.tree_scope);
        if (scope == m_font_feature_values.end())
            return *s_no_font_feature_values;
        auto it = scope->value.find(family);
        return it == scope->value.end() ? *s_no_font_feature_values : it->value;
    };
    auto font_list = resolve_font_cascade(snapshot, key.font_families.span(), key.font_size, key.font_slope, key.font_weight, key.font_width, key.font_optical_sizing, key.font_variation_settings, key.font_feature_data, font_feature_values_provider ? font_feature_values_provider : &published_provider);
    if (it != m_cascades.end()) {
        // OPTIMIZATION: An answer that a newer table did not change keeps its identity, so that
        //               every element holding it keeps holding the same cascade.
        if (font_list->equals(*it->value.font_list)) {
            it->value.generation = snapshot.generation;
            return it->value.font_list;
        }
        it->value = Entry { snapshot.generation, font_list };
        return font_list;
    }
    m_cascades.set(move(key), Entry { snapshot.generation, font_list });
    return font_list;
}

void FontCascadeMemo::publish_font_feature_values(ScopedFontFeatureValuesTables const& values)
{
    MutexLocker locker { m_mutex };
    if (m_font_feature_values == values)
        return;
    m_font_feature_values = values;
    m_cascades.clear();
}

void FontCascadeMemo::take_matching(Function<bool(ComputedFontCacheKey const&, Gfx::FontCascadeList const&)> const& is_stale)
{
    MutexLocker locker { m_mutex };
    m_cascades.remove_all_matching([&](auto const& key, auto const& entry) {
        return is_stale(key, *entry.font_list);
    });
}

void FontCascadeMemo::take_changed(FontFaceSnapshotView const& snapshot, Function<bool(ComputedFontCacheKey const&)> const& might_change, Function<void(Gfx::FontCascadeList const&)> const& did_change)
{
    MutexLocker locker { m_mutex };
    m_cascades.remove_all_matching([&](auto const& key, auto const& entry) {
        if (!might_change(key))
            return false;
        FontFeatureValuesProvider published_provider = [this, &key](Utf16FlyString const& family) -> HashMap<FontFeatureValueKey, Vector<u32>> const& {
            auto scope = m_font_feature_values.find(key.tree_scope);
            if (scope == m_font_feature_values.end())
                return *s_no_font_feature_values;
            auto it = scope->value.find(family);
            return it == scope->value.end() ? *s_no_font_feature_values : it->value;
        };
        auto updated_font_list = resolve_font_cascade(snapshot, key.font_families.span(), key.font_size, key.font_slope, key.font_weight, key.font_width, key.font_optical_sizing, key.font_variation_settings, key.font_feature_data, &published_provider);
        if (!entry.font_list->has_pending_faces() && entry.font_list->equals(*updated_font_list))
            return false;
        did_change(*entry.font_list);
        return true;
    });
}

// Reads the engine's value data without wrapping it in a StyleValue, so the style stage's font
// batch can call this on any thread.
Vector<ComputedFontFamily> computed_font_families_from_value_data(StyleValueFFI::StyleValueData const& font_family)
{
    auto count = StyleValueFFI::rust_style_value_copy_computed_font_families(&font_family, nullptr, 0);
    Vector<StyleValueFFI::FfiComputedFontFamilyEntry> entries;
    entries.resize(count);
    VERIFY(StyleValueFFI::rust_style_value_copy_computed_font_families(&font_family, entries.data(), entries.size()) == count);

    Vector<ComputedFontFamily> families;
    families.ensure_capacity(count);
    for (auto const& entry : entries) {
        if (entry.kind == StyleValueFFI::COMPUTED_FONT_FAMILY_GENERIC) {
            auto family = keyword_to_generic_font_family(static_cast<Keyword>(entry.keyword));
            VERIFY(family.has_value());
            families.unchecked_append(family.release_value());
            continue;
        }
        VERIFY(entry.kind == StyleValueFFI::COMPUTED_FONT_FAMILY_CUSTOM_IDENT || entry.kind == StyleValueFFI::COMPUTED_FONT_FAMILY_STRING);
        families.unchecked_append(ComputedFontFamilyName {
            .name = css_string_from_rust(entry.string),
            .syntax = entry.kind == StyleValueFFI::COMPUTED_FONT_FAMILY_STRING
                ? ComputedFontFamilySyntax::String
                : ComputedFontFamilySyntax::CustomIdent,
        });
    }
    return families;
}

NonnullRefPtr<Gfx::FontCascadeList const> resolve_font_cascade(FontFaceSnapshotView const& snapshot, ReadonlySpan<ComputedFontFamily const> font_families, CSSPixels const& font_size, int slope, double font_weight, Percentage const& font_width, FontOpticalSizing font_optical_sizing, HashMap<Utf16FlyString, double> const& font_variation_settings, FontFeatureData const& font_feature_data, FontFeatureValuesProvider const* font_feature_values_provider)
{
    // FIXME: We round to int here as that is what is expected by our font infrastructure below
    auto weight = round_to<int>(font_weight);

    // FIXME: We need to respect `font-size-adjust` once that is implemented.
    auto font_size_used_value = font_size.to_float();

    Gfx::FontVariationSettings variation;
    variation.set_weight(font_weight);
    variation.set_width(font_width.value());

    // NB: The spec recommends that we use the 'used value' of font-size for 'opsz' when font-optical-sizing is 'auto'.
    // FIXME: User agents must not select a value for the "opsz" axis which is not supported by the font used for
    //        rendering the text. This can be accomplished by clamping a chosen value to the range supported by the
    //        font. https://drafts.csswg.org/css-fonts/#font-optical-sizing-def
    if (font_optical_sizing == FontOpticalSizing::Auto)
        variation.set_optical_sizing(font_size_used_value);

    for (auto const& [tag_string, value] : font_variation_settings) {
        auto tag = open_type_tag_to_four_cc(tag_string);
        if (!tag.has_value())
            continue;

        variation.axes.set(*tag, value);
    }

    // FIXME: Implement the full font-matching algorithm: https://www.w3.org/TR/css-fonts-4/#font-matching-algorithm
    float const font_size_in_pt = font_size_used_value * 0.75f;

    auto font_feature_values_for_family = [&](Utf16FlyString const& family) -> HashMap<FontFeatureValueKey, Vector<u32>> const& {
        if (font_feature_values_provider)
            return (*font_feature_values_provider)(family);
        // Other callers can omit a provider if they have no named feature values.
        VERIFY(!font_feature_data.font_variant_alternates.has_value()
            || font_feature_data.font_variant_alternates->font_feature_value_entries.is_empty());
        return *s_no_font_feature_values;
    };

#ifdef AK_OS_MACOS
    auto find_macos_system_ui_font = [&](Gfx::SystemUIFontKind kind, Utf16FlyString const& family) -> RefPtr<Gfx::FontCascadeList const> {
        auto const& font_feature_values = font_feature_values_for_family(family);
        auto shape_features = font_feature_data.to_shape_features(font_feature_values);
        auto typeface = Gfx::TypefaceSkia::match_system_ui(kind, font_size_used_value, weight, font_width_bucket_from_percentage(font_width.value()), slope);
        if (typeface.is_error() || !typeface.value())
            return {};

        auto font_list = Gfx::FontCascadeList::create();
        font_list->add(typeface.value()->font(font_size_in_pt, variation, shape_features));
        return font_list;
    };
#endif

    auto find_font = [&](Utf16FlyString const& family) -> RefPtr<Gfx::FontCascadeList const> {
        auto const& font_feature_values = font_feature_values_for_family(family);

        // OPTIMIZATION: Look for an exact match in loaded fonts first.
        // FIXME: Respect the other font-* descriptors
        for (u32 key_index = 0; key_index < snapshot.key_count; ++key_index) {
            auto const& key = snapshot.keys[key_index];
            if (key.weight_min != weight || key.weight_max != weight || key.slope != slope || key.width != static_cast<int>(font_width.value()))
                continue;
            if (!snapshot.family_name(key).equals_ignoring_ascii_case(family.view()))
                continue;
            auto shape_features = font_feature_data.to_shape_features(font_feature_values);
            auto result = Gfx::FontCascadeList::create();
            for (u32 index = 0; index < key.record_count; ++index) {
                if (auto face_fonts = font_for_face(snapshot, snapshot.records[key.first_record + index], font_size_in_pt, variation, shape_features))
                    result->extend(*face_fonts);
            }
            if (!result->is_empty())
                return result;
            break;
        }

#ifdef AK_OS_MACOS
        auto platform_family = font_family_name_for_platform(family.view());
        if (auto system_ui_font_kind = macos_system_ui_font_kind_from_family_name(platform_family.bytes_as_string_view()); system_ui_font_kind.has_value()) {
            if (auto system_font = find_macos_system_ui_font(system_ui_font_kind.value(), family))
                return system_font;
        }
#endif

        if (auto found_font = font_matching_algorithm(snapshot, family, weight, font_width, slope, font_size_in_pt, variation, font_feature_data, font_feature_values); found_font && !found_font->is_empty())
            return found_font;

        return {};
    };

    auto find_generic_font = [&](GenericFontFamily family) -> RefPtr<Gfx::FontCascadeList const> {
        auto font_id = to_keyword(family);
#ifdef AK_OS_MACOS
        if (auto system_ui_font_kind = macos_system_ui_font_kind_from_family_name(string_from_keyword(font_id)); system_ui_font_kind.has_value()) {
            auto family = utf16_fly_string_from_keyword(font_id);
            if (auto system_font = find_macos_system_ui_font(system_ui_font_kind.value(), family))
                return system_font;
        }
#endif

        Platform::GenericFont generic_font {};
        switch (font_id) {
        case Keyword::Monospace:
            generic_font = Platform::GenericFont::Monospace;
            break;
        case Keyword::UiMonospace:
            generic_font = Platform::GenericFont::UiMonospace;
            break;
        case Keyword::Serif:
            generic_font = Platform::GenericFont::Serif;
            break;
        case Keyword::Fantasy:
            generic_font = Platform::GenericFont::Fantasy;
            break;
        case Keyword::SansSerif:
            generic_font = Platform::GenericFont::SansSerif;
            break;
        case Keyword::UiSerif:
            generic_font = Platform::GenericFont::UiSerif;
            break;
        case Keyword::UiRounded:
            generic_font = Platform::GenericFont::UiRounded;
            break;
        case Keyword::SystemUi:
        case Keyword::UiSansSerif:
            generic_font = Platform::GenericFont::UiSansSerif;
            break;
        case Keyword::Cursive:
            generic_font = Platform::GenericFont::Cursive;
            break;
        default:
            return {};
        }
        return find_font(Utf16FlyString::from_fly_string(Platform::FontPlugin::the().generic_font_name(generic_font, weight, slope)));
    };

    auto font_list = Gfx::FontCascadeList::create();

    for (auto const& family : font_families) {
        RefPtr<Gfx::FontCascadeList const> other_font_list;
        if (family.has<GenericFontFamily>()) {
            other_font_list = find_generic_font(family.get<GenericFontFamily>());
        } else {
            other_font_list = find_font(family.get<ComputedFontFamilyName>().name);
        }

        if (other_font_list)
            font_list->extend(*other_font_list);
    }

    // NB: @font-feature-values can't apply to the default font since it's not loaded from CSS
    auto default_font = Platform::FontPlugin::the().default_font(font_size_in_pt, variation, font_feature_data.to_shape_features({}));
    if (font_list->is_empty()) {
        if (auto fallback_font_list = find_font(Utf16FlyString::from_fly_string(Platform::FontPlugin::the().generic_font_name(Platform::GenericFont::UiSansSerif, weight, slope))))
            font_list->extend(*fallback_font_list);
    }
    if (font_list->is_empty()) {
        // This is needed to make sure we check default font before reaching to emojis.
        font_list->add(*default_font);
    }

    // The default font is already included in the font list, but we explicitly set it
    // as the last-resort font. This ensures that if none of the specified fonts contain
    // the requested code point, there is still a font available to provide a fallback glyph.
    font_list->set_last_resort_font(*default_font);

    if (Platform::FontPlugin::the().is_layout_test_mode()) {
        for (auto font_name : Platform::FontPlugin::the().symbol_font_names()) {
            if (auto other_font_list = find_font(Utf16FlyString::from_fly_string(font_name)))
                font_list->extend_fallback(*other_font_list);
        }
    } else {
        font_list->set_system_font_fallback_callback([](u32 code_point, Gfx::EmojiPresentation presentation, Gfx::Font const& reference_font) -> RefPtr<Gfx::Font const> {
            return Gfx::system_fallback_font({
                .code_point = code_point,
                .weight = static_cast<u16>(reference_font.weight()),
                .width = reference_font.typeface().width(),
                .slope = static_cast<u8>(reference_font.slope()),
                .prefer_color_emoji = presentation == Gfx::EmojiPresentation::Emoji,
                .point_size = reference_font.point_size(),
            });
        });
    }

    // The cascade is complete. Freeze it here, on the document thread, so that every render pass
    // that receives it reads a snapshot instead of the live list.
    font_list->freeze();

    return font_list;
}

}

// The style engine holds the memo by address, so that the handle it keeps is plain data.
extern "C" void ladybird_libweb_font_cascade_memo_ref(void const* memo)
{
    VERIFY(memo);
    static_cast<Web::CSS::FontCascadeMemo const*>(memo)->ref();
}

extern "C" void ladybird_libweb_font_cascade_memo_unref(void const* memo)
{
    VERIFY(memo);
    static_cast<Web::CSS::FontCascadeMemo const*>(memo)->unref();
}
