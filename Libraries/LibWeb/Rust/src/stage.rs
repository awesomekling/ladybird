/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Capabilities separating the main thread from render pipeline stages.

use crate::layout::HostTables;
use std::ffi::c_void;
use std::marker::PhantomData;

/// Proof that a call entered Rust from the document's main thread, and the way to the host that
/// document's arena answers to.
///
/// Render pipeline stage runners deliberately do not receive this token. Host callback wrappers
/// require it, and the host tables are reached only through it, making a callback from a stage a
/// type error. The raw pointer marker makes the token neither [`Send`] nor [`Sync`], so the proof
/// cannot cross onto a render thread.
pub(crate) struct MainThread<'host> {
    host_tables: Option<&'host HostTables>,
    not_send_or_sync: PhantomData<*const ()>,
}

mod private {
    pub trait FfiEntry {}
}

pub(crate) trait FfiEntry: private::FfiEntry {}

impl<'host> MainThread<'host> {
    /// Mint a main-thread capability at an FFI entry point called by the document thread.
    ///
    /// # Safety
    ///
    /// The caller must be an FFI entry point whose C++ contract requires the document thread.
    unsafe fn from_ffi_entry(host_tables: Option<&'host HostTables>) -> Self {
        Self {
            host_tables,
            not_send_or_sync: PhantomData,
        }
    }

    /// Mint a main-thread capability for a unit test, which runs on the thread that owns its arena.
    /// An arena a test makes has no host.
    #[cfg(test)]
    pub(crate) fn for_test() -> Self {
        Self {
            host_tables: None,
            not_send_or_sync: PhantomData,
        }
    }

    /// Mint a main-thread capability for a unit test whose arena answers to `host_tables`.
    #[cfg(test)]
    pub(crate) fn for_test_with_host(host_tables: &'host HostTables) -> Self {
        Self {
            host_tables: Some(host_tables),
            not_send_or_sync: PhantomData,
        }
    }

    /// The host tables of the arena the entry was called for, or none for an entry that names no
    /// arena.
    pub(crate) fn host_tables(&self) -> Option<&'host HostTables> {
        self.host_tables
    }
}

/// Mint a main-thread capability for a designated FFI entry module, for an entry called on the
/// arena `arena_handle` names.
///
/// The sealed marker must be constructed by the module that owns it, so a stage module cannot
/// call this function safely with a marker of its own. A module whose code a stage reaches keeps
/// its marker in a `main_thread_entries` child that holds only the entries minting with it, and
/// keeps those entries private, so its stage code can neither mint nor call an entry that does.
///
/// # Safety
///
/// The caller must be an FFI entry point whose C++ contract requires the document thread, and
/// `arena_handle` must come from `layout_arena_create` and outlive the capability.
///
/// ```compile_fail
/// struct StageEntry;
/// impl libweb_rust::stage::FfiEntry for StageEntry {}
/// ```
///
/// ```compile_fail
/// fn move_to_worker(_: impl Send) {}
/// fn main_thread_entry(token: libweb_rust::stage::MainThread) {
///     move_to_worker(token);
/// }
/// ```
pub(crate) unsafe fn from_ffi_entry<'host>(_: &impl FfiEntry, arena_handle: *mut c_void) -> MainThread<'host> {
    // SAFETY: Implementations are restricted below to marker types whose values can only be
    // constructed in their designated FFI entry module, and the caller vouches for the handle.
    unsafe { MainThread::from_ffi_entry(Some(HostTables::from_handle(arena_handle))) }
}

/// Mint a main-thread capability for a designated FFI entry module, for an entry that names no
/// arena and so reaches no host tables.
///
/// # Safety
///
/// As for [`from_ffi_entry`].
pub(crate) unsafe fn from_ffi_entry_without_arena(_: &impl FfiEntry) -> MainThread<'static> {
    // SAFETY: As above.
    unsafe { MainThread::from_ffi_entry(None) }
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
ffi_entry!(crate::layout::TreeBuildMainThreadFfiEntry);
ffi_entry!(crate::layout::TextQueriesMainThreadFfiEntry);
ffi_entry!(crate::layout::TraceMainThreadFfiEntry);
ffi_entry!(crate::painting::display_list::dump::MainThreadFfiEntry);
ffi_entry!(crate::painting::ffi::MainThreadFfiEntry);
ffi_entry!(crate::painting::layout_tree_dump::MainThreadFfiEntry);
ffi_entry!(crate::painting::stacking_context::dump::MainThreadFfiEntry);

#[cfg(test)]
mod tests {
    use super::*;

    trait AmbiguousIfSend<A> {
        fn marker() {}
    }

    impl<T: ?Sized> AmbiguousIfSend<()> for T {}
    impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}

    #[test]
    fn main_thread_capability_is_not_send() {
        <MainThread as AmbiguousIfSend<_>>::marker();
    }
}
