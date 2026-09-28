/*
 * Copyright (c) 2026-present, the Ladybird developers.
 *
 * SPDX-License-Identifier: BSD-2-Clause
 */

mod drive;
mod pseudo;
mod winner_store;

use winner_store::{WinnerDeclaration, WinnerStore, WinnerValue, shorthand_longhand_data};

use super::style_invalidation::property_feeds_post_compute_adjustment;
use super::*;
use crate::css::computed_longhand_table::ComputedLonghandTable;
pub(crate) use drive::drive_font_metric;
pub(super) use drive::{Drive, Suspension, Unanswered};
use drive::{FontDriveGoal, FullDrive, PartialDrive, table_names_animations};
pub(crate) use pseudo::OwedPseudoSettle;

/// Another element's published style that a first-time computation may build over: the element
/// whose cascade state stands in for the previous one, and the record it must still hold.
#[derive(Clone, Copy, Debug)]
#[cfg(any(test, feature = "style-recording"))]
pub(crate) struct ExactCascadeDonor {
    pub node: StyleNodeID,
    pub style_record: u64,
}

#[cfg(any(test, feature = "style-recording"))]
pub(super) struct ExactCascadeContext {
    previous: Option<CascadeStateID>,
    lower_bound_state: Option<CascadeStateID>,
    dependency_target: computed::ComputedStyleTarget,
    donor_used: bool,
}

/// Root-relative computation reads these inputs independently of its inheritance parent.
/// Bitwise keys keep equality exact without using an invalidation generation as a substitute
/// for the values. The viewport-dependence bit matters even when today's metrics agree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) struct RootFontInputs {
    pub(super) metrics: [u64; 5],
    pub(super) depends_on_viewport: bool,
}

/// An element's old and new style records.
pub(super) type RecordDelta = (computed::FinalStyleRecordID, computed::FinalStyleRecordID);

/// What the computation of an element's record answers: its record delta, or, for the root-input
/// probe, the document element's font inputs where the probe proved them.
pub(super) enum ElementAnswer {
    Delta(RecordDelta),
    RootInputs(Option<RootFontInputs>),
}

impl ElementAnswer {
    /// The delta of a computation that was no root-input probe.
    fn delta(self) -> RecordDelta {
        match self {
            Self::Delta(delta) => delta,
            Self::RootInputs(_) => unreachable!("only the root-input probe answers with root inputs"),
        }
    }
}

impl RootFontInputs {
    fn apply_to(self, inputs: &mut bridge::FfiDocumentStyleComputationInputs) {
        inputs.root_font_size = f64::from_bits(self.metrics[0]);
        inputs.root_font_x_height = f64::from_bits(self.metrics[1]);
        inputs.root_font_cap_height = f64::from_bits(self.metrics[2]);
        inputs.root_font_zero_advance = f64::from_bits(self.metrics[3]);
        inputs.root_line_height = f64::from_bits(self.metrics[4]);
        inputs.root_font_metrics_depend_on_viewport_metrics = self.depends_on_viewport;
    }

    pub(super) fn from_document(inputs: &bridge::FfiDocumentStyleComputationInputs) -> Self {
        Self {
            metrics: [
                inputs.root_font_size.to_bits(),
                inputs.root_font_x_height.to_bits(),
                inputs.root_font_cap_height.to_bits(),
                inputs.root_font_zero_advance.to_bits(),
                inputs.root_line_height.to_bits(),
            ],
            depends_on_viewport: inputs.root_font_metrics_depend_on_viewport_metrics,
        }
    }
}

impl RetainedState {
    /// The inheritance parent a node's record is computed from. C++ styles an element whose
    /// inheritance parent has no style, such as a slot inside a `display: none` subtree, from the
    /// initial values, the way it styles the document element. A parent whose record the host
    /// has yet to install is no such parent: the pass stops before its children until it is
    /// installed, and a demand settles its ancestors first.
    pub(super) fn record_inheritance_parent(&self, node: StyleNodeID) -> Option<StyleNodeID> {
        self.tree
            .inheritance_parent(node)
            .filter(|&parent| self.computed_group_sets.assigned_style_record(parent).is_some())
    }

    pub(crate) fn retained_highlight_inheritance_parent_style_record(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> Option<computed::FinalStyleRecordID> {
        let mut ancestor = self.tree.inheritance_parent(node);
        while let Some(candidate) = ancestor {
            if let Some(record) = self.computed_group_sets.pseudo_style_record(candidate, pseudo_kind) {
                return Some(record);
            }
            ancestor = self.tree.inheritance_parent(candidate);
        }
        None
    }

    fn retained_inheritance_parent_node(&self, node: StyleNodeID, pseudo_kind: u8) -> Option<StyleNodeID> {
        let parent = if pseudo_kind == crate::css::cascaded_properties::NO_PSEUDO_ELEMENT {
            self.tree.inheritance_parent(node)?
        } else if (bridge::FIRST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND
            ..=bridge::LAST_ELEMENT_REFERENCE_PSEUDO_ELEMENT_KIND)
            .contains(&pseudo_kind)
        {
            let Some(shadow_root) = self.tree.shadow_root_of(node) else {
                return Some(node);
            };
            let mut pending = self.tree.dom_children(shadow_root).collect::<Vec<_>>();
            let represented_element = loop {
                let Some(candidate) = pending.pop() else {
                    break None;
                };
                if self.computed_group_sets.associated_pseudo_kind(candidate) == Some(pseudo_kind) {
                    break Some(candidate);
                }
                pending.extend(self.tree.dom_children(candidate));
            };
            match represented_element.and_then(|represented_element| self.tree.parent(represented_element)) {
                Some(parent) if parent != shadow_root => parent,
                _ => node,
            }
        } else {
            node
        };
        Some(parent)
    }

    pub(super) fn retained_store_supports_property(target: computed::ComputedStyleTarget, property: u16) -> bool {
        if property > crate::css::property_metadata::LAST_LONGHAND_PROPERTY_ID
            || (crate::css::property_metadata::property_id::ANIMATION_COMPOSITION
                ..=crate::css::property_metadata::property_id::ANIMATION_TIMING_FUNCTION)
                .contains(&property)
            || (crate::css::property_metadata::property_id::TRANSITION_BEHAVIOR
                ..=crate::css::property_metadata::property_id::TRANSITION_TIMING_FUNCTION)
                .contains(&property)
            || crate::css::property_metadata::property_is_in_logical_group(property)
        {
            return false;
        }
        !target.is_pseudo()
            || (property != crate::css::property_metadata::property_id::CONTENT
                && crate::css::property_metadata::pseudo_element_supports_property(target.pseudo_kind(), property))
    }

    #[allow(dead_code)]
    pub(crate) fn engine_constructed_cascade_store(
        &self,
        target: computed::ComputedStyleTarget,
    ) -> Option<CascadedPropertyStore> {
        let answer = self.current_published_answer(target.node())?;
        if !answer.cascade_winners_are_complete {
            return None;
        }
        let key = target.pseudo_element_target().map_or_else(
            || WinnerGroupKey::current(target.node(), self.program.version()),
            |pseudo| WinnerGroupKey::current_pseudo(target.node(), pseudo, self.program.version()),
        );
        let Lookup::Known((_, state)) = self.current_winner_groups().token_for(key) else {
            return None;
        };

        let mut store = CascadedPropertyStore::new();
        for winner in self.winner_groups.winners_in_state(state) {
            let winner = self.winner_groups.resolved_winner(winner)?;
            if !Self::retained_store_supports_property(target, winner.property) {
                return None;
            }
            let Lookup::Known(value) = self.specified_values.retained_value(winner.key.value) else {
                return None;
            };
            if crate::css::style_compute::external_value_dependencies(value.data())
                .may_need_style_sheet_resource_context
            {
                return None;
            }
            store.seed_retained_property(winner.property, value, winner.important, false);
        }
        Some(store)
    }

    /// Derive the record a published-style reaction moves `node` to, when the engine can compute
    /// it exactly: the winners that moved compute in the drive's remaining phase from the values
    /// their declarations were written with, against the record's own font, the document's
    /// computation inputs, and the parent's record. Every node of one cohort - the same old
    /// record moved to the same winner state - derives the same record, so the second and later
    /// members take the first one's answer.
    pub(super) fn engine_computed_record_delta(
        &mut self,
        node: StyleNodeID,
        cascade_winners_are_complete: bool,
        exact_flipped_rules: Option<FlippedRules>,
        parent_inputs_moved: ParentInputsMoved,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> Drive<RecordDelta> {
        let delta = self.decide_engine_computed_record_delta(
            node,
            cascade_winners_are_complete,
            exact_flipped_rules,
            parent_inputs_moved,
            scratch,
            counters,
        );
        self.apply_substitution_effects(scratch);
        delta
    }

    /// Whether a document hosts this engine. The host installs its font resolver when it creates the engine, and
    /// publishes the document's inputs and every table a record is computed against before it asks for any row. An
    /// engine no document hosts, such as a unit test's or a replay's, computes no records. Only those builds can
    /// create such an engine, so the browser build has no unhosted path at all.
    #[cfg(any(test, feature = "style-replay"))]
    pub(super) fn computes_records(&self) -> bool {
        self.font_resolution.is_some()
    }

    /// A browser build's engine always has a document to host it.
    #[cfg(not(any(test, feature = "style-replay")))]
    pub(super) const fn computes_records(&self) -> bool {
        true
    }

    /// The step itself, which decides the substituted-record facts rather than writing them.
    #[allow(clippy::too_many_arguments)]
    fn decide_engine_computed_record_delta(
        &mut self,
        node: StyleNodeID,
        cascade_winners_are_complete: bool,
        exact_flipped_rules: Option<FlippedRules>,
        parent_inputs_moved: ParentInputsMoved,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> Drive<RecordDelta> {
        debug_assert!(self.computes_records(), "only a hosted engine drives records");
        let pending_element = scratch.pending_element.take();
        if pending_element.is_none() {
            scratch.pseudo_deltas.clear();
            scratch.next_pseudo = 0;
            scratch.pseudo_explicitly_inherited_groups = 0;
            scratch.pseudo_uses_substitution = false;
            scratch.noted_substitution = None;
            scratch.flipped_pseudo_rules = exact_flipped_rules.map_or(0, |flipped| flipped.pseudos);
        }
        let delta = match pending_element {
            Some(delta) => delta,
            None => self
                .engine_computed_element_record_delta(
                    node,
                    cascade_winners_are_complete,
                    exact_flipped_rules,
                    parent_inputs_moved,
                    scratch,
                    FontDriveGoal::Complete,
                    counters,
                )?
                .delta(),
        };
        // The element's pseudo-elements are settled beside its record, as the C++ computation
        // refreshes them after the element's own; a pseudo-element the engine cannot settle
        // sends the whole element to C++.
        // A pseudo-element asks about its originating element too. The element's new record may
        // have changed that container's type, name or style after the first verdict check.
        if self.container_verdicts_moved(node) {
            self.republish_driven_winners(node, counters);
            self.container_effects_for_host.remove(&node);
            // The republish published the verdicts it just evaluated; this notes their effects
            // for the host.
            let verdicts_stand = self.container_verdicts_stand(node);
            debug_assert!(
                verdicts_stand,
                "container verdicts moved after their winners were republished"
            );
        }
        let old_style_record = (delta.0 != computed::FinalStyleRecordID::NONE).then_some(delta.0);
        let generation = self.winner_groups.generation();
        if let Err(unanswered) =
            self.engine_pseudo_records(node, old_style_record, delta.1, generation, scratch, counters)
        {
            let Unanswered::Suspended(_) = unanswered;
            scratch.pending_element = Some(delta);
            return Err(unanswered);
        }
        if let Lookup::Known((_, state)) = self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        {
            self.note_container_unit_effects_for_host(node, delta.1, self.state_container_unit_mask(node, state));
        }
        // What this element's record was computed from decides the fact when the computation
        // noted it; a record that stands unchanged keeps the fact the node already carries.
        let uses_substitution = scratch
            .noted_substitution
            .unwrap_or_else(|| self.nodes_with_substituted_records.contains(&node))
            || scratch.pseudo_uses_substitution
            || self
                .current_winner_groups()
                .pseudo_states(node)
                .any(|(_, version, state, priority_current)| {
                    version == self.program.version() && priority_current && self.state_has_substitutions(node, state)
                });
        scratch.element_uses_substitution = uses_substitution;
        scratch.substitution_effects.push((node, uses_substitution));
        Ok(delta)
    }

    /// The winners a driven element's record is computed from. A republish admits the row
    /// whatever the memory budget; a demand's pending winners can still stand over the row it
    /// published, and an answer that published nothing is matched again.
    fn driven_element_winners(
        &mut self,
        node: StyleNodeID,
        winner_key: WinnerGroupKey,
        counters: &mut Counters,
    ) -> Option<(u64, CascadeStateID)> {
        if let Lookup::Known(token) = self.current_winner_groups().token_for(winner_key) {
            return Some(token);
        }
        if let Lookup::Known(token) = self.winner_groups.token_for(winner_key) {
            return Some(token);
        }
        self.rematch_driven_winners(node, counters);
        match self.winner_groups.token_for(winner_key) {
            Lookup::Known(token) => Some(token),
            _ => None,
        }
    }

    /// A driven element that holds no winners even after matching again has nothing to compute
    /// a record from. None does: a rematch publishes its row whatever the memory budget. Should
    /// one, the element keeps the record it has.
    fn driven_element_without_winners(&self, node: StyleNodeID) -> RecordDelta {
        debug_assert!(false, "a driven element holds no winners after matching again");
        let record = self
            .computed_group_sets
            .assigned_style_record(node)
            .unwrap_or(computed::FinalStyleRecordID::NONE);
        (record, record)
    }

    /// `exact_flipped_rules` are the rules that flipped for the node when the reaction is exactly
    /// those flips and nothing else the record depends on moved. `parent_inputs_moved` says which
    /// of the parent's inputs may have moved under the record.
    #[allow(clippy::too_many_arguments)]
    fn engine_computed_element_record_delta(
        &mut self,
        node: StyleNodeID,
        mut cascade_winners_are_complete: bool,
        exact_flipped_rules: Option<FlippedRules>,
        mut parent_inputs_moved: ParentInputsMoved,
        scratch: &mut EngineComputedRecordScratch,
        goal: FontDriveGoal,
        counters: &mut Counters,
    ) -> Drive<ElementAnswer> {
        use crate::css::computed_value_types::{
            STYLE_GROUP_INDEX_ANCHOR, STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_SURROUND,
        };
        use crate::css::property_metadata::{FIRST_LONGHAND_PROPERTY_ID, LONGHAND_WORD_COUNT};

        let target = computed::ComputedStyleTarget::new(node, u8::MAX);
        if let Some(backed) = self.backed_host_pseudo_element(node) {
            return self
                .engine_backing_element_record(node, backed, cascade_winners_are_complete, scratch, counters)
                .map(ElementAnswer::Delta);
        }
        // The winners hold a gated rule where its container conditions held when they were
        // published; they answer for the node while every one decides as it did, over containers
        // its settled ancestors published. The pass drives the node only once those are installed.
        // A row omitted from winner publication can still carry a retained selector answer.
        // Rebuild its winners before comparing them with the record's cascade state: otherwise
        // an empty delta can describe yesterday's answer after this flush flipped a rule. So does
        // a row that holds no winners for the current program at all. Exact flips of pseudo-element
        // rules alone leave the element row as it was: the pseudo rows are refreshed where the
        // pseudo-elements settle.
        let winner_key = WinnerGroupKey::current(node, self.program.version());
        let stale_element_winners = (scratch.answer_or_declarations_moved
            && exact_flipped_rules.is_none_or(|flipped| flipped.element)
            && !self.published_container_verdicts.contains_key(&node)
            && !self.container_gates_unheld.contains(&node)
            && self.current_winner_groups().row_stamp(node) != Some(self.flush_stamp))
            || !matches!(self.current_winner_groups().token_for(winner_key), Lookup::Known(_));
        if self.container_gates_unheld.contains(&node) || self.container_verdicts_moved(node) || stale_element_winners {
            // A published row has the fact row its winners are matched from.
            cascade_winners_are_complete = self.republish_driven_winners(node, counters);
        }
        // Verdicts that did not move stand, and a republish published the ones it just evaluated;
        // this notes their effects for the host.
        let verdicts_stand = self.container_verdicts_stand(node);
        debug_assert!(
            verdicts_stand,
            "container verdicts moved after their winners were republished"
        );
        // A custom property the cascade declares is no winner the columns hold; the engine
        // computes the environment it decides itself.
        debug_assert!(
            cascade_winners_are_complete || self.cascade_winners_are_complete_but_for_custom_properties(node),
            "a driven row's winners are complete but for custom properties"
        );
        // The winners the record was computed from, against the winners the node holds now: the
        // same comparison a C++ publication makes to select what it recomputes.
        let Some((generation, state)) = self.driven_element_winners(node, winner_key, counters) else {
            return Ok(ElementAnswer::Delta(self.driven_element_without_winners(node)));
        };
        let written = self.state_written_facts(node, state);
        let container_unit_mask = written.container_relative_length_unit_mask;
        if written.has_written_tree_counting || self.nodes_with_tree_counting_records.contains(&node) {
            scratch.recompute_in_full = true;
        }
        // An element's animations compose into its style in the C++ computation.
        let facts = self.computed_group_sets.adjustment_facts(node);
        // Either the root's font inputs moved under this element, or the element's own font
        // environment did: a face its cascade names became available or failed. The winners are
        // the same either way, so nothing else below would notice, and the record has to be
        // driven again in full rather than stand. The root's font the root-input probe computed
        // is left pending for the root's own row, which resumes it the same way.
        let font_inputs_moved = (scratch.root_font_inputs_changed
            && facts & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT == 0)
            || scratch.font_environment_moved
            || scratch.font_drive.is_pending_for(node);
        // An element's animations compose into its style in the C++ computation, and the record it
        // holds is the one they were composed into. Deriving another record from it, or moving it
        // to another environment, would publish the composition as if it were the element's own
        // style - so all of that stays in C++ while the element animates. Answering with the very
        // record the element already holds does not: it is what the row asks for when nothing the
        // record was computed from has moved, and the overlay on it is the host's either way.
        let animations_bind_the_record = facts & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0;
        let has_registered_declarations =
            self.declares_registered_custom_property(node, None, &self.document_style_computation_inputs);
        let Some(old_style_record) = self.computed_group_sets.assigned_style_record(node) else {
            return self.engine_cold_record(node, (generation, state), scratch, goal, counters);
        };
        // The same of a record that holds an animation overlay. What the transitions its table
        // declares would start is a different question, decided below against the values the
        // delta moves.
        let animations_bind_the_record =
            animations_bind_the_record || self.record_holds_an_animation_overlay(old_style_record);
        // Even the record that stands is not an answer for an element whose CSS animations were
        // planned against a `@keyframes` table that has since moved: which keyframes an animation
        // runs is decided by the plan, the plan is no part of the record, and only the computation
        // that re-plans it can tell the element. An element animating through the Web Animations
        // API or a transition has no rule behind it at all, and one whose plan was published
        // against the table as it stands has already been told.
        let record_may_stand_while_animating = !animations_bind_the_record
            || !self.css_defined_animations.node_runs_a_css_animation(node)
            || self
                .css_defined_animations
                .node_is_planned_against(node, self.animation_keyframes.generation());
        // A record C++ computed holds no cascade state, so there is no earlier state to take a
        // delta from: the record is driven again in full from the node's winners, which binds
        // the state.
        // A winner written with `attr()` reads the element's attributes, and a substituted value
        // reads the custom-property registry. Neither change appears in a winner delta.
        if written.reads_attributes
            || (self.custom_property_registrations_changed && self.node_style_reads_custom_properties(node))
            // A function body or an if() condition can change without moving the declaration
            // that calls it. The record has no version for those external inputs, so a requested
            // row must resolve its ordinary winners again even when their cascade delta is empty.
            || written.custom_condition_usage & ((1 << 0) | (1 << 2)) != 0
        {
            scratch.recompute_in_full = true;
        }
        let delta = match self.computed_group_sets.cascade_state(target) {
            Some((previous_generation, previous_state)) => {
                if previous_generation != generation {
                    // The old record's cascade state belongs to a replaced rule program. Drive
                    // the current winners in full instead of comparing states from different
                    // generations; the old record still supplies the before-change values.
                    scratch.recompute_in_full = true;
                    self.winner_groups.semantic_delta(Some(state), state)
                } else {
                    self.winner_groups.semantic_delta(Some(previous_state), state)
                }
            }
            None => {
                scratch.recompute_in_full = true;
                self.winner_groups.semantic_delta(Some(state), state)
            }
        };
        let mut inputs = self.document_style_computation_inputs;
        if let Some((root, root_inputs)) = scratch.root_element_inputs
            && root == node
        {
            root_inputs.apply_to(&mut inputs);
        }
        // The environment the node's own custom declarations resolve to over the parent's. A node
        // declaring none keeps its record's, which is the parent's; a moved environment
        // republishes the record under the new one.
        let (mut environment, mut current_environment) = {
            let parent_environment = self
                .record_inheritance_parent(node)
                .map_or(0, |parent| self.held_custom_property_environment(parent));
            // The row keeps the record it reads its own font from unless the font itself is
            // moving, so that is when a registered name can be computed here rather than by the
            // host: the value absolutizes against the same metrics C++ would use.
            let registered = self
                .own_font_length_resolution_context(old_style_record, &inputs)
                .map(|(length, color_scheme)| custom_property_cascade::RegisteredValueContext { length, color_scheme });
            let environment =
                self.engine_custom_property_environment(node, parent_environment, &inputs, registered, counters)?;
            // A record an animation composed into was published with no environment of its own:
            // what the element's own declarations resolved to is on the style beneath it. Every
            // installed record is published with one; a record without is republished as moved.
            let old_environment = self
                .computed_group_sets
                .animation_overlay_base_custom_property_environment(old_style_record.raw())
                .or_else(|| {
                    self.computed_group_sets
                        .style_record_custom_property_environment(old_style_record.raw())
                });
            debug_assert!(
                old_environment.is_some(),
                "an installed record was published with a custom-property environment"
            );
            // An unmoved environment is the one the record holds, so the current one is always
            // what the declarations resolved to.
            (
                (old_environment != Some(environment)).then_some(environment),
                environment,
            )
        };
        // A moved environment reaches every winner written with a substitution: such a record
        // is driven again in full under the new one.
        let environment_moved_under_substitutions = environment.is_some() && written.has_substitutions;
        // A CSS animation planned against a `@keyframes` table that has since moved is planned
        // again from the record's longhand table; a record holding none is driven again in full,
        // which settles the plan from the table the drive computes.
        if !record_may_stand_while_animating
            && self
                .computed_group_sets
                .style_record_view(old_style_record.raw())
                .is_none_or(|view| view.longhand_table.is_null())
        {
            scratch.recompute_in_full = true;
        }
        if delta.is_empty() {
            // A moved environment beneath transitions, or beneath a composition the host cannot
            // sample again over the new base, leaves them to a record driven again in full.
            if environment.is_some()
                && animations_bind_the_record
                && (self.record_declares_transitions(old_style_record)
                    || (self.computed_group_sets.node_has_animation_overlay(node)
                        && !self.composition_resamples_over_a_new_base(old_style_record, facts)))
            {
                scratch.recompute_in_full = true;
            }
            // A sample over the standing base cannot adjust the base values an animated box-type,
            // overflow, or text-alignment input feeds, or its sample would ask for this row again.
            // Such a composition is driven again beneath its overlay and sampled over the new base.
            if computed::ComputedGroupSets::record_is_animation_overlay(old_style_record.raw())
                && self.composition_feeds_a_post_compute_adjustment(old_style_record)
            {
                scratch.recompute_in_full = true;
            }
            // A record whose environment alone moves under a running CSS animation keeps its
            // plan, decided from the record's table. A record holding none is driven in full,
            // which decides the plan from the driven table instead.
            let standing_animation_plan = (environment.is_some()
                && animations_bind_the_record
                && self.css_defined_animations.node_runs_a_css_animation(node))
            .then(|| {
                self.settled_animation_plan_from_record(
                    node,
                    old_style_record,
                    self.animation_name_declaration_scope(node, state),
                )
            });
            if let Some(None) = standing_animation_plan {
                scratch.recompute_in_full = true;
            }
            // A flipped rule that lost the cascade may leave this row's stamp old, but an exact
            // retained-answer publication has already established the same semantic winners.
            // A document environment action keeps those winners and drives their values again
            // below against the document inputs published for this transaction.
            // The winners stand while the parent's inherited style or display moved under the
            // record: it is driven again in full against the parent as it is now. The record
            // does not say which parent display it was transformed under, and a winner's own
            // value may read the parent (a relative length, an inherit keyword).
            if !parent_inputs_moved.any()
                && !font_inputs_moved
                && !environment_moved_under_substitutions
                && !scratch.document_environment_moved
                && !scratch.recompute_in_full
                && !(scratch.viewport_moved && self.record_reads_the_viewport(old_style_record))
            {
                // A declaration in an inherited payload group does not prove that the other
                // properties in that group still inherit from the current parent. Re-drive the
                // record in full when its payloads cannot prove the relationship.
                if self.record_inherits_from_current_parent(node, state, 0)
                    || (environment.is_some()
                        && animations_bind_the_record
                        && self.animation_base_inherits_from_current_parent(node, state, old_style_record))
                {
                    if goal == FontDriveGoal::RootInputs {
                        // NB: This proof covers the retained font, without publishing the root's
                        //     remaining properties or custom-property environment during preparation.
                        return Ok(ElementAnswer::RootInputs(
                            self.root_font_inputs_from_record(old_style_record),
                        ));
                    }
                    // Only the environment moved: keep the old composition alive while the host
                    // samples its effects over a candidate with the new base environment. A CSS
                    // animation also needs its plan applied before that sample.
                    if let Some(environment) = environment
                        && animations_bind_the_record
                    {
                        let css_animation_plan = standing_animation_plan.flatten();
                        // With no sampled overlay, the old record is already the animation's base. It
                        // moves to the new environment like any record, and the host applies the plan
                        // and samples the element's effects over it after installing it.
                        if !self.computed_group_sets.node_has_animation_overlay(node) {
                            let delta = self
                                .computed_group_sets
                                .republish_engine_record_with_environment(node, environment)
                                .expect("an assigned record without an overlay moves to any environment");
                            self.note_engine_computed_record(node, delta, (generation, state), 0, 0, counters);
                            if let Some(plan) = css_animation_plan {
                                self.nodes_owing_animation_definitions.insert((node, u8::MAX), plan);
                            }
                            self.nodes_owing_an_animation_sample.insert(node);
                            return Ok(ElementAnswer::Delta(delta));
                        }
                        // An element's overlay is always in its slot: a record composed over a
                        // base is only ever assigned through one.
                        let assembly = self
                            .computed_group_sets
                            .republish_animated_base_with_environment(node, environment)
                            .expect("an assigned record with an overlay holds its slot");
                        self.batch_pinned_compositions.push((
                            node,
                            assembly.pinned_composition.expect("a warm composition was retained"),
                        ));
                        let delta = assembly.delta;
                        self.note_engine_computed_record(node, delta, (generation, state), 0, 0, counters);
                        if let Some(plan) = css_animation_plan {
                            self.nodes_owing_animation_definitions.insert((node, u8::MAX), plan);
                        }
                        self.nodes_owing_an_animation_sample.insert(node);
                        return Ok(ElementAnswer::Delta(delta));
                    }
                    // Where animations do not bind the record, it holds no overlay.
                    if let Some(environment) = environment {
                        let delta = self
                            .computed_group_sets
                            .republish_engine_record_with_environment(node, environment)
                            .expect("an assigned record without an overlay moves to any environment");
                        counters.bump(Counter::EngineComputedRecordUnchangedWinners);
                        self.note_engine_computed_record(node, delta, (generation, state), 0, 0, counters);
                        return Ok(ElementAnswer::Delta(delta));
                    }
                    // An animation overlay's record lives in a slot the next sample releases.
                    // Keep the composition alive through installation and sample it again. A
                    // stale CSS animation plan is decided from the unchanged longhand table.
                    let css_animation_plan = (!record_may_stand_while_animating)
                        .then(|| {
                            self.settled_animation_plan_from_record(
                                node,
                                old_style_record,
                                self.animation_name_declaration_scope(node, state),
                            )
                        })
                        .flatten();
                    if computed::ComputedGroupSets::record_is_animation_overlay(old_style_record.raw()) {
                        self.computed_group_sets.pin_style_record(old_style_record.raw());
                        self.batch_pinned_compositions.push((node, old_style_record.raw()));
                        self.nodes_owing_an_animation_sample.insert(node);
                    }
                    if let Some(plan) = css_animation_plan {
                        self.nodes_owing_animation_definitions.insert((node, u8::MAX), plan);
                    }
                    counters.bump(Counter::EngineComputedRecordUnchangedWinners);
                    counters.bump(Counter::CascadeWinnerDeltaStops);
                    self.note_engine_computed_record(
                        node,
                        (old_style_record, old_style_record),
                        (generation, state),
                        0,
                        0,
                        counters,
                    );
                    return Ok(ElementAnswer::Delta((old_style_record, old_style_record)));
                }
                // Otherwise the parent's inherited style moved under the record without the row
                // being told: the parent is authoritative here (an ancestor the host drives in this
                // batch holds the row back before it gets this far), so the record is driven again
                // in full against it.
                parent_inputs_moved.inherited_style = true;
            }
        }
        let requires_full_drive = parent_inputs_moved.any()
            || font_inputs_moved
            || environment_moved_under_substitutions
            || scratch.document_environment_moved
            || scratch.recompute_in_full
            || delta.properties().iter().any(|&property| {
                !property_computes_in_remaining_phase(property) || property_feeds_box_type_transformation(property)
            });
        // A moved animation declaration or a changed keyframes table leaves a plan for the host
        // to apply before descendants continue, beside any transition step the row also owes: the
        // host applies the plan, samples the installed record, and then runs the step against the
        // record the row moved away from, all where it applies the row.
        let owes_an_animation_plan = delta
            .properties()
            .iter()
            .any(|&property| longhand_declares_a_css_animation(property))
            || (!record_may_stand_while_animating
                && !self
                    .element_css_defined_animations(node, animations::ELEMENT_ANIMATION_SLOT)
                    .is_empty());
        // An element's animations compose over the record the row derives. Its base is driven in
        // full, from the winners and the parent as they are now, and the host samples every effect
        // the element holds over it once installed: the sample composes the animated font and the
        // groups it writes, and makes the box-type, overflow, and text-alignment adjustments, the
        // same way over any freshly driven base. A record that runs or names CSS animations
        // settles its complete plan from the new base, which the host applies before sampling. A
        // transition is decided at installation against the composed record the row moves away
        // from. An element whose effects have all gone still holds its last composition; the
        // sample over the new base, with nothing left to compose, clears it. A child derived in the
        // same batch waits for that composition.
        let derived_beneath_a_composition = animations_bind_the_record;
        let names_an_animation = !self.state_has_no_animation_name(state);
        // A moved font-phase longhand reaches every value the font feeds, so the record is driven
        // through every phase and every group is rebuilt. A moved box-type transformation input
        // takes the same route, as does a record whose parent inputs moved or a record that read
        // the viewport under a viewport move: the transformation and the inheritance are part of
        // the full drive.
        let full_drive = requires_full_drive
            || derived_beneath_a_composition
            || (scratch.viewport_moved && self.record_reads_the_viewport(old_style_record))
            || environment_moved_under_substitutions
            || scratch.document_environment_moved
            || scratch.recompute_in_full
            || (has_registered_declarations && environment.is_some())
            || delta.properties().iter().any(|&property| {
                !property_computes_in_remaining_phase(property)
                    || font_group_carries_longhand(property)
                    || property_feeds_box_type_transformation(property)
            });
        let delta_property_count = delta.properties().len() as u64;
        // A delta that moves a longhand declaring the element's CSS transitions is a row whose only
        // remaining obligation is the transition step, and the host can run that step after the
        // batch: it needs the style the row moved away from, which the published delta names, and
        // the style it moved to, which is the record the host installs. The record the delta starts
        // at is asked to hold no animation of its own - which is what the record's animation state
        // is asked about here, not somewhere else's - so the step decides over transitions alone,
        // and nothing the element already holds can be cancelled. A derived base beneath an
        // unrelated composition is also safe: no moved property is covered by that overlay, and
        // the host samples it before applying the transition step to the installed record.
        //
        // When the delta moves nothing else, every other group is copied from the record the delta
        // starts at, so no value a transition runs on moved either: the registration is the whole
        // of the step, and the host takes the cheaper of the two drains.
        //
        // A record whose table already declares transitions owes the step whatever the delta
        // moved, since the step is where the element's before-change style is kept up to date.
        // A delta that moves a longhand one of them runs on starts a transition, and the started
        // transition samples its start value into an overlay the host step publishes over the
        // installed record; a child derived in the same batch waits for that composition. Such a
        // record can drive its base in full and leave the whole transition decision to
        // installation, including when its parent's inherited style moved: the decision reads an
        // inherited animated value's after-change value from the ancestor that animates it, or
        // its parent's or its own display moved: the host applies a display change after the step,
        // as a C++ computation does. The step decides nothing for a hidden record.
        let record_declares_transitions = self.record_owes_a_transition_decision(old_style_record);
        let old_record_is_hidden = self
            .computed_group_sets
            .style_record_view(old_style_record.raw())
            .is_some_and(|view| view.dependency_flags & (1 << 2) != 0);
        let owes_a_transition_step = (!full_drive
            || delta
                .properties()
                .iter()
                .any(|&property| longhand_only_declares_a_css_transition(property))
            || match record_declares_transitions {
                true => !scratch.ancestor_became_visible,
                false => {
                    !delta
                        .properties()
                        .contains(&crate::css::property_metadata::property_id::DISPLAY)
                        && self.tree.flat_tree_children(node).all(|child| child.is_text())
                }
            })
            && (record_declares_transitions
                || delta
                    .properties()
                    .iter()
                    .any(|&property| longhand_only_declares_a_css_transition(property)));
        let owes_a_transition_registration = owes_a_transition_step.then(|| {
            !full_drive
                && !record_declares_transitions
                && delta
                    .properties()
                    .iter()
                    .all(|&property| longhand_only_declares_a_css_transition(property))
        });
        // Partial drives can share across parents whose inherited inputs agree. Keep the full
        // parent record in the key when a non-inherited property explicitly inherits, including
        // through substitution, or when a full drive may read more of the parent's style.
        let parent = self.record_inheritance_parent(node);
        let parent_record = parent.and_then(|parent| self.computed_group_sets.assigned_style_record(parent));
        let mut cohort_parent = RecordDeltaParent::Exact(parent_record.map_or(0, |record| record.raw()));
        if !full_drive
            && let (Some(parent), Some(parent_record)) = (parent, parent_record)
            && !written.has_substitutions
            && let Some(inputs) = self.cold_record_parent(node, parent, parent_record, state)
        {
            cohort_parent = RecordDeltaParent::Inputs(inputs);
        }
        // The environment the record is published under, which the key names whether it moved or
        // not: a node moving to the empty environment and one keeping its record's must not meet.
        let cohort = (
            generation,
            old_style_record.raw(),
            state,
            facts,
            cohort_parent,
            current_environment,
            RootFontInputs::from_document(&inputs),
            self.monospace_cohort_key(node, state),
            self.element_reads(node, None, state, current_environment),
        );
        // The row takes another node's record whole, so its plan is decided from that record's
        // own longhands rather than from a drive of this node's; a record holding none is no
        // cohort for a row that owes a plan, which drives its own instead.
        if let Some((new_style_record, cohort_explicitly_inherited_groups, animation_plan)) = (container_unit_mask == 0
            && self.state_custom_condition_usage(node, state) == 0
            && !self.computed_group_sets.node_has_animation_overlay(node)
            && !derived_beneath_a_composition
            && (!has_registered_declarations || !full_drive))
            .then(|| {
                scratch.cohorts.get(&cohort).copied().or_else(|| {
                    self.may_take_a_kept_warm_record_cohort(node)
                        .then(|| self.engine_warm_record_cohorts.get(&cohort).copied())
                        .flatten()
                        .filter(|&(record, _)| self.warm_record_cohort_is_current(node, record))
                })
            })
            .flatten()
            .and_then(|(record, groups)| match owes_an_animation_plan {
                true => self
                    .settled_animation_plan_from_record(
                        node,
                        record,
                        self.animation_name_declaration_scope(node, state),
                    )
                    .map(|plan| (record, groups, Some(plan))),
                false => Some((record, groups, None)),
            })
        {
            self.note_node_substitution(node, scratch, state, current_environment);
            // The cohort is keyed by the node's own assigned record and taken only without an
            // overlay, and its record was published this flush.
            let delta = self
                .computed_group_sets
                .assign_engine_computed_record(node, old_style_record, new_style_record)
                .expect("a cohort record moves any node holding the cohort's old record");
            if delta.0 == delta.1 {
                counters.bump(Counter::ComputedWinnerPropagationStops);
            }
            self.note_engine_computed_record(node, delta, (generation, state), delta_property_count, 0, counters);
            counters.bump(Counter::EngineComputedRecordCohortHits);
            // The debt is per node, not per record: an element taking the record from the cohort
            // owes its own parent the same mark the element that computed it owed.
            if cohort_explicitly_inherited_groups != 0 {
                self.nodes_owing_explicit_inheritance
                    .insert(node, cohort_explicitly_inherited_groups);
            }
            match owes_a_transition_registration {
                Some(registration_only) => self
                    .nodes_owing_a_transition_registration
                    .insert(node, registration_only),
                None => self.nodes_owing_a_transition_registration.remove(&node),
            };
            if let Some(plan) = animation_plan {
                self.nodes_owing_animation_definitions.insert((node, u8::MAX), plan);
            }
            return Ok(ElementAnswer::Delta(delta));
        }

        // The moved properties, the groups they feed, and the drive selection. A moved member of
        // a logical property group takes its counterpart along: which of the pair the other
        // derives from is a cascade decision the drive makes for both.
        let mut groups_to_rebuild = 0_u32;
        let mut selected = [0_u64; LONGHAND_WORD_COUNT];
        let mut select = |property: u16| {
            let index = usize::from(property - FIRST_LONGHAND_PROPERTY_ID);
            selected[index / 64] |= 1 << (index % 64);
        };
        // The record being driven again has a live base record. A record
        // without one cannot say what a partial drive would leave standing: it is driven in full.
        let mut old_record_is_unreadable = false;
        let (writing_mode, direction) = match self.computed_group_sets.base_style_record_view(old_style_record) {
            Some(view) => {
                let inherited_box = unsafe {
                    view.payloads[crate::css::computed_value_types::STYLE_GROUP_INDEX_INHERITED_BOX]
                        .cast::<crate::css::computed_values::InheritedBoxValues>()
                        .deref()
                };
                (inherited_box.writing_mode, inherited_box.direction)
            }
            None => {
                debug_assert!(false, "the record being driven again has a live base record");
                old_record_is_unreadable = true;
                (
                    crate::css::css_enums::writing_mode::HORIZONTAL_TB,
                    crate::css::css_enums::direction::LTR,
                )
            }
        };
        for &property in delta.properties() {
            // A delta that moves transition declarations owes the host the transition step, and
            // one that moves animation declarations owes the animation plan; both leave the batch
            // as row effects.
            debug_assert!(
                !property_starts_animation(property)
                    || (owes_a_transition_step && longhand_only_declares_a_css_transition(property))
                    || (owes_an_animation_plan && longhand_declares_a_css_animation(property)),
                "a moved animation longhand leaves the row no effect for the host"
            );
            groups_to_rebuild |= longhand_group_dependency_mask(property);
            select(property);
            let bits = crate::css::style_compute::table_row_bits(property);
            let counterpart = if bits & crate::css::style_compute::LOGICAL_ALIAS_BIT != 0 {
                crate::css::style_compute::map_logical_alias_to_physical(property, writing_mode, direction)
            } else if bits & crate::css::style_compute::PHYSICAL_TO_LOGICAL_BIT != 0 {
                crate::css::style_compute::map_physical_to_logical_alias(property, writing_mode, direction)
            } else {
                property
            };
            if counterpart != property {
                groups_to_rebuild |= longhand_group_dependency_mask(counterpart);
                select(counterpart);
            }
        }
        if groups_to_rebuild & (1 << STYLE_GROUP_INDEX_ANCHOR) != 0 {
            groups_to_rebuild |= 1 << STYLE_GROUP_INDEX_SURROUND;
        }
        // A moved `color` reaches every group holding a value resolved against currentcolor.
        if delta
            .properties()
            .contains(&crate::css::property_metadata::property_id::COLOR)
        {
            // A record holding no table cannot say which values read currentcolor: it is driven
            // in full, as a partial drive of it is.
            let dependencies = self.computed_group_sets.current_color_dependency_mask(target);
            let dependent_properties = self.computed_group_sets.current_color_dependency_properties(target);
            old_record_is_unreadable |= dependencies.is_none() || dependent_properties.is_none();
            groups_to_rebuild |= dependencies.unwrap_or(0);
            // The dependents compute again from their specified values, so the table spells them
            // the way a fresh computation does, not the way an inherited-group swap resolved them.
            let dependent_properties = dependent_properties.unwrap_or_default();
            for (word, &bits) in dependent_properties.iter().enumerate() {
                let mut bits = bits;
                while bits != 0 {
                    let bit = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let property = FIRST_LONGHAND_PROPERTY_ID + (word * 64 + bit) as u16;
                    select(property);
                    if crate::css::style_compute::table_row_bits(property)
                        & crate::css::style_compute::LOGICAL_ALIAS_BIT
                        != 0
                    {
                        select(crate::css::style_compute::map_logical_alias_to_physical(
                            property,
                            writing_mode,
                            direction,
                        ));
                    }
                }
            }
        }
        // A partial delta that reaches the font group reaches every value the font feeds, so it
        // is driven in full, as a partial drive whose driver inputs moved is below.
        let mut driver_input_moved =
            !full_drive && (old_record_is_unreadable || groups_to_rebuild & (1 << STYLE_GROUP_INDEX_FONT) != 0);
        if full_drive || driver_input_moved {
            groups_to_rebuild = (1 << crate::css::table_group_builder::group_index::COUNT) - 1;
        }

        if goal == FontDriveGoal::RootInputs && !full_drive && !driver_input_moved {
            // NB: No font property moved, but borrowing the retained font still needs the
            //     proof that only the named rule flips changed the computation's inputs.
            if let Some(root_inputs) =
                exact_flipped_rules.and_then(|_| self.root_font_inputs_from_record(old_style_record))
            {
                return Ok(ElementAnswer::RootInputs(Some(root_inputs)));
            }
            // Without that proof the root's font is computed, not borrowed.
            driver_input_moved = true;
            groups_to_rebuild = (1 << crate::css::table_group_builder::group_index::COUNT) - 1;
        }

        // Attributes, inherited custom values, conditions, and function definitions can differ
        // between nodes sharing the same own environment and winner state.
        let reads_external_substitution = written.reads_attributes || written.custom_condition_usage != 0;
        let substitution_environment = self.substitution_environment(node, current_environment);
        let mut store = match scratch
            .stores
            .get(&(state, substitution_environment))
            .filter(|_| !reads_external_substitution)
        {
            Some(store) => store.clone(),
            None => {
                let mut substituted = false;
                let inheritance_environment = self.held_inheritance_environment(node, None);
                let store = std::sync::Arc::new(self.cascaded_store_for_state(
                    node,
                    state,
                    None,
                    substitution_environment,
                    inheritance_environment,
                    &inputs,
                    &mut substituted,
                    counters,
                ));
                scratch.store_capacity_bytes += store.capacity_bytes();
                if !reads_external_substitution {
                    scratch.stores.insert((state, substitution_environment), store.clone());
                }
                if substituted {
                    scratch.substituted_states.insert((state, substitution_environment));
                }
                store
            }
        };
        self.note_node_substitution(node, scratch, state, substitution_environment);
        let mut explicitly_inherited_groups = 0;
        let partial = if full_drive || driver_input_moved {
            None
        } else {
            match self.engine_driven_table(
                node,
                old_style_record,
                &store,
                &selected,
                &inputs,
                &mut explicitly_inherited_groups,
                counters,
            )? {
                PartialDrive::Driven(partial) => Some(partial),
                // A partial drive whose driver inputs moved reaches values it did not select, so
                // the record is driven in full and every group is rebuilt.
                PartialDrive::DriverInputMoved => {
                    driver_input_moved = true;
                    groups_to_rebuild = (1 << crate::css::table_group_builder::group_index::COUNT) - 1;
                    None
                }
            }
        };
        let (table, length, longhand_evaluations, font) = match partial {
            Some(partial) => partial,
            None => {
                let subject = self.element_drive_subject(node);
                let driven = self.engine_full_drive(
                    subject,
                    Some(old_style_record),
                    &store,
                    &inputs,
                    &mut scratch.font_drive,
                    goal,
                    has_registered_declarations,
                    &mut explicitly_inherited_groups,
                    counters,
                )?;
                let driven = if let FullDrive::AwaitsRegisteredContext(registered) = driven {
                    let parent_environment = parent.map_or(0, |parent| self.held_custom_property_environment(parent));
                    current_environment = self.engine_custom_property_environment(
                        node,
                        parent_environment,
                        &inputs,
                        Some(registered),
                        counters,
                    )?;
                    environment = Some(current_environment);
                    let substitution_environment = self.substitution_environment(node, current_environment);
                    let mut substituted = false;
                    let inheritance_environment = self.held_inheritance_environment(node, None);
                    let final_store = self.cascaded_store_for_state(
                        node,
                        state,
                        None,
                        substitution_environment,
                        inheritance_environment,
                        &inputs,
                        &mut substituted,
                        counters,
                    );
                    scratch.store_capacity_bytes += final_store.capacity_bytes();
                    store = std::sync::Arc::new(final_store);
                    if substituted {
                        scratch.substituted_states.insert((state, substitution_environment));
                    }
                    self.note_node_substitution(node, scratch, state, substitution_environment);
                    self.engine_full_drive(
                        subject,
                        Some(old_style_record),
                        &store,
                        &inputs,
                        &mut scratch.font_drive,
                        goal,
                        false,
                        &mut explicitly_inherited_groups,
                        counters,
                    )?
                } else {
                    driven
                };
                match driven {
                    FullDrive::Driven(driven) => driven,
                    FullDrive::RootInputs(root_inputs) => return Ok(ElementAnswer::RootInputs(Some(root_inputs))),
                    FullDrive::AwaitsRegisteredContext(_) => {
                        unreachable!("a drive resumed with its registered context has no registered declarations left")
                    }
                }
            }
        };
        // The plan is decided from the longhands this drive computed, before the table goes into
        // the record.
        // A retained record from display:none contributes its base to the drive. Its animation
        // names must be planned from the new base before the host samples the installed record.
        let animation_plan = (owes_an_animation_plan
            || (derived_beneath_a_composition
                && (names_an_animation || self.css_defined_animations.node_runs_a_css_animation(node)))
            || (old_record_is_hidden && table_names_animations(&table)))
        .then(|| {
            self.settled_animation_plan(
                node,
                u8::MAX,
                &table,
                self.animation_name_declaration_scope(node, state),
            )
        });
        let parent_in_display_none_subtree = self
            .tree
            .flat_tree_parent(node)
            .and_then(|parent| self.computed_group_sets.assigned_style_record(parent))
            .and_then(|record| self.computed_group_sets.style_record_view(record.raw()))
            .is_some_and(|view| view.dependency_flags & (1 << 2) != 0);
        let counter_environment = self.table_counter_style_environment_identity(node, &table);
        if full_drive && owes_a_transition_registration == Some(false) {
            // The deferred transition step reads the before-change record after this replacement.
            self.computed_group_sets.pin_style_record(old_style_record.raw());
            self.batch_pinned_compositions.push((node, old_style_record.raw()));
        }
        let assembly = self.computed_group_sets.replace_engine_computed_table(
            node,
            old_style_record,
            old_style_record,
            table,
            groups_to_rebuild,
            &length,
            font.as_ref(),
            parent_in_display_none_subtree,
            environment,
            counter_environment,
            Some((generation, state)),
        );
        self.settle_computed_memory();
        counters.add(
            Counter::ComputedOutputGroupsCanonicalized,
            u64::from(assembly.canonicalized_groups),
        );
        if assembly.group_set_unchanged {
            counters.bump(Counter::ComputedWinnerPropagationStops);
        }
        if let Some(composition) = assembly.pinned_composition {
            self.batch_pinned_compositions.push((node, composition));
        }
        let delta = assembly.delta;
        self.note_engine_computed_record(
            node,
            delta,
            (generation, state),
            delta_property_count,
            longhand_evaluations,
            counters,
        );
        // A record driven in full stands for a cohort keyed by the parent's inherited inputs only
        // when the drive was partial.
        if container_unit_mask == 0 && !driver_input_moved && (!has_registered_declarations || !full_drive) {
            scratch.cohorts.insert(cohort, (delta.1, explicitly_inherited_groups));
            self.remember_warm_record_cohort(cohort, (delta.1, explicitly_inherited_groups));
        }
        if explicitly_inherited_groups != 0 {
            self.nodes_owing_explicit_inheritance
                .insert(node, explicitly_inherited_groups);
        }
        // A partial drive whose driver inputs moved was driven in full instead, so values the
        // delta does not name may have moved too: the host runs the whole step for such a row. A
        // row that owes no step replaces what an earlier row the host never installed owed.
        match owes_a_transition_registration {
            Some(registration_only) => self
                .nodes_owing_a_transition_registration
                .insert(node, registration_only && !driver_input_moved),
            None => self.nodes_owing_a_transition_registration.remove(&node),
        };
        if let Some(plan) = animation_plan {
            self.nodes_owing_animation_definitions.insert((node, u8::MAX), plan);
        }
        if derived_beneath_a_composition {
            self.nodes_owing_an_animation_sample.insert(node);
        }
        Ok(ElementAnswer::Delta(delta))
    }

    /// The animation plan a settled row leaves for the host, decided from one longhand table.
    fn settled_animation_plan(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
        table: &ComputedLonghandTable,
        declaration_scope: Option<tree::TreeScopeID>,
    ) -> animations::SettledAnimationPlan {
        crate::css::style_compute::build_settled_animation_plan(
            table,
            self.element_css_defined_animations(
                node,
                if pseudo_kind == u8::MAX {
                    animations::ELEMENT_ANIMATION_SLOT
                } else {
                    pseudo_kind + 1
                },
            ),
            self.animation_keyframes(),
            declaration_scope,
            self.tree.tree_scope(node),
        )
    }

    /// The same, for a row that takes another node's record whole: the plan is decided from the
    /// longhands that record carries.
    fn settled_animation_plan_from_record(
        &self,
        node: StyleNodeID,
        style_record: computed::FinalStyleRecordID,
        declaration_scope: Option<tree::TreeScopeID>,
    ) -> Option<animations::SettledAnimationPlan> {
        let view = self.computed_group_sets.style_record_view(style_record.raw())?;
        // SAFETY: A record's table outlives the view the assignment below takes it from.
        let table = unsafe { view.longhand_table.as_ref() }?;
        Some(self.settled_animation_plan(node, u8::MAX, table, declaration_scope))
    }

    /// The tree scope the winning `animation-name` declaration was written in, where its
    /// `@keyframes` are looked for first. An author rule's is the one scope its sheet is attached
    /// to; the document's rules, the other origins' and the element's own declarations have none
    /// of their own (`None`). A sheet several scopes adopt matched in the one its priority's
    /// encapsulation context names among the element's contexts.
    fn animation_name_declaration_scope(&self, node: StyleNodeID, state: CascadeStateID) -> Option<tree::TreeScopeID> {
        let winner = self
            .winner_groups
            .winner_in_state(state, crate::css::property_metadata::property_id::ANIMATION_NAME)
            .and_then(|winner| self.winner_groups.resolved_winner(winner))?;
        let scope = match winner.source {
            cascade::WinnerSource::Rule(rule) => {
                let sheet = self.program.rule_sheet(rule);
                if self.program.sheet_origin(sheet) != crate::css::cascaded_properties::CascadeOrigin::Author {
                    return None;
                }
                let sheet_scopes = self.program.sheet_scopes(sheet);
                match sheet_scopes.as_slice() {
                    &[scope] => scope,
                    _ => {
                        let scope = winner
                            .priority
                            .author_context_depth()
                            .zip(self.author_contexts(node))
                            .and_then(|(depth, contexts)| contexts.get(depth as usize).copied())
                            .filter(|scope| sheet_scopes.contains(scope));
                        // The rule matched through one of the element's encapsulation contexts,
                        // and its priority names which.
                        debug_assert!(scope.is_some(), "an adopted rule matched outside its scopes");
                        scope?
                    }
                }
            }
            cascade::WinnerSource::Element(_) => return None,
            // A replayed exact cascade names no declaration; its names resolve from the element's
            // own scope outward.
            cascade::WinnerSource::ExactCascade => return None,
        };
        (scope != tree::TreeScopeID::DOCUMENT).then_some(scope)
    }

    /// The plan the engine-computed record the host installs for this node leaves to be applied after the batch, as
    /// the host's copy of it took it, so that exactly one application drains it.
    pub(crate) fn take_settled_animation_plan_taken_by_host(&mut self, node: StyleNodeID, pseudo_kind: u8) {
        self.nodes_owing_animation_definitions
            .take_taken_by_host(&(node, pseudo_kind));
    }

    /// A copy of the plan a row left for `node`, or for its pseudo-element of `pseudo_kind`, for the host to apply.
    pub(crate) fn settled_animation_plan_for_host(
        &self,
        node: StyleNodeID,
        pseudo_kind: u8,
    ) -> Option<animations::SettledAnimationPlan> {
        let plan = self.nodes_owing_animation_definitions.get(&(node, pseudo_kind))?;
        // The host takes a reference to each set a definition names, so it must be one a scope
        // still publishes.
        assert!(
            plan.definitions()
                .iter()
                .all(|definition| definition.keyframe_set.is_null()
                    || self
                        .animation_keyframes
                        .description(definition.keyframe_set as usize)
                        .is_some()),
            "a settled animation plan names a keyframe set no scope publishes"
        );
        Some(plan.clone())
    }

    /// Account for a record the engine derived and leave its commitment to C++'s acknowledgement.
    /// Move a node's record to the environment C++ refreshed its inherited custom-property data
    /// to, without recomputing anything: what an inherited-custom-properties reaction C++ settled
    /// by refreshing the data alone publishes. The new record's identity, or nothing when the node
    /// holds no base record to move.
    /// The style groups an engine-computed record read straight from the parent through an
    /// explicit `inherit`, taken with the answer so that exactly one application marks the parent.
    pub(crate) fn take_explicit_inheritance_debt(&mut self, node: StyleNodeID) -> u32 {
        let debt = self.nodes_owing_explicit_inheritance.remove(&node).unwrap_or(0);
        if debt != 0
            && let Some(parent) = self.tree.parent(node)
        {
            self.children_explicitly_inherit_marks.insert(parent);
        }
        debt
    }

    /// Give back the debts a published row took that the host did not settle, as the node owed
    /// them before: the host will settle them with a later row of the node.
    pub(crate) fn restore_row_debts(&mut self, node: StyleNodeID, explicit_inheritance_debt: u32, row_effect_debt: u8) {
        if explicit_inheritance_debt != 0 {
            *self.nodes_owing_explicit_inheritance.entry(node).or_default() |= explicit_inheritance_debt;
        }
        match row_effect_debt & 0b11 {
            1 => {
                self.nodes_owing_a_transition_registration.entry(node).or_insert(true);
            }
            2 => {
                self.nodes_owing_a_transition_registration.entry(node).or_insert(false);
            }
            _ => {}
        }
        if row_effect_debt & OWES_AN_ANIMATION_SAMPLE != 0 {
            self.nodes_owing_an_animation_sample.insert(node);
        }
    }

    /// The host marked `node`'s children as explicitly inheriting a non-inherited property.
    pub(crate) fn note_children_explicitly_inherit(&mut self, node: StyleNodeID) {
        self.children_explicitly_inherit_marks.insert(node);
    }

    /// What the engine-computed record the host is about to install for this node leaves to be
    /// applied after the batch, taking the transition debt with the answer so that exactly one
    /// application drains it: the low two bits are the transition step - 0 nothing, 1 moved
    /// declarations that leave the host nothing to do, 2 the whole step - and `OWES_AN_ANIMATION_PLAN` says the row also left
    /// an animation plan, which the host then takes for itself.
    ///
    /// Both effects are answered in one call because every installed row asks, and asking twice
    /// costs more than the answer.
    pub(crate) fn take_settled_row_effect_debt(&mut self, node: StyleNodeID) -> u8 {
        let transition = match self.nodes_owing_a_transition_registration.remove(&node) {
            Some(true) => 1,
            Some(false) => 2,
            None => 0,
        };
        let plan = match self
            .nodes_owing_animation_definitions
            .keys()
            .any(|(owner, _)| *owner == node)
        {
            true => OWES_AN_ANIMATION_PLAN,
            false => 0,
        };
        let sample = match self.nodes_owing_an_animation_sample.remove(&node) {
            true => OWES_AN_ANIMATION_SAMPLE,
            false => 0,
        };
        transition | plan | sample
    }

    pub(crate) fn republish_record_environment(&mut self, node: StyleNodeID, environment: u64) -> Option<u64> {
        let (_, new_style_record) = self
            .computed_group_sets
            .republish_engine_record_with_environment(node, environment)?;
        Some(new_style_record.raw())
    }

    /// Note whether the node's record was computed with a substituted winner, for C++ to record
    /// the node as a reader of custom properties when it installs the record. The step records
    /// the fact; the boundary that installs the record applies it.
    fn note_node_substitution(
        &self,
        node: StyleNodeID,
        scratch: &mut EngineComputedRecordScratch,
        state: CascadeStateID,
        environment: u64,
    ) {
        let substituted = scratch.substituted_states.contains(&(state, environment));
        scratch.noted_substitution = Some(substituted);
        scratch.substitution_effects.push((node, substituted));
    }

    /// Write the substituted-record facts the step decided. The set is a retained per-node fact
    /// C++ reads when it installs a record, so the step decides it once and the record's
    /// installation boundary writes it once, rather than the step reading its own writes back.
    fn apply_substitution_effects(&mut self, scratch: &mut EngineComputedRecordScratch) {
        for (node, uses_substitution) in scratch.substitution_effects.drain(..) {
            if uses_substitution {
                self.nodes_with_substituted_records.insert(node);
            } else {
                self.nodes_with_substituted_records.remove(&node);
            }
        }
    }

    fn note_engine_computed_record(
        &mut self,
        node: StyleNodeID,
        delta: (computed::FinalStyleRecordID, computed::FinalStyleRecordID),
        cascade_state: (u64, CascadeStateID),
        delta_property_count: u64,
        longhand_evaluations: u32,
        counters: &mut Counters,
    ) {
        counters.add(Counter::CascadeWinnerDeltaProperties, delta_property_count);
        counters.add(Counter::ComputedWinnerDeltaPropertiesConsumed, delta_property_count);
        counters.bump(Counter::EngineComputedRecordDeltas);
        // Ordinary candidates already carry their final container facts. Effect-bearing rows
        // refresh this projection again when the host completes their composition.
        self.set_element_container_query_inputs(node, delta.1.raw());
        self.computed_group_sets.set_sampled_composition_identity(node, 0);
        // An effect on a custom property holds no overlay, and the host samples it again over the
        // new base without republishing values that stand. The element keeps the environment its
        // animations sampled into while that is composed over the new base's, so a child derived
        // after it inherits the animated values.
        if let Some(base_environment) = self.computed_group_sets.custom_property_environment_identity(node) {
            let environment = self.substitution_environment(node, base_environment);
            if environment != base_environment {
                self.computed_group_sets
                    .set_node_custom_property_environment(node, environment);
            }
        }
        self.engine_computed_records_pending
            .entry(node)
            .or_default()
            .push(PendingEngineComputedRecord {
                node,
                pseudo_kind: u8::MAX,
                old_style_record: delta.0,
                new_style_record: delta.1,
                cascade_state: Some(cascade_state),
                longhand_evaluations,
            });
    }

    /// C++ installed the record the engine derived for `node`: the winner state it was computed
    /// from becomes the node's cascade state, and the answer counts as consumed.
    /// Drop the batch's pins on the compositions a node's row derived beneath: the host has the
    /// record now, and the composition it named is nobody's.
    fn drop_pinned_compositions(&mut self, node: StyleNodeID) {
        let mut index = 0;
        while index < self.batch_pinned_compositions.len() {
            if self.batch_pinned_compositions[index].0 != node {
                index += 1;
                continue;
            }
            let (_, composition) = self.batch_pinned_compositions.swap_remove(index);
            self.computed_group_sets.unpin_style_record(composition);
        }
    }

    /// The host is sampling an overlay after installing a base this batch drove from winners.
    /// The old composition remains pinned until acknowledgement, so a post-compute adjustment
    /// has already received the fresh base it would otherwise request through another reaction.
    pub(super) fn animation_base_was_just_driven(&self, style_record: u64) -> bool {
        self.batch_pinned_compositions.iter().any(|(node, _)| {
            self.computed_group_sets
                .assigned_style_record(*node)
                .is_some_and(|record| record.raw() == style_record)
                && self.engine_computed_records_pending.get(node).is_some_and(|pending| {
                    pending.iter().any(|record| {
                        record.pseudo_kind == u8::MAX
                            && record.new_style_record.raw() == style_record
                            && record.longhand_evaluations != 0
                    })
                })
        })
    }

    pub(crate) fn acknowledge_engine_computed_record(&mut self, node: StyleNodeID, counters: &mut Counters) {
        if let Some(pending_records) = self.engine_computed_records_pending.remove(&node) {
            for pending in pending_records {
                let target = computed::ComputedStyleTarget::new(node, pending.pseudo_kind);
                // A pseudo-element settled as gone is removed now that C++ has cleared its style.
                if pending.pseudo_kind != u8::MAX && pending.new_style_record == computed::FinalStyleRecordID::NONE {
                    self.remove_computed_pseudo(node, pending.pseudo_kind, counters);
                    continue;
                }
                self.computed_group_sets.take_pending_cascade_state(target);
                if let Some(cascade_state) = pending.cascade_state {
                    self.computed_group_sets.bind_cascade_state(target, cascade_state);
                }
                // A pseudo-element's record was computed from this very state, as the retained
                // cascade would have observed had C++ computed it.
                if pending.pseudo_kind != u8::MAX {
                    self.computed_group_sets
                        .observe_pseudo_retained_cascade_state(target, pending.cascade_state);
                }
                counters.add(
                    Counter::EngineComputedLonghandEvaluations,
                    u64::from(pending.longhand_evaluations),
                );
            }
        }
        if let Some(record) = self.computed_group_sets.assigned_style_record(node) {
            self.set_element_container_query_inputs(node, record.raw());
        }
        self.drop_pinned_compositions(node);
        self.mark_published_answer_observed(node);
    }

    pub(super) fn forget_engine_computed_record(&mut self, target: computed::ComputedStyleTarget) {
        let Some(pending_records) = self.engine_computed_records_pending.get_mut(&target.node()) else {
            return;
        };
        pending_records.retain(|pending| pending.pseudo_kind != target.pseudo_kind());
        if pending_records.is_empty() {
            self.engine_computed_records_pending.remove(&target.node());
        }
    }

    /// The transaction's outputs are gone: every derived record C++ did not install goes back to
    /// the record the node held, unless a publication has moved the node on since.
    pub(super) fn discard_engine_computed_records(&mut self, counters: &mut Counters) {
        for pending in std::mem::take(&mut self.engine_computed_records_pending)
            .into_values()
            .flatten()
        {
            if pending.pseudo_kind != u8::MAX {
                self.revert_engine_computed_pseudo_record(&pending, counters);
                continue;
            }
            self.computed_group_sets.revert_engine_computed_record(
                pending.node,
                pending.new_style_record,
                pending.old_style_record,
            );
            // The container-query projection took the derived record when it was noted; it
            // follows the node back to the record it holds now, or to none.
            match self.computed_group_sets.assigned_style_record(pending.node) {
                Some(record) => self.set_element_container_query_inputs(pending.node, record.raw()),
                None => self.container_query_inputs.clear(pending.node),
            }
        }
        for (_, composition) in std::mem::take(&mut self.batch_pinned_compositions) {
            self.computed_group_sets.unpin_style_record(composition);
        }
    }

    /// Derive a node's first record: every winner of its state driven through every phase, every
    /// group built against the parent's payloads, and the record published the way a C++ first
    /// computation publishes it, with the parent's custom-property environment. A node alike in
    /// everything a first record is computed from takes the record an earlier node got, whether
    /// in this flush or one before it.
    fn engine_cold_record(
        &mut self,
        node: StyleNodeID,
        cascade_state: (u64, CascadeStateID),
        scratch: &mut EngineComputedRecordScratch,
        goal: FontDriveGoal,
        counters: &mut Counters,
    ) -> Drive<ElementAnswer> {
        // A first record whose winners declare CSS animations owes the host the plan that starts
        // them, the way a warm row does: which animations to create or keep, their timing and the
        // keyframe sets they run are decided here, and the host only creates the objects. An
        // element that already runs CSS animations without a record owes the complete plan too,
        // which keeps, retimes or cancels them; the host samples them over the installed base.
        let owes_an_animation_plan = self
            .winner_groups
            .semantic_delta_properties(None, cascade_state.1)
            .any(longhand_declares_a_css_animation)
            || self.css_defined_animations.node_runs_a_css_animation(node);
        let delta = match self.engine_cold_record_impl(node, cascade_state, scratch, goal, counters)? {
            ElementAnswer::Delta(delta) => delta,
            probe @ ElementAnswer::RootInputs(_) => return Ok(probe),
        };
        if owes_an_animation_plan {
            // The plan is decided from the record the row installs, which carries the longhands the
            // drive computed: every record the engine assembles holds its table.
            let declaration_scope = self.animation_name_declaration_scope(node, cascade_state.1);
            let plan = self.settled_animation_plan_from_record(node, delta.1, declaration_scope);
            debug_assert!(plan.is_some(), "an engine-assembled record without its longhand table");
            if let Some(plan) = plan {
                self.nodes_owing_animation_definitions.insert((node, u8::MAX), plan);
            }
        }
        if self.computed_group_sets.adjustment_facts(node) & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0 {
            self.nodes_owing_an_animation_sample.insert(node);
        }
        Ok(ElementAnswer::Delta(delta))
    }

    /// With no winning `animation-name`, timing and keyword longhands describe no CSS animation
    /// to create or retime. The drive still computes their values into the record.
    fn state_has_no_animation_name(&self, state: CascadeStateID) -> bool {
        self.winner_groups
            .winner_in_state(state, crate::css::property_metadata::property_id::ANIMATION_NAME)
            .and_then(|winner| self.winner_groups.resolved_winner(winner))
            .is_none()
    }

    /// A node's custom-property environment as this flush leaves it. A row this flush settles
    /// holds its new environment already, but an ancestor between it and a first record may be one
    /// the host refreshes only once the batch applies, so the first record would resolve over the
    /// environment that ancestor held before. Each ancestor's is resolved again over its parent's
    /// as it is now, and kept for the rows after it. A node composing animated custom properties
    /// keeps the environment its sample published.
    fn current_custom_property_environment(
        &mut self,
        node: StyleNodeID,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        scratch: &mut EngineComputedRecordScratch,
    ) -> Result<u64, Suspension> {
        if let Some(&current) = scratch.current_custom_property_environments.get(&node) {
            return Ok(current);
        }
        let held = self.held_custom_property_environment(node);
        let animates =
            self.computed_group_sets.adjustment_facts(node) & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0
                || self.computed_group_sets.node_has_animation_overlay(node);
        let current = match self.record_inheritance_parent(node) {
            Some(parent) if !animates => {
                let parent_environment = self.current_custom_property_environment(parent, inputs, scratch)?;
                let parent_moved =
                    self.computed_group_sets.custom_property_environment_identity(parent) != Some(parent_environment);
                self.environment_over_current_parent(node, parent_environment, parent_moved, inputs)?
                    .unwrap_or(held)
            }
            _ => held,
        };
        scratch.current_custom_property_environments.insert(node, current);
        Ok(current)
    }

    /// The custom-property environment a node with a record holds. Every record is assigned with
    /// its environment, so a node the drive reads as an inheritance parent always holds one.
    fn held_custom_property_environment(&self, node: StyleNodeID) -> u64 {
        let environment = self.computed_group_sets.custom_property_environment_identity(node);
        debug_assert!(environment.is_some(), "an inheritance parent without an environment");
        environment.unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    fn engine_cold_record_impl(
        &mut self,
        node: StyleNodeID,
        cascade_state: (u64, CascadeStateID),
        scratch: &mut EngineComputedRecordScratch,
        goal: FontDriveGoal,
        counters: &mut Counters,
    ) -> Drive<ElementAnswer> {
        debug_assert!(self.computes_records(), "only a hosted engine drives records");
        let target = computed::ComputedStyleTarget::new(node, u8::MAX);
        let (_, state) = cascade_state;
        let mut inputs = self.document_style_computation_inputs;
        if let Some((root, root_inputs)) = scratch.root_element_inputs
            && root == node
        {
            root_inputs.apply_to(&mut inputs);
        }
        let facts = self.computed_group_sets.adjustment_facts(node);
        let mut explicitly_inherited_groups = 0;
        // An element without a styled inheritance parent, the document element among them,
        // inherits from the initial values.
        let parent = self.record_inheritance_parent(node);
        let parent_record = parent.and_then(|parent| {
            self.computed_group_sets
                .sampled_composition_identity(parent)
                .and_then(computed::FinalStyleRecordID::from_raw)
                .or_else(|| self.computed_group_sets.assigned_style_record(parent))
        });
        // The document element's environment is its own, which is nothing without declarations;
        // any other node's is its declarations resolved over the parent's.
        let parent_environment = parent
            .map_or(Ok(0), |parent| {
                self.current_custom_property_environment(parent, &inputs, scratch)
            })
            .map_err(Unanswered::Suspended)?;
        let has_registered_declarations = self.declares_registered_custom_property(node, None, &inputs);
        let pseudo_styles = self.pseudo_style_mask_or_rematch(node, counters);
        let written = self.state_written_facts(node, state);
        let cache_key = (written.container_relative_length_unit_mask == 0)
            .then_some(())
            .and(parent)
            .zip(parent_record)
            .filter(|_| {
                !(scratch.targeted_record_demand && facts & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0)
            })
            .and_then(|(parent, parent_record)| self.cold_record_parent(node, parent, parent_record, state))
            .map(|parent| ColdRecordKey {
                monospace_recascaded_font_size: self.monospace_cohort_key(node, state),
                element_reads: self.element_reads(node, None, state, parent_environment),
                parent,
                previous_style_record: 0,
                generation: cascade_state.0,
                state,
                facts,
                pseudo_styles,
                environment: parent_environment,
                font_environment_generation: inputs.font_environment_generation,
                root_font_inputs: RootFontInputs::from_document(&inputs),
            });
        let delta_property_count = self.winner_groups.winner_count_in_state(state) as u64;
        if !(scratch.targeted_record_demand && facts & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0)
            && !self.node_declares_custom_properties(node)
            && let Some(delta) = self.assign_cached_cold_record(
                node,
                target,
                cascade_state,
                cache_key,
                parent,
                state,
                computed::FinalStyleRecordID::NONE,
                delta_property_count,
                scratch,
                counters,
            )
        {
            return Ok(ElementAnswer::Delta(delta));
        }
        let provisional_registered =
            has_registered_declarations.then(|| self.provisional_registered_value_context(parent_record, &inputs));
        let mut environment = self.engine_custom_property_environment(
            node,
            parent_environment,
            &inputs,
            provisional_registered,
            counters,
        )?;
        // A store with substituted values is the environment's as well as the state's, and one
        // that reads attributes or custom conditions is the element's alone.
        let reads_external_substitution = written.reads_attributes || written.custom_condition_usage != 0;
        let mut store = match scratch
            .stores
            .get(&(state, environment))
            .filter(|_| !reads_external_substitution)
        {
            Some(store) => store.clone(),
            None => {
                let mut substituted = false;
                let store = self.cascaded_store_for_state(
                    node,
                    state,
                    None,
                    environment,
                    Some(parent_environment),
                    &inputs,
                    &mut substituted,
                    counters,
                );
                let store = std::sync::Arc::new(store);
                scratch.store_capacity_bytes += store.capacity_bytes();
                if !reads_external_substitution {
                    scratch.stores.insert((state, environment), store.clone());
                }
                if substituted {
                    scratch.substituted_states.insert((state, environment));
                }
                store
            }
        };
        self.note_node_substitution(node, scratch, state, environment);
        let cache_key = (!has_registered_declarations
            && !reads_external_substitution
            && written.container_relative_length_unit_mask == 0)
            .then_some(())
            .and(parent)
            .zip(parent_record)
            .and_then(|(parent, parent_record)| self.cold_record_parent(node, parent, parent_record, state))
            .map(|parent| ColdRecordKey {
                monospace_recascaded_font_size: self.monospace_cohort_key(node, state),
                element_reads: self.element_reads(node, None, state, environment),
                parent,
                previous_style_record: 0,
                generation: cascade_state.0,
                state,
                facts,
                pseudo_styles,
                environment,
                font_environment_generation: inputs.font_environment_generation,
                root_font_inputs: RootFontInputs::from_document(&inputs),
            });
        if let Some(delta) = self.assign_cached_cold_record(
            node,
            target,
            cascade_state,
            cache_key,
            parent,
            state,
            computed::FinalStyleRecordID::NONE,
            delta_property_count,
            scratch,
            counters,
        ) {
            return Ok(ElementAnswer::Delta(delta));
        }
        let donor = cache_key.and_then(|key| {
            let donor_key = ColdRecordDonorKey {
                parent: key.parent,
                generation: key.generation,
                property_shape_hash: self.winner_groups.property_shape_hash(state),
                facts: key.facts,
                pseudo_styles: key.pseudo_styles,
                environment: key.environment,
                font_environment_generation: key.font_environment_generation,
                root_font_inputs: key.root_font_inputs,
                element_reads: key.element_reads,
            };
            self.engine_cold_record_donors
                .get(&donor_key)?
                .iter()
                .rev()
                .copied()
                .filter(|donor| {
                    self.winner_groups.property_shapes_are_equal(donor.state, state)
                        && self
                            .computed_group_sets
                            .final_style_record_is_live(donor.record.record.raw())
                        && self.record_counter_environment_is_current(node, donor.record.record)
                })
                .min_by_key(|donor| {
                    self.winner_groups
                        .semantic_delta_properties(Some(donor.state), state)
                        .count()
                })
        });
        if !scratch.font_drive.is_pending()
            && let Some(donor) = donor
        {
            let donor_delta = self.winner_groups.semantic_delta(Some(donor.state), state);
            if let Some((groups_to_rebuild, selected)) =
                self.cold_record_donor_selection(donor, donor_delta.properties())
                && let Ok(PartialDrive::Driven((table, length, longhand_evaluations, _))) = self.engine_driven_table(
                    node,
                    donor.record.record,
                    &store,
                    &selected,
                    &inputs,
                    &mut explicitly_inherited_groups,
                    counters,
                )
            {
                let parent_in_display_none_subtree = parent_record
                    .and_then(|record| self.computed_group_sets.style_record_view(record.raw()))
                    .is_some_and(|view| view.dependency_flags & (1 << 2) != 0);
                let counter_environment = self.table_counter_style_environment_identity(node, &table);
                let assembly = self.computed_group_sets.replace_engine_computed_table(
                    node,
                    donor.record.record,
                    computed::FinalStyleRecordID::NONE,
                    table,
                    groups_to_rebuild,
                    &length,
                    None,
                    parent_in_display_none_subtree,
                    Some(environment),
                    counter_environment,
                    Some(cascade_state),
                );
                self.computed_group_sets
                    .set_pending_cascade_state(target, cascade_state);
                self.settle_computed_memory();
                self.note_node_substitution(node, scratch, state, environment);
                self.note_engine_computed_record(
                    node,
                    assembly.delta,
                    cascade_state,
                    delta_property_count,
                    longhand_evaluations,
                    counters,
                );
                // The key names the parent's environment: a record whose own declarations
                // resolved another is no answer for an element declaring none.
                if let Some(cache_key) = cache_key.filter(|key| key.environment == environment) {
                    let record = ColdRecord {
                        record: assembly.delta.1,
                        swap_eligible: self.computed_group_sets.node_inherited_group_swap_eligible(node),
                        explicitly_inherited_groups,
                    };
                    scratch.cold_cohorts.insert(cache_key, record);
                    self.remember_cold_record(cache_key, record);
                }
                if explicitly_inherited_groups != 0 {
                    self.nodes_owing_explicit_inheritance
                        .insert(node, explicitly_inherited_groups);
                }
                return Ok(ElementAnswer::Delta(assembly.delta));
            }
        }
        let subject = DriveSubject {
            target,
            parent,
            // A scoped demand computes the base style of a cold animation target.
            // The host samples its animation over that base after installation.
            facts: if scratch.targeted_record_demand {
                facts & !bridge::element_adjustment_fact::HAS_ANIMATIONS
            } else {
                facts
            },
            highlight_parent: None,
        };
        let driven = self.engine_full_drive(
            subject,
            None,
            &store,
            &inputs,
            &mut scratch.font_drive,
            goal,
            has_registered_declarations,
            &mut explicitly_inherited_groups,
            counters,
        )?;
        let driven = if let FullDrive::AwaitsRegisteredContext(registered) = driven {
            environment =
                self.engine_custom_property_environment(node, parent_environment, &inputs, Some(registered), counters)?;
            let mut substituted = false;
            let final_store = self.cascaded_store_for_state(
                node,
                state,
                None,
                environment,
                Some(parent_environment),
                &inputs,
                &mut substituted,
                counters,
            );
            scratch.store_capacity_bytes += final_store.capacity_bytes();
            store = std::sync::Arc::new(final_store);
            if substituted {
                scratch.substituted_states.insert((state, environment));
            }
            self.note_node_substitution(node, scratch, state, environment);
            self.engine_full_drive(
                subject,
                None,
                &store,
                &inputs,
                &mut scratch.font_drive,
                goal,
                false,
                &mut explicitly_inherited_groups,
                counters,
            )?
        } else {
            driven
        };
        let (table, length, longhand_evaluations, font) = match driven {
            FullDrive::Driven(driven) => driven,
            FullDrive::RootInputs(root_inputs) => return Ok(ElementAnswer::RootInputs(Some(root_inputs))),
            FullDrive::AwaitsRegisteredContext(_) => {
                unreachable!("a drive resumed with its registered context has no registered declarations left")
            }
        };
        let font = font.expect("a full drive resolves the font");
        let (new_style_record, swap_eligible) = self.assemble_and_publish_engine_record(
            target,
            true,
            parent_record,
            table,
            &length,
            &font,
            environment,
            pseudo_styles,
            0,
            Some(cascade_state),
            counters,
        );
        let delta = (computed::FinalStyleRecordID::NONE, new_style_record);
        // The publication itself kept the record for later transactions; alike elements in this
        // one take it from the cohort. The key names the parent's environment, so a record whose
        // own declarations resolved another is kept for no one.
        if let Some(cache_key) = cache_key.filter(|key| key.environment == environment) {
            let record = ColdRecord {
                record: delta.1,
                swap_eligible,
                explicitly_inherited_groups,
            };
            scratch.cold_cohorts.insert(cache_key, record);
            self.remember_cold_record(cache_key, record);
        }
        if explicitly_inherited_groups != 0 {
            self.nodes_owing_explicit_inheritance
                .insert(node, explicitly_inherited_groups);
        }
        self.note_engine_computed_record(
            node,
            delta,
            cascade_state,
            delta_property_count,
            longhand_evaluations,
            counters,
        );
        Ok(ElementAnswer::Delta(delta))
    }

    #[allow(clippy::too_many_arguments)]
    fn assign_cached_cold_record(
        &mut self,
        node: StyleNodeID,
        target: computed::ComputedStyleTarget,
        cascade_state: (u64, CascadeStateID),
        cache_key: Option<ColdRecordKey>,
        parent: Option<StyleNodeID>,
        state: CascadeStateID,
        old_style_record: computed::FinalStyleRecordID,
        delta_property_count: u64,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> Option<(computed::FinalStyleRecordID, computed::FinalStyleRecordID)> {
        let own_groups = self.state_owned_inherited_groups(state);
        let derived_under_parent = |engine: &Self, record: ColdRecord| {
            parent.is_some_and(|parent| {
                engine
                    .computed_group_sets
                    .final_style_record_is_live(record.record.raw())
                    && engine.record_counter_environment_is_current(node, record.record)
                    && engine.computed_group_sets.style_record_inherits_from_node(
                        record.record.raw(),
                        parent,
                        own_groups,
                    )
            })
        };
        let (
            ColdRecord {
                record,
                swap_eligible,
                explicitly_inherited_groups,
            },
            from_cache,
        ) = cache_key.and_then(|cache_key| {
            scratch
                .cold_cohorts
                .get(&cache_key)
                .copied()
                .filter(|&record| derived_under_parent(self, record))
                .map(|record| (record, false))
                .or_else(|| {
                    let record = *self.engine_cold_record_cache.get(&cache_key)?;
                    derived_under_parent(self, record).then_some((record, true))
                })
        })?;
        // A record with an animation overlay is not shared; the row drives its own. Nor is one
        // declaring transitions for a row that had a record: the row owes the host a transition
        // step from it. A first record starts none, so every first record alike shares. Missing
        // the cache declines nothing.
        if !self.record_is_shareable(record, old_style_record == computed::FinalStyleRecordID::NONE) {
            return None;
        }
        self.computed_group_sets
            .set_pending_cascade_state(target, cascade_state);
        let publication = self.assign_shared_style_record(
            target,
            record.raw(),
            computed::ENGINE_INHERITED_GROUP_COUNT,
            swap_eligible,
            counters,
        );
        let delta = (old_style_record, publication.style_record_identity);
        self.note_engine_computed_record(node, delta, cascade_state, delta_property_count, 0, counters);
        counters.bump(if from_cache {
            Counter::EngineComputedRecordSharedHits
        } else {
            Counter::EngineComputedRecordCohortHits
        });
        if explicitly_inherited_groups != 0 {
            self.nodes_owing_explicit_inheritance
                .insert(node, explicitly_inherited_groups);
        }
        Some(delta)
    }

    /// What a record this node publishes must name, when what it computed reads the registry.
    /// Zero when it reads none, which is what the record carries for every other element.
    fn counter_style_environment_identity_for(&self, node: StyleNodeID) -> u64 {
        // A counter style name is looked for in the element's own tree scope and then in the
        // scope that one hangs off, which is where a shadow tree taking the document's style
        // sheets finds it. C++ stamps the record with the identity of the scope that answered,
        // so a lookup that stopped at the element's own tree scope named a different registry
        // from the one the record was computed against, and no record ever matched.
        self.counter_style_environment_identities
            .get(&self.tree.tree_scope(node))
            .or_else(|| {
                self.counter_style_environment_identities
                    .get(&crate::css::style::tree::TreeScopeID::DOCUMENT)
            })
            .copied()
            .unwrap_or(0)
    }

    fn table_counter_style_environment_identity(&self, node: StyleNodeID, table: &ComputedLonghandTable) -> u64 {
        use crate::css::property_metadata::property_id as prop;
        let current = self.counter_style_environment_identity_for(node);
        if current == 0 {
            return 0;
        }
        let names_one = [prop::CONTENT, prop::LIST_STYLE_TYPE].into_iter().any(|property| {
            let value = table.effective_value(None, property, true).value;
            !value.is_null() && value_reads_counter_style_environment(unsafe { &*value.cast::<StyleValueData>() })
        });
        if names_one { current } else { 0 }
    }

    /// Reuse preserves publication metadata, so a cached record must name the registry this
    /// subject would publish. This also separates identical declarations in different scopes.
    fn record_counter_environment_is_current(&self, node: StyleNodeID, record: computed::FinalStyleRecordID) -> bool {
        self.computed_group_sets
            .style_record_view(record.raw())
            .is_some_and(|view| {
                view.counter_style_environment_identity == 0
                    || view.counter_style_environment_identity == self.counter_style_environment_identity_for(node)
            })
    }

    /// Whether the record holds a value resolved against the viewport, its own or its font's.
    /// `merge_dependency_flags` puts both in the record's publication flags.
    pub(super) fn record_reads_the_viewport(&self, record: computed::FinalStyleRecordID) -> bool {
        const DEPENDS_ON_VIEWPORT_METRICS: u8 = 1;
        const FONT_METRICS_DEPEND_ON_VIEWPORT_METRICS: u8 = 1 << 1;
        self.computed_group_sets
            .style_record_view(record.raw())
            .is_some_and(|view| {
                view.dependency_flags & (DEPENDS_ON_VIEWPORT_METRICS | FONT_METRICS_DEPEND_ON_VIEWPORT_METRICS) != 0
            })
    }

    /// Whether the cascade state's winning `font-family` is monospace, which is what makes C++
    /// recascade the element's font-size against a 13px default instead of the 16px one. It is the
    /// element's own declaration that decides this, as it is in `cascaded_properties`, not what it
    /// inherits.
    pub(super) fn font_family_winner_is_monospace(&self, state: CascadeStateID) -> bool {
        self.winner_groups
            .winner_in_state(state, crate::css::property_metadata::property_id::FONT_FAMILY)
            .and_then(|winner| self.winner_groups.resolved_winner(winner))
            .is_some_and(|winner| match self.specified_values.value(winner.key.value) {
                Lookup::Known(data) => crate::css::style_compute::font_family_is_monospace(data),
                _ => true,
            })
    }

    /// Whether any of a state's longhand winners is written with `attr()`.
    pub(super) fn state_reads_attributes(&self, node: StyleNodeID, state: CascadeStateID) -> bool {
        self.state_written_facts(node, state).reads_attributes
    }

    /// What the per-state queries below answer, in one walk of the state's winners.
    pub(super) fn state_written_facts(&self, node: StyleNodeID, state: CascadeStateID) -> StateWrittenFacts {
        let mut facts = StateWrittenFacts::default();
        // Most states hold only winners of rules written without anything these facts are about,
        // which the state's rules answer without walking its winners.
        if !self.winner_groups.state_has_element_winners(state)
            && self
                .winner_groups
                .state_winning_rules(state)
                .iter()
                .all(|&rule| self.program.written_values_add_no_state_facts(rule))
        {
            return facts;
        }
        for winner in self.winner_groups.winners_in_state(state) {
            let Some(winner) = self.winner_groups.resolved_winner(winner) else {
                continue;
            };
            if let WinnerSource::Rule(rule) = winner.source
                && self.program.written_values_add_no_state_facts(rule)
            {
                continue;
            }
            let Some((_, _, checks)) = self.written_winner_value(node, &winner) else {
                continue;
            };
            facts.container_relative_length_unit_mask |= checks.container_relative_length_unit_mask;
            facts.has_written_tree_counting |= checks.uses_tree_counting_function;
            facts.may_read_element_random |= match checks.substitution {
                WrittenSubstitution::None => checks.reads_element_random,
                WrittenSubstitution::Unresolved | WrittenSubstitution::PendingShorthand => true,
            };
            if winner.property >= crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID
                && checks.substitution != WrittenSubstitution::None
            {
                facts.has_substitutions = true;
                facts.reads_attributes |= checks.reads_attributes;
                facts.custom_condition_usage |= checks.custom_condition_usage;
            }
        }
        facts
    }

    /// The element whose attributes `attr()` reads for a node's record or one of its pseudo-elements':
    /// an element standing for its shadow host's pseudo-element is computed as that pseudo-element,
    /// so it reads the host's, as C++ does for all but its ::first-letter.
    pub(super) fn substitution_attribute_element(&self, node: StyleNodeID, pseudo_kind: Option<u8>) -> StyleNodeID {
        if pseudo_kind == Some(pseudo_kind::FIRST_LETTER)
            || self.computed_group_sets.associated_pseudo_kind(node).is_none()
        {
            return node;
        }
        self.tree.shadow_host_of(node).unwrap_or(node)
    }

    /// What a record `state` computes under the custom-property `environment` reads of `node`, or of its
    /// pseudo-element `pseudo_kind`, beyond its winners and its parent. A substituted winner reads what its value
    /// under `environment` reads, and one `environment` has not substituted yet may read anything: elements whose
    /// winners substitute alike share a record, wherever they sit and whatever else their attributes hold.
    pub(super) fn element_reads(
        &self,
        node: StyleNodeID,
        pseudo_kind: Option<u8>,
        state: CascadeStateID,
        environment: u64,
    ) -> ElementReads {
        use std::hash::{Hash, Hasher};
        let written = self.state_written_facts(node, state);
        // Random bases and container units are the element's own: a record reading either is no one else's.
        let mut reads_element = false;
        let mut reads_tree_counting = written.has_written_tree_counting;
        if written.may_read_element_random || written.has_substitutions {
            for winner in self.winner_groups.winners_in_state(state) {
                let Some(winner) = self.winner_groups.resolved_winner(winner) else {
                    continue;
                };
                let Some((_, value, checks)) = self.written_winner_value(node, &winner) else {
                    continue;
                };
                let substituted = match checks.substitution {
                    WrittenSubstitution::None => {
                        reads_element |= checks.reads_element_random;
                        continue;
                    }
                    WrittenSubstitution::Unresolved => {
                        self.custom_property_environments
                            .substitution(value, winner.property, environment)
                    }
                    // A longhand pending its shorthand's substitution reads what the shorthand's does.
                    WrittenSubstitution::PendingShorthand => match value.data() {
                        crate::css::style_value::StyleValueData::PendingSubstitution {
                            original_shorthand_value,
                        } => self
                            .shorthand_declaration_written_as(node, winner.source, original_shorthand_value.pointer())
                            .and_then(|(shorthand, written)| {
                                self.custom_property_environments
                                    .substitution(&written, shorthand, environment)
                            }),
                        _ => None,
                    },
                };
                let longhand = winner.property >= crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID;
                let (element, tree_counting) = substituted.map_or((true, true), |value| {
                    let reads = crate::css::style_compute::collect_external_value_dependencies(value.data());
                    (
                        reads.has_unfixed_random_sharing || reads.container_relative_length_unit_mask != 0,
                        reads.uses_tree_counting_function,
                    )
                });
                reads_element |= element;
                reads_tree_counting |= longhand && tree_counting;
            }
        }
        let attributes = if reads_element || written.reads_attributes {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            if reads_element {
                node.hash(&mut hasher);
            }
            self.facts
                .substitution_attributes(self.substitution_attribute_element(node, pseudo_kind))
                .hash(&mut hasher);
            hasher.finish() | 1
        } else {
            0
        };
        ElementReads {
            attributes,
            tree_counting: if reads_tree_counting {
                (self.tree.tree_scope(node).0, self.element_tree_counting_inputs(node))
            } else {
                (0, 0)
            },
        }
    }

    /// What a record for this node under this cascade state owes the monospace recascade, as a
    /// cohort key carries it. Two nodes whose parents hold equal records can still sit under
    /// different cascaded font-size chains, and the recascade reads the chain rather than the
    /// records, so a cohort that ignored this would hand one node the other's font size.
    fn monospace_cohort_key(&self, node: StyleNodeID, state: CascadeStateID) -> i32 {
        if !self.font_family_winner_is_monospace(state) {
            return 0;
        }
        let inputs = self.document_style_computation_inputs;
        match self.monospace_recascaded_font_size(computed::ComputedStyleTarget::new(node, u8::MAX), &inputs) {
            drive::MonospaceRecascade::Size(size, _) => size,
            // The drive this key is for waits for the same font, and takes the key again after.
            drive::MonospaceRecascade::AwaitsFont(_) => i32::MIN,
        }
    }

    /// Whether a record holds a composition its animations made. The transitions its table
    /// declares are a different question: what they start is decided against the values a delta
    /// moves, and the step the row leaves runs where the row is applied.
    fn record_holds_an_animation_overlay(&self, record: computed::FinalStyleRecordID) -> bool {
        self.computed_group_sets
            .style_record_view(record.raw())
            .is_none_or(|view| !view.animated_overlay.is_null())
    }

    /// Whether an element's composition animates an input of the box-type, overflow, or
    /// text-alignment adjustments, which a sample over the record's standing base cannot make again.
    fn composition_feeds_a_post_compute_adjustment(&self, record: computed::FinalStyleRecordID) -> bool {
        self.computed_group_sets
            .style_record_view(record.raw())
            .and_then(|view| unsafe { view.animated_overlay.as_ref() })
            .is_some_and(|overlay| {
                overlay
                    .entries()
                    .iter()
                    .any(|entry| property_feeds_post_compute_adjustment(entry.property))
            })
    }

    /// Whether sampling an element's composition again over a new base answers it. A transition's
    /// entry decides against the before-change style the base moves, and a sample cannot adjust
    /// the base values an animated box-type, overflow, or text-alignment input feeds.
    fn composition_resamples_over_a_new_base(&self, record: computed::FinalStyleRecordID, facts: u32) -> bool {
        facts & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0
            && !self.record_declares_transitions(record)
            && self
                .computed_group_sets
                .style_record_view(record.raw())
                .and_then(|view| unsafe { view.animated_overlay.as_ref() })
                .is_some_and(|overlay| {
                    overlay.entries().iter().all(|entry| {
                        !entry.result_of_transition && !property_feeds_post_compute_adjustment(entry.property)
                    })
                })
    }

    /// The environment an element's own values substitute under: the one its animations sampled
    /// custom properties into, where that is composed over `environment`, or `environment`.
    pub(super) fn substitution_environment(&self, node: StyleNodeID, environment: u64) -> u64 {
        match self.sampled_custom_property_environments.get(&node) {
            Some(&sampled) if self.sampled_environment_is_over(sampled, environment) => sampled,
            _ => environment,
        }
    }

    /// Whether `sampled`, an environment an element's animations composed custom properties into,
    /// is composed over `environment`. One the engine minted over another identity still is where
    /// the two resolve alike, as a record computed again publishes the same environment under a new
    /// identity.
    pub(super) fn sampled_environment_is_over(&self, sampled: u64, environment: u64) -> bool {
        use crate::css::custom_properties::CustomPropertyStore;
        let store_of = |identity: u64| match identity {
            0 => Some(std::ptr::null()),
            identity => self.custom_property_environments.store(identity),
        };
        let Some(base_store) = store_of(environment) else {
            return false;
        };
        if let Some((_, sampled_base)) = self.custom_property_environments.engine_environment(sampled) {
            return sampled_base == environment
                || store_of(sampled_base).is_some_and(|sampled_base_store| unsafe {
                    CustomPropertyStore::resolve_alike(sampled_base_store, base_store)
                });
        }
        self.custom_property_environments
            .store(sampled)
            .is_some_and(|sampled_store| unsafe { CustomPropertyStore::is_composed_over(sampled_store, base_store) })
    }

    /// A partial drive reuses the base groups beneath an element's own composition. They must
    /// still inherit the parent's sampled values, since the drive only rebuilds moved groups.
    fn animation_base_inherits_from_current_parent(
        &self,
        node: StyleNodeID,
        state: CascadeStateID,
        record: computed::FinalStyleRecordID,
    ) -> bool {
        let Some(parent) = self.tree.flat_tree_parent(node) else {
            return false;
        };
        let Some(parent_record) = self.computed_group_sets.assigned_style_record(parent) else {
            return false;
        };
        let Some(view) = self.computed_group_sets.base_style_record_view(record) else {
            return false;
        };
        let Some(parent_view) = self.computed_group_sets.style_record_view(parent_record.raw()) else {
            return false;
        };
        let own_groups = self.state_owned_inherited_groups(state);
        (0..computed::ENGINE_INHERITED_GROUP_COUNT).all(|group| {
            own_groups & (1 << group) != 0
                || view.payloads[group] == parent_view.payloads[group]
                || crate::css::computed_values::style_group_payloads_equal(
                    group,
                    view.payloads[group].as_ptr(),
                    parent_view.payloads[group].as_ptr(),
                )
        })
    }

    /// Whether the table a record was computed into declares transitions at all, and whether any
    /// of the moved properties is a longhand one of them runs on. A record the engine cannot look
    /// into answers both, so the row that asks is refused.
    /// Whether a transition step decides anything for a record: it declares transitions, or its
    /// composition still runs one that a transition-property of `all` with no duration left keeps.
    fn record_owes_a_transition_decision(&self, record: computed::FinalStyleRecordID) -> bool {
        self.record_declares_transitions(record)
            || self
                .computed_group_sets
                .style_record_view(record.raw())
                .and_then(|view| unsafe { view.animated_overlay.as_ref() })
                .is_some_and(|overlay| overlay.entries().iter().any(|entry| entry.result_of_transition))
    }

    pub(super) fn record_declares_transitions(&self, record: computed::FinalStyleRecordID) -> bool {
        self.computed_group_sets
            .style_record_view(record.raw())
            .and_then(|view| unsafe { view.longhand_table.as_ref() })
            .is_none_or(crate::css::style_compute::has_active_transition_properties)
    }

    /// The context a registered custom property's lengths resolve against for a row that keeps
    /// its record's font: the element's OWN font metrics, which is what C++ absolutizes a
    /// registered value with once the font phase has run. `None` where the row has no record to
    /// read them from, so the caller leaves the registered names to the host.
    pub(super) fn own_font_length_resolution_context(
        &self,
        record: computed::FinalStyleRecordID,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> Option<(crate::css::style_compute::FfiLengthResolutionContext, u8)> {
        use crate::css::computed_value_types::{STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_INHERITED_BOX};
        use crate::css::style_compute::{FfiFontMetrics, FfiLengthResolutionContext};

        let view = self.computed_group_sets.base_style_record_view(record)?;
        let payloads = view.payloads;
        let table = unsafe { view.longhand_table.as_ref() }?;
        let font = unsafe {
            payloads[STYLE_GROUP_INDEX_FONT]
                .cast::<crate::css::computed_value_types::FontValues>()
                .deref()
        };
        let inherited_box = unsafe {
            payloads[STYLE_GROUP_INDEX_INHERITED_BOX]
                .cast::<crate::css::computed_values::InheritedBoxValues>()
                .deref()
        };
        let length = FfiLengthResolutionContext {
            viewport_width: inputs.viewport_width,
            viewport_height: inputs.viewport_height,
            font_metrics: FfiFontMetrics {
                font_size: font.font_size.to_double(),
                x_height: drive_font_metric(font.font_x_height),
                cap_height: drive_font_metric(font.font_ascent),
                zero_advance: drive_font_metric(font.font_zero_advance),
                line_height: font.line_height_used.to_double(),
            },
            root_font_metrics: FfiFontMetrics {
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
            subject_inline_axis_is_horizontal: inherited_box.writing_mode
                == crate::css::css_enums::writing_mode::HORIZONTAL_TB,
            resolved_viewport_relative_length: std::ptr::null_mut(),
        };
        Some((length, table.effective_color_scheme() as u8))
    }

    /// The context a registered custom property resolves against when its caller brings none:
    /// the record the node (or, for a pseudo-element, its originating element) holds, or else
    /// the provisional context over the node's inheritance parent.
    pub(super) fn standing_registered_value_context(
        &self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> custom_property_cascade::RegisteredValueContext {
        let own = self.computed_group_sets.assigned_style_record(node);
        if pseudo.is_none()
            && let Some((length, color_scheme)) =
                own.and_then(|record| self.own_font_length_resolution_context(record, inputs))
        {
            return custom_property_cascade::RegisteredValueContext { length, color_scheme };
        }
        let context_record = match pseudo {
            Some(_) => own,
            None => self
                .record_inheritance_parent(node)
                .and_then(|parent| self.computed_group_sets.assigned_style_record(parent)),
        };
        self.provisional_registered_value_context(context_record, inputs)
    }

    pub(super) fn provisional_registered_value_context(
        &self,
        parent_record: Option<computed::FinalStyleRecordID>,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
    ) -> custom_property_cascade::RegisteredValueContext {
        use crate::css::style_compute::{FfiFontMetrics, FfiLengthResolutionContext};
        if let Some((length, color_scheme)) =
            parent_record.and_then(|record| self.own_font_length_resolution_context(record, inputs))
        {
            return custom_property_cascade::RegisteredValueContext { length, color_scheme };
        }
        let metrics = FfiFontMetrics {
            font_size: inputs.initial_font_size,
            x_height: inputs.initial_font_x_height,
            cap_height: inputs.initial_font_cap_height,
            zero_advance: inputs.initial_font_zero_advance,
            line_height: 0.0,
        };
        custom_property_cascade::RegisteredValueContext {
            length: FfiLengthResolutionContext {
                viewport_width: inputs.viewport_width,
                viewport_height: inputs.viewport_height,
                font_metrics: metrics,
                root_font_metrics: metrics,
                font_metrics_depend_on_viewport_metrics: false,
                root_font_metrics_depend_on_viewport_metrics: false,
                has_container_width_basis: false,
                has_container_height_basis: false,
                container_width_basis: 0.0,
                container_height_basis: 0.0,
                container_width_basis_depends_on_viewport_metrics: false,
                container_height_basis_depends_on_viewport_metrics: false,
                subject_inline_axis_is_horizontal: true,
                resolved_viewport_relative_length: std::ptr::null_mut(),
            },
            color_scheme: inputs.preferred_color_scheme,
        }
    }

    fn record_is_shareable(&self, record: computed::FinalStyleRecordID, as_first_record: bool) -> bool {
        self.computed_group_sets
            .style_record_view(record.raw())
            .is_some_and(|view| {
                view.animated_overlay.is_null()
                    && (unsafe { view.longhand_table.as_ref() }).is_some_and(|table| {
                        as_first_record || !crate::css::style_compute::has_active_transition_properties(table)
                    })
            })
    }

    /// Select the remaining-phase properties and output groups needed to derive a record from a
    /// donor. Dependencies belong to the donor's published record, since the new node has no
    /// computed row yet.
    fn cold_record_donor_selection(
        &mut self,
        donor: ColdRecordDonor,
        properties: &[u16],
    ) -> Option<(u32, [u64; crate::css::property_metadata::LONGHAND_WORD_COUNT])> {
        use crate::css::computed_value_types::{
            STYLE_GROUP_INDEX_ANCHOR, STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_SURROUND,
        };
        use crate::css::computed_values::computed_group_dependency_mask;
        use crate::css::property_metadata::{FIRST_LONGHAND_PROPERTY_ID, LONGHAND_WORD_COUNT};

        if properties.is_empty()
            || properties.iter().any(|&property| {
                !property_computes_in_remaining_phase(property) || property_feeds_box_type_transformation(property)
            })
        {
            return None;
        }
        let mut groups_to_rebuild = 0_u32;
        let mut selected = [0_u64; LONGHAND_WORD_COUNT];
        let mut select = |property: u16| {
            let index = usize::from(property - FIRST_LONGHAND_PROPERTY_ID);
            selected[index / 64] |= 1 << (index % 64);
        };
        let view = self.computed_group_sets.style_record_view(donor.record.record.raw())?;
        let inherited_box = unsafe {
            view.payloads[crate::css::computed_value_types::STYLE_GROUP_INDEX_INHERITED_BOX]
                .cast::<crate::css::computed_values::InheritedBoxValues>()
                .deref()
        };
        for &property in properties {
            if property_starts_animation(property) {
                return None;
            }
            let groups = computed_group_dependency_mask(property)?;
            groups_to_rebuild |= groups;
            select(property);
            let bits = crate::css::style_compute::table_row_bits(property);
            let counterpart = if bits & crate::css::style_compute::LOGICAL_ALIAS_BIT != 0 {
                crate::css::style_compute::map_logical_alias_to_physical(
                    property,
                    inherited_box.writing_mode,
                    inherited_box.direction,
                )
            } else if bits & crate::css::style_compute::PHYSICAL_TO_LOGICAL_BIT != 0 {
                crate::css::style_compute::map_physical_to_logical_alias(
                    property,
                    inherited_box.writing_mode,
                    inherited_box.direction,
                )
            } else {
                property
            };
            if counterpart != property {
                groups_to_rebuild |= computed_group_dependency_mask(counterpart)?;
                select(counterpart);
            }
        }
        if groups_to_rebuild & (1 << STYLE_GROUP_INDEX_ANCHOR) != 0 {
            groups_to_rebuild |= 1 << STYLE_GROUP_INDEX_SURROUND;
        }
        if properties.contains(&crate::css::property_metadata::property_id::COLOR) {
            let (dependent_groups, dependent_properties) = self
                .computed_group_sets
                .record_current_color_dependencies(donor.record.record)?;
            groups_to_rebuild |= dependent_groups;
            for (word, &bits) in dependent_properties.iter().enumerate() {
                let mut bits = bits;
                while bits != 0 {
                    let bit = bits.trailing_zeros() as usize;
                    bits &= bits - 1;
                    let property = FIRST_LONGHAND_PROPERTY_ID + (word * 64 + bit) as u16;
                    select(property);
                    if crate::css::style_compute::table_row_bits(property)
                        & crate::css::style_compute::LOGICAL_ALIAS_BIT
                        != 0
                    {
                        select(crate::css::style_compute::map_logical_alias_to_physical(
                            property,
                            inherited_box.writing_mode,
                            inherited_box.direction,
                        ));
                    }
                }
            }
        }
        if groups_to_rebuild & (1 << STYLE_GROUP_INDEX_FONT) != 0 {
            return None;
        }
        Some((groups_to_rebuild, selected))
    }

    fn drive_record_over_installed_ancestors_step(
        &mut self,
        node: StyleNodeID,
        armed: bool,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> Drive<u64> {
        let facts = self.computed_group_sets.adjustment_facts(node);
        // An element standing for its host's pseudo-element is its host's to cascade.
        if self.backs_host_pseudo_element(node) {
            let parent_inputs_moved = ParentInputsMoved {
                inherited_style: true,
                display: true,
            };
            return self
                .engine_computed_record_delta(node, true, None, parent_inputs_moved, scratch, counters)
                .map(|(_, record)| record.raw());
        }
        // The layout snapshot carries the style record whose box supplied its basis. Republish
        // stale winners only when every queried basis belongs to the now-settled containers.
        let container_winners_are_stale = self.published_container_verdicts.get(&node).is_some_and(|verdicts| {
            !verdicts.is_empty()
                && verdicts.iter().all(|&(rule, pseudo, _)| {
                    self.rule_container_verdict(rule, node.raw(), pseudo)
                        .is_some_and(|verdict| {
                            !verdict.effects.iter().any(|(_, effect, _)| {
                                matches!(effect, bridge::FfiContainerEffectKind::NeedsEvaluationAfterLayout)
                            }) && (!verdict.depends_on_size
                                || verdict.effects.iter().all(|(basis_raw, effect, _)| {
                                    if !matches!(effect, bridge::FfiContainerEffectKind::SizeContainerUsage) {
                                        return true;
                                    }
                                    StyleNodeID::from_raw(*basis_raw).is_some_and(|basis| {
                                        self.container_query_inputs(basis).is_some_and(|inputs| {
                                            self.layout_style_snapshots.row(basis).is_some_and(|snapshot| {
                                                snapshot.has_committed_box
                                                    && self.committed_container_box_applies(
                                                        snapshot.style_record,
                                                        inputs.style_record,
                                                    )
                                            })
                                        })
                                    })
                                }))
                        })
                })
        }) && self.current_winner_groups().row_stamp(node) != Some(self.flush_stamp);
        let winner_key = WinnerGroupKey::current(node, self.program.version());
        let republished_complete = if container_winners_are_stale
            || self.container_gates_unheld.contains(&node)
            || self.container_verdicts_moved(node)
            || !matches!(self.current_winner_groups().token_for(winner_key), Lookup::Known(_))
        {
            Some(self.republish_driven_winners(node, counters))
        } else {
            None
        };
        // A row left without winners has no cached record to take; the drive below answers it.
        let cascade_state = self.driven_element_winners(node, winner_key, counters);
        // An armed row's answer may declare past its winners: a record computed from it is no
        // function of the winner state the cold record cache is keyed by.
        if let Some(cascade_state) = cascade_state
            && !scratch.font_drive.is_pending()
            && !(armed && self.computed_group_sets.node_answer_is_incomplete(node))
            && !self.node_declares_custom_properties(node)
            && self.state_container_unit_mask(node, cascade_state.1) == 0
            && let Some(old_style_record) = self.computed_group_sets.assigned_style_record(node)
            && let Some(parent) = self.tree.flat_tree_parent(node)
            && let Some(parent_record) = self.computed_group_sets.assigned_style_record(parent)
            && let Some(environment) = self.computed_group_sets.custom_property_environment_identity(parent)
            && let Some(pseudo_styles) = self.pseudo_style_mask(node)
        {
            let cache_key = self
                .cold_record_parent(node, parent, parent_record, cascade_state.1)
                .map(|parent| ColdRecordKey {
                    monospace_recascaded_font_size: self.monospace_cohort_key(node, cascade_state.1),
                    element_reads: self.element_reads(node, None, cascade_state.1, environment),
                    parent,
                    previous_style_record: old_style_record.raw(),
                    generation: cascade_state.0,
                    state: cascade_state.1,
                    facts,
                    pseudo_styles,
                    environment,
                    font_environment_generation: self.document_style_computation_inputs.font_environment_generation,
                    root_font_inputs: RootFontInputs::from_document(&self.document_style_computation_inputs),
                });
            if let Some((old_record, record)) = self.assign_cached_cold_record(
                node,
                computed::ComputedStyleTarget::new(node, u8::MAX),
                cascade_state,
                cache_key,
                Some(parent),
                cascade_state.1,
                old_style_record,
                0,
                scratch,
                counters,
            ) {
                let pseudos_settled =
                    self.engine_pseudo_records(node, Some(old_record), record, cascade_state.0, scratch, counters);
                if pseudos_settled.is_ok() && scratch.pseudo_uses_substitution {
                    scratch.substitution_effects.push((node, true));
                }
                if let Err(unanswered) = pseudos_settled {
                    let Unanswered::Suspended(_) = unanswered;
                    scratch.pending_element = Some((old_record, record));
                    self.apply_substitution_effects(scratch);
                    return Err(unanswered);
                }
                counters.bump(Counter::RetryAfterAncestorColdHits);
                self.apply_substitution_effects(scratch);
                return Ok(record.raw());
            }
        }
        // A row the flush armed drives its incomplete answer as the flush would have, over the
        // installed ancestor's final groups. A row introduced while the host applied the batch
        // was never offered an incomplete answer.
        debug_assert!(
            armed || scratch.font_drive.is_pending() || !self.computed_group_sets.node_answer_is_incomplete(node),
            "an unarmed row's answer is complete but for custom properties"
        );
        // A row this transaction published no answer for (one whose ancestor's environment moved,
        // say) has winners no answer here vouches for. A shadow host's answer is never retained
        // either, so rebuild the winners from an exact match: that answer says whether they are
        // complete.
        let cascade_winners_are_complete = match republished_complete {
            Some(complete) => complete,
            None => match self.current_published_answer(node) {
                Some(answer) => answer.cascade_winners_are_complete,
                None => self.republish_driven_winners(node, counters),
            },
        };
        let record = self.engine_computed_record_delta(
            node,
            cascade_winners_are_complete,
            None,
            ParentInputsMoved {
                inherited_style: true,
                display: true,
            },
            scratch,
            counters,
        );
        record.map(|(_, record)| record.raw())
    }

    /// Build a driven table's groups against the parent record's payloads and publish the record
    /// for `target` the way a C++ computation publishes one; the record's swap eligibility comes
    /// back beside its identity. A record not `assign`ed to `target` is held by nothing but its
    /// pins.
    #[allow(clippy::too_many_arguments)]
    fn assemble_and_publish_engine_record(
        &mut self,
        target: computed::ComputedStyleTarget,
        assign: bool,
        parent_record: Option<computed::FinalStyleRecordID>,
        mut table: ComputedLonghandTable,
        length: &crate::css::style_compute::FfiLengthResolutionContext,
        font: &crate::css::table_group_builder::FfiFontGroupBuildInputs,
        environment: u64,
        pseudo_styles: u64,
        counter_style_environment_identity: u64,
        cascade_state: Option<(u64, CascadeStateID)>,
        counters: &mut Counters,
    ) -> (computed::FinalStyleRecordID, bool) {
        use crate::css::table_group_builder::group_index;

        // The document element's groups build against no parent payloads. A parent record is one
        // the caller holds, which stays live through the assembly.
        let mut parent_payloads = [SharedPayload::null(); group_index::COUNT];
        let parent_in_display_none_subtree = parent_record.is_some_and(|parent_record| {
            let parent_view = self
                .computed_group_sets
                .style_record_view(parent_record.raw())
                .expect("a held parent record has a view");
            for (payload, &parent_payload) in parent_payloads.iter_mut().zip(parent_view.payloads) {
                *payload = parent_payload;
            }
            parent_view.dependency_flags & (1 << 2) != 0
        });
        let display_is_none = crate::css::style_compute::effective_display(&table, None).is_none();
        table.set_in_display_none_subtree(parent_in_display_none_subtree || display_is_none);
        table.freeze();
        let swap_eligible = table.property_inheritance_is_standard()
            && !table.display_is_list_item()
            && !crate::css::style_compute::has_active_transition_properties(&table);
        let table = table.into_raw_shared();
        // A table the catalog already holds assembled to the same payloads the last time it was
        // built against these inputs; only a new one is built.
        let assembly_inputs = computed::GroupAssemblyInputs::new(length, font);
        let assembled = self.computed_group_sets.assembled_payloads(
            Some(target),
            cascade_state,
            unsafe { &*table },
            assembly_inputs,
        );
        let reuses_assembly = assembled.is_some();
        let payloads = assembled.unwrap_or_else(|| {
            let color_inputs = crate::css::table_group_builder::assembly_color_inputs(unsafe { &*table }, length);
            parent_payloads
                .iter()
                .enumerate()
                .take(group_index::COUNT)
                .map(|(group, &parent_payload)| {
                    SharedPayload::new(unsafe {
                        crate::css::table_group_builder::assemble_group_from_table(
                            &*table,
                            group,
                            Some(font),
                            parent_payload.as_ptr(),
                            color_inputs,
                            length,
                        )
                    })
                })
                .collect::<Vec<_>>()
        });
        let holds_image_values = crate::css::computed_values::style_group_payloads_hold_image_values(
            HostShared::as_pointer_slice(&payloads),
        );
        let dependency_flags = unsafe { &*table }.publication_dependency_flags()
            | (u8::from(swap_eligible) * computed::INHERITED_GROUP_SWAP_ELIGIBLE)
            | (u8::from(holds_image_values) * computed::HOLDS_IMAGE_VALUES);
        // A record that named a counter style is the answer only while that scope's registry is
        // the one it read, so it carries the identity the host published for the scope.
        let counter_style_environment_identity = if counter_style_environment_identity != 0 {
            counter_style_environment_identity
        } else {
            self.table_counter_style_environment_identity(target.node(), unsafe { &*table })
        };
        let metadata_input = computed::ComputedMetadataInput {
            pseudo_element_styles: pseudo_styles,
            dependency_flags,
            counter_style_environment_identity,
            animation_overlay_identity: 0,
            animated_overlay: HostShared::null(),
            animation_overlay_payloads: &[],
            longhand_table: HostShared::new(table),
        };
        if let Some(cascade_state) = cascade_state {
            self.computed_group_sets
                .set_pending_cascade_state(target, cascade_state);
        }
        // The drive built every payload and the table, and holds the only reference to each:
        // hand them to the catalog rather than have it retain a second one per published payload.
        // Reused payloads are the catalog's own.
        let owned = computed::PendingRecordOwnership {
            groups: if reuses_assembly {
                0
            } else {
                u32::try_from((1_u64 << payloads.len()) - 1).expect("a style group index fits the ownership mask")
            },
            table: true,
        };
        let publication = self.publish_computed_groups_impl(
            assign.then_some(target),
            &payloads,
            computed::ENGINE_INHERITED_GROUP_COUNT,
            environment,
            metadata_input,
            owned,
            counters,
        );
        self.computed_group_sets.remember_table_assembly(
            publication.style_record_identity,
            cascade_state,
            assembly_inputs,
            !reuses_assembly,
        );
        let transferred = publication.transferred;
        for (group, payload) in payloads.into_iter().enumerate().filter(|_| !reuses_assembly) {
            if transferred.groups & (1 << group) == 0 {
                crate::css::computed_values::release_group_payload(group, payload.as_ptr());
            }
        }
        if !transferred.table {
            unsafe {
                crate::css::computed_longhand_table::rust_computed_longhand_table_release(table.cast_mut());
            }
        }
        (publication.style_record_identity, swap_eligible)
    }

    /// Whether a node's record inherits from the parent it has now: each inherited group outside
    /// `owned_groups` is the parent's own, no non-inherited property is inherited explicitly, and
    /// its custom-property environment is the parent's.
    fn record_inherits_from_current_parent(&self, node: StyleNodeID, state: CascadeStateID, owned_groups: u32) -> bool {
        let Some(parent) = self.tree.flat_tree_parent(node) else {
            return false;
        };
        if self.state_explicitly_inherits_non_inherited_property(node, state) {
            return false;
        }
        self.computed_group_sets
            .inherited_groups_follow_parent(node, parent, owned_groups)
            .unwrap_or(false)
            && self
                .computed_group_sets
                .custom_property_environment_identity(node)
                .is_some_and(|environment| {
                    self.computed_group_sets.custom_property_environment_identity(parent) == Some(environment)
                })
    }

    /// The inherited groups a state's winners rebuild for their element: the groups its
    /// declarations land in, the font group when it declares a longhand that group carries
    /// without landing in it, and the ones holding colors when it declares `color`, which their
    /// currentcolor-dependent values resolve against.
    fn state_owned_inherited_groups(&self, state: CascadeStateID) -> u32 {
        use crate::css::computed_value_types::{
            STYLE_GROUP_INDEX_FONT, STYLE_GROUP_INDEX_INHERITED_SVG, STYLE_GROUP_INDEX_INHERITED_TEXT,
            STYLE_GROUP_INDEX_INHERITED_UI,
        };
        use crate::css::property_metadata::{property_id as prop, property_style_group_index};
        self.winner_groups
            .properties_in_state(state)
            .fold(0_u32, |mask, property| {
                let mask = mask | property_style_group_index(property).map_or(0, |group| 1 << group);
                // A font longhand the group carries rather than holds, such as
                // `font-variant-numeric`, has no group of its own, so the group it does rebuild
                // has to be named here or nothing names it.
                let mask = if font_group_carries_longhand(property) {
                    mask | (1 << STYLE_GROUP_INDEX_FONT)
                } else {
                    mask
                };
                if property == prop::COLOR {
                    mask | (1 << STYLE_GROUP_INDEX_INHERITED_UI)
                        | (1 << STYLE_GROUP_INDEX_INHERITED_SVG)
                        | (1 << STYLE_GROUP_INDEX_INHERITED_TEXT)
                } else {
                    mask
                }
            })
    }

    fn root_font_inputs_from_record(&self, record: computed::FinalStyleRecordID) -> Option<RootFontInputs> {
        self.root_font_inputs_from_raw_record(record.raw())
    }

    /// The font metrics a `rem` resolves against when the document element holds `record`.
    pub(super) fn root_font_inputs_from_raw_record(&self, record: u64) -> Option<RootFontInputs> {
        use crate::css::computed_value_types::STYLE_GROUP_INDEX_FONT;
        let view = self.computed_group_sets.style_record_view(record)?;
        let font = unsafe {
            view.payloads[STYLE_GROUP_INDEX_FONT]
                .cast::<crate::css::computed_value_types::FontValues>()
                .deref()
        };
        Some(RootFontInputs {
            metrics: [
                font.font_size.to_double().to_bits(),
                drive_font_metric(font.font_x_height).to_bits(),
                drive_font_metric(font.font_ascent).to_bits(),
                drive_font_metric(font.font_zero_advance).to_bits(),
                font.line_height_used.to_double().to_bits(),
            ],
            depends_on_viewport: view.dependency_flags & (1 << 1) != 0,
        })
    }

    /// The host boundary publishes root inputs after materializing a document element.
    /// Cache entries name these inputs; publication does not clear them as a side effect.
    fn prepare_root_font_metrics_from_record(&mut self, record: computed::FinalStyleRecordID) {
        let Some(root_inputs) = self.root_font_inputs_from_record(record) else {
            return;
        };
        root_inputs.apply_to(&mut self.document_style_computation_inputs);
    }

    fn element_drive_subject(&self, node: StyleNodeID) -> DriveSubject {
        DriveSubject {
            target: computed::ComputedStyleTarget::new(node, u8::MAX),
            parent: self.record_inheritance_parent(node),
            facts: self.computed_group_sets.adjustment_facts(node),
            highlight_parent: None,
        }
    }

    /// Keeps a warm record's cohort past its transaction. An element whose style moves back and
    /// forth between the same states, a class toggled on and off, moves between the same records
    /// every time, and the cohort a transaction kept answers the next one's alike rows.
    fn remember_warm_record_cohort(&mut self, key: RecordCohortKey, value: RecordCohortValue) {
        if self.engine_warm_record_cohorts.len() >= COLD_RECORD_CACHE_LIMIT {
            self.engine_warm_record_cohorts.clear();
        }
        self.engine_warm_record_cohorts.insert(key, value);
    }

    /// Whether a node can take a record a cohort kept past its transaction at all. The custom
    /// properties a node declares, and the container queries its rules wait on, are decided for it
    /// after its cohort is looked up, as they are for a first record the cold record cache answers;
    /// a kept record was decided under what they were then.
    fn may_take_a_kept_warm_record_cohort(&self, node: StyleNodeID) -> bool {
        !self.node_declares_custom_properties(node)
            && !self.published_container_verdicts.contains_key(&node)
            && !self.container_gates_unheld.contains(&node)
    }

    /// Whether a record a cohort kept past its transaction can still answer for the node: it is
    /// live, and what it computed from the counter-style registry is what the node would read now.
    /// The rest of what it was computed from is in the key, and the document inputs that are not
    /// clear the kept cohorts when they move.
    fn warm_record_cohort_is_current(&self, node: StyleNodeID, record: computed::FinalStyleRecordID) -> bool {
        self.computed_group_sets.final_style_record_is_live(record.raw())
            && self.record_counter_environment_is_current(node, record)
    }

    /// A later element alike in what a first record is computed from takes this record, the way a
    /// C++ computation shares across transactions. The cache is small and bounded.
    fn remember_cold_record(&mut self, key: ColdRecordKey, record: ColdRecord) {
        if self.engine_cold_record_cache.len() >= COLD_RECORD_CACHE_LIMIT {
            self.engine_cold_record_cache.clear();
            self.engine_cold_record_donors.clear();
        }
        self.engine_cold_record_cache.insert(key, record);
        if key.previous_style_record == 0 {
            let donor_key = ColdRecordDonorKey {
                parent: key.parent,
                generation: key.generation,
                property_shape_hash: self.winner_groups.property_shape_hash(key.state),
                facts: key.facts,
                pseudo_styles: key.pseudo_styles,
                environment: key.environment,
                font_environment_generation: key.font_environment_generation,
                root_font_inputs: key.root_font_inputs,
                element_reads: key.element_reads,
            };
            let donors = self.engine_cold_record_donors.entry(donor_key).or_default();
            if let Some(existing) = donors.iter_mut().find(|donor| donor.state == key.state) {
                existing.record = record;
                return;
            }
            if donors.len() == MAXIMUM_COLD_RECORD_DONORS_PER_KEY {
                donors.remove(0);
            }
            donors.push(ColdRecordDonor {
                state: key.state,
                record,
            });
        }
    }

    /// A query verdict can stand while a container-relative value changes. Only a row whose
    /// entire retained value inventory excludes those reads may keep its record on this input.
    pub(super) fn container_input_requires_full_drive(&self, node: StyleNodeID) -> bool {
        if !self.published_container_verdicts.contains_key(&node)
            || self.computed_group_sets.adjustment_facts(node) & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0
        {
            return true;
        }
        let Lookup::Known((_, state)) = self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        else {
            return true;
        };
        std::iter::once(state)
            .chain(
                self.current_winner_groups()
                    .pseudo_states(node)
                    .map(|(_, _, state, _)| state),
            )
            .any(|state| {
                self.state_has_substitutions(node, state)
                    || self
                        .winner_groups
                        .winners_in_state(state)
                        .filter_map(|winner| self.winner_groups.resolved_winner(winner))
                        .any(|winner| match self.written_winner_value(node, &winner) {
                            Some((_, value, _)) => {
                                crate::css::style_compute::external_value_dependencies(value.data())
                                    .container_relative_length_unit_mask
                                    != 0
                            }
                            _ => true,
                        })
            })
    }

    /// The parent-side half of a first record's sharing key, or nothing when the parent's style
    /// is one no first record may be shared under.
    fn cold_record_parent(
        &self,
        node: StyleNodeID,
        parent: StyleNodeID,
        parent_record: computed::FinalStyleRecordID,
        state: CascadeStateID,
    ) -> Option<ColdRecordParent> {
        let view = self.computed_group_sets.style_record_view(parent_record.raw())?;
        if !view.animated_overlay.is_null() {
            return None;
        }
        let dependency_flags = view.dependency_flags;
        let environment = self.computed_group_sets.custom_property_environment_identity(parent)?;
        let inherited_groups = self.computed_group_sets.node_inherited_groups_identity(parent)?;
        let parent_display = self.box_type_parent_display(parent)?;
        let record = if self.state_explicitly_inherits_non_inherited_property(node, state) {
            parent_record.raw()
        } else {
            0
        };
        Some(ColdRecordParent {
            record,
            inherited_groups,
            environment,
            dependency_flags,
            parent_display,
        })
    }

    /// The display the box-type transformation reads as the parent's for a child of the parent:
    /// the parent's own, past any display:contents ancestor, packed into one word.
    fn box_type_parent_display(&self, parent: StyleNodeID) -> Option<u32> {
        self.box_type_display_key(parent, false)
    }

    /// The same for a pseudo-element of `node`. Its records are computed under a display:none
    /// element too, from that display, so the key carries it.
    pub(super) fn box_type_originating_display(&self, node: StyleNodeID) -> Option<u32> {
        self.box_type_display_key(node, true)
    }

    fn box_type_display_key(&self, parent: StyleNodeID, admits_none: bool) -> Option<u32> {
        let mut ancestor = Some(parent);
        while let Some(current) = ancestor {
            let record = self.computed_group_sets.assigned_style_record(current)?;
            let view = self.computed_group_sets.style_record_view(record.raw())?;
            let table = unsafe { view.longhand_table.as_ref() }?;
            let display = crate::css::style_compute::effective_display(table, None);
            // C++ styles the children of a display:none element on demand, past the engine's
            // view of the parent, so no first record is computed under one.
            if display.is_none() && !admits_none {
                return None;
            }
            if !display.is_contents() {
                return Some(
                    u32::from(display.tag)
                        | u32::from(display.outside) << 8
                        | u32::from(display.inside) << 16
                        | u32::from(display.internal) << 24
                        | u32::from(display.list_item) << 3
                        | u32::from(display.box_value) << 4,
                );
            }
            ancestor = self.tree.flat_tree_parent(current);
        }
        None
    }

    /// The display the box-type transformation reads for one computation target. A
    /// pseudo-element inherits from its originating element; an element starts at its flat-tree
    /// parent. The returned display is retained computed state, never a DOM projection.
    pub fn box_type_parent_display_for_target(
        &self,
        node: StyleNodeID,
        is_pseudo_element: bool,
    ) -> Option<crate::css::display::FfiDisplay> {
        let mut ancestor = is_pseudo_element
            .then_some(node)
            .or_else(|| self.tree.flat_tree_parent(node));
        while let Some(current) = ancestor {
            let record = self.computed_group_sets.assigned_style_record(current)?;
            let view = self.computed_group_sets.style_record_view(record.raw())?;
            let table = unsafe { view.longhand_table.as_ref() }?;
            let display =
                crate::css::style_compute::effective_display(table, unsafe { view.animated_overlay.as_ref() });
            if !display.is_contents() {
                return Some(display);
            }
            ancestor = self.tree.flat_tree_parent(current);
        }
        None
    }

    /// Whether a winner state declares `inherit` for a non-inherited property, or carries a value
    /// the engine cannot see the spelling of.
    pub(super) fn node_explicitly_inherits_non_inherited_property(&self, node: StyleNodeID) -> bool {
        let Lookup::Known((_, state)) = self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        else {
            return false;
        };
        self.state_explicitly_inherits_non_inherited_property(node, state)
    }

    fn state_explicitly_inherits_non_inherited_property(&self, node: StyleNodeID, state: CascadeStateID) -> bool {
        let is_inherit_keyword = |value: Option<&crate::css::style_value::RetainedStyleValueData>| {
            value.is_none_or(|value| {
                matches!(value.data(), crate::css::style_value::StyleValueData::Keyword { keyword }
                    if *keyword == crate::css::style_compute::keyword::INHERIT)
            })
        };
        self.winner_groups.winners_in_state(state).any(|winner| {
            if crate::css::property_metadata::property_is_inherited(winner.property) {
                return false;
            }
            let Some(winner) = self.winner_groups.resolved_winner(winner) else {
                return false;
            };
            match winner.source {
                WinnerSource::Rule(rule) => is_inherit_keyword(self.program.written_winner_value(
                    rule,
                    winner.property,
                    winner.important,
                    winner.key.value,
                )),
                WinnerSource::Element(kind) => {
                    let (declared, _) = self.facts.element_declared_properties(node, kind);
                    let written = self.facts.element_written_declared_values(node, kind);
                    let index = declared.iter().rposition(|declared| {
                        declared.property == winner.property
                            && declared.important == winner.important
                            && declared.value == winner.key.value
                    });
                    is_inherit_keyword(index.and_then(|index| written.get(index)))
                }
                WinnerSource::ExactCascade => true,
            }
        })
    }

    /// Keep a record C++ published for an element as a first record a later alike element can
    /// take, when it was computed from nothing but what the engine keys first records on: the
    /// parent's inherited style and environment, the winner state, the element facts and the
    /// pseudo-elements it has rules for.
    #[allow(clippy::too_many_arguments)]
    fn remember_cold_record_candidate(
        &mut self,
        target: computed::ComputedStyleTarget,
        cascade_state: (u64, CascadeStateID),
        custom_property_environment: u64,
        pseudo_styles: u64,
        previous_style_record: Option<computed::FinalStyleRecordID>,
        style_record: computed::FinalStyleRecordID,
        is_base_record: bool,
    ) {
        if target.is_pseudo() || !is_base_record {
            return;
        }
        let inputs = self.document_style_computation_inputs;
        let node = target.node();
        let facts = self.computed_group_sets.adjustment_facts(node);
        if self.node_declares_custom_properties(node) {
            return;
        }
        if self.state_container_unit_mask(node, cascade_state.1) != 0 {
            return;
        }
        // A record C++ computed for an element with animations is not what its winner state alone
        // describes.
        if facts & bridge::element_adjustment_fact::HAS_ANIMATIONS != 0 {
            return;
        }
        // A state minted before the winner groups were evicted names nothing in the table now.
        if cascade_state.0 != self.winner_groups.generation() {
            return;
        }
        let Some(parent) = self.tree.flat_tree_parent(node) else {
            return;
        };
        let Some(parent_record) = self.computed_group_sets.assigned_style_record(parent) else {
            return;
        };
        if self.computed_group_sets.custom_property_environment_identity(parent) != Some(custom_property_environment) {
            return;
        }
        let Some(parent) = self.cold_record_parent(node, parent, parent_record, cascade_state.1) else {
            return;
        };
        let swap_eligible = self.computed_group_sets.node_inherited_group_swap_eligible(node);
        let key = ColdRecordKey {
            monospace_recascaded_font_size: self.monospace_cohort_key(node, cascade_state.1),
            element_reads: self.element_reads(node, None, cascade_state.1, custom_property_environment),
            parent,
            previous_style_record: previous_style_record.map_or(0, computed::FinalStyleRecordID::raw),
            generation: cascade_state.0,
            state: cascade_state.1,
            facts,
            pseudo_styles,
            environment: custom_property_environment,
            font_environment_generation: inputs.font_environment_generation,
            root_font_inputs: RootFontInputs::from_document(&inputs),
        };
        self.remember_cold_record(
            key,
            ColdRecord {
                record: style_record,
                swap_eligible,
                // FIXME: C++ computed this record and knows whether it read the parent's
                //        non-inherited groups, but does not name that here, so an element the
                //        engine hands it to cannot owe its own parent the mark.
                explicitly_inherited_groups: 0,
            },
        );
    }

    /// The synthetic pseudo-elements the node has rules for, as a C++ record's pseudo-style mask,
    /// or no bits when the engine holds no answer for the node.
    pub(crate) fn published_pseudo_style_mask(&self, node: StyleNodeID) -> u64 {
        self.pseudo_style_mask(node).unwrap_or(0)
    }

    /// Whether the node's published style holds a `::first-letter` record. The tree build asks a
    /// block this before it goes looking for the letter to style, and again of each block it
    /// descends into, which stops the search where a nested block styles its own first letter.
    #[must_use]
    pub(crate) fn has_published_first_letter_style(&self, node: StyleNodeID) -> bool {
        self.computed_group_sets
            .pseudo_style_record(node, pseudo_kind::FIRST_LETTER)
            .is_some()
    }

    /// Whether the style record answers for a counter or a quote: the two things whose state runs
    /// along the whole tree rather than staying inside one box.
    fn style_record_affects_generated_content_state(&self, style_record: Option<computed::FinalStyleRecordID>) -> bool {
        self.published_style_record_view(style_record)
            .is_some_and(crate::css::computed_value_views::ComputedValuesView::affects_generated_content_state)
    }

    /// Whether the node or any of its DOM descendants styles a counter or a quote. Moving such a
    /// subtree renumbers what follows it, so the layout tree build has to rebuild rather than
    /// splice, and so does a removal.
    #[must_use]
    pub fn subtree_affects_generated_content_state(&self, node: StyleNodeID) -> bool {
        if node.element_index().is_some()
            && (self.style_record_affects_generated_content_state(self.computed_group_sets.assigned_style_record(node))
                || [pseudo_kind::BEFORE, pseudo_kind::AFTER, pseudo_kind::MARKER]
                    .into_iter()
                    .any(|kind| {
                        self.style_record_affects_generated_content_state(
                            self.computed_group_sets.pseudo_style_record(node, kind),
                        )
                    }))
        {
            return true;
        }
        self.tree
            .dom_children(node)
            .any(|child| self.subtree_affects_generated_content_state(child))
    }

    /// The value a winner's declaration was written with, and the declaration's index in its
    /// block: the drive computes from the spelling the declaration was written in, which the
    /// cascade's canonical identity may have rewritten. A rule keeps its written values beside its
    /// declarations, and an element's own declarations keep theirs beside the facts.
    fn written_winner_value(
        &self,
        node: StyleNodeID,
        winner: &PropertyWinner,
    ) -> Option<(
        usize,
        &crate::css::style_value::RetainedStyleValueData,
        WrittenValueChecks,
    )> {
        match winner.source {
            WinnerSource::Rule(rule) => self
                .program
                .written_winner_declaration(rule, winner.property, winner.important, winner.key.value)
                .map(|(index, value)| (index, value, self.program.written_value_checks(rule, index))),
            WinnerSource::Element(kind) => {
                // The host publishes an element's declarations complete and with the values they
                // were written with; only a replayed recording carries none, which the engine
                // cannot read.
                let (declared, _) = self.facts.element_declared_properties(node, kind);
                let written = self.facts.element_written_declared_values(node, kind);
                declared
                    .iter()
                    .rposition(|declared| {
                        declared.property == winner.property
                            && declared.important == winner.important
                            && declared.value == winner.key.value
                    })
                    .and_then(|index| {
                        Some((
                            index,
                            written.get(index)?,
                            self.facts.element_written_value_checks(node, kind, index),
                        ))
                    })
            }
            WinnerSource::ExactCascade => None,
        }
    }

    /// Whether a winner's declaration was written with a substitution the engine resolves itself:
    /// var() or inherit() references of its own, or a longhand pending a shorthand written with them. A value
    /// reading anything else - a custom function, an attribute, a style query - is C++'s, and what
    /// it computes to can move without any winner moving.
    #[cfg(any(test, feature = "style-recording"))]
    fn winner_is_written_with_substitution(&self, node: StyleNodeID, winner: &PropertyWinner) -> bool {
        let written = match winner.source {
            WinnerSource::Rule(rule) => self
                .program
                .written_winner_declaration(rule, winner.property, winner.important, winner.key.value)
                .map(|(_, value)| value),
            WinnerSource::Element(kind) => {
                let (declared, _) = self.facts.element_declared_properties(node, kind);
                let written = self.facts.element_written_declared_values(node, kind);
                if written.len() != declared.len() {
                    return false;
                }
                declared
                    .iter()
                    .rposition(|declared| {
                        declared.property == winner.property
                            && declared.important == winner.important
                            && declared.value == winner.key.value
                    })
                    .map(|index| &written[index])
            }
            WinnerSource::ExactCascade => None,
        };
        written.is_some_and(|value| match value.data() {
            unresolved @ crate::css::style_value::StyleValueData::Unresolved { .. } => {
                custom_property_cascade::value_is_engine_resolvable_substitution(unresolved)
            }
            crate::css::style_value::StyleValueData::PendingSubstitution {
                original_shorthand_value,
            } => custom_property_cascade::value_is_engine_resolvable_substitution(original_shorthand_value.data()),
            _ => false,
        })
    }

    /// The shorthand declared in a winner's block with the written value a pending longhand
    /// names: which shorthand it is, and that written value.
    fn underlying_shorthand_substitution(
        mut value: crate::css::style_value::RetainedStyleValueData,
    ) -> crate::css::style_value::RetainedStyleValueData {
        while let crate::css::style_value::StyleValueData::PendingSubstitution {
            original_shorthand_value,
        } = value.data()
        {
            value = original_shorthand_value.clone_retained();
        }
        value
    }

    fn shorthand_declaration_written_as(
        &self,
        node: StyleNodeID,
        source: WinnerSource,
        written_value: *const crate::css::style_value::StyleValueData,
    ) -> Option<(u16, crate::css::style_value::RetainedStyleValueData)> {
        let (declared, written): (&[DeclaredProperty], &[crate::css::style_value::RetainedStyleValueData]) =
            match source {
                WinnerSource::Rule(rule) => (
                    self.program.declared_properties_of(rule),
                    self.program.written_values_of(rule),
                ),
                WinnerSource::Element(kind) => (
                    self.facts.element_declared_properties(node, kind).0,
                    self.facts.element_written_declared_values(node, kind),
                ),
                WinnerSource::ExactCascade => return None,
            };
        if written.len() != declared.len() {
            return None;
        }
        let shorthand = declared.iter().zip(written).find(|(declared, written)| {
            declared.property < crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID
                && std::ptr::eq(written.pointer(), written_value)
        });
        if let Some((declared, written)) = shorthand {
            // A shorthand nested in another one (`border-width` in `border`) is itself pending the
            // outer shorthand's substitution; the substituted source parses under the outermost
            // shorthand's grammar.
            if let crate::css::style_value::StyleValueData::PendingSubstitution {
                original_shorthand_value,
            } = written.data()
                && let Some(outer) =
                    self.shorthand_declaration_written_as(node, source, original_shorthand_value.pointer())
            {
                return Some(outer);
            }
            let value = Self::underlying_shorthand_substitution(written.clone_retained());
            return Some((declared.property, value));
        }

        // Inline declarations retain the expanded pending longhands without a separate
        // shorthand declaration. Their shared original value and exact expansion identify the
        // shorthand whose grammar must parse the substituted source.
        let mut pending_longhands = Vec::new();
        let mut original = None;
        for (declared, written) in declared.iter().zip(written) {
            if let crate::css::style_value::StyleValueData::PendingSubstitution {
                original_shorthand_value,
            } = written.data()
                && std::ptr::eq(original_shorthand_value.pointer(), written_value)
            {
                pending_longhands.push(declared.property);
                original = Some(original_shorthand_value.clone_retained());
            }
        }
        pending_longhands.sort_unstable();
        pending_longhands.dedup();
        if pending_longhands.is_empty() {
            return None;
        }
        let shorthand = (crate::css::property_metadata::FIRST_SHORTHAND_PROPERTY_ID
            ..=crate::css::property_metadata::LAST_SHORTHAND_PROPERTY_ID)
            .find(|&candidate| {
                // A shorthand nested in another one (`border-width` in `border`) expands the
                // outer one into its leaf longhands.
                fn expand(property: u16, longhands: &mut Vec<u16>) {
                    for &longhand in crate::css::property_metadata::longhands_for_shorthand(property) {
                        if longhand < crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID {
                            expand(longhand, longhands);
                        } else {
                            longhands.push(longhand);
                        }
                    }
                }
                let mut expansion = Vec::new();
                expand(candidate, &mut expansion);
                expansion.sort_unstable();
                expansion.dedup();
                expansion == pending_longhands
            })?;
        Some((shorthand, Self::underlying_shorthand_substitution(original?)))
    }

    /// Whether any winner of a state was written with a substitution, so the record computed
    /// from it reads the node's custom-property environment.
    pub(super) fn state_has_substitutions(&self, node: StyleNodeID, state: CascadeStateID) -> bool {
        self.state_written_facts(node, state).has_substitutions
    }

    /// The condition and function dependencies of ordinary winners, including longhands that
    /// carry a pending shorthand substitution. The host uses these bits to schedule reactions
    /// when a function definition or one of the condition inputs changes.
    pub(super) fn state_custom_condition_usage(&self, node: StyleNodeID, state: CascadeStateID) -> u8 {
        self.state_written_facts(node, state).custom_condition_usage
    }

    pub(super) fn state_container_unit_mask(&self, node: StyleNodeID, state: CascadeStateID) -> u8 {
        self.state_written_facts(node, state)
            .container_relative_length_unit_mask
    }

    /// A tree-counting value reads its tree scope and sibling position. A substitution may
    /// produce one even when the written winner does not name it, so its cache key carries the
    /// position as well.
    pub(crate) fn state_tree_counting_key(&self, node: StyleNodeID, state: CascadeStateID) -> (u32, u64) {
        let facts = self.state_written_facts(node, state);
        if facts.has_written_tree_counting || facts.has_substitutions {
            (self.tree.tree_scope(node).0, self.element_tree_counting_inputs(node))
        } else {
            (0, 0)
        }
    }

    /// The environment `inherit()` reads for a node, as the node's parent holds it: the parent's
    /// computed custom properties, non-inheriting registrations included - the full environment
    /// the subject's own was resolved over. A pseudo-element's parent is its originating element,
    /// and the document element's is nothing.
    fn held_inheritance_environment(&self, node: StyleNodeID, pseudo_kind: Option<u8>) -> Option<u64> {
        if pseudo_kind.is_some() {
            return self.computed_group_sets.custom_property_environment_identity(node);
        }
        match self.record_inheritance_parent(node) {
            Some(parent) => self.computed_group_sets.custom_property_environment_identity(parent),
            None => Some(0),
        }
    }

    /// The cascade a winner state describes, as the drive consumes it: every winner's written
    /// value, seeded in cascade order so a logical property pair resolves the way it cascaded.
    #[allow(clippy::too_many_arguments)]
    fn cascaded_store_for_state(
        &mut self,
        node: StyleNodeID,
        state: CascadeStateID,
        pseudo_kind: Option<u8>,
        environment: u64,
        inheritance_environment: Option<u64>,
        inputs: &bridge::FfiDocumentStyleComputationInputs,
        substituted: &mut bool,
        counters: &mut Counters,
    ) -> WinnerStore {
        crate::css::ffi_stats::bump(crate::css::ffi_stats::FfiOp::WinnerStoreBuilds);
        // Seeded in cascade order, and within one rule in declaration order, since a logical
        // property and its physical associate resolve by order of appearance.
        let mut declarations = Vec::with_capacity(self.winner_groups.winner_count_in_state(state));
        // A parent declaring none has no inherited value, and `inherit()` takes its fallback. A
        // record's parent holds its environment before the record is driven.
        let inheritance_store = match inheritance_environment {
            Some(0) => std::ptr::null(),
            Some(identity) => self.custom_property_environments.store(identity).unwrap_or_else(|| {
                debug_assert!(false, "an inherited environment without a store");
                std::ptr::null()
            }),
            None => {
                debug_assert!(false, "a record driven before its parent holds an environment");
                std::ptr::null()
            }
        };
        for winner in self.winner_groups.winners_in_state(state).collect::<Vec<_>>() {
            // A revert whose continuation resumes at nothing leaves the property undeclared.
            let Some(winner) = self.winner_groups.resolved_winner(winner) else {
                continue;
            };
            // A pseudo-element's cascade keeps the properties its kind supports.
            if let Some(kind) = pseudo_kind
                && !crate::css::property_metadata::pseudo_element_supports_property(kind, winner.property)
            {
                continue;
            }
            // A shorthand written with a substitution is declared beside the longhands it
            // pends; those carry it.
            if winner.property < crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID {
                continue;
            }
            // A published winner always names a declaration written where it was published.
            let Some((index, value, checks)) = self.written_winner_value(node, &winner) else {
                debug_assert!(false, "a cascade winner without its written declaration");
                continue;
            };
            // A longhand declared through a shorthand keeps the whole shorthand as its written
            // value; the store takes the longhand's own part of it.
            let location = WinnerValue::Written {
                node,
                source: winner.source,
                index,
            };
            let (value, borrowed) = match value.data() {
                crate::css::style_value::StyleValueData::Shorthand { .. } => {
                    let Some(value) = shorthand_longhand_data(winner.property, value.data()) else {
                        debug_assert!(false, "a longhand declared through a shorthand that does not hold it");
                        continue;
                    };
                    (location, Some(value))
                }
                // A value with var() references substitutes under the node's environment, as the
                // C++ cascade substitutes it; a value invalid at computed-value time is unset.
                crate::css::style_value::StyleValueData::Unresolved { .. } => {
                    *substituted = true;
                    let value = value.clone_retained();
                    let value = self.substitute_written_value(
                        node,
                        pseudo_kind,
                        environment,
                        winner.property,
                        value,
                        inheritance_store,
                        *inputs,
                        counters,
                    );
                    (
                        WinnerValue::Substituted {
                            value: invalid_as_unset(value),
                            source: winner.source,
                        },
                        None,
                    )
                }
                // A longhand pending its shorthand's substitution takes its part of the
                // substituted shorthand.
                crate::css::style_value::StyleValueData::PendingSubstitution {
                    original_shorthand_value,
                } => {
                    *substituted = true;
                    let Some((shorthand, written)) =
                        self.shorthand_declaration_written_as(node, winner.source, original_shorthand_value.pointer())
                    else {
                        debug_assert!(false, "a pending longhand without the shorthand it pends");
                        continue;
                    };
                    let resolved = self.substitute_written_value(
                        node,
                        pseudo_kind,
                        environment,
                        shorthand,
                        written,
                        inheritance_store,
                        *inputs,
                        counters,
                    );
                    let value = match resolved.data() {
                        crate::css::style_value::StyleValueData::GuaranteedInvalid => unset_value(),
                        _ => expanded_longhand_value(shorthand, winner.property, &resolved)
                            .map_or_else(unset_value, invalid_as_unset),
                    };
                    (
                        WinnerValue::Substituted {
                            value,
                            source: winner.source,
                        },
                        None,
                    )
                }
                _ => (location, Some(value.data())),
            };
            let data = match &value {
                WinnerValue::Substituted { value, .. } => value.data(),
                WinnerValue::Written { .. } => borrowed.expect("written declaration is borrowed"),
            };
            // A `url()` resolves against the sheet its rule came from, written or substituted, or
            // the document's base URL for an element's own declaration.
            let resources_are_known = match &value {
                WinnerValue::Written {
                    source: WinnerSource::Rule(rule),
                    ..
                }
                | WinnerValue::Substituted {
                    source: WinnerSource::Rule(rule),
                    ..
                } => self
                    .rule_source_identity(*rule)
                    .is_some_and(|source| self.document_resource_contexts.for_source(source).is_some()),
                WinnerValue::Written {
                    source: WinnerSource::Element(_),
                    ..
                }
                | WinnerValue::Substituted {
                    source: WinnerSource::Element(_),
                    ..
                } => true,
                _ => false,
            };
            let context_free = checks
                .longhand_context_free
                .unwrap_or_else(|| value_computes_without_document_context(data))
                || (resources_are_known && value_computes_without_document_context_but_for_resources(data).is_some())
                || value_computes_with_random_inputs(data, resources_are_known)
                || (matches!(&value, WinnerValue::Written { .. })
                    && value_computes_with_container_inputs(data, resources_are_known))
                || (value_computes_with_tree_counting_inputs(data, resources_are_known)
                    && self.element_tree_counting_inputs(node) != 0);
            debug_assert!(context_free, "a cascade winner whose value needs document context");
            if matches!(value, WinnerValue::Substituted { .. }) {
                crate::css::ffi_stats::bump(crate::css::ffi_stats::FfiOp::WinnerStoreValueRetains);
            }
            declarations.push((
                winner.priority,
                index,
                WinnerDeclaration::new(winner.property, winner.important, value),
            ));
        }
        declarations.sort_by_key(|(priority, index, ..)| (*priority, *index));
        let store = WinnerStore::new(
            declarations
                .into_iter()
                .map(|(_, _, declaration)| declaration)
                .collect(),
        );
        if store.uses_tree_counting_function(self) {
            self.nodes_with_tree_counting_records.insert(node);
        }
        store
    }

    /// Whether every cascade winner that moved on this node since its record was computed is a
    /// longhand the engine computes itself. Such a change cannot reach the node's descendants:
    /// nothing inherited moves, and no custom property does, so a descendant's engine-computed
    /// record stays exact even though this ancestor changes in the same batch.
    pub(super) fn winner_delta_is_engine_confined(&self, node: StyleNodeID) -> bool {
        let target = computed::ComputedStyleTarget::new(node, u8::MAX);
        let Lookup::Known((generation, state)) = self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        else {
            return false;
        };
        let Some((previous_generation, previous_state)) = self.computed_group_sets.cascade_state(target) else {
            return false;
        };
        if previous_generation != generation {
            return false;
        }
        // A descendant's engine-computed record is assembled before C++ applies the ancestor, so
        // nothing the descendant inherits may move; custom properties inherit unless registered
        // otherwise, and a child's box-type transformation reads its parent's display.
        let mut moves_non_inherited = false;
        let confined = self
            .winner_groups
            .semantic_delta_properties(Some(previous_state), state)
            .all(|property| {
                moves_non_inherited = true;
                property != crate::css::property_metadata::property_id::CUSTOM
                    && property != crate::css::property_metadata::property_id::DISPLAY
                    && !crate::css::property_metadata::property_is_inherited(property)
            });
        // A child that explicitly inherits a non-inherited property inherits it like any other,
        // and passes it on to a descendant whose record is assembled from the child's.
        confined && !(moves_non_inherited && self.children_explicitly_inherit_non_inherited_properties(node))
    }

    /// Whether any of the node's children, as its descendants inherit through them, explicitly
    /// inherits a non-inherited property.
    fn children_explicitly_inherit_non_inherited_properties(&self, node: StyleNodeID) -> bool {
        let light_children = std::iter::successors(self.tree.first_element_child(node), |&child| {
            self.tree.next_element_sibling(child)
        })
        .filter(|&child| self.tree.assigned_slot_of(child).is_none());
        let shadow_children = std::iter::successors(
            self.tree
                .shadow_root_of(node)
                .and_then(|root| self.tree.first_element_child(root)),
            |&child| self.tree.next_element_sibling(child),
        );
        light_children
            .chain(shadow_children)
            .any(|child| self.node_explicitly_inherits_non_inherited_property(child))
    }

    /// Publish the immutable computed-group payloads of one element's base style. This assigns
    /// dense identities to shared payloads and their ordered tuple, so an equal handle proves equal
    /// groups and downstream operators can consume one node handle.
    pub(crate) fn publish_computed_groups(
        &mut self,
        target: computed::ComputedStyleTarget,
        payloads: &[SharedPayload],
        inherited_group_count: usize,
        custom_property_environment: u64,
        metadata_input: computed::ComputedMetadataInput<'_>,
        counters: &mut Counters,
    ) -> computed::ComputedGroupPublication {
        self.publish_computed_groups_impl(
            Some(target),
            payloads,
            inherited_group_count,
            custom_property_environment,
            metadata_input,
            computed::PendingRecordOwnership::default(),
            counters,
        )
    }

    pub(crate) fn assign_shared_style_record(
        &mut self,
        target: computed::ComputedStyleTarget,
        style_record: u64,
        inherited_group_count: usize,
        inherited_group_swap_eligible: bool,
        counters: &mut Counters,
    ) -> computed::ComputedGroupPublication {
        let group_count = self
            .computed_group_sets
            .style_record_payloads(style_record)
            .expect("a shared style record must name a live base record")
            .len();
        let current_cascade_state = self.computed_group_sets.take_pending_cascade_state(target);
        let publication = self
            .computed_group_sets
            .assign_shared_style_record(
                target,
                style_record,
                inherited_group_count,
                inherited_group_swap_eligible,
            )
            .expect("a shared style record must name a live base record");
        if let Some(current_cascade_state) = current_cascade_state {
            self.bind_published_cascade_state(target, current_cascade_state, publication.node_handle_changed, counters);
        } else {
            self.computed_group_sets.clear_cascade_state(target);
        }
        self.settle_computed_memory();
        counters.add(Counter::ComputedGroupsReused, group_count as u64);
        self.note_identity_mints(counters);
        counters.bump(Counter::ComputedGroupSetsReused);
        counters.bump(Counter::InheritedGroupSetsReused);
        counters.bump(Counter::CustomPropertyEnvironmentsReused);
        counters.bump(Counter::ComputedFixedMetadataReused);
        counters.bump(Counter::StyleRecordsReused);
        if publication.node_handle_changed {
            counters.bump(Counter::ComputedGroupNodeHandlesPublished);
        }
        if publication.inherited_node_handle_changed {
            counters.bump(Counter::InheritedGroupNodeHandlesPublished);
        }
        if publication.custom_property_environment_node_handle_changed {
            counters.bump(Counter::CustomPropertyEnvironmentNodeHandlesPublished);
        }
        if publication.computed_fixed_metadata_node_handle_changed {
            counters.bump(Counter::ComputedFixedMetadataNodeHandlesPublished);
        }
        if publication.style_record_node_handle_changed {
            counters.bump(Counter::StyleRecordNodeHandlesPublished);
        }
        if publication.animation_overlay_slot_released {
            counters.bump(Counter::AnimationOverlaySlotsReleased);
        }
        counters.set(
            Counter::LiveAnimationOverlayRecords,
            publication.live_animation_overlay_records as u64,
        );
        if publication.is_pseudo && publication.style_record_node_handle_changed {
            counters.bump(Counter::ComputedPseudoAssignmentsPublished);
        }
        publication
    }

    /// Holds group identity to payload addresses for the span of a C++ verification pass, so
    /// that the second copy of a record it interns to check the first decides nothing.
    pub(crate) fn suspend_computed_group_content_identities(&mut self, suspended: bool) {
        self.computed_group_sets.set_content_identities_suspended(suspended);
    }

    /// Intern the immutable computed-group payloads of a style which has no live StyleEngine target.
    pub(crate) fn intern_computed_groups(
        &mut self,
        payloads: &[SharedPayload],
        inherited_group_count: usize,
        custom_property_environment: u64,
        metadata_input: computed::ComputedMetadataInput<'_>,
        counters: &mut Counters,
    ) -> computed::ComputedGroupPublication {
        self.publish_computed_groups_impl(
            None,
            payloads,
            inherited_group_count,
            custom_property_environment,
            metadata_input,
            computed::PendingRecordOwnership::default(),
            counters,
        )
    }

    pub(crate) fn style_record_payloads(&self, style_record: u64) -> Option<&[SharedPayload]> {
        self.style_record_payload_owner(style_record)
            .map(|payloads| payloads.as_slice())
    }

    /// The record as a value that owns everything a read of it reads, or `None` for a record the
    /// engine no longer holds; see [`super::published_record`].
    pub(crate) fn publish_style_record(
        &self,
        style_record: u64,
    ) -> Option<std::sync::Arc<super::published_record::PublishedStyleRecord>> {
        if !computed::ComputedGroupSets::record_is_animation_overlay(style_record)
            && !self.computed_group_sets.final_style_record_is_live(style_record)
        {
            return None;
        }
        self.computed_group_sets.publish_style_record(style_record)
    }

    /// The record's payloads as the value that owns them; see
    /// [`computed::ComputedGroupSets::style_record_payload_owner`].
    pub(crate) fn style_record_payload_owner(
        &self,
        style_record: u64,
    ) -> Option<&std::sync::Arc<super::record_payloads::StyleRecordPayloads>> {
        if !computed::ComputedGroupSets::record_is_animation_overlay(style_record)
            && !self.computed_group_sets.final_style_record_is_live(style_record)
        {
            return None;
        }
        self.computed_group_sets.style_record_payload_owner(style_record)
    }

    pub(crate) fn style_record_dependency_flags(&self, style_record: u64) -> Option<u8> {
        self.computed_group_sets.style_record_dependency_flags(style_record)
    }

    pub(crate) fn recording_computed_group_identities(&self, style_record: u64) -> Option<Vec<u32>> {
        #[cfg(feature = "style-recording")]
        return self.computed_group_sets.recording_group_identities(style_record);
        #[cfg(not(feature = "style-recording"))]
        {
            let _ = style_record;
            None
        }
    }

    pub(crate) fn recording_computed_group_retained_bytes(&self, style_record: u64) -> Option<Vec<u64>> {
        #[cfg(feature = "style-recording")]
        return self.computed_group_sets.recording_group_retained_bytes(style_record);
        #[cfg(not(feature = "style-recording"))]
        {
            let _ = style_record;
            None
        }
    }

    pub(crate) fn recording_computed_longhand_table(&self, style_record: u64) -> Option<(u32, &[SharedPayload])> {
        #[cfg(feature = "style-recording")]
        return self.computed_group_sets.recording_longhand_table(style_record);
        #[cfg(not(feature = "style-recording"))]
        {
            let _ = style_record;
            None
        }
    }

    pub(crate) fn style_record_view(&self, style_record: u64) -> Option<computed::StyleRecordView<'_>> {
        self.computed_group_sets.style_record_view(style_record)
    }

    pub(crate) fn pin_style_record(&mut self, style_record: u64) {
        self.computed_group_sets.pin_style_record(style_record);
    }

    pub(crate) fn begin_style_record_view_epoch(&mut self) {
        self.computed_group_sets.begin_style_record_view_epoch();
    }

    /// Publishes how many identities each catalog has minted. These count the sharing partition
    /// a run produces; the reuse counters beside them credit whichever publication interned an
    /// identity first, which is an execution-order decision.
    fn note_identity_mints(&mut self, counters: &mut Counters) {
        let mints = self.computed_group_sets.identity_mints();
        counters.set(Counter::ComputedGroupIdentitiesMinted, mints.groups);
        counters.set(Counter::ComputedGroupSetIdentitiesMinted, mints.group_sets);
        counters.set(Counter::InheritedGroupSetIdentitiesMinted, mints.inherited_group_sets);
        counters.set(Counter::StyleRecordIdentitiesMinted, mints.style_records);
    }

    pub(super) fn settle_computed_memory(&mut self) {
        self.computed_group_sets.settle_nested_memory(&mut self.memory);
        self.computed_group_set_memory.resize_required_to(
            &mut self.memory,
            self.computed_group_sets.group_set_header_capacity_bytes(),
        );
        self.custom_property_environment_memory.resize_required_to(
            &mut self.memory,
            self.computed_group_sets.custom_property_environment_capacity_bytes()
                + self.custom_property_environments.capacity_bytes(),
        );
        self.computed_fixed_metadata_memory.resize_required_to(
            &mut self.memory,
            self.computed_group_sets.computed_fixed_metadata_capacity_bytes(),
        );
        self.computed_longhand_table_memory.resize_required_to(
            &mut self.memory,
            self.computed_group_sets.longhand_table_header_capacity_bytes(),
        );
        self.style_record_memory
            .resize_required_to(&mut self.memory, self.computed_group_sets.style_record_capacity_bytes());
        self.animation_overlay_memory.resize_required_to(
            &mut self.memory,
            self.computed_group_sets.animation_overlay_header_capacity_bytes(),
        );
        self.computed_pseudo_assignment_memory.resize_required_to(
            &mut self.memory,
            self.computed_group_sets.pseudo_assignment_header_capacity_bytes(),
        );
    }

    pub(crate) fn style_record_is_unassigned_animation_overlay(&self, style_record: u64) -> bool {
        self.computed_group_sets
            .style_record_is_unassigned_animation_overlay(style_record)
    }

    pub(crate) fn unpin_style_record(&mut self, style_record: u64) {
        self.computed_group_sets.unpin_style_record(style_record);
    }

    fn bind_published_cascade_state(
        &mut self,
        target: computed::ComputedStyleTarget,
        (current_generation, current_cascade_state): (u64, CascadeStateID),
        node_handle_changed: bool,
        counters: &mut Counters,
    ) {
        let previous_cascade_state = self
            .computed_group_sets
            .bind_cascade_state(target, (current_generation, current_cascade_state))
            .and_then(|(previous_generation, previous_state)| {
                (previous_generation == current_generation).then_some(previous_state)
            });
        let delta = self
            .winner_groups
            .semantic_delta(previous_cascade_state, current_cascade_state);
        if delta.is_empty() {
            counters.bump(Counter::CascadeWinnerDeltaStops);
            return;
        }
        counters.add(Counter::CascadeWinnerDeltaProperties, delta.properties().len() as u64);
        counters.add(
            Counter::ComputedWinnerDeltaPropertiesConsumed,
            delta.properties().len() as u64,
        );
        if !node_handle_changed {
            counters.bump(Counter::ComputedWinnerPropagationStops);
        }
    }

    #[allow(clippy::too_many_arguments)]
    /// `owned` says which payloads, and whether the table, the caller hands to the catalog; the
    /// publication says which of those references the catalog took.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn publish_computed_groups_impl(
        &mut self,
        target: Option<computed::ComputedStyleTarget>,
        payloads: &[SharedPayload],
        inherited_group_count: usize,
        custom_property_environment: u64,
        metadata_input: computed::ComputedMetadataInput<'_>,
        owned: computed::PendingRecordOwnership,
        counters: &mut Counters,
    ) -> computed::ComputedGroupPublication {
        let current_cascade_state =
            target.and_then(|target| self.computed_group_sets.take_pending_cascade_state(target));
        let is_base_record = metadata_input.animation_overlay_identity == 0;
        let pseudo_styles = metadata_input.pseudo_element_styles;
        let publication = self.computed_group_sets.publish(
            target,
            payloads,
            inherited_group_count,
            custom_property_environment,
            metadata_input,
            owned,
        );
        if let Some(target) = target
            && !target.is_pseudo()
            && self.computed_group_sets.adjustment_facts(target.node())
                & bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT
                != 0
        {
            self.prepare_root_font_metrics_from_record(publication.style_record_identity);
        }
        if let Some(current_cascade_state) = current_cascade_state {
            let target = target.expect("only a target has pending cascade state");
            self.bind_published_cascade_state(target, current_cascade_state, publication.node_handle_changed, counters);
            self.remember_cold_record_candidate(
                target,
                current_cascade_state,
                custom_property_environment,
                pseudo_styles,
                publication.previous_style_record_identity,
                publication.style_record_identity,
                is_base_record,
            );
        } else if let Some(target) = target {
            self.computed_group_sets.clear_cascade_state(target);
        }
        self.settle_computed_memory();
        counters.add(
            Counter::ComputedOutputGroupsCanonicalized,
            publication.canonical_output_groups_reused as u64,
        );
        counters.add(
            Counter::ComputedGroupsReused,
            (payloads.len() - publication.new_groups) as u64,
        );
        self.note_identity_mints(counters);
        if !publication.new_group_set {
            counters.bump(Counter::ComputedGroupSetsReused);
        }
        if !publication.new_inherited_group_set {
            counters.bump(Counter::InheritedGroupSetsReused);
        }
        if publication.node_handle_changed {
            counters.bump(Counter::ComputedGroupNodeHandlesPublished);
        }
        if publication.inherited_node_handle_changed {
            counters.bump(Counter::InheritedGroupNodeHandlesPublished);
        }
        if !publication.new_custom_property_environment {
            counters.bump(Counter::CustomPropertyEnvironmentsReused);
        }
        if publication.custom_property_environment_node_handle_changed {
            counters.bump(Counter::CustomPropertyEnvironmentNodeHandlesPublished);
        }
        match publication.new_computed_fixed_metadata {
            true => counters.bump(Counter::ComputedFixedMetadataInterned),
            false => counters.bump(Counter::ComputedFixedMetadataReused),
        }
        if publication.computed_fixed_metadata_node_handle_changed {
            counters.bump(Counter::ComputedFixedMetadataNodeHandlesPublished);
        }
        match publication.new_style_record {
            true => counters.bump(Counter::StyleRecordsInterned),
            false => counters.bump(Counter::StyleRecordsReused),
        }
        if publication.style_record_node_handle_changed {
            counters.bump(Counter::StyleRecordNodeHandlesPublished);
        }
        if publication.animation_overlay_slot_allocated {
            counters.bump(Counter::AnimationOverlaySlotsAllocated);
        }
        if publication.animation_overlay_slot_released {
            counters.bump(Counter::AnimationOverlaySlotsReleased);
        }
        if publication.animation_overlay_record_updated {
            counters.bump(Counter::AnimationOverlayRecordsUpdated);
        }
        counters.set(
            Counter::LiveAnimationOverlayRecords,
            publication.live_animation_overlay_records as u64,
        );
        if publication.is_pseudo && publication.style_record_node_handle_changed {
            counters.bump(Counter::ComputedPseudoAssignmentsPublished);
        }
        publication
    }

    #[cfg(any(test, feature = "style-recording"))]
    pub(crate) fn exact_cascade_generation_snapshot(
        &self,
        target: computed::ComputedStyleTarget,
    ) -> (u64, Option<u64>) {
        (
            self.winner_groups.generation(),
            self.computed_group_sets
                .cascade_state(target)
                .map(|(generation, _)| generation),
        )
    }

    #[cfg(feature = "style-recording")]
    pub(crate) fn publish_exact_cascade_winners(
        &mut self,
        target: computed::ComputedStyleTarget,
        exact_winners: &[(u16, SpecifiedWinnerKey)],
        inherited_style_groups: u8,
        donor: Option<ExactCascadeDonor>,
        counters: &mut Counters,
    ) -> (bridge::FfiExactCascadePublication, bool) {
        let context = self.prepare_exact_cascade_publication(target, donor);
        let had_previous = context.previous.is_some();
        (
            self.publish_exact_cascade_winners_with_context(
                target,
                exact_winners,
                inherited_style_groups,
                context,
                counters,
            ),
            had_previous,
        )
    }

    #[cfg(any(test, feature = "style-recording"))]
    pub(super) fn prepare_exact_cascade_publication(
        &mut self,
        target: computed::ComputedStyleTarget,
        donor: Option<ExactCascadeDonor>,
    ) -> ExactCascadeContext {
        let generation = self.winner_groups.generation();
        let mut previous =
            self.computed_group_sets
                .cascade_state(target)
                .and_then(|(previous_generation, previous_state)| {
                    (previous_generation == generation).then_some(previous_state)
                });
        let mut dependency_target = target;
        let mut donor_used = false;
        if !target.is_pseudo()
            && let Some(donor) = donor
            && self
                .computed_group_sets
                .assigned_style_record(donor.node)
                .is_some_and(|record| record.raw() == donor.style_record)
        {
            let donor_target = computed::ComputedStyleTarget::new(donor.node, u8::MAX);
            if let Some(state) = self
                .computed_group_sets
                .cascade_state(donor_target)
                .and_then(|(donor_generation, donor_state)| (donor_generation == generation).then_some(donor_state))
            {
                previous = Some(state);
                dependency_target = donor_target;
                donor_used = true;
            }
        }
        let winner_key = target.pseudo_element_target().map_or_else(
            || WinnerGroupKey::current(target.node(), self.program.version()),
            |pseudo| WinnerGroupKey::current_pseudo(target.node(), pseudo, self.program.version()),
        );
        let lower_bound_state = self
            .current_winner_groups()
            .token_for(winner_key)
            .sparse()
            .ok()
            .map(|(_, state)| state);
        if target.is_pseudo() {
            self.computed_group_sets
                .observe_pseudo_retained_cascade_state(target, lower_bound_state.map(|state| (generation, state)));
            self.settle_computed_memory();
        }
        ExactCascadeContext {
            previous,
            lower_bound_state,
            dependency_target,
            donor_used,
        }
    }

    #[cfg(any(test, feature = "style-recording"))]
    pub(super) fn publish_exact_cascade_winners_with_context(
        &mut self,
        target: computed::ComputedStyleTarget,
        exact_winners: &[(u16, SpecifiedWinnerKey)],
        inherited_style_groups: u8,
        context: ExactCascadeContext,
        counters: &mut Counters,
    ) -> bridge::FfiExactCascadePublication {
        let ExactCascadeContext {
            previous,
            lower_bound_state,
            dependency_target,
            donor_used,
        } = context;
        let mut winners = Vec::with_capacity(exact_winners.len());
        for &(property, key) in exact_winners {
            // The engine's own winner stands for the exact one when it is the same declaration,
            // or a declaration written with a substitution: the exact value is what that
            // substitutes to, which the engine computes for itself.
            let lower_bound_winner = lower_bound_state
                .and_then(|state| self.winner_groups.winner_in_state(state, property))
                .filter(|winner| {
                    self.winner_groups.resolved_winner(*winner).is_some_and(|resolved| {
                        (resolved.key == key || self.winner_is_written_with_substitution(target.node(), &resolved))
                            && matches!(self.specified_values.value(resolved.key.value), Lookup::Known(_))
                    })
                });
            winners.push(lower_bound_winner.unwrap_or(PropertyWinner {
                property,
                important: key.important,
                key,
                priority: CascadePriority::exact_output_placeholder(),
                source: WinnerSource::ExactCascade,
            }));
        }
        let state = self.intern_cascade_state(&winners, previous, counters);
        self.winner_groups.settle_memory(&mut self.memory);
        let generation = self.winner_groups.generation();
        let delta = self.winner_groups.semantic_delta(previous, state);
        let unchanged = previous.is_some() && delta.is_empty() && !donor_used;
        let current_color_dependency_mask = delta
            .properties()
            .contains(&crate::css::property_metadata::property_id::COLOR)
            .then(|| {
                self.computed_group_sets
                    .current_color_dependency_mask(dependency_target)
            });
        let color_scheme_dependency_mask = delta
            .properties()
            .contains(&crate::css::property_metadata::property_id::COLOR_SCHEME)
            .then(|| self.computed_group_sets.color_scheme_dependency_mask(dependency_target));
        const INHERITED_FONT_GROUP: u8 = 1 << 6;
        let font_group_mask = computed_group_output_mask(crate::css::property_metadata::property_id::FONT_SIZE);
        let font_dependency_mask = font_group_mask.and_then(|font_group_mask| {
            (inherited_style_groups & INHERITED_FONT_GROUP != 0
                || delta
                    .properties()
                    .iter()
                    .copied()
                    .any(|property| computed_group_output_mask(property) == Some(font_group_mask)))
            .then(|| self.computed_group_sets.font_dependency_mask(dependency_target))
        });
        let font_dependency_properties = font_group_mask.and_then(|font_group_mask| {
            (inherited_style_groups & INHERITED_FONT_GROUP != 0
                || delta
                    .properties()
                    .iter()
                    .copied()
                    .any(|property| computed_group_output_mask(property) == Some(font_group_mask)))
            .then(|| self.computed_group_sets.font_dependency_properties(dependency_target))
        });
        const INHERITED_STATIC_GROUPS: u8 = (1 << 0) | (1 << 1) | (1 << 3);
        const INHERITED_UI_GROUP: u8 = 1 << 2;
        const INHERITED_TEXT_GROUP: u8 = 1 << 4;
        const INHERITED_GROUPS_WITH_COMPUTED_CLOSURE: u8 =
            INHERITED_STATIC_GROUPS | INHERITED_UI_GROUP | INHERITED_TEXT_GROUP | INHERITED_FONT_GROUP;
        let inherited_property_closure_requested = delta.is_empty()
            && inherited_style_groups != 0
            && inherited_style_groups & !INHERITED_GROUPS_WITH_COMPUTED_CLOSURE == 0;
        let inherited_current_color_dependency_mask =
            (inherited_property_closure_requested && inherited_style_groups & INHERITED_TEXT_GROUP != 0).then(|| {
                self.computed_group_sets
                    .current_color_dependency_mask(dependency_target)
            });
        let inherited_current_color_dependency_properties =
            (inherited_property_closure_requested && inherited_style_groups & INHERITED_TEXT_GROUP != 0).then(|| {
                self.computed_group_sets
                    .current_color_dependency_properties(dependency_target)
            });
        let inherited_color_scheme_dependency_mask = (inherited_property_closure_requested
            && inherited_style_groups & INHERITED_UI_GROUP != 0)
            .then(|| self.computed_group_sets.color_scheme_dependency_mask(dependency_target));
        let inherited_color_scheme_dependency_properties =
            (inherited_property_closure_requested && inherited_style_groups & INHERITED_UI_GROUP != 0).then(|| {
                self.computed_group_sets
                    .color_scheme_dependency_properties(dependency_target)
            });
        let inherited_property_closure_is_exact = inherited_property_closure_requested
            && (inherited_style_groups & INHERITED_TEXT_GROUP == 0
                || inherited_current_color_dependency_properties.is_some_and(|properties| properties.is_some()))
            && (inherited_style_groups & INHERITED_UI_GROUP == 0
                || inherited_color_scheme_dependency_properties.is_some_and(|properties| properties.is_some()))
            && (inherited_style_groups & INHERITED_FONT_GROUP == 0
                || font_dependency_properties.is_some_and(|properties| properties.is_some()));
        let inherited_computed_group_mask = if inherited_property_closure_is_exact {
            [
                inherited_current_color_dependency_mask,
                inherited_color_scheme_dependency_mask,
                font_dependency_mask,
            ]
            .into_iter()
            .flatten()
            .flatten()
            .fold(u32::from(inherited_style_groups), |mask, dependencies| {
                mask | dependencies
            })
        } else {
            0
        };
        let computed_group_mask = previous.map_or(u32::MAX, |_| {
            // NB: Inherited groups the closure cannot answer exactly forbid narrowing: the caller
            //     passes every changed inherited group whenever its group swap may decline, and a
            //     mask that silently dropped an unanswered group would let the masked recompute
            //     keep stale values for it.
            if inherited_style_groups != 0 && !inherited_property_closure_is_exact {
                return u32::MAX;
            }
            delta
                .properties()
                .iter()
                .copied()
                .try_fold(0, |mask, property| {
                    let groups = computed_group_dependency_mask(property)?;
                    let dynamic_groups = if property == crate::css::property_metadata::property_id::COLOR {
                        current_color_dependency_mask.flatten()?
                            | computed_group_output_mask(crate::css::property_metadata::property_id::CARET_COLOR)
                                .unwrap_or(u32::MAX)
                    } else if property == crate::css::property_metadata::property_id::COLOR_SCHEME {
                        color_scheme_dependency_mask.flatten()?
                    } else if Some(groups) == font_group_mask {
                        font_dependency_mask.flatten()?
                    } else {
                        0
                    };
                    Some(mask | groups | dynamic_groups)
                })
                .unwrap_or(u32::MAX)
                | inherited_computed_group_mask
        });
        self.computed_group_sets
            .set_pending_cascade_state(target, (generation, state));
        bridge::FfiExactCascadePublication {
            unchanged,
            computed_group_mask,
            donor_used,
        }
    }

    pub(crate) fn current_color_dependent_group_mask(&self, node: StyleNodeID, pseudo_kind: u8) -> Option<u32> {
        let target = computed::ComputedStyleTarget::new(node, pseudo_kind);
        let dependencies = self.computed_group_sets.current_color_dependency_mask(target)?;
        let caret_color_group = computed_group_output_mask(crate::css::property_metadata::property_id::CARET_COLOR)?;
        let accent_color_group = computed_group_output_mask(crate::css::property_metadata::property_id::ACCENT_COLOR)?;
        Some(dependencies | caret_color_group | accent_color_group)
    }

    /// Carry the exact winner state behind a style an element hands back unchanged into its next
    /// publication. Reuse is granted only when the record's inputs are the ones the cascade ran
    /// on, so the state bound by that cascade still describes the element's winners.
    pub(crate) fn retain_exact_cascade_state(&mut self, node: StyleNodeID) {
        let target = computed::ComputedStyleTarget::new(node, u8::MAX);
        let Some(bound) = self.computed_group_sets.cascade_state(target) else {
            return;
        };
        // A re-evaluation can mint a new state for the same winners; the reused style then carries
        // the current one, so the next publication finds the winners it describes.
        let Some(retained) = self.current_state_for_bound(node, bound) else {
            return;
        };
        // A state bound before the winner groups were evicted describes nothing the table holds
        // now; the next publication binds a fresh one.
        if retained.0 != self.winner_groups.generation() {
            return;
        }
        self.computed_group_sets.set_pending_cascade_state(target, retained);
    }

    /// The winner state the engine holds for a node now, when it describes the same winners as the
    /// state the node's last cascade bound: the bound state itself, or one minted since for
    /// winners that did not move. `None` when the winners moved or the current state is unknown.
    fn current_state_for_bound(
        &self,
        node: StyleNodeID,
        bound: (u64, CascadeStateID),
    ) -> Option<(u64, CascadeStateID)> {
        match self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        {
            Lookup::Known(current) if current == bound => Some(current),
            Lookup::Known(current)
                if current.0 == bound.0 && self.winner_groups.semantic_delta(Some(bound.1), current.1).is_empty() =>
            {
                Some(current)
            }
            Lookup::Known(_) => None,
            _ => Some(bound),
        }
    }

    /// Whether the winner state an element's last cascade bound is still the one its current
    /// winners describe. A style handed back unchanged may only carry that state forward when it is.
    pub(crate) fn exact_cascade_state_is_current(&self, node: StyleNodeID) -> bool {
        let target = computed::ComputedStyleTarget::new(node, u8::MAX);
        let Some(bound) = self.computed_group_sets.cascade_state(target) else {
            return false;
        };
        self.current_state_for_bound(node, bound).is_some()
    }

    /// Bind an exact winner state to a style-sharing publication which consumes the same complete
    /// cascade input without running the C++ cascade.
    pub(crate) fn prepare_shared_exact_cascade_state(&mut self, node: StyleNodeID) {
        let published_answer_is_complete = self
            .current_published_answer(node)
            .is_some_and(|answer| answer.cascade_winners_are_complete);
        let retained_answer_is_complete = || {
            self.batch_matching_traversal.as_ref()?;
            let retained = match self.retained_match_answer(node) {
                Lookup::Known(answer) => answer,
                Lookup::KnownAbsent | Lookup::Missing(_) => return None,
            };
            // Completeness depends only on rule inventory and scope. Cascade rank,
            // specificity, and the materialized node never participate.
            for entry in retained.iter() {
                self.programs.get(entry.program).entries().get(entry.entry as usize)?;
                if self.program.rule_is_gated_by_container_query(entry.rule)
                    || !self.match_scope_is_complete_for(Some(node), entry.rule, entry.tree_scope)
                {
                    return Some(false);
                }
            }
            Some(
                ElementDeclarationKind::ALL
                    .iter()
                    .all(|&kind| self.facts.element_declared_properties(node, kind).1),
            )
        };
        if !published_answer_is_complete && retained_answer_is_complete() != Some(true) {
            return;
        }
        let (generation, state) = match self
            .current_winner_groups()
            .token_for(WinnerGroupKey::current(node, self.program.version()))
        {
            Lookup::Known(state) => state,
            Lookup::KnownAbsent | Lookup::Missing(_) => return,
        };
        self.computed_group_sets
            .set_pending_cascade_state(computed::ComputedStyleTarget::new(node, u8::MAX), (generation, state));
    }

    pub(crate) fn discard_pending_exact_cascade_state(&mut self, target: computed::ComputedStyleTarget) {
        self.computed_group_sets.take_pending_cascade_state(target);
    }

    pub(crate) fn remove_computed_pseudo(
        &mut self,
        node: StyleNodeID,
        pseudo_kind: u8,
        counters: &mut Counters,
    ) -> Option<computed::FinalStyleRecordID> {
        let target = computed::ComputedStyleTarget::new(node, pseudo_kind);
        if let Some(state) = self.computed_group_sets.take_pending_cascade_state(target) {
            self.computed_group_sets
                .observe_absent_pseudo_cascade_state(target, state);
        }
        let live_animation_overlays_before = self.computed_group_sets.live_animation_overlay_records();
        let removed_style_record = self.computed_group_sets.remove_pseudo(node, pseudo_kind);
        let live_animation_overlays_after = self.computed_group_sets.live_animation_overlay_records();
        self.settle_computed_memory();
        let removed_style_record = removed_style_record?;
        counters.bump(Counter::ComputedPseudoAssignmentsRemoved);
        counters.bump(Counter::StyleRecordNodeHandlesPublished);
        counters.add(
            Counter::AnimationOverlaySlotsReleased,
            (live_animation_overlays_before - live_animation_overlays_after) as u64,
        );
        counters.set(
            Counter::LiveAnimationOverlayRecords,
            live_animation_overlays_after as u64,
        );
        Some(removed_style_record)
    }
}

impl StyleEngineState {
    fn restore_private_ancestor_records(
        &mut self,
        records: Vec<(
            StyleNodeID,
            computed::FinalStyleRecordID,
            computed::FinalStyleRecordID,
            Option<tree::ContainerQueryInputRow>,
        )>,
    ) {
        for (node, private, previous, inputs) in records.into_iter().rev() {
            self.computed_group_sets
                .revert_engine_computed_record(node, private, previous);
            match inputs {
                Some(row) => self.container_query_inputs.set(node, row),
                None => self.container_query_inputs.clear(node),
            }
        }
    }

    /// The record of an element no rule reaches, such as one outside the document: the cascade of
    /// its own declarations alone, in cascade order, over the initial values. The element has no
    /// style node, so the drive is keyed by `subject`, the document's, which names no parent and
    /// no siblings. A value that would substitute has no environment to substitute from here, and
    /// is unset. Returns the record pinned for the caller.
    pub(crate) fn declared_only_record(
        &mut self,
        subject: StyleNodeID,
        facts: u32,
        declarations: &[(ElementDeclarationKind, &crate::css::declaration_block::DeclaredProperty)],
        counters: &mut Counters,
    ) -> Option<computed::FinalStyleRecordID> {
        use crate::css::style_value::{RetainedStyleValueData, StyleValueData, retain_style_value};
        // An engine no document hosts computes no records.
        if !self.computes_records() {
            return None;
        }
        let inputs = self.document_style_computation_inputs;
        let retained = |value: *const StyleValueData| unsafe {
            RetainedStyleValueData::from_retained_pointer(retain_style_value(value))
        };
        let mut winners: Vec<WinnerDeclaration> = Vec::with_capacity(declarations.len());
        for important in [false, true] {
            for &(kind, declaration) in declarations {
                let property = declaration.property_id;
                if declaration.important != important
                    || property < crate::css::property_metadata::FIRST_LONGHAND_PROPERTY_ID
                {
                    continue;
                }
                let value = match declaration.value.as_ref() {
                    data @ StyleValueData::Shorthand { .. } => match shorthand_longhand_data(property, data) {
                        Some(longhand) => retained(longhand),
                        None => continue,
                    },
                    StyleValueData::Unresolved { .. } | StyleValueData::PendingSubstitution { .. } => unset_value(),
                    data => retained(data),
                };
                winners.retain(|winner| winner.property != property);
                winners.push(WinnerDeclaration::new(
                    property,
                    important,
                    WinnerValue::Substituted {
                        value: invalid_as_unset(value),
                        source: WinnerSource::Element(kind),
                    },
                ));
            }
        }
        let store = WinnerStore::new(winners);
        let target = computed::ComputedStyleTarget::new(subject, u8::MAX);
        let drive_subject = DriveSubject {
            target,
            parent: None,
            facts: facts & !bridge::element_adjustment_fact::IS_DOCUMENT_ELEMENT,
            highlight_parent: None,
        };
        let mut scratch = EngineComputedRecordScratch::default();
        let driven = loop {
            match self.engine_full_drive(
                drive_subject,
                None,
                &store,
                &inputs,
                &mut scratch.font_drive,
                FontDriveGoal::Complete,
                false,
                &mut 0,
                counters,
            ) {
                Ok(FullDrive::Driven(driven)) => break driven,
                Ok(FullDrive::AwaitsRegisteredContext(_) | FullDrive::RootInputs(_)) => {
                    unreachable!("a complete drive without registered declarations finishes")
                }
                Err(Unanswered::Suspended(suspension)) => {
                    self.refill_suspension(suspension, None, &mut scratch.font_drive, counters);
                }
            }
        };
        let (table, length, _, font) = driven;
        let font = font.expect("a full drive resolves the font");
        let (record, _) = self
            .assemble_and_publish_engine_record(target, false, None, table, &length, &font, 0, 0, 0, None, counters);
        self.computed_group_sets.pin_style_record(record.raw());
        Some(record)
    }

    /// Answer an observation of one node without draining the document's transaction. A
    /// read-only observation uses a retained match answer or a private matching traversal and
    /// leaves the published winner rows and invalidation facts untouched.
    ///
    /// Every demand is answered: the row's record, or a pseudo-element that generates no box.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn answer_record_demand(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        exclude_inline_style: bool,
        targeted: bool,
        read_only: bool,
        parent_highlight: u64,
        counters: &mut Counters,
    ) -> RecordDemandAnswer {
        match self.answer_or_decline_record_demand(
            node,
            pseudo,
            exclude_inline_style,
            targeted,
            read_only,
            parent_highlight,
            counters,
        ) {
            Ok(Some(record)) => RecordDemandAnswer::Record(record),
            Ok(None) => RecordDemandAnswer::Absent,
            Err(cause) => self.declined_record_demand_fallback(node, pseudo, read_only, cause, counters),
        }
    }

    /// A demand the engine declined still answers. None does: each decline is an input the
    /// demand's caller owes first. Should one happen, the row keeps the record it has; an element
    /// without one takes the initial values.
    fn declined_record_demand_fallback(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        read_only: bool,
        cause: &'static str,
        counters: &mut Counters,
    ) -> RecordDemandAnswer {
        debug_assert!(false, "a record demand declined ({cause})");
        let installed = match pseudo {
            Some(kind) => self.computed_group_sets.pseudo_style_record(node, kind),
            None => self.computed_group_sets.assigned_style_record(node),
        };
        let answer = |record: computed::FinalStyleRecordID| {
            RecordDemandAnswer::Record(RetriedEngineRecord {
                style_record: record.raw(),
                ..RetriedEngineRecord::default()
            })
        };
        if pseudo.is_some() {
            return installed.map_or(RecordDemandAnswer::Absent, answer);
        }
        let record = match installed {
            Some(record) if !read_only => return answer(record),
            Some(record) => {
                self.computed_group_sets.pin_style_record(record.raw());
                record
            }
            None => {
                let mut document = node;
                while let Some(parent) = self.tree.parent(document) {
                    document = parent;
                }
                let facts = self.computed_group_sets.adjustment_facts(node);
                let Some(record) = self.declared_only_record(document, facts, &[], counters) else {
                    return RecordDemandAnswer::Absent;
                };
                record
            }
        };
        // A private answer is pinned where the demand keeps its answers; a published one is the
        // row's record.
        if read_only {
            self.drop_demand_pseudo_record(node, u8::MAX);
            self.demand_pseudo_records.insert((node, u8::MAX), record);
            return answer(record);
        }
        let inherited_group_count = self
            .computed_group_sets
            .inherited_group_count(record.raw())
            .unwrap_or(computed::ENGINE_INHERITED_GROUP_COUNT);
        let assigned = self
            .assign_shared_style_record(
                computed::ComputedStyleTarget::new(node, u8::MAX),
                record.raw(),
                inherited_group_count,
                false,
                counters,
            )
            .style_record_identity;
        self.computed_group_sets.unpin_style_record(record.raw());
        answer(assigned)
    }

    #[allow(clippy::too_many_arguments)]
    fn answer_or_decline_record_demand(
        &mut self,
        node: StyleNodeID,
        pseudo: Option<u8>,
        exclude_inline_style: bool,
        targeted: bool,
        read_only: bool,
        parent_highlight: u64,
        counters: &mut Counters,
    ) -> Result<Option<RetriedEngineRecord>, &'static str> {
        // A kind the engine holds no rows for generates no box.
        if pseudo.is_some_and(|kind| kind >= 20) {
            return Ok(None);
        }
        // Only a read-only observation of an element leaves its inline style out.
        debug_assert!(!exclude_inline_style || (read_only && pseudo.is_none()));
        let target_has_pending_facts = self.host.journal.inputs().any(|input| {
            matches!(input.key, InputKey::LocalFeature(changed, _) | InputKey::State(changed, _) if changed == node)
        });
        // The host takes the pending transaction before it asks, so a demand never reads facts
        // or a program with changes still staged. Should one, it is declined.
        let transaction_is_pending = !self.tree.is_live(node)
            || !self.host.tree_staging.is_empty()
            || self.host.program_staging.is_dirty()
            || self.host.sheet_rule_replacement.is_some()
            || !self.host.journal.markers().is_empty()
            || self.host.journal.inputs().any(|input| {
                input.key.style_node().is_none()
                    || matches!(input.key, InputKey::TreeRelations(_))
                    || (!read_only
                        && matches!(input.key, InputKey::LocalFeature(changed, _) | InputKey::State(changed, _) if changed == node))
            });
        debug_assert!(
            !transaction_is_pending,
            "a record demand ahead of its pending transaction"
        );
        if transaction_is_pending {
            return Err("GateReaction");
        }
        let mut ancestor = self.tree.flat_tree_parent(node);
        let mut ancestors = Vec::new();
        while let Some(parent) = ancestor {
            let pending = self
                .host
                .journal
                .inputs()
                .any(|input| input.key.style_node() == Some(parent))
                || self
                    .host
                    .deferred_element_style_inputs
                    .iter()
                    .any(|input| input.key.style_node() == Some(parent));
            // A demand the host makes where it applies a row reads its ancestors as the host
            // installed them before it; what is still pending for one of them reaches the row
            // through the reaction that ancestor derives. A private observation of a
            // pseudo-element has no such later reaction.
            debug_assert!(
                !(pending && read_only && pseudo.is_some()),
                "a private pseudo-element demand under an ancestor with pending input"
            );
            if pending && read_only && pseudo.is_some() {
                return Err("GateReaction");
            }
            ancestors.push((parent, pending));
            ancestor = self.tree.flat_tree_parent(parent);
        }
        let provisional = read_only
            && pseudo.is_none()
            && (target_has_pending_facts || ancestors.iter().any(|(_, pending)| *pending));

        // A private observation of a row the host is still installing would read a record it has
        // not taken yet; the host installs its batch before any such read.
        let row_is_being_installed = read_only
            && (self.engine_computed_records_pending.contains_key(&node)
                || self.batch_pinned_compositions.iter().any(|(owner, _)| *owner == node));
        debug_assert!(
            !row_is_being_installed,
            "a private demand for a row the host is installing"
        );
        if row_is_being_installed {
            return Err("GateReaction");
        }
        let previous_container_inputs = read_only.then(|| self.container_query_inputs.get(node).cloned());
        let previous_substitution = read_only.then(|| self.nodes_with_substituted_records.contains(&node));
        let previous_explicit_inheritance =
            read_only.then(|| self.nodes_owing_explicit_inheritance.get(&node).copied());
        let previous_container_effect = read_only.then(|| self.container_effects_for_host.get(&node).cloned());
        let mut private_origin_record = None;
        if pseudo.is_some() && self.computed_group_sets.assigned_style_record(node).is_none() {
            let parent = self.answer_or_decline_record_demand(node, None, false, targeted, read_only, 0, counters)?;
            // An element demand answers a live record. Without one the pseudo-element has no
            // originating record, and generates no box.
            let record = parent.map(|parent| parent.style_record).filter(|&record| {
                read_only
                    && self
                        .computed_group_sets
                        .assign_shared_style_record(
                            computed::ComputedStyleTarget::new(node, u8::MAX),
                            record,
                            computed::ENGINE_INHERITED_GROUP_COUNT,
                            false,
                        )
                        .is_some()
            });
            debug_assert!(
                !read_only || record.is_some(),
                "an element demand answers an assignable record"
            );
            if let Some(record) = record {
                self.set_element_container_query_inputs(node, record);
                private_origin_record = computed::FinalStyleRecordID::from_raw(record);
            }
        }

        let mut private_ancestor_records = Vec::new();
        if read_only && pseudo.is_none() {
            let mut private_counters = counters.clone();
            // An ancestor without a record, one in a subtree C++ has not styled yet, is settled
            // first too: the target inherits from it, as a C++ read styles the whole chain.
            if let Some(farthest) = ancestors.iter().rposition(|&(parent, pending)| {
                pending || self.computed_group_sets.assigned_style_record(parent).is_none()
            }) {
                for &(parent, _) in ancestors[..=farthest].iter().rev() {
                    let previous = self
                        .computed_group_sets
                        .assigned_style_record(parent)
                        .unwrap_or(computed::FinalStyleRecordID::NONE);
                    let inputs = self.container_query_inputs.get(parent).cloned();
                    let answer = self.answer_or_decline_record_demand(
                        parent,
                        None,
                        false,
                        targeted,
                        true,
                        0,
                        &mut private_counters,
                    );
                    let Some(private) = answer
                        .ok()
                        .flatten()
                        .and_then(|answer| computed::FinalStyleRecordID::from_raw(answer.style_record))
                    else {
                        self.restore_private_ancestor_records(private_ancestor_records);
                        return Err("GateReaction");
                    };
                    // The record the demand just derived is live; one that is not leaves the
                    // ancestor its installed record.
                    let assigned = self.computed_group_sets.assign_shared_style_record(
                        computed::ComputedStyleTarget::new(parent, u8::MAX),
                        private.raw(),
                        computed::ENGINE_INHERITED_GROUP_COUNT,
                        false,
                    );
                    debug_assert!(assigned.is_some(), "a record a demand derived is assignable");
                    if assigned.is_none() {
                        continue;
                    }
                    self.set_element_container_query_inputs(parent, private.raw());
                    private_ancestor_records.push((parent, private, previous, inputs));
                }
            }
        }

        let hidden_inline_declarations = exclude_inline_style
            .then(|| self.facts.hide_inline_declarations_for_demand(node))
            .flatten();
        if !read_only {
            self.forget_node_match_answer_for_demand(node);
        }
        // The batch's answers for its other rows stay theirs: a row the host still applies, or
        // retries after its ancestors installed, reads its answer after this demand.
        let mut published_answers = Some(std::mem::take(&mut self.published_match_answers));
        if read_only || !self.begin_cold_matching_batch(node, counters) {
            self.begin_adaptive_cold_matching_batch(node, counters);
        }
        let retained_dispatch = (read_only && !target_has_pending_facts)
            .then(|| self.retained_answer_dispatch_for_traversal(true))
            .flatten();
        let answer = self.complete_published_match_answer(node, retained_dispatch.as_deref(), counters);
        if read_only {
            if let Ok(answer) = &answer {
                let mut traversal = self
                    .batch_matching_traversal
                    .take()
                    .expect("a demand has a matching traversal");
                traversal.pending_published.push(
                    PublishedMatchAnswer {
                        node,
                        cascade_input: answer.cascade_input,
                        matches: answer.matches.clone(),
                        cascade_winners_are_complete: answer.cascade_winners_are_complete,
                        observed: false,
                    },
                    &mut self.memory,
                    counters,
                );
                traversal.answer_effects.note_published(
                    node,
                    traversal.pending_published.entries.len() - 1,
                    &mut self.memory,
                );
                self.batch_matching_traversal = Some(traversal);
            }
        } else {
            self.end_cold_matching_batch(counters);
            self.published_match_answers = published_answers.take().expect("a demand saved the batch's answers");
        }
        let result = (|| {
            let answer = answer.map_err(|_| "GateIncompleteAnswer")?;
            let complete = answer.cascade_winners_are_complete
                || self.cascade_winners_are_complete_but_for_custom_properties(node);
            if !read_only {
                self.computed_group_sets.set_node_answer_incomplete(node, !complete);
                self.retained
                    .published_match_answers
                    .push(answer, &mut self.retained.memory, counters);
                self.retained.published_match_answers.sort();
            }

            // An engine no document hosts computes no records.
            if !self.computes_records() {
                return Ok(None);
            }
            if let Some(kind) = pseudo {
                let record = self.demand_pseudo_record(
                    node,
                    kind,
                    read_only,
                    targeted,
                    computed::FinalStyleRecordID::from_raw(parent_highlight),
                    counters,
                );
                Ok(record.map(|record| RetriedEngineRecord {
                    style_record: record.raw(),
                    ..RetriedEngineRecord::default()
                }))
            } else {
                let mut scratch = EngineComputedRecordScratch {
                    recompute_in_full: targeted,
                    targeted_record_demand: targeted,
                    ..EngineComputedRecordScratch::default()
                };
                let (_, record) = self.settled_engine_computed_record_delta(
                    node,
                    complete,
                    None,
                    ParentInputsMoved {
                        inherited_style: targeted,
                        display: targeted,
                    },
                    &mut scratch,
                    counters,
                );
                let mut result = RetriedEngineRecord {
                    style_record: record.raw(),
                    provisional,
                    ..RetriedEngineRecord::default()
                };
                for delta in &scratch.pseudo_deltas {
                    let kind = usize::from(delta.kind);
                    if kind < bridge::RETRY_PSEUDO_RECORD_SLOTS {
                        result.pseudo_records_present |= 1 << kind;
                        result.pseudo_records[kind] = delta.new_style_record.raw();
                    }
                }
                if !read_only {
                    self.host.journal.acknowledge_node(node, &mut self.retained.memory);
                    self.consume_element_style_input(node);
                    self.style_input_nodes_for_cpp.remove(&node);
                    self.tree_counting_input_nodes.remove(&node);
                }
                Ok(Some(result))
            }
        })();
        if let Some(hidden) = hidden_inline_declarations {
            self.facts.restore_inline_declarations_after_demand(node, hidden);
        }
        if read_only {
            if let Ok(Some(record)) = result
                && pseudo.is_none()
            {
                self.drop_demand_pseudo_record(node, u8::MAX);
                self.computed_group_sets.pin_style_record(record.style_record);
                self.demand_pseudo_records.insert(
                    (node, u8::MAX),
                    computed::FinalStyleRecordID::from_raw(record.style_record).unwrap(),
                );
            }
            for pending in self.engine_computed_records_pending.remove(&node).into_iter().flatten() {
                if pending.pseudo_kind == u8::MAX {
                    self.computed_group_sets.revert_engine_computed_record(
                        node,
                        pending.new_style_record,
                        pending.old_style_record,
                    );
                } else {
                    self.revert_engine_computed_pseudo_record(&pending, counters);
                }
            }
            if let Some(record) = private_origin_record {
                self.computed_group_sets.revert_engine_computed_record(
                    node,
                    record,
                    computed::FinalStyleRecordID::NONE,
                );
            }
            self.drop_pinned_compositions(node);
            match previous_container_inputs.expect("read-only demand saved container inputs") {
                Some(row) => self.container_query_inputs.set(node, row),
                None => self.container_query_inputs.clear(node),
            }
            if previous_substitution.expect("read-only demand saved substitution") {
                self.nodes_with_substituted_records.insert(node);
            } else {
                self.nodes_with_substituted_records.remove(&node);
            }
            match previous_explicit_inheritance.expect("read-only demand saved inheritance") {
                Some(groups) => {
                    self.nodes_owing_explicit_inheritance.insert(node, groups);
                }
                None => {
                    self.nodes_owing_explicit_inheritance.remove(&node);
                }
            }
            match previous_container_effect.expect("read-only demand saved container effects") {
                Some(effect) => {
                    self.container_effects_for_host.insert(node, effect);
                }
                None => {
                    self.container_effects_for_host.remove(&node);
                }
            }
            self.discard_private_record_demand_matching_batch();
            self.published_match_answers = published_answers.expect("read-only demand saved published answers");
        }
        self.restore_private_ancestor_records(private_ancestor_records);
        result
    }

    pub(super) fn reclaim_computed_memory_if_needed(&mut self, counters: &mut Counters) {
        self.retained.computed_group_sets.reclaim_retired_animation_overlays();
        // Recording dictionaries are keyed by computed identities. Reusing an identity for new
        // semantics would make later events refer to the first definition replay saw for it.
        if self.recording_id().is_none()
            && let Some(retention) = self.retained.computed_group_sets.reclaim_unreachable_if_needed()
        {
            counters.set(Counter::ComputedGroupsRetained, retention.retained as u64);
            counters.set(Counter::ComputedGroupsReachable, retention.reachable as u64);
            let live: super::fast_hash::FastSet<u64> = self
                .retained
                .computed_group_sets
                .live_custom_property_environments()
                .collect();
            self.retained
                .custom_property_environments
                .retain_only(|identity| live.contains(&identity));
            // The records the engine keeps for reuse are keyed by the parent's inherited group set
            // and custom property environment, whose reclaimed identities the next computations
            // reuse for other contents.
            self.retained.engine_cold_record_cache.clear();
            self.retained.engine_cold_record_donors.clear();
            self.retained.engine_warm_record_cohorts.clear();
            self.retained.engine_pseudo_record_cache.clear();
        }
        self.settle_computed_memory();
    }

    pub(super) fn publish_animation_overlay_impl(
        &mut self,
        target: computed::ComputedStyleTarget,
        source_identity: u64,
        animated_overlay: HostShared<crate::css::animated_overlay::AnimatedOverlay>,
        payloads: &[SharedPayload],
        counters: &mut Counters,
    ) -> Option<computed::AnimationOverlayUpdate> {
        if self.recording_id().is_some() {
            return None;
        }
        let publication = self.retained.computed_group_sets.publish_animation_overlay(
            target,
            source_identity,
            animated_overlay,
            payloads,
        )?;
        self.settle_computed_memory();
        if publication.slot_allocated {
            counters.bump(Counter::AnimationOverlaySlotsAllocated);
        }
        if publication.slot_released {
            counters.bump(Counter::AnimationOverlaySlotsReleased);
        }
        if publication.record_updated {
            counters.bump(Counter::AnimationOverlayRecordsUpdated);
        }
        counters.set(Counter::LiveAnimationOverlayRecords, publication.live_records as u64);
        Some(publication)
    }
}

/// What a first record was derived from: the parent's side of the computation, the winner state
/// (with the generation its identity belongs to), the element facts, the pseudo-elements the
/// element has rules for, and the font environment.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ColdRecordKey {
    /// What the monospace font-size recascade gives this node, or zero when its winning
    /// font-family is not monospace. The recascade reads the whole cascaded chain, which equal
    /// parent records do not pin down.
    monospace_recascaded_font_size: i32,
    parent: ColdRecordParent,
    previous_style_record: u64,
    generation: u64,
    state: CascadeStateID,
    facts: u32,
    /// The pseudo-elements the element has rules for: the record's metadata says which, and
    /// C++ computes their styles beside it.
    pseudo_styles: u64,
    /// The custom-property environment the record is published with.
    environment: u64,
    font_environment_generation: u64,
    root_font_inputs: RootFontInputs,
    element_reads: ElementReads,
}

/// What a first record reads of the parent's style: its inherited groups, its custom-property
/// environment, its dependency flags, and the display the box-type transformation takes as the
/// parent's. A state that explicitly inherits a non-inherited property reads the parent's whole
/// table, so it keys on the parent's record instead.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ColdRecordParent {
    record: u64,
    inherited_groups: u32,
    environment: u64,
    dependency_flags: u8,
    parent_display: u32,
}

/// What a warm record's cohort is keyed by, and what it answers with: the record, and the style
/// groups it read straight from the parent through an explicit `inherit`. The winner generation
/// comes first, so a key kept past its transaction names the winners the state stood for then.
pub(super) type RecordCohortKey = (
    u64,
    u64,
    CascadeStateID,
    u32,
    RecordDeltaParent,
    u64,
    RootFontInputs,
    i32,
    ElementReads,
);
pub(super) type RecordCohortValue = (computed::FinalStyleRecordID, u32);

/// What a record reads of its element beyond its winners and its parent, on which the elements
/// sharing the record must agree: the element's attributes and element-scoped random bases, and its
/// tree-counting inputs, each zero where no winner reads it ([`RetainedState::element_reads`]).
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub(super) struct ElementReads {
    attributes: u64,
    tree_counting: (u32, u64),
}

/// A first record the engine keeps for reuse, with the swap eligibility its assignment carries.
#[derive(Clone, Copy)]
pub(super) struct ColdRecord {
    record: computed::FinalStyleRecordID,
    swap_eligible: bool,
    /// The style groups the record read straight from the parent through an explicit `inherit`,
    /// which every element the record answers for owes its own parent.
    explicitly_inherited_groups: u32,
}

/// The value-independent half of a first-record key. Records under the same key may seed one
/// another, with the semantic cascade delta selecting the values which must be recomputed.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct ColdRecordDonorKey {
    parent: ColdRecordParent,
    generation: u64,
    property_shape_hash: u64,
    facts: u32,
    pseudo_styles: u64,
    environment: u64,
    font_environment_generation: u64,
    root_font_inputs: RootFontInputs,
    element_reads: ElementReads,
}

#[derive(Clone, Copy)]
pub(super) struct ColdRecordDonor {
    state: CascadeStateID,
    record: ColdRecord,
}

const MAXIMUM_COLD_RECORD_DONORS_PER_KEY: usize = 4;

/// First records the engine keeps for reuse; cleared wholesale past this many entries.
const COLD_RECORD_CACHE_LIMIT: usize = 4096;

/// A record the engine derived for a published reaction, awaiting C++'s installation.
pub(super) struct PendingEngineComputedRecord {
    node: StyleNodeID,
    /// The pseudo-element the record is for, or `u8::MAX` for the element's own.
    pseudo_kind: u8,
    old_style_record: computed::FinalStyleRecordID,
    new_style_record: computed::FinalStyleRecordID,
    /// The winner state the record was derived from; an implicit marker without rules has none.
    cascade_state: Option<(u64, CascadeStateID)>,
    /// Longhands the drive evaluated for the record; counted once C++ installs it.
    longhand_evaluations: u32,
}

/// What one flush accumulates while deriving engine-computed records: the record each cohort
/// (an old record moved to a winner state) derived, and the cascade each winner state describes.
/// Which of a parent's inputs to its children's records may have moved under a record.
#[derive(Clone, Copy, Default)]
pub(super) struct ParentInputsMoved {
    /// The parent's inherited style groups.
    pub(super) inherited_style: bool,
    /// The parent's display, which the children's box-type transformation reads.
    pub(super) display: bool,
}

impl ParentInputsMoved {
    pub(super) fn any(self) -> bool {
        self.inherited_style || self.display
    }
}

/// Static checks attached to the original declaration spelling at input preparation, and what
/// the per-state queries ask of the written value, which every element holding the declaration
/// as a winner would otherwise walk the value for again.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) struct WrittenValueChecks {
    whole_context_free: bool,
    longhand_context_free: Option<bool>,
    /// `var()`, `attr()` and the like, or a longhand pending its shorthand's substitution.
    substitution: WrittenSubstitution,
    /// Written with `attr()`.
    reads_attributes: bool,
    /// What `state_custom_condition_usage` reads: `if()`, `inherit()` and dashed functions.
    custom_condition_usage: u8,
    container_relative_length_unit_mask: u8,
    uses_tree_counting_function: bool,
    /// A value with no substitution that draws an element-scoped `random()`.
    reads_element_random: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum WrittenSubstitution {
    None,
    Unresolved,
    PendingShorthand,
}

impl WrittenValueChecks {
    pub(super) fn prepare(property: u16, value: &crate::css::style_value::RetainedStyleValueData) -> Self {
        use crate::css::style_value::StyleValueData;
        let whole_context_free = value_computes_without_document_context(value.data());
        let longhand_context_free = match value.data() {
            StyleValueData::Unresolved { .. } | StyleValueData::PendingSubstitution { .. } => None,
            StyleValueData::Shorthand { .. } => Some(
                shorthand_longhand_value(property, value.data())
                    .is_some_and(|value| value_computes_without_document_context(value.data())),
            ),
            _ => Some(whole_context_free),
        };
        let substitution = match value.data() {
            StyleValueData::Unresolved { .. } => WrittenSubstitution::Unresolved,
            StyleValueData::PendingSubstitution { .. } => WrittenSubstitution::PendingShorthand,
            _ => WrittenSubstitution::None,
        };
        let dependencies = crate::css::style_compute::collect_external_value_dependencies(value.data());
        Self {
            whole_context_free,
            longhand_context_free,
            substitution,
            reads_attributes: matches!(
                value.data(),
                StyleValueData::Unresolved {
                    presence_attr: true,
                    ..
                }
            ),
            custom_condition_usage: custom_condition_usage_of(value.data()),
            container_relative_length_unit_mask: dependencies.container_relative_length_unit_mask,
            uses_tree_counting_function: dependencies.uses_tree_counting_function,
            reads_element_random: substitution == WrittenSubstitution::None && value_reads_element_random(value.data()),
        }
    }

    /// Whether a winner written with this value leaves `StateWrittenFacts` as it is.
    pub(super) fn adds_no_state_facts(&self) -> bool {
        self.substitution == WrittenSubstitution::None
            && !self.reads_element_random
            && self.container_relative_length_unit_mask == 0
            && !self.uses_tree_counting_function
    }
}

/// The condition and function dependencies a written value substitutes with, through any pending
/// shorthand it stands for.
fn custom_condition_usage_of(mut value: &crate::css::style_value::StyleValueData) -> u8 {
    while let crate::css::style_value::StyleValueData::PendingSubstitution {
        original_shorthand_value,
    } = value
    {
        value = original_shorthand_value.data();
    }
    match value {
        crate::css::style_value::StyleValueData::Unresolved {
            presence_if,
            presence_inherit,
            presence_dashed_function,
            ..
        } => u8::from(*presence_if) | (u8::from(*presence_inherit) << 1) | (u8::from(*presence_dashed_function) << 2),
        _ => 0,
    }
}

/// What a node's winners under one cascade state were written with, as the per-state queries
/// ask it, gathered in one walk of the winners.
#[derive(Clone, Copy, Default)]
pub(super) struct StateWrittenFacts {
    /// Of the longhand winners, whether any substitutes, and whether any reads attributes.
    pub(super) has_substitutions: bool,
    pub(super) reads_attributes: bool,
    pub(super) custom_condition_usage: u8,
    /// Of every winner.
    pub(super) container_relative_length_unit_mask: u8,
    pub(super) has_written_tree_counting: bool,
    /// Whether any winner may read an element-scoped `random()` base: one written with one, or
    /// with a substitution whose value is only known once it is resolved.
    pub(super) may_read_element_random: bool,
}

#[derive(Default)]
pub(super) struct EngineComputedRecordScratch {
    /// A scoped read can request the base record of a cold animation target.
    pub(super) targeted_record_demand: bool,
    pub(super) font_drive: drive::FontDriveScratch,
    // NB: Preserve the root's existing remaining-phase context after preparing consumer inputs.
    root_element_inputs: Option<(StyleNodeID, RootFontInputs)>,
    pub(super) root_font_inputs_changed: bool,
    pending_element: Option<(computed::FinalStyleRecordID, computed::FinalStyleRecordID)>,
    next_pseudo: usize,
    pseudo_explicitly_inherited_groups: u32,
    pseudo_uses_substitution: bool,
    /// What the element being derived noted about substituted winners, when its computation
    /// reached the point of deciding. A record that stands unchanged notes nothing.
    noted_substitution: Option<bool>,
    /// Whether the element derived last was computed with a substituted winner: what the flush
    /// publishes beside its record, decided by the step rather than read back out of the
    /// retained set.
    pub(super) element_uses_substitution: bool,
    /// The nodes whose substituted-record fact the step decided, in the order it decided them.
    /// The boundary that installs the record applies them.
    substitution_effects: Vec<(StyleNodeID, bool)>,
    /// The pseudo-element records settled beside the element derived last.
    pub(super) pseudo_deltas: Vec<PseudoRecordDelta>,
    /// The pseudo-element rules that flipped for the element being derived.
    pub(super) flipped_pseudo_rules: u64,
    /// Whether this flush carries a document environment action. A record's winners can stand
    /// through one while the values they computed to do not, so such a record is driven again in
    /// full rather than kept - and rather than handed back to C++.
    pub(super) document_environment_moved: bool,
    /// An input to the record the step derives, set beside the node the flush is about to derive:
    /// whether that element's font environment moved. It rides here rather than in the argument
    /// list because the record loop is shared with four other lines of work. The flush assigns it
    /// for every node it derives, and it is false for the whole of a flush that derives none.
    pub(super) font_environment_moved: bool,
    /// Set the same way: whether the reaction drives the element's record again in full whatever
    /// its winners did, for inputs the winners do not show.
    pub(super) recompute_in_full: bool,
    /// Set the same way: whether an ancestor of the element left display:none, which cleared the
    /// element's style on entry.
    pub(super) ancestor_became_visible: bool,
    /// This row's selector answer or declaration values moved without publishing current winners.
    pub(super) answer_or_declarations_moved: bool,
    /// Whether the viewport moved since the last flush. A record that reads it holds values its
    /// winners do not name, so it cannot stand, and the row drives again against the new one.
    pub(super) viewport_moved: bool,
    pub(super) prepared_root_font: Option<(StyleNodeID, ParentInputsMoved, drive::FontDriveScratch)>,
    cohorts: HashMap<RecordCohortKey, RecordCohortValue>,
    /// What each node the walk has reached tells its children: whether the chain above it is
    /// confined, and whether it resolved the record its children inherit from. A column with
    /// touched-page allocation, because the walk writes a row for every node it processes and
    /// reads one for every node's parent.
    pub(super) derived_child_inputs: column::PagedColumn<column::PagedValuePage<DerivedChildInputs>>,
    /// First records derived this flush, by what they were derived from.
    pub(super) cold_cohorts: HashMap<ColdRecordKey, ColdRecord>,
    store_capacity_bytes: u64,
    pub(super) stores: HashMap<(CascadeStateID, u64), std::sync::Arc<WinnerStore>>,
    /// The states whose store, under an environment, substituted a custom property into a winner.
    pub(super) substituted_states: HashSet<(CascadeStateID, u64)>,
    /// Pseudo-element records derived this flush, by what they were derived from.
    pub(super) pseudo_cohorts: HashMap<PseudoCohortKey, computed::FinalStyleRecordID>,
    pub(super) pseudo_stores: HashMap<(u8, CascadeStateID, u64), std::sync::Arc<WinnerStore>>,
    /// The nodes whose custom-property environment this flush has brought up to date.
    current_custom_property_environments: HashMap<StyleNodeID, u64>,
}

/// What one node tells its flat-tree children, decided where the node settles and read by the
/// children in the same pass: whether the node resolved the record its children inherit from, and
/// the accumulated proof about the chain above it, so a child reads one row instead of walking to
/// the document element for every gate it has to pass.
#[derive(Clone, Copy, Default)]
pub(super) struct DerivedChildInputs {
    /// Whether the node's record settled this flush: what its descendants inherit from is in place.
    pub(super) settled: bool,
    /// Whether the node's final record is only in place once the host installs it: a record the
    /// host computes, or one whose animation sample, transition step or animation plan the host
    /// completes at installation.
    pub(super) awaits_host: bool,
    /// Whether the host installed the node in an earlier wave of the pass: what its change moved
    /// for its descendants is in place, or joins the pass as rows of their own.
    pub(super) installed: bool,
    /// Whether the node took an inherited-style reaction and resolved no record of its own, so
    /// its immediate children cannot take the direct inherited-group path. Deliberately separate
    /// from the chain proof: that is the accumulated confinement argument, this is the immediate
    /// parent's own unresolved fact.
    pub(super) inheritance_unresolved: bool,
    /// The proof about everything from this node upwards, folded on the first ask a child makes
    /// and kept only once every fact it folds is final.
    pub(super) chain: Option<AncestorChain>,
}

/// The published chain from one node to the root, as a child's gate reads it.
#[derive(Clone, Copy)]
pub(super) struct AncestorChain {
    /// Whether any published node from this one to the root publishes a change the engine cannot
    /// prove confined. Once an unsettled node sits below one of those, no descendant's record is
    /// exact.
    unconfined_above: bool,
    /// `None` when an unsettled gap sits below an unconfined published ancestor, and otherwise
    /// whether a child relies on an ancestor this flush settled.
    proof: Option<bool>,
}

impl AncestorChain {
    /// What the document element's parent says: nothing above it, and nothing unsettled.
    pub(super) const ROOT: Self = Self {
        unconfined_above: false,
        proof: Some(false),
    };

    /// The proof a child of this node needs.
    pub(super) fn ancestors_are_confined(self) -> Option<bool> {
        self.proof
    }

    /// Fold one node onto what its parent says. `published` says the flush published a reaction
    /// for the node, `unconfined` that the published change may move something a descendant
    /// inherits, and `settled` that the node's record is in place.
    ///
    /// This is the upward walk written as a recurrence. The walk carries "something closer to the
    /// child is unsettled" downward and stops at the first unconfined published ancestor once it
    /// is set, which is exactly `unconfined_above` read from the unsettled node.
    pub(super) fn fold(parent: Self, published: bool, unconfined: bool, settled: bool) -> Self {
        let unconfined_here = published && unconfined;
        let proof = if unconfined_here && !settled {
            None
        } else if settled {
            parent.proof.map(|relied| unconfined_here || relied)
        } else if parent.unconfined_above {
            None
        } else {
            Some(unconfined_here)
        };
        Self {
            unconfined_above: unconfined_here || parent.unconfined_above,
            proof,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) enum RecordDeltaParent {
    Exact(u64),
    Inputs(ColdRecordParent),
}

/// Which element and pseudo winner rows must reflect this update's exact rule flips.
#[derive(Clone, Copy, Default)]
pub(super) struct FlippedRules {
    element: bool,
    pseudos: u64,
}

impl FromIterator<Option<u16>> for FlippedRules {
    fn from_iter<T: IntoIterator<Item = Option<u16>>>(kinds: T) -> Self {
        let mut result = Self::default();
        for kind in kinds {
            match kind {
                None => result.element = true,
                Some(kind) => {
                    // NB: Only the existing synthetic pseudo inventory consumes these bits.
                    if let Some(bit) = 1_u64.checked_shl(u32::from(kind)) {
                        result.pseudos |= bit;
                    }
                }
            }
        }
        result
    }
}

impl EngineComputedRecordScratch {
    /// The document element, where the pass drives its font inputs.
    pub(super) fn root_element_inputs(&self) -> Option<StyleNodeID> {
        self.root_element_inputs.map(|(root, _)| root)
    }

    pub(super) fn capacity_bytes(&self) -> u64 {
        capacity::capacity_bytes! {
            shallow [self.cohorts, self.derived_child_inputs, self.cold_cohorts, self.stores,
                self.substituted_states, self.pseudo_cohorts, self.pseudo_stores, self.current_custom_property_environments,
                self.pseudo_deltas, self.substitution_effects];
            cached [self.store_capacity_bytes, self.font_drive.capacity_bytes(),
                self.prepared_root_font.as_ref().map_or(0, |(_, _, drive)| drive.capacity_bytes())];
            nested [];
            skip [];
        }
    }
}

/// What a drive computes for: the node whose record it inherits from (the flat-tree parent, or
/// the originating element of a pseudo-element) and the element facts the computation's
/// adjustments read.
#[derive(Clone, Copy)]
pub(super) struct DriveSubject {
    /// What is being driven, element or pseudo-element. A drive that suspends to wait for a font
    /// names it, so the suspended drive can only be resumed by the one it belongs to.
    target: computed::ComputedStyleTarget,
    /// The flat-tree parent the element inherits from; the document element has none and
    /// inherits from the initial values.
    parent: Option<StyleNodeID>,
    facts: u32,
    highlight_parent: Option<computed::FinalStyleRecordID>,
}

/// What a pass knows of its transaction that a row it drives over the installed ancestors reads.
#[derive(Clone, Copy)]
pub(super) struct DriveOverInstalledAncestors {
    pub(super) document_environment_moved: bool,
    pub(super) root_font_inputs_changed: bool,
    pub(super) viewport_moved: bool,
}

/// What a demand or a drive over the installed ancestors settles: the element's record, and the
/// pseudo-element records the engine settled beside it, one slot per synthetic kind with a present
/// bit each; a present slot holding zero is a removal.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct RetriedEngineRecord {
    pub(crate) style_record: u64,
    pub(crate) provisional: bool,
    pub(crate) pseudo_records_present: u8,
    pub(crate) pseudo_records: [u64; bridge::RETRY_PSEUDO_RECORD_SLOTS],
}

const _: () = assert!(pseudo_kind::SYNTHETIC_COUNT == bridge::RETRY_PSEUDO_RECORD_SLOTS);

/// The answer to one record demand: the row's record, or, for a pseudo-element that generates
/// no box, its absence.
#[derive(Clone, Copy, Debug)]
pub(crate) enum RecordDemandAnswer {
    Record(RetriedEngineRecord),
    Absent,
}

/// A pseudo-element record the engine settled beside its originating element's; a removal when
/// the new record is none.
#[derive(Clone, Copy)]
pub(super) struct PseudoRecordDelta {
    pub(super) kind: u8,
    pub(super) old_style_record: computed::FinalStyleRecordID,
    pub(super) new_style_record: computed::FinalStyleRecordID,
}

/// What a pseudo-element record is derived from: the originating element's inherited style,
/// display, dependency flags and custom-property environment (and its record, when the state
/// inherits a non-inherited property from it), the pseudo-element's winner state and the
/// element facts and font environment the drive reads.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PseudoCohortKey {
    parent_record: u64,
    highlight_parent_record: u64,
    inherited_groups: u32,
    parent_display: u32,
    dependency_flags: u8,
    environment: u64,
    kind: u8,
    generation: u64,
    state: Option<CascadeStateID>,
    facts: u32,
    font_environment_generation: u64,
    custom_property_registration_generation: u64,
    root_font_inputs: RootFontInputs,
    element_reads: ElementReads,
}

/// The synthetic pseudo-element kinds, as the C++ `PseudoElement` enumeration numbers them.
mod pseudo_kind {
    pub(super) const AFTER: u8 = 0;
    pub(super) const BACKDROP: u8 = 1;
    pub(super) const BEFORE: u8 = 2;
    pub(super) const FIRST_LETTER: u8 = 3;
    pub(super) const MARKER: u8 = 5;
    pub(super) const SELECTION: u8 = 6;
    pub(super) const SYNTHETIC_COUNT: usize = 8;
}

/// The element facts a pseudo-element's computation reads: the C++ adjustments for what the
/// originating element is stay off for its pseudo-elements, past the ones about its markup
/// language.
const PSEUDO_ELEMENT_ADJUSTMENT_FACTS: u32 = {
    use bridge::element_adjustment_fact as fact;
    fact::IS_MATHML
        | fact::IS_MATHML_MTABLE
        | fact::IS_MATHML_MTR
        | fact::IS_MATHML_MTD
        | fact::IS_TH
        | fact::HAS_ANIMATIONS
};

/// The value a shorthand value carries for one of its longhands, through nested shorthands.
/// The `unset` keyword, which a declaration invalid at computed-value time computes as.
fn unset_value() -> crate::css::style_value::RetainedStyleValueData {
    crate::css::style_value::RetainedStyleValueData::from_owned(crate::css::style_value::StyleValueData::Keyword {
        keyword: crate::css::style_compute::keyword::UNSET,
    })
}

/// A substituted value as the cascade takes it: one invalid at computed-value time is unset, and
/// so is a `revert` or `revert-layer` the substitution produced, which the cascade does not roll
/// back once substitution has run.
fn invalid_as_unset(
    value: crate::css::style_value::RetainedStyleValueData,
) -> crate::css::style_value::RetainedStyleValueData {
    use crate::css::style_compute::keyword;
    match value.data() {
        crate::css::style_value::StyleValueData::GuaranteedInvalid => unset_value(),
        crate::css::style_value::StyleValueData::Keyword { keyword }
            if *keyword == keyword::REVERT || *keyword == keyword::REVERT_LAYER =>
        {
            unset_value()
        }
        _ => value,
    }
}

/// The longhand's part of a substituted shorthand value, as the cascade expands it.
fn expanded_longhand_value(
    shorthand: u16,
    property: u16,
    value: &crate::css::style_value::RetainedStyleValueData,
) -> Option<crate::css::style_value::RetainedStyleValueData> {
    let mut found = None;
    crate::css::style_compute::expand_shorthands_with(
        shorthand,
        value.pointer().cast(),
        false,
        &mut |longhand, data, _| {
            if longhand == property && found.is_none() {
                found = Some(unsafe {
                    crate::css::style_value::RetainedStyleValueData::from_retained_pointer(
                        crate::css::style_value::retain_style_value(data.cast()),
                    )
                });
            }
        },
    );
    found
}

fn shorthand_longhand_value(
    property: u16,
    data: &crate::css::style_value::StyleValueData,
) -> Option<crate::css::style_value::RetainedStyleValueData> {
    let crate::css::style_value::StyleValueData::Shorthand {
        sub_properties, values, ..
    } = data
    else {
        return None;
    };
    for (&sub_property, sub_value) in sub_properties.as_slice().iter().zip(values.as_slice()) {
        let sub_data = unsafe { &*sub_value.pointer().cast::<crate::css::style_value::StyleValueData>() };
        if sub_property == property {
            return Some(unsafe {
                crate::css::style_value::RetainedStyleValueData::from_retained_pointer(
                    crate::css::style_value::retain_style_value(sub_data),
                )
            });
        }
        if let Some(found) = shorthand_longhand_value(property, sub_data) {
            return Some(found);
        }
    }
    None
}

/// The computed style groups a longhand's winner feeds. A longhand bound to no group of its own is
/// one the font group carries: the host checks the group bindings against every longhand's
/// declared style group when it registers them.
fn longhand_group_dependency_mask(property: u16) -> u32 {
    use crate::css::computed_value_types::STYLE_GROUP_INDEX_FONT;
    computed_group_dependency_mask(property).unwrap_or(1 << STYLE_GROUP_INDEX_FONT)
}

fn property_computes_in_remaining_phase(property: u16) -> bool {
    use crate::css::property_metadata::{FIRST_LONGHAND_PROPERTY_ID, LONGHAND_WORD_COUNT};
    use crate::css::style_compute::{LONGHAND_DRIVE_PHASE_REMAINING, property_computation_order_for_phase};
    static REMAINING: std::sync::OnceLock<[u64; LONGHAND_WORD_COUNT]> = std::sync::OnceLock::new();
    let words = REMAINING.get_or_init(|| {
        let mut words = [0_u64; LONGHAND_WORD_COUNT];
        for &property in property_computation_order_for_phase(LONGHAND_DRIVE_PHASE_REMAINING) {
            let index = usize::from(property - FIRST_LONGHAND_PROPERTY_ID);
            words[index / 64] |= 1 << (index % 64);
        }
        words
    });
    let Some(index) = property.checked_sub(FIRST_LONGHAND_PROPERTY_ID).map(usize::from) else {
        return false;
    };
    index / 64 < words.len() && words[index / 64] & (1 << (index % 64)) != 0
}

/// Whether a longhand is read by the box-type transformation, which rewrites the computed display.
/// A `-webkit-box` becomes a block container while its `-webkit-box-orient` and `continue` say
/// that `-webkit-line-clamp` applies to it.
fn property_feeds_box_type_transformation(property: u16) -> bool {
    use crate::css::property_metadata::property_id as prop;
    matches!(
        property,
        prop::DISPLAY | prop::POSITION | prop::FLOAT | prop::_WEBKIT_BOX_ORIENT | prop::CONTINUE
    )
}

/// Whether a longhand's new value would start an animation or a transition in the C++
/// computation, register anchor names there, or feed the counter-style environment identity it
/// resolves.
/// The longhands the drive's font resolution selects a font by, which its request carries.
fn font_resolution_selects_by(property: u16) -> bool {
    use crate::css::property_metadata::property_id as prop;
    matches!(
        property,
        prop::FONT_FAMILY | prop::FONT_STYLE | prop::FONT_WEIGHT | prop::FONT_WIDTH | prop::FONT_OPTICAL_SIZING
    )
}

/// The font-phase longhands the font group carries without a group binding of their own.
fn font_group_carries_longhand(property: u16) -> bool {
    use crate::css::property_metadata::property_id as prop;
    font_resolution_selects_by(property)
        || matches!(
            property,
            prop::FONT_FEATURE_SETTINGS
                | prop::FONT_KERNING
                | prop::FONT_LANGUAGE_OVERRIDE
                | prop::FONT_VARIANT_ALTERNATES
                | prop::FONT_VARIANT_CAPS
                | prop::FONT_VARIANT_EAST_ASIAN
                | prop::FONT_VARIANT_EMOJI
                | prop::FONT_VARIANT_LIGATURES
                | prop::FONT_VARIANT_NUMERIC
                | prop::FONT_VARIANT_POSITION
                | prop::FONT_VARIATION_SETTINGS
                | prop::MATH_DEPTH
                | prop::MATH_SHIFT
                | prop::MATH_STYLE
                | prop::TEXT_RENDERING
        )
}

/// Whether publication must stamp a counter-style registry identity. Generated content
/// carries the same dependency as the host producer, including predefined counter names;
/// list-style-type only needs it for overridable names.
fn value_reads_counter_style_environment(value: &StyleValueData) -> bool {
    match value {
        StyleValueData::Counter { .. } | StyleValueData::Content { .. } => {
            crate::css::style_compute::content_reads_counter_style_environment(value)
        }
        StyleValueData::ValueList { values, .. } => values
            .as_slice()
            .iter()
            .any(|value| value.optional_data().is_some_and(value_reads_counter_style_environment)),
        StyleValueData::CounterStyle { is_symbols, name, .. } => {
            !*is_symbols && !crate::css::style_compute::counter_style_name_is_non_overridable(name.units())
        }
        _ => false,
    }
}

/// Whether the longhand is one of the five that declare an element's CSS transitions. The
/// `transition` shorthand is not one: a shorthand is never a longhand delta's property, and a
/// property outside the longhand range keeps its record in C++ for its own reasons.
fn longhand_only_declares_a_css_transition(property: u16) -> bool {
    use crate::css::property_metadata::{FIRST_LONGHAND_PROPERTY_ID, LAST_LONGHAND_PROPERTY_ID, property_id as prop};
    (FIRST_LONGHAND_PROPERTY_ID..=LAST_LONGHAND_PROPERTY_ID).contains(&property)
        && matches!(
            property,
            prop::TRANSITION_BEHAVIOR
                | prop::TRANSITION_DELAY
                | prop::TRANSITION_DURATION
                | prop::TRANSITION_PROPERTY
                | prop::TRANSITION_TIMING_FUNCTION
        )
}

/// In a settled row's effect debt: the row left an animation plan for the host to take.
pub(crate) const OWES_AN_ANIMATION_PLAN: u8 = 1 << 2;

/// In a settled row's effect debt: the row derived a record beneath the element's animations, so
/// the host samples them again over it once the batch is applied.
pub(crate) const OWES_AN_ANIMATION_SAMPLE: u8 = 1 << 3;

/// Whether a longhand does nothing but declare one of the element's CSS animations, so that a delta
/// carrying it owes the host the animation plan and nothing else.
fn longhand_declares_a_css_animation(property: u16) -> bool {
    use crate::css::property_metadata::{FIRST_LONGHAND_PROPERTY_ID, LAST_LONGHAND_PROPERTY_ID, property_id as prop};
    (FIRST_LONGHAND_PROPERTY_ID..=LAST_LONGHAND_PROPERTY_ID).contains(&property)
        && matches!(
            property,
            prop::ANIMATION_COMPOSITION
                | prop::ANIMATION_DELAY
                | prop::ANIMATION_DIRECTION
                | prop::ANIMATION_DURATION
                | prop::ANIMATION_FILL_MODE
                | prop::ANIMATION_ITERATION_COUNT
                | prop::ANIMATION_NAME
                | prop::ANIMATION_PLAY_STATE
                | prop::ANIMATION_TIMELINE
                | prop::ANIMATION_TIMING_FUNCTION
        )
}

fn property_starts_animation(property: u16) -> bool {
    use crate::css::property_metadata::{
        FIRST_LONGHAND_PROPERTY_ID, LAST_LONGHAND_PROPERTY_ID, property_id as prop, property_style_group_index,
    };
    if !(FIRST_LONGHAND_PROPERTY_ID..=LAST_LONGHAND_PROPERTY_ID).contains(&property) {
        return true;
    }
    // A view transition name is a plain computed value; it starts nothing. An anchor name is one
    // the host registers from whichever record it installs. A named timeline is a plain computed
    // value too: what finds it is the animation that names it in `animation-timeline`, and that
    // animation reads it from whichever record the element holds when it starts. A list style
    // retains its counter-style name, and record assembly stamps the published registry identity
    // against which that name is resolved.
    !matches!(
        property,
        prop::VIEW_TRANSITION_NAME
            | prop::SCROLL_TIMELINE_NAME
            | prop::SCROLL_TIMELINE_AXIS
            | prop::TIMELINE_SCOPE
            | prop::VIEW_TIMELINE_NAME
            | prop::VIEW_TIMELINE_AXIS
            | prop::VIEW_TIMELINE_INSET
    ) && property_style_group_index(property)
        .is_some_and(|group| usize::from(group) == crate::css::table_group_builder::group_index::ANIMATION)
}

fn value_reads_element_random(value: &StyleValueData) -> bool {
    let mut sharings = Vec::new();
    crate::css::style_compute::collect_unfixed_random_sharings_in_value(value, &mut sharings);
    sharings.into_iter().any(|source| {
        matches!(
            unsafe { &*source },
            StyleValueData::RandomValueSharing {
                is_auto: true,
                element_shared: false,
                ..
            }
        )
    })
}

/// Random bases are retained inputs, filled where a drive suspends on them before it resumes.
fn value_computes_with_random_inputs(value: &StyleValueData, resources_are_known: bool) -> bool {
    if crate::css::style_compute::value_is_computationally_independent(value).is_none() {
        return false;
    }
    let dependencies = crate::css::style_compute::external_value_dependencies(value);
    dependencies.uses_random_function
        && !dependencies.uses_tree_counting_function
        && dependencies.container_relative_length_unit_mask == 0
        && (resources_are_known
            || (!dependencies.needs_document_base_url && !dependencies.may_need_style_sheet_resource_context))
}

/// Container bases are published per subject, and the drive supplies them to nested values too.
fn value_computes_with_container_inputs(value: &StyleValueData, resources_are_known: bool) -> bool {
    if crate::css::style_compute::value_is_computationally_independent(value).is_none() {
        return false;
    }
    let dependencies = crate::css::style_compute::collect_external_value_dependencies(value);
    dependencies.container_relative_length_unit_mask != 0
        && !dependencies.uses_tree_counting_function
        && !dependencies.has_unfixed_random_sharing
        && !dependencies.uses_random_function
        && (resources_are_known
            || (!dependencies.needs_document_base_url && !dependencies.may_need_style_sheet_resource_context))
}

/// The retained tree supplies the sibling count and index to the longhand drive.
fn value_computes_with_tree_counting_inputs(value: &StyleValueData, resources_are_known: bool) -> bool {
    if crate::css::style_compute::value_is_computationally_independent(value).is_none() {
        return false;
    }
    let dependencies = crate::css::style_compute::collect_external_value_dependencies(value);
    dependencies.uses_tree_counting_function
        && dependencies.container_relative_length_unit_mask == 0
        && !dependencies.has_unfixed_random_sharing
        && !dependencies.uses_random_function
        && (resources_are_known
            || (!dependencies.needs_document_base_url && !dependencies.may_need_style_sheet_resource_context))
}

/// Whether a written value computes from the record, the parent and the document's computation
/// inputs alone: no custom-property substitution, and none of the element or sheet facts the C++
/// computation gathers per drive.
fn value_computes_without_document_context(value: &StyleValueData) -> bool {
    value_computes_without_document_context_but_for_resources(value).is_some_and(|dependencies| {
        !dependencies.needs_document_base_url && !dependencies.may_need_style_sheet_resource_context
    })
}

/// The same question for a value that may read the base URLs a `url()` resolves against, which
/// the engine holds as published inputs: its dependencies, or `None` when it needs anything else.
fn value_computes_without_document_context_but_for_resources(
    value: &StyleValueData,
) -> Option<crate::css::style_compute::ExternalValueDependencies> {
    // A longhand a shorthand written with a substitution declares holds a pending substitution
    // until the shorthand resolves; both compute in C++.
    if matches!(
        value,
        StyleValueData::Unresolved { .. } | StyleValueData::PendingSubstitution { .. }
    ) || crate::css::style_compute::value_is_computationally_independent(value).is_none()
    {
        return None;
    }
    let dependencies = crate::css::style_compute::external_value_dependencies(value);
    (!dependencies.uses_tree_counting_function
        && dependencies.container_relative_length_unit_mask == 0
        && !dependencies.has_unfixed_random_sharing
        && !dependencies.uses_random_function)
        .then_some(dependencies)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retained_highlight_inheritance_parent_uses_nearest_ancestor_pseudo_record() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut raw_nodes = [0; 4];
        engine.allocate_style_nodes(&mut raw_nodes);
        let [root, parent, child, slot] = raw_nodes.map(|node| StyleNodeID::from_raw(node).unwrap());
        engine.tree.set_parent(parent, Some(root));
        engine.tree.set_parent(child, Some(parent));
        let retained = &mut engine.state.retained;
        retained.tree.set_assigned_slot(child, Some(slot), &mut retained.memory);
        retained.tree.set_parent(slot, Some(root));
        let publish = |engine: &mut StyleEngine, node, pseudo_kind| {
            engine
                .publish_computed_groups(
                    computed::ComputedStyleTarget::new(node, pseudo_kind),
                    &[],
                    0,
                    0,
                    computed::ComputedMetadataInput {
                        pseudo_element_styles: 0,
                        dependency_flags: 0,
                        counter_style_environment_identity: 0,
                        animation_overlay_identity: 0,
                        animated_overlay: HostShared::null(),
                        animation_overlay_payloads: &[],
                        longhand_table: HostShared::null(),
                    },
                )
                .style_record_identity
        };
        let root_record = publish(&mut engine, root, 0);
        assert_eq!(
            engine.retained_highlight_inheritance_parent_style_record(parent, 0),
            Some(root_record)
        );
        assert_eq!(
            engine.retained_highlight_inheritance_parent_style_record(child, 0),
            Some(root_record)
        );
        let slot_record = publish(&mut engine, slot, 0);
        assert_eq!(
            engine.retained_highlight_inheritance_parent_style_record(child, 0),
            Some(slot_record)
        );
    }

    #[test]
    fn acknowledgement_without_pending_records_observes_published_answer() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut raw_nodes = [0; 2];
        engine.allocate_style_nodes(&mut raw_nodes);
        let [first, second] = raw_nodes.map(|node| StyleNodeID::from_raw(node).unwrap());
        for node in [first, second] {
            engine.state.retained.published_match_answers.push(
                PublishedMatchAnswer {
                    node,
                    cascade_input: None,
                    matches: None,
                    cascade_winners_are_complete: true,
                    observed: false,
                },
                &mut engine.state.retained.memory,
                &mut engine.counters,
            );
        }
        engine.published_match_answers.sort();
        assert!(engine.engine_computed_records_pending.is_empty());

        engine.acknowledge_engine_computed_record(second);
        let effects = std::mem::take(&mut engine.published_match_answers.answer_effects);
        engine.state.install_answer_effects(effects);
        assert!(engine.published_match_answers.lookup(second).unwrap().observed);
        assert!(!engine.published_match_answers.lookup(first).unwrap().observed);
        engine.acknowledge_engine_computed_record(second);
        let effects = std::mem::take(&mut engine.published_match_answers.answer_effects);
        engine.state.install_answer_effects(effects);
        assert!(engine.published_match_answers.lookup(second).unwrap().observed);
        assert_eq!(engine.counters.get(Counter::EngineComputedLonghandEvaluations), 0);
    }

    #[test]
    fn pending_records_acknowledge_and_rollback_independently_by_node() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut raw_nodes = [0; 3];
        engine.allocate_style_nodes(&mut raw_nodes);
        let [first, second, third] = raw_nodes.map(|node| StyleNodeID::from_raw(node).unwrap());
        let publish = |engine: &mut StyleEngine, node, pseudo_kind| {
            let record = engine
                .publish_computed_groups(
                    computed::ComputedStyleTarget::new(node, pseudo_kind),
                    &[],
                    0,
                    0,
                    computed::ComputedMetadataInput {
                        pseudo_element_styles: 0,
                        dependency_flags: 0,
                        counter_style_environment_identity: 0,
                        animation_overlay_identity: 0,
                        animated_overlay: HostShared::null(),
                        animation_overlay_payloads: &[],
                        longhand_table: HostShared::null(),
                    },
                )
                .style_record_identity;
            engine
                .engine_computed_records_pending
                .entry(node)
                .or_default()
                .push(PendingEngineComputedRecord {
                    node,
                    pseudo_kind,
                    old_style_record: computed::FinalStyleRecordID::NONE,
                    new_style_record: record,
                    cascade_state: None,
                    longhand_evaluations: 1,
                });
            record
        };
        let first_record = publish(&mut engine, first, u8::MAX);
        publish(&mut engine, third, u8::MAX);
        let second_record = publish(&mut engine, second, u8::MAX);
        let first_pseudo = publish(&mut engine, first, 0);
        publish(&mut engine, third, 0);

        // Acknowledge out of publication order, including a repeated acknowledgement.
        engine.acknowledge_engine_computed_record(second);
        engine.acknowledge_engine_computed_record(second);
        assert_eq!(engine.counters.get(Counter::EngineComputedLonghandEvaluations), 1);
        engine.acknowledge_engine_computed_record(first);
        assert_eq!(engine.counters.get(Counter::EngineComputedLonghandEvaluations), 3);

        // A later batch can use the same node, and discarding it leaves installed nodes alone.
        publish(&mut engine, third, u8::MAX);
        publish(&mut engine, third, 0);
        engine.state.discard_engine_computed_records(&mut engine.counters);
        engine.acknowledge_engine_computed_record(third);
        assert!(engine.engine_computed_records_pending.is_empty());
        assert_eq!(engine.counters.get(Counter::EngineComputedLonghandEvaluations), 3);
        assert_eq!(engine.computed_group_sets.assigned_style_record(third), None);
        assert_eq!(engine.computed_group_sets.pseudo_style_record(third, 0), None);
        assert_eq!(
            engine.computed_group_sets.assigned_style_record(first),
            Some(first_record)
        );
        assert_eq!(
            engine.computed_group_sets.assigned_style_record(second),
            Some(second_record)
        );
        assert_eq!(
            engine.computed_group_sets.pseudo_style_record(first, 0),
            Some(first_pseudo)
        );
    }

    #[test]
    fn discarded_first_record_leaves_no_container_query_inputs() {
        let mut engine = StyleEngine::new(DeviceClass::ForegroundDesktop);
        let mut raw_nodes = [0; 1];
        engine.allocate_style_nodes(&mut raw_nodes);
        let node = StyleNodeID::from_raw(raw_nodes[0]).unwrap();
        let record = engine
            .publish_computed_groups(
                computed::ComputedStyleTarget::new(node, u8::MAX),
                &[],
                0,
                0,
                computed::ComputedMetadataInput {
                    pseudo_element_styles: 0,
                    dependency_flags: 0,
                    counter_style_environment_identity: 0,
                    animation_overlay_identity: 0,
                    animated_overlay: HostShared::null(),
                    animation_overlay_payloads: &[],
                    longhand_table: HostShared::null(),
                },
            )
            .style_record_identity;
        engine
            .engine_computed_records_pending
            .entry(node)
            .or_default()
            .push(PendingEngineComputedRecord {
                node,
                pseudo_kind: u8::MAX,
                old_style_record: computed::FinalStyleRecordID::NONE,
                new_style_record: record,
                cascade_state: None,
                longhand_evaluations: 1,
            });
        // The projection noting the derived record takes it, as note_engine_computed_record does
        // for a complete record.
        engine.container_query_inputs.set(
            node,
            tree::ContainerQueryInputRow {
                style_record: record.raw(),
                names: Vec::new(),
                is_size_container: true,
                is_inline_size_container: false,
                is_scroll_state_container: false,
                writing_mode: 0,
                direction: 0,
            },
        );

        // The host never installed the node's first record, so the node goes back to none, and a
        // container query must not read the discarded record through the projection.
        engine.state.discard_engine_computed_records(&mut engine.counters);
        assert_eq!(engine.computed_group_sets.assigned_style_record(node), None);
        assert!(engine.container_query_inputs(node).is_none());
    }
}

impl StyleEngineState {
    pub(crate) fn end_style_record_view_epoch(&mut self, counters: &mut Counters) {
        self.retained.computed_group_sets.end_style_record_view_epoch();
        self.reclaim_computed_memory_if_needed(counters);
    }
}

impl StyleEngineState {
    /// `engine_computed_record_delta` driven until the row settles. What a drive suspends on is
    /// refilled where it suspends and the same row is driven again at once, so no row waits behind
    /// another, and an element alike one before it finds the record that one computed.
    pub(super) fn settled_engine_computed_record_delta(
        &mut self,
        node: StyleNodeID,
        cascade_winners_are_complete: bool,
        exact_flipped_rules: Option<FlippedRules>,
        parent_inputs_moved: ParentInputsMoved,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> RecordDelta {
        loop {
            match self.engine_computed_record_delta(
                node,
                cascade_winners_are_complete,
                exact_flipped_rules,
                parent_inputs_moved,
                scratch,
                counters,
            ) {
                Ok(delta) => return delta,
                Err(Unanswered::Suspended(suspension)) => {
                    self.refill_suspension(suspension, Some(node), &mut scratch.font_drive, counters);
                }
            }
        }
    }

    /// Refill what a drive suspended on, so its caller can drive the same subject again: the
    /// font its request names, or the random bases it asked for. Both are the owner's to answer
    /// on the spot.
    pub(super) fn refill_suspension(
        &mut self,
        suspension: Suspension,
        node: Option<StyleNodeID>,
        font_drive: &mut drive::FontDriveScratch,
        counters: &mut Counters,
    ) {
        match suspension {
            Suspension::Font => self.refill_font_request(node, font_drive.take_suspended_request(), counters),
            Suspension::RandomBases => self.refill_random_base_requests(),
        }
    }

    pub(super) fn refill_font_request(
        &mut self,
        node: Option<StyleNodeID>,
        request: font_resolution::FontRequest,
        counters: &mut Counters,
    ) {
        // NB: Use resident selector-tree depths for this diagnostic. They are not flat-tree
        //     dependency-span proofs and must not buy ancestor traversals just for counting.
        if let Some(node) = node {
            counters.set(
                Counter::FontRefillBlockedDepth,
                counters
                    .get(Counter::FontRefillBlockedDepth)
                    .max(u64::from(self.tree.depth(node)) + 1),
            );
        }
        let resolver = self.host.font_resolver.as_ref().expect("a request has a font resolver");
        let snapshot = self.retained.font_face_snapshot.clone();
        let memo = self
            .retained
            .font_cascade_memo
            .as_ref()
            .map_or(0, |memo| memo.address());
        let resolutions = self
            .retained
            .font_resolution
            .as_mut()
            .expect("a request has a font resolution cache");
        if resolver.refill(memo, snapshot.as_ref(), resolutions, request) {
            counters.bump(Counter::FontRefillRounds);
            counters.bump(Counter::FontResolutionRequests);
        }
    }
}

impl StyleEngineState {
    /// What the host registered for one tree scope's `@counter-style` rules, as an identity that
    /// moves when the registry does. A record that read the registry names the identity it read,
    /// so an edit to those rules is what moves the record rather than the loss of the record.
    pub(crate) fn set_counter_style_environment_identity(&mut self, tree_scope: TreeScopeID, identity: u64) {
        self.retained
            .counter_style_environment_identities
            .insert(tree_scope, identity);
    }

    /// Establish the document element's font input before the consumer pass. The root's
    /// remaining properties and pseudos complete in their normal canonical position.
    pub(super) fn prepare_root_font_inputs(
        &mut self,
        node: StyleNodeID,
        cascade_winners_are_complete: bool,
        exact_flipped_rules: Option<FlippedRules>,
        parent_inputs_moved: ParentInputsMoved,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) {
        let inputs = self.document_style_computation_inputs;
        scratch.root_element_inputs = Some((node, RootFontInputs::from_document(&inputs)));
        let probe = |state: &mut Self, scratch: &mut EngineComputedRecordScratch, counters: &mut Counters| {
            state.engine_computed_element_record_delta(
                node,
                cascade_winners_are_complete,
                exact_flipped_rules,
                parent_inputs_moved,
                scratch,
                FontDriveGoal::RootInputs,
                counters,
            )
        };
        let mut answer = probe(self, scratch, counters);
        if let Err(Unanswered::Suspended(Suspension::RandomBases)) = answer {
            self.refill_random_base_requests();
            answer = probe(self, scratch, counters);
        }
        if let Err(Unanswered::Suspended(Suspension::Font)) = answer {
            let request = scratch.font_drive.take_suspended_request();
            self.root_font_request = Some(request.for_generation(inputs.font_environment_generation));
            // This update computed a new root request after the begin boundary. Resolve that
            // exceptional miss before consumers run.
            self.refill_font_request(Some(node), request, counters);
            answer = probe(self, scratch, counters);
        }
        let prepared = match answer {
            Ok(ElementAnswer::RootInputs(prepared)) => prepared,
            Ok(ElementAnswer::Delta(_)) | Err(_) => None,
        };
        if let Some(root_inputs) = prepared {
            scratch.root_font_inputs_changed = RootFontInputs::from_document(&inputs) != root_inputs;
            root_inputs.apply_to(&mut self.document_style_computation_inputs);
            counters.bump(Counter::RootFontInputsPrepared);
        } else {
            // NB: Preserve the current host root-metric route. Unproven font inputs do not
            //     turn every descendant into a host-boundary retry.
            counters.bump(Counter::RootFontInputsUnprovenFallbacks);
        }
        if scratch.font_drive.is_pending() {
            scratch.prepared_root_font = Some((node, parent_inputs_moved, std::mem::take(&mut scratch.font_drive)));
        }
        self.apply_substitution_effects(scratch);
    }

    /// Drive a row the pass declined over the ancestors the host installed before it, as the
    /// host would compute it where it applies the row. `armed` says the pass tied the row to those
    /// ancestors, which lets it drive an answer that declares past its winners.
    pub(super) fn drive_record_over_installed_ancestors(
        &mut self,
        node: StyleNodeID,
        armed: bool,
        pass_facts: DriveOverInstalledAncestors,
        counters: &mut Counters,
    ) -> RetriedEngineRecord {
        // An engine no document hosts computes no records.
        if !self.retained.computes_records() {
            return RetriedEngineRecord::default();
        }
        let font_environment_generation = self
            .retained
            .document_style_computation_inputs
            .font_environment_generation;
        if let Some(resolver) = &mut self.retained.font_resolution {
            resolver.prepare(font_environment_generation);
        }
        counters.bump(Counter::RetryAfterAncestorCalls);
        let started_at = std::time::Instant::now();
        let mut scratch = EngineComputedRecordScratch {
            root_font_inputs_changed: pass_facts.root_font_inputs_changed,
            document_environment_moved: pass_facts.document_environment_moved,
            viewport_moved: pass_facts.viewport_moved,
            ..EngineComputedRecordScratch::default()
        };
        let style_record = self.drive_record_over_installed_ancestors_loop(node, armed, &mut scratch, counters);
        counters.add(
            Counter::RetryAfterAncestorMicroseconds,
            u64::try_from(started_at.elapsed().as_micros()).unwrap_or(u64::MAX),
        );
        let mut retried = RetriedEngineRecord {
            style_record,
            ..RetriedEngineRecord::default()
        };
        if style_record != 0 {
            for delta in &scratch.pseudo_deltas {
                let kind = usize::from(delta.kind);
                if kind < bridge::RETRY_PSEUDO_RECORD_SLOTS {
                    retried.pseudo_records_present |= 1 << kind;
                    retried.pseudo_records[kind] = delta.new_style_record.raw();
                }
            }
        }
        retried
    }

    fn drive_record_over_installed_ancestors_loop(
        &mut self,
        node: StyleNodeID,
        armed: bool,
        scratch: &mut EngineComputedRecordScratch,
        counters: &mut Counters,
    ) -> u64 {
        loop {
            match self.drive_record_over_installed_ancestors_step(node, armed, scratch, counters) {
                Ok(record) => {
                    counters.bump(Counter::RetryAfterAncestorSettled);
                    return record;
                }
                Err(Unanswered::Suspended(suspension)) => {
                    self.refill_suspension(suspension, Some(node), &mut scratch.font_drive, counters);
                }
            }
        }
    }
}

/// A walk's scratch is movable to the worker that owns it for the length of that walk, and comes
/// back at the join. Nothing in it is shared while the walk runs, so `Send` is the whole bound:
/// the values a half-built record carries are borrowed through `HostShared`, and a frozen
/// `ComputedLonghandTable` fills its lazy memos atomically.
const _: () = {
    const fn assert_send<T: Send>() {}
    assert_send::<EngineComputedRecordScratch>();
};

impl StyleEngineState {
    pub(super) fn refill_random_base_requests(&mut self) {
        let requests = std::mem::take(&mut self.retained.random_base_requests);
        if requests.is_empty() {
            return;
        }
        for (node, name, shared) in requests {
            self.ensure_random_base_value(node, &name, shared);
        }
    }
}
