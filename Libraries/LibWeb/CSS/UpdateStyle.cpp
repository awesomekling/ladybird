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
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/CustomPropertyData.h>
#include <LibWeb/CSS/Invalidation/SlotInvalidator.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEngineInput.h>
#include <LibWeb/CSS/StyleInputRecord.h>
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
    bool prefers_broad_matching_batch { false };
    // The transaction continues the style change whose reactions were applied last, one tree
    // generation further, rather than answering new inputs.
    bool only_derived_child_reactions { false };
};

static StyleEngineTransaction take_style_engine_transaction(DOM::Document& document)
{
    StyleEngineTransaction transaction;
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
        return transaction;
    }

    auto published_transaction = style_computer.style_engine().take_style_transaction(root->style_node_id());
    document.style_invalidation_counters().style_update_submission_microseconds += published_transaction.submission_microseconds;
    document.style_invalidation_counters().style_update_bridge_microseconds += published_transaction.bridge_microseconds;
    if (!published_transaction.reactions.is_empty())
        style_computer.style_engine().note_published_transaction_version(published_transaction.version);
    for (auto const& answer : published_transaction.reactions) {
        // The complete answer remains in Rust transaction scratch under this node. The identity
        // names both the semantic reaction and the payload that consumes it.
        VERIFY(style_computer.element_for_style_node(answer.style_node));
        transaction.reactions.append(answer);
    }

    // A reaction batch covering more than one sixteenth of the connected elements is dense enough that
    // packing the scope once is cheaper than repeatedly reconstructing cold facts while matching
    // the planned elements.
    transaction.prefers_broad_matching_batch = !published_transaction.is_scoped
        || transaction.reactions.size() * 16 > style_computer.style_engine().connected_element_count();
    transaction.only_derived_child_reactions = published_transaction.only_derived_child_reactions;

    return transaction;
}

static StyleEngine::PublishedStyleDelta make_materialize_gap_delta(StyleNodeID style_node, u8 reaction, u8 inherited_style_groups = 0)
{
    return {
        .style_node = style_node.value(),
        .match_answer = 0,
        .old_style_record = 0,
        .new_style_record = 0,
        .damage = StyleEngineFFI::FfiStyleDeltaDamage::None,
        .reaction = reaction,
        .inherited_style_groups = inherited_style_groups,
        .pseudo_kind = NumericLimits<u8>::max(),
        .gap = StyleEngineFFI::FfiStyleDeltaGap::Materialize,
        .uses_substitution = false,
    };
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

static RefPtr<CustomPropertyData const> custom_property_environment_base(DOM::Element& element, RefPtr<CustomPropertyData const> data)
{
    if (data && data->is_animation_overlay_for({ element }))
        return data->parent();
    return data;
}

// Every custom property whose value differs between the environment an element held and the one
// it holds now. The names come from what either environment holds itself, then from the
// environments above them; two chains that share an ancestor pair share that pair's answer, which
// is worked out once per style update. Most moves under a root that redefines hundreds of names
// meet the same root pair.
class ChangedCustomPropertyNames {
public:
    Vector<Utf16FlyString> const& between(CustomPropertyData const* old_data, CustomPropertyData const* new_data)
    {
        if (old_data == new_data)
            return m_none;
        auto key = Pair { old_data ? old_data->identity() : 0, new_data ? new_data->identity() : 0 };
        if (auto cached = m_memo.get(key); cached.has_value())
            return *cached;
        Vector<Utf16FlyString> names;
        HashTable<Utf16FlyString> seen;
        auto consider = [&](Utf16FlyString const& name) {
            if (seen.set(name) != AK::HashSetResult::InsertedNewEntry)
                return;
            if (custom_property_value_moved(name, old_data, new_data))
                names.append(name);
        };
        if (old_data) {
            for (auto const& [name, property] : old_data->own_values())
                consider(name);
        }
        if (new_data) {
            for (auto const& [name, property] : new_data->own_values())
                consider(name);
        }
        for (auto const& name : between(old_data ? old_data->parent().ptr() : nullptr, new_data ? new_data->parent().ptr() : nullptr))
            consider(name);
        quick_sort(names);
        auto& stored = m_memo.ensure(key);
        stored = move(names);
        return stored;
    }

private:
    struct Pair {
        u64 old_data;
        u64 new_data;
        bool operator==(Pair const&) const = default;
    };
    struct PairTraits : public DefaultTraits<Pair> {
        static unsigned hash(Pair const& pair) { return pair_int_hash(u64_hash(pair.old_data), u64_hash(pair.new_data)); }
    };

    HashMap<Pair, Vector<Utf16FlyString>, PairTraits> m_memo;
    Vector<Utf16FlyString> m_none;
};

static bool sorted_names_intersect(ReadonlySpan<Utf16FlyString> a, ReadonlySpan<Utf16FlyString> b)
{
    size_t i = 0;
    size_t j = 0;
    while (i < a.size() && j < b.size()) {
        if (a[i] == b[j])
            return true;
        if (a[i] < b[j])
            ++i;
        else
            ++j;
    }
    return false;
}

// An element's custom properties moved. Every styled descendant holds the environment it inherits
// by identity, so each takes the moved one here, directly, and only the descendants whose cascades
// read a name that changed value are asked to compute again. The engine is told nothing: the walk
// is the propagation, in the flat tree the engine would have derived reactions over.
class CustomPropertyEnvironmentMove {
public:
    CustomPropertyEnvironmentMove(DOM::Document& document, ChangedCustomPropertyNames& changed_custom_property_names, CustomPropertyData const* old_origin_base, CustomPropertyData const* new_origin_base)
        : m_document(document)
        , m_style_engine(document.style_computer().style_engine())
        , m_changed_custom_property_names(changed_custom_property_names)
        , m_old_origin_base(old_origin_base)
        , m_new_origin_base(new_origin_base)
    {
    }

    void visit_children(DOM::Element& parent, RefPtr<CustomPropertyData const> const& old_parent_data)
    {
        for (auto* child = parent.first_child(); child; child = child->next_sibling()) {
            auto* element = as_if<DOM::Element>(*child);
            if (!element || element->assigned_slot())
                continue;
            visit(*element, parent, old_parent_data);
        }
        if (auto shadow_root = parent.shadow_root()) {
            for (auto* child = shadow_root->first_child(); child; child = child->next_sibling()) {
                if (auto* element = as_if<DOM::Element>(*child))
                    visit(*element, parent, old_parent_data);
            }
        }
        if (auto* slot = as_if<HTML::HTMLSlotElement>(parent)) {
            for (auto const& node : slot->assigned_nodes()) {
                if (auto* element = as_if<DOM::Element>(*node))
                    visit(*element, parent, old_parent_data);
            }
        }
    }

private:
    // Whether the element's cascades read a name that changed, through var(); an element whose
    // computation reads past what any list of names can say is asked to compute again outright.
    bool var_reads_a_changed_name(DOM::Element& element) const
    {
        if (auto const* record = element.style_input_record()) {
            if (!record->custom_property_reads_are_complete)
                return true;
            return sorted_names_intersect(record->custom_property_reads, changed_names());
        }
        // An element without a style input record has a record the engine computed: the engine
        // knows whether that reads custom properties at all, and settles the reaction it gets.
        return m_style_engine.node_style_reads_custom_properties(element.style_node_id());
    }

    // An element that has to compute again is recorded with a recompute reaction alone: its
    // descendants are this walk's, or that computation's, to reach. (The engine fans an inherited
    // custom-properties reaction out to every child of an applied reaction.)
    // The names are worked out only once a descendant has to be asked about them. A move that reaches
    // no styled descendant, such as an element arriving with its subtree, needs none of them.
    Vector<Utf16FlyString> const& changed_names() const
    {
        if (!m_changed_names)
            m_changed_names = &m_changed_custom_property_names.between(m_old_origin_base, m_new_origin_base);
        return *m_changed_names;
    }

    bool needs_recompute(DOM::Element& element) const
    {
        return element.style_uses_if_css_function() || element.style_uses_inherit_css_function() || element.style_uses_custom_function()
            || element.style_depends_on_style_container_query() || var_reads_a_changed_name(element);
    }

    void mark(DOM::Element& element)
    {
        m_style_engine.record_derived_element_style_input_change(element.style_node_id(), StyleEngine::RecomputeStyle);
    }

    void visit(DOM::Element& element, DOM::Element& parent, RefPtr<CustomPropertyData const> const& old_parent_data)
    {
        // An unstyled subtree materializes against whatever it inherits then.
        if (!element.has_style())
            return;
        auto new_parent_inheritable = [&]() -> RefPtr<CustomPropertyData const> {
            auto data = custom_property_environment_base(parent, parent.custom_property_data({}));
            return data ? data->inheritable(m_document) : nullptr;
        }();
        auto old_parent_inheritable = [&]() -> RefPtr<CustomPropertyData const> {
            auto data = custom_property_environment_base(parent, old_parent_data);
            return data ? data->inheritable(m_document) : nullptr;
        }();
        auto existing = element.custom_property_data({});
        bool const has_animation_overlay = existing && existing->is_animation_overlay_for({ element });
        auto existing_base = custom_property_environment_base(element, existing);
        auto move_pseudo_element_environments = [&](RefPtr<CustomPropertyData const> const& moved) {
            auto existing_inheritable = existing_base ? existing_base->inheritable(m_document) : nullptr;
            auto moved_inheritable = moved ? moved->inheritable(m_document) : nullptr;
            for (auto kind = 0; kind < to_underlying(PseudoElement::KnownPseudoElementCount); ++kind) {
                auto pseudo_element = static_cast<PseudoElement>(kind);
                auto pseudo_data = element.custom_property_data(pseudo_element);
                if (!pseudo_data)
                    continue;
                if (pseudo_data.ptr() == existing_base.ptr())
                    element.set_custom_property_data(pseudo_element, moved);
                else if (pseudo_data.ptr() == existing_inheritable.ptr())
                    element.set_custom_property_data(pseudo_element, moved_inheritable);
                else
                    mark(element);
            }
        };
        // An element that has to compute again takes the moved environment in that computation,
        // which reaches its own descendants in turn.
        //
        // An element declaring no custom property of its own holds the environment it inherits,
        // whichever data it holds it in; its cascade declaring some now is a computation's to find.
        bool const holds_inherited_environment = existing_base.ptr() == old_parent_inheritable.ptr()
            || ((!existing_base || existing_base->declared_count() == 0) && !m_style_engine.node_declares_custom_properties(element.style_node_id()));
        if (holds_inherited_environment && !has_animation_overlay) {
            if (needs_recompute(element)) {
                mark(element);
                return;
            }
            if (new_parent_inheritable.ptr() == existing_base.ptr())
                return;
            move_pseudo_element_environments(new_parent_inheritable);
            element.set_custom_property_data({}, new_parent_inheritable);
            element.republish_style_record_environment();
            visit_children(element, existing_base);
            return;
        }
        // The element declares custom properties of its own over the environment it inherits. Its
        // declared values stand while it reads nothing that changed; a computation decides the rest.
        if (!existing_base || existing_base->declared_count() == 0 || has_animation_overlay || needs_recompute(element)) {
            mark(element);
            return;
        }
        if (existing_base->parent().ptr() == new_parent_inheritable.ptr())
            return;
        OrderedHashMap<Utf16FlyString, StyleProperty> own_values;
        size_t declared = 0;
        for (auto const& [name, property] : existing_base->own_values()) {
            if (declared++ >= existing_base->declared_count())
                break;
            own_values.set(name, property);
        }
        RefPtr<CustomPropertyData const> moved = CustomPropertyData::create(move(own_values), new_parent_inheritable);
        move_pseudo_element_environments(moved);
        element.set_custom_property_data({}, moved);
        element.republish_style_record_environment();
        visit_children(element, existing_base);
    }

    GC::Ref<DOM::Document> m_document;
    StyleEngine& m_style_engine;
    ChangedCustomPropertyNames& m_changed_custom_property_names;
    CustomPropertyData const* m_old_origin_base { nullptr };
    CustomPropertyData const* m_new_origin_base { nullptr };
    mutable Vector<Utf16FlyString> const* m_changed_names { nullptr };
};

static void propagate_custom_property_environment_move(DOM::Document& document, DOM::Element& origin, RefPtr<CustomPropertyData const> old_origin_data, ChangedCustomPropertyNames& changed_custom_property_names)
{
    // Nothing inherits from an element with nothing below it in the flat tree.
    if (!origin.first_element_child() && !origin.shadow_root() && !is<HTML::HTMLSlotElement>(origin))
        return;
    auto old_origin_base = custom_property_environment_base(origin, move(old_origin_data));
    auto new_origin_base = custom_property_environment_base(origin, origin.custom_property_data({}));
    CustomPropertyEnvironmentMove walk { document, changed_custom_property_names, old_origin_base.ptr(), new_origin_base.ptr() };
    walk.visit_children(origin, old_origin_base);
}

static RequiredInvalidationAfterStyleChange apply_style_engine_reactions(DOM::Document& document, Vector<StyleEngine::PublishedStyleDelta> const& reactions)
{
    // Reactions are applied in preorder, so every element's inheritance inputs are ready when it is
    // applied. What an applied element's change means for its (flat-tree) children is the
    // engine's to derive: it reads each application and plans the children as the next
    // transaction of this style update.
    RequiredInvalidationAfterStyleChange transaction_invalidation;
    ChangedCustomPropertyNames changed_custom_property_names;
    begin_noting_declaration_changes_during_apply();
    ScopeGuard end_noting_declaration_changes = [] { end_noting_declaration_changes_during_apply(); };
    // Unstyled descendants of display:none need no record until a targeted read or visibility
    // change asks for one. SVG resources and existing animations can still consume style while
    // hidden, so retain their inheritance prerequisites in this batch.
    HashTable<StyleNodeID> required_in_hidden_subtrees;
    // The effects the batch's rows leave for the host. A row the engine settled can carry work
    // the C++ computation would have done beside the record it computed; the host applies it once
    // the whole batch is installed, in the order the batch applied the rows, which is flat-tree
    // order. Nothing a later row in the batch computes may depend on one of these being applied.
    // A row whose record read a non-inherited property straight from the parent, through an
    // explicit `inherit`, owes the parent the mark C++ writes beside such a computation. The
    // union is monotone and a parent applies before its children, so draining it after the batch
    // marks the parent no later than the C++ path does.
    struct ExplicitInheritanceEffectRow {
        StyleNodeID style_node;
        u32 style_groups;
    };
    Vector<ExplicitInheritanceEffectRow> explicit_inheritance_effect_rows;
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
            auto element = document.style_computer().element_for_style_node(published_reaction.style_node);
            if (!element)
                continue;
            // A reaction the engine derived for this element while applying an earlier one in
            // this batch joins the element's own reaction where it covers it.
            auto reaction = published_reaction;
            if (auto absorbed = document.style_computer().style_engine().absorb_element_style_input(
                    StyleNodeID { reaction.style_node }, reaction.reaction, reaction.inherited_style_groups,
                    reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Materialize);
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

            // The pseudo-element records a retry settled beside the element's record.
            Optional<DOM::Element::EnginePseudoElementRecords> retried_pseudo_element_records;
            bool retried_unstyled_materialization = false;
            bool retried_after_installed_ancestors = false;
            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::RetryAfterAncestor
                || (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Materialize
                    && reaction.reaction & (StyleEngine::RecomputeStyle | StyleEngine::RecomputeDescendantStyles | StyleEngine::AncestorBecameVisible | StyleEngine::InheritedStyle | StyleEngine::InheritedCustomProperties)
                    && !element->has_associated_animations())) {
                // The preceding row has installed and published this element's parent. Ask now,
                // before applying this row, rather than deriving its descendants ahead of their
                // own install boundaries.
                auto retried_rows = document.style_computer().style_engine().retry_engine_records_after_ancestor(reaction.style_node);
                if (!retried_rows.is_empty() && retried_rows[0].style_node == reaction.style_node && retried_rows[0].record.style_record != 0) {
                    retried_unstyled_materialization = reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Materialize && !element->has_style();
                    retried_after_installed_ancestors = reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::RetryAfterAncestor;
                    auto const& retried = retried_rows[0].record;
                    reaction.new_style_record = retried.style_record;
                    reaction.uses_substitution = retried.uses_substitution;
                    reaction.damage = StyleEngineFFI::FfiStyleDeltaDamage::Full;
                    reaction.gap = StyleEngineFFI::FfiStyleDeltaGap::Computed;
                    DOM::Element::EnginePseudoElementRecords pseudo_element_records {};
                    for (size_t kind = 0; kind < array_size(retried.pseudo_records); ++kind) {
                        if ((retried.pseudo_records_present >> kind) & 1)
                            pseudo_element_records[kind] = StyleRecordID { retried.pseudo_records[kind] };
                    }
                    retried_pseudo_element_records = pseudo_element_records;
                } else {
                    // A size query can only be settled after layout publishes its first box.
                    // Keep this element's previous record through
                    // that layout pass; recording the pending effect schedules a new reaction.
                    auto container_effects = StyleEngineFFI::style_engine_take_container_effects(document.style_computer().style_engine().rust_handle(), reaction.style_node);
                    ScopeGuard release_container_effects = [&] { StyleEngineFFI::style_engine_native_container_effects_release(container_effects.effects); };
                    bool awaits_layout_basis = false;
                    auto effect_count = StyleEngineFFI::style_engine_native_container_effect_count(container_effects.effects);
                    for (size_t effect_index = 0; effect_index < effect_count; ++effect_index) {
                        auto effect = StyleEngineFFI::style_engine_native_container_effect(container_effects.effects, effect_index);
                        if (effect.kind == StyleEngineFFI::FfiContainerEffectKind::NeedsEvaluationAfterLayout) {
                            awaits_layout_basis = true;
                            break;
                        }
                    }
                    if (document.is_running_update_layout() && container_effects.depends_on_size && reaction.old_style_record != 0
                        && element->style_record_identity().value() == reaction.old_style_record && awaits_layout_basis) {
                        StyleComputer::record_container_query_effects(DOM::AbstractElement { *element }, container_effects);
                        continue;
                    }
                    if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::RetryAfterAncestor)
                        reaction.gap = StyleEngineFFI::FfiStyleDeltaGap::Materialize;
                }
            }

            // An engine-computed first record installs on an element without style, as does one
            // for an element whose ancestor became visible: its style was cleared on entry to
            // display:none while the engine kept the record. A materialization retried above is
            // also an install even when the engine held an old record the DOM never received.
            // Hidden SVG resource styles are cleared and then restored from derived records.
            // Other record deltas assume the style they move.
            if (!element->has_style()
                && reaction.gap != StyleEngineFFI::FfiStyleDeltaGap::Materialize
                && !(reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Computed
                    && (reaction.old_style_record == 0 || (reaction.reaction & StyleEngine::AncestorBecameVisible)
                        || retried_unstyled_materialization || element->is_svg_element())))
                continue;
            // An earlier display:none reaction in this batch can clear the style of a materialization gap after the
            // inheritance closure was built. The gap must then rematerialize rather than letting its descendants
            // compute against a missing inheritance parent.
            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Materialize && reaction.reaction == 0 && !element->has_style())
                reaction.reaction = StyleEngine::RecomputeStyle;

            bool const has_published_style_reaction = reaction.reaction & StyleEngine::PublishedStyle;
            if (has_published_style_reaction) {
                ++document.style_invalidation_counters().style_engine_published_reactions;
            }
            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Materialize) {
                if (has_published_style_reaction)
                    ++document.style_invalidation_counters().style_engine_materialized_gaps;
            } else {
                ++document.style_invalidation_counters().style_engine_record_deltas_applied;
            }

            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Materialize) {
                VERIFY(reaction.new_style_record == 0);
                VERIFY(reaction.damage == StyleEngineFFI::FfiStyleDeltaDamage::None);
            } else {
                // An engine-computed record is the engine's current answer for the element, which
                // may have skipped a delta C++ never installed; it is applied against whatever the
                // element holds now.
                VERIFY(reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Computed || reaction.old_style_record == element->style_record_identity().value());
                VERIFY(reaction.new_style_record != 0);
                VERIFY(reaction.damage == StyleEngineFFI::FfiStyleDeltaDamage::Full);
            }

            auto previous_style_record = element->style_record_identity();
            auto old_custom_property_data = element->custom_property_data({});
            bool const was_unstyled = !previous_style_record;
            auto const* previous_box_values = element->style_group<ComputedValues::BoxValues>();
            auto const previous_display = previous_box_values
                ? Optional<Display> { display_from_ffi_display(previous_box_values->display) }
                : Optional<Display> {};
            bool const was_display_none = previous_display.has_value() && previous_display->is_none();
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

            // An element declaring custom properties of its own layers them over the environment it
            // inherits, which its cascade decides.
            bool const cascade_declares_custom_properties = document.style_computer().style_engine().node_declares_custom_properties(reaction.style_node);
            bool const needs_full_custom_property_recompute = needs_custom_property_recompute && (element->style_uses_var_css_function() || element->style_uses_inherit_css_function() || cascade_declares_custom_properties);
            // The engine settled the element's record, and the pseudo-element records beside it:
            // C++ installs them.
            auto engine_record_comparison = DOM::Element::EngineRecordComparison::AtInstallation;
            auto apply_engine_computed_records = [&](DOM::Element::EnginePseudoElementRecords const& pseudo_element_records, bool acknowledge) {
                auto& style_engine = document.style_computer().style_engine();
                document.style_computer().pin_transition_stabilization_baseline_if_a_later_pass_may_need_it(DOM::AbstractElement { *element });
                // A first record answers the element's recorded arrival; nothing is left for a
                // later transaction to plan. Neither is anything for a record retried after the
                // ancestors applied before it installed: it reads them as they now stand, as the
                // computation it stands for would have.
                if (!element->has_style() || retried_after_installed_ancestors)
                    style_engine.consume_recorded_element_style_input_change(reaction.style_node);
                invalidation = element->apply_engine_computed_style_record(StyleRecordID { reaction.new_style_record }, pseudo_element_records, reaction.uses_substitution, did_change_custom_properties, engine_record_comparison);
                // What the row's container conditions read of its containers, recorded as the host
                // records it for a row it computes.
                auto container_effects = StyleEngineFFI::style_engine_take_container_effects(style_engine.rust_handle(), reaction.style_node);
                ScopeGuard release_container_effects = [&] { StyleEngineFFI::style_engine_native_container_effects_release(container_effects.effects); };
                StyleComputer::record_container_query_effects(DOM::AbstractElement { *element }, container_effects);
                if (acknowledge)
                    style_engine.acknowledge_engine_computed_record(StyleNodeID { reaction.style_node });
            };
            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::None) {
                VERIFY(!needs_regular_style_recompute);
                VERIFY(needs_inherited_style_recompute);
                VERIFY(!needs_custom_property_recompute);
                VERIFY(reaction.pseudo_kind == NumericLimits<u8>::max());
                // The engine swapped the element's inherited groups for its parent's: the record
                // installs as an engine record. The engine refuses the swap to an element that
                // animates, declares transitions, or inherits from an animating parent.
                apply_engine_computed_records({}, false);
            } else if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Computed) {
                // The engine computed the new record from this element's moved cascade winners,
                // from its parent's moved inherited style or display, or from its moved
                // inherited custom-property environment.
                VERIFY(needs_regular_style_recompute || needs_inherited_style_recompute || needs_custom_property_recompute);
                VERIFY(reaction.pseudo_kind == NumericLimits<u8>::max());
                // Installing an earlier row can update this element's declaration block, or
                // re-sample its parent's animated custom properties into a new environment. The
                // batch record names the old block or environment, so answer a new demand from
                // the current ones before deciding whether this row needs the host computation.
                bool refreshed_declarations = false;
                if ((declarations_changed_during_apply(StyleNodeID { reaction.style_node })
                        || !engine_computed_record_environment_is_installable(*element, StyleRecordID { reaction.new_style_record }))
                    && !element->has_associated_animations()) {
                    auto demand = document.style_computer().style_engine().answer_record_demand(
                        StyleNodeID { reaction.style_node }, {}, false, true);
                    if (demand.record.style_record) {
                        reaction.new_style_record = demand.record.style_record;
                        reaction.uses_substitution = demand.record.uses_substitution;
                        reaction.damage = StyleEngineFFI::FfiStyleDeltaDamage::Full;
                        DOM::Element::EnginePseudoElementRecords pseudo_element_records {};
                        for (size_t kind = 0; kind < array_size(demand.record.pseudo_records); ++kind) {
                            if ((demand.record.pseudo_records_present >> kind) & 1)
                                pseudo_element_records[kind] = StyleRecordID { demand.record.pseudo_records[kind] };
                        }
                        retried_pseudo_element_records = pseudo_element_records;
                        refreshed_declarations = true;
                    }
                }
                auto pseudo_element_records = retried_pseudo_element_records.value_or({});
                for (auto next = reaction_index + 1; next < reactions.size() && reactions[next].style_node == published_reaction.style_node && reactions[next].pseudo_kind != NumericLimits<u8>::max(); ++next)
                    pseudo_element_records[reactions[next].pseudo_kind] = StyleRecordID { reactions[next].new_style_record };
                // The row's own effects come with the decision that settled it, whether or not
                // the record is the one that installs: a C++ computation of this element runs the
                // transition step itself, so the debt is discharged either way.
                auto const explicit_inheritance_debt = document.style_computer().style_engine().take_explicit_inheritance_debt(StyleNodeID { reaction.style_node });
                auto const row_effect_debt = document.style_computer().style_engine().take_settled_row_effect_debt(StyleNodeID { reaction.style_node });
                auto const transition_debt = row_effect_debt & StyleEngine::SettledRowTransitionDebt;
                if (row_effect_debt & StyleEngine::SettledRowOwesAnAnimationPlan)
                    animation_plan = document.style_computer().take_settled_animation_plan(StyleNodeID { reaction.style_node }, NumericLimits<u8>::max());
                bool const has_animations_or_plan = animation_plan.has_value() || element->has_relevant_animations()
                    || element->has_associated_animations();
                if (!engine_computed_record_environment_is_installable(*element, StyleRecordID { reaction.new_style_record })
                    || (declarations_changed_during_apply(StyleNodeID { reaction.style_node }) && !refreshed_declarations)) {
                    // The record names declarations or an environment an earlier row of this batch
                    // has since moved, and no fresh answer replaced it. The move schedules the next
                    // transaction, which asks for this element again.
                    StyleEngineFFI::style_engine_native_container_effects_release(StyleEngineFFI::style_engine_take_container_effects(document.style_computer().style_engine().rust_handle(), reaction.style_node).effects);
                    for (size_t kind = 0; kind < pseudo_element_records.size(); ++kind) {
                        if (pseudo_element_records[kind].has_value())
                            (void)document.style_computer().take_settled_animation_plan(StyleNodeID { reaction.style_node }, static_cast<u8>(kind));
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
                    apply_engine_computed_records(pseudo_element_records, false);
                    DOM::AbstractElement settled { *element };
                    if (animation_plan.has_value())
                        document.style_computer().apply_settled_animation_plan(settled, *animation_plan);
                    bool installed_pseudo_animation_plan = false;
                    auto apply_pseudo_animation_plan = [&](size_t kind) {
                        if (kind >= pseudo_element_records.size())
                            return;
                        if (!pseudo_element_records[kind].has_value() || !*pseudo_element_records[kind])
                            return;
                        auto pseudo_plan = document.style_computer().take_settled_animation_plan(StyleNodeID { reaction.style_node }, static_cast<u8>(kind));
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
                        (void)document.style_computer().record_transition_stabilization_baseline(settled, StyleRecordID { reaction.old_style_record });
                    bool const compares_after_sample = engine_record_comparison == DOM::Element::EngineRecordComparison::AfterSample;
                    if (settled.has_style() && (has_animation_effects || animation_plan.has_value() || row_effect_debt & StyleEngine::SettledRowOwesAnAnimationSample))
                        sample_animations_for_installed_record(settled, compares_after_sample ? SampleInvalidation::AppliedByCaller : SampleInvalidation::Applied);
                    if (compares_after_sample)
                        invalidation = element->compare_engine_computed_style_record_after_sample(old_style_record, *old_originating_style, invalidation);
                    // The step runs here rather than after the batch: a descendant applied later
                    // reads this element's after-change style, which is what the step decides
                    // against, and the C++ computation this row replaces runs it inside itself.
                    // A row that owes only the registration leaves nothing for the host: the step
                    // reads the element's transition declarations from the installed record.
                    if (transition_debt == 2
                        || (transition_debt == 0 && reaction.old_style_record != 0 && element->associated_shadow_host_pseudo_element().has_value())) {
                        DOM::AbstractElement settled { *element };
                        if (settled.has_style()) {
                            auto step_invalidation = document.style_computer().run_transition_step_for_installed_record(
                                settled, StyleRecordID { reaction.old_style_record });
                            if (!step_invalidation.is_none()) {
                                apply_element_style_invalidation_after_style_change(*element, step_invalidation);
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
                            element->apply_display_none_change(old_originating_style->base_values().display().is_none() != new_style->base_values().display().is_none(),
                                !old_originating_style->display().is_none() && new_style->display().is_none());
                        }
                        auto settled_pseudos = document.style_computer().style_engine().settle_pseudo_records_after_host_record(StyleNodeID { reaction.style_node }, old_is_list_item);
                        DOM::Element::EnginePseudoElementRecords final_pseudo_records {};
                        for (size_t kind = 0; kind < array_size(settled_pseudos.pseudo_records); ++kind) {
                            if ((settled_pseudos.pseudo_records_present >> kind) & 1)
                                final_pseudo_records[kind] = StyleRecordID { settled_pseudos.pseudo_records[kind] };
                        }
                        g_deferring_engine_pseudo_installation = previous_pseudo_deferral;
                        auto pseudo_invalidation = element->install_engine_pseudo_element_records_after_sample(
                            did_change_custom_properties, old_is_list_item,
                            old_originating_style ? &*old_originating_style : nullptr,
                            settled_pseudos.style_record ? &final_pseudo_records : nullptr);
                        invalidation |= pseudo_invalidation;
                    }
                    if (element->has_associated_animations() || installed_pseudo_animation_plan)
                        sample_animations_for_installed_pseudos(*element);
                    document.style_computer().style_engine().acknowledge_engine_computed_record(StyleNodeID { reaction.style_node });
                    if (explicit_inheritance_debt != 0)
                        explicit_inheritance_effect_rows.append({ StyleNodeID { reaction.style_node }, explicit_inheritance_debt });
                }
            } else if (needs_regular_style_recompute || needs_inherited_style_recompute || needs_full_custom_property_recompute) {
                if (needs_regular_style_recompute)
                    document.style_computer().style_engine().consume_recorded_element_style_input_change(reaction.style_node);
                auto pseudo_element_inputs = (reaction.reaction & StyleEngine::PseudoInputsMayHaveChanged)
                    ? DOM::Element::PseudoElementInputs::Changed
                    : DOM::Element::PseudoElementInputs::Unchanged;
                // A reaction the engine derived from an ancestor's application, rather than from a
                // published match answer, changes nothing the element's style input record does not
                // name: the element may answer with its own last style when the record still holds.
                bool const is_derived_reaction = !has_published_style_reaction && reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::Materialize;
                document.style_computer().set_materializing_for_derived_reaction(is_derived_reaction);
                ScopeGuard reset_derived_reaction = [&] {
                    document.style_computer().set_materializing_for_derived_reaction(false);
                };
                invalidation = element->apply_style_engine_reaction(did_change_custom_properties, pseudo_element_inputs);
                if (pseudo_element_inputs == DOM::Element::PseudoElementInputs::Changed)
                    sample_animations_for_installed_pseudos(*element);
            } else if (needs_custom_property_recompute && element->refresh_inherited_custom_property_data()) {
                did_change_custom_properties = true;
                element->republish_style_record_environment();
                element->invalidate_descendant_styles_depending_on_style_container_query();
            }

            auto const* current_inherited_box_values = element->style_group<ComputedValues::InheritedBoxValues>();
            if (previous_visibility.has_value() && current_inherited_box_values
                && *previous_visibility != static_cast<Visibility>(current_inherited_box_values->visibility)) {
                document.throttled_animation_visibility_changed();
            }

            apply_element_style_invalidation_after_style_change(*element, invalidation);
            transaction_invalidation |= invalidation;

            auto current_style_record = element->style_record_identity();
            auto& style_engine = document.style_computer().style_engine();
            u32 facts = 0;
            // The environment moved: the element's descendants take it here, and the ones that read
            // a moved name are recorded for their own computation. The engine derives no reactions
            // for the move.
            if (did_change_custom_properties)
                propagate_custom_property_environment_move(document, *element, old_custom_property_data, changed_custom_property_names);
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
            if (was_unstyled)
                facts |= StyleEngine::WasUnstyled;
            if (was_display_none)
                facts |= StyleEngine::WasDisplayNone;
            // A descendant whose style was cleared on entry to display:none can receive a reaction which only
            // updates style-engine bookkeeping, such as a synthetic pseudo-element reaction. Keep the DOM style
            // unmaterialized until a CSSOM read or the ancestor becomes visible.
            if (!!current_style_record) {
                facts |= StyleEngine::HasStyle;
                auto const* current_box_values = element->style_group<ComputedValues::BoxValues>();
                VERIFY(current_box_values);
                if (display_from_ffi_display(current_box_values->display).is_none())
                    facts |= StyleEngine::IsDisplayNone;
                if (previous_display.has_value() && *previous_display != display_from_ffi_display(current_box_values->display))
                    facts |= StyleEngine::DisplayChanged;
                if (has_flag(style_engine.style_record_dependency_flags(current_style_record), StyleRecordDependencyFlag::InDisplayNoneSubtree))
                    facts |= StyleEngine::InDisplayNoneSubtree;
            } else {
                VERIFY(was_unstyled);
            }
            style_engine.note_style_reaction_applied(reaction.style_node, reaction.reaction, invalidation.inherited_style_groups_changed(), facts);
        }
    }

    // The batch is installed: drain what its rows left behind, in the order they were applied.
    for (auto const& row : explicit_inheritance_effect_rows) {
        auto element = document.style_computer().element_for_style_node(row.style_node);
        if (!element || !element->is_connected() || &element->document() != &document)
            continue;
        if (auto* parent = element->parent())
            parent->add_children_explicitly_inherited_non_inherited_style_groups(row.style_groups == NumericLimits<u32>::max() ? ComputedValues::all_style_groups : row.style_groups);
    }
    return transaction_invalidation;
}

static void update_style(DOM::Document& document, DocumentWithoutBrowsingContext document_without_browsing_context)
{
    auto style_update_started_at = MonotonicTime::now();
    auto& timing_counters = document.style_invalidation_counters();
    auto const submission_before = timing_counters.style_update_submission_microseconds;
    auto const bridge_before = timing_counters.style_update_bridge_microseconds;
    auto const apply_before = timing_counters.style_update_apply_microseconds;
    ScopeGuard record_style_update_time = [&] {
        auto whole = (MonotonicTime::now() - style_update_started_at).to_truncated_microseconds();
        auto measured = timing_counters.style_update_submission_microseconds - submission_before
            + timing_counters.style_update_bridge_microseconds - bridge_before
            + timing_counters.style_update_apply_microseconds - apply_before;
        timing_counters.style_update_microseconds += whole;
        // NB: Counters are read only to attribute time, never to choose style work. The
        //     intervals are disjoint; rounding each down leaves fractional time here too.
        timing_counters.style_update_remainder_microseconds += whole - measured;
    };
    // NOTE: If our parent document needs a relayout, we must do that *first*. This is required as it may cause the
    // viewport to change which will can affect media query evaluation and the value of the `vw` unit.
    if (auto navigable = document.navigable(); navigable && navigable->container() && &navigable->container()->document() != &document)
        navigable->container()->document().update_layout(DOM::UpdateLayoutReason::ChildDocumentStyleUpdate);

    if (!document.browsing_context() && document_without_browsing_context == DocumentWithoutBrowsingContext::Skip)
        return;

    // NOTE: If this is a document hosting <template> contents, style update is unnecessary.
    if (document.created_for_appropriate_template_contents())
        return;

    auto submission_started_at = MonotonicTime::now();
    document.style_computer().begin_style_update();
    ScopeGuard end_style_update = [&] {
        document.style_computer().end_style_update();
    };

    document.style_computer().begin_style_record_view_epoch();
    ScopeGuard end_style_record_view_epoch = [&] {
        document.style_computer().end_style_record_view_epoch();
    };

    document.synchronize_dirty_style_attributes();

    document.begin_style_stabilization_epoch();
    ScopeGuard end_stabilization_epoch = [&] {
        document.end_style_stabilization_epoch();
    };

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
    ScopeGuard leave_complete_style_update = [&] { finish_complete_style_update(document); };

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
            return;
    }

    // Publish each tree scope's counter-style registry before the engine answers any rows.
    // Shadow scopes can override names, and a warm transaction can already read a registry
    // changed by a stylesheet edit; neither can wait for C++ to resolve a list style on demand.
    (void)document.style_scope().counter_style_environment_identity();
    document.for_each_shadow_root([](DOM::ShadowRoot& shadow_root) {
        (void)shadow_root.style_scope().counter_style_environment_identity();
    });

    // A style flush is a transaction boundary. Everything recorded since the last one crosses into
    // StyleEngine as one flat batch, is normalized there, and is routed into the region its
    // transpose programs reach. A transaction that could not be proven narrower publishes a
    // complete document reaction batch. Only a transaction that cannot complete its answers falls
    // back to document invalidation.
    auto style_engine_transaction = take_style_engine_transaction(document);
    ScopeGuard discard_style_engine_transaction_outputs = [&] {
        document.style_computer().style_engine().discard_style_transaction_outputs();
    };

    if (!style_engine_transaction.reactions.is_empty())
        document.note_style_stabilization_has_style_reactions();
    document.sample_animation_effects_needing_style_update();

    auto style_engine_reactions = move(style_engine_transaction.reactions);
    auto prefers_broad_matching_batch = style_engine_transaction.prefers_broad_matching_batch;
    auto transaction_only_derived_child_reactions = style_engine_transaction.only_derived_child_reactions;
    if (style_engine_reactions.is_empty()
        && document.style_computer().style_engine().has_pending_transaction()) {
        auto feedback_transaction = take_style_engine_transaction(document);
        style_engine_reactions = move(feedback_transaction.reactions);
        prefers_broad_matching_batch = feedback_transaction.prefers_broad_matching_batch;
        transaction_only_derived_child_reactions = feedback_transaction.only_derived_child_reactions;
    }

    document.build_registered_properties_cache_for_style_update();

    // This pass belongs to the current style change event. The outer stabilization epoch advanced
    // the transition generation once, before any style, animation, or layout feedback ran.
    document.record_style_stabilization_pass();

    if (style_engine_reactions.is_empty())
        return;

    bool has_cold_matching_traversal = false;
    if (auto* root = document.document_element(); root && root->style_node_id() != 0) {
        if (prefers_broad_matching_batch) {
            has_cold_matching_traversal = document.style_computer().style_engine().begin_cold_matching_batch(root->style_node_id());
        } else {
            document.style_computer().style_engine().begin_adaptive_cold_matching_batch(root->style_node_id());
            has_cold_matching_traversal = true;
        }
    }
    ScopeGuard end_cold_matching_batch = [&] {
        if (has_cold_matching_traversal)
            document.style_computer().style_engine().end_cold_matching_batch();
    };

    RequiredInvalidationAfterStyleChange invalidation;
    constexpr size_t max_style_update_passes = 8;
    size_t style_update_pass = 0;
    size_t style_reaction_pass = 0;
    while (!style_engine_reactions.is_empty()) {
        auto apply_started_at = MonotonicTime::now();
        ArmedScopeGuard record_apply_time = [&] {
            timing_counters.style_update_apply_microseconds += (MonotonicTime::now() - apply_started_at).to_truncated_microseconds();
        };
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
                break;
            }
        }

        HashTable<StyleNodeID> reaction_set;
        reaction_set.ensure_capacity(style_engine_reactions.size());
        for (auto const& reaction : style_engine_reactions)
            reaction_set.set(StyleNodeID { reaction.style_node });
        Vector<StyleNodeID> inheritance_closure;

        // A reaction can name an element created by editing after its new inheritance parent was
        // inserted. Close the batch over unstyled inheritance prerequisites, which are bounded by
        // the reaction paths rather than discovered by a document traversal.
        for (size_t index = 0; index < style_engine_reactions.size(); ++index) {
            auto element = document.style_computer().element_for_style_node(style_engine_reactions[index].style_node);
            if (!element || !element->is_connected() || &element->document() != &document)
                continue;
            for (auto ancestor = DOM::AbstractElement { *element }.element_to_inherit_style_from(); ancestor.has_value() && !ancestor->has_style(); ancestor = ancestor->element_to_inherit_style_from()) {
                auto prerequisite = ancestor->element().style_node_id();
                VERIFY(prerequisite != 0);
                if (reaction_set.set(prerequisite) == AK::HashSetResult::InsertedNewEntry) {
                    style_engine_reactions.append(make_materialize_gap_delta(prerequisite, StyleEngine::RecomputeStyle));
                    inheritance_closure.append(prerequisite);
                }
            }
        }

        // A published descendant may have an inheritance ancestor in the batch while the nodes
        // between them have no selector reaction of their own. Keep zero-bit scheduling slots for
        // that gap so derived inheritance bits can reach the descendant before its published
        // reaction is consumed.
        auto reaction_count_before_inheritance_closure = style_engine_reactions.size();
        Vector<StyleNodeID, 16> inheritance_gap;
        for (size_t index = 0; index < reaction_count_before_inheritance_closure; ++index) {
            auto element = document.style_computer().element_for_style_node(style_engine_reactions[index].style_node);
            if (!element)
                continue;
            inheritance_gap.clear_with_capacity();
            for (auto ancestor = DOM::AbstractElement { *element }.element_to_inherit_style_from(); ancestor.has_value(); ancestor = ancestor->element_to_inherit_style_from()) {
                auto ancestor_style_node = ancestor->element().style_node_id();
                VERIFY(ancestor_style_node != 0);
                if (reaction_set.contains(ancestor_style_node)) {
                    for (auto style_node : inheritance_gap) {
                        if (reaction_set.set(style_node) == AK::HashSetResult::InsertedNewEntry) {
                            style_engine_reactions.append(make_materialize_gap_delta(style_node, 0));
                            inheritance_closure.append(style_node);
                        }
                    }
                    break;
                }
                inheritance_gap.append(ancestor_style_node);
            }
        }
        if (!inheritance_closure.is_empty())
            VERIFY(document.style_computer().style_engine().complete_published_match_answers_for_closure(inheritance_closure));

        Vector<StyleEngine::PublishedStyleDelta> applicable_style_engine_reactions;
        for (auto const& reaction : style_engine_reactions) {
            auto element = document.style_computer().element_for_style_node(reaction.style_node);
            if (!element)
                continue;
            if (!element->is_connected() || &element->document() != &document)
                continue;
            applicable_style_engine_reactions.append(reaction);
        }
        style_engine_reactions.clear();
        if (!applicable_style_engine_reactions.is_empty()) {
            // Apply each inheritance branch contiguously in preorder. Besides making every parent
            // ready before its descendants, this lets a parent's derived reaction merge into an
            // unconsumed child reaction in the same batch.
            document.style_computer().style_engine().sort_style_deltas_for_direct_application(applicable_style_engine_reactions);
            Vector<StyleNodeID> frozen_input_nodes;
            frozen_input_nodes.ensure_capacity(applicable_style_engine_reactions.size());
            for (auto const& reaction : applicable_style_engine_reactions)
                frozen_input_nodes.unchecked_append(StyleNodeID { reaction.style_node });
            document.style_computer().style_engine().freeze_longhand_inputs(frozen_input_nodes);
            ScopeGuard clear_frozen_longhand_inputs = [&] {
                document.style_computer().style_engine().freeze_longhand_inputs({});
            };
            auto& counters = document.style_invalidation_counters();
            if (published_reaction_count > 0) {
                ++counters.style_engine_reaction_batch_runs;
                counters.style_engine_reaction_elements += published_reaction_count;
            }
            invalidation |= apply_style_engine_reactions(document, applicable_style_engine_reactions);
        }

        timing_counters.style_update_apply_microseconds += (MonotonicTime::now() - apply_started_at).to_truncated_microseconds();
        record_apply_time.disarm();

        // Exact consequences produced while recomputing become the next transaction in this
        // stabilization epoch. Take it only after consuming the current published answers, since
        // a new transaction retires their scratch.
        if (document.style_computer().style_engine().has_pending_transaction()) {
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

// What a targeted materialization of one element found, reported to the engine the way a reaction
// pass reports it, so the element's children get the same derived reactions either way.
static void note_targeted_style_reaction_applied(DOM::Element& element, RequiredInvalidationAfterStyleChange const& invalidation, bool did_change_custom_properties, bool descendant_style_recompute_needed, bool was_unstyled, bool was_display_none, bool display_changed)
{
    auto& style_engine = element.document().style_computer().style_engine();
    u8 reaction = StyleEngine::PublishedStyle | StyleEngine::RecomputeStyle;
    if (descendant_style_recompute_needed)
        reaction |= StyleEngine::RecomputeDescendantStyles;
    u32 facts = 0;
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
    if (was_unstyled)
        facts |= StyleEngine::WasUnstyled;
    if (was_display_none)
        facts |= StyleEngine::WasDisplayNone;
    if (display_changed)
        facts |= StyleEngine::DisplayChanged;
    auto style_record = element.style_record_identity();
    if (!!style_record) {
        facts |= StyleEngine::HasStyle;
        auto const* box_values = element.style_group<ComputedValues::BoxValues>();
        VERIFY(box_values);
        if (display_from_ffi_display(box_values->display).is_none())
            facts |= StyleEngine::IsDisplayNone;
        if (has_flag(style_engine.style_record_dependency_flags(style_record), StyleRecordDependencyFlag::InDisplayNoneSubtree))
            facts |= StyleEngine::InDisplayNoneSubtree;
    }
    style_engine.note_style_reaction_applied(element.style_node_id(), reaction, invalidation.inherited_style_groups_changed(), facts);
}

static void apply_targeted_style_invalidation(DOM::Element& element, RequiredInvalidationAfterStyleChange const& invalidation, bool did_change_custom_properties, bool descendant_style_recompute_needed, bool was_unstyled, bool was_display_none, bool display_changed)
{
    if (!invalidation.is_none() || did_change_custom_properties)
        Invalidation::invalidate_assigned_slottables_after_slot_style_change(element);
    apply_element_style_invalidation_after_style_change(element, invalidation);
    note_targeted_style_reaction_applied(element, invalidation, did_change_custom_properties, descendant_style_recompute_needed, was_unstyled, was_display_none, display_changed);
    apply_document_style_invalidation_after_style_change(element.document(), invalidation);
}

// Install the engine's answer for a targeted demand of one element, or return nothing when the engine declines it.
static Optional<RequiredInvalidationAfterStyleChange> install_targeted_record_demand_answer(DOM::Element& element, bool& did_change_custom_properties)
{
    auto& style_computer = element.document().style_computer();
    auto& engine = style_computer.style_engine();
    auto answer = engine.answer_record_demand(element.style_node_id(), {}, false, true);
    if (!answer.record.style_record)
        return {};

    bool environment_is_installable = false;
    (void)element.custom_property_environment_of_engine_record(StyleRecordID { answer.record.style_record }, environment_is_installable);
    if (!environment_is_installable)
        return {};

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
    auto invalidation = element.apply_engine_computed_style_record(StyleRecordID { answer.record.style_record }, pseudo_element_records, answer.record.uses_substitution, did_change_custom_properties,
        samples_over_the_record ? DOM::Element::EngineRecordComparison::AfterSample : DOM::Element::EngineRecordComparison::AtInstallation);
    if (!!old_style_record && element.associated_shadow_host_pseudo_element().has_value())
        invalidation |= style_computer.run_transition_step_for_installed_record({ element }, old_style_record);
    auto container_effects = StyleEngineFFI::style_engine_take_container_effects(engine.rust_handle(), element.style_node_id().value());
    ScopeGuard release_container_effects = [&] { StyleEngineFFI::style_engine_native_container_effects_release(container_effects.effects); };
    StyleComputer::record_container_query_effects(DOM::AbstractElement { element }, container_effects);
    engine.acknowledge_engine_computed_record(element.style_node_id());
    if (samples_over_the_record) {
        sample_animations_for_installed_record(DOM::AbstractElement { element }, SampleInvalidation::AppliedByCaller);
        invalidation = element.compare_engine_computed_style_record_after_sample(old_style_record, *old_style, invalidation);
    }
    return invalidation;
}

static RequiredInvalidationAfterStyleChange materialize_style_for_targeted_update(DOM::Element& element, bool& did_change_custom_properties)
{
    // A targeted update only reaches connected elements, and every one of them has a parent.
    auto& style_computer = element.document().style_computer();
    bool const was_unstyled = !element.has_style();
    auto invalidation = install_targeted_record_demand_answer(element, did_change_custom_properties);
    if (invalidation.has_value()) {
        // A scoped read of an unstyled hidden animation target installs its
        // base record first. Sample its effects over that record now: the
        // document's ordinary animation tick skips hidden descendants.
        if (was_unstyled && element.has_relevant_animations()) {
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
        auto const* box_values = element.style_group<ComputedValues::BoxValues>();
        if (box_values && box_values->is_scroll_state_container && element.style_depends_on_size_container_query()) {
            DOM::Element::EnginePseudoElementRecords pseudo_records {};
            bool settled_pseudo = false;
            for (auto kind : { PseudoElement::Before, PseudoElement::After, PseudoElement::FirstLetter, PseudoElement::Marker }) {
                auto answer = style_computer.style_engine().answer_record_demand(element.style_node_id(), to_underlying(kind), false, true, true);
                if (answer.decline_cause_length)
                    continue;
                pseudo_records[to_underlying(kind)] = StyleRecordID { answer.record.style_record };
                settled_pseudo = true;
            }
            if (settled_pseudo)
                *invalidation |= element.apply_engine_computed_style_record(element.style_record_identity(), pseudo_records, false, did_change_custom_properties);
            // The container's pseudo rules can change after its descendants finish style and
            // layout and the scroll-state snapshot is published.
            invalidation->recompute_descendant_styles = true;
        }
        return *invalidation;
    }

    style_computer.style_engine().consume_recorded_element_style_input_change(element.style_node_id());
    return element.apply_style_engine_reaction(did_change_custom_properties);
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

    bool descendant_style_recompute_needed = false;
    for (size_t i = *topmost_element_to_recompute + 1; i > 0; --i) {
        auto& element = inheritance_chain[i - 1];
        bool did_change_custom_properties = false;
        bool const was_unstyled = !element->has_style();
        auto const* previous_box_values = element->style_group<ComputedValues::BoxValues>();
        bool const was_display_none = previous_box_values && display_from_ffi_display(previous_box_values->display).is_none();
        auto const previous_display = previous_box_values ? Optional<Display> { display_from_ffi_display(previous_box_values->display) } : Optional<Display> {};
        auto invalidation = materialize_style_for_targeted_update(element, did_change_custom_properties);
        auto const* current_box_values = element->style_group<ComputedValues::BoxValues>();
        bool const display_changed = previous_display.has_value() && current_box_values && *previous_display != display_from_ffi_display(current_box_values->display);
        apply_targeted_style_invalidation(element, invalidation, did_change_custom_properties, descendant_style_recompute_needed, was_unstyled, was_display_none, display_changed);

        descendant_style_recompute_needed |= invalidation.recompute_descendant_styles;

        VERIFY(element->has_style());
        auto const* box_values = element->style_group<ComputedValues::BoxValues>();
        VERIFY(box_values);
        if (display_from_ffi_display(box_values->display).is_none()) {
            if (mode == StyleUpdateMode::StopAtDisplayNone)
                return false;
            descendant_style_recompute_needed = false;
        }

        if (did_change_custom_properties || invalidation.needs_layout_tree_rebuild())
            descendant_style_recompute_needed = true;
    }

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

void Document::update_style()
{
    update_selection_style_observability();
    CSS::update_style(*this);
}

bool Document::update_style_for_element(AbstractElement const& abstract_element)
{
    update_selection_style_observability();
    flush_throttled_animation_style_update_for_node(abstract_element.element());
    return CSS::update_style_for_element(*this, abstract_element, StyleUpdateMode::Normal);
}

bool Document::update_style_for_element(AbstractElement const& abstract_element, StyleUpdateMode mode)
{
    update_selection_style_observability();
    flush_throttled_animation_style_update_for_node(abstract_element.element());
    return CSS::update_style_for_element(*this, abstract_element, mode);
}

}
