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
//! - a change: owned `Send` data, fire and forget. An [`ArenaChange`], numbered per document ([`ChangeSeq`]), the
//!   owner queues and applies, in order, as soon as it idles ([`take_in_sent_changes`]), or at the next point that
//!   needs it: a unit of a rendering update, or a query. A write to the style engine waits in the engine's home, and
//!   whoever reaches the engine next applies it first: the owner as it idles, if nothing else does before.
//! - a [`RenderingUpdate`]: the stages of a frame, which the owner runs as units ([`FrameUnit`]) and answers with
//!   the frame's news ([`crate::frame_news`]), the typed results the main thread adopts where it takes the frame in.
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

mod devtools;

use crate::css::style::tree::StyleNodeID;
use crate::fast_hash::FastMap as HashMap;
use crate::layout::node_data::{NodeKind, NodeSlotId};
use crate::layout::{ArenaHandle, FfiCssPixelRect, LayoutNodeArena};
use crate::painting::geometry_read::GeometryRead;
use std::cell::RefCell;
use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

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

/// The one wait of a script API call that needs a current answer (getComputedStyle, an element's geometry, hit testing,
/// innerText and the like). The host entry the API calls mints it, and the wait takes it by value, so the call waits for
/// the owner once with it: it is neither `Clone` nor `Copy`, and stays on the document thread. A call that lays the
/// document out spends it on [`force_read`], the one job of its update. Internal code (drains, painting, event
/// dispatch) holds none: it reads the rows the owner published last ([`crate::layout::row_reads::FrameRows`]) and what
/// it sent ahead of them, and waits for the owner otherwise only with a [`LockstepProof`] or a job's permit.
pub(crate) struct ScriptForcedRead {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl ScriptForcedRead {
    /// # Safety
    ///
    /// Only a host entry that a script API calls for a current answer mints one, once per call, on the document
    /// thread.
    pub(crate) unsafe fn at_script_entry() -> Self {
        Self {
            _not_send: std::marker::PhantomData,
        }
    }
}

/// The right of a main thread that cannot go on without the owner to wait for it outside a script's forced read. Each
/// reason has a constructor of its own, so the waits internal code makes are these and no others; everything else reads
/// what the owner published ([`crate::layout::row_reads::FrameRows`]).
pub(crate) struct LockstepProof {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl LockstepProof {
    const fn new() -> Self {
        Self {
            _not_send: std::marker::PhantomData,
        }
    }

    /// A layout write (a subtree dropped, a top layer element detached) whose payment the host makes before it goes
    /// on.
    pub(crate) const fn host_pays_the_write() -> Self {
        Self::new()
    }

    /// A user's selection by word, which reads the text the owner shaped.
    pub(crate) const fn input_selects_by_word() -> Self {
        Self::new()
    }

    /// A user's input (a wheel or key scroll, an event's target), which reads the boxes the owner laid out.
    pub(crate) const fn input_reads_boxes() -> Self {
        Self::new()
    }

    /// A recording the main thread makes itself, which predicts the vector images it paints from the damage only the
    /// owner holds, or reads what the owner's last recording left.
    pub(crate) const fn recording_on_main() -> Self {
        Self::new()
    }

    /// A paint step the host runs over the owner's paint state: the visual contexts, the scroll state, the rendering
    /// preparation after a layout, the compositor's animations and the SVG paint resources.
    pub(crate) const fn host_paint_step() -> Self {
        Self::new()
    }

    /// A door of the style engine the host's style code calls in the middle of its own steps (a match, a record, a
    /// sample, a transition's decision), which only the engine answers.
    pub(crate) const fn engine_door() -> Self {
        Self::new()
    }

    /// The snap areas of a scroll container, which scroll snapping reads once a scroll ends.
    pub(crate) const fn scroll_snaps() -> Self {
        Self::new()
    }

    /// A clean read's query snapshot, which holds the rows as of every change the document thread sent.
    pub(crate) const fn query_snapshot() -> Self {
        Self::new()
    }

    /// A layout update the host waits for on its own account, for no script API call: an event's dispatch, a child
    /// document's style update, an inspection, a screenshot. It is spent as a script's read is, on [`force_read`].
    pub(crate) const fn host_reads_layout() -> Self {
        Self::new()
    }
}

/// What a layout update the document thread waits for spends on its one job, [`force_read`]: the read of the script API
/// call it runs for, or the host's own. The update's first style or layout job takes it.
pub(crate) enum ForcedRead {
    Script(ScriptForcedRead),
    Host(LockstepProof),
}

/// The right to send the owner a job of a layout frame that is not its forced read's one job, and wait for it. Only
/// these constructors mint one, so every extra job of a frame says why it is one.
pub(crate) struct FrameJobPermit {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl FrameJobPermit {
    const fn new() -> Self {
        Self {
            _not_send: std::marker::PhantomData,
        }
    }

    /// The first job of a rendering update's frame, which no forced read waits for.
    pub(crate) const fn of_rendering_update() -> Self {
        Self::new()
    }

    /// A job after one that ended with another round the owner does not start itself: the round has style to run, or
    /// a tree the document roots.
    pub(crate) const fn for_next_round() -> Self {
        Self::new()
    }

    /// The first job of a frame whose forced read a style transaction spent without running the job: only
    /// [`force_read`]'s answer holds one.
    const fn after_style() -> Self {
        Self::new()
    }
}

/// The right to send the owner a style transaction that is not a forced read's one job, and wait for it.
pub(crate) struct StyleJobPermit {
    _not_send: std::marker::PhantomData<*const ()>,
}

impl StyleJobPermit {
    const fn new() -> Self {
        Self {
            _not_send: std::marker::PhantomData,
        }
    }

    /// A transaction no forced read spent its read on: a style wave after an update's first, or a style update that
    /// runs outside a layout update.
    pub(crate) const fn of_style_update() -> Self {
        Self::new()
    }

    /// The finish of the transaction whose pass a rendering update submitted, once the update's frame is back.
    pub(crate) const fn finishing_submitted_pass() -> Self {
        Self::new()
    }
}

/// What a style or layout job's message carries to show that a typed entry of this module ([`force_read`],
/// [`run_frame_job`], [`run_style_transaction`]) spent the wait for it. Only this module makes one, so nothing outside
/// it sends such a job.
pub(crate) struct SpentWait(());

mod sealed_wait {
    pub trait Sealed {}
    impl Sealed for super::ScriptForcedRead {}
    impl Sealed for super::LockstepProof {}
    impl Sealed for super::SpentWait {}
}

/// What lets the main thread wait for the owner, taken by value by every wait: a script's forced read, a
/// [`LockstepProof`], or the [`SpentWait`] of a layout update's job, which only this module makes.
pub(crate) trait OwnerWait: sealed_wait::Sealed {}
impl OwnerWait for ScriptForcedRead {}
impl OwnerWait for LockstepProof {}
impl OwnerWait for SpentWait {}

/// The number of a change within its document's stream. The first change a document sends is 1; 0 names the point
/// before any change.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default)]
pub(crate) struct ChangeSeq(u64);

#[cfg(test)]
impl ChangeSeq {
    pub(crate) const fn nth(n: u64) -> Self {
        Self(n)
    }
}

/// The changes a document thread sent for a document: the number of the last one, and of the last one that alters the
/// rows the owner publishes; and what it sent that may move the document's layout.
#[derive(Default)]
struct SentChanges {
    through: ChangeSeq,
    altering_rows_through: ChangeSeq,
    moving_layout: LayoutMark,
}

/// What a document thread sent the owner of a document that may move the document's layout, as of a point: the last
/// change that does not keep a layout ([`ArenaChange::keeps_layout`]), and how many units and questions that may move
/// it the thread sent ([`ToOwner::may_move_layout`]). A layout the owner ran at that point stands as long as the mark
/// stays where it is.
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub(crate) struct LayoutMark {
    changes: ChangeSeq,
    units: u64,
}

/// A write the main thread makes to a document's layout arena, as owned data the owner applies in the order the main
/// thread made it, before the next unit or query that reaches the arena.
///
/// Adding a kind of write is adding a variant and its arm in [`ArenaChange::apply`]: the main thread sends it with
/// [`send_arena_change`]. A write to the document's style engine waits in the engine's home instead, for whoever
/// reaches the engine next ([`crate::css::style::StyleEngineInputHandle::send`]).
pub(crate) enum ArenaChange {
    /// Whether the document is an SVG file decoded as an image, which the layout of its SVG roots reads.
    DocumentIsDecodedSvg(bool),
    /// The scroll-state query inputs of the document's scroll containers, which style reads as one generation.
    StyleSnapshotScrollStates(Vec<crate::layout::style_snapshot::FfiLayoutStyleScrollState>),
    /// The document handed an image box the provider a finished layout frame owed it.
    OwnedProviderHandedOver(NodeSlotId),
    /// The host adopted what the clock's ticks installed in the arena ahead of it: what it did not adopt leaves the
    /// arena's log, with its pins.
    DropUnadoptedAnimationSamples,
    /// Whether the host listens for the boxes the document's nodes gain and lose.
    HostHearsBoxPresence(bool),
    /// A write to the document's layout marks or layout facts.
    Layout(crate::layout::layout_changes::LayoutChange),
    /// The host installed a style update whose transactions the owner applied the batches of to the layout nodes: a
    /// row the install did not adopt the record of is put back with the record its element holds, and what that owes
    /// the host goes with the next payment the owner hands it. Where `host_adopted_every_row`, the install took every
    /// row the owner applied, so none is put back.
    FinishOwnerStyleHostHalf { host_adopted_every_row: bool },
    /// The `::selection` style of an element changed: the subtree of its nearest painted ancestor in the ancestry the
    /// layout tree was built from (a `display: contents` element has no box of its own) paints again, so cached text
    /// commands take the new highlight.
    SelectionStyleChanged(StyleNodeID),
    /// An element published a `::selection` style, which the rows that paint text under it take.
    SelectionPseudoStylePublished(StyleNodeID),
    /// Each layout pass and formatting context run from now on leaves a line in the layout trace, for tests.
    BeginLayoutTrace,
    /// The host took the layout trace: tracing ends.
    EndLayoutTrace,
    /// What an SVG element's presentation attributes parse to, with the points of a `<polyline>` or `<polygon>`.
    SvgAttributeFacts {
        element: StyleNodeID,
        facts: Box<crate::layout::svg_formatting_context::FfiSvgAttributeFacts>,
        points: Vec<crate::layout::svg_formatting_context::FfiFloatPoint>,
    },
    /// The resources an SVG graphics element's style names: its mask, clip path, fill and stroke.
    SvgStyleReferences { element: StyleNodeID, references: [u32; 4] },
    /// An SVG element left the document with what its presentation attributes parse to.
    SvgAttributeFactsCleared(StyleNodeID),
    /// The arena's nodes take their style from the document's style engine.
    LinkStyleEngine(crate::layout::StyleEngineLink),
    /// The host is about to destroy the document's style engine.
    UnlinkStyleEngine,
    /// The anchor names registration moved in the document's style engine are published to the arena.
    PublishAnchorNames,
    /// A write to the document's paint state.
    Paint(crate::painting::paint_changes::PaintChange),
    /// The counter styles a tree scope registers, which replace the ones it registered before.
    CounterStyles {
        tree_scope: u32,
        scope: crate::css::counter_representation::CounterStyleScope,
    },
}

impl ArenaChange {
    /// Whether applying the change can alter what the rows the owner publishes answer the document thread (see
    /// [`crate::layout::row_reads`]), so that a read the thread makes after sending it waits for rows that reflect it.
    /// What the next layout or paint reads, and what the arena keeps for the host, alter none.
    fn alters_published_rows(&self) -> bool {
        match self {
            ArenaChange::Layout(change) => change.alters_published_rows(),
            ArenaChange::Paint(change) => change.alters_published_rows(),
            // A row the install did not adopt the record of takes another style.
            ArenaChange::FinishOwnerStyleHostHalf { host_adopted_every_row } => !host_adopted_every_row,
            ArenaChange::DocumentIsDecodedSvg(_)
            | ArenaChange::StyleSnapshotScrollStates(_)
            | ArenaChange::OwnedProviderHandedOver(_)
            | ArenaChange::DropUnadoptedAnimationSamples
            | ArenaChange::HostHearsBoxPresence(_)
            | ArenaChange::SelectionStyleChanged(_)
            | ArenaChange::BeginLayoutTrace
            | ArenaChange::EndLayoutTrace
            | ArenaChange::SvgAttributeFacts { .. }
            | ArenaChange::SvgStyleReferences { .. }
            | ArenaChange::SvgAttributeFactsCleared(_)
            | ArenaChange::LinkStyleEngine(_)
            | ArenaChange::UnlinkStyleEngine
            | ArenaChange::SelectionPseudoStylePublished(_)
            | ArenaChange::PublishAnchorNames
            | ArenaChange::CounterStyles { .. } => false,
        }
    }

    /// Whether applying the change leaves a layout the owner ran before it as it is: it neither alters the rows the owner
    /// publishes nor marks layout, and moves nothing a layout reads. A layout frame's job that rode a style transaction
    /// stands only if the document thread sends nothing else after it (see [`LayoutMark`]).
    fn keeps_layout(&self) -> bool {
        match self {
            ArenaChange::Layout(change) => change.keeps_layout(),
            ArenaChange::Paint(change) => !change.alters_published_rows(),
            ArenaChange::FinishOwnerStyleHostHalf { host_adopted_every_row } => *host_adopted_every_row,
            ArenaChange::DropUnadoptedAnimationSamples
            | ArenaChange::HostHearsBoxPresence(_)
            | ArenaChange::SelectionStyleChanged(_)
            | ArenaChange::SelectionPseudoStylePublished(_)
            | ArenaChange::BeginLayoutTrace
            | ArenaChange::EndLayoutTrace => true,
            ArenaChange::DocumentIsDecodedSvg(_)
            | ArenaChange::StyleSnapshotScrollStates(_)
            | ArenaChange::OwnedProviderHandedOver(_)
            | ArenaChange::SvgAttributeFacts { .. }
            | ArenaChange::SvgStyleReferences { .. }
            | ArenaChange::SvgAttributeFactsCleared(_)
            | ArenaChange::LinkStyleEngine(_)
            | ArenaChange::UnlinkStyleEngine
            | ArenaChange::PublishAnchorNames
            | ArenaChange::CounterStyles { .. } => false,
        }
    }

    /// The engine the change links the arena to, which applying it reaches.
    fn linked_engine(&self) -> Option<crate::layout::StyleEngineHold> {
        match self {
            ArenaChange::LinkStyleEngine(link) => Some(link.hold()),
            _ => None,
        }
    }

    /// Applies the change to `arena`, with the engine the arena links, or the one the change links it to, where the
    /// unit reaches one. An unlink returns the arena's link to the engine, which may go away with it: the caller drops
    /// it once its reach of the engine has ended.
    #[must_use]
    fn apply(
        self,
        arena: &mut LayoutNodeArena,
        engine: Option<&mut crate::css::style::StyleEngine>,
    ) -> Option<crate::layout::StyleEngineLink> {
        match self {
            ArenaChange::DocumentIsDecodedSvg(is_decoded_svg) => arena.set_document_is_decoded_svg(is_decoded_svg),
            ArenaChange::StyleSnapshotScrollStates(states) => arena.publish_style_snapshot_scroll_states(&states),
            ArenaChange::OwnedProviderHandedOver(row) => {
                if arena.slot_is_live(row) {
                    arena.note_owned_provider_handed_over(row);
                }
            }
            ArenaChange::DropUnadoptedAnimationSamples => arena.drop_animation_adoptions(),
            ArenaChange::HostHearsBoxPresence(hears) => arena.set_host_hears_box_presence(hears),
            ArenaChange::Layout(change) => change.apply(arena),
            ArenaChange::FinishOwnerStyleHostHalf { host_adopted_every_row } => {
                arena.finish_owner_style_host_half(host_adopted_every_row);
            }
            ArenaChange::SelectionStyleChanged(element) => {
                crate::painting::selection::repaint_after_selection_style_change(arena, element);
            }
            ArenaChange::SelectionPseudoStylePublished(element) => {
                crate::painting::selection::sync_selection_pseudo_style(arena, element);
            }
            ArenaChange::BeginLayoutTrace => arena.layout_trace().begin(),
            ArenaChange::EndLayoutTrace => arena.layout_trace().end(),
            ArenaChange::SvgAttributeFacts { element, facts, points } => {
                arena.set_style_node_svg_attribute_facts(element, facts, &points);
            }
            ArenaChange::SvgStyleReferences { element, references } => {
                arena.set_style_node_svg_style_references(element, references);
            }
            ArenaChange::SvgAttributeFactsCleared(element) => arena.clear_style_node_svg_attribute_facts(element),
            ArenaChange::LinkStyleEngine(link) => match engine {
                Some(engine) => arena.link_style_engine(link, engine),
                None => debug_assert!(false, "linking the style engine reaches it"),
            },
            ArenaChange::UnlinkStyleEngine => return arena.unlink_style_engine(),
            ArenaChange::PublishAnchorNames => match engine {
                Some(engine) => engine.publish_anchor_names(arena),
                None => debug_assert!(false, "publishing anchor names reaches the engine"),
            },
            ArenaChange::Paint(change) => change.apply(arena),
            ArenaChange::CounterStyles { tree_scope, scope } => arena.publish_counter_styles(tree_scope, scope),
        }
        None
    }
}

/// The right to act as the render owner: to run units over the render states it owns, and reach the style engines
/// they link. Only the thread that handles the owner's messages, and so holds its render states, has one, for as long
/// as it handles one; it cannot leave that thread.
pub(crate) struct Owner(std::marker::PhantomData<*const ()>);

impl Owner {
    /// The owner, on the thread that handles its messages or holds the render state of the document at hand: the
    /// Rendering thread, the thread the messages are handled on where there is none, or a document thread that does
    /// the owner's work itself where a test holds the owner's run.
    fn here() -> Self {
        Self(std::marker::PhantomData)
    }
}

/// Runs `op` as the owner, on a thread that waits for the owner and does the owner's work itself where the owner
/// cannot: see [`crate::stage_thread::wait_for_owner`].
pub(crate) fn do_owner_work_here<R>(op: impl FnOnce(&Owner) -> R) -> R {
    op(&Owner::here())
}

/// How a unit that applies a document's changes reaches its style engine.
enum EngineReach<'a> {
    /// As the owner.
    Owner(&'a Owner),
    /// As the owner, beside the main thread, which does not wait for it.
    OwnerBesideMain(&'a Owner),
    /// The unit runs beside the main thread, which lent it the engine.
    Lent(&'a mut crate::css::style::engine_home::StyleEngineLoan),
}

impl EngineReach<'_> {
    fn reach<T>(
        &mut self,
        engine: &crate::layout::StyleEngineHold,
        run: impl FnOnce(&mut crate::css::style::StyleEngine) -> T,
    ) -> T {
        match self {
            // SAFETY: The engine is the document's, whose render state the owner holds.
            Self::Owner(owner) => unsafe { engine.reach_on_owner(owner, run) },
            // SAFETY: As above.
            Self::OwnerBesideMain(owner) => unsafe { engine.reach_on_owner_beside_main(owner, run) },
            Self::Lent(loan) => loan.lend_to_this_thread(run),
        }
    }
}

/// A document's arena changes the owner has received and not applied yet, in order.
#[derive(Default)]
struct ChangeQueue {
    received_through: ChangeSeq,
    pending: Vec<ArenaChange>,
}

impl ChangeQueue {
    /// Receives changes `first`, `first + 1`, ...
    fn receive(&mut self, first: ChangeSeq, changes: Vec<ArenaChange>) {
        debug_assert_eq!(
            first.0,
            self.received_through.0 + 1,
            "a document's changes reach the owner in order"
        );
        self.received_through = ChangeSeq(first.0 + changes.len() as u64 - 1);
        if self.pending.is_empty() {
            self.pending = changes;
        } else {
            self.pending.extend(changes);
        }
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
    take_in: TakeIn,
}

/// When the owner takes in what a document thread sent: as it idles, or only in a unit the thread waits for.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TakeIn {
    /// The document thread goes on beside the owner, which takes in what it sent as it idles.
    AsTheOwnerIdles,
    /// The thread that sends the changes holds the document's style engine itself (a unit test's, the replay tool's):
    /// nothing may reach the engine beside it.
    WhenWaitedFor,
}

impl RenderState {
    /// Applies the changes the owner has received, which every unit and query comes after. What the main thread wrote
    /// to the style engine the reach of the engine applies first.
    fn apply_changes(&mut self, mut reach: EngineReach<'_>) {
        if self.changes.pending.is_empty() {
            return;
        }
        let changes = std::mem::take(&mut self.changes.pending);
        // Linking the arena reaches the engine it links. Putting back the rows a style install did not adopt, and
        // retaining the atoms an SVG publication names, reach the engine through the arena.
        let engine = changes
            .iter()
            .find_map(ArenaChange::linked_engine)
            .or_else(|| self.arena.arena().hold_style_engine());
        let face_owner = std::ptr::from_mut::<ArenaHandle>(&mut self.arena) as u64;
        // A display tick that ran after the host took what the clock's ticks left installed samples the host has not
        // taken: what it did not adopt before goes with a later drop.
        let keeps_unadopted = self
            .clock
            .as_ref()
            .is_some_and(crate::clock_frames::DocumentClock::left_samples_to_adopt);
        let arena = self.arena.arena_mut();
        let mut unlinked = None;
        let apply = |arena: &mut LayoutNodeArena, mut engine: Option<&mut crate::css::style::StyleEngine>| {
            for change in changes {
                match change {
                    ArenaChange::DropUnadoptedAnimationSamples if keeps_unadopted => {}
                    change => {
                        if let Some(link) = change.apply(arena, engine.as_deref_mut()) {
                            unlinked = Some(link);
                        }
                    }
                }
            }
        };
        match &engine {
            None => apply(arena, None),
            Some(engine) => {
                // The faces what the main thread wrote to the engine wants are this document's, whichever document's
                // unit the owner serves the changes beside.
                let _wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(face_owner);
                reach.reach(engine, |engine| apply(arena, Some(engine)));
            }
        }
        // The engine the arena no longer links goes away, if the main thread let go of it, once its reach has ended.
        drop(unlinked);
        drop(engine);
        arena.note_changes_taken_in(self.changes.received_through);
    }

    /// Answers `query` from the state as the units before it left it.
    fn answer(&mut self, owner: &Owner, query: Query) -> Answer {
        if query.publishes_rows() {
            self.arena.let_go_of_rows_while_document_thread_waits();
        }
        self.apply_changes(EngineReach::Owner(owner));
        match query {
            Query::Engine(query) => {
                let Some(engine) = self.arena.arena().hold_style_engine() else {
                    debug_assert!(
                        false,
                        "the owner answers the engine queries of a document with an engine"
                    );
                    return Answer::left_to_host(Query::Engine(query));
                };
                // The faces a read's style computation wants are this document's.
                let _wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(std::ptr::from_mut::<ArenaHandle>(
                    &mut self.arena,
                ) as u64);
                // SAFETY: The engine is the document's, and the document thread waits for the answer, keeping what
                // the query borrows live.
                unsafe { engine.reach_on_owner(owner, |engine| query.answer(engine, self.arena.arena())) };
                Answer::Engine(EngineAnswered::Answered)
            }
            _ => Answer::of_state_reaching_engine(owner, query, &mut self.arena),
        }
    }

    /// The document's style engine, which the arena links (null before it links one), for what the owner tells of it
    /// without reaching it. The units the owner runs reach it through a hold of the arena's link
    /// ([`crate::layout::StyleEngineHold::reach_on_owner`]).
    fn style_engine(&self) -> crate::css::style::StyleEngineHandle {
        self.arena.arena().style_engine_handle()
    }

    /// The state's arena and what lives beside it, which the owner hands its units.
    fn state(&mut self, owner: &Owner) -> *mut ArenaHandle {
        self.apply_changes(EngineReach::Owner(owner));
        std::ptr::from_mut::<ArenaHandle>(&mut self.arena)
    }

    /// The state's arena and what lives beside it, for a unit the document thread waits for, which publishes the rows
    /// before it answers: the rows published last are let go of before the unit, and the changes it applies first,
    /// write what they shared.
    fn state_for_waiting_thread(&mut self, owner: &Owner) -> *mut ArenaHandle {
        self.arena.let_go_of_rows_while_document_thread_waits();
        self.state(owner)
    }

    /// The state's arena and what lives beside it, for a rendering update, which runs beside the main thread with the
    /// style engine lent to it as `style_engine`.
    fn state_beside_main_thread(
        &mut self,
        owner: &Owner,
        style_engine: Option<&mut crate::css::style::engine_home::StyleEngineLoan>,
    ) -> *mut ArenaHandle {
        match style_engine {
            Some(loan) => self.apply_changes(EngineReach::Lent(loan)),
            // Only an update of a document with no engine is lent none, and its changes reach no engine.
            None if self.style_engine().is_null() => self.apply_changes(EngineReach::Owner(owner)),
            // The changes wait for the next unit the main thread waits for.
            None => debug_assert!(false, "a rendering update is lent its document's style engine"),
        }
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
    /// A read of the document's layout arena.
    Arena(ArenaQuery),
    /// A read of the document's style engine, which the owner answers into the query the main thread holds.
    Engine(crate::css::style::owner_calls::StyleQueryRef),
    /// A write to the document's layout tree the main thread waits for, answered with what it owes the host.
    Write(crate::layout::layout_changes::LayoutWrite),
    /// The document's rows as of every change the main thread sent, which the owner publishes: with the scrollable
    /// overflow a commit or a writer left measured first where `measured_overflow`.
    CommittedRows { measured_overflow: bool },
    /// A read for tests and debugging, which only Internals and the WebContent debug requests ask.
    DevTools(devtools::DevToolsQuery),
}

impl Query {
    /// Whether the owner publishes the rows before it answers the query.
    fn publishes_rows(&self) -> bool {
        matches!(
            self,
            Self::Geometry { .. } | Self::Write(_) | Self::CommittedRows { .. }
        )
    }
}

/// A read of a document's layout arena, which [`Query::Arena`] asks.
#[derive(Clone, Copy, Debug)]
pub(crate) enum ArenaQuery {
    /// The text a pseudo-element's generated content resolved to when its box was built: its alt text when it has one,
    /// otherwise every string in it.
    GeneratedContentAccessibleText(crate::layout::counters::CounterOwner),
    /// The text the rows of the text node whose primary row is `primary` render, with whitespace collapsed where their
    /// style collapses it if `collapse_whitespace`.
    RenderedText {
        primary: NodeSlotId,
        collapse_whitespace: bool,
    },
    /// The DOM range of the word at `dom_offset` in the text the rows of the text node whose primary row is `primary`
    /// render.
    WordRange { primary: NodeSlotId, dom_offset: usize },
    /// The text rows below `viewport` find-in-page searches, where the document has no searchable text for it.
    SearchCandidates { viewport: NodeSlotId },
    /// Where `query` occurs in the searchable text below `viewport`, which leaves out the `excluded` search candidates
    /// where it is built for the query.
    FindText {
        viewport: NodeSlotId,
        query: LentSlice<u16>,
        case_sensitive: bool,
        excluded: LentSlice<NodeSlotId>,
    },
    /// The SVG-as-image renders the next recording, which reads the given inputs, is predicted to paint: the ones the
    /// last recording painted or missed, and the first paints of the rows it records afresh.
    PaintedVectorImages {
        css_viewport_rect: crate::layout::used_values::FfiCssPixelRect,
        document_declares_light_or_dark_color_scheme: bool,
        image_color_scheme_fallback: u8,
    },
}

/// A slice the document thread lends the owner with a query it waits for the answer to.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LentSlice<T: 'static>(std::ptr::NonNull<[T]>);

// SAFETY: The document thread waits for the answer to the query that carries the slice, which keeps it live and
// unwritten until the owner has answered.
unsafe impl<T: Sync> Send for LentSlice<T> {}

impl<T> LentSlice<T> {
    pub(crate) fn new(slice: &[T]) -> Self {
        Self(std::ptr::NonNull::from(slice))
    }

    /// # Safety
    ///
    /// Only in answering the query that carries the slice, which the document thread waits for.
    unsafe fn get<'a>(self) -> &'a [T] {
        // SAFETY: Guaranteed by the caller.
        unsafe { self.0.as_ref() }
    }
}

/// The answer to an [`ArenaQuery`].
#[derive(Debug)]
pub(crate) enum ArenaAnswer {
    Text(Vec<u16>),
    Range(crate::layout::rendered_text::FfiTextSourceRange),
    Rows(Vec<NodeSlotId>),
    TextRanges(Vec<crate::layout::text_queries::FfiDomTextRange>),
    VectorImages(Vec<crate::painting::record::vector_images::VectorImageRenderRequest>),
}

impl ArenaQuery {
    fn left_to_host(self) -> ArenaAnswer {
        match self {
            ArenaQuery::GeneratedContentAccessibleText(_) | ArenaQuery::RenderedText { .. } => {
                ArenaAnswer::Text(Vec::new())
            }
            ArenaQuery::WordRange { dom_offset, .. } => {
                ArenaAnswer::Range(crate::layout::rendered_text::FfiTextSourceRange {
                    start: dom_offset,
                    length: 0,
                })
            }
            ArenaQuery::SearchCandidates { .. } => ArenaAnswer::Rows(Vec::new()),
            ArenaQuery::FindText { .. } => ArenaAnswer::TextRanges(Vec::new()),
            ArenaQuery::PaintedVectorImages { .. } => ArenaAnswer::VectorImages(Vec::new()),
        }
    }

    /// Readies `arena` to answer the query from.
    fn prepare(self, arena: &mut LayoutNodeArena) {
        match self {
            ArenaQuery::RenderedText { primary, .. } | ArenaQuery::WordRange { primary, .. } => {
                arena.sync_text_fragments(primary);
            }
            ArenaQuery::FindText { viewport, excluded, .. } => {
                // SAFETY: The document thread waits for the answer.
                arena.prepare_searchable_text(viewport, unsafe { excluded.get() });
            }
            _ => {}
        }
    }

    fn answer(self, arena: &LayoutNodeArena) -> ArenaAnswer {
        match self {
            ArenaQuery::GeneratedContentAccessibleText(owner) => {
                ArenaAnswer::Text(arena.generated_content().borrow().accessible_text(owner).to_vec())
            }
            ArenaQuery::RenderedText {
                primary,
                collapse_whitespace,
            } => ArenaAnswer::Text(arena.rendered_text(primary, collapse_whitespace)),
            ArenaQuery::WordRange { primary, dom_offset } => {
                ArenaAnswer::Range(arena.text_word_range(primary, dom_offset))
            }
            ArenaQuery::SearchCandidates { viewport } => ArenaAnswer::Rows(arena.search_candidates(viewport)),
            ArenaQuery::FindText {
                query, case_sensitive, ..
            } => {
                // SAFETY: The document thread waits for the answer.
                ArenaAnswer::TextRanges(arena.matching_text(unsafe { query.get() }, case_sensitive))
            }
            ArenaQuery::PaintedVectorImages {
                css_viewport_rect,
                document_declares_light_or_dark_color_scheme,
                image_color_scheme_fallback,
            } => ArenaAnswer::VectorImages(crate::painting::record::vector_images::painted_vector_images(
                arena,
                css_viewport_rect,
                document_declares_light_or_dark_color_scheme,
                image_color_scheme_fallback,
            )),
        }
    }
}

/// The answer to a [`Query`], of the variant the query asked for.
#[derive(Debug)]
pub(crate) enum Answer {
    Geometry(FfiGeometryReadAnswer),
    Arena(ArenaAnswer),
    Engine(EngineAnswered),
    Payment(crate::layout::HostPayment),
    /// The owner published the rows the document thread reads.
    Published,
    DevTools(devtools::DevToolsAnswer),
}

/// What became of a [`Query::Engine`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EngineAnswered {
    /// The owner wrote the answer into the query.
    Answered,
    /// The owner holds no engine of the document to answer with: the main thread answers the query.
    LeftToHost,
    /// Nothing answered the query: the document has no owner. Nothing answers it again.
    Unanswered,
}

impl Answer {
    /// The answer that leaves the question to the main thread, as it answered it before the owner did.
    fn left_to_host(query: Query) -> Self {
        match query {
            Query::Geometry { .. } => Self::Geometry(FfiGeometryReadAnswer::default()),
            Query::Arena(query) => Self::Arena(query.left_to_host()),
            Query::Engine(_) => Self::Engine(EngineAnswered::LeftToHost),
            Query::Write(_) => Self::Payment(crate::layout::HostPayment::nothing()),
            Query::CommittedRows { .. } => Self::Published,
            Query::DevTools(query) => Self::DevTools(query.left_to_host()),
        }
    }

    /// The answer to a question nobody answered: the document has no owner. A question of the engine is not left to
    /// the main thread to answer from there.
    fn unanswered(query: Query) -> Self {
        match query {
            Query::Engine(_) => Self::Engine(EngineAnswered::Unanswered),
            _ => Self::left_to_host(query),
        }
    }

    /// Readies `arena` to answer `query` from: a geometry read reads the rows as committed, which publishes what the
    /// units before it wrote.
    pub(crate) fn prepare(query: Query, arena: &mut LayoutNodeArena) {
        match query {
            Query::Geometry { .. }
            | Query::CommittedRows {
                measured_overflow: true,
            } => arena.publish_committed_rows(),
            Query::CommittedRows {
                measured_overflow: false,
            } => arena.publish_rows(),
            Query::Arena(query) => query.prepare(arena),
            _ => {}
        }
    }

    /// Answers `query` from `state` as [`Self::of_state`] does, with the document's style engine lent to the calling
    /// thread, as a question may read the engine (the source of a text's rendered text, the counter styles of generated
    /// content). The document thread waits for the answer.
    fn of_state_reaching_engine(owner: &Owner, query: Query, state: &mut ArenaHandle) -> Self {
        let Some(engine) = state.arena().hold_style_engine() else {
            return Self::of_state(query, state);
        };
        // SAFETY: The engine is the document's, whose render state the owner holds.
        unsafe { engine.reach_on_owner(owner, |_| Self::of_state(query, state)) }
    }

    /// Answers `query` from the arena of `state` and the layout scratch beside it, which it readies first, or makes
    /// the write it is, publishing the rows it changed.
    fn of_state(query: Query, state: &mut ArenaHandle) -> Self {
        match query {
            Query::Write(write) => {
                let arena = state.arena_mut();
                let payment = write.apply(arena);
                arena.publish_rows();
                Self::Payment(payment)
            }
            Query::DevTools(query) => Self::DevTools(query.answer(state)),
            _ => {
                let arena = state.arena_mut();
                Self::prepare(query, arena);
                Self::of(query, arena)
            }
        }
    }

    /// Answers `query` from `arena` alone, which [`Self::prepare`] readied. A question reads the arena, and writes
    /// nothing of it. A question the engine answers is left to the main thread.
    pub(crate) fn of(query: Query, arena: &LayoutNodeArena) -> Self {
        match query {
            Query::Geometry { node, kind } => Self::Geometry(answer_geometry(arena, node, kind)),
            Query::Arena(query) => Self::Arena(query.answer(arena)),
            Query::Engine(_) => Self::left_to_host(query),
            Query::Write(_) | Query::DevTools(_) => {
                debug_assert!(
                    false,
                    "a write or a devtools read is answered with the state, not from the arena"
                );
                Self::left_to_host(query)
            }
            Query::CommittedRows { .. } => Self::Published,
        }
    }
}

/// The stages of one rendering update of a document, as the main thread prepared them. The owner runs them unit by
/// unit (style, the layout rounds, paint preparation, the recording) and regains control after each: where the main
/// thread recalls it ([`ToOwner::Recall`]), the update ends there, and the main thread goes on from where it ended.
pub(crate) struct RenderingUpdate {
    flight: crate::flight::Flight,
    style_engine: Option<crate::css::style::engine_home::StyleEngineLoan>,
    /// The frame the update is, and where the owner posts its news.
    seq: crate::frame_news::FrameSeq,
    news: crate::frame_news::NewsSender,
    /// How the owner runs it, with the render state of its document, where the owner holds one. The owner reaches
    /// the pipeline only through the updates it is sent, so what reaches the owner without reaching the pipeline (the
    /// unit tests' stage threads) links without it.
    run: fn(Self, &Owner, Option<*mut ArenaHandle>),
}

impl RenderingUpdate {
    pub(crate) fn new(
        flight: crate::flight::Flight,
        style_engine: Option<crate::css::style::engine_home::StyleEngineLoan>,
        seq: crate::frame_news::FrameSeq,
        news: crate::frame_news::NewsSender,
    ) -> Self {
        Self {
            flight,
            style_engine,
            seq,
            news,
            run: Self::run_flight,
        }
    }

    fn run(self: Box<Self>, owner: &Owner, state: Option<*mut ArenaHandle>) {
        (self.run)(*self, owner, state);
    }

    fn run_flight(self, owner: &Owner, state: Option<*mut ArenaHandle>) {
        let (outcome, ran) = self.flight.run(owner, self.style_engine, state);
        self.news.post(crate::frame_news::FrameNews::FlightEnded {
            seq: self.seq,
            outcome,
            ran,
        });
    }
}

/// A message to the owner. All documents share one FIFO, since a child document's layout reads its container's.
pub(crate) enum ToOwner {
    /// Takes in the render state of `document`, whose arena the document thread took from the spare the owner built
    /// ahead of it ([`create_document`]), and builds the next spare.
    Create {
        document: DocumentId,
        arena: SpareArena,
        take_in: TakeIn,
    },
    /// Changes `first`, `first + 1`, ... of `document`.
    Changes {
        document: DocumentId,
        first: ChangeSeq,
        changes: Vec<ArenaChange>,
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
        reply: crate::stage_thread::OwnerReplyTo<crate::layout::update_layout::OwnerFrameJobAnswer>,
        /// What shows that [`force_read`] or [`run_frame_job`] sent the job.
        _spent: SpentWait,
    },
    /// Runs a paint preparation pass over the render state of `document` for the document thread, which waits for it.
    Paint {
        document: DocumentId,
        pass: Box<crate::painting::owner_pass::OwnerPaintPass>,
    },
    /// Runs the style transaction `transaction` of `document`, which the document thread takes and waits for: begins
    /// it, runs its pass and finishes it. Where the transaction leaves the document thread nothing a layout reads, the
    /// first job of the layout frame the transaction's round is for, `then_layout`, runs right after it, and answers
    /// with it.
    Style {
        document: DocumentId,
        transaction: Box<crate::css::style::bridge::OwnerStyleTransaction>,
        then_layout: Option<Box<crate::layout::update_layout::OwnerFrameJob>>,
        reply: crate::stage_thread::OwnerReplyTo<StyleJobAnswer>,
        /// What shows that [`force_read`] or [`run_style_transaction`] sent the transaction.
        _spent: SpentWait,
    },
    /// Answers `query` about `document` after the changes sent before it. The document thread waits.
    Ask {
        document: DocumentId,
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
    /// Panics on the owner as it answers the document thread, which waits: a test's owner that dies.
    PanicForTesting {
        document: DocumentId,
        reply: crate::stage_thread::OwnerReplyTo<()>,
    },
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
            | Self::Destroy { document }
            | Self::PanicForTesting { document, .. } => *document,
            Self::Clock(message) => message.document(),
        }
    }

    /// Whether handling the message may move the layout of its document: it runs units over its arena, or writes its
    /// layout tree. A change counts by itself ([`ArenaChange::keeps_layout`]), and a question only reads.
    fn may_move_layout(&self) -> bool {
        match self {
            Self::RenderingUpdate { .. } | Self::Layout { .. } | Self::Paint { .. } | Self::Style { .. } => true,
            Self::Ask { query, .. } => matches!(query, Query::Write(_)),
            Self::Create { .. }
            | Self::Changes { .. }
            | Self::Recall { .. }
            | Self::Destroy { .. }
            | Self::Clock(_)
            | Self::PanicForTesting { .. } => false,
        }
    }

    /// Whether the owner applies the arena changes it received before handling the message.
    fn reaches_arena(&self) -> bool {
        matches!(
            self,
            Self::RenderingUpdate { .. }
                | Self::Style { .. }
                | Self::Layout { .. }
                | Self::Paint { .. }
                | Self::Ask { .. }
        )
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
                | Self::PanicForTesting { .. }
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
    static STATES: RefCell<HashMap<DocumentId, RenderState>> = RefCell::default();
    // On the owner thread, the documents whose rendering update the document thread recalled before the update began,
    // which the update ends at its first unit.
    static RECALLED: RefCell<crate::fast_hash::FastSet<DocumentId>> = RefCell::default();
    // On a document thread, the number of the last change it sent for each document, and of the last one that alters the
    // rows the owner publishes.
    static SENT_THROUGH: RefCell<HashMap<DocumentId, SentChanges>> = RefCell::default();
    // On a document thread, the number of the last change it sent for each document ahead of a unit or question that
    // reaches the document's arena, which the owner applies every change it received before.
    static TAKEN_IN_THROUGH: RefCell<HashMap<DocumentId, ChangeSeq>> = RefCell::default();
    // On a document thread, the changes it sent that wait to go to the owner together.
    static HELD: RefCell<HeldChanges> = RefCell::new(HeldChanges::default());
    // On a document thread, the address of each document's arena it created, which names the frame in flight of the
    // document: nothing reaches the arena through it.
    static FRAME_KEYS: RefCell<HashMap<DocumentId, usize>> = RefCell::default();
}

/// Handles `message`, on the owner thread. A panic in handling it ends that message, not the owner thread: a document
/// thread that waits for an answer finds its reply dropped unanswered, and ends the process.
pub(crate) fn handle(message: ToOwner) {
    let owner = Owner::here();
    if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| handle_message(&owner, message))).is_err() {
        debug_assert!(false, "the render owner panicked handling a message");
    }
}

fn handle_message(owner: &Owner, message: ToOwner) {
    match message {
        ToOwner::Create {
            document,
            arena,
            take_in,
        } => {
            let state = RenderState {
                arena: arena.0,
                changes: ChangeQueue::default(),
                clock: None,
                take_in,
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
            with_state(document, |state| state.changes.receive(first, changes));
        }
        ToOwner::RenderingUpdate {
            document,
            mut update,
            ticket,
        } => {
            debug_assert!(
                !document.is_valid() || STATES.with_borrow(|states| states.contains_key(&document)),
                "a rendering update of a document with no render state"
            );
            // A test's update of no document runs with the arena its flight names.
            let state = STATES.with_borrow_mut(|states| {
                states
                    .get_mut(&document)
                    .map(|state| state.state_beside_main_thread(owner, update.style_engine.as_mut()))
            });
            ticket.run(|| update.run(owner, state));
        }
        ToOwner::Style {
            document,
            transaction,
            then_layout,
            reply,
            ..
        } => reply.answer(|| run_style_job_on_owner(owner, document, transaction, then_layout)),
        ToOwner::Layout {
            document, job, reply, ..
        } => {
            // The state's borrow ends before the job runs, which may reach another document's state. The job finds
            // the arena inside its answer, so that a panic there answers the waiting document thread.
            reply.answer(|| {
                (*job).run(owner, || {
                    with_state(document, |state| state.state_for_waiting_thread(owner))
                })
            });
        }
        ToOwner::Paint { document, pass } => {
            // As for a layout unit, the pass finds the arena inside its answer.
            (*pass).run(owner, || {
                with_state(document, |state| state.state_for_waiting_thread(owner))
            });
        }
        ToOwner::Ask { document, query, reply } => reply.answer(|| {
            with_state(document, |state| state.answer(owner, query)).unwrap_or_else(|| Answer::left_to_host(query))
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
        ToOwner::Clock(message) => crate::clock_frames::handle_on_owner(owner, message),
        ToOwner::PanicForTesting { reply, .. } => reply.answer(|| panic!("the render owner panicked for a test")),
    }
}

/// On the owner thread, with no message waiting: applies what the document threads sent each render state, which the
/// state queued for the next unit or question that reaches it. The owner takes it in beside the document thread, which
/// goes on writing, so the unit it waits for next finds it applied.
pub(crate) fn take_in_sent_changes() {
    let owner = Owner::here();
    STATES.with_borrow_mut(|states| {
        for state in states
            .values_mut()
            .filter(|state| state.take_in == TakeIn::AsTheOwnerIdles)
        {
            state.arena.arena_mut().drop_rows_let_go();
            // Only the owner leaves news, so what the main thread has adopted stays adopted until the reach below.
            let engine = state.style_engine();
            if !engine.is_null() && engine.has_unadopted_news() {
                continue;
            }
            state.apply_changes(EngineReach::OwnerBesideMain(&owner));
            // What the main thread sent the engine with no arena change beside it, a reach of the engine applies.
            if let Some(engine) = state.arena.arena().hold_style_engine()
                && engine.handle().has_unapplied_changes()
            {
                // SAFETY: The engine is the document's, whose render state the owner holds.
                unsafe { engine.reach_on_owner_beside_main(&owner, |_| ()) };
            }
        }
    });
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

/// On the owner thread: a hold of the style engine the arena of `document`'s render state links.
pub(crate) fn style_engine_of(document: DocumentId) -> Option<crate::layout::StyleEngineHold> {
    with_state(document, |state| state.arena.arena().hold_style_engine()).flatten()
}

/// On the owner thread: runs `operation` on the clock slot of `document`'s render state, leaving its arena alone.
pub(crate) fn with_clock_slot<R>(
    document: DocumentId,
    operation: impl FnOnce(&mut Option<crate::clock_frames::DocumentClock>) -> R,
) -> Option<R> {
    with_state(document, |state| operation(&mut state.clock))
}

/// On the owner thread: runs `operation` on the clock slot of `document`'s render state, with the arena it ticks, which
/// the render state keeps alive.
pub(crate) fn with_clock<R>(
    owner: &Owner,
    document: DocumentId,
    operation: impl FnOnce(&mut Option<crate::clock_frames::DocumentClock>, *mut ArenaHandle) -> R,
) -> Option<R> {
    with_state(document, |state| {
        let arena = state.state(owner);
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
            .find(|(_, state)| state.clock.as_ref().is_some_and(|clock| clock.ticks_at(context)))
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
pub(crate) fn create_document(take_in: TakeIn) -> (DocumentId, *mut c_void) {
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
        take_in,
    });
    (document, address)
}

/// Drops the render state of `document` on the owner. Nothing waits for it.
pub(crate) fn destroy_document(document: DocumentId) {
    SENT_THROUGH.with_borrow_mut(|sent| sent.remove(&document));
    TAKEN_IN_THROUGH.with_borrow_mut(|taken_in| taken_in.remove(&document));
    FRAME_KEYS.with_borrow_mut(|keys| keys.remove(&document));
    send(ToOwner::Destroy { document });
}

/// Sends the arena write `change` for `document`, which the owner applies before the next unit or query that reaches
/// the arena. Inside a change batch ([`render_owner_open_change_batch`]), the change waits to go with the rest of it.
pub(crate) fn send_arena_change(document: DocumentId, change: ArenaChange) -> ChangeSeq {
    let alters_published_rows = change.alters_published_rows();
    let keeps_layout = change.keeps_layout();
    let seq = SENT_THROUGH.with_borrow_mut(|sent| {
        let sent = sent.entry(document).or_default();
        sent.through.0 += 1;
        if alters_published_rows {
            sent.altering_rows_through = sent.through;
        }
        if !keeps_layout {
            sent.moving_layout.changes = sent.through;
        }
        sent.through
    });
    // What the thread holds of another document goes first, so the owner receives every change in the order sent.
    if HELD.with_borrow(|held| !held.changes.is_empty() && held.document != document) {
        send_held_changes();
    }
    let holds_on = HELD.with_borrow_mut(|held| {
        if held.changes.is_empty() {
            held.document = document;
            held.first = seq;
        }
        held.changes.push(change);
        held.open_batches > 0 && held.changes.len() < HeldChanges::MOST_PER_MESSAGE
    });
    if !holds_on {
        send_held_changes();
    }
    seq
}

/// The arena changes a document thread sent that wait to go to the owner together: all of `document`, numbered from
/// `first` on.
#[derive(Default)]
struct HeldChanges {
    /// How many change batches are open on the thread.
    open_batches: u32,
    document: DocumentId,
    first: ChangeSeq,
    changes: Vec<ArenaChange>,
}

impl HeldChanges {
    /// The most changes a batch holds before it sends them: the owner takes in the first changes of a long drain
    /// while the main thread writes the rest, rather than all of them once the unit that comes after is waited for.
    const MOST_PER_MESSAGE: usize = 256;
}

/// Sends the owner what the calling document thread holds, as one message.
fn send_held_changes() {
    let held = HELD.with_borrow_mut(|held| {
        (!held.changes.is_empty()).then(|| (held.document, held.first, std::mem::take(&mut held.changes)))
    });
    if let Some((document, first, changes)) = held {
        send(ToOwner::Changes {
            document,
            first,
            changes,
        });
    }
}

/// Opens a change batch on the calling document thread: until the outermost batch closes, the arena changes the
/// thread sends wait, and go to the owner as one message as it closes, or ahead of any other message the thread sends
/// before. A drain that writes a change per row sends one message, not one per row.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_open_change_batch() {
    HELD.with_borrow_mut(|held| held.open_batches += 1);
}

/// Closes the change batch [`render_owner_open_change_batch`] opened last on the calling thread.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_close_change_batch() {
    let outermost = HELD.with_borrow_mut(|held| {
        debug_assert!(held.open_batches > 0, "a change batch closed that was never opened");
        held.open_batches = held.open_batches.saturating_sub(1);
        held.open_batches == 0
    });
    if outermost {
        send_held_changes();
    }
}

/// Whether the owner takes in change `seq` the calling document thread sent for `document` before anything it asks of
/// the document's arena next: a unit or question the thread sent since reaches the arena after it.
pub(crate) fn taken_in_before_next_arena_reach(document: DocumentId, seq: ChangeSeq) -> bool {
    TAKEN_IN_THROUGH.with_borrow(|taken_in| taken_in.get(&document).is_some_and(|through| *through >= seq))
}

/// On a document thread, as it sends the owner `message`: the changes it holds go first, and where the message reaches
/// its document's arena, the owner takes in every change the thread sent before it first.
pub(crate) fn note_sending(message: &ToOwner) {
    send_held_changes();
    if message.may_move_layout() {
        SENT_THROUGH.with_borrow_mut(|sent| sent.entry(message.document()).or_default().moving_layout.units += 1);
    }
    if message.reaches_arena() {
        let document = message.document();
        let through = sent_through(document);
        TAKEN_IN_THROUGH.with_borrow_mut(|taken_in| taken_in.insert(document, through));
    }
}

/// What the calling document thread sent the owner of `document` so far that may move its layout.
pub(crate) fn layout_mark(document: DocumentId) -> LayoutMark {
    SENT_THROUGH.with_borrow(|sent| {
        sent.get(&document)
            .map_or_else(LayoutMark::default, |sent| sent.moving_layout)
    })
}

/// The number of the last change the calling document thread sent for `document`.
pub(crate) fn sent_through(document: DocumentId) -> ChangeSeq {
    SENT_THROUGH.with_borrow(|sent| sent.get(&document).map_or_else(ChangeSeq::default, |sent| sent.through))
}

/// The number of the last change the calling document thread sent for `document` that alters the rows the owner
/// publishes: rows that reflect it answer a read the thread makes now.
pub(crate) fn sent_row_changes_through(document: DocumentId) -> ChangeSeq {
    SENT_THROUGH.with_borrow(|sent| {
        sent.get(&document)
            .map_or_else(ChangeSeq::default, |sent| sent.altering_rows_through)
    })
}

/// Recalls the rendering update of `document` the owner runs, if it runs one: the document thread takes its frame back.
pub(crate) fn recall_rendering_update(document: DocumentId) {
    if document.is_valid() {
        send(ToOwner::Recall { document });
    }
}

/// Asks the owner `query` about `document`, whose arena the calling document thread names as `arena`, and waits for
/// the answer, as of every change the thread sent before. Where the owner cannot answer it (a test holds the run it
/// would queue behind), the thread reads its arena right here, as every door of the port does.
///
/// # Safety
///
/// `arena` must be the live arena of `document`, which no stage the document thread submitted owns.
pub(crate) unsafe fn ask(document: DocumentId, arena: *mut c_void, query: Query, wait: impl OwnerWait) -> Answer {
    if !document.is_valid() {
        // The owner holds no state of an arena of no document (a unit test's): the thread that holds it answers.
        // SAFETY: Guaranteed by the caller.
        let owner = Owner::here();
        return Answer::of_state_reaching_engine(&owner, query, unsafe {
            &mut *ArenaHandle::held_by_waiting_thread(&owner, arena)
        });
    }
    crate::stage_thread::wait_for_owner(
        wait,
        |reply| ToOwner::Ask { document, query, reply },
        |owner| {
            if let Some(answer) =
                STATES.with_borrow_mut(|states| states.get_mut(&document).map(|state| state.answer(owner, query)))
            {
                return answer;
            }
            // SAFETY: Guaranteed by the caller.
            Answer::of_state_reaching_engine(owner, query, unsafe {
                &mut *ArenaHandle::held_by_waiting_thread(owner, arena)
            })
        },
    )
}

/// Asks the owner `query` about the document whose arena the calling document thread names as `arena`, once the frame
/// in flight that owns the arena, if any, has been taken back.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
pub(crate) unsafe fn ask_about(arena: *mut c_void, query: Query, wait: impl OwnerWait) -> Answer {
    assert!(!arena.is_null(), "layout node arena handle is null");
    crate::stage_thread::join_frame_in_flight(arena);
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { ArenaHandle::document_of(arena) };
    // SAFETY: As above.
    unsafe { ask(document, arena, query, wait) }
}

/// Asks the owner the engine query `query` about `document` and waits for the answer, as of every change the calling
/// thread sent before. Only the owner answers it: a run a test holds serves it between the run's units. The thread that
/// holds the document's render state (the owner) answers it right here.
pub(crate) fn ask_engine(document: DocumentId, query: Query, wait: impl OwnerWait) -> Answer {
    if let Some(answer) = STATES.with_borrow_mut(|states| {
        states
            .get_mut(&document)
            .map(|state| state.answer(&Owner::here(), query))
    }) {
        return answer;
    }
    crate::stage_thread::wait_for_owner_thread(wait, |reply| ToOwner::Ask { document, query, reply }).unwrap_or_else(
        || {
            debug_assert!(false, "the engine query of document {document:?} has no owner");
            Answer::unanswered(query)
        },
    )
}

/// Asks the owner `query` about `document` and waits for the answer, as [`ask`] does, for a document thread that
/// names no arena: where the owner cannot answer it, the question is left to the host.
pub(crate) fn ask_owner(document: DocumentId, query: Query, wait: impl OwnerWait) -> Answer {
    crate::stage_thread::wait_for_owner(
        wait,
        |reply| ToOwner::Ask { document, query, reply },
        |owner| {
            STATES
                .with_borrow_mut(|states| states.get_mut(&document).map(|state| state.answer(owner, query)))
                .unwrap_or_else(|| Answer::left_to_host(query))
        },
    )
}

/// The border box of the principal box of the element with `style_node` in `document` as its render state committed
/// it last, which is what the owner laid out and presented: read without a layout update, for tests.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_committed_border_box(document: DocumentId, style_node: u32) -> FfiGeometryReadAnswer {
    let Some(node) = StyleNodeID::from_raw(style_node).filter(|_| document.is_valid()) else {
        return FfiGeometryReadAnswer::default();
    };
    let kind = FfiGeometryReadKind::BorderBox;
    match ask_owner(document, Query::Geometry { node, kind }, unsafe {
        ScriptForcedRead::at_script_entry()
    }) {
        Answer::Geometry(answer) => answer,
        _ => FfiGeometryReadAnswer::default(),
    }
}

/// Makes the owner panic answering the calling thread, which waits for it, for a test of what a panicked owner does to
/// the process: it ends it rather than leave the thread waiting.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_panic_for_testing(document: DocumentId) {
    // SAFETY: Internals' script API calls it, once per call.
    let wait = unsafe { ScriptForcedRead::at_script_entry() };
    let _ = crate::stage_thread::wait_for_owner_thread(wait, |reply| ToOwner::PanicForTesting { document, reply });
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
pub(crate) fn ask_arena(document: DocumentId, query: ArenaQuery, wait: impl OwnerWait) -> ArenaAnswer {
    if !document.is_valid() {
        return query.left_to_host();
    }
    join_frame_of(document);
    match ask_owner(document, Query::Arena(query), wait) {
        Answer::Arena(answer) => answer,
        _ => {
            debug_assert!(false, "an arena query is answered from the arena");
            query.left_to_host()
        }
    }
}

/// Asks the owner `query` of the layout arena the calling document thread names as `arena`, once the frame in flight
/// that owns the arena, if any, has been taken back.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
pub(crate) unsafe fn ask_arena_of(arena: *mut c_void, query: ArenaQuery, wait: impl OwnerWait) -> ArenaAnswer {
    // SAFETY: Guaranteed by the caller.
    match unsafe { ask_about(arena, Query::Arena(query), wait) } {
        Answer::Arena(answer) => answer,
        _ => {
            debug_assert!(false, "an arena query is answered from the arena");
            query.left_to_host()
        }
    }
}

/// The text the generated content of the pseudo-element `generated_for` of the element `style_node` in `document`
/// resolved to when its box was built, as an `AK::Utf16String` raw representation the caller adopts.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_generated_content_accessible_text(
    document: DocumentId,
    style_node: u32,
    generated_for: u8,
) -> usize {
    let text = StyleNodeID::from_raw(style_node).and_then(|element| {
        let owner = crate::layout::counters::CounterOwner { element, generated_for };
        match ask_arena(document, ArenaQuery::GeneratedContentAccessibleText(owner), unsafe {
            ScriptForcedRead::at_script_entry()
        }) {
            ArenaAnswer::Text(text) => Some(text),
            _ => None,
        }
    });
    ak::Utf16String::from_utf16(text.as_deref().unwrap_or_default()).into_raw()
}

/// What the owner answers a style transaction with: the transaction's view, and the answer of the layout frame's job
/// that rode it, where the transaction let the owner run it.
pub(crate) struct StyleJobAnswer {
    view: crate::css::style::bridge::OwnerStyleTransactionView,
    layout: Option<crate::layout::update_layout::OwnerFrameJobAnswer>,
}

/// A forced read's first style transaction, as its one job, with the layout frame's first job where the document
/// readied it to ride the transaction.
pub(crate) struct StyleRound {
    pub(crate) transaction: crate::css::style::bridge::OwnerStyleTransaction,
    pub(crate) then_layout: Option<Box<crate::layout::update_layout::OwnerFrameJob>>,
}

/// What the owner answers a [`StyleRound`] with: the transaction's view, and the answer of the frame's first job where
/// it rode the transaction and ran. Where it did not, the frame sends the job on its own with the permit that takes its
/// place: the read's second wait, which the type shows.
pub(crate) struct StyleRoundAnswer {
    pub(crate) view: crate::css::style::bridge::OwnerStyleTransactionView,
    pub(crate) layout: Result<crate::layout::update_layout::OwnerFrameJobAnswer, FrameJobPermit>,
}

/// The one job of a forced read, which [`force_read`] sends: its first style transaction, or, where the read ran no
/// style, its layout frame's first job. Only this module calls `send`, as only it makes a [`SpentWait`].
pub(crate) trait ForcedReadJob {
    type Answer;
    fn send(self, document: DocumentId, spent: SpentWait) -> Self::Answer;
}

impl ForcedReadJob for StyleRound {
    type Answer = StyleRoundAnswer;

    fn send(self, document: DocumentId, spent: SpentWait) -> StyleRoundAnswer {
        let StyleJobAnswer { view, layout } = send_style_job(document, self.transaction, self.then_layout, spent);
        StyleRoundAnswer {
            view,
            layout: layout.ok_or_else(FrameJobPermit::after_style),
        }
    }
}

impl ForcedReadJob for Box<crate::layout::update_layout::OwnerFrameJob> {
    type Answer = crate::layout::update_layout::OwnerFrameJobAnswer;

    fn send(self, document: DocumentId, spent: SpentWait) -> Self::Answer {
        send_frame_job(document, self, spent)
    }
}

/// The one wait of a forced read of `document`: spends `read`, sends the owner `job`, and waits for its answer. Every
/// other style or layout job spends a permit that says why it is one ([`run_frame_job`], [`run_style_transaction`]).
pub(crate) fn force_read<J: ForcedReadJob>(_read: ForcedRead, document: DocumentId, job: J) -> J::Answer {
    job.send(document, SpentWait(()))
}

/// Runs `job`, a layout frame's job that is not its forced read's one job, on the owner that holds `document`'s render
/// state, spending `permit`, and waits for its answer.
pub(crate) fn run_frame_job(
    _permit: FrameJobPermit,
    document: DocumentId,
    job: Box<crate::layout::update_layout::OwnerFrameJob>,
) -> crate::layout::update_layout::OwnerFrameJobAnswer {
    send_frame_job(document, job, SpentWait(()))
}

/// Runs the style transaction `transaction` of `document` that is not a forced read's one job, which the calling
/// document thread takes, on the owner, spending `permit`, and waits for its view.
pub(crate) fn run_style_transaction(
    _permit: StyleJobPermit,
    document: DocumentId,
    transaction: crate::css::style::bridge::OwnerStyleTransaction,
) -> crate::css::style::bridge::OwnerStyleTransactionView {
    send_style_job(document, transaction, None, SpentWait(())).view
}

/// Sends the layout frame's job `job` to the owner that holds `document`'s render state, and waits for its answer.
/// Where the calling thread is the owner, or the job would queue behind a run a test holds, it runs right here, as the
/// owner.
fn send_frame_job(
    document: DocumentId,
    job: Box<crate::layout::update_layout::OwnerFrameJob>,
    spent: SpentWait,
) -> crate::layout::update_layout::OwnerFrameJobAnswer {
    let job = std::cell::Cell::new(Some(job));
    crate::stage_thread::wait_for_owner(
        SpentWait(()),
        |reply| ToOwner::Layout {
            document,
            job: job.take().expect("a job is sent once"),
            reply,
            _spent: spent,
        },
        |owner| job.take().expect("a job runs once").run_here(owner),
    )
}

/// Runs the style transaction `transaction` of `document`, which the calling document thread takes, on the owner, and
/// waits for its answers, with the layout frame's job `then_layout` right after it where the transaction lets it run
/// (see [`ToOwner::Style`]). The owner serves it between the units of whatever it runs. Where the calling thread holds
/// the document's render state, it is the owner (a unit the owner runs may take a transaction), and runs the
/// transaction as the owner does one it is sent.
fn send_style_job(
    document: DocumentId,
    transaction: crate::css::style::bridge::OwnerStyleTransaction,
    then_layout: Option<Box<crate::layout::update_layout::OwnerFrameJob>>,
    spent: SpentWait,
) -> StyleJobAnswer {
    let transaction = Box::new(transaction);
    if STATES.with_borrow(|states| states.contains_key(&document)) {
        return run_style_job_on_owner(&Owner::here(), document, transaction, then_layout);
    }
    let ran = crate::stage_thread::wait_for_owner_thread(SpentWait(()), |reply| ToOwner::Style {
        document,
        transaction,
        then_layout,
        reply,
        _spent: spent,
    });
    ran.unwrap_or_else(|| {
        debug_assert!(false, "the style transaction of document {document:?} has no owner");
        StyleJobAnswer {
            view: crate::css::style::bridge::OwnerStyleTransactionView::unanswered(),
            layout: None,
        }
    })
}

/// On the owner: runs the style transaction `transaction` of `document`, and the layout frame's job `then_layout` right
/// after it where the transaction lets it run, while the document thread waits for both.
fn run_style_job_on_owner(
    owner: &Owner,
    document: DocumentId,
    transaction: Box<crate::css::style::bridge::OwnerStyleTransaction>,
    then_layout: Option<Box<crate::layout::update_layout::OwnerFrameJob>>,
) -> StyleJobAnswer {
    let view = run_style_on_owner(owner, document, transaction);
    let layout = then_layout.filter(|_| view.lets_layout_follow()).map(|job| {
        // As for a job sent on its own, the job finds the arena inside the answer.
        job.run(owner, || {
            with_state(document, |state| state.state_for_waiting_thread(owner))
        })
    });
    StyleJobAnswer { view, layout }
}

/// On the owner: runs the style transaction `transaction` of `document` with the engine its render state links, while
/// the document thread waits for it.
fn run_style_on_owner(
    owner: &Owner,
    document: DocumentId,
    transaction: Box<crate::css::style::bridge::OwnerStyleTransaction>,
) -> crate::css::style::bridge::OwnerStyleTransactionView {
    // The state's changes, the link to its engine among them, go in before the engine is read.
    let reached = with_state(document, |state| {
        let state_handle = state.state_for_waiting_thread(owner);
        (state.arena.arena().hold_style_engine(), state_handle)
    });
    let Some((Some(engine), state)) = reached else {
        debug_assert!(
            false,
            "the owner runs the style transaction of a document with an engine"
        );
        return crate::css::style::bridge::OwnerStyleTransactionView::unanswered();
    };
    let view = {
        // The faces the transaction wants are this document's, for its layout end to request, whichever document's
        // update the owner serves the transaction beside.
        let _wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(state as u64);
        // SAFETY: The engine and the state are the document's, which only the owner reaches, and the document thread
        // waits for the transaction.
        unsafe { engine.reach_on_owner(owner, |engine| transaction.run(engine, &mut *state)) }
    };
    // The rows go out as the transaction left them, and in any case: the unit let go of those published before.
    // SAFETY: The state is the document's, which only the owner reaches, and the document thread waits.
    unsafe { &mut *state }.arena_mut().publish_rows();
    view
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
fn answer_geometry(arena: &LayoutNodeArena, node: StyleNodeID, kind: FfiGeometryReadKind) -> FfiGeometryReadAnswer {
    let has_layout_root = !arena.layout_root().is_invalid();
    let published = arena.published_rows();
    let Some(slot) = published.bound_row(node) else {
        return FfiGeometryReadAnswer::no_box();
    };
    // A table's principal box is its wrapper, which the document thread finds.
    if published
        .parent(slot)
        .and_then(|parent| published.node(parent))
        .is_some_and(|parent| parent.kind == NodeKind::TableWrapper)
    {
        return FfiGeometryReadAnswer::default();
    }
    let absolute_rects = std::cell::RefCell::default();
    let rows = crate::painting::published_frame::PaintSource::of_rows(&published, &absolute_rects);
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
    let (document, arena) = create_document(TakeIn::AsTheOwnerIdles);
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
            changes: vec![ArenaChange::BeginLayoutTrace],
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
            changes: vec![ArenaChange::BeginLayoutTrace],
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
                take_in: TakeIn::WhenWaitedFor,
            });
            assert!(SPARE.lock().unwrap().is_some());
            let seq = ChangeSeq(1);
            handle(ToOwner::Changes {
                document,
                first: seq,
                changes: vec![ArenaChange::BeginLayoutTrace],
            });
            let received = with_state(document, |state| state.changes.pending.len()).unwrap();
            assert_eq!(received, 1);
            handle(ToOwner::Destroy { document });
            STATES.with_borrow(|states| states.is_empty())
        });
        assert!(owner.join().unwrap());
    }

    #[test]
    fn the_owner_takes_in_what_a_document_thread_sent_as_it_idles() {
        let owner = std::thread::spawn(|| {
            let beside = DocumentId::mint();
            let waited_for = DocumentId::mint();
            for (document, take_in) in [(beside, TakeIn::AsTheOwnerIdles), (waited_for, TakeIn::WhenWaitedFor)] {
                handle(ToOwner::Create {
                    document,
                    arena: SpareArena(Box::new(ArenaHandle::new_for(document, std::thread::current().id()))),
                    take_in,
                });
                handle(ToOwner::Changes {
                    document,
                    first: ChangeSeq(1),
                    changes: vec![ArenaChange::BeginLayoutTrace],
                });
            }
            take_in_sent_changes();
            let pending = |document| with_state(document, |state| state.changes.pending.len()).unwrap();
            // The thread that holds the engine of the other document itself may be reaching it: its changes wait for
            // a unit it waits for.
            let pending = (pending(beside), pending(waited_for));
            handle(ToOwner::Destroy { document: beside });
            handle(ToOwner::Destroy { document: waited_for });
            pending
        });
        assert_eq!(owner.join().unwrap(), (0, 1));
    }

    // The owner of a test build panics answering the style read of a document with no engine.
    #[cfg(debug_assertions)]
    #[test]
    fn a_style_read_the_owner_panicked_answering_drops_its_reply() {
        let owner = std::thread::spawn(|| {
            let document = DocumentId::mint();
            let arena = Box::new(ArenaHandle::new_for(document, std::thread::current().id()));
            handle(ToOwner::Create {
                document,
                arena: SpareArena(arena),
                take_in: TakeIn::WhenWaitedFor,
            });
            let mut cell = crate::css::style::owner_calls::StyleQueryCell::new(
                crate::css::style::owner_calls::StyleQuery::ReadDemand(crate::css::style::bridge::RecordDemand {
                    node: 1,
                    pseudo_kind: u8::MAX,
                    exclude_inline_style: false,
                    targeted: false,
                    read_only: true,
                    parent_highlight: 0,
                }),
            );
            let query = Query::Engine(cell.for_owner());
            let (reply, answered) = crate::stage_thread::owner_reply_for_test();
            let owner = Owner::here();
            let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                handle_message(&owner, ToOwner::Ask { document, query, reply });
            }));
            assert!(panicked.is_err(), "the owner panicked answering");
            // The host finds no answer, which ends the process where it waits, rather than answer the read again with
            // the engine the owner left.
            assert!(answered().is_none());
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
                take_in: TakeIn::WhenWaitedFor,
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
            assert!(answered().is_some(), "the pass answers");
            handle(ToOwner::Destroy { document });
            STATES.with_borrow(|states| states.is_empty())
        });
        assert!(owner.join().unwrap());
    }
}
