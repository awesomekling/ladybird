/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/Noncopyable.h>
#include <AK/Optional.h>
#include <AK/Span.h>
#include <AK/StringView.h>
#include <AK/Types.h>
#include <AK/Utf16FlyString.h>
#include <AK/Vector.h>
#include <LibGC/Cell.h>
#include <LibGC/Ptr.h>
#include <LibWeb/CSS/StyleDrainScope.h>
#include <LibWeb/CSS/StyleEngineIdentifiers.h>
#include <LibWeb/CSS/StyleRecordID.h>
#include <LibWeb/ComputedValuesRustFFI.h>
#include <LibWeb/Export.h>
#include <LibWeb/StyleEngineRustFFI.h>

namespace Web::CSS::Parser::ValueParserFFI {

struct DeclarationBlockData;

}

namespace Web::CSS::StyleValueFFI {

struct FfiTransitionAction;
struct FfiTransitionInput;

}

namespace Web::CSS {

enum class StyleRecordDependencyFlag : u8;

class StyleComputer;
class RustDeclarationBlock;
struct StyleProperty;

// Owns one document's StyleEngine. The engine itself lives entirely on the Rust side: selector
// evaluation, cascade, computed values, and every index and identity they are keyed by. C++ keeps
// what only C++ can own -- DOM and CSSOM object identity, mutation semantics, document lifecycle,
// and the observation barriers -- and holds no second copy of the style state.
//
// Most input crosses in one flat batch per style flush. Neighbour-relation changes are published
// immediately because the DOM mutation path already has the old relations in hand.
class WEB_API StyleEngine {
    AK_MAKE_NONCOPYABLE(StyleEngine);
    AK_MAKE_NONMOVABLE(StyleEngine);

public:
    using DeviceClass = StyleEngineFFI::FfiDeviceClass;
    explicit StyleEngine(DeviceClass, StyleComputer* = nullptr);
    ~StyleEngine();

    void visit_edges(GC::Cell::Visitor&);

#include <LibWeb/StyleEngineBridgeGenerated.h>

    [[nodiscard]] double ensure_random_base_value(StyleNodeID, Utf16View name, bool element_shared);
    void set_element_custom_property_data(StyleNodeID, CustomPropertyData const*, bool is_animation_overlay = false, bool declares = false);

    // The host names a node the moment it connects, from identities the engine granted it ahead of
    // time, and the mint crosses with the next transaction ahead of everything written to the identity
    // since. Identity 0 is never minted; it means "no node".
    StyleNodeID mint_style_node();
    void mint_style_nodes(Span<StyleNodeID> nodes);
    void mint_text_style_nodes(Span<StyleNodeID> nodes);
    // An identity that stands in the tree only to be named by relations, and is never styled.
    StyleNodeID mint_relation_only_style_node();
    // Makes the grant cover this many mints of each kind, so that minting a subtree asks for identities
    // at most once.
    void ensure_granted_style_nodes(size_t element_count, size_t text_count);
    void defer_element_initial_features(StyleNodeID style_node)
    {
        m_nodes_with_pending_initial_features.set(style_node);
        m_nodes_awaiting_first_style_computation.set(style_node);
    }
    void cancel_deferred_element_initial_features(StyleNodeID style_node)
    {
        m_nodes_with_pending_initial_features.remove(style_node);
        m_nodes_awaiting_first_style_computation.remove(style_node);
    }
    [[nodiscard]] bool has_deferred_element_initial_features(StyleNodeID style_node) const { return m_nodes_with_pending_initial_features.contains(style_node); }
    HashTable<StyleNodeID> take_deferred_element_initial_features();
    HashTable<StyleNodeID> take_elements_awaiting_first_style_computation();

    void set_element_parts(StyleNodeID node, ReadonlySpan<StyleAtomID> names, ReadonlySpan<StyleNodeID> hosts);
    void set_element_language(StyleNodeID node, StyleAtomID language, Utf16View tag);
    // Which longhand properties one of an element's own declarations covers, their canonical
    // specified values and their authored aliases, and whether the inventory has complete
    // continuation semantics.
    void set_element_presentational_hint_properties(StyleNodeID node, StyleEngineFFI::FfiElementDeclarationKind, ReadonlySpan<StyleProperty>);
    struct StyleRecordDelta {
        StyleRecordID old_style_record;
        StyleRecordID new_style_record;
    };
    using StyleRecordView = StyleEngineFFI::FfiStyleRecordView;
    // Publish the immutable input identities of an element or pseudo-element's base style and
    // return its previous and current StyleRecordID assignments. A zero node interns an unassigned
    // record for a style target which is not registered in the engine.
    [[nodiscard]] StyleRecordDelta publish_computed_groups(StyleNodeID node, u8 pseudo_kind, ReadonlySpan<void const*> payloads, size_t inherited_group_count, u64 custom_property_environment, bool inherited_group_swap_candidate, u64 counter_style_environment_identity, u64 animation_overlay_identity, void const* animated_overlay, ReadonlySpan<void const*> animation_overlay_payloads, void const* computed_longhand_table, void const* custom_property_store);
    [[nodiscard]] Optional<StyleRecordDelta> publish_animation_overlay(StyleNodeID node, u8 pseudo_kind, u64 animation_overlay_identity, void const* animated_overlay, ReadonlySpan<void const*> payloads);
    // The borrowed payload array is stable while a base record exists or an animation-overlay
    // generation remains assigned or pinned.
    [[nodiscard]] void const* style_record_payloads(StyleRecordID style_record) const;
    // The payloads of a record an element or box holds, which keeps it alive.
    [[nodiscard]] void const* held_style_record_payloads(StyleRecordID style_record) const;
    [[nodiscard]] StyleRecordDependencyFlag style_record_dependency_flags(StyleRecordID style_record) const;
    [[nodiscard]] u64 style_record_custom_property_environment(StyleRecordID style_record) const;
    [[nodiscard]] bool animation_overlay_changed(StyleRecordID old_style_record, void const* animated_overlay) const;
    [[nodiscard]] StyleEngineFFI::FfiAnimationInvalidation compare_animation_overlay(StyleRecordID old_style_record, void const* animated_overlay, ReadonlySpan<void const*> payloads, bool is_document_element) const;
    // The animation definitions an engine-settled row left for the host, taken so that exactly one
    // application drains them. Borrowed until the next row's are taken.
    struct SettledAnimationDefinitions {
        ReadonlySpan<ComputedValuesFFI::FfiComputedAnimation> definitions;
        bool owed { false };
        bool in_display_none_subtree { false };
    };
    [[nodiscard]] SettledAnimationDefinitions take_settled_animation_definitions(StyleNodeID node, u8 pseudo_kind);
    [[nodiscard]] StyleRecordView style_record_view(StyleRecordID style_record) const;
    void begin_style_record_view_epoch();
    void end_style_record_view_epoch();
    void decide_transitions(StyleRecordID before_style_record, void const* after_longhand_table, void const* after_animated_overlay, StyleValueFFI::FfiTransitionInput&, StyleValueFFI::FfiTransitionAction*) const;
    // Remove the retained input identities for one pseudo-element kind and return its removal.
    [[nodiscard]] StyleRecordDelta remove_computed_pseudo(StyleNodeID node, u8 pseudo_kind);
    void finish_sheet_rules_replacement(SheetID sheet);
    // A fresh identity for an element-sourced declaration block.
    //
    // A block's contents change while the CSSOM object stays the same, so its address is not what
    // makes one version of it different from the next. A version is: an edit that reported the same
    // identity on both sides would cancel in the journal and invalidate nothing.
    [[nodiscard]] u32 next_declaration_block_version() { return StyleEngineFFI::style_engine_next_declaration_block_version(m_impl); }

    // Interns one selector-mentioned name and returns its process-global atom, retained by this
    // document.
    //
    // Utf16FlyString is already interned, so its one-word raw form is the identity: this is a hash
    // lookup on that word plus one reference to keep the name alive. No string is copied, and
    // neither side pays an ASCII or UTF-16 conversion for a fact a u32 comparison answers.
    StyleAtomID intern_atom(Utf16FlyString const&);
    // The engine keeps what a custom property's name spells, once per name, for the environments
    // it computes.
    void note_custom_property_name(StyleAtomID, Utf16FlyString const&);
    // The store of an environment the engine resolved, with one strong reference transferred, and
    // the environment it was resolved over; null for one C++ published.
    [[nodiscard]] void const* borrow_engine_custom_property_environment(u64 identity, u64& parent_identity) const;
    // Whether a demand, or a settlement of an element's pseudo-elements, also takes the debts the
    // node's computation left, for the caller to settle.
    enum class TakeRowDebts : u8 {
        No,
        Yes,
    };
    [[nodiscard]] StyleEngineFFI::FfiRecordDemandAnswer answer_record_demand(StyleNodeID, Optional<u8> pseudo_kind, bool exclude_inline_style, bool targeted, bool read_only = false, TakeRowDebts = TakeRowDebts::No);
    [[nodiscard]] StyleEngineFFI::FfiEngineComputedRecord settle_pseudo_records_after_host_record(StyleNodeID, bool old_is_list_item, TakeRowDebts = TakeRowDebts::No);
    void prepare_root_font_resolution(u64 font_environment_generation);
    void publish_font_faces();

    // Whether an environment identity is one the engine minted for an environment it resolved.
    [[nodiscard]] static bool is_engine_custom_property_environment(u64 identity) { return (identity & (1ull << 62)) != 0; }
    [[nodiscard]] u64 atom_generation() const { return m_atom_generation; }
    // The namespace `[*|x]` names, which is any of them. No interned namespace is zero, so this
    // keys a form of its own in the same table.
    static constexpr StyleAtomID any_namespace { 0 };

    // A name both a selector and the DOM produce as text, with no interned identity on either side:
    // a language subtag and a `:dir()` keyword. Matched ASCII case-insensitively.
    StyleAtomID intern_text_atom(Utf16View);
    StyleAtomID intern_language_atom(Utf16View);
    // The same, without the ASCII folding, for names compared literally such as namespace URIs.
    StyleAtomID intern_case_sensitive_text_atom(Utf16View);

    // Interns the exact identity an attribute fact uses and memoizes its namespace and folded
    // forms. Demand expansion revisits every live attribute, so these forms must not cross the
    // boundary again merely to recover an already published name.
    StyleAtomID intern_attribute_name(Utf16FlyString const& local_name, Optional<Utf16FlyString> const& namespace_uri);

    // Interns an attribute value and records what it spells for selector matching and substituted
    // values. Values repeat heavily, so the text crosses once per distinct value.
    StyleAtomID intern_attribute_value(StyleAtomID name, Utf16String const& value);
    // Demand expansion already has every value identity. Check the name before interning the text
    // so attributes no selector reads do not pay another string hash.
    void backfill_attribute_value_text_if_required(StyleAtomID name, Utf16String const& value);

    // Deltas accumulate here and cross in one flat batch per style flush, never one call per
    // element.
    void record_tree_delta(StyleEngineFFI::FfiTreeDelta const&);
    void record_element_arrival(StyleEngineFFI::FfiElementArrival, ReadonlySpan<StyleAtomID> custom_states);
    void record_local_feature_delta(StyleEngineFFI::FfiLocalFeatureDelta const&);
    void record_state_delta(StyleEngineFFI::FfiStateDelta const&);
    void record_element_declaration_delta(StyleEngineFFI::FfiElementDeclarationDelta const&);
    // Writes to the facts of the mirror the DOM holds and nothing selects or invalidates on: the DOM
    // child sequence, what a text node holds, and what an element is. They cross with the next
    // transaction, in the order they were made.
    //
    // DOM-order links are `(node, parent, previous sibling)` triples of raw identities in tree order,
    // so that each previous sibling is linked first.
    void record_dom_order_links(ReadonlySpan<u32> links);
    void record_dom_order_unlink(StyleNodeID node, StyleNodeID parent);
    void record_text_retirements(ReadonlySpan<StyleNodeID>);
    void record_text_is_ascii_whitespace(StyleNodeID, bool);
    void record_text_is_in_user_agent_shadow_tree(StyleNodeID, bool);
    void record_text_is_password_input(StyleNodeID, bool);
    void record_text_data(StyleNodeID, Utf16String const&);
    void record_adjustment_facts(StyleNodeID, u32 facts);
    void record_associated_pseudo_kind(StyleNodeID, u8 pseudo_kind_plus_one);
    void record_construction_facts(StyleNodeID, u32 facts, u8 box_kind);
    // A snapshot of the element's inline style declarations, or null for none. The write takes the
    // snapshot's reference.
    void record_inline_style_properties(StyleNodeID, Parser::ValueParserFFI::DeclarationBlockData const*);
    enum StyleReaction : u8 {
        PublishedStyle = 1 << 0,
        RecomputeStyle = 1 << 1,
        InheritedStyle = 1 << 2,
        InheritedCustomProperties = 1 << 3,
        RecomputeDescendantStyles = 1 << 4,
        AncestorBecameVisible = 1 << 5,
        PseudoInputsMayHaveChanged = 1 << 6,
        FontInputsChanged = 1 << 7,
    };
    // What a record the engine settled leaves for the host to apply once the batch is installed:
    // the transition step it owes, and whether it also left an animation plan.
    enum SettledRowEffectDebt : u8 {
        SettledRowTransitionDebt = 3,
        SettledRowOwesAnAnimationPlan = 1 << 2,
        SettledRowOwesAnAnimationSample = 1 << 3,
    };
    // What the winners an element's records were computed from read beyond their cascade.
    enum NodeRecordReads : u8 {
        NodeRecordReadsIfFunction = 1 << 0,
        NodeRecordReadsInheritFunction = 1 << 1,
        NodeRecordReadsCustomFunction = 1 << 2,
        NodeRecordReadsAttributes = 1 << 3,
        NodeRecordReadsTreeCounting = 1 << 4,
    };
    // What applying a style reaction found, reported so the engine derives the children's reactions.
    enum StyleReactionAppliedFact : u32 {
        DidChangeCustomProperties = 1 << 0,
        InvalidationIsNone = 1 << 1,
        NeedsLayoutTreeRebuild = 1 << 2,
        RecomputeDescendants = 1 << 3,
        ChildrenExplicitlyInherit = 1 << 4,
        ShadowChildrenExplicitlyInherit = 1 << 5,
        // What the element held as the application began, against what it holds now.
        RowWasUnstyled = 1 << 6,
        RowWasDisplayNone = 1 << 7,
        RowDisplayChanged = 1 << 8,
    };
    void record_container_query_input_change(StyleNodeID);
    // Records every element whose style a size query or container-relative unit decided against the container.
    void record_size_container_query_dependents(StyleNodeID container);
    void record_element_style_input_change(StyleNodeID style_node, u8 reaction = PublishedStyle | RecomputeStyle, u8 inherited_style_groups = 0);
    // A reaction C++ derived from one it applied, for the engine to settle where it can.
    void record_derived_element_style_input_change(StyleNodeID style_node, u8 reaction, u8 inherited_style_groups = 0);
    void record_tree_counting_style_input_change(StyleNodeID style_node);
    void record_flat_tree_descendant_style_input_changes(StyleNodeID style_node, u8 reaction, u8 inherited_style_groups = 0);
    [[nodiscard]] Vector<StyleNodeID> viewport_dependent_style_nodes();
    [[nodiscard]] bool has_recorded_element_style_input_change(StyleNodeID style_node) const;
    void record_benchmark_marker(Utf16View);
    [[nodiscard]] bool has_recorded_input() const;
    [[nodiscard]] bool has_pending_transaction() const;
    [[nodiscard]] bool has_deferred_geometry_transaction() const;
    [[nodiscard]] bool has_deferred_element_style_inputs() const;
    [[nodiscard]] bool has_deferred_element_style_input(StyleNodeID style_node) const;
    [[nodiscard]] bool pending_transaction_may_affect_layout_geometry();
    [[nodiscard]] bool defer_pending_transaction_for_geometry_read();
    [[nodiscard]] bool begin_deferred_geometry_transaction_flush();
    void end_deferred_geometry_transaction_flush();
    // Geometry reads establish the before-change style used by CSS transitions. Keep this
    // monotonic because an inactive rule or a later inline edit can expose the transition only
    // after that boundary.
    void note_css_transitions_may_observe_style_changes() { m_css_transitions_may_observe_style_changes = true; }
    [[nodiscard]] bool css_transitions_may_observe_style_changes() const { return m_css_transitions_may_observe_style_changes; }

    // Submits everything recorded since the last flush as one transaction and normalizes it.
    void flush();

    using PublishedStyleDelta = StyleEngineFFI::FfiStyleDelta;
    struct PublishedTransactionVersion {
        u64 transaction;
        u64 program;
    };
    // The version pair the last non-empty style transaction published. The program version names
    // the match program the engine answered from; the transaction version names the publication.
    // A reader that saw a value at a program version is looking at the same program while it has
    // not moved.
    [[nodiscard]] PublishedTransactionVersion published_transaction_version() const { return m_published_transaction_version; }
    void note_published_transaction_version(PublishedTransactionVersion version) { m_published_transaction_version = version; }
    // The elements connected to the document as the last style transaction was taken, published
    // with it.
    [[nodiscard]] u32 connected_element_count_at_last_transaction() const { return m_connected_element_count_at_last_transaction; }

    struct PublishedStyleTransaction {
        PublishedTransactionVersion version;
        ReadonlySpan<PublishedStyleDelta> reactions;
        bool is_scoped;
        bool only_derived_child_reactions;
        // Returned to the caller so diagnostic transactions do not charge style-update clocks.
        u64 submission_microseconds;
        u64 bridge_microseconds;
    };

    // Takes pending inputs. The diagnostic transaction reports reaction nodes and then discards
    // its matching outputs. The style transaction publishes versioned match-answer records. False
    // means the result is broad enough to prefer complete matching scratch.
    // NB: The returned reactions borrow Rust storage until the next mutable engine call or an
    //     explicit discard. Consume them synchronously before asking the engine anything else.
    bool take_diagnostic_style_transaction(StyleNodeID root, Function<void(ReadonlySpan<StyleNodeID>)>&&);
    PublishedStyleTransaction take_style_transaction(StyleNodeID root);
    void discard_style_transaction_outputs(StyleDrainScope const&);

    using RuleMatch = StyleEngineFFI::FfiRuleMatch;

    enum class MatchPurpose {
        Exact,
        Cascade,
    };

    // Every rule that decides for one element, in the order the cascade applies them. Cascade
    // callers may omit rules whose declarations cannot win; exact callers receive the same answer
    // as the document pass. Returns false when matching could not complete.
    bool match_element(StyleNodeID node, Vector<RuleMatch>&, MatchPurpose);
    void* compile_selector_query(ReadonlySpan<void const*> selectors);
    // For an engine with no StyleComputer to reach its elements through: when the new query demands attribute value
    // text that earlier facts were published without, the callback republishes every attribute value the engine holds.
    void* compile_selector_query(ReadonlySpan<void const*> selectors, Function<void()> const& backfill_attribute_value_texts);
    static void destroy_selector_query(void*);
    void prepare_selector_query();
    Optional<bool> selector_query_matches(void const* query, StyleNodeID node, StyleNodeID scope_root, StyleNodeID shadow_root);
    Optional<bool> selector_query_matches_without_document_root(void const* query, StyleNodeID node, StyleNodeID scope_root, StyleNodeID shadow_root);
    bool selector_query_all(void* query, StyleNodeID root, bool include_root, StyleNodeID scope_root, StyleNodeID shadow_root, bool has_document_root, Vector<StyleNodeID>& matches);
    bool selector_query_first(void* query, StyleNodeID root, bool include_root, StyleNodeID scope_root, StyleNodeID shadow_root, bool has_document_root, StyleNodeID& matched);

    // Enumerates the engine's counters. Returns false once index is past the last counter.
    bool counter(size_t index, StringView& out_name, u64& out_value) const;

    [[nodiscard]] void* rust_handle() { return m_impl; }
    [[nodiscard]] void const* rust_handle() const { return m_impl; }

private:
    using InputTransaction = StyleEngineFFI::FfiStyleInputTransaction;

    bool read_matches(StyleNodeID, Vector<RuleMatch>&, Optional<MatchPurpose>);
    void apply_transaction(InputTransaction const&);
    void submit_recorded_input();
    void record_host_fact_write(StyleEngineFFI::FfiHostFactWrite);
    void mint_style_nodes(Span<StyleNodeID>, Vector<StyleNodeID>& granted, size_t& grant_request, StyleEngineFFI::FfiHostFactKind, u8 value);
    bool refresh_attribute_value_text_requirements();
    [[nodiscard]] bool attribute_name_requires_value_text(StyleAtomID);
    void publish_attribute_value_text(StyleAtomID, Utf16View, bool affects_selector_catalog);

    void* m_impl { nullptr };
    GC::Ptr<StyleComputer> m_style_computer;
    // The recording stream the engine records under, or zero.
    u64 m_recording_stream { 0 };

    // No record is reclaimed within a style-record view epoch, so what a base record's identity
    // names does not change during one: the engine answers each of these once per record.
    struct EpochStyleRecordFacts {
        Optional<StyleRecordView> view;
        Optional<u64> custom_property_environment;
        Optional<u8> dependency_flags;
    };
    [[nodiscard]] EpochStyleRecordFacts* epoch_style_record_facts(StyleRecordID) const;
    u32 m_style_record_view_epoch_depth { 0 };
    mutable HashMap<u64, EpochStyleRecordFacts> m_epoch_style_record_facts;

    HashMap<FlatPtr, StyleAtomID> m_atoms;
    HashTable<StyleAtomID> m_published_language_atoms;
    HashTable<StyleAtomID> m_published_custom_property_names;
    HashMap<StyleAtomID, HashMap<StyleAtomID, StyleAtomID>> m_attribute_name_atoms;
    HashMap<StyleAtomID, bool> m_attribute_names_requiring_value_text;
    u64 m_atom_generation { 1 };
    PublishedTransactionVersion m_published_transaction_version { 0, 0 };
    u32 m_connected_element_count_at_last_transaction { 0 };
    u64 m_attribute_value_text_requirements_version { 0 };
    HashTable<StyleNodeID> m_nodes_with_pending_initial_features;
    HashTable<StyleNodeID> m_nodes_awaiting_first_style_computation;
    size_t m_element_match_capacity { 64 };

    Vector<StyleEngineFFI::FfiTreeDelta> m_tree_deltas;
    Vector<StyleEngineFFI::FfiElementArrival> m_element_arrivals;
    Vector<u32> m_arrival_custom_state_atoms;
    Vector<StyleEngineFFI::FfiLocalFeatureDelta> m_local_feature_deltas;
    Vector<StyleEngineFFI::FfiStateDelta> m_state_deltas;
    Vector<StyleEngineFFI::FfiElementDeclarationDelta> m_element_declaration_deltas;
    Vector<StyleEngineFFI::FfiHostFactWrite> m_host_fact_writes;
    // How many of the host fact writes are atom adoptions, which are no input to style.
    size_t m_pending_atom_adoption_count { 0 };
    // What each `TextData` write holds, by the index its `data` names until the writes cross.
    Vector<Utf16String> m_host_fact_text_data;
    // The identities the engine granted and the host has yet to mint, and how many more the host asks
    // for with the next transaction.
    Vector<StyleNodeID> m_granted_style_nodes;
    Vector<StyleNodeID> m_granted_text_style_nodes;
    size_t m_style_node_grant_request { 0 };
    size_t m_text_style_node_grant_request { 0 };
    bool m_css_transitions_may_observe_style_changes { false };
};

}
