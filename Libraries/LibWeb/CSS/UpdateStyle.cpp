/*
 * Copyright (c) 2018-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/HashTable.h>
#include <AK/QuickSort.h>
#include <AK/ScopeGuard.h>
#include <LibGC/RootVector.h>
#include <LibWeb/Animations/Animation.h>
#include <LibWeb/Animations/AnimationEffect.h>
#include <LibWeb/Animations/KeyframeEffect.h>
#include <LibWeb/CSS/CSSStyleProperties.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/CustomPropertyData.h>
#include <LibWeb/CSS/Invalidation/SlotInvalidator.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEffectDrain.h>
#include <LibWeb/CSS/StyleEngineInput.h>
#include <LibWeb/CSS/StyleInvalidation.h>
#include <LibWeb/DOM/AbstractElement.h>
#include <LibWeb/DOM/CommitMessages.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/Node.h>
#include <LibWeb/DOM/PseudoElement.h>
#include <LibWeb/DOM/Range.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/HTML/FormAssociatedElement.h>
#include <LibWeb/HTML/HTMLSlotElement.h>
#include <LibWeb/HTML/LocalNavigable.h>
#include <LibWeb/HTML/NavigableContainer.h>
#include <LibWeb/Layout/Box.h>
#include <LibWeb/Selection/Selection.h>

namespace Web::CSS {

bool deferring_engine_pseudo_installation();

static thread_local bool g_deferring_engine_pseudo_installation = false;

bool deferring_engine_pseudo_installation()
{
    return g_deferring_engine_pseudo_installation;
}

using StyleUpdateMode = DOM::Document::StyleUpdateMode;

extern "C" void ladybird_utf16_fly_string_unref(size_t);
extern "C" void rust_style_seal_set_in_effect_drain(bool);

}

namespace Web::DOM {

void begin_style_row_counter_style_invalidation(Element const&);
CSS::RequiredInvalidationAfterStyleChange end_style_row_counter_style_invalidation(Element const&);
bool style_row_computed_damage_itself(Element const&);

}

namespace Web::CSS {

static void finish_complete_style_update(DOM::Document& document)
{
    auto releases = StyleValueFFI::rust_style_ffi_complete_style_update_end();
    ScopeGuard clear_releases = StyleValueFFI::rust_deferred_cpp_releases_clear;
    for (size_t i = 0; i < releases.fly_string_count; ++i)
        ladybird_utf16_fly_string_unref(releases.fly_strings[i]);
    document.commit_messages().apply_style_messages();
}

enum class DocumentWithoutBrowsingContext {
    Skip,
    Update,
};
static void update_style(DOM::Document&, DocumentWithoutBrowsingContext = DocumentWithoutBrowsingContext::Skip);
static bool update_style_for_element(DOM::Document&, DOM::AbstractElement const&, StyleUpdateMode);

static void apply_element_style_invalidation_after_style_change(DOM::Element& element, RequiredInvalidationAfterStyleChange const& invalidation)
{
    if (invalidation.accumulated_visual_contexts() == AccumulatedVisualContextInvalidation::UpdateValues)
        element.document().schedule_accumulated_visual_context_update(element, DOM::Document::AccumulatedVisualContextUpdateScope::Values);
    else if (invalidation.accumulated_visual_contexts() == AccumulatedVisualContextInvalidation::Rebuild)
        element.document().schedule_accumulated_visual_context_update(element, DOM::Document::AccumulatedVisualContextUpdateScope::Structure);

    if (invalidation.needs_scroll_container_resnap)
        element.document().schedule_scroll_container_resnap();

    // Only a full layout pass applies viewport propagation again, so a relayout of an element the viewport takes its
    // overflow, writing mode, or direction from must not finish as a partial relayout of that element.
    bool const element_is_viewport_propagation_source = element.is_viewport_propagation_source();
    if (invalidation.needs_relayout() && element_is_viewport_propagation_source)
        element.document().record_partial_relayout_escape(DOM::PartialRelayoutEscapeReason::ViewportPropagationSourceChangedByStyleChange);

    if (invalidation.needs_relayout()) {
        // A relayout-only style change on an absolutely positioned partial relayout boundary
        // stays confined to it: the box contributes nothing to ancestor layout, and partial
        // relayout re-resolves the boundary's own size and position. A rendered ::backdrop
        // disqualifies the element, because pseudo-element style diffs are merged into the
        // element's invalidation while the ::backdrop box is a sibling of the element's box,
        // outside the subtree a boundary-self relayout covers.
        auto* box = as_if<Layout::Box>(element.unsafe_layout_node());
        if (!invalidation.needs_layout_tree_rebuild()
            && !element_is_viewport_propagation_source
            && box
            && box->is_absolutely_positioned()
            && box->is_partial_relayout_boundary()
            && !element.pseudo_element_unsafe_layout_node(CSS::PseudoElement::Backdrop)) {
            box->set_needs_own_geometry_update();
            element.set_needs_layout_update(DOM::SetNeedsLayoutReason::StyleChange, Layout::LayoutUpdatePropagation::BoundarySelfOnly);
        } else {
            element.set_needs_layout_update(DOM::SetNeedsLayoutReason::StyleChange);
        }
    }
    if (invalidation.needs_layout_tree_rebuild())
        element.set_needs_layout_tree_rebuild(DOM::SetNeedsLayoutTreeUpdateReason::StyleChange, invalidation.layout_tree_rebuild_root());
}

void StyleEffectDrain::install(DOM::Document& document, Function<void(StyleDrainScope const&)> const& install)
{
    // What the drain asks of the engine is the pass's output being installed and applied, which the
    // style seal counts apart from the pass's round trips.
    rust_style_seal_set_in_effect_drain(true);
    auto& style_engine = document.style_computer().style_engine();
    style_engine.enter_effect_drain();
    ScopeGuard end_effect_drain = [&] {
        style_engine.leave_effect_drain();
        // Once a batch is drained, nothing the pass published waits for the host.
        style_engine.set_published_batch_waits(false);
        rust_style_seal_set_in_effect_drain(false);
    };
    StyleDrainScope const scope { style_engine };
    install(scope);
}

void StyleEffectDrain::apply(DOM::Document& document)
{
    install(document, [&](StyleDrainScope const& scope) { apply(scope, document); });
}

void StyleEffectDrain::apply(StyleDrainScope const& scope, DOM::Document& document)
{
    for (auto const& effect : m_effects) {
        if (auto const* row = effect.get_pointer<RestoreRowDebts>()) {
            scope.engine().restore_row_debts(row->style_node, row->explicit_inheritance_debt, row->row_effect_debt);
            continue;
        }
        if (auto const* row = effect.get_pointer<AcknowledgeRecord>()) {
            scope.engine().acknowledge_engine_computed_record(row->style_node);
            continue;
        }
        if (auto const* row = effect.get_pointer<DiscardContainerQueryEffects>()) {
            StyleEngineFFI::style_engine_native_container_effects_release(StyleEngineFFI::style_engine_take_container_effects(scope.engine().rust_handle(), row->style_node.value()).effects);
            continue;
        }
        auto element = document.style_computer().element_for_style_node(effect.visit([](auto const& row) { return row.style_node; }));
        if (!element)
            continue;
        effect.visit(
            [&](LayoutNodeStyle const& row) {
                element->apply_computed_style_to_layout_node_if_needed(row.invalidation);
            },
            [&](ElementInvalidation const& row) {
                apply_element_style_invalidation_after_style_change(*element, row.invalidation);
            },
            [&](ExplicitInheritance const& row) {
                if (!element->is_connected() || &element->document() != &document)
                    return;
                if (auto* parent = element->parent())
                    parent->add_children_explicitly_inherited_non_inherited_style_groups(row.style_groups == NumericLimits<u32>::max() ? ComputedValues::all_style_groups : row.style_groups);
            },
            [&](AnchorNames const& row) {
                if (auto style = element->computed_style())
                    element->update_anchor_name_registry(row.old_names, *style);
            },
            [&](AnimationNames const&) {
                element->republish_animation_name_registry();
            },
            [&](ContainerQueryEffects const& row) {
                auto container_effects = StyleEngineFFI::style_engine_take_container_effects(scope.engine().rust_handle(), row.style_node.value());
                ScopeGuard release_container_effects = [&] { StyleEngineFFI::style_engine_native_container_effects_release(container_effects.effects); };
                StyleComputer::record_container_query_effects(scope, DOM::AbstractElement { *element }, container_effects);
            },
            [&](AnimationPlan const& row) {
                document.style_computer().apply_settled_animation_plan(DOM::AbstractElement { *element }, row.plan);
            },
            [&](DisplayNoneAnimations const&) {
                element->apply_display_none_change(scope, true, false);
            },
            [&](RestoreRowDebts const&) {
                VERIFY_NOT_REACHED();
            },
            [&](AcknowledgeRecord const&) {
                VERIFY_NOT_REACHED();
            },
            [&](DiscardContainerQueryEffects const&) {
                VERIFY_NOT_REACHED();
            });
    }
    m_effects.clear();
}

static void apply_document_style_invalidation_after_style_change(DOM::Document& document, RequiredInvalidationAfterStyleChange const& invalidation)
{
    if (!invalidation.needs_repaint())
        return;
    if (invalidation.invalidates_hit_test_display_list())
        document.set_needs_to_record_display_list();
    else
        document.set_needs_to_record_display_list_keeping_hit_test_display_list();
}

// Consume everything recorded since the last transaction boundary and publish its match answers.
//
// The reaction batch is a superset by construction: routing may over-approximate, and every subject
// it yields is checked exactly before publication. What it may not do is under-approximate, so a
// region that could not be proven narrower covers the whole document rather than guessing at part
// of it. Reactions are consumed as the engine emits them so bridge scratch cannot determine the
// transaction's scope.
struct StyleEngineTransaction {
    Vector<StyleEngine::PublishedStyleDelta> reactions;
    // The transaction continues the style change whose reactions were applied last, one tree
    // generation further, rather than answering new inputs.
    bool only_derived_child_reactions { false };
};

// Readies the document for a style transaction. Returns the root to take it for, if there is one.
static Optional<StyleNodeID> begin_style_engine_transaction(DOM::Document& document)
{
    auto& style_computer = document.style_computer();
    // One element's computed style answers for another only while the inputs it was keyed on still
    // mean what they meant. A transaction boundary is exactly where they stop doing so: a
    // declaration keyed on by identity may have been edited, and a sheet may have come or gone.
    auto transaction_setup_started_at = MonotonicTime::now();
    ++document.style_invalidation_counters().style_engine_transaction_setups;
    // The engine resolves custom properties against the registrations in force, and an
    // @property rule registers through this cache.
    document.build_registered_properties_cache_for_style_update();
    style_computer.prepare_for_style_engine_transaction();
    auto setup_microseconds = (MonotonicTime::now() - transaction_setup_started_at).to_truncated_microseconds();
    document.style_invalidation_counters().style_engine_transaction_setup_microseconds += setup_microseconds;
    document.style_invalidation_counters().style_update_submission_microseconds += setup_microseconds;
    auto* root = document.document_element();
    if (!root || root->style_node_id() == 0) {
        style_computer.style_engine().flush();
        return {};
    }
    return root->style_node_id();
}

static StyleEngineTransaction accept_style_engine_transaction(DOM::Document& document, StyleEngine::PublishedStyleTransaction const& published_transaction)
{
    StyleEngineTransaction transaction;
    auto& style_computer = document.style_computer();
    document.style_invalidation_counters().style_update_submission_microseconds += published_transaction.submission_microseconds;
    document.style_invalidation_counters().style_update_bridge_microseconds += published_transaction.bridge_microseconds;
    if (!published_transaction.reactions.is_empty()) {
        style_computer.style_engine().note_published_transaction_version(published_transaction.version);
        style_computer.style_engine().set_published_batch_waits(true);
    }
    for (auto const& answer : published_transaction.reactions) {
        // The complete answer remains in Rust transaction scratch under this node. The identity
        // names both the semantic reaction and the payload that consumes it.
        VERIFY(style_computer.element_for_style_node(answer.style_node));
        transaction.reactions.append(answer);
    }

    transaction.only_derived_child_reactions = published_transaction.only_derived_child_reactions;

    return transaction;
}

static StyleEngineTransaction take_style_engine_transaction(DOM::Document& document)
{
    auto root = begin_style_engine_transaction(document);
    if (!root.has_value())
        return {};
    return accept_style_engine_transaction(document, document.style_computer().style_engine().take_style_transaction(*root));
}

enum class SampleInvalidation {
    Applied,
    AppliedByCaller,
};

static void sample_animations_for_installed_record(DOM::AbstractElement abstract_element, SampleInvalidation sample_invalidation = SampleInvalidation::Applied)
{
    auto record = abstract_element.style_record_identity();
    if (!record)
        return;
    auto& style_computer = abstract_element.document().style_computer();
    Animations::AnimationUpdateContext context;
    Animations::AnimationUpdateContext::ElementData data { record, style_computer.reconstruct_computed_properties_for_animation(record) };
    data.caller_applies_invalidation = sample_invalidation == SampleInvalidation::AppliedByCaller;
    context.elements.set(abstract_element, move(data));
}

// The environment a style pass composed an element's animated custom properties into, viewed as the
// element's, or the element's record's environment again where the sample animated none; and what
// that moves, recorded for the next transaction.
static void install_sampled_custom_property_environment(StyleDrainScope const& scope, DOM::Element& element, StyleEngineFFI::FfiRowSampledInPass const& sample)
{
    auto data = element.custom_property_data({});
    RefPtr<CustomPropertyData const> base = data;
    if (data && data->is_animation_overlay_for({ element }))
        base = data->parent();
    RefPtr<CustomPropertyData const> installed = base;
    if (sample.custom_property_environment != 0) {
        VERIFY(sample.custom_property_store);
        installed = CustomPropertyData::view_animation_overlay(sample.custom_property_store, sample.custom_property_environment, base, { element });
    }
    element.replace_custom_property_data(scope, {}, installed);
    auto& style_engine = element.document().style_computer().style_engine();
    if (sample.custom_property_reactions & 1)
        style_engine.record_derived_element_style_input_change(element.style_node_id(), StyleEngine::PublishedStyle | StyleEngine::RecomputeStyle);
    if (sample.custom_property_reactions & 2) {
        style_engine.record_flat_tree_descendant_style_input_changes(
            element.style_node_id(),
            StyleEngine::InheritedStyle,
            RequiredInvalidationAfterStyleChange::all_inherited_style_groups);
    }
}

// The pass sampled the element's animations over the record the row settled and published the
// composition, which the rows after it already read: install it as the host's own sample would
// have, and record what the sample found out on the element and its parent.
static bool install_composition_sampled_in_pass(StyleDrainScope const& scope, DOM::AbstractElement abstract_element, StyleEngineFFI::FfiRowSampledInPass const& sample, SampleInvalidation sample_invalidation)
{
    auto& element = const_cast<DOM::Element&>(abstract_element.element());
    auto& document = element.document();
    // A sample the host published since the pass composed over the row's record again.
    if (StyleEngineFFI::style_engine_assigned_style_record(document.style_computer().style_engine().rust_handle(), element.style_node_id().value(), NumericLimits<u8>::max()) != sample.style_record)
        return false;
    if (sample.substitution_marks & ComputedValuesFFI::SUBSTITUTION_MARK_VAR)
        element.set_style_uses_var_css_function();
    if (sample.substitution_marks & ComputedValuesFFI::SUBSTITUTION_MARK_ATTR)
        element.set_style_uses_attr_css_function();
    if (sample.substitution_marks & ComputedValuesFFI::SUBSTITUTION_MARK_IF)
        element.set_style_uses_if_css_function();
    if (sample.substitution_marks & ComputedValuesFFI::SUBSTITUTION_MARK_INHERIT)
        element.set_style_uses_inherit_css_function();
    if (sample.substitution_marks & ComputedValuesFFI::SUBSTITUTION_MARK_DASHED_FUNCTION)
        element.set_style_uses_custom_function();
    if (sample.uses_tree_counting_function)
        element.set_style_uses_tree_counting_function();
    if (sample.custom_property_environment_moved)
        install_sampled_custom_property_environment(scope, element, sample);
    // A keyframe-borne `inherit` on a non-inherited property leaves the same mark on the parent a
    // full style computation does.
    if (auto style_groups = sample.keyframes_inherited_non_inherited_style_groups; style_groups != 0) {
        if (style_groups == NumericLimits<u32>::max())
            style_groups = ComputedValues::all_style_groups;
        if (auto* parent = element.parent())
            parent->add_children_explicitly_inherited_non_inherited_style_groups(style_groups);
    }
    if (abstract_element.style_record_identity().value() == sample.style_record)
        return true;
    if (!sample.overlay_is_empty && document.is_in_style_stabilization_epoch()
        && (document.style_stabilization_has_style_reactions() || sample.invalidation.requires_base_style_recomputation))
        document.style_computer().record_transition_stabilization_baseline(scope, abstract_element);
    (void)element.unsafe_layout_node();
    Animations::apply_published_animation_overlay(scope, abstract_element, sample.invalidation, StyleRecordID { sample.style_record }, sample_invalidation == SampleInvalidation::AppliedByCaller);
    return true;
}

static void sample_animations_for_installed_pseudos(DOM::Element& element)
{
    // A dirty effect may have been visited before a newly generated pseudo had a record.
    // Compose it at installation so its first observable style includes that effect.
    if (!element.has_associated_animations())
        return;
    for (size_t kind = 0; kind < to_underlying(PseudoElement::KnownPseudoElementCount); ++kind) {
        DOM::AbstractElement pseudo { element, static_cast<PseudoElement>(kind) };
        if (pseudo.has_style())
            sample_animations_for_installed_record(pseudo);
    }
}

// Whether the custom-property environment an engine-computed record was published with can be
// installed: the one the element inherits - the parent's inheritable data, which is the parent's
// own unless a registration made some of it non-inherited - or one the engine resolved over it.
static bool engine_computed_record_environment_is_installable(DOM::Element& element, StyleRecordID style_record)
{
    bool installable = false;
    (void)element.custom_property_environment_of_engine_record(style_record, installable);
    return installable;
}

// What an element held as a style row began, which the engine derives the children's reactions
// from with what the row leaves it holding.
struct StyleRowStart {
    bool had_style { false };
    Optional<Display> display;
};

static StyleRowStart style_row_start(DOM::Element& element)
{
    StyleRowStart start { element.has_style(), {} };
    if (auto const* box_values = element.style_group<ComputedValues::BoxValues>())
        start.display = display_from_ffi_display(box_values->display);
    return start;
}

static u32 style_row_start_facts(DOM::Element& element, StyleRowStart const& start)
{
    if (!start.had_style)
        return StyleEngine::RowWasUnstyled;
    u32 facts = 0;
    if (start.display.has_value() && start.display->is_none())
        facts |= StyleEngine::RowWasDisplayNone;
    if (auto const* box_values = element.style_group<ComputedValues::BoxValues>(); box_values && start.display.has_value()
        && display_from_ffi_display(box_values->display) != *start.display)
        facts |= StyleEngine::RowDisplayChanged;
    return facts;
}

// The debts the engine took as it published a computed element row, which the host settles as it
// installs the row, or hands back.
struct PublishedRowDebts {
    u32 explicit_inheritance { 0 };
    u8 row_effect { 0 };

    bool is_empty() const { return explicit_inheritance == 0 && row_effect == 0; }
};

// `declined_rows` names the rows the previous wave declined, and returns the ones this wave declines.
// `declined_a_row_again` says whether this wave declined one of the previous wave's again.
static RequiredInvalidationAfterStyleChange apply_style_engine_reactions(StyleDrainScope const& scope, DOM::Document& document, Vector<StyleEngine::PublishedStyleDelta> const& reactions, HashTable<StyleNodeID>& declined_rows, bool& declined_a_row_again)
{
    auto const rows_declined_by_previous_wave = move(declined_rows);
    declined_rows.clear();
    // Reactions are applied in preorder, so every element's inheritance inputs are ready when it is
    // applied. What an applied element's change means for its (flat-tree) children is the
    // engine's to derive: it reads each application and plans the children as the next
    // transaction of this style update.
    RequiredInvalidationAfterStyleChange transaction_invalidation;
    begin_noting_declaration_changes_during_apply();
    ScopeGuard end_noting_declaration_changes = [] { end_noting_declaration_changes_during_apply(); };
    // Unstyled descendants of display:none need no record until a targeted read or visibility
    // change asks for one. SVG resources and existing animations can still consume style while
    // hidden, so retain their inheritance prerequisites in this batch.
    HashTable<StyleNodeID> required_in_hidden_subtrees;
    // The effects the batch's rows leave for the host, applied once the whole batch is installed. The
    // explicit-inheritance marks are monotone and a parent applies before its children, so draining
    // them after the batch marks the parent no later than the C++ path does.
    StyleEffectDrain row_effects;
    // The elements whose records an environment move of a row before them republished.
    HashTable<StyleNodeID> republished_nodes;
    for (auto const& reaction : reactions) {
        auto element = document.style_computer().element_for_style_node(reaction.style_node);
        if (!element || (!element->is_svg_element() && !element->has_relevant_animations()))
            continue;
        for (Optional<DOM::AbstractElement> ancestor = DOM::AbstractElement { *element }; ancestor.has_value(); ancestor = ancestor->element_to_inherit_style_from()) {
            if (required_in_hidden_subtrees.set(ancestor->element().style_node_id()) == HashSetResult::KeptExistingEntry)
                break;
        }
    }
    {
        for (size_t reaction_index = 0; reaction_index < reactions.size(); ++reaction_index) {
            auto const& published_reaction = reactions[reaction_index];
            // A pseudo-element record installs with its element's, which leads it.
            if (published_reaction.pseudo_kind != NumericLimits<u8>::max())
                continue;
            // A row that does not install hands the debts the engine took with it back.
            PublishedRowDebts row_debts { published_reaction.explicit_inheritance_debt, static_cast<u8>(published_reaction.row_effect_debt) };
            ScopeGuard restore_unsettled_row_debts = [&] {
                if (!row_debts.is_empty())
                    row_effects.append(StyleEffectDrain::RestoreRowDebts { StyleNodeID { published_reaction.style_node }, row_debts.explicit_inheritance, row_debts.row_effect });
            };
            auto element = document.style_computer().element_for_style_node(published_reaction.style_node);
            if (!element)
                continue;
            // An ancestor's custom-property environment moved, and the engine moved the element's with
            // it as it settled the ancestor's row: the element takes the moved environment, and the
            // record the engine republished over it.
            if (published_reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::EnvironmentMoved) {
                if (auto record = scope.engine().acknowledge_environment_move(StyleNodeID { published_reaction.style_node }); record != 0 && record != element->style_record_identity().value()) {
                    element->refresh_computed_style(scope, {}, StyleRecordID { record });
                    republished_nodes.set(StyleNodeID { published_reaction.style_node });
                }
                continue;
            }
            // A reaction the engine derived for this element while applying an earlier one in
            // this batch joins the element's own reaction where it covers it.
            auto reaction = published_reaction;
            if (auto absorbed = scope.engine().absorb_element_style_input(
                    StyleNodeID { reaction.style_node }, reaction.reaction, reaction.inherited_style_groups);
                absorbed != 0) {
                reaction.reaction = static_cast<u8>(absorbed & 0xff);
                reaction.inherited_style_groups = static_cast<u8>(absorbed >> 8);
            }
            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::SkippedHidden)
                continue;

            if (!element->has_style() && !required_in_hidden_subtrees.contains(element->style_node_id())) {
                bool hidden = false;
                for (auto ancestor = DOM::AbstractElement { *element }.element_to_inherit_style_from(); ancestor.has_value(); ancestor = ancestor->element_to_inherit_style_from()) {
                    auto identity = ancestor->style_record_identity();
                    if (!!identity) {
                        hidden = has_flag(document.style_computer().style_engine().style_record_dependency_flags(identity), StyleRecordDependencyFlag::InDisplayNoneSubtree);
                        break;
                    }
                }
                if (hidden)
                    continue;
            }

            bool retried_unstyled_materialization = false;
            bool retried_after_installed_ancestors = false;
            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::RetriedAfterAncestors
                || reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::RetriedMaterialization) {
                // The engine computed this row over the rows installed before it, the way the host
                // would have computed it here.
                StyleEngineFFI::style_engine_note_host_step(reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::RetriedAfterAncestors ? StyleEngineFFI::FfiStyleHostStep::RetriedAfterAncestors : StyleEngineFFI::FfiStyleHostStep::RetriedMaterialization);
                retried_unstyled_materialization = reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::RetriedMaterialization && !element->has_style();
                retried_after_installed_ancestors = reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::RetriedAfterAncestors;
                reaction.gap = StyleEngineFFI::FfiStyleDeltaGap::Computed;
            }

            // An engine-computed first record installs on an element without style, as does one
            // for an element whose ancestor became visible: its style was cleared on entry to
            // display:none while the engine kept the record. A materialization retried above is
            // also an install even when the engine held an old record the DOM never received.
            // Hidden SVG resource styles are cleared and then restored from derived records.
            // Other record deltas assume the style they move.
            if (!element->has_style()
                && !(reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Computed
                    && (reaction.old_style_record == 0 || (reaction.reaction & StyleEngine::AncestorBecameVisible)
                        || retried_unstyled_materialization || element->is_svg_element())))
                continue;

            bool const has_published_style_reaction = reaction.reaction & StyleEngine::PublishedStyle;
            if (has_published_style_reaction) {
                ++document.style_invalidation_counters().style_engine_published_reactions;
            }
            ++document.style_invalidation_counters().style_engine_record_deltas_applied;

            // An engine-computed record is the engine's current answer for the element, which may
            // have skipped a delta C++ never installed; it is applied against whatever the element
            // holds now.
            VERIFY(reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Computed || reaction.old_style_record == element->style_record_identity().value());
            VERIFY(reaction.new_style_record != 0);
            VERIFY(reaction.damage == StyleEngineFFI::FfiStyleDeltaDamage::Full);

            // What the element holds now is what the row moves it from: the engine reads the row's
            // facts for the children from that and from what the element holds when it is noted.
            StyleEngineFFI::style_engine_note_host_step(StyleEngineFFI::FfiStyleHostStep::Row);
            // The engine derived the children's reactions from the row's move away from what the
            // element holds, when nothing the host absorbed into the row asks for more.
            bool const engine_derived_children = [&] {
                if (reaction.reaction != published_reaction.reaction || reaction.inherited_style_groups != published_reaction.inherited_style_groups)
                    return false;
                if (reaction.record_damage & to_underlying(StyleEngineFFI::FfiStyleInvalidationField::ChildrenDerivedOverOldRecord))
                    return element->style_record_identity().value() == reaction.old_style_record;
                if (reaction.record_damage & to_underlying(StyleEngineFFI::FfiStyleInvalidationField::ChildrenDerivedOverNoRecord))
                    return !element->has_style();
                return false;
            }();
            auto const row_start = style_row_start(*element);
            DOM::begin_style_row_counter_style_invalidation(*element);
            auto const* previous_inherited_box_values = element->style_group<ComputedValues::InheritedBoxValues>();
            auto const previous_visibility = previous_inherited_box_values
                ? Optional<Visibility> { static_cast<Visibility>(previous_inherited_box_values->visibility) }
                : Optional<Visibility> {};
            bool const needs_regular_style_recompute = reaction.reaction & (StyleEngine::PublishedStyle | StyleEngine::RecomputeStyle | StyleEngine::RecomputeDescendantStyles | StyleEngine::AncestorBecameVisible);
            bool const needs_custom_property_recompute = reaction.reaction & StyleEngine::InheritedCustomProperties;
            bool const needs_inherited_style_recompute = reaction.reaction & StyleEngine::InheritedStyle;
            bool did_change_custom_properties = false;
            RequiredInvalidationAfterStyleChange invalidation;
            // The animation plan the row leaves for the host, taken in the branch that installs an
            // engine-computed record below.
            Optional<StyleComputer::SettledAnimationPlan> animation_plan;

            // What the row's node holds travels with the row, until the host asks the engine for the
            // node's record again.
            u32 row_facts = published_reaction.row_facts;
            // The engine settled the element's record, and the pseudo-element records beside it:
            // C++ installs them.
            auto engine_record_comparison = DOM::Element::EngineRecordComparison::AtInstallation;
            auto apply_engine_computed_records = [&](DOM::Element::EnginePseudoElementRecords const& pseudo_element_records, DOM::Element::EnginePseudoElementDamages const* pseudo_element_damages = nullptr) {
                auto& style_engine = document.style_computer().style_engine();
                document.style_computer().pin_transition_stabilization_baseline_if_a_later_pass_may_need_it(scope, DOM::AbstractElement { *element });
                // A first record answers the element's recorded arrival; nothing is left for a
                // later transaction to plan. Neither is anything for a record retried after the
                // ancestors applied before it installed: it reads them as they now stand, as the
                // computation it stands for would have.
                if (!element->has_style() || retried_after_installed_ancestors)
                    style_engine.consume_recorded_element_style_input_change(reaction.style_node);
                // A row that moved nothing names the record the element held when the transaction
                // published it. An ancestor applied before it can have republished that record over
                // a moved custom-property environment since, and the element holds the republished
                // record the engine now assigns it: the published one is nobody's any more.
                auto new_style_record = StyleRecordID { reaction.new_style_record };
                bool const holds_republished_record = reaction.new_style_record == reaction.old_style_record && element->style_record_identity().value() != reaction.old_style_record
                    && republished_nodes.contains(StyleNodeID { reaction.style_node });
                if (holds_republished_record)
                    new_style_record = element->style_record_identity();
                // The engine answered the record with what the move from the record it names damages.
                Optional<DOM::Element::EngineRecordDamage> engine_record_damage;
                if (reaction.record_damage & to_underlying(StyleEngineFFI::FfiStyleInvalidationField::EngineComputed))
                    engine_record_damage = DOM::Element::EngineRecordDamage { StyleRecordID { reaction.old_style_record }, reaction.record_damage };
                invalidation = element->apply_engine_computed_style_record(scope, new_style_record, pseudo_element_records, reaction.uses_substitution, row_facts, did_change_custom_properties, engine_record_comparison, engine_record_damage, pseudo_element_damages, &row_effects);
                // What the row's container conditions read of its containers, recorded as the host
                // records it for a row it computes.
                row_effects.append(StyleEffectDrain::ContainerQueryEffects { StyleNodeID { reaction.style_node } });
            };
            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::None) {
                VERIFY(!needs_regular_style_recompute);
                VERIFY(needs_inherited_style_recompute);
                VERIFY(!needs_custom_property_recompute);
                VERIFY(reaction.pseudo_kind == NumericLimits<u8>::max());
                // The engine swapped the element's inherited groups for its parent's: the record
                // installs as an engine record. The engine refuses the swap to an element that
                // animates, declares transitions, or inherits from an animating parent.
                apply_engine_computed_records({});
            } else if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Computed) {
                // The engine computed the new record from this element's moved cascade winners,
                // from its parent's moved inherited style or display, or from its moved
                // inherited custom-property environment.
                VERIFY(needs_regular_style_recompute || needs_inherited_style_recompute || needs_custom_property_recompute);
                VERIFY(reaction.pseudo_kind == NumericLimits<u8>::max());
                DOM::Element::EnginePseudoElementRecords pseudo_element_records {};
                DOM::Element::EnginePseudoElementDamages pseudo_element_damages {};
                for (auto next = reaction_index + 1; next < reactions.size() && reactions[next].style_node == published_reaction.style_node && reactions[next].pseudo_kind != NumericLimits<u8>::max(); ++next) {
                    auto const& pseudo_reaction = reactions[next];
                    pseudo_element_records[pseudo_reaction.pseudo_kind] = StyleRecordID { pseudo_reaction.new_style_record };
                    if (pseudo_reaction.record_damage & to_underlying(StyleEngineFFI::FfiStyleInvalidationField::EngineComputed))
                        pseudo_element_damages[pseudo_reaction.pseudo_kind] = DOM::Element::EngineRecordDamage { StyleRecordID { pseudo_reaction.old_style_record }, pseudo_reaction.record_damage, StyleRecordID { published_reaction.new_style_record } };
                }
                // The row's own effects come with the decision that settled it, whether or not
                // the record is the one that installs: a C++ computation of this element runs the
                // transition step itself, so the debt is discharged either way. The engine took
                // them as it published the row.
                auto const explicit_inheritance_debt = row_debts.explicit_inheritance;
                auto const row_effect_debt = row_debts.row_effect;
                row_debts = {};
                auto const row_sampled_in_pass = StyleEngineFFI::style_engine_take_row_sampled_in_pass(document.style_computer().style_engine().rust_handle(), reaction.style_node);
                auto const transition_debt = row_effect_debt & StyleEngine::SettledRowTransitionDebt;
                if (row_effect_debt & StyleEngine::SettledRowOwesAnAnimationPlan)
                    animation_plan = document.style_computer().take_settled_animation_plan(scope, StyleNodeID { reaction.style_node }, NumericLimits<u8>::max());
                bool const has_animations_or_plan = animation_plan.has_value() || element->has_relevant_animations()
                    || element->has_associated_animations();
                if (!engine_computed_record_environment_is_installable(*element, StyleRecordID { reaction.new_style_record })
                    || declarations_changed_during_apply(StyleNodeID { reaction.style_node })) {
                    // The record names declarations or an environment an earlier row of this batch
                    // has since moved. The move schedules the next transaction, whose pass computes
                    // this element again over them.
                    // A row the engine answered again over what the previous wave moved names what
                    // the element's inputs are now: declining it twice would ask for it forever.
                    ASSERT(!rows_declined_by_previous_wave.contains(StyleNodeID { reaction.style_node }));
                    if (rows_declined_by_previous_wave.contains(StyleNodeID { reaction.style_node }))
                        declined_a_row_again = true;
                    StyleEngineFFI::style_engine_note_host_step(StyleEngineFFI::FfiStyleHostStep::DeclinedRow);
                    declined_rows.set(StyleNodeID { reaction.style_node });
                    row_effects.append(StyleEffectDrain::DiscardContainerQueryEffects { StyleNodeID { reaction.style_node } });
                    for (size_t kind = 0; kind < pseudo_element_records.size(); ++kind) {
                        if (pseudo_element_records[kind].has_value())
                            (void)document.style_computer().take_settled_animation_plan(scope, StyleNodeID { reaction.style_node }, static_cast<u8>(kind));
                    }
                    document.style_computer().style_engine().record_derived_element_style_input_change(StyleNodeID { reaction.style_node }, StyleEngine::RecomputeStyle);
                } else {
                    bool const defer_pseudos = has_animations_or_plan
                        || row_effect_debt & (StyleEngine::SettledRowTransitionDebt | StyleEngine::SettledRowOwesAnAnimationSample);
                    auto old_originating_style = element->computed_style();
                    auto const old_style_record = element->style_record_identity();
                    bool const old_is_list_item = old_originating_style && old_originating_style->display().is_list_item();
                    auto const previous_pseudo_deferral = g_deferring_engine_pseudo_installation;
                    g_deferring_engine_pseudo_installation = defer_pseudos;
                    ScopeGuard restore_pseudo_deferral = [&] { g_deferring_engine_pseudo_installation = previous_pseudo_deferral; };
                    // The host samples the element's animations over the record the row installs. A
                    // composition the element held before is one the sample composes again, so the
                    // row is compared once, after the sample.
                    if (old_originating_style
                        && (animation_plan.has_value() || element->has_relevant_animations() || element->has_associated_animations()
                            || row_effect_debt & StyleEngine::SettledRowOwesAnAnimationSample))
                        engine_record_comparison = DOM::Element::EngineRecordComparison::AfterSample;
                    apply_engine_computed_records(pseudo_element_records, &pseudo_element_damages);
                    DOM::AbstractElement settled { *element };
                    // A row the pass sampled over the stack its plan leaves leaves the plan to the
                    // drain once the composition installs; the host's own sample reads the
                    // animations the plan applies.
                    Optional<StyleComputer::SettledAnimationPlan> plan_after_pass_sample;
                    if (animation_plan.has_value() && row_sampled_in_pass.present) {
                        plan_after_pass_sample = move(*animation_plan);
                    } else if (animation_plan.has_value()) {
                        document.style_computer().apply_settled_animation_plan(settled, *animation_plan);
                        row_effects.append(StyleEffectDrain::AnimationNames { StyleNodeID { reaction.style_node } });
                    }
                    bool installed_pseudo_animation_plan = false;
                    auto apply_pseudo_animation_plan = [&](size_t kind) {
                        if (kind >= pseudo_element_records.size())
                            return;
                        if (!pseudo_element_records[kind].has_value() || !*pseudo_element_records[kind])
                            return;
                        auto pseudo_plan = document.style_computer().take_settled_animation_plan(scope, StyleNodeID { reaction.style_node }, static_cast<u8>(kind));
                        if (!pseudo_plan.has_value())
                            return;
                        DOM::AbstractElement pseudo { *element, static_cast<PseudoElement>(kind) };
                        document.style_computer().apply_settled_animation_plan(pseudo, *pseudo_plan);
                        installed_pseudo_animation_plan = true;
                    };
                    // CSS animation ordering follows the pseudo tree order, which differs from
                    // the enum's order. Creation order breaks ties in the host's effect stack.
                    apply_pseudo_animation_plan(to_underlying(PseudoElement::Marker));
                    apply_pseudo_animation_plan(to_underlying(PseudoElement::Before));
                    for (size_t kind = 0; kind < pseudo_element_records.size(); ++kind) {
                        if (kind == to_underlying(PseudoElement::Marker)
                            || kind == to_underlying(PseudoElement::Before)
                            || kind == to_underlying(PseudoElement::After))
                            continue;
                        apply_pseudo_animation_plan(kind);
                    }
                    apply_pseudo_animation_plan(to_underlying(PseudoElement::After));
                    bool const has_animation_effects = element->has_relevant_animations() || element->has_associated_animations();
                    // https://drafts.csswg.org/css-transitions-2/#defining-before-change-style
                    // Sampling the installed record can pin the epoch's baseline, and the record the
                    // element holds by then is the after-change one. A row that owes the whole step
                    // pins the record it moved away from first.
                    if (transition_debt == 2 && document.is_in_style_stabilization_epoch() && settled.has_style())
                        (void)document.style_computer().record_transition_stabilization_baseline(scope, settled, StyleRecordID { reaction.old_style_record });
                    bool const compares_after_sample = engine_record_comparison == DOM::Element::EngineRecordComparison::AfterSample;
                    auto const row_sample_invalidation = compares_after_sample ? SampleInvalidation::AppliedByCaller : SampleInvalidation::Applied;
                    bool const installed_pass_sample = row_sampled_in_pass.present && settled.has_style()
                        && install_composition_sampled_in_pass(scope, settled, row_sampled_in_pass, row_sample_invalidation);
                    if (plan_after_pass_sample.has_value()) {
                        // The transition step below samples the element's animations as the plan
                        // leaves them.
                        if (installed_pass_sample && transition_debt != 2)
                            row_effects.append(StyleEffectDrain::AnimationPlan { StyleNodeID { reaction.style_node }, plan_after_pass_sample.release_value() });
                        else
                            document.style_computer().apply_settled_animation_plan(settled, *plan_after_pass_sample);
                        row_effects.append(StyleEffectDrain::AnimationNames { StyleNodeID { reaction.style_node } });
                    }
                    if (!installed_pass_sample && settled.has_style() && (has_animation_effects || animation_plan.has_value() || row_effect_debt & StyleEngine::SettledRowOwesAnAnimationSample))
                        sample_animations_for_installed_record(settled, row_sample_invalidation);
                    if (compares_after_sample)
                        invalidation = element->compare_engine_computed_style_record_after_sample(scope, old_style_record, invalidation, &row_effects);
                    // The step runs here rather than after the batch: a descendant applied later
                    // reads this element's after-change style, which is what the step decides
                    // against, and the C++ computation this row replaces runs it inside itself.
                    // A row that owes only the registration leaves nothing for the host: the step
                    // reads the element's transition declarations from the installed record.
                    if (transition_debt == 2
                        || (transition_debt == 0 && reaction.old_style_record != 0 && element->associated_shadow_host_pseudo_element().has_value())) {
                        DOM::AbstractElement settled { *element };
                        if (settled.has_style()) {
                            // A step the pass decided is applied as it decided it, its composition
                            // already installed.
                            auto const decided = transition_debt == 2
                                ? StyleEngineFFI::style_engine_take_transition_step_decided_in_pass(scope.engine().rust_handle(), reaction.style_node)
                                : StyleEngineFFI::FfiTransitionStepDecidedInPass {};
                            auto step_invalidation = document.style_computer().run_transition_step_for_installed_record(scope,
                                settled, StyleRecordID { reaction.old_style_record }, decided.present ? &decided : nullptr);
                            if (!step_invalidation.is_none()) {
                                row_effects.append(StyleEffectDrain::ElementInvalidation { StyleNodeID { reaction.style_node }, step_invalidation });
                                transaction_invalidation |= step_invalidation;
                            }
                        }
                    }
                    // A descendant may read this row only after its effect decisions have
                    // published their final composition. Keep the old composition pinned until
                    // that point so transition selection can still read its before-change style.
                    document.style_computer().style_engine().set_sampled_composition_identity(
                        StyleNodeID { reaction.style_node }, element->style_record_identity());
                    if (defer_pseudos) {
                        if (old_originating_style) {
                            auto const new_style = element->computed_style();
                            if (old_originating_style->base_values().display().is_none() != new_style->base_values().display().is_none())
                                row_effects.append(StyleEffectDrain::DisplayNoneAnimations { StyleNodeID { reaction.style_node } });
                            element->apply_display_none_change(scope, false, !old_originating_style->display().is_none() && new_style->display().is_none());
                        }
                        StyleEngineFFI::style_engine_note_host_step(StyleEngineFFI::FfiStyleHostStep::PseudoSettle);
                        auto settled_pseudos = document.style_computer().style_engine().settle_pseudo_records_after_host_record(StyleNodeID { reaction.style_node }, old_is_list_item);
                        DOM::Element::EnginePseudoElementRecords final_pseudo_records {};
                        for (size_t kind = 0; kind < array_size(settled_pseudos.pseudo_records); ++kind) {
                            if ((settled_pseudos.pseudo_records_present >> kind) & 1)
                                final_pseudo_records[kind] = StyleRecordID { settled_pseudos.pseudo_records[kind] };
                        }
                        g_deferring_engine_pseudo_installation = previous_pseudo_deferral;
                        auto pseudo_invalidation = element->install_engine_pseudo_element_records_after_sample(
                            scope, did_change_custom_properties, old_is_list_item,
                            old_originating_style ? &*old_originating_style : nullptr,
                            &final_pseudo_records, &row_effects);
                        invalidation |= pseudo_invalidation;
                    }
                    if (element->has_associated_animations() || installed_pseudo_animation_plan)
                        sample_animations_for_installed_pseudos(*element);
                    row_effects.append(StyleEffectDrain::AcknowledgeRecord { StyleNodeID { reaction.style_node } });
                    if (explicit_inheritance_debt != 0)
                        row_effects.append(StyleEffectDrain::ExplicitInheritance { StyleNodeID { reaction.style_node }, explicit_inheritance_debt });
                }
            }

            auto const* current_inherited_box_values = element->style_group<ComputedValues::InheritedBoxValues>();
            if (previous_visibility.has_value() && current_inherited_box_values
                && *previous_visibility != static_cast<Visibility>(current_inherited_box_values->visibility)) {
                document.throttled_animation_visibility_changed();
            }

            bool const row_computed_damage_itself = DOM::style_row_computed_damage_itself(*element);
            // A counter-style rebuild is the row's effect, not a move of style its children react to.
            auto effects = invalidation;
            effects |= DOM::end_style_row_counter_style_invalidation(*element);
            row_effects.append(StyleEffectDrain::ElementInvalidation { StyleNodeID { reaction.style_node }, effects });
            transaction_invalidation |= effects;

            // The environment moved. Where the engine moved the descendants' environments as it
            // settled the row, their records follow the row in the batch. Otherwise the row reports
            // the move, and the next transaction's pass computes the children over the moved
            // environment.
            bool const reports_environment_move = did_change_custom_properties && !(reaction.record_damage & to_underlying(StyleEngineFFI::FfiStyleInvalidationField::EnvironmentMovedInPass));
            // The children of a row the engine derived them for have their reactions in the batch
            // already. A row that installed another record than the one the engine derived them
            // from, or whose move the host compared itself, derives them here.
            if (engine_derived_children && !row_computed_damage_itself && !reports_environment_move && element->style_record_identity().value() == reaction.new_style_record)
                continue;
            u32 facts = style_row_start_facts(*element, row_start);
            if (reports_environment_move)
                facts |= StyleEngine::DidChangeCustomProperties;
            if (invalidation.is_none())
                facts |= StyleEngine::InvalidationIsNone;
            if (invalidation.needs_layout_tree_rebuild())
                facts |= StyleEngine::NeedsLayoutTreeRebuild;
            if (invalidation.recompute_descendant_styles)
                facts |= StyleEngine::RecomputeDescendants;
            if (element->children_explicitly_inherited_non_inherited_style_groups() != 0)
                facts |= StyleEngine::ChildrenExplicitlyInherit;
            if (auto shadow_root = element->shadow_root(); shadow_root && shadow_root->children_explicitly_inherited_non_inherited_style_groups() != 0)
                facts |= StyleEngine::ShadowChildrenExplicitlyInherit;
            document.style_computer().style_engine().record_applied_style_reaction(reaction.style_node, reaction.reaction, invalidation.inherited_style_groups_changed(), facts);
        }
    }

    // The batch is installed: drain what its rows left behind, in the order they were applied.
    row_effects.apply(scope, document);
    return transaction_invalidation;
}

// Install a published style batch inside the effect drain: close it over the unstyled inheritance
// prerequisites of its rows, apply the rows in preorder, and apply the effects they leave.
static RequiredInvalidationAfterStyleChange install_style_batch(StyleDrainScope const& scope, DOM::Document& document, Vector<StyleEngine::PublishedStyleDelta>& style_engine_reactions, size_t published_reaction_count, HashTable<StyleNodeID>& declined_rows, bool& declined_a_row_again)
{
    RequiredInvalidationAfterStyleChange invalidation;
    // The engine closed the batch over the elements its rows inherit from and ordered it for
    // direct application as it published it.
    if (style_engine_reactions.is_empty())
        return invalidation;
    auto& counters = document.style_invalidation_counters();
    if (published_reaction_count > 0) {
        ++counters.style_engine_reaction_batch_runs;
        counters.style_engine_reaction_elements += published_reaction_count;
    }
    invalidation = apply_style_engine_reactions(scope, document, style_engine_reactions, declined_rows, declined_a_row_again);
    style_engine_reactions.clear();
    return invalidation;
}

// The rows of a batch the update gives up on hand back the debts the engine took with them.
static void restore_unsettled_row_debts(DOM::Document& document, ReadonlySpan<StyleEngine::PublishedStyleDelta> reactions)
{
    StyleEffectDrain drain;
    for (auto const& reaction : reactions) {
        if (reaction.explicit_inheritance_debt != 0 || reaction.row_effect_debt != 0)
            drain.append(StyleEffectDrain::RestoreRowDebts { StyleNodeID { reaction.style_node }, reaction.explicit_inheritance_debt, static_cast<u8>(reaction.row_effect_debt) });
    }
    drain.apply(document);
}

// A style update of a document. Its first transaction's pass can run beside the main thread
// (LIBWEB_STAGE_OVERLAP=style): the update then lives on the heap between begin() and finish(), and
// every scope it holds open stays open while the pass is in flight.
class StyleUpdate {
    AK_ALLOC_WITH_KMALLOC;
    AK_MAKE_NONCOPYABLE(StyleUpdate);
    AK_MAKE_NONMOVABLE(StyleUpdate);

public:
    explicit StyleUpdate(DOM::Document& document)
        : m_document(document)
        , m_started_at(MonotonicTime::now())
    {
        auto& timing_counters = document.style_invalidation_counters();
        m_submission_before = timing_counters.style_update_submission_microseconds;
        m_bridge_before = timing_counters.style_update_bridge_microseconds;
        m_apply_before = timing_counters.style_update_apply_microseconds;
    }

    ~StyleUpdate();

    // Runs the update up to its first transaction. Returns false if the update has nothing to take.
    bool begin(DocumentWithoutBrowsingContext);
    // Takes the first transaction in place and runs the rest of the update.
    void take_and_finish();
    // Submits the first transaction's pass. Returns false, and submits nothing, if the document has no root to take
    // it for: the update then finishes in place.
    bool submit();
    // Runs the rest of the update once the frame of its submitted pass has been taken back.
    void finish_submitted();

private:
    void finish(StyleEngineTransaction);

    DOM::Document& m_document;
    MonotonicTime m_started_at;
    u64 m_submission_before { 0 };
    u64 m_bridge_before { 0 };
    u64 m_apply_before { 0 };
    bool m_began_style_update { false };
    bool m_began_complete_style_update { false };
    bool m_took_transaction { false };
};

StyleUpdate::~StyleUpdate()
{
    auto& document = m_document;
    if (m_took_transaction) {
        document.style_computer().style_engine().set_published_batch_waits(false);
        StyleEffectDrain::install(document, [](StyleDrainScope const& scope) {
            scope.engine().discard_style_transaction_outputs(scope);
        });
    }
    if (m_began_complete_style_update)
        finish_complete_style_update(document);
    if (m_began_style_update) {
        document.end_style_stabilization_epoch();
        document.style_computer().end_style_record_view_epoch();
        document.style_computer().end_style_update();
    }
    auto& timing_counters = document.style_invalidation_counters();
    auto whole = (MonotonicTime::now() - m_started_at).to_truncated_microseconds();
    auto measured = timing_counters.style_update_submission_microseconds - m_submission_before
        + timing_counters.style_update_bridge_microseconds - m_bridge_before
        + timing_counters.style_update_apply_microseconds - m_apply_before;
    timing_counters.style_update_microseconds += whole;
    // NB: Counters are read only to attribute time, never to choose style work. The
    //     intervals are disjoint; rounding each down leaves fractional time here too.
    timing_counters.style_update_remainder_microseconds += whole - measured;
}

bool StyleUpdate::begin(DocumentWithoutBrowsingContext document_without_browsing_context)
{
    auto& document = m_document;
    auto& timing_counters = document.style_invalidation_counters();
    // NOTE: If our parent document needs a relayout, we must do that *first*. This is required as it may cause the
    // viewport to change which will can affect media query evaluation and the value of the `vw` unit.
    if (auto navigable = document.navigable(); navigable && navigable->container() && &navigable->container()->document() != &document)
        navigable->container()->document().update_layout(DOM::UpdateLayoutReason::ChildDocumentStyleUpdate);

    if (!document.browsing_context() && document_without_browsing_context == DocumentWithoutBrowsingContext::Skip)
        return false;

    // NOTE: If this is a document hosting <template> contents, style update is unnecessary.
    if (document.created_for_appropriate_template_contents())
        return false;

    auto submission_started_at = MonotonicTime::now();
    document.style_computer().begin_style_update();
    document.style_computer().begin_style_record_view_epoch();
    document.synchronize_dirty_style_attributes();
    document.begin_style_stabilization_epoch();
    m_began_style_update = true;

    // Fetch the viewport rect once, instead of repeatedly, during style computation.
    document.update_style_computer_viewport_rect();

    // An element may have rendering-only descendants that must join the transaction which first styles it. Prepare
    // those descendants before selector inputs cross the transaction boundary.
    document.style_computer().prepare_elements_for_style_computation();

    // Media rules are evaluated before the transaction boundary below, because evaluating them is
    // itself a source of inputs: a rule that starts or stops applying publishes its activation. A
    // transaction taken ahead of that would leave those inputs for the next flush, so the flush that made
    // a rule apply would not be the flush that recomputed the elements it applies to.
    if (document.needs_media_rule_evaluation())
        document.evaluate_media_rules_for_style_update();

    // Publish the complete document environment while DOM and page state are still available. Variable
    // substitution borrows this immutable snapshot for the rest of the update instead of calling back into
    // the document after the style-stage seal has opened.
    (void)document.style_computer().ensure_media_environment_for_style_update();
    (void)document.style_computer().ensure_document_environment_for_style_update();
    document.publish_animation_environment_for_style_update();
    document.style_computer().style_engine().prepare_root_font_resolution(
        document.font_computer().environment_generation());
    StyleValueFFI::rust_style_ffi_complete_style_update_begin();
    m_began_complete_style_update = true;

    // The user-agent and user sheets have no author-sheet attachment event, so compare their
    // identities before deciding whether there is a transaction to take. Rendering opportunities
    // call update_style() even for quiescent documents, and animation ticks do not themselves
    // change selector or cascade inputs. Apply an animation-only update first, then take a
    // transaction only if the resulting inherited-style feedback requires one.
    record_non_author_stylesheets(document);
    timing_counters.style_update_submission_microseconds += (MonotonicTime::now() - submission_started_at).to_truncated_microseconds();
    if (document.has_completed_style_update()
        && !document.style_computer().style_engine().has_pending_transaction()) {
        document.sample_animation_effects_needing_style_update();
        if (!document.style_computer().style_engine().has_pending_transaction())
            return false;
    }

    // Publish each tree scope's counter-style registry before the engine answers any rows.
    // Shadow scopes can override names, and a warm transaction can already read a registry
    // changed by a stylesheet edit; neither can wait for C++ to resolve a list style on demand.
    (void)document.style_scope().counter_style_environment_identity();
    document.for_each_shadow_root([](DOM::ShadowRoot& shadow_root) {
        (void)shadow_root.style_scope().counter_style_environment_identity();
    });
    return true;
}

void StyleUpdate::take_and_finish()
{
    // A style flush is a transaction boundary. Everything recorded since the last one crosses into
    // StyleEngine as one flat batch, is normalized there, and is routed into the region its
    // transpose programs reach. A transaction that could not be proven narrower publishes a
    // complete document reaction batch. Only a transaction that cannot complete its answers falls
    // back to document invalidation.
    auto style_engine_transaction = take_style_engine_transaction(m_document);
    m_took_transaction = true;
    finish(move(style_engine_transaction));
}

bool StyleUpdate::submit()
{
    auto root = begin_style_engine_transaction(m_document);
    m_took_transaction = true;
    if (!root.has_value()) {
        finish({});
        return false;
    }
    auto& style_engine = m_document.style_computer().style_engine();
    style_engine.submit_style_transaction(*root);
    // Inputs published beside the pass are ones it does not see: they wait for its drain.
    style_engine.set_published_batch_waits(true);
    return true;
}

void StyleUpdate::finish_submitted()
{
    VERIFY(m_took_transaction);
    auto& style_engine = m_document.style_computer().style_engine();
    auto style_engine_transaction = accept_style_engine_transaction(m_document, style_engine.finish_submitted_style_transaction());
    // An empty batch leaves nothing waiting for the host: the inputs published beside the pass go through now, as
    // they would have before a transaction taken in place.
    if (style_engine_transaction.reactions.is_empty())
        style_engine.set_published_batch_waits(false);
    finish(move(style_engine_transaction));
}

void StyleUpdate::finish(StyleEngineTransaction style_engine_transaction)
{
    auto& document = m_document;
    auto& timing_counters = document.style_invalidation_counters();
    if (!style_engine_transaction.reactions.is_empty())
        document.note_style_stabilization_has_style_reactions();
    document.sample_animation_effects_needing_style_update();

    auto style_engine_reactions = move(style_engine_transaction.reactions);
    auto transaction_only_derived_child_reactions = style_engine_transaction.only_derived_child_reactions;
    if (style_engine_reactions.is_empty()
        && document.style_computer().style_engine().has_pending_transaction()) {
        StyleEngineFFI::style_engine_note_host_step(StyleEngineFFI::FfiStyleHostStep::Wave);
        auto feedback_transaction = take_style_engine_transaction(document);
        style_engine_reactions = move(feedback_transaction.reactions);
        transaction_only_derived_child_reactions = feedback_transaction.only_derived_child_reactions;
    }

    document.build_registered_properties_cache_for_style_update();

    // This pass belongs to the current style change event. The outer stabilization epoch advanced
    // the transition generation once, before any style, animation, or layout feedback ran.
    document.record_style_stabilization_pass();

    if (style_engine_reactions.is_empty())
        return;

    // No script runs while the host installs the batches of this update, so the timelines and
    // every animation a batch does not install hold still: they are published once for all of them.
    Animations::AnimationUpdateContext::BatchPublication animation_publication { document };
    RequiredInvalidationAfterStyleChange invalidation;
    constexpr size_t max_style_update_passes = 8;
    size_t style_update_pass = 0;
    size_t style_reaction_pass = 0;
    // Far above the flat tree depths a style change reaches by derived reactions alone.
    constexpr size_t max_style_reaction_waves = 16384;
    HashTable<StyleNodeID> declined_rows;
    while (!style_engine_reactions.is_empty()) {
        auto apply_started_at = MonotonicTime::now();
        ArmedScopeGuard record_apply_time = [&] {
            timing_counters.style_update_apply_microseconds += (MonotonicTime::now() - apply_started_at).to_truncated_microseconds();
        };
        // Waves of derived reactions do not count as passes, but a style change that keeps
        // producing them is a feedback loop, such as an animation overlay recomputing its element
        // over and over. Stop it rather than let it grow without end.
        if (style_reaction_pass >= max_style_reaction_waves) {
            dbgln("FIXME: Style update stopped after {} waves of reactions", style_reaction_pass);
            ++document.style_invalidation_counters().style_update_pass_guard_hits;
            break;
        }
        // One more tree generation of the same style change is not a new pass of it.
        if (style_reaction_pass++ > 0 && !transaction_only_derived_child_reactions)
            document.record_style_stabilization_pass();

        size_t published_reaction_count = 0;
        for (auto const& reaction : style_engine_reactions) {
            if (reaction.reaction & StyleEngine::PublishedStyle)
                ++published_reaction_count;
        }
        if (published_reaction_count > 0 && !transaction_only_derived_child_reactions) {
            if (++style_update_pass > max_style_update_passes) {
                ++document.style_invalidation_counters().style_update_pass_guard_hits;
                restore_unsettled_row_debts(document, style_engine_reactions);
                break;
            }
        }

        bool declined_a_row_again = false;
        StyleEffectDrain::install(document, [&](StyleDrainScope const& scope) {
            invalidation |= install_style_batch(scope, document, style_engine_reactions, published_reaction_count, declined_rows, declined_a_row_again);
        });
        // NB: A wave of derived reactions is not a pass of the style change, but one that declines
        //     a row again makes no progress, and counts against the passes of the update.
        if (declined_a_row_again && ++style_update_pass > max_style_update_passes) {
            ++document.style_invalidation_counters().style_update_pass_guard_hits;
            break;
        }

        timing_counters.style_update_apply_microseconds += (MonotonicTime::now() - apply_started_at).to_truncated_microseconds();
        record_apply_time.disarm();

        // Exact consequences produced while recomputing become the next transaction in this
        // stabilization epoch. Take it only after consuming the current published answers, since
        // a new transaction retires their scratch.
        if (document.style_computer().style_engine().has_pending_transaction()) {
            StyleEngineFFI::style_engine_note_host_step(StyleEngineFFI::FfiStyleHostStep::Wave);
            auto next_transaction = take_style_engine_transaction(document);
            style_engine_reactions = move(next_transaction.reactions);
            transaction_only_derived_child_reactions = next_transaction.only_derived_child_reactions;
        }
        if (style_engine_reactions.is_empty())
            break;
    }

    document.set_has_completed_style_update();
    apply_document_style_invalidation_after_style_change(document, invalidation);
    document.sample_animation_effects_needing_style_update();
}

static void update_style(DOM::Document& document, DocumentWithoutBrowsingContext document_without_browsing_context)
{
    StyleUpdate update { document };
    if (update.begin(document_without_browsing_context))
        update.take_and_finish();
}

// The style update whose first pass is in flight, which it owns. At most one frame is in flight per event loop.
static StyleUpdate* s_submitted_style_update = nullptr;

static bool submit_style_update(DOM::Document& document)
{
    VERIFY(!s_submitted_style_update);
    auto update = make<StyleUpdate>(document);
    if (!update->begin(DocumentWithoutBrowsingContext::Skip))
        return false;
    if (!update->submit())
        return false;
    s_submitted_style_update = update.leak_ptr();
    return true;
}

static void finish_submitted_style_update(DOM::Document& document)
{
    VERIFY(s_submitted_style_update);
    auto update = adopt_own(*exchange(s_submitted_style_update, nullptr));
    // What was marked beside the pass is what the next drain writes.
    document.release_held_invalidation_marks();
    update->finish_submitted();
}

// What a targeted materialization of one element found, reported to the engine the way a reaction
// pass reports it, so the element's children get the same derived reactions either way.
static void note_targeted_style_reaction_applied(StyleDrainScope const& scope, DOM::Element& element, StyleRowStart const& row_start, RequiredInvalidationAfterStyleChange const& invalidation, bool did_change_custom_properties, bool descendant_style_recompute_needed)
{
    auto& style_engine = scope.engine();
    u8 reaction = StyleEngine::PublishedStyle | StyleEngine::RecomputeStyle;
    if (descendant_style_recompute_needed)
        reaction |= StyleEngine::RecomputeDescendantStyles;
    u32 facts = style_row_start_facts(element, row_start);
    if (did_change_custom_properties)
        facts |= StyleEngine::DidChangeCustomProperties;
    if (invalidation.is_none())
        facts |= StyleEngine::InvalidationIsNone;
    if (invalidation.needs_layout_tree_rebuild())
        facts |= StyleEngine::NeedsLayoutTreeRebuild;
    if (invalidation.recompute_descendant_styles)
        facts |= StyleEngine::RecomputeDescendants;
    if (element.children_explicitly_inherited_non_inherited_style_groups() != 0)
        facts |= StyleEngine::ChildrenExplicitlyInherit;
    if (auto shadow_root = element.shadow_root(); shadow_root && shadow_root->children_explicitly_inherited_non_inherited_style_groups() != 0)
        facts |= StyleEngine::ShadowChildrenExplicitlyInherit;
    style_engine.record_applied_style_reaction(element.style_node_id(), reaction, invalidation.inherited_style_groups_changed(), facts);
}

static void apply_targeted_style_invalidation(StyleDrainScope const& scope, DOM::Element& element, StyleRowStart const& row_start, RequiredInvalidationAfterStyleChange const& invalidation, RequiredInvalidationAfterStyleChange const& counter_style_invalidation, bool did_change_custom_properties, bool descendant_style_recompute_needed)
{
    if (!invalidation.is_none() || did_change_custom_properties)
        Invalidation::invalidate_assigned_slottables_after_slot_style_change(element);
    // A counter-style rebuild is the element's effect, not a move of style its children react to.
    auto effects = invalidation;
    effects |= counter_style_invalidation;
    apply_element_style_invalidation_after_style_change(element, effects);
    note_targeted_style_reaction_applied(scope, element, row_start, invalidation, did_change_custom_properties, descendant_style_recompute_needed);
    apply_document_style_invalidation_after_style_change(element.document(), effects);
}

// The engine answers a targeted read's demand for one element's record, or for one of its
// pseudo-elements', in a style stage run of its own, before the host installs the answer.
static StyleEngineFFI::FfiRecordDemandAnswer answer_targeted_record_demand(DOM::Element& element, Optional<PseudoElement> pseudo_element = {})
{
    auto& engine = element.document().style_computer().style_engine();
    return StyleEngineFFI::style_engine_answer_read_demand(engine.rust_handle(), element.style_node_id().value(),
        pseudo_element.has_value() ? to_underlying(*pseudo_element) : NumericLimits<u8>::max(), false, true, pseudo_element.has_value(), 0);
}

// Install the engine's answer for a targeted demand of one element.
static Optional<RequiredInvalidationAfterStyleChange> install_targeted_record_demand_answer(StyleDrainScope const& scope, DOM::Element& element, StyleEngineFFI::FfiRecordDemandAnswer const& answer, bool& did_change_custom_properties)
{
    auto& style_computer = element.document().style_computer();
    auto& engine = scope.engine();

    // The engine answers every element of the document it hosts, over the custom-property
    // environment its installed ancestors hold. Should an answer not install, the element keeps
    // the record it has, and the style stage seal reports the row.
    bool environment_is_installable = false;
    if (answer.record.style_record)
        (void)element.custom_property_environment_of_engine_record(StyleRecordID { answer.record.style_record }, environment_is_installable);
    ASSERT(environment_is_installable);
    if (!environment_is_installable) {
        static constexpr u8 refused_host_entry = 1;
        engine.note_host_entry(element.style_node_id(), refused_host_entry, element.has_style() ? 0 : 1 << 2);
        return {};
    }

    DOM::Element::EnginePseudoElementRecords pseudo_element_records {};
    for (size_t kind = 0; kind < array_size(answer.record.pseudo_records); ++kind) {
        if (answer.record.pseudo_records_present & (1 << kind))
            pseudo_element_records[kind] = StyleRecordID { answer.record.pseudo_records[kind] };
    }
    auto old_style_record = element.style_record_identity();
    // A targeted record that drops the composition the element held is one its animations are
    // sampled over again. Compare it once, after that sample, or each step sees every animated
    // value move and asks for layout even when the composed style is unchanged.
    auto const old_style = element.computed_style();
    bool const samples_over_the_record = old_style
        && engine.style_record_view(old_style_record).animation_overlay_identity != 0
        && engine.style_record_view(StyleRecordID { answer.record.style_record }).animation_overlay_identity == 0;
    auto invalidation = element.apply_engine_computed_style_record(scope, StyleRecordID { answer.record.style_record }, pseudo_element_records, answer.record.uses_substitution, answer.row_facts, did_change_custom_properties,
        samples_over_the_record ? DOM::Element::EngineRecordComparison::AfterSample : DOM::Element::EngineRecordComparison::AtInstallation);
    if (!!old_style_record && element.associated_shadow_host_pseudo_element().has_value())
        invalidation |= style_computer.run_transition_step_for_installed_record(scope, { element }, old_style_record);
    auto container_effects = StyleEngineFFI::style_engine_take_container_effects(engine.rust_handle(), element.style_node_id().value());
    ScopeGuard release_container_effects = [&] { StyleEngineFFI::style_engine_native_container_effects_release(container_effects.effects); };
    StyleComputer::record_container_query_effects(scope, DOM::AbstractElement { element }, container_effects);
    engine.acknowledge_engine_computed_record(element.style_node_id());
    if (samples_over_the_record) {
        sample_animations_for_installed_record(DOM::AbstractElement { element }, SampleInvalidation::AppliedByCaller);
        invalidation = element.compare_engine_computed_style_record_after_sample(scope, old_style_record, invalidation);
    }
    return invalidation;
}

// A targeted read of an unstyled hidden animation target installs its base record first. Sample its
// effects over that record now: the document's ordinary animation tick skips hidden descendants.
static void sample_animations_of_newly_styled_target(DOM::Element& element)
{
    Animations::AnimationUpdateContext context;
    for (auto& animation : element.associated_animations_in_composite_order()) {
        if (animation->is_idle() || !animation->effect() || !is<Animations::KeyframeEffect>(*animation->effect()))
            continue;
        auto& effect = static_cast<Animations::KeyframeEffect&>(*animation->effect());
        if (effect.target().ptr() != &element || effect.pseudo_element_type().has_value())
            continue;
        effect.update_computed_properties_for_style(context, DOM::AbstractElement { element });
    }
}

// The pseudo-elements a scroll-state container's pseudo rules can style.
static constexpr Array scroll_state_container_pseudo_elements { PseudoElement::Before, PseudoElement::After, PseudoElement::FirstLetter, PseudoElement::Marker };

// Install the records a targeted read demands for the inheritance chain, from its topmost stale
// element down. Each record is answered by the engine before a drain installs it. False when the
// walk stops at a display:none element.
static bool install_targeted_styles(DOM::Document& document, GC::RootVector<GC::Ref<DOM::Element>>& inheritance_chain, size_t topmost_element_to_recompute, StyleUpdateMode mode)
{
    bool descendant_style_recompute_needed = false;
    for (size_t i = topmost_element_to_recompute + 1; i > 0; --i) {
        auto& element = inheritance_chain[i - 1];
        bool const was_unstyled = !element->has_style();
        bool did_change_custom_properties = false;
        auto const row_start = style_row_start(*element);
        auto const answer = answer_targeted_record_demand(*element);
        Optional<RequiredInvalidationAfterStyleChange> installed;
        bool reads_scroll_state_pseudo_elements = false;
        auto finish_element = [&](StyleDrainScope const& scope) {
            auto invalidation = installed.value_or({});
            auto const counter_style_invalidation = DOM::end_style_row_counter_style_invalidation(*element);
            apply_targeted_style_invalidation(scope, element, row_start, invalidation, counter_style_invalidation, did_change_custom_properties, descendant_style_recompute_needed);
            descendant_style_recompute_needed |= invalidation.recompute_descendant_styles;
        };
        StyleEffectDrain::install(document, [&](StyleDrainScope const& scope) {
            DOM::begin_style_row_counter_style_invalidation(*element);
            installed = install_targeted_record_demand_answer(scope, element, answer, did_change_custom_properties);
            if (installed.has_value()) {
                if (was_unstyled && element->has_relevant_animations())
                    sample_animations_of_newly_styled_target(element);
                auto const* box_values = element->style_group<ComputedValues::BoxValues>();
                reads_scroll_state_pseudo_elements = box_values && box_values->is_scroll_state_container && element->style_depends_on_size_container_query();
            }
            if (!reads_scroll_state_pseudo_elements)
                finish_element(scope);
        });
        if (reads_scroll_state_pseudo_elements) {
            DOM::Element::EnginePseudoElementRecords pseudo_records {};
            u32 row_facts = 0;
            for (auto kind : scroll_state_container_pseudo_elements) {
                auto pseudo_answer = answer_targeted_record_demand(element, kind);
                pseudo_records[to_underlying(kind)] = StyleRecordID { pseudo_answer.record.style_record };
                row_facts = pseudo_answer.row_facts;
            }
            StyleEffectDrain::install(document, [&](StyleDrainScope const& scope) {
                *installed |= element->apply_engine_computed_style_record(scope, element->style_record_identity(), pseudo_records, false, row_facts, did_change_custom_properties);
                // The container's pseudo rules can change after its descendants finish style and
                // layout and the scroll-state snapshot is published.
                installed->recompute_descendant_styles = true;
                finish_element(scope);
            });
        }

        VERIFY(element->has_style());
        auto const* box_values = element->style_group<ComputedValues::BoxValues>();
        VERIFY(box_values);
        if (display_from_ffi_display(box_values->display).is_none()) {
            if (mode == StyleUpdateMode::StopAtDisplayNone)
                return false;
            descendant_style_recompute_needed = false;
        }

        if (did_change_custom_properties || (installed.has_value() && installed->needs_layout_tree_rebuild()))
            descendant_style_recompute_needed = true;
    }
    return true;
}

// A targeted style update has nothing to do when every source of style work in the document is settled: A full style
// update has completed, no style engine transaction or recorded input is pending, no media rule evaluation is queued,
// and no animated style refresh is due.
static bool document_has_no_pending_style_work(DOM::Document const& document)
{
    // A rootless flush drains the journal but preserves element style inputs for the first transaction with a document
    // root — so has_pending_transaction() alone would report a settled engine still owing an element its recomputation.
    return document.has_completed_style_update()
        && !document.style_computer().style_engine().has_pending_transaction()
        && !document.style_computer().style_engine().has_deferred_element_style_inputs()
        && !document.needs_media_rule_evaluation()
        && !document.needs_animated_style_update();
}

// Whether every embedding document up the container chain needs no style or layout work — so, bringing the embedding
// chain up to date couldn't invalidate anything in this document.
static bool embedding_document_chain_has_no_pending_style_or_layout_work(DOM::Document const& document)
{
    auto const* embedded_document = &document;
    while (auto navigable = embedded_document->navigable()) {
        auto container = navigable->container();
        if (!container || &container->document() == embedded_document)
            return true;
        auto& embedding_document = container->document();
        if (!document_has_no_pending_style_work(embedding_document)
            || !embedding_document.layout_is_up_to_date()
            || !container->has_style())
            return false;
        embedded_document = &embedding_document;
    }
    return true;
}

static bool update_style_for_element(DOM::Document& document, DOM::AbstractElement const& abstract_element, StyleUpdateMode mode)
{
    if (!abstract_element.element().is_connected())
        return false;
    document.ensure_style_engine_tracks_tree();

    // OPTIMIZATION: When nothing style-related is pending anywhere that could affect this document, the only question
    // left is, if the element's inheritance chain already has style. If it does, the walk below would conclude there's
    // nothing to recompute. So answer that directly — without constructing a style record view for every ancestor.
    if (mode == StyleUpdateMode::OnlyIfNeeded
        && !abstract_element.pseudo_element().has_value()
        && document_has_no_pending_style_work(document)
        && embedding_document_chain_has_no_pending_style_or_layout_work(document)) {
        bool inheritance_chain_has_style = abstract_element.element().has_style();
        for (auto cursor = abstract_element.element_to_inherit_style_from(); inheritance_chain_has_style && cursor.has_value(); cursor = cursor->element_to_inherit_style_from())
            inheritance_chain_has_style = cursor->element().has_style();
        if (inheritance_chain_has_style)
            return true;
    }

    document.style_computer().begin_style_update();
    ScopeGuard end_style_update = [&] {
        document.style_computer().end_style_update();
    };

    document.style_computer().begin_style_record_view_epoch();
    ScopeGuard end_style_record_view_epoch = [&] {
        document.style_computer().end_style_record_view_epoch();
    };

    bool complete_style_update_started = false;
    ScopeGuard leave_complete_style_update = [&] {
        if (complete_style_update_started)
            finish_complete_style_update(document);
    };

    // Refresh computed properties for an abstract element. An ordinary read first consumes the complete exact
    // reaction batch. A reentrant layout read leaves that transaction untouched and walks the flat-tree inheritance
    // chain, re-cascading from the rootmost stale element on the path back down to the target. Normal mode also
    // re-cascades the target path under display:none ancestors.

    bool embedding_document_layout_was_stale = false;
    if (auto navigable = document.navigable(); navigable && navigable->container() && &navigable->container()->document() != &document) {
        auto& container = *navigable->container();
        auto& embedding_document = container.document();
        update_style_for_element(embedding_document, DOM::AbstractElement { container }, StyleUpdateMode::OnlyIfNeeded);
        embedding_document_layout_was_stale = !embedding_document.layout_is_up_to_date();
        embedding_document.update_layout(DOM::UpdateLayoutReason::ChildDocumentStyleUpdate);
    }

    bool entered_stabilization_epoch = false;
    ScopeGuard end_stabilization_epoch = [&] {
        if (entered_stabilization_epoch)
            document.end_style_stabilization_epoch();
    };

    bool ran_regular_style_update = false;
    // NB: A document without a browsing context (for example, one from createHTMLDocument()) has no rendering
    //     opportunities to publish its elements to the style engine. A targeted read runs its transaction here, so
    //     the engine has the facts it matches the read element and its ancestors against.
    if (document.browsing_context() || !document.created_for_appropriate_template_contents()) {
        document.begin_style_stabilization_epoch();
        entered_stabilization_epoch = true;
        document.update_style_computer_viewport_rect();

        if (document.style_computer().style_engine().has_pending_transaction() || document.needs_media_rule_evaluation())
            document.note_style_stabilization_has_style_reactions();

        // Media query evaluation can enqueue normal style invalidations, so do it before deciding what pending
        // invalidation work needs to run.
        if (document.needs_media_rule_evaluation())
            document.evaluate_media_rules_for_style_update();

        // The embedding document has settled the viewport and media rules above. Snapshot the resulting
        // environment before any style computation can enter the sealed stage.
        (void)document.style_computer().ensure_media_environment_for_style_update();
        (void)document.style_computer().ensure_document_environment_for_style_update();
        document.publish_animation_environment_for_style_update();
        document.style_computer().style_engine().prepare_root_font_resolution(
            document.font_computer().environment_generation());
        StyleValueFFI::rust_style_ffi_complete_style_update_begin();
        complete_style_update_started = true;

        auto const can_run_regular_style_update = !document.is_running_update_layout()
            && (!document.has_completed_style_update()
                || document.style_computer().style_engine().has_pending_transaction());
        if (can_run_regular_style_update) {
            update_style(document, DocumentWithoutBrowsingContext::Update);
            ran_regular_style_update = true;
        } else {
            document.sample_animation_effects_needing_style_update();
            if (!document.is_running_update_layout()
                && document.style_computer().style_engine().has_pending_transaction()) {
                update_style(document, DocumentWithoutBrowsingContext::Update);
                ran_regular_style_update = true;
            }
        }
    }

    if (!complete_style_update_started) {
        (void)document.style_computer().ensure_media_environment_for_style_update();
        (void)document.style_computer().ensure_document_environment_for_style_update();
        document.publish_animation_environment_for_style_update();
        document.style_computer().style_engine().prepare_root_font_resolution(
            document.font_computer().environment_generation());
        StyleValueFFI::rust_style_ffi_complete_style_update_begin();
        complete_style_update_started = true;
    }

    // Element-backed pseudo-elements read their computed style from the element that backs them (for example,
    // ::details-content reads from the slot inside the details element's UA shadow tree). Close the originating
    // element's transaction before redirecting to the backing element, which can consume feedback from that
    // transaction and is not in the originating element's inheritance chain.
    if (abstract_element.pseudo_element().has_value() && is_element_reference_pseudo_element(*abstract_element.pseudo_element())) {
        if (auto pseudo_element = abstract_element.element().get_pseudo_element(*abstract_element.pseudo_element()); pseudo_element.has_value()) {
            if (auto const* element_reference = as_if<DOM::ElementReferencePseudoElement>(*pseudo_element))
                return update_style_for_element(document, DOM::AbstractElement { element_reference->referenced_element() }, mode);
        }
    }

    if (ran_regular_style_update && mode != StyleUpdateMode::OnlyIfNeeded) {
        auto style_record = abstract_element.style_record_identity();
        if (!!style_record
            && !has_flag(document.style_computer().style_engine().style_record_dependency_flags(style_record), StyleRecordDependencyFlag::InDisplayNoneSubtree))
            return true;
    }

    // Single walk up the inheritance chain: collect each ancestor and remember the index of the topmost display:none
    // entry seen. Pseudo-element styles are refreshed when the originating element is recomputed, so don't put the pseudo
    // on the path.
    GC::RootVector<GC::Ref<DOM::Element>> inheritance_chain;
    if (!abstract_element.pseudo_element().has_value())
        inheritance_chain.append(const_cast<DOM::Element&>(abstract_element.element()));

    Optional<size_t> topmost_display_none_index;
    Optional<size_t> topmost_element_requiring_style;
    for (auto cursor = abstract_element.element_to_inherit_style_from(); cursor.has_value(); cursor = cursor->element_to_inherit_style_from()) {
        auto& ancestor = const_cast<DOM::Element&>(cursor->element());
        inheritance_chain.append(ancestor);
    }

    for (size_t i = inheritance_chain.size(); i > 0; --i) {
        auto& ancestor = inheritance_chain[i - 1];
        if (!topmost_element_requiring_style.has_value()
            && (document.style_computer().style_engine().has_recorded_element_style_input_change(ancestor->style_node_id())
                || document.style_computer().style_engine().has_deferred_element_style_input(ancestor->style_node_id())
                || !ancestor->has_style())) {
            topmost_element_requiring_style = i - 1;
        }

        auto const* box_values = ancestor->style_group<ComputedValues::BoxValues>();
        if (box_values && display_from_ffi_display(box_values->display).is_none()) {
            topmost_display_none_index = i - 1;
            if (mode == StyleUpdateMode::StopAtDisplayNone && !topmost_element_requiring_style.has_value())
                return false;
        }
    }

    Optional<size_t> topmost_element_to_recompute = topmost_element_requiring_style;
    if (mode == StyleUpdateMode::Normal && topmost_display_none_index.has_value()) {
        if (!topmost_element_to_recompute.has_value() && *topmost_display_none_index > 0)
            topmost_element_to_recompute = *topmost_display_none_index - 1;
    }

    if (!topmost_element_to_recompute.has_value()) {
        if ((mode == StyleUpdateMode::Normal || embedding_document_layout_was_stale) && !inheritance_chain.is_empty())
            topmost_element_to_recompute = 0;
        else
            return abstract_element.has_style();
    }

    if (!install_targeted_styles(document, inheritance_chain, *topmost_element_to_recompute, mode))
        return false;
    return abstract_element.has_style();
}

}

namespace Web::DOM {

void Document::update_selection_style_observability()
{
    // NB: Editing commands temporarily select content to restore its formatting. Style reads
    //     during the action must not activate selection styles throughout the document for these
    //     intermediate ranges. Observe the final selection after the action, including in input
    //     event handlers. Explicit ::selection queries can still compute their style on demand.
    if (m_running_editing_command_action)
        return;

    auto selection = get_selection();
    auto* text_control = as_if<HTML::FormAssociatedTextControlElement>(focused_area().ptr());
    bool observable = selection && !selection->is_collapsed();
    bool text_control_selection_is_observable = text_control && text_control->selection_start() != text_control->selection_end();
    observable |= text_control_selection_is_observable;
    if (observable == m_selection_styles_are_observable && !m_needs_selection_style_update)
        return;
    m_needs_selection_style_update = false;
    m_selection_styles_are_observable = observable;
    style_computer().style_engine().set_pseudo_element_style_deferred(to_underlying(CSS::PseudoElement::Selection), !observable);
    if (!observable)
        return;

    auto record_element = [&](Node& node) {
        if (auto* element = as_if<Element>(node); element && element->has_style()) {
            style_computer().style_engine().make_deferred_pseudo_element_style_observable(element->style_node_id());
            style_computer().style_engine().record_derived_element_style_input_change(element->style_node_id(),
                CSS::StyleEngine::PseudoInputsMayHaveChanged);
        }
    };
    auto record_subtree = [&](Node& root, Range const* range) {
        // NB: Ancestors supply inherited highlight styles, and text controls paint through
        //     their internal shadow trees. Neither requires visiting unrelated subtrees.
        for (auto* ancestor = root.parent_or_shadow_host(); ancestor; ancestor = ancestor->parent_or_shadow_host())
            record_element(*ancestor);
        root.for_each_shadow_including_inclusive_descendant([&](Node& node) {
            if (range && &node.root() == &range->start_container()->root() && !range->intersects_node(node))
                return TraversalDecision::SkipChildrenAndContinue;
            record_element(node);
            return TraversalDecision::Continue;
        });
    };
    if (selection && !selection->is_collapsed()) {
        auto range = selection->range();
        auto root = range->common_ancestor_container();
        record_subtree(root, range.ptr());
    }
    if (text_control_selection_is_observable)
        record_subtree(*focused_area(), nullptr);
}

// A style update beside this document's frame in flight would rewrite the style the frame reads, so
// it waits for the frame first.
void Document::update_style()
{
    join_frame_in_flight();
    update_selection_style_observability();
    CSS::update_style(*this);
}

bool Document::submit_style_for_rendering_update()
{
    join_frame_in_flight();
    update_selection_style_observability();
    return CSS::submit_style_update(*this);
}

void Document::finish_submitted_style_update()
{
    CSS::finish_submitted_style_update(*this);
}

bool Document::update_style_for_element(AbstractElement const& abstract_element)
{
    join_frame_in_flight();
    update_selection_style_observability();
    flush_throttled_animation_style_update_for_node(abstract_element.element());
    return CSS::update_style_for_element(*this, abstract_element, StyleUpdateMode::Normal);
}

bool Document::update_style_for_element(AbstractElement const& abstract_element, StyleUpdateMode mode)
{
    join_frame_in_flight();
    update_selection_style_observability();
    flush_throttled_animation_style_update_for_node(abstract_element.element());
    return CSS::update_style_for_element(*this, abstract_element, mode);
}

}
