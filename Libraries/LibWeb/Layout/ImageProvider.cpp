/*
 * Copyright (c) 2024, Tim Flynn <trflynn89@serenityos.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGfx/DecodedImageFrame.h>
#include <LibWeb/HTML/DecodedImageData.h>
#include <LibWeb/HTML/HTMLImageElement.h>
#include <LibWeb/HTML/HTMLInputElement.h>
#include <LibWeb/HTML/HTMLObjectElement.h>
#include <LibWeb/Layout/ImageProvider.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/PaintFacts.h>

namespace Web::Layout {

Optional<CSSPixels> ImageProvider::intrinsic_width() const
{
    if (auto const& data = decoded_image_data())
        return data->intrinsic_width();
    return {};
}

Optional<CSSPixels> ImageProvider::intrinsic_height() const
{
    if (auto const& data = decoded_image_data())
        return data->intrinsic_height();
    return {};
}

Optional<CSSPixelFraction> ImageProvider::intrinsic_aspect_ratio() const
{
    if (auto const& data = decoded_image_data())
        return data->intrinsic_aspect_ratio();
    return {};
}

Optional<CSSPixelSize> ImageProvider::intrinsic_size() const
{
    auto width = intrinsic_width();
    auto height = intrinsic_height();
    if (!width.has_value() || !height.has_value())
        return {};

    return CSSPixelSize { *width, *height };
}

Optional<Gfx::DecodedImageFrame> ImageProvider::current_image_frame(Optional<Gfx::IntSize> size) const
{
    if (auto const& data = decoded_image_data())
        return data->current_frame(size.value_or(intrinsic_size().value_or({}).to_type<int>()));
    return {};
}

Optional<Gfx::DecodedImageFrame> ImageProvider::default_image_frame(Optional<Gfx::IntSize> size) const
{
    if (auto const& data = decoded_image_data())
        return data->default_frame(size.value_or(intrinsic_size().value_or({}).to_type<int>()));
    return {};
}

void ImageProvider::image_provider_contents_changed() const
{
    auto box = image_provider_box();
    if (!box)
        return;
    if (box.kind() == RustFFI::NodeKind::ImageBox) {
        // A box that owns its provider is handed it once the frame that built the box is over.
        if (RustFFI::layout_arena_image_box_awaits_owned_provider(box.arena(), box.slot()) || image_provider_of_image_box(box) != this)
            return;
    }
    Painting::push_replaced_image_paint_facts(*this, box);
}

ImageProvider const* image_provider_of_image_box(Painting::BoxSlot const& box)
{
    if (!box || box.kind() != RustFFI::NodeKind::ImageBox)
        return nullptr;
    if (auto const* owned = static_cast<ImageProvider const*>(RustFFI::layout_arena_owned_image_provider(box.arena(), box.slot())))
        return owned;
    auto element = box.dom_node();
    if (auto const* image = as_if<HTML::HTMLImageElement>(element.ptr()))
        return image;
    if (auto const* input = as_if<HTML::HTMLInputElement>(element.ptr()))
        return input;
    if (auto const* object = as_if<HTML::HTMLObjectElement>(element.ptr()))
        return object;
    return nullptr;
}

}
