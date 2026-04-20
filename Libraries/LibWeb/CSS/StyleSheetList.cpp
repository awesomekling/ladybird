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
#include <LibWeb/DOM/AbstractElement.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/StyleInvalidator.h>
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

struct StylesheetInvalidationRule {
    InvalidationSet anchor_match_set;
    bool anchor_matches_any { false };
    NonnullRefPtr<InvalidationPlan> payload;
};

struct StylesheetInvalidation {
    Vector<Selector const*> direct_selectors;
    Vector<StylesheetInvalidationRule> anchor_rules;
    bool invalidate_whole_subtree { false };
};

static bool simple_selector_group_matches_any(Vector<Selector::SimpleSelector> const& simple_selectors)
{
    return simple_selectors.size() == 1 && simple_selectors.first().type == Selector::SimpleSelector::Type::Universal;
}

static bool simple_selector_group_matches_any_element(Vector<Selector::SimpleSelector> const& simple_selectors)
{
    if (simple_selector_group_matches_any(simple_selectors))
        return true;
    return !simple_selectors.is_empty() && all_of(simple_selectors, [](auto const& simple_selector) {
        return simple_selector.type == Selector::SimpleSelector::Type::Universal
            || simple_selector.type == Selector::SimpleSelector::Type::PseudoElement;
    });
}

static InvalidationSet build_subject_match_set_for_simple_selectors(Vector<Selector::SimpleSelector> const& simple_selectors, StyleInvalidationData& throwaway_data)
{
    InvalidationSet set;
    for (auto const& simple : simple_selectors)
        build_invalidation_sets_for_simple_selector(simple, set, ExcludePropertiesNestedInNotPseudoClass::Yes, throwaway_data, InsideNthChildPseudoClass::No);
    return set;
}

static NonnullRefPtr<InvalidationPlan> make_invalidate_self_invalidation()
{
    auto invalidation = InvalidationPlan::create();
    invalidation->invalidate_self = true;
    return invalidation;
}

static NonnullRefPtr<InvalidationPlan> build_invalidation_for_combinator(Selector::Combinator combinator, InvalidationSet const& subject_match_set, bool subject_matches_any)
{
    if (!subject_matches_any && subject_match_set.is_empty()) {
        auto invalidation = InvalidationPlan::create();
        invalidation->invalidate_whole_subtree = true;
        return invalidation;
    }

    auto invalidation = InvalidationPlan::create();
    auto payload = make_invalidate_self_invalidation();
    switch (combinator) {
    case Selector::Combinator::ImmediateChild:
    case Selector::Combinator::Descendant:
        invalidation->descendant_rules.append({ subject_match_set, subject_matches_any, payload });
        break;
    case Selector::Combinator::NextSibling:
        invalidation->sibling_rules.append({ SiblingInvalidationReach::Adjacent, subject_match_set, subject_matches_any, payload });
        break;
    case Selector::Combinator::SubsequentSibling:
        invalidation->sibling_rules.append({ SiblingInvalidationReach::Subsequent, subject_match_set, subject_matches_any, payload });
        break;
    default:
        invalidation->invalidate_whole_subtree = true;
        break;
    }
    return invalidation;
}

static StylesheetInvalidation build_invalidation_for_stylesheet(CSSStyleSheet const& sheet)
{
    StylesheetInvalidation invalidation;
    StyleInvalidationData throwaway_data;

    sheet.for_each_effective_style_producing_rule([&](CSSRule const& rule) {
        if (!is<CSSStyleRule>(rule))
            return;

        auto const& style_rule = as<CSSStyleRule>(rule);
        for (auto const& selector : style_rule.absolutized_selectors()) {
            auto const& compound_selectors = selector->compound_selectors();
            if (compound_selectors.is_empty())
                continue;

            auto const& rightmost = compound_selectors.last();
            auto rightmost_subject_match_set = build_subject_match_set_for_simple_selectors(rightmost.simple_selectors, throwaway_data);
            bool rightmost_matches_any = simple_selector_group_matches_any_element(rightmost.simple_selectors);

            if (compound_selectors.size() == 1) {
                if (rightmost_matches_any || !rightmost_subject_match_set.is_empty()) {
                    invalidation.direct_selectors.append(selector.ptr());
                    continue;
                }
            } else {
                auto const& anchor = compound_selectors[compound_selectors.size() - 2];
                auto anchor_match_set = build_subject_match_set_for_simple_selectors(anchor.simple_selectors, throwaway_data);
                bool anchor_matches_any = simple_selector_group_matches_any(anchor.simple_selectors);
                auto payload = build_invalidation_for_combinator(rightmost.combinator, rightmost_subject_match_set, rightmost_matches_any);
                if ((anchor_matches_any || anchor_match_set.has_properties()) && !payload->invalidate_whole_subtree) {
                    invalidation.anchor_rules.append({
                        .anchor_match_set = move(anchor_match_set),
                        .anchor_matches_any = anchor_matches_any,
                        .payload = move(payload),
                    });
                    continue;
                }
            }

            invalidation.invalidate_whole_subtree = true;
            return;
        }
    });

    return invalidation;
}

static void enqueue_stylesheet_invalidation_rule(DOM::Node& root, StylesheetInvalidationRule const& rule)
{
    auto& document = root.document();
    root.for_each_in_inclusive_subtree_of_type<DOM::Element>([&](DOM::Element& element) {
        if (!rule.anchor_matches_any && !element.includes_properties_from_invalidation_set(rule.anchor_match_set))
            return TraversalDecision::Continue;
        (void)document.style_invalidator().enqueue_invalidation_plan(element, DOM::StyleInvalidationReason::StyleSheetListAddSheet, *rule.payload);
        return TraversalDecision::Continue;
    });
}

static void invalidate_elements_matching_selector(DOM::Node& root, Selector const& selector)
{
    SelectorEngine::MatchContext context;
    Optional<CSS::PseudoElement> pseudo_element;
    if (selector.target_pseudo_element().has_value())
        pseudo_element = selector.target_pseudo_element()->type();

    root.for_each_in_inclusive_subtree_of_type<DOM::Element>([&](DOM::Element& element) {
        DOM::AbstractElement abstract_element { element, pseudo_element };
        if (SelectorEngine::matches(selector, abstract_element, nullptr, context))
            element.set_needs_style_update(true);
        return TraversalDecision::Continue;
    });
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

    if (sheet.rules().length() == 0) {
        // NOTE: If the added sheet has no rules, we don't have to invalidate anything.
        return;
    }

    if (auto* shadow_root = as_if<DOM::ShadowRoot>(document_or_shadow_root())) {
        shadow_root->style_scope().invalidate_rule_cache();
    } else {
        document_or_shadow_root().document().style_scope().invalidate_rule_cache();
    }

    auto styles_have_been_computed_for_scope = [&]() {
        if (auto* shadow_root = as_if<DOM::ShadowRoot>(document_or_shadow_root())) {
            auto* host = shadow_root->host();
            return host && host->computed_properties();
        }
        auto* document_element = document().document_element();
        return document_element && document_element->computed_properties();
    };

    if (!styles_have_been_computed_for_scope()) {
        // The first full style pass will already compute styles using this sheet.
        // Avoid enqueuing extra targeted invalidations before any style exists.
        return;
    }

    if (document_or_shadow_root().entire_subtree_needs_style_update()) {
        // NOTE: If the entire subtree is already marked for style update,
        //       there's no point spending time building invalidation sets.
        return;
    }

    auto invalidation = build_invalidation_for_stylesheet(sheet);
    if (auto* shadow_root = as_if<DOM::ShadowRoot>(document_or_shadow_root())) {
        if (auto* host = shadow_root->host()) {
            if (invalidation.invalidate_whole_subtree) {
                host->invalidate_style(DOM::StyleInvalidationReason::StyleSheetListAddSheet);
            } else {
                for (auto const* selector : invalidation.direct_selectors)
                    invalidate_elements_matching_selector(*host, *selector);
                for (auto const& rule : invalidation.anchor_rules)
                    enqueue_stylesheet_invalidation_rule(*host, rule);
            }
        }
    } else {
        if (invalidation.invalidate_whole_subtree) {
            document_or_shadow_root().invalidate_style(DOM::StyleInvalidationReason::StyleSheetListAddSheet);
        } else {
            for (auto const* selector : invalidation.direct_selectors)
                invalidate_elements_matching_selector(document_or_shadow_root(), *selector);
            for (auto const& rule : invalidation.anchor_rules)
                enqueue_stylesheet_invalidation_rule(document_or_shadow_root(), rule);
        }
    }
}

void StyleSheetList::remove_sheet(CSSStyleSheet& sheet)
{
    sheet.remove_owning_document_or_shadow_root(document_or_shadow_root());
    bool did_remove = m_sheets.remove_first_matching([&](auto& entry) { return entry.ptr() == &sheet; });
    VERIFY(did_remove);

    if (sheet.rules().length() == 0) {
        // NOTE: If the removed sheet had no rules, we don't have to invalidate anything.
        return;
    }

    if (auto* shadow_root = as_if<DOM::ShadowRoot>(document_or_shadow_root())) {
        if (auto* host = shadow_root->host()) {
            host->invalidate_style(DOM::StyleInvalidationReason::StyleSheetListRemoveSheet);
        }
        shadow_root->style_scope().invalidate_rule_cache();
    } else {
        document_or_shadow_root().invalidate_style(DOM::StyleInvalidationReason::StyleSheetListRemoveSheet);
        document_or_shadow_root().document().style_scope().invalidate_rule_cache();
    }
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
