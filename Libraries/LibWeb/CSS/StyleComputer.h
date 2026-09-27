/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021-2024, Sam Atkins <sam@ladybird.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/HashMap.h>
#include <AK/JsonArray.h>
#include <AK/Optional.h>
#include <AK/OwnPtr.h>
#include <AK/StringView.h>
#include <AK/Utf16String.h>
#include <AK/Utf16View.h>
#include <AK/WeakPtr.h>
#include <LibWeb/Animations/KeyframeEffect.h>
#include <LibWeb/CSS/CSSAnimationProperties.h>
#include <LibWeb/CSS/CSSFontFaceRule.h>
#include <LibWeb/CSS/CSSKeyframesRule.h>
#include <LibWeb/CSS/CascadeOrigin.h>
#include <LibWeb/CSS/ComputedStyleWorkingSet.h>
#include <LibWeb/CSS/ComputedValues.h>
#include <LibWeb/CSS/CustomPropertyData.h>
#include <LibWeb/CSS/MediaQuery.h>
#include <LibWeb/CSS/RustDeclarationBlock.h>
#include <LibWeb/CSS/Selector.h>
#include <LibWeb/CSS/SelectorMatching.h>
#include <LibWeb/CSS/SharedCompiledStyleSheet.h>
#include <LibWeb/CSS/StyleGroupPayloadPins.h>
#include <LibWeb/CSS/StyleInvalidation.h>
#include <LibWeb/CSS/StyleScope.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>

#include <LibWeb/CSS/StyleEngineBridge.h>

namespace Web::CSS {

class WEB_API StyleComputer final : public GC::Cell {
    GC_CELL(StyleComputer, GC::Cell);
    GC_DECLARE_ALLOCATOR(StyleComputer);

public:
    static void record_container_query_effects(StyleDrainScope const&, DOM::AbstractElement, StyleEngineFFI::FfiNativeContainerMatchResult const&);

    static void for_each_property_expanding_shorthands(PropertyID, StyleValue const&, Function<void(PropertyID, StyleValue const&)> const& set_longhand_property);

    static Optional<Utf16String> user_agent_style_sheet_source(Utf16View name);

    static Vector<StyleProperty> collect_presentational_hint_properties(DOM::AbstractElement);

    explicit StyleComputer(DOM::Document&);
    virtual ~StyleComputer() override = default;

    DOM::Document& document() { return m_document; }
    DOM::Document const& document() const { return m_document; }

    [[nodiscard]] NonnullRefPtr<ComputedValues const> create_document_style() const;

    // The document element's style installed: the metrics `rem` resolves against are its font's.
    void update_root_element_font_metrics(ComputedValues const&);
    [[nodiscard]] JsonArray collect_devtools_applied_style_rules(DOM::AbstractElement, bool include_inherited, bool include_user_agent_styles);

    // The way back from a StyleEngine rule identity to what that rule contributes to the cascade.
    // Matching in the engine answers with identities; the cascade needs a declaration and the place
    // it sits in. Rust owns the rule data and per-document identities; this side resolves only
    // the source's document/loading state.
    // DevTools resolves native identities to CSSOM wrappers only when requested.
    void register_style_engine_sheet_source(StyleSheetState const&);
    [[nodiscard]] Optional<StyleEngineRuleTarget> style_engine_rule_target(StyleEngineRuleID rule_id) const;

    static CSSPixels default_user_font_size();
    static void ensure_style_metadata_tables_installed();
    static CSSPixels absolute_size_mapping(AbsoluteSize, CSSPixels default_font_size);

    void set_viewport_rect(Badge<DOM::Document>, CSSPixelRect const& viewport_rect) { m_viewport_rect = viewport_rect; }
    [[nodiscard]] CSSPixelRect const& viewport_rect_for_style_environment() const { return m_viewport_rect; }
    // Moves with the viewport rect the environment resolves viewport units against. The key a shared
    // style record is looked up under names it.
    void bump_viewport_environment_version() { ++m_viewport_environment_version; }
    [[nodiscard]] Length::FontMetrics const& root_element_font_metrics() const { return m_root_element_font_metrics; }
    [[nodiscard]] bool root_element_font_metrics_depend_on_viewport_metrics() const { return m_root_element_font_metrics_depend_on_viewport_metrics; }
    // The environment as the sharing caches name it: the document's version and the viewport's.
    [[nodiscard]] u64 style_environment_version_for_sharing() const;

    // Drop caches whose keys contain inputs that are stable only within one engine transaction.
    // Style sharing has a self-validating key and survives ordinary transaction boundaries.
    void prepare_for_style_engine_transaction() const;

    void begin_style_update() const;
    void end_style_update() const;
    [[nodiscard]] Parser::ValueParserFFI::FfiMediaEnvironment const* ensure_media_environment_for_style_update() const;

    // The document- and page-wide answers a style computation needs that nothing in a style update
    // can move. Taken once, at the update's begin boundary, so that the sealed stage reads the
    // snapshot rather than the page.
    struct DocumentEnvironmentSnapshot {
        u8 preferred_color_scheme { 0 };
        bool has_supported_color_schemes { false };
        Vector<u8> supported_color_scheme_codes;
        String serialized_base_url;
        double device_pixels_per_css_pixel { 1 };
        // The document half of a row's font length-resolution context. Taken once per update so
        // no row reads the navigable or the document's initial font.
        CSSPixelRect viewport_rect;
        Optional<Length::FontMetrics> initial_font_metrics;
    };
    [[nodiscard]] DocumentEnvironmentSnapshot const& ensure_document_environment_for_style_update() const;

    struct ComputedStyleInvalidation {
        RequiredInvalidationAfterStyleChange invalidation;
        bool any_computed_value_changed { false };
    };

    // Give a layout-only variant of an element or pseudo-element style an authoritative record
    // without replacing the StyleEngine assignment of its DOM target.
    [[nodiscard]] StyleRecordID intern_computed_style_inputs(DOM::AbstractElement, ComputedValues const&) const;
    // Anonymous layout boxes have no style target, but their immutable group tuple is still an
    // authoritative shared record rather than layout-owned complete computed values.
    [[nodiscard]] StyleRecordID intern_anonymous_layout_style(ComputedValues const&) const;

    void pin_style_record(StyleRecordID) const;
    void unpin_style_record(StyleRecordID) const;
    void begin_style_record_view_epoch() const;
    void end_style_record_view_epoch() const;

    void sweep_custom_property_environments() const;
    // The environment the style engine resolved under `identity`, materialized over the data the
    // element inherits, which must be the environment the engine resolved it over. Nothing when
    // the identity is no engine environment or was resolved over another.
    // `did_materialize` reports that the environment object had to be made here, which is what an
    // identity the engine has not been asked for before costs.
    [[nodiscard]] RefPtr<CustomPropertyData const> engine_custom_property_environment(u64 identity, bool* did_materialize = nullptr) const;

    void apply_animation_definitions(DOM::AbstractElement& abstract_element, ReadonlySpan<AnimationProperties> animation_definitions, ReadonlySpan<i32> definition_matches, ReadonlySpan<RefPtr<Animations::KeyframeEffect::KeyFrameSet const>> definition_keyframe_sets, bool in_display_none_subtree) const;
    // The animation plan a record the engine settled left for the host: what the C++ computation of
    // that row would have applied beside the record it computed.
    struct SettledAnimationPlan {
        Vector<AnimationProperties> definitions;
        Vector<i32> definition_matches;
        Vector<RefPtr<Animations::KeyframeEffect::KeyFrameSet const>> definition_keyframe_sets;
        bool in_display_none_subtree { false };
    };
    [[nodiscard]] Optional<SettledAnimationPlan> take_settled_animation_plan(StyleDrainScope const&, StyleNodeID, u8 pseudo_kind) const;
    void apply_settled_animation_plan(DOM::AbstractElement, SettledAnimationPlan const&) const;

    ComputationContext fallback_computation_context_for_custom_property(AbstractOrHypotheticalElement const&) const;

    static NonnullRefPtr<StyleValue const> compute_font_size(NonnullRefPtr<StyleValue const> const& absolutized_value, int computed_math_depth, Optional<DOM::AbstractElement> const& inheritance_parent, CSSPixels initial_font_size = InitialValues::font_size());
    static NonnullRefPtr<StyleValue const> compute_font_style(NonnullRefPtr<StyleValue const> const& absolutized_value);
    static NonnullRefPtr<StyleValue const> compute_font_weight(NonnullRefPtr<StyleValue const> const& absolutized_value, Optional<DOM::AbstractElement> const& inheritance_parent);
    static NonnullRefPtr<StyleValue const> compute_font_width(NonnullRefPtr<StyleValue const> const& absolutized_value);

    [[nodiscard]] NonnullRefPtr<ComputedStyleWorkingSet> reconstruct_computed_properties(ComputedValues const&) const;
    void apply_animated_properties_to_reconstruction(ComputedStyleWorkingSet&, ComputedValues const&) const;
    [[nodiscard]] NonnullRefPtr<ComputedStyleWorkingSet> reconstruct_computed_properties_for_animation(PublishedStyleRecord const&) const;

    void begin_transition_stabilization_epoch();
    // Says whether a baseline was recorded, which is a main-side write.
    bool record_transition_stabilization_baseline(StyleDrainScope const&, DOM::AbstractElement, Optional<StyleRecordID> before_change_style_record = {}) const;
    bool pin_transition_stabilization_baseline_if_a_later_pass_may_need_it(StyleDrainScope const&, DOM::AbstractElement) const;
    // Runs the whole transition step for an installed record, against the record the element moved
    // away from. Returns what publishing a started transition's values invalidates.
    [[nodiscard]] RequiredInvalidationAfterStyleChange run_transition_step_for_installed_record(StyleDrainScope const&, DOM::AbstractElement, StyleRecordID before_change_style_record, StyleEngineFFI::FfiTransitionStepDecidedInPass const* decided = nullptr) const;
    void commit_transition_stabilization_epoch();
    void for_each_provisional_transition_effect_on_element(DOM::Element const&, Function<void(Animations::KeyframeEffect&)> const&) const;

private:
    virtual void finalize() override;

    virtual void visit_edges(Visitor&) override;

    [[nodiscard]] StyleEngine::StyleRecordDelta record_computed_style_inputs(Optional<DOM::AbstractElement>, ComputedValues const&, StyleNodeID style_node_id) const;

    void start_needed_transitions(StyleDrainScope const&, ComputedStyleWorkingSet&, DOM::AbstractElement, StyleRecordID before_change_style_record, StyleEngineFFI::FfiTransitionStepDecidedInPass const*) const;
    [[nodiscard]] bool has_provisional_transition_states(DOM::AbstractElement) const;
    void finalize_style(ComputedStyleWorkingSet&, DOM::AbstractElement, ComputedValuesFFI::FfiStyleFinalizationMode) const;

    [[nodiscard]] CSSPixelRect viewport_rect() const { return m_viewport_rect; }

public:
    // The document's StyleEngine, to read. It is written through the document's render inputs only
    // (DOM::Document::render_inputs_for_write()).
    [[nodiscard]] StyleEngine const& style_engine() const { return m_style_engine.engine(); }
    // What a read asks of the engine that changes no answer of a pass (StyleEngineQueries).
    [[nodiscard]] StyleEngineQueries style_engine_queries() const { return StyleEngineQueries { const_cast<StyleEngine&>(m_style_engine.engine()) }; }

    // The user-agent and user sheets attached to this document's StyleEngine. User-agent sheets are
    // shared between documents, so a per-document identity cannot live on the sheet itself.
    struct NonAuthorStyleSheet {
        RefPtr<StyleSheetState> sheet;
        SheetID sheet_id;
    };
    [[nodiscard]] Vector<NonAuthorStyleSheet>& non_author_style_sheets() { return m_non_author_style_sheets; }

    [[nodiscard]] StyleEngineRuleID style_engine_rule_id_for(RustRule const&) const;
    [[nodiscard]] SheetID style_engine_sheet_id_for(StyleSheetState const&) const;
    void set_style_engine_sheet_id_for(StyleSheetState&, SheetID);

    [[nodiscard]] HashMap<SharedCompiledStyleSheetKey, RefPtr<SharedCompiledStyleSheet>>& shared_compiled_style_sheets() { return m_shared_compiled_style_sheets; }

    // The reverse of a node's style node identity. StyleEngine plans in identities; turning a
    // plan back into nodes needs this, and it is maintained at exactly the two points the
    // identity itself is.
    void register_style_node(StyleNodeID style_node_id, DOM::Node&);
    void ensure_style_node_slot(StyleNodeID);
    void unregister_style_node(StyleNodeID style_node_id);
    [[nodiscard]] GC::Ptr<DOM::Element> element_for_style_node(StyleNodeID style_node_id) const;
    [[nodiscard]] GC::Ptr<DOM::Node> node_for_style_node(StyleNodeID style_node_id) const;
    void prepare_elements_for_style_computation();
    void for_each_style_node(Function<void(DOM::Element&)>) const;

    // Style scopes are numbered per document, with zero naming the document's own scope. A scope is
    // never reused, so a sheet detached with an identity that has been retired detaches nothing
    // rather than something else.
    [[nodiscard]] TreeScopeID allocate_tree_scope() { return ++m_next_tree_scope; }

private:
    GC::Ref<DOM::Document> m_document;

    Length::FontMetrics m_default_font_metrics;
    mutable Length::FontMetrics m_root_element_font_metrics;
    mutable bool m_root_element_font_metrics_depend_on_viewport_metrics { false };

    mutable Optional<ComputationContext> m_cached_font_computation_context;
    mutable Optional<ComputationContext> m_cached_line_height_computation_context;
    mutable Optional<ComputationContext> m_cached_generic_computation_context;
    mutable u64 m_style_update_depth { 0 };
    mutable Optional<MediaEnvironmentSnapshot> m_style_update_media_environment;
    mutable Optional<Parser::ValueParserFFI::FfiMediaEnvironment> m_style_update_ffi_media_environment;
    mutable Optional<DocumentEnvironmentSnapshot> m_style_update_document_environment;

    // The environments the style engine resolved, by the identity it minted, materialized once.
    mutable HashMap<u64, NonnullRefPtr<CustomPropertyData const>> m_engine_custom_property_environments;

    u64 m_viewport_environment_version { 0 };

    enum class ProvisionalTransitionAction : u8 {
        None,
        Remove,
        Cancel,
        Start,
        RemoveAndStart,
        CancelRemoveAndStart,
    };
    struct ProvisionalTransitionState {
        GC::Ptr<DOM::Element> element;
        Optional<PseudoElement> pseudo_element;
        PropertyID property_id;
        GC::Ptr<CSSTransition> committed_transition;
        GC::Ptr<CSSTransition> proposed_transition;
        ProvisionalTransitionAction action { ProvisionalTransitionAction::None };
        bool has_decision { false };
    };
    mutable Vector<ProvisionalTransitionState> m_provisional_transition_states;
    mutable HashMap<u64, size_t> m_provisional_transition_state_indices;
    mutable HashMap<u64, Vector<size_t>> m_provisional_transition_state_indices_by_target;

    ComputationContext make_computation_context_for_property(PropertyID, ComputedStyleWorkingSet const&, Optional<DOM::AbstractElement>) const;
    ComputationContext const& get_computation_context_for_property(PropertyID, ComputedStyleWorkingSet const&, Optional<DOM::AbstractElement>) const;
    void clear_computation_context_caches() const
    {
        const_cast<StyleComputer*>(this)->m_cached_font_computation_context = {};
        const_cast<StyleComputer*>(this)->m_cached_line_height_computation_context = {};
        const_cast<StyleComputer*>(this)->m_cached_generic_computation_context = {};
    }

    bool computation_context_cache_is_empty() const
    {
        return !m_cached_font_computation_context.has_value() && !m_cached_line_height_computation_context.has_value() && !m_cached_generic_computation_context.has_value();
    }

    CSSPixelRect m_viewport_rect;

    // Hands the engine out to write only to the document's render inputs, so that no write to it, here or anywhere,
    // leaves the query snapshot the document published in place.
    class StyleEngineCell {
        AK_MAKE_NONCOPYABLE(StyleEngineCell);
        AK_MAKE_NONMOVABLE(StyleEngineCell);

    public:
        // The engine starts out with the root element's font metrics the style computer starts out with.
        StyleEngineCell(StyleEngine::DeviceClass device_class, StyleComputer* style_computer, ReadonlySpan<u64> root_element_font_metrics, bool root_element_font_metrics_depend_on_viewport_metrics)
            : m_engine(device_class, style_computer)
        {
            m_engine.set_root_element_font_metrics(root_element_font_metrics, root_element_font_metrics_depend_on_viewport_metrics);
        }
        StyleEngine const& engine() const { return m_engine; }
        void visit_edges(GC::Cell::Visitor& visitor) { m_engine.visit_edges(visitor); }

    private:
        friend class DOM::RenderInputs;
        StyleEngine& engine_for_write() { return m_engine; }

        StyleEngine m_engine;
    };
    friend class DOM::RenderInputs;
    StyleEngineCell m_style_engine;
    mutable u32 m_style_record_view_epoch_depth { 0 };
    // Indexed by each kind's dense index; see style_node_is_text().
    // An element or a shadow root; the root is not an element but it owns a child sequence, so it is
    // named here too.
    Vector<GC::Ptr<DOM::Node>> m_element_style_nodes;
    Vector<GC::Ptr<DOM::Node>> m_text_style_nodes;
    TreeScopeID m_next_tree_scope;
    Vector<NonAuthorStyleSheet> m_non_author_style_sheets;
    HashMap<RefPtr<StyleSheetState const>, SheetID> m_constructed_sheet_ids;
    HashMap<SharedCompiledStyleSheetKey, RefPtr<SharedCompiledStyleSheet>> m_shared_compiled_style_sheets;
    HashMap<u64, WeakPtr<StyleSheetState const>> m_style_engine_sheet_sources;
};

}
