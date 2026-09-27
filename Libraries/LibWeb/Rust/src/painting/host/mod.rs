/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

pub mod hit_test;
pub mod paint;
pub use libcompositing_rust::host::replay;
pub mod visual_context;

pub use hit_test::*;
pub use paint::*;
pub use replay::*;
pub use visual_context::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(C)]
pub struct FfiRootBackgroundSource {
    pub use_body_background_properties: bool,
    pub root_layout_node: crate::layout::node_data::NodeSlotId,
    pub body_layout_node: crate::layout::node_data::NodeSlotId,
}

impl Default for FfiRootBackgroundSource {
    fn default() -> Self {
        Self {
            use_body_background_properties: false,
            root_layout_node: crate::layout::node_data::NodeSlotId::INVALID,
            body_layout_node: crate::layout::node_data::NodeSlotId::INVALID,
        }
    }
}

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiGeometryHostCallbacks {
    pub context: *mut std::ffi::c_void,
    /// Stores a scroll offset the render side settled on. Called after the pass that settled it,
    /// never from inside one.
    pub set_scroll_offset: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        crate::layout::node_data::NodeSlotId,
        crate::layout::used_values::FfiCssPixelPoint,
    ),
}

#[derive(Clone, Copy)]
pub(crate) struct GeometryHostCallbacks {
    context: *mut std::ffi::c_void,
    set_scroll_offset: unsafe extern "C" fn(
        *mut std::ffi::c_void,
        crate::layout::node_data::NodeSlotId,
        crate::layout::used_values::FfiCssPixelPoint,
    ),
}

impl From<FfiGeometryHostCallbacks> for GeometryHostCallbacks {
    fn from(host: FfiGeometryHostCallbacks) -> Self {
        Self {
            context: host.context,
            set_scroll_offset: host.set_scroll_offset,
        }
    }
}

impl GeometryHostCallbacks {
    /// # Safety
    ///
    /// `layout_node` must name a live row. The host re-enters geometry queries and writes the
    /// store the offset lives in, so no arena or cache borrow may be held across this call and no
    /// pass may be running.
    pub(crate) unsafe fn set_scroll_offset(
        &self,
        _: &crate::stage::MainThread,
        layout_node: crate::layout::node_data::NodeSlotId,
        offset: crate::layout::used_values::FfiCssPixelPoint,
    ) {
        crate::painting::seal::note_host_call("set_scroll_offset");
        // SAFETY: The caller guarantees the row is live and no borrow is held.
        unsafe { (self.set_scroll_offset)(self.context, layout_node, offset) };
    }
}
