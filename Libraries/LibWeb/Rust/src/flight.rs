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
}

const FLIGHT_END_REASON_COUNT: usize = FfiFlightEndReason::StageRunsOnMain as usize + 1;

/// Where a flight began and ended: the first stage it ran, the last (a flight always runs its
/// first), and why it ran no further.
#[repr(C)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FfiFlightOutcome {
    pub began: FfiFlightStage,
    pub reached: FfiFlightStage,
    pub end: FfiFlightEndReason,
}

/// The stages a flight runs, as the main thread prepared them.
pub(crate) struct Flight {
    began: FfiFlightStage,
    style: Option<StylePassJob>,
    layout: Option<LayoutPassJob>,
}

/// What the main thread runs as it takes a flight back, for the stages the flight ran.
struct FlightTakeBack {
    layout: Option<LayoutPassTakeBack>,
}

impl Flight {
    /// A flight that runs the style pass `style` and goes on from there.
    pub(crate) fn from_style_pass(style: StylePassJob) -> Self {
        Self {
            began: FfiFlightStage::Style,
            style: Some(style),
            layout: None,
        }
    }

    /// A flight that runs the rest of the layout round `layout` has readied, and goes on from there.
    pub(crate) fn from_layout_pass(layout: LayoutPassJob) -> Self {
        Self {
            began: FfiFlightStage::Rounds,
            style: None,
            layout: Some(layout),
        }
    }

    /// The furthest stage the flight may run, which the frame in flight holds the document for.
    fn reach(&self) -> &'static str {
        if self.layout.is_some() { "layout" } else { "style" }
    }

    fn take_back(&self) -> FlightTakeBack {
        FlightTakeBack {
            layout: self.layout.as_ref().map(LayoutPassJob::take_back),
        }
    }

    /// Runs the flight's stages, on the stage thread.
    fn run(mut self) -> FfiFlightOutcome {
        let began = self.began;
        let mut reached = began;
        let mut next = began;
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
                    layout.run();
                    reached = FfiFlightStage::Rounds;
                    next = FfiFlightStage::PaintPrep;
                    None
                }
                FfiFlightStage::StyleRenderHalf
                | FfiFlightStage::PaintPrep
                | FfiFlightStage::Record
                | FfiFlightStage::Present => Some(FfiFlightEndReason::StageRunsOnMain),
            };
            if let Some(end) = end {
                return FfiFlightOutcome { began, reached, end };
            }
        }
    }
}

impl FlightTakeBack {
    /// Ends, on the main thread, what each stage the flight ran left for it, in the order it ran them.
    fn finish(self) {
        if let Some(layout) = self.layout {
            layout.finish();
        }
    }
}

thread_local! {
    // On the main thread, the outcome of the flight it took back last, until the frame scheduler
    // takes it.
    static TAKEN_BACK_OUTCOME: Cell<Option<FfiFlightOutcome>> = const { Cell::new(None) };
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
    let take_back = flight.take_back();
    // SAFETY: Guaranteed by the caller.
    unsafe {
        crate::stage_thread::submit_flight(
            reach,
            arena,
            move || {
                let ran = flight.run();
                *outcome_of_stage.lock().expect("a flight that ran left its outcome") = Some(ran);
            },
            move || {
                take_back.finish();
                let outcome = outcome
                    .lock()
                    .expect("a flight that ran left its outcome")
                    .take()
                    .expect("a flight is taken back once it has run");
                FLIGHT_ENDS.with(|ends| {
                    let mut counts = ends.get();
                    counts[outcome.end as usize][outcome.reached as usize] += 1;
                    ends.set(counts);
                });
                TAKEN_BACK_OUTCOME.with(|taken_back| taken_back.set(Some(outcome)));
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
