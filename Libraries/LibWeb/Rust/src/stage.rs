/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Capabilities separating the main thread from render pipeline stages.

use std::marker::PhantomData;

/// Proof that a call entered Rust from the document's main thread.
///
/// Render pipeline stage runners deliberately do not receive this token. Host callback wrappers
/// require it, making a callback from a stage a type error. The raw pointer marker makes the token
/// neither [`Send`] nor [`Sync`], so the proof cannot cross onto a render thread.
pub(crate) struct MainThread {
    not_send_or_sync: PhantomData<*const ()>,
}

mod private {
    pub trait FfiEntry {}
}

pub(crate) trait FfiEntry: private::FfiEntry {}

impl MainThread {
    /// Mint a main-thread capability at an FFI entry point called by the document thread.
    ///
    /// # Safety
    ///
    /// The caller must be an FFI entry point whose C++ contract requires the document thread.
    unsafe fn from_ffi_entry() -> Self {
        Self {
            not_send_or_sync: PhantomData,
        }
    }
}

/// Mint a main-thread capability for a designated FFI entry module.
///
/// The sealed marker must be constructed by the module that owns it, so a stage module cannot
/// call this function safely with a marker of its own.
///
/// ```compile_fail
/// struct StageEntry;
/// impl libweb_rust::stage::FfiEntry for StageEntry {}
/// ```
pub(crate) unsafe fn from_ffi_entry(_: &impl FfiEntry) -> MainThread {
    // SAFETY: Implementations are restricted below to marker types whose values can only be
    // constructed in their designated FFI entry module.
    unsafe { MainThread::from_ffi_entry() }
}

macro_rules! ffi_entry {
    ($entry:path) => {
        impl private::FfiEntry for $entry {}
        impl FfiEntry for $entry {}
    };
}

ffi_entry!(crate::layout::formatting_context::MainThreadFfiEntry);
ffi_entry!(crate::layout::ArenaMainThreadFfiEntry);
ffi_entry!(crate::layout::UpdateMainThreadFfiEntry);
ffi_entry!(crate::painting::ffi::MainThreadFfiEntry);

const _: () = assert!(size_of::<MainThread>() == 0);

#[cfg(test)]
mod tests {
    use super::*;

    trait AmbiguousIfSend<A> {
        fn marker() {}
    }

    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

    #[test]
    fn main_thread_capability_is_zero_sized() {
        assert_eq!(size_of::<MainThread>(), 0);
    }

    #[test]
    fn main_thread_capability_is_not_send() {
        <MainThread as AmbiguousIfSend<_>>::marker();
    }
}
