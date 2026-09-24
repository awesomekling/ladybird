/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The layout update: the style and layout stabilization loop a document runs before it can
//! answer geometry queries or paint. The steps that touch the DOM stay on the C++ host and
//! answer through a callback table registered once per arena. The loop runs as one stage and
//! joins the document thread for those steps.

use super::LayoutNodeArena;
use super::formatting_context::{
    PendingLayoutCommit, commit_root_layout_to_arena, commit_subtree_layout_to_arena, compute_root_layout,
    compute_subtree_layout_fragments, prepare_root_layout_from_sources, read_viewport_propagation_facts,
};
use super::layout_node_arena::{EnrolledContentSources, apply_enrolled_content_sources, read_enrolled_content_sources};
use super::node_data::NodeSlotId;
use super::node_facts;
use super::partial_relayout::FfiPartialRelayoutHostFacts;
use super::tree_builder::FfiLayoutTreeBuildOutcome;
use super::viewport_propagation::FfiViewportPropagationFacts;
use crate::abort_on_panic;
use crate::css::ffi_support::FfiUtf16View;
use std::ffi::c_void;
use std::time::Instant;

mod main_thread_entries;

pub(crate) use main_thread_entries::MainThreadFfiEntry;

/// The document-side steps of a layout update. Each callback receives the registered
/// `context`, the owning document, first and answers synchronously; any of them may run the
/// layout update of another document, so the loop holds no arena borrow across a call.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiLayoutUpdateHostCallbacks {
    pub context: *mut c_void,
    pub connected_element_count: unsafe extern "C" fn(*mut c_void) -> u32,
    pub update_style: unsafe extern "C" fn(*mut c_void),
    pub process_pending_list_item_renumbers: unsafe extern "C" fn(*mut c_void),
    pub process_pending_top_layer_layout_changes: unsafe extern "C" fn(*mut c_void),
    pub document_facts: unsafe extern "C" fn(*mut c_void) -> FfiLayoutUpdateDocumentFacts,
    pub needs_style_update_after_layout: unsafe extern "C" fn(*mut c_void) -> bool,
    pub prepare_for_rendering: unsafe extern "C" fn(*mut c_void),
    /// Builds or updates the layout tree and installs its viewport as the document's layout root.
    pub build_layout_tree: unsafe extern "C" fn(*mut c_void) -> FfiLayoutTreeBuildOutcome,
    /// True when stale list-item counters marked more of the tree for a rebuild.
    pub reconcile_stale_list_item_counters_after_tree_build: unsafe extern "C" fn(*mut c_void) -> bool,
    /// Refreshes what derives from committed layout; the flag says whether the tree changed.
    pub after_layout_commit: unsafe extern "C" fn(*mut c_void, bool),
    pub note_full_layouts_performed: unsafe extern "C" fn(*mut c_void, u64),
    pub evaluate_pending_container_queries: unsafe extern "C" fn(*mut c_void),
    pub record_stabilization_bound_failure: unsafe extern "C" fn(*mut c_void),
}

/// What the loop needs to know about the document at one point in time. Every host call can
/// change these, so the loop asks again after each one it depends on.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiLayoutUpdateDocumentFacts {
    /// The document is its navigable's active document; an inactive document counts as laid out.
    pub document_is_active: bool,
    /// The document node or one of its descendants needs a layout tree update.
    pub document_needs_layout_tree_build: bool,
    pub container_query_evaluation_is_pending: bool,
    /// A top layer membership change or zone rebuild is waiting for the next pass.
    pub top_layer_work_pending: bool,
    pub should_collect_devtools_layout_data: bool,
    pub document_in_quirks_mode: bool,
    pub viewport_inline_size_raw: i32,
    pub viewport_block_size_raw: i32,
}

/// What one layout update was asked for.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiLayoutUpdateInputs {
    pub reason_is_inspect_devtools_layout_data: bool,
    /// A document hosting template contents never needs layout.
    pub is_template_contents_document: bool,
    /// The update reason's name, read only when tracing is enabled.
    pub reason_name: FfiUtf16View,
}

/// Confinement report of the most recent layout tree build, for tests observing whether a
/// partial rebuild stayed inside its rebuilt subtrees.
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FfiLayoutTreeBuildStats {
    pub builds: u64,
    pub last_build_rebuilt_subtree_roots: u64,
    pub last_build_escaped_rebuild_roots: bool,
}

#[derive(Clone, Copy)]
pub(crate) struct LayoutUpdateHost {
    context: *mut c_void,
    connected_element_count: unsafe extern "C" fn(*mut c_void) -> u32,
    update_style: unsafe extern "C" fn(*mut c_void),
    process_pending_list_item_renumbers: unsafe extern "C" fn(*mut c_void),
    process_pending_top_layer_layout_changes: unsafe extern "C" fn(*mut c_void),
    document_facts: unsafe extern "C" fn(*mut c_void) -> FfiLayoutUpdateDocumentFacts,
    needs_style_update_after_layout: unsafe extern "C" fn(*mut c_void) -> bool,
    prepare_for_rendering: unsafe extern "C" fn(*mut c_void),
    build_layout_tree: unsafe extern "C" fn(*mut c_void) -> FfiLayoutTreeBuildOutcome,
    reconcile_stale_list_item_counters_after_tree_build: unsafe extern "C" fn(*mut c_void) -> bool,
    after_layout_commit: unsafe extern "C" fn(*mut c_void, bool),
    note_full_layouts_performed: unsafe extern "C" fn(*mut c_void, u64),
    evaluate_pending_container_queries: unsafe extern "C" fn(*mut c_void),
    record_stabilization_bound_failure: unsafe extern "C" fn(*mut c_void),
}

impl From<FfiLayoutUpdateHostCallbacks> for LayoutUpdateHost {
    fn from(host: FfiLayoutUpdateHostCallbacks) -> Self {
        Self {
            context: host.context,
            connected_element_count: host.connected_element_count,
            update_style: host.update_style,
            process_pending_list_item_renumbers: host.process_pending_list_item_renumbers,
            process_pending_top_layer_layout_changes: host.process_pending_top_layer_layout_changes,
            document_facts: host.document_facts,
            needs_style_update_after_layout: host.needs_style_update_after_layout,
            prepare_for_rendering: host.prepare_for_rendering,
            build_layout_tree: host.build_layout_tree,
            reconcile_stale_list_item_counters_after_tree_build: host
                .reconcile_stale_list_item_counters_after_tree_build,
            after_layout_commit: host.after_layout_commit,
            note_full_layouts_performed: host.note_full_layouts_performed,
            evaluate_pending_container_queries: host.evaluate_pending_container_queries,
            record_stabilization_bound_failure: host.record_stabilization_bound_failure,
        }
    }
}

impl LayoutUpdateHost {
    // SAFETY (for every call below): The C++ host answers synchronously from its live document.
    fn connected_element_count(&self, _: &crate::stage::MainThread) -> u32 {
        unsafe { (self.connected_element_count)(self.context) }
    }

    fn update_style(&self, _: &crate::stage::MainThread) {
        unsafe { (self.update_style)(self.context) }
    }

    fn process_pending_list_item_renumbers(&self, _: &crate::stage::MainThread) {
        unsafe { (self.process_pending_list_item_renumbers)(self.context) }
    }

    fn process_pending_top_layer_layout_changes(&self, _: &crate::stage::MainThread) {
        unsafe { (self.process_pending_top_layer_layout_changes)(self.context) }
    }

    fn document_facts(&self, _: &crate::stage::MainThread) -> FfiLayoutUpdateDocumentFacts {
        unsafe { (self.document_facts)(self.context) }
    }

    fn needs_style_update_after_layout(&self, _: &crate::stage::MainThread) -> bool {
        unsafe { (self.needs_style_update_after_layout)(self.context) }
    }

    fn prepare_for_rendering(&self, _: &crate::stage::MainThread) {
        unsafe { (self.prepare_for_rendering)(self.context) }
    }

    fn build_layout_tree(&self, _: &crate::stage::MainThread) -> FfiLayoutTreeBuildOutcome {
        unsafe { (self.build_layout_tree)(self.context) }
    }

    fn reconcile_stale_list_item_counters_after_tree_build(&self, _: &crate::stage::MainThread) -> bool {
        unsafe { (self.reconcile_stale_list_item_counters_after_tree_build)(self.context) }
    }

    fn after_layout_commit(&self, _: &crate::stage::MainThread, layout_tree_changed: bool) {
        unsafe { (self.after_layout_commit)(self.context, layout_tree_changed) }
    }

    fn note_full_layouts_performed(&self, _: &crate::stage::MainThread, count: u64) {
        unsafe { (self.note_full_layouts_performed)(self.context, count) }
    }

    fn evaluate_pending_container_queries(&self, _: &crate::stage::MainThread) {
        unsafe { (self.evaluate_pending_container_queries)(self.context) }
    }

    fn record_stabilization_bound_failure(&self, _: &crate::stage::MainThread) {
        unsafe { (self.record_stabilization_bound_failure)(self.context) }
    }
}

/// The same predicate the document exposes: an inactive document is left alone, and the arena
/// answers for the tree it holds.
fn layout_is_up_to_date(arena: &LayoutNodeArena, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
    if !facts.document_is_active {
        return true;
    }
    arena.layout_is_up_to_date(facts.document_needs_layout_tree_build)
}

/// The `TREEBUILD` and `LAYOUT` timing lines, off unless `LIBWEB_UPDATE_LAYOUT_TRACE` is set.
struct UpdateLayoutTrace {
    reason: Option<String>,
}

fn update_layout_trace_is_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("LIBWEB_UPDATE_LAYOUT_TRACE").is_some())
}

impl UpdateLayoutTrace {
    /// # Safety
    ///
    /// `reason_name` must satisfy [`FfiUtf16View::to_utf16`]'s requirements.
    unsafe fn new(reason_name: FfiUtf16View) -> Self {
        if !update_layout_trace_is_enabled() {
            return Self { reason: None };
        }
        // SAFETY: Guaranteed by the caller.
        let reason = unsafe { reason_name.to_utf16() }
            .map(|units| String::from_utf16_lossy(&units))
            .unwrap_or_default();
        Self { reason: Some(reason) }
    }

    fn now(&self) -> Option<Instant> {
        self.reason.as_ref().map(|_| Instant::now())
    }

    fn tree_build(&self, started: Option<Instant>) {
        if let Some(started) = started {
            eprintln!("TREEBUILD {} µs", started.elapsed().as_micros());
        }
    }

    fn layout(&self, started: Option<Instant>) {
        if let (Some(reason), Some(started)) = (&self.reason, started) {
            eprintln!("LAYOUT {reason} {} µs", started.elapsed().as_micros());
        }
    }
}

enum PartialRelayout {
    NotEligible,
    Done,
    NeedsAnotherLayoutPass,
}

const ORDINARY_STABILIZATION_ROUND_LIMIT: u64 = 8;

/// # Safety
///
/// `arena_handle` must be a live handle with registered layout and layout update hosts, used on
/// the document thread or by a stage it waits for, between `layout_arena_begin_update_layout` and
/// its end.
unsafe fn arena<'a>(arena_handle: *mut c_void) -> &'a LayoutNodeArena {
    // SAFETY: Guaranteed by the caller.
    unsafe { LayoutNodeArena::from_handle(arena_handle) }
}

/// The document-thread work a layout frame still interleaves with. The frame waits at each of
/// these while the document thread runs it, so each one is a place where the frame cannot yet
/// overlap with script.
#[derive(Clone, Copy)]
enum FrameJoin {
    /// Style, then the list item renumbers and top layer changes it leaves, then the facts after
    /// them, and the sources of the round's layout pass when no tree build comes first. Style is
    /// the document's own loop over its elements.
    Style,
    /// The layout tree build: the builder reads the DOM, and runs its walk as a stage of its own.
    /// Unless the build asks for another pass, the join then reconciles the list item counters
    /// the build left stale, which live in the document's element sets, and answers with the
    /// sources of the pass that follows. A partial relayout's build also answers with the facts
    /// after it, since the build can resize this document's viewport through its embedding
    /// document.
    BuildLayoutTree,
    /// The host half of a partial relayout boundary's commit when another boundary follows it:
    /// the host is paid its handbacks and delivered the commit messages the document applies at
    /// once, and only then are the arena's update flags settled for the next boundary's pass.
    LayoutCommit,
    /// The host half of the last pass's commit, then what derives from committed layout on the
    /// document side (selection, viewport clients, content-visibility, scroll snapping), then the
    /// container queries the commit made pending, then the facts after them.
    AfterLayoutCommit,
    /// Whether style or layout work is still pending once the loop has run out of rounds. A loop
    /// that stabilizes has these facts from the join that ended it already.
    FinalFacts,
}

/// What a layout frame is started with. The document drains its invalidation journal into the
/// arena before the frame starts; the document facts the frame reads are snapshots the joins
/// hand back.
struct FrameInputs {
    host: LayoutUpdateHost,
    arena_handle: *mut c_void,
    reason_is_inspect_devtools_layout_data: bool,
    is_template_contents_document: bool,
    trace: UpdateLayoutTrace,
}

/// What a layout pass reads from the document ahead of it: the root and body styles the viewport
/// takes over, and the replaced content enrolled for sync. The join the pass follows reads them,
/// so the pass itself prepares the arena without the document thread.
struct LayoutPassSources {
    propagation_facts: FfiViewportPropagationFacts,
    content: EnrolledContentSources,
}

impl LayoutPassSources {
    /// # Safety
    ///
    /// As for [`arena`], on the document thread.
    unsafe fn read(main_thread: &crate::stage::MainThread, arena_handle: *mut c_void) -> Self {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            Self {
                propagation_facts: read_viewport_propagation_facts(main_thread, arena_handle),
                content: read_enrolled_content_sources(main_thread, arena_handle),
            }
        }
    }
}

/// What a finished layout frame leaves for the document thread to apply.
#[must_use]
#[derive(Default)]
struct FrameMessages {
    prepare_for_rendering: bool,
    full_layouts_performed: u64,
    stabilization_bound_failed: bool,
}

impl FrameMessages {
    fn apply(self, main_thread: &crate::stage::MainThread, host: &LayoutUpdateHost) {
        if self.full_layouts_performed > 0 {
            host.note_full_layouts_performed(main_thread, self.full_layouts_performed);
        }
        if self.stabilization_bound_failed {
            host.record_stabilization_bound_failure(main_thread);
            unreachable!("the layout update did not stabilize within its exact bound");
        }
        if self.prepare_for_rendering {
            host.prepare_for_rendering(main_thread);
        }
    }
}

/// The style and layout stabilization loop of one layout update, run as one stage.
struct LayoutFrame<'a> {
    inputs: FrameInputs,
    joins: &'a crate::stage_thread::MainJoins<'a>,
    messages: FrameMessages,
    /// The sources the last join read for the layout pass that follows it.
    pass_sources: Option<LayoutPassSources>,
}

/// The document facts together with what a join answered.
struct Joined<T> {
    value: T,
    facts: FfiLayoutUpdateDocumentFacts,
}

impl LayoutFrame<'_> {
    fn join<R: Send>(
        &self,
        _: FrameJoin,
        work: impl FnOnce(&crate::stage::MainThread<'_>, &LayoutUpdateHost) -> R,
    ) -> R {
        let host = self.inputs.host;
        self.joins.join(|main_thread| work(main_thread, &host))
    }

    fn arena(&self) -> &LayoutNodeArena {
        // SAFETY: The frame runs for the update the arena is in, and no borrow spans a join.
        unsafe { arena(self.inputs.arena_handle) }
    }

    /// Whether a round with these facts lays out at all.
    fn round_lays_out(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        let force_devtools_layout_data_collection =
            facts.should_collect_devtools_layout_data && self.inputs.reason_is_inspect_devtools_layout_data;
        !layout_is_up_to_date(self.arena(), facts) || force_devtools_layout_data_collection
    }

    fn needs_layout_tree_rebuild(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        self.arena().layout_root().is_invalid()
            || facts.document_needs_layout_tree_build
            || self.arena().needs_full_layout_tree_update()
    }

    /// Reads the sources of the round's layout pass on the document thread when the pass follows
    /// the style join directly, without a tree build.
    fn read_pass_sources_after_style(
        &self,
        main_thread: &crate::stage::MainThread,
        facts: &FfiLayoutUpdateDocumentFacts,
    ) -> Option<LayoutPassSources> {
        let pass_follows_style = self.round_lays_out(facts)
            && !self.inputs.is_template_contents_document
            && !self.needs_layout_tree_rebuild(facts);
        // SAFETY: The frame runs for the update the arena is in.
        pass_follows_style.then(|| unsafe { LayoutPassSources::read(main_thread, self.inputs.arena_handle) })
    }

    fn take_pass_sources(&mut self) -> LayoutPassSources {
        self.pass_sources
            .take()
            .expect("the join ahead of a layout pass reads its sources")
    }

    fn run(mut self) -> FrameMessages {
        // Size-query dependencies point from a descendant to an ancestor query container. They are
        // therefore acyclic, and a coherent style/layout pass can settle at least one more level of
        // a nested dependency chain. One pass per connected element is a conservative exact bound.
        // The count is taken after each style update because an initial style update can enroll
        // the elements of a freshly parsed document after the layout update has already started.
        let mut connected_element_count: u32 = 0;
        let mut layout_pass: u64 = 0;
        while layout_pass < ORDINARY_STABILIZATION_ROUND_LIMIT + u64::from(connected_element_count) + 1 {
            layout_pass += 1;

            let Joined {
                value: (element_count, pass_sources),
                facts,
            } = self.join(FrameJoin::Style, |main_thread, host| {
                host.update_style(main_thread);
                host.process_pending_list_item_renumbers(main_thread);
                host.process_pending_top_layer_layout_changes(main_thread);
                let facts = host.document_facts(main_thread);
                Joined {
                    value: (
                        host.connected_element_count(main_thread),
                        self.read_pass_sources_after_style(main_thread, &facts),
                    ),
                    facts,
                }
            });
            connected_element_count = element_count;
            self.pass_sources = pass_sources;

            if !self.round_lays_out(&facts) {
                self.messages.prepare_for_rendering = true;
                return self.messages;
            }

            let mut registered_partial_relayout_roots = self.arena().take_partial_relayout_boundary_roots();

            // NOTE: If this is a document hosting <template> contents, layout is unnecessary.
            if self.inputs.is_template_contents_document {
                return self.messages;
            }

            let mut needs_layout_tree_rebuild = self.needs_layout_tree_rebuild(&facts);

            let mut facts = facts;
            match self.try_partial_relayout(
                &mut facts,
                &mut registered_partial_relayout_roots,
                &mut needs_layout_tree_rebuild,
            ) {
                PartialRelayout::Done => return self.messages,
                PartialRelayout::NeedsAnotherLayoutPass => continue,
                PartialRelayout::NotEligible => {}
            }
            drop(registered_partial_relayout_roots);

            let layout_started = self.inputs.trace.now();

            if needs_layout_tree_rebuild {
                let arena_handle = self.inputs.arena_handle;
                let pass_sources = self.join(FrameJoin::BuildLayoutTree, |main_thread, host| {
                    let outcome = host.build_layout_tree(main_thread);
                    // SAFETY: The frame runs for the update the arena is in.
                    let arena = unsafe { arena(arena_handle) };
                    arena.record_layout_tree_build(&outcome);
                    if outcome.needs_another_build_pass {
                        return None;
                    }

                    // The full layout below covers every boundary the build's invalidation
                    // registered.
                    drop(arena.take_partial_relayout_boundary_roots());

                    // The reconciliation can mark the tree for another build, so it follows the
                    // reset of the full tree update flag.
                    arena.set_needs_full_layout_tree_update(false);
                    self.inputs.trace.tree_build(layout_started);

                    // SAFETY: As above.
                    (!host.reconcile_stale_list_item_counters_after_tree_build(main_thread))
                        .then(|| unsafe { LayoutPassSources::read(main_thread, arena_handle) })
                });
                let Some(pass_sources) = pass_sources else {
                    continue;
                };
                self.pass_sources = Some(pass_sources);
            }

            let layout_root = self.arena().layout_root();
            assert!(!layout_root.is_invalid(), "a full layout pass needs a layout root");
            let arena_handle = self.inputs.arena_handle;
            let LayoutPassSources {
                propagation_facts,
                content,
            } = self.take_pass_sources();
            // SAFETY (for the three steps below): The frame runs for the update the arena is in,
            // the viewport box stays live between them, and no row was freed since the sources
            // were read.
            unsafe { prepare_root_layout_from_sources(arena_handle, layout_root, &propagation_facts, content) };
            let output = unsafe {
                compute_root_layout(
                    arena_handle,
                    layout_root,
                    facts.viewport_inline_size_raw,
                    facts.viewport_block_size_raw,
                    facts.document_in_quirks_mode,
                    facts.should_collect_devtools_layout_data,
                )
            };
            let pending_commit = unsafe { commit_root_layout_to_arena(arena_handle, layout_root, &output) };
            drop(output);

            self.messages.full_layouts_performed += 1;
            self.arena().note_full_layout();

            let Joined {
                value: needs_style_update_after_layout,
                facts,
            } = self.join(FrameJoin::AfterLayoutCommit, |main_thread, host| {
                // SAFETY: The frame runs for the update the arena is in.
                unsafe {
                    pending_commit.finish(main_thread);
                    arena(arena_handle).end_layout_pass_preparation_handbacks(main_thread);
                }
                host.after_layout_commit(main_thread, true);
                host.evaluate_pending_container_queries(main_thread);
                Joined {
                    value: host.needs_style_update_after_layout(main_thread),
                    facts: host.document_facts(main_thread),
                }
            });
            self.inputs.trace.layout(layout_started);

            if needs_style_update_after_layout {
                continue;
            }

            // A zone rebuild requested during layout tree construction runs as another pass.
            if facts.top_layer_work_pending {
                continue;
            }

            // Layout-only invalidations still need to be flushed before we can exit. The refresh
            // join has just answered the final facts, and nothing has run on the document thread
            // since, so the loop has stabilized.
            if layout_is_up_to_date(self.arena(), &facts) {
                return self.messages;
            }
        }

        let Joined {
            value: needs_style_update_after_layout,
            facts,
        } = self.join(FrameJoin::FinalFacts, |main_thread, host| Joined {
            value: host.needs_style_update_after_layout(main_thread),
            facts: host.document_facts(main_thread),
        });
        if needs_style_update_after_layout || !layout_is_up_to_date(self.arena(), &facts) {
            self.messages.stabilization_bound_failed = true;
        }
        self.messages
    }

    /// Attempts to satisfy the pending layout update by re-laying out only the registered partial
    /// relayout boundary subtrees. Runs the incremental layout tree build itself when tree updates
    /// are pending (consuming `needs_layout_tree_rebuild`), so an ineligible update continues to
    /// the full layout path without rebuilding again; `facts` then holds the facts after the build.
    fn try_partial_relayout(
        &mut self,
        facts: &mut FfiLayoutUpdateDocumentFacts,
        registered_partial_relayout_roots: &mut Vec<NodeSlotId>,
        needs_layout_tree_rebuild: &mut bool,
    ) -> PartialRelayout {
        let partial_relayout_facts = FfiPartialRelayoutHostFacts {
            container_query_evaluation_is_pending: facts.container_query_evaluation_is_pending,
            should_collect_devtools_layout_data: facts.should_collect_devtools_layout_data,
        };
        if !self.arena().partial_relayout_may_be_attempted(
            self.arena().layout_root(),
            registered_partial_relayout_roots,
            partial_relayout_facts,
        ) {
            return PartialRelayout::NotEligible;
        }

        let mut layout_tree_was_built_in_partial_branch = false;
        if *needs_layout_tree_rebuild {
            let tree_build_started = self.inputs.trace.now();
            let arena_handle = self.inputs.arena_handle;
            let Joined {
                value: (outcome, counters_were_stale, pass_sources),
                facts: facts_after_build,
            } = self.join(FrameJoin::BuildLayoutTree, |main_thread, host| {
                let outcome = host.build_layout_tree(main_thread);
                // SAFETY (for both uses): The frame runs for the update the arena is in.
                unsafe { arena(arena_handle) }.record_layout_tree_build(&outcome);
                let counters_were_stale = host.reconcile_stale_list_item_counters_after_tree_build(main_thread);
                let facts = host.document_facts(main_thread);
                let pass_follows = !counters_were_stale && !outcome.needs_another_build_pass;
                Joined {
                    value: (
                        outcome,
                        counters_were_stale,
                        pass_follows.then(|| unsafe { LayoutPassSources::read(main_thread, arena_handle) }),
                    ),
                    facts,
                }
            });
            *facts = facts_after_build;
            *needs_layout_tree_rebuild = false;
            if counters_were_stale || outcome.needs_another_build_pass {
                return PartialRelayout::NeedsAnotherLayoutPass;
            }
            self.pass_sources = pass_sources;
            layout_tree_was_built_in_partial_branch = true;

            // The build invalidates what deferred child list insertions reach, which can register
            // more boundaries.
            registered_partial_relayout_roots.extend(self.arena().take_partial_relayout_boundary_roots());
            self.inputs.trace.tree_build(tree_build_started);
        }

        let layout_root = self.arena().layout_root();
        let Some(partial_relayout_roots) = self
            .arena()
            .plan_partial_relayout_with_pending_rebuilt_roots(layout_root, registered_partial_relayout_roots)
        else {
            return PartialRelayout::NotEligible;
        };
        for &root in &partial_relayout_roots {
            debug_assert!(self.arena().slot_is_live(root));
            debug_assert!(node_facts::kind_is_box(self.arena().data(root).kind.get()));
        }

        let arena_handle = self.inputs.arena_handle;
        let content = self.take_pass_sources().content;
        // SAFETY (for the steps below): The frame runs for the update the arena is in, the
        // planned boundaries and the viewport box stay live across them, and no row was freed
        // since the sources were read.
        unsafe { apply_enrolled_content_sources(arena_handle, content) };
        let mut pending_commit: Option<PendingLayoutCommit> = None;
        for &root in &partial_relayout_roots {
            // The next boundary's pass starts from the arena the previous commit settled.
            if let Some(pending_commit) = pending_commit.take() {
                self.join(FrameJoin::LayoutCommit, |main_thread, _| unsafe {
                    pending_commit.finish(main_thread);
                });
            }
            let output = unsafe {
                compute_subtree_layout_fragments(
                    arena_handle,
                    root,
                    layout_root,
                    facts.viewport_inline_size_raw,
                    facts.viewport_block_size_raw,
                    facts.document_in_quirks_mode,
                )
            };
            pending_commit = Some(unsafe { commit_subtree_layout_to_arena(arena_handle, root, &output) });
        }

        self.arena().note_partial_layout();

        let Joined {
            value: needs_style_update_after_layout,
            facts,
        } = self.join(FrameJoin::AfterLayoutCommit, |main_thread, host| {
            if let Some(pending_commit) = pending_commit {
                // SAFETY: The frame runs for the update the arena is in.
                unsafe { pending_commit.finish(main_thread) };
            }
            host.after_layout_commit(main_thread, layout_tree_was_built_in_partial_branch);
            Joined {
                value: host.needs_style_update_after_layout(main_thread),
                facts: host.document_facts(main_thread),
            }
        });
        if needs_style_update_after_layout || !layout_is_up_to_date(self.arena(), &facts) {
            return PartialRelayout::NeedsAnotherLayoutPass;
        }
        PartialRelayout::Done
    }
}

/// Runs the layout update as one frame: the whole stabilization loop is one stage run, which joins
/// the document thread for the steps listed in [`FrameJoin`], and the document thread applies the
/// frame's messages once it has finished.
///
/// # Safety
///
/// As for [`arena`], on the document thread, and `inputs` must satisfy
/// [`UpdateLayoutTrace::new`]'s requirements.
unsafe fn update_layout(
    main_thread: &crate::stage::MainThread,
    arena_handle: *mut c_void,
    inputs: &FfiLayoutUpdateInputs,
) {
    let host = main_thread
        .host_tables()
        .and_then(|host_tables| host_tables.layout_update_host.get())
        .expect("layout node arena has no layout update host");
    // SAFETY: Guaranteed by the caller.
    assert!(
        unsafe { arena(arena_handle) }.update_layout_is_running(),
        "the layout update runs between layout_arena_begin_update_layout and its end"
    );
    let inputs = FrameInputs {
        host,
        arena_handle,
        reason_is_inspect_devtools_layout_data: inputs.reason_is_inspect_devtools_layout_data,
        is_template_contents_document: inputs.is_template_contents_document,
        // SAFETY: Guaranteed by the caller.
        trace: unsafe { UpdateLayoutTrace::new(inputs.reason_name) },
    };
    // SAFETY: The frame reaches the arena and the document only while the document thread waits
    // for it, or through the joins it runs on that thread.
    let messages = unsafe {
        crate::stage_thread::run_stage_with_joins(main_thread, |joins| {
            LayoutFrame {
                inputs,
                joins,
                messages: FrameMessages::default(),
                pass_sources: None,
            }
            .run()
        })
    };
    messages.apply(main_thread, &host);
}

/// # Safety
///
/// `arena` must be a live handle on the document thread. The callbacks must remain valid until
/// they are cleared or the arena is destroyed.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_set_layout_update_host_callbacks(
    arena: *mut c_void,
    callbacks: FfiLayoutUpdateHostCallbacks,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The caller keeps the arena alive for this synchronous call.
    unsafe { crate::layout::HostTables::from_handle(arena) }
        .layout_update_host
        .set(Some(callbacks.into()));
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_clear_layout_update_host_callbacks(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { crate::layout::HostTables::from_handle(arena) }
        .layout_update_host
        .set(None);
}

/// # Safety
///
/// `arena` must be a live handle on the document thread, with no layout update running.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_begin_update_layout(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.begin_update_layout();
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_update_layout_is_running(arena: *mut c_void) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.update_layout_is_running()
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_partial_layout_count(arena: *mut c_void) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.partial_layout_count()
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_full_layout_count(arena: *mut c_void) -> u64 {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.full_layout_count()
}

/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_layout_tree_build_stats(arena: *mut c_void) -> FfiLayoutTreeBuildStats {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { LayoutNodeArena::from_handle(arena) }.layout_tree_build_stats()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts_for() -> FfiLayoutUpdateDocumentFacts {
        FfiLayoutUpdateDocumentFacts {
            document_is_active: true,
            document_needs_layout_tree_build: false,
            container_query_evaluation_is_pending: false,
            top_layer_work_pending: false,
            should_collect_devtools_layout_data: false,
            document_in_quirks_mode: false,
            viewport_inline_size_raw: 0,
            viewport_block_size_raw: 0,
        }
    }

    #[test]
    fn an_inactive_document_counts_as_laid_out_and_a_rootless_active_one_does_not() {
        let arena = LayoutNodeArena::new();
        let mut facts = facts_for();
        assert!(!layout_is_up_to_date(&arena, &facts));
        facts.document_is_active = false;
        assert!(layout_is_up_to_date(&arena, &facts));
    }

    #[test]
    fn dom_side_pending_work_keeps_a_rooted_document_from_being_laid_out() {
        let mut arena = LayoutNodeArena::new();
        let viewport = arena.allocate_unbound();
        arena.set_layout_root(viewport);
        arena.reset_layout_update_flags_in_subtree(viewport);
        let facts = facts_for();
        assert!(layout_is_up_to_date(&arena, &facts));

        let mut tree_build_pending = facts;
        tree_build_pending.document_needs_layout_tree_build = true;
        assert!(!layout_is_up_to_date(&arena, &tree_build_pending));

        arena.set_needs_full_layout_tree_update(true);
        assert!(!layout_is_up_to_date(&arena, &facts));
        arena.set_needs_full_layout_tree_update(false);
        assert!(layout_is_up_to_date(&arena, &facts));

        arena.free_subtree(viewport).destroy_shells_and_invoke_callbacks();
        assert!(!layout_is_up_to_date(&arena, &facts));
    }

    #[test]
    fn the_layout_counters_start_at_zero_and_count_each_pass() {
        let arena = LayoutNodeArena::new();
        assert_eq!(arena.partial_layout_count(), 0);
        assert_eq!(arena.full_layout_count(), 0);
        assert_eq!(arena.layout_tree_build_stats().builds, 0);
        arena.note_partial_layout();
        arena.note_full_layout();
        arena.note_full_layout();
        arena.record_layout_tree_build(&FfiLayoutTreeBuildOutcome {
            viewport: NodeSlotId::INVALID,
            rebuilt_subtree_root_count: 3,
            layout_tree_update_escaped_rebuild_roots: true,
            needs_another_build_pass: false,
        });
        assert_eq!(arena.partial_layout_count(), 1);
        assert_eq!(arena.full_layout_count(), 2);
        let stats = arena.layout_tree_build_stats();
        assert_eq!(stats.builds, 1);
        assert_eq!(stats.last_build_rebuilt_subtree_roots, 3);
        assert!(stats.last_build_escaped_rebuild_roots);
    }
}
