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
//!   for one unit of another document's update at most, and the update goes on after it.
//!
//! Every message reaches the owner through one FIFO ([`ToOwner`]), so a document's changes, its rendering updates,
//! its queries and its destruction arrive in the order the main thread sent them. The owner never joins the main
//! thread: where a rendering step still needs the main thread (the host steps of a style update around its passes,
//! the end of a layout frame), the main thread runs it as its own between the units it sends the owner. The style
//! computation itself is the owner's: every style transaction the main thread waits for runs on the owner
//! ([`ToOwner::Style`]), with the engine the document's render state links, and so does the rest of each layout round
//! ([`ToOwner::Layout`]).
//!
//! During the port the main thread still reaches the arena and the style engine directly through the handles the
//! owner gives out when it creates the state ([`FfiRenderDocument`]); those doors are what the flip deletes.

use crate::css::style::bridge::InputForPass;
use crate::css::style::tree::StyleNodeID;
use crate::layout::node_data::NodeKind;
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
pub(crate) enum Change {
    /// The style inputs the main thread recorded since the last transaction: DOM tree insertions, removals and
    /// moves, element arrivals, class, ID and attribute features, element states, inline style and presentational
    /// hint declarations, and the host facts they read. The engine applies them as one batch, which is how its
    /// invalidation sees them.
    StyleInputs(InputForPass),
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
        }
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

    fn take_through(&mut self, through: ChangeSeq) -> Vec<Change> {
        debug_assert!(
            through <= self.received_through,
            "a unit applies only changes sent before it"
        );
        let mut taken = Vec::new();
        while let Some((seq, change)) = self.pending.pop_front() {
            if seq > through {
                self.pending.push_front((seq, change));
                break;
            }
            self.applied_through = seq;
            taken.push(change);
        }
        taken
    }
}

/// One document's render state, which the Rendering thread owns.
pub(crate) struct RenderState {
    /// The layout arena, with the host tables and scratch beside it. The arena links the document's style engine,
    /// its paint preparation and its animation sampling state.
    arena: Box<ArenaHandle>,
    changes: ChangeQueue,
}

impl RenderState {
    /// Answers `query` from the state as the units before it left it.
    fn answer(&mut self, query: Query) -> Answer {
        Answer::of(query, self.arena.arena_mut())
    }

    /// The document's style engine, which the arena links (null before it links one). The units the owner runs for the
    /// document reach it through [`crate::css::style::StyleEngineHandle::reach_on_owner`].
    fn style_engine(&self) -> crate::css::style::StyleEngineHandle {
        self.arena.arena().style_engine_handle()
    }

    /// The handle of the state's arena, which the units the owner runs for the document reach it through.
    fn arena_handle(&mut self) -> *mut c_void {
        std::ptr::from_mut::<ArenaHandle>(&mut self.arena).cast::<c_void>()
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
}

/// The answer to a [`Query`], of the variant the query asked for.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Answer {
    Geometry(FfiGeometryReadAnswer),
}

impl Answer {
    /// The answer that leaves the question to the main thread, as it answered it before the owner did.
    fn unanswered(query: Query) -> Self {
        match query {
            Query::Geometry { .. } => Self::Geometry(FfiGeometryReadAnswer::default()),
        }
    }

    fn of(query: Query, arena: &mut LayoutNodeArena) -> Self {
        match query {
            Query::Geometry { node, kind } => Self::Geometry(answer_geometry(arena, node, kind)),
        }
    }
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
    /// How the owner runs it. The owner reaches the pipeline only through the updates it is sent, so what reaches
    /// the owner without reaching the pipeline (the unit tests' stage threads) links without it.
    run: fn(Self),
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

    fn run(self: Box<Self>) {
        (self.run)(*self);
    }

    fn run_flight(self) {
        let (outcome, ran) = self.flight.run(self.style_engine);
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
    /// Runs a unit of a layout frame of `document` for the document thread, which waits for it and runs the steps of
    /// the frame that need it itself.
    Layout {
        document: DocumentId,
        unit: Box<crate::layout::update_layout::OwnerLayoutUnit>,
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
            ticket.run(|| update.run());
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
            Ok(unsafe { engine.reach_on_owner(|engine| transaction.run(engine)) })
        }),
        ToOwner::Layout { document, unit } => {
            // The state's borrow ends before the unit runs, which may reach another document's state. The unit finds
            // the arena inside its answer, so that a panic there answers the waiting document thread.
            (*unit).run(|| with_state(document, RenderState::arena_handle));
        }
        ToOwner::Paint { document, pass } => {
            // As for a layout unit, the pass finds the arena inside its answer.
            (*pass).run(|| with_state(document, RenderState::arena_handle));
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
            .unwrap_or_else(|| Answer::unanswered(query))
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
    let changes = with_state(document, |state| state.changes.take_through(through)).unwrap_or_default();
    for change in changes {
        change.apply(target);
    }
}

/// Sends `message` to the owner: to the Rendering thread, or handled right here where there is none.
fn send(message: ToOwner) {
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
    send(ToOwner::Create {
        document,
        arena: SpareArena(arena),
    });
    (document, address)
}

/// Drops the render state of `document` on the owner. Nothing waits for it.
pub(crate) fn destroy_document(document: DocumentId) {
    SENT_THROUGH.with_borrow_mut(|sent| sent.remove(&document));
    send(ToOwner::Destroy { document });
}

/// Sends `change` for `document`, and answers with its number: a unit that applies the changes through it applies
/// this one.
pub(crate) fn send_change(document: DocumentId, change: Change) -> ChangeSeq {
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
/// would queue behind), the thread reads its arena right here, as every door of the port does.
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
            Answer::of(query, unsafe { &mut *arena.cast::<ArenaHandle>() }.arena_mut())
        },
    );
    answer.unwrap_or_else(|_| {
        debug_assert!(false, "the render owner panicked answering {query:?}");
        Answer::unanswered(query)
    })
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
        Err(transaction) => unsafe { transaction.run(engine.enter("style transaction the owner did not run")) },
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
    // The render side no longer ticks the document's animations.
    crate::clock_frames::rust_clock_lease_revoke(arena);
    crate::layout::flush_arena_censuses();
    destroy_document(document.document);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_applies_the_changes_through_its_number_in_order() {
        let mut queue = ChangeQueue::default();
        assert!(queue.take_through(ChangeSeq(0)).is_empty());
        queue.receive(ChangeSeq(1), Change::StyleInputs(InputForPass::empty()));
        queue.receive(ChangeSeq(2), Change::StyleInputs(InputForPass::empty()));
        queue.receive(ChangeSeq(3), Change::StyleInputs(InputForPass::empty()));
        assert_eq!(queue.take_through(ChangeSeq(2)).len(), 2);
        assert_eq!(queue.applied_through, ChangeSeq(2));
        assert_eq!(queue.take_through(ChangeSeq(2)).len(), 0);
        assert_eq!(queue.take_through(ChangeSeq(3)).len(), 1);
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
