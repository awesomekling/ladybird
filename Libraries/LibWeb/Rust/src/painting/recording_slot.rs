/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! What a document keeps of its display list recordings from one to the next, and the ticket a
//! submitted recording answers on.
//!
//! A submitted recording owns the recorder state it records with until it answers, and the frame's
//! presentation publishes the answer to the host without the document. The document reads what a
//! recording left only through [`TakenIn`], which [`LayoutNodeArena::recording`] makes after taking
//! the answer in, so no read finds the recorder state missing or a publication half applied.

use crate::layout::LayoutNodeArena;
use crate::painting::paint_state::{PendingRecording, PendingRecordingTrace};
use crate::painting::record::RecordingOutput;
use crate::painting::record::recorder_state::RecorderState;
use std::cell::RefMut;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};

/// What a recording answers once it has run: the recorder state it was handed, and what it
/// recorded.
pub(crate) struct RecordingAnswer {
    pub(crate) recorder: RecorderState,
    pub(crate) pending: PendingRecording,
    pub(crate) trace: Option<PendingRecordingTrace>,
}

/// What a frame's presentation made of a recording's answer: the output it handed the host, for the
/// document to take in.
pub(crate) struct PresentedRecording {
    pub(crate) recorder: RecorderState,
    pub(crate) output: RecordingOutput,
    pub(crate) publishes_recording: bool,
    pub(crate) trace: Option<PendingRecordingTrace>,
}

enum TicketState {
    Recording,
    Answered(Box<RecordingAnswer>),
    Presenting,
    Presented(Box<PresentedRecording>),
    TakenIn,
}

/// Where a submitted recording answers, and where the frame's presentation leaves what it
/// published of the answer.
pub(crate) struct RecordingTicket {
    state: Mutex<TicketState>,
    changed: Condvar,
    // Whether the frame's presentation publishes the answer, which the document then waits for.
    presented_by_frame: AtomicBool,
}

impl RecordingTicket {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(TicketState::Recording),
            changed: Condvar::new(),
            presented_by_frame: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> MutexGuard<'_, TicketState> {
        self.state
            .lock()
            .expect("a recording ticket is not held across a panic")
    }

    fn set(&self, state: TicketState) {
        *self.lock() = state;
        self.changed.notify_all();
    }

    /// On the recording's stage, once it has run.
    pub(crate) fn answer(&self, answer: RecordingAnswer) {
        self.set(TicketState::Answered(Box::new(answer)));
    }

    /// On the document thread, as the frame's presentation is submitted: it publishes the answer.
    pub(crate) fn will_be_presented(&self) {
        self.presented_by_frame.store(true, Ordering::Relaxed);
    }

    fn wait_for_answer(&self) -> MutexGuard<'_, TicketState> {
        let mut state = self.lock();
        while matches!(*state, TicketState::Recording) {
            state = self
                .changed
                .wait(state)
                .expect("a recording ticket is not held across a panic");
        }
        state
    }

    /// On the presentation stage: presents the recording's answer with `present`, which makes its
    /// output from what it recorded and the recorder state it recorded with, and leaves that for the
    /// document to take in. Returns nothing if the recording has nothing to present.
    pub(crate) fn present<R>(
        &self,
        present: impl FnOnce(PendingRecording, &RecorderState) -> (RecordingOutput, R),
    ) -> Option<R> {
        let answer = {
            let mut state = self.wait_for_answer();
            if !matches!(&*state, TicketState::Answered(_)) {
                return None;
            }
            let TicketState::Answered(answer) = std::mem::replace(&mut *state, TicketState::Presenting) else {
                unreachable!()
            };
            answer
        };
        let RecordingAnswer {
            recorder,
            pending,
            trace,
        } = *answer;
        let publishes_recording = pending.publishes_recording;
        let (output, result) = present(pending, &recorder);
        self.set(TicketState::Presented(Box::new(PresentedRecording {
            recorder,
            output,
            publishes_recording,
            trace,
        })));
        Some(result)
    }

    /// Takes what the recording left for the document, once it is all it will leave: waiting for it
    /// with `wait`, or returning nothing if it is not yet.
    fn take(&self, wait: bool) -> Option<TicketState> {
        let mut state = self.lock();
        loop {
            let is_final = match &*state {
                TicketState::Recording | TicketState::Presenting => false,
                TicketState::Answered(_) => !self.presented_by_frame.load(Ordering::Relaxed),
                TicketState::Presented(_) => true,
                TicketState::TakenIn => unreachable!("a document takes a recording in once"),
            };
            if is_final {
                return Some(std::mem::replace(&mut *state, TicketState::TakenIn));
            }
            if !wait {
                return None;
            }
            state = self
                .changed
                .wait(state)
                .expect("a recording ticket is not held across a panic");
        }
    }
}

/// What a document keeps of its recordings: the recording its host is to publish, and the recorder
/// state the next recording records with. A submitted recording holds the recorder state until it
/// answers.
#[derive(Default)]
pub(crate) struct RecordingSlot {
    pending_recording: Option<PendingRecording>,
    pending_recording_trace: Option<PendingRecordingTrace>,
    recorder: RecorderState,
    in_flight: Option<Arc<RecordingTicket>>,
}

/// The document's recording slot, with the recording in flight taken in.
pub(crate) struct TakenIn<'a>(RefMut<'a, RecordingSlot>);

impl TakenIn<'_> {
    pub(crate) fn pending_recording(&mut self) -> &mut Option<PendingRecording> {
        &mut self.0.pending_recording
    }

    pub(crate) fn pending_recording_trace(&mut self) -> &mut Option<PendingRecordingTrace> {
        &mut self.0.pending_recording_trace
    }

    pub(crate) fn recorder(&mut self) -> &mut RecorderState {
        &mut self.0.recorder
    }

    /// The recorder state, for a recording to take.
    pub(crate) fn take_recorder(&mut self) -> RecorderState {
        std::mem::take(&mut self.0.recorder)
    }

    /// Takes back the recorder state from a recording whose answer is dropped.
    pub(crate) fn give_back_recorder(&mut self, recorder: RecorderState) {
        self.0.recorder = recorder;
    }

    /// Hands the recording that is to answer on `ticket` the recorder state, until it does.
    pub(crate) fn await_recording(&mut self, ticket: Arc<RecordingTicket>) {
        self.0.in_flight = Some(ticket);
    }

    /// Takes in what a recording answered.
    pub(crate) fn accept_recording_answer(&mut self, answer: RecordingAnswer) {
        let slot = &mut *self.0;
        slot.recorder = answer.recorder;
        slot.pending_recording_trace = answer.trace;
        slot.pending_recording = Some(answer.pending);
    }

    pub(crate) fn forget_published_frame(&mut self) {
        self.0.recorder.forget_published_recording();
    }

    /// Drops the pending recording unpublished.
    pub(crate) fn discard_pending_recording(&mut self) {
        self.0.pending_recording_trace = None;
        if self
            .0
            .pending_recording
            .take()
            .is_some_and(|pending| pending.publishes_recording)
        {
            self.forget_published_frame();
        }
    }
}

impl LayoutNodeArena {
    /// What the document keeps of its recordings, with the recording in flight taken in: this waits
    /// for it to answer, and for the frame's presentation to publish the answer if it does.
    pub(crate) fn recording(&self) -> TakenIn<'_> {
        self.take_in_recording(true);
        TakenIn(self.recording_slot().borrow_mut())
    }

    /// Takes in the recording in flight if it has answered and its answer is presented, without
    /// waiting for it.
    pub(crate) fn try_take_in_recording(&self) {
        self.take_in_recording(false);
    }

    pub(crate) fn has_recording_in_flight(&self) -> bool {
        self.recording_slot().borrow().in_flight.is_some()
    }

    /// The ticket of the recording in flight, for the frame's presentation to publish its answer.
    pub(crate) fn recording_ticket_for_presentation(&self) -> Option<Arc<RecordingTicket>> {
        let ticket = self.recording_slot().borrow().in_flight.clone()?;
        ticket.will_be_presented();
        Some(ticket)
    }

    fn take_in_recording(&self, wait: bool) {
        let Some(ticket) = self.recording_slot().borrow().in_flight.clone() else {
            return;
        };
        let Some(state) = ticket.take(wait) else {
            return;
        };
        self.recording_slot().borrow_mut().in_flight = None;
        match state {
            TicketState::Answered(answer) => {
                TakenIn(self.recording_slot().borrow_mut()).accept_recording_answer(*answer);
            }
            TicketState::Presented(presented) => {
                {
                    let mut slot = self.recording_slot().borrow_mut();
                    slot.recorder = presented.recorder;
                    slot.pending_recording_trace = presented.trace;
                }
                crate::painting::record::publish::take_in_published_output(
                    self,
                    presented.output,
                    presented.publishes_recording,
                );
            }
            TicketState::Recording | TicketState::Presenting | TicketState::TakenIn => unreachable!(),
        }
    }
}
