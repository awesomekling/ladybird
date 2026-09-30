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
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/Layout/ImageProvider.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/TreeBuilder.h>
#include <LibWeb/Layout/TreeBuilderRustFFI.h>
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
        m_box = {};
    }

    static NonnullOwnPtr<GeneratedContentImageProvider> create(DOM::Document& document, NonnullRefPtr<CSS::AbstractImageStyleValue> image)
    {
        return adopt_own(*new GeneratedContentImageProvider(document, move(image)));
    }

    void set_box(Painting::BoxSlot const& box)
    {
        m_box = box;
        publish_natural_size();
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
    virtual Painting::BoxSlot image_provider_box() const override
    {
        return m_box;
    }

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
            if (!m_owner.m_box)
                return;
            m_owner.publish_natural_size();
            m_owner.image_provider_contents_changed();
            m_owner.m_box.document().render_inputs_for_write().set_needs_layout_update(m_owner.m_box.slot(), DOM::SetNeedsLayoutReason::GeneratedContentImageFinishedLoading);
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

    // The box's replaced content facts are derived from the natural size of the image it shows,
    // which the provider publishes to the box's row as it is handed over and as its image loads:
    // zero while the image is not available.
    void publish_natural_size() const
    {
        RustFFI::FfiReplacedContentFacts facts {};
        auto natural_size = is_image_available() ? this->natural_size() : CSS::SizeWithAspectRatio { 0, 0, {} };
        facts.has_auto_content_width = natural_size.width.has_value();
        facts.auto_content_width = natural_size.width.value_or(0);
        facts.has_auto_content_height = natural_size.height.has_value();
        facts.auto_content_height = natural_size.height.value_or(0);
        if (natural_size.aspect_ratio.has_value()) {
            facts.auto_content_aspect_ratio_numerator = natural_size.aspect_ratio->numerator();
            facts.auto_content_aspect_ratio_denominator = natural_size.aspect_ratio->denominator();
        }
        if (m_box)
            m_box.document().render_inputs_for_write().set_owned_image_natural_size(m_box.slot(), facts);
    }

    CSS::SizeWithAspectRatio natural_size() const
    {
        auto decoded_image_data = this->decoded_image_data();
        if (!decoded_image_data)
            return {};
        return m_image->natural_size(*decoded_image_data);
    }

    // The box the provider answers for, until it is detached.
    mutable Painting::BoxSlot m_box;
    NonnullRefPtr<CSS::AbstractImageStyleValue> m_image;
    mutable OwnPtr<ImageClient> m_image_client;
};

// The provider a box owns belongs to the arena, which deletes it with the box's row.
static void attach_owned_image_provider(Painting::BoxSlot const& image_box, CSS::AbstractImageStyleValue& image)
{
    ASSERT(image_box.kind() == RustFFI::NodeKind::ImageBox);
    if (image_box.kind() != RustFFI::NodeKind::ImageBox)
        return;
    auto& document = image_box.document();
    image.load_any_resources(document);
    auto image_provider = GeneratedContentImageProvider::create(document, image);
    auto& image_provider_ref = *image_provider;
    RustFFI::layout_arena_set_owned_image_provider(image_box.arena(), image_box.slot(), image_provider.leak_ptr());
    image_provider_ref.set_box(image_box);
}

static bool image_box_image_is_available(Painting::BoxSlot const& image_box)
{
    auto const* image_provider = image_provider_of_image_box(image_box);
    return image_provider && image_provider->is_image_available();
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
static void attach_content_replacement_image(Painting::BoxSlot const& image_box)
{
    auto const* content_values = image_box.style_group<CSS::ComputedValues::ContentValues>();
    auto replacement_image = content_values ? content_replacement_image(content_values->computed_content_value()) : nullptr;
    ASSERT(replacement_image);
    if (replacement_image)
        attach_owned_image_provider(image_box, const_cast<CSS::AbstractImageStyleValue&>(*replacement_image));
}

// The node an identity the walk carries names. The document is the build's root and is not in the
// style computer's node index, because a document holding a reference back to itself there would
// keep itself alive; every other identity resolves through the index.
// The DOM node the style mirror files under `style_node`. The document is the only node the
// style computer does not answer for, because it is not in the node map.
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

bool attach_owed_style_resources(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot, bool owns_content_replacement_image)
{
    auto box = Painting::BoxSlot::of(document, slot);
    ASSERT(box);
    if (!box)
        return false;
    // A box that replaces its element's contents with a single image owns the provider that
    // answers for it. The image is named by the same style record the box was stamped from,
    // and it loads before the resources the rest of that style asks for, as it did when the
    // box was built around it.
    bool image_was_available = false;
    if (owns_content_replacement_image) {
        attach_content_replacement_image(box);
        image_was_available = image_box_image_is_available(box);
        if (image_was_available)
            document.render_inputs_for_write().set_needs_layout_update(box.slot(), DOM::SetNeedsLayoutReason::GeneratedContentImageFinishedLoading);
    }
    attach_style_resources_to_box(box);
    return image_was_available;
}

bool attach_owed_generated_image(DOM::Document& document, Compositing::RustFFI::NodeSlotId slot, u32 style_node, RustFFI::FfiPseudoElement ffi_pseudo, RustFFI::FfiGeneratedContentItem item, Compositing::RustFFI::NodeSlotId pseudo_element_box_slot)
{
    // A generator that went away beside the frame that built its pseudo-element's boxes (removed, or adopted into
    // another document) names no image any more, and the boxes go away with it.
    auto generator = document.style_computer().node_for_style_node(CSS::StyleNodeID { style_node });
    if (!generator)
        return false;
    auto& element = as<DOM::Element>(*generator);
    auto image_box = Painting::BoxSlot::of(document, slot);
    // The marker a list-item pseudo-element nests takes its content's style from itself.
    auto style_box = Painting::BoxSlot::of(document, item.nested_marker.index != Compositing::RustFFI::INVALID_NODE_SLOT_INDEX ? item.nested_marker : pseudo_element_box_slot);
    ASSERT(image_box && style_box);
    if (!image_box || !style_box)
        return false;
    auto image = [&] -> NonnullRefPtr<CSS::AbstractImageStyleValue const> {
        if (item.kind == RustFFI::FfiGeneratedContentItemKind::ListStyleImage)
            return *style_box.style_group<CSS::ComputedValues::InheritedListValues>()->list_style_image_value();
        auto const* payloads = DOM::AbstractElement { element, css_pseudo_element(ffi_pseudo) }.style_record_payloads();
        VERIFY(payloads);
        auto content = CSS::style_group_from_payloads<CSS::ComputedValues::ContentValues>(payloads)->computed_content_value();
        return content->as_content().content().values()[item.content_index]->as_abstract_image();
    }();
    attach_owned_image_provider(image_box, const_cast<CSS::AbstractImageStyleValue&>(*image));
    bool image_was_available = image_box_image_is_available(image_box);
    if (image_was_available)
        document.render_inputs_for_write().set_needs_layout_update(image_box.slot(), DOM::SetNeedsLayoutReason::GeneratedContentImageFinishedLoading);
    attach_style_resources_to_box(image_box);
    return image_was_available;
}

// The viewport's style is the document's, which the style computer makes on demand rather than
// publishing, so the document makes it for a round whose build may build the viewport as it reads
// the round, with the navigable's scroll offset the viewport's row holds. The owner interns it as
// the round's job begins, while the document keeps it.
RustFFI::FfiDocumentStyleForBuild document_style_for_build(DOM::Document& document)
{
    auto document_style = document.style_computer().create_document_style();
    auto const& base = document_style->base_values();
    RustFFI::FfiDocumentStyleForBuild style {};
    static_assert(array_size(style.payloads.groups) == to_underlying(CSS::StyleGroupIndex::Count));
    for (size_t index = 0; index < array_size(style.payloads.groups); ++index)
        style.payloads.groups[index] = base.style_group_payload(static_cast<CSS::StyleGroupIndex>(index));
    style.longhand_table = base.computed_longhand_table();
    auto navigable = document.navigable();
    style.viewport_scroll_offset = navigable ? navigable->viewport_scroll_offset() : CSSPixelPoint {};
    document.keep_style_for_layout_tree_build(move(document_style));
    return style;
}

void detach_top_layer_element_layout_subtree(DOM::Element& element)
{
    RustFFI::rust_detach_top_layer_element_layout_subtree(
        document_layout_arena(element.document()), element.style_node_id().value());
}

// https://drafts.csswg.org/css-tables-3/#fixup-algorithm

}
