/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! One flight: the stages of a rendering update (style, the layout rounds, the recording and the
//! presentation) run one after another as one submitted stage run. The main thread seals what the
//! stages read when it submits the flight. The flight runs its stages until it has run them all or reaches one that
//! needs the main thread, and ends there: the frame scheduler goes on with the rendering update
//! where the flight ended, on the path a rendering update that submitted that stage on its own
//! would have taken.

use crate::css::style::StyleEngineHandle;
use crate::css::style::bridge::StylePassJob;
use crate::css::style::engine_home::{Holder, Owed, StyleEngineLoan, StyleEngineSettlement};
use crate::frame_news::{FrameSent, FrameSeq};
use crate::layout::update_layout::{LayoutPassJob, LayoutPassTakeBack};
use crate::render_owner::RenderingUpdate;
use std::cell::Cell;
use std::ffi::c_void;

/// The stages of a flight, in the order it runs them.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum FfiFlightStage {
    /// The style pass.
    Style,
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
}

const FLIGHT_END_REASON_COUNT: usize = FfiFlightEndReason::Preempted as usize + 1;

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
    layout: Option<LayoutPassJob>,
    paint: Option<crate::painting::ffi::FlightPaintSeal>,
}

/// What a flight's stages left for the main thread, besides where it ended.
#[derive(Default)]
pub(crate) struct FlightRan {
    paint: Option<crate::painting::ffi::FlightPaintProducts>,
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
            layout: Some(layout),
            paint: crate::painting::ffi::take_sealed_flight_paint(arena),
        }
    }

    /// The stages of the flight a test's hold may name.
    fn stage_holds(&self) -> &'static [&'static str] {
        match (self.layout.is_some(), self.paint.is_some()) {
            (false, _) => &["flight:style"],
            (true, true) => &[
                "flight:layout",
                "flight:laid-out",
                "flight:record",
                "flight:recorded",
                "flight:present",
            ],
            (true, false) => &["flight:layout", "flight:laid-out"],
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
        Some(match self.layout.is_some() {
            true => style_engine.lend(Holder::LayoutPass, Owed::Nothing),
            false => style_engine.lend(Holder::StylePass, Owed::TakeBack),
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
                    // A flight that begins with style runs nothing after it: the rendering update lays out once it
                    // has taken the flight back.
                    Some(FfiFlightEndReason::StageRunsOnMain)
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
                    // tree build or image to pay for, no rebuild to ask for.
                    let owed = if round.may_be_presented {
                        Owed::Nothing
                    } else {
                        Owed::TakeBack
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
                FfiFlightStage::Record => Some(FfiFlightEndReason::StageRunsOnMain),
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

/// A flight the main thread sent, as it waits for the flight's news: what it runs as it adopts the news.
struct SentFlight {
    seq: FrameSeq,
    began: FfiFlightStage,
    settlement: Option<StyleEngineSettlement>,
    take_back: FlightTakeBack,
}

thread_local! {
    // On the main thread, the flights it sent whose news it has not adopted yet, in the order it sent them.
    static SENT_FLIGHTS: std::cell::RefCell<std::collections::VecDeque<SentFlight>> =
        const { std::cell::RefCell::new(std::collections::VecDeque::new()) };
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
}

/// Submits `flight` for the document whose arena is `arena`. What it leaves comes back as the frame's news.
///
/// # Safety
///
/// As for [`crate::stage_thread::submit_rendering_update`]: what each of the flight's stages reaches, the
/// frame in flight owns until the main thread takes it back.
pub(crate) unsafe fn submit(arena: *mut c_void, flight: Flight) -> FrameSent {
    let reach = flight.reach();
    let stage_holds = flight.stage_holds();
    // SAFETY: Guaranteed by the caller: the document thread still owns the arena.
    let style_engine = unsafe { crate::layout::HostTables::beside_frame(arena) }.style_engine();
    let (loan, settlement) = flight.lend_style_engine(style_engine).unzip();
    let (sent, seq, news) = crate::frame_news::send_frame();
    SENT_FLIGHTS.with_borrow_mut(|flights| {
        flights.push_back(SentFlight {
            seq,
            began: flight.began,
            settlement,
            take_back: flight.take_back(),
        });
    });
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { crate::layout::ArenaHandle::document_of(arena) };
    // SAFETY: Guaranteed by the caller.
    unsafe {
        crate::stage_thread::submit_rendering_update(
            reach,
            stage_holds,
            arena,
            document,
            RenderingUpdate::new(flight, loan, seq, news),
        );
    }
    sent
}

/// Adopts the news that the flight `seq` ended with `outcome`, having left `ran`: ends on the main thread what each
/// of its stages left for it, and keeps where it ended for the frame scheduler.
pub(crate) fn adopt_flight_end(seq: FrameSeq, mut outcome: FfiFlightOutcome, ran: FlightRan) {
    let sent = SENT_FLIGHTS.with_borrow_mut(|flights| {
        let sent = flights.pop_front();
        debug_assert!(
            sent.as_ref().is_some_and(|sent| sent.seq == seq),
            "flight news comes in the order the flights were sent"
        );
        sent
    });
    let Some(sent) = sent else {
        return;
    };
    if let Some(settlement) = sent.settlement {
        settlement.settle();
    }
    sent.take_back.finish(&mut outcome);
    FLIGHT_ENDS.with(|ends| {
        let mut counts = ends.get();
        counts[outcome.end as usize][outcome.reached as usize] += 1;
        ends.set(counts);
    });
    TAKEN_BACK_OUTCOME.with(|taken_back| taken_back.set(Some(outcome)));
    TAKEN_BACK_PAINT.with_borrow_mut(|paint| *paint = ran.paint);
}

/// Ends each flight the main thread took back that posted no news, as its run panicked: where it began, as if it had
/// run nothing, so the rendering update runs the frame from there.
pub(crate) fn end_flights_without_news() {
    while let Some(sent) = SENT_FLIGHTS.with_borrow(|flights| flights.front().map(|sent| (sent.seq, sent.began))) {
        debug_assert!(false, "a flight that was taken back posted its news");
        let (seq, began) = sent;
        adopt_flight_end(
            seq,
            FfiFlightOutcome {
                began,
                reached: began,
                end: FfiFlightEndReason::StageRunsOnMain,
            },
            FlightRan::default(),
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

/// How many flights the main thread took back that ended for `reason` after running up to
/// `reached`.
#[unsafe(no_mangle)]
pub extern "C" fn rust_flight_ends(reason: FfiFlightEndReason, reached: FfiFlightStage) -> u64 {
    FLIGHT_ENDS.with(|ends| ends.get()[reason as usize][reached as usize])
}

/// Forgets how the calling thread's flights ended so far.
#[unsafe(no_mangle)]
pub extern "C" fn rust_reset_flight_ends() {
    FLIGHT_ENDS.with(|ends| ends.set([[0; FLIGHT_STAGE_COUNT]; FLIGHT_END_REASON_COUNT]));
}
