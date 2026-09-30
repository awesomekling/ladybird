/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use std::collections::{HashMap, HashSet};
use std::ops::Range;

use crate::painting::display_list::builder::RecordedDisplayList;
use crate::painting::display_list::commands::{
    DisplayListCommandType, DisplayListResourceId, ImageFrameResourceId, PaintNestedDisplayList, VideoSinkResourceId,
};
use crate::painting::record::vector_images::{PaintedVectorImages, VectorImageDisplayLists, VectorImageRenderRequest};
use libgfx_rust::font::{FontHandle, FontId};
use libgfx_rust::image_frame::ImageFrameHandle;

#[derive(Default)]
pub(crate) struct RecordingResourceManifest {
    pub(crate) fonts: HashMap<FontId, FontHandle>,
    pub(crate) image_frames: HashMap<u64, ImageFrameHandle>,
    pub(crate) video_sinks: HashMap<u64, u64>,
    // The SVG-as-image renders the recording painted, by the display list it painted each with:
    // the ones it recorded and the ones in output it copied from the published frame.
    pub(crate) painted_vector_images: PaintedVectorImages,
    // The renders the recording's map lacked, which it painted as empty images.
    pub(crate) missed_vector_images: HashSet<VectorImageRenderRequest>,
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

    /// The display list of `request` from the map the main thread resolved. A render the map lacks
    /// is missed: the caller paints nothing for it, and the main thread resolves it ahead of the
    /// next recording.
    pub(crate) fn vector_image_display_list(
        &mut self,
        request: VectorImageRenderRequest,
        resolved: &VectorImageDisplayLists,
    ) -> Option<DisplayListResourceId> {
        let Some(render) = resolved.get(&request) else {
            self.missed_vector_images.insert(request);
            return None;
        };
        self.painted_vector_images
            .entry(render.display_list)
            .or_insert_with(|| render.clone());
        Some(render.display_list)
    }

    /// Notes the SVG-as-image renders in `bytes` of `source`, whose renders are `source_vector_images`,
    /// as painted by this recording: output copied from the published frame paints them too.
    pub(crate) fn note_copied_vector_images(
        &mut self,
        source: &RecordedDisplayList,
        source_vector_images: &PaintedVectorImages,
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
                if let Some(render) = source_vector_images.get(&id) {
                    self.painted_vector_images.entry(id).or_insert_with(|| render.clone());
                }
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::css::css_pixels::CssPixels;
    use crate::painting::record::vector_images::VectorImageRender;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn request(image_identity: u64) -> VectorImageRenderRequest {
        VectorImageRenderRequest::new(
            image_identity,
            0,
            CssPixels::from_integer(10),
            CssPixels::from_integer(20),
            1.0,
        )
    }

    // The tests' renders retain a counter of their releases.
    unsafe extern "C" fn count_release(retained: *const std::ffi::c_void) {
        // SAFETY: Each test's counter outlives its renders.
        unsafe { &*retained.cast::<AtomicUsize>() }.fetch_add(1, Ordering::Relaxed);
    }

    fn render(image_identity: u64, display_list: u64, released: &AtomicUsize) -> VectorImageRender {
        // SAFETY: `count_release` releases the counter by counting.
        unsafe {
            VectorImageRender::adopt(
                request(image_identity),
                DisplayListResourceId(display_list),
                std::ptr::from_ref(released).cast(),
                count_release,
            )
        }
    }

    fn requests(painted: &PaintedVectorImages) -> HashMap<DisplayListResourceId, VectorImageRenderRequest> {
        painted.iter().map(|(id, render)| (*id, render.request)).collect()
    }

    #[test]
    fn a_resolved_render_is_looked_up_and_a_missed_one_waits_for_the_main_thread() {
        let released = AtomicUsize::new(0);
        let mut resolved = VectorImageDisplayLists::default();
        resolved.insert(render(1, 42, &released));
        let mut manifest = RecordingResourceManifest::default();
        assert_eq!(
            manifest.vector_image_display_list(request(1), &resolved),
            Some(DisplayListResourceId(42))
        );
        assert_eq!(manifest.vector_image_display_list(request(2), &resolved), None);
        assert_eq!(manifest.vector_image_display_list(request(2), &resolved), None);
        assert_eq!(manifest.missed_vector_images, HashSet::from([request(2)]));
        assert_eq!(
            requests(&manifest.painted_vector_images),
            HashMap::from([(DisplayListResourceId(42), request(1))])
        );
        // The recording carries the render itself to its publication, which outlives the map it was looked up in.
        drop(resolved);
        assert_eq!(released.load(Ordering::Relaxed), 0);
        drop(manifest);
        assert_eq!(released.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_copied_render_stays_painted() {
        let released = AtomicUsize::new(0);
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
        let source_vector_images = PaintedVectorImages::from([
            (DisplayListResourceId(7), Arc::new(render(1, 7, &released))),
            (DisplayListResourceId(8), Arc::new(render(2, 8, &released))),
        ]);
        let mut manifest = RecordingResourceManifest::default();
        manifest.note_copied_vector_images(&source, &source_vector_images, 0..first_command_end);
        assert_eq!(
            requests(&manifest.painted_vector_images),
            HashMap::from([(DisplayListResourceId(7), request(1))])
        );
        // The frame it copied from lets its renders go; the copy keeps the one it paints.
        drop(source_vector_images);
        assert_eq!(released.load(Ordering::Relaxed), 1);
        drop(manifest);
        assert_eq!(released.load(Ordering::Relaxed), 2);
    }
}
