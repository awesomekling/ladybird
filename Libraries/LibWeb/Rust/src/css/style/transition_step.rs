/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

//! The transition step of a row the pass settled, decided by the engine over what the host's
//! `StyleComputer::start_needed_transitions` decides it over: the before-change record, the
//! after-change style the row installs, and the transitions the element holds.

use std::collections::HashMap;

use super::animations::{
    AnimationSlot, AnimationTimingRow, PublishedEasing, PublishedEffect, row_easing_output, row_plays_unfinished,
    row_timeline_time,
};
use super::bridge::FfiPublishedTransition;
use super::bridge::FfiTransitionStepAction;
use super::tree::StyleNodeID;
use super::{RetainedState, StyleEngineState, engine_sample_check};
use crate::css::animated_overlay::{AnimatedOverlay, overlay_wins};
use crate::css::computed_longhand_table::ComputedLonghandTable;
use crate::css::computed_value_views::ComputedValuesView;
use crate::css::host_shared::SharedPayload;
use crate::css::property_metadata::property_id;
use crate::css::style_compute::StartedTransition;
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

/// What the pass decided for a row's transition step: the transitions it starts, and what the host
/// applies.
#[derive(Default)]
struct TransitionStep {
    started: Vec<StartedTransition>,
    /// The effects of the transitions it removes.
    removed: Vec<u64>,
    for_host: TransitionStepForHost,
}

/// A transition step the pass decided, which the host applies where it installs the row: the
/// decision for each property, and what a transition it starts runs from and to, which the step
/// keeps alive.
#[derive(Default)]
pub(crate) struct TransitionStepForHost {
    actions: Vec<FfiTransitionStepAction>,
    _values: Vec<RetainedStyleValueData>,
}

// SAFETY: The actions' values point into `_values`, which own a reference to each.
unsafe impl Send for TransitionStepForHost {}
unsafe impl Sync for TransitionStepForHost {}

impl TransitionStepForHost {
    #[must_use]
    pub(crate) fn actions(&self) -> &[FfiTransitionStepAction] {
        &self.actions
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
    ) -> Result<TransitionStep, &'static str> {
        const SLOT: AnimationSlot = 0;
        const IN_DISPLAY_NONE_SUBTREE: u8 = 1 << 2;

        // https://drafts.csswg.org/css-transitions-2/#defining-before-change-style
        let before_style_record = match self.transition_baseline(node, u8::MAX) {
            0 => old_style_record,
            baseline => baseline,
        };
        if before_style_record == 0 {
            return Ok(TransitionStep::default());
        }
        let Some(before) = self.style_record_view(before_style_record) else {
            return Ok(TransitionStep::default());
        };
        if before.dependency_flags & IN_DISPLAY_NONE_SUBTREE != 0 {
            return Ok(TransitionStep::default());
        }
        if let Some(parent) = self.tree.inheritance_parent(node)
            && let Some(parent_record) = self.computed_group_sets.assigned_style_record(parent)
            && let Some(parent_view) = self.style_record_view(parent_record.raw())
            && parent_view.dependency_flags & IN_DISPLAY_NONE_SUBTREE != 0
        {
            return Ok(TransitionStep::default());
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
            return Ok(TransitionStep::default());
        }
        // Script replaced the effect of one of the element's transitions, whose timing no longer
        // says whether the transition still runs: the host decides the step.
        if transitions.iter().any(|transition| transition.effect_replaced) {
            return Err("a transition whose effect script replaced");
        }

        let mut context = crate::css::animation::FfiAnimationContext {
            allow_discrete: false,
            current_color: effective_value(after_table, after_overlay, property_id::COLOR),
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
        crate::css::ffi_stats::rust_style_ffi_note_transition_decision();
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
        // What the step starts, as `CSSTransition` builds it, for the composition the step leaves.
        let mut removed = Vec::new();
        let mut started = Vec::new();
        let mut for_host = TransitionStepForHost::default();
        for (property, action) in properties.iter().zip(&actions) {
            for_host.actions.push(FfiTransitionStepAction {
                property_id: action.property_id,
                kind: action.kind as u8,
                delay: action.delay,
                active_duration: action.active_duration,
                reversing_shortening_factor: action.reversing_shortening_factor,
                start_value: std::ptr::null(),
                end_value: std::ptr::null(),
            });
            let (start_value, end_value) = match action.kind {
                FfiTransitionActionKind::None => continue,
                FfiTransitionActionKind::Remove | FfiTransitionActionKind::Cancel => {
                    removed.push(property.property_id);
                    continue;
                }
                FfiTransitionActionKind::Start => (property.before_change_value, property.after_change_value),
                FfiTransitionActionKind::RemoveAndStart => {
                    removed.push(property.property_id);
                    (property.before_change_value, property.after_change_value)
                }
                FfiTransitionActionKind::CancelRemoveAndStartReversing
                | FfiTransitionActionKind::CancelRemoveAndStartInterrupted => {
                    removed.push(property.property_id);
                    (property.current_value, property.after_change_value)
                }
            };
            let entry = entries
                .iter()
                .find(|entry| entry.property_id == action.property_id)
                .ok_or("a started transition with no transition-property entry")?;
            let easing = PublishedEasing::from_computed_timing_function(unsafe { &*entry.timing_function })
                .ok_or("a transition-timing-function the engine cannot describe")?;
            let retain = |value: *const StyleValueData| unsafe {
                RetainedStyleValueData::from_retained_pointer(crate::css::style_value::rust_style_value_retain(value))
            };
            let for_host_action = for_host.actions.last_mut().expect("the action was just pushed");
            for_host_action.start_value = start_value.cast();
            for_host_action.end_value = end_value.cast();
            for_host._values.push(retain(start_value));
            for_host._values.push(retain(end_value));
            started.push(StartedTransition {
                effect: PublishedEffect::for_css_transition(action.property_id, retain(start_value), retain(end_value)),
                row: AnimationTimingRow::for_new_css_transition(
                    node,
                    action.property_id,
                    action.delay,
                    action.active_duration,
                    &easing,
                ),
                easing,
            });
        }
        let removed = removed
            .iter()
            .filter_map(|property_id| {
                transitions
                    .iter()
                    .find(|transition| transition.property_id == *property_id)
                    .map(|transition| transition.effect_identity)
            })
            .collect();
        Ok(TransitionStep {
            started,
            removed,
            for_host,
        })
    }
}

impl StyleEngineState {
    /// Decide the transition step of a row that owes the whole step, which the host would decide
    /// when it installs the row, and compose what the transitions it starts and removes leave of the
    /// composition the row installs: the host then applies the decisions, and installs the
    /// composition as the row's. A step the engine cannot decide or compose is left to the host.
    pub(crate) fn decide_settled_row_transition_step(
        &mut self,
        node: StyleNodeID,
        old_style_record: u64,
        settled_style_record: u64,
        installed_style_record: u64,
        layout_arena: super::animations::LentLayoutArena,
        counters: &mut super::Counters,
    ) {
        let step = match self.decide_transition_step(node, old_style_record, installed_style_record, layout_arena) {
            Ok(step) => step,
            Err(reason) => {
                engine_sample_check::note_declined(&format!("transition step: {reason}"));
                return;
            }
        };
        // A step that removes a transition collects the element's effects again without it, which
        // the host does once it applied the row's animation plan, and the published rows are the
        // effects from before.
        if !step.removed.is_empty()
            && self
                .retained
                .nodes_owing_animation_definitions
                .contains_key(&(node, u8::MAX))
        {
            engine_sample_check::note_declined("transition step: a step that removes a transition beside a plan");
            return;
        }
        if !step.started.is_empty() || !step.removed.is_empty() {
            let removed = (!step.removed.is_empty()).then_some(&step.removed[..]);
            let published = crate::css::style_compute::sample_transition_step(
                self,
                node,
                installed_style_record,
                removed,
                &step.started,
                layout_arena,
            )
            .and_then(|overlay| {
                self.publish_transition_step_composition(
                    node,
                    settled_style_record,
                    installed_style_record,
                    overlay,
                    counters,
                )
                .map_err(String::from)
            });
            if let Err(reason) = published {
                engine_sample_check::note_declined(&format!("transition step: {reason}"));
                return;
            }
        }
        engine_sample_check::note_taken("transition step");
        self.retained
            .transition_steps_decided_in_pass
            .insert(node, step.for_host);
    }

    /// Publish the composition a step left as the element's record, as the pass publishes its
    /// sample of the row: over the record the row settled, with what the sample the step was
    /// layered over found out.
    fn publish_transition_step_composition(
        &mut self,
        node: StyleNodeID,
        settled_style_record: u64,
        installed_style_record: u64,
        overlay: Box<AnimatedOverlay>,
        counters: &mut super::Counters,
    ) -> Result<(), &'static str> {
        // Publishing the step's composition does not compose the element's animated custom
        // properties again.
        if self.retained.sampled_custom_property_environments.contains_key(&node) {
            return Err("a composition over animated custom properties");
        }
        let view = self
            .style_record_view(installed_style_record)
            .ok_or("an installed record with no view")?;
        let table = unsafe { view.longhand_table.as_ref() }.ok_or("an installed record with no table")?;
        // The display before the box-type transformation an animated display publishes with is the
        // one the host's step reconstructs from the installed record: the base record's display
        // where the installed composition animated display already, the table's otherwise.
        let installed_animates_display = unsafe { view.animated_overlay.as_ref() }
            .is_some_and(|installed| installed.get(property_id::DISPLAY).is_some());
        let animated_display_before_box_type_transformation =
            overlay
                .get(property_id::DISPLAY)
                .map(|_| match installed_animates_display {
                    true => {
                        let base_payloads = match view.base_payloads.is_empty() {
                            true => view.payloads,
                            false => view.base_payloads,
                        };
                        ComputedValuesView::new(SharedPayload::as_pointer_slice(base_payloads))
                            .box_values()
                            .display
                            .encoded()
                    }
                    false => table.display_before_box_type_transformation(),
                });
        let previous = self.retained.rows_sampled_in_pass.get(&node).copied();
        let sample = crate::css::style_compute::SettledRowSample {
            style_record: settled_style_record,
            keyframes_inherited_non_inherited_style_groups: previous
                .map_or(0, |previous| previous.keyframes_inherited_non_inherited_style_groups),
            uses_tree_counting_function: previous.is_some_and(|previous| previous.uses_tree_counting_function),
            substitution_marks: previous.map_or(0, |previous| previous.substitution_marks),
            animated_display_before_box_type_transformation,
            animated_custom_properties: Vec::new(),
            style: super::engine_sample::EngineSampledStyle {
                table: unsafe { crate::css::computed_longhand_table::rust_computed_longhand_table_retain(table) }
                    .cast_mut(),
                overlay: Box::into_raw(overlay),
            },
        };
        self.publish_settled_row_sample(node, sample, counters)?;
        Ok(())
    }

    /// Take the step the pass decided for a row, so that exactly one installation applies it. What
    /// it points at stays alive until the next take.
    pub(crate) fn take_transition_step_decided_in_pass(&mut self, node: StyleNodeID) -> Option<&TransitionStepForHost> {
        let step = self.retained.transition_steps_decided_in_pass.remove(&node)?;
        self.retained.taken_transition_step = Some(step);
        self.retained.taken_transition_step.as_ref()
    }

    /// A row the pass decides the step of again leaves no earlier decision behind.
    pub(crate) fn forget_transition_step_decided_in_pass(&mut self, node: StyleNodeID) {
        self.retained.transition_steps_decided_in_pass.remove(&node);
    }
}
