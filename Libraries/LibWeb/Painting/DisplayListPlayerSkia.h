/*
 * Copyright (c) 2024-2026, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/NonnullRefPtr.h>
#include <AK/RefPtr.h>
#include <LibGfx/Forward.h>
#include <LibWeb/Painting/DisplayList.h>
#include <LibWeb/Painting/DisplayListCommand.h>
#include <LibWeb/Painting/DisplayListRecorder.h>

class GrDirectContext;
class SkPaint;

namespace Web::Painting {

class WEB_API DisplayListPlayerSkia final : public DisplayListPlayer {
public:
    using CompositedContextResolver = Function<RefPtr<Gfx::PaintingSurface>(Web::Compositor::CompositorContextId)>;

    DisplayListPlayerSkia();
    explicit DisplayListPlayerSkia(RefPtr<Gfx::SkiaBackendContext>);
    ~DisplayListPlayerSkia();

    using DisplayListPlayer::execute;
    void execute(
        DisplayList const&,
        AccumulatedVisualContextTree const&,
        DisplayListResourceStorage const&,
        ScrollStateSnapshot const&,
        RefPtr<Gfx::PaintingSurface>,
        CanvasSurfaceRegistry const*,
        CompositedContextResolver const*);

    void flush(Gfx::PaintingSurface&) override;
    void flush_async(Gfx::PaintingSurface&, Function<void()>&&);
    void paint_scrollbar(Gfx::PaintingSurface&, PaintScrollBar const&);

private:
#define DECLARE_PLAY_COMMAND(command_type, player_method) \
    void play_command(command_type const&) override;
    ENUMERATE_DISPLAY_LIST_COMMANDS(DECLARE_PLAY_COMMAND)
#undef DECLARE_PLAY_COMMAND
    void play_command(ApplyEffects const&, Gfx::Filter const*) override;
    void apply_transform(Gfx::FloatPoint origin, Gfx::FloatMatrix4x4 const&) override;

    void add_clip_path(Gfx::Path const&, Gfx::WindingRule) override;

    bool would_be_fully_clipped_by_painter(Gfx::IntRect) const override;

    SkPaint paint_style_to_skia_paint(DisplayListPaintStyle const&, Gfx::FloatRect const& bounding_rect);
    Gfx::Path path_from_data(DisplayListDataSpan) const;
    ReadonlySpan<Color> gradient_colors(DisplayListGradientColorStops) const;
    ReadonlySpan<float> gradient_positions(DisplayListGradientColorStops) const;
    void clear_nested_display_list_raster_cache();

    RefPtr<Gfx::SkiaBackendContext> m_skia_backend_context;
    CompositedContextResolver const* m_composited_context_resolver { nullptr };

    // Rasterizations of the visible portions of nested display lists. Display lists are immutable while their
    // resource storage is current, so the cache is bounded by total byte size and cleared when storage changes.
    struct NestedDisplayListRasterCacheKey {
        u64 resource_storage_cache_id { 0 };
        u64 display_list_id { 0 };
        Gfx::IntRect visible_rect_in_list_space;

        bool operator==(NestedDisplayListRasterCacheKey const&) const = default;
    };
    struct NestedDisplayListRasterCacheKeyTraits : public DefaultTraits<NestedDisplayListRasterCacheKey> {
        static unsigned hash(NestedDisplayListRasterCacheKey const& key)
        {
            auto const& rect = key.visible_rect_in_list_space;
            u32 rect_hash = pair_int_hash(pair_int_hash(rect.x(), rect.y()), pair_int_hash(rect.width(), rect.height()));
            return pair_int_hash(pair_int_hash(u64_hash(key.resource_storage_cache_id), u64_hash(key.display_list_id)), rect_hash);
        }
    };
    HashMap<NestedDisplayListRasterCacheKey, NonnullRefPtr<Gfx::PaintingSurface>, NestedDisplayListRasterCacheKeyTraits> m_nested_display_list_raster_cache;
    size_t m_nested_display_list_raster_cache_bytes { 0 };
    u64 m_nested_display_list_raster_cache_resource_storage_cache_id { 0 };
};

}
