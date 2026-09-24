/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use std::collections::HashMap;
use std::ops::Range;

use crate::painting::display_list::builder::RecordedDisplayList;
use crate::painting::display_list::commands::{
    DisplayListCommandType, DisplayListResourceId, ImageFrameResourceId, PaintNestedDisplayList, VideoSinkResourceId,
};
use crate::painting::record::vector_images::{
    VECTOR_IMAGE_PLACEHOLDER_TAG, VectorImageDisplayLists, VectorImageRenderRequest,
};
use libgfx_rust::font::{FontHandle, FontId};
use libgfx_rust::image_frame::ImageFrameHandle;

#[derive(Default)]
pub(crate) struct RecordingResourceManifest {
    pub(crate) fonts: HashMap<FontId, FontHandle>,
    pub(crate) image_frames: HashMap<u64, ImageFrameHandle>,
    pub(crate) video_sinks: HashMap<u64, u64>,
    // The SVG-as-image renders the recording painted with a resolved display list, by that list:
    // the ones it recorded and the ones in output it copied from the published frame. A missed
    // render joins once its publish patches it in. The main thread resolves these ahead of the
    // next recording.
    pub(crate) painted_vector_images: HashMap<DisplayListResourceId, VectorImageRenderRequest>,
    // The renders the recording's map lacked; each one's placeholder names its index here.
    pub(crate) missed_vector_images: Vec<VectorImageRenderRequest>,
    missed_vector_image_indices: HashMap<VectorImageRenderRequest, u32>,
}

impl RecordingResourceManifest {
    pub(crate) fn note_font(&mut self, font: &FontHandle) -> u64 {
        self.fonts.entry(font.id()).or_insert_with(|| font.clone());
        font.id().0
    }

    pub(crate) fn note_image_frame(&mut self, frame: &ImageFrameHandle) -> ImageFrameResourceId {
        self.image_frames.entry(frame.id()).or_insert_with(|| frame.clone());
        ImageFrameResourceId(frame.id())
    }

    pub(crate) fn note_video_sink(&mut self, resource_id: u64, sink_handle: u64) -> VideoSinkResourceId {
        self.video_sinks.entry(resource_id).or_insert(sink_handle);
        VideoSinkResourceId(resource_id)
    }

    /// The display list of `request` from the map the main thread resolved, or a placeholder its
    /// publish patches once the main thread has resolved the render.
    pub(crate) fn vector_image_display_list(
        &mut self,
        request: VectorImageRenderRequest,
        resolved: &VectorImageDisplayLists,
    ) -> DisplayListResourceId {
        if let Some(display_list) = resolved.get(&request) {
            self.painted_vector_images.insert(display_list, request);
            return display_list;
        }
        let next_index = self.missed_vector_images.len() as u32;
        let index = *self.missed_vector_image_indices.entry(request).or_insert_with(|| {
            self.missed_vector_images.push(request);
            next_index
        });
        DisplayListResourceId(VECTOR_IMAGE_PLACEHOLDER_TAG | u64::from(index))
    }

    /// Notes the SVG-as-image renders in `bytes` of `source`, whose renders are `source_vector_images`,
    /// as painted by this recording: output copied from the published frame paints them too.
    pub(crate) fn note_copied_vector_images(
        &mut self,
        source: &RecordedDisplayList,
        source_vector_images: &HashMap<DisplayListResourceId, VectorImageRenderRequest>,
        bytes: Range<u32>,
    ) {
        if source_vector_images.is_empty() {
            return;
        }
        crate::painting::display_list::nested_records::for_each_command_including_nested(
            &source.bytes[bytes.start as usize..bytes.end as usize],
            &mut |command_type, _, payload| {
                if command_type != DisplayListCommandType::PaintNestedDisplayList {
                    return;
                }
                let id = crate::painting::display_list::builder::read_command::<PaintNestedDisplayList>(payload)
                    .display_list_id;
                if let Some(request) = source_vector_images.get(&id) {
                    self.painted_vector_images.insert(id, *request);
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::css_pixels::CssPixels;
    use crate::painting::record::vector_images::{is_vector_image_placeholder, vector_image_placeholder_index};

    fn request(image_identity: u64) -> VectorImageRenderRequest {
        VectorImageRenderRequest::new(
            image_identity,
            0,
            CssPixels::from_integer(10),
            CssPixels::from_integer(20),
            1.0,
        )
    }

    #[test]
    fn a_resolved_render_is_looked_up_and_a_missed_one_waits_for_the_main_thread() {
        let mut resolved = VectorImageDisplayLists::default();
        resolved.insert(request(1), DisplayListResourceId(42));
        let mut manifest = RecordingResourceManifest::default();
        assert_eq!(
            manifest.vector_image_display_list(request(1), &resolved),
            DisplayListResourceId(42)
        );
        let missed = manifest.vector_image_display_list(request(2), &resolved);
        assert!(is_vector_image_placeholder(missed));
        assert_eq!(vector_image_placeholder_index(missed), 0);
        assert_eq!(manifest.vector_image_display_list(request(2), &resolved), missed);
        assert_eq!(manifest.missed_vector_images, vec![request(2)]);
        assert_eq!(
            manifest.painted_vector_images,
            HashMap::from([(DisplayListResourceId(42), request(1))])
        );
    }

    #[test]
    fn a_copied_render_stays_painted() {
        let mut recorder = crate::painting::display_list::recorder::DisplayListRecorder::new(None);
        for id in [7, 8] {
            recorder.paint_nested_display_list(
                DisplayListResourceId(id),
                libgfx_rust::FloatRect::new(0.0, 0.0, 10.0, 20.0),
                libgfx_rust::IntSize { width: 10, height: 20 },
            );
        }
        let source = recorder.into_builder().finish();
        let first_command_end = source.bytes.len() as u32 / 2;
        let source_vector_images = HashMap::from([
            (DisplayListResourceId(7), request(1)),
            (DisplayListResourceId(8), request(2)),
        ]);
        let mut manifest = RecordingResourceManifest::default();
        manifest.note_copied_vector_images(&source, &source_vector_images, 0..first_command_end);
        assert_eq!(
            manifest.painted_vector_images,
            HashMap::from([(DisplayListResourceId(7), request(1))])
        );
    }
}
