/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::layout::LayoutNodeArena;
use crate::painting::host::{PublishesToHost, RecordingPublishHost};
use crate::painting::paint_state::PendingRecording;
use crate::painting::record::recorder_state::RecorderState;
use crate::painting::record::resources::RecordingResourceManifest;
use crate::painting::record::{RecordingOutput, RecordingResult};

/// Publishes a pending recording from the document: hands its resources to the host and takes
/// its output in.
pub(crate) fn publish_recording(
    arena: &LayoutNodeArena,
    pending: PendingRecording,
    main_thread: &impl PublishesToHost,
    publish: &RecordingPublishHost,
) {
    let publishes_recording = pending.publishes_recording;
    let output = publish_to_host(pending, arena.recording().recorder(), main_thread, publish);
    take_in_published_output(arena, output, publishes_recording);
}

/// Hands a recording's resources to the host and makes its output, reading nothing but the
/// recording and the recorder state it recorded with: what a publication does before the
/// document takes the output in.
pub(crate) fn publish_to_host(
    pending: PendingRecording,
    recorder: &RecorderState,
    main_thread: &impl PublishesToHost,
    publish: &RecordingPublishHost,
) -> RecordingOutput {
    let PendingRecording {
        recording: RecordingResult { mut output, resources },
        recording_from_scratch,
        publishes_recording: _,
        frame_generation: _,
        svg_paint_resources,
    } = pending;
    let RecordingResourceManifest {
        fonts,
        image_frames,
        video_sinks,
        painted_vector_images,
        missed_vector_images,
        ..
    } = resources;
    for font in fonts.values() {
        publish.add_font(main_thread, font);
    }
    for frame in image_frames.values() {
        publish.add_image_frame(main_thread, frame);
    }
    for frame in published_filter_image_frames(&svg_paint_resources) {
        publish.add_image_frame(main_thread, &frame);
    }
    for (resource_id, sink_handle) in video_sinks {
        publish.add_video_sink(main_thread, resource_id, sink_handle);
    }
    for render in painted_vector_images.values() {
        publish.add_vector_image_render(main_thread, render);
    }
    output.vector_images = painted_vector_images;
    output.missed_vector_images = missed_vector_images;
    if let Some(recording_from_scratch) = recording_from_scratch {
        crate::painting::record::verify::verify_assembled_recording_matches_fresh(
            &output,
            &recording_from_scratch.output,
        );
    }
    output.is_identical_to_published_frame = recorder
        .published_recording
        .as_ref()
        .zip(recorder.published_hit_test_items.as_ref())
        .is_some_and(|(source, item_source)| {
            std::sync::Arc::ptr_eq(&output.display_list, &source.display_list)
                && std::sync::Arc::ptr_eq(&output.hit_test_list.items, &item_source.items)
                && output.recorded_structural_epoch == source.recorded_structural_epoch
                && output.wheel_event_listener_state_generation == source.wheel_event_listener_state_generation
                && output.has_blocking_wheel_event_listeners == source.has_blocking_wheel_event_listeners
        });
    output
}

// Resource callbacks and verification must finish before the new frame becomes the source.
/// Takes a published recording's output in: its hit-test list, and for a recording that publishes,
/// the source the next recording copies from and the damage it consumed. Returns the generation of
/// the document's hit-test list.
pub(crate) fn take_in_published_output(
    arena: &LayoutNodeArena,
    mut output: RecordingOutput,
    publishes_recording: bool,
) -> u64 {
    let mut recording = arena.recording();
    let recorder = recording.recorder();
    let list = std::mem::take(&mut output.hit_test_list);
    let mut hit_test_list = arena.hit_test_list.borrow_mut();
    let previous_list_is_the_source = hit_test_list
        .as_ref()
        .zip(recorder.published_hit_test_items.as_ref())
        .is_some_and(|(list, source)| std::sync::Arc::ptr_eq(&list.items, &source.items));
    let mut paint_state = arena.paint_state().borrow_mut();
    if output.is_identical_to_published_frame && previous_list_is_the_source {
        drop(list);
    } else {
        paint_state.hit_test_list_generation += 1;
        debug_assert_eq!(list.generation, paint_state.hit_test_list_generation);
        if publishes_recording {
            recorder.published_hit_test_items =
                Some(std::sync::Arc::new(crate::painting::record::PublishedHitTestItems {
                    items: list.items.clone(),
                }));
        }
        *hit_test_list = Some(std::sync::Arc::new(list));
    }
    let output = std::sync::Arc::new(output);
    if publishes_recording {
        recorder.published_recording = Some(output.clone());
        // Read-only recordings publish no frame and must not consume the damage.
        arena.clear_paint_damage_consumed_by_published_recording();
        paint_state.visual_context.quarantined_slots_are_releasable = true;
    }
    paint_state.last_recording = Some(output);
    hit_test_list.as_ref().map_or(0, |list| list.generation)
}

/// The image frames of the published SVG filters in `rows`, once each.
fn published_filter_image_frames(
    rows: &crate::painting::svg_paint_resources::SvgPaintResourceRows,
) -> Vec<libgfx_rust::image_frame::ImageFrameHandle> {
    crate::painting::svg_paint_resources::published_filter_image_frames_in(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::painting::hit_test::HitTestList;
    use crate::painting::record::damage::PaintDamage;
    use std::sync::Arc;

    #[test]
    fn read_only_publication_keeps_the_source_frame_and_the_pending_damage() {
        let mut arena = LayoutNodeArena::new();
        let row = arena.allocate_for_test().slot;
        arena.populate_paintable_row(row);
        let mut original_source = None;
        // Publish a source, a read-only recording, and then the pending repaint.
        for (hit_test_generation, read_write) in [(1, true), (2, false), (3, true)] {
            if read_write {
                arena.note_publishing_paint_recording_started();
            }
            let output = RecordingOutput {
                hit_test_list: HitTestList {
                    generation: hit_test_generation,
                    ..Default::default()
                },
                ..Default::default()
            };
            assert_eq!(
                take_in_published_output(&arena, output, read_write),
                hit_test_generation
            );
            let source = arena.recording().recorder().published_recording.clone().unwrap();
            match hit_test_generation {
                1 => {
                    original_source = Some(source);
                    arena.push_paint_damage(row, PaintDamage::DRAW_FOREGROUND);
                }
                2 => {
                    assert!(Arc::ptr_eq(original_source.as_ref().unwrap(), &source));
                    assert_eq!(arena.paint_damage_of_row(row), PaintDamage::DRAW_FOREGROUND);
                }
                3 => {
                    assert!(!Arc::ptr_eq(original_source.as_ref().unwrap(), &source));
                    assert_eq!(arena.paint_damage_of_row(row), PaintDamage::NONE);
                }
                _ => unreachable!(),
            }
        }
    }
}
