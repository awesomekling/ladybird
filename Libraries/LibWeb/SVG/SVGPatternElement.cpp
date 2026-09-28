/*
 * Copyright (c) 2026, Tim Ledbetter <tim.ledbetter@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibGfx/Matrix4x4.h>
#include <LibWeb/CSS/StyleEngineInput.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/SVG/AttributeNames.h>
#include <LibWeb/SVG/AttributeParsing.h>
#include <LibWeb/SVG/FragmentIdentifier.h>
#include <LibWeb/SVG/SVGGraphicsElement.h>
#include <LibWeb/SVG/SVGPatternElement.h>

namespace Web::SVG {

GC_DEFINE_ALLOCATOR(SVGPatternElement);

SVGPatternElement::SVGPatternElement(DOM::Document& document, DOM::QualifiedName qualified_name)
    : SVGElement(document, move(qualified_name))
{
}

void SVGPatternElement::initialize_element()
{
    SVGFitToViewBox::initialize_fit_to_view_box();
}

void SVGPatternElement::visit_edges(Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);
    SVGURIReferenceMixin::visit_edges(visitor);
    SVGFitToViewBox::visit_edges(visitor);
}

void SVGPatternElement::attribute_changed(Utf16FlyString const& name, Optional<Utf16String> const& old_value, Optional<Utf16String> const& value, Optional<Utf16FlyString> const& namespace_)
{
    Base::attribute_changed(name, old_value, value, namespace_);
    SVGFitToViewBox::attribute_changed(*this, name, value);

    if (name == AttributeNames::patternUnits) {
        m_pattern_units = parse_units(value.value_or({}));
    } else if (name == AttributeNames::patternContentUnits) {
        m_pattern_content_units = parse_units(value.value_or({}));
    } else if (name == AttributeNames::patternTransform) {
        if (auto transform_list = parse_transform(value.value_or({})); transform_list.has_value()) {
            m_pattern_transform = transform_from_transform_list(*transform_list);
        } else {
            m_pattern_transform = {};
        }
    } else if (name == AttributeNames::x) {
        m_x = parse_number_percentage(value.value_or({}));
    } else if (name == AttributeNames::y) {
        m_y = parse_number_percentage(value.value_or({}));
    } else if (name == AttributeNames::width) {
        m_width = parse_number_percentage(value.value_or({}));
    } else if (name == AttributeNames::height) {
        m_height = parse_number_percentage(value.value_or({}));
    }

    // A pattern that names this one inherits the attributes it does not carry, so a change here is
    // a change to every pattern whose chain passes through this one.
    document().republish_inheriting_svg_pattern_attribute_facts();
}

// Only a pattern in the document's node tree takes part: a pattern's `href` resolves in the
// document scope, so a pattern inside a shadow tree can neither be named by one nor name one.
void SVGPatternElement::inserted()
{
    Base::inserted();

    if (root().is_document())
        register_in_document_pattern_list();
}

void SVGPatternElement::removed_from(IsSubtreeRoot is_subtree_root, Node* old_ancestor, Node& old_root)
{
    Base::removed_from(is_subtree_root, old_ancestor, old_root);

    if (old_root.is_document())
        unregister_from_document_pattern_list();
}

void SVGPatternElement::moved_from(IsSubtreeRoot is_subtree_root, GC::Ptr<Node> old_ancestor)
{
    Base::moved_from(is_subtree_root, old_ancestor);

    if (!old_ancestor)
        return;

    auto was_in_document_tree = old_ancestor->root().is_document();
    auto is_in_document_tree = root().is_document();
    if (was_in_document_tree == is_in_document_tree)
        return;

    if (was_in_document_tree)
        unregister_from_document_pattern_list();
    else
        register_in_document_pattern_list();
}

void SVGPatternElement::finalize()
{
    Base::finalize();

    // A GC'ed pattern may never run its removal steps, so unlink it here rather than leave the
    // document's list holding a destroyed node.
    unregister_from_document_pattern_list();
}

void SVGPatternElement::register_in_document_pattern_list()
{
    if (m_list_node.is_in_list())
        return;
    document().register_svg_pattern_element({}, *this);
}

void SVGPatternElement::unregister_from_document_pattern_list()
{
    if (!m_list_node.is_in_list())
        return;
    document().unregister_svg_pattern_element({}, *this);
}

Optional<Utf16String> SVGPatternElement::href_attribute_value() const
{
    if (has_attribute(AttributeNames::href))
        return get_attribute(AttributeNames::href);
    return get_attribute(AttributeNames::xlink_href);
}

GC::Ptr<SVGPatternElement const> SVGPatternElement::linked_pattern(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    // FIXME: This can only resolve same-document references. The spec allows cross-document references.
    auto link = href_attribute_value();
    if (!link.has_value() || link->is_empty())
        return {};

    auto url = document().encoding_parse_url(*link);
    if (!url.has_value())
        return {};

    auto id = url->fragment();
    if (!id.has_value() || id->is_empty())
        return {};

    auto element = document().get_element_by_id(decode_fragment_identifier(id.value()));
    if (!element)
        return {};

    if (element == GC::Ref { *this })
        return {};
    auto* pattern = as_if<SVGPatternElement>(*element);
    if (!pattern)
        return {};

    // Detect circular references in the template chain.
    if (seen_patterns.set(pattern) != AK::HashSetResult::InsertedNewEntry)
        return {};

    return pattern;
}

GC::Ptr<SVGPatternElement const> SVGPatternElement::pattern_content_element() const
{
    GC::RootHashTable<SVGPatternElement const*> seen_patterns;
    return pattern_content_element_impl(seen_patterns);
}

GC::Ptr<SVGPatternElement const> SVGPatternElement::pattern_content_element_impl(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    if (child_element_count() > 0)
        return this;
    if (auto pattern = linked_pattern(seen_patterns))
        return pattern->pattern_content_element_impl(seen_patterns);
    return {};
}

// https://svgwg.org/svg2-draft/pservers.html#PatternElementPatternUnitsAttribute
SVGUnits SVGPatternElement::pattern_units() const
{
    GC::RootHashTable<SVGPatternElement const*> seen_patterns;
    return pattern_units_impl(seen_patterns);
}

SVGUnits SVGPatternElement::pattern_units_impl(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    if (m_pattern_units.has_value())
        return *m_pattern_units;
    if (auto pattern = linked_pattern(seen_patterns))
        return pattern->pattern_units_impl(seen_patterns);
    // Initial value: objectBoundingBox
    return SVGUnits::ObjectBoundingBox;
}

// https://svgwg.org/svg2-draft/pservers.html#PatternElementPatternContentUnitsAttribute
SVGUnits SVGPatternElement::pattern_content_units() const
{
    GC::RootHashTable<SVGPatternElement const*> seen_patterns;
    return pattern_content_units_impl(seen_patterns);
}

SVGUnits SVGPatternElement::pattern_content_units_impl(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    if (m_pattern_content_units.has_value())
        return *m_pattern_content_units;
    if (auto pattern = linked_pattern(seen_patterns))
        return pattern->pattern_content_units_impl(seen_patterns);
    // Initial value: userSpaceOnUse
    return SVGUnits::UserSpaceOnUse;
}

// https://svgwg.org/svg2-draft/pservers.html#PatternElementPatternTransformAttribute
Optional<Gfx::AffineTransform> SVGPatternElement::pattern_transform() const
{
    GC::RootHashTable<SVGPatternElement const*> seen_patterns;
    return pattern_transform_impl(seen_patterns);
}

Optional<Gfx::AffineTransform> SVGPatternElement::pattern_transform_impl(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    if (m_pattern_transform.has_value())
        return m_pattern_transform;
    if (auto pattern = linked_pattern(seen_patterns))
        return pattern->pattern_transform_impl(seen_patterns);
    return {};
}

// https://svgwg.org/svg2-draft/pservers.html#PatternElementXAttribute
NumberPercentage SVGPatternElement::pattern_x() const
{
    GC::RootHashTable<SVGPatternElement const*> seen_patterns;
    return pattern_x_impl(seen_patterns);
}

NumberPercentage SVGPatternElement::pattern_x_impl(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    if (m_x.has_value())
        return *m_x;
    if (auto pattern = linked_pattern(seen_patterns))
        return pattern->pattern_x_impl(seen_patterns);
    return NumberPercentage::create_number(0);
}

// https://svgwg.org/svg2-draft/pservers.html#PatternElementYAttribute
NumberPercentage SVGPatternElement::pattern_y() const
{
    GC::RootHashTable<SVGPatternElement const*> seen_patterns;
    return pattern_y_impl(seen_patterns);
}

NumberPercentage SVGPatternElement::pattern_y_impl(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    if (m_y.has_value())
        return *m_y;
    if (auto pattern = linked_pattern(seen_patterns))
        return pattern->pattern_y_impl(seen_patterns);
    return NumberPercentage::create_number(0);
}

// https://svgwg.org/svg2-draft/pservers.html#PatternElementWidthAttribute
NumberPercentage SVGPatternElement::pattern_width() const
{
    GC::RootHashTable<SVGPatternElement const*> seen_patterns;
    return pattern_width_impl(seen_patterns);
}

NumberPercentage SVGPatternElement::pattern_width_impl(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    if (m_width.has_value())
        return *m_width;
    if (auto pattern = linked_pattern(seen_patterns))
        return pattern->pattern_width_impl(seen_patterns);
    return NumberPercentage::create_number(0);
}

// https://svgwg.org/svg2-draft/pservers.html#PatternElementHeightAttribute
NumberPercentage SVGPatternElement::pattern_height() const
{
    GC::RootHashTable<SVGPatternElement const*> seen_patterns;
    return pattern_height_impl(seen_patterns);
}

NumberPercentage SVGPatternElement::pattern_height_impl(GC::RootHashTable<SVGPatternElement const*>& seen_patterns) const
{
    if (m_height.has_value())
        return *m_height;
    if (auto pattern = linked_pattern(seen_patterns))
        return pattern->pattern_height_impl(seen_patterns);
    return NumberPercentage::create_number(0);
}

// Reflected length accessors are generated by SVGElement's reflection macro.

CSS::ElementBoxKind SVGPatternElement::box_kind() const
{
    return CSS::ElementBoxKind::NoBox;
}

}
