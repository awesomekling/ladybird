/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/RefPtr.h>
#include <AK/Types.h>
#include <LibGfx/Forward.h>

namespace Gfx {

struct SystemFallbackFontKey {
    u32 code_point { 0 };
    u16 weight { 0 };
    u16 width { 0 };
    u8 slope { 0 };
    bool prefer_color_emoji { false };
    float point_size { 0 };

    [[nodiscard]] bool operator==(SystemFallbackFontKey const&) const = default;
};

// Matches a code point against the installed font set from any thread.
//
// A sandboxed process cannot match fonts itself: the answer comes from the process that can, over
// an IPC connection, and a connection belongs to one thread. Whoever owns one that a render pass
// may use installs it here, and a miss made from a pass is routed through it.
class SystemFallbackFontService {
public:
    virtual ~SystemFallbackFontService();

    // Null means no installed font covers the code point, which is an answer like any other.
    virtual RefPtr<Font const> match_font_for_code_point(SystemFallbackFontKey const&) = 0;

    // The installed font set changed, so anything matched against the old one is stale.
    virtual void did_change_font_set() = 0;
};

void install_render_side_system_fallback_font_service(NonnullOwnPtr<SystemFallbackFontService>);
[[nodiscard]] bool has_render_side_system_fallback_font_service();

// The installed font that covers a code point at one style. The answer depends on the key and the
// font set alone, never on a document, so one memo serves every document in the process and the
// answer for a key never changes while the font set stands.
RefPtr<Font const> system_fallback_font(SystemFallbackFontKey const&);

// The same memo, asked from inside a render stage. A miss goes through the render side's own font
// service when one is installed, so that it never reaches the document thread's connection.
RefPtr<Font const> system_fallback_font_from_render_side(SystemFallbackFontKey const&);

// `reached_document_thread` says whether the miss had to be matched through the process's system
// font provider after all, which in a renderer means the connection the calling thread owns. The
// stage seals report that; nothing else needs the overload.
RefPtr<Font const> system_fallback_font_from_render_side(SystemFallbackFontKey const&, bool& reached_document_thread);

// Drops every memoized answer. Call this whenever the installed font set changes.
void clear_system_fallback_font_cache();

[[nodiscard]] size_t system_fallback_font_cache_size();

}
