/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Clock frames (on unless `LIBWEB_RENDER_CLOCK_FRAMES=0`): the animations of a document ticked on
//! the render owner.
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
//! the display ticks the compositor delivers for the clock's compositor context, without the main
//! thread: see [`rust_render_clock_post_tick`]. During the port the main thread still reaches the
//! arena itself, so such a tick runs only while the main thread is idle, blocked in its outermost
//! event loop with no frame in flight, which it tells the owner in order with everything it sent
//! before (see [`rust_render_clock_main_will_idle`]); the main thread waits for a tick that runs
//! before it goes on when it wakes (see [`rust_render_clock_main_did_wake`]).

use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::ThreadId;

use crate::css::style::StyleEngine;
use crate::css::style::StyleEngineInputHandle;
use crate::css::style::bridge::{
    FfiRowSampledInPass, FfiStyleInvalidationField, sample_installed_record_for_clock_tick,
};
use crate::css::style::engine_home::{Holder, Owed, StyleEngineLoan};
use crate::css::style::tree::StyleNodeID;
use crate::layout::node_data::NodeSlotId;
use crate::layout::update_layout::ClockLayoutFrame;
use crate::layout::{ArenaHandle, LayoutNodeArena};
use crate::render_owner::DocumentId;

/// Whether clock frames are on: unless `LIBWEB_RENDER_CLOCK_FRAMES=0`.
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("LIBWEB_RENDER_CLOCK_FRAMES").as_deref() != Ok("0"))
}

/// An element whose animations a clock ticks, with the record it holds.
#[derive(Clone, Copy)]
pub(crate) struct ClockTarget {
    style_node: StyleNodeID,
    style_record: u64,
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
    /// The record the target held before the tick.
    pub(crate) style_record_before: u64,
    /// The sample; `present` is false where the engine could not take it and the host samples the
    /// target itself.
    pub(crate) sample: FfiRowSampledInPass,
    /// Whether the arena took the sample's record ahead of the host.
    pub(crate) installed_in_arena: bool,
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

/// What the ticks of a document's clock publish for the main thread, which reads it once no tick
/// runs: the owner writes it as a tick ends, and the main thread reads it when it wakes, or once it
/// has taken a submitted tick back.
pub(crate) struct ClockPublication {
    /// The timeline time of the last tick that presented, or of the start, as `f64` bits.
    time: AtomicU64,
    /// How the last tick ended.
    outcome: Mutex<Option<FfiClockTickOutcome>>,
    /// Whether the render side presented every tick the host has not adopted yet, so that adopting
    /// them repaints nothing.
    presented_since_adoption: AtomicBool,
    /// How many display ticks in a row the render clock could not run because the main thread held
    /// the arena or had paused the clock, while nothing else ticked it: from
    /// [`MISSED_TICKS_BEFORE_MAIN`] on, the main thread's rendering updates tick the clock in its
    /// place.
    ticks_missed: AtomicU32,
    /// The time of the clock, as `f64` bits, when the render clock last missed a tick.
    time_at_missed_tick: AtomicU64,
    /// The progress (percent) at which the last tick sampled each scroll progress timeline, or the
    /// host held it at the start, by the timeline's style engine identity.
    scroll_progress: Mutex<Vec<(u32, f64)>>,
    /// What the ticks left for the host to adopt, one entry per target.
    entries: Mutex<Vec<ClockTickEntry>>,
    /// The layout frame the render clock's ticks lay out in, which the main thread hands the clock
    /// and takes in when it wakes.
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
            ticks_missed: AtomicU32::new(0),
            time_at_missed_tick: AtomicU64::new(f64::NAN.to_bits()),
            scroll_progress: Mutex::default(),
            entries: Mutex::default(),
            layout_frame: Mutex::default(),
        }
    }

    fn time(&self) -> f64 {
        f64::from_bits(self.time.load(Ordering::Acquire))
    }

    fn set_outcome(&self, outcome: FfiClockTickOutcome) {
        *self.outcome.lock().expect("clock publication outcome") = Some(outcome);
    }

    /// How many rounds the ticks laid out in the layout frame the main thread has yet to take in.
    fn rounds_owed(&self) -> u32 {
        self.layout_frame
            .lock()
            .expect("clock publication layout frame")
            .as_ref()
            .map_or(0, ClockLayoutFrame::rounds)
    }

    fn take_entry(&self) -> Option<ClockTickEntry> {
        let mut entries = self.entries.lock().expect("clock publication entries");
        if entries.is_empty() {
            return None;
        }
        Some(entries.remove(0))
    }
}

/// The clock of a document, which the owner keeps in the document's render state: what its ticks
/// sample, until when, and what they publish.
pub(crate) struct DocumentClock {
    /// The compositor context whose display ticks the render clock hands the clock, or 0.
    context: u64,
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
    pub(crate) fn context(&self) -> u64 {
        self.context
    }

    /// The timeline time of a frame shown at `frame_time` (unsafe shared current time, ms).
    fn timeline_time_at(&self, frame_time: f64) -> f64 {
        frame_time - self.timeline_zero
    }

    /// Samples the clock's targets at timeline time `time` and installs what they compose into the
    /// arena `arena_handle`, ahead of the host, which adopts the entries this publishes (see
    /// `style_engine_clock_tick_take_entry`). No tick samples at or past the deadline. The scroll
    /// timelines are sampled where the host sampled them.
    ///
    /// # Safety
    ///
    /// On the owner, which holds `state`, the arena of the clock's document, and `engine`, its style engine: inside
    /// the `clock` run the host submitted, or with the main thread idle.
    unsafe fn run_tick(
        &mut self,
        state: *mut ArenaHandle,
        engine: Option<&mut StyleEngine>,
        time: f64,
    ) -> FfiClockTickOutcome {
        let outcome = if time >= self.deadline {
            FfiClockTickOutcome::PastDeadline
        } else if let Some(engine) = engine {
            // The main thread pins and unpins its host's records beside the tick, which reads none of them.
            engine.begin_clock_lend_beside_host_pins();
            // SAFETY: Guaranteed by the caller.
            let outcome = unsafe { self.sample_and_install(state, engine, time) };
            engine.end_clock_tick_beside_host_pins();
            outcome
        } else {
            FfiClockTickOutcome::NeedsMain
        };
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
        // A scroll progress timeline is where the host sampled it. Past the progress at which an effect
        // changes its phase or its iteration, the host has events to send.
        let mut scroll_progress = Vec::with_capacity(self.scroll_timelines.len());
        for timeline in &self.scroll_timelines {
            let progress = samples
                .sample(timeline.identity)
                .flatten()
                .filter(|sample| sample.is_percentage)
                .map(|sample| sample.value);
            let Some(progress) = progress else {
                return FfiClockTickOutcome::NeedsMain;
            };
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
        let mut entries = self.published.entries.lock().expect("clock publication entries");
        self.repaints.clear();
        self.tick_started_fresh = entries.is_empty();
        let mut presentable = true;
        for target in &mut self.targets {
            // SAFETY: As above; the borrow ends with the call.
            let sampled = unsafe {
                sample_installed_record_for_clock_tick(
                    &mut *engine,
                    target.style_node,
                    target.style_record,
                    arena_handle,
                    &samples,
                )
            };
            let Some(sample) = sampled else {
                outcome = FfiClockTickOutcome::NeedsMain;
                presentable = false;
                entries.push(ClockTickEntry {
                    style_node: target.style_node,
                    style_record_before: target.style_record,
                    sample: declined_sample(),
                    installed_in_arena: false,
                });
                continue;
            };
            // The sample moved nothing the record composed.
            if sample.style_record == target.style_record {
                continue;
            }
            let previous_record = target.style_record;
            let level = sample.invalidation.invalidation & 0x3;
            let needs_layout_tree_rebuild = level >= 3;
            let needs_relayout = level >= 2;
            let installed_in_arena = !needs_layout_tree_rebuild
                && arena.install_animation_sample(target.style_node, sample.style_record, needs_relayout);
            presentable &= installed_in_arena
                && render_side_shows(&sample, || {
                    !target.has_pseudo_element_style_outside_box
                        && box_holds_only_text(arena, arena.bound_row(target.style_node))
                });
            if level >= 1 && installed_in_arena {
                let affects_hit_testing =
                    sample.invalidation.invalidation & FfiStyleInvalidationField::AffectsHitTesting as u32 != 0;
                self.repaints
                    .push((arena.bound_row(target.style_node), affects_hit_testing));
            }
            target.style_record = sample.style_record;
            // Ticks the host has not adopted yet fold into one entry per target, over the record the
            // host holds: the arena's log keeps only the last record too.
            if let Some(entry) = entries.iter_mut().find(|entry| {
                entry.style_node == target.style_node
                    && entry.sample.style_record == previous_record
                    && entry.installed_in_arena
                    && installed_in_arena
            }) {
                entry.sample = folded_sample(&entry.sample, sample);
                continue;
            }
            entries.push(ClockTickEntry {
                style_node: target.style_node,
                style_record_before: previous_record,
                sample,
                installed_in_arena,
            });
        }
        self.presentable = presentable;
        outcome
    }

    /// Lays out what the tick's samples left, in the layout frame the main thread handed the clock.
    /// Returns whether a round laid out, or `None` where the main thread has to.
    ///
    /// # Safety
    ///
    /// As for [`Self::run_tick`], with the main thread idle.
    unsafe fn lay_out(&self, state: *mut ArenaHandle) -> Option<bool> {
        let mut frame = self
            .published
            .layout_frame
            .lock()
            .expect("clock publication layout frame");
        let Some(frame) = frame.as_mut() else {
            // SAFETY: The caller holds the arena.
            let arena = unsafe { &*state }.arena();
            return arena.layout_is_up_to_date(false).then_some(false);
        };
        // SAFETY: Guaranteed by the caller.
        if !unsafe { frame.run_round() } {
            return None;
        }
        Some(frame.laid_out())
    }

    /// Shows what the tick installed and laid out: repaints the rows its samples repaint, records
    /// the display list again and has the host present it. Returns false where the render side
    /// cannot show it, and the main thread has to.
    ///
    /// # Safety
    ///
    /// As for [`Self::lay_out`].
    unsafe fn present(&self, state: *mut ArenaHandle) -> bool {
        if !self.presentable {
            return false;
        }
        let Some(present) = PRESENT.get() else {
            return false;
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
        if !unsafe { crate::painting::ffi::record_for_clock_tick(arena_handle) } {
            return false;
        }
        if present(arena_handle) {
            return true;
        }
        // Nothing presents the recording; the main thread records the frame again.
        arena.recording().discard_pending_recording();
        false
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

fn declined_sample() -> FfiRowSampledInPass {
    FfiRowSampledInPass {
        present: false,
        style_record: 0,
        invalidation: Default::default(),
        overlay_is_empty: true,
        substitution_marks: 0,
        keyframes_inherited_non_inherited_style_groups: 0,
        uses_tree_counting_function: false,
        custom_property_environment_moved: false,
        custom_property_environment: 0,
        custom_property_store: std::ptr::null(),
        custom_property_reactions: 0,
        custom_property_environment_named: false,
        rebuilt_every_group: false,
    }
}

thread_local! {
    // On the main thread, what the clock of each document it started publishes, until it stops it.
    static PUBLICATIONS: RefCell<HashMap<DocumentId, Arc<ClockPublication>>> = RefCell::new(HashMap::new());
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
    // SAFETY: Every caller passes the live arena of a document on the main thread.
    let document = unsafe { document_of(arena) };
    PUBLICATIONS.with_borrow(|publications| publications.get(&document).cloned())
}

/// Sends the owner `message` about the clock of a document.
fn send(message: ClockMessage) {
    crate::render_owner::send(crate::render_owner::ToOwner::Clock(message));
}

/// On the main thread: the document whose render state the owner dropped had its clock dropped
/// with it.
pub(crate) fn document_destroyed(document: DocumentId) {
    PUBLICATIONS.with_borrow_mut(|publications| publications.remove(&document));
}

// The ticks the host adopted that installed a sample.
static CLOCK_TICKS_PRESENTED: AtomicU64 = AtomicU64::new(0);

/// Whether clock frames are on.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_frames_enabled() -> bool {
    enabled()
}

/// Whether a rendering update may submit a clock tick: clock frames are on and the stage thread
/// runs the `clock` stage beside the main thread.
#[unsafe(no_mangle)]
pub extern "C" fn rust_stage_thread_submits_clock() -> bool {
    enabled() && crate::stage_thread::submits("clock")
}

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
    let published = Arc::new(ClockPublication::new(time));
    PUBLICATIONS.with_borrow_mut(|publications| publications.insert(document, Arc::clone(&published)));
    send(ClockMessage::Start {
        document,
        clock: Box::new(DocumentClock {
            context,
            timeline_identity,
            timeline_zero,
            deadline,
            paused: false,
            needs_main: false,
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
    style_records: *const u64,
    pseudo_element_styles_outside_box: *const bool,
    count: usize,
) {
    if publication_of(arena).is_none() {
        return;
    }
    let mut targets = Vec::with_capacity(count);
    for index in 0..count {
        // SAFETY: Guaranteed by the caller.
        let (node, record, outside_box) = unsafe {
            (
                *style_nodes.add(index),
                *style_records.add(index),
                *pseudo_element_styles_outside_box.add(index),
            )
        };
        if let Some(style_node) = StyleNodeID::from_raw(node) {
            targets.push(ClockTarget {
                style_node,
                style_record: record,
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

/// Stops the clock of the document whose layout arena is `arena`, if it has one. What its last tick
/// left for the host to adopt goes with it.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_document_clock_stop(arena: *mut c_void) {
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    let Some(published) = PUBLICATIONS.with_borrow_mut(|publications| publications.remove(&document)) else {
        return;
    };
    published.entries.lock().expect("clock publication entries").clear();
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
    if !rust_stage_thread_submits_clock() {
        return false;
    }
    let Some(published) = publication_of(arena) else {
        return false;
    };
    // SAFETY: Guaranteed by the caller.
    let document = unsafe { document_of(arena) };
    *published.outcome.lock().expect("clock publication outcome") = None;
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
    publication_of(arena).is_some_and(|published| published.presented_since_adoption.load(Ordering::Acquire))
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

/// Takes the next entry the ticks of the clock of the document whose layout arena is `arena` left
/// for the host to adopt.
pub(crate) fn take_clock_tick_entry(arena: *mut c_void) -> Option<ClockTickEntry> {
    publication_of(arena)?.take_entry()
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

/// Whether the render clock missed so many display ticks of the clock of the document whose layout
/// arena is `arena` in a row that the main thread's rendering updates tick it in its place.
#[unsafe(no_mangle)]
pub extern "C" fn rust_document_clock_misses_ticks(arena: *mut c_void) -> bool {
    publication_of(arena).is_some_and(|published| {
        published.ticks_missed.load(Ordering::Acquire) >= MISSED_TICKS_BEFORE_MAIN
            && published.time_at_missed_tick.load(Ordering::Acquire) == published.time.load(Ordering::Acquire)
    })
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
    /// The main thread `main_thread` went idle: the render clock's ticks may run until it wakes,
    /// unless it woke already (see [`rust_render_clock_main_will_idle`]).
    MainIdle { generation: u64, main_thread: ThreadId },
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
    /// The document the message is about; none for what the idle main thread and the render clock
    /// send.
    pub(crate) fn document(&self) -> DocumentId {
        match self {
            Self::Start { document, .. }
            | Self::Targets { document, .. }
            | Self::ScrollTimelines { document, .. }
            | Self::Paused { document, .. }
            | Self::Stop { document }
            | Self::SubmittedTick { document, .. } => *document,
            Self::MainIdle { .. } | Self::DisplayTick { .. } | Self::InjectedTick { .. } => DocumentId::default(),
        }
    }
}

/// How the owner runs a display tick for a compositor context at a frame time. The owner reaches what a tick runs (the
/// sampling, the layout, the recording) only through the ticks it is sent, so what reaches the owner without it (the
/// unit tests' stage threads) links without it.
pub(crate) type RunDisplayTick = fn(u64, i64);

/// A tick a rendering update submitted: at timeline time `time`, with the style engine the update
/// lent it, publishing to `published`, which the main thread reads once it has taken the tick back.
/// `run` runs it, as [`RunDisplayTick`] does a display tick.
pub(crate) struct SubmittedTick {
    time: f64,
    style_engine: Option<StyleEngineLoan>,
    published: Arc<ClockPublication>,
    run: fn(DocumentId, SubmittedTick),
}

/// Handles `message` on the owner.
pub(crate) fn handle_on_owner(message: ClockMessage) {
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
            crate::render_owner::with_clock_slot(document, |slot| *slot = None);
        }
        ClockMessage::MainIdle {
            generation,
            main_thread,
        } => {
            let mut gate = idle_gate().state.lock().expect("render clock idle gate");
            if gate.holder == ArenaHolder::Main && gate.idle_requested == Some(generation) {
                gate.holder = ArenaHolder::Idle(main_thread);
                gate.idle_requested = None;
            }
        }
        ClockMessage::DisplayTick { context, slot, run } => {
            // A read-modify-write, which a later tick's post orders against: either that post finds the
            // slot free and posts a tick of its own, or the time it stored is the one read here.
            slot.queued.swap(false, Ordering::AcqRel);
            let frame_time_nanoseconds = slot.frame_time_nanoseconds.load(Ordering::Acquire);
            run(context, frame_time_nanoseconds);
        }
        ClockMessage::InjectedTick {
            context,
            frame_time_nanoseconds,
            run,
        } => {
            run(context, frame_time_nanoseconds);
            end_injected_tick();
        }
        ClockMessage::SubmittedTick { document, tick, ticket } => ticket.run(|| (tick.run)(document, *tick)),
    }
}

/// Takes the clock of `document` out of its render state, with the arena it ticks, for a tick that
/// may reach other documents' state while it runs. [`put_clock_back`] puts it back.
fn take_clock(document: DocumentId) -> Option<(DocumentClock, *mut ArenaHandle)> {
    crate::render_owner::with_clock(document, |slot, arena| slot.take().map(|clock| (clock, arena))).flatten()
}

fn put_clock_back(document: DocumentId, clock: DocumentClock) {
    crate::render_owner::with_clock_slot(document, |slot| {
        debug_assert!(slot.is_none(), "nothing starts a clock while its tick runs");
        slot.get_or_insert(clock);
    });
}

/// Runs the tick a rendering update of `document` submitted, on the owner inside its run.
fn run_submitted_tick(document: DocumentId, tick: SubmittedTick) {
    let SubmittedTick {
        time,
        style_engine,
        published,
        run: _,
    } = tick;
    // Taking the clock applies the arena's pending changes, which may reach the engine: inside the loan.
    let run = |engine: Option<&mut StyleEngine>| {
        let Some((mut clock, arena)) = take_clock(document) else {
            published.set_outcome(FfiClockTickOutcome::Stopped);
            return;
        };
        debug_assert!(
            Arc::ptr_eq(&clock.published, &published),
            "a submitted tick publishes to the clock it was submitted for"
        );
        // SAFETY: The run owns the arena and its engine until the main thread takes it back.
        unsafe { clock.run_tick(arena, engine, time) };
        put_clock_back(document, clock);
    };
    match style_engine {
        Some(mut loan) => loan.lend_to_this_thread(|engine| run(Some(engine))),
        None => run(None),
    }
}

/// Runs the display tick at `frame_time_nanoseconds` for the clock that ticks at the compositor
/// context `context`, on the owner.
fn run_display_tick(context: u64, frame_time_nanoseconds: i64) {
    count(&COUNTERS.ticks_run);
    let Some(document) = crate::render_owner::document_with_clock_at(context) else {
        count(&COUNTERS.ticks_dropped_without_clock);
        return;
    };
    let idle_tick = match IdleTick::begin() {
        Ok(idle_tick) => idle_tick,
        Err(main_holds_arena) => {
            count(&COUNTERS.ticks_dropped_main_busy);
            if main_holds_arena {
                let published = crate::render_owner::with_clock_slot(document, |slot| {
                    slot.as_ref().map(|clock| Arc::clone(&clock.published))
                })
                .flatten();
                if let Some(published) = published {
                    miss_tick(context, &published);
                }
            }
            return;
        }
    };
    // Taking the clock applies the arena's pending changes, which may reach the engine the idle main thread left home.
    let taken = match crate::render_owner::style_engine_of(document) {
        // SAFETY: The main thread is idle with nothing in flight, and waits for this tick when it wakes.
        Some(engine) if !engine.is_null() => unsafe { engine.reach_on_owner(|_| take_clock(document)) },
        _ => take_clock(document),
    };
    let Some((mut clock, arena)) = taken else {
        count(&COUNTERS.ticks_dropped_without_clock);
        return;
    };
    run_display_tick_on(&mut clock, arena, idle_tick, context, frame_time_nanoseconds);
    put_clock_back(document, clock);
}

/// Runs a display tick of `clock`, whose document's arena is `arena`, while `idle_tick` holds the
/// arenas.
fn run_display_tick_on(
    clock: &mut DocumentClock,
    arena: *mut ArenaHandle,
    idle_tick: IdleTick,
    context: u64,
    frame_time_nanoseconds: i64,
) {
    if clock.paused {
        count(&COUNTERS.ticks_dropped_paused);
        drop(idle_tick);
        miss_tick(context, &clock.published);
        return;
    }
    if clock.needs_main {
        count(&COUNTERS.ticks_dropped_needing_main);
        return;
    }
    let published = Arc::clone(&clock.published);
    published.ticks_missed.store(0, Ordering::Release);
    let time = clock.timeline_time_at(frame_time_nanoseconds as f64 / 1.0e6);
    if time.partial_cmp(&published.time()) != Some(std::cmp::Ordering::Greater) {
        count(&COUNTERS.ticks_dropped_stale);
        return;
    }
    let mut tick = None;
    crate::stage_thread::run_detached_for(idle_tick.caller, arena as usize, || {
        let run_tick = || {
            // SAFETY: The main thread is idle with nothing in flight, and waits for this tick when
            // it wakes: the owner holds the arena and its engine until `idle_tick` is dropped.
            let engine = unsafe { &*arena }.arena().style_engine_handle();
            // SAFETY: As above.
            let outcome = if engine.is_null() {
                unsafe { clock.run_tick(arena, None, time) }
            } else {
                unsafe { engine.reach_on_owner(|engine| clock.run_tick(arena, Some(engine), time)) }
            };
            if outcome != FfiClockTickOutcome::Presented {
                return (outcome, false, false);
            }
            // A sample the arena did not take, the host installs over the record the target held
            // before: only its entry keeps that record alive, and no later tick may sample over it.
            if published
                .entries
                .lock()
                .expect("clock publication entries")
                .iter()
                .any(|entry| !entry.installed_in_arena)
            {
                return (FfiClockTickOutcome::NeedsMain, false, false);
            }
            // SAFETY: As above.
            let Some(laid_out) = (unsafe { clock.lay_out(arena) }) else {
                return (FfiClockTickOutcome::NeedsMain, false, false);
            };
            // A round that moved a box that owns a clip, a transform or a scroll frame moved the
            // visual contexts, which the compositor has from the main thread's frames.
            // SAFETY: As above.
            if laid_out && !unsafe { crate::painting::ffi::settle_visual_contexts_for_clock_tick(arena.cast()) } {
                count(&COUNTERS.ticks_moving_visual_contexts);
                return (FfiClockTickOutcome::NeedsMain, laid_out, false);
            }
            // A tick that moved nothing shows nothing new.
            let moved_nothing = !laid_out && clock.repaints.is_empty();
            // SAFETY: As above.
            if !moved_nothing && !unsafe { clock.present(arena) } {
                return (FfiClockTickOutcome::NeedsMain, laid_out, false);
            }
            (outcome, laid_out, !moved_nothing)
        };
        tick = Some(std::panic::catch_unwind(std::panic::AssertUnwindSafe(run_tick)));
    });
    let Some(Ok((outcome, laid_out, presented_frame))) = tick else {
        // A tick has nobody to hand a panic to.
        std::process::abort();
    };
    let presented = outcome == FfiClockTickOutcome::Presented;
    if !presented {
        // The clock stops at the host, which adopts what the ticks left first.
        published.set_outcome(outcome);
        clock.needs_main = true;
    }
    if clock.tick_started_fresh {
        published.presented_since_adoption.store(presented, Ordering::Release);
    } else if !presented {
        published.presented_since_adoption.store(false, Ordering::Release);
    }
    if published.entries.lock().is_ok_and(|entries| !entries.is_empty()) {
        TICKS_TO_ADOPT.store(true, Ordering::Release);
    }
    let rounds_owed = published.rounds_owed();
    drop(idle_tick);
    if presented {
        count(&COUNTERS.ticks_installed);
        if laid_out {
            count(&COUNTERS.ticks_laid_out);
        }
        // A tick that moved nothing presented no frame.
        if presented_frame {
            count(&COUNTERS.ticks_presented);
        }
        // What the rounds owe the document piles up in the frame until the main thread takes it in:
        // one that idles for long takes it in every so often, and pays for a few rounds at a time.
        if laid_out && rounds_owed >= MAX_CLOCK_ROUNDS_OWED {
            count(&COUNTERS.ticks_waking_main_to_adopt);
            if let Some(wake_main) = WAKE_MAIN.get() {
                wake_main();
            }
        }
        return;
    }
    count(&COUNTERS.ticks_needing_main);
    if let Some(needs_main) = NEEDS_MAIN.get() {
        needs_main(context);
    }
}

/// How many display ticks in a row a clock may miss before the main thread's rendering updates tick
/// it: a task that holds the arena for a frame or two is over before anyone sees the difference.
const MISSED_TICKS_BEFORE_MAIN: u32 = 2;

/// Notes a display tick the render clock could not run for the clock that publishes to
/// `published`: the main thread held its arena or had paused it. Nothing else has the main thread
/// render, so one that keeps missing them asks it for a rendering update at every display tick
/// until the render clock runs one again.
fn miss_tick(context: u64, published: &ClockPublication) {
    // A rendering update that ticked the clock since the last miss ends the run: one the render
    // clock's tick ran into is no reason to ask for another.
    let time = published.time.load(Ordering::Acquire);
    let missed = if published.time_at_missed_tick.swap(time, Ordering::AcqRel) == time {
        published.ticks_missed.load(Ordering::Acquire).saturating_add(1)
    } else {
        1
    };
    published.ticks_missed.store(missed, Ordering::Release);
    if missed < MISSED_TICKS_BEFORE_MAIN {
        return;
    }
    count(&COUNTERS.ticks_missed_asking_main);
    if let Some(needs_main) = NEEDS_MAIN.get() {
        needs_main(context);
    }
}

// The render clock: display ticks delivered to a thread of their own, which sends them to the owner
// for the clock of their compositor context, which it ticks while the main thread is idle.

/// Who may reach the arenas of documents with a clock now.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ArenaHolder {
    /// The main thread: it is awake, or it has a frame in flight, or the owner has yet to learn that
    /// it went idle.
    Main,
    /// Nobody: the main thread is blocked in its outermost event loop, and a render clock tick may
    /// start. The tick acts for the thread named.
    Idle(ThreadId),
    /// A render clock tick, which the main thread waits for when it wakes.
    Tick,
}

struct GateState {
    holder: ArenaHolder,
    /// How many times the main thread went idle.
    idle_generation: u64,
    /// The time the main thread went idle that the owner has yet to learn of, while it has not woken
    /// since: [`ClockMessage::MainIdle`] with this generation lets the ticks in.
    idle_requested: Option<u64>,
}

struct IdleGate {
    state: Mutex<GateState>,
    tick_ended: Condvar,
}

fn idle_gate() -> &'static IdleGate {
    static GATE: OnceLock<IdleGate> = OnceLock::new();
    GATE.get_or_init(|| IdleGate {
        state: Mutex::new(GateState {
            holder: ArenaHolder::Main,
            idle_generation: 0,
            idle_requested: None,
        }),
        tick_ended: Condvar::new(),
    })
}

/// A render clock tick's hold on the arenas, taken where the main thread is idle.
struct IdleTick {
    /// The thread the tick acts for, which it gives the arenas back to when it ends.
    caller: ThreadId,
}

impl IdleTick {
    /// Takes the arenas for a tick. Fails where the main thread holds them, with `true`, or where
    /// another tick holds them, with `false`.
    fn begin() -> Result<Self, bool> {
        let mut state = idle_gate().state.lock().expect("render clock idle gate");
        let caller = match state.holder {
            ArenaHolder::Idle(caller) => caller,
            ArenaHolder::Tick => return Err(false),
            ArenaHolder::Main => return Err(true),
        };
        state.holder = ArenaHolder::Tick;
        Ok(Self { caller })
    }
}

impl Drop for IdleTick {
    fn drop(&mut self) {
        let gate = idle_gate();
        let mut state = gate.state.lock().expect("render clock idle gate");
        if state.holder == ArenaHolder::Tick {
            state.holder = ArenaHolder::Idle(self.caller);
        }
        drop(state);
        gate.tick_ended.notify_all();
    }
}

// Whether a render clock tick installed something since the main thread last woke.
static TICKS_TO_ADOPT: AtomicBool = AtomicBool::new(false);

/// The main thread is about to block in its outermost event loop. Unless it has a frame in flight,
/// render clock ticks may reach the arenas of documents with a clock until it wakes, once the owner
/// has handled what the main thread sent it before.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_main_will_idle() {
    if !enabled() || crate::stage_thread::has_frame_in_flight() {
        return;
    }
    let generation = {
        let mut state = idle_gate().state.lock().expect("render clock idle gate");
        if state.holder != ArenaHolder::Main {
            return;
        }
        state.idle_generation += 1;
        state.idle_requested = Some(state.idle_generation);
        state.idle_generation
    };
    send(ClockMessage::MainIdle {
        generation,
        main_thread: std::thread::current().id(),
    });
}

/// The main thread woke. Waits for a render clock tick that is running to end, and takes the arenas
/// back. Returns whether a tick installed something since it last woke, which the documents adopt
/// before anything else reaches them.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_main_did_wake() -> bool {
    if !enabled() {
        return false;
    }
    take_arenas_back();
    TICKS_TO_ADOPT.swap(false, Ordering::AcqRel)
}

fn take_arenas_back() {
    let gate = idle_gate();
    let mut state = gate.state.lock().expect("render clock idle gate");
    // A tick a test injected while the main thread idled runs before it takes the arenas back.
    while state.holder == ArenaHolder::Tick || INJECTED_TICKS_PENDING.load(Ordering::Acquire) > 0 {
        state = gate.tick_ended.wait(state).expect("render clock idle gate");
    }
    state.holder = ArenaHolder::Main;
    state.idle_requested = None;
}

// The ticks a test injected that the owner has not run yet.
static INJECTED_TICKS_PENDING: AtomicUsize = AtomicUsize::new(0);

/// Wakes the main thread from its event loop. Runs on the Rendering thread.
static WAKE_MAIN: OnceLock<extern "C" fn()> = OnceLock::new();

/// Has the render clock call `wake_main()` on the Rendering thread where a tick a test injected ended, for the main
/// thread to go on with what waits for it, and where the ticks laid out many rounds for an idle main thread to take
/// in. The first one set stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_set_wake_main(wake_main: extern "C" fn()) {
    let _ = WAKE_MAIN.set(wake_main);
}

/// Hands the owner a display tick at `frame_time_nanoseconds` (monotonic time) for the compositor context `context`,
/// as the render clock hands it one, for a test that drives the ticks itself. Call it where the main thread has just
/// let render clock ticks in (see [`rust_render_clock_main_will_idle`]): the main thread waits for the tick before it
/// takes the arenas back, and the tick wakes it when it ends. Returns false where there is no owner to tick it.
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
    INJECTED_TICKS_PENDING.fetch_add(1, Ordering::AcqRel);
    let sent = crate::stage_thread::send_to_owner(crate::render_owner::ToOwner::Clock(ClockMessage::InjectedTick {
        context,
        frame_time_nanoseconds,
        run: run_display_tick,
    }))
    .is_ok();
    if !sent {
        end_injected_tick();
    }
    sent
}

fn end_injected_tick() {
    let gate = idle_gate();
    {
        let _state = gate.state.lock().expect("render clock idle gate");
        INJECTED_TICKS_PENDING.fetch_sub(1, Ordering::AcqRel);
    }
    gate.tick_ended.notify_all();
    if let Some(wake_main) = WAKE_MAIN.get() {
        wake_main();
    }
}

/// Where the render clock's display ticks wait for the owner: one per compositor context. Ticks
/// that arrive while one waits fold into it, with the latest time.
pub(crate) struct ClockSlot {
    frame_time_nanoseconds: AtomicI64,
    queued: AtomicBool,
}

/// How many rounds the ticks lay out in one layout frame before they wake an idle main thread to
/// take it in: about two seconds of display ticks.
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
    /// Ticks that found the main thread holding the arenas, and were dropped.
    pub ticks_dropped_main_busy: u64,
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
    /// Ticks that woke an idle main thread to take in the rounds they laid out.
    pub ticks_waking_main_to_adopt: u64,
    pub ticks_missed_asking_main: u64,
}

#[derive(Default)]
struct RenderClockCounters {
    ticks_posted: AtomicU64,
    ticks_folded: AtomicU64,
    ticks_run: AtomicU64,
    ticks_dropped_main_busy: AtomicU64,
    ticks_dropped_without_clock: AtomicU64,
    ticks_dropped_paused: AtomicU64,
    ticks_dropped_needing_main: AtomicU64,
    ticks_dropped_stale: AtomicU64,
    ticks_installed: AtomicU64,
    ticks_laid_out: AtomicU64,
    ticks_presented: AtomicU64,
    ticks_needing_main: AtomicU64,
    ticks_moving_visual_contexts: AtomicU64,
    ticks_waking_main_to_adopt: AtomicU64,
    ticks_missed_asking_main: AtomicU64,
}

static COUNTERS: RenderClockCounters = RenderClockCounters {
    ticks_posted: AtomicU64::new(0),
    ticks_folded: AtomicU64::new(0),
    ticks_run: AtomicU64::new(0),
    ticks_dropped_main_busy: AtomicU64::new(0),
    ticks_dropped_without_clock: AtomicU64::new(0),
    ticks_dropped_paused: AtomicU64::new(0),
    ticks_dropped_needing_main: AtomicU64::new(0),
    ticks_dropped_stale: AtomicU64::new(0),
    ticks_installed: AtomicU64::new(0),
    ticks_laid_out: AtomicU64::new(0),
    ticks_presented: AtomicU64::new(0),
    ticks_needing_main: AtomicU64::new(0),
    ticks_moving_visual_contexts: AtomicU64::new(0),
    ticks_waking_main_to_adopt: AtomicU64::new(0),
    ticks_missed_asking_main: AtomicU64::new(0),
};

fn count(counter: &AtomicU64) {
    counter.fetch_add(1, Ordering::Relaxed);
}

/// Presents the display list a render clock tick recorded for the document of an arena: publishes
/// it and hands the frame to the compositor. Runs on the Rendering thread, with the main thread idle.
static PRESENT: OnceLock<extern "C" fn(*mut c_void) -> bool> = OnceLock::new();

/// Has the render clock call `present(arena)` to present what a tick recorded. The first one set
/// stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_set_present(present: extern "C" fn(*mut c_void) -> bool) {
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

/// A render clock's way to the owner, or null where there is no owner beside the main thread to tick
/// clocks on (the stages do not overlap, or clock frames are off). The render clock thread owns it,
/// and destroys it with [`rust_render_clock_sender_destroy`].
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_sender_create() -> *mut ClockSender {
    if !enabled() || !crate::stage_thread::owner_runs_beside_main() {
        return std::ptr::null_mut();
    }
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
/// `context` to the owner, which ticks the clock of that context with it if the main thread is idle
/// then. Where a tick for the context is still waiting there, it takes this time instead. Returns
/// false where there is no owner, which there only is not when the process is going.
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
        ticks_dropped_main_busy: load(&COUNTERS.ticks_dropped_main_busy),
        ticks_dropped_without_clock: load(&COUNTERS.ticks_dropped_without_clock),
        ticks_dropped_paused: load(&COUNTERS.ticks_dropped_paused),
        ticks_dropped_needing_main: load(&COUNTERS.ticks_dropped_needing_main),
        ticks_dropped_stale: load(&COUNTERS.ticks_dropped_stale),
        ticks_installed: load(&COUNTERS.ticks_installed),
        ticks_laid_out: load(&COUNTERS.ticks_laid_out),
        ticks_presented: load(&COUNTERS.ticks_presented),
        ticks_needing_main: load(&COUNTERS.ticks_needing_main),
        ticks_moving_visual_contexts: load(&COUNTERS.ticks_moving_visual_contexts),
        ticks_waking_main_to_adopt: load(&COUNTERS.ticks_waking_main_to_adopt),
        ticks_missed_asking_main: load(&COUNTERS.ticks_missed_asking_main),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(record: u64, invalidation: u32, groups: u32) -> FfiRowSampledInPass {
        let mut sample = declined_sample();
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

    #[test]
    fn idle_gate_lets_a_tick_in_only_once_the_owner_learns_the_main_thread_idles_and_waits_for_it() {
        take_arenas_back();
        assert_eq!(IdleTick::begin().err(), Some(true), "the main thread holds the arenas");
        let main_thread = std::thread::current().id();
        let generation = {
            let mut state = idle_gate().state.lock().unwrap();
            state.idle_generation += 1;
            state.idle_requested = Some(state.idle_generation);
            state.idle_generation
        };
        // A stale idle, one the main thread woke from before the owner learned of it, lets nothing in.
        handle_on_owner(ClockMessage::MainIdle {
            generation: generation - 1,
            main_thread,
        });
        assert_eq!(IdleTick::begin().err(), Some(true), "a stale idle lets no tick in");
        handle_on_owner(ClockMessage::MainIdle {
            generation,
            main_thread,
        });
        let tick = IdleTick::begin().expect("the main thread is idle");
        assert_eq!(IdleTick::begin().err(), Some(false), "another tick holds the arenas");
        let woke = Arc::new(AtomicBool::new(false));
        let waker = {
            let woke = Arc::clone(&woke);
            std::thread::spawn(move || {
                take_arenas_back();
                woke.store(true, Ordering::SeqCst);
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(20));
        assert!(!woke.load(Ordering::SeqCst), "the main thread waits for the tick");
        drop(tick);
        waker.join().unwrap();
        assert!(woke.load(Ordering::SeqCst));
        assert!(IdleTick::begin().is_err());
        // Once the main thread woke, the idle it asked for before lets nothing in.
        handle_on_owner(ClockMessage::MainIdle {
            generation,
            main_thread,
        });
        assert_eq!(IdleTick::begin().err(), Some(true));
    }
}
