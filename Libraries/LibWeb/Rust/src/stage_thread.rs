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
//! A stage run can also join its caller: [`run_stage_with_joins`] hands the stage a [`MainJoins`],
//! through which it runs a piece of main-thread work on the waiting caller and continues with the
//! result. The caller runs only the work it is handed, and the stage thread runs only the stages
//! the caller starts from inside that work, so the two still take turns.
//!
//! There is one stage thread per process. A WebContent process runs every document it hosts on its
//! one main thread, so a thread per process is also a thread per event loop.

use crate::css::ffi_stats::{StyleUpdateScope, install_style_update_scope, take_style_update_scope};
use crate::stage::MainThread;
use std::any::Any;
use std::cell::{Cell, RefCell};
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
        .get_or_init(|| {
            std::env::var_os("LIBWEB_STAGE_THREAD")
                .is_some_and(|mode| mode == "lockstep")
                .then(StageThread::spawn)
        })
        .as_ref()
}

/// Runs `stage` on the stage thread if there is one, and returns its result once it has finished;
/// the calling thread waits meanwhile. Without a stage thread, or when called from the stage thread
/// itself, `stage` runs right here.
///
/// A panic in `stage` continues on the calling thread, as it would have if `stage` had run there;
/// a build that aborts on panic aborts on the stage thread instead.
///
/// # Safety
///
/// `stage` may capture references to state that is neither `Send` nor `Sync`. The caller must
/// ensure that no thread other than the calling thread can reach that state while `stage` runs.
/// The calling thread itself cannot, since it waits for the result.
pub(crate) unsafe fn run_stage<R: Send>(stage: impl FnOnce() -> R) -> R {
    match stage_thread() {
        // SAFETY: Guaranteed by the caller.
        Some(thread) => unsafe { run_stage_on(thread, None, |_| stage()) },
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
/// # Safety
///
/// As for [`run_stage`], and the same holds for the work each join hands the calling thread.
pub(crate) unsafe fn run_stage_with_joins<R: Send>(
    main_thread: &MainThread<'_>,
    stage: impl FnOnce(&MainJoins<'_>) -> R,
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
        tsan::release(thread);
        if caller
            .send(CallerMessage::Join(work, take_style_update_scope()))
            .is_err()
        {
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

struct CallerWaits<F>(F);
// SAFETY: Whoever wraps a value vouches that nothing it holds is reachable from a third thread,
// and the thread it came from waits until the value has been used and dropped.
unsafe impl<F> Send for CallerWaits<F> {}
impl<F> CallerWaits<F> {
    // Taken through a method, so a closure captures the wrapper rather than its field.
    fn into_inner(self) -> F {
        self.0
    }
}

/// # Safety
///
/// As for [`run_stage_with_joins`]. A stage that joins needs `main_thread`.
unsafe fn run_stage_on<R: Send>(
    thread: &'static StageThread,
    main_thread: Option<&MainThread<'_>>,
    stage: impl FnOnce(&MainJoins<'_>) -> R,
) -> R {
    if std::thread::current().id() == thread.id {
        return stage(&MainJoins(JoinTarget::InPlace(main_thread)));
    }

    let (to_caller, from_stage) = channel::<CallerMessage>();
    let mut outcome: Option<Result<R, Box<dyn Any + Send>>> = None;
    let slot = &mut outcome;
    let stage = CallerWaits(stage);
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
        *slot = Some(std::panic::catch_unwind(AssertUnwindSafe(|| {
            stage.into_inner()(&joins)
        })));
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
    let style_update = loop {
        match from_stage.recv() {
            Ok(CallerMessage::Finished(style_update)) => break style_update,
            Ok(CallerMessage::Join(work, style_update)) => {
                tsan::acquire(thread);
                install_style_update_scope(style_update);
                let main_thread = main_thread.expect("only a stage started with joins joins its caller");
                let outcome = work(main_thread);
                let style_update = take_style_update_scope();
                tsan::release(thread);
                if thread
                    .jobs
                    .send(StageMessage::JoinFinished(Box::new(style_update), outcome))
                    .is_err()
                {
                    std::process::abort();
                }
            }
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
///
/// # Safety
///
/// As for [`run_stage`].
#[cfg(test)]
pub(crate) unsafe fn run_stage_for_test<R: Send>(stage: impl FnOnce() -> R) -> R {
    // SAFETY: Guaranteed by the caller.
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
        let state = Cell::new(1);
        // SAFETY: `state` lives on this thread only.
        let (result, ran_on) = unsafe {
            run_stage_for_test(|| {
                state.set(state.get() + 1);
                (state.get() * 10, std::thread::current().id())
            })
        };
        assert_eq!(result, 20);
        assert_eq!(state.get(), 2);
        assert_eq!(ran_on, test_thread().id);
    }

    #[test]
    fn a_join_runs_on_the_caller_and_the_stages_it_starts_run_on_the_stage_thread() {
        let main_thread = crate::stage::MainThread::for_test();
        let caller = std::thread::current().id();
        let state = Cell::new(1);
        // SAFETY: `state` lives on this thread only.
        let (stage_thread, joined_on, nested_stage_ran_on) = unsafe {
            run_stage_on(joining_test_thread(), Some(&main_thread), |joins| {
                state.set(state.get() + 1);
                let (joined_on, nested_stage_ran_on) = joins.join(|_| {
                    state.set(state.get() * 10);
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
        assert_eq!(state.get(), 20);
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
        // SAFETY: Nothing is captured.
        assert_eq!(unsafe { run_stage_for_test(|| 7) }, 7);
    }

    #[test]
    fn nested_stage_runs_in_place() {
        // SAFETY: Nothing is captured.
        let (outer, inner) = unsafe {
            run_stage_for_test(|| {
                let outer = std::thread::current().id();
                (outer, run_stage_for_test(|| std::thread::current().id()))
            })
        };
        assert_eq!(outer, test_thread().id);
        assert_eq!(inner, outer);
    }

    #[test]
    fn a_stage_run_defers_releases_into_the_callers_style_update() {
        use crate::css::ffi_stats::*;
        rust_style_ffi_complete_style_update_begin();
        // SAFETY: Nothing is captured.
        unsafe { run_stage_for_test(|| release_utf16_fly_string(0x1230)) };
        let releases = rust_style_ffi_complete_style_update_end();
        // SAFETY: The view stays valid until the releases are cleared.
        let released = unsafe { std::slice::from_raw_parts(releases.fly_strings, releases.fly_string_count) };
        // Tests run in parallel, and their drops join any update that is open at the time.
        assert!(released.contains(&0x1230));
        rust_deferred_cpp_releases_clear();
    }

    #[test]
    fn panic_in_stage_reaches_the_caller_and_the_thread_survives() {
        // SAFETY: Nothing is captured.
        let outcome = std::panic::catch_unwind(|| unsafe { run_stage_for_test(|| panic!("stage failed")) });
        let payload = outcome.expect_err("the panic must reach the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"stage failed"));
        // SAFETY: Nothing is captured.
        assert_eq!(unsafe { run_stage_for_test(|| 7) }, 7);
    }
}
