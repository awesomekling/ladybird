/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use std::collections::{HashMap, HashSet};

use crate::painting::display_list::commands::{DisplayListResourceId, ImageFrameResourceId, VideoSinkResourceId};
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
    // Every SVG-as-image render the recording painted, which is what the main thread resolves
    // ahead of the next recording.
    pub(crate) painted_vector_images: HashSet<VectorImageRenderRequest>,
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
        self.painted_vector_images.insert(request);
        if let Some(display_list) = resolved.get(&request) {
            return display_list;
        }
        let next_index = self.missed_vector_images.len() as u32;
        let index = *self.missed_vector_image_indices.entry(request).or_insert_with(|| {
            self.missed_vector_images.push(request);
            next_index
        });
        DisplayListResourceId(VECTOR_IMAGE_PLACEHOLDER_TAG | u64::from(index))
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
        assert_eq!(manifest.painted_vector_images.len(), 2);
    }
}
