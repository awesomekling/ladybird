/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! One flight: the stages of a rendering update (style, the layout rounds, the recording and the
//! presentation) run one after another as one submitted stage run, by default or under
//! `LIBWEB_STAGE_OVERLAP` naming `flight`. The main thread seals what the stages read when it
//! submits the flight. The flight runs its stages until it has run them all or reaches one that
//! needs the main thread, and ends there: the frame scheduler goes on with the rendering update
//! where the flight ended, on the path a rendering update that submitted that stage on its own
//! would have taken.

use crate::css::style::StyleEngineHandle;
use crate::css::style::bridge::StylePassJob;
use crate::css::style::engine_home::{Holder, Owed, StyleEngineLoan, StyleEngineSettlement};
use crate::css::style::flight_style_rows::{FLIGHT_STYLE_DECLINE_COUNT, FfiFlightStyleDecline};
use crate::layout::update_layout::{LayoutPassJob, LayoutPassTakeBack};
use crate::render_owner::{FrameEffects, RenderingUpdate};
use std::cell::Cell;
use std::ffi::c_void;

/// The stages of a flight, in the order it runs them.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum FfiFlightStage {
    /// The style pass.
    Style,
    /// The render half of the style pass's drain.
    StyleRenderHalf,
    /// The layout rounds.
    Rounds,
    /// The paint preparation of the recording.
    PaintPrep,
    /// The recording.
    Record,
    /// The presentation.
    Present,
}

const FLIGHT_STAGE_COUNT: usize = FfiFlightStage::Present as usize + 1;

/// Why a flight ended.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FfiFlightEndReason {
    /// It ran every stage.
    Done,
    /// The stage it reached does not run in a flight yet: the main thread runs it.
    StageRunsOnMain,
    /// The main thread sealed no paint for it: the document is painted on the main thread.
    PaintNotSealed,
    /// Its layout round left work for the document thread that a recording would not show.
    RoundLeftWork,
    /// The document's SVG paint resources waited for the main thread.
    SvgPaintResources,
    /// The recording would have painted an SVG-as-image render the main thread had not made.
    VectorImages,
    /// The document had no viewport box to record.
    NoViewport,
    /// Paying the layout round's host halves at the take-back left style or layout work, so its
    /// recording was dropped.
    HostLeftWork,
    /// A forced join waited for it, and it stopped at the end of the stage it ran.
    Preempted,
    /// Its style pass published a batch whose install on the document thread reaches what its
    /// layout reads: the document thread installs it and lays out after it.
    StyleNeedsHost,
}

const FLIGHT_END_REASON_COUNT: usize = FfiFlightEndReason::StyleNeedsHost as usize + 1;

impl From<crate::painting::ffi::FlightPaintStop> for FfiFlightEndReason {
    fn from(stop: crate::painting::ffi::FlightPaintStop) -> Self {
        use crate::painting::ffi::FlightPaintStop;
        match stop {
            FlightPaintStop::SvgPaintResources => Self::SvgPaintResources,
            FlightPaintStop::VectorImages => Self::VectorImages,
            FlightPaintStop::NoViewport => Self::NoViewport,
        }
    }
}

/// Where a flight began and ended: the first stage it ran, the last (a flight always runs its
/// first), and why it ran no further.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FfiFlightOutcome {
    pub began: FfiFlightStage,
    pub reached: FfiFlightStage,
    pub end: FfiFlightEndReason,
}

/// What a flight that recorded prepared of the paint state, for the document to take in.
#[repr(C)]
#[derive(Clone, Copy, Default)]
pub struct FfiFlightPaint {
    /// Whether the flight prepared the paint state at all.
    pub prepared: bool,
    pub visual_context_update: crate::painting::host::FfiVisualContextUpdateOutcome,
    /// Whether the scroll state was refreshed, and the document takes the snapshot it is handed.
    pub scroll_state_refreshed: bool,
}

/// The stages a flight runs, as the main thread prepared them.
pub(crate) struct Flight {
    began: FfiFlightStage,
    arena: usize,
    style: Option<StylePassJob>,
    /// The elements the viewport propagates from, a relayout of which is not a partial one.
    viewport_propagation_sources: Vec<crate::css::style::tree::StyleNodeID>,
    layout: Option<LayoutPassJob>,
    paint: Option<crate::painting::ffi::FlightPaintSeal>,
}

/// What a flight's stages left for the main thread, besides where it ended.
#[derive(Default)]
pub(crate) struct FlightRan {
    paint: Option<crate::painting::ffi::FlightPaintProducts>,
    /// For a flight that ran its layout's style: whether it applied the style's batch to the arena
    /// itself, or why it left it to the document thread.
    style: Option<Result<(), FfiFlightStyleDecline>>,
}

/// What the main thread runs as it takes a flight back, for the stages the flight ran.
struct FlightTakeBack {
    layout: Option<LayoutPassTakeBack>,
}

impl Flight {
    /// A flight that runs the style pass `style` and goes on from there.
    pub(crate) fn from_style_pass(arena: *mut c_void, style: StylePassJob) -> Self {
        Self {
            began: FfiFlightStage::Style,
            arena: arena as usize,
            viewport_propagation_sources: Vec::new(),
            style: Some(style),
            layout: None,
            paint: None,
        }
    }

    /// A flight that runs the rest of the layout round `layout` has readied, and goes on from there:
    /// to record the document after it, if the main thread sealed that paint.
    pub(crate) fn from_layout_pass(arena: *mut c_void, layout: LayoutPassJob) -> Self {
        Self {
            began: FfiFlightStage::Rounds,
            arena: arena as usize,
            style: None,
            viewport_propagation_sources: Vec::new(),
            layout: Some(layout),
            paint: crate::painting::ffi::take_sealed_flight_paint(arena),
        }
    }

    /// A flight that runs the style pass `style` of a layout frame's first round, applies the
    /// batch it publishes to the arena itself where the document thread's install of it would
    /// reach nothing the round reads, and goes on with the rest of the round `layout` has readied,
    /// and from there as a flight from the layout pass does. Where the batch's install reaches
    /// what the round reads, the flight ends after the pass, and the document thread installs the
    /// batch and lays out after it.
    pub(crate) fn from_style_and_layout_pass(
        arena: *mut c_void,
        style: StylePassJob,
        layout: LayoutPassJob,
        viewport_propagation_sources: Vec<crate::css::style::tree::StyleNodeID>,
    ) -> Self {
        Self {
            began: FfiFlightStage::Style,
            arena: arena as usize,
            viewport_propagation_sources,
            style: Some(style),
            layout: Some(layout),
            paint: crate::painting::ffi::take_sealed_flight_paint(arena),
        }
    }

    /// The stages of the flight a test's hold may name.
    fn stage_holds(&self) -> &'static [&'static str] {
        match (self.began, self.layout.is_some(), self.paint.is_some()) {
            (FfiFlightStage::Style, true, true) => &[
                "flight:style",
                "flight:layout",
                "flight:laid-out",
                "flight:record",
                "flight:recorded",
                "flight:present",
            ],
            (FfiFlightStage::Style, true, false) => &["flight:style", "flight:layout", "flight:laid-out"],
            (FfiFlightStage::Style, false, _) => &["flight:style"],
            (_, _, true) => &[
                "flight:layout",
                "flight:laid-out",
                "flight:record",
                "flight:recorded",
                "flight:present",
            ],
            (_, _, false) => &["flight:layout", "flight:laid-out"],
        }
    }

    /// The furthest stage the flight may run, which the frame in flight holds the document for. A
    /// layout pass's hold covers a recording's.
    fn reach(&self) -> &'static str {
        if self.layout.is_some() { "layout" } else { "style" }
    }

    /// Lends the flight its document's style engine `style_engine`: as to a layout
    /// pass, which may send it home as soon as its rounds have run, or to a style pass alone. On the
    /// document thread.
    fn lend_style_engine(&self, style_engine: StyleEngineHandle) -> Option<(StyleEngineLoan, StyleEngineSettlement)> {
        if style_engine.is_null() {
            return None;
        }
        Some(match (self.layout.is_some(), self.began) {
            (true, FfiFlightStage::Style) => style_engine.lend(Holder::LayoutPass, Owed::Install),
            (true, _) => style_engine.lend(Holder::LayoutPass, Owed::Nothing),
            (false, _) => style_engine.lend(Holder::StylePass, Owed::TakeBack),
        })
    }

    fn take_back(&self) -> FlightTakeBack {
        FlightTakeBack {
            layout: self.layout.as_ref().map(LayoutPassJob::take_back),
        }
    }

    /// Runs the flight's stages, on the stage thread, with the style engine lent to it as
    /// `style_engine`. The flight sends the engine home once its layout rounds have run, and the
    /// stages after them reach no style engine.
    pub(crate) fn run(
        mut self,
        owner: &crate::render_owner::Owner,
        mut style_engine: Option<StyleEngineLoan>,
        state: Option<*mut crate::layout::ArenaHandle>,
    ) -> (FfiFlightOutcome, FlightRan) {
        // SAFETY: The frame in flight owns the arena, and a flight of no document's render state names the one it
        // runs in.
        let state = state.unwrap_or_else(|| unsafe {
            crate::layout::ArenaHandle::held_by_waiting_thread(owner, self.arena as *mut c_void)
        });
        let owns_arena = self.layout.is_some();
        if let Some(layout) = self.layout.as_mut() {
            layout.hand_state(state);
        }
        let began = self.began;
        let mut reached = began;
        let mut next = began;
        let mut ran = FlightRan::default();
        let mut may_be_presented = false;
        // SAFETY: The frame in flight owns the arena.
        let document = unsafe { crate::layout::ArenaHandle::document_of(self.arena as *const c_void) };
        // Whether the main thread recalled the flight, to take the frame back where it is. Between its stages, the
        // flight serves what the owner was sent meanwhile that may go before the rest of it: a query waits for one
        // stage of it at most.
        let mut recalled = crate::render_owner::take_recall(document);
        let mut first_stage = true;
        loop {
            if !std::mem::take(&mut first_stage) {
                recalled |= crate::stage_thread::serve_messages_between_units(document);
            }
            let end = match next {
                FfiFlightStage::Style => {
                    crate::stage_thread::hold_before_flight_stage("flight:style");
                    let style = self.style.take().expect("a flight that begins with style has its pass");
                    style.run(
                        style_engine
                            .as_mut()
                            .expect("a flight that runs a style pass holds its engine's token"),
                    );
                    reached = FfiFlightStage::Style;
                    next = FfiFlightStage::StyleRenderHalf;
                    None
                }
                FfiFlightStage::Rounds => {
                    crate::stage_thread::hold_before_flight_stage("flight:layout");
                    let layout = self
                        .layout
                        .take()
                        .expect("a flight that begins with layout has its pass");
                    let round = match style_engine.as_mut() {
                        Some(style_engine) => style_engine.lend_to_this_thread(|_| layout.run()),
                        None => layout.run(),
                    };
                    may_be_presented = round.may_be_presented;
                    // What the flight runs after its layout reads nothing of the style engine, so
                    // it sends the engine home. The main thread's writes may go on beside it only if
                    // what the round owes the document thread reaches no node they could change: no
                    // tree build or image to pay for, no rebuild to ask for. A flight that ran the
                    // round's style still owes the document thread the install of the batch it
                    // published, which lets only reads of the records go on.
                    let owed = match (round.may_be_presented, self.began) {
                        (false, _) => Owed::TakeBack,
                        (true, FfiFlightStage::Style) => Owed::Install,
                        (true, _) => Owed::Nothing,
                    };
                    if let Some(style_engine) = style_engine.take() {
                        style_engine.send_home(owed);
                    }
                    reached = FfiFlightStage::Rounds;
                    next = FfiFlightStage::PaintPrep;
                    if self.paint.is_none() {
                        Some(FfiFlightEndReason::PaintNotSealed)
                    } else if {
                        recalled |= crate::stage_thread::serve_messages_between_units(document);
                        recalled
                    } || (owed == Owed::TakeBack && crate::css::style::engine_home::main_waits_for_arrival())
                    {
                        // The main thread waits for the engine, which comes home owing the
                        // take-back: it takes the flight in next.
                        Some(FfiFlightEndReason::Preempted)
                    } else if !round.may_be_painted {
                        Some(FfiFlightEndReason::RoundLeftWork)
                    } else {
                        None
                    }
                }
                FfiFlightStage::PaintPrep => {
                    crate::stage_thread::hold_before_flight_stage("flight:record");
                    let paint = self.paint.take().expect("a flight that paints has sealed its paint");
                    // SAFETY: The frame in flight owns the arena, as the layout pass before it did.
                    match unsafe { crate::painting::ffi::paint_in_flight(state, paint) } {
                        Ok(products) => {
                            let stopped = products.stopped;
                            let presents = products.presentation.is_some();
                            ran.paint = Some(products);
                            if let Some(stop) = stopped {
                                Some(stop.into())
                            } else {
                                reached = FfiFlightStage::Record;
                                next = FfiFlightStage::Present;
                                // A frame whose layout still owes the document thread work it may
                                // ask for again is presented once the flight is taken back.
                                (!presents || !may_be_presented).then_some(FfiFlightEndReason::StageRunsOnMain)
                            }
                        }
                        Err(stop) => Some(stop.into()),
                    }
                }
                FfiFlightStage::Present => {
                    crate::stage_thread::hold_before_flight_stage("flight:present");
                    let products = ran.paint.as_ref().expect("a flight presents what it recorded");
                    // SAFETY: The frame in flight owns the arena, and the recording is pending in it.
                    unsafe { crate::painting::ffi::present_in_flight(state, products) };
                    reached = FfiFlightStage::Present;
                    Some(FfiFlightEndReason::Done)
                }
                FfiFlightStage::StyleRenderHalf if self.layout.is_some() => {
                    let applied = if recalled {
                        None
                    } else {
                        let style_engine = style_engine
                            .as_mut()
                            .expect("a flight that runs a style pass holds its engine's token");
                        // SAFETY: The frame in flight owns the arena, and the pass has run.
                        Some(
                            style_engine
                                .lend_to_this_thread(|engine| unsafe { self.apply_style_render_half(engine, state) }),
                        )
                    };
                    let applied_it = applied == Some(Ok(()));
                    FLIGHT_STYLE_DECISION.store(
                        if applied_it {
                            STYLE_DECIDED_APPLIED
                        } else {
                            STYLE_DECIDED_LEFT_TO_HOST
                        },
                        std::sync::atomic::Ordering::Release,
                    );
                    if let Some(applied) = applied {
                        ran.style = Some(applied);
                    }
                    if applied_it {
                        let layout = self.layout.as_mut().expect("a flight that lays out has its pass");
                        let style_engine = style_engine
                            .as_mut()
                            .expect("a flight that runs a style pass holds its engine's token");
                        // SAFETY: As above.
                        style_engine.lend_to_this_thread(|_| unsafe { layout.ready_after_style_in_flight() });
                        reached = FfiFlightStage::StyleRenderHalf;
                        next = FfiFlightStage::Rounds;
                        None
                    } else {
                        // The document thread installs the batch, and ends the frame the round
                        // readied, which lays out ahead of what it installs.
                        self.layout.take().expect("a flight that lays out has its pass").park();
                        Some(match applied {
                            None => FfiFlightEndReason::Preempted,
                            Some(_) => FfiFlightEndReason::StyleNeedsHost,
                        })
                    }
                }
                FfiFlightStage::StyleRenderHalf | FfiFlightStage::Record => Some(FfiFlightEndReason::StageRunsOnMain),
            };
            if let Some(end) = end {
                // A style pass alone does not own the arena, which the main thread goes on writing beside it.
                if owns_arena {
                    // SAFETY: The frame in flight owns the arena, and nothing borrows it between stages.
                    unsafe { &mut *state }.arena_mut().publish_rows();
                }
                if reached >= FfiFlightStage::Rounds {
                    crate::stage_thread::hold_before_flight_completion("flight:laid-out");
                }
                if reached >= FfiFlightStage::Record {
                    crate::stage_thread::hold_before_flight_completion("flight:recorded");
                }
                return (FfiFlightOutcome { began, reached, end }, ran);
            }
        }
    }
}

impl Flight {
    /// Applies the batch the flight's style pass published to the arena, ahead of the document
    /// thread's install of it, if that install reaches nothing the rest of the round reads.
    ///
    /// # Safety
    ///
    /// The frame in flight owns the arena of the render state `state`, and the pass has run in
    /// `engine`.
    unsafe fn apply_style_render_half(
        &self,
        engine: &crate::css::style::StyleEngine,
        state: *mut crate::layout::ArenaHandle,
    ) -> Result<(), FfiFlightStyleDecline> {
        if !self
            .layout
            .as_ref()
            .is_some_and(LayoutPassJob::lays_out_the_tree_it_has)
        {
            return Err(FfiFlightStyleDecline::Rebuild);
        }
        let rows = engine.rows_a_flight_applies(&self.viewport_propagation_sources)?;
        // SAFETY: Guaranteed by the caller.
        let arena = unsafe { &*state }.arena();
        arena.apply_flight_style_rows(&rows)
    }
}

impl FlightTakeBack {
    /// Ends, on the main thread, what each stage the flight ran left for it, in the order it ran
    /// them. A recording made after a layout round stands only if the round's host halves leave no
    /// work: otherwise it is dropped, and the flight counts as ended after the round.
    fn finish(self, outcome: &mut FfiFlightOutcome) {
        let Some(layout) = self.layout else {
            return;
        };
        if outcome.reached < FfiFlightStage::Record {
            layout.finish();
            return;
        }
        if layout.finish_after_recording() {
            return;
        }
        // What the flight prepared of the paint state stands, and its recording is published for the
        // paint caches it filled: the document takes both in all the same, and paints again. A frame
        // the flight presented is the compositor's already, and the flight counts as having presented.
        if outcome.reached < FfiFlightStage::Present {
            outcome.reached = FfiFlightStage::Rounds;
        }
        outcome.end = FfiFlightEndReason::HostLeftWork;
    }
}

thread_local! {
    // On the main thread, the outcome of the flight it took back last, until the frame scheduler
    // takes it.
    static TAKEN_BACK_OUTCOME: Cell<Option<FfiFlightOutcome>> = const { Cell::new(None) };
    // On the main thread, what the flight it took back last prepared of the paint state, until the
    // document takes it.
    static TAKEN_BACK_PAINT: std::cell::RefCell<Option<crate::painting::ffi::FlightPaintProducts>> =
        const { std::cell::RefCell::new(None) };
    // On the main thread, how many flights ended where, by end reason and by the last stage they ran.
    static FLIGHT_ENDS: Cell<[[u64; FLIGHT_STAGE_COUNT]; FLIGHT_END_REASON_COUNT]> =
        const { Cell::new([[0; FLIGHT_STAGE_COUNT]; FLIGHT_END_REASON_COUNT]) };
    // On the main thread, how many flights that ran their layout's style applied its batch
    // themselves (the first count), and how many left it to the document thread, by why.
    static FLIGHT_STYLE_ENDS: Cell<[u64; FLIGHT_STYLE_DECLINE_COUNT + 1]> =
        const { Cell::new([0; FLIGHT_STYLE_DECLINE_COUNT + 1]) };
}

// What the flight in flight that runs its layout's style decided about the batch its style pass
// published: nothing yet, applied in the flight, or left to the document thread.
const STYLE_UNDECIDED: u8 = 0;
const STYLE_DECIDED_APPLIED: u8 = 1;
const STYLE_DECIDED_LEFT_TO_HOST: u8 = 2;
static FLIGHT_STYLE_DECISION: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(STYLE_UNDECIDED);

/// Whether the flight in flight that runs its layout's style applies the batch its style pass
/// published itself, and so goes on to lay out and record. Waits (bounded) for the flight to decide,
/// unless a test holds it before it does.
#[unsafe(no_mangle)]
pub extern "C" fn rust_flight_applies_its_style() -> bool {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        match FLIGHT_STYLE_DECISION.load(std::sync::atomic::Ordering::Acquire) {
            STYLE_DECIDED_APPLIED => return true,
            STYLE_DECIDED_LEFT_TO_HOST => return false,
            _ => {}
        }
        if crate::stage_thread::stage_thread_holds_a_run() || std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_micros(100));
    }
}

/// Submits `flight` for the document whose arena is `arena`.
///
/// # Safety
///
/// As for [`crate::stage_thread::submit_stage_with_take_back`]: what each of the flight's stages reaches, the
/// frame in flight owns until the main thread takes it back.
pub(crate) unsafe fn submit(arena: *mut c_void, flight: Flight) {
    let (effects, effects_of_update) = std::sync::mpsc::channel::<crate::stage_thread::FrameOwns<FrameEffects>>();
    let reach = flight.reach();
    let stage_holds = flight.stage_holds();
    // SAFETY: Guaranteed by the caller: the document thread still owns the arena.
    let style_engine = unsafe { crate::layout::HostTables::beside_frame(arena) }.style_engine();
    let (loan, settlement) = flight.lend_style_engine(style_engine).unzip();
    let take_back = flight.take_back();
    let began = flight.began;
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { crate::layout::ArenaHandle::document_of(arena) };
    FLIGHT_STYLE_DECISION.store(STYLE_UNDECIDED, std::sync::atomic::Ordering::Release);
    // SAFETY: Guaranteed by the caller.
    unsafe {
        crate::stage_thread::submit_rendering_update(
            reach,
            stage_holds,
            arena,
            document,
            RenderingUpdate::new(flight, loan, effects),
            move || {
                if let Some(settlement) = settlement {
                    settlement.settle();
                }
                // An update whose handling panicked sent none: the main thread runs the frame from where it began.
                let FrameEffects { mut outcome, ran } = effects_of_update.try_recv().map_or_else(
                    |_| {
                        debug_assert!(false, "a rendering update that was taken back sent its effects");
                        FrameEffects {
                            outcome: FfiFlightOutcome {
                                began,
                                reached: began,
                                end: FfiFlightEndReason::StageRunsOnMain,
                            },
                            ran: FlightRan::default(),
                        }
                    },
                    crate::stage_thread::FrameOwns::into_inner,
                );
                take_back.finish(&mut outcome);
                if let Some(style) = ran.style {
                    FLIGHT_STYLE_ENDS.with(|ends| {
                        let mut counts = ends.get();
                        counts[style.err().map_or(0, |decline| decline as usize + 1)] += 1;
                        ends.set(counts);
                    });
                }
                FLIGHT_ENDS.with(|ends| {
                    let mut counts = ends.get();
                    counts[outcome.end as usize][outcome.reached as usize] += 1;
                    ends.set(counts);
                });
                TAKEN_BACK_OUTCOME.with(|taken_back| taken_back.set(Some(outcome)));
                TAKEN_BACK_PAINT.with_borrow_mut(|paint| *paint = ran.paint);
            },
        );
    }
}

/// Takes the outcome of the flight the main thread took back last. Called once per flight, by the
/// frame scheduler's consume-commit.
#[unsafe(no_mangle)]
pub extern "C" fn rust_flight_take_outcome() -> FfiFlightOutcome {
    TAKEN_BACK_OUTCOME
        .with(Cell::take)
        .expect("a flight was taken back before its consume-commit")
}

/// Takes what the flight the main thread took back last prepared of the paint state, for the
/// document it recorded: hands `publish` the scroll state snapshot, if the flight refreshed it.
///
/// # Safety
///
/// `publish` is called synchronously with `sink` and a view of the snapshot valid only for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_flight_take_paint(
    sink: *mut c_void,
    publish: unsafe extern "C" fn(*mut c_void, *const libgfx_rust::FloatPoint, usize),
) -> FfiFlightPaint {
    let Some(products) = TAKEN_BACK_PAINT.with_borrow_mut(Option::take) else {
        return FfiFlightPaint::default();
    };
    if let Some(snapshot) = &products.scroll_state_snapshot {
        // SAFETY: The C++ sink copies the offsets synchronously.
        unsafe { publish(sink, snapshot.as_ptr(), snapshot.len()) };
    }
    FfiFlightPaint {
        prepared: true,
        visual_context_update: products.visual_context_update,
        scroll_state_refreshed: products.scroll_state_snapshot.is_some(),
    }
}

/// Whether the rendering update submits its style pass, and its layout pass, as a flight.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_submits_flight() -> bool {
    crate::stage_thread::submits_flight()
}

/// How many flights the main thread took back that ended for `reason` after running up to
/// `reached`.
#[unsafe(no_mangle)]
pub extern "C" fn rust_flight_ends(reason: FfiFlightEndReason, reached: FfiFlightStage) -> u64 {
    FLIGHT_ENDS.with(|ends| ends.get()[reason as usize][reached as usize])
}

/// How many flights that ran their layout's style the main thread took back that applied its batch
/// themselves (`decline` below zero), or left it to the document thread for `decline`.
#[unsafe(no_mangle)]
pub extern "C" fn rust_flight_style_ends(decline: i32) -> u64 {
    let index = usize::try_from(decline + 1).unwrap_or(0);
    FLIGHT_STYLE_ENDS.with(|ends| ends.get().get(index).copied().unwrap_or(0))
}

/// Forgets how the calling thread's flights ended so far.
#[unsafe(no_mangle)]
pub extern "C" fn rust_reset_flight_ends() {
    FLIGHT_ENDS.with(|ends| ends.set([[0; FLIGHT_STAGE_COUNT]; FLIGHT_END_REASON_COUNT]));
    FLIGHT_STYLE_ENDS.with(|ends| ends.set([0; FLIGHT_STYLE_DECLINE_COUNT + 1]));
}
