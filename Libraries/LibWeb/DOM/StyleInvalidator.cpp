/*
 * Copyright (c) 2025, Aliaksandr Kalenik <kalenik.aliaksandr@gmail.com>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Debug.h>
#include <AK/QuickSort.h>
#include <AK/ScopeGuard.h>
#include <AK/StringBuilder.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/DOM/StyleInvalidator.h>

namespace Web::DOM {

GC_DEFINE_ALLOCATOR(StyleInvalidator);

struct DescendantRuleDebugStatistics {
    u64 match_count { 0 };
    u64 self_marks { 0 };
    u64 whole_subtree_marks { 0 };
    u64 descendant_rules_pushed { 0 };
    u64 sibling_rules_applied { 0 };
    Vector<String> sample_elements;
};

struct DescendantInvalidationDebugStatistics {
    u64 pending_roots { 0 };
    u64 pending_descendant_rules { 0 };
    u64 activated_roots { 0 };
    u64 activated_descendant_rules { 0 };
    u64 visited_nodes { 0 };
    u64 visited_elements_with_active_rules { 0 };
    u64 matched_rule_applications { 0 };
    u64 max_active_descendant_invalidations { 0 };
    u64 propagated_entire_subtree_nodes { 0 };
    u64 self_marks { 0 };
    u64 whole_subtree_marks { 0 };
    u64 descendant_rules_pushed { 0 };
    u64 sibling_rules_applied { 0 };
    HashTable<Node const*> unique_matched_elements;
    HashTable<Node const*> unique_style_marked_nodes;
    HashTable<Node const*> unique_subtree_marked_nodes;
    HashMap<String, DescendantRuleDebugStatistics> rule_statistics;
    Vector<String> activated_root_samples;
};

thread_local DescendantInvalidationDebugStatistics* s_descendant_invalidation_debug_statistics;

static void append_unique_debug_sample(Vector<String>& samples, String sample, size_t max_samples = 3)
{
    for (auto const& existing_sample : samples) {
        if (existing_sample == sample)
            return;
    }
    if (samples.size() >= max_samples)
        return;
    samples.append(move(sample));
}

static String format_invalidation_plan_for_debug(CSS::InvalidationPlan const& plan)
{
    StringBuilder builder;
    builder.appendff("self={} whole_subtree={} descendant_rules={} sibling_rules={}",
        plan.invalidate_self,
        plan.invalidate_whole_subtree,
        plan.descendant_rules.size(),
        plan.sibling_rules.size());
    return MUST(builder.to_string());
}

static String format_descendant_rule_for_debug(CSS::DescendantInvalidationRule const& rule)
{
    StringBuilder builder;
    builder.append("match=["sv);
    if (rule.match_any) {
        builder.append("<any>"sv);
    } else if (rule.match_set.is_empty()) {
        builder.append("<empty>"sv);
    } else {
        builder.appendff("{}", rule.match_set);
    }
    builder.appendff("] payload={{ {} }}", format_invalidation_plan_for_debug(*rule.payload));
    return MUST(builder.to_string());
}

static bool should_log_descendant_invalidation_summary(DescendantInvalidationDebugStatistics const& statistics)
{
    return statistics.unique_style_marked_nodes.size() >= 512
        || statistics.matched_rule_applications >= 1024
        || statistics.visited_elements_with_active_rules >= 2000
        || statistics.whole_subtree_marks > 0
        || statistics.propagated_entire_subtree_nodes > 0;
}

static String format_activated_root_samples_for_debug(DescendantInvalidationDebugStatistics const& statistics, size_t max_roots = 6)
{
    if (statistics.activated_root_samples.is_empty())
        return "<none>"_string;

    StringBuilder builder;
    size_t appended_roots = 0;
    for (auto const& sample : statistics.activated_root_samples) {
        if (appended_roots > 0)
            builder.append(" | "sv);
        builder.append(sample);
        ++appended_roots;
        if (appended_roots >= max_roots)
            break;
    }
    if (statistics.activated_root_samples.size() > appended_roots)
        builder.appendff(" | ... +{}", statistics.activated_root_samples.size() - appended_roots);
    return MUST(builder.to_string());
}

static String format_top_descendant_rules_for_debug(DescendantInvalidationDebugStatistics const& statistics, size_t max_rules = 4)
{
    struct RankedRule {
        String const* key;
        DescendantRuleDebugStatistics const* statistics;
    };

    Vector<RankedRule> ranked_rules;
    ranked_rules.ensure_capacity(statistics.rule_statistics.size());
    for (auto const& it : statistics.rule_statistics)
        ranked_rules.append({ &it.key, &it.value });

    quick_sort(ranked_rules, [](auto const& a, auto const& b) {
        if (a.statistics->match_count != b.statistics->match_count)
            return a.statistics->match_count > b.statistics->match_count;
        return *a.key < *b.key;
    });

    StringBuilder builder;
    size_t appended_rules = 0;
    for (auto const& ranked_rule : ranked_rules) {
        if (appended_rules > 0)
            builder.append("; "sv);
        builder.appendff("matches={} self={} subtree={} pushed_desc={} siblings={} rule={{ {} }} samples=[",
            ranked_rule.statistics->match_count,
            ranked_rule.statistics->self_marks,
            ranked_rule.statistics->whole_subtree_marks,
            ranked_rule.statistics->descendant_rules_pushed,
            ranked_rule.statistics->sibling_rules_applied,
            *ranked_rule.key);
        bool first_sample = true;
        for (auto const& sample : ranked_rule.statistics->sample_elements) {
            if (!first_sample)
                builder.append(" | "sv);
            builder.append(sample);
            first_sample = false;
        }
        builder.append(']');
        ++appended_rules;
        if (appended_rules >= max_rules)
            break;
    }

    if (ranked_rules.size() > appended_rules)
        builder.appendff("; ... +{} more", ranked_rules.size() - appended_rules);
    return MUST(builder.to_string());
}

static bool element_matches_invalidation_rule(Element const& element, CSS::InvalidationSet const& match_set, bool match_any)
{
    return match_any || element.includes_properties_from_invalidation_set(match_set);
}

void StyleInvalidator::visit_edges(Cell::Visitor& visitor)
{
    Base::visit_edges(visitor);
    for (auto const& it : m_pending_invalidations)
        visitor.visit(it.key);
}

void StyleInvalidator::invalidate(Node& node)
{
    if constexpr (LAYOUT_THRASH_DEBUG) {
        DescendantInvalidationDebugStatistics debug_statistics;
        for (auto const& it : m_pending_invalidations) {
            ++debug_statistics.pending_roots;
            debug_statistics.pending_descendant_rules += it.value.size();
        }

        auto* previous_debug_statistics = s_descendant_invalidation_debug_statistics;
        s_descendant_invalidation_debug_statistics = &debug_statistics;
        ScopeGuard restore_debug_statistics = [&] {
            s_descendant_invalidation_debug_statistics = previous_debug_statistics;
        };

        perform_pending_style_invalidations(node, false);

        if (debug_statistics.pending_descendant_rules > 0 && should_log_descendant_invalidation_summary(debug_statistics)) {
            dbgln("Descendant invalidation summary pending={{roots={} rules={}}} activated={{roots={} rules={} samples=[{}]}} traversal={{visited_nodes={} active_rule_elements={} matched_rule_apps={} unique_matched_elements={} max_active_rules={} subtree_nodes={}}} result={{unique_style_marked={} whole_subtree_roots={} self_marks={} whole_subtree_marks={} pushed_descendant_rules={} sibling_rules={}}}",
                debug_statistics.pending_roots,
                debug_statistics.pending_descendant_rules,
                debug_statistics.activated_roots,
                debug_statistics.activated_descendant_rules,
                format_activated_root_samples_for_debug(debug_statistics),
                debug_statistics.visited_nodes,
                debug_statistics.visited_elements_with_active_rules,
                debug_statistics.matched_rule_applications,
                debug_statistics.unique_matched_elements.size(),
                debug_statistics.max_active_descendant_invalidations,
                debug_statistics.propagated_entire_subtree_nodes,
                debug_statistics.unique_style_marked_nodes.size(),
                debug_statistics.unique_subtree_marked_nodes.size(),
                debug_statistics.self_marks,
                debug_statistics.whole_subtree_marks,
                debug_statistics.descendant_rules_pushed,
                debug_statistics.sibling_rules_applied);
            dbgln("Descendant invalidation top rules {}", format_top_descendant_rules_for_debug(debug_statistics));
        }
    } else {
        perform_pending_style_invalidations(node, false);
    }
    m_pending_invalidations.clear();
}

bool StyleInvalidator::enqueue_invalidation_plan(Node& node, StyleInvalidationReason reason, CSS::InvalidationPlan const& plan)
{
    if (plan.is_empty())
        return false;

    if (plan.invalidate_whole_subtree) {
        node.invalidate_style(reason);
        return true;
    }

    if (plan.invalidate_self)
        node.set_needs_style_update(true);

    add_pending_invalidation(node, reason, plan);

    if (auto* element = as_if<Element>(node)) {
        for (auto const& sibling_rule : plan.sibling_rules)
            apply_sibling_invalidation(*element, reason, sibling_rule);
    }

    return false;
}

void StyleInvalidator::add_pending_invalidation(GC::Ref<Node> node, StyleInvalidationReason reason, CSS::InvalidationPlan const& plan)
{
    if (plan.descendant_rules.is_empty())
        return;

    auto& pending_invalidations = m_pending_invalidations.ensure(node, [] {
        return Vector<PendingDescendantInvalidation> {};
    });
    for (auto const& descendant_rule : plan.descendant_rules)
        pending_invalidations.append({ reason, descendant_rule });
}

void StyleInvalidator::apply_invalidation_plan(Element& element, StyleInvalidationReason reason, CSS::InvalidationPlan const& plan, bool& invalidate_entire_subtree)
{
    if (plan.is_empty())
        return;

    if constexpr (LAYOUT_THRASH_DEBUG) {
        if (auto* debug_statistics = s_descendant_invalidation_debug_statistics) {
            debug_statistics->descendant_rules_pushed += plan.descendant_rules.size();
            debug_statistics->sibling_rules_applied += plan.sibling_rules.size();
        }
    }

    if (plan.invalidate_whole_subtree) {
        if constexpr (LAYOUT_THRASH_DEBUG) {
            if (auto* debug_statistics = s_descendant_invalidation_debug_statistics) {
                ++debug_statistics->whole_subtree_marks;
                debug_statistics->unique_style_marked_nodes.set(&element);
                debug_statistics->unique_subtree_marked_nodes.set(&element);
            }
        }
        element.invalidate_style(reason);
        invalidate_entire_subtree = true;
        element.set_needs_style_update_internal(true);
        if (element.has_child_nodes())
            element.set_child_needs_style_update(true);
        return;
    }

    if (plan.invalidate_self) {
        if constexpr (LAYOUT_THRASH_DEBUG) {
            if (auto* debug_statistics = s_descendant_invalidation_debug_statistics) {
                ++debug_statistics->self_marks;
                debug_statistics->unique_style_marked_nodes.set(&element);
            }
        }
        element.set_needs_style_update(true);
    }

    for (auto const& descendant_rule : plan.descendant_rules)
        m_active_descendant_invalidations.append({ reason, descendant_rule });

    for (auto const& sibling_rule : plan.sibling_rules)
        apply_sibling_invalidation(element, reason, sibling_rule);
}

void StyleInvalidator::apply_sibling_invalidation(Element& element, StyleInvalidationReason reason, CSS::SiblingInvalidationRule const& sibling_rule)
{
    auto apply_if_matching = [&](Element* sibling) {
        if (!sibling)
            return;
        if (!element_matches_invalidation_rule(*sibling, sibling_rule.match_set, sibling_rule.match_any))
            return;
        (void)enqueue_invalidation_plan(*sibling, reason, *sibling_rule.payload);
    };

    switch (sibling_rule.reach) {
    case CSS::SiblingInvalidationReach::Adjacent:
        apply_if_matching(element.next_element_sibling());
        break;
    case CSS::SiblingInvalidationReach::Subsequent:
        for (auto* sibling = element.next_element_sibling(); sibling; sibling = sibling->next_element_sibling())
            apply_if_matching(sibling);
        break;
    default:
        VERIFY_NOT_REACHED();
    }
}

// This function makes a full pass over the entire DOM and:
// - converts "entire subtree needs style update" into "needs style update" for each inclusive descendant where it's found.
// - applies descendant invalidation rules to matching elements
void StyleInvalidator::perform_pending_style_invalidations(Node& node, bool invalidate_entire_subtree)
{
    if constexpr (LAYOUT_THRASH_DEBUG) {
        if (auto* debug_statistics = s_descendant_invalidation_debug_statistics) {
            ++debug_statistics->visited_nodes;
            if (invalidate_entire_subtree)
                ++debug_statistics->propagated_entire_subtree_nodes;
        }
    }

    invalidate_entire_subtree |= node.entire_subtree_needs_style_update();

    if (invalidate_entire_subtree) {
        node.set_needs_style_update_internal(true);
        if (node.has_child_nodes())
            node.set_child_needs_style_update(true);
    }

    auto previous_active_descendant_invalidations_size = m_active_descendant_invalidations.size();
    ScopeGuard restore_state = [this, previous_active_descendant_invalidations_size] {
        m_active_descendant_invalidations.shrink(previous_active_descendant_invalidations_size);
    };

    if (!invalidate_entire_subtree) {
        if (auto pending_invalidations = m_pending_invalidations.get(node); pending_invalidations.has_value()) {
            if constexpr (LAYOUT_THRASH_DEBUG) {
                if (auto* debug_statistics = s_descendant_invalidation_debug_statistics) {
                    ++debug_statistics->activated_roots;
                    debug_statistics->activated_descendant_rules += pending_invalidations->size();
                    if (debug_statistics->activated_root_samples.size() < 8)
                        append_unique_debug_sample(debug_statistics->activated_root_samples, MUST(String::formatted("{} rules={}", node.debug_description(), pending_invalidations->size())), 8);
                }
            }
            m_active_descendant_invalidations.extend(*pending_invalidations);
        }

        if constexpr (LAYOUT_THRASH_DEBUG) {
            if (auto* debug_statistics = s_descendant_invalidation_debug_statistics)
                debug_statistics->max_active_descendant_invalidations = max(debug_statistics->max_active_descendant_invalidations, static_cast<u64>(m_active_descendant_invalidations.size()));
        }

        if (auto* element = as_if<Element>(node)) {
            if constexpr (LAYOUT_THRASH_DEBUG) {
                if (auto* debug_statistics = s_descendant_invalidation_debug_statistics; debug_statistics && !m_active_descendant_invalidations.is_empty())
                    ++debug_statistics->visited_elements_with_active_rules;
            }
            size_t invalidation_index = 0;
            while (invalidation_index < m_active_descendant_invalidations.size()) {
                auto const& pending_invalidation = m_active_descendant_invalidations[invalidation_index++];
                if (!element_matches_invalidation_rule(*element, pending_invalidation.rule.match_set, pending_invalidation.rule.match_any))
                    continue;

                if constexpr (LAYOUT_THRASH_DEBUG) {
                    if (auto* debug_statistics = s_descendant_invalidation_debug_statistics) {
                        ++debug_statistics->matched_rule_applications;
                        debug_statistics->unique_matched_elements.set(element);
                        auto rule_key = format_descendant_rule_for_debug(pending_invalidation.rule);
                        auto& rule_statistics = debug_statistics->rule_statistics.ensure(move(rule_key), [] {
                            return DescendantRuleDebugStatistics {};
                        });
                        ++rule_statistics.match_count;
                        if (pending_invalidation.rule.payload->invalidate_self)
                            ++rule_statistics.self_marks;
                        if (pending_invalidation.rule.payload->invalidate_whole_subtree)
                            ++rule_statistics.whole_subtree_marks;
                        rule_statistics.descendant_rules_pushed += pending_invalidation.rule.payload->descendant_rules.size();
                        rule_statistics.sibling_rules_applied += pending_invalidation.rule.payload->sibling_rules.size();
                        append_unique_debug_sample(rule_statistics.sample_elements, element->debug_description(), 3);
                    }
                }

                apply_invalidation_plan(*element, pending_invalidation.reason, *pending_invalidation.rule.payload, invalidate_entire_subtree);
                if (invalidate_entire_subtree)
                    break;
            }

            if (invalidate_entire_subtree) {
                node.set_needs_style_update_internal(true);
                if (node.has_child_nodes())
                    node.set_child_needs_style_update(true);
            }
        }
    }

    for (auto* child = node.first_child(); child; child = child->next_sibling())
        perform_pending_style_invalidations(*child, invalidate_entire_subtree);

    if (node.is_element()) {
        auto& element = static_cast<Element&>(node);
        if (auto shadow_root = element.shadow_root()) {
            perform_pending_style_invalidations(*shadow_root, invalidate_entire_subtree);
            if (invalidate_entire_subtree)
                node.set_child_needs_style_update(true);
        }
    }

    node.set_entire_subtree_needs_style_update(false);
}

}
