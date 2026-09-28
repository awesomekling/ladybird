/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What the main thread asks of the style engine of its document, which the render owner owns.
//!
//! The main thread does not enter the engine. A write is an [`EngineChange`] it sends as a change of the document: the
//! owner applies it, in order with the style inputs, before the next unit or query that reaches the engine. A read is
//! a [`StyleQuery`] it asks: the owner answers it in one round trip, after every change the main thread sent before
//! it, into the query the main thread holds while it waits.
//!
//! An engine no document's render state links (one a unit test or the replay tool makes) has no owner: the calling
//! thread applies and answers right here, as the owner would.

use super::StyleEngine;
use super::bridge::{
    BoundaryRead, BoundaryWrite, FfiAppliedStyleReaction, FfiElementDeclarationKind, FfiNativeRuleTarget,
    FfiPublishedAnimationCustomDeclaration, FfiPublishedAnimationDeclaration, FfiPublishedAnimationEffect,
    FfiPublishedAnimationKeyframe, FfiPublishedLinearEasingPoint, FfiPublishedTransition, FfiRuleMatch,
};
use super::engine_home::{StyleEngineHandle, StyleEngineInputHandle};
use crate::render_owner::{Answer, DocumentId, EngineAnswered, Query};
use std::ffi::c_void;
use std::ptr::NonNull;

/// A write to a document's style engine, which the render owner applies. A write that lends the engine what the host
/// owns for the call is a [`StyleQuery`] instead, which the main thread waits for.
#[derive(Debug)]
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
}

impl EngineChange {
    /// Applies the change to `engine`, on the owner.
    pub(crate) fn apply(self, engine: &mut StyleEngine) {
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
        }
    }
}

/// A read of a document's style engine, or a write that lends the engine what the host owns for the call, which the
/// render owner answers. What its pointers point at is the main thread's, which waits for the answer. Each variant is
/// the FFI entry of its name (`style_engine_match_element` for [`Self::MatchElement`]), whose documentation says what
/// it reads and writes, and the owner answers it with that entry's body.
pub(crate) enum StyleQuery {
    /// Nothing but the changes before it: the main thread is about to reach the engine itself.
    ApplyChanges,
    /// Gives up the `@keyframes` row of a shadow root's scope, which is on its way out.
    UnpublishTreeScopeAnimationKeyframes {
        tree_scope: u32,
        shadow_root_identity: usize,
    },
    /// A read the boundary specification generates.
    Boundary(BoundaryRead),
    /// A style read the host answers synchronously, as a CSSOM read does.
    ReadDemand(super::bridge::RecordDemand),
    MatchElement {
        node: u32,
        out: *mut FfiRuleMatch,
        capacity: usize,
        compact_for_cascade: bool,
    },
    ConsumePublishedMatchAnswer {
        node: u32,
        out: *mut FfiRuleMatch,
        capacity: usize,
    },
    ElementRecordDamage {
        node: u32,
        old_style_record: u64,
        new_style_record: u64,
    },
    PseudoElementRecordDamage {
        node: u32,
        pseudo_kind: u8,
        old_style_record: u64,
        new_style_record: u64,
        originating_style_record: u64,
        counter_styles_changed: bool,
    },
    Counter {
        index: usize,
        out_value: *mut u64,
        out_name_length: *mut usize,
    },
    SizeQueryContainerScanVisits {
        reset: bool,
    },
    HasSizeContainersNeedingEvaluationAfterLayout,
    HasSuspendedStylePass,
    ElementCustomPropertyData {
        node: u32,
        identity: *mut u64,
    },
    PseudoElementCustomPropertyData {
        node: u32,
        pseudo: u8,
        identity: *mut u64,
    },
    PseudoElementsWithCustomPropertyData {
        node: u32,
    },
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
    NativeRuleId {
        identity: u64,
    },
    NativeRuleTarget {
        rule: u32,
        result: *mut FfiNativeRuleTarget,
    },
    NativeRuleSuccessor {
        sheet: *const c_void,
        identity: u64,
    },
    SampleInstalledRecord {
        node: u32,
        pseudo_kind: u8,
        style_record: u64,
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
    SetElementCustomPropertyData {
        node: u32,
        data: *const c_void,
        store: *const c_void,
        environment: u64,
        is_animation_overlay: bool,
        declares: bool,
        animation_base: *const c_void,
        animation_base_store: *const c_void,
        animation_base_environment: u64,
    },
    SetPseudoElementCustomPropertyData {
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
    },
    InstallSampledCustomPropertyEnvironment {
        node: u32,
        pseudo_kind: u8,
        environment: u64,
    },
    SetElementTransitions {
        node: u32,
        slot: u8,
        transitions: *const FfiPublishedTransition,
        count: usize,
    },
    SetElementAnimationEffectDescriptions {
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
    },
    SetTreeScopeAnimationKeyframes {
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
    #[cfg(feature = "style-recording")]
    BenchmarkMarker {
        name: *const c_void,
        length: usize,
        is_ascii: bool,
    },
    /// Publishes the declarations the native rule of `identity` now holds, if the engine holds it.
    RuleDeclarationsChanged {
        identity: u64,
        declarations: Option<std::sync::Arc<crate::css::declaration_block::DeclarationBlockData>>,
    },
    /// Removes the native rules of `count` identities in order, and writes the engine id plus one each had as it went,
    /// or 0 for one gone already.
    RemoveNativeRules {
        identities: *const u64,
        ids: *mut u32,
        count: usize,
    },
    /// Compiles a sheet's rules into the engine, or replaces their selectors, as the walk says.
    Compile(crate::css::rule::compilation::OwnerCompilation),
    TakeContainerEffects {
        node: u32,
    },
    RegisterAnchorNames {
        node: u32,
        style_record: u64,
    },
    TakePseudoElementEnvironmentNamedInSettle {
        node: u32,
        pseudo_kind: u8,
    },
    TakePseudoElementSampledInPass {
        node: u32,
        pseudo_kind: u8,
    },
    TakePseudoElementTransitionStepDecidedInPass {
        node: u32,
        pseudo_kind: u8,
    },
    TakeRowSampledInPass {
        node: u32,
    },
    TakeSettledAnimationDefinitions {
        node: u32,
        pseudo_kind: u8,
    },
    TakeTransitionStepDecidedInPass {
        node: u32,
    },
    PublishFontFaceSnapshot {
        snapshot: *const c_void,
        memo: usize,
    },
    /// Publishes the anchor names registration moved to the layout arena whose handle is `arena`, or leaves them for
    /// an arena where it is null.
    PublishAnchorNames {
        arena: *mut c_void,
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
    RuleDeclarations(Option<super::bridge::PublishedRuleDeclarations>),
    ContainerEffects(super::bridge::FfiNativeContainerMatchResult),
    TransitionStep(super::bridge::FfiTransitionStepDecidedInPass),
    AnimationDefinitions(super::bridge::FfiSettledAnimationDefinitions),
    PreparedStylePass(super::bridge::PreparedStylePass),
    RecordDemand(super::bridge::FfiRecordDemandAnswer),
}

impl From<BoundaryResult> for StyleAnswer {
    fn from(result: BoundaryResult) -> Self {
        match result {
            BoundaryResult::Bool(value) => Self::Bool(value),
            BoundaryResult::U32(value) => Self::U32(value),
            BoundaryResult::U64(value) => Self::U64(value),
            BoundaryResult::Usize(value) => Self::Usize(value),
        }
    }
}

/// The value a generated boundary read answers.
#[derive(Clone, Copy, Debug)]
pub(crate) enum BoundaryResult {
    Bool(bool),
    U32(u32),
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

    pub(crate) fn rule_declarations(self) -> Option<super::bridge::PublishedRuleDeclarations> {
        match self {
            Self::RuleDeclarations(value) => value,
            _ => {
                debug_assert!(false, "a declaration edit is answered with what it published");
                None
            }
        }
    }

    pub(crate) fn container_effects(self) -> super::bridge::FfiNativeContainerMatchResult {
        match self {
            Self::ContainerEffects(value) => value,
            _ => {
                debug_assert!(false, "container effects are answered with container effects");
                super::bridge::FfiNativeContainerMatchResult::default()
            }
        }
    }

    pub(crate) fn transition_step(self) -> super::bridge::FfiTransitionStepDecidedInPass {
        match self {
            Self::TransitionStep(value) => value,
            _ => {
                debug_assert!(false, "a transition step is answered with a transition step");
                super::bridge::FfiTransitionStepDecidedInPass::absent()
            }
        }
    }

    pub(crate) fn animation_definitions(self) -> super::bridge::FfiSettledAnimationDefinitions {
        match self {
            Self::AnimationDefinitions(value) => value,
            _ => {
                debug_assert!(false, "animation definitions are answered with animation definitions");
                super::bridge::FfiSettledAnimationDefinitions::absent()
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
    fn answer(self, engine: &mut StyleEngine) -> StyleAnswer {
        match self {
            Self::ApplyChanges => StyleAnswer::None,
            Self::UnpublishTreeScopeAnimationKeyframes {
                tree_scope,
                shadow_root_identity,
            } => {
                super::animations::owner_unpublish_tree_scope_animation_keyframes(
                    engine,
                    tree_scope,
                    shadow_root_identity,
                );
                StyleAnswer::None
            }
            Self::Boundary(read) => read.answer(engine).into(),
            // A read answered where there is no owner runs as the owner would run it: on the stage thread, whose
            // stack a style computation needs.
            Self::ReadDemand(demand) => {
                StyleAnswer::RecordDemand(crate::stage_thread::run_stage(move || demand.answer(engine)).into_ffi())
            }
            Self::MatchElement {
                node,
                out,
                capacity,
                compact_for_cascade,
            } => StyleAnswer::Usize(unsafe {
                crate::css::style::bridge::owner_match_element(engine, node, out, capacity, compact_for_cascade)
            }),
            Self::ConsumePublishedMatchAnswer { node, out, capacity } => StyleAnswer::Usize(unsafe {
                crate::css::style::bridge::owner_consume_published_match_answer(engine, node, out, capacity)
            }),
            Self::ElementRecordDamage {
                node,
                old_style_record,
                new_style_record,
            } => StyleAnswer::U32(unsafe {
                crate::css::style::bridge::owner_element_record_damage(engine, node, old_style_record, new_style_record)
            }),
            Self::PseudoElementRecordDamage {
                node,
                pseudo_kind,
                old_style_record,
                new_style_record,
                originating_style_record,
                counter_styles_changed,
            } => StyleAnswer::U32(unsafe {
                crate::css::style::bridge::owner_pseudo_element_record_damage(
                    engine,
                    node,
                    pseudo_kind,
                    old_style_record,
                    new_style_record,
                    originating_style_record,
                    counter_styles_changed,
                )
            }),
            Self::Counter {
                index,
                out_value,
                out_name_length,
            } => StyleAnswer::Pointer(
                unsafe { crate::css::style::bridge::owner_counter(engine, index, out_value, out_name_length) }.cast(),
            ),
            Self::SizeQueryContainerScanVisits { reset } => StyleAnswer::U64(unsafe {
                crate::css::style::bridge::owner_size_query_container_scan_visits(engine, reset)
            }),
            Self::HasSizeContainersNeedingEvaluationAfterLayout => StyleAnswer::Bool(unsafe {
                crate::css::style::bridge::owner_has_size_containers_needing_evaluation_after_layout(engine)
            }),
            Self::HasSuspendedStylePass => {
                StyleAnswer::Bool(unsafe { crate::css::style::bridge::owner_has_suspended_style_pass(engine) })
            }
            Self::ElementCustomPropertyData { node, identity } => StyleAnswer::Pointer(unsafe {
                crate::css::style::bridge::owner_element_custom_property_data(engine, node, identity)
            }),
            Self::PseudoElementCustomPropertyData { node, pseudo, identity } => StyleAnswer::Pointer(unsafe {
                crate::css::style::bridge::owner_pseudo_element_custom_property_data(engine, node, pseudo, identity)
            }),
            Self::PseudoElementsWithCustomPropertyData { node } => StyleAnswer::U64(unsafe {
                crate::css::style::bridge::owner_pseudo_elements_with_custom_property_data(engine, node)
            }),
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
            Self::NativeRuleId { identity } => {
                StyleAnswer::U32(unsafe { crate::css::style::bridge::owner_native_rule_id(engine, identity) })
            }
            Self::NativeRuleTarget { rule, result } => {
                StyleAnswer::Bool(unsafe { crate::css::style::bridge::owner_native_rule_target(engine, rule, result) })
            }
            Self::NativeRuleSuccessor { sheet, identity } => StyleAnswer::U32(unsafe {
                crate::css::style::bridge::owner_native_rule_successor(engine, sheet, identity)
            }),
            Self::SampleInstalledRecord {
                node,
                pseudo_kind,
                style_record,
                layout_arena,
            } => StyleAnswer::RowSampled(unsafe {
                crate::css::style::bridge::owner_sample_installed_record(
                    engine,
                    node,
                    pseudo_kind,
                    style_record,
                    layout_arena,
                )
            }),
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
            Self::SetElementCustomPropertyData {
                node,
                data,
                store,
                environment,
                is_animation_overlay,
                declares,
                animation_base,
                animation_base_store,
                animation_base_environment,
            } => {
                unsafe {
                    crate::css::style::bridge::owner_set_element_custom_property_data(
                        engine,
                        node,
                        data,
                        store,
                        environment,
                        is_animation_overlay,
                        declares,
                        animation_base,
                        animation_base_store,
                        animation_base_environment,
                    );
                };
                StyleAnswer::None
            }
            Self::SetPseudoElementCustomPropertyData {
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
            } => {
                unsafe {
                    crate::css::style::bridge::owner_set_pseudo_element_custom_property_data(
                        engine,
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
                    );
                };
                StyleAnswer::None
            }
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
            Self::SetElementTransitions {
                node,
                slot,
                transitions,
                count,
            } => {
                unsafe {
                    crate::css::style::bridge::owner_set_element_transitions(engine, node, slot, transitions, count);
                };
                StyleAnswer::None
            }
            Self::SetElementAnimationEffectDescriptions {
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
            } => {
                unsafe {
                    crate::css::style::bridge::owner_set_element_animation_effect_descriptions(
                        engine,
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
                    );
                };
                StyleAnswer::None
            }
            Self::SetTreeScopeAnimationKeyframes {
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
            } => {
                unsafe {
                    crate::css::style::bridge::owner_set_tree_scope_animation_keyframes(
                        engine,
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
                    );
                };
                StyleAnswer::None
            }
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
            #[cfg(feature = "style-recording")]
            Self::BenchmarkMarker { name, length, is_ascii } => {
                unsafe { super::bridge::owner_record_benchmark_marker(engine, name, length, is_ascii) };
                StyleAnswer::None
            }
            Self::RuleDeclarationsChanged { identity, declarations } => StyleAnswer::RuleDeclarations(
                super::bridge::owner_rule_declarations_changed(engine, identity, declarations),
            ),
            Self::RemoveNativeRules { identities, ids, count } => {
                // SAFETY: The main thread lends both arrays, of `count` each, until it has the answer.
                let (identities, ids) = unsafe {
                    (
                        std::slice::from_raw_parts(identities, count),
                        std::slice::from_raw_parts_mut(ids, count),
                    )
                };
                for (identity, id) in identities.iter().zip(ids) {
                    *id = engine.native_rule_id(*identity).map_or(0, |rule| rule.0 + 1);
                    if *id != 0 {
                        super::bridge::operations::remove_rule(engine, *id);
                    }
                }
                StyleAnswer::None
            }
            Self::Compile(compilation) => {
                // SAFETY: The main thread waits for the answer, keeping what the walk points at live.
                unsafe { compilation.run(engine) };
                StyleAnswer::None
            }
            Self::TakeContainerEffects { node } => StyleAnswer::ContainerEffects(unsafe {
                crate::css::style::bridge::owner_take_container_effects(engine, node)
            }),
            Self::RegisterAnchorNames { node, style_record } => StyleAnswer::U32(u32::from(unsafe {
                crate::css::style::bridge::owner_register_anchor_names(engine, node, style_record)
            })),
            Self::TakePseudoElementEnvironmentNamedInSettle { node, pseudo_kind } => StyleAnswer::Bool(unsafe {
                crate::css::style::bridge::owner_take_pseudo_element_environment_named_in_settle(
                    engine,
                    node,
                    pseudo_kind,
                )
            }),
            Self::TakePseudoElementSampledInPass { node, pseudo_kind } => StyleAnswer::RowSampled(unsafe {
                crate::css::style::bridge::owner_take_pseudo_element_sampled_in_pass(engine, node, pseudo_kind)
            }),
            Self::TakePseudoElementTransitionStepDecidedInPass { node, pseudo_kind } => {
                StyleAnswer::TransitionStep(unsafe {
                    crate::css::style::bridge::owner_take_pseudo_element_transition_step_decided_in_pass(
                        engine,
                        node,
                        pseudo_kind,
                    )
                })
            }
            Self::TakeRowSampledInPass { node } => StyleAnswer::RowSampled(unsafe {
                crate::css::style::bridge::owner_take_row_sampled_in_pass(engine, node)
            }),
            Self::TakeSettledAnimationDefinitions { node, pseudo_kind } => StyleAnswer::AnimationDefinitions(unsafe {
                crate::css::style::bridge::owner_take_settled_animation_definitions(engine, node, pseudo_kind)
            }),
            Self::TakeTransitionStepDecidedInPass { node } => StyleAnswer::TransitionStep(unsafe {
                crate::css::style::bridge::owner_take_transition_step_decided_in_pass(engine, node)
            }),
            Self::PublishFontFaceSnapshot { snapshot, memo } => {
                unsafe { crate::css::style::bridge::owner_publish_font_face_snapshot(engine, snapshot, memo) };
                StyleAnswer::None
            }
            Self::PublishAnchorNames { arena } => {
                unsafe { crate::css::style::bridge::owner_publish_anchor_names(engine, arena) };
                StyleAnswer::None
            }
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

/// A query and, once the owner has answered it, its answer, which the main thread holds while it waits.
pub(crate) struct StyleQueryCell {
    query: Option<StyleQuery>,
    answer: Option<StyleAnswer>,
    /// The custom-property data the engine retired answering the query, and before it, which only the main thread
    /// releases: it drops the cell once the owner has answered.
    retired: Vec<super::inputs::RetainedCustomPropertyData>,
}

impl StyleQueryCell {
    pub(crate) fn new(query: StyleQuery) -> Self {
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
    /// Answers the query from `engine`, on the owner.
    ///
    /// # Safety
    ///
    /// The main thread must wait for the answer, with the cell live.
    pub(crate) unsafe fn answer(self, engine: &mut StyleEngine) {
        // SAFETY: Guaranteed by the caller.
        let cell = unsafe { &mut *self.0.as_ptr() };
        if let Some(query) = cell.query.take() {
            cell.answer = Some(query.answer(engine));
        }
        cell.retired = std::mem::take(&mut engine.host.retired_custom_property_data);
    }
}

/// The document whose render state links `engine`, if its render owner runs its style.
pub(crate) fn owning_document(engine: StyleEngineHandle) -> Option<DocumentId> {
    Some(engine.document()).filter(|document| crate::render_owner::runs_style_of(*document))
}

/// Runs `run` with `engine` on the calling thread, which holds it alone: an engine no document's render state links
/// (a unit test's, the replay tool's, a document's that has none) has no owner to ask. `entry` names the door the
/// calling thread took, for the style seal.
pub(crate) fn without_owner<R>(
    engine: StyleEngineHandle,
    entry: &'static str,
    run: impl FnOnce(&mut StyleEngine) -> R,
) -> R {
    debug_assert!(
        owning_document(engine).is_none(),
        "the engine of a document's render state is the owner's to reach"
    );
    // SAFETY: No owner reaches an engine no document's render state links.
    let engine = unsafe { engine.enter(entry) };
    super::seal::note_engine_call(entry);
    run(engine)
}

/// Sends `change` to the owner of `engine`'s document, which applies it before the next unit or query that reaches
/// the engine. `entry` names the door the main thread took, for the style seal.
pub(crate) fn send(engine: StyleEngineInputHandle, entry: &'static str, change: EngineChange) {
    let handle = engine.home();
    let Some(document) = owning_document(handle) else {
        without_owner(handle, entry, |engine| change.apply(engine));
        return;
    };
    handle.bring_home(entry);
    super::seal::note_engine_call(entry);
    crate::render_owner::send_change(
        engine.through_render_inputs(),
        document,
        crate::render_owner::Change::Engine(change),
    );
}

/// Asks the owner of `engine`'s document `query`, and waits for the answer, which comes after every change the thread
/// sent before. `entry` names the door the main thread took, for the style seal.
pub(crate) fn ask(engine: StyleEngineHandle, entry: &'static str, query: StyleQuery) -> StyleAnswer {
    let Some(document) = owning_document(engine) else {
        return without_owner(engine, entry, |engine| query.answer(engine));
    };
    engine.bring_home(entry);
    super::seal::note_engine_call(entry);
    ask_document(document, entry, query)
}

/// Like [`ask`], for a garbage collection's finalizer, which must not wait for the engine: the main thread may
/// hold the loan that would send it home. The owner, which answers, never waits for the main thread.
pub(crate) fn ask_from_finalizer(engine: StyleEngineHandle, entry: &'static str, query: StyleQuery) -> StyleAnswer {
    let Some(document) = owning_document(engine) else {
        return without_owner(engine, entry, |engine| query.answer(engine));
    };
    super::seal::note_engine_call(entry);
    ask_document(document, entry, query)
}

/// Like [`ask`], for a read of what a published record holds only, which goes on while the install of the batch a
/// stage published is still owed.
pub(crate) fn ask_records(engine: StyleEngineHandle, entry: &'static str, query: StyleQuery) -> StyleAnswer {
    let Some(document) = owning_document(engine) else {
        return without_owner(engine, entry, |engine| query.answer(engine));
    };
    engine.bring_home_to_read_records(entry);
    super::seal::note_engine_call(entry);
    ask_document(document, entry, query)
}

fn ask_document(document: DocumentId, entry: &'static str, query: StyleQuery) -> StyleAnswer {
    let mut cell = StyleQueryCell::new(query);
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

/// On the main thread, about to reach `engine` itself at a door still left: has the owner apply the changes to the
/// engine the thread sent before, which the thread's reach comes after.
pub(crate) fn apply_changes_before_main_reaches(engine: StyleEngineHandle) {
    let document = engine.document();
    if !crate::render_owner::has_unapplied_style_changes(document) {
        return;
    }
    let mut cell = StyleQueryCell::new(StyleQuery::ApplyChanges);
    let answered = crate::render_owner::ask_engine(document, Query::Engine(cell.for_owner()));
    debug_assert!(
        !matches!(answered, Answer::Engine(EngineAnswered::Unanswered)),
        "the render owner panicked applying style changes"
    );
}
