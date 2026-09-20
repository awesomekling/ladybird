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

impl MainThread {
    /// Mint a main-thread capability at an FFI entry point called by the document thread.
    ///
    /// # Safety
    ///
    /// The caller must be an FFI entry point whose C++ contract requires the document thread.
    pub(crate) unsafe fn from_ffi_entry() -> Self {
        Self {
            not_send_or_sync: PhantomData,
        }
    }
}

const _: () = assert!(size_of::<MainThread>() == 0);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_thread_capability_is_zero_sized() {
        assert_eq!(size_of::<MainThread>(), 0);
    }
}
