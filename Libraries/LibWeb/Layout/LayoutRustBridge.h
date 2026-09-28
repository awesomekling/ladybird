/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullRefPtr.h>
#include <AK/Optional.h>
#include <AK/OwnPtr.h>
#include <AK/RefPtr.h>
#include <AK/Variant.h>
#include <AK/Vector.h>
#include <LibGC/Ptr.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/PercentageOr.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Layout/TreeBuilderRustFFI.h>
#include <LibWeb/SVG/AttributeParsing.h>

namespace Web::CSS {

class LengthPercentage;
class LengthPercentageOrAuto;
class Size;

}

namespace Web::Layout {

// The document's layout node arena as the Rust layout entries take it; the first creates the arena if the document
// has none yet.
WEB_API void* document_layout_arena(DOM::Document&);
WEB_API void* document_layout_arena_if_created(DOM::Document const&);
// The name the render owner knows the document's render state by, if the document has an arena.
WEB_API Optional<RustFFI::DocumentId> document_render_document_if_created(DOM::Document const&);
// The same, made now if the document has no arena yet.
WEB_API RustFFI::DocumentId document_render_document(DOM::Document&);

// A layout tree build's walk runs on the render side; the document readies it.
RustFFI::FfiDocumentStyleForBuild document_style_for_build(DOM::Document&);

// What a layout tree build owes the rows it stamped for their images, which the document attaches once the frame the
// build ran in is over. Each answers whether the box was handed a provider whose image is already there.
bool attach_owed_style_resources(DOM::Document&, Compositing::RustFFI::NodeSlotId, bool owns_content_replacement_image);
bool attach_owed_generated_image(DOM::Document&, Compositing::RustFFI::NodeSlotId, u32 style_node, RustFFI::FfiPseudoElement, RustFFI::FfiGeneratedContentItem, Compositing::RustFFI::NodeSlotId pseudo_element_box);

// Registers the document-side answers every layout pass needs on the document's arena, once it has one.
WEB_API void register_layout_host(DOM::Document&);

// What the document publishes to the arena about a node, under its identity, for the rows built for it: what the rows
// are painted and hit-tested with, what the element is scrolled to, whether the node sits in the focused text control,
// and the spans a table cell or column takes from its attributes.
void publish_dom_paint_facts(DOM::Node const&);
void publish_element_scroll_offset(DOM::Element const&);
void publish_is_in_focused_text_control(DOM::Node const&);
void publish_table_spans(DOM::Element const&);
u8 dom_paint_facts_of(GC::Ptr<DOM::Node const>);
struct TableSpans {
    u16 column_span { 1 };
    u16 row_span { 1 };
    u32 raw_column_span { 1 };
};
TableSpans table_spans_of(DOM::Node const*);

// Sets the record of the box's row as its element's record moved, without applying the style to the box.
WEB_API void set_box_style_record(Painting::BoxSlot const&, CSS::PublishedStyleRecord const*);

// What a style change does to a box beyond the record its row installs: re-deriving its values from the record, or,
// for a change that only moves the images it names, loading and observing those.
WEB_API void apply_style_to_box(Painting::BoxSlot const&, CSS::PublishedStyleRecord const&);
WEB_API void attach_style_resources_to_box(Painting::BoxSlot const&);
// Makes the host's mirror of the box now, from the record its row holds, if nothing has made it yet.
WEB_API void make_host_mirror_of_box(Painting::BoxSlot const&);

// Publishes what the SVG element's presentation attributes parse to, under its style node, and
// retires that publication. An element's attributes are layout input that no pass can change, so
// the document publishes them as they are written rather than answering for them while a pass runs.
void publish_svg_attribute_facts(DOM::Element&);
void publish_svg_style_references(DOM::Element&);
void clear_svg_attribute_facts(DOM::Document&, CSS::StyleNodeID);

inline RustFFI::FfiSvgNumberPercentage to_ffi_number_percentage(SVG::NumberPercentage value)
{
    return { .value = value.value(), .is_percentage = value.is_percentage() };
}

}

// Per-code-point classification lookups for native text processing. The
// line-break-class groupings implement the css-text-4 word-break policies.
extern "C" WEB_API u8 ladybird_layout_text_type_for_code_point(u32);
extern "C" WEB_API bool ladybird_layout_code_point_has_break_all_line_break_class(u32);
extern "C" WEB_API bool ladybird_layout_code_point_has_keep_all_line_break_class(u32);
extern "C" WEB_API bool ladybird_layout_code_point_has_combining_mark_line_break_class(u32);
extern "C" WEB_API bool ladybird_layout_code_point_has_emoji_property(u32);
extern "C" WEB_API Web::Layout::RustFFI::FfiCodePointCategoryFacts ladybird_layout_code_point_category_facts(u32);

extern "C" WEB_API void ladybird_layout_node_shell_destroy(void*);
extern "C" WEB_API void ladybird_layout_owned_image_provider_destroy(void*);
extern "C" WEB_API void ladybird_layout_image_observers_destroy(void*);
extern "C" WEB_API void ladybird_layout_owned_image_provider_notify_detach(void*);
