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
//! There is one stage thread per process. A WebContent process runs every document it hosts on its
//! one main thread, so a thread per process is also a thread per event loop.

use crate::css::ffi_stats::{StyleUpdateScope, install_style_update_scope, take_style_update_scope};
use std::any::Any;
use std::cell::Cell;
use std::panic::AssertUnwindSafe;
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::thread::ThreadId;

type Job = Box<dyn FnOnce() + Send>;

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
    jobs: Sender<Job>,
    id: ThreadId,
}

// The size Linux and macOS give a process's main thread, where the stages ran before.
const STAGE_THREAD_STACK_SIZE: usize = 8 * 1024 * 1024;

impl StageThread {
    fn spawn() -> Self {
        let (jobs, incoming) = channel::<Job>();
        let thread = std::thread::Builder::new()
            .name("RenderStages".into())
            .stack_size(STAGE_THREAD_STACK_SIZE)
            .spawn(move || {
                for job in incoming {
                    job();
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
        Some(thread) => unsafe { run_stage_on(thread, stage) },
        None => stage(),
    }
}

/// # Safety
///
/// As for [`run_stage`].
unsafe fn run_stage_on<R: Send>(thread: &StageThread, stage: impl FnOnce() -> R) -> R {
    if std::thread::current().id() == thread.id {
        return stage();
    }

    struct CallerWaits<F>(F);
    // SAFETY: The caller guarantees that nothing `stage` captures is reachable from a third
    // thread, and the calling thread waits below until the job has run and dropped its captures.
    unsafe impl<F> Send for CallerWaits<F> {}
    impl<F> CallerWaits<F> {
        // Taken through a method, so the job captures the wrapper rather than its field.
        fn into_inner(self) -> F {
            self.0
        }
    }

    type Outcome<R> = (Result<R, Box<dyn Any + Send>>, StyleUpdateScope);
    let (reply, result) = channel::<Outcome<R>>();
    let stage = CallerWaits(stage);
    let caller = std::thread::current().id();
    // The stage runs inside whatever style update the caller has open, so it takes that update's
    // state along and hands it back with its result.
    let style_update = take_style_update_scope();
    let job: Box<dyn FnOnce() + Send + '_> = Box::new(move || {
        tsan::acquire(thread);
        WAITING_CALLER.with(|waiting| waiting.set(Some(caller)));
        install_style_update_scope(style_update);
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(stage.into_inner()));
        let style_update = take_style_update_scope();
        WAITING_CALLER.with(|waiting| waiting.set(None));
        tsan::release(thread);
        // The calling thread is waiting on this reply, so it cannot have gone away.
        let _ = reply.send((outcome, style_update));
    });
    // SAFETY: The job borrows from the calling thread's frame. It drops everything it captured
    // before it replies, and this function does not return before the reply arrives.
    let job = unsafe { std::mem::transmute::<Box<dyn FnOnce() + Send + '_>, Job>(job) };
    tsan::release(thread);
    if thread.jobs.send(job).is_err() {
        // The stage thread only goes away if the process is going away.
        std::process::abort();
    }
    let Ok((outcome, style_update)) = result.recv() else {
        std::process::abort();
    };
    tsan::acquire(thread);
    install_style_update_scope(style_update);
    match outcome {
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
    unsafe { run_stage_on(tests::test_thread(), stage) }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn test_thread() -> &'static StageThread {
        static THREAD: OnceLock<StageThread> = OnceLock::new();
        THREAD.get_or_init(StageThread::spawn)
    }

    #[test]
    fn stage_runs_on_the_stage_thread_and_sees_the_callers_state() {
        let state = Cell::new(1);
        // SAFETY: `state` lives on this thread only.
        let (result, ran_on) = unsafe {
            run_stage_on(test_thread(), || {
                state.set(state.get() + 1);
                (state.get() * 10, std::thread::current().id())
            })
        };
        assert_eq!(result, 20);
        assert_eq!(state.get(), 2);
        assert_eq!(ran_on, test_thread().id);
    }

    #[test]
    fn nested_stage_runs_in_place() {
        // SAFETY: Nothing is captured.
        let (outer, inner) = unsafe {
            run_stage_on(test_thread(), || {
                let outer = std::thread::current().id();
                (outer, run_stage_on(test_thread(), || std::thread::current().id()))
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
        unsafe { run_stage_on(test_thread(), || release_utf16_fly_string(0x1230)) };
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
        let outcome = std::panic::catch_unwind(|| unsafe { run_stage_on(test_thread(), || panic!("stage failed")) });
        let payload = outcome.expect_err("the panic must reach the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"stage failed"));
        // SAFETY: Nothing is captured.
        assert_eq!(unsafe { run_stage_on(test_thread(), || 7) }, 7);
    }
}
