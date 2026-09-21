/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The font service's half of the stage seals.
//!
//! Fonts are the one purity exception the design grants a render stage: a shared, thread-safe
//! resource service. `Gfx::FontCascadeList` is not one. Its lookup writes four unsynchronized
//! caches, and it can resolve a pending web face, which enters the document, starts a fetch and
//! arms an event-loop timer. The pipeline reads a frozen snapshot of it instead
//! (`libgfx_rust::font::FrozenFontList`), and nothing in a stage names the live list any more.
//!
//! The same is true of a system fallback *miss*: the answer has to come from the UI process, and
//! reaching it on the document thread's connection is what `WebView::RendererFontService` exists
//! to avoid. A miss that takes the old route reports itself as well.
//!
//! Neither the layout seal nor the paint seal would notice if something did, because both count
//! only calls through LibWeb's own host callback tables and these go through LibGfx. So LibGfx
//! reports them instead, through a hook installed here the first time a stage begins.

pub(crate) fn install_once() {
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        // LibWeb's copy of the graphics crate, reporting itself to the one store LibGfx keeps.
        libgfx_rust::ladybird_gfx_register_rust_crate_copy();
        libgfx_rust::font::set_host_reaching_call_hook(report);
    });
}

/// The name arrives as bytes rather than as a `&'static str` because the hook is held by LibGfx's
/// C++ side: the graphics crate is compiled into two libraries and must keep no state of its own.
extern "C" fn report(name: *const u8, length: usize) {
    // SAFETY: LibGfx passes back the bytes of the `&'static str` literal a call site named, so
    // they are valid UTF-8 and outlive the process.
    let callback: &'static str =
        unsafe { std::str::from_utf8_unchecked(std::slice::from_raw_parts::<'static, u8>(name, length)) };
    crate::layout::seal::note_host_call(crate::layout::seal::a_layout_pass_is_running(), callback);
    crate::painting::seal::note_host_call(callback);
}
