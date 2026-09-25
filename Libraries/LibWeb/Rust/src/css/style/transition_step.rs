/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The transition step of a row the pass settled, decided by the engine over what the host's
//! `StyleComputer::start_needed_transitions` decides it over: the before-change record, the
//! after-change style the row installs, and the transitions the element holds.

use std::collections::HashMap;

use super::animations::{AnimationSlot, row_easing_output, row_plays_unfinished, row_timeline_time};
use super::bridge::FfiPublishedTransition;
use super::tree::StyleNodeID;
use super::{RetainedState, StyleEngineState, engine_sample_check};
use crate::css::animated_overlay::{AnimatedOverlay, overlay_wins};
use crate::css::computed_longhand_table::ComputedLonghandTable;
use crate::css::style_value::{RetainedStyleValueData, StyleValueData};
use crate::css::transition::{FfiTransitionAction, FfiTransitionActionKind, FfiTransitionPropertyInput};

pub(crate) struct PublishedTransition {
    property_id: u16,
    effect_identity: u64,
    effect_replaced: bool,
    end_value: RetainedStyleValueData,
    reversing_adjusted_start_value: RetainedStyleValueData,
    reversing_shortening_factor: f64,
    start_time: f64,
    end_time: f64,
}

/// Per element and pseudo-element, the transitions it holds.
#[derive(Default)]
pub(crate) struct ElementTransitions {
    rows: HashMap<(StyleNodeID, AnimationSlot), Box<[PublishedTransition]>>,
}

impl ElementTransitions {
    /// Replace one list. An empty list drops the row.
    ///
    /// # Safety
    /// Every value must be a live style value the host holds a reference to for the duration of the
    /// call.
    pub(crate) unsafe fn set(
        &mut self,
        node: StyleNodeID,
        slot: AnimationSlot,
        transitions: &[FfiPublishedTransition],
    ) {
        if transitions.is_empty() {
            self.rows.remove(&(node, slot));
            return;
        }
        let retain = |value: *const std::ffi::c_void| unsafe {
            RetainedStyleValueData::from_retained_pointer(crate::css::style_value::rust_style_value_retain(
                value.cast(),
            ))
        };
        let transitions = transitions
            .iter()
            .map(|transition| PublishedTransition {
                property_id: transition.property_id,
                effect_identity: transition.effect_identity,
                effect_replaced: transition.effect_replaced,
                end_value: retain(transition.end_value),
                reversing_adjusted_start_value: retain(transition.reversing_adjusted_start_value),
                reversing_shortening_factor: transition.reversing_shortening_factor,
                start_time: transition.start_time,
                end_time: transition.end_time,
            })
            .collect();
        self.rows.insert((node, slot), transitions);
    }

    #[must_use]
    fn get(&self, node: StyleNodeID, slot: AnimationSlot) -> &[PublishedTransition] {
        self.rows
            .get(&(node, slot))
            .map_or(&[][..], |transitions| &transitions[..])
    }

    /// Give up the lists of identities that have been retired, which can be minted again.
    pub(crate) fn retire(&mut self, nodes: &[StyleNodeID]) {
        if self.rows.is_empty() {
            return;
        }
        self.rows.retain(|&(node, _), _| !nodes.contains(&node));
    }
}

/// The value of a property in a style: the overlay's where it wins over the table's.
fn effective_value(
    table: &ComputedLonghandTable,
    overlay: Option<&AnimatedOverlay>,
    property_id: u16,
) -> *const StyleValueData {
    if let Some(entry) = overlay.and_then(|overlay| overlay.get(property_id))
        && overlay_wins(entry, table.is_important(property_id))
    {
        return entry.value_pointer();
    }
    table.get(property_id).map_or(std::ptr::null(), |value| value.pointer())
}

/// What the step decided for one property, compared with the host's decision.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TransitionStepDecision {
    kind: FfiTransitionActionKind,
    delay: f64,
    active_duration: f64,
    reversing_shortening_factor: f64,
}

impl From<&FfiTransitionAction> for TransitionStepDecision {
    fn from(action: &FfiTransitionAction) -> Self {
        Self {
            kind: action.kind,
            delay: action.delay,
            active_duration: action.active_duration,
            reversing_shortening_factor: action.reversing_shortening_factor,
        }
    }
}

impl RetainedState {
    /// Replace the transitions one of an element's lists holds.
    ///
    /// # Safety
    /// Every value must be a live style value for the duration of the call.
    pub(crate) unsafe fn set_element_transitions(
        &mut self,
        node: StyleNodeID,
        slot: AnimationSlot,
        transitions: &[FfiPublishedTransition],
    ) {
        unsafe { self.element_transitions.set(node, slot, transitions) };
    }

    /// The transition step of an element whose row installs `installed_style_record` over
    /// `old_style_record`, decided as the host decides it: the property and action of each
    /// transition it starts, cancels or removes. `None` where the host's step decides nothing.
    fn decide_transition_step(
        &self,
        node: StyleNodeID,
        old_style_record: u64,
        installed_style_record: u64,
        layout_arena: super::animations::LentLayoutArena,
    ) -> Result<Vec<(u16, TransitionStepDecision)>, &'static str> {
        const SLOT: AnimationSlot = 0;
        const IN_DISPLAY_NONE_SUBTREE: u8 = 1 << 2;

        // https://drafts.csswg.org/css-transitions-2/#defining-before-change-style
        let before_style_record = match self.transition_baseline(node, u8::MAX) {
            0 => old_style_record,
            baseline => baseline,
        };
        if before_style_record == 0 {
            return Ok(Vec::new());
        }
        let Some(before) = self.style_record_view(before_style_record) else {
            return Ok(Vec::new());
        };
        if before.dependency_flags & IN_DISPLAY_NONE_SUBTREE != 0 {
            return Ok(Vec::new());
        }
        if let Some(parent) = self.tree.inheritance_parent(node)
            && let Some(parent_record) = self.computed_group_sets.assigned_style_record(parent)
            && let Some(parent_view) = self.style_record_view(parent_record.raw())
            && parent_view.dependency_flags & IN_DISPLAY_NONE_SUBTREE != 0
        {
            return Ok(Vec::new());
        }

        let installed = self
            .style_record_view(installed_style_record)
            .ok_or("no installed record")?;
        let after_table = unsafe { installed.longhand_table.as_ref() }.ok_or("an installed record with no table")?;
        let after_overlay = unsafe { installed.animated_overlay.as_ref() };
        let transitions = self.element_transitions.get(node, SLOT);
        let entries = match crate::css::style_compute::transition_delay_and_duration_are_single_zero(after_table)
            && transitions.is_empty()
        {
            true => Vec::new(),
            false => crate::css::style_compute::transition_entries(after_table).0,
        };
        if entries.is_empty() && transitions.is_empty() {
            return Ok(Vec::new());
        }
        // Script replaced the effect of one of the element's transitions, whose timing no longer
        // says whether the transition still runs: the host decides the step.
        if transitions.iter().any(|transition| transition.effect_replaced) {
            return Err("a transition whose effect script replaced");
        }

        let mut context = crate::css::animation::FfiAnimationContext {
            allow_discrete: false,
            current_color: effective_value(
                after_table,
                after_overlay,
                crate::css::property_metadata::property_id::COLOR,
            ),
            has_length_resolution_context: false,
            // SAFETY: A plain-data context the flag says is absent.
            length_resolution_context: unsafe { std::mem::zeroed() },
            has_transform_reference_box: false,
            transform_reference_box_width: 0.0,
            transform_reference_box_height: 0.0,
        };
        if let Some(length_resolution_context) = self.transition_length_resolution_context(installed_style_record) {
            context.has_length_resolution_context = true;
            context.length_resolution_context = length_resolution_context;
        }
        if let Some((width, height)) =
            unsafe { super::animations::committed_transform_reference_box(layout_arena.as_ptr(), node) }
        {
            context.has_transform_reference_box = true;
            context.transform_reference_box_width = width;
            context.transform_reference_box_height = height;
        }

        let rows = self.element_animation_timing_rows(node, SLOT);
        let linear_points = self.element_animation_timing_row_linear_points(node, SLOT);
        let input = |property_id: u16, entry: Option<&crate::css::transition::FfiTransitionEntry>| {
            let existing = transitions
                .iter()
                .find(|transition| transition.property_id == property_id);
            let running = existing.and_then(|transition| {
                let row = rows
                    .iter()
                    .find(|row| row.effect_identity() == transition.effect_identity)?;
                let time = row_timeline_time(row, &self.animation_timeline_samples)?;
                row_plays_unfinished(row, time)?.then_some((transition, row, time))
            });
            // `CSSTransition::timing_function_output_at_time()` at the style change event, which is
            // the time of the document timeline the transition runs on.
            let old_timing_function_output = running.and_then(|(transition, row, time)| {
                let time = time?.value;
                let progress = match transition.start_time < transition.end_time {
                    true => (time - transition.start_time) / (transition.end_time - transition.start_time),
                    false => 1.0,
                };
                row_easing_output(row, linear_points, progress, time < transition.start_time)
            });
            let mut property = FfiTransitionPropertyInput {
                property_id,
                before_change_value: std::ptr::null(),
                after_change_value: std::ptr::null(),
                current_value: std::ptr::null(),
                existing_end_value: existing.map_or(std::ptr::null(), |transition| transition.end_value.pointer()),
                reversing_adjusted_start_value: existing.map_or(std::ptr::null(), |transition| {
                    transition.reversing_adjusted_start_value.pointer()
                }),
                has_matching_transition: entry.is_some(),
                allow_discrete: false,
                has_running_transition: running.is_some(),
                has_completed_transition: existing.is_some() && running.is_none(),
                delay: 0.0,
                duration: 0.0,
                old_timing_function_output: 0.0,
                old_reversing_shortening_factor: 1.0,
            };
            if let Some(entry) = entry {
                property.delay = entry.delay;
                property.duration = entry.duration;
                property.allow_discrete = entry.behavior == 1;
                if let Some(existing) = existing {
                    property.old_reversing_shortening_factor = existing.reversing_shortening_factor;
                    property.old_timing_function_output = old_timing_function_output.unwrap_or(0.0);
                }
            }
            property
        };
        let mut properties = Vec::with_capacity(entries.len() + transitions.len());
        for entry in &entries {
            properties.push(input(entry.property_id, Some(entry)));
        }
        for transition in transitions {
            if !entries.iter().any(|entry| entry.property_id == transition.property_id) {
                properties.push(input(transition.property_id, None));
            }
        }
        let mut actions = properties
            .iter()
            .map(|property| FfiTransitionAction {
                property_id: property.property_id,
                kind: FfiTransitionActionKind::None,
                delay: 0.0,
                active_duration: 0.0,
                reversing_shortening_factor: 1.0,
            })
            .collect::<Vec<_>>();
        crate::css::transition::decide_transitions(
            self,
            before_style_record,
            after_table,
            after_overlay,
            &context,
            (u64::from(node.raw()) << 8) | u64::from(u8::MAX),
            &mut properties,
            &mut actions,
        );
        Ok(actions
            .iter()
            .map(|action| (action.property_id, TransitionStepDecision::from(action)))
            .collect())
    }
}

impl StyleEngineState {
    /// Decide the transition step of a row that owes the whole step, for the report to compare
    /// with the host's decision when the host runs the step.
    pub(crate) fn decide_settled_row_transition_step(
        &mut self,
        node: StyleNodeID,
        old_style_record: u64,
        installed_style_record: u64,
        layout_arena: super::animations::LentLayoutArena,
    ) {
        let decision = self.decide_transition_step(node, old_style_record, installed_style_record, layout_arena);
        if let Err(reason) = decision {
            engine_sample_check::note_declined(&format!("transition step: {reason}"));
        }
        self.retained.transition_step_decisions.insert(node, decision.ok());
    }

    /// Compare the step the host decided for an element with the one the pass decided.
    pub(crate) fn check_transition_step(&mut self, node: StyleNodeID, host_actions: &[FfiTransitionAction]) {
        if !engine_sample_check::is_reporting() {
            return;
        }
        let Some(decided) = self.retained.transition_step_decisions.remove(&node) else {
            engine_sample_check::note_declined("transition step: the pass decided no step");
            return;
        };
        let Some(decided) = decided else {
            return;
        };
        let differs = host_actions.len() != decided.len()
            || host_actions.iter().any(|action| {
                !decided.iter().any(|(property_id, decision)| {
                    *property_id == action.property_id && *decision == TransitionStepDecision::from(action)
                })
            });
        match differs {
            false => engine_sample_check::note_taken("transition step agrees"),
            true => engine_sample_check::note_declined(&format!(
                "transition step differs: host {:?} pass {decided:?}",
                host_actions
                    .iter()
                    .map(|action| (action.property_id, TransitionStepDecision::from(action)))
                    .collect::<Vec<_>>()
            )),
        }
    }
}
