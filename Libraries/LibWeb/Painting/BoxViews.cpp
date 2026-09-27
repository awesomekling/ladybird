/*
 * Copyright (c) 2026, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/CSS/StyleInvalidation.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/InvalidationJournal.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/DOM/Position.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/DOM/Text.h>
#include <LibWeb/HTML/FormAssociatedElement.h>
#include <LibWeb/HTML/HTMLAreaElement.h>
#include <LibWeb/HTML/HTMLBRElement.h>
#include <LibWeb/HTML/HTMLHtmlElement.h>
#include <LibWeb/HTML/HTMLImageElement.h>
#include <LibWeb/HTML/HTMLMapElement.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/Window.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/DocumentPaintState.h>
#include <LibWeb/Painting/PaintingRustBridge.h>
#include <LibWeb/SVG/SVGFilterElement.h>
#include <LibWebCommon/CSS/SystemColor.h>

namespace Web::Painting {

static bool g_paint_viewport_scrollbars = true;

void set_paint_viewport_scrollbars(bool enabled)
{
    g_paint_viewport_scrollbars = enabled;
}

bool should_paint_viewport_scrollbars()
{
    return g_paint_viewport_scrollbars;
}

static bool body_background_is_propagated_to_root(BoxSlot const& row)
{
    if (!row.has_flag(Layout::RustFFI::NodeFlag::IsBody))
        return false;
    auto const* html_element = row.document().html_element();
    return html_element && html_element->should_use_body_background_properties();
}

GC::Ptr<SVG::SVGFilterElement> resolve_svg_filter_reference(CSS::ComputedValuesFFI::ComputedStyleValueHandle const& url_value, DOM::Document const& document)
{
    auto fragment = CSS::ComputedFilterView::url_fragment(url_value);
    auto referenced_element = fragment.is_empty() ? nullptr : document.get_element_by_id(fragment);
    return referenced_element ? as_if<SVG::SVGFilterElement>(*referenced_element) : nullptr;
}

Compositing::RustFFI::NodeSlotId viewport_row_slot(DOM::Document const& document)
{
    return BoxSlot::viewport_of(document).slot();
}

static BoxSlot box_slot(DOM::Document const& document, DOM::NodeIdentity identity)
{
    return BoxSlot::bound_to(document, identity);
}

static Layout::RustFFI::FfiCommittedRow committed_row(BoxSlot const& node)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_committed_row(node.arena(), node.slot());
}

bool has_committed_box(BoxSlot const& node)
{
    return node && Layout::RustFFI::layout_arena_has_committed_box(node.arena(), node.slot());
}

Optional<Layout::RustFFI::NodeKind> bound_row_kind(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return box.kind();
}

Compositing::RustFFI::NodeSlotId committed_row_slot(DOM::Document const& document, DOM::NodeIdentity identity)
{
    return box_slot(document, identity).slot();
}

bool has_committed_box(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    return box && Layout::RustFFI::layout_arena_has_committed_box(box.arena(), box.slot());
}

BoxSlot committed_box(DOM::Document const& document, Compositing::RustFFI::NodeSlotId slot)
{
    auto* arena = Layout::document_layout_arena_if_created(document);
    if (!arena || slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return {};
    return BoxSlot::of(document, Layout::RustFFI::layout_arena_paintable_committed_slot(arena, slot));
}

u64 committed_row_reset_version(DOM::Document const& document, Compositing::RustFFI::NodeSlotId slot)
{
    auto* arena = Layout::document_layout_arena_if_created(document);
    return arena ? Layout::RustFFI::layout_arena_paintable_row_reset_version(arena, slot) : 0;
}

DOM::NodeIdentity dom_node_identity_of_committed_slot(DOM::Document const& document, Compositing::RustFFI::NodeSlotId slot)
{
    return BoxSlot::of(document, slot).dom_node_identity();
}

static PixelBox pixel_box_from_ffi(Layout::RustFFI::FfiPixelBox const& box)
{
    return { box.top, box.right, box.bottom, box.left };
}

CSSPixelRect absolute_rect(BoxSlot const& node)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_paintable_absolute_rect(node.arena(), node.slot());
}

CSSPixelRect absolute_padding_box_rect(BoxSlot const& node)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_paintable_absolute_padding_box_rect(node.arena(), node.slot());
}

CSSPixelRect absolute_border_box_rect(BoxSlot const& node)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_paintable_absolute_border_box_rect(node.arena(), node.slot());
}

CSSPixelPoint absolute_position(BoxSlot const& node)
{
    return absolute_rect(node).location();
}

CSSPixelSize content_size(BoxSlot const& node)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_paintable_content_size(node.arena(), node.slot());
}

CSSPixels content_width(BoxSlot const& node)
{
    return content_size(node).width();
}

CSSPixels content_height(BoxSlot const& node)
{
    return content_size(node).height();
}

BoxModelMetrics box_model(BoxSlot const& node)
{
    if (!node)
        return {};
    auto metrics = Layout::RustFFI::layout_arena_paintable_box_model(node.arena(), node.slot());
    return {
        .margin = pixel_box_from_ffi(metrics.margin),
        .padding = pixel_box_from_ffi(metrics.padding),
        .border = pixel_box_from_ffi(metrics.border),
        .inset = pixel_box_from_ffi(metrics.inset),
    };
}

CSSPixels border_box_width(BoxSlot const& node)
{
    auto border_box = box_model(node).border_box();
    return content_width(node) + border_box.left + border_box.right;
}

CSSPixels border_box_height(BoxSlot const& node)
{
    auto border_box = box_model(node).border_box();
    return content_height(node) + border_box.top + border_box.bottom;
}

bool has_scrollable_overflow(BoxSlot const& node)
{
    if (!node)
        return {};
    auto overflow = Layout::RustFFI::layout_arena_paintable_scrollable_overflow(node.arena(), node.slot());
    return overflow.has_value && overflow.value.has_scrollable_overflow;
}

Optional<CSSPixelRect> scrollable_overflow_rect(BoxSlot const& node)
{
    if (!node)
        return {};
    auto overflow = Layout::RustFFI::layout_arena_paintable_scrollable_overflow(node.arena(), node.slot());
    if (!overflow.has_value)
        return {};
    return overflow.value.rect;
}

// Whether the style the row holds leaves it visible.
static bool style_is_visible(BoxSlot const& style_source)
{
    auto const* inherited_box = style_source.style_group<CSS::ComputedValues::InheritedBoxValues>();
    auto const* effects = style_source.style_group<CSS::ComputedValues::EffectsValues>();
    if (!inherited_box || !effects)
        return false;
    return static_cast<CSS::Visibility>(inherited_box->visibility) == CSS::Visibility::Visible && effects->opacity != 0;
}

static Color caret_color(BoxSlot const& style_source)
{
    auto const* ui = style_source.style_group<CSS::ComputedValues::InheritedUIValues>();
    return ui ? ui->caret_color_value() : Color {};
}

static CSS::Display display_of_row(BoxSlot const& row)
{
    auto const* box_values = row.style_group<CSS::ComputedValues::BoxValues>();
    return box_values ? CSS::display_from_ffi_display(box_values->display) : CSS::Display {};
}

bool is_visible(BoxSlot const& node)
{
    if (!has_committed_box(node))
        return false;
    return style_is_visible(node);
}

bool visible_for_hit_testing(BoxSlot const& node)
{
    if (!has_committed_box(node))
        return false;
    if (auto dom_node = node.dom_node(); dom_node && dom_node->is_inert())
        return false;
    auto const* ui = node.style_group<CSS::ComputedValues::InheritedUIValues>();
    return ui && ui->pointer_events_value() != CSS::PointerEvents::None;
}

bool has_stacking_context(BoxSlot const& node)
{
    return committed_row(node).establishes_stacking_context;
}

CSS::Display display(BoxSlot const& node)
{
    if (!has_committed_box(node))
        return {};
    auto const* box_values = node.style_group<CSS::ComputedValues::BoxValues>();
    return box_values ? CSS::display_from_ffi_display(box_values->display) : CSS::Display {};
}

bool is_positioned(BoxSlot const& node)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_paintable_is_positioned(node.arena(), node.slot());
}

bool is_navigable_container_viewport_paintable(BoxSlot const& node)
{
    return has_committed_box(node) && node.kind() == Layout::RustFFI::NodeKind::NavigableContainerViewport;
}

bool is_viewport_paintable(BoxSlot const& node)
{
    return has_committed_box(node) && node.kind() == Layout::RustFFI::NodeKind::Viewport;
}

bool is_paintable_with_lines(BoxSlot const& node)
{
    if (!node)
        return {};
    if (!has_committed_box(node))
        return false;
    switch (node.kind()) {
    case Layout::RustFFI::NodeKind::Viewport:
    case Layout::RustFFI::NodeKind::BlockContainer:
    case Layout::RustFFI::NodeKind::LegendBox:
    case Layout::RustFFI::NodeKind::TableWrapper:
    case Layout::RustFFI::NodeKind::TextAreaBox:
    case Layout::RustFFI::NodeKind::TextInputBox:
    case Layout::RustFFI::NodeKind::RangeInputBox:
    case Layout::RustFFI::NodeKind::ListItemMarkerBox:
    case Layout::RustFFI::NodeKind::SVGForeignObjectBox:
        return true;
    case Layout::RustFFI::NodeKind::ListItemBox:
        return !Layout::RustFFI::layout_arena_node_is_fragmented_inline(node.arena(), node.slot());
    default:
        return false;
    }
}

bool is_inline_paintable(BoxSlot const& node)
{
    if (!node)
        return {};
    return has_committed_box(node) && Layout::RustFFI::layout_arena_node_is_fragmented_inline(node.arena(), node.slot());
}

bool is_svg_svg_paintable(BoxSlot const& node)
{
    return has_committed_box(node) && node.kind() == Layout::RustFFI::NodeKind::SVGSVGBox;
}

bool has_accumulated_visual_context(BoxSlot const& node)
{
    return committed_row(node).has_accumulated_visual_context;
}

Compositing::ContextRef accumulated_visual_context(BoxSlot const& node)
{
    return committed_row(node).accumulated_visual_context;
}

Compositing::ContextRef accumulated_visual_context_for_descendants(BoxSlot const& node)
{
    return committed_row(node).accumulated_visual_context_for_descendants;
}

Compositing::SpatialNodeIndex enclosing_scroll_node_index(BoxSlot const& node)
{
    auto row = committed_row(node);
    return row.is_populated ? row.enclosing_scroll_node_index : Compositing::VISUAL_VIEWPORT_NODE_INDEX;
}

Compositing::SpatialNodeIndex own_scroll_node_index(BoxSlot const& node)
{
    auto row = committed_row(node);
    return row.is_populated ? row.own_scroll_node_index : Compositing::VISUAL_VIEWPORT_NODE_INDEX;
}

Gfx::Path const* committed_svg_path(BoxSlot const& node)
{
    if (!node)
        return {};
    return static_cast<Gfx::Path const*>(Layout::RustFFI::layout_arena_paintable_computed_svg_path(node.arena(), node.slot()));
}

CSSPixelSize svg_viewport_size(BoxSlot const& node)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_paintable_svg_viewport_size(node.arena(), node.slot());
}

Optional<Gfx::AffineTransform> svg_viewport_transform(BoxSlot const& node)
{
    if (!node)
        return {};
    auto result = Layout::RustFFI::layout_arena_paintable_svg_viewport_transform(node.arena(), node.slot());
    if (!result.has_value)
        return {};
    auto const& transform = result.transform;
    return Gfx::AffineTransform { transform.a, transform.b, transform.c, transform.d, transform.e, transform.f };
}

CSS::RustStyleValueHandle used_value_for_grid_template(BoxSlot const& node, CSS::PropertyID property)
{
    if (!node)
        return {};
    VERIFY(property == CSS::PropertyID::GridTemplateColumns || property == CSS::PropertyID::GridTemplateRows);
    auto* value = Layout::RustFFI::layout_arena_paintable_used_grid_tracks(node.arena(), node.slot(), property == CSS::PropertyID::GridTemplateColumns);
    if (!value)
        return {};
    return CSS::RustStyleValueHandle { static_cast<CSS::StyleValueFFI::StyleValueData const*>(value) };
}

CSSPixelPoint box_type_agnostic_position(BoxSlot const& node)
{
    if (!has_committed_box(node))
        return {};
    if (is_inline_paintable(node)) {
        auto result = Layout::RustFFI::layout_arena_inline_paintable_first_piece_position(node.arena(), node.slot());
        if (result.has_value)
            return { result.x, result.y };
    }
    return absolute_position(node);
}

static bool has_content(BoxSlot const& node)
{
    // Interrupting block-in-inline children produce only placeholder pieces, so any child
    // paintable also counts as content.
    return Layout::RustFFI::layout_arena_inline_paintable_has_content_pieces(node.arena(), node.slot())
        || Layout::RustFFI::layout_arena_paintable_has_child_paintables(node.arena(), node.slot());
}

static CSSPixelRect caret_rect_for_empty_line(BoxSlot const& node, CSSPixelPoint position)
{
    // NB: Match the font-height caret used by text fragments, centered in the empty line's line-height box.
    auto const* font_values = node.style_group<CSS::ComputedValues::FontValues>();
    if (!font_values)
        return { position.x(), position.y(), 1, 0 };
    auto const& font_metrics = font_values->font_list_value().first_available_font().pixel_metrics();
    auto line_height = font_values->line_height_used;
    auto caret_height = min(line_height, CSSPixels::nearest_value_for(font_metrics.ascent + font_metrics.descent));
    return { position.x(), position.y() + (line_height - caret_height) / 2, 1, caret_height };
}

static Optional<Layout::RustFFI::FfiCaretRectResult> caret_at_atomic_child(BoxSlot const& layout_node, size_t offset)
{
    auto node = layout_node.dom_node();
    if (!node)
        return {};
    auto resolve = [&](size_t child_offset) -> Optional<Layout::RustFFI::FfiCaretRectResult> {
        auto const* child = node->child_at_index(child_offset);
        auto child_slot = child ? committed_row_slot(layout_node.document(), DOM::NodeIdentity::of(*child)) : Compositing::RustFFI::NodeSlotId { Compositing::RustFFI::INVALID_NODE_SLOT_INDEX };
        if (child_slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX || !Layout::RustFFI::layout_arena_node_is_atomic_inline(layout_node.arena(), child_slot))
            return {};
        auto result = Layout::RustFFI::layout_arena_atomic_inline_caret_rect_for_position(
            layout_node.arena(), child_slot, child_offset < offset);
        if (!result.found)
            return {};
        return result;
    };
    if (offset > 0) {
        if (auto result = resolve(offset - 1); result.has_value())
            return result;
    }
    return resolve(offset);
}

// Caret rect for a cursor parked on this paintable's DOM node at the given child offset, e.g. on an empty line
// rendered by a <br> child or in an empty editable element.
CSSPixelRect caret_rect_for_child_offset(BoxSlot const& block, size_t offset)
{
    if (!has_committed_box(block))
        return {};
    auto const* font_values = block.style_group<CSS::ComputedValues::FontValues>();
    if (!font_values)
        return {};

    auto content_box = absolute_padding_box_rect(block);
    auto line_height = font_values->line_height_used;
    auto rect = caret_rect_for_empty_line(block, content_box.location());
    auto caret_offset_in_line = rect.y() - content_box.y();

    auto dom_node = block.dom_node();
    if (!dom_node)
        return rect;

    if (auto atomic_caret = caret_at_atomic_child(block, offset); atomic_caret.has_value())
        return atomic_caret->rect;

    // NB: A boundary beside a text child has the same geometry as the corresponding text offset.
    //     Editors can leave the selection on the parent after inserting their first character.
    //     Use the text fragment's position and font metrics instead of the empty-block fallback.
    auto caret_rect_in_text = [&](DOM::Node const* node, size_t text_offset) -> Optional<CSSPixelRect> {
        auto const* text = as_if<DOM::Text>(node);
        auto text_slot = text ? committed_row_slot(block.document(), DOM::NodeIdentity::of(*text)) : Compositing::RustFFI::NodeSlotId { Compositing::RustFFI::INVALID_NODE_SLOT_INDEX };
        if (text_slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
            return {};
        auto result = Layout::RustFFI::layout_arena_text_caret_rect_for_position(
            block.arena(), text_slot, text_offset, true);
        if (result.found)
            return result.rect;
        return {};
    };
    if (offset > 0) {
        auto const* previous_child = dom_node->child_at_index(offset - 1);
        if (auto text_rect = caret_rect_in_text(previous_child, previous_child ? previous_child->length() : 0); text_rect.has_value())
            return *text_rect;
    }
    if (auto text_rect = caret_rect_in_text(dom_node->child_at_index(offset), 0); text_rect.has_value())
        return *text_rect;

    auto* child = dom_node->child_at_index(offset);
    if (!child || !is<HTML::HTMLBRElement>(*child))
        return rect;

    // A caret parked before a <br> sits on the line below the content preceding the <br>. Layout produces no
    // fragments for <br>, so start below the fragments of any preceding content, and add one line height for each
    // empty line rendered by earlier <br>s.
    struct PrecedingContentContext {
        GC::Ref<DOM::Node> child;
        DOM::Document& document;
        Optional<CSSPixels> preceding_content_bottom;
    } preceding_context { const_cast<DOM::Node&>(*child), block.document(), {} };
    Layout::RustFFI::layout_arena_for_each_subtree_fragment_rect(
        block.arena(), block.slot(), &preceding_context,
        [](void* context_pointer, Compositing::RustFFI::NodeSlotId fragment_slot, CSSPixelRect rect) {
            auto& context = *static_cast<PrecedingContentContext*>(context_pointer);
            auto fragment_dom_node = BoxSlot::of(context.document, fragment_slot).dom_node();
            if (!fragment_dom_node || !(context.child->compare_document_position(fragment_dom_node) & DOM::Node::DOCUMENT_POSITION_PRECEDING))
                return;
            auto bottom = rect.bottom();
            if (!context.preceding_content_bottom.has_value() || bottom > *context.preceding_content_bottom)
                context.preceding_content_bottom = bottom;
        });
    auto& preceding_content_bottom = preceding_context.preceding_content_bottom;

    size_t preceding_empty_lines = 0;
    dom_node->for_each_in_subtree_of_type<HTML::HTMLBRElement>([&](auto& br) {
        if (&br == child)
            return TraversalDecision::Break;
        if (br.represents_empty_line())
            ++preceding_empty_lines;
        return TraversalDecision::Continue;
    });

    rect.set_y(preceding_content_bottom.value_or(content_box.y()) + line_height * preceding_empty_lines + caret_offset_in_line);
    return rect;
}

Layout::RustFFI::FfiCaretPaint resolve_document_caret_paint(DOM::Document& document)
{
    Layout::RustFFI::FfiCaretPaint caret {};
    Compositing::RustFFI::NodeSlotId const no_slot { Compositing::RustFFI::INVALID_NODE_SLOT_INDEX };
    caret.kind = Layout::RustFFI::FfiCaretPaintKind::None;
    caret.block = no_slot;
    caret.owner = no_slot;

    auto cursor_position = document.cursor_position();
    if (!cursor_position)
        return caret;
    // The caret paints only while the window has focus and the cursor node is editable (or a mutable text
    // control has focus); every candidate box below has a committed box.
    auto navigable = document.navigable();
    if (!navigable || !navigable->is_focused())
        return caret;
    auto const* cursor_node = cursor_position->node().ptr();
    if (!cursor_node)
        return caret;
    bool cursor_is_editable = false;
    if (auto const* text_control = as_if<HTML::FormAssociatedTextControlElement>(document.focused_area().ptr()); text_control && text_control->text_control_to_html_element().is_mutable())
        cursor_is_editable = true;
    else
        cursor_is_editable = cursor_node->is_editable_or_editing_host();
    if (!cursor_is_editable)
        return caret;

    auto fill = [&](Layout::RustFFI::FfiCaretPaintKind kind, Compositing::RustFFI::NodeSlotId block, Compositing::RustFFI::NodeSlotId owner, CSSPixelRect rect, Color color) {
        caret.kind = kind;
        caret.block = block;
        caret.owner = owner;
        caret.rect = rect;
        caret.color = color;
        caret.blink_cycle_start_time_ns = document.cursor_blink_cycle_start_time_ns();
        caret.should_blink = !HTML::Window::in_test_mode();
    };

    if (auto const* text = as_if<DOM::Text>(cursor_node)) {
        auto text_box = BoxSlot::bound_to(document, DOM::NodeIdentity::of(*text));
        if (!text_box)
            return caret;
        auto* arena = text_box.arena();
        auto result = Layout::RustFFI::layout_arena_text_caret_rect_for_position(
            arena, text_box.slot(), cursor_position->offset(),
            cursor_position->affinity() == TextAffinity::Downstream);
        if (result.found) {
            auto style_source = BoxSlot::of(document, result.style_source);
            if (style_source && style_is_visible(style_source))
                fill(Layout::RustFFI::FfiCaretPaintKind::InBlock, result.owner_paintable, result.nearest_self_painting_inline, result.rect, caret_color(style_source));
            return caret;
        }
        // No fragment holds the position: the caret sits on an empty line of the block that lays the text out.
        for (auto block = text_box.parent(); block; block = block.parent()) {
            if (!has_committed_box(block) || !is_visible(block))
                continue;
            auto empty_line = Layout::RustFFI::layout_arena_paintable_empty_line_caret_rect(
                arena, block.slot(), text_box.slot(), cursor_position->offset());
            if (!empty_line.has_value)
                continue;
            auto style_source = BoxSlot::of(document, empty_line.style_source);
            if (!style_source)
                return caret;
            auto empty_line_rect = empty_line.rect;
            fill(Layout::RustFFI::FfiCaretPaintKind::InBlock, block.slot(), no_slot, CSSPixelRect { empty_line_rect.x(), empty_line_rect.y(), 1, empty_line_rect.height() }, caret_color(style_source));
            return caret;
        }
        return caret;
    }

    // The cursor is parked on an element: its own box paints the caret at the child offset, or, for an
    // empty editable inline, at the box's position.
    auto box = BoxSlot::bound_to(document, DOM::NodeIdentity::of(*cursor_node));
    if (!box || !has_committed_box(box))
        return caret;
    if (is_inline_paintable(box)) {
        if (auto atomic_caret = caret_at_atomic_child(box, cursor_position->offset()); atomic_caret.has_value()) {
            if (style_is_visible(box))
                fill(Layout::RustFFI::FfiCaretPaintKind::InBlock, atomic_caret->owner_paintable, atomic_caret->nearest_self_painting_inline, atomic_caret->rect, caret_color(box));
            return caret;
        }
        if (has_content(box))
            return caret;
        auto position = box_type_agnostic_position(box);
        fill(Layout::RustFFI::FfiCaretPaintKind::EmptyInline, box.slot(), no_slot, caret_rect_for_empty_line(box, position), caret_color(box));
        return caret;
    }
    if (!is_visible(box))
        return caret;
    fill(Layout::RustFFI::FfiCaretPaintKind::InBlock, box.slot(), no_slot, caret_rect_for_child_offset(box, cursor_position->offset()), caret_color(box));
    return caret;
}

Layout::RustFFI::FfiFocusedTextControlSelection resolve_focused_text_control_selection(DOM::Document const& document)
{
    Layout::RustFFI::FfiFocusedTextControlSelection selection {};
    auto const* text_control = as_if<HTML::FormAssociatedTextControlElement>(document.focused_area().ptr());
    if (!text_control)
        return selection;
    auto text_node = text_control->form_associated_element_to_text_node();
    if (!text_node)
        return selection;
    auto selection_start = text_control->selection_start();
    auto selection_end = text_control->selection_end();
    if (selection_start == selection_end)
        return selection;
    auto text_slot = committed_row_slot(document, DOM::NodeIdentity::of(*text_node));
    if (text_slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return selection;
    selection.text_node = text_slot;
    selection.start = selection_start;
    selection.end = selection_end;
    return selection;
}

Layout::RustFFI::FfiFocusedAreaOutline resolve_focused_area_outline(DOM::Document const& document, Vector<u8>& path_bytes)
{
    // https://html.spec.whatwg.org/multipage/interaction.html#focusable-area
    // The shapes of area elements in an image map associated with an img element that is being rendered and is not
    // inert. Focused area elements have no box of their own, so the image whose rendering makes the area's shape a
    // focusable area paints the focus outline along that shape.
    Layout::RustFFI::FfiFocusedAreaOutline outline {};
    auto const* area_element = as_if<HTML::HTMLAreaElement>(document.focused_area().ptr());
    if (!area_element)
        return outline;
    auto const* map_element = area_element->first_ancestor_of_type<HTML::HTMLMapElement>();
    if (!map_element)
        return outline;
    auto image_element = map_element->first_painted_image_with_focusable_shapes();
    if (!image_element)
        return outline;
    auto image_identity = DOM::NodeIdentity::of(*image_element);
    if (!has_committed_box(document, image_identity))
        return outline;
    VERIFY(document.layout_is_up_to_date());
    auto area_computed_values = area_element->computed_style();
    if (!area_computed_values || area_computed_values->outline_style() != CSS::OutlineStyle::Auto)
        return outline;
    auto outline_data = Painting::outline_data(*image_element, *area_computed_values);
    if (!outline_data.has_value())
        return outline;
    auto path = area_element->shape_path(absolute_rect(document, image_identity).size());
    if (!path.has_value())
        return outline;
    path_bytes = path->serialize_to_bytes();
    outline.image = committed_row_slot(document, image_identity);
    outline.path_bytes = path_bytes.data();
    outline.path_byte_count = path_bytes.size();
    outline.color = outline_data->color;
    outline.width = outline_data->width;
    return outline;
}

static Optional<CSS::BorderData> border_data_for_outline(DOM::Element const& element, Color outline_color, CSS::OutlineStyle outline_style, CSSPixels outline_width)
{
    CSS::LineStyle line_style;
    if (outline_style == CSS::OutlineStyle::Auto) {
        line_style = CSS::LineStyle::Solid;
        outline_color = CSS::KeywordStyleValue::create(CSS::Keyword::Accentcolor)->to_color(CSS::ColorResolutionContext::for_element(DOM::AbstractElement { element })).value();
        outline_width = 2;
    } else {
        line_style = CSS::keyword_to_line_style(CSS::to_keyword(outline_style)).value_or(CSS::LineStyle::None);
    }

    if (outline_color.alpha() == 0 || line_style == CSS::LineStyle::None || outline_width == 0)
        return {};

    return CSS::BorderData {
        .color = outline_color,
        .line_style = line_style,
        .width = outline_width,
    };
}

Optional<CSS::BorderData> outline_data(DOM::Element const& element, CSS::ComputedValues const& computed_values)
{
    if (!has_committed_box(element.document(), DOM::NodeIdentity::of(element)))
        return {};

    // The `auto` outline is the UA focus ring; like native controls, it is only shown while the window has focus.
    auto navigable = element.document().navigable();
    if (computed_values.outline_style() == CSS::OutlineStyle::Auto && (!navigable || !navigable->is_focused()))
        return {};

    return border_data_for_outline(element, computed_values.outline_color(), computed_values.outline_style(), computed_values.outline_width());
}

CSSPixelRect transform_reference_box(BoxSlot const& node)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_paintable_transform_reference_box(node.arena(), node.slot());
}

CSSPixelRect transform_rect_to_viewport(BoxSlot const& node, CSSPixelRect const& rect, Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform include_visual_viewport_transform)
{
    auto row = committed_row(node);
    if (!row.is_populated)
        return {};
    auto const& document = node.document();
    if (!document.is_rendered())
        return rect;
    auto pixel_ratio = static_cast<float>(document.page().client().device_pixels_per_css_pixel());
    auto result = document.visual_context_tree().transform_rect_to_viewport(
        row.accumulated_visual_context.spatial, rect.to_type<float>() * pixel_ratio,
        document.scroll_state_snapshot(), include_visual_viewport_transform);
    return (result * (1.f / pixel_ratio)).to_type<CSSPixels>();
}

Optional<CSSPixelPoint> transform_point_to_local(BoxSlot const& node, CSSPixelPoint position)
{
    auto row = committed_row(node);
    if (!row.is_populated)
        return {};
    auto const& document = node.document();
    if (!document.is_rendered())
        return position;
    auto pixel_ratio = static_cast<float>(document.page().client().device_pixels_per_css_pixel());
    auto result = document.visual_context_tree().transform_point_for_hit_test(
        row.accumulated_visual_context, position.to_type<float>() * pixel_ratio,
        document.scroll_state_snapshot());
    if (!result.has_value())
        return {};
    return (*result / pixel_ratio).to_type<CSSPixels>();
}

CSSPixelPoint inverse_transform_point(BoxSlot const& node, CSSPixelPoint position)
{
    auto row = committed_row(node);
    if (!row.is_populated)
        return {};
    auto const& document = node.document();
    if (!document.is_rendered())
        return position;
    auto pixel_ratio = static_cast<float>(document.page().client().device_pixels_per_css_pixel());
    auto result = document.visual_context_tree().inverse_transform_point(row.accumulated_visual_context.spatial, position.to_type<float>() * pixel_ratio);
    return (result / pixel_ratio).to_type<CSSPixels>();
}

CSSPixelPoint transform_to_local_coordinates(BoxSlot const& node, CSSPixelPoint position)
{
    if (!has_committed_box(node))
        return {};
    return transform_point_to_local(node, position).value_or(position);
}

Optional<String> grid_layout_json(BoxSlot const& node, UniqueNodeID container_node_id)
{
    if (!node)
        return {};
    Optional<String> result;
    Layout::RustFFI::layout_arena_paintable_grid_layout_json(node.arena(), node.slot(), container_node_id.value(), &result,
        [](void* context, u8 const* bytes, size_t length) {
            *static_cast<Optional<String>*>(context) = MUST(String::from_utf8(StringView { bytes, length }));
        });
    return result;
}

// The devtools protocol names a flex item by its DOM node's unique id, while the layout row that
// recorded the item names it by its style node.
static i64 devtools_node_id_for_style_node(void* context, u32 style_node)
{
    auto& document = *static_cast<DOM::Document*>(context);
    auto dom_node = DOM::NodeIdentity::of_style_node(CSS::StyleNodeID { style_node }).resolve(document);
    return dom_node ? dom_node->unique_id().value() : -1;
}

Optional<String> flex_layout_json(BoxSlot const& node, UniqueNodeID container_node_id)
{
    if (!node)
        return {};
    Optional<String> result;
    auto& document = const_cast<DOM::Document&>(node.document());
    Layout::RustFFI::layout_arena_paintable_flex_layout_json(
        node.arena(), node.slot(), container_node_id.value(), &result,
        [](void* context, u8 const* bytes, size_t length) {
            *static_cast<Optional<String>*>(context) = MUST(String::from_utf8(StringView { bytes, length }));
        },
        &document, devtools_node_id_for_style_node);
    return result;
}

void push_selection_pseudo_style(DOM::Element const& element)
{
    if (auto* arena = Layout::document_layout_arena_if_created(element.document()))
        Layout::RustFFI::layout_arena_sync_selection_pseudo_style(arena, element.style_node_id().value());
}

class BoxViewRepaintAccess {
public:
    static void set_document_needs_repaint(DOM::Document& document, InvalidateDisplayList should_invalidate_display_list)
    {
        document.set_needs_repaint(Badge<BoxViewRepaintAccess> {}, should_invalidate_display_list);
    }
};

void set_needs_repaint(BoxSlot const& row, InvalidateDisplayList should_invalidate_display_list)
{
    if (!has_committed_box(row))
        return;

    auto identity = row.dom_node_identity();
    if (!identity) {
        // Anonymous rows cannot be resolved by the journal. The layout operation that owns them
        // keeps their slots live while this apply-only path pushes damage.
        apply_repaint_damage(row, should_invalidate_display_list);
        return;
    }
    row.document().invalidation_journal().note_needs_repaint(identity, should_invalidate_display_list);
}

void apply_repaint_damage(BoxSlot const& row, InvalidateDisplayList should_invalidate_display_list)
{
    if (!has_committed_box(row))
        return;

    auto& document = row.document();
    if (should_invalidate_display_list != InvalidateDisplayList::No) {
        Layout::RustFFI::layout_arena_paintable_invalidate_for_repaint(row.arena(), row.slot(), should_invalidate_display_list == InvalidateDisplayList::PaintCommandsAndHitTestList);

        // The root element paints the body's propagated background, so a body repaint must also refresh the
        // root's cached background. Changes to the propagation source are handled during paint preparation.
        if (body_background_is_propagated_to_root(row)) {
            if (auto const* document_element = document.document_element()) {
                if (auto identity = DOM::NodeIdentity::of(*document_element); bound_row_kind(document, identity).has_value())
                    invalidate_paint_cache(document, identity);
            }
        }
    }
    BoxViewRepaintAccess::set_document_needs_repaint(document, should_invalidate_display_list);
}

void repaint_document_after_owner_style_change(DOM::Document& document, InvalidateDisplayList should_invalidate_display_list)
{
    BoxViewRepaintAccess::set_document_needs_repaint(document, should_invalidate_display_list);
}

void apply_text_repaint_damage(BoxSlot const& text, InvalidateDisplayList should_invalidate_display_list)
{
    if (auto containing_block = text.containing_block())
        apply_repaint_damage(containing_block, should_invalidate_display_list);

    if (should_invalidate_display_list != InvalidateDisplayList::No)
        Layout::RustFFI::layout_arena_invalidate_nearest_self_painting_inline_paint_cache(text.arena(), text.slot());
}

void set_needs_repaint(DOM::Document& document, DOM::NodeIdentity identity, InvalidateDisplayList should_invalidate_display_list)
{
    if (!identity || !has_committed_box(document, identity))
        return;
    document.invalidation_journal().note_needs_repaint(identity, should_invalidate_display_list);
}

void set_needs_repaint_in_subtree(DOM::Document& document, DOM::NodeIdentity identity)
{
    if (!identity || !has_committed_box(document, identity))
        return;
    document.invalidation_journal().note_needs_repaint_in_subtree(identity);
}

void set_needs_repaint_in_subtree(BoxSlot const& row)
{
    if (!has_committed_box(row))
        return;
    auto identity = row.dom_node_identity();
    if (!identity) {
        apply_subtree_repaint_damage(row);
        apply_repaint_damage(row, InvalidateDisplayList::PaintCommandsAndHitTestList);
        return;
    }
    row.document().invalidation_journal().note_needs_repaint_in_subtree(identity);
}

void apply_subtree_repaint_damage(BoxSlot const& row)
{
    if (!has_committed_box(row))
        return;
    Layout::RustFFI::layout_arena_paintable_invalidate_subtree_for_repaint(row.arena(), row.slot());
}

void invalidate_paint_cache(DOM::Document const& document, DOM::NodeIdentity identity)
{
    const_cast<DOM::Document&>(document).invalidation_journal().note_paint_cache_invalidation(identity, PaintCacheInvalidation::PaintAndHitTest);
}

void invalidate_propagated_text_decoration_caches(BoxSlot const& row)
{
    if (!row)
        return;
    auto identity = row.dom_node_identity();
    if (!identity) {
        // An anonymous row cannot be resolved from a journal entry.
        apply_paint_cache_invalidation(row, PaintCacheInvalidation::PropagatedTextDecorations);
        return;
    }
    row.document().invalidation_journal().note_paint_cache_invalidation(identity, PaintCacheInvalidation::PropagatedTextDecorations);
}

void apply_paint_cache_invalidation(BoxSlot const& row, PaintCacheInvalidation invalidation)
{
    if (!row)
        return;
    Layout::RustFFI::layout_arena_paintable_invalidate_paint_cache(
        row.arena(), row.slot(), invalidation == PaintCacheInvalidation::PropagatedTextDecorations);
}

static void schedule_structural_visual_context_update(BoxSlot const& row)
{
    if (!has_committed_box(row))
        return;
    auto& document = row.document();
    document.render_inputs_for_write().note_visual_context_box_dirty(row.slot(), Layout::RustFFI::FfiVisualContextBoxDirtyKind::StyleStructuralChange);
    document.set_needs_accumulated_visual_contexts_update(true);
}

void repaint_after_style_change(BoxSlot const& row, CSS::RequiredInvalidationAfterStyleChange const& invalidation)
{
    if (!row)
        return;
    if (invalidation.needs_repaint())
        set_needs_repaint(row, invalidation.invalidates_hit_test_display_list() ? InvalidateDisplayList::PaintCommandsAndHitTestList : InvalidateDisplayList::PaintCommands);
    if (invalidation.repaint_propagated_text_decorations)
        invalidate_propagated_text_decoration_caches(row);
    if (invalidation.needs_stacking_context_tree_rebuild()) {
        schedule_structural_visual_context_update(row);
        if (has_committed_box(row) && display_of_row(row).is_table_inside()) {
            if (auto parent = row.parent(); parent && parent.kind() == Layout::RustFFI::NodeKind::TableWrapper)
                schedule_structural_visual_context_update(parent);
        }
    }
}

Layout::RustFFI::FfiRectToViewportTransform identity_rect_to_viewport_transform()
{
    return {};
}

Layout::RustFFI::FfiRectToViewportTransform rect_to_viewport_transform(DOM::Document const& document, Compositing::AccumulatedVisualContextTree const& visual_context_tree)
{
    if (!document.has_committed_viewport_box())
        return identity_rect_to_viewport_transform();
    auto scroll_offsets = document.scroll_state_snapshot().device_offsets();
    return {
        .visual_context_tree = visual_context_tree.rust_handle(),
        .scroll_offsets = scroll_offsets.data(),
        .scroll_offsets_len = scroll_offsets.size(),
        .device_pixels_per_css_pixel = static_cast<float>(document.page().client().device_pixels_per_css_pixel()),
    };
}

Vector<CSSPixelRect> client_rects(BoxSlot const& node, Layout::RustFFI::FfiRectToViewportTransform const& rect_to_viewport_transform)
{
    if (!node)
        return {};
    Vector<CSSPixelRect> rects;
    Layout::RustFFI::layout_arena_client_rects(
        node.arena(), node.slot(), rect_to_viewport_transform, &rects,
        [](void* context, CSSPixelRect rect) {
            static_cast<Vector<CSSPixelRect>*>(context)->append(rect);
        });
    return rects;
}

CSSPixelRect bounding_client_rect(BoxSlot const& node, Layout::RustFFI::FfiRectToViewportTransform const& rect_to_viewport_transform)
{
    if (!node)
        return {};
    return Layout::RustFFI::layout_arena_bounding_client_rect(node.arena(), node.slot(), rect_to_viewport_transform);
}

CSSPixelPoint cumulative_scroll_compensation(BoxSlot const& node)
{
    auto index = enclosing_scroll_node_index(node);
    if (index == Compositing::VISUAL_VIEWPORT_NODE_INDEX)
        return {};
    auto const& document = node.document();
    if (!document.is_rendered())
        return {};
    auto pixel_ratio = static_cast<float>(document.page().client().device_pixels_per_css_pixel());
    auto device_offset = document.visual_context_tree().cumulative_scroll_chain_offset(index, document.scroll_state_snapshot());
    return { CSSPixels::nearest_value_for(device_offset.x() / pixel_ratio), CSSPixels::nearest_value_for(device_offset.y() / pixel_ratio) };
}

CSSPixelRect absolute_rect(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return absolute_rect(box);
}

CSSPixelRect absolute_padding_box_rect(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return absolute_padding_box_rect(box);
}

CSSPixelRect absolute_border_box_rect(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return absolute_border_box_rect(box);
}

CSSPixelPoint absolute_position(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return absolute_position(box);
}

CSSPixelSize content_size(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return content_size(box);
}

CSSPixels content_width(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return content_width(box);
}

CSSPixels content_height(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return content_height(box);
}

BoxModelMetrics box_model(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return box_model(box);
}

CSSPixels border_box_width(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return border_box_width(box);
}

CSSPixels border_box_height(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return border_box_height(box);
}

bool has_scrollable_overflow(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return has_scrollable_overflow(box);
}

Optional<CSSPixelRect> scrollable_overflow_rect(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return scrollable_overflow_rect(box);
}

bool is_positioned(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return is_positioned(box);
}

CSSPixelSize svg_viewport_size(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return svg_viewport_size(box);
}

Optional<Gfx::AffineTransform> svg_viewport_transform(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return svg_viewport_transform(box);
}

CSSPixelRect transform_reference_box(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return transform_reference_box(box);
}

Vector<CSSPixelRect> client_rects(DOM::Document const& document, DOM::NodeIdentity identity, Layout::RustFFI::FfiRectToViewportTransform const& rect_to_viewport_transform)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return client_rects(box, rect_to_viewport_transform);
}

CSSPixelRect bounding_client_rect(DOM::Document const& document, DOM::NodeIdentity identity, Layout::RustFFI::FfiRectToViewportTransform const& rect_to_viewport_transform)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return bounding_client_rect(box, rect_to_viewport_transform);
}

bool is_visible(DOM::Element const& element)
{
    if (!has_committed_box(element.document(), DOM::NodeIdentity::of(element)))
        return false;
    auto const* inherited_box = element.style_group<CSS::ComputedValues::InheritedBoxValues>();
    auto const* effects = element.style_group<CSS::ComputedValues::EffectsValues>();
    if (!inherited_box || !effects)
        return false;
    return static_cast<CSS::Visibility>(inherited_box->visibility) == CSS::Visibility::Visible && effects->opacity != 0;
}

bool visible_for_hit_testing(DOM::Element const& element)
{
    if (!has_committed_box(element.document(), DOM::NodeIdentity::of(element)))
        return false;
    if (element.is_inert())
        return false;
    auto const* ui = element.style_group<CSS::ComputedValues::InheritedUIValues>();
    return ui && ui->pointer_events_value() != CSS::PointerEvents::None;
}

CSS::Display display(DOM::Element const& element)
{
    if (!has_committed_box(element.document(), DOM::NodeIdentity::of(element)))
        return {};
    auto const* box_values = element.style_group<CSS::ComputedValues::BoxValues>();
    return box_values ? CSS::display_from_ffi_display(box_values->display) : CSS::Display {};
}

bool has_stacking_context(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    return has_stacking_context(box);
}

bool has_accumulated_visual_context(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    return has_accumulated_visual_context(box);
}

Compositing::ContextRef accumulated_visual_context(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    return accumulated_visual_context(box);
}

Compositing::ContextRef accumulated_visual_context_for_descendants(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    return accumulated_visual_context_for_descendants(box);
}

Compositing::SpatialNodeIndex enclosing_scroll_node_index(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    return enclosing_scroll_node_index(box);
}

Compositing::SpatialNodeIndex own_scroll_node_index(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    return own_scroll_node_index(box);
}

CSSPixelRect transform_rect_to_viewport(DOM::Document const& document, DOM::NodeIdentity identity, CSSPixelRect const& rect, Compositing::AccumulatedVisualContextTree::IncludeVisualViewportTransform include_visual_viewport_transform)
{
    auto box = box_slot(document, identity);
    return transform_rect_to_viewport(box, rect, include_visual_viewport_transform);
}

Optional<CSSPixelPoint> transform_point_to_local(DOM::Document const& document, DOM::NodeIdentity identity, CSSPixelPoint position)
{
    auto box = box_slot(document, identity);
    return transform_point_to_local(box, position);
}

CSSPixelPoint inverse_transform_point(DOM::Document const& document, DOM::NodeIdentity identity, CSSPixelPoint position)
{
    auto box = box_slot(document, identity);
    return inverse_transform_point(box, position);
}

CSSPixelPoint cumulative_scroll_compensation(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    return cumulative_scroll_compensation(box);
}

Gfx::Path const* committed_svg_path(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return committed_svg_path(box);
}

CSS::RustStyleValueHandle used_value_for_grid_template(DOM::Document const& document, DOM::NodeIdentity identity, CSS::PropertyID property)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return used_value_for_grid_template(box, property);
}

bool is_navigable_container_viewport_paintable(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return is_navigable_container_viewport_paintable(box);
}

bool is_viewport_paintable(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return is_viewport_paintable(box);
}

bool is_paintable_with_lines(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return is_paintable_with_lines(box);
}

bool is_inline_paintable(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return is_inline_paintable(box);
}

bool is_svg_svg_paintable(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return is_svg_svg_paintable(box);
}

CSSPixelPoint box_type_agnostic_position(DOM::Document const& document, DOM::NodeIdentity identity)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return box_type_agnostic_position(box);
}

CSSPixelPoint transform_to_local_coordinates(DOM::Document const& document, DOM::NodeIdentity identity, CSSPixelPoint position)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return transform_to_local_coordinates(box, position);
}

Optional<String> grid_layout_json(DOM::Document const& document, DOM::NodeIdentity identity, UniqueNodeID container_node_id)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return grid_layout_json(box, container_node_id);
}

Optional<String> flex_layout_json(DOM::Document const& document, DOM::NodeIdentity identity, UniqueNodeID container_node_id)
{
    auto box = box_slot(document, identity);
    if (!box)
        return {};
    return flex_layout_json(box, container_node_id);
}

}
