/*
 * Copyright (c) 2020-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2025, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <LibWeb/Bindings/Intrinsics.h>
#include <LibWeb/Bindings/StyleSheetListPrototype.h>
#include <LibWeb/CSS/CSSStyleRule.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/SelectorEngine.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleInvalidationData.h>
#include <LibWeb/CSS/StyleSheetList.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/HTML/HTMLHtmlElement.h>
#include <LibWeb/HTML/HTMLInputElement.h>
#include <LibWeb/HTML/HTMLSelectElement.h>
#include <LibWeb/HTML/HTMLTextAreaElement.h>
#include <LibWeb/HTML/Window.h>

namespace Web::CSS {

GC_DEFINE_ALLOCATOR(StyleSheetList);

// https://www.w3.org/TR/cssom/#remove-a-css-style-sheet
void StyleSheetList::remove_a_css_style_sheet(CSS::CSSStyleSheet& sheet)
{
    // 1. Remove the CSS style sheet from the list of document or shadow root CSS style sheets.
    remove_sheet(sheet);

    // 2. Set the CSS style sheet’s parent CSS style sheet, owner node and owner CSS rule to null.
    sheet.set_parent_css_style_sheet(nullptr);
    sheet.set_owner_node(nullptr);
    sheet.set_owner_css_rule(nullptr);
}

// https://www.w3.org/TR/cssom/#add-a-css-style-sheet
void StyleSheetList::add_a_css_style_sheet(CSS::CSSStyleSheet& sheet)
{
    // 1. Add the CSS style sheet to the list of document or shadow root CSS style sheets at the appropriate location. The remainder of these steps deal with the disabled flag.
    add_sheet(sheet);

    // 2. If the disabled flag is set, then return.
    if (sheet.disabled())
        return;

    // 3. If the title is not the empty string, the alternate flag is unset, and preferred CSS style sheet set name is the empty string change the preferred CSS style sheet set name to the title.
    if (!sheet.title().is_empty() && !sheet.is_alternate() && m_preferred_css_style_sheet_set_name.is_empty()) {
        m_preferred_css_style_sheet_set_name = sheet.title();
    }

    // 4. If any of the following is true, then unset the disabled flag and return:
    //    - The title is the empty string.
    //    - The last CSS style sheet set name is null and the title is a case-sensitive match for the preferred CSS style sheet set name.
    //    - The title is a case-sensitive match for the last CSS style sheet set name.
    // NOTE: We don't enable alternate sheets with an empty title.  This isn't directly mentioned in the algorithm steps, but the
    // HTML specification says that the title element must be specified with a non-empty value for alternative style sheets.
    // See: https://html.spec.whatwg.org/multipage/links.html#the-link-is-an-alternative-stylesheet
    if ((sheet.title().is_empty() && !sheet.is_alternate())
        || (!m_last_css_style_sheet_set_name.has_value() && sheet.title().equals_ignoring_case(m_preferred_css_style_sheet_set_name))
        || (m_last_css_style_sheet_set_name.has_value() && sheet.title().equals_ignoring_case(m_last_css_style_sheet_set_name.value()))) {
        sheet.set_disabled(false);
        return;
    }

    // 5. Set the disabled flag.
    sheet.set_disabled(true);
}

// https://www.w3.org/TR/cssom/#create-a-css-style-sheet
GC::Ref<CSSStyleSheet> StyleSheetList::create_a_css_style_sheet(String const& css_text, String type, DOM::Element* owner_node, String media, String title, Alternate alternate, OriginClean origin_clean, Optional<::URL::URL> location, CSSStyleSheet* parent_style_sheet, CSSRule* owner_rule)
{
    // 1. Create a new CSS style sheet object and set its properties as specified.
    // AD-HOC: The spec never tells us when to parse this style sheet, but the most logical place is here.
    auto sheet = parse_css_stylesheet(Parser::ParsingParams { document() }, css_text, location);

    sheet->set_parent_css_style_sheet(parent_style_sheet);
    sheet->set_owner_css_rule(owner_rule);
    sheet->set_owner_node(owner_node);
    sheet->set_type(move(type));
    sheet->set_media(move(media));
    sheet->set_title(move(title));
    sheet->set_alternate(alternate == Alternate::Yes);
    sheet->set_origin_clean(origin_clean == OriginClean::Yes);

    // 2. Then run the add a CSS style sheet steps for the newly created CSS style sheet.
    add_a_css_style_sheet(*sheet);

    return sheet;
}

struct StylesheetInvalidationSets {
    InvalidationSet direct_set;
    InvalidationSet subtree_root_set;
    Vector<InvalidationSet> specific_direct_sets;
    Vector<InvalidationSet> specific_subtree_root_sets;
    Vector<Selector const*> fallback_selectors;
};

static InvalidationSet build_targeted_stylesheet_invalidation_set(InvalidationSet const& invalidation_set)
{
    InvalidationSet filtered_invalidation_set;
    invalidation_set.for_each_property([&](auto const& property) {
        if (property.type == InvalidationSet::Property::Type::PseudoClass && property.value.template get<PseudoClass>() == PseudoClass::Has)
            return IterationDecision::Continue;

        switch (property.type) {
        case InvalidationSet::Property::Type::InvalidateSelf:
            filtered_invalidation_set.set_needs_invalidate_self();
            break;
        case InvalidationSet::Property::Type::InvalidateWholeSubtree:
            filtered_invalidation_set.set_needs_invalidate_whole_subtree();
            break;
        case InvalidationSet::Property::Type::Class:
            filtered_invalidation_set.set_needs_invalidate_class(property.name());
            break;
        case InvalidationSet::Property::Type::Id:
            filtered_invalidation_set.set_needs_invalidate_id(property.name());
            break;
        case InvalidationSet::Property::Type::TagName:
            filtered_invalidation_set.set_needs_invalidate_tag_name(property.name());
            break;
        case InvalidationSet::Property::Type::Attribute:
            filtered_invalidation_set.set_needs_invalidate_attribute(property.name());
            break;
        case InvalidationSet::Property::Type::PseudoClass:
            filtered_invalidation_set.set_needs_invalidate_pseudo_class(property.value.template get<PseudoClass>());
            break;
        default:
            VERIFY_NOT_REACHED();
        }
        return IterationDecision::Continue;
    });
    return filtered_invalidation_set;
}

static size_t count_registered_stylesheet_invalidation_properties(InvalidationSet const& invalidation_set)
{
    size_t property_count = 0;
    invalidation_set.for_each_property([&](auto const&) {
        ++property_count;
        return IterationDecision::Continue;
    });
    return property_count;
}

static bool invalidation_sets_have_same_properties(InvalidationSet const& a, InvalidationSet const& b)
{
    Vector<InvalidationSet::Property> a_properties;
    a.for_each_property([&](auto const& property) {
        a_properties.append(property);
        return IterationDecision::Continue;
    });

    size_t b_property_count = 0;
    bool has_all_properties = true;
    b.for_each_property([&](auto const& property) {
        ++b_property_count;
        if (!a_properties.contains_slow(property)) {
            has_all_properties = false;
            return IterationDecision::Break;
        }
        return IterationDecision::Continue;
    });

    return has_all_properties && a_properties.size() == b_property_count;
}

static void add_targeted_stylesheet_invalidation_set(InvalidationSet& generic_set, Vector<InvalidationSet>& specific_sets, InvalidationSet const& invalidation_set)
{
    auto targeted_set = build_targeted_stylesheet_invalidation_set(invalidation_set);
    if (!targeted_set.has_properties())
        return;

    if (count_registered_stylesheet_invalidation_properties(targeted_set) <= 1) {
        generic_set.include_all_from(targeted_set);
        return;
    }

    for (auto const& existing_set : specific_sets) {
        if (invalidation_sets_have_same_properties(existing_set, targeted_set))
            return;
    }

    specific_sets.append(move(targeted_set));
}

static bool element_matches_stylesheet_invalidation_property(DOM::Element const& element, InvalidationSet::Property const& property)
{
    switch (property.type) {
    case InvalidationSet::Property::Type::Class:
        return element.has_class(property.name());
    case InvalidationSet::Property::Type::Id:
        return element.id() == property.name();
    case InvalidationSet::Property::Type::TagName:
        return element.local_name() == property.name();
    case InvalidationSet::Property::Type::Attribute:
        if (property.name() == HTML::AttributeNames::id || property.name() == HTML::AttributeNames::class_)
            return true;
        return element.has_attribute(property.name());
    case InvalidationSet::Property::Type::PseudoClass:
        switch (property.value.get<PseudoClass>()) {
        case PseudoClass::Has:
            return false;
        case PseudoClass::Enabled:
            return element.matches_enabled_pseudo_class();
        case PseudoClass::Disabled:
            return element.matches_disabled_pseudo_class();
        case PseudoClass::Defined:
            return element.is_defined();
        case PseudoClass::Checked:
            return element.matches_checked_pseudo_class();
        case PseudoClass::PlaceholderShown:
            return element.matches_placeholder_shown_pseudo_class();
        case PseudoClass::AnyLink:
        case PseudoClass::Link:
            return element.matches_link_pseudo_class();
        case PseudoClass::LocalLink:
            return element.matches_local_link_pseudo_class();
        case PseudoClass::Root:
            return is<HTML::HTMLHtmlElement>(element);
        case PseudoClass::Host:
            return element.is_shadow_host();
        case PseudoClass::Required:
        case PseudoClass::Optional:
            return is<HTML::HTMLInputElement>(element) || is<HTML::HTMLSelectElement>(element) || is<HTML::HTMLTextAreaElement>(element);
        default:
            VERIFY_NOT_REACHED();
        }
    case InvalidationSet::Property::Type::InvalidateSelf:
        return false;
    case InvalidationSet::Property::Type::InvalidateWholeSubtree:
        return true;
    default:
        VERIFY_NOT_REACHED();
    }
}

static bool element_matches_all_properties_in_stylesheet_invalidation_set(DOM::Element const& element, InvalidationSet const& invalidation_set)
{
    bool did_check_property = false;
    bool matches_all = true;
    invalidation_set.for_each_property([&](auto const& property) {
        did_check_property = true;
        if (!element_matches_stylesheet_invalidation_property(element, property)) {
            matches_all = false;
            return IterationDecision::Break;
        }
        return IterationDecision::Continue;
    });
    return did_check_property && matches_all;
}

static bool selector_matches_element_or_pseudo(Selector const& selector, DOM::Element& element)
{
    SelectorEngine::MatchContext context;
    if (SelectorEngine::matches(selector, element, {}, context, {}))
        return true;
    if (SelectorEngine::matches(selector, element, {}, context, PseudoElement::Before))
        return true;
    return SelectorEngine::matches(selector, element, {}, context, PseudoElement::After);
}

static InvalidationSet build_invalidation_set_for_compound_selector(Selector::CompoundSelector const& compound_selector, StyleInvalidationData& throwaway_data)
{
    InvalidationSet set;
    for (auto const& simple : compound_selector.simple_selectors)
        build_invalidation_sets_for_simple_selector(simple, set, ExcludePropertiesNestedInNotPseudoClass::No, throwaway_data, InsideNthChildPseudoClass::No);
    return set;
}

static StylesheetInvalidationSets build_invalidation_sets_for_stylesheet(CSSStyleSheet const& sheet)
{
    StylesheetInvalidationSets sets;
    StyleInvalidationData throwaway_data;
    Optional<String> first_selector_requiring_selector_scan;

    sheet.for_each_effective_style_producing_rule([&](CSSRule const& rule) {
        if (!is<CSSStyleRule>(rule))
            return;

        auto const& style_rule = as<CSSStyleRule>(rule);
        for (auto const& selector : style_rule.absolutized_selectors()) {
            auto const& compound_selectors = selector->compound_selectors();
            if (compound_selectors.is_empty())
                continue;

            auto const& rightmost = compound_selectors.last();
            auto rightmost_set = build_targeted_stylesheet_invalidation_set(build_invalidation_set_for_compound_selector(rightmost, throwaway_data));

            if (!rightmost_set.has_properties()) {
                bool found_subtree_root = false;
                for (ssize_t compound_index = static_cast<ssize_t>(compound_selectors.size()) - 2; compound_index >= 0; --compound_index) {
                    auto candidate_set = build_targeted_stylesheet_invalidation_set(build_invalidation_set_for_compound_selector(compound_selectors[compound_index], throwaway_data));
                    if (!candidate_set.has_properties())
                        continue;

                    add_targeted_stylesheet_invalidation_set(sets.subtree_root_set, sets.specific_subtree_root_sets, candidate_set);
                    found_subtree_root = true;
                    break;
                }

                if (found_subtree_root)
                    continue;

                if constexpr (LAYOUT_THRASH_DEBUG) {
                    if (!first_selector_requiring_selector_scan.has_value())
                        first_selector_requiring_selector_scan = selector->serialize();
                }
                sets.fallback_selectors.append(selector.ptr());
                continue;
            }

            add_targeted_stylesheet_invalidation_set(sets.direct_set, sets.specific_direct_sets, rightmost_set);
        }
    });

    if constexpr (LAYOUT_THRASH_DEBUG) {
        if (!sets.fallback_selectors.is_empty()) {
            auto* owner_node = const_cast<CSSStyleSheet&>(sheet).owner_node();
            dbgln("Stylesheet invalidation falls back to selector scan: owner={} first_selector={}",
                owner_node ? owner_node->debug_description() : "<no owner>"sv,
                first_selector_requiring_selector_scan.value_or("<unknown>"_string));
        }
    }

    return sets;
}

static void invalidate_elements_matching_stylesheet_invalidation_sets(DOM::Node& root, StylesheetInvalidationSets const& sets)
{
    root.for_each_in_inclusive_subtree_of_type<DOM::Element>([&](DOM::Element& element) {
        bool matches_subtree_root_set = !sets.subtree_root_set.is_empty() && element.includes_properties_from_invalidation_set(sets.subtree_root_set);
        if (!matches_subtree_root_set) {
            matches_subtree_root_set = any_of(sets.specific_subtree_root_sets, [&](auto const& invalidation_set) {
                return element_matches_all_properties_in_stylesheet_invalidation_set(element, invalidation_set);
            });
        }
        if (matches_subtree_root_set) {
            element.set_needs_style_update(true);
            element.set_entire_subtree_needs_style_update(true);
        }

        bool matches_direct_set = !sets.direct_set.is_empty() && element.includes_properties_from_invalidation_set(sets.direct_set);
        if (!matches_direct_set) {
            matches_direct_set = any_of(sets.specific_direct_sets, [&](auto const& invalidation_set) {
                return element_matches_all_properties_in_stylesheet_invalidation_set(element, invalidation_set);
            });
        }
        if (matches_direct_set)
            element.set_needs_style_update(true);

        if (!sets.fallback_selectors.is_empty() && !element.needs_style_update()) {
            for (auto const* selector : sets.fallback_selectors) {
                if (!selector_matches_element_or_pseudo(*selector, element))
                    continue;
                element.set_needs_style_update(true);
                break;
            }
        }
        return TraversalDecision::Continue;
    });
}

static void invalidate_after_stylesheet_list_change(DOM::Node& document_or_shadow_root, CSSStyleSheet const& sheet, DOM::StyleInvalidationReason reason)
{
    if (sheet.rules().length() == 0)
        return;

    auto invalidate_root = [&](DOM::Node& root, bool is_shadow_host_for_shadow_stylesheet) {
        if (root.entire_subtree_needs_style_update() || root.document().needs_full_style_update())
            return;

        auto invalidation_sets = build_invalidation_sets_for_stylesheet(sheet);
        if (is_shadow_host_for_shadow_stylesheet && !invalidation_sets.fallback_selectors.is_empty())
            root.invalidate_style(reason);
        else
            invalidate_elements_matching_stylesheet_invalidation_sets(root, invalidation_sets);
    };

    if (auto* shadow_root = as_if<DOM::ShadowRoot>(document_or_shadow_root)) {
        shadow_root->style_scope().invalidate_rule_cache();
        if (auto* host = shadow_root->host())
            invalidate_root(*host, true);
        return;
    }

    document_or_shadow_root.document().style_scope().invalidate_rule_cache();
    invalidate_root(document_or_shadow_root, false);
}

void StyleSheetList::add_sheet(CSSStyleSheet& sheet)
{
    sheet.add_owning_document_or_shadow_root(document_or_shadow_root());

    sheet.load_pending_image_resources(document());

    if (m_sheets.is_empty()) {
        // This is the first sheet, append it to the list.
        m_sheets.append(sheet);
    } else {
        // We have sheets from before. Insert the new sheet in the correct position (DOM tree order).
        bool did_insert = false;
        for (ssize_t i = m_sheets.size() - 1; i >= 0; --i) {
            auto& existing_sheet = *m_sheets[i];
            auto position = existing_sheet.owner_node()->compare_document_position(sheet.owner_node());
            if (position & DOM::Node::DocumentPosition::DOCUMENT_POSITION_FOLLOWING) {
                m_sheets.insert(i + 1, sheet);
                did_insert = true;
                break;
            }
        }
        if (!did_insert)
            m_sheets.prepend(sheet);
    }

    // NOTE: We evaluate media queries immediately when adding a new sheet.
    //       This coalesces the full document style invalidations.
    //       If we don't do this, we invalidate now, and then again when Document updates media rules.
    sheet.evaluate_media_queries(document());

    invalidate_after_stylesheet_list_change(document_or_shadow_root(), sheet, DOM::StyleInvalidationReason::StyleSheetListAddSheet);
}

void StyleSheetList::remove_sheet(CSSStyleSheet& sheet)
{
    sheet.remove_owning_document_or_shadow_root(document_or_shadow_root());
    bool did_remove = m_sheets.remove_first_matching([&](auto& entry) { return entry.ptr() == &sheet; });
    VERIFY(did_remove);

    invalidate_after_stylesheet_list_change(document_or_shadow_root(), sheet, DOM::StyleInvalidationReason::StyleSheetListRemoveSheet);
}

GC::Ref<StyleSheetList> StyleSheetList::create(GC::Ref<DOM::Node> document_or_shadow_root)
{
    auto& realm = document_or_shadow_root->realm();
    return realm.create<StyleSheetList>(document_or_shadow_root);
}

StyleSheetList::StyleSheetList(GC::Ref<DOM::Node> document_or_shadow_root)
    : Bindings::PlatformObject(document_or_shadow_root->realm())
    , m_document_or_shadow_root(document_or_shadow_root)
{
    m_legacy_platform_object_flags = LegacyPlatformObjectFlags { .supports_indexed_properties = true };
}

void StyleSheetList::initialize(JS::Realm& realm)
{
    WEB_SET_PROTOTYPE_FOR_INTERFACE(StyleSheetList);
    Base::initialize(realm);
}

void StyleSheetList::visit_edges(Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_document_or_shadow_root);
    visitor.visit(m_sheets);
}

Optional<JS::Value> StyleSheetList::item_value(size_t index) const
{
    if (index >= m_sheets.size())
        return {};

    return m_sheets[index].ptr();
}

DOM::Document& StyleSheetList::document()
{
    return m_document_or_shadow_root->document();
}

DOM::Document const& StyleSheetList::document() const
{
    return m_document_or_shadow_root->document();
}

}
