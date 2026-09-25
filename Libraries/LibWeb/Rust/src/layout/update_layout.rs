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
use super::layout_node_arena::{
    EnrolledContentSources, OwedImageResources, apply_enrolled_content_sources, read_enrolled_content_sources,
};
use super::node_data::NodeSlotId;
use super::node_facts;
use super::partial_relayout::FfiPartialRelayoutHostFacts;
use super::tree_builder::{
    FfiGeneratedContentItem, FfiLayoutTreeBuildOutcome, FfiPseudoElement, LayoutTreeBuildWalk, walk_layout_tree_build,
};
use super::viewport_propagation::FfiViewportPropagationFacts;
use crate::abort_on_panic;
use crate::css::ffi_support::FfiUtf16View;
use crate::css::style::tree::StyleNodeID;
use crate::layout::used_values::FfiCssPixelPoint;
use crate::painting::host::FfiRootBackgroundSource;
use crate::painting::paintable_data::FfiSelectionSnapshot;
use crate::painting::selection::SelectionSnapshot;
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
    pub update_style: unsafe extern "C" fn(*mut c_void),
    pub process_pending_list_item_renumbers: unsafe extern "C" fn(*mut c_void),
    pub process_pending_top_layer_layout_changes: unsafe extern "C" fn(*mut c_void),
    pub document_facts: unsafe extern "C" fn(*mut c_void) -> FfiLayoutUpdateDocumentFacts,
    pub needs_style_update_after_layout: unsafe extern "C" fn(*mut c_void) -> bool,
    pub prepare_for_rendering: unsafe extern "C" fn(*mut c_void),
    /// The root element and body boxes the root background is painted from, and whether the body's
    /// background properties are the ones used.
    pub root_background_source: unsafe extern "C" fn(*mut c_void) -> FfiRootBackgroundSource,
    /// Readies the document for a layout tree build, and answers with the document's style node,
    /// which the build walks from.
    pub prepare_layout_tree_build: unsafe extern "C" fn(*mut c_void) -> u32,
    /// Pays the host half of the tree build walk the second argument holds, then installs the
    /// build's viewport as the document's layout root in place of the third.
    pub finish_layout_tree_build:
        unsafe extern "C" fn(*mut c_void, *mut c_void, NodeSlotId) -> FfiLayoutTreeBuildOutcome,
    /// Marks the list owners the frame found showing stale list-item counters for a layout tree
    /// rebuild, named by their style nodes.
    pub rebuild_list_owners_with_stale_item_counters: unsafe extern "C" fn(*mut c_void, *const u32, usize),
    /// Reads the document's selection range, when it has one, into a snapshot the third argument
    /// receives with the second as its context. The snapshot is valid for that call.
    pub read_selection:
        unsafe extern "C" fn(*mut c_void, *mut c_void, unsafe extern "C" fn(*mut c_void, *const FfiSelectionSnapshot)),
    /// Applies what the frame's layout commits leave for the document once the frame is over.
    pub apply_layout_commit_effects: unsafe extern "C" fn(*mut c_void, *const FfiLayoutCommitEffects),
    pub note_full_layouts_performed: unsafe extern "C" fn(*mut c_void, u64),
    pub record_stabilization_bound_failure: unsafe extern "C" fn(*mut c_void),
    /// Attaches the image resources a box's style asks for, which a tree build in the frame owed
    /// it. Principal and pseudo-element boxes both go through this; nothing about it depends on
    /// which the box is. The flag says the box replaces its element's contents with a single
    /// image, which it owns the provider for.
    pub attach_style_resources: unsafe extern "C" fn(*mut c_void, NodeSlotId, bool),
    /// Gives an image a pseudo-element's generated content names the provider it renders, and
    /// attaches its box's style resources. The arguments are the image's row, the element the
    /// pseudo-element is generated for, the pseudo-element, the content item, and the
    /// pseudo-element's own box.
    pub attach_generated_image:
        unsafe extern "C" fn(*mut c_void, NodeSlotId, u32, FfiPseudoElement, FfiGeneratedContentItem, NodeSlotId),
    /// Ends the layout update on the document side. The frame calls it in its last join, so the
    /// frame is over for the document once that join is.
    pub finish_update_layout: unsafe extern "C" fn(*mut c_void),
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
    /// A style has given some element `content-visibility: auto` since the document was created.
    pub may_have_content_visibility_auto_style: bool,
    pub viewport_inline_size_raw: i32,
    pub viewport_block_size_raw: i32,
}

/// What the layout commits of a frame leave for the document, which it applies once the frame is
/// over, as nothing in the frame reads them.
#[repr(C)]
pub struct FfiLayoutCommitEffects {
    /// Whether a layout pass committed at all. A commit invalidates the document's display list and
    /// hit test list, asks for a repaint, and has the scroll containers resnap.
    pub layout_committed: bool,
    /// Whether a commit changed the layout tree, so the document's viewport clients are told the
    /// viewport rect for the new boxes to know whether they are visible.
    pub layout_tree_changed: bool,
    /// Whether the boxes with `content-visibility: auto` were collected again, after a commit that
    /// changed the tree, and which ones they are.
    pub boxes_with_auto_content_visibility_collected: bool,
    pub boxes_with_auto_content_visibility: *const NodeSlotId,
    pub boxes_with_auto_content_visibility_count: usize,
    /// The scroll offsets the commits' overflow measurement clamped, in the order it clamped them,
    /// for the document to store.
    pub clamped_scroll_offsets: *const FfiClampedScrollOffset,
    pub clamped_scroll_offsets_count: usize,
}

/// A scroll offset a commit's overflow measurement brought back into the range its box now allows,
/// named by the node the box is built for rather than by the box, which a later tree build in the
/// same frame may free: an element by its style node, a pseudo-element by its generator's style node
/// and its kind, and the document's viewport by no style node at all.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiClampedScrollOffset {
    pub style_node: u32,
    pub generated_for: u8,
    pub offset: FfiCssPixelPoint,
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
    update_style: unsafe extern "C" fn(*mut c_void),
    process_pending_list_item_renumbers: unsafe extern "C" fn(*mut c_void),
    process_pending_top_layer_layout_changes: unsafe extern "C" fn(*mut c_void),
    document_facts: unsafe extern "C" fn(*mut c_void) -> FfiLayoutUpdateDocumentFacts,
    needs_style_update_after_layout: unsafe extern "C" fn(*mut c_void) -> bool,
    prepare_for_rendering: unsafe extern "C" fn(*mut c_void),
    root_background_source: unsafe extern "C" fn(*mut c_void) -> FfiRootBackgroundSource,
    prepare_layout_tree_build: unsafe extern "C" fn(*mut c_void) -> u32,
    finish_layout_tree_build: unsafe extern "C" fn(*mut c_void, *mut c_void, NodeSlotId) -> FfiLayoutTreeBuildOutcome,
    rebuild_list_owners_with_stale_item_counters: unsafe extern "C" fn(*mut c_void, *const u32, usize),
    read_selection:
        unsafe extern "C" fn(*mut c_void, *mut c_void, unsafe extern "C" fn(*mut c_void, *const FfiSelectionSnapshot)),
    apply_layout_commit_effects: unsafe extern "C" fn(*mut c_void, *const FfiLayoutCommitEffects),
    note_full_layouts_performed: unsafe extern "C" fn(*mut c_void, u64),
    record_stabilization_bound_failure: unsafe extern "C" fn(*mut c_void),
    attach_style_resources: unsafe extern "C" fn(*mut c_void, NodeSlotId, bool),
    attach_generated_image:
        unsafe extern "C" fn(*mut c_void, NodeSlotId, u32, FfiPseudoElement, FfiGeneratedContentItem, NodeSlotId),
    finish_update_layout: unsafe extern "C" fn(*mut c_void),
}

impl From<FfiLayoutUpdateHostCallbacks> for LayoutUpdateHost {
    fn from(host: FfiLayoutUpdateHostCallbacks) -> Self {
        Self {
            context: host.context,
            update_style: host.update_style,
            process_pending_list_item_renumbers: host.process_pending_list_item_renumbers,
            process_pending_top_layer_layout_changes: host.process_pending_top_layer_layout_changes,
            document_facts: host.document_facts,
            needs_style_update_after_layout: host.needs_style_update_after_layout,
            prepare_for_rendering: host.prepare_for_rendering,
            root_background_source: host.root_background_source,
            prepare_layout_tree_build: host.prepare_layout_tree_build,
            finish_layout_tree_build: host.finish_layout_tree_build,
            rebuild_list_owners_with_stale_item_counters: host.rebuild_list_owners_with_stale_item_counters,
            read_selection: host.read_selection,
            apply_layout_commit_effects: host.apply_layout_commit_effects,
            note_full_layouts_performed: host.note_full_layouts_performed,
            record_stabilization_bound_failure: host.record_stabilization_bound_failure,
            attach_style_resources: host.attach_style_resources,
            attach_generated_image: host.attach_generated_image,
            finish_update_layout: host.finish_update_layout,
        }
    }
}

impl LayoutUpdateHost {
    // SAFETY (for every call below): The C++ host answers synchronously from its live document.
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

    fn root_background_source(&self, _: &crate::stage::MainThread) -> FfiRootBackgroundSource {
        unsafe { (self.root_background_source)(self.context) }
    }

    fn prepare_layout_tree_build(&self, _: &crate::stage::MainThread) -> u32 {
        unsafe { (self.prepare_layout_tree_build)(self.context) }
    }

    fn finish_layout_tree_build(
        &self,
        _: &crate::stage::MainThread,
        walked: WalkedLayoutTreeBuild,
    ) -> FfiLayoutTreeBuildOutcome {
        let mut walk = Some(walked.walk);
        let outcome = unsafe {
            (self.finish_layout_tree_build)(self.context, (&raw mut walk).cast(), walked.replaced_layout_root)
        };
        assert!(walk.is_none(), "the host pays the layout tree build walk it is handed");
        outcome
    }

    fn rebuild_list_owners_with_stale_item_counters(&self, _: &crate::stage::MainThread, list_owners: &[StyleNodeID]) {
        if list_owners.is_empty() {
            return;
        }
        let list_owners: Vec<u32> = list_owners.iter().map(|list_owner| list_owner.raw()).collect();
        unsafe {
            (self.rebuild_list_owners_with_stale_item_counters)(self.context, list_owners.as_ptr(), list_owners.len());
        }
    }

    fn read_selection(&self, _: &crate::stage::MainThread) -> Option<SelectionSnapshot> {
        unsafe extern "C" fn receive(sink: *mut c_void, snapshot: *const FfiSelectionSnapshot) {
            // SAFETY: The sink is the option below, and the host hands over a snapshot that is valid
            // for this call.
            unsafe { *sink.cast::<Option<SelectionSnapshot>>() = Some(SelectionSnapshot::from_ffi(&*snapshot)) };
        }
        let mut selection: Option<SelectionSnapshot> = None;
        unsafe { (self.read_selection)(self.context, (&raw mut selection).cast(), receive) };
        selection
    }

    fn apply_layout_commit_effects(&self, _: &crate::stage::MainThread, effects: &FfiLayoutCommitEffects) {
        unsafe { (self.apply_layout_commit_effects)(self.context, effects) }
    }

    fn note_full_layouts_performed(&self, _: &crate::stage::MainThread, count: u64) {
        unsafe { (self.note_full_layouts_performed)(self.context, count) }
    }

    fn record_stabilization_bound_failure(&self, _: &crate::stage::MainThread) {
        unsafe { (self.record_stabilization_bound_failure)(self.context) }
    }

    /// Attaches what a tree build in the frame owed `row`, which must be live.
    fn attach_image_resources(&self, _: &crate::stage::MainThread, row: NodeSlotId, owed: OwedImageResources) {
        match owed {
            OwedImageResources::StyleResources {
                owns_content_replacement_image,
            } => unsafe { (self.attach_style_resources)(self.context, row, owns_content_replacement_image) },
            OwedImageResources::GeneratedImage {
                generator,
                pseudo_element,
                item,
                pseudo_element_box,
            } => unsafe {
                (self.attach_generated_image)(
                    self.context,
                    row,
                    generator.raw(),
                    pseudo_element,
                    item,
                    pseudo_element_box,
                );
            },
        }
    }

    fn finish_update_layout(&self, _: &crate::stage::MainThread) {
        unsafe { (self.finish_update_layout)(self.context) }
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
    /// The layout tree update marks of the list owners the last build found showing stale counters,
    /// then style, then the list item renumbers and top layer changes it leaves, then the facts
    /// after them. When the round lays out, the join readies what comes next: the tree build, or
    /// the sources of the layout pass when no tree build comes first. Style is the document's own
    /// loop over its elements, and a tree update mark is set on the DOM node, which widens it to
    /// what the node's layout node and its document ask for.
    Style,
    /// The host half of a layout tree build whose walk the frame has run, all of which is the
    /// document's C++ and GC-side objects: the shells of the rows the walk freed and the box
    /// presence it changed, the DOM nodes its commit messages resolve to, the shells, style
    /// resources and generated image providers of its new rows, the retirement of the shells of
    /// the tree a new viewport replaced, and the document paint state of the new one. When a pass
    /// follows, the join answers with its sources, which the document reads from its root and body
    /// elements' style and from the shells of replaced content. A partial relayout's build also
    /// answers with the facts after it, since the build can resize this document's viewport
    /// through its embedding document.
    BuildLayoutTree,
    /// The host halves of the partial relayout boundaries' commits the frame settled ahead of
    /// them, in commit order, and of the last pass's commit, then the container queries the commit
    /// made pending, which are the document's query container elements, then the facts after them.
    /// What derives from the commit, the frame does after the join, and what the document only
    /// reads once the frame is over, the frame leaves in its messages.
    AfterLayoutCommit,
    /// Whether style or layout work is still pending once the loop has run out of rounds, after the
    /// marks a last build left, as the style join would have set them. A loop that stabilizes has
    /// these facts from the join that ended it already.
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
/// takes over, the replaced content enrolled for sync, and the boxes the root background is painted
/// from, which the rendering preparation after the pass's commit reads. The join the pass follows
/// reads them, so the pass itself prepares the arena, and its commit prepares for rendering,
/// without the document thread.
struct LayoutPassSources {
    propagation_facts: FfiViewportPropagationFacts,
    content: EnrolledContentSources,
    root_background_source: FfiRootBackgroundSource,
}

impl LayoutPassSources {
    /// # Safety
    ///
    /// As for [`arena`], on the document thread.
    unsafe fn read(main_thread: &crate::stage::MainThread, host: &LayoutUpdateHost, arena_handle: *mut c_void) -> Self {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            Self {
                propagation_facts: read_viewport_propagation_facts(main_thread, arena_handle),
                content: read_enrolled_content_sources(main_thread, arena_handle),
                root_background_source: host.root_background_source(main_thread),
            }
        }
    }
}

/// A tree build walk the frame has run, the document it walked, and the layout root the build may
/// have replaced.
struct WalkedLayoutTreeBuild {
    walk: LayoutTreeBuildWalk,
    document_style_node: StyleNodeID,
    replaced_layout_root: NodeSlotId,
}

/// What the style join readied for the rest of its round.
#[derive(Default)]
struct RoundAfterStyle {
    pass_sources: Option<LayoutPassSources>,
    tree_build_document_style_node: Option<u32>,
    selection: Option<SelectionSnapshot>,
}

/// What a finished layout frame leaves for the document thread to apply.
#[must_use]
#[derive(Default)]
struct FrameMessages {
    prepare_for_rendering: bool,
    full_layouts_performed: u64,
    stabilization_bound_failed: bool,
    /// Whether a layout pass committed in the frame.
    layout_committed: bool,
    /// Whether a commit in the frame changed the layout tree.
    layout_tree_changed: bool,
    /// The boxes with `content-visibility: auto` the last commit that changed the tree left, when
    /// the document may have any. The HTML event loop reads them so it does not have to traverse
    /// the whole tree every time.
    boxes_with_auto_content_visibility: Option<Vec<NodeSlotId>>,
    /// The scroll offsets the rendering preparations after the commits clamped.
    clamped_scroll_offsets: Vec<FfiClampedScrollOffset>,
    /// The image resources the frame's tree builds owe the rows they stamped, in the order the
    /// builds came to owe them: the images to load and observe, and the providers of the images
    /// that image boxes show. Until then an image box that owns its provider has no image, and the
    /// host lays it out again if the image it is handed is already there.
    owed_image_resources: Vec<(NodeSlotId, OwedImageResources)>,
}

impl FrameMessages {
    fn apply(self, main_thread: &crate::stage::MainThread, host: &LayoutUpdateHost, arena: &LayoutNodeArena) {
        // A later build in the frame can have freed a row it was owed for.
        for (row, owed) in self.owed_image_resources {
            if !arena.slot_is_live(row) {
                continue;
            }
            arena.note_owned_provider_handed_over(row);
            host.attach_image_resources(main_thread, row, owed);
        }
        if self.full_layouts_performed > 0 {
            host.note_full_layouts_performed(main_thread, self.full_layouts_performed);
        }
        if self.layout_committed {
            let boxes = self.boxes_with_auto_content_visibility.as_deref();
            host.apply_layout_commit_effects(
                main_thread,
                &FfiLayoutCommitEffects {
                    layout_committed: true,
                    layout_tree_changed: self.layout_tree_changed,
                    boxes_with_auto_content_visibility_collected: boxes.is_some(),
                    boxes_with_auto_content_visibility: boxes.map_or(std::ptr::null(), <[NodeSlotId]>::as_ptr),
                    boxes_with_auto_content_visibility_count: boxes.map_or(0, <[NodeSlotId]>::len),
                    clamped_scroll_offsets: self.clamped_scroll_offsets.as_ptr(),
                    clamped_scroll_offsets_count: self.clamped_scroll_offsets.len(),
                },
            );
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
    /// The document style node of the tree build the style join readied.
    tree_build_document_style_node: Option<u32>,
    /// The list owners the last build found showing stale counters, which the next join marks
    /// for a layout tree rebuild.
    list_owners_to_rebuild: Vec<StyleNodeID>,
    /// The document's selection as the style join of the round read it, if it has one.
    selection: Option<SelectionSnapshot>,
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

    /// Readies what follows the style join on the document thread when the round lays out: the
    /// tree build when one comes first, and otherwise the sources of the layout pass.
    fn ready_round_after_style(
        &self,
        main_thread: &crate::stage::MainThread,
        host: &LayoutUpdateHost,
        facts: &FfiLayoutUpdateDocumentFacts,
    ) -> RoundAfterStyle {
        if !self.round_lays_out(facts) || self.inputs.is_template_contents_document {
            return RoundAfterStyle::default();
        }
        // The commits of the round stamp the selection states of the boxes they build from the
        // selection as it is now, as nothing on the document thread changes the tree until the
        // frame is over.
        let selection = host.read_selection(main_thread);
        if self.needs_layout_tree_rebuild(facts) {
            return RoundAfterStyle {
                tree_build_document_style_node: Some(host.prepare_layout_tree_build(main_thread)),
                selection,
                ..RoundAfterStyle::default()
            };
        }
        RoundAfterStyle {
            // SAFETY: The frame runs for the update the arena is in.
            pass_sources: Some(unsafe { LayoutPassSources::read(main_thread, host, self.inputs.arena_handle) }),
            selection,
            ..RoundAfterStyle::default()
        }
    }

    /// Walks the tree build the style join readied, in the frame. Its host half is left to the
    /// BuildLayoutTree join.
    fn walk_layout_tree_build(&mut self) -> WalkedLayoutTreeBuild {
        let document_style_node = self
            .tree_build_document_style_node
            .take()
            .expect("the style join readies the tree build");
        let replaced_layout_root = self.arena().layout_root();
        WalkedLayoutTreeBuild {
            // SAFETY: The frame runs for the update the arena is in, and the style join published
            // the document's style for the build.
            walk: unsafe { walk_layout_tree_build(self.inputs.arena_handle, document_style_node) },
            document_style_node: StyleNodeID::from_raw(document_style_node)
                .expect("the document has a style node when it builds a layout tree"),
            replaced_layout_root,
        }
    }

    /// Settles the list owners with stale counters after a tree build, and holds on to the ones
    /// the build found showing them for the next join to mark.
    fn reconcile_stale_list_item_counters(&mut self, walked: &WalkedLayoutTreeBuild) {
        debug_assert!(self.list_owners_to_rebuild.is_empty());
        self.list_owners_to_rebuild = self
            .arena()
            .reconcile_stale_list_item_counters_after_tree_build(walked.document_style_node);
    }

    /// What derives from a layout commit, once the AfterLayoutCommit join has finished it: the
    /// rendering preparation, the selection states of the boxes it built are stamped again from the
    /// round's selection, the searchable text is dropped, and after a tree change the boxes with
    /// `content-visibility: auto` are collected again for the document's paint state, and the
    /// document's viewport clients are to be told the viewport rect.
    fn note_layout_commit(
        &mut self,
        layout_tree_changed: bool,
        facts: &FfiLayoutUpdateDocumentFacts,
        root_background_source: FfiRootBackgroundSource,
    ) {
        self.prepare_for_rendering_after_commit(root_background_source);
        if let Some(selection) = &self.selection {
            // SAFETY: The frame runs for the update the arena is in, and no borrow of it is held here.
            selection.apply(unsafe { LayoutNodeArena::from_handle_mut(self.inputs.arena_handle) });
        }
        // SAFETY: The frame runs for the update the arena is in, and no borrow of it is held here.
        unsafe { super::text_queries::layout_arena_invalidate_searchable_text(self.inputs.arena_handle) };
        self.messages.layout_committed = true;
        self.messages.layout_tree_changed |= layout_tree_changed;
        if layout_tree_changed && facts.may_have_content_visibility_auto_style {
            let mut boxes = Vec::new();
            let arena = self.arena();
            crate::painting::content_visibility::for_each_box_with_auto_content_visibility(
                &arena.paintable_rows(),
                arena.layout_root(),
                |slot| boxes.push(slot),
            );
            self.messages.boxes_with_auto_content_visibility = Some(boxes);
        }
    }

    /// The rendering preparation a commit asks for: the root background source the pass sources
    /// read is taken over, and the overflow the commit left unmeasured is measured. The scroll
    /// offsets that measurement clamps are left for the document to store once the frame is over.
    ///
    /// The commit has the document update its accumulated visual contexts and record its display
    /// list again, which covers everything else the preparation can ask for, so what it answers
    /// with is not needed.
    fn prepare_for_rendering_after_commit(&mut self, root_background_source: FfiRootBackgroundSource) {
        let arena = self.arena();
        let (background_source_changed, clamped) =
            crate::painting::ffi::prepare_root_background_and_overflow(arena, root_background_source);
        let clamped: Vec<_> = clamped
            .into_iter()
            .filter_map(|(slot, offset)| {
                let (style_node, generated_for) = arena.bound_node_name(slot)?;
                Some(FfiClampedScrollOffset {
                    style_node: style_node.map_or(0, StyleNodeID::raw),
                    generated_for,
                    offset: offset.into(),
                })
            })
            .collect();
        let _ = crate::painting::ffi::finish_rendering_preparation(arena, background_source_changed, true);
        self.messages.clamped_scroll_offsets.extend(clamped);
    }

    /// Records a paid tree build, and takes the image resources it owes its rows for the host to
    /// attach once the frame is over.
    fn note_layout_tree_build(&mut self, outcome: &FfiLayoutTreeBuildOutcome) {
        self.arena().record_layout_tree_build(outcome);
        let owed = self.arena().take_image_resources_owed_to_host();
        self.messages.owed_image_resources.extend(owed);
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

            let list_owners_to_rebuild = std::mem::take(&mut self.list_owners_to_rebuild);
            let Joined {
                value: (element_count, round_after_style),
                facts,
            } = self.join(FrameJoin::Style, |main_thread, host| {
                host.rebuild_list_owners_with_stale_item_counters(main_thread, &list_owners_to_rebuild);
                host.update_style(main_thread);
                host.process_pending_list_item_renumbers(main_thread);
                host.process_pending_top_layer_layout_changes(main_thread);
                let facts = host.document_facts(main_thread);
                Joined {
                    value: (
                        self.arena()
                            .with_style_store(|engine| engine.tree().connected_element_count()),
                        self.ready_round_after_style(main_thread, host, &facts),
                    ),
                    facts,
                }
            });
            connected_element_count = element_count;
            self.pass_sources = round_after_style.pass_sources;
            self.tree_build_document_style_node = round_after_style.tree_build_document_style_node;
            self.selection = round_after_style.selection;

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
                let walked = self.walk_layout_tree_build();
                let needs_another_build_pass = walked.walk.needs_another_build_pass();
                if !needs_another_build_pass {
                    self.reconcile_stale_list_item_counters(&walked);
                }
                let pass_follows = !needs_another_build_pass && self.list_owners_to_rebuild.is_empty();
                let (outcome, pass_sources) = self.join(FrameJoin::BuildLayoutTree, |main_thread, host| {
                    let outcome = host.finish_layout_tree_build(main_thread, walked);
                    // SAFETY: The frame runs for the update the arena is in.
                    let pass_sources =
                        pass_follows.then(|| unsafe { LayoutPassSources::read(main_thread, host, arena_handle) });
                    (outcome, pass_sources)
                });
                self.note_layout_tree_build(&outcome);
                debug_assert_eq!(outcome.needs_another_build_pass, needs_another_build_pass);
                debug_assert_eq!(outcome.needs_another_build_pass, needs_another_build_pass);
                if needs_another_build_pass {
                    continue;
                }

                // The full layout below covers every boundary the build's invalidation registered.
                drop(self.arena().take_partial_relayout_boundary_roots());

                // The list owners the reconciliation holds on to are marked for another build by
                // the next style join, after the reset of the full tree update flag.
                self.arena().set_needs_full_layout_tree_update(false);
                self.inputs.trace.tree_build(layout_started);

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
                root_background_source,
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
            self.arena().evaluate_size_containers_needing_evaluation_after_layout();

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
                Joined {
                    value: host.needs_style_update_after_layout(main_thread),
                    facts: host.document_facts(main_thread),
                }
            });
            self.note_layout_commit(true, &facts, root_background_source);
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

        let list_owners_to_rebuild = std::mem::take(&mut self.list_owners_to_rebuild);
        let Joined {
            value: needs_style_update_after_layout,
            facts,
        } = self.join(FrameJoin::FinalFacts, |main_thread, host| {
            host.rebuild_list_owners_with_stale_item_counters(main_thread, &list_owners_to_rebuild);
            Joined {
                value: host.needs_style_update_after_layout(main_thread),
                facts: host.document_facts(main_thread),
            }
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
            let walked = self.walk_layout_tree_build();
            let needs_another_build_pass = walked.walk.needs_another_build_pass();
            self.reconcile_stale_list_item_counters(&walked);
            let counters_were_stale = !self.list_owners_to_rebuild.is_empty();
            let pass_follows = !counters_were_stale && !needs_another_build_pass;
            let Joined {
                value: (outcome, pass_sources),
                facts: facts_after_build,
            } = self.join(FrameJoin::BuildLayoutTree, |main_thread, host| {
                let outcome = host.finish_layout_tree_build(main_thread, walked);
                let facts = host.document_facts(main_thread);
                Joined {
                    value: (
                        outcome,
                        // SAFETY: The frame runs for the update the arena is in.
                        pass_follows.then(|| unsafe { LayoutPassSources::read(main_thread, host, arena_handle) }),
                    ),
                    facts,
                }
            });
            self.note_layout_tree_build(&outcome);
            *facts = facts_after_build;
            *needs_layout_tree_rebuild = false;
            if !pass_follows {
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
        let LayoutPassSources {
            content,
            root_background_source,
            ..
        } = self.take_pass_sources();
        // SAFETY (for the steps below): The frame runs for the update the arena is in, the
        // planned boundaries and the viewport box stay live across them, and no row was freed
        // since the sources were read.
        unsafe { apply_enrolled_content_sources(arena_handle, content) };
        let mut pending_commit: Option<PendingLayoutCommit> = None;
        let mut deferred_host_halves = Vec::new();
        for &root in &partial_relayout_roots {
            // The next boundary's pass starts from the arena the previous commit settled; the
            // commit's host half waits for the join after the last boundary.
            if let Some(pending_commit) = pending_commit.take() {
                // SAFETY: The frame runs for the update the arena is in, and delivers the host
                // halves in commit order below.
                deferred_host_halves.push(unsafe { pending_commit.settle_ahead_of_host() });
            }
            let output = unsafe {
                compute_subtree_layout_fragments(
                    arena_handle,
                    root,
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
            for host_half in deferred_host_halves {
                // SAFETY: The frame runs for the update the arena is in.
                unsafe { host_half.deliver(main_thread) };
            }
            if let Some(pending_commit) = pending_commit {
                // SAFETY: The frame runs for the update the arena is in.
                unsafe { pending_commit.finish(main_thread) };
            }
            Joined {
                value: host.needs_style_update_after_layout(main_thread),
                facts: host.document_facts(main_thread),
            }
        });
        self.note_layout_commit(layout_tree_was_built_in_partial_branch, &facts, root_background_source);
        if needs_style_update_after_layout || !layout_is_up_to_date(self.arena(), &facts) {
            return PartialRelayout::NeedsAnotherLayoutPass;
        }
        PartialRelayout::Done
    }
}

fn layout_update_host(main_thread: &crate::stage::MainThread) -> LayoutUpdateHost {
    main_thread
        .host_tables()
        .and_then(|host_tables| host_tables.layout_update_host.get())
        .expect("layout node arena has no layout update host")
}

/// Where a document's layout frame stands, seen from the document thread.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FfiLayoutFrameState {
    /// No layout update is running for the document.
    Idle,
    /// The document's frame is in flight beside the document thread, which runs its event loop
    /// meanwhile. What reads or rewrites what the frame owns waits for it, and a mark the document
    /// thread makes waits for the next frame.
    InFlight,
    /// The document thread runs as part of the frame: in one of its joins, or running the frame
    /// itself. What it marks is what the frame reads next, so it goes through at once.
    MainInsideJoin,
}

/// Where the layout frame of the document `arena_handle` belongs to stands.
///
/// # Safety
///
/// As for [`arena`], on the document thread.
unsafe fn frame_state(arena_handle: *mut c_void) -> FfiLayoutFrameState {
    // A frame in flight owns the arena, so this is asked without reading it.
    if crate::stage_thread::frame_in_flight_owns(arena_handle) {
        return FfiLayoutFrameState::InFlight;
    }
    // SAFETY: Guaranteed by the caller.
    if unsafe { arena(arena_handle) }.update_layout_is_running() {
        FfiLayoutFrameState::MainInsideJoin
    } else {
        FfiLayoutFrameState::Idle
    }
}

/// Runs the layout update as one frame: the whole stabilization loop is one stage run, which joins
/// the document thread for the steps listed in [`FrameJoin`]. Its last join applies the frame's
/// messages and ends the update on the document side, so a document thread that joins the frame
/// in flight finds the document idle afterwards, and may run a layout update of its own.
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
    let host = layout_update_host(main_thread);
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
    // SAFETY: The frame reaches the arena and the document through the handle only while the
    // document thread waits for it, or through the joins it runs on that thread.
    let inputs = unsafe { crate::stage_thread::CallerWaits::new(inputs) };
    // SAFETY: As above, for the work the frame's joins hand the document thread.
    unsafe {
        crate::stage_thread::run_stage_with_joins(main_thread, move |joins| {
            let frame = LayoutFrame {
                inputs: inputs.into_inner(),
                joins,
                messages: FrameMessages::default(),
                pass_sources: None,
                tree_build_document_style_node: None,
                list_owners_to_rebuild: Vec::new(),
                selection: None,
            };
            let arena_handle = frame.inputs.arena_handle;
            let messages = frame.run();
            joins.join(|main_thread| {
                let host = layout_update_host(main_thread);
                // The frame runs for the update the arena is in, and is over.
                messages.apply(main_thread, &host, arena(arena_handle));
                host.finish_update_layout(main_thread);
            });
        });
    }
}

/// Where the layout frame of the document stands, seen from the document thread. Asking does not
/// wait for a frame in flight.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_frame_state(arena: *mut c_void) -> FfiLayoutFrameState {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    unsafe { frame_state(arena) }
}

/// Waits for the document's layout frame if it is in flight. Once this returns, the document is
/// idle or the thread runs inside its frame. `file` and `line` name the C++ call site for the
/// forced-join log.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread. `file` and `file_length` must name a
/// string that lives for the rest of the process, as a `SourceLocation`'s file name does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_join_frame_in_flight(
    arena: *mut c_void,
    file: *const u8,
    file_length: usize,
    line: u32,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The caller passes a string that lives for the rest of the process.
    let file = unsafe { crate::stage_thread::call_site_file(file, file_length) };
    crate::stage_thread::join_frame_in_flight_at(arena, file, line, 0);
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
    let arena_ref = unsafe { LayoutNodeArena::from_handle(arena) };
    // The borrow above has joined a frame in flight that owns the arena; a layout update never runs under one.
    assert!(
        !crate::stage_thread::frame_in_flight_owns(arena),
        "update_layout nested in a frame in flight"
    );
    arena_ref.begin_update_layout();
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
            may_have_content_visibility_auto_style: false,
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
