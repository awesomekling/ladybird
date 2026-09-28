/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Clock frames: the animations of a document ticked on the render owner.
//!
//! At the end of a rendering update in which nothing but the running animations of a document's
//! timeline would change what the next one shows, the main thread starts the document's clock: it
//! sends the owner what the clock ticks ([`ClockMessage`]), and the owner keeps it in the document's
//! render state as its [`DocumentClock`]. A tick is a job on the owner over that state: it samples
//! those animations at one time and installs the records they compose into the document's layout
//! arena, ahead of the host. What it leaves (the samples, the time, how it ended, the layout frame
//! it laid out in) it publishes ([`ClockPublication`]), and the main thread reads that and adopts
//! the samples once the tick is over. Each rendering update of the document submits one tick as a
//! run of its own, labelled `clock`, ahead of its layout pass.
//!
//! A render clock (`Web::Compositor::RenderClock`, a thread of its own) also has the owner tick a clock, at
//! the display ticks the compositor delivers for the clock's compositor context, whatever the main
//! thread is doing: see [`rust_render_clock_post_tick`]. Such a tick is a job on the owner like any
//! other, so it never runs beside a unit of a rendering update, and it presents what it lays out while
//! the main thread lends it the navigable's presenter. The main thread adopts what the ticks published
//! when it next runs (see [`rust_render_clock_take_ticks_to_adopt`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::num::NonZeroU64;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::ThreadId;

use crate::css::style::StyleEngine;
use crate::css::style::bridge::{
    FfiRowSampledInPass, FfiStyleInvalidationField, sample_installed_record_for_clock_tick,
};
use crate::css::style::engine_home::{AtHome, Holder, Owed, StyleEngineLoan};
use crate::css::style::tree::StyleNodeID;
use crate::css::style::{StyleEngineHandle, StyleEngineInputHandle};
use crate::layout::node_data::NodeSlotId;
use crate::layout::update_layout::ClockLayoutFrame;
use crate::layout::{ArenaHandle, LayoutNodeArena};
use crate::painting::query_snapshot::{FfiQuerySnapshotViewport, QuerySnapshot};
use crate::render_owner::DocumentId;

/// An element whose animations a clock ticks.
#[derive(Clone, Copy)]
pub(crate) struct ClockTarget {
    style_node: StyleNodeID,
    /// Whether the element has a pseudo-element style that inherits from it outside its box: a
    /// ::backdrop or a ::selection style, which only the main thread derives.
    has_pseudo_element_style_outside_box: bool,
}

/// A scroll progress timeline whose animations a clock ticks, as the host planned it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiClockScrollTimeline {
    /// The style engine identity of the timeline.
    pub identity: u32,
    /// The stable identity of its scroller's scroll node: the unique id of its document (for the
    /// viewport) or of its element, its kind and its pseudo-element.
    pub node_id: i64,
    pub kind: u8,
    pub pseudo_element_type: u8,
    /// Whether the timeline follows the vertical scroll offset.
    pub vertical: bool,
    /// The scroll offset at 100% progress, in CSS pixels: the scroller's as the host last laid it out.
    pub max_scroll_offset: f64,
    /// The progress (in percent) from which, and up to which, no effect the clock ticks on the
    /// timeline changes its phase or its iteration, which the host has events for.
    pub progress_start: f64,
    pub progress_end: f64,
    /// The progress (in percent) at the scroll offset the host holds.
    pub progress: f64,
}

impl FfiClockScrollTimeline {
    fn holds(&self, progress: f64) -> bool {
        progress >= self.progress_start && progress < self.progress_end
    }
}

/// What one tick did for one target, for the host to adopt.
pub(crate) struct ClockTickEntry {
    pub(crate) style_node: StyleNodeID,
    /// The record the target held before the tick: one, so that an element that holds none adopts nothing.
    pub(crate) style_record_before: NonZeroU64,
    /// The sample; `present` is false where the engine could not take it and the host samples the
    /// target itself.
    pub(crate) sample: FfiRowSampledInPass,
    /// Whether the arena took the sample's record ahead of the host.
    pub(crate) installed_in_arena: bool,
}

/// How the host took the frame a tick recorded.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FfiClockPresent {
    /// It presented the frame.
    Presented,
    /// The main thread presents from the navigable's presenter now, and its frames show what the tick laid out.
    MainPresents,
    /// The render side cannot show the frame: the main thread has to.
    Declined,
}

/// How a tick ended.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FfiClockTickOutcome {
    /// The tick installed its samples; the host adopts them and shows the frame.
    Presented,
    /// A target needs the host: it samples that target itself, and the clock stops.
    NeedsMain,
    /// The tick reached the clock's deadline, where the host's rendering update takes over.
    PastDeadline,
    /// The clock was stopped before the tick ran.
    Stopped,
}

/// What the ticks of a document's clocks publish for the main thread: the owner writes it as a tick
/// ends, and the main thread adopts it when it next runs, beside the ticks that go on, or once it
/// has taken a submitted tick back. A document keeps one across the clocks it starts and stops: a
/// tick that ran before the owner learned of a new clock or of a stop leaves its samples here too.
pub(crate) struct ClockPublication {
    /// The timeline time of the last tick that presented, or of the start, as `f64` bits.
    time: AtomicU64,
    /// How the last tick ended.
    outcome: Mutex<Option<FfiClockTickOutcome>>,
    /// Whether the render side presented every tick the host has not adopted yet, so that adopting
    /// them repaints nothing.
    presented_since_adoption: AtomicBool,
    /// The progress (percent) at which the last tick sampled each scroll progress timeline, or the
    /// host held it at the start, by the timeline's style engine identity.
    scroll_progress: Mutex<Vec<(u32, f64)>>,
    /// What the ticks left for the host to adopt, which a display tick writes as a whole: the host
    /// takes all of one tick or none.
    adoption: Mutex<Adoption>,
    /// The layout frame the render clock's ticks lay out in, which the main thread hands the clock
    /// and takes in as it adopts the ticks.
    layout_frame: Mutex<Option<ClockLayoutFrame>>,
}

// SAFETY: `ClockTickEntry` carries a borrowed custom-property store pointer, which only the thread
// that owns the arena (and so the engine) dereferences.
unsafe impl Send for ClockPublication {}
// SAFETY: As above; everything else is behind atomics and mutexes.
unsafe impl Sync for ClockPublication {}

impl ClockPublication {
    fn new(time: f64) -> Self {
        Self {
            time: AtomicU64::new(time.to_bits()),
            outcome: Mutex::default(),
            presented_since_adoption: AtomicBool::new(false),
            scroll_progress: Mutex::default(),
            adoption: Mutex::default(),
            layout_frame: Mutex::default(),
        }
    }

    fn time(&self) -> f64 {
        f64::from_bits(self.time.load(Ordering::Acquire))
    }

    fn set_outcome(&self, outcome: FfiClockTickOutcome) {
        *self.outcome.lock().expect("clock publication outcome") = Some(outcome);
    }

    fn adoption(&self) -> std::sync::MutexGuard<'_, Adoption> {
        self.adoption.lock().expect("clock publication adoption")
    }
}

/// What the ticks of a document's clocks left for the host to adopt.
#[derive(Default)]
struct Adoption {
    /// One entry per target, which the host takes with its style engine home ([`Self::take_entries`]).
    entries: Vec<ClockTickEntry>,
    /// The committed geometry the last display tick laid out, which the host's reads answer from once it adopted the
    /// entries, as they would from a snapshot it published.
    query_snapshot: Option<QuerySnapshot>,
    /// The rows the last display tick published, which the host's row reads answer from once it adopted the entries.
    rows: Option<Arc<crate::layout::row_reads::RowSnapshot>>,
}

impl Adoption {
    /// The entries, which the host checks against what its elements hold and installs with nothing in between that
    /// takes a frame in: a frame that holds the engine would move what an element holds under a sample it checked, or
    /// drop the sample's record.
    fn take_entries(&mut self, _: AtHome) -> Vec<ClockTickEntry> {
        std::mem::take(&mut self.entries)
    }
}

/// The clock of a document, which the owner keeps in the document's render state: what its ticks
/// sample, until when, and what they publish.
pub(crate) struct DocumentClock {
    /// The compositor context whose display ticks the render clock hands the clock, or 0.
    context: u64,
    /// The main thread of the document, which a display tick runs for.
    main_thread: ThreadId,
    /// The style engine identity of the document timeline the clock ticks.
    timeline_identity: u32,
    /// The unsafe shared current time, in milliseconds, at which the timeline reads zero: a frame
    /// time `t` is timeline time `t - timeline_zero`.
    timeline_zero: f64,
    /// The timeline time at which the host has something observable to do (an event, a phase
    /// change, the end of an effect): no tick samples at or past it.
    deadline: f64,
    /// Whether the render clock leaves the clock alone: the main thread moved what it ticks, and
    /// its rendering update decides what becomes of the clock.
    paused: bool,
    /// Whether a render clock tick ended the clock: the render clock ticks it no more, and the main
    /// thread adopts what its ticks left before its rendering update stops it. A later tick would
    /// sample over records only the entries the host has not adopted yet keep alive.
    needs_main: bool,
    /// Whether the main thread stopped the clock: nothing ticks it, and it stays for what its ticks
    /// left the host until the next clock of the document replaces it.
    stopped: bool,
    targets: Vec<ClockTarget>,
    scroll_timelines: Vec<FfiClockScrollTimeline>,
    /// The rows the last tick's samples repaint, and whether they hit test differently.
    repaints: Vec<(NodeSlotId, bool)>,
    /// Whether the render side can show what the last tick installed without the main thread: no
    /// sample moved what only the main thread derives (descendants' styles, visual contexts).
    presentable: bool,
    /// Whether the last tick found nothing left for the host to adopt.
    tick_started_fresh: bool,
    published: Arc<ClockPublication>,
}

impl DocumentClock {
    /// Whether the render clock ticks the clock at the display ticks of the compositor context
    /// `context`.
    pub(crate) fn ticks_at(&self, context: u64) -> bool {
        !self.stopped && self.context == context
    }

    /// Whether the ticks left samples the host has not taken yet.
    pub(crate) fn left_samples_to_adopt(&self) -> bool {
        !self.published.adoption().entries.is_empty()
    }

    /// The timeline time of a frame shown at `frame_time` (unsafe shared current time, ms).
    fn timeline_time_at(&self, frame_time: f64) -> f64 {
        frame_time - self.timeline_zero
    }

    /// Samples the clock's targets at timeline time `time` and installs what they compose into the
    /// arena `arena_handle`, ahead of the host, which adopts the `entries` this leaves (see
    /// `style_engine_clock_tick_take_entry`). No tick samples at or past the deadline. The scroll
    /// timelines are sampled where the host held them as it started the clock.
    ///
    /// # Safety
    ///
    /// On the owner, which holds `state`, the arena of the clock's document, and `engine`, its style engine: inside
    /// the `clock` run the host submitted, or in a display tick.
    unsafe fn run_tick(
        &mut self,
        entries: &mut Vec<ClockTickEntry>,
        state: *mut ArenaHandle,
        engine: Option<&mut StyleEngine>,
        time: f64,
    ) -> FfiClockTickOutcome {
        let outcome = if time >= self.deadline {
            FfiClockTickOutcome::PastDeadline
        } else if let Some(engine) = engine {
            // The main thread pins and unpins its host's records beside the tick, which reads none of them.
            let lend = engine.lend_host_pins_beside();
            // SAFETY: Guaranteed by the caller.
            let outcome = unsafe { self.sample_and_install(entries, state, engine, time) };
            engine.restore_host_pins(lend);
            outcome
        } else {
            FfiClockTickOutcome::NeedsMain
        };
        // SAFETY: Guaranteed by the caller; nothing borrows the arena once the tick has run.
        unsafe { &mut *state }.arena_mut().publish_rows();
        if outcome == FfiClockTickOutcome::Presented {
            let previous = self.published.time();
            self.published
                .time
                .store(previous.max(time).to_bits(), Ordering::Release);
        }
        self.published.set_outcome(outcome);
        outcome
    }

    /// # Safety
    ///
    /// As for [`Self::run_tick`].
    unsafe fn sample_and_install(
        &mut self,
        entries: &mut Vec<ClockTickEntry>,
        state: *mut ArenaHandle,
        engine: &mut StyleEngine,
        time: f64,
    ) -> FfiClockTickOutcome {
        let arena_handle = state.cast::<c_void>();
        // SAFETY: The caller holds the arena, which the document's render state keeps alive.
        let arena = unsafe { &*state }.arena();
        let engine: *mut StyleEngine = engine;
        // SAFETY: The caller lent the engine; no other borrow of it is live here.
        let mut samples = unsafe { &*engine }
            .animation_timeline_samples()
            .with_time(self.timeline_identity, time);
        // A scroll progress timeline is where the host held it as it started the clock. Past the progress at
        // which an effect changes its phase or its iteration, the host has events to send.
        let mut scroll_progress = Vec::with_capacity(self.scroll_timelines.len());
        for timeline in &self.scroll_timelines {
            let progress = timeline.progress;
            if !timeline.holds(progress) {
                return FfiClockTickOutcome::NeedsMain;
            }
            samples = samples.with_percentage(timeline.identity, progress);
            scroll_progress.push((timeline.identity, progress));
        }
        *self
            .published
            .scroll_progress
            .lock()
            .expect("clock publication scroll progress") = scroll_progress;
        let mut outcome = FfiClockTickOutcome::Presented;
        self.repaints.clear();
        self.tick_started_fresh = entries.is_empty();
        let mut presentable = true;
        for target in &self.targets {
            // A tick samples over the record the target's box holds: the host's, or the one an earlier
            // tick installed ahead of it. A target without a box, or whose box holds no style, is the host's.
            let row = arena.bound_row(target.style_node);
            let Some(style_record) = (!row.is_invalid())
                .then(|| arena.node_style_record(row))
                .and_then(NonZeroU64::new)
            else {
                outcome = FfiClockTickOutcome::NeedsMain;
                presentable = false;
                continue;
            };
            // SAFETY: As above; the borrow ends with the call.
            let sampled = unsafe {
                sample_installed_record_for_clock_tick(
                    &mut *engine,
                    target.style_node,
                    style_record.get(),
                    arena_handle,
                    &samples,
                )
            };
            let Some(sample) = sampled else {
                outcome = FfiClockTickOutcome::NeedsMain;
                presentable = false;
                entries.push(ClockTickEntry {
                    style_node: target.style_node,
                    style_record_before: style_record,
                    sample: FfiRowSampledInPass::absent(),
                    installed_in_arena: false,
                });
                continue;
            };
            // The sample moved nothing the record composed.
            if sample.style_record == style_record.get() {
                continue;
            }
            let level = sample.invalidation.invalidation & 0x3;
            let needs_layout_tree_rebuild = level >= 3;
            let needs_relayout = level >= 2;
            let installed_in_arena = !needs_layout_tree_rebuild
                && arena.install_animation_sample(target.style_node, sample.style_record, needs_relayout);
            presentable &= installed_in_arena
                && render_side_shows(&sample, || {
                    !target.has_pseudo_element_style_outside_box && box_holds_only_text(arena, row)
                });
            if level >= 1 && installed_in_arena {
                let affects_hit_testing =
                    sample.invalidation.invalidation & FfiStyleInvalidationField::AffectsHitTesting as u32 != 0;
                self.repaints.push((row, affects_hit_testing));
            }
            // Ticks the host has not adopted yet fold into one entry per target, over the record the
            // host holds: the arena's log keeps only the last record too.
            if let Some(entry) = entries.iter_mut().find(|entry| {
                entry.style_node == target.style_node
                    && entry.sample.style_record == style_record.get()
                    && entry.installed_in_arena
                    && installed_in_arena
            }) {
                entry.sample = folded_sample(&entry.sample, sample);
                continue;
            }
            entries.push(ClockTickEntry {
                style_node: target.style_node,
                style_record_before: style_record,
                sample,
                installed_in_arena,
            });
        }
        self.presentable = presentable;
        outcome
    }

    /// Shows what the tick installed and laid out: repaints the rows its samples repaint, records
    /// the display list again and has the host present it.
    ///
    /// # Safety
    ///
    /// As for [`Self::lay_out`].
    unsafe fn present(&self, state: *mut ArenaHandle) -> FfiClockPresent {
        if !self.presentable {
            return FfiClockPresent::Declined;
        }
        let Some(present) = PRESENT.get() else {
            return FfiClockPresent::Declined;
        };
        let arena_handle = state.cast::<c_void>();
        // SAFETY: The caller holds the arena.
        let arena = unsafe { &*state }.arena();
        {
            use crate::painting::record::damage::PaintDamage;
            for &(row, affects_hit_testing) in &self.repaints {
                if row.is_invalid() {
                    continue;
                }
                let damage = if affects_hit_testing {
                    PaintDamage::ALL_PRODUCERS
                } else {
                    PaintDamage::ALL_DRAW
                };
                arena.push_paint_damage_for_repaint(row, damage);
            }
        }
        // SAFETY: As above.
        if !unsafe { crate::painting::ffi::record_for_clock_tick(state) } {
            return FfiClockPresent::Declined;
        }
        let presented = present(arena_handle);
        if presented != FfiClockPresent::Presented {
            // Nothing presents the recording; the main thread records the frame again.
            arena.recording().discard_pending_recording();
        }
        presented
    }
}

/// Whether what `sample` changes is all the render side shows without the main thread: its own
/// row's style, layout and paint. What moves descendants' styles, visual contexts, stacking
/// contexts or scroll snapping, the main thread derives, and it loads the images a sample swaps in.
/// Inherited properties move the styles of descendants only where `box_holds_only_text` says no:
/// text lays out with its parent's style, and the element has no pseudo-element style outside its
/// box to inherit them.
fn render_side_shows(sample: &FfiRowSampledInPass, box_holds_only_text: impl FnOnce() -> bool) -> bool {
    use FfiStyleInvalidationField as Field;
    let invalidation = sample.invalidation.invalidation;
    let visual_context = (invalidation >> Field::VisualContextShift as u32) & Field::LevelMask as u32;
    let inherited_groups = (invalidation >> Field::InheritedGroupsShift as u32) & Field::InheritedGroupsMask as u32;
    let main_only = Field::RebuildStackingContext as u32
        | Field::ResnapScrollContainer as u32
        | Field::RecomputeDescendants as u32
        | Field::RepaintTextDecorations as u32
        | Field::NonInheritedInheritanceSource as u32
        | Field::RepaintSelection as u32;
    visual_context == 0
        && invalidation & main_only == 0
        && !sample.invalidation.requires_base_style_recomputation
        && !sample.invalidation.requires_style_resource_update
        && !sample.custom_property_environment_moved
        && sample.custom_property_reactions == 0
        && sample.keyframes_inherited_non_inherited_style_groups == 0
        && (inherited_groups == 0 || box_holds_only_text())
}

/// Whether every child of the box in `row` is text, which has no style of its own.
fn box_holds_only_text(arena: &LayoutNodeArena, row: NodeSlotId) -> bool {
    if !arena.slot_is_live(row) {
        return false;
    }
    let mut child = arena.data(row).first_child.get();
    while !child.is_invalid() {
        let data = arena.data(child);
        if !crate::layout::node_facts::kind_is_text(data.kind.get()) {
            return false;
        }
        child = data.next_sibling.get();
    }
    true
}

/// One sample for what `earlier` and `later`, taken over the record `earlier` installed, did: the
/// record of `later`, and what either invalidated.
fn folded_sample(earlier: &FfiRowSampledInPass, later: FfiRowSampledInPass) -> FfiRowSampledInPass {
    let level = (earlier.invalidation.invalidation & 0x3).max(later.invalidation.invalidation & 0x3);
    let flags = (earlier.invalidation.invalidation | later.invalidation.invalidation) & !0x3;
    let later_moved_environment = later.custom_property_environment_moved;
    let mut folded = later;
    folded.invalidation.invalidation = flags | level;
    folded.invalidation.changed_non_inherited_style_groups |= earlier.invalidation.changed_non_inherited_style_groups;
    folded.invalidation.requires_base_style_recomputation |= earlier.invalidation.requires_base_style_recomputation;
    folded.invalidation.requires_layout_node_style_application |=
        earlier.invalidation.requires_layout_node_style_application;
    folded.invalidation.requires_style_resource_update |= earlier.invalidation.requires_style_resource_update;
    folded.substitution_marks |= earlier.substitution_marks;
    folded.keyframes_inherited_non_inherited_style_groups |= earlier.keyframes_inherited_non_inherited_style_groups;
    folded.uses_tree_counting_function |= earlier.uses_tree_counting_function;
    folded.custom_property_reactions |= earlier.custom_property_reactions;
    folded.rebuilt_every_group |= earlier.rebuilt_every_group;
    if !later_moved_environment && earlier.custom_property_environment_moved {
        folded.custom_property_environment_moved = true;
        folded.custom_property_environment = earlier.custom_property_environment;
        folded.custom_property_store = earlier.custom_property_store;
        folded.custom_property_environment_named = earlier.custom_property_environment_named;
    }
    folded
}

/// On the main thread: what the clocks of a document publish, and whether one runs.
struct MainSideClock {
    published: Arc<ClockPublication>,
    running: bool,
}

thread_local! {
    // On the main thread, the clocks of each document it started a clock for, until the document goes.
    static CLOCKS: RefCell<HashMap<DocumentId, MainSideClock>> = RefCell::new(HashMap::new());
    // On the main thread, the entries of the adoption it took last that the host has yet to adopt.
    static ADOPTING: RefCell<std::collections::VecDeque<ClockTickEntry>> = RefCell::default();
}

/// The document whose layout arena is `arena`.
///
/// # Safety
///
/// `arena` must be the live layout arena of a document on the main thread.
unsafe fn document_of(arena: *mut c_void) -> DocumentId {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: Guaranteed by the caller.
    unsafe { crate::layout::ArenaHandle::document_of(arena) }
}

/// On the main thread: what the clock of the document whose layout arena is `arena` publishes, if
/// the main thread started it and has not stopped it.
fn publication_of(arena: *mut c_void) -> Option<Arc<ClockPublication>> {
    main_side_clock_of(arena, |clock| clock.running.then(|| Arc::clone(&clock.published))).flatten()
}

/// On the main thread: what the clocks of the document whose layout arena is `arena` publish, whether
/// one runs or not.
fn samples_publication_of(arena: *mut c_void) -> Option<Arc<ClockPublication>> {
    main_side_clock_of(arena, |clock| Arc::clone(&clock.published))
}

fn main_side_clock_of<R>(arena: *mut c_void, operation: impl FnOnce(&mut MainSideClock) -> R) -> Option<R> {
    // SAFETY: Every caller passes the live arena of a document on the main thread.
    let document = unsafe { document_of(arena) };
    CLOCKS.with_borrow_mut(|clocks| clocks.get_mut(&document).map(operation))
}

/// Sends the owner `message` about the clock of a document.
fn send(message: ClockMessage) {
    crate::render_owner::send(crate::render_owner::ToOwner::Clock(message));
}

/// On the main thread: the document whose render state the owner dropped had its clock dropped
/// with it.
pub(crate) fn document_destroyed(document: DocumentId) {
    CLOCKS.with_borrow_mut(|clocks| clocks.remove(&document));
}

// The ticks the host adopted that installed a sample.
static CLOCK_TICKS_PRESENTED: AtomicU64 = AtomicU64::new(0);

/// Starts the clock of the document whose layout arena is `arena`, in place of the one it had: it
/// ticks the document timeline, which the style engine knows as `timeline_identity`, reading zero
/// at `timeline_zero` and now at `time`, until `deadline` (timeline times, ms). The render clock
/// ticks it at the display ticks of the compositor context `context`, unless that is 0.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_start(
    arena: *mut c_void,
    context: u64,
    timeline_identity: u32,
    timeline_zero: f64,
    time: f64,
    deadline: f64,
) {
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    let published = CLOCKS.with_borrow_mut(|clocks| {
        let clock = clocks.entry(document).or_insert_with(|| MainSideClock {
            published: Arc::new(ClockPublication::new(time)),
            running: false,
        });
        clock.running = true;
        let published = &clock.published;
        published.time.store(time.to_bits(), Ordering::Release);
        published.presented_since_adoption.store(false, Ordering::Release);
        Arc::clone(published)
    });
    send(ClockMessage::Start {
        document,
        clock: Box::new(DocumentClock {
            context,
            main_thread: std::thread::current().id(),
            timeline_identity,
            timeline_zero,
            deadline,
            paused: false,
            needs_main: false,
            stopped: false,
            targets: Vec::new(),
            scroll_timelines: Vec::new(),
            repaints: Vec::new(),
            presentable: false,
            tick_started_fresh: false,
            published,
        }),
    });
}

/// Sets the elements the clock of the document whose layout arena is `arena` ticks, with the
/// records they hold now, and whether each has a pseudo-element style that inherits from it outside
/// its box.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread, and `style_nodes`,
/// `style_records` and `pseudo_element_styles_outside_box` point at `count` values each.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_set_targets(
    arena: *mut c_void,
    style_nodes: *const u32,
    pseudo_element_styles_outside_box: *const bool,
    count: usize,
) {
    if publication_of(arena).is_none() {
        return;
    }
    let mut targets = Vec::with_capacity(count);
    for index in 0..count {
        // SAFETY: Guaranteed by the caller.
        let (node, outside_box) = unsafe { (*style_nodes.add(index), *pseudo_element_styles_outside_box.add(index)) };
        if let Some(style_node) = StyleNodeID::from_raw(node) {
            targets.push(ClockTarget {
                style_node,
                has_pseudo_element_style_outside_box: outside_box,
            });
        }
    }
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    send(ClockMessage::Targets { document, targets });
}

/// Sets the scroll progress timelines whose animations the clock of the document whose layout arena
/// is `arena` ticks.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread, and `timelines` points at
/// `count` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_set_scroll_timelines(
    arena: *mut c_void,
    timelines: *const FfiClockScrollTimeline,
    count: usize,
) {
    let Some(published) = publication_of(arena) else {
        return;
    };
    let timelines = match count {
        0 => Vec::new(),
        // SAFETY: Guaranteed by the caller.
        _ => unsafe { std::slice::from_raw_parts(timelines, count) }.to_vec(),
    };
    // Until a tick samples them, the timelines are where the host holds them.
    *published
        .scroll_progress
        .lock()
        .expect("clock publication scroll progress") = timelines
        .iter()
        .map(|timeline| (timeline.identity, timeline.progress))
        .collect();
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    send(ClockMessage::ScrollTimelines { document, timelines });
}

/// The progress (percent) at which the last tick of the clock of the document whose layout arena is
/// `arena` sampled the scroll progress timeline `identity`, or the host held it at the start, or
/// NaN.
#[unsafe(no_mangle)]
pub extern "C" fn rust_document_clock_scroll_progress(arena: *mut c_void, identity: u32) -> f64 {
    publication_of(arena).map_or(f64::NAN, |published| {
        published
            .scroll_progress
            .lock()
            .expect("clock publication scroll progress")
            .iter()
            .find(|(timeline, _)| *timeline == identity)
            .map_or(f64::NAN, |(_, progress)| *progress)
    })
}

/// Stops the clock of the document whose layout arena is `arena`, if it has one. What its ticks
/// left stays for the host to adopt, beside what a tick that runs before the owner learns of the
/// stop leaves.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_stop(arena: *mut c_void) {
    if main_side_clock_of(arena, |clock| std::mem::replace(&mut clock.running, false)) != Some(true) {
        return;
    }
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    send(ClockMessage::Stop { document });
}

/// Has the render clock leave the clock of the document whose layout arena is `arena` alone, or
/// tick it again.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_set_paused(arena: *mut c_void, paused: bool) {
    if publication_of(arena).is_none() {
        return;
    }
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    send(ClockMessage::Paused { document, paused });
}

/// Hands the clock of the document whose layout arena is `arena` the layout frame its render clock
/// ticks lay out in. A frame the ticks laid out in stays until the main thread takes it in.
pub(crate) fn set_clock_layout_frame(arena: *mut c_void, frame: ClockLayoutFrame) {
    if let Some(published) = publication_of(arena) {
        let mut current = published.layout_frame.lock().expect("clock publication layout frame");
        if !current.as_ref().is_some_and(ClockLayoutFrame::laid_out) {
            *current = Some(frame);
        }
    }
}

/// Takes the layout frame of the clock of the document whose layout arena is `arena` if its ticks
/// laid out in it.
pub(crate) fn take_laid_out_clock_layout_frame(arena: *mut c_void) -> Option<ClockLayoutFrame> {
    let published = publication_of(arena)?;
    let mut frame = published.layout_frame.lock().expect("clock publication layout frame");
    if !frame.as_ref().is_some_and(ClockLayoutFrame::laid_out) {
        return None;
    }
    frame.take()
}

pub(crate) fn clock_layout_frame_laid_out(arena: *mut c_void) -> bool {
    publication_of(arena).is_some_and(|published| {
        published
            .layout_frame
            .lock()
            .expect("clock publication layout frame")
            .as_ref()
            .is_some_and(ClockLayoutFrame::laid_out)
    })
}

/// Whether the main thread started the clock of the document whose layout arena is `arena` and has
/// not stopped it.
#[unsafe(no_mangle)]
pub extern "C" fn rust_document_clock_is_running(arena: *mut c_void) -> bool {
    publication_of(arena).is_some()
}

/// The timeline time of the last tick of the clock of the document whose layout arena is `arena`,
/// or NaN without a clock.
#[unsafe(no_mangle)]
pub extern "C" fn rust_document_clock_time(arena: *mut c_void) -> f64 {
    publication_of(arena).map_or(f64::NAN, |published| published.time())
}

/// Submits a tick of the clock of the document whose layout arena is `arena` at timeline time
/// `time`, which the owner runs as the stage `clock`, which owns the arena and holds the document's
/// style engine `engine`, lent to it, until the host takes it back. Returns false, having done
/// nothing, where the document has no clock or the stage thread takes no `clock` stage.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread, with nothing in flight
/// that owns it, and `engine` is its style engine, or null.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_submit_tick(
    engine: StyleEngineInputHandle,
    arena: *mut c_void,
    time: f64,
) -> bool {
    if !crate::stage_thread::submits() {
        return false;
    }
    let Some(published) = publication_of(arena) else {
        return false;
    };
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    // The host shows what this tick installs itself.
    published.presented_since_adoption.store(false, Ordering::Release);
    // The tick samples the engine, and takes it along.
    let engine = engine.home();
    let (loan, settlement) = (!engine.is_null())
        .then(|| engine.lend(Holder::LayoutPass, Owed::TakeBack))
        .unzip();
    let tick = Box::new(SubmittedTick {
        time,
        style_engine: loan,
        published,
        run: run_submitted_tick,
    });
    let taken_back = move || {
        if let Some(settlement) = settlement {
            settlement.settle();
        }
    };
    // SAFETY: The run reaches the arena and its engine only, which the `clock` stage owns until the
    // main thread takes it back, and every main-thread path to either joins it first.
    unsafe {
        crate::stage_thread::submit_clock_tick(
            arena,
            |ticket| crate::render_owner::ToOwner::Clock(ClockMessage::SubmittedTick { document, tick, ticket }),
            taken_back,
        );
    }
    true
}

/// How the last tick of the clock of the document whose layout arena is `arena` ended; `Stopped`
/// without a clock.
#[unsafe(no_mangle)]
pub extern "C" fn rust_document_clock_tick_outcome(arena: *mut c_void) -> FfiClockTickOutcome {
    publication_of(arena)
        .and_then(|published| *published.outcome.lock().expect("clock publication outcome"))
        .unwrap_or(FfiClockTickOutcome::Stopped)
}

/// Whether the render side presented every tick of the clock of the document whose layout arena is
/// `arena` the host has not adopted yet: adopting them repaints nothing.
pub(crate) fn presented_since_adoption(arena: *mut c_void) -> bool {
    samples_publication_of(arena).is_some_and(|published| published.presented_since_adoption.load(Ordering::Acquire))
}

/// As [`presented_since_adoption`].
#[unsafe(no_mangle)]
pub extern "C" fn rust_document_clock_presented_since_adoption(arena: *mut c_void) -> bool {
    presented_since_adoption(arena)
}

/// Counts a tick the host adopted that installed a sample.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_ticks_note_presented() {
    CLOCK_TICKS_PRESENTED.fetch_add(1, Ordering::Relaxed);
}

/// The ticks the host adopted that installed a sample, for tests.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_ticks_presented() -> u64 {
    CLOCK_TICKS_PRESENTED.load(Ordering::Relaxed)
}

/// What the host adopts of what the ticks of the clocks of a document left, all of it as one tick
/// left it: whether the ticks left samples, which `style_engine_clock_tick_take_entry` hands out
/// from now on, the time of that tick (NaN without a running clock), and the query snapshot of what
/// it laid out, or null, which the host owns (`query_snapshot_release`).
#[repr(C)]
pub struct FfiClockAdoption {
    pub has_samples: bool,
    pub time: f64,
    pub query_snapshot: *const c_void,
}

/// Takes what the ticks of the clocks of the document whose layout arena is `arena` and whose style
/// engine is `engine` left for the host to adopt. The query snapshot converts rects to viewport space
/// with `viewport`, the document's scroll state as it holds it now. Where the ticks left samples, a
/// frame in flight that holds the engine is taken in first.
///
/// # Safety
///
/// `viewport.device_scroll_offsets` addresses `viewport.device_scroll_offsets_len` points.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_take_adoption(
    engine: StyleEngineHandle,
    arena: *mut c_void,
    viewport: FfiQuerySnapshotViewport,
) -> FfiClockAdoption {
    let running = publication_of(arena).is_some();
    let Some(published) = samples_publication_of(arena) else {
        ADOPTING.with_borrow_mut(std::collections::VecDeque::clear);
        return FfiClockAdoption {
            has_samples: false,
            time: f64::NAN,
            query_snapshot: std::ptr::null(),
        };
    };
    // With the publication unlocked: taking a frame in can adopt a tick of its own.
    let left_samples = !published.adoption().entries.is_empty();
    let home = left_samples.then(|| engine.bring_home("rust_document_clock_take_adoption"));
    let mut adoption = published.adoption();
    let entries = home.map_or_else(Vec::new, |home| adoption.take_entries(home));
    let has_samples = !entries.is_empty();
    if let Some(rows) = adoption.rows.take() {
        // SAFETY: Every caller passes the live arena of a document on the main thread, which borrows no rows here.
        unsafe { crate::layout::HostTables::beside_frame(arena) }
            .adopted_rows
            .publish(rows);
    }
    ADOPTING.with_borrow_mut(|adopting| *adopting = entries.into());
    FfiClockAdoption {
        has_samples,
        time: if running { published.time() } else { f64::NAN },
        query_snapshot: adoption.query_snapshot.take().map_or(std::ptr::null(), |mut snapshot| {
            snapshot.convert_with(&viewport);
            crate::painting::query_snapshot::into_handle(snapshot)
        }),
    }
}

/// Takes the next entry of the adoption the host took last.
pub(crate) fn take_clock_tick_entry() -> Option<ClockTickEntry> {
    ADOPTING.with_borrow_mut(std::collections::VecDeque::pop_front)
}

/// Whether the clocks of the document whose layout arena is `arena` left something for the host to
/// adopt: samples its ticks installed, or the query snapshot of what a tick laid out.
#[unsafe(no_mangle)]
pub extern "C" fn rust_document_clock_left_adoption(arena: *mut c_void) -> bool {
    samples_publication_of(arena).is_some_and(|published| {
        let adoption = published.adoption();
        !adoption.entries.is_empty() || adoption.query_snapshot.is_some()
    })
}

/// Drops what the host did not adopt of the records the ticks of the clock of the document whose
/// layout arena is `arena` installed in the arena ahead of it, with their pins: the rows of elements
/// that left the document before the host could adopt their samples.
///
/// The owner drops them as an arena change
/// ([`crate::render_owner::ArenaChange::DropUnadoptedAnimationSamples`]).
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_drop_unadopted(arena: *mut c_void) {
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    crate::render_owner::send_arena_change(
        document,
        crate::render_owner::ArenaChange::DropUnadoptedAnimationSamples,
    );
}

/// A message about the clocks of documents, which the owner handles in order with every other
/// message it is sent ([`crate::render_owner::ToOwner::Clock`]).
pub(crate) enum ClockMessage {
    /// Starts the clock of `document`, in place of the one it had.
    Start {
        document: DocumentId,
        clock: Box<DocumentClock>,
    },
    /// Sets the elements the clock of `document` ticks.
    Targets {
        document: DocumentId,
        targets: Vec<ClockTarget>,
    },
    /// Sets the scroll progress timelines whose animations the clock of `document` ticks.
    ScrollTimelines {
        document: DocumentId,
        timelines: Vec<FfiClockScrollTimeline>,
    },
    /// Has the render clock leave the clock of `document` alone, or tick it again.
    Paused { document: DocumentId, paused: bool },
    /// Stops the clock of `document`.
    Stop { document: DocumentId },
    /// A display tick for the compositor context `context`, at the time its slot holds when it runs, which `run`
    /// runs.
    DisplayTick {
        context: u64,
        slot: Arc<ClockSlot>,
        run: RunDisplayTick,
    },
    /// A display tick a test injected for the compositor context `context`, which `run` runs.
    InjectedTick {
        context: u64,
        frame_time_nanoseconds: i64,
        run: RunDisplayTick,
    },
    /// The tick a rendering update of `document` submitted, as the run `ticket`.
    SubmittedTick {
        document: DocumentId,
        tick: Box<SubmittedTick>,
        ticket: crate::stage_thread::SubmittedRunTicket,
    },
}

impl ClockMessage {
    /// The document the message is about; none for what the render clock sends.
    pub(crate) fn document(&self) -> DocumentId {
        match self {
            Self::Start { document, .. }
            | Self::Targets { document, .. }
            | Self::ScrollTimelines { document, .. }
            | Self::Paused { document, .. }
            | Self::Stop { document }
            | Self::SubmittedTick { document, .. } => *document,
            Self::DisplayTick { .. } | Self::InjectedTick { .. } => DocumentId::default(),
        }
    }
}

/// How the owner runs a display tick for a compositor context at a frame time. The owner reaches what a tick runs (the
/// sampling, the layout, the recording) only through the ticks it is sent, so what reaches the owner without it (the
/// unit tests' stage threads) links without it.
pub(crate) type RunDisplayTick = fn(&crate::render_owner::Owner, u64, i64);

/// A tick a rendering update submitted: at timeline time `time`, with the style engine the update
/// lent it, publishing to `published`, which the main thread reads once it has taken the tick back.
/// `run` runs it, as [`RunDisplayTick`] does a display tick.
pub(crate) struct SubmittedTick {
    time: f64,
    style_engine: Option<StyleEngineLoan>,
    published: Arc<ClockPublication>,
    run: fn(&crate::render_owner::Owner, DocumentId, SubmittedTick),
}

/// Handles `message` on the owner.
pub(crate) fn handle_on_owner(owner: &crate::render_owner::Owner, message: ClockMessage) {
    match message {
        ClockMessage::Start { document, clock } => {
            crate::render_owner::with_clock_slot(document, |slot| *slot = Some(*clock));
        }
        ClockMessage::Targets { document, targets } => {
            crate::render_owner::with_clock_slot(document, |slot| {
                if let Some(clock) = slot {
                    clock.targets = targets;
                }
            });
        }
        ClockMessage::ScrollTimelines { document, timelines } => {
            crate::render_owner::with_clock_slot(document, |slot| {
                if let Some(clock) = slot {
                    clock.scroll_timelines = timelines;
                }
            });
        }
        ClockMessage::Paused { document, paused } => {
            crate::render_owner::with_clock_slot(document, |slot| {
                if let Some(clock) = slot {
                    clock.paused = paused;
                }
            });
        }
        ClockMessage::Stop { document } => {
            crate::render_owner::with_clock_slot(document, |slot| {
                if let Some(clock) = slot {
                    clock.stopped = true;
                }
            });
        }
        ClockMessage::DisplayTick { context, slot, run } => {
            // A read-modify-write, which a later tick's post orders against: either that post finds the
            // slot free and posts a tick of its own, or the time it stored is the one read here.
            slot.queued.swap(false, Ordering::AcqRel);
            let frame_time_nanoseconds = slot.frame_time_nanoseconds.load(Ordering::Acquire);
            run(owner, context, frame_time_nanoseconds);
        }
        ClockMessage::InjectedTick {
            context,
            frame_time_nanoseconds,
            run,
        } => {
            run(owner, context, frame_time_nanoseconds);
            adopt_on_main(true);
        }
        ClockMessage::SubmittedTick { document, tick, ticket } => ticket.run(|| (tick.run)(owner, document, *tick)),
    }
}

/// Takes the clock of `document` out of its render state, with the arena it ticks, for a tick that
/// may reach other documents' state while it runs. [`put_clock_back`] puts it back.
fn take_clock(owner: &crate::render_owner::Owner, document: DocumentId) -> Option<(DocumentClock, *mut ArenaHandle)> {
    crate::render_owner::with_clock(owner, document, |slot, arena| slot.take().map(|clock| (clock, arena))).flatten()
}

fn put_clock_back(document: DocumentId, clock: DocumentClock) {
    crate::render_owner::with_clock_slot(document, |slot| {
        debug_assert!(slot.is_none(), "nothing starts a clock while its tick runs");
        slot.get_or_insert(clock);
    });
}

/// Runs the tick a rendering update of `document` submitted, on the owner inside its run.
fn run_submitted_tick(owner: &crate::render_owner::Owner, document: DocumentId, tick: SubmittedTick) {
    let SubmittedTick {
        time,
        style_engine,
        published,
        run: _,
    } = tick;
    // Taking the clock applies the arena's pending changes, which may reach the engine: inside the loan.
    let run = |engine: Option<&mut StyleEngine>| {
        let Some((mut clock, arena)) = take_clock(owner, document) else {
            published.set_outcome(FfiClockTickOutcome::Stopped);
            return;
        };
        if clock.stopped {
            published.set_outcome(FfiClockTickOutcome::Stopped);
            put_clock_back(document, clock);
            return;
        }
        debug_assert!(
            Arc::ptr_eq(&clock.published, &published),
            "a submitted tick publishes to the clock it was submitted for"
        );
        // SAFETY: The run owns the arena and its engine until the main thread takes it back.
        unsafe { clock.run_tick(&mut published.adoption().entries, arena, engine, time) };
        put_clock_back(document, clock);
    };
    match style_engine {
        Some(mut loan) => loan.lend_to_this_thread(|engine| run(Some(engine))),
        None => run(None),
    }
}

/// Runs the display tick at `frame_time_nanoseconds` for the clock that ticks at the compositor
/// context `context`, on the owner, whatever the main thread is doing.
fn run_display_tick(owner: &crate::render_owner::Owner, context: u64, frame_time_nanoseconds: i64) {
    count(&COUNTERS.ticks_run);
    let Some(document) = crate::render_owner::document_with_clock_at(context) else {
        count(&COUNTERS.ticks_dropped_without_clock);
        return;
    };
    // Taking the clock applies the arena's pending changes, which may reach the engine.
    let taken = match crate::render_owner::style_engine_of(document) {
        // SAFETY: The owner holds the document's render state, and the engine with it.
        Some(engine) if !engine.is_null() => unsafe {
            engine.reach_on_owner_beside_main(owner, |_| take_clock(owner, document))
        },
        _ => take_clock(owner, document),
    };
    let Some((mut clock, arena)) = taken else {
        count(&COUNTERS.ticks_dropped_without_clock);
        return;
    };
    run_display_tick_on(owner, &mut clock, arena, context, frame_time_nanoseconds);
    put_clock_back(document, clock);
}

/// Runs a display tick of `clock`, whose document's arena is `arena`.
fn run_display_tick_on(
    owner: &crate::render_owner::Owner,
    clock: &mut DocumentClock,
    arena: *mut ArenaHandle,
    context: u64,
    frame_time_nanoseconds: i64,
) {
    if clock.paused {
        count(&COUNTERS.ticks_dropped_paused);
        return;
    }
    if clock.needs_main {
        count(&COUNTERS.ticks_dropped_needing_main);
        return;
    }
    let published = Arc::clone(&clock.published);
    let time = clock.timeline_time_at(frame_time_nanoseconds as f64 / 1.0e6);
    if time.partial_cmp(&published.time()) != Some(std::cmp::Ordering::Greater) {
        count(&COUNTERS.ticks_dropped_stale);
        return;
    }
    // The main thread takes what the tick leaves, and the layout frame it lays out in, once it is over.
    let mut adoption = published.adoption();
    let mut layout_frame = published.layout_frame.lock().expect("clock publication layout frame");
    // The main thread hands the clock the next layout frame as it takes the last one in.
    let Some(frame) = layout_frame.as_mut() else {
        return;
    };
    let mut tick = None;
    // What the tick publishes of the arena's rows, the main thread reads once it adopts the tick, not beside its task.
    // SAFETY: The owner holds the document's render state.
    unsafe { &mut *arena }.arena_mut().hold_published_rows();
    crate::stage_thread::run_detached_for(clock.main_thread, arena as usize, || {
        let mut run_tick = |engine: Option<&mut StyleEngine>| {
            let entries = &mut adoption.entries;
            // SAFETY: The owner holds the document's render state: its arena, and the engine the arena links.
            let outcome = unsafe { clock.run_tick(entries, arena, engine, time) };
            if outcome != FfiClockTickOutcome::Presented {
                return (outcome, false, false);
            }
            // A sample the arena did not take, the host installs over the record the target held
            // before: only its entry keeps that record alive, and no later tick may sample over it.
            if entries.iter().any(|entry| !entry.installed_in_arena) {
                return (FfiClockTickOutcome::NeedsMain, false, false);
            }
            // What the samples left, the tick lays out in the frame the main thread handed the clock.
            // SAFETY: As above.
            if !unsafe { frame.run_round(owner) } {
                return (FfiClockTickOutcome::NeedsMain, false, false);
            }
            let laid_out = frame.laid_out();
            // A round that moved a box that owns a clip, a transform or a scroll frame moved the
            // visual contexts, which the compositor has from the main thread's frames.
            // SAFETY: As above.
            if laid_out && !unsafe { crate::painting::ffi::settle_visual_contexts_for_clock_tick(arena) } {
                count(&COUNTERS.ticks_moving_visual_contexts);
                return (FfiClockTickOutcome::NeedsMain, laid_out, false);
            }
            // Reads answer from what the tick left once the host adopted it.
            // SAFETY: As above.
            adoption.query_snapshot = Some(
                unsafe { &mut *arena }
                    .arena_mut()
                    .publish_query_snapshot(&CLOCK_TICK_QUERY_SNAPSHOT_VIEWPORT),
            );
            // A tick that moved nothing shows nothing new.
            let moved_nothing = !laid_out && clock.repaints.is_empty();
            if moved_nothing {
                return (outcome, laid_out, false);
            }
            // SAFETY: As above.
            match unsafe { clock.present(arena) } {
                FfiClockPresent::Presented => (outcome, laid_out, true),
                FfiClockPresent::MainPresents => (outcome, laid_out, false),
                FfiClockPresent::Declined => (FfiClockTickOutcome::NeedsMain, laid_out, false),
            }
        };
        // The whole tick reaches the engine as the owner, beside the main thread: its layout round and its recording
        // too.
        // SAFETY: The owner holds the document's render state: its arena, and the engine the arena links.
        let engine = unsafe { &*arena }.arena().style_engine_handle();
        let reach_and_run_tick = || {
            if engine.is_null() {
                return run_tick(None);
            }
            // SAFETY: As above.
            unsafe { engine.reach_on_owner_beside_main(owner, |engine| run_tick(Some(engine))) }
        };
        tick = Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(
            reach_and_run_tick,
        )));
    });
    // SAFETY: As above.
    if let Some(rows) = unsafe { &mut *arena }.arena_mut().take_held_rows() {
        adoption.rows = Some(rows);
    }
    let (outcome, laid_out, presented_frame) = match tick {
        Some(Ok(ran)) => ran,
        // A tick has nobody to hand a panic to: it is dropped, and the clock stops at the host, whose next rendering
        // update takes it.
        _ => {
            debug_assert!(false, "a display tick panicked");
            (FfiClockTickOutcome::NeedsMain, false, false)
        }
    };
    let presented = outcome == FfiClockTickOutcome::Presented;
    if !presented {
        // The clock stops at the host, which adopts what the ticks left first.
        published.set_outcome(outcome);
        clock.needs_main = true;
    }
    // What the main thread presents, it repaints as it adopts it.
    let moved = laid_out || !clock.repaints.is_empty();
    let shown_on_render_side = presented && (presented_frame || !moved);
    if clock.tick_started_fresh {
        published
            .presented_since_adoption
            .store(shown_on_render_side, Ordering::Release);
    } else if !shown_on_render_side {
        published.presented_since_adoption.store(false, Ordering::Release);
    }
    if !adoption.entries.is_empty() {
        TICKS_TO_ADOPT.store(true, Ordering::Release);
    }
    let rounds_owed = layout_frame.as_ref().map_or(0, ClockLayoutFrame::rounds);
    drop(layout_frame);
    drop(adoption);
    if !presented {
        count(&COUNTERS.ticks_needing_main);
        if let Some(needs_main) = NEEDS_MAIN.get() {
            needs_main(context);
        }
        return;
    }
    count(&COUNTERS.ticks_installed);
    if laid_out {
        count(&COUNTERS.ticks_laid_out);
    }
    // A tick that moved nothing presented no frame, and the main thread presents what one moved while it keeps up.
    if presented_frame {
        count(&COUNTERS.ticks_presented);
    } else if moved {
        count(&COUNTERS.ticks_presented_by_main);
    }
    // What the rounds owe the document piles up in the frame until the main thread takes it in: one
    // that renders nothing for long is asked to take it in every so often, and pays for a few rounds
    // at a time.
    if laid_out && rounds_owed >= MAX_CLOCK_ROUNDS_OWED {
        count(&COUNTERS.ticks_asking_main_to_adopt);
        adopt_on_main(false);
    }
}

/// What a display tick's query snapshot converts rects to viewport space with until the main thread adopts it, which
/// converts them with the scroll state it holds (see [`rust_document_clock_take_adoption`]): the tick keeps none.
const CLOCK_TICK_QUERY_SNAPSHOT_VIEWPORT: FfiQuerySnapshotViewport = FfiQuerySnapshotViewport {
    has_committed_viewport_box: true,
    visual_contexts_are_up_to_date: false,
    viewport_scroll_offset_is_zero: false,
    device_scroll_offsets: std::ptr::null(),
    device_scroll_offsets_len: 0,
    device_pixels_per_css_pixel: 1.0,
};

// Whether a display tick left something for the main thread to adopt since it last took it.
static TICKS_TO_ADOPT: AtomicBool = AtomicBool::new(false);

/// Whether the render clock's ticks left something for the documents to adopt since the main thread
/// last asked, which they adopt before anything else reaches them.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_take_ticks_to_adopt() -> bool {
    TICKS_TO_ADOPT.swap(false, Ordering::AcqRel)
}

/// Has the main thread adopt what the render clock's ticks left. Runs on the Rendering thread.
static ADOPT_ON_MAIN: OnceLock<extern "C" fn(bool)> = OnceLock::new();

/// Has the render clock call `adopt_on_main(injected_tick_ran)` on the Rendering thread where the
/// main thread should adopt what the ticks left without waiting for something else to run on it: a
/// tick a test injected ran, or the ticks laid out many rounds for it to take in. The first one set
/// stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_set_adopt_on_main(adopt: extern "C" fn(bool)) {
    let _ = ADOPT_ON_MAIN.set(adopt);
}

fn adopt_on_main(injected_tick_ran: bool) {
    if let Some(adopt) = ADOPT_ON_MAIN.get() {
        adopt(injected_tick_ran);
    }
}

/// Hands the owner a display tick at `frame_time_nanoseconds` (monotonic time) for the compositor
/// context `context`, as the render clock hands it one, for a test that drives the ticks itself.
/// Once the tick has run, the render clock has the main thread adopt what it left (see
/// [`rust_render_clock_set_adopt_on_main`]). Returns false where there is no owner to tick it.
///
/// # Safety
///
/// `sender` came from [`rust_render_clock_sender_create`], on the thread that owns it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_render_clock_inject_tick(
    sender: *mut ClockSender,
    context: u64,
    frame_time_nanoseconds: i64,
) -> bool {
    assert!(!sender.is_null(), "render clock sender is null");
    crate::stage_thread::send_to_owner(crate::render_owner::ToOwner::Clock(ClockMessage::InjectedTick {
        context,
        frame_time_nanoseconds,
        run: run_display_tick,
    }))
    .is_ok()
}

/// Where the render clock's display ticks wait for the owner: one per compositor context. Ticks
/// that arrive while one waits fold into it, with the latest time.
pub(crate) struct ClockSlot {
    frame_time_nanoseconds: AtomicI64,
    queued: AtomicBool,
}

/// How many rounds the ticks lay out in one layout frame before they ask the main thread to take it
/// in: about two seconds of display ticks.
const MAX_CLOCK_ROUNDS_OWED: u32 = 120;

/// How many slots a render clock keeps before it lets go of those no tick waits in.
const MAX_IDLE_CLOCK_SLOTS: usize = 16;

/// The render clock's way to the owner. Owned by the render clock thread.
pub struct ClockSender {
    slots: HashMap<u64, Arc<ClockSlot>>,
}

/// What the render clock did with the display ticks it was handed, for tests.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct FfiRenderClockCounters {
    /// Display ticks posted to the stage thread.
    pub ticks_posted: u64,
    /// Display ticks that folded into one already waiting for the stage thread.
    pub ticks_folded: u64,
    /// Ticks the stage thread ran.
    pub ticks_run: u64,
    /// Ticks the main thread presented, as it kept up with the display.
    pub ticks_presented_by_main: u64,
    /// Ticks for a context no document's clock ticks at.
    pub ticks_dropped_without_clock: u64,
    /// Ticks for a clock the main thread moved what it ticks of.
    pub ticks_dropped_paused: u64,
    /// Ticks for a clock an earlier tick ended, which waits for the main thread.
    pub ticks_dropped_needing_main: u64,
    /// Ticks at a time no later than the clock's last.
    pub ticks_dropped_stale: u64,
    /// Ticks that installed their samples for the main thread to adopt.
    pub ticks_installed: u64,
    /// Ticks after which a layout frame on the render side held what the main thread takes in.
    pub ticks_laid_out: u64,
    /// Ticks the render side presented, without the main thread.
    pub ticks_presented: u64,
    /// Ticks that ended their clock: past its deadline, or with a sample only the main thread takes.
    pub ticks_needing_main: u64,
    /// Ticks whose layout moved the visual contexts, which ended their clock.
    pub ticks_moving_visual_contexts: u64,
    /// Ticks that asked the main thread to take in the rounds they laid out.
    pub ticks_asking_main_to_adopt: u64,
}

#[derive(Default)]
struct RenderClockCounters {
    ticks_posted: AtomicU64,
    ticks_folded: AtomicU64,
    ticks_run: AtomicU64,
    ticks_presented_by_main: AtomicU64,
    ticks_dropped_without_clock: AtomicU64,
    ticks_dropped_paused: AtomicU64,
    ticks_dropped_needing_main: AtomicU64,
    ticks_dropped_stale: AtomicU64,
    ticks_installed: AtomicU64,
    ticks_laid_out: AtomicU64,
    ticks_presented: AtomicU64,
    ticks_needing_main: AtomicU64,
    ticks_moving_visual_contexts: AtomicU64,
    ticks_asking_main_to_adopt: AtomicU64,
}

static COUNTERS: RenderClockCounters = RenderClockCounters {
    ticks_posted: AtomicU64::new(0),
    ticks_folded: AtomicU64::new(0),
    ticks_run: AtomicU64::new(0),
    ticks_presented_by_main: AtomicU64::new(0),
    ticks_dropped_without_clock: AtomicU64::new(0),
    ticks_dropped_paused: AtomicU64::new(0),
    ticks_dropped_needing_main: AtomicU64::new(0),
    ticks_dropped_stale: AtomicU64::new(0),
    ticks_installed: AtomicU64::new(0),
    ticks_laid_out: AtomicU64::new(0),
    ticks_presented: AtomicU64::new(0),
    ticks_needing_main: AtomicU64::new(0),
    ticks_moving_visual_contexts: AtomicU64::new(0),
    ticks_asking_main_to_adopt: AtomicU64::new(0),
};

fn count(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// Presents the display list a render clock tick recorded for the document of an arena: publishes
/// it and hands the frame to the compositor. Runs on the Rendering thread.
static PRESENT: OnceLock<extern "C" fn(*mut c_void) -> FfiClockPresent> = OnceLock::new();

/// Has the render clock call `present(arena)` to present what a tick recorded. The first one set
/// stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_set_present(present: extern "C" fn(*mut c_void) -> FfiClockPresent) {
    let _ = PRESENT.set(present);
}

/// What the main thread does for a clock a render clock tick ended. Runs on the Rendering thread.
static NEEDS_MAIN: OnceLock<extern "C" fn(u64)> = OnceLock::new();

/// Has the render clock call `needs_main(context)` on the Rendering thread where a tick ended the
/// clock that ticks at `context`: the main thread's rendering update takes over there. The first
/// one set stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_set_needs_main(needs_main: extern "C" fn(u64)) {
    let _ = NEEDS_MAIN.set(needs_main);
}

/// A render clock's way to the owner. The render clock thread owns it, and destroys it with
/// [`rust_render_clock_sender_destroy`].
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_sender_create() -> *mut ClockSender {
    Box::into_raw(Box::new(ClockSender { slots: HashMap::new() }))
}

/// # Safety
///
/// `sender` is null or came from [`rust_render_clock_sender_create`], on the thread that owns it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_render_clock_sender_destroy(sender: *mut ClockSender) {
    if !sender.is_null() {
        // SAFETY: Guaranteed by the caller.
        drop(unsafe { Box::from_raw(sender) });
    }
}

/// Hands the display tick at `frame_time_nanoseconds` (monotonic time) for the compositor context
/// `context` to the owner, which ticks the clock of that context with it. Where a tick for the
/// context is still waiting there, it takes this time instead. Returns false where there is no
/// owner, which there only is not when the process is going.
///
/// # Safety
///
/// `sender` came from [`rust_render_clock_sender_create`], on the thread that owns it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_render_clock_post_tick(
    sender: *mut ClockSender,
    context: u64,
    frame_time_nanoseconds: i64,
) -> bool {
    // SAFETY: Guaranteed by the caller.
    let sender = unsafe { &mut *sender };
    // A context whose tick no message holds any more gets a slot again if it ticks again: the slots
    // of contexts that went away go.
    if sender.slots.len() >= MAX_IDLE_CLOCK_SLOTS && !sender.slots.contains_key(&context) {
        sender.slots.retain(|_, slot| Arc::strong_count(slot) > 1);
    }
    let slot = sender.slots.entry(context).or_insert_with(|| {
        Arc::new(ClockSlot {
            frame_time_nanoseconds: AtomicI64::new(0),
            queued: AtomicBool::new(false),
        })
    });
    slot.frame_time_nanoseconds
        .store(frame_time_nanoseconds, Ordering::Release);
    if slot.queued.swap(true, Ordering::AcqRel) {
        count(&COUNTERS.ticks_folded);
        return true;
    }
    count(&COUNTERS.ticks_posted);
    let slot = Arc::clone(slot);
    crate::stage_thread::send_to_owner(crate::render_owner::ToOwner::Clock(ClockMessage::DisplayTick {
        context,
        slot,
        run: run_display_tick,
    }))
    .is_ok()
}

/// What the render clock did with the display ticks it was handed.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_counters() -> FfiRenderClockCounters {
    let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
    FfiRenderClockCounters {
        ticks_posted: load(&COUNTERS.ticks_posted),
        ticks_folded: load(&COUNTERS.ticks_folded),
        ticks_run: load(&COUNTERS.ticks_run),
        ticks_presented_by_main: load(&COUNTERS.ticks_presented_by_main),
        ticks_dropped_without_clock: load(&COUNTERS.ticks_dropped_without_clock),
        ticks_dropped_paused: load(&COUNTERS.ticks_dropped_paused),
        ticks_dropped_needing_main: load(&COUNTERS.ticks_dropped_needing_main),
        ticks_dropped_stale: load(&COUNTERS.ticks_dropped_stale),
        ticks_installed: load(&COUNTERS.ticks_installed),
        ticks_laid_out: load(&COUNTERS.ticks_laid_out),
        ticks_presented: load(&COUNTERS.ticks_presented),
        ticks_needing_main: load(&COUNTERS.ticks_needing_main),
        ticks_moving_visual_contexts: load(&COUNTERS.ticks_moving_visual_contexts),
        ticks_asking_main_to_adopt: load(&COUNTERS.ticks_asking_main_to_adopt),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(record: u64, invalidation: u32, groups: u32) -> FfiRowSampledInPass {
        let mut sample = FfiRowSampledInPass::absent();
        sample.present = true;
        sample.style_record = record;
        sample.invalidation.invalidation = invalidation;
        sample.invalidation.changed_non_inherited_style_groups = groups;
        sample
    }

    #[test]
    fn folded_sample_takes_the_later_record_and_what_either_invalidated() {
        let folded = folded_sample(&sample(1, 0x2 | 0x10, 0b01), sample(2, 0x1 | 0x20, 0b10));
        assert_eq!(folded.style_record, 2);
        assert_eq!(folded.invalidation.invalidation & 0x3, 0x2);
        assert_eq!(folded.invalidation.invalidation & !0x3, 0x30);
        assert_eq!(folded.invalidation.changed_non_inherited_style_groups, 0b11);
    }
}
