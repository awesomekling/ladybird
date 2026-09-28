/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#include <AK/AnyOf.h>
#include <AK/HashTable.h>
#include <AK/ScopeGuard.h>
#include <AK/StdLibExtras.h>
#include <AK/TemporaryChange.h>
#include <AK/Time.h>
#include <LibGfx/Font/SharedFontProvider.h>
#include <LibWeb/CSS/ComputedStyleWorkingSet.h>
#include <LibWeb/CSS/CustomPropertyData.h>
#include <LibWeb/CSS/FontResolution.h>
#include <LibWeb/CSS/PublishedStyleRecord.h>
#include <LibWeb/CSS/RustDeclarationBlock.h>
#include <LibWeb/CSS/SharedCompiledStyleSheet.h>
#include <LibWeb/CSS/StyleComputer.h>
#include <LibWeb/CSS/StyleEffectDrain.h>
#include <LibWeb/CSS/StyleEngineBridge.h>
#include <LibWeb/CSS/StyleEngineInput.h>
#include <LibWeb/CSS/StyleInputScope.h>
#include <LibWeb/CSS/StyleScope.h>
#include <LibWeb/CSS/StyleSheetImport.h>
#include <LibWeb/CSS/StyleSheetState.h>
#include <LibWeb/DOM/Document.h>
#include <LibWeb/DOM/ShadowRoot.h>
#include <LibWeb/HTML/Scripting/Environments.h>
#include <LibWeb/Layout/LayoutRustBridge.h>
#include <LibWeb/Layout/LayoutRustFFI.h>
#include <LibWeb/Page/Page.h>
#include <LibWeb/Painting/BoxViews.h>
#include <LibWeb/StyleValueRustFFI.h>

namespace Web::CSS {

extern "C" void rust_font_face_snapshot_view(void const*, FontFaceSnapshotView*);

bool StyleEngine::layout_pass_is_in_flight() const
{
    return Layout::RustFFI::rust_stage_thread_layout_pass_in_flight_for(rust_handle());
}

void StyleEngine::publish_input(Function<void(StyleInputScope const&)>&& input)
{
    if (pass_is_in_flight() || layout_pass_is_in_flight()) {
        m_inputs_queued_during_pass.enqueue(move(input));
        return;
    }
    // A layout pass taken back publishes what waited for it as its frame ends, and code the take-back runs before
    // that may publish too: what waited goes first. An input that one of those publishes is part of it, and goes
    // before the rest of what waited.
    if (!m_publishing_queued_inputs)
        publish_inputs_queued_during_pass();
    StyleInputScope const scope { *this };
    input(scope);
}

void StyleEngine::begin_holding_input_recorded_beside_pass()
{
    VERIFY(!m_holds_input_recorded_beside_pass);
    // With nothing recorded beside the pass, what the drain records goes on with its waves as it does in place.
    m_holds_input_recorded_beside_pass = has_recorded_input() || !m_host_fact_writes.is_empty() || has_changed_node_lists() || m_style_node_grant_request || m_text_style_node_grant_request || !m_inputs_queued_during_pass.is_empty();
}

void StyleEngine::end_holding_input_recorded_beside_pass()
{
    m_holds_input_recorded_beside_pass = false;
    // What was sent to the engine beside the pass goes behind what its drain sent.
    StyleEngineFFI::style_engine_release_changes_sent_beside_pass(rust_handle());
    publish_inputs_queued_during_pass();
}

void StyleEngine::publish_inputs_queued_during_pass()
{
    // What waited is published one input at a time, each of them whole, in the order the host published them.
    if (m_publishing_queued_inputs)
        return;
    TemporaryChange publishing_queued_inputs { m_publishing_queued_inputs, true };
    while (!pass_is_in_flight() && !m_holds_input_recorded_beside_pass && !m_inputs_queued_during_pass.is_empty() && !layout_pass_is_in_flight()) {
        auto input = m_inputs_queued_during_pass.dequeue();
        StyleInputScope const scope { *this };
        input(scope);
    }
    // Every removal published beside the pass has reached the engine, which answers for none of those nodes again.
    if (!m_submitted_pass_in_flight && !pass_is_in_flight() && !m_holds_input_recorded_beside_pass && m_inputs_queued_during_pass.is_empty()) {
        m_style_nodes_retired_beside_pass.clear();
        m_parents_whose_children_changed_beside_pass.clear();
    }
}

// The style stage's between-pass font batch. It is a function of the document's published
// `@font-face` table and the request, and of the process-wide font services behind them: no
// document is reachable from here, and no pointer to one is passed in. That is what lets the
// stage's own thread run this batch instead of joining the document's.
static StyleEngineFFI::FfiResolvedFont resolve_font(FontCascadeMemo& memo, FontFaceSnapshotView const& font_faces, StyleEngineFFI::FfiFontResolutionRequest request)
{
    // The engine holds these values as opaque handles, never as pointers it could follow; the
    // bridge is where they become the engine's value data again. They are read as that data and
    // never wrapped in a StyleValue: this runs on whichever thread runs the pass, and a
    // StyleValue's reference count (the process-wide interned keywords' among them) is not atomic.
    // Each request the engine batches retains the values it names, so they outlive this call.
    auto value_data = [](StyleEngineFFI::FfiHostHandle handle) {
        return reinterpret_cast<StyleValueFFI::StyleValueData const*>(handle);
    };
    // A numeric variant selects shaping features, so it belongs to the resolution rather than to
    // the record. The engine names nothing when it is `normal`.
    // The engine names a value for each input it did not leave at its initial value; the rest are
    // absent and the assembler reads them as the initial value they are.
    Array<StyleValueFFI::StyleValueData const*, to_underlying(FontResolutionFeatureInput::Count)> feature_values;
    for (size_t index = 0; index < feature_values.size(); ++index)
        feature_values[index] = value_data(request.font_feature_values[index]);
    ComputedFontCacheKey key {
        .tree_scope = request.tree_scope,
        .font_families = computed_font_families_from_value_data(*value_data(request.font_family)),
        .font_optical_sizing = static_cast<FontOpticalSizing>(request.font_optical_sizing),
        .font_size = CSSPixels::from_raw(request.font_size_raw),
        .font_slope = request.font_slope,
        .font_weight = request.font_weight,
        .font_width = Percentage(request.font_width),
        .font_variation_settings = font_variation_settings_from_value_data(feature_values.span()),
        .font_feature_data = font_feature_data_from_value_data(feature_values.span()),
    };
    auto font_list = memo.resolve(font_faces, move(key));
    // The metric probe must not load a face: the first available font answers without one.
    auto const& first_available_font = font_list->first_available_font();
    auto const metrics = first_available_font.pixel_metrics();
    // The engine's resolver cache adopts this reference and releases it on eviction.
    return {
        // Handles, not pointers: the engine names these host objects and hands them back here.
        .first_available_font = reinterpret_cast<StyleEngineFFI::FfiHostHandle>(&first_available_font),
        .font_cascade_list = reinterpret_cast<StyleEngineFFI::FfiHostHandle>(&font_list.leak_ref()),
        .ascent = metrics.ascent,
        .descent = metrics.descent,
        .x_height = metrics.x_height,
        .zero_advance = metrics.advance_of_ascii_zero,
    };
}

static void resolve_fonts(uintptr_t font_cascade_memo, void const* font_face_snapshot, StyleEngineFFI::FfiFontResolutionRequest const* requests, StyleEngineFFI::FfiResolvedFont* resolved_fonts, size_t count)
{
    FontFaceSnapshotView font_faces;
    rust_font_face_snapshot_view(font_face_snapshot, &font_faces);
    auto& memo = *reinterpret_cast<FontCascadeMemo*>(font_cascade_memo);
    // Family matching leaves the process for the first use of any system family, and the
    // connection the font provider's callbacks use belongs to the document thread. Inside this
    // scope those questions go out on the render side's own connection instead.
    Gfx::RenderSideFontScope render_side_font_scope;
    // A face the published table says is pending stays pending for the batch: its live state is
    // the document thread's, as is the font it may have loaded since.
    Gfx::PublishedPendingFaceScope published_pending_face_scope;
    for (size_t index = 0; index < count; ++index)
        resolved_fonts[index] = resolve_font(memo, font_faces, requests[index]);
}

static_assert(StyleEngineFFI::LAST_SYNTHETIC_PSEUDO_ELEMENT_KIND == to_underlying(last_synthetic_pseudo_element));
static_assert(!IsMoveConstructible<StyleEngine>);
static_assert(!IsMoveAssignable<StyleEngine>);

#include <LibWeb/StyleEngineBridgeGenerated.inc>

StyleEngine::StyleEngine(void* render_state_arena, DeviceClass device_class, StyleComputer* style_computer)
    : m_host_style_record_pins(StyleEngineFFI::style_record_host_pins_create())
    , m_style_computer(style_computer)
{
    // The engine is born linked to its document's render state, with the pin table lent to it and, for a document that
    // computes style, the font resolver installed, and answers the recording stream it records under.
    m_impl = StyleEngineFFI::style_engine_create(render_state_arena, device_class, m_host_style_record_pins, m_style_computer ? resolve_fonts : nullptr, &m_recording_stream);
    if (m_style_computer)
        set_pseudo_element_style_deferred(to_underlying(PseudoElement::Selection), true);
}

void StyleEngine::prepare_root_font_resolution(u64 font_environment_generation)
{
    publish_font_faces();
    publish_input_or_apply_in_drain([this, font_environment_generation](auto const& scope) {
        StyleEngineFFI::style_engine_prepare_root_font_resolution(scope, rust_handle(), font_environment_generation);
    });
}

// The document's `@font-face` table, handed to the engine for the generation it is about to
// compute against. Publishing here and before a transaction covers every entry into the stage.
//
// Dual (see StyleInputScope): a font change beside a pass is already left as published input, and inside
// a drain its later waves resolve against the table, so it goes to the engine at once there. Beside a
// pass that finished and waits for its drain it waits too: every transaction republishes the table.
void StyleEngine::publish_font_faces()
{
    if (!m_style_computer)
        return;
    publish_input_or_apply_in_drain([this](auto const& scope) {
        auto& font_computer = m_style_computer->document().font_computer();
        font_computer.font_cascade_memo().publish_font_feature_values(font_computer.published_font_feature_values());
        StyleEngineFFI::style_engine_publish_font_face_snapshot(scope, rust_handle(), font_computer.published_font_faces(), reinterpret_cast<size_t>(&font_computer.font_cascade_memo()));
    });
}

StyleEngine::~StyleEngine()
{
    // What a write took that never crossed is still the write's to give up.
    for (auto const& write : m_host_fact_writes) {
        if (write.kind == StyleEngineFFI::FfiHostFactKind::ElementInlineStyleProperties || write.kind == StyleEngineFFI::FfiHostFactKind::ElementPresentationalHints)
            Parser::ValueParserFFI::rust_declaration_data_release(bit_cast<Parser::ValueParserFFI::DeclarationBlockData const*>(write.data));
        else if (write.kind == StyleEngineFFI::FfiHostFactKind::AdoptAtom)
            StyleEngineFFI::style_engine_release_host_atom(write.data, write.facts);
        else if (write.kind == StyleEngineFFI::FfiHostFactKind::AdoptQualifiedAtom)
            StyleEngineFFI::style_engine_release_host_qualified_atom(write.node, write.parent, write.facts);
    }
    StyleEngineFFI::style_engine_destroy(rust_handle());
    StyleEngineFFI::style_record_host_pins_destroy(m_host_style_record_pins);
    for (auto const& atom : m_atoms)
        Utf16FlyString::unref_raw(atom.key);
}

void StyleEngine::visit_edges(GC::Cell::Visitor& visitor)
{
    visitor.visit(m_style_computer);
}

// A grant covers what a burst of insertions mints between two transactions, and is topped up with the
// next one once it runs low. Only a burst larger than that asks for identities on the spot.
static constexpr size_t STYLE_NODE_GRANT_SIZE = 256;
static constexpr size_t STYLE_NODE_GRANT_LOW_WATER = 64;

// Minted last to first, so the identities the engine granted first are minted first.
static void adopt_identity_grant(Vector<StyleNodeID>& granted, Vector<StyleNodeID> const& grant)
{
    for (auto node : grant.in_reverse())
        granted.append(node);
}

StyleNodeID StyleEngine::mint_style_node()
{
    StyleNodeID node;
    mint_style_nodes({ &node, 1 });
    return node;
}

void StyleEngine::mint_style_nodes(Span<StyleNodeID> nodes)
{
    mint_style_nodes(nodes, m_granted_style_nodes, m_style_node_grant_request, StyleEngineFFI::FfiHostFactKind::MintElement, 0);
}

void StyleEngine::mint_text_style_nodes(Span<StyleNodeID> nodes)
{
    mint_style_nodes(nodes, m_granted_text_style_nodes, m_text_style_node_grant_request, StyleEngineFFI::FfiHostFactKind::MintText, 0);
}

StyleNodeID StyleEngine::mint_relation_only_style_node()
{
    StyleNodeID node;
    mint_style_nodes({ &node, 1 }, m_granted_style_nodes, m_style_node_grant_request, StyleEngineFFI::FfiHostFactKind::MintElement, 1);
    return node;
}

void StyleEngine::mint_style_nodes(Span<StyleNodeID> nodes, Vector<StyleNodeID>& granted, size_t& grant_request, StyleEngineFFI::FfiHostFactKind kind, u8 value)
{
    if (nodes.is_empty())
        return;
    if (granted.size() < nodes.size()) {
        auto is_text = kind == StyleEngineFFI::FfiHostFactKind::MintText;
        ensure_granted_style_nodes(is_text ? 0 : nodes.size(), is_text ? nodes.size() : 0);
    }
    for (auto& node : nodes) {
        node = granted.take_last();
        record_host_fact_write({ .kind = kind, .value = value, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
    }
    if (granted.size() < STYLE_NODE_GRANT_LOW_WATER)
        grant_request = STYLE_NODE_GRANT_SIZE - granted.size();
}

void StyleEngine::ensure_granted_style_nodes(size_t element_count, size_t text_count)
{
    if (m_granted_style_nodes.size() >= element_count && m_granted_text_style_nodes.size() >= text_count)
        return;
    // A grant asked for on the spot is no input: it readies slots no node the engine knows names, and changes
    // no answer of a pass. So it is granted at once, even while a pass is in flight (a text control the drain
    // gives a shadow tree, say), and what the host recorded so far waits for its own transaction.
    Vector<StyleNodeID> style_node_grant;
    Vector<StyleNodeID> text_style_node_grant;
    if (m_granted_style_nodes.size() < element_count)
        style_node_grant.resize(element_count - m_granted_style_nodes.size() + STYLE_NODE_GRANT_SIZE);
    if (m_granted_text_style_nodes.size() < text_count)
        text_style_node_grant.resize(text_count - m_granted_text_style_nodes.size() + STYLE_NODE_GRANT_SIZE);
    StyleEngineFFI::style_engine_grant_style_nodes(rust_handle(), reinterpret_cast<u32*>(style_node_grant.data()), style_node_grant.size(), reinterpret_cast<u32*>(text_style_node_grant.data()), text_style_node_grant.size());
    adopt_identity_grant(m_granted_style_nodes, style_node_grant);
    adopt_identity_grant(m_granted_text_style_nodes, text_style_node_grant);
}

HashTable<StyleNodeID> StyleEngine::take_deferred_element_initial_features()
{
    return move(m_nodes_with_pending_initial_features);
}

HashTable<StyleNodeID> StyleEngine::take_elements_awaiting_first_style_computation()
{
    return move(m_nodes_awaiting_first_style_computation);
}

void StyleEngine::set_element_parts(StyleNodeID node, ReadonlySpan<StyleAtomID> names, ReadonlySpan<StyleNodeID> hosts)
{
    VERIFY(names.size() == hosts.size());
    if (names.is_empty()) {
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementParts, .value = 1, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
        return;
    }
    for (size_t index = 0; index < names.size(); ++index)
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementParts, .value = index == 0, .node = node.value(), .parent = hosts[index].value(), .previous_sibling = 0, .facts = names[index].value(), .data = 0 });
}

void StyleEngine::set_element_id_name(StyleNodeID node, StyleAtomID name)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementIdName, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = name.value(), .data = 0 });
}

void StyleEngine::set_element_directionality(StyleNodeID node, StyleAtomID directionality)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementDirectionality, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = directionality.value(), .data = 0 });
}

void StyleEngine::set_element_heading_level(StyleNodeID node, u8 level)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementHeadingLevel, .value = level, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::set_element_custom_states(StyleNodeID node, ReadonlySpan<StyleAtomID> states)
{
    if (states.is_empty()) {
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementCustomState, .value = 1, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
        return;
    }
    for (size_t index = 0; index < states.size(); ++index)
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementCustomState, .value = index == 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = states[index].value(), .data = 0 });
}

void StyleEngine::set_element_part_exposure(StyleNodeID node, StyleNodeID exposure)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementPartExposure, .value = 0, .node = node.value(), .parent = exposure.value(), .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::finish_sheet_rules_replacement(SheetID sheet)
{
    publish_input([sheet](StyleInputScope const& input) {
        StyleEngineFFI::style_engine_finish_sheet_rules_replacement(input.engine().rust_handle(), sheet.value());
    });
}

static bool property_defines_a_css_transition(PropertyID property_id)
{
    switch (property_id) {
    case PropertyID::Transition:
    case PropertyID::TransitionBehavior:
    case PropertyID::TransitionDelay:
    case PropertyID::TransitionDuration:
    case PropertyID::TransitionProperty:
    case PropertyID::TransitionTimingFunction:
        return true;
    default:
        return false;
    }
}

void StyleEngine::set_element_presentational_hint_properties(StyleNodeID node, StyleEngineFFI::FfiElementDeclarationKind kind, ReadonlySpan<StyleProperty> properties)
{
    Vector<Parser::ValueParserFFI::FfiDeclaredProperty> declarations;
    declarations.ensure_capacity(properties.size());
    for (auto const& property : properties) {
        declarations.unchecked_append({
            .property_id = to_underlying(property.property_id),
            .important = property.important == Important::Yes,
            .value = property.value->rust_style_value_data(),
            .name = {},
        });
    }
    // The write carries a snapshot of the hints, whose one reference goes to the engine.
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementPresentationalHints, .value = to_underlying(kind), .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = bit_cast<FlatPtr>(Parser::ValueParserFFI::rust_declaration_data_from_views(declarations.data(), declarations.size())) });
    if (any_of(properties, [](auto const& property) { return property_defines_a_css_transition(property.property_id); }))
        note_css_transitions_may_observe_style_changes();
}

StyleEngine::StyleRecordDelta StyleEngine::publish_computed_groups(StyleNodeID node, u8 pseudo_kind, ReadonlySpan<void const*> payloads, size_t inherited_group_count, u64 custom_property_environment, bool inherited_group_swap_candidate, u64 counter_style_environment_identity, u64 animation_overlay_identity, void const* animated_overlay, ReadonlySpan<void const*> animation_overlay_payloads, void const* computed_longhand_table, void const* custom_property_store)
{
    VERIFY(inherited_group_count <= payloads.size());
    auto delta = StyleEngineFFI::style_engine_publish_computed_groups(rust_handle(), node.value(), pseudo_kind, payloads.data(), payloads.size(), inherited_group_count, custom_property_environment, inherited_group_swap_candidate, counter_style_environment_identity, animation_overlay_identity, animated_overlay, animation_overlay_payloads.data(), animation_overlay_payloads.size(), computed_longhand_table, custom_property_store);
    return { StyleRecordID { delta.old_style_record }, StyleRecordID { delta.new_style_record } };
}

RefPtr<PublishedStyleRecord const> StyleEngine::publish_style_record(StyleDrainScope const& scope, StyleRecordID style_record) const
{
    if (!style_record)
        return nullptr;
    return PublishedStyleRecord::adopt(StyleEngineFFI::style_engine_publish_style_record_in_drain(scope, rust_handle(), style_record.value()));
}

StyleEngineFFI::FfiRecordDemandAnswer StyleEngine::answer_read_demand(StyleNodeID node, u8 pseudo_kind, bool exclude_inline_style, bool targeted, bool read_only, StyleRecordID parent_highlight)
{
    auto* layout_arena = m_style_computer ? m_style_computer->document().layout_arena_handle() : nullptr;
    return StyleEngineFFI::style_engine_answer_read_demand(rust_handle(), layout_arena, node.value(), pseudo_kind, exclude_inline_style, targeted, read_only, parent_highlight.value());
}

StyleEngine::SettledAnimationDefinitions StyleEngine::take_settled_animation_definitions(StyleDrainScope const& scope, StyleNodeID node, u8 pseudo_kind)
{
    auto taken = StyleEngineFFI::style_engine_take_settled_animation_definitions(scope, rust_handle(), node.value(), pseudo_kind);
    return {
        .definitions = { static_cast<ComputedValuesFFI::FfiComputedAnimation const*>(taken.definitions), taken.count },
        .owed = taken.owed,
        .in_display_none_subtree = taken.in_display_none_subtree,
    };
}

void StyleEngine::pin_style_record(StyleRecordID style_record) const
{
    StyleEngineFFI::style_record_host_pins_pin(m_host_style_record_pins, style_record.value());
}

void StyleEngine::unpin_style_record(StyleRecordID style_record) const
{
    StyleEngineFFI::style_record_host_pins_unpin(m_host_style_record_pins, style_record.value());
}

void StyleEngine::begin_pin_waiting_for_frame() const
{
    StyleEngineFFI::style_record_host_pins_begin_pin_waiting_for_frame(m_host_style_record_pins);
}

void StyleEngine::end_pin_waiting_for_frame() const
{
    StyleEngineFFI::style_record_host_pins_end_pin_waiting_for_frame(m_host_style_record_pins);
}

void StyleEngine::begin_style_record_view_epoch()
{
    StyleEngineFFI::style_engine_begin_style_record_view_epoch(rust_handle());
}

void StyleEngine::end_style_record_view_epoch()
{
    StyleEngineFFI::style_engine_end_style_record_view_epoch(rust_handle());
}

void StyleEngine::decide_transitions(StyleRecordID before_style_record, void const* after_longhand_table, void const* after_animated_overlay, StyleValueFFI::FfiTransitionInput& input, StyleValueFFI::FfiTransitionAction* actions) const
{
    StyleValueFFI::rust_decide_transitions(rust_handle(), before_style_record.value(), after_longhand_table, after_animated_overlay, &input, actions);
}

StyleEngine::StyleRecordDelta StyleEngine::remove_computed_pseudo(StyleNodeID node, u8 pseudo_kind)
{
    auto delta = StyleEngineFFI::style_engine_remove_computed_pseudo(rust_handle(), node.value(), pseudo_kind);
    return { StyleRecordID { delta.old_style_record }, StyleRecordID { delta.new_style_record } };
}

StyleAtomID StyleEngine::intern_atom(Utf16FlyString const& name)
{
    // Utf16FlyString is already interned, so its one-word raw form is the name's identity. The atom
    // itself is assigned by the process-wide table selector names intern into as well: two tables
    // keyed by the same word but each assigning its own sequence would compare unequal for the same
    // name, which fails to match silently rather than loudly.
    // First time seen, the leaked reference is kept so the identity cannot be reused while the
    // atom is live. Duplicates release their new reference and return without crossing the FFI.
    auto raw = name.to_raw_leaked();
    if (auto atom = m_atoms.get(raw); atom.has_value()) {
        Utf16FlyString::unref_raw(raw);
        return atom.release_value();
    }
    // The table is shared and locked, so acquiring the atom writes nothing of this document's. The
    // document takes the atom with the next transaction.
    auto atom = StyleAtomID { StyleEngineFFI::style_engine_acquire_host_atom(m_recording_stream, raw) };
    m_host_fact_writes.append({ .kind = StyleEngineFFI::FfiHostFactKind::AdoptAtom, .value = 0, .node = 0, .parent = 0, .previous_sibling = 0, .facts = atom.value(), .data = raw });
    ++m_pending_atom_adoption_count;
    m_atoms.set(raw, atom);
    return atom;
}

// A qualified name is acquired as intern_atom() acquires a plain one: from the shared table, for the document
// to take with its next transaction, so a name first seen beside a style pass waits for no pass.
StyleAtomID StyleEngine::acquire_qualified_atom(StyleAtomID namespace_atom, StyleAtomID name_atom)
{
    auto atom = StyleAtomID { StyleEngineFFI::style_engine_acquire_host_qualified_atom(m_recording_stream, namespace_atom.value(), name_atom.value()) };
    m_host_fact_writes.append({ .kind = StyleEngineFFI::FfiHostFactKind::AdoptQualifiedAtom, .value = 0, .node = namespace_atom.value(), .parent = name_atom.value(), .previous_sibling = 0, .facts = atom.value(), .data = 0 });
    ++m_pending_atom_adoption_count;
    return atom;
}

void const* StyleEngine::borrow_engine_custom_property_environment(u64 identity, u64& parent_identity) const
{
    return StyleEngineFFI::style_engine_borrow_engine_custom_property_environment(rust_handle(), identity, &parent_identity);
}

StyleAtomID StyleEngine::intern_text_atom(Utf16View text)
{
    return intern_atom(Utf16FlyString::from_utf16(text).to_ascii_lowercase());
}

StyleAtomID StyleEngine::intern_language_atom(Utf16View text)
{
    auto atom = intern_text_atom(text);
    if (atom == 0 || text.is_empty() || m_published_language_atoms.set(atom) != AK::HashSetResult::InsertedNewEntry)
        return atom;

    record_element_language_write(0, atom, text);
    return atom;
}

StyleAtomID StyleEngine::intern_case_sensitive_text_atom(Utf16View text)
{
    return intern_atom(Utf16FlyString::from_utf16(text));
}

// The name an attribute is published under, and the any-namespace name it shares.
//
// Three selectors ask three different questions of an attribute called `x`. `[ns|x]` reaches only
// the one in that namespace, `[x]` reaches only the one in no namespace - which is what the bare
// local name is - and `[*|x]` reaches whichever of them the element carries. The first two name
// exactly one of an element's attributes, so they are the key: an element can hold `x` in several
// namespaces at once, and each is a fact with its own value. `[*|x]` asks about all of them
// together, so the shared form is published as an identity of the name rather than as a fact of its
// own, and one entry per attribute answers all three.
StyleAtomID StyleEngine::intern_attribute_name(Utf16FlyString const& local_name, Optional<Utf16FlyString> const& namespace_uri)
{
    auto local = intern_atom(local_name);
    auto namespace_atom = !namespace_uri.has_value() || namespace_uri->is_empty()
        ? StyleAtomID {}
        : intern_case_sensitive_text_atom(namespace_uri->view());
    auto& names_by_namespace = m_attribute_name_atoms.ensure(local, [] { return HashMap<StyleAtomID, StyleAtomID> {}; });
    if (auto name = names_by_namespace.get(namespace_atom); name.has_value())
        return name.release_value();

    auto in_namespace = [&](StyleAtomID name) {
        if (namespace_atom == 0)
            return name;
        return acquire_qualified_atom(namespace_atom, name);
    };
    auto any_namespace = acquire_qualified_atom(StyleEngine::any_namespace, local);
    auto name = in_namespace(local);

    StyleAtomID folded_name;
    StyleAtomID folded_local;
    auto folded = local_name.to_ascii_lowercase();
    if (folded != local_name) {
        auto folded_atom = intern_atom(folded);
        folded_name = in_namespace(folded_atom);
        folded_local = acquire_qualified_atom(StyleEngine::any_namespace, folded_atom);
    }

    auto local_name_view = local_name.view();
    Vector<u16> local_name_code_units;
    local_name_code_units.ensure_capacity(local_name_view.length_in_code_units());
    for (size_t i = 0; i < local_name_view.length_in_code_units(); ++i)
        local_name_code_units.unchecked_append(local_name_view.code_unit_at(i));
    // Beside a style pass the engine is the pass's: the forms wait for its drain with the attribute change that
    // names them.
    publish_input([name, any_namespace, folded_name, folded_local, local_name_code_units = move(local_name_code_units), has_no_namespace = namespace_atom == 0](StyleInputScope const& input) {
        input.engine().note_attribute_name_forms(name, any_namespace, folded_name, folded_local, local_name_code_units, has_no_namespace);
    });
    names_by_namespace.set(namespace_atom, name);
    m_attribute_names.set(name, { .folded_local_name = move(folded), .has_no_namespace = namespace_atom == 0, .value_text_readers = {} });
    return name;
}

// The reader bits RetainedState::attribute_value_text_readers() answers.
static constexpr u32 attribute_value_text_read_by_selectors = 1;

StyleAtomID StyleEngine::intern_attribute_value(StyleAtomID name, Utf16String const& value)
{
    auto atom = intern_atom(Utf16FlyString { value });
    // Beside a style pass the engine is the pass's: the text, and whether the name asks for it, wait for its drain
    // with the attribute change that names the value. Beside a layout pass, which reads what the engine holds, they
    // wait for the pass to be taken back.
    if (Layout::RustFFI::rust_stage_thread_style_pass_holds_style_engine(rust_handle()) || layout_pass_is_in_flight()) {
        publish_input([name, atom, value](StyleInputScope const& input) {
            auto& engine = input.engine();
            if (auto readers = engine.attribute_value_text_readers(name))
                engine.publish_attribute_value_text(atom, value, readers & attribute_value_text_read_by_selectors);
        });
        return atom;
    }
    if (auto readers = attribute_value_text_readers(name))
        publish_attribute_value_text(atom, value, readers & attribute_value_text_read_by_selectors);
    return atom;
}

void StyleEngine::backfill_attribute_value_text_if_required(StyleAtomID name, Utf16String const& value)
{
    auto readers = attribute_value_text_readers(name);
    if (!readers)
        return;

    auto atom = intern_atom(Utf16FlyString { value });
    publish_attribute_value_text(atom, value, readers & attribute_value_text_read_by_selectors);
}

void StyleEngine::publish_attribute_value_text(StyleAtomID atom, Utf16View value, bool read_by_selectors)
{
    // The engine holds one copy of the text per currently used value, and keeps the one it holds. Text an attr()
    // asked for is still news to the selectors once one of their names spells the same value.
    Vector<u16> code_units;
    code_units.ensure_capacity(value.length_in_code_units());
    for (size_t i = 0; i < value.length_in_code_units(); ++i)
        code_units.unchecked_append(value.code_unit_at(i));
    StyleEngineFFI::style_engine_set_attribute_value_text(rust_handle(), atom.value(), code_units.data(), code_units.size(), read_by_selectors);
}

bool StyleEngine::refresh_attribute_value_text_requirements()
{
    auto version = StyleEngineFFI::style_engine_attribute_value_text_requirements_version(rust_handle());
    if (version == m_attribute_value_text_requirements_version)
        return false;
    m_attribute_value_text_requirements_version = version;
    for (auto& it : m_attribute_names)
        it.value.value_text_readers = {};
    return true;
}

u32 StyleEngine::attribute_value_text_readers(StyleAtomID name)
{
    auto it = m_attribute_names.find(name);
    if (it == m_attribute_names.end())
        return 0;
    auto& attribute_name = it->value;
    if (!attribute_name.value_text_readers.has_value())
        attribute_name.value_text_readers = StyleEngineFFI::style_engine_attribute_value_text_readers(rust_handle(), attribute_name.folded_local_name.raw_identity(), attribute_name.has_no_namespace);
    return *attribute_name.value_text_readers;
}

void StyleEngine::record_element_language_write(u32 node, StyleAtomID language, Utf16View tag)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementLanguage, .value = 0, .node = node, .parent = 0, .previous_sibling = 0, .facts = language.value(), .data = m_host_fact_text_data.size() });
    m_host_fact_text_data.append(Utf16String::from_utf16(tag));
}

void StyleEngine::set_element_language(StyleNodeID node, StyleAtomID language, Utf16View tag)
{
    // A language range is not a name, so `:lang()` compares against the tag itself rather than
    // against the atom. The text is recorded once per language, not once per element.
    if (language == 0 || tag.is_empty() || m_published_language_atoms.set(language) != AK::HashSetResult::InsertedNewEntry)
        tag = {};
    record_element_language_write(node.value(), language, tag);
}

// Recording input gives the next rendering update style work to do, but touches no layout tree
// and no paintable, so nothing else asks the page for a frame. On a quiet document a change made
// from a timer would otherwise sit unflushed indefinitely, and a transition it should start would
// not run until something unrelated woke the rendering loop. Only the first input needs the poke:
// the frame it schedules flushes everything recorded before it runs.
static void note_recorded_input(StyleEngine const& style_engine, GC::Ptr<StyleComputer> style_computer)
{
    if (!style_computer)
        return;
    if (style_engine.has_recorded_input())
        return;
    style_computer->document().page().client().request_frame();
}

static void flush_deferred_geometry_transaction_before_non_replayable_input(StyleEngine const& style_engine, GC::Ptr<StyleComputer> style_computer)
{
    if (style_computer && style_engine.has_deferred_geometry_transaction())
        style_computer->document().flush_deferred_style_change_event();
}

void StyleEngine::note_pending_arrivals(size_t count)
{
    // An arrival is recorded as the input is next submitted, where the transaction a geometry read deferred must not
    // be waiting: it is flushed here, where the insertion would have recorded the arrival.
    flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
    note_recorded_input(*this, m_style_computer);
    m_pending_arrival_count += count;
}

void StyleEngine::record_tree_delta(StyleEngineFFI::FfiTreeDelta const& delta)
{
    flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
    note_recorded_input(*this, m_style_computer);
    m_tree_deltas.append(delta);
}

void StyleEngine::record_element_arrival(StyleEngineFFI::FfiElementArrival arrival, ReadonlySpan<StyleAtomID> custom_states)
{
    flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
    note_recorded_input(*this, m_style_computer);
    VERIFY(m_arrival_custom_state_atoms.size() <= NumericLimits<u32>::max());
    VERIFY(custom_states.size() <= NumericLimits<u32>::max());
    VERIFY(m_arrival_custom_state_atoms.size() + custom_states.size() <= NumericLimits<u32>::max());
    arrival.custom_state_offset = static_cast<u32>(m_arrival_custom_state_atoms.size());
    arrival.custom_state_count = static_cast<u32>(custom_states.size());
    for (auto state : custom_states)
        m_arrival_custom_state_atoms.append(state.value());
    m_element_arrivals.append(arrival);
}

void StyleEngine::record_local_feature_delta(StyleEngineFFI::FfiLocalFeatureDelta const& delta)
{
    note_recorded_input(*this, m_style_computer);
    m_local_feature_deltas.append(delta);
}

void StyleEngine::record_state_delta(StyleEngineFFI::FfiStateDelta const& delta)
{
    note_recorded_input(*this, m_style_computer);
    m_state_deltas.append(delta);
}

void StyleEngine::record_element_declaration_delta(StyleEngineFFI::FfiElementDeclarationDelta const& delta)
{
    flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
    note_recorded_input(*this, m_style_computer);
    m_element_declaration_deltas.append(delta);
}

void StyleEngine::record_host_fact_write(StyleEngineFFI::FfiHostFactWrite write)
{
    // A text arrival alone is five writes. The first since the last transaction is the one that
    // needs a frame, and the mutation behind each of them notes the render state change itself.
    if (m_host_fact_writes.size() == m_pending_atom_adoption_count)
        note_recorded_input(*this, m_style_computer);
    // Inserting markup records thousands of writes in one transaction, and growing by a quarter at a time copies
    // them over and over.
    if (m_host_fact_writes.size() == m_host_fact_writes.capacity())
        m_host_fact_writes.ensure_capacity(max<size_t>(64, m_host_fact_writes.capacity() * 2));
    m_host_fact_writes.append(write);
}

void StyleEngine::record_dom_order_links(ReadonlySpan<u32> links)
{
    VERIFY(links.size() % 3 == 0);
    for (size_t i = 0; i < links.size(); i += 3)
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::LinkInDomOrder, .value = 0, .node = links[i], .parent = links[i + 1], .previous_sibling = links[i + 2], .facts = 0, .data = 0 });
}

void StyleEngine::record_dom_order_unlink(StyleNodeID node, StyleNodeID parent)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::UnlinkFromDomOrder, .value = 0, .node = node.value(), .parent = parent.value(), .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_text_retirements(ReadonlySpan<StyleNodeID> nodes)
{
    for (auto node : nodes)
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::RetireText, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_unique_node_id(StyleNodeID node, u64 unique_node_id)
{
    static_assert(sizeof(FlatPtr) == sizeof(u64));
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementUniqueNodeId, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = static_cast<FlatPtr>(unique_node_id) });
}

void StyleEngine::record_shadow_root(StyleNodeID host, StyleNodeID shadow_root)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ShadowRoot, .value = 0, .node = host.value(), .parent = shadow_root.value(), .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_tree_scope_root(TreeScopeID tree_scope, StyleNodeID root)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::TreeScopeRoot, .value = 0, .node = root.value(), .parent = 0, .previous_sibling = 0, .facts = tree_scope.value(), .data = 0 });
}

void StyleEngine::record_tree_scope_uses_document_sheets(TreeScopeID tree_scope)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::TreeScopeUsesDocumentSheets, .value = 0, .node = 0, .parent = 0, .previous_sibling = 0, .facts = tree_scope.value(), .data = 0 });
}

void StyleEngine::note_slot_assignment_changed(StyleNodeID slot)
{
    if (!has_changed_node_lists())
        note_recorded_input(*this, m_style_computer);
    m_slots_whose_assignment_changed.set(slot);
}

void StyleEngine::note_top_layer_changed()
{
    if (!has_changed_node_lists())
        note_recorded_input(*this, m_style_computer);
    m_top_layer_changed = true;
}

void StyleEngine::record_slot_assigned_nodes(StyleNodeID slot, ReadonlySpan<StyleNodeID> assigned)
{
    if (assigned.is_empty()) {
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::SlotAssignedNode, .value = 1, .node = slot.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
        return;
    }
    for (size_t index = 0; index < assigned.size(); ++index)
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::SlotAssignedNode, .value = index == 0, .node = slot.value(), .parent = assigned[index].value(), .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_top_layer_elements(ReadonlySpan<StyleNodeID> elements)
{
    if (elements.is_empty()) {
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::TopLayerElement, .value = 1, .node = 0, .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
        return;
    }
    for (size_t index = 0; index < elements.size(); ++index)
        record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::TopLayerElement, .value = index == 0, .node = elements[index].value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_size_query_container(StyleNodeID node)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::SizeQueryContainer, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_style_depends_on_size_container_query(StyleNodeID node)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::StyleDependsOnSizeContainerQuery, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_recomputes_on_environment_move(StyleNodeID node)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::RecomputesOnEnvironmentMove, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_size_container_needs_evaluation_after_layout(StyleNodeID node)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::SizeContainerNeedsEvaluationAfterLayout, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_children_explicitly_inherit(StyleNodeID node)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ChildrenExplicitlyInherit, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_rule_conditions_hold(u64 rule_identity, bool holds)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::RuleConditionsHold, .value = holds, .node = 0, .parent = 0, .previous_sibling = 0, .facts = 0, .data = rule_identity });
}

void StyleEngine::record_dom_paint_facts(StyleNodeID node, u8 facts)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::NodeDomPaintFacts, .value = facts, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_table_spans(StyleNodeID node, u16 column_span, u16 row_span, u32 raw_column_span)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementTableSpans, .value = 0, .node = node.value(), .parent = raw_column_span, .previous_sibling = 0, .facts = static_cast<u32>(column_span) | (static_cast<u32>(row_span) << 16), .data = 0 });
}

void StyleEngine::record_text_is_ascii_whitespace(StyleNodeID node, bool value)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::TextIsAsciiWhitespace, .value = value, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_text_is_in_user_agent_shadow_tree(StyleNodeID node, bool value)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::TextIsInUserAgentShadowTree, .value = value, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_text_is_password_input(StyleNodeID node, bool value)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::TextIsPasswordInput, .value = value, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_text_data(StyleNodeID node, Utf16String const& data)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::TextData, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = m_host_fact_text_data.size() });
    m_host_fact_text_data.append(data);
}

void StyleEngine::record_adjustment_facts(StyleNodeID node, u32 facts)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementAdjustmentFacts, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = facts, .data = 0 });
}

void StyleEngine::record_associated_pseudo_kind(StyleNodeID node, u8 pseudo_kind_plus_one)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementAssociatedPseudoKind, .value = pseudo_kind_plus_one, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

void StyleEngine::record_construction_facts(StyleNodeID node, u32 facts, u8 box_kind)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementConstructionFacts, .value = box_kind, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = facts, .data = 0 });
}

void StyleEngine::record_replaced_content_input(StyleNodeID node, StyleEngineFFI::FfiReplacedContentInput const& input)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementReplacedContentInput, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = m_host_fact_replaced_content_inputs.size() });
    m_host_fact_replaced_content_inputs.append(input);
}

void StyleEngine::record_inline_style_properties(StyleNodeID node, Parser::ValueParserFFI::DeclarationBlockData const* declarations)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ElementInlineStyleProperties, .value = 0, .node = node.value(), .parent = 0, .previous_sibling = 0, .facts = 0, .data = bit_cast<FlatPtr>(declarations) });
}

void StyleEngine::record_container_query_input_change(StyleNodeID style_node)
{
    if (style_node == 0)
        return;

    flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
    note_recorded_input(*this, m_style_computer);
    record_container_query_input(style_node);
}

void StyleEngine::record_size_container_query_dependents(StyleNodeID container)
{
    if (container == 0)
        return;
    flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
    note_recorded_input(*this, m_style_computer);
    StyleEngineFFI::style_engine_record_size_container_query_dependents(rust_handle(), container.value());
}

void StyleEngine::record_derived_element_style_input_change(StyleNodeID style_node, u8 reaction, u8 inherited_style_groups)
{
    if (style_node != 0 && reaction != 0) {
        flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
        note_recorded_input(*this, m_style_computer);
        record_derived_element_style_input(style_node, reaction, inherited_style_groups);
    }
}

void StyleEngine::record_tree_counting_style_input_change(StyleNodeID style_node)
{
    if (style_node == 0)
        return;
    flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
    note_recorded_input(*this, m_style_computer);
    record_tree_counting_style_input(style_node);
}

void StyleEngine::record_flat_tree_descendant_style_input_changes(StyleNodeID style_node, u8 reaction, u8 inherited_style_groups)
{
    if (style_node == 0 || reaction == 0)
        return;

    flush_deferred_geometry_transaction_before_non_replayable_input(*this, m_style_computer);
    // The relation columns must include every tree delta recorded before this derived action.
    submit_recorded_input();
    note_recorded_input(*this, m_style_computer);
    record_flat_tree_descendant_style_inputs(style_node, reaction, inherited_style_groups);
}

void StyleEngine::record_viewport_dependent_style_inputs(u8 reaction)
{
    record_host_fact_write({ .kind = StyleEngineFFI::FfiHostFactKind::ViewportDependentStyleInputs, .value = reaction, .node = 0, .parent = 0, .previous_sibling = 0, .facts = 0, .data = 0 });
}

bool StyleEngine::has_recorded_element_style_input_change(StyleNodeID style_node) const
{
    return has_deferred_element_style_input(style_node);
}
void StyleEngine::record_benchmark_marker(Utf16View name)
{
    auto const* data = name.has_ascii_storage()
        ? static_cast<void const*>(name.bytes().data())
        : static_cast<void const*>(name.utf16_span().data());
    StyleEngineFFI::style_engine_record_benchmark_marker(rust_handle(), data, name.length_in_code_units(), name.has_ascii_storage());
}

bool StyleEngine::has_recorded_input() const
{
    return m_pending_arrival_count > 0 || has_journaled_input();
}

bool StyleEngine::has_journaled_input() const
{
    return !m_tree_deltas.is_empty()
        || !m_element_arrivals.is_empty()
        || !m_local_feature_deltas.is_empty()
        || !m_state_deltas.is_empty()
        || !m_element_declaration_deltas.is_empty()
        || has_changed_node_lists()
        // An atom's adoption is no input to style.
        || m_host_fact_writes.size() > m_pending_atom_adoption_count;
}

// A page that inserts markup records thousands of writes per transaction: the buffers keep their capacity for the next
// one. What the host recorded since the transaction took them moves into them, where a write's index into its side
// buffer still finds what it names.
template<typename Buffer>
static void reuse_journal_buffer(Buffer& member, Buffer& buffer)
{
    buffer.clear_with_capacity();
    buffer.extend(move(member));
    member = move(buffer);
}

void StyleEngine::submit_recorded_input(RecordedInputGoesTo goes_to)
{
    // The recorded input is the next transaction's journal. While a pass is in flight it stays there.
    if (pass_is_in_flight() || m_holds_input_recorded_beside_pass)
        return;
    if (m_style_computer) {
        take_in_pending_style_arrivals(m_style_computer->document());
        record_changed_node_lists(m_style_computer->document(), *this);
    }
    StyleInputScope const input { *this };
    if (m_style_computer)
        publish_pending_element_features(*this, *m_style_computer);
    // Nodes still waiting to arrive when a read decided it does not need them go in with a later submission.
    if (!has_journaled_input() && m_host_fact_writes.is_empty() && !m_style_node_grant_request && !m_text_style_node_grant_request) {
        if (refresh_attribute_value_text_requirements() && m_style_computer)
            publish_required_attribute_value_texts(*this, *m_style_computer);
        return;
    }

    // A selector that came to read attribute value texts has the texts published after the input goes in, before
    // matching reads them, which a pass that applies the input itself would not wait for.
    if (goes_to == RecordedInputGoesTo::Transaction && StyleEngineFFI::style_engine_attribute_value_text_requirements_version(rust_handle()) != m_attribute_value_text_requirements_version)
        goes_to = RecordedInputGoesTo::Engine;

    // What the engine calls back into while it applies these records the next transaction's.
    auto host_fact_writes = move(m_host_fact_writes);
    auto host_fact_text_data = move(m_host_fact_text_data);
    auto host_fact_replaced_content_inputs = move(m_host_fact_replaced_content_inputs);
    m_pending_atom_adoption_count = 0;
    // Each text data write hands the engine one reference to what it holds, and each replaced
    // content input write lends it the input for the call.
    for (auto& write : host_fact_writes) {
        if (write.kind == StyleEngineFFI::FfiHostFactKind::TextData || write.kind == StyleEngineFFI::FfiHostFactKind::ElementLanguage)
            write.data = host_fact_text_data[write.data].to_raw_leaked();
        else if (write.kind == StyleEngineFFI::FfiHostFactKind::ElementReplacedContentInput)
            write.data = bit_cast<FlatPtr>(&host_fact_replaced_content_inputs[write.data]);
    }
    // The grant answers the transaction. It is taken aside until the call returns, because the host may
    // mint from what it was granted before while the engine calls back into it.
    Vector<StyleNodeID> style_node_grant;
    Vector<StyleNodeID> text_style_node_grant;
    style_node_grant.resize(exchange(m_style_node_grant_request, 0));
    text_style_node_grant.resize(exchange(m_text_style_node_grant_request, 0));

    if (goes_to == RecordedInputGoesTo::Transaction) {
        // The transaction copies the input as it is handed over, and applies it where its pass runs.
        m_recorded_input_for_pass = RecordedInputForPass {
            .tree_deltas = move(m_tree_deltas),
            .element_arrivals = move(m_element_arrivals),
            .arrival_custom_state_atoms = move(m_arrival_custom_state_atoms),
            .local_feature_deltas = move(m_local_feature_deltas),
            .state_deltas = move(m_state_deltas),
            .element_declaration_deltas = move(m_element_declaration_deltas),
            .host_fact_writes = move(host_fact_writes),
            .host_fact_text_data = move(host_fact_text_data),
            .host_fact_replaced_content_inputs = move(host_fact_replaced_content_inputs),
            .style_node_grant = move(style_node_grant),
            .text_style_node_grant = move(text_style_node_grant),
        };
        return;
    }

    InputTransaction transaction {
        .tree_deltas = m_tree_deltas.data(),
        .tree_delta_count = m_tree_deltas.size(),
        .element_arrivals = m_element_arrivals.data(),
        .element_arrival_count = m_element_arrivals.size(),
        .arrival_custom_state_atoms = m_arrival_custom_state_atoms.data(),
        .arrival_custom_state_atom_count = m_arrival_custom_state_atoms.size(),
        .local_feature_deltas = m_local_feature_deltas.data(),
        .local_feature_delta_count = m_local_feature_deltas.size(),
        .state_deltas = m_state_deltas.data(),
        .state_delta_count = m_state_deltas.size(),
        .element_declaration_deltas = m_element_declaration_deltas.data(),
        .element_declaration_delta_count = m_element_declaration_deltas.size(),
        .element_style_inputs = nullptr,
        .element_style_input_count = 0,
        .host_fact_writes = host_fact_writes.data(),
        .host_fact_write_count = host_fact_writes.size(),
        .element_identity_grant = reinterpret_cast<u32*>(style_node_grant.data()),
        .element_identity_grant_count = style_node_grant.size(),
        .text_identity_grant = reinterpret_cast<u32*>(text_style_node_grant.data()),
        .text_identity_grant_count = text_style_node_grant.size(),
    };
    apply_transaction(input, transaction);
    adopt_identity_grant(m_granted_style_nodes, style_node_grant);
    adopt_identity_grant(m_granted_text_style_nodes, text_style_node_grant);
    reuse_journal_buffer(m_host_fact_writes, host_fact_writes);
    reuse_journal_buffer(m_host_fact_text_data, host_fact_text_data);
    reuse_journal_buffer(m_host_fact_replaced_content_inputs, host_fact_replaced_content_inputs);

    m_tree_deltas.clear_with_capacity();
    m_element_arrivals.clear_with_capacity();
    m_arrival_custom_state_atoms.clear_with_capacity();
    m_local_feature_deltas.clear_with_capacity();
    m_state_deltas.clear_with_capacity();
    m_element_declaration_deltas.clear_with_capacity();

    // Selector demand can arrive while the program change and element facts are still staged.
    // Refresh after applying the fact batch, then backfill values before matching observes it.
    if (refresh_attribute_value_text_requirements() && m_style_computer)
        publish_required_attribute_value_texts(*this, *m_style_computer);
}

void StyleEngine::apply_transaction(StyleInputScope const& input, InputTransaction const& transaction)
{
    StyleEngineFFI::style_engine_apply_transaction(input, rust_handle(), &transaction);
}

void StyleEngine::flush()
{
    submit_recorded_input();
    StyleEngineFFI::style_engine_flush(rust_handle());
}

void StyleEngine::flush_without_document_root()
{
    flush();
    // Without a root there is no transaction to take them, as the engine holds back its element style inputs.
    m_install_feedback_held_back = true;
}

bool StyleEngine::take_diagnostic_style_transaction(StyleNodeID root, Function<void(ReadonlySpan<StyleNodeID>)>&& consume)
{
    Vector<StyleNodeID> reaction_nodes;
    auto transaction = take_style_transaction(root);
    auto take_reaction_nodes = [&](auto const& reactions) {
        for (auto const& reaction : reactions) {
            // A pseudo-element record is part of its element's reaction.
            if (reaction.pseudo_kind != NumericLimits<u8>::max())
                continue;
            // A row that joined the pass for a reaction another row derived for it, or the batch for
            // an element its rows inherit from, is no row the transaction planned.
            if (reaction.record_damage & (to_underlying(StyleEngineFFI::FfiStyleInvalidationField::JoinedByDerivation) | to_underlying(StyleEngineFFI::FfiStyleInvalidationField::JoinedForInheritance)))
                continue;
            // Nor is a record an environment move republished as the pass settled a row above it.
            if (reaction.gap == StyleEngineFFI::FfiStyleDeltaGap::EnvironmentMoved)
                continue;
            reaction_nodes.append(StyleNodeID { reaction.style_node });
        }
    };
    take_reaction_nodes(transaction.reactions);
    // A pass the host would install in waves reports every wave. The reactions its rows derived
    // for children outside it wait for the next transaction, which is no part of this one.
    while (StyleEngineFFI::style_engine_has_suspended_style_pass(rust_handle())) {
        auto wave = take_style_transaction(root);
        if (wave.reactions.is_empty())
            break;
        take_reaction_nodes(wave.reactions);
    }
    // The host installs nothing a diagnostic transaction published: draining it discards its outputs.
    StyleEffectDrain::install(m_style_computer->document(), [](StyleDrainScope const& scope) {
        scope.engine().discard_style_transaction_outputs(scope);
    });
    if (!transaction.is_scoped)
        return false;
    consume(reaction_nodes.span());
    return true;
}

void StyleEngine::discard_style_transaction_outputs(StyleDrainScope const& scope)
{
    StyleEngineFFI::style_engine_discard_style_transaction_outputs(scope, rust_handle());
}

namespace {

struct StyleSheetResourceContextCollection {
    DOM::Document& document;
    // The base URL of the document's sheets that have neither a base URL nor a location of their
    // own, which StyleSheetState::style_resource_base_url() resolves against the document.
    String const& document_api_base_url;
    HashTable<StyleSheetState const*> collected {};
    Vector<CollectedStyleSheetResourceContext> contexts {};
};

Optional<String> style_resource_base_url(StyleSheetState const& sheet, StyleSheetResourceContextCollection const& collection)
{
    if (!sheet.base_url().has_value() && !sheet.location().has_value() && sheet.owning_document().ptr() == &collection.document)
        return collection.document_api_base_url;
    if (auto base_url = sheet.style_resource_base_url(); base_url.has_value())
        return base_url->to_string();
    return {};
}

void collect_style_sheet_resource_context(StyleSheetState& sheet, StyleSheetResourceContextCollection& collection)
{
    // A sheet's context is its own, whichever scope reaches it: a constructed sheet adopted by many
    // shadow roots is collected once.
    if (collection.collected.set(&sheet) != AK::HashSetResult::InsertedNewEntry)
        return;
    auto base_url = style_resource_base_url(sheet, collection);
    collection.contexts.append({
        .source_identity = Parser::ValueParserFFI::rust_style_sheet_identity(sheet.native_sheet().handle()),
        .base_url = base_url.value_or(String {}),
        .has_base_url = base_url.has_value(),
        .origin_clean = sheet.is_origin_clean(),
    });
    for (auto const& import : sheet.import_rules()) {
        if (auto* imported = import->loaded_style_sheet())
            collect_style_sheet_resource_context(*imported, collection);
    }
    // A sheet whose rules are compiled from a shared snapshot has those rules name the snapshot's
    // native sheet, which shares the sheet's base URL.
    if (auto* shared = sheet.shared_compiled_style_sheet(); shared && &shared->contents() != &sheet)
        collect_style_sheet_resource_context(shared->contents(), collection);
}

Vector<CollectedStyleSheetResourceContext> collect_style_sheet_resource_contexts(DOM::Document& document, String const& document_api_base_url)
{
    StyleSheetResourceContextCollection collection { .document = document, .document_api_base_url = document_api_base_url };
    Function<void(StyleSheetState&)> collect = [&](StyleSheetState& sheet) { collect_style_sheet_resource_context(sheet, collection); };
    for (auto origin : { CascadeOrigin::UserAgent, CascadeOrigin::User, CascadeOrigin::Author })
        document.style_scope().for_each_stylesheet(origin, collect);
    document.for_each_shadow_root([&](DOM::ShadowRoot& shadow_root) {
        shadow_root.style_scope().for_each_stylesheet(CascadeOrigin::Author, collect);
    });
    return move(collection.contexts);
}

}

// Readies the document's inputs to a style transaction and lends them to `take`, which hands them to the engine
// with the document's layout arena.
void StyleEngine::lend_style_transaction_inputs(RecordedInputGoesTo recorded_input_goes_to, Function<void(StyleEngineFFI::FfiDocumentStyleComputationInputs const&, void* layout_arena, InputTransaction const* input)> const& take)
{
    submit_recorded_input(recorded_input_goes_to);
    publish_font_faces();
    StyleEngineFFI::FfiDocumentStyleComputationInputs computation_inputs {};
    // Lent to the engine for the call below, which copies them.
    String document_base_url;
    Vector<StyleEngineFFI::FfiStyleSheetResourceContextEntry> resource_contexts;
    Vector<StyleEngineFFI::FfiCustomFunctionEntry> custom_functions;
    if (m_style_computer) {
        auto const viewport_rect = m_style_computer->viewport_rect_for_style_environment();
        auto const* media_environment = m_style_computer->ensure_media_environment_for_style_update();
        // The media snapshot may update the active @function definitions. Publish the resulting
        // CSSOM registry only after that update, before any custom-property row resolves it.
        HashTable<StyleScope const*> visited;
        struct FunctionScope {
            StyleScope const* scope;
            u32 tree_scope;
        };
        Vector<FunctionScope> scopes;
        auto append_scope = [&](StyleScope const& scope, u32 tree_scope) {
            if (visited.set(&scope) == AK::HashSetResult::InsertedNewEntry)
                scopes.append({ &scope, tree_scope });
        };
        auto& document = m_style_computer->document();
        append_scope(document.style_scope(), document.style_scope().style_engine_tree_scope().value());
        document.for_each_shadow_root([&](DOM::ShadowRoot& shadow_root) {
            auto& scope = shadow_root.style_scope();
            append_scope(scope, scope.style_engine_tree_scope().value());
        });
        for (size_t index = 0; index < scopes.size(); ++index) {
            auto const& scope = *scopes[index].scope;
            auto tree_scope = scopes[index].tree_scope;
            scope.for_each_visible_function_definition([&](StyleScope::FunctionDefinitionAndScope const& definition) {
                custom_functions.append({ .function = definition.function.handle(), .caller_scope = bit_cast<FlatPtr>(&scope), .definition_scope = bit_cast<FlatPtr>(&definition.scope), .tree_scope = tree_scope });
                append_scope(definition.scope, NumericLimits<u32>::max());
            });
        }
        auto const& root_font_metrics = m_style_computer->root_element_font_metrics();
        auto const& initial_font = m_style_computer->document().font_computer().initial_font();
        Length::FontMetrics const initial_font_metrics { CSSPixels { initial_font.pixel_size() }, initial_font.pixel_metrics(), InitialValues::line_height() };
        document_base_url = document.serialized_base_url();
        auto document_api_base_url = HTML::relevant_settings_object(document).api_base_url().to_string();
        if (!m_style_sheet_resource_contexts.has_value()
            || m_style_sheet_resource_contexts->style_sheet_set_generation != document.style_sheet_set_generation()
            || m_style_sheet_resource_contexts->document_api_base_url != document_api_base_url) {
            m_style_sheet_resource_contexts = StyleSheetResourceContexts {
                .contexts = collect_style_sheet_resource_contexts(document, document_api_base_url),
                .style_sheet_set_generation = document.style_sheet_set_generation(),
                .document_api_base_url = move(document_api_base_url),
            };
        }
        auto const& collected_resource_contexts = m_style_sheet_resource_contexts->contexts;
        resource_contexts.ensure_capacity(collected_resource_contexts.size());
        for (auto const& context : collected_resource_contexts) {
            resource_contexts.unchecked_append({
                .source_identity = context.source_identity,
                .base_url = context.base_url.bytes().data(),
                .base_url_length = context.base_url.bytes().size(),
                .has_base_url = context.has_base_url,
                .origin_clean = context.origin_clean,
            });
        }
        computation_inputs = {
            .in_quirks_mode = m_style_computer->document().in_quirks_mode(),
            .viewport_width = viewport_rect.width().to_double(),
            .viewport_height = viewport_rect.height().to_double(),
            .root_font_size = root_font_metrics.font_size.to_double(),
            .root_font_x_height = root_font_metrics.x_height.to_double(),
            .root_font_cap_height = root_font_metrics.cap_height.to_double(),
            .root_font_zero_advance = root_font_metrics.zero_advance.to_double(),
            .root_line_height = root_font_metrics.line_height.to_double(),
            .root_font_metrics_depend_on_viewport_metrics = m_style_computer->root_element_font_metrics_depend_on_viewport_metrics(),
            .initial_font_size = initial_font_metrics.font_size.to_double(),
            .initial_font_x_height = initial_font_metrics.x_height.to_double(),
            .initial_font_cap_height = initial_font_metrics.cap_height.to_double(),
            .initial_font_zero_advance = initial_font_metrics.zero_advance.to_double(),
            .initial_font_size_raw = InitialValues::font_size().raw_value(),
            .default_font_size_raw = StyleComputer::default_user_font_size().raw_value(),
            .device_pixels_per_css_pixel = m_style_computer->document().page().client().device_pixels_per_css_pixel(),
            .font_environment_generation = m_style_computer->document().font_computer().environment_generation(),
            .style_environment_version = m_style_computer->style_environment_version_for_sharing(),
            .preferred_color_scheme = static_cast<u8>(to_underlying(m_style_computer->document().page().preferred_color_scheme())),
            .has_document_supported_schemes = false,
            .document_supported_scheme_count = 0,
            .document_supported_scheme_codes = {},
            .custom_property_registry = reinterpret_cast<StyleEngineFFI::FfiHostHandle>(m_style_computer->document().rust_custom_property_registry()),
            .custom_property_registration_generation = m_style_computer->document().custom_property_registration_generation(),
            .document_base_url = reinterpret_cast<StyleEngineFFI::FfiHostHandle>(document_base_url.bytes().data()),
            .document_base_url_length = document_base_url.bytes().size(),
            .style_sheet_resource_contexts = reinterpret_cast<StyleEngineFFI::FfiHostHandle>(resource_contexts.data()),
            .style_sheet_resource_context_count = resource_contexts.size(),
            .media_feature_values = reinterpret_cast<StyleEngineFFI::FfiHostHandle>(media_environment->values),
            .media_feature_value_count = media_environment->value_count,
            .media_length_resolution_context = reinterpret_cast<StyleEngineFFI::FfiHostHandle>(media_environment->length_resolution_context),
            .custom_functions = reinterpret_cast<StyleEngineFFI::FfiHostHandle>(custom_functions.data()),
            .custom_function_count = custom_functions.size(),
        };
        if (auto supported = m_style_computer->document().supported_color_schemes(); supported.has_value()) {
            computation_inputs.has_document_supported_schemes = true;
            for (auto const& scheme : *supported) {
                auto preferred_scheme = preferred_color_scheme_from_string(scheme);
                if (preferred_scheme == PreferredColorScheme::Auto)
                    continue;
                auto code = static_cast<u8>(to_underlying(preferred_scheme));
                auto supported_codes = Span<u8> { computation_inputs.document_supported_scheme_codes };
                if (supported_codes.trim(computation_inputs.document_supported_scheme_count).contains_slow(code))
                    continue;
                VERIFY(computation_inputs.document_supported_scheme_count < supported_codes.size());
                supported_codes[computation_inputs.document_supported_scheme_count++] = code;
            }
        }
    }
    // The render owner runs the transaction with the document's render state, which the arena is
    // created with. A sample the pass takes resolves a percentage translation against the boxes
    // the last layout committed.
    auto* layout_arena = m_style_computer ? Layout::document_layout_arena(m_style_computer->document()) : nullptr;
    // The transaction takes the reactions the host applied along, whatever else it takes.
    ScopeGuard install_feedback_went = [&] {
        m_applied_style_reactions.clear_with_capacity();
        m_pseudo_element_settles.clear_with_capacity();
        m_install_feedback_held_back = false;
    };
    if (!m_recorded_input_for_pass.has_value()) {
        take(computation_inputs, layout_arena, nullptr);
        return;
    }

    auto& input = *m_recorded_input_for_pass;
    InputTransaction transaction {
        .tree_deltas = input.tree_deltas.data(),
        .tree_delta_count = input.tree_deltas.size(),
        .element_arrivals = input.element_arrivals.data(),
        .element_arrival_count = input.element_arrivals.size(),
        .arrival_custom_state_atoms = input.arrival_custom_state_atoms.data(),
        .arrival_custom_state_atom_count = input.arrival_custom_state_atoms.size(),
        .local_feature_deltas = input.local_feature_deltas.data(),
        .local_feature_delta_count = input.local_feature_deltas.size(),
        .state_deltas = input.state_deltas.data(),
        .state_delta_count = input.state_deltas.size(),
        .element_declaration_deltas = input.element_declaration_deltas.data(),
        .element_declaration_delta_count = input.element_declaration_deltas.size(),
        .element_style_inputs = nullptr,
        .element_style_input_count = 0,
        .host_fact_writes = input.host_fact_writes.data(),
        .host_fact_write_count = input.host_fact_writes.size(),
        .element_identity_grant = reinterpret_cast<u32*>(input.style_node_grant.data()),
        .element_identity_grant_count = input.style_node_grant.size(),
        .text_identity_grant = reinterpret_cast<u32*>(input.text_style_node_grant.data()),
        .text_identity_grant_count = input.text_style_node_grant.size(),
    };
    take(computation_inputs, layout_arena, &transaction);
    adopt_identity_grant(m_granted_style_nodes, input.style_node_grant);
    adopt_identity_grant(m_granted_text_style_nodes, input.text_style_node_grant);
    reuse_journal_buffer(m_tree_deltas, input.tree_deltas);
    reuse_journal_buffer(m_element_arrivals, input.element_arrivals);
    reuse_journal_buffer(m_arrival_custom_state_atoms, input.arrival_custom_state_atoms);
    reuse_journal_buffer(m_local_feature_deltas, input.local_feature_deltas);
    reuse_journal_buffer(m_state_deltas, input.state_deltas);
    reuse_journal_buffer(m_element_declaration_deltas, input.element_declaration_deltas);
    reuse_journal_buffer(m_host_fact_writes, input.host_fact_writes);
    reuse_journal_buffer(m_host_fact_text_data, input.host_fact_text_data);
    reuse_journal_buffer(m_host_fact_replaced_content_inputs, input.host_fact_replaced_content_inputs);
    m_recorded_input_for_pass.clear();
}

StyleEngine::PublishedStyleTransaction StyleEngine::take_style_transaction(StyleNodeID root, Optional<OwnerRenderHalf> owner_render_half)
{
    auto submission_started_at = MonotonicTime::now();
    StyleEngineFFI::FfiStyleTransactionView view {};
    MonotonicTime bridge_started_at = submission_started_at;
    StyleEngineFFI::FfiOwnerRenderHalf render_half {
        .applies = owner_render_half.has_value(),
        .viewport_propagation_sources = owner_render_half.has_value() ? reinterpret_cast<u32 const*>(owner_render_half->viewport_propagation_sources.data()) : nullptr,
        .viewport_propagation_source_count = owner_render_half.has_value() ? owner_render_half->viewport_propagation_sources.size() : 0,
    };
    lend_style_transaction_inputs(RecordedInputGoesTo::Transaction, [&](auto const& computation_inputs, void* layout_arena, InputTransaction const* input) {
        bridge_started_at = MonotonicTime::now();
        view = StyleEngineFFI::style_engine_take_style_transaction(rust_handle(), root.value(), computation_inputs, layout_arena, input, install_feedback(), render_half);
        // What applying the batch handed back was paid, and what its rows marked held for the install, as the
        // transaction came back.
        if (view.render_half_applied) {
            m_owner_applied_render_half = true;
            // The owner marked the visual contexts the rows moved, which the document's paint preparation updates.
            if (view.render_half_moved_visual_contexts && m_style_computer)
                m_style_computer->document().set_needs_accumulated_visual_contexts_update(true);
            // The owner marked the layout nodes it repainted, and the document's navigable paints them again.
            if (view.render_half_repaint != 0 && m_style_computer)
                Painting::repaint_document_after_owner_style_change(m_style_computer->document(), view.render_half_repaint == 2 ? InvalidateDisplayList::PaintCommandsAndHitTestList : InvalidateDisplayList::PaintCommands);
        }
    });
    auto bridge_microseconds = (MonotonicTime::now() - bridge_started_at).to_truncated_microseconds();
    return publish_style_transaction_view(view, (bridge_started_at - submission_started_at).to_truncated_microseconds(), bridge_microseconds);
}

void StyleEngine::submit_style_transaction(StyleNodeID root)
{
    auto submission_started_at = MonotonicTime::now();
    lend_style_transaction_inputs(RecordedInputGoesTo::Transaction, [&](auto const& computation_inputs, void* layout_arena, InputTransaction const* input) {
        StyleEngineFFI::style_engine_submit_style_transaction(rust_handle(), root.value(), computation_inputs, layout_arena, input, install_feedback());
    });
    m_submitted_style_transaction_microseconds = (MonotonicTime::now() - submission_started_at).to_truncated_microseconds();
    m_submitted_pass_in_flight = true;
}

void StyleEngine::note_style_node_retired(StyleNodeID style_node)
{
    if (m_submitted_pass_in_flight)
        m_style_nodes_retired_beside_pass.set(style_node);
}

StyleEngine::PublishedStyleTransaction StyleEngine::finish_submitted_style_transaction()
{
    m_submitted_pass_in_flight = false;
    auto bridge_started_at = MonotonicTime::now();
    // What was recorded beside the pass is held (see begin_holding_input_recorded_beside_pass()), and it may name an
    // atom the pass found unused.
    // The render owner finishes it with the document's render state, which the arena the pass was submitted for is.
    auto* layout_arena = m_style_computer ? m_style_computer->document().layout_arena_handle() : nullptr;
    auto view = StyleEngineFFI::style_engine_finish_submitted_style_transaction(rust_handle(), m_holds_input_recorded_beside_pass, layout_arena);
    auto bridge_microseconds = (MonotonicTime::now() - bridge_started_at).to_truncated_microseconds();
    return publish_style_transaction_view(view, exchange(m_submitted_style_transaction_microseconds, 0), bridge_microseconds);
}

StyleEngine::PublishedStyleTransaction StyleEngine::publish_style_transaction_view(StyleEngineFFI::FfiStyleTransactionView const& view, i64 submission_microseconds, i64 bridge_microseconds)
{
    if (view.reclaimed_style_atom_count != 0) {
        HashTable<StyleAtomID> reclaimed_atoms;
        reclaimed_atoms.ensure_capacity(view.reclaimed_style_atom_count);
        for (auto const& reclaimed : ReadonlySpan<StyleEngineFFI::FfiReclaimedStyleAtom> { view.reclaimed_style_atoms, view.reclaimed_style_atom_count }) {
            auto atom_id = StyleAtomID { reclaimed.atom };
            reclaimed_atoms.set(atom_id);
            m_published_language_atoms.remove(atom_id);
            m_published_custom_property_names.remove(atom_id);
            m_attribute_names.remove(atom_id);
            if (reclaimed.raw == 0)
                continue;
            auto atom = m_atoms.take(reclaimed.raw);
            VERIFY(atom.has_value());
            VERIFY(atom.release_value() == reclaimed.atom);
            Utf16FlyString::unref_raw(reclaimed.raw);
        }
        m_attribute_name_atoms.remove_all_matching([&](StyleAtomID local, auto& names_by_namespace) {
            if (reclaimed_atoms.contains(local))
                return true;
            names_by_namespace.remove_all_matching([&](StyleAtomID namespace_atom, StyleAtomID name) {
                return reclaimed_atoms.contains(namespace_atom) || reclaimed_atoms.contains(name);
            });
            return names_by_namespace.is_empty();
        });
        ++m_atom_generation;
    }
    m_connected_element_count_at_last_transaction = view.connected_element_count;
    return {
        .version = { view.transaction_version, view.program_version },
        .reactions = { view.answers, view.count },
        .is_scoped = view.scoped,
        .only_derived_child_reactions = view.only_derived_child_reactions,
        .submission_microseconds = static_cast<u64>(submission_microseconds),
        .bridge_microseconds = static_cast<u64>(bridge_microseconds),
    };
}

bool StyleEngine::may_have_child_dependent_selectors() const
{
    return StyleEngineFFI::style_engine_pending_facts(rust_handle()).child_dependent_selectors;
}

bool StyleEngine::has_pending_transaction() const
{
    return has_recorded_input() || has_install_feedback() || StyleEngineFFI::style_engine_pending_facts(rust_handle()).transaction;
}

void StyleEngine::settle_pseudo_elements_in_next_pass(StyleDrainScope const&, StyleNodeID style_node, bool old_is_list_item, ReadonlySpan<u64> held_pseudo_records)
{
    StyleEngineFFI::FfiPseudoElementSettle settle {
        .node = style_node.value(),
        .old_is_list_item = old_is_list_item,
        .held_pseudo_records = {},
    };
    VERIFY(held_pseudo_records.size() <= array_size(settle.held_pseudo_records));
    for (size_t kind = 0; kind < held_pseudo_records.size(); ++kind)
        settle.held_pseudo_records[kind] = held_pseudo_records[kind];
    m_pseudo_element_settles.append(settle);
    m_install_feedback_held_back = false;
}

StyleEngineFFI::FfiInstallFeedback StyleEngine::install_feedback() const
{
    return {
        .applied_style_reactions = m_applied_style_reactions.data(),
        .applied_style_reaction_count = m_applied_style_reactions.size(),
        .pseudo_element_settles = m_pseudo_element_settles.data(),
        .pseudo_element_settle_count = m_pseudo_element_settles.size(),
    };
}

void StyleEngine::record_applied_style_reaction(StyleNodeID style_node, u8 reaction, u8 inherited_style_groups_changed, u32 facts)
{
    m_applied_style_reactions.append({
        .node = style_node.value(),
        .reaction = reaction,
        .inherited_style_groups_changed = inherited_style_groups_changed,
        .facts = facts,
    });
    m_install_feedback_held_back = false;
}

bool StyleEngine::has_deferred_geometry_transaction() const
{
    // The submitted pass took the transaction a geometry read deferred with the rest of its inputs, and only a
    // geometry read, which takes the pass back first, defers another one. A layout pass is submitted once the
    // frame's style rounds have applied every transaction, a deferred one included.
    if (Layout::RustFFI::rust_stage_thread_style_pass_holds_style_engine(rust_handle()) || layout_pass_is_in_flight())
        return false;
    return StyleEngineFFI::style_engine_pending_facts(rust_handle()).deferred_geometry_transaction;
}

bool StyleEngine::has_deferred_element_style_inputs() const
{
    return StyleEngineFFI::style_engine_pending_facts(rust_handle()).deferred_element_style_inputs;
}

bool StyleEngine::has_deferred_element_style_input(StyleNodeID style_node) const
{
    if (StyleEngineFFI::style_engine_has_deferred_element_style_input(rust_handle(), style_node.value()))
        return true;
    // What the reactions held for the next transaction derive for the element is owed to it as well.
    return !m_applied_style_reactions.is_empty()
        && StyleEngineFFI::style_engine_applied_style_reactions_derive_input(rust_handle(), style_node.value(), m_applied_style_reactions.data(), m_applied_style_reactions.size());
}

bool StyleEngine::defer_pending_transaction_for_geometry_read()
{
    submit_recorded_input();
    // Only a pending transaction needs the owner to look into it, and to defer it.
    auto facts = StyleEngineFFI::style_engine_pending_facts(rust_handle());
    if (!facts.transaction)
        return !facts.may_affect_layout_geometry;
    return StyleEngineFFI::style_engine_defer_pending_transaction_for_geometry_read(rust_handle());
}

bool StyleEngine::begin_deferred_geometry_transaction_flush()
{
    submit_recorded_input();
    return StyleEngineFFI::style_engine_begin_deferred_geometry_transaction_flush(rust_handle());
}

void StyleEngine::end_deferred_geometry_transaction_flush()
{
    StyleEngineFFI::style_engine_end_deferred_geometry_transaction_flush(rust_handle());
}

bool StyleEngine::match_element(StyleNodeID node, Vector<RuleMatch>& matches, MatchPurpose purpose)
{
    // A synchronous match is an observation boundary. Most matching follows a published style
    // transaction, but detached-document style reads can arrive directly while mutation facts are
    // still staged. Settle those facts before asking the committed arrangement.
    if (has_pending_transaction())
        flush();
    matches.resize(max(m_element_match_capacity, 16u));
    auto read = [&] {
        return StyleEngineFFI::style_engine_match_element(rust_handle(), node.value(), matches.data(), matches.size(), purpose == MatchPurpose::Cascade);
    };
    auto count = read();
    if (count == NumericLimits<size_t>::max())
        return false;
    if (count > matches.size()) {
        // Nothing was written, so grow and ask again rather than reporting a truncated answer.
        m_element_match_capacity = count * 2;
        matches.resize(m_element_match_capacity);
        count = read();
        if (count == NumericLimits<size_t>::max() || count > matches.size())
            return false;
    }
    matches.shrink(count);
    return true;
}

void StyleEngine::for_each_counter(Function<void(StringView name, u64 value)> const& callback) const
{
    Vector<u64> values;
    values.resize(StyleEngineFFI::style_engine_counter_count());
    StyleEngineFFI::style_engine_counters(rust_handle(), values.data(), values.size());
    for (size_t index = 0; index < values.size(); ++index) {
        size_t name_length = 0;
        auto const* name = StyleEngineFFI::style_engine_counter_name(index, &name_length);
        callback(StringView { name, name_length }, values[index]);
    }
}

}
