/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

use super::LayoutNodeArena;
use super::formatting_context::{FormattingContextType, LayoutMode, LayoutPurpose};
use super::node_data::{NodeFlag, NodeKind, NodeSlotId};
use std::cell::RefCell;
use std::ffi::c_void;
use std::fmt::Write;

pub(crate) struct MainThreadFfiEntry {
    _private: (),
}

const MAIN_THREAD_FFI_ENTRY: MainThreadFfiEntry = MainThreadFfiEntry { _private: () };

type AppendText = unsafe extern "C" fn(*mut c_void, *const u8, usize);
type DescribeNode = unsafe extern "C" fn(*mut c_void, *mut c_void, AppendText);

struct Trace {
    describe_node: DescribeNode,
    lines: Vec<Line>,
    depth: usize,
}

/// One traced event: what it says, and the node it names, if any. The node is named once the
/// pass is over, since naming it can materialise its shell, which asks the document something.
struct Line {
    depth: usize,
    prefix: &'static str,
    owner: Option<NodeSlotId>,
    owner_name: Option<String>,
    text: String,
}

/// Observation belongs to the document, not to a single pass: geometry reads and
/// style stabilization can cause several passes within one measured mutation.
#[derive(Default)]
pub(crate) struct LayoutTrace(RefCell<Option<Trace>>);

pub(super) struct Scope<'a>(&'a LayoutTrace);

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        self.0.0.borrow_mut().as_mut().unwrap().depth -= 1;
    }
}

impl LayoutTrace {
    fn begin(&self, describe_node: DescribeNode) {
        assert!(self.0.borrow().as_ref().is_none_or(|trace| trace.depth == 0));
        *self.0.borrow_mut() = Some(Trace {
            describe_node,
            lines: Vec::new(),
            depth: 0,
        });
    }

    fn take(&self, main_thread: &crate::stage::MainThread, arena: &LayoutNodeArena) -> String {
        self.name_owners(main_thread, arena);
        let Some(trace) = self.0.borrow_mut().take() else {
            return String::new();
        };
        assert_eq!(trace.depth, 0, "incomplete layout trace");
        let mut text = String::new();
        for line in trace.lines {
            writeln!(
                text,
                "{}{}{}{}",
                "  ".repeat(line.depth),
                line.prefix,
                line.owner_name.unwrap_or_default(),
                line.text
            )
            .unwrap();
        }
        text
    }

    /// Names the nodes the traced events name. This runs once a pass is over, while the nodes the
    /// pass ran for are still live: a subsequent mutation may remove them or reuse their arena
    /// slots before JavaScript takes the trace.
    pub(super) fn name_owners(&self, main_thread: &crate::stage::MainThread, arena: &LayoutNodeArena) {
        let (describe, unnamed) = {
            let state = self.0.borrow();
            let Some(trace) = state.as_ref() else {
                return;
            };
            let unnamed: Vec<(usize, NodeSlotId)> = trace
                .lines
                .iter()
                .enumerate()
                .filter(|(_, line)| line.owner_name.is_none())
                .filter_map(|(index, line)| line.owner.map(|owner| (index, owner)))
                .collect();
            (trace.describe_node, unnamed)
        };
        let names: Vec<(usize, String)> = unnamed
            .into_iter()
            .map(|(index, owner)| (index, owner_name(main_thread, arena, owner, describe)))
            .collect();
        if let Some(trace) = self.0.borrow_mut().as_mut() {
            for (index, name) in names {
                trace.lines[index].owner_name = Some(name);
            }
        }
    }

    fn scope(
        &self,
        prefix: &'static str,
        owner: Option<NodeSlotId>,
        text: impl FnOnce() -> String,
    ) -> Option<Scope<'_>> {
        let mut state = self.0.borrow_mut();
        let trace = state.as_mut()?;
        let depth = trace.depth;
        trace.lines.push(Line {
            depth,
            prefix,
            owner,
            owner_name: None,
            text: text(),
        });
        trace.depth += 1;
        Some(Scope(self))
    }

    pub(super) fn pass(&self, partial_root: Option<NodeSlotId>) -> Option<Scope<'_>> {
        match partial_root {
            Some(root) => self.scope("layout PARTIAL ", Some(root), String::new),
            None => self.scope("layout FULL", None, String::new),
        }
    }

    pub(super) fn run(
        &self,
        root: NodeSlotId,
        fc_type: FormattingContextType,
        purpose: LayoutPurpose,
        mode: LayoutMode,
        action: impl FnOnce() -> &'static str,
    ) -> Option<Scope<'_>> {
        self.scope("", Some(root), || {
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

fn owner_name(
    main_thread: &crate::stage::MainThread,
    arena: &LayoutNodeArena,
    root: NodeSlotId,
    describe: DescribeNode,
) -> String {
    if arena.data(root).kind.get() == NodeKind::Viewport {
        return "@viewport".into();
    }
    unsafe extern "C" fn append(sink: *mut c_void, bytes: *const u8, length: usize) {
        // SAFETY: describe receives this live vector and supplies bytes valid for this call.
        unsafe { &mut *sink.cast::<Vec<u8>>() }.extend_from_slice(unsafe { std::slice::from_raw_parts(bytes, length) });
    }
    // A row nothing has materialised a shell for is named from the row, the way its shell would
    // describe itself, since materialising one would ask the document something mid-pass.
    let data = arena.data(root);
    if data.shell.get().is_null() {
        let kind = data.kind.get();
        if data.flags.get() & NodeFlag::Anonymous as u32 != 0 {
            return format!("{kind:?}(anonymous)");
        }
        if kind == NodeKind::TextNode {
            return format!("{kind:?}<#text>");
        }
    }
    let mut bytes = Vec::<u8>::new();
    // SAFETY: the traced run holds the arena and its shells alive; describe copies
    // the node's description synchronously without changing layout.
    unsafe { describe(arena.shell_if_live(main_thread, root), (&raw mut bytes).cast(), append) };
    String::from_utf8(bytes).expect("layout trace label must be UTF-8")
}

/// # Safety
/// The arena must be live. The callback must remain valid until tracing stops and
/// must synchronously describe its live node shell without mutating layout.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_begin_layout_trace(arena: *mut c_void, describe_node: DescribeNode) {
    unsafe { LayoutNodeArena::from_handle(arena) }
        .layout_trace
        .begin(describe_node);
}

/// # Safety
/// The arena must be live, and append_text must synchronously copy the supplied bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_take_layout_trace(
    arena: *mut c_void,
    context: *mut c_void,
    append_text: AppendText,
) {
    let main_thread = unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) };
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    let text = arena.layout_trace.take(&main_thread, arena);
    unsafe { append_text(context, text.as_ptr(), text.len()) };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn main_thread_for_test() -> crate::stage::MainThread {
        // SAFETY: Tests run on the thread that owns their arena.
        unsafe { crate::stage::from_ffi_entry(&MAIN_THREAD_FFI_ENTRY) }
    }

    unsafe extern "C" fn unused_description(_: *mut c_void, _: *mut c_void, _: AppendText) {
        panic!("no nodes to describe in this test");
    }

    #[test]
    fn disabled_trace_does_not_construct_labels() {
        let arena = LayoutNodeArena::new();
        let trace = LayoutTrace::default();
        assert!(trace.scope("", None, || panic!("disabled observation")).is_none());
        assert_eq!(trace.take(&main_thread_for_test(), &arena), "");
    }

    #[test]
    fn preserves_nesting_repeated_runs_and_multiple_passes() {
        let arena = LayoutNodeArena::new();
        let trace = LayoutTrace::default();
        trace.begin(unused_description);
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
            trace.take(&main_thread_for_test(), &arena),
            "layout FULL\n  @viewport/block RUN (cache=bypass)\n    #child/block REUSE SUBTREE\n    #child/block RUN (cache=miss)\nlayout PARTIAL #boundary\n"
        );
        assert!(trace.scope("", None, || panic!("take must disable tracing")).is_none());
        assert_eq!(trace.take(&main_thread_for_test(), &arena), "");
    }

    #[test]
    fn begin_discards_previous_events() {
        let arena = LayoutNodeArena::new();
        let trace = LayoutTrace::default();
        trace.begin(unused_description);
        drop(trace.scope("", None, || "old pass".into()));
        trace.begin(unused_description);
        assert_eq!(trace.take(&main_thread_for_test(), &arena), "");
    }
}
