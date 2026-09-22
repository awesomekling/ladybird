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

/// The word the timeline kind sits in, and its shift, so the stage can refuse to decide the one
/// kind whose materialization reads the tree.
const APPLIED_DEFINITION_FLAGS_WORD: usize = 3;
/// `animation-duration: auto`, whose value is the effect's intrinsic duration rather than the
/// definition's.
const APPLIED_DEFINITION_DURATION_IS_AUTO: u64 = 1;
const APPLIED_DEFINITION_TIMELINE_KIND_SHIFT: u32 = 40;
/// The fields of the flags word a change to which moves no time: `animation-direction`,
/// `animation-fill-mode` and `animation-composition`, each a byte. Everything else in that word -
/// `duration_is_auto`, the play state and the timeline - is a change the retime cannot describe.
const APPLIED_DEFINITION_RETIMABLE_FLAGS_MASK: u64 = (0xff << 8) | (0xff << 24) | (0xff << 32);
/// The `animation-play-state` byte of the flags word. `apply_css_properties` compares it against
/// the play state the last definition applied, and runs `play_from_css()` or `pause_from_css()`
/// when the two differ.
const APPLIED_DEFINITION_PLAY_STATE_MASK: u64 = 0xff << 16;
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

    /// Whether the timeline this definition asks for is one whose materialization the stage can
    /// predict. A scroll timeline is rebuilt from the element's surroundings every time it is
    /// applied, and whether the rebuilt one would replace the animation's is a question about the
    /// tree, so a definition that names one is never called unchanged.
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
        for index in 0..APPLIED_DEFINITION_WORD_COUNT {
            if index == APPLIED_DEFINITION_TIMING_FUNCTION_WORD {
                continue;
            }
            if self.words[index] != published.words[index] {
                return false;
            }
        }
        unsafe {
            crate::css::style_value::rust_style_value_equals(
                self.words[APPLIED_DEFINITION_TIMING_FUNCTION_WORD] as *const _,
                published.words[APPLIED_DEFINITION_TIMING_FUNCTION_WORD] as *const _,
            )
        }
    }

    /// Whether applying `self` to an animation that last had `published` applied would leave its
    /// timing exactly as it is and only give its effect another keyframe set.
    ///
    /// The host applies such a definition by handing the effect its new keyframes and then taking
    /// `apply_css_properties`' early return, since every property that function compares is
    /// unchanged. Handing over keyframes moves no time, changes no play state and creates nothing:
    /// the animation keeps its identity, its row and its place in the element's list, and the only
    /// thing that changes about it is the `@keyframes` rule its declarations come from.
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

    /// Whether applying `self` to an animation that last had `published` applied would change only
    /// what its effect is sampled from and how far a given time is along it, and move no time.
    ///
    /// `apply_css_properties` hands such a definition to the effect's plain setters -
    /// `set_specified_iteration_duration`, `set_specified_start_delay`, `set_iteration_count`,
    /// `set_fill_mode`, `set_playback_direction`, `set_composite` - and then normalizes the
    /// specified timing. None of them notifies the animation, so the start time, the hold time and
    /// the pending tasks stay exactly as they are: the retimed row is the published row with those
    /// three times restamped and those two flag fields replaced.
    ///
    /// Everything that *would* move time is refused: a play-state change runs `play_from_css()` or
    /// `pause_from_css()`, and an `auto` duration is the effect's intrinsic one rather than the
    /// definition's.
    #[must_use]
    pub(crate) fn change_is_only_simple_timing(&self, published: &Self) -> bool {
        if !self.timeline_is_decidable() {
            return false;
        }
        // An animation no plan has described yet publishes a null timing function, and nothing is
        // known about the timing it is being retimed from.
        if published.words[APPLIED_DEFINITION_TIMING_FUNCTION_WORD] == 0 {
            return false;
        }
        if self.words[APPLIED_DEFINITION_FLAGS_WORD] & APPLIED_DEFINITION_DURATION_IS_AUTO != 0 {
            return false;
        }
        self.words[APPLIED_DEFINITION_FLAGS_WORD] & !APPLIED_DEFINITION_RETIMABLE_FLAGS_MASK
            == published.words[APPLIED_DEFINITION_FLAGS_WORD] & !APPLIED_DEFINITION_RETIMABLE_FLAGS_MASK
    }

    /// Whether the only thing applying `self` to an animation that last had `published` applied
    /// would do is run `play_from_css()` or `pause_from_css()` on it.
    ///
    /// Every other field of the definition is unchanged, so `apply_css_properties` hands the effect
    /// the values it already has and the play-state branch at its end is the whole of the change.
    /// Whether that branch moves anything the stage samples is a question about the animation's
    /// published row, which `row_absorbs_a_play_state_change` answers.
    #[must_use]
    pub(crate) fn change_is_only_play_state(&self, published: &Self) -> bool {
        if self.words[APPLIED_DEFINITION_FLAGS_WORD] & APPLIED_DEFINITION_PLAY_STATE_MASK
            == published.words[APPLIED_DEFINITION_FLAGS_WORD] & APPLIED_DEFINITION_PLAY_STATE_MASK
        {
            return false;
        }
        let mut without_the_play_state = *self;
        without_the_play_state.words[APPLIED_DEFINITION_FLAGS_WORD] = (self.words[APPLIED_DEFINITION_FLAGS_WORD]
            & !APPLIED_DEFINITION_PLAY_STATE_MASK)
            | (published.words[APPLIED_DEFINITION_FLAGS_WORD] & APPLIED_DEFINITION_PLAY_STATE_MASK);
        without_the_play_state.would_change_nothing(published)
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

    /// Whether the element runs a CSS animation at all, in any of its lists. Only such an element
    /// is re-planned when a `@keyframes` rule moves, and the plan is no part of its record.
    #[must_use]
    pub(crate) fn node_runs_a_css_animation(&self, node: StyleNodeID) -> bool {
        self.rows.keys().any(|&(row_node, _)| row_node == node)
    }

    #[must_use]
    pub(crate) fn applied_definitions(&self, node: StyleNodeID, slot: AnimationSlot) -> &[AppliedAnimationDefinition] {
        self.rows.get(&(node, slot)).map_or(&[][..], |row| &row.1[..])
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
    /// A provisionally started transition's row. The pass that started the transition samples its
    /// effect, so the row is published for it to be sampled from, but the transition is not
    /// associated with its target yet and the row answers nothing about what the element holds.
    pub(crate) const NOT_ASSOCIATED: u32 = 1 << 27;
    /// The animation names an owning element, which is the first thing the class-specific composite
    /// order of a CSS animation or transition compares.
    pub(crate) const HAS_OWNING_ELEMENT: u32 = 1 << 28;
    /// The owning element currently lists this CSS animation at the place its class-specific key
    /// names. A CSS animation the element has stopped listing - one script revived after a plan
    /// cancelled it - keeps the place it was last given, so the key alone does not say which of the
    /// two animations claiming a place the element's plan works on.
    pub(crate) const LISTED_BY_OWNING_ELEMENT: u32 = 1 << 29;
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
    /// arithmetic, which is why the caller may sample the row without a published sample for it.
    ///
    /// `None` for a definition whose row this cannot settle: a scroll timeline, which is
    /// materialized from the element's surroundings, and an `auto` duration, which the host takes
    /// from the effect's intrinsic duration.
    #[must_use]
    pub(crate) fn for_new_css_animation(
        definition: &crate::css::style_compute::FfiComputedAnimation,
        owning_node: StyleNodeID,
        owning_slot: AnimationSlot,
        name_index: u32,
        synthesized_index: u32,
    ) -> Option<Self> {
        use crate::css::style_compute::FfiAnimationTimelineKind;
        use timing_row_flag as flag;

        // NB: `animation-duration: auto` - the initial value - has the intrinsic iteration duration
        //     of the effect, which against a monotonic timeline is zero; the drive already computed
        //     the definition's duration as zero for it.
        if definition.timeline_kind != FfiAnimationTimelineKind::Document {
            return None;
        }
        // `Bindings::PlaybackDirection` and `Bindings::FillMode` are in IDL order, which is not the
        // order the CSS keywords are in: a mirror of `css_animation_direction_to_playback_direction`
        // and `css_fill_mode_to_bindings_fill_mode`.
        let direction = match definition.direction {
            0 => 2, // alternate
            1 => 3, // alternate-reverse
            2 => 0, // normal
            3 => 1, // reverse
            _ => return None,
        };
        let fill_mode = match definition.fill_mode {
            0 => 2, // backwards
            1 => 3, // both
            2 => 1, // forwards
            3 => 0, // none
            _ => return None,
        };
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
                | flag::HAS_TIMELINE
                | flag::TIMELINE_IS_MONOTONICALLY_INCREASING
                | flag::HAS_OWNING_ELEMENT
                // The plan starts this animation into the place the definition holds, so the
                // element lists it there for as long as the row stands for it.
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
            composite_class: 0,
            composite_owning_slot: owning_slot,
            composite_transition_property: 0,
            composite_owning_node: owning_node.raw(),
            // The host's class-specific composite order key for a CSS animation is its place in the
            // `animation-name` list, which is the place the plan gives this definition.
            composite_class_key: name_index,
            // Only two CSS transitions with no owning element are ordered by the global list, and a
            // CSS animation this element owns is neither.
            global_list_order: 0,
            // A CSS animation's `animation-timing-function` is applied per keyframe, so the effect's
            // own easing is always the identity `linear`.
            first_linear_point: 0,
            linear_point_count: 0,
            times,
            synthesized_index: Some(synthesized_index),
        })
    }

    /// This row with the timing a definition that moves no time would stamp on it: the three
    /// specified times and the two flag fields `apply_css_properties` sets through the effect's
    /// plain setters, which notify the animation of nothing.
    ///
    /// `None` for a definition whose direction or fill mode is not one of the CSS keywords.
    #[must_use]
    pub(crate) fn retimed_for_definition(
        &self,
        definition: &crate::css::style_compute::FfiComputedAnimation,
    ) -> Option<Self> {
        use timing_row_flag as flag;

        // The same two IDL-order mappings `for_new_css_animation` makes.
        let direction = match definition.direction {
            0 => 2, // alternate
            1 => 3, // alternate-reverse
            2 => 0, // normal
            3 => 1, // reverse
            _ => return None,
        };
        let fill_mode = match definition.fill_mode {
            0 => 2, // backwards
            1 => 3, // both
            2 => 1, // forwards
            3 => 0, // none
            _ => return None,
        };
        // A time the host holds as a percentage of a progress-based timeline is not the specified
        // one the definition carries, so it is not restamped from it.
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

    #[must_use]
    pub(crate) fn effect_identity(&self) -> u64 {
        self.effect_identity
    }

    /// Which of the computation's starting animations this row stands for, for a row the stage
    /// synthesized rather than read from the published list.
    #[must_use]
    pub(crate) fn synthesized_index(&self) -> Option<u32> {
        self.synthesized_index
    }

    /// The place in the element's `animation-name` list of the CSS animation this row describes,
    /// for a row that is one of the animations `(node, slot)`'s own plan works on. `None` for every
    /// other row: a transition, an animation script started, a CSS animation another element owns,
    /// and a CSS animation whose owning element has stopped listing it.
    ///
    /// The host's class-specific composite order key for a CSS animation *is* that place, so the
    /// row already carries it - but only an animation the element still lists there really holds
    /// it, and only one the element lists is an animation its plan works on.
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

/// Whether running `play_from_css()` or `pause_from_css()` on the animation this row describes
/// would leave everything the stage samples exactly as published.
///
/// A mirror of `Animation::play_an_animation` with the auto-rewind flag and of `Animation::pause`,
/// restricted to the envelope in which neither of them moves a time:
///
/// - a monotonically increasing timeline, so neither procedure has a finite timeline, neither ever
///   auto-aligns a start time, and `row_is_relevant` never consults the play state at all;
/// - a resolved current time that is at least zero and below the associated effect end, with a
///   playback rate above zero and no pending playback rate - so the rate is already the effective
///   one, `play_an_animation` takes none of its three rewind branches at step 6, and `pause` needs
///   no seek at step 5;
/// - not already marked finished, so `update_finished_state` cannot clear that flag underneath
///   `is_in_play`.
///
/// What is then left of either procedure is bookkeeping in the two pending-task bits: each cancels
/// whichever task was scheduled and schedules its own. A row that already carries one of them is
/// fine - `play_an_animation` runs only for an animation that is not already running, and `pause`
/// only for one that is not already paused, so the two never fight. Those bits then turn
/// `update_finished_state`'s step 2 off, so no hold time is touched either. A play whose animation
/// holds no hold time and aborts no pause aborts at step 10 and does nothing at all; one that does
/// hold a hold time keeps it and only loses its start time, and a current time read from a hold
/// time does not consult the start time. `row_current_key` never consults the play state or the
/// pending tasks, and `row_is_relevant` consults them only through `play_state`, which it asks for
/// only about a timeline that is not monotonically increasing.
#[must_use]
pub(crate) fn row_absorbs_a_play_state_change(row: &AnimationTimingRow, timeline_time: Option<TimeValue>) -> bool {
    use timing_row_flag as flag;

    if row.has(flag::UNDECIDABLE)
        || !row.has(flag::HAS_TIMELINE)
        || !row.has(flag::TIMELINE_IS_MONOTONICALLY_INCREASING)
        || row.has(flag::HAS_PENDING_PLAYBACK_RATE)
        || row.has(flag::IS_FINISHED)
    {
        return false;
    }
    // A rate that is not a number is one no comparison the host makes is true of.
    if row.times[TIME_PLAYBACK_RATE].partial_cmp(&0.0) != Some(std::cmp::Ordering::Greater) {
        return false;
    }
    let Some(timing) = resolve_timing(row, timeline_time) else {
        return false;
    };
    let Some(current_time) = timing.current_time else {
        return false;
    };
    // The host compares the lower bound against the raw value and the upper one as a time.
    current_time.value >= 0.0
        && current_time
            .compare(timing.end_time)
            .is_some_and(std::cmp::Ordering::is_lt)
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
pub(crate) fn row_current_key(
    row: &AnimationTimingRow,
    linear_points: &[crate::css::animation::FfiLinearEasingPoint],
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
    let easing_kind = (row.flags >> flag::EASING_KIND_SHIFT) & flag::EASING_KIND_MASK;
    let output_progress = match easing_kind {
        // `linear()`, whose stops the row names by range in the list's shared buffer. An empty
        // range is `linear` itself, which the host holds as `linear(0, 1)`.
        0 => crate::css::animation::evaluate_linear_easing(
            match row.linear_point_count {
                0 => &[
                    crate::css::animation::FfiLinearEasingPoint {
                        input: 0.0,
                        output: 0.0,
                    },
                    crate::css::animation::FfiLinearEasingPoint {
                        input: 1.0,
                        output: 1.0,
                    },
                ],
                count => linear_points
                    .get(row.first_linear_point as usize..)?
                    .get(..count as usize)?,
            },
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
/// One element's published list: the rows, and the `linear()` stops the rows name by range.
#[derive(PartialEq)]
struct PublishedTimingRows {
    rows: Box<[AnimationTimingRow]>,
    linear_points: Box<[crate::css::animation::FfiLinearEasingPoint]>,
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
                .map(|&[input, output]| crate::css::animation::FfiLinearEasingPoint { input, output })
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
    ) -> &[crate::css::animation::FfiLinearEasingPoint] {
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
/// effect stack yet, and composes below every effect that is.
///
/// Where the spec asks for the tree order of two differing owning elements, the host has a `FIXME`
/// that returns 0 and leaves the global animation list to decide. That is mirrored as it stands -
/// the point here is to preserve the host's order exactly, not to fix it.
#[must_use]
pub(crate) fn composite_order(a: &AnimationTimingRow, b: &AnimationTimingRow) -> std::cmp::Ordering {
    use crate::css::property_metadata::property_name;
    use std::cmp::Ordering;
    use timing_row_flag as flag;

    match a.has(flag::NOT_ASSOCIATED).cmp(&b.has(flag::NOT_ASSOCIATED)) {
        // `false` orders before `true`, so an associated effect would sort first. It is the
        // provisional transition that composes below, so the two are compared the other way round.
        Ordering::Equal => {}
        order => return order.reverse(),
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

/// The rows an element would publish once a plan that does nothing to its CSS animations but
/// cancel and renumber them has been applied: a cancelled animation drops out of the effect stack,
/// one the plan moved takes its new place in it, and the composite order is redone over what is
/// left.
///
/// `new_indices[j]` is the place `animation-name` order gives the element's `j`th CSS animation,
/// or `NO_MATCHED_ANIMATION` for one no definition claimed and that the plan therefore cancels.
/// Everything else the element holds - its transitions, the animations script started - the plan
/// does not touch, so those rows travel unchanged.
///
/// `None` where the published rows are not the list the plan is about: an animation the plan works
/// on that published no row at all, or two rows claiming one place in the list.
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

/// The rows an element would publish once a plan that also starts animations has been applied: the
/// rows the plan leaves behind, with the ones the stage synthesized for the animations it starts
/// merged into the composite order.
///
/// The published list is already in composite order and no two of an element's own CSS animations
/// can claim one place in its `animation-name` list, so a stable sort settles the merge.
#[must_use]
pub(crate) fn rows_with_synthesized(
    published: &[AnimationTimingRow],
    synthesized: &[AnimationTimingRow],
) -> Vec<AnimationTimingRow> {
    let mut rows = Vec::with_capacity(published.len() + synthesized.len());
    rows.extend_from_slice(published);
    rows.extend_from_slice(synthesized);
    rows.sort_by(composite_order);
    rows
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

/// Whether any of an element's animations for one pseudo-element is relevant, which is what
/// `Element::get_animations_internal()` filters its list by. `None` where any single row declines
/// to answer, since an unanswered row could be the relevant one.
#[must_use]
pub(crate) fn any_row_is_relevant(rows: &[AnimationTimingRow], samples: &AnimationTimelineSamples) -> Option<bool> {
    let mut any = false;
    for row in rows {
        if row.has(timing_row_flag::NOT_ASSOCIATED) {
            continue;
        }
        any |= row_is_relevant(row, row_timeline_time(row, samples)?)?;
    }
    Some(any)
}

/// Whether the row describes an effect that belongs to no animation the element holds - §19.2's
/// provisional transition duplicate. The host's own effect list has no such effect, so every walk
/// of the published rows has to skip them.
#[must_use]
pub(crate) fn row_is_not_associated(row: &AnimationTimingRow) -> bool {
    row.has(timing_row_flag::NOT_ASSOCIATED)
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
    /// The easing a computed `animation-timing-function` describes, which fills in the hole a
    /// keyframe with no easing of its own keeps. A mirror of `EasingFunction::from_style_value`.
    ///
    /// `None` for a `linear()` with stops of its own: the host canonicalizes those before reading
    /// them, which resolves each stop's calculated values and interpolates the inputs it was not
    /// given, and a definition that names one is left to the host.
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
                        crate::css::animation::FfiLinearEasingPoint {
                            input: 0.0,
                            output: 0.0,
                        },
                        crate::css::animation::FfiLinearEasingPoint {
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
                // The host reads each of these with its own `numeric()`, which resolves a
                // calculation on the spot; a definition that needs one is left to it.
                let numeric = |value: &crate::css::style_value::RetainedStyleValueData| match value.data() {
                    StyleValueData::Number { value } => Some(*value),
                    StyleValueData::Integer { value } => Some(*value as f64),
                    StyleValueData::Percentage { value } => Some(*value),
                    _ => None,
                };
                match kind {
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
    #[must_use]
    pub(crate) fn is_covered(&self) -> bool {
        self.flags & effect_flag::NOT_COVERED == 0
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
///
/// The description is the same shape as an element effect's, with the two holes a rule keeps until
/// an animation runs it - a keyframe's own easing, and `composite: auto` - left open for the
/// definition to fill in.
pub(crate) struct PublishedKeyframesSet {
    pub(crate) pointer: usize,
    pub(crate) description: PublishedEffect,
    /// Whether starting this animation is more than creating it: a value that counts the tree -
    /// `sibling-index()` and its kin - makes the element recompute when its siblings move, a
    /// `url()` resolves against the sheet the rule was written in, and a custom property or an
    /// unresolved value travels as the tokens it was written as, so nothing here can see what it
    /// asks for. All of them are noted by the computation that resolves the keyframes, and a row
    /// the engine settles never runs one: it leaves a plan naming such a rule to C++ whole.
    pub(crate) needs_the_host: bool,
    /// Whether the rule animates a value the element's descendants inherit. A C++ computation
    /// samples an animation it starts into the very record it publishes, so a descendant computed
    /// after it in the same batch inherits the animated value; a first record the engine settles
    /// publishes the style beneath the animation and applies its plan once the whole batch is
    /// installed, which is after those descendants were computed. Such a rule therefore keeps a
    /// first record in C++. A later record's descendants already hold records of their own and
    /// take the animated values through the overlay's invalidation, so it does not bind them.
    pub(crate) declares_an_inherited_property: bool,
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
}

/// Whether a rule animates a value an element's descendants inherit; see
/// `declares_an_inherited_property`. A shorthand is read as one: `all` covers every inherited
/// longhand, and the others are not worth expanding here.
#[must_use]
fn description_declares_an_inherited_property(description: &PublishedEffect) -> bool {
    use crate::css::property_metadata::{property_is_inherited, property_is_shorthand};
    // A custom property inherits, and every one of them is already refused by `needs_the_host`.
    !description.custom_declarations.is_empty()
        || description.declarations.iter().any(|declaration| {
            property_is_inherited(declaration.property_id) || property_is_shorthand(declaration.property_id)
        })
}

/// Whether starting an animation from this rule is more than creating it; see `needs_the_host`.
#[must_use]
fn description_needs_the_host(description: &PublishedEffect) -> bool {
    if !description.is_covered() {
        return true;
    }
    // Having a resource context is ordinary - every sheet with a base URL records one. Needing it
    // is not: a `url()` in a keyframe resolves against the sheet the rule was written in, which
    // only the computation that resolves the keyframes does.
    if description.flags & effect_flag::HAS_RESOURCE_CONTEXT != 0
        && description.declarations.iter().any(|declaration| {
            declaration
                .value
                .optional_data()
                .is_some_and(crate::css::style_compute::value_may_need_style_sheet_resource_context)
        })
    {
        return true;
    }
    // A custom property's keyframe value travels as the token stream it was written as, so nothing
    // here can see what it asks for: a rule declaring one is refused outright.
    if !description.custom_declarations.is_empty() {
        return true;
    }
    description.declarations.iter().any(|declaration| {
        declaration.value.optional_data().is_some_and(|data| {
            crate::css::style_compute::collect_external_value_dependencies(data).uses_tree_counting_function
        })
    })
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
        let mut offset = 0usize;
        for (&length, description) in name_lengths.iter().zip(descriptions) {
            let end = offset + length as usize;
            assert!(end <= name_units.len(), "@keyframes name lengths overrun their buffer");
            let name = KeyframesName(CssString::from_utf16(&name_units[offset..end]));
            offset = end;
            let needs_the_host = description_needs_the_host(&description);
            let declares_an_inherited_property = description_declares_an_inherited_property(&description);
            sets.insert(
                name,
                PublishedKeyframesSet {
                    // The host names a set by its own pointer, which is what it publishes as the
                    // description's identity.
                    pointer: description.identity as usize,
                    description,
                    needs_the_host,
                    declares_an_inherited_property,
                },
            );
        }
        self.scopes.insert(tree_scope, sets);
    }

    #[must_use]
    fn in_scope(&self, tree_scope: TreeScopeID, name: &KeyframesName) -> Option<&PublishedKeyframesSet> {
        self.scopes.get(&tree_scope)?.get(name)
    }

    /// Whether a first record may start the animations this document defines at all.
    ///
    /// A record the engine settles publishes the style beneath its animations and applies its plan
    /// once the whole batch is installed. For a *first* record that is too late for two things the
    /// C++ computation does inside itself: sampling the animation into the record its descendants
    /// inherit from in this same batch, and noting what resolving the keyframes read. So a first
    /// record starts an animation only where every rule in the document is one it can account for -
    /// asked of the document rather than of the plan, because the question has to be answered
    /// before the record is driven, and a plan refused after that would leave a record assigned
    /// that nothing installs.
    #[must_use]
    pub(crate) fn a_first_record_may_start_an_animation(&self) -> bool {
        self.only_the_document_scope_defines_keyframes()
            && self
                .scopes
                .values()
                .flat_map(HashMap::values)
                .all(|set| !set.needs_the_host && !set.declares_an_inherited_property)
    }

    /// Whether every `@keyframes` the document defines is defined in the document's own scope.
    ///
    /// The chain `resolve` walks ends at the document scope, so where no other scope defines
    /// anything the answer is the document's rule for the name whatever the first two links are:
    /// neither the scope the winning `animation-name` declaration was written in nor the scope the
    /// element is in can change it. That is what lets a record the engine settled carry an
    /// animation plan at all, since the winner store the engine cascades from does not record
    /// which shadow root a declaration was written in.
    #[must_use]
    pub(crate) fn only_the_document_scope_defines_keyframes(&self) -> bool {
        self.scopes.keys().all(|&scope| scope == TreeScopeID::DOCUMENT)
    }

    /// The keyframe set an animation of this name runs, or `None` where no scope in its chain
    /// defines one and the host makes an effect with no keyframes.
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
    ) -> Option<&PublishedKeyframesSet> {
        if self.scopes.is_empty() {
            return None;
        }
        let name = KeyframesName(name.clone());
        if declaration_shadow_root_identity != 0
            && let Some(scope) = self
                .scope_by_shadow_root
                .get(&declaration_shadow_root_identity)
                .copied()
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
/// on the record that table went into, surviving the batch.
pub(crate) struct SettledAnimationPlan {
    definitions: Box<[crate::css::style_compute::FfiComputedAnimation]>,
    #[expect(
        dead_code,
        reason = "the names the definitions point at, owned for as long as they are"
    )]
    names: Box<[CssString]>,
    #[expect(
        dead_code,
        reason = "the timing functions the definitions point at, retained for as long as they are"
    )]
    timing_functions: Box<[crate::css::style_value::RetainedStyleValueData]>,
    element_display_is_none: bool,
}

// SAFETY: Every pointer a definition holds addresses something the plan owns and only ever shares -
//         one of its own names, one of its own retained timing functions - except the keyframe-set
//         identity, which is a host pointer this side never dereferences and only hands back, as
//         `PublishedKeyframesSet` holds one.
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
    ) -> Self {
        Self {
            definitions,
            names,
            timing_functions,
            element_display_is_none,
        }
    }

    #[must_use]
    pub(crate) fn definitions(&self) -> &[crate::css::style_compute::FfiComputedAnimation] {
        &self.definitions
    }

    /// Whether the record the row installs computes `display: none` for the element itself, which
    /// is the half of "is this element rendered" the plan can answer. The host walks the element's
    /// ancestors for the other half, as it does for a plan a C++ computation decided.
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

impl super::StyleEngine {
    /// The container-unit basis for one physical axis of `subject`.
    ///
    /// A mirror of `nearest_query_container_for_axis` plus the basis read that follows it, taken
    /// from the published container-query inputs and the retained layout snapshot instead of from
    /// the DOM and the layout tree. `viewport` is the subject's own viewport length for the axis,
    /// which is what the host falls back to.
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
            let snapshot = self.layout_style_snapshot(node).unwrap_or_default();
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

    #[test]
    fn a_name_that_disappeared_claims_nothing() {
        assert_eq!(
            match_existing_animations(&[name("a"), name("b")], &[name("b")]),
            vec![1]
        );
    }
}
