/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The thread the render pipeline's sealed stages run on.
//!
//! With `LIBWEB_STAGE_THREAD=lockstep`, the tree build walk, the layout stage and the display list
//! recording stage run on one thread of their own while the thread that called them waits for the
//! result. Nothing runs concurrently, so the stages see exactly the state they would have seen on
//! the calling thread, but everything they depend on that belongs to a thread (thread-local state,
//! thread-bound handles, stack assumptions) is exercised the way a render thread will exercise it.
//! Without the variable, stages run on the calling thread.
//!
//! There is one stage thread per process. A WebContent process runs every document it hosts on its
//! one main thread, so a thread per process is also a thread per event loop.

use std::any::Any;
use std::cell::Cell;
use std::panic::AssertUnwindSafe;
use std::sync::OnceLock;
use std::sync::mpsc::{Sender, channel};
use std::thread::ThreadId;

type Job = Box<dyn FnOnce() + Send>;

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

    let (reply, result) = channel::<Result<R, Box<dyn Any + Send>>>();
    let stage = CallerWaits(stage);
    let caller = std::thread::current().id();
    let job: Box<dyn FnOnce() + Send + '_> = Box::new(move || {
        WAITING_CALLER.with(|waiting| waiting.set(Some(caller)));
        let outcome = std::panic::catch_unwind(AssertUnwindSafe(stage.into_inner()));
        WAITING_CALLER.with(|waiting| waiting.set(None));
        // The calling thread is waiting on this reply, so it cannot have gone away.
        let _ = reply.send(outcome);
    });
    // SAFETY: The job borrows from the calling thread's frame. It drops everything it captured
    // before it replies, and this function does not return before the reply arrives.
    let job = unsafe { std::mem::transmute::<Box<dyn FnOnce() + Send + '_>, Job>(job) };
    if thread.jobs.send(job).is_err() {
        // The stage thread only goes away if the process is going away.
        std::process::abort();
    }
    match result.recv() {
        Ok(Ok(value)) => value,
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => std::process::abort(),
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
    fn panic_in_stage_reaches_the_caller_and_the_thread_survives() {
        // SAFETY: Nothing is captured.
        let outcome = std::panic::catch_unwind(|| unsafe { run_stage_on(test_thread(), || panic!("stage failed")) });
        let payload = outcome.expect_err("the panic must reach the caller");
        assert_eq!(payload.downcast_ref::<&str>(), Some(&"stage failed"));
        // SAFETY: Nothing is captured.
        assert_eq!(unsafe { run_stage_on(test_thread(), || 7) }, 7);
    }
}
