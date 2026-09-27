/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Where one document's style engine lives, and the only way to it.
//!
//! C++ and the layout arena name an engine by a [`StyleEngineHandle`], which points to the engine's
//! home and has no way to the engine but the home's: every path to the engine goes through the
//! home, which makes the main thread wait for a frame in flight that reaches the engine first.

use super::StyleEngine;
use std::ffi::c_void;
use std::ptr::NonNull;

/// What C++ holds for one document's style engine: a pointer to its home, opaque to C++. It has no
/// way to the engine but the home's.
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StyleEngineHandle(*mut c_void);

/// Where one document's style engine lives.
struct StyleEngineHome {
    engine: NonNull<StyleEngine>,
}

impl StyleEngineHandle {
    /// A handle that names no engine.
    pub const fn null() -> Self {
        Self(std::ptr::null_mut())
    }

    pub fn is_null(self) -> bool {
        self.0.is_null()
    }

    /// Gives `engine` a home, and returns the handle that names it.
    pub(crate) fn create(engine: Box<StyleEngine>) -> Self {
        let home = Box::new(StyleEngineHome {
            engine: NonNull::from(Box::leak(engine)),
        });
        Self(Box::into_raw(home).cast())
    }

    /// A home for an engine a unit test owns, which the handle names for as long as the engine
    /// lives. The home stays behind.
    #[cfg(test)]
    pub(crate) fn for_test_engine(engine: *mut StyleEngine) -> Self {
        let home = Box::new(StyleEngineHome {
            engine: NonNull::new(engine).expect("a test engine is not null"),
        });
        Self(Box::into_raw(home).cast())
    }

    /// The handle as C++ holds it.
    pub(crate) fn into_ffi(self) -> *mut c_void {
        self.0
    }

    /// Takes the engine out of its home, which goes away. The main thread waits for a frame in
    /// flight that reaches the engine first.
    ///
    /// # Safety
    ///
    /// The handle must come from [`Self::create`], be used on the document thread, and not be used
    /// again.
    pub(crate) unsafe fn destroy(self, entry: &'static str) -> Box<StyleEngine> {
        assert!(!self.is_null(), "style engine handle is null");
        crate::stage_thread::join_frame_for_style_engine_entrance(self, entry);
        // SAFETY: Guaranteed by the caller.
        let home = unsafe { Box::from_raw(self.0.cast::<StyleEngineHome>()) };
        // SAFETY: The home owned the engine, which `create` leaked into it.
        unsafe { Box::from_raw(home.engine.as_ptr()) }
    }

    /// A handle that names no home, only the address `address` identifies an engine by.
    #[cfg(test)]
    pub(crate) fn for_test(address: usize) -> Self {
        Self(address as *mut c_void)
    }

    /// The address that identifies the engine to the frame in flight.
    pub fn address(self) -> usize {
        self.0 as usize
    }

    /// The engine, for the main thread, which enters it at `entry`: waits for a frame in flight
    /// that reaches the engine first.
    ///
    /// # Safety
    ///
    /// The handle must name a live engine, and no other borrow of the engine may be live while the
    /// returned one is used.
    pub(crate) unsafe fn enter<'a>(self, entry: &'static str) -> &'a mut StyleEngine {
        self.bring_home(entry);
        // SAFETY: Guaranteed by the caller.
        unsafe { self.engine() }
    }

    /// Brings the engine home for the main thread, which is about to enter it at `entry` (as
    /// [`Self::enter`] does) once it has done what it does before.
    pub(crate) fn bring_home(self, entry: &'static str) {
        crate::stage_thread::join_frame_for_style_engine_entrance(self, entry);
    }

    /// Like [`Self::enter`], for an entrance that only reads what a published record holds, which
    /// the record keeps as it is whatever else the engine does.
    ///
    /// # Safety
    ///
    /// As for [`Self::enter`].
    pub(crate) unsafe fn enter_to_read_records<'a>(self, entry: &'static str) -> &'a StyleEngine {
        crate::stage_thread::join_frame_for_style_engine_entrance_to_read_records(self, entry);
        // SAFETY: Guaranteed by the caller.
        unsafe { self.engine() }
    }

    /// The engine of an engine that runs only for a replay, which no frame is ever in flight for.
    ///
    /// # Safety
    ///
    /// The handle must name a live engine made by `style_engine_create_for_replay`, and no other
    /// borrow of the engine may be live while the returned one is used.
    pub unsafe fn for_replay<'a>(self) -> &'a mut StyleEngine {
        // SAFETY: Guaranteed by the caller.
        unsafe { self.engine() }
    }

    /// # Safety
    ///
    /// As for [`Self::enter`].
    unsafe fn engine<'a>(self) -> &'a mut StyleEngine {
        assert!(!self.is_null(), "style engine handle is null");
        // SAFETY: Guaranteed by the caller.
        unsafe { &mut *(*self.0.cast::<StyleEngineHome>()).engine.as_ptr() }
    }
}
