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

/// The animation list of the element itself, rather than one of its pseudo-elements'.
pub(crate) const ELEMENT_ANIMATION_SLOT: AnimationSlot = 0;

/// The definition that claimed no existing animation and asks for a new one.
pub(crate) const NO_MATCHED_ANIMATION: i32 = -1;

/// How many words of the published buffer one animation's applied definition occupies. A mirror of
/// `CSS::AppliedAnimationDefinitionRow`.
pub(crate) const APPLIED_DEFINITION_WORD_COUNT: usize = 6;

/// One animation's applied definition, as the host published it: what the plan that last touched
/// this animation computed for it. Two rows that compare equal describe a plan that would change
/// nothing, which is a plan the stage does not have to cross to the host to apply.
///
/// The words are opaque except for the last one, which is a borrowed pointer to the computed
/// `animation-timing-function` the animation retains. A recomputed style builds a fresh allocation
/// for a declaration that has not changed, so that one is compared by value.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) struct AppliedAnimationDefinition {
    words: [u64; APPLIED_DEFINITION_WORD_COUNT],
}

/// The word the flags sit in.
const APPLIED_DEFINITION_FLAGS_WORD: usize = 3;
/// `animation-duration: auto`, whose value is the effect's intrinsic duration rather than the
/// definition's.
const APPLIED_DEFINITION_DURATION_IS_AUTO: u64 = 1;
const APPLIED_DEFINITION_TIMELINE_KIND_SHIFT: u32 = 40;
/// The fields of the flags word a change to which moves no time: `animation-direction`,
/// `animation-fill-mode` and `animation-composition`, each a byte. Everything else in that word -
/// `duration_is_auto`, the play state and the timeline - is a change a retime cannot describe.
const APPLIED_DEFINITION_RETIMABLE_FLAGS_MASK: u64 = (0xff << 8) | (0xff << 24) | (0xff << 32);
const APPLIED_DEFINITION_KEYFRAME_SET_WORD: usize = 4;
const APPLIED_DEFINITION_TIMING_FUNCTION_WORD: usize = 5;
/// `AnimationTimelineSource::Kind::Scroll`.
const APPLIED_DEFINITION_TIMELINE_KIND_SCROLL: u64 = 2;

impl AppliedAnimationDefinition {
    #[must_use]
    pub(crate) fn from_words(words: &[u64]) -> Self {
        let mut row = Self {
            words: [0; APPLIED_DEFINITION_WORD_COUNT],
        };
        row.words.copy_from_slice(words);
        row
    }

    /// The same packing the host does over the definition it applied, so that a definition just
    /// computed and one published back compare word for word.
    #[must_use]
    pub(crate) fn from_definition(animation: &crate::css::style_compute::FfiComputedAnimation) -> Self {
        Self {
            words: [
                animation.duration.to_bits(),
                animation.iteration_count.to_bits(),
                animation.delay.to_bits(),
                u64::from(animation.duration_is_auto)
                    | (u64::from(animation.direction) << 8)
                    | (u64::from(animation.play_state) << 16)
                    | (u64::from(animation.fill_mode) << 24)
                    | (u64::from(animation.composition) << 32)
                    | (u64::from(animation.timeline_kind as u8) << APPLIED_DEFINITION_TIMELINE_KIND_SHIFT)
                    | (u64::from(animation.scroll_scroller) << 48)
                    | (u64::from(animation.scroll_axis) << 56),
                animation.keyframe_set as u64,
                animation.timing_function as u64,
            ],
        }
    }

    /// Whether the timeline this definition asks for is one whose materialization the engine can
    /// predict. A scroll timeline is rebuilt from the element's surroundings every time it is
    /// applied, so a definition that names one is never called unchanged.
    #[must_use]
    fn timeline_is_decidable(&self) -> bool {
        (self.words[APPLIED_DEFINITION_FLAGS_WORD] >> APPLIED_DEFINITION_TIMELINE_KIND_SHIFT) & 0xff
            != APPLIED_DEFINITION_TIMELINE_KIND_SCROLL
    }

    /// Whether applying `self` to an animation that last had `published` applied would leave it
    /// exactly as it is.
    #[must_use]
    pub(crate) fn would_change_nothing(&self, published: &Self) -> bool {
        if !self.timeline_is_decidable() {
            return false;
        }
        // An animation no plan has described yet publishes a null timing function, which no
        // computed definition ever has.
        if published.words[APPLIED_DEFINITION_TIMING_FUNCTION_WORD] == 0 {
            return false;
        }
        (0..APPLIED_DEFINITION_WORD_COUNT)
            .filter(|&index| index != APPLIED_DEFINITION_TIMING_FUNCTION_WORD)
            .all(|index| self.words[index] == published.words[index])
            && unsafe {
                crate::css::style_value::rust_style_value_equals(
                    self.words[APPLIED_DEFINITION_TIMING_FUNCTION_WORD] as *const _,
                    published.words[APPLIED_DEFINITION_TIMING_FUNCTION_WORD] as *const _,
                )
            }
    }

    /// Whether applying `self` to an animation that last had `published` applied would leave its
    /// timing exactly as it is and only give its effect another keyframe set, which moves no time,
    /// changes no play state and creates nothing: `apply_css_properties` takes its early return.
    #[must_use]
    pub(crate) fn change_is_only_keyframes(&self, published: &Self) -> bool {
        if self.words[APPLIED_DEFINITION_KEYFRAME_SET_WORD] == published.words[APPLIED_DEFINITION_KEYFRAME_SET_WORD] {
            return false;
        }
        let mut without_the_keyframes = *self;
        without_the_keyframes.words[APPLIED_DEFINITION_KEYFRAME_SET_WORD] =
            published.words[APPLIED_DEFINITION_KEYFRAME_SET_WORD];
        without_the_keyframes.would_change_nothing(published)
    }

    /// The `animation-play-state` this definition asks for, where it is not the one `published`
    /// asked for: `Some(true)` for `running`. `None` where the two ask for the same.
    #[must_use]
    pub(crate) fn changed_play_state(&self, published: &Self) -> Option<bool> {
        let play_state = |row: &Self| (row.words[APPLIED_DEFINITION_FLAGS_WORD] >> 16) & 0xff;
        // `animation_play_state::PAUSED` is 0.
        (play_state(self) != play_state(published)).then(|| play_state(self) != 0)
    }

    /// This definition with the play state `published` asked for, which is what is left to compare
    /// once the play state change is taken apart.
    #[must_use]
    pub(crate) fn with_play_state_of(&self, published: &Self) -> Self {
        const PLAY_STATE_MASK: u64 = 0xff << 16;
        let mut row = *self;
        row.words[APPLIED_DEFINITION_FLAGS_WORD] = (row.words[APPLIED_DEFINITION_FLAGS_WORD] & !PLAY_STATE_MASK)
            | (published.words[APPLIED_DEFINITION_FLAGS_WORD] & PLAY_STATE_MASK);
        row
    }

    /// Whether applying `self` to an animation that last had `published` applied would change only
    /// what its effect is sampled from and how far a given time is along it, and move no time.
    ///
    /// `apply_css_properties` hands such a definition to the effect's plain setters, none of which
    /// notifies the animation, so the start time, the hold time and the pending tasks stay as they
    /// are: the retimed row is the published row with the three specified times restamped and the
    /// fill and direction fields replaced. A play-state change and an `auto` duration move time.
    #[must_use]
    pub(crate) fn change_is_only_simple_timing(&self, published: &Self) -> bool {
        if !self.timeline_is_decidable() || published.words[APPLIED_DEFINITION_TIMING_FUNCTION_WORD] == 0 {
            return false;
        }
        if self.words[APPLIED_DEFINITION_FLAGS_WORD] & APPLIED_DEFINITION_DURATION_IS_AUTO != 0 {
            return false;
        }
        self.words[APPLIED_DEFINITION_FLAGS_WORD] & !APPLIED_DEFINITION_RETIMABLE_FLAGS_MASK
            == published.words[APPLIED_DEFINITION_FLAGS_WORD] & !APPLIED_DEFINITION_RETIMABLE_FLAGS_MASK
    }
}

/// Per element and pseudo-element, the names of the CSS animations the host holds for it, in the
/// order the host holds them, and the definition the last plan applied to each.
#[derive(Default)]
pub(crate) struct CssDefinedAnimations {
    /// Owning a CSS animation is rare, so only the elements that do have a row.
    rows: HashMap<(StyleNodeID, AnimationSlot), CssDefinedAnimationRow>,
    /// The `@keyframes` generation each row was published at: what the plan that made it was
    /// decided against. A row published before the table moved says nothing about what its
    /// animations run now.
    keyframes_generations: HashMap<(StyleNodeID, AnimationSlot), u64>,
}

/// One element's list: the animations' names, and the definition the last plan applied to each.
type CssDefinedAnimationRow = (Box<[CssString]>, Box<[AppliedAnimationDefinition]>);

impl CssDefinedAnimations {
    /// Replace one list. An empty list drops the row, so an element that stops animating stops
    /// costing anything.
    pub(crate) fn set(
        &mut self,
        node: StyleNodeID,
        slot: AnimationSlot,
        names: Box<[CssString]>,
        definitions: Box<[AppliedAnimationDefinition]>,
        keyframes_generation: u64,
    ) {
        if names.is_empty() {
            self.rows.remove(&(node, slot));
            self.keyframes_generations.remove(&(node, slot));
            return;
        }
        self.rows.insert((node, slot), (names, definitions));
        self.keyframes_generations.insert((node, slot), keyframes_generation);
    }

    /// Whether every list this element holds was published against the `@keyframes` table as it
    /// stands. Where one was not, what its animations run may have moved with the table, and only
    /// the computation that re-plans them can say.
    #[must_use]
    pub(crate) fn node_is_planned_against(&self, node: StyleNodeID, keyframes_generation: u64) -> bool {
        self.rows
            .keys()
            .all(|key| key.0 != node || self.keyframes_generations.get(key) == Some(&keyframes_generation))
    }

    #[must_use]
    pub(crate) fn names(&self, node: StyleNodeID, slot: AnimationSlot) -> &[CssString] {
        self.rows.get(&(node, slot)).map_or(&[][..], |row| &row.0[..])
    }

    /// The definition the last plan applied to each animation of the list, in the list's order.
    #[must_use]
    pub(crate) fn definitions(&self, node: StyleNodeID, slot: AnimationSlot) -> &[AppliedAnimationDefinition] {
        self.rows.get(&(node, slot)).map_or(&[][..], |row| &row.1[..])
    }

    /// Whether the element runs a CSS animation at all, in any of its lists. Only such an element
    /// is re-planned when a `@keyframes` rule moves, and the plan is no part of its record.
    #[must_use]
    pub(crate) fn node_runs_a_css_animation(&self, node: StyleNodeID) -> bool {
        self.rows.keys().any(|&(row_node, _)| row_node == node)
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
    /// A provisionally started transition's row. The pass that started the transition samples its
    /// effect, so the row is published for it to be sampled from, but the transition is not
    /// associated with its target yet and the row answers nothing about what the element holds.
    pub(crate) const NOT_ASSOCIATED: u32 = 1 << 27;
    /// The animation names an owning element, which is the first thing the class-specific composite
    /// order of a CSS animation or transition compares.
    pub(crate) const HAS_OWNING_ELEMENT: u32 = 1 << 28;
    /// The owning element currently lists this CSS animation at the place its class-specific key
    /// names.
    pub(crate) const LISTED_BY_OWNING_ELEMENT: u32 = 1 << 29;
    /// Script played or paused this CSS animation, so a change to `animation-play-state` no longer
    /// plays or pauses it.
    pub(crate) const CSS_PLAY_STATE_OVERRIDDEN_BY_SCRIPT: u32 = 1 << 30;
}

/// `Animations::AnimationClass`, in declaration order, which is also the inter-class composite
/// order the host sorts by.
mod animation_class {
    pub(super) const CSS_ANIMATION_WITH_OWNING_ELEMENT: u8 = 0;
    pub(super) const CSS_TRANSITION: u8 = 1;
    pub(super) const CSS_ANIMATION_WITHOUT_OWNING_ELEMENT: u8 = 2;
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
pub(crate) const TIMING_ROW_WORDS: usize = 11;
pub(crate) const TIMING_ROW_TIMES: usize = 13;

const WORD_FLAGS: usize = 0;
const WORD_TIMELINE: usize = 1;
const WORD_EASING_INTERVAL_COUNT: usize = 2;
const WORD_EFFECT_IDENTITY_LOW: usize = 3;
const WORD_EFFECT_IDENTITY_HIGH: usize = 4;
/// The class in the low byte, the owning element's pseudo-element slot in the second, and the
/// transition property in the high half.
const WORD_COMPOSITE: usize = 5;
const WORD_COMPOSITE_OWNING_NODE: usize = 6;
const WORD_COMPOSITE_CLASS_KEY: usize = 7;
const WORD_GLOBAL_LIST_ORDER: usize = 8;
/// Where in the list's shared stop buffer this row's `linear()` control points start, and how many
/// of them there are. A count of zero is the identity `linear(0, 1)`.
const WORD_FIRST_LINEAR_POINT: usize = 9;
const WORD_LINEAR_POINT_COUNT: usize = 10;

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
    composite_class: u8,
    composite_owning_slot: u8,
    composite_transition_property: u16,
    composite_owning_node: u32,
    composite_class_key: u32,
    global_list_order: u32,
    first_linear_point: u32,
    linear_point_count: u32,
    times: [f64; TIMING_ROW_TIMES],
    /// For a row this stage synthesized for an animation the host has not created yet, which of the
    /// computation's starting animations it stands for. `None` for a row the host published.
    synthesized_index: Option<u32>,
}

impl AnimationTimingRow {
    /// One row's worth of the word buffer the host packs a list into. The times travel in their own
    /// buffer and are filled in by the caller.
    #[must_use]
    pub(crate) fn from_words(words: &[u32]) -> Self {
        Self {
            flags: words[WORD_FLAGS],
            timeline: words[WORD_TIMELINE],
            easing_interval_count: words[WORD_EASING_INTERVAL_COUNT] as i32,
            effect_identity: u64::from(words[WORD_EFFECT_IDENTITY_LOW])
                | (u64::from(words[WORD_EFFECT_IDENTITY_HIGH]) << 32),
            composite_class: words[WORD_COMPOSITE] as u8,
            composite_owning_slot: (words[WORD_COMPOSITE] >> 8) as u8,
            composite_transition_property: (words[WORD_COMPOSITE] >> 16) as u16,
            composite_owning_node: words[WORD_COMPOSITE_OWNING_NODE],
            composite_class_key: words[WORD_COMPOSITE_CLASS_KEY],
            global_list_order: words[WORD_GLOBAL_LIST_ORDER],
            first_linear_point: words[WORD_FIRST_LINEAR_POINT],
            linear_point_count: words[WORD_LINEAR_POINT_COUNT],
            times: [0.0; TIMING_ROW_TIMES],
            synthesized_index: None,
        }
    }

    /// The row a CSS animation this definition is about to start would publish, built from the
    /// definition alone.
    ///
    /// `CSSAnimation::apply_css_properties` settles the effect's timing from the definition and
    /// then starts it, and a brand-new animation's current time is unresolved, so both "play an
    /// animation" and "pause an animation" hold it at time zero and leave the rest to a task that
    /// runs after this style update. The timeline's current time therefore never enters the
    /// arithmetic.
    ///
    /// `None` for a definition whose row this cannot settle: a scroll or view timeline, which is
    /// materialized from the element's surroundings. `animation-timeline: none` materializes no
    /// timeline at all.
    #[must_use]
    pub(crate) fn for_new_css_animation(
        definition: &crate::css::style_compute::FfiComputedAnimation,
        owning_node: StyleNodeID,
        owning_slot: AnimationSlot,
        name_index: u32,
    ) -> Option<Self> {
        use crate::css::style_compute::FfiAnimationTimelineKind;
        use timing_row_flag as flag;

        // NB: `animation-duration: auto` - the initial value - has the intrinsic iteration duration
        //     of the effect, which against a monotonic timeline is zero; the drive already computed
        //     the definition's duration as zero for it.
        let timeline_flags = match definition.timeline_kind {
            FfiAnimationTimelineKind::Document => flag::HAS_TIMELINE | flag::TIMELINE_IS_MONOTONICALLY_INCREASING,
            FfiAnimationTimelineKind::None => 0,
            FfiAnimationTimelineKind::Scroll => return None,
        };
        // `Bindings::PlaybackDirection` and `Bindings::FillMode` are in IDL order, which is not the
        // order the CSS keywords are in: a mirror of `css_animation_direction_to_playback_direction`
        // and `css_fill_mode_to_bindings_fill_mode`.
        let direction = css_playback_direction(definition.direction)?;
        let fill_mode = css_fill_mode(definition.fill_mode)?;
        // A pending play or pause task settles nothing the phase or the active time is derived
        // from, but the row the host publishes for this animation carries one, so this one does
        // too. `animation_play_state::PAUSED` is 0.
        let pending_task = match definition.play_state {
            0 => flag::HAS_PENDING_PAUSE_TASK,
            _ => flag::HAS_PENDING_PLAY_TASK,
        };
        let mut times = [0.0; TIMING_ROW_TIMES];
        times[TIME_HOLD] = 0.0;
        times[TIME_START_DELAY] = definition.delay;
        times[TIME_ITERATION_DURATION] = definition.duration;
        times[TIME_ITERATION_COUNT] = definition.iteration_count;
        times[TIME_PLAYBACK_RATE] = 1.0;
        Some(Self {
            flags: flag::HAS_HOLD_TIME
                | timeline_flags
                | flag::HAS_OWNING_ELEMENT
                // The plan starts this animation into the place the definition holds, so the
                // element lists it there.
                | flag::LISTED_BY_OWNING_ELEMENT
                | pending_task
                | (fill_mode << flag::FILL_MODE_SHIFT)
                | (direction << flag::PLAYBACK_DIRECTION_SHIFT),
            // The document timeline's identity is never asked for: the hold time settles the
            // current time, so the row is sampled with no timeline time at all.
            timeline: 0,
            easing_interval_count: 0,
            // The effect this animation would get has no identity until the host creates it.
            effect_identity: 0,
            composite_class: animation_class::CSS_ANIMATION_WITH_OWNING_ELEMENT,
            composite_owning_slot: owning_slot,
            composite_transition_property: 0,
            composite_owning_node: owning_node.raw(),
            // The host's class-specific composite order key for a CSS animation is its place in the
            // `animation-name` list, which is the place the plan gives this definition.
            composite_class_key: name_index,
            // The global animation list orders two CSS animations only where their owning elements
            // differ, and the host has not given this one its place in the list yet.
            global_list_order: 0,
            // A CSS animation's `animation-timing-function` is applied per keyframe, so the effect's
            // own easing is always the identity `linear`.
            first_linear_point: 0,
            linear_point_count: 0,
            times,
            synthesized_index: None,
        })
    }

    /// The row `Animation::style_timing_row()` publishes for a CSS transition the moment it starts:
    /// played, so held at time zero with its play task pending, running the effect
    /// `CSSTransition` builds - the transition's delay, its duration as one iteration, backwards
    /// fill and the transition's timing function. The linear stops of `easing` are the row's own,
    /// from zero.
    #[must_use]
    pub(crate) fn for_new_css_transition(
        owning_node: StyleNodeID,
        property_id: u16,
        delay: f64,
        active_duration: f64,
        easing: &PublishedEasing,
    ) -> Self {
        use timing_row_flag as flag;

        let mut times = [0.0; TIMING_ROW_TIMES];
        times[TIME_HOLD] = 0.0;
        times[TIME_START_DELAY] = delay;
        times[TIME_ITERATION_DURATION] = active_duration;
        times[TIME_ITERATION_COUNT] = 1.0;
        times[TIME_PLAYBACK_RATE] = 1.0;
        times[TIME_EASING_X1] = easing.x1;
        times[TIME_EASING_Y1] = easing.y1;
        times[TIME_EASING_X2] = easing.x2;
        times[TIME_EASING_Y2] = easing.y2;
        Self {
            flags: flag::HAS_HOLD_TIME
                | flag::HAS_TIMELINE
                | flag::TIMELINE_IS_MONOTONICALLY_INCREASING
                | flag::HAS_OWNING_ELEMENT
                | flag::HAS_PENDING_PLAY_TASK
                | (fill_mode::BACKWARDS << flag::FILL_MODE_SHIFT)
                | (u32::from(easing.kind) << flag::EASING_KIND_SHIFT)
                | (u32::from(easing.step_position) << flag::EASING_STEP_POSITION_SHIFT),
            timeline: 0,
            easing_interval_count: easing.interval_count,
            effect_identity: 0,
            composite_class: animation_class::CSS_TRANSITION,
            composite_owning_slot: ELEMENT_ANIMATION_SLOT,
            composite_transition_property: property_id,
            composite_owning_node: owning_node.raw(),
            composite_class_key: 0,
            global_list_order: 0,
            first_linear_point: 0,
            linear_point_count: easing.linear_points.len() as u32,
            times,
            synthesized_index: None,
        }
    }

    /// This row, standing for the `index`th animation a plan starts.
    #[must_use]
    pub(crate) fn with_synthesized_index(self, index: u32) -> Self {
        Self {
            synthesized_index: Some(index),
            ..self
        }
    }

    /// Which of a plan's starting animations this row stands for, for a row the engine synthesized
    /// rather than read from the published list.
    #[must_use]
    pub(crate) fn synthesized_index(&self) -> Option<u32> {
        self.synthesized_index
    }

    #[must_use]
    pub(crate) fn effect_identity(&self) -> u64 {
        self.effect_identity
    }

    /// The place in the element's `animation-name` list of the CSS animation this row describes,
    /// for a row that is one of the animations `(node, slot)`'s own plan works on. `None` for every
    /// other row: a transition, an animation script started, a CSS animation another element owns,
    /// and one whose owning element has stopped listing it.
    #[must_use]
    pub(crate) fn owned_css_animation_index(&self, node: StyleNodeID, slot: AnimationSlot) -> Option<u32> {
        if self.composite_class != animation_class::CSS_ANIMATION_WITH_OWNING_ELEMENT
            || !self.has(timing_row_flag::HAS_OWNING_ELEMENT)
            || !self.has(timing_row_flag::LISTED_BY_OWNING_ELEMENT)
            || self.composite_owning_node != node.raw()
            || self.composite_owning_slot != slot
        {
            return None;
        }
        Some(self.composite_class_key)
    }

    /// This row with the timing a definition that moves no time stamps on it: the three specified
    /// times and the two flag fields `apply_css_properties` sets through the effect's plain setters.
    /// `None` for a definition whose direction or fill mode is not one of the CSS keywords, and for
    /// a row whose times are percentages of a progress-based timeline.
    #[must_use]
    pub(crate) fn retimed_for_definition(
        &self,
        definition: &crate::css::style_compute::FfiComputedAnimation,
    ) -> Option<Self> {
        use timing_row_flag as flag;

        let direction = css_playback_direction(definition.direction)?;
        let fill_mode = css_fill_mode(definition.fill_mode)?;
        if self.flags
            & (flag::START_DELAY_IS_PERCENTAGE | flag::ITERATION_DURATION_IS_PERCENTAGE | flag::END_DELAY_IS_PERCENTAGE)
            != 0
        {
            return None;
        }
        let mut retimed = *self;
        retimed.times[TIME_START_DELAY] = definition.delay;
        retimed.times[TIME_ITERATION_DURATION] = definition.duration;
        retimed.times[TIME_ITERATION_COUNT] = definition.iteration_count;
        retimed.flags &= !((flag::FILL_MODE_MASK << flag::FILL_MODE_SHIFT)
            | (flag::PLAYBACK_DIRECTION_MASK << flag::PLAYBACK_DIRECTION_SHIFT));
        retimed.flags |= (fill_mode << flag::FILL_MODE_SHIFT) | (direction << flag::PLAYBACK_DIRECTION_SHIFT);
        Some(retimed)
    }

    /// This row once a definition that changes `animation-play-state` is applied: what
    /// `CSSAnimation::apply_css_properties` does to an animation script has not played or paused,
    /// which is to play it when the definition says `running` and it is not running, and to pause
    /// it when the definition says `paused` and it is not paused. `timeline_time` is the time of
    /// the row's timeline the host reads.
    ///
    /// `None` for a row whose play or pause this cannot settle: one on a progress-based or
    /// non-monotonic timeline, one whose times are percentages, and one the host would fail to
    /// rewind or pause over an infinite effect end.
    #[must_use]
    pub(crate) fn with_css_play_state(&self, running: bool, timeline_time: Option<TimeValue>) -> Option<Self> {
        use timing_row_flag as flag;

        if self.has(flag::CSS_PLAY_STATE_OVERRIDDEN_BY_SCRIPT) {
            return Some(*self);
        }
        if self.has(flag::UNDECIDABLE)
            || self.has(flag::TIMELINE_IS_PROGRESS_BASED)
            || (self.has(flag::HAS_TIMELINE) && !self.has(flag::TIMELINE_IS_MONOTONICALLY_INCREASING))
            || self.flags
                & (flag::START_TIME_IS_PERCENTAGE
                    | flag::HOLD_TIME_IS_PERCENTAGE
                    | flag::START_DELAY_IS_PERCENTAGE
                    | flag::ITERATION_DURATION_IS_PERCENTAGE
                    | flag::END_DELAY_IS_PERCENTAGE)
                != 0
        {
            return None;
        }
        let playback_rate = self.times[TIME_PLAYBACK_RATE];
        // https://www.w3.org/TR/web-animations-1/#animation-current-time
        let current_time = if self.has(flag::HAS_HOLD_TIME) {
            Some(self.times[TIME_HOLD])
        } else if !self.has(flag::HAS_TIMELINE) || !self.has(flag::HAS_START_TIME) {
            None
        } else {
            timeline_time.map(|time| (time.value - self.times[TIME_START]) * playback_rate)
        };
        // https://www.w3.org/TR/web-animations-1/#associated-effect-end
        let iteration_duration = self.times[TIME_ITERATION_DURATION];
        let iteration_count = self.times[TIME_ITERATION_COUNT];
        let active_duration = match iteration_duration == 0.0 || iteration_count == 0.0 {
            true => 0.0,
            false => iteration_duration * iteration_count,
        };
        let effect_end = (self.times[TIME_START_DELAY] + active_duration + self.times[TIME_END_DELAY]).max(0.0);
        let effective_playback_rate = match self.has(flag::HAS_PENDING_PLAYBACK_RATE) {
            true => self.times[TIME_PENDING_PLAYBACK_RATE],
            false => playback_rate,
        };
        let has_start_time = self.has(flag::HAS_START_TIME);
        let pending_play = self.has(flag::HAS_PENDING_PLAY_TASK);
        let pending_pause = self.has(flag::HAS_PENDING_PAUSE_TASK);
        // https://www.w3.org/TR/web-animations-1/#play-states
        let is_paused = pending_pause || (!has_start_time && !pending_play);
        let is_idle = current_time.is_none() && !has_start_time && !pending_play && !pending_pause;
        let is_finished = current_time.is_some_and(|time| {
            (effective_playback_rate > 0.0 && time >= effect_end) || (effective_playback_rate < 0.0 && time <= 0.0)
        });
        let is_running = !is_idle && !is_paused && !is_finished;

        let mut row = *self;
        let set_hold_time = |row: &mut Self, time: f64| {
            row.flags |= flag::HAS_HOLD_TIME;
            row.times[TIME_HOLD] = time;
        };
        if running {
            if is_running {
                return Some(row);
            }
            // https://drafts.csswg.org/web-animations-2/#play-an-animation, with the auto-rewind
            // flag set and no finite timeline.
            let aborted_pause = pending_pause;
            if effective_playback_rate > 0.0 && current_time.is_none_or(|time| time < 0.0 || time >= effect_end) {
                set_hold_time(&mut row, 0.0);
            } else if effective_playback_rate < 0.0 && current_time.is_none_or(|time| time <= 0.0 || time > effect_end)
            {
                if effect_end.is_infinite() {
                    return None;
                }
                set_hold_time(&mut row, effect_end);
            } else if effective_playback_rate == 0.0 && current_time.is_none() {
                set_hold_time(&mut row, 0.0);
            }
            if row.has(flag::HAS_HOLD_TIME) {
                row.flags &= !flag::HAS_START_TIME;
            }
            row.flags &= !(flag::HAS_PENDING_PLAY_TASK | flag::HAS_PENDING_PAUSE_TASK);
            if !row.has(flag::HAS_HOLD_TIME) && !aborted_pause && !row.has(flag::HAS_PENDING_PLAYBACK_RATE) {
                return Some(row);
            }
            // Updating the finished state with a pending play task moves no time.
            row.flags |= flag::HAS_PENDING_PLAY_TASK;
            return Some(row);
        }
        // https://www.w3.org/TR/web-animations-1/#pausing-an-animation-section
        if pending_pause || is_paused {
            return Some(row);
        }
        if current_time.is_none() {
            let seek_time = match playback_rate >= 0.0 {
                true => 0.0,
                false if effect_end.is_infinite() => return None,
                false => effect_end,
            };
            set_hold_time(&mut row, seek_time);
        }
        // Updating the finished state with a pending pause task moves no time.
        row.flags &= !flag::HAS_PENDING_PLAY_TASK;
        row.flags |= flag::HAS_PENDING_PAUSE_TASK;
        Some(row)
    }

    /// Whether the element the row was published for holds the animation, rather than a pass having
    /// started it provisionally.
    #[must_use]
    pub(crate) fn is_associated(&self) -> bool {
        !self.has(timing_row_flag::NOT_ASSOCIATED)
    }

    /// Whether the animation holds its current time, which no timeline's time then moves.
    #[must_use]
    pub(crate) fn has_hold_time(&self) -> bool {
        self.has(timing_row_flag::HAS_HOLD_TIME)
    }

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

/// `AnimationEffect::Phase`.
#[derive(Clone, Copy, PartialEq)]
enum Phase {
    Before,
    Active,
    After,
    Idle,
}

/// A mirror of `AnimationEffect::ResolvedTiming`, with what `Animation` contributes to it.
#[derive(Clone, Copy)]
struct ResolvedTiming {
    phase: Phase,
    active_time: Option<TimeValue>,
    active_duration: TimeValue,
    iteration_duration: TimeValue,
    iteration_count: f64,
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
        active_time,
        active_duration,
        iteration_duration,
        iteration_count,
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
pub(crate) fn row_current_key(
    row: &AnimationTimingRow,
    linear_points: &[crate::css::easing::FfiLinearEasingPoint],
    timeline_time: Option<TimeValue>,
) -> Option<Option<f64>> {
    use timing_row_flag as flag;

    if row.has(flag::UNDECIDABLE) {
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
    let output_progress = row_easing_output(row, linear_points, directed_progress, before_flag)?;

    // `AnimationKeyFrameKeyScaleFactor`, and the host's clamp to what an `i64` key can hold.
    let key = output_progress * 100.0 * 1000.0;
    Some(Some(key.clamp(i64::MIN as f64, i64::MAX as f64)))
}

/// The row's own easing at `progress`: `EasingFunction::evaluate_at()`. `None` for an easing the
/// row has no room to spell out.
#[must_use]
pub(crate) fn row_easing_output(
    row: &AnimationTimingRow,
    linear_points: &[crate::css::easing::FfiLinearEasingPoint],
    progress: f64,
    before_flag: bool,
) -> Option<f64> {
    use timing_row_flag as flag;

    let easing_kind = (row.flags >> flag::EASING_KIND_SHIFT) & flag::EASING_KIND_MASK;
    let output_progress = match easing_kind {
        // `linear()`, whose stops the row names by range in the list's shared buffer. An empty
        // range is `linear` itself, which the host holds as `linear(0, 1)`.
        0 => crate::css::easing::evaluate_linear_easing(
            match row.linear_point_count {
                0 => &[
                    crate::css::easing::FfiLinearEasingPoint {
                        input: 0.0,
                        output: 0.0,
                    },
                    crate::css::easing::FfiLinearEasingPoint {
                        input: 1.0,
                        output: 1.0,
                    },
                ],
                count => linear_points
                    .get(row.first_linear_point as usize..)?
                    .get(..count as usize)?,
            },
            progress,
            before_flag,
        ),
        1 => crate::css::easing::evaluate_cubic_bezier_easing(
            row.times[TIME_EASING_X1],
            row.times[TIME_EASING_Y1],
            row.times[TIME_EASING_X2],
            row.times[TIME_EASING_Y2],
            progress,
        ),
        2 => crate::css::easing::evaluate_steps_easing(
            row.easing_interval_count,
            ((row.flags >> flag::EASING_STEP_POSITION_SHIFT) & flag::EASING_STEP_POSITION_MASK) as u8,
            progress,
            before_flag,
        ),
        _ => return None,
    };
    Some(output_progress)
}

/// Whether the animation a row describes is neither finished nor idle, which is what makes a CSS
/// transition a running one: a mirror of `Animation::play_state_at()` at the timeline's time.
/// `None` where the host's arithmetic would refuse to mix two times' kinds.
#[must_use]
pub(crate) fn row_plays_unfinished(row: &AnimationTimingRow, timeline_time: Option<TimeValue>) -> Option<bool> {
    use timing_row_flag as flag;

    let progress_based = row.has(flag::TIMELINE_IS_PROGRESS_BASED);
    let zero = TimeValue::zero(progress_based);
    let current_time = if row.has(flag::HAS_HOLD_TIME) {
        Some(row.time(TIME_HOLD, flag::HOLD_TIME_IS_PERCENTAGE))
    } else if !row.has(flag::HAS_TIMELINE) || timeline_time.is_none() || !row.has(flag::HAS_START_TIME) {
        None
    } else {
        let start_time = row.time(TIME_START, flag::START_TIME_IS_PERCENTAGE);
        Some(
            timeline_time?
                .subtract(start_time)?
                .scale(row.times[TIME_PLAYBACK_RATE]),
        )
    };
    let pending = row.has(flag::HAS_PENDING_PLAY_TASK) || row.has(flag::HAS_PENDING_PAUSE_TASK);
    // https://www.w3.org/TR/web-animations-1/#play-states
    if current_time.is_none() && !row.has(flag::HAS_START_TIME) && !pending {
        return Some(false);
    }
    if row.has(flag::HAS_PENDING_PAUSE_TASK)
        || (!row.has(flag::HAS_START_TIME) && !row.has(flag::HAS_PENDING_PLAY_TASK))
    {
        return Some(true);
    }
    let Some(current_time) = current_time else {
        return Some(true);
    };
    let effective_playback_rate = match row.has(flag::HAS_PENDING_PLAYBACK_RATE) {
        true => row.times[TIME_PENDING_PLAYBACK_RATE],
        false => row.times[TIME_PLAYBACK_RATE],
    };
    // https://www.w3.org/TR/web-animations-1/#associated-effect-end
    let iteration_duration = row.time(TIME_ITERATION_DURATION, flag::ITERATION_DURATION_IS_PERCENTAGE);
    let iteration_count = row.times[TIME_ITERATION_COUNT];
    let active_duration = if iteration_duration.value == 0.0 || iteration_count == 0.0 {
        zero
    } else {
        iteration_duration.scale(iteration_count)
    };
    let effect_end = row
        .time(TIME_START_DELAY, flag::START_DELAY_IS_PERCENTAGE)
        .add(active_duration)?
        .add(row.time(TIME_END_DELAY, flag::END_DELAY_IS_PERCENTAGE))?
        .largest(zero)?;
    let finished = (effective_playback_rate > 0.0 && current_time.compare(effect_end)?.is_ge())
        || (effective_playback_rate < 0.0 && current_time.value <= 0.0);
    Some(!finished)
}

/// One effect a sample composes, as the engine chooses it from an element's published timing rows:
/// the effect's identity and the key its keyframes are sampled at.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct RowSelectedEffect {
    pub(crate) identity: u64,
    pub(crate) current_key: f64,
}

/// `Bindings::PlaybackDirection` of an `animation-direction` keyword: the IDL order is not the
/// order the CSS keywords are in. A mirror of `css_animation_direction_to_playback_direction`.
fn css_playback_direction(direction: u8) -> Option<u32> {
    match direction {
        0 => Some(2), // alternate
        1 => Some(3), // alternate-reverse
        2 => Some(0), // normal
        3 => Some(1), // reverse
        _ => None,
    }
}

/// `Bindings::FillMode` of an `animation-fill-mode` keyword. A mirror of
/// `css_fill_mode_to_bindings_fill_mode`.
fn css_fill_mode(fill_mode: u8) -> Option<u32> {
    match fill_mode {
        0 => Some(2), // backwards
        1 => Some(3), // both
        2 => Some(1), // forwards
        3 => Some(0), // none
        _ => None,
    }
}

/// The rows an element would publish once a plan that cancels and renumbers its CSS animations
/// has been applied: a cancelled animation drops out of the effect stack, one the plan moved takes
/// its new place in it, and the composite order is redone over what is left.
///
/// `new_indices[j]` is the place `animation-name` order gives the element's `j`th CSS animation,
/// or `NO_MATCHED_ANIMATION` for one no definition claimed and the plan therefore cancels.
/// Everything else the element holds - its transitions, the animations script started - the plan
/// does not touch. `None` where the published rows are not the list the plan is about.
#[must_use]
pub(crate) fn rows_after_cancel_and_renumber(
    rows: &[AnimationTimingRow],
    node: StyleNodeID,
    slot: AnimationSlot,
    new_indices: &[i32],
) -> Option<Vec<AnimationTimingRow>> {
    let mut planned = Vec::with_capacity(rows.len());
    let mut was_found = vec![false; new_indices.len()];
    for row in rows {
        let Some(existing) = row.owned_css_animation_index(node, slot) else {
            planned.push(*row);
            continue;
        };
        let existing = existing as usize;
        if *was_found.get(existing)? {
            return None;
        }
        was_found[existing] = true;
        let new_index = new_indices[existing];
        if new_index == NO_MATCHED_ANIMATION {
            continue;
        }
        let mut planned_row = *row;
        planned_row.composite_class_key = new_index as u32;
        planned.push(planned_row);
    }
    if was_found.iter().any(|found| !found) {
        return None;
    }
    // The published list is already in composite order, so a stable sort keeps the relative order
    // of the rows the order declines to tell apart.
    planned.sort_by(composite_order);
    Some(planned)
}

/// The rows an element would publish once a plan that also starts animations has been applied:
/// the rows the plan leaves behind, with the ones synthesized for the animations it starts merged
/// into the composite order.
#[must_use]
pub(crate) fn rows_with_synthesized(
    planned: Vec<AnimationTimingRow>,
    synthesized: &[AnimationTimingRow],
) -> Vec<AnimationTimingRow> {
    let mut rows = planned;
    rows.extend_from_slice(synthesized);
    rows.sort_by(composite_order);
    rows
}

/// The effects of one of an element's animation lists that a sample composes, in the order it
/// composes them, chosen from the published timing rows alone.
///
/// A mirror of the list the host's sample walks: the provisionally started transitions, then the
/// animations associated with the element in composite order - which is the order the rows are
/// published in - keeping each effect whose transformed progress resolves. An idle animation's
/// progress never resolves, so it is dropped with the rest. `None` where a row declines to be
/// decided, or needs the time of a timeline no sample was published for.
#[must_use]
pub(crate) fn select_sampled_effects(
    rows: &[AnimationTimingRow],
    linear_points: &[crate::css::easing::FfiLinearEasingPoint],
    samples: &AnimationTimelineSamples,
) -> Option<Vec<RowSelectedEffect>> {
    let mut selected = Vec::with_capacity(rows.len());
    for row in rows {
        // A hold time is the current time, whatever the timeline's is - which for a timeline this
        // update associated with the document, a brand-new animation's most of all, was never
        // sampled.
        let timeline_time = match row.has(timing_row_flag::HAS_HOLD_TIME) {
            true => None,
            false => row_timeline_time(row, samples)?,
        };
        let Some(current_key) = row_current_key(row, linear_points, timeline_time)? else {
            continue;
        };
        selected.push(RowSelectedEffect {
            identity: row.effect_identity,
            current_key,
        });
    }
    Some(selected)
}

/// Per element and pseudo-element, the timing of every animation the host holds a keyframe effect
/// for, published whole whenever any of it can have changed.
/// One element's published list: the rows, and the `linear()` stops the rows name by range.
#[derive(PartialEq)]
struct PublishedTimingRows {
    rows: Box<[AnimationTimingRow]>,
    linear_points: Box<[crate::css::easing::FfiLinearEasingPoint]>,
}

#[derive(Default)]
pub(crate) struct AnimationTimingRows {
    rows: HashMap<(StyleNodeID, AnimationSlot), PublishedTimingRows>,
}

impl AnimationTimingRows {
    /// Replace one list, from the three buffers the host packs it into. An empty list drops the row.
    pub(crate) fn set(
        &mut self,
        node: StyleNodeID,
        slot: AnimationSlot,
        words: &[u32],
        times: &[f64],
        linear_points: &[f64],
    ) {
        if words.is_empty() {
            self.rows.remove(&(node, slot));
            return;
        }
        let count = words.len() / TIMING_ROW_WORDS;
        assert!(
            times.len() == count * TIMING_ROW_TIMES,
            "an animation timing row has thirteen times"
        );
        assert!(
            linear_points.len().is_multiple_of(2),
            "a linear easing stop is an input and an output"
        );
        let mut rows = Vec::with_capacity(count);
        for index in 0..count {
            let mut row = AnimationTimingRow::from_words(&words[index * TIMING_ROW_WORDS..][..TIMING_ROW_WORDS]);
            row.times
                .copy_from_slice(&times[index * TIMING_ROW_TIMES..][..TIMING_ROW_TIMES]);
            rows.push(row);
        }
        let published = PublishedTimingRows {
            rows: rows.into_boxed_slice(),
            linear_points: linear_points
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&[input, output]| crate::css::easing::FfiLinearEasingPoint { input, output })
                .collect(),
        };
        // Republishing an unchanged list is the common case - the host cannot cheaply tell that
        // nothing moved - so compare before giving up the allocation the engine already holds.
        match self.rows.get(&(node, slot)) {
            Some(existing) if *existing == published => {}
            _ => {
                self.rows.insert((node, slot), published);
            }
        }
    }

    #[must_use]
    pub(crate) fn rows(&self, node: StyleNodeID, slot: AnimationSlot) -> &[AnimationTimingRow] {
        self.rows
            .get(&(node, slot))
            .map_or(&[][..], |published| &published.rows[..])
    }

    /// The `linear()` stops this list's rows name by range.
    #[must_use]
    pub(crate) fn linear_points(
        &self,
        node: StyleNodeID,
        slot: AnimationSlot,
    ) -> &[crate::css::easing::FfiLinearEasingPoint] {
        self.rows
            .get(&(node, slot))
            .map_or(&[][..], |published| &published.linear_points[..])
    }

    /// The row of one effect, which the stage names by the identity it already uses to look its
    /// description up. The list is published in composite order, but the stage holds a subset of
    /// it, so a position in the stage's own list is not an answer.
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

/// Whether two rows name different owning elements, which is what the class-specific composite
/// order of a CSS animation and of a CSS transition asks first.
#[must_use]
fn owning_element_differs(a: &AnimationTimingRow, b: &AnimationTimingRow) -> bool {
    a.composite_owning_node != b.composite_owning_node || a.composite_owning_slot != b.composite_owning_slot
}

/// A mirror of `KeyframeEffect::composite_order()` over two published rows, plus the one rule the
/// published list adds on top of it: a provisionally started transition is not in the element's
/// effect stack yet, and composes below every effect that is, in the order it was started. The
/// caller's sort must be stable.
///
/// Where the spec asks for the tree order of two differing owning elements, the host has a `FIXME`
/// that returns 0 and leaves the global animation list to decide. That is mirrored as it stands -
/// the point here is to preserve the host's order exactly, not to fix it.
#[must_use]
pub(crate) fn composite_order(a: &AnimationTimingRow, b: &AnimationTimingRow) -> std::cmp::Ordering {
    use crate::css::property_metadata::property_name;
    use std::cmp::Ordering;
    use timing_row_flag as flag;

    match (a.has(flag::NOT_ASSOCIATED), b.has(flag::NOT_ASSOCIATED)) {
        (false, false) => {}
        // The host samples its provisional transitions in the order it started them, which is the
        // order it publishes them in, and the sort keeps it.
        (true, true) => return Ordering::Equal,
        // It is the provisional transition that composes below.
        (true, false) => return Ordering::Less,
        (false, true) => return Ordering::Greater,
    }

    // 1. Animations that differ by class are sorted by the inter-class composite order.
    if a.composite_class != b.composite_class {
        return a.composite_class.cmp(&b.composite_class);
    }

    // 2. Otherwise by the class-specific composite order of their common class.
    let class_specific = match a.composite_class {
        animation_class::CSS_ANIMATION_WITH_OWNING_ELEMENT => match owning_element_differs(a, b) {
            true => Ordering::Equal,
            false => a.composite_class_key.cmp(&b.composite_class_key),
        },
        animation_class::CSS_TRANSITION => {
            let a_owns = a.has(flag::HAS_OWNING_ELEMENT);
            let b_owns = b.has(flag::HAS_OWNING_ELEMENT);
            if !a_owns && !b_owns {
                a.global_list_order.cmp(&b.global_list_order)
            } else if a_owns != b_owns {
                // The one with an owning element sorts first.
                match a_owns {
                    true => Ordering::Less,
                    false => Ordering::Greater,
                }
            } else if owning_element_differs(a, b) {
                Ordering::Equal
            } else if a.composite_class_key != b.composite_class_key {
                a.composite_class_key.cmp(&b.composite_class_key)
            } else {
                property_name(a.composite_transition_property).cmp(property_name(b.composite_transition_property))
            }
        }
        animation_class::CSS_ANIMATION_WITHOUT_OWNING_ELEMENT => a.global_list_order.cmp(&b.global_list_order),
        _ => Ordering::Equal,
    };

    // 3. Otherwise by the position of the animations in the global animation list.
    match class_specific {
        Ordering::Equal => a.global_list_order.cmp(&b.global_list_order),
        order => order,
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

/// The font metrics a `rem` resolves against, as the host last left them.
///
/// The host keeps these in a member it refreshes only while the document element itself is
/// computed, so a document whose root was never recomputed still resolves every `rem` against the
/// default font. The row is therefore published at each of the host's writes rather than derived
/// from the root element's committed style: a stage that read the committed style would disagree
/// with the host wherever the member is stale.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct RootElementFontMetrics {
    pub(crate) font_size: f64,
    pub(crate) x_height: f64,
    pub(crate) cap_height: f64,
    pub(crate) zero_advance: f64,
    pub(crate) line_height: f64,
    pub(crate) depends_on_viewport_metrics: bool,
}

impl RootElementFontMetrics {
    /// The five metrics arrive as the bit patterns of their doubles, the way the style record's
    /// cache key already packs them.
    pub(crate) const WORDS: usize = 5;

    pub(crate) fn from_words(words: &[u64], depends_on_viewport_metrics: bool) -> Self {
        assert!(words.len() == Self::WORDS);
        Self {
            font_size: f64::from_bits(words[0]),
            x_height: f64::from_bits(words[1]),
            cap_height: f64::from_bits(words[2]),
            zero_advance: f64::from_bits(words[3]),
            line_height: f64::from_bits(words[4]),
            depends_on_viewport_metrics,
        }
    }
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
    pub(crate) const HAS_RESOURCE_CONTEXT: u32 = 1 << 2;
    pub(crate) const RESOURCE_CONTEXT_IS_ORIGIN_CLEAN: u32 = 1 << 3;
}

/// One easing function, as the host resolved it when it described the effect. The linear points are
/// owned so a published keyframe can hand out an `FfiEasingDescriptor` that borrows them.
#[derive(Clone)]
pub(crate) struct PublishedEasing {
    kind: u8,
    linear_points: Box<[crate::css::easing::FfiLinearEasingPoint]>,
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    interval_count: i32,
    step_position: u8,
}

impl PublishedEasing {
    /// `linear`, spelled out as `linear(0, 1)` the way the host describes a keyframe that runs it.
    #[must_use]
    pub(crate) fn linear() -> Self {
        Self {
            kind: 0,
            linear_points: Box::new([
                crate::css::easing::FfiLinearEasingPoint {
                    input: 0.0,
                    output: 0.0,
                },
                crate::css::easing::FfiLinearEasingPoint {
                    input: 1.0,
                    output: 1.0,
                },
            ]),
            x1: 0.0,
            y1: 0.0,
            x2: 0.0,
            y2: 0.0,
            interval_count: 0,
            step_position: 0,
        }
    }

    #[must_use]
    pub(crate) fn linear_points(&self) -> &[crate::css::easing::FfiLinearEasingPoint] {
        &self.linear_points
    }

    /// The easing a computed `animation-timing-function` describes, which fills in the hole a
    /// keyframe with no easing of its own keeps. A mirror of `EasingFunction::from_style_value`.
    #[must_use]
    pub(crate) fn from_computed_timing_function(value: &crate::css::style_value::StyleValueData) -> Option<Self> {
        use crate::css::style_value::StyleValueData;
        use crate::layout::keyword;

        let cubic_bezier = |x1, y1, x2, y2| Self {
            kind: 1,
            linear_points: Box::new([]),
            x1,
            y1,
            x2,
            y2,
            interval_count: 0,
            step_position: 0,
        };
        match value {
            // https://drafts.csswg.org/css-easing-2/#typedef-easing-function
            StyleValueData::Keyword { keyword } => match *keyword {
                keyword::LINEAR => Some(Self {
                    kind: 0,
                    // `linear` is `linear(0, 1)`, the identity curve, spelled out the way the host
                    // spells it out when it describes a keyframe that runs it.
                    linear_points: Box::new([
                        crate::css::easing::FfiLinearEasingPoint {
                            input: 0.0,
                            output: 0.0,
                        },
                        crate::css::easing::FfiLinearEasingPoint {
                            input: 1.0,
                            output: 1.0,
                        },
                    ]),
                    x1: 0.0,
                    y1: 0.0,
                    x2: 0.0,
                    y2: 0.0,
                    interval_count: 0,
                    step_position: 0,
                }),
                keyword::EASE => Some(cubic_bezier(0.25, 0.1, 0.25, 1.0)),
                keyword::EASE_IN => Some(cubic_bezier(0.42, 0.0, 1.0, 1.0)),
                keyword::EASE_OUT => Some(cubic_bezier(0.0, 0.0, 0.58, 1.0)),
                keyword::EASE_IN_OUT => Some(cubic_bezier(0.42, 0.0, 0.58, 1.0)),
                _ => None,
            },
            StyleValueData::Easing {
                kind,
                step_position,
                x1,
                y1,
                x2,
                y2,
                number_of_intervals,
                ..
            } => {
                // Each argument reads the way the host's `numeric()` reads it, which resolves a
                // calculation on the spot.
                let numeric = |value: &crate::css::style_value::RetainedStyleValueData| match value.data() {
                    StyleValueData::Number { value } => Some(*value),
                    StyleValueData::Integer { value } => Some(*value as f64),
                    StyleValueData::Percentage { value } => Some(*value),
                    calculated @ StyleValueData::Calculated { .. } => {
                        crate::css::calc::resolve_calculated_number_without_context(calculated)
                            .or_else(|| crate::css::calc::resolve_calculated_percentage_without_context(calculated))
                    }
                    _ => None,
                };
                match kind {
                    0 => {
                        // The stops are canonicalized first, which resolves each one's calculated
                        // values and interpolates the inputs it was not given.
                        unsafe extern "C" fn retain_child(
                            _: *const std::ffi::c_void,
                            child: &StyleValueData,
                        ) -> *const StyleValueData {
                            unsafe { crate::css::style_value::retain_style_value(child) }
                        }
                        // SAFETY: the absolutization hands back one reference, which the retained
                        //         value below owns.
                        let canonical = unsafe {
                            crate::css::style_value::RetainedStyleValueData::from_retained_pointer(
                                crate::css::absolutize::rust_composite_style_value_absolutize(
                                    value,
                                    std::ptr::null(),
                                    retain_child,
                                ),
                            )
                        };
                        let StyleValueData::Easing { linear_stops, .. } = canonical.data() else {
                            return None;
                        };
                        let linear_points = linear_stops
                            .as_slice()
                            .iter()
                            .map(|stop| {
                                Some(crate::css::easing::FfiLinearEasingPoint {
                                    input: numeric(stop.input())? / 100.0,
                                    output: numeric(stop.output())?,
                                })
                            })
                            .collect::<Option<Box<[_]>>>()?;
                        Some(Self {
                            kind: 0,
                            linear_points,
                            x1: 0.0,
                            y1: 0.0,
                            x2: 0.0,
                            y2: 0.0,
                            interval_count: 0,
                            step_position: 0,
                        })
                    }
                    1 => Some(cubic_bezier(numeric(x1)?, numeric(y1)?, numeric(x2)?, numeric(y2)?)),
                    2 => Some(Self {
                        kind: 2,
                        linear_points: Box::new([]),
                        x1: 0.0,
                        y1: 0.0,
                        x2: 0.0,
                        y2: 0.0,
                        #[expect(clippy::cast_possible_truncation)]
                        interval_count: numeric(number_of_intervals)?.round_ties_even() as i32,
                        step_position: *step_position,
                    }),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    #[must_use]
    pub(crate) fn descriptor(&self) -> crate::css::easing::FfiEasingDescriptor {
        use crate::css::easing::{FfiEasingDescriptor, FfiEasingKind};
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
    /// The keyframe's own easing where it still has to be substituted on the element being
    /// sampled; `easing` is then what it runs if the value resolves to none.
    pub(crate) easing_value: crate::css::style_value::RetainedStyleValueData,
    pub(crate) composite: u8,
    declaration_range: std::ops::Range<usize>,
    custom_declaration_range: std::ops::Range<usize>,
}

/// The two holes a `@keyframes` rule's own description keeps, which only the animation running it
/// can fill: a keyframe with no easing runs the animation's `animation-timing-function`, and one
/// that says `composite: auto` composites the way its effect does. An element's effect descriptions
/// never carry them - the host fills both in as it describes the effect. Mirrored in
/// `CSS/StyleEngineInput.cpp`; keep the two in step.
pub(crate) const KEYFRAME_EASING_FROM_ANIMATION: u8 = 3;
pub(crate) const KEYFRAME_COMPOSITE_FROM_ANIMATION: u8 = 0xff;

impl PublishedKeyframe {
    /// This keyframe's easing, with the hole a `@keyframes` rule keeps filled in from the animation
    /// running it.
    #[must_use]
    pub(crate) fn easing_from<'a>(&'a self, animation: &'a PublishedEasing) -> &'a PublishedEasing {
        match self.easing.kind {
            KEYFRAME_EASING_FROM_ANIMATION => animation,
            _ => &self.easing,
        }
    }

    /// This keyframe's composite operation, with the same hole filled in.
    #[must_use]
    pub(crate) fn composite_from(&self, animation: u8) -> u8 {
        match self.composite {
            KEYFRAME_COMPOSITE_FROM_ANIMATION => animation,
            composite => composite,
        }
    }
}

/// One property a published keyframe declares. `use_initial` marks the keyframe the host synthesized
/// to hold the element's own value, whose value is not known until the element is sampled.
pub(crate) struct PublishedDeclaration {
    pub(crate) property_id: u16,
    pub(crate) use_initial: bool,
    pub(crate) value: crate::css::style_value::RetainedStyleValueData,
}

/// One custom property a published keyframe declares. The name is retained, because a description
/// outlives the call that published it and a fly string's raw representation is only an identity
/// while the string is alive. `use_initial` marks the keyframe the host synthesized to hold the
/// element's underlying value for the name, which is not known until the element is sampled, and
/// then there is no value.
pub(crate) struct PublishedCustomDeclaration {
    pub(crate) name: crate::css::retained_fly_string::RetainedUtf16FlyString,
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
    pub(crate) custom_declarations: Box<[PublishedCustomDeclaration]>,
}

impl PublishedEffect {
    /// The effect `CSSTransition` builds for a transition it starts: the start value at the first
    /// keyframe and the end value at the last, each running `linear` and replacing, described as
    /// the host describes it.
    #[must_use]
    pub(crate) fn for_css_transition(
        property_id: u16,
        start_value: crate::css::style_value::RetainedStyleValueData,
        end_value: crate::css::style_value::RetainedStyleValueData,
    ) -> Self {
        let keyframe = |key, declaration| PublishedKeyframe {
            key,
            easing: PublishedEasing::linear(),
            easing_value: crate::css::style_value::RetainedStyleValueData::none(),
            composite: 0,
            declaration_range: declaration..declaration + 1,
            custom_declaration_range: 0..0,
        };
        let declaration = |value| PublishedDeclaration {
            property_id,
            use_initial: false,
            value,
        };
        Self {
            identity: 0,
            generation: 0,
            flags: effect_flag::IS_TRANSITION,
            base_url: Box::new([]),
            keyframes: Box::new([keyframe(0, 0), keyframe(100 * 1000, 1)]),
            declarations: Box::new([declaration(start_value), declaration(end_value)]),
            custom_declarations: Box::new([]),
        }
    }

    #[must_use]
    pub(crate) fn declarations_of(&self, keyframe: &PublishedKeyframe) -> &[PublishedDeclaration] {
        &self.declarations[keyframe.declaration_range.clone()]
    }

    #[must_use]
    pub(crate) fn custom_declarations_of(&self, keyframe: &PublishedKeyframe) -> &[PublishedCustomDeclaration] {
        &self.custom_declarations[keyframe.custom_declaration_range.clone()]
    }

    /// Whether any keyframe of this effect declares a custom property. The host's own fast path
    /// through a description resolves longhands alone, so it refuses such an effect and walks the
    /// keyframe sets as it always has; the stage's tail samples it.
    #[must_use]
    pub(crate) fn declares_custom_properties(&self) -> bool {
        !self.custom_declarations.is_empty()
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
    pub custom_declarations: &'a [super::bridge::FfiPublishedAnimationCustomDeclaration],
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
            custom_declarations,
            linear_points,
            base_url_bytes,
        } = published_buffers;
        if effects.is_empty() {
            self.rows.remove(&(node, slot));
            return;
        }
        let published = unsafe {
            build_published_effects(PublishedEffectBuffers {
                effects,
                keyframes,
                declarations,
                custom_declarations,
                linear_points,
                base_url_bytes,
            })
        };
        self.rows.insert((node, slot), published.into_boxed_slice());
    }

    /// Lend one list out, for a caller that samples the effects while it substitutes against the
    /// engine the list lives in. `restore` puts it back.
    pub(crate) fn take(&mut self, node: StyleNodeID, slot: AnimationSlot) -> Option<Box<[PublishedEffect]>> {
        self.rows.remove(&(node, slot))
    }

    pub(crate) fn restore(&mut self, node: StyleNodeID, slot: AnimationSlot, effects: Box<[PublishedEffect]>) {
        self.rows.insert((node, slot), effects);
    }

    /// Give up the rows of identities that have been retired, which can be minted again.
    pub(crate) fn retire(&mut self, nodes: &[StyleNodeID]) {
        if self.rows.is_empty() {
            return;
        }
        self.rows.retain(|&(node, _), _| !nodes.contains(&node));
    }
}

/// Unpack the flat buffers one list of descriptions travels in. Shared by an element's effects and
/// by the `@keyframes` a style scope defines, which are described alike: the scope's descriptions
/// are the same shape with the two per-animation holes (see `PublishedKeyframe`) left open.
///
/// # Safety
/// Every declaration's `value` must be a live style value the host holds a reference to for the
/// duration of the call.
unsafe fn build_published_effects(published_buffers: PublishedEffectBuffers<'_>) -> Vec<PublishedEffect> {
    let PublishedEffectBuffers {
        effects,
        keyframes,
        declarations,
        custom_declarations,
        linear_points,
        base_url_bytes,
    } = published_buffers;
    {
        let mut published = Vec::with_capacity(effects.len());
        for effect in effects {
            let keyframe_range =
                effect.first_keyframe as usize..(effect.first_keyframe + effect.keyframe_count) as usize;
            let mut published_keyframes = Vec::with_capacity(keyframe_range.len());
            let mut published_declarations = Vec::new();
            let mut published_custom_declarations = Vec::new();
            for keyframe in &keyframes[keyframe_range] {
                let points = linear_points[keyframe.first_linear_point as usize..]
                    [..keyframe.linear_point_count as usize]
                    .iter()
                    .map(|point| crate::css::easing::FfiLinearEasingPoint {
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
                let first_custom = published_custom_declarations.len();
                for declaration in &custom_declarations[keyframe.first_custom_declaration as usize..]
                    [..keyframe.custom_declaration_count as usize]
                {
                    // SAFETY: the caller holds a reference to the value and the name for the call,
                    //         and each retain below takes one of its own for the engine.
                    let value = match declaration.value.is_null() {
                        true => crate::css::style_value::RetainedStyleValueData::none(),
                        false => unsafe {
                            crate::css::style_value::RetainedStyleValueData::from_retained_pointer(
                                crate::css::style_value::rust_style_value_retain(declaration.value.cast()),
                            )
                        },
                    };
                    published_custom_declarations.push(PublishedCustomDeclaration {
                        name: unsafe {
                            crate::css::retained_fly_string::RetainedUtf16FlyString::from_borrowed_raw(
                                declaration.name_raw,
                            )
                        },
                        use_initial: declaration.use_initial,
                        value,
                    });
                }
                // SAFETY: the caller holds a reference to the value for the call, and
                //         `rust_style_value_retain` takes one of its own for the engine.
                let easing_value = match keyframe.easing_value.is_null() {
                    true => crate::css::style_value::RetainedStyleValueData::none(),
                    false => unsafe {
                        crate::css::style_value::RetainedStyleValueData::from_retained_pointer(
                            crate::css::style_value::rust_style_value_retain(keyframe.easing_value.cast()),
                        )
                    },
                };
                published_keyframes.push(PublishedKeyframe {
                    key: keyframe.key,
                    easing_value,
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
                    custom_declaration_range: first_custom..published_custom_declarations.len(),
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
                custom_declarations: published_custom_declarations.into_boxed_slice(),
            });
        }
        published
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
/// One `@keyframes` rule of a scope: the host's keyframe set, and what the rule declares.
pub(crate) struct PublishedKeyframesSet {
    pub(crate) pointer: usize,
}

#[derive(Default)]
pub(crate) struct AnimationKeyframes {
    /// Bumped whenever any scope's row is replaced. What an element's animations run is decided
    /// against the table as it was, so a reader that wants to know whether a plan is still the
    /// plan compares this against the generation the plan was published at.
    generation: u64,
    scopes: HashMap<TreeScopeID, HashMap<KeyframesName, PublishedKeyframesSet>>,
    /// Which scope a shadow root's host-side pointer identity names. The cascade attributes the
    /// winning `animation-name` declaration to a shadow root by that identity, and the scope it
    /// names is where the declaration's `@keyframes` are looked for first.
    scope_by_shadow_root: HashMap<usize, TreeScopeID>,
    /// What each keyframe set a scope names declares, by the pointer that names it, with how many
    /// scopes name it: the description a brand-new animation of the rule is sampled from, with the
    /// two holes a rule keeps until an animation runs it left open.
    descriptions: HashMap<usize, (PublishedEffect, u32)>,
}

impl AnimationKeyframes {
    /// Replace one scope's row. The names arrive packed into one buffer of code units with a length
    /// each, the way an element's animation names do, and each one's description in the same flat
    /// buffers an element's effect descriptions travel in, in the order the names are given.
    ///
    /// # Safety
    /// Every declaration's `value` must be a live style value the host holds a reference to for the
    /// duration of the call.
    #[must_use]
    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) unsafe fn set(
        &mut self,
        tree_scope: TreeScopeID,
        shadow_root_identity: usize,
        name_lengths: &[u32],
        name_units: &[u16],
        published_buffers: PublishedEffectBuffers<'_>,
    ) {
        assert!(
            name_lengths.len() == published_buffers.effects.len(),
            "a published @keyframes name must come with its keyframe set"
        );
        self.generation += 1;
        if let Some(previous) = self.scopes.get(&tree_scope) {
            for set in previous.values() {
                if let std::collections::hash_map::Entry::Occupied(mut entry) = self.descriptions.entry(set.pointer) {
                    entry.get_mut().1 -= 1;
                    if entry.get().1 == 0 {
                        entry.remove();
                    }
                }
            }
        }
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
        let descriptions = unsafe { build_published_effects(published_buffers) };
        let mut sets = HashMap::with_capacity(name_lengths.len());
        let mut published = HashMap::with_capacity(name_lengths.len());
        let mut offset = 0usize;
        for (&length, description) in name_lengths.iter().zip(descriptions) {
            let end = offset + length as usize;
            assert!(end <= name_units.len(), "@keyframes name lengths overrun their buffer");
            let name = KeyframesName(CssString::from_utf16(&name_units[offset..end]));
            offset = end;
            // The host names a set by its own pointer, which is what it publishes as the
            // description's identity.
            let pointer = description.identity as usize;
            sets.insert(name, PublishedKeyframesSet { pointer });
            published.insert(pointer, description);
        }
        for set in sets.values() {
            match self.descriptions.entry(set.pointer) {
                std::collections::hash_map::Entry::Occupied(mut entry) => entry.get_mut().1 += 1,
                std::collections::hash_map::Entry::Vacant(entry) => {
                    if let Some(description) = published.remove(&set.pointer) {
                        entry.insert((description, 1));
                    }
                }
            }
        }
        self.scopes.insert(tree_scope, sets);
    }

    /// What the keyframe set the host names by `pointer` declares, for as long as a scope names it.
    #[must_use]
    pub(crate) fn description(&self, pointer: usize) -> Option<&PublishedEffect> {
        self.descriptions.get(&pointer).map(|(description, _)| description)
    }

    #[must_use]
    fn in_scope(&self, tree_scope: TreeScopeID, name: &KeyframesName) -> Option<&PublishedKeyframesSet> {
        self.scopes.get(&tree_scope)?.get(name)
    }

    /// The same lookup, for a declaration whose tree scope is already known. `None` is the scope of
    /// a declaration the document or the element itself holds, which has none of its own.
    pub(crate) fn resolve_in_declaration_scope(
        &self,
        declaration_scope: Option<TreeScopeID>,
        element_tree_scope: TreeScopeID,
        name: &CssString,
    ) -> Option<&PublishedKeyframesSet> {
        if self.scopes.is_empty() {
            return None;
        }
        let name = KeyframesName(name.clone());
        if let Some(scope) = declaration_scope
            && let Some(set) = self.in_scope(scope, &name)
        {
            return Some(set);
        }
        if element_tree_scope != TreeScopeID::DOCUMENT
            && let Some(set) = self.in_scope(element_tree_scope, &name)
        {
            return Some(set);
        }
        self.in_scope(TreeScopeID::DOCUMENT, &name)
    }
}

/// The animation definitions a record the engine settled leaves for the host to apply, in the shape
/// the host's own plan application takes them in.
///
/// A record the engine settles never enters the C++ computation that would have decided the
/// element's animations beside it, so the plan is decided here and rides out of the batch as an
/// effect of the row, the way a transition step does. The host applies it once the whole batch is
/// installed, in the order the batch applied the rows.
///
/// The plan owns everything its definitions name - each name, and each computed
/// `animation-timing-function` - so none of it depends on the table the drive built them from, or
/// on the record that table went into, surviving the batch. The keyframe set each definition names
/// is the host's, which only holds the sets its scopes publish now: the plan resolves its names
/// again whenever a scope's row is replaced, so it never names a set the host has let go of.
pub(crate) struct SettledAnimationPlan {
    definitions: Box<[crate::css::style_compute::FfiComputedAnimation]>,
    /// The names the definitions point at, owned for as long as they are.
    names: Box<[CssString]>,
    #[expect(
        dead_code,
        reason = "the timing functions the definitions point at, retained for as long as they are"
    )]
    timing_functions: Box<[crate::css::style_value::RetainedStyleValueData]>,
    element_display_is_none: bool,
    /// Where the definitions' names resolve: the scope the winning `animation-name` declaration
    /// was written in, then the element's own.
    declaration_scope: Option<TreeScopeID>,
    element_tree_scope: TreeScopeID,
}

// SAFETY: Every pointer a definition holds addresses something the plan owns and only ever shares -
//         one of its own names, one of its own retained timing functions - except the keyframe-set
//         identity, which is a host pointer this side never dereferences and only hands back, as
//         `PublishedKeyframesSet` holds one, and which `resolve_keyframes_again` keeps one the
//         table publishes.
unsafe impl Send for SettledAnimationPlan {}
unsafe impl Sync for SettledAnimationPlan {}

impl SettledAnimationPlan {
    /// Assume ownership of the definitions and of the name and timing function each one names. The
    /// pointers in `definitions` must address the entries of `names` and `timing_functions`.
    pub(crate) fn new(
        definitions: Box<[crate::css::style_compute::FfiComputedAnimation]>,
        names: Box<[CssString]>,
        timing_functions: Box<[crate::css::style_value::RetainedStyleValueData]>,
        element_display_is_none: bool,
        declaration_scope: Option<TreeScopeID>,
        element_tree_scope: TreeScopeID,
    ) -> Self {
        Self {
            definitions,
            names,
            timing_functions,
            element_display_is_none,
            declaration_scope,
            element_tree_scope,
        }
    }

    /// Name the keyframe sets the table publishes now. A plan can be owed across a scope's
    /// republication, and the host lets go of the sets the scope published before, so the pointers
    /// the plan was built with may name freed sets by the time the host takes it.
    pub(crate) fn resolve_keyframes_again(&mut self, keyframes: &AnimationKeyframes) {
        for (definition, name) in self.definitions.iter_mut().zip(self.names.iter()) {
            definition.keyframe_set = keyframes
                .resolve_in_declaration_scope(self.declaration_scope, self.element_tree_scope, name)
                .map_or(std::ptr::null(), |set| set.pointer as *const std::ffi::c_void);
        }
    }

    #[must_use]
    pub(crate) fn definitions(&self) -> &[crate::css::style_compute::FfiComputedAnimation] {
        &self.definitions
    }

    /// Whether the record the row installs computes `display: none` for the element itself, which
    /// is the half of "is this element rendered" the plan can answer.
    #[must_use]
    pub(crate) fn element_display_is_none(&self) -> bool {
        self.element_display_is_none
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
/// `arena` must be the document's live layout arena, or null for a document that has none, which
/// has no committed boxes.
#[must_use]
pub(crate) unsafe fn committed_transform_reference_box(
    arena: *mut std::ffi::c_void,
    node: StyleNodeID,
) -> Option<(f64, f64)> {
    let arena = unsafe { arena.cast::<crate::layout::LayoutNodeArena>().as_ref() }?;
    let row = arena.bound_row(node);
    if row.is_invalid() || !arena.paintable_row_is_populated(row) {
        return None;
    }
    let style = arena.node_style_if_live(row)?;
    let paintable_rows = arena.paintable_rows();
    let rect = crate::painting::visual_context::node_values::transform_reference_box(style, &paintable_rows, row);
    Some((rect.width.to_double(), rect.height.to_double()))
}

/// The document's layout arena, which the host lends a style pass for the one call it blocks on, so
/// a sample the pass takes resolves a percentage translation against the boxes the last layout
/// committed. The engine never keeps it past the call.
#[derive(Clone, Copy)]
pub struct LentLayoutArena(*mut std::ffi::c_void);

// SAFETY: The host blocks on the call it lends the arena for, and the pass only reads the rows the
// last layout committed.
unsafe impl Send for LentLayoutArena {}

impl LentLayoutArena {
    /// No arena: a document that has none, which has no committed boxes.
    pub(crate) const NONE: Self = Self(std::ptr::null_mut());

    /// # Safety
    /// `arena` must be the document's live layout arena, or null, for as long as the call lasts.
    pub(crate) unsafe fn lend(arena: *mut std::ffi::c_void) -> Self {
        Self(arena)
    }

    pub(crate) fn as_ptr(self) -> *mut std::ffi::c_void {
        self.0
    }
}

/// One physical axis' container-unit basis for an element, and what resolving it says about the
/// DOM.
///
/// A mirror of the per-axis half of `Length::container_relative_length_to_px_without_rounding`:
/// the basis of `100cqw` / `100cqh` is the content size of the nearest flat-tree ancestor that
/// accepts size queries on that axis, the small viewport when the walk finds none, and zero when
/// the container it found has no box committed yet.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ContainerUnitBasis {
    pub(crate) basis: f64,
    /// Whether the basis is the viewport's, which makes every value computed from it a viewport
    /// dependency.
    pub(crate) depends_on_viewport_metrics: bool,
    /// The query container the walk landed on, which has to learn that something under it asks
    /// about its size.
    pub(crate) container: Option<StyleNodeID>,
    /// Whether that container has no committed box, so the answer is zero until layout runs.
    pub(crate) container_has_no_box: bool,
}

/// The groups an animation overlay record rebuilds over its base, and the storage their payloads
/// travel back to the host in.
pub(crate) struct AnimationOverlayPayloads {
    /// Every group's payload: the rebuilt ones, which this owns a reference to, and the base's,
    /// which the record it came from keeps alive.
    pub(crate) payloads: Vec<*const std::ffi::c_void>,
    pub(crate) rebuilt_groups: u32,
    /// Whether the overlay named a value the groups could not be told from, so every group was
    /// rebuilt.
    pub(crate) rebuilt_every_group: bool,
}

impl Drop for AnimationOverlayPayloads {
    fn drop(&mut self) {
        for (group, payload) in self.payloads.iter().enumerate() {
            if self.rebuilt_groups & (1 << group) != 0 && !payload.is_null() {
                crate::css::computed_values::release_group_payload(group, *payload);
            }
        }
    }
}

impl super::StyleEngineState {
    /// The payloads of the record an element's sampled animation overlay composes over its current
    /// record: the groups a value of the overlay lives in, and the groups that read an animated
    /// `color`, rebuilt from the longhand table with the overlay applied, and every other group the
    /// base's. `None` where the engine holds no such record.
    ///
    /// `font` supplies the platform font of the animated style where the font group has to be
    /// rebuilt; `None` where the font is not resolved leaves no payloads.
    ///
    /// # Safety
    /// `table` must be the longhand table the overlay was sampled over, and both must be live for
    /// the call.
    #[expect(
        clippy::too_many_arguments,
        reason = "the overlay, its table and the host's per-element answers travel together"
    )]
    pub(crate) unsafe fn build_animation_overlay_payloads(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
        style_record: u64,
        table: &crate::css::computed_longhand_table::ComputedLonghandTable,
        overlay: Option<&crate::css::animated_overlay::AnimatedOverlay>,
        used_color_scheme: u8,
        display_before_box_type_transformation_raw: u32,
        font: &mut dyn FnMut() -> Option<crate::css::table_group_builder::FfiFontGroupBuildInputs>,
    ) -> Option<AnimationOverlayPayloads> {
        use crate::css::computed_value_types::{
            STYLE_GROUP_INDEX_ANCHOR, STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_SURROUND,
        };
        use crate::css::table_group_builder::group_index;

        let view = self.style_record_view(style_record)?;
        let base_payloads = match view.base_payloads.is_empty() {
            true => view.payloads,
            false => view.base_payloads,
        };
        let mut result = AnimationOverlayPayloads {
            payloads: SharedPayload::as_pointer_slice(base_payloads).to_vec(),
            rebuilt_groups: 0,
            rebuilt_every_group: false,
        };
        let all_groups = (1u32 << group_index::COUNT) - 1;
        // An overlay that animates nothing leaves the base as it is: publishing it releases the
        // element's overlay record.
        let Some(overlay) = overlay.filter(|overlay| !overlay.is_empty()) else {
            result.rebuilt_every_group = true;
            return Some(result);
        };
        let mut groups = 0u32;
        for entry in overlay.entries() {
            let Some(group) = crate::css::property_metadata::property_style_group_index(entry.property) else {
                groups = all_groups;
                result.rebuilt_every_group = true;
                break;
            };
            groups |= 1 << group;
            if entry.property == crate::css::property_metadata::property_id::COLOR {
                match self.current_color_dependent_group_mask(node, pseudo_kind) {
                    Some(dependent) => groups |= dependent,
                    None => {
                        groups = all_groups;
                        result.rebuilt_every_group = true;
                        break;
                    }
                }
            }
        }
        // The surround group duplicates `position-anchor` for layout, so rebuilding the anchor
        // group refreshes it too.
        if groups & (1 << STYLE_GROUP_INDEX_ANCHOR) != 0 {
            groups |= 1 << STYLE_GROUP_INDEX_SURROUND;
        }

        // Colors resolve against the element's font metrics as they stood when the overlay was last
        // published, or against the animated font where the overlay rebuilds the font group.
        let inputs = self.document_style_computation_inputs();
        let font_inputs = match groups & (1 << STYLE_GROUP_INDEX_FONT) != 0 {
            true => Some(font()?),
            false => None,
        };
        let values = ComputedValuesView::new(SharedPayload::as_pointer_slice(view.payloads));
        let font_metrics = match &font_inputs {
            Some(font) => crate::css::style_compute::FfiFontMetrics {
                font_size: crate::css::css_pixels::CssPixels::from_raw(font.font_size_raw).to_double(),
                x_height: super::publication::drive_font_metric(font.font_x_height),
                cap_height: super::publication::drive_font_metric(font.font_ascent),
                zero_advance: super::publication::drive_font_metric(font.font_zero_advance),
                line_height: crate::css::css_pixels::CssPixels::from_raw(font.line_height_used_raw).to_double(),
            },
            None => crate::css::style_compute::FfiFontMetrics {
                font_size: values.font_size().to_double(),
                x_height: super::publication::drive_font_metric(values.font_x_height()),
                cap_height: super::publication::drive_font_metric(values.font_ascent()),
                zero_advance: super::publication::drive_font_metric(values.font_zero_advance()),
                line_height: values.line_height().to_double(),
            },
        };
        let length = crate::css::style_compute::FfiLengthResolutionContext {
            viewport_width: inputs.viewport_width,
            viewport_height: inputs.viewport_height,
            font_metrics,
            root_font_metrics: crate::css::style_compute::FfiFontMetrics {
                font_size: inputs.root_font_size,
                x_height: inputs.root_font_x_height,
                cap_height: inputs.root_font_cap_height,
                zero_advance: inputs.root_font_zero_advance,
                line_height: inputs.root_line_height,
            },
            font_metrics_depend_on_viewport_metrics: view.dependency_flags & (1 << 1) != 0,
            root_font_metrics_depend_on_viewport_metrics: inputs.root_font_metrics_depend_on_viewport_metrics,
            has_container_width_basis: false,
            has_container_height_basis: false,
            container_width_basis: 0.0,
            container_height_basis: 0.0,
            container_width_basis_depends_on_viewport_metrics: false,
            container_height_basis_depends_on_viewport_metrics: false,
            subject_inline_axis_is_horizontal: values.writing_mode()
                == crate::css::css_enums::writing_mode::HORIZONTAL_TB,
            resolved_viewport_relative_length: std::ptr::null_mut(),
        };
        // The element's own color resolves first, since every other group resolves `currentcolor`
        // against it.
        let color_value = table
            .effective_value(Some(overlay), crate::css::property_metadata::property_id::COLOR, true)
            .value;
        let mut color_input = crate::css::color_resolution::FfiColorResolutionInput {
            has_scheme: true,
            scheme: used_color_scheme,
            has_current_color: true,
            current_color_rgba: [0, 0, 0, 255],
            current_color_value: color_value,
            length: (&raw const length).cast(),
            channels_present: [false; 13],
            channels: [0.0; 13],
            has_channels: false,
        };
        let color =
            unsafe { crate::css::color_resolution::rust_style_value_to_color(color_value, &raw const color_input) };
        if color.resolved {
            color_input.current_color_rgba = color.rgba;
        }

        let build_inputs = crate::css::table_group_builder::FfiTableGroupBuildInputs {
            color_input: (&raw const color_input).cast(),
            used_color_scheme,
            animated_overlay: overlay,
            box_display_before_transformation_raw: display_before_box_type_transformation_raw,
            font: font_inputs.as_ref().map_or(std::ptr::null(), std::ptr::from_ref),
        };
        let parents = [std::ptr::null::<std::ffi::c_void>(); group_index::COUNT];
        let mut rebuilt = [std::ptr::null::<std::ffi::c_void>(); group_index::COUNT];
        unsafe {
            crate::css::table_group_builder::rust_build_group_payloads_from_table(
                table,
                groups,
                parents.as_ptr(),
                &raw const build_inputs,
                rebuilt.as_mut_ptr(),
                group_index::COUNT,
            );
        }
        for (group, payload) in rebuilt.into_iter().enumerate() {
            if groups & (1 << group) == 0 || payload.is_null() {
                continue;
            }
            result.payloads[group] = payload;
            result.rebuilt_groups |= 1 << group;
        }
        Some(result)
    }
}

impl super::RetainedState {
    pub(crate) fn committed_container_box_applies(&self, committed_record: u64, current_record: u64) -> bool {
        if committed_record == current_record {
            return true;
        }
        self.computed_group_sets
            .style_record_payloads(committed_record)
            .zip(self.computed_group_sets.style_record_payloads(current_record))
            .is_some_and(|(committed, current)| {
                let committed = crate::css::computed_value_views::ComputedValuesView::new(
                    crate::css::host_shared::SharedPayload::as_pointer_slice(committed),
                );
                let current = crate::css::computed_value_views::ComputedValuesView::new(
                    crate::css::host_shared::SharedPayload::as_pointer_slice(current),
                );
                committed.box_values().display == current.box_values().display
                    && committed.box_values().display_before_box_type_transformation
                        == current.box_values().display_before_box_type_transformation
                    && committed.box_values().position == current.box_values().position
                    && committed.box_values().float_ == current.box_values().float_
                    && committed.box_values().size_containment == current.box_values().size_containment
                    && committed.box_values().inline_size_containment == current.box_values().inline_size_containment
                    && committed.box_values().layout_containment == current.box_values().layout_containment
                    && committed.content_visibility() == current.content_visibility()
                    && committed.box_values().is_size_container == current.box_values().is_size_container
                    && committed.box_values().is_inline_size_container == current.box_values().is_inline_size_container
                    && committed.writing_mode() == current.writing_mode()
            })
    }

    pub(crate) fn container_unit_basis(
        &self,
        subject: StyleNodeID,
        axis_is_horizontal: bool,
        viewport: f64,
    ) -> ContainerUnitBasis {
        let mut ancestor = self.tree().flat_tree_parent(subject);
        while let Some(node) = ancestor {
            ancestor = self.tree().flat_tree_parent(node);
            let Some(inputs) = self.container_query_inputs(node) else {
                continue;
            };
            // The container's own writing mode decides which of its axes is the inline one, and an
            // `inline-size` container answers only for that axis.
            let container_inline_axis_is_horizontal =
                inputs.writing_mode == crate::css::css_enums::writing_mode::HORIZONTAL_TB;
            let eligible = if axis_is_horizontal == container_inline_axis_is_horizontal {
                inputs.is_size_container || inputs.is_inline_size_container
            } else {
                inputs.is_size_container
            };
            if !eligible {
                continue;
            }
            // This published input names the container's currently installed style. The layout
            // snapshot can still hold a box from before that style stopped generating one.
            if self
                .published_style_record_view(super::computed::FinalStyleRecordID::from_raw(inputs.style_record))
                .is_some_and(|view| view.display().is_none() || view.display().is_contents())
            {
                return ContainerUnitBasis {
                    basis: 0.0,
                    depends_on_viewport_metrics: false,
                    container: Some(node),
                    container_has_no_box: true,
                };
            }
            let snapshot = self.layout_style_snapshots.row(node).unwrap_or_default();
            // The host resolves container units against the committed box until layout replaces
            // it, including when the container's style has changed since that box was committed.
            if !snapshot.has_committed_box {
                return ContainerUnitBasis {
                    basis: 0.0,
                    depends_on_viewport_metrics: false,
                    container: Some(node),
                    container_has_no_box: true,
                };
            }
            let raw = if axis_is_horizontal {
                snapshot.content_width_raw
            } else {
                snapshot.content_height_raw
            };
            return ContainerUnitBasis {
                basis: crate::css::css_pixels::CssPixels::from_raw(raw).to_double(),
                depends_on_viewport_metrics: false,
                container: Some(node),
                container_has_no_box: false,
            };
        }
        ContainerUnitBasis {
            basis: viewport,
            depends_on_viewport_metrics: true,
            container: None,
            container_has_no_box: false,
        }
    }
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

    /// A CSS animation on the document timeline: 1000ms long, started at `start` or holding
    /// `hold`.
    fn document_timeline_row(start: Option<f64>, hold: Option<f64>, extra_flags: u32) -> AnimationTimingRow {
        use timing_row_flag as flag;
        let mut row = AnimationTimingRow {
            flags: flag::HAS_TIMELINE | flag::TIMELINE_IS_MONOTONICALLY_INCREASING | extra_flags,
            ..Default::default()
        };
        row.times[TIME_ITERATION_DURATION] = 1000.0;
        row.times[TIME_ITERATION_COUNT] = 1.0;
        row.times[TIME_PLAYBACK_RATE] = 1.0;
        if let Some(start) = start {
            row.flags |= flag::HAS_START_TIME;
            row.times[TIME_START] = start;
        }
        if let Some(hold) = hold {
            row.flags |= flag::HAS_HOLD_TIME;
            row.times[TIME_HOLD] = hold;
        }
        row
    }

    fn time(value: f64) -> Option<TimeValue> {
        Some(TimeValue {
            is_percentage: false,
            value,
        })
    }

    #[test]
    fn pausing_a_running_animation_keeps_its_time_until_the_pause_task() {
        use timing_row_flag as flag;
        let running = document_timeline_row(Some(100.0), None, 0);
        let paused = running.with_css_play_state(false, time(400.0)).unwrap();
        assert!(paused.has(flag::HAS_PENDING_PAUSE_TASK));
        assert!(paused.has(flag::HAS_START_TIME) && !paused.has(flag::HAS_HOLD_TIME));
        // Playing a running animation does nothing.
        assert_eq!(running.with_css_play_state(true, time(400.0)).unwrap(), running);
    }

    #[test]
    fn playing_a_paused_animation_keeps_its_hold_time_or_rewinds_a_finished_one() {
        use timing_row_flag as flag;
        let paused = document_timeline_row(None, Some(300.0), 0);
        let played = paused.with_css_play_state(true, time(400.0)).unwrap();
        assert!(played.has(flag::HAS_PENDING_PLAY_TASK) && !played.has(flag::HAS_PENDING_PAUSE_TASK));
        assert_eq!(played.times[TIME_HOLD], 300.0);
        // Pausing a paused animation does nothing.
        assert_eq!(paused.with_css_play_state(false, time(400.0)).unwrap(), paused);
        // A paused animation held at its end rewinds to zero.
        let at_end = document_timeline_row(None, Some(1000.0), 0);
        assert_eq!(
            at_end.with_css_play_state(true, time(400.0)).unwrap().times[TIME_HOLD],
            0.0
        );
    }

    #[test]
    fn playing_an_animation_with_a_pending_pause_aborts_the_pause() {
        use timing_row_flag as flag;
        let pausing = document_timeline_row(Some(100.0), None, flag::HAS_PENDING_PAUSE_TASK);
        let played = pausing.with_css_play_state(true, time(400.0)).unwrap();
        assert!(played.has(flag::HAS_PENDING_PLAY_TASK) && !played.has(flag::HAS_PENDING_PAUSE_TASK));
        assert!(played.has(flag::HAS_START_TIME) && !played.has(flag::HAS_HOLD_TIME));
    }

    #[test]
    fn a_play_state_script_overrode_is_left_alone() {
        use timing_row_flag as flag;
        let running = document_timeline_row(Some(100.0), None, flag::CSS_PLAY_STATE_OVERRIDDEN_BY_SCRIPT);
        assert_eq!(running.with_css_play_state(false, time(400.0)).unwrap(), running);
    }

    #[test]
    fn a_name_that_disappeared_claims_nothing() {
        assert_eq!(
            match_existing_animations(&[name("a"), name("b")], &[name("b")]),
            vec![1]
        );
    }

    /// Publish one scope's `@keyframes` as the host does, each name's set identified by `identity`.
    fn publish(keyframes: &mut AnimationKeyframes, tree_scope: TreeScopeID, sets: &[(&str, u64)]) {
        let name_lengths = sets
            .iter()
            .map(|(name, _)| name.encode_utf16().count() as u32)
            .collect::<Vec<_>>();
        let name_units = sets
            .iter()
            .flat_map(|(name, _)| name.encode_utf16())
            .collect::<Vec<_>>();
        let effects = sets
            .iter()
            .map(|&(_, identity)| super::super::bridge::FfiPublishedAnimationEffect {
                identity,
                generation: 0,
                flags: 0,
                first_keyframe: 0,
                keyframe_count: 0,
                base_url_offset: 0,
                base_url_length: 0,
            })
            .collect::<Vec<_>>();
        unsafe {
            keyframes.set(
                tree_scope,
                0,
                &name_lengths,
                &name_units,
                PublishedEffectBuffers {
                    effects: &effects,
                    keyframes: &[],
                    declarations: &[],
                    custom_declarations: &[],
                    linear_points: &[],
                    base_url_bytes: &[],
                },
            );
        }
    }

    #[test]
    fn an_owed_plan_names_the_keyframe_sets_a_republication_leaves() {
        let mut keyframes = AnimationKeyframes::default();
        publish(
            &mut keyframes,
            TreeScopeID::DOCUMENT,
            &[("fade", 0x1000), ("tint", 0x2000)],
        );
        let names = vec![name("fade"), name("tint")].into_boxed_slice();
        let definitions = names
            .iter()
            .map(|name| crate::css::style_compute::FfiComputedAnimation {
                duration_is_auto: false,
                duration: 1000.0,
                timing_function: std::ptr::null(),
                iteration_count: 1.0,
                direction: 0,
                play_state: 0,
                delay: 0.0,
                fill_mode: 0,
                composition: 0,
                name: name.as_ptr(),
                timeline_kind: crate::css::style_compute::FfiAnimationTimelineKind::Document,
                scroll_scroller: 0,
                scroll_axis: 0,
                matched_existing_index: NO_MATCHED_ANIMATION,
                keyframe_set: keyframes
                    .resolve_in_declaration_scope(None, TreeScopeID::DOCUMENT, name)
                    .map_or(std::ptr::null(), |set| set.pointer as *const std::ffi::c_void),
            })
            .collect::<Vec<_>>()
            .into_boxed_slice();
        let mut plan = SettledAnimationPlan::new(definitions, names, Box::new([]), false, None, TreeScopeID::DOCUMENT);
        let sets = |plan: &SettledAnimationPlan| {
            plan.definitions()
                .iter()
                .map(|definition| definition.keyframe_set as usize)
                .collect::<Vec<_>>()
        };
        assert_eq!(sets(&plan), vec![0x1000, 0x2000]);

        // A rebuilt rule cache publishes new sets for the same names; the host lets go of the old ones.
        publish(
            &mut keyframes,
            TreeScopeID::DOCUMENT,
            &[("fade", 0x3000), ("tint", 0x4000)],
        );
        plan.resolve_keyframes_again(&keyframes);
        assert_eq!(sets(&plan), vec![0x3000, 0x4000]);
        assert!(sets(&plan).iter().all(|&set| keyframes.description(set).is_some()));

        // A name no scope defines any more has no keyframes.
        publish(&mut keyframes, TreeScopeID::DOCUMENT, &[("tint", 0x5000)]);
        plan.resolve_keyframes_again(&keyframes);
        assert_eq!(sets(&plan), vec![0, 0x5000]);
    }
}
