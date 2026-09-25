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
    DeferredLayoutCommitHostHalf, PendingLayoutCommit, commit_root_layout_to_arena, commit_subtree_layout_to_arena,
    compute_root_layout, compute_subtree_layout_fragments, prepare_root_layout_from_sources,
};
use super::layout_node_arena::{
    EnrolledContentSources, OwedImageResources, apply_enrolled_content_sources, read_enrolled_content_sources,
};
use super::node_data::NodeSlotId;
use super::node_facts;
use super::partial_relayout::FfiPartialRelayoutHostFacts;
use super::tree_builder::{
    FfiGeneratedContentItem, FfiLayoutTreeBuildOutcome, FfiPseudoElement, TreeBuildHostHalf, walk_layout_tree_build,
};
use crate::abort_on_panic;
use crate::css::ffi_support::FfiUtf16View;
use crate::css::style::tree::StyleNodeID;
use crate::layout::used_values::FfiCssPixelPoint;
use crate::painting::paintable_data::FfiSelectionSnapshot;
use crate::painting::selection::SelectionSnapshot;
use std::cell::Cell;
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
    /// Readies the document for a layout tree build, and answers with the document's style node,
    /// which the build walks from.
    pub prepare_layout_tree_build: unsafe extern "C" fn(*mut c_void) -> u32,
    /// Gives the document's new layout tree, which a tree build placed in place of the one before,
    /// a new paint state.
    pub renew_paint_state: unsafe extern "C" fn(*mut c_void),
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
    /// Ends the layout update on the document side. The document thread calls it as it takes in
    /// the frame's end, so the frame is over for the document once the update returns.
    pub finish_update_layout: unsafe extern "C" fn(*mut c_void, FfiLayoutUpdateEnd),
}

/// Where a layout update ends.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FfiLayoutUpdateEnd {
    /// In the layout update the document thread runs, or waits for.
    InUpdate,
    /// Where the document thread takes back the frame in flight that ran the update's full layout
    /// pass, which can be in the middle of any main-thread code. What the frame tells the
    /// document that can run script waits for the next layout update to end.
    FrameTakenBack,
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
    /// Whether the render side showed what the commits laid out already, having updated the
    /// visual contexts and recorded and presented the frame (`LIBWEB_RENDER_CLOCK_FRAMES`): the
    /// document has nothing to paint again for them.
    pub shown_on_render_side: bool,
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
    /// Whether the update may submit its first full layout pass to run beside the document thread
    /// (under `LIBWEB_STAGE_OVERLAP=layout`), rather than waiting for it.
    pub may_submit_pass: bool,
}

/// How far a layout update has got when it returns.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum FfiLayoutUpdateOutcome {
    /// The update is over.
    Finished,
    /// The update's full layout pass runs beside the document thread. The frame in flight owns the
    /// arena, and the update ends once the document thread has taken the frame back.
    PassSubmitted,
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
    prepare_layout_tree_build: unsafe extern "C" fn(*mut c_void) -> u32,
    renew_paint_state: unsafe extern "C" fn(*mut c_void),
    rebuild_list_owners_with_stale_item_counters: unsafe extern "C" fn(*mut c_void, *const u32, usize),
    read_selection:
        unsafe extern "C" fn(*mut c_void, *mut c_void, unsafe extern "C" fn(*mut c_void, *const FfiSelectionSnapshot)),
    apply_layout_commit_effects: unsafe extern "C" fn(*mut c_void, *const FfiLayoutCommitEffects),
    note_full_layouts_performed: unsafe extern "C" fn(*mut c_void, u64),
    record_stabilization_bound_failure: unsafe extern "C" fn(*mut c_void),
    attach_style_resources: unsafe extern "C" fn(*mut c_void, NodeSlotId, bool),
    attach_generated_image:
        unsafe extern "C" fn(*mut c_void, NodeSlotId, u32, FfiPseudoElement, FfiGeneratedContentItem, NodeSlotId),
    finish_update_layout: unsafe extern "C" fn(*mut c_void, FfiLayoutUpdateEnd),
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
            prepare_layout_tree_build: host.prepare_layout_tree_build,
            renew_paint_state: host.renew_paint_state,
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

    fn prepare_layout_tree_build(&self, _: &crate::stage::MainThread) -> u32 {
        unsafe { (self.prepare_layout_tree_build)(self.context) }
    }

    /// Pays what a tree build owed the document thread beyond its own join, then renews the
    /// document's paint state if the build replaced its layout tree.
    fn pay_tree_build_host_half(
        &self,
        main_thread: &crate::stage::MainThread,
        arena: &LayoutNodeArena,
        host_half: TreeBuildHostHalf,
    ) {
        let replaced_layout_tree = host_half.replaced_layout_tree();
        host_half.pay(main_thread, arena);
        if replaced_layout_tree {
            unsafe { (self.renew_paint_state)(self.context) }
        }
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

    fn finish_update_layout(&self, _: &crate::stage::MainThread, end: FfiLayoutUpdateEnd) {
        unsafe { (self.finish_update_layout)(self.context, end) }
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

    fn disabled() -> Self {
        Self { reason: None }
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
    /// Whether style or layout work is still pending once the loop has run out of rounds, after the
    /// marks a last build left, as the style round would have set them. A loop that stabilizes has
    /// these facts from the document thread's take-in of the frame's end already.
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

/// What a layout pass takes ahead of it: what the facts of the replaced content enrolled for sync
/// are derived from, read in the frame once the round's tree build, if any, has run.
struct LayoutPassSources {
    content: EnrolledContentSources,
}

impl LayoutPassSources {
    /// # Safety
    ///
    /// As for [`arena`].
    unsafe fn read(arena_handle: *mut c_void) -> Self {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            Self {
                content: read_enrolled_content_sources(arena_handle),
            }
        }
    }
}

/// A tree build walk the frame has run, and the document it walked.
struct WalkedLayoutTreeBuild {
    outcome: FfiLayoutTreeBuildOutcome,
    document_style_node: StyleNodeID,
}

/// What the style round readied for the rest of its round.
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
    /// Whether the render side showed what the commits laid out (see `FfiLayoutCommitEffects`).
    shown_on_render_side: bool,
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
                    shown_on_render_side: self.shown_on_render_side,
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

/// What a tree build or a commit the frame made owes the document thread beyond its own join.
enum OwedHostHalf {
    TreeBuild(TreeBuildHostHalf),
    Commit(DeferredLayoutCommitHostHalf),
}

/// Takes back the layout tree update marks the frame lent a tree build, then pays what the frame
/// owed the document thread, in the order the frame made it owe it.
///
/// # Safety
///
/// As for [`arena`], on the document thread, with no borrow of the arena held across the call and
/// no stage of the frame reaching the arena meanwhile.
unsafe fn pay_owed_host_halves(
    main_thread: &crate::stage::MainThread,
    host: &LayoutUpdateHost,
    arena_handle: *mut c_void,
    owed_host_halves: Vec<OwedHostHalf>,
) {
    // What the build owes the document can mark nodes for another build.
    // SAFETY: Guaranteed by the caller.
    unsafe { super::tree_update_marks::take_back_from_frame(arena_handle) };
    for owed in owed_host_halves {
        match owed {
            // SAFETY: Guaranteed by the caller.
            OwedHostHalf::TreeBuild(owed) => {
                host.pay_tree_build_host_half(main_thread, unsafe { arena(arena_handle) }, owed);
            }
            // SAFETY: Guaranteed by the caller.
            OwedHostHalf::Commit(owed) => unsafe { owed.deliver(main_thread) },
        }
    }
    // What the document thread wrote to the marks beside the frame, it wrote after all of that.
    // SAFETY: Guaranteed by the caller. The marks are handed back.
    unsafe { super::tree_update_marks::write_marks_waiting_for_frame(arena_handle) };
}

/// How a layout frame ends.
#[derive(Clone, Copy)]
enum FrameEnd {
    /// The frame found its last commit stable from what it owns. The host halves of its commits can
    /// still leave style or layout work pending: the container queries the commits made pending
    /// (the document's query container elements), the other style work, the top layer changes and
    /// the layout tree update marks they made. If they do, the frame is resumed for another round.
    UnlessHostLeftWork,
    /// The frame is over, and the update ends where the value says.
    Over(FfiLayoutUpdateEnd),
}

/// The style and layout stabilization loop of one layout update, run as one stage. It holds no
/// borrow of the stage run it is in: the joins are handed to each step that makes one, so a round
/// can stop ahead of its full layout pass and go on once the pass has run.
struct LayoutFrame {
    inputs: FrameInputs,
    messages: FrameMessages,
    /// The rounds the loop has started, and the connected element count the last style round
    /// answered, which bound them.
    layout_pass: u64,
    connected_element_count: u32,
    /// The sources the last join read for the layout pass that follows it.
    pass_sources: Option<LayoutPassSources>,
    /// The document style node of the tree build the style round readied.
    tree_build_document_style_node: Option<u32>,
    /// The list owners the last build found showing stale counters, which the next join marks
    /// for a layout tree rebuild.
    list_owners_to_rebuild: Vec<StyleNodeID>,
    /// The document's selection as the style round of the round read it, if it has one.
    selection: Option<SelectionSnapshot>,
    /// What the frame's tree builds and commits owe the document thread beyond their own joins, in
    /// the order the frame made them, which the next join pays before its work.
    owed_host_halves: Cell<Vec<OwedHostHalf>>,
}

/// The document facts together with what a join answered.
struct Joined<T> {
    value: T,
    facts: FfiLayoutUpdateDocumentFacts,
}

/// A full layout pass a round has readied. Everything it reads is in hand, so it runs without the
/// document thread, and what follows its commit is left to [`LayoutFrame::finish_round`].
struct PendingLayoutPass {
    arena_handle: *mut c_void,
    layout_root: NodeSlotId,
    sources: LayoutPassSources,
    facts: FfiLayoutUpdateDocumentFacts,
    started: Option<Instant>,
}

impl PendingLayoutPass {
    /// # Safety
    ///
    /// The frame must run for the update the arena is in, and nothing but the pass may reach the
    /// arena until it returns.
    unsafe fn run(self) -> LaidOutPass {
        let Self {
            arena_handle,
            layout_root,
            sources: LayoutPassSources { content },
            facts,
            started,
        } = self;
        // SAFETY (for the three steps below): Guaranteed by the caller; the viewport box stays live
        // between them, and no row was freed since the sources were read.
        unsafe { prepare_root_layout_from_sources(arena_handle, layout_root, content) };
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
        // SAFETY: Guaranteed by the caller.
        let arena = unsafe { arena(arena_handle) };
        arena.evaluate_size_containers_needing_evaluation_after_layout();
        // SAFETY: Guaranteed by the caller, and the frame delivers the host half at its next join,
        // in commit order.
        let commit_host_half = unsafe { pending_commit.settle_ahead_of_host() };
        arena.end_layout_pass_preparation_handbacks();
        arena.note_full_layout();
        LaidOutPass {
            commit_host_half,
            facts,
            started,
        }
    }
}

/// A full layout pass that has run and settled its commit in the arena, whose commit's host half
/// the frame still owes the document thread.
struct LaidOutPass {
    commit_host_half: DeferredLayoutCommitHostHalf,
    facts: FfiLayoutUpdateDocumentFacts,
    started: Option<Instant>,
}

/// Where a layout frame goes on once a step of it has run.
enum FrameStep {
    /// Another round, whose style the document thread runs first, with no stage run outstanding.
    NeedsStyle,
    /// A round has readied its full layout pass.
    PassReady(PendingLayoutPass),
    /// The loop is over, and the frame ends as the value says.
    Ended(FrameEnd),
}

impl LayoutFrame {
    fn join<R: Send>(
        &self,
        joins: &crate::stage_thread::MainJoins<'_>,
        _: FrameJoin,
        work: impl FnOnce(&crate::stage::MainThread<'_>, &LayoutUpdateHost) -> R,
    ) -> R {
        let host = self.inputs.host;
        let arena_handle = self.inputs.arena_handle;
        let owed_host_halves = self.owed_host_halves.take();
        joins.join(|main_thread| {
            // SAFETY: The frame runs for the update the arena is in, and no borrow spans a join.
            unsafe { pay_owed_host_halves(main_thread, &host, arena_handle, owed_host_halves) };
            work(main_thread, &host)
        })
    }

    /// Leaves what the frame owes the document thread to the next join, after what it owed before.
    fn owe_host_half(&self, owed: OwedHostHalf) {
        let mut owed_host_halves = self.owed_host_halves.take();
        owed_host_halves.push(owed);
        self.owed_host_halves.set(owed_host_halves);
    }

    /// Settles a commit's arena half, and leaves its host half for the next join.
    fn settle_commit_ahead_of_host(&self, pending_commit: PendingLayoutCommit) {
        // SAFETY: The frame runs for the update the arena is in, and the next join delivers the
        // host halves in commit order.
        self.owe_host_half(OwedHostHalf::Commit(unsafe { pending_commit.settle_ahead_of_host() }));
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

    /// Readies what follows the style round on the document thread when the round lays out: the
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
            pass_sources: Some(unsafe { LayoutPassSources::read(self.inputs.arena_handle) }),
            selection,
            ..RoundAfterStyle::default()
        }
    }

    /// Walks the tree build the style round readied, in the frame. Its host half (the shells of the
    /// rows the walk freed, the box presence it changed, the DOM nodes its commit messages resolve
    /// to, a new viewport's paint state) is left to the next join, and the style resources and
    /// generated image providers of its new rows to the end of the frame.
    fn walk_layout_tree_build(&mut self) -> (WalkedLayoutTreeBuild, TreeBuildHostHalf) {
        let document_style_node = self
            .tree_build_document_style_node
            .take()
            .expect("the style round readies the tree build");
        // SAFETY: The frame runs for the update the arena is in, and the style round published the
        // document's style for the build.
        let (outcome, host_half) = unsafe { walk_layout_tree_build(self.inputs.arena_handle, document_style_node) };
        let walked = WalkedLayoutTreeBuild {
            outcome,
            document_style_node: StyleNodeID::from_raw(document_style_node)
                .expect("the document has a style node when it builds a layout tree"),
        };
        (walked, host_half)
    }

    /// Leaves what a tree build owes the document thread beyond its own join to the next join.
    fn owe_tree_build_host_half(&self, host_half: TreeBuildHostHalf) {
        let owed_host_halves = self.owed_host_halves.take();
        assert!(
            !owed_host_halves
                .iter()
                .any(|owed| matches!(owed, OwedHostHalf::TreeBuild(_))),
            "a join pays a tree build's host half before the next build"
        );
        self.owed_host_halves.set(owed_host_halves);
        self.owe_host_half(OwedHostHalf::TreeBuild(host_half));
    }

    /// Settles the list owners with stale counters after a tree build, and holds on to the ones
    /// the build found showing them for the next join to mark.
    fn reconcile_stale_list_item_counters(&mut self, walked: &WalkedLayoutTreeBuild) {
        debug_assert!(self.list_owners_to_rebuild.is_empty());
        self.list_owners_to_rebuild = self
            .arena()
            .reconcile_stale_list_item_counters_after_tree_build(walked.document_style_node);
    }

    /// What derives from a layout commit, once its arena half has settled: the rendering preparation,
    /// the selection states of the boxes it built are stamped again from the round's selection, the
    /// searchable text is dropped, and after a tree change the boxes with `content-visibility: auto`
    /// are collected again for the document's paint state, and the document's viewport clients are
    /// to be told the viewport rect.
    fn note_layout_commit(&mut self, layout_tree_changed: bool) {
        self.prepare_for_rendering_after_commit();
        if let Some(selection) = &self.selection {
            // SAFETY: The frame runs for the update the arena is in, and no borrow of it is held here.
            selection.apply(unsafe { LayoutNodeArena::from_handle_mut(self.inputs.arena_handle) });
        }
        // SAFETY: The frame runs for the update the arena is in, and no borrow of it is held here.
        unsafe { super::text_queries::layout_arena_invalidate_searchable_text(self.inputs.arena_handle) };
        self.messages.layout_committed = true;
        self.messages.layout_tree_changed |= layout_tree_changed;
        if layout_tree_changed && self.arena().may_have_auto_content_visibility() {
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

    /// The rendering preparation a commit asks for: the root background source is taken over, and
    /// the overflow the commit left unmeasured is measured. The scroll
    /// offsets that measurement clamps are left for the document to store once the frame is over.
    ///
    /// The commit has the document update its accumulated visual contexts and record its display
    /// list again, which covers everything else the preparation can ask for, so what it answers
    /// with is not needed.
    fn prepare_for_rendering_after_commit(&mut self) {
        let arena = self.arena();
        let root_background_source = super::root_background_source(arena);
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

    /// Whether what the frame owns shows layout work its last commit left for another round. What
    /// the host halves of the commits leave pending, the document thread asks the document once it
    /// has paid them, as it takes in the frame's end.
    fn commit_left_layout_work(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        facts.document_is_active && !self.arena().layout_is_up_to_date(false)
    }

    /// Takes the frame's end in on the document thread, with no stage run outstanding: pays what
    /// the frame owed, then either answers false if the host halves left it work for another round,
    /// or applies the frame's messages, ends the update on the document side, and answers true.
    ///
    /// # Safety
    ///
    /// As for [`arena`], on the document thread, with no stage run of the frame outstanding.
    unsafe fn take_in_end(&mut self, main_thread: &crate::stage::MainThread, end: FrameEnd) -> bool {
        debug_assert!(
            self.list_owners_to_rebuild.is_empty(),
            "a join marks the list owners before the frame ends"
        );
        let host = self.inputs.host;
        let arena_handle = self.inputs.arena_handle;
        // SAFETY: Guaranteed by the caller.
        unsafe { pay_owed_host_halves(main_thread, &host, arena_handle, self.owed_host_halves.take()) };
        // SAFETY: As above.
        let arena = unsafe { arena(arena_handle) };
        let end = match end {
            FrameEnd::UnlessHostLeftWork => {
                let facts = host.document_facts(main_thread);
                if host.needs_style_update_after_layout(main_thread)
                    || facts.top_layer_work_pending
                    || !layout_is_up_to_date(arena, &facts)
                {
                    return false;
                }
                FfiLayoutUpdateEnd::InUpdate
            }
            FrameEnd::Over(end) => end,
        };
        std::mem::take(&mut self.messages).apply(main_thread, &host, arena);
        host.finish_update_layout(main_thread, end);
        true
    }

    /// Runs the frame's loop from the document thread. Each round's style runs here, with no stage
    /// run outstanding, and each step after it runs as a stage of its own. With `stop_at_round`, the
    /// loop stops once the style of the first round that lays out has run, and answers the facts
    /// the rest of that round runs with, for it to go in flight, and the frame has not ended;
    /// otherwise the loop goes on until the frame has ended.
    ///
    /// # Safety
    ///
    /// On the document thread, for the update the arena is in.
    unsafe fn drive(
        &mut self,
        main_thread: &crate::stage::MainThread,
        stop_at_round: bool,
    ) -> Option<FfiLayoutUpdateDocumentFacts> {
        let mut step = FrameStep::NeedsStyle;
        loop {
            // SAFETY (for the steps below): Guaranteed by the caller, and the document thread
            // waits for each stage.
            step = match step {
                FrameStep::Ended(end) => {
                    if unsafe { self.take_in_end(main_thread, end) } {
                        return None;
                    }
                    FrameStep::NeedsStyle
                }
                FrameStep::NeedsStyle if !self.may_start_round() => unsafe {
                    self.run_stage(main_thread, |frame, joins| {
                        FrameStep::Ended(frame.run_out_of_rounds(joins))
                    })
                },
                FrameStep::NeedsStyle => {
                    let facts = unsafe { self.run_style_round(main_thread) };
                    if stop_at_round && self.round_lays_out_in_frame(&facts) {
                        return Some(facts);
                    }
                    unsafe { self.run_stage(main_thread, move |frame, _| frame.run_round(facts)) }
                }
                FrameStep::PassReady(pass) => unsafe {
                    self.run_stage(main_thread, move |frame, _| {
                        let laid_out = pass.run();
                        frame.finish_round(laid_out)
                    })
                },
            };
        }
    }

    /// Runs a step of the frame as a stage, which may join the document thread.
    ///
    /// # Safety
    ///
    /// On the document thread, for the update the arena is in.
    unsafe fn run_stage(
        &mut self,
        main_thread: &crate::stage::MainThread,
        step: impl FnOnce(&mut Self, &crate::stage_thread::MainJoins<'_>) -> FrameStep,
    ) -> FrameStep {
        let arena_handle = self.inputs.arena_handle;
        // SAFETY: The frame and the step reach the arena and the document only while the document
        // thread waits for the stage, or through the joins it runs on that thread.
        let frame = unsafe { crate::stage_thread::CallerWaits::new(self) };
        let step = unsafe { crate::stage_thread::CallerWaits::new(step) };
        // SAFETY: As above, for the work the frame's joins hand the document thread.
        let next = unsafe {
            crate::stage_thread::run_document_stage_with_joins(main_thread, arena_handle, move |joins| {
                let next = (step.into_inner())(frame.into_inner(), joins);
                // SAFETY: The step goes back to the document thread, which waits for the stage.
                crate::stage_thread::CallerWaits::new(next)
            })
        };
        next.into_inner()
    }

    /// Whether the loop may start another round. Size-query dependencies point from a descendant to
    /// an ancestor query container. They are therefore acyclic, and a coherent style/layout pass can
    /// settle at least one more level of a nested dependency chain. One pass per connected element
    /// is a conservative exact bound. The count is taken after each style round because an initial
    /// style update can enroll the elements of a freshly parsed document after the layout update
    /// has already started.
    fn may_start_round(&self) -> bool {
        self.layout_pass < ORDINARY_STABILIZATION_ROUND_LIMIT + u64::from(self.connected_element_count) + 1
    }

    /// Starts a round on the document thread, with no stage run outstanding: pays what the frame
    /// owes the document thread, then marks the list owners the last build found showing stale
    /// counters for a layout tree rebuild, runs style and the list item renumbers and top layer
    /// changes it leaves, and reads the facts after them. When the round lays out, it readies what
    /// comes next: the tree build, or the sources of the layout pass when no tree build comes
    /// first. Style is the document's own loop over its elements, and a tree update mark is set on
    /// the DOM node, which widens it to what the node's layout node and its document ask for.
    ///
    /// # Safety
    ///
    /// The frame must run for the update the arena is in, and no stage may reach it until this
    /// returns.
    unsafe fn run_style_round(&mut self, main_thread: &crate::stage::MainThread) -> FfiLayoutUpdateDocumentFacts {
        self.layout_pass += 1;
        let host = self.inputs.host;
        // SAFETY: Guaranteed by the caller.
        unsafe {
            pay_owed_host_halves(
                main_thread,
                &host,
                self.inputs.arena_handle,
                self.owed_host_halves.take(),
            );
        }
        let list_owners_to_rebuild = std::mem::take(&mut self.list_owners_to_rebuild);
        host.rebuild_list_owners_with_stale_item_counters(main_thread, &list_owners_to_rebuild);
        host.update_style(main_thread);
        host.process_pending_list_item_renumbers(main_thread);
        host.process_pending_top_layer_layout_changes(main_thread);
        let facts = host.document_facts(main_thread);
        self.connected_element_count = self
            .arena()
            .with_style_store(|engine| engine.tree().connected_element_count());
        let round_after_style = self.ready_round_after_style(main_thread, &host, &facts);
        if round_after_style.tree_build_document_style_node.is_some() {
            // SAFETY: Guaranteed by the caller.
            unsafe { super::tree_update_marks::lend_to_frame(self.inputs.arena_handle) };
        }
        self.pass_sources = round_after_style.pass_sources;
        self.tree_build_document_style_node = round_after_style.tree_build_document_style_node;
        self.selection = round_after_style.selection;
        facts
    }

    /// Whether the rest of a round with these facts lays out, rather than ending the frame at once.
    fn round_lays_out_in_frame(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        self.round_lays_out(facts) && !self.inputs.is_template_contents_document
    }

    /// Runs the rest of a round whose style the document thread has run, its tree build and its
    /// layout pass, without the document thread: the round the frame in flight runs. Answers where
    /// the frame would go on, which the document thread leaves to the next layout update once it
    /// takes the frame back.
    ///
    /// # Safety
    ///
    /// Nothing but the frame may reach the arena until this returns.
    unsafe fn run_round_through_pass(&mut self, facts: FfiLayoutUpdateDocumentFacts) -> FrameStep {
        match self.run_round(facts) {
            FrameStep::PassReady(pass) => {
                // SAFETY: Guaranteed by the caller.
                let laid_out = unsafe { pass.run() };
                self.finish_round(laid_out)
            }
            step => step,
        }
    }

    /// Runs the rest of a round whose style the document thread has run, up to its full layout
    /// pass, or until the round ends in another round or the end of the frame.
    fn run_round(&mut self, facts: FfiLayoutUpdateDocumentFacts) -> FrameStep {
        if !self.round_lays_out(&facts) {
            self.messages.prepare_for_rendering = true;
            return FrameStep::Ended(FrameEnd::Over(FfiLayoutUpdateEnd::InUpdate));
        }

        let mut registered_partial_relayout_roots = self.arena().take_partial_relayout_boundary_roots();

        // NOTE: If this is a document hosting <template> contents, layout is unnecessary.
        if self.inputs.is_template_contents_document {
            return FrameStep::Ended(FrameEnd::Over(FfiLayoutUpdateEnd::InUpdate));
        }

        let mut needs_layout_tree_rebuild = self.needs_layout_tree_rebuild(&facts);

        match self.try_partial_relayout(
            &facts,
            &mut registered_partial_relayout_roots,
            &mut needs_layout_tree_rebuild,
        ) {
            PartialRelayout::Done => return FrameStep::Ended(FrameEnd::UnlessHostLeftWork),
            PartialRelayout::NeedsAnotherLayoutPass => return FrameStep::NeedsStyle,
            PartialRelayout::NotEligible => {}
        }
        drop(registered_partial_relayout_roots);

        let layout_started = self.inputs.trace.now();

        if needs_layout_tree_rebuild {
            let arena_handle = self.inputs.arena_handle;
            let (walked, host_half) = self.walk_layout_tree_build();
            let needs_another_build_pass = walked.outcome.needs_another_build_pass;
            if !needs_another_build_pass {
                self.reconcile_stale_list_item_counters(&walked);
            }
            let pass_follows = !needs_another_build_pass && self.list_owners_to_rebuild.is_empty();
            // SAFETY: The frame runs for the update the arena is in.
            let pass_sources = pass_follows.then(|| unsafe { LayoutPassSources::read(arena_handle) });
            self.owe_tree_build_host_half(host_half);
            self.note_layout_tree_build(&walked.outcome);
            if needs_another_build_pass {
                return FrameStep::NeedsStyle;
            }

            // The full layout below covers every boundary the build's invalidation registered.
            drop(self.arena().take_partial_relayout_boundary_roots());

            // The list owners the reconciliation holds on to are marked for another build by
            // the next style round, after the reset of the full tree update flag.
            self.arena().set_needs_full_layout_tree_update(false);
            self.inputs.trace.tree_build(layout_started);

            let Some(pass_sources) = pass_sources else {
                return FrameStep::NeedsStyle;
            };
            self.pass_sources = Some(pass_sources);
        }

        let layout_root = self.arena().layout_root();
        assert!(!layout_root.is_invalid(), "a full layout pass needs a layout root");
        FrameStep::PassReady(PendingLayoutPass {
            arena_handle: self.inputs.arena_handle,
            layout_root,
            sources: self.take_pass_sources(),
            facts,
            started: layout_started,
        })
    }

    /// Ends a loop that has run out of rounds, noting whether style or layout work is still
    /// pending.
    fn run_out_of_rounds(&mut self, joins: &crate::stage_thread::MainJoins<'_>) -> FrameEnd {
        let list_owners_to_rebuild = std::mem::take(&mut self.list_owners_to_rebuild);
        let Joined {
            value: needs_style_update_after_layout,
            facts,
        } = self.join(joins, FrameJoin::FinalFacts, |main_thread, host| {
            host.rebuild_list_owners_with_stale_item_counters(main_thread, &list_owners_to_rebuild);
            Joined {
                value: host.needs_style_update_after_layout(main_thread),
                facts: host.document_facts(main_thread),
            }
        });
        if needs_style_update_after_layout || !layout_is_up_to_date(self.arena(), &facts) {
            self.messages.stabilization_bound_failed = true;
        }
        FrameEnd::Over(FfiLayoutUpdateEnd::InUpdate)
    }

    /// Ends the round of a full layout pass that has run: what derives from its commit, and whether
    /// the loop has stabilized as far as what the frame owns shows. The frame then ends, unless the
    /// host halves of its commits left work for another round.
    fn finish_round(&mut self, laid_out: LaidOutPass) -> FrameStep {
        let facts = self.note_laid_out_pass(laid_out);

        // Layout-only invalidations still need to be flushed before we can exit.
        if self.commit_left_layout_work(&facts) {
            return FrameStep::NeedsStyle;
        }

        // The document thread asks what the host halves left as it takes in the frame's end, and
        // nothing else runs on it until then, so if they left nothing the loop has stabilized.
        FrameStep::Ended(FrameEnd::UnlessHostLeftWork)
    }

    /// Takes in a full layout pass that has run: its commit's host half is left for the next join,
    /// and what derives from the commit is done. Answers the facts the pass ran with.
    fn note_laid_out_pass(&mut self, laid_out: LaidOutPass) -> FfiLayoutUpdateDocumentFacts {
        let LaidOutPass {
            commit_host_half,
            facts,
            started,
        } = laid_out;
        self.owe_host_half(OwedHostHalf::Commit(commit_host_half));
        self.messages.full_layouts_performed += 1;
        self.note_layout_commit(true);
        self.inputs.trace.layout(started);
        facts
    }

    /// Attempts to satisfy the pending layout update by re-laying out only the registered partial
    /// relayout boundary subtrees. Runs the incremental layout tree build itself when tree updates
    /// are pending (consuming `needs_layout_tree_rebuild`), so an ineligible update continues to
    /// the full layout path without rebuilding again; `facts` then holds the facts after the build.
    fn try_partial_relayout(
        &mut self,
        facts: &FfiLayoutUpdateDocumentFacts,
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
            let (walked, host_half) = self.walk_layout_tree_build();
            let needs_another_build_pass = walked.outcome.needs_another_build_pass;
            self.reconcile_stale_list_item_counters(&walked);
            let counters_were_stale = !self.list_owners_to_rebuild.is_empty();
            let pass_follows = !counters_were_stale && !needs_another_build_pass;
            // As after a full layout's build, the host half waits for the next join, which finds
            // what paying it changes (it can resize this document's viewport through its embedding
            // document).
            // SAFETY: The frame runs for the update the arena is in.
            let pass_sources = pass_follows.then(|| unsafe { LayoutPassSources::read(arena_handle) });
            self.owe_tree_build_host_half(host_half);
            self.note_layout_tree_build(&walked.outcome);
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
        let LayoutPassSources { content } = self.take_pass_sources();
        // SAFETY (for the steps below): The frame runs for the update the arena is in, the
        // planned boundaries and the viewport box stay live across them, and no row was freed
        // since the sources were read.
        unsafe { apply_enrolled_content_sources(arena_handle, content) };
        for &root in &partial_relayout_roots {
            // The next boundary's pass starts from the arena the previous commit settled; the
            // commit's host half waits for the next join.
            let output = unsafe {
                compute_subtree_layout_fragments(
                    arena_handle,
                    root,
                    facts.viewport_inline_size_raw,
                    facts.viewport_block_size_raw,
                    facts.document_in_quirks_mode,
                )
            };
            let pending_commit = unsafe { commit_subtree_layout_to_arena(arena_handle, root, &output) };
            self.settle_commit_ahead_of_host(pending_commit);
        }

        self.arena().note_partial_layout();

        self.note_layout_commit(layout_tree_was_built_in_partial_branch);
        if self.commit_left_layout_work(facts) {
            return PartialRelayout::NeedsAnotherLayoutPass;
        }
        PartialRelayout::Done
    }
}

/// A layout frame a clock lease's ticks lay out in on the render side while the main thread idles
/// (`LIBWEB_RENDER_CLOCK_FRAMES`). The main thread makes it with what its rounds read from the
/// document (the facts, the selection), which nothing changes while it idles, and takes it in when
/// it wakes: it pays what the rounds owe and applies their messages, as it takes in a submitted
/// pass's frame.
pub(crate) struct ClockLayoutFrame {
    frame: LayoutFrame,
    facts: FfiLayoutUpdateDocumentFacts,
    /// Whether a round laid out in the frame, so that it has something to take in.
    laid_out: bool,
}

// SAFETY: The frame reaches only the arena, which a clock tick owns while it runs a round in it, and
// its document, which only the main thread reaches, once it has taken the frame back.
unsafe impl Send for ClockLayoutFrame {}

impl ClockLayoutFrame {
    /// Runs the round of layout the samples a clock tick installed left. Returns false where the
    /// round needs the main thread: a layout tree build, or another style round.
    ///
    /// # Safety
    ///
    /// On the thread that owns the arena, with the main thread idle.
    pub(crate) unsafe fn run_round(&mut self) -> bool {
        if !self.frame.round_lays_out(&self.facts) {
            return true;
        }
        if self.frame.needs_layout_tree_rebuild(&self.facts) || self.frame.inputs.is_template_contents_document {
            return false;
        }
        // SAFETY: Guaranteed by the caller.
        self.frame.pass_sources = Some(unsafe { LayoutPassSources::read(self.frame.inputs.arena_handle) });
        self.laid_out = true;
        // SAFETY: Guaranteed by the caller.
        match unsafe { self.frame.run_round_through_pass(self.facts) } {
            FrameStep::Ended(_) => !self.frame.commit_left_layout_work(&self.facts),
            FrameStep::NeedsStyle | FrameStep::PassReady(_) => false,
        }
    }

    pub(crate) fn laid_out(&self) -> bool {
        self.laid_out
    }
}

/// Takes in the layout frame of a clock lease's ticks, and ends the update the document began for
/// it: pays what the rounds owe the document, and applies their messages at once, since the
/// document thread takes the frame in at the top of its event loop, where they can run. Where the
/// render side showed every tick that laid out in the frame, the document paints nothing again.
///
/// # Safety
///
/// On the document thread, with the ticks over and the update begun.
unsafe fn take_in_clock_layout_frame(
    main_thread: &crate::stage::MainThread,
    mut frame: ClockLayoutFrame,
    shown_on_render_side: bool,
) {
    frame.frame.messages.shown_on_render_side = shown_on_render_side;
    let host = frame.frame.inputs.host;
    // SAFETY: Guaranteed by the caller.
    unsafe {
        pay_owed_host_halves(
            main_thread,
            &host,
            frame.frame.inputs.arena_handle,
            frame.frame.owed_host_halves.take(),
        );
    }
    let list_owners_to_rebuild = std::mem::take(&mut frame.frame.list_owners_to_rebuild);
    host.rebuild_list_owners_with_stale_item_counters(main_thread, &list_owners_to_rebuild);
    // SAFETY: As above.
    let over = unsafe {
        frame
            .frame
            .take_in_end(main_thread, FrameEnd::Over(FfiLayoutUpdateEnd::InUpdate))
    };
    debug_assert!(over, "a clock layout frame taken in is over");
}

/// Makes the frame a clock lease's ticks lay out in, with the document as it stands now.
///
/// # Safety
///
/// On the document thread, with no layout update running.
unsafe fn make_clock_layout_frame(
    main_thread: &crate::stage::MainThread,
    arena_handle: *mut c_void,
) -> ClockLayoutFrame {
    let host = layout_update_host(main_thread);
    let facts = host.document_facts(main_thread);
    ClockLayoutFrame {
        frame: LayoutFrame {
            inputs: FrameInputs {
                host,
                arena_handle,
                reason_is_inspect_devtools_layout_data: false,
                is_template_contents_document: false,
                trace: UpdateLayoutTrace::disabled(),
            },
            messages: FrameMessages::default(),
            layout_pass: 0,
            connected_element_count: 0,
            pass_sources: None,
            tree_build_document_style_node: None,
            list_owners_to_rebuild: Vec::new(),
            selection: host.read_selection(main_thread),
            owed_host_halves: Cell::default(),
        },
        facts,
        laid_out: false,
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
    // A frame in flight may own the arena, so this is asked without reading it. A style pass does
    // not own it, but the document's frame is in flight all the same.
    if crate::stage_thread::document_frame_in_flight(arena_handle) {
        return FfiLayoutFrameState::InFlight;
    }
    // SAFETY: Guaranteed by the caller.
    if unsafe { arena(arena_handle) }.update_layout_is_running() {
        FfiLayoutFrameState::MainInsideJoin
    } else {
        FfiLayoutFrameState::Idle
    }
}

/// Runs the layout update as one frame. The document thread drives the stabilization loop: it runs
/// the style of each round itself, and the rest of the round as a stage run, which joins the
/// document thread for the steps listed in [`FrameJoin`]. Once the frame ends, the document thread
/// takes in its end: it pays what the frame still owes, goes on with another round if that left
/// style or layout work pending, and otherwise applies the frame's messages and ends the update on
/// the document side, so the document is idle once this returns.
/// A frame whose full layout pass is submitted ends that way once the document thread takes it
/// back instead.
///
/// # Safety
///
/// As for [`arena`], on the document thread, and `inputs` must satisfy
/// [`UpdateLayoutTrace::new`]'s requirements.
unsafe fn update_layout(
    main_thread: &crate::stage::MainThread,
    arena_handle: *mut c_void,
    inputs: &FfiLayoutUpdateInputs,
) -> FfiLayoutUpdateOutcome {
    let host = layout_update_host(main_thread);
    // SAFETY: Guaranteed by the caller.
    assert!(
        unsafe { arena(arena_handle) }.update_layout_is_running(),
        "the layout update runs between layout_arena_begin_update_layout and its end"
    );
    let may_submit_pass = inputs.may_submit_pass;
    let inputs = FrameInputs {
        host,
        arena_handle,
        reason_is_inspect_devtools_layout_data: inputs.reason_is_inspect_devtools_layout_data,
        is_template_contents_document: inputs.is_template_contents_document,
        // SAFETY: Guaranteed by the caller.
        trace: unsafe { UpdateLayoutTrace::new(inputs.reason_name) },
    };
    let submits_pass = may_submit_pass && crate::stage_thread::submits("layout");
    // SAFETY: Guaranteed by the caller.
    let Some(pass) = (unsafe { LayoutPassJob::prepare(main_thread, inputs, submits_pass) }) else {
        return FfiLayoutUpdateOutcome::Finished;
    };
    if crate::stage_thread::submits_flight() {
        // SAFETY: As below.
        unsafe { crate::flight::submit(arena_handle, crate::flight::Flight::from_layout_pass(pass)) };
        return FfiLayoutUpdateOutcome::PassSubmitted;
    }
    let take_back = pass.take_back();
    // SAFETY: The frame reaches only the arena, which the frame in flight owns until the document
    // thread takes it back, and every document-thread path to the arena, the style mirror its tree
    // build walks and the tree update marks it holds joins the frame first.
    unsafe {
        crate::stage_thread::submit_stage_with_take_back(
            "layout",
            arena_handle,
            move || pass.run(),
            move || take_back.finish(),
        );
    }
    FfiLayoutUpdateOutcome::PassSubmitted
}

/// A layout frame the document thread has driven up to its full layout pass, which it hands to a
/// stage to run the rest of its round. The stage leaves the frame where [`LayoutPassTakeBack`]
/// finds it once the document thread has taken the frame back.
pub(crate) struct LayoutPassJob {
    arena_handle: usize,
    frame: crate::stage_thread::FrameOwns<LayoutFrame>,
    facts: FfiLayoutUpdateDocumentFacts,
    ran: std::sync::Arc<std::sync::Mutex<Option<crate::stage_thread::FrameOwns<LayoutFrame>>>>,
}

/// What the document thread runs once it has taken back the frame of a [`LayoutPassJob`].
pub(crate) struct LayoutPassTakeBack {
    arena_handle: *mut c_void,
    ran: std::sync::Arc<std::sync::Mutex<Option<crate::stage_thread::FrameOwns<LayoutFrame>>>>,
}

impl LayoutPassJob {
    /// Drives the layout frame of `inputs` on the document thread up to its full layout pass, if
    /// `submits_pass`, and returns the pass. Returns `None` if the frame ended on the document
    /// thread instead.
    ///
    /// # Safety
    ///
    /// As for [`update_layout`]. The pass returned has to run in a frame in flight that owns the arena.
    unsafe fn prepare(main_thread: &crate::stage::MainThread, inputs: FrameInputs, submits_pass: bool) -> Option<Self> {
        let mut frame = LayoutFrame {
            inputs,
            messages: FrameMessages::default(),
            layout_pass: 0,
            connected_element_count: 0,
            pass_sources: None,
            tree_build_document_style_node: None,
            list_owners_to_rebuild: Vec::new(),
            selection: None,
            owed_host_halves: Cell::default(),
        };
        // SAFETY: Guaranteed by the caller.
        let facts = unsafe { frame.drive(main_thread, submits_pass) }?;
        // The style round paid what the frame owed. What the round in flight comes to owe (its tree
        // build's host half, its commits' host halves) is paid as the frame is taken back: tasks that
        // run beside the flight and would read it reach it through the arena's doors, which join.
        debug_assert!(
            frame.owed_host_halves.get_mut().is_empty(),
            "the style round pays what the frame owed"
        );
        Some(Self {
            arena_handle: frame.inputs.arena_handle as usize,
            // SAFETY: Guaranteed by the caller.
            frame: unsafe { crate::stage_thread::FrameOwns::new(frame) },
            facts,
            ran: std::sync::Arc::new(std::sync::Mutex::new(None)),
        })
    }

    /// What the document thread runs once it has taken the frame back.
    pub(crate) fn take_back(&self) -> LayoutPassTakeBack {
        LayoutPassTakeBack {
            arena_handle: self.arena_handle as *mut c_void,
            ran: self.ran.clone(),
        }
    }

    /// Runs the rest of the frame's round, on the stage that owns the arena.
    pub(crate) fn run(self) {
        let Self { frame, facts, ran, .. } = self;
        let mut frame = frame.into_inner();
        // Where the frame would go on from here is the next layout update's to find: the take-back
        // ends it wherever the document thread is.
        // SAFETY: The frame in flight owns the arena, as LayoutPassJob::prepare requires.
        let _ = unsafe { frame.run_round_through_pass(facts) };
        // SAFETY: As above.
        *ran.lock().expect("a frame that ran left itself") =
            Some(unsafe { crate::stage_thread::FrameOwns::new(frame) });
    }
}

impl LayoutPassTakeBack {
    /// Ends the frame the stage ran, on the document thread, which has taken the frame back.
    pub(crate) fn finish(self) {
        let frame = self
            .ran
            .lock()
            .expect("a frame that ran left itself")
            .take()
            .expect("the frame is taken back once its round has run")
            .into_inner();
        main_thread_entries::finish_layout_frame_taken_back(self.arena_handle, frame);
    }
}

/// Ends a layout frame whose round the document thread has taken back, and then the update: takes
/// back the tree update marks, pays what the round owes, and applies the frame's messages.
/// Whatever the round leaves pending (another build, another round), the next layout update does:
/// the frame is taken back wherever the document thread reaches what it owns, which can be in the
/// middle of a DOM mutation, so it runs no style update.
///
/// # Safety
///
/// On the document thread, which has just taken back the frame in flight that ran the round.
unsafe fn finish_layout_frame(main_thread: &crate::stage::MainThread, mut frame: LayoutFrame) {
    let host = frame.inputs.host;
    // SAFETY: The frame is the document thread's again, and it runs for the update the arena is in.
    unsafe {
        pay_owed_host_halves(
            main_thread,
            &host,
            frame.inputs.arena_handle,
            frame.owed_host_halves.take(),
        );
    }
    // The list owners the round's build found showing stale counters are marked for the build of
    // the next layout update, as the next style round would have marked them.
    let list_owners_to_rebuild = std::mem::take(&mut frame.list_owners_to_rebuild);
    host.rebuild_list_owners_with_stale_item_counters(main_thread, &list_owners_to_rebuild);
    // SAFETY: As above.
    let over = unsafe { frame.take_in_end(main_thread, FrameEnd::Over(FfiLayoutUpdateEnd::FrameTakenBack)) };
    debug_assert!(over, "a frame taken back is over");
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
    crate::stage_thread::join_document_frame_in_flight_at(arena, file, line, 0);
}

/// Waits for the document's frame in flight only if one of its stages reaches the document's style
/// engine (see [`crate::stage_thread::join_frame_reaching_style_engine_at`]). `file` and `line`
/// name the C++ call site for the forced-join log.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread. `file` and `file_length` must name a
/// string that lives for the rest of the process, as a `SourceLocation`'s file name does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_join_frame_reaching_style_engine(
    arena: *mut c_void,
    file: *const u8,
    file_length: usize,
    line: u32,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The caller passes a string that lives for the rest of the process.
    let file = unsafe { crate::stage_thread::call_site_file(file, file_length) };
    crate::stage_thread::join_frame_reaching_style_engine_at(arena, file, line, 0);
}

/// Waits for the document's frame in flight only if one of its stages owns the arena (a layout pass
/// or a recording), not for a style pass alone, which records its arena without owning it. For a
/// write to what the arena's stages read. `file` and `line` name the C++ call site for the
/// forced-join log.
///
/// # Safety
///
/// As for [`layout_arena_join_frame_reaching_style_engine`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_join_frame_owning_arena(
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
    // The borrow above has joined a frame in flight that owns the arena, and the style update ahead of
    // it one that reaches the document's style engine; a layout update never runs under either.
    assert!(
        !crate::stage_thread::document_frame_in_flight(arena),
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
