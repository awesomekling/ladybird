/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

#pragma once

#include <AK/Function.h>
#include <AK/Span.h>
#include <LibGC/Ptr.h>
#include <LibWeb/CSS/PseudoClass.h>
#include <LibWeb/CSS/StyleProperty.h>
#include <LibWeb/Export.h>
#include <LibWeb/Forward.h>

namespace Web::CSS {

class RustRule;
class StyleEngine;

// Settle a style-change boundary deferred by a geometry read before mutating a rule's declaration
// block. Constructed sheets can have copies in multiple document engines.
WEB_API void flush_deferred_style_change_events_for_rule(CSSRule&);

// Translates DOM and CSSOM mutations into StyleEngine's typed semantic inputs.
//
// These are recording calls, not evaluation: each one appends to the batch that crosses into the
// engine at the next style flush. They are deliberately cheap and total -- an element with no style
// node identity records nothing at all, which is what keeps disconnected and never-styled content
// free.
//
// Called once the document's tree starts being tracked, before anything in it connects. Allocates
// the document's style node identity, which is the parent every top-level child names.
WEB_API void record_document_tree_tracked(DOM::Document&);

// Called once a subtree has been linked into a connected tree. Marks it as waiting to arrive: the
// elements, text nodes and shadow roots in it that have no style node identity yet take one, and
// the elements record their arrival, only once something observes the style engine.
WEB_API void record_subtree_connecting(DOM::Node& root);

// Called once a node has been linked into a connected tree. Marks the element as waiting to arrive
// if it has no style node identity yet and no subtree waiting to arrive covers it.
WEB_API void record_element_connected(DOM::Element&);

// Called once a text node has been linked into a connected tree. Marks it as waiting to arrive if it
// has no style node identity yet and no subtree waiting to arrive covers it.
WEB_API void record_text_connected(DOM::Text&);

// Gives every node waiting to arrive its style node identity and records its arrival, in tree
// order. Whatever reads the style engine, or a connected node's identity, calls this first.
WEB_API void take_in_pending_style_arrivals(DOM::Document&);

// Whether a style read of an element needs the nodes still waiting to arrive: they can decide its
// style, or the style of a node that has arrived, which the read's style update may compute.
WEB_API bool pending_style_arrivals_may_decide_style_of(DOM::AbstractElement const&);

// Takes in the nodes waiting to arrive for a style read that needs them, even inside a read that
// left them waiting.
WEB_API void take_in_pending_style_arrivals_for_read(DOM::Document&);

// While one is alive, the nodes of the document that wait to arrive keep waiting: a read of one
// element's style that does not need them leaves them to the next observer that does.
class WEB_API PendingStyleArrivalsWaitScope {
public:
    explicit PendingStyleArrivalsWaitScope(DOM::Document const&);
    ~PendingStyleArrivalsWaitScope();

private:
    DOM::Document const* m_previous_document { nullptr };
};

// Called once a text node's data has stopped being, or started being, nothing but ASCII whitespace.
// That is the only thing about its data the mirror carries.
WEB_API void record_text_whitespace_state_changed(DOM::Text&);
WEB_API void record_text_data_changed(DOM::Text&);
WEB_API void publish_pending_element_features(StyleEngine&, StyleComputer&);
WEB_API void publish_required_attribute_value_texts(StyleEngine&, StyleComputer&);

WEB_API void configure_isolated_selector_query_engine(StyleEngine&, DOM::Document&);

// Populate an isolated engine with the current facts of a DOM tree. The callback receives the temporary identity
// assigned to each element; no identity or transaction in the document's resident engine is changed.
// Returns the query root: the element itself, a fragment's synthetic root, or a document's document element.
WEB_API StyleNodeID populate_isolated_selector_query_engine(StyleEngine&, DOM::ParentNode&, Function<void(GC::Ref<DOM::Element>, StyleNodeID)> const&);

// Tell the document's engine whether this is an HTML document. Selectors compile against that fact,
// so it is published before any rule compiles; a selector query compiled by an early script can run
// before the first sheet attaches, and has to say it itself.
WEB_API void record_document_kind(DOM::Document&);

// Called while the subtree is still linked, so its old relations are still readable.
WEB_API void record_subtree_disconnecting(DOM::Node&);

// Report that an element moved without leaving the tree. `moveBefore()` keeps the element's state
// and its identity, so nothing disconnects and nothing connects, and only its relations move.
WEB_API void record_element_moved(DOM::Element&, DOM::Node* old_parent, DOM::Element* old_previous_sibling, DOM::Element* old_next_sibling);

// Report that an element or text node moved without leaving the tree, which moves its place in the
// DOM child sequence even where its element relations stay the same.
WEB_API void record_node_moved_in_dom_order(DOM::Node&, DOM::Node const& old_parent);

// A slottable's assigned slot is its parent in the flat tree, and a slot's name changing reassigns
// it there without any DOM mutation. Nothing else says so: the element did not move, so no tree
// delta carries it.
WEB_API void record_element_assigned_slot_changed(DOM::Element&, DOM::Element* old_slot);

// Report the whole ordered list of slottables a slot has assigned to it. The per-slottable relation
// above cannot stand in for it: a text slottable holds no relation row to stage a change on, and the
// order is the DOM's rather than the order assignments arrive in -- a manual assignment orders its
// nodes the way `assign()` named them, and a reorder among one slot's assignees changes no
// slottable's slot at all.
//
// Assignment runs inside an insertion, before the inserted subtree is named, so a slottable's
// arrival republishes the list it is now a member of.
WEB_API void record_slot_assignment_changed(HTML::HTMLSlotElement&);
WEB_API void record_top_layer_elements_changed(DOM::Document&);
// Records the slot assignments and the top layer that changed since the recorded input was last submitted, each read
// from the DOM whole, once.
void record_changed_node_lists(DOM::Document&, StyleEngine&);

// Called once every element of a shadow tree has recorded its own removal, so nothing still names
// the root as a parent. A shadow root's identity follows its host's lifetime: keeping it across a
// move to another document would name an identity that document's engine never minted.
WEB_API void record_shadow_root_disconnecting(DOM::ShadowRoot&);
WEB_API void record_shadow_root_connected(DOM::ShadowRoot&);

// Report the shadow parts an element exposes. A part is a fact about the element like a class is.
WEB_API void record_element_parts_changed(DOM::Element&);

// The custom states an element is in, which `:state()` tests. A state is a named fact about one
// element like a class is, so it is published as a set and the engine journals what moved.
WEB_API void record_element_custom_states_changed(DOM::Element&);

// The element's resolved language and directionality, which `:lang()` and `:dir()` test. Both
// inherit, so a change on one element is a change for its whole subtree, and each descendant
// publishes the value it now resolves to.
WEB_API void record_element_language_and_directionality(DOM::Element&);
WEB_API void record_element_directionality(DOM::Element&);

// The element facts the style computation's box-type transformation and element style adjustments
// read of the DOM. Mirrors Rust `element_adjustment_fact`.
enum ElementStyleAdjustmentFact : u32 {
    IsBr = 1 << 0,
    IsWbr = 1 << 1,
    DisallowDisplayContents = 1 << 2,
    RewriteInlineFlow = 1 << 3,
    IsButton = 1 << 4,
    ForceLineHeightNormal = 1 << 5,
    CheckInputLineHeight = 1 << 6,
    HideAudioWithoutControls = 1 << 7,
    IsTable = 1 << 8,
    ForcePositionStatic = 1 << 9,
    ForceSymbolDisplayInline = 1 << 10,
    IsMathML = 1 << 11,
    IsMathMLMtable = 1 << 12,
    IsMathMLMtr = 1 << 13,
    IsMathMLMtd = 1 << 14,
    IsTh = 1 << 15,
    IsDocumentElement = 1 << 16,
    HasAnimations = 1 << 17,
    // An SVG graphics element folds its own transform into its SVG container's layout, which the
    // style engine's damage for the element reads.
    IsSvgGraphicsElement = 1 << 18,
    // The element stands for an element-reference pseudo-element of its shadow host, whose style
    // it takes.
    IsShadowHostPseudoElement = 1 << 19,
    // An HTML <body>. The root's first one propagates its overflow to the viewport, which the style
    // engine's damage for the element reads.
    IsHtmlBodyElement = 1 << 20,
    // The element types layout tree construction branches on. An element's type is fixed when it is
    // created, so the store holds these rather than the tree builder asking the DOM for them.
    IsSvgElement = 1 << 21,
    IsSvgSwitchElement = 1 << 22,
    IsSvgContainer = 1 << 23,
    RequiresSvgContainer = 1 << 24,
    IsSvgForeignObjectElement = 1 << 25,
    IsSvgMaskElement = 1 << 26,
    IsSvgClipPathElement = 1 << 27,
    IsSvgPatternElement = 1 << 28,
    // Whether the element is rendered in the top layer. Unlike the type facts above it moves during
    // the element's lifetime, and every move is recorded where the top layer is maintained.
    RenderedInTopLayer = 1 << 29,
};
// What a layout row records about the element it is built for at the moment it is allocated. The
// tree build reads these out of the mirror rather than out of the DOM node.
// Mirrors Rust `element_construction_fact`.
enum ElementConstructionFact : u32 {
    IsHtmlInputElement = 1 << 0,
    IsHtmlHtmlElement = 1 << 1,
    IsInUserAgentShadowTree = 1 << 2,
    UsesButtonLayout = 1 << 3,
    IsEditingHost = 1 << 4,
    IsBody = 1 << 5,
    // Also an ElementStyleAdjustmentFact, which the style computation reads. A row is built out of
    // this word alone, so the fact is published into both rather than read across two.
    ConstructedAsDocumentElement = 1 << 6,
    IsHtmlImageElement = 1 << 7,
};
// What an element's `disabled` attribute makes of it, as the walk from a hit node to an event
// target reads it.
enum ElementFormControlDisabledFact : u8 {
    // A button, input, select, textarea or form-associated custom element carrying the attribute.
    // It is disabled, and so is everything written under it.
    DisabledFormControl = 1 << 0,
    // A `<fieldset>` carrying the attribute. The fieldset itself stays enabled; everything written
    // under it does not, its first `<legend>` included.
    DisabledFieldSet = 1 << 1,
};
// Which principal box an element asks for before its computed style has a say. The element's own
// type and state decide this; the tree build resolves it against the element's computed display
// and appearance. Mirrors Rust `FfiElementBoxKind`.
enum class ElementBoxKind : u8 {
    // The computed display decides the box on its own.
    FromDisplay,
    // The element generates no box, whatever its display says.
    NoBox,
    Break,
    FieldSet,
    Legend,
    Audio,
    Video,
    Canvas,
    NavigableContainerViewport,
    TextArea,
    Image,
    SvgGraphics,
    SvgSvg,
    SvgText,
    SvgTextPath,
    SvgForeignObject,
    SvgImage,
    SvgGeometry,
    // An input's native widget. `appearance: none` suppresses it, and then the computed display
    // decides the box like it does for any other element.
    InputButton,
    InputCheckBox,
    InputRadioButton,
    InputRange,
    InputText,
};
WEB_API u32 element_construction_facts(DOM::Element const&);

// Whether an event aimed at the node named by `identity` would reach a disabled form control on its
// way out of the tree: the node itself is one, or one of the nodes it is written under is. Each
// element on the way answers from its own type and attribute alone.
WEB_API bool event_dispatch_is_disabled(DOM::Document&, DOM::NodeIdentity);
WEB_API u32 element_style_adjustment_facts(DOM::Element const&);
WEB_API u32 element_box_type_adjustment_facts(DOM::Element const&);
WEB_API void record_element_adjustment_facts(DOM::Element&);
WEB_API void record_element_construction_facts(DOM::Element&);
WEB_API void record_element_replaced_content_input(DOM::Element&);
WEB_API bool record_element_presentational_hint_properties(DOM::Element&, ReadonlySpan<StyleProperty>);
WEB_API void republish_presentational_hints(DOM::Element&);
WEB_API void record_element_animation_names(DOM::Element&, ReadonlySpan<Utf16FlyString>);
WEB_API void record_element_css_defined_animations(DOM::Element&, u8 slot, ReadonlySpan<Utf16FlyString> names, ReadonlySpan<u64> definition_words);
WEB_API void record_element_animation_timing_rows(DOM::Element&, u8 slot, ReadonlySpan<u32> words, ReadonlySpan<u64> times, ReadonlySpan<u64> linear_points);
WEB_API void record_element_animation_effect_descriptions(DOM::Element&, u8 slot, ReadonlySpan<GC::Ref<Animations::KeyframeEffect>>);
// The keyframe sets travel as the pointers the scope's name table names them by: naming their type
// here would mean pulling `Animations::KeyframeEffect` into every translation unit that styles.
WEB_API void record_tree_scope_animation_keyframes(DOM::Document&, TreeScopeID, FlatPtr shadow_root_identity, ReadonlySpan<u32> name_lengths, ReadonlySpan<u16> name_units, ReadonlySpan<FlatPtr> keyframe_sets);
WEB_API void record_animation_timeline_samples(DOM::Document&, ReadonlySpan<u32> identities, ReadonlySpan<u32> words, ReadonlySpan<u64> times);
WEB_API void record_element_custom_property_names(DOM::Element&, ReadonlySpan<Utf16FlyString>, bool uses_unnamed, bool uses_custom_functions);

// The same index, from the environments the element and its pseudo-elements resolved to, plus
// names read outside substitution (for example by style queries). The names each environment
// declares are interned into engine identities once for that environment. The references must hold
// no duplicates.
WEB_API void record_element_custom_property_names(DOM::Element&, CustomPropertyData const*, ReadonlySpan<RefPtr<CustomPropertyData const>> pseudo_element_data, ReadonlySpan<Utf16FlyString> references, bool uses_unnamed, bool uses_custom_functions);

// Report that a child of an element arrived, left, or changed in a way that can move whether the
// element is empty. `counted_before` and `counts_after` say whether that child is one `:empty` counts:
// an element child always counts, a text child counts while its data is not empty, and anything else
// never does. The element's other children decide the rest, so a change that moves nothing says
// nothing. A text node connects no element, so nothing but this says it.
WEB_API void record_element_emptiness_changed(DOM::Element&, DOM::Node const& changing_child, bool counted_before, bool counts_after);

// Called for each element whose pseudo-class state actually changed, with the value it changed to.
// Only the states StyleEngine models as facts are recorded; the rest arrive when it models them.
WEB_API bool can_record_element_state_change(DOM::Element&);
WEB_API void record_element_state_changed(DOM::Element&, PseudoClass, bool new_value);

// Called before each style flush. The user-agent and user origins have no sheet list to announce
// themselves from, so the engine is told about them from here.
WEB_API void record_non_author_stylesheets(DOM::Document&);

// Called once a sheet has taken its place in the sheet list, so its successor is known.
WEB_API void record_stylesheet_attached(StyleSheetState&, DOM::Node& document_or_shadow_root, StyleSheetState* before);

// The order one tree scope declares its qualified cascade layer names in. Rules retain the interned
// names, while StyleEngine derives the scope-local ranks used by every cascade consumer.
WEB_API TreeScopeID style_engine_tree_scope_for(DOM::Node&);

// A sheet's rules are not immutable, and each of these is one rule moving rather than the sheet
// being rebuilt. Patching what moved is the point: retiring identities that did not change would
// turn an `insertRule` into a program change over the whole sheet.
WEB_API void record_style_rule_inserted(CSSRule&);
WEB_API void record_style_rule_inserted(RustRule const&, StyleSheetState&);
WEB_API void record_imported_style_sheet_loaded(u64 import_rule_identity, StyleSheetState&);
WEB_API void record_style_rule_removed(CSSRule&);
WEB_API void record_style_rule_removed(StyleSheetState&, RustRule const&, StyleSheetState const* detached_import = nullptr);
WEB_API void record_style_rule_selector_changed(CSSStyleRule&);
WEB_API void record_style_rule_declarations_changed(CSSRule&);
WEB_API void record_style_rule_declarations_changed(RustRule const&, StyleSheetState&);

// `replace()` is the exception: it swaps the whole rule list, so there is nothing to keep.
WEB_API void record_stylesheet_rules_replaced(StyleSheetState&);
WEB_API void record_stylesheet_detached(StyleSheetState&, DOM::Node& document_or_shadow_root);
WEB_API bool stop_sharing_compiled_style_sheet(StyleSheetState&);

// Called once a sheet's media queries have been evaluated.
WEB_API void record_stylesheet_conditions(StyleSheetState&, DOM::Node& document_or_shadow_root, bool conditions_hold);
WEB_API void record_stylesheet_rule_conditions(StyleSheetState&);
WEB_API void record_stylesheet_rule_conditions(StyleSheetState&, DOM::Document&);

WEB_API void record_element_id_changed(DOM::Element&, Optional<Utf16FlyString> const& old_value, Optional<Utf16FlyString> const& new_value);
WEB_API void record_element_class_list_changed(DOM::Element&, Vector<Utf16FlyString> const& old_classes, Vector<Utf16FlyString> const& new_classes);
WEB_API void record_element_attribute_changed(DOM::Element&, Utf16FlyString const& name, Optional<Utf16FlyString> const& namespace_uri, Optional<Utf16String> const& old_value, Optional<Utf16String> const& new_value);

// Called when a declaration block the element itself sources has changed: its inline style, its
// presentational hints, or its SVG presentation attributes. What that changes is which declaration
// wins on this element, never which elements match, so it reaches the element and nothing else.
enum class ElementDeclarationKind : u8 {
    InlineStyle,
    PresentationalHint,
    SvgPresentationAttribute,
};
WEB_API void record_element_declarations_changed(DOM::Element&, ElementDeclarationKind, bool had_declarations, bool has_declarations);

// While a batch's reactions are applied, a host's own style application may rewrite the
// declarations of an element in its shadow tree, after the engine computed that element's record
// from the ones it had. These name the elements whose declarations changed that way.
void begin_noting_declaration_changes_during_apply();
void end_noting_declaration_changes_during_apply();
bool declarations_changed_during_apply(StyleNodeID);

}
