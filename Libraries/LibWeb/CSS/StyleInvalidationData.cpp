/*
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Debug.h>
#include <AK/GenericShorthands.h>
#include <AK/Optional.h>
#include <AK/StringBuilder.h>
#include <LibWeb/CSS/Selector.h>
#include <LibWeb/CSS/StyleInvalidationData.h>
#include <LibWeb/HTML/AttributeNames.h>

namespace Web::CSS {

bool InvalidationPlan::is_empty() const
{
    return !invalidate_self && !invalidate_whole_subtree && descendant_rules.is_empty() && sibling_rules.is_empty();
}

void InvalidationPlan::include_all_from(InvalidationPlan const& other)
{
    invalidate_self |= other.invalidate_self;

    if (invalidate_whole_subtree)
        return;

    if (other.invalidate_whole_subtree) {
        invalidate_whole_subtree = true;
        descendant_rules.clear();
        sibling_rules.clear();
        return;
    }

    descendant_rules.extend(other.descendant_rules);
    sibling_rules.extend(other.sibling_rules);
}

// Iterates over the given selector, grouping consecutive simple selectors that have no combinator (Combinator::None).
// For example, given "div:not(.a) + .b[foo]", the callback is invoked twice:
// once for "div:not(.a)" and once for ".b[foo]".
template<typename Callback>
static void for_each_consecutive_simple_selector_group(Selector const& selector, Callback callback)
{
    auto const& compound_selectors = selector.compound_selectors();
    int compound_selector_index = compound_selectors.size() - 1;
    Vector<Selector::SimpleSelector const&> simple_selectors;
    Selector::Combinator combinator = Selector::Combinator::None;
    bool is_rightmost = true;
    while (compound_selector_index >= 0) {
        if (!simple_selectors.is_empty()) {
            callback(simple_selectors, combinator, is_rightmost);
            simple_selectors.clear();
            is_rightmost = false;
        }

        auto const& compound_selector = compound_selectors[compound_selector_index];
        for (auto const& simple_selector : compound_selector.simple_selectors)
            simple_selectors.append(simple_selector);
        combinator = compound_selector.combinator;

        --compound_selector_index;
    }
    if (!simple_selectors.is_empty())
        callback(simple_selectors, combinator, is_rightmost);
}

static Optional<InvalidationSet::Property> precise_class_attribute_invalidation_property_for_selector(Selector::SimpleSelector::Attribute const& attribute)
{
    if (attribute.qualified_name.name.lowercase_name != HTML::AttributeNames::class_)
        return {};
    if (attribute.case_type == Selector::SimpleSelector::Attribute::CaseType::CaseInsensitiveMatch)
        return {};

    auto value = FlyString(attribute.value);
    switch (attribute.match_type) {
    case Selector::SimpleSelector::Attribute::MatchType::HasAttribute:
        return {};
    case Selector::SimpleSelector::Attribute::MatchType::ExactValueMatch:
        return InvalidationSet::Property { InvalidationSet::Property::Type::ClassAttributeExactValue, value };
    case Selector::SimpleSelector::Attribute::MatchType::ContainsWord:
        return InvalidationSet::Property { InvalidationSet::Property::Type::ClassAttributeContainsWord, value };
    case Selector::SimpleSelector::Attribute::MatchType::ContainsString:
        return InvalidationSet::Property { InvalidationSet::Property::Type::ClassAttributeContainsString, value };
    case Selector::SimpleSelector::Attribute::MatchType::StartsWithSegment:
        return InvalidationSet::Property { InvalidationSet::Property::Type::ClassAttributeStartsWithSegment, value };
    case Selector::SimpleSelector::Attribute::MatchType::StartsWithString:
        return InvalidationSet::Property { InvalidationSet::Property::Type::ClassAttributeStartsWithString, value };
    case Selector::SimpleSelector::Attribute::MatchType::EndsWithString:
        return InvalidationSet::Property { InvalidationSet::Property::Type::ClassAttributeEndsWithString, value };
    default:
        VERIFY_NOT_REACHED();
    }
}

static InvalidationSet::Property invalidation_property_for_attribute_selector(Selector::SimpleSelector::Attribute const& attribute)
{
    if (auto property = precise_class_attribute_invalidation_property_for_selector(attribute); property.has_value())
        return *property;
    return { InvalidationSet::Property::Type::Attribute, attribute.qualified_name.name.lowercase_name };
}

static void record_property_used_in_has(InvalidationSet::Property const& property, StyleInvalidationData& style_invalidation_data, InvalidationSet* selector_trigger_properties)
{
    style_invalidation_data.properties_used_in_has_selectors.set(property);
    if (is_precise_class_attribute_invalidation_property(property))
        style_invalidation_data.precise_class_attribute_invalidation_properties.set(property);
    if (selector_trigger_properties)
        selector_trigger_properties->include_property(property);
}

static void collect_properties_used_in_has(Selector::SimpleSelector const& selector, StyleInvalidationData& style_invalidation_data, bool in_has, InvalidationSet* selector_trigger_properties)
{
    switch (selector.type) {
    case Selector::SimpleSelector::Type::Id: {
        if (in_has)
            record_property_used_in_has({ InvalidationSet::Property::Type::Id, selector.name() }, style_invalidation_data, selector_trigger_properties);
        break;
    }
    case Selector::SimpleSelector::Type::Class: {
        if (in_has)
            record_property_used_in_has({ InvalidationSet::Property::Type::Class, selector.name() }, style_invalidation_data, selector_trigger_properties);
        break;
    }
    case Selector::SimpleSelector::Type::Attribute: {
        if (in_has)
            record_property_used_in_has(invalidation_property_for_attribute_selector(selector.attribute()), style_invalidation_data, selector_trigger_properties);
        break;
    }
    case Selector::SimpleSelector::Type::TagName: {
        if (in_has)
            record_property_used_in_has({ InvalidationSet::Property::Type::TagName, selector.qualified_name().name.lowercase_name }, style_invalidation_data, selector_trigger_properties);
        break;
    }
    case Selector::SimpleSelector::Type::PseudoClass: {
        auto const& pseudo_class = selector.pseudo_class();
        switch (pseudo_class.type) {
        case PseudoClass::Enabled:
        case PseudoClass::Disabled:
        case PseudoClass::Defined:
        case PseudoClass::PlaceholderShown:
        case PseudoClass::Checked:
        case PseudoClass::Required:
        case PseudoClass::Optional:
        case PseudoClass::Link:
        case PseudoClass::AnyLink:
        case PseudoClass::LocalLink:
        case PseudoClass::Default:
            if (in_has)
                record_property_used_in_has({ InvalidationSet::Property::Type::PseudoClass, pseudo_class.type }, style_invalidation_data, selector_trigger_properties);
            break;
        default:
            break;
        }
        for (auto const& child_selector : pseudo_class.argument_selector_list) {
            for (auto const& compound_selector : child_selector->compound_selectors()) {
                for (auto const& simple_selector : compound_selector.simple_selectors)
                    collect_properties_used_in_has(simple_selector, style_invalidation_data, in_has || pseudo_class.type == PseudoClass::Has, selector_trigger_properties);
            }
        }
        break;
    }
    default:
        break;
    }
}

static bool simple_selector_contains_has(Selector::SimpleSelector const& selector)
{
    if (selector.type != Selector::SimpleSelector::Type::PseudoClass)
        return false;

    auto const& pseudo_class = selector.pseudo_class();
    if (pseudo_class.type == PseudoClass::Has)
        return true;

    for (auto const& child_selector : pseudo_class.argument_selector_list) {
        if (child_selector->contains_pseudo_class(PseudoClass::Has))
            return true;
    }
    return false;
}

static InvalidationSet build_invalidation_sets_for_selector_impl(StyleInvalidationData& style_invalidation_data, Selector const& selector, InsideNthChildPseudoClass inside_nth_child_pseudo_class);

static void add_invalidation_sets_to_cover_scope_leakage_of_relative_selector_in_has_pseudo_class(Selector const& selector, StyleInvalidationData& style_invalidation_data);

static bool should_register_invalidation_property(InvalidationSet::Property const& property)
{
    return !AK::first_is_one_of(property.type, InvalidationSet::Property::Type::InvalidateSelf, InvalidationSet::Property::Type::InvalidateWholeSubtree);
}

static InvalidationSet build_invalidation_set_for_simple_selectors(Vector<Selector::SimpleSelector const&> const& simple_selectors, ExcludePropertiesNestedInNotPseudoClass exclude_properties_nested_in_not_pseudo_class, StyleInvalidationData& style_invalidation_data, InsideNthChildPseudoClass inside_nth_child_pseudo_class)
{
    InvalidationSet invalidation_set;
    for (auto const& simple_selector : simple_selectors)
        build_invalidation_sets_for_simple_selector(simple_selector, invalidation_set, exclude_properties_nested_in_not_pseudo_class, style_invalidation_data, inside_nth_child_pseudo_class);
    return invalidation_set;
}

static bool simple_selector_group_matches_any(Vector<Selector::SimpleSelector const&> const& simple_selectors)
{
    return simple_selectors.size() == 1 && simple_selectors.first().type == Selector::SimpleSelector::Type::Universal;
}

static NonnullRefPtr<InvalidationPlan> make_invalidate_self_invalidation()
{
    auto invalidation = InvalidationPlan::create();
    invalidation->invalidate_self = true;
    return invalidation;
}

static NonnullRefPtr<InvalidationPlan> make_invalidate_whole_subtree_invalidation()
{
    auto invalidation = InvalidationPlan::create();
    invalidation->invalidate_whole_subtree = true;
    return invalidation;
}

static void add_invalidation_plan_for_properties(HashMap<InvalidationSet::Property, NonnullRefPtr<InvalidationPlan>>& target_invalidation_plans, InvalidationSet const& invalidation_properties, InvalidationPlan const& plan)
{
    invalidation_properties.for_each_property([&](auto const& invalidation_property) {
        if (!should_register_invalidation_property(invalidation_property))
            return IterationDecision::Continue;

        auto& stored_invalidation = target_invalidation_plans.ensure(invalidation_property, [] {
            return InvalidationPlan::create();
        });
        stored_invalidation->include_all_from(plan);
        return IterationDecision::Continue;
    });
}

static void record_debug_selector_for_property_map(HashMap<InvalidationSet::Property, HashTable<String>>& target_debug_selectors, InvalidationSet const& invalidation_properties, Selector const& selector)
{
    if (!LAYOUT_THRASH_DEBUG)
        return;

    Optional<String> selector_text;
    invalidation_properties.for_each_property([&](auto const& invalidation_property) {
        if (!should_register_invalidation_property(invalidation_property))
            return IterationDecision::Continue;

        if (!selector_text.has_value())
            selector_text = selector.serialize();

        auto& selectors = target_debug_selectors.ensure(invalidation_property, [] {
            return HashTable<String> {};
        });
        selectors.set(*selector_text);
        return IterationDecision::Continue;
    });
}

static void record_debug_selector_for_hash_table(HashTable<String>& target_debug_selectors, Selector const& selector)
{
    if (!LAYOUT_THRASH_DEBUG)
        return;

    target_debug_selectors.set(selector.serialize());
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

static bool properties_include_all_from_invalidation_set(Vector<InvalidationSet::Property> const& properties, InvalidationSet const& invalidation_set)
{
    bool matches = true;
    invalidation_set.for_each_property([&](auto const& property) {
        if (!properties.contains_slow(property)) {
            matches = false;
            return IterationDecision::Break;
        }
        return IterationDecision::Continue;
    });
    return matches;
}

static bool properties_intersect_invalidation_set(Vector<InvalidationSet::Property> const& properties, InvalidationSet const& invalidation_set)
{
    bool matches = false;
    invalidation_set.for_each_property([&](auto const& property) {
        if (!properties.contains_slow(property))
            return IterationDecision::Continue;

        matches = true;
        return IterationDecision::Break;
    });
    return matches;
}

static void add_specific_has_invalidation_plan(Vector<SpecificHasInvalidationPlan>& target_invalidation_plans, InvalidationSet const& invalidation_properties, InvalidationPlan const& plan, Selector const& selector)
{
    for (auto& entry : target_invalidation_plans) {
        if (!invalidation_sets_have_same_properties(entry.match_set, invalidation_properties))
            continue;

        entry.plan->include_all_from(plan);
        record_debug_selector_for_hash_table(entry.debug_selectors, selector);
        return;
    }

    auto specific_plan = SpecificHasInvalidationPlan {
        .match_set = invalidation_properties,
        .plan = InvalidationPlan::create(),
        .debug_selectors = {},
    };
    specific_plan.plan->include_all_from(plan);
    record_debug_selector_for_hash_table(specific_plan.debug_selectors, selector);
    target_invalidation_plans.append(move(specific_plan));
}

static void add_triggered_has_invalidation_plan(Vector<TriggeredHasInvalidationPlan>& target_invalidation_plans, InvalidationSet const& trigger_properties, InvalidationSet const& match_properties, InvalidationPlan const& plan, Selector const& selector)
{
    if (!trigger_properties.has_properties())
        return;

    for (auto& entry : target_invalidation_plans) {
        if (!invalidation_sets_have_same_properties(entry.trigger_set, trigger_properties))
            continue;
        if (!invalidation_sets_have_same_properties(entry.match_set, match_properties))
            continue;

        entry.plan->include_all_from(plan);
        record_debug_selector_for_hash_table(entry.debug_selectors, selector);
        return;
    }

    auto triggered_plan = TriggeredHasInvalidationPlan {
        .trigger_set = trigger_properties,
        .match_set = match_properties,
        .plan = InvalidationPlan::create(),
        .debug_selectors = {},
    };
    triggered_plan.plan->include_all_from(plan);
    record_debug_selector_for_hash_table(triggered_plan.debug_selectors, selector);
    target_invalidation_plans.append(move(triggered_plan));
}

void StyleInvalidationData::record_debug_selector_for_properties(InvalidationSet const& invalidation_properties, Selector const& selector)
{
    record_debug_selector_for_property_map(debug_selectors_by_invalidation_property, invalidation_properties, selector);
}

void StyleInvalidationData::record_debug_selector_for_has_subject_properties(InvalidationSet const& invalidation_properties, Selector const& selector)
{
    record_debug_selector_for_property_map(debug_selectors_by_has_subject_invalidation_property, invalidation_properties, selector);
}

void StyleInvalidationData::record_debug_selector_for_has_non_subject_properties(InvalidationSet const& invalidation_properties, Selector const& selector)
{
    record_debug_selector_for_property_map(debug_selectors_by_has_non_subject_invalidation_property, invalidation_properties, selector);
}

void StyleInvalidationData::add_generic_has_subject_invalidation_plan(InvalidationPlan const& plan, Selector const& selector)
{
    generic_has_subject_invalidation_plan->include_all_from(plan);
    record_debug_selector_for_hash_table(debug_selectors_for_generic_has_subject_invalidation, selector);
}

void StyleInvalidationData::add_generic_has_non_subject_invalidation_plan(InvalidationPlan const& plan, Selector const& selector)
{
    generic_has_non_subject_invalidation_plan->include_all_from(plan);
    record_debug_selector_for_hash_table(debug_selectors_for_generic_has_non_subject_invalidation, selector);
}

static String debug_description_for_property_map(HashMap<InvalidationSet::Property, HashTable<String>> const& debug_selectors_by_property, Vector<InvalidationSet::Property> const& properties, size_t max_selectors)
{
    if (!LAYOUT_THRASH_DEBUG)
        return "<none>"_string;

    StringBuilder builder;
    bool first_property = true;
    for (auto const& property : properties) {
        auto selectors = debug_selectors_by_property.get(property);
        if (!selectors.has_value() || selectors->is_empty())
            continue;

        if (!first_property)
            builder.append("; "sv);
        first_property = false;

        builder.appendff("{}: count={} selectors=[", property, selectors->size());

        size_t appended_selectors = 0;
        for (auto const& selector_text : *selectors) {
            if (appended_selectors > 0)
                builder.append(" | "sv);
            builder.append(selector_text);
            ++appended_selectors;
            if (appended_selectors >= max_selectors)
                break;
        }

        if (selectors->size() > appended_selectors)
            builder.appendff(" | ... +{}", selectors->size() - appended_selectors);
        builder.append(']');
    }

    if (first_property)
        return "<none>"_string;
    return MUST(builder.to_string());
}

static String debug_description_for_selector_hash_table(HashTable<String> const& selectors, StringView label, size_t max_selectors)
{
    if (!LAYOUT_THRASH_DEBUG)
        return "<none>"_string;
    if (selectors.is_empty())
        return "<none>"_string;

    StringBuilder builder;
    builder.appendff("{}: count={} selectors=[", label, selectors.size());

    size_t appended_selectors = 0;
    for (auto const& selector_text : selectors) {
        if (appended_selectors > 0)
            builder.append(" | "sv);
        builder.append(selector_text);
        ++appended_selectors;
        if (appended_selectors >= max_selectors)
            break;
    }

    if (selectors.size() > appended_selectors)
        builder.appendff(" | ... +{}", selectors.size() - appended_selectors);
    builder.append(']');
    return MUST(builder.to_string());
}

static String debug_description_for_matching_specific_has_invalidation_plans(Vector<SpecificHasInvalidationPlan> const& plans, Vector<InvalidationSet::Property> const& properties, StringView label, size_t max_selectors)
{
    HashTable<String> matching_selectors;
    for (auto const& plan : plans) {
        if (!properties_include_all_from_invalidation_set(properties, plan.match_set))
            continue;

        for (auto const& selector_text : plan.debug_selectors)
            matching_selectors.set(selector_text);
    }

    return debug_description_for_selector_hash_table(matching_selectors, label, max_selectors);
}

static String debug_description_for_matching_triggered_has_invalidation_plans(Vector<TriggeredHasInvalidationPlan> const& plans, Vector<InvalidationSet::Property> const& properties, Vector<InvalidationSet::Property> const& trigger_properties, StringView label, size_t max_selectors)
{
    HashTable<String> matching_selectors;
    for (auto const& plan : plans) {
        if (!properties_include_all_from_invalidation_set(properties, plan.match_set))
            continue;
        if (!properties_intersect_invalidation_set(trigger_properties, plan.trigger_set))
            continue;

        for (auto const& selector_text : plan.debug_selectors)
            matching_selectors.set(selector_text);
    }

    return debug_description_for_selector_hash_table(matching_selectors, label, max_selectors);
}

String StyleInvalidationData::debug_description_for_properties(Vector<InvalidationSet::Property> const& properties, size_t max_selectors) const
{
    return debug_description_for_property_map(debug_selectors_by_invalidation_property, properties, max_selectors);
}

String StyleInvalidationData::debug_description_for_has_subject_properties(Vector<InvalidationSet::Property> const& properties, size_t max_selectors) const
{
    auto description = debug_description_for_matching_specific_has_invalidation_plans(specific_has_subject_invalidation_plans, properties, "<specific-subject-has>"sv, max_selectors);
    auto generic_description = debug_description_for_selector_hash_table(debug_selectors_for_generic_has_subject_invalidation, "<generic-subject-has>"sv, max_selectors);
    if (description == "<none>"sv)
        return generic_description;
    if (generic_description == "<none>"sv)
        return description;
    return MUST(String::formatted("{}; {}", description, generic_description));
}

String StyleInvalidationData::debug_description_for_has_non_subject_properties(Vector<InvalidationSet::Property> const& properties, size_t max_selectors) const
{
    auto description = debug_description_for_matching_specific_has_invalidation_plans(specific_has_non_subject_invalidation_plans, properties, "<specific-non-subject-has>"sv, max_selectors);
    auto generic_description = debug_description_for_selector_hash_table(debug_selectors_for_generic_has_non_subject_invalidation, "<generic-non-subject-has>"sv, max_selectors);
    if (description == "<none>"sv)
        return generic_description;
    if (generic_description == "<none>"sv)
        return description;
    return MUST(String::formatted("{}; {}", description, generic_description));
}

String StyleInvalidationData::debug_description_for_triggered_has_subject_properties(Vector<InvalidationSet::Property> const& properties, Vector<InvalidationSet::Property> const& trigger_properties, size_t max_selectors) const
{
    return debug_description_for_matching_triggered_has_invalidation_plans(triggered_has_subject_invalidation_plans, properties, trigger_properties, "<triggered-subject-has>"sv, max_selectors);
}

String StyleInvalidationData::debug_description_for_triggered_has_non_subject_properties(Vector<InvalidationSet::Property> const& properties, Vector<InvalidationSet::Property> const& trigger_properties, size_t max_selectors) const
{
    return debug_description_for_matching_triggered_has_invalidation_plans(triggered_has_non_subject_invalidation_plans, properties, trigger_properties, "<triggered-non-subject-has>"sv, max_selectors);
}

struct SelectorRighthand {
    InvalidationSet subject_match_set;
    bool subject_matches_any { false };
    NonnullRefPtr<InvalidationPlan> payload;
};

static NonnullRefPtr<InvalidationPlan> build_invalidation_for_combinator(Selector::Combinator combinator, SelectorRighthand const& righthand)
{
    if (righthand.payload->invalidate_whole_subtree || (!righthand.subject_matches_any && righthand.subject_match_set.is_empty()))
        return make_invalidate_whole_subtree_invalidation();

    auto invalidation = InvalidationPlan::create();
    switch (combinator) {
    case Selector::Combinator::ImmediateChild:
    case Selector::Combinator::Descendant:
        invalidation->descendant_rules.append({ righthand.subject_match_set, righthand.subject_matches_any, righthand.payload });
        break;
    case Selector::Combinator::NextSibling:
        invalidation->sibling_rules.append({ SiblingInvalidationReach::Adjacent, righthand.subject_match_set, righthand.subject_matches_any, righthand.payload });
        break;
    case Selector::Combinator::SubsequentSibling:
        invalidation->sibling_rules.append({ SiblingInvalidationReach::Subsequent, righthand.subject_match_set, righthand.subject_matches_any, righthand.payload });
        break;
    default:
        invalidation->invalidate_whole_subtree = true;
        break;
    }
    return invalidation;
}

void build_invalidation_sets_for_simple_selector(Selector::SimpleSelector const& selector, InvalidationSet& invalidation_set, ExcludePropertiesNestedInNotPseudoClass exclude_properties_nested_in_not_pseudo_class, StyleInvalidationData& style_invalidation_data, InsideNthChildPseudoClass inside_nth_child_selector)
{
    switch (selector.type) {
    case Selector::SimpleSelector::Type::Class:
        invalidation_set.set_needs_invalidate_class(selector.name());
        break;
    case Selector::SimpleSelector::Type::Id:
        invalidation_set.set_needs_invalidate_id(selector.name());
        break;
    case Selector::SimpleSelector::Type::TagName:
        invalidation_set.set_needs_invalidate_tag_name(selector.qualified_name().name.lowercase_name);
        break;
    case Selector::SimpleSelector::Type::Attribute:
        invalidation_set.include_property(invalidation_property_for_attribute_selector(selector.attribute()));
        if (auto property = precise_class_attribute_invalidation_property_for_selector(selector.attribute()); property.has_value())
            style_invalidation_data.precise_class_attribute_invalidation_properties.set(*property);
        break;
    case Selector::SimpleSelector::Type::PseudoClass: {
        auto const& pseudo_class = selector.pseudo_class();
        switch (pseudo_class.type) {
        case PseudoClass::Enabled:
        case PseudoClass::Defined:
        case PseudoClass::Disabled:
        case PseudoClass::PlaceholderShown:
        case PseudoClass::Checked:
        case PseudoClass::Has: {
            for (auto const& nested_selector : pseudo_class.argument_selector_list)
                add_invalidation_sets_to_cover_scope_leakage_of_relative_selector_in_has_pseudo_class(*nested_selector, style_invalidation_data);
            [[fallthrough]];
        }
        case PseudoClass::Link:
        case PseudoClass::AnyLink:
        case PseudoClass::LocalLink:
        case PseudoClass::Required:
        case PseudoClass::Optional:
            invalidation_set.set_needs_invalidate_pseudo_class(pseudo_class.type);
            break;
        default:
            break;
        }
        if (pseudo_class.type == PseudoClass::Has)
            break;
        if (exclude_properties_nested_in_not_pseudo_class == ExcludePropertiesNestedInNotPseudoClass::Yes && pseudo_class.type == PseudoClass::Not)
            break;
        InsideNthChildPseudoClass inside_nth_child_pseudo_class_for_nested = inside_nth_child_selector;
        if (AK::first_is_one_of(pseudo_class.type, PseudoClass::NthChild, PseudoClass::NthLastChild, PseudoClass::NthOfType, PseudoClass::NthLastOfType))
            inside_nth_child_pseudo_class_for_nested = InsideNthChildPseudoClass::Yes;
        for (auto const& nested_selector : pseudo_class.argument_selector_list) {
            auto rightmost_invalidation_set_for_selector = build_invalidation_sets_for_selector_impl(style_invalidation_data, *nested_selector, inside_nth_child_pseudo_class_for_nested);
            invalidation_set.include_all_from(rightmost_invalidation_set_for_selector);

            // Propagate :has() from inner selectors where it appears in non-rightmost compounds.
            // The rightmost set only carries properties from the rightmost compound, so :has() in
            // non-rightmost positions (e.g., :is(:has(.x) .y)) is not propagated. We need it in the
            // outer invalidation set so outer compounds register plans for pseudo_class:Has that
            // account for the full selector context.
            // Additionally, when :has() is inside a complex :is()/:where() argument (multiple
            // compounds), the outer invalidation plan can't correctly capture the nested combinator
            // structure — e.g., sibling combinators at the outer level would be applied at the wrong
            // DOM level. Fall back to whole-subtree invalidation for :has() in these cases.
            if (nested_selector->contains_pseudo_class(PseudoClass::Has)) {
                invalidation_set.set_needs_invalidate_pseudo_class(PseudoClass::Has);
                if (nested_selector->compound_selectors().size() > 1) {
                    InvalidationSet has_only;
                    has_only.set_needs_invalidate_pseudo_class(PseudoClass::Has);
                    add_invalidation_plan_for_properties(style_invalidation_data.invalidation_plans, has_only, *make_invalidate_whole_subtree_invalidation());
                    style_invalidation_data.record_debug_selector_for_properties(has_only, *nested_selector);
                }
            }
        }
        break;
    }
    default:
        break;
    }
}

static void add_invalidation_sets_to_cover_scope_leakage_of_relative_selector_in_has_pseudo_class(Selector const& selector, StyleInvalidationData& style_invalidation_data)
{
    // Normally, :has() invalidation scope is limited to ancestors and ancestor siblings, however it could require
    // descendants invalidation when :is() with complex selector is used inside :has() relative selector.
    // For example ".a:has(:is(.b .c))" requires invalidation whenever "b" class is added or removed.
    // To cover this case, we add descendant invalidation set that requires whole subtree invalidation for each
    // property used in non-subject part of complex selector.

    auto invalidate_whole_subtree_for_invalidation_properties_in_non_subject_part_of_complex_selector = [&](Selector const& selector_to_invalidate) {
        for_each_consecutive_simple_selector_group(selector_to_invalidate, [&](Vector<Selector::SimpleSelector const&> const& simple_selectors, Selector::Combinator, bool rightmost) {
            if (rightmost)
                return;

            auto invalidation_set = build_invalidation_set_for_simple_selectors(simple_selectors, ExcludePropertiesNestedInNotPseudoClass::No, style_invalidation_data, InsideNthChildPseudoClass::No);
            add_invalidation_plan_for_properties(style_invalidation_data.invalidation_plans, invalidation_set, *make_invalidate_whole_subtree_invalidation());
            style_invalidation_data.record_debug_selector_for_properties(invalidation_set, selector_to_invalidate);
        });
    };

    for_each_consecutive_simple_selector_group(selector, [&](Vector<Selector::SimpleSelector const&> const& simple_selectors, Selector::Combinator, bool) {
        for (auto const& simple_selector : simple_selectors) {
            if (simple_selector.type != Selector::SimpleSelector::Type::PseudoClass)
                continue;
            auto const& pseudo_class = simple_selector.pseudo_class();
            if (pseudo_class.type == PseudoClass::Is || pseudo_class.type == PseudoClass::Where || pseudo_class.type == PseudoClass::Not) {
                for (auto const& nested_selector : pseudo_class.argument_selector_list)
                    invalidate_whole_subtree_for_invalidation_properties_in_non_subject_part_of_complex_selector(*nested_selector);
            }
        }
    });
}

static InvalidationSet build_specific_has_subject_invalidation_set(InvalidationSet const& invalidation_set)
{
    InvalidationSet filtered_invalidation_set;
    invalidation_set.for_each_property([&](auto const& property) {
        if (property.type == InvalidationSet::Property::Type::PseudoClass && property.value.template get<PseudoClass>() == PseudoClass::Has)
            return IterationDecision::Continue;

        filtered_invalidation_set.include_property(property);

        return IterationDecision::Continue;
    });
    return filtered_invalidation_set;
}

static InvalidationSet build_invalidation_sets_for_selector_impl(StyleInvalidationData& style_invalidation_data, Selector const& selector, InsideNthChildPseudoClass inside_nth_child_pseudo_class)
{
    bool const selector_contains_has = selector.contains_pseudo_class(PseudoClass::Has);
    auto const& compound_selectors = selector.compound_selectors();
    int compound_selector_index = compound_selectors.size() - 1;
    VERIFY(compound_selector_index >= 0);

    InvalidationSet invalidation_set_for_rightmost_selector;
    InvalidationSet selector_has_trigger_properties;
    Selector::Combinator previous_compound_combinator = Selector::Combinator::None;
    Optional<SelectorRighthand> selector_righthand;
    size_t processed_simple_selector_group_count = 0;
    for_each_consecutive_simple_selector_group(selector, [&](Vector<Selector::SimpleSelector const&> const& simple_selectors, Selector::Combinator combinator, bool is_rightmost) {
        bool const has_left_hand_context = processed_simple_selector_group_count + 1 < compound_selectors.size();

        // Collect properties used in :has() so we can decide if only specific properties
        // trigger descendant invalidation or if the entire document must be invalidated.
        bool simple_selector_group_contains_has = false;
        for (auto const& simple_selector : simple_selectors) {
            bool const simple_selector_has = simple_selector_contains_has(simple_selector);
            bool in_has = false;
            if (simple_selector.type == Selector::SimpleSelector::Type::PseudoClass && simple_selector.pseudo_class().type == PseudoClass::Has)
                in_has = true;
            simple_selector_group_contains_has |= simple_selector_has;
            collect_properties_used_in_has(simple_selector, style_invalidation_data, in_has, &selector_has_trigger_properties);
        }

        auto invalidation_properties = build_invalidation_set_for_simple_selectors(simple_selectors, ExcludePropertiesNestedInNotPseudoClass::No, style_invalidation_data, inside_nth_child_pseudo_class);
        auto specific_has_subject_invalidation_properties = build_specific_has_subject_invalidation_set(invalidation_properties);
        auto subject_match_set = build_invalidation_set_for_simple_selectors(simple_selectors, ExcludePropertiesNestedInNotPseudoClass::Yes, style_invalidation_data, inside_nth_child_pseudo_class);
        bool subject_matches_any = subject_match_set.is_empty() && simple_selector_group_matches_any(simple_selectors);

        if (is_rightmost) {
            // The rightmost selector is handled twice:
            //  1) Include properties nested in :not()
            //  2) Exclude properties nested in :not()
            //
            // This ensures we handle cases like:
            //   :not(.foo) => produce invalidation set .foo { $ } ($ = invalidate self)
            //   .bar :not(.foo) => produce invalidation sets .foo { $ } and .bar { * } (* = invalidate subtree)
            //                      which means invalidation_set_for_rightmost_selector should be empty
            auto root_plan = make_invalidate_self_invalidation();
            if (inside_nth_child_pseudo_class == InsideNthChildPseudoClass::Yes) {
                // When invalidation property is nested in nth-child selector like p:nth-child(even of #t1, #t2, #t3)
                // we need to make sure all affected siblings are invalidated.
                root_plan->invalidate_whole_subtree = true;
            }
            add_invalidation_plan_for_properties(style_invalidation_data.invalidation_plans, invalidation_properties, *root_plan);
            style_invalidation_data.record_debug_selector_for_properties(invalidation_properties, selector);
            if (simple_selector_group_contains_has && !has_left_hand_context) {
                if (specific_has_subject_invalidation_properties.has_properties()) {
                    add_invalidation_plan_for_properties(style_invalidation_data.has_subject_invalidation_plans, specific_has_subject_invalidation_properties, *root_plan);
                    add_specific_has_invalidation_plan(style_invalidation_data.specific_has_subject_invalidation_plans, specific_has_subject_invalidation_properties, *root_plan, selector);
                    add_triggered_has_invalidation_plan(style_invalidation_data.triggered_has_subject_invalidation_plans, selector_has_trigger_properties, specific_has_subject_invalidation_properties, *root_plan, selector);
                    style_invalidation_data.record_debug_selector_for_has_subject_properties(specific_has_subject_invalidation_properties, selector);
                } else {
                    if (selector_has_trigger_properties.has_properties())
                        add_triggered_has_invalidation_plan(style_invalidation_data.triggered_has_subject_invalidation_plans, selector_has_trigger_properties, {}, *root_plan, selector);
                    else
                        style_invalidation_data.add_generic_has_subject_invalidation_plan(*root_plan, selector);
                }
            }

            invalidation_set_for_rightmost_selector = subject_match_set;
            selector_righthand = SelectorRighthand {
                .subject_match_set = move(subject_match_set),
                .subject_matches_any = subject_matches_any,
                .payload = root_plan,
            };
        } else {
            VERIFY(previous_compound_combinator != Selector::Combinator::None);
            VERIFY(selector_righthand.has_value());

            auto plan = build_invalidation_for_combinator(previous_compound_combinator, *selector_righthand);
            add_invalidation_plan_for_properties(style_invalidation_data.invalidation_plans, invalidation_properties, *plan);
            style_invalidation_data.record_debug_selector_for_properties(invalidation_properties, selector);
            if (simple_selector_group_contains_has && !has_left_hand_context) {
                if (specific_has_subject_invalidation_properties.has_properties()) {
                    add_invalidation_plan_for_properties(style_invalidation_data.has_subject_invalidation_plans, specific_has_subject_invalidation_properties, *plan);
                    add_specific_has_invalidation_plan(style_invalidation_data.specific_has_subject_invalidation_plans, specific_has_subject_invalidation_properties, *plan, selector);
                    add_triggered_has_invalidation_plan(style_invalidation_data.triggered_has_subject_invalidation_plans, selector_has_trigger_properties, specific_has_subject_invalidation_properties, *plan, selector);
                    style_invalidation_data.record_debug_selector_for_has_subject_properties(specific_has_subject_invalidation_properties, selector);
                } else {
                    if (selector_has_trigger_properties.has_properties())
                        add_triggered_has_invalidation_plan(style_invalidation_data.triggered_has_subject_invalidation_plans, selector_has_trigger_properties, {}, *plan, selector);
                    else
                        style_invalidation_data.add_generic_has_subject_invalidation_plan(*plan, selector);
                }
            }
            if (selector_contains_has) {
                if (specific_has_subject_invalidation_properties.has_properties()) {
                    add_invalidation_plan_for_properties(style_invalidation_data.has_non_subject_invalidation_plans, specific_has_subject_invalidation_properties, *plan);
                    add_specific_has_invalidation_plan(style_invalidation_data.specific_has_non_subject_invalidation_plans, specific_has_subject_invalidation_properties, *plan, selector);
                    add_triggered_has_invalidation_plan(style_invalidation_data.triggered_has_non_subject_invalidation_plans, selector_has_trigger_properties, specific_has_subject_invalidation_properties, *plan, selector);
                    style_invalidation_data.record_debug_selector_for_has_non_subject_properties(specific_has_subject_invalidation_properties, selector);
                } else {
                    if (selector_has_trigger_properties.has_properties())
                        add_triggered_has_invalidation_plan(style_invalidation_data.triggered_has_non_subject_invalidation_plans, selector_has_trigger_properties, {}, *plan, selector);
                    else
                        style_invalidation_data.add_generic_has_non_subject_invalidation_plan(*plan, selector);
                }
            }

            selector_righthand = SelectorRighthand {
                .subject_match_set = move(subject_match_set),
                .subject_matches_any = subject_matches_any,
                .payload = move(plan),
            };
        }

        previous_compound_combinator = combinator;
        ++processed_simple_selector_group_count;
    });

    return invalidation_set_for_rightmost_selector;
}

void StyleInvalidationData::build_invalidation_sets_for_selector(Selector const& selector)
{
    (void)build_invalidation_sets_for_selector_impl(*this, selector, InsideNthChildPseudoClass::No);
}

}
