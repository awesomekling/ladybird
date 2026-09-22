/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021, the SerenityOS developers.
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>
 * Copyright (c) 2025, Callum Law <callumlaw1709@outlook.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/ByteString.h>
#include <AK/Utf16FlyString.h>
#include <LibGC/CellAllocator.h>
#include <LibGfx/FontCascadeList.h>
#include <LibWeb/CSS/Fetch.h>
#include <LibWeb/CSS/FontFeatureData.h>
#include <LibWeb/CSS/Percentage.h>
#include <LibWeb/CSS/StyleValues/StyleValue.h>
#include <LibWeb/CSS/URL.h>
#include <LibWeb/DOM/DocumentLoadEventDelayer.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWebCommon/PixelUnits.h>

namespace Web::CSS {

class FontCascadeMemo;

struct FontWeightRange {
    int min { 0 };
    int max { 0 };
    [[nodiscard]] u32 hash() const { return pair_int_hash(min, max); }
    [[nodiscard]] bool operator==(FontWeightRange const&) const = default;
    [[nodiscard]] bool contains_inclusive(int weight) const { return min <= weight && weight <= max; }
};

struct FontFaceKey {
    Utf16FlyString family_name;
    FontWeightRange weight;
    int slope { 0 };
    int width { 100 };
    [[nodiscard]] u32 hash() const { return pair_int_hash(family_name.ascii_case_insensitive_hash(), pair_int_hash(weight.hash(), pair_int_hash(slope, width))); }
    [[nodiscard]] bool operator==(FontFaceKey const& other) const
    {
        return family_name.equals_ignoring_ascii_case(other.family_name)
            && weight == other.weight
            && slope == other.slope
            && width == other.width;
    }
};

enum class ComputedFontFamilySyntax {
    CustomIdent,
    String,
};

struct ComputedFontFamilyName {
    Utf16FlyString name;
    ComputedFontFamilySyntax syntax { ComputedFontFamilySyntax::CustomIdent };

    bool operator==(ComputedFontFamilyName const&) const = default;
};

using ComputedFontFamily = Variant<GenericFontFamily, ComputedFontFamilyName>;

struct ComputedFontCacheKey {
    u32 tree_scope { 0 };
    Vector<ComputedFontFamily> font_families;
    FontOpticalSizing font_optical_sizing;
    CSSPixels font_size;
    int font_slope;
    double font_weight;
    Percentage font_width;
    HashMap<Utf16FlyString, double> font_variation_settings;
    FontFeatureData font_feature_data;

    [[nodiscard]] bool operator==(ComputedFontCacheKey const& other) const = default;
};

using FontFeatureValuesTable = HashMap<Utf16FlyString, HashMap<FontFeatureValueKey, Vector<u32>>>;
using ScopedFontFeatureValuesTables = HashMap<u32, FontFeatureValuesTable>;

class FontLoader final : public GC::Cell {
    GC_CELL(FontLoader, GC::Cell);
    GC_DECLARE_ALLOCATOR(FontLoader);

public:
    using Source = Variant<Utf16FlyString, URL>;
    FontLoader(FontComputer&, RuleOrDeclaration, Vector<Source> sources, GC::Ptr<GC::Function<void(RefPtr<Gfx::Typeface const>)>> on_load = {});

    virtual ~FontLoader();

    void start_loading_next_source();

    bool is_loading() const;
    void did_request_for_rendering();
    bool may_finish_from_cache() const;
    bool has_started_request() const;
    bool has_received_font_data() const { return m_has_received_font_data; }

    void subscribe(GC::Ref<GC::Function<void(RefPtr<Gfx::Typeface const>)>>);

private:
    virtual void visit_edges(Visitor&) override;

    Optional<ByteString> try_load_font_mime_type_essence(Fetch::Infrastructure::Response const&, ByteBuffer const&);

    void font_did_load_or_fail(RefPtr<Gfx::Typeface const>);

    GC::Ref<FontComputer> m_font_computer;
    RuleOrDeclaration m_rule_or_declaration;
    RefPtr<Gfx::Typeface const> m_typeface;
    Vector<Source> m_sources;
    GC::Ptr<Fetch::Infrastructure::FetchController> m_fetch_controller;
    Vector<GC::Ref<GC::Function<void(RefPtr<Gfx::Typeface const>)>>> m_subscribers;
    Optional<DOM::DocumentLoadEventDelayer> m_document_load_event_delayer;
    bool m_has_completed { false };
    bool m_has_received_font_data { false };
};

class WEB_API FontComputer final : public GC::Cell {
    GC_CELL(FontComputer, GC::Cell);
    GC_DECLARE_ALLOCATOR(FontComputer);

public:
    FontComputer();
    explicit FontComputer(DOM::Document&);
    virtual ~FontComputer() override;

    DOM::Document& document() { return *m_document; }
    DOM::Document const& document() const { return *m_document; }

    Gfx::Font const& initial_font() const;
    bool should_defer_initial_paint();
    bool has_completed_initial_paint() const { return m_has_completed_initial_paint; }
    bool initial_paint_had_pending_fonts() const { return m_initial_paint_had_pending_fonts; }

    void clear_computed_font_cache(Utf16FlyString const& family_name);
    void clear_font_feature_values_cache(Utf16FlyString const& family_name);
    void invalidate_font_feature_values_snapshot();
    void did_load_font(Utf16FlyString const& family_name);
    void did_load_font(FontFaceKey const&);

    // A face's typeface becomes available one task before the font-loading task announces it, and
    // a resolution in between must see it rather than the pending face it replaces.
    void did_parse_font_face(FontFaceKey const& key) { did_load_font(key); }

    // The one funnel: every change to what a font resolution would answer passes through here.
    void bump_environment_generation();

    void register_font_face(NonnullRefPtr<FontFaceState>);
    void unregister_font_face(NonnullRefPtr<FontFaceState>);
    void synchronize_font_face_order(Vector<NonnullRefPtr<FontFaceState>> const&);

    GC::Ptr<FontLoader> load_font_face(ParsedFontFace const&, RefPtr<StyleSheetState>, GC::Ptr<GC::Function<void(RefPtr<Gfx::Typeface const>)>> on_load = {});

    void load_fonts_from_sheet(StyleSheetState&);
    void unload_fonts_from_sheet(StyleSheetState&);

    NonnullRefPtr<Gfx::FontCascadeList const> compute_font_for_style_values(Vector<ComputedFontFamily> font_families, CSSPixels const& font_size, int font_slope, double font_weight, Percentage const& font_width, FontOpticalSizing font_optical_sizing, HashMap<Utf16FlyString, double> const& font_variation_settings, FontFeatureData const& font_feature_data) const;
    NonnullRefPtr<Gfx::FontCascadeList const> compute_font_for_style_values(StyleValue const& font_family, CSSPixels const& font_size, int font_slope, double font_weight, Percentage const& font_width, FontOpticalSizing font_optical_sizing, HashMap<Utf16FlyString, double> const& font_variation_settings, FontFeatureData const& font_feature_data) const;
    u64 environment_generation() const { return m_environment_generation; }

    // The `@font-face` table as everything outside the document sees it: an immutable snapshot of
    // one font-environment generation, owned by an `Arc` on the Rust side. Rebuilt by the single
    // funnel that bumps the generation, and nowhere else.
    [[nodiscard]] void const* published_font_faces() const { return m_published_font_faces; }
    [[nodiscard]] FontCascadeMemo& font_cascade_memo() const { return *m_font_cascade_memo; }
    [[nodiscard]] ScopedFontFeatureValuesTables const& published_font_feature_values() const;

private:
    virtual void visit_edges(Visitor&) override;

    void begin_font_face_change_batch();
    void end_font_face_change_batch();
    void clear_computed_font_cache_for_families(Vector<Utf16FlyString> const& family_names);

    [[nodiscard]] void const* build_font_face_snapshot() const;
    void publish_font_faces();

    NonnullRefPtr<Gfx::FontCascadeList const> compute_font_for_style_values_impl(ReadonlySpan<ComputedFontFamily const> font_families, CSSPixels const& font_size, int font_slope, double font_weight, Percentage const& font_width, FontOpticalSizing font_optical_sizing, HashMap<Utf16FlyString, double> const& font_variation_settings, FontFeatureData const& font_feature_data) const;

    HashMap<FontFeatureValueKey, Vector<u32>> const& font_feature_values_for_family(Utf16FlyString const& family_name) const;

    GC::Ptr<DOM::Document> m_document;

    HashMap<FontFaceKey, Vector<NonnullRefPtr<FontFaceState>>> m_font_faces;
    HashMap<String, GC::Ref<FontLoader>> m_loaders_by_source;

    // Shared rather than owned: the style stage's between-pass batch fills this too.
    NonnullRefPtr<FontCascadeMemo> m_font_cascade_memo;
    mutable HashMap<Utf16FlyString, HashMap<FontFeatureValueKey, Vector<u32>>> m_font_feature_values_cache;
    mutable ScopedFontFeatureValuesTables m_published_font_feature_values;
    mutable bool m_font_feature_values_snapshot_dirty { true };

    bool m_has_completed_initial_paint { false };
    bool m_initial_paint_had_pending_fonts { false };
    u32 m_font_face_change_batch_depth { 0 };
    u64 m_environment_generation { 1 };
    Vector<Utf16FlyString> m_batched_font_face_change_families;

    // An owned `Arc<FontFaceSnapshot>` from the Rust style engine.
    void const* m_published_font_faces { nullptr };
};

}

namespace AK {

template<>
struct Traits<Web::CSS::FontFaceKey> : public DefaultTraits<Web::CSS::FontFaceKey> {
    static unsigned hash(Web::CSS::FontFaceKey const& key) { return key.hash(); }
};

template<>
struct Traits<Web::CSS::ComputedFontCacheKey> : public DefaultTraits<Web::CSS::ComputedFontCacheKey> {
    static unsigned hash(Web::CSS::ComputedFontCacheKey const& key)
    {
        unsigned hash = key.tree_scope;
        for (auto const& family : key.font_families) {
            if (family.has<Web::CSS::GenericFontFamily>()) {
                hash = pair_int_hash(hash, to_underlying(family.get<Web::CSS::GenericFontFamily>()));
            } else {
                auto const& name = family.get<Web::CSS::ComputedFontFamilyName>();
                hash = pair_int_hash(hash, pair_int_hash(name.name.hash(), to_underlying(name.syntax)));
            }
        }

        hash = pair_int_hash(hash, to_underlying(key.font_optical_sizing));
        hash = pair_int_hash(hash, Traits<Web::CSSPixels>::hash(key.font_size));
        hash = pair_int_hash(hash, key.font_slope);
        hash = pair_int_hash(hash, Traits<double>::hash(key.font_weight));
        hash = pair_int_hash(hash, Traits<double>::hash(key.font_width.value()));
        for (auto const& [variation_name, variation_value] : key.font_variation_settings)
            hash = pair_int_hash(hash, pair_int_hash(variation_name.hash(), Traits<double>::hash(variation_value)));
        hash = pair_int_hash(hash, Traits<Web::CSS::FontFeatureData>::hash(key.font_feature_data));

        return hash;
    }
};

}
