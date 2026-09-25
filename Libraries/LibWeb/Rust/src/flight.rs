/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! One flight: the stages of a rendering update (style, the layout rounds, the recording and the
//! presentation) run one after another as one submitted stage run, under `LIBWEB_STAGE_OVERLAP`
//! naming `flight`. The main thread seals what the stages read when it submits the flight. The
//! flight runs its stages until it has run them all or reaches one that needs the main thread,
//! and ends there: the frame scheduler goes on with the rendering update where the flight ended,
//! on the path a rendering update that submitted that stage on its own would have taken.

use crate::css::style::bridge::StylePassJob;
use crate::layout::update_layout::{LayoutPassJob, LayoutPassTakeBack};
use std::cell::Cell;
use std::ffi::c_void;
use std::sync::{Arc, Mutex};

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
    releases_style_engine: crate::stage_thread::FlightReleasesStyleEngine,
}

/// What a flight's stages left for the main thread, besides where it ended.
#[derive(Default)]
struct FlightRan {
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
            releases_style_engine: Default::default(),
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
            releases_style_engine: Default::default(),
        }
    }

    /// The stages of the flight a test's hold may name.
    fn stage_holds(&self) -> &'static [&'static str] {
        match (self.began, self.paint.is_some()) {
            (FfiFlightStage::Style, _) => &["flight:style"],
            (_, true) => &[
                "flight:layout",
                "flight:laid-out",
                "flight:record",
                "flight:recorded",
                "flight:present",
            ],
            (_, false) => &["flight:layout", "flight:laid-out"],
        }
    }

    /// The furthest stage the flight may run, which the frame in flight holds the document for. A
    /// layout pass's hold covers a recording's.
    fn reach(&self) -> &'static str {
        if self.layout.is_some() { "layout" } else { "style" }
    }

    fn take_back(&self) -> FlightTakeBack {
        FlightTakeBack {
            layout: self.layout.as_ref().map(LayoutPassJob::take_back),
        }
    }

    /// Runs the flight's stages, on the stage thread.
    fn run(mut self) -> (FfiFlightOutcome, FlightRan) {
        let began = self.began;
        let mut reached = began;
        let mut next = began;
        let mut ran = FlightRan::default();
        let mut may_be_presented = false;
        loop {
            let end = match next {
                FfiFlightStage::Style => {
                    crate::stage_thread::hold_before_flight_stage("flight:style");
                    let style = self.style.take().expect("a flight that begins with style has its pass");
                    style.run();
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
                    let round = layout.run();
                    may_be_presented = round.may_be_presented;
                    // What the flight runs after its layout reads nothing of the style engine. The
                    // main thread's writes may go on beside it only if what the round owes the
                    // document thread reaches no node they could change: no tree build or image
                    // to pay for, no rebuild to ask for.
                    if round.may_be_presented {
                        self.releases_style_engine.release();
                    }
                    reached = FfiFlightStage::Rounds;
                    next = FfiFlightStage::PaintPrep;
                    if self.paint.is_none() {
                        Some(FfiFlightEndReason::PaintNotSealed)
                    } else if crate::stage_thread::flight_is_preempted() {
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
                    match unsafe { crate::painting::ffi::paint_in_flight(self.arena as *mut c_void, paint) } {
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
                    unsafe { crate::painting::ffi::present_in_flight(self.arena as *mut c_void, products) };
                    reached = FfiFlightStage::Present;
                    Some(FfiFlightEndReason::Done)
                }
                FfiFlightStage::StyleRenderHalf | FfiFlightStage::Record => Some(FfiFlightEndReason::StageRunsOnMain),
            };
            if let Some(end) = end {
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
}

/// Submits `flight` for the document whose arena is `arena`.
///
/// # Safety
///
/// As for [`crate::stage_thread::submit_stage`]: what each of the flight's stages reaches, the
/// frame in flight owns until the main thread takes it back.
pub(crate) unsafe fn submit(arena: *mut c_void, flight: Flight) {
    let outcome = Arc::new(Mutex::new(None));
    let outcome_of_stage = outcome.clone();
    let reach = flight.reach();
    let stage_holds = flight.stage_holds();
    let releases_style_engine = flight.releases_style_engine.clone();
    let take_back = flight.take_back();
    // SAFETY: Guaranteed by the caller.
    unsafe {
        crate::stage_thread::submit_flight(
            reach,
            stage_holds,
            &releases_style_engine,
            arena,
            move || {
                let ran = flight.run();
                *outcome_of_stage.lock().expect("a flight that ran left its outcome") =
                    Some(crate::stage_thread::FrameOwns::new(ran));
            },
            move || {
                let (mut outcome, ran) = outcome
                    .lock()
                    .expect("a flight that ran left its outcome")
                    .take()
                    .expect("a flight is taken back once it has run")
                    .into_inner();
                take_back.finish(&mut outcome);
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
