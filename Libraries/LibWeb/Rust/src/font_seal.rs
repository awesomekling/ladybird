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
    INSTALLED.call_once(|| libgfx_rust::font::set_host_reaching_call_hook(report));
}

fn report(callback: &'static str) {
    crate::layout::seal::note_host_call(crate::layout::seal::a_layout_pass_is_running(), callback);
    crate::painting::seal::note_host_call(callback);
}
