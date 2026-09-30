/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::painting::host::{FfiImageContent, FfiImageContentKind};
use libgfx_rust::image_frame::ImageFrameHandle;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) enum ImageContent {
    #[default]
    None,
    Raster(Option<ImageFrameHandle>),
    Vector {
        content_identity: u64,
        image_identity: u64,
        has_active_view_box: bool,
    },
}

impl ImageContent {
    pub(crate) fn from_ffi(content: &FfiImageContent) -> Self {
        match content.kind {
            FfiImageContentKind::None => Self::None,
            FfiImageContentKind::Raster => Self::Raster(ImageFrameHandle::resolve(content.frame_id)),
            FfiImageContentKind::Vector => Self::Vector {
                content_identity: content.vector_content_identity,
                image_identity: content.vector_image_identity,
                has_active_view_box: content.vector_has_active_view_box,
            },
        }
    }
}
