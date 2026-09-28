/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2025, Jelle Raaijmakers <jelle@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/NonnullOwnPtr.h>
#include <AK/NonnullRefPtr.h>
#include <AK/OwnPtr.h>
#include <AK/RefCounted.h>
#include <AK/Vector.h>
#include <AK/WeakPtr.h>
#include <AK/Weakable.h>
#include <AK/kmalloc.h>
#include <LibGC/Cell.h>
#include <LibGC/Root.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/StyleEngineIdentifiers.h>
#include <LibWeb/CSS/StyleValues/AbstractImageStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageStyleValue.h>
#include <LibWeb/DOM/NodeIdentity.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/NodeArena.h>
#include <LibWeb/Layout/TreeBuilderRustFFI.h>
#include <LibWeb/TreeTraversal.h>

namespace Web::Layout {

static_assert(sizeof(Compositing::RustFFI::NodeSlotId) == sizeof(u32));
static_assert(offsetof(Compositing::RustFFI::NodeSlotId, index) == 0);

static_assert(sizeof(RustFFI::NodeKind) == sizeof(u8));
static_assert(sizeof(RustFFI::NodeFlag) == sizeof(u32));

enum class BindToPreparedArenaSlot {
    Yes,
};

class WEB_API Node : public Weakable<Node> {
public:
    AK_ALLOC_WITH_KMALLOC_PARTITION(HeapPartition::Layout);

    virtual ~Node();
    static void delete_arena_owned_shell(Node&);
    StringView class_name() const;

    static Compositing::RustFFI::NodeSlotId slot_id(Node const*);
    RustFFI::NodeKind kind() const { return m_kind; }
    void* arena_handle() const { return m_arena; }

    Node* parent_ptr() { return linked_node(RustFFI::FfiNodeLink::Parent); }
    Node const* parent_ptr() const { return linked_node(RustFFI::FfiNodeLink::Parent); }
    Node* first_child_ptr() { return linked_node(RustFFI::FfiNodeLink::FirstChild); }
    Node const* first_child_ptr() const { return linked_node(RustFFI::FfiNodeLink::FirstChild); }
    Node* last_child_ptr() { return linked_node(RustFFI::FfiNodeLink::LastChild); }
    Node* next_sibling_ptr() { return linked_node(RustFFI::FfiNodeLink::NextSibling); }
    Node const* next_sibling_ptr() const { return linked_node(RustFFI::FfiNodeLink::NextSibling); }
    Node* previous_sibling_ptr() { return linked_node(RustFFI::FfiNodeLink::PreviousSibling); }
    bool has_children() const { return first_child_ptr() != nullptr; }

    Node* first_child() { return first_child_ptr(); }
    Node const* first_child() const { return first_child_ptr(); }
    Node* next_sibling() { return next_sibling_ptr(); }
    Node const* next_sibling() const { return next_sibling_ptr(); }
    Node* previous_sibling() { return previous_sibling_ptr(); }
    Node const* previous_sibling() const { return const_cast<Node*>(this)->previous_sibling_ptr(); }

    template<typename Callback>
    TraversalDecision for_each_in_inclusive_subtree(Callback callback) const
    {
        return traverse_preorder(*this, IncludeTraversalRoot::Yes, callback);
    }

    template<typename Callback>
    TraversalDecision for_each_in_inclusive_subtree(Callback callback)
    {
        return traverse_preorder(*this, IncludeTraversalRoot::Yes, callback);
    }

    template<typename U, typename Callback>
    TraversalDecision for_each_in_inclusive_subtree_of_type(Callback callback)
    {
        return for_each_in_inclusive_subtree([callback = move(callback)](Node& node) {
            if (auto* node_of_type = as_if<U>(node))
                return callback(*node_of_type);
            return TraversalDecision::Continue;
        });
    }

    template<typename U, typename Callback>
    TraversalDecision for_each_in_inclusive_subtree_of_type(Callback callback) const
    {
        return for_each_in_inclusive_subtree([callback = move(callback)](Node const& node) {
            if (auto const* node_of_type = as_if<U>(node))
                return callback(*node_of_type);
            return TraversalDecision::Continue;
        });
    }

    template<typename Callback>
    void for_each_child(Callback callback) const
    {
        return const_cast<Node&>(*this).for_each_child(move(callback));
    }

    template<typename Callback>
    void for_each_child(Callback callback)
    {
        for (auto* node = first_child_ptr(); node; node = node->next_sibling_ptr()) {
            if (callback(*node) == IterationDecision::Break)
                return;
        }
    }

    template<typename U, typename Callback>
    void for_each_child_of_type(Callback callback)
    {
        for (auto* node = first_child_ptr(); node; node = node->next_sibling_ptr()) {
            auto* node_of_type = as_if<U>(*node);
            if (!node_of_type)
                continue;
            if (callback(*node_of_type) == IterationDecision::Break)
                return;
        }
    }

    template<typename U, typename Callback>
    void for_each_child_of_type(Callback callback) const
    {
        return const_cast<Node&>(*this).template for_each_child_of_type<U>(move(callback));
    }

    Node* next_in_pre_order()
    {
        if (auto* child = first_child_ptr())
            return child;
        for (auto* node = this; node; node = node->parent_ptr()) {
            if (auto* next = node->next_sibling_ptr())
                return next;
        }
        return nullptr;
    }

    Node const* next_in_pre_order() const
    {
        return const_cast<Node*>(this)->next_in_pre_order();
    }

    Node* previous_in_pre_order()
    {
        if (auto* node = previous_sibling_ptr()) {
            while (auto* last_child = node->last_child_ptr())
                node = last_child;
            return node;
        }
        return parent_ptr();
    }

    Node const* previous_in_pre_order() const
    {
        return const_cast<Node*>(this)->previous_in_pre_order();
    }

    bool is_before(Node const& other) const
    {
        if (this == &other)
            return false;
        for (auto const* node = this; node; node = node->next_in_pre_order()) {
            if (node == &other)
                return true;
        }
        return false;
    }

    bool is_anonymous() const { return has_flag(RustFFI::NodeFlag::Anonymous); }
    bool is_document_element() const { return has_flag(RustFFI::NodeFlag::IsDocumentElement); }
    DOM::Node const* dom_node() const;
    DOM::Node* dom_node();
    // The identity of the DOM node this row belongs to, which names nothing for an anonymous row
    // and for a row whose node has left the tree.
    DOM::NodeIdentity dom_node_identity() const;

    GC::Ptr<DOM::Element const> pseudo_element_generator() const;
    GC::Ptr<DOM::Element> pseudo_element_generator();
    DOM::NodeIdentity pseudo_element_generator_identity() const;

    bool needs_layout_update() const { return has_flag(RustFFI::NodeFlag::NeedsLayoutUpdate); }

    // The arena measures a box that holds a scroll offset eagerly after a full commit, so the box carries that fact
    // as a flag: it is set when a box becomes an element's or a pseudo-element's box, and again whenever the stored
    // offset changes, each time re-derived from the one place the offset is stored.
    // The offset is also published to the arena, so that the render side can read a box's scroll
    // offset without asking the DOM where it is stored. Published from the same places, plus the
    // viewport's box, whose offset the navigable stores.
    void publish_scroll_offset();
    void publish_own_scroll_offset();
    [[nodiscard]] CSSPixelPoint dom_target_scroll_offset() const;

    [[nodiscard]] i64 dom_target_unique_node_id() const;

    // Whether this row's node sits in the user agent shadow tree of a text control that has focus.
    // The overflow measurement reserves a pixel for the caret of one, so the render side is told
    // the answer rather than asking: published when a row is built, and again for both sides of a
    // change of the document's focused area.

    // A rebuild can leave an old row and its replacement both built for one node, and the document
    // hands out only the replacement. The old row is still read as that node's box until the commit
    // that retires it, so a fact published about the node reaches every row built for it, each row
    // answering for itself: what a row holds depends on whether it is the one the node is bound to.
    void publish_to_every_row_built_for_dom_node(void (Node::*publish)());

    bool is_generated_for_pseudo_element() const { return generated_for() != 0; }
    Optional<CSS::PseudoElement> generated_for_pseudo_element() const
    {
        if (!is_generated_for_pseudo_element())
            return {};
        return static_cast<CSS::PseudoElement>(generated_for() - 1);
    }
    bool is_generated_for_before_pseudo_element() const { return generated_for() == encode_generated_for(CSS::PseudoElement::Before); }
    bool is_generated_for_after_pseudo_element() const { return generated_for() == encode_generated_for(CSS::PseudoElement::After); }
    static constexpr u8 encode_generated_for(CSS::PseudoElement pseudo_element)
    {
        static_assert(static_cast<u8>(CSS::PseudoElement::UnknownWebKit) < 0xff);
        return static_cast<u8>(pseudo_element) + 1;
    }

    // The StyleNodeID of the element or text node this row is bound to, or of the element it is
    // generated for, or 0.
    CSS::StyleNodeID style_node_id() const;
    // The StyleNodeID a row bound to this DOM node records, or 0 for a node that has none.

    void pin_style_record_for_detachment();

    DOM::Document& document();
    DOM::Document const& document() const;

    GC::Ptr<HTML::LocalNavigable> navigable() const;

    String debug_description() const;

    bool has_style() const { return has_flag(RustFFI::NodeFlag::HasStyle); }
    bool has_style_or_parent_with_style() const;

    bool is_atomic_inline() const;

    // These optimize hot is<T> variants for the surviving layout classes where dynamic_cast is too slow.
    virtual bool is_box() const { return false; }
    virtual bool is_text_node() const { return false; }
    virtual bool is_viewport() const { return false; }
    virtual bool is_node_with_style() const { return false; }

    bool is_inline_node() const { return kind() == RustFFI::NodeKind::InlineNode; }
    bool is_svg_box() const { return RustFFI::layout_node_kind_is_svg_box(kind()); }
    bool is_replaced_box() const { return RustFFI::layout_node_kind_is_replaced_box(kind()); }
    bool is_table_wrapper() const { return kind() == RustFFI::NodeKind::TableWrapper; }

    template<typename T>
    bool fast_is() const = delete;

    // The arena finds the containing block by walking up the layout tree; it is always a Box or null.
    [[nodiscard]] Box const* containing_block() const;
    [[nodiscard]] Box* containing_block();

    Gfx::Font const& first_available_font() const;

    NodeWithStyle* parent();
    NodeWithStyle const* parent() const;

    bool is_editing_host() const { return has_flag(RustFFI::NodeFlag::IsEditingHost); }
    void set_is_editing_host(bool value) { set_flag(RustFFI::HostNodeFlag::IsEditingHost, value); }

    // https://drafts.csswg.org/css-ui/#propdef-user-select
    CSS::UserSelect user_select_used_value() const;

protected:
    Node(DOM::Document&, BindToPreparedArenaSlot, Compositing::RustFFI::NodeSlotId, RustFFI::NodeKind);

    bool has_flag(RustFFI::NodeFlag flag) const
    {
        return (RustFFI::layout_arena_node_flags(m_arena, m_slot) & static_cast<u32>(flag)) != 0;
    }

    bool dom_target_stores_scroll_offset() const;

    bool has_compositor_animation_frame(RustFFI::CompositorAnimationFrameKind kind) const
    {
        return RustFFI::layout_arena_node_has_compositor_animation_frame(m_arena, m_slot, kind);
    }

    void set_flag(RustFFI::HostNodeFlag flag, bool value)
    {
        RustFFI::layout_arena_set_node_flag(m_arena, m_slot, flag, value);
    }

private:
    friend class NodeWithStyle;

    Node* linked_node(RustFFI::FfiNodeLink link) const
    {
        return static_cast<Node*>(RustFFI::layout_arena_node_link_shell(m_arena, m_slot, link));
    }

    Node* containing_block_node_if_live() const
    {
        return static_cast<Node*>(RustFFI::layout_arena_node_containing_block_shell_if_live(m_arena, m_slot));
    }

    u8 generated_for() const { return RustFFI::layout_arena_node_generated_for(m_arena, m_slot); }
    bool is_bound_to_pseudo_element() const;

    // The shell dies with its row, which dies with the document's render state, so neither outlives the document.
    GC::Ref<DOM::Document> m_document;
    void* m_arena { nullptr };
    Compositing::RustFFI::NodeSlotId m_slot;
    RustFFI::NodeKind m_kind { RustFFI::NodeKind::Unset };
    bool m_arena_is_destroying_shell { false };
};

template<typename T, typename... Args>
T& allocate_layout_node(Args&&... args)
{
    return *new T(forward<Args>(args)...);
}

class WEB_API NodeWithStyle : public Node {
public:
    NodeWithStyle(DOM::Document&, BindToPreparedArenaSlot, Compositing::RustFFI::NodeSlotId, RustFFI::NodeKind);

    virtual ~NodeWithStyle() override;

    CSS::StyleRecordID style_record_identity() const { return m_style_record_identity; }
    void const* style_payloads() const { return m_style_payloads; }

    template<typename StyleGroup>
    StyleGroup const& style_group() const
    {
        VERIFY(m_style_payloads);
        return style_group_of<StyleGroup>(m_style_payloads);
    }

    // A group of the style whose payloads a row holds, read without the row's shell.
    template<typename StyleGroup>
    static StyleGroup const& style_group_of(void const* style_payloads)
    {
        auto const* payloads = static_cast<void const* const*>(style_payloads);
        auto const* payload = payloads[StyleGroup::style_group_index];
        VERIFY(payload);
        return *static_cast<StyleGroup const*>(payload);
    }

    static CSS::LengthPercentageOrAuto length_percentage_or_auto(CSS::ComputedValuesFFI::ComputedLengthPercentageOrAuto const& value)
    {
        if (value.is_auto)
            return CSS::LengthPercentageOrAuto::make_auto();
        return CSS::LengthPercentage::view(value.value);
    }

    static CSS::LengthBox length_box(CSS::ComputedValuesFFI::ComputedLengthBox const& box)
    {
        return {
            length_percentage_or_auto(box.top),
            length_percentage_or_auto(box.right),
            length_percentage_or_auto(box.bottom),
            length_percentage_or_auto(box.left),
        };
    }

    CSS::Display display() const { return CSS::display_from_ffi_display(style_group<CSS::ComputedValues::BoxValues>().display); }
    CSS::Float float_() const { return static_cast<CSS::Float>(style_group<CSS::ComputedValues::BoxValues>().float_); }
    CSS::Clear clear() const { return static_cast<CSS::Clear>(style_group<CSS::ComputedValues::BoxValues>().clear); }
    CSS::Positioning position() const { return static_cast<CSS::Positioning>(style_group<CSS::ComputedValues::BoxValues>().position); }
    CSS::BoxSizing box_sizing() const { return static_cast<CSS::BoxSizing>(style_group<CSS::ComputedValues::BoxValues>().box_sizing); }
    CSS::Overflow overflow_x() const { return static_cast<CSS::Overflow>(style_group<CSS::ComputedValues::BoxValues>().overflow_x); }
    CSS::Overflow overflow_y() const { return static_cast<CSS::Overflow>(style_group<CSS::ComputedValues::BoxValues>().overflow_y); }
    CSS::Resize resize() const { return static_cast<CSS::Resize>(style_group<CSS::ComputedValues::BoxValues>().resize); }
    Variant<CSS::VerticalAlign, CSS::LengthPercentage> vertical_align() const
    {
        auto const& value = style_group<CSS::ComputedValues::BoxValues>().vertical_align;
        if (value.is_keyword)
            return static_cast<CSS::VerticalAlign>(value.keyword);
        return CSS::LengthPercentage::view(value.value);
    }
    Optional<int> z_index() const
    {
        auto const& values = style_group<CSS::ComputedValues::BoxValues>();
        if (!values.has_z_index)
            return {};
        return values.z_index;
    }
    CSS::Containment contain() const
    {
        auto const& values = style_group<CSS::ComputedValues::BoxValues>();
        return { values.size_containment, values.inline_size_containment, values.layout_containment, values.style_containment, values.paint_containment };
    }
    CSS::ContainerType container_type() const
    {
        auto const& values = style_group<CSS::ComputedValues::BoxValues>();
        return { values.is_size_container, values.is_inline_size_container, values.is_scroll_state_container };
    }
    CSS::ContentVisibility content_visibility() const { return static_cast<CSS::ContentVisibility>(style_group<CSS::ComputedValues::InheritedBoxValues>().content_visibility); }
    CSS::Direction direction() const { return static_cast<CSS::Direction>(style_group<CSS::ComputedValues::InheritedBoxValues>().direction); }
    CSS::WritingMode writing_mode() const { return static_cast<CSS::WritingMode>(style_group<CSS::ComputedValues::InheritedBoxValues>().writing_mode); }
    bool inline_axis_is_reverse() const
    {
        switch (writing_mode()) {
        case CSS::WritingMode::HorizontalTb:
        case CSS::WritingMode::VerticalRl:
        case CSS::WritingMode::VerticalLr:
        case CSS::WritingMode::SidewaysRl:
            return direction() == CSS::Direction::Rtl;
        case CSS::WritingMode::SidewaysLr:
            return direction() == CSS::Direction::Ltr;
        }
        VERIFY_NOT_REACHED();
    }
    CSS::Visibility visibility() const { return static_cast<CSS::Visibility>(style_group<CSS::ComputedValues::InheritedBoxValues>().visibility); }
    CSS::ImageRendering image_rendering() const { return static_cast<CSS::ImageRendering>(style_group<CSS::ComputedValues::InheritedBoxValues>().image_rendering); }
    Color caret_color() const { return style_group<CSS::ComputedValues::InheritedUIValues>().caret_color_value(); }
    Optional<Color> accent_color() const { return style_group<CSS::ComputedValues::InheritedUIValues>().accent_color_value(); }
    CSS::PreferredColorScheme color_scheme() const { return style_group<CSS::ComputedValues::InheritedUIValues>().color_scheme_value(); }
    ReadonlySpan<Utf16FlyString> color_schemes() const { return style_group<CSS::ComputedValues::InheritedUIValues>().color_schemes_span(); }
    bool color_scheme_only() const { return style_group<CSS::ComputedValues::InheritedUIValues>().color_scheme_only; }
    ReadonlySpan<CSS::ComputedValuesFFI::ComputedCursor> cursor() const { return style_group<CSS::ComputedValues::InheritedUIValues>().cursor_span(); }
    CSS::PointerEvents pointer_events() const { return style_group<CSS::ComputedValues::InheritedUIValues>().pointer_events_value(); }
    CSS::Appearance appearance() const { return static_cast<CSS::Appearance>(style_group<CSS::ComputedValues::MiscResetValues>().appearance); }
    CSS::WillChange will_change() const { return style_group<CSS::ComputedValues::MiscResetValues>().will_change_value(); }
    CSS::LengthBox scroll_margin() const { return length_box(style_group<CSS::ComputedValues::MiscResetValues>().scroll_margin); }
    CSS::ScrollSnapStop scroll_snap_stop() const { return static_cast<CSS::ScrollSnapStop>(style_group<CSS::ComputedValues::MiscResetValues>().scroll_snap_stop); }
    CSS::ScrollbarWidth scrollbar_width() const { return static_cast<CSS::ScrollbarWidth>(style_group<CSS::ComputedValues::MiscResetValues>().scrollbar_width); }
    CSS::UserSelect user_select() const { return static_cast<CSS::UserSelect>(style_group<CSS::ComputedValues::MiscResetValues>().user_select); }
    Optional<Utf16FlyString> view_transition_name() const { return style_group<CSS::ComputedValues::MiscResetValues>().view_transition_name_value(); }
    Color outline_color() const { return Color::from_bgra(style_group<CSS::ComputedValues::MiscResetValues>().outline_color); }
    Color column_rule_color() const { return Color::from_bgra(style_group<CSS::ComputedValues::MiscResetValues>().column_rule_color); }
    CSS::OutlineStyle outline_style() const { return static_cast<CSS::OutlineStyle>(style_group<CSS::ComputedValues::MiscResetValues>().outline_style); }
    CSSPixels outline_width() const { return style_group<CSS::ComputedValues::MiscResetValues>().outline_width; }
    Color background_color() const { return style_group<CSS::ComputedValues::BackgroundValues>().background_color_value(); }
    Vector<CSS::BackgroundLayerData> const& background_layers() const
    {
        if (!m_background_layers.has_value())
            m_background_layers = style_group<CSS::ComputedValues::BackgroundValues>().background_layers_value();
        return *m_background_layers;
    }
    Vector<CSS::BackgroundLayerData> const& mask_layers() const
    {
        if (!m_mask_layers.has_value())
            m_mask_layers = style_group<CSS::ComputedValues::MaskValues>().mask_layers_value();
        return *m_mask_layers;
    }
    CSS::ListStyleType const& list_style_type() const
    {
        if (!m_list_style_type.has_value()) {
            m_list_style_type = style_group<CSS::ComputedValues::InheritedListValues>().list_style_type_value(style_scope());
        }
        return *m_list_style_type;
    }
    CSS::ListStylePosition list_style_position() const { return static_cast<CSS::ListStylePosition>(style_group<CSS::ComputedValues::InheritedListValues>().list_style_position); }
    CSS::AbstractImageStyleValue const* list_style_image() const
    {
        if (!m_list_style_image.has_value())
            m_list_style_image = style_group<CSS::ComputedValues::InheritedListValues>().list_style_image_value();
        return m_list_style_image->ptr();
    }
    CSS::ComputedFilterView backdrop_filter() const { return style_group<CSS::ComputedValues::EffectsValues>().backdrop_filter_value(); }
    CSS::Clip clip() const { return style_group<CSS::ComputedValues::EffectsValues>().clip_value(); }
    CSS::ComputedFilterView filter() const { return style_group<CSS::ComputedValues::EffectsValues>().filter_value(); }
    CSS::MixBlendMode mix_blend_mode() const { return style_group<CSS::ComputedValues::EffectsValues>().mix_blend_mode_value(); }
    float opacity() const { return style_group<CSS::ComputedValues::EffectsValues>().opacity; }
    ReadonlySpan<CSS::ShadowData> box_shadow() const { return style_group<CSS::ComputedValues::EffectsValues>().box_shadow_span(); }
    CSS::BorderData const& border_left() const { return style_group<CSS::ComputedValues::BorderValues>().border_left_value(); }
    CSS::BorderData const& border_top() const { return style_group<CSS::ComputedValues::BorderValues>().border_top_value(); }
    CSS::BorderData const& border_right() const { return style_group<CSS::ComputedValues::BorderValues>().border_right_value(); }
    CSS::BorderData const& border_bottom() const { return style_group<CSS::ComputedValues::BorderValues>().border_bottom_value(); }
    CSS::BorderImageData const& border_image() const
    {
        if (!m_border_image.has_value())
            m_border_image = style_group<CSS::ComputedValues::BorderValues>().border_image_value();
        return *m_border_image;
    }
    Color color() const { return style_group<CSS::ComputedValues::InheritedTextValues>().color_value(); }
    Color webkit_text_fill_color() const { return style_group<CSS::ComputedValues::InheritedTextValues>().webkit_text_fill_color_value(); }
    CSSPixels letter_spacing() const { return style_group<CSS::ComputedValues::InheritedTextValues>().letter_spacing_value(); }
    ReadonlySpan<CSS::ShadowData> text_shadow() const { return style_group<CSS::ComputedValues::InheritedTextValues>().text_shadow_span(); }
    CSS::TextTransform text_transform() const { return style_group<CSS::ComputedValues::InheritedTextValues>().text_transform_value(); }
    CSS::WhiteSpaceCollapse white_space_collapse() const { return style_group<CSS::ComputedValues::InheritedTextValues>().white_space_collapse_value(); }
    Color text_decoration_color() const { return Color::from_bgra(style_group<CSS::ComputedValues::TextResetValues>().text_decoration_color); }
    CSSPixels line_height() const { return style_group<CSS::ComputedValues::FontValues>().line_height_used; }
    CSSPixels font_size() const { return style_group<CSS::ComputedValues::FontValues>().font_size; }
    Gfx::FontCascadeList const& font_list() const { return style_group<CSS::ComputedValues::FontValues>().font_list_value(); }
    CSS::FlexDirection flex_direction() const { return static_cast<CSS::FlexDirection>(style_group<CSS::ComputedValues::AlignmentValues>().flex_direction); }
    CSS::AlignSelf align_self() const { return static_cast<CSS::AlignSelf>(style_group<CSS::ComputedValues::AlignmentValues>().align_self); }
    CSS::JustifySelf justify_self() const { return static_cast<CSS::JustifySelf>(style_group<CSS::ComputedValues::AlignmentValues>().justify_self); }
    i32 order() const { return style_group<CSS::ComputedValues::AlignmentValues>().order; }
    CSS::Size const& width() const { return CSS::Size::view(style_group<CSS::ComputedValues::SizingValues>().width); }
    CSS::Size const& min_width() const { return CSS::Size::view(style_group<CSS::ComputedValues::SizingValues>().min_width); }
    CSS::Size const& max_width() const { return CSS::Size::view(style_group<CSS::ComputedValues::SizingValues>().max_width); }
    CSS::Size const& height() const { return CSS::Size::view(style_group<CSS::ComputedValues::SizingValues>().height); }
    CSS::Size const& min_height() const { return CSS::Size::view(style_group<CSS::ComputedValues::SizingValues>().min_height); }
    CSS::Size const& max_height() const { return CSS::Size::view(style_group<CSS::ComputedValues::SizingValues>().max_height); }
    CSS::LengthBox inset() const { return length_box(style_group<CSS::ComputedValues::SurroundValues>().inset); }
    CSS::LengthBox margin() const { return length_box(style_group<CSS::ComputedValues::SurroundValues>().margin); }
    CSS::LengthBox padding() const { return length_box(style_group<CSS::ComputedValues::SurroundValues>().padding); }
    CSS::PositionAnchor position_anchor_value() const { return style_group<CSS::ComputedValues::AnchorValues>().position_anchor_value(); }
    bool has_transformations() const { return style_group<CSS::ComputedValues::TransformValues>().has_transformations(); }
    template<typename Callback>
    void for_each_transformation(Callback callback) const
    {
        style_group<CSS::ComputedValues::TransformValues>().for_each_transformation(callback);
    }
    template<typename Callback>
    void for_each_resolved_transform(Callback callback) const
    {
        style_group<CSS::ComputedValues::TransformValues>().for_each_resolved_transform(callback);
    }
    CSS::TransformOrigin transform_origin() const { return style_group<CSS::ComputedValues::TransformValues>().transform_origin_value(); }
    CSS::TransformStyle transform_style() const { return style_group<CSS::ComputedValues::TransformValues>().transform_style_value(); }
    RefPtr<CSS::TransformationStyleValue const> rotate() const { return style_group<CSS::ComputedValues::TransformValues>().rotate_value(); }
    RefPtr<CSS::TransformationStyleValue const> translate() const { return style_group<CSS::ComputedValues::TransformValues>().translate_value(); }
    RefPtr<CSS::TransformationStyleValue const> scale() const { return style_group<CSS::ComputedValues::TransformValues>().scale_value(); }
    bool has_rotate() const { return rotate() != nullptr; }
    bool has_translate() const { return translate() != nullptr; }
    bool has_scale() const { return scale() != nullptr; }
    Optional<CSSPixels> perspective() const { return style_group<CSS::ComputedValues::TransformValues>().perspective_value(); }
    Optional<CSS::MaskReference> mask() const { return style_group<CSS::ComputedValues::MaskValues>().mask_value(); }
    CSS::MaskType mask_type() const { return style_group<CSS::ComputedValues::MaskValues>().mask_type_value(); }
    Optional<CSS::URL> clip_path() const { return style_group<CSS::ComputedValues::MaskValues>().clip_path_value(); }
    Optional<CSS::BaselineMetric> dominant_baseline() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().dominant_baseline_value(); }
    Optional<CSS::SVGPaint> fill() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().fill_value(); }
    Optional<CSS::SVGPaint> stroke() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().stroke_value(); }
    float fill_opacity() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().fill_opacity; }
    ReadonlySpan<CSS::ComputedValuesFFI::ComputedSvgDash> stroke_dasharray() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().stroke_dasharray_span(); }
    CSS::LengthPercentage const& stroke_dashoffset() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().stroke_dashoffset_value(); }
    CSS::StrokeLinecap stroke_linecap() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().stroke_linecap_value(); }
    CSS::StrokeLinejoin stroke_linejoin() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().stroke_linejoin_value(); }
    double stroke_miterlimit() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().stroke_miterlimit; }
    float stroke_opacity() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().stroke_opacity; }
    CSS::LengthPercentage const& stroke_width() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().stroke_width_value(); }
    CSS::PaintOrderList paint_order() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().paint_order_value(); }
    CSS::TextAnchor text_anchor() const { return style_group<CSS::ComputedValues::InheritedSVGValues>().text_anchor_value(); }
    bool is_inline_block() const;
    Gfx::AffineTransform used_svg_element_transform() const;

    bool is_positioned() const;
    bool is_fixed_position() const;
    bool is_sticky_position() const;

    bool establishes_an_absolute_positioning_containing_block() const;
    bool establishes_a_fixed_positioning_containing_block() const;

    void apply_style(CSS::PublishedStyleRecord const&);
    // Applies the style to the row as apply_style() does to its shell, without making a shell for a row
    // that has none.
    static void apply_style(Row const&, CSS::PublishedStyleRecord const&);
    void attach_style_resources();
    bool synchronize_table_span_data();

    Gfx::Font const& first_available_font() const;
    CSS::StyleScope const& style_scope() const;

    bool is_scroll_container() const;

    void set_computed_values(NonnullRefPtr<CSS::ComputedValues const>);
    void set_style_record(CSS::PublishedStyleRecord const*);
    // Sets the row's record as set_style_record() does its shell's, without making a shell for a row that has none.
    static void set_style_record(Row const&, CSS::PublishedStyleRecord const*);
    void refresh_style_from_arena(CSS::StyleRecordID, void const* payloads, bool should_attach_resources);
    // The pin lives on the node's arena row and is released with it, so
    // Document::tear_down_layout_tree() must free the layout root before the document's style
    // computer goes away. Every document destruction path goes through that teardown.
    void pin_style_record_for_cxx_consumers();
    void release_pinned_style_record();

    void set_display(CSS::Display);

    void initialize_stamped_style_record();

private:
    virtual bool is_node_with_style() const final { return true; }

    void initialize_from_style_record();
    void publish_style_record_to_node_data();
    void did_update_style_record();

    void const* m_style_payloads { nullptr };
    bool has_layout_derived_style() const;
    CSS::StyleRecordID m_style_record_identity;
    mutable Optional<Vector<CSS::BackgroundLayerData>> m_background_layers;
    mutable Optional<Vector<CSS::BackgroundLayerData>> m_mask_layers;
    mutable Optional<CSS::BorderImageData> m_border_image;
    mutable Optional<CSS::ListStyleType> m_list_style_type;
    mutable Optional<RefPtr<CSS::AbstractImageStyleValue const>> m_list_style_image;
};

template<>
inline bool Node::fast_is<NodeWithStyle>() const { return is_node_with_style(); }

inline bool Node::has_style_or_parent_with_style() const
{
    return has_style() || (parent() != nullptr && parent()->has_style_or_parent_with_style());
}

inline Gfx::Font const& Node::first_available_font() const
{
    VERIFY(has_style_or_parent_with_style());
    if (has_style())
        return static_cast<NodeWithStyle const*>(this)->first_available_font();
    return parent()->first_available_font();
}

inline NodeWithStyle const* Node::parent() const
{
    return static_cast<NodeWithStyle const*>(parent_ptr());
}

inline NodeWithStyle* Node::parent()
{
    return static_cast<NodeWithStyle*>(parent_ptr());
}

inline Gfx::Font const& NodeWithStyle::first_available_font() const
{
    return font_list().first_available_font();
}

}
