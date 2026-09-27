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
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
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

static Layout::Row bound_row(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto const* arena = document.layout_node_arena_if_created();
    return arena && identity ? identity.bound_row(*arena) : Layout::Row {};
}

static void push_form_control_paint_facts_onto(HTML::HTMLInputElement const& input, Layout::Row const& row)
{
    Layout::RustFFI::FfiFormControlPaintFacts facts {
        .enabled = input.enabled(),
        .checked = input.checked(),
        .indeterminate = input.indeterminate(),
        .being_activated = input.is_being_activated(),
    };
    Layout::RustFFI::layout_arena_set_form_control_paint_facts(row.arena_handle(), row.slot(), facts);
}

void push_form_control_paint_facts(HTML::HTMLInputElement& input)
{
    // The journal finds the box as it drains and gives the facts only to a box that paints from them. Finding the box
    // here would read its style record, which a style pass in flight owns.
    using enum HTML::HTMLInputElement::TypeAttributeState;
    if (!input.has_layout_box() || !first_is_one_of(input.type_state(), Checkbox, RadioButton))
        return;
    input.document().invalidation_journal().note_form_control_paint_facts(
        DOM::NodeIdentity::of(input), input.enabled(), input.checked(), input.indeterminate(), input.is_being_activated());
}

static void push_canvas_paint_facts_onto(HTML::HTMLCanvasElement const& canvas, Layout::Row const& row)
{
    Layout::RustFFI::FfiCanvasPaintFacts facts {};
    if (auto content_size = canvas.canvas_surface_content_size(); content_size.has_value()) {
        facts.has_content = true;
        facts.content_width = content_size->width();
        facts.content_height = content_size->height();
        facts.canvas_id = canvas.canvas_id().value().value();
        facts.content_generation = canvas.content_generation();
    }
    bool changed = Layout::RustFFI::layout_arena_set_canvas_paint_facts(row.arena_handle(), row.slot(), facts);
    // This reconciles facts while a layout row is being published, so the damage belongs to that
    // publication rather than a later identity-based journal entry.
    if (changed && has_committed_box(row))
        apply_paint_cache_invalidation(row, PaintCacheInvalidation::PaintAndHitTest);
}

void push_canvas_paint_facts(HTML::HTMLCanvasElement const& canvas)
{
    if (bound_row_kind(canvas.document(), DOM::NodeIdentity::of(canvas)) != Layout::RustFFI::NodeKind::CanvasBox)
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

static void push_navigable_container_paint_facts_onto(HTML::NavigableContainer const& navigable_container, Layout::Row const& row, ReconcilingBeforeRecording reconciling = ReconcilingBeforeRecording::No)
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
    bool changed = Layout::RustFFI::layout_arena_set_navigable_container_paint_facts(row.arena_handle(), row.slot(), facts);
    if (!changed)
        return;
    if (reconciling == ReconcilingBeforeRecording::Yes) {
        // Recording reads these facts immediately below its call site, after the journal has
        // already drained. Reconcile and damage them as one named pre-recording stage.
        apply_paint_cache_invalidation(row, PaintCacheInvalidation::PaintAndHitTest);
        return;
    }
    if (has_committed_box(row))
        invalidate_paint_cache(navigable_container.document(), DOM::NodeIdentity::of(navigable_container));
}

// The facts are read from the container as it is at the drain.
void push_navigable_container_paint_facts(HTML::NavigableContainer const& navigable_container)
{
    // Beside a frame that owns the arena, the box is looked up and its facts noted once the frame has been taken in.
    if (HTML::FrameScheduler::arena_changes_wait_for_frame(navigable_container.document())) {
        HTML::main_thread_event_loop().frame_scheduler().defer_arena_change(GC::create_function(navigable_container.heap(), [navigable_container = GC::Ref { navigable_container }] {
            push_navigable_container_paint_facts(navigable_container);
        }));
        return;
    }
    if (bound_row_kind(navigable_container.document(), DOM::NodeIdentity::of(navigable_container)) != Layout::RustFFI::NodeKind::NavigableContainerViewport)
        return;
    const_cast<DOM::Document&>(navigable_container.document()).invalidation_journal().note_paint_facts(DOM::NodeIdentity::of(navigable_container), DOM::PaintFactsFamily::NavigableContainer, [](Layout::Row const& current_row) {
        if (current_row.kind() != Layout::RustFFI::NodeKind::NavigableContainerViewport)
            return;
        push_navigable_container_paint_facts_onto(as<HTML::NavigableContainer>(*current_row.shell().dom_node()), current_row);
    });
}

void reconcile_navigable_container_paint_facts(DOM::Document const& document, ReconcileAheadOfLayout ahead_of_layout)
{
    for (auto const* navigable_container : HTML::NavigableContainer::all_instances()) {
        if (&navigable_container->document() != &document)
            continue;
        // Ahead of the layout a flight runs, the boxes are those the last layout left. A box the flight's tree build
        // makes is given its facts as that build's host half commits it.
        auto row = bound_row(document, DOM::NodeIdentity::of(*navigable_container));
        if (row && ahead_of_layout == ReconcileAheadOfLayout::No)
            VERIFY(document.layout_is_up_to_date());
        if (!row || row.kind() != Layout::RustFFI::NodeKind::NavigableContainerViewport || !has_committed_box(row))
            continue;
        push_navigable_container_paint_facts_onto(*navigable_container, row, ReconcilingBeforeRecording::Yes);
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

static DOM::NodeIdentity paint_facts_journal_anchor(Layout::Row row)
{
    for (; row; row = row.linked(Layout::RustFFI::FfiNodeLink::Parent)) {
        if (auto identity = row.dom_node_identity(); identity)
            return identity;
    }
    return {};
}

// The update is handed the row its entry is anchored to at the drain, and finds the row the facts
// are for from there.
static void note_paint_facts(Layout::Row const& row, DOM::PaintFactsFamily family, DOM::PaintFactsUpdate&& update)
{
    auto identity = paint_facts_journal_anchor(row);
    auto& journal = row.document().invalidation_journal();
    if (identity)
        journal.note_paint_facts(identity, family, move(update));
    else
        journal.note_unanchored_paint_facts(row.slot(), move(update));
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
    note_paint_facts(layout_node, DOM::PaintFactsFamily::LayerImage, [target_slot, entries = move(entries), current_frame_handles = move(current_frame_handles)](Layout::Row const& anchor_row) {
        (void)current_frame_handles;
        auto current_row = anchor_row.arena().row_if_live(target_slot);
        if (!current_row)
            return;
        Layout::RustFFI::layout_arena_set_layer_image_paint_facts(current_row.arena_handle(), current_row.slot(), entries.data(), entries.size());
    });
}

void push_replaced_image_paint_facts(Layout::ImageProvider const& image_provider, Layout::Row const& row)
{
    if (row.kind() != Layout::RustFFI::NodeKind::ImageBox && row.kind() != Layout::RustFFI::NodeKind::SVGImageBox)
        return;
    Optional<Gfx::ImageFrameHandle> current_frame_handle;
    Layout::RustFFI::FfiReplacedImagePaintFacts facts {
        .natural = natural_size_facts(image_provider.intrinsic_width(), image_provider.intrinsic_height(), image_provider.intrinsic_aspect_ratio()),
        .content = image_content_facts(image_provider.decoded_image_data(), current_frame_handle),
    };
    auto target_slot = row.slot();
    note_paint_facts(row, DOM::PaintFactsFamily::ReplacedImage, [target_slot, facts, current_frame_handle = move(current_frame_handle)](Layout::Row const& anchor_row) {
        (void)current_frame_handle;
        auto current_row = anchor_row.arena().row_if_live(target_slot);
        if (!current_row)
            return;
        if (current_row.kind() != Layout::RustFFI::NodeKind::ImageBox && current_row.kind() != Layout::RustFFI::NodeKind::SVGImageBox)
            return;
        if (Layout::RustFFI::layout_arena_set_replaced_image_paint_facts(current_row.arena_handle(), current_row.slot(), facts))
            set_needs_repaint(current_row, InvalidateDisplayList::PaintCommands);
    });
}

static void push_video_paint_facts_onto(HTML::HTMLVideoElement const& video_element, Layout::Row const& row)
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
    auto target_slot = row.slot();
    note_paint_facts(row, DOM::PaintFactsFamily::Video, [target_slot, facts, poster_frame_handle = move(poster_frame_handle)](Layout::Row const& anchor_row) {
        (void)poster_frame_handle;
        auto current_row = anchor_row.arena().row_if_live(target_slot);
        if (!current_row || current_row.kind() != Layout::RustFFI::NodeKind::VideoBox)
            return;
        if (Layout::RustFFI::layout_arena_set_video_paint_facts(current_row.arena_handle(), current_row.slot(), facts))
            set_needs_repaint(current_row, InvalidateDisplayList::PaintCommands);
    });
}

void push_video_paint_facts(HTML::HTMLVideoElement const& video_element)
{
    // Beside a frame that owns the arena, the box is looked up and its facts pushed once the frame has been taken in.
    if (HTML::FrameScheduler::arena_changes_wait_for_frame(video_element.document())) {
        HTML::main_thread_event_loop().frame_scheduler().defer_arena_change(GC::create_function(video_element.heap(), [video_element = GC::Ref { video_element }] {
            push_video_paint_facts(video_element);
        }));
        return;
    }
    auto row = bound_row(video_element.document(), DOM::NodeIdentity::of(video_element));
    if (!row || row.kind() != Layout::RustFFI::NodeKind::VideoBox)
        return;
    push_video_paint_facts_onto(video_element, row);
}

// The `<area>` elements of the image map an image is associated with, in tree order, each named
// by its style-tree identity, because that is what a hit hands back. An area is never rendered, so
// it has no row of its own to carry its shape or its editability; the image whose map lists it
// does.
static void push_image_map_area_facts_onto(HTML::HTMLImageElement& image_element, Layout::Row const& row)
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
    Layout::RustFFI::layout_arena_publish_image_map_areas(row.arena_handle(), row.slot(), areas.data(), areas.size(), coords.data(), coords.size());
}

void push_image_map_area_facts(HTML::HTMLImageElement& image_element)
{
    // Any box an image has answers for its map, including the one it takes when it renders as its
    // alt text, which is where the association was read from before it was published.
    if (auto row = bound_row(image_element.document(), DOM::NodeIdentity::of(image_element)))
        push_image_map_area_facts_onto(image_element, row);
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

void push_paint_facts_after_style_attach(Layout::Row const& row, DOM::Node* dom_node, StyleHoldsImageValues style_holds_image_values)
{
    if (auto* image_element = as_if<HTML::HTMLImageElement>(dom_node))
        push_image_map_area_facts_onto(*image_element, row);
    if (style_holds_image_values == StyleHoldsImageValues::Yes) {
        push_layer_image_paint_facts(as<Layout::NodeWithStyle>(row.shell()));
    } else {
        auto& journal = row.document().invalidation_journal();
        if (auto identity = row.dom_node_identity()) {
            journal.note_layer_image_paint_facts_cleared(identity);
        } else {
            journal.note_unanchored_paint_facts(row.slot(), [](Layout::Row const& current_row) {
                Layout::RustFFI::layout_arena_set_layer_image_paint_facts(current_row.arena_handle(), current_row.slot(), nullptr, 0);
            });
        }
    }
    // Only these kinds of boxes paint from facts their DOM node keeps; the style of any other box is all it paints from.
    // A box kept after its node went away beside the frame that built it (removed, or adopted into another document)
    // has no node to take them from, and paints from its style until the node's removal takes it away.
    if (!dom_node && row.kind() != Layout::RustFFI::NodeKind::ImageBox)
        return;
    switch (row.kind()) {
    case Layout::RustFFI::NodeKind::CheckBox:
    case Layout::RustFFI::NodeKind::RadioButton:
        push_form_control_paint_facts_onto(as<HTML::HTMLInputElement>(*dom_node), row);
        break;
    case Layout::RustFFI::NodeKind::CanvasBox:
        push_canvas_paint_facts_onto(as<HTML::HTMLCanvasElement>(*dom_node), row);
        break;
    case Layout::RustFFI::NodeKind::ImageBox:
        push_image_box_paint_facts(static_cast<Layout::Box const&>(row.shell()));
        break;
    case Layout::RustFFI::NodeKind::SVGImageBox:
        push_replaced_image_paint_facts(as<SVG::SVGImageElement>(*dom_node), row);
        break;
    case Layout::RustFFI::NodeKind::VideoBox:
        push_video_paint_facts_onto(as<HTML::HTMLVideoElement>(*dom_node), row);
        break;
    case Layout::RustFFI::NodeKind::NavigableContainerViewport:
        push_navigable_container_paint_facts_onto(as<HTML::NavigableContainer>(*dom_node), row);
        break;
    default:
        break;
    }
}

}
