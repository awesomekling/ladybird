/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The host calls that free or notify what a freed row held. The imports are private to this
//! module, so the only way to reach them is a wrapper that takes the main thread capability.

use std::ffi::c_void;

unsafe extern "C" {
    fn ladybird_layout_owned_image_provider_destroy(provider: *mut c_void);
    fn ladybird_layout_image_observers_destroy(observers: *mut c_void);
    fn ladybird_layout_owned_image_provider_notify_detach(provider: *mut c_void);
}

/// The host makes no shells any more, so it has none to destroy; only a test binds a row one.
pub(crate) fn destroy_shell(_: &crate::stage::MainThread, _shell: *mut c_void) {}

/// An image provider a row owns outlives no row: the arena hands it back when the row is freed and
/// the host deletes it. Deleting one never reads a layout node, so the order against the shells is
/// free.
pub(crate) fn destroy_owned_image_provider(_: &crate::stage::MainThread, provider: *mut c_void) {
    if provider.is_null() {
        return;
    }
    // SAFETY: The arena has already freed the provider's row, and deleting a provider never
    // re-enters the arena.
    crate::layout::tree_build_seal::note_host_call("owned_image_provider_destroy");
    unsafe { ladybird_layout_owned_image_provider_destroy(provider) };
}

/// An image observer set a row holds outlives no row, and deleting one never reads a layout node.
pub(crate) fn destroy_image_observers(_: &crate::stage::MainThread, observers: *mut c_void) {
    if observers.is_null() {
        return;
    }
    // SAFETY: The arena has already freed the set's row, and deleting a set never re-enters the
    // arena.
    crate::layout::tree_build_seal::note_host_call("image_observers_destroy");
    unsafe { ladybird_layout_image_observers_destroy(observers) };
}

/// Tells the provider a row owns that the row is leaving the layout tree. An element's provider
/// outlives its box and keeps nothing about it, so only a provider a row owns hears about this.
pub(crate) fn notify_owned_image_provider_of_detach(_: &crate::stage::MainThread, provider: *mut c_void) {
    if provider.is_null() {
        return;
    }
    // SAFETY: The provider belongs to a live row and the notification does not re-enter the arena.
    crate::layout::tree_build_seal::note_host_call("owned_image_provider_notify_detach");
    unsafe { ladybird_layout_owned_image_provider_notify_detach(provider) };
}
