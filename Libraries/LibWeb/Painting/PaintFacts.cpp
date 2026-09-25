/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGfx/ImageFrameHandle.h>
#include <LibWeb/CSS/StyleValues/AbstractImageStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageSetStyleValue.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/InvalidationJournal.h>
#include <LibWeb/HTML/DecodedImageData.h>
#include <LibWeb/HTML/HTMLAreaElement.h>
#include <LibWeb/HTML/HTMLCanvasElement.h>
#include <LibWeb/HTML/HTMLImageElement.h>
#include <LibWeb/HTML/HTMLInputElement.h>
#include <LibWeb/HTML/HTMLMapElement.h>
#include <LibWeb/HTML/HTMLVideoElement.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/NavigableContainer.h>
#include <LibWeb/HTML/RemoteNavigable.h>
#include <LibWeb/Layout/Box.h>
#include <LibWeb/Layout/ImageProvider.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/PaintFacts.h>
#include <LibWeb/SVG/SVGDecodedImageData.h>
#include <LibWeb/SVG/SVGImageElement.h>

namespace Web::Painting {

static bool paints_form_control_from_facts(Layout::Node const& layout_node)
{
    return layout_node.kind() == Layout::RustFFI::NodeKind::CheckBox || layout_node.kind() == Layout::RustFFI::NodeKind::RadioButton;
}

static void push_form_control_paint_facts_onto(HTML::HTMLInputElement const& input, Layout::Node const& layout_node)
{
    Layout::RustFFI::FfiFormControlPaintFacts facts {
        .enabled = input.enabled(),
        .checked = input.checked(),
        .indeterminate = input.indeterminate(),
        .being_activated = input.is_being_activated(),
    };
    Layout::RustFFI::layout_arena_set_form_control_paint_facts(layout_node.arena_handle(), Layout::Node::slot_id(&layout_node), facts);
}

void push_form_control_paint_facts(HTML::HTMLInputElement& input)
{
    auto const* layout_node = input.unsafe_layout_node();
    if (!layout_node || !paints_form_control_from_facts(*layout_node))
        return;
    input.document().invalidation_journal().note_form_control_paint_facts(
        DOM::NodeIdentity::of(input), input.enabled(), input.checked(), input.indeterminate(), input.is_being_activated());
}

static void push_canvas_paint_facts_onto(HTML::HTMLCanvasElement const& canvas, Layout::Node const& layout_node)
{
    Layout::RustFFI::FfiCanvasPaintFacts facts {};
    if (auto content_size = canvas.canvas_surface_content_size(); content_size.has_value()) {
        facts.has_content = true;
        facts.content_width = content_size->width();
        facts.content_height = content_size->height();
        facts.canvas_id = canvas.canvas_id().value().value();
        facts.content_generation = canvas.content_generation();
    }
    bool changed = Layout::RustFFI::layout_arena_set_canvas_paint_facts(layout_node.arena_handle(), Layout::Node::slot_id(&layout_node), facts);
    // This reconciles facts while a layout row is being published, so the damage belongs to that
    // publication rather than a later identity-based journal entry.
    if (changed && has_committed_box(layout_node))
        apply_paint_cache_invalidation(layout_node, PaintCacheInvalidation::PaintAndHitTest, PaintCacheInvalidationStage::PaintFactReconciliation);
}

void push_canvas_paint_facts(HTML::HTMLCanvasElement const& canvas)
{
    auto const* layout_node = canvas.unsafe_layout_node();
    if (!layout_node || layout_node->kind() != Layout::RustFFI::NodeKind::CanvasBox)
        return;
    auto content_size = canvas.canvas_surface_content_size();
    const_cast<DOM::Document&>(canvas.document()).invalidation_journal().note_canvas_paint_facts(DOM::NodeIdentity::of(canvas), content_size.has_value(), content_size.has_value() ? content_size->width() : 0, content_size.has_value() ? content_size->height() : 0, content_size.has_value() ? canvas.canvas_id().value().value() : 0, content_size.has_value() ? canvas.content_generation() : 0);
}

static Optional<u64> composited_context_id_for_navigable_container(HTML::NavigableContainer const& navigable_container)
{
    auto content_navigable = navigable_container.content_navigable();
    if (!content_navigable || content_navigable->has_been_destroyed())
        return {};
    Optional<Compositing::CompositorContextId> context_id;
    if (auto const* remote_navigable = as_if<HTML::RemoteNavigable>(*content_navigable)) {
        // The content is composited by the process hosting it.
        context_id = remote_navigable->compositor_context_id();
    } else {
        auto const& local_navigable = as<HTML::LocalNavigable>(*content_navigable);
        if (local_navigable.has_compositor_context()) {
            auto const* hosted_document = navigable_container.content_document_without_origin_check();
            if (!hosted_document || !hosted_document->is_render_blocked())
                context_id = local_navigable.compositor_context().id();
        }
    }
    if (!context_id.has_value())
        return {};
    return context_id->value();
}

enum class ReconcilingBeforeRecording : u8 {
    No,
    Yes,
};

static void push_navigable_container_paint_facts_onto(HTML::NavigableContainer const& navigable_container, Layout::Node const& layout_node, ReconcilingBeforeRecording reconciling = ReconcilingBeforeRecording::No)
{
    Layout::RustFFI::FfiNavigableContainerPaintFacts facts {};
    if (auto context_id = composited_context_id_for_navigable_container(navigable_container); context_id.has_value()) {
        facts.has_composited_context = true;
        facts.composited_context_id = *context_id;
    }
    // Only a navigable this process hosts: an event over content hosted elsewhere goes to the
    // process hosting it, and a local navigable standing in for that content carries the same id.
    if (auto content_navigable = navigable_container.content_navigable(); content_navigable && (is<HTML::LocalNavigable>(*content_navigable))) {
        facts.local_content_navigable.namespace_id = content_navigable->id().namespace_id;
        facts.local_content_navigable.local_id = content_navigable->id().local_id;
    }
    bool changed = Layout::RustFFI::layout_arena_set_navigable_container_paint_facts(layout_node.arena_handle(), Layout::Node::slot_id(&layout_node), facts);
    if (!changed)
        return;
    if (reconciling == ReconcilingBeforeRecording::Yes) {
        // Recording reads these facts immediately below its call site, after the journal has
        // already drained. Reconcile and damage them as one named pre-recording stage.
        apply_paint_cache_invalidation(layout_node, PaintCacheInvalidation::PaintAndHitTest, PaintCacheInvalidationStage::PaintFactReconciliation);
        return;
    }
    if (has_committed_box(layout_node))
        invalidate_paint_cache(layout_node);
}

// The facts are read from the container as it is at the drain.
void push_navigable_container_paint_facts(HTML::NavigableContainer const& navigable_container)
{
    auto const* layout_node = navigable_container.unsafe_layout_node();
    if (!layout_node || layout_node->kind() != Layout::RustFFI::NodeKind::NavigableContainerViewport)
        return;
    const_cast<DOM::Document&>(navigable_container.document()).invalidation_journal().note_paint_facts(DOM::NodeIdentity::of(navigable_container), DOM::PaintFactsFamily::NavigableContainer, [](Layout::Node const& current_layout_node) {
        if (current_layout_node.kind() != Layout::RustFFI::NodeKind::NavigableContainerViewport)
            return;
        push_navigable_container_paint_facts_onto(as<HTML::NavigableContainer>(*current_layout_node.dom_node()), current_layout_node);
    });
}

void reconcile_navigable_container_paint_facts(DOM::Document const& document)
{
    for (auto const* navigable_container : HTML::NavigableContainer::all_instances()) {
        if (&navigable_container->document() != &document)
            continue;
        auto const* layout_node = navigable_container->layout_node();
        if (!layout_node || !is_navigable_container_viewport_paintable(*layout_node))
            continue;
        push_navigable_container_paint_facts_onto(*navigable_container, *layout_node, ReconcilingBeforeRecording::Yes);
    }
}

static Layout::RustFFI::FfiNaturalSize natural_size_facts(Optional<CSSPixels> width, Optional<CSSPixels> height, Optional<CSSPixelFraction> aspect_ratio)
{
    Layout::RustFFI::FfiNaturalSize natural {};
    natural.width = width;
    natural.height = height;
    if (aspect_ratio.has_value()) {
        natural.has_aspect_ratio = true;
        natural.aspect_ratio_numerator = aspect_ratio->numerator();
        natural.aspect_ratio_denominator = aspect_ratio->denominator();
    }
    return natural;
}

static Layout::RustFFI::FfiImageContent image_content_facts(GC::Ptr<HTML::DecodedImageData> decoded_image_data, Optional<Gfx::ImageFrameHandle>& current_frame_handle)
{
    Layout::RustFFI::FfiImageContent content {};
    if (!decoded_image_data)
        return content;
    if (auto const* svg_image_data = as_if<SVG::SVGDecodedImageData>(*decoded_image_data)) {
        content.kind = Layout::RustFFI::FfiImageContentKind::Vector;
        content.vector_content_identity = svg_image_data->vector_content_identity();
        content.vector_image_identity = svg_image_data->vector_image_identity();
        content.vector_has_active_view_box = svg_image_data->has_active_view_box();
        return content;
    }
    content.kind = Layout::RustFFI::FfiImageContentKind::Raster;
    if (auto current_frame = decoded_image_data->current_frame(); current_frame.has_value()) {
        current_frame_handle.emplace(current_frame.value());
        content.frame_id = current_frame_handle->id();
    }
    return content;
}

static Layout::RustFFI::FfiLayerImagePaintFacts layer_image_paint_facts_for(CSS::AbstractImageStyleValue const& image, GC::Ptr<HTML::DecodedImageData> decoded_image_data, Optional<Gfx::ImageFrameHandle>& current_frame_handle)
{
    Layout::RustFFI::FfiLayerImagePaintFacts facts {};
    facts.is_paintable = image.is_paintable(decoded_image_data);
    facts.content = image_content_facts(decoded_image_data, current_frame_handle);
    if (decoded_image_data) {
        auto natural_size = image.natural_size(*decoded_image_data);
        facts.natural = natural_size_facts(natural_size.width, natural_size.height, natural_size.aspect_ratio);
        facts.single_pixel_color = decoded_image_data->color_if_single_pixel_bitmap();
    }
    if (auto const* image_set = as_if<CSS::ImageSetStyleValue>(image)) {
        if (auto selected_option_index = image_set->selected_option_index(); selected_option_index.has_value()) {
            facts.has_image_set_selected_option = true;
            facts.image_set_selected_option_index = *selected_option_index;
        }
    }
    return facts;
}

static GC::Ptr<HTML::DecodedImageData> decoded_image_data_of(Layout::NodeWithStyle::ImageObserver const* observer)
{
    if (!observer)
        return nullptr;
    return observer->decoded_image_data();
}

static DOM::NodeIdentity paint_facts_journal_anchor(Layout::Node const& layout_node)
{
    for (auto const* ancestor = &layout_node; ancestor; ancestor = ancestor->parent_ptr()) {
        if (auto identity = ancestor->dom_node_identity(); identity)
            return identity;
    }
    return {};
}

// The update is handed the row its entry is anchored to at the drain, and finds the row the facts
// are for from there.
static void note_paint_facts(Layout::Node const& layout_node, DOM::PaintFactsFamily family, Function<void(Layout::Node const&)>&& update)
{
    auto identity = paint_facts_journal_anchor(layout_node);
    auto& journal = const_cast<DOM::Document&>(layout_node.document()).invalidation_journal();
    if (identity)
        journal.note_paint_facts(identity, family, move(update));
    else
        journal.note_unanchored_paint_facts(Layout::Node::slot_id(&layout_node), move(update));
}

void push_layer_image_paint_facts(Layout::NodeWithStyle const& layout_node)
{
    auto const& background_layers = layout_node.background_layers();
    auto const& mask_layers = layout_node.mask_layers();
    Vector<Layout::RustFFI::FfiLayerImagePaintFactsEntry> entries;
    Vector<Optional<Gfx::ImageFrameHandle>> current_frame_handles;
    current_frame_handles.ensure_capacity(background_layers.size() + mask_layers.size() + 1);
    auto append_entry = [&](Layout::RustFFI::FfiLayerImageList list, size_t computed_index, CSS::AbstractImageStyleValue const* image, Layout::NodeWithStyle::ImageObserver const* observer) {
        if (!image)
            return;
        current_frame_handles.append({});
        entries.append({
            .list = list,
            .computed_index = static_cast<u32>(computed_index),
            .facts = layer_image_paint_facts_for(*image, decoded_image_data_of(observer), current_frame_handles.last()),
        });
    };
    for (size_t layer_index = 0; layer_index < background_layers.size(); ++layer_index)
        append_entry(Layout::RustFFI::FfiLayerImageList::Background, layer_index, background_layers[layer_index].background_image.ptr(), layout_node.background_image_observer(layer_index));
    for (size_t layer_index = 0; layer_index < mask_layers.size(); ++layer_index)
        append_entry(Layout::RustFFI::FfiLayerImageList::Mask, layer_index, mask_layers[layer_index].background_image.ptr(), layout_node.mask_image_observer(layer_index));
    append_entry(Layout::RustFFI::FfiLayerImageList::BorderImageSource, 0, layout_node.border_image().source.ptr(), layout_node.border_image_source_observer());
    auto target_slot = Layout::Node::slot_id(&layout_node);
    note_paint_facts(layout_node, DOM::PaintFactsFamily::LayerImage, [target_slot, entries = move(entries), current_frame_handles = move(current_frame_handles)](Layout::Node const& anchor_layout_node) {
        (void)current_frame_handles;
        auto* current_layout_node = anchor_layout_node.node_arena().node_if_live(target_slot);
        if (!current_layout_node)
            return;
        Layout::RustFFI::layout_arena_set_layer_image_paint_facts(current_layout_node->arena_handle(), Layout::Node::slot_id(current_layout_node), entries.data(), entries.size());
    });
}

void push_replaced_image_paint_facts(Layout::ImageProvider const& image_provider, Layout::Node const& layout_node)
{
    if (layout_node.kind() != Layout::RustFFI::NodeKind::ImageBox && layout_node.kind() != Layout::RustFFI::NodeKind::SVGImageBox)
        return;
    Optional<Gfx::ImageFrameHandle> current_frame_handle;
    Layout::RustFFI::FfiReplacedImagePaintFacts facts {
        .natural = natural_size_facts(image_provider.intrinsic_width(), image_provider.intrinsic_height(), image_provider.intrinsic_aspect_ratio()),
        .content = image_content_facts(image_provider.decoded_image_data(), current_frame_handle),
    };
    auto target_slot = Layout::Node::slot_id(&layout_node);
    note_paint_facts(layout_node, DOM::PaintFactsFamily::ReplacedImage, [target_slot, facts, current_frame_handle = move(current_frame_handle)](Layout::Node const& anchor_layout_node) {
        (void)current_frame_handle;
        auto* current_layout_node = anchor_layout_node.node_arena().node_if_live(target_slot);
        if (!current_layout_node)
            return;
        if (current_layout_node->kind() != Layout::RustFFI::NodeKind::ImageBox && current_layout_node->kind() != Layout::RustFFI::NodeKind::SVGImageBox)
            return;
        if (Layout::RustFFI::layout_arena_set_replaced_image_paint_facts(current_layout_node->arena_handle(), Layout::Node::slot_id(current_layout_node), facts))
            set_needs_repaint(*current_layout_node, InvalidateDisplayList::PaintCommands);
    });
}

static void push_video_paint_facts_onto(HTML::HTMLVideoElement const& video_element, Layout::Node const& layout_node)
{
    Layout::RustFFI::FfiVideoPaintFacts facts {};
    Optional<Gfx::ImageFrameHandle> poster_frame_handle;
    switch (video_element.current_representation()) {
    case HTML::HTMLVideoElement::Representation::FirstVideoFrame:
    case HTML::HTMLVideoElement::Representation::VideoFrame: {
        facts.representation = Layout::RustFFI::FfiVideoRepresentation::VideoFrame;
        auto sink_handle = video_element.video_sink_handle();
        if (sink_handle.has_value() && video_element.natural_media_size().has_value()) {
            facts.has_video_frame = true;
            auto src_size = video_element.natural_media_size()->to_type<int>();
            facts.video_src_width = src_size.width();
            facts.video_src_height = src_size.height();
            facts.video_sink_resource_id = video_element.video_sink_resource_id().value().value();
            facts.video_sink_handle = sink_handle->value();
        }
        break;
    }
    case HTML::HTMLVideoElement::Representation::PosterFrame:
        facts.representation = Layout::RustFFI::FfiVideoRepresentation::PosterFrame;
        if (auto const& poster_frame = video_element.poster_frame(); poster_frame.has_value()) {
            poster_frame_handle.emplace(poster_frame.value());
            facts.poster_frame_id = poster_frame_handle->id();
        }
        break;
    case HTML::HTMLVideoElement::Representation::TransparentBlack:
        facts.representation = Layout::RustFFI::FfiVideoRepresentation::TransparentBlack;
        break;
    }
    auto target_slot = Layout::Node::slot_id(&layout_node);
    note_paint_facts(layout_node, DOM::PaintFactsFamily::Video, [target_slot, facts, poster_frame_handle = move(poster_frame_handle)](Layout::Node const& anchor_layout_node) {
        (void)poster_frame_handle;
        auto* current_layout_node = anchor_layout_node.node_arena().node_if_live(target_slot);
        if (!current_layout_node || current_layout_node->kind() != Layout::RustFFI::NodeKind::VideoBox)
            return;
        if (Layout::RustFFI::layout_arena_set_video_paint_facts(current_layout_node->arena_handle(), Layout::Node::slot_id(current_layout_node), facts))
            set_needs_repaint(*current_layout_node, InvalidateDisplayList::PaintCommands);
    });
}

void push_video_paint_facts(HTML::HTMLVideoElement const& video_element)
{
    auto const* layout_node = video_element.unsafe_layout_node();
    if (!layout_node || layout_node->kind() != Layout::RustFFI::NodeKind::VideoBox)
        return;
    push_video_paint_facts_onto(video_element, *layout_node);
}

// The `<area>` elements of the image map an image is associated with, in tree order, each named
// by its style-tree identity, because that is what a hit hands back. An area is never rendered, so
// it has no row of its own to carry its shape or its editability; the image whose map lists it
// does.
static void push_image_map_area_facts_onto(HTML::HTMLImageElement& image_element, Layout::Node const& layout_node)
{
    Vector<Layout::RustFFI::FfiImageMapArea> areas;
    Vector<double> coords;
    if (auto map_element = image_element.associated_map_element()) {
        map_element->for_each_in_subtree_of_type<HTML::HTMLAreaElement>([&](HTML::HTMLAreaElement& area_element) {
            auto area_coords = area_element.shape_coords();
            areas.append({
                .style_node = DOM::NodeIdentity::of(area_element).style_node().value(),
                .shape = to_underlying(area_element.shape_state()),
                .editable = static_cast<u8>(area_element.is_editable_or_editing_host()),
                .coords_offset = static_cast<u32>(coords.size()),
                .coords_count = static_cast<u32>(area_coords.size()),
            });
            coords.extend(move(area_coords));
            return TraversalDecision::Continue;
        });
    }
    Layout::RustFFI::layout_arena_publish_image_map_areas(layout_node.arena_handle(), Layout::Node::slot_id(&layout_node), areas.data(), areas.size(), coords.data(), coords.size());
}

void push_image_map_area_facts(HTML::HTMLImageElement& image_element)
{
    // Any box an image has answers for its map, including the one it takes when it renders as its
    // alt text, which is where the association was read from before it was published.
    if (auto const* layout_node = image_element.unsafe_layout_node())
        push_image_map_area_facts_onto(image_element, *layout_node);
}

// Which map an image is associated with is a hash-name reference resolved against the image's
// root, so any map or area of the document can decide any image's areas and there is no smaller
// funnel than the document. Nearly every page has no image map at all, and this runs only when one
// of their elements changes.
void refresh_image_map_area_facts(DOM::Document& document)
{
    document.for_each_shadow_including_descendant([](DOM::Node& node) {
        if (auto* image_element = as_if<HTML::HTMLImageElement>(node))
            push_image_map_area_facts(*image_element);
        return TraversalDecision::Continue;
    });
}

static void push_image_box_paint_facts(Layout::Box const& image_box)
{
    // A box that owns its image's provider is handed it once the frame that built the box is over,
    // and handing it over pushes these facts. A restyle within that frame has no provider to read.
    if (Layout::RustFFI::layout_arena_image_box_awaits_owned_provider(image_box.arena_handle(), Layout::Node::slot_id(&image_box)))
        return;
    push_replaced_image_paint_facts(image_box.image_provider(), image_box);
}

void push_paint_facts_after_style_attach(Layout::NodeWithStyle& layout_node, StyleHoldsImageValues style_holds_image_values)
{
    if (auto* image_element = as_if<HTML::HTMLImageElement>(layout_node.dom_node()))
        push_image_map_area_facts_onto(*image_element, layout_node);
    if (style_holds_image_values == StyleHoldsImageValues::Yes)
        push_layer_image_paint_facts(layout_node);
    else {
        auto clear_layer_image_paint_facts = [](Layout::Node const& current_layout_node) {
            Layout::RustFFI::layout_arena_set_layer_image_paint_facts(current_layout_node.arena_handle(), Layout::Node::slot_id(&current_layout_node), nullptr, 0);
        };
        auto& journal = const_cast<DOM::Document&>(layout_node.document()).invalidation_journal();
        if (auto identity = layout_node.dom_node_identity())
            journal.note_paint_facts(identity, DOM::PaintFactsFamily::LayerImage, move(clear_layer_image_paint_facts));
        else
            journal.note_unanchored_paint_facts(Layout::Node::slot_id(&layout_node), move(clear_layer_image_paint_facts));
    }
    if (paints_form_control_from_facts(layout_node))
        push_form_control_paint_facts_onto(as<HTML::HTMLInputElement>(*layout_node.dom_node()), layout_node);
    else if (layout_node.kind() == Layout::RustFFI::NodeKind::CanvasBox)
        push_canvas_paint_facts_onto(as<HTML::HTMLCanvasElement>(*layout_node.dom_node()), layout_node);
    else if (layout_node.kind() == Layout::RustFFI::NodeKind::ImageBox)
        push_image_box_paint_facts(static_cast<Layout::Box const&>(layout_node));
    else if (layout_node.kind() == Layout::RustFFI::NodeKind::SVGImageBox)
        push_replaced_image_paint_facts(as<SVG::SVGImageElement>(*layout_node.dom_node()), layout_node);
    else if (layout_node.kind() == Layout::RustFFI::NodeKind::VideoBox)
        push_video_paint_facts_onto(as<HTML::HTMLVideoElement>(*layout_node.dom_node()), layout_node);
    else if (layout_node.kind() == Layout::RustFFI::NodeKind::NavigableContainerViewport)
        push_navigable_container_paint_facts_onto(as<HTML::NavigableContainer>(*layout_node.dom_node()), layout_node);
}

}
