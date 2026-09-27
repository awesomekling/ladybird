/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Where one document's style engine lives, and the right to reach it.
//!
//! C++ and the layout arena name an engine by a [`StyleEngineHandle`], which points to the engine's
//! home and has no way to the engine but the home's. The right to reach the engine is its
//! [`StyleEngineToken`], of which there is one per engine. It is at home on the main thread, or
//! with the one submitted stage that took it by value ([`StyleEngineHandle::lend`]) and sends it
//! home once it is done with the engine ([`StyleEngineLoan::send_home`], or the loan's drop).
//!
//! The main thread enters the engine through the home. With the token home it goes on at once.
//! With the token away it waits for the stage that holds it and nothing else, unless what the stage
//! will still owe the frame it runs in (the install of its style batch, or the frame's take-back)
//! keeps the entrance out: then it takes that frame in, as a forced join does.

use super::StyleEngine;
use std::cell::{Cell, UnsafeCell};
use std::ffi::c_void;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};

/// What C++ holds for one document's style engine: a pointer to its home, opaque to C++. It has no
/// way to the engine but the home's.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StyleEngineHandle(*mut c_void);

/// The right to reach one style engine. Exactly one exists per engine: only the engine's home makes
/// it, and it cannot be cloned. It is `Send`, so a stage can take it along, and not `Sync`, so a
/// shared borrow of it stays on the thread that holds it.
pub(crate) struct StyleEngineToken {
    engine: NonNull<StyleEngine>,
    not_sync: PhantomData<Cell<()>>,
}

// SAFETY: The token is the only right to reach its engine, and the engine is `Send` (asserted
// beside the layout arena's link to it): whoever holds the token reaches the engine alone.
unsafe impl Send for StyleEngineToken {}

/// What the main thread still owes the frame of the stage that sent the token home, before an
/// entrance may go on.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Owed {
    /// Nothing: the token is home.
    Nothing,
    /// The install of the style batch the stage's pass published, which changes no published
    /// record: an entrance that only reads one goes on.
    Install,
    /// The frame's take-back.
    TakeBack,
}

/// The kind of stage the token is lent to, until the frame it runs in is taken back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Holder {
    /// A style pass alone, which what the host publishes to the engine waits to be drained by.
    StylePass,
    /// A layout pass, on its own or in a flight, or a clock tick, which what the host publishes to
    /// the engine waits for the take-back of.
    LayoutPass,
    /// The render clock's ticks, while the main thread runs a task.
    ClockLend,
}

/// A token a stage sends home.
struct Arrival {
    token: StyleEngineToken,
    owed: Owed,
}

/// Where the token is: home, owing its stage's frame nothing or `owed`, or away.
struct Slot {
    token: Option<StyleEngineToken>,
    owed: Owed,
    /// While a stage holds the token: where it sends the token home, and the least it will owe then.
    away: Option<(Receiver<Arrival>, Owed)>,
}

/// Where one document's style engine lives.
struct StyleEngineHome {
    engine: NonNull<StyleEngine>,
    /// Written on the main thread only, and read on a stage only while the main thread waits for it.
    slot: UnsafeCell<Slot>,
    /// Whom the token is lent to, until the frame it went into is taken back.
    holder: Cell<Option<Holder>>,
    /// The layout arena of the engine's document, which the stages that take the token run for.
    arena: Cell<usize>,
}

/// A token lent to a stage, which it sends home when it is done with the engine, or when it drops
/// the loan, owing the frame's take-back then.
pub(crate) struct StyleEngineLoan {
    token: Option<StyleEngineToken>,
    /// The home, which names the engine lent to a thread.
    home: usize,
    to_home: Sender<Arrival>,
}

thread_local! {
    // The engine a stage lent the token it holds to while it runs, as its home and the engine.
    static LENT_TO_THIS_THREAD: Cell<(usize, *mut StyleEngine)> = const { Cell::new((0, std::ptr::null_mut())) };
}

/// Set while the main thread waits for a token to come home, for a flight that sends one home
/// owing its take-back to end there.
static MAIN_WAITS_FOR_ARRIVAL: AtomicBool = AtomicBool::new(false);

/// Whether the main thread waits for a stage to send a token home.
pub(crate) fn main_waits_for_arrival() -> bool {
    MAIN_WAITS_FOR_ARRIVAL.load(Ordering::Acquire)
}

impl StyleEngineLoan {
    /// Runs `run` with the engine, which is lent to the calling thread meanwhile: whatever `run`
    /// calls that reaches the engine through its handle or its document's arena reaches it through
    /// this loan.
    pub(crate) fn lend_to_this_thread<T>(&mut self, run: impl FnOnce(&mut StyleEngine) -> T) -> T {
        struct Restore((usize, *mut StyleEngine));
        impl Drop for Restore {
            fn drop(&mut self) {
                LENT_TO_THIS_THREAD.set(self.0);
            }
        }
        let engine = self.token.as_ref().expect("a loan holds its token").engine.as_ptr();
        let _restore = Restore(LENT_TO_THIS_THREAD.replace((self.home, engine)));
        // SAFETY: The loan holds the token, and so the right to reach the engine.
        run(unsafe { &mut *engine })
    }

    /// Sends the token home: the stage is done with the engine, and the main thread owes its frame
    /// `owed` before an entrance goes on.
    pub(crate) fn send_home(mut self, owed: Owed) {
        self.send(owed);
    }

    fn send(&mut self, owed: Owed) {
        let Some(token) = self.token.take() else {
            return;
        };
        crate::stage_thread::release_handoff();
        // The home keeps its receiver until the token has arrived.
        let _ = self.to_home.send(Arrival { token, owed });
    }
}

impl Drop for StyleEngineLoan {
    fn drop(&mut self) {
        self.send(Owed::TakeBack);
    }
}

/// What an entrance does with the engine.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    /// Anything.
    Any,
    /// Only reads what a published record holds.
    RecordRead,
}

impl Access {
    /// Whether the access may go on while the main thread owes the frame `owed`.
    fn goes_on_owing(self, owed: Owed) -> bool {
        match self {
            Self::Any => owed == Owed::Nothing,
            Self::RecordRead => owed <= Owed::Install,
        }
    }
}

impl StyleEngineHome {
    /// # Safety
    ///
    /// On the main thread, or on a stage the main thread waits for, which only reads.
    #[allow(clippy::mut_from_ref)]
    unsafe fn slot(&self) -> &mut Slot {
        // SAFETY: Guaranteed by the caller.
        unsafe { &mut *self.slot.get() }
    }

    /// Takes in the token a stage sent home, if one has, or waits for it if `wait`. On the main
    /// thread.
    fn take_in_arrival(&self, wait: bool) {
        // SAFETY: On the main thread.
        let slot = unsafe { self.slot() };
        let Some((arrival, _)) = &slot.away else {
            return;
        };
        let received = if wait {
            crate::stage_thread::release_holds_for_style_engine_wait(self.arena.get());
            MAIN_WAITS_FOR_ARRIVAL.store(true, Ordering::Release);
            let received = arrival.recv().map_err(|_| TryRecvError::Disconnected);
            MAIN_WAITS_FOR_ARRIVAL.store(false, Ordering::Release);
            received
        } else {
            arrival.try_recv()
        };
        let Arrival { token, owed } = match received {
            Ok(arrival) => arrival,
            Err(TryRecvError::Empty) => return,
            // The stage lost the token, which only a stage that aborts does.
            Err(TryRecvError::Disconnected) => std::process::abort(),
        };
        crate::stage_thread::acquire_handoff();
        *slot = Slot {
            token: Some(token),
            owed,
            away: None,
        };
    }

    /// Whether the token is home, and what the main thread owes, or at best will owe, the frame of
    /// the stage it was lent to, once whatever the stage sent home has been taken in.
    fn state(&self) -> (bool, Owed) {
        self.take_in_arrival(false);
        // SAFETY: On the main thread.
        let slot = unsafe { self.slot() };
        match &slot.away {
            Some((_, owed_at_best)) => (false, *owed_at_best),
            None => (true, slot.owed),
        }
    }

    /// Brings the token home for an entrance that does `access`, as the module describes. `file`,
    /// `line` and `column` name the entrance for the forced-join log.
    fn bring_home(&self, access: Access, file: &'static str, line: u32, column: u32) {
        if self.state() == (true, Owed::Nothing) {
            return;
        }
        // Work a stage the main thread waits for joined it for reaches the engine as that stage does.
        if crate::stage_thread::running_join_work() {
            return;
        }
        let only_wait = crate::stage_thread::style_engine_entrances_only_wait();
        let mut joined = false;
        loop {
            let (arrived, owed) = self.state();
            if arrived && (only_wait || access.goes_on_owing(owed)) {
                return;
            }
            if !arrived && (only_wait || access.goes_on_owing(owed)) {
                self.wait_for_arrival();
                continue;
            }
            if !joined {
                joined = true;
                crate::stage_thread::join_frame_holding_style_engine(self.arena.get(), file, line, column);
                continue;
            }
            // Nothing could take the frame in, as work a stage joined the main thread for, or a
            // garbage collection, cannot: the entrance only waits for the stage to be done.
            if !arrived {
                self.wait_for_arrival();
            }
            return;
        }
    }

    /// Waits for the stage that holds the token to send it home, and takes it in. A lend to the
    /// render clock's ticks sends it home only when it is recalled, so it is recalled first.
    fn wait_for_arrival(&self) {
        if self.holder.get() == Some(Holder::ClockLend) {
            crate::stage_thread::recall_lends_holding_style_engine(self.arena.get());
        }
        self.take_in_arrival(true);
    }

    /// Takes the token back from the stage it was lent to, whose frame the main thread has taken
    /// back. On the main thread.
    fn settle(&self) {
        self.take_in_arrival(true);
        // SAFETY: On the main thread.
        unsafe { self.slot() }.owed = Owed::Nothing;
        self.holder.set(None);
    }
}

impl StyleEngineHandle {
    /// A handle that names no engine.
    pub const fn null() -> Self {
        Self(std::ptr::null_mut())
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    /// Gives `engine` a home, with its token, and returns the handle that names it.
    pub(crate) fn create(engine: Box<StyleEngine>) -> Self {
        Self::with_home(NonNull::from(Box::leak(engine)))
    }

    fn with_home(engine: NonNull<StyleEngine>) -> Self {
        let home = Box::new(StyleEngineHome {
            engine,
            slot: UnsafeCell::new(Slot {
                token: Some(StyleEngineToken {
                    engine,
                    not_sync: PhantomData,
                }),
                owed: Owed::Nothing,
                away: None,
            }),
            holder: Cell::new(None),
            arena: Cell::new(0),
        });
        Self(Box::into_raw(home).cast())
    }

    /// A home for an engine a unit test owns, which the handle names for as long as the engine
    /// lives. The home stays behind.
    #[cfg(test)]
    pub(crate) fn for_test_engine(engine: *mut StyleEngine) -> Self {
        Self::with_home(NonNull::new(engine).expect("a test engine is not null"))
    }

    /// The handle as C++ holds it.
    pub(crate) fn into_ffi(self) -> *mut c_void {
        self.0
    }

    /// The address that identifies the engine.
    pub fn address(self) -> usize {
        self.0 as usize
    }

    fn home<'a>(self) -> &'a StyleEngineHome {
        assert!(!self.is_null(), "style engine handle is null");
        // SAFETY: A handle that is not null names a live home.
        unsafe { &*self.0.cast::<StyleEngineHome>() }
    }

    /// Takes the engine out of its home, which goes away. The main thread brings the token home
    /// first.
    ///
    /// # Safety
    ///
    /// The handle must come from [`Self::create`], be used on the document thread, and not be used
    /// again.
    pub(crate) unsafe fn destroy(self, entry: &'static str) -> Box<StyleEngine> {
        self.bring_home(entry);
        let home = self.home();
        home.settle();
        // SAFETY: Guaranteed by the caller.
        let home = unsafe { Box::from_raw(self.0.cast::<StyleEngineHome>()) };
        // SAFETY: The home owned the engine, which `create` leaked into it, and its token is home.
        unsafe { Box::from_raw(home.engine.as_ptr()) }
    }

    /// Names the layout arena of the engine's document, which the stages that take the token run for.
    pub(crate) fn link_arena(self, arena: usize) {
        self.home().arena.set(arena);
    }

    /// Lends the token to a stage of the `holder` kind, which sends it home owing no less than
    /// `owed_at_best`. On the main thread, which brings the token home first. [`Self::settle`] takes
    /// it back once the main thread has taken the stage back.
    pub(crate) fn lend(self, holder: Holder, owed_at_best: Owed) -> StyleEngineLoan {
        let home = self.home();
        debug_assert!(
            home.state() == (true, Owed::Nothing),
            "a style engine is lent to a stage while another holds it"
        );
        home.bring_home(Access::Any, "style engine lend", 0, 0);
        // SAFETY: On the main thread.
        let slot = unsafe { home.slot() };
        let (to_home, arrival) = channel();
        let token = slot.token.take().expect("the token is home");
        slot.away = Some((arrival, owed_at_best));
        home.holder.set(Some(holder));
        crate::stage_thread::release_handoff();
        StyleEngineLoan {
            token: Some(token),
            home: self.address(),
            to_home,
        }
    }

    /// Takes the token back from the stage it was lent to, which the main thread has taken back.
    pub(crate) fn settle(self) {
        self.home().settle();
    }

    /// The kind of stage the token is lent to, until its frame is taken back.
    pub(crate) fn holder(self) -> Option<Holder> {
        if self.is_null() {
            return None;
        }
        self.home().holder.get()
    }

    /// Whether the main thread may write the engine now, without waiting for a stage.
    pub(crate) fn is_home(self) -> bool {
        self.is_null() || self.home().state() == (true, Owed::Nothing)
    }

    /// The engine, for whoever holds the token: a stage it is lent to, on the thread the stage lent
    /// it to, or the main thread, which brings the token home first for an entrance at `entry`.
    ///
    /// # Safety
    ///
    /// The handle must name a live engine, and no other borrow of the engine may be live while the
    /// returned one is used.
    pub(crate) unsafe fn enter<'a>(self, entry: &'static str) -> &'a mut StyleEngine {
        // SAFETY: Guaranteed by the caller.
        unsafe { self.enter_for(Access::Any, entry) }
    }

    /// Like [`Self::enter`], for an entrance that only reads what a published record holds, which
    /// the record keeps as it is whatever else the engine does.
    ///
    /// # Safety
    ///
    /// As for [`Self::enter`].
    pub(crate) unsafe fn enter_to_read_records<'a>(self, entry: &'static str) -> &'a StyleEngine {
        // SAFETY: Guaranteed by the caller.
        unsafe { self.enter_for(Access::RecordRead, entry) }
    }

    /// # Safety
    ///
    /// As for [`Self::enter`].
    unsafe fn enter_for<'a>(self, access: Access, entry: &'static str) -> &'a mut StyleEngine {
        let home = self.home();
        // A token is lent only to a submitted stage, so with none submitted every token is home.
        if crate::stage_thread::no_stage_is_submitted() {
            // SAFETY: Guaranteed by the caller.
            return unsafe { &mut *home.engine.as_ptr() };
        }
        let (lent_home, lent_engine) = LENT_TO_THIS_THREAD.get();
        if lent_home == self.address() {
            // SAFETY: The stage that holds the token lent it to this thread; guaranteed by the caller.
            return unsafe { &mut *lent_engine };
        }
        if crate::stage_thread::running_inside_stage() {
            debug_assert!(
                !crate::stage_thread::running_submitted_stage(),
                "a submitted stage reaches a style engine it holds no token for"
            );
            // A stage the main thread waits for reaches the engine as the main thread would, which
            // brought the token home before it waited.
            // SAFETY: Guaranteed by the caller.
            return unsafe { &mut *home.engine.as_ptr() };
        }
        home.bring_home(access, entry, 0, 0);
        // SAFETY: The token is home, or the stage that holds it is done with the engine as far as
        // `access` reaches; guaranteed by the caller.
        unsafe { &mut *home.engine.as_ptr() }
    }

    /// Brings the token home for the main thread, which is about to enter the engine at `entry`
    /// once it has done what it does before.
    pub(crate) fn bring_home(self, entry: &'static str) {
        self.bring_home_at(entry, 0, 0);
    }

    /// Like [`Self::bring_home`], for a C++ call site `file` and `line` name (`column` 0 where it
    /// has none).
    pub(crate) fn bring_home_at(self, file: &'static str, line: u32, column: u32) {
        if self.is_null() || crate::stage_thread::no_stage_is_submitted() {
            return;
        }
        let (lent_home, _) = LENT_TO_THIS_THREAD.get();
        if lent_home == self.address() || crate::stage_thread::running_inside_stage() {
            return;
        }
        self.home().bring_home(Access::Any, file, line, column);
    }

    /// The engine of an engine that runs only for a replay, which no frame is ever in flight for.
    ///
    /// # Safety
    ///
    /// The handle must name a live engine made by `style_engine_create_for_replay`, and no other
    /// borrow of the engine may be live while the returned one is used.
    pub unsafe fn for_replay<'a>(self) -> &'a mut StyleEngine {
        // SAFETY: Guaranteed by the caller.
        unsafe { &mut *self.home().engine.as_ptr() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> (Box<StyleEngine>, StyleEngineHandle) {
        let mut engine = Box::new(StyleEngine::new(super::super::memory::DeviceClass::ForegroundDesktop));
        let handle = StyleEngineHandle::for_test_engine(&raw mut *engine);
        (engine, handle)
    }

    #[test]
    fn a_lent_token_comes_home_with_what_its_stage_sends() {
        let (_engine, handle) = test_engine();
        let loan = handle.lend(Holder::LayoutPass, Owed::Nothing);
        assert!(!handle.is_home());
        assert_eq!(handle.holder(), Some(Holder::LayoutPass));
        loan.send_home(Owed::Nothing);
        assert!(handle.is_home());
        // The holder stays until the frame is taken back.
        assert_eq!(handle.holder(), Some(Holder::LayoutPass));
        handle.settle();
        assert_eq!(handle.holder(), None);
    }

    #[test]
    fn a_dropped_loan_sends_the_token_home_owing_the_take_back() {
        let (_engine, handle) = test_engine();
        let loan = handle.lend(Holder::StylePass, Owed::TakeBack);
        drop(loan);
        assert!(!handle.is_home());
        assert_eq!(handle.home().state(), (true, Owed::TakeBack));
        handle.settle();
        assert!(handle.is_home());
    }

    #[test]
    fn record_reads_go_on_while_the_install_is_owed() {
        let (_engine, handle) = test_engine();
        let loan = handle.lend(Holder::LayoutPass, Owed::Install);
        loan.send_home(Owed::Install);
        assert!(Access::RecordRead.goes_on_owing(handle.home().state().1));
        assert!(!Access::Any.goes_on_owing(handle.home().state().1));
        handle.settle();
    }

    #[test]
    fn an_entrance_that_only_waits_recalls_a_clock_lend_that_holds_the_token() {
        use std::cell::RefCell;
        use std::rc::Rc;

        let (_engine, handle) = test_engine();
        let arena = std::ptr::NonNull::<c_void>::dangling().as_ptr();
        handle.link_arena(arena as usize);
        let loan = Rc::new(RefCell::new(Some(handle.lend(Holder::ClockLend, Owed::TakeBack))));
        let taken_back = Rc::new(Cell::new(false));
        // SAFETY: Nothing reaches the arena, which the lend only names.
        unsafe {
            crate::stage_thread::lend_arena(
                arena,
                move || {
                    drop(loan.borrow_mut().take());
                    handle.settle();
                },
                {
                    let taken_back = taken_back.clone();
                    move || taken_back.set(true)
                },
            );
        }
        // A garbage collection's finalizer enters the engine, which only waits for the token.
        crate::stage_thread::rust_stage_thread_begin_style_engine_entrances_that_only_wait();
        // SAFETY: The engine is live and nothing else borrows it.
        let _ = unsafe { handle.enter("finalizer") };
        crate::stage_thread::rust_stage_thread_end_style_engine_entrances_that_only_wait();
        assert!(handle.is_home());
        assert_eq!(handle.holder(), None);
        // What follows the take-back waits for whatever takes the lend back.
        assert!(!taken_back.get());
        assert!(crate::stage_thread::has_lent_arena());
        crate::stage_thread::take_lent_arenas();
        assert!(!crate::stage_thread::has_lent_arena());
    }

    #[test]
    fn a_stage_reaches_the_engine_through_the_loan_it_lent_to_its_thread() {
        // A handle is not `Send`: a stage reaches its engine through its token alone. C++ the stage
        // calls may hold the handle all the same, which this stands in for.
        struct HandleInCpp(StyleEngineHandle);
        // SAFETY: The test only uses the handle as C++ would.
        unsafe impl Send for HandleInCpp {}

        let (mut engine, handle) = test_engine();
        let engine_address = &raw mut *engine;
        let mut loan = handle.lend(Holder::LayoutPass, Owed::Nothing);
        let handle_in_cpp = HandleInCpp(handle);
        let reached = std::thread::spawn(move || {
            let handle_in_cpp = handle_in_cpp;
            let reached = loan.lend_to_this_thread(|_| {
                // SAFETY: The loan is lent to this thread.
                std::ptr::from_mut(unsafe { handle_in_cpp.0.enter("test entrance") }) as usize
            });
            loan.send_home(Owed::Nothing);
            reached
        })
        .join()
        .unwrap();
        assert_eq!(reached, engine_address as usize);
        // The main thread's entrance takes the token in, and goes on.
        // SAFETY: The engine is live and nothing else borrows it.
        let entered = unsafe { handle.enter("test entrance") };
        assert_eq!(std::ptr::from_mut(entered), engine_address);
        handle.settle();
    }
}
