/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Array.h>
#include <AK/Debug.h>
#include <AK/GenericShorthands.h>
#include <AK/Math.h>
#include <AK/NeverDestroyed.h>
#include <AK/NumericLimits.h>
#include <AK/Variant.h>
#include <LibGfx/Point.h>
#include <LibGfx/TextLayout.h>
#include <LibUnicode/CharacterTypes.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/Display.h>
#include <LibWeb/CSS/LengthBox.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleInputScope.h>
#include <LibWeb/CSS/StyleValues/AnchorStyleValue.h>
#include <LibWeb/CSS/StyleValues/CalculatedStyleValue.h>
#include <LibWeb/CSS/StyleValues/CursorStyleValue.h>
#include <LibWeb/CSS/ValueType.h>
#include <LibWeb/DOM/AbstractElement.h>
#include <LibWeb/DOM/CommitMessages.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/PseudoElement.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/DOM/Text.h>
#include <LibWeb/HTML/AttributeNames.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/HTML/FormAssociatedElement.h>
#include <LibWeb/HTML/HTMLElement.h>
#include <LibWeb/HTML/HTMLTableCellElement.h>
#include <LibWeb/HTML/HTMLTableColElement.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/NavigableContainer.h>
#include <LibWeb/Layout/ImageProvider.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxSlot.h>
#include <LibWeb/Painting/PaintFacts.h>
#include <LibWeb/Painting/PaintableTypes.h>
#include <LibWeb/Painting/PaintingRustBridge.h>
#include <LibWeb/Painting/ScrollSnap.h>
#include <LibWeb/Painting/StyleImageObservers.h>
#include <LibWeb/SVG/FragmentIdentifier.h>
#include <LibWeb/SVG/SVGCircleElement.h>
#include <LibWeb/SVG/SVGClipPathElement.h>
#include <LibWeb/SVG/SVGEllipseElement.h>
#include <LibWeb/SVG/SVGImageElement.h>
#include <LibWeb/SVG/SVGLineElement.h>
#include <LibWeb/SVG/SVGMaskElement.h>
#include <LibWeb/SVG/SVGPathElement.h>
#include <LibWeb/SVG/SVGPatternElement.h>
#include <LibWeb/SVG/SVGPolygonElement.h>
#include <LibWeb/SVG/SVGPolylineElement.h>
#include <LibWeb/SVG/SVGRectElement.h>
#include <LibWeb/SVG/SVGSVGElement.h>
#include <LibWeb/SVG/SVGSymbolElement.h>
#include <LibWeb/SVG/SVGTextElement.h>
#include <LibWeb/SVG/SVGTextPathElement.h>
#include <LibWeb/SVG/SVGTextPositioningElement.h>
#include <LibWeb/SVG/SVGUseElement.h>

namespace Web::Layout {

static_assert(to_underlying(CSS::StyleGroupIndex::Count) == RustFFI::STYLE_GROUP_COUNT);
static_assert(to_underlying(CSS::StyleGroupIndex::GridValues) == RustFFI::STYLE_GROUP_INDEX_GRID);
static_assert(to_underlying(CSS::StyleGroupIndex::AnchorValues) == RustFFI::STYLE_GROUP_INDEX_ANCHOR);
static_assert(to_underlying(CSS::StyleGroupIndex::InheritedTableValues) == RustFFI::STYLE_GROUP_INDEX_INHERITED_TABLE);
static_assert(to_underlying(CSS::StyleGroupIndex::InheritedTextValues) == RustFFI::STYLE_GROUP_INDEX_INHERITED_TEXT);
static_assert(to_underlying(CSS::StyleGroupIndex::InheritedBoxValues) == RustFFI::STYLE_GROUP_INDEX_INHERITED_BOX);
static_assert(to_underlying(CSS::StyleGroupIndex::FontValues) == RustFFI::STYLE_GROUP_INDEX_FONT);
static_assert(to_underlying(CSS::StyleGroupIndex::SVGResetValues) == RustFFI::STYLE_GROUP_INDEX_SVG_RESET);
static_assert(to_underlying(CSS::StyleGroupIndex::BorderValues) == RustFFI::STYLE_GROUP_INDEX_BORDER);
static_assert(to_underlying(CSS::StyleGroupIndex::AlignmentValues) == RustFFI::STYLE_GROUP_INDEX_ALIGNMENT);
static_assert(to_underlying(CSS::StyleGroupIndex::SizingValues) == RustFFI::STYLE_GROUP_INDEX_SIZING);
static_assert(to_underlying(CSS::StyleGroupIndex::SurroundValues) == RustFFI::STYLE_GROUP_INDEX_SURROUND);
static_assert(to_underlying(CSS::StyleGroupIndex::BoxValues) == RustFFI::STYLE_GROUP_INDEX_BOX);
static_assert(to_underlying(CSS::StyleGroupIndex::ContentValues) == RustFFI::STYLE_GROUP_INDEX_CONTENT);

static RustFFI::FfiSvgViewBox to_ffi_svg_view_box(SVG::ViewBox const& view_box)
{
    return {
        .min_x = view_box.min_x,
        .min_y = view_box.min_y,
        .width = view_box.width,
        .height = view_box.height,
    };
}

static RustFFI::FfiSvgLengthValue to_ffi_svg_length_value(Optional<SVG::SVGLengthValue> const& value)
{
    if (!value.has_value())
        return { .value = 0, .kind = RustFFI::SVG_LENGTH_KIND_NONE, .unit = 0 };
    switch (value->kind()) {
    case SVG::SVGLengthValue::Kind::Number:
        return { .value = value->value(), .kind = RustFFI::SVG_LENGTH_KIND_NUMBER, .unit = 0 };
    case SVG::SVGLengthValue::Kind::Length:
        return { .value = value->value(), .kind = RustFFI::SVG_LENGTH_KIND_LENGTH, .unit = static_cast<u8>(to_underlying(value->unit())) };
    case SVG::SVGLengthValue::Kind::Percentage:
        return { .value = value->value(), .kind = RustFFI::SVG_LENGTH_KIND_PERCENTAGE, .unit = 0 };
    }
    VERIFY_NOT_REACHED();
}

// The element an SVG reference names, as the style mirror's id index can answer for it: the URL's
// decoded fragment, interned as the atom the element's id is indexed under. Parsing a URL is
// document work rather than layout work, so a reference travels as an atom and the pass resolves
// it through the index instead of asking the document for the element.
// FIXME: A same-document fragment is all this carries, which is all SVG resolves today.
static CSS::StyleAtomID svg_reference_fragment_atom(DOM::Element& element, Optional<Utf16String> const& url_string)
{
    if (!url_string.has_value())
        return {};
    auto url = element.document().encoding_parse_url(*url_string);
    if (!url.has_value() || !url->fragment().has_value())
        return {};
    auto fragment = SVG::decode_fragment_identifier(*url->fragment());
    auto style_engine = element.document().style_computer().style_engine_queries();
    return style_engine.intern_atom(Utf16FlyString::from_utf16(fragment.utf16_view()));
}

// The same, for a reference a graphics element's style carries rather than its `href`.
// `SVGGraphicsElement::resolve_url_to_element(CSS::URL const&)` takes the text after the first `#`
// of the URL as written, with no base-URL resolution, so `url(other.svg#shape)` names `#shape` in
// this document. Reproduced here as written.
// FIXME: Complete and use the entire URL, not just the fragment.
static CSS::StyleAtomID svg_style_reference_fragment_atom(DOM::Element& element, Optional<CSS::URL> const& url)
{
    if (!url.has_value())
        return {};
    auto fragment_offset = Utf16View { url->url() }.find_code_unit_offset('#');
    if (!fragment_offset.has_value())
        return {};
    auto fragment = SVG::decode_fragment_identifier(url->url().substring_view(fragment_offset.value() + 1));
    auto style_engine = element.document().style_computer().style_engine_queries();
    return style_engine.intern_atom(Utf16FlyString::from_utf16(fragment.utf16_view()));
}

static Optional<CSS::URL> svg_paint_url(Optional<CSS::SVGPaint> const& paint)
{
    if (!paint.has_value() || !paint->is_url())
        return {};
    return paint->as_url();
}

// The four resources `SVGGraphicsElement::{mask,clip_path,fill_pattern,stroke_pattern}` name.
// Read from the record's group payloads rather than through a materialized view: this runs for
// every SVG graphics element whose style is installed, and pinning a record to look at four
// properties is most of the cost of looking at them.
static Array<CSS::StyleAtomID, 4> svg_style_reference_atoms(DOM::Element& element)
{
    if (!is<SVG::SVGGraphicsElement>(element))
        return {};
    auto const* payloads = static_cast<void const* const*>(element.style_record_payloads());
    if (!payloads)
        return {};
    auto const* mask_payload = static_cast<CSS::ComputedValues::MaskValues const*>(payloads[CSS::ComputedValues::MaskValues::style_group_index]);
    auto const* svg_payload = static_cast<CSS::ComputedValues::InheritedSVGValues const*>(payloads[CSS::ComputedValues::InheritedSVGValues::style_group_index]);
    if (!mask_payload || !svg_payload)
        return {};
    auto const& mask = mask_payload->mask_value();
    return {
        svg_style_reference_fragment_atom(element, mask.has_value() ? Optional<CSS::URL> { mask->url() } : OptionalNone {}),
        svg_style_reference_fragment_atom(element, mask_payload->clip_path_value()),
        svg_style_reference_fragment_atom(element, svg_paint_url(svg_payload->fill_value())),
        svg_style_reference_fragment_atom(element, svg_paint_url(svg_payload->stroke_value())),
    };
}

// The SVG presentation attributes an element parses, as the layout stage reads them.
static RustFFI::FfiSvgAttributeFacts build_svg_attribute_facts(DOM::Element& dom_node)
{
    auto const* svg_element = as_if<SVG::SVGElement>(dom_node);
    if (!svg_element)
        return {};
    auto const* fit_to_view_box = svg_element->fit_to_view_box();

    Optional<SVG::ViewBox> active_view_box;
    if (auto const* svg_graphics_element = as_if<SVG::SVGGraphicsElement>(dom_node))
        active_view_box = svg_graphics_element->active_view_box();
    else if (fit_to_view_box)
        active_view_box = fit_to_view_box->view_box();

    SVG::PreserveAspectRatio preserve_aspect_ratio {};
    if (fit_to_view_box)
        preserve_aspect_ratio = fit_to_view_box->preserve_aspect_ratio().value_or(SVG::PreserveAspectRatio {});
    else if (is<SVG::SVGMaskElement>(dom_node) || is<SVG::SVGClipPathElement>(dom_node))
        preserve_aspect_ratio = { SVG::PreserveAspectRatio::Align::None, {} };

    SVG::SVGUnits content_units {};
    SVG::SVGUnits pattern_units {};
    SVG::SVGUnits mask_units {};
    SVG::NumberPercentage mask_x = SVG::NumberPercentage::create_number(0);
    SVG::NumberPercentage mask_y = SVG::NumberPercentage::create_number(0);
    SVG::NumberPercentage mask_width = SVG::NumberPercentage::create_number(0);
    SVG::NumberPercentage mask_height = SVG::NumberPercentage::create_number(0);
    SVG::NumberPercentage pattern_width = SVG::NumberPercentage::create_number(0);
    SVG::NumberPercentage pattern_height = SVG::NumberPercentage::create_number(0);
    if (auto const* mask_element = as_if<SVG::SVGMaskElement>(dom_node)) {
        content_units = mask_element->mask_content_units();
        mask_units = mask_element->mask_units();
        mask_x = mask_element->mask_x();
        mask_y = mask_element->mask_y();
        mask_width = mask_element->mask_width();
        mask_height = mask_element->mask_height();
    } else if (auto const* clip_path_element = as_if<SVG::SVGClipPathElement>(dom_node))
        content_units = clip_path_element->clip_path_units();
    else if (auto const* pattern_element = as_if<SVG::SVGPatternElement>(dom_node)) {
        content_units = pattern_element->pattern_content_units();
        pattern_units = pattern_element->pattern_units();
        pattern_width = pattern_element->pattern_width();
        pattern_height = pattern_element->pattern_height();
    }

    auto geometry_kind = RustFFI::SVG_GEOMETRY_KIND_NONE;
    if (is<SVG::SVGPathElement>(dom_node))
        geometry_kind = RustFFI::SVG_GEOMETRY_KIND_PATH;
    else if (is<SVG::SVGRectElement>(dom_node))
        geometry_kind = RustFFI::SVG_GEOMETRY_KIND_RECT;
    else if (is<SVG::SVGCircleElement>(dom_node))
        geometry_kind = RustFFI::SVG_GEOMETRY_KIND_CIRCLE;
    else if (is<SVG::SVGEllipseElement>(dom_node))
        geometry_kind = RustFFI::SVG_GEOMETRY_KIND_ELLIPSE;
    else if (is<SVG::SVGPolylineElement>(dom_node))
        geometry_kind = RustFFI::SVG_GEOMETRY_KIND_POLYLINE;
    else if (is<SVG::SVGPolygonElement>(dom_node))
        geometry_kind = RustFFI::SVG_GEOMETRY_KIND_POLYGON;

    SVG::NumberPercentage line_x1 = SVG::NumberPercentage::create_number(0);
    SVG::NumberPercentage line_y1 = SVG::NumberPercentage::create_number(0);
    SVG::NumberPercentage line_x2 = SVG::NumberPercentage::create_number(0);
    SVG::NumberPercentage line_y2 = SVG::NumberPercentage::create_number(0);
    if (auto const* line_element = as_if<SVG::SVGLineElement>(dom_node)) {
        geometry_kind = RustFFI::SVG_GEOMETRY_KIND_LINE;
        line_x1 = line_element->x1_value();
        line_y1 = line_element->y1_value();
        line_x2 = line_element->x2_value();
        line_y2 = line_element->y2_value();
    }

    SVG::SVGTextPositioningElement::ParsedTextPositioning text_positioning;
    if (auto const* text_positioning_element = as_if<SVG::SVGTextPositioningElement>(dom_node))
        text_positioning = text_positioning_element->parsed_text_positioning();

    CSS::StyleAtomID reference_fragment;
    SVG::NumberPercentage start_offset = SVG::NumberPercentage::create_number(0);
    if (auto const* text_path_element = as_if<SVG::SVGTextPathElement>(dom_node)) {
        reference_fragment = svg_reference_fragment_atom(dom_node, text_path_element->href_attribute_value());
        start_offset = text_path_element->parsed_start_offset().value_or(start_offset);
    } else if (auto const* pattern_element = as_if<SVG::SVGPatternElement>(dom_node)) {
        // The pattern a <pattern> inherits its content and attributes from. An empty href names
        // nothing, rather than naming the document's own fragment.
        auto link = pattern_element->href_attribute_value();
        if (link.has_value() && !link->is_empty())
            reference_fragment = svg_reference_fragment_atom(dom_node, link);
    }

    // What an <svg>'s natural size is negotiated from: its width and height where they are a
    // <length>, which the pass resolves against the box's style, and the aspect ratio its active
    // SVG view or viewBox gives it.
    RustFFI::FfiSvgLengthValue natural_width { .value = 0, .kind = RustFFI::SVG_LENGTH_KIND_NONE, .unit = 0 };
    RustFFI::FfiSvgLengthValue natural_height { .value = 0, .kind = RustFFI::SVG_LENGTH_KIND_NONE, .unit = 0 };
    Optional<CSSPixelFraction> view_box_aspect_ratio;
    if (auto const* svg_element = as_if<SVG::SVGSVGElement>(dom_node)) {
        auto to_ffi_length = [](Optional<CSS::Length> const& length) -> RustFFI::FfiSvgLengthValue {
            if (!length.has_value())
                return { .value = 0, .kind = RustFFI::SVG_LENGTH_KIND_NONE, .unit = 0 };
            return { .value = length->raw_value(), .kind = RustFFI::SVG_LENGTH_KIND_LENGTH, .unit = static_cast<u8>(to_underlying(length->unit())) };
        };
        natural_width = to_ffi_length(svg_element->width_attribute_length());
        natural_height = to_ffi_length(svg_element->height_attribute_length());
        view_box_aspect_ratio = SVG::SVGSVGElement::view_box_natural_aspect_ratio(*svg_element);
    }

    // The resources an element's style names. They live with the presentation attributes because
    // both are read as a box is built, but they change with the element's style rather than with
    // an attribute, so they have a republication of their own.
    auto style_references = svg_style_reference_atoms(dom_node);

    return {
        .is_graphics_element = is<SVG::SVGGraphicsElement>(dom_node),
        .is_use_element = is<SVG::SVGUseElement>(dom_node),
        .is_svg_svg_element = is<SVG::SVGSVGElement>(dom_node),
        .is_symbol_element = is<SVG::SVGSymbolElement>(dom_node),
        .is_text_element = is<SVG::SVGTextElement>(dom_node),
        .is_fit_to_view_box = fit_to_view_box != nullptr,
        .has_active_view_box = active_view_box.has_value(),
        .active_view_box = active_view_box.has_value() ? to_ffi_svg_view_box(*active_view_box) : RustFFI::FfiSvgViewBox {},
        .preserve_aspect_ratio_align = static_cast<u8>(to_underlying(preserve_aspect_ratio.align)),
        .preserve_aspect_ratio_meet_or_slice = static_cast<u8>(to_underlying(preserve_aspect_ratio.meet_or_slice)),
        .content_units = static_cast<u8>(to_underlying(content_units)),
        .pattern_units = static_cast<u8>(to_underlying(pattern_units)),
        .pattern_width = to_ffi_number_percentage(pattern_width),
        .pattern_height = to_ffi_number_percentage(pattern_height),
        .mask_units = static_cast<u8>(to_underlying(mask_units)),
        .mask_x = to_ffi_number_percentage(mask_x),
        .mask_y = to_ffi_number_percentage(mask_y),
        .mask_width = to_ffi_number_percentage(mask_width),
        .mask_height = to_ffi_number_percentage(mask_height),
        .geometry_kind = geometry_kind,
        .line_x1 = to_ffi_number_percentage(line_x1),
        .line_y1 = to_ffi_number_percentage(line_y1),
        .line_x2 = to_ffi_number_percentage(line_x2),
        .line_y2 = to_ffi_number_percentage(line_y2),
        .text_x = to_ffi_svg_length_value(text_positioning.x),
        .text_y = to_ffi_svg_length_value(text_positioning.y),
        .text_dx = to_ffi_svg_length_value(text_positioning.dx),
        .text_dy = to_ffi_svg_length_value(text_positioning.dy),
        .reference_fragment_atom = reference_fragment.value(),
        .mask_reference_atom = style_references[0].value(),
        .clip_path_reference_atom = style_references[1].value(),
        .fill_reference_atom = style_references[2].value(),
        .stroke_reference_atom = style_references[3].value(),
        .text_path_start_offset = to_ffi_number_percentage(start_offset),
        .natural_width = natural_width,
        .natural_height = natural_height,
        .has_view_box_aspect_ratio = view_box_aspect_ratio.has_value(),
        .view_box_aspect_ratio_numerator = view_box_aspect_ratio.has_value() ? view_box_aspect_ratio->numerator() : 0,
        .view_box_aspect_ratio_denominator = view_box_aspect_ratio.has_value() ? view_box_aspect_ratio->denominator() : 0,
    };
}

// The publication is keyed by the element's style node rather than by a row, because an element
// that draws nothing itself - the <path> inside a <defs> that a <textPath> follows - has no row at
// all, while a mask, a clip or a pattern has one row per referencing element.
void publish_svg_attribute_facts(DOM::Element& element)
{
    VERIFY(element.style_node_id() != 0);
    ReadonlySpan<Gfx::FloatPoint> points;
    if (auto const* polygon = as_if<SVG::SVGPolygonElement>(element))
        points = polygon->points();
    else if (auto const* polyline = as_if<SVG::SVGPolylineElement>(element))
        points = polyline->points();
    static_assert(sizeof(Gfx::FloatPoint) == sizeof(RustFFI::FfiFloatPoint));
    RustFFI::layout_arena_set_style_node_svg_attribute_facts(
        document_layout_arena(element.document()),
        element.style_node_id().value(),
        build_svg_attribute_facts(element),
        reinterpret_cast<RustFFI::FfiFloatPoint const*>(points.data()),
        points.size());
}

// Republished on its own when an element's style is installed: the presentation attributes it sits
// beside are unchanged, and parsing all of them again for four names would make every style change
// on every SVG element pay for it.
void publish_svg_style_references(DOM::Element& element)
{
    VERIFY(element.style_node_id() != 0);
    auto references = svg_style_reference_atoms(element);
    RustFFI::layout_arena_set_style_node_svg_style_references(
        document_layout_arena(element.document()),
        element.style_node_id().value(),
        references[0].value(),
        references[1].value(),
        references[2].value(),
        references[3].value());
}

void clear_svg_attribute_facts(DOM::Document& document, CSS::StyleNodeID style_node)
{
    RustFFI::layout_arena_clear_style_node_svg_attribute_facts(document_layout_arena(document), style_node.value());
}

// Classifies a code point for direction-run splitting during text chunking: strong LTR/RTL,
// direction-neutral Common, or ContextDependent (resolved from surrounding runs).
static Gfx::GlyphRun::TextType text_type_for_code_point(u32 code_point)
{
    // Fast path for ASCII using a lookup table.
    // Each ASCII character has a statically known bidi class.
    if (code_point < 0x80) {
        using enum Gfx::GlyphRun::TextType;
        // clang-format off
        static constexpr auto L = Ltr;
        static constexpr auto C = Common;
        static constexpr auto X = ContextDependent;
        static constexpr Gfx::GlyphRun::TextType ascii_text_types[128] = {
            // 0x00-0x0F: Control characters (BN=Common, S/B/WS=ContextDependent)
            C, C, C, C, C, C, C, C, C, X, X, X, X, X, C, C,
            // 0x10-0x1F: Control characters
            C, C, C, C, C, C, C, C, C, C, C, C, X, X, X, X,
            // 0x20-0x2F: Space and punctuation
            X, C, C, X, X, X, C, C, C, C, C, X, X, X, X, X,
            // 0x30-0x3F: Digits and punctuation
            X, X, X, X, X, X, X, X, X, X, X, C, C, C, C, C,
            // 0x40-0x4F: @ and uppercase letters
            C, L, L, L, L, L, L, L, L, L, L, L, L, L, L, L,
            // 0x50-0x5F: Uppercase letters and punctuation
            L, L, L, L, L, L, L, L, L, L, L, C, C, C, C, C,
            // 0x60-0x6F: ` and lowercase letters
            C, L, L, L, L, L, L, L, L, L, L, L, L, L, L, L,
            // 0x70-0x7F: Lowercase letters and punctuation
            L, L, L, L, L, L, L, L, L, L, L, C, C, C, C, C,
        };
        // clang-format on
        return ascii_text_types[code_point];
    }

    switch (Unicode::bidirectional_class(code_point)) {
    case Unicode::BidiClass::WhiteSpaceNeutral:

    case Unicode::BidiClass::BlockSeparator:
    case Unicode::BidiClass::SegmentSeparator:
    case Unicode::BidiClass::CommonNumberSeparator:
    case Unicode::BidiClass::DirNonSpacingMark:

    case Unicode::BidiClass::ArabicNumber:
    case Unicode::BidiClass::EuropeanNumber:
    case Unicode::BidiClass::EuropeanNumberSeparator:
    case Unicode::BidiClass::EuropeanNumberTerminator:
        return Gfx::GlyphRun::TextType::ContextDependent;

    case Unicode::BidiClass::BoundaryNeutral:
    case Unicode::BidiClass::OtherNeutral:
    case Unicode::BidiClass::FirstStrongIsolate:
    case Unicode::BidiClass::PopDirectionalFormat:
    case Unicode::BidiClass::PopDirectionalIsolate:
        return Gfx::GlyphRun::TextType::Common;

    case Unicode::BidiClass::LeftToRight:
    case Unicode::BidiClass::LeftToRightEmbedding:
    case Unicode::BidiClass::LeftToRightIsolate:
    case Unicode::BidiClass::LeftToRightOverride:
        return Gfx::GlyphRun::TextType::Ltr;

    case Unicode::BidiClass::RightToLeft:
    case Unicode::BidiClass::RightToLeftArabic:
    case Unicode::BidiClass::RightToLeftEmbedding:
    case Unicode::BidiClass::RightToLeftIsolate:
    case Unicode::BidiClass::RightToLeftOverride:
        return Gfx::GlyphRun::TextType::Rtl;

    default:
        VERIFY_NOT_REACHED();
    }
}

void* document_layout_arena(DOM::Document& document)
{
    return document.layout_arena();
}

void* document_layout_arena_if_created(DOM::Document const& document)
{
    return document.layout_arena_handle();
}

Optional<RustFFI::DocumentId> document_render_document_if_created(DOM::Document const& document)
{
    if (!document.layout_arena_handle())
        return {};
    return document.render_document_id();
}

RustFFI::DocumentId document_render_document(DOM::Document& document)
{
    (void)document_layout_arena(document);
    return document.render_document_id();
}

// The StyleNodeID a row bound to this DOM node records: an element's or a text node's.
static CSS::StyleNodeID style_node_of(DOM::Node const* node)
{
    if (auto const* element = as_if<DOM::Element>(node))
        return element->style_node_id();
    if (auto const* text = as_if<DOM::Text>(node))
        return text->style_node_id();
    return {};
}

u8 dom_paint_facts_of(GC::Ptr<DOM::Node const> node)
{
    if (!node)
        return 0;
    u8 facts = 0;
    if (node->is_inert())
        facts |= static_cast<u8>(RustFFI::DomPaintFact::Inert);
    if (node->is_editable_or_editing_host())
        facts |= static_cast<u8>(RustFFI::DomPaintFact::EditableOrEditingHost);
    if (node->inside_blocking_wheel_event_handler())
        facts |= static_cast<u8>(RustFFI::DomPaintFact::InsideBlockingWheelEventHandler);
    if (auto const* navigable_container = as_if<HTML::NavigableContainer>(*node); navigable_container && navigable_container->content_navigable())
        facts |= static_cast<u8>(RustFFI::DomPaintFact::NestedNavigableContainer);
    return facts;
}

// What a row built for the node is painted and hit-tested with, recorded under the node's
// identity for the rows the build has yet to stamp and journalled for the rows it already has.
// Neither half needs the node to have a box, which is why this is not a row's own business: the
// build reads the recorded answer for a node that gains one.
void publish_dom_paint_facts(DOM::Node const& dom_node)
{
    auto& document = const_cast<DOM::Document&>(dom_node.document());
    auto facts = dom_paint_facts_of(&dom_node);
    // A document nothing is ever inert, editable or wheel-handled in publishes nothing, and the
    // flag is what keeps a node's arrival free on such a page. It is set before the first non-zero
    // publication, so a later drop back to zero is still published.
    if (facts == 0 && !document.may_have_dom_paint_facts())
        return;
    if (facts != 0)
        document.set_may_have_dom_paint_facts();
    auto identity = dom_node.is_document() ? document.style_node_id() : style_node_of(&dom_node);
    // A change is recorded as the arrival was, so the last one recorded is what goes in.
    if (identity.value() != 0)
        document.render_inputs_for_write().style_engine().record_dom_paint_facts(identity, facts);
    document.invalidation_journal().note_dom_paint_facts(DOM::NodeIdentity::of(dom_node), facts);
}

// Whether a row built for the node sits in the user agent shadow tree of the focused text
// control, which is what a caret and a selection are painted inside. The answer moves only when
// the focused area does, and a node in no user agent shadow tree can never hold it, so the
// published set holds one control's shadow tree at a time.
void publish_is_in_focused_text_control(DOM::Node const& node)
{
    auto& document = const_cast<DOM::Document&>(node.document());
    auto shadow_root = node.containing_shadow_root();
    auto value = shadow_root
        && shadow_root->is_user_agent_internal()
        && is<HTML::FormAssociatedTextControlElement>(shadow_root->host())
        && shadow_root->host()->is_focused();
    auto* arena = document_layout_arena_if_created(document);
    if (!arena) {
        if (!value)
            return;
        arena = document_layout_arena(document);
    }
    auto identity = style_node_of(&node);
    if (identity.value() != 0)
        RustFFI::layout_arena_set_identity_in_focused_text_control(arena, identity.value(), value);
}

// What a row built for the element is scrolled to. The element's box is replaced whenever its
// subtree is rebuilt, so the offset is held against the identity that outlives it and a row the
// build stamps reads it there, the way a pseudo-element's box already does. The rows the element
// already has are published to separately, by the caller that has one in hand.
void publish_element_scroll_offset(DOM::Element const& element)
{
    auto& document = const_cast<DOM::Document&>(element.document());
    auto offset = element.scroll_offset({});
    auto* arena = document_layout_arena_if_created(document);
    if (!arena) {
        // Nothing has scrolled anything before a layout tree exists, so there is no offset to
        // forget, as for a pseudo-element's.
        if (offset.is_zero())
            return;
        arena = document_layout_arena(document);
    }
    if (element.style_node_id().value() != 0)
        RustFFI::layout_arena_set_element_scroll_offset(arena, element.style_node_id().value(), offset);
}

TableSpans table_spans_of(DOM::Node const* node)
{
    TableSpans spans;
    if (auto const* cell = as_if<HTML::HTMLTableCellElement>(node)) {
        spans.column_span = static_cast<u16>(cell->col_span());
        spans.row_span = static_cast<u16>(cell->row_span());
    } else if (auto const* column = as_if<HTML::HTMLTableColElement>(node)) {
        spans.column_span = static_cast<u16>(column->span());
        // The raw span keeps the unclamped attribute value; its only consumer is the
        // table formatting context's column handling, so other elements' span
        // attributes stay out of the arena map.
        spans.raw_column_span = column->get_attribute_value(HTML::AttributeNames::span).to_number<u32>().value_or(1);
    }
    return spans;
}

// The spans a row built for a table cell or table column takes from its attributes, recorded
// under the element's identity for the rows the build has yet to stamp. The rows the element
// already has are synchronised by the attribute change that moved the spans.
void publish_table_spans(DOM::Element const& element)
{
    if (!is<HTML::HTMLTableCellElement>(element) && !is<HTML::HTMLTableColElement>(element))
        return;
    auto identity = element.style_node_id();
    if (identity.value() == 0)
        return;
    auto spans = table_spans_of(&element);
    // A change is recorded as the arrival was, so the last one recorded is what goes in.
    const_cast<DOM::Document&>(element.document()).render_inputs_for_write().style_engine().record_table_spans(identity, spans.column_span, spans.row_span, spans.raw_column_span);
}

// The box whose scroll snap container a box's style describes: the viewport for the root element's, as the scroll snap
// properties specified on the root element apply to the viewport rather than to its own box.
static Painting::BoxSlot scroll_snap_container_of(Painting::BoxSlot const& box, DOM::Node const* dom_node)
{
    if (box.is_viewport() || (dom_node && dom_node == box.document().document_element()))
        return Painting::BoxSlot::viewport_of(box.document());
    if (!box.is_scroll_container())
        return {};
    return box;
}

// What a box taking a style record tells the rest of the document. `dom_node` is the node the box was built for.
static void did_update_box_style_record(Painting::BoxSlot const& box, DOM::Node const* dom_node, void const* style_payloads)
{
    auto& document = box.document();
    if (auto const* element = as_if<DOM::Element>(dom_node); element && element->has_style(CSS::PseudoElement::Selection))
        Painting::push_selection_pseudo_style(*element);

    if (CSS::style_group_from_payloads<CSS::ComputedValues::MiscResetValues>(style_payloads)->scroll_snap_type_value().strictness != CSS::ScrollSnapStrictness::None)
        document.set_may_have_scroll_snap_areas();

    // NB: The root element's style can be published before the layout tree gives the document a viewport to snap
    //     with, and is published again once building the layout tree binds this node's style record.
    auto snap_container = scroll_snap_container_of(box, dom_node);
    if (!snap_container)
        return;

    // A style change can make a box a snap container without the paint tree being built again, so the box registers
    // itself here as well as when it is built.
    if (Painting::is_scroll_snap_container(snap_container)) {
        document.register_scroll_snap_container(snap_container);
        return;
    }

    // A box that does not snap is snapped to no snap areas, so that a scroll it is given while it does not snap is not
    // undone by a re-snap once it snaps again.
    document.forget_snapped_areas_of_scroll_container(snap_container);
}

enum class RowStyle {
    Install,
    AdoptOwners,
};

static void apply_style_to_box(Painting::BoxSlot const& box, CSS::PublishedStyleRecord const& style_record, RowStyle row_style)
{
    if (!box.is_live() || box.is_text())
        return;
    // The install lets go of the row's image observers and notes that it attached no images, which is all a style that
    // holds none attaches. The observers go once the row's new ones observe, so a shared resource is never dropped and
    // refetched.
    auto* send_row_style = row_style == RowStyle::AdoptOwners ? RustFFI::layout_arena_adopt_owner_row_style : RustFFI::layout_arena_install_row_style;
    auto released_image_observers = adopt_own_if_nonnull(static_cast<Painting::StyleImageObserverSet*>(send_row_style(box.arena(), box.slot(), style_record.handle())));
    auto dom_node = box.dom_node();
    did_update_box_style_record(box, dom_node.ptr(), style_record.payloads());
    if (has_flag(style_record.dependency_flags(), CSS::StyleRecordDependencyFlag::HoldsImageValues)) {
        attach_style_resources_to_box(box);
        return;
    }
    // Only a row that held images has paint facts of them to clear.
    Painting::push_paint_facts_after_style_attach(box, dom_node.ptr(), released_image_observers ? Painting::StyleHoldsImageValues::No : Painting::StyleHoldsImageValues::NoAndHeldNone);
}

void apply_style_to_box(Painting::BoxSlot const& box, CSS::PublishedStyleRecord const& style_record)
{
    apply_style_to_box(box, style_record, RowStyle::Install);
}

void adopt_owner_style_of_box(Painting::BoxSlot const& box, CSS::PublishedStyleRecord const& style_record)
{
    apply_style_to_box(box, style_record, RowStyle::AdoptOwners);
}

void attach_style_resources_to_box(Painting::BoxSlot const& box)
{
    auto const* style_payloads = box.style_payloads();
    if (!style_payloads)
        return;
    auto& document = box.document();
    auto* arena = box.arena();
    auto slot = box.slot();
    auto dom_node = box.dom_node();

    // The style engine notes at publication whether a record holds an <image> anywhere this box would load and
    // observe one. Nearly every style holds none, and that answer is one flag read; the walk below stays for the
    // styles that do.
    auto dependency_flags = static_cast<CSS::StyleRecordDependencyFlag>(RustFFI::layout_arena_node_style_dependency_flags(arena, slot));
    if (!has_flag(dependency_flags, CSS::StyleRecordDependencyFlag::HoldsImageValues)) {
        Painting::replace_style_image_observers(document, slot, nullptr);
        // The row keeps nothing a later attach would have to take away, which is what lets the
        // tree build skip asking for one at all.
        RustFFI::layout_arena_note_style_image_resources_attached(arena, slot, false);
        Painting::push_paint_facts_after_style_attach(box, dom_node.ptr(), Painting::StyleHoldsImageValues::No);
        return;
    }

    auto observers = make<Painting::StyleImageObserverSet>();
    observers->background_layer_data = CSS::style_group_from_payloads<CSS::ComputedValues::BackgroundValues>(style_payloads)->background_layers_value();
    observers->mask_layer_data = CSS::style_group_from_payloads<CSS::ComputedValues::MaskValues>(style_payloads)->mask_layers_value();
    observers->border_image = CSS::style_group_from_payloads<CSS::ComputedValues::BorderValues>(style_payloads)->border_image_value();
    auto cursors = CSS::style_group_from_payloads<CSS::ComputedValues::InheritedUIValues>(style_payloads)->cursor_span();
    RefPtr<CSS::AbstractImageStyleValue const> list_style_image = CSS::style_group_from_payloads<CSS::ComputedValues::InheritedListValues>(style_payloads)->list_style_image_value();

    auto load_image = [&](CSS::AbstractImageStyleValue const* image) {
        if (image)
            const_cast<CSS::AbstractImageStyleValue&>(*image).load_any_resources(document);
    };
    for (auto const& layer : observers->background_layer_data)
        load_image(layer.background_image.ptr());
    for (auto const& layer : observers->mask_layer_data)
        load_image(layer.background_image.ptr());
    load_image(observers->border_image.source.ptr());
    observers->cursor_style_values.ensure_capacity(cursors.size());
    for (auto const& cursor_data : cursors) {
        auto cursor_style_value = CSS::ComputedValues::InheritedUIValues::cursor_style_value(cursor_data);
        if (cursor_style_value)
            load_image(&cursor_style_value->image());
        observers->cursor_style_values.unchecked_append(move(cursor_style_value));
    }
    load_image(list_style_image.ptr());

    auto observer_for = [&](CSS::AbstractImageStyleValue const* image) -> OwnPtr<Painting::StyleImageObserver> {
        if (!image)
            return nullptr;
        auto const* image_to_observe = image->selected_image_style_value();
        if (!image_to_observe)
            return nullptr;
        return make<Painting::StyleImageObserver>(document, slot, *image_to_observe);
    };
    for (auto const& layer : observers->background_layer_data)
        observers->background_layers.append(observer_for(layer.background_image.ptr()));
    for (auto const& layer : observers->mask_layer_data)
        observers->mask_layers.append(observer_for(layer.background_image.ptr()));
    for (auto const& cursor_style_value : observers->cursor_style_values)
        observers->cursors.append(cursor_style_value ? observer_for(&cursor_style_value->image()) : nullptr);
    observers->border_image_source = observer_for(observers->border_image.source.ptr());
    observers->list_style_image = observer_for(list_style_image.ptr());
    // TODO: Observe other <image> accepting properties once we support them.

    Painting::replace_style_image_observers(document, slot, move(observers));
    RustFFI::layout_arena_note_style_image_resources_attached(arena, slot, true);
    Painting::push_paint_facts_after_style_attach(box, dom_node.ptr(), Painting::StyleHoldsImageValues::Yes);
}

void set_style_record_of_box(Painting::BoxSlot const& box, CSS::PublishedStyleRecord const* style_record)
{
    // A box has style for as long as it lives: a record taken away from its DOM target leaves the box the one it has.
    if (!style_record || !box.is_live() || box.is_text())
        return;
    // A layout-derived record is independent of its DOM target's record. A rendering consequence replaces and
    // re-derives it explicitly through apply_style_to_box().
    if (RustFFI::layout_arena_replace_row_style_record(box.arena(), box.slot(), style_record->handle()))
        did_update_box_style_record(box, box.dom_node().ptr(), style_record->payloads());
}

// Whether a box is the one its pseudo-element is bound to. The generated content inside the box carries the same
// generator and type, so only this binding tells the box from its content.
static bool is_bound_to_pseudo_element(Painting::BoxSlot const& box, DOM::Element const& generator, CSS::PseudoElement pseudo_element)
{
    return Painting::BoxSlot::bound_to(box.document(), DOM::NodeIdentity::of(generator), pseudo_element) == box;
}

// The scroll offset a box's DOM target holds for it. An element's box holds the element's scroll offset. Everything
// generated for a pseudo-element names it as generator, but only the pseudo-element's own box is what scrolls, so only
// that box holds the pseudo-element's offset; the generated content inside it holds none.
static CSSPixelPoint dom_target_scroll_offset(Painting::BoxSlot const& box)
{
    if (box.is_viewport()) {
        auto navigable = box.document().navigable();
        return navigable ? navigable->viewport_scroll_offset() : CSSPixelPoint {};
    }
    if (auto pseudo_element = box.generated_for_pseudo_element(); pseudo_element.has_value()) {
        // A compositor scroll can reach a removed generator's box before the layout tree drops it.
        auto generator = box.pseudo_element_generator();
        if (!generator)
            return {};
        auto synthetic_pseudo_element = generator->get_synthetic_pseudo_element(*pseudo_element);
        if (!synthetic_pseudo_element.has_value() || !is_bound_to_pseudo_element(box, *generator, *pseudo_element))
            return {};
        return synthetic_pseudo_element->scroll_offset();
    }
    if (auto const* element = as_if<DOM::Element>(box.dom_node().ptr()))
        return element->scroll_offset({});
    return {};
}

void publish_scroll_offset_of_box(Painting::BoxSlot const& box)
{
    if (!box.is_live())
        return;
    RustFFI::layout_arena_for_each_row_built_for_same_node(box.arena(), box.slot(), &box.document(),
        [](void* context, Compositing::RustFFI::NodeSlotId slot) {
            auto row = Painting::BoxSlot::of(*static_cast<DOM::Document*>(context), slot);
            auto offset = dom_target_scroll_offset(row);
            // The navigable stores the viewport's offset, and the arena knows the viewport's box holds it without
            // being told.
            RustFFI::layout_arena_publish_scroll_offset(row.arena(), slot, offset, !row.is_viewport() && !offset.is_zero());
        });
}

void synchronize_table_spans_of_box(Painting::BoxSlot const& box)
{
    if (!box.is_live())
        return;
    auto spans = table_spans_of(box.dom_node().ptr());
    RustFFI::layout_arena_set_table_spans(box.arena(), box.slot(), spans.column_span, spans.row_span, spans.raw_column_span);
}

bool update_empty_line_box_fragment_flag_of_box(Painting::BoxSlot const& text_box)
{
    if (!text_box.is_live())
        return false;
    // Text controls and editing hosts rely on their text node producing a zero-width fragment even
    // when it has no text: the fragment keeps the line box alive with real font metrics, giving the
    // caret an anchor to paint at and the control its baseline. Stamping this as a node flag keeps
    // layout itself unaware of editing state.
    auto produces_line_box_fragment_when_empty = [&] {
        auto const* dom_text = as_if<DOM::Text>(text_box.dom_node().ptr());
        if (!dom_text)
            return false;
        if (auto const* shadow_root = as_if<DOM::ShadowRoot>(dom_text->root())) {
            if (as_if<HTML::FormAssociatedTextControlElement>(shadow_root->host()))
                return true;
        }
        return dom_text->parent() && dom_text->parent()->is_editing_host();
    }();
    if (text_box.has_flag(RustFFI::NodeFlag::ProducesLineBoxFragmentWhenEmpty) == produces_line_box_fragment_when_empty)
        return false;
    RustFFI::layout_arena_set_node_flag(text_box.arena(), text_box.slot(), RustFFI::HostNodeFlag::ProducesLineBoxFragmentWhenEmpty, produces_line_box_fragment_when_empty);
    return true;
}

void register_layout_host(DOM::Document& document)
{
    auto* arena = document_layout_arena_if_created(document);
    VERIFY(arena);
    RustFFI::layout_arena_register_style_engine(arena, document.style_computer().style_engine().rust_handle());
    // The render side says which nodes have a box and which of those boxes layout committed, so that DOM code reads a
    // bit instead of looking up the node's row.
    RustFFI::layout_arena_set_box_presence_host(arena, &document, [](void* context, u32 style_node, u8 bits) {
        auto& document = *static_cast<DOM::Document*>(context);
        // The document has no style node of its own; it is named by 0.
        auto identity = style_node == 0 ? DOM::NodeIdentity::of_document() : DOM::NodeIdentity::of_style_node(CSS::StyleNodeID { style_node });
        document.commit_messages().note_box_presence(identity,
            (bits & RustFFI::BOX_PRESENCE_HAS_LAYOUT_BOX) != 0,
            (bits & RustFFI::BOX_PRESENCE_HAS_COMMITTED_BOX) != 0);
    });
    RustFFI::layout_arena_set_chrome_state_callback(arena, &document,
        [](void* context, bool viewport_row_was_recommitted) {
            auto& document = *static_cast<DOM::Document*>(context);
            document.chrome_widget_registry().drop_widgets_of_reset_rows();
            if (viewport_row_was_recommitted)
                document.paint_state().viewport_row_was_reset();
        });
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::None) == 0);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMinYMin) == 1);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMidYMin) == 2);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMaxYMin) == 3);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMinYMid) == 4);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMidYMid) == 5);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMaxYMid) == 6);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMinYMax) == 7);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMidYMax) == 8);
    static_assert(to_underlying(SVG::PreserveAspectRatio::Align::xMaxYMax) == 9);
    static_assert(to_underlying(SVG::PreserveAspectRatio::MeetOrSlice::Meet) == 0);
    static_assert(to_underlying(SVG::PreserveAspectRatio::MeetOrSlice::Slice) == 1);
    static_assert(to_underlying(SVG::SVGUnits::ObjectBoundingBox) == 0);
    static_assert(to_underlying(SVG::SVGUnits::UserSpaceOnUse) == 1);
    RustFFI::FfiLayoutHostCallbacks callbacks {
        .context = &document,
        .deliver_commit_messages = [](void* context, RustFFI::FfiCommitMessage const* messages, size_t count) {
            auto& document = *static_cast<DOM::Document*>(context);
            for (auto const& message : ReadonlySpan<RustFFI::FfiCommitMessage> { messages, count }) {
                // A new layout tree gets a new paint state once the document has taken in what the build found out
                // before it, and before what comes after it.
                if (message.kind == RustFFI::FfiCommitMessageKind::LayoutTreeReplaced) {
                    document.commit_messages().apply_script_free();
                    document.renew_paint_state();
                    continue;
                }
                document.commit_messages().append(message);
            }
            // The pass that produced them reads back what they change before it ends. They can arrive
            // as a forced join takes a frame back, so the continuations wait for the next drain point.
            document.commit_messages().apply_script_free(); },
        .take_built_scroll_containers = [](void* context, RustFFI::FfiBuiltScrollContainer const* built, size_t count) {
            auto& document = *static_cast<DOM::Document*>(context);
            for (auto const& scroll_container : ReadonlySpan<RustFFI::FfiBuiltScrollContainer> { built, count })
                Painting::take_built_scroll_container(document, scroll_container.slot, scroll_container.is_scroll_snap_container); },
    };
    RustFFI::layout_arena_set_layout_host_callbacks(arena, callbacks);
    RustFFI::layout_arena_set_document_is_decoded_svg(arena, document.is_decoded_svg());
    Painting::register_geometry_host(document);
}

void unregister_layout_host(DOM::Document& document)
{
    auto* arena = document_layout_arena_if_created(document);
    if (!arena)
        return;
    RustFFI::layout_arena_clear_chrome_state_callback(arena);
    RustFFI::layout_arena_unregister_style_engine(arena);
    RustFFI::layout_arena_clear_layout_host_callbacks(arena);
    RustFFI::layout_arena_clear_layout_update_host_callbacks(arena);
}

}

extern "C" WEB_API u8 ladybird_layout_text_type_for_code_point(u32 code_point)
{
    return static_cast<u8>(to_underlying(Web::Layout::text_type_for_code_point(code_point)));
}

extern "C" WEB_API bool ladybird_layout_code_point_has_break_all_line_break_class(u32 code_point)
{
    return first_is_one_of(Unicode::line_break_class(code_point),
        Unicode::LineBreakClass::Alphabetic,
        Unicode::LineBreakClass::Numeric,
        Unicode::LineBreakClass::ComplexContext,
        Unicode::LineBreakClass::Ideographic);
}

extern "C" WEB_API bool ladybird_layout_code_point_has_keep_all_line_break_class(u32 code_point)
{
    return first_is_one_of(Unicode::line_break_class(code_point),
        Unicode::LineBreakClass::Alphabetic,
        Unicode::LineBreakClass::Numeric,
        Unicode::LineBreakClass::Ambiguous,
        Unicode::LineBreakClass::Ideographic);
}

extern "C" WEB_API bool ladybird_layout_code_point_has_combining_mark_line_break_class(u32 code_point)
{
    return Unicode::line_break_class(code_point) == Unicode::LineBreakClass::CombiningMark;
}

extern "C" WEB_API bool ladybird_layout_code_point_has_emoji_property(u32 code_point)
{
    return Unicode::code_point_has_emoji_property(code_point);
}

extern "C" WEB_API Web::Layout::RustFFI::FfiCodePointCategoryFacts ladybird_layout_code_point_category_facts(u32 code_point)
{
    static auto const ps = Unicode::general_category_from_string("Ps"sv).value();
    static auto const pd = Unicode::general_category_from_string("Pd"sv).value();
    return {
        .is_space_separator = Unicode::code_point_has_space_separator_general_category(code_point),
        .is_punctuation = Unicode::code_point_has_punctuation_general_category(code_point),
        .is_letter = Unicode::code_point_has_letter_general_category(code_point),
        .is_number = Unicode::code_point_has_number_general_category(code_point),
        .is_symbol = Unicode::code_point_has_symbol_general_category(code_point),
        .is_open_punctuation = Unicode::code_point_has_general_category(code_point, ps),
        .is_dash_punctuation = Unicode::code_point_has_general_category(code_point, pd),
    };
}

extern "C" WEB_API void ladybird_layout_owned_image_provider_destroy(void* image_provider)
{
    delete static_cast<Web::Layout::ImageProvider*>(image_provider);
}

extern "C" WEB_API void ladybird_layout_owned_image_provider_notify_detach(void* image_provider)
{
    static_cast<Web::Layout::ImageProvider*>(image_provider)->layout_node_was_detached();
}
