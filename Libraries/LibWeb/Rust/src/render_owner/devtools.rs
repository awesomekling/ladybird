/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The reads of a document's render state that only Internals and the WebContent debug requests make, for tests and
//! debugging. They reach the owner as [`Query::DevTools`], and only the entry points here can name a [`DevToolsQuery`]:
//! nothing the engine does asks the owner one.

use super::{Answer, DocumentId, Query};
use crate::layout::debug_text::{AppendBytes, DebugText, DescribeDomNode};
use crate::layout::node_data::NodeSlotId;
use crate::layout::update_layout::FfiLayoutTreeBuildStats;
use crate::layout::{ArenaHandle, LayoutNodeArena};
use std::ffi::c_void;

/// A read of a document's render state for tests and debugging, which [`Query::DevTools`] asks.
#[derive(Clone, Copy, Debug)]
pub(crate) enum DevToolsQuery {
    /// How many layout passes, tree builds and arena measurements the document's layout has run.
    LayoutCounts,
    /// How many rows of the layout subtree `root` heads carry a pre-order label no greater than the row before them.
    PreOrderLabelViolations { root: NodeSlotId },
    /// What the layout trace traced since it began.
    LayoutTrace,
    /// Where the stacking context structure below `viewport` differs from what paint preparation recorded.
    StackingContextVerification { viewport: NodeSlotId },
    /// The style record a row holds.
    NodeStyleRecord(NodeSlotId),
    /// The document's stacking context tree.
    StackingContextTree,
}

/// The answer to a [`DevToolsQuery`].
#[derive(Debug)]
pub(crate) enum DevToolsAnswer {
    LayoutCounts(LayoutCounts),
    Number(u64),
    Trace(DebugText),
    Report(String),
}

/// How many layout passes and tree builds a document's layout has run.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct LayoutCounts {
    partial_layouts: u64,
    full_layouts: u64,
    tree_builds: FfiLayoutTreeBuildStats,
    arena: FfiArenaCounts,
}

/// How many slots and measurements a document's layout arena holds or has taken, for tests.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct FfiArenaCounts {
    pub live_slots: u64,
    pub pre_order_relabels: u64,
    pub intrinsic_measurements: u64,
    pub intrinsic_inline_measurements: u64,
    pub table_cell_measurement_cache_misses: u64,
    pub retained_inline_items: u64,
}

impl DevToolsQuery {
    /// The answer where the owner holds no render state to answer from.
    pub(super) fn left_to_host(self) -> DevToolsAnswer {
        match self {
            Self::LayoutCounts => DevToolsAnswer::LayoutCounts(LayoutCounts::default()),
            Self::PreOrderLabelViolations { .. } | Self::NodeStyleRecord(_) => DevToolsAnswer::Number(0),
            Self::LayoutTrace | Self::StackingContextTree => DevToolsAnswer::Trace(DebugText::default()),
            Self::StackingContextVerification { .. } => DevToolsAnswer::Report(String::new()),
        }
    }

    /// Answers the query from the arena of `state` and the layout scratch beside it.
    pub(super) fn answer(self, state: &mut ArenaHandle) -> DevToolsAnswer {
        let (arena, scratch) = state.arena_and_scratch();
        match self {
            Self::LayoutCounts => DevToolsAnswer::LayoutCounts(LayoutCounts {
                partial_layouts: arena.partial_layout_count(),
                full_layouts: arena.full_layout_count(),
                tree_builds: arena.layout_tree_build_stats(),
                arena: FfiArenaCounts {
                    live_slots: u64::from(arena.live_slot_count()),
                    pre_order_relabels: arena.pre_order_relabel_count(),
                    intrinsic_measurements: arena.intrinsic_measurement_count(),
                    intrinsic_inline_measurements: arena.intrinsic_inline_measurement_count(),
                    table_cell_measurement_cache_misses: arena.table_cell_measurement_cache_miss_count(),
                    retained_inline_items: scratch.retained_inline_item_count(),
                },
            }),
            Self::PreOrderLabelViolations { root } => DevToolsAnswer::Number(pre_order_label_violations(arena, root)),
            Self::LayoutTrace => DevToolsAnswer::Trace(arena.layout_trace().text()),
            Self::StackingContextVerification { viewport } => DevToolsAnswer::Report(
                crate::painting::stacking_context::verify::verification_report(arena, viewport),
            ),
            Self::NodeStyleRecord(row) => DevToolsAnswer::Number(if arena.slot_is_live(row) {
                arena.node_style_record(row)
            } else {
                0
            }),
            Self::StackingContextTree => {
                DevToolsAnswer::Trace(crate::painting::stacking_context::dump::stacking_context_tree(arena))
            }
        }
    }
}

fn pre_order_label_violations(arena: &LayoutNodeArena, root: NodeSlotId) -> u64 {
    if !arena.slot_is_live(root) {
        return 0;
    }
    let mut violation_count = 0u64;
    let mut previous_label: Option<u64> = None;
    arena.for_each_node_in_layout_subtree_in_pre_order(root, |node| {
        let label = arena.node_pre_order_label(node);
        if previous_label.is_some_and(|previous| label <= previous) {
            violation_count += 1;
        }
        previous_label = Some(label);
    });
    violation_count
}

/// Asks the owner `query` about the document whose arena the calling document thread names as `arena`, once the frame
/// in flight that owns the arena, if any, has been taken back.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
unsafe fn ask(arena: *mut c_void, query: DevToolsQuery) -> DevToolsAnswer {
    // SAFETY: Guaranteed by the caller.
    match unsafe { super::ask_about(arena, Query::DevTools(query)) } {
        Answer::DevTools(answer) => answer,
        _ => query.left_to_host(),
    }
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
unsafe fn layout_counts(arena: *mut c_void) -> LayoutCounts {
    // SAFETY: Guaranteed by the caller.
    match unsafe { ask(arena, DevToolsQuery::LayoutCounts) } {
        DevToolsAnswer::LayoutCounts(counts) => counts,
        _ => LayoutCounts::default(),
    }
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_partial_layout_count(arena: *mut c_void) -> u64 {
    // SAFETY: Guaranteed by the caller.
    unsafe { layout_counts(arena) }.partial_layouts
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_full_layout_count(arena: *mut c_void) -> u64 {
    // SAFETY: Guaranteed by the caller.
    unsafe { layout_counts(arena) }.full_layouts
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_layout_tree_build_stats(arena: *mut c_void) -> FfiLayoutTreeBuildStats {
    // SAFETY: Guaranteed by the caller.
    unsafe { layout_counts(arena) }.tree_builds
}

/// Hands `append_text` the stacking context tree of `document`, with the DOM node of each box it names as
/// `describe_node` describes it.
///
/// # Safety
///
/// `describe_node` and `append_text` must be callable with `context` for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn render_owner_dump_stacking_context_tree(
    document: DocumentId,
    context: *mut c_void,
    describe_node: DescribeDomNode,
    append_text: AppendBytes,
) {
    if !document.is_valid() {
        return;
    }
    super::join_frame_of(document);
    let Answer::DevTools(DevToolsAnswer::Trace(text)) =
        super::ask_owner(document, Query::DevTools(DevToolsQuery::StackingContextTree))
    else {
        return;
    };
    // SAFETY: Guaranteed by the caller.
    let text = unsafe { text.finish_with_host(context, describe_node) };
    // SAFETY: As above.
    unsafe { append_text(context, text.as_ptr(), text.len()) };
}

/// The counts of the layout arena of `document`.
#[unsafe(no_mangle)]
pub extern "C" fn render_owner_arena_counts(document: DocumentId) -> FfiArenaCounts {
    if !document.is_valid() {
        return FfiArenaCounts::default();
    }
    super::join_frame_of(document);
    match super::ask_owner(document, Query::DevTools(DevToolsQuery::LayoutCounts)) {
        Answer::DevTools(DevToolsAnswer::LayoutCounts(counts)) => counts.arena,
        _ => FfiArenaCounts::default(),
    }
}

/// How many rows of the layout subtree `root` heads come in pre-order after a row with a label not below theirs.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_pre_order_label_violation_count(arena: *mut c_void, root: NodeSlotId) -> u64 {
    // SAFETY: Guaranteed by the caller.
    match unsafe { ask(arena, DevToolsQuery::PreOrderLabelViolations { root }) } {
        DevToolsAnswer::Number(count) => count,
        _ => 0,
    }
}

/// The style record a row holds.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_node_style_record(arena: *mut c_void, id: NodeSlotId) -> u64 {
    // SAFETY: Guaranteed by the caller.
    match unsafe { ask(arena, DevToolsQuery::NodeStyleRecord(id)) } {
        DevToolsAnswer::Number(record) => record,
        _ => 0,
    }
}

/// Ends tracing the layout of the document whose arena `arena` names, and hands `append_text` what was traced.
///
/// # Safety
///
/// `arena` must be a live arena handle on the document thread, and `append_text` must be callable with `context` for
/// the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_take_layout_trace(
    arena: *mut c_void,
    context: *mut c_void,
    append_text: AppendBytes,
) {
    // SAFETY: Guaranteed by the caller.
    let text = match unsafe { ask(arena, DevToolsQuery::LayoutTrace) } {
        DevToolsAnswer::Trace(text) => Some(text),
        _ => None,
    };
    // SAFETY: As above.
    unsafe { crate::layout::trace::end_and_hand_over(arena, text, context, append_text) };
}

/// # Safety
///
/// `arena` must be a live handle from `layout_arena_create`, used on the document thread;
/// `consume` copies the byte span synchronously.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_stacking_context_structure_verification_report(
    arena: *mut c_void,
    viewport: NodeSlotId,
    context: *mut c_void,
    consume: unsafe extern "C" fn(*mut c_void, *const u8, usize),
) {
    // SAFETY: Guaranteed by the caller.
    let DevToolsAnswer::Report(report) =
        (unsafe { ask(arena, DevToolsQuery::StackingContextVerification { viewport }) })
    else {
        return;
    };
    if !report.is_empty() {
        // SAFETY: The consumer copies the byte span synchronously.
        unsafe { consume(context, report.as_ptr(), report.len()) };
    }
}
