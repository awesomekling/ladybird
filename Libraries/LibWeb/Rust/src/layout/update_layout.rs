/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The layout update: the style and layout stabilization loop a document runs before it can
//! answer geometry queries or paint. The steps that touch the DOM stay on the C++ host and
//! answer through a callback table registered once per arena. The document thread drives the
//! loop and runs those steps itself; the rest of each round runs on the render owner.

use super::formatting_context::{
    CommitPayment, DeferredLayoutCommitHostHalf, PendingLayoutCommit, commit_root_layout_to_arena,
    commit_subtree_layout_to_arena, compute_root_layout, compute_subtree_layout_fragments,
    prepare_root_layout_from_sources,
};
use super::layout_node_arena::{
    EnrolledContentSources, HostPayment, OwedImageResources, apply_enrolled_content_sources,
    read_enrolled_content_sources,
};
use super::node_data::NodeSlotId;
use super::node_facts;
use super::partial_relayout::FfiPartialRelayoutHostFacts;
use super::tree_builder::{
    FfiGeneratedContentItem, FfiLayoutTreeBuildOutcome, FfiPseudoElement, TreeBuildHostHalf, TreeBuildPayment,
    walk_layout_tree_build,
};
use super::{ArenaHandle, LayoutNodeArena};
use crate::abort_on_panic;
use crate::css::style::tree::StyleNodeID;
use crate::layout::used_values::FfiCssPixelPoint;
use crate::painting::paintable_data::FfiSelectionSnapshot;
use crate::painting::selection::SelectionSnapshot;
use std::cell::Cell;
use std::ffi::c_void;

mod main_thread_entries;

pub(crate) use main_thread_entries::MainThreadFfiEntry;

/// The document-side steps of a layout update, which the document thread runs between the jobs of the frame it hands
/// the render owner. Each callback receives the registered `context`, the owning document, first and answers
/// synchronously; any of them may run the layout update of another document, so the frame holds no arena borrow
/// across a call.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiLayoutUpdateHostCallbacks {
    pub context: *mut c_void,
    pub document_facts: unsafe extern "C" fn(*mut c_void) -> FfiLayoutUpdateDocumentFacts,
    /// Takes in what the frame left for the document, and ends the layout update on the document
    /// side. The document thread calls it as it takes in the frame's end, so the frame is over for
    /// the document once the update returns. The effects are valid for the call.
    pub take_in_frame_effects: unsafe extern "C" fn(*mut c_void, *const FfiLayoutFrameEffects),
    /// Starts a round after the first: runs the document's style update first where the flag says, then reads the
    /// round (see [`FfiLayoutRoundFacts`]), with the document's style where its tree build may build the viewport, and
    /// hands it to `layout_frame_take_round` with the last argument while it is valid.
    pub start_round: unsafe extern "C" fn(*mut c_void, bool, *mut c_void),
}

/// The document's style for a tree build that may build the viewport, as the document thread made it: its group
/// payloads and its longhand table, which the owner interns as the job the round starts begins, and the scroll offset of
/// the document's navigable. The document keeps the style alive until it makes the next, and makes none while the job
/// runs.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiDocumentStyleForBuild {
    pub payloads: super::node_data::FfiStylePayloads,
    pub longhand_table: *const c_void,
    pub viewport_scroll_offset: super::FfiCssPixelPoint,
}

/// The document's style for a tree build (see [`FfiDocumentStyleForBuild`]), as the owner reads it.
pub(crate) struct DocumentStyleForBuild {
    pub(crate) payloads: [crate::css::host_shared::SharedPayload; super::node_data::STYLE_GROUP_COUNT],
    pub(crate) longhand_table:
        crate::css::host_shared::HostShared<crate::css::computed_longhand_table::ComputedLonghandTable>,
    pub(crate) viewport_scroll_offset: super::FfiCssPixelPoint,
}

impl DocumentStyleForBuild {
    fn from_ffi(style: &FfiDocumentStyleForBuild) -> Self {
        Self {
            payloads: style.payloads.groups.map(crate::css::host_shared::SharedPayload::new),
            longhand_table: crate::css::host_shared::HostShared::new(style.longhand_table.cast()),
            viewport_scroll_offset: style.viewport_scroll_offset,
        }
    }
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
#[derive(Clone, Copy, Default)]
#[repr(C)]
pub struct FfiLayoutUpdateDocumentFacts {
    /// The document is its navigable's active document; an inactive document counts as laid out.
    pub document_is_active: bool,
    /// The document node or one of its descendants needs a layout tree update.
    pub document_needs_layout_tree_build: bool,
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
    /// The document's style node, which a layout tree build walks from. The document readies the build of a round
    /// as it reads the round's facts: it hands a build that may build the viewport the document's style.
    pub document_style_node: u32,
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
    /// visual contexts and recorded and presented the frame: the
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
    /// The document's selection range, or null when it has none or the document finds the round's layout up to date.
    pub selection: *const FfiSelectionSnapshot,
    /// Whether the document made its style for the round, which `document_style` is then: it does where the round's
    /// tree build may build the viewport.
    pub has_document_style: bool,
    pub document_style: FfiDocumentStyleForBuild,
}

/// A layout round's facts as the document read them (see [`FfiLayoutRoundFacts`]).
struct LayoutRoundFacts {
    facts: FfiLayoutUpdateDocumentFacts,
    /// The selection the commits of the round stamp the selection states of the boxes they build
    /// from, as nothing on the document thread changes the tree until the frame is over.
    selection: Option<SelectionSnapshot>,
    /// The document's style, for a tree build of the round that may build the viewport.
    document_style: Option<DocumentStyleForBuild>,
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
            document_style: round
                .has_document_style
                .then(|| DocumentStyleForBuild::from_ffi(&round.document_style)),
        }
    }
}

/// Takes the round the document read for a `start_round` host call (see [`FfiLayoutUpdateHostCallbacks`]).
///
/// # Safety
///
/// `sink` must be the one the host call was handed, and `round` must be valid, and so must the selection it points to,
/// if any.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_frame_take_round(sink: *mut c_void, round: *const FfiLayoutRoundFacts) {
    assert!(!sink.is_null() && !round.is_null());
    // SAFETY: Guaranteed by the caller.
    unsafe { *sink.cast::<Option<LayoutRoundFacts>>() = Some(LayoutRoundFacts::from_ffi(&*round)) };
}

/// What one layout update was asked for.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct FfiLayoutUpdateInputs {
    pub reason_is_inspect_devtools_layout_data: bool,
    /// A document hosting template contents never needs layout.
    pub is_template_contents_document: bool,
    /// Whether the update may submit its first full layout pass to run beside the document thread,
    /// rather than waiting for it.
    pub may_submit_pass: bool,
    /// The first round's facts, which the document read once it had run the round's style ahead of
    /// the update.
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
}

/// Confinement report of the most recent layout tree build, for tests observing whether a
/// partial rebuild stayed inside its rebuilt subtrees.
#[derive(Clone, Copy, Debug, Default)]
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
    take_in_frame_effects: unsafe extern "C" fn(*mut c_void, *const FfiLayoutFrameEffects),
    start_round: unsafe extern "C" fn(*mut c_void, bool, *mut c_void),
}

impl From<FfiLayoutUpdateHostCallbacks> for LayoutUpdateHost {
    fn from(host: FfiLayoutUpdateHostCallbacks) -> Self {
        Self {
            context: host.context,
            document_facts: host.document_facts,
            take_in_frame_effects: host.take_in_frame_effects,
            start_round: host.start_round,
        }
    }
}

impl LayoutUpdateHost {
    // SAFETY (for every call below): The C++ host answers synchronously from its live document.
    fn document_facts(&self, _: &crate::stage::MainThread) -> FfiLayoutUpdateDocumentFacts {
        unsafe { (self.document_facts)(self.context) }
    }

    fn take_in_frame_effects(&self, _: &crate::stage::MainThread, effects: &FfiLayoutFrameEffects) {
        unsafe { (self.take_in_frame_effects)(self.context, effects) }
    }

    /// Starts a round after the first on the document side, running its style first where `runs_style`, and answers
    /// what the document read for it.
    fn start_round(&self, main_thread: &crate::stage::MainThread, runs_style: bool) -> LayoutRoundFacts {
        let mut round: Option<LayoutRoundFacts> = None;
        unsafe {
            (self.start_round)(
                self.context,
                runs_style,
                std::ptr::from_mut(&mut round).cast::<c_void>(),
            );
        }
        round.unwrap_or_else(|| {
            debug_assert!(false, "the document reads the round it starts");
            // The round goes on with the facts as they are, and without a selection.
            LayoutRoundFacts {
                facts: self.document_facts(main_thread),
                selection: None,
                document_style: None,
            }
        })
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

enum PartialRelayout {
    NotEligible,
    Done,
    NeedsAnotherLayoutPass,
}

const ORDINARY_STABILIZATION_ROUND_LIMIT: u64 = 8;

/// What a layout frame is started with. The document drains its invalidation journal into the
/// arena before the frame starts; the document facts the frame reads are snapshots the document
/// thread hands it at each step it runs.
struct FrameInputs {
    host: LayoutUpdateHost,
    arena_handle: *mut c_void,
    reason_is_inspect_devtools_layout_data: bool,
    is_template_contents_document: bool,
}

/// What a layout pass takes ahead of it: what the facts of the replaced content enrolled for sync
/// are derived from, read in the frame once the round's tree build, if any, has run.
struct LayoutPassSources {
    content: EnrolledContentSources,
}

impl LayoutPassSources {
    /// # Safety
    ///
    /// `state` must be the live render state of the frame's document, which nothing else reaches meanwhile.
    unsafe fn read(state: *mut ArenaHandle) -> Self {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            Self {
                content: read_enrolled_content_sources(state),
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
    /// What ending the host half of an owner-applied style update owed the host.
    StyleInstallLeft(HostPayment),
    TreeBuild(TreeBuildPayment),
    Commit(CommitPayment),
}

/// Where a round ends once its full layout pass, if it has one, has run.
#[derive(Clone, Copy)]
enum RoundEnd {
    /// The round found its last commit stable from what the frame owns. The host halves of its
    /// commits can still leave style or layout work pending: the container queries the commits made
    /// pending (the document's query container elements), the other style work, the top layer
    /// changes and the layout tree update marks they made. If they do, another round follows.
    UnlessHostLeftWork,
    /// The round left work for another round.
    AnotherRound,
    /// The frame is over, and the update ends where the value says.
    Over(FfiLayoutUpdateEnd),
}

/// The style and layout stabilization loop of one layout update. It holds no borrow of the job it
/// is in, so a round can stop ahead of its full layout pass (for a flight) and go on once the pass
/// has run.
struct LayoutFrame {
    inputs: FrameInputs,
    /// The render state of the frame's document, which the owner hands a job of the frame, or a stage of it, for as
    /// long as it runs; null between them.
    state: *mut ArenaHandle,
    messages: FrameMessages,
    /// The rounds the loop has started, and the connected element count the last style round
    /// answered, which bound them.
    layout_pass: u64,
    connected_element_count: u32,
    /// The sources the last style round read for the layout pass that follows it.
    pass_sources: Option<LayoutPassSources>,
    /// The document style node of the tree build the style round readied.
    tree_build_document_style_node: Option<u32>,
    /// The document's selection as the style round of the round read it, if it has one.
    selection: Option<SelectionSnapshot>,
    /// What the frame's tree builds and commits owe the document thread, in the order the frame
    /// made them, until the job or stage that made them resolves them into `host_payments`.
    owed_host_halves: Cell<Vec<OwedHostHalf>>,
    /// What the frame owes the document thread, which the next step on the document thread pays before its work.
    host_payments: Vec<HostHalfPayment>,
    /// The facts the document read for the first round once it had run the round's style ahead of the update, until
    /// the round starts. The style of every later round runs on the document thread as it starts the round.
    first_round: Option<LayoutRoundFacts>,
    /// What the arena and the engine showed as the round a flight ran ended, for the document thread to tell whether
    /// the flight's recording stands as it takes the frame back.
    owner_end: Option<OwnerEndFacts>,
}

/// A full layout pass a round has readied. Everything it reads is in hand, so it runs without the
/// document thread, and what follows its commit is left to [`LayoutFrame::finish_round`].
struct PendingLayoutPass {
    state: *mut ArenaHandle,
    layout_root: NodeSlotId,
    sources: LayoutPassSources,
    facts: FfiLayoutUpdateDocumentFacts,
}

impl PendingLayoutPass {
    /// # Safety
    ///
    /// The frame must run for the update the arena is in, and nothing but the pass may reach the
    /// arena until it returns.
    unsafe fn run(self) -> LaidOutPass {
        let Self {
            state,
            layout_root,
            sources: LayoutPassSources { content },
            facts,
        } = self;
        // SAFETY (for the three steps below): Guaranteed by the caller; the viewport box stays live
        // between them, and no row was freed since the sources were read.
        unsafe { prepare_root_layout_from_sources(state, layout_root, content) };
        let output = unsafe {
            compute_root_layout(
                state,
                layout_root,
                facts.viewport_inline_size_raw,
                facts.viewport_block_size_raw,
                facts.document_in_quirks_mode,
                facts.should_collect_devtools_layout_data,
            )
        };
        let pending_commit = unsafe { commit_root_layout_to_arena(state, layout_root, &output) };
        drop(output);
        // SAFETY: Guaranteed by the caller.
        let arena = unsafe { &*state }.arena();
        arena.evaluate_size_containers_needing_evaluation_after_layout();
        // SAFETY: Guaranteed by the caller, and the frame delivers the host half at its next join,
        // in commit order.
        let commit_host_half = unsafe { pending_commit.settle_ahead_of_host() };
        arena.end_layout_pass_preparation_handbacks();
        arena.note_full_layout();
        LaidOutPass {
            commit_host_half,
            facts,
        }
    }
}

/// A full layout pass that has run and settled its commit in the arena, whose commit's host half
/// the frame still owes the document thread.
struct LaidOutPass {
    commit_host_half: DeferredLayoutCommitHostHalf,
    facts: FfiLayoutUpdateDocumentFacts,
}

/// What the owner's arena and style engine show as a frame job ends, which the document thread reads with the facts
/// the document reads once it has paid what the job left the frame owing it. Paying hands nothing back into the arena
/// or the engine, so they still hold then.
#[derive(Clone, Copy, Debug)]
struct OwnerEndFacts {
    /// The arena's layout is up to date, the document's own tree build marks aside (see [`layout_is_up_to_date`]).
    arena_laid_out: bool,
    /// The engine asks for style after layout: for a container a style computation asked about before it had a box,
    /// or for a transaction it has pending.
    engine_style_follows: bool,
    /// The engine holds deferred element style inputs, which the style of a round takes.
    deferred_element_style_inputs: bool,
}

impl OwnerEndFacts {
    fn read(arena: &LayoutNodeArena) -> Self {
        let (engine_style_follows, deferred_element_style_inputs) = arena.with_style_store(|engine| {
            (
                engine.has_size_containers_needing_evaluation_after_layout() || engine.has_pending_transaction(),
                engine.has_deferred_element_style_inputs(),
            )
        });
        Self {
            arena_laid_out: arena.layout_is_up_to_date(false),
            engine_style_follows,
            deferred_element_style_inputs,
        }
    }

    /// Whether the document's style is to be updated again after its layout: for input the document still holds, or
    /// for what the engine asks for.
    fn style_follows(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        facts.style_input_waits_on_document || self.engine_style_follows
    }

    /// Whether the document counts as laid out: an inactive document is left alone.
    fn laid_out(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        !facts.document_is_active || (self.arena_laid_out && !facts.document_needs_layout_tree_build)
    }

    /// Whether the document, with the facts `facts`, has style or layout work left for another round.
    fn left_work(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        self.style_follows(facts) || facts.top_layer_work_pending || !self.laid_out(facts)
    }

    /// Whether the next round has style to run. A round after the first has style to run only if the rounds before it
    /// left some: what their layout noted in the engine, or what paying handed the document. Otherwise the document's
    /// style update would find nothing to do.
    fn next_round_runs_style(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        self.style_follows(facts) || self.deferred_element_style_inputs
    }
}

/// What the document thread hands the owner with a job of a layout frame: everything the rounds of the job read from
/// the document, which it read before it sent the job.
struct FrameInput {
    /// The facts, the selection and the document's style of the round the job starts with, as the document read them
    /// once the round's style ran.
    round: LayoutRoundFacts,
    /// What the document thread asked about the document, which the owner answers from the arena as the job's rounds
    /// leave it.
    query: Option<crate::render_owner::Query>,
    /// Whether the job readies the first round that lays out to go in flight, rather than running it.
    readies_flight: bool,
}

/// A job of a layout frame, which the owner runs while the document thread waits.
enum FrameJob {
    /// The frame's rounds from the round whose style ran, until the frame's end or a round the owner cannot start.
    Rounds(FrameInput),
}

/// Where a job of a layout frame ended. None of these is in the middle of a round: the job runs every round it starts
/// to its end, and hands nothing back to the document thread before.
enum FrameJobEnd {
    /// The frame is over, and the update ends where the value says, once the document thread has paid what it owes.
    Over(FfiLayoutUpdateEnd),
    /// The frame's last commit was stable as far as the owner sees. The frame is over unless paying what it owes the
    /// document thread leaves the document style or layout work, which the facts the document reads after it tell.
    Settled(OwnerEndFacts),
    /// The last round left another round the owner does not start: what the rounds owe the document thread may leave
    /// it work, or the round has style to run (the host steps of the style update are the document thread's) or top
    /// layer changes to take in. The document thread pays, and starts the round.
    NextRound(OwnerEndFacts),
    /// The first round that lays out is readied to go in flight with `facts`, building a tree first where `builds`.
    ReadiedForFlight {
        facts: FfiLayoutUpdateDocumentFacts,
        builds: bool,
    },
}

/// What the owner hands back for a job of the rounds of a frame: where it ended, and the answer to the query the job
/// came with.
struct FrameOutput {
    end: FrameJobEnd,
    answer: Option<crate::render_owner::Answer>,
}

/// What the owner answers a job of a layout frame with.
enum FrameJobAnswer {
    Rounds(FrameOutput),
}

/// A job of a layout frame the document thread waits for, which the render owner runs with the document's render
/// state: on its own ([`crate::render_owner::ToOwner::Layout`]), or right after the style transaction of the frame's
/// first round, in the same message ([`crate::render_owner::ToOwner::Style`]). The frame stays with the document thread
/// meanwhile.
pub(crate) struct OwnerFrameJob {
    frame: crate::stage_thread::CallerWaits<*mut LayoutFrame>,
    job: FrameJob,
    /// How the owner runs it. The owner reaches the layout pipeline only through the jobs it is sent, so what reaches
    /// the owner without reaching the pipeline (the unit tests' stage threads) links without it.
    run: unsafe fn(*mut LayoutFrame, FrameJob) -> FrameJobAnswer,
}

/// Runs `run`, the job `job` of the frame `frame`, with the render state `state` of the frame's document.
///
/// # Safety
///
/// Nothing but the thread running the job reaches the frame, its arena and the style engine the arena links until this
/// returns.
unsafe fn run_job(
    owner: &crate::render_owner::Owner,
    run: unsafe fn(*mut LayoutFrame, FrameJob) -> FrameJobAnswer,
    frame: *mut LayoutFrame,
    job: FrameJob,
    state: *mut ArenaHandle,
) -> FrameJobAnswer {
    let run_in_state = || {
        // SAFETY: Guaranteed by the caller.
        unsafe {
            (*frame).state = state;
            let answer = run(frame, job);
            (*frame).state = std::ptr::null_mut();
            // The rows go out as committed, their overflow measured, so a read of the layout asks nothing more.
            (*state).arena_mut().publish_committed_rows();
            answer
        }
    };
    // The rounds reach the style engine the arena links as the owner: what the main thread sent the engine goes in
    // first, and what the rounds leave in it goes to the main thread once they are done.
    // SAFETY: Guaranteed by the caller.
    match unsafe { &*state }.arena().hold_style_engine() {
        None => run_in_state(),
        // SAFETY: As above.
        Some(engine) => unsafe { engine.reach_on_owner(owner, |_| run_in_state()) },
    }
}

/// What the owner answers the document thread that waits for a job of its frame with.
pub(crate) struct OwnerFrameJobAnswer(crate::stage_thread::CallerWaits<FrameJobAnswer>);

impl OwnerFrameJob {
    /// Runs the job on the owner, for the document thread that waits for it, with the arena of the render state it
    /// holds for the job's document. Where the owner holds none (a bug of the sender's), the job runs with the arena its
    /// frame names.
    pub(crate) fn run(
        self,
        owner: &crate::render_owner::Owner,
        arena: impl FnOnce() -> Option<*mut ArenaHandle>,
    ) -> OwnerFrameJobAnswer {
        let Self { frame, job, run } = self;
        let frame = frame.into_inner();
        // SAFETY: The document thread waits for the job, and reaches neither the frame nor the arena meanwhile.
        let frame_arena = unsafe { (*frame).inputs.arena_handle };
        let state = arena();
        debug_assert_eq!(
            Some(frame_arena),
            state.map(<*mut ArenaHandle>::cast::<c_void>),
            "a job runs with its document's arena"
        );
        // SAFETY: As above.
        let state = state.unwrap_or_else(|| unsafe { ArenaHandle::held_by_waiting_thread(owner, frame_arena) });
        // The faces the rounds want are their document's, for that document's layout end to request.
        let _wanted_face_owner = libgfx_rust::font::WantedFaceOwner::enter(state as u64);
        // SAFETY: As above.
        let answer = unsafe { run_job(owner, run, frame, job, state) };
        // SAFETY: The answer goes back to the document thread, which waits for it.
        OwnerFrameJobAnswer(unsafe { crate::stage_thread::CallerWaits::new(answer) })
    }
}

/// The next job of a layout frame the document thread runs with the owner.
enum NextJob {
    /// The owner ran the job already, and answered it.
    Answered(FrameJobAnswer),
    /// The frame sends the job, with the round the document read for it.
    Send(LayoutRoundFacts),
}

/// Where a layout frame goes on once a step of a round has run.
enum FrameStep {
    /// The round has readied its full layout pass.
    PassReady(PendingLayoutPass),
    /// The round is over, and ends as the value says.
    Ended(RoundEnd),
}

/// Whether a round readied by [`LayoutFrame::ready_round`] lays out.
enum RoundReadiness {
    /// The round lays out, building a tree first where `builds`.
    LaysOut { builds: bool },
    /// The round does not lay out.
    DoesNotLayOut,
}

/// Where the document thread's run of a layout frame with the owner left it.
enum FrameRun {
    /// The frame is over, and the update with it, with the owner's answer to the query the document thread asked.
    Ended(Option<crate::render_owner::Answer>),
    /// The first round that lays out is readied to go in flight with these facts, and the frame has not ended.
    ReadiedForFlight(FfiLayoutUpdateDocumentFacts),
}

impl LayoutFrame {
    fn new(inputs: FrameInputs, first_round: Option<LayoutRoundFacts>) -> Self {
        Self {
            inputs,
            state: std::ptr::null_mut(),
            messages: FrameMessages::default(),
            layout_pass: 0,
            connected_element_count: 0,
            pass_sources: None,
            tree_build_document_style_node: None,
            selection: None,
            owed_host_halves: Cell::default(),
            host_payments: Vec::new(),
            first_round,
            owner_end: None,
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

    /// Takes back the layout tree update marks the frame lent a tree build, then pays what the frame owed the
    /// document thread, in the order the frame made it owe it.
    ///
    /// # Safety
    ///
    /// On the document thread, for the update the arena is in, with no job or stage of the frame outstanding.
    unsafe fn pay_host_halves(&mut self, main_thread: &crate::stage::MainThread) {
        let arena_handle = self.inputs.arena_handle;
        // What the build owes the document can mark nodes for another build.
        // SAFETY: Guaranteed by the caller.
        unsafe { super::tree_update_marks::take_back_from_frame(arena_handle) };
        for payment in std::mem::take(&mut self.host_payments) {
            match payment {
                HostHalfPayment::StyleInstallLeft(payment) => payment.pay(main_thread),
                HostHalfPayment::TreeBuild(payment) => payment.pay(main_thread),
                HostHalfPayment::Commit(payment) => {
                    // SAFETY: Guaranteed by the caller.
                    unsafe { payment.deliver(main_thread) };
                }
            }
        }
        // What the document thread wrote to the marks beside the frame, it wrote after all of that.
        // SAFETY: Guaranteed by the caller. The marks are handed back.
        unsafe { super::tree_update_marks::write_marks_waiting_for_frame(arena_handle) };
    }

    /// Resolves what the frame's tree builds and commits owe the document thread from the arena, as the job that
    /// made them ends, for the document thread to pay without the arena, after what it owed before.
    ///
    /// # Safety
    ///
    /// On the thread that owns the arena, with nothing else reaching it.
    unsafe fn resolve_owed_host_halves(&mut self) {
        // What the changes the owner applied owe the host goes before all the frame owes.
        let leftover = self.arena().take_leftover_payment();
        if !leftover.is_nothing() {
            self.host_payments
                .insert(0, HostHalfPayment::StyleInstallLeft(leftover));
        }
        // A later build of the frame may have freed a row an earlier one owed image resources.
        // SAFETY: Guaranteed by the caller.
        let arena = unsafe { &*self.state() }.arena();
        self.messages
            .owed_image_resources
            .retain(|(row, _)| arena.slot_is_live(*row));
        for owed in self.owed_host_halves.take() {
            let payment = match owed {
                OwedHostHalf::TreeBuild(owed) => HostHalfPayment::TreeBuild(owed.resolve(self.arena())),
                // SAFETY: Guaranteed by the caller.
                OwedHostHalf::Commit(owed) => HostHalfPayment::Commit(unsafe { owed.resolve() }),
            };
            self.host_payments.push(payment);
        }
    }

    /// The render state the job or stage of the frame that runs now was handed.
    fn state(&self) -> *mut ArenaHandle {
        debug_assert!(
            !self.state.is_null(),
            "a job of a layout frame runs with its document's render state"
        );
        if self.state.is_null() {
            // SAFETY: The frame runs for the update the arena is in, and the job that reaches it alone runs.
            return crate::render_owner::do_owner_work_here(|owner| unsafe {
                ArenaHandle::held_by_waiting_thread(owner, self.inputs.arena_handle)
            });
        }
        self.state
    }

    fn arena(&self) -> &LayoutNodeArena {
        // SAFETY: The state is the frame's document's, which only the job or stage that runs reaches.
        unsafe { &*self.state() }.arena()
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
    /// it, on the owner: the tree build when one comes first, from the document's style node, and otherwise the
    /// sources of the layout pass. The document handed the round its style for a build that may build the viewport
    /// as it read the facts; a round that builds no tree lets it go. Answers whether the round lays out, and
    /// whether it builds.
    fn ready_round(&mut self, facts: &FfiLayoutUpdateDocumentFacts) -> RoundReadiness {
        self.pass_sources = None;
        self.tree_build_document_style_node = None;
        self.connected_element_count = self
            .arena()
            .with_style_store(|engine| engine.tree().connected_element_count());
        if !self.round_lays_out_in_frame(facts) {
            self.arena().release_published_document_style();
            return RoundReadiness::DoesNotLayOut;
        }
        if self.needs_layout_tree_rebuild(facts) {
            // The document reads whether the build may build the viewport as the arena does, from its own marks.
            if self.build_needs_document_style(facts) {
                debug_assert!(
                    false,
                    "the document hands its style to a round whose build may build the viewport"
                );
                // Without it, the viewport's row keeps the style it is built with.
            }
            self.tree_build_document_style_node = Some(facts.document_style_node);
            return RoundReadiness::LaysOut { builds: true };
        }
        self.arena().release_published_document_style();
        // SAFETY: The frame runs for the update the arena is in, with the state the owner handed it.
        self.pass_sources = Some(unsafe { LayoutPassSources::read(self.state()) });
        RoundReadiness::LaysOut { builds: false }
    }

    /// Whether the tree build the round readies needs the document's style, which the arena does not hold published
    /// for it: the build may build the viewport, and the document did not hand the round its style. A document hosting
    /// template contents builds no viewport.
    fn build_needs_document_style(&self, facts: &FfiLayoutUpdateDocumentFacts) -> bool {
        if self.inputs.is_template_contents_document || self.arena().holds_published_document_style() {
            return false;
        }
        let document_needs_layout_tree_update = StyleNodeID::from_raw(facts.document_style_node)
            .is_some_and(|document| self.arena().layout_tree_update_needs(document));
        self.arena()
            .tree_build_may_create_viewport(document_needs_layout_tree_update)
    }

    /// Walks the tree build the style round readied, in the frame. Its host half (the host objects of
    /// the rows the walk freed, the box presence it changed, the DOM nodes its commit messages resolve
    /// to, a new viewport's paint state) is left to the next join, and the style resources and
    /// generated image providers of its new rows to the end of the frame.
    fn walk_layout_tree_build(&mut self) -> (WalkedLayoutTreeBuild, TreeBuildHostHalf) {
        let document_style_node = self
            .tree_build_document_style_node
            .take()
            .expect("the style round readies the tree build");
        // SAFETY: The frame runs for the update the arena is in, and the style round published the
        // document's style for the build.
        let (outcome, host_half) = unsafe { walk_layout_tree_build(self.state(), document_style_node) };
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
            selection.apply(unsafe { &mut *self.state() }.arena_mut());
        }
        // SAFETY: As above.
        unsafe { &mut *self.state() }.arena_mut().invalidate_searchable_text();
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
        // SAFETY: Guaranteed by the caller.
        unsafe { self.pay_host_halves(main_thread) };
        std::mem::take(&mut self.messages).take_in(main_thread, &self.inputs.host, end);
    }

    /// Pays what the frame owes the document thread, which may leave the document style or layout work, and reads
    /// the document's facts after it.
    ///
    /// # Safety
    ///
    /// As for [`Self::take_in_end`].
    unsafe fn pay_and_read_facts(&mut self, main_thread: &crate::stage::MainThread) -> FfiLayoutUpdateDocumentFacts {
        // SAFETY: Guaranteed by the caller.
        unsafe { self.pay_host_halves(main_thread) };
        self.inputs.host.document_facts(main_thread)
    }

    /// Runs the frame with the render owner, from the document thread. The document thread hands the owner a job with
    /// everything the job's rounds read from the document (the facts, the selection, the document's style for a build
    /// that may build the viewport, the query it asks), read before it sends it, and waits: the owner runs every round
    /// it starts to its end, with the document's render state, and the job's end is the only point where the frame
    /// comes back to this thread. This thread then pays what the job left the frame owing it (typed effects it
    /// applies without the arena) and reads the document's facts, and the frame is over unless those, with what the
    /// owner's arena and engine showed as the job ended, leave the document style or layout work. Such work, and a
    /// round the owner did not start itself, is another round, which this thread starts (the host steps of its style
    /// update are its own, as are its facts) and hands the owner as the next job, unless the loop has run out of
    /// rounds, which ends the frame. With `readies_flight`, the first round that lays out is readied to go in flight
    /// rather than run, and the frame has not ended.
    ///
    /// # Safety
    ///
    /// On the document thread, for the update the arena is in.
    unsafe fn run_with_owner(
        &mut self,
        main_thread: &crate::stage::MainThread,
        readies_flight: bool,
        query: Option<crate::render_owner::Query>,
        first_job_rode_style: Option<RiddenJob>,
    ) -> FrameRun {
        // SAFETY: Guaranteed by the caller.
        let document = unsafe { super::ArenaHandle::document_of(self.inputs.arena_handle) };
        let mut next = match first_job_rode_style {
            // The first job rode the style transaction of its round, and the owner answered it then.
            Some(ridden) => {
                self.layout_pass += 1;
                NextJob::Answered(self.ridden_first_job(main_thread, document, ridden))
            }
            // SAFETY: As above.
            None => NextJob::Send(unsafe { self.start_first_round(main_thread) }),
        };
        loop {
            let answer = match next {
                NextJob::Answered(answer) => answer,
                NextJob::Send(round) => {
                    if !readies_flight {
                        // The job may build, which reads the marks: they are its until this thread pays what the job
                        // owes it.
                        // SAFETY: As above, with no job of the frame outstanding.
                        unsafe { super::tree_update_marks::lend_to_frame(self.inputs.arena_handle) };
                    }
                    let input = FrameInput {
                        round,
                        query,
                        readies_flight,
                    };
                    self.run_job_on_owner(document, FrameJob::Rounds(input))
                }
            };
            let FrameJobAnswer::Rounds(FrameOutput { end, answer }) = answer;
            // SAFETY (for the steps below): As above, with no job of the frame outstanding.
            let (owner, facts) = match end {
                FrameJobEnd::Over(end) => {
                    unsafe { self.take_in_end(main_thread, end) };
                    return FrameRun::Ended(answer);
                }
                FrameJobEnd::ReadiedForFlight { facts, builds } => {
                    // The flight's build reads the marks until the document thread takes the frame back.
                    if builds {
                        unsafe { super::tree_update_marks::lend_to_frame(self.inputs.arena_handle) };
                    }
                    return FrameRun::ReadiedForFlight(facts);
                }
                FrameJobEnd::Settled(owner) => {
                    let facts = unsafe { self.pay_and_read_facts(main_thread) };
                    if !owner.left_work(&facts) {
                        unsafe { self.take_in_end(main_thread, FfiLayoutUpdateEnd::InUpdate) };
                        return FrameRun::Ended(answer);
                    }
                    (owner, facts)
                }
                FrameJobEnd::NextRound(owner) => (owner, unsafe { self.pay_and_read_facts(main_thread) }),
            };
            if !self.may_start_round() {
                // A loop that runs out of rounds with work pending did not stabilize within its bound.
                if owner.style_follows(&facts) || !owner.laid_out(&facts) {
                    self.messages.stabilization_bound_failed = true;
                }
                unsafe { self.take_in_end(main_thread, FfiLayoutUpdateEnd::InUpdate) };
                return FrameRun::Ended(answer);
            }
            self.layout_pass += 1;
            next = NextJob::Send(
                self.inputs
                    .host
                    .start_round(main_thread, owner.next_round_runs_style(&facts)),
            );
        }
    }

    /// The answer of the frame's first job, which rode the style transaction of its round, as it stands once the
    /// document thread has installed the round's style: as the owner answered it, unless the thread sent the owner
    /// what may move the layout since, or left the document a tree to build where the job's round ended the frame. The
    /// job's rounds are over then, and the frame goes on with another round, as after a round that left work.
    fn ridden_first_job(
        &self,
        main_thread: &crate::stage::MainThread,
        document: crate::render_owner::DocumentId,
        RiddenJob { answer, sent }: RiddenJob,
    ) -> FrameJobAnswer {
        let FrameJobAnswer::Rounds(FrameOutput { end, answer }) = answer;
        let stands = crate::render_owner::layout_mark(document) == sent
            && match end {
                // A round that laid nothing out ends the frame without the document being asked again.
                FrameJobEnd::Over(_) => {
                    let facts = self.inputs.host.document_facts(main_thread);
                    !facts.document_needs_layout_tree_build && !facts.top_layer_work_pending
                }
                FrameJobEnd::Settled(_) | FrameJobEnd::NextRound(_) | FrameJobEnd::ReadiedForFlight { .. } => true,
            };
        if stands {
            return FrameJobAnswer::Rounds(FrameOutput { end, answer });
        }
        let owner = match end {
            FrameJobEnd::Settled(owner) | FrameJobEnd::NextRound(owner) => owner,
            FrameJobEnd::Over(_) | FrameJobEnd::ReadiedForFlight { .. } => OwnerEndFacts {
                arena_laid_out: false,
                engine_style_follows: false,
                deferred_element_style_inputs: false,
            },
        };
        FrameJobAnswer::Rounds(FrameOutput {
            end: FrameJobEnd::NextRound(OwnerEndFacts {
                arena_laid_out: false,
                ..owner
            }),
            answer: None,
        })
    }

    /// Runs `job`, a job of the frame, on the render owner, which holds `document`'s render state, and waits for its
    /// answer.
    fn run_job_on_owner(&mut self, document: crate::render_owner::DocumentId, job: FrameJob) -> FrameJobAnswer {
        let frame = std::ptr::from_mut(self);
        let job = std::cell::Cell::new(Some(job));
        let OwnerFrameJobAnswer(answer) = crate::stage_thread::wait_for_owner(
            crate::render_owner::LockstepProof::layout_update(),
            |reply| crate::render_owner::ToOwner::Layout {
                document,
                job: Box::new(OwnerFrameJob {
                    // SAFETY: This thread waits for the job, and reaches the frame only once it has the answer.
                    frame: unsafe { crate::stage_thread::CallerWaits::new(frame) },
                    job: job.take().expect("a job is sent once"),
                    run: Self::run_job_in_state,
                }),
                reply,
            },
            |owner| {
                // SAFETY: The frame runs for the update the arena is in, on the document thread, which waits for
                // nothing.
                let state = unsafe { ArenaHandle::held_by_waiting_thread(owner, (*frame).inputs.arena_handle) };
                let job = job.take().expect("a job runs once");
                // SAFETY: As above.
                OwnerFrameJobAnswer(unsafe {
                    crate::stage_thread::CallerWaits::new(run_job(owner, Self::run_job_in_state, frame, job, state))
                })
            },
        );
        answer.into_inner()
    }

    /// Runs `job` with the render state the frame was handed.
    ///
    /// # Safety
    ///
    /// Nothing but the thread running the job reaches the frame and its arena until this returns.
    unsafe fn run_job_in_state(frame: *mut LayoutFrame, job: FrameJob) -> FrameJobAnswer {
        // SAFETY: Guaranteed by the caller.
        let frame = unsafe { &mut *frame };
        let FrameJob::Rounds(input) = job;
        FrameJobAnswer::Rounds(frame.run_rounds_job(input))
    }

    /// Runs a job of the frame's rounds, on the owner, from the round `input` hands it, and answers where the job
    /// ended, with what the rounds left the frame owing the document thread resolved from the arena, and the answer to
    /// the query the job came with.
    fn run_rounds_job(&mut self, input: FrameInput) -> FrameOutput {
        let FrameInput {
            round:
                LayoutRoundFacts {
                    facts,
                    selection,
                    document_style,
                },
            query,
            readies_flight,
        } = input;
        if let Some(document_style) = document_style {
            // SAFETY: The document thread keeps the style alive while it waits for the job.
            unsafe { self.arena().publish_document_style(&document_style) };
        }
        // The commits of the rounds stamp the selection states of the boxes they build from the selection as it is
        // now, as nothing on the document thread changes the tree until the job is over.
        self.selection = selection;
        let end = if readies_flight {
            self.ready_round_for_flight(facts)
        } else {
            self.run_rounds(facts)
        };
        // SAFETY: The job runs with the state the owner handed the frame, which nothing else reaches.
        unsafe { self.resolve_owed_host_halves() };
        let answer = query.map(|query| {
            // SAFETY: As above.
            let arena = unsafe { &mut *self.state() }.arena_mut();
            crate::render_owner::Answer::prepare(query, arena);
            crate::render_owner::Answer::of(query, arena)
        });
        FrameOutput { end, answer }
    }

    /// Readies the round to go in flight, where it lays out; a round that does not ends the frame at once.
    fn ready_round_for_flight(&mut self, facts: FfiLayoutUpdateDocumentFacts) -> FrameJobEnd {
        match self.ready_round(&facts) {
            RoundReadiness::LaysOut { builds } => FrameJobEnd::ReadiedForFlight { facts, builds },
            // SAFETY: The job runs with the state the owner handed the frame, which nothing else reaches.
            RoundReadiness::DoesNotLayOut => match unsafe { self.run_round_through_pass(facts) } {
                RoundEnd::Over(end) => FrameJobEnd::Over(end),
                RoundEnd::UnlessHostLeftWork => FrameJobEnd::Settled(OwnerEndFacts::read(self.arena())),
                RoundEnd::AnotherRound => FrameJobEnd::NextRound(OwnerEndFacts::read(self.arena())),
            },
        }
    }

    /// Runs the rounds of the frame from the round whose style ran, with the facts `facts` the document read after it,
    /// to the frame's end, or to a round the owner does not start itself.
    fn run_rounds(&mut self, facts: FfiLayoutUpdateDocumentFacts) -> FrameJobEnd {
        loop {
            let _ = self.ready_round(&facts);
            // SAFETY: The job runs with the state the owner handed the frame, which nothing else reaches.
            match unsafe { self.run_round_through_pass(facts) } {
                RoundEnd::Over(end) => return FrameJobEnd::Over(end),
                RoundEnd::UnlessHostLeftWork => return FrameJobEnd::Settled(OwnerEndFacts::read(self.arena())),
                RoundEnd::AnotherRound => {
                    let owner = OwnerEndFacts::read(self.arena());
                    if !self.starts_next_round_itself(&facts, &owner) {
                        return FrameJobEnd::NextRound(owner);
                    }
                    // What the rounds so far owe is resolved from the arena as they left it, before the next one
                    // changes it.
                    // SAFETY: As above.
                    unsafe { self.resolve_owed_host_halves() };
                    self.layout_pass += 1;
                }
            }
        }
    }

    /// Whether the owner starts the round after one that left work itself: nothing the rounds so far owe the document
    /// thread leaves it work, and the round has no style to run and no top layer change to take in, so the document's
    /// facts and selection are as it read them for the round before.
    fn starts_next_round_itself(&self, facts: &FfiLayoutUpdateDocumentFacts, owner: &OwnerEndFacts) -> bool {
        self.owes_the_host_no_work()
            && !owner.next_round_runs_style(facts)
            && !facts.top_layer_work_pending
            && self.may_start_round()
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
    /// the document read for it once it had run the round's style ahead of the update.
    ///
    /// # Safety
    ///
    /// The frame must run for the update the arena is in, and no stage may reach it until this returns.
    unsafe fn start_first_round(&mut self, main_thread: &crate::stage::MainThread) -> LayoutRoundFacts {
        self.layout_pass += 1;
        // SAFETY: Guaranteed by the caller.
        unsafe { self.pay_host_halves(main_thread) };
        self.first_round.take().unwrap_or_else(|| {
            debug_assert!(false, "the first round's facts come with the update");
            // The round goes on with the facts as they are, and without a selection.
            LayoutRoundFacts {
                facts: self.inputs.host.document_facts(main_thread),
                selection: None,
                document_style: None,
            }
        })
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
    unsafe fn run_round_through_pass(&mut self, facts: FfiLayoutUpdateDocumentFacts) -> RoundEnd {
        match self.run_round(facts) {
            FrameStep::PassReady(pass) => {
                // SAFETY: Guaranteed by the caller.
                let laid_out = unsafe { pass.run() };
                self.finish_round(laid_out)
            }
            FrameStep::Ended(end) => end,
        }
    }

    /// Runs the rest of a round whose style the document thread has run, up to its full layout
    /// pass, or until the round ends in another round or the end of the frame.
    fn run_round(&mut self, facts: FfiLayoutUpdateDocumentFacts) -> FrameStep {
        if !self.round_lays_out(&facts) {
            self.messages.prepare_for_rendering = true;
            return FrameStep::Ended(RoundEnd::Over(FfiLayoutUpdateEnd::InUpdate));
        }

        let mut registered_partial_relayout_roots = self.arena().take_partial_relayout_boundary_roots();

        // NOTE: If this is a document hosting <template> contents, layout is unnecessary.
        if self.inputs.is_template_contents_document {
            return FrameStep::Ended(RoundEnd::Over(FfiLayoutUpdateEnd::InUpdate));
        }

        let mut needs_layout_tree_rebuild = self.needs_layout_tree_rebuild(&facts);

        match self.try_partial_relayout(
            &facts,
            &mut registered_partial_relayout_roots,
            &mut needs_layout_tree_rebuild,
        ) {
            PartialRelayout::Done => return FrameStep::Ended(RoundEnd::UnlessHostLeftWork),
            PartialRelayout::NeedsAnotherLayoutPass => return FrameStep::Ended(RoundEnd::AnotherRound),
            PartialRelayout::NotEligible => {}
        }
        drop(registered_partial_relayout_roots);

        if needs_layout_tree_rebuild {
            let state = self.state();
            let (walked, mut host_half) = self.walk_layout_tree_build();
            let needs_another_build_pass = walked.outcome.needs_another_build_pass;
            let counters_were_stale =
                !needs_another_build_pass && self.reconcile_stale_list_item_counters(&walked, &mut host_half);
            let pass_follows = !needs_another_build_pass && !counters_were_stale;
            // SAFETY: The frame runs for the update the arena is in.
            let pass_sources = pass_follows.then(|| unsafe { LayoutPassSources::read(state) });
            self.owe_tree_build_host_half(host_half);
            self.note_layout_tree_build(&walked.outcome);
            if needs_another_build_pass {
                return FrameStep::Ended(RoundEnd::AnotherRound);
            }

            // The full layout below covers every boundary the build's invalidation registered.
            drop(self.arena().take_partial_relayout_boundary_roots());

            // The list owners the reconciliation reported are marked for another build as the
            // next style round pays the build's host half, after the reset of the full tree update
            // flag.
            self.arena().set_needs_full_layout_tree_update(false);

            let Some(pass_sources) = pass_sources else {
                return FrameStep::Ended(RoundEnd::AnotherRound);
            };
            self.pass_sources = Some(pass_sources);
        }

        let layout_root = self.arena().layout_root();
        assert!(!layout_root.is_invalid(), "a full layout pass needs a layout root");
        FrameStep::PassReady(PendingLayoutPass {
            state: self.state(),
            layout_root,
            sources: self.take_pass_sources(),
            facts,
        })
    }

    /// Ends the round of a full layout pass that has run: what derives from its commit, and whether
    /// the loop has stabilized as far as what the frame owns shows. The frame then ends, unless the
    /// host halves of its commits left work for another round.
    fn finish_round(&mut self, laid_out: LaidOutPass) -> RoundEnd {
        let facts = self.note_laid_out_pass(laid_out);

        // Layout-only invalidations still need to be flushed before we can exit.
        if self.commit_left_layout_work(&facts) {
            return RoundEnd::AnotherRound;
        }

        // The document thread asks what the host halves left as it takes in the frame's end, and
        // nothing else runs on it until then, so if they left nothing the loop has stabilized.
        RoundEnd::UnlessHostLeftWork
    }

    /// Takes in a full layout pass that has run: its commit's host half is left for the next join,
    /// and what derives from the commit is done. Answers the facts the pass ran with.
    fn note_laid_out_pass(&mut self, laid_out: LaidOutPass) -> FfiLayoutUpdateDocumentFacts {
        let LaidOutPass {
            commit_host_half,
            facts,
        } = laid_out;
        self.owe_host_half(OwedHostHalf::Commit(commit_host_half));
        self.messages.full_layouts_performed += 1;
        self.note_layout_commit(true);
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
            container_query_evaluation_is_pending: self
                .arena()
                .with_style_store(|engine| engine.has_size_containers_needing_evaluation_after_layout()),
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
            let state = self.state();
            let (walked, mut host_half) = self.walk_layout_tree_build();
            let needs_another_build_pass = walked.outcome.needs_another_build_pass;
            let counters_were_stale = self.reconcile_stale_list_item_counters(&walked, &mut host_half);
            let pass_follows = !counters_were_stale && !needs_another_build_pass;
            // As after a full layout's build, the host half waits for the next join, which finds
            // what paying it changes (it can resize this document's viewport through its embedding
            // document).
            // SAFETY: The frame runs for the update the arena is in.
            let pass_sources = pass_follows.then(|| unsafe { LayoutPassSources::read(state) });
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
        }

        let layout_root = self.arena().layout_root();
        let Some(partial_relayout_roots) = self
            .arena()
            .plan_partial_relayout_with_pending_rebuilt_roots(layout_root, registered_partial_relayout_roots)
        else {
            return PartialRelayout::NotEligible;
        };
        for boundary in &partial_relayout_roots {
            debug_assert!(node_facts::kind_is_box(self.arena().data(boundary.root()).kind.get()));
        }

        let state = self.state();
        let LayoutPassSources { content } = self.take_pass_sources();
        // SAFETY (for the steps below): The frame runs for the update the arena is in, the
        // planned boundaries and the viewport box stay live across them, and no row was freed
        // since the sources were read.
        unsafe { apply_enrolled_content_sources(state, content) };
        for &boundary in &partial_relayout_roots {
            // The next boundary's pass starts from the arena the previous commit settled; the
            // commit's host half waits for the next join.
            let output = unsafe {
                compute_subtree_layout_fragments(
                    state,
                    boundary,
                    facts.viewport_inline_size_raw,
                    facts.viewport_block_size_raw,
                    facts.document_in_quirks_mode,
                )
            };
            let pending_commit = unsafe { commit_subtree_layout_to_arena(state, boundary.root(), &output) };
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

/// A layout frame a document's clock ticks lay out in on the render side while the main thread idles. The main thread
/// makes it with what its rounds read from the
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
    pub(crate) unsafe fn run_round(&mut self, owner: &crate::render_owner::Owner) -> bool {
        // A tick runs beside the document thread rather than as a job of the owner, and reaches the state the
        // document named.
        // SAFETY: Guaranteed by the caller.
        self.frame.state = unsafe { ArenaHandle::held_by_waiting_thread(owner, self.frame.inputs.arena_handle) };
        let laid_out_whole = self.run_round_in_state();
        self.frame.state = std::ptr::null_mut();
        laid_out_whole
    }

    /// As for [`Self::run_round`], with the frame's state handed to it.
    fn run_round_in_state(&mut self) -> bool {
        if !self.frame.round_lays_out(&self.facts) {
            return true;
        }
        if self.frame.needs_layout_tree_rebuild(&self.facts) || self.frame.inputs.is_template_contents_document {
            return false;
        }
        // SAFETY: The frame holds its document's state.
        self.frame.pass_sources = Some(unsafe { LayoutPassSources::read(self.frame.state()) });
        self.laid_out = true;
        self.rounds += 1;
        // SAFETY: As above.
        let end = unsafe { self.frame.run_round_through_pass(self.facts) };
        // SAFETY: As above.
        unsafe { self.frame.resolve_owed_host_halves() };
        match end {
            RoundEnd::Over(_) | RoundEnd::UnlessHostLeftWork => !self.frame.commit_left_layout_work(&self.facts),
            RoundEnd::AnotherRound => false,
        }
    }

    pub(crate) fn laid_out(&self) -> bool {
        self.laid_out
    }

    pub(crate) fn rounds(&self) -> u32 {
        self.rounds
    }
}

/// Takes in the layout frame of a document's clock ticks, and ends the update the document began for
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
    // SAFETY: Guaranteed by the caller.
    unsafe { frame.frame.take_in_end(main_thread, FfiLayoutUpdateEnd::InUpdate) };
}

/// Makes the frame a document's clock ticks lay out in, with the document as it stands now, which the
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
    let LayoutRoundFacts { facts, selection, .. } = round;
    ClockLayoutFrame {
        frame: LayoutFrame {
            state: std::ptr::null_mut(),
            inputs: FrameInputs {
                host,
                arena_handle,
                reason_is_inspect_devtools_layout_data: false,
                is_template_contents_document: false,
            },
            messages: FrameMessages::default(),
            layout_pass: 0,
            connected_element_count: 0,
            pass_sources: None,
            tree_build_document_style_node: None,
            selection,
            owed_host_halves: Cell::default(),
            host_payments: Vec::new(),
            first_round: None,
            owner_end: None,
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
    if unsafe { super::HostTables::beside_frame(arena_handle) }.layout_update_is_running() {
        FfiLayoutFrameState::MainInsideJoin
    } else {
        FfiLayoutFrameState::Idle
    }
}

/// A forced read's layout frame, which the document offers the style transaction of the frame's first round before it
/// runs the round's style: the owner runs the frame's first job right after the transaction, in the same message
/// ([`crate::render_owner::ToOwner::Style`]), so that the read waits for the owner once for its style and its layout.
struct OfferedFrame {
    frame: Box<LayoutFrame>,
    query: Option<crate::render_owner::Query>,
    ride: Ride,
}

/// Where the first job of an [`OfferedFrame`] stands. Only the update's first style transaction takes it: the rounds
/// after the first read what the drain of the transaction before them left.
enum Ride {
    /// The document has taken no transaction since it offered the frame.
    Offered,
    /// The job rides the transaction the document takes next, with the round the document read for it.
    Readied(Box<OwnerFrameJob>),
    /// The owner ran the job right after the transaction.
    Ran(RiddenJob),
    /// The job did not ride: the frame sends it on its own, with its round read once the round's style has run.
    Declined,
}

/// The answer of a frame's first job, which the owner ran right after the style transaction of its round, when the
/// document thread had sent it what `sent` marks.
struct RiddenJob {
    answer: FrameJobAnswer,
    sent: crate::render_owner::LayoutMark,
}

thread_local! {
    /// On a document thread, the frames its documents offered their style transactions.
    static OFFERED_FRAMES: std::cell::RefCell<Vec<OfferedFrame>> = const { std::cell::RefCell::new(Vec::new()) };
}

/// Takes the frame the document whose arena `arena_handle` names offered, if it offered one.
fn take_offered_frame(arena_handle: *mut c_void) -> Option<OfferedFrame> {
    OFFERED_FRAMES.with_borrow_mut(|offered| {
        let index = offered
            .iter()
            .position(|offered| offered.frame.inputs.arena_handle == arena_handle)?;
        Some(offered.swap_remove(index))
    })
}

/// Runs `step` on the frame the document whose arena `arena_handle` names offered, if it offered one, and answers what
/// the step answers. The step may call the document, which may run the layout update of another document.
fn with_offered_frame<R>(arena_handle: *mut c_void, step: impl FnOnce(&mut OfferedFrame) -> R) -> Option<R> {
    let mut offered = take_offered_frame(arena_handle)?;
    let result = step(&mut offered);
    OFFERED_FRAMES.with_borrow_mut(|frames| frames.push(offered));
    Some(result)
}

/// Offers the style transaction the document takes next the first round of the frame of the forced read whose layout
/// update the document thread runs.
///
/// # Safety
///
/// As for [`update_layout`], before the update's first style runs.
unsafe fn offer_first_round(main_thread: &crate::stage::MainThread, arena_handle: *mut c_void) {
    let inputs = FrameInputs {
        host: layout_update_host(main_thread),
        arena_handle,
        reason_is_inspect_devtools_layout_data: false,
        is_template_contents_document: false,
    };
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { super::ArenaHandle::document_of(arena_handle) };
    let stale = take_offered_frame(arena_handle);
    debug_assert!(stale.is_none(), "a layout update offers one frame");
    OFFERED_FRAMES.with_borrow_mut(|frames| {
        frames.push(OfferedFrame {
            frame: Box::new(LayoutFrame::new(inputs, None)),
            query: crate::render_owner::asked_about(document),
            ride: Ride::Offered,
        });
    });
}

/// Readies the first job of the frame the document offered to ride the style transaction it takes next, if it offered
/// one and has taken no transaction since: the document reads the round as it reads a round it starts. A round that
/// builds a tree does not ride: the rest of the round's style may mark more of the tree.
///
/// # Safety
///
/// As for [`update_layout`], with the transaction about to be taken.
unsafe fn ready_ride(main_thread: &crate::stage::MainThread, arena_handle: *mut c_void) {
    with_offered_frame(arena_handle, |offered| {
        if !matches!(offered.ride, Ride::Offered) {
            return;
        }
        let round = offered.frame.inputs.host.start_round(main_thread, false);
        if round.facts.document_needs_layout_tree_build
            || round.facts.top_layer_work_pending
            || round.document_style.is_some()
        {
            offered.ride = Ride::Declined;
            return;
        }
        // The job may build, which reads the marks: they are its until this thread pays what the job owes it.
        // SAFETY: Guaranteed by the caller, with no job of the frame outstanding.
        unsafe { super::tree_update_marks::lend_to_frame(arena_handle) };
        offered.ride = Ride::Readied(Box::new(OwnerFrameJob {
            // SAFETY: This thread waits for the transaction the job rides, and reaches the frame only once it has the
            // answer. The frame is boxed, so it stays where it is until then.
            frame: unsafe { crate::stage_thread::CallerWaits::new(std::ptr::from_mut(&mut *offered.frame)) },
            job: FrameJob::Rounds(FrameInput {
                round,
                query: offered.query,
                readies_flight: false,
            }),
            run: LayoutFrame::run_job_in_state,
        }));
    });
}

/// Takes the job the document readied to ride the style transaction it takes now (see [`ready_ride`]), if it readied
/// one.
pub(crate) fn take_readied_ride(arena_handle: *mut c_void) -> Option<Box<OwnerFrameJob>> {
    with_offered_frame(arena_handle, |offered| {
        match std::mem::replace(&mut offered.ride, Ride::Declined) {
            Ride::Readied(job) => Some(job),
            ride => {
                offered.ride = ride;
                None
            }
        }
    })
    .flatten()
}

/// Settles the ride of the job [`take_readied_ride`] took, with the owner's answer to it where the transaction let it
/// run. A job that did not run is declined.
///
/// # Safety
///
/// On the document thread, right after the transaction the job rode, for the arena `arena_handle`.
pub(crate) unsafe fn settle_ride(arena_handle: *mut c_void, answer: Option<OwnerFrameJobAnswer>) {
    let Some(OwnerFrameJobAnswer(answer)) = answer else {
        // SAFETY: Guaranteed by the caller. The job did not run, and the marks are the document's.
        unsafe { super::tree_update_marks::take_back_from_frame(arena_handle) };
        return;
    };
    // SAFETY: Guaranteed by the caller.
    let sent = crate::render_owner::layout_mark(unsafe { super::ArenaHandle::document_of(arena_handle) });
    let settled = with_offered_frame(arena_handle, |offered| {
        offered.ride = Ride::Ran(RiddenJob {
            answer: answer.into_inner(),
            sent,
        });
    });
    debug_assert!(
        settled.is_some(),
        "a job rides the transaction of the frame that offered it"
    );
}

/// Whether the first job of the frame the document offered rode the style transaction of its round, and ran.
fn first_round_rode_style(arena_handle: *mut c_void) -> bool {
    OFFERED_FRAMES.with_borrow(|offered| {
        offered
            .iter()
            .any(|offered| offered.frame.inputs.arena_handle == arena_handle && matches!(offered.ride, Ride::Ran(_)))
    })
}

/// Runs the layout update as one frame, which the render owner runs job by job while the document thread waits (see
/// [`LayoutFrame::run_with_owner`]): the document thread hands each job everything its rounds read from the document,
/// and nothing of a job comes back to it before the job has ended. Once the frame ends, the document thread has paid
/// what the frame owed it, applied the frame's messages and ended the update on the document side, so the document is
/// idle once the update returns finished, with the owner's answer to the query the document thread asked about it, if
/// it asked one. A frame whose full layout pass is submitted ends that way once the document thread takes it back
/// instead.
///
/// # Safety
///
/// As for [`arena`], on the document thread. The first round's selection, if any, must be valid.
unsafe fn update_layout(
    main_thread: &crate::stage::MainThread,
    arena_handle: *mut c_void,
    inputs: &FfiLayoutUpdateInputs,
) -> FfiLayoutUpdateOutcome {
    let host = layout_update_host(main_thread);
    // SAFETY: Guaranteed by the caller.
    debug_assert!(
        unsafe { super::HostTables::beside_frame(arena_handle) }.layout_update_is_running(),
        "the layout update runs between layout_arena_begin_update_layout and its end"
    );
    let may_submit_pass = inputs.may_submit_pass;
    // SAFETY: Guaranteed by the caller.
    let first_round = unsafe { LayoutRoundFacts::from_ffi(&inputs.first_round) };
    let inputs = FrameInputs {
        host,
        arena_handle,
        reason_is_inspect_devtools_layout_data: inputs.reason_is_inspect_devtools_layout_data,
        is_template_contents_document: inputs.is_template_contents_document,
    };
    let submits_pass = may_submit_pass && crate::stage_thread::submits();
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { super::ArenaHandle::document_of(arena_handle) };
    // The owner answers the query the document thread asked about the update from the frame it runs.
    let query = if submits_pass {
        None
    } else {
        crate::render_owner::asked_about(document)
    };
    // A frame the document offered its first round's style transaction goes on from there.
    let offered = take_offered_frame(arena_handle);
    debug_assert!(
        offered.is_none() || !submits_pass,
        "only a forced read offers its frame"
    );
    // SAFETY: Guaranteed by the caller.
    let driven = unsafe { LayoutPassJob::prepare(main_thread, inputs, submits_pass, first_round, query, offered) };
    // SAFETY: As above.
    unsafe { go_on_from_driven_frame(main_thread, arena_handle, driven, submits_pass) }
}

/// Goes on with a layout update from where the document thread's run of its frame left it: hands the document thread
/// the owner's answer to the query it asked, or readies or submits the full layout pass the frame readied.
///
/// # Safety
///
/// As for [`update_layout`].
unsafe fn go_on_from_driven_frame(
    main_thread: &crate::stage::MainThread,
    arena_handle: *mut c_void,
    driven: DrivenFrame,
    submits_pass: bool,
) -> FfiLayoutUpdateOutcome {
    debug_assert!(
        submits_pass || matches!(driven, DrivenFrame::Ended(_)),
        "a frame that submits no pass ends on the document thread"
    );
    let pass = match driven {
        DrivenFrame::Ended(answer) => {
            if let Some(answer) = answer {
                crate::render_owner::answered(answer);
            }
            return FfiLayoutUpdateOutcome::Finished;
        }
        DrivenFrame::Pass(pass) => *pass,
    };
    // The flight goes on to record the document once it has laid it out, if the document seals what
    // that reads before it submits the flight, with the rest of the round's host steps done.
    crate::painting::ffi::discard_sealed_flight_paint();
    let Some(host_tables) = main_thread.host_tables() else {
        debug_assert!(false, "layout node arena has no host tables");
        // With nowhere to keep the flight for the document to seal, it goes unsealed: it lays the
        // document out and records nothing.
        // SAFETY: Guaranteed by the caller.
        unsafe { submit_flight(arena_handle, pass) };
        return FfiLayoutUpdateOutcome::PassSubmitted;
    };
    let prepared = host_tables.prepared_flight.replace(Some(pass));
    debug_assert!(
        prepared.is_none(),
        "a document submits the flight it prepared before another"
    );
    FfiLayoutUpdateOutcome::FlightReady
}

/// Submits the flight the document's layout update readied, once the document has sealed what its
/// recording reads.
///
/// # Safety
///
/// As for [`update_layout`], right after it answered with a flight ready.
unsafe fn submit_prepared_flight(main_thread: &crate::stage::MainThread, arena_handle: *mut c_void) {
    let pass = main_thread
        .host_tables()
        .and_then(|host_tables| host_tables.prepared_flight.take());
    debug_assert!(pass.is_some(), "the document's layout update readied a flight");
    if let Some(pass) = pass {
        // SAFETY: Guaranteed by the caller.
        unsafe { submit_flight(arena_handle, pass) };
    }
}

/// Submits the flight that runs the rest of the round `pass` has readied.
///
/// # Safety
///
/// As for [`update_layout`], which readied the pass.
unsafe fn submit_flight(arena_handle: *mut c_void, pass: LayoutPassJob) {
    let flight = crate::flight::Flight::from_layout_pass(arena_handle, pass);
    // SAFETY: The frame in flight owns what the flight's stages reach until the document thread
    // takes it back: every document-thread path to the arena joins the frame first.
    unsafe { crate::flight::submit(arena_handle, flight) };
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

/// Where the document thread's run of a layout frame left it.
enum DrivenFrame {
    /// The frame ended on the document thread, and the update with it, with the owner's answer to the query the
    /// document thread asked.
    Ended(Option<crate::render_owner::Answer>),
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
    /// Runs the layout frame of `inputs` with the owner up to its full layout pass, if `submits_pass`, and returns the
    /// pass, unless the frame ended on the document thread instead, with the owner's answer to `query`.
    ///
    /// # Safety
    ///
    /// As for [`update_layout`]. The pass returned has to run in a frame in flight that owns the arena.
    unsafe fn prepare(
        main_thread: &crate::stage::MainThread,
        inputs: FrameInputs,
        submits_pass: bool,
        first_round: LayoutRoundFacts,
        query: Option<crate::render_owner::Query>,
        offered: Option<OfferedFrame>,
    ) -> DrivenFrame {
        let (mut frame, ridden) = match offered {
            Some(OfferedFrame { frame, ride, .. }) => {
                let mut frame = *frame;
                let arena_handle = inputs.arena_handle;
                frame.inputs = inputs;
                match ride {
                    Ride::Ran(ridden) => (frame, Some(ridden)),
                    Ride::Offered | Ride::Declined => {
                        frame.first_round = Some(first_round);
                        (frame, None)
                    }
                    // No transaction took the readied job: the document had no root to take one for.
                    Ride::Readied(_) => {
                        // SAFETY: Guaranteed by the caller. The job did not run, and the marks are the document's.
                        unsafe { super::tree_update_marks::take_back_from_frame(arena_handle) };
                        frame.first_round = Some(first_round);
                        (frame, None)
                    }
                }
            }
            None => (LayoutFrame::new(inputs, Some(first_round)), None),
        };
        // SAFETY: Guaranteed by the caller.
        let facts = match unsafe { frame.run_with_owner(main_thread, submits_pass, query, ridden) } {
            FrameRun::Ended(answer) => return DrivenFrame::Ended(answer),
            FrameRun::ReadiedForFlight(facts) => facts,
        };
        // The style round paid what the frame owed. What the round in flight comes to owe (its tree
        // build's host half, its commits' host halves) is paid as the frame is taken back: tasks that
        // run beside the flight and would read it reach it through the arena's doors, which join.
        debug_assert!(
            frame.owed_host_halves.get_mut().is_empty() && frame.host_payments.is_empty(),
            "the style round pays what the frame owed"
        );
        DrivenFrame::Pass(Box::new(Self {
            arena_handle: frame.inputs.arena_handle as usize,
            // SAFETY: Guaranteed by the caller.
            frame: unsafe { crate::stage_thread::FrameOwns::new(frame) },
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

    /// Hands the frame the render state of its document, which the flight that runs it holds.
    pub(crate) fn hand_state(&mut self, state: *mut ArenaHandle) {
        self.frame.get_mut().state = state;
    }

    /// Runs the rest of the frame's round, on the stage that owns the arena, and answers how far a
    /// frame made from the arena before the frame is taken back may go.
    pub(crate) fn run(self) -> RoundInFlight {
        let Self { frame, facts, ran, .. } = self;
        let mut frame = frame.into_inner();
        // Where the frame would go on from here is the next layout update's to find: the take-back
        // ends it wherever the document thread is.
        // SAFETY: The frame in flight owns the arena, as LayoutPassJob::prepare requires.
        let end = unsafe { frame.run_round_through_pass(facts) };
        let may_be_painted = matches!(end, RoundEnd::UnlessHostLeftWork) && frame.may_be_painted_before_take_back();
        let may_be_presented = may_be_painted && frame.owes_the_host_no_work();
        // SAFETY: As above.
        unsafe { frame.resolve_owed_host_halves() };
        // The document thread tells from these whether a recording made after the round stands, as it takes it back.
        frame.owner_end = Some(OwnerEndFacts::read(frame.arena()));
        // The frame's layout goes out at once, as committed: a read beside the rest of the flight reads it without
        // asking.
        // SAFETY: As above.
        unsafe { &mut *frame.state() }.arena_mut().publish_committed_rows();
        frame.state = std::ptr::null_mut();
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
/// middle of a DOM mutation. It starts no style update there.
///
/// # Safety
///
/// On the document thread, which has just taken back the frame in flight that ran the round.
unsafe fn finish_layout_frame(main_thread: &crate::stage::MainThread, mut frame: LayoutFrame) {
    // SAFETY: The frame is the document thread's again, and it runs for the update the arena is in.
    unsafe { frame.pay_host_halves(main_thread) };
    // The list owners the round's build found showing stale counters were marked for the build of
    // the next layout update as its host half was paid, as the next style round would have marked
    // them.
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
    // SAFETY: The frame is the document thread's again, and it runs for the update the arena is in.
    unsafe { frame.pay_host_halves(main_thread) };
    let facts = host.document_facts(main_thread);
    // What the arena and the engine showed as the round ended still holds: paying hands nothing back into them.
    let owner = frame.owner_end.take();
    debug_assert!(
        owner.is_some(),
        "a round a flight ran leaves what the owner showed at its end"
    );
    let recording_stands = owner.is_some_and(|owner| !owner.left_work(&facts));
    let mut messages = std::mem::take(&mut frame.messages);
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

/// Brings the document's style engine home for a main-side change of what the style drain reads on the main thread:
/// a style pass in flight is taken back and drained first. `file` and `line` name the C++ call site for the
/// forced-join log.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread. `file` and `file_length` must name a
/// string that lives for the rest of the process, as a `SourceLocation`'s file name does.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_join_frame_before_style_drain_reads(
    arena: *mut c_void,
    file: *const u8,
    file_length: usize,
    line: u32,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: The caller passes a string that lives for the rest of the process.
    let file = unsafe { crate::stage_thread::call_site_file(file, file_length) };
    // SAFETY: Guaranteed by the caller.
    unsafe { super::HostTables::beside_frame(arena) }
        .style_engine()
        .bring_home_at(file, line);
}

/// Waits for the document's frame in flight only if one of its stages owns the arena (a layout pass
/// or a recording), not for a style pass alone, which records its arena without owning it. For a
/// write to what the arena's stages read. `file` and `line` name the C++ call site for the
/// forced-join log.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread. `file` and `file_length` must name a
/// string that lives for the rest of the process, as a `SourceLocation`'s file name does.
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
/// [`FfiLayoutFrameEffects::owed_image_resources`], which lists the rows live as the frame ended):
/// the image box stops waiting for the provider it owns, which the document attaches next.
///
/// # Safety
///
/// `arena` must be a live handle on the document thread, which is taking in the frame's effects.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn layout_arena_hand_over_owed_image_resources(arena: *mut c_void, row: NodeSlotId) {
    // SAFETY: As above.
    unsafe { crate::layout::HostTables::beside_frame(arena) }
        .image_boxes_awaiting_owned_provider
        .borrow_mut()
        .remove(&row);
    // SAFETY: As above.
    let document = unsafe { ArenaHandle::document_of(arena) };
    crate::render_owner::send_arena_change(document, crate::render_owner::ArenaChange::OwnedProviderHandedOver(row));
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
    let host_tables = unsafe { super::HostTables::from_handle(arena) };
    // Reaching the host tables has joined a frame in flight that owns the arena, and the style update
    // ahead of it one that reaches the document's style engine; a layout update never runs under either.
    assert!(
        !crate::stage_thread::document_frame_in_flight(arena),
        "update_layout nested in a frame in flight"
    );
    host_tables.begin_layout_update();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts_for() -> FfiLayoutUpdateDocumentFacts {
        FfiLayoutUpdateDocumentFacts {
            document_is_active: true,
            document_needs_layout_tree_build: false,
            style_input_waits_on_document: false,
            top_layer_work_pending: false,
            should_collect_devtools_layout_data: false,
            document_in_quirks_mode: false,
            viewport_inline_size_raw: 0,
            viewport_block_size_raw: 0,
            document_style_node: 0,
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

        arena.free_subtree(viewport).invoke_callbacks();
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
