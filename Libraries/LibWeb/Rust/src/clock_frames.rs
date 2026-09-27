/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Clock frames (on unless `LIBWEB_RENDER_CLOCK_FRAMES=0`): the animations of a document ticked on
//! the render side under a lease the main thread grants.
//!
//! The main thread grants a document a [`ClockLease`] at the end of a rendering update in which
//! nothing but the running animations of its document timeline would change what the next one
//! shows. A tick samples those animations at one time and installs the records they compose into
//! the document's layout arena, ahead of the host, which adopts them when it takes the tick back.
//! The main thread still drives the ticks: each rendering update of a leased document submits one
//! as a stage of its own, labelled `clock`, ahead of its layout pass.
//!
//! The registry is keyed by the layout arena handle of the document. A caller holding an
//! [`Arc<ClockLease>`] from [`clock_lease_for`] may tick it on the thread that owns the arena.
//!
//! A render clock (`Web::Compositor::RenderClock`, a thread of its own) also ticks a lease, at the display
//! ticks the compositor delivers for the lease's compositor context, without the main thread: see
//! [`rust_render_clock_post_tick`]. Such a tick runs on the stage thread only while the main thread
//! is idle, blocked in its outermost event loop with no frame in flight, and the main thread waits
//! for it before it goes on when it wakes (see [`rust_render_clock_main_did_wake`]).

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::thread::ThreadId;

use crate::css::style::StyleEngine;
use crate::css::style::bridge::{
    FfiRowSampledInPass, FfiStyleInvalidationField, sample_installed_record_for_clock_tick,
};
use crate::css::style::engine_home::{Holder, Owed};
use crate::css::style::tree::StyleNodeID;
use crate::layout::LayoutNodeArena;
use crate::layout::node_data::NodeSlotId;
use crate::layout::update_layout::ClockLayoutFrame;

/// Whether clock frames are on: unless `LIBWEB_RENDER_CLOCK_FRAMES=0`.
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var("LIBWEB_RENDER_CLOCK_FRAMES").as_deref() != Ok("0"))
}

/// An element whose animations a lease ticks, with the record it holds.
#[derive(Clone, Copy)]
struct ClockTarget {
    style_node: StyleNodeID,
    style_record: u64,
    /// Whether the element has a pseudo-element style that inherits from it outside its box: a
    /// ::backdrop or a ::selection style, which only the main thread derives.
    has_pseudo_element_style_outside_box: bool,
}

/// A scroll progress timeline whose animations a lease ticks, as the host granted it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FfiClockLeaseScrollTimeline {
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
    /// The progress (in percent) from which, and up to which, no effect of the lease on the timeline
    /// changes its phase or its iteration, which the host has events for.
    pub progress_start: f64,
    pub progress_end: f64,
    /// The progress (in percent) at the scroll offset the host holds.
    pub progress: f64,
}

impl FfiClockLeaseScrollTimeline {
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
    /// A target needs the host: it samples that target itself, and the lease ends.
    NeedsMain,
    /// The tick reached the lease's deadline, where the host's rendering update takes over.
    PastDeadline,
    /// The lease was revoked before the tick ran.
    Revoked,
}

/// A document's clock lease.
pub struct ClockLease {
    arena: usize,
    /// The compositor context whose display ticks the render clock hands the lease, or 0.
    context: u64,
    /// The style engine identity of the document timeline the lease ticks.
    timeline_identity: u32,
    /// The unsafe shared current time, in milliseconds, at which the timeline reads zero: a frame
    /// time `t` is timeline time `t - timeline_zero`.
    timeline_zero: f64,
    /// The timeline time at which the host has something observable to do (an event, a phase
    /// change, the end of an effect): no tick samples at or past it.
    deadline: f64,
    /// The timeline time of the last tick, as `f64` bits.
    time: AtomicU64,
    revoked: AtomicBool,
    /// Whether the render clock leaves the lease alone: the main thread moved what it ticks, and
    /// its rendering update decides what becomes of the lease.
    paused: AtomicBool,
    targets: Mutex<Vec<ClockTarget>>,
    /// The scroll progress timelines whose animations the lease ticks, with the progress (percent) of
    /// the last tick that sampled each, or NaN.
    scroll_timelines: Mutex<Vec<(FfiClockLeaseScrollTimeline, f64)>>,
    entries: Mutex<Vec<ClockTickEntry>>,
    /// The layout frame the render clock's ticks lay out in, which the main thread takes in when
    /// it wakes.
    layout_frame: Mutex<Option<ClockLayoutFrame>>,
    /// The rows the last tick's samples repaint, and whether they hit test differently.
    repaints: Mutex<Vec<(NodeSlotId, bool)>>,
    /// Whether the render side can show what the last tick installed without the main thread: no
    /// sample moved what only the main thread derives (descendants' styles, visual contexts).
    presentable: AtomicBool,
    /// Whether the last tick found nothing left for the host to adopt.
    tick_started_fresh: AtomicBool,
    /// Whether the render side presented every tick the host has not adopted yet, so that adopting
    /// them repaints nothing.
    presented_since_adoption: AtomicBool,
    /// Whether a render clock tick ended the lease: the render clock ticks it no more, and the main
    /// thread adopts what its ticks left before its rendering update ends it. A later tick would
    /// sample over records only the entries the host has not adopted yet keep alive.
    needs_main: AtomicBool,
    /// How many display ticks in a row the render clock could not run because the main thread held
    /// the arena or had paused the lease, while nothing else ticked it: from
    /// [`MISSED_TICKS_BEFORE_MAIN`] on, the main thread's rendering updates tick the lease in its
    /// place.
    ticks_missed: AtomicU32,
    /// The time of the lease, as `f64` bits, when the render clock last missed a tick.
    time_at_missed_tick: AtomicU64,
    outcome: Mutex<Option<FfiClockTickOutcome>>,
}

// SAFETY: `ClockTickEntry` carries a borrowed custom-property store pointer, which only the thread
// that owns the arena (and so the engine) dereferences.
unsafe impl Send for ClockLease {}
// SAFETY: As above; everything else is behind atomics and mutexes.
unsafe impl Sync for ClockLease {}

impl ClockLease {
    /// The timeline time of the last tick, or of the grant.
    pub fn time(&self) -> f64 {
        f64::from_bits(self.time.load(Ordering::Acquire))
    }

    pub fn deadline(&self) -> f64 {
        self.deadline
    }

    pub fn is_revoked(&self) -> bool {
        self.revoked.load(Ordering::Acquire)
    }

    /// The timeline time of a frame shown at `frame_time` (unsafe shared current time, ms).
    pub fn timeline_time_at(&self, frame_time: f64) -> f64 {
        frame_time - self.timeline_zero
    }

    /// Samples the lease's targets at timeline time `time` and installs what they compose into the
    /// arena, ahead of the host, which adopts the entries this leaves (see
    /// `style_engine_clock_tick_take_entry`). No tick samples at or past `deadline`. The scroll
    /// timelines are sampled where the host sampled them.
    ///
    /// # Safety
    ///
    /// On the thread that owns the lease's arena and its style engine: the stage thread inside the
    /// `clock` stage the host submitted, or the main thread with nothing in flight.
    pub unsafe fn run_tick(&self, time: f64, deadline: f64) -> FfiClockTickOutcome {
        let outcome = if self.is_revoked() {
            FfiClockTickOutcome::Revoked
        } else if time >= deadline {
            FfiClockTickOutcome::PastDeadline
        } else {
            // SAFETY: Guaranteed by the caller.
            unsafe { self.sample_and_install(time) }
        };
        if outcome == FfiClockTickOutcome::Presented {
            let previous = self.time();
            self.time.store(previous.max(time).to_bits(), Ordering::Release);
        }
        *self.outcome.lock().expect("clock lease outcome") = Some(outcome);
        outcome
    }

    /// # Safety
    ///
    /// As for [`Self::run_tick`].
    unsafe fn sample_and_install(&self, time: f64) -> FfiClockTickOutcome {
        let arena_handle = self.arena as *mut c_void;
        // SAFETY: The caller owns the arena, which the lease's registration keeps alive.
        let arena = unsafe { &*arena_handle.cast::<LayoutNodeArena>() };
        let engine = arena.style_engine_handle();
        if engine.is_null() {
            return FfiClockTickOutcome::NeedsMain;
        }
        // SAFETY: The caller owns the engine; no other borrow of it is live here.
        let engine: *mut StyleEngine = unsafe { engine.enter("clock tick") };
        // SAFETY: The caller owns the engine; no other borrow of it is live here.
        let mut samples = unsafe { &*engine }
            .animation_timeline_samples()
            .with_time(self.timeline_identity, time);
        // A scroll progress timeline is where the host sampled it. Past the progress at which an effect
        // changes its phase or its iteration, the host has events to send.
        for (timeline, last_progress) in self
            .scroll_timelines
            .lock()
            .expect("clock lease scroll timelines")
            .iter_mut()
        {
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
            *last_progress = progress;
        }
        let mut outcome = FfiClockTickOutcome::Presented;
        let mut targets = self.targets.lock().expect("clock lease targets");
        let mut entries = self.entries.lock().expect("clock lease entries");
        let mut repaints = self.repaints.lock().expect("clock lease repaints");
        repaints.clear();
        self.tick_started_fresh.store(entries.is_empty(), Ordering::Release);
        let mut presentable = true;
        for target in targets.iter_mut() {
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
                repaints.push((arena.bound_row(target.style_node), affects_hit_testing));
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
        self.presentable.store(presentable, Ordering::Release);
        outcome
    }

    /// How many rounds the ticks laid out in the layout frame the main thread has yet to take in.
    fn rounds_owed(&self) -> u32 {
        self.layout_frame
            .lock()
            .expect("clock lease layout frame")
            .as_ref()
            .map_or(0, ClockLayoutFrame::rounds)
    }

    /// Lays out what the tick's samples left, in the layout frame the main thread handed the lease.
    /// Returns whether a round laid out, or `None` where the main thread has to.
    ///
    /// # Safety
    ///
    /// As for [`Self::run_tick`], with the main thread idle.
    unsafe fn lay_out(&self) -> Option<bool> {
        let mut frame = self.layout_frame.lock().expect("clock lease layout frame");
        let Some(frame) = frame.as_mut() else {
            // SAFETY: The caller owns the arena.
            let arena = unsafe { &*(self.arena as *const LayoutNodeArena) };
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
    unsafe fn present(&self) -> bool {
        if !self.presentable.load(Ordering::Acquire) {
            return false;
        }
        let Some(present) = PRESENT.get() else {
            return false;
        };
        // SAFETY: The caller owns the arena.
        let arena = unsafe { &*(self.arena as *const LayoutNodeArena) };
        {
            use crate::painting::record::damage::PaintDamage;
            for &(row, affects_hit_testing) in self.repaints.lock().expect("clock lease repaints").iter() {
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
        if !unsafe { crate::painting::ffi::record_for_clock_tick(self.arena as *mut c_void) } {
            return false;
        }
        if present(self.arena as *mut c_void) {
            return true;
        }
        // Nothing presents the recording; the main thread records the frame again.
        arena.recording().discard_pending_recording();
        false
    }

    fn take_entry(&self) -> Option<ClockTickEntry> {
        let mut entries = self.entries.lock().expect("clock lease entries");
        if entries.is_empty() {
            return None;
        }
        Some(entries.remove(0))
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

fn registry() -> &'static Mutex<HashMap<usize, Arc<ClockLease>>> {
    static LEASES: OnceLock<Mutex<HashMap<usize, Arc<ClockLease>>>> = OnceLock::new();
    LEASES.get_or_init(Mutex::default)
}

/// The live lease the render clock ticks for the compositor context `context`, if one holds it.
fn clock_lease_for_context(context: u64) -> Option<Arc<ClockLease>> {
    if context == 0 {
        return None;
    }
    registry()
        .lock()
        .expect("clock lease registry")
        .values()
        .find(|lease| lease.context == context && !lease.is_revoked())
        .cloned()
}

/// The live lease of the document whose layout arena is `arena`, if it holds one.
pub fn clock_lease_for(arena: usize) -> Option<Arc<ClockLease>> {
    registry()
        .lock()
        .expect("clock lease registry")
        .get(&arena)
        .filter(|lease| !lease.is_revoked())
        .cloned()
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

/// Grants the document whose layout arena is `arena` a lease over its document timeline, which the
/// style engine knows as `timeline_identity`, reading zero at `timeline_zero` and now at `time`,
/// until `deadline` (timeline times, ms). The render clock ticks it at the display ticks of the compositor context
/// `context`, unless that is 0. Replaces a lease the document held.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_grant(
    arena: *mut c_void,
    context: u64,
    timeline_identity: u32,
    timeline_zero: f64,
    time: f64,
    deadline: f64,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let lease = Arc::new(ClockLease {
        arena: arena as usize,
        context,
        timeline_identity,
        timeline_zero,
        deadline,
        time: AtomicU64::new(time.to_bits()),
        revoked: AtomicBool::new(false),
        paused: AtomicBool::new(false),
        targets: Mutex::default(),
        scroll_timelines: Mutex::default(),
        entries: Mutex::default(),
        layout_frame: Mutex::default(),
        repaints: Mutex::default(),
        presentable: AtomicBool::new(false),
        tick_started_fresh: AtomicBool::new(false),
        presented_since_adoption: AtomicBool::new(false),
        needs_main: AtomicBool::new(false),
        ticks_missed: AtomicU32::new(0),
        time_at_missed_tick: AtomicU64::new(f64::NAN.to_bits()),
        outcome: Mutex::default(),
    });
    if let Some(previous) = registry()
        .lock()
        .expect("clock lease registry")
        .insert(arena as usize, lease)
    {
        previous.revoked.store(true, Ordering::Release);
    }
}

/// Sets the elements the lease of `arena` ticks, with the records they hold now, and whether each
/// has a pseudo-element style that inherits from it outside its box.
///
/// # Safety
///
/// `style_nodes`, `style_records` and `pseudo_element_styles_outside_box` point at `count` values
/// each.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_clock_lease_set_targets(
    arena: *mut c_void,
    style_nodes: *const u32,
    style_records: *const u64,
    pseudo_element_styles_outside_box: *const bool,
    count: usize,
) {
    let Some(lease) = clock_lease_for(arena as usize) else {
        return;
    };
    let mut targets = lease.targets.lock().expect("clock lease targets");
    targets.clear();
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
}

/// Sets the scroll progress timelines whose animations the lease of `arena` ticks.
///
/// # Safety
///
/// `timelines` points at `count` values.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_clock_lease_set_scroll_timelines(
    arena: *mut c_void,
    timelines: *const FfiClockLeaseScrollTimeline,
    count: usize,
) {
    let Some(lease) = clock_lease_for(arena as usize) else {
        return;
    };
    let timelines = match count {
        0 => &[][..],
        // SAFETY: Guaranteed by the caller.
        _ => unsafe { std::slice::from_raw_parts(timelines, count) },
    };
    let mut leased = lease.scroll_timelines.lock().expect("clock lease scroll timelines");
    leased.clear();
    leased.extend(timelines.iter().map(|timeline| (*timeline, timeline.progress)));
}

/// The progress (percent) at which the last tick of the lease of `arena` sampled the scroll progress
/// timeline `identity`, or the host held it at the grant, or NaN.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_scroll_progress(arena: *mut c_void, identity: u32) -> f64 {
    clock_lease_for(arena as usize).map_or(f64::NAN, |lease| {
        lease
            .scroll_timelines
            .lock()
            .expect("clock lease scroll timelines")
            .iter()
            .find(|(timeline, _)| timeline.identity == identity)
            .map_or(f64::NAN, |(_, progress)| *progress)
    })
}

/// Ends the lease of the document whose layout arena is `arena`, if it holds one. What its last
/// tick left for the host to adopt goes with it.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_revoke(arena: *mut c_void) {
    let removed = registry()
        .lock()
        .expect("clock lease registry")
        .remove(&(arena as usize));
    if let Some(lease) = removed {
        lease.revoked.store(true, Ordering::Release);
        lease.entries.lock().expect("clock lease entries").clear();
    }
}

/// Has the render clock leave the lease of `arena` alone, or tick it again.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_set_paused(arena: *mut c_void, paused: bool) {
    if let Some(lease) = clock_lease_for(arena as usize) {
        lease.paused.store(paused, Ordering::Release);
    }
}

/// Hands the lease of `arena` the layout frame its render clock ticks lay out in. A frame the ticks
/// laid out in stays until the main thread takes it in.
pub(crate) fn set_clock_layout_frame(arena: *mut c_void, frame: ClockLayoutFrame) {
    if let Some(lease) = clock_lease_for(arena as usize) {
        let mut current = lease.layout_frame.lock().expect("clock lease layout frame");
        if !current.as_ref().is_some_and(ClockLayoutFrame::laid_out) {
            *current = Some(frame);
        }
    }
}

/// Takes the layout frame of the lease of `arena` if its ticks laid out in it.
pub(crate) fn take_laid_out_clock_layout_frame(arena: *mut c_void) -> Option<ClockLayoutFrame> {
    let lease = clock_lease_for(arena as usize)?;
    let mut frame = lease.layout_frame.lock().expect("clock lease layout frame");
    if !frame.as_ref().is_some_and(ClockLayoutFrame::laid_out) {
        return None;
    }
    frame.take()
}

pub(crate) fn clock_layout_frame_laid_out(arena: *mut c_void) -> bool {
    clock_lease_for(arena as usize).is_some_and(|lease| {
        lease
            .layout_frame
            .lock()
            .expect("clock lease layout frame")
            .as_ref()
            .is_some_and(ClockLayoutFrame::laid_out)
    })
}

/// Whether the document whose layout arena is `arena` holds a live lease.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_is_live(arena: *mut c_void) -> bool {
    clock_lease_for(arena as usize).is_some()
}

/// The timeline time of the lease's last tick, or NaN without a lease.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_time(arena: *mut c_void) -> f64 {
    clock_lease_for(arena as usize).map_or(f64::NAN, |lease| lease.time())
}

/// Submits a tick of the lease of `arena` at timeline time `time` as the stage `clock`, which owns
/// the arena and its style engine until the host takes it back. Returns false, having done
/// nothing, where the document holds no lease or the stage thread takes no `clock` stage.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread, with nothing in flight
/// that owns it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_clock_lease_submit_tick(arena: *mut c_void, time: f64) -> bool {
    if !rust_stage_thread_submits_clock() {
        return false;
    }
    let Some(lease) = clock_lease_for(arena as usize) else {
        return false;
    };
    *lease.outcome.lock().expect("clock lease outcome") = None;
    // The host shows what this tick installs itself.
    lease.presented_since_adoption.store(false, Ordering::Release);
    // The main thread's pin table is its own until it has taken the tick back, as it is beside a style
    // pass in flight.
    // SAFETY: Guaranteed by the caller: the main thread owns the engine until the submit below.
    let engine = unsafe { LayoutNodeArena::from_handle(arena) }.style_engine_handle();
    if !engine.is_null() {
        // SAFETY: As above.
        unsafe { engine.enter("clock tick submission") }.begin_clock_lend_beside_host_pins();
    }
    // The tick samples the engine, and takes its token along.
    let (loan, settlement) = (!engine.is_null())
        .then(|| engine.lend(Holder::LayoutPass, Owed::TakeBack))
        .unzip();
    let tick = move || {
        // SAFETY: The stage owns the arena, as below.
        let run_tick = || unsafe { lease.run_tick(time, lease.deadline()) };
        let mut loan = loan;
        match loan.as_mut() {
            Some(loan) => loan.lend_to_this_thread(|engine| {
                // The main thread pins and unpins its host's records beside the tick, which may read
                // none of them.
                debug_assert!(
                    !engine.reads_host_style_record_pins(),
                    "a clock tick beside the main thread reads the host's style-record pins"
                );
                run_tick()
            }),
            None => run_tick(),
        };
    };
    let taken_back = move || {
        if let Some(settlement) = settlement {
            settlement.settle();
            // SAFETY: The main thread owns the engine again, which outlives the tick of its arena.
            unsafe { engine.enter("clock tick take-back") }.finish_clock_lend_beside_host_pins();
        }
    };
    // SAFETY: The stage reaches the arena and its engine only, which the `clock` stage owns until
    // the main thread takes it back, and every main-thread path to either joins it first.
    unsafe { crate::stage_thread::submit_stage_with_take_back("clock", arena, tick, taken_back) };
    true
}

/// How the last tick of the lease of `arena` ended; `Revoked` without a lease.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_tick_outcome(arena: *mut c_void) -> FfiClockTickOutcome {
    let lease = registry()
        .lock()
        .expect("clock lease registry")
        .get(&(arena as usize))
        .cloned();
    lease
        .and_then(|lease| *lease.outcome.lock().expect("clock lease outcome"))
        .unwrap_or(FfiClockTickOutcome::Revoked)
}

/// Whether the render side presented every tick of the lease of `arena` the host has not adopted
/// yet: adopting them repaints nothing.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_presented_since_adoption(arena: *mut c_void) -> bool {
    clock_lease_for(arena as usize).is_some_and(|lease| lease.presented_since_adoption.load(Ordering::Acquire))
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

/// Takes the next entry the last tick of the lease of `arena` left for the host to adopt.
pub(crate) fn take_clock_tick_entry(arena: *mut c_void) -> Option<ClockTickEntry> {
    let lease = registry()
        .lock()
        .expect("clock lease registry")
        .get(&(arena as usize))
        .cloned()?;
    lease.take_entry()
}

/// Drops what the host did not adopt of the records the lease's last tick installed in the arena
/// of `arena` ahead of it, with their pins: the rows of elements that left the document before the
/// host could adopt their samples.
///
/// # Safety
///
/// `arena` is the live layout arena of a document on the main thread, with the tick taken back.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_clock_lease_drop_unadopted(arena: *mut c_void) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    // SAFETY: Guaranteed by the caller.
    unsafe { LayoutNodeArena::from_handle(arena) }.drop_animation_adoptions();
}

// The render clock: display ticks delivered to a thread of their own, which hands them to the stage
// thread for the lease of their compositor context while the main thread is idle.

/// Who may reach the arenas of leased documents now.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ArenaHolder {
    /// The main thread: it is awake, or it has a frame in flight.
    Main,
    /// Nobody: the main thread is blocked in its outermost event loop, and a render clock tick may
    /// start. The tick acts for the thread named.
    Idle(ThreadId),
    /// A render clock tick, which the main thread waits for when it wakes.
    Tick,
}

struct IdleGate {
    holder: Mutex<ArenaHolder>,
    tick_ended: Condvar,
}

fn idle_gate() -> &'static IdleGate {
    static GATE: OnceLock<IdleGate> = OnceLock::new();
    GATE.get_or_init(|| IdleGate {
        holder: Mutex::new(ArenaHolder::Main),
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
        let mut holder = idle_gate().holder.lock().expect("render clock idle gate");
        let caller = match *holder {
            ArenaHolder::Idle(caller) => caller,
            ArenaHolder::Tick => return Err(false),
            ArenaHolder::Main => return Err(true),
        };
        *holder = ArenaHolder::Tick;
        Ok(Self { caller })
    }
}

impl Drop for IdleTick {
    fn drop(&mut self) {
        let gate = idle_gate();
        let mut holder = gate.holder.lock().expect("render clock idle gate");
        if *holder == ArenaHolder::Tick {
            *holder = ArenaHolder::Idle(self.caller);
        }
        drop(holder);
        gate.tick_ended.notify_all();
    }
}

// Whether a render clock tick installed something since the main thread last woke.
static TICKS_TO_ADOPT: AtomicBool = AtomicBool::new(false);

/// The main thread is about to block in its outermost event loop. Unless it has a frame in flight,
/// render clock ticks may reach the arenas of leased documents until it wakes.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_main_will_idle() {
    if !enabled() || crate::stage_thread::has_frame_in_flight() {
        return;
    }
    let caller = std::thread::current().id();
    let mut holder = idle_gate().holder.lock().expect("render clock idle gate");
    if *holder == ArenaHolder::Main {
        *holder = ArenaHolder::Idle(caller);
    }
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
    let mut holder = gate.holder.lock().expect("render clock idle gate");
    // A tick a test injected while the main thread idled runs before it takes the arenas back.
    while *holder == ArenaHolder::Tick
        || (matches!(*holder, ArenaHolder::Idle(_)) && INJECTED_TICKS_PENDING.load(Ordering::Acquire) > 0)
    {
        holder = gate.tick_ended.wait(holder).expect("render clock idle gate");
    }
    *holder = ArenaHolder::Main;
}

// The ticks a test injected that the stage thread has not run yet.
static INJECTED_TICKS_PENDING: AtomicUsize = AtomicUsize::new(0);

/// Wakes the main thread from its event loop. Runs on the stage thread.
static WAKE_MAIN: OnceLock<extern "C" fn()> = OnceLock::new();

/// Has the render clock call `wake_main()` on the stage thread where a tick a test injected ended, for the main
/// thread to go on with what waits for it, and where the ticks laid out many rounds for an idle main thread to take
/// in. The first one set stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_set_wake_main(wake_main: extern "C" fn()) {
    let _ = WAKE_MAIN.set(wake_main);
}

/// Hands the stage thread a display tick at `frame_time_nanoseconds` (monotonic time) for the compositor context
/// `context`, as the render clock hands it one, for a test that drives the ticks itself. Call it where the main thread
/// has just let render clock ticks in (see [`rust_render_clock_main_will_idle`]): the main thread waits for the tick
/// before it takes the arenas back, and the tick wakes it when it ends. Returns false where the stage thread is gone.
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
    // SAFETY: Guaranteed by the caller.
    let sender = unsafe { &mut *sender };
    INJECTED_TICKS_PENDING.fetch_add(1, Ordering::AcqRel);
    let sent = sender.jobs.send(move || {
        run_render_clock_tick_at(context, frame_time_nanoseconds);
        end_injected_tick();
    });
    if !sent {
        end_injected_tick();
    }
    sent
}

fn end_injected_tick() {
    let gate = idle_gate();
    {
        let _holder = gate.holder.lock().expect("render clock idle gate");
        INJECTED_TICKS_PENDING.fetch_sub(1, Ordering::AcqRel);
    }
    gate.tick_ended.notify_all();
    if let Some(wake_main) = WAKE_MAIN.get() {
        wake_main();
    }
}

/// Where the render clock's display ticks wait for the stage thread: one per compositor context.
/// Ticks that arrive while one waits fold into it, with the latest time.
struct ClockSlot {
    frame_time_nanoseconds: AtomicI64,
    queued: AtomicBool,
}

/// How many rounds the ticks lay out in one layout frame before they wake an idle main thread to
/// take it in: about two seconds of display ticks.
const MAX_CLOCK_ROUNDS_OWED: u32 = 120;

/// How many slots a render clock keeps before it lets go of those no tick waits in.
const MAX_IDLE_CLOCK_SLOTS: usize = 16;

/// The render clock's way onto the stage thread. Owned by the render clock thread.
pub struct ClockSender {
    jobs: crate::stage_thread::DetachedJobSender,
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
    /// Ticks that ran inside a stage the main thread submitted or waits for, and were dropped.
    pub ticks_dropped_nested: u64,
    /// Ticks that found the main thread holding the arenas, and were dropped.
    pub ticks_dropped_main_busy: u64,
    /// Ticks for a context no live lease holds.
    pub ticks_dropped_without_lease: u64,
    /// Ticks for a lease the main thread moved what it ticks of.
    pub ticks_dropped_paused: u64,
    /// Ticks for a lease an earlier tick ended, which waits for the main thread.
    pub ticks_dropped_needing_main: u64,
    /// Ticks at a time no later than the lease's last.
    pub ticks_dropped_stale: u64,
    /// Ticks that installed their samples for the main thread to adopt.
    pub ticks_installed: u64,
    /// Ticks after which a layout frame on the render side held what the main thread takes in.
    pub ticks_laid_out: u64,
    /// Ticks the render side presented, without the main thread.
    pub ticks_presented: u64,
    /// Ticks that ended their lease: past its deadline, or with a sample only the main thread takes.
    pub ticks_needing_main: u64,
    /// Ticks whose layout moved the visual contexts, which ended their lease.
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
    ticks_dropped_nested: AtomicU64,
    ticks_dropped_main_busy: AtomicU64,
    ticks_dropped_without_lease: AtomicU64,
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
    ticks_dropped_nested: AtomicU64::new(0),
    ticks_dropped_main_busy: AtomicU64::new(0),
    ticks_dropped_without_lease: AtomicU64::new(0),
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
/// it and hands the frame to the compositor. Runs on the stage thread, with the main thread idle.
static PRESENT: OnceLock<extern "C" fn(*mut c_void) -> bool> = OnceLock::new();

/// Has the render clock call `present(arena)` to present what a tick recorded. The first one set
/// stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_set_present(present: extern "C" fn(*mut c_void) -> bool) {
    let _ = PRESENT.set(present);
}

/// What the main thread does for a lease a render clock tick ended. Runs on the stage thread.
static NEEDS_MAIN: OnceLock<extern "C" fn(u64)> = OnceLock::new();

/// Has the render clock call `needs_main(context)` on the stage thread where a tick ended the lease
/// of `context`: the main thread's rendering update takes over there. The first one set stays.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_set_needs_main(needs_main: extern "C" fn(u64)) {
    let _ = NEEDS_MAIN.set(needs_main);
}

/// A render clock's way onto the stage thread, or null where there is none to tick leases on (the
/// stages do not overlap, or clock frames are off). The render clock thread owns it, and destroys
/// it with [`rust_render_clock_sender_destroy`].
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_sender_create() -> *mut ClockSender {
    if !enabled() {
        return std::ptr::null_mut();
    }
    let Some(jobs) = crate::stage_thread::detached_job_sender() else {
        return std::ptr::null_mut();
    };
    Box::into_raw(Box::new(ClockSender {
        jobs,
        slots: HashMap::new(),
    }))
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
/// `context` to the stage thread, which ticks the lease of that context with it if the main thread
/// is idle then. Where a tick for the context is still waiting there, it takes this time instead.
/// Returns false where the stage thread is gone, which it only is when the process is.
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
    // A context whose tick no job holds any more gets a slot again if it ticks again: the slots of
    // contexts that went away go.
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
    sender.jobs.send(move || run_render_clock_tick(context, &slot))
}

/// Runs a display tick for the lease of `context` on the stage thread.
fn run_render_clock_tick(context: u64, slot: &ClockSlot) {
    // A read-modify-write, which a later tick's post orders against: either that post finds the slot
    // free and posts a tick of its own, or the time it stored is the one read here.
    slot.queued.swap(false, Ordering::AcqRel);
    let frame_time_nanoseconds = slot.frame_time_nanoseconds.load(Ordering::Acquire);
    run_render_clock_tick_at(context, frame_time_nanoseconds);
}

/// Runs the display tick at `frame_time_nanoseconds` for the lease of `context` on the stage thread.
fn run_render_clock_tick_at(context: u64, frame_time_nanoseconds: i64) {
    count(&COUNTERS.ticks_run);
    // The stage thread runs any job while a stage it runs waits for a join, which may be this one:
    // the main thread is not idle then, and the stage owns what the tick would reach.
    if crate::stage_thread::running_inside_stage() {
        count(&COUNTERS.ticks_dropped_nested);
        return;
    }
    let Some(lease) = clock_lease_for_context(context) else {
        count(&COUNTERS.ticks_dropped_without_lease);
        return;
    };
    let idle_tick = match IdleTick::begin() {
        Ok(idle_tick) => idle_tick,
        Err(main_holds_arena) => {
            count(&COUNTERS.ticks_dropped_main_busy);
            if main_holds_arena {
                miss_tick(context, &lease);
            }
            return;
        }
    };
    // Only the main thread grants and revokes, and it holds none of the lease's arena now: the lease
    // stays as it is found, and so does its arena, which revoking it takes back first.
    if lease.is_revoked() {
        count(&COUNTERS.ticks_dropped_without_lease);
        return;
    }
    if lease.paused.load(Ordering::Acquire) {
        count(&COUNTERS.ticks_dropped_paused);
        drop(idle_tick);
        miss_tick(context, &lease);
        return;
    }
    if lease.needs_main.load(Ordering::Acquire) {
        count(&COUNTERS.ticks_dropped_needing_main);
        return;
    }
    lease.ticks_missed.store(0, Ordering::Release);
    let time = lease.timeline_time_at(frame_time_nanoseconds as f64 / 1.0e6);
    if time.partial_cmp(&lease.time()) != Some(std::cmp::Ordering::Greater) {
        count(&COUNTERS.ticks_dropped_stale);
        return;
    }
    let mut tick = None;
    crate::stage_thread::run_detached_for(idle_tick.caller, lease.arena, || {
        let run_tick = || {
            // SAFETY: The main thread is idle with nothing in flight, and waits for this tick when
            // it wakes: the stage thread owns the arena and its engine until `idle_tick` is dropped.
            let outcome = unsafe { lease.run_tick(time, lease.deadline()) };
            if outcome != FfiClockTickOutcome::Presented {
                return (outcome, false, false);
            }
            // A sample the arena did not take, the host installs over the record the target held
            // before: only its entry keeps that record alive, and no later tick may sample over it.
            if lease
                .entries
                .lock()
                .expect("clock lease entries")
                .iter()
                .any(|entry| !entry.installed_in_arena)
            {
                return (FfiClockTickOutcome::NeedsMain, false, false);
            }
            // SAFETY: As above.
            let Some(laid_out) = (unsafe { lease.lay_out() }) else {
                return (FfiClockTickOutcome::NeedsMain, false, false);
            };
            // A round that moved a box that owns a clip, a transform or a scroll frame moved the
            // visual contexts, which the compositor has from the main thread's frames.
            // SAFETY: As above.
            if laid_out
                && !unsafe { crate::painting::ffi::settle_visual_contexts_for_clock_tick(lease.arena as *mut c_void) }
            {
                count(&COUNTERS.ticks_moving_visual_contexts);
                return (FfiClockTickOutcome::NeedsMain, laid_out, false);
            }
            // A tick that moved nothing shows nothing new.
            let moved_nothing = !laid_out && lease.repaints.lock().is_ok_and(|repaints| repaints.is_empty());
            // SAFETY: As above.
            if !moved_nothing && !unsafe { lease.present() } {
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
        // The lease ends at the host, which adopts what the ticks left first.
        *lease.outcome.lock().expect("clock lease outcome") = Some(outcome);
        lease.needs_main.store(true, Ordering::Release);
    }
    if lease.tick_started_fresh.load(Ordering::Acquire) {
        lease.presented_since_adoption.store(presented, Ordering::Release);
    } else if !presented {
        lease.presented_since_adoption.store(false, Ordering::Release);
    }
    if lease.entries.lock().is_ok_and(|entries| !entries.is_empty()) {
        TICKS_TO_ADOPT.store(true, Ordering::Release);
    }
    let rounds_owed = lease.rounds_owed();
    drop(idle_tick);
    if outcome == FfiClockTickOutcome::Presented {
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

/// How many display ticks in a row a lease may miss before the main thread's rendering updates tick
/// it: a task that holds the arena for a frame or two is over before anyone sees the difference.
const MISSED_TICKS_BEFORE_MAIN: u32 = 2;

/// Notes a display tick the render clock could not run for `lease`: the main thread held its arena
/// or had paused it. Nothing else has the main thread render, so one that keeps missing them asks
/// it for a rendering update at every display tick until the render clock runs one again.
fn miss_tick(context: u64, lease: &ClockLease) {
    // A rendering update that ticked the lease since the last miss ends the run: one the render
    // clock's tick ran into is no reason to ask for another.
    let time = lease.time.load(Ordering::Acquire);
    let missed = if lease.time_at_missed_tick.swap(time, Ordering::AcqRel) == time {
        lease.ticks_missed.load(Ordering::Acquire).saturating_add(1)
    } else {
        1
    };
    lease.ticks_missed.store(missed, Ordering::Release);
    if missed < MISSED_TICKS_BEFORE_MAIN {
        return;
    }
    count(&COUNTERS.ticks_missed_asking_main);
    if let Some(needs_main) = NEEDS_MAIN.get() {
        needs_main(context);
    }
}

/// Whether the render clock missed so many display ticks of the lease of `arena` in a row that the
/// main thread's rendering updates tick it in its place.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_misses_ticks(arena: *mut c_void) -> bool {
    clock_lease_for(arena as usize).is_some_and(|lease| {
        lease.ticks_missed.load(Ordering::Acquire) >= MISSED_TICKS_BEFORE_MAIN
            && lease.time_at_missed_tick.load(Ordering::Acquire) == lease.time.load(Ordering::Acquire)
    })
}

/// What the render clock did with the display ticks it was handed.
#[unsafe(no_mangle)]
pub extern "C" fn rust_render_clock_counters() -> FfiRenderClockCounters {
    let load = |counter: &AtomicU64| counter.load(Ordering::Relaxed);
    FfiRenderClockCounters {
        ticks_posted: load(&COUNTERS.ticks_posted),
        ticks_folded: load(&COUNTERS.ticks_folded),
        ticks_run: load(&COUNTERS.ticks_run),
        ticks_dropped_nested: load(&COUNTERS.ticks_dropped_nested),
        ticks_dropped_main_busy: load(&COUNTERS.ticks_dropped_main_busy),
        ticks_dropped_without_lease: load(&COUNTERS.ticks_dropped_without_lease),
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
    fn idle_gate_lets_a_tick_in_only_while_the_main_thread_is_idle_and_waits_for_it() {
        take_arenas_back();
        assert_eq!(IdleTick::begin().err(), Some(true), "the main thread holds the arenas");
        *idle_gate().holder.lock().unwrap() = ArenaHolder::Idle(std::thread::current().id());
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
    }
}
