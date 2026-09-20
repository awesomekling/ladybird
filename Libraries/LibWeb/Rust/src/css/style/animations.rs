/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The CSS animations an element owns, mirrored for the style stage.
//!
//! Reconciling an element's `CSSAnimation` objects against its freshly computed `animation-*`
//! longhands used to be a host call, because only the host could say which animations the element
//! already owned. The names of those animations are published here instead, so the style
//! computation decides the reconciliation from its own inputs and hands the host a plan.
//!
//! Only the names are mirrored: matching an animation is matching its name, and everything the
//! host does once a definition has found its animation - applying the timing, resolving the
//! keyframes, cancelling what no definition claimed - it does from the list it already holds.

use super::tree::{StyleNodeID, TreeScopeID};
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::css_string::CssString;
use crate::css::host_shared::SharedPayload;
use std::collections::HashMap;

/// Which of an element's animation lists a row belongs to, in the host's own numbering: zero for
/// the element itself, and the pseudo-element's value plus one for each pseudo-element.
pub(crate) type AnimationSlot = u8;

/// The definition that claimed no existing animation and asks for a new one.
pub(crate) const NO_MATCHED_ANIMATION: i32 = -1;

/// Per element and pseudo-element, the names of the CSS animations the host holds for it, in the
/// order the host holds them.
#[derive(Default)]
pub(crate) struct CssDefinedAnimations {
    /// Owning a CSS animation is rare, so only the elements that do have a row.
    rows: HashMap<(StyleNodeID, AnimationSlot), Box<[CssString]>>,
}

impl CssDefinedAnimations {
    /// Replace one list. An empty list drops the row, so an element that stops animating stops
    /// costing anything.
    pub(crate) fn set(&mut self, node: StyleNodeID, slot: AnimationSlot, names: Box<[CssString]>) {
        if names.is_empty() {
            self.rows.remove(&(node, slot));
            return;
        }
        self.rows.insert((node, slot), names);
    }

    #[must_use]
    pub(crate) fn names(&self, node: StyleNodeID, slot: AnimationSlot) -> &[CssString] {
        self.rows.get(&(node, slot)).map_or(&[][..], |names| &names[..])
    }

    /// Give up the rows of identities that have been retired. An identity can be minted again for
    /// another element, so a row left behind would be read as that element's.
    pub(crate) fn retire(&mut self, nodes: &[StyleNodeID]) {
        if self.rows.is_empty() {
            return;
        }
        self.rows.retain(|&(node, _), _| !nodes.contains(&node));
    }
}

/// Match newly computed animation definitions against the animations the element already owns.
///
/// <https://drafts.csswg.org/css-animations-1/#animations>: the new list is walked last to first,
/// and each definition takes the last existing animation of the same name that no later definition
/// has taken. Returns, per definition, the index of the animation it took, or
/// `NO_MATCHED_ANIMATION` where a new one has to be created.
#[must_use]
pub(crate) fn match_existing_animations(existing: &[CssString], definition_names: &[CssString]) -> Vec<i32> {
    let mut matches = vec![NO_MATCHED_ANIMATION; definition_names.len()];
    if existing.is_empty() {
        return matches;
    }
    // The same `@keyframes` name may repeat within one `animation-name`, so an animation that has
    // been claimed may not be claimed twice.
    let mut claimed = vec![false; existing.len()];
    for (index, name) in definition_names.iter().enumerate().rev() {
        for candidate in (0..existing.len()).rev() {
            if !claimed[candidate] && existing[candidate] == *name {
                claimed[candidate] = true;
                matches[index] = i32::try_from(candidate).expect("an element cannot own that many animations");
                break;
            }
        }
    }
    matches
}

/// Whether an element's published style record computed `display: none`, as the value stood before
/// any animation overlay was layered on it. The base payloads are exactly what
/// `ComputedValues::base_values()` exposes, so an element animating its own `display` answers with
/// the value its animations are running against.
#[must_use]
fn published_base_display_is_none(engine: &super::StyleEngine, node: StyleNodeID) -> bool {
    // No record names a node that is not an element - a shadow root, the document - and one whose
    // identity has been retired or whose style has not reached it yet.
    let Some((record, _)) = engine.element_published_style_record(node) else {
        return false;
    };
    let Some(view) = engine.style_record_view(record) else {
        return false;
    };
    let payloads = match view.base_payloads.is_empty() {
        true => view.payloads,
        false => view.base_payloads,
    };
    if payloads.is_empty() {
        return false;
    }
    ComputedValuesView::new(SharedPayload::as_pointer_slice(payloads))
        .display()
        .is_none()
}

/// Whether `node` or one of its inclusive ancestors is `display: none`, ignoring animations.
///
/// A mirror of `Node::has_inclusive_ancestor_with_display_none_ignoring_animations()`. The walk
/// climbs `parent_or_shadow_host()`, which is the tree the element is *in*: a slotted element
/// continues through its light-DOM parent, not through the slot it is assigned to, and a shadow
/// root continues through its host. Nodes that are not elements hold no record and are skipped,
/// the way the host's walk skips them.
#[must_use]
pub(crate) fn has_inclusive_ancestor_with_display_none_ignoring_animations(
    engine: &super::StyleEngine,
    node: StyleNodeID,
) -> bool {
    let mut current = Some(node);
    while let Some(ancestor) = current {
        if published_base_display_is_none(engine, ancestor) {
            return true;
        }
        current = engine
            .tree()
            .parent(ancestor)
            .or_else(|| engine.tree().host_of(ancestor));
    }
    false
}

/// A `Animations::TimeValue`, mirrored. A time is either a duration or a proportion of a
/// progress-based timeline, and the host's arithmetic on two times of different kinds is a
/// `VERIFY` failure, so the mirror refuses to decide rather than reproducing a crash.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TimeValue {
    pub(crate) is_percentage: bool,
    pub(crate) value: f64,
}

impl TimeValue {
    /// The zero of the kind the timeline measures in, which is what `TimeValue::create_zero()`
    /// returns.
    #[must_use]
    fn zero(is_percentage: bool) -> Self {
        Self {
            is_percentage,
            value: 0.0,
        }
    }

    /// Two times may be combined when they measure the same thing. A zero measures nothing, so the
    /// host's `VERIFY` cannot fire on it whichever kind it was built with.
    #[must_use]
    fn agrees_with(self, other: Self) -> bool {
        self.is_percentage == other.is_percentage || self.value == 0.0 || other.value == 0.0
    }

    #[must_use]
    fn add(self, other: Self) -> Option<Self> {
        self.agrees_with(other).then_some(Self {
            is_percentage: self.is_percentage || other.is_percentage,
            value: self.value + other.value,
        })
    }

    #[must_use]
    fn subtract(self, other: Self) -> Option<Self> {
        self.agrees_with(other).then_some(Self {
            is_percentage: self.is_percentage || other.is_percentage,
            value: self.value - other.value,
        })
    }

    #[must_use]
    fn scale(self, factor: f64) -> Self {
        Self {
            is_percentage: self.is_percentage,
            value: self.value * factor,
        }
    }

    /// `operator<=>`, which is also where the host verifies that two times are comparable.
    #[must_use]
    fn compare(self, other: Self) -> Option<std::cmp::Ordering> {
        if !self.agrees_with(other) {
            return None;
        }
        Some(self.value.total_cmp(&other.value))
    }

    #[must_use]
    fn largest(self, other: Self) -> Option<Self> {
        Some(match self.compare(other)? {
            std::cmp::Ordering::Less => other,
            _ => self,
        })
    }

    #[must_use]
    fn smallest(self, other: Self) -> Option<Self> {
        Some(match self.compare(other)? {
            std::cmp::Ordering::Greater => other,
            _ => self,
        })
    }
}

/// How the host packs one animation's timing into the two buffers a row travels in. The flags are
/// the presence and kind bits; the eight words are the times and rates, in `WORD_*` order.
pub(crate) mod timing_row_flag {
    pub(crate) const HAS_START_TIME: u32 = 1 << 0;
    pub(crate) const START_TIME_IS_PERCENTAGE: u32 = 1 << 1;
    pub(crate) const HAS_HOLD_TIME: u32 = 1 << 2;
    pub(crate) const HOLD_TIME_IS_PERCENTAGE: u32 = 1 << 3;
    pub(crate) const START_DELAY_IS_PERCENTAGE: u32 = 1 << 4;
    pub(crate) const END_DELAY_IS_PERCENTAGE: u32 = 1 << 5;
    pub(crate) const ITERATION_DURATION_IS_PERCENTAGE: u32 = 1 << 6;
    pub(crate) const HAS_PENDING_PLAYBACK_RATE: u32 = 1 << 7;
    pub(crate) const HAS_PENDING_PLAY_TASK: u32 = 1 << 8;
    pub(crate) const HAS_PENDING_PAUSE_TASK: u32 = 1 << 9;
    pub(crate) const IS_FINISHED: u32 = 1 << 10;
    pub(crate) const REPLACE_STATE_IS_REMOVED: u32 = 1 << 11;
    pub(crate) const HAS_TIMELINE: u32 = 1 << 12;
    pub(crate) const TIMELINE_IS_MONOTONICALLY_INCREASING: u32 = 1 << 13;
    pub(crate) const TIMELINE_IS_PROGRESS_BASED: u32 = 1 << 14;
    pub(crate) const FILL_MODE_SHIFT: u32 = 15;
    pub(crate) const FILL_MODE_MASK: u32 = 0b111;
    /// The host could not describe this animation - an effect whose local time is overridden for
    /// observation, for instance - so the mirror must not answer for it.
    pub(crate) const UNDECIDABLE: u32 = 1 << 18;
    /// `Bindings::PlaybackDirection`, in IDL order.
    pub(crate) const PLAYBACK_DIRECTION_SHIFT: u32 = 19;
    pub(crate) const PLAYBACK_DIRECTION_MASK: u32 = 0b11;
    /// The effect's own easing: 0 the identity `linear`, 1 `cubic-bezier()`, 2 `steps()`.
    pub(crate) const EASING_KIND_SHIFT: u32 = 21;
    pub(crate) const EASING_KIND_MASK: u32 = 0b11;
    pub(crate) const EASING_STEP_POSITION_SHIFT: u32 = 23;
    pub(crate) const EASING_STEP_POSITION_MASK: u32 = 0b111;
    /// A `linear()` easing that has control points of its own, which the row has no room to spell
    /// out. The mirror declines the key rather than answering with the identity curve.
    pub(crate) const EASING_HAS_CONTROL_POINTS: u32 = 1 << 26;
}

/// `Bindings::PlaybackDirection`, in IDL order.
mod playback_direction {
    pub(super) const NORMAL: u32 = 0;
    pub(super) const REVERSE: u32 = 1;
    pub(super) const ALTERNATE_REVERSE: u32 = 3;
}

/// `Bindings::FillMode`, in IDL order.
mod fill_mode {
    pub(super) const FORWARDS: u32 = 1;
    pub(super) const BACKWARDS: u32 = 2;
    pub(super) const BOTH: u32 = 3;
}

/// How many words of each buffer one row occupies.
pub(crate) const TIMING_ROW_WORDS: usize = 5;
pub(crate) const TIMING_ROW_TIMES: usize = 13;

const WORD_FLAGS: usize = 0;
const WORD_TIMELINE: usize = 1;
const WORD_EASING_INTERVAL_COUNT: usize = 2;
const WORD_EFFECT_IDENTITY_LOW: usize = 3;
const WORD_EFFECT_IDENTITY_HIGH: usize = 4;

const TIME_START: usize = 0;
const TIME_HOLD: usize = 1;
const TIME_START_DELAY: usize = 2;
const TIME_END_DELAY: usize = 3;
const TIME_ITERATION_DURATION: usize = 4;
const TIME_PLAYBACK_RATE: usize = 5;
const TIME_PENDING_PLAYBACK_RATE: usize = 6;
const TIME_ITERATION_COUNT: usize = 7;
const TIME_ITERATION_START: usize = 8;
const TIME_EASING_X1: usize = 9;
const TIME_EASING_Y1: usize = 10;
const TIME_EASING_X2: usize = 11;
const TIME_EASING_Y2: usize = 12;

/// One animation's timing, as the host held it when the style update began.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AnimationTimingRow {
    flags: u32,
    timeline: u32,
    easing_interval_count: i32,
    effect_identity: u64,
    times: [f64; TIMING_ROW_TIMES],
}

impl AnimationTimingRow {
    #[must_use]
    fn has(&self, flag: u32) -> bool {
        self.flags & flag != 0
    }

    #[must_use]
    fn time(&self, index: usize, percentage_flag: u32) -> TimeValue {
        TimeValue {
            is_percentage: self.has(percentage_flag),
            value: self.times[index],
        }
    }
}

/// `Animations::AnimationPlayState`, as far as relevance needs it.
#[derive(Clone, Copy, PartialEq)]
enum PlayState {
    Idle,
    Paused,
    Finished,
    Running,
}

/// `AnimationEffect::Phase`.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Before,
    Active,
    After,
    Idle,
}

/// Whether the animation this row describes is relevant, at `timeline_time`.
///
/// A mirror of `Animation::is_relevant()` and everything under it:
/// `AnimationEffect::is_current()`, `is_in_effect()`, `phase()` and `Animation::play_state()`.
/// `None` means the mirror declines to answer - a row the host marked undecidable, a timeline it
/// did not publish a sample for, or times whose kinds the host's arithmetic would refuse to mix -
/// and the caller must ask the host instead.
#[must_use]
pub(crate) fn row_is_relevant(row: &AnimationTimingRow, timeline_time: Option<TimeValue>) -> Option<bool> {
    use timing_row_flag as flag;

    if row.has(flag::UNDECIDABLE) {
        return None;
    }

    // An animation is relevant if its associated effect is current or in effect, and its replace
    // state is not removed. Rows exist only for animations that have a keyframe effect.
    if row.has(flag::REPLACE_STATE_IS_REMOVED) {
        return Some(false);
    }

    let timing = resolve_timing(row, timeline_time)?;

    // https://www.w3.org/TR/web-animations-1/#in-play
    let is_in_play = timing.phase == Phase::Active && !row.has(flag::IS_FINISHED);

    // https://www.w3.org/TR/web-animations-1/#current
    let is_current = is_in_play
        || (timing.playback_rate > 0.0 && timing.phase == Phase::Before)
        || (timing.playback_rate < 0.0 && timing.phase == Phase::After)
        || (row.has(flag::HAS_TIMELINE)
            && !row.has(flag::TIMELINE_IS_MONOTONICALLY_INCREASING)
            && play_state(row, timing.current_time, timing.end_time)? != PlayState::Idle);
    if is_current {
        return Some(true);
    }

    // https://www.w3.org/TR/web-animations-1/#in-effect, via the active time.
    Some(timing.active_time.is_some())
}

/// A mirror of `AnimationEffect::ResolvedTiming`, with what `Animation` contributes to it.
#[derive(Clone, Copy)]
struct ResolvedTiming {
    phase: Phase,
    current_time: Option<TimeValue>,
    active_time: Option<TimeValue>,
    active_duration: TimeValue,
    end_time: TimeValue,
    iteration_duration: TimeValue,
    iteration_count: f64,
    playback_rate: f64,
}

/// Resolve everything the phase and the active time are derived from. `None` where the host's
/// arithmetic would refuse to mix two times' kinds, which is where the mirror must decline.
#[must_use]
fn resolve_timing(row: &AnimationTimingRow, timeline_time: Option<TimeValue>) -> Option<ResolvedTiming> {
    use timing_row_flag as flag;

    let progress_based = row.has(flag::TIMELINE_IS_PROGRESS_BASED);
    let zero = TimeValue::zero(progress_based);
    let playback_rate = row.times[TIME_PLAYBACK_RATE];

    // https://www.w3.org/TR/web-animations-1/#animation-current-time
    let current_time = if row.has(flag::HAS_HOLD_TIME) {
        Some(row.time(TIME_HOLD, flag::HOLD_TIME_IS_PERCENTAGE))
    } else if !row.has(flag::HAS_TIMELINE) || timeline_time.is_none() || !row.has(flag::HAS_START_TIME) {
        None
    } else {
        let start_time = row.time(TIME_START, flag::START_TIME_IS_PERCENTAGE);
        Some(timeline_time?.subtract(start_time)?.scale(playback_rate))
    };

    let start_delay = row.time(TIME_START_DELAY, flag::START_DELAY_IS_PERCENTAGE);
    let end_delay = row.time(TIME_END_DELAY, flag::END_DELAY_IS_PERCENTAGE);
    let iteration_duration = row.time(TIME_ITERATION_DURATION, flag::ITERATION_DURATION_IS_PERCENTAGE);
    let iteration_count = row.times[TIME_ITERATION_COUNT];

    // https://www.w3.org/TR/web-animations-1/#active-duration
    let active_duration = if iteration_duration.value == 0.0 || iteration_count == 0.0 {
        zero
    } else {
        iteration_duration.scale(iteration_count)
    };
    // https://www.w3.org/TR/web-animations-1/#end-time
    let end_time = start_delay.add(active_duration)?.add(end_delay)?.largest(zero)?;
    let before_active_boundary_time = start_delay.smallest(end_time)?.largest(zero)?;
    let after_active_boundary_time = start_delay.add(active_duration)?.smallest(end_time)?.largest(zero)?;

    // https://www.w3.org/TR/web-animations-1/#animation-direction: "backwards" when the playback
    // rate is negative, "forwards" otherwise.
    let direction_is_backwards = playback_rate < 0.0;
    let phase = match current_time {
        None => Phase::Idle,
        Some(local_time) => {
            let before = local_time.compare(before_active_boundary_time)?;
            if before.is_lt() || (direction_is_backwards && before.is_eq()) {
                Phase::Before
            } else {
                let after = local_time.compare(after_active_boundary_time)?;
                if after.is_gt() || (!direction_is_backwards && after.is_eq()) {
                    Phase::After
                } else {
                    Phase::Active
                }
            }
        }
    };

    // https://www.w3.org/TR/web-animations-1/#active-time
    let fill_mode = (row.flags >> flag::FILL_MODE_SHIFT) & flag::FILL_MODE_MASK;
    let active_time = match phase {
        Phase::Before => match fill_mode == fill_mode::BACKWARDS || fill_mode == fill_mode::BOTH {
            true => Some(current_time?.subtract(start_delay)?.largest(zero)?),
            false => None,
        },
        Phase::Active => Some(current_time?.subtract(start_delay)?),
        Phase::After => match fill_mode == fill_mode::FORWARDS || fill_mode == fill_mode::BOTH {
            true => Some(
                current_time?
                    .subtract(start_delay)?
                    .smallest(active_duration)?
                    .largest(zero)?,
            ),
            false => None,
        },
        Phase::Idle => None,
    };

    Some(ResolvedTiming {
        phase,
        current_time,
        active_time,
        active_duration,
        end_time,
        iteration_duration,
        iteration_count,
        playback_rate,
    })
}

/// The key the style stage samples an effect's keyframes at, which is
/// `AnimationEffect::transformed_progress()` scaled the way `collect_animation_effects_into` scales
/// it. The outer `None` is the mirror declining; the inner `None` is the host's unresolved
/// progress, which is the stage skipping the effect.
///
/// A mirror of `transformed_progress()` and everything under it: `overall_progress()`,
/// `simple_iteration_progress()`, `current_iteration()`, `current_direction()`,
/// `directed_progress()` and `EasingFunction::evaluate_at()`.
#[must_use]
pub(crate) fn row_current_key(row: &AnimationTimingRow, timeline_time: Option<TimeValue>) -> Option<Option<f64>> {
    use timing_row_flag as flag;

    if row.has(flag::UNDECIDABLE) || row.has(flag::EASING_HAS_CONTROL_POINTS) {
        return None;
    }
    let timing = resolve_timing(row, timeline_time)?;

    // https://www.w3.org/TR/web-animations-1/#overall-progress
    let Some(active_time) = timing.active_time else {
        return Some(None);
    };
    let iteration_start = row.times[TIME_ITERATION_START];
    let iterations_elapsed = if timing.iteration_duration.value == 0.0 {
        match timing.phase {
            Phase::Before => 0.0,
            _ => timing.iteration_count,
        }
    } else {
        // `TimeValue::operator/` verifies that the two times measure the same thing.
        if !active_time.agrees_with(timing.iteration_duration) {
            return None;
        }
        active_time.value / timing.iteration_duration.value
    };
    let overall_progress = iterations_elapsed + iteration_start;

    // https://www.w3.org/TR/web-animations-1/#simple-iteration-progress
    let mut simple_iteration_progress = match overall_progress.is_infinite() {
        true => iteration_start % 1.0,
        false => overall_progress % 1.0,
    };
    if simple_iteration_progress == 0.0
        && (timing.phase == Phase::Active || timing.phase == Phase::After)
        && active_time.compare(timing.active_duration)?.is_eq()
        && timing.iteration_count != 0.0
    {
        simple_iteration_progress = 1.0;
    }

    // https://www.w3.org/TR/web-animations-1/#current-iteration
    let current_iteration = if timing.phase == Phase::After && timing.iteration_count.is_infinite() {
        timing.iteration_count
    } else if simple_iteration_progress == 1.0 {
        overall_progress.floor() - 1.0
    } else {
        overall_progress.floor()
    };

    // https://www.w3.org/TR/web-animations-1/#directed-progress, step 2.
    let direction = (row.flags >> flag::PLAYBACK_DIRECTION_SHIFT) & flag::PLAYBACK_DIRECTION_MASK;
    let going_forwards = match direction {
        playback_direction::NORMAL => true,
        playback_direction::REVERSE => false,
        _ => {
            let mut iteration = current_iteration;
            if direction == playback_direction::ALTERNATE_REVERSE {
                iteration += 1.0;
            }
            iteration.is_infinite() || iteration % 2.0 == 0.0
        }
    };
    let directed_progress = match going_forwards {
        true => simple_iteration_progress,
        false => 1.0 - simple_iteration_progress,
    };

    // https://www.w3.org/TR/web-animations-1/#transformed-progress
    let before_flag =
        (timing.phase == Phase::Before && going_forwards) || (timing.phase == Phase::After && !going_forwards);
    let easing_kind = (row.flags >> flag::EASING_KIND_SHIFT) & flag::EASING_KIND_MASK;
    let output_progress = match easing_kind {
        // `linear`, which the host holds as `linear(0, 1)`.
        0 => crate::css::animation::evaluate_linear_easing(
            &[
                crate::css::animation::FfiLinearEasingPoint {
                    input: 0.0,
                    output: 0.0,
                },
                crate::css::animation::FfiLinearEasingPoint {
                    input: 1.0,
                    output: 1.0,
                },
            ],
            directed_progress,
            before_flag,
        ),
        1 => crate::css::animation::evaluate_cubic_bezier_easing(
            row.times[TIME_EASING_X1],
            row.times[TIME_EASING_Y1],
            row.times[TIME_EASING_X2],
            row.times[TIME_EASING_Y2],
            directed_progress,
        ),
        2 => crate::css::animation::evaluate_steps_easing(
            row.easing_interval_count,
            ((row.flags >> flag::EASING_STEP_POSITION_SHIFT) & flag::EASING_STEP_POSITION_MASK) as u8,
            directed_progress,
            before_flag,
        ),
        _ => return None,
    };

    // `AnimationKeyFrameKeyScaleFactor`, and the host's clamp to what an `i64` key can hold.
    let key = output_progress * 100.0 * 1000.0;
    Some(Some(key.clamp(i64::MIN as f64, i64::MAX as f64)))
}

/// A mirror of `Animation::play_state_at()`. `associated_effect_end` is the effect's end time,
/// since a row only exists for an animation that has one.
#[must_use]
fn play_state(
    row: &AnimationTimingRow,
    current_time: Option<TimeValue>,
    associated_effect_end: TimeValue,
) -> Option<PlayState> {
    use timing_row_flag as flag;

    let pending = row.has(flag::HAS_PENDING_PLAY_TASK) || row.has(flag::HAS_PENDING_PAUSE_TASK);
    if current_time.is_none() && !row.has(flag::HAS_START_TIME) && !pending {
        return Some(PlayState::Idle);
    }
    if row.has(flag::HAS_PENDING_PAUSE_TASK)
        || (!row.has(flag::HAS_START_TIME) && !row.has(flag::HAS_PENDING_PLAY_TASK))
    {
        return Some(PlayState::Paused);
    }
    let effective_playback_rate = match row.has(flag::HAS_PENDING_PLAYBACK_RATE) {
        true => row.times[TIME_PENDING_PLAYBACK_RATE],
        false => row.times[TIME_PLAYBACK_RATE],
    };
    if let Some(current_time) = current_time {
        let finished = (effective_playback_rate > 0.0 && current_time.compare(associated_effect_end)?.is_ge())
            || (effective_playback_rate < 0.0 && current_time.value <= 0.0);
        if finished {
            return Some(PlayState::Finished);
        }
    }
    Some(PlayState::Running)
}

/// Per element and pseudo-element, the timing of every animation the host holds a keyframe effect
/// for, published whole whenever any of it can have changed.
#[derive(Default)]
pub(crate) struct AnimationTimingRows {
    rows: HashMap<(StyleNodeID, AnimationSlot), Box<[AnimationTimingRow]>>,
}

impl AnimationTimingRows {
    /// Replace one list, from the two buffers the host packs it into. An empty list drops the row.
    pub(crate) fn set(&mut self, node: StyleNodeID, slot: AnimationSlot, words: &[u32], times: &[f64]) {
        if words.is_empty() {
            self.rows.remove(&(node, slot));
            return;
        }
        let count = words.len() / TIMING_ROW_WORDS;
        assert!(
            times.len() == count * TIMING_ROW_TIMES,
            "an animation timing row has eight times"
        );
        let mut rows = Vec::with_capacity(count);
        for index in 0..count {
            let words = &words[index * TIMING_ROW_WORDS..][..TIMING_ROW_WORDS];
            let mut row = AnimationTimingRow {
                flags: words[WORD_FLAGS],
                timeline: words[WORD_TIMELINE],
                easing_interval_count: words[WORD_EASING_INTERVAL_COUNT] as i32,
                effect_identity: u64::from(words[WORD_EFFECT_IDENTITY_LOW])
                    | (u64::from(words[WORD_EFFECT_IDENTITY_HIGH]) << 32),
                times: [0.0; TIMING_ROW_TIMES],
            };
            row.times
                .copy_from_slice(&times[index * TIMING_ROW_TIMES..][..TIMING_ROW_TIMES]);
            rows.push(row);
        }
        // Republishing an unchanged list is the common case - the host cannot cheaply tell that
        // nothing moved - so compare before giving up the allocation the engine already holds.
        match self.rows.get(&(node, slot)) {
            Some(existing) if **existing == rows[..] => {}
            _ => {
                self.rows.insert((node, slot), rows.into_boxed_slice());
            }
        }
    }

    #[must_use]
    pub(crate) fn rows(&self, node: StyleNodeID, slot: AnimationSlot) -> &[AnimationTimingRow] {
        self.rows.get(&(node, slot)).map_or(&[][..], |rows| &rows[..])
    }

    /// The row of one effect, which the stage names by the identity it already uses to look its
    /// description up. The host publishes the rows in its own order, not the composite order the
    /// stage walks in, so a position is not an answer.
    #[must_use]
    pub(crate) fn row_for_effect(
        &self,
        node: StyleNodeID,
        slot: AnimationSlot,
        effect_identity: u64,
    ) -> Option<&AnimationTimingRow> {
        self.rows(node, slot)
            .iter()
            .find(|row| row.effect_identity == effect_identity)
    }

    /// Give up the rows of identities that have been retired, which can be minted again.
    pub(crate) fn retire(&mut self, nodes: &[StyleNodeID]) {
        if self.rows.is_empty() {
            return;
        }
        self.rows.retain(|&(node, _), _| !nodes.contains(&node));
    }
}

/// The current time each of the document's animation timelines was sampled at when the style
/// update began. A timeline's time is a cached value that only the rendering loop moves, so one
/// sample serves the whole update.
#[derive(Default)]
pub(crate) struct AnimationTimelineSamples {
    samples: HashMap<u32, Option<TimeValue>>,
}

impl AnimationTimelineSamples {
    const SAMPLE_HAS_TIME: u32 = 1 << 0;
    const SAMPLE_IS_PERCENTAGE: u32 = 1 << 1;

    pub(crate) fn set(&mut self, identities: &[u32], words: &[u32], times: &[f64]) {
        assert!(words.len() == identities.len() && times.len() == identities.len());
        self.samples.clear();
        for index in 0..identities.len() {
            let sample = (words[index] & Self::SAMPLE_HAS_TIME != 0).then(|| TimeValue {
                is_percentage: words[index] & Self::SAMPLE_IS_PERCENTAGE != 0,
                value: times[index],
            });
            self.samples.insert(identities[index], sample);
        }
    }

    /// The sample for one timeline, or `None` when no sample was published for it - a timeline of
    /// another document, most of all, whose identities are not this engine's to read.
    #[must_use]
    pub(crate) fn sample(&self, identity: u32) -> Option<Option<TimeValue>> {
        self.samples.get(&identity).copied()
    }
}

/// Whether any of an element's animations for one pseudo-element is relevant, which is what
/// `Element::get_animations_internal()` filters its list by. `None` where any single row declines
/// to answer, since an unanswered row could be the relevant one.
#[must_use]
pub(crate) fn any_row_is_relevant(rows: &[AnimationTimingRow], samples: &AnimationTimelineSamples) -> Option<bool> {
    let mut any = false;
    for row in rows {
        any |= row_is_relevant(row, row_timeline_time(row, samples)?)?;
    }
    Some(any)
}

/// The current time of the timeline a row names, as the host sampled it when this style update
/// began. `None` where the host published no sample for that timeline.
#[must_use]
pub(crate) fn row_timeline_time(
    row: &AnimationTimingRow,
    samples: &AnimationTimelineSamples,
) -> Option<Option<TimeValue>> {
    match row.has(timing_row_flag::HAS_TIMELINE) {
        true => samples.sample(row.timeline),
        false => Some(None),
    }
}

/// Flags on a published animation effect.
pub(crate) mod effect_flag {
    /// The effect belongs to a CSS transition, which the interpolation treats differently.
    pub(crate) const IS_TRANSITION: u32 = 1 << 0;
    /// The host could not describe this effect for the stage - a keyframe value that still needs
    /// substitution, a custom property, an easing that is itself a style value - so the stage has to
    /// collect it the way it always has.
    pub(crate) const NOT_COVERED: u32 = 1 << 1;
    pub(crate) const HAS_RESOURCE_CONTEXT: u32 = 1 << 2;
    pub(crate) const RESOURCE_CONTEXT_IS_ORIGIN_CLEAN: u32 = 1 << 3;
}

/// One easing function, as the host resolved it when it described the effect. The linear points are
/// owned so a published keyframe can hand out an `FfiEasingDescriptor` that borrows them.
#[derive(Clone)]
pub(crate) struct PublishedEasing {
    kind: u8,
    linear_points: Box<[crate::css::animation::FfiLinearEasingPoint]>,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    interval_count: i32,
    step_position: u8,
}

impl PublishedEasing {
    #[must_use]
    pub(crate) fn descriptor(&self) -> crate::css::animation::FfiEasingDescriptor {
        use crate::css::animation::{FfiEasingDescriptor, FfiEasingKind};
        FfiEasingDescriptor {
            kind: match self.kind {
                1 => FfiEasingKind::CubicBezier,
                2 => FfiEasingKind::Steps,
                _ => FfiEasingKind::Linear,
            },
            linear_points: self.linear_points.as_ptr(),
            linear_point_count: self.linear_points.len(),
            x1: self.x1,
            y1: self.y1,
            x2: self.x2,
            y2: self.y2,
            interval_count: self.interval_count,
            step_position: self.step_position,
        }
    }
}

/// One keyframe of a published effect: its offset on the 0..100000 scale the host keys keyframes by,
/// the easing that governs the interval starting at it, and the composite operation, already
/// resolved against the effect's own where the keyframe said `auto`.
pub(crate) struct PublishedKeyframe {
    pub(crate) key: i64,
    pub(crate) easing: PublishedEasing,
    pub(crate) composite: u8,
    declaration_range: std::ops::Range<usize>,
}

/// One property a published keyframe declares. `use_initial` marks the keyframe the host synthesized
/// to hold the element's own value, whose value is not known until the element is sampled.
pub(crate) struct PublishedDeclaration {
    pub(crate) property_id: u16,
    pub(crate) use_initial: bool,
    pub(crate) value: crate::css::style_value::RetainedStyleValueData,
}

/// One of an element's animation effects, described for the style stage.
pub(crate) struct PublishedEffect {
    pub(crate) identity: u64,
    pub(crate) generation: u64,
    pub(crate) flags: u32,
    pub(crate) base_url: Box<[u8]>,
    pub(crate) keyframes: Box<[PublishedKeyframe]>,
    pub(crate) declarations: Box<[PublishedDeclaration]>,
}

impl PublishedEffect {
    #[must_use]
    pub(crate) fn is_covered(&self) -> bool {
        self.flags & effect_flag::NOT_COVERED == 0
    }

    #[must_use]
    pub(crate) fn declarations_of(&self, keyframe: &PublishedKeyframe) -> &[PublishedDeclaration] {
        &self.declarations[keyframe.declaration_range.clone()]
    }

    #[must_use]
    pub(crate) fn resource_context(&self) -> crate::css::animation::FfiAnimationStyleSheetResourceContext {
        crate::css::animation::FfiAnimationStyleSheetResourceContext {
            base_url: self.base_url.as_ptr(),
            base_url_length: self.base_url.len(),
            has_value: self.flags & effect_flag::HAS_RESOURCE_CONTEXT != 0,
            origin_clean: self.flags & effect_flag::RESOURCE_CONTEXT_IS_ORIGIN_CLEAN != 0,
        }
    }
}

/// The flat buffers one element's effect descriptions travel in.
pub struct PublishedEffectBuffers<'a> {
    pub effects: &'a [super::bridge::FfiPublishedAnimationEffect],
    pub keyframes: &'a [super::bridge::FfiPublishedAnimationKeyframe],
    pub declarations: &'a [super::bridge::FfiPublishedAnimationDeclaration],
    pub linear_points: &'a [super::bridge::FfiPublishedLinearEasingPoint],
    pub base_url_bytes: &'a [u8],
}

/// Per element and pseudo-element, the effects the host holds, in composite order, described well
/// enough for the style stage to build the animation batch itself.
#[derive(Default)]
pub(crate) struct AnimationEffectDescriptions {
    rows: HashMap<(StyleNodeID, AnimationSlot), Box<[PublishedEffect]>>,
}

impl AnimationEffectDescriptions {
    /// Replace one list from the flat buffers the host packs it into. An empty list drops the row.
    ///
    /// # Safety
    /// Every declaration's `value` must be a live style value the host holds a reference to for the
    /// duration of the call.
    pub(crate) unsafe fn set(
        &mut self,
        node: StyleNodeID,
        slot: AnimationSlot,
        published_buffers: PublishedEffectBuffers<'_>,
    ) {
        let PublishedEffectBuffers {
            effects,
            keyframes,
            declarations,
            linear_points,
            base_url_bytes,
        } = published_buffers;
        if effects.is_empty() {
            self.rows.remove(&(node, slot));
            return;
        }
        let mut published = Vec::with_capacity(effects.len());
        for effect in effects {
            let keyframe_range =
                effect.first_keyframe as usize..(effect.first_keyframe + effect.keyframe_count) as usize;
            let mut published_keyframes = Vec::with_capacity(keyframe_range.len());
            let mut published_declarations = Vec::new();
            for keyframe in &keyframes[keyframe_range] {
                let points = linear_points[keyframe.first_linear_point as usize..]
                    [..keyframe.linear_point_count as usize]
                    .iter()
                    .map(|point| crate::css::animation::FfiLinearEasingPoint {
                        input: point.input,
                        output: point.output,
                    })
                    .collect::<Vec<_>>();
                let first = published_declarations.len();
                for declaration in
                    &declarations[keyframe.first_declaration as usize..][..keyframe.declaration_count as usize]
                {
                    // SAFETY: the caller holds a reference to the value for the call, and
                    //         `rust_style_value_retain` takes one of its own for the engine.
                    let value = match declaration.value.is_null() {
                        true => crate::css::style_value::RetainedStyleValueData::none(),
                        false => unsafe {
                            crate::css::style_value::RetainedStyleValueData::from_retained_pointer(
                                crate::css::style_value::rust_style_value_retain(declaration.value.cast()),
                            )
                        },
                    };
                    published_declarations.push(PublishedDeclaration {
                        property_id: declaration.property_id,
                        use_initial: declaration.use_initial,
                        value,
                    });
                }
                published_keyframes.push(PublishedKeyframe {
                    key: keyframe.key,
                    easing: PublishedEasing {
                        kind: keyframe.easing_kind,
                        linear_points: points.into_boxed_slice(),
                        x1: keyframe.x1,
                        y1: keyframe.y1,
                        x2: keyframe.x2,
                        y2: keyframe.y2,
                        interval_count: keyframe.interval_count,
                        step_position: keyframe.step_position,
                    },
                    composite: keyframe.composite,
                    declaration_range: first..published_declarations.len(),
                });
            }
            published.push(PublishedEffect {
                identity: effect.identity,
                generation: effect.generation,
                flags: effect.flags,
                base_url: base_url_bytes[effect.base_url_offset as usize..][..effect.base_url_length as usize]
                    .to_vec()
                    .into_boxed_slice(),
                keyframes: published_keyframes.into_boxed_slice(),
                declarations: published_declarations.into_boxed_slice(),
            });
        }
        self.rows.insert((node, slot), published.into_boxed_slice());
    }

    #[must_use]
    pub(crate) fn effects(&self, node: StyleNodeID, slot: AnimationSlot) -> &[PublishedEffect] {
        self.rows.get(&(node, slot)).map_or(&[][..], |effects| &effects[..])
    }

    /// Give up the rows of identities that have been retired, which can be minted again.
    pub(crate) fn retire(&mut self, nodes: &[StyleNodeID]) {
        if self.rows.is_empty() {
            return;
        }
        self.rows.retain(|&(node, _), _| !nodes.contains(&node));
    }
}

/// A `@keyframes` name as a hash key. `CssString` compares by content but implements no `Hash`,
/// and a name is looked up once per animation definition, so the key hashes the code units.
#[derive(PartialEq, Eq)]
struct KeyframesName(CssString);

impl std::hash::Hash for KeyframesName {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        std::hash::Hash::hash(self.0.units(), state);
    }
}

/// The `@keyframes` every style scope of the document defines, as each scope's rule cache resolved
/// them.
///
/// Resolving an animation's keyframes used to build the scope's rule cache inside the style stage -
/// parsing the user sheet, evaluating the user-agent sheet's media queries and allocating the cache
/// on the spot - and then walk the live tree for the scopes to look in. The host builds every
/// scope's cache at the style update's begin boundary instead and publishes what came out, so the
/// stage's answer is a lookup in this table.
///
/// The keyframe set is the host's own refcounted object, borrowed: the host holds a reference to
/// each published scope's cache for as long as the table names it, and replaces a scope's whole row
/// when that scope's rule cache is rebuilt. A replayed engine is never published to and resolves
/// nothing, the way it reads no layout arena.
#[derive(Default)]
pub(crate) struct AnimationKeyframes {
    scopes: HashMap<TreeScopeID, HashMap<KeyframesName, usize>>,
    /// Which scope a shadow root's host-side pointer identity names. The cascade attributes the
    /// winning `animation-name` declaration to a shadow root by that identity, and the scope it
    /// names is where the declaration's `@keyframes` are looked for first.
    scope_by_shadow_root: HashMap<usize, TreeScopeID>,
}

impl AnimationKeyframes {
    /// Replace one scope's row. The names arrive packed into one buffer of code units with a length
    /// each, the way an element's animation names do.
    pub(crate) fn set(
        &mut self,
        tree_scope: TreeScopeID,
        shadow_root_identity: usize,
        name_lengths: &[u32],
        name_units: &[u16],
        keyframe_sets: &[usize],
    ) {
        assert!(
            name_lengths.len() == keyframe_sets.len(),
            "a published @keyframes name must come with its keyframe set"
        );
        if name_lengths.is_empty() {
            self.scopes.remove(&tree_scope);
            // A scope that defines nothing and a scope with no row answer alike, so the identity
            // may as well stop naming it: giving the row up is how a shadow root on its way out
            // takes its address out of the table before another root can be allocated there.
            self.scope_by_shadow_root.remove(&shadow_root_identity);
            return;
        }
        if shadow_root_identity != 0 {
            self.scope_by_shadow_root.insert(shadow_root_identity, tree_scope);
        }
        let mut sets = HashMap::with_capacity(name_lengths.len());
        let mut offset = 0usize;
        for (index, &length) in name_lengths.iter().enumerate() {
            let end = offset + length as usize;
            assert!(end <= name_units.len(), "@keyframes name lengths overrun their buffer");
            sets.insert(
                KeyframesName(CssString::from_utf16(&name_units[offset..end])),
                keyframe_sets[index],
            );
            offset = end;
        }
        self.scopes.insert(tree_scope, sets);
    }

    #[must_use]
    fn in_scope(&self, tree_scope: TreeScopeID, name: &KeyframesName) -> Option<usize> {
        self.scopes.get(&tree_scope)?.get(name).copied()
    }

    /// The keyframe set an animation of this name runs, or zero where no scope in its chain defines
    /// one and the host makes an effect with no keyframes.
    ///
    /// The chain is the one the host walked: the tree scope of the winning `animation-name`
    /// declaration first, because that declaration can come from a shadow-root rule - `:host()` and
    /// `::slotted()` - while the element it styles is outside that subtree, and a same-named
    /// document rule must not win over it; then the scope the element itself is in; then the
    /// document.
    #[must_use]
    pub(crate) fn resolve(
        &self,
        declaration_shadow_root_identity: usize,
        element_tree_scope: TreeScopeID,
        name: &CssString,
    ) -> usize {
        if self.scopes.is_empty() {
            return 0;
        }
        let name = KeyframesName(name.clone());
        if declaration_shadow_root_identity != 0
            && let Some(scope) = self
                .scope_by_shadow_root
                .get(&declaration_shadow_root_identity)
                .copied()
            && let Some(set) = self.in_scope(scope, &name)
        {
            return set;
        }
        if element_tree_scope != TreeScopeID::DOCUMENT
            && let Some(set) = self.in_scope(element_tree_scope, &name)
        {
            return set;
        }
        self.in_scope(TreeScopeID::DOCUMENT, &name).unwrap_or(0)
    }
}

/// The transform reference box the last committed layout left for an element, in CSS pixels, which
/// a keyframe or transition resolves a percentage translation against. `None` where the element
/// has no committed box, which is the host's own condition for having no reference box.
///
/// This is a read of an earlier stage's committed output rather than of the element: the box is
/// taken from the layout arena's paintable rows by style-node identity, so the stage never follows
/// the element's layout-node pointer into the DOM. Publishing a box per node at commit time
/// instead would mean resolving every committed row's absolute rect on every layout, which layout
/// does lazily today and only for the rows that are painted.
///
/// # Safety
/// `arena` must be the document's live layout arena.
#[must_use]
pub(crate) unsafe fn committed_transform_reference_box(
    arena: *mut std::ffi::c_void,
    node: StyleNodeID,
) -> Option<(f64, f64)> {
    let arena = unsafe { &*arena.cast::<crate::layout::LayoutNodeArena>() };
    let row = arena.bound_row(node);
    if row.is_invalid() || !arena.paintable_row_is_populated(row) {
        return None;
    }
    let style = arena.node_style_if_live(row)?;
    let paintable_rows = arena.paintable_rows();
    let rect = crate::painting::visual_context::node_values::transform_reference_box(style, &paintable_rows, row);
    Some((rect.width.to_double(), rect.height.to_double()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name(text: &str) -> CssString {
        CssString::from_utf16(&text.encode_utf16().collect::<Vec<_>>())
    }

    #[test]
    fn an_empty_existing_list_creates_every_animation() {
        assert_eq!(
            match_existing_animations(&[], &[name("a"), name("b")]),
            vec![NO_MATCHED_ANIMATION, NO_MATCHED_ANIMATION]
        );
    }

    #[test]
    fn a_repeated_name_takes_the_last_unclaimed_animation_first() {
        // `a` becoming `a, a` keeps the existing animation as the second entry and creates the first.
        assert_eq!(
            match_existing_animations(&[name("a")], &[name("a"), name("a")]),
            vec![NO_MATCHED_ANIMATION, 0]
        );
        assert_eq!(
            match_existing_animations(&[name("a"), name("a")], &[name("a"), name("a")]),
            vec![0, 1]
        );
    }

    #[test]
    fn a_name_that_disappeared_claims_nothing() {
        assert_eq!(
            match_existing_animations(&[name("a"), name("b")], &[name("b")]),
            vec![1]
        );
    }
}
