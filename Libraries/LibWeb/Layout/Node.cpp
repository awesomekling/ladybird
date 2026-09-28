/*
 * Copyright (c) 2018-2023, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2025, Jelle Raaijmakers <jelle@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Demangle.h>
#include <LibWeb/CSS/ComputedStyleWorkingSet.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleInputScope.h>
#include <LibWeb/CSS/StyleValues/AbstractImageStyleValue.h>
#include <LibWeb/CSS/StyleValues/CursorStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageStyleValue.h>
#include <LibWeb/DOM/AbstractElement.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/InvalidationJournal.h>
#include <LibWeb/DOM/Text.h>
#include <LibWeb/Dump.h>
#include <LibWeb/HTML/EventLoop/EventLoop.h>
#include <LibWeb/HTML/EventLoop/FrameScheduler.h>
#include <LibWeb/HTML/HTMLElement.h>
#include <LibWeb/HTML/HTMLHtmlElement.h>
#include <LibWeb/HTML/HTMLTableCellElement.h>
#include <LibWeb/HTML/HTMLTableColElement.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/NavigableContainer.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Layout/TextNode.h>
#include <LibWeb/Layout/Viewport.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/Painting/PaintFacts.h>
#include <LibWeb/Painting/ScrollSnap.h>
#include <LibWeb/Painting/StyleImageObservers.h>
#include <LibWeb/SVG/SVGClipPathElement.h>
#include <LibWeb/SVG/SVGElement.h>
#include <LibWeb/SVG/SVGFilterElement.h>
#include <LibWeb/SVG/SVGGradientElement.h>
#include <LibWeb/SVG/SVGTextContentElement.h>

namespace Web::Layout {

static u8 dom_paint_facts_of(GC::Ptr<DOM::Node const> node)
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

// The StyleNodeID a row bound to this DOM node records: an element's or a text node's.
CSS::StyleNodeID Node::style_node_of(DOM::Node const* node)
{
    if (auto const* element = as_if<DOM::Element>(node))
        return element->style_node_id();
    if (auto const* text = as_if<DOM::Text>(node))
        return text->style_node_id();
    return {};
}

// Every element fact a row is built with is published in the style mirror under the row's
// identity; see `CSS::ElementConstructionFact`. The row reads them there rather than here.
static RustFFI::FfiNodeConstructionFacts build_node_construction_facts(GC::Ptr<DOM::Node> node, RustFFI::NodeKind kind, void* shell)
{
    return {
        .kind = kind,
        .shell = shell,
        .is_anonymous = node == nullptr,
        .dom_paint_facts = dom_paint_facts_of(node),
        .style_node = Node::style_node_of(node.ptr()).value(),
    };
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
    auto identity = dom_node.is_document() ? document.style_node_id() : Node::style_node_of(&dom_node);
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
    auto* arena = document.layout_node_arena_if_created();
    if (!arena) {
        if (!value)
            return;
        arena = &document.layout_node_arena();
    }
    auto identity = Node::style_node_of(&node);
    if (identity.value() != 0)
        RustFFI::layout_arena_set_identity_in_focused_text_control(arena->handle(), identity.value(), value);
}

// What a row built for the element is scrolled to. The element's box is replaced whenever its
// subtree is rebuilt, so the offset is held against the identity that outlives it and a row the
// build stamps reads it there, the way a pseudo-element's box already does. The rows the element
// already has are published to separately, by the caller that has one in hand.
void publish_element_scroll_offset(DOM::Element const& element)
{
    auto& document = const_cast<DOM::Document&>(element.document());
    auto offset = element.scroll_offset({});
    auto* arena = document.layout_node_arena_if_created();
    if (!arena) {
        // Nothing has scrolled anything before a layout tree exists, so there is no offset to
        // forget, as for a pseudo-element's.
        if (offset.is_zero())
            return;
        arena = &document.layout_node_arena();
    }
    if (element.style_node_id().value() != 0)
        RustFFI::layout_arena_set_element_scroll_offset(arena->handle(), element.style_node_id().value(), offset);
}

Node::Node(DOM::Document& document, GC::Ptr<DOM::Node> node, RustFFI::NodeKind kind, AttachToDOMNode attach_to_dom_node)
    : m_arena(document.layout_node_arena())
    , m_slot(m_arena->allocate(build_node_construction_facts(node, kind, this)))
    , m_kind(kind)
{
    publish_own_scroll_offset();
    // The node is in hand here, so this does not have to look one up.
    RustFFI::layout_arena_publish_unique_node_id(m_arena->handle(), m_slot,
        is_viewport() ? document.unique_id().value() : (is<DOM::Element>(node.ptr()) ? node->unique_id().value() : 0));

    if (!node)
        return;
    take_over_rows_of_dom_node(*node, attach_to_dom_node);
}

// What a row built around a DOM node owes the node's other rows, and the node itself. A row the
// build prepared reaches this through its identity instead of through a pointer it was handed.
void Node::take_over_rows_of_dom_node(DOM::Node& node, AttachToDOMNode attach_to_dom_node)
{
    auto* row_already_bound_to_dom_node = node.unsafe_layout_node();
    if (row_already_bound_to_dom_node)
        RustFFI::layout_arena_note_rows_share_dom_node(m_arena->handle(), row_already_bound_to_dom_node->m_slot, m_slot);
    if (attach_to_dom_node == AttachToDOMNode::Yes) {
        if (row_already_bound_to_dom_node)
            row_already_bound_to_dom_node->pin_style_record_for_detachment();
        RustFFI::layout_arena_bind_row(m_arena->handle(), m_slot);
    }
}

// The build stamps a row out of the node's identity and materialises its shell here. Everything
// the DOM-backed constructor read off the node it was handed, this one reaches through the
// identity the row already carries; a row stamped for no node at all is an anonymous box.
Node::Node(DOM::Document& document, BindToPreparedArenaSlot, Compositing::RustFFI::NodeSlotId slot, RustFFI::NodeKind kind)
    : m_arena(document.layout_node_arena())
    , m_slot(slot)
    , m_kind(kind)
{
    RustFFI::layout_arena_attach_shell(m_arena->handle(), m_slot, this);
}

Node::~Node()
{
    VERIFY(m_arena_is_destroying_shell);
}

void Node::delete_arena_owned_shell(Node& node)
{
    node.m_arena_is_destroying_shell = true;
    delete &node;
}

Compositing::RustFFI::NodeSlotId Node::slot_id(Node const* node)
{
    return node ? node->m_slot : Compositing::RustFFI::NodeSlotId_INVALID;
}

StringView Node::class_name() const
{
#define LAYOUT_NODE_KIND_NAME_CASE(kind_name) \
    case RustFFI::NodeKind::kind_name:        \
        return #kind_name##sv;
    switch (kind()) {
        LAYOUT_NODE_KIND_NAME_CASE(AudioBox)
        LAYOUT_NODE_KIND_NAME_CASE(BlockContainer)
        LAYOUT_NODE_KIND_NAME_CASE(Box)
        LAYOUT_NODE_KIND_NAME_CASE(BreakNode)
        LAYOUT_NODE_KIND_NAME_CASE(CanvasBox)
        LAYOUT_NODE_KIND_NAME_CASE(CheckBox)
        LAYOUT_NODE_KIND_NAME_CASE(FieldSetBox)
        LAYOUT_NODE_KIND_NAME_CASE(GeneratedTextNode)
        LAYOUT_NODE_KIND_NAME_CASE(ImageBox)
        LAYOUT_NODE_KIND_NAME_CASE(InlineNode)
        LAYOUT_NODE_KIND_NAME_CASE(LegendBox)
        LAYOUT_NODE_KIND_NAME_CASE(ListItemBox)
        LAYOUT_NODE_KIND_NAME_CASE(ListItemMarkerBox)
        LAYOUT_NODE_KIND_NAME_CASE(NavigableContainerViewport)
        LAYOUT_NODE_KIND_NAME_CASE(Node)
        LAYOUT_NODE_KIND_NAME_CASE(NodeWithStyle)
        LAYOUT_NODE_KIND_NAME_CASE(RadioButton)
        LAYOUT_NODE_KIND_NAME_CASE(RangeInputBox)
        LAYOUT_NODE_KIND_NAME_CASE(ReplacedBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGClipBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGForeignObjectBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGGeometryBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGGraphicsBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGImageBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGMaskBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGPatternBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGSVGBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGTextBox)
        LAYOUT_NODE_KIND_NAME_CASE(SVGTextPathBox)
        LAYOUT_NODE_KIND_NAME_CASE(TableWrapper)
        LAYOUT_NODE_KIND_NAME_CASE(TextAreaBox)
        LAYOUT_NODE_KIND_NAME_CASE(TextInputBox)
        LAYOUT_NODE_KIND_NAME_CASE(TextNode)
        LAYOUT_NODE_KIND_NAME_CASE(VideoBox)
        LAYOUT_NODE_KIND_NAME_CASE(Viewport)
    case RustFFI::NodeKind::Unset:
        break;
    }
#undef LAYOUT_NODE_KIND_NAME_CASE
    VERIFY_NOT_REACHED();
}

void* Node::arena_handle() const
{
    return m_arena->handle();
}

Box const* Node::containing_block() const
{
    return static_cast<Box const*>(containing_block_node_if_live());
}

Box* Node::containing_block()
{
    return static_cast<Box*>(containing_block_node_if_live());
}

void Node::pin_style_record_for_detachment()
{
    if (auto* node_with_style = as_if<NodeWithStyle>(*this))
        node_with_style->pin_style_record_for_cxx_consumers();
}

bool NodeWithStyle::establishes_an_absolute_positioning_containing_block() const
{
    return RustFFI::layout_arena_node_establishes_an_absolute_positioning_containing_block(arena_handle(), Node::slot_id(this));
}

bool NodeWithStyle::establishes_a_fixed_positioning_containing_block() const
{
    return RustFFI::layout_arena_node_establishes_a_fixed_positioning_containing_block(arena_handle(), Node::slot_id(this));
}

GC::Ptr<HTML::LocalNavigable> Node::navigable() const
{
    return document().navigable();
}

Viewport& Node::root()
{
    // NB: Called during layout, which is in progress.
    VERIFY(document().unsafe_layout_node());
    return *document().unsafe_layout_node();
}

bool NodeWithStyle::is_positioned() const
{
    return position() != CSS::Positioning::Static;
}

bool NodeWithStyle::is_fixed_position() const
{
    auto position = this->position();
    return position == CSS::Positioning::Fixed;
}

bool NodeWithStyle::is_sticky_position() const
{
    auto position = this->position();
    return position == CSS::Positioning::Sticky;
}

NodeWithStyle::NodeWithStyle(DOM::Document& document, GC::Ptr<DOM::Node> node, CSS::LayoutStyle style, RustFFI::NodeKind kind)
    : Node(document, node, kind)
{
    adopt_style(document, node, move(style));
}

void NodeWithStyle::adopt_style(DOM::Document& document, GC::Ptr<DOM::Node> node, CSS::LayoutStyle style)
{
    VERIFY(style);
    if (!!style.style_record_identity()) {
        m_style_record_identity = style.style_record_identity();
    } else if (auto* element = as_if<DOM::Element>(node.ptr())) {
        m_style_record_identity = document.style_computer().intern_computed_style_inputs({ *element }, *style.values());
    } else {
        m_style_record_identity = document.style_computer().intern_anonymous_layout_style(*style.values());
    }
    initialize_from_style_record();
    if (!style.style_record_identity())
        RustFFI::layout_arena_adopt_derived_node_style(arena_handle(), slot_id(this), m_style_record_identity.value());
}

NodeWithStyle::NodeWithStyle(DOM::Document& document, BindToPreparedArenaSlot bind, Compositing::RustFFI::NodeSlotId slot, RustFFI::NodeKind kind)
    : Node(document, bind, slot, kind)
{
    m_style_record_identity = CSS::StyleRecordID { RustFFI::layout_arena_node_style_record(arena_handle(), slot) };
    VERIFY(m_style_record_identity);
    // The shell reads its style through the payloads of the record the row owns.
    m_style_payloads = RustFFI::layout_arena_node_style_payloads(arena_handle(), slot);
    VERIFY(m_style_payloads);
}

NodeWithStyle::NodeWithStyle(DOM::Document& document, BindToPreparedArenaSlot bind, Compositing::RustFFI::NodeSlotId slot, RustFFI::NodeKind kind, CSS::LayoutStyle style)
    : Node(document, bind, slot, kind)
{
    adopt_style(document, dom_node(), move(style));
}

// A row stamped for an element already carries the style record the mirror published for it, so
// the shell adopts that record rather than writing one back to the row. What remains is what
// adopting a style tells the rest of the document about it.
void NodeWithStyle::initialize_stamped_style_record()
{
    VERIFY(m_style_record_identity);
    VERIFY(m_style_payloads);
    did_update_style_record();
}

void NodeWithStyle::initialize_from_style_record()
{
    publish_style_record_to_node_data();
    synchronize_table_span_data();
}

bool NodeWithStyle::has_layout_derived_style() const
{
    return RustFFI::layout_arena_node_has_derived_style(arena_handle(), slot_id(this));
}

NodeWithStyle::~NodeWithStyle()
{
    // NB: The arena destroys a shell only after it has freed the shell's row, and freeing the row
    //     took its image observers and released its host-pinned style record. Nothing is left to
    //     release by slot, and asking would reach whichever row holds the slot next.
}

}

namespace Web::Layout {

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

// What a row taking a style record tells the rest of the document.
static void did_update_row_style_record(Painting::BoxSlot const& box, DOM::Node const* dom_node, void const* style_payloads)
{
    auto& document = box.document();
    if (auto const* element = as_if<DOM::Element>(dom_node); element && element->has_style(CSS::PseudoElement::Selection))
        Painting::push_selection_pseudo_style(*element);

    if (NodeWithStyle::style_group_of<CSS::ComputedValues::MiscResetValues>(style_payloads).scroll_snap_type_value().strictness != CSS::ScrollSnapStrictness::None)
        document.set_may_have_scroll_snap_areas();

    // NB: The root element's style can be published before the layout tree gives the document a viewport to snap
    //     with, and is published again once building the layout tree binds this node's style record.
    auto snap_container = scroll_snap_container_of(box, dom_node);
    if (!snap_container)
        return;

    // What the layout tree builds found out about their scroll containers comes before what this style says.
    Painting::take_built_scroll_snap_containers(document);

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

static bool style_record_holds_image_values(CSS::StyleRecordDependencyFlag dependency_flags)
{
    return has_flag(dependency_flags, CSS::StyleRecordDependencyFlag::HoldsImageValues);
}

// Whether moving a row or its shell from one record to the other can change its layout, from the payloads each record
// names: an overlay record borrows other payloads than its base, so carrying one is reason enough.
static bool style_change_affects_layout(CSS::StyleRecordID old_style_record, void const* old_style_payloads, CSS::PublishedStyleRecord const& new_style_record)
{
    return !old_style_record
        || !old_style_payloads
        || CSS::PublishedStyleRecord::identity_is_animation_overlay(old_style_record)
        || new_style_record.is_animation_overlay()
        || CSS::ComputedValues::layout_affecting_group_payloads_differ(static_cast<void const* const*>(old_style_payloads), new_style_record.view().payloads);
}

void NodeWithStyle::apply_style(Row const& row, CSS::PublishedStyleRecord const& style_record)
{
    auto& document = row.document();
    auto const* dom_node = row.dom_node_identity().resolve(document).ptr();
    if (row.shell_if_made()) {
        as<NodeWithStyle>(row.shell()).apply_style(style_record);
        return;
    }

    // What apply_style() does to the row, with no shell to keep a mirror of it. A shell made later is made from the row.
    auto* old_image_observers = RustFFI::layout_arena_install_row_style(row.arena_handle(), row.slot(), style_record.identity().value());
    auto box = Painting::BoxSlot::of(document, row.slot());
    did_update_row_style_record(box, dom_node, style_record.payloads());
    delete static_cast<Painting::StyleImageObserverSet*>(old_image_observers);
    attach_style_resources_to_box(box);
}

static Row row_of_box(Painting::BoxSlot const& box)
{
    auto* arena = box ? box.document().layout_node_arena_if_created() : nullptr;
    return arena ? arena->row_if_live(box.slot()) : Row {};
}

void apply_style_to_box(Painting::BoxSlot const& box, CSS::PublishedStyleRecord const& style_record)
{
    if (auto row = row_of_box(box); row && !row.is_text())
        NodeWithStyle::apply_style(row, style_record);
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
    if (!style_record_holds_image_values(dependency_flags)) {
        Painting::replace_style_image_observers(document, slot, nullptr);
        // The row keeps nothing a later attach would have to take away, which is what lets the
        // tree build skip asking for one at all.
        RustFFI::layout_arena_note_style_image_resources_attached(arena, slot, false);
        Painting::push_paint_facts_after_style_attach(box, dom_node.ptr(), Painting::StyleHoldsImageValues::No);
        return;
    }

    auto observers = make<Painting::StyleImageObserverSet>();
    observers->background_layer_data = NodeWithStyle::style_group_of<CSS::ComputedValues::BackgroundValues>(style_payloads).background_layers_value();
    observers->mask_layer_data = NodeWithStyle::style_group_of<CSS::ComputedValues::MaskValues>(style_payloads).mask_layers_value();
    observers->border_image = NodeWithStyle::style_group_of<CSS::ComputedValues::BorderValues>(style_payloads).border_image_value();
    auto cursors = NodeWithStyle::style_group_of<CSS::ComputedValues::InheritedUIValues>(style_payloads).cursor_span();
    RefPtr<CSS::AbstractImageStyleValue const> list_style_image = NodeWithStyle::style_group_of<CSS::ComputedValues::InheritedListValues>(style_payloads).list_style_image_value();

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

void make_host_mirror_of_box(Painting::BoxSlot const& box)
{
    if (auto row = row_of_box(box))
        (void)row.shell();
}

void NodeWithStyle::apply_style(CSS::PublishedStyleRecord const& style_record)
{
    auto const style_record_identity = style_record.identity();
    // A flight installed the record over the row ahead of the host, with what a style change over the row leaves in the
    // arena: the host only takes it into its own mirror of the row. A shell first asked for since then was made with
    // the record already.
    // The pin holds the record the node names. An animation-overlay record the engine no longer
    // assigns lives by that pin alone, so a node that keeps its record keeps the pin.
    if (style_record_identity != m_style_record_identity)
        release_pinned_style_record();
    // Taking the adoption hands the host a pin of its own over a record that lives by the adoption's
    // pin alone, so this comes after the node let go of its old record's pin.
    bool const installed_ahead = RustFFI::layout_arena_take_animation_adoption(arena_handle(), slot_id(this), style_record_identity.value());
    m_background_layers.clear();
    m_mask_layers.clear();
    m_border_image.clear();
    m_list_style_type.clear();
    m_list_style_image.clear();
    m_style_record_identity = style_record_identity;
    if (installed_ahead) {
        m_style_payloads = RustFFI::layout_arena_node_style_payloads(arena_handle(), slot_id(this));
        VERIFY(m_style_payloads);
        did_update_style_record();
    } else {
        publish_style_record_to_node_data();
        set_flag(RustFFI::HostNodeFlag::HasAnimatedOpacityOrTransform, false);
        // A style change can introduce the properties that make a node carry replaced-content facts,
        // such as size containment arriving on a kept layout node.
        RustFFI::layout_arena_reinherit_anonymous_descendants(arena_handle(), slot_id(this));
    }
    attach_style_resources();
    // A pseudo layout node can outlive replacement of the DOM pseudo's record until the layout
    // tree is rebuilt. Root its record across that gap, including metadata-only style changes that
    // keep the existing layout node.
    if (is_generated_for_pseudo_element())
        pin_style_record_for_cxx_consumers();
}

void NodeWithStyle::attach_style_resources()
{
    attach_style_resources_to_box(Painting::BoxSlot::of(document(), slot_id(this)));
}

CSS::StyleScope const& NodeWithStyle::style_scope() const
{
    if (auto const* dom_node = this->dom_node())
        return dom_node->style_scope();

    if (is_generated_for_pseudo_element())
        return pseudo_element_generator()->style_scope();

    if (auto const* parent = this->parent())
        return parent->style_scope();

    return document().style_scope();
}

void NodeWithStyle::refresh_style_from_arena(CSS::StyleRecordID record, void const* payloads, bool should_attach_resources)
{
    release_pinned_style_record();
    m_style_record_identity = record;
    m_style_payloads = payloads;
    m_background_layers.clear();
    m_mask_layers.clear();
    m_border_image.clear();
    m_list_style_type.clear();
    m_list_style_image.clear();
    did_update_style_record();
    if (should_attach_resources)
        attach_style_resources();
}

String Node::debug_description() const
{
    StringBuilder builder;
    builder.append(class_name());
    if (dom_node()) {
        builder.appendff("<{}>", dom_node()->node_name());
        if (dom_node()->is_element()) {
            auto& element = static_cast<DOM::Element const&>(*dom_node());
            if (element.id().has_value())
                builder.appendff("#{}", element.id().value());
            for (auto const& class_name : element.class_names())
                builder.appendff(".{}", class_name);
        }
    } else {
        builder.append("(anonymous)"sv);
    }
    return MUST(builder.to_string());
}

bool NodeWithStyle::is_inline_block() const
{
    auto display = this->display();
    return display.is_inline_outside() && display.is_flow_root_inside();
}

bool Node::is_atomic_inline() const
{
    return RustFFI::layout_arena_node_is_atomic_inline(arena_handle(), slot_id(this));
}

// https://drafts.csswg.org/css-transforms-1/#transformable-element
// The used transform of an SVG element in its own user space, for bounding box computation:
// style transforms in property-application order plus the element's additional transform, without
// transform-origin conjugation. Percentages resolve against an empty reference box because the
// box is not available at layout time, so such transforms under-report the bounding box.
Gfx::AffineTransform NodeWithStyle::used_svg_element_transform() const
{
    auto matrix = Gfx::FloatMatrix4x4::identity();
    for_each_resolved_transform([&](auto const& transform) {
        matrix = matrix * transform.to_matrix({}, {});
    });
    auto transform = Gfx::extract_2d_affine_transform(matrix);
    if (auto const* graphics_element = as_if<SVG::SVGGraphicsElement>(dom_node()))
        transform.multiply(graphics_element->additional_element_transform());
    return transform;
}

void NodeWithStyle::set_computed_values(NonnullRefPtr<CSS::ComputedValues const> computed_values)
{
    CSS::StyleRecordID record;
    if (is_generated_for_pseudo_element())
        record = document().style_computer().intern_computed_style_inputs({ *pseudo_element_generator(), generated_for_pseudo_element() }, *computed_values);
    else if (auto* element = as_if<DOM::Element>(dom_node()))
        record = document().style_computer().intern_computed_style_inputs({ *element }, *computed_values);
    else
        record = document().style_computer().intern_anonymous_layout_style(*computed_values);
    RustFFI::layout_arena_adopt_derived_node_style(arena_handle(), slot_id(this), record.value());
}

void NodeWithStyle::set_style_record(CSS::PublishedStyleRecord const* style_record)
{
    // A detached or layout-derived record is independent of its DOM target's record. A
    // rendering consequence replaces and re-derives it explicitly through apply_style().
    if (has_layout_derived_style())
        return;
    // A box has style for as long as it lives: a record taken away from its DOM target leaves the box the one it has.
    if (!style_record)
        return;
    auto const style_record_identity = style_record->identity();
    if (m_style_record_identity == style_record_identity) {
        publish_style_record_to_node_data();
        return;
    }

    bool should_repin_style_record = RustFFI::layout_arena_node_style_record_pinned_by_host(arena_handle(), slot_id(this)) != 0;
    // Both answers come from the records themselves, so neither side needs a ComputedValues built for it.
    bool changes_layout_affecting_style = style_change_affects_layout(m_style_record_identity, m_style_payloads, *style_record);

    release_pinned_style_record();
    // An animation sample installed the record over the row ahead of the host, with the caches and marks a
    // style change over the row leaves: the host only takes it into its own mirror of the row. The adoption's
    // pin held the record through the reads above; one the engine no longer assigns now has the host's pin.
    bool installed_ahead = RustFFI::layout_arena_take_animation_adoption(arena_handle(), slot_id(this), style_record_identity.value());
    m_background_layers.clear();
    m_mask_layers.clear();
    m_border_image.clear();
    m_list_style_type.clear();
    m_list_style_image.clear();
    m_style_record_identity = style_record_identity;
    if (installed_ahead) {
        m_style_payloads = RustFFI::layout_arena_node_style_payloads(arena_handle(), slot_id(this));
        VERIFY(m_style_payloads);
        did_update_style_record();
    } else {
        publish_style_record_to_node_data();
    }
    if (should_repin_style_record)
        pin_style_record_for_cxx_consumers();

    if (changes_layout_affecting_style && !installed_ahead)
        document().render_inputs_for_write().reset_intrinsic_size_caches_of_self_and_ancestors(slot_id(this));
}

void NodeWithStyle::set_style_record(Row const& row, CSS::PublishedStyleRecord const* style_record)
{
    // A box has style for as long as it lives (see set_style_record() above).
    if (!style_record)
        return;
    if (row.shell_if_made()) {
        as<NodeWithStyle>(row.shell()).set_style_record(style_record);
        return;
    }
    auto* arena = row.arena_handle();
    auto slot = row.slot();
    auto const row_style_record = RustFFI::layout_arena_row_style_record(arena, slot);
    if (row_style_record.derived)
        return;
    auto& document = row.document();
    auto const* dom_node = row.dom_node_identity().resolve(document).ptr();

    // What set_style_record() does to the row, with no shell to keep a mirror of it. A record installed ahead of the
    // host is the row's already, where the shell's mirror still names the old one, so the row takes its adoption all
    // the same.
    auto const old_style_record_identity = CSS::StyleRecordID { row_style_record.record };
    bool changes_layout_affecting_style = false;
    if (old_style_record_identity != style_record->identity())
        changes_layout_affecting_style = style_change_affects_layout(old_style_record_identity, RustFFI::layout_arena_node_style_payloads(arena, slot), *style_record);
    RustFFI::layout_arena_replace_row_style_record(arena, slot, style_record->identity().value(), changes_layout_affecting_style);
    did_update_row_style_record(Painting::BoxSlot::of(document, slot), dom_node, style_record->payloads());
}

void NodeWithStyle::pin_style_record_for_cxx_consumers()
{
    VERIFY(m_style_record_identity);
    RustFFI::layout_arena_pin_node_style_record_for_host(arena_handle(), slot_id(this), m_style_record_identity.value());
}

void NodeWithStyle::release_pinned_style_record()
{
    RustFFI::layout_arena_release_node_style_record_pin_for_host(arena_handle(), slot_id(this));
}

void NodeWithStyle::publish_style_record_to_node_data()
{
    RustFFI::layout_arena_set_node_style(arena_handle(), slot_id(this), m_style_record_identity.value());
    // The shell reads the payloads of the record the row now owns.
    m_style_payloads = RustFFI::layout_arena_node_style_payloads(arena_handle(), slot_id(this));
    VERIFY(m_style_payloads);
    did_update_style_record();
}

void NodeWithStyle::did_update_style_record()
{
    did_update_row_style_record(Painting::BoxSlot::of(document(), slot_id(this)), dom_node(), m_style_payloads);
}

namespace {

struct TableSpans {
    u16 column_span { 1 };
    u16 row_span { 1 };
    u32 raw_column_span { 1 };
};

}

static TableSpans table_spans_of(DOM::Node const* node)
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

bool NodeWithStyle::synchronize_table_span_data()
{
    auto spans = table_spans_of(dom_node());
    return RustFFI::layout_arena_set_table_spans(arena_handle(), slot_id(this), spans.column_span, spans.row_span, spans.raw_column_span);
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

void NodeWithStyle::set_display(CSS::Display display)
{
    RustFFI::layout_arena_set_layout_display(arena_handle(), slot_id(this), bit_cast<u32>(display));
}

bool overflow_value_makes_box_a_scroll_container(CSS::Overflow overflow)
{
    return Painting::overflow_value_makes_box_a_scroll_container(overflow);
}

bool NodeWithStyle::is_scroll_container() const
{
    // NOTE: This isn't in the spec, but we want the viewport to behave like a scroll container.
    if (is_viewport())
        return true;

    return overflow_value_makes_box_a_scroll_container(overflow_x())
        || overflow_value_makes_box_a_scroll_container(overflow_y());
}

DOM::Node const* Node::dom_node() const
{
    return const_cast<Node*>(this)->dom_node();
}

DOM::Node* Node::dom_node()
{
    auto* document = m_arena->document();
    if (!document)
        return nullptr;
    return dom_node_identity().resolve(*document).ptr();
}

DOM::NodeIdentity Node::dom_node_identity() const
{
    if (is_anonymous())
        return {};
    // The document has no StyleNodeID; its row is the viewport.
    if (m_kind == RustFFI::NodeKind::Viewport)
        return DOM::NodeIdentity::of_document();
    // A row kept after its node was removed has a StyleNodeID of 0 and names nothing.
    return DOM::NodeIdentity::of_style_node(style_node_id());
}

GC::Ptr<DOM::Element const> Node::pseudo_element_generator() const
{
    return const_cast<Node*>(this)->pseudo_element_generator();
}

GC::Ptr<DOM::Element> Node::pseudo_element_generator()
{
    auto* document = m_arena->document();
    if (!document)
        return nullptr;
    return as_if<DOM::Element>(pseudo_element_generator_identity().resolve(*document).ptr());
}

DOM::NodeIdentity Node::pseudo_element_generator_identity() const
{
    VERIFY(is_generated_for_pseudo_element());
    // A stale row's StyleNodeID is 0 once its generator disconnects, so it names nothing.
    return DOM::NodeIdentity::of_style_node(style_node_id());
}

CSS::StyleNodeID Node::style_node_id() const
{
    return RustFFI::layout_arena_node_style_node(m_arena->handle(), m_slot);
}

// An element's box holds the element's scroll offset. Everything generated for a pseudo-element
// names it as generator, but only the pseudo-element's own box is what scrolls, so only that box
// holds the pseudo-element's offset; the generated content inside it holds none.
bool Node::dom_target_stores_scroll_offset() const
{
    if (auto pseudo_element = generated_for_pseudo_element(); pseudo_element.has_value()) {
        // A compositor scroll can reach a removed generator's box before the layout tree drops it.
        auto generator = pseudo_element_generator();
        if (!generator)
            return false;
        auto synthetic_pseudo_element = generator->get_synthetic_pseudo_element(*pseudo_element);
        return synthetic_pseudo_element.has_value()
            && synthetic_pseudo_element->unsafe_layout_node() == this
            && !synthetic_pseudo_element->scroll_offset().is_zero();
    }
    if (auto const* element = as_if<DOM::Element>(dom_node()))
        return !element->scroll_offset({}).is_zero();
    return false;
}

// The same targets, plus the viewport: the navigable stores the viewport's offset, and the
// viewport's box is where the render side looks for it. The flag above deliberately does not cover
// the viewport, whose box the arena knows about without being told.
CSSPixelPoint Node::dom_target_scroll_offset() const
{
    if (is_viewport()) {
        auto navigable = document().navigable();
        return navigable ? navigable->viewport_scroll_offset() : CSSPixelPoint {};
    }
    if (auto pseudo_element = generated_for_pseudo_element(); pseudo_element.has_value()) {
        auto generator = pseudo_element_generator();
        if (!generator)
            return {};
        auto synthetic_pseudo_element = generator->get_synthetic_pseudo_element(*pseudo_element);
        if (!synthetic_pseudo_element.has_value() || synthetic_pseudo_element->unsafe_layout_node() != this)
            return {};
        return synthetic_pseudo_element->scroll_offset();
    }
    if (auto const* element = as_if<DOM::Element>(dom_node()))
        return element->scroll_offset({});
    return {};
}

void Node::publish_to_every_row_built_for_dom_node(void (Node::*publish)())
{
    struct Publication {
        void (Node::*publish)();
    } publication { publish };
    RustFFI::layout_arena_for_each_row_built_for_same_node(m_arena->handle(), m_slot, &publication,
        [](void* context, void* shell) {
            auto* row = static_cast<Node*>(shell);
            (row->*static_cast<Publication*>(context)->publish)();
        });
}

void Node::publish_scroll_offset()
{
    publish_to_every_row_built_for_dom_node(&Node::publish_own_scroll_offset);
}

void Node::publish_own_scroll_offset()
{
    RustFFI::layout_arena_publish_scroll_offset(m_arena->handle(), m_slot, dom_target_scroll_offset(), dom_target_stores_scroll_offset());
}

// The same three answers the render side used to ask the document for, in the same order: the
// viewport's box names the document, a pseudo-element's box names its generator, and every other
// box names the element it belongs to. Anything else - an anonymous box, a text node's row - names
// nothing.
i64 Node::dom_target_unique_node_id() const
{
    if (is_viewport())
        return document().unique_id().value();
    if (generated_for_pseudo_element().has_value()) {
        auto generator = pseudo_element_generator();
        return generator ? generator->unique_id().value() : 0;
    }
    if (auto const* element = as_if<DOM::Element>(dom_node()))
        return element->unique_id().value();
    return 0;
}

DOM::Document& Node::document()
{
    VERIFY(m_arena->document());
    return *m_arena->document();
}

DOM::Document const& Node::document() const
{
    VERIFY(m_arena->document());
    return *m_arena->document();
}

// https://drafts.csswg.org/css-ui/#propdef-user-select
CSS::UserSelect Node::user_select_used_value() const
{
    if (!has_style_or_parent_with_style())
        return CSS::UserSelect::None;

    if (!is_generated_for_pseudo_element()) {
        if (auto const* node = dom_node())
            return node->user_select_used_value();
    }

    auto const* style_source = as_if<NodeWithStyle>(*this);
    if (!style_source)
        style_source = parent();
    auto computed_value = style_source->user_select();
    if (computed_value != CSS::UserSelect::Auto)
        return computed_value;

    if (is_generated_for_before_pseudo_element() || is_generated_for_after_pseudo_element())
        return CSS::UserSelect::None;

    if (auto parent_node = parent())
        return parent_node->user_select_used_value();

    return CSS::UserSelect::Text;
}

}

extern "C" WEB_API void ladybird_layout_node_shell_destroy(void* shell)
{
    Web::Layout::Node::delete_arena_owned_shell(*static_cast<Web::Layout::Node*>(shell));
}
