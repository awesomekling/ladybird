/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The render owner: the Rendering thread owns each document's render state.
//!
//! A document's [`RenderState`] (its layout arena, and through it its style engine, paint preparation and animation
//! sampling state) is constructed on the Rendering thread and dropped there. The main thread holds a [`DocumentId`]
//! and has three typed ways in, none of which takes a closure:
//!
//! - a [`Change`]: owned `Send` data, numbered per document ([`ChangeSeq`]), fire and forget. The owner queues it
//!   and applies it, in order, at the next point that needs it: a unit of a rendering update, or a query.
//! - a [`RenderingUpdate`]: the stages of a frame, which the owner runs as units ([`FrameUnit`]) and answers with
//!   [`FrameEffects`], the typed results the main thread applies where it takes the frame in.
//! - a [`Query`]: a question about the document as of the changes sent before it, answered in one round trip
//!   ([`Answer`]). The owner serves a waiting query between units: a rendering update in progress serves the
//!   messages that arrived meanwhile after each unit (style, the layout rounds, paint preparation), so a query waits
//!   for one unit of another document's update at most, and the update goes on after it. A query that needs the
//!   document laid out first goes with the job of the layout frame that lays it out, which answers it as it ends.
//!
//! Every message reaches the owner through one FIFO ([`ToOwner`]), so a document's changes, its rendering updates,
//! its queries and its destruction arrive in the order the main thread sent them. The owner never joins the main
//! thread, and a job it runs never hands control back to the main thread before the job is over: the main thread
//! sends what the job reads before it, and applies the typed effects the job leaves after it. The style computation
//! is the owner's: every style transaction the main thread waits for runs on the owner ([`ToOwner::Style`]), with the
//! engine the document's render state links, and so do the rounds of a layout frame ([`ToOwner::Layout`]), each job
//! of which runs every round it starts to its end. Where a rendering step still needs the main thread (the host steps
//! of a style update around its passes, what paying a tree build's host half leaves), the main thread runs it as its
//! own after the job that left it, and sends the next job.
//!
//! During the port the main thread still reaches the arena and the style engine directly through the handles the
//! owner gives out when it creates the state ([`FfiRenderDocument`]); those doors are what the flip deletes.

use crate::css::style::bridge::InputForPass;
use crate::css::style::tree::StyleNodeID;
use crate::layout::node_data::{NodeKind, NodeSlotId};
use crate::layout::{ArenaHandle, FfiCssPixelRect, LayoutNodeArena};
use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::Sender;

/// The main thread's name for one document's render state. The main thread mints it, so naming a new document
/// needs no round trip.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct DocumentId(pub u64);

impl DocumentId {
    fn mint() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }

    pub(crate) fn is_valid(self) -> bool {
        self.0 != 0
    }
}

/// The number of a change within its document's stream. The first change a document sends is 1; 0 names the point
/// before any change.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub(crate) struct ChangeSeq(u64);

/// One write to a document's render state, as owned data the owner applies in the order the main thread made it.
///
/// Adding a kind of write is adding a variant and its arm in [`Change::apply`]: the main thread sends it with
/// [`send_change`], and the owner applies it with everything before it at the next unit that needs it.
#[allow(clippy::large_enum_variant)]
pub(crate) enum Change {
    /// The style inputs the main thread recorded since the last transaction: DOM tree insertions, removals and
    /// moves, element arrivals, class, ID and attribute features, element states, inline style and presentational
    /// hint declarations, and the host facts they read. The engine applies them as one batch, which is how its
    /// invalidation sees them.
    StyleInputs(InputForPass),
    /// A write to the document's layout arena, which the owner applies before the next unit or query that reaches
    /// the arena.
    Arena(ArenaChange),
}

/// A write the main thread makes to a document's layout arena.
pub(crate) enum ArenaChange {
    /// Whether the document is an SVG file decoded as an image, which the layout of its SVG roots reads.
    DocumentIsDecodedSvg(bool),
    /// The scroll-state query inputs of the document's scroll containers, which style reads as one generation.
    StyleSnapshotScrollStates(Vec<crate::layout::style_snapshot::FfiLayoutStyleScrollState>),
    /// The document handed an image box the provider a finished layout frame owed it.
    OwnedProviderHandedOver(NodeSlotId),
}

impl ArenaChange {
    fn apply(self, arena: &mut LayoutNodeArena) {
        match self {
            ArenaChange::DocumentIsDecodedSvg(is_decoded_svg) => arena.set_document_is_decoded_svg(is_decoded_svg),
            ArenaChange::StyleSnapshotScrollStates(states) => arena.publish_style_snapshot_scroll_states(&states),
            ArenaChange::OwnedProviderHandedOver(row) => {
                if arena.slot_is_live(row) {
                    arena.note_owned_provider_handed_over(row);
                }
            }
        }
    }
}

/// What of a document's render state the unit applying changes holds. A unit applies only the changes whose target
/// it holds; the rest stay queued for the next unit that does.
pub(crate) struct ChangeTarget<'a> {
    pub(crate) style_engine: &'a mut crate::css::style::StyleEngine,
}

impl Change {
    fn apply(self, target: &mut ChangeTarget<'_>) {
        match self {
            Change::StyleInputs(inputs) => inputs.apply(target.style_engine),
            Change::Arena(_) => debug_assert!(false, "a style unit applies no arena change"),
        }
    }

    fn writes_arena(&self) -> bool {
        matches!(self, Change::Arena(_))
    }
}

/// A document's changes the owner has received and not applied yet, in order.
#[derive(Default)]
struct ChangeQueue {
    received_through: ChangeSeq,
    applied_through: ChangeSeq,
    pending: VecDeque<(ChangeSeq, Change)>,
}

impl ChangeQueue {
    fn receive(&mut self, seq: ChangeSeq, change: Change) {
        debug_assert_eq!(
            seq.0,
            self.received_through.0 + 1,
            "a document's changes reach the owner in order"
        );
        self.received_through = seq;
        self.pending.push_back((seq, change));
    }

    /// Takes the changes through `through` that `wanted` picks, in order, and leaves the rest queued. A change to the
    /// arena and one to the engine write disjoint state, so each kind keeps its own order.
    fn take_through(&mut self, through: ChangeSeq, wanted: impl Fn(&Change) -> bool) -> Vec<Change> {
        debug_assert!(
            through <= self.received_through,
            "a unit applies only changes sent before it"
        );
        let mut taken = Vec::new();
        let mut kept = VecDeque::new();
        while let Some((seq, change)) = self.pending.pop_front() {
            if seq > through {
                self.pending.push_front((seq, change));
                break;
            }
            if wanted(&change) {
                self.applied_through = self.applied_through.max(seq);
                taken.push(change);
            } else {
                kept.push_back((seq, change));
            }
        }
        kept.append(&mut self.pending);
        self.pending = kept;
        taken
    }
}

/// One document's render state, which the Rendering thread owns.
pub(crate) struct RenderState {
    /// The layout arena, with the host tables and scratch beside it. The arena links the document's style engine,
    /// its paint preparation and its animation sampling state.
    arena: Box<ArenaHandle>,
    changes: ChangeQueue,
    /// The document's clock, which ticks its animations while the main thread started it.
    clock: Option<crate::clock_frames::DocumentClock>,
}

impl RenderState {
    /// Applies the arena changes the owner has received, which every unit and query that reaches the arena comes after.
    fn apply_arena_changes(&mut self) {
        let received = self.changes.received_through;
        let changes = self.changes.take_through(received, Change::writes_arena);
        if changes.is_empty() {
            return;
        }
        let arena = self.arena.arena_mut();
        for change in changes {
            if let Change::Arena(change) = change {
                change.apply(arena);
            }
        }
    }

    /// Answers `query` from the state as the units before it left it.
    fn answer(&mut self, query: Query) -> Answer {
        self.apply_arena_changes();
        match query {
            Query::ComputedStyle(demand) => {
                let engine = self.style_engine();
                if engine.is_null() {
                    debug_assert!(false, "the owner answers the style reads of a document with an engine");
                    return Answer::left_to_host(query);
                }
                // SAFETY: The engine is the document's, and the document thread waits for the answer with the
                // engine's token home.
                Answer::ComputedStyle(StyleReadAnswer::Answered(unsafe {
                    engine.reach_on_owner(|engine| demand.answer(engine))
                }))
            }
            Query::FinishOwnerStyleHostHalf => {
                let engine = self.style_engine();
                let arena = self.arena.arena();
                if engine.is_null() {
                    return Answer::of(query, self.arena.arena_mut());
                }
                // SAFETY: As for a style read: the document thread waits for the answer with the engine's token home.
                Answer::Payment(OwedToHost(unsafe {
                    engine.reach_on_owner(|_| arena.finish_flight_style_host_half().1)
                }))
            }
            _ => Answer::of(query, self.arena.arena_mut()),
        }
    }

    /// The document's style engine, which the arena links (null before it links one). The units the owner runs for the
    /// document reach it through [`crate::css::style::StyleEngineHandle::reach_on_owner`].
    fn style_engine(&self) -> crate::css::style::StyleEngineHandle {
        self.arena.arena().style_engine_handle()
    }

    /// The handle of the state's arena, which names the document to what files work under it.
    fn arena_handle(&mut self) -> *mut c_void {
        self.apply_arena_changes();
        std::ptr::from_mut::<ArenaHandle>(&mut self.arena).cast::<c_void>()
    }

    /// The state's arena and what lives beside it, which the owner hands the units it runs for the document.
    fn state(&mut self) -> *mut ArenaHandle {
        self.apply_arena_changes();
        std::ptr::from_mut::<ArenaHandle>(&mut self.arena)
    }

    /// Drops the state. Changes no unit applied, as those sent for a pass that never ran, give up what they hold with
    /// it. An arena that still holds nodes is leaked rather than freed under whatever reaches them.
    fn retire(self) {
        if self.arena.arena().is_retired() {
            drop(self);
            return;
        }
        debug_assert!(false, "layout node arena destroyed with live slots");
        std::mem::forget(self);
    }
}

/// A question the main thread asks about a document's render state, answered as of every change it sent before.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Query {
    /// The geometry of an element's principal box.
    Geometry {
        node: StyleNodeID,
        kind: FfiGeometryReadKind,
    },
    /// How many layout passes and tree builds the document's layout has run, for tests.
    LayoutCounts,
    /// The computed style of an element or one of its pseudo-elements, which a read cannot wait for a style update
    /// to install: a getComputedStyle() read of an element whose style is not up to date, for one. The owner answers
    /// the demand with the engine of the document's render state.
    ComputedStyle(crate::css::style::bridge::RecordDemand),
    /// How many rows of the layout subtree `root` heads carry a pre-order label no greater than the row before them,
    /// for tests.
    PreOrderLabelViolations { root: NodeSlotId },
    /// The element the tree build last saw as the shadow-including parent of the element `node`.
    ShadowIncludingParentElement { node: StyleNodeID },
    /// Whether `row` still names a live row.
    RowIsLive { row: NodeSlotId },
    /// Ends the host half of the batches the owner applied to the layout nodes as the document thread took a style
    /// update's transactions, once the update has installed them: a row the install did not adopt the record of is
    /// put back with the record its element holds. Answers what that owes the host.
    FinishOwnerStyleHostHalf,
    /// A read of the document's layout arena.
    Arena(ArenaQuery),
}

/// A read of a document's layout arena, which [`Query::Arena`] asks.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ArenaQuery {
    /// Whether a box of the document has ever been given a scroll snap type.
    MayHaveScrollSnapAreas,
    /// Takes the scroll containers finished layout tree builds gave a style, each with whether it snaps.
    TakeBuiltScrollSnapContainers,
    /// Whether the innermost list-item counter of an element counts forward and was created by it.
    InnermostListItemCounterIsOwnForwardCounter(StyleNodeID),
    /// What a pseudo-element's box has scrolled to.
    PseudoElementScrollOffset { generator: StyleNodeID, pseudo_kind: u8 },
    /// Whether the counter styles an element's or pseudo-element's generated content names differ from the ones its
    /// box was built with.
    ContentCounterStylesChanged(crate::layout::counters::CounterOwner),
}

/// The answer to an [`ArenaQuery`].
#[derive(Debug)]
pub(crate) enum ArenaAnswer {
    Flag(bool),
    Byte(u8),
    Point(crate::layout::used_values::FfiCssPixelPoint),
    BuiltScrollSnapContainers(Vec<(crate::layout::node_data::NodeSlotId, bool)>),
}

impl ArenaQuery {
    fn left_to_host(self) -> ArenaAnswer {
        match self {
            ArenaQuery::MayHaveScrollSnapAreas | ArenaQuery::InnermostListItemCounterIsOwnForwardCounter(_) => {
                ArenaAnswer::Flag(false)
            }
            ArenaQuery::TakeBuiltScrollSnapContainers => ArenaAnswer::BuiltScrollSnapContainers(Vec::new()),
            ArenaQuery::PseudoElementScrollOffset { .. } => ArenaAnswer::Point(Default::default()),
            ArenaQuery::ContentCounterStylesChanged(_) => {
                ArenaAnswer::Byte(LayoutNodeArena::CONTENT_COUNTER_STYLES_NOT_RECORDED)
            }
        }
    }

    fn answer(self, arena: &mut LayoutNodeArena) -> ArenaAnswer {
        match self {
            ArenaQuery::MayHaveScrollSnapAreas => ArenaAnswer::Flag(arena.may_have_scroll_snap_areas()),
            ArenaQuery::TakeBuiltScrollSnapContainers => {
                ArenaAnswer::BuiltScrollSnapContainers(arena.take_built_scroll_snap_containers())
            }
            ArenaQuery::InnermostListItemCounterIsOwnForwardCounter(element) => ArenaAnswer::Flag(
                arena
                    .counters_sets()
                    .borrow()
                    .innermost_list_item_counter_is_own_forward_counter(element),
            ),
            ArenaQuery::PseudoElementScrollOffset { generator, pseudo_kind } => {
                ArenaAnswer::Point(arena.pseudo_element_scroll_offset(generator, pseudo_kind))
            }
            ArenaQuery::ContentCounterStylesChanged(owner) => {
                ArenaAnswer::Byte(arena.content_counter_styles_changed(owner))
            }
        }
    }
}

/// The answer to a [`Query`], of the variant the query asked for.
#[derive(Debug)]
pub(crate) enum Answer {
    Geometry(FfiGeometryReadAnswer),
    LayoutCounts(LayoutCounts),
    ComputedStyle(StyleReadAnswer),
    Arena(ArenaAnswer),
    Count(u64),
    /// An element, or 0 for none.
    Element(u32),
    Is(bool),
    Payment(OwedToHost),
}

/// What the owner's answer owes the host, which the document thread pays.
pub(crate) struct OwedToHost(pub(crate) crate::layout::HostPayment);

impl std::fmt::Debug for OwedToHost {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OwedToHost")
    }
}

impl Answer {
    /// The count a [`Query::PreOrderLabelViolations`] answered.
    pub(crate) fn count(self) -> u64 {
        match self {
            Self::Count(count) => count,
            _ => {
                debug_assert!(false, "a count is answered with a count");
                0
            }
        }
    }

    /// The element a [`Query::ShadowIncludingParentElement`] answered.
    pub(crate) fn element(self) -> u32 {
        match self {
            Self::Element(element) => element,
            _ => {
                debug_assert!(false, "an element is answered with an element");
                0
            }
        }
    }

    /// What a query that owes the host answered.
    pub(crate) fn payment(self) -> crate::layout::HostPayment {
        match self {
            Self::Payment(OwedToHost(payment)) => payment,
            _ => {
                debug_assert!(false, "a query that owes the host is answered with a payment");
                crate::layout::HostPayment::nothing()
            }
        }
    }

    /// What a yes-or-no query answered.
    pub(crate) fn is(self) -> bool {
        match self {
            Self::Is(is) => is,
            _ => {
                debug_assert!(false, "a yes-or-no query is answered with yes or no");
                false
            }
        }
    }
}

/// What became of a [`Query::ComputedStyle`].
#[derive(Debug)]
pub(crate) enum StyleReadAnswer {
    /// The record the demand answered, as the value the main thread reads it through.
    Answered(crate::css::style::bridge::RecordDemandAnswer),
    /// The owner holds no engine of the document to answer with: the main thread answers the demand.
    LeftToHost,
    /// The owner panicked answering the demand, which it may have left half done in the engine. Nothing answers it
    /// again: the read goes unanswered.
    Unanswered,
}

/// How many layout passes and tree builds a document's layout has run.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LayoutCounts {
    pub(crate) partial_layouts: u64,
    pub(crate) full_layouts: u64,
    pub(crate) tree_builds: crate::layout::update_layout::FfiLayoutTreeBuildStats,
    pub(crate) arena: FfiArenaCounts,
}

/// How many slots, shells and measurements a document's layout arena holds or has taken, for tests.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FfiArenaCounts {
    pub live_slots: u64,
    pub shells: u64,
    pub pre_order_relabels: u64,
    pub intrinsic_measurements: u64,
    pub intrinsic_inline_measurements: u64,
    pub table_cell_measurement_cache_misses: u64,
}

impl Answer {
    /// The answer that leaves the question to the main thread, as it answered it before the owner did.
    fn left_to_host(query: Query) -> Self {
        match query {
            Query::Geometry { .. } => Self::Geometry(FfiGeometryReadAnswer::default()),
            Query::LayoutCounts => Self::LayoutCounts(LayoutCounts::default()),
            Query::ComputedStyle(_) => Self::ComputedStyle(StyleReadAnswer::LeftToHost),
            Query::PreOrderLabelViolations { .. } => Self::Count(0),
            Query::ShadowIncludingParentElement { .. } => Self::Element(0),
            Query::RowIsLive { .. } => Self::Is(false),
            Query::FinishOwnerStyleHostHalf => Self::Payment(OwedToHost(crate::layout::HostPayment::nothing())),
            Query::Arena(query) => Self::Arena(query.left_to_host()),
        }
    }

    /// The answer to a question the owner panicked answering. What the owner reached may be half changed, so the
    /// main thread is not left to answer it again from there.
    fn unanswered(query: Query) -> Self {
        match query {
            Query::ComputedStyle(_) => Self::ComputedStyle(StyleReadAnswer::Unanswered),
            _ => Self::left_to_host(query),
        }
    }

    /// The answer the document thread takes of what the owner did with `query`.
    fn of_outcome(query: Query, outcome: std::thread::Result<Self>) -> Self {
        outcome.unwrap_or_else(|_| Self::unanswered(query))
    }

    /// Answers `query` from `arena` alone. A question the engine answers is left to the main thread.
    pub(crate) fn of(query: Query, arena: &mut LayoutNodeArena) -> Self {
        match query {
            Query::Geometry { node, kind } => Self::Geometry(answer_geometry(arena, node, kind)),
            Query::LayoutCounts => Self::LayoutCounts(LayoutCounts {
                partial_layouts: arena.partial_layout_count(),
                full_layouts: arena.full_layout_count(),
                tree_builds: arena.layout_tree_build_stats(),
                arena: FfiArenaCounts {
                    live_slots: u64::from(arena.live_slot_count()),
                    shells: u64::from(arena.shell_count()),
                    pre_order_relabels: arena.pre_order_relabel_count(),
                    intrinsic_measurements: arena.intrinsic_measurement_count(),
                    intrinsic_inline_measurements: arena.intrinsic_inline_measurement_count(),
                    table_cell_measurement_cache_misses: arena.table_cell_measurement_cache_miss_count(),
                },
            }),
            Query::ComputedStyle(_) => Self::left_to_host(query),
            Query::PreOrderLabelViolations { root } => Self::Count(pre_order_label_violations(arena, root)),
            Query::ShadowIncludingParentElement { node } => Self::Element(arena.shadow_including_parent(node).element),
            Query::RowIsLive { row } => Self::Is(arena.slot_is_live(row)),
            Query::FinishOwnerStyleHostHalf => Self::Payment(OwedToHost(arena.finish_flight_style_host_half().1)),
            Query::Arena(query) => Self::Arena(query.answer(arena)),
        }
    }
}

fn pre_order_label_violations(arena: &LayoutNodeArena, root: NodeSlotId) -> u64 {
    if !arena.slot_is_live(root) {
        return 0;
    }
    let mut violation_count = 0u64;
    let mut previous_label: Option<u64> = None;
    arena.for_each_node_in_layout_subtree_in_pre_order(root, |node| {
        let label = arena.node_pre_order_label(node);
        if previous_label.is_some_and(|previous| label <= previous) {
            violation_count += 1;
        }
        previous_label = Some(label);
    });
    violation_count
}

/// The typed results of a rendering update the owner ran, which the main thread applies where it takes the frame
/// in: where the update ended, and what its stages left for the document.
pub(crate) struct FrameEffects {
    pub(crate) outcome: crate::flight::FfiFlightOutcome,
    pub(crate) ran: crate::flight::FlightRan,
}

/// The stages of one rendering update of a document, as the main thread prepared them. The owner runs them unit by
/// unit (style, the layout rounds, paint preparation, the recording) and regains control after each: where the main
/// thread recalls it ([`ToOwner::Recall`]), the update ends there, and the main thread goes on from where it ended.
pub(crate) struct RenderingUpdate {
    flight: crate::flight::Flight,
    style_engine: Option<crate::css::style::engine_home::StyleEngineLoan>,
    /// Where the owner sends the update's effects.
    effects: Sender<crate::stage_thread::FrameOwns<FrameEffects>>,
    /// How the owner runs it, with the render state of its document, where the owner holds one. The owner reaches
    /// the pipeline only through the updates it is sent, so what reaches the owner without reaching the pipeline (the
    /// unit tests' stage threads) links without it.
    run: fn(Self, Option<*mut ArenaHandle>),
}

impl RenderingUpdate {
    pub(crate) fn new(
        flight: crate::flight::Flight,
        style_engine: Option<crate::css::style::engine_home::StyleEngineLoan>,
        effects: Sender<crate::stage_thread::FrameOwns<FrameEffects>>,
    ) -> Self {
        Self {
            flight,
            style_engine,
            effects,
            run: Self::run_flight,
        }
    }

    fn run(self: Box<Self>, state: Option<*mut ArenaHandle>) {
        (self.run)(*self, state);
    }

    fn run_flight(self, state: Option<*mut ArenaHandle>) {
        let (outcome, ran) = self.flight.run(self.style_engine, state);
        // SAFETY: What the update's stages left is the frame's, which the main thread reaches only once it has taken
        // the frame back.
        let effects = unsafe { crate::stage_thread::FrameOwns::new(FrameEffects { outcome, ran }) };
        // The main thread keeps the receiver until it has taken the frame back.
        let _ = self.effects.send(effects);
    }
}

/// A message to the owner. All documents share one FIFO, since a child document's layout reads its container's.
pub(crate) enum ToOwner {
    /// Takes in the render state of `document`, whose arena the document thread took from the spare the owner built
    /// ahead of it ([`create_document`]), and builds the next spare.
    Create { document: DocumentId, arena: SpareArena },
    /// Changes `first`, `first + 1`, ... of `document`.
    Changes {
        document: DocumentId,
        first: ChangeSeq,
        changes: Vec<Change>,
    },
    /// Runs the rendering update `update` of `document` as the submitted run `ticket`.
    RenderingUpdate {
        document: DocumentId,
        update: Box<RenderingUpdate>,
        ticket: crate::stage_thread::SubmittedRunTicket,
    },
    /// Runs a job of a layout frame of `document` for the document thread, which waits for it: every round the job
    /// starts, to its end, with what the document thread read for the rounds before it sent the job.
    Layout {
        document: DocumentId,
        job: Box<crate::layout::update_layout::OwnerFrameJob>,
    },
    /// Runs a paint preparation pass over the render state of `document` for the document thread, which waits for it.
    Paint {
        document: DocumentId,
        pass: Box<crate::painting::owner_pass::OwnerPaintPass>,
    },
    /// Runs the style transaction `transaction` of `document`, which the document thread takes and waits for: begins
    /// it, runs its pass and finishes it. Answers with the transaction where the owner could not run it.
    Style {
        document: DocumentId,
        transaction: Box<crate::css::style::bridge::OwnerStyleTransaction>,
        reply: crate::stage_thread::OwnerReplyTo<StyleTransactionRan>,
    },
    /// Answers `query` about `document` after its changes through `through`. The document thread waits.
    Ask {
        document: DocumentId,
        through: ChangeSeq,
        query: Query,
        reply: crate::stage_thread::OwnerReplyTo<Answer>,
    },
    /// The document thread takes the frame in flight of `document` back: a rendering update of it in progress ends at
    /// its next unit. Where none is, there is nothing to end.
    Recall { document: DocumentId },
    /// Drops the render state of `document`. Nothing waits for it.
    Destroy { document: DocumentId },
    /// Starts, changes, stops or ticks the clock of a document, or lets the ticks in as the main thread idles.
    Clock(crate::clock_frames::ClockMessage),
}

impl ToOwner {
    /// The document the message is about.
    pub(crate) fn document(&self) -> DocumentId {
        match self {
            Self::Create { document, .. }
            | Self::Changes { document, .. }
            | Self::RenderingUpdate { document, .. }
            | Self::Layout { document, .. }
            | Self::Paint { document, .. }
            | Self::Style { document, .. }
            | Self::Ask { document, .. }
            | Self::Recall { document }
            | Self::Destroy { document } => *document,
            Self::Clock(message) => message.document(),
        }
    }

    /// Whether the owner may handle the message inside a stage it runs for a document thread, which it waits for in
    /// the middle of: the message is a unit or a question a document thread waits for, or what those come after.
    pub(crate) fn may_be_served_inside_a_stage(&self) -> bool {
        matches!(
            self,
            Self::Create { .. }
                | Self::Changes { .. }
                | Self::Style { .. }
                | Self::Layout { .. }
                | Self::Paint { .. }
                | Self::Ask { .. }
        )
    }
}

/// What the owner does with a message that arrives while it runs a rendering update, between two of its units.
pub(crate) enum BetweenUnits {
    /// The document thread recalled the update.
    Recalled,
    /// The message may go before the rest of the update.
    Serve(ToOwner),
    /// The message waits for the update to end.
    Defer(ToOwner),
}

/// Sorts `message`, which arrived while the owner runs a rendering update of `running`, after the messages it deferred
/// meanwhile, which are about the documents `deferred` names (`None` for a message about no document it knows). A
/// message goes after what it defers of its own document, so the messages of a document keep their order; those of
/// another document go on. A recall goes first of all: it only ends an update.
pub(crate) fn between_units(message: ToOwner, running: DocumentId, deferred: &[Option<DocumentId>]) -> BetweenUnits {
    let document = message.document();
    match message {
        ToOwner::Recall { .. } if document == running => BetweenUnits::Recalled,
        ToOwner::Recall { .. } => BetweenUnits::Serve(message),
        message
            if deferred
                .iter()
                .any(|deferred| deferred.is_none_or(|deferred| deferred == document)) =>
        {
            BetweenUnits::Defer(message)
        }
        ToOwner::Destroy { .. } if document != running => BetweenUnits::Serve(message),
        // A paint pass reaches the arena the update runs in; it goes after the update.
        ToOwner::Paint { .. } if document == running => BetweenUnits::Defer(message),
        message if message.may_be_served_inside_a_stage() => BetweenUnits::Serve(message),
        message => BetweenUnits::Defer(message),
    }
}

thread_local! {
    // On the owner thread, the render state of each document it owns.
    static STATES: RefCell<HashMap<DocumentId, RenderState>> = RefCell::new(HashMap::new());
    // On the owner thread, the documents whose rendering update the document thread recalled before the update began,
    // which the update ends at its first unit.
    static RECALLED: RefCell<std::collections::HashSet<DocumentId>> = RefCell::new(std::collections::HashSet::new());
    // On a document thread, the number of the last change it sent for each document.
    static SENT_THROUGH: RefCell<HashMap<DocumentId, ChangeSeq>> = RefCell::new(HashMap::new());
    // On a document thread, the address of each document's arena it created, which names the frame in flight of the
    // document: nothing reaches the arena through it.
    static FRAME_KEYS: RefCell<HashMap<DocumentId, usize>> = RefCell::new(HashMap::new());
}

/// Handles `message`, on the owner thread. A panic in handling it ends that message, not the owner: a document thread
/// that waits for an answer gets it as its answer.
pub(crate) fn handle(message: ToOwner) {
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_message(message))).is_err() {
        debug_assert!(false, "the render owner panicked handling a message");
    }
}

fn handle_message(message: ToOwner) {
    match message {
        ToOwner::Create { document, arena } => {
            let state = RenderState {
                arena: arena.0,
                changes: ChangeQueue::default(),
                clock: None,
            };
            STATES.with_borrow_mut(|states| {
                let previous = states.insert(document, state);
                debug_assert!(previous.is_none(), "document {document:?} created twice");
            });
            build_spare();
        }
        ToOwner::Changes {
            document,
            first,
            changes,
        } => {
            // The changes of a document with no state have nothing to change: they are dropped.
            with_state(document, |state| {
                for (index, change) in changes.into_iter().enumerate() {
                    state.changes.receive(ChangeSeq(first.0 + index as u64), change);
                }
            });
        }
        ToOwner::RenderingUpdate {
            document,
            update,
            ticket,
        } => {
            debug_assert!(
                !document.is_valid() || STATES.with_borrow(|states| states.contains_key(&document)),
                "a rendering update of a document with no render state"
            );
            // A test's update of no document runs with the arena its flight names.
            let state = STATES.with_borrow_mut(|states| states.get_mut(&document).map(RenderState::state));
            ticket.run(|| update.run(state));
        }
        ToOwner::Style {
            document,
            transaction,
            reply,
        } => reply.answer(|| {
            let reached = with_state(document, |state| (state.style_engine(), state.arena_handle()))
                .filter(|(engine, _)| !engine.is_null());
            let Some((engine, arena)) = reached else {
                debug_assert!(
                    false,
                    "the owner runs the style transaction of a document with an engine"
                );
                return Err(transaction);
            };
            // The faces the transaction wants are this document's, for its layout end to request, whichever
            // document's update the owner serves the transaction beside.
            let _wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(arena as u64);
            // SAFETY: The engine is the document's, and the document thread waits for the transaction.
            Ok(unsafe { engine.reach_on_owner(|engine| transaction.run(engine, true)) })
        }),
        ToOwner::Layout { document, job } => {
            // The state's borrow ends before the job runs, which may reach another document's state. The job finds
            // the arena inside its answer, so that a panic there answers the waiting document thread.
            (*job).run(|| with_state(document, RenderState::state));
        }
        ToOwner::Paint { document, pass } => {
            // As for a layout unit, the pass finds the arena inside its answer.
            (*pass).run(|| with_state(document, RenderState::state));
        }
        ToOwner::Ask {
            document,
            through,
            query,
            reply,
        } => reply.answer(|| {
            with_state(document, |state| {
                debug_assert!(
                    state.changes.pending.front().is_none_or(|(seq, _)| *seq > through),
                    "a query is answered after the changes sent before it"
                );
                state.answer(query)
            })
            .unwrap_or_else(|| Answer::left_to_host(query))
        }),
        ToOwner::Recall { document } => {
            // A rendering update the owner deferred ends at its first unit. The update was sent before the recall, so
            // where none waits, it is over already.
            if crate::stage_thread::defers_rendering_update_of(document) {
                RECALLED.with_borrow_mut(|recalled| recalled.insert(document));
            }
        }
        ToOwner::Destroy { document } => {
            let state = STATES.with_borrow_mut(|states| states.remove(&document));
            debug_assert!(state.is_some(), "document {document:?} destroyed twice");
            if let Some(state) = state {
                state.retire();
            }
        }
        ToOwner::Clock(message) => crate::clock_frames::handle_on_owner(message),
    }
}

/// On the owner thread, as the rendering update of `document` begins: whether the document thread recalled it already.
pub(crate) fn take_recall(document: DocumentId) -> bool {
    RECALLED.with_borrow_mut(|recalled| recalled.remove(&document))
}

/// Runs `operation` on the render state of `document`, on the owner thread. A message about a document with no
/// state here is a bug of the sender's; it gets no answer from the state, and the caller falls back.
fn with_state<R>(document: DocumentId, operation: impl FnOnce(&mut RenderState) -> R) -> Option<R> {
    STATES.with_borrow_mut(|states| {
        let state = states.get_mut(&document);
        debug_assert!(
            state.is_some(),
            "document {document:?} has no render state on this thread"
        );
        state.map(operation)
    })
}

/// On the owner thread: applies the changes of `document` through `through` that `target` can take.
pub(crate) fn apply_changes_through(document: DocumentId, through: ChangeSeq, target: &mut ChangeTarget<'_>) {
    if !document.is_valid() {
        return;
    }
    let changes = with_state(document, |state| {
        state.changes.take_through(through, |change| !change.writes_arena())
    })
    .unwrap_or_default();
    for change in changes {
        change.apply(target);
    }
}

/// On the owner thread: runs `operation` on the clock slot of `document`'s render state, with the handle of the arena
/// it ticks.
pub(crate) fn with_clock<R>(
    document: DocumentId,
    operation: impl FnOnce(&mut Option<crate::clock_frames::DocumentClock>, *mut c_void) -> R,
) -> Option<R> {
    with_state(document, |state| {
        let arena = state.arena_handle();
        operation(&mut state.clock, arena)
    })
}

/// On the owner thread: the document whose clock the render clock ticks at the display ticks of the compositor
/// context `context`.
pub(crate) fn document_with_clock_at(context: u64) -> Option<DocumentId> {
    if context == 0 {
        return None;
    }
    STATES.with_borrow(|states| {
        states
            .iter()
            .find(|(_, state)| state.clock.as_ref().is_some_and(|clock| clock.context() == context))
            .map(|(document, _)| *document)
    })
}

/// Sends `message` to the owner: to the Rendering thread, or handled right here where there is none.
pub(crate) fn send(message: ToOwner) {
    if let Err(message) = crate::stage_thread::send_to_owner(message) {
        handle(message);
    }
}

/// An arena the owner built ahead of the next document, for the document thread to take without waiting.
pub(crate) struct SpareArena(Box<ArenaHandle>);

// SAFETY: A spare's host tables are empty, and nothing reaches the arena but the thread that holds the spare.
unsafe impl Send for SpareArena {}

/// The spare the owner built last, which the next document takes.
static SPARE: std::sync::Mutex<Option<SpareArena>> = std::sync::Mutex::new(None);

/// On the owner thread: builds the arena the next document takes, if there is none.
fn build_spare() {
    let mut spare = SPARE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    if spare.is_none() {
        *spare = Some(SpareArena(Box::new(ArenaHandle::new_for(
            DocumentId::default(),
            std::thread::current().id(),
        ))));
    }
}

/// Creates the render state of a new document, and answers with its name and the address of its arena. The arena is
/// the spare the owner built ahead, or where the owner has built none yet, one built here; either way the owner takes
/// the state in, and nothing waits for it.
pub(crate) fn create_document() -> (DocumentId, *mut c_void) {
    let document = DocumentId::mint();
    let spare = SPARE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take();
    // The owner built the spare before it released the handling of the message it built it in.
    crate::stage_thread::acquire_owner();
    let mut arena = spare.map_or_else(
        || Box::new(ArenaHandle::new_for(document, std::thread::current().id())),
        |spare| spare.0,
    );
    arena.adopt(document, std::thread::current().id());
    let address = std::ptr::from_mut::<ArenaHandle>(&mut arena).cast::<c_void>();
    FRAME_KEYS.with_borrow_mut(|keys| keys.insert(document, address as usize));
    send(ToOwner::Create {
        document,
        arena: SpareArena(arena),
    });
    (document, address)
}

/// Drops the render state of `document` on the owner. Nothing waits for it.
pub(crate) fn destroy_document(document: DocumentId) {
    SENT_THROUGH.with_borrow_mut(|sent| sent.remove(&document));
    FRAME_KEYS.with_borrow_mut(|keys| keys.remove(&document));
    send(ToOwner::Destroy { document });
}

/// Sends `change` for `document`, and answers with its number: a unit that applies the changes through it applies
/// this one. Only the document thread that came through the document's render inputs sends one, having dropped the
/// query snapshot the document published.
pub(crate) fn send_change(
    _through: crate::css::style::ThroughRenderInputs,
    document: DocumentId,
    change: Change,
) -> ChangeSeq {
    let seq = SENT_THROUGH.with_borrow_mut(|sent| {
        let seq = sent.entry(document).or_default();
        seq.0 += 1;
        *seq
    });
    send(ToOwner::Changes {
        document,
        first: seq,
        changes: vec![change],
    });
    seq
}

/// Sends the arena write `change` for `document`, which the owner applies before the next unit or query that reaches
/// the arena.
pub(crate) fn send_arena_change(document: DocumentId, change: ArenaChange) -> ChangeSeq {
    let seq = SENT_THROUGH.with_borrow_mut(|sent| {
        let seq = sent.entry(document).or_default();
        seq.0 += 1;
        *seq
    });
    send(ToOwner::Changes {
        document,
        first: seq,
        changes: vec![Change::Arena(change)],
    });
    seq
}

/// The number of the last change the calling document thread sent for `document`.
pub(crate) fn sent_through(document: DocumentId) -> ChangeSeq {
    SENT_THROUGH.with_borrow(|sent| sent.get(&document).copied().unwrap_or_default())
}

/// Recalls the rendering update of `document` the owner runs, if it runs one: the document thread takes its frame back.
pub(crate) fn recall_rendering_update(document: DocumentId) {
    if document.is_valid() {
        send(ToOwner::Recall { document });
    }
}

/// Asks the owner `query` about `document`, whose arena the calling document thread names as `arena`, and waits for
/// the answer, as of every change the thread sent before. Where the owner cannot answer it (a test holds the run it
/// would queue behind), the thread reads its arena right here, as every door of the port does. A question the owner
/// panicked answering is [`Answer::unanswered`].
///
/// # Safety
///
/// `arena` must be the live arena of `document`, which no stage the document thread submitted owns.
pub(crate) unsafe fn ask(document: DocumentId, arena: *mut c_void, query: Query) -> Answer {
    let through = sent_through(document);
    let answer = crate::stage_thread::wait_for_owner(
        |reply| ToOwner::Ask {
            document,
            through,
            query,
            reply,
        },
        || {
            if let Some(answer) =
                STATES.with_borrow_mut(|states| states.get_mut(&document).map(|state| state.answer(query)))
            {
                return answer;
            }
            // SAFETY: Guaranteed by the caller.
            Answer::of(
                query,
                unsafe { &mut *ArenaHandle::held_by_waiting_thread(arena) }.arena_mut(),
            )
        },
    );
    debug_assert!(answer.is_ok(), "the render owner panicked answering {query:?}");
    Answer::of_outcome(query, answer)
}

/// Asks the owner `query` about the document whose arena the calling document thread names as `arena`, once the frame
/// in flight that owns the arena, if any, has been taken back.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
pub(crate) unsafe fn ask_about(arena: *mut c_void, query: Query) -> Answer {
    assert!(!arena.is_null(), "layout node arena handle is null");
    crate::stage_thread::join_frame_in_flight(arena);
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { ArenaHandle::document_of(arena) };
    // SAFETY: As above.
    unsafe { ask(document, arena, query) }
}

/// Asks the owner `query` about `document` and waits for the answer, as [`ask`] does, for a document thread that
/// names no arena: where the owner cannot answer it, the question is left to the host.
pub(crate) fn ask_owner(document: DocumentId, query: Query) -> Answer {
    let through = sent_through(document);
    let answer = crate::stage_thread::wait_for_owner(
        |reply| ToOwner::Ask {
            document,
            through,
            query,
            reply,
        },
        || {
            STATES
                .with_borrow_mut(|states| states.get_mut(&document).map(|state| state.answer(query)))
                .unwrap_or_else(|| Answer::left_to_host(query))
        },
    );
    debug_assert!(answer.is_ok(), "the render owner panicked answering {query:?}");
    Answer::of_outcome(query, answer)
}

/// The counts of the layout arena of `document`, which the owner answers, for tests.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_arena_counts(document: DocumentId) -> FfiArenaCounts {
    if !document.is_valid() {
        return FfiArenaCounts::default();
    }
    join_frame_of(document);
    match ask_owner(document, Query::LayoutCounts) {
        Answer::LayoutCounts(counts) => counts.arena,
        _ => {
            debug_assert!(false, "layout counts are answered with counts");
            FfiArenaCounts::default()
        }
    }
}

/// On a document thread: takes back the frame in flight of `document`, if any, so that a read finds the document as
/// the frame left it.
#[track_caller]
fn join_frame_of(document: DocumentId) {
    if let Some(key) = FRAME_KEYS.with_borrow(|keys| keys.get(&document).copied()) {
        crate::stage_thread::join_frame_in_flight(key as *mut c_void);
    }
}

/// Asks the owner `query` of the layout arena of `document`, once its frame in flight is taken back.
#[track_caller]
fn ask_arena(document: DocumentId, query: ArenaQuery) -> ArenaAnswer {
    if !document.is_valid() {
        return query.left_to_host();
    }
    join_frame_of(document);
    match ask_owner(document, Query::Arena(query)) {
        Answer::Arena(answer) => answer,
        _ => {
            debug_assert!(false, "an arena query is answered from the arena");
            query.left_to_host()
        }
    }
}

fn ask_arena_flag(document: DocumentId, query: ArenaQuery) -> bool {
    match ask_arena(document, query) {
        ArenaAnswer::Flag(flag) => flag,
        _ => false,
    }
}

/// Whether a box of `document` has ever been given a scroll snap type, by a restyle or by a layout tree build.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_may_have_scroll_snap_areas(document: DocumentId) -> bool {
    ask_arena_flag(document, ArenaQuery::MayHaveScrollSnapAreas)
}

/// Hands the host the scroll containers finished layout tree builds of `document` gave a style, each with whether it
/// was a scroll snap container then.
///
/// # Safety
///
/// `callback` must be callable with `context` for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn render_owner_take_built_scroll_snap_containers(
    document: DocumentId,
    context: *mut c_void,
    callback: unsafe extern "C" fn(*mut c_void, crate::layout::node_data::NodeSlotId, bool),
) {
    let ArenaAnswer::BuiltScrollSnapContainers(built) = ask_arena(document, ArenaQuery::TakeBuiltScrollSnapContainers)
    else {
        return;
    };
    for (row, is_scroll_snap_container) in built {
        // SAFETY: Guaranteed by the caller.
        unsafe { callback(context, row, is_scroll_snap_container) };
    }
}

/// Whether the innermost list-item counter of the element with `style_node` in `document` counts forward and was
/// created by that element.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_innermost_list_item_counter_is_own_forward_counter(
    document: DocumentId,
    style_node: u32,
) -> bool {
    let Some(element) = StyleNodeID::from_raw(style_node) else {
        return false;
    };
    ask_arena_flag(
        document,
        ArenaQuery::InnermostListItemCounterIsOwnForwardCounter(element),
    )
}

/// What the pseudo-element `pseudo_kind` of the element with `generator` in `document` has scrolled to.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_pseudo_element_scroll_offset(
    document: DocumentId,
    generator: u32,
    pseudo_kind: u8,
) -> crate::layout::used_values::FfiCssPixelPoint {
    let Some(generator) = StyleNodeID::from_raw(generator) else {
        return Default::default();
    };
    match ask_arena(
        document,
        ArenaQuery::PseudoElementScrollOffset { generator, pseudo_kind },
    ) {
        ArenaAnswer::Point(point) => point,
        _ => Default::default(),
    }
}

/// Whether the counter styles the record of the pseudo-element `generated_for` of the element `style_node` in
/// `document` names now differ from the ones the box built for it renders from, as the `CONTENT_COUNTER_STYLES_*`
/// answers.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_content_counter_styles_changed(
    document: DocumentId,
    style_node: u32,
    generated_for: u8,
) -> u8 {
    let Some(element) = StyleNodeID::from_raw(style_node) else {
        return LayoutNodeArena::CONTENT_COUNTER_STYLES_NOT_RECORDED;
    };
    let owner = crate::layout::counters::CounterOwner { element, generated_for };
    match ask_arena(document, ArenaQuery::ContentCounterStylesChanged(owner)) {
        ArenaAnswer::Byte(answer) => answer,
        _ => LayoutNodeArena::CONTENT_COUNTER_STYLES_NOT_RECORDED,
    }
}

/// What became of a style transaction the owner was sent: its answers, or the transaction where the owner could not
/// run it.
pub(crate) type StyleTransactionRan =
    Result<crate::css::style::bridge::OwnerStyleTransactionView, Box<crate::css::style::bridge::OwnerStyleTransaction>>;

/// Runs the style transaction `transaction` of `document`, which the calling document thread takes, on the owner, and
/// waits for its answers. Where the owner does not run it (a test holds the run it would queue behind, or a bug of the
/// sender's), the document thread runs it with the engine `engine` right here, as every door of the port does.
///
/// # Safety
///
/// `engine` must be the live engine of `document`, whose token is home, and the calling thread must reach nothing of
/// it until this returns.
pub(crate) unsafe fn run_style_transaction(
    document: DocumentId,
    engine: crate::css::style::StyleEngineHandle,
    transaction: crate::css::style::bridge::OwnerStyleTransaction,
) -> crate::css::style::bridge::OwnerStyleTransactionView {
    let transaction = std::cell::Cell::new(Some(Box::new(transaction)));
    let ran = crate::stage_thread::wait_for_owner(
        |reply| {
            let mut transaction = transaction.take().expect("the transaction is sent once");
            // Its input goes ahead of it, as a change of its document.
            transaction.send_input_to_owner(document);
            ToOwner::Style {
                document,
                transaction,
                reply,
            }
        },
        || Err(transaction.take().expect("the transaction runs once")),
    );
    match ran.unwrap_or_else(|payload| std::panic::resume_unwind(payload)) {
        Ok(view) => view,
        // SAFETY: Guaranteed by the caller.
        Err(transaction) => unsafe { transaction.run(engine.enter("style transaction the owner did not run"), false) },
    }
}

/// Whether the style transactions of `document` run on the owner: a document the owner holds render state for, whose
/// transactions a document thread waits for. The style update around each transaction stays the document thread's
/// (its host steps freeze the transaction's inputs and install the answers it published), but the transaction, the
/// style computation, is the owner's.
pub(crate) fn runs_style_of(document: DocumentId) -> bool {
    document.is_valid()
}

thread_local! {
    // On a document thread, the query it asks about the next layout update of the document it names: the update
    // runs on the owner, which answers the query from it.
    static ASKED: std::cell::Cell<Option<(DocumentId, Query)>> = const { std::cell::Cell::new(None) };
    // On a document thread, the answer to the query it asked.
    static ANSWERED: std::cell::Cell<Option<Answer>> = const { std::cell::Cell::new(None) };
}

/// On a document thread, in a layout update of `document`: the query the update is for, if the thread asked one.
pub(crate) fn asked_about(document: DocumentId) -> Option<Query> {
    ASKED
        .get()
        .filter(|(asked, _)| *asked == document)
        .map(|(_, query)| query)
}

/// On a document thread: leaves the answer to the query it asked, for [`render_owner_take_geometry_answer`].
pub(crate) fn answered(answer: Answer) {
    ANSWERED.set(Some(answer));
}

/// Asks the geometry read `kind` about the element with `style_node` in `document`, answered by the owner in the
/// layout update the read runs next. The document thread takes the answer with
/// [`render_owner_take_geometry_answer`] once it has updated the layout.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_begin_geometry_query(document: DocumentId, style_node: u32, kind: FfiGeometryReadKind) {
    ANSWERED.set(None);
    ASKED.set(
        StyleNodeID::from_raw(style_node)
            .filter(|_| document.is_valid())
            .map(|node| (document, Query::Geometry { node, kind })),
    );
}

/// Takes the answer to the geometry read asked last, which a layout update of its document gave, and forgets the
/// question. A read no layout update answered is not `answered`.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_take_geometry_answer() -> FfiGeometryReadAnswer {
    ASKED.set(None);
    match ANSWERED.take() {
        Some(Answer::Geometry(answer)) => answer,
        Some(_) => {
            debug_assert!(false, "a geometry read is answered with geometry");
            FfiGeometryReadAnswer::default()
        }
        None => FfiGeometryReadAnswer::default(),
    }
}

/// What a geometry read asks about its element's principal box.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
// C++ names the kinds.
#[allow(dead_code)]
pub enum FfiGeometryReadKind {
    /// The bounding client rect, as `getBoundingClientRect()` returns it.
    BoundingClientRect,
    /// The absolute border box, which the `offset*` getters read.
    BorderBox,
}

/// The answer to a geometry read. A read that is not `answered` reads the arena as before.
#[derive(Clone, Copy, Default, Debug)]
#[repr(C)]
pub struct FfiGeometryReadAnswer {
    pub answered: bool,
    /// The element has a principal box with a committed paintable.
    pub has_box: bool,
    pub rect: FfiCssPixelRect,
}

impl FfiGeometryReadAnswer {
    /// The answer for an element with no principal box.
    fn no_box() -> Self {
        Self {
            answered: true,
            ..Default::default()
        }
    }
}

/// Answers the geometry read `kind` about `node` from `arena`, which the units before it have laid out.
fn answer_geometry(arena: &mut LayoutNodeArena, node: StyleNodeID, kind: FfiGeometryReadKind) -> FfiGeometryReadAnswer {
    let slot = arena.bound_row(node);
    let has_layout_root = !arena.layout_root().is_invalid();
    let rows = arena.committed_paintable_rows();
    if slot.is_invalid() || rows.node_data_if_live(slot).is_none() {
        return FfiGeometryReadAnswer::no_box();
    }
    // A table's principal box is its wrapper, which the document thread finds.
    let parent = rows.node_data_if_live(slot).map(|data| data.parent.get());
    if parent
        .and_then(|parent| rows.node_data_if_live(parent))
        .is_some_and(|data| data.kind.get() == NodeKind::TableWrapper)
    {
        return FfiGeometryReadAnswer::default();
    }
    match kind {
        FfiGeometryReadKind::BoundingClientRect => {
            // Only a box that needs no visual context: the document thread checks that its own facts (a zero
            // viewport scroll offset, a pending visual context update) agree before it takes the answer.
            if !has_layout_root
                || !crate::painting::client_rects::can_compute_client_rects_without_visual_context_update(
                    &rows, slot, true,
                )
            {
                return FfiGeometryReadAnswer::default();
            }
            FfiGeometryReadAnswer {
                answered: true,
                has_box: true,
                rect: crate::painting::client_rects::bounding_client_rect(&rows, slot, None).into(),
            }
        }
        FfiGeometryReadKind::BorderBox => {
            if !rows.paintable_row_is_populated(slot) {
                return FfiGeometryReadAnswer::no_box();
            }
            FfiGeometryReadAnswer {
                answered: true,
                has_box: true,
                rect: crate::painting::paintable_geometry::absolute_border_box_rect(&rows, slot).into(),
            }
        }
    }
}

/// What the owner gave the main thread for a document it created: the document's name, and the arena its
/// remaining doors reach.
#[repr(C)]
pub struct FfiRenderDocument {
    pub document: DocumentId,
    pub arena: *mut c_void,
}

/// Creates the render state of a new document on the owner, and answers with its name and the arena the main
/// thread's remaining doors reach.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_create_document() -> FfiRenderDocument {
    let (document, arena) = create_document();
    FfiRenderDocument { document, arena }
}

/// Drops the render state `document` names on the owner, once the document thread holds nothing of it.
///
/// # Safety
///
/// `document` must come from [`render_owner_create_document`], be destroyed once, and its arena must be reached no
/// more.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn render_owner_destroy_document(document: FfiRenderDocument) {
    let arena = document.arena;
    assert!(!arena.is_null(), "layout node arena handle is null");
    // A frame in flight owns the arena until it is taken back, and a stage of the document that reaches no arena (a
    // style pass) its engine: the owner drops the state only once the main thread has taken back every stage of it.
    crate::stage_thread::join_document_frame_in_flight_at(arena, file!(), line!(), column!());
    // The document's clock goes with its render state.
    crate::clock_frames::document_destroyed(document.document);
    crate::layout::flush_arena_censuses();
    destroy_document(document.document);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_applies_the_changes_through_its_number_in_order() {
        let mut queue = ChangeQueue::default();
        assert!(queue.take_through(ChangeSeq(0), |_| true).is_empty());
        queue.receive(ChangeSeq(1), Change::StyleInputs(InputForPass::empty()));
        queue.receive(ChangeSeq(2), Change::StyleInputs(InputForPass::empty()));
        queue.receive(ChangeSeq(3), Change::StyleInputs(InputForPass::empty()));
        assert_eq!(queue.take_through(ChangeSeq(2), |_| true).len(), 2);
        assert_eq!(queue.applied_through, ChangeSeq(2));
        assert_eq!(queue.take_through(ChangeSeq(2), |_| true).len(), 0);
        assert_eq!(queue.take_through(ChangeSeq(3), |_| true).len(), 1);
        assert!(queue.pending.is_empty());
    }

    #[test]
    fn a_running_update_defers_its_own_documents_destroy_and_serves_the_rest() {
        let running = DocumentId::mint();
        let other = DocumentId::mint();
        let sorted = |message| match between_units(message, running, &[]) {
            BetweenUnits::Recalled => "recalled",
            BetweenUnits::Serve(_) => "serve",
            BetweenUnits::Defer(_) => "defer",
        };
        // Its own document's state is dropped only once the update is over, after what it defers before it.
        assert_eq!(sorted(ToOwner::Destroy { document: running }), "defer");
        assert_eq!(sorted(ToOwner::Destroy { document: other }), "serve");
        assert_eq!(sorted(ToOwner::Recall { document: running }), "recalled");
        assert_eq!(sorted(ToOwner::Recall { document: other }), "serve");
        let changes = |document| ToOwner::Changes {
            document,
            first: ChangeSeq(1),
            changes: vec![Change::StyleInputs(InputForPass::empty())],
        };
        assert_eq!(sorted(changes(running)), "serve");
        assert_eq!(sorted(changes(other)), "serve");
        // A paint pass reaches the arena of its document: the running update's goes after the update.
        let paint = |document| {
            let (reply, _) = crate::stage_thread::owner_reply_for_test();
            ToOwner::Paint {
                document,
                pass: Box::new(crate::painting::owner_pass::OwnerPaintPass::new_for_test(
                    std::ptr::null_mut(),
                    |_, ()| {},
                    reply,
                )),
            }
        };
        assert_eq!(sorted(paint(running)), "defer");
        assert_eq!(sorted(paint(other)), "serve");
    }

    #[test]
    fn a_running_update_keeps_each_documents_order_behind_what_it_deferred() {
        let running = DocumentId::mint();
        let deferred_document = DocumentId::mint();
        let other = DocumentId::mint();
        let sorted = |message, deferred: &[Option<DocumentId>]| match between_units(message, running, deferred) {
            BetweenUnits::Recalled => "recalled",
            BetweenUnits::Serve(_) => "serve",
            BetweenUnits::Defer(_) => "defer",
        };
        let changes = |document| ToOwner::Changes {
            document,
            first: ChangeSeq(1),
            changes: vec![Change::StyleInputs(InputForPass::empty())],
        };
        // A message of a document the owner deferred a message of goes after it; another document's goes on.
        let deferred = [Some(deferred_document)];
        assert_eq!(sorted(changes(deferred_document), &deferred), "defer");
        assert_eq!(sorted(changes(other), &deferred), "serve");
        assert_eq!(sorted(changes(running), &deferred), "serve");
        assert_eq!(sorted(ToOwner::Destroy { document: other }, &deferred), "serve");
        assert_eq!(
            sorted(
                ToOwner::Destroy {
                    document: deferred_document
                },
                &deferred
            ),
            "defer"
        );
        // A recall goes first of all, even of a document with a deferred message: it only ends an update.
        assert_eq!(
            sorted(
                ToOwner::Recall {
                    document: deferred_document
                },
                &deferred
            ),
            "serve"
        );
        assert_eq!(sorted(ToOwner::Recall { document: running }, &deferred), "recalled");
        // Behind a message about no document the owner knows, everything waits but a recall.
        let deferred = [None];
        assert_eq!(sorted(changes(other), &deferred), "defer");
        assert_eq!(sorted(ToOwner::Destroy { document: other }, &deferred), "defer");
        assert_eq!(sorted(ToOwner::Recall { document: running }, &deferred), "recalled");
    }

    #[test]
    fn a_recall_with_no_deferred_update_leaves_the_next_update_alone() {
        let document = DocumentId::mint();
        // The update the recall is for was sent before it; with none deferred, it is over already.
        handle(ToOwner::Recall { document });
        assert!(!take_recall(document));
    }

    #[test]
    fn document_ids_are_minted_without_the_owner() {
        let first = DocumentId::mint();
        let second = DocumentId::mint();
        assert!(first.is_valid() && second.is_valid());
        assert_ne!(first, second);
    }

    #[test]
    fn the_owner_takes_in_render_state_and_builds_the_next_spare() {
        let owner = std::thread::spawn(|| {
            let document = DocumentId::mint();
            handle(ToOwner::Create {
                document,
                arena: SpareArena(Box::new(ArenaHandle::new_for(document, std::thread::current().id()))),
            });
            assert!(SPARE.lock().unwrap().is_some());
            let seq = ChangeSeq(1);
            handle(ToOwner::Changes {
                document,
                first: seq,
                changes: vec![Change::StyleInputs(InputForPass::empty())],
            });
            let applied = with_state(document, |state| state.changes.take_through(seq).len()).unwrap();
            assert_eq!(applied, 1);
            handle(ToOwner::Destroy { document });
            STATES.with_borrow(|states| states.is_empty())
        });
        assert!(owner.join().unwrap());
    }

    // The owner of a test build panics answering the style read of a document with no engine.
    #[cfg(debug_assertions)]
    #[test]
    fn a_style_read_the_owner_panicked_answering_goes_unanswered_on_the_host() {
        let owner = std::thread::spawn(|| {
            let document = DocumentId::mint();
            let arena = Box::new(ArenaHandle::new_for(document, std::thread::current().id()));
            handle(ToOwner::Create {
                document,
                arena: SpareArena(arena),
            });
            let query = Query::ComputedStyle(crate::css::style::bridge::RecordDemand {
                node: 1,
                pseudo_kind: u8::MAX,
                exclude_inline_style: false,
                targeted: false,
                read_only: true,
                parent_highlight: 0,
            });
            let (reply, answered) = crate::stage_thread::owner_reply_for_test();
            handle(ToOwner::Ask {
                document,
                through: ChangeSeq::default(),
                query,
                reply,
            });
            let outcome = answered();
            assert!(outcome.is_err(), "the owner panicked answering");
            // The host takes the read as unanswered, and does not answer it again with the engine the owner left.
            let answer = crate::css::style::bridge::host_answer_of_owner_read(Answer::of_outcome(query, outcome));
            assert!(
                answer.is_some_and(|answer| answer.unanswered && answer.is_absent && answer.published_record.is_null()),
                "the host answers the read without the engine"
            );
            // A read the owner leaves to the host is the host's to answer.
            assert!(crate::css::style::bridge::host_answer_of_owner_read(Answer::left_to_host(query)).is_none());
            handle(ToOwner::Destroy { document });
            STATES.with_borrow(|states| states.is_empty())
        });
        assert!(owner.join().unwrap());
    }

    #[test]
    fn the_owner_runs_a_paint_pass_with_the_arena_of_the_render_state_and_answers() {
        let owner = std::thread::spawn(|| {
            let document = DocumentId::mint();
            let mut arena = Box::new(ArenaHandle::new_for(document, std::thread::current().id()));
            let sent = std::ptr::from_mut::<ArenaHandle>(&mut arena).cast::<c_void>();
            handle(ToOwner::Create {
                document,
                arena: SpareArena(arena),
            });
            // The owner finds the arena of the document's render state, the arena the document thread sent.
            let (reply, answered) = crate::stage_thread::owner_reply_for_test();
            handle(ToOwner::Paint {
                document,
                pass: Box::new(crate::painting::owner_pass::OwnerPaintPass::new_for_test(
                    sent,
                    |arena, ()| assert!(arena.document().is_valid(), "the pass runs with a document's arena"),
                    reply,
                )),
            });
            assert!(answered().is_ok(), "the pass answers");
            handle(ToOwner::Destroy { document });
            STATES.with_borrow(|states| states.is_empty())
        });
        assert!(owner.join().unwrap());
    }
}
