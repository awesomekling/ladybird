/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The layout update: the style and layout stabilization loop a document runs before it can
//! answer geometry queries or paint. The steps that touch the DOM stay on the C++ host and
//! answer through a callback table registered once per arena. The document thread drives the
//! loop and runs those steps itself; the rest of each round runs on the render owner.

use super::LayoutNodeArena;
use super::formatting_context::{
    CommitPayment, DeferredLayoutCommitHostHalf, PendingLayoutCommit, commit_root_layout_to_arena,
    commit_subtree_layout_to_arena, compute_root_layout, compute_subtree_layout_fragments,
    prepare_root_layout_from_sources,
};
use super::layout_node_arena::{
    EnrolledContentSources, OwedImageResources, apply_enrolled_content_sources, read_enrolled_content_sources,
};
use super::node_data::NodeSlotId;
use super::node_facts;
use super::partial_relayout::FfiPartialRelayoutHostFacts;
use super::tree_builder::{
    FfiGeneratedContentItem, FfiLayoutTreeBuildOutcome, FfiPseudoElement, TreeBuildHostHalf, TreeBuildPayment,
    walk_layout_tree_build,
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
    pub document_facts: unsafe extern "C" fn(*mut c_void) -> FfiLayoutUpdateDocumentFacts,
    /// Readies the document for a layout tree build, and answers with the document's style node,
    /// which the build walks from.
    pub prepare_layout_tree_build: unsafe extern "C" fn(*mut c_void) -> u32,
    /// Takes in what the frame left for the document, and ends the layout update on the document
    /// side. The document thread calls it as it takes in the frame's end, so the frame is over for
    /// the document once the update returns. The effects are valid for the call.
    pub take_in_frame_effects: unsafe extern "C" fn(*mut c_void, *const FfiLayoutFrameEffects),
    /// Installs what the style pass a flight ran published, and runs the rest of the style update.
    pub finish_submitted_style_update: unsafe extern "C" fn(*mut c_void),
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
    /// The document holds style input it has not handed to the style engine yet (media rules to
    /// evaluate again and style attributes to read again among it), or animation effects whose
    /// style it has yet to sample.
    pub style_input_waits_on_document: bool,
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
    /// Whether the flight that ran the frame recorded the document after it, which the document
    /// publishes: the visual contexts, display list and repaint a commit asks for are that recording.
    pub recorded_in_flight: bool,
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

/// An image resource a tree build in the frame owes a row it stamped, which the document attaches once
/// the frame is over, if the row is still live then.
#[derive(Clone, Copy)]
#[repr(C, u8)]
// NB: The fields are read by C++ through the FFI.
#[allow(dead_code)]
pub enum FfiOwedImageResources {
    /// The resources the row's style asks for. Principal and pseudo-element boxes both owe these;
    /// nothing about them depends on which the box is. The flag says the box replaces its
    /// element's contents with a single image, which it owns the provider for.
    StyleResources {
        row: NodeSlotId,
        owns_content_replacement_image: bool,
    },
    /// The provider an image a pseudo-element's generated content names renders, and its box's style
    /// resources: the image's row, the element the pseudo-element is generated for, the
    /// pseudo-element, the content item, and the pseudo-element's own box.
    GeneratedImage {
        row: NodeSlotId,
        generator: u32,
        pseudo_element: FfiPseudoElement,
        item: FfiGeneratedContentItem,
        pseudo_element_box: NodeSlotId,
    },
}

impl FfiOwedImageResources {
    fn new(row: NodeSlotId, owed: OwedImageResources) -> Self {
        match owed {
            OwedImageResources::StyleResources {
                owns_content_replacement_image,
            } => Self::StyleResources {
                row,
                owns_content_replacement_image,
            },
            OwedImageResources::GeneratedImage {
                generator,
                pseudo_element,
                item,
                pseudo_element_box,
            } => Self::GeneratedImage {
                row,
                generator: generator.raw(),
                pseudo_element,
                item,
                pseudo_element_box,
            },
        }
    }
}

/// What a layout frame leaves for the document once it is over. The document takes it in as one,
/// in the order of the fields, and then ends the layout update on its side where `end` says.
#[repr(C)]
pub struct FfiLayoutFrameEffects {
    /// For a frame whose flight ran the style of its first round: the install owed the flight the
    /// repaint of the style batch, which the flight's recording is if it stands
    /// (`flight_style_repaint_recorded`), and otherwise the document paints again.
    pub settles_flight_style_repaint: bool,
    pub flight_style_repaint_recorded: bool,
    /// The image resources the frame's tree builds owe the rows they stamped, in the order the
    /// builds came to owe them: the images to load and observe, and the providers of the images
    /// that image boxes show. A later build in the frame can have freed a row it was owed for, so
    /// the document hands each over through `layout_arena_hand_over_owed_image_resources`, which
    /// answers whether the row is still live.
    pub owed_image_resources: *const FfiOwedImageResources,
    pub owed_image_resources_count: usize,
    pub full_layouts_performed: u64,
    /// What the frame's layout commits leave for the document, if one committed.
    pub commit: FfiLayoutCommitEffects,
    /// Whether the frame found nothing to lay out, and leaves the rendering preparation to the
    /// document.
    pub prepare_for_rendering: bool,
    pub end: FfiLayoutUpdateEnd,
}

/// What the document reads for a layout round once the round's style has run and the list item
/// renumbers and top layer changes it leaves are processed.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiLayoutRoundFacts {
    pub facts: FfiLayoutUpdateDocumentFacts,
    /// The document's selection range, or null when it has none.
    pub selection: *const FfiSelectionSnapshot,
}

/// A layout round's facts as the document read them (see [`FfiLayoutRoundFacts`]).
struct LayoutRoundFacts {
    facts: FfiLayoutUpdateDocumentFacts,
    /// The selection the commits of the round stamp the selection states of the boxes they build
    /// from, as nothing on the document thread changes the tree until the frame is over.
    selection: Option<SelectionSnapshot>,
}

impl LayoutRoundFacts {
    /// # Safety
    ///
    /// `round` must be valid, and so must the selection it points to, if any.
    unsafe fn from_ffi(round: &FfiLayoutRoundFacts) -> Self {
        Self {
            facts: round.facts,
            // SAFETY: Guaranteed by the caller.
            selection: unsafe { round.selection.as_ref() }
                .map(|selection| unsafe { SelectionSnapshot::from_ffi(selection) }),
        }
    }
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
    /// Whether the update's first round runs its style pass in the flight that runs its layout,
    /// which the document thread submitted ahead of the update (see
    /// `layout_arena_collect_style_pass_for_flight`), rather than having run it ahead of the update.
    pub style_in_flight: bool,
    /// The style nodes of the elements the viewport propagates from, or zero, a relayout of which
    /// the flight does not finish as a partial relayout.
    pub viewport_propagation_sources: [u32; 2],
    /// The first round's facts, which the document read once it had run the round's style ahead of
    /// the update, or submitted it for the flight.
    pub first_round: FfiLayoutRoundFacts,
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
    /// The update readied its full layout pass as a flight, which goes on to record the document once
    /// it has laid it out, if the document seals what that reads first. The document seals it, and
    /// then submits the flight (`layout_arena_submit_prepared_flight`), which ends the update as a
    /// submitted pass does.
    FlightReady,
    /// As `FlightReady`, for a flight that runs the style of the layout's first round too, which the
    /// document's pending style is. The document seals the flight's paint ahead of that style, whose
    /// render half the flight applies itself.
    FlightWithStyleReady,
    /// The round the update started has style for the document thread to run: it runs its style
    /// update, reads the round's facts after it (see [`FfiLayoutRoundFacts`]), then resumes the
    /// update with them (`layout_arena_resume_update_layout`), which goes on from there as it would
    /// have.
    NeedsStyle,
    /// As `NeedsStyle`, for a round with no style to run: the document only reads the round's facts.
    NeedsRoundFacts,
}

/// A flight a layout update readied, which waits for the document to seal what its recording reads
/// before it is submitted.
pub(crate) struct PreparedFlight {
    pass: LayoutPassJob,
    style: Option<crate::css::style::bridge::StylePassJob>,
    viewport_propagation_sources: Vec<StyleNodeID>,
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
    document_facts: unsafe extern "C" fn(*mut c_void) -> FfiLayoutUpdateDocumentFacts,
    prepare_layout_tree_build: unsafe extern "C" fn(*mut c_void) -> u32,
    take_in_frame_effects: unsafe extern "C" fn(*mut c_void, *const FfiLayoutFrameEffects),
    finish_submitted_style_update: unsafe extern "C" fn(*mut c_void),
}

impl From<FfiLayoutUpdateHostCallbacks> for LayoutUpdateHost {
    fn from(host: FfiLayoutUpdateHostCallbacks) -> Self {
        Self {
            context: host.context,
            document_facts: host.document_facts,
            prepare_layout_tree_build: host.prepare_layout_tree_build,
            take_in_frame_effects: host.take_in_frame_effects,
            finish_submitted_style_update: host.finish_submitted_style_update,
        }
    }
}

impl LayoutUpdateHost {
    // SAFETY (for every call below): The C++ host answers synchronously from its live document.
    fn finish_submitted_style_update(&self, _: &crate::stage::MainThread) {
        unsafe { (self.finish_submitted_style_update)(self.context) }
    }

    fn document_facts(&self, _: &crate::stage::MainThread) -> FfiLayoutUpdateDocumentFacts {
        unsafe { (self.document_facts)(self.context) }
    }

    fn prepare_layout_tree_build(&self, _: &crate::stage::MainThread) -> u32 {
        unsafe { (self.prepare_layout_tree_build)(self.context) }
    }

    fn take_in_frame_effects(&self, _: &crate::stage::MainThread, effects: &FfiLayoutFrameEffects) {
        unsafe { (self.take_in_frame_effects)(self.context, effects) }
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

/// Whether the document's style is to be updated again after its layout: for input the document
/// still holds, for a container a style computation asked about before it had a box, or for a
/// transaction the style engine has pending.
fn style_update_follows_layout(arena: &LayoutNodeArena, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
    facts.style_input_waits_on_document
        || arena.with_style_store(|engine| {
            engine.has_size_containers_needing_evaluation_after_layout() || engine.has_pending_transaction()
        })
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

/// What a layout frame is started with. The document drains its invalidation journal into the
/// arena before the frame starts; the document facts the frame reads are snapshots the document
/// thread hands it at each step it runs.
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

/// What a finished layout frame leaves for the document thread, which takes it in as one once the
/// frame is over (see [`FfiLayoutFrameEffects`]).
#[must_use]
#[derive(Default)]
struct FrameMessages {
    /// For a frame whose flight ran the style of its first round: whether the flight's recording is
    /// the repaint the install of the style batch owes it.
    flight_style_repaint_recorded: Option<bool>,
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
    /// Whether the flight that ran the frame recorded the document after it.
    recorded_in_flight: bool,
}

impl FrameMessages {
    /// Hands the document what the frame left for it, and ends the update on the document side where
    /// `end` says.
    fn take_in(self, main_thread: &crate::stage::MainThread, host: &LayoutUpdateHost, end: FfiLayoutUpdateEnd) {
        assert!(
            !self.stabilization_bound_failed,
            "the layout update did not stabilize within its exact bound"
        );
        let owed_image_resources: Vec<_> = self
            .owed_image_resources
            .into_iter()
            .map(|(row, owed)| FfiOwedImageResources::new(row, owed))
            .collect();
        let boxes = self.boxes_with_auto_content_visibility.as_deref();
        host.take_in_frame_effects(
            main_thread,
            &FfiLayoutFrameEffects {
                settles_flight_style_repaint: self.flight_style_repaint_recorded.is_some(),
                flight_style_repaint_recorded: self.flight_style_repaint_recorded == Some(true),
                owed_image_resources: owed_image_resources.as_ptr(),
                owed_image_resources_count: owed_image_resources.len(),
                full_layouts_performed: self.full_layouts_performed,
                commit: FfiLayoutCommitEffects {
                    layout_committed: self.layout_committed,
                    layout_tree_changed: self.layout_tree_changed,
                    boxes_with_auto_content_visibility_collected: boxes.is_some(),
                    boxes_with_auto_content_visibility: boxes.map_or(std::ptr::null(), <[NodeSlotId]>::as_ptr),
                    boxes_with_auto_content_visibility_count: boxes.map_or(0, <[NodeSlotId]>::len),
                    clamped_scroll_offsets: self.clamped_scroll_offsets.as_ptr(),
                    clamped_scroll_offsets_count: self.clamped_scroll_offsets.len(),
                    shown_on_render_side: self.shown_on_render_side,
                    recorded_in_flight: self.recorded_in_flight,
                },
                prepare_for_rendering: self.prepare_for_rendering,
                end,
            },
        );
    }
}

/// What a tree build or a commit the frame made owes the document thread beyond its own join, as the work left it.
/// The frame resolves it from the arena as the work that owes it ends ([`LayoutFrame::resolve_owed_host_halves`]),
/// into what the document thread pays.
enum OwedHostHalf {
    TreeBuild(TreeBuildHostHalf),
    Commit(DeferredLayoutCommitHostHalf),
}

/// What the frame owes the document thread, as typed effects it applies without the arena.
enum HostHalfPayment {
    /// The install of the style pass the frame's first round ran in its flight, and the rest of
    /// the style update, which come before what the round's layout owes.
    StyleInstall,
    TreeBuild(TreeBuildPayment),
    Commit(CommitPayment),
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
    payments: Vec<HostHalfPayment>,
) -> bool {
    let mut restored_style = false;
    // What the build owes the document can mark nodes for another build.
    // SAFETY: Guaranteed by the caller.
    unsafe { super::tree_update_marks::take_back_from_frame(arena_handle) };
    let mut committed = false;
    for payment in payments {
        match payment {
            // The install is the rest of the style update, as the commit of a style flight runs it
            // at the join that takes the flight back, forced or not: a forced join in the middle of
            // a DOM mutation runs it there as it does for the style flight.
            // What applying the batch in flight handed back, the install reads: it is paid first.
            HostHalfPayment::StyleInstall => {
                // SAFETY: Guaranteed by the caller.
                unsafe { arena(arena_handle) }.pay_flight_style_handbacks(main_thread);
                host.finish_submitted_style_update(main_thread);
                // SAFETY: Guaranteed by the caller.
                restored_style |= unsafe { arena(arena_handle) }.finish_flight_style_host_half(main_thread);
            }
            HostHalfPayment::TreeBuild(payment) => payment.pay(main_thread),
            HostHalfPayment::Commit(payment) => {
                // SAFETY: Guaranteed by the caller.
                unsafe { payment.deliver(main_thread) };
                committed = true;
            }
        }
    }
    // A document that traces its layout names the owners of the lines the passes left.
    if committed && main_thread.host_tables().is_some_and(super::HostTables::traces_layout) {
        // SAFETY: Guaranteed by the caller. Host callbacks have returned.
        unsafe { arena(arena_handle) }.name_layout_trace_owners(main_thread);
    }
    // What the document thread wrote to the marks beside the frame, it wrote after all of that.
    // SAFETY: Guaranteed by the caller. The marks are handed back.
    unsafe { super::tree_update_marks::write_marks_waiting_for_frame(arena_handle) };
    restored_style
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

/// Where a style round's style runs.
enum RoundStyle {
    /// On the document thread, as the round starts, which the update returns to for it: a round
    /// after the first.
    OnDocumentThread,
    /// On the document thread ahead of the update: the first round's, with the facts the document
    /// read after it.
    RanAhead(LayoutRoundFacts),
    /// In the flight, as the pass the document thread submitted ahead of the update: the first
    /// round's, with the facts the document read after submitting it.
    InFlight(LayoutRoundFacts),
}

/// The style and layout stabilization loop of one layout update. It holds no borrow of the unit it
/// is in, so a round can stop ahead of its full layout pass (for a flight) and go on once the pass
/// has run.
struct LayoutFrame {
    inputs: FrameInputs,
    messages: FrameMessages,
    /// The rounds the loop has started, and the connected element count the last style round
    /// answered, which bound them.
    layout_pass: u64,
    connected_element_count: u32,
    /// The sources the last style round read for the layout pass that follows it.
    pass_sources: Option<LayoutPassSources>,
    /// The document style node of the tree build the style round readied.
    tree_build_document_style_node: Option<u32>,
    /// The document's style node, as the document thread readied the document for a tree build of the round with.
    document_style_node: u32,
    /// The document's selection as the style round of the round read it, if it has one.
    selection: Option<SelectionSnapshot>,
    /// What the frame's tree builds and commits owe the document thread, in the order the frame
    /// made them, until the unit that made them resolves them into `host_payments`.
    owed_host_halves: Cell<Vec<OwedHostHalf>>,
    /// What the frame owes the document thread, which the next step on the document thread pays before its work.
    host_payments: Vec<HostHalfPayment>,
    /// Where the next style round's style runs.
    round_style: RoundStyle,
    /// The style pass the first round collected, which the flight runs as the round's style.
    style_pass: Option<crate::css::style::bridge::StylePassJob>,
    /// Whether the frame's first round ran its style in the flight.
    style_ran_in_flight: bool,
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

/// Where the loop of a layout frame the document thread drives stops.
enum DriveStop {
    /// The frame has ended, and the update with it.
    Ended,
    /// The first round that lays out has run its style, and the rest of it runs with these facts.
    AtRound(FfiLayoutUpdateDocumentFacts),
    /// The round the loop started goes on once the document thread has read its facts, running
    /// its style first if `runs_style`.
    ForDocument { runs_style: bool },
}

/// A unit of a layout frame the document thread waits for, which the render owner runs with the document's render
/// state: the rest of a round whose style the document thread has run, up to the next step that needs the document
/// thread (the next round's style, or the frame's end). The frame stays on the document thread's stack meanwhile.
pub(crate) struct OwnerLayoutUnit {
    frame: crate::stage_thread::CallerWaits<*mut LayoutFrame>,
    facts: FfiLayoutUpdateDocumentFacts,
    reply: crate::stage_thread::OwnerReplyTo<OwnerLayoutStep>,
    /// How the owner runs it. The owner reaches the layout pipeline only through the units it is sent, so what
    /// reaches the owner without reaching the pipeline (the unit tests' stage threads) links without it.
    run: unsafe fn(*mut LayoutFrame, FfiLayoutUpdateDocumentFacts) -> FrameStep,
}

/// Runs `run`, a unit of the frame `frame`, with `facts`, and resolves what the unit left the frame owing the document
/// thread, which the thread pays once it has the unit's answer.
///
/// # Safety
///
/// Nothing but the thread running the unit reaches the frame and its arena until this returns.
unsafe fn run_unit(
    run: unsafe fn(*mut LayoutFrame, FfiLayoutUpdateDocumentFacts) -> FrameStep,
    frame: *mut LayoutFrame,
    facts: FfiLayoutUpdateDocumentFacts,
) -> FrameStep {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        let step = run(frame, facts);
        (*frame).resolve_owed_host_halves();
        step
    }
}

/// Where a frame goes on once the owner has run a unit of it.
pub(crate) struct OwnerLayoutStep(crate::stage_thread::CallerWaits<FrameStep>);

impl OwnerLayoutUnit {
    /// Runs the unit on the owner, with the arena of the render state it holds for the unit's document, and answers
    /// the waiting document thread. Where the owner holds none (a bug of the sender's), the unit runs with the arena
    /// its frame names.
    pub(crate) fn run(self, arena: impl FnOnce() -> Option<*mut c_void>) {
        let Self {
            frame,
            facts,
            reply,
            run,
        } = self;
        let frame = frame.into_inner();
        reply.answer(|| {
            // SAFETY: The document thread waits for the unit, and reaches neither the frame nor the arena meanwhile.
            let frame_arena = unsafe { (*frame).inputs.arena_handle };
            let arena = arena();
            debug_assert_eq!(Some(frame_arena), arena, "a unit runs with its document's arena");
            let arena = arena.unwrap_or(frame_arena);
            // The faces the round wants are its document's, for that document's layout end to request.
            let _wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(arena as u64);
            // SAFETY: As above.
            let step = unsafe { run_unit(run, frame, facts) };
            // SAFETY: The step goes back to the document thread, which waits for it.
            OwnerLayoutStep(unsafe { crate::stage_thread::CallerWaits::new(step) })
        });
    }

    /// Readies the rest of the round whose style ran, and runs it.
    ///
    /// # Safety
    ///
    /// Nothing but the owner reaches the frame and its arena until this returns.
    unsafe fn ready_and_run_round(frame: *mut LayoutFrame, facts: FfiLayoutUpdateDocumentFacts) -> FrameStep {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            (*frame).ready_round(&facts);
            (*frame).run_round_through_pass(facts)
        }
    }

    /// Readies the round to go in flight, where it lays out; a round that does not ends the frame at once.
    ///
    /// # Safety
    ///
    /// As for [`Self::ready_and_run_round`].
    unsafe fn ready_round_for_flight(frame: *mut LayoutFrame, facts: FfiLayoutUpdateDocumentFacts) -> FrameStep {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            match (*frame).ready_round(&facts) {
                (true, builds) => FrameStep::RoundReady { facts, builds },
                (false, _) => (*frame).run_round_through_pass(facts),
            }
        }
    }

    /// # Safety
    ///
    /// As for [`Self::ready_and_run_round`].
    unsafe fn end_unless_host_left_work(frame: *mut LayoutFrame, facts: FfiLayoutUpdateDocumentFacts) -> FrameStep {
        // SAFETY: Guaranteed by the caller.
        unsafe { (*frame).go_on_after_host_halves(facts, true) }
    }

    /// # Safety
    ///
    /// As for [`Self::ready_and_run_round`].
    unsafe fn start_round(frame: *mut LayoutFrame, facts: FfiLayoutUpdateDocumentFacts) -> FrameStep {
        // SAFETY: Guaranteed by the caller.
        unsafe { (*frame).go_on_after_host_halves(facts, false) }
    }
}

/// Where a layout frame goes on once a step of it has run.
enum FrameStep {
    /// Another round, whose style the document thread runs first, with no stage run outstanding.
    NeedsStyle,
    /// A round has readied its full layout pass.
    PassReady(PendingLayoutPass),
    /// The loop is over, and the frame ends as the value says.
    Ended(FrameEnd),
    /// Another round has started, which goes on once the document thread has read its facts, running its style
    /// first where `document_style`.
    RoundStarted { document_style: bool },
    /// The round whose style ran has readied the rest of it to go in flight with `facts`, and `builds` a tree
    /// first.
    RoundReady {
        facts: FfiLayoutUpdateDocumentFacts,
        builds: bool,
    },
}

impl LayoutFrame {
    fn new(inputs: FrameInputs, round_style: RoundStyle) -> Self {
        Self {
            inputs,
            messages: FrameMessages::default(),
            layout_pass: 0,
            connected_element_count: 0,
            pass_sources: None,
            tree_build_document_style_node: None,
            document_style_node: 0,
            selection: None,
            owed_host_halves: Cell::default(),
            host_payments: Vec::new(),
            round_style,
            style_pass: None,
            style_ran_in_flight: false,
        }
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

    /// Resolves what the frame's tree builds and commits owe the document thread from the arena, as the unit that
    /// made them ends, for the document thread to pay without the arena, after what it owed before.
    ///
    /// # Safety
    ///
    /// On the thread that owns the arena, with nothing else reaching it.
    unsafe fn resolve_owed_host_halves(&mut self) {
        for owed in self.owed_host_halves.take() {
            let payment = match owed {
                OwedHostHalf::TreeBuild(owed) => HostHalfPayment::TreeBuild(owed.resolve(self.arena())),
                // SAFETY: Guaranteed by the caller.
                OwedHostHalf::Commit(owed) => HostHalfPayment::Commit(unsafe { owed.resolve() }),
            };
            self.host_payments.push(payment);
        }
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

    /// Readies the rest of the round the document thread has run the style of, with the facts `facts` it read after
    /// it, on the owner: the tree build when one comes first, with the document's style node the document thread
    /// prepared it with, and otherwise the sources of the layout pass. Answers whether the round lays out, and
    /// whether it builds. The style a flight runs decides whether the round lays out, and the flight reads the
    /// sources of its pass once it has applied that style.
    fn ready_round(&mut self, facts: &FfiLayoutUpdateDocumentFacts) -> (bool, bool) {
        self.pass_sources = None;
        self.tree_build_document_style_node = None;
        self.connected_element_count = self
            .arena()
            .with_style_store(|engine| engine.tree().connected_element_count());
        if self.style_pass.is_some() {
            return (true, false);
        }
        if !self.round_lays_out_in_frame(facts) {
            return (false, false);
        }
        if self.needs_layout_tree_rebuild(facts) {
            self.tree_build_document_style_node = Some(self.document_style_node);
            return (true, true);
        }
        // SAFETY: The frame runs for the update the arena is in, on the thread that owns it.
        self.pass_sources = Some(unsafe { LayoutPassSources::read(self.inputs.arena_handle) });
        (true, false)
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

    /// Settles the list owners with stale counters after a tree build, and has the build's host half
    /// report the ones the build found showing them, which the document marks for a layout tree
    /// rebuild as the next join pays the half. Answers whether the build found any.
    fn reconcile_stale_list_item_counters(
        &self,
        walked: &WalkedLayoutTreeBuild,
        host_half: &mut TreeBuildHostHalf,
    ) -> bool {
        let list_owners = self
            .arena()
            .reconcile_stale_list_item_counters_after_tree_build(walked.document_style_node);
        host_half.report_list_owners_with_stale_item_counters(&list_owners);
        !list_owners.is_empty()
    }

    /// What derives from a layout commit, once its arena half has settled: the rendering preparation,
    /// the selection states of the boxes it built are stamped again from the round's selection, the
    /// searchable text is dropped, and after a tree change the boxes with `content-visibility: auto`
    /// are collected again for the document's paint state, and the document's viewport clients are
    /// to be told the viewport rect.
    /// Whether nothing the frame leaves for the document thread changes what a recording made from
    /// the arena now would show: no scroll offsets to store, no image resources to attach, and no
    /// boxes whose relevance to the user the rendering update determines. A round whose build found
    /// list owners to rebuild goes on to another round, so it is not painted either.
    ///
    /// Nor may a round that resized a navigable the document hosts: that navigable lays itself out
    /// at its new size later in the rendering update and paints then, and a frame of the document
    /// handed off before would compose the navigable's frame at its old size.
    fn may_be_painted_before_take_back(&self) -> bool {
        self.messages.clamped_scroll_offsets.is_empty()
            && self.messages.owed_image_resources.is_empty()
            && !self.messages.prepare_for_rendering
            && !self.arena().may_have_auto_content_visibility()
            && !self.resized_a_hosted_navigable()
    }

    fn resized_a_hosted_navigable(&self) -> bool {
        let owed = self.owed_host_halves.take();
        let resized = owed.iter().any(|owed| match owed {
            OwedHostHalf::TreeBuild(_) => false,
            OwedHostHalf::Commit(commit) => commit.resized_a_hosted_navigable(),
        });
        self.owed_host_halves.set(owed);
        resized
    }

    /// Whether what the frame owes the document thread leaves it no style or layout work: no tree
    /// build's host half, and commits whose host halves ask for nothing again.
    fn owes_the_host_no_work(&self) -> bool {
        let owed = self.owed_host_halves.take();
        let owes_no_work = owed.iter().all(|owed| match owed {
            OwedHostHalf::TreeBuild(_) => false,
            OwedHostHalf::Commit(commit) => commit.leaves_the_host_no_work(),
        });
        self.owed_host_halves.set(owed);
        owes_no_work
    }

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

    /// Takes the frame's end in on the document thread, with no stage run outstanding: pays what the frame owed,
    /// applies the frame's messages, and ends the update on the document side where `end` says.
    ///
    /// # Safety
    ///
    /// As for [`arena`], on the document thread, with no stage run of the frame outstanding.
    unsafe fn take_in_end(&mut self, main_thread: &crate::stage::MainThread, end: FfiLayoutUpdateEnd) {
        let host = self.inputs.host;
        // SAFETY: Guaranteed by the caller.
        unsafe {
            pay_owed_host_halves(
                main_thread,
                &host,
                self.inputs.arena_handle,
                std::mem::take(&mut self.host_payments),
            )
        };
        std::mem::take(&mut self.messages).take_in(main_thread, &host, end);
    }

    /// Pays what the frame owes the document thread, which may leave the document style or layout work, and reads
    /// the document's facts after it, which the owner goes on from.
    ///
    /// # Safety
    ///
    /// As for [`Self::take_in_end`].
    unsafe fn pay_and_read_facts(&mut self, main_thread: &crate::stage::MainThread) -> FfiLayoutUpdateDocumentFacts {
        let host = self.inputs.host;
        // SAFETY: Guaranteed by the caller.
        unsafe {
            pay_owed_host_halves(
                main_thread,
                &host,
                self.inputs.arena_handle,
                std::mem::take(&mut self.host_payments),
            )
        };
        host.document_facts(main_thread)
    }

    /// Runs the frame's loop from the document thread, with the render owner. Each round starts here, with no unit
    /// of the frame outstanding: the steps that need the document thread (the host steps of each round's style
    /// update, paying what the frame owes it, applying the frame's messages) run right here, as the document thread's
    /// own, and each style transaction of the round, the rest of the round, and where the frame goes on once this
    /// thread has paid what it owed (the frame's end, another round, running out of rounds) run on the owner, with the
    /// document's render state, while this thread waits. Nothing joins this thread meanwhile. A round after the first
    /// stops the loop for the
    /// document to read its facts, running its style first if it has some, and the loop goes on `resumed_with` them.
    /// With `stop_at_round`, the loop stops once the style of the first round that lays out has run, and answers the
    /// facts the rest of that round runs with, for it to go in flight, and the frame has not ended; otherwise the loop
    /// goes on until the frame has ended.
    ///
    /// # Safety
    ///
    /// On the document thread, for the update the arena is in.
    unsafe fn drive(
        &mut self,
        main_thread: &crate::stage::MainThread,
        stop_at_round: bool,
        mut resumed_with: Option<LayoutRoundFacts>,
    ) -> DriveStop {
        // SAFETY: Guaranteed by the caller.
        let document = unsafe { super::ArenaHandle::document_of(self.inputs.arena_handle) };
        let mut step = FrameStep::NeedsStyle;
        loop {
            // SAFETY (for the steps below): Guaranteed by the caller, and no unit of the frame is outstanding.
            step = match step {
                FrameStep::Ended(FrameEnd::Over(end)) => {
                    unsafe { self.take_in_end(main_thread, end) };
                    return DriveStop::Ended;
                }
                // What the frame owed may leave the document work for another round, which the owner finds once this
                // thread has paid it.
                FrameStep::Ended(FrameEnd::UnlessHostLeftWork) => {
                    let facts = unsafe { self.pay_and_read_facts(main_thread) };
                    self.run_unit_on_owner(document, facts, OwnerLayoutUnit::end_unless_host_left_work)
                }
                FrameStep::NeedsStyle => match resumed_with.take() {
                    Some(round) => {
                        let facts = self.finish_style_round(main_thread, round);
                        if stop_at_round {
                            self.run_unit_on_owner(document, facts, OwnerLayoutUnit::ready_round_for_flight)
                        } else {
                            // The unit may build, which reads the marks: they are its until this thread pays what
                            // the unit owes it.
                            unsafe { super::tree_update_marks::lend_to_frame(self.inputs.arena_handle) };
                            self.run_unit_on_owner(document, facts, OwnerLayoutUnit::ready_and_run_round)
                        }
                    }
                    None if !matches!(self.round_style, RoundStyle::OnDocumentThread) => {
                        resumed_with = Some(unsafe { self.start_first_round(main_thread) });
                        FrameStep::NeedsStyle
                    }
                    None => {
                        let facts = unsafe { self.pay_and_read_facts(main_thread) };
                        self.run_unit_on_owner(document, facts, OwnerLayoutUnit::start_round)
                    }
                },
                FrameStep::RoundReady { facts, builds } => {
                    // The flight's build reads the marks until the document thread takes the frame back.
                    if builds {
                        unsafe { super::tree_update_marks::lend_to_frame(self.inputs.arena_handle) };
                    }
                    return DriveStop::AtRound(facts);
                }
                FrameStep::RoundStarted { document_style } => {
                    return DriveStop::ForDocument {
                        runs_style: document_style,
                    };
                }
                FrameStep::PassReady(pass) => {
                    // A unit ends at a step that needs the document thread, which a ready pass does not.
                    debug_assert!(false, "a unit on the owner runs the pass it readies");
                    // With the pass in hand, the frame goes on as the unit would have.
                    let laid_out = unsafe { pass.run() };
                    let step = self.finish_round(laid_out);
                    unsafe { self.resolve_owed_host_halves() };
                    step
                }
            };
        }
    }

    /// Runs the frame with the render owner, as [`Self::drive`] does, and asks it `query` about `document` once the
    /// frame is over. A round after the first stops the loop for the document to read its facts, running its style
    /// first if `runs_style`, which answers `Err(runs_style)` and goes on `resumed_with` the facts.
    ///
    /// # Safety
    ///
    /// As for [`Self::drive`].
    unsafe fn drive_on_owner(
        &mut self,
        main_thread: &crate::stage::MainThread,
        document: crate::render_owner::DocumentId,
        query: crate::render_owner::Query,
        resumed_with: Option<LayoutRoundFacts>,
    ) -> Result<crate::render_owner::Answer, bool> {
        // SAFETY: Guaranteed by the caller.
        match unsafe { self.drive(main_thread, false, resumed_with) } {
            DriveStop::Ended => {}
            DriveStop::ForDocument { runs_style } => return Err(runs_style),
            DriveStop::AtRound(_) => {
                // The owner answers from the arena as far as the frame laid it out.
                debug_assert!(false, "a frame driven to its end does not stop at a round");
            }
        }
        // SAFETY: The frame is over, and it ran for the update the arena is in.
        Ok(unsafe { crate::render_owner::ask(document, self.inputs.arena_handle, query) })
    }

    /// Runs `run`, the next unit of the frame, with `facts` on the render owner, which holds `document`'s render state,
    /// and waits for where the frame goes on.
    fn run_unit_on_owner(
        &mut self,
        document: crate::render_owner::DocumentId,
        facts: FfiLayoutUpdateDocumentFacts,
        run: unsafe fn(*mut LayoutFrame, FfiLayoutUpdateDocumentFacts) -> FrameStep,
    ) -> FrameStep {
        let frame = std::ptr::from_mut(self);
        let outcome = crate::stage_thread::wait_for_owner(
            |reply| crate::render_owner::ToOwner::Layout {
                document,
                unit: Box::new(OwnerLayoutUnit {
                    // SAFETY: This thread waits for the unit, and reaches the frame only once it has the answer.
                    frame: unsafe { crate::stage_thread::CallerWaits::new(frame) },
                    facts,
                    reply,
                    run,
                }),
            },
            // SAFETY: The frame runs for the update the arena is in, on the document thread, which waits for nothing.
            || OwnerLayoutStep(unsafe { crate::stage_thread::CallerWaits::new(run_unit(run, frame, facts)) }),
        );
        match outcome {
            Ok(OwnerLayoutStep(step)) => step.into_inner(),
            // A layout unit that panicked on the owner panics here, as a stage the document thread waits for does.
            Err(payload) => std::panic::resume_unwind(payload),
        }
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

    /// Starts the frame's first round on the document thread, with no stage run outstanding, and answers the facts
    /// the document read for it ahead of the update. Its style is the document's to run ahead of the update, or to
    /// submit for the flight to run. The document only submits it when the round lays out the tree it has, which the
    /// flight then styles: the round readies no tree build.
    ///
    /// # Safety
    ///
    /// The frame must run for the update the arena is in, and no stage may reach it until this returns.
    unsafe fn start_first_round(&mut self, main_thread: &crate::stage::MainThread) -> LayoutRoundFacts {
        self.layout_pass += 1;
        let host = self.inputs.host;
        // SAFETY: Guaranteed by the caller.
        unsafe {
            pay_owed_host_halves(
                main_thread,
                &host,
                self.inputs.arena_handle,
                std::mem::take(&mut self.host_payments),
            );
        }
        match std::mem::replace(&mut self.round_style, RoundStyle::OnDocumentThread) {
            RoundStyle::RanAhead(round) => round,
            RoundStyle::InFlight(round) => {
                self.style_pass = crate::css::style::bridge::take_style_pass_collected_for_flight();
                round
            }
            RoundStyle::OnDocumentThread => {
                debug_assert!(false, "the first round's facts come with the update");
                // The round goes on with the facts as they are, and without a selection.
                LayoutRoundFacts {
                    facts: host.document_facts(main_thread),
                    selection: None,
                }
            }
        }
    }

    /// Goes on, on the owner, from what the frame owed the document thread, which the document thread has paid, and
    /// the facts `facts` it read after that. Where the frame `ending` found its last commit stable, it ends, unless
    /// paying left the document style or layout work. Otherwise another round starts, unless the loop has run out of
    /// rounds, which ends the frame. A round after the first has style to run only if the rounds before it left some:
    /// what their layout noted in the engine, or what paying handed the document. Otherwise the document's style update
    /// would find nothing to do.
    fn go_on_after_host_halves(&mut self, facts: FfiLayoutUpdateDocumentFacts, ending: bool) -> FrameStep {
        if ending
            && !style_update_follows_layout(self.arena(), &facts)
            && !facts.top_layer_work_pending
            && layout_is_up_to_date(self.arena(), &facts)
        {
            return FrameStep::Ended(FrameEnd::Over(FfiLayoutUpdateEnd::InUpdate));
        }
        if !self.may_start_round() {
            return FrameStep::Ended(self.end_out_of_rounds(facts));
        }
        self.layout_pass += 1;
        let document_style = style_update_follows_layout(self.arena(), &facts)
            || self
                .arena()
                .with_style_store(|engine| engine.has_deferred_element_style_inputs());
        FrameStep::RoundStarted { document_style }
    }

    /// Goes on with the round the loop started, once its style has run, with the facts the document read after the
    /// list item renumbers and top layer changes the style left, and its selection, and readies the document for a
    /// tree build the round may run. Style is the document's own loop over its elements, and a tree update mark is set
    /// on the DOM node, which widens it to what the node's layout node and its document ask for. The commits of the
    /// round stamp the selection states of the boxes they build from the selection as it is now, as nothing on the
    /// document thread changes the tree until the frame is over.
    fn finish_style_round(
        &mut self,
        main_thread: &crate::stage::MainThread,
        round: LayoutRoundFacts,
    ) -> FfiLayoutUpdateDocumentFacts {
        let LayoutRoundFacts { facts, selection } = round;
        self.selection = selection;
        // A round whose style runs in the flight lays out the tree it has, and a document hosting template contents
        // lays out nothing.
        if self.style_pass.is_none() && !self.inputs.is_template_contents_document {
            self.document_style_node = self.inputs.host.prepare_layout_tree_build(main_thread);
        }
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
            let (walked, mut host_half) = self.walk_layout_tree_build();
            let needs_another_build_pass = walked.outcome.needs_another_build_pass;
            let counters_were_stale =
                !needs_another_build_pass && self.reconcile_stale_list_item_counters(&walked, &mut host_half);
            let pass_follows = !needs_another_build_pass && !counters_were_stale;
            // SAFETY: The frame runs for the update the arena is in.
            let pass_sources = pass_follows.then(|| unsafe { LayoutPassSources::read(arena_handle) });
            self.owe_tree_build_host_half(host_half);
            self.note_layout_tree_build(&walked.outcome);
            if needs_another_build_pass {
                return FrameStep::NeedsStyle;
            }

            // The full layout below covers every boundary the build's invalidation registered.
            drop(self.arena().take_partial_relayout_boundary_roots());

            // The list owners the reconciliation reported are marked for another build as the
            // next style round pays the build's host half, after the reset of the full tree update
            // flag.
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

    /// Ends a loop that has run out of rounds, noting whether style or layout work is still pending.
    fn end_out_of_rounds(&mut self, facts: FfiLayoutUpdateDocumentFacts) -> FrameEnd {
        if style_update_follows_layout(self.arena(), &facts) || !layout_is_up_to_date(self.arena(), &facts) {
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
            let (walked, mut host_half) = self.walk_layout_tree_build();
            let needs_another_build_pass = walked.outcome.needs_another_build_pass;
            let counters_were_stale = self.reconcile_stale_list_item_counters(&walked, &mut host_half);
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
    /// How many rounds laid out in the frame, each of which owes the document its host half until
    /// the frame is taken in.
    rounds: u32,
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
        self.rounds += 1;
        // SAFETY: Guaranteed by the caller.
        let step = unsafe { self.frame.run_round_through_pass(self.facts) };
        // SAFETY: As above.
        unsafe { self.frame.resolve_owed_host_halves() };
        match step {
            FrameStep::Ended(_) => !self.frame.commit_left_layout_work(&self.facts),
            FrameStep::NeedsStyle
            | FrameStep::RoundStarted { .. }
            | FrameStep::RoundReady { .. }
            | FrameStep::PassReady(_) => false,
        }
    }

    pub(crate) fn laid_out(&self) -> bool {
        self.laid_out
    }

    pub(crate) fn rounds(&self) -> u32 {
        self.rounds
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
            std::mem::take(&mut frame.frame.host_payments),
        );
    }
    // SAFETY: As above.
    unsafe { frame.frame.take_in_end(main_thread, FfiLayoutUpdateEnd::InUpdate) };
}

/// Makes the frame a clock lease's ticks lay out in, with the document as it stands now, which the
/// document read into `round`.
///
/// # Safety
///
/// On the document thread, with no layout update running.
unsafe fn make_clock_layout_frame(
    main_thread: &crate::stage::MainThread,
    arena_handle: *mut c_void,
    round: LayoutRoundFacts,
) -> ClockLayoutFrame {
    let host = layout_update_host(main_thread);
    let LayoutRoundFacts { facts, selection } = round;
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
            document_style_node: 0,
            selection,
            owed_host_halves: Cell::default(),
            host_payments: Vec::new(),
            round_style: RoundStyle::OnDocumentThread,
            style_pass: None,
            style_ran_in_flight: false,
        },
        facts,
        laid_out: false,
        rounds: 0,
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
    /// The document thread runs as part of the frame: it drives the frame itself. What it marks is
    /// what the frame reads next, so it goes through at once.
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

/// Runs the layout update as one frame. The document thread drives the stabilization loop: it starts
/// each round itself, and the render owner runs the rest of the round as a unit the document thread
/// waits for, which joins nothing. A round after the first that has style to run returns to
/// the document for it ([`FfiLayoutUpdateOutcome::NeedsStyle`]), which resumes the update once it
/// has run it. Once the frame ends, the document thread takes in its end: it pays what the frame
/// still owes, goes on with another round if that left style or layout work pending, and otherwise
/// applies the frame's messages and ends the update on the document side, so the document is idle
/// once the update returns finished.
/// A frame whose full layout pass is submitted ends that way once the document thread takes it
/// back instead.
///
/// # Safety
///
/// As for [`arena`], on the document thread, and `inputs` must satisfy
/// [`UpdateLayoutTrace::new`]'s requirements. The first round's selection, if any, must be valid.
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
    let style_in_flight = inputs.style_in_flight;
    // SAFETY: Guaranteed by the caller.
    let first_round = unsafe { LayoutRoundFacts::from_ffi(&inputs.first_round) };
    let first_round_style = if style_in_flight {
        RoundStyle::InFlight(first_round)
    } else {
        RoundStyle::RanAhead(first_round)
    };
    let viewport_propagation_sources = inputs
        .viewport_propagation_sources
        .iter()
        .filter_map(|&style_node| StyleNodeID::from_raw(style_node))
        .collect::<Vec<_>>();
    let inputs = FrameInputs {
        host,
        arena_handle,
        reason_is_inspect_devtools_layout_data: inputs.reason_is_inspect_devtools_layout_data,
        is_template_contents_document: inputs.is_template_contents_document,
        // SAFETY: Guaranteed by the caller.
        trace: unsafe { UpdateLayoutTrace::new(inputs.reason_name) },
    };
    let submits_pass = may_submit_pass && crate::stage_thread::submits("layout");
    debug_assert!(
        !style_in_flight || runs_style_in_flight(may_submit_pass),
        "the document submits the style of a layout update that runs it in the flight"
    );
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { super::ArenaHandle::document_of(arena_handle) };
    if !submits_pass && let Some(query) = crate::render_owner::asked_about(document) {
        // The update runs with the owner, which answers the query the document thread asked from it.
        let frame = LayoutFrame::new(inputs, first_round_style);
        // SAFETY: Guaranteed by the caller.
        return unsafe { go_on_with_owner(main_thread, frame, document, query, None) };
    }
    // SAFETY: Guaranteed by the caller.
    let driven = unsafe { LayoutPassJob::prepare(main_thread, inputs, submits_pass, first_round_style) };
    // SAFETY: As above.
    unsafe {
        go_on_from_driven_frame(
            main_thread,
            arena_handle,
            driven,
            submits_pass,
            viewport_propagation_sources,
        )
    }
}

/// A layout update whose frame stopped for the document thread to run the style of the round it
/// started, which the document resumes once it has.
pub(crate) struct ParkedLayoutUpdate {
    frame: Box<LayoutFrame>,
    submits_pass: bool,
    viewport_propagation_sources: Vec<StyleNodeID>,
    /// The query the frame runs with the owner for, which the owner answers once the frame is over.
    asked: Option<(crate::render_owner::DocumentId, crate::render_owner::Query)>,
}

/// Resumes the layout update of the document, which returned to the document thread for the style
/// and the facts of the round it started, with the facts the document read into `round`.
///
/// # Safety
///
/// As for [`update_layout`], after it answered [`FfiLayoutUpdateOutcome::NeedsStyle`] or
/// [`FfiLayoutUpdateOutcome::NeedsRoundFacts`] for the arena and the document did what that asks.
unsafe fn resume_update_layout(
    main_thread: &crate::stage::MainThread,
    arena_handle: *mut c_void,
    round: LayoutRoundFacts,
) -> FfiLayoutUpdateOutcome {
    let parked = main_thread
        .host_tables()
        .and_then(|host_tables| host_tables.parked_layout_update.take());
    let Some(ParkedLayoutUpdate {
        frame,
        submits_pass,
        viewport_propagation_sources,
        asked,
    }) = parked
    else {
        debug_assert!(false, "a layout update is resumed after it returned for style");
        // With no frame to go on with, the update has nothing left to do.
        return FfiLayoutUpdateOutcome::Finished;
    };
    if let Some((document, query)) = asked {
        // SAFETY: Guaranteed by the caller.
        return unsafe { go_on_with_owner(main_thread, *frame, document, query, Some(round)) };
    }
    // SAFETY: Guaranteed by the caller.
    let driven = unsafe { LayoutPassJob::drive_frame(main_thread, *frame, submits_pass, Some(round)) };
    // SAFETY: As above.
    unsafe {
        go_on_from_driven_frame(
            main_thread,
            arena_handle,
            driven,
            submits_pass,
            viewport_propagation_sources,
        )
    }
}

/// Goes on with a layout update the document thread drives with the render owner for `query`, from
/// the start of a round or, `resumed_with` the facts of the round it started, once the document has
/// read them: parks a frame that stops for the document, and hands the owner's answer to the
/// document thread once the frame is over.
///
/// # Safety
///
/// As for [`update_layout`].
unsafe fn go_on_with_owner(
    main_thread: &crate::stage::MainThread,
    mut frame: LayoutFrame,
    document: crate::render_owner::DocumentId,
    query: crate::render_owner::Query,
    resumed_with: Option<LayoutRoundFacts>,
) -> FfiLayoutUpdateOutcome {
    // SAFETY: Guaranteed by the caller.
    let runs_style = match unsafe { frame.drive_on_owner(main_thread, document, query, resumed_with) } {
        Ok(answer) => {
            crate::render_owner::answered(answer);
            return FfiLayoutUpdateOutcome::Finished;
        }
        Err(runs_style) => runs_style,
    };
    let Some(host_tables) = main_thread.host_tables() else {
        debug_assert!(false, "a document whose layout update runs has host tables");
        // With nowhere to park the frame, it goes on without the document's style for the round,
        // which the next rendering update runs, and with the facts as they are.
        let round = LayoutRoundFacts {
            facts: frame.inputs.host.document_facts(main_thread),
            selection: None,
        };
        // SAFETY: Guaranteed by the caller.
        return unsafe { go_on_with_owner(main_thread, frame, document, query, Some(round)) };
    };
    let parked = ParkedLayoutUpdate {
        frame: Box::new(frame),
        submits_pass: false,
        viewport_propagation_sources: Vec::new(),
        asked: Some((document, query)),
    };
    let previous = host_tables.parked_layout_update.replace(Some(parked));
    debug_assert!(previous.is_none(), "a document runs one layout update at a time");
    if runs_style {
        FfiLayoutUpdateOutcome::NeedsStyle
    } else {
        FfiLayoutUpdateOutcome::NeedsRoundFacts
    }
}

/// Goes on with a layout update from where the document thread's drive of its frame stopped: parks
/// a frame that stopped for the document's style, and readies or submits the full layout pass a
/// frame readied.
///
/// # Safety
///
/// As for [`update_layout`].
unsafe fn go_on_from_driven_frame(
    main_thread: &crate::stage::MainThread,
    arena_handle: *mut c_void,
    driven: DrivenFrame,
    submits_pass: bool,
    viewport_propagation_sources: Vec<StyleNodeID>,
) -> FfiLayoutUpdateOutcome {
    let mut pass = match driven {
        DrivenFrame::Ended => return FfiLayoutUpdateOutcome::Finished,
        DrivenFrame::ForDocument { frame, runs_style } => {
            let Some(host_tables) = main_thread.host_tables() else {
                debug_assert!(false, "a document whose layout update runs has host tables");
                // With nowhere to park the frame, it goes on without the document's style for the
                // round, which the next rendering update runs, and with the facts as they are.
                let round = LayoutRoundFacts {
                    facts: frame.inputs.host.document_facts(main_thread),
                    selection: None,
                };
                // SAFETY: Guaranteed by the caller.
                let driven = unsafe { LayoutPassJob::drive_frame(main_thread, *frame, submits_pass, Some(round)) };
                // SAFETY: As above.
                return unsafe {
                    go_on_from_driven_frame(
                        main_thread,
                        arena_handle,
                        driven,
                        submits_pass,
                        viewport_propagation_sources,
                    )
                };
            };
            let parked = ParkedLayoutUpdate {
                frame,
                submits_pass,
                viewport_propagation_sources,
                asked: None,
            };
            let previous = host_tables.parked_layout_update.replace(Some(parked));
            debug_assert!(previous.is_none(), "a document runs one layout update at a time");
            return if runs_style {
                FfiLayoutUpdateOutcome::NeedsStyle
            } else {
                FfiLayoutUpdateOutcome::NeedsRoundFacts
            };
        }
        DrivenFrame::Pass(pass) => *pass,
    };
    if crate::stage_thread::submits_flight() {
        // The flight goes on to record the document once it has laid it out, if the document seals what
        // that reads before it submits the flight, with its style and the rest of the round's host steps
        // done.
        crate::painting::ffi::layout_arena_discard_sealed_flight_paint();
        let style = pass.take_style_pass();
        let outcome = match style {
            Some(_) => FfiLayoutUpdateOutcome::FlightWithStyleReady,
            None => FfiLayoutUpdateOutcome::FlightReady,
        };
        let flight = PreparedFlight {
            pass,
            style,
            viewport_propagation_sources,
        };
        let Some(host_tables) = main_thread.host_tables() else {
            debug_assert!(false, "layout node arena has no host tables");
            // With nowhere to keep the flight for the document to seal, it goes unsealed: it lays the
            // document out and records nothing.
            // SAFETY: Guaranteed by the caller.
            unsafe { flight.submit(arena_handle) };
            return FfiLayoutUpdateOutcome::PassSubmitted;
        };
        let prepared = host_tables.prepared_flight.replace(Some(flight));
        debug_assert!(
            prepared.is_none(),
            "a document submits the flight it prepared before another"
        );
        return outcome;
    }
    let take_back = pass.take_back();
    // The pass reads the style engine through the arena, and takes the engine's token along.
    // SAFETY: Guaranteed by the caller.
    let style_engine = unsafe { arena(arena_handle) }.style_engine_handle();
    let (loan, settlement) = (!style_engine.is_null())
        .then(|| {
            style_engine.lend(
                crate::css::style::engine_home::Holder::LayoutPass,
                crate::css::style::engine_home::Owed::TakeBack,
            )
        })
        .unzip();
    // SAFETY: The frame reaches only the arena and its style engine, which the frame in flight owns
    // until the document thread takes it back: every document-thread path to the arena and the tree
    // update marks it holds joins the frame first, and the engine's token goes with the pass.
    unsafe {
        crate::stage_thread::submit_stage_with_take_back(
            "layout",
            arena_handle,
            move || {
                let mut loan = loan;
                let _ = match loan.as_mut() {
                    Some(loan) => loan.lend_to_this_thread(|_| pass.run()),
                    None => pass.run(),
                };
            },
            move || {
                if let Some(settlement) = settlement {
                    settlement.settle();
                }
                take_back.finish();
            },
        );
    }
    FfiLayoutUpdateOutcome::PassSubmitted
}

/// Whether a layout update that may submit its full layout pass runs its first round's style in the
/// flight it submits, if the document asks for that.
fn runs_style_in_flight(may_submit_pass: bool) -> bool {
    may_submit_pass && crate::stage_thread::submits("layout") && crate::stage_thread::submits_flight()
}

/// Submits the flight the document's layout update readied, once the document has sealed what its
/// recording reads.
///
/// # Safety
///
/// As for [`update_layout`], right after it answered with a flight ready.
unsafe fn submit_prepared_flight(main_thread: &crate::stage::MainThread, arena_handle: *mut c_void) {
    let flight = main_thread
        .host_tables()
        .and_then(|host_tables| host_tables.prepared_flight.take());
    debug_assert!(flight.is_some(), "the document's layout update readied a flight");
    if let Some(flight) = flight {
        // SAFETY: Guaranteed by the caller.
        unsafe { flight.submit(arena_handle) };
    }
}

impl PreparedFlight {
    /// Submits the flight.
    ///
    /// # Safety
    ///
    /// As for [`update_layout`], which readied the flight.
    unsafe fn submit(self, arena_handle: *mut c_void) {
        let Self {
            pass,
            style,
            viewport_propagation_sources,
        } = self;
        let flight = match style {
            Some(style) => crate::flight::Flight::from_style_and_layout_pass(
                arena_handle,
                style,
                pass,
                viewport_propagation_sources,
            ),
            None => crate::flight::Flight::from_layout_pass(arena_handle, pass),
        };
        // SAFETY: The frame in flight owns what the flight's stages reach until the document thread
        // takes it back: every document-thread path to the arena joins the frame first.
        unsafe { crate::flight::submit(arena_handle, flight) };
    }
}

/// A layout frame the document thread has driven up to its full layout pass, which it hands to a
/// stage to run the rest of its round. The stage leaves the frame where [`LayoutPassTakeBack`]
/// finds it once the document thread has taken the frame back.
pub(crate) struct LayoutPassJob {
    arena_handle: usize,
    frame: crate::stage_thread::FrameOwns<LayoutFrame>,
    /// The style pass the frame's first round runs in the flight, ahead of the rest of the round.
    style: Option<crate::css::style::bridge::StylePassJob>,
    facts: FfiLayoutUpdateDocumentFacts,
    ran: std::sync::Arc<std::sync::Mutex<Option<crate::stage_thread::FrameOwns<LayoutFrame>>>>,
}

/// Where the document thread's drive of a layout frame left it.
enum DrivenFrame {
    /// The frame ended on the document thread, and the update with it.
    Ended,
    /// The frame stopped for the document to read the facts of the round it started, running the
    /// round's style first if `runs_style`.
    ForDocument { frame: Box<LayoutFrame>, runs_style: bool },
    /// The frame readied the full layout pass it runs in flight.
    Pass(Box<LayoutPassJob>),
}

/// How far a frame made from the arena after a layout round in flight may go before the round is
/// taken back.
pub(crate) struct RoundInFlight {
    /// The round left the document laid out as far as the arena knows, with nothing for the
    /// document thread to do but pay the host halves: the document may be recorded from the arena,
    /// and the recording stands unless paying them leaves more work.
    pub(crate) may_be_painted: bool,
    /// Paying the host halves leaves no work either, as far as they tell: the recording may be
    /// presented before the round is taken back.
    pub(crate) may_be_presented: bool,
}

/// What the document thread runs once it has taken back the frame of a [`LayoutPassJob`].
pub(crate) struct LayoutPassTakeBack {
    arena_handle: *mut c_void,
    ran: std::sync::Arc<std::sync::Mutex<Option<crate::stage_thread::FrameOwns<LayoutFrame>>>>,
}

impl LayoutPassJob {
    /// Drives the layout frame of `inputs` on the document thread up to its full layout pass, if
    /// `submits_pass`, and returns the pass, unless the frame ended on the document thread instead,
    /// or stopped for the document's style.
    ///
    /// # Safety
    ///
    /// As for [`update_layout`]. The pass returned has to run in a frame in flight that owns the arena.
    unsafe fn prepare(
        main_thread: &crate::stage::MainThread,
        inputs: FrameInputs,
        submits_pass: bool,
        first_round_style: RoundStyle,
    ) -> DrivenFrame {
        let frame = LayoutFrame::new(inputs, first_round_style);
        // SAFETY: Guaranteed by the caller.
        unsafe { Self::drive_frame(main_thread, frame, submits_pass, None) }
    }

    /// Drives `frame` on the document thread, as [`Self::prepare`] does, from the start of a round
    /// or, `resumed_with` the facts of the round it started, once the document has read them.
    ///
    /// # Safety
    ///
    /// As for [`Self::prepare`].
    unsafe fn drive_frame(
        main_thread: &crate::stage::MainThread,
        mut frame: LayoutFrame,
        submits_pass: bool,
        resumed_with: Option<LayoutRoundFacts>,
    ) -> DrivenFrame {
        // SAFETY: Guaranteed by the caller.
        let facts = match unsafe { frame.drive(main_thread, submits_pass, resumed_with) } {
            DriveStop::Ended => return DrivenFrame::Ended,
            DriveStop::ForDocument { runs_style } => {
                return DrivenFrame::ForDocument {
                    frame: Box::new(frame),
                    runs_style,
                };
            }
            DriveStop::AtRound(facts) => facts,
        };
        // The style round paid what the frame owed. What the round in flight comes to owe (its tree
        // build's host half, its commits' host halves) is paid as the frame is taken back: tasks that
        // run beside the flight and would read it reach it through the arena's doors, which join.
        debug_assert!(
            frame.owed_host_halves.get_mut().is_empty() && frame.host_payments.is_empty(),
            "the style round pays what the frame owed"
        );
        // The style the flight runs is installed on the document thread first of all the frame
        // owes it.
        let style = frame.style_pass.take();
        if style.is_some() {
            frame.host_payments.push(HostHalfPayment::StyleInstall);
            frame.style_ran_in_flight = true;
        }
        DrivenFrame::Pass(Box::new(Self {
            arena_handle: frame.inputs.arena_handle as usize,
            // SAFETY: Guaranteed by the caller.
            frame: unsafe { crate::stage_thread::FrameOwns::new(frame) },
            style,
            facts,
            ran: std::sync::Arc::new(std::sync::Mutex::new(None)),
        }))
    }

    /// What the document thread runs once it has taken the frame back.
    pub(crate) fn take_back(&self) -> LayoutPassTakeBack {
        LayoutPassTakeBack {
            arena_handle: self.arena_handle as *mut c_void,
            ran: self.ran.clone(),
        }
    }

    /// Takes the style pass the frame's first round runs in the flight, if it has one.
    pub(crate) fn take_style_pass(&mut self) -> Option<crate::css::style::bridge::StylePassJob> {
        self.style.take()
    }

    /// Whether the rest of the round lays out the layout tree the arena has, which a flight that ran
    /// the round's style may then style itself: the round has no tree build to ready, which only the
    /// document thread readies once the style is installed.
    pub(crate) fn lays_out_the_tree_it_has(&self) -> bool {
        let frame = self.frame.get();
        frame.tree_build_document_style_node.is_none() && !frame.needs_layout_tree_rebuild(&self.facts)
    }

    /// Readies the rest of the round once the flight has applied the style of its first round to
    /// the arena: reads the sources of the pass, as the style round reads them for a round whose
    /// style ran on the document thread.
    ///
    /// # Safety
    ///
    /// The frame in flight owns the arena.
    pub(crate) unsafe fn ready_after_style_in_flight(&mut self) {
        let frame = self.frame.get_mut();
        if frame.pass_sources.is_none() && frame.tree_build_document_style_node.is_none() {
            // SAFETY: Guaranteed by the caller.
            frame.pass_sources = Some(unsafe { LayoutPassSources::read(frame.inputs.arena_handle) });
        }
    }

    /// Leaves the frame unrun for the document thread, which ends it as it takes it back: the flight
    /// left the style of its first round to the document thread, which lays out after it.
    pub(crate) fn park(self) {
        let Self { frame, ran, .. } = self;
        *ran.lock().expect("a frame that ran left itself") = Some(frame);
    }

    /// Runs the rest of the frame's round, on the stage that owns the arena, and answers how far a
    /// frame made from the arena before the frame is taken back may go.
    pub(crate) fn run(self) -> RoundInFlight {
        let Self { frame, facts, ran, .. } = self;
        let mut frame = frame.into_inner();
        // Where the frame would go on from here is the next layout update's to find: the take-back
        // ends it wherever the document thread is.
        // SAFETY: The frame in flight owns the arena, as LayoutPassJob::prepare requires.
        let step = unsafe { frame.run_round_through_pass(facts) };
        let may_be_painted =
            matches!(step, FrameStep::Ended(FrameEnd::UnlessHostLeftWork)) && frame.may_be_painted_before_take_back();
        let may_be_presented = may_be_painted && frame.owes_the_host_no_work();
        // SAFETY: As above.
        unsafe { frame.resolve_owed_host_halves() };
        // SAFETY: As above.
        *ran.lock().expect("a frame that ran left itself") =
            Some(unsafe { crate::stage_thread::FrameOwns::new(frame) });
        RoundInFlight {
            may_be_painted,
            may_be_presented,
        }
    }
}

impl LayoutPassTakeBack {
    /// Ends the frame the stage ran, on the document thread, which has taken the frame back.
    pub(crate) fn finish(self) {
        let frame = self.take_frame();
        main_thread_entries::finish_layout_frame_taken_back(self.arena_handle, frame);
    }

    /// Ends the frame the stage ran, after which the flight recorded the document, on the document
    /// thread, which has taken the frame back. Answers whether the recording stands: unless paying
    /// the host halves left style or layout work, which the recording does not show.
    pub(crate) fn finish_after_recording(self) -> bool {
        let frame = self.take_frame();
        main_thread_entries::finish_layout_frame_recorded_in_flight(self.arena_handle, frame)
    }

    fn take_frame(&self) -> LayoutFrame {
        self.ran
            .lock()
            .expect("a frame that ran left itself")
            .take()
            .expect("the frame is taken back once its round has run")
            .into_inner()
    }
}

/// Ends a layout frame whose round the document thread has taken back, and then the update: takes
/// back the tree update marks, pays what the round owes, and applies the frame's messages.
/// Whatever the round leaves pending (another build, another round), the next layout update does:
/// the frame is taken back wherever the document thread reaches what it owns, which can be in the
/// middle of a DOM mutation. It starts no style update there; the rest of a style update the flight
/// ran (its install) is paid all the same, as the commit of a style flight pays it at a forced join.
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
            std::mem::take(&mut frame.host_payments),
        );
    }
    // The list owners the round's build found showing stale counters were marked for the build of
    // the next layout update as its host half was paid, as the next style round would have marked
    // them.
    // The install owed the flight the repaint of a batch the flight applied, or parked and did not:
    // either way nothing recorded after it here.
    if frame.style_ran_in_flight {
        // SAFETY: As above.
        unsafe { arena(frame.inputs.arena_handle) }.take_flight_style_applied();
        frame.messages.flight_style_repaint_recorded = Some(false);
    }
    // SAFETY: As above.
    unsafe { frame.take_in_end(main_thread, FfiLayoutUpdateEnd::FrameTakenBack) };
}

/// Ends a layout frame whose round a flight has run and recorded the document after, as
/// [`finish_layout_frame`] ends one. Answers whether the recording stands: the frame is over only if
/// paying what the round owes leaves no style or layout work, as a frame that ends on the document
/// thread checks. Otherwise the next layout update runs that work, and the document records again.
///
/// # Safety
///
/// As for [`finish_layout_frame`].
unsafe fn finish_layout_frame_recorded_in_flight(
    main_thread: &crate::stage::MainThread,
    mut frame: LayoutFrame,
) -> bool {
    let host = frame.inputs.host;
    let arena_handle = frame.inputs.arena_handle;
    // A style row the flight applied that the host declined was put back: the recording shows
    // what the host did not install, and does not stand.
    // SAFETY: The frame is the document thread's again, and it runs for the update the arena is in.
    let restored_style = unsafe {
        pay_owed_host_halves(
            main_thread,
            &host,
            arena_handle,
            std::mem::take(&mut frame.host_payments),
        )
    };
    // SAFETY: As above.
    let arena = unsafe { arena(arena_handle) };
    let facts = host.document_facts(main_thread);
    let recording_stands = !restored_style
        && !style_update_follows_layout(arena, &facts)
        && !facts.top_layer_work_pending
        && layout_is_up_to_date(arena, &facts);
    // The install owed the flight the repaint of its batch whether the flight applied it or parked:
    // the recording is that repaint only if the flight applied the batch before it recorded.
    let mut messages = std::mem::take(&mut frame.messages);
    if frame.style_ran_in_flight {
        let applied = arena.take_flight_style_applied();
        messages.flight_style_repaint_recorded = Some(applied && recording_stands);
    }
    messages.recorded_in_flight = recording_stands;
    messages.take_in(main_thread, &host, FfiLayoutUpdateEnd::FrameTakenBack);
    recording_stands
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

/// Brings the document's style engine token home, as an entrance of the engine does (see
/// `crate::css::style::engine_home`), for a main-side write to the engine. `file` and `line` name
/// the C++ call site for the forced-join log.
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
    // SAFETY: Guaranteed by the caller. No stage writes the arena's link to its style engine.
    unsafe { &*arena.cast::<crate::layout::LayoutNodeArena>() }
        .style_engine_handle()
        .bring_home_at(file, line, 0);
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

/// Hands `row` the image resources a finished layout frame owed it (see
/// [`FfiLayoutFrameEffects::owed_image_resources`]), if a later build in the frame did not free it:
/// the image box stops waiting for the provider it owns, which the document attaches next. Answers
/// whether the row is live.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread, which is taking in the frame's effects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hand_over_owed_image_resources(arena: *mut c_void, row: NodeSlotId) -> bool {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: As above.
    let arena = unsafe { LayoutNodeArena::from_handle(arena) };
    if !arena.slot_is_live(row) {
        return false;
    }
    arena.note_owned_provider_handed_over(row);
    true
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
            style_input_waits_on_document: false,
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
