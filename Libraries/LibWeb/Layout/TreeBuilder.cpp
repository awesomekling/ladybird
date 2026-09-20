/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2022-2026, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2022, MacDue <macdue@dueutil.tech>
 * Copyright (c) 2025, Jelle Raaijmakers <jelle@ladybird.org>
 * Copyright (c) 2025, Aziz B. Yesilyurt <abyesilyurt@gmail.com>
 * Copyright (c) 2025, Manuel Zahariev <manuel@duck.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/CharacterTypes.h>
#include <AK/Optional.h>
#include <AK/OwnPtr.h>
#include <AK/Utf16String.h>
#include <LibGfx/DecodedImageFrame.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/CounterStyle.h>
#include <LibWeb/CSS/CountersSet.h>
#include <LibWeb/CSS/Enums.h>
#include <LibWeb/CSS/GeneratedContent.h>
#include <LibWeb/CSS/PseudoElement.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEngineInput.h>
#include <LibWeb/CSS/StyleInvalidation.h>
#include <LibWeb/CSS/StyleValues/ContentStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleValue.h>
#include <LibWeb/CSS/StyleValues/DisplayStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageStyleValue.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/ParentNode.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/DOM/Text.h>
#include <LibWeb/Dump.h>
#include <LibWeb/HTML/HTMLInputElement.h>
#include <LibWeb/Layout/Box.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Layout/TextNode.h>
#include <LibWeb/Layout/TreeBuilder.h>
#include <LibWeb/Layout/TreeBuilderRustFFI.h>
#include <LibWeb/Layout/Viewport.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/SVG/SVGClipPathElement.h>
#include <LibWeb/SVG/SVGMaskElement.h>
#include <LibWeb/SVG/SVGPatternElement.h>

namespace Web::Layout {

class LayoutTreeBuildBridge {
public:
    ~LayoutTreeBuildBridge();

    RustFFI::FfiLayoutTreeBuildOutcome build(DOM::Node&);

    static void detach_top_layer_element_layout_subtree(DOM::Element&);

private:
    static bool svg_resource_box_survives(DOM::Document&, RustFFI::NodeSlotId, u32 cleared_subtree_root);

    RustFFI::FfiDomTreeBuilderCallbacks make_ffi_dom_tree_builder_callbacks();
    RustFFI::FfiPseudoTreeBuilderCallbacks make_ffi_pseudo_tree_builder_callbacks();

    static Box& create_list_item_marker(Box& list_box, CSS::LayoutStyle marker_style);
    static RustFFI::FfiFirstLetterNodes create_first_letter_nodes(DOM::Element&, RustFFI::FfiFirstLetterTarget);

    void pin_style_record_for_build(CSS::StyleRecordID);

    GC::Ptr<DOM::Document> m_document;
    // Every style record a box is built from, held for the whole build. Letting go of a record
    // the build has stopped looking at buys nothing before the build ends, and holding them all
    // in one place is what lets a visit carry no C++ frame of its own.
    Vector<CSS::StyleRecordID> m_pinned_style_records;
};

void LayoutTreeBuilderAccess::set_synthetic_pseudo_element_node(DOM::Element& element, CSS::PseudoElement pseudo_element, Layout::NodeWithStyle* layout_node)
{
    element.set_synthetic_pseudo_element_node({}, pseudo_element, layout_node);
}

static void update_style_if_needed_for_layout_tree_bypass_path(DOM::Element&);

class GeneratedContentImageProvider final
    : public ImageProvider {
public:
    AK_ALLOC_WITH_KMALLOC;

    virtual ~GeneratedContentImageProvider() override = default;

    virtual void layout_node_was_detached() const override
    {
        m_image_client = nullptr;
        m_layout_node = nullptr;
    }

    static NonnullOwnPtr<GeneratedContentImageProvider> create(DOM::Document& document, NonnullRefPtr<CSS::AbstractImageStyleValue> image)
    {
        return adopt_own(*new GeneratedContentImageProvider(document, move(image)));
    }

    void set_layout_node(Layout::Node& layout_node)
    {
        m_layout_node = layout_node;
    }

    virtual GC::Ptr<HTML::DecodedImageData> decoded_image_data() const override
    {
        if (!m_image_client)
            return nullptr;
        return m_image_client->decoded_image_data();
    }

    virtual Optional<CSSPixels> intrinsic_width() const override { return natural_size().width; }
    virtual Optional<CSSPixels> intrinsic_height() const override { return natural_size().height; }
    virtual Optional<CSSPixelFraction> intrinsic_aspect_ratio() const override { return natural_size().aspect_ratio; }
    virtual Layout::Node const* image_provider_layout_node() const override { return m_layout_node.ptr(); }

private:
    class ImageClient final : public CSS::ImageStyleValue::Client {
    public:
        AK_ALLOC_WITH_KMALLOC;

        ImageClient(GeneratedContentImageProvider const& owner, DOM::Document& document, CSS::ImageStyleValue const& image)
            : CSS::ImageStyleValue::Client(document, image)
            , m_owner(owner)
        {
        }

        virtual ~ImageClient() override
        {
            image_style_value_finalize();
        }

        virtual void image_style_value_did_update(CSS::ImageStyleValue&) override
        {
            if (!m_owner.m_layout_node)
                return;
            m_owner.image_provider_contents_changed();
            m_owner.m_layout_node->set_needs_layout_update(DOM::SetNeedsLayoutReason::GeneratedContentImageFinishedLoading);
        }

    private:
        GeneratedContentImageProvider const& m_owner;
    };

    GeneratedContentImageProvider(DOM::Document& document, NonnullRefPtr<CSS::AbstractImageStyleValue> image)
        : m_image(move(image))
    {
        if (auto const* image = m_image->selected_image_style_value())
            m_image_client = make<ImageClient>(*this, document, *image);
    }

    CSS::SizeWithAspectRatio natural_size() const
    {
        auto decoded_image_data = this->decoded_image_data();
        if (!decoded_image_data)
            return {};
        return m_image->natural_size(*decoded_image_data);
    }

    mutable WeakPtr<Layout::Node> m_layout_node;
    NonnullRefPtr<CSS::AbstractImageStyleValue> m_image;
    mutable OwnPtr<ImageClient> m_image_client;
};

static void attach_owned_image_provider(Box& image_box, CSS::AbstractImageStyleValue& image)
{
    auto& document = image_box.document();
    image.load_any_resources(document);
    auto image_provider = GeneratedContentImageProvider::create(document, image);
    auto& image_provider_ref = *image_provider;
    image_box.set_owned_image_provider(move(image_provider));
    image_provider_ref.set_layout_node(image_box);
}

static Box& create_content_image_box(DOM::Document& document, GC::Ptr<DOM::Element> element, CSS::LayoutStyle style, CSS::AbstractImageStyleValue& image)
{
    auto& image_box = allocate_layout_node<Box>(document, element, style, RustFFI::NodeKind::ImageBox);
    attach_owned_image_provider(image_box, image);
    return image_box;
}

static RefPtr<CSS::AbstractImageStyleValue const> content_replacement_image(CSS::StyleValue const& content)
{
    if (!content.is_content())
        return nullptr;
    auto const& items = content.as_content().content().values();
    if (items.size() != 1 || !items.first()->is_abstract_image())
        return nullptr;
    return &items.first()->as_abstract_image();
}

// The image a box replaces its element's contents with, named by the record the box was stamped
// from - the same record the retired construction path read it out of.
static void attach_content_replacement_image(Box& image_box)
{
    auto replacement_image = content_replacement_image(image_box.style_group<CSS::ComputedValues::ContentValues>().computed_content_value());
    VERIFY(replacement_image);
    attach_owned_image_provider(image_box, const_cast<CSS::AbstractImageStyleValue&>(*replacement_image));
}

struct FirstLetterTextSlices {
    TextNode* first_letter_slice;
    TextNode* remainder_slice;
};

static FirstLetterTextSlices create_first_letter_text_slices(DOM::Document& document, TextNode& text_node, size_t letter_end)
{
    auto const full_length = text_node.text().length_in_code_units();

    // The first-letter and remainder boxes render slices of the same DOM text node; generated text
    // (from a content property) has no DOM node and gets plain generated slices of its text instead.
    if (auto* dom_text = text_node.dom_text()) {
        auto& mutable_dom_text = const_cast<DOM::Text&>(*dom_text);
        auto& remainder_slice = allocate_layout_node<TextNode>(document, mutable_dom_text, Node::AttachToDOMNode::Yes);
        auto& first_letter_slice = allocate_layout_node<TextNode>(document, mutable_dom_text, Node::AttachToDOMNode::No);
        return { &first_letter_slice, &remainder_slice };
    }

    auto text = text_node.text();
    return {
        &allocate_layout_node<GeneratedTextNode>(document, Utf16String::from_utf16(text.utf16_view().substring_view(0, letter_end))),
        &allocate_layout_node<GeneratedTextNode>(document, Utf16String::from_utf16(text.utf16_view().substring_view(letter_end, full_length - letter_end))),
    };
}

RustFFI::FfiFirstLetterNodes LayoutTreeBuildBridge::create_first_letter_nodes(DOM::Element& element, RustFFI::FfiFirstLetterTarget target)
{
    VERIFY(target.found);
    auto& text_node = as<TextNode>(*static_cast<Node*>(target.text_node));
    auto& document = element.document();

    auto [first_letter_slice, remainder_slice] = create_first_letter_text_slices(document, text_node, target.letter_end);

    auto const* first_letter_box_values = element.style_group<CSS::ComputedValues::BoxValues>(CSS::PseudoElement::FirstLetter);
    VERIFY(first_letter_box_values);
    auto display = first_letter_box_values->display_value();
    auto first_letter_wrapper = DOM::Element::create_layout_node_for_display_type(document, display, CSS::LayoutStyle { element.style_record_identity(CSS::PseudoElement::FirstLetter) }, nullptr);
    if (first_letter_wrapper) {
        first_letter_wrapper->attach_style_resources();
        first_letter_wrapper->set_generated_for(CSS::PseudoElement::FirstLetter, element);
        LayoutTreeBuilderAccess::set_synthetic_pseudo_element_node(element, CSS::PseudoElement::FirstLetter, first_letter_wrapper);
    }
    return {
        .wrapper = Node::slot_id(first_letter_wrapper),
        .first_letter_slice = Node::slot_id(first_letter_slice),
        .remainder_slice = Node::slot_id(remainder_slice),
    };
}

Box& LayoutTreeBuildBridge::create_list_item_marker(Box& list_box, CSS::LayoutStyle marker_style)
{
    auto& list_item_marker = allocate_layout_node<Box>(list_box.document(), nullptr, move(marker_style), RustFFI::NodeKind::ListItemMarkerBox);
    list_item_marker.set_list_marker_is_inside(list_box.list_style_position() == CSS::ListStylePosition::Inside);
    return list_item_marker;
}

static void* layout_node_arena_handle(DOM::AbstractElement const& element_reference)
{
    return element_reference.document().layout_node_arena().handle();
}

static u8 generated_for(DOM::AbstractElement const& element_reference)
{
    return Node::encode_generated_for(*element_reference.pseudo_element());
}

// https://drafts.csswg.org/css-lists-3/#text-markers
// NB: The tree build generates the marker string. What it is generated from is resolved here, when the marker box is
//     built, since resolving a counter style name settles the style scope's counter styles.
static Vector<ValueComparingRefPtr<CSS::CounterStyle const>> publish_normal_marker_content(DOM::AbstractElement const& element_reference, BlockContainer const& list_box, BlockContainer const& marker)
{
    RustFFI::FfiMarkerContent content {
        .kind = RustFFI::FfiMarkerContentKind::Image,
        .string = 0,
        .counter_style = nullptr,
        .text_depends_on_list_item_counter = false,
    };
    Vector<ValueComparingRefPtr<CSS::CounterStyle const>> counter_style_dependencies;
    if (!marker.list_style_image()) {
        auto const& list_style_type = list_box.list_style_type();
        content.text_depends_on_list_item_counter = CSS::marker_text_depends_on_list_item_counter_value(list_style_type);
        auto use_counter_style = [&](RefPtr<CSS::CounterStyle const> const& counter_style) {
            content.kind = RustFFI::FfiMarkerContentKind::CounterStyle;
            if (counter_style) {
                content.counter_style = counter_style->rust_counter_style();
                counter_style_dependencies.append(counter_style);
            }
        };
        list_style_type.visit(
            [](Empty const&) { VERIFY_NOT_REACHED(); },
            [&](RefPtr<CSS::CounterStyle const> const& counter_style) {
                use_counter_style(counter_style);
            },
            [&](Utf16String const& string) {
                content.kind = RustFFI::FfiMarkerContentKind::String;
                content.string = string.to_raw_leaked();
            },
            [&](CSS::UnresolvedCounterStyleName const&) {
                use_counter_style(nullptr);
            },
            [&](CSS::ListStyleSymbols const& symbols) {
                use_counter_style(symbols.counter_style);
            });
    }
    RustFFI::layout_arena_set_marker_content(layout_node_arena_handle(element_reference), element_reference.element().style_node_id().value(),
        generated_for(element_reference), element_reference.style_scope().style_engine_tree_scope().value(), content);
    return counter_style_dependencies;
}

// NB: The tree build resolves a pseudo-element's content. The counter styles it names are resolved here, when the
//     box is built, since resolving a counter style name settles the style scope's counter styles.
static void publish_generated_content(DOM::AbstractElement const& element_reference, NodeWithStyle& layout_node, BlockContainer const* originating_list_box)
{
    auto const* payloads = element_reference.style_record_payloads();
    VERIFY(payloads);
    auto const& content_values = *CSS::style_group_from_payloads<CSS::ComputedValues::ContentValues>(payloads);
    if (layout_node.is_list_item_marker_box() && content_values.content_is_normal()) {
        VERIFY(originating_list_box);
        layout_node.set_content_counter_style_dependencies(publish_normal_marker_content(element_reference, *originating_list_box, static_cast<BlockContainer const&>(layout_node)));
        return;
    }

    auto const& style_scope = element_reference.style_scope();
    auto value = content_values.computed_content_value();
    auto counter_style_dependencies = CSS::content_counter_style_dependencies(*value, style_scope);
    if (value->is_content()) {
        Vector<void const*> counter_styles;
        counter_styles.ensure_capacity(counter_style_dependencies.size());
        for (auto const& counter_style : counter_style_dependencies)
            counter_styles.unchecked_append(counter_style ? counter_style->rust_counter_style() : nullptr);
        RustFFI::layout_arena_set_content_counter_styles(layout_node_arena_handle(element_reference), element_reference.element().style_node_id().value(),
            generated_for(element_reference), style_scope.style_engine_tree_scope().value(), counter_styles.data(), counter_styles.size());
    }
    layout_node.set_content_counter_style_dependencies(move(counter_style_dependencies));
}

// The node an identity the walk carries names. The document is the build's root and is not in the
// style computer's node index, because a document holding a reference back to itself there would
// keep itself alive; every other identity resolves through the index.
// The DOM node the style mirror files under `style_node`. The document is the only node the
// style computer does not answer for, because it is not in the node map.
static DOM::Node& dom_node_for_style_node(DOM::Document& document, u32 style_node)
{
    CSS::StyleNodeID identity { style_node };
    if (identity == document.style_node_id())
        return document;
    auto node = document.style_computer().node_for_style_node(identity);
    VERIFY(node);
    return *node;
}

static CSS::PseudoElement css_pseudo_element(RustFFI::FfiPseudoElement pseudo_element)
{
    switch (pseudo_element) {
    case RustFFI::FfiPseudoElement::Before:
        return CSS::PseudoElement::Before;
    case RustFFI::FfiPseudoElement::After:
        return CSS::PseudoElement::After;
    case RustFFI::FfiPseudoElement::Marker:
        return CSS::PseudoElement::Marker;
    case RustFFI::FfiPseudoElement::Backdrop:
        return CSS::PseudoElement::Backdrop;
    case RustFFI::FfiPseudoElement::Other:
    case RustFFI::FfiPseudoElement::None:
        VERIFY_NOT_REACHED();
    }
    VERIFY_NOT_REACHED();
}

// A box the build produced for a pseudo-element, named by its arena row. The build hands these
// back by slot rather than keeping a pointer to them, so the frame carries no box of its own.
static NodeWithStyle* pseudo_element_build_node(DOM::Document& document, RustFFI::NodeSlotId slot)
{
    if (slot.index == RustFFI::INVALID_NODE_SLOT_INDEX)
        return nullptr;
    auto* layout_node = static_cast<Node*>(RustFFI::layout_arena_node_shell_if_live(document.layout_node_arena().handle(), slot));
    VERIFY(layout_node);
    return &as<NodeWithStyle>(*layout_node);
}

RustFFI::FfiPseudoTreeBuilderCallbacks LayoutTreeBuildBridge::make_ffi_pseudo_tree_builder_callbacks()
{
    return {
        .builder = this,
        .create_layout_node = [](void* builder_pointer, u32 style_node, RustFFI::FfiPseudoElement ffi_pseudo, RustFFI::FfiPseudoElementDecision decision, RustFFI::NodeSlotId originating_list_box_slot) -> RustFFI::NodeSlotId {
            VERIFY(builder_pointer);
            auto& builder = *static_cast<LayoutTreeBuildBridge*>(builder_pointer);
            auto& element = as<DOM::Element>(dom_node_for_style_node(*builder.m_document, style_node));
            auto pseudo_element = css_pseudo_element(ffi_pseudo);
            auto style_record_identity = element.style_record_identity(pseudo_element);
            VERIFY(style_record_identity);
            builder.pin_style_record_for_build(style_record_identity);
            auto const* pseudo_payloads = element.style_record_payloads(pseudo_element);
            VERIFY(pseudo_payloads);
            auto const display = CSS::style_group_from_payloads<CSS::ComputedValues::BoxValues>(pseudo_payloads)->display_value();
            auto const replacement_image = content_replacement_image(CSS::style_group_from_payloads<CSS::ComputedValues::ContentValues>(pseudo_payloads)->computed_content_value());
            CSS::LayoutStyle style { style_record_identity };
            auto& document = element.document();
            BlockContainer* originating_list_box = nullptr;
            if (auto* originating_box = pseudo_element_build_node(document, originating_list_box_slot))
                originating_list_box = &as<BlockContainer>(*originating_box);
            NodeWithStyle* layout_node = nullptr;
            switch (decision) {
            case RustFFI::FfiPseudoElementDecision::None:
                VERIFY_NOT_REACHED();
            case RustFFI::FfiPseudoElementDecision::ContentReplacement:
                VERIFY(replacement_image);
                layout_node = &create_content_image_box(document, nullptr, style, const_cast<CSS::AbstractImageStyleValue&>(*replacement_image));
                break;
            case RustFFI::FfiPseudoElementDecision::Contents:
                layout_node = &allocate_layout_node<NodeWithStyle>(document, nullptr, style, RustFFI::NodeKind::InlineNode);
                layout_node->set_display(CSS::Display(CSS::DisplayOutside::Inline, CSS::DisplayInside::Flow));
                break;
            case RustFFI::FfiPseudoElementDecision::Box:
                if (originating_list_box) {
                    layout_node = &create_list_item_marker(*originating_list_box, style);
                    break;
                }
                layout_node = DOM::Element::create_layout_node_for_display_type(document, display, style, nullptr);
                break;
            }
            if (layout_node)
                publish_generated_content({ element, pseudo_element }, *layout_node, originating_list_box);
            return Node::slot_id(layout_node); },
        .create_nested_list_marker = [](void* builder_pointer, u32 style_node, RustFFI::FfiPseudoElement originating_pseudo, Compositing::RustFFI::NodeSlotId pseudo_element_box_slot) -> Compositing::RustFFI::NodeSlotId {
            VERIFY(builder_pointer);
            auto& builder = *static_cast<LayoutTreeBuildBridge*>(builder_pointer);
            auto& element = as<DOM::Element>(dom_node_for_style_node(*builder.m_document, style_node));
            auto& list_item_box = as<BlockContainer>(*pseudo_element_build_node(element.document(), pseudo_element_box_slot));
            auto marker_style = [&] {
                // NB: Republishing the element's own ::marker style can retire its animation record while the
                //     pseudo-element still refers to it. Give the nested marker a copy of the existing style.
                if (auto style = element.computed_style(CSS::PseudoElement::Marker))
                    return CSS::ComputedValues::Builder { *style }.build();
                return element.document().style_computer().materialize_style_record({ element, CSS::PseudoElement::Marker });
            }();
            auto& list_item_marker = create_list_item_marker(list_item_box, move(marker_style));
            list_item_marker.attach_style_resources();
            // NB: The marker of a list-item ::before or ::after belongs to that pseudo-element, not to the element's own
            //     ::marker, so it is generated for the originating pseudo-element and never becomes the ::marker's box.
            list_item_marker.set_generated_for(css_pseudo_element(originating_pseudo), element);
            list_item_marker.set_content_counter_style_dependencies(publish_normal_marker_content({ element, css_pseudo_element(originating_pseudo) }, list_item_box, list_item_marker));
            return Node::slot_id(&list_item_marker); },
        .create_content_item = [](void* builder_pointer, u32 style_node, RustFFI::FfiPseudoElement ffi_pseudo, RustFFI::FfiGeneratedContentItem item, Compositing::RustFFI::NodeSlotId pseudo_element_box_slot) -> Compositing::RustFFI::NodeSlotId {
            VERIFY(builder_pointer);
            auto& builder = *static_cast<LayoutTreeBuildBridge*>(builder_pointer);
            auto& element = as<DOM::Element>(dom_node_for_style_node(*builder.m_document, style_node));
            auto& pseudo_element_box = *pseudo_element_build_node(element.document(), pseudo_element_box_slot);
            // The marker a list-item pseudo-element nests takes its content's style from itself.
            BlockContainer* nested_marker = nullptr;
            if (item.nested_marker.index != RustFFI::INVALID_NODE_SLOT_INDEX)
                nested_marker = &as<BlockContainer>(*pseudo_element_build_node(element.document(), item.nested_marker));
            Node* content_item = nullptr;
            if (item.kind == RustFFI::FfiGeneratedContentItemKind::Text) {
                content_item = &allocate_layout_node<GeneratedTextNode>(element.document(), Utf16String::adopt_raw(item.text));
            } else {
                auto& style_box = nested_marker ? static_cast<NodeWithStyle&>(*nested_marker) : pseudo_element_box;
                auto image = [&] -> NonnullRefPtr<CSS::AbstractImageStyleValue const> {
                    if (item.kind == RustFFI::FfiGeneratedContentItemKind::ListStyleImage)
                        return *style_box.list_style_image();
                    auto const* payloads = DOM::AbstractElement { element, css_pseudo_element(ffi_pseudo) }.style_record_payloads();
                    VERIFY(payloads);
                    auto content = CSS::style_group_from_payloads<CSS::ComputedValues::ContentValues>(payloads)->computed_content_value();
                    return content->as_content().content().values()[item.content_index]->as_abstract_image();
                }();
                auto& image_box = create_content_image_box(element.document(), nullptr, style_box.copy_computed_values(), const_cast<CSS::AbstractImageStyleValue&>(*image));
                // https://drafts.csswg.org/css-content-3/#content-property
                // For <image>, this is an inline anonymous replaced element.
                image_box.set_display(CSS::Display(CSS::DisplayOutside::Inline, CSS::DisplayInside::Flow));
                image_box.attach_style_resources();
                content_item = &image_box;
            }
            content_item->set_generated_for(css_pseudo_element(ffi_pseudo), element);
            return Node::slot_id(content_item); },
    };
}

// SVGPatternBox, SVGMaskBox, and SVGClipBox are created on behalf of a referencing element and
// attached to that element's layout subtree, so they survive cleanup of their DOM ancestor unless
// their layout attachment is inside the subtree being cleared too. The cleared root is only ever
// looked at here, so the walk carries it as an identity and it is resolved once one of these boxes
// asks rather than once per cleared node.
bool LayoutTreeBuildBridge::svg_resource_box_survives(DOM::Document& document, RustFFI::NodeSlotId slot, u32 cleared_subtree_root)
{
    if (cleared_subtree_root == 0)
        return true;
    auto cleared_subtree_root_node = document.style_computer().node_for_style_node(CSS::StyleNodeID { cleared_subtree_root });
    if (!cleared_subtree_root_node)
        return true;
    auto* layout_node = static_cast<Node*>(RustFFI::layout_arena_node_shell_if_live(document.layout_node_arena().handle(), slot));
    VERIFY(layout_node);
    for (auto* ancestor = layout_node->parent(); ancestor; ancestor = ancestor->parent()) {
        if (ancestor->is_anonymous())
            continue;
        auto const* dom_node = ancestor->dom_node();
        if (dom_node && dom_node->is_shadow_including_inclusive_descendant_of(*cleared_subtree_root_node))
            return false;
    }
    return true;
}

void LayoutTreeBuildBridge::detach_top_layer_element_layout_subtree(DOM::Element& element)
{
    RustFFI::FfiTopLayerDetachCallbacks callbacks {
        .context = &element.document(),
        .svg_resource_box_survives = [](void* document_pointer, RustFFI::NodeSlotId slot, u32 cleared_subtree_root) -> bool {
            VERIFY(document_pointer);
            return svg_resource_box_survives(*static_cast<DOM::Document*>(document_pointer), slot, cleared_subtree_root); },
    };
    RustFFI::rust_detach_top_layer_element_layout_subtree(
        &callbacks, element.document().layout_node_arena().handle(), element.style_node_id().value());
}

LayoutTreeBuildBridge::~LayoutTreeBuildBridge()
{
    if (m_pinned_style_records.is_empty())
        return;
    auto const& style_computer = m_document->style_computer();
    for (auto style_record_identity : m_pinned_style_records)
        style_computer.unpin_style_record(style_record_identity);
}

void LayoutTreeBuildBridge::pin_style_record_for_build(CSS::StyleRecordID style_record_identity)
{
    m_document->style_computer().pin_style_record(style_record_identity);
    m_pinned_style_records.append(style_record_identity);
}

RustFFI::FfiDomTreeBuilderCallbacks LayoutTreeBuildBridge::make_ffi_dom_tree_builder_callbacks()
{
    return {
        .builder = this,
        .svg_resource_box_survives = [](void* builder_pointer, RustFFI::NodeSlotId slot, u32 cleared_subtree_root) -> bool {
            VERIFY(builder_pointer);
            auto& builder = *static_cast<LayoutTreeBuildBridge*>(builder_pointer);
            return svg_resource_box_survives(*builder.m_document, slot, cleared_subtree_root); },
        .create_first_letter_nodes = [](void* builder_pointer, u32 style_node, RustFFI::FfiFirstLetterTarget target) -> RustFFI::FfiFirstLetterNodes {
            VERIFY(builder_pointer);
            auto& builder = *static_cast<LayoutTreeBuildBridge*>(builder_pointer);
            return create_first_letter_nodes(as<DOM::Element>(dom_node_for_style_node(*builder.m_document, style_node)), target); },
        .prepare_principal_element = [](void* builder_pointer, u32 style_node, bool should_create_layout_node) {
            VERIFY(builder_pointer);
            auto& builder = *static_cast<LayoutTreeBuildBridge*>(builder_pointer);
            auto& element = as<DOM::Element>(dom_node_for_style_node(*builder.m_document, style_node));
            element.update_inside_blocking_wheel_event_handler_state();
            if (should_create_layout_node)
                update_style_if_needed_for_layout_tree_bypass_path(element); },
        .attach_style_resources = [](void* builder_pointer, Compositing::RustFFI::NodeSlotId slot, bool owns_content_replacement_image) {
            VERIFY(builder_pointer);
            auto& builder = *static_cast<LayoutTreeBuildBridge*>(builder_pointer);
            auto* layout_node = static_cast<Node*>(RustFFI::layout_arena_node_shell_if_live(builder.m_document->layout_node_arena().handle(), slot));
            VERIFY(layout_node);
            // A box that replaces its element's contents with a single image owns the provider that
            // answers for it. The image is named by the same style record the box was stamped from,
            // and it loads before the resources the rest of that style asks for, as it did when the
            // box was built around it.
            if (owns_content_replacement_image)
                attach_content_replacement_image(as<Box>(*layout_node));
            as<NodeWithStyle>(*layout_node).attach_style_resources(); },

        .pseudo = make_ffi_pseudo_tree_builder_callbacks(),
    };
}

// A bypass path (top-layer iteration, slot projection, SVG mask/clip-path or pattern reference)
// may reach an element whose `computed_values` is null. Route through `update_style_for_element`,
// which seeds the style computer's ancestor filter so descendant-combinator selectors continue to
// match during the lazy re-cascade.
static void update_style_if_needed_for_layout_tree_bypass_path(DOM::Element& element)
{
    if (!element.has_style())
        element.document().update_style_for_element({ element });
}

RustFFI::FfiLayoutTreeBuildOutcome LayoutTreeBuildBridge::build(DOM::Node& dom_node)
{
    m_document = &dom_node.document();
    auto callbacks = make_ffi_dom_tree_builder_callbacks();
    auto& document = dom_node.document();
    return RustFFI::rust_build_layout_tree(&callbacks, document.layout_node_arena().handle(), &dom_node, document.style_node_id().value());
}

RustFFI::FfiLayoutTreeBuildOutcome build_layout_tree(DOM::Node& dom_node)
{
    LayoutTreeBuildBridge bridge;
    return bridge.build(dom_node);
}

void detach_top_layer_element_layout_subtree(DOM::Element& element)
{
    LayoutTreeBuildBridge::detach_top_layer_element_layout_subtree(element);
}

// https://drafts.csswg.org/css-tables-3/#fixup-algorithm

}
