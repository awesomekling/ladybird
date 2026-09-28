/*
 * Copyright (c) 2026, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/StdLibExtras.h>
#include <AK/StringBuilder.h>
#include <LibCompositing/DisplayList/AccumulatedVisualContext.h>
#include <LibCompositing/DisplayList/DisplayList.h>
#include <LibCompositing/DisplayList/DisplayListCommand.h>
#include <LibCompositing/DisplayList/DisplayListResourceStorage.h>
#include <LibCore/ElapsedTimer.h>
#include <LibCore/Environment.h>
#include <LibGfx/CornerRadii.h>
#include <LibGfx/Filter.h>
#include <LibGfx/GradientInterpolation.h>
#include <LibGfx/Matrix4x4.h>
#include <LibGfx/Path.h>
#include <LibGfx/TextLayout.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/StyleValues/AbstractImageStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorStyleValue.h>
#include <LibWeb/CSS/VisualViewport.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/HTML/FormAssociatedElement.h>
#include <LibWeb/HTML/HTMLBRElement.h>
#include <LibWeb/HTML/HTMLCanvasElement.h>
#include <LibWeb/HTML/HTMLHtmlElement.h>
#include <LibWeb/HTML/HTMLImageElement.h>
#include <LibWeb/HTML/HTMLInputElement.h>
#include <LibWeb/HTML/HTMLVideoElement.h>
#include <LibWeb/HTML/ImageRequest.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/NavigableContainer.h>
#include <LibWeb/Layout/ImageProvider.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Page/EventHandler.h>
#include <LibWeb/Page/MiddleButtonScrollHandler.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/Blending.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/ChromeMetrics.h>
#include <LibWeb/Painting/ChromeWidget.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/ImagePaint.h>
#include <LibWeb/Painting/PaintFacts.h>
#include <LibWeb/Painting/PaintingRustBridge.h>
#include <LibWeb/Painting/ResizeHandle.h>
#include <LibWeb/Painting/ScrollSnap.h>
#include <LibWeb/Painting/Scrollbar.h>
#include <LibWeb/Painting/Scrolling.h>
#include <LibWeb/Platform/FontPlugin.h>
#include <LibWeb/SVG/SVGClipPathElement.h>
#include <LibWeb/SVG/SVGDecodedImageData.h>
#include <LibWeb/SVG/SVGFilterElement.h>
#include <LibWeb/SVG/SVGGradientElement.h>
#include <LibWeb/SVG/SVGGraphicsElement.h>
#include <LibWeb/SVG/SVGImageElement.h>
#include <LibWeb/SVG/SVGMaskElement.h>
#include <LibWebCommon/CSS/SystemColor.h>

namespace Web::Painting {

SubmittedRecordingTicket SubmittedRecordingTicket::adopt(void const* ticket)
{
    SubmittedRecordingTicket adopted;
    adopted.m_ticket = ticket;
    return adopted;
}

SubmittedRecordingTicket::SubmittedRecordingTicket(SubmittedRecordingTicket const& other)
    : m_ticket(other.m_ticket ? Layout::RustFFI::layout_recording_ticket_retain(other.m_ticket) : nullptr)
{
}

SubmittedRecordingTicket::SubmittedRecordingTicket(SubmittedRecordingTicket&& other)
    : m_ticket(exchange(other.m_ticket, nullptr))
{
}

SubmittedRecordingTicket& SubmittedRecordingTicket::operator=(SubmittedRecordingTicket const& other)
{
    if (this != &other) {
        SubmittedRecordingTicket copy { other };
        swap(m_ticket, copy.m_ticket);
    }
    return *this;
}

SubmittedRecordingTicket& SubmittedRecordingTicket::operator=(SubmittedRecordingTicket&& other)
{
    if (this != &other) {
        Layout::RustFFI::layout_recording_ticket_release(m_ticket);
        m_ticket = exchange(other.m_ticket, nullptr);
    }
    return *this;
}

SubmittedRecordingTicket::~SubmittedRecordingTicket()
{
    Layout::RustFFI::layout_recording_ticket_release(m_ticket);
}

bool SubmittedRecordingTicket::is_in_flight() const
{
    return m_ticket && Layout::RustFFI::layout_recording_ticket_is_in_flight(m_ticket);
}

void const* SubmittedRecordingTicket::retain_for_presentation() const
{
    VERIFY(m_ticket);
    return Layout::RustFFI::layout_recording_ticket_retain_for_presentation(m_ticket);
}

static_assert(to_underlying(CSS::FontSmoothing::Auto) == to_underlying(Compositing::FontSmoothing::Auto));
static_assert(to_underlying(CSS::FontSmoothing::None) == to_underlying(Compositing::FontSmoothing::None));
static_assert(to_underlying(CSS::FontSmoothing::Antialiased) == to_underlying(Compositing::FontSmoothing::Antialiased));
static_assert(to_underlying(CSS::FontSmoothing::SubpixelAntialiased) == to_underlying(Compositing::FontSmoothing::SubpixelAntialiased));

static_assert(sizeof(Layout::RustFFI::ScrollDirection) == sizeof(ScrollDirection));
static_assert(to_underlying(Layout::RustFFI::ScrollDirection::Horizontal) == to_underlying(ScrollDirection::Horizontal));
static_assert(to_underlying(Layout::RustFFI::ScrollDirection::Vertical) == to_underlying(ScrollDirection::Vertical));

namespace {

template<typename T>
struct RustOptionalLayout {
    T value;
    bool has_value;
};

}

static_assert(sizeof(CSSPixelRect) == 16);

static_assert(sizeof(RustOptionalLayout<CSSPixels>) == sizeof(Optional<CSSPixels>));
static_assert(alignof(RustOptionalLayout<CSSPixels>) == alignof(Optional<CSSPixels>));
static_assert(sizeof(RustOptionalLayout<CSSPixelRect>) == sizeof(Optional<CSSPixelRect>));
static_assert(alignof(RustOptionalLayout<CSSPixelRect>) == alignof(Optional<CSSPixelRect>));
static_assert(sizeof(RustOptionalLayout<Gfx::IntRect>) == sizeof(Optional<Gfx::IntRect>));
static_assert(alignof(RustOptionalLayout<Gfx::IntRect>) == alignof(Optional<Gfx::IntRect>));
static_assert(sizeof(RustOptionalLayout<float>) == sizeof(Optional<float>));
static_assert(alignof(RustOptionalLayout<float>) == alignof(Optional<float>));
static_assert(sizeof(RustOptionalLayout<Gfx::FloatPoint>) == sizeof(Optional<Gfx::FloatPoint>));
static_assert(alignof(RustOptionalLayout<Gfx::FloatPoint>) == alignof(Optional<Gfx::FloatPoint>));
static_assert(sizeof(RustOptionalLayout<Gfx::FloatSize>) == sizeof(Optional<Gfx::FloatSize>));
static_assert(alignof(RustOptionalLayout<Gfx::FloatSize>) == alignof(Optional<Gfx::FloatSize>));
static_assert(sizeof(RustOptionalLayout<i64>) == sizeof(Optional<i64>));
static_assert(alignof(RustOptionalLayout<i64>) == alignof(Optional<i64>));
static_assert(sizeof(RustOptionalLayout<size_t>) == sizeof(Optional<size_t>));
static_assert(alignof(RustOptionalLayout<size_t>) == alignof(Optional<size_t>));

static_assert(sizeof(Optional<CSSPixels>) == 8);
static_assert(alignof(Optional<CSSPixels>) == 4);

static_assert(sizeof(Compositing::ClipMode) == sizeof(u8));
static_assert(to_underlying(Compositing::ClipMode::Intersect) == 0);
static_assert(to_underlying(Compositing::ClipMode::Difference) == 1);

#define VERIFY_SHARED_FFI_TYPE(type) static_assert(IsTriviallyCopyable<type>)
VERIFY_SHARED_FFI_TYPE(CSSPixels);
VERIFY_SHARED_FFI_TYPE(CSSPixelPoint);
VERIFY_SHARED_FFI_TYPE(CSSPixelSize);
VERIFY_SHARED_FFI_TYPE(CSSPixelRect);
VERIFY_SHARED_FFI_TYPE(Gfx::IntPoint);
VERIFY_SHARED_FFI_TYPE(Gfx::FloatPoint);
VERIFY_SHARED_FFI_TYPE(Gfx::IntSize);
VERIFY_SHARED_FFI_TYPE(Gfx::FloatSize);
VERIFY_SHARED_FFI_TYPE(Gfx::FloatVector3);
VERIFY_SHARED_FFI_TYPE(Gfx::IntRect);
VERIFY_SHARED_FFI_TYPE(Gfx::FloatRect);
VERIFY_SHARED_FFI_TYPE(Gfx::Color);
VERIFY_SHARED_FFI_TYPE(Gfx::AffineTransform);
VERIFY_SHARED_FFI_TYPE(Gfx::FloatMatrix4x4);
VERIFY_SHARED_FFI_TYPE(Gfx::CornerRadius);
VERIFY_SHARED_FFI_TYPE(Gfx::CornerRadii);
VERIFY_SHARED_FFI_TYPE(Gfx::GradientInterpolationMethod);
VERIFY_SHARED_FFI_TYPE(Gfx::WindingRule);
VERIFY_SHARED_FFI_TYPE(Gfx::MaskKind);
VERIFY_SHARED_FFI_TYPE(Gfx::CompositingAndBlendingOperator);
VERIFY_SHARED_FFI_TYPE(Gfx::ScalingMode);
VERIFY_SHARED_FFI_TYPE(Gfx::InterpolationColorSpace);
VERIFY_SHARED_FFI_TYPE(Compositing::ClipMode);
VERIFY_SHARED_FFI_TYPE(ChromeMetrics);
static_assert(sizeof(ChromeMetrics) == 7 * sizeof(CSSPixels));
static_assert(offsetof(ChromeMetrics, scroll_thumb_min_length) == 0);
static_assert(offsetof(ChromeMetrics, scroll_thumb_padding_thin) == sizeof(CSSPixels));
static_assert(offsetof(ChromeMetrics, scroll_thumb_thickness_thin) == 2 * sizeof(CSSPixels));
static_assert(offsetof(ChromeMetrics, scroll_thumb_thickness) == 3 * sizeof(CSSPixels));
static_assert(offsetof(ChromeMetrics, scroll_gutter_thickness) == 4 * sizeof(CSSPixels));
static_assert(offsetof(ChromeMetrics, resize_gripper_size) == 5 * sizeof(CSSPixels));
static_assert(offsetof(ChromeMetrics, resize_gripper_padding) == 6 * sizeof(CSSPixels));
VERIFY_SHARED_FFI_TYPE(Optional<CSSPixels>);
VERIFY_SHARED_FFI_TYPE(Optional<CSSPixelRect>);
VERIFY_SHARED_FFI_TYPE(Optional<Gfx::IntRect>);
VERIFY_SHARED_FFI_TYPE(Optional<Gfx::FloatPoint>);
VERIFY_SHARED_FFI_TYPE(Optional<Gfx::FloatSize>);
VERIFY_SHARED_FFI_TYPE(Optional<Gfx::FloatRect>);
VERIFY_SHARED_FFI_TYPE(Optional<Gfx::Color>);
VERIFY_SHARED_FFI_TYPE(Optional<Gfx::AffineTransform>);
VERIFY_SHARED_FFI_TYPE(Optional<i64>);
VERIFY_SHARED_FFI_TYPE(Optional<size_t>);
VERIFY_SHARED_FFI_TYPE(Optional<u32>);
VERIFY_SHARED_FFI_TYPE(Optional<float>);
#undef VERIFY_SHARED_FFI_TYPE

namespace {

static bool rust_painting_timing_enabled()
{
    static bool enabled = [] {
        auto value = Core::Environment::get("LADYBIRD_RUST_PAINTING_TIMING"sv);
        return value.has_value() && !value->is_empty() && *value != "0"sv;
    }();
    return enabled;
}

}

namespace {

// The render side draws into a viewport it never asks about: this is published before every pass
// that reads it, and a pass reads what was published rather than the document.
void publish_visual_context_tree_inputs(DOM::Document& document)
{
    Compositing::RustFFI::FfiVisualContextTreeInputs inputs {};
    inputs.device_pixels_per_css_pixel = document.page().client().device_pixels_per_css_pixel();
    auto const& visual_viewport = *document.visual_viewport();
    auto offset = visual_viewport.offset().to_type<double>();
    inputs.visual_viewport_offset_x = offset.x();
    inputs.visual_viewport_offset_y = offset.y();
    inputs.visual_viewport_scale = visual_viewport.scale();
    auto viewport_overflow = overflow_values_applied_to_viewport_for_wheel_scrolling(document);
    inputs.viewport_wheel_overflow_x = static_cast<u8>(to_underlying(viewport_overflow.x));
    inputs.viewport_wheel_overflow_y = static_cast<u8>(to_underlying(viewport_overflow.y));
    Layout::RustFFI::layout_arena_publish_visual_context_tree_inputs(Layout::document_layout_arena(document), inputs);
}

}

Optional<Gfx::Filter> filter_from_functions(ReadonlySpan<Compositing::RustFFI::FfiFilterFunction> functions)
{
    ByteBuffer serialized_filter;
    bool has_filter = Layout::RustFFI::layout_arena_filter_functions_serialize(
        functions.data(),
        functions.size(),
        [](void* context, u8 const* bytes, size_t length) {
            static_cast<ByteBuffer*>(context)->append(bytes, length);
        },
        &serialized_filter);
    if (!has_filter)
        return {};
    return Gfx::Filter { move(serialized_filter) };
}

static void* layout_arena_handle(DOM::Document const& document)
{
    return Layout::document_layout_arena(const_cast<DOM::Document&>(document));
}

Layout::RustFFI::FfiVisualContextUpdateOutcome rust_update_accumulated_visual_contexts(DOM::Document& document)
{
    auto update_timer = Core::ElapsedTimer::start_new(Core::TimerType::Precise);
    publish_visual_context_tree_inputs(document);
    auto outcome = Layout::RustFFI::layout_arena_update_accumulated_visual_contexts(layout_arena_handle(document), viewport_row_slot(document));
    if (rust_painting_timing_enabled())
        dbgln("AVC_UPDATE rust={} µs {}", update_timer.elapsed_time().to_microseconds(), outcome.performed_full_build ? "full"sv : "incremental"sv);
    return outcome;
}

static Vector<u32> rust_owned_visual_context_node_indices(void* arena, Compositing::RustFFI::NodeSlotId slot, Layout::RustFFI::FfiVisualContextBoxNodeList list)
{
    Vector<u32> indices;
    indices.resize(Layout::RustFFI::layout_arena_paintable_visual_context_node_count(arena, slot, list));
    if (!indices.is_empty())
        Layout::RustFFI::layout_arena_paintable_visual_context_copy_node_indices(arena, slot, list, indices.data(), indices.size());
    return indices;
}

Vector<u32> rust_owned_visual_context_node_indices(DOM::Document const& document, DOM::NodeIdentity identity, Layout::RustFFI::FfiVisualContextBoxNodeList list)
{
    if (!has_committed_box(document, identity))
        return {};
    return rust_owned_visual_context_node_indices(layout_arena_handle(document), committed_row_slot(document, identity), list);
}

bool rust_background_color_can_be_compositor_animated(BoxSlot const& box)
{
    if (!has_committed_box(box))
        return false;
    return Layout::RustFFI::layout_arena_background_color_can_be_compositor_animated(box.arena(), box.slot());
}

bool rust_background_color_can_be_compositor_animated(DOM::Document const& document, DOM::NodeIdentity identity)
{
    if (!has_committed_box(document, identity))
        return false;
    return Layout::RustFFI::layout_arena_background_color_can_be_compositor_animated(
        layout_arena_handle(document), committed_row_slot(document, identity));
}

void const* retain_rust_main_visual_context_tree(DOM::Document const& document)
{
    auto const* tree = Layout::RustFFI::layout_arena_main_visual_context_tree_retain(layout_arena_handle(document));
    VERIFY(tree);
    return tree;
}

Layout::RustFFI::FfiPhysicalOverflowDirections rust_physical_overflow_directions(BoxSlot const& box)
{
    if (!box)
        return {};
    return Layout::RustFFI::layout_arena_physical_overflow_directions(box.arena(), box.slot());
}

Layout::RustFFI::FfiPhysicalOverflowDirections rust_physical_overflow_directions(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto slot = committed_row_slot(document, identity);
    if (slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return {};
    return Layout::RustFFI::layout_arena_physical_overflow_directions(layout_arena_handle(document), slot);
}

static void set_scroll_offset_from_render_side(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot, CSSPixelPoint offset)
{
    set_scroll_offset(BoxSlot::of(document, slot), offset);
    // The render side reads the offsets it handed over as soon as the handover is done.
    document.drain_invalidation_journal();
}

void register_geometry_host(DOM::Document& document)
{
    Layout::RustFFI::FfiGeometryHostCallbacks callbacks {
        .context = &document,
        .set_scroll_offset = [](void* context, Compositing::RustFFI::NodeSlotId slot, CSSPixelPoint offset) { set_scroll_offset_from_render_side(*static_cast<DOM::Document*>(context), slot, offset); },
    };
    // Registered once the document has made its arena.
    auto* arena = Layout::document_layout_arena_if_created(document);
    if (!arena)
        return;
    Layout::RustFFI::layout_arena_set_geometry_host(arena, callbacks);
}

Layout::RustFFI::FfiRenderingPreparationOutcome rust_prepare_for_rendering(DOM::Document& document, bool visual_context_update_pending)
{
    publish_visual_context_tree_inputs(document);
    return Layout::RustFFI::layout_arena_prepare_for_rendering(
        layout_arena_handle(document), visual_context_update_pending);
}

// Whether a `color-scheme` declaration names a scheme an SVG used as an image can answer
// `prefers-color-scheme` with.
static bool declares_light_or_dark_color_scheme(ReadonlySpan<Utf16FlyString> schemes)
{
    return schemes.contains_slow("light"_utf16) || schemes.contains_slow("dark"_utf16);
}

CSS::ColorResolutionContext gradient_stop_color_resolution_context(BoxSlot const& box)
{
    void const* current_color_style_value_data = nullptr;
    if (auto dom_node = box.dom_node()) {
        if (auto* element = as_if<DOM::Element>(*dom_node)) {
            if (auto const* values = element->style_group<CSS::ComputedValues::InheritedTextValues>())
                current_color_style_value_data = values->color_style_value.pointer;
        }
    }
    auto const* ui_values = box.style_group<CSS::ComputedValues::InheritedUIValues>();
    auto const* text_values = box.style_group<CSS::ComputedValues::InheritedTextValues>();
    return {
        .color_scheme = ui_values ? ui_values->color_scheme_value() : CSS::PreferredColorScheme {},
        .current_color = text_values ? text_values->color_value() : Color {},
        .current_color_style_value_data = current_color_style_value_data,
        .calculation_resolution_context = {},
    };
}

CSS::ColorResolutionContext gradient_stop_color_resolution_context(DOM::Element const& element)
{
    auto const* text_values = element.style_group<CSS::ComputedValues::InheritedTextValues>();
    auto const* ui_values = element.style_group<CSS::ComputedValues::InheritedUIValues>();
    return {
        .color_scheme = ui_values ? ui_values->color_scheme_value() : CSS::PreferredColorScheme {},
        .current_color = text_values ? text_values->color_value() : Color {},
        .current_color_style_value_data = text_values ? text_values->color_style_value.pointer : nullptr,
        .calculation_resolution_context = {},
    };
}

void rust_update_visual_viewport_transform(DOM::Document& document)
{
    publish_visual_context_tree_inputs(document);
    Layout::RustFFI::layout_arena_update_visual_viewport_transform(layout_arena_handle(document));
}

bool rust_refresh_scroll_state(DOM::Document& document, Compositing::ScrollStateSnapshot& snapshot, ForceScrollStateRefresh force)
{
    publish_visual_context_tree_inputs(document);
    return Layout::RustFFI::layout_arena_refresh_scroll_state(
        layout_arena_handle(document), force == ForceScrollStateRefresh::Yes,
        &snapshot, [](void* sink, Gfx::FloatPoint const* offsets, size_t count) {
            static_cast<Compositing::ScrollStateSnapshot*>(sink)->assign_device_offsets({ offsets, count });
        });
}

void rust_invalidate_scroll_state(DOM::Document& document)
{
    Layout::RustFFI::layout_arena_invalidate_scroll_state(layout_arena_handle(document));
}

// What the dumps and traces print for the box `slot` names.
static void push_box_description(DOM::Document const& document, Compositing::RustFFI::NodeSlotId slot, void* description_sink)
{
    auto description = BoxSlot::of(document, slot).debug_description();
    auto bytes = description.bytes();
    Layout::RustFFI::layout_arena_paint_push_bytes(description_sink, bytes.data(), bytes.size());
}

Utf16String serialize_painting_dump(DOM::Document const& document, Compositing::AccumulatedVisualContextTree const& visual_context_tree, Compositing::DisplayList const& display_list, Compositing::DisplayListResourceStorage const& resource_storage)
{
    struct DumpContext {
        DOM::Document const& document;
        Compositing::DisplayListResourceStorage const& resource_storage;
        Utf16String dump;
    } context { document, resource_storage, {} };

    Layout::RustFFI::FfiPaintingDumpCallbacks callbacks {
        .context = &context,
        .debug_description = [](void* context_pointer, Compositing::RustFFI::NodeSlotId slot, void* description_sink) { push_box_description(static_cast<DumpContext*>(context_pointer)->document, slot, description_sink); },
        .command_bytes = [](void*, void const* display_list_pointer, size_t* byte_count) -> u8 const* {
            auto bytes = static_cast<Compositing::DisplayList const*>(display_list_pointer)->command_bytes();
            *byte_count = bytes.size();
            return bytes.data();
        },
        .nested_display_list = [](void* context_pointer, u64 display_list_id) -> void const* {
            auto& context = *static_cast<DumpContext*>(context_pointer);
            return &context.resource_storage.display_list(Compositing::DisplayListResourceId { display_list_id });
        },
        .append_text = [](void* context_pointer, u8 const* bytes, size_t byte_count) { static_cast<DumpContext*>(context_pointer)->dump = Utf16String::from_utf8_without_validation(StringView { bytes, byte_count }); },
    };
    auto command_runs = display_list.command_runs();
    Layout::RustFFI::painting_dump(layout_arena_handle(document), viewport_row_slot(document), visual_context_tree.rust_handle(), command_runs.data(), command_runs.size(), &display_list, callbacks);
    return move(context.dump);
}

static void append_bytes_to_string_builder(void* context, u8 const* bytes, size_t byte_count)
{
    static_cast<StringBuilder*>(context)->append(StringView { bytes, byte_count });
}

void dump_stacking_context_tree(StringBuilder& builder, DOM::Document const& document)
{
    struct DumpContext {
        DOM::Document const& document;
        StringBuilder& builder;
    } context { document, builder };
    Layout::RustFFI::FfiStackingContextDumpCallbacks callbacks {
        .context = &context,
        .debug_description = [](void* context_pointer, Compositing::RustFFI::NodeSlotId slot, void* description_sink) { push_box_description(static_cast<DumpContext*>(context_pointer)->document, slot, description_sink); },
        .append_text = [](void* context_pointer, u8 const* bytes, size_t byte_count) { static_cast<DumpContext*>(context_pointer)->builder.append(StringView { bytes, byte_count }); },
    };
    Layout::RustFFI::layout_arena_dump_stacking_context_tree(
        layout_arena_handle(document), viewport_row_slot(document), callbacks);
}

static void push_bytes_to_dump_sink(void* sink, ReadonlyBytes bytes)
{
    Layout::RustFFI::layout_arena_paint_push_bytes(sink, bytes.data(), bytes.size());
}

// What a layout tree dump of a document writes through: the document, whose rows the dump names by slot, and where the
// dump goes.
struct LayoutTreeDumpContext {
    DOM::Document const& document;
    void* output_context;
    void (*append_text)(void*, u8 const*, size_t);
};

static void dump_layout_tree(BoxSlot const& root, size_t initial_indent, bool interactive, void* output_context, void (*append_text)(void*, u8 const*, size_t))
{
    LayoutTreeDumpContext context { root.document(), output_context, append_text };
    Layout::RustFFI::FfiLayoutTreeDumpCallbacks callbacks {
        .context = &context,
        .describe_dom_node = [](void* context_pointer, Compositing::RustFFI::NodeSlotId slot, void* tag_name_sink, void* identifier_sink) {
            // A row whose node has been removed no longer names one, and the dump says so.
            auto dom_node = BoxSlot::of(static_cast<LayoutTreeDumpContext*>(context_pointer)->document, slot).dom_node();
            if (!dom_node) {
                push_bytes_to_dump_sink(tag_name_sink, "(detached)"sv.bytes());
                return;
            }
            auto const* element = as_if<DOM::Element>(*dom_node);
            StringBuilder tag_name_builder;
            tag_name_builder.append(element ? element->local_name() : dom_node->node_name());
            push_bytes_to_dump_sink(tag_name_sink, tag_name_builder.string_view().bytes());
            if (!element)
                return;
            StringBuilder identifier_builder;
            if (element->id().has_value() && !element->id()->is_empty()) {
                identifier_builder.append('#');
                identifier_builder.append(*element->id());
            }
            for (auto const& class_name : element->class_names()) {
                identifier_builder.append('.');
                identifier_builder.append(class_name);
            }
            push_bytes_to_dump_sink(identifier_sink, identifier_builder.string_view().bytes()); },
        .navigable_container_content_document = [](void* context_pointer, Compositing::RustFFI::NodeSlotId slot, void* url_sink) -> Layout::RustFFI::FfiNestedLayoutRoot {
            auto dom_node = BoxSlot::of(static_cast<LayoutTreeDumpContext*>(context_pointer)->document, slot).dom_node();
            auto const* container = as_if<HTML::NavigableContainer>(dom_node.ptr());
            auto const* content_document = container ? container->content_document_without_origin_check() : nullptr;
            if (!content_document)
                return { .has_document = false, .document = nullptr };
            auto serialized_url = content_document->url().serialize();
            push_bytes_to_dump_sink(url_sink, serialized_url.bytes());
            // The dump brings every hosted document's layout up to date first.
            ASSERT(content_document->layout_is_up_to_date());
            if (!BoxSlot::viewport_of(*content_document))
                return { .has_document = true, .document = nullptr };
            return { .has_document = true, .document = const_cast<DOM::Document*>(content_document) }; },
        .svg_as_image_document = [](void* context_pointer, Compositing::RustFFI::NodeSlotId slot) -> void* {
            auto dom_node = BoxSlot::of(static_cast<LayoutTreeDumpContext*>(context_pointer)->document, slot).dom_node();
            auto const* image_element = as_if<HTML::HTMLImageElement>(dom_node.ptr());
            if (!image_element)
                return nullptr;
            auto const* svg_image_data = as_if<SVG::SVGDecodedImageData>(image_element->current_request().image_data().ptr());
            if (!svg_image_data)
                return nullptr;
            auto& svg_document = svg_image_data->svg_document();
            if (!BoxSlot::viewport_of(svg_document))
                return nullptr;
            return &const_cast<DOM::Document&>(svg_document); },
        .dump_nested_layout_tree = [](void*, void* document, size_t indent, bool interactive, void* output_sink) { dump_layout_tree(BoxSlot::viewport_of(*static_cast<DOM::Document const*>(document)), indent, interactive, output_sink, Layout::RustFFI::layout_arena_paint_push_bytes); },
        .append_text = [](void* context_pointer, u8 const* bytes, size_t byte_count) {
            auto& context = *static_cast<LayoutTreeDumpContext*>(context_pointer);
            context.append_text(context.output_context, bytes, byte_count); },
    };
    Layout::RustFFI::layout_arena_dump_layout_tree(root.arena(), root.slot(), initial_indent, interactive, callbacks);
}

void dump_layout_tree(StringBuilder& builder, BoxSlot const& root, bool interactive)
{
    if (!root)
        return;
    dump_layout_tree(root, 0, interactive, &builder, append_bytes_to_string_builder);
}

namespace {

struct RecordingPublishContext {
    Compositing::DisplayListResourceStorage& resource_storage;
    GC::Ref<DOM::Document const> document;
};

// What a recording's publication adds its resources to. It reaches no document, so the render side can publish.
struct RecordingPublishStorage {
    Compositing::DisplayListResourceStorage& resource_storage;
};

static Layout::RustFFI::FfiRecordingPublishCallbacks recording_publish_callbacks(RecordingPublishStorage& context)
{
    return {
        .context = &context,
        .add_font = [](void* context_pointer, void const* font) {
            auto& context = *static_cast<RecordingPublishStorage*>(context_pointer);
            context.resource_storage.add_font(*static_cast<Gfx::Font const*>(font)); },
        .add_image_frame = [](void* context_pointer, void const* frame) {
            auto& context = *static_cast<RecordingPublishStorage*>(context_pointer);
            context.resource_storage.add_image_frame(*static_cast<Gfx::DecodedImageFrame const*>(frame)); },
        .add_video_sink = [](void* context_pointer, u64 resource_id, u64 sink_handle) {
            auto& context = *static_cast<RecordingPublishStorage*>(context_pointer);
            context.resource_storage.add_video_sink(Compositing::VideoSinkResourceId { resource_id }, Media::VideoSinkHandle { sink_handle }); },
    };
}

static Layout::RustFFI::FfiVectorImageCallbacks vector_image_callbacks(RecordingPublishContext& context)
{
    return {
        .context = &context,
        .resolve_vector_image_display_list = [](void* context_pointer, Layout::RustFFI::FfiVectorImageRenderRequest const* request) -> u64 {
            auto& context = *static_cast<RecordingPublishContext*>(context_pointer);
            auto const& document = *context.document;
            auto empty_display_list = [&] {
                return context.resource_storage.add_display_list(Compositing::DisplayList::create(document.paint_state().visual_context_tree(document)), document.paint_state().visual_context_tree(document)).value();
            };
            // The recording published the image and the scheme it renders with, so finding it is a
            // lookup rather than a walk back to the element that references it.
            auto const* svg_image_data = SVG::SVGDecodedImageData::with_vector_image_identity(request->image_identity);
            if (!svg_image_data)
                return empty_display_list();
            auto display_list = svg_image_data->record_display_list_at_scale({ request->css_width, request->css_height }, request->raster_scale, static_cast<CSS::PreferredColorScheme>(request->color_scheme), context.resource_storage);
            if (!display_list.has_value())
                return empty_display_list();
            return context.resource_storage.add_display_list(move(*display_list)).value();
        },
    };
}

// The platform default font at an overlay label's CSS size and at that size in device pixels, kept alive for the
// recording call.
struct OverlayLabelFonts {
    RefPtr<Gfx::Font> css_font;
    RefPtr<Gfx::Font> device_font;

    Layout::RustFFI::FfiOverlayLabelFonts ffi() const { return { .css_font = css_font.ptr(), .device_font = device_font.ptr() }; }
};

static OverlayLabelFonts overlay_label_fonts(float css_size, double device_pixels_per_css_pixel)
{
    OverlayLabelFonts fonts {
        .css_font = Platform::FontPlugin::the().default_font(css_size),
        .device_font = Platform::FontPlugin::the().default_font(css_size * static_cast<float>(device_pixels_per_css_pixel)),
    };
    VERIFY(fonts.css_font && fonts.device_font);
    return fonts;
}

}

void take_recording_trace_if_pending(DOM::Document& document)
{
    struct TraceContext {
        DOM::Document const& document;
        StringBuilder trace;
    } context { document, {} };
    bool has_pending_trace = Layout::RustFFI::layout_arena_take_recording_trace(
        layout_arena_handle(document), &context,
        [](void* context_pointer, Compositing::RustFFI::NodeSlotId slot, void* description_sink) { push_box_description(static_cast<TraceContext*>(context_pointer)->document, slot, description_sink); },
        [](void* context_pointer, u8 const* bytes, size_t byte_count) { static_cast<TraceContext*>(context_pointer)->trace.append(StringView { bytes, byte_count }); });
    if (has_pending_trace)
        document.paint_state().append_recording_trace(MUST(context.trace.to_string()));
}

bool last_recording_missed_vector_images(DOM::Document const& document)
{
    return Layout::RustFFI::layout_arena_last_recording_missed_vector_images(layout_arena_handle(document));
}

namespace {

// What a recording reads of the host, with the storage the pointers in its inputs point into.
struct HostRecordingInputs {
    AK_MAKE_NONCOPYABLE(HostRecordingInputs);
    AK_MAKE_NONMOVABLE(HostRecordingInputs);

public:
    HostRecordingInputs() = default;

    Layout::RustFFI::FfiRecordingInputs inputs {};
    Vector<Layout::RustFFI::FfiGridOverlayInput> grid_overlays;
    OverlayLabelFonts grid_label_fonts;
    Vector<Layout::RustFFI::FfiFlexOverlayInput> flex_overlays;
    ByteString inspector_highlight_label_text;
    OverlayLabelFonts inspector_label_fonts;
    Vector<u8> focused_area_path_bytes;
    DevicePixelRect device_viewport_rect;
    BlockingWheelEventRegionState wheel_event_region_state;
};

}

// Whether the recording inputs are read ahead of the layout that the recording follows, for a flight.
enum class ReadAheadOfLayout : u8 {
    No,
    Yes,
};

static void read_host_recording_inputs(HostRecordingInputs& host, DOM::Document& document, PaintCommandCacheMode cache_mode, HTML::PaintConfig const& config, InspectorOverlayInputs const& overlay_inputs, ReadAheadOfLayout read_ahead_of_layout = ReadAheadOfLayout::No)
{
    auto device_pixels_per_css_pixel = document.page().client().device_pixels_per_css_pixel();
    host.device_viewport_rect = document.page().css_to_device_rect(document.viewport_rect());
    auto const& device_viewport_rect = host.device_viewport_rect;
    host.wheel_event_region_state = document.paint_state().collect_root_blocking_wheel_event_regions(document);
    auto const& wheel_event_region_state = host.wheel_event_region_state;
    auto& inputs = host.inputs;
    if (overlay_inputs.highlighted_box) {
        inputs.has_inspector_highlight = true;
        inputs.inspector_highlight_paintable = overlay_inputs.highlighted_box.slot();
    }
    inputs.tooltip_color = overlay_inputs.tooltip_color;
    inputs.tooltip_text_color = overlay_inputs.tooltip_text_color;
    inputs.tooltip_border_color = overlay_inputs.tooltip_border_color;
    auto& ffi_grid_overlays = host.grid_overlays;
    auto& grid_label_fonts = host.grid_label_fonts;
    if (!overlay_inputs.grid_highlights.is_empty()) {
        grid_label_fonts = overlay_label_fonts(10.0f, device_pixels_per_css_pixel);
        inputs.grid_label_fonts = grid_label_fonts.ffi();
    }
    for (auto const& highlight : overlay_inputs.grid_highlights) {
        ffi_grid_overlays.append({
            .paintable = highlight.box.slot(),
            .color = highlight.options.color,
            .label_foreground_color = highlight.options.color.with_alpha(235).suggested_foreground_color(),
            .label_css_pixel_size = grid_label_fonts.css_font->pixel_size(),
            .show_area_names = highlight.options.show_area_names,
            .show_line_numbers = highlight.options.show_line_numbers,
            .show_track_sizes = highlight.options.show_track_sizes,
            .show_infinite_lines = highlight.options.show_infinite_lines,
        });
    }
    inputs.grid_overlays = ffi_grid_overlays.data();
    inputs.grid_overlay_count = ffi_grid_overlays.size();
    auto& ffi_flex_overlays = host.flex_overlays;
    for (auto const& highlight : overlay_inputs.flex_highlights) {
        ffi_flex_overlays.append({
            .paintable = highlight.box.slot(),
            .color = highlight.options.color,
        });
    }
    inputs.flex_overlays = ffi_flex_overlays.data();
    inputs.flex_overlay_count = ffi_flex_overlays.size();
    inputs.caret_debug_rect = overlay_inputs.caret_debug_rect;
    auto& inspector_highlight_label_text = host.inspector_highlight_label_text;
    auto& inspector_label_fonts = host.inspector_label_fonts;
    if (auto const& box = overlay_inputs.highlighted_box) {
        auto border_rect = absolute_border_box_rect(box);
        inspector_highlight_label_text = ByteString::formatted("{} {}x{} @ {},{}", box.debug_description(), border_rect.width(), border_rect.height(), border_rect.x(), border_rect.y());
        inspector_label_fonts = overlay_label_fonts(12.0f, device_pixels_per_css_pixel);
        inputs.inspector_highlight_label = {
            .fonts = inspector_label_fonts.ffi(),
            .text = inspector_highlight_label_text.bytes().data(),
            .text_byte_count = inspector_highlight_label_text.length(),
        };
    }
    inputs.device_viewport_rect = device_viewport_rect.to_type<int>();
    if (auto navigable = document.navigable())
        inputs.css_viewport_rect = navigable->viewport_rect();
    inputs.should_show_line_box_borders = config.should_show_line_box_borders;
    inputs.force_dark_enabled = config.force_dark_enabled;
    inputs.force_dark_foreground_threshold = config.force_dark_foreground_threshold;
    inputs.force_dark_background_threshold = config.force_dark_background_threshold;
    inputs.should_paint_overlay = config.paint_overlay;
    inputs.is_recording_async_scrolling_metadata = true;
    inputs.document_id = document.unique_id().value();
    inputs.has_blocking_wheel_event_region_covering_viewport = wheel_event_region_state.has_blocking_wheel_event_region_covering_viewport;
    inputs.wheel_event_listener_state_generation = document.page().wheel_event_listener_state_generation();
    inputs.chrome_metrics = document.page().chrome_metrics();
    inputs.paint_viewport_scrollbars = should_paint_viewport_scrollbars();
    inputs.async_scrolling_enabled = document.page().async_scrolling_enabled();
    if (auto navigable = document.navigable()) {
        if (auto handler = navigable->event_handler().middle_button_scroll_handler(); handler.has_value()) {
            inputs.middle_button_scroll_active = true;
            inputs.middle_button_scroll_origin = handler->origin();
        }
    }
    inputs.publishes_recording = cache_mode == PaintCommandCacheMode::ReadWrite;
    {
        auto navigable = document.navigable();
        inputs.window_is_focused = navigable && navigable->is_focused();
        inputs.outline_auto_color = CSS::SystemColor::accent_color(CSS::PreferredColorScheme::Auto);
        auto palette = document.page().palette();
        inputs.palette_is_dark = palette.is_dark();
        inputs.selection_background_from_palette = CSS::SystemColor::transform_selection_background_color(inputs.window_is_focused ? palette.selection() : palette.inactive_selection());
        inputs.selection_background_light = CSS::SystemColor::transform_selection_background_color(inputs.window_is_focused ? CSS::SystemColor::highlight(CSS::PreferredColorScheme::Light) : CSS::SystemColor::inactive_highlight(CSS::PreferredColorScheme::Light));
        inputs.selection_background_dark = CSS::SystemColor::transform_selection_background_color(inputs.window_is_focused ? CSS::SystemColor::highlight(CSS::PreferredColorScheme::Dark) : CSS::SystemColor::inactive_highlight(CSS::PreferredColorScheme::Dark));
        inputs.document_has_supported_color_schemes = document.supported_color_schemes().has_value();
        auto supported_color_schemes = document.supported_color_schemes();
        inputs.document_declares_light_or_dark_color_scheme = supported_color_schemes.has_value() && declares_light_or_dark_color_scheme(*supported_color_schemes);
        inputs.image_color_scheme_fallback = to_underlying(document.svg_image_color_scheme().value_or(document.page().preferred_color_scheme()));
    }
    inputs.caret = resolve_document_caret_paint(document);
    inputs.focused_text_control = resolve_focused_text_control_selection(document);
    auto& focused_area_path_bytes = host.focused_area_path_bytes;
    inputs.focused_area_outline = resolve_focused_area_outline(document, focused_area_path_bytes);
    {
        // NB: The root's style is final ahead of the layout, and a flight's recording is dropped if the root's box shows
        //     another canvas once it has laid out (see LocalNavigable::finish_flight_paint()).
        auto color_scheme = read_ahead_of_layout == ReadAheadOfLayout::Yes ? document.canvas_color_scheme_as_last_laid_out() : document.canvas_color_scheme();
        bool opaque_canvas = false;
        // NB: The container's document laid out ahead of this one's, which a flight reads ahead of its layout.
        auto container_ui_values = [&]() -> CSS::ComputedValues::InheritedUIValues const* {
            auto container_element = document.navigable()->container();
            if (!container_element)
                return nullptr;
            return BoxSlot::bound_to(*container_element).style_group<CSS::ComputedValues::InheritedUIValues>();
        };
        if (auto const* container_values = container_ui_values()) {
            auto container_scheme = container_values->color_scheme_value();
            if (container_scheme == CSS::PreferredColorScheme::Auto)
                container_scheme = CSS::PreferredColorScheme::Light;
            opaque_canvas = container_scheme != color_scheme;
        }
        inputs.canvas_fill_rect = config.canvas_fill_rect;
        inputs.canvas_color = CSS::SystemColor::canvas(color_scheme);
        inputs.opaque_canvas = opaque_canvas;
        Gfx::IntRect bitmap_rect { {}, device_viewport_rect.size().to_type<int>() };
        inputs.bitmap_rect = bitmap_rect;
        inputs.background_color = document.background_color();
    }
}

StringView recording_origin_name(RecordingOrigin origin)
{
    switch (origin) {
    case RecordingOrigin::RenderingUpdate:
        return "renderingUpdate"sv;
    case RecordingOrigin::SynchronousRenderingUpdate:
        return "synchronousRenderingUpdate"sv;
    case RecordingOrigin::WaitsForRecordings:
        return "waitsForRecordings"sv;
    case RecordingOrigin::NoFrameScheduler:
        return "noFrameScheduler"sv;
    case RecordingOrigin::HitTest:
        return "hitTest"sv;
    case RecordingOrigin::Screenshot:
        return "screenshot"sv;
    case RecordingOrigin::PaintIfNeeded:
        return "paintIfNeeded"sv;
    case RecordingOrigin::DocumentRecord:
        return "documentRecord"sv;
    case RecordingOrigin::Count:
        break;
    }
    VERIFY_NOT_REACHED();
}

StringView flight_paint_decline_name(FlightPaintDecline decline)
{
    switch (decline) {
    case FlightPaintDecline::Inactive:
        return "inactive"sv;
    case FlightPaintDecline::NotPaintedThatWay:
        return "notPaintedThatWay"sv;
    case FlightPaintDecline::PresenterLent:
        return "presenterLent"sv;
    case FlightPaintDecline::NothingToPaintYet:
        return "nothingToPaintYet"sv;
    case FlightPaintDecline::InspectorOverlay:
        return "inspectorOverlay"sv;
    case FlightPaintDecline::Caret:
        return "caret"sv;
    case FlightPaintDecline::FocusedTextControl:
        return "focusedTextControl"sv;
    case FlightPaintDecline::MiddleButtonScroll:
        return "middleButtonScroll"sv;
    case FlightPaintDecline::ResizeObserver:
        return "resizeObserver"sv;
    case FlightPaintDecline::Animations:
        return "animations"sv;
    case FlightPaintDecline::ForcedCompositorLayer:
        return "forcedCompositorLayer"sv;
    case FlightPaintDecline::ViewTransitionOrScrollState:
        return "viewTransitionOrScrollState"sv;
    case FlightPaintDecline::HostedNavigable:
        return "hostedNavigable"sv;
    case FlightPaintDecline::Count:
        break;
    }
    VERIFY_NOT_REACHED();
}

RecordingOrigin& current_recording_origin()
{
    static thread_local RecordingOrigin origin { RecordingOrigin::PaintIfNeeded };
    return origin;
}

Optional<PendingDisplayListRecording> begin_rust_display_list_recording(DOM::Document& document, Compositing::DisplayList const& placeholder_display_list, Compositing::DisplayListResourceStorage& resource_storage, PaintCommandCacheMode cache_mode, HTML::PaintConfig const& config, InspectorOverlayInputs const& overlay_inputs, RecordingRun run)
{
    auto* arena = layout_arena_handle(document);
    RecordingPublishContext publish_context { resource_storage, document };
    HostRecordingInputs host;
    read_host_recording_inputs(host, document, cache_mode, config, overlay_inputs);
    auto& inputs = host.inputs;
    auto const& device_viewport_rect = host.device_viewport_rect;
    auto const& wheel_event_region_state = host.wheel_event_region_state;
    reconcile_navigable_container_paint_facts(document);
    // Rendering an SVG-as-image lays out and records its document, so it happens here on the main
    // thread before the recording starts; the recording only looks the renders up.
    Layout::RustFFI::layout_arena_resolve_painted_vector_images(arena, &inputs, vector_image_callbacks(publish_context));
    // NB: Asking for the visual context tree can drain the document's invalidation journal, which joins a frame in
    //     flight, so it is taken before the recording is submitted.
    auto visual_context_tree = document.paint_state().visual_context_tree(document);
    auto rust_timer = Core::ElapsedTimer::start_new(Core::TimerType::Precise);
    auto ffi_run = run == RecordingRun::InSubmittedFrame ? Layout::RustFFI::FfiRecordingRun::InSubmittedFrame : Layout::RustFFI::FfiRecordingRun::Now;
    void const* ticket = nullptr;
    if (!Layout::RustFFI::layout_arena_record_display_list(arena, viewport_row_slot(document), inputs, ffi_run, &ticket))
        return {};
    // NB: The render side may still record while the main thread waits, if the frame scheduler does not submit recordings.
    auto submitted_ticket = SubmittedRecordingTicket::adopt(ticket);
    auto const submitted = submitted_ticket.is_in_flight();
    if (!submitted)
        HTML::main_thread_event_loop().did_wait_for_recording(current_recording_origin(), rust_timer.elapsed_time().to_nanoseconds());
    return PendingDisplayListRecording {
        .document = document,
        .arena = arena,
        .resource_storage = resource_storage,
        .visual_context_tree = move(visual_context_tree),
        .cache_mode = cache_mode,
        .run = submitted ? RecordingRun::InSubmittedFrame : RecordingRun::Now,
        .submitted_ticket = move(submitted_ticket),
        .surface_clear_color = placeholder_display_list.surface_clear_color(),
        .device_viewport_rect = device_viewport_rect,
        .wheel_event_region_state = wheel_event_region_state,
        .timer = rust_timer,
    };
}

FlightRecordingSeal seal_rust_display_list_recording_for_flight(DOM::Document& document, Compositing::DisplayListResourceStorage& resource_storage, HTML::PaintConfig const& config, InspectorOverlayInputs const& overlay_inputs, FlightPresent present, void* present_context)
{
    auto* arena = layout_arena_handle(document);
    RecordingPublishContext publish_context { resource_storage, document };
    // The flight updates the visual contexts for these inputs, as a recording prepared here would.
    publish_visual_context_tree_inputs(document);
    HostRecordingInputs host;
    read_host_recording_inputs(host, document, PaintCommandCacheMode::ReadWrite, config, overlay_inputs, ReadAheadOfLayout::Yes);
    reconcile_navigable_container_paint_facts(document, ReconcileAheadOfLayout::Yes);
    Layout::RustFFI::layout_arena_resolve_painted_vector_images(arena, &host.inputs, vector_image_callbacks(publish_context));
    Layout::RustFFI::layout_arena_seal_flight_paint(arena, host.inputs, present, present_context);
    return {
        .device_viewport_rect = host.device_viewport_rect,
        .wheel_event_region_state = host.wheel_event_region_state,
        .canvas_color = host.inputs.canvas_color,
        .background_color = host.inputs.background_color,
    };
}

bool discard_retired_rust_display_list_recording(PendingDisplayListRecording& recording)
{
    auto& document = *recording.document;
    if (!Layout::RustFFI::layout_arena_discard_retired_recording(layout_arena_handle(document)))
        return false;
    // What was marked beside a recording in the frame in flight is what the next drain writes.
    if (recording.run == RecordingRun::InSubmittedFrame)
        document.release_held_invalidation_marks();
    return true;
}

void add_published_svg_filter_image_frames(DOM::Document const& document, Compositing::DisplayListResourceStorage& resource_storage)
{
    RecordingPublishStorage publish_storage { resource_storage };
    Layout::RustFFI::layout_arena_publish_svg_filter_image_frames(layout_arena_handle(document), recording_publish_callbacks(publish_storage));
}

DocumentPresentationSource::DocumentPresentationSource(DOM::Document& document, u64 adopted_async_scroll_sequence)
    : m_document(document)
    , m_adopted_async_scroll_sequence(adopted_async_scroll_sequence)
{
}

Compositing::AccumulatedVisualContextTree DocumentPresentationSource::published_display_list_visual_context_tree()
{
    return m_document->visual_context_tree();
}

Optional<Compositor::AsyncScrollingStamp> DocumentPresentationSource::async_scrolling_stamp()
{
    auto navigable = m_document->navigable();
    if (!navigable)
        return {};
    return Compositor::AsyncScrollingStamp {
        .wheel_event_listener_state_generation = navigable->page().wheel_event_listener_state_generation(),
        .device_pixels_per_css_pixel = navigable->page().client().device_pixels_per_css_pixel(),
    };
}

void DocumentPresentationSource::did_publish_recording()
{
    take_recording_trace_if_pending(m_document);
}

Compositing::AccumulatedVisualContextTree DocumentPresentationSource::visual_context_tree(Compositing::DisplayListResourceStorage& resource_storage)
{
    // Reading the tree synchronizes SVG paint resources first, which can publish a filter image the recording (and
    // the frame before it) never saw. The image the tree references goes into the storage before the tree is sent.
    auto tree = m_document->paint_state().visual_context_tree(m_document);
    add_published_svg_filter_image_frames(m_document, resource_storage);
    return tree;
}

bool DocumentPresentationSource::visual_context_tree_needs_compositor_update()
{
    return m_document->paint_state().visual_context_tree_needs_compositor_update();
}

void DocumentPresentationSource::did_update_visual_context_tree_in_compositor()
{
    m_document->paint_state().did_update_visual_context_tree_in_compositor();
}

Compositing::ScrollStateSnapshot DocumentPresentationSource::scroll_state_snapshot()
{
    Compositing::ScrollStateSnapshot scroll_state_snapshot { m_document->paint_state().scroll_state_snapshot() };
    scroll_state_snapshot.set_adopted_async_scroll_sequence(m_adopted_async_scroll_sequence);
    return scroll_state_snapshot;
}

enum class PublicationSite : u8 {
    MainThread,
    FrameInFlight,
};

static Optional<Compositor::PublishedDisplayList> publish_rust_display_list_recording(PublicationSite site, PendingDisplayListRecording& recording, void const* recording_ticket, Compositing::DisplayList* paint_command_cache_source, Compositing::DisplayListResourceSet const& paint_command_cache_source_resources, Compositor::PresentationSource& source)
{
    auto* arena = recording.arena;
    RecordingPublishStorage publish_storage { recording.resource_storage };
    auto const& device_viewport_rect = recording.device_viewport_rect;
    auto& wheel_event_region_state = recording.wheel_event_region_state;
    auto const& rust_timer = recording.timer;
    // A recording submitted with a ticket is published from the ticket, and the document takes it in later; any other
    // is published in and read back from its arena.
    Layout::RustFFI::FfiPresentedRecording presented {};
    if (recording_ticket) {
        ASSERT(site == PublicationSite::FrameInFlight);
        // The frame presents an answered recording once; a ticket with nothing to present shows nothing.
        bool const did_present = Layout::RustFFI::layout_recording_ticket_publish_in_frame(recording_ticket, recording_publish_callbacks(publish_storage), &presented);
        ASSERT(did_present);
        if (!did_present)
            return {};
    } else {
        if (site == PublicationSite::FrameInFlight)
            Layout::RustFFI::layout_arena_publish_recording_in_frame(arena, recording_publish_callbacks(publish_storage));
        else
            Layout::RustFFI::layout_arena_publish_recording(arena, recording_publish_callbacks(publish_storage));
        presented.is_identical_to_published_frame = Layout::RustFFI::layout_arena_last_recording_is_identical_to_published_frame(arena);
        presented.has_blocking_wheel_event_listeners = Layout::RustFFI::layout_arena_last_recording_has_blocking_wheel_event_listeners(arena);
    }
    source.did_publish_recording();
    if (presented.has_blocking_wheel_event_listeners)
        wheel_event_region_state.has_blocking_wheel_event_listeners = true;
    auto stamp_async_scrolling_metadata_with_current_viewport_rect = [&](Compositing::DisplayList& display_list) {
        if (auto stamp = source.async_scrolling_stamp(); stamp.has_value()) {
            display_list.set_async_scrolling_metadata({
                .viewport_rect = device_viewport_rect.to_type<int>(),
                .wheel_event_listener_state_generation = stamp->wheel_event_listener_state_generation,
                .has_blocking_wheel_event_listeners = wheel_event_region_state.has_blocking_wheel_event_listeners,
                .has_blocking_wheel_event_region_covering_viewport = wheel_event_region_state.has_blocking_wheel_event_region_covering_viewport,
                .device_pixels_per_css_pixel = stamp->device_pixels_per_css_pixel,
            });
        }
    };

    if (presented.is_identical_to_published_frame) {
        if (paint_command_cache_source) {
            if (presented.display_list)
                Compositing::RustFFI::display_list_release_command_storage(presented.display_list);
            if (rust_painting_timing_enabled())
                dbgln("PAINT_RECORD rust={} µs identical to the previous recording", rust_timer.elapsed_time().to_microseconds());
            stamp_async_scrolling_metadata_with_current_viewport_rect(*paint_command_cache_source);
            return Compositor::PublishedDisplayList {
                .display_list = *paint_command_cache_source,
                .command_resources = paint_command_cache_source_resources,
                .is_paint_command_cache_source = true,
                .becomes_paint_command_cache_source = false,
            };
        }
    }

    auto display_list = Compositing::DisplayList::adopt_rust_command_storage(source.published_display_list_visual_context_tree(), recording_ticket ? presented.display_list : Layout::RustFFI::layout_arena_retain_recorded_display_list(arena));
    if (rust_painting_timing_enabled())
        dbgln("PAINT_RECORD rust={} µs commands={} bytes", rust_timer.elapsed_time().to_microseconds(), display_list->command_bytes().size());

    if (recording.surface_clear_color.has_value())
        display_list->set_surface_clear_color(*recording.surface_clear_color);
    stamp_async_scrolling_metadata_with_current_viewport_rect(*display_list);
    auto command_resources = recording.resource_storage.collect_referenced_resources(*display_list);
    return Compositor::PublishedDisplayList {
        .display_list = move(display_list),
        .command_resources = move(command_resources),
        .is_paint_command_cache_source = false,
        .becomes_paint_command_cache_source = recording.cache_mode == PaintCommandCacheMode::ReadWrite,
    };
}

Compositor::PublishedDisplayList publish_rust_display_list_recording(PendingDisplayListRecording& recording, Compositing::DisplayList* paint_command_cache_source, Compositing::DisplayListResourceSet const& paint_command_cache_source_resources, Compositor::PresentationSource& source)
{
    // Only a ticket leaves nothing to publish.
    return publish_rust_display_list_recording(PublicationSite::MainThread, recording, nullptr, paint_command_cache_source, paint_command_cache_source_resources, source).release_value();
}

Optional<Compositor::PublishedDisplayList> publish_rust_display_list_recording_in_frame(PendingDisplayListRecording& recording, void const* recording_ticket, Compositing::DisplayList* paint_command_cache_source, Compositing::DisplayListResourceSet const& paint_command_cache_source_resources, Compositor::PresentationSource& source)
{
    return publish_rust_display_list_recording(PublicationSite::FrameInFlight, recording, recording_ticket, paint_command_cache_source, paint_command_cache_source_resources, source);
}

Compositing::DisplayListResource record_image_paint_display_list(ImagePaint const& paint, ImagePaintRequest const& request, double device_pixels_per_css_pixel)
{
    Layout::RustFFI::FfiImagePaintRecordInputs inputs {};
    inputs.dest_rect = request.dest_rect;
    inputs.device_pixels_per_css_pixel = device_pixels_per_css_pixel;
    Optional<CSS::ComputedValuesFFI::FfiLengthResolutionContext> gradient_stop_length_resolution_context_storage;
    CSS::StyleValueFFI::FfiColorResolutionInput gradient_stop_color_resolution_input {};
    paint.value.visit(
        [&](ImagePaint::DecodedFrame const& decoded_frame) {
            inputs.kind = Layout::RustFFI::FfiImagePaintRecordKind::DecodedFrame;
            inputs.frame_id = request.resource_storage.add_image_frame(decoded_frame.frame).value();
            inputs.scaling_mode = CSS::to_gfx_scaling_mode(request.image_rendering, decoded_frame.natural_size, request.dest_rect.to_rounded<int>().size());
        },
        [&](ImagePaint::NestedDisplayList const& nested) {
            inputs.kind = Layout::RustFFI::FfiImagePaintRecordKind::NestedDisplayList;
            inputs.nested_display_list_id = request.resource_storage.add_display_list(nested.resource.display_list, nested.resource.visual_context_tree).value();
            inputs.nested_display_list_size = nested.list_size;
        },
        [&](ImagePaint::Gradient const& gradient) {
            inputs.kind = Layout::RustFFI::FfiImagePaintRecordKind::Gradient;
            inputs.gradient_style_value = gradient.style_value->rust_style_value_data();
            inputs.gradient_tile_size = request.dest_rect.size().to_type<CSSPixels>();
            gradient_stop_color_resolution_input = CSS::make_rust_color_resolution_input(request.gradient_stop_color_resolution_context, gradient_stop_length_resolution_context_storage);
            inputs.gradient_stop_color_resolution_input = &gradient_stop_color_resolution_input;
        });
    Optional<Compositing::DisplayListResource> recorded_display_list;
    Layout::RustFFI::ladybird_web_record_image_paint_display_list(&inputs, &recorded_display_list,
        [](void* context, void const* retained_commands, void const* retained_tree) {
            auto visual_context_tree = Compositing::AccumulatedVisualContextTree::adopt_rust_handle(retained_tree);
            auto display_list = Compositing::DisplayList::adopt_rust_command_storage(visual_context_tree, retained_commands);
            *static_cast<Optional<Compositing::DisplayListResource>*>(context) = Compositing::DisplayListResource { move(display_list), move(visual_context_tree) };
        });
    return recorded_display_list.release_value();
}

}
