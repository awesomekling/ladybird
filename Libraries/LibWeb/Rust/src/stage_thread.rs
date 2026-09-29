/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The Rendering thread, which the render owner and the render pipeline's stages run on, and the
//! Painting thread, which runs the recordings and presentations of the frame in flight.
//!
//! There is one of each per process. A WebContent process runs every document it hosts on its one
//! main thread, so a thread per process is also a thread per event loop.
//!
//! The rendering update submits its stages instead of waiting for them: [`submit_rendering_update`]
//! hands them to the Rendering thread and returns, and the main thread goes back to its event
//! loop while the stage runs. A submitted stage has no joins; it owns the arena of the document it
//! runs for until the main thread takes it back. The frame scheduler takes it back at the top of the
//! event loop once the stage has finished, or a main-thread access to that arena takes it back first
//! (a forced join, logged once per call site). Either way the main thread blocks on the stage's reply
//! and never spins its event loop inside a stage run, and the scheduler's consume-commit runs before
//! the access goes on.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread::ThreadId;
use std::time::{Duration, Instant};

type Job = Box<dyn FnOnce() + Send>;

enum StageMessage {
    Run(Job),
    /// A typed message to the render owner, which the Rendering thread is.
    Owner(crate::render_owner::ToOwner),
}

// This crate is not instrumented by ThreadSanitizer, so TSan cannot see the ordering the stage
// thread's channels provide between the calling thread and the stage thread. Tell it explicitly,
// or every access a stage makes to the calling thread's state would be reported as a race.
#[cfg(feature = "thread-sanitizer")]
mod tsan {
    unsafe extern "C" {
        fn __tsan_acquire(address: *mut std::ffi::c_void);
        fn __tsan_release(address: *mut std::ffi::c_void);
    }

    pub(super) fn release(token: &super::StageThread) {
        // SAFETY: The TSan runtime only uses the address as a synchronization key.
        unsafe { __tsan_release(std::ptr::from_ref(token).cast_mut().cast()) }
    }

    pub(super) fn acquire(token: &super::StageThread) {
        // SAFETY: The TSan runtime only uses the address as a synchronization key.
        unsafe { __tsan_acquire(std::ptr::from_ref(token).cast_mut().cast()) }
    }
}

#[cfg(not(feature = "thread-sanitizer"))]
mod tsan {
    pub(super) fn release(_: &super::StageThread) {}
    pub(super) fn acquire(_: &super::StageThread) {}
}

struct StageThread {
    jobs: Sender<StageMessage>,
    id: ThreadId,
    /// How long a thread waiting for this thread's answer polls for it before sleeping.
    answer_spin: Duration,
}

/// The stage thread's end of its callers' messages, and how it waits for them.
struct Incoming {
    messages: Receiver<StageMessage>,
    /// Whether the stage thread answered a thread waiting for it since it last waited for a message.
    answered_a_waiting_thread: Cell<bool>,
    /// How long the stage thread polls for the next message after answering a thread that waited for it.
    next_question_spin: Duration,
}

/// What the frame scheduler on the main thread does for a submitted stage.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiFrameSchedulerHost {
    /// Tells the main thread that a submitted stage has finished. Runs on the stage thread.
    pub frame_completion_notify: unsafe extern "C" fn(),
    /// Takes in the frame whose stages a forced join has just waited for (consume-commit). Runs on
    /// the main thread, with no stage in flight.
    pub consume_commit: unsafe extern "C" fn(),
    /// Whether the main thread's garbage collector is finalizing or destroying cells, where nothing
    /// may take a frame in: its consume runs script and allocates. Runs on the main thread.
    pub tearing_down_cells: unsafe extern "C" fn() -> bool,
}

static FRAME_SCHEDULER_HOST: OnceLock<FfiFrameSchedulerHost> = OnceLock::new();

/// Whether no frame scheduler host is installed yet: the first main-thread frame scheduler installs one.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_wants_frame_scheduler_host() -> bool {
    FRAME_SCHEDULER_HOST.get().is_none()
}

/// Whether the calling thread submits the stages of its rendering updates, see [`submits`].
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_submits() -> bool {
    submits()
}

/// Installs the main thread's frame scheduler host. The first host installed stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_set_frame_scheduler_host(host: FfiFrameSchedulerHost) {
    let _ = FRAME_SCHEDULER_HOST.set(host);
}

static THREAD_SETUP: OnceLock<extern "C" fn()> = OnceLock::new();

/// Makes the stage thread run `setup` first thing once it starts. Call it before the first stage
/// runs; the first setup installed stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_set_thread_setup(setup: extern "C" fn()) {
    let _ = THREAD_SETUP.set(setup);
}

// The size Linux and macOS give a process's main thread, where the stages ran before.
const STAGE_THREAD_STACK_SIZE: usize = 8 * 1024 * 1024;

impl StageThread {
    fn spawn(name: &str) -> Self {
        let (jobs, messages) = channel::<StageMessage>();
        // A thread woken from sleep takes tens of microseconds to run again, which a handoff between a document thread
        // and the stage thread pays twice. With one core, polling only keeps the thread that would answer from running.
        let polling_helps = std::thread::available_parallelism().is_ok_and(|cores| cores.get() > 1);
        let spin = |duration| if polling_helps { duration } else { Duration::ZERO };
        let incoming = Incoming {
            messages,
            answered_a_waiting_thread: Cell::new(false),
            // A thread that just had its answer mostly asks again right away, in a script's run of style and layout
            // reads.
            next_question_spin: spin(Duration::from_micros(200)),
        };
        let thread = std::thread::Builder::new()
            .name(name.into())
            .stack_size(STAGE_THREAD_STACK_SIZE)
            .spawn(move || {
                if let Some(setup) = THREAD_SETUP.get() {
                    setup();
                }
                INCOMING.with(|slot| *slot.borrow_mut() = Some(incoming));
                while let Some(message) = next_message() {
                    match message {
                        StageMessage::Run(job) => job(),
                        StageMessage::Owner(message) => {
                            tsan::acquire(stage_thread());
                            crate::render_owner::handle(message);
                            tsan::release(stage_thread());
                        }
                    }
                }
            })
            .expect("the stage thread could not be started");
        Self {
            jobs,
            id: thread.thread().id(),
            answer_spin: spin(Duration::from_millis(1)),
        }
    }
}

thread_local! {
    // On the stage thread, the thread waiting for the stage it is running, or the thread that
    // submitted it.
    static WAITING_CALLER: Cell<Option<ThreadId>> = const { Cell::new(None) };
    // On the stage thread, where the caller's messages arrive.
    static INCOMING: RefCell<Option<Incoming>> = const { RefCell::new(None) };
    // On the stage thread, the messages that arrived while it ran something that could not take them, in the order
    // they arrived, ahead of everything still incoming.
    static DEFERRED: RefCell<VecDeque<StageMessage>> = const { RefCell::new(VecDeque::new()) };
    // On the calling thread, the stages it has submitted for a document's arena or style engine and
    // not taken back yet, in submission order, and likewise the recordings and presentations it has
    // submitted to the paint lane, which reach neither. Together they are the frame in flight.
    static SUBMITTED: RefCell<Vec<SubmittedStage>> = const { RefCell::new(Vec::new()) };
    static PAINTING: RefCell<Vec<PaintStage>> = const { RefCell::new(Vec::new()) };
    // On the calling thread, how many forced joins took a style pass back.
    static STYLE_PASS_FORCED_JOINS: Cell<u64> = const { Cell::new(0) };
    // On the calling thread, the call sites that forced a join already logged.
    static FORCED_JOIN_SITES: RefCell<std::collections::HashSet<(&'static str, usize, u32)>> =
        RefCell::new(std::collections::HashSet::new());
    // On the calling thread, how many forced joins took in a frame with a stage of each label.
    static FORCED_JOINS: RefCell<Vec<(&'static str, u64)>> = const { RefCell::new(Vec::new()) };
    // On the calling thread, the call sites a garbage collection reached the frame in flight from, each logged once.
    static SITES_REACHED_FROM_COLLECTION: RefCell<std::collections::HashSet<(&'static str, u32)>> =
        RefCell::new(std::collections::HashSet::new());
}

type StageOutcome = Result<(), Box<dyn Any + Send>>;

/// How a submitted run answers the calling thread once it has run.
struct StageReply {
    from_stage: Receiver<StageOutcome>,
    outcome: Option<StageOutcome>,
}

impl StageReply {
    fn poll(&mut self) -> bool {
        if self.outcome.is_none() {
            match self.from_stage.try_recv() {
                Ok(outcome) => self.outcome = Some(outcome),
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => std::process::abort(),
            }
        }
        self.outcome.is_some()
    }

    /// Waits for the run to finish without taking its outcome, which stays for the frame's consume.
    fn wait_until_finished(&mut self) {
        if self.outcome.is_none() {
            self.outcome = Some(self.from_stage.recv().unwrap_or_else(|_| std::process::abort()));
        }
    }

    fn wait(&mut self) -> StageOutcome {
        match self.outcome.take() {
            Some(outcome) => outcome,
            None => self.from_stage.recv().unwrap_or_else(|_| std::process::abort()),
        }
    }
}

/// A recording or presentation the calling thread has submitted to the paint lane and not taken back
/// yet. It owns the frame it records or presents and reaches nothing of its document: no arena, no
/// style engine. So no per-document question about the frame in flight (a door of the arena, a style
/// engine entrance, an arena change) can find it; only the frame's own take-back and a join of the
/// whole frame wait for it, and a record of its document joins its recording
/// ([`join_recording_in_flight_of`]).
struct PaintStage {
    // `"recording"` or [`PRESENTATION_STAGE`], which a test's hold names it by.
    label: &'static str,
    // The document the stage records or presents a frame of, as the handle the main thread knows its
    // arena by, or 0 for a presentation that records nothing. A test's hold names a stage by it.
    document: usize,
    reply: StageReply,
}

impl PaintStage {
    fn hold_labels(&self) -> &[&'static str] {
        std::slice::from_ref(&self.label)
    }

    fn poll(&mut self) -> bool {
        self.reply.poll()
    }

    /// Waits for the stage, releasing a test's hold on it, as [`SubmittedStage::wait`] does.
    fn wait(&mut self) -> &mut StageReply {
        if !self.reply.poll() {
            release_hold_on(std::slice::from_ref(&self.label));
        }
        &mut self.reply
    }
}

/// A stage the calling thread has submitted for a document's arena or style engine and not taken
/// back yet.
struct SubmittedStage {
    label: &'static str,
    // The stage whose hold on the document this one has: its own label, or for a flight, the label
    // of the furthest stage it may run (see [`submit_rendering_update`]).
    role: &'static str,
    // The labels a test's hold may name to hold this run: its own, and those of the stages of a
    // flight it may run ("flight:style").
    hold_labels: Vec<&'static str>,
    // The arena of the document the stage runs for, as the handle the main thread knows it by. The
    // stage owns it while it runs, unless it is a style pass ([`SubmittedStage::owns_arena`]). A
    // test's hold names a stage by it.
    arena: usize,
    reply: StageReply,
    // What the main thread runs once it has taken a clock tick back, before anything else reaches
    // what the tick owned. A flight's end comes as the frame's news instead.
    on_taken_back: Option<Box<dyn FnOnce()>>,
    _count: SubmittedStageCount,
}

/// How many submitted stages for a document's arena or style engine exist on any thread, taken back
/// or not, until they are dropped; paint lane stages are not among them. The per-document checks of
/// the frame in flight read it before the calling thread's own list: while it is zero no thread has
/// such a stage, which is the common case even beside a recording, and they need not reach any
/// thread-local state.
static SUBMITTED_STAGES: AtomicUsize = AtomicUsize::new(0);

/// A submitted stage's place in [`SUBMITTED_STAGES`]. A thread only asks about the stages it
/// submitted itself, which it counted before it asks, so the count needs no ordering.
struct SubmittedStageCount;

impl SubmittedStageCount {
    fn new() -> Self {
        SUBMITTED_STAGES.fetch_add(1, Ordering::Relaxed);
        Self
    }
}

impl Drop for SubmittedStageCount {
    fn drop(&mut self) {
        SUBMITTED_STAGES.fetch_sub(1, Ordering::Relaxed);
    }
}

/// Whether no thread has a stage in flight for a document's arena or style engine: every check of
/// [`SUBMITTED`] on the calling thread finds none.
#[inline]
pub(crate) fn no_stage_is_submitted() -> bool {
    SUBMITTED_STAGES.load(Ordering::Relaxed) == 0
}

impl SubmittedStage {
    /// Whether the stage owns its arena while it runs. A style pass does not: it reaches only its
    /// style engine, and the main thread goes on writing the arena beside it.
    fn owns_arena(&self) -> bool {
        self.role != "style"
    }

    fn poll(&mut self) -> bool {
        self.reply.poll()
    }

    /// A held stage would never finish while the main thread waits for it, so waiting releases a
    /// hold on it. A run that finished without reaching the hold, as a flight that ended before the
    /// stage the hold names, leaves the hold armed for the run that reaches it.
    fn release_hold_unless_finished(&mut self) {
        if !self.poll() {
            release_hold_on(&self.hold_labels);
        }
    }

    /// Waits for the stage to finish without taking its outcome, which stays for the frame's
    /// consume.
    fn wait_until_finished(&mut self) {
        self.release_hold_unless_finished();
        self.reply.wait_until_finished();
    }

    fn wait(&mut self) -> StageOutcome {
        self.release_hold_unless_finished();
        self.reply.wait()
    }
}

/// Whether the rendering update submits its stages rather than waiting for them: on the main thread, once its frame
/// scheduler is installed. The Rendering thread runs what it reaches in place.
pub(crate) fn submits() -> bool {
    FRAME_SCHEDULER_HOST.get().is_some() && !on_owner_thread()
}

/// Submits the display list recording `stage` of the document whose arena is `arena` to the frame
/// in flight, on the paint lane. The recording reaches no arena: it owns the frame it records and the
/// recorder state it records with, and answers on a ticket. So no door, read, style update or arena
/// change of the document waits for it ([`PaintStage`]). `arena` names the document a test's hold
/// names the recording by.
pub(crate) fn submit_recording(arena: *mut c_void, stage: impl FnOnce() + Send + 'static) {
    submit_paint_stage("recording", arena as usize, stage);
}

/// The label of a flight: one stage run that runs the stages of a rendering update one after
/// another (see `crate::flight`).
pub(crate) const FLIGHT_STAGE: &str = "flight";

/// Submits the rendering update `update` of the document whose arena is `arena`, which the render owner runs as the
/// frame's flight, as [`submit_to_owner`] does: the frame holds the document as a submitted stage `reach`
/// would, which holds it as every stage before it does. A test's hold on one of `stage_holds`, the stages the flight
/// may run, holds it.
///
/// # Safety
///
/// As for [`submit_to_owner`].
pub(crate) unsafe fn submit_rendering_update(
    reach: &'static str,
    stage_holds: &[&'static str],
    arena: *mut c_void,
    document: crate::render_owner::DocumentId,
    update: crate::render_owner::RenderingUpdate,
) {
    let hold_labels = std::iter::once(FLIGHT_STAGE)
        .chain(stage_holds.iter().copied())
        .collect();
    // SAFETY: Guaranteed by the caller.
    unsafe {
        submit_to_owner(
            FLIGHT_STAGE,
            reach,
            hold_labels,
            arena,
            |ticket| crate::render_owner::ToOwner::RenderingUpdate {
                document,
                update: Box::new(update),
                ticket,
            },
            None,
        );
    }
}

/// Submits a clock tick of the document whose arena is `arena`, which the render owner runs as the message `tick`
/// makes of its run, as [`submit_to_owner`] does: the frame holds the arena as the stage `clock`.
///
/// # Safety
///
/// As for [`submit_to_owner`].
pub(crate) unsafe fn submit_clock_tick(
    arena: *mut c_void,
    tick: impl FnOnce(SubmittedRunTicket) -> crate::render_owner::ToOwner,
    on_taken_back: impl FnOnce() + 'static,
) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        submit_to_owner(
            "clock",
            "clock",
            vec!["clock"],
            arena,
            tick,
            Some(Box::new(on_taken_back)),
        );
    };
}

/// Sends the render owner the message `message` makes of a submitted run labelled `label`, and returns at once. The
/// main thread runs `on_taken_back`, if any, once it has taken the run back: at the top of the event loop, or in the
/// forced join that takes it back first, ahead of the frame's news and the frame scheduler's consume-commit. The run owns
/// the arena `arena` until the main thread takes the frame back: the frame scheduler does at the top of its event loop
/// once the run has finished, and a main-thread access to the arena does before it goes on ([`join_frame_in_flight`]).
/// A style pass reaches only its document's style engine, which it takes along (`crate::css::style::engine_home`).
///
/// # Safety
///
/// Until the frame is taken back, nothing but the run may reach what it holds: every main-thread path to the arena has
/// to go through [`join_frame_in_flight`] first.
unsafe fn submit_to_owner(
    label: &'static str,
    role: &'static str,
    hold_labels: Vec<&'static str>,
    arena: *mut c_void,
    message: impl FnOnce(SubmittedRunTicket) -> crate::render_owner::ToOwner,
    on_taken_back: Option<Box<dyn FnOnce()>>,
) {
    let thread = stage_thread();
    let (ticket, reply) = SubmittedRunTicket::new(thread, next_submitted_run(label, arena));
    let message = message(ticket);
    crate::render_owner::note_sending(&message);
    tsan::release(thread);
    if thread.jobs.send(StageMessage::Owner(message)).is_err() {
        // The stage thread only goes away if the process is going away.
        std::process::abort();
    }
    note_submitted(label, role, hold_labels, arena, reply, on_taken_back);
}

fn next_submitted_run(label: &'static str, arena: *mut c_void) -> SubmittedRun {
    debug_assert!(submits(), "the stage {label} is not submitted");
    SubmittedRun {
        label,
        arena: arena as usize,
        document: arena as usize,
        number: NEXT_SUBMITTED_RUN.fetch_add(1, Ordering::Relaxed),
    }
}

fn note_submitted(
    label: &'static str,
    role: &'static str,
    hold_labels: Vec<&'static str>,
    arena: *mut c_void,
    reply: StageReply,
    on_taken_back: Option<Box<dyn FnOnce()>>,
) {
    SUBMITTED.with(|submitted| {
        submitted.borrow_mut().push(SubmittedStage {
            label,
            role,
            hold_labels,
            arena: arena as usize,
            reply,
            on_taken_back,
            _count: SubmittedStageCount::new(),
        });
    });
}

/// Submits `stage`, a recording or presentation labelled `label` of the document whose arena is
/// `document` (0 for none), to the frame in flight, on the paint lane: the one thread that runs every
/// recording and presentation of the frame in flight, in submission order, so frames reach their
/// compositor contexts in the order the rendering update painted them.
fn submit_paint_stage(label: &'static str, document: usize, stage: impl FnOnce() + Send + 'static) {
    let thread = paint_lane();
    debug_assert!(submits(), "the stage {label} is not submitted");
    let run = SubmittedRun {
        label,
        arena: 0,
        document,
        number: NEXT_SUBMITTED_RUN.fetch_add(1, Ordering::Relaxed),
    };
    let reply = send_submitted_run(thread, run, stage);
    PAINTING.with_borrow_mut(|painting| painting.push(PaintStage { label, document, reply }));
}

/// Hands `thread` the submitted run `run` of `stage`, which answers the calling thread on the reply.
fn send_submitted_run(
    thread: &'static StageThread,
    run: SubmittedRun,
    stage: impl FnOnce() + Send + 'static,
) -> StageReply {
    let (ticket, reply) = SubmittedRunTicket::new(thread, run);
    let job: Job = Box::new(move || ticket.run(stage));
    tsan::release(thread);
    if thread.jobs.send(StageMessage::Run(job)).is_err() {
        // The stage thread only goes away if the process is going away.
        std::process::abort();
    }
    reply
}

/// A run the main thread submitted, as the thread it is sent to receives it: which run it is, and how it answers the
/// main thread once it has run.
pub(crate) struct SubmittedRunTicket {
    thread: &'static StageThread,
    run: SubmittedRun,
    caller: ThreadId,
    to_caller: Sender<StageOutcome>,
}

impl SubmittedRunTicket {
    fn new(thread: &'static StageThread, run: SubmittedRun) -> (Self, StageReply) {
        let (to_caller, from_stage) = channel::<StageOutcome>();
        let ticket = Self {
            thread,
            run,
            caller: std::thread::current().id(),
            to_caller,
        };
        (
            ticket,
            StageReply {
                from_stage,
                outcome: None,
            },
        )
    }

    /// Runs `stage` as the submitted run, on the thread the run was sent to, and answers the main thread.
    pub(crate) fn run(self, stage: impl FnOnce()) {
        let Self {
            thread,
            run,
            caller,
            to_caller,
        } = self;
        RUNNING_SUBMITTED_RUN.with(|running| running.set(Some(run)));
        hold_here(FfiStageHoldPoint::BeforeRun);
        tsan::acquire(thread);
        let waiting_caller = WAITING_CALLER.with(|waiting| waiting.replace(Some(caller)));
        // The faces the stage wants are its document's, for that document's layout end to request.
        let wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(run.arena as u64);
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(stage));
        drop(wanted_face_owner);
        WAITING_CALLER.with(|waiting| waiting.set(waiting_caller));
        tsan::release(thread);
        hold_here(FfiStageHoldPoint::BeforeCompletion);
        RUNNING_SUBMITTED_RUN.with(|running| running.set(None));
        RUNNING_FLIGHT_STAGE.with(|running| running.set(None));
        RECALLED_WHILE_HELD.with(|recalled| recalled.set(false));
        // The caller keeps the receiver until it has taken this reply.
        let _ = to_caller.send(outcome);
        frame_completion_notify();
    }
}

/// Runs `work` on the Rendering thread as a stage the main thread `caller` submitted for the arena
/// `arena` would run, from a message that thread does not wait for: for that thread.
pub(crate) fn run_detached_for(caller: ThreadId, arena: usize, work: impl FnOnce()) {
    let waiting_caller = WAITING_CALLER.with(|waiting| waiting.replace(Some(caller)));
    let wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(arena as u64);
    work();
    drop(wanted_face_owner);
    WAITING_CALLER.with(|waiting| waiting.set(waiting_caller));
}

/// Whether the stage thread is inside a stage the main thread submitted or waits for: a detached
/// job the stage thread runs while such a stage waits for a join runs nested inside it.
pub(crate) fn running_inside_stage() -> bool {
    RUNNING_SUBMITTED_RUN.with(Cell::get).is_some() || WAITING_CALLER.with(Cell::get).is_some()
}

/// The label of the stage that presents a navigable's frame at the end of the frame in flight.
const PRESENTATION_STAGE: &str = "present";

/// Submits `present(context)` to the frame in flight as a presentation stage on the paint lane,
/// which runs once the recordings and presentations submitted before it have (a navigable's
/// recording among them). It publishes that recording from its ticket and reaches no arena.
/// `document` is the arena of the document whose frame it presents, which it may not reach, by
/// which a test's hold names it; null for a frame that records nothing.
///
/// # Safety
///
/// `present` must be safe to call with `context` on the stage thread, and `context` must stay valid
/// until the main thread has taken the frame back. Until then nothing but the stage may reach what
/// `context` lends it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_stage_thread_submit_presentation(
    document: *const c_void,
    present: unsafe extern "C" fn(*mut c_void),
    context: *mut c_void,
) {
    // SAFETY: Guaranteed by the caller.
    let context = unsafe { FrameOwns::new(context) };
    submit_paint_stage(PRESENTATION_STAGE, document as usize, move || {
        let context = context.into_inner();
        // SAFETY: Guaranteed by the caller.
        unsafe { present(context) };
    });
}

/// Where in a submitted run of a stage a test's hold makes the stage thread wait.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FfiStageHoldPoint {
    /// Before the stage runs.
    BeforeRun,
    /// In the recording, once it holds the arena's paint state and scratch, before it records.
    MidRecording,
    /// Once the stage has run, before the main thread hears that it has finished.
    BeforeCompletion,
}

/// A submitted run of a stage, as the stage thread knows it while it runs it.
#[derive(Clone, Copy)]
struct SubmittedRun {
    label: &'static str,
    arena: usize,
    document: usize,
    // In submission order, from 1.
    number: u64,
}

// The number of the next submitted run.
static NEXT_SUBMITTED_RUN: AtomicU64 = AtomicU64::new(1);

thread_local! {
    // On the stage thread, the submitted run it is running.
    static RUNNING_SUBMITTED_RUN: Cell<Option<SubmittedRun>> = const { Cell::new(None) };
    // On the stage thread, the stage of the flight it is running, as a hold names it ("flight:record").
    static RUNNING_FLIGHT_STAGE: Cell<Option<&'static str>> = const { Cell::new(None) };
    // On the owner, whether the main thread recalled the rendering update it runs while a test held the update's run.
    static RECALLED_WHILE_HELD: Cell<bool> = const { Cell::new(false) };
}

/// Which run a test's hold is armed for.
#[derive(Clone)]
struct ArmedHold {
    label: String,
    point: FfiStageHoldPoint,
    // The arena of the run to hold, or 0 for any.
    arena: usize,
}

/// A test's hold on the next submitted run of a stage: the stage thread waits at a point of that
/// run until the hold is released, so that the main thread runs its tasks beside the frame in
/// flight at a point the test chooses.
#[derive(Default)]
struct StageHold {
    armed: Option<ArmedHold>,
    // Runs submitted before this one are not held: they were submitted before the hold was armed.
    first_holdable_run: u64,
    // The hold the stage thread is holding a run for.
    holding: Option<ArmedHold>,
    // The thread that holds it: the stage thread or the paint lane.
    holding_thread: Option<ThreadId>,
}

/// The hold, what announces its changes, and whether a test ever armed one: until one does, no run is held, and the
/// checks every change the main thread sends and every hold point of a run make go without the lock.
fn stage_hold() -> &'static (Mutex<StageHold>, Condvar, AtomicBool) {
    static STAGE_HOLD: OnceLock<(Mutex<StageHold>, Condvar, AtomicBool)> = OnceLock::new();
    STAGE_HOLD.get_or_init(Default::default)
}

fn lock_stage_hold() -> (std::sync::MutexGuard<'static, StageHold>, &'static Condvar) {
    let (hold, changed, _) = stage_hold();
    (hold.lock().expect("the stage hold is never poisoned"), changed)
}

/// Whether no test ever armed a hold, so that none can hold a run.
fn no_hold_was_ever_armed() -> bool {
    !stage_hold().2.load(Ordering::Acquire)
}

/// Makes the stage thread wait at `point` of the next submitted run of the stage `label` names
/// (for the arena `arena` only, unless it is null), until [`rust_stage_thread_release_held_stage`]
/// or a main-thread wait for the stage releases it. A hold on a stage of a flight is named after
/// the flight, as "flight:style", and a hold may name the stages it holds the first run of, as
/// "recording|flight:record".
///
/// # Safety
///
/// `label` must point to `label_length` bytes of UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_stage_thread_hold_next_submitted_stage(
    label: *const u8,
    label_length: usize,
    point: FfiStageHoldPoint,
    arena: *mut c_void,
) {
    // SAFETY: Guaranteed by the caller.
    let label = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(label, label_length)) };
    stage_hold().2.store(true, Ordering::Release);
    let (mut hold, _) = lock_stage_hold();
    hold.armed = Some(ArmedHold {
        label: label.to_owned(),
        point,
        arena: arena as usize,
    });
    hold.first_holdable_run = NEXT_SUBMITTED_RUN.load(Ordering::Relaxed);
}

/// Releases a held stage, or disarms a hold no stage has reached yet.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_release_held_stage() {
    let (mut hold, changed) = lock_stage_hold();
    hold.armed = None;
    hold.holding = None;
    changed.notify_all();
}

/// Test only: waits up to `timeout_ms` for the stage thread to hold a run. Returns where it holds
/// it, if it does. Returns false at once if the calling thread has submitted no run the hold is
/// armed for, since it submits none while it waits.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_wait_for_held_stage(timeout_ms: u32, held_at: &mut FfiStageHoldPoint) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.into());
    let (mut hold, changed) = lock_stage_hold();
    loop {
        if let Some(holding) = &hold.holding {
            *held_at = holding.point;
            return true;
        }
        let now = std::time::Instant::now();
        let Some(armed) = &hold.armed else {
            return false;
        };
        // A run that finished without reaching the hold, as a flight that ended before the stage it
        // names, holds nothing.
        if !has_unfinished_run_armed(armed) || now >= deadline {
            return false;
        }
        // A run that finishes without reaching the hold tells nobody: look again shortly.
        hold = changed
            .wait_timeout(hold, (deadline - now).min(std::time::Duration::from_millis(1)))
            .expect("the stage hold is never poisoned")
            .0;
    }
}

/// Test only: waits for every thread that runs submitted stages, but the one holding a run for a
/// test's hold, to run what it was handed before. A stage the calling thread submitted to one of
/// them has run by then, so a test can tell a stage queued behind the held run from one that runs
/// beside it without waiting for a while. None of those threads may be waiting for the calling
/// thread.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_wait_for_threads_beside_held_run() {
    let holding_thread = {
        let (hold, _) = lock_stage_hold();
        hold.holding.as_ref().and(hold.holding_thread)
    };
    let threads = [Some(stage_thread()), PAINT_LANE.get()];
    for thread in threads.into_iter().flatten() {
        if Some(thread.id) == holding_thread || std::thread::current().id() == thread.id {
            continue;
        }
        let (ran, has_run) = channel::<()>();
        let job: Job = Box::new(move || {
            let _ = ran.send(());
        });
        if thread.jobs.send(StageMessage::Run(job)).is_ok() {
            let _ = has_run.recv();
        }
    }
}

/// Whether a hold is armed for a stage the frame in flight has not submitted yet, while other stages
/// of it are in flight: the style pass a rendering update submits before its layout pass, which it
/// submits once the main thread has taken the style pass back between tasks, or a flight that ended
/// before the stage the hold names, which the rendering update runs once it has taken the flight back.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_armed_hold_awaits_submission() -> bool {
    let (hold, _) = lock_stage_hold();
    if hold.holding.is_some() {
        return false;
    }
    let Some(armed) = &hold.armed else {
        return false;
    };
    let submitted_any = SUBMITTED.with_borrow(|submitted| !submitted.is_empty())
        || PAINTING.with_borrow(|painting| !painting.is_empty());
    submitted_any && !has_unfinished_run_armed(armed)
}

/// Releases the run the stage thread is holding, and disarms a hold for `label` that the stage
/// thread has not reached yet. A hold for another stage stays armed.
fn release_hold_on(hold_labels: &[&'static str]) {
    let (mut hold, changed) = lock_stage_hold();
    if hold
        .armed
        .as_ref()
        .is_some_and(|armed| hold_names_stage(&armed.label, hold_labels))
    {
        hold.armed = None;
    }
    if hold.holding.take().is_some() {
        changed.notify_all();
    }
}

/// Called where the main thread waits for a recording in flight to answer on its ticket, which is not
/// a join: a test's hold on the recording or its presentation would keep it from answering.
pub(crate) fn release_holds_on_recording() {
    release_hold_on(&["recording", PRESENTATION_STAGE]);
}

/// Called before the main thread queues a stage behind the submitted ones. A held run stays held
/// until its test releases it, whatever the main thread queues behind it, so this waits until an
/// armed hold holds its run, or until no submitted run it could hold is left. Returns whether the
/// stage thread holds a run, which the queued stage would wait behind.
fn stage_thread_holds_run_for_queued_stage() -> bool {
    if no_hold_was_ever_armed() {
        return false;
    }
    let (mut hold, changed) = lock_stage_hold();
    loop {
        if hold.holding.is_some() {
            return true;
        }
        let Some(armed) = &hold.armed else {
            return false;
        };
        if !has_unfinished_run_armed(armed) {
            return false;
        }
        // The stage thread gets to the run's hold point, or finishes the run, without this thread.
        hold = changed
            .wait_timeout(hold, std::time::Duration::from_millis(1))
            .expect("the stage hold is never poisoned")
            .0;
    }
}

/// Whether the calling thread has submitted a run the hold `armed` names that has not finished yet.
fn has_unfinished_run_armed(armed: &ArmedHold) -> bool {
    let names = |hold_labels: &[&'static str], document: usize| {
        hold_names_stage(&armed.label, hold_labels) && (armed.arena == 0 || armed.arena == document)
    };
    SUBMITTED.with_borrow_mut(|submitted| {
        submitted
            .iter_mut()
            .any(|stage| names(&stage.hold_labels, stage.arena) && !stage.poll())
    }) || PAINTING.with_borrow_mut(|painting| {
        painting
            .iter_mut()
            .any(|stage| names(stage.hold_labels(), stage.document) && !stage.poll())
    })
}

/// Whether a hold armed for `armed` holds a run of a submitted stage a hold may name by one of
/// `hold_labels`. A hold names the stages it holds the first run of, as "recording|flight:record".
fn hold_names_stage(armed: &str, hold_labels: &[&'static str]) -> bool {
    armed.split('|').any(|armed| hold_labels.contains(&armed))
}

/// On the stage thread, at `point` of a submitted run: waits while a hold is armed for it.
/// Anywhere else, does nothing.
pub(crate) fn hold_here(point: FfiStageHoldPoint) {
    hold_at(point, None);
}

/// On the stage thread, before a flight runs its stage `stage` (a label such as "flight:style"):
/// waits while a hold is armed for that stage at [`FfiStageHoldPoint::BeforeRun`]. A hold in the
/// middle of the recording names the stage too, until the flight runs the next one.
pub(crate) fn hold_before_flight_stage(stage: &'static str) {
    RUNNING_FLIGHT_STAGE.with(|running| running.set(Some(stage)));
    hold_at(FfiStageHoldPoint::BeforeRun, Some(stage));
}

/// On the stage thread, once a flight has run up to its stage `stage`: waits while a hold is armed
/// for that stage at [`FfiStageHoldPoint::BeforeCompletion`].
pub(crate) fn hold_before_flight_completion(stage: &'static str) {
    hold_at(FfiStageHoldPoint::BeforeCompletion, Some(stage));
}

fn hold_at(point: FfiStageHoldPoint, flight_stage: Option<&'static str>) {
    let Some(run) = RUNNING_SUBMITTED_RUN.with(Cell::get) else {
        return;
    };
    if no_hold_was_ever_armed() {
        return;
    }
    let (mut hold, changed) = lock_stage_hold();
    let holds_run = hold.armed.as_ref().is_some_and(|armed| {
        let held_label = flight_stage
            .or_else(|| {
                (point == FfiStageHoldPoint::MidRecording)
                    .then(|| RUNNING_FLIGHT_STAGE.with(Cell::get))
                    .flatten()
            })
            .unwrap_or(run.label);
        armed.label.split('|').any(|stage| stage == held_label)
            && armed.point == point
            && (armed.arena == 0 || armed.arena == run.document)
    });
    if !holds_run || run.number < hold.first_holdable_run {
        return;
    }
    hold.holding = hold.armed.take();
    hold.holding_thread = Some(std::thread::current().id());
    changed.notify_all();
    // The owner holding a run goes on serving what may go between the run's units, as it does between any two: a
    // style transaction or a question the main thread waits for meanwhile is the owner's to run, not the main thread's.
    let serves = on_owner_thread();
    if !serves {
        while hold.holding.is_some() {
            hold = changed.wait(hold).expect("the stage hold is never poisoned");
        }
        return;
    }
    let document = if run.arena == 0 {
        crate::render_owner::DocumentId::default()
    } else {
        // SAFETY: The run's arena is live while its run is.
        unsafe { crate::layout::ArenaHandle::document_of(run.arena as *const c_void) }
    };
    while hold.holding.is_some() {
        drop(hold);
        if serve_messages_between_units(document) {
            RECALLED_WHILE_HELD.with(|recalled| recalled.set(true));
        }
        hold = lock_stage_hold().0;
        if hold.holding.is_some() {
            hold = changed
                .wait_timeout(hold, std::time::Duration::from_millis(1))
                .expect("the stage hold is never poisoned")
                .0;
        }
    }
}

/// The frame scheduler's completion notification. The scheduler runs its consume from the top of
/// the event loop, which this has to reach even if nothing else happens on the main thread.
fn frame_completion_notify() {
    if let Some(host) = FRAME_SCHEDULER_HOST.get() {
        // SAFETY: The notification may be sent from any thread.
        unsafe { (host.frame_completion_notify)() }
    }
}

/// Whether the calling thread has submitted stages it has not taken back yet.
pub(crate) fn has_frame_in_flight() -> bool {
    SUBMITTED.with_borrow(|submitted| !submitted.is_empty()) || PAINTING.with_borrow(|painting| !painting.is_empty())
}

/// Whether the frame in flight owns the arena `arena`.
pub(crate) fn frame_in_flight_owns(arena: *mut c_void) -> bool {
    !no_stage_is_submitted()
        && SUBMITTED.with_borrow(|submitted| {
            submitted
                .iter()
                .any(|stage| stage.owns_arena() && stage.arena == arena as usize)
        })
}

/// Whether the frame in flight has a stage for the document whose arena is `arena` that owns the
/// arena or reaches its style engine.
pub(crate) fn document_frame_in_flight(arena: *mut c_void) -> bool {
    !no_stage_is_submitted()
        && SUBMITTED.with_borrow(|submitted| submitted.iter().any(|stage| stage.arena == arena as usize))
}

/// Whether every stage of the frame in flight has finished. Does not wait.
pub(crate) fn frame_in_flight_has_finished() -> bool {
    SUBMITTED.with_borrow_mut(|submitted| submitted.iter_mut().all(SubmittedStage::poll))
        && PAINTING.with_borrow_mut(|painting| painting.iter_mut().all(PaintStage::poll))
}

/// Waits for every stage of the frame in flight and takes the frame back, adopting its news at `at`. Returns whether
/// there was one. A panic in one of its stages continues here. The frame's effects are the caller's to apply.
pub(crate) fn take_frame_in_flight(at: &crate::frame_news::LegacyJoin) -> bool {
    take_back_submitted(at)
}

/// Takes the frame in flight back at the top of the event loop, if every stage of it has finished, adopting its news.
/// Returns whether it took one. Never waits.
pub(crate) fn take_finished_frame(at: &crate::frame_news::TaskBoundary) -> bool {
    frame_in_flight_has_finished() && take_back_submitted(at)
}

fn take_back_submitted(at: &impl crate::frame_news::AdoptionPoint) -> bool {
    let stages = SUBMITTED.with_borrow_mut(std::mem::take);
    let paint_stages = PAINTING.with_borrow_mut(std::mem::take);
    if stages.is_empty() && paint_stages.is_empty() {
        return false;
    }
    let mut panic = None;
    let mut on_taken_back = Vec::new();
    for mut stage in stages {
        if let Err(payload) = stage.wait() {
            panic.get_or_insert(payload);
        }
        on_taken_back.extend(stage.on_taken_back);
    }
    for mut stage in paint_stages {
        if let Err(payload) = stage.wait().wait() {
            panic.get_or_insert(payload);
        }
    }
    acquire_stage_threads();
    if let Some(payload) = panic {
        std::panic::resume_unwind(payload);
    }
    for take_back in on_taken_back {
        take_back();
    }
    crate::frame_news::adopt_news(at);
    true
}

/// Called where main-thread code reaches render-owned state: if the frame in flight owns the arena
/// `arena` (or has any stage, for a null `arena`), waits for the frame, takes it back and runs the
/// frame scheduler's consume-commit, so the access finds the document as the frame left it. Logs
/// each call site that forced a join once.
#[track_caller]
pub(crate) fn join_frame_in_flight(arena: *mut c_void) {
    let location = std::panic::Location::caller();
    join_frame_in_flight_at(arena, location.file(), location.line(), location.column());
}

/// Like [`join_frame_in_flight`], for a call site outside Rust that names itself (column 0 when it
/// has none).
pub(crate) fn join_frame_in_flight_at(arena: *mut c_void, file: &'static str, line: u32, column: u32) {
    if !arena.is_null() {
        join_frame_in_flight_for_stage(
            |stage| stage.owns_arena() && stage.arena == arena as usize,
            file,
            line,
            column,
        );
        return;
    }
    let first_stage = SUBMITTED
        .with_borrow(|submitted| submitted.first().map(|stage| (stage.label, stage.role)))
        .or_else(|| PAINTING.with_borrow(|painting| painting.first().map(|stage| (stage.label, stage.label))));
    if let Some((label, role)) = first_stage {
        join_reached_stage(label, role, file, line, column);
    }
}

/// Like [`join_frame_in_flight_at`], for a main-thread operation on the document whose arena is
/// `arena` rather than an access to the arena: it also joins a stage that runs for the document
/// without owning its arena, such as a style pass.
pub(crate) fn join_document_frame_in_flight_at(arena: *mut c_void, file: &'static str, line: u32, column: u32) {
    join_frame_in_flight_for_stage(|stage| stage.arena == arena as usize, file, line, column);
}

/// Like [`join_frame_in_flight`], for the recording of the document whose arena is `arena` in flight
/// on the paint lane: the frame's consume publishes it, as the frame's presentation may not have.
#[track_caller]
pub(crate) fn join_recording_in_flight_of(arena: *mut c_void) {
    let recording_in_flight = PAINTING.with_borrow(|painting| {
        painting
            .iter()
            .any(|stage| stage.label == "recording" && stage.document == arena as usize)
    });
    if recording_in_flight {
        let location = std::panic::Location::caller();
        join_reached_stage(
            "recording",
            "recording",
            location.file(),
            location.line(),
            location.column(),
        );
    }
}

/// If a stage of the frame in flight for a document's arena or style engine is `reached`, waits for
/// the frame, takes it back and runs the frame scheduler's consume-commit, as
/// [`join_frame_in_flight`] describes.
fn join_frame_in_flight_for_stage(
    reached: impl Fn(&SubmittedStage) -> bool,
    file: &'static str,
    line: u32,
    column: u32,
) {
    if no_stage_is_submitted() {
        return;
    }
    let reached_stage = SUBMITTED.with_borrow(|submitted| {
        submitted
            .iter()
            .find(|stage| reached(stage))
            .map(|stage| (stage.label, stage.role))
    });
    if let Some((label, role)) = reached_stage {
        join_reached_stage(label, role, file, line, column);
    }
}

/// Joins the frame in flight for a reached stage labelled `label` with the role `role`: waits for
/// the frame, takes it back and runs the frame scheduler's consume-commit.
fn join_reached_stage(label: &'static str, role: &'static str, file: &'static str, line: u32, column: u32) {
    if role == "style" {
        STYLE_PASS_FORCED_JOINS.with(|joins| joins.set(joins.get() + 1));
    }
    let first_time = FORCED_JOIN_SITES.with(|sites| sites.borrow_mut().insert((file, line as usize, column)));
    if first_time {
        // A style engine entrance names itself, and a C++ call site has no column.
        if line == 0 {
            eprintln!("STAGE OVERLAP: forced join of {label} at style engine entrance {file}");
        } else if column == 0 {
            eprintln!("STAGE OVERLAP: forced join of {label} at {file}:{line}");
        } else {
            eprintln!("STAGE OVERLAP: forced join of {label} at {file}:{line}:{column}");
        }
    }
    count_forced_join();
    let host = FRAME_SCHEDULER_HOST.get().expect("a submitted frame has a scheduler");
    // SAFETY: Called on the main thread.
    if unsafe { (host.tearing_down_cells)() } {
        refuse_join_while_tearing_down_cells(file, line);
        return;
    }
    // The join takes the whole frame back, so each flight it waits for stops at the end of the stage it runs, or
    // before its first if it has not begun.
    if label == FLIGHT_STAGE {
        let flight_arenas = SUBMITTED.with_borrow(|submitted| {
            submitted
                .iter()
                .filter(|stage| stage.label == FLIGHT_STAGE)
                .map(|stage| stage.arena)
                .collect::<Vec<_>>()
        });
        for arena in flight_arenas {
            // SAFETY: A submitted stage's arena outlives the frame in flight.
            let document = unsafe { crate::layout::ArenaHandle::document_of(arena as *const c_void) };
            crate::render_owner::recall_rendering_update(document);
        }
    }
    take_frame_in_flight(&crate::frame_news::LegacyJoin::waiting_for_the_frame());
    // SAFETY: Called on the main thread, with the frame taken back.
    unsafe { (host.consume_commit)() }
}

/// A finalizer or a cell's destructor reached the frame in flight. Taking the frame in there would
/// run its consume, which runs script and allocates in the middle of the collection, so no such
/// code may need the frame: it defers its work to the next frame, or drops what a collected cell no
/// longer needs. Should one reach it anyway, it only waits for the frame's stages to finish, so it
/// does not race them, and leaves the frame for its consume at the top of the event loop.
fn refuse_join_while_tearing_down_cells(file: &'static str, line: u32) {
    debug_assert!(
        false,
        "a garbage collection reached the frame in flight at {file}:{line}, and may not take it in"
    );
    if SITES_REACHED_FROM_COLLECTION.with_borrow_mut(|sites| sites.insert((file, line))) {
        eprintln!(
            "STAGE OVERLAP: a garbage collection reached the frame in flight at {file}:{line}; only waiting for it"
        );
    }
    wait_for_submitted_stages();
}

/// Waits for every stage of the calling thread's frame in flight to finish, and leaves the frame in
/// flight for its consume.
fn wait_for_submitted_stages() {
    let waited_for_stages = SUBMITTED.with_borrow_mut(|submitted| {
        submitted.iter_mut().for_each(SubmittedStage::wait_until_finished);
        !submitted.is_empty()
    });
    let waited_for_paint_stages = PAINTING.with_borrow_mut(|painting| {
        painting.iter_mut().for_each(|stage| stage.wait().wait_until_finished());
        !painting.is_empty()
    });
    if waited_for_stages || waited_for_paint_stages {
        acquire_stage_threads();
    }
}

/// Test only: how many forced joins on the calling thread took a style pass back.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_style_pass_forced_joins() -> u64 {
    STYLE_PASS_FORCED_JOINS.with(Cell::get)
}

/// Whether the style engine `engine` is lent to a style pass alone: the one stage beside which a
/// main-side write to that engine's document can queue its style inputs for the pass's drain
/// instead of joining it.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_style_pass_holds_style_engine(
    engine: crate::css::style::StyleEngineHandle,
) -> bool {
    engine.holder() == Some(crate::css::style::engine_home::Holder::StylePass)
}

/// Whether the style engine `engine` is lent to a layout pass that reads it, or a clock tick that
/// samples it: what the host publishes to that engine beside it waits for the stage to be taken
/// back (see `StyleEngine::publish_input`).
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_layout_pass_in_flight_for(engine: crate::css::style::StyleEngineHandle) -> bool {
    engine.holder() == Some(crate::css::style::engine_home::Holder::LayoutPass)
}

/// The main thread is about to wait for a stage of the frame that runs for the arena `arena` (or of
/// any frame, for 0) to send a style engine home. A held stage would never send it, so the
/// wait releases a test's hold on the stages it may wait for, as a join does.
pub(crate) fn release_holds_for_style_engine_wait(arena: usize) {
    SUBMITTED.with_borrow_mut(|submitted| {
        submitted
            .iter_mut()
            .filter(|stage| arena == 0 || stage.arena == arena)
            .for_each(SubmittedStage::release_hold_unless_finished);
    });
}

/// The main thread takes in the frame that holds the style engine of the document whose arena is
/// `arena` (or any frame, for an engine no arena links), for an entrance that `file` and `line`
/// name (see `crate::css::style::engine_home`).
pub(crate) fn join_frame_holding_style_engine(arena: usize, file: &'static str, line: u32) {
    join_frame_in_flight_for_stage(|stage| arena == 0 || stage.arena == arena, file, line, 0);
}

/// Whether the frame in flight owns the arena `arena` with its layout pass or clock tick only,
/// beside which what the document publishes to its style engine waits (see
/// [`rust_stage_thread_layout_pass_in_flight_for`]). A main-side change the arena would take in
/// beside such a frame can wait for the frame's take-back instead of joining it.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_arena_changes_wait_for_frame(arena: *mut c_void) -> bool {
    !no_stage_is_submitted()
        && SUBMITTED.with_borrow(|submitted| {
            let mut owners = submitted
                .iter()
                .filter(|stage| stage.arena == arena as usize)
                .peekable();
            // A style pass reaches the style engine beside what the document publishes to it.
            owners.peek().is_some() && owners.all(|stage| stage.role != "style")
        })
}

/// Counts a forced join against the label of each stage of the frame in flight it takes in.
fn count_forced_join() {
    // A flight is counted as the stage whose hold it has.
    let mut roles: Vec<&'static str> =
        SUBMITTED.with_borrow(|submitted| submitted.iter().map(|stage| stage.role).collect());
    roles.extend(PAINTING.with_borrow(|painting| painting.iter().map(|stage| stage.label).collect::<Vec<_>>()));
    FORCED_JOINS.with_borrow_mut(|counts| {
        for (index, role) in roles.iter().enumerate() {
            if roles[..index].contains(role) {
                continue;
            }
            match counts.iter_mut().find(|(label, _)| label == role) {
                Some((_, count)) => *count += 1,
                None => counts.push((role, 1)),
            }
        }
    });
}

/// How many forced joins on the calling thread took in a frame with a stage labelled `label`.
///
/// # Safety
///
/// `label` must point to `label_length` bytes of UTF-8.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_stage_thread_forced_joins(label: *const u8, label_length: usize) -> u64 {
    // SAFETY: Guaranteed by the caller.
    let label = unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts(label, label_length)) };
    FORCED_JOINS.with_borrow(|counts| {
        counts
            .iter()
            .find(|(counted, _)| *counted == label)
            .map_or(0, |(_, count)| *count)
    })
}

/// Orders what the calling thread wrote before it hands a style engine over (the main thread to a stage,
/// or a stage back home) before what the thread that takes it reads after [`acquire_handoff`].
pub(crate) fn release_handoff() {
    tsan::release(stage_thread());
}

/// The taking thread's side of [`release_handoff`].
pub(crate) fn acquire_handoff() {
    tsan::acquire(stage_thread());
}

/// Test only: waits up to `timeout_ms` for every stage of the main thread's frame in flight to
/// finish, without taking the frame back. Returns false if there is no frame in flight or it has not
/// finished in time.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_wait_for_frame_in_flight_to_finish(timeout_ms: u32) -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_millis(timeout_ms.into());
    while has_frame_in_flight() {
        if frame_in_flight_has_finished() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    false
}

/// Whether the main thread has a frame in flight. Asking does not wait for it.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_has_frame_in_flight() -> bool {
    has_frame_in_flight()
}

/// Whether every stage of the main thread's frame in flight has finished. Does not wait.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_frame_in_flight_has_finished() -> bool {
    frame_in_flight_has_finished()
}

/// Waits for the main thread's frame in flight and takes it back, without running its
/// consume-commit, which is the caller's. Returns whether there was one.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_take_frame_in_flight() -> bool {
    take_frame_in_flight(&crate::frame_news::LegacyJoin::waiting_for_the_frame())
}

/// A forced join of the whole frame in flight, as an access to render state makes one: waits for
/// it, takes it back and runs the scheduler's consume-commit. `file` and `line` name the C++ call
/// site for the forced-join log.
///
/// # Safety
///
/// `file` and `file_length` must name a string that lives for the rest of the process, as a
/// `SourceLocation`'s file name does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_stage_thread_join_frame_in_flight(file: *const u8, file_length: usize, line: u32) {
    // SAFETY: The caller passes a string that lives for the rest of the process.
    let file = unsafe { call_site_file(file, file_length) };
    join_frame_in_flight_at(std::ptr::null_mut(), file, line, 0);
}

/// The file name of a C++ call site, as `SourceLocation` gives it.
///
/// # Safety
///
/// `file` and `file_length` must name a string that lives for the rest of the process.
pub(crate) unsafe fn call_site_file(file: *const u8, file_length: usize) -> &'static str {
    assert!(!file.is_null(), "call site file name is null");
    // SAFETY: The caller passes a string that lives for the rest of the process.
    let bytes = unsafe { std::slice::from_raw_parts(file, file_length) };
    std::str::from_utf8(bytes).unwrap_or("<non-UTF-8 file name>")
}

/// Sends `message` to the render owner, the Rendering thread. Hands it back where the calling thread is the owner.
pub(crate) fn send_to_owner(message: crate::render_owner::ToOwner) -> Result<(), crate::render_owner::ToOwner> {
    crate::render_owner::note_sending(&message);
    if on_owner_thread() {
        return Err(message);
    }
    crate::css::style::engine_home::free_returned_style_engines();
    let thread = stage_thread();
    tsan::release(thread);
    if thread.jobs.send(StageMessage::Owner(message)).is_err() {
        // The Rendering thread only goes away if the process is going away.
        std::process::abort();
    }
    Ok(())
}

/// Whether the calling thread is the render owner's Rendering thread.
pub(crate) fn on_owner_thread() -> bool {
    std::thread::current().id() == stage_thread().id
}

/// Tells TSan about the ordering a reply from the render owner gave the calling thread.
pub(crate) fn acquire_owner() {
    tsan::acquire(stage_thread());
}

/// The next message of the stage thread's own loop: the oldest deferred one, or the next to arrive.
fn next_message() -> Option<StageMessage> {
    if let Some(message) = DEFERRED.with_borrow_mut(VecDeque::pop_front) {
        return Some(message);
    }
    // With nothing waiting, the owner takes in what the document threads sent it before it waits for more: beside
    // them, rather than at the start of the next unit or question they would wait for.
    if on_owner_thread() {
        let waiting = try_incoming();
        if waiting.is_some() {
            return waiting;
        }
        tsan::acquire(stage_thread());
        crate::render_owner::take_in_sent_changes();
        tsan::release(stage_thread());
    }
    next_incoming()
}

/// The next message to arrive, for a wait inside something the stage thread runs, which leaves what it defers for the
/// stage thread's own loop.
fn next_incoming() -> Option<StageMessage> {
    INCOMING.with_borrow(|incoming| {
        let incoming = incoming.as_ref()?;
        let spin = if incoming.answered_a_waiting_thread.replace(false) {
            incoming.next_question_spin
        } else {
            Duration::ZERO
        };
        recv_after_spinning(&incoming.messages, spin).ok()
    })
}

/// The next message that has already arrived, if any.
fn try_incoming() -> Option<StageMessage> {
    INCOMING.with_borrow(|incoming| incoming.as_ref()?.messages.try_recv().ok())
}

/// On the stage thread, notes that it answered a thread waiting for it.
fn note_answered_a_waiting_thread() {
    INCOMING.with_borrow(|incoming| {
        if let Some(incoming) = incoming {
            incoming.answered_a_waiting_thread.set(true);
        }
    });
}

/// Receives from `receiver`, polling it for up to `spin` before sleeping.
fn recv_after_spinning<T>(receiver: &Receiver<T>, spin: Duration) -> Result<T, std::sync::mpsc::RecvError> {
    if spin.is_zero() {
        return receiver.recv();
    }
    let deadline = Instant::now() + spin;
    loop {
        match receiver.try_recv() {
            Ok(value) => return Ok(value),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => return Err(std::sync::mpsc::RecvError),
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
        if Instant::now() >= deadline {
            return receiver.recv();
        }
        std::hint::spin_loop();
    }
}

/// On the Rendering thread, between two units of the rendering update of `document` it runs: serves every message
/// that has arrived and may be served in the middle of that update (a query, a layout unit a main thread waits for,
/// changes to queue, another document's destroy), and defers the rest, and every message about a document after one
/// it defers about that document, for its own loop (see [`crate::render_owner::between_units`]). Answers whether
/// the main thread recalled the update meanwhile, to take its frame back where it is.
pub(crate) fn serve_messages_between_units(document: crate::render_owner::DocumentId) -> bool {
    let thread = stage_thread();
    let mut recalled = RECALLED_WHILE_HELD.with(|recalled| recalled.replace(false));
    while let Some(message) = try_incoming() {
        let deferred = DEFERRED.with_borrow(|deferred| {
            deferred
                .iter()
                .map(|message| match message {
                    StageMessage::Owner(message) => Some(message.document()),
                    StageMessage::Run(_) => None,
                })
                .collect::<Vec<_>>()
        });
        let message = match message {
            StageMessage::Owner(message) => match crate::render_owner::between_units(message, document, &deferred) {
                crate::render_owner::BetweenUnits::Recalled => {
                    recalled = true;
                    continue;
                }
                crate::render_owner::BetweenUnits::Serve(message) => {
                    // What the message runs is not the update's run.
                    let running = RUNNING_SUBMITTED_RUN.with(|running| running.take());
                    tsan::acquire(thread);
                    crate::render_owner::handle(message);
                    tsan::release(thread);
                    RUNNING_SUBMITTED_RUN.with(|slot| slot.set(running));
                    continue;
                }
                crate::render_owner::BetweenUnits::Defer(message) => StageMessage::Owner(message),
            },
            message => message,
        };
        DEFERRED.with_borrow_mut(|deferred| deferred.push_back(message));
    }
    recalled
}

/// On the Rendering thread: whether a rendering update of `document` waits among the messages it deferred.
pub(crate) fn defers_rendering_update_of(document: crate::render_owner::DocumentId) -> bool {
    DEFERRED.with_borrow(|deferred| {
        deferred.iter().any(|message| {
            matches!(message, StageMessage::Owner(message @ crate::render_owner::ToOwner::RenderingUpdate { .. })
                if message.document() == document)
        })
    })
}

/// The thread the running code acts for: on the stage thread, the thread that handed it the stage
/// it is running; anywhere else, the current thread. State owned by one thread may be used by a
/// stage run for that thread.
pub(crate) fn acting_thread() -> ThreadId {
    WAITING_CALLER
        .with(Cell::get)
        .unwrap_or_else(|| std::thread::current().id())
}

/// The Rendering thread, which the render owner runs on.
fn stage_thread() -> &'static StageThread {
    static STAGE_THREAD: OnceLock<StageThread> = OnceLock::new();
    STAGE_THREAD.get_or_init(|| StageThread::spawn("Rendering"))
}

static PAINT_LANE: OnceLock<StageThread> = OnceLock::new();

/// The thread that runs the stages a frame submits for a document without reaching its arena: its
/// display list recordings, which record a frame the document published, and the presentations
/// that publish them from their tickets. Nothing the Rendering thread runs for the main thread,
/// such as a style or layout pass the main thread waits for, queues behind them.
fn paint_lane() -> &'static StageThread {
    PAINT_LANE.get_or_init(|| StageThread::spawn("Painting"))
}

/// Tells TSan about the ordering waiting for the frame's stages gave the calling thread: with every
/// thread that runs them.
fn acquire_stage_threads() {
    tsan::acquire(stage_thread());
    if let Some(lane) = PAINT_LANE.get() {
        tsan::acquire(lane);
    }
}

/// Runs `stage` on the stage thread, and returns its result once it has finished; the calling
/// thread waits meanwhile. When called from the stage thread itself, `stage` runs right here.
///
/// A panic in `stage` continues on the calling thread, as it would have if `stage` had run there;
/// a build that aborts on panic aborts on the stage thread instead.
///
/// `stage` and its result are `Send`, so everything a stage reaches is checked by the compiler,
/// as for a scoped thread. A stage that has to take along a value the compiler cannot check names
/// it with [`CallerWaits`].
pub(crate) fn run_stage<R: Send>(stage: impl FnOnce() -> R + Send) -> R {
    // SAFETY: The stage is `Send`.
    unsafe { run_stage_on(stage_thread(), stage) }
}

/// A value a submitted stage owns although the compiler cannot check that it may cross threads.
pub(crate) struct FrameOwns<F>(F);
// SAFETY: Whoever wraps a value vouches that nothing it holds is reachable from a third thread,
// and that the main thread reaches it only once it has taken the frame back.
unsafe impl<F> Send for FrameOwns<F> {}
impl<F> FrameOwns<F> {
    /// # Safety
    ///
    /// Nothing `value` holds may be reachable from a thread other than the main thread and the
    /// stage thread, the main thread may reach it only once it has taken back the frame the
    /// value goes into, and `value` has to be safe to use and drop on either thread.
    pub(crate) unsafe fn new(value: F) -> Self {
        Self(value)
    }

    // Taken through a method, so a closure captures the wrapper rather than its field.
    pub(crate) fn into_inner(self) -> F {
        self.0
    }

    /// The value, for the thread that owns the frame it goes into as it is.
    pub(crate) fn get_mut(&mut self) -> &mut F {
        &mut self.0
    }
}

/// A value a stage takes along although the compiler cannot check that it may cross threads.
pub(crate) struct CallerWaits<F>(F);
// SAFETY: Whoever wraps a value vouches that nothing it holds is reachable from a third thread,
// and the thread it came from waits until the value has been used and dropped.
unsafe impl<F> Send for CallerWaits<F> {}
impl<F> CallerWaits<F> {
    /// # Safety
    ///
    /// Nothing `value` holds may be reachable from a thread other than the calling thread, which
    /// waits for the stage that takes it, or the stage thread, and `value` has to be safe to use
    /// and drop on either.
    pub(crate) unsafe fn new(value: F) -> Self {
        Self(value)
    }

    // Taken through a method, so a closure captures the wrapper rather than its field.
    pub(crate) fn into_inner(self) -> F {
        self.0
    }
}

/// # Safety
///
/// The stage may reach what the calling thread holds only while it waits.
unsafe fn run_stage_on<R: Send>(thread: &'static StageThread, stage: impl FnOnce() -> R + Send) -> R {
    if std::thread::current().id() == thread.id {
        return stage();
    }
    // This stage would queue behind the submitted ones, and a run a test holds there stays held, so
    // the stage runs right here instead. It reaches nothing a submitted stage owns, and a wait for
    // the held run in it lets that run go on as anywhere.
    if has_frame_in_flight() && stage_thread_holds_run_for_queued_stage() {
        return stage();
    }

    let (to_caller, from_stage) = channel::<()>();
    let mut outcome: Option<Result<R, Box<dyn Any + Send>>> = None;
    let slot = &mut outcome;
    let caller = std::thread::current().id();
    let job: Box<dyn FnOnce() + Send + '_> = Box::new(move || {
        tsan::acquire(thread);
        let waiting_caller = WAITING_CALLER.with(|waiting| waiting.replace(Some(caller)));
        *slot = Some(std::panic::catch_unwind(AssertUnwindSafe(stage)));
        WAITING_CALLER.with(|waiting| waiting.set(waiting_caller));
        tsan::release(thread);
        note_answered_a_waiting_thread();
        // The calling thread is waiting on this reply, so it cannot have gone away.
        let _ = to_caller.send(());
    });
    // SAFETY: The job borrows from the calling thread's frame. It drops everything it captured
    // before it replies, and this function does not return before the reply arrives.
    let job = unsafe { std::mem::transmute::<Box<dyn FnOnce() + Send + '_>, Job>(job) };
    tsan::release(thread);
    if thread.jobs.send(StageMessage::Run(job)).is_err() {
        // The stage thread only goes away if the process is going away.
        std::process::abort();
    }
    // A stage submitted earlier runs first; this stage queues behind it and does not reach what it
    // owns, so the caller waits for this stage's reply only.
    recv_after_spinning(&from_stage, thread.answer_spin).unwrap_or_else(|_| std::process::abort());
    tsan::acquire(thread);
    match outcome.expect("a finished stage leaves its outcome") {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Where the render owner answers a main thread that waits for it, and which thread it acts for meanwhile.
pub(crate) struct OwnerReplyTo<R> {
    thread: &'static StageThread,
    caller: ThreadId,
    reply: Sender<std::thread::Result<R>>,
}

impl<R> OwnerReplyTo<R> {
    /// On the owner: answers with what `unit` answers, acting for the waiting thread. A panic in `unit` goes to the
    /// waiting thread as its answer, and the owner goes on.
    pub(crate) fn answer(self, unit: impl FnOnce() -> R) {
        let Self { thread, caller, reply } = self;
        tsan::acquire(thread);
        let waiting_caller = WAITING_CALLER.with(|waiting| waiting.replace(Some(caller)));
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(unit));
        WAITING_CALLER.with(|waiting| waiting.set(waiting_caller));
        tsan::release(thread);
        note_answered_a_waiting_thread();
        // The waiting thread keeps the receiver until it has the answer.
        let _ = reply.send(outcome);
    }
}

/// Whether a thread that waits for the owner now does the owner's work itself: the owner holds a run for a test, which
/// what the thread would wait for queues behind.
pub(crate) fn owner_work_runs_here() -> bool {
    !on_owner_thread() && has_frame_in_flight() && stage_thread_holds_run_for_queued_stage()
}

/// Sends the render owner the message `message` makes of where it answers, and waits for the answer, spending `_wait`,
/// the caller's right to wait. The owner joins the calling thread for nothing: the thread only waits. Where the calling
/// thread is the owner, or the message would queue behind a run a test holds, `here` answers right here, as the owner.
pub(crate) fn wait_for_owner<R>(
    _wait: impl crate::render_owner::OwnerWait,
    message: impl FnOnce(OwnerReplyTo<R>) -> crate::render_owner::ToOwner,
    here: impl FnOnce(&crate::render_owner::Owner) -> R,
) -> std::thread::Result<R> {
    if on_owner_thread() || (has_frame_in_flight() && stage_thread_holds_run_for_queued_stage()) {
        return Ok(crate::render_owner::do_owner_work_here(here));
    }
    send_and_wait(stage_thread(), message)
}

/// Sends the render owner, which is not the calling thread, the message `message` makes of where it answers, and waits
/// for the answer, as [`wait_for_owner`] does, for what only the owner runs: the owner serves it between the units of
/// whatever it runs, a run a test holds included. Answers `None` where the calling thread is the owner.
pub(crate) fn wait_for_owner_thread<R>(
    _wait: impl crate::render_owner::OwnerWait,
    message: impl FnOnce(OwnerReplyTo<R>) -> crate::render_owner::ToOwner,
) -> Option<std::thread::Result<R>> {
    (!on_owner_thread()).then(|| send_and_wait(stage_thread(), message))
}

fn send_and_wait<R>(
    thread: &'static StageThread,
    message: impl FnOnce(OwnerReplyTo<R>) -> crate::render_owner::ToOwner,
) -> std::thread::Result<R> {
    let (reply, answered) = channel();
    let message = message(OwnerReplyTo {
        thread,
        caller: std::thread::current().id(),
        reply,
    });
    crate::render_owner::note_sending(&message);
    tsan::release(thread);
    if thread.jobs.send(StageMessage::Owner(message)).is_err() {
        // The Rendering thread only goes away if the process is going away.
        std::process::abort();
    }
    let outcome = recv_after_spinning(&answered, thread.answer_spin).unwrap_or_else(|_| std::process::abort());
    tsan::acquire(thread);
    outcome
}

/// Where a unit test's owner answers a unit, with how the test waits for the answer.
#[cfg(test)]
pub(crate) fn owner_reply_for_test<R>() -> (OwnerReplyTo<R>, impl FnOnce() -> std::thread::Result<R>) {
    let (reply, answered) = channel();
    let reply = OwnerReplyTo {
        thread: tests::test_thread(),
        caller: std::thread::current().id(),
        reply,
    };
    // A reply dropped unanswered is the answer's error.
    (reply, move || {
        answered.recv().unwrap_or_else(|error| Err(Box::new(error)))
    })
}

/// Runs `stage` on a stage thread of the unit tests' own, whatever the environment says.
#[cfg(test)]
pub(crate) fn run_stage_for_test<R: Send>(stage: impl FnOnce() -> R + Send) -> R {
    // SAFETY: The stage is `Send`.
    unsafe { run_stage_on(tests::test_thread(), stage) }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn test_thread() -> &'static StageThread {
        static THREAD: OnceLock<StageThread> = OnceLock::new();
        THREAD.get_or_init(|| StageThread::spawn("Rendering"))
    }

    #[test]
    fn stage_runs_on_the_stage_thread_and_sees_the_callers_state() {
        let mut state = 1;
        let (result, ran_on) = run_stage_for_test(|| {
            state += 1;
            (state * 10, std::thread::current().id())
        });
        assert_eq!(result, 20);
        assert_eq!(state, 2);
        assert_eq!(ran_on, test_thread().id);
    }

    #[test]
    fn a_panic_in_a_unit_for_a_waiting_thread_answers_it_and_the_owner_goes_on() {
        let (reply, answered) = channel();
        let unit = OwnerReplyTo::<u32> {
            thread: test_thread(),
            caller: std::thread::current().id(),
            reply,
        };
        run_stage_for_test(move || unit.answer(|| panic!("unit failed")));
        let outcome = answered.recv().unwrap_or_else(|error| Err(Box::new(error)));
        let payload = outcome.expect_err("the panic is the answer");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"unit failed"));
        assert_eq!(run_stage_for_test(|| 7), 7);
    }

    #[test]
    fn nested_stage_runs_in_place() {
        let (outer, inner) = run_stage_for_test(|| {
            let outer = std::thread::current().id();
            (outer, run_stage_for_test(|| std::thread::current().id()))
        });
        assert_eq!(outer, test_thread().id);
        assert_eq!(inner, outer);
    }

    #[test]
    fn a_stage_run_defers_releases_into_the_callers_style_update() {
        use crate::css::ffi_stats::*;
        rust_style_ffi_complete_style_update_begin();
        run_stage_for_test(|| release_utf16_fly_string(0x1230));
        let releases = rust_style_ffi_complete_style_update_end();
        // SAFETY: The view stays valid until the releases are cleared.
        let released = unsafe { std::slice::from_raw_parts(releases.fly_strings, releases.fly_string_count) };
        // Tests run in parallel, and their drops join any update that is open at the time.
        assert!(released.contains(&0x1230));
        rust_deferred_cpp_releases_clear();
    }

    #[test]
    fn panic_in_stage_reaches_the_caller_and_the_thread_survives() {
        let outcome = std::panic::catch_unwind(|| run_stage_for_test(|| panic!("stage failed")));
        let payload = outcome.expect_err("the panic must reach the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"stage failed"));
        assert_eq!(run_stage_for_test(|| 7), 7);
    }
}
