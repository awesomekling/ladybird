/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use crate::display_list::commands::{EffectNodeIndex, ReplayClip, ReplayLayer, ReplayMask};
use crate::display_list::replay::ReplayPainter;
use libgfx_rust::path::OwnedPath;
use libgfx_rust::{FloatMatrix4x4, FloatVector3, IntRect, WindingRule};
use std::ffi::c_void;

#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiDisplayListReplayCallbacks {
    pub context: *mut c_void,
    pub canvas_matrix: unsafe extern "C" fn(*mut c_void) -> FloatMatrix4x4,
    pub set_matrix: unsafe extern "C" fn(*mut c_void, *const FloatMatrix4x4),
    pub would_be_fully_clipped_by_painter: unsafe extern "C" fn(*mut c_void, IntRect) -> bool,
    pub push_clip: unsafe extern "C" fn(*mut c_void, *const ReplayClip),
    pub push_clip_path: unsafe extern "C" fn(*mut c_void, *const c_void, WindingRule),
    pub push_layer: unsafe extern "C" fn(*mut c_void, *const ReplayLayer),
    pub push_mask: unsafe extern "C" fn(*mut c_void, *const ReplayMask),
    pub pop_mask: unsafe extern "C" fn(*mut c_void, *const ReplayMask, EffectNodeIndex),
    pub pop: unsafe extern "C" fn(*mut c_void),
    pub push_device_space_plane_clip: unsafe extern "C" fn(*mut c_void, *const FloatVector3, usize),
    pub execute_run: unsafe extern "C" fn(*mut c_void, usize),
}

pub(crate) struct DisplayListReplayHost<'a> {
    callbacks: FfiDisplayListReplayCallbacks,
    _main_thread: &'a crate::stage::MainThread<'a>,
}

impl<'a> DisplayListReplayHost<'a> {
    pub(crate) fn new(callbacks: FfiDisplayListReplayCallbacks, main_thread: &'a crate::stage::MainThread) -> Self {
        Self {
            callbacks,
            _main_thread: main_thread,
        }
    }
}

impl ReplayPainter for DisplayListReplayHost<'_> {
    fn canvas_matrix(&mut self) -> FloatMatrix4x4 {
        // SAFETY: The C++ painter answers synchronously.
        unsafe { (self.callbacks.canvas_matrix)(self.callbacks.context) }
    }

    fn set_matrix(&mut self, matrix: &FloatMatrix4x4) {
        // SAFETY: The C++ painter reads the matrix synchronously.
        unsafe { (self.callbacks.set_matrix)(self.callbacks.context, matrix) };
    }

    fn would_be_fully_clipped_by_painter(&mut self, rect: IntRect) -> bool {
        // SAFETY: The C++ painter answers synchronously.
        unsafe { (self.callbacks.would_be_fully_clipped_by_painter)(self.callbacks.context, rect) }
    }

    fn push_clip(&mut self, clip: &ReplayClip) {
        // SAFETY: The C++ painter reads the clip synchronously.
        unsafe { (self.callbacks.push_clip)(self.callbacks.context, clip) };
    }

    fn push_clip_path(&mut self, path: &OwnedPath, winding_rule: WindingRule) {
        // SAFETY: The C++ painter reads the Gfx::Path synchronously; the tree keeps it alive.
        unsafe { (self.callbacks.push_clip_path)(self.callbacks.context, path.as_raw(), winding_rule) };
    }

    fn push_layer(&mut self, layer: &ReplayLayer) {
        // SAFETY: The C++ painter reads the layer and its filter bytes synchronously; the tree keeps them alive.
        unsafe { (self.callbacks.push_layer)(self.callbacks.context, layer) };
    }

    fn push_mask(&mut self, mask: &ReplayMask) {
        // SAFETY: The C++ painter reads the mask synchronously.
        unsafe { (self.callbacks.push_mask)(self.callbacks.context, mask) };
    }

    fn pop_mask(&mut self, mask: &ReplayMask, effect: EffectNodeIndex) {
        // SAFETY: The C++ painter reads the mask synchronously.
        unsafe { (self.callbacks.pop_mask)(self.callbacks.context, mask, effect) };
    }

    fn pop(&mut self) {
        // SAFETY: The C++ painter pops synchronously.
        unsafe { (self.callbacks.pop)(self.callbacks.context) };
    }

    fn push_device_space_plane_clip(&mut self, vertices: &[FloatVector3]) {
        // SAFETY: The C++ painter reads the vertices synchronously.
        unsafe {
            (self.callbacks.push_device_space_plane_clip)(self.callbacks.context, vertices.as_ptr(), vertices.len());
        };
    }

    fn execute_run(&mut self, run_index: usize) {
        // SAFETY: The C++ painter plays the run's commands synchronously.
        unsafe { (self.callbacks.execute_run)(self.callbacks.context, run_index) };
    }
}
