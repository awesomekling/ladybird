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
static NodeWithStyle* pseudo_element_build_node(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot)
{
    if (slot.index == Compositing::RustFFI::INVALID_NODE_SLOT_INDEX)
        return nullptr;
    auto* layout_node = static_cast<Node*>(RustFFI::layout_arena_node_shell_if_live(document.layout_node_arena().handle(), slot));
    VERIFY(layout_node);
    return &as<NodeWithStyle>(*layout_node);
}

bool attach_owed_style_resources(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot, bool owns_content_replacement_image)
{
    auto* layout_node = static_cast<Node*>(RustFFI::layout_arena_node_shell_if_live(document.layout_node_arena().handle(), slot));
    VERIFY(layout_node);
    // A box that replaces its element's contents with a single image owns the provider that
    // answers for it. The image is named by the same style record the box was stamped from,
    // and it loads before the resources the rest of that style asks for, as it did when the
    // box was built around it.
    bool image_was_available = false;
    if (owns_content_replacement_image) {
        auto& image_box = as<Box>(*layout_node);
        attach_content_replacement_image(image_box);
        image_was_available = image_box.image_provider().is_image_available();
        if (image_was_available)
            image_box.set_needs_layout_update(DOM::SetNeedsLayoutReason::GeneratedContentImageFinishedLoading);
    }
    as<NodeWithStyle>(*layout_node).attach_style_resources();
    return image_was_available;
}

bool attach_owed_generated_image(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot, u32 style_node, RustFFI::FfiPseudoElement ffi_pseudo, RustFFI::FfiGeneratedContentItem item, Compositing::RustFFI::NodeSlotId pseudo_element_box_slot)
{
    auto& element = as<DOM::Element>(dom_node_for_style_node(document, style_node));
    auto& image_box = as<Box>(*pseudo_element_build_node(document, slot));
    // The marker a list-item pseudo-element nests takes its content's style from itself.
    auto& style_box = item.nested_marker.index != Compositing::RustFFI::INVALID_NODE_SLOT_INDEX
        ? *pseudo_element_build_node(document, item.nested_marker)
        : *pseudo_element_build_node(document, pseudo_element_box_slot);
    auto image = [&] -> NonnullRefPtr<CSS::AbstractImageStyleValue const> {
        if (item.kind == RustFFI::FfiGeneratedContentItemKind::ListStyleImage)
            return *style_box.list_style_image();
        auto const* payloads = DOM::AbstractElement { element, css_pseudo_element(ffi_pseudo) }.style_record_payloads();
        VERIFY(payloads);
        auto content = CSS::style_group_from_payloads<CSS::ComputedValues::ContentValues>(payloads)->computed_content_value();
        return content->as_content().content().values()[item.content_index]->as_abstract_image();
    }();
    attach_owned_image_provider(image_box, const_cast<CSS::AbstractImageStyleValue&>(*image));
    bool image_was_available = image_box.image_provider().is_image_available();
    if (image_was_available)
        image_box.set_needs_layout_update(DOM::SetNeedsLayoutReason::GeneratedContentImageFinishedLoading);
    image_box.attach_style_resources();
    return image_was_available;
}

u32 prepare_layout_tree_build(DOM::Document& document)
{
    auto* arena = document.layout_node_arena().handle();
    // The viewport's style is the document's, which the style computer makes on demand rather than
    // publishing, so a build that may build the viewport is handed it before it starts.
    if (RustFFI::layout_arena_tree_build_may_create_viewport(arena, document.style_node_id().value())) {
        auto& style_computer = document.style_computer();
        auto document_style = style_computer.intern_anonymous_layout_style(*style_computer.create_document_style());
        RustFFI::layout_arena_publish_document_style_record(arena, document_style.value());
    }
    return document.style_node_id().value();
}

void detach_top_layer_element_layout_subtree(DOM::Element& element)
{
    RustFFI::rust_detach_top_layer_element_layout_subtree(
        element.document().layout_node_arena().handle(), element.style_node_id().value());
}

// https://drafts.csswg.org/css-tables-3/#fixup-algorithm

}
