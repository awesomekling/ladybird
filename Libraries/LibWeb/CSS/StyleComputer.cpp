/*
 * Copyright (c) 2018-2025, Andreas Kling <andreas@ladybird.org>
 * Copyright (c) 2021, the SerenityOS developers.
 * Copyright (c) 2021-2026, Sam Atkins <sam@ladybird.org>
 * Copyright (c) 2024, Matthew Olsson <mattco@serenityos.org>
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/Array.h>
#include <AK/BinarySearch.h>
#include <AK/Bitmap.h>
#include <AK/BuiltinWrappers.h>
#include <AK/Debug.h>
#include <AK/Error.h>
#include <AK/Find.h>
#include <AK/FixedBitmap.h>
#include <AK/Function.h>
#include <AK/HashMap.h>
#include <AK/HashTable.h>
#include <AK/JsonObject.h>
#include <AK/Math.h>
#include <AK/NeverDestroyed.h>
#include <AK/NonnullRawPtr.h>
#include <AK/QuickSort.h>
#include <AK/ScopeGuard.h>
#include <AK/Utf8View.h>
#include <LibGC/Heap.h>
#include <LibGfx/Font/FontDatabase.h>
#include <LibWeb/Animations/AnimationEffect.h>
#include <LibWeb/Animations/DocumentTimeline.h>
#include <LibWeb/Animations/ScrollTimeline.h>
#include <LibWeb/Bindings/PrincipalHostDefined.h>
#include <LibWeb/CSS/AnimationEvent.h>
#include <LibWeb/CSS/CSSAnimation.h>
#include <LibWeb/CSS/CSSImportRule.h>
#include <LibWeb/CSS/CSSLayerBlockRule.h>
#include <LibWeb/CSS/CSSLayerStatementRule.h>
#include <LibWeb/CSS/CSSNestedDeclarations.h>
#include <LibWeb/CSS/CSSScopeRule.h>
#include <LibWeb/CSS/CSSStyleProperties.h>
#include <LibWeb/CSS/CSSStyleRule.h>
#include <LibWeb/CSS/CSSTransition.h>
#include <LibWeb/CSS/ComputedStyleWorkingSet.h>
#include <LibWeb/CSS/ContainerQuery.h>
#include <LibWeb/CSS/CustomPropertyData.h>
#include <LibWeb/CSS/CustomPropertyRegistration.h>
#include <LibWeb/CSS/FontComputer.h>
#include <LibWeb/CSS/FontFace.h>
#include <LibWeb/CSS/FontFaceState.h>
#include <LibWeb/CSS/HypotheticalElement.h>
#include <LibWeb/CSS/Parser/Parser.h>
#include <LibWeb/CSS/Parser/SyntaxParsing.h>
#include <LibWeb/CSS/PropertyNameAndID.h>
#include <LibWeb/CSS/SelectorMatching.h>
#include <LibWeb/CSS/StyleComputeFFI.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleDrainScope.h>
#include <LibWeb/CSS/StyleEffectDrain.h>
#include <LibWeb/CSS/StyleEngineInput.h>
#include <LibWeb/CSS/StyleInputScope.h>
#include <LibWeb/CSS/StyleProperty.h>
#include <LibWeb/CSS/StyleScope.h>
#include <LibWeb/CSS/StyleSheetIdentifier.h>
#include <LibWeb/CSS/StyleSheetState.h>
#include <LibWeb/CSS/StyleValues/AngleStyleValue.h>
#include <LibWeb/CSS/StyleValues/BorderRadiusStyleValue.h>
#include <LibWeb/CSS/StyleValues/ColorStyleValue.h>
#include <LibWeb/CSS/StyleValues/CounterStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/CustomIdentStyleValue.h>
#include <LibWeb/CSS/StyleValues/DisplayStyleValue.h>
#include <LibWeb/CSS/StyleValues/FontStyleStyleValue.h>
#include <LibWeb/CSS/StyleValues/FrequencyStyleValue.h>
#include <LibWeb/CSS/StyleValues/FunctionStyleValue.h>
#include <LibWeb/CSS/StyleValues/IntegerStyleValue.h>
#include <LibWeb/CSS/StyleValues/KeywordStyleValue.h>
#include <LibWeb/CSS/StyleValues/LengthStyleValue.h>
#include <LibWeb/CSS/StyleValues/NumberStyleValue.h>
#include <LibWeb/CSS/StyleValues/OpenTypeTaggedStyleValue.h>
#include <LibWeb/CSS/StyleValues/PendingSubstitutionStyleValue.h>
#include <LibWeb/CSS/StyleValues/PercentageStyleValue.h>
#include <LibWeb/CSS/StyleValues/PositionStyleValue.h>
#include <LibWeb/CSS/StyleValues/RandomValueSharingStyleValue.h>
#include <LibWeb/CSS/StyleValues/RatioStyleValue.h>
#include <LibWeb/CSS/StyleValues/RectStyleValue.h>
#include <LibWeb/CSS/StyleValues/ShorthandStyleValue.h>
#include <LibWeb/CSS/StyleValues/StringStyleValue.h>
#include <LibWeb/CSS/StyleValues/StyleValueList.h>
#include <LibWeb/CSS/StyleValues/SuperellipseStyleValue.h>
#include <LibWeb/CSS/StyleValues/TimeStyleValue.h>
#include <LibWeb/CSS/StyleValues/TransformationStyleValue.h>
#include <LibWeb/CSS/StyleValues/UnresolvedStyleValue.h>
#include <LibWeb/DOM/Attr.h>
#include <LibWeb/DOM/CommitMessages.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/Element.h>
#include <LibWeb/DOM/SelectorQuery.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/HTML/AttributeNames.h>
#include <LibWeb/HTML/HTMLBRElement.h>
#include <LibWeb/HTML/HTMLImageElement.h>
#include <LibWeb/HTML/HTMLInputElement.h>
#include <LibWeb/HTML/HTMLSlotElement.h>
#include <LibWeb/HTML/Parser/HTMLParser.h>
#include <LibWeb/HTML/Scripting/Environments.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Namespace.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxSlot.h>
#include <LibWeb/Platform/FontPlugin.h>
#include <LibWeb/SVG/SVGElement.h>
#include <LibWeb/StyleValueRustFFI.h>
#include <LibWeb/ValueParserRustFFI.h>
#include <math.h>
#include <stdio.h>

extern "C" void rust_style_seal_flush_census_for_update();

namespace Web::CSS {

// A host computation runs the transition step right after installing its record, and hands the
// step's invalidation to its own caller, which reacts to it like to any other style change. The
// step then asks for no further style work of its own: the host computation layers no provisional
// transition, so a base recomputation it asked for would only undo what the step published.
static thread_local bool g_transition_step_follow_up_left_to_caller = false;

void set_transition_step_follow_up_left_to_caller(bool);

void set_transition_step_follow_up_left_to_caller(bool left_to_caller)
{
    g_transition_step_follow_up_left_to_caller = left_to_caller;
}

GC_DEFINE_ALLOCATOR(StyleComputer);

// What a rule contributes, for the two rule types that carry a declaration block.
static RustDeclarationBlock const& declaration_of_rule(CSSRule const& rule)
{
    if (rule.type() == CSSRule::Type::Style)
        return static_cast<CSSStyleRule const&>(rule).declaration();
    if (rule.type() == CSSRule::Type::NestedDeclarations)
        return static_cast<CSSNestedDeclarations const&>(rule).declaration();
    VERIFY_NOT_REACHED();
}

static Array<u64, 5> font_metrics_words(Length::FontMetrics const& metrics)
{
    return {
        bit_cast<u64>(metrics.font_size.to_double()),
        bit_cast<u64>(metrics.x_height.to_double()),
        bit_cast<u64>(metrics.cap_height.to_double()),
        bit_cast<u64>(metrics.zero_advance.to_double()),
        bit_cast<u64>(metrics.line_height.to_double()),
    };
}

StyleComputer::StyleComputer(DOM::Document& document)
    : m_document(document)
    , m_default_font_metrics(16, Platform::FontPlugin::the().default_font(16)->pixel_metrics(), InitialValues::line_height())
    , m_root_element_font_metrics(m_default_font_metrics)
    , m_style_engine(document.render_state_arena({}), StyleEngine::DeviceClass::ForegroundDesktop, this, font_metrics_words(m_root_element_font_metrics).span(), m_root_element_font_metrics_depend_on_viewport_metrics)
{
    // The style engine decides which groups a winner reaches from the dependency masks the default
    // group payloads register. Register them before the engine computes its first record, which is
    // the document element's, rather than when C++ first reads a group.
    style_group_default_payload(0);
}

void StyleComputer::finalize()
{
    Base::finalize();
}

void StyleComputer::prepare_for_style_engine_transaction() const
{
    sweep_custom_property_environments();
}

void StyleComputer::begin_style_update() const
{
    ++m_style_update_depth;
    begin_deferred_web_face_loads();
}

void StyleComputer::end_style_update() const
{
    VERIFY(m_style_update_depth > 0);
    // Loading a web face a style selected runs author callbacks and starts a fetch, so it waits
    // for the outermost update to finish. This guard is what lets it happen.
    ScopeGuard drain_deferred_web_face_loads = [] { end_deferred_web_face_loads(); };
    if (--m_style_update_depth != 0)
        return;
    rust_style_seal_flush_census_for_update();
    m_style_update_ffi_media_environment.clear();
    m_style_update_media_environment.clear();
    m_style_update_document_environment.clear();
}

Parser::ValueParserFFI::FfiMediaEnvironment const* StyleComputer::ensure_media_environment_for_style_update() const
{
    // NB: Outside a style update there is nothing to clear the cached snapshot, so always take a
    //     fresh one. Every style-computation entry point currently opens a scope, so this is a
    //     defensive path rather than one the engine relies on.
    if (m_style_update_depth == 0 || !m_style_update_media_environment.has_value()) {
        m_style_update_media_environment.emplace(m_document);
        m_style_update_ffi_media_environment = m_style_update_media_environment->ffi_environment();
    }
    return &*m_style_update_ffi_media_environment;
}

StyleComputer::DocumentEnvironmentSnapshot const& StyleComputer::ensure_document_environment_for_style_update() const
{
    // NB: Outside a style update there is nothing to clear the cached snapshot, so always take a
    //     fresh one, the way the media environment above does.
    if (m_style_update_depth == 0 || !m_style_update_document_environment.has_value()) {
        DocumentEnvironmentSnapshot snapshot;
        snapshot.preferred_color_scheme = static_cast<u8>(to_underlying(document().page().preferred_color_scheme()));
        if (auto supported = document().supported_color_schemes(); supported.has_value()) {
            snapshot.has_supported_color_schemes = true;
            snapshot.supported_color_scheme_codes.ensure_capacity(supported->size());
            for (auto const& scheme : *supported)
                snapshot.supported_color_scheme_codes.unchecked_append(to_underlying(preferred_color_scheme_from_string(scheme)));
        }
        snapshot.serialized_base_url = document().serialized_base_url();
        snapshot.device_pixels_per_css_pixel = document().page().client().device_pixels_per_css_pixel();
        snapshot.viewport_rect = viewport_rect();
        auto const& initial_font = document().font_computer().initial_font();
        snapshot.initial_font_metrics = Length::FontMetrics { CSSPixels { initial_font.pixel_size() }, initial_font.pixel_metrics(), InitialValues::line_height() };
        m_style_update_document_environment = move(snapshot);
    }
    return *m_style_update_document_environment;
}

void StyleComputer::pin_style_record(StyleRecordID style_record_identity) const
{
    VERIFY(style_record_identity);
    m_style_engine.engine().pin_style_record(style_record_identity);
}

void StyleComputer::unpin_style_record(StyleRecordID style_record_identity) const
{
    VERIFY(style_record_identity);
    m_style_engine.engine().unpin_style_record(style_record_identity);
}

void StyleComputer::begin_style_record_view_epoch() const
{
    if (m_style_record_view_epoch_depth++ == 0)
        style_engine_queries().begin_style_record_view_epoch();
}

void StyleComputer::end_style_record_view_epoch() const
{
    VERIFY(m_style_record_view_epoch_depth > 0);
    if (--m_style_record_view_epoch_depth == 0)
        style_engine_queries().end_style_record_view_epoch();
}

void StyleComputer::register_style_node(StyleNodeID style_node_id, DOM::Node& node)
{
    if (style_node_id == 0)
        return;
    ensure_style_node_slot(style_node_id);
    if (style_node_is_text(style_node_id)) {
        m_text_style_nodes[style_node_index(style_node_id)] = node;
        return;
    }
    m_element_style_nodes[style_node_index(style_node_id)] = node;
    // Registration is where an element's publications keyed by its style node begin: an attribute
    // written before it had one published nothing.
    if (auto* svg_element = as_if<SVG::SVGElement>(node))
        Layout::publish_svg_attribute_facts(*svg_element);
}

static void ensure_slot(auto& nodes, u32 index)
{
    if (index >= nodes.size()) {
        nodes.grow_capacity(index + 1);
        nodes.resize(index + 1);
    }
}

void StyleComputer::ensure_style_node_slot(StyleNodeID style_node_id)
{
    if (style_node_id == 0)
        return;
    if (style_node_is_text(style_node_id))
        ensure_slot(m_text_style_nodes, style_node_index(style_node_id));
    else
        ensure_slot(m_element_style_nodes, style_node_index(style_node_id));
}

void StyleComputer::unregister_style_node(StyleNodeID style_node_id)
{
    if (style_node_id == 0)
        return;
    auto index = style_node_index(style_node_id);
    if (style_node_is_text(style_node_id)) {
        if (index < m_text_style_nodes.size())
            m_text_style_nodes[index] = nullptr;
        return;
    }
    if (index < m_element_style_nodes.size()) {
        // A style node identity is reissued, so what was published under it leaves with it.
        if (auto* svg_element = as_if<SVG::SVGElement>(m_element_style_nodes[index].ptr()))
            Layout::clear_svg_attribute_facts(svg_element->document(), style_node_id);
        auto& style_engine = m_document->render_inputs_for_write().style_engine();
        style_engine.note_style_node_retired(style_node_id);
        m_element_style_nodes[index] = nullptr;
        style_engine.publish_input([style_node_id](StyleInputScope const& input) {
            input.engine().consume_recorded_element_style_input_change(input, style_node_id);
        });
    }
}

GC::Ptr<DOM::Element> StyleComputer::element_for_style_node(StyleNodeID style_node_id) const
{
    return as_if<DOM::Element>(node_for_style_node(style_node_id).ptr());
}

GC::Ptr<DOM::Node> StyleComputer::node_for_style_node(StyleNodeID style_node_id) const
{
    if (!style_node_is_text(style_node_id)) {
        if (style_node_id == 0 || style_node_id.value() >= m_element_style_nodes.size())
            return nullptr;
        return m_element_style_nodes[style_node_id.value()];
    }
    auto index = style_node_index(style_node_id);
    if (index >= m_text_style_nodes.size())
        return nullptr;
    return m_text_style_nodes[index];
}

void StyleComputer::prepare_elements_for_style_computation()
{
    for (;;) {
        if (!style_engine().has_elements_awaiting_first_style_computation())
            break;
        auto elements = m_document->render_inputs_for_write().style_engine().take_elements_awaiting_first_style_computation();
        if (elements.is_empty())
            break;
        for (auto style_node : elements) {
            auto element = element_for_style_node(style_node);
            if (element && element->is_connected())
                element->prepare_for_style_computation({});
        }
    }
}

void StyleComputer::for_each_style_node(Function<void(DOM::Element&)> callback) const
{
    for (auto node : m_element_style_nodes) {
        if (auto* element = as_if<DOM::Element>(node.ptr()))
            callback(*element);
    }
}

void StyleComputer::visit_edges(Visitor& visitor)
{
    Base::visit_edges(visitor);
    visitor.visit(m_document);
    m_style_engine.visit_edges(visitor);
    visitor.visit(m_element_style_nodes);
    visitor.visit(m_text_style_nodes);
    // NB: Source sheets are weak references; their owners trace them.
    visitor.ignore(m_style_engine_sheet_sources);
    for (auto const& entry : m_non_author_style_sheets)
        visitor.visit(entry.sheet);
    for (auto const& entry : m_constructed_sheet_ids)
        visitor.visit(entry.key);
    for (auto const& entry : m_shared_compiled_style_sheets) {
        if (entry.value)
            entry.value->contents().visit_edges(visitor);
    }

    if (m_cached_font_computation_context.has_value())
        m_cached_font_computation_context->visit_edges(visitor);
    if (m_cached_line_height_computation_context.has_value())
        m_cached_line_height_computation_context->visit_edges(visitor);
    for (auto const& state : m_provisional_transition_states) {
        visitor.visit(state.element);
        visitor.visit(state.committed_transition);
        visitor.visit(state.proposed_transition);
    }
    if (m_cached_generic_computation_context.has_value())
        m_cached_generic_computation_context->visit_edges(visitor);
}

void StyleComputer::begin_transition_stabilization_epoch()
{
    VERIFY(m_provisional_transition_states.is_empty());
    VERIFY(m_provisional_transition_state_indices.is_empty());
    VERIFY(m_provisional_transition_state_indices_by_target.is_empty());
}

// Whether a later pass of the stabilization epoch can still give this element a transition whose
// before-change style is the one it holds now: the element's scope has size container queries, so
// a later pass can happen at all, or a later pass has already happened.
bool StyleComputer::pin_transition_stabilization_baseline_if_a_later_pass_may_need_it(StyleDrainScope const& scope, DOM::AbstractElement abstract_element) const
{
    if (abstract_element.element().style_node_id() == 0)
        return false;
    if (!abstract_element.style_scope().rule_cache().has_size_container_queries
        && !document().is_in_style_stabilization_feedback_epoch())
        return false;
    return record_transition_stabilization_baseline(scope, abstract_element);
}

bool StyleComputer::record_transition_stabilization_baseline(StyleDrainScope const& scope, DOM::AbstractElement abstract_element, Optional<StyleRecordID> before_change_style_record) const
{
    auto style_node_id = abstract_element.element().style_node_id();
    if (style_node_id == 0)
        return false;
    // A row the engine settled is drained once its record is installed, so the style the element
    // holds is already the after-change one. The row names the style it moved away from.
    auto style_record_identity = before_change_style_record.value_or_lazy_evaluated([&] { return abstract_element.style_record_identity(); });
    return scope.engine().record_transition_baseline(scope, style_node_id, pseudo_element_to_ffi(abstract_element.pseudo_element()), style_record_identity);
}

// A provisionally started transition already contributed to the style published by the pass that
// started it, but it is not associated with its target until the stabilization epoch commits. An
// animated style update running before that commit samples it anyway, from the timing rows the
// element publishes for every one of its animation lists, or it rebuilds the target's style without
// the transition's value and clobbers it.
void StyleComputer::for_each_provisional_transition_effect_on_element(DOM::Element const& element, Function<void(Animations::KeyframeEffect&)> const& callback) const
{
    for (auto const& state : m_provisional_transition_states) {
        if (state.element.ptr() != &element)
            continue;
        if (!state.proposed_transition)
            continue;
        if (auto effect = state.proposed_transition->effect(); effect && effect->is_keyframe_effect())
            callback(static_cast<Animations::KeyframeEffect&>(*effect));
    }
}

static void release_transition_baselines(StyleDrainScope const& scope)
{
    scope.engine().release_transition_baselines(scope);
}

void StyleComputer::commit_transition_stabilization_epoch()
{
    for (auto const& state : m_provisional_transition_states) {
        VERIFY(state.element);
        auto& element = *state.element;
        auto remove_committed_transition = [&] {
            if (element.property_transition(state.pseudo_element, state.property_id) == state.committed_transition)
                element.remove_transition(state.pseudo_element, state.property_id);
        };
        auto cancel_and_remove_committed_transition = [&] {
            VERIFY(state.committed_transition);
            state.committed_transition->cancel();
            remove_committed_transition();
        };
        auto commit_proposed_transition = [&] {
            VERIFY(state.proposed_transition);
            state.proposed_transition->commit_provisional_transition();
            ++document().style_invalidation_counters().committed_transitions_started;
        };

        switch (state.action) {
        case ProvisionalTransitionAction::None:
            continue;
        case ProvisionalTransitionAction::Remove:
            remove_committed_transition();
            break;
        case ProvisionalTransitionAction::Cancel:
            VERIFY(state.committed_transition);
            state.committed_transition->cancel();
            break;
        case ProvisionalTransitionAction::Start:
            commit_proposed_transition();
            break;
        case ProvisionalTransitionAction::RemoveAndStart:
            remove_committed_transition();
            commit_proposed_transition();
            break;
        case ProvisionalTransitionAction::CancelRemoveAndStart:
            cancel_and_remove_committed_transition();
            commit_proposed_transition();
            break;
        }
        ++document().style_invalidation_counters().committed_transition_actions;
    }
    // A provisional transition's timing row is published while the epoch is open. Once it closes,
    // every one of those transitions has either been associated with its target or dropped, so the
    // rows the targets publish have to be built again from what they hold now.
    GC::ConservativeVector<GC::Ref<DOM::Element>> elements_with_provisional_rows;
    for (auto const& state : m_provisional_transition_states) {
        if (state.proposed_transition && !elements_with_provisional_rows.contains_slow(GC::Ref { *state.element }))
            elements_with_provisional_rows.append(*state.element);
    }
    m_provisional_transition_states.clear();
    m_provisional_transition_state_indices.clear();
    m_provisional_transition_state_indices_by_target.clear();
    for (auto& element : elements_with_provisional_rows)
        element->publish_animation_timing_rows();
    // The before-change styles the epoch's drains pinned are released with the last of them.
    StyleEffectDrain::install(document(), release_transition_baselines);
}

template<size_t length>
static constexpr Utf16View utf16_view(char16_t const (&string)[length])
{
    return { string, length - 1 };
}

Optional<Utf16String> StyleComputer::user_agent_style_sheet_source(Utf16View name)
{
    extern String const& default_stylesheet_source;
    extern String const& quirks_mode_stylesheet_source;
    extern String const& mathml_stylesheet_source;
    extern String const& svg_stylesheet_source;

    if (name == utf16_view(u"CSS/Default.css"))
        return Utf16String::from_utf8(default_stylesheet_source);
    if (name == utf16_view(u"CSS/QuirksMode.css"))
        return Utf16String::from_utf8(quirks_mode_stylesheet_source);
    if (name == utf16_view(u"MathML/Default.css"))
        return Utf16String::from_utf8(mathml_stylesheet_source);
    if (name == utf16_view(u"SVG/Default.css"))
        return Utf16String::from_utf8(svg_stylesheet_source);
    return {};
}

void StyleComputer::for_each_property_expanding_shorthands(PropertyID property_id, StyleValue const& value, Function<void(PropertyID, StyleValue const&)> const& set_longhand_property)
{
    // The expansion recursion and pending-substitution values live in the Rust style value graph.
    // This wrapper creates C++ facades only for the returned longhand roots.
    auto expansion = ComputedValuesFFI::rust_expand_property_shorthands(
        to_underlying(property_id), value.rust_style_value_data());
    ScopeGuard destroy_expansion = [&] {
        ComputedValuesFFI::rust_shorthand_expansion_destroy(expansion.storage);
    };
    HashMap<void const*, NonnullRefPtr<StyleValue const>> wrapper_cache;
    for (size_t i = 0; i < expansion.count; ++i) {
        auto const& property = expansion.properties[i];
        auto& expanded_value = wrapper_cache.ensure(property.data, [&] {
            return StyleValue::adopt_rust_style_value_data(StyleValueFFI::rust_style_value_retain(
                static_cast<StyleValueFFI::StyleValueData const*>(property.data)));
        });
        set_longhand_property(static_cast<PropertyID>(property.property_id), *expanded_value);
    }
}

// The size of the element's transform reference box, as the last committed layout left it, which a
// keyframe or transition resolves a percentage translation against. The stage asks the layout arena
// by identity rather than following the element's layout-node pointer: the box is an earlier stage's
// committed output, and the pointer is a live read of a later stage's objects.
static void apply_committed_transform_reference_box(StyleDrainScope const& scope, DOM::AbstractElement abstract_element, StyleValueFFI::FfiAnimationContext& animation_context)
{
    auto committed = StyleEngineFFI::layout_arena_committed_transform_reference_box(scope,
        abstract_element.document().layout_arena_handle(), abstract_element.element().style_node_id().value());
    if (!committed.has_box)
        return;
    animation_context.has_transform_reference_box = true;
    animation_context.transform_reference_box_width = committed.width;
    animation_context.transform_reference_box_height = committed.height;
}

// The animation plan a style computation decided, in the shape the plan application takes it in.
static void marshal_animation_definitions(ReadonlySpan<ComputedValuesFFI::FfiComputedAnimation> animations, Vector<AnimationProperties>& animation_definitions, Vector<i32>& definition_matches, Vector<RefPtr<Animations::KeyframeEffect::KeyFrameSet const>>& definition_keyframe_sets)
{
    animation_definitions.ensure_capacity(animations.size());
    definition_matches.ensure_capacity(animations.size());
    definition_keyframe_sets.ensure_capacity(animations.size());
    for (auto const& animation : animations) {
        definition_matches.unchecked_append(animation.matched_existing_index);
        definition_keyframe_sets.unchecked_append(static_cast<Animations::KeyframeEffect::KeyFrameSet const*>(animation.keyframe_set));
        Variant<double, Utf16String> duration { animation.duration };
        if (animation.duration_is_auto)
            duration = "auto"_utf16;
        auto timing_function = StyleValue::adopt_rust_style_value_data(StyleValueFFI::rust_style_value_retain(
            static_cast<StyleValueFFI::StyleValueData const*>(animation.timing_function)));
        static_assert(to_underlying(AnimationTimelineSource::Kind::Document) == to_underlying(ComputedValuesFFI::FfiAnimationTimelineKind::Document));
        static_assert(to_underlying(AnimationTimelineSource::Kind::None) == to_underlying(ComputedValuesFFI::FfiAnimationTimelineKind::None));
        static_assert(to_underlying(AnimationTimelineSource::Kind::Scroll) == to_underlying(ComputedValuesFFI::FfiAnimationTimelineKind::Scroll));
        AnimationTimelineSource timeline {
            .kind = static_cast<AnimationTimelineSource::Kind>(animation.timeline_kind),
            .scroller = static_cast<Scroller>(animation.scroll_scroller),
            .axis = static_cast<Axis>(animation.scroll_axis),
        };
        animation_definitions.unchecked_append({
            .duration = move(duration),
            .timing_function = EasingFunction::from_style_value(timing_function),
            .iteration_count = animation.iteration_count,
            .direction = static_cast<AnimationDirection>(animation.direction),
            .play_state = static_cast<AnimationPlayState>(animation.play_state),
            .delay = animation.delay,
            .fill_mode = static_cast<AnimationFillMode>(animation.fill_mode),
            .composition = static_cast<AnimationComposition>(animation.composition),
            .name = css_string_from_rust(animation.name),
            .timeline = timeline,
            .timing_function_value = RustStyleValueHandle::retained(
                static_cast<StyleValueFFI::StyleValueData const*>(animation.timing_function)),
        });
    }
}

// Takes the animation plan a record the engine settled left for the host, out of the engine's own
// storage and into the batch, which applies it once every record is installed.
Optional<StyleComputer::SettledAnimationPlan> StyleComputer::take_settled_animation_plan(StyleDrainScope const& scope, StyleNodeID style_node, u8 pseudo_kind) const
{
    auto taken = scope.engine().take_settled_animation_definitions(scope, style_node, pseudo_kind);
    if (!taken.owed)
        return {};
    SettledAnimationPlan plan;
    plan.in_display_none_subtree = taken.in_display_none_subtree;
    marshal_animation_definitions(taken.definitions, plan.definitions, plan.definition_matches, plan.definition_keyframe_sets);
    return plan;
}

// The animation plan a record the engine settled left for the host, applied once the record is
// installed.
//
// The plan names both existing animations to keep or retime and definitions to start. The
// sampling pass after installation publishes their values.
void StyleComputer::apply_settled_animation_plan(DOM::AbstractElement abstract_element, SettledAnimationPlan const& plan) const
{
    auto const* existing_animations = abstract_element.css_defined_animations();
    if (!existing_animations)
        return;
    // Installing the record can have cancelled animations the plan was decided against, so which
    // animation each definition claims is decided against the ones the element holds now.
    Vector<i32> matches;
    matches.ensure_capacity(plan.definitions.size());
    for (size_t i = 0; i < plan.definitions.size(); ++i)
        matches.unchecked_append(-1);
    Vector<bool> claimed;
    claimed.resize(existing_animations->size());
    for (size_t i = plan.definitions.size(); i-- > 0;) {
        for (size_t candidate = existing_animations->size(); candidate-- > 0;) {
            if (claimed[candidate] || (*existing_animations)[candidate]->animation_name() != plan.definitions[i].name)
                continue;
            claimed[candidate] = true;
            matches[i] = candidate;
            break;
        }
    }
    apply_animation_definitions(abstract_element, plan.definitions, matches, plan.definition_keyframe_sets, plan.in_display_none_subtree);
}

void StyleComputer::apply_animation_definitions(DOM::AbstractElement& abstract_element, ReadonlySpan<AnimationProperties> animation_definitions, ReadonlySpan<i32> definition_matches, ReadonlySpan<RefPtr<Animations::KeyframeEffect::KeyFrameSet const>> definition_keyframe_sets, bool in_display_none_subtree) const
{
    auto& document = abstract_element.document();

    auto const* element_animations = abstract_element.css_defined_animations();

    // If we have a nullptr for element_animations it means that the pseudo element was invalid and thus we shouldn't apply animations
    if (!element_animations)
        return;

    // https://drafts.csswg.org/css-animations-1/#animations
    // Setting the 'display' property to 'none' will terminate any running animation applied to the element and its
    // descendants. If an element has a 'display' of 'none', updating 'display' to a value other than 'none' will
    // start all animations applied to the element by the 'animation-name' property, as well as all animations
    // applied to descendants with 'display' other than 'none'.
    // NB: We must not start animations on elements that are not rendered due to display:none. Once display becomes
    //     something other than none, the resulting style recomputation re-enters this function and starts them.
    //     Termination of running animations when display becomes none is handled by
    //     Element::play_or_cancel_animations_after_display_property_change(). The style engine answers whether
    //     the element is in such a subtree.

    // Which animation each definition claims is decided by the style computation, from the names of
    // the animations this element owns, which it publishes. What it decides is what
    // https://drafts.csswg.org/css-animations-1/#animations describes: the new list is walked last
    // to first, and each definition takes the last existing animation of the same name that no later
    // definition has taken, so updating animation-name from 'a' to 'a, a' makes the existing
    // animation the second of the two and creates the first.
    VERIFY(definition_matches.size() == animation_definitions.size());
    VERIFY(definition_keyframe_sets.size() == animation_definitions.size());

    auto existing_animations = *element_animations;
    Vector<bool> existing_animation_was_claimed;
    existing_animation_was_claimed.resize(existing_animations.size());
    Vector<GC::Ref<CSSAnimation>> new_animations;

    for (size_t i = animation_definitions.size(); i-- > 0;) {
        auto const& animation_properties = animation_definitions[i];

        auto matched_index = definition_matches[i];
        VERIFY(matched_index < static_cast<i32>(existing_animations.size()));

        if (matched_index >= 0) {
            auto existing_animation = existing_animations[matched_index];
            VERIFY(!existing_animation_was_claimed[matched_index]);
            existing_animation_was_claimed[matched_index] = true;

            if (auto effect = existing_animation->effect()) {
                as<Animations::KeyframeEffect>(*effect).set_key_frame_set(definition_keyframe_sets[i]);
                existing_animation->apply_css_properties(animation_properties, abstract_element);
            }
            existing_animation->set_animation_name_index(i);
            new_animations.append(existing_animation);
            continue;
        }

        if (in_display_none_subtree)
            continue;

        // An animation applies to an element if its name appears as one of the identifiers in the computed value of the
        // animation-name property and the animation uses a valid @keyframes rule
        auto animation = CSSAnimation::create(document.relevant_settings_object());
        animation->set_animation_name(animation_properties.name);
        animation->set_owning_element(abstract_element);

        auto effect = Animations::KeyframeEffect::create();
        animation->set_effect(effect);

        animation->apply_css_properties(animation_properties, abstract_element);
        animation->set_animation_name_index(i);

        effect->set_key_frame_set(definition_keyframe_sets[i]);

        effect->set_target(abstract_element);
        new_animations.append(animation);
    }

    // Once an animation has started it continues until it ends or the animation-name is removed
    // NB: An animation no definition claimed is one whose animation-name entry has gone.
    for (size_t i = 0; i < existing_animations.size(); ++i) {
        if (!existing_animation_was_claimed[i])
            existing_animations[i]->cancel(Animations::Animation::ShouldInvalidate::No);
    }

    // NB: We create animations in reverse definition order so flip it back.
    new_animations.reverse();

    abstract_element.set_css_defined_animations(move(new_animations));

    // The plan just created, retimed and cancelled animations of this element. Republish their
    // timing so the rest of this style update reads what they are now, not what they were.
    abstract_element.element().publish_animation_timing_rows();
}

static void collect_dimension_attribute(Vector<StyleProperty>& properties, DOM::Element const& element, Utf16FlyString const& attribute_name, CSS::PropertyID property_id)
{
    auto attribute = element.attribute(attribute_name);
    if (!attribute.has_value())
        return;

    auto parsed_value = HTML::parse_dimension_value(*attribute);
    if (!parsed_value)
        return;

    properties.append({ .property_id = property_id, .value = parsed_value.release_nonnull() });
}

// The engine's sample of an installed record, over the effects its timing rows name.
static StyleEngineFFI::FfiRowSampledInPass resample_installed_record_after_host_step(StyleDrainScope const& scope, DOM::AbstractElement abstract_element, StyleRecordID installed_style_record)
{
    auto& element = abstract_element.element();
    auto resampled = StyleEngineFFI::style_engine_sample_installed_record(scope.engine().rust_handle(), element.style_node_id().value(),
        pseudo_element_to_ffi(abstract_element.pseudo_element()), installed_style_record.value(), element.document().layout_arena_handle());
    if (resampled.present && resampled.custom_property_environment_moved)
        Animations::install_sampled_custom_property_environment(scope, abstract_element, resampled);
    return resampled;
}

// Whether a transition step the engine decided names each property the host decides the step over:
// those with a matching transition-property entry, followed by those of the element's transitions
// without one.
static bool transition_step_names_each_property(StyleEngineFFI::FfiTransitionStepDecidedInPass const& step, ReadonlySpan<PropertyID> matching_property_ids, ReadonlySpan<PropertyID> existing_property_ids)
{
    // A `transition: all` names every longhand, so the lists are looked up by property, not scanned.
    ReadonlySpan<StyleEngineFFI::FfiTransitionStepAction> actions { step.actions, step.action_count };
    HashTable<u16> named_property_ids;
    named_property_ids.ensure_capacity(actions.size());
    for (auto const& action : actions)
        named_property_ids.set(action.property_id);
    auto names = [&](PropertyID property_id) { return named_property_ids.contains(to_underlying(property_id)); };
    HashTable<PropertyID> matching_property_id_set;
    matching_property_id_set.ensure_capacity(matching_property_ids.size());
    for (auto property_id : matching_property_ids)
        matching_property_id_set.set(property_id);
    size_t property_count = matching_property_ids.size();
    for (auto property_id : existing_property_ids) {
        if (!matching_property_id_set.contains(property_id))
            ++property_count;
    }
    return actions.size() == property_count && all_of(matching_property_ids, names) && all_of(existing_property_ids, names);
}

// The whole transition step for an element's record, run once the record is installed, whether
// the engine settled it or the host computed it.
//
// The step needs two styles: the one the element moved away from, which the caller names, and the
// one it moved to, which is the record just installed. Both are records, and the step reads the
// before-change half only through `decide_transitions`' baseline, so naming the old record is the
// whole of what the step needs. A transition the step starts layers its current
// values into the working set to keep the frame from jumping; publishing that is the same animation
// overlay publication an animation sampling performs, on the same element, against the same base.
RequiredInvalidationAfterStyleChange StyleComputer::run_transition_step_for_installed_record(StyleDrainScope const& scope, DOM::AbstractElement abstract_element, StyleRecordID before_change_style_record, StyleEngineFFI::FfiTransitionStepDecidedInPass const* decided) const
{
    auto installed_style = abstract_element.computed_style();
    if (!installed_style)
        return {};
    auto installed_style_record = abstract_element.style_record_identity();
    VERIFY(installed_style_record);

    // https://drafts.csswg.org/css-transitions-2/#defining-before-change-style
    (void)record_transition_stabilization_baseline(scope, abstract_element, before_change_style_record);
    if (auto baseline = scope.engine().transition_baseline(scope, abstract_element.element().style_node_id(), pseudo_element_to_ffi(abstract_element.pseudo_element())); baseline != 0)
        before_change_style_record = StyleRecordID { baseline };

    // A transition starts from the before-change style. The newly installed record may itself
    // have display: none; checking it would skip the discrete transition into that state.
    ComputedStyleRecordView before_change_style { scope.engine().publish_style_record(scope, before_change_style_record) };
    if (!before_change_style || before_change_style->in_display_none_subtree())
        return {};
    if (auto parent = abstract_element.element_to_inherit_style_from(); parent.has_value()) {
        if (auto parent_style = parent->computed_style(); parent_style && parent_style->in_display_none_subtree())
            return {};
        // A display:none change clears the styles of the subtree below it, while an SVG element
        // there can still install a record. The record the engine assigned the parent says whether
        // it is hidden, as the pass that decided the step read it.
        if (!parent->computed_style()) {
            auto parent_record = StyleEngineFFI::style_engine_assigned_style_record(scope.engine().rust_handle(), parent->element().style_node_id().value(), pseudo_element_to_ffi(parent->pseudo_element()));
            if (auto parent_style_record = scope.engine().publish_style_record(scope, StyleRecordID { parent_record }); parent_style_record && has_flag(parent_style_record->dependency_flags(), StyleRecordDependencyFlag::InDisplayNoneSubtree))
                return {};
        }
    }
    // OPTIMIZATION: The two lists `start_needed_transitions` decides over, plus this element's own
    //               provisional states. With none of them there is nothing to decide, and the
    //               after-change style need not be reconstructed at all.
    auto matching_property_ids = abstract_element.element().property_ids_with_matching_transition_property_entry(abstract_element.pseudo_element());
    auto existing_property_ids = abstract_element.element().property_ids_with_existing_transitions(abstract_element.pseudo_element());
    if (matching_property_ids.is_empty() && existing_property_ids.is_empty() && !has_provisional_transition_states(abstract_element))
        return {};
    // A step the pass decided names each property of the two lists. One that does not was decided
    // over state that moved before the row was installed, and the step is decided again here.
    auto const decided_names_each_property = !decided || transition_step_names_each_property(*decided, matching_property_ids, existing_property_ids);
    ASSERT(decided_names_each_property);
    if (!decided_names_each_property)
        decided = nullptr;

    begin_style_update();
    ScopeGuard end_style_update = [&] { this->end_style_update(); };
    auto const& installed_published_style_record = *abstract_element.published_style_record();
    auto new_style = reconstruct_computed_properties_for_animation(installed_published_style_record);
    // The installed record was sampled before the step, so its overlay holds the current values of
    // the element's running transitions and animations. A C++ computation collects the same effects
    // into its working set before the step, and a running transition's current value is read there.
    auto const* installed_overlay = static_cast<ComputedValuesFFI::AnimatedOverlay const*>(installed_published_style_record.view().animated_overlay);
    if (installed_overlay)
        new_style->install_animated_overlay_from_rust(Badge<StyleComputer> {}, ComputedValuesFFI::rust_animated_overlay_clone(installed_overlay));
    // The engine decides a step the pass did not, over the same two records, and publishes the
    // composition it leaves; the host applies the decisions as it applies the pass's.
    StyleEngineFFI::FfiRowSampledInPass engine_composition {};
    StyleEngineFFI::FfiTransitionStepDecidedInPass engine_decided {};
    if (!decided) {
        auto& element = abstract_element.element();
        engine_composition = StyleEngineFFI::style_engine_decide_transition_step_for_installed_record(scope.engine().rust_handle(),
            element.style_node_id().value(), pseudo_element_to_ffi(abstract_element.pseudo_element()), before_change_style_record.value(),
            installed_style_record.value(), document().layout_arena_handle());
        if (engine_composition.present) {
            engine_decided = abstract_element.pseudo_element().has_value()
                ? StyleEngineFFI::style_engine_take_pseudo_element_transition_step_decided_in_pass(scope.engine().rust_handle(), element.style_node_id().value(), pseudo_element_to_ffi(abstract_element.pseudo_element()))
                : StyleEngineFFI::style_engine_take_transition_step_decided_in_pass(scope.engine().rust_handle(), element.style_node_id().value());
            VERIFY(engine_decided.present);
            auto const engine_decided_names_each_property = transition_step_names_each_property(engine_decided, matching_property_ids, existing_property_ids);
            ASSERT(engine_decided_names_each_property);
            if (!engine_decided_names_each_property)
                engine_composition = {};
        }
    }
    start_needed_transitions(scope, *new_style, abstract_element, before_change_style_record, decided ? decided : engine_composition.present ? &engine_decided
                                                                                                                                             : nullptr);
    // Starting a transition associates a new animation with the element.
    abstract_element.element().publish_animation_timing_rows();
    // The row installed the composition the pass left for a step it decided.
    if (decided)
        return {};
    if (engine_composition.present && engine_composition.style_record == installed_style_record.value())
        return {};

    // The engine published the composition the step leaves of the installed record.
    StyleEngineFFI::FfiAnimationInvalidation animated_property_invalidation {};
    StyleRecordID new_style_record;
    if (engine_composition.present) {
        if (engine_composition.rebuilt_every_group)
            document().style_invalidation_counters().animated_style_full_builds++;
        else
            document().style_invalidation_counters().animated_style_overlay_builds++;
        animated_property_invalidation = engine_composition.invalidation;
        new_style_record = StyleRecordID { engine_composition.style_record };
    } else {
        // The host decided a step the engine could not; the engine samples the installed record
        // again over the effects the step left, which its timing rows now name.
        auto resampled = resample_installed_record_after_host_step(scope, abstract_element, installed_style_record);
        if (!resampled.present || resampled.style_record == installed_style_record.value())
            return {};
        animated_property_invalidation = resampled.invalidation;
        new_style_record = StyleRecordID { resampled.style_record };
    }
    auto& element = abstract_element.element();
    element.refresh_computed_style(scope, abstract_element.pseudo_element(), new_style_record);
    if (auto* svg_element = as_if<SVG::SVGElement>(element))
        svg_element->note_svg_paint_resource_description_may_have_changed();
    // Box-type, overflow and text-alignment adjustments consume the unadjusted base values, which
    // an animation-only overlay update deliberately does not reconstruct.
    if (animated_property_invalidation.requires_base_style_recomputation && !g_transition_step_follow_up_left_to_caller)
        scope.engine().record_derived_element_style_input_change(
            element.style_node_id(), StyleEngine::PublishedStyle | StyleEngine::RecomputeStyle);
    auto invalidation = decode_style_invalidation(animated_property_invalidation.invalidation);
    // The published values reach the element's pseudo-elements after sampling, when the style
    // transaction settles them from this composition. Descendants follow as one feedback batch.
    auto inherited_style_groups = invalidation.inherited_style_groups_changed();
    if (!invalidation.inherited_style_changed()) {
        auto child_explicit_inheritance_groups = element.children_explicitly_inherited_non_inherited_style_groups();
        if (auto shadow_root = element.shadow_root())
            child_explicit_inheritance_groups |= shadow_root->children_explicitly_inherited_non_inherited_style_groups();
        auto descendants_may_observe_non_inherited_properties = (child_explicit_inheritance_groups & animated_property_invalidation.changed_non_inherited_style_groups) != 0
            || invalidation.recompute_descendant_styles
            || invalidation.needs_layout_tree_rebuild()
            || element.is_html_slot_element();
        if (descendants_may_observe_non_inherited_properties)
            inherited_style_groups = RequiredInvalidationAfterStyleChange::all_inherited_style_groups;
    }
    if (!abstract_element.pseudo_element().has_value() && inherited_style_groups != 0 && !g_transition_step_follow_up_left_to_caller)
        scope.engine().record_flat_tree_descendant_style_input_changes(element.style_node_id(), StyleEngine::InheritedStyle, inherited_style_groups);
    // Refreshing the computed style published the record to the box; inherited values and image
    // resources need the C++ side effects on top.
    if (auto box = abstract_element.box()) {
        if (animated_property_invalidation.requires_layout_node_style_application)
            Layout::apply_style_to_box(box, *abstract_element.published_style_record());
        else if (animated_property_invalidation.requires_style_resource_update)
            Layout::attach_style_resources_to_box(box);
    }
    return invalidation;
}

// https://drafts.csswg.org/css-transitions/#starting
void StyleComputer::start_needed_transitions(StyleDrainScope const& scope, ComputedStyleWorkingSet& new_style, DOM::AbstractElement abstract_element, StyleRecordID before_change_style_record, StyleEngineFFI::FfiTransitionStepDecidedInPass const* decided) const
{
    auto had_pending_animated_style_update = m_document->needs_animated_style_update();

    // FIXME: Add some transition helpers to AbstractElement.
    auto& element = abstract_element.element();
    auto pseudo_element = abstract_element.pseudo_element();
    auto style_node_id = element.style_node_id();
    Optional<u64> transition_target_key;
    if (style_node_id != 0)
        transition_target_key = (static_cast<u64>(style_node_id.value()) << 8) | pseudo_element_to_ffi(pseudo_element);
    Vector<size_t> existing_stabilization_state_indices;
    if (transition_target_key.has_value()) {
        if (auto indices = m_provisional_transition_state_indices_by_target.get(*transition_target_key); indices.has_value())
            existing_stabilization_state_indices = *indices;
    } else {
        for (size_t index = 0; index < m_provisional_transition_states.size(); ++index) {
            auto const& state = m_provisional_transition_states[index];
            if (state.element == GC::Ptr { element } && state.pseudo_element == pseudo_element)
                existing_stabilization_state_indices.append(index);
        }
    }

    // NB: We know that a DocumentTimeline's current time is always in milliseconds
    auto current_time = m_document->timeline()->current_time();
    if (!current_time.has_value())
        return;
    VERIFY(current_time->type == Animations::TimeValue::Type::Milliseconds);
    auto style_change_event_time = current_time->value;

    // The after-change style's transition declarations, per longhand they name. A declaration
    // whose delay and duration are each the single value 0s starts nothing, so it is read only
    // when the element holds a transition such an entry could cancel.
    auto const* after_change_table = new_style.computed_longhand_table();
    auto existing_transition_property_ids = element.property_ids_with_existing_transitions(pseudo_element);
    StyleValueFFI::FfiTransitionEntries transition_entries {};
    if (!StyleValueFFI::rust_transition_delay_and_duration_are_single_zero(after_change_table) || !existing_transition_property_ids.is_empty())
        transition_entries = StyleValueFFI::rust_transition_entries(after_change_table);
    ScopeGuard release_transition_entries = [&] {
        if (transition_entries.storage)
            StyleValueFFI::rust_transition_entries_release(transition_entries.storage);
    };
    ReadonlySpan<StyleValueFFI::FfiTransitionEntry> matching_entries { transition_entries.entries, transition_entries.count };

    // OPTIMIZATION: The two lists below are what this decides over, and an element with neither
    //               starts nothing. Answering that first is worth doing because the after-change
    //               style is a whole computed style built for the comparison, and every recompute
    //               of every element that has a style at all reaches here.
    if (matching_entries.is_empty()
        && existing_transition_property_ids.is_empty()
        && existing_stabilization_state_indices.is_empty())
        return;

    StyleValueFFI::FfiAnimationContext transition_animation_context {
        .allow_discrete = false,
        .current_color = new_style.property(PropertyID::Color).rust_style_value_data(),
        .has_length_resolution_context = false,
        .length_resolution_context = {},
        .has_transform_reference_box = false,
        .transform_reference_box_width = 0,
        .transform_reference_box_height = 0,
    };
    // The lengths the transitions resolve against are those of the record the element installed.
    transition_animation_context.has_length_resolution_context = StyleValueFFI::rust_transition_length_resolution_context(
        m_style_engine.engine().rust_handle(), abstract_element.style_record_identity().value(), &transition_animation_context.length_resolution_context);
    apply_committed_transform_reference_box(scope, abstract_element, transition_animation_context);

    struct PreparedTransition {
        size_t stabilization_state_index;
        PropertyID property_id;
        RefPtr<StyleValue const> before_change_value;
        RefPtr<StyleValue const> after_change_value;
        RefPtr<StyleValue const> current_value;
        GC::Ptr<CSSTransition> existing_transition;
        StyleValueFFI::StyleValueData const* timing_function;
    };
    Vector<PreparedTransition> prepared_transitions;
    Vector<StyleValueFFI::FfiTransitionPropertyInput> ffi_properties;

    auto ensure_stabilization_state = [&](PropertyID property_id) -> size_t {
        Optional<u64> state_key;
        if (transition_target_key.has_value()) {
            auto property = to_underlying(property_id);
            VERIFY(property <= NumericLimits<u16>::max());
            state_key = (*transition_target_key << 16) | property;
            if (auto index = m_provisional_transition_state_indices.get(*state_key); index.has_value())
                return *index;
        } else {
            for (size_t index = 0; index < m_provisional_transition_states.size(); ++index) {
                auto const& state = m_provisional_transition_states[index];
                if (state.element == GC::Ptr { element } && state.pseudo_element == pseudo_element && state.property_id == property_id)
                    return index;
            }
        }
        VERIFY(document().is_in_style_stabilization_epoch());
        auto existing_transition = element.property_transition(pseudo_element, property_id);
        m_provisional_transition_states.append({
            .element = element,
            .pseudo_element = pseudo_element,
            .property_id = property_id,
            .committed_transition = existing_transition,
            .proposed_transition = nullptr,
            .action = ProvisionalTransitionAction::None,
            .has_decision = false,
        });
        auto index = m_provisional_transition_states.size() - 1;
        if (state_key.has_value()) {
            m_provisional_transition_state_indices.set(*state_key, index);
            m_provisional_transition_state_indices_by_target.ensure(*transition_target_key).append(index);
        }
        return index;
    };
    auto append_transition_input = [&](PropertyID property_id, StyleValueFFI::FfiTransitionEntry const* matching_entry) {
        auto stabilization_state_index = ensure_stabilization_state(property_id);
        auto const& stabilization_state = m_provisional_transition_states[stabilization_state_index];
        auto existing_transition = stabilization_state.committed_transition;
        bool has_running_transition = existing_transition && !existing_transition->is_finished() && !existing_transition->is_idle();
        bool has_completed_transition = existing_transition && !has_running_transition;
        bool allow_discrete = false;
        double delay = 0;
        double duration = 0;
        double old_timing_function_output = 0;
        double old_reversing_shortening_factor = 1;

        if (matching_entry) {
            delay = matching_entry->delay;
            duration = matching_entry->duration;
            allow_discrete = static_cast<TransitionBehavior>(matching_entry->behavior) == TransitionBehavior::AllowDiscrete;
            if (existing_transition) {
                old_reversing_shortening_factor = existing_transition->reversing_shortening_factor();
                if (has_running_transition)
                    old_timing_function_output = existing_transition->timing_function_output_at_time(style_change_event_time);
            }
        }

        ffi_properties.append({
            .property_id = to_underlying(property_id),
            .before_change_value = nullptr,
            .after_change_value = nullptr,
            .current_value = nullptr,
            .existing_end_value = existing_transition ? existing_transition->transition_end_value()->rust_style_value_data() : nullptr,
            .reversing_adjusted_start_value = existing_transition ? existing_transition->reversing_adjusted_start_value()->rust_style_value_data() : nullptr,
            .has_matching_transition = matching_entry != nullptr,
            .allow_discrete = allow_discrete,
            .has_running_transition = has_running_transition,
            .has_completed_transition = has_completed_transition,
            .delay = delay,
            .duration = duration,
            .old_timing_function_output = old_timing_function_output,
            .old_reversing_shortening_factor = old_reversing_shortening_factor,
        });
        prepared_transitions.append({
            .stabilization_state_index = stabilization_state_index,
            .property_id = property_id,
            .before_change_value = {},
            .after_change_value = {},
            .current_value = {},
            .existing_transition = existing_transition,
            .timing_function = matching_entry ? matching_entry->timing_function : nullptr,
        });
    };

    // OPTIMIZATION: Instead of iterating over all properties we collect properties which appear in
    //               transition-property, followed by existing transitions without a matching entry.
    for (auto const& entry : matching_entries)
        append_transition_input(static_cast<PropertyID>(entry.property_id), &entry);
    for (auto property_id : existing_transition_property_ids) {
        if (!any_of(matching_entries, [&](auto const& entry) { return entry.property_id == to_underlying(property_id); }))
            append_transition_input(property_id, nullptr);
    }

    for (auto stabilization_state_index : existing_stabilization_state_indices) {
        auto& stabilization_state = m_provisional_transition_states[stabilization_state_index];
        bool has_prepared_transition = false;
        for (auto const& prepared_transition : prepared_transitions) {
            if (prepared_transition.stabilization_state_index == stabilization_state_index) {
                has_prepared_transition = true;
                break;
            }
        }
        if (has_prepared_transition)
            continue;
        ++document().style_invalidation_counters().provisional_transition_decisions;
        if (stabilization_state.has_decision)
            ++document().style_invalidation_counters().superseded_provisional_transition_decisions;
        stabilization_state.has_decision = true;
        if (stabilization_state.proposed_transition)
            stabilization_state.proposed_transition->discard_provisional_transition();
        stabilization_state.proposed_transition = nullptr;
        stabilization_state.action = ProvisionalTransitionAction::None;
    }

    StyleValueFFI::FfiTransitionInput input {
        .context = transition_animation_context,
        .properties = ffi_properties.data(),
        .property_count = ffi_properties.size(),
        .target_key = transition_target_key.value_or(0),
    };
    Vector<StyleValueFFI::FfiTransitionAction> actions;
    actions.resize(prepared_transitions.size());
    if (decided) {
        // The pass decided the step over the same styles and transitions, and handed over what each
        // transition it starts runs from and to.
        ReadonlySpan<StyleEngineFFI::FfiTransitionStepAction> decided_actions { decided->actions, decided->action_count };
        ASSERT(decided_actions.size() == ffi_properties.size());
        // A property takes the last action the pass names it in. A `transition: all` names every
        // longhand, so the actions are looked up by property, not scanned for each.
        HashMap<u16, StyleEngineFFI::FfiTransitionStepAction const*> decided_action_by_property;
        decided_action_by_property.ensure_capacity(decided_actions.size());
        for (auto const& action : decided_actions)
            decided_action_by_property.set(action.property_id, &action);
        for (size_t index = 0; index < ffi_properties.size(); ++index) {
            auto& property = ffi_properties[index];
            auto const* decided_action = decided_action_by_property.get(property.property_id).value_or(nullptr);
            ASSERT(decided_action);
            auto kind = static_cast<StyleValueFFI::FfiTransitionActionKind>(decided_action->kind);
            actions[index] = {
                .property_id = decided_action->property_id,
                .kind = kind,
                .delay = decided_action->delay,
                .active_duration = decided_action->active_duration,
                .reversing_shortening_factor = decided_action->reversing_shortening_factor,
            };
            auto const* start_value = static_cast<StyleValueFFI::StyleValueData const*>(decided_action->start_value);
            property.after_change_value = static_cast<StyleValueFFI::StyleValueData const*>(decided_action->end_value);
            if (kind == StyleValueFFI::FfiTransitionActionKind::CancelRemoveAndStartReversing || kind == StyleValueFFI::FfiTransitionActionKind::CancelRemoveAndStartInterrupted)
                property.current_value = start_value;
            else
                property.before_change_value = start_value;
        }
    } else {
        m_style_engine.engine().decide_transitions(
            before_change_style_record,
            new_style.computed_longhand_table(),
            new_style.animated_overlay(Badge<StyleComputer> {}),
            input,
            actions.data());
    }
    auto retain_style_value = [](StyleValueFFI::StyleValueData const* value) -> RefPtr<StyleValue const> {
        if (!value)
            return {};
        return StyleValue::adopt_rust_style_value_data(StyleValueFFI::rust_style_value_retain(value));
    };
    for (size_t index = 0; index < prepared_transitions.size(); ++index) {
        auto& prepared_transition = prepared_transitions[index];
        auto const& property = ffi_properties[index];
        switch (actions[index].kind) {
        case StyleValueFFI::FfiTransitionActionKind::None:
        case StyleValueFFI::FfiTransitionActionKind::Remove:
        case StyleValueFFI::FfiTransitionActionKind::Cancel:
            break;
        case StyleValueFFI::FfiTransitionActionKind::Start:
        case StyleValueFFI::FfiTransitionActionKind::RemoveAndStart:
            prepared_transition.before_change_value = retain_style_value(property.before_change_value);
            prepared_transition.after_change_value = retain_style_value(property.after_change_value);
            break;
        case StyleValueFFI::FfiTransitionActionKind::CancelRemoveAndStartReversing:
        case StyleValueFFI::FfiTransitionActionKind::CancelRemoveAndStartInterrupted:
            prepared_transition.after_change_value = retain_style_value(property.after_change_value);
            prepared_transition.current_value = retain_style_value(property.current_value);
            break;
        }
    }

    Vector<GC::Ref<Animations::KeyframeEffect>> newly_started_transition_effects;
    for (size_t index = 0; index < prepared_transitions.size(); ++index) {
        auto const& prepared_transition = prepared_transitions[index];
        auto& stabilization_state = m_provisional_transition_states[prepared_transition.stabilization_state_index];
        auto property_id = prepared_transition.property_id;
        auto const& action = actions[index];
        VERIFY(action.property_id == to_underlying(property_id));
        auto existing_transition = prepared_transition.existing_transition;
        ++document().style_invalidation_counters().provisional_transition_decisions;
        if (stabilization_state.has_decision)
            ++document().style_invalidation_counters().superseded_provisional_transition_decisions;
        stabilization_state.has_decision = true;
        if (stabilization_state.proposed_transition)
            stabilization_state.proposed_transition->discard_provisional_transition();
        stabilization_state.proposed_transition = nullptr;
        auto start_a_transition = [&](StyleValue const& start_value, StyleValue const& end_value, StyleValue const& reversing_adjusted_start_value) {
            dbgln_if(CSS_TRANSITIONS_DEBUG, "Proposing a transition of {} from {} to {}", string_from_property_id(property_id), start_value.to_string(SerializationMode::Normal), end_value.to_string(SerializationMode::Normal));
            auto start_time = style_change_event_time;
            auto end_time = start_time + action.active_duration;
            auto timing_function = EasingFunction::from_style_value(StyleValue::adopt_rust_style_value_data(StyleValueFFI::rust_style_value_retain(prepared_transition.timing_function)));
            auto transition = CSSTransition::start_a_transition(abstract_element, property_id,
                document().transition_generation(), action.delay, start_time, end_time, start_value, end_value, reversing_adjusted_start_value, action.reversing_shortening_factor, move(timing_function), CSSTransition::Publication::Provisional);
            stabilization_state.proposed_transition = transition;
            newly_started_transition_effects.append(as<Animations::KeyframeEffect>(*transition->effect()));
        };

        switch (action.kind) {
        case StyleValueFFI::FfiTransitionActionKind::None:
            stabilization_state.action = ProvisionalTransitionAction::None;
            break;
        case StyleValueFFI::FfiTransitionActionKind::Remove:
            stabilization_state.action = ProvisionalTransitionAction::Remove;
            break;
        case StyleValueFFI::FfiTransitionActionKind::Cancel:
            VERIFY(existing_transition);
            stabilization_state.action = ProvisionalTransitionAction::Cancel;
            break;
        case StyleValueFFI::FfiTransitionActionKind::Start:
            stabilization_state.action = ProvisionalTransitionAction::Start;
            start_a_transition(*prepared_transition.before_change_value, *prepared_transition.after_change_value, *prepared_transition.before_change_value);
            break;
        case StyleValueFFI::FfiTransitionActionKind::RemoveAndStart:
            stabilization_state.action = ProvisionalTransitionAction::RemoveAndStart;
            start_a_transition(*prepared_transition.before_change_value, *prepared_transition.after_change_value, *prepared_transition.before_change_value);
            break;
        case StyleValueFFI::FfiTransitionActionKind::CancelRemoveAndStartReversing: {
            VERIFY(existing_transition);
            auto reversing_adjusted_start_value = existing_transition->transition_end_value();
            stabilization_state.action = ProvisionalTransitionAction::CancelRemoveAndStart;
            start_a_transition(*prepared_transition.current_value, *prepared_transition.after_change_value, *reversing_adjusted_start_value);
            break;
        }
        case StyleValueFFI::FfiTransitionActionKind::CancelRemoveAndStartInterrupted:
            stabilization_state.action = ProvisionalTransitionAction::CancelRemoveAndStart;
            start_a_transition(*prepared_transition.current_value, *prepared_transition.after_change_value, *prepared_transition.current_value);
            break;
        }
    }

    // The engine composes what the step starts and removes into the element's composition. The
    // transitions it started are provisional, so nothing has published their timing yet.
    if (!newly_started_transition_effects.is_empty()) {
        abstract_element.element().publish_animation_timing_rows();
        // NB: Construction does not invalidate animated style because the effects were just evaluated. Request the
        //     first animation frame directly so timeline updates can schedule subsequent animated style updates.
        m_document->page().client().request_frame();
        if (!had_pending_animated_style_update)
            m_document->clear_needs_animated_style_update();
    }
}

bool StyleComputer::has_provisional_transition_states(DOM::AbstractElement abstract_element) const
{
    auto& element = abstract_element.element();
    auto pseudo_element = abstract_element.pseudo_element();
    if (auto style_node_id = element.style_node_id(); style_node_id != 0) {
        auto transition_target_key = (static_cast<u64>(style_node_id.value()) << 8) | pseudo_element_to_ffi(pseudo_element);
        return m_provisional_transition_state_indices_by_target.contains(transition_target_key);
    }
    return any_of(m_provisional_transition_states, [&](auto const& state) {
        return state.element == GC::Ptr { element } && state.pseudo_element == pseudo_element;
    });
}

// What an evaluation of an element's container conditions read of its containers, recorded for the
// commit: the containers it asked about, the facts that re-evaluate it after layout, and that the
// element's style depends on its containers, which bounds the scan that re-styles it when they move.
void StyleComputer::record_container_query_effects(StyleDrainScope const& scope, DOM::AbstractElement abstract_element, StyleEngineFFI::FfiNativeContainerMatchResult const& match_result)
{
    auto effect_count = StyleEngineFFI::style_engine_native_container_effect_count(scope, match_result.effects);
    for (size_t effect_index = 0; effect_index < effect_count; ++effect_index) {
        auto effect = StyleEngineFFI::style_engine_native_container_effect(scope, match_result.effects, effect_index);
        auto identity = DOM::NodeIdentity::of_style_node(StyleNodeID { effect.style_node });
        switch (effect.kind) {
        case StyleEngineFFI::FfiContainerEffectKind::SizeContainerUsage:
            abstract_element.document().commit_messages().note_style_query_container_usage(identity, 1);
            break;
        case StyleEngineFFI::FfiContainerEffectKind::StyleContainerUsage:
            abstract_element.document().commit_messages().note_style_query_container_usage(identity, 2);
            break;
        case StyleEngineFFI::FfiContainerEffectKind::ScrollStateContainerUsage:
            abstract_element.document().commit_messages().note_scroll_state_query_container_usage(identity);
            break;
        case StyleEngineFFI::FfiContainerEffectKind::NeedsEvaluationAfterLayout:
            // The engine recorded the container as it handed the effects over.
            break;
        case StyleEngineFFI::FfiContainerEffectKind::SubjectViewportDependency:
            abstract_element.document().commit_messages().note_style_viewport_dependency(identity);
            break;
        case StyleEngineFFI::FfiContainerEffectKind::CustomPropertyReference:
            abstract_element.document().commit_messages().note_style_query_custom_property_reference(
                identity, abstract_element.pseudo_element(), Utf16FlyString::from_utf16({ reinterpret_cast<char16_t const*>(effect.name), effect.name_length }));
            break;
        }
    }
    u8 dependencies = (match_result.depends_on_size ? 1 : 0) | (match_result.depends_on_style ? 2 : 0);
    if (dependencies)
        abstract_element.document().commit_messages().note_style_container_query_dependencies(DOM::NodeIdentity::of(abstract_element.element()), dependencies);
}

void StyleComputer::register_style_engine_sheet_source(StyleSheetState const& sheet)
{
    auto identity = Parser::ValueParserFFI::rust_style_sheet_identity(sheet.native_sheet().handle());
    m_style_engine_sheet_sources.set(identity, sheet.make_weak_ptr<StyleSheetState const>());
}

Optional<StyleEngineRuleTarget> StyleComputer::style_engine_rule_target(StyleEngineRuleID rule_id) const
{
    StyleEngineFFI::FfiNativeRuleTarget target {};
    if (!StyleEngineFFI::style_engine_native_rule_target(m_style_engine.engine().rust_handle(), rule_id.value(), &target))
        return {};
    RustDeclarationBlockSnapshot declaration { static_cast<Parser::ValueParserFFI::DeclarationBlockData const*>(target.declarations) };
    auto source = m_style_engine_sheet_sources.get(target.source_identity);
    if (!source.has_value())
        return {};
    RefPtr<StyleSheetState const> source_sheet = *source;
    if (!source_sheet)
        return {};
    auto layer_name = Utf16FlyString::from_utf16({ reinterpret_cast<char16_t const*>(target.layer_name), target.layer_name_length });
    return StyleEngineRuleTarget {
        .rule_identity = target.identity,
        .declaration_version = target.declaration_version,
        .declaration = move(declaration),
        .source_style_sheet = move(source_sheet),
        .has_container_conditions = target.has_container_conditions,
        .qualified_layer_name = move(layer_name),
        .cascade_origin = static_cast<CascadeOrigin>(target.origin),
    };
}

StyleEngineRuleID StyleComputer::style_engine_rule_id_for(RustRule const& rule) const
{
    return StyleEngineRuleID { StyleEngineFFI::style_engine_native_rule_id(m_style_engine.engine().rust_handle(), rule.identity()) };
}

SheetID StyleComputer::style_engine_sheet_id_for(StyleSheetState const& sheet) const
{
    if (sheet.constructed())
        return m_constructed_sheet_ids.get(&sheet).value_or(0);
    return sheet.style_engine_sheet_id();
}

void StyleComputer::set_style_engine_sheet_id_for(StyleSheetState& sheet, SheetID sheet_id)
{
    if (sheet.constructed())
        m_constructed_sheet_ids.set(&sheet, sheet_id);
    else
        sheet.set_style_engine_sheet_id(sheet_id);
}

static bool custom_property_inherits(DOM::Document const& document, Utf16FlyString const& name)
{
    // A custom property inherits unless it has been registered with an explicit `inherits: false`.
    auto registration = document.get_registered_custom_property(name);
    return !registration.has_value() || registration->inherit;
}

enum class IsCustomProperty : u8 {
    No,
    Yes,
};

enum class Inherits : u8 {
    No,
    Yes,
};

enum class NameIsValid : u8 {
    No,
    Yes,
};

enum class IsValid : u8 {
    No,
    Yes,
};

static JsonObject serialize_devtools_style_declaration(
    String name,
    String value,
    Important important,
    IsCustomProperty is_custom_property,
    Inherits inherits,
    NameIsValid is_name_valid,
    IsValid is_valid)
{
    JsonObject serialized_property;
    serialized_property.set("name"sv, move(name));
    serialized_property.set("value"sv, move(value));
    serialized_property.set("priority"sv, important == Important::Yes ? "important"sv : ""sv);
    serialized_property.set("isCustomProperty"sv, is_custom_property == IsCustomProperty::Yes);
    serialized_property.set("inherits"sv, inherits == Inherits::Yes);
    serialized_property.set("isNameValid"sv, is_name_valid == NameIsValid::Yes);
    serialized_property.set("isValid"sv, is_valid == IsValid::Yes);
    return serialized_property;
}

static JsonArray serialize_devtools_style_declarations(DOM::Document const& document, RustDeclarationBlock const& declaration)
{
    JsonArray declarations;

    auto serialize_property = [&](Utf16FlyString const& name, StyleProperty const& property, IsCustomProperty is_custom_property, Inherits inherits) {
        declarations.must_append(serialize_devtools_style_declaration(
            name.to_utf16_string().to_utf8_but_should_be_ported_to_utf16(),
            property.value->to_string(SerializationMode::Normal),
            property.important,
            is_custom_property,
            inherits,
            NameIsValid::Yes,
            IsValid::Yes));
    };

    for (auto const& property : declaration.properties()) {
        serialize_property(
            string_from_property_id(property.property_id),
            property,
            IsCustomProperty::No,
            is_inherited_property(property.property_id) ? Inherits::Yes : Inherits::No);
    }

    for (auto const& custom_property : declaration.custom_properties())
        serialize_property(
            custom_property.key,
            custom_property.value,
            IsCustomProperty::Yes,
            custom_property_inherits(document, custom_property.key) ? Inherits::Yes : Inherits::No);

    return declarations;
}

static JsonArray serialize_devtools_style_declarations(DOM::Document const& document, Vector<Parser::DevToolsStyleDeclaration> const& declarations)
{
    JsonArray serialized_declarations;

    for (auto const& declaration : declarations) {
        bool inherits = declaration.is_custom_property
            ? custom_property_inherits(document, declaration.name)
            : PropertyNameAndID::from_name(declaration.name)
                  .map([](auto const& property) { return !property.is_custom_property() && is_inherited_property(property.id()); })
                  .value_or(false);

        serialized_declarations.must_append(serialize_devtools_style_declaration(
            MUST(declaration.name.view().to_utf8()),
            declaration.value.to_utf8(),
            declaration.important,
            declaration.is_custom_property ? IsCustomProperty::Yes : IsCustomProperty::No,
            inherits ? Inherits::Yes : Inherits::No,
            declaration.is_name_valid ? NameIsValid::Yes : NameIsValid::No,
            declaration.is_valid ? IsValid::Yes : IsValid::No));
    }

    return serialized_declarations;
}

static Vector<Parser::DevToolsStyleDeclaration> parse_devtools_style_declarations(DOM::Document const& document, StringView declaration_block)
{
    return Parser::parse_css_declaration_block_for_devtools(Parser::ParsingParams(document), declaration_block);
}

static Vector<Parser::DevToolsStyleDeclaration> parse_devtools_style_declarations(DOM::Document const& document, Utf16View declaration_block)
{
    return Parser::parse_css_declaration_block_for_devtools(Parser::ParsingParams(document), declaration_block);
}

static Optional<size_t> source_offset_for_line_and_column(StringView source, SourcePosition const& position)
{
    size_t line = 0;
    size_t column = 0;

    Utf8View source_code_points { source };
    for (auto it = source_code_points.begin(); it != source_code_points.end();) {
        auto offset = source_code_points.byte_offset_of(it);
        if (line == position.line && column == position.column)
            return offset;

        auto code_point = *it;
        ++it;

        if (code_point == '\r') {
            if (offset + 1 < source.length() && source[offset + 1] == '\n')
                ++it;
            ++line;
            column = 0;
        } else if (code_point == '\n' || code_point == '\f') {
            ++line;
            column = 0;
        } else {
            ++column;
        }
    }

    if (line == position.line && column == position.column)
        return source.length();

    return {};
}

static Optional<String> extract_css_declaration_block_from_source(CSSRule const& rule)
{
    if (rule.type() != CSSRule::Type::Style)
        return {};

    auto const* style_sheet = rule.parent_style_sheet();
    if (!style_sheet)
        return {};

    auto const source_text = style_sheet->source_text();
    if (!source_text.has_value())
        return {};

    auto source = source_text->to_utf8();
    auto source_view = source.bytes_as_string_view();
    auto const& source_location = rule.source_location();
    if (!source_location.has_value())
        return {};

    auto maybe_offset = source_offset_for_line_and_column(source_view, *source_location);
    if (!maybe_offset.has_value())
        return {};

    Optional<u8> string_quote;
    bool in_comment = false;
    bool escaped = false;
    Optional<size_t> block_start;
    size_t block_depth = 0;

    for (size_t offset = *maybe_offset; offset < source_view.length(); ++offset) {
        auto ch = source_view[offset];
        auto next_ch = offset + 1 < source_view.length() ? source_view[offset + 1] : '\0';

        if (in_comment) {
            if (ch == '*' && next_ch == '/') {
                in_comment = false;
                ++offset;
            }
            continue;
        }

        if (string_quote.has_value()) {
            if (escaped) {
                escaped = false;
                continue;
            }
            if (ch == '\\') {
                escaped = true;
                continue;
            }
            if (ch == *string_quote)
                string_quote = {};
            continue;
        }

        if (ch == '/' && next_ch == '*') {
            in_comment = true;
            ++offset;
            continue;
        }

        if (ch == '"' || ch == '\'') {
            string_quote = ch;
            continue;
        }

        if (ch == '{') {
            if (!block_start.has_value())
                block_start = offset + 1;
            ++block_depth;
            continue;
        }

        if (ch == '}' && block_start.has_value()) {
            VERIFY(block_depth > 0);
            --block_depth;
            if (block_depth == 0)
                return MUST(String::from_utf8(source_view.substring_view(*block_start, offset - *block_start)));
        }
    }

    return {};
}

static bool has_inherited_declaration(DOM::Document const& document, ReadonlySpan<StyleProperty> properties, OrderedHashMap<Utf16FlyString, StyleProperty> const& custom_properties)
{
    if (any_of(properties, [](auto const& property) {
            return CSS::is_inherited_property(property.property_id);
        })) {
        return true;
    }

    return any_of(custom_properties, [&](auto const& custom_property) {
        return custom_property_inherits(document, custom_property.key);
    });
}

static JsonObject serialize_devtools_style_sheet_identifier(StyleSheetIdentifier const& identifier)
{
    JsonObject serialized_identifier;
    serialized_identifier.set("type"sv, style_sheet_identifier_type_to_string(identifier.type));
    if (identifier.dom_element_unique_id.has_value())
        serialized_identifier.set("domElementUniqueId"sv, identifier.dom_element_unique_id->value());
    if (identifier.url.has_value())
        serialized_identifier.set("url"sv, identifier.url->to_utf8());
    serialized_identifier.set("ruleCount"sv, identifier.rule_count);
    return serialized_identifier;
}

// What DevTools shows for one rule the engine says decides for this element. Which of the rule's
// selectors matched is asked through the ad-hoc engine query path because the engine reports the
// rule rather than the entry, and a panel can afford to ask a question a style pass cannot.
static JsonObject serialize_devtools_applied_rule(DOM::Document& document, CSSRule const& rule, DOM::AbstractElement const& element)
{
    auto const& declaration = declaration_of_rule(rule);
    auto authored_text = extract_css_declaration_block_from_source(rule);
    SelectorList const* selector_list = nullptr;
    if (auto const* style_rule = as_if<CSSStyleRule>(rule))
        selector_list = &style_rule->absolutized_selectors();
    else if (auto const* nested = as_if<CSSNestedDeclarations>(rule))
        selector_list = &nested->absolutized_selectors();
    SelectorList const empty_selectors;
    auto const& selectors = selector_list ? *selector_list : empty_selectors;

    JsonArray serialized_selectors;
    JsonArray specificities;
    JsonArray matched_selector_indexes;
    for (size_t index = 0; index < selectors.size(); ++index) {
        auto const& selector = selectors[index];
        serialized_selectors.must_append(selector->serialize().to_utf8());
        specificities.must_append(selector->specificity());
        SelectorList selector_query_list;
        selector_query_list.append(selector);
        auto selector_query = DOM::SelectorQuery::create(move(selector_query_list));
        if (selector_query->matches(element.element(), document))
            matched_selector_indexes.must_append(index);
    }

    JsonObject serialized_rule;
    serialized_rule.set("type"sv, to_underlying(rule.type()));
    serialized_rule.set("className"sv, rule.type() == CSSRule::Type::Style ? "CSSStyleRule"sv : "CSSNestedDeclarations"sv);
    serialized_rule.set("selectors"sv, move(serialized_selectors));
    serialized_rule.set("selectorsSpecificity"sv, move(specificities));
    serialized_rule.set("matchedSelectorIndexes"sv, move(matched_selector_indexes));
    serialized_rule.set("cssText"sv, rule.serialized().to_utf8());
    if (authored_text.has_value()) {
        serialized_rule.set("authoredText"sv, *authored_text);
        serialized_rule.set("declarations"sv, serialize_devtools_style_declarations(document, parse_devtools_style_declarations(document, authored_text->bytes_as_string_view())));
    } else {
        auto style = rule.type() == CSSRule::Type::Style
            ? static_cast<CSSStyleRule const&>(rule).style()
            : static_cast<CSSNestedDeclarations const&>(rule).style();
        serialized_rule.set("authoredText"sv, style->serialized().to_utf8());
        serialized_rule.set("declarations"sv, serialize_devtools_style_declarations(document, declaration));
    }
    if (auto* sheet = rule.parent_style_sheet()) {
        if (auto identifier = style_sheet_identifier_for(*sheet); identifier.has_value())
            serialized_rule.set("ruleId"sv, serialize_devtools_style_sheet_identifier(*identifier));
    }
    return serialized_rule;
}

static JsonObject serialize_devtools_inline_style(DOM::Document const& document, DOM::AbstractElement abstract_element, CSSStyleProperties const& declaration)
{
    auto authored_text = abstract_element.element().get_attribute(HTML::AttributeNames::style);

    JsonObject serialized_rule;
    serialized_rule.set("type"sv, 100);
    serialized_rule.set("className"sv, 100);
    serialized_rule.set("cssText"sv, declaration.serialized().to_utf8());
    if (authored_text.has_value()) {
        auto authored_text_utf8 = authored_text->to_utf8();
        serialized_rule.set("authoredText"sv, authored_text_utf8);
        serialized_rule.set("declarations"sv, serialize_devtools_style_declarations(document, parse_devtools_style_declarations(document, authored_text->utf16_view())));
    } else {
        serialized_rule.set("authoredText"sv, declaration.serialized().to_utf8());
        serialized_rule.set("declarations"sv, serialize_devtools_style_declarations(document, declaration.declaration_block()));
    }
    serialized_rule.set("isSystem"sv, false);
    serialized_rule.set("nodeId"sv, abstract_element.element().unique_id().value());
    return serialized_rule;
}

static void append_devtools_applied_style_entry(JsonArray& entries, JsonObject rule, Optional<UniqueNodeID> inherited_node_id = {})
{
    JsonObject entry;

    JsonValue matched_selector_indexes { JsonArray {} };
    if (auto value = rule.get("matchedSelectorIndexes"sv); value.has_value())
        matched_selector_indexes = *value;
    rule.remove("matchedSelectorIndexes"sv);
    auto is_system = rule.get_bool("isSystem"sv).value_or(false);

    entry.set("rule"sv, move(rule));
    entry.set("isSystem"sv, is_system);
    entry.set("matchedSelectorIndexes"sv, move(matched_selector_indexes));
    if (inherited_node_id.has_value())
        entry.set("inheritedNodeId"sv, inherited_node_id->value());
    else
        entry.set("inherited"sv, JsonValue {});

    entries.must_append(move(entry));
}

JsonArray StyleComputer::collect_devtools_applied_style_rules(DOM::AbstractElement abstract_element, bool include_inherited, bool include_user_agent_styles)
{
    JsonArray entries;

    auto append_rules_for_abstract_element = [&](DOM::AbstractElement current_element, Optional<UniqueNodeID> inherited_node_id) {
        if (auto inline_style = current_element.inline_style()) {
            if (!inherited_node_id.has_value() || has_inherited_declaration(m_document, inline_style->properties(), inline_style->custom_properties()))
                append_devtools_applied_style_entry(entries, serialize_devtools_inline_style(m_document, current_element, *inline_style), inherited_node_id);
        }

        auto node = current_element.element().style_node_id();
        if (node == 0)
            return;
        Vector<StyleEngine::RuleMatch> matches;
        if (!style_engine_queries().match_element(node, matches, StyleEngine::MatchPurpose::Exact))
            return;

        // The engine reports the rules in the order the cascade applies them, and the panel lists
        // the winning one first.
        for (auto const& match : matches.in_reverse()) {
            if (match.pseudo_element != NumericLimits<u32>::max())
                continue;
            auto target = style_engine_rule_target(StyleEngineRuleID { match.rule });
            if (!target.has_value() || !target->source_style_sheet)
                continue;
            if (target->cascade_origin == CascadeOrigin::UserAgent && !include_user_agent_styles)
                continue;
            if (inherited_node_id.has_value()) {
                struct Context {
                    GC::Ref<DOM::Document const> document;
                    bool has_inherited_declaration { false };
                } context { m_document };
                Parser::ValueParserFFI::rust_declaration_data_visit(target->declaration.data(), &context, [](void* raw_context, Parser::ValueParserFFI::FfiDeclaredProperty const* property) {
                    auto& context = *static_cast<Context*>(raw_context);
                    if (context.has_inherited_declaration)
                        return;
                    if (property->name.length == 0)
                        context.has_inherited_declaration = is_inherited_property(static_cast<PropertyID>(property->property_id));
                    else
                        context.has_inherited_declaration = custom_property_inherits(context.document, Utf16FlyString::from_utf16({ reinterpret_cast<char16_t const*>(property->name.utf16), property->name.length }));
                });
                if (!context.has_inherited_declaration)
                    continue;
            }
            auto* rule = target->source_style_sheet->rules().rule_for_identity(target->rule_identity);
            if (!rule)
                continue;
            append_devtools_applied_style_entry(entries, serialize_devtools_applied_rule(m_document, *rule, current_element), inherited_node_id);
        }
    };

    append_rules_for_abstract_element(abstract_element, {});

    if (!include_inherited)
        return entries;

    for (auto current_element = abstract_element.element_to_inherit_style_from(); current_element.has_value(); current_element = current_element->element_to_inherit_style_from())
        append_rules_for_abstract_element(*current_element, current_element->element().unique_id());

    return entries;
}

Vector<StyleProperty> StyleComputer::collect_presentational_hint_properties(DOM::AbstractElement abstract_element)
{
    Vector<StyleProperty> properties;
    if (abstract_element.pseudo_element().has_value())
        return properties;

    auto& element = abstract_element.element();
    element.apply_presentational_hints(properties);
    if (element.supports_dimension_attributes()) {
        auto const& dimension_source = is<HTML::HTMLImageElement>(element)
            ? static_cast<HTML::HTMLImageElement const&>(element).dimension_attribute_source()
            : element;
        collect_dimension_attribute(properties, dimension_source, HTML::AttributeNames::width, CSS::PropertyID::Width);
        collect_dimension_attribute(properties, dimension_source, HTML::AttributeNames::height, CSS::PropertyID::Height);
    }
    HashTable<CSS::PropertyID> seen_properties;
    for (size_t i = properties.size(); i > 0; --i) {
        if (seen_properties.set(properties[i - 1].property_id) != AK::HashSetResult::InsertedNewEntry)
            properties.remove(i - 1);
    }
    // Which properties a hint decides is a fact about the element, and this is the one place that
    // knows it: mapping the attributes needs the element fully built, and for a table cell it needs
    // the table's computed style, so it cannot be done when the element arrives.
    if (element.presentational_hint_properties_need_publication(properties)
        && record_element_presentational_hint_properties(element, properties))
        element.did_publish_presentational_hint_properties(properties);
    return properties;
}

RefPtr<CustomPropertyData const> StyleComputer::engine_custom_property_environment(u64 identity, bool* did_materialize) const
{
    if (!StyleEngine::is_engine_custom_property_environment(identity))
        return {};
    if (auto existing = m_engine_custom_property_environments.get(identity); existing.has_value())
        return *existing;
    if (did_materialize)
        *did_materialize = true;
    u64 parent_identity = 0;
    auto const* store = m_style_engine.engine().borrow_engine_custom_property_environment(identity, parent_identity);
    if (!store)
        return {};
    // What an element's or a pseudo-element's animations composed its custom properties into is its
    // animation overlay, over the environment its record was resolved over.
    u32 owner_style_node = 0;
    u8 owner_pseudo_kind = 0;
    if (StyleEngineFFI::style_engine_sampled_custom_property_environment_owner(m_style_engine.engine().rust_handle(), identity, &owner_style_node, &owner_pseudo_kind)) {
        auto index = style_node_index(StyleNodeID { owner_style_node });
        if (auto* owner = index < m_element_style_nodes.size() ? as_if<DOM::Element>(m_element_style_nodes[index].ptr()) : nullptr) {
            Optional<PseudoElement> owner_pseudo_element;
            if (owner_pseudo_kind != NumericLimits<u8>::max())
                owner_pseudo_element = static_cast<PseudoElement>(owner_pseudo_kind);
            auto data = CustomPropertyData::view_animation_overlay(store, identity, engine_custom_property_environment(parent_identity),
                DOM::AbstractElement { *owner, owner_pseudo_element });
            ComputedValuesFFI::rust_custom_property_store_destroy(store);
            m_engine_custom_property_environments.set(identity, *data);
            return data;
        }
    }
    // An environment resolved over another engine environment is held over the host's view of it, as
    // the engine's store is over the parent's: a child inherits the parent's environment by identity
    // wherever the element's own declarations do not inherit.
    RefPtr<CustomPropertyData const> data;
    if (auto parent = engine_custom_property_environment(parent_identity))
        data = CustomPropertyData::from_rust_store(store, move(parent), identity, false);
    else
        data = CustomPropertyData::from_rust_store(store, nullptr, identity, true);
    m_engine_custom_property_environments.set(identity, *data);
    return data;
}

// An environment nothing but the table holds is one no element is in, and the table is the only
// thing keeping it - and its parent chain - alive.
void StyleComputer::sweep_custom_property_environments() const
{
    m_engine_custom_property_environments.remove_all_matching([](auto&, NonnullRefPtr<CustomPropertyData const> const& data) { return data->ref_count() == 1; });
}

void StyleComputer::update_root_element_font_metrics(ComputedValues const& values)
{
    // NB: The style engine reads the metrics from the record the document element holds when it is
    //     installed, so only the host's copy is refreshed here.
    m_root_element_font_metrics = Length::FontMetrics { values.font_size(), values.font_list().first_available_font().pixel_metrics(), values.line_height() };
    m_root_element_font_metrics_depend_on_viewport_metrics = values.font_metrics_depend_on_viewport_metrics();
}

CSSPixels StyleComputer::default_user_font_size()
{
    // FIXME: This value should be configurable by the user.
    return 16;
}

// https://w3c.github.io/csswg-drafts/css-fonts/#absolute-size-mapping
CSSPixels StyleComputer::absolute_size_mapping(AbsoluteSize absolute_size, CSSPixels default_font_size)
{
    // An <absolute-size> keyword refers to an entry in a table of font sizes computed and kept by the user agent. See
    // § 2.5.1 Absolute Size Keyword Mapping Table.
    switch (absolute_size) {
    case AbsoluteSize::XxSmall:
        return default_font_size * CSSPixels(3) / 5;
    case AbsoluteSize::XSmall:
        return default_font_size * CSSPixels(3) / 4;
    case AbsoluteSize::Small:
        return default_font_size * CSSPixels(8) / 9;
    case AbsoluteSize::Medium:
        return default_font_size;
    case AbsoluteSize::Large:
        return default_font_size * CSSPixels(6) / 5;
    case AbsoluteSize::XLarge:
        return default_font_size * CSSPixels(3) / 2;
    case AbsoluteSize::XxLarge:
        return default_font_size * 2;
    case AbsoluteSize::XxxLarge:
        return default_font_size * 3;
    }

    VERIFY_NOT_REACHED();
}

ComputationContext StyleComputer::make_computation_context_for_property(PropertyID property_id, ComputedStyleWorkingSet const& style, Optional<DOM::AbstractElement> abstract_element) const
{
    auto tree_scope = abstract_element.has_value()
        ? abstract_element->style_scope().style_engine_tree_scope().value()
        : document().style_scope().style_engine_tree_scope().value();
    auto subject_inline_axis_is_horizontal = [&]() {
        auto writing_mode = [&](DOM::AbstractElement const& candidate) -> Optional<WritingMode> {
            auto const* inherited_box = candidate.style_group<ComputedValues::InheritedBoxValues>();
            if (!inherited_box)
                return {};
            return static_cast<WritingMode>(inherited_box->writing_mode);
        };
        if (!abstract_element.has_value())
            return true;
        if (auto mode = writing_mode(*abstract_element); mode.has_value())
            return *mode == WritingMode::HorizontalTb;
        if (auto inheritance_parent = abstract_element->element_to_inherit_style_from(); inheritance_parent.has_value() && inheritance_parent->has_style())
            return writing_mode(*inheritance_parent).value_or(WritingMode::HorizontalTb) == WritingMode::HorizontalTb;
        return true;
    }();

    bool const is_document_element = abstract_element.has_value()
        && !abstract_element->pseudo_element().has_value()
        && abstract_element->element().is_document_element();

    switch (property_id) {
    // FIXME: While `color-scheme` doesn't actually require a computation context (since it only takes keyword values),
    //        callers request one uniformly. Since `color-scheme` must be computed before creating a generic computation
    //        context, use the font context instead.
    case PropertyID::ColorScheme:
    case PropertyID::FontFamily:
    case PropertyID::FontFeatureSettings:
    case PropertyID::FontKerning:
    case PropertyID::FontOpticalSizing:
    case PropertyID::FontSize:
    case PropertyID::FontStyle:
    case PropertyID::FontVariantAlternates:
    case PropertyID::FontVariantCaps:
    case PropertyID::FontVariantEastAsian:
    case PropertyID::FontVariantEmoji:
    case PropertyID::FontVariantLigatures:
    case PropertyID::FontVariantNumeric:
    case PropertyID::FontVariantPosition:
    case PropertyID::FontVariationSettings:
    case PropertyID::FontWeight:
    case PropertyID::FontWidth:
    case PropertyID::MathDepth:
    case PropertyID::TextRendering: {
        auto inheritance_parent = abstract_element.map([](auto& element) { return element.element_to_inherit_style_from(); }).value_or(OptionalNone {});
        auto length_resolution_context = inheritance_parent.has_value() && inheritance_parent->has_style() && inheritance_parent->element().navigable()
            ? Length::ResolutionContext::for_element(inheritance_parent.value())
            : Length::ResolutionContext::for_document(m_document);
        length_resolution_context.subject_inline_axis_is_horizontal = subject_inline_axis_is_horizontal;
        length_resolution_context.subject_element = abstract_element.has_value() ? &abstract_element->element() : nullptr;

        return {
            .length_resolution_context = length_resolution_context,
            .abstract_element = abstract_element
        };
    }
    case PropertyID::LineHeight: {
        auto inheritance_parent = abstract_element.map([](auto& element) { return element.element_to_inherit_style_from(); }).value_or(OptionalNone {});

        auto line_height_font_metrics = Length::FontMetrics {
            style.font_size(),
            style.first_available_computed_font(document().font_computer(), tree_scope)->pixel_metrics(),
            inheritance_parent.has_value() && inheritance_parent->has_style() ? inheritance_parent->computed_style()->line_height() : InitialValues::line_height()
        };

        return {
            .length_resolution_context = {
                .viewport_rect = viewport_rect(),
                .font_metrics = line_height_font_metrics,
                .root_font_metrics = is_document_element
                    ? line_height_font_metrics
                    : m_root_element_font_metrics,
                .font_metrics_depend_on_viewport_metrics = style.font_metrics_depend_on_viewport_metrics(),
                .root_font_metrics_depend_on_viewport_metrics = is_document_element
                    ? style.font_metrics_depend_on_viewport_metrics()
                    : m_root_element_font_metrics_depend_on_viewport_metrics,
                .subject_inline_axis_is_horizontal = subject_inline_axis_is_horizontal,
                .subject_element = abstract_element.has_value() ? &abstract_element->element() : nullptr,
            },
            .abstract_element = abstract_element
        };
    }
    default: {
        auto font_metrics = Length::FontMetrics {
            style.font_size(),
            style.first_available_computed_font(document().font_computer(), tree_scope)->pixel_metrics(),
            style.line_height(document().font_computer(), tree_scope)
        };
        return {
            .length_resolution_context = {
                .viewport_rect = viewport_rect(),
                .font_metrics = font_metrics,
                .root_font_metrics = is_document_element ? font_metrics : m_root_element_font_metrics,
                .font_metrics_depend_on_viewport_metrics = style.font_metrics_depend_on_viewport_metrics(),
                .root_font_metrics_depend_on_viewport_metrics = is_document_element ? style.font_metrics_depend_on_viewport_metrics() : m_root_element_font_metrics_depend_on_viewport_metrics,
                .subject_inline_axis_is_horizontal = subject_inline_axis_is_horizontal,
                .subject_element = abstract_element.has_value() ? &abstract_element->element() : nullptr,
            },
            .abstract_element = abstract_element,
            .color_scheme = style.color_scheme(document().page().preferred_color_scheme(), document().supported_color_schemes())
        };
    }
    }

    VERIFY_NOT_REACHED();
}

ComputationContext const& StyleComputer::get_computation_context_for_property(PropertyID property_id, ComputedStyleWorkingSet const& style, Optional<DOM::AbstractElement> abstract_element) const
{
    switch (property_id) {
    case PropertyID::ColorScheme:
    case PropertyID::FontFamily:
    case PropertyID::FontFeatureSettings:
    case PropertyID::FontKerning:
    case PropertyID::FontOpticalSizing:
    case PropertyID::FontSize:
    case PropertyID::FontStyle:
    case PropertyID::FontVariantAlternates:
    case PropertyID::FontVariantCaps:
    case PropertyID::FontVariantEastAsian:
    case PropertyID::FontVariantEmoji:
    case PropertyID::FontVariantLigatures:
    case PropertyID::FontVariantNumeric:
    case PropertyID::FontVariantPosition:
    case PropertyID::FontVariationSettings:
    case PropertyID::FontWeight:
    case PropertyID::FontWidth:
    case PropertyID::MathDepth:
    case PropertyID::TextRendering:
        if (!m_cached_font_computation_context.has_value())
            m_cached_font_computation_context = make_computation_context_for_property(property_id, style, abstract_element);
        return m_cached_font_computation_context.value();
    case PropertyID::LineHeight:
        if (!m_cached_line_height_computation_context.has_value())
            m_cached_line_height_computation_context = make_computation_context_for_property(property_id, style, abstract_element);
        return m_cached_line_height_computation_context.value();
    default:
        if (!m_cached_generic_computation_context.has_value())
            m_cached_generic_computation_context = make_computation_context_for_property(property_id, style, abstract_element);
        return m_cached_generic_computation_context.value();
    }
}

enum class BoxTypeParentDisplaySource {
    Host,
    Retained,
};

static ComputedValuesFFI::FfiBoxTypeTransformationInput make_box_type_transformation_input(
    DOM::AbstractElement abstract_element, Optional<Display> known_parent_display = {}, Optional<u32> published_adjustment_facts = {}, BoxTypeParentDisplaySource parent_display_source = BoxTypeParentDisplaySource::Host)
{
    auto& element = abstract_element.element();

    Optional<Display> parent_display = known_parent_display;
    if (parent_display_source == BoxTypeParentDisplaySource::Host) {
        // NOTE: If we're computing style for a pseudo-element, the effective parent will be the originating element itself, not its parent.
        auto parent = abstract_element.element_to_inherit_style_from();

        // Climb out of `display: contents` context.
        parent_display.clear();
        while (parent.has_value() && parent->has_style()) {
            auto display = [&] {
                if (known_parent_display.has_value())
                    return *known_parent_display;
                return parent->computed_style()->display();
            }();
            known_parent_display.clear();
            if (!display.is_contents()) {
                parent_display = display;
                break;
            }
            parent = parent->element_to_inherit_style_from();
        }
    }

    return ComputedValuesFFI::rust_box_type_transformation_input(
        published_adjustment_facts.value_or_lazy_evaluated([&] { return element_box_type_adjustment_facts(element); }),
        abstract_element.pseudo_element().has_value()
            ? ComputedValuesFFI::FfiStyleAdjustmentTarget::PseudoElement
            : ComputedValuesFFI::FfiStyleAdjustmentTarget::Element,
        parent_display.has_value(),
        parent_display.has_value() ? to_ffi_display(*parent_display) : ComputedValuesFFI::FfiDisplay {});
}

static ComputedValuesFFI::FfiInputLineHeightMetrics input_line_height_metrics(ComputedStyleWorkingSet const& style, DOM::AbstractElement abstract_element, bool should_measure)
{
    ComputedValuesFFI::FfiInputLineHeightMetrics line_height_metrics {};
    if (should_measure) {
        auto tree_scope = abstract_element.style_scope().style_engine_tree_scope().value();
        line_height_metrics.current_line_height = style.line_height(abstract_element.element().document().font_computer(), tree_scope).to_double();
        line_height_metrics.minimum_line_height = normal_line_height(style.first_available_computed_font(abstract_element.element().document().font_computer(), tree_scope)->pixel_metrics()).to_double();
    }
    return line_height_metrics;
}

void StyleComputer::finalize_style(ComputedStyleWorkingSet& style, DOM::AbstractElement abstract_element, ComputedValuesFFI::FfiStyleFinalizationMode mode) const
{
    bool const animated_box_type = mode == ComputedValuesFFI::FfiStyleFinalizationMode::AnimatedBoxType;
    VERIFY(animated_box_type || mode == ComputedValuesFFI::FfiStyleFinalizationMode::BoxType);
    ComputedValuesFFI::FfiStyleFinalizationInput input {};
    input.mode = mode;
    input.box_type = make_box_type_transformation_input(abstract_element);
    auto line_height_metrics = input_line_height_metrics(style, abstract_element, input.box_type.check_input_line_height);
    auto* animated_overlay = style.prepare_animated_overlay_for_rust_finalization(
        Badge<StyleComputer> {}, animated_box_type ? ComputedStyleWorkingSet::CreateAnimatedOverlay::Yes : ComputedStyleWorkingSet::CreateAnimatedOverlay::No);
    auto finalization = ComputedValuesFFI::rust_finalize_style(
        &input, style.mutable_computed_longhand_table(), animated_overlay, &line_height_metrics);
    style.did_apply_style_finalization_from_rust(finalization.invalidated_longhands);
    style.finish_animated_overlay_rust_mutation(Badge<StyleComputer> {});
}

NonnullRefPtr<ComputedValues const> StyleComputer::create_document_style() const
{
    ensure_style_metadata_tables_installed();

    Vector<u8> document_supported_color_scheme_codes;
    auto document_supported_color_schemes = document().supported_color_schemes();
    if (document_supported_color_schemes.has_value()) {
        document_supported_color_scheme_codes.ensure_capacity(document_supported_color_schemes->size());
        for (auto const& scheme : *document_supported_color_schemes)
            document_supported_color_scheme_codes.unchecked_append(to_underlying(preferred_color_scheme_from_string(scheme)));
    }
    auto length_resolution_context = CSS::Length::ResolutionContext::for_document(document());
    auto viewport_rect = this->viewport_rect();
    ComputedValuesFFI::FfiDocumentLonghandInput const input {
        .color_scheme_input = {
            .preferred_color_scheme = static_cast<u8>(to_underlying(document().page().preferred_color_scheme())),
            .has_document_supported_schemes = document_supported_color_schemes.has_value(),
            .document_supported_scheme_codes = document_supported_color_scheme_codes.data(),
            .document_supported_scheme_count = document_supported_color_scheme_codes.size(),
        },
        .length_resolution_context = to_ffi_length_resolution_context(length_resolution_context),
        .device_pixels_per_css_pixel = m_document->page().client().device_pixels_per_css_pixel(),
        .initial_font_size_raw = InitialValues::font_size().raw_value(),
        .default_font_size_raw = default_user_font_size().raw_value(),
        .viewport_width = viewport_rect.width().to_double(),
        .viewport_height = viewport_rect.height().to_double(),
    };
    auto computed_properties = CSS::ComputedStyleWorkingSet::create_with_longhand_table(ComputedValuesFFI::rust_create_document_longhand_table(&input));
    CSS::ColorResolutionContext color_resolution_context {
        .color_scheme = document().page().preferred_color_scheme(),
        .current_color = CSS::InitialValues::color(),
        .current_color_style_value = &computed_properties->property(PropertyID::Color),
        .calculation_resolution_context = { .length_resolution_context = CSS::Length::ResolutionContext::for_document(document()) },
    };
    auto computed_values = CSS::ComputedValues::create(*computed_properties, document(), document().style_scope(), move(color_resolution_context));
    return computed_values;
}

StyleRecordID StyleComputer::intern_computed_style_inputs(DOM::AbstractElement abstract_element, ComputedValues const& values) const
{
    return record_computed_style_inputs(Optional<DOM::AbstractElement> { abstract_element }, values, 0).new_style_record;
}

StyleRecordID StyleComputer::intern_anonymous_layout_style(ComputedValues const& values) const
{
    return record_computed_style_inputs({}, values, 0).new_style_record;
}

StyleEngine::StyleRecordDelta StyleComputer::record_computed_style_inputs(Optional<DOM::AbstractElement> abstract_element, ComputedValues const& values, StyleNodeID style_node_id) const
{
    auto const& base = values.base_values();
    // An unassigned record cannot own an animation overlay, so a layout-derived copy of an
    // animated style interns its final merged payloads directly. Splitting off the base there
    // would intern (and paint) the un-animated values.
    bool const unassigned_with_animations = style_node_id == 0 && (values.has_animated_values() || values.animated_properties());
    auto const& payload_source = unassigned_with_animations ? values : base;
    Array<void const*, to_underlying(StyleGroupIndex::Count)> payloads;
    for (size_t index = 0; index < payloads.size(); ++index)
        payloads[index] = payload_source.style_group_payload(static_cast<StyleGroupIndex>(index));
    auto custom_property_environment = abstract_element.has_value() ? abstract_element->custom_property_data() : nullptr;
    u64 counter_style_environment_identity = 0;
    if (abstract_element.has_value()
        && ComputedValuesFFI::rust_computed_style_reads_counter_style_environment(values.base_values().computed_longhand_table(), abstract_element->pseudo_element().has_value()))
        counter_style_environment_identity = abstract_element->style_scope().counter_style_environment_identity();
    auto animated_properties = style_node_id != 0 ? values.animated_properties() : nullptr;
    u64 animation_overlay_identity = animated_properties ? animated_properties->identity() : 0;
    Array<void const*, to_underlying(StyleGroupIndex::Count)> animation_overlay_payloads;
    if (animated_properties) {
        for (size_t index = 0; index < animation_overlay_payloads.size(); ++index)
            animation_overlay_payloads[index] = values.style_group_payload(static_cast<StyleGroupIndex>(index));
    }
    auto pseudo_kind = pseudo_element_to_ffi(abstract_element.has_value() ? abstract_element->pseudo_element() : Optional<CSS::PseudoElement> {});
    auto publication = m_document->render_inputs_for_write().style_engine().publish_computed_groups(style_node_id, pseudo_kind, payloads, ComputedValues::inherited_style_group_count, custom_property_environment ? custom_property_environment->identity() : 0, false, counter_style_environment_identity, animation_overlay_identity, animated_properties ? animated_properties->overlay() : nullptr, animated_properties ? animation_overlay_payloads.span() : ReadonlySpan<void const*> {}, base.computed_longhand_table(), custom_property_environment ? custom_property_environment->rust_store() : nullptr);
    return publication;
}

NonnullRefPtr<ComputedStyleWorkingSet> StyleComputer::reconstruct_computed_properties(ComputedValues const& computed_values) const
{
    auto style = ComputedStyleWorkingSet::create_with_base_values_from(computed_values);
    // The recorded pre-box-type-transformation display tracks the animated display while one is applied, on both
    // the animated style and its base. When the animation stops covering `display`, re-adjustment must start over
    // from the base style's display, or the sampled value the finished animation left behind is resurrected as
    // the element's display. Box-type transformations are idempotent, so the adjusted base display is a sound
    // transformation input.
    if (auto const* animated_properties = computed_values.animated_properties(); animated_properties && animated_properties->has_property(PropertyID::Display))
        style->set_display_before_box_type_transformation(computed_values.base_values().display());
    style->freeze_computed_longhand_table();
    apply_animated_properties_to_reconstruction(*style, computed_values);
    return style;
}

void StyleComputer::apply_animated_properties_to_reconstruction(ComputedStyleWorkingSet& style, ComputedValues const& computed_values) const
{
    auto const* animated_properties = computed_values.animated_properties();
    if (!animated_properties)
        return;
    for (auto const& entry : animated_properties->entries()) {
        auto property_id = static_cast<PropertyID>(entry.property);
        style.set_animated_property(
            Badge<StyleComputer> {}, property_id, animated_properties->property(property_id),
            entry.result_of_transition ? AnimatedPropertyResultOfTransition::Yes : AnimatedPropertyResultOfTransition::No,
            entry.inherited ? ComputedStyleWorkingSet::Inherited::Yes : ComputedStyleWorkingSet::Inherited::No);
    }
}

NonnullRefPtr<ComputedStyleWorkingSet> StyleComputer::reconstruct_computed_properties_for_animation(PublishedStyleRecord const& style_record) const
{
    auto const& record = style_record.view();
    auto style = ComputedStyleWorkingSet::create_for_animation_update(
        static_cast<ComputedValuesFFI::ComputedLonghandTable const*>(record.longhand_table),
        static_cast<ComputedValuesFFI::AnimatedOverlay const*>(record.animated_overlay));
    if (record.animated_overlay && ComputedValuesFFI::rust_animated_overlay_contains(static_cast<ComputedValuesFFI::AnimatedOverlay const*>(record.animated_overlay), to_underlying(PropertyID::Display))) {
        auto const* box = static_cast<ComputedValuesFFI::BoxValues const*>(record.base_payloads[to_underlying(StyleGroupIndex::BoxValues)]);
        style->set_display_before_box_type_transformation(display_from_ffi_display(box->display));
    }
    return style;
}

u64 StyleComputer::style_environment_version_for_sharing() const
{
    return document().style_environment_version() ^ (m_viewport_environment_version << 32);
}

void StyleComputer::ensure_style_metadata_tables_installed()
{
    static bool const installed = [] {
        // Transfer one shared Rust reference for every longhand initial value, so
        // initial-value selection never crosses the FFI.
        Vector<void const*> initial_value_entries;
        initial_value_entries.ensure_capacity(number_of_longhand_properties);
        for (auto i = to_underlying(first_longhand_property_id); i <= to_underlying(last_longhand_property_id); ++i) {
            auto initial_value = property_initial_value(static_cast<PropertyID>(i));
            initial_value_entries.unchecked_append(StyleValueFFI::rust_style_value_retain(initial_value->rust_style_value_data()));
        }
        ComputedValuesFFI::rust_style_metadata_set_initial_value_table(initial_value_entries.data(), initial_value_entries.size());

        return true;
    }();
    (void)installed;
}

ComputationContext StyleComputer::fallback_computation_context_for_custom_property(AbstractOrHypotheticalElement const& element) const
{
    auto abstract_element = element.abstract_element();

    auto context_from_computed_values = [&](DOM::AbstractElement const& styled_element) -> ComputationContext {
        auto length_resolution_context = Length::ResolutionContext::for_element(styled_element);
        length_resolution_context.subject_element = &abstract_element.element();
        return {
            .length_resolution_context = move(length_resolution_context),
            .abstract_element = abstract_element,
            .color_scheme = styled_element.computed_style()->color_scheme(),
        };
    };

    bool can_resolve_element_lengths = abstract_element.element().navigable()
        && document().document_element()
        && document().document_element()->has_style();

    if (can_resolve_element_lengths && abstract_element.has_style())
        return context_from_computed_values(abstract_element);

    if (auto parent = abstract_element.element_to_inherit_style_from(); can_resolve_element_lengths && parent.has_value() && parent->has_style())
        return context_from_computed_values(*parent);

    auto length_resolution_context = Length::ResolutionContext::for_document(document());
    length_resolution_context.subject_element = &abstract_element.element();
    return {
        .length_resolution_context = length_resolution_context,
        .abstract_element = abstract_element,
    };
}

NonnullRefPtr<StyleValue const> StyleComputer::compute_font_size(NonnullRefPtr<StyleValue const> const& absolutized_value, int computed_math_depth, Optional<DOM::AbstractElement> const& inheritance_parent, CSSPixels initial_font_size)
{
    auto inherited_font_size = inheritance_parent.has_value() && inheritance_parent->has_style()
        ? inheritance_parent->computed_style()->font_size()
        : initial_font_size;

    auto inherited_math_depth = inheritance_parent.has_value() && inheritance_parent->has_style()
        ? inheritance_parent->computed_style()->math_depth()
        : InitialValues::math_depth();

    // The size keyword tables and the math scaling rules live in the Rust style computation core.
    auto result = ComputedValuesFFI::rust_compute_font_size(absolutized_value->rust_style_value_data(), computed_math_depth, inherited_font_size.raw_value(), inherited_math_depth, default_user_font_size().raw_value());
    if (result.handled) {
        if (result.unchanged)
            return absolutized_value;
        return LengthStyleValue::create(Length::make_px(result.value));
    }

    VERIFY(absolutized_value->is_calculated());
    return LengthStyleValue::create(absolutized_value->as_calculated().resolve_length({ .percentage_basis = Length::make_px(inherited_font_size) }).value());
}

// The FontStyleKeyword discriminants cross the boundary as the mapped keyword code; pin them.
static_assert(to_underlying(FontStyleKeyword::Normal) == 0);
static_assert(to_underlying(FontStyleKeyword::Italic) == 1);
static_assert(to_underlying(FontStyleKeyword::Left) == 2);
static_assert(to_underlying(FontStyleKeyword::Right) == 3);
static_assert(to_underlying(FontStyleKeyword::Oblique) == 4);

NonnullRefPtr<StyleValue const> StyleComputer::compute_font_style(NonnullRefPtr<StyleValue const> const& absolutized_value)
{
    // https://drafts.csswg.org/css-fonts-4/#font-style-prop
    // the keyword specified, plus angle in degrees if specified

    // The keyword-to-font-style-keyword mapping lives in the Rust style computation core.
    // NB: We always parse as a FontStyleStyleValue, but StylePropertyMap is able to set a KeywordStyleValue directly.
    auto computation = ComputedValuesFFI::rust_compute_font_style(absolutized_value->rust_style_value_data());
    if (computation.is_keyword)
        return FontStyleStyleValue::create(static_cast<FontStyleKeyword>(computation.font_style_keyword));

    return absolutized_value;
}

NonnullRefPtr<StyleValue const> StyleComputer::compute_font_weight(NonnullRefPtr<StyleValue const> const& absolutized_value, Optional<DOM::AbstractElement> const& inheritance_parent)
{
    auto inherited_font_weight = inheritance_parent.has_value() && inheritance_parent->has_style()
        ? inheritance_parent->computed_style()->font_weight()
        : InitialValues::font_weight();

    // The weight chart lives in the Rust style computation core.
    auto result = ComputedValuesFFI::rust_compute_font_weight(absolutized_value->rust_style_value_data(), inherited_font_weight);
    if (result.handled) {
        if (result.unchanged)
            return absolutized_value;
        return NumberStyleValue::create(result.value);
    }

    // AD-HOC: Anywhere we support a numbers we should also support calcs
    VERIFY(absolutized_value->is_calculated());
    return NumberStyleValue::create(absolutized_value->as_calculated().resolve_number({}).value());
}

NonnullRefPtr<StyleValue const> StyleComputer::compute_font_width(NonnullRefPtr<StyleValue const> const& absolutized_value)
{
    // The width keyword percentage table lives in the Rust style computation core.
    auto result = ComputedValuesFFI::rust_compute_font_width(absolutized_value->rust_style_value_data());
    if (result.handled) {
        if (result.unchanged)
            return absolutized_value;
        return PercentageStyleValue::create(Percentage(result.value));
    }

    // AD-HOC: We support calculated percentages as well
    VERIFY(absolutized_value->is_calculated());
    return PercentageStyleValue::create(absolutized_value->as_calculated().resolve_percentage({}).value());
}

}
