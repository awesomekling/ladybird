/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::painting::record::order_tree::PaintOrderTree;
use crate::painting::record::scratch::RecordingScratch;
use crate::painting::record::{PublishedHitTestItems, RecordingOutput};
use std::sync::Arc;

/// What the display list recording keeps from one recording of a document to the next. Only the
/// recording and its publication write it, and nothing the document publishes for the paint side
/// is in it, so it moves with the recording rather than being shared with what it reads.
#[derive(Default)]
pub(crate) struct RecorderState {
    /// The last recording that published, which the next one copies clean output from.
    pub(crate) published_recording: Option<Arc<RecordingOutput>>,
    pub(crate) published_hit_test_items: Option<Arc<PublishedHitTestItems>>,
    /// The paint-order tree describing the published recording; a recording appends to it and
    /// publication or discarding decides what stays.
    pub(crate) paint_order_tree: PaintOrderTree,
    /// The recording's workspace, whose tables one recording leaves for the next to reuse.
    pub(crate) scratch: RecordingScratch,
}

const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<RecorderState>();
};

impl RecorderState {
    /// A recording that publishes assembles its frame in the retained paint-order tree, which no
    /// longer describes the published recording once that recording is dropped: the next recording
    /// copies nothing from it and records from scratch into a tree of its own.
    pub(crate) fn forget_published_recording(&mut self) {
        self.published_recording = None;
        self.published_hit_test_items = None;
        self.paint_order_tree = Default::default();
    }
}
