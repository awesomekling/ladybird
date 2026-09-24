/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The thread the render pipeline's sealed stages run on.
//!
//! With `LIBWEB_STAGE_THREAD=lockstep`, the tree build walk, the layout stage, the display list
//! recording stage and the render passes that run before the rows are published (the scrollable
//! overflow measurement, the visual context update, the scroll state refresh and the hit-test
//! list's derived structures) run on one thread of their own while the thread that called them
//! waits for the result. Nothing runs concurrently, so the stages see exactly the state they would have seen on
//! the calling thread, but everything they depend on that belongs to a thread (thread-local state,
//! thread-bound handles, stack assumptions) is exercised the way a render thread will exercise it.
//! Without the variable, stages run on the calling thread.
//!
//! A stage run can also join its caller: [`run_overlappable_stage_with_joins`] hands the stage a [`MainJoins`],
//! through which it runs a piece of main-thread work on the waiting caller and continues with the
//! result. The caller runs only the work it is handed, and the stage thread runs only the stages
//! the caller starts from inside that work, so the two still take turns.
//!
//! There is one stage thread per process. A WebContent process runs every document it hosts on its
//! one main thread, so a thread per process is also a thread per event loop.
//!
//! `LIBWEB_STAGE_THREAD=overlap` runs every stage on the stage thread as lockstep does, and lets
//! the rendering update submit the stages [`LIBWEB_STAGE_OVERLAP`](overlapping_stages) names
//! instead of waiting for them: [`submit_stage`] hands the stage to the stage thread and returns,
//! and the main thread goes back to its event loop while the stage runs. A submitted stage has no
//! joins; it owns the arena of the document it runs for until the main thread takes it back. The
//! frame scheduler takes it back at the top of the event loop once the stage has finished, or a
//! main-thread access to that arena takes it back first (a forced join, logged once per call
//! site). Either way the main thread blocks on the stage's reply and never spins its event loop
//! inside a stage run, and the scheduler's consume-commit runs before the access goes on.

use crate::css::ffi_stats::{StyleUpdateScope, install_style_update_scope, take_style_update_scope};
use crate::stage::MainThread;
use std::any::Any;
use std::cell::{Cell, RefCell};
use std::ffi::c_void;
use std::panic::AssertUnwindSafe;
use std::sync::OnceLock;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::ThreadId;

type Job = Box<dyn FnOnce() + Send>;
type JoinWork<'a> = Box<dyn for<'main> FnOnce(&MainThread<'main>) -> Result<(), Box<dyn Any + Send>> + Send + 'a>;
type MainWork = JoinWork<'static>;

enum StageMessage {
    Run(Job),
    /// The caller has run the work the stage's innermost join handed it.
    JoinFinished(Box<StyleUpdateScope>, Result<(), Box<dyn Any + Send>>),
}

enum CallerMessage {
    Finished(StyleUpdateScope),
    Join(MainWork, StyleUpdateScope),
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
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum StageThreadMode {
    Lockstep,
    Overlap,
}

fn stage_thread_mode() -> Option<StageThreadMode> {
    static MODE: OnceLock<Option<StageThreadMode>> = OnceLock::new();
    *MODE.get_or_init(|| match std::env::var_os("LIBWEB_STAGE_THREAD") {
        Some(mode) if mode == "lockstep" => Some(StageThreadMode::Lockstep),
        Some(mode) if mode == "overlap" => Some(StageThreadMode::Overlap),
        _ => None,
    })
}

/// The stages the rendering update submits under `LIBWEB_STAGE_THREAD=overlap`: a comma-separated
/// list in `LIBWEB_STAGE_OVERLAP`, or the recording when it is not set.
fn overlapping_stages() -> &'static [String] {
    static STAGES: OnceLock<Vec<String>> = OnceLock::new();
    STAGES.get_or_init(|| {
        std::env::var("LIBWEB_STAGE_OVERLAP")
            .unwrap_or_else(|_| "recording".into())
            .split(',')
            .map(|stage| stage.trim().to_owned())
            .filter(|stage| !stage.is_empty())
            .collect()
    })
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
}

static FRAME_SCHEDULER_HOST: OnceLock<FfiFrameSchedulerHost> = OnceLock::new();

/// Whether the stages run under `LIBWEB_STAGE_THREAD=overlap`, and so want a frame scheduler host.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_wants_frame_scheduler_host() -> bool {
    stage_thread_mode() == Some(StageThreadMode::Overlap) && FRAME_SCHEDULER_HOST.get().is_none()
}

/// Installs the main thread's frame scheduler host. The first host installed stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_set_frame_scheduler_host(host: FfiFrameSchedulerHost) {
    let _ = FRAME_SCHEDULER_HOST.set(host);
}

// The size Linux and macOS give a process's main thread, where the stages ran before.
const STAGE_THREAD_STACK_SIZE: usize = 8 * 1024 * 1024;

impl StageThread {
    fn spawn() -> Self {
        let (jobs, incoming) = channel::<StageMessage>();
        let thread = std::thread::Builder::new()
            .name("RenderStages".into())
            .stack_size(STAGE_THREAD_STACK_SIZE)
            .spawn(move || {
                INCOMING.with(|slot| *slot.borrow_mut() = Some(incoming));
                while let Some(message) = next_message() {
                    match message {
                        StageMessage::Run(job) => job(),
                        StageMessage::JoinFinished(..) => unreachable!("a join finished with no stage waiting for it"),
                    }
                }
            })
            .expect("the stage thread could not be started");
        Self {
            jobs,
            id: thread.thread().id(),
        }
    }
}

thread_local! {
    // On the stage thread, the thread waiting for the stage it is running, or the thread that
    // submitted it.
    static WAITING_CALLER: Cell<Option<ThreadId>> = const { Cell::new(None) };
    // On the stage thread, where the caller's messages arrive. A stage waiting for a join reads
    // them too, since the work it joined for can start stages of its own.
    static INCOMING: RefCell<Option<Receiver<StageMessage>>> = const { RefCell::new(None) };
    // On the calling thread, the stages it has submitted and not taken back yet, in submission
    // order. Together they are the frame in flight.
    static SUBMITTED: RefCell<Vec<SubmittedStage>> = const { RefCell::new(Vec::new()) };
    // On the calling thread, how deep it is in work a stage joined it for.
    static RUNNING_JOIN_WORK: Cell<u32> = const { Cell::new(0) };
    // On the calling thread, the call sites that forced a join already logged.
    static FORCED_JOIN_SITES: RefCell<std::collections::HashSet<(&'static str, usize, u32)>> =
        RefCell::new(std::collections::HashSet::new());
}

type StageOutcome = Result<(), Box<dyn Any + Send>>;

/// A stage the calling thread has submitted and not taken back yet.
struct SubmittedStage {
    label: &'static str,
    // The arena the stage owns while it runs, as the handle the main thread knows it by.
    arena: usize,
    from_stage: Receiver<StageOutcome>,
    outcome: Option<StageOutcome>,
}

impl SubmittedStage {
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

    fn wait(&mut self) -> StageOutcome {
        match self.outcome.take() {
            Some(outcome) => outcome,
            None => self.from_stage.recv().unwrap_or_else(|_| std::process::abort()),
        }
    }
}

fn run_join_work(
    thread: &'static StageThread,
    main_thread: Option<&MainThread<'_>>,
    work: MainWork,
    style_update: StyleUpdateScope,
) {
    tsan::acquire(thread);
    install_style_update_scope(style_update);
    let main_thread = main_thread.expect("only a stage started with joins joins its caller");
    RUNNING_JOIN_WORK.with(|depth| depth.set(depth.get() + 1));
    let outcome = work(main_thread);
    RUNNING_JOIN_WORK.with(|depth| depth.set(depth.get() - 1));
    // The reply is built before the release, so its box is ordered before the stage reads it.
    let reply = StageMessage::JoinFinished(Box::new(take_style_update_scope()), outcome);
    tsan::release(thread);
    if thread.jobs.send(reply).is_err() {
        std::process::abort();
    }
}

/// Whether the rendering update submits the stage `label` rather than waiting for it: under
/// `LIBWEB_STAGE_THREAD=overlap`, with a frame scheduler, if `LIBWEB_STAGE_OVERLAP` names `label`,
/// and only from the main thread's own code, not from work a stage joined it for.
pub(crate) fn submits(label: &'static str) -> bool {
    stage_thread_mode() == Some(StageThreadMode::Overlap)
        && FRAME_SCHEDULER_HOST.get().is_some()
        && RUNNING_JOIN_WORK.with(Cell::get) == 0
        && stage_thread().is_some_and(|thread| std::thread::current().id() != thread.id)
        && overlapping_stages().iter().any(|stage| stage == label)
}

/// Hands `stage` to the stage thread and returns at once. The stage owns the arena `arena` until
/// the main thread takes the frame back: the frame scheduler does at the top of its event loop
/// once the stage has finished, and a main-thread access to the arena does before it goes on
/// ([`join_frame_in_flight`]).
///
/// # Safety
///
/// Until the frame is taken back, nothing but `stage` may reach what `stage` holds: every
/// main-thread path to it has to go through [`join_frame_in_flight`] first.
pub(crate) unsafe fn submit_stage(label: &'static str, arena: *mut c_void, stage: impl FnOnce() + Send + 'static) {
    let thread = stage_thread().expect("only a stage thread runs submitted stages");
    debug_assert!(submits(label), "the stage {label} is not submitted");
    let (to_caller, from_stage) = channel::<StageOutcome>();
    let caller = std::thread::current().id();
    let job: Job = Box::new(move || {
        tsan::acquire(thread);
        let waiting_caller = WAITING_CALLER.with(|waiting| waiting.replace(Some(caller)));
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(stage));
        // A submitted stage runs outside any style update of the caller's; whatever it left in
        // the stage thread's style update state goes with it.
        drop(take_style_update_scope());
        WAITING_CALLER.with(|waiting| waiting.set(waiting_caller));
        tsan::release(thread);
        // The caller keeps the receiver until it has taken this reply.
        let _ = to_caller.send(outcome);
        frame_completion_notify();
    });
    SUBMITTED.with(|submitted| {
        submitted.borrow_mut().push(SubmittedStage {
            label,
            arena: arena as usize,
            from_stage,
            outcome: None,
        });
    });
    tsan::release(thread);
    if thread.jobs.send(StageMessage::Run(job)).is_err() {
        // The stage thread only goes away if the process is going away.
        std::process::abort();
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
    SUBMITTED.with(|submitted| !submitted.borrow().is_empty())
}

/// Whether the frame in flight owns the arena `arena`.
pub(crate) fn frame_in_flight_owns(arena: *mut c_void) -> bool {
    SUBMITTED.with(|submitted| submitted.borrow().iter().any(|stage| stage.arena == arena as usize))
}

/// Whether every stage of the frame in flight has finished. Does not wait.
pub(crate) fn frame_in_flight_has_finished() -> bool {
    SUBMITTED.with(|submitted| submitted.borrow_mut().iter_mut().all(SubmittedStage::poll))
}

/// Waits for every stage of the frame in flight and takes the frame back. Returns whether there was
/// one. A panic in one of its stages continues here. The frame's effects are the caller's to apply.
pub(crate) fn take_frame_in_flight() -> bool {
    let stages = SUBMITTED.with(|submitted| std::mem::take(&mut *submitted.borrow_mut()));
    if stages.is_empty() {
        return false;
    }
    let thread = stage_thread().expect("only a stage thread runs submitted stages");
    let mut panic = None;
    for mut stage in stages {
        if let Err(payload) = stage.wait() {
            panic.get_or_insert(payload);
        }
    }
    tsan::acquire(thread);
    if let Some(payload) = panic {
        std::panic::resume_unwind(payload);
    }
    true
}

/// Called where main-thread code reaches render-owned state: if the frame in flight owns the arena
/// `arena` (or any, for a null `arena`), waits for the frame, takes it back and runs the frame
/// scheduler's consume-commit, so the access finds the document as the frame left it. Work a
/// stage joined the main thread for belongs to that stage and does not wait. Logs each call site
/// that forced a join once.
#[track_caller]
pub(crate) fn join_frame_in_flight(arena: *mut c_void) {
    if RUNNING_JOIN_WORK.with(Cell::get) != 0 {
        return;
    }
    let label = SUBMITTED.with(|submitted| {
        submitted
            .borrow()
            .iter()
            .find(|stage| arena.is_null() || stage.arena == arena as usize)
            .map(|stage| stage.label)
    });
    let Some(label) = label else {
        return;
    };
    let location = std::panic::Location::caller();
    let first_time = FORCED_JOIN_SITES.with(|sites| {
        sites
            .borrow_mut()
            .insert((location.file(), location.line() as usize, location.column()))
    });
    if first_time {
        eprintln!("STAGE OVERLAP: forced join of {label} at {location}");
    }
    take_frame_in_flight();
    let host = FRAME_SCHEDULER_HOST.get().expect("a submitted frame has a scheduler");
    // SAFETY: Called on the main thread, with the frame taken back.
    unsafe { (host.consume_commit)() }
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
    take_frame_in_flight()
}

/// A forced join of the whole frame in flight, as an access to render state makes one: waits for
/// it, takes it back and runs the scheduler's consume-commit.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_join_frame_in_flight() {
    join_frame_in_flight(std::ptr::null_mut());
}

fn next_message() -> Option<StageMessage> {
    INCOMING.with(|incoming| incoming.borrow().as_ref().and_then(|incoming| incoming.recv().ok()))
}

/// The thread the running code acts for: on the stage thread, the thread that handed it the stage
/// it is running; anywhere else, the current thread. State owned by one thread may be used by a
/// stage run for that thread.
pub(crate) fn acting_thread() -> ThreadId {
    WAITING_CALLER
        .with(Cell::get)
        .unwrap_or_else(|| std::thread::current().id())
}

fn stage_thread() -> Option<&'static StageThread> {
    static STAGE_THREAD: OnceLock<Option<StageThread>> = OnceLock::new();
    STAGE_THREAD
        .get_or_init(|| stage_thread_mode().map(|_| StageThread::spawn()))
        .as_ref()
}

/// Runs `stage` on the stage thread if there is one, and returns its result once it has finished;
/// the calling thread waits meanwhile. Without a stage thread, or when called from the stage thread
/// itself, `stage` runs right here.
///
/// A panic in `stage` continues on the calling thread, as it would have if `stage` had run there;
/// a build that aborts on panic aborts on the stage thread instead.
///
/// `stage` and its result are `Send`, so everything a stage reaches is checked by the compiler,
/// as for a scoped thread. A stage that has to take along a value the compiler cannot check names
/// it with [`CallerWaits`].
pub(crate) fn run_stage<R: Send>(stage: impl FnOnce() -> R + Send) -> R {
    match stage_thread() {
        // SAFETY: The stage has no joins, and it is `Send`.
        Some(thread) => unsafe { run_stage_on(thread, None, |_| stage()) },
        None => stage(),
    }
}

/// Runs `stage` as [`run_stage`] does, and lets it join the calling thread: while `stage` waits
/// in [`MainJoins::join`], the calling thread runs the work the join hands it, with the main
/// thread capability it holds.
///
/// A join is answered by the next reply the stage thread gets, which holds because one thread
/// per process, the one its documents live on, starts the stages. A stage with joins is never
/// submitted: its caller always waits for it.
///
/// # Safety
///
/// The work each join hands the calling thread may capture references to state that is neither
/// `Send` nor `Sync`. The caller must ensure that no thread other than the stage thread can reach
/// that state while the work runs. The stage thread itself cannot, since it waits for the result.
pub(crate) unsafe fn run_stage_with_joins<R: Send>(
    main_thread: &MainThread<'_>,
    stage: impl FnOnce(&MainJoins<'_>) -> R + Send,
) -> R {
    match stage_thread() {
        // SAFETY: Guaranteed by the caller.
        Some(thread) => unsafe { run_stage_on(thread, Some(main_thread), stage) },
        None => stage(&MainJoins(JoinTarget::InPlace(Some(main_thread)))),
    }
}

/// How a stage started by [`run_stage_with_joins`] reaches the thread that waits for it.
pub(crate) struct MainJoins<'main>(JoinTarget<'main>);

enum JoinTarget<'main> {
    /// The stage runs on the calling thread itself; a stage started without joins has no main
    /// thread to join.
    InPlace(Option<&'main MainThread<'main>>),
    Caller {
        thread: &'static StageThread,
        caller: Sender<CallerMessage>,
    },
}

impl MainJoins<'_> {
    /// Runs `work` on the thread that waits for this stage and returns its result. The stage waits
    /// meanwhile, and runs any stage `work` starts.
    pub(crate) fn join<R: Send>(&self, work: impl FnOnce(&MainThread<'_>) -> R) -> R {
        let (thread, caller) = match &self.0 {
            JoinTarget::InPlace(main_thread) => {
                return work(main_thread.expect("only a stage started with joins joins its caller"));
            }
            JoinTarget::Caller { thread, caller } => (*thread, caller),
        };
        let mut result = None;
        let slot = &mut result;
        let work = CallerWaits(work);
        let work: JoinWork<'_> = Box::new(move |main_thread| {
            let work = work.into_inner();
            std::panic::catch_unwind(AssertUnwindSafe(|| *slot = Some(work(main_thread))))
        });
        // SAFETY: The work borrows from this frame. The caller replies only once it has run the
        // work and dropped it, and this function does not return before the reply arrives.
        let work = unsafe { std::mem::transmute::<JoinWork<'_>, MainWork>(work) };
        let request = CallerMessage::Join(work, take_style_update_scope());
        tsan::release(thread);
        if caller.send(request).is_err() {
            // The caller waits for this stage, so it cannot have gone away.
            std::process::abort();
        }
        loop {
            match next_message() {
                // The work started a stage of its own.
                Some(StageMessage::Run(job)) => job(),
                Some(StageMessage::JoinFinished(style_update, outcome)) => {
                    tsan::acquire(thread);
                    install_style_update_scope(*style_update);
                    if let Err(payload) = outcome {
                        std::panic::resume_unwind(payload);
                    }
                    break;
                }
                None => std::process::abort(),
            }
        }
        result.expect("a join that finished without panicking has a result")
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
/// As for [`run_stage_with_joins`]. A stage that joins needs `main_thread`.
unsafe fn run_stage_on<R: Send>(
    thread: &'static StageThread,
    main_thread: Option<&MainThread<'_>>,
    stage: impl FnOnce(&MainJoins<'_>) -> R + Send,
) -> R {
    if std::thread::current().id() == thread.id {
        return stage(&MainJoins(JoinTarget::InPlace(main_thread)));
    }

    let (to_caller, from_stage) = channel::<CallerMessage>();
    let mut outcome: Option<Result<R, Box<dyn Any + Send>>> = None;
    let slot = &mut outcome;
    let caller = std::thread::current().id();
    // The stage runs inside whatever style update the caller has open, so it takes that update's
    // state along and hands it back with its result.
    let style_update = take_style_update_scope();
    let job: Box<dyn FnOnce() + Send + '_> = Box::new(move || {
        tsan::acquire(thread);
        let waiting_caller = WAITING_CALLER.with(|waiting| waiting.replace(Some(caller)));
        install_style_update_scope(style_update);
        let joins = MainJoins(JoinTarget::Caller {
            thread,
            caller: to_caller.clone(),
        });
        *slot = Some(std::panic::catch_unwind(AssertUnwindSafe(|| stage(&joins))));
        drop(joins);
        let style_update = take_style_update_scope();
        WAITING_CALLER.with(|waiting| waiting.set(waiting_caller));
        tsan::release(thread);
        // The calling thread is waiting on this reply, so it cannot have gone away.
        let _ = to_caller.send(CallerMessage::Finished(style_update));
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
    let style_update = loop {
        match from_stage.recv() {
            Ok(CallerMessage::Finished(style_update)) => break style_update,
            Ok(CallerMessage::Join(work, style_update)) => run_join_work(thread, main_thread, work, style_update),
            Err(_) => std::process::abort(),
        }
    };
    tsan::acquire(thread);
    install_style_update_scope(style_update);
    match outcome.expect("a finished stage leaves its outcome") {
        Ok(value) => value,
        Err(payload) => std::panic::resume_unwind(payload),
    }
}

/// Runs `stage` on a stage thread of the unit tests' own, whatever the environment says.
#[cfg(test)]
pub(crate) fn run_stage_for_test<R: Send>(stage: impl FnOnce() -> R + Send) -> R {
    // SAFETY: The stage has no joins, and it is `Send`.
    unsafe { run_stage_on(tests::test_thread(), None, |_| stage()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn test_thread() -> &'static StageThread {
        static THREAD: OnceLock<StageThread> = OnceLock::new();
        THREAD.get_or_init(StageThread::spawn)
    }

    // A join pairs with the next reply the stage thread gets, so a test that joins needs a stage
    // thread no other test is calling into at the same time.
    fn joining_test_thread() -> &'static StageThread {
        thread_local! {
            static THREAD: &'static StageThread = Box::leak(Box::new(StageThread::spawn()));
        }
        THREAD.with(|thread| *thread)
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
    fn a_join_runs_on_the_caller_and_the_stages_it_starts_run_on_the_stage_thread() {
        let main_thread = crate::stage::MainThread::for_test();
        let caller = std::thread::current().id();
        let mut state = 1;
        // SAFETY: The join captures only the stage's own borrow of `state`.
        let (stage_thread, joined_on, nested_stage_ran_on) = unsafe {
            run_stage_on(joining_test_thread(), Some(&main_thread), |joins| {
                state += 1;
                let (joined_on, nested_stage_ran_on) = joins.join(|_| {
                    state *= 10;
                    (
                        std::thread::current().id(),
                        run_stage_on(joining_test_thread(), None, |_| std::thread::current().id()),
                    )
                });
                (std::thread::current().id(), joined_on, nested_stage_ran_on)
            })
        };
        assert_eq!(stage_thread, joining_test_thread().id);
        assert_eq!(joined_on, caller);
        assert_eq!(nested_stage_ran_on, joining_test_thread().id);
        assert_eq!(state, 20);
    }

    #[test]
    fn a_stage_started_inside_a_join_can_join_again() {
        let main_thread = crate::stage::MainThread::for_test();
        let caller = std::thread::current().id();
        // SAFETY: Nothing is captured that another thread can reach.
        let innermost = unsafe {
            run_stage_on(joining_test_thread(), Some(&main_thread), |joins| {
                joins.join(|main_thread| {
                    run_stage_on(joining_test_thread(), Some(main_thread), |joins| {
                        joins.join(|_| std::thread::current().id())
                    })
                })
            })
        };
        assert_eq!(innermost, caller);
    }

    #[test]
    fn a_panic_in_a_join_reaches_the_stage_and_then_the_caller() {
        let main_thread = crate::stage::MainThread::for_test();
        // SAFETY: Nothing is captured.
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(|| unsafe {
            run_stage_on(joining_test_thread(), Some(&main_thread), |joins| {
                joins.join(|_| -> () { panic!("join failed") })
            })
        }));
        let payload = outcome.expect_err("the panic must reach the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"join failed"));
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
