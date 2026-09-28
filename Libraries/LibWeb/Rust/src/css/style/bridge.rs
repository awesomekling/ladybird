/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The C++/Rust boundary for StyleEngine.
//!
//! C++ emits one flat immutable transaction per style flush. The arrays use fixed-width IDs and
//! tagged records and contain no owning C++ pointers, and each typed delta kind travels in its own
//! array. The point of the shape is what it forbids: there is never one call per element or per
//! selector operator.
//!
//! No string crosses here. Selector-mentioned tags, IDs, classes, attribute names, and attribute
//! values are interned on the C++ side, where the authoritative `Utf16FlyString` payloads already
//! live, and reach Rust as `StyleAtomID` words. Interning a name is a hash lookup plus a reference
//! count bump on a string that already exists; nothing is copied, and neither side pays a UTF-16 or
//! ASCII conversion for a fact that a `u32` comparison can answer. Values too unique to intern do
//! not become atoms at all - their exact test reads the live DOM through a borrowed string view
//! that preserves the C++ side's ASCII-or-UTF-16 representation, and only after the cheaper atom
//! and name checks have already passed.
//!
//! Identity allocation is batched for the same reason as everything else: C++ owns DOM lifecycle
//! and asks for a run of `StyleNodeID` values, Rust owns the arena and the relation columns keyed by
//! them.

use std::ffi::c_void;

use crate::abort_on_panic as abort_on_boundary_panic;
use crate::css::custom_properties::CustomPropertyRegistry;
use crate::css::host_shared::{HostShared, SharedPayload};
use crate::css::selector::CompiledSelector;
use crate::css::style_value::RetainedStyleValueData;

use super::HashSet;
use super::batch_matcher::RuleMatch;
use super::cascade::CascadeOperator;
use super::compiler::ImplicitScopeRoot;
use super::compiler::NamespaceScope;
use super::compiler::ScopeChain;
use super::engine_home::{Holder, Owed, StyleEngineLoan};
use super::index::FeatureValue;
use super::index::LocalFeatureKey;
use super::index::StyleAtomID;
use super::memory::DeviceClass;
#[cfg(feature = "style-recording")]
use super::memory::MEMORY_CATEGORIES;
#[cfg(feature = "style-recording")]
use super::memory::MEMORY_CATEGORY_COUNT;
#[cfg(feature = "style-recording")]
use super::memory::MemoryCategory;
#[cfg(feature = "style-recording")]
use super::memory::TIER3_REFUSAL_CATEGORIES;
use super::program::CascadeLayerID;
use super::program::CascadeOrigin;
use super::program::CustomDeclaration;
use super::program::DeclarationBlockID;
use super::program::DeclaredProperty;
use super::program::RuleID;
use super::program::RuleKind;
use super::program::SheetID;
use super::program::StyleSheetObjectID;
use super::record_replay::EventKind;
use super::transaction::ElementDeclarationKind;
use super::transaction::InputKey;
use super::transaction::InputValue;
use super::transaction::StateFact;
use super::transaction::TreeRelations;
use super::tree::StyleNodeID;
use super::tree::TreeScopeID;
use super::{Counters, StyleEngine, StyleEngineState};
use super::{StyleEngineHandle, StyleEngineInputHandle};

fn abort_on_panic<F: FnOnce() -> R, R>(operation: F) -> R {
    abort_on_boundary_panic(operation)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FfiStyleInvalidationField {
    LevelMask = 0x3,
    VisualContextShift = 2,
    RebuildRootShift = 4,
    RebuildRootMask = 0x7,
    RebuildStackingContext = 1 << 7,
    ResnapScrollContainer = 1 << 8,
    RecomputeDescendants = 1 << 9,
    InheritedGroupsShift = 10,
    InheritedGroupsMask = 0x7f,
    RepaintTextDecorations = 1 << 18,
    NonInheritedInheritanceSource = 1 << 19,
    AnyComputedValueChanged = 1 << 20,
    CacheHit = 1 << 21,
    AffectsHitTesting = 1 << 22,
    /// The word holds the damage the engine computed with its answer.
    EngineComputed = 1 << 23,
    /// Selection highlights, which text descendants paint, repaint.
    RepaintSelection = 1 << 24,
    /// The row is no row the transaction planned: it joined the pass for a reaction a row the pass
    /// settled derived for it.
    JoinedByDerivation = 1 << 25,
    /// The engine derived the children's reactions from the row's move away from the old record
    /// it names: the host applying the row over that record derives nothing more.
    ChildrenDerivedOverOldRecord = 1 << 26,
    /// The engine derived the children's reactions from the row's move away from no record: the
    /// host applying the row to an element without style derives nothing more.
    ChildrenDerivedOverNoRecord = 1 << 27,
    /// The row is no row the transaction planned: it joined the batch for an element a row
    /// inherits from that has no style, or for one between two rows of the batch.
    JoinedForInheritance = 1 << 28,
    /// The row's custom-property environment moved, and the pass moved the environments below it
    /// as it settled the row: the host walks nothing below it.
    EnvironmentMovedInPass = 1 << 29,
    /// The damage the engine computed is all the move damages: the element's box was built from
    /// the old record with no counter styles the host would compare, as it was no list item and
    /// its content named no counters.
    DamageIsTotal = 1 << 30,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct FfiAnimationInvalidation {
    pub invalidation: u32,
    pub changed_non_inherited_style_groups: u32,
    pub requires_base_style_recomputation: bool,
    pub requires_layout_node_style_application: bool,
    pub requires_style_resource_update: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiStyleDeltaGap {
    None,
    Materialize,
    /// The engine computed the new record itself from the moved cascade winners; C++ applies it
    /// without running a style computation.
    Computed,
    /// An unstyled descendant of a hidden ancestor needs no record in this batch.
    SkippedHidden = 4,
    /// The engine computed the new record over the ancestors the host installed before it, for a
    /// row tied to those ancestors. C++ applies it as a record that reads them as they now stand.
    RetriedAfterAncestors,
    /// The engine computed the new record over the installed ancestors for a row C++ would
    /// otherwise have computed itself. C++ installs it even on an element without a style.
    RetriedMaterialization,
    /// An ancestor's custom-property environment moved, and the pass republished the element's
    /// record over the moved one as it settled the ancestor. C++ installs the record, and the element
    /// takes the moved environment as it acknowledges it.
    EnvironmentMoved,
    /// The pass settled the synthetic pseudo-elements of an element over the composition the host
    /// installed for it since the pass that settled the element: the element's row names the record
    /// they were settled over, with the kinds whose animations the pass sampled as its
    /// `inherited_style_groups`, and the rows after it name their records. C++ installs those.
    PseudoElementsSettled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum FfiStyleDeltaDamage {
    None,
    Full,
}

/// One final style assignment or a typed request to materialize it in C++.
///
/// A zero match-answer identity means the complete answer is node-contextual and cannot be shared
/// across elements. It remains in transaction scratch under the style-node identity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct FfiStyleDelta {
    pub style_node: u32,
    pub match_answer: u32,
    pub old_style_record: u64,
    pub new_style_record: u64,
    pub damage: FfiStyleDeltaDamage,
    pub reaction: u8,
    pub inherited_style_groups: u8,
    pub pseudo_kind: u8,
    pub gap: FfiStyleDeltaGap,
    /// Substitution usage for an engine-computed element, including its pseudo-elements.
    pub uses_substitution: bool,
    /// What moving the element from the old record to the new one damages, packed as an
    /// `FfiStyleInvalidationField` word. Only a word with `EngineComputed` set holds an answer; the
    /// host computes the damage of any other move itself.
    pub record_damage: u32,
    /// What an element row's node holds as the transaction publishes it, packed as an
    /// `FfiStyleRowFact` word. A word without `Present` holds no answer.
    pub row_facts: u32,
    /// The explicit inheritance debt of a computed element row, taken as the row is published.
    pub explicit_inheritance_debt: u32,
    /// The settled row effect debt of a computed element row, taken as the row is published.
    pub row_effect_debt: u32,
}

/// The facts of an element row's node the host reads with the row, where it would otherwise ask
/// the engine for each as it installs the row. They hold until the host computes the node again.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum FfiStyleRowFact {
    /// The node's `node_record_reads` bits.
    RecordReadsMask = 0xff,
    /// The node's cascade declares custom properties of its own.
    DeclaresCustomProperties = 1 << 8,
    /// The word holds the node's facts.
    Present = 1 << 30,
}

/// A retried engine record and the metadata needed to install it.
#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct FfiEngineComputedRecord {
    pub style_record: u64,
    pub uses_substitution: bool,
    /// The synthetic pseudo-element kinds whose records the engine settled beside the
    /// element's, as a bit per kind; a present slot holding zero is a removal.
    pub pseudo_records_present: u8,
    pub pseudo_records: [u64; RETRY_PSEUDO_RECORD_SLOTS],
}

/// One synchronous record demand: the row's record, or the absence of a pseudo-element that
/// generates no box.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiRecordDemandAnswer {
    pub record: FfiEngineComputedRecord,
    pub is_absent: bool,
    pub is_provisional: bool,
    /// The node's `FfiStyleRowFact` word as the demand leaves it.
    pub row_facts: u32,
    /// The answered record as a published value (see [`super::published_record`]), which the
    /// caller owns one reference of; null where the answer is absent.
    pub published_record: *const c_void,
    /// Whether nothing answered the demand: the render owner panicked answering it, and may have
    /// left it half done in the engine, so nothing answers it again. Absent as well.
    pub unanswered: bool,
}

impl FfiRecordDemandAnswer {
    /// The answer of an absent record.
    pub(crate) fn absent() -> Self {
        Self {
            record: FfiEngineComputedRecord::default(),
            is_absent: true,
            is_provisional: false,
            row_facts: 0,
            published_record: std::ptr::null(),
            unanswered: false,
        }
    }
}

// SAFETY: `published_record` is null, or an owned `Arc` of a `PublishedStyleRecord`, which is
// `Send + Sync`, in raw form: the answer moves the reference with it.
unsafe impl Send for FfiRecordDemandAnswer {}

/// One record slot per synthetic pseudo-element kind in a retried record.
pub const RETRY_PSEUDO_RECORD_SLOTS: usize = 8;

#[derive(Default)]
pub(super) struct FfiStyleTransactionOutput {
    scoped: bool,
    style_atoms_swept: bool,
    only_derived_child_reactions: bool,
    transaction_version: u64,
    program_version: u64,
    answers: Vec<FfiStyleDelta>,
    reclaimed_style_atoms: Vec<FfiReclaimedStyleAtom>,
    /// Whether each element row already holds its node's facts, as a submitted pass leaves them.
    row_facts_settled: bool,
}

impl FfiStyleTransactionOutput {
    /// The rows the transaction published, in the order the host applies them.
    pub(super) fn answers(&self) -> &[FfiStyleDelta] {
        &self.answers
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct FfiReclaimedStyleAtom {
    pub raw: usize,
    pub atom: u32,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiStyleTransactionView {
    pub transaction_version: u64,
    pub program_version: u64,
    pub answers: *const FfiStyleDelta,
    pub count: usize,
    pub reclaimed_style_atoms: *const FfiReclaimedStyleAtom,
    pub reclaimed_style_atom_count: usize,
    pub scoped: bool,
    pub style_atoms_swept: bool,
    /// The transaction planned nothing but the child reactions the engine derived from the
    /// reactions C++ applied last: one more generation of the same style change, not a new one.
    pub only_derived_child_reactions: bool,
    /// The elements connected to the document as the transaction was taken.
    pub connected_element_count: u32,
    /// The render owner applied the batch's rows to the layout nodes of their elements as it took
    /// the transaction, as a flight does for its pass: the host pays what that handed back before
    /// it installs the batch, and ends that render half once its style update has installed it.
    pub render_half_applied: bool,
    /// A row the owner applied moved the visual contexts of its layout nodes, which the document's
    /// next paint preparation updates.
    pub render_half_moved_visual_contexts: bool,
    /// A row the owner applied repainted its layout nodes, which the document's navigable paints
    /// again: 0 for none, 1 with the paint commands, 2 with the hit test items too.
    pub render_half_repaint: u8,
}

/// A host-owned object the engine names but never follows.
///
/// The engine holds it as an integer rather than a raw pointer, because the read side an
/// evaluation step borrows has to be `Sync` and a raw pointer is neither `Send` nor `Sync` — for
/// the good reason that nothing about it says who may follow it. Only the bridge turns one back
/// into a pointer, at a host call, which never happens inside a step.
/// C++ hands one over as the address of the object it owns.
///
/// NB: The pointer that becomes a handle is exposed, and the handle becomes a pointer through
///     that exposed provenance, because the host call does dereference it (a style value is
///     retained, a registry is read). A bare address would be a pointer nothing may access.
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FfiHostHandle {
    pub address: usize,
}

impl FfiHostHandle {
    #[must_use]
    pub fn from_pointer(pointer: *const c_void) -> Self {
        Self {
            address: pointer.expose_provenance(),
        }
    }

    #[must_use]
    pub fn as_pointer(self) -> *const c_void {
        std::ptr::with_exposed_provenance(self.address)
    }

    #[must_use]
    pub fn is_none(self) -> bool {
        self.address == 0
    }
}

/// Document-wide scalar computation inputs captured at a style transaction boundary.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiDocumentStyleComputationInputs {
    pub in_quirks_mode: bool,
    pub viewport_width: f64,
    pub viewport_height: f64,
    pub root_font_size: f64,
    pub root_font_x_height: f64,
    pub root_font_cap_height: f64,
    pub root_font_zero_advance: f64,
    pub root_line_height: f64,
    pub root_font_metrics_depend_on_viewport_metrics: bool,
    /// The metrics of the document's initial font, which the document element's own font
    /// resolves against.
    pub initial_font_size: f64,
    pub initial_font_x_height: f64,
    pub initial_font_cap_height: f64,
    pub initial_font_zero_advance: f64,
    pub initial_font_size_raw: i32,
    pub default_font_size_raw: i32,
    pub device_pixels_per_css_pixel: f64,
    pub font_environment_generation: u64,
    /// The host's own version of everything a computation reads that is neither the element's
    /// cascade nor its parent's style: the document environment and the viewport. A record the
    /// engine settles is stamped with it, so the element's next C++ computation can tell that
    /// nothing but its declarations moved.
    pub style_environment_version: u64,
    /// The page's preferred color scheme and the document's supported schemes, as
    /// PreferredColorScheme codes; up to four supported schemes are carried.
    pub preferred_color_scheme: u8,
    pub has_document_supported_schemes: bool,
    pub document_supported_scheme_count: u8,
    pub document_supported_scheme_codes: [u8; 4],
    /// The document's custom-property registry, and the generation its registrations are at:
    /// what an engine-computed environment resolves registered names against.
    pub custom_property_registry: FfiHostHandle,
    pub custom_property_registration_generation: u64,
    /// What a `url()` resolves against, lent for the boundary call only: the document's base URL,
    /// and an `FfiStyleSheetResourceContextEntry` for each style sheet a rule may come from. The
    /// engine copies them and clears these fields before it keeps the inputs.
    pub document_base_url: FfiHostHandle,
    pub document_base_url_length: usize,
    pub style_sheet_resource_contexts: FfiHostHandle,
    pub style_sheet_resource_context_count: usize,
    /// The style update's media snapshot, copied at the transaction boundary.
    pub media_feature_values: FfiHostHandle,
    pub media_feature_value_count: usize,
    pub media_length_resolution_context: FfiHostHandle,
    /// The document's visible custom functions, `FfiCustomFunctionEntry`s, copied at the
    /// transaction boundary.
    pub custom_functions: FfiHostHandle,
    pub custom_function_count: usize,
}

/// One custom function definition a caller scope can see, as the host publishes it for a
/// transaction.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FfiCustomFunctionEntry {
    pub function: *const c_void,
    pub caller_scope: usize,
    pub definition_scope: usize,
    pub tree_scope: u32,
}

/// One style sheet's resource context, keyed by the identity of its native sheet: the base URL a
/// `url()` in its rules resolves against, and whether the sheet is origin-clean.
#[repr(C)]
pub struct FfiStyleSheetResourceContextEntry {
    pub source_identity: u64,
    pub base_url: *const u8,
    pub base_url_length: usize,
    pub has_base_url: bool,
    pub origin_clean: bool,
}

impl Default for FfiDocumentStyleComputationInputs {
    fn default() -> Self {
        Self {
            in_quirks_mode: false,
            viewport_width: 0.0,
            viewport_height: 0.0,
            root_font_size: 0.0,
            root_font_x_height: 0.0,
            root_font_cap_height: 0.0,
            root_font_zero_advance: 0.0,
            root_line_height: 0.0,
            root_font_metrics_depend_on_viewport_metrics: false,
            initial_font_size: 0.0,
            initial_font_x_height: 0.0,
            initial_font_cap_height: 0.0,
            initial_font_zero_advance: 0.0,
            initial_font_size_raw: 0,
            default_font_size_raw: 0,
            device_pixels_per_css_pixel: 0.0,
            font_environment_generation: 0,
            style_environment_version: 0,
            preferred_color_scheme: 0,
            has_document_supported_schemes: false,
            document_supported_scheme_count: 0,
            document_supported_scheme_codes: [0; 4],
            custom_property_registry: FfiHostHandle { address: 0 },
            custom_property_registration_generation: 0,
            document_base_url: FfiHostHandle { address: 0 },
            document_base_url_length: 0,
            style_sheet_resource_contexts: FfiHostHandle { address: 0 },
            style_sheet_resource_context_count: 0,
            media_feature_values: FfiHostHandle { address: 0 },
            media_feature_value_count: 0,
            media_length_resolution_context: FfiHostHandle { address: 0 },
            custom_functions: FfiHostHandle { address: 0 },
            custom_function_count: 0,
        }
    }
}

/// Matches `Web::CSS::FontResolutionFeatureInput::Count`.
pub const FONT_RESOLUTION_FEATURE_INPUT_COUNT: usize = 11;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiFontResolutionRequest {
    pub font_family: FfiHostHandle,
    pub tree_scope: u32,
    /// The computed values the resolver reads beside the family, in `FontResolutionFeatureInput`
    /// order, each null when the property has its initial value. They select shaping features and
    /// variations, so two elements differing only in one of them resolve to different fonts.
    pub font_feature_values: [FfiHostHandle; FONT_RESOLUTION_FEATURE_INPUT_COUNT],
    pub font_size_raw: i32,
    pub font_slope: i32,
    pub font_weight: f64,
    pub font_width: f64,
    pub font_optical_sizing: u8,
    pub font_environment_generation: u64,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FfiResolvedFont {
    /// The two host font objects a resolution names. The engine holds them as handles and hands
    /// them straight back to C++ when it publishes a record; nothing in Rust follows either.
    pub first_available_font: FfiHostHandle,
    pub font_cascade_list: FfiHostHandle,
    pub ascent: f32,
    pub descent: f32,
    pub x_height: f32,
    pub zero_advance: f32,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiStyleNodeSlice {
    pub nodes: *const u32,
    pub count: usize,
}

impl Default for FfiStyleNodeSlice {
    fn default() -> Self {
        Self {
            nodes: std::ptr::null(),
            count: 0,
        }
    }
}

impl Default for FfiStyleTransactionView {
    fn default() -> Self {
        Self {
            transaction_version: 0,
            program_version: 0,
            answers: std::ptr::null(),
            count: 0,
            reclaimed_style_atoms: std::ptr::null(),
            reclaimed_style_atom_count: 0,
            scoped: false,
            only_derived_child_reactions: false,
            connected_element_count: 0,
            style_atoms_swept: false,
            render_half_applied: false,
            render_half_moved_visual_contexts: false,
            render_half_repaint: 0,
        }
    }
}

fn write_style_transaction_outputs(
    output: &FfiStyleTransactionOutput,
    payload: &mut super::record_replay::PayloadWriter,
) {
    payload.write_bool(output.scoped);
    payload.write_length(0);
    payload.write_length(usize::from(!output.answers.is_empty()));
    if !output.answers.is_empty() {
        payload.write_u64(output.transaction_version);
        payload.write_u64(output.program_version);
        payload.write_length(output.answers.len());
        for answer in &output.answers {
            payload.write_u32(answer.style_node);
            payload.write_u32(answer.match_answer);
            payload.write_u64(answer.old_style_record);
            payload.write_u64(answer.new_style_record);
            payload.write_u16(answer.damage as u16);
            payload.write_u8(answer.reaction);
            payload.write_u8(answer.inherited_style_groups);
            payload.write_u8(answer.pseudo_kind);
            payload.write_u8(answer.gap as u8);
            payload.write_bool(answer.uses_substitution);
        }
    }
    payload.write_bool(output.style_atoms_swept);
    payload.write_length(output.reclaimed_style_atoms.len());
    for reclaimed in &output.reclaimed_style_atoms {
        payload.write_u32(reclaimed.atom);
    }
}

/// One final base-style assignment. Zero names the absent side of an insertion or removal.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(C)]
pub struct FfiStyleRecordDelta {
    pub old_style_record: u64,
    pub new_style_record: u64,
}

#[cfg(feature = "style-recording")]
#[derive(Clone, Copy, Debug, Default)]
pub struct FfiMemoryPressureSnapshot {
    pub tier3_limit: u64,
    pub tier4_limit: u64,
    pub tier3_bytes: u64,
    pub tier4_bytes: u64,
    pub tier3_refusals: u64,
    pub tier4_refusals: u64,
    pub tier3_refusal_categories: [u64; TIER3_REFUSAL_CATEGORIES.len()],
    pub tier4_refusal_categories: [u64; 2],
    pub tier3_evictions: u64,
    pub category_bytes: [u64; MEMORY_CATEGORY_COUNT],
}

/// A synchronous borrowed view of one final style record.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FfiStyleRecordView {
    pub payloads: *const *const c_void,
    pub base_payloads: *const *const c_void,
    /// The base record's borrowed table, matching `WithAnimationsApplied::No`.
    pub longhand_table: *const c_void,
    pub animated_overlay: *const c_void,
    pub payload_count: usize,
    pub pseudo_element_styles: u64,
    pub counter_style_environment_identity: u64,
    pub animation_overlay_identity: u64,
    pub dependency_flags: u8,
    pub present: bool,
}

impl FfiStyleRecordView {
    fn missing() -> Self {
        Self {
            payloads: std::ptr::null(),
            base_payloads: std::ptr::null(),
            longhand_table: std::ptr::null(),
            animated_overlay: std::ptr::null(),
            payload_count: 0,
            pseudo_element_styles: 0,
            counter_style_environment_identity: 0,
            animation_overlay_identity: 0,
            dependency_flags: 0,
            present: false,
        }
    }
}

/// The exact cascade comparison and the computed-group dependency closure it wakes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct FfiExactCascadePublication {
    pub computed_group_mask: u32,
    pub unchanged: bool,
    pub donor_used: bool,
}

impl FfiExactCascadePublication {}

#[cfg(feature = "style-recording")]
#[derive(Clone, Copy)]
pub struct RecordedExactCascadeWinner {
    pub property: u16,
    pub value: u64,
    pub operator: CascadeOperator,
    pub important: bool,
}

/// The tree relations of one style node on one side of a mutation. Zero means "no such relation".
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiTreeRelations {
    pub parent: u32,
    pub previous_element_sibling: u32,
    pub next_element_sibling: u32,
    pub tree_scope: u32,
    pub assigned_slot: u32,
    // Retained as zero to preserve the record-replay wire layout.
    pub reserved: u32,
}

impl FfiTreeRelations {
    fn decode(self) -> TreeRelations {
        TreeRelations {
            parent: StyleNodeID::from_raw(self.parent),
            previous_element_sibling: StyleNodeID::from_raw(self.previous_element_sibling),
            next_element_sibling: StyleNodeID::from_raw(self.next_element_sibling),
            tree_scope: TreeScopeID(self.tree_scope),
            assigned_slot: StyleNodeID::from_raw(self.assigned_slot),
        }
    }
}

/// One structural change. `old_connected` and `new_connected` distinguish insertion, removal, and
/// movement without needing three record types.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiTreeDelta {
    pub node: u32,
    pub old_connected: bool,
    pub new_connected: bool,
    pub old_relations: FfiTreeRelations,
    pub new_relations: FfiTreeRelations,
}

/// Selector-visible facts which exist when one style node first joins the tree. Variable-width
/// custom states occupy one shared atom column and are named by this row's offset and count.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiElementArrival {
    pub node: u32,
    pub namespace_atom: u32,
    pub language_atom: u32,
    pub directionality_atom: u32,
    pub custom_state_offset: u32,
    pub custom_state_count: u32,
    pub heading_level: u8,
    pub is_slot: bool,
    /// The element's `FfiElementBoxKind`: which principal box it asks for.
    pub box_kind: u8,
    /// One plus the element-reference pseudo kind represented by this element, or zero.
    pub associated_pseudo_kind_plus_one: u8,
    /// The element's `ElementStyleAdjustmentFact` bits: what the box-type transformation and the
    /// element style adjustments read of the DOM. Mirrors the C++ enum.
    pub adjustment_facts: u32,
    /// The element's `ElementConstructionFact` bits: what a layout row built for it records.
    pub construction_facts: u32,
}

/// The custom-property environment a row inherits from, answered from retained state. `is_present`
/// false means the engine holds no answer and the host has to walk for itself; a present row with
/// a null `data` is an element that holds no environment.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FfiRetainedCustomPropertyData {
    pub data: *const c_void,
    pub store: *const c_void,
    pub is_present: bool,
}

/// The last pseudo-element kind C++ materializes as a synthetic pseudo-element; the kinds up to
/// it are the bits a style record's pseudo-element mask carries. Mirrors the C++
/// `last_synthetic_pseudo_element`.
pub const LAST_SYNTHETIC_PSEUDO_ELEMENT_KIND: u16 = 7;
pub const FIRST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND: u8 = 8;
pub const LAST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND: u8 = 13;

/// What C++ reports about a style reaction it applied, for the engine to derive the reactions of
/// the element's children. Mirrors C++ `StyleReactionAppliedFact`.
pub mod style_reaction_applied_fact {
    pub const DID_CHANGE_CUSTOM_PROPERTIES: u32 = 1 << 0;
    pub const INVALIDATION_IS_NONE: u32 = 1 << 1;
    pub const NEEDS_LAYOUT_TREE_REBUILD: u32 = 1 << 2;
    pub const RECOMPUTE_DESCENDANT_STYLES: u32 = 1 << 3;
    pub const CHILDREN_EXPLICITLY_INHERIT: u32 = 1 << 4;
    pub const SHADOW_CHILDREN_EXPLICITLY_INHERIT: u32 = 1 << 5;
    pub const ROW_WAS_UNSTYLED: u32 = 1 << 6;
    pub const ROW_WAS_DISPLAY_NONE: u32 = 1 << 7;
    pub const ROW_DISPLAY_CHANGED: u32 = 1 << 8;
}

/// The element facts the style computation's box-type transformation and element style
/// adjustments read. Mirrors C++ `ElementStyleAdjustmentFact`.
pub mod element_adjustment_fact {
    pub const IS_BR: u32 = 1 << 0;
    pub const IS_WBR: u32 = 1 << 1;
    pub const DISALLOW_DISPLAY_CONTENTS: u32 = 1 << 2;
    pub const REWRITE_INLINE_FLOW: u32 = 1 << 3;
    pub const IS_BUTTON: u32 = 1 << 4;
    pub const FORCE_LINE_HEIGHT_NORMAL: u32 = 1 << 5;
    pub const CHECK_INPUT_LINE_HEIGHT: u32 = 1 << 6;
    pub const HIDE_AUDIO_WITHOUT_CONTROLS: u32 = 1 << 7;
    pub const IS_TABLE: u32 = 1 << 8;
    pub const FORCE_POSITION_STATIC: u32 = 1 << 9;
    pub const FORCE_SYMBOL_DISPLAY_INLINE: u32 = 1 << 10;
    pub const IS_MATHML: u32 = 1 << 11;
    pub const IS_MATHML_MTABLE: u32 = 1 << 12;
    pub const IS_MATHML_MTR: u32 = 1 << 13;
    pub const IS_MATHML_MTD: u32 = 1 << 14;
    pub const IS_TH: u32 = 1 << 15;
    pub const IS_DOCUMENT_ELEMENT: u32 = 1 << 16;
    pub const HAS_ANIMATIONS: u32 = 1 << 17;
    /// An SVG graphics element folds its own transform into its SVG container's layout, which the
    /// engine's damage for the element reads.
    pub const IS_SVG_GRAPHICS_ELEMENT: u32 = 1 << 18;
    /// The element stands for an element-reference pseudo-element of its shadow host, whose
    /// style C++ computes and installs on it.
    pub const IS_SHADOW_HOST_PSEUDO_ELEMENT: u32 = 1 << 19;
    /// An HTML `<body>`. The root's first one propagates its overflow to the viewport, which the
    /// engine's damage for the element reads.
    pub const IS_HTML_BODY_ELEMENT: u32 = 1 << 20;
    // The element types layout tree construction branches on. An element's type is fixed when it
    // is created, so the store holds these rather than the tree builder asking the DOM for them.
    pub const IS_SVG_ELEMENT: u32 = 1 << 21;
    pub const IS_SVG_SWITCH_ELEMENT: u32 = 1 << 22;
    pub const IS_SVG_CONTAINER: u32 = 1 << 23;
    pub const REQUIRES_SVG_CONTAINER: u32 = 1 << 24;
    pub const IS_SVG_FOREIGN_OBJECT_ELEMENT: u32 = 1 << 25;
    pub const IS_SVG_MASK_ELEMENT: u32 = 1 << 26;
    pub const IS_SVG_CLIP_PATH_ELEMENT: u32 = 1 << 27;
    pub const IS_SVG_PATTERN_ELEMENT: u32 = 1 << 28;
    /// Whether the element is rendered in the top layer. Unlike the type facts above it moves
    /// during the element's lifetime, and every move is recorded where the top layer is maintained.
    pub const RENDERED_IN_TOP_LAYER: u32 = 1 << 29;
}

/// What a layout row records about the element it is built for at the moment it is allocated. The
/// tree build reads these out of the mirror rather than out of the DOM node.
/// Mirrors the C++ `ElementConstructionFact`.
pub mod element_construction_fact {
    pub const IS_HTML_INPUT_ELEMENT: u32 = 1 << 0;
    pub const IS_HTML_HTML_ELEMENT: u32 = 1 << 1;
    pub const IS_IN_USER_AGENT_SHADOW_TREE: u32 = 1 << 2;
    pub const USES_BUTTON_LAYOUT: u32 = 1 << 3;
    pub const IS_EDITING_HOST: u32 = 1 << 4;
    pub const IS_BODY: u32 = 1 << 5;
    /// Also an `element_adjustment_fact`, which the style computation reads. A row is built out
    /// of this word alone, so the fact is published into both rather than read across two.
    pub const IS_DOCUMENT_ELEMENT: u32 = 1 << 6;
    /// The row answers for the areas of the image map its element is associated with, whatever box
    /// the element ends up with - an image rendering as its alt text still answers for its map.
    pub const IS_HTML_IMAGE_ELEMENT: u32 = 1 << 7;
}

/// Which principal box an element asks for before its computed style has a say. The element's own
/// type and state decide this; the tree build resolves it against the computed `display` and
/// `appearance`. Mirrors the C++ `CSS::ElementBoxKind`; it crosses the boundary as its raw byte.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub(crate) enum ElementBoxKind {
    /// The computed display decides the box on its own.
    FromDisplay = 0,
    /// The element generates no box, whatever its display says.
    NoBox = 1,
    Break = 2,
    FieldSet = 3,
    Legend = 4,
    Audio = 5,
    Video = 6,
    Canvas = 7,
    NavigableContainerViewport = 8,
    TextArea = 9,
    Image = 10,
    SvgGraphics = 11,
    SvgSvg = 12,
    SvgText = 13,
    SvgTextPath = 14,
    SvgForeignObject = 15,
    SvgImage = 16,
    SvgGeometry = 17,
    // An input's native widget. `appearance: none` suppresses it, and then the computed display
    // decides the box like it does for any other element.
    InputButton = 18,
    InputCheckBox = 19,
    InputRadioButton = 20,
    InputRange = 21,
    InputText = 22,
}

impl ElementBoxKind {
    /// The kind the mirror holds under `raw`. A node the mirror holds nothing for asks for
    /// nothing in particular, which is what a fresh column reads as.
    #[must_use]
    pub(crate) fn from_raw(raw: u8) -> Self {
        match raw {
            1 => Self::NoBox,
            2 => Self::Break,
            3 => Self::FieldSet,
            4 => Self::Legend,
            5 => Self::Audio,
            6 => Self::Video,
            7 => Self::Canvas,
            8 => Self::NavigableContainerViewport,
            9 => Self::TextArea,
            10 => Self::Image,
            11 => Self::SvgGraphics,
            12 => Self::SvgSvg,
            13 => Self::SvgText,
            14 => Self::SvgTextPath,
            15 => Self::SvgForeignObject,
            16 => Self::SvgImage,
            17 => Self::SvgGeometry,
            18 => Self::InputButton,
            19 => Self::InputCheckBox,
            20 => Self::InputRadioButton,
            21 => Self::InputRange,
            22 => Self::InputText,
            _ => Self::FromDisplay,
        }
    }

    /// Whether `appearance: none` suppresses the box this kind asks for.
    #[must_use]
    pub(crate) fn is_suppressed_by_appearance_none(self) -> bool {
        matches!(
            self,
            Self::InputButton | Self::InputCheckBox | Self::InputRadioButton | Self::InputRange | Self::InputText
        )
    }
}

/// Which local fact a feature delta describes.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiFeatureKind {
    TagName = 0,
    Id = 1,
    Class = 2,
    Attribute = 3,
    /// The ASCII-lowercase folding of the element's local name, recorded only when it differs from
    /// the name itself. It is what makes a case-insensitively written type selector reach the
    /// element without every such selector having to dispatch universally.
    FoldedTagName = 4,
    /// Whether the element has no children at all. It is not a feature of any one child, which is
    /// why a text node arriving publishes it: `:empty` is about the element, and a text node
    /// changes it while connecting no element for a tree delta to be recorded from.
    Emptiness = 5,
}

/// How a feature value is represented on one side of a change.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiFeatureValueKind {
    Absent = 0,
    /// Present, but the payload is not internable; an exact test reads the live DOM.
    Present = 1,
    Atom = 2,
    /// Present, with a value that differs from the one the fact held before. Attribute values do
    /// not cross as text - an exact test reads the live DOM - but a change to one is still a
    /// change, and a delta that reported presence on both sides would cancel in the journal and
    /// invalidate nothing.
    ChangedValue = 3,
}

/// One change to a local selector feature: a tag name, an ID, one class, or one attribute.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiLocalFeatureDelta {
    pub node: u32,
    pub feature_kind: FfiFeatureKind,
    /// The class atom or attribute-name atom the key is about. Unused for tag names and IDs.
    pub name_atom: u32,
    pub old_kind: FfiFeatureValueKind,
    pub old_atom: u32,
    pub new_kind: FfiFeatureValueKind,
    pub new_atom: u32,
}

// An element or document state fact. Values mirror `StateFact` one for one, so the boundary can
// publish every boolean pseudo-class the parser can produce.
include!(concat!(env!("OUT_DIR"), "/ffi_state_fact_generated.rs"));

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiStateDelta {
    pub node: u32,
    pub fact: FfiStateFact,
    pub new_value: bool,
}

/// Which element-sourced declaration block changed. Each kind keeps its language-defined cascade
/// placement; a presentational hint is not inline style wearing a different name.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiElementDeclarationKind {
    InlineStyle = 0,
    PresentationalHint = 1,
    SvgPresentationAttribute = 2,
}

/// One change to a declaration block sourced from a style node. Zero means "no block".
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiElementDeclarationDelta {
    pub node: u32,
    pub kind: FfiElementDeclarationKind,
    pub old_block: u32,
    pub new_block: u32,
}

/// One exact non-selector style reaction for an element.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiElementStyleInput {
    pub style_node: u32,
    pub reaction: u8,
    pub inherited_style_groups: u8,
}

// SAFETY: the six transaction row types are pointer-free repr(C) with alignment four, and their
// enum fields are recorded only from live FFI values, so raw bytes round-trip on the capturing
// host (the RawRecord contract).
unsafe impl super::record_replay::RawRecord for FfiTreeDelta {}
unsafe impl super::record_replay::RawRecord for FfiElementArrival {}
unsafe impl super::record_replay::RawRecord for FfiLocalFeatureDelta {}
unsafe impl super::record_replay::RawRecord for FfiStateDelta {}
unsafe impl super::record_replay::RawRecord for FfiElementDeclarationDelta {}
unsafe impl super::record_replay::RawRecord for FfiElementStyleInput {}

/// Which fact of the mirror one host fact write replaces. See [`FfiHostFactWrite`].
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiHostFactKind {
    /// `node` takes its place in `parent`'s DOM child sequence after `previous_sibling`.
    LinkInDomOrder = 0,
    /// `node` leaves `parent`'s DOM child sequence.
    UnlinkFromDomOrder = 1,
    /// The text node `node` disconnected, and its identity retires.
    RetireText = 2,
    /// `value` says whether the text node's data is nothing but ASCII whitespace.
    TextIsAsciiWhitespace = 3,
    /// `value` says whether the text node sits in a user-agent shadow tree.
    TextIsInUserAgentShadowTree = 4,
    /// `value` says whether the text node holds the value of a password input.
    TextIsPasswordInput = 5,
    /// `data` is the characters the text node now holds.
    TextData = 6,
    /// `facts` is the element's style adjustment facts.
    ElementAdjustmentFacts = 7,
    /// `value` is the element-reference pseudo kind the element represents, plus one.
    ElementAssociatedPseudoKind = 8,
    /// `facts` is the element's construction facts, and `value` the box kind it asks for.
    ElementConstructionFacts = 9,
    /// `data` is a snapshot of the element's inline style declarations, or null for none.
    ElementInlineStyleProperties = 10,
    /// The document takes the atom `facts` the host acquired for the raw name `data`, with the
    /// reference the acquisition took. See `style_engine_acquire_host_atom`.
    AdoptAtom = 11,
    /// The host minted the element identity `node` from its grant. `value` says whether it stands in
    /// the tree only to be named by relations.
    MintElement = 12,
    /// The host minted the text identity `node` from its grant.
    MintText = 13,
    /// `data` points at an `FfiReplacedContentInput`: what the element gives the natural size of
    /// its replaced content.
    ElementReplacedContentInput = 14,
    /// The document takes the atom `facts` the host acquired for the name atom `parent` qualified by
    /// the namespace atom `node`, with the reference the acquisition took. See
    /// `style_engine_acquire_host_qualified_atom`.
    AdoptQualifiedAtom = 15,
    /// `data` is the unique id the document knows the node `node` by, which a box built for one of
    /// the node's pseudo-elements answers by.
    ElementUniqueNodeId = 16,
    /// `value` is how a row built for the node `node` is painted and hit-tested, recorded as the
    /// node arrives and as it changes.
    NodeDomPaintFacts = 17,
    /// `facts` holds the column span of the table cell or column `node` in its low half and its row
    /// span in its high half, and `parent` its raw column span, recorded as for `NodeDomPaintFacts`.
    ElementTableSpans = 18,
    /// The shadow root `parent` is attached to the host `node`.
    ShadowRoot = 19,
    /// `node` is the root of the tree scope `facts`.
    TreeScopeRoot = 20,
    /// The tree scope `facts` is styled by the document's sheets rather than its own.
    TreeScopeUsesDocumentSheets = 21,
    /// `parent` is assigned to the slot `node`. A list starts with a write whose `value` is 1, and
    /// the writes after it for the same slot with `value` 0 go on with it. An empty list is one
    /// write with no `parent`.
    SlotAssignedNode = 22,
    /// `node` is in the document's top layer, a list at a time as for `SlotAssignedNode`.
    TopLayerElement = 23,
    /// A container query selected the element `node` as its size query container.
    SizeQueryContainer = 24,
    /// A size query or container-relative unit decided the style of the element `node`.
    StyleDependsOnSizeContainerQuery = 25,
    /// A moved custom-property environment computes the element `node` again: its style reads the
    /// environment other than through `var()`.
    RecomputesOnEnvironmentMove = 26,
    /// A style computation asked about the container `node` before it had a committed box.
    SizeContainerNeedsEvaluationAfterLayout = 27,
    /// A keyframe-borne `inherit` on a non-inherited property marked the children of `node` as
    /// explicitly inheriting it.
    ChildrenExplicitlyInherit = 28,
    /// `facts` is the language atom of the element `node`, or of no element for none, and `data`
    /// the tag `:lang()` compares against, as for `TextData` (empty once the language has one).
    ElementLanguage = 29,
    /// `value` says whether the conditions of the rule the host knows by the identity `data` hold:
    /// what a media evaluation found flipped.
    RuleConditionsHold = 30,
    /// The element `node` exposes the part named by the atom `facts` for the host `parent`, a
    /// list at a time as for `SlotAssignedNode`. An empty list is one write with no `parent`.
    ElementParts = 31,
    /// `data` is a snapshot of the presentational hints of the element `node`, as for
    /// `ElementInlineStyleProperties`, and `value` their `FfiElementDeclarationKind`.
    ElementPresentationalHints = 32,
    /// The viewport moved: every element or pseudo row whose style depends on viewport metrics
    /// takes the derived reaction `value`.
    ViewportDependentStyleInputs = 33,
    /// `facts` is the atom the element `node` is known by to `getElementById`, or 0 for none.
    ElementIdName = 34,
    /// `facts` is the directionality atom of the element `node`.
    ElementDirectionality = 35,
    /// `value` is the heading level of the element `node`, 0 for no heading.
    ElementHeadingLevel = 36,
    /// The element `node` is in the custom state named by the atom `facts`, a list at a time as for
    /// `SlotAssignedNode`. An empty list is one write with no `facts`.
    ElementCustomState = 37,
    /// `parent` is the outermost host the parts of the element `node` reach.
    ElementPartExposure = 38,
}

/// Which element an `FfiReplacedContentInput` holds the values of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiReplacedContentInputKind {
    None = 0,
    /// A `<textarea>`: `first` is its `cols`, and `second` its `rows`.
    TextArea = 1,
    /// An `<input>` whose type makes it no text entry widget: `first` is its `size`.
    Input = 2,
    /// An `<input>` whose type makes it a text entry widget: `first` is its `size`.
    TextEntryInput = 3,
    /// A `<canvas>`: `first` is its `width`, and `second` its `height`.
    Canvas = 4,
    /// The natural size of what an element has loaded, such as a video's, in raw fixed-point CSS
    /// pixels: `first` is its width, `second` its height, and `third` and `fourth` the numerator
    /// and denominator of its aspect ratio. `present` says which of them it has.
    NaturalSize = 5,
    /// An SVG `<image>` whose image has decoded: its natural size, as `NaturalSize`.
    DecodedSvgImage = 6,
}

/// The bits of an `FfiReplacedContentInput`'s `present`, for the kinds whose values can be missing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiReplacedContentInputPresent {
    First = 1 << 0,
    Second = 1 << 1,
    ThirdAndFourth = 1 << 2,
}

/// What an element gives the natural size of its replaced content, which layout resolves against
/// the style of the element's box. What the values mean depends on `kind`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FfiReplacedContentInput {
    pub kind: FfiReplacedContentInputKind,
    pub present: u8,
    pub first: u32,
    pub second: u32,
    pub third: u32,
    pub fourth: u32,
}

/// One write the host made to a fact of the mirror, which the engine applies with the next
/// transaction in the order the host made it. These are facts the DOM holds and nothing selects
/// or invalidates on: the DOM child sequence, what a text node holds, and what an element is.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiHostFactWrite {
    pub kind: FfiHostFactKind,
    pub value: u8,
    pub node: u32,
    pub parent: u32,
    pub previous_sibling: u32,
    pub facts: u32,
    /// For `TextData`, a raw `AK::Utf16String`, and for `ElementInlineStyleProperties`, an Arc-owned
    /// `DeclarationBlockData`. The write transfers its one reference to the engine.
    pub data: usize,
}

/// A style reaction the host applied to an element as it installed a batch, which the next
/// transaction derives the element's children's reactions from. See
/// `StyleEngineState::record_applied_style_reaction`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FfiAppliedStyleReaction {
    pub node: u32,
    pub reaction: u8,
    pub inherited_style_groups_changed: u8,
    pub facts: u32,
}

/// An element whose synthetic pseudo-elements the host left for the next pass to settle, as it
/// installed a composition they inherit from after the pass that settled the element. See
/// `StyleEngineState::settle_pseudo_elements_in_next_pass`.
#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub struct FfiPseudoElementSettle {
    pub node: u32,
    pub old_is_list_item: bool,
    pub held_pseudo_records: [u64; RETRY_PSEUDO_RECORD_SLOTS],
}

/// What the host's install of a batch hands back for the next transaction, the next wave of its
/// style update: the reactions it applied, and the elements whose pseudo-elements it left to settle.
/// Every array is borrowed for the duration of the call.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiInstallFeedback {
    pub applied_style_reactions: *const FfiAppliedStyleReaction,
    pub applied_style_reaction_count: usize,
    pub pseudo_element_settles: *const FfiPseudoElementSettle,
    pub pseudo_element_settle_count: usize,
}

impl FfiInstallFeedback {
    /// # Safety
    /// Each array must point at its stated count for `'a`.
    unsafe fn borrow<'a>(&self) -> InstallFeedback<'a> {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            InstallFeedback {
                applied_style_reactions: borrow(self.applied_style_reactions, self.applied_style_reaction_count),
                pseudo_element_settles: borrow(self.pseudo_element_settles, self.pseudo_element_settle_count),
            }
        }
    }
}

/// The rows of an [`FfiInstallFeedback`].
#[derive(Clone, Copy)]
pub(crate) struct InstallFeedback<'a> {
    applied_style_reactions: &'a [FfiAppliedStyleReaction],
    pseudo_element_settles: &'a [FfiPseudoElementSettle],
}

impl InstallFeedback<'_> {
    fn is_empty(&self) -> bool {
        self.applied_style_reactions.is_empty() && self.pseudo_element_settles.is_empty()
    }
}

/// One flat style input transaction. Every array is borrowed for the duration of the call.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiStyleInputTransaction {
    pub tree_deltas: *const FfiTreeDelta,
    pub tree_delta_count: usize,
    pub element_arrivals: *const FfiElementArrival,
    pub element_arrival_count: usize,
    pub arrival_custom_state_atoms: *const u32,
    pub arrival_custom_state_atom_count: usize,
    pub local_feature_deltas: *const FfiLocalFeatureDelta,
    pub local_feature_delta_count: usize,
    pub state_deltas: *const FfiStateDelta,
    pub state_delta_count: usize,
    pub element_declaration_deltas: *const FfiElementDeclarationDelta,
    pub element_declaration_delta_count: usize,
    pub element_style_inputs: *const FfiElementStyleInput,
    pub element_style_input_count: usize,
    pub host_fact_writes: *const FfiHostFactWrite,
    pub host_fact_write_count: usize,
    /// Where the engine writes the element identities it grants the host to mint from, and how many
    /// the host asks for. See `StyleEngineState::grant_style_nodes`.
    pub element_identity_grant: *mut u32,
    pub element_identity_grant_count: usize,
    /// Where the engine writes the text identities it grants the host, and how many it asks for.
    pub text_identity_grant: *mut u32,
    pub text_identity_grant_count: usize,
}

/// Device class selecting the document's memory budget coefficients.
#[derive(Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiDeviceClass {
    ForegroundDesktop = 0,
}

impl FfiDeviceClass {
    fn decode(self) -> DeviceClass {
        match self {
            Self::ForegroundDesktop => DeviceClass::ForegroundDesktop,
        }
    }
}

/// Cascade origin of a sheet, as the boundary names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum FfiCascadeOrigin {
    Author = 0,
    AuthorPresentationalHint = 1,
    User = 2,
    UserAgent = 3,
}

impl FfiCascadeOrigin {
    fn decode(self) -> CascadeOrigin {
        match self {
            Self::Author => CascadeOrigin::Author,
            Self::AuthorPresentationalHint => CascadeOrigin::AuthorPresentationalHint,
            Self::User => CascadeOrigin::User,
            Self::UserAgent => CascadeOrigin::UserAgent,
        }
    }
}

fn decode_feature_key(delta: &FfiLocalFeatureDelta) -> LocalFeatureKey {
    match delta.feature_kind {
        FfiFeatureKind::TagName => LocalFeatureKey::TagName,
        FfiFeatureKind::FoldedTagName => LocalFeatureKey::FoldedTagName,
        FfiFeatureKind::Emptiness => LocalFeatureKey::Emptiness,
        FfiFeatureKind::Id => LocalFeatureKey::Id,
        FfiFeatureKind::Class => LocalFeatureKey::Class(StyleAtomID(delta.name_atom)),
        FfiFeatureKind::Attribute => LocalFeatureKey::Attribute(StyleAtomID(delta.name_atom)),
    }
}

fn decode_feature_value(kind: FfiFeatureValueKind, atom: u32) -> FeatureValue {
    match kind {
        FfiFeatureValueKind::Absent => FeatureValue::Absent,
        FfiFeatureValueKind::Present => FeatureValue::Present,
        FfiFeatureValueKind::Atom => FeatureValue::Atom(StyleAtomID(atom)),
        FfiFeatureValueKind::ChangedValue => FeatureValue::ChangedValue,
    }
}

fn decode_element_declaration_kind(kind: FfiElementDeclarationKind) -> ElementDeclarationKind {
    match kind {
        FfiElementDeclarationKind::InlineStyle => ElementDeclarationKind::InlineStyle,
        FfiElementDeclarationKind::PresentationalHint => ElementDeclarationKind::PresentationalHint,
        FfiElementDeclarationKind::SvgPresentationAttribute => ElementDeclarationKind::SvgPresentationAttribute,
    }
}

fn write_recording_atom_mappings(engine: &StyleEngine, payload: &mut super::record_replay::PayloadWriter) {
    let mappings = engine.recording_atom_mappings();
    enum Mapping {
        Atom { token: u64, atom: u32 },
        Qualified { namespace: u32, name: u32, atom: u32 },
    }
    let mut ordered = mappings
        .atoms
        .into_iter()
        .map(|(token, atom)| Mapping::Atom { token, atom })
        .chain(
            mappings
                .qualified_atoms
                .into_iter()
                .map(|(namespace, name, atom)| Mapping::Qualified { namespace, name, atom }),
        )
        .collect::<Vec<_>>();
    ordered.sort_unstable_by_key(|mapping| match mapping {
        Mapping::Atom { atom, .. } | Mapping::Qualified { atom, .. } => *atom,
    });
    payload.write_length(ordered.len());
    for mapping in ordered {
        match mapping {
            Mapping::Atom { token, atom } => {
                payload.write_u8(0);
                payload.write_u64(token);
                payload.write_u32(atom);
            }
            Mapping::Qualified { namespace, name, atom } => {
                payload.write_u8(1);
                payload.write_u32(namespace);
                payload.write_u32(name);
                payload.write_u32(atom);
            }
        }
    }
}

fn write_declared_properties(declared: &[DeclaredProperty], payload: &mut super::record_replay::PayloadWriter) {
    payload.write_length(declared.len());
    for property in declared {
        payload.write_u16(property.property);
        payload.write_bool(property.important);
        payload.write_u8(match property.operator {
            CascadeOperator::Declared => 0,
            CascadeOperator::Inherit => 1,
            CascadeOperator::Initial => 2,
            CascadeOperator::Unset => 3,
            CascadeOperator::Revert => 4,
            CascadeOperator::RevertLayer => 5,
        });
        payload.write_u64(property.value.0);
    }
}

fn write_custom_declarations(declared: &[CustomDeclaration], payload: &mut super::record_replay::PayloadWriter) {
    payload.write_length(declared.len());
    for property in declared {
        payload.write_u32(property.name.0);
        payload.write_bool(property.important);
        payload.write_u8(match property.operator {
            CascadeOperator::Declared => 0,
            CascadeOperator::Inherit => 1,
            CascadeOperator::Initial => 2,
            CascadeOperator::Unset => 3,
            CascadeOperator::Revert => 4,
            CascadeOperator::RevertLayer => 5,
        });
        payload.write_u64(property.value.0);
    }
}

impl StyleEngineState {
    fn install_ffi_style_node_query(&mut self, nodes: Vec<u32>) -> FfiStyleNodeSlice {
        let bytes = (nodes.capacity() * size_of::<u32>()) as u64;
        self.host.ffi_style_node_query = nodes;
        self.host
            .ffi_style_node_query_memory
            .resize_required_to(&mut self.retained.memory, bytes);
        FfiStyleNodeSlice {
            nodes: self.host.ffi_style_node_query.as_ptr(),
            count: self.host.ffi_style_node_query.len(),
        }
    }

    fn clear_ffi_style_node_query(&mut self) {
        self.host.ffi_style_node_query = Vec::new();
        self.host.ffi_style_node_query_memory.shrink_to(0);
    }

    fn install_ffi_style_transaction_output(&mut self, output: FfiStyleTransactionOutput) {
        let bytes = (output.answers.capacity() * size_of::<FfiStyleDelta>()
            + output.reclaimed_style_atoms.capacity() * size_of::<FfiReclaimedStyleAtom>()) as u64;
        self.host.ffi_style_transaction_output = output;
        self.host
            .ffi_style_transaction_output_memory
            .resize_required_to(&mut self.retained.memory, bytes);
    }

    pub(super) fn clear_ffi_style_transaction_output(&mut self) {
        self.host.ffi_style_transaction_output = FfiStyleTransactionOutput::default();
        self.host.ffi_style_transaction_output_memory.shrink_to(0);
    }

    /// Apply one flat transaction. Tree deltas are staged in arrival order so derived neighbour
    /// rows follow the live tree step by step, while the journal normalizes for discovery.
    #[allow(clippy::too_many_arguments)]
    pub fn apply_transaction_batch(
        &mut self,
        tree_deltas: &[FfiTreeDelta],
        arrival_columns: (&[FfiElementArrival], &[u32]),
        local_feature_deltas: &[FfiLocalFeatureDelta],
        state_deltas: &[FfiStateDelta],
        element_declaration_deltas: &[FfiElementDeclarationDelta],
        element_style_inputs: &[FfiElementStyleInput],
        counters: &mut Counters,
    ) {
        let (element_arrivals, arrival_custom_state_atoms) = arrival_columns;
        let largest_element_index = tree_deltas
            .iter()
            .filter_map(|delta| StyleNodeID::from_raw(delta.node)?.element_index())
            .max()
            .unwrap_or(0);
        let mut arriving_nodes = vec![false; largest_element_index as usize + 1];
        let mut initial_tree_was_bulk_loaded = false;
        if self.can_bulk_load_initial_tree() && !tree_deltas.is_empty() {
            let mut initial_arrivals = Vec::with_capacity(tree_deltas.len());
            let mut initial_document_root = None;
            let mut can_bulk_load_initial_tree = true;
            for delta in tree_deltas {
                let Some(node) = StyleNodeID::from_raw(delta.node) else {
                    can_bulk_load_initial_tree = false;
                    continue;
                };
                let Some(index) = node.element_index() else {
                    can_bulk_load_initial_tree = false;
                    continue;
                };
                let is_unique_arrival = !delta.old_connected && delta.new_connected && !arriving_nodes[index as usize];
                can_bulk_load_initial_tree &= is_unique_arrival;
                arriving_nodes[index as usize] = is_unique_arrival;
                if is_unique_arrival {
                    let relations = delta.new_relations.decode();
                    if relations.parent.is_none() && relations.tree_scope == TreeScopeID::DOCUMENT {
                        can_bulk_load_initial_tree &= initial_document_root.replace(node).is_none();
                    }
                    initial_arrivals.push((node, relations));
                }
            }
            if can_bulk_load_initial_tree && let Some(document_root) = initial_document_root {
                self.bulk_load_initial_tree(document_root, &initial_arrivals, counters);
                initial_tree_was_bulk_loaded = true;
            }
        }
        if !initial_tree_was_bulk_loaded {
            self.host.initial_tree_batch_applied |= !tree_deltas.is_empty();
            for delta in tree_deltas {
                let Some(node) = StyleNodeID::from_raw(delta.node) else {
                    continue;
                };
                let old = delta.old_connected.then(|| delta.old_relations.decode());
                let new = delta.new_connected.then(|| delta.new_relations.decode());
                self.record_tree_delta(node, old, new, counters);
            }
            for delta in tree_deltas {
                let Some(node) = StyleNodeID::from_raw(delta.node) else {
                    continue;
                };
                let Some(index) = node.element_index() else {
                    continue;
                };
                arriving_nodes[index as usize] = self.node_arrival_is_pending(node);
            }
        }
        let node_is_arriving = |node: StyleNodeID| {
            node.element_index()
                .and_then(|index| arriving_nodes.get(index as usize))
                .copied()
                .unwrap_or(false)
        };

        if !element_arrivals.is_empty() {
            for arrival in element_arrivals {
                let Some(node) = StyleNodeID::from_raw(arrival.node) else {
                    debug_assert!(false, "an element arrival named an invalid style node");
                    continue;
                };
                let Some(custom_state_end) = arrival.custom_state_offset.checked_add(arrival.custom_state_count) else {
                    debug_assert!(false, "an element arrival custom-state range overflowed");
                    continue;
                };
                let Ok(custom_state_range) = usize::try_from(arrival.custom_state_offset)
                    .and_then(|start| usize::try_from(custom_state_end).map(|end| start..end))
                else {
                    debug_assert!(false, "an element arrival custom-state range exceeded usize");
                    continue;
                };
                let Some(custom_states) = arrival_custom_state_atoms.get(custom_state_range) else {
                    debug_assert!(
                        false,
                        "an element arrival named custom states outside the shared atom column"
                    );
                    continue;
                };
                let custom_states = custom_states.iter().copied().map(StyleAtomID).collect::<Vec<_>>();
                self.record_element_arrival(node, arrival, &custom_states, node_is_arriving(node), counters);
            }
            self.settle_batched_inputs(counters);
        }

        for delta in local_feature_deltas {
            let Some(node) = StyleNodeID::from_raw(delta.node) else {
                continue;
            };
            self.record_batched_input(
                InputKey::LocalFeature(node, decode_feature_key(delta)),
                InputValue::Feature(decode_feature_value(delta.old_kind, delta.old_atom)),
                InputValue::Feature(decode_feature_value(delta.new_kind, delta.new_atom)),
                node_is_arriving(node),
                counters,
            );
        }
        self.settle_batched_inputs(counters);

        for delta in state_deltas {
            let Some(node) = StyleNodeID::from_raw(delta.node) else {
                continue;
            };
            self.record_batched_state(
                node,
                decode_state_fact(delta.fact),
                delta.new_value,
                node_is_arriving(node),
                counters,
            );
        }
        self.settle_batched_inputs(counters);

        for delta in element_declaration_deltas {
            let Some(node) = StyleNodeID::from_raw(delta.node) else {
                continue;
            };
            // Zero means the node has no declaration block of that kind on that side.
            let block = |raw: u32| (raw != 0).then_some(DeclarationBlockID(raw));
            // The block's contents moved even where the host's object for it did not, so what makes
            // this a change is a fresh version of the block, which is minted here rather than by the
            // host: the host says only that the node has one.
            let new_block =
                (delta.new_block != 0).then(|| DeclarationBlockID(self.retained.next_declaration_block_version()));
            self.record_input(
                InputKey::ElementDeclaration(node, decode_element_declaration_kind(delta.kind)),
                InputValue::ElementDeclaration(block(delta.old_block)),
                InputValue::ElementDeclaration(new_block),
                counters,
            );
        }
        for input in element_style_inputs {
            let Some(node) = StyleNodeID::from_raw(input.style_node) else {
                continue;
            };
            self.record_input(
                InputKey::ElementStyleInput(node),
                InputValue::ElementStyleInput {
                    reaction: 0,
                    inherited_style_groups: 0,
                },
                InputValue::ElementStyleInput {
                    reaction: input.reaction,
                    inherited_style_groups: input.inherited_style_groups,
                },
                counters,
            );
        }
    }
}

/// Creates one document's style engine, which the arena `arena` of the document's render state links from the start,
/// with the document thread's pin table lent to it (see [`super::host_pins`]) and, for a document that computes style,
/// `resolve` installed as its font resolver. Writes the recording stream the engine records under, or zero, to
/// `recording_stream`.
///
/// # Safety
/// `arena` must be the arena of a render state the owner just created, which links no engine and outlives this one,
/// `pins` must come from [`style_record_host_pins_create`] and outlive the engine, and `recording_stream` must be
/// valid for a write.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_create(
    arena: *mut c_void,
    device_class: FfiDeviceClass,
    pins: *mut c_void,
    resolve: Option<
        unsafe extern "C" fn(usize, *const c_void, *const FfiFontResolutionRequest, *mut FfiResolvedFont, usize),
    >,
    recording_stream: *mut u64,
) -> *mut c_void {
    super::seal::note_engine_call("style_engine_create");
    let device_class = device_class.decode();
    let mut engine = Box::new(StyleEngine::new(device_class));
    engine.begin_recording(device_class);
    engine.record_boundary_call(EventKind::SetComputedGroupDependencyMasks, |payload| {
        let mapping = crate::css::computed_values::property_dependency_masks_snapshot();
        payload.write_bool(mapping.is_some());
        if let Some((first_property, masks, output_masks)) = mapping {
            payload.write_u16(first_property);
            payload.write_u32_slice(masks);
            payload.write_u32_slice(output_masks);
        }
    });
    // SAFETY: Guaranteed by the caller.
    let handle = unsafe { super::host_pins::HostPinsHandle::new(pins.cast()) };
    engine
        .computed_group_sets
        .lend_host_pins(super::host_pins::HostPinsLend::Lent(handle));
    if let Some(resolve) = resolve {
        engine.host.font_resolver = Some(super::font_resolution::FontResolverHost::new(resolve));
        engine.retained.font_resolution = Some(super::font_resolution::FontResolutionCache::default());
    }
    // SAFETY: Guaranteed by the caller.
    unsafe { recording_stream.write(engine.recording_id().unwrap_or(0)) };
    // SAFETY: Guaranteed by the caller.
    unsafe { StyleEngineHandle::create(engine, arena) }.into_ffi()
}

/// Creates the document thread's style-record pin table. See [`super::host_pins`].
#[unsafe(no_mangle)]
pub extern "C" fn style_record_host_pins_create() -> *mut c_void {
    Box::into_raw(Box::<super::host_pins::HostStyleRecordPins>::default()).cast()
}

/// # Safety
/// `pins` must come from [`style_record_host_pins_create`], and the engine it was lent to must
/// already be destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_record_host_pins_destroy(pins: *mut c_void) {
    drop(unsafe { Box::from_raw(pins.cast::<super::host_pins::HostStyleRecordPins>()) });
}

/// Pins a style record for the document thread's readers. Enters no engine, so it never waits for
/// a pass in flight.
///
/// # Safety
/// `pins` must be a live table from [`style_record_host_pins_create`], on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_record_host_pins_pin(pins: *const c_void, style_record: u64) {
    unsafe { &*pins.cast::<super::host_pins::HostStyleRecordPins>() }.pin(style_record);
}

/// Releases a pin [`style_record_host_pins_pin`] took. The engine reclaims the record once it next
/// reads the table and nothing else holds it.
///
/// # Safety
/// As for [`style_record_host_pins_pin`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_record_host_pins_unpin(pins: *const c_void, style_record: u64) {
    unsafe { &*pins.cast::<super::host_pins::HostStyleRecordPins>() }.unpin(style_record);
}

/// Promises a pin the document thread takes once the frame in flight is taken in. Until
/// [`style_record_host_pins_end_pin_waiting_for_frame`] says it has landed, the engine reclaims no
/// record.
///
/// # Safety
/// As for [`style_record_host_pins_pin`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_record_host_pins_begin_pin_waiting_for_frame(pins: *const c_void) {
    unsafe { &*pins.cast::<super::host_pins::HostStyleRecordPins>() }.begin_pin_waiting_for_frame();
}

/// Ends a wait [`style_record_host_pins_begin_pin_waiting_for_frame`] began.
///
/// # Safety
/// As for [`style_record_host_pins_pin`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_record_host_pins_end_pin_waiting_for_frame(pins: *const c_void) {
    unsafe { &*pins.cast::<super::host_pins::HostStyleRecordPins>() }.end_pin_waiting_for_frame();
}

/// Publishes the document's `@font-face` table for the generation this update computes against.
/// The engine holds its own reference, so resolving a font needs nothing the document owns.
///
/// # Safety
/// `engine` must point to a live style engine, and `snapshot` must be null or a live pointer from
/// `rust_font_face_snapshot_build`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_publish_font_face_snapshot(
    engine: StyleEngineInputHandle,
    snapshot: *const c_void,
    memo: usize,
) {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_publish_font_face_snapshot",
        crate::css::style::owner_calls::StyleQuery::PublishFontFaceSnapshot { snapshot, memo },
    );
}

/// Answers [`style_engine_publish_font_face_snapshot`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_publish_font_face_snapshot`].
pub(crate) unsafe fn owner_publish_font_face_snapshot(
    engine: &mut crate::css::style::StyleEngine,
    snapshot: *const c_void,
    memo: usize,
) {
    if engine
        .retained
        .font_cascade_memo
        .as_ref()
        .is_none_or(|held| held.address() != memo)
    {
        // SAFETY: The caller guarantees the memo is live.
        engine.retained.font_cascade_memo = unsafe { super::font_faces::RetainedFontCascadeMemo::retain(memo) };
    }
    let held = &mut engine.retained.font_face_snapshot;
    if held
        .as_ref()
        .is_some_and(|current| super::font_faces::as_pointer(current) == snapshot)
    {
        return;
    }
    // SAFETY: The caller guarantees the pointer is live.
    *held = unsafe { super::font_faces::retained(snapshot) };
}

/// Publishes the previous document-element font answer before style evaluation begins.
///
/// # Safety
/// `engine` must point to a live style engine.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_prepare_root_font_resolution(engine: StyleEngineInputHandle, generation: u64) {
    crate::css::style::owner_calls::send(
        engine,
        "style_engine_prepare_root_font_resolution",
        crate::css::style::owner_calls::EngineChange::PrepareRootFontResolution { generation },
    );
}

/// Answers [`style_engine_prepare_root_font_resolution`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_prepare_root_font_resolution`].
pub(crate) unsafe fn owner_prepare_root_font_resolution(engine: &mut crate::css::style::StyleEngine, generation: u64) {
    let state = &mut engine.state;
    let Some(request) = state
        .retained
        .root_font_request
        .as_ref()
        .map(|request| request.for_generation(generation))
    else {
        return;
    };
    let resolver = state
        .host
        .font_resolver
        .as_ref()
        .expect("a root request has a font resolver");
    let snapshot = state.retained.font_face_snapshot.clone();
    let memo = state
        .retained
        .font_cascade_memo
        .as_ref()
        .map_or(0, |memo| memo.address());
    let cache = state
        .retained
        .font_resolution
        .as_mut()
        .expect("a root request has a font resolution cache");
    resolver.refill(
        memo,
        snapshot.as_ref(),
        cache,
        vec![request],
        super::font_resolution::FontService::RootPreparation,
    );
}

/// Creates a replay engine whose atom keys are opaque capture tokens rather than live fly strings.
pub fn style_engine_create_for_replay(device_class: FfiDeviceClass) -> super::OwnedStyleEngine {
    abort_on_panic(|| super::OwnedStyleEngine::new(Box::new(StyleEngine::new_for_replay(device_class.decode()))))
}

/// Applies the memory policy used while producing a replay recording to an engine that runs only for a replay.
///
/// # Safety
/// `engine` must be a live engine made by `style_engine_create_for_replay`, which nothing else borrows meanwhile.
pub unsafe fn use_recording_memory_policy_for_replay(engine: StyleEngineHandle) {
    // SAFETY: Guaranteed by the caller.
    unsafe { engine.for_replay() }.memory.enable_recording_policy();
}

#[unsafe(no_mangle)]
pub extern "C" fn style_engine_verification_gate_bits() -> u8 {
    super::seal::note_engine_call("style_engine_verification_gate_bits");
    super::verification_gate_bits()
}

/// Publishes the `@keyframes` one style scope defines, by name, with the host's keyframe set for
/// each. The names travel packed into one buffer of code units with a length each, the way an
/// element's animation names do.
///
/// The host publishes every scope at the style update's begin boundary, once per rebuild of that
/// scope's rule cache, and holds a reference to what it published for as long as the table names
/// it. This is not a recorded boundary event: a keyframe set is a host pointer, which a replayed
/// engine could not be handed, and which it never asks for.
///
/// # Safety
/// `engine` must be live, and each buffer must hold the count it is given.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_set_tree_scope_animation_keyframes(
    engine: StyleEngineInputHandle,
    tree_scope: u32,
    shadow_root_identity: usize,
    name_lengths: *const u32,
    name_units: *const u16,
    name_unit_count: usize,
    count: usize,
    descriptions: *const FfiPublishedAnimationEffect,
    description_count: usize,
    keyframes: *const FfiPublishedAnimationKeyframe,
    keyframe_count: usize,
    declarations: *const FfiPublishedAnimationDeclaration,
    declaration_count: usize,
    custom_declarations: *const FfiPublishedAnimationCustomDeclaration,
    custom_declaration_count: usize,
    linear_points: *const FfiPublishedLinearEasingPoint,
    linear_point_count: usize,
    base_url_bytes: *const u8,
    base_url_byte_count: usize,
) {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_set_tree_scope_animation_keyframes",
        crate::css::style::owner_calls::StyleQuery::SetTreeScopeAnimationKeyframes {
            tree_scope,
            shadow_root_identity,
            name_lengths,
            name_units,
            name_unit_count,
            count,
            descriptions,
            description_count,
            keyframes,
            keyframe_count,
            declarations,
            declaration_count,
            custom_declarations,
            custom_declaration_count,
            linear_points,
            linear_point_count,
            base_url_bytes,
            base_url_byte_count,
        },
    );
}

/// Answers [`style_engine_set_tree_scope_animation_keyframes`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_set_tree_scope_animation_keyframes`].
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn owner_set_tree_scope_animation_keyframes(
    engine: &mut crate::css::style::StyleEngine,
    tree_scope: u32,
    shadow_root_identity: usize,
    name_lengths: *const u32,
    name_units: *const u16,
    name_unit_count: usize,
    count: usize,
    descriptions: *const FfiPublishedAnimationEffect,
    description_count: usize,
    keyframes: *const FfiPublishedAnimationKeyframe,
    keyframe_count: usize,
    declarations: *const FfiPublishedAnimationDeclaration,
    declaration_count: usize,
    custom_declarations: *const FfiPublishedAnimationCustomDeclaration,
    custom_declaration_count: usize,
    linear_points: *const FfiPublishedLinearEasingPoint,
    linear_point_count: usize,
    base_url_bytes: *const u8,
    base_url_byte_count: usize,
) {
    let name_lengths = match count {
        0 => &[][..],
        _ => unsafe { std::slice::from_raw_parts(name_lengths, count) },
    };
    let name_units = match name_unit_count {
        0 => &[][..],
        _ => unsafe { std::slice::from_raw_parts(name_units, name_unit_count) },
    };
    let buffers = unsafe {
        published_effect_buffers(
            descriptions,
            description_count,
            keyframes,
            keyframe_count,
            declarations,
            declaration_count,
            custom_declarations,
            custom_declaration_count,
            linear_points,
            linear_point_count,
            base_url_bytes,
            base_url_byte_count,
        )
    };
    unsafe {
        engine.set_tree_scope_animation_keyframes(
            TreeScopeID(tree_scope),
            shadow_root_identity,
            name_lengths,
            name_units,
            buffers,
        );
    }
    engine.count_animation_keyframe_scopes();
}

/// How one effect's description travels across the boundary: a header naming the ranges of the flat
/// keyframe and declaration buffers that belong to it.
#[repr(C)]
pub struct FfiPublishedAnimationEffect {
    pub identity: u64,
    pub generation: u64,
    pub flags: u32,
    pub first_keyframe: u32,
    pub keyframe_count: u32,
    pub base_url_offset: u32,
    pub base_url_length: u32,
}

/// One published keyframe, with its easing spelled out and its declarations named by range.
#[repr(C)]
pub struct FfiPublishedAnimationKeyframe {
    pub key: i64,
    pub easing_kind: u8,
    pub composite: u8,
    pub step_position: u8,
    pub interval_count: i32,
    pub x1: f64,
    pub y1: f64,
    pub x2: f64,
    pub y2: f64,
    pub first_linear_point: u32,
    pub linear_point_count: u32,
    pub first_declaration: u32,
    pub declaration_count: u32,
    /// NB: Additive, and the last thing on this keyframe: the custom properties it declares travel
    ///     in a buffer of their own, ranged exactly the way the longhand declarations above are.
    ///     A custom property is named rather than numbered, and its value composes against the
    ///     element's own environment rather than against a longhand table, so the two never mix.
    pub first_custom_declaration: u32,
    pub custom_declaration_count: u32,
    /// The keyframe's own `animation-timing-function` where it was written as a value the element
    /// has to substitute, or null. The easing above is then the one the keyframe runs if the value
    /// resolves to no easing.
    pub easing_value: *const c_void,
}

/// One published custom-property declaration of a keyframe.
///
/// The name is the raw one-word representation of the host's `Utf16FlyString`, borrowed for the
/// call and retained by the engine the way every published style payload is. `use_initial` marks
/// the keyframe the host synthesized to hold the element's own value, whose value - the element's
/// underlying value for the name - is not known until the element is sampled, and then the value
/// is null.
#[repr(C)]
pub struct FfiPublishedAnimationCustomDeclaration {
    pub name_raw: usize,
    pub use_initial: bool,
    pub value: *const std::ffi::c_void,
}

/// One control point of a published `linear()` easing.
#[repr(C)]
pub struct FfiPublishedLinearEasingPoint {
    pub input: f64,
    pub output: f64,
}

/// One published declaration. The value is a style value the host holds; the engine retains it, the
/// way every published style payload is retained rather than borrowed.
#[repr(C)]
pub struct FfiPublishedAnimationDeclaration {
    pub property_id: u16,
    pub use_initial: bool,
    pub value: *const std::ffi::c_void,
}

/// Describe one of an element's animation lists for the style stage.
///
/// Hand-written, like the `@keyframes` publication above, because a keyframe declaration carries a
/// style value the host holds and a replayed engine could not be handed one.
///
/// # Safety
/// `engine` must be live, every buffer must hold the count it is given, and every declaration's
/// value must be a live style value for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_set_element_animation_effect_descriptions(
    engine: StyleEngineInputHandle,
    node: u32,
    slot: u8,
    effects: *const FfiPublishedAnimationEffect,
    effect_count: usize,
    keyframes: *const FfiPublishedAnimationKeyframe,
    keyframe_count: usize,
    declarations: *const FfiPublishedAnimationDeclaration,
    declaration_count: usize,
    custom_declarations: *const FfiPublishedAnimationCustomDeclaration,
    custom_declaration_count: usize,
    linear_points: *const FfiPublishedLinearEasingPoint,
    linear_point_count: usize,
    base_url_bytes: *const u8,
    base_url_byte_count: usize,
) {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_set_element_animation_effect_descriptions",
        crate::css::style::owner_calls::StyleQuery::SetElementAnimationEffectDescriptions {
            node,
            slot,
            effects,
            effect_count,
            keyframes,
            keyframe_count,
            declarations,
            declaration_count,
            custom_declarations,
            custom_declaration_count,
            linear_points,
            linear_point_count,
            base_url_bytes,
            base_url_byte_count,
        },
    );
}

/// Answers [`style_engine_set_element_animation_effect_descriptions`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_set_element_animation_effect_descriptions`].
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn owner_set_element_animation_effect_descriptions(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    slot: u8,
    effects: *const FfiPublishedAnimationEffect,
    effect_count: usize,
    keyframes: *const FfiPublishedAnimationKeyframe,
    keyframe_count: usize,
    declarations: *const FfiPublishedAnimationDeclaration,
    declaration_count: usize,
    custom_declarations: *const FfiPublishedAnimationCustomDeclaration,
    custom_declaration_count: usize,
    linear_points: *const FfiPublishedLinearEasingPoint,
    linear_point_count: usize,
    base_url_bytes: *const u8,
    base_url_byte_count: usize,
) {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return;
    };
    let buffers = unsafe {
        published_effect_buffers(
            effects,
            effect_count,
            keyframes,
            keyframe_count,
            declarations,
            declaration_count,
            custom_declarations,
            custom_declaration_count,
            linear_points,
            linear_point_count,
            base_url_bytes,
            base_url_byte_count,
        )
    };
    unsafe { engine.set_element_animation_effect_descriptions(node, slot, buffers) };
}

/// Gather the flat buffers a list of effect descriptions travels in. An element's effects and the
/// `@keyframes` a style scope defines are described alike and travel the same way.
///
/// # Safety
/// Each pointer must be null with a zero count, or name that many elements, and every declaration's
/// value must be a live style value for the duration of the call.
#[expect(clippy::too_many_arguments)]
unsafe fn published_effect_buffers<'a>(
    effects: *const FfiPublishedAnimationEffect,
    effect_count: usize,
    keyframes: *const FfiPublishedAnimationKeyframe,
    keyframe_count: usize,
    declarations: *const FfiPublishedAnimationDeclaration,
    declaration_count: usize,
    custom_declarations: *const FfiPublishedAnimationCustomDeclaration,
    custom_declaration_count: usize,
    linear_points: *const FfiPublishedLinearEasingPoint,
    linear_point_count: usize,
    base_url_bytes: *const u8,
    base_url_byte_count: usize,
) -> crate::css::style::animations::PublishedEffectBuffers<'a> {
    macro_rules! slice {
        ($pointer:expr, $count:expr) => {
            match $count {
                0 => &[][..],
                count => unsafe { std::slice::from_raw_parts($pointer, count) },
            }
        };
    }
    crate::css::style::animations::PublishedEffectBuffers {
        effects: slice!(effects, effect_count),
        keyframes: slice!(keyframes, keyframe_count),
        declarations: slice!(declarations, declaration_count),
        custom_declarations: slice!(custom_declarations, custom_declaration_count),
        linear_points: slice!(linear_points, linear_point_count),
        base_url_bytes: slice!(base_url_bytes, base_url_byte_count),
    }
}

/// The size of an element's transform reference box, as the last committed layout left it.
#[repr(C)]
pub struct FfiCommittedTransformReferenceBox {
    pub has_box: bool,
    pub width: f64,
    pub height: f64,
}

/// One transition an element holds for a property, whether it still runs or completed: what the
/// transition step reads of it. Published whenever the element's set of transitions moves; whether
/// it still runs is read from its timing row when the step is decided.
#[repr(C)]
pub struct FfiPublishedTransition {
    pub property_id: u16,
    pub effect_identity: u64,
    /// Script replaced the effect the transition started with.
    pub effect_replaced: bool,
    /// The identity of the keyframe effect the transition's animation runs now, which its timing
    /// row is published under; zero where it runs none.
    pub current_effect_identity: u64,
    pub end_value: *const std::ffi::c_void,
    pub reversing_adjusted_start_value: *const std::ffi::c_void,
    pub reversing_shortening_factor: f64,
    pub start_time: f64,
    pub end_time: f64,
}

/// The transitions one of an element's lists holds, replacing what it held: published whenever the
/// set moves, for the pass to decide the element's transition step over.
///
/// # Safety
/// `engine` must be a live style engine, and `transitions` must point to `count` rows whose values
/// are live style values for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_set_element_transitions(
    engine: StyleEngineInputHandle,
    node: u32,
    slot: u8,
    transitions: *const FfiPublishedTransition,
    count: usize,
) {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_set_element_transitions",
        crate::css::style::owner_calls::StyleQuery::SetElementTransitions {
            node,
            slot,
            transitions,
            count,
        },
    );
}

/// Answers [`style_engine_set_element_transitions`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_set_element_transitions`].
pub(crate) unsafe fn owner_set_element_transitions(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    slot: u8,
    transitions: *const FfiPublishedTransition,
    count: usize,
) {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return;
    };
    let transitions = match count {
        0 => &[][..],
        _ => unsafe { std::slice::from_raw_parts(transitions, count) },
    };
    unsafe { engine.set_element_transitions(node, slot, transitions) };
}

/// One property's decision in a transition step the pass decided: the action, as
/// `FfiTransitionActionKind`, and for a transition it starts, the values it runs from and to.
#[repr(C)]
pub struct FfiTransitionStepAction {
    pub property_id: u16,
    pub kind: u8,
    pub delay: f64,
    pub active_duration: f64,
    pub reversing_shortening_factor: f64,
    pub start_value: *const c_void,
    pub end_value: *const c_void,
}

/// The transition step the pass decided for a row, borrowed until the next take.
#[repr(C)]
pub struct FfiTransitionStepDecidedInPass {
    pub present: bool,
    pub actions: *const FfiTransitionStepAction,
    pub action_count: usize,
}

impl FfiTransitionStepDecidedInPass {
    /// What a row the pass decided no step for is answered with.
    pub(crate) fn absent() -> Self {
        Self {
            present: false,
            actions: std::ptr::null(),
            action_count: 0,
        }
    }
}

/// Take the transition step the pass decided for an element's row, which the host applies instead
/// of deciding it: the pass already composed the transitions it starts into the row's composition.
///
/// # Safety
/// `engine` must be a live style engine.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_take_transition_step_decided_in_pass(
    engine: StyleEngineInputHandle,
    node: u32,
) -> FfiTransitionStepDecidedInPass {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_take_transition_step_decided_in_pass",
        crate::css::style::owner_calls::StyleQuery::TakeTransitionStepDecidedInPass { node },
    )
    .transition_step()
}

/// Answers [`style_engine_take_transition_step_decided_in_pass`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_take_transition_step_decided_in_pass`].
pub(crate) unsafe fn owner_take_transition_step_decided_in_pass(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
) -> FfiTransitionStepDecidedInPass {
    match StyleNodeID::from_raw(node).and_then(|node| engine.take_transition_step_decided_in_pass(node)) {
        Some(step) => FfiTransitionStepDecidedInPass {
            present: true,
            actions: step.actions().as_ptr(),
            action_count: step.actions().len(),
        },
        None => FfiTransitionStepDecidedInPass::absent(),
    }
}

/// Take the transition step the engine decided for a synthetic pseudo-element it settled, which
/// the host applies instead of deciding it: the engine already composed what it starts into the
/// record the pseudo-element installs.
///
/// # Safety
/// `engine` must be a live style engine.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_take_pseudo_element_transition_step_decided_in_pass(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
) -> FfiTransitionStepDecidedInPass {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_take_pseudo_element_transition_step_decided_in_pass",
        crate::css::style::owner_calls::StyleQuery::TakePseudoElementTransitionStepDecidedInPass { node, pseudo_kind },
    )
    .transition_step()
}

/// Answers [`style_engine_take_pseudo_element_transition_step_decided_in_pass`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_take_pseudo_element_transition_step_decided_in_pass`].
pub(crate) unsafe fn owner_take_pseudo_element_transition_step_decided_in_pass(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo_kind: u8,
) -> FfiTransitionStepDecidedInPass {
    match StyleNodeID::from_raw(node)
        .and_then(|node| engine.take_pseudo_element_transition_step_decided_in_pass(node, pseudo_kind))
    {
        Some(step) => FfiTransitionStepDecidedInPass {
            present: true,
            actions: step.actions().as_ptr(),
            action_count: step.actions().len(),
        },
        None => FfiTransitionStepDecidedInPass::absent(),
    }
}

/// The transform reference box the last committed layout left for `node`, which the animation
/// stage resolves percentage translations against. An element with no committed box, and every
/// element while the document has no layout arena, has none.
///
/// # Safety
/// `arena` must be the document's live layout arena, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_committed_transform_reference_box(
    arena: *mut c_void,
    node: u32,
) -> FfiCommittedTransformReferenceBox {
    super::seal::note_engine_call("layout_arena_committed_transform_reference_box");
    let none = FfiCommittedTransformReferenceBox {
        has_box: false,
        width: 0.0,
        height: 0.0,
    };
    let Some(node) = StyleNodeID::from_raw(node) else {
        return none;
    };
    // SAFETY: The caller passes a live arena or null, and this reads its committed paintable rows
    // without touching the engine it can reach back into.
    match unsafe { super::animations::committed_transform_reference_box(arena, node) } {
        Some((width, height)) => FfiCommittedTransformReferenceBox {
            has_box: true,
            width,
            height,
        },
        None => none,
    }
}

/// # Safety
/// `engine` must be a pointer returned by `style_engine_create` and not yet destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_destroy(engine: StyleEngineHandle) {
    // SAFETY: Guaranteed by the caller.
    let mut engine = unsafe { engine.destroy("style_engine_destroy") };
    super::seal::note_engine_call("style_engine_destroy");
    super::seal::flush_engine_decline_census(engine.counters().iter());
    super::seal::flush_census();
    engine.end_recording();
}

/// Returns the live element descendants whose inheritance path begins at `root` in the flat tree.
///
/// # Safety
/// `engine` must be live. The returned node slice remains valid until the next mutable
/// `style_engine_*` entry point or an explicit discard of the flat-tree descendants.
pub unsafe fn replay_flat_tree_descendants(engine: StyleEngineHandle, root: u32) -> FfiStyleNodeSlice {
    let engine = unsafe { engine.for_replay() };
    engine.clear_ffi_style_node_query();
    let Some(root) = StyleNodeID::from_raw(root) else {
        return FfiStyleNodeSlice::default();
    };
    let mut descendants = Vec::new();
    engine.for_each_flat_tree_descendant(root, |node| {
        descendants.push(node.raw());
    });
    engine.record_boundary_call(EventKind::ForEachFlatTreeDescendant, |payload| {
        payload.write_u32(root.raw());
        payload.write_u32_slice(&descendants);
    });
    engine.install_ffi_style_node_query(descendants)
}

/// Discards the borrowed flat-tree descendant slice.
///
/// # Safety
/// `engine` must be live.
pub unsafe fn replay_discard_flat_tree_descendants(engine: StyleEngineHandle) {
    let engine = unsafe { engine.for_replay() };
    engine.clear_ffi_style_node_query();
}

/// Grants the host identities to mint on its own, beside the ones a transaction's answer carries:
/// fills `elements` and `texts` with element and text identities whose slots the engine has
/// readied. A grant makes no identity live, and no node the engine knows names one, so it changes no
/// answer of a pass and needs no transaction.
///
/// # Safety
/// `engine` must be live, and `elements` and `texts` must point at their stated number of writable
/// slots for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_grant_style_nodes(
    engine: StyleEngineInputHandle,
    elements: *mut u32,
    element_count: usize,
    texts: *mut u32,
    text_count: usize,
) {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_grant_style_nodes",
        crate::css::style::owner_calls::StyleQuery::GrantStyleNodes {
            elements,
            element_count,
            texts,
            text_count,
        },
    );
}

/// Answers [`style_engine_grant_style_nodes`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_grant_style_nodes`].
pub(crate) unsafe fn owner_grant_style_nodes(
    engine: &mut crate::css::style::StyleEngine,
    elements: *mut u32,
    element_count: usize,
    texts: *mut u32,
    text_count: usize,
) {
    // SAFETY: the caller vouches that each pointer covers its stated count for this call.
    let elements = unsafe { borrow_mut(elements, element_count) };
    let texts = unsafe { borrow_mut(texts, text_count) };
    grant_style_nodes(engine, elements, texts);
}

/// Applies one flat style input transaction.
///
/// # Safety
/// `engine` must be live and every array in `transaction` must point at its stated number of valid
/// records for the duration of the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_apply_transaction(
    engine: StyleEngineInputHandle,
    transaction: &FfiStyleInputTransaction,
) {
    let handle = engine.home();
    // The input goes to the owner as a change, which it applies before the next unit or query that reaches the
    // engine. The grant answers the host now: the owner grants the identities after the input, which names none of
    // them.
    let grant = StyleNodeGrant::of(transaction);
    // SAFETY: Guaranteed by the caller.
    let input = unsafe { InputForPass::take_from(transaction) };
    handle.bring_home("style_engine_apply_transaction");
    super::seal::note_engine_call("style_engine_apply_transaction");
    crate::render_owner::send_change(
        engine.through_render_inputs(),
        handle.document(),
        crate::render_owner::Change::StyleInputs(input),
    );
    if !grant.is_empty() {
        crate::css::style::owner_calls::ask(
            handle,
            "style_engine_apply_transaction",
            crate::css::style::owner_calls::StyleQuery::GrantStyleNodes {
                elements: grant.elements,
                element_count: grant.element_count,
                texts: grant.texts,
                text_count: grant.text_count,
            },
        );
    }
}

/// The rows of a style input transaction.
struct InputRows<'a> {
    tree: &'a [FfiTreeDelta],
    arrivals: &'a [FfiElementArrival],
    arrival_custom_state_atoms: &'a [u32],
    features: &'a [FfiLocalFeatureDelta],
    states: &'a [FfiStateDelta],
    declarations: &'a [FfiElementDeclarationDelta],
    element_style_inputs: &'a [FfiElementStyleInput],
    host_fact_writes: &'a [FfiHostFactWrite],
}

impl<'a> InputRows<'a> {
    /// # Safety
    /// Every array in `transaction` must point at its stated number of valid records for `'a`.
    unsafe fn borrow_from(transaction: &'a FfiStyleInputTransaction) -> Self {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            Self {
                tree: borrow(transaction.tree_deltas, transaction.tree_delta_count),
                arrivals: borrow(transaction.element_arrivals, transaction.element_arrival_count),
                arrival_custom_state_atoms: borrow(
                    transaction.arrival_custom_state_atoms,
                    transaction.arrival_custom_state_atom_count,
                ),
                features: borrow(transaction.local_feature_deltas, transaction.local_feature_delta_count),
                states: borrow(transaction.state_deltas, transaction.state_delta_count),
                declarations: borrow(
                    transaction.element_declaration_deltas,
                    transaction.element_declaration_delta_count,
                ),
                element_style_inputs: borrow(transaction.element_style_inputs, transaction.element_style_input_count),
                host_fact_writes: borrow(transaction.host_fact_writes, transaction.host_fact_write_count),
            }
        }
    }
}

/// Applies a transaction's batch, once its host fact writes are in.
fn apply_input_batch(engine: &mut StyleEngine, rows: &InputRows<'_>) {
    let InputRows {
        tree,
        arrivals,
        arrival_custom_state_atoms,
        features,
        states,
        declarations,
        element_style_inputs,
        host_fact_writes: _,
    } = *rows;
    if tree.is_empty()
        && arrivals.is_empty()
        && features.is_empty()
        && states.is_empty()
        && declarations.is_empty()
        && element_style_inputs.is_empty()
    {
        return;
    }
    engine.apply_transaction_batch(
        tree,
        (arrivals, arrival_custom_state_atoms),
        features,
        states,
        declarations,
        element_style_inputs,
    );
    engine.record_boundary_call(EventKind::ApplyTransaction, |payload| {
        write_recording_tree_deltas(tree, payload);
        payload.write_raw_slice(arrivals);
        payload.write_u32_slice(arrival_custom_state_atoms);
        payload.write_raw_slice(features);
        write_recording_state_deltas(states, payload);
        payload.write_raw_slice(declarations);
        write_recording_element_style_inputs(element_style_inputs, payload);
    });
}

/// A style input transaction the host hands over with the style pass it submits, which applies it as
/// its first step, beside the document thread. It owns what its host fact writes hand over (see
/// [`apply_host_fact_writes`]) until then.
pub(crate) struct InputForPass {
    tree: Vec<FfiTreeDelta>,
    arrivals: Vec<FfiElementArrival>,
    arrival_custom_state_atoms: Vec<u32>,
    features: Vec<FfiLocalFeatureDelta>,
    states: Vec<FfiStateDelta>,
    declarations: Vec<FfiElementDeclarationDelta>,
    element_style_inputs: Vec<FfiElementStyleInput>,
    host_fact_writes: Vec<FfiHostFactWrite>,
    /// What the `ElementReplacedContentInput` writes point at, which the host lends only for the
    /// call that hands the transaction over. Each such write names its input here by index.
    replaced_content_inputs: Vec<FfiReplacedContentInput>,
    /// The reactions the host applied as it installed the batches before, which the transaction
    /// derives the children's reactions from.
    applied_style_reactions: Vec<FfiAppliedStyleReaction>,
    /// The elements whose pseudo-elements the host left for the transaction's pass to settle.
    pseudo_element_settles: Vec<FfiPseudoElementSettle>,
}

impl InputForPass {
    /// A transaction that writes nothing.
    #[cfg(test)]
    pub(crate) fn empty() -> Self {
        Self {
            tree: Vec::new(),
            arrivals: Vec::new(),
            arrival_custom_state_atoms: Vec::new(),
            features: Vec::new(),
            states: Vec::new(),
            declarations: Vec::new(),
            element_style_inputs: Vec::new(),
            host_fact_writes: Vec::new(),
            replaced_content_inputs: Vec::new(),
            applied_style_reactions: Vec::new(),
            pseudo_element_settles: Vec::new(),
        }
    }

    /// What the host hands over with a transaction it takes or submits: the input it recorded, if
    /// it recorded any, and what its install of the batches before handed back.
    ///
    /// # Safety
    /// `input` is null or as for [`Self::take_from`].
    unsafe fn handed_over(
        input: *const FfiStyleInputTransaction,
        install_feedback: InstallFeedback<'_>,
    ) -> Option<Self> {
        // SAFETY: Guaranteed by the caller.
        let mut handed_over = match unsafe { input.as_ref() } {
            // SAFETY: Guaranteed by the caller.
            Some(input) => unsafe { Self::take_from(input) },
            None if install_feedback.is_empty() => return None,
            None => Self {
                tree: Vec::new(),
                arrivals: Vec::new(),
                arrival_custom_state_atoms: Vec::new(),
                features: Vec::new(),
                states: Vec::new(),
                declarations: Vec::new(),
                element_style_inputs: Vec::new(),
                host_fact_writes: Vec::new(),
                replaced_content_inputs: Vec::new(),
                applied_style_reactions: Vec::new(),
                pseudo_element_settles: Vec::new(),
            },
        };
        handed_over.applied_style_reactions = install_feedback.applied_style_reactions.to_vec();
        handed_over.pseudo_element_settles = install_feedback.pseudo_element_settles.to_vec();
        Some(handed_over)
    }

    /// # Safety
    /// As for [`style_engine_apply_transaction`]'s `transaction`. What its host fact writes hand over
    /// is this transaction's from now on.
    unsafe fn take_from(transaction: &FfiStyleInputTransaction) -> Self {
        // SAFETY: Guaranteed by the caller.
        let rows = unsafe { InputRows::borrow_from(transaction) };
        let mut replaced_content_inputs = Vec::new();
        let host_fact_writes = rows
            .host_fact_writes
            .iter()
            .map(|write| {
                let mut write = *write;
                if write.kind == FfiHostFactKind::ElementReplacedContentInput {
                    // SAFETY: The caller vouches that the write points at an input that outlives the call.
                    replaced_content_inputs.push(unsafe { *(write.data as *const FfiReplacedContentInput) });
                    write.data = replaced_content_inputs.len() - 1;
                }
                write
            })
            .collect();
        Self {
            tree: rows.tree.to_vec(),
            arrivals: rows.arrivals.to_vec(),
            arrival_custom_state_atoms: rows.arrival_custom_state_atoms.to_vec(),
            features: rows.features.to_vec(),
            states: rows.states.to_vec(),
            declarations: rows.declarations.to_vec(),
            element_style_inputs: rows.element_style_inputs.to_vec(),
            host_fact_writes,
            replaced_content_inputs,
            applied_style_reactions: Vec::new(),
            pseudo_element_settles: Vec::new(),
        }
    }

    /// Applies the transaction as [`style_engine_apply_transaction`] does, but for the grant, which
    /// answered the host as it handed the transaction over.
    pub(crate) fn apply(mut self, engine: &mut StyleEngine) {
        // The host left the settles before it recorded what the rest of the transaction applies,
        // whose removals end the settles of the nodes they retire.
        settle_pseudo_elements_in_next_pass(engine, &self.pseudo_element_settles);
        let mut host_fact_writes = std::mem::take(&mut self.host_fact_writes);
        for write in &mut host_fact_writes {
            if write.kind == FfiHostFactKind::ElementReplacedContentInput {
                write.data = std::ptr::from_ref(&self.replaced_content_inputs[write.data]) as usize;
            }
        }
        // SAFETY: The writes hand over what this transaction owns, and each replaced content input
        // write points at an input it holds.
        unsafe { apply_host_fact_writes(engine, &host_fact_writes) };
        apply_input_batch(
            engine,
            &InputRows {
                tree: &self.tree,
                arrivals: &self.arrivals,
                arrival_custom_state_atoms: &self.arrival_custom_state_atoms,
                features: &self.features,
                states: &self.states,
                declarations: &self.declarations,
                element_style_inputs: &self.element_style_inputs,
                host_fact_writes: &[],
            },
        );
        record_applied_style_reactions(engine, &self.applied_style_reactions);
    }
}

/// Whether one of the style reactions the host applied since the last transaction and holds for the
/// next, `applied` (`count` of them), derives a style input for `node` once that transaction takes
/// it. A read of `node` before then owes `node` that input, which the engine does not hold yet.
///
/// # Safety
/// `engine` must be live for this call, and `applied` must point at `count` reactions.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_applied_style_reactions_derive_input(
    engine: StyleEngineHandle,
    node: u32,
    applied: *const FfiAppliedStyleReaction,
    count: usize,
) -> bool {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_applied_style_reactions_derive_input",
        crate::css::style::owner_calls::StyleQuery::AppliedStyleReactionsDeriveInput { node, applied, count },
    )
    .is()
}

/// Answers [`style_engine_applied_style_reactions_derive_input`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_applied_style_reactions_derive_input`].
pub(crate) unsafe fn owner_applied_style_reactions_derive_input(
    engine: &crate::css::style::StyleEngine,
    node: u32,
    applied: *const FfiAppliedStyleReaction,
    count: usize,
) -> bool {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return false;
    };
    // SAFETY: Guaranteed by the caller.
    let applied = unsafe { borrow(applied, count) };
    engine.applied_style_reactions_derive_input(
        node,
        applied
            .iter()
            .filter_map(super::child_reactions::AppliedStyleReaction::from_host),
    )
}

/// Keeps the elements whose pseudo-elements the host left for the next transaction's pass to settle.
fn settle_pseudo_elements_in_next_pass(engine: &mut StyleEngine, settles: &[FfiPseudoElementSettle]) {
    for settle in settles {
        operations::settle_pseudo_elements_in_next_pass(
            engine,
            settle.node,
            settle.old_is_list_item,
            &settle.held_pseudo_records,
        );
    }
}

/// Keeps the reactions the host applied, for the next transaction's pass to derive the children's
/// reactions from.
fn record_applied_style_reactions(engine: &mut StyleEngine, reactions: &[FfiAppliedStyleReaction]) {
    for reaction in reactions {
        operations::record_applied_style_reaction(
            engine,
            reaction.node,
            reaction.reaction,
            reaction.inherited_style_groups_changed,
            reaction.facts,
        );
    }
}

impl Drop for InputForPass {
    fn drop(&mut self) {
        // A transaction no pass applied gives up what its writes hand over, as the host does for
        // writes that never crossed.
        for write in &self.host_fact_writes {
            match write.kind {
                // SAFETY: The write transfers one reference to a live string.
                FfiHostFactKind::TextData | FfiHostFactKind::ElementLanguage => {
                    drop(unsafe { ak::Utf16String::from_raw_owned(write.data) });
                }
                FfiHostFactKind::ElementInlineStyleProperties | FfiHostFactKind::ElementPresentationalHints
                    if write.data != 0 =>
                {
                    // SAFETY: The write transfers one reference to the snapshot.
                    drop(unsafe {
                        std::sync::Arc::from_raw(
                            write.data as *const crate::css::declaration_block::DeclarationBlockData,
                        )
                    });
                }
                FfiHostFactKind::AdoptAtom => {
                    super::atoms::release_raw_without_adoption(write.data, StyleAtomID(write.facts));
                }
                FfiHostFactKind::AdoptQualifiedAtom => super::atoms::release_qualified_without_adoption(
                    StyleAtomID(write.node),
                    StyleAtomID(write.parent),
                    StyleAtomID(write.facts),
                ),
                _ => {}
            }
        }
    }
}

/// The writes of the node list that starts `run`: its first write and the ones after it that go on
/// with it, for the same slot where `per_slot`.
fn node_list_run(run: &[FfiHostFactWrite], per_slot: bool) -> &[FfiHostFactWrite] {
    debug_assert_eq!(run[0].value, 1, "a node list starts with a write that says so");
    let length = 1 + run[1..]
        .iter()
        .take_while(|member| member.value == 0 && (!per_slot || member.node == run[0].node))
        .count();
    &run[..length]
}

/// Apply the host's fact writes in the order it made them. Each is recorded as the boundary call
/// it stands for, so a replay applies it on its own.
///
/// # Safety
/// Every `TextData` write must carry a raw `AK::Utf16String`, every `ElementInlineStyleProperties`
/// write null or an Arc-owned `DeclarationBlockData`, whose reference it transfers, and every
/// `ElementReplacedContentInput` write a pointer to an `FfiReplacedContentInput` live for the call.
unsafe fn apply_host_fact_writes(engine: &mut StyleEngine, writes: &[FfiHostFactWrite]) {
    // Only what a style attribute holds when the writes cross is an input to style, and no other write reads what a
    // node's inline style write leaves. A script that edits one style attribute over and over before a style update (a
    // list that shows its footer again for every item it adds) would otherwise have the engine repair the node's
    // cascade winners for every edit, so a node's last write stands for all of them.
    let mut last_inline_style_write = super::HashMap::default();
    for (index, write) in writes.iter().enumerate() {
        if write.kind == FfiHostFactKind::ElementInlineStyleProperties {
            last_inline_style_write.insert(write.node, index);
        }
    }
    let mut index = 0;
    while index < writes.len() {
        let write = &writes[index];
        // Links and retirements arrive a subtree at a time, so a run of them crosses as one call.
        let run_length = writes[index..]
            .iter()
            .take_while(|next| next.kind == write.kind)
            .count();
        match write.kind {
            FfiHostFactKind::LinkInDomOrder => {
                let links: Vec<u32> = writes[index..index + run_length]
                    .iter()
                    .flat_map(|link| [link.node, link.parent, link.previous_sibling])
                    .collect();
                operations::link_style_nodes_in_dom_order(engine, &links);
                index += run_length;
                continue;
            }
            FfiHostFactKind::MintElement => {
                let nodes: Vec<u32> = writes[index..index + run_length].iter().map(|mint| mint.node).collect();
                mint_style_nodes(engine, &nodes);
                for mint in &writes[index..index + run_length] {
                    if mint.value != 0 {
                        operations::mark_relation_only_style_node(engine, mint.node);
                    }
                }
                index += run_length;
                continue;
            }
            FfiHostFactKind::MintText => {
                let nodes: Vec<u32> = writes[index..index + run_length].iter().map(|mint| mint.node).collect();
                mint_text_style_nodes(engine, &nodes);
                index += run_length;
                continue;
            }
            FfiHostFactKind::RetireText => {
                let nodes: Vec<u32> = writes[index..index + run_length]
                    .iter()
                    .map(|retirement| retirement.node)
                    .collect();
                operations::retire_text_style_nodes(engine, &nodes);
                index += run_length;
                continue;
            }
            FfiHostFactKind::SlotAssignedNode => {
                let list = node_list_run(&writes[index..index + run_length], true);
                let assigned: Vec<u32> = list.iter().map(|member| member.parent).collect();
                operations::set_slot_assigned_nodes(engine, write.node, &assigned);
                index += list.len();
                continue;
            }
            FfiHostFactKind::ElementParts => {
                let list = node_list_run(&writes[index..index + run_length], true);
                if let Some(node) = StyleNodeID::from_raw(write.node) {
                    let pairs = list
                        .iter()
                        .filter_map(|member| {
                            StyleNodeID::from_raw(member.parent).map(|host| (StyleAtomID(member.facts), host))
                        })
                        .collect::<Vec<_>>();
                    set_element_parts(engine, node, &pairs);
                }
                index += list.len();
                continue;
            }
            FfiHostFactKind::ElementCustomState => {
                let list = node_list_run(&writes[index..index + run_length], true);
                let states: Vec<u32> = list
                    .iter()
                    .filter(|member| member.facts != 0)
                    .map(|member| member.facts)
                    .collect();
                operations::set_element_custom_states(engine, write.node, &states);
                index += list.len();
                continue;
            }
            FfiHostFactKind::TopLayerElement => {
                let list = node_list_run(&writes[index..index + run_length], false);
                let elements: Vec<u32> = list.iter().map(|member| member.node).collect();
                operations::set_top_layer_elements(engine, &elements);
                index += list.len();
                continue;
            }
            FfiHostFactKind::UnlinkFromDomOrder => {
                operations::unlink_style_node_from_dom_order(engine, write.node, write.parent);
            }
            FfiHostFactKind::TextIsAsciiWhitespace => {
                operations::set_text_is_ascii_whitespace(engine, write.node, write.value != 0);
            }
            FfiHostFactKind::TextIsInUserAgentShadowTree => {
                operations::set_text_is_in_user_agent_shadow_tree(engine, write.node, write.value != 0);
            }
            FfiHostFactKind::TextIsPasswordInput => {
                operations::set_text_is_password_input(engine, write.node, write.value != 0);
            }
            FfiHostFactKind::TextData => {
                // SAFETY: The caller vouches that the write transfers one reference to a live string.
                let data = unsafe { ak::Utf16String::from_raw_owned(write.data) };
                set_text_data(engine, write.node, data);
            }
            FfiHostFactKind::RuleConditionsHold => {
                if let Some(rule) = engine.native_rule_id(write.data as u64) {
                    operations::set_rule_conditions_hold(engine, rule.0 + 1, write.value != 0);
                }
            }
            FfiHostFactKind::ElementLanguage => {
                // SAFETY: As for `TextData`.
                let text = unsafe { ak::Utf16String::from_raw_owned(write.data) };
                set_element_language(engine, write.node, write.facts, &text.to_utf16());
            }
            FfiHostFactKind::ElementIdName => {
                operations::set_element_id_name(engine, write.node, write.facts);
            }
            FfiHostFactKind::ElementDirectionality => {
                operations::set_element_directionality(engine, write.node, write.facts);
            }
            FfiHostFactKind::ElementHeadingLevel => {
                operations::set_element_heading_level(engine, write.node, write.value);
            }
            FfiHostFactKind::ElementPartExposure => {
                operations::set_element_part_exposure(engine, write.node, write.parent);
            }
            FfiHostFactKind::ElementAdjustmentFacts => {
                operations::set_element_adjustment_facts(engine, write.node, write.facts);
            }
            FfiHostFactKind::ElementUniqueNodeId => {
                operations::set_element_unique_node_id(engine, write.node, write.data as u64);
            }
            FfiHostFactKind::NodeDomPaintFacts => {
                operations::set_node_dom_paint_facts(engine, write.node, write.value);
            }
            FfiHostFactKind::ElementTableSpans => {
                operations::set_element_table_spans(
                    engine,
                    write.node,
                    write.facts & 0xffff,
                    write.facts >> 16,
                    write.parent,
                );
            }
            FfiHostFactKind::ShadowRoot => {
                operations::set_shadow_root(engine, write.node, write.parent);
            }
            FfiHostFactKind::TreeScopeRoot => {
                operations::set_tree_scope_root(engine, write.facts, write.node);
            }
            FfiHostFactKind::TreeScopeUsesDocumentSheets => {
                operations::set_tree_scope_uses_document_sheets(engine, write.facts);
            }
            FfiHostFactKind::SizeQueryContainer => {
                if let Some(node) = StyleNodeID::from_raw(write.node) {
                    engine.note_size_query_container(node);
                }
            }
            FfiHostFactKind::StyleDependsOnSizeContainerQuery => {
                if let Some(node) = StyleNodeID::from_raw(write.node) {
                    engine.note_style_depends_on_size_container_query(node);
                }
            }
            FfiHostFactKind::RecomputesOnEnvironmentMove => {
                if let Some(node) = StyleNodeID::from_raw(write.node) {
                    engine.note_element_recomputes_on_environment_move(node);
                }
            }
            FfiHostFactKind::SizeContainerNeedsEvaluationAfterLayout => {
                if let Some(node) = StyleNodeID::from_raw(write.node) {
                    engine.note_size_container_needs_evaluation_after_layout(node);
                }
            }
            FfiHostFactKind::ChildrenExplicitlyInherit => {
                if let Some(node) = StyleNodeID::from_raw(write.node) {
                    engine.note_children_explicitly_inherit(node);
                }
            }
            FfiHostFactKind::ViewportDependentStyleInputs => {
                for node in engine.computed_group_sets.viewport_dependent_nodes() {
                    if let Some(node) = StyleNodeID::from_raw(node) {
                        engine.record_derived_element_style_input(node, write.value, 0);
                    }
                }
            }
            FfiHostFactKind::ElementAssociatedPseudoKind => {
                operations::set_element_associated_pseudo_kind(engine, write.node, write.value);
            }
            FfiHostFactKind::ElementConstructionFacts => {
                operations::set_element_construction_facts(engine, write.node, write.facts, write.value);
            }
            FfiHostFactKind::ElementReplacedContentInput => {
                // SAFETY: The caller vouches that the write points at an input that outlives the call.
                let input = unsafe { &*(write.data as *const FfiReplacedContentInput) };
                operations::set_element_replaced_content_input(
                    engine,
                    write.node,
                    input.kind as u8,
                    input.present,
                    input.first,
                    input.second,
                    input.third,
                    input.fourth,
                );
            }
            FfiHostFactKind::AdoptAtom => engine.adopt_atom(write.data, StyleAtomID(write.facts)),
            FfiHostFactKind::AdoptQualifiedAtom => engine.adopt_qualified_atom(
                StyleAtomID(write.node),
                StyleAtomID(write.parent),
                StyleAtomID(write.facts),
            ),
            FfiHostFactKind::ElementPresentationalHints => {
                // SAFETY: The write transfers one reference to the snapshot.
                let data = unsafe {
                    std::sync::Arc::from_raw(write.data as *const crate::css::declaration_block::DeclarationBlockData)
                };
                if let Some(node) = StyleNodeID::from_raw(write.node) {
                    let kind = match write.value {
                        0 => FfiElementDeclarationKind::InlineStyle,
                        1 => FfiElementDeclarationKind::PresentationalHint,
                        _ => FfiElementDeclarationKind::SvgPresentationAttribute,
                    };
                    register_element_declared_properties(engine, node, kind, &data.properties, &[]);
                }
            }
            FfiHostFactKind::ElementInlineStyleProperties => {
                // SAFETY: The caller vouches that the write transfers one reference to the snapshot.
                let data = (write.data != 0).then(|| unsafe {
                    std::sync::Arc::from_raw(write.data as *const crate::css::declaration_block::DeclarationBlockData)
                });
                if last_inline_style_write.get(&write.node) == Some(&index)
                    && let Some(node) = StyleNodeID::from_raw(write.node)
                {
                    register_element_declared_properties(
                        engine,
                        node,
                        FfiElementDeclarationKind::InlineStyle,
                        data.as_ref().map_or(&[], |data| data.properties.as_slice()),
                        data.as_ref().map_or(&[], |data| data.custom_properties.as_slice()),
                    );
                }
            }
        }
        index += 1;
    }
}

fn grant_style_nodes(engine: &mut StyleEngine, elements: &mut [u32], texts: &mut [u32]) {
    if elements.is_empty() && texts.is_empty() {
        return;
    }
    engine.grant_style_nodes(elements);
    engine.grant_text_style_nodes(texts);
    engine.record_boundary_call(EventKind::GrantStyleNodes, |payload| {
        payload.write_u32_slice(elements);
        payload.write_u32_slice(texts);
    });
}

fn mint_style_nodes(engine: &mut StyleEngine, nodes: &[u32]) {
    let identities: Vec<StyleNodeID> = nodes.iter().copied().filter_map(StyleNodeID::from_raw).collect();
    engine.mint_style_nodes(&identities);
    engine.record_boundary_call(EventKind::MintStyleNodes, |payload| payload.write_u32_slice(nodes));
}

fn mint_text_style_nodes(engine: &mut StyleEngine, nodes: &[u32]) {
    let identities: Vec<StyleNodeID> = nodes.iter().copied().filter_map(StyleNodeID::from_raw).collect();
    engine.mint_text_style_nodes(&identities);
    engine.record_boundary_call(EventKind::MintTextStyleNodes, |payload| payload.write_u32_slice(nodes));
}

/// Grants what a recorded grant granted, answering whether the engine granted the same identities.
///
/// # Safety
/// `engine` must be live.
#[cfg(feature = "style-recording")]
pub unsafe fn replay_grant_style_nodes(engine: StyleEngineHandle, elements: &mut [u32], texts: &mut [u32]) {
    let engine = unsafe { engine.for_replay() };
    grant_style_nodes(engine, elements, texts);
}

/// Mints identities a recorded mint named.
///
/// # Safety
/// `engine` must be live, and every identity must have been granted and not yet minted.
#[cfg(feature = "style-recording")]
pub unsafe fn replay_mint_style_nodes(engine: StyleEngineHandle, nodes: &[u32], text: bool) {
    let engine = unsafe { engine.for_replay() };
    if text {
        mint_text_style_nodes(engine, nodes);
    } else {
        mint_style_nodes(engine, nodes);
    }
}

fn write_recording_tree_deltas(tree: &[FfiTreeDelta], payload: &mut super::record_replay::PayloadWriter) {
    payload.write_raw_rows(
        tree.len(),
        size_of::<FfiTreeDelta>(),
        align_of::<FfiTreeDelta>(),
        |payload| {
            for delta in tree {
                payload.write_native_u32(delta.node);
                payload.write_bool(delta.old_connected);
                payload.write_bool(delta.new_connected);
                payload.write_native_u16(0);
                write_recording_tree_relations(delta.old_relations, payload);
                write_recording_tree_relations(delta.new_relations, payload);
            }
        },
    );
}

fn write_recording_tree_relations(relations: FfiTreeRelations, payload: &mut super::record_replay::PayloadWriter) {
    payload.write_native_u32(relations.parent);
    payload.write_native_u32(relations.previous_element_sibling);
    payload.write_native_u32(relations.next_element_sibling);
    payload.write_native_u32(relations.tree_scope);
    payload.write_native_u32(relations.assigned_slot);
    payload.write_native_u32(relations.reserved);
}

fn write_recording_state_deltas(state_deltas: &[FfiStateDelta], payload: &mut super::record_replay::PayloadWriter) {
    payload.write_raw_rows(
        state_deltas.len(),
        size_of::<FfiStateDelta>(),
        align_of::<FfiStateDelta>(),
        |payload| {
            for delta in state_deltas {
                payload.write_native_u32(delta.node);
                payload.write_u8(delta.fact as u8);
                payload.write_bool(delta.new_value);
                payload.write_native_u16(0);
            }
        },
    );
}

fn write_recording_element_style_inputs(
    element_style_inputs: &[FfiElementStyleInput],
    payload: &mut super::record_replay::PayloadWriter,
) {
    payload.write_raw_rows(
        element_style_inputs.len(),
        size_of::<FfiElementStyleInput>(),
        align_of::<FfiElementStyleInput>(),
        |payload| {
            for input in element_style_inputs {
                payload.write_native_u32(input.style_node);
                payload.write_u8(input.reaction);
                payload.write_u8(input.inherited_style_groups);
                payload.write_native_u16(0);
            }
        },
    );
}

/// Publish already bound immutable selector inputs, after all host interning has finished.
pub(crate) fn publish_style_rule(
    engine: &mut StyleEngine,
    sheet: u32,
    before_rule: u32,
    compiled: &[&CompiledSelector],
    namespaces: NamespaceScope,
    bound_scope: &BoundScopeChain,
) -> u32 {
    if sheet == 0 || compiled.is_empty() {
        engine.record_boundary_call(EventKind::AddStyleRule, |payload| {
            payload.write_u32(sheet);
            payload.write_u32(before_rule);
            payload.write_u32(0);
        });
        return 0;
    }
    let scope: Vec<_> = bound_scope.roots.iter().map(|selector| selector.as_ref()).collect();
    let limits: Vec<_> = bound_scope.limits.iter().map(|selector| selector.as_ref()).collect();
    let before = match before_rule {
        0 => None,
        id => Some(RuleID(id - 1)),
    };
    let scope = ScopeChain {
        roots: &scope,
        limits: &limits,
        levels: &bound_scope.levels,
        implicit_roots: &bound_scope.implicit_roots,
    };
    let rule = engine.add_style_rule_in_scope(SheetID(sheet - 1), before, compiled, namespaces, &scope);
    let result = rule.0 + 1;
    engine.record_boundary_call(EventKind::AddStyleRule, |payload| {
        payload.write_u32(sheet);
        payload.write_u32(before_rule);
        payload.write_u32(result);
        write_recording_atom_mappings(engine, payload);
        super::selector::replay::write(engine.selector_program_for_rule(rule), payload);
    });
    result
}

/// Installs one already compiled semantic selector program.
///
/// # Safety
/// `engine` must be live, and the sheet and optional rule handles must belong to it.
#[cfg(feature = "style-recording")]
pub unsafe fn replay_add_style_rule(
    engine: StyleEngineHandle,
    sheet: u32,
    before_rule: u32,
    selector_program: super::selector::SelectorProgram,
) -> u32 {
    let engine = unsafe { engine.for_replay() };
    if sheet == 0 {
        return 0;
    }
    let before = (before_rule != 0).then(|| RuleID(before_rule - 1));
    engine
        .add_replayed_style_rule(SheetID(sheet - 1), before, selector_program)
        .0
        + 1
}

/// Replaces one selector list with an already compiled semantic program.
///
/// # Safety
/// `engine` must be live and `rule` must name one of its style rules.
#[cfg(feature = "style-recording")]
pub unsafe fn replay_replace_style_rule_selectors(
    engine: StyleEngineHandle,
    rule: u32,
    selector_program: super::selector::SelectorProgram,
) {
    let engine = unsafe { engine.for_replay() };
    engine.replace_replayed_style_rule_selectors(RuleID(rule - 1), selector_program);
}

/// Installs semantic declared-property identities without C++ style-value pointers.
///
/// # Safety
/// `engine` must be live and `node` must name one of its style nodes.
#[cfg(feature = "style-recording")]
pub unsafe fn replay_set_element_declared_properties(
    engine: StyleEngineHandle,
    node: u32,
    kind: FfiElementDeclarationKind,
    declared: &[DeclaredProperty],
    custom_declarations: &[CustomDeclaration],
) {
    let engine = unsafe { engine.for_replay() };
    let node = StyleNodeID::from_raw(node).expect("recorded style node identities are nonzero");
    engine.set_element_declared_properties(
        node,
        decode_element_declaration_kind(kind),
        declared,
        Vec::new(),
        custom_declarations.to_vec(),
        Vec::new(),
    );
}

/// Installs semantic declared-property identities without C++ style-value pointers.
///
/// # Safety
/// `engine` must be live and `rule` must name one of its style rules.
#[cfg(feature = "style-recording")]
pub unsafe fn replay_set_rule_declared_properties(
    engine: StyleEngineHandle,
    rule: u32,
    declared: &[DeclaredProperty],
    custom_declarations: &[CustomDeclaration],
) {
    let engine = unsafe { engine.for_replay() };
    engine.set_rule_declared_properties_with_written_values(
        RuleID(rule - 1),
        declared,
        Vec::new(),
        custom_declarations.to_vec(),
        Vec::new(),
    );
}

/// Replace selectors using immutable compilation inputs, preserving recording and rule identity.
pub(crate) fn publish_style_rule_selectors(
    engine: &mut StyleEngine,
    rule: u32,
    compiled: &[&CompiledSelector],
    namespaces: NamespaceScope,
    bound_scope: &BoundScopeChain,
) {
    if rule == 0 || compiled.is_empty() {
        return;
    }
    let scope: Vec<_> = bound_scope.roots.iter().map(|selector| selector.as_ref()).collect();
    let limits: Vec<_> = bound_scope.limits.iter().map(|selector| selector.as_ref()).collect();
    let scope = ScopeChain {
        roots: &scope,
        limits: &limits,
        levels: &bound_scope.levels,
        implicit_roots: &bound_scope.implicit_roots,
    };
    engine.replace_style_rule_selectors(RuleID(rule - 1), compiled, namespaces, &scope);
    engine.record_boundary_call(EventKind::ReplaceStyleRuleSelectors, |payload| {
        payload.write_u32(rule);
        write_recording_atom_mappings(engine, payload);
        super::selector::replay::write(engine.selector_program_for_rule(RuleID(rule - 1)), payload);
    });
}

/// Records the parts the element exposes, each by its name and the host it is exposed for.
fn set_element_parts(engine: &mut StyleEngine, node: StyleNodeID, pairs: &[(StyleAtomID, StyleNodeID)]) {
    engine.set_element_parts(node, pairs);
    engine.record_boundary_call(EventKind::SetElementParts, |payload| {
        payload.write_u32(node.raw());
        payload.write_length(pairs.len());
        for (name, host) in pairs {
            payload.write_u32(name.0);
            payload.write_u32(host.raw());
        }
    });
}

/// Records the characters a text node holds, as the document spells them.
fn set_text_data(engine: &mut StyleEngine, node: u32, data: ak::Utf16String) {
    engine.record_boundary_call(EventKind::SetTextData, |payload| {
        payload.write_u32(node);
        payload.write_u16_slice(&data.to_utf16());
    });
    let Some(node) = StyleNodeID::from_raw(node) else {
        return;
    };
    engine.set_text_data(node, data);
}

/// Records the element's resolved language, as its primary subtag atom.
fn set_element_language(engine: &mut StyleEngine, node: u32, language: u32, text: &[u16]) {
    let text = if language != 0 { text } else { &[] };
    if language != 0 && !text.is_empty() {
        // A range is not a name, so `:lang()` compares against the tag itself. It is recorded
        // once per language rather than once per element.
        engine.set_element_language_text(StyleAtomID(language), text);
    }
    if let Some(node) = StyleNodeID::from_raw(node) {
        engine.set_element_language(node, StyleAtomID(language));
    }
    engine.record_boundary_call(EventKind::SetElementLanguage, |payload| {
        payload.write_u32(node);
        payload.write_u32(language);
        payload.write_u16_slice(text);
    });
}

#[derive(Clone, Default)]
pub(crate) struct BoundScopeChain {
    roots: Vec<std::sync::Arc<CompiledSelector>>,
    limits: Vec<std::sync::Arc<CompiledSelector>>,
    levels: Vec<(u32, u32)>,
    implicit_roots: Vec<Option<ImplicitScopeRoot>>,
}

impl BoundScopeChain {
    pub(crate) fn push(
        &mut self,
        start: Option<&crate::css::selector_parser::RustParsedSelectorList>,
        end: Option<&crate::css::selector_parser::RustParsedSelectorList>,
        implicit_root: u32,
    ) {
        self.levels.push((
            u32::try_from(start.map_or(0, |list| list.selectors.len())).expect("scope root count exceeds u32"),
            u32::try_from(end.map_or(0, |list| list.selectors.len())).expect("scope limit count exceeds u32"),
        ));
        // An explicit start that transforms to an empty list matches no roots. It must not acquire
        // the DOM root that an omitted start uses.
        let implicit_root = if start.is_none() { implicit_root } else { 0 };
        self.implicit_roots.push(match implicit_root {
            0 => None,
            u32::MAX => Some(ImplicitScopeRoot::ContainingTree),
            node => StyleNodeID::from_raw(node).map(ImplicitScopeRoot::Node),
        });
        if let Some(start) = start {
            self.roots.extend(start.selectors.iter().cloned());
        }
        if let Some(end) = end {
            self.limits.extend(end.selectors.iter().cloned());
        }
    }
}

/// One concrete match, as the boundary carries it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct FfiRuleMatch {
    pub node: u32,
    /// The rule identity as the boundary numbers it: one more than the engine's, so that zero can
    /// mean no rule.
    pub rule: u32,
    /// Collision-checked identity of the rule's complete semantic declaration inventory, or zero
    /// when custom properties or another declaration kind remain on the C++ side.
    pub semantic_declaration: u32,
    /// The pseudo-element the match targets, or `u32::MAX` for the originating element itself.
    pub pseudo_element: u32,
    /// The host whose shadow tree this match decides in, or 0 for the document's own context. How
    /// deeply encapsulated a rule is orders it against the contexts around it, and the scope the
    /// match resolved through is not always the one the element is in: `:host`, `::slotted()` and
    /// `::part()` all decide for an element outside their own tree.
    pub scope_host: u32,
    /// Generational hops from the scoping root the match resolved through, or `u32::MAX` for a rule
    /// that is not scoped. A nearer root wins the proximity comparison.
    pub scope_proximity: u32,
}

unsafe fn write_rule_matches(
    engine: &mut StyleEngine,
    matches: &[RuleMatch],
    out: *mut FfiRuleMatch,
    capacity: usize,
) -> usize {
    if matches.len() > capacity {
        return matches.len();
    }
    for (index, entry) in matches.iter().enumerate() {
        unsafe {
            *out.add(index) = FfiRuleMatch {
                node: entry.node.raw(),
                rule: entry.rule.0 + 1,
                semantic_declaration: if engine.program.declarations_are_complete_for(entry.rule) {
                    engine.program.ensure_semantic_declaration(entry.rule).0
                } else {
                    0
                },
                pseudo_element: entry.pseudo_element.map_or(u32::MAX, |target| u32::from(target.kind.0)),
                scope_host: engine.cascade_context_host(entry.rule, entry.tree_scope),
                scope_proximity: entry.scope_proximity,
            };
        }
    }
    matches.len()
}

/// Consumes the complete answer published by the immediately preceding style transaction.
///
/// Returns `usize::MAX` when that transaction did not publish an answer for `node`, or a count
/// larger than `capacity` when nothing was written because the buffer was too small.
///
/// # Safety
/// `engine` must be live and `out` must point at `capacity` writable `FfiRuleMatch` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_consume_published_match_answer(
    engine: StyleEngineInputHandle,
    node: u32,
    out: *mut FfiRuleMatch,
    capacity: usize,
) -> usize {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_consume_published_match_answer",
        crate::css::style::owner_calls::StyleQuery::ConsumePublishedMatchAnswer { node, out, capacity },
    )
    .usize()
}

/// Answers [`style_engine_consume_published_match_answer`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_consume_published_match_answer`].
pub(crate) unsafe fn owner_consume_published_match_answer(
    engine: &mut StyleEngine,
    node: u32,
    out: *mut FfiRuleMatch,
    capacity: usize,
) -> usize {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return usize::MAX;
    };
    let result = engine
        .consume_published_match_answer_with(
            node,
            capacity,
            |index, node, rule, semantic_declaration, pseudo_element, scope_host, scope_proximity| unsafe {
                *out.add(index) = FfiRuleMatch {
                    node: node.raw(),
                    rule: rule.0 + 1,
                    semantic_declaration: semantic_declaration.0,
                    pseudo_element: pseudo_element.map_or(u32::MAX, |target| u32::from(target.kind.0)),
                    scope_host,
                    scope_proximity,
                };
            },
        )
        .unwrap_or(usize::MAX);
    engine.record_boundary_call(EventKind::ConsumePublishedMatchAnswer, |payload| {
        payload.write_u32(node.raw());
        payload.write_u64(u64::try_from(capacity).expect("match capacity exceeds u64"));
        payload.write_u64(u64::try_from(result).expect("match count exceeds u64"));
        let written = result != usize::MAX && result <= capacity;
        payload.write_bool(written);
        if written {
            let matches = unsafe { std::slice::from_raw_parts(out, result) };
            payload.write_length(matches.len());
            for entry in matches {
                payload.write_u32(entry.node);
                payload.write_u32(entry.rule);
                payload.write_u32(entry.pseudo_element);
                payload.write_u32(entry.scope_host);
                payload.write_u32(entry.scope_proximity);
            }
        }
    });
    result
}
/// Matches one element and writes its matches, in cascade order, into `out`.
///
/// The same answer the document pass gives for that element, asked one element at a time, which is
/// what a style recompute needs.
///
/// Returns the number written, `usize::MAX` when the pass could not complete, or a count larger
/// than `capacity` when nothing was written because the buffer was too small.
///
/// # Safety
/// `engine` must be live and `out` must point at `capacity` writable `FfiRuleMatch` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_match_element(
    engine: StyleEngineInputHandle,
    node: u32,
    out: *mut FfiRuleMatch,
    capacity: usize,
    compact_for_cascade: bool,
) -> usize {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_match_element",
        crate::css::style::owner_calls::StyleQuery::MatchElement {
            node,
            out,
            capacity,
            compact_for_cascade,
        },
    )
    .usize()
}

/// Answers [`style_engine_match_element`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_match_element`].
pub(crate) unsafe fn owner_match_element(
    engine: &mut StyleEngine,
    node: u32,
    out: *mut FfiRuleMatch,
    capacity: usize,
    compact_for_cascade: bool,
) -> usize {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return usize::MAX;
    };
    let result = match compact_for_cascade {
        true => engine.match_element_for_cascade(node),
        false => engine.match_element(node),
    };
    let Ok(matches) = result else {
        return usize::MAX;
    };
    let result = unsafe { write_rule_matches(engine, &matches, out, capacity) };
    engine.record_boundary_call(EventKind::MatchElement, |payload| {
        payload.write_u32(node.raw());
        payload.write_u64(u64::try_from(capacity).expect("match capacity exceeds u64"));
        payload.write_bool(compact_for_cascade);
        payload.write_u64(u64::try_from(result).expect("match count exceeds u64"));
        let written = result != usize::MAX && result <= capacity;
        payload.write_bool(written);
        if written {
            let matches = unsafe { std::slice::from_raw_parts(out, result) };
            payload.write_length(matches.len());
            for entry in matches {
                payload.write_u32(entry.node);
                payload.write_u32(entry.rule);
                payload.write_u32(entry.pseudo_element);
                payload.write_u32(entry.scope_host);
                payload.write_u32(entry.scope_proximity);
            }
        }
    });
    result
}

fn collect_native_custom_declarations(
    engine: &mut StyleEngine,
    custom_properties: &[crate::css::declaration_block::CustomProperty],
) -> (Vec<CustomDeclaration>, Vec<RetainedStyleValueData>) {
    let mut custom_written_values = Vec::new();
    let custom_declarations = custom_properties
        .iter()
        .map(|property| {
            let name = property.name.to_fly_string();
            let atom = intern_native_atom(engine, name.raw());
            unsafe { note_native_custom_property_name(engine, atom, name.raw(), property.name.units()) };
            let declaration = &property.declaration;
            // Custom properties retain their authored values, without normal-property
            // canonicalization. Their token spelling is observable after substitution.
            let value = unsafe { engine.intern_specified_value(std::sync::Arc::as_ptr(&declaration.value)) };
            custom_written_values.push(unsafe {
                RetainedStyleValueData::from_retained_pointer(std::sync::Arc::into_raw(
                    property.declaration.value.clone(),
                ))
            });
            CustomDeclaration {
                name: atom,
                important: declaration.important,
                operator: super::program_updates::declaration_operator(&declaration.value),
                value,
            }
        })
        .collect::<Vec<_>>();
    (custom_declarations, custom_written_values)
}

fn register_element_declared_properties(
    engine: &mut StyleEngine,
    node: StyleNodeID,
    kind: FfiElementDeclarationKind,
    declarations: &[crate::css::declaration_block::DeclaredProperty],
    custom_properties: &[crate::css::declaration_block::CustomProperty],
) -> bool {
    use crate::css::property_metadata::property_defines_a_css_transition;
    let has_transitions = declarations
        .iter()
        .any(|declaration| property_defines_a_css_transition(declaration.property_id));
    let (declared, written_values) = engine.intern_element_declared_properties(declarations);
    let (custom_declarations, custom_written_values) = collect_native_custom_declarations(engine, custom_properties);
    engine.set_element_declared_properties(
        node,
        decode_element_declaration_kind(kind),
        &declared,
        written_values,
        custom_declarations.clone(),
        custom_written_values,
    );
    engine.record_boundary_call(EventKind::SetElementDeclaredProperties, |payload| {
        payload.write_u32(node.raw());
        payload.write_u8(kind as u8);
        write_declared_properties(&declared, payload);
        write_custom_declarations(&custom_declarations, payload);
    });
    has_transitions
}

#[cfg(feature = "style-recording")]
pub unsafe fn replay_publish_exact_cascade_state(
    engine: StyleEngineHandle,
    node: u32,
    pseudo_kind: u8,
    winners: &[RecordedExactCascadeWinner],
    inherited_style_groups: u8,
    donor_node: u32,
    donor_style_record: u64,
) -> (FfiExactCascadePublication, bool) {
    let engine = unsafe { engine.for_replay() };
    let node = StyleNodeID::from_raw(node).expect("recorded style node identities are nonzero");
    let winners = winners
        .iter()
        .map(|winner| {
            (
                winner.property,
                super::cascade::SpecifiedWinnerKey {
                    value: super::cascade::SpecifiedValueID(winner.value),
                    operator: winner.operator,
                    continuation: super::cascade::CascadeContinuationID::default(),
                    important: winner.important,
                },
            )
        })
        .collect::<Vec<_>>();
    let (publication, had_previous) = engine.publish_exact_cascade_winners(
        super::computed::ComputedStyleTarget::new(node, pseudo_kind),
        &winners,
        inherited_style_groups,
        exact_cascade_donor(donor_node, donor_style_record),
    );
    (publication, had_previous)
}

#[cfg(any(test, feature = "style-recording"))]
fn exact_cascade_donor(donor_node: u32, donor_style_record: u64) -> Option<super::publication::ExactCascadeDonor> {
    let node = StyleNodeID::from_raw(donor_node)?;
    (donor_style_record != 0).then_some(super::publication::ExactCascadeDonor {
        node,
        style_record: donor_style_record,
    })
}

#[cfg(feature = "style-recording")]
pub unsafe fn replay_exact_cascade_generation_snapshot(
    engine: StyleEngineHandle,
    node: u32,
    pseudo_kind: u8,
) -> (u64, Option<u64>) {
    let engine: &StyleEngine = unsafe { engine.for_replay() };
    let node = StyleNodeID::from_raw(node).expect("recorded style node identities are nonzero");
    engine.exact_cascade_generation_snapshot(super::computed::ComputedStyleTarget::new(node, pseudo_kind))
}

#[cfg(feature = "style-replay")]
pub fn replay_style_value(token: u64, dependency_flags: u8) -> *const c_void {
    crate::css::style_value::register_replay_style_value(token, dependency_flags).cast()
}

#[cfg(feature = "style-replay")]
pub fn replay_style_value_token(value: *const c_void) -> Option<u64> {
    crate::css::style_value::replay_style_value_token(value.cast())
}

#[cfg(feature = "style-recording")]
pub unsafe fn replay_memory_pressure_snapshot(engine: StyleEngineHandle) -> FfiMemoryPressureSnapshot {
    let engine: &StyleEngine = unsafe { engine.for_replay() };
    let memory = engine.memory();
    let tier3_refusal_categories = TIER3_REFUSAL_CATEGORIES.map(|category| memory.refusals(category));
    let tier4_refusal_categories: [u64; 2] = [MemoryCategory::NormalizationJournal, MemoryCategory::BatchScratch]
        .into_iter()
        .map(|category| memory.refusals(category))
        .collect::<Vec<_>>()
        .try_into()
        .expect("fixed memory category count");
    FfiMemoryPressureSnapshot {
        tier3_limit: memory.tier3_limit(),
        tier4_limit: memory.tier4_limit(),
        tier3_bytes: memory.bytes_in_tier(super::memory::Tier::Acceleration),
        tier4_bytes: memory.bytes_in_tier(super::memory::Tier::Scratch),
        tier3_refusals: tier3_refusal_categories.iter().sum(),
        tier4_refusals: tier4_refusal_categories.iter().sum(),
        tier3_refusal_categories,
        tier4_refusal_categories,
        tier3_evictions: engine
            .counters()
            .get(super::instrumentation::Counter::Tier3BenefitEvictions),
        category_bytes: MEMORY_CATEGORIES.map(|category| memory.bytes_in_category(category)),
    }
}
/// Publishes the immutable inputs of one element's base style and returns its old and new
/// `StyleRecordID` assignments. An equal pair means the final semantic output did not change.
///
/// # Safety
/// `engine` must be live. Every non-empty array must have its reported number of readable entries.
/// Group payloads and style values must remain live for this call. `animated_overlay` must be
/// null or point at a live Rust animation overlay. `longhand_table` must be null for an anonymous
/// layout style or point to a live, frozen `ComputedLonghandTable`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_publish_computed_groups(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
    payloads: *const *const c_void,
    count: usize,
    inherited_group_count: usize,
    custom_property_environment: u64,
    inherited_group_swap_candidate: bool,
    counter_style_environment_identity: u64,
    animation_overlay_identity: u64,
    animated_overlay: *const c_void,
    animation_overlay_payloads: *const *const c_void,
    animation_overlay_payload_count: usize,
    longhand_table: *const c_void,
    custom_property_store: *const c_void,
) -> FfiStyleRecordDelta {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_publish_computed_groups",
        crate::css::style::owner_calls::StyleQuery::PublishComputedGroups {
            node,
            pseudo_kind,
            payloads,
            count,
            inherited_group_count,
            custom_property_environment,
            inherited_group_swap_candidate,
            counter_style_environment_identity,
            animation_overlay_identity,
            animated_overlay,
            animation_overlay_payloads,
            animation_overlay_payload_count,
            longhand_table,
            custom_property_store,
        },
    )
    .record_delta()
}

/// Answers [`style_engine_publish_computed_groups`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_publish_computed_groups`].
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn owner_publish_computed_groups(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo_kind: u8,
    payloads: *const *const c_void,
    count: usize,
    inherited_group_count: usize,
    custom_property_environment: u64,
    inherited_group_swap_candidate: bool,
    counter_style_environment_identity: u64,
    animation_overlay_identity: u64,
    animated_overlay: *const c_void,
    animation_overlay_payloads: *const *const c_void,
    animation_overlay_payload_count: usize,
    longhand_table: *const c_void,
    custom_property_store: *const c_void,
) -> FfiStyleRecordDelta {
    if count != 0 && payloads.is_null() {
        return FfiStyleRecordDelta::default();
    }
    let payloads = match count {
        0 => &[],
        _ => SharedPayload::from_pointer_slice(unsafe { std::slice::from_raw_parts(payloads, count) }),
    };
    if animation_overlay_payload_count != 0 && animation_overlay_payloads.is_null() {
        return FfiStyleRecordDelta::default();
    }
    let animation_overlay_payloads = match animation_overlay_payload_count {
        0 => &[],
        _ => SharedPayload::from_pointer_slice(unsafe {
            std::slice::from_raw_parts(animation_overlay_payloads, animation_overlay_payload_count)
        }),
    };
    let longhand_table = unsafe {
        longhand_table
            .cast::<crate::css::computed_longhand_table::ComputedLonghandTable>()
            .as_ref()
    };
    publish_computed_groups_from_inputs(
        engine,
        node,
        pseudo_kind,
        payloads,
        inherited_group_count,
        custom_property_environment,
        inherited_group_swap_candidate,
        counter_style_environment_identity,
        animation_overlay_identity,
        animated_overlay,
        animation_overlay_payloads,
        longhand_table,
        custom_property_store,
    )
}

// Shared by host publication and native layout-style derivation. Recording stays at the
// engine input boundary even when the producer and the style store both live in Rust.
#[allow(clippy::too_many_arguments)]
pub(crate) fn publish_computed_groups_from_inputs(
    engine: &mut StyleEngine,
    node: u32,
    pseudo_kind: u8,
    payloads: &[SharedPayload],
    inherited_group_count: usize,
    custom_property_environment: u64,
    inherited_group_swap_candidate: bool,
    counter_style_environment_identity: u64,
    animation_overlay_identity: u64,
    animated_overlay: *const c_void,
    animation_overlay_payloads: &[SharedPayload],
    longhand_table: Option<&crate::css::computed_longhand_table::ComputedLonghandTable>,
    custom_property_store: *const c_void,
) -> FfiStyleRecordDelta {
    let raw_cascaded_font_size = longhand_table.map_or(std::ptr::null(), |table| table.raw_cascaded_font_size());
    assert!(longhand_table.is_some() || (node == 0 && pseudo_kind == u8::MAX));
    let pseudo_element_styles = longhand_table.map_or(0, |table| table.pseudo_element_styles());
    let inherited_group_swap_eligible = longhand_table.is_some_and(|table| {
        inherited_group_swap_candidate && table.property_inheritance_is_standard() && !table.display_is_list_item()
    });
    let holds_image_values =
        crate::css::computed_values::style_group_payloads_hold_image_values(HostShared::as_pointer_slice(payloads))
            || crate::css::computed_values::style_group_payloads_hold_image_values(HostShared::as_pointer_slice(
                animation_overlay_payloads,
            ));
    let dependency_flags = longhand_table.map_or(0, |table| table.publication_dependency_flags())
        | (u8::from(inherited_group_swap_eligible) * super::computed::INHERITED_GROUP_SWAP_ELIGIBLE)
        | (u8::from(holds_image_values) * super::computed::HOLDS_IMAGE_VALUES);
    if animation_overlay_identity != 0 && animated_overlay.is_null() {
        return FfiStyleRecordDelta::default();
    }
    let metadata_input = super::computed::ComputedMetadataInput {
        pseudo_element_styles,
        dependency_flags,
        counter_style_environment_identity,
        animation_overlay_identity,
        animated_overlay: HostShared::new(animated_overlay).cast(),
        animation_overlay_payloads,
        longhand_table: HostShared::new(longhand_table.map_or(std::ptr::null(), std::ptr::from_ref)),
    };
    // A computation with no style node behind it interns its record without assigning it.
    let publication = if let Some(node) = StyleNodeID::from_raw(node) {
        let target = super::computed::ComputedStyleTarget::new(node, pseudo_kind);
        engine.forget_engine_computed_record(target);
        engine.publish_computed_groups(
            target,
            payloads,
            inherited_group_count,
            custom_property_environment,
            metadata_input,
        )
    } else {
        engine.suspend_computed_group_content_identities(true);
        let interned = engine.intern_computed_groups(
            if animation_overlay_identity != 0 {
                animation_overlay_payloads
            } else {
                payloads
            },
            inherited_group_count,
            custom_property_environment,
            metadata_input,
        );
        engine.suspend_computed_group_content_identities(false);
        interned
    };
    // The store is what the environment resolves to; an engine-computed environment builds on it,
    // and an element alike in its custom declarations takes the environment itself.
    unsafe {
        engine
            .custom_property_environments
            .retain(custom_property_environment, custom_property_store);
    }
    if pseudo_kind == u8::MAX
        && let Some(node) = StyleNodeID::from_raw(node)
    {
        engine.remember_cpp_custom_property_environment(node, custom_property_environment);
    }
    let result = FfiStyleRecordDelta {
        old_style_record: publication
            .previous_style_record_identity
            .map_or(0, super::computed::FinalStyleRecordID::raw),
        new_style_record: publication.style_record_identity.raw(),
    };
    engine.record_boundary_call(EventKind::PublishComputedGroups, |payload| {
        let pointer_token = |pointer: *const c_void| match pointer.is_null() {
            true => 0,
            false => engine
                .recording_pointer_token(pointer as usize)
                .expect("an enabled recorder must tokenize the pointer"),
        };
        payload.write_u32(node);
        payload.write_u8(pseudo_kind);
        let groups = engine
            .recording_computed_group_identities(result.new_style_record)
            .expect("a published style record must retain its computed groups");
        let retained_bytes = engine
            .recording_computed_group_retained_bytes(result.new_style_record)
            .expect("a published style record must retain its computed group sizes");
        assert_eq!(groups.len(), retained_bytes.len());
        payload.write_length(groups.len());
        for (identity, retained_bytes) in groups.into_iter().zip(retained_bytes) {
            payload.write_u32(identity);
            payload.write_u64(retained_bytes);
        }
        payload.write_length(inherited_group_count);
        payload.write_u64(custom_property_environment);
        payload.write_u64(pseudo_element_styles);
        payload.write_u8(dependency_flags);
        payload.write_u64(counter_style_environment_identity);
        payload.write_u64(animation_overlay_identity);
        payload.write_u64(u64::from(!animated_overlay.is_null()));
        payload.write_length(animation_overlay_payloads.len());
        for &pointer in animation_overlay_payloads {
            payload.write_u64(pointer_token(pointer.as_ptr()));
        }
        payload.write_bytes(longhand_table.map_or(&[], |table| table.importance_bits()));
        payload.write_bytes(longhand_table.map_or(&[], |table| table.inheritance_bits()));
        payload.write_length(longhand_table.map_or(0, |table| table.inheritance_dependent_values().count()));
        for (property, value) in longhand_table
            .into_iter()
            .flat_map(crate::css::computed_longhand_table::ComputedLonghandTable::inheritance_dependent_values)
        {
            payload.write_u16(property);
            payload.write_u64(pointer_token(value));
            payload.write_u8(crate::css::style_value::style_value_dependency_flags(value.cast()));
        }
        payload.write_u64(pointer_token(raw_cascaded_font_size));
        payload.write_u8(match raw_cascaded_font_size.is_null() {
            true => 0,
            false => crate::css::style_value::style_value_dependency_flags(raw_cascaded_font_size.cast()),
        });
        payload.write_bool(longhand_table.is_some());
        if longhand_table.is_some() {
            let (identity, canonical_values) = engine
                .recording_computed_longhand_table(result.new_style_record)
                .expect("a published style record must retain its longhand table");
            payload.write_u32(identity);
            let record_definition = engine.recording_first_response(2, u64::from(identity));
            payload.write_bool(record_definition);
            if record_definition {
                let stored_values = canonical_values
                    .iter()
                    .enumerate()
                    .filter(|(_, value)| !value.is_null())
                    .collect::<Vec<_>>();
                payload.write_length(stored_values.len());
                for (index, &value) in stored_values {
                    payload.write_u16(crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID + index as u16);
                    payload.write_u64(pointer_token(value.as_ptr()));
                    payload.write_u8(crate::css::style_value::style_value_dependency_flags(
                        value.cast().as_ptr(),
                    ));
                }
            }
        }
        payload.write_u64(result.old_style_record);
        payload.write_u64(result.new_style_record);
    });
    result
}

/// Return the record the engine holds assigned to an element or one of its pseudo-elements,
/// composed with the animation overlay it holds, or 0 while it holds none.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_assigned_style_record(
    engine: StyleEngineHandle,
    node: u32,
    pseudo_kind: u8,
) -> u64 {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_assigned_style_record",
        crate::css::style::owner_calls::StyleQuery::AssignedStyleRecord { node, pseudo_kind },
    )
    .u64()
}

/// Answers [`style_engine_assigned_style_record`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_assigned_style_record`].
pub(crate) unsafe fn owner_assigned_style_record(engine: &StyleEngine, node: u32, pseudo_kind: u8) -> u64 {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return 0;
    };
    engine
        .retained
        .computed_group_sets
        .assigned_final_style_record(super::computed::ComputedStyleTarget::new(node, pseudo_kind))
        .map_or(0, |record| record.raw())
}

/// Replays an old capture's read of a record's group payloads, which the host no longer makes.
///
/// # Safety
/// `engine` must be live for this call and for every read through the returned pointer.
pub unsafe fn replay_style_record_payloads(engine: StyleEngineHandle, style_record: u64) -> *const c_void {
    let engine = unsafe { engine.for_replay() };
    engine
        .style_record_payloads(style_record)
        .map_or(std::ptr::null(), |payloads| payloads.as_ptr().cast())
}

/// What the winners the node's records were computed from read, beyond their cascade, one bit per
/// kind: see [`node_record_reads`].
pub mod node_record_reads {
    pub const IF_FUNCTION: u8 = 1 << 0;
    pub const INHERIT_FUNCTION: u8 = 1 << 1;
    pub const CUSTOM_FUNCTION: u8 = 1 << 2;
    /// An `attr()` substitution: an attribute change reaches such records.
    pub const ATTRIBUTES: u8 = 1 << 3;
    /// A sibling-position read from a retained winner.
    pub const TREE_COUNTING: u8 = 1 << 4;
}

impl StyleEngine {
    /// What the winners the node's records were computed from read beyond their cascade, as
    /// `node_record_reads` bits.
    fn node_record_reads(&self, node: StyleNodeID) -> u8 {
        // What the element's and its pseudo-elements' winners were written with, one walk of each
        // state. A pseudo-element's winners read its originating element's attributes, and
        // pseudo-elements resolve with the originating element's environment.
        let groups = self.current_winner_groups();
        let version = self.program.version();
        let (mut usage, mut reads_attributes) = self.retained.custom_declarations_reads(node, None);
        let mut uses_tree_counting = self.retained.nodes_with_tree_counting_records.contains(&node);
        if let super::partial_view::Lookup::Known((_, state)) =
            groups.token_for(super::cascade::WinnerGroupKey::current(node, version))
        {
            let written = self.retained.state_written_facts(node, state);
            usage |= written.custom_condition_usage;
            reads_attributes |= written.reads_attributes;
            uses_tree_counting |= written.has_written_tree_counting;
        }
        for (pseudo, pseudo_version, state, priority_current) in groups.pseudo_states(node) {
            let written = self.retained.state_written_facts(node, state);
            usage |= written.custom_condition_usage;
            reads_attributes |= written.reads_attributes;
            // Only a pseudo-element's current winners read its sibling position.
            uses_tree_counting |= pseudo_version == version && priority_current && written.has_written_tree_counting;
            if let Ok(kind) = u8::try_from(pseudo.kind.0) {
                let (pseudo_usage, pseudo_reads_attributes) = self.retained.custom_declarations_reads(node, Some(kind));
                usage |= pseudo_usage;
                reads_attributes |= pseudo_reads_attributes;
            }
        }
        let mut reads = usage
            & (node_record_reads::IF_FUNCTION
                | node_record_reads::INHERIT_FUNCTION
                | node_record_reads::CUSTOM_FUNCTION);
        if reads_attributes {
            reads |= node_record_reads::ATTRIBUTES;
        }
        if uses_tree_counting {
            reads |= node_record_reads::TREE_COUNTING;
        }
        reads
    }

    /// The `FfiStyleRowFact` word of an element row's node.
    fn style_row_facts(&self, node: StyleNodeID) -> u32 {
        let mut facts = FfiStyleRowFact::Present as u32 | u32::from(self.node_record_reads(node));
        if self.node_declares_custom_properties(node) {
            facts |= FfiStyleRowFact::DeclaresCustomProperties as u32;
        }
        facts
    }
}

/// Computes what moving an element from one final style record to another damages, from the
/// records and the element's own facts. The counter styles its box was built with are the host's to
/// compare.
///
/// # Safety
/// `engine` must be live and both style records must remain pinned or assigned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_element_record_damage(
    engine: StyleEngineInputHandle,
    node: u32,
    old_style_record: u64,
    new_style_record: u64,
) -> u32 {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_element_record_damage",
        crate::css::style::owner_calls::StyleQuery::ElementRecordDamage {
            node,
            old_style_record,
            new_style_record,
        },
    )
    .u32()
}

/// Answers [`style_engine_element_record_damage`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_element_record_damage`].
pub(crate) unsafe fn owner_element_record_damage(
    engine: &mut StyleEngine,
    node: u32,
    old_style_record: u64,
    new_style_record: u64,
) -> u32 {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return super::style_invalidation::unreadable_record_damage("ElementRecordDamageWithoutStyleNode");
    };
    engine.element_record_damage(node, false, old_style_record, new_style_record)
}

/// Computes what moving one of an element's pseudo-elements from one final style record to another
/// damages, where either record can be zero. `counter_styles_changed` is the host's comparison of
/// the counter styles the pseudo-element's box was built with.
///
/// # Safety
/// `engine` must be live and every nonzero style record must remain pinned or assigned.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_pseudo_element_record_damage(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
    old_style_record: u64,
    new_style_record: u64,
    originating_style_record: u64,
    counter_styles_changed: bool,
) -> u32 {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_pseudo_element_record_damage",
        crate::css::style::owner_calls::StyleQuery::PseudoElementRecordDamage {
            node,
            pseudo_kind,
            old_style_record,
            new_style_record,
            originating_style_record,
            counter_styles_changed,
        },
    )
    .u32()
}

/// Answers [`style_engine_pseudo_element_record_damage`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_pseudo_element_record_damage`].
pub(crate) unsafe fn owner_pseudo_element_record_damage(
    engine: &mut StyleEngine,
    node: u32,
    pseudo_kind: u8,
    old_style_record: u64,
    new_style_record: u64,
    originating_style_record: u64,
    counter_styles_changed: bool,
) -> u32 {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return super::style_invalidation::unreadable_record_damage("PseudoElementRecordDamageWithoutStyleNode");
    };
    engine.pseudo_element_record_damage(
        node,
        pseudo_kind,
        old_style_record,
        new_style_record,
        originating_style_record,
        counter_styles_changed,
    )
}

/// What the pass published for a row whose animations it sampled itself; `present` is false for a
/// row the host samples.
#[repr(C)]
pub struct FfiRowSampledInPass {
    pub present: bool,
    /// The composition the element installs.
    pub style_record: u64,
    /// What publishing the composition over the row's record invalidated.
    pub invalidation: FfiAnimationInvalidation,
    pub overlay_is_empty: bool,
    /// The `SUBSTITUTION_MARK_*` bits of every value a keyframe substituted.
    pub substitution_marks: u8,
    /// The style groups a keyframe read straight from the parent through an explicit `inherit`,
    /// or every group as `u32::MAX`.
    pub keyframes_inherited_non_inherited_style_groups: u32,
    pub uses_tree_counting_function: bool,
    /// Whether the sample moved the element's custom-property environment: to the one the engine
    /// composed its animated custom properties into, borrowed as its identity and its store, or,
    /// with zero and null, back to the one its record was published with.
    pub custom_property_environment_moved: bool,
    pub custom_property_environment: u64,
    pub custom_property_store: *const c_void,
    /// `1` where the element's own style reads custom properties, and `2` where a name its
    /// descendants inherit moved: what the host records for the next transaction.
    pub custom_property_reactions: u8,
    /// Whether the engine named the environment the sample moved the pseudo-element to itself, as
    /// it settled it: the host installs nothing of it and only records the reactions.
    pub custom_property_environment_named: bool,
    /// Whether building the composition rebuilt every style group.
    pub rebuilt_every_group: bool,
}

/// Takes what the pass published for a row whose animations it sampled, so that exactly one
/// installation applies it.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_take_row_sampled_in_pass(
    engine: StyleEngineInputHandle,
    node: u32,
) -> FfiRowSampledInPass {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_take_row_sampled_in_pass",
        crate::css::style::owner_calls::StyleQuery::TakeRowSampledInPass { node },
    )
    .row_sampled()
}

/// Answers [`style_engine_take_row_sampled_in_pass`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_take_row_sampled_in_pass`].
pub(crate) unsafe fn owner_take_row_sampled_in_pass(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
) -> FfiRowSampledInPass {
    let published = StyleNodeID::from_raw(node).and_then(|node| engine.take_row_sampled_in_pass(node));
    row_sampled_in_pass(engine, published)
}

/// Takes what the engine published for a synthetic pseudo-element whose animations it sampled as
/// it settled it, so that exactly one installation applies it.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_take_pseudo_element_sampled_in_pass(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
) -> FfiRowSampledInPass {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_take_pseudo_element_sampled_in_pass",
        crate::css::style::owner_calls::StyleQuery::TakePseudoElementSampledInPass { node, pseudo_kind },
    )
    .row_sampled()
}

/// Answers [`style_engine_take_pseudo_element_sampled_in_pass`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_take_pseudo_element_sampled_in_pass`].
pub(crate) unsafe fn owner_take_pseudo_element_sampled_in_pass(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo_kind: u8,
) -> FfiRowSampledInPass {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return row_sampled_in_pass(engine, None);
    };
    let published = engine.take_pseudo_element_sampled_in_pass(node, pseudo_kind);
    let mut sampled = row_sampled_in_pass(engine, published);
    sampled.custom_property_environment_named = engine.pseudo_element_environment_named_from_sample(node, pseudo_kind);
    sampled
}

/// The host installs a sample that moved the custom-property environment of an element, or of one
/// of its synthetic pseudo-elements, to `environment`, or, with zero, back to the one beneath: the
/// engine takes it itself, and the host views it by identity. Returns false where the host installs
/// its own view of it.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_install_sampled_custom_property_environment(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
    environment: u64,
) -> bool {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_install_sampled_custom_property_environment",
        crate::css::style::owner_calls::StyleQuery::InstallSampledCustomPropertyEnvironment {
            node,
            pseudo_kind,
            environment,
        },
    )
    .is()
}

/// Answers [`style_engine_install_sampled_custom_property_environment`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_install_sampled_custom_property_environment`].
pub(crate) unsafe fn owner_install_sampled_custom_property_environment(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo_kind: u8,
    environment: u64,
) -> bool {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return false;
    };
    match pseudo_kind {
        u8::MAX => engine.install_sampled_element_environment(node, environment),
        pseudo_kind => {
            engine
                .computed_group_sets
                .owns_animation_overlay_slot(super::computed::ComputedStyleTarget::new(node, pseudo_kind))
                && engine.install_sampled_pseudo_element_environment(node, pseudo_kind, environment)
        }
    }
}

/// The element or pseudo-element whose animations composed their custom properties into an
/// environment the engine resolved, for the host to view it as that pseudo-element's animation overlay. Returns
/// false for any other environment.
///
/// # Safety
/// `engine` must be live, and `node` and `pseudo_kind` writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_sampled_custom_property_environment_owner(
    engine: StyleEngineHandle,
    environment: u64,
    node: *mut u32,
    pseudo_kind: *mut u8,
) -> bool {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_sampled_custom_property_environment_owner",
        crate::css::style::owner_calls::StyleQuery::SampledCustomPropertyEnvironmentOwner {
            environment,
            node,
            pseudo_kind,
        },
    )
    .is()
}

/// Answers [`style_engine_sampled_custom_property_environment_owner`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_sampled_custom_property_environment_owner`].
pub(crate) unsafe fn owner_sampled_custom_property_environment_owner(
    engine: &StyleEngine,
    environment: u64,
    node: *mut u32,
    pseudo_kind: *mut u8,
) -> bool {
    let Some((owner, kind)) = engine
        .sampled_pseudo_element_custom_property_environments
        .iter()
        .find_map(|(&key, &sampled)| (sampled == environment).then_some(key))
        .or_else(|| {
            engine
                .sampled_custom_property_environments
                .iter()
                .find_map(|(&node, &sampled)| (sampled == environment).then_some((node, u8::MAX)))
        })
    else {
        return false;
    };
    unsafe {
        *node = owner.raw();
        *pseudo_kind = kind;
    }
    true
}

/// Sample the animations of an element, or of one of its pseudo-elements, over the record the host
/// holds for it, from the engine's own inputs, and publish the composition as its record. `present`
/// is false where the engine cannot, and the host samples it itself; an answer naming
/// `style_record` again says the sample moved nothing.
///
/// # Safety
/// `engine` must be live, and `layout_arena` the document's live layout arena or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_sample_installed_record(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
    style_record: u64,
    layout_arena: *mut c_void,
) -> FfiRowSampledInPass {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_sample_installed_record",
        crate::css::style::owner_calls::StyleQuery::SampleInstalledRecord {
            node,
            pseudo_kind,
            style_record,
            layout_arena,
        },
    )
    .row_sampled()
}

/// Answers [`style_engine_sample_installed_record`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_sample_installed_record`].
pub(crate) unsafe fn owner_sample_installed_record(
    engine: &mut StyleEngine,
    node: u32,
    pseudo_kind: u8,
    style_record: u64,
    layout_arena: *mut c_void,
) -> FfiRowSampledInPass {
    abort_on_panic(|| {
        let Some(style_node) = StyleNodeID::from_raw(node) else {
            return row_sampled_in_pass(engine, None);
        };
        let pseudo = (pseudo_kind != u8::MAX).then_some(pseudo_kind);
        let layout_arena = unsafe { super::animations::CommittedTransformReferenceBoxes::lend(layout_arena) };
        // The host samples at the times it published.
        let timeline_samples = engine.animation_timeline_samples().clone();
        let owns_slot = engine
            .computed_group_sets
            .owns_animation_overlay_slot(super::computed::ComputedStyleTarget::new(style_node, pseudo_kind));
        let sampled = match owns_slot {
            true => engine.sample_installed_record(style_node, pseudo, style_record, layout_arena, &timeline_samples),
            false => sample_record_without_overlay_slot(
                engine,
                style_node,
                pseudo_kind,
                style_record,
                layout_arena,
                &timeline_samples,
            ),
        };
        match sampled {
            Ok(published) => {
                super::engine_sample_check::note_taken("installed record sample");
                row_sampled_in_pass(engine, Some(published))
            }
            Err(reason) => {
                super::engine_sample_check::note_declined(&format!("installed record: {reason}"));
                row_sampled_in_pass(engine, None)
            }
        }
    })
}

/// Sample the animations of an element over the record the host holds for it at the times
/// `timeline_samples` names, and publish the composition as its record, as
/// [`style_engine_sample_installed_record`] does for the host: for a clock tick, which samples on
/// the render side. `None` where the element owns no overlay slot or the engine cannot sample it,
/// and the host samples it itself.
///
/// # Safety
/// `layout_arena` must be the document's live layout arena, which the caller owns, as it owns the
/// engine.
pub(crate) unsafe fn sample_installed_record_for_clock_tick(
    engine: &mut StyleEngine,
    node: StyleNodeID,
    style_record: u64,
    layout_arena: *mut c_void,
    timeline_samples: &super::animations::AnimationTimelineSamples,
) -> Option<FfiRowSampledInPass> {
    if !engine
        .computed_group_sets
        .owns_animation_overlay_slot(super::computed::ComputedStyleTarget::new(node, u8::MAX))
    {
        return None;
    }
    // SAFETY: Guaranteed by the caller.
    let layout_arena = unsafe { super::animations::CommittedTransformReferenceBoxes::lend(layout_arena) };
    match engine.sample_installed_record(node, None, style_record, layout_arena, timeline_samples) {
        Ok(published) => {
            super::engine_sample_check::note_taken("clock tick sample");
            Some(row_sampled_in_pass(engine, Some(published)))
        }
        Err(reason) => {
            super::engine_sample_check::note_declined(&format!("clock tick: {reason}"));
            None
        }
    }
}

/// Takes the next sample the last clock tick of the document whose layout arena is `layout_arena`
/// left for the host to adopt: the element's style node and the record it held before the tick, and
/// whether the arena took the sample's record ahead of the host. False once none is left.
///
/// # Safety
/// The out pointers must be valid for writes, and the tick taken back.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_clock_tick_take_entry(
    layout_arena: *mut c_void,
    node: *mut u32,
    style_record_before: *mut u64,
    installed_in_arena: *mut bool,
    sample: *mut FfiRowSampledInPass,
) -> bool {
    let Some(entry) = crate::clock_frames::take_clock_tick_entry(layout_arena) else {
        return false;
    };
    // SAFETY: Guaranteed by the caller.
    unsafe {
        *node = entry.style_node.raw();
        *style_record_before = entry.style_record_before;
        *installed_in_arena = entry.installed_in_arena;
        sample.write(entry.sample);
    }
    true
}

/// Sample the animations of a pseudo-element the engine holds no assignment for, which owns no
/// overlay slot, over the record the host holds for it, and publish the composition as the whole
/// record again over the same base, as the host's own publication does.
fn sample_record_without_overlay_slot(
    engine: &mut StyleEngine,
    node: StyleNodeID,
    pseudo_kind: u8,
    style_record: u64,
    layout_arena: super::animations::CommittedTransformReferenceBoxes,
    timeline_samples: &super::animations::AnimationTimelineSamples,
) -> Result<super::engine_sample::SettledRowPublication, String> {
    let pseudo = (pseudo_kind != u8::MAX).then_some(pseudo_kind);
    let sample = crate::css::style_compute::sample_settled_row(
        &mut engine.state,
        node,
        pseudo,
        false,
        Some(style_record),
        None,
        layout_arena,
        timeline_samples,
    )?;
    if !sample.animated_custom_properties.is_empty() {
        return Err("a sample without an overlay slot that animates custom properties".into());
    }
    let overlay_is_empty = unsafe { sample.style.overlay.as_ref() }.is_none_or(|overlay| overlay.is_empty());
    let unchanged = super::engine_sample::SettledRowPublication {
        style_record,
        custom_properties: None,
        invalidation: FfiAnimationInvalidation::default(),
        overlay_is_empty,
        substitution_marks: sample.substitution_marks,
        keyframes_inherited_non_inherited_style_groups: sample.keyframes_inherited_non_inherited_style_groups,
        uses_tree_counting_function: sample.uses_tree_counting_function,
        rebuilt_every_group: false,
    };
    if !engine.animation_overlay_changed(style_record, sample.style.overlay) {
        return Ok(unchanged);
    }
    let payloads = {
        let StyleEngine { state, counters } = engine;
        state.build_settled_row_payloads(node, pseudo_kind, &sample, counters)?
    };
    let shared = SharedPayload::from_pointer_slice(&payloads.payloads);
    let invalidation = engine.compare_animation_overlay(style_record, sample.style.overlay, shared, false);
    let (base_payloads, counter_style_environment_identity, longhand_table) = {
        let view = engine
            .computed_group_sets
            .style_record_view(style_record)
            .ok_or("a record with no view")?;
        let base_payloads = match view.base_payloads.is_empty() {
            true => view.payloads.to_vec(),
            false => view.base_payloads.to_vec(),
        };
        (
            base_payloads,
            view.counter_style_environment_identity,
            view.longhand_table,
        )
    };
    let custom_property_environment = engine
        .computed_group_sets
        .style_record_custom_property_environment(engine.computed_group_sets.base_style_record_of(style_record))
        .unwrap_or(0);
    let custom_property_store = match custom_property_environment {
        0 => std::ptr::null(),
        environment => engine
            .custom_property_environments
            .store(environment)
            .ok_or("an environment without a store")?,
    };
    let identity = match overlay_is_empty {
        true => 0,
        false => {
            engine.retained.next_engine_animation_overlay_identity += 1;
            (1 << 63) | engine.retained.next_engine_animation_overlay_identity
        }
    };
    let delta = publish_computed_groups_from_inputs(
        engine,
        node.raw(),
        pseudo_kind,
        &base_payloads,
        super::computed::ENGINE_INHERITED_GROUP_COUNT,
        custom_property_environment,
        false,
        counter_style_environment_identity,
        identity,
        match overlay_is_empty {
            true => std::ptr::null(),
            false => sample.style.overlay.cast_const().cast(),
        },
        match overlay_is_empty {
            true => &[],
            false => shared,
        },
        unsafe { longhand_table.as_ref() },
        custom_property_store,
    );
    if delta.new_style_record == 0 {
        return Err("no whole publication".into());
    }
    Ok(super::engine_sample::SettledRowPublication {
        style_record: delta.new_style_record,
        invalidation,
        rebuilt_every_group: payloads.rebuilt_every_group,
        ..unchanged
    })
}

/// Decide the transition step of an element, or of one of its pseudo-elements, over the record the
/// host installed for it, from `before_change_style_record`, as the host would decide it now, and
/// publish the composition the step leaves as its record. The host takes the decisions as it takes
/// a step the pass decided. `present` is false where the engine cannot decide it, and the host
/// does; an answer naming `installed_style_record` again says the step moved nothing composed.
///
/// # Safety
/// `engine` must be live, and `layout_arena` the document's live layout arena or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_decide_transition_step_for_installed_record(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
    before_change_style_record: u64,
    installed_style_record: u64,
    layout_arena: *mut c_void,
) -> FfiRowSampledInPass {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_decide_transition_step_for_installed_record",
        crate::css::style::owner_calls::StyleQuery::DecideTransitionStepForInstalledRecord {
            node,
            pseudo_kind,
            before_change_style_record,
            installed_style_record,
            layout_arena,
        },
    )
    .row_sampled()
}

/// Answers [`style_engine_decide_transition_step_for_installed_record`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_decide_transition_step_for_installed_record`].
pub(crate) unsafe fn owner_decide_transition_step_for_installed_record(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo_kind: u8,
    before_change_style_record: u64,
    installed_style_record: u64,
    layout_arena: *mut c_void,
) -> FfiRowSampledInPass {
    abort_on_panic(|| {
        let Some(style_node) = StyleNodeID::from_raw(node) else {
            return row_sampled_in_pass(engine, None);
        };
        let pseudo = (pseudo_kind != u8::MAX).then_some(pseudo_kind);
        let layout_arena = unsafe { super::animations::CommittedTransformReferenceBoxes::lend(layout_arena) };
        // The host steps at the times it published.
        let timeline_samples = engine.animation_timeline_samples().clone();
        match engine.decide_installed_record_transition_step(
            style_node,
            pseudo,
            before_change_style_record,
            installed_style_record,
            layout_arena,
            &timeline_samples,
        ) {
            Ok(published) => {
                super::engine_sample_check::note_taken("installed record transition step");
                let mut answer = row_sampled_in_pass(engine, published);
                if !answer.present {
                    answer.present = true;
                    answer.style_record = installed_style_record;
                }
                answer
            }
            Err(reason) => {
                super::engine_sample_check::note_declined(&format!("installed record transition step: {reason}"));
                row_sampled_in_pass(engine, None)
            }
        }
    })
}

impl FfiRowSampledInPass {
    /// What a row the host samples itself is answered with.
    pub(crate) fn absent() -> Self {
        Self {
            present: false,
            style_record: 0,
            invalidation: FfiAnimationInvalidation::default(),
            overlay_is_empty: true,
            substitution_marks: 0,
            keyframes_inherited_non_inherited_style_groups: 0,
            uses_tree_counting_function: false,
            custom_property_environment_moved: false,
            custom_property_environment: 0,
            custom_property_store: std::ptr::null(),
            custom_property_reactions: 0,
            custom_property_environment_named: false,
            rebuilt_every_group: false,
        }
    }
}

fn row_sampled_in_pass(
    engine: &StyleEngine,
    published: Option<super::engine_sample::SettledRowPublication>,
) -> FfiRowSampledInPass {
    match published {
        None => FfiRowSampledInPass::absent(),
        Some(published) => FfiRowSampledInPass {
            present: true,
            style_record: published.style_record,
            invalidation: published.invalidation,
            overlay_is_empty: published.overlay_is_empty,
            substitution_marks: published.substitution_marks,
            keyframes_inherited_non_inherited_style_groups: published.keyframes_inherited_non_inherited_style_groups,
            uses_tree_counting_function: published.uses_tree_counting_function,
            custom_property_environment_moved: published.custom_properties.is_some(),
            custom_property_environment: published.custom_properties.map_or(0, |moved| moved.environment),
            custom_property_store: published
                .custom_properties
                .and_then(|moved| engine.custom_property_environments.store(moved.environment))
                .unwrap_or(std::ptr::null()),
            custom_property_reactions: published.custom_properties.map_or(0, |moved| {
                u8::from(moved.element_reads) | (u8::from(moved.inherited_names_moved) << 1)
            }),
            custom_property_environment_named: false,
            rebuilt_every_group: published.rebuilt_every_group,
        },
    }
}

/// The animation definitions one engine-settled row left for the host, as the host's own plan
/// application takes them: a borrowed array of `ComputedValuesFFI::FfiComputedAnimation`.
#[repr(C)]
pub struct FfiSettledAnimationDefinitions {
    /// Borrowed until the next row's definitions are taken, and null for a row that names no
    /// animation - which is still a row whose animation declarations moved.
    pub definitions: *const c_void,
    pub count: usize,
    /// Whether the row owed a plan at all.
    pub owed: bool,
    /// Whether the element is in a `display: none` subtree, which starts no animation.
    pub in_display_none_subtree: bool,
}

impl FfiSettledAnimationDefinitions {
    /// What a row that owes no animation plan is answered with.
    pub(crate) fn absent() -> Self {
        Self {
            definitions: std::ptr::null(),
            count: 0,
            owed: false,
            in_display_none_subtree: false,
        }
    }
}

/// Takes the animation plan an engine-settled row left for the host, so that exactly one
/// application drains it.
///
/// # Safety
/// `engine` must be live for this call, and the definitions must be read before the next call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_take_settled_animation_definitions(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
) -> FfiSettledAnimationDefinitions {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_take_settled_animation_definitions",
        crate::css::style::owner_calls::StyleQuery::TakeSettledAnimationDefinitions { node, pseudo_kind },
    )
    .animation_definitions()
}

/// Answers [`style_engine_take_settled_animation_definitions`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_take_settled_animation_definitions`].
pub(crate) unsafe fn owner_take_settled_animation_definitions(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo_kind: u8,
) -> FfiSettledAnimationDefinitions {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return FfiSettledAnimationDefinitions::absent();
    };
    let Some(element_display_is_none) = engine
        .take_settled_animation_definitions(node, pseudo_kind)
        .map(|plan| plan.element_display_is_none())
    else {
        return FfiSettledAnimationDefinitions::absent();
    };
    // https://drafts.csswg.org/css-animations-1/#animations
    // An element that is not rendered starts no animation. The record says whether the element's
    // own display is `none`; a pseudo-element's originating element and any other element's parent
    // and its ancestors say the rest. Which definitions start an animation is decided as the plan
    // is applied, against the animations the element holds then, so this is answered either way.
    let in_display_none_subtree = element_display_is_none || {
        let start = match pseudo_kind {
            u8::MAX => engine.tree().parent(node).or_else(|| engine.tree().host_of(node)),
            _ => Some(node),
        };
        start.is_some_and(|start| {
            super::animations::has_inclusive_ancestor_with_display_none_ignoring_animations(engine, start)
        })
    };
    let plan = engine
        .settled_animation_definitions_being_applied()
        .expect("the plan was just taken");
    FfiSettledAnimationDefinitions {
        definitions: plan.definitions().as_ptr().cast(),
        count: plan.definitions().len(),
        owed: true,
        in_display_none_subtree,
    }
}

/// The record as a published value that owns everything a read of it reads (see
/// [`super::published_record`]), which the caller owns one reference of; null for a record the
/// engine no longer holds. Only the drain installing what a pass published hands a record to the
/// host this way: every read of it after is made through the value.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_publish_style_record(
    engine: StyleEngineHandle,
    style_record: u64,
) -> *const c_void {
    crate::css::style::owner_calls::ask_records(
        engine,
        "style_engine_publish_style_record",
        crate::css::style::owner_calls::StyleQuery::PublishStyleRecord { style_record },
    )
    .pointer()
}

/// Answers [`style_engine_publish_style_record`] from `engine`, on the render owner.
pub(crate) fn owner_publish_style_record(engine: &StyleEngine, style_record: u64) -> *const c_void {
    if engine.recording_id().is_some() {
        record_style_record_view(engine, style_record);
    }
    engine
        .publish_style_record(style_record)
        .map_or(std::ptr::null(), super::published_record::into_handle)
}

/// The engine's borrowed view of a base or live animation-overlay record, recorded as the host's
/// first read of it: the host reads a record through what `style_engine_publish_style_record`
/// published, which is this view of it.
fn record_style_record_view(engine: &StyleEngine, style_record: u64) -> FfiStyleRecordView {
    let view = engine.style_record_view(style_record);
    let result = match &view {
        None => FfiStyleRecordView::missing(),
        Some(view) => FfiStyleRecordView {
            payloads: SharedPayload::as_pointer_slice(view.payloads).as_ptr(),
            base_payloads: SharedPayload::as_pointer_slice(view.base_payloads).as_ptr(),
            longhand_table: view.longhand_table.cast().as_ptr(),
            animated_overlay: view.animated_overlay.cast().as_ptr(),
            payload_count: view.payloads.len(),
            pseudo_element_styles: view.pseudo_element_styles,
            counter_style_environment_identity: view.counter_style_environment_identity,
            animation_overlay_identity: view.animation_overlay_identity,
            dependency_flags: view.dependency_flags,
            present: true,
        },
    };
    let record_response = engine.recording_first_response(1, style_record);
    if !record_response {
        return result;
    }
    engine.record_boundary_call(EventKind::StyleRecordView, |payload| {
        payload.write_u64(style_record);
        payload.write_bool(record_response);
        payload.write_bool(result.present);
        let Some(view) = view else {
            return;
        };
        let base_payloads = engine
            .recording_computed_group_identities(style_record)
            .expect("a style record view must retain its base computed groups")
            .into_iter()
            .map(|identity| u64::from(identity) + 1)
            .collect::<Vec<_>>();
        let style_payloads = match view.animation_overlay_identity {
            0 => base_payloads.clone(),
            _ => view
                .payloads
                .iter()
                .map(|&pointer| {
                    engine
                        .recording_pointer_token(pointer.addr())
                        .expect("an enabled recorder must tokenize the pointer")
                })
                .collect(),
        };
        payload.write_length(style_payloads.len());
        for pointer in style_payloads {
            payload.write_u64(pointer);
        }
        payload.write_length(base_payloads.len());
        for pointer in base_payloads {
            payload.write_u64(pointer);
        }
        let longhand_table = unsafe { view.longhand_table.as_ref() };
        payload.write_bytes(longhand_table.map_or(&[], |table| table.importance_bits()));
        payload.write_bytes(longhand_table.map_or(&[], |table| table.inheritance_bits()));
        payload.write_length(longhand_table.map_or(0, |table| table.inheritance_dependent_values().count()));
        for (property, value) in longhand_table
            .into_iter()
            .flat_map(crate::css::computed_longhand_table::ComputedLonghandTable::inheritance_dependent_values)
        {
            payload.write_u16(property);
            payload.write_u64(
                engine
                    .recording_pointer_token(value as usize)
                    .expect("an enabled recorder must tokenize the pointer"),
            );
        }
        let raw_cascaded_font_size = longhand_table.map_or(std::ptr::null(), |table| table.raw_cascaded_font_size());
        payload.write_u64(match raw_cascaded_font_size.is_null() {
            true => 0,
            false => engine
                .recording_pointer_token(raw_cascaded_font_size as usize)
                .expect("an enabled recorder must tokenize the pointer"),
        });
        payload.write_u64(u64::from(!view.animated_overlay.is_null()));
        payload.write_u64(view.pseudo_element_styles);
        payload.write_u64(view.counter_style_environment_identity);
        payload.write_u64(view.animation_overlay_identity);
        payload.write_u8(view.dependency_flags);
        payload.write_length(view.longhand_values.len());
        for &value in view.longhand_values {
            payload.write_u64(match value.is_null() {
                true => 0,
                false => engine
                    .recording_pointer_token(value.addr())
                    .expect("an enabled recorder must tokenize the pointer"),
            });
        }
    });
    result
}

/// Replays an old capture's read of a record's view, which the host no longer makes.
///
/// # Safety
/// `engine` must be live for this call and for every read through the returned pointers.
pub unsafe fn replay_style_record_view(engine: StyleEngineHandle, style_record: u64) -> FfiStyleRecordView {
    record_style_record_view(unsafe { engine.for_replay() }, style_record)
}

/// Removes the retained computed-input assignment for one pseudo-element kind.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_remove_computed_pseudo(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
) -> FfiStyleRecordDelta {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_remove_computed_pseudo",
        crate::css::style::owner_calls::StyleQuery::RemoveComputedPseudo { node, pseudo_kind },
    )
    .record_delta()
}

/// Answers [`style_engine_remove_computed_pseudo`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_remove_computed_pseudo`].
pub(crate) unsafe fn owner_remove_computed_pseudo(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo_kind: u8,
) -> FfiStyleRecordDelta {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return FfiStyleRecordDelta::default();
    };
    let result = FfiStyleRecordDelta {
        old_style_record: engine
            .remove_computed_pseudo(node, pseudo_kind)
            .map_or(0, super::computed::FinalStyleRecordID::raw),
        new_style_record: 0,
    };
    engine.record_boundary_call(EventKind::RemoveComputedPseudo, |payload| {
        payload.write_u32(node.raw());
        payload.write_u8(pseudo_kind);
        payload.write_u64(result.old_style_record);
        payload.write_u64(result.new_style_record);
    });
    result
}
pub(crate) fn publish_rule_declarations(
    engine: &mut StyleEngine,
    rule: u32,
    data: &crate::css::declaration_block::DeclarationBlockData,
) -> bool {
    if rule == 0 {
        return false;
    }
    let mut has_transitions = false;
    let declared = data
        .properties
        .iter()
        .map(|declaration| {
            use crate::css::property_metadata::property_defines_a_css_transition;
            has_transitions |= property_defines_a_css_transition(declaration.property_id);
            engine.intern_declared_property(declaration)
        })
        .collect::<Vec<_>>();
    let written_values: Vec<_> = data
        .properties
        .iter()
        .map(|declaration| unsafe {
            RetainedStyleValueData::from_retained_pointer(std::sync::Arc::into_raw(declaration.value.clone()))
        })
        .collect();
    let (custom_declarations, custom_written_values) =
        collect_native_custom_declarations(engine, &data.custom_properties);
    engine.set_rule_declared_properties_with_written_values(
        RuleID(rule - 1),
        &declared,
        written_values,
        custom_declarations.clone(),
        custom_written_values,
    );
    engine.record_boundary_call(EventKind::SetRuleDeclaredProperties, |payload| {
        payload.write_u32(rule);
        write_declared_properties(&declared, payload);
        write_custom_declarations(&custom_declarations, payload);
    });
    has_transitions
}

/// Replays a record moved to a moved environment.
///
/// # Safety
/// `engine` must be live.
pub unsafe fn replay_republish_record_environment(engine: StyleEngineHandle, node: u32, environment: u64) -> u64 {
    let engine = unsafe { engine.for_replay() };
    StyleNodeID::from_raw(node)
        .and_then(|node| engine.republish_record_environment(node, environment))
        .unwrap_or(0)
}

/// A style read the host has to answer synchronously, as a CSSOM read of an element whose style is
/// not up to date does: the record of an element or, for `pseudo_kind != u8::MAX`, of one of its
/// pseudo-elements, which no style update installs.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RecordDemand {
    pub(crate) node: u32,
    pub(crate) pseudo_kind: u8,
    pub(crate) exclude_inline_style: bool,
    pub(crate) targeted: bool,
    pub(crate) read_only: bool,
    pub(crate) parent_highlight: u64,
}

/// The answer to a [`RecordDemand`]: the record, as the value the host reads it through, beside what
/// the engine says of it.
pub(crate) struct RecordDemandAnswer {
    /// The answer as the host takes it, with no record handle of its own.
    ffi: FfiRecordDemandAnswer,
    record: Option<std::sync::Arc<super::published_record::PublishedStyleRecord>>,
}

impl std::fmt::Debug for RecordDemandAnswer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordDemandAnswer")
            .field("style_record", &self.ffi.record.style_record)
            .field("is_absent", &self.ffi.is_absent)
            .finish_non_exhaustive()
    }
}

impl RecordDemand {
    /// Answers the demand with `engine`, where the engine computes every other record.
    pub(crate) fn answer(self, engine: &mut StyleEngine) -> RecordDemandAnswer {
        let ffi = answer_record_demand_for_host(
            engine,
            self.node,
            self.pseudo_kind,
            self.exclude_inline_style,
            self.targeted,
            self.read_only,
            self.parent_highlight,
        );
        let record = (!ffi.is_absent)
            .then(|| engine.publish_style_record(ffi.record.style_record))
            .flatten();
        RecordDemandAnswer { ffi, record }
    }
}

impl RecordDemandAnswer {
    /// The answer as the host takes it, which owns a reference to the record.
    pub(crate) fn into_ffi(self) -> FfiRecordDemandAnswer {
        FfiRecordDemandAnswer {
            published_record: self
                .record
                .map_or(std::ptr::null(), super::published_record::into_handle),
            ..self.ffi
        }
    }
}

/// Answer a style read the host has to answer synchronously, as a CSSOM read does. The render owner
/// answers it with the engine of the document's render state, in a round trip after the document's
/// earlier changes. `pseudo_kind == u8::MAX` selects the originating element.
///
/// # Safety
/// `engine` must be live. `layout_arena` is the document's live layout arena or null, which names the
/// same document as the engine's link.
#[unsafe(no_mangle)]
#[allow(clippy::too_many_arguments)]
pub unsafe extern "C" fn style_engine_answer_read_demand(
    engine: StyleEngineInputHandle,
    _layout_arena: *mut c_void,
    node: u32,
    pseudo_kind: u8,
    exclude_inline_style: bool,
    targeted: bool,
    read_only: bool,
    parent_highlight: u64,
) -> FfiRecordDemandAnswer {
    let demand = RecordDemand {
        node,
        pseudo_kind,
        exclude_inline_style,
        targeted,
        read_only,
        parent_highlight,
    };
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_answer_read_demand",
        crate::css::style::owner_calls::StyleQuery::ReadDemand(demand),
    )
    .record_demand()
}

/// Replays a recorded record demand.
///
/// # Safety
/// `engine` must be live.
pub unsafe fn replay_answer_read_demand(
    engine: StyleEngineHandle,
    node: u32,
    pseudo_kind: u8,
    exclude_inline_style: bool,
    targeted: bool,
    read_only: bool,
    parent_highlight: u64,
) -> FfiRecordDemandAnswer {
    let demand = RecordDemand {
        node,
        pseudo_kind,
        exclude_inline_style,
        targeted,
        read_only,
        parent_highlight,
    };
    // The replay compares what the engine answered, and holds none of the records.
    demand.answer(unsafe { engine.for_replay() }).ffi
}

#[allow(clippy::too_many_arguments)]
fn answer_record_demand_for_host(
    engine: &mut StyleEngine,
    node: u32,
    pseudo_kind: u8,
    exclude_inline_style: bool,
    targeted: bool,
    read_only: bool,
    parent_highlight: u64,
) -> FfiRecordDemandAnswer {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return FfiRecordDemandAnswer::absent();
    };
    let mut result = match engine.answer_record_demand(
        node,
        (pseudo_kind != u8::MAX).then_some(pseudo_kind),
        exclude_inline_style,
        targeted,
        read_only,
        parent_highlight,
    ) {
        super::publication::RecordDemandAnswer::Record(answer) => FfiRecordDemandAnswer {
            record: FfiEngineComputedRecord {
                style_record: answer.style_record,
                uses_substitution: engine.nodes_with_substituted_records.contains(&node),
                pseudo_records_present: answer.pseudo_records_present,
                pseudo_records: answer.pseudo_records,
            },
            is_absent: false,
            is_provisional: answer.provisional,
            row_facts: 0,
            published_record: std::ptr::null(),
            unanswered: false,
        },
        super::publication::RecordDemandAnswer::Absent => FfiRecordDemandAnswer::absent(),
    };
    result.row_facts = engine.style_row_facts(node);
    engine.record_boundary_call(EventKind::AnswerRecordDemand, |payload| {
        payload.write_u32(node.raw());
        payload.write_u8(pseudo_kind);
        payload.write_bool(exclude_inline_style);
        payload.write_bool(targeted);
        payload.write_bool(read_only);
        payload.write_u64(parent_highlight);
        payload.write_u64(result.record.style_record);
        payload.write_bool(result.is_absent);
        payload.write_bool(result.record.uses_substitution);
        payload.write_u8(result.record.pseudo_records_present);
        for record in result.record.pseudo_records {
            payload.write_u64(record);
        }
        // Where a declined demand once named its cause; kept so recordings keep their format.
        payload.write_bytes(&[]);
    });
    result
}

/// The record of an element no rule reaches, computed from its presentational hints and its
/// inline style alone over the initial values; see `declared_only_record`. `subject` is the
/// document's style node. Returns the record as a published value (see
/// [`super::published_record`]), which the caller owns one reference of, or null when the engine
/// cannot compute it. Nothing pins the record itself.
///
/// # Safety
/// `engine` must be live. `hints` must borrow `hint_count` `FfiDeclaredProperty` entries whose
/// values point at live, Arc-backed `StyleValueData` roots. A non-null `inline_block` must borrow
/// a live `DeclarationBlock`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_declared_only_record(
    engine: StyleEngineInputHandle,
    subject: u32,
    facts: u32,
    hint_kind: FfiElementDeclarationKind,
    hints: *const c_void,
    hint_count: usize,
    inline_block: *const c_void,
) -> *const c_void {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_declared_only_record",
        crate::css::style::owner_calls::StyleQuery::DeclaredOnlyRecord {
            subject,
            facts,
            hint_kind,
            hints,
            hint_count,
            inline_block,
        },
    )
    .pointer()
}

/// Answers [`style_engine_declared_only_record`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_declared_only_record`].
pub(crate) unsafe fn owner_declared_only_record(
    engine: &mut StyleEngine,
    subject: u32,
    facts: u32,
    hint_kind: FfiElementDeclarationKind,
    hints: *const c_void,
    hint_count: usize,
    inline_block: *const c_void,
) -> *const c_void {
    abort_on_panic(|| {
        // A record held by no node is no event a replay could reproduce.
        let Some(subject) = StyleNodeID::from_raw(subject).filter(|_| engine.recording_id().is_none()) else {
            return std::ptr::null();
        };
        unsafe {
            with_declared_only_declarations(hint_kind, hints, hint_count, inline_block, |declarations| {
                let Some(record) = engine.declared_only_record(subject, facts, declarations) else {
                    return std::ptr::null();
                };
                let published = engine.publish_style_record(record.raw());
                engine.unpin_style_record(record.raw());
                published.map_or(std::ptr::null(), super::published_record::into_handle)
            })
        }
    })
}

/// The declarations a declared-only record cascades, hints first and inline style after them.
///
/// # Safety
/// As for `style_engine_declared_only_record`.
unsafe fn with_declared_only_declarations<R>(
    hint_kind: FfiElementDeclarationKind,
    hints: *const c_void,
    hint_count: usize,
    inline_block: *const c_void,
    body: impl FnOnce(&[(ElementDeclarationKind, &crate::css::declaration_block::DeclaredProperty)]) -> R,
) -> R {
    use crate::css::declaration_block::{DeclarationBlock, FfiDeclaredProperty, declaration_from_view};
    let hints = if hint_count == 0 {
        &[]
    } else {
        unsafe { std::slice::from_raw_parts(hints.cast::<FfiDeclaredProperty>(), hint_count) }
    };
    let hints = hints
        .iter()
        .map(|hint| unsafe { declaration_from_view(hint) })
        .collect::<Vec<_>>();
    let inline_data = unsafe { inline_block.cast::<DeclarationBlock>().as_ref() }.map(DeclarationBlock::data);
    let hint_kind = decode_element_declaration_kind(hint_kind);
    let declarations = hints
        .iter()
        .map(|hint| (hint_kind, hint))
        .chain(inline_data.iter().flat_map(|data| {
            data.properties
                .iter()
                .map(|declaration| (ElementDeclarationKind::InlineStyle, declaration))
        }))
        .collect::<Vec<_>>();
    body(&declarations)
}

/// The host installs the record of a synthetic pseudo-element the engine settled: the
/// pseudo-element takes the custom-property environment the engine named for it as it settled it.
/// Returns false where the engine named none, and the host installs the environment itself.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_take_pseudo_element_environment_named_in_settle(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo_kind: u8,
) -> bool {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_take_pseudo_element_environment_named_in_settle",
        crate::css::style::owner_calls::StyleQuery::TakePseudoElementEnvironmentNamedInSettle { node, pseudo_kind },
    )
    .is()
}

/// Answers [`style_engine_take_pseudo_element_environment_named_in_settle`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_take_pseudo_element_environment_named_in_settle`].
pub(crate) unsafe fn owner_take_pseudo_element_environment_named_in_settle(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo_kind: u8,
) -> bool {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return false;
    };
    engine.take_pseudo_element_environment_named_in_settle(node, pseudo_kind)
}

/// The store of an environment the engine resolved, with one strong reference transferred to the
/// caller, and the environment it was resolved over; null for an environment C++ published.
///
/// # Safety
/// `engine` must be live and `parent` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_borrow_engine_custom_property_environment(
    engine: StyleEngineHandle,
    identity: u64,
    parent: *mut u64,
) -> *const c_void {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_borrow_engine_custom_property_environment",
        crate::css::style::owner_calls::StyleQuery::BorrowEngineCustomPropertyEnvironment { identity, parent },
    )
    .pointer()
}

/// Answers [`style_engine_borrow_engine_custom_property_environment`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_borrow_engine_custom_property_environment`].
pub(crate) unsafe fn owner_borrow_engine_custom_property_environment(
    engine: &StyleEngine,
    identity: u64,
    parent: *mut u64,
) -> *const c_void {
    let Some((store, parent_identity)) = engine.custom_property_environments.engine_environment(identity) else {
        return std::ptr::null();
    };
    unsafe {
        std::sync::Arc::increment_strong_count(store.cast::<crate::css::custom_properties::CustomPropertyStore>());
        *parent = parent_identity;
    }
    store
}

// The raw identity must remain a live Utf16FlyString for the native engine to retain it.
unsafe fn note_native_custom_property_name(engine: &mut StyleEngine, name: StyleAtomID, raw: usize, text: &[u16]) {
    unsafe { engine.note_custom_property_name(name, raw, text) };
    engine.record_boundary_call(EventKind::NoteCustomPropertyName, |payload| {
        payload.write_u32(name.0);
        payload.write_u16_slice(text);
    });
}

/// Replays recorded element parts.
///
/// # Safety
/// `engine` must be live.
pub unsafe fn replay_set_element_parts(engine: StyleEngineHandle, node: u32, names: &[u32], hosts: &[u32]) {
    let engine = unsafe { engine.for_replay() };
    let Some(node) = StyleNodeID::from_raw(node) else {
        return;
    };
    let pairs = names
        .iter()
        .zip(hosts)
        .filter_map(|(name, host)| StyleNodeID::from_raw(*host).map(|host| (StyleAtomID(*name), host)))
        .collect::<Vec<_>>();
    set_element_parts(engine, node, &pairs);
}

/// Replays a recorded element language.
///
/// # Safety
/// `engine` must be live.
pub unsafe fn replay_set_element_language(engine: StyleEngineHandle, node: u32, language: u32, text: &[u16]) {
    let engine = unsafe { engine.for_replay() };
    set_element_language(engine, node, language, text);
}

/// Replays a recorded custom-property name without a fly string behind it.
///
/// # Safety
/// `engine` must be live.
pub unsafe fn replay_note_custom_property_name(engine: StyleEngineHandle, name: u32, text: &[u16]) {
    let engine = unsafe { engine.for_replay() };
    unsafe { engine.note_custom_property_name(StyleAtomID(name), 0, text) };
}

#[repr(C)]
pub struct FfiNativeRuleTarget {
    pub identity: u64,
    pub declaration_version: u32,
    pub source_identity: u64,
    pub declarations: *const c_void,
    pub layer_name: *const u16,
    pub layer_name_length: usize,
    pub has_container_conditions: bool,
    pub origin: FfiCascadeOrigin,
}

#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FfiNativeContainerMatchResult {
    pub matches: bool,
    pub depends_on_size: bool,
    pub depends_on_style: bool,
    pub effects: *mut c_void,
}

#[derive(Clone, Copy)]
#[repr(u8)]
pub enum FfiContainerEffectKind {
    SizeContainerUsage,
    StyleContainerUsage,
    ScrollStateContainerUsage,
    NeedsEvaluationAfterLayout,
    SubjectViewportDependency,
    CustomPropertyReference,
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiContainerEffect {
    pub style_node: u32,
    pub kind: FfiContainerEffectKind,
    pub name: *const u16,
    pub name_length: usize,
}

struct ContainerEffects {
    effects: Vec<(u32, FfiContainerEffectKind, Vec<u16>)>,
}

#[unsafe(no_mangle)]
/// # Safety
///
/// `effects` is null or a live handle returned by `style_engine_native_rule_matches_containers`.
pub unsafe extern "C" fn style_engine_native_container_effect_count(effects: *const c_void) -> usize {
    super::seal::note_engine_call("style_engine_native_container_effect_count");
    unsafe { effects.cast::<ContainerEffects>().as_ref() }.map_or(0, |effects| effects.effects.len())
}

#[unsafe(no_mangle)]
/// # Safety
///
/// `effects` is a live handle returned by `style_engine_native_rule_matches_containers`, and
/// `index` is smaller than the count returned for that handle.
pub unsafe extern "C" fn style_engine_native_container_effect(
    effects: *const c_void,
    index: usize,
) -> FfiContainerEffect {
    super::seal::note_engine_call("style_engine_native_container_effect");
    let (style_node, kind, name) = &unsafe { &*effects.cast::<ContainerEffects>() }.effects[index];
    FfiContainerEffect {
        style_node: *style_node,
        kind: *kind,
        name: name.as_ptr(),
        name_length: name.len(),
    }
}

#[unsafe(no_mangle)]
/// # Safety
///
/// `effects` is null or a live handle returned by `style_engine_native_rule_matches_containers`
/// that has not already been released.
pub unsafe extern "C" fn style_engine_native_container_effects_release(effects: *mut c_void) {
    super::seal::note_engine_call("style_engine_native_container_effects_release");
    if !effects.is_null() {
        drop(unsafe { Box::from_raw(effects.cast::<ContainerEffects>()) });
    }
}

/// Resolve a native rule identity in this document's engine, including shared sheets.
///
/// # Safety
/// Engine must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_native_rule_id(engine: StyleEngineHandle, identity: u64) -> u32 {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_native_rule_id",
        crate::css::style::owner_calls::StyleQuery::NativeRuleId { identity },
    )
    .u32()
}

/// Answers [`style_engine_native_rule_id`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_native_rule_id`].
pub(crate) unsafe fn owner_native_rule_id(engine: &StyleEngine, identity: u64) -> u32 {
    engine.native_rule_id(identity).map_or(0, |id| id.0 + 1)
}

/// Publish a native declaration edit through its owning rule and return whether it declares
/// transitions. The host is notified before publishing, without an engine or graph borrow.
///
/// # Safety
/// Engine and rule must be live. The callback must not mutate the native rule graph or destroy the
/// engine, and must remain valid for this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_native_rule_declarations_changed(
    engine: StyleEngineInputHandle,
    rule: *const c_void,
    context: *mut c_void,
    notify: unsafe extern "C" fn(*mut c_void, u32),
) -> bool {
    let (identity, declarations) = {
        let rule = unsafe { &*rule.cast::<crate::css::rule::NativeRule>() };
        let Some(identity) = rule.declaration_owner_identity() else {
            return false;
        };
        (identity, rule.cascade_declarations())
    };
    // The host's notification reaches no engine (it moves the document's style environment version on), so the owner
    // publishes the declarations first, in one round trip, and the host hears of a rule the engine holds after.
    let published = crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_native_rule_declarations_changed",
        crate::css::style::owner_calls::StyleQuery::RuleDeclarationsChanged { identity, declarations },
    )
    .rule_declarations();
    let Some(published) = published else {
        return false;
    };
    super::seal::note_host_call("native_rule_declarations_changed.notify");
    unsafe { notify(context, published.rule) };
    published.declares_transitions
}

/// What publishing a native rule's declarations did, where the engine holds the rule.
#[derive(Clone, Copy)]
pub(crate) struct PublishedRuleDeclarations {
    /// The rule's engine id plus one.
    pub(crate) rule: u32,
    pub(crate) declares_transitions: bool,
}

/// Publishes the declarations the native rule of `identity` now holds, on the render owner, if the engine holds the
/// rule.
pub(crate) fn owner_rule_declarations_changed(
    engine: &mut StyleEngine,
    identity: u64,
    declarations: Option<std::sync::Arc<crate::css::declaration_block::DeclarationBlockData>>,
) -> Option<PublishedRuleDeclarations> {
    let id = engine.native_rules.identities.get(&identity)?;
    let rule = id.0 + 1;
    let target = engine.native_rules.targets.get_mut(&id)?;
    target.declarations = declarations.clone();
    let version = engine.next_declaration_block_version();
    operations::record_rule_declarations_changed(engine, rule, version);
    Some(PublishedRuleDeclarations {
        rule,
        declares_transitions: declarations
            .is_some_and(|declarations| publish_rule_declarations(engine, rule, &declarations)),
    })
}

/// Find the next compiled rule after an inserted native subtree, without creating CSSOM objects.
///
/// # Safety
/// Engine and sheet must be live native allocations.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_native_rule_successor(
    engine: StyleEngineHandle,
    sheet: *const c_void,
    identity: u64,
) -> u32 {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_native_rule_successor",
        crate::css::style::owner_calls::StyleQuery::NativeRuleSuccessor { sheet, identity },
    )
    .u32()
}

/// Answers [`style_engine_native_rule_successor`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_native_rule_successor`].
pub(crate) unsafe fn owner_native_rule_successor(engine: &StyleEngine, sheet: *const c_void, identity: u64) -> u32 {
    let sheet = unsafe { &*sheet.cast::<crate::css::style_sheet::NativeStyleSheet>() };
    crate::css::rule::mutation::successor(sheet, identity, |identity| {
        engine.native_rules.identities.get(&identity).map_or(0, |id| id.0 + 1)
    })
}

/// Retire a native subtree, with host callbacks only for document and cascade-cache notifications.
///
/// # Safety
/// Engine, sheet, rule, callbacks, and any non-null detached import must be live. Native rules must
/// belong to Arc allocations. No graph or engine borrow spans a host callback.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_remove_native_rule(
    engine: StyleEngineInputHandle,
    sheet: *const c_void,
    rule: *const c_void,
    detached_import: *const c_void,
    _sheet_id: u32,
    context: *mut c_void,
    begin: unsafe extern "C" fn(*mut c_void, bool, bool),
    notify: unsafe extern "C" fn(*mut c_void, u32, bool),
) {
    use crate::css::rule::{NativeRule, NativeRuleType, mutation, read::RuleRef};
    use crate::css::style_sheet::NativeStyleSheet;
    let removed = unsafe {
        mutation::removed_rules(
            &*rule.cast::<NativeRule>(),
            &*sheet.cast::<NativeStyleSheet>(),
            detached_import.cast::<NativeStyleSheet>().as_ref(),
        )
    };
    let changes_environment =
        RuleRef::Materialized(unsafe { &*rule.cast::<NativeRule>() }).change_needs_style_environment_bump();
    let has_counter_style = removed
        .iter()
        .any(|rule| RuleRef::Materialized(rule).rule_type() == NativeRuleType::CounterStyle);
    // The owner removes the rules in order, each with what removing the ones before it left, and names the engine id
    // each had then (0 for one removed with an ancestor), which the host is told of. The host's callbacks reach no
    // engine: they note what the document and its scopes derive from the rules, and republish the layer order.
    let identities = removed
        .iter()
        .map(|rule| RuleRef::Materialized(rule).identity())
        .collect::<Vec<_>>();
    let mut ids = vec![0u32; identities.len()];
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_remove_native_rule",
        crate::css::style::owner_calls::StyleQuery::RemoveNativeRules {
            identities: identities.as_ptr(),
            ids: ids.as_mut_ptr(),
            count: identities.len(),
        },
    );
    super::seal::note_host_call("remove_native_rule.begin");
    unsafe { begin(context, changes_environment, has_counter_style) };
    for (rule, id) in removed.iter().zip(ids) {
        super::seal::note_host_call("remove_native_rule.notify");
        unsafe { notify(context, id, mutation::declares_layer(rule)) };
    }
}

/// Read a rule's native cascade data without a CSSOM facade.
///
/// # Safety
/// Engine must be live. On success the caller owns declarations and must release it with
/// rust_declaration_data_release. Layer text is borrowed until the next rule mutation; callers
/// must copy any text needed across such a mutation. Source identity never retains a native sheet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_native_rule_target(
    engine: StyleEngineHandle,
    rule: u32,
    result: *mut FfiNativeRuleTarget,
) -> bool {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_native_rule_target",
        crate::css::style::owner_calls::StyleQuery::NativeRuleTarget { rule, result },
    )
    .is()
}

/// Answers [`style_engine_native_rule_target`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_native_rule_target`].
pub(crate) unsafe fn owner_native_rule_target(
    engine: &StyleEngine,
    rule: u32,
    result: *mut FfiNativeRuleTarget,
) -> bool {
    let Some(target) = rule
        .checked_sub(1)
        .and_then(|id| engine.native_rules.targets.get(&RuleID(id)))
    else {
        return false;
    };
    let Some(declarations) = target.declarations() else {
        return false;
    };
    let origin = engine.program.sheet_origin(engine.program.rule_sheet(RuleID(rule - 1)));
    // SAFETY: The caller vouches that `result` is writable.
    let result = unsafe { &mut *result };
    *result = FfiNativeRuleTarget {
        identity: target.identity.get(),
        declaration_version: engine
            .current_rule_version(RuleID(rule - 1))
            .declaration_block
            .map_or(0, |version| version.0),
        source_identity: target.source_identity,
        declarations: std::sync::Arc::into_raw(declarations.clone()).cast(),
        layer_name: target.layer_name().as_ptr(),
        layer_name_length: target.layer_name().len(),
        has_container_conditions: !target.containers().is_empty(),
        origin: match origin {
            CascadeOrigin::Author => FfiCascadeOrigin::Author,
            CascadeOrigin::AuthorPresentationalHint => FfiCascadeOrigin::AuthorPresentationalHint,
            CascadeOrigin::User => FfiCascadeOrigin::User,
            CascadeOrigin::UserAgent => FfiCascadeOrigin::UserAgent,
            CascadeOrigin::Animation | CascadeOrigin::Transition => {
                unreachable!("stylesheets cannot have an animation origin")
            }
        },
    };
    true
}

/// Whether the engine holds a style pass the host has taken only some waves of.
///
/// # Safety
/// Engine must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_has_suspended_style_pass(engine: StyleEngineHandle) -> bool {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_has_suspended_style_pass",
        crate::css::style::owner_calls::StyleQuery::HasSuspendedStylePass,
    )
    .is()
}

/// Answers [`style_engine_has_suspended_style_pass`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_has_suspended_style_pass`].
pub(crate) unsafe fn owner_has_suspended_style_pass(engine: &StyleEngine) -> bool {
    engine.state.host.suspended_style_pass.is_some()
}

/// What the container conditions of an element's row left for the host to record, in the shape an
/// evaluation answers with. `matches` is unused. The engine has recorded what it reads of them
/// itself by the time this returns.
///
/// # Safety
/// Engine must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_take_container_effects(
    engine: StyleEngineInputHandle,
    node: u32,
) -> FfiNativeContainerMatchResult {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_take_container_effects",
        crate::css::style::owner_calls::StyleQuery::TakeContainerEffects { node },
    )
    .container_effects()
}

/// Answers [`style_engine_take_container_effects`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_take_container_effects`].
pub(crate) unsafe fn owner_take_container_effects(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
) -> FfiNativeContainerMatchResult {
    let Some(verdict) = StyleNodeID::from_raw(node).and_then(|node| engine.take_and_record_container_effects(node))
    else {
        return FfiNativeContainerMatchResult::default();
    };
    let effects = (!verdict.effects.is_empty()).then(|| {
        Box::into_raw(Box::new(ContainerEffects {
            effects: verdict.effects,
        }))
        .cast()
    });
    FfiNativeContainerMatchResult {
        matches: true,
        depends_on_size: verdict.depends_on_size,
        depends_on_style: verdict.depends_on_style,
        effects: effects.unwrap_or(std::ptr::null_mut()),
    }
}

/// Drops what the container conditions of a declined row read of its containers: nothing records
/// it, and the next transaction computes the element again.
///
/// # Safety
/// Engine must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_discard_container_effects(engine: StyleEngineInputHandle, node: u32) {
    crate::css::style::owner_calls::send(
        engine,
        "style_engine_discard_container_effects",
        crate::css::style::owner_calls::EngineChange::DiscardContainerEffects { node },
    );
}

/// Answers [`style_engine_discard_container_effects`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_discard_container_effects`].
pub(crate) unsafe fn owner_discard_container_effects(engine: &mut crate::css::style::StyleEngine, node: u32) {
    if let Some(node) = StyleNodeID::from_raw(node) {
        let _ = engine.take_container_effects_for_host(node);
    }
}

/// Registers the anchor names of the record `style_record` installs on an element in place of the
/// ones it registered before. The names that moved wait for `style_engine_publish_anchor_names`. A
/// zero record registers nothing. Returns bit 0 when the element had names registered, and bit 1
/// when it has now.
///
/// # Safety
/// Engine must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_register_anchor_names(
    engine: StyleEngineInputHandle,
    node: u32,
    style_record: u64,
) -> u8 {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_register_anchor_names",
        crate::css::style::owner_calls::StyleQuery::RegisterAnchorNames { node, style_record },
    )
    .u32()
    .try_into()
    .unwrap_or(0)
}

/// Answers [`style_engine_register_anchor_names`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_register_anchor_names`].
pub(crate) unsafe fn owner_register_anchor_names(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    style_record: u64,
) -> u8 {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return 0;
    };
    let registered = engine.register_anchor_names(node, style_record);
    u8::from(registered.had_names) | (u8::from(registered.has_names) << 1)
}

/// Publishes the anchor names registration moved since the last publication to the document's
/// layout arena; without one, they wait for an arena.
///
/// # Safety
/// Engine must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_publish_anchor_names(engine: StyleEngineInputHandle) {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_publish_anchor_names",
        crate::css::style::owner_calls::StyleQuery::PublishAnchorNames,
    );
}

/// Interns one name identity and returns its document-local atom.
///
/// The caller passes the one-word identity of an interned string it holds a reference to, so the
/// identity cannot be reused while the atom is live.
///
/// # Safety
/// `engine` must be live.
pub unsafe fn replay_intern_atom(engine: StyleEngineHandle, raw: usize) -> u32 {
    let engine = unsafe { engine.for_replay() };
    let result = engine.intern_atom(raw);
    record_interned_atom(engine, raw, result);
    result.0
}

/// Acquires the process-global atom for a name the host holds, for the document to adopt with its
/// next transaction (`FfiHostFactKind::AdoptAtom`). Touches no engine: the table is shared by every
/// document and locked. `recording_stream` is the one the document's engine records under, or zero.
///
/// # Safety
/// `raw` must be the one-word identity of an interned string the caller holds a reference to.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_acquire_host_atom(recording_stream: u64, raw: usize) -> u32 {
    let atom = super::atoms::acquire_raw_for_adoption(raw);
    // A replay interns the name where the host acquired it, which is where the atom was numbered.
    #[cfg(feature = "style-recording")]
    if recording_stream != 0 {
        let token = super::record_replay::atom_pointer_token(raw);
        super::record_replay::record_engine_event(recording_stream, EventKind::InternAtom, |payload| {
            payload.write_u64(token);
            payload.write_u32(atom.0);
        });
    }
    #[cfg(not(feature = "style-recording"))]
    let _ = recording_stream;
    atom.0
}

/// Acquires the process-global atom for the name atom `name` qualified by the namespace atom
/// `namespace`, for the document to adopt with its next transaction
/// (`FfiHostFactKind::AdoptQualifiedAtom`). Touches no engine, as `style_engine_acquire_host_atom`.
#[unsafe(no_mangle)]
pub extern "C" fn style_engine_acquire_host_qualified_atom(recording_stream: u64, namespace: u32, name: u32) -> u32 {
    let atom = super::atoms::acquire_qualified_for_adoption(StyleAtomID(namespace), StyleAtomID(name));
    // A replay interns the qualified name where the host acquired it, which is where the atom was
    // numbered.
    #[cfg(feature = "style-recording")]
    if recording_stream != 0 {
        super::record_replay::record_engine_event(recording_stream, EventKind::InternQualifiedAtom, |payload| {
            payload.write_u32(namespace);
            payload.write_u32(name);
            payload.write_u32(atom.0);
        });
    }
    #[cfg(not(feature = "style-recording"))]
    let _ = recording_stream;
    atom.0
}

/// Gives up an atom `style_engine_acquire_host_qualified_atom` acquired whose adoption never
/// crossed.
#[unsafe(no_mangle)]
pub extern "C" fn style_engine_release_host_qualified_atom(namespace: u32, name: u32, atom: u32) {
    super::atoms::release_qualified_without_adoption(StyleAtomID(namespace), StyleAtomID(name), StyleAtomID(atom));
}

/// Gives up an atom `style_engine_acquire_host_atom` acquired whose adoption never crossed.
///
/// # Safety
/// `raw` and `atom` must be an acquisition that was not adopted.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_release_host_atom(raw: usize, atom: u32) {
    super::atoms::release_raw_without_adoption(raw, StyleAtomID(atom));
}

pub(crate) fn intern_native_text(engine: &mut StyleEngine, units: &[u16]) -> StyleAtomID {
    let name = ak::Utf16FlyString::from_utf16(units);
    intern_native_atom(engine, name.raw_identity())
}

pub(crate) fn intern_native_atom(engine: &mut StyleEngine, raw: usize) -> StyleAtomID {
    let result = engine.atoms.intern_raw(raw);
    record_interned_atom(engine, raw, result);
    result
}

fn record_interned_atom(engine: &mut StyleEngine, raw: usize, atom: StyleAtomID) {
    let token = engine.recording_atom_pointer_token(raw);
    engine.record_boundary_call(EventKind::InternAtom, |payload| {
        payload.write_u64(token.expect("an enabled recorder must tokenize the pointer"));
        payload.write_u32(atom.0);
    });
}

/// Takes the pending style transaction and returns its versioned semantic match answers. The
/// transaction applies `input`, the style input the host recorded since the last one, first, and
/// keeps the `install_feedback` the host's install of the batches before handed back.
///
/// # Safety
/// `engine` must be live, and `layout_arena` the document's live layout arena or null. `input` is
/// null or as for [`style_engine_apply_transaction`]'s `transaction`, its grant arrays kept live
/// until the call returns, and each array of `install_feedback` points at its stated count. `render_half` says whether the render owner applies the batch to the layout nodes
/// itself, its viewport propagation sources pointing at their stated count. The returned answer
/// slice remains valid until the next mutable `style_engine_*` entry point or an explicit discard of
/// the transaction outputs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_take_style_transaction(
    engine: StyleEngineInputHandle,
    root: u32,
    computation_inputs: FfiDocumentStyleComputationInputs,
    layout_arena: *mut c_void,
    input: *const FfiStyleInputTransaction,
    install_feedback: FfiInstallFeedback,
    render_half: FfiOwnerRenderHalf,
) -> FfiStyleTransactionView {
    // SAFETY: Guaranteed by the caller.
    let install_feedback = unsafe { install_feedback.borrow() };
    // The owner takes the whole transaction: it begins it with the inputs the host froze, runs its pass and
    // finishes it, and the host reads the answers it left. This thread only brings the engine home, so
    // that no stage holds the engine meanwhile.
    engine.home().bring_home("style_engine_take_style_transaction");
    super::seal::note_engine_call("style_engine_take_style_transaction");
    let Some(root) = StyleNodeID::from_raw(root) else {
        return FfiStyleTransactionView::default();
    };
    // SAFETY: Guaranteed by the caller.
    let grant = unsafe { input.as_ref() }.map_or_else(StyleNodeGrant::default, StyleNodeGrant::of);
    // SAFETY: As above.
    let input = unsafe { InputForPass::handed_over(input, install_feedback) }.map_or(PassInput::None, |input| {
        PassInput::Here(Box::new(input), engine.through_render_inputs())
    });
    // SAFETY: Guaranteed by the caller.
    let render_half = render_half.applies.then(|| unsafe {
        borrow(
            render_half.viewport_propagation_sources,
            render_half.viewport_propagation_source_count,
        )
        .iter()
        .filter_map(|&node| StyleNodeID::from_raw(node))
        .collect()
    });
    let transaction = OwnerStyleTransaction::Whole {
        root,
        computation_inputs,
        input,
        grant,
        render_half,
    };
    // This thread reaches the engine again only once the owner has finished the transaction.
    let OwnerStyleTransactionView(view, retired, applied) =
        crate::render_owner::run_style_transaction(engine.home().document(), transaction);
    // Font cascade lists and custom-property data are the document thread's to give up.
    crate::css::ffi_stats::release_deferred_font_cascade_lists();
    drop(retired);
    // What the owner handed back of applying the batch is paid, and what its rows marked held for the install,
    // before the host installs the batch, which reads both.
    if let Some(applied) = applied {
        // SAFETY: This is the document thread's FFI entry, and the arena is the one the owner applied the batch to.
        unsafe { applied.hand_to_host(layout_arena) };
    }
    view
}

/// A style transaction the render owner runs with the style engine of the document's render state while the document
/// thread waits for it.
#[expect(clippy::large_enum_variant, reason = "the transaction travels to the owner boxed")]
pub(crate) enum OwnerStyleTransaction {
    /// A transaction the document thread takes, which the owner begins, runs the pass of and finishes.
    Whole {
        root: StyleNodeID,
        /// What the host froze for the transaction. What it points to is the document thread's, which keeps it as it
        /// is while it waits.
        computation_inputs: FfiDocumentStyleComputationInputs,
        /// The style input the host recorded since the last transaction, which the transaction applies first.
        input: PassInput,
        /// Where the engine writes the identities it grants the host with the input.
        grant: StyleNodeGrant,
        /// Whether the owner applies the batch's rows to the layout nodes of their elements itself, with the elements
        /// the viewport propagates from.
        render_half: Option<Vec<StyleNodeID>>,
    },
    /// The transaction whose pass a rendering update of the document ran, which the owner finishes once the document
    /// thread has taken the update's frame back. An atom the host named beside the pass may be one the pass found
    /// unused, where `host_named_atoms_beside_pass`.
    FinishSubmitted { host_named_atoms_beside_pass: bool },
}

/// Whether the render owner applies the batch of a style transaction it takes to the layout nodes of
/// the rows' elements itself: the host asks where the layout tree is one the next layout lays out
/// as it is, and names the elements the viewport propagates its overflow, writing mode and direction
/// from.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiOwnerRenderHalf {
    pub applies: bool,
    pub viewport_propagation_sources: *const u32,
    pub viewport_propagation_source_count: usize,
}

// SAFETY: The document thread waits for the transaction, keeping what its inputs and its grant point to live and
// unchanged, and the owner that runs it holds the arena.
unsafe impl Send for OwnerStyleTransaction {}

/// Where the engine writes the style node identities it grants the host with a style input transaction, and how many
/// the host asks for, in the host's arrays.
pub(crate) struct StyleNodeGrant {
    elements: *mut u32,
    element_count: usize,
    texts: *mut u32,
    text_count: usize,
}

impl Default for StyleNodeGrant {
    fn default() -> Self {
        Self {
            elements: std::ptr::null_mut(),
            element_count: 0,
            texts: std::ptr::null_mut(),
            text_count: 0,
        }
    }
}

impl StyleNodeGrant {
    /// Whether the host asks for no identity.
    fn is_empty(&self) -> bool {
        self.element_count == 0 && self.text_count == 0
    }

    fn of(transaction: &FfiStyleInputTransaction) -> Self {
        Self {
            elements: transaction.element_identity_grant,
            element_count: transaction.element_identity_grant_count,
            texts: transaction.text_identity_grant,
            text_count: transaction.text_identity_grant_count,
        }
    }

    /// Grants the host its identities from `engine`.
    ///
    /// # Safety
    ///
    /// The host's arrays must be live, with nothing else reaching them.
    unsafe fn grant(self, engine: &mut StyleEngine) {
        // SAFETY: Guaranteed by the caller.
        let (elements, texts) = unsafe {
            (
                borrow_mut(self.elements, self.element_count),
                borrow_mut(self.texts, self.text_count),
            )
        };
        grant_style_nodes(engine, elements, texts);
    }
}

/// The answers of an [`OwnerStyleTransaction`], which the owner left in the engine for the document thread to read, the
/// custom-property data it retired, which the document thread releases, and what applying the batch to the layout
/// nodes left, if the owner applied it, which the document thread hands to the host before the host installs the batch.
pub(crate) struct OwnerStyleTransactionView(
    FfiStyleTransactionView,
    RetiredCustomPropertyData,
    Option<crate::layout::OwnerAppliedStyle>,
);

// SAFETY: The view points into the engine, which nothing changes until the document thread's next entrance of it, and
// the retired data goes to the document thread, which alone releases it.
unsafe impl Send for OwnerStyleTransactionView {}

impl OwnerStyleTransactionView {
    /// The view of a transaction no owner ran, which answers nothing.
    pub(crate) fn unanswered() -> Self {
        Self(
            FfiStyleTransactionView::default(),
            RetiredCustomPropertyData { _data: Vec::new() },
            None,
        )
    }
}

impl OwnerStyleTransaction {
    /// Sends the transaction's input to the owner as a change of `document`, which the transaction applies as its
    /// first step on the owner.
    pub(crate) fn send_input_to_owner(&mut self, document: crate::render_owner::DocumentId) {
        if let Self::Whole { input, .. } = self {
            input.send_to_owner(document);
        }
    }

    /// Runs the transaction with the document's engine `engine`, on the render owner, which then
    /// applies the render half of its batch too where the host asked. `state` is the document's
    /// render state, whose committed boxes the pass samples.
    ///
    /// # Safety
    ///
    /// The document thread waits for it, as the type requires.
    pub(crate) unsafe fn run(
        self,
        engine: &mut StyleEngine,
        state: &mut crate::layout::ArenaHandle,
    ) -> OwnerStyleTransactionView {
        let (view, retired, applied) = match self {
            Self::Whole {
                root,
                computation_inputs,
                input,
                grant,
                render_half,
            } => {
                // SAFETY: Guaranteed by the caller.
                unsafe { grant.grant(engine) };
                input.apply(engine);
                // SAFETY: Guaranteed by the caller.
                unsafe { begin_style_transaction(engine, computation_inputs) };
                // SAFETY: The owner holds the render state, and the pass is done with the boxes when it returns.
                let committed_boxes = unsafe {
                    super::animations::CommittedTransformReferenceBoxes::lend(std::ptr::from_mut(state).cast())
                };
                // The pass samples at the times the host published for this update.
                let timeline_samples = engine.animation_timeline_samples().clone();
                let output = run_style_pass(engine, root, committed_boxes, &timeline_samples);
                let (mut view, retired) = finish_style_transaction(engine, root, output);
                let mut applied = None;
                if let Some(viewport_propagation_sources) = render_half
                    && let Some(effects) =
                        apply_render_half_on_owner(engine, state.arena_mut(), &viewport_propagation_sources)
                {
                    view.render_half_applied = true;
                    view.render_half_moved_visual_contexts = effects.moved_visual_contexts;
                    view.render_half_repaint = effects.repaint;
                    applied = Some(effects.applied);
                }
                (view, retired, applied)
            }
            Self::FinishSubmitted {
                host_named_atoms_beside_pass,
            } => {
                let (view, retired) = finish_submitted_style_transaction(engine, host_named_atoms_beside_pass);
                (view, retired, None)
            }
        };
        OwnerStyleTransactionView(view, retired, applied)
    }
}

/// Applies the batch a style transaction the owner took left to the layout nodes of the rows'
/// elements, as a flight applies its pass's: each row's record, and what the row's move marks of
/// layout, paint and the visual contexts, which the host's install then leaves alone. Answers what
/// the rows' moves ask of the document if it applied the batch; a batch any row of which the host
/// styles in a way of its own is the host's to apply whole.
fn apply_render_half_on_owner(
    engine: &StyleEngine,
    arena: &mut crate::layout::LayoutNodeArena,
    viewport_propagation_sources: &[StyleNodeID],
) -> Option<OwnerRenderHalfEffects> {
    let rows = engine.rows_the_owner_applies(viewport_propagation_sources).ok()?;
    if rows.is_empty() {
        return None;
    }
    arena.apply_flight_style_rows(&rows).ok()?;
    // No flight reads whether one applied a batch: the host's render half ends with the update.
    arena.take_flight_style_applied();
    let mut effects = OwnerRenderHalfEffects {
        moved_visual_contexts: false,
        repaint: 0,
        applied: crate::layout::OwnerAppliedStyle::take_from(arena),
    };
    for row in &rows {
        let marks = super::style_invalidation::layout_node_marks(row.damage);
        effects.moved_visual_contexts |= marks.visual_context != 0 || marks.stacking_context;
        if marks.repaint {
            effects.repaint = effects.repaint.max(if marks.repaint_hit_test { 2 } else { 1 });
        }
    }
    Some(effects)
}

/// What the rows the owner applied of a batch ask of the document, which the host applies once it
/// has the transaction back: the owner marked the layout nodes, but the document's navigable is the
/// host's to have painted again, and what applying the rows handed back and marked is the host's to
/// pay and read as it installs the batch.
struct OwnerRenderHalfEffects {
    moved_visual_contexts: bool,
    /// As [`FfiStyleTransactionView::render_half_repaint`].
    repaint: u8,
    applied: crate::layout::OwnerAppliedStyle,
}

/// Takes the pending style transaction as [`style_engine_take_style_transaction`] does, and hands
/// its pass to the stage thread instead of waiting for it: the pass runs beside the main thread
/// with the engine, which it sends home once it has run. It does not own the layout arena
/// `layout_arena`, which it never reaches.
/// [`style_engine_finish_submitted_style_transaction`] then returns its answers. The host hands
/// over the input it recorded since the last transaction as `input` (or null for none), which the
/// pass applies as its first step; the grant `input` asks for answers the host at once.
///
/// # Safety
/// `engine` must be live and `root` a styled node's raw ID; `layout_arena` must be the document's
/// live layout arena. `input` must be null or a transaction as for
/// [`style_engine_apply_transaction`], applied to no engine yet.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_submit_style_transaction(
    engine: StyleEngineInputHandle,
    root: u32,
    computation_inputs: FfiDocumentStyleComputationInputs,
    layout_arena: *mut c_void,
    input: *const FfiStyleInputTransaction,
    install_feedback: FfiInstallFeedback,
) {
    // SAFETY: Guaranteed by the caller.
    let install_feedback = unsafe { install_feedback.borrow() };
    // SAFETY: Guaranteed by the caller.
    let mut pass =
        unsafe { prepare_style_pass(engine, root, computation_inputs, layout_arena, input, install_feedback) };
    let engine = engine.home();
    // A layout frame that runs its first round's style in its flight takes the pass along instead.
    let Some(mut pass) = STYLE_PASS_FOR_FLIGHT.with(|collected| match collected.borrow_mut().as_mut() {
        Some(slot) => {
            // The frame goes on to read the engine before its flight runs the pass, and each of its reads comes after
            // the input, which the owner applies first.
            // SAFETY: As above.
            unsafe { pass.send_input_to_owner(layout_arena) };
            *slot = Some(pass);
            None
        }
        None => Some(pass),
    }) else {
        return;
    };
    // SAFETY: As above.
    unsafe { pass.send_input_to_owner(layout_arena) };
    if crate::stage_thread::submits_flight() {
        // SAFETY: As above.
        unsafe { crate::flight::submit(layout_arena, crate::flight::Flight::from_style_pass(layout_arena, pass)) };
        return;
    }
    // The pass takes the engine along, and sends it home once it has run.
    let (loan, settlement) = engine.lend(Holder::StylePass, Owed::TakeBack);
    // SAFETY: As above.
    unsafe {
        crate::stage_thread::submit_stage_with_take_back(
            "style",
            layout_arena,
            move || {
                let mut loan = loan;
                pass.run(&mut loan);
            },
            move || settlement.settle(),
        );
    }
}

thread_local! {
    // On the main thread, from the point the style pass a layout frame's flight is to run is
    // submitted for it until the frame's first round takes it: the pass, once submitted.
    static STYLE_PASS_FOR_FLIGHT: std::cell::RefCell<Option<Option<StylePassJob>>> =
        const { std::cell::RefCell::new(None) };
}

/// Collects the style pass the main thread submits next, unless the document has no style to
/// update, for a layout frame's flight to run.
pub(crate) fn collect_next_style_pass_for_flight() {
    STYLE_PASS_FOR_FLIGHT.with(|collected| {
        let previous = collected.borrow_mut().replace(None);
        debug_assert!(previous.is_none(), "one style pass is collected at a time");
    });
}

/// The style pass collected for a layout frame's flight, if the main thread submitted one.
pub(crate) fn take_style_pass_collected_for_flight() -> Option<StylePassJob> {
    STYLE_PASS_FOR_FLIGHT.with(|collected| collected.borrow_mut().take().flatten())
}

/// A style pass the main thread has prepared to run beside it, with what it takes along from the
/// main thread. The stage that runs it holds its engine's loan. Running it leaves its output in
/// the engine, for [`style_engine_finish_submitted_style_transaction`].
pub(crate) struct StylePassJob {
    root: StyleNodeID,
    snapshot: super::animations::CommittedTransformReferenceBoxSnapshot,
    timeline_samples: super::animations::AnimationTimelineSamples,
    /// The input the host recorded since the last transaction, which the pass applies first.
    input: PassInput,
}

/// Where the input a style pass applies first is.
pub(crate) enum PassInput {
    None,
    /// With the pass, until it is sent to the owner or applied on the document thread. The document thread recorded it
    /// through its render inputs.
    Here(Box<InputForPass>, crate::css::style::ThroughRenderInputs),
    /// Sent to the owner as a change of `document`: the pass applies that document's changes through `through`.
    Sent {
        document: crate::render_owner::DocumentId,
        through: crate::render_owner::ChangeSeq,
    },
}

impl PassInput {
    /// Sends the input to the owner of `document`'s render state as a change, if it is still here.
    fn send_to_owner(&mut self, document: crate::render_owner::DocumentId) {
        if !document.is_valid() {
            return;
        }
        let Self::Here(input, through_render_inputs) = std::mem::replace(self, Self::None) else {
            return;
        };
        let through = crate::render_owner::send_change(
            through_render_inputs,
            document,
            crate::render_owner::Change::StyleInputs(*input),
        );
        *self = Self::Sent { document, through };
    }

    /// Applies the input to `engine`, where it is: on the owner, a sent input with the changes before it.
    fn apply(self, engine: &mut StyleEngine) {
        match self {
            Self::None => {}
            Self::Here(input, _) => input.apply(engine),
            Self::Sent { document, through } => crate::render_owner::apply_changes_through(
                document,
                through,
                &mut crate::render_owner::ChangeTarget { style_engine: engine },
            ),
        }
    }
}

impl StylePassJob {
    /// Sends the pass's input to the owner of `layout_arena`'s render state as a change, which the pass applies as its
    /// first step on the owner.
    ///
    /// # Safety
    ///
    /// `layout_arena` must be the live arena of the pass's document.
    unsafe fn send_input_to_owner(&mut self, layout_arena: *const c_void) {
        // SAFETY: Guaranteed by the caller.
        let document = unsafe { crate::layout::ArenaHandle::document_of(layout_arena) };
        self.input.send_to_owner(document);
    }

    /// Runs the pass, on the stage the engine is lent to as `loan`.
    pub(crate) fn run(self, loan: &mut StyleEngineLoan) {
        let Self {
            root,
            snapshot,
            timeline_samples,
            input,
        } = self;
        loan.lend_to_this_thread(|engine| Self::run_in(engine, root, &snapshot, &timeline_samples, input));
    }

    fn run_in(
        engine: &mut StyleEngine,
        root: StyleNodeID,
        snapshot: &super::animations::CommittedTransformReferenceBoxSnapshot,
        timeline_samples: &super::animations::AnimationTimelineSamples,
        input: PassInput,
    ) {
        input.apply(engine);
        // SAFETY: The pass owns the snapshot for as long as it runs.
        let committed_boxes = unsafe { super::animations::CommittedTransformReferenceBoxes::taken_along(snapshot) };
        let mut output = run_style_pass(engine, root, committed_boxes, timeline_samples);
        // What the finish reads of the engine alone, it reads here, off the main thread: nothing
        // reaches the engine between the pass and the finish but the pass's own atom sweep, which
        // an active cold matching batch would put off, so that batch waits for the finish.
        settle_style_row_facts(engine, &mut output);
        if !engine.host.atom_sweep_skipped_by_submitted_pass {
            begin_update_cold_matching_batch(engine, root, &output);
        }
        engine.host.submitted_style_pass_output = Some((root, Box::new(output)));
    }
}

/// Takes the pending style transaction as [`style_engine_submit_style_transaction`] does, up to the
/// point where its pass would be submitted, and returns that pass.
///
/// # Safety
/// As for [`style_engine_submit_style_transaction`]: the pass returned has to run on a stage that
/// holds the engine's loan.
pub(crate) unsafe fn prepare_style_pass(
    engine: StyleEngineInputHandle,
    root: u32,
    computation_inputs: FfiDocumentStyleComputationInputs,
    layout_arena: *mut c_void,
    input: *const FfiStyleInputTransaction,
    install_feedback: InstallFeedback<'_>,
) -> StylePassJob {
    let through_render_inputs = engine.through_render_inputs();
    debug_assert!(
        !layout_arena.is_null(),
        "a submitted style pass is submitted for its document's layout arena"
    );
    let root = StyleNodeID::from_raw(root).expect("a submitted style pass has a root");
    // SAFETY: Guaranteed by the caller.
    let grant = unsafe { input.as_ref() }.map_or_else(StyleNodeGrant::default, StyleNodeGrant::of);
    // The owner grants the identities, begins the transaction with the inputs the host froze, and takes what the pass
    // takes along, while this thread waits with them.
    let prepared = crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_take_style_transaction",
        crate::css::style::owner_calls::StyleQuery::PrepareStylePass {
            computation_inputs,
            layout_arena,
            grant,
        },
    )
    .prepared_style_pass();
    // SAFETY: Guaranteed by the caller.
    let input = unsafe { InputForPass::handed_over(input, install_feedback) };
    let (snapshot, timeline_samples) = *prepared;
    StylePassJob {
        root,
        snapshot,
        timeline_samples,
        input: input.map_or(PassInput::None, |input| {
            PassInput::Here(Box::new(input), through_render_inputs)
        }),
    }
}

/// What a submitted style pass takes along from its preparation on the render owner: the committed boxes of the nodes
/// it may sample, and the times it samples at.
pub(crate) type PreparedStylePass = Box<(
    super::animations::CommittedTransformReferenceBoxSnapshot,
    super::animations::AnimationTimelineSamples,
)>;

/// Prepares the style pass [`prepare_style_pass`] submits with `engine`, on the render owner.
///
/// # Safety
///
/// As for [`prepare_style_pass`], with the host's inputs and grant arrays live while the document thread waits.
pub(crate) unsafe fn owner_prepare_style_pass(
    engine: &mut StyleEngine,
    computation_inputs: FfiDocumentStyleComputationInputs,
    layout_arena: *mut c_void,
    grant: StyleNodeGrant,
) -> PreparedStylePass {
    debug_assert!(
        engine.host.submitted_style_pass_output.is_none(),
        "one style pass is in flight at a time"
    );
    // A pass left unfinished answers nobody: the one submitted now replaces it.
    engine.host.submitted_style_pass_output = None;
    // SAFETY: Guaranteed by the caller.
    unsafe { grant.grant(engine) };
    engine.computed_group_sets.begin_pass_beside_host_pins();
    // SAFETY: Guaranteed by the caller.
    unsafe { begin_style_transaction(engine, computation_inputs) };
    engine.host.atom_sweep_waits_for_host = true;
    // The pass never reaches the arena, which the main thread goes on writing beside it: it takes
    // along the committed boxes of the nodes it may sample against them.
    // SAFETY: The owner holds the arena, and the document thread waits.
    let snapshot = unsafe {
        super::animations::CommittedTransformReferenceBoxSnapshot::take(layout_arena, engine.state.animated_nodes())
    };
    // It takes along the times the host published for this update as well, which it samples at.
    let timeline_samples = engine.animation_timeline_samples().clone();
    Box::new((snapshot, timeline_samples))
}

/// The answers of the style pass [`style_engine_submit_style_transaction`] submitted, once the
/// main thread has taken its frame back. The pass left its atom sweep to this call, which runs it
/// unless `host_named_atoms_beside_pass`: an atom the host named beside the pass may be one the
/// pass found unused.
///
/// The render owner finishes the transaction with the engine of the document whose layout arena is
/// `layout_arena`, as it runs the transactions the main thread takes.
///
/// # Safety
/// `engine` must be live, with no frame in flight that owns it, and `layout_arena` its document's
/// live layout arena. The answer slice stays valid as for [`style_engine_take_style_transaction`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_finish_submitted_style_transaction(
    engine: StyleEngineInputHandle,
    host_named_atoms_beside_pass: bool,
    layout_arena: *mut c_void,
) -> FfiStyleTransactionView {
    debug_assert!(
        !layout_arena.is_null(),
        "a submitted style pass is submitted for its document's layout arena"
    );
    engine
        .home()
        .bring_home("style_engine_finish_submitted_style_transaction");
    super::seal::note_engine_call("style_engine_finish_submitted_style_transaction");
    let transaction = OwnerStyleTransaction::FinishSubmitted {
        host_named_atoms_beside_pass,
    };
    // This thread reaches the engine again only once the owner has finished the transaction.
    let OwnerStyleTransactionView(view, retired, applied) =
        crate::render_owner::run_style_transaction(engine.home().document(), transaction);
    debug_assert!(
        applied.is_none(),
        "finishing a submitted transaction applies no batch on the owner"
    );
    // Font cascade lists and custom-property data are the document thread's to give up.
    crate::css::ffi_stats::release_deferred_font_cascade_lists();
    drop(retired);
    view
}

/// Finishes the transaction whose pass a rendering update ran, as
/// [`style_engine_finish_submitted_style_transaction`] describes.
fn finish_submitted_style_transaction(
    engine: &mut StyleEngine,
    host_named_atoms_beside_pass: bool,
) -> (FfiStyleTransactionView, RetiredCustomPropertyData) {
    engine.settle_atom_sweep_of_submitted_pass(host_named_atoms_beside_pass);
    let (root, output) = engine
        .host
        .submitted_style_pass_output
        .take()
        .expect("the submitted style pass has run");
    engine.computed_group_sets.finish_pass_beside_host_pins();
    finish_style_transaction(engine, root, *output)
}

/// Freezes a style transaction's inputs in the engine before its pass runs.
///
/// # Safety
/// As for [`style_engine_take_style_transaction`]'s `computation_inputs`.
unsafe fn begin_style_transaction(engine: &mut StyleEngine, mut computation_inputs: FfiDocumentStyleComputationInputs) {
    let resource_contexts =
        unsafe { super::resource_contexts::DocumentResourceContexts::take_from(&mut computation_inputs) };
    engine.document_media_snapshot =
        unsafe { super::custom_property_cascade::DocumentMediaSnapshot::take_from(&mut computation_inputs) };
    engine.document_function_snapshot =
        unsafe { super::custom_property_cascade::DocumentFunctionSnapshot::take_from(&mut computation_inputs) };
    let resource_contexts_moved = engine.document_resource_contexts.moved_for_records(&resource_contexts);
    engine.document_resource_contexts = resource_contexts;
    engine.custom_property_registrations_changed = engine
        .document_style_computation_inputs
        .custom_property_registration_generation
        != computation_inputs.custom_property_registration_generation;
    if engine.custom_property_registrations_changed {
        engine.custom_property_environments.forget_substitutions();
    }
    // Freeze the registry at every transaction boundary. The registration generation is a
    // semantic invalidation key, while a stylesheet rebuild can replace the native registry
    // contents before that generation becomes the retained transaction's current input.
    engine.custom_property_registry = std::sync::Arc::new(
        unsafe {
            computation_inputs
                .custom_property_registry
                .as_pointer()
                .cast::<crate::css::custom_properties::CustomPropertyRegistry>()
                .as_ref()
        }
        .map_or_else(
            crate::css::custom_properties::CustomPropertyRegistry::empty,
            Clone::clone,
        ),
    );
    // Inputs that name no registry are those of a document that registers nothing: they name the
    // empty one the engine froze instead.
    if computation_inputs.custom_property_registry.is_none() {
        computation_inputs.custom_property_registry =
            FfiHostHandle::from_pointer(std::sync::Arc::as_ptr(&engine.custom_property_registry).cast());
    }
    if engine.document_style_computation_inputs != computation_inputs || resource_contexts_moved {
        // Persistent records are derived from every document computation input, not only the
        // font generation carried in their keys.
        engine.engine_cold_record_cache.clear();
        engine.engine_cold_record_donors.clear();
        engine.engine_warm_record_cohorts.clear();
        engine.engine_pseudo_record_cache.clear();
    }
    engine.document_style_computation_inputs = computation_inputs;
    engine.clear_ffi_style_transaction_output();
}

/// A style transaction's pass: the part of the take that runs as a stage.
fn run_style_pass(
    engine: &mut StyleEngine,
    root: StyleNodeID,
    committed_boxes: super::animations::CommittedTransformReferenceBoxes,
    timeline_samples: &super::animations::AnimationTimelineSamples,
) -> FfiStyleTransactionOutput {
    let mut output = FfiStyleTransactionOutput::default();
    let emitted = &mut output;
    let scoped = engine.take_style_transaction_with_committed_boxes(
        root,
        committed_boxes,
        timeline_samples,
        |transaction_version, program_version, answers| {
            assert!(
                emitted.answers.is_empty(),
                "a style transaction emitted more than one batch"
            );
            emitted.transaction_version = transaction_version.0;
            emitted.program_version = program_version.0;
            emitted.answers.extend_from_slice(answers);
        },
    );
    output.scoped = scoped;
    output
}

/// The node of an element row that carries its node's facts.
fn element_row_node(answer: &FfiStyleDelta) -> Option<StyleNodeID> {
    if answer.pseudo_kind != u8::MAX
        || matches!(
            answer.gap,
            FfiStyleDeltaGap::SkippedHidden
                | FfiStyleDeltaGap::EnvironmentMoved
                | FfiStyleDeltaGap::PseudoElementsSettled
        )
    {
        return None;
    }
    StyleNodeID::from_raw(answer.style_node)
}

/// Hands each element row of a pass's output the facts its node holds now, once.
fn settle_style_row_facts(engine: &StyleEngine, output: &mut FfiStyleTransactionOutput) {
    if std::mem::replace(&mut output.row_facts_settled, true) {
        return;
    }
    for answer in &mut output.answers {
        if let Some(node) = element_row_node(answer) {
            answer.row_facts = engine.style_row_facts(node);
        }
    }
}

/// Begins the style update's cold matching batch with the first of its transactions whose output
/// publishes rows.
fn begin_update_cold_matching_batch(engine: &mut StyleEngine, root: StyleNodeID, output: &FfiStyleTransactionOutput) {
    // The rows of a style update are matched in one cold matching batch, begun with the first of
    // its transactions that publishes rows and ended as the update discards its outputs. A batch
    // covering more than one sixteenth of the connected elements is dense enough that packing the
    // scope once is cheaper than repeatedly reconstructing cold facts while matching its rows. The
    // rows the pass joined for reactions its rows derived, and the records an environment move
    // republished, were matched in the pass or need no matching.
    if !output.answers.is_empty() && engine.host.update_cold_matching_batch.is_none() {
        let planned_rows = output
            .answers
            .iter()
            .filter(|answer| {
                !matches!(
                    answer.gap,
                    FfiStyleDeltaGap::EnvironmentMoved | FfiStyleDeltaGap::PseudoElementsSettled
                ) && answer.record_damage & FfiStyleInvalidationField::JoinedByDerivation as u32 == 0
            })
            .count();
        let broad = !output.scoped || planned_rows * 16 > engine.connected_element_count() as usize;
        let has_traversal = if broad {
            engine.begin_cold_matching_batch(root)
        } else {
            engine.begin_adaptive_cold_matching_batch(root);
            true
        };
        engine.host.update_cold_matching_batch = Some(has_traversal);
    }
}

/// What the main thread does with a style pass's output once the pass has run: hands each row the
/// facts and debts of its node, and publishes the batch.
fn finish_style_transaction(
    engine: &mut StyleEngine,
    root: StyleNodeID,
    mut output: FfiStyleTransactionOutput,
) -> (FfiStyleTransactionView, RetiredCustomPropertyData) {
    // What each element row's node holds now travels with the row, and a computed row takes the
    // debts its computation left: the host settles them as it installs the row, or hands them back.
    settle_style_row_facts(engine, &mut output);
    for answer in &mut output.answers {
        let Some(node) = element_row_node(answer) else {
            continue;
        };
        if matches!(
            answer.gap,
            FfiStyleDeltaGap::Computed
                | FfiStyleDeltaGap::RetriedAfterAncestors
                | FfiStyleDeltaGap::RetriedMaterialization
        ) {
            answer.explicit_inheritance_debt = engine.take_explicit_inheritance_debt(node);
            answer.row_effect_debt = u32::from(engine.take_settled_row_effect_debt(node));
        }
    }
    // Font cascade lists the transaction's font resolutions gave up on the stage thread.
    crate::css::ffi_stats::release_deferred_font_cascade_lists();
    output.reclaimed_style_atoms = std::mem::take(&mut engine.host.reclaimed_style_atoms)
        .into_iter()
        .map(|reclaimed| FfiReclaimedStyleAtom {
            raw: reclaimed.raw,
            atom: reclaimed.atom.0,
        })
        .collect();
    output.style_atoms_swept = std::mem::take(&mut engine.host.style_atoms_swept);
    output.only_derived_child_reactions = engine.take_only_derived_child_reactions();
    begin_update_cold_matching_batch(engine, root, &output);
    close_style_deltas_over_inheritance(engine, &mut output.answers);
    sort_style_deltas_for_direct_application(engine, &mut output.answers);
    if engine.recording_id().is_some() {
        let computation_inputs = engine.document_style_computation_inputs;
        engine.record_boundary_call(EventKind::StyleDeltaBatch, |payload| {
            payload.write_u32(root.raw());
            payload.write_u64(computation_inputs.viewport_width.to_bits());
            payload.write_u64(computation_inputs.viewport_height.to_bits());
            payload.write_u64(computation_inputs.root_font_size.to_bits());
            payload.write_u64(computation_inputs.root_font_x_height.to_bits());
            payload.write_u64(computation_inputs.root_font_cap_height.to_bits());
            payload.write_u64(computation_inputs.root_font_zero_advance.to_bits());
            payload.write_u64(computation_inputs.root_line_height.to_bits());
            payload.write_bool(computation_inputs.root_font_metrics_depend_on_viewport_metrics);
            payload.write_u64(computation_inputs.initial_font_size.to_bits());
            payload.write_u64(computation_inputs.initial_font_x_height.to_bits());
            payload.write_u64(computation_inputs.initial_font_cap_height.to_bits());
            payload.write_u64(computation_inputs.initial_font_zero_advance.to_bits());
            payload.write_i32(computation_inputs.initial_font_size_raw);
            payload.write_i32(computation_inputs.default_font_size_raw);
            payload.write_u64(computation_inputs.device_pixels_per_css_pixel.to_bits());
            payload.write_u64(computation_inputs.font_environment_generation);
            payload.write_u8(computation_inputs.preferred_color_scheme);
            payload.write_bool(computation_inputs.has_document_supported_schemes);
            payload.write_u8(computation_inputs.document_supported_scheme_count);
            for code in computation_inputs.document_supported_scheme_codes {
                payload.write_u8(code);
            }
            let custom_property_registry_is_engine_usable = !unsafe {
                &*computation_inputs
                    .custom_property_registry
                    .as_pointer()
                    .cast::<CustomPropertyRegistry>()
            }
            .has_registrations();
            payload.write_bool(custom_property_registry_is_engine_usable);
            payload.write_u64(computation_inputs.custom_property_registration_generation);
            payload.write_bool(computation_inputs.in_quirks_mode);
            let mut outputs = super::record_replay::PayloadWriter::default();
            write_style_transaction_outputs(&output, &mut outputs);
            payload.write_bytes(outputs.as_bytes());
            payload.write_u64(outputs.stable_digest());
        });
        engine.forget_recording_atom_mappings(output.reclaimed_style_atoms.iter().map(|reclaimed| reclaimed.atom));
    }
    engine.install_ffi_style_transaction_output(output);
    let output = &engine.host.ffi_style_transaction_output;
    let view = FfiStyleTransactionView {
        transaction_version: output.transaction_version,
        program_version: output.program_version,
        answers: output.answers.as_ptr(),
        count: output.answers.len(),
        reclaimed_style_atoms: output.reclaimed_style_atoms.as_ptr(),
        reclaimed_style_atom_count: output.reclaimed_style_atoms.len(),
        scoped: output.scoped,
        only_derived_child_reactions: output.only_derived_child_reactions,
        connected_element_count: engine.connected_element_count(),
        style_atoms_swept: output.style_atoms_swept,
        render_half_applied: false,
        render_half_moved_visual_contexts: false,
        render_half_repaint: 0,
    };
    // The custom-property data the transaction retired is the document thread's to release, once the transaction is
    // over: the caller drops it there. It is taken last, so that a panic in the steps before leaves it with the engine
    // rather than dropping it on whichever thread runs the transaction.
    let retired_custom_property_data = std::mem::take(&mut engine.host.retired_custom_property_data);
    (
        view,
        RetiredCustomPropertyData {
            _data: retired_custom_property_data,
        },
    )
}

/// The custom-property data a style transaction retired, which only the document thread releases.
/// Dropping it releases the data.
pub(crate) struct RetiredCustomPropertyData {
    _data: Vec<super::inputs::RetainedCustomPropertyData>,
}

/// Orders a completed reaction batch for direct application in C++: each inheritance branch
/// contiguously in preorder. Besides making every parent ready before its descendants, this lets a
/// parent's derived reaction merge into an unconsumed child reaction in the same batch.
fn sort_style_deltas_for_direct_application(engine: &StyleEngine, deltas: &mut [FfiStyleDelta]) {
    // An element's own delta leads the pseudo-element deltas settled beside it, and the record an
    // environment move republished for it leads both: the element's own row was computed over it.
    let pseudo_rank = |delta: &FfiStyleDelta| {
        if delta.gap == FfiStyleDeltaGap::EnvironmentMoved {
            0
        } else if delta.pseudo_kind == u8::MAX {
            1
        } else {
            2 + u16::from(delta.pseudo_kind)
        }
    };
    // Small batches cost less to compare directly. A large batch names its dependency
    // order once instead of walking both ancestor chains in every sort comparison.
    if deltas.len() > 32 {
        let ranks = engine.tree.style_reaction_order_ranks(
            deltas
                .iter()
                .map(|delta| StyleNodeID::from_raw(delta.style_node).expect("a style delta must name an element")),
        );
        deltas.sort_unstable_by_key(|delta| {
            let node = StyleNodeID::from_raw(delta.style_node).unwrap();
            (ranks[&node], pseudo_rank(delta))
        });
        return;
    }
    deltas.sort_unstable_by(|first, second| {
        let first_node = StyleNodeID::from_raw(first.style_node).expect("a style delta must name an element");
        let second_node = StyleNodeID::from_raw(second.style_node).expect("a style delta must name an element");
        engine
            .tree
            .compare_style_reaction_order(first_node, second_node)
            .then_with(|| pseudo_rank(first).cmp(&pseudo_rank(second)))
    });
}

/// A row the batch joins for an element its rows need styled before them, with the record the
/// engine settled for it.
fn inheritance_prerequisite_delta(
    engine: &StyleEngine,
    node: StyleNodeID,
    style_record: u64,
    pseudo_kind: u8,
) -> FfiStyleDelta {
    let is_element = pseudo_kind == u8::MAX;
    FfiStyleDelta {
        style_node: node.raw(),
        match_answer: 0,
        old_style_record: 0,
        new_style_record: style_record,
        damage: FfiStyleDeltaDamage::Full,
        reaction: super::transaction::STYLE_REACTION_RECOMPUTE_STYLE,
        inherited_style_groups: 0,
        pseudo_kind,
        gap: FfiStyleDeltaGap::Computed,
        uses_substitution: is_element && engine.nodes_with_substituted_records.contains(&node),
        record_damage: FfiStyleInvalidationField::JoinedForInheritance as u32,
        row_facts: if is_element { engine.style_row_facts(node) } else { 0 },
        explicit_inheritance_debt: 0,
        row_effect_debt: 0,
    }
}

/// Closes a published batch over the elements its rows inherit from and the host holds no style
/// for.
fn close_style_deltas_over_inheritance(engine: &mut StyleEngine, deltas: &mut Vec<FfiStyleDelta>) {
    let mut rows: HashSet<StyleNodeID> = deltas
        .iter()
        .filter_map(|delta| StyleNodeID::from_raw(delta.style_node))
        .collect();
    let mut closure = Vec::new();

    // A reaction can name an element created by editing after its new inheritance parent was
    // inserted. Close the batch over unstyled inheritance prerequisites, which are bounded by
    // the reaction paths rather than discovered by a document traversal.
    for delta in deltas.iter() {
        let node = StyleNodeID::from_raw(delta.style_node).expect("a style delta must name an element");
        let mut ancestor = engine.tree.inheritance_parent(node);
        while let Some(prerequisite) = ancestor {
            if engine.host.held_style_records.contains_key(&prerequisite) || !rows.insert(prerequisite) {
                break;
            }
            closure.push(prerequisite);
            ancestor = engine.tree.inheritance_parent(prerequisite);
        }
    }
    if closure.is_empty() {
        return;
    }
    engine
        .complete_published_match_answers_for_closure(&closure)
        .expect("the rows a batch joins for inheritance have complete match answers");

    // Each joined element settles before the host installs the batch, as a row the engine computed,
    // over the ones it inherits from before it.
    closure.sort_unstable_by(|first, second| engine.tree.compare_style_reaction_order(*first, *second));
    let settled = closure
        .into_iter()
        .filter_map(|node| {
            let super::publication::RecordDemandAnswer::Record(answer) =
                engine.answer_record_demand(node, None, false, true, false, 0)
            else {
                debug_assert!(false, "an element's record demand is always answered");
                // The batch goes without the row, as it would had the host held its style.
                return None;
            };
            Some((node, answer))
        })
        .collect::<Vec<_>>();
    for (node, answer) in settled {
        deltas.push(inheritance_prerequisite_delta(
            engine,
            node,
            answer.style_record,
            u8::MAX,
        ));
        for kind in 0..RETRY_PSEUDO_RECORD_SLOTS {
            if answer.pseudo_records_present & (1 << kind) != 0 {
                deltas.push(inheritance_prerequisite_delta(
                    engine,
                    node,
                    answer.pseudo_records[kind],
                    kind as u8,
                ));
            }
        }
    }
}

/// Keeps the custom-property environment an element now holds. A null `data` records that the
/// element holds none. An environment the element's animations sampled custom properties into
/// names the environment it was composed over, which its store does not: `animation_base`,
/// `animation_base_store` and `animation_base_environment`, null and zero for an overlay composed
/// over none. Every other environment passes `is_animation_overlay` false.
///
/// # Safety
/// `engine` must be live, and `data` must be null or a live `Web::CSS::CustomPropertyData`
/// carrying `store`, and holding the base environment an overlay names alive.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_set_element_custom_property_data(
    engine: StyleEngineInputHandle,
    node: u32,
    data: *const c_void,
    store: *const c_void,
    environment: u64,
    is_animation_overlay: bool,
    declares: bool,
    animation_base: *const c_void,
    animation_base_store: *const c_void,
    animation_base_environment: u64,
) {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_set_element_custom_property_data",
        crate::css::style::owner_calls::StyleQuery::SetElementCustomPropertyData {
            node,
            data,
            store,
            environment,
            is_animation_overlay,
            declares,
            animation_base,
            animation_base_store,
            animation_base_environment,
        },
    );
}

/// Answers [`style_engine_set_element_custom_property_data`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_set_element_custom_property_data`].
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn owner_set_element_custom_property_data(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    data: *const c_void,
    store: *const c_void,
    environment: u64,
    is_animation_overlay: bool,
    declares: bool,
    animation_base: *const c_void,
    animation_base_store: *const c_void,
    animation_base_environment: u64,
) {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return;
    };
    let animation_base =
        is_animation_overlay.then_some((animation_base_environment, animation_base_store, animation_base));
    // What the element held before is the document thread's to release.
    let retired =
        unsafe { engine.set_element_custom_property_data(node, data, store, environment, declares, animation_base) };
    engine
        .host
        .retired_custom_property_data
        .extend(retired.and_then(|held| held.data));
}

/// The custom-property environment an element holds: the host's object for it, or null with the
/// identity of an environment the engine resolved in `identity`, or null and zero for none.
///
/// # Safety
/// `engine` must be live and `identity` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_element_custom_property_data(
    engine: StyleEngineHandle,
    node: u32,
    identity: *mut u64,
) -> *const c_void {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_element_custom_property_data",
        crate::css::style::owner_calls::StyleQuery::ElementCustomPropertyData { node, identity },
    )
    .pointer()
}

/// Answers [`style_engine_element_custom_property_data`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_element_custom_property_data`].
pub(crate) unsafe fn owner_element_custom_property_data(
    engine: &StyleEngine,
    node: u32,
    identity: *mut u64,
) -> *const c_void {
    let (data, environment) =
        StyleNodeID::from_raw(node).map_or((std::ptr::null(), 0), |node| engine.element_custom_property_data(node));
    unsafe { *identity = environment };
    data
}

/// Keeps the custom-property environment one of an element's synthetic pseudo-elements now holds. A
/// null `data` records that it holds none. An overlay names what it was composed over as an
/// element's does, and `declares_own` is whether what the pseudo-element's style resolved to is not
/// simply the environment its originating element passes on.
///
/// # Safety
/// As `style_engine_set_element_custom_property_data`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_set_pseudo_element_custom_property_data(
    engine: StyleEngineInputHandle,
    node: u32,
    pseudo: u8,
    data: *const c_void,
    store: *const c_void,
    environment: u64,
    is_animation_overlay: bool,
    declares_own: bool,
    animation_base: *const c_void,
    animation_base_store: *const c_void,
    animation_base_environment: u64,
) {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_set_pseudo_element_custom_property_data",
        crate::css::style::owner_calls::StyleQuery::SetPseudoElementCustomPropertyData {
            node,
            pseudo,
            data,
            store,
            environment,
            is_animation_overlay,
            declares_own,
            animation_base,
            animation_base_store,
            animation_base_environment,
        },
    );
}

/// Answers [`style_engine_set_pseudo_element_custom_property_data`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_set_pseudo_element_custom_property_data`].
#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn owner_set_pseudo_element_custom_property_data(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
    pseudo: u8,
    data: *const c_void,
    store: *const c_void,
    environment: u64,
    is_animation_overlay: bool,
    declares_own: bool,
    animation_base: *const c_void,
    animation_base_store: *const c_void,
    animation_base_environment: u64,
) {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return;
    };
    let animation_base =
        is_animation_overlay.then_some((animation_base_environment, animation_base_store, animation_base));
    // What the pseudo-element held before is the document thread's to release.
    let retired = unsafe {
        engine.set_pseudo_element_custom_property_data(
            node,
            pseudo,
            data,
            store,
            environment,
            declares_own,
            animation_base,
        )
    };
    engine
        .host
        .retired_custom_property_data
        .extend(retired.and_then(|held| held.data));
}

/// The custom-property environment one of an element's synthetic pseudo-elements holds, as
/// `style_engine_element_custom_property_data` answers for the element.
///
/// # Safety
/// `engine` must be live and `identity` must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_pseudo_element_custom_property_data(
    engine: StyleEngineHandle,
    node: u32,
    pseudo: u8,
    identity: *mut u64,
) -> *const c_void {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_pseudo_element_custom_property_data",
        crate::css::style::owner_calls::StyleQuery::PseudoElementCustomPropertyData { node, pseudo, identity },
    )
    .pointer()
}

/// Answers [`style_engine_pseudo_element_custom_property_data`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_pseudo_element_custom_property_data`].
pub(crate) unsafe fn owner_pseudo_element_custom_property_data(
    engine: &StyleEngine,
    node: u32,
    pseudo: u8,
    identity: *mut u64,
) -> *const c_void {
    let (data, environment) = StyleNodeID::from_raw(node).map_or((std::ptr::null(), 0), |node| {
        engine.pseudo_element_custom_property_data(node, pseudo)
    });
    unsafe { *identity = environment };
    data
}

/// The kinds of the element's synthetic pseudo-elements that hold a custom-property environment,
/// one bit per kind, so the host asks for only those.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_pseudo_elements_with_custom_property_data(
    engine: StyleEngineHandle,
    node: u32,
) -> u64 {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_pseudo_elements_with_custom_property_data",
        crate::css::style::owner_calls::StyleQuery::PseudoElementsWithCustomPropertyData { node },
    )
    .u64()
}

/// Answers [`style_engine_pseudo_elements_with_custom_property_data`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_pseudo_elements_with_custom_property_data`].
pub(crate) unsafe fn owner_pseudo_elements_with_custom_property_data(engine: &StyleEngine, node: u32) -> u64 {
    StyleNodeID::from_raw(node).map_or(0, |node| engine.pseudo_elements_with_custom_property_data(node))
}

/// Installs the authoritative release order recorded for the next replay transaction.
///
/// # Safety
/// `engine` must be live and `atoms` must name `count` readable atom identities.
pub unsafe fn replay_set_reclaimed_style_atoms(engine: StyleEngineHandle, atoms: *const u32, count: usize) {
    let engine = unsafe { engine.for_replay() };
    assert!(engine.host.replay_reclaimed_style_atoms.is_none());
    let atoms = if count == 0 {
        &[]
    } else {
        assert!(!atoms.is_null());
        unsafe { std::slice::from_raw_parts(atoms, count) }
    };
    engine.host.replay_reclaimed_style_atoms = Some(atoms.iter().copied().map(StyleAtomID).collect());
}
/// Reads one counter by index, returning its stable name and writing its value and name length, or
/// null once the index is past the end. C++ enumerates the counters this way rather than
/// duplicating the list. The name is borrowed static UTF-8 and is not nul-terminated.
///
/// # Safety
/// `engine` must be live, and the out pointers must be writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_counter(
    engine: StyleEngineHandle,
    index: usize,
    out_value: *mut u64,
    out_name_length: *mut usize,
) -> *const u8 {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_counter",
        crate::css::style::owner_calls::StyleQuery::Counter {
            index,
            out_value,
            out_name_length,
        },
    )
    .pointer()
    .cast::<u8>()
}

/// Answers [`style_engine_counter`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_counter`].
pub(crate) unsafe fn owner_counter(
    engine: &StyleEngine,
    index: usize,
    out_value: *mut u64,
    out_name_length: *mut usize,
) -> *const u8 {
    let result = engine.counters().iter().nth(index);
    engine.record_boundary_call(EventKind::Counter, |payload| {
        payload.write_u64(u64::try_from(index).expect("counter index exceeds u64"));
        payload.write_bool(result.is_some());
        if let Some((name, value)) = result {
            payload.write_bytes(name.as_bytes());
            payload.write_u64(value);
        }
    });
    let Some((name, value)) = result else {
        return std::ptr::null();
    };
    unsafe {
        *out_value = value;
        *out_name_length = name.len();
    }
    name.as_ptr()
}

/// Records a benchmark phase marker when capture is enabled.
///
/// # Safety
/// `engine` must be live, and `name` must point at `length` readable UTF-16 code units.
#[cfg(feature = "style-recording")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_record_benchmark_marker(
    engine: StyleEngineHandle,
    name: *const c_void,
    length: usize,
    is_ascii: bool,
) {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_record_benchmark_marker",
        crate::css::style::owner_calls::StyleQuery::BenchmarkMarker { name, length, is_ascii },
    );
}

/// Answers [`style_engine_record_benchmark_marker`] with `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_record_benchmark_marker`].
#[cfg(feature = "style-recording")]
pub(crate) unsafe fn owner_record_benchmark_marker(
    engine: &StyleEngine,
    name: *const c_void,
    length: usize,
    is_ascii: bool,
) {
    if engine.recording_id().is_none() {
        return;
    }
    engine.record_boundary_call(EventKind::BenchmarkMarker, |payload| {
        if is_ascii {
            let name = unsafe { borrow(name.cast::<u8>(), length) };
            payload.write_length(name.len());
            for &code_unit in name {
                payload.write_u16(u16::from(code_unit));
            }
        } else {
            payload.write_u16_slice(unsafe { borrow(name.cast::<u16>(), length) });
        }
    });
}

pub(crate) unsafe fn borrow<'a, T>(pointer: *const T, count: usize) -> &'a [T] {
    if count == 0 {
        return &[];
    }
    assert!(!pointer.is_null(), "a non-empty delta array must not be null");
    unsafe { std::slice::from_raw_parts(pointer, count) }
}

unsafe fn borrow_mut<'a, T>(pointer: *mut T, count: usize) -> &'a mut [T] {
    if count == 0 {
        return &mut [];
    }
    assert!(!pointer.is_null(), "a non-empty output array must not be null");
    unsafe { std::slice::from_raw_parts_mut(pointer, count) }
}

use super::owner_calls::BoundaryResult;

include!(concat!(env!("OUT_DIR"), "/style_engine_boundary_generated.rs"));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::style::Lookup;
    use crate::css::style::instrumentation::Counter;
    use crate::css::style::memory::MemoryCategory;
    use crate::css::style::memory::Tier;
    use crate::css::style::transaction::InputKind;

    fn no_relations() -> FfiTreeRelations {
        FfiTreeRelations {
            parent: 0,
            previous_element_sibling: 0,
            next_element_sibling: 0,
            tree_scope: 0,
            assigned_slot: 0,
            reserved: 0,
        }
    }

    #[cfg(feature = "style-recording")]
    #[test]
    fn recording_transaction_rows_zero_struct_padding() {
        let relations = no_relations();
        let mut tree = std::mem::MaybeUninit::<FfiTreeDelta>::uninit();
        let tree_pointer = tree.as_mut_ptr();
        // SAFETY: every byte starts initialized and every typed field is then written with a valid
        // value before the row is assumed initialized.
        let tree = unsafe {
            tree_pointer.cast::<u8>().write_bytes(0xaa, size_of::<FfiTreeDelta>());
            std::ptr::addr_of_mut!((*tree_pointer).node).write(1);
            std::ptr::addr_of_mut!((*tree_pointer).old_connected).write(false);
            std::ptr::addr_of_mut!((*tree_pointer).new_connected).write(true);
            std::ptr::addr_of_mut!((*tree_pointer).old_relations).write(relations);
            std::ptr::addr_of_mut!((*tree_pointer).new_relations).write(relations);
            tree.assume_init()
        };
        let mut payload = crate::css::style::record_replay::PayloadWriter::default();
        write_recording_tree_deltas(&[tree], &mut payload);
        assert_eq!(&payload.as_bytes()[18..20], &[0, 0]);

        let mut state = std::mem::MaybeUninit::<FfiStateDelta>::uninit();
        let state_pointer = state.as_mut_ptr();
        // SAFETY: every byte starts initialized and every typed field is then written with a valid
        // value before the row is assumed initialized.
        let state = unsafe {
            state_pointer.cast::<u8>().write_bytes(0xaa, size_of::<FfiStateDelta>());
            std::ptr::addr_of_mut!((*state_pointer).node).write(1);
            std::ptr::addr_of_mut!((*state_pointer).fact).write(FfiStateFact::Hover);
            std::ptr::addr_of_mut!((*state_pointer).new_value).write(true);
            state.assume_init()
        };
        let mut first_payload = crate::css::style::record_replay::PayloadWriter::default();
        write_recording_state_deltas(&[state], &mut first_payload);
        assert_eq!(&first_payload.as_bytes()[18..20], &[0, 0]);

        let mut second_state = std::mem::MaybeUninit::<FfiStateDelta>::uninit();
        let second_state_pointer = second_state.as_mut_ptr();
        // SAFETY: every byte starts initialized and every typed field is then written with a valid
        // value before the row is assumed initialized.
        let second_state = unsafe {
            second_state_pointer
                .cast::<u8>()
                .write_bytes(0xbb, size_of::<FfiStateDelta>());
            std::ptr::addr_of_mut!((*second_state_pointer).node).write(1);
            std::ptr::addr_of_mut!((*second_state_pointer).fact).write(FfiStateFact::Hover);
            std::ptr::addr_of_mut!((*second_state_pointer).new_value).write(true);
            second_state.assume_init()
        };
        let mut second_payload = crate::css::style::record_replay::PayloadWriter::default();
        write_recording_state_deltas(&[second_state], &mut second_payload);
        assert_eq!(first_payload.as_bytes(), second_payload.as_bytes());

        let mut style_input = std::mem::MaybeUninit::<FfiElementStyleInput>::uninit();
        let style_input_pointer = style_input.as_mut_ptr();
        // SAFETY: every byte starts initialized and every typed field is then written with a valid
        // value before the row is assumed initialized.
        let style_input = unsafe {
            style_input_pointer
                .cast::<u8>()
                .write_bytes(0xaa, size_of::<FfiElementStyleInput>());
            std::ptr::addr_of_mut!((*style_input_pointer).style_node).write(1);
            std::ptr::addr_of_mut!((*style_input_pointer).reaction).write(2);
            std::ptr::addr_of_mut!((*style_input_pointer).inherited_style_groups).write(3);
            style_input.assume_init()
        };
        let mut payload = crate::css::style::record_replay::PayloadWriter::default();
        write_recording_element_style_inputs(&[style_input], &mut payload);
        assert_eq!(&payload.as_bytes()[18..20], &[0, 0]);
    }

    #[test]
    fn initial_tree_batch_uses_the_document_root_as_its_transaction_envelope() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut nodes = [0_u32; 4];
        engine.allocate_style_nodes(&mut nodes);

        let initial_tree = [
            FfiTreeDelta {
                node: nodes[0],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: no_relations(),
            },
            FfiTreeDelta {
                node: nodes[1],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: FfiTreeRelations {
                    parent: nodes[0],
                    ..no_relations()
                },
            },
            FfiTreeDelta {
                node: nodes[2],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: FfiTreeRelations {
                    parent: nodes[1],
                    ..no_relations()
                },
            },
        ];
        engine.apply_transaction_batch(&initial_tree, (&[], &[]), &[], &[], &[], &[]);

        let root = StyleNodeID::from_raw(nodes[0]).unwrap();
        let child = StyleNodeID::from_raw(nodes[1]).unwrap();
        let grandchild = StyleNodeID::from_raw(nodes[2]).unwrap();
        assert_eq!(
            engine.tree().preorder(root).collect::<Vec<_>>(),
            vec![root, child, grandchild]
        );

        let transaction = engine.take_transaction();
        assert_eq!(transaction.inputs.len(), 2);
        assert!(transaction.inputs.iter().all(|input| matches!(
            input.key,
            InputKey::TreeRelations(node) | InputKey::LocalFeature(node, LocalFeatureKey::ArrivingFacts) if node == root
        )));
        engine.release_transaction(transaction);

        assert_eq!(engine.counters().get(Counter::InitialBulkLoads), 1);
        assert_eq!(engine.counters().get(Counter::InitialBulkTreeRows), 3);
        assert_eq!(engine.counters().get(Counter::RawMutationRecords), 3);
        assert_eq!(engine.counters().get(Counter::TreeDeltas), 3);

        let later_arrival = [FfiTreeDelta {
            node: nodes[3],
            old_connected: false,
            new_connected: true,
            old_relations: no_relations(),
            new_relations: FfiTreeRelations {
                parent: nodes[0],
                previous_element_sibling: nodes[1],
                ..no_relations()
            },
        }];
        engine.apply_transaction_batch(&later_arrival, (&[], &[]), &[], &[], &[], &[]);
        let transaction = engine.take_transaction();
        assert_eq!(transaction.inputs.len(), 3);
        let later = StyleNodeID::from_raw(nodes[3]).unwrap();
        assert!(transaction.inputs.iter().any(|input| matches!(
            input.key,
            InputKey::TreeRelations(node) if node == child
        )));
        assert_eq!(
            transaction
                .inputs
                .iter()
                .filter(|input| matches!(
                    input.key,
                    InputKey::TreeRelations(node) | InputKey::LocalFeature(node, LocalFeatureKey::ArrivingFacts) if node == later
                ))
                .count(),
            2
        );
        engine.release_transaction(transaction);
        assert_eq!(engine.counters().get(Counter::InitialBulkLoads), 1);
        assert_eq!(engine.counters().get(Counter::InitialBulkTreeRows), 3);
    }

    #[test]
    fn preallocated_siblings_keep_their_disconnected_before_rows() {
        for split_batches in [false, true] {
            let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
            let mut nodes = [0_u32; 4];
            engine.allocate_style_nodes(&mut nodes);
            let root = StyleNodeID::from_raw(nodes[0]).unwrap();
            engine.apply_transaction_batch(
                &[FfiTreeDelta {
                    node: nodes[0],
                    old_connected: false,
                    new_connected: true,
                    old_relations: no_relations(),
                    new_relations: no_relations(),
                }],
                (&[], &[]),
                &[],
                &[],
                &[],
                &[],
            );
            let transaction = engine.take_transaction();
            engine.release_transaction(transaction);

            let arrivals: Vec<_> = (1..nodes.len())
                .map(|index| FfiTreeDelta {
                    node: nodes[index],
                    old_connected: false,
                    new_connected: true,
                    old_relations: no_relations(),
                    new_relations: FfiTreeRelations {
                        parent: nodes[0],
                        previous_element_sibling: if index == 1 { 0 } else { nodes[index - 1] },
                        next_element_sibling: nodes.get(index + 1).copied().unwrap_or(0),
                        ..no_relations()
                    },
                })
                .collect();
            let chunk_size = if split_batches { 1 } else { arrivals.len() };
            for chunk in arrivals.chunks(chunk_size) {
                engine.apply_transaction_batch(chunk, (&[], &[]), &[], &[], &[], &[]);
                // Applying the staged tree between batches must not lose a row's before side.
                engine.apply_staged_tree_deltas();
            }
            let transaction = engine.take_transaction();
            for raw in &nodes[1..] {
                let node = StyleNodeID::from_raw(*raw).unwrap();
                let input = transaction
                    .inputs
                    .iter()
                    .find(|input| input.key == InputKey::TreeRelations(node))
                    .unwrap();
                assert_eq!(input.old, InputValue::TreeRelations(None));
                assert!(
                    transaction
                        .inputs
                        .iter()
                        .any(|input| input.key == InputKey::LocalFeature(node, LocalFeatureKey::ArrivingFacts))
                );
            }
            assert_eq!(
                engine.tree().preorder(root).collect::<Vec<_>>(),
                nodes.map(|raw| StyleNodeID::from_raw(raw).unwrap())
            );
            engine.release_transaction(transaction);
        }
    }

    #[test]
    fn element_arrival_rows_install_intrinsic_facts() {
        assert_eq!(size_of::<FfiElementArrival>(), 36);
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut nodes = [0_u32; 2];
        engine.allocate_style_nodes(&mut nodes);
        let tree = [
            FfiTreeDelta {
                node: nodes[0],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: no_relations(),
            },
            FfiTreeDelta {
                node: nodes[1],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: FfiTreeRelations {
                    parent: nodes[0],
                    ..no_relations()
                },
            },
        ];
        let arrivals = [
            FfiElementArrival {
                node: nodes[0],
                namespace_atom: 11,
                language_atom: 12,
                directionality_atom: 13,
                adjustment_facts: 0,
                construction_facts: 0,
                custom_state_offset: 0,
                custom_state_count: 2,
                heading_level: 4,
                is_slot: true,
                box_kind: 0,
                associated_pseudo_kind_plus_one: 0,
            },
            FfiElementArrival {
                node: nodes[1],
                namespace_atom: 21,
                language_atom: 22,
                directionality_atom: 23,
                adjustment_facts: 0,
                construction_facts: 0,
                custom_state_offset: 2,
                custom_state_count: 1,
                heading_level: 0,
                is_slot: false,
                box_kind: 0,
                associated_pseudo_kind_plus_one: 0,
            },
        ];
        engine.apply_transaction_batch(&tree, (&arrivals, &[31, 32, 33]), &[], &[], &[], &[]);
        let transaction = engine.take_transaction();

        let root = StyleNodeID::from_raw(nodes[0]).unwrap();
        let child = StyleNodeID::from_raw(nodes[1]).unwrap();
        assert_eq!(engine.facts.namespace_of(root), StyleAtomID(11));
        assert_eq!(engine.facts.language_of(root), StyleAtomID(12));
        assert_eq!(engine.facts.directionality_of(root), StyleAtomID(13));
        assert_eq!(engine.facts.heading_level_of(root), 4);
        assert!(engine.facts.is_slot(root));
        assert_eq!(engine.facts.custom_states_of(root), &[StyleAtomID(31), StyleAtomID(32)]);
        assert_eq!(engine.facts.namespace_of(child), StyleAtomID(21));
        assert_eq!(engine.facts.custom_states_of(child), &[StyleAtomID(33)]);
        engine.release_transaction(transaction);
    }

    fn arrival_for(node: u32, custom_state_offset: u32, custom_state_count: u32) -> FfiElementArrival {
        FfiElementArrival {
            node,
            namespace_atom: 1,
            language_atom: 2,
            directionality_atom: 3,
            adjustment_facts: 0,
            construction_facts: 0,
            custom_state_offset,
            custom_state_count,
            heading_level: 0,
            is_slot: false,
            box_kind: 0,
            associated_pseudo_kind_plus_one: 0,
        }
    }

    #[test]
    #[should_panic(expected = "an element arrival named an invalid style node")]
    fn malformed_element_arrival_rejects_an_invalid_node() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        engine.apply_transaction_batch(&[], (&[arrival_for(0, 0, 0)], &[]), &[], &[], &[], &[]);
    }

    #[test]
    #[should_panic(expected = "an element arrival custom-state range overflowed")]
    fn malformed_element_arrival_rejects_an_overflowing_custom_state_range() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut nodes = [0];
        engine.allocate_style_nodes(&mut nodes);
        engine.apply_transaction_batch(&[], (&[arrival_for(nodes[0], u32::MAX, 1)], &[]), &[], &[], &[], &[]);
    }

    #[test]
    #[should_panic(expected = "an element arrival named custom states outside the shared atom column")]
    fn malformed_element_arrival_rejects_an_out_of_bounds_custom_state_range() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut nodes = [0];
        engine.allocate_style_nodes(&mut nodes);
        engine.apply_transaction_batch(&[], (&[arrival_for(nodes[0], 0, 2)], &[1]), &[], &[], &[], &[]);
    }

    #[test]
    fn initial_tree_bulk_load_publishes_match_answers_before_traversal() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut nodes = [0_u32; 3];
        engine.allocate_style_nodes(&mut nodes);
        let initial_tree = [
            FfiTreeDelta {
                node: nodes[0],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: no_relations(),
            },
            FfiTreeDelta {
                node: nodes[1],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: FfiTreeRelations {
                    parent: nodes[0],
                    ..no_relations()
                },
            },
            FfiTreeDelta {
                node: nodes[2],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: FfiTreeRelations {
                    parent: nodes[1],
                    ..no_relations()
                },
            },
        ];
        let initial_features = nodes.map(|node| FfiLocalFeatureDelta {
            node,
            feature_kind: FfiFeatureKind::TagName,
            name_atom: 0,
            old_kind: FfiFeatureValueKind::Absent,
            old_atom: 0,
            new_kind: FfiFeatureValueKind::Atom,
            new_atom: 1,
        });
        engine.apply_transaction_batch(&initial_tree, (&[], &[]), &initial_features, &[], &[], &[]);

        let root = StyleNodeID::from_raw(nodes[0]).unwrap();
        let mut published = Vec::new();
        assert!(!engine.take_style_transaction(root, |_, _, answers| {
            published.extend_from_slice(answers);
        }));
        // The pass stops before a row whose ancestor only the host settles; its later waves
        // publish the rest.
        while engine.state.host.suspended_style_pass.is_some() {
            engine.take_style_transaction(root, |_, _, answers| {
                published.extend_from_slice(answers);
            });
        }
        assert_eq!(
            published.iter().map(|delta| delta.style_node).collect::<Vec<_>>(),
            nodes
        );
        assert!(published.iter().all(|delta| {
            delta.pseudo_kind == u8::MAX
                && delta.old_style_record == 0
                && delta.new_style_record == 0
                && delta.damage == FfiStyleDeltaDamage::None
        }));
        assert_eq!(
            published.iter().map(|delta| delta.gap).collect::<Vec<_>>(),
            [FfiStyleDeltaGap::Materialize; 3]
        );
        assert_eq!(engine.counters().get(Counter::InitialBulkMatchLoads), 1);
        assert_eq!(engine.counters().get(Counter::InitialBulkMatchRows), 3);
        assert_eq!(engine.counters().get(Counter::PublishedMatchAnswerRecords), 3);
        assert_eq!(engine.counters().get(Counter::PreparedMatchingBatchRowsCloned), 3);

        let upqueries = engine.counters().get(Counter::MatchAnswerUpqueries);
        assert!(engine.begin_cold_matching_batch(root));
        for raw in nodes[..2].iter().copied() {
            let node = StyleNodeID::from_raw(raw).unwrap();
            assert_eq!(engine.consume_published_match_answer(node), Some(Vec::new()));
        }
        engine.end_cold_matching_batch();
        assert!(matches!(
            engine.retained_match_answer(StyleNodeID::from_raw(nodes[0]).unwrap()),
            Lookup::Known(_)
        ));
        assert!(matches!(
            engine.retained_match_answer(StyleNodeID::from_raw(nodes[2]).unwrap()),
            Lookup::Missing(_)
        ));
        assert_eq!(engine.counters().get(Counter::MatchAnswerUpqueries), upqueries);
        assert_eq!(
            engine
                .counters()
                .get(Counter::MatchElementCallsDuringPublishedStyleTransaction),
            0
        );
    }

    #[test]
    fn one_batch_carries_every_typed_delta_kind() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut nodes = [0_u32; 3];
        engine.allocate_style_nodes(&mut nodes);

        // Connect the two elements the batch below reports facts about. An arriving element folds
        // everything it publishes onto its arrival entry, so a state change is only its own input
        // once the element is already there.
        let arrival = [
            FfiTreeDelta {
                node: nodes[0],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: no_relations(),
            },
            FfiTreeDelta {
                node: nodes[1],
                old_connected: false,
                new_connected: true,
                old_relations: no_relations(),
                new_relations: FfiTreeRelations {
                    parent: nodes[0],
                    ..no_relations()
                },
            },
        ];
        engine.apply_transaction_batch(&arrival, (&[], &[]), &[], &[], &[], &[]);
        let settled = engine.take_transaction();
        engine.release_transaction(settled);

        let tree = [FfiTreeDelta {
            node: nodes[2],
            old_connected: false,
            new_connected: true,
            old_relations: no_relations(),
            new_relations: FfiTreeRelations {
                parent: nodes[0],
                previous_element_sibling: nodes[1],
                ..no_relations()
            },
        }];
        let features = [FfiLocalFeatureDelta {
            node: nodes[1],
            feature_kind: FfiFeatureKind::Class,
            name_atom: 42,
            old_kind: FfiFeatureValueKind::Absent,
            old_atom: 0,
            new_kind: FfiFeatureValueKind::Present,
            new_atom: 0,
        }];
        let states = [FfiStateDelta {
            node: nodes[1],
            fact: FfiStateFact::Hover,
            new_value: true,
        }];
        let declarations = [FfiElementDeclarationDelta {
            node: nodes[1],
            kind: FfiElementDeclarationKind::InlineStyle,
            old_block: 0,
            new_block: 7,
        }];

        let style_inputs = [FfiElementStyleInput {
            style_node: nodes[1],
            reaction: crate::css::style::transaction::STYLE_REACTION_RECOMPUTE_STYLE,
            inherited_style_groups: 0,
        }];
        engine.apply_transaction_batch(&tree, (&[], &[]), &features, &states, &declarations, &style_inputs);

        let transaction = engine.take_transaction();
        let node0 = StyleNodeID::from_raw(nodes[0]).unwrap();
        let node1 = StyleNodeID::from_raw(nodes[1]).unwrap();
        let node2 = StyleNodeID::from_raw(nodes[2]).unwrap();
        assert_eq!(engine.tree().children(node0).collect::<Vec<_>>(), vec![node1, node2]);
        let kinds: Vec<InputKind> = transaction.inputs.iter().map(|input| input.key.kind()).collect();
        assert!(kinds.contains(&InputKind::TreeRelations));
        assert!(kinds.contains(&InputKind::LocalFeature));
        assert!(kinds.contains(&InputKind::State));
        assert!(kinds.contains(&InputKind::ElementDeclaration));
        assert!(kinds.contains(&InputKind::ElementStyleInput));
        engine.release_transaction(transaction);

        assert_eq!(engine.counters().get(Counter::TreeDeltas), 4);
        assert_eq!(engine.counters().get(Counter::LocalFeatureDeltas), 1);
        assert_eq!(engine.counters().get(Counter::StateDeltas), 1);
        assert_eq!(engine.counters().get(Counter::ElementDeclarationDeltas), 1);
    }

    #[test]
    fn a_batch_of_no_deltas_costs_nothing() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        engine.apply_transaction_batch(&[], (&[], &[]), &[], &[], &[], &[]);
        let transaction = engine.take_transaction();
        assert!(transaction.is_empty());
        engine.release_transaction(transaction);
        assert_eq!(engine.memory().bytes_in_tier(Tier::Acceleration), 0);
    }

    #[test]
    fn identities_are_minted_in_one_call_per_batch() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut nodes = [0_u32; 512];
        engine.allocate_style_nodes(&mut nodes);
        assert_eq!(engine.counters().get(Counter::StyleNodesAllocated), 512);
        assert!(nodes.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(engine.memory().bytes_in_category(MemoryCategory::RelationColumns) > 0);
    }
}

/// Whether a container asked about before it had a box still waits for the layout that gives it one.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_has_size_containers_needing_evaluation_after_layout(
    engine: StyleEngineHandle,
) -> bool {
    crate::css::style::owner_calls::ask(
        engine,
        "style_engine_has_size_containers_needing_evaluation_after_layout",
        crate::css::style::owner_calls::StyleQuery::HasSizeContainersNeedingEvaluationAfterLayout,
    )
    .is()
}

/// Answers [`style_engine_has_size_containers_needing_evaluation_after_layout`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_has_size_containers_needing_evaluation_after_layout`].
pub(crate) unsafe fn owner_has_size_containers_needing_evaluation_after_layout(engine: &StyleEngine) -> bool {
    engine.has_size_containers_needing_evaluation_after_layout()
}

/// The elements the size query container dependent walks have visited; `reset` starts the count
/// again.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_size_query_container_scan_visits(
    engine: StyleEngineInputHandle,
    reset: bool,
) -> u64 {
    crate::css::style::owner_calls::ask(
        engine.home(),
        "style_engine_size_query_container_scan_visits",
        crate::css::style::owner_calls::StyleQuery::SizeQueryContainerScanVisits { reset },
    )
    .u64()
}

/// Answers [`style_engine_size_query_container_scan_visits`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_size_query_container_scan_visits`].
pub(crate) unsafe fn owner_size_query_container_scan_visits(engine: &mut StyleEngine, reset: bool) -> u64 {
    engine.size_query_container_scan_visits(reset)
}

/// Records every element whose style a size query or container-relative unit decided against the
/// container `node`, as a change of what the container answers does.
///
/// # Safety
/// `engine` must be live.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn style_engine_record_size_container_query_dependents(
    engine: StyleEngineInputHandle,
    node: u32,
) {
    crate::css::style::owner_calls::send(
        engine,
        "style_engine_record_size_container_query_dependents",
        crate::css::style::owner_calls::EngineChange::RecordSizeContainerQueryDependents { node },
    );
}

/// Answers [`style_engine_record_size_container_query_dependents`] from `engine`, on the render owner.
///
/// # Safety
///
/// As for [`style_engine_record_size_container_query_dependents`].
pub(crate) unsafe fn owner_record_size_container_query_dependents(
    engine: &mut crate::css::style::StyleEngine,
    node: u32,
) {
    let Some(node) = StyleNodeID::from_raw(node) else {
        return;
    };
    engine.size_container_content_size_changed(node);
}

/// A step the host takes for a style pass between its engine calls, which the style seal counts.
/// NB: It crosses the FFI by value at full register width. The x86-64 ABI leaves the upper bits of
///     a byte-sized argument undefined, and GCC-built callers leave them set, while the Rust side
///     assumes they are clear and indexes its name table with the whole register.
#[derive(Clone, Copy)]
#[repr(u32)]
pub enum FfiStyleHostStep {
    /// Another style transaction taken within the same update.
    Wave,
    /// A row of a published batch the host applies.
    Row,
    RetriedAfterAncestors,
    RetriedMaterialization,
    DeclinedRow,
    InheritedCustomPropertyRefresh,
}

impl FfiStyleHostStep {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Wave => "host:wave",
            Self::Row => "host:row",
            Self::RetriedAfterAncestors => "host:retried_after_ancestors",
            Self::RetriedMaterialization => "host:retried_materialization",
            Self::DeclinedRow => "host:declined_row",
            Self::InheritedCustomPropertyRefresh => "host:inherited_custom_property_refresh",
        }
    }
}

/// Counts one step the host takes for a style pass between its engine calls, by the reason it
/// takes it, as the style seal counts the engine calls.
#[unsafe(no_mangle)]
pub extern "C" fn style_engine_note_host_step(step: FfiStyleHostStep) {
    super::seal::note_engine_call(step.name());
}
