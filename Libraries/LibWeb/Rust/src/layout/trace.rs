/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::LayoutNodeArena;
use super::debug_text::{AppendBytes, DebugText, DescribeDomNode};
use super::formatting_context::{FormattingContextType, LayoutMode, LayoutPurpose};
use super::node_data::{NodeFlag, NodeKind, NodeSlotId};
use crate::render_owner::ArenaChange;
use std::cell::RefCell;
use std::ffi::c_void;

struct Trace {
    /// A line per traced event, each naming the box it is about as the event happened: a later mutation may remove
    /// the box or reuse its slot before JavaScript takes the trace.
    text: DebugText,
    depth: usize,
    /// How many of the DOM nodes the text names the layout commits handed the host to name.
    handed_to_host: usize,
}

/// Observation belongs to the document, not to a single pass: geometry reads and
/// style stabilization can cause several passes within one measured mutation.
#[derive(Default)]
pub(crate) struct LayoutTrace(RefCell<Option<Trace>>);

pub(super) struct Scope<'a>(&'a LayoutTrace);

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        if let Some(trace) = self.0.0.borrow_mut().as_mut() {
            trace.depth -= 1;
        }
    }
}

impl LayoutTrace {
    pub(crate) fn begin(&self) {
        debug_assert!(self.0.borrow().as_ref().is_none_or(|trace| trace.depth == 0));
        *self.0.borrow_mut() = Some(Trace {
            text: DebugText::default(),
            depth: 0,
            handed_to_host: 0,
        });
    }

    /// The DOM nodes the trace named since this was asked last, which a layout commit hands the host to name while
    /// they are live, with the number of the first among all the trace names.
    pub(crate) fn take_nodes_to_name(&self) -> TracedNodes {
        let mut trace = self.0.borrow_mut();
        let Some(trace) = trace.as_mut() else {
            return TracedNodes::default();
        };
        let first = trace.handed_to_host;
        trace.handed_to_host = trace.text.node_count();
        TracedNodes {
            first,
            nodes: trace.text.nodes_from(first).collect(),
        }
    }

    /// What was traced since the trace began.
    pub(crate) fn text(&self) -> DebugText {
        let trace = self.0.borrow();
        debug_assert!(
            trace.as_ref().is_none_or(|trace| trace.depth == 0),
            "incomplete layout trace"
        );
        trace.as_ref().map(|trace| trace.text.clone()).unwrap_or_default()
    }

    pub(crate) fn end(&self) {
        *self.0.borrow_mut() = None;
    }

    fn scope(
        &self,
        prefix: &'static str,
        owner: Option<(&LayoutNodeArena, NodeSlotId)>,
        text: impl FnOnce() -> String,
    ) -> Option<Scope<'_>> {
        let mut state = self.0.borrow_mut();
        let trace = state.as_mut()?;
        let line = trace.text.text();
        line.push_str(&"  ".repeat(trace.depth));
        line.push_str(prefix);
        if let Some((arena, owner)) = owner {
            name_owner(&mut trace.text, arena, owner);
        }
        let line = trace.text.text();
        line.push_str(&text());
        line.push('\n');
        trace.depth += 1;
        Some(Scope(self))
    }
}

impl LayoutNodeArena {
    pub(crate) fn layout_trace(&self) -> &LayoutTrace {
        &self.layout_trace
    }

    pub(super) fn trace_pass(&self, partial_root: Option<NodeSlotId>) -> Option<Scope<'_>> {
        match partial_root {
            Some(root) => self
                .layout_trace
                .scope("layout PARTIAL ", Some((self, root)), String::new),
            None => self.layout_trace.scope("layout FULL", None, String::new),
        }
    }

    pub(super) fn trace_run(
        &self,
        root: NodeSlotId,
        fc_type: FormattingContextType,
        purpose: LayoutPurpose,
        mode: LayoutMode,
        action: impl FnOnce() -> &'static str,
    ) -> Option<Scope<'_>> {
        self.layout_trace.scope("", Some((self, root)), || {
            let context = match fc_type {
                FormattingContextType::Block => "block",
                FormattingContextType::Inline => "inline",
                FormattingContextType::Flex => "flex",
                FormattingContextType::Grid => "grid",
                FormattingContextType::Table => "table",
                FormattingContextType::Svg => "svg",
                FormattingContextType::ReplacedWithChildren => "replaced-with-children",
                FormattingContextType::InternalReplaced => "internal-replaced",
                FormattingContextType::InternalDummy => "internal-dummy",
            };
            let measurement = match (purpose.is_measurement(), mode) {
                (false, LayoutMode::Normal) => "",
                (false, LayoutMode::IntrinsicSizing) => " (intrinsic)",
                (true, LayoutMode::Normal) => " (measurement)",
                (true, LayoutMode::IntrinsicSizing) => " (measurement, intrinsic)",
            };
            format!("/{context}{measurement} {}", action())
        })
    }
}

fn name_owner(text: &mut DebugText, arena: &LayoutNodeArena, root: NodeSlotId) {
    let data = arena.data(root);
    let kind = data.kind.get();
    if kind == NodeKind::Viewport {
        text.text().push_str("@viewport");
    } else if kind == NodeKind::TextNode && data.flags.get() & NodeFlag::Anonymous as u32 == 0 {
        text.text().push_str("TextNode<#text>");
    } else {
        text.push_box(arena, root);
    }
}

/// DOM nodes a layout trace named, from the `first`th of all it named.
#[derive(Default)]
pub(crate) struct TracedNodes {
    first: usize,
    nodes: Vec<u32>,
}

/// How the host names the DOM nodes a layout trace mentions, and the names it gave them so far, by their number among
/// all the trace names.
pub(crate) struct LayoutTraceNames {
    context: *mut c_void,
    describe: DescribeDomNode,
    names: Vec<Option<String>>,
}

impl LayoutTraceNames {
    fn describe(&self, node: u32, text: &mut String) {
        let mut name = DebugText::default();
        name.push_dom_node(node);
        // SAFETY: The host keeps the callback callable with its context while the document traces its layout.
        text.push_str(&unsafe { name.finish_with_host(self.context, self.describe) });
    }
}

/// On the document thread, as it takes a layout commit in: names the DOM nodes the layout trace named, while they are
/// live.
pub(crate) fn name_traced_nodes(main_thread: &crate::stage::MainThread, traced: TracedNodes) {
    if traced.nodes.is_empty() {
        return;
    }
    let Some(host_tables) = main_thread.host_tables() else {
        return;
    };
    let mut trace_names = host_tables.layout_trace_names.borrow_mut();
    let Some(trace_names) = trace_names.as_mut() else {
        return;
    };
    let end = traced.first + traced.nodes.len();
    if trace_names.names.len() < end {
        trace_names.names.resize(end, None);
    }
    for (index, node) in (traced.first..end).zip(traced.nodes) {
        let mut name = String::new();
        trace_names.describe(node, &mut name);
        trace_names.names[index] = Some(name);
    }
}

/// Begins tracing the layout of the document whose arena `arena` names, for tests: each layout pass and formatting
/// context run from now on leaves a line, naming the box it is about as the host's `describe_node` describes the DOM
/// node the box was built for.
///
/// # Safety
///
/// `arena` must be a live arena handle on the document thread, and `describe_node` must be callable with `context`
/// until the trace is taken.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_begin_layout_trace(
    arena: *mut c_void,
    context: *mut c_void,
    describe_node: DescribeDomNode,
) {
    // SAFETY: Guaranteed by the caller.
    let host_tables = unsafe { super::HostTables::from_handle(arena) };
    *host_tables.layout_trace_names.borrow_mut() = Some(LayoutTraceNames {
        context,
        describe: describe_node,
        names: Vec::new(),
    });
    // SAFETY: As above.
    let document = unsafe { super::ArenaHandle::document_of(arena) };
    crate::render_owner::send_arena_change(document, ArenaChange::BeginLayoutTrace);
}

/// Ends tracing the layout of the document whose arena `arena` names, and hands `append_text` the `text` the owner
/// traced, if it traced any.
///
/// # Safety
///
/// `arena` must be a live arena handle on the document thread, and `append_text` must be callable with `context` for
/// the duration of this call.
pub(crate) unsafe fn end_and_hand_over(
    arena: *mut c_void,
    text: Option<DebugText>,
    context: *mut c_void,
    append_text: AppendBytes,
) {
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { super::ArenaHandle::document_of(arena) };
    crate::render_owner::send_arena_change(document, ArenaChange::EndLayoutTrace);
    // SAFETY: As above.
    let trace_names = unsafe { super::HostTables::from_handle(arena) }
        .layout_trace_names
        .take();
    let (Some(text), Some(trace_names)) = (text, trace_names) else {
        return;
    };
    // A node no commit named is named now, if it is still live.
    let mut index = 0;
    let text = text.finish(|node, text| {
        match trace_names.names.get(index).and_then(Option::as_deref) {
            Some(name) => text.push_str(name),
            None => trace_names.describe(node, text),
        }
        index += 1;
    });
    // SAFETY: Guaranteed by the caller.
    unsafe { append_text(context, text.as_ptr(), text.len()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn take(trace: &LayoutTrace) -> String {
        let text = trace.text().finish(|_, _| {});
        trace.end();
        text
    }

    #[test]
    fn disabled_trace_does_not_construct_labels() {
        let trace = LayoutTrace::default();
        assert!(trace.scope("", None, || panic!("disabled observation")).is_none());
        assert_eq!(take(&trace), "");
    }

    #[test]
    fn preserves_nesting_repeated_runs_and_multiple_passes() {
        let trace = LayoutTrace::default();
        trace.begin();
        {
            let _pass = trace.scope("layout FULL", None, String::new);
            let _run = trace.scope("", None, || "@viewport/block RUN (cache=bypass)".into());
            {
                let _child = trace.scope("", None, || "#child/block REUSE SUBTREE".into());
            }
            let _child = trace.scope("", None, || "#child/block RUN (cache=miss)".into());
        }
        {
            let _pass = trace.scope("layout PARTIAL ", None, || "#boundary".into());
        }
        assert_eq!(
            take(&trace),
            "layout FULL\n  @viewport/block RUN (cache=bypass)\n    #child/block REUSE SUBTREE\n    #child/block RUN (cache=miss)\nlayout PARTIAL #boundary\n"
        );
        assert!(trace.scope("", None, || panic!("take must disable tracing")).is_none());
        assert_eq!(take(&trace), "");
    }

    #[test]
    fn begin_discards_previous_events() {
        let trace = LayoutTrace::default();
        trace.begin();
        drop(trace.scope("", None, || "old pass".into()));
        trace.begin();
        assert_eq!(take(&trace), "");
    }
}
