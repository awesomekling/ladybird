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
//! `LIBWEB_STAGE_THREAD=overlap` is a diagnostic mode for finding what stands in the way of a render
//! thread that runs alongside the main thread. It runs every stage on the stage thread as lockstep
//! does, but while one of the stages [`LIBWEB_STAGE_OVERLAP`](overlapping_stages) names runs, the
//! caller does not simply wait: it spins its event loop, so tasks, timers and IPC messages run on
//! the main thread concurrently with the stage. The first time main-thread code turns an arena
//! handle into the arena meanwhile, it waits for the stage to finish (a forced join, logged once
//! per call site). Everything main-thread code reaches past the arena races with the stage, which
//! is what a ThreadSanitizer build of this mode is for.

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

/// The stages that overlap the main thread under `LIBWEB_STAGE_THREAD=overlap`: a comma-separated
/// list in `LIBWEB_STAGE_OVERLAP`, or the handoff and the recording when it is not set.
fn overlapping_stages() -> &'static [String] {
    static STAGES: OnceLock<Vec<String>> = OnceLock::new();
    STAGES.get_or_init(|| {
        std::env::var("LIBWEB_STAGE_OVERLAP")
            .unwrap_or_else(|_| "handoff,recording".into())
            .split(',')
            .map(|stage| stage.trim().to_owned())
            .filter(|stage| !stage.is_empty())
            .collect()
    })
}

/// How the main thread keeps its event loop going while an overlapping stage runs.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct FfiStageOverlapHost {
    /// Spins the main thread's event loop until `done(context)` returns true. Runs on the main thread.
    pub spin_until: unsafe extern "C" fn(done: unsafe extern "C" fn(*mut c_void) -> bool, context: *mut c_void),
    /// Wakes the main thread's event loop. Runs on any thread.
    pub wake: unsafe extern "C" fn(),
}

static OVERLAP_HOST: OnceLock<FfiStageOverlapHost> = OnceLock::new();

/// Whether the stages run under `LIBWEB_STAGE_THREAD=overlap`, and so want an overlap host.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_wants_overlap_host() -> bool {
    stage_thread_mode() == Some(StageThreadMode::Overlap) && OVERLAP_HOST.get().is_none()
}

/// Installs the main thread's overlap host. The first host installed stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_set_overlap_host(host: FfiStageOverlapHost) {
    let _ = OVERLAP_HOST.set(host);
}

fn wake_overlapping_caller() {
    if let Some(host) = OVERLAP_HOST.get() {
        // SAFETY: The host's wake may be called from any thread.
        unsafe { (host.wake)() }
    }
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
    // On the stage thread, the thread waiting for the stage it is running.
    static WAITING_CALLER: Cell<Option<ThreadId>> = const { Cell::new(None) };
    // On the stage thread, where the caller's messages arrive. A stage waiting for a join reads
    // them too, since the work it joined for can start stages of its own.
    static INCOMING: RefCell<Option<Receiver<StageMessage>>> = const { RefCell::new(None) };
    // On the calling thread, the overlapping stage it spins its event loop for.
    static IN_FLIGHT: Cell<*const InFlight<'static>> = const { Cell::new(std::ptr::null()) };
    // On the calling thread, the stages whose runs are suspended in a spin further up its stack,
    // finished or not.
    static SPINNING: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
    // On the calling thread, how deep it is in work a stage joined it for.
    static RUNNING_JOIN_WORK: Cell<u32> = const { Cell::new(0) };
    // On the calling thread, the call sites that forced a join already logged.
    static FORCED_JOIN_SITES: RefCell<std::collections::HashSet<(&'static str, usize, u32)>> =
        RefCell::new(std::collections::HashSet::new());
}

/// An overlapping stage the calling thread has handed off and not seen finish yet.
struct InFlight<'main> {
    label: &'static str,
    thread: &'static StageThread,
    from_stage: Receiver<CallerMessage>,
    main_thread: Option<&'main MainThread<'main>>,
    finished: Cell<Option<StyleUpdateScope>>,
    done: Cell<bool>,
}

impl InFlight<'_> {
    /// Handles one message from the stage, running the work a join hands over.
    fn handle(&self, message: CallerMessage) {
        match message {
            CallerMessage::Finished(style_update) => {
                self.finished.set(Some(style_update));
                self.done.set(true);
            }
            CallerMessage::Join(work, style_update) => run_join_work(self.thread, self.main_thread, work, style_update),
        }
    }

    /// Blocks until the stage has finished.
    fn wait(&self) {
        while !self.done.get() {
            match self.from_stage.recv() {
                Ok(message) => self.handle(message),
                Err(_) => std::process::abort(),
            }
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

/// Whether a run of the stage `label` is suspended in a spin further up the calling thread's
/// stack. Its caller has not taken in its result yet, and may hold what a second run would need.
pub(crate) fn is_suspended_in_spin(label: &'static str) -> bool {
    SPINNING.with(|spinning| spinning.borrow().contains(&label))
}

/// Whether the calling thread runs beside an overlapping stage that has not finished: it spins its
/// event loop for the stage, and is not running work the stage joined it for.
pub(crate) fn runs_beside_an_overlapping_stage() -> bool {
    let in_flight = IN_FLIGHT.with(Cell::get);
    if in_flight.is_null() || RUNNING_JOIN_WORK.with(Cell::get) != 0 {
        return false;
    }
    // SAFETY: As in `join_overlapping_stage`.
    !unsafe { &*in_flight }.done.get()
}

/// Called where main-thread code reaches render-owned state: if an overlapping stage is running,
/// waits for it to finish. Work a stage joined the main thread for belongs to the stage and does
/// not wait. Logs each call site that forced a join once.
#[track_caller]
pub(crate) fn join_overlapping_stage() {
    let in_flight = IN_FLIGHT.with(Cell::get);
    if in_flight.is_null() || RUNNING_JOIN_WORK.with(Cell::get) != 0 {
        return;
    }
    // SAFETY: The in-flight record lives in the frame of the stage run that spins below us, which
    // clears it before it returns.
    let in_flight = unsafe { &*in_flight };
    if in_flight.done.get() {
        return;
    }
    let location = std::panic::Location::caller();
    let first_time = FORCED_JOIN_SITES.with(|sites| {
        sites
            .borrow_mut()
            .insert((location.file(), location.line() as usize, location.column()))
    });
    if first_time {
        eprintln!("STAGE OVERLAP: forced join of {} at {location}", in_flight.label);
    }
    in_flight.wait();
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

/// Whether the stage `label` overlaps its caller rather than running in lockstep with it.
fn overlaps(label: &'static str) -> bool {
    stage_thread_mode() == Some(StageThreadMode::Overlap)
        && OVERLAP_HOST.get().is_some()
        && IN_FLIGHT.with(Cell::get).is_null()
        && RUNNING_JOIN_WORK.with(Cell::get) == 0
        && overlapping_stages().iter().any(|stage| stage == label)
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
        Some(thread) => unsafe { run_stage_on(thread, None, false, "", |_| stage()) },
        None => stage(),
    }
}

/// Runs `stage` as [`run_stage`] does, but under `LIBWEB_STAGE_THREAD=overlap` the calling thread
/// spins its event loop meanwhile if `LIBWEB_STAGE_OVERLAP` names `label`.
///
/// # Safety
///
/// Under overlap, main-thread code runs while the stage does and may reach what the stage holds
/// through a handle; only diagnostics rely on that mode. Otherwise there is nothing to uphold.
pub(crate) unsafe fn run_overlappable_stage<R: Send>(label: &'static str, stage: impl FnOnce() -> R + Send) -> R {
    match stage_thread() {
        // SAFETY: Guaranteed by the caller.
        Some(thread) => unsafe { run_stage_on(thread, None, overlaps(label), label, |_| stage()) },
        None => stage(),
    }
}

/// Runs `stage` as [`run_stage`] does, and lets it join the calling thread: while `stage` waits
/// in [`MainJoins::join`], the calling thread runs the work the join hands it, with the main
/// thread capability it holds.
///
/// A join is answered by the next reply the stage thread gets, which holds because one thread
/// per process, the one its documents live on, starts the stages.
///
/// Under `LIBWEB_STAGE_THREAD=overlap`, the stage overlaps its caller as in
/// [`run_overlappable_stage`] if `LIBWEB_STAGE_OVERLAP` names `label`.
///
/// # Safety
///
/// The work each join hands the calling thread may capture references to state that is neither
/// `Send` nor `Sync`. The caller must ensure that no thread other than the stage thread can reach
/// that state while the work runs. The stage thread itself cannot, since it waits for the result.
/// Under overlap, the same holds as for [`run_overlappable_stage`].
pub(crate) unsafe fn run_overlappable_stage_with_joins<R: Send>(
    label: &'static str,
    main_thread: &MainThread<'_>,
    stage: impl FnOnce(&MainJoins<'_>) -> R + Send,
) -> R {
    match stage_thread() {
        // SAFETY: Guaranteed by the caller.
        Some(thread) => unsafe { run_stage_on(thread, Some(main_thread), overlaps(label), label, stage) },
        None => stage(&MainJoins(JoinTarget::InPlace(Some(main_thread)))),
    }
}

/// How a stage started by [`run_overlappable_stage_with_joins`] reaches the thread that waits for it.
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
        wake_overlapping_caller();
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
/// As for [`run_overlappable_stage_with_joins`]. A stage that joins needs `main_thread`. With `overlap`, the
/// caller spins its event loop while the stage runs.
unsafe fn run_stage_on<R: Send>(
    thread: &'static StageThread,
    main_thread: Option<&MainThread<'_>>,
    overlap: bool,
    label: &'static str,
    stage: impl FnOnce(&MainJoins<'_>) -> R + Send,
) -> R {
    if std::thread::current().id() == thread.id {
        return stage(&MainJoins(JoinTarget::InPlace(main_thread)));
    }
    // A stage started while another overlaps would queue behind it, and could not answer its joins.
    join_overlapping_stage();

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
        if overlap {
            wake_overlapping_caller();
        }
    });
    // SAFETY: The job borrows from the calling thread's frame. It drops everything it captured
    // before it replies, and this function does not return before the reply arrives.
    let job = unsafe { std::mem::transmute::<Box<dyn FnOnce() + Send + '_>, Job>(job) };
    tsan::release(thread);
    if thread.jobs.send(StageMessage::Run(job)).is_err() {
        // The stage thread only goes away if the process is going away.
        std::process::abort();
    }
    let in_flight = InFlight {
        label,
        thread,
        from_stage,
        main_thread,
        finished: Cell::new(None),
        done: Cell::new(false),
    };
    match OVERLAP_HOST.get().filter(|_| overlap) {
        Some(host) => {
            unsafe extern "C" fn stage_is_done(context: *mut c_void) -> bool {
                // SAFETY: The context is the in-flight record below, which outlives the spin.
                let in_flight = unsafe { &*context.cast::<InFlight<'_>>() };
                while !in_flight.done.get() {
                    match in_flight.from_stage.try_recv() {
                        Ok(message) => in_flight.handle(message),
                        Err(std::sync::mpsc::TryRecvError::Empty) => break,
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => std::process::abort(),
                    }
                }
                in_flight.done.get()
            }
            // SAFETY: The record is cleared again before it goes away; the lifetime is erased only
            // for the thread-local.
            let record = std::ptr::from_ref(&in_flight).cast::<InFlight<'static>>();
            IN_FLIGHT.with(|slot| slot.set(record));
            SPINNING.with(|spinning| spinning.borrow_mut().push(label));
            // SAFETY: The host spins on this thread, and the record outlives the spin.
            unsafe { (host.spin_until)(stage_is_done, std::ptr::from_ref(&in_flight).cast_mut().cast()) };
            // A spin that returns early (its event loop is exiting) still waits for the stage.
            in_flight.wait();
            IN_FLIGHT.with(|slot| slot.set(std::ptr::null()));
            SPINNING.with(|spinning| spinning.borrow_mut().pop());
        }
        None => in_flight.wait(),
    }
    let style_update = in_flight
        .finished
        .take()
        .expect("a finished stage hands back its style update");
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
    unsafe { run_stage_on(tests::test_thread(), None, false, "", |_| stage()) }
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
            run_stage_on(joining_test_thread(), Some(&main_thread), false, "", |joins| {
                state += 1;
                let (joined_on, nested_stage_ran_on) = joins.join(|_| {
                    state *= 10;
                    (
                        std::thread::current().id(),
                        run_stage_on(joining_test_thread(), None, false, "", |_| std::thread::current().id()),
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
            run_stage_on(joining_test_thread(), Some(&main_thread), false, "", |joins| {
                joins.join(|main_thread| {
                    run_stage_on(joining_test_thread(), Some(main_thread), false, "", |joins| {
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
            run_stage_on(joining_test_thread(), Some(&main_thread), false, "", |joins| {
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
