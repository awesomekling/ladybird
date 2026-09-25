/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! Clock frames (`LIBWEB_RENDER_CLOCK_FRAMES`): the animations of a document ticked on the render
//! side under a lease the main thread grants.
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

use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::css::style::StyleEngine;
use crate::css::style::bridge::{FfiRowSampledInPass, sample_installed_record_for_clock_tick};
use crate::css::style::tree::StyleNodeID;
use crate::layout::LayoutNodeArena;

/// Whether clock frames are on: `LIBWEB_RENDER_CLOCK_FRAMES` set to anything but empty or `0`.
pub(crate) fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        std::env::var("LIBWEB_RENDER_CLOCK_FRAMES").is_ok_and(|value| !value.is_empty() && value != "0")
    })
}

/// An element whose animations a lease ticks, with the record it holds.
#[derive(Clone, Copy)]
struct ClockTarget {
    style_node: StyleNodeID,
    style_record: u64,
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
    targets: Mutex<Vec<ClockTarget>>,
    entries: Mutex<Vec<ClockTickEntry>>,
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
    /// `style_engine_clock_tick_take_entry`).
    ///
    /// # Safety
    ///
    /// On the thread that owns the lease's arena and its style engine: the stage thread inside the
    /// `clock` stage the host submitted, or the main thread with nothing in flight.
    pub unsafe fn run_tick(&self, time: f64) -> FfiClockTickOutcome {
        let outcome = if self.is_revoked() {
            FfiClockTickOutcome::Revoked
        } else if time >= self.deadline {
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
        let engine = arena.style_engine_handle().cast::<StyleEngine>();
        if engine.is_null() {
            return FfiClockTickOutcome::NeedsMain;
        }
        // SAFETY: The caller owns the engine; no other borrow of it is live here.
        let samples = unsafe { &*engine }
            .animation_timeline_samples()
            .with_time(self.timeline_identity, time);
        let mut outcome = FfiClockTickOutcome::Presented;
        let mut targets = self.targets.lock().expect("clock lease targets");
        let mut entries = self.entries.lock().expect("clock lease entries");
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
            let level = sample.invalidation.invalidation & 0x3;
            let needs_layout_tree_rebuild = level >= 3;
            let needs_relayout = level >= 2;
            let installed_in_arena = !needs_layout_tree_rebuild
                && arena.install_animation_sample(target.style_node, sample.style_record, needs_relayout);
            entries.push(ClockTickEntry {
                style_node: target.style_node,
                style_record_before: target.style_record,
                sample,
                installed_in_arena,
            });
            target.style_record = entries.last().expect("just pushed").sample.style_record;
        }
        outcome
    }

    fn take_entry(&self) -> Option<ClockTickEntry> {
        let mut entries = self.entries.lock().expect("clock lease entries");
        if entries.is_empty() {
            return None;
        }
        Some(entries.remove(0))
    }
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
/// until `deadline` (timeline times, ms). Replaces a lease the document held.
#[unsafe(no_mangle)]
pub extern "C" fn rust_clock_lease_grant(
    arena: *mut c_void,
    timeline_identity: u32,
    timeline_zero: f64,
    time: f64,
    deadline: f64,
) {
    assert!(!arena.is_null(), "layout node arena handle is null");
    let lease = Arc::new(ClockLease {
        arena: arena as usize,
        timeline_identity,
        timeline_zero,
        deadline,
        time: AtomicU64::new(time.to_bits()),
        revoked: AtomicBool::new(false),
        targets: Mutex::default(),
        entries: Mutex::default(),
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

/// Sets the elements the lease of `arena` ticks, with the records they hold now.
///
/// # Safety
///
/// `style_nodes` and `style_records` point at `count` values each.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn rust_clock_lease_set_targets(
    arena: *mut c_void,
    style_nodes: *const u32,
    style_records: *const u64,
    count: usize,
) {
    let Some(lease) = clock_lease_for(arena as usize) else {
        return;
    };
    let mut targets = lease.targets.lock().expect("clock lease targets");
    targets.clear();
    for index in 0..count {
        // SAFETY: Guaranteed by the caller.
        let (node, record) = unsafe { (*style_nodes.add(index), *style_records.add(index)) };
        if let Some(style_node) = StyleNodeID::from_raw(node) {
            targets.push(ClockTarget {
                style_node,
                style_record: record,
            });
        }
    }
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
    let tick = move || {
        // SAFETY: The stage owns the arena, as below.
        unsafe { lease.run_tick(time) };
    };
    // SAFETY: The stage reaches the arena and its engine only, which the `clock` stage owns until
    // the main thread takes it back, and every main-thread path to either joins it first.
    unsafe { crate::stage_thread::submit_stage("clock", arena, tick) };
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
