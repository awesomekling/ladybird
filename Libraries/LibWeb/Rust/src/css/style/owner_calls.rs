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
use super::bridge::{BoundaryRead, BoundaryWrite, FfiElementDeclarationKind, FfiNativeRuleTarget, FfiRuleMatch};
use super::engine_home::{StyleEngineHandle, StyleEngineInputHandle};
use crate::render_owner::{Answer, DocumentId, EngineAnswered, Query};
use std::ffi::c_void;
use std::ptr::NonNull;

/// A write to a document's style engine, which the render owner applies.
#[derive(Debug)]
pub(crate) enum EngineChange {
    /// A write the boundary specification generates.
    #[expect(
        dead_code,
        reason = "the generated writes go to the owner with the style inputs recorded then read back"
    )]
    Boundary(BoundaryWrite),
}

impl EngineChange {
    /// Applies the change to `engine`, on the owner.
    pub(crate) fn apply(self, engine: &mut StyleEngine) {
        match self {
            Self::Boundary(write) => write.apply(engine),
        }
    }
}

/// A read of a document's style engine, which the render owner answers. What its pointers point at is the main
/// thread's, which waits for the answer; each variant's reader says what it reads and writes.
pub(crate) enum StyleQuery {
    /// Nothing but the changes before it: the main thread is about to reach the engine itself.
    ApplyChanges,
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
    #[expect(
        dead_code,
        reason = "the generated reads of 32 bits go to the owner with the transaction drivers"
    )]
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
}

impl StyleQueryCell {
    pub(crate) fn new(query: StyleQuery) -> Self {
        Self {
            query: Some(query),
            answer: None,
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
    }
}

/// The document whose render state links `engine`, if its render owner runs its style.
fn owning_document(engine: StyleEngineHandle) -> Option<DocumentId> {
    Some(engine.document()).filter(|document| crate::render_owner::runs_style_of(*document))
}

/// Sends `change` to the owner of `engine`'s document, which applies it before the next unit or query that reaches
/// the engine. `entry` names the door the main thread took, for the style seal.
#[expect(
    dead_code,
    reason = "the generated writes go to the owner with the style inputs recorded then read back"
)]
pub(crate) fn send(engine: StyleEngineInputHandle, entry: &'static str, change: EngineChange) {
    let handle = engine.home();
    handle.bring_home(entry);
    super::seal::note_engine_call(entry);
    let Some(document) = owning_document(handle) else {
        // SAFETY: No owner reaches an engine no document's render state links: the calling thread holds it alone.
        change.apply(unsafe { handle.enter(entry) });
        return;
    };
    crate::render_owner::send_change(
        engine.through_render_inputs(),
        document,
        crate::render_owner::Change::Engine(change),
    );
}

/// Asks the owner of `engine`'s document `query`, and waits for the answer, which comes after every change the thread
/// sent before. `entry` names the door the main thread took, for the style seal.
pub(crate) fn ask(engine: StyleEngineHandle, entry: &'static str, query: StyleQuery) -> StyleAnswer {
    engine.bring_home(entry);
    super::seal::note_engine_call(entry);
    let Some(document) = owning_document(engine) else {
        // SAFETY: No owner reaches an engine no document's render state links: the calling thread holds it alone.
        return query.answer(unsafe { engine.enter(entry) });
    };
    ask_document(engine, document, entry, query)
}

fn ask_document(
    engine: StyleEngineHandle,
    document: DocumentId,
    entry: &'static str,
    query: StyleQuery,
) -> StyleAnswer {
    let mut cell = StyleQueryCell::new(query);
    let answered = crate::render_owner::ask_owner(document, Query::Engine(cell.for_owner()));
    let StyleQueryCell { query, answer } = cell;
    match (answered, answer, query) {
        (Answer::Engine(EngineAnswered::Answered), Some(answer), _) => answer,
        // Where the owner holds no engine of the document (a test holds the run the query would queue behind), the
        // calling thread answers as the owner would.
        (Answer::Engine(EngineAnswered::LeftToHost), _, Some(query)) => {
            // SAFETY: The owner does not reach the engine meanwhile: the thread holds the run it would reach it in.
            query.answer(unsafe { engine.enter(entry) })
        }
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
    let answered = crate::render_owner::ask_owner(document, Query::Engine(cell.for_owner()));
    debug_assert!(
        !matches!(answered, Answer::Engine(EngineAnswered::Unanswered)),
        "the render owner panicked applying style changes"
    );
}
