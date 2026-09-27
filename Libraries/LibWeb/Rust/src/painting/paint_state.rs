/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::layout::node_data::NodeSlotId;
use std::sync::Arc;

pub(crate) struct PendingRecording {
    pub(crate) recording: crate::painting::record::RecordingResult,
    pub(crate) recording_from_scratch: Option<crate::painting::record::RecordingResult>,
    pub(crate) publishes_recording: bool,
    /// The generation of the arena's render state the recording was made for.
    pub(crate) frame_generation: u64,
}

pub(crate) struct PendingRecordingTrace {
    pub(crate) viewport: NodeSlotId,
    pub(crate) should_paint_overlay: bool,
}

/// What a recording answers once it has run: the recorder state it was handed, and what it
/// recorded, unless a read cancelled it.
pub(crate) struct RecordingAnswer {
    pub(crate) recorder: crate::painting::record::recorder_state::RecorderState,
    pub(crate) publishes_recording: bool,
    pub(crate) recorded: Option<(PendingRecording, Option<PendingRecordingTrace>)>,
}

/// Where a submitted recording sends its answer.
pub(crate) type RecordingTicket = std::sync::mpsc::Receiver<RecordingAnswer>;

/// What a clock lease's tick records its document's display list with.
#[derive(Clone)]
pub(crate) struct ClockRecording {
    pub(crate) viewport: NodeSlotId,
    pub(crate) inputs: crate::painting::record::RecordingInputs<'static>,
}

#[derive(Default)]
pub struct PaintState {
    pub(crate) trace_recordings: bool,
    // The four below are what a recording answers. While a submitted recording has not answered,
    // it holds the recorder state, and each is read through a method that takes its answer in.
    pending_recording_trace: Option<PendingRecordingTrace>,
    pending_recording: Option<PendingRecording>,
    // Set by a recording in flight that a read cancelled, which left nothing pending: its frame's
    // presentation shows nothing, and the frame's consume has the document record again.
    recording_was_cancelled: bool,
    recorder: crate::painting::record::recorder_state::RecorderState,
    recording_in_flight: Option<RecordingTicket>,
    // The SVG-as-image renders the next recording looks up, resolved by the main thread.
    pub(crate) vector_image_display_lists:
        std::sync::Arc<crate::painting::record::vector_images::VectorImageDisplayLists>,
    pub(crate) visual_context: crate::painting::visual_context::VisualContextState,
    pub(crate) root_background_source: Option<crate::painting::host::FfiRootBackgroundSource>,
    pub(crate) hit_test_list_generation: u64,
    pub(crate) last_recording: Option<Arc<crate::painting::record::RecordingOutput>>,
    pub(crate) selection: Option<Arc<crate::painting::selection::SelectionRange>>,
    // LIBWEB_RENDER_CLOCK_FRAMES: the inputs of the last recording the main thread published, which
    // a clock lease's ticks record again with while the main thread idles.
    pub(crate) clock_recording: Option<ClockRecording>,
    // Shared with the frames published since it last changed.
    pub(crate) selection_pseudo_styles: Arc<SelectionPseudoStyles>,
}

pub(crate) type SelectionPseudoStyles =
    std::collections::HashMap<NodeSlotId, Arc<crate::painting::record::paint::text::SelectionStyleAnswer>>;

impl PaintState {
    /// Takes in the answer of the recording in flight, if one is, waiting for it to answer.
    fn take_recording_answer(&mut self) {
        let Some(ticket) = self.recording_in_flight.take() else {
            return;
        };
        let answer = ticket.recv().expect("a submitted recording always answers");
        self.accept_recording_answer(answer);
    }

    /// Hands the recording that is to answer on `ticket` the recorder state, until it does.
    pub(crate) fn await_recording(&mut self, ticket: RecordingTicket) {
        assert!(
            self.recording_in_flight.is_none(),
            "one recording of a document is in flight at a time"
        );
        self.recording_in_flight = Some(ticket);
    }

    /// Takes in what a recording answered.
    pub(crate) fn accept_recording_answer(&mut self, answer: RecordingAnswer) {
        self.recorder = answer.recorder;
        match answer.recorded {
            Some((pending, trace)) => {
                self.pending_recording_trace = trace;
                self.pending_recording = Some(pending);
            }
            None => {
                if answer.publishes_recording {
                    self.forget_published_frame();
                }
                self.recording_was_cancelled = true;
            }
        }
    }

    /// The recorder state, for a recording to take.
    pub(crate) fn take_recorder(&mut self) -> crate::painting::record::recorder_state::RecorderState {
        self.take_recording_answer();
        std::mem::take(&mut self.recorder)
    }

    /// Takes back the recorder state from a recording whose answer is dropped.
    pub(crate) fn give_back_recorder(&mut self, recorder: crate::painting::record::recorder_state::RecorderState) {
        debug_assert!(self.recording_in_flight.is_none());
        self.recorder = recorder;
    }

    pub(crate) fn recorder(&mut self) -> &mut crate::painting::record::recorder_state::RecorderState {
        self.take_recording_answer();
        &mut self.recorder
    }

    pub(crate) fn pending_recording(&mut self) -> &mut Option<PendingRecording> {
        self.take_recording_answer();
        &mut self.pending_recording
    }

    pub(crate) fn pending_recording_trace(&mut self) -> &mut Option<PendingRecordingTrace> {
        self.take_recording_answer();
        &mut self.pending_recording_trace
    }

    pub(crate) fn recording_was_cancelled(&mut self) -> &mut bool {
        self.take_recording_answer();
        &mut self.recording_was_cancelled
    }

    /// Drops the pending recording unpublished.
    pub(crate) fn discard_pending_recording(&mut self) {
        *self.pending_recording_trace() = None;
        if self
            .pending_recording()
            .take()
            .is_some_and(|pending| pending.publishes_recording)
        {
            self.forget_published_frame();
        }
    }

    pub(crate) fn forget_published_frame(&mut self) {
        self.recorder().forget_published_recording();
    }

    pub(crate) fn update_root_background_source(
        &mut self,
        arena: &crate::layout::LayoutNodeArena,
        source: crate::painting::host::FfiRootBackgroundSource,
    ) -> bool {
        let Some(previous) = self.root_background_source.replace(source) else {
            return false;
        };
        if previous == source {
            return false;
        }
        use crate::painting::record::damage::PaintDamage;
        // Propagation changes which box paints the body's background. Push both the old
        // and new owners, including inline pieces painted by their containing block.
        for source in [previous, source] {
            for slot in [source.root_layout_node, source.body_layout_node] {
                arena.push_paint_damage_for_repaint(slot, PaintDamage::DRAW_BACKGROUND);
            }
            // The viewport's scrollbars take their colors from the propagated background.
            if let Some(viewport) = arena.node_parent_if_live(source.root_layout_node) {
                arena.push_paint_damage(viewport, PaintDamage::DRAW_OVERLAY | PaintDamage::SCROLL_METADATA);
            }
        }
        true
    }

    pub(crate) fn reset_visual_context_state(&mut self) {
        self.visual_context = crate::painting::visual_context::VisualContextState {
            needs_to_refresh_scroll_state: true,
            ..Default::default()
        };
        self.visual_context
            .dirty_boxes
            .request_full_rebuild(crate::painting::visual_context::dirty::VisualContextGlobalRebuildReason::FirstBuild);
    }
}
