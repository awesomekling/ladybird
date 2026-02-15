/*
 * Copyright (c) 2018-2023, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021-2025, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2025, Jelle Raaijmakers <jelle@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Demangle.h>
#include <LibWeb/CSS/CSSImageResource.h>
#include <LibWeb/CSS/ComputedProperties.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleValues/AbstractImageStyleValue.h>
#include <LibWeb/CSS/StyleValues/CalculatedStyleValue.h>
#include <LibWeb/CSS/StyleValues/ImageStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/Dump.h>
#include <LibWeb/HTML/FormAssociatedElement.h>
#include <LibWeb/HTML/HTMLHtmlElement.h>
#include <LibWeb/HTML/Navigable.h>
#include <LibWeb/Layout/BlockContainer.h>
#include <LibWeb/Layout/FormattingContext.h>
#include <LibWeb/Layout/InlineNode.h>
#include <LibWeb/Layout/Node.h>
#include <LibWeb/Layout/SVGSVGBox.h>
#include <LibWeb/Layout/TableWrapper.h>
#include <LibWeb/Layout/TextNode.h>
#include <LibWeb/Layout/Viewport.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/SVG/SVGFilterElement.h>
#include <LibWeb/SVG/SVGForeignObjectElement.h>

namespace Web::Layout {

Node::Node(DOM::Document& document, DOM::Node* node)
    : m_dom_node(node ? *node : document)
    , m_anonymous(node == nullptr)
{
    if (node)
        node->set_layout_node({}, *this);
}

Node::~Node() = default;

void Node::visit_edges(Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_dom_node);
    for (auto const& paintable : m_paintable) {
        visitor.visit(GC::Ptr { &paintable });
    }
    visitor.visit(m_containing_block);
    visitor.visit(m_inline_containing_block_if_applicable);
    visitor.visit(m_pseudo_element_generator);
    TreeNode::visit_edges(visitor);
}

// https://www.w3.org/TR/css-display-3/#out-of-flow
bool Node::is_out_of_flow(FormattingContext const& formatting_context) const
{
    // A layout node is out of flow if either:

    // 1. It is floated (which requires that floating is not inhibited).
    if (!formatting_context.inhibits_floating() && computed_values().float_() != CSS::Float::None)
        return true;

    // 2. It is "absolutely positioned".
    if (is_absolutely_positioned())
        return true;

    return false;
}

// https://drafts.csswg.org/css-position-3/#absolute-positioning-containing-block
// Checks if the computed values of this node would establish an absolute positioning
// containing block. This is separate from establishes_an_absolute_positioning_containing_block()
// because that function also checks is<Box>, but we need these checks for inline elements too.
bool Node::computed_values_establish_absolute_positioning_containing_block() const
{
    auto const& computed_values = this->computed_values();

    if (computed_values.position() != CSS::Positioning::Static)
        return true;

    // https://drafts.csswg.org/css-will-change/#will-change
    // If any non-initial value of a property would cause the element to generate a containing block for absolutely
    // positioned elements, specifying that property in will-change must cause the element to generate a containing
    // block for absolutely positioned elements.
    auto will_change_property = [&](CSS::PropertyID property_id) {
        return computed_values.will_change().has_property(property_id);
    };

    // https://drafts.csswg.org/css-transforms-1/#propdef-transform
    // Any computed value other than none for the transform affects containing block and stacking context
    if (!computed_values.transformations().is_empty() || will_change_property(CSS::PropertyID::Transform))
        return true;
    if (computed_values.translate() || will_change_property(CSS::PropertyID::Translate))
        return true;
    if (computed_values.rotate() || will_change_property(CSS::PropertyID::Rotate))
        return true;
    if (computed_values.scale() || will_change_property(CSS::PropertyID::Scale))
        return true;

    // https://drafts.csswg.org/css-transforms-2/#propdef-perspective
    // The use of this property with any value other than 'none' establishes a stacking context. It also establishes
    // a containing block for all descendants, just like the 'transform' property does.
    if (computed_values.perspective().has_value() || will_change_property(CSS::PropertyID::Perspective))
        return true;

    // https://drafts.csswg.org/filter-effects-1/#FilterProperty
    // A value other than none for the filter property results in the creation of a containing block for absolute and
    // fixed positioned descendants, unless the element it applies to is a document root element in the current
    // browsing context.
    if ((computed_values.filter().has_filters() || will_change_property(CSS::PropertyID::Filter)) && !is_root_element())
        return true;

    // https://drafts.csswg.org/filter-effects-2/#BackdropFilterProperty
    // A computed value of other than none results in the creation of both a stacking context and a containing block
    // for absolute and fixed position descendants, unless the element it applies to is a document root element in the
    // current browsing context.
    if ((computed_values.backdrop_filter().has_filters() || will_change_property(CSS::PropertyID::BackdropFilter)) && !is_root_element())
        return true;

    // https://drafts.csswg.org/css-contain-2/#containment-types
    // 4. The layout containment box establishes an absolute positioning containing block and a fixed positioning
    //    containing block.
    // 4. The paint containment box establishes an absolute positioning containing block and a fixed positioning
    //    containing block.
    if (has_layout_containment() || has_paint_containment() || will_change_property(CSS::PropertyID::Contain))
        return true;

    // https://drafts.csswg.org/css-transforms-2/#transform-style-property
    // A computed value of 'preserve-3d' for 'transform-style' on a transformable element establishes both a
    // stacking context and a containing block for all descendants.
    // FIXME: Check that the element is a transformable element.
    if (computed_values.transform_style() == CSS::TransformStyle::Preserve3d || will_change_property(CSS::PropertyID::TransformStyle))
        return true;

    // https://drafts.csswg.org/css-view-transitions-1/#snapshot-containing-block-concept
    // FIXME: The snapshot containing block is considered to be an absolute positioning containing block and a fixed
    //        positioning containing block for ::view-transition and its descendants.

    return false;
}

// https://drafts.csswg.org/css-position-3/#absolute-positioning-containing-block
bool Node::establishes_an_absolute_positioning_containing_block() const
{
    if (!is<Box>(*this))
        return false;

    if (is<Viewport>(*this))
        return true;

    return computed_values_establish_absolute_positioning_containing_block();
}

// https://drafts.csswg.org/css-position-3/#fixed-positioning-containing-block
bool Node::establishes_a_fixed_positioning_containing_block() const
{
    if (!is<Box>(*this))
        return false;

    auto const& computed_values = this->computed_values();

    // https://drafts.csswg.org/css-will-change/#will-change
    // If any non-initial value of a property would cause the element to generate a containing block for fixed
    // positioned elements, specifying that property in will-change must cause the element to generate a containing
    // block for fixed positioned elements.
    auto will_change_property = [&](CSS::PropertyID property_id) {
        return computed_values.will_change().has_property(property_id);
    };

    // https://drafts.csswg.org/css-transforms-1/#propdef-transform
    // Any computed value other than none for the transform affects containing block and stacking context
    if (!computed_values.transformations().is_empty() || will_change_property(CSS::PropertyID::Transform))
        return true;
    if (computed_values.translate() || will_change_property(CSS::PropertyID::Translate))
        return true;
    if (computed_values.rotate() || will_change_property(CSS::PropertyID::Rotate))
        return true;
    if (computed_values.scale() || will_change_property(CSS::PropertyID::Scale))
        return true;

    // https://drafts.csswg.org/css-transforms-2/#propdef-perspective
    // The use of this property with any value other than 'none' establishes a stacking context. It also establishes
    // a containing block for all descendants, just like the 'transform' property does.
    if (computed_values.perspective().has_value() || will_change_property(CSS::PropertyID::Perspective))
        return true;

    // https://drafts.csswg.org/filter-effects-1/#FilterProperty
    // A value other than none for the filter property results in the creation of a containing block for absolute and
    // fixed positioned descendants, unless the element it applies to is a document root element in the current
    // browsing context.
    if ((computed_values.filter().has_filters() || will_change_property(CSS::PropertyID::Filter)) && !is_root_element())
        return true;

    // https://drafts.csswg.org/filter-effects-2/#BackdropFilterProperty
    // A computed value of other than none results in the creation of both a stacking context and a containing block
    // for absolute and fixed position descendants, unless the element it applies to is a document root element in the
    // current browsing context.
    if ((computed_values.backdrop_filter().has_filters() || will_change_property(CSS::PropertyID::BackdropFilter)) && !is_root_element())
        return true;

    // https://drafts.csswg.org/css-contain-2/#containment-types
    // 4. The layout containment box establishes an absolute positioning containing block and a fixed positioning
    //    containing block.
    // 4. The paint containment box establishes an absolute positioning containing block and a fixed positioning
    //    containing block.
    if (has_layout_containment() || has_paint_containment() || will_change_property(CSS::PropertyID::Contain))
        return true;

    // https://drafts.csswg.org/css-transforms-2/#transform-style-property
    // A computed value of 'preserve-3d' for 'transform-style' on a transformable element establishes both a
    // stacking context and a containing block for all descendants.
    // FIXME: Check that the element is a transformable element.
    if (computed_values.transform_style() == CSS::TransformStyle::Preserve3d || will_change_property(CSS::PropertyID::TransformStyle))
        return true;

    // https://drafts.csswg.org/css-view-transitions-1/#snapshot-containing-block-concept
    // FIXME: The snapshot containing block is considered to be an absolute positioning containing block and a fixed
    //        positioning containing block for ::view-transition and its descendants.

    return false;
}

static GC::Ptr<Box> nearest_ancestor_capable_of_forming_a_containing_block(Node& node)
{
    for (auto* ancestor = node.parent(); ancestor; ancestor = ancestor->parent()) {
        if (ancestor->is_block_container()
            || ancestor->display().is_flex_inside()
            || ancestor->display().is_grid_inside()
            || ancestor->is_svg_svg_box()) {
            return as<Box>(ancestor);
        }
    }
    return nullptr;
}

void Node::recompute_containing_block(Badge<DOM::Document>)
{
    // Reset the inline containing block - we'll set it below if applicable.
    m_inline_containing_block_if_applicable = nullptr;

    if (is<TextNode>(*this)) {
        m_containing_block = nearest_ancestor_capable_of_forming_a_containing_block(*this);
        return;
    }

    auto position = computed_values().position();

    // https://drafts.csswg.org/css-position-3/#absolute-cb
    if (position == CSS::Positioning::Absolute) {
        auto* ancestor = parent();
        while (ancestor && !ancestor->establishes_an_absolute_positioning_containing_block())
            ancestor = ancestor->parent();
        m_containing_block = static_cast<Box*>(ancestor);

        // FIXME: Containing block handling for absolutely positioned elements needs architectural improvements.
        //
        //        The CSS specification defines the containing block as a *rectangle*, not a box. For most cases,
        //        this rectangle is derived from the padding box of the nearest positioned ancestor Box. However,
        //        when the positioned ancestor is an *inline* element (e.g., a <span> with position: relative),
        //        the containing block rectangle should be the bounding box of that inline's fragments.
        //
        //        Currently, m_containing_block is typed as Box*, which cannot represent inline elements.
        //        The proper fix would be to:
        //        1. Separate the concept of "the node that establishes the containing block" from "the containing
        //           block rectangle".
        //        2. Store a reference to the establishing node (which could be InlineNode or Box).
        //        3. Compute the containing block rectangle on demand based on the establishing node's type.
        //
        //        For now, we use a workaround: check if there's an inline element with position:relative (or
        //        other containing-block-establishing properties) between this node and its containing_block()
        //        in the DOM tree. If found, store it in m_inline_containing_block_if_applicable.
        //
        //        We check the DOM tree here (rather than the layout tree) because when a block element is inside
        //        an inline element, the layout tree restructures so the block becomes a sibling of the inline.
        //        But the CSS containing block relationship is based on the DOM structure.
        if (m_containing_block) {
            auto const* containing_block_dom_node = m_containing_block->dom_node();

            // For pseudo-elements, we need to start from the generating element itself, since it may
            // be the inline containing block. For regular elements, start from parent_element().
            GC::Ptr<DOM::Element const> first_ancestor_to_check;
            if (is_generated_for_pseudo_element()) {
                first_ancestor_to_check = m_pseudo_element_generator.ptr();
            } else if (auto const* this_dom_node = dom_node()) {
                first_ancestor_to_check = this_dom_node->parent_element();
            }

            for (auto dom_ancestor = first_ancestor_to_check; dom_ancestor; dom_ancestor = dom_ancestor->parent_element()) {
                // Stop if we reach the DOM node of the containing block.
                if (dom_ancestor.ptr() == containing_block_dom_node)
                    break;

                // Check if this DOM element has an InlineNode in the layout tree.
                auto layout_node = dom_ancestor->layout_node();
                if (!layout_node || !is<InlineNode>(*layout_node))
                    continue;

                // Check if this inline establishes an absolute positioning containing block.
                if (layout_node->computed_values_establish_absolute_positioning_containing_block()) {
                    m_inline_containing_block_if_applicable = const_cast<InlineNode*>(static_cast<InlineNode const*>(layout_node.ptr()));
                    break;
                }
            }
        }

        return;
    }

    // https://drafts.csswg.org/css-position-3/#fixed-cb
    if (position == CSS::Positioning::Fixed) {
        // The containing block is established by the nearest ancestor box that establishes an fixed positioning
        // containing block, with the bounds of the containing block determined identically to the absolute positioning
        // containing block.
        auto* ancestor = parent();
        while (ancestor && !ancestor->establishes_a_fixed_positioning_containing_block())
            ancestor = ancestor->parent();
        // If no ancestor establishes one, the box’s fixed positioning containing block is the initial fixed containing
        // block:
        if (!ancestor) {
            //  - in continuous media, the layout viewport (whose size matches the dynamic viewport size); as a result,
            //    fixed boxes do not move when the document is scrolled.
            ancestor = &root();
            // FIXME: - in paged media, the page area of each page; fixed positioned boxes are thus replicated on every
            //   page. (They are fixed with respect to the page box only, and are not affected by being seen through a
            //   viewport; as in the case of print preview, for example.)
        }
        m_containing_block = static_cast<Box*>(ancestor);
        return;
    }

    m_containing_block = nearest_ancestor_capable_of_forming_a_containing_block(*this);
}

// returns containing block this node would have had if its position was static
Box const* Node::static_position_containing_block() const
{
    return nearest_ancestor_capable_of_forming_a_containing_block(const_cast<Node&>(*this));
}

Box const* Node::non_anonymous_containing_block() const
{
    auto nearest_ancestor_box = containing_block();
    VERIFY(nearest_ancestor_box);
    while (nearest_ancestor_box->is_anonymous()) {
        nearest_ancestor_box = nearest_ancestor_box->containing_block();
        VERIFY(nearest_ancestor_box);
    }
    return nearest_ancestor_box;
}

// https://developer.mozilla.org/en-US/docs/Web/CSS/CSS_Positioning/Understanding_z_index/The_stacking_context
bool Node::establishes_stacking_context() const
{
    // NOTE: While MDN is not authoritative, there isn't a single convenient location
    //       in the CSS specifications where the rules for stacking contexts is described.
    //       That's why the "spec link" here points to MDN.

    if (!has_style())
        return false;

    if (is_svg_box())
        return false;

    // We make a stacking context for the viewport. Painting and hit testing starts from here.
    if (is_viewport())
        return true;

    // Root element of the document (<html>).
    if (is_root_element())
        return true;

    auto const& computed_values = this->computed_values();

    auto position = computed_values.position();

    // https://drafts.csswg.org/css-will-change/#will-change
    // If any non-initial value of a property would create a stacking context on the element, specifying that property
    // in will-change must create a stacking context on the element.
    auto will_change_property = [&](CSS::PropertyID property_id) {
        return computed_values.will_change().has_property(property_id);
    };

    auto has_z_index = computed_values.z_index().has_value() || will_change_property(CSS::PropertyID::ZIndex);

    // Element with a position value absolute or relative and z-index value other than auto.
    if (position == CSS::Positioning::Absolute || position == CSS::Positioning::Relative) {
        if (has_z_index) {
            return true;
        }
    }

    // Element with a position value fixed or sticky.
    if (position == CSS::Positioning::Fixed || position == CSS::Positioning::Sticky
        || will_change_property(CSS::PropertyID::Position)) {
        return true;
    }

    if (!computed_values.transformations().is_empty() || will_change_property(CSS::PropertyID::Transform))
        return true;

    if (computed_values.translate() || will_change_property(CSS::PropertyID::Translate))
        return true;

    if (computed_values.rotate() || will_change_property(CSS::PropertyID::Rotate))
        return true;

    if (computed_values.scale() || will_change_property(CSS::PropertyID::Scale))
        return true;

    // Element that is a child of a flex container, with z-index value other than auto.
    if (parent() && parent()->display().is_flex_inside() && has_z_index)
        return true;

    // Element that is a child of a grid container, with z-index value other than auto.
    if (parent() && parent()->display().is_grid_inside() && has_z_index)
        return true;

    // https://drafts.fxtf.org/filter-effects/#FilterProperty
    // https://drafts.fxtf.org/filter-effects-2/#backdrop-filter-operation
    // A computed value of other than none results in the creation of both a stacking context
    // [CSS21] and a Containing Block for absolute and fixed position descendants, unless the
    // element it applies to is a document root element in the current browsing context.
    // Spec Note: This rule works in the same way as for the filter property.
    if (computed_values.backdrop_filter().has_filters() || computed_values.filter().has_filters()
        || will_change_property(CSS::PropertyID::BackdropFilter)
        || will_change_property(CSS::PropertyID::Filter)) {
        return true;
    }

    // Element with any of the following properties with value other than none:
    // - transform
    // - filter
    // - backdrop-filter
    // - perspective
    // - clip-path
    // - mask / mask-image / mask-border
    if (computed_values.mask().has_value() || computed_values.clip_path().has_value() || computed_values.mask_image()
        || will_change_property(CSS::PropertyID::Mask)
        || will_change_property(CSS::PropertyID::ClipPath)
        || will_change_property(CSS::PropertyID::MaskImage)) {
        return true;
    }

    if (is_svg_foreign_object_box())
        return true;

    // https://drafts.fxtf.org/compositing/#propdef-isolation
    // For CSS, setting isolation to isolate will turn the element into a stacking context.
    if (computed_values.isolation() == CSS::Isolation::Isolate || will_change_property(CSS::PropertyID::Isolation))
        return true;

    // https://drafts.csswg.org/css-contain-2/#containment-types
    // 5. The layout containment box creates a stacking context.
    // 3. The paint containment box creates a stacking context.
    if (has_layout_containment() || has_paint_containment() || will_change_property(CSS::PropertyID::Contain))
        return true;

    // https://drafts.fxtf.org/compositing/#mix-blend-mode
    // Applying a blendmode other than normal to the element must establish a new stacking context.
    if (computed_values.mix_blend_mode() != CSS::MixBlendMode::Normal || will_change_property(CSS::PropertyID::MixBlendMode))
        return true;

    // https://drafts.csswg.org/css-view-transitions-1/#named-and-transitioning
    // Elements captured in a view transition during a view transition or whose view-transition-name computed value is
    // not 'none' (at any time):
    // - Form a stacking context.
    if (computed_values.view_transition_name().has_value() || will_change_property(CSS::PropertyID::ViewTransitionName))
        return true;

    // https://drafts.csswg.org/css-transforms-2/#propdef-perspective
    // The use of this property with any value other than 'none' establishes a stacking context.
    if (computed_values.perspective().has_value() || will_change_property(CSS::PropertyID::Perspective))
        return true;

    // https://drafts.csswg.org/css-transforms-2/#transform-style-property
    // A computed value of 'preserve-3d' for 'transform-style' on a transformable element establishes both a
    // stacking context and a containing block for all descendants.
    // FIXME: Check that the element is a transformable element.
    if (computed_values.transform_style() == CSS::TransformStyle::Preserve3d || will_change_property(CSS::PropertyID::TransformStyle))
        return true;

    return computed_values.opacity() < 1.0f || will_change_property(CSS::PropertyID::Opacity);
}

GC::Ptr<HTML::Navigable> Node::navigable() const
{
    return document().navigable();
}

Viewport const& Node::root() const
{
    VERIFY(document().layout_node());
    return *document().layout_node();
}

Viewport& Node::root()
{
    VERIFY(document().layout_node());
    return *document().layout_node();
}

bool Node::is_floating() const
{
    if (!has_style())
        return false;
    // flex-items don't float.
    if (is_flex_item())
        return false;
    return computed_values().float_() != CSS::Float::None;
}

bool Node::is_positioned() const
{
    return has_style() && computed_values().position() != CSS::Positioning::Static;
}

bool Node::is_absolutely_positioned() const
{
    if (!has_style())
        return false;
    auto position = computed_values().position();
    return position == CSS::Positioning::Absolute || position == CSS::Positioning::Fixed;
}

bool Node::is_fixed_position() const
{
    if (!has_style())
        return false;
    auto position = computed_values().position();
    return position == CSS::Positioning::Fixed;
}

bool Node::is_sticky_position() const
{
    if (!has_style())
        return false;
    auto position = computed_values().position();
    return position == CSS::Positioning::Sticky;
}

NodeWithStyle::NodeWithStyle(DOM::Document& document, DOM::Node* node, NonnullRefPtr<CSS::ComputedProperties> computed_style)
    : Node(document, node)
{
    m_has_style = true;
    if (auto* element = as_if<DOM::Element>(node)) {
        m_computed_values_ptr = &element->ensure_computed_values();
    } else {
        m_owned_computed_values = make<CSS::ComputedValues>();
        m_computed_values_ptr = m_owned_computed_values.ptr();
    }
    apply_style(computed_style);
}

NodeWithStyle::NodeWithStyle(DOM::Document& document, DOM::Node* node, NonnullOwnPtr<CSS::ComputedValues> computed_values)
    : Node(document, node)
    , m_owned_computed_values(move(computed_values))
    , m_computed_values_ptr(m_owned_computed_values.ptr())
{
    m_has_style = true;
}

void NodeWithStyle::visit_edges(Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_list_style_image_resource);
    if (m_owned_computed_values)
        m_owned_computed_values->visit_edges(visitor);
}

void NodeWithStyle::apply_style(CSS::ComputedProperties const& computed_style)
{
    // For nodes that own their ComputedValues (pseudo-elements), all properties
    // need to be populated here via populate_computed_values().
    // For element-backed nodes, populate_computed_values() is called during style computation.
    if (m_owned_computed_values)
        CSS::StyleComputer::populate_computed_values(mutable_computed_values(), computed_style, document());

    apply_style();
}

void NodeWithStyle::apply_style()
{
    auto const& computed_values = this->computed_values();

    if (auto list_style_image = computed_values.property_value(CSS::PropertyID::ListStyleImage)) {
        if (list_style_image->is_abstract_image()) {
            m_list_style_image = list_style_image->as_abstract_image();
            if (m_list_style_image->is_image())
                m_list_style_image_resource = document().ensure_css_image_resource(m_list_style_image->as_image().url());
        }
    }

    propagate_style_to_anonymous_wrappers();

    if (auto* box_node = as_if<NodeWithStyleAndBoxModelMetrics>(*this))
        box_node->propagate_style_along_continuation();
}

void NodeWithStyle::propagate_non_inherit_values(NodeWithStyle& target_node) const
{
    // NOTE: These properties are not inherited, but we still have to propagate them to anonymous wrappers.
    target_node.mutable_computed_values().set_text_decoration_line(computed_values().text_decoration_line());
    target_node.mutable_computed_values().set_text_decoration_thickness(computed_values().text_decoration_thickness());
    target_node.mutable_computed_values().set_text_decoration_color(computed_values().text_decoration_color());
    target_node.mutable_computed_values().set_text_decoration_style(computed_values().text_decoration_style());
}

void NodeWithStyle::propagate_style_to_anonymous_wrappers()
{
    // Update the style of any anonymous wrappers that inherit from this node.
    // FIXME: This is pretty hackish. It would be nicer if they shared the inherited style
    //        data structure somehow, so this wasn't necessary.

    // If this is a `display:table` box with an anonymous wrapper parent,
    // the parent inherits style from *this* node, not the other way around.
    if (auto* table_wrapper = as_if<TableWrapper>(parent()); table_wrapper && display().is_table_inside()) {
        static_cast<CSS::MutableComputedValues&>(static_cast<CSS::ComputedValues&>(const_cast<CSS::ImmutableComputedValues&>(table_wrapper->computed_values()))).inherit_from(computed_values());
        transfer_table_box_computed_values_to_wrapper_computed_values(table_wrapper->mutable_computed_values());
    }

    // Propagate style to all anonymous children (except table wrappers!)
    for_each_child_of_type<NodeWithStyle>([&](NodeWithStyle& child) {
        if (child.is_anonymous() && !is<TableWrapper>(child)) {
            auto& child_computed_values = static_cast<CSS::MutableComputedValues&>(static_cast<CSS::ComputedValues&>(const_cast<CSS::ImmutableComputedValues&>(child.computed_values())));
            child_computed_values.inherit_from(computed_values());
            propagate_non_inherit_values(child);
            child.propagate_style_to_anonymous_wrappers();
        }
        return IterationDecision::Continue;
    });
}

bool Node::is_root_element() const
{
    if (is_anonymous())
        return false;
    return is<HTML::HTMLHtmlElement>(*dom_node());
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

CSS::Display Node::display() const
{
    if (!has_style()) {
        // NOTE: No style means this is dumb text content.
        return CSS::Display(CSS::DisplayOutside::Inline, CSS::DisplayInside::Flow);
    }

    return computed_values().display();
}

CSS::Display Node::display_before_box_type_transformation() const
{
    if (!has_style()) {
        return CSS::Display(CSS::DisplayOutside::Inline, CSS::DisplayInside::Flow);
    }

    return computed_values().display_before_box_type_transformation();
}

bool Node::is_inline() const
{
    return display().is_inline_outside();
}

bool Node::is_inline_block() const
{
    auto display = this->display();
    return display.is_inline_outside() && display.is_flow_root_inside();
}

bool Node::is_inline_table() const
{
    auto display = this->display();
    return display.is_inline_outside() && display.is_table_inside();
}

bool Node::is_atomic_inline() const
{
    if (is_replaced_box())
        return true;
    auto display = this->display();
    return display.is_inline_outside() && !display.is_flow_inside();
}

GC::Ref<NodeWithStyle> NodeWithStyle::create_anonymous_wrapper() const
{
    auto wrapper = heap().allocate<BlockContainer>(const_cast<DOM::Document&>(document()), nullptr, computed_values().clone_inherited_values());
    wrapper->mutable_computed_values().set_display(CSS::Display(CSS::DisplayOutside::Block, CSS::DisplayInside::Flow));
    propagate_non_inherit_values(*wrapper);
    // CSS 2.2 9.2.1.1 creates anonymous block boxes, but 9.4.1 states inline-block creates a BFC.
    // Set wrapper to inline-block to participate correctly in the IFC within the parent inline-block.
    if (display().is_inline_block() && !has_children()) {
        wrapper->mutable_computed_values().set_display(CSS::Display::from_short(CSS::Display::Short::InlineBlock));
    }
    return *wrapper;
}

void NodeWithStyle::set_owned_computed_values(NonnullOwnPtr<CSS::ComputedValues> computed_values)
{
    m_owned_computed_values = move(computed_values);
    m_computed_values_ptr = m_owned_computed_values.ptr();
}

void NodeWithStyle::reset_table_box_computed_values_used_by_wrapper_to_init_values()
{
    VERIFY(this->display().is_table_inside());

    auto& mutable_computed_values = this->mutable_computed_values();
    mutable_computed_values.set_position(CSS::InitialValues::position());
    mutable_computed_values.set_float(CSS::InitialValues::float_());
    mutable_computed_values.set_clear(CSS::InitialValues::clear());
    mutable_computed_values.set_inset(CSS::InitialValues::inset());
    mutable_computed_values.set_margin(CSS::InitialValues::margin());
    // AD-HOC:
    // To match other browsers, z-index needs to be moved to the wrapper box as well,
    // even if the spec does not mention that: https://github.com/w3c/csswg-drafts/issues/11689
    // Note that there may be more properties that need to be added to this list.
    mutable_computed_values.set_z_index(CSS::InitialValues::z_index());
}

void NodeWithStyle::transfer_table_box_computed_values_to_wrapper_computed_values(CSS::ComputedValues& wrapper_computed_values)
{
    // The computed values of properties 'position', 'float', 'margin-*', 'top', 'right', 'bottom', and 'left' on the table element are used on the table wrapper box and not the table box;
    // all other values of non-inheritable properties are used on the table box and not the table wrapper box.
    // (Where the table element's values are not used on the table and table wrapper boxes, the initial values are used instead.)
    auto& cv = computed_values();
    auto& mutable_wrapper_computed_values = static_cast<CSS::MutableComputedValues&>(wrapper_computed_values);
    if (display().is_inline_outside())
        mutable_wrapper_computed_values.set_display(CSS::Display::from_short(CSS::Display::Short::InlineBlock));
    else
        mutable_wrapper_computed_values.set_display(CSS::Display::from_short(CSS::Display::Short::FlowRoot));

    // NB: Read transferred properties from the property value map rather than typed field
    //     accessors, because a previous table transfer may have reset the typed fields to
    //     initial values. The property value map always has the original values.
    auto keyword_property = [&](CSS::PropertyID id, auto to_typed, auto initial) {
        if (auto value = cv.property_value(id))
            return to_typed(value->to_keyword()).value_or(initial);
        return initial;
    };
    auto length_box_from_property_values = [&](CSS::PropertyID left_id, CSS::PropertyID top_id, CSS::PropertyID right_id, CSS::PropertyID bottom_id, CSS::LengthPercentageOrAuto const& default_value) {
        auto side = [&](CSS::PropertyID id) -> CSS::LengthPercentageOrAuto {
            auto value = cv.property_value(id);
            if (!value)
                return default_value;
            if (value->is_calculated() || value->is_percentage() || value->is_length() || value->has_auto())
                return CSS::LengthPercentageOrAuto::from_style_value(*value);
            return default_value;
        };
        return CSS::LengthBox { side(top_id), side(right_id), side(bottom_id), side(left_id) };
    };
    mutable_wrapper_computed_values.set_position(keyword_property(CSS::PropertyID::Position, CSS::keyword_to_positioning, CSS::InitialValues::position()));
    mutable_wrapper_computed_values.set_float(keyword_property(CSS::PropertyID::Float, CSS::keyword_to_float, CSS::InitialValues::float_()));
    mutable_wrapper_computed_values.set_clear(keyword_property(CSS::PropertyID::Clear, CSS::keyword_to_clear, CSS::InitialValues::clear()));
    mutable_wrapper_computed_values.set_inset(length_box_from_property_values(CSS::PropertyID::Left, CSS::PropertyID::Top, CSS::PropertyID::Right, CSS::PropertyID::Bottom, CSS::LengthPercentageOrAuto::make_auto()));
    mutable_wrapper_computed_values.set_margin(length_box_from_property_values(CSS::PropertyID::MarginLeft, CSS::PropertyID::MarginTop, CSS::PropertyID::MarginRight, CSS::PropertyID::MarginBottom, CSS::Length::make_px(0)));
    // AD-HOC:
    // To match other browsers, z-index needs to be moved to the wrapper box as well,
    // even if the spec does not mention that: https://github.com/w3c/csswg-drafts/issues/11689
    // Note that there may be more properties that need to be added to this list.
    if (auto value = cv.property_value(CSS::PropertyID::ZIndex)) {
        if (value->has_auto())
            mutable_wrapper_computed_values.set_z_index({});
        else if (value->is_integer())
            mutable_wrapper_computed_values.set_z_index(value->as_integer().integer());
        else if (value->is_calculated())
            mutable_wrapper_computed_values.set_z_index(static_cast<int>(value->as_calculated().resolve_integer({}).value_or(0)));
    }

    reset_table_box_computed_values_used_by_wrapper_to_init_values();
}

bool NodeWithStyle::is_body() const
{
    return dom_node() && dom_node() == document().body();
}

bool overflow_value_makes_box_a_scroll_container(CSS::Overflow overflow)
{
    switch (overflow) {
    case CSS::Overflow::Clip:
    case CSS::Overflow::Visible:
        return false;
    case CSS::Overflow::Auto:
    case CSS::Overflow::Hidden:
    case CSS::Overflow::Scroll:
        return true;
    }
    VERIFY_NOT_REACHED();
}

bool NodeWithStyle::is_scroll_container() const
{
    // NOTE: This isn't in the spec, but we want the viewport to behave like a scroll container.
    if (is_viewport())
        return true;

    return overflow_value_makes_box_a_scroll_container(overflow_x())
        || overflow_value_makes_box_a_scroll_container(overflow_y());
}

CSS::Overflow NodeWithStyle::overflow_x() const
{
    if (m_overflow_propagated_to_viewport)
        return CSS::Overflow::Visible;
    return computed_values().overflow_x();
}

CSS::Overflow NodeWithStyle::overflow_y() const
{
    if (m_overflow_propagated_to_viewport)
        return CSS::Overflow::Visible;
    return computed_values().overflow_y();
}

void Node::add_paintable(GC::Ptr<Painting::Paintable> paintable)
{
    if (!paintable)
        return;
    m_paintable.append(*paintable);
}

void Node::clear_paintables()
{
    m_paintable.clear();
}

GC::Ptr<Painting::Paintable> Node::create_paintable() const
{
    return nullptr;
}

bool Node::is_anonymous() const
{
    return m_anonymous;
}

DOM::Node const* Node::dom_node() const
{
    if (m_anonymous)
        return nullptr;
    return m_dom_node.ptr();
}

DOM::Node* Node::dom_node()
{
    if (m_anonymous)
        return nullptr;
    return m_dom_node.ptr();
}

DOM::Element const* Node::pseudo_element_generator() const
{
    VERIFY(m_generated_for.has_value());
    return m_pseudo_element_generator.ptr();
}

DOM::Element* Node::pseudo_element_generator()
{
    VERIFY(m_generated_for.has_value());
    return m_pseudo_element_generator.ptr();
}

DOM::Document& Node::document()
{
    return m_dom_node->document();
}

DOM::Document const& Node::document() const
{
    return m_dom_node->document();
}

// https://drafts.csswg.org/css-ui/#propdef-user-select
CSS::UserSelect Node::user_select_used_value() const
{
    // The used value is the same as the computed value, except:
    auto computed_value = computed_values().user_select();

    // 1. on editable elements where the used value is always 'contain' regardless of the computed value

    // 2. when the computed value is 'auto', in which case the used value is one of the other values as defined below

    // For the purpose of this specification, an editable element is either an editing host or a mutable form control with
    // textual content, such as textarea.
    auto* form_control = as_if<HTML::FormAssociatedTextControlElement>(dom_node());
    // FIXME: Check if this needs to exclude input elements with types such as color or range, and if so, which ones exactly.
    if ((dom_node() && dom_node()->is_editing_host()) || (form_control && form_control->is_mutable())) {
        return CSS::UserSelect::Contain;
    } else if (computed_value == CSS::UserSelect::Auto) {
        // The used value of 'auto' is determined as follows:
        // - On the '::before' and '::after' pseudo-elements, the used value is 'none'
        if (is_generated_for_before_pseudo_element() || is_generated_for_after_pseudo_element()) {
            return CSS::UserSelect::None;
        }

        // - If the element is an editable element, the used value is 'contain'
        // NOTE: We already handled this above.

        auto parent_element = parent();
        if (parent_element) {
            auto parent_used_value = parent_element->user_select_used_value();

            // - Otherwise, if the used value of user-select on the parent of this element is 'all', the used value is 'all'
            if (parent_used_value == CSS::UserSelect::All) {
                return CSS::UserSelect::All;
            }

            // - Otherwise, if the used value of user-select on the parent of this element is 'none', the used value is
            //   'none'
            if (parent_used_value == CSS::UserSelect::None) {
                return CSS::UserSelect::None;
            }
        }

        // - Otherwise, the used value is 'text'
        return CSS::UserSelect::Text;
    }

    return computed_value;
}

// https://drafts.csswg.org/css-contain-2/#containment-size
bool Node::has_size_containment() const
{
    // However, giving an element size containment has no effect if any of the following are true:

    // - if the element does not generate a principal box (as is the case with 'display: contents' or 'display: none')
    // Note: This is the principal box

    // - if its inner display type is 'table'
    if (display().is_table_inside())
        return false;

    // - if its principal box is an internal table box
    if (display().is_internal_table())
        return false;

    // - if its principal box is an internal ruby box or a non-atomic inline-level box
    // FIXME: Implement this.

    if (computed_values().contain().size_containment)
        return true;

    if (computed_values().container_type().is_size_container)
        return true;

    return false;
}
// https://drafts.csswg.org/css-contain-2/#containment-inline-size
bool Node::has_inline_size_containment() const
{
    // Giving an element inline-size containment has no effect if any of the following are true:

    // - if the element does not generate a principal box (as is the case with 'display: contents' or 'display: none')
    // Note: This is the principal box

    // - if its inner display type is 'table'
    if (display().is_table_inside())
        return false;

    // - if its principal box is an internal table box
    if (display().is_internal_table())
        return false;

    // - if its principal box is an internal ruby box or a non-atomic inline-level box
    // FIXME: Implement this.

    if (computed_values().contain().inline_size_containment)
        return true;

    if (computed_values().container_type().is_inline_size_container)
        return true;

    return false;
}
// https://drafts.csswg.org/css-contain-2/#containment-layout
bool Node::has_layout_containment() const
{
    // However, giving an element layout containment has no effect if any of the following are true:

    // - if the element does not generate a principal box (as is the case with 'display: contents' or 'display: none')
    // Note: This is the principal box

    // - if its principal box is an internal table box other than 'table-cell'
    if (display().is_internal_table() && !display().is_table_cell())
        return false;

    // - if its principal box is an internal ruby box or a non-atomic inline-level box
    // FIXME: Implement this.

    if (computed_values().contain().layout_containment)
        return true;

    // https://drafts.csswg.org/css-contain-2/#valdef-content-visibility-auto
    // Changes the used value of the 'contain' property so as to turn on layout containment, style containment, and
    // paint containment for the element.
    if (computed_values().content_visibility() == CSS::ContentVisibility::Auto)
        return true;

    return false;
}
// https://drafts.csswg.org/css-contain-2/#containment-style
bool Node::has_style_containment() const
{
    // However, giving an element style containment has no effect if any of the following are true:

    // - if the element does not generate a principal box (as is the case with 'display: contents' or 'display: none')
    // Note: This is the principal box

    if (computed_values().contain().style_containment)
        return true;

    if (computed_values().container_type().is_size_container || computed_values().container_type().is_inline_size_container)
        return true;

    // https://drafts.csswg.org/css-contain-2/#valdef-content-visibility-auto
    // Changes the used value of the 'contain' property so as to turn on layout containment, style containment, and
    // paint containment for the element.
    if (computed_values().content_visibility() == CSS::ContentVisibility::Auto)
        return true;

    return false;
}
// https://drafts.csswg.org/css-contain-2/#containment-paint
bool Node::has_paint_containment() const
{
    // However, giving an element paint containment has no effect if any of the following are true:

    // - if the element does not generate a principal box (as is the case with 'display: contents' or 'display: none')
    // Note: This is the principal box

    // - if its principal box is an internal table box other than 'table-cell'
    if (display().is_internal_table() && !display().is_table_cell())
        return false;

    // - if its principal box is an internal ruby box or a non-atomic inline-level box
    // FIXME: Implement this

    if (computed_values().contain().paint_containment)
        return true;

    // https://drafts.csswg.org/css-contain-2/#valdef-content-visibility-auto
    // Changes the used value of the 'contain' property so as to turn on layout containment, style containment, and
    // paint containment for the element.
    if (computed_values().content_visibility() == CSS::ContentVisibility::Auto)
        return true;

    return false;
}

bool NodeWithStyleAndBoxModelMetrics::should_create_inline_continuation() const
{
    // This node must have an inline parent.
    if (!parent())
        return false;
    auto const& parent_display = parent()->display();
    if (!parent_display.is_inline_outside() || !parent_display.is_flow_inside())
        return false;

    // This node must not be inline itself or out of flow (which gets handled separately).
    if (display().is_inline_outside() || is_out_of_flow())
        return false;

    // This node must not have `display: contents`; inline continuation gets handled by its children.
    if (display().is_contents())
        return false;

    // Internal table display types and table captions are handled by the table fixup algorithm.
    if (display().is_internal_table() || display().is_table_caption())
        return false;

    // Parent element must not be <foreignObject>
    if (is<SVG::SVGForeignObjectElement>(parent()->dom_node()))
        return false;

    // SVG related boxes should never be split.
    if (is_svg_box() || is_svg_svg_box() || is_svg_foreign_object_box())
        return false;

    return true;
}

void NodeWithStyleAndBoxModelMetrics::propagate_style_along_continuation() const
{
    auto continuation = continuation_of_node();
    while (continuation && continuation->is_anonymous())
        continuation = continuation->continuation_of_node();
    if (continuation)
        continuation->apply_style();
}

void NodeWithStyleAndBoxModelMetrics::visit_edges(Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_continuation_of_node);
}

void Node::set_needs_layout_update(DOM::SetNeedsLayoutReason reason)
{
    if (m_needs_layout_update)
        return;

    if constexpr (UPDATE_LAYOUT_DEBUG) {
        // NOTE: We check some conditions here to avoid debug spam in documents that don't do layout.
        auto navigable = this->navigable();
        if (navigable && navigable->active_document() == &document())
            dbgln_if(UPDATE_LAYOUT_DEBUG, "NEED LAYOUT {}", DOM::to_string(reason));
    }

    m_needs_layout_update = true;

    if (auto* box = as_if<Box>(this))
        box->reset_cached_intrinsic_sizes();

    // Mark any anonymous children generated by this node for layout update.
    // NOTE: if this node generated an anonymous parent, all ancestors are indiscriminately marked below.
    for_each_child_of_type<Box>([&](Box& child) {
        if (child.is_anonymous() && !is<TableWrapper>(child)) {
            child.m_needs_layout_update = true;
            child.reset_cached_intrinsic_sizes();
        }
        return IterationDecision::Continue;
    });

    for (auto* ancestor = parent(); ancestor; ancestor = ancestor->parent()) {
        if (ancestor->m_needs_layout_update)
            break;
        ancestor->m_needs_layout_update = true;
        if (auto* svg_box = as_if<SVGSVGBox>(ancestor)) {
            // Walk from `this` up to the SVG root to check if any abspos node in the path has its containing block
            // outside the SVG subtree. If so, partial SVG relayout cannot handle it — the abspos element is laid out
            // by its containing block's FC which is outside the SVG subtree.
            bool can_use_boundary = true;
            for (auto* node = this; node != svg_box; node = node->parent()) {
                if (node->is_absolutely_positioned()) {
                    if (auto cb = node->containing_block(); cb && !svg_box->is_inclusive_ancestor_of(*cb)) {
                        can_use_boundary = false;
                        break;
                    }
                }
            }
            if (!can_use_boundary)
                continue;
            document().mark_svg_root_as_needing_relayout(*svg_box);
            break;
        }
    }

    // Reset intrinsic size caches for ancestors up to abspos or SVG root boundary.
    // Absolutely positioned elements don't contribute to ancestor intrinsic sizes,
    // so changes inside an abspos box don't require resetting ancestor caches.
    // SVG root elements have intrinsic sizes determined solely by their own attributes
    // (width, height, viewBox), not by their children, so the same logic applies.
    for (auto* ancestor = parent(); ancestor; ancestor = ancestor->parent()) {
        auto* box = as_if<Box>(ancestor);
        if (!box)
            continue;
        box->reset_cached_intrinsic_sizes();
        if (box->is_absolutely_positioned() || box->is_svg_svg_box())
            break;
    }
}

}
