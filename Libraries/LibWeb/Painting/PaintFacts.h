/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>

namespace Web::Painting {

enum class StyleHoldsImageValues : u8 {
    // The row may have held images before: their paint facts are cleared.
    No,
    // The row held no images before either, so it has no paint facts of them to clear.
    NoAndHeldNone,
    Yes,
};

// `dom_node` is the node the row was built for.
WEB_API void push_paint_facts_after_style_attach(BoxSlot const&, DOM::Node* dom_node, StyleHoldsImageValues);
WEB_API void push_layer_image_paint_facts(BoxSlot const&);
WEB_API void push_form_control_paint_facts(HTML::HTMLInputElement&);
WEB_API void push_canvas_paint_facts(HTML::HTMLCanvasElement const&);
WEB_API void push_navigable_container_paint_facts(HTML::NavigableContainer const&);
enum class ReconcileAheadOfLayout : u8 {
    No,
    Yes,
};
WEB_API void reconcile_navigable_container_paint_facts(DOM::Document const&, ReconcileAheadOfLayout = ReconcileAheadOfLayout::No);
WEB_API void push_replaced_image_paint_facts(Layout::ImageProvider const&, BoxSlot const&);
WEB_API void push_video_paint_facts(HTML::HTMLVideoElement const&);
WEB_API void push_image_map_area_facts(HTML::HTMLImageElement&);
WEB_API void refresh_image_map_area_facts(DOM::Document&);

}
