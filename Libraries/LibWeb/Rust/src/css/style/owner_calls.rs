/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the main thread asks of the style engine of its document, which the render owner owns.
//!
//! The main thread does not enter the engine. A write is a [`StyleChange`] it leaves in the engine's home: whoever
//! reaches the engine next applies it first, in order with the style inputs. A read is a [`StyleQuery`] it asks: the
//! owner answers it in one round trip, after every change the main thread made before it, into the query the main
//! thread holds while it waits.

use super::StyleEngine;
use super::bridge::{
    BoundaryRead, BoundaryWrite, FfiAppliedStyleReaction, FfiElementDeclarationKind, FfiNativeRuleTarget, FfiRuleMatch,
    InputForPass,
};
use super::engine_home::{PendingFacts, StyleEngineHandle, StyleEngineInputHandle};
use super::inputs::HandedCustomPropertyEnvironment;
use super::tree::StyleNodeID;
use crate::layout::LayoutNodeArena;
use crate::render_owner::{Answer, DocumentId, EngineAnswered, Query};
use smallvec::SmallVec;
use std::ffi::c_void;
use std::ptr::NonNull;

/// A write to a document's style engine, which the render owner applies. A write that lends the engine what the host
/// owns for the call is a [`StyleQuery`] instead, which the main thread waits for.
pub(crate) enum EngineChange {
    /// A write the boundary specification generates.
    Boundary(BoundaryWrite),
    /// Records every element whose style a size query or container-relative unit decided against the container
    /// `node`, as a change of what the container answers.
    RecordSizeContainerQueryDependents { node: u32 },
    /// Whether the conditions of each native rule, by identity, hold now.
    RuleConditionsHold(Vec<(u64, bool)>),
    /// The cascade layer order of the author sheets of the tree scope `tree_scope`, by name; an empty name is the
    /// unlayered rules' place.
    LayerOrder { tree_scope: u32, names: Vec<Vec<u16>> },
    /// Drops what the container conditions of a declined row read of its containers.
    DiscardContainerEffects { node: u32 },
    /// Publishes the previous document-element font answer before style evaluation begins.
    PrepareRootFontResolution { generation: u64 },
    /// The document's `@font-face` table for the generation the next update computes against, and the memo its
    /// resolutions go into.
    PublishFontFaceSnapshot {
        snapshot: Option<std::sync::Arc<super::font_faces::FontFaceSnapshot>>,
        memo: Option<super::font_faces::RetainedFontCascadeMemo>,
    },
    /// The host took what the pass published for the row of an element whose animations it sampled.
    RowSampledTakenByHost(StyleNodeID),
    /// The host took the animation plan the row of an element, or of its pseudo-element of `pseudo_kind`, left.
    AnimationPlanTakenByHost { node: StyleNodeID, pseudo_kind: u8 },
    /// The host took what the container conditions of an element's row read of its containers, which the engine
    /// records.
    ContainerEffectsTakenByHost(StyleNodeID),
    /// A sheet of `origin`, which the main thread names `sheet` as it adds it: the engine's sheets are numbered in the
    /// order they are added.
    AddSheet {
        object: super::program::StyleSheetObjectID,
        origin: crate::css::cascaded_properties::CascadeOrigin,
        sheet: super::program::SheetID,
    },
    /// The declarations the native rule of `identity` now holds, which the engine publishes where it holds the rule.
    RuleDeclarationsChanged {
        identity: u64,
        declarations: Option<std::sync::Arc<crate::css::declaration_block::DeclarationBlockData>>,
    },
    /// The anchor names the record `style_record` installs on an element, in place of the ones it registered before,
    /// which the main thread knows it `has_names` of.
    RegisterAnchorNames {
        node: StyleNodeID,
        style_record: u64,
        has_names: bool,
    },
    /// The host took the transition step the pass decided for an element's row, or for its synthetic pseudo-element
    /// of `pseudo_kind`.
    TransitionStepTakenByHost { node: StyleNodeID, pseudo_kind: Option<u8> },
    /// The host took the environment the engine named for a synthetic pseudo-element as it settled it, from its copy.
    PseudoElementEnvironmentTakenByHost { node: StyleNodeID, pseudo_kind: u8 },
    /// The rules the main thread compiled of a sheet, and the selectors it replaced, which the engine publishes.
    CompileRules(Box<crate::css::rule::compilation::CompiledRules>),
    /// The `@keyframes` one style scope defines now, in place of the ones it defined before.
    SetTreeScopeAnimationKeyframes {
        tree_scope: super::tree::TreeScopeID,
        shadow_root_identity: usize,
        keyframes: super::animations::TreeScopeKeyframes,
    },
    /// The native rules of `identities` left their sheet, in the order the engine removes them: each with what
    /// removing the ones before it left, where it still holds it.
    RemoveNativeRules(Box<[u64]>),
    /// The transitions one of an element's lists now holds.
    SetElementTransitions {
        node: StyleNodeID,
        slot: super::animations::AnimationSlot,
        transitions: Box<[super::transition_step::PublishedTransition]>,
    },
    /// The effects the host holds for one of an element's animation lists, in composite order.
    SetElementAnimationEffectDescriptions {
        node: StyleNodeID,
        slot: super::animations::AnimationSlot,
        effects: Vec<super::animations::PublishedEffect>,
    },
    /// The custom-property environment an element now holds, or that it holds none.
    SetElementCustomPropertyData(StyleNodeID, Option<HandedCustomPropertyEnvironment>),
    /// The custom-property environment one of an element's synthetic pseudo-elements now holds, or that it holds none.
    SetPseudoElementCustomPropertyData(StyleNodeID, u8, Option<HandedCustomPropertyEnvironment>),
    /// A benchmark phase marker, in UTF-16, which a recording engine records.
    #[cfg(feature = "style-recording")]
    BenchmarkMarker(Box<[u16]>),
}

impl EngineChange {
    /// What the change may leave the engine holding for its next style transaction, where it holds `held`, which the
    /// main thread knows once it sends it.
    pub(crate) fn leaves(&self, held: PendingFacts) -> PendingFacts {
        use BoundaryWrite as Write;
        match self {
            Self::Boundary(Write::MakeDeferredPseudoElementStyleObservable { .. }) => {
                PendingFacts::OBSERVABLE_DEFERRED_PSEUDO_ELEMENTS
            }
            // Only the elements whose deferred style was made observable owe an input.
            Self::Boundary(Write::SetPseudoElementStyleDeferred { .. })
                if !held.contains(PendingFacts::OBSERVABLE_DEFERRED_PSEUDO_ELEMENTS) =>
            {
                PendingFacts::NONE
            }
            // What only keeps the host's rows, indexes and publications beside the engine's state, or takes an input
            // away.
            Self::Boundary(
                Write::SetFoldIdAndClassNameCase { .. }
                | Write::SetHtmlElementNamespace { .. }
                | Write::SetCounterStyleEnvironmentIdentity { .. }
                | Write::RestoreRowDebts { .. }
                | Write::SetSampledCompositionIdentity { .. }
                | Write::NoteAttributeNameForms { .. }
                | Write::ConsumeElementStyleInput { .. }
                | Write::AbsorbElementStyleInput { .. }
                | Write::AcknowledgeEngineComputedRecord { .. }
                | Write::DiscardStyleTransactionOutputs { .. }
                | Write::RecordTransitionBaseline { .. }
                | Write::ReleaseTransitionBaselines { .. }
                | Write::BeginStyleRecordViewEpoch { .. }
                | Write::EndStyleRecordViewEpoch { .. }
                | Write::SetAttributeValueText { .. }
                | Write::SetElementCustomPropertyNames { .. }
                | Write::SetElementAnimationNames { .. }
                | Write::SetElementCssDefinedAnimations { .. }
                | Write::SetElementAnimationTimingRows { .. }
                | Write::SetAnimationTimelineSamples { .. }
                | Write::SetRootElementFontMetrics { .. },
            )
            | Self::DiscardContainerEffects { .. }
            | Self::PrepareRootFontResolution { .. }
            | Self::PublishFontFaceSnapshot { .. }
            | Self::RowSampledTakenByHost(_)
            | Self::AnimationPlanTakenByHost { .. }
            | Self::TransitionStepTakenByHost { .. }
            | Self::PseudoElementEnvironmentTakenByHost { .. }
            | Self::SetTreeScopeAnimationKeyframes { .. }
            | Self::SetElementCustomPropertyData(..)
            | Self::SetPseudoElementCustomPropertyData(..)
            | Self::AddSheet { .. }
            | Self::RegisterAnchorNames { .. }
            | Self::SetElementTransitions { .. }
            | Self::SetElementAnimationEffectDescriptions { .. } => PendingFacts::NONE,
            #[cfg(feature = "style-recording")]
            Self::BenchmarkMarker(_) => PendingFacts::NONE,
            // What the host takes records the containers it reads, some to evaluate after layout.
            Self::ContainerEffectsTakenByHost(_) => PendingFacts::SIZE_CONTAINERS_AFTER_LAYOUT,
            // Only an element that loses its record may owe its resources an input.
            Self::Boundary(Write::SetElementContainerQueryInputs { record, .. }) if *record != 0 => PendingFacts::NONE,
            Self::Boundary(
                Write::RecordElementStyleInput { .. }
                | Write::RecordDerivedElementStyleInput { .. }
                | Write::RecordTreeCountingStyleInput { .. }
                | Write::RecordFlatTreeDescendantStyleInputs { .. }
                | Write::RecordContainerQueryInput { .. }
                | Write::SetPseudoElementStyleDeferred { .. }
                | Write::SetElementContainerQueryInputs { .. },
            )
            | Self::RecordSizeContainerQueryDependents { .. } => PendingFacts::ELEMENT_INPUT,
            _ => PendingFacts::ANY_CHANGE,
        }
    }

    /// Applies the change to `engine`, on the owner.
    pub(crate) fn apply(self, engine: &mut StyleEngine) {
        if !cfg!(debug_assertions) {
            return self.apply_to(engine);
        }
        let held = engine.pending_facts();
        let may_leave = held.union(self.leaves(held));
        self.apply_to(engine);
        assert!(
            may_leave.contains(engine.pending_facts()),
            "a style engine change left more than the main thread took it to"
        );
    }

    fn apply_to(self, engine: &mut StyleEngine) {
        // The host's copy of the deferred element style inputs followed a write it knows the input of by itself.
        let deferred_inputs_moved = engine.host.deferred_element_style_inputs_moved;
        let followed_by_host = matches!(
            self,
            Self::Boundary(
                BoundaryWrite::RecordElementStyleInput { .. }
                    | BoundaryWrite::RecordDerivedElementStyleInput { .. }
                    | BoundaryWrite::RecordTreeCountingStyleInput { .. }
                    | BoundaryWrite::RecordContainerQueryInput { .. }
            )
        );
        self.apply_write(engine);
        if followed_by_host {
            engine.host.deferred_element_style_inputs_moved = deferred_inputs_moved;
        }
    }

    fn apply_write(self, engine: &mut StyleEngine) {
        match self {
            Self::Boundary(write) => write.apply(engine),
            Self::RecordSizeContainerQueryDependents { node } => unsafe {
                crate::css::style::bridge::owner_record_size_container_query_dependents(engine, node);
            },
            Self::RuleConditionsHold(conditions) => {
                for (identity, holds) in conditions {
                    if let Some(rule) = engine.native_rule_id(identity) {
                        super::bridge::operations::set_rule_conditions_hold(engine, rule.0 + 1, holds);
                    }
                }
            }
            Self::LayerOrder { tree_scope, names } => {
                let layers = names
                    .iter()
                    .map(|name| {
                        if name.is_empty() {
                            0
                        } else {
                            super::bridge::intern_native_text(engine, name).0
                        }
                    })
                    .collect::<Vec<_>>();
                super::bridge::operations::set_layer_order(engine, tree_scope, &layers);
            }
            Self::DiscardContainerEffects { node } => unsafe {
                crate::css::style::bridge::owner_discard_container_effects(engine, node);
            },
            Self::PrepareRootFontResolution { generation } => unsafe {
                crate::css::style::bridge::owner_prepare_root_font_resolution(engine, generation);
            },
            Self::PublishFontFaceSnapshot { snapshot, memo } => {
                crate::css::style::bridge::owner_publish_font_face_snapshot(engine, snapshot, memo);
            }
            Self::RowSampledTakenByHost(node) => {
                engine.take_row_sampled_in_pass(node);
            }
            Self::AnimationPlanTakenByHost { node, pseudo_kind } => {
                engine.take_settled_animation_plan_taken_by_host(node, pseudo_kind);
            }
            Self::ContainerEffectsTakenByHost(node) => {
                engine.take_and_record_container_effects(node);
            }
            #[cfg(feature = "style-recording")]
            Self::BenchmarkMarker(name) => super::bridge::record_benchmark_marker(engine, &name),
            Self::RegisterAnchorNames { node, style_record, .. } => {
                engine.register_anchor_names(node, style_record);
            }
            Self::TransitionStepTakenByHost { node, pseudo_kind } => {
                engine.transition_step_taken_by_host(node, pseudo_kind);
            }
            Self::PseudoElementEnvironmentTakenByHost { node, pseudo_kind } => {
                engine.take_pseudo_element_environment_named_in_settle(node, pseudo_kind);
            }
            Self::SetElementTransitions {
                node,
                slot,
                transitions,
            } => engine.set_element_transitions(node, slot, transitions),
            Self::SetElementAnimationEffectDescriptions { node, slot, effects } => {
                engine.set_element_animation_effect_descriptions(node, slot, effects);
            }
            Self::AddSheet { object, origin, sheet } => {
                let added = engine.add_sheet(object, origin);
                debug_assert!(added == sheet, "the engine numbers its sheets as the main thread does");
            }
            Self::CompileRules(mut compiled) => {
                compiled.publish(engine);
                engine.host.published_rules.push(compiled);
            }
            Self::SetTreeScopeAnimationKeyframes {
                tree_scope,
                shadow_root_identity,
                keyframes,
            } => {
                engine.set_tree_scope_animation_keyframes(tree_scope, shadow_root_identity, keyframes);
                engine.count_animation_keyframe_scopes();
            }
            Self::RemoveNativeRules(identities) => {
                for identity in identities {
                    if let Some(rule) = engine.native_rule_id(identity) {
                        super::bridge::operations::remove_rule(engine, rule.0 + 1);
                    }
                }
            }
            Self::RuleDeclarationsChanged { identity, declarations } => {
                super::bridge::owner_rule_declarations_changed(engine, identity, declarations);
            }
            // What the element held before is the document thread's to release.
            Self::SetElementCustomPropertyData(node, handed) => {
                let retired = engine.set_element_custom_property_data(node, handed);
                engine.host.retired_custom_property_data.extend(retired);
            }
            Self::SetPseudoElementCustomPropertyData(node, pseudo, handed) => {
                let retired = engine.set_pseudo_element_custom_property_data(node, pseudo, handed);
                engine.host.retired_custom_property_data.extend(retired);
            }
        }
    }
}

/// A write the main thread makes to its document's style engine, which waits in the engine's home for whoever reaches
/// the engine next to apply first ([`StyleEngineInputHandle::send`]), in the order the main thread made it.
pub(crate) enum StyleChange {
    /// The style inputs the host recorded since the last transaction: DOM tree insertions, removals and moves, element
    /// arrivals, class, ID and attribute features, element states, inline style and presentational hint declarations,
    /// and the host facts they read. The engine applies them as one batch, which is how its invalidation sees them.
    /// Boxed, as there is one per transaction and a drain sends several engine changes per row beside it.
    Inputs(Box<InputForPass>),
    Engine(EngineChange),
}

impl StyleChange {
    /// What the change may leave the engine holding for its next style transaction, where it holds `held`.
    pub(crate) fn leaves(&self, held: PendingFacts) -> PendingFacts {
        match self {
            // The host's facts that go with them may name a size container to evaluate after layout.
            Self::Inputs(_) => PendingFacts::ELEMENT_INPUT.union(PendingFacts::SIZE_CONTAINERS_AFTER_LAYOUT),
            Self::Engine(change) => change.leaves(held),
        }
    }

    /// Applies the change, which the main thread took to leave `leaves`, to `engine`.
    pub(crate) fn apply(self, engine: &mut StyleEngine, leaves: PendingFacts) {
        // What the main thread cannot follow of the deferred inputs, it adopts from the engine.
        let followed = super::bridge::DeferredInputs::follows(&self, leaves);
        match self {
            Self::Inputs(inputs) => inputs.apply(engine),
            Self::Engine(change) => change.apply(engine),
        }
        engine.host.deferred_element_style_inputs_moved |= !followed;
    }
}

/// A read of a document's style engine, or a write that lends the engine what the host owns for the call, which the
/// render owner answers. What its pointers point at is the main thread's, which waits for the answer. Each variant is
/// the FFI entry of its name (`style_engine_match_element` for [`Self::MatchElement`]), whose documentation says what
/// it reads and writes, and the owner answers it with that entry's body.
pub(crate) enum StyleQuery {
    /// Gives up the `@keyframes` row of a shadow root's scope, which is on its way out.
    /// A read the boundary specification generates.
    Boundary(BoundaryRead),
    /// A style read the host answers synchronously, as a CSSOM read does.
    ReadDemand(super::bridge::RecordDemand),
    AssignedStyleRecord {
        node: u32,
        pseudo_kind: u8,
    },
    DeclaredOnlyRecord {
        subject: u32,
        facts: u32,
        hint_kind: FfiElementDeclarationKind,
        hints: *const c_void,
        hint_count: usize,
        inline_block: *const c_void,
    },
    NativeRuleTarget {
        rule: u32,
        result: *mut FfiNativeRuleTarget,
    },
    SampleInstalledRecords {
        records: *const super::bridge::FfiInstalledRecord,
        count: usize,
        samples: *mut super::bridge::FfiRowSampledInPass,
        layout_arena: *mut c_void,
    },
    SampledCustomPropertyEnvironmentOwner {
        environment: u64,
        node: *mut u32,
        pseudo_kind: *mut u8,
    },
    BorrowEngineCustomPropertyEnvironment {
        identity: u64,
        parent: *mut u64,
    },
    RandomSharingAbsolutize {
        value: *const crate::css::style_value::StyleValueData,
        length: *const c_void,
        node: u32,
    },
    TransitionLengthResolutionContext {
        style_record: u64,
        context: *mut crate::css::animation::FfiAnimationLengthResolutionContext,
    },
    SubstituteCompositorKeyframeValue {
        custom_property_store: *const std::ffi::c_void,
        property_id: u16,
        value: *const crate::css::style_value::StyleValueData,
    },
    InstallSampledCustomPropertyEnvironment {
        node: u32,
        pseudo_kind: u8,
        environment: u64,
    },
    DecideTransitionStepForInstalledRecord {
        node: u32,
        pseudo_kind: u8,
        before_change_style_record: u64,
        installed_style_record: u64,
        layout_arena: *mut c_void,
    },
    RemoveComputedPseudo {
        node: u32,
        pseudo_kind: u8,
    },
    PublishComputedGroups {
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
    },
    AppliedStyleReactionsDeriveInput {
        node: u32,
        applied: *const FfiAppliedStyleReaction,
        count: usize,
    },
    GrantStyleNodes {
        elements: *mut u32,
        element_count: usize,
        texts: *mut u32,
        text_count: usize,
    },
    DecideTransitions {
        before_style_record: u64,
        after_longhand_table: *const c_void,
        after_animated_overlay: *const c_void,
        input: *mut crate::css::transition::FfiTransitionInput,
        actions: *mut crate::css::transition::FfiTransitionAction,
    },
    /// Removes the native rules of `count` identities in order, and writes the engine id plus one each had as it went,
    /// or 0 for one gone already.
    /// The engine's id of the native rule of `identity` plus one, or zero where it holds none.
    NativeRuleId {
        identity: u64,
    },
    TakePseudoElementSampledInPass {
        node: u32,
        pseudo_kind: u8,
    },
    /// Takes the animation plan a row left, which the main thread's copy may not follow.
    TakeSettledAnimationPlan {
        node: StyleNodeID,
        pseudo_kind: u8,
    },
    /// A record as a published value, which the drain installs.
    PublishStyleRecord {
        style_record: u64,
    },
    /// Prepares a style pass the main thread submits: grants the identities its input asks for, begins its
    /// transaction with the inputs the host froze, and takes what the pass takes along.
    PrepareStylePass {
        computation_inputs: super::bridge::FfiDocumentStyleComputationInputs,
        layout_arena: *mut c_void,
        grant: super::bridge::StyleNodeGrant,
    },
}

/// The answer to a [`StyleQuery`], of the variant the query asks for.
pub(crate) enum StyleAnswer {
    None,
    Bool(bool),
    U32(u32),
    U64(u64),
    Usize(usize),
    /// A pointer into what the engine or the host owns, as the query's reader says.
    Pointer(*const c_void),
    RowSampled(super::bridge::FfiRowSampledInPass),
    RecordDelta(super::bridge::FfiStyleRecordDelta),
    AnimationPlan(Option<super::bridge::AnimationPlanForHost>),
    PreparedStylePass(super::bridge::PreparedStylePass),
    RecordDemand(super::bridge::FfiRecordDemandAnswer),
}

impl From<BoundaryResult> for StyleAnswer {
    fn from(result: BoundaryResult) -> Self {
        match result {
            BoundaryResult::Bool(value) => Self::Bool(value),
            BoundaryResult::U64(value) => Self::U64(value),
            BoundaryResult::Usize(value) => Self::Usize(value),
        }
    }
}

/// The value a generated boundary read answers.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BoundaryResult {
    Bool(bool),
    U64(u64),
    Usize(usize),
}

impl StyleAnswer {
    pub(crate) fn is(self) -> bool {
        match self {
            Self::Bool(value) => value,
            _ => {
                debug_assert!(false, "a yes-or-no read is answered with yes or no");
                false
            }
        }
    }

    pub(crate) fn u32(self) -> u32 {
        match self {
            Self::U32(value) => value,
            _ => {
                debug_assert!(false, "a 32-bit read is answered with 32 bits");
                0
            }
        }
    }

    pub(crate) fn u64(self) -> u64 {
        match self {
            Self::U64(value) => value,
            _ => {
                debug_assert!(false, "a 64-bit read is answered with 64 bits");
                0
            }
        }
    }

    pub(crate) fn usize(self) -> usize {
        match self {
            Self::Usize(value) => value,
            _ => {
                debug_assert!(false, "a size read is answered with a size");
                0
            }
        }
    }

    pub(crate) fn pointer(self) -> *const c_void {
        match self {
            Self::Pointer(value) => value,
            _ => {
                debug_assert!(false, "a pointer read is answered with a pointer");
                std::ptr::null()
            }
        }
    }

    pub(crate) fn row_sampled(self) -> super::bridge::FfiRowSampledInPass {
        match self {
            Self::RowSampled(value) => value,
            _ => {
                debug_assert!(false, "a sample is answered with a sample");
                super::bridge::FfiRowSampledInPass::absent()
            }
        }
    }

    pub(crate) fn record_delta(self) -> super::bridge::FfiStyleRecordDelta {
        match self {
            Self::RecordDelta(value) => value,
            _ => {
                debug_assert!(false, "a record move is answered with a record move");
                super::bridge::FfiStyleRecordDelta::default()
            }
        }
    }

    pub(crate) fn animation_plan(self) -> Option<super::bridge::AnimationPlanForHost> {
        match self {
            Self::AnimationPlan(value) => value,
            _ => {
                debug_assert!(false, "an animation plan is answered with an animation plan");
                None
            }
        }
    }

    pub(crate) fn prepared_style_pass(self) -> super::bridge::PreparedStylePass {
        match self {
            Self::PreparedStylePass(value) => value,
            _ => {
                debug_assert!(
                    false,
                    "a style pass preparation is answered with what the pass takes along"
                );
                Box::default()
            }
        }
    }

    pub(crate) fn record_demand(self) -> super::bridge::FfiRecordDemandAnswer {
        match self {
            Self::RecordDemand(value) => value,
            _ => super::bridge::FfiRecordDemandAnswer {
                unanswered: true,
                ..super::bridge::FfiRecordDemandAnswer::absent()
            },
        }
    }
}

impl StyleQuery {
    /// Answers the query from `engine`, on the owner.
    fn answer(self, engine: &mut StyleEngine, arena: &LayoutNodeArena) -> StyleAnswer {
        match self {
            Self::Boundary(read) => read.answer(engine).into(),
            // A read answered where there is no owner runs as the owner would run it: on the stage thread, whose
            // stack a style computation needs.
            Self::ReadDemand(demand) => {
                let node = demand.node;
                let answering = &mut *engine;
                let answer = crate::stage_thread::run_stage(move || demand.answer(answering)).into_ffi();
                // The host installs the pseudo-element records it answers beside the element's as it installs a
                // transaction's, comparing each against its box.
                if let Some(node) = StyleNodeID::from_raw(node) {
                    let record = &answer.record;
                    let mut verdicts: SmallVec<[_; super::bridge::RETRY_PSEUDO_RECORD_SLOTS]> = (0_u8..)
                        .zip(record.pseudo_records)
                        .filter(|&(pseudo_kind, pseudo_record)| {
                            pseudo_record != 0 && record.pseudo_records_present & (1 << pseudo_kind) != 0
                        })
                        .map(|(pseudo_kind, pseudo_record)| ((node, pseudo_kind), (pseudo_record, 0)))
                        .collect();
                    super::bridge::decide_content_counter_styles(arena, &mut verdicts);
                    for (row, verdict) in verdicts {
                        engine.host.content_counter_style_verdicts.insert(row, verdict);
                    }
                }
                StyleAnswer::RecordDemand(answer)
            }
            Self::AssignedStyleRecord { node, pseudo_kind } => StyleAnswer::U64(unsafe {
                crate::css::style::bridge::owner_assigned_style_record(engine, node, pseudo_kind)
            }),
            Self::DeclaredOnlyRecord {
                subject,
                facts,
                hint_kind,
                hints,
                hint_count,
                inline_block,
            } => StyleAnswer::Pointer(unsafe {
                crate::css::style::bridge::owner_declared_only_record(
                    engine,
                    subject,
                    facts,
                    hint_kind,
                    hints,
                    hint_count,
                    inline_block,
                )
            }),
            Self::NativeRuleTarget { rule, result } => {
                StyleAnswer::Bool(unsafe { crate::css::style::bridge::owner_native_rule_target(engine, rule, result) })
            }
            Self::SampleInstalledRecords {
                records,
                count,
                samples,
                layout_arena,
            } => {
                unsafe {
                    crate::css::style::bridge::owner_sample_installed_records(
                        engine,
                        records,
                        count,
                        samples,
                        layout_arena,
                    );
                };
                StyleAnswer::None
            }
            Self::SampledCustomPropertyEnvironmentOwner {
                environment,
                node,
                pseudo_kind,
            } => StyleAnswer::Bool(unsafe {
                crate::css::style::bridge::owner_sampled_custom_property_environment_owner(
                    engine,
                    environment,
                    node,
                    pseudo_kind,
                )
            }),
            Self::BorrowEngineCustomPropertyEnvironment { identity, parent } => StyleAnswer::Pointer(unsafe {
                crate::css::style::bridge::owner_borrow_engine_custom_property_environment(engine, identity, parent)
            }),
            Self::RandomSharingAbsolutize { value, length, node } => StyleAnswer::Pointer(
                unsafe { crate::css::absolutize::owner_random_sharing_absolutize(Some(engine), value, length, node) }
                    .cast(),
            ),
            Self::TransitionLengthResolutionContext { style_record, context } => StyleAnswer::Bool(unsafe {
                crate::css::transition::owner_transition_length_resolution_context(engine, style_record, context)
            }),
            Self::SubstituteCompositorKeyframeValue {
                custom_property_store,
                property_id,
                value,
            } => StyleAnswer::Pointer(
                unsafe {
                    crate::css::animation::owner_substitute_compositor_keyframe_value(
                        engine,
                        custom_property_store,
                        property_id,
                        value,
                    )
                }
                .cast(),
            ),
            Self::InstallSampledCustomPropertyEnvironment {
                node,
                pseudo_kind,
                environment,
            } => StyleAnswer::Bool(unsafe {
                crate::css::style::bridge::owner_install_sampled_custom_property_environment(
                    engine,
                    node,
                    pseudo_kind,
                    environment,
                )
            }),
            Self::DecideTransitionStepForInstalledRecord {
                node,
                pseudo_kind,
                before_change_style_record,
                installed_style_record,
                layout_arena,
            } => StyleAnswer::RowSampled(unsafe {
                crate::css::style::bridge::owner_decide_transition_step_for_installed_record(
                    engine,
                    node,
                    pseudo_kind,
                    before_change_style_record,
                    installed_style_record,
                    layout_arena,
                )
            }),
            Self::RemoveComputedPseudo { node, pseudo_kind } => StyleAnswer::RecordDelta(unsafe {
                crate::css::style::bridge::owner_remove_computed_pseudo(engine, node, pseudo_kind)
            }),
            Self::PublishComputedGroups {
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
            } => StyleAnswer::RecordDelta(unsafe {
                crate::css::style::bridge::owner_publish_computed_groups(
                    engine,
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
                )
            }),
            Self::AppliedStyleReactionsDeriveInput { node, applied, count } => StyleAnswer::Bool(unsafe {
                crate::css::style::bridge::owner_applied_style_reactions_derive_input(engine, node, applied, count)
            }),
            Self::GrantStyleNodes {
                elements,
                element_count,
                texts,
                text_count,
            } => {
                unsafe {
                    crate::css::style::bridge::owner_grant_style_nodes(
                        engine,
                        elements,
                        element_count,
                        texts,
                        text_count,
                    );
                };
                StyleAnswer::None
            }
            Self::DecideTransitions {
                before_style_record,
                after_longhand_table,
                after_animated_overlay,
                input,
                actions,
            } => {
                unsafe {
                    crate::css::transition::owner_decide_transitions(
                        engine,
                        before_style_record,
                        after_longhand_table,
                        after_animated_overlay,
                        input,
                        actions,
                    );
                }
                StyleAnswer::None
            }
            Self::NativeRuleId { identity } => {
                StyleAnswer::U32(engine.native_rule_id(identity).map_or(0, |id| id.0 + 1))
            }
            Self::TakePseudoElementSampledInPass { node, pseudo_kind } => StyleAnswer::RowSampled(unsafe {
                crate::css::style::bridge::owner_take_pseudo_element_sampled_in_pass(engine, node, pseudo_kind)
            }),
            Self::TakeSettledAnimationPlan { node, pseudo_kind } => StyleAnswer::AnimationPlan(
                crate::css::style::bridge::owner_take_settled_animation_plan(engine, node, pseudo_kind),
            ),
            Self::PublishStyleRecord { style_record } => StyleAnswer::Pointer(
                crate::css::style::bridge::owner_publish_style_record(engine, style_record),
            ),
            Self::PrepareStylePass {
                computation_inputs,
                layout_arena,
                grant,
            } => StyleAnswer::PreparedStylePass(unsafe {
                super::bridge::owner_prepare_style_pass(engine, computation_inputs, layout_arena, grant)
            }),
        }
    }

    /// The answer the main thread goes on with where the owner panicked answering: nothing answers the query again.
    fn unanswered(&self) -> StyleAnswer {
        match self {
            Self::Boundary(read) => read.unanswered().into(),
            _ => StyleAnswer::None,
        }
    }
}

/// A read only DevTools and Internals make of a document's style engine, which the render owner answers as it answers
/// a [`StyleQuery`]. No rendering update asks one.
pub(crate) enum DevToolsStyleQuery {
    MatchElement {
        node: u32,
        out: *mut FfiRuleMatch,
        capacity: usize,
        compact_for_cascade: bool,
    },
    Counters {
        values: *mut u64,
        count: usize,
    },
    SizeQueryContainerScanVisits {
        reset: bool,
    },
    HasSuspendedStylePass,
}

impl DevToolsStyleQuery {
    /// Answers the query from `engine`, on the owner.
    fn answer(self, engine: &mut StyleEngine) -> StyleAnswer {
        match self {
            Self::MatchElement {
                node,
                out,
                capacity,
                compact_for_cascade,
            } => StyleAnswer::Usize(unsafe {
                super::bridge::owner_match_element(engine, node, out, capacity, compact_for_cascade)
            }),
            Self::Counters { values, count } => {
                unsafe { super::bridge::owner_counters(engine, values, count) };
                StyleAnswer::None
            }
            Self::SizeQueryContainerScanVisits { reset } => {
                StyleAnswer::U64(engine.size_query_container_scan_visits(reset))
            }
            Self::HasSuspendedStylePass => StyleAnswer::Bool(engine.state.host.suspended_style_pass.is_some()),
        }
    }
}

/// What the main thread asks the owner: a style read, or a DevTools one.
#[allow(clippy::large_enum_variant)]
enum Question {
    Style(StyleQuery),
    DevTools(DevToolsStyleQuery),
}

impl Question {
    fn answer(self, engine: &mut StyleEngine, arena: &LayoutNodeArena) -> StyleAnswer {
        match self {
            Self::Style(query) => query.answer(engine, arena),
            Self::DevTools(query) => query.answer(engine),
        }
    }

    fn unanswered(&self) -> StyleAnswer {
        match self {
            Self::Style(query) => query.unanswered(),
            Self::DevTools(_) => StyleAnswer::None,
        }
    }
}

/// A query and, once the owner has answered it, its answer, which the main thread holds while it waits.
pub(crate) struct StyleQueryCell {
    query: Option<Question>,
    answer: Option<StyleAnswer>,
    /// The custom-property data the engine retired answering the query, and before it, which only the main thread
    /// releases: it drops the cell once the owner has answered.
    retired: Vec<super::inputs::RetainedCustomPropertyData>,
}

impl StyleQueryCell {
    #[cfg(test)]
    pub(crate) fn new(query: StyleQuery) -> Self {
        Self::asking(Question::Style(query))
    }

    fn asking(query: Question) -> Self {
        Self {
            query: Some(query),
            answer: None,
            retired: Vec::new(),
        }
    }

    /// The query as [`Query::Engine`] carries it to the owner, which the cell must outlive the answer of.
    pub(crate) fn for_owner(&mut self) -> StyleQueryRef {
        StyleQueryRef(NonNull::from(self))
    }
}

/// The query the main thread holds, as [`Query::Engine`] carries it to the owner.
#[derive(Clone, Copy, Debug)]
pub(crate) struct StyleQueryRef(NonNull<StyleQueryCell>);

// SAFETY: The main thread waits for the owner's answer, keeping the cell live, and reaches it again only once the owner
// has answered.
unsafe impl Send for StyleQueryRef {}

impl StyleQueryRef {
    /// Answers the query from `engine`, on the owner, whose arena of the document is `arena`.
    ///
    /// # Safety
    ///
    /// The main thread must wait for the answer, with the cell live.
    pub(crate) unsafe fn answer(self, engine: &mut StyleEngine, arena: &LayoutNodeArena) {
        // SAFETY: Guaranteed by the caller.
        let cell = unsafe { &mut *self.0.as_ptr() };
        if let Some(query) = cell.query.take() {
            cell.answer = Some(query.answer(engine, arena));
        }
        cell.retired = std::mem::take(&mut engine.host.retired_custom_property_data);
    }
}

/// Leaves `change` for whoever reaches `engine` next to apply first. The main thread goes on at once.
pub(crate) fn send(engine: StyleEngineInputHandle, change: EngineChange) {
    engine.send(StyleChange::Engine(change));
}

/// Asks the owner of `engine`'s document `query`, and waits for the answer, which comes after every change the thread
/// sent before. `entry` names the door the main thread took.
pub(crate) fn ask(engine: StyleEngineHandle, entry: &'static str, query: StyleQuery) -> StyleAnswer {
    engine.bring_home(entry);
    ask_document(engine.document(), entry, Question::Style(query))
}

/// Leaves the change that gives up the `@keyframes` row of a shadow root's scope, from a garbage collection's
/// finalizer, which must not wait for the engine (the main thread may hold the loan that would send it home) and must
/// not borrow the home's answers (the collection may run while the main thread does). The change leaves nothing the
/// answers follow.
pub(crate) fn unpublish_tree_scope_keyframes_from_finalizer(
    engine: StyleEngineInputHandle,
    tree_scope: super::tree::TreeScopeID,
    shadow_root_identity: usize,
) {
    engine.send_unfollowed(StyleChange::Engine(EngineChange::SetTreeScopeAnimationKeyframes {
        tree_scope,
        shadow_root_identity,
        keyframes: super::animations::TreeScopeKeyframes::none(),
    }));
}

/// Asks the owner of `engine`'s document the DevTools read `query`, as [`ask`] asks a style read.
pub(crate) fn ask_devtools(engine: StyleEngineHandle, entry: &'static str, query: DevToolsStyleQuery) -> StyleAnswer {
    engine.bring_home(entry);
    ask_document(engine.document(), entry, Question::DevTools(query))
}

fn ask_document(document: DocumentId, entry: &'static str, query: Question) -> StyleAnswer {
    let mut cell = StyleQueryCell::asking(query);
    let answered = crate::render_owner::ask_engine(document, Query::Engine(cell.for_owner()));
    let StyleQueryCell { query, answer, retired } = cell;
    // What the owner retired is released here, on the main thread.
    drop(retired);
    match (answered, answer, query) {
        (Answer::Engine(EngineAnswered::Answered), Some(answer), _) => answer,
        (_, _, query) => {
            debug_assert!(false, "the render owner left the style read {entry} unanswered");
            query.map_or(StyleAnswer::None, |query| query.unanswered())
        }
    }
}
