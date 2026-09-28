/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Where one document's style engine lives, and the right to reach it.
//!
//! C++ and the layout arena name an engine by a [`StyleEngineHandle`], which points to the engine's
//! home and has no way to the engine but the home's. The engine is at home, or lent to the one
//! submitted stage that holds its [`StyleEngineLoan`] ([`StyleEngineHandle::lend`]), which sends
//! word home once it is done with the engine ([`StyleEngineLoan::send_home`], or the loan's drop).
//! The main thread settles the lend once it has taken the stage back ([`StyleEngineSettlement`]),
//! which keeps the home until then, even where the engine has gone away first.
//!
//! The engine is reached by the render owner, which holds the document's render state
//! ([`StyleEngineHandle::reach_on_owner`], which only the owner's [`crate::render_owner::Owner`] can call), by a stage
//! it is lent to, or by the thread that owns it with its render state ([`OwnedStyleEngine`]). The main thread waits for
//! the engine before what it sends the render owner. With the engine home it goes on at once. With the engine lent it
//! waits for the stage that holds it and nothing else, unless what the stage will still owe the frame it runs in (the
//! install of its style batch, or the frame's take-back) keeps the entrance out: then it takes that frame in, as a
//! forced join does.
//!
//! Whoever reaches the engine may run beside the main thread (a display tick does), so the two share nothing but the
//! home's [`Exchange`], which a lock guards. What the main thread writes to the engine waits there
//! ([`StyleEngineInputHandle::send`]), and whoever reaches the engine next applies it first. Whoever reaches the
//! engine leaves the main thread [`EngineNews`] of what the engine holds as it is done, moved into the exchange. The
//! main thread reads only its own [`HomeAnswers`]: the news it adopted last, with what each change it sent since
//! leaves.

use super::StyleEngine;
use super::bridge::{EngineNews, HomeAnswers};
use super::owner_calls::StyleChange;
use std::cell::{Cell, UnsafeCell};
use std::ffi::c_void;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, TryRecvError, channel};
use std::sync::{Mutex, MutexGuard, PoisonError};

/// What C++ holds for one document's style engine: a pointer to its home, opaque to C++. It has no
/// way to the engine but the home's.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StyleEngineHandle(*mut c_void);

/// What C++ holds to send one document's style engine an input or run its style update: the same
/// home as a [`StyleEngineHandle`], which C++ reads the engine through. Only a style engine C++ may
/// write hands it out, and only the document's render inputs give that out, dropping the query
/// snapshot the document published: every entrance that takes the engine to write takes one of
/// these, and a read's handle does not convert to it.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StyleEngineInputHandle(StyleEngineHandle);

impl StyleEngineInputHandle {
    /// The home the handle names, to enter.
    pub(crate) fn home(self) -> StyleEngineHandle {
        self.0
    }

    /// Leaves `change` in the engine's home, for whoever reaches the engine next to apply first. On the main thread.
    pub(crate) fn send(self, change: StyleChange) {
        // SAFETY: On the main thread, which borrows its answers only here.
        let answers = unsafe { self.0.answers() };
        let leaves = change.leaves(answers.pending);
        answers.follow_sent(&change, leaves);
        self.0.home().exchange().unapplied.push((change, leaves));
    }
}

/// What a style engine holds for its next style transaction, which the main thread reads from the engine's home.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub(crate) struct PendingFacts(u8);

impl PendingFacts {
    pub(crate) const NONE: Self = Self(0);
    /// [`StyleEngine::has_pending_transaction`].
    pub(crate) const TRANSACTION: Self = Self(1);
    /// [`StyleEngine::has_deferred_geometry_transaction`].
    pub(crate) const DEFERRED_GEOMETRY: Self = Self(1 << 1);
    /// [`StyleEngine::has_deferred_element_style_inputs`].
    pub(crate) const DEFERRED_ELEMENT_INPUTS: Self = Self(1 << 2);
    /// [`StyleEngine::may_have_child_dependent_selectors`].
    pub(crate) const CHILD_DEPENDENT_SELECTORS: Self = Self(1 << 3);
    /// What is pending may move layout geometry, as far as the engine tells without looking into its journal:
    /// [`StyleEngine::pending_transaction_may_affect_layout_geometry`] with the journal taken to affect it.
    pub(crate) const MAY_AFFECT_GEOMETRY: Self = Self(1 << 4);
    /// An element's deferred pseudo-element style was made observable, which a change of what is deferred owes an
    /// input.
    pub(crate) const OBSERVABLE_DEFERRED_PSEUDO_ELEMENTS: Self = Self(1 << 5);
    /// [`StyleEngine::has_size_containers_needing_evaluation_after_layout`].
    pub(crate) const SIZE_CONTAINERS_AFTER_LAYOUT: Self = Self(1 << 6);
    /// What an element's style input may leave.
    pub(crate) const ELEMENT_INPUT: Self =
        Self(Self::TRANSACTION.0 | Self::DEFERRED_ELEMENT_INPUTS.0 | Self::MAY_AFFECT_GEOMETRY.0);
    /// What any change may leave, but a deferred geometry transaction, which only a geometry read defers.
    pub(crate) const ANY_CHANGE: Self = Self(Self::ELEMENT_INPUT.0 | Self::CHILD_DEPENDENT_SELECTORS.0);

    pub(crate) const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

/// What the main thread still owes the frame of the stage that sent the engine home, before an
/// entrance may go on.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Owed {
    /// Nothing: the engine is home.
    Nothing,
    /// The install of the style batch the stage's pass published, which changes no published
    /// record: an entrance that only reads one goes on.
    Install,
    /// The frame's take-back.
    TakeBack,
}

/// The kind of stage the engine is lent to, until the frame it runs in is taken back.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Holder {
    /// A style pass alone, which what the host publishes to the engine waits to be drained by.
    StylePass,
    /// A layout pass, on its own or in a flight, or a clock tick, which what the host publishes to
    /// the engine waits for the take-back of.
    LayoutPass,
}

/// Where the engine is: home, owing its stage's frame `owed`, or away.
struct Slot {
    owed: Owed,
    /// While a stage holds the engine: where it sends what it owes once it is done with the engine,
    /// and the least it will owe then.
    away: Option<(Receiver<Owed>, Owed)>,
}

/// Where one document's style engine lives.
struct StyleEngineHome {
    engine: NonNull<StyleEngine>,
    /// Written on the main thread only, and read on a stage only while the main thread waits for it.
    slot: UnsafeCell<Slot>,
    /// Whom the engine is lent to, until the frame it went into is taken back.
    holder: Cell<Option<Holder>>,
    /// The layout arena of the engine's document, which the stages the engine is lent to run for.
    arena: usize,
    /// The document whose render state's arena links the engine, whose render owner owns the engine.
    document: crate::render_owner::DocumentId,
    /// All that crosses between the main thread and whoever reaches the engine, which may run beside it.
    exchange: Mutex<Exchange>,
    /// What the home answers the main thread with, of what the engine holds. The main thread's alone.
    answers: UnsafeCell<HomeAnswers>,
    /// Whether the thread that owns the engine with its render state (a unit test's, or the replay tool's) reached it
    /// through [`OwnedStyleEngine::engine`] since the home last followed it.
    reached_by_owning_thread: Cell<bool>,
}

/// What the main thread and whoever reaches its engine hand each other.
#[derive(Default)]
struct Exchange {
    /// What the main thread wrote to the engine since it was last reached, in order, each with what it may leave: the
    /// main thread pushes, and whoever reaches the engine next takes them.
    unapplied: Vec<(StyleChange, PendingFacts)>,
    /// What the engine held as whoever reached it last was done, which the main thread has not adopted yet. Whoever
    /// reaches the engine takes it back as it begins, and leaves it again with its own as it is done, so the main
    /// thread never adopts news older than a change it no longer finds unapplied.
    news: Option<EngineNews>,
}

impl StyleEngineHome {
    fn exchange(&self) -> MutexGuard<'_, Exchange> {
        self.exchange.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Takes back the news the main thread has not adopted yet, for a reach of the engine that begins.
    fn take_news(&self) -> EngineNews {
        self.exchange().news.take().unwrap_or_default()
    }

    /// Leaves the main thread `news`, with what `engine` holds now, as a reach of it is done.
    fn leave_news(&self, mut news: EngineNews, engine: &mut StyleEngine) {
        news.gather(engine);
        self.exchange().news = Some(news);
    }

    /// Applies to `engine` what the main thread wrote to it since it was last reached, by whoever reaches it now.
    fn apply_unapplied(&self, engine: &mut StyleEngine) {
        // Taken whole, as applying a change may reach the engine's handle again, and the main thread sends on meanwhile.
        let changes = std::mem::take(&mut self.exchange().unapplied);
        for (change, leaves) in changes {
            change.apply(engine, leaves);
        }
    }

    /// Applies what the main thread wrote to `engine`, and leaves it the news, for an entrance with no end the home
    /// sees.
    fn catch_up(&self, engine: &mut StyleEngine) {
        let mut exchange = self.exchange();
        if exchange.unapplied.is_empty() {
            return;
        }
        let news = exchange.news.take().unwrap_or_default();
        drop(exchange);
        self.apply_unapplied(engine);
        self.leave_news(news, engine);
    }
}

/// What the main thread owes the home of an engine it lent to a stage, once it has taken the stage
/// back: the lend's settlement, which keeps the home until then. On the main thread.
#[must_use = "a lend is settled once its stage is taken back"]
pub(crate) struct StyleEngineSettlement {
    home: Rc<StyleEngineHome>,
}

impl StyleEngineSettlement {
    /// Takes the engine back from the stage it was lent to, which the main thread has taken back.
    pub(crate) fn settle(self) {
        self.home.settle();
    }
}

/// The right to reach one style engine, lent to a stage: the one right to it while the stage holds
/// it. It is `Send`, so a stage can take it along, and not `Sync`, so a shared borrow of it stays on
/// the thread that holds it. The loan's drop sends the engine home, owing what [`Self::send_home`]
/// said, or the frame's take-back.
pub(crate) struct StyleEngineLoan {
    engine: NonNull<StyleEngine>,
    /// The home, which names the engine lent to a thread.
    home: usize,
    to_home: Sender<Owed>,
    owed: Owed,
}

// SAFETY: The loan is the only right to reach its engine, and the engine is `Send` (asserted beside
// the layout arena's link to it): whoever holds the loan reaches the engine alone.
unsafe impl Send for StyleEngineLoan {}

thread_local! {
    // The home of the engine a stage lent the loan it holds to while it runs.
    static LENT_TO_THIS_THREAD: Cell<usize> = const { Cell::new(0) };
}

/// Set while the main thread waits for an engine to come home, for a flight that sends one home
/// owing its take-back to end there.
static MAIN_WAITS_FOR_ARRIVAL: AtomicBool = AtomicBool::new(false);

/// Whether the main thread waits for a stage to send an engine home.
pub(crate) fn main_waits_for_arrival() -> bool {
    MAIN_WAITS_FOR_ARRIVAL.load(Ordering::Acquire)
}

/// Runs `run` with `engine`, whose home is `home`, which whatever `run` calls reaches through the engine's handle or
/// its document's arena too, and leaves the home the [`PendingFacts`] `run` left.
///
/// # Safety
///
/// `home` must be live, and nothing else may reach the engine until this returns.
unsafe fn reach_on_this_thread<T>(home: usize, engine: *mut StyleEngine, run: impl FnOnce(&mut StyleEngine) -> T) -> T {
    struct Restore(usize);
    impl Drop for Restore {
        fn drop(&mut self) {
            LENT_TO_THIS_THREAD.set(self.0);
        }
    }
    /// Leaves the main thread the news of the outermost reach as it ends, or unwinds.
    struct LeaveNews<'a> {
        home: &'a StyleEngineHome,
        engine: *mut StyleEngine,
        news: Option<EngineNews>,
    }
    impl Drop for LeaveNews<'_> {
        fn drop(&mut self) {
            if let Some(news) = self.news.take() {
                // SAFETY: The reach has ended: nothing else borrows the engine.
                self.home.leave_news(news, unsafe { &mut *self.engine });
            }
        }
    }
    let outer = LENT_TO_THIS_THREAD.replace(home);
    // A reach within one of the same engine finds it as the outer one left it.
    let outermost = outer != home;
    let _restore = Restore(outer);
    // SAFETY: Guaranteed by the caller. Of the home, only the engine and the exchange are touched off the main thread.
    let home = unsafe { &*(home as *const StyleEngineHome) };
    let _leave_news = LeaveNews {
        home,
        engine,
        news: outermost.then(|| home.take_news()),
    };
    // SAFETY: Guaranteed by the caller.
    let engine = unsafe { &mut *engine };
    home.apply_unapplied(engine);
    run(engine)
}

impl StyleEngineLoan {
    /// Runs `run` with the engine, which is lent to the calling thread meanwhile: whatever `run`
    /// calls that reaches the engine through its handle or its document's arena reaches it through
    /// this loan.
    pub(crate) fn lend_to_this_thread<T>(&mut self, run: impl FnOnce(&mut StyleEngine) -> T) -> T {
        // SAFETY: The loan is the right to reach the engine.
        unsafe { reach_on_this_thread(self.home, self.engine.as_ptr(), run) }
    }

    /// Sends the engine home: the stage is done with it, and the main thread owes its frame `owed`
    /// before an entrance goes on.
    pub(crate) fn send_home(mut self, owed: Owed) {
        self.owed = owed;
    }
}

impl Drop for StyleEngineLoan {
    fn drop(&mut self) {
        crate::stage_thread::release_handoff();
        // The home keeps its receiver until the engine has arrived.
        let _ = self.to_home.send(self.owed);
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

    /// Takes in the engine a stage sent home, if it has, or waits for it if `wait`. On the main
    /// thread.
    fn take_in_arrival(&self, wait: bool) {
        // SAFETY: On the main thread.
        let slot = unsafe { self.slot() };
        let Some((arrival, _)) = &slot.away else {
            return;
        };
        let received = if wait {
            crate::stage_thread::release_holds_for_style_engine_wait(self.arena);
            MAIN_WAITS_FOR_ARRIVAL.store(true, Ordering::Release);
            let received = arrival.recv().map_err(|_| TryRecvError::Disconnected);
            MAIN_WAITS_FOR_ARRIVAL.store(false, Ordering::Release);
            received
        } else {
            arrival.try_recv()
        };
        let owed = match received {
            Ok(owed) => owed,
            Err(TryRecvError::Empty) => return,
            // A loan's drop sends the engine home, so only a stage that aborts loses it.
            Err(TryRecvError::Disconnected) => {
                debug_assert!(false, "a stage lost the style engine lent to it");
                Owed::TakeBack
            }
        };
        crate::stage_thread::acquire_handoff();
        *slot = Slot { owed, away: None };
    }

    /// Whether the engine is home, and what the main thread owes, or at best will owe, the frame of
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

    /// Brings the engine home for an entrance that does `access`, as the module describes. `file`
    /// and `line` name the entrance for the forced-join log.
    fn bring_home(&self, access: Access, file: &'static str, line: u32) {
        if self.state() == (true, Owed::Nothing) {
            return;
        }
        let mut joined = false;
        loop {
            let (arrived, owed) = self.state();
            if access.goes_on_owing(owed) {
                if arrived {
                    return;
                }
                self.take_in_arrival(true);
                continue;
            }
            if !joined {
                joined = true;
                crate::stage_thread::join_frame_holding_style_engine(self.arena, file, line);
                continue;
            }
            // Nothing could take the frame in, as work a stage joined the main thread for, or a
            // garbage collection, cannot: the entrance only waits for the stage to be done.
            if !arrived {
                self.take_in_arrival(true);
            }
            return;
        }
    }

    /// Takes the engine back from the stage it was lent to, whose frame the main thread has taken
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

    /// Gives `engine` a home and returns the handle that names it, which `arena`, the arena of the render state of the
    /// engine's document, links from here on: an engine is born with its owner, which links the arena as it takes the
    /// change in. The handle holds the home until [`Self::destroy`], and each lend's settlement until it is settled.
    ///
    /// # Safety
    ///
    /// `arena` must be the arena of a live render state, on its document thread.
    pub(crate) unsafe fn create(engine: Box<StyleEngine>, arena: *mut c_void) -> Self {
        // SAFETY: Guaranteed by the caller.
        let document = unsafe { crate::layout::ArenaHandle::document_of(arena) };
        let home = Rc::new(StyleEngineHome {
            engine: NonNull::from(Box::leak(engine)),
            slot: UnsafeCell::new(Slot {
                owed: Owed::Nothing,
                away: None,
            }),
            holder: Cell::new(None),
            arena: arena.addr(),
            document,
            exchange: Mutex::default(),
            answers: UnsafeCell::default(),
            reached_by_owning_thread: Cell::new(false),
        });
        let handle = Self(Rc::into_raw(home).cast_mut().cast());
        crate::render_owner::send_arena_change(
            document,
            crate::render_owner::ArenaChange::LinkStyleEngine(crate::layout::StyleEngineLink::to(handle)),
        );
        handle
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

    /// Takes the engine out of its home, which goes away once every lend of the engine is settled.
    /// The main thread brings the engine home first.
    ///
    /// # Safety
    ///
    /// The handle must come from [`Self::create`], be used on the document thread, and not be used
    /// again.
    pub(crate) unsafe fn destroy(self, entry: &'static str) -> Box<StyleEngine> {
        self.bring_home(entry);
        let home = self.home();
        home.settle();
        // SAFETY: Guaranteed by the caller: this is the handle's hold on the home, from `create`.
        let home = unsafe { Rc::from_raw(self.0.cast::<StyleEngineHome>().cast_const()) };
        // SAFETY: The home owned the engine, which `create` leaked into it, and the engine is home.
        unsafe { Box::from_raw(home.engine.as_ptr()) }
    }

    /// As the render `owner`, in a unit it runs with the render state of the engine's document: runs `run` with the
    /// engine, which whatever `run` calls reaches through the handle too. The owner takes no loan: it holds the
    /// document's render state. The main thread may run meanwhile: it only sends the engine changes and adopts what
    /// the reach leaves it, through the home's exchange.
    ///
    /// # Safety
    ///
    /// The handle must name a live engine of a document whose render state the owner holds.
    pub(crate) unsafe fn reach_on_owner<T>(
        self,
        _owner: &crate::render_owner::Owner,
        run: impl FnOnce(&mut StyleEngine) -> T,
    ) -> T {
        let engine = self.home().engine.as_ptr();
        // SAFETY: Guaranteed by the caller; the owner runs one unit at a time, so nothing else on its thread reaches
        // the engine.
        unsafe { reach_on_this_thread(self.address(), engine, run) }
    }

    /// What the engine holds for its next style transaction, as whoever last reached it left it, with what the main
    /// thread sent it since. On the main thread.
    pub(crate) fn pending_facts(self) -> PendingFacts {
        // SAFETY: On the main thread, which borrows its answers only here.
        unsafe { self.answers() }.pending
    }

    /// What the home answers the main thread with, of what the engine holds, which adopts the news whoever reached the
    /// engine left since.
    ///
    /// # Safety
    ///
    /// On the main thread, with no other borrow of the answers live.
    #[allow(clippy::mut_from_ref)]
    pub(crate) unsafe fn answers<'a>(self) -> &'a mut HomeAnswers {
        let home = self.home();
        if home.reached_by_owning_thread.take() {
            // SAFETY: The owning thread reaches the engine only through `OwnedStyleEngine::engine`, whose borrow has
            // ended.
            home.leave_news(home.take_news(), unsafe { &mut *home.engine.as_ptr() });
        }
        // SAFETY: Guaranteed by the caller.
        let answers = unsafe { &mut *home.answers.get() };
        let mut exchange = home.exchange();
        if let Some(news) = exchange.news.take() {
            // What the main thread sent since the news was left, it follows again over it.
            answers.adopt(news);
            for (change, leaves) in &exchange.unapplied {
                answers.follow_sent(change, *leaves);
            }
        }
        answers
    }

    /// The document whose render state's arena links the engine, whose render owner owns it.
    pub(crate) fn document(self) -> crate::render_owner::DocumentId {
        self.home().document
    }

    /// Lends the engine to a stage of the `holder` kind, which sends it home owing no less than
    /// `owed_at_best`. On the main thread, which brings the engine home first, and settles the lend
    /// with the settlement once it has taken the stage back.
    pub(crate) fn lend(self, holder: Holder, owed_at_best: Owed) -> (StyleEngineLoan, StyleEngineSettlement) {
        let home = self.home();
        debug_assert!(
            home.state() == (true, Owed::Nothing),
            "a style engine is lent to a stage while another holds it"
        );
        home.bring_home(Access::Any, "style engine lend", 0);
        // SAFETY: On the main thread.
        let slot = unsafe { home.slot() };
        let (to_home, arrival) = channel();
        slot.away = Some((arrival, owed_at_best));
        home.holder.set(Some(holder));
        crate::stage_thread::release_handoff();
        let home_pointer = self.0.cast::<StyleEngineHome>().cast_const();
        // SAFETY: The handle names a live home, which `Rc::into_raw` made; the settlement holds it too.
        let home = unsafe {
            Rc::increment_strong_count(home_pointer);
            Rc::from_raw(home_pointer)
        };
        (
            StyleEngineLoan {
                engine: home.engine,
                home: self.address(),
                to_home,
                owed: Owed::TakeBack,
            },
            StyleEngineSettlement { home },
        )
    }

    /// The kind of stage the engine is lent to, until its frame is taken back.
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

    /// The engine of an engine the calling thread owns with its render state ([`OwnedStyleEngine`]: a unit test's, or
    /// the replay tool's), which no owner job runs beside, or on the thread a stage lent it to.
    ///
    /// # Safety
    ///
    /// The handle must name a live engine, and no other borrow of the engine may be live while the
    /// returned one is used.
    unsafe fn enter<'a>(self, entry: &'static str) -> &'a mut StyleEngine {
        let engine = self.home().engine.as_ptr();
        if LENT_TO_THIS_THREAD.get() == self.address() {
            // SAFETY: The stage that holds the loan lent the engine to this thread; guaranteed by the
            // caller.
            return unsafe { &mut *engine };
        }
        self.bring_home(entry);
        // SAFETY: The engine is home; guaranteed by the caller.
        let engine = unsafe { &mut *engine };
        // What the main thread wrote to the engine goes in before anything reaches it.
        self.home().catch_up(engine);
        engine
    }

    /// The engine, for the document's arena that links it, where the arena is reached: in a unit of the render owner,
    /// which reaches or is lent the engine on this thread or runs while the main thread waits for it. The main thread
    /// does not reach an engine through its arena: it sends the owner what it writes, and asks what it reads, unless it
    /// does the owner's work itself, where a test holds the owner's run.
    ///
    /// # Safety
    ///
    /// As for [`Self::enter`].
    pub(crate) unsafe fn reach_linked<'a>(self) -> &'a mut StyleEngine {
        // SAFETY: The unit reaches the engine, and the caller guarantees the rest.
        let engine = unsafe { &mut *self.home().engine.as_ptr() };
        if LENT_TO_THIS_THREAD.get() == self.address() {
            return engine;
        }
        if crate::stage_thread::owner_work_runs_here() {
            // As an entrance of the main thread's own would, it brings the engine home first.
            self.bring_home("owner work on the main thread");
        } else {
            debug_assert!(
                crate::stage_thread::running_inside_stage() || !crate::stage_thread::owner_is_elsewhere(),
                "the main thread reaches a style engine through its arena"
            );
        }
        // A unit the main thread waits for reaches the engine as the main thread would, which brought the engine home
        // before it waited; what the main thread wrote to it goes in first.
        self.home().catch_up(engine);
        engine
    }

    /// Brings the engine home for the main thread, which is about to enter it at `entry`
    /// once it has done what it does before.
    pub(crate) fn bring_home(self, entry: &'static str) {
        self.bring_home_for(Access::Any, entry, 0);
    }

    /// Like [`Self::bring_home`], for a read of what a published record holds only, which goes on while the install of
    /// the stage's batch is still owed.
    pub(crate) fn bring_home_to_read_records(self, entry: &'static str) {
        self.bring_home_for(Access::RecordRead, entry, 0);
    }

    /// Like [`Self::bring_home`], for a C++ call site `file` and `line` name.
    pub(crate) fn bring_home_at(self, file: &'static str, line: u32) {
        self.bring_home_for(Access::Any, file, line);
    }

    fn bring_home_for(self, access: Access, file: &'static str, line: u32) {
        if self.is_null() || crate::stage_thread::no_stage_is_submitted() {
            return;
        }
        if LENT_TO_THIS_THREAD.get() == self.address() || crate::stage_thread::running_inside_stage() {
            return;
        }
        self.home().bring_home(access, file, line);
    }

    /// The engine of an engine that runs only for a replay, which no frame is ever in flight for.
    ///
    /// # Safety
    ///
    /// The handle must name a live engine made by `style_engine_create_for_replay`, and no other
    /// borrow of the engine may be live while the returned one is used.
    pub unsafe fn for_replay<'a>(self) -> &'a mut StyleEngine {
        // SAFETY: Guaranteed by the caller.
        unsafe { self.enter("style replay") }
    }
}

/// A style engine together with the render state of a document of its own, both of which the thread that makes it
/// holds as their document thread: a unit test's, or the style replay tool's. Like a document's, the engine is born
/// with its owner, and goes with it.
pub struct OwnedStyleEngine {
    handle: StyleEngineHandle,
    document: crate::render_owner::DocumentId,
}

impl OwnedStyleEngine {
    pub(crate) fn new(engine: Box<StyleEngine>) -> Self {
        let (document, arena) = crate::render_owner::create_document();
        // SAFETY: The owner just created the arena, which it keeps until the document is destroyed, with this.
        let handle = unsafe { StyleEngineHandle::create(engine, arena) };
        Self { handle, document }
    }

    /// The handle that names the engine, as C++ would hold it.
    pub fn handle(&self) -> StyleEngineHandle {
        self.handle
    }

    /// The handle that names the engine, as C++ holds it to write the engine.
    pub fn input_handle(&self) -> StyleEngineInputHandle {
        StyleEngineInputHandle(self.handle)
    }

    /// The engine, once its owner has applied every change the thread sent it. The home follows what the thread
    /// leaves in it before it next answers.
    pub fn engine(&mut self) -> &mut StyleEngine {
        self.handle.home().reached_by_owning_thread.set(true);
        // SAFETY: The engine is live while this is, and the borrow of this keeps any other out.
        unsafe { self.handle.enter("owned style engine") }
    }
}

impl Drop for OwnedStyleEngine {
    fn drop(&mut self) {
        crate::render_owner::destroy_document(self.document);
        // SAFETY: The handle came from `StyleEngineHandle::create`, and goes with this.
        unsafe { super::bridge::style_engine_destroy(self.handle) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_engine() -> (OwnedStyleEngine, StyleEngineHandle) {
        let engine = OwnedStyleEngine::new(Box::new(StyleEngine::new(
            super::super::memory::DeviceClass::ForegroundDesktop,
        )));
        let handle = engine.handle();
        (engine, handle)
    }

    #[test]
    fn a_lent_engine_comes_home_with_what_its_stage_sends() {
        let (_engine, handle) = test_engine();
        let (loan, settlement) = handle.lend(Holder::LayoutPass, Owed::Nothing);
        assert!(!handle.is_home());
        assert_eq!(handle.holder(), Some(Holder::LayoutPass));
        loan.send_home(Owed::Nothing);
        assert!(handle.is_home());
        // The holder stays until the frame is taken back.
        assert_eq!(handle.holder(), Some(Holder::LayoutPass));
        settlement.settle();
        assert_eq!(handle.holder(), None);
    }

    #[test]
    fn a_dropped_loan_sends_the_engine_home_owing_the_take_back() {
        let (_engine, handle) = test_engine();
        let (loan, settlement) = handle.lend(Holder::StylePass, Owed::TakeBack);
        drop(loan);
        assert!(!handle.is_home());
        assert_eq!(handle.home().state(), (true, Owed::TakeBack));
        settlement.settle();
        assert!(handle.is_home());
    }

    #[test]
    fn record_reads_go_on_while_the_install_is_owed() {
        let (_engine, handle) = test_engine();
        let (loan, settlement) = handle.lend(Holder::LayoutPass, Owed::Install);
        loan.send_home(Owed::Install);
        assert!(Access::RecordRead.goes_on_owing(handle.home().state().1));
        assert!(!Access::Any.goes_on_owing(handle.home().state().1));
        settlement.settle();
    }

    #[test]
    fn a_lend_settles_after_its_engine_has_gone_away() {
        let (engine, handle) = test_engine();
        let (loan, settlement) = handle.lend(Holder::StylePass, Owed::TakeBack);
        loan.send_home(Owed::TakeBack);
        drop(engine);
        // The settlement kept the home, which goes away with it.
        assert_eq!(Rc::strong_count(&settlement.home), 1);
        settlement.settle();
    }

    #[test]
    fn a_stage_reaches_the_engine_through_the_loan_it_lent_to_its_thread() {
        // A handle is not `Send`: a stage reaches its engine through its loan alone. C++ the stage
        // calls may hold the handle all the same, which this stands in for.
        struct HandleInCpp(StyleEngineHandle);
        // SAFETY: The test only uses the handle as C++ would.
        unsafe impl Send for HandleInCpp {}

        let (mut engine, handle) = test_engine();
        let engine_address = std::ptr::from_mut(engine.engine());
        let (mut loan, settlement) = handle.lend(Holder::LayoutPass, Owed::Nothing);
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
        // The main thread's entrance takes the engine in, and goes on.
        // SAFETY: The engine is live and nothing else borrows it.
        let entered = unsafe { handle.enter("test entrance") };
        assert_eq!(std::ptr::from_mut(entered), engine_address);
        settlement.settle();
    }
}
