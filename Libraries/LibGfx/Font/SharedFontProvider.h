/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/HashFunctions.h>
#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/Mutex.h>
#include <AK/OwnPtr.h>
#include <LibCore/MappedFile.h>
#include <LibGfx/Font/FontCatalog.h>
#include <LibGfx/Font/FontDatabase.h>
#include <LibGfx/Font/PathFontProvider.h>
#include <LibIPC/File.h>

namespace Gfx {

struct BrokeredFontFile {
    u32 ttc_index { 0 };
    FontFileFormat format { FontFileFormat::OpenType };
    IPC::File file;
};

// A system font the client re-matches in its own process, for typefaces whose data does not survive a round trip.
struct SystemFontReference {
    String family;
    u16 weight { 0 };
    u16 width { 0 };
    u8 slope { 0 };
};

struct BrokeredFont {
    u64 face_id { 0 };
    Variant<Empty, BrokeredFontFile, SystemFontReference> source;
};

// Build a typeface out of what the font service brokered. Neither touches provider state, so a
// caller on a thread of its own can use them too.
RefPtr<Typeface> load_typeface_from_font_file(u32 ttc_index, FontFileFormat, IPC::File);
RefPtr<Typeface> load_typeface_from_system_font_reference(SystemFontReference const&);

// The render side's own way out of the process for the questions a font match asks.
//
// A renderer cannot match fonts itself: the answer comes from the process that can, over an IPC
// connection, and a connection belongs to the thread that constructed it. The connection the
// callbacks below use is the document thread's, so a match made from a render stage has to leave
// on a different one. Whoever owns such a connection installs it here.
class RenderSideFontBroker {
public:
    virtual ~RenderSideFontBroker();

    virtual BrokeredFont open_font(u64 generation, u64 face_id) = 0;
    virtual BrokeredFont match_font(String const& family, u16 weight, u16 width, u8 slope) = 0;
    virtual Optional<FlyString> resolve_generic_family(String const& family, u16 weight, u8 slope) = 0;
};

void install_render_side_font_broker(RenderSideFontBroker&);
[[nodiscard]] bool has_render_side_font_broker();

// The scope in which a font match must not use the document thread's connection. The style stage's
// between-pass font batch runs inside one; every other caller leaves it closed and keeps today's
// path. Nesting is allowed, and the scope is per thread, so the stage carries it with it when it
// moves off the document thread.
class RenderSideFontScope {
    AK_MAKE_NONCOPYABLE(RenderSideFontScope);
    AK_MAKE_NONMOVABLE(RenderSideFontScope);

public:
    RenderSideFontScope();
    ~RenderSideFontScope();

    // How many questions this scope had to ask on the document thread's connection after all,
    // because no broker was installed. The stage seals report that; nothing else needs it.
    [[nodiscard]] u64 questions_that_reached_the_document_thread() const;

private:
    u64 m_questions_at_entry { 0 };
};

struct SharedFontProviderCallbacks {
    Function<BrokeredFont(u64 generation, u64 face_id)> open_font;
    Function<BrokeredFont(String const& name)> match_local_font;
    Function<BrokeredFont(String const& family, u16 weight, u16 width, u8 slope)> match_font;
    Function<BrokeredFont(u32 code_point, u16 weight, u16 width, u8 slope, bool prefer_color_emoji)> match_font_for_code_point;
    Function<Optional<FlyString>(String const& family, u16 weight, u8 slope)> resolve_generic_family;
};

class SharedFontProvider final : public SystemFontProvider {
    AK_MAKE_NONCOPYABLE(SharedFontProvider);
    AK_MAKE_NONMOVABLE(SharedFontProvider);

public:
    AK_ALLOC_WITH_KMALLOC;

    static ErrorOr<NonnullOwnPtr<SharedFontProvider>> create(NonnullOwnPtr<Core::MappedFile>, u64 generation, SharedFontProviderCallbacks&&);
    static ErrorOr<NonnullOwnPtr<SharedFontProvider>> create_from_catalog_file_or_empty(IPC::File, u64 size, u64 generation, SharedFontProviderCallbacks&&);
    static ErrorOr<NonnullOwnPtr<SharedFontProvider>> create_empty(u64 generation, SharedFontProviderCallbacks&&);
    virtual ~SharedFontProvider() override;

    ErrorOr<void> replace_catalog(NonnullOwnPtr<Core::MappedFile>, u64 generation);
    ErrorOr<void> replace_catalog(IPC::File, u64 size, u64 generation);

    virtual RefPtr<Gfx::Font> get_font(FlyString const& family, float point_size, unsigned weight, unsigned width, unsigned slope, Optional<FontVariationSettings> const& = {}, Optional<Gfx::ShapeFeatures> const& = {}) override;
    virtual void for_each_typeface_with_family_name(FlyString const&, Function<void(Typeface const&)>) override;
    virtual RefPtr<Typeface> get_typeface_by_id(u64 generation, u64 face_id) override;
    virtual RefPtr<Typeface> get_typeface_by_local_name(String const&) override;
    virtual RefPtr<Gfx::Font> get_font_for_code_point(u32 code_point, float point_size, u16 weight, u16 width, u8 slope, bool prefer_color_emoji) override;
    virtual Optional<FlyString> resolve_generic_family(StringView family_name, u16 weight, u8 slope) override;
    virtual StringView name() const LIFETIME_BOUND override { return "Shared"sv; }

private:
    struct CodePointCacheKey {
        u32 code_point { 0 };
        u16 weight { 0 };
        u16 width { 0 };
        u8 slope { 0 };
        bool prefer_color_emoji { false };

        bool operator==(CodePointCacheKey const&) const = default;
    };

    struct CodePointCacheKeyTraits : public DefaultTraits<CodePointCacheKey> {
        static unsigned hash(CodePointCacheKey const& key)
        {
            auto style_hash = pair_int_hash(pair_int_hash(key.weight, key.width), pair_int_hash(key.slope, key.prefer_color_emoji));
            return pair_int_hash(key.code_point, style_hash);
        }
    };

    SharedFontProvider(NonnullOwnPtr<Core::MappedFile>, NonnullOwnPtr<FontCatalog>, SharedFontProviderCallbacks);

    RefPtr<Typeface> load_catalog_face(FontCatalogFace const&);
    RefPtr<Typeface> load_brokered_font(BrokeredFont);
    RefPtr<Typeface> load_font_file(u64 face_id, u32 ttc_index, FontFileFormat, IPC::File);
    RefPtr<Typeface> load_font_reference(u64 face_id, SystemFontReference const&);
    void clear_typeface_cache();

    // Guards everything below it. A font match is no longer the document thread's alone: the style
    // stage's batch resolves from a published @font-face table and these process-wide services.
    Mutex m_mutex;
    NonnullOwnPtr<Core::MappedFile> m_catalog_mapping;
    NonnullOwnPtr<FontCatalog> m_catalog;
    SharedFontProviderCallbacks m_callbacks;
    PathFontProvider m_resource_fonts;
    HashMap<u64, NonnullRefPtr<Typeface>> m_typeface_cache;
    HashTable<u64> m_failed_face_ids;
    HashMap<CodePointCacheKey, RefPtr<Typeface>, CodePointCacheKeyTraits> m_code_point_cache;
};

}

namespace IPC {

template<>
ErrorOr<void> encode(Encoder&, Gfx::BrokeredFontFile const&);

template<>
ErrorOr<Gfx::BrokeredFontFile> decode(Decoder&);

template<>
ErrorOr<void> encode(Encoder&, Gfx::SystemFontReference const&);

template<>
ErrorOr<Gfx::SystemFontReference> decode(Decoder&);

template<>
ErrorOr<void> encode(Encoder&, Gfx::BrokeredFont const&);

template<>
ErrorOr<Gfx::BrokeredFont> decode(Decoder&);

}
