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

use super::tree::StyleNodeID;
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
}

/// `Bindings::FillMode`, in IDL order.
mod fill_mode {
    pub(super) const FORWARDS: u32 = 1;
    pub(super) const BACKWARDS: u32 = 2;
    pub(super) const BOTH: u32 = 3;
}

/// How many words of each buffer one row occupies.
pub(crate) const TIMING_ROW_WORDS: usize = 2;
pub(crate) const TIMING_ROW_TIMES: usize = 8;

const WORD_FLAGS: usize = 0;
const WORD_TIMELINE: usize = 1;

const TIME_START: usize = 0;
const TIME_HOLD: usize = 1;
const TIME_START_DELAY: usize = 2;
const TIME_END_DELAY: usize = 3;
const TIME_ITERATION_DURATION: usize = 4;
const TIME_PLAYBACK_RATE: usize = 5;
const TIME_PENDING_PLAYBACK_RATE: usize = 6;
const TIME_ITERATION_COUNT: usize = 7;

/// One animation's timing, as the host held it when the style update began.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct AnimationTimingRow {
    flags: u32,
    timeline: u32,
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

    // https://www.w3.org/TR/web-animations-1/#in-play
    let is_in_play = phase == Phase::Active && !row.has(flag::IS_FINISHED);

    // https://www.w3.org/TR/web-animations-1/#current
    let is_current = is_in_play
        || (playback_rate > 0.0 && phase == Phase::Before)
        || (playback_rate < 0.0 && phase == Phase::After)
        || (row.has(flag::HAS_TIMELINE)
            && !row.has(flag::TIMELINE_IS_MONOTONICALLY_INCREASING)
            && play_state(row, current_time, end_time)? != PlayState::Idle);
    if is_current {
        return Some(true);
    }

    // https://www.w3.org/TR/web-animations-1/#in-effect, via the active time.
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
    Some(active_time.is_some())
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
        let timeline_time = match row.has(timing_row_flag::HAS_TIMELINE) {
            true => samples.sample(row.timeline)?,
            false => None,
        };
        any |= row_is_relevant(row, timeline_time)?;
    }
    Some(any)
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
