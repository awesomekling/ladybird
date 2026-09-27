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
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};

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
    // The recording unwound before it answered, taking the recorder state with it. Its panic
    // continues where the main thread takes its stage back.
    Abandoned,
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

/// How a submitted recording answers on its ticket, which only it can. Dropped without answering, as
/// when the recording panics, it leaves the ticket abandoned, so nothing that waits for the answer
/// waits forever.
pub(crate) struct RecordingAnswerer(Option<Arc<RecordingTicket>>);

impl RecordingAnswerer {
    /// On the recording's stage, once it has run.
    pub(crate) fn answer(mut self, answer: RecordingAnswer) {
        let ticket = self.0.take().expect("a recording answers once");
        ticket.set(TicketState::Answered(Box::new(answer)));
    }
}

impl Drop for RecordingAnswerer {
    fn drop(&mut self) {
        if let Some(ticket) = self.0.take() {
            ticket.set(TicketState::Abandoned);
        }
    }
}

impl RecordingTicket {
    /// A ticket, and the one answerer that answers on it.
    pub(crate) fn new() -> (Arc<Self>, RecordingAnswerer) {
        let ticket = Arc::new(Self {
            state: Mutex::new(TicketState::Recording),
            changed: Condvar::new(),
            presented_by_frame: AtomicBool::new(false),
        });
        let answerer = RecordingAnswerer(Some(ticket.clone()));
        (ticket, answerer)
    }

    fn lock(&self) -> MutexGuard<'_, TicketState> {
        // Nothing panics while holding the state, so a poisoned lock still holds a whole one.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn set(&self, state: TicketState) {
        *self.lock() = state;
        self.changed.notify_all();
    }

    /// On the document thread, as the frame's presentation is submitted: it publishes the answer.
    pub(crate) fn will_be_presented(&self) {
        self.presented_by_frame.store(true, Ordering::Relaxed);
    }

    /// Whether the document took what the recording left in.
    pub(crate) fn is_taken_in(&self) -> bool {
        matches!(*self.lock(), TicketState::TakenIn)
    }

    /// On the presentation stage: whether the recording unwound, and left nothing to present.
    pub(crate) fn was_abandoned(&self) -> bool {
        matches!(*self.wait_for_answer(), TicketState::Abandoned)
    }

    fn wait_for_answer(&self) -> MutexGuard<'_, TicketState> {
        let mut state = self.lock();
        while matches!(*state, TicketState::Recording) {
            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
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
        let mut released_holds = false;
        loop {
            let is_final = match &*state {
                TicketState::Recording | TicketState::Presenting => false,
                TicketState::Answered(_) => !self.presented_by_frame.load(Ordering::Relaxed),
                TicketState::Abandoned | TicketState::Presented(_) => true,
                TicketState::TakenIn => {
                    debug_assert!(false, "a document takes a recording in once");
                    true
                }
            };
            if is_final {
                return Some(std::mem::replace(&mut *state, TicketState::TakenIn));
            }
            if !wait {
                return None;
            }
            // A test's hold on the recording or its presentation would keep them from answering.
            if !released_holds {
                crate::stage_thread::release_holds_on_recording();
                released_holds = true;
            }
            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
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

    #[cfg(test)]
    pub(crate) fn has_recording_in_flight(&self) -> bool {
        self.recording_slot().borrow().in_flight.is_some()
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
            // The recording unwound with the recorder state: the next one starts from none.
            TicketState::Abandoned | TicketState::TakenIn => {}
            TicketState::Recording | TicketState::Presenting => unreachable!("a ticket is taken in final"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn panic_while_recording(answerer: RecordingAnswerer) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let _answerer = answerer;
            panic!("the recording failed");
        })
    }

    #[test]
    fn a_recording_that_panics_leaves_its_ticket_abandoned_for_the_presentation() {
        let (ticket, answerer) = RecordingTicket::new();
        ticket.will_be_presented();
        assert!(panic_while_recording(answerer).join().is_err());
        assert!(ticket.was_abandoned());
        assert!(
            ticket
                .present::<()>(|_, _| unreachable!("an abandoned recording presents nothing"))
                .is_none()
        );
        assert!(matches!(ticket.take(true), Some(TicketState::Abandoned)));
    }

    #[test]
    fn a_document_waiting_for_a_recording_that_panics_takes_in_nothing() {
        let arena = LayoutNodeArena::new();
        let (ticket, answerer) = RecordingTicket::new();
        arena.recording().await_recording(ticket.clone());
        ticket.will_be_presented();
        let recording = panic_while_recording(answerer);
        {
            let mut taken_in = arena.recording();
            assert!(taken_in.pending_recording().is_none());
            assert!(taken_in.recorder().published_recording.is_none());
        }
        assert!(!arena.has_recording_in_flight());
        assert!(recording.join().is_err());
    }
}
