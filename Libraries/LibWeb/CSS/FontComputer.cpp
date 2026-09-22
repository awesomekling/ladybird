/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021, the SerenityOS developers.
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>
 * Copyright (c) 2025, Callum Law <callumlaw1709@outlook.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include "FontComputer.h"
#include <AK/Platform.h>
#include <AK/ScopeGuard.h>
#include <LibGC/RootHashTable.h>
#include <LibGfx/Font/Font.h>
#include <LibGfx/Font/FontDatabase.h>
#include <LibGfx/Font/SystemFallbackFonts.h>
#include <LibGfx/Font/TypefaceSkia.h>
#include <LibWeb/CSS/Fetch.h>
#include <LibWeb/CSS/FontFaceSet.h>
#include <LibWeb/CSS/FontFaceState.h>
#include <LibWeb/CSS/FontLoading.h>
#include <LibWeb/CSS/FontResolution.h>
#include <LibWeb/CSS/RustFontFeatureValues.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEngineBridge.h>
#include <LibWeb/CSS/StyleSheetState.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/Fetch/Infrastructure/HTTP/MIME.h>
#include <LibWeb/Fetch/Response.h>
#include <LibWeb/MimeSniff/Resource.h>
#include <LibWeb/Platform/FontPlugin.h>
#include <LibWeb/StyleValueRustFFI.h>

extern "C" {
void const* rust_font_face_snapshot_build(Web::CSS::FontFaceSnapshotView const*);
void rust_font_face_snapshot_release(void const*);
void rust_font_face_snapshot_view(void const*, Web::CSS::FontFaceSnapshotView*);
}

namespace Web::CSS {

GC_DEFINE_ALLOCATOR(FontComputer);
GC_DEFINE_ALLOCATOR(FontLoader);

}

namespace Web::CSS {

FontComputer::FontComputer() = default;

FontComputer::FontComputer(DOM::Document& document)
    : m_document(document)
    , m_font_cascade_memo(FontCascadeMemo::create())
{
    publish_font_faces();
}

FontComputer::~FontComputer()
{
    rust_font_face_snapshot_release(m_published_font_faces);
}

// Every change to what a font resolution would answer passes through here: a new generation, and
// the table republished for it. Nothing else may touch m_environment_generation.
void FontComputer::bump_environment_generation()
{
    ++m_environment_generation;
    publish_font_faces();
    // A style update holds the table it was given for its whole length, so a change made while one
    // is running has to reach it here or the rest of that update resolves against a table the
    // document has already left behind.
    document().style_computer().style_engine().publish_font_faces();
}

void const* FontComputer::build_font_face_snapshot() const
{
    Vector<FontFaceSnapshotKey> keys;
    Vector<FontFaceSnapshotRecord> records;
    Vector<char16_t> family_text;
    Vector<FontFaceSnapshotRange> ranges;
    keys.ensure_capacity(m_font_faces.size());

    for (auto const& [key, faces] : m_font_faces) {
        auto family = key.family_name.view();
        auto family_offset = family_text.size();
        for (size_t index = 0; index < family.length_in_code_units(); ++index)
            family_text.append(family.code_unit_at(index));
        keys.append({
            .family_offset = static_cast<u32>(family_offset),
            .family_length = static_cast<u32>(family.length_in_code_units()),
            .weight_min = key.weight.min,
            .weight_max = key.weight.max,
            .slope = key.slope,
            .width = key.width,
            .first_record = static_cast<u32>(records.size()),
            .record_count = static_cast<u32>(faces.size()),
        });
        for (auto const& face : faces) {
            auto range_offset = ranges.size();
            for (auto const& range : face->unicode_ranges())
                ranges.append({ range.min_code_point(), range.max_code_point() });
            u8 flags = 0;
            if (face->has_urls())
                flags |= FaceHasUrls;
            if (face->is_unusable_for_rendering())
                flags |= FaceIsUnusable;
            if (face->has_non_default_unicode_range())
                flags |= FaceHasNonDefaultUnicodeRange;
            records.append({
                .face_id = face->id(),
                // The snapshot takes its own reference; the typeface outlives any face that
                // produced it, and a resolution running elsewhere may be the last to let go.
                .typeface = reinterpret_cast<u64>(face->typeface().ptr()),
                .range_offset = static_cast<u32>(range_offset),
                .range_count = static_cast<u32>(face->unicode_ranges().size()),
                // What resolve_for_rendering() would answer, without starting anything.
                .pending_state = static_cast<u8>(to_underlying(face->rendering_state_without_requesting())),
                .flags = flags,
            });
        }
    }

    FontFaceSnapshotView view {
        .keys = keys.data(),
        .key_count = keys.size(),
        .records = records.data(),
        .record_count = records.size(),
        .family_text = family_text.data(),
        .family_text_length = family_text.size(),
        .ranges = ranges.data(),
        .range_count = ranges.size(),
        .generation = m_environment_generation,
    };
    return rust_font_face_snapshot_build(&view);
}

void FontComputer::publish_font_faces()
{
    auto* published = build_font_face_snapshot();
    rust_font_face_snapshot_release(m_published_font_faces);
    m_published_font_faces = published;
}

// The uncached resolution, for a caller that is already inside the memo.
NonnullRefPtr<Gfx::FontCascadeList const> FontComputer::compute_font_for_style_values_impl(ReadonlySpan<ComputedFontFamily const> font_families, CSSPixels const& font_size, int font_slope, double font_weight, Percentage const& font_width, FontOpticalSizing font_optical_sizing, HashMap<Utf16FlyString, double> const& font_variation_settings, FontFeatureData const& font_feature_data) const
{
    FontFeatureValuesProvider font_feature_values = [this](Utf16FlyString const& family) -> HashMap<FontFeatureValueKey, Vector<u32>> const& {
        return font_feature_values_for_family(family);
    };
    FontFaceSnapshotView view;
    rust_font_face_snapshot_view(m_published_font_faces, &view);
    VERIFY(view.generation == m_environment_generation);
    return resolve_font_cascade(view, font_families, font_size, font_slope, font_weight, font_width, font_optical_sizing, font_variation_settings, font_feature_data, &font_feature_values);
}

FontLoader::FontLoader(FontComputer& font_computer, RuleOrDeclaration rule_or_declaration, Vector<Source> sources, GC::Ptr<GC::Function<void(RefPtr<Gfx::Typeface const>)>> on_load)
    : m_font_computer(font_computer)
    , m_rule_or_declaration(rule_or_declaration)
    , m_sources(move(sources))
{
    if (on_load)
        m_subscribers.append(*on_load);
}

FontLoader::~FontLoader() = default;

void FontLoader::subscribe(GC::Ref<GC::Function<void(RefPtr<Gfx::Typeface const>)>> callback)
{
    if (m_has_completed) {
        callback->function()(m_typeface);
        return;
    }
    m_subscribers.append(callback);
}

void FontLoader::visit_edges(Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_font_computer);
    if (auto* rule = m_rule_or_declaration.value.get_pointer<RuleOrDeclaration::Rule>())
        visitor.visit(rule->parent_style_sheet);
    else if (auto* block = m_rule_or_declaration.value.get_pointer<RuleOrDeclaration::StyleDeclaration>())
        visitor.visit(block->parent_rule);
    visitor.visit(m_fetch_controller);
    visitor.visit(m_subscribers);
}

bool FontLoader::is_loading() const
{
    return m_fetch_controller && !m_typeface;
}

bool FontLoader::may_finish_from_cache() const
{
    return !m_fetch_controller || !m_fetch_controller->requires_network();
}

bool FontLoader::has_started_request() const
{
    return m_fetch_controller && m_fetch_controller->has_started_request();
}

void FontLoader::did_request_for_rendering()
{
    // INTEROP: Delay the document load event til the fetch for this font has settled. Blink, Gecko, and WebKit all
    //          keep the document from completing while a font load requested for rendering is pending — so pages
    //          that measure font-dependent geometry in a load-event handler see the loaded font, not a fallback.
    if (is_loading() && !m_document_load_event_delayer.has_value())
        m_document_load_event_delayer.emplace(m_font_computer->document());
}

void FontLoader::start_loading_next_source()
{
    // A loader that has settled must not consume another source: its typeface is final, and fetching a
    // further src would replace it after subscribers already saw the settled one. load_font_face()
    // hands back an existing loader for a shared source list, so a second FontFace can reach here after
    // the first one finished.
    if (m_has_completed)
        return;

    if (m_fetch_controller && m_fetch_controller->state() == Fetch::Infrastructure::FetchController::State::Ongoing)
        return;

    while (true) {
        // https://drafts.csswg.org/css-fonts-4/#src-desc
        // NB: Try local names and URLs in source order, continuing after an unavailable face.
        while (!m_sources.is_empty() && m_sources.first().has<Utf16FlyString>()) {
            auto name = MUST(m_sources.take_first().get<Utf16FlyString>().view().to_utf8());
            if (auto typeface = Gfx::FontDatabase::the().get_typeface_by_local_name(name)) {
                font_did_load_or_fail(move(typeface));
                return;
            }
        }
        if (m_sources.is_empty()) {
            font_did_load_or_fail(nullptr);
            return;
        }

        // https://drafts.csswg.org/css-fonts-4/#fetch-a-font
        // To fetch a font given a selected <url> url for @font-face rule, fetch url, with ruleOrDeclaration being rule,
        // destination "font", CORS mode "cors", and processResponse being the following steps given response res and null,
        // failure or a byte stream stream:
        m_has_received_font_data = false;
        m_fetch_controller = fetch_a_style_resource(m_sources.take_first().get<URL>(), m_rule_or_declaration, Fetch::Infrastructure::Request::Destination::Font, CorsMode::Cors,
            [loader = this](auto response, auto stream) {
                // 1. If stream is null, return.
                // 2. Load a font from stream according to its type.

                auto* immutable_bytes = stream.template get_pointer<Core::ImmutableBytes>();
                if (!immutable_bytes) {
                    if (loader->m_sources.is_empty()) {
                        loader->font_did_load_or_fail(nullptr);
                    } else {
                        loader->m_fetch_controller = nullptr;
                        loader->start_loading_next_source();
                    }
                    return;
                }
                loader->m_has_received_font_data = true;
                auto bytes = immutable_bytes->copy_to_byte_buffer().release_value_but_fixme_should_propagate_errors();

                auto mime_type_essence = loader->try_load_font_mime_type_essence(response, bytes);
                if (!requires_off_thread_vector_font_preparation(bytes, mime_type_essence)) {
                    auto maybe_typeface = try_load_vector_font(bytes, mime_type_essence);
                    if (maybe_typeface.is_error()) {
                        if (loader->m_sources.is_empty()) {
                            loader->font_did_load_or_fail(nullptr);
                        } else {
                            loader->m_fetch_controller = nullptr;
                            loader->start_loading_next_source();
                        }
                        return;
                    }

                    loader->font_did_load_or_fail(maybe_typeface.release_value());
                    return;
                }

                auto loader_handle = GC::make_root(GC::Ref(*loader));
                prepare_vector_font_data_off_thread(move(bytes), [loader = move(loader_handle)](auto prepared_font_data) mutable {
                if (prepared_font_data.is_error()) {
                    // NB: If we have other sources available, try the next one.
                    if (loader->m_sources.is_empty()) {
                        loader->font_did_load_or_fail(nullptr);
                    } else {
                        loader->m_fetch_controller = nullptr;
                        loader->start_loading_next_source();
                    }
                    return;
                }

                auto prepared = prepared_font_data.release_value();
                auto maybe_typeface = Gfx::Typeface::try_load_from_anonymous_buffer(move(prepared));
                if (maybe_typeface.is_error()) {
                    if (loader->m_sources.is_empty()) {
                        loader->font_did_load_or_fail(nullptr);
                    } else {
                        loader->m_fetch_controller = nullptr;
                        loader->start_loading_next_source();
                    }
                    return;
                }

                loader->font_did_load_or_fail(maybe_typeface.release_value()); });
            });

        if (m_fetch_controller || m_has_completed)
            return;
    }
}

void FontLoader::font_did_load_or_fail(RefPtr<Gfx::Typeface const> typeface)
{
    if (typeface)
        m_typeface = typeface.release_nonnull();
    m_has_completed = true;
    m_document_load_event_delayer.clear();
    // Each subscriber publishes its now-loaded FontFace to FontComputer. The face's complete
    // descriptors are needed to compare the old and new selections, so invalidation happens there.
    for (auto& callback : m_subscribers)
        callback->function()(m_typeface);
    m_subscribers.clear();
    m_fetch_controller = nullptr;
}

Optional<ByteString> FontLoader::try_load_font_mime_type_essence(Fetch::Infrastructure::Response const& response, ByteBuffer const& bytes)
{
    // FIXME: This could maybe use the format() provided in @font-face as well, since often the mime type is just application/octet-stream and we have to try every format
    auto mime_type = Fetch::Infrastructure::extract_mime_type(response.header_list());
    if (!mime_type.has_value() || !mime_type->is_font()) {
        mime_type = MimeSniff::Resource::sniff(bytes, MimeSniff::SniffingConfiguration { .sniffing_context = MimeSniff::SniffingContext::Font });
    }
    if (!mime_type.has_value())
        return {};
    return mime_type->essence().to_byte_string();
}

void FontComputer::visit_edges(Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_document);
    for (auto& [_, faces] : m_font_faces)
        visitor.visit(faces);
    for (auto& [_, loader] : m_loaders_by_source)
        visitor.visit(loader);
}

HashMap<FontFeatureValueKey, Vector<u32>> const& FontComputer::font_feature_values_for_family(Utf16FlyString const& family_name) const
{
    return m_font_feature_values_cache.ensure(family_name, [&]() {
        HashMap<FontFeatureValueKey, Vector<u32>> font_feature_values;

        // https://drafts.csswg.org/css-fonts/#font-feature-values-syntax
        // For each <family-name> in the @font-feature-values prelude, each font feature value declaration defines a
        // mapping between a (family name, feature block name, declaration name) tuple and the list of one or more
        // integers from the declaration’s value. If the same tuple appears more than once in a document (such as if a
        // single block), the last-defined one is used.

        if (!m_document)
            return font_feature_values;

        // FIXME: We only account for Author stylesheets here, we should also account for UserAgent and User
        m_document->style_scope().for_each_active_css_style_sheet([&](CSS::StyleSheetState const& sheet) {
            sheet.for_each_effective_rule_data(TraversalOrder::Preorder, [&](RustRuleView const& rule, Utf16View) {
                if (rule.type() != RustRule::Type::FontFeatureValues)
                    return;
                auto values = rule.font_feature_values();
                bool matches_family = false;
                for (size_t index = 0; index < values.family_count(); ++index) {
                    if (values.family_at(index) == family_name.view()) {
                        matches_family = true;
                        break;
                    }
                }
                if (!matches_family)
                    return;

                auto append = [&](FontFeatureValuesRuleKind kind, FontFeatureValueType type) {
                    values.for_each_entry(kind, [&](auto key, auto values) {
                        Vector<u32> copy;
                        copy.append(values.data(), values.size());
                        font_feature_values.set({ type, Utf16FlyString::from_utf16(key) }, move(copy));
                    });
                };
                append(FontFeatureValuesRuleKind::Annotation, FontFeatureValueType::Annotation);
                append(FontFeatureValuesRuleKind::Ornaments, FontFeatureValueType::Ornaments);
                append(FontFeatureValuesRuleKind::Stylistic, FontFeatureValueType::Stylistic);
                append(FontFeatureValuesRuleKind::Swash, FontFeatureValueType::Swash);
                append(FontFeatureValuesRuleKind::CharacterVariant, FontFeatureValueType::CharacterVariant);
                append(FontFeatureValuesRuleKind::Styleset, FontFeatureValueType::Styleset);

                // NB: We don't include historical-forms since it can't be referenced - it seems like it's inclusion in the syntax
                //     for @font-feature-values was a mistake and isn't supported by Chrome or Firefox. See
                //     https://github.com/w3c/csswg-drafts/issues/9926#issuecomment-2017241274
            });
        });

        return font_feature_values;
    });

    return m_font_feature_values_cache.get(family_name).value();
}

ScopedFontFeatureValuesTables const& FontComputer::published_font_feature_values() const
{
    if (!m_font_feature_values_snapshot_dirty)
        return m_published_font_feature_values;

    HashTable<Utf16FlyString> families;
    m_document->style_scope().for_each_active_css_style_sheet([&](CSS::StyleSheetState const& sheet) {
        sheet.for_each_effective_rule_data(TraversalOrder::Preorder, [&](RustRuleView const& rule, Utf16View) {
            if (rule.type() != RustRule::Type::FontFeatureValues)
                return;
            auto values = rule.font_feature_values();
            for (size_t index = 0; index < values.family_count(); ++index)
                families.set(Utf16FlyString::from_utf16(values.family_at(index)));
        });
    });

    m_published_font_feature_values.clear();
    FontFeatureValuesTable document_values;
    for (auto const& family : families)
        document_values.set(family, font_feature_values_for_family(family));
    m_published_font_feature_values.set(m_document->style_scope().style_engine_tree_scope().value(), move(document_values));

    m_document->for_each_shadow_root([&](DOM::ShadowRoot& shadow_root) {
        FontFeatureValuesTable scope_values;
        shadow_root.style_scope().for_each_active_css_style_sheet([&](CSS::StyleSheetState const& sheet) {
            sheet.for_each_effective_rule_data(TraversalOrder::Preorder, [&](RustRuleView const& rule, Utf16View) {
                if (rule.type() != RustRule::Type::FontFeatureValues)
                    return;
                auto values = rule.font_feature_values();
                for (size_t index = 0; index < values.family_count(); ++index) {
                    auto family = Utf16FlyString::from_utf16(values.family_at(index));
                    auto& family_values = scope_values.ensure(family, [] { return HashMap<FontFeatureValueKey, Vector<u32>> {}; });
                    auto append = [&](FontFeatureValuesRuleKind kind, FontFeatureValueType type) {
                        values.for_each_entry(kind, [&](auto key, auto feature_values) {
                            Vector<u32> copy;
                            copy.append(feature_values.data(), feature_values.size());
                            family_values.set({ type, Utf16FlyString::from_utf16(key) }, move(copy));
                        });
                    };
                    append(FontFeatureValuesRuleKind::Annotation, FontFeatureValueType::Annotation);
                    append(FontFeatureValuesRuleKind::Ornaments, FontFeatureValueType::Ornaments);
                    append(FontFeatureValuesRuleKind::Stylistic, FontFeatureValueType::Stylistic);
                    append(FontFeatureValuesRuleKind::Swash, FontFeatureValueType::Swash);
                    append(FontFeatureValuesRuleKind::CharacterVariant, FontFeatureValueType::CharacterVariant);
                    append(FontFeatureValuesRuleKind::Styleset, FontFeatureValueType::Styleset);
                }
            });
        });
        m_published_font_feature_values.set(shadow_root.style_scope().style_engine_tree_scope().value(), move(scope_values));
    });
    m_font_feature_values_snapshot_dirty = false;
    return m_published_font_feature_values;
}

NonnullRefPtr<Gfx::FontCascadeList const> FontComputer::compute_font_for_style_values(Vector<ComputedFontFamily> font_families, CSSPixels const& font_size, int font_slope, double font_weight, Percentage const& font_width, FontOpticalSizing font_optical_sizing, HashMap<Utf16FlyString, double> const& font_variation_settings, FontFeatureData const& font_feature_data) const
{
    ComputedFontCacheKey cache_key {
        .font_families = move(font_families),
        .font_optical_sizing = font_optical_sizing,
        .font_size = font_size,
        .font_slope = font_slope,
        .font_weight = font_weight,
        .font_width = font_width,
        .font_variation_settings = font_variation_settings,
        .font_feature_data = font_feature_data,
    };

    FontFeatureValuesProvider font_feature_values = [this](Utf16FlyString const& family) -> HashMap<FontFeatureValueKey, Vector<u32>> const& {
        return font_feature_values_for_family(family);
    };
    FontFaceSnapshotView view;
    rust_font_face_snapshot_view(m_published_font_faces, &view);
    auto font_list = m_font_cascade_memo->resolve(view, cache_key, &font_feature_values);

    // A cascade this computed may have wanted a web face loaded. Inside a style update the loads
    // wait for its end; everywhere else - canvas, getComputedStyle - they happen right here, which
    // is where the cascade build used to perform them itself.
    (void)request_wanted_web_faces();

    return font_list;
}

NonnullRefPtr<Gfx::FontCascadeList const> FontComputer::compute_font_for_style_values(StyleValue const& font_family, CSSPixels const& font_size, int font_slope, double font_weight, Percentage const& font_width, FontOpticalSizing font_optical_sizing, HashMap<Utf16FlyString, double> const& font_variation_settings, FontFeatureData const& font_feature_data) const
{
    return compute_font_for_style_values(computed_font_families_from_style_value(font_family), font_size, font_slope, font_weight, font_width, font_optical_sizing, font_variation_settings, font_feature_data);
}

Gfx::Font const& FontComputer::initial_font() const
{
    // FIXME: This is not correct.
    static auto const& font = Platform::FontPlugin::the().default_font(12).release_nonnull().leak_ref();
    return font;
}

static bool style_value_references_any_font_family(StyleValue const& font_family_value, Vector<Utf16FlyString> const& family_names)
{
    if (!font_family_value.is_value_list())
        return false;

    for (auto const& item : font_family_value.as_value_list().values()) {
        if (item->is_keyword())
            continue; // Skip generic keywords (monospace, serif, etc.)

        auto item_family_name = string_from_style_value(*item);

        if (any_of(family_names, [&](auto const& family_name) { return item_family_name.equals_ignoring_ascii_case(family_name); }))
            return true;
    }
    return false;
}

static bool computed_font_families_reference_any_family(ReadonlySpan<ComputedFontFamily const> font_families, Vector<Utf16FlyString> const& family_names)
{
    return any_of(font_families, [&](ComputedFontFamily const& family) {
        return family.has<ComputedFontFamilyName>()
            && any_of(family_names, [&](auto const& family_name) { return family.get<ComputedFontFamilyName>().name.equals_ignoring_ascii_case(family_name); });
    });
}

static bool font_values_reference_any_font_family(ComputedValues::FontValues const& font_values, Vector<Utf16FlyString> const& family_names)
{
    auto font_family = font_values.font_family_style_value();
    return font_family && style_value_references_any_font_family(*font_family, family_names);
}

void FontComputer::clear_computed_font_cache(Utf16FlyString const& family_name)
{
    Vector<Utf16FlyString> family_names;
    family_names.append(family_name);
    clear_computed_font_cache_for_families(family_names);
}

static void record_font_input_change(DOM::Element& element)
{
    if (auto* record = element.style_input_record())
        record->font_environment_changed = true;
    constexpr u8 font_group = 1u << ComputedValues::FontValues::style_group_index;
    // Derived, not recorded: this is not C++ asking for a computation only it can do. The change
    // is to the published `@font-face` table, which the engine holds and versions, so the engine
    // settles the element's record itself wherever the rest of its reaction lets it.
    element.document().style_computer().style_engine().record_derived_element_style_input_change(
        element.style_node_id(), StyleEngine::PublishedStyle | StyleEngine::RecomputeStyle | StyleEngine::FontInputsChanged, font_group);
}

void FontComputer::clear_computed_font_cache_for_families(Vector<Utf16FlyString> const& family_names)
{
    VERIFY(!family_names.is_empty());
    bump_environment_generation();

    // Only forget remembered resolutions that reference the loaded font family.
    m_font_cascade_memo->take_matching([&](ComputedFontCacheKey const& key, Gfx::FontCascadeList const&) {
        return computed_font_families_reference_any_family(key.font_families, family_names);
    });

    auto element_uses_font_family = [&](DOM::Element const& element) {
        // Check the element's own font-family.
        if (auto const* values = element.style_group<ComputedValues::FontValues>()) {
            if (font_values_reference_any_font_family(*values, family_names))
                return true;
        }

        // Check pseudo-elements, which may use a different font-family than the element itself.
        bool synthetic_pseudo_element_uses_font_family = false;
        element.for_each_synthetic_pseudo_element([&](Web::CSS::PseudoElement pseudo_element, Web::DOM::SyntheticPseudoElement const&) {
            if (auto const* values = element.style_group<ComputedValues::FontValues>(pseudo_element)) {
                if (font_values_reference_any_font_family(*values, family_names)) {
                    synthetic_pseudo_element_uses_font_family = true;
                    return IterationDecision::Break;
                }
            }
            return IterationDecision::Continue;
        });

        return synthetic_pseudo_element_uses_font_family;
    };

    // Walk the DOM tree (including shadow trees) and publish inputs for elements that use this font family.
    document().for_each_shadow_including_inclusive_descendant([&](DOM::Node& node) {
        auto* element = as_if<DOM::Element>(node);
        if (!element)
            return TraversalDecision::Continue;

        if (element_uses_font_family(*element)) {
            record_font_input_change(*element);
            return TraversalDecision::Continue;
        }

        return TraversalDecision::Continue;
    });
}

void FontComputer::clear_font_feature_values_cache(Utf16FlyString const& family_name)
{
    m_font_feature_values_cache.remove(family_name);
    m_font_feature_values_snapshot_dirty = true;
}

void FontComputer::invalidate_font_feature_values_snapshot()
{
    m_font_feature_values_cache.clear();
    m_font_feature_values_snapshot_dirty = true;
}

bool FontComputer::should_defer_initial_paint()
{
    if (m_has_completed_initial_paint)
        return false;
    bool has_pending_fonts = false;
    // OPTIMIZATION: Finish cache lookups and decode cache-resident fonts before the first paint. A cache miss
    //               releases this wait before any network activity; subsequent paints use the font display timeline.
    for (auto const& entry : m_font_faces) {
        for (auto const& face : entry.value) {
            if (face->is_pending_rendering_from_cache())
                return true;
            has_pending_fonts |= face->has_pending_rendering();
        }
    }
    m_initial_paint_had_pending_fonts = has_pending_fonts;
    m_has_completed_initial_paint = true;
    return false;
}

void FontComputer::did_load_font(Utf16FlyString const& family_name)
{
    if (m_font_face_change_batch_depth > 0) {
        if (!any_of(m_batched_font_face_change_families, [&](auto const& existing_family_name) { return existing_family_name.equals_ignoring_ascii_case(family_name); }))
            m_batched_font_face_change_families.append(family_name);
        return;
    }

    clear_computed_font_cache(family_name);
}

void FontComputer::did_load_font(FontFaceKey const& changed_face)
{
    if (m_font_face_change_batch_depth > 0) {
        did_load_font(changed_face.family_name);
        return;
    }

    bump_environment_generation();
    // A family can contain many faces, but one face becoming available changes only the remembered
    // selections which now resolve to it. Compare those selections before forgetting them, then
    // find the elements holding the discarded cascade identities.
    FontFaceSnapshotView view;
    rust_font_face_snapshot_view(m_published_font_faces, &view);
    Vector<Utf16FlyString> changed_family { changed_face.family_name };
    HashTable<Gfx::FontCascadeList const*> invalidated_font_lists;
    Vector<NonnullRefPtr<Gfx::FontCascadeList const>> invalidated_font_lists_kept_alive_for_the_walk;
    m_font_cascade_memo->take_matching([&](ComputedFontCacheKey const& key, Gfx::FontCascadeList const& font_list) {
        if (!computed_font_families_reference_any_family(key.font_families, changed_family))
            return false;
        auto updated_font_list = compute_font_for_style_values_impl(key.font_families, key.font_size, key.font_slope, key.font_weight, key.font_width, key.font_optical_sizing, key.font_variation_settings, key.font_feature_data);
        if (!font_list.has_pending_faces() && font_list.equals(*updated_font_list))
            return false;
        invalidated_font_lists.set(&font_list);
        invalidated_font_lists_kept_alive_for_the_walk.append(font_list);
        return true;
    });
    if (invalidated_font_lists.is_empty())
        return;

    document().for_each_shadow_including_inclusive_descendant([&](DOM::Node& node) {
        auto* element = as_if<DOM::Element>(node);
        if (!element)
            return TraversalDecision::Continue;
        auto uses_invalidated_font_list = [&](Optional<CSS::PseudoElement> pseudo_element = {}) {
            auto const* values = element->style_group<ComputedValues::FontValues>(pseudo_element);
            return values && invalidated_font_lists.contains(&values->font_list_value());
        };
        bool should_recompute = uses_invalidated_font_list();
        element->for_each_synthetic_pseudo_element([&](CSS::PseudoElement pseudo_element, DOM::SyntheticPseudoElement const&) {
            if (uses_invalidated_font_list(pseudo_element)) {
                should_recompute = true;
                return IterationDecision::Break;
            }
            return IterationDecision::Continue;
        });
        if (should_recompute)
            record_font_input_change(*element);
        return TraversalDecision::Continue;
    });
}

void FontComputer::begin_font_face_change_batch()
{
    ++m_font_face_change_batch_depth;
}

void FontComputer::end_font_face_change_batch()
{
    VERIFY(m_font_face_change_batch_depth > 0);
    --m_font_face_change_batch_depth;

    if (m_font_face_change_batch_depth > 0 || m_batched_font_face_change_families.is_empty())
        return;

    clear_computed_font_cache_for_families(m_batched_font_face_change_families);
    m_batched_font_face_change_families.clear();
}

void FontComputer::register_font_face(NonnullRefPtr<FontFaceState> face)
{
    VERIFY(face->should_be_registered_with_font_computer());

    auto key = face->matching_key();
    auto& faces = m_font_faces.ensure(key);
    if (!faces.contains_slow(face))
        faces.append(face);
    did_load_font(key);
}

void FontComputer::unregister_font_face(NonnullRefPtr<FontFaceState> face)
{
    VERIFY(face->should_be_registered_with_font_computer());

    auto key = face->matching_key();
    if (auto it = m_font_faces.find(key); it != m_font_faces.end()) {
        it->value.remove_all_matching([&](auto const& entry) { return entry == face; });
        if (it->value.is_empty())
            m_font_faces.remove(it);
    }
    did_load_font(key);
}

void FontComputer::synchronize_font_face_order(Vector<NonnullRefPtr<FontFaceState>> const& font_source_order)
{
    for (auto& entry : m_font_faces) {
        Vector<NonnullRefPtr<FontFaceState>> ordered_faces;
        for (auto& font_face : font_source_order) {
            if (entry.value.contains_slow(font_face))
                ordered_faces.append(font_face);
        }
        for (auto& font_face : entry.value) {
            if (!ordered_faces.contains_slow(font_face))
                ordered_faces.append(font_face);
        }

        bool order_changed = ordered_faces.size() != entry.value.size();
        if (!order_changed) {
            for (size_t index = 0; index < entry.value.size(); ++index) {
                if (ordered_faces[index] != entry.value[index]) {
                    order_changed = true;
                    break;
                }
            }
        }
        if (!order_changed)
            continue;

        entry.value = move(ordered_faces);
        did_load_font(entry.key.family_name);
    }
}

GC::Ptr<FontLoader> FontComputer::load_font_face(ParsedFontFace const& font_face, RefPtr<StyleSheetState> parent_style_sheet, GC::Ptr<GC::Function<void(RefPtr<Gfx::Typeface const>)>> on_load)
{
    if (font_face.sources().is_empty()) {
        if (on_load)
            on_load->function()({});
        return {};
    }

    Vector<FontLoader::Source> sources;
    StringBuilder key_builder;
    for (auto const& source : font_face.sources()) {
        sources.append(source.local_or_url);
        auto value = source.local_or_url.has<URL>()
            ? source.local_or_url.get<URL>().to_string()
            : MUST(source.local_or_url.get<Utf16FlyString>().view().to_utf8());
        key_builder.appendff("{}{}:{}", source.local_or_url.has<URL>() ? 'u' : 'l', value.bytes_as_string_view().length(), value);
    }

    RuleOrDeclaration rule_or_declaration {
        .environment_settings_object = document().relevant_settings_object(),
        .value = RuleOrDeclaration::Rule {
            .parent_style_sheet = parent_style_sheet,
        },
        .style_resource_base_url = {},
        .parent_style_sheet_origin_clean = {},
    };

    auto key = MUST(key_builder.to_string());
    if (auto it = m_loaders_by_source.find(key); it != m_loaders_by_source.end()) {
        if (on_load)
            it->value->subscribe(*on_load);
        return it->value;
    }

    auto loader = GC::Heap::the().allocate<FontLoader>(*this, rule_or_declaration, move(sources), move(on_load));
    m_loaders_by_source.set(move(key), loader);
    return loader;
}

static void for_each_nested_font_rule(Parser::ValueParserFFI::NativeRuleList const* rules, Function<void(RustRuleView const&)> const& callback)
{
    Parser::ValueParserFFI::rust_rule_list_visit_font_rules(rules, &callback, [](void const* context, Parser::ValueParserFFI::NativeRuleView const* rule) {
        (*static_cast<Function<void(RustRuleView const&)> const*>(context))(RustRuleView { *rule });
    });
}

static void clear_font_feature_values_caches(RustRuleView const& rule, FontComputer& font_computer)
{
    auto values = rule.font_feature_values();
    for (size_t index = 0; index < values.family_count(); ++index) {
        auto family = Utf16FlyString::from_utf16(values.family_at(index));
        font_computer.clear_font_feature_values_cache(family);
        font_computer.clear_computed_font_cache(family);
    }
}

void FontComputer::load_fonts_from_sheet(StyleSheetState& sheet)
{
    begin_font_face_change_batch();
    ScopeGuard finish_font_face_change_batch = [&] {
        end_font_face_change_batch();
    };

    HashTable<u64> effective_font_rules;
    bool ancestors_match = true;
    for (auto* ancestor = &sheet; ancestor; ancestor = ancestor->parent_style_sheet()) {
        if (ancestor->disabled() || !ancestor->native_media_list().matches()) {
            ancestors_match = false;
            break;
        }
    }
    if (ancestors_match) {
        sheet.for_each_effective_rule_data(TraversalOrder::Preorder, [&](RustRuleView const& rule, Utf16View) {
            // Only this sheet's rules are connected below. Imported sheets synchronize separately.
            if (rule.type() == RustRule::Type::FontFace)
                effective_font_rules.set(rule.identity());
        });
    }

    for_each_nested_font_rule(sheet.native_rules().handle(), [&](RustRuleView const& rule) {
        if (rule.type() == RustRule::Type::FontFace) {
            auto descriptors = rule.descriptors();
            auto should_be_css_connected = effective_font_rules.contains(rule.identity())
                && descriptors.descriptor(DescriptorNameAndID::from_id(DescriptorID::FontFamily))
                && descriptors.descriptor(DescriptorNameAndID::from_id(DescriptorID::Src));
            auto connected_font_face = sheet.css_connected_font_face(rule.identity());
            if (!should_be_css_connected) {
                if (connected_font_face)
                    connected_font_face->disconnect_from_css_rule();
                return;
            }

            if (connected_font_face)
                return;

            // https://drafts.csswg.org/css-font-loading/#font-face-css-connection
            // A CSS @font-face rule automatically defines a corresponding FontFace object, which is automatically
            // placed in the document's font source when the rule is parsed. This FontFace object is CSS-connected.
            auto font_face = FontFaceState::create_css_connected(HTML::relevant_realm(document()), rule.identity(), sheet);
            document().fonts()->add_css_connected_font(font_face);
        }

        if (rule.type() == RustRule::Type::FontFeatureValues)
            clear_font_feature_values_caches(rule, *this);
    });

    document().fonts()->synchronize_css_connected_font_order();
}

void FontComputer::unload_fonts_from_sheet(StyleSheetState& sheet)
{
    begin_font_face_change_batch();
    ScopeGuard finish_font_face_change_batch = [&] {
        end_font_face_change_batch();
    };

    // https://drafts.csswg.org/css-font-loading/#font-face-css-connection
    // If a @font-face rule is removed from the document, its connected FontFace object is no longer CSS-connected.
    for_each_nested_font_rule(sheet.native_rules().handle(), [&](RustRuleView const& rule) {
        if (rule.type() == RustRule::Type::FontFace) {
            if (auto font_face = sheet.css_connected_font_face(rule.identity()))
                font_face->disconnect_from_css_rule();
        }

        if (rule.type() == RustRule::Type::FontFeatureValues)
            clear_font_feature_values_caches(rule, *this);
    });
}

}
